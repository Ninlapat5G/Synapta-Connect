//! Interactive terminal (PTY) — ConPTY บน Windows (WinPTY บนรุ่นที่ไม่มี ConPTY), PTY ปกติบน Unix
//!
//! PTY เป็นของ WebSocket connection ที่เปิด: output ส่งกลับเป็น notification
//! `pty.output` / `pty.exit` และทุก PTY ถูกฆ่าเมื่อ connection ปิดหรือ session ถูก revoke
//!
//! PTY มีสองแบบในตารางเดียวกัน: ของโปรเซสนี้ (`Local`) และที่ helper เปิดด้วยบัญชีเจ้าของ (`Owner`, ดู run_as.rs)
//! ตารางนี้จึงเป็นตัวกันไม่ให้ client สั่งงาน terminal ของ client อื่น ไม่ว่าจะอยู่ที่ไหน

use crate::rpc::{CONFLICT, Res, RpcError, notification};
use crate::{config::Mode, hex::random_id};
use base64::{Engine, engine::general_purpose::STANDARD as B64};
use portable_pty::{Child, ChildKiller, CommandBuilder, MasterPty, PtySize, native_pty_system};
use serde::Deserialize;
use serde_json::{Value, json};
use std::{
    collections::HashMap,
    io::{Read, Write},
    sync::{Arc, Mutex, mpsc},
};
use tokio::sync::mpsc::Sender;

const MAX_PTYS_PER_CONNECTION: usize = 8;

struct Pty {
    master: Box<dyn MasterPty + Send>,
    writer: Box<dyn Write + Send>,
    killer: Box<dyn ChildKiller + Send + Sync>,
}

enum Entry {
    Local(Pty),
    /// เปิดอยู่ใน helper — งานกับมันต้องส่งผ่าน helper
    Owner,
}

/// PTY ทั้งหมดของ connection เดียว
#[derive(Clone, Default)]
pub struct Registry(Arc<Mutex<HashMap<String, Entry>>>);

#[derive(Deserialize)]
pub struct OpenParams {
    pub cols: Option<u16>,
    pub rows: Option<u16>,
    /// โปรแกรม shell (โดยปริยาย powershell.exe บน Windows, $SHELL หรือ sh บน Unix)
    pub shell: Option<String>,
    pub cwd: Option<String>,
}

#[derive(Deserialize)]
pub struct WriteParams {
    pub pty_id: String,
    pub data: String,
}

#[derive(Deserialize)]
pub struct ResizeParams {
    pub pty_id: String,
    pub cols: u16,
    pub rows: u16,
}

#[derive(Deserialize)]
pub struct CloseParams {
    pub pty_id: String,
}

fn not_found(id: &str) -> RpcError {
    RpcError::new(CONFLICT, format!("ไม่มี pty '{id}' (อาจจบไปแล้ว)"))
}

impl Registry {
    /// ยังเปิดเพิ่มได้ไหม — ใช้ทั้งที่นี่และก่อนขอให้ helper เปิด (เต็มแล้ว = ไม่ต้องรบกวน helper)
    pub fn check_room(&self) -> Res<()> {
        if self.0.lock().unwrap().len() >= MAX_PTYS_PER_CONNECTION {
            return Err(RpcError::new(CONFLICT, "เปิด pty ได้ไม่เกิน 8 อันต่อ connection"));
        }
        Ok(())
    }

    /// จด terminal ที่ helper เปิดให้ connection นี้
    pub fn adopt_owner(&self, id: &str) {
        self.0.lock().unwrap().insert(id.to_string(), Entry::Owner);
    }

    /// Some(true) = terminal ของ helper · Some(false) = ของโปรเซสนี้ · None = ไม่ใช่ของ connection นี้
    pub fn is_owner(&self, id: &str) -> Option<bool> {
        self.0.lock().unwrap().get(id).map(|e| matches!(e, Entry::Owner))
    }

    /// terminal ของ helper จบแล้ว
    pub fn forget(&self, id: &str) {
        self.0.lock().unwrap().remove(id);
    }

    pub fn open(&self, p: OpenParams, out: Sender<String>, mode: Mode) -> Res<Value> {
        self.open_as(random_id(4), p, out, mode)
    }

    /// เปิดด้วย id ที่ผู้เรียกกำหนด — helper ใช้ id ที่ service จองเส้นทาง output ไว้ก่อนแล้ว (output แรกจะไม่หาย)
    pub fn open_as(&self, id: String, p: OpenParams, out: Sender<String>, mode: Mode) -> Res<Value> {
        self.check_room()?;
        if self.0.lock().unwrap().contains_key(&id) {
            return Err(RpcError::new(CONFLICT, format!("pty '{id}' เปิดอยู่แล้ว")));
        }
        let size = PtySize {
            rows: p.rows.unwrap_or(30),
            cols: p.cols.unwrap_or(120),
            pixel_width: 0,
            pixel_height: 0,
        };
        let shell = p.shell.unwrap_or_else(default_shell);
        let Started {
            master,
            mut reader,
            writer,
            mut child,
        } = start(size, &shell, p.cwd.as_deref(), mode)?;

        let pty = Pty {
            writer,
            killer: child.clone_killer(),
            master,
        };
        self.0.lock().unwrap().insert(id.clone(), Entry::Local(pty));

        // thread รอโปรเซสจบ → ปล่อย master (บน Windows reader จะไม่ EOF จนกว่า ConPTY จะถูกปิด)
        // แล้วบอก reader ว่า exit code คืออะไร เพื่อให้ pty.exit มาหลัง output ก้อนสุดท้ายเสมอ
        let (code_tx, code_rx) = mpsc::channel::<Option<u32>>();
        let registry = self.clone();
        let waiter_id = id.clone();
        std::thread::spawn(move || {
            let code = child.wait().ok().map(|s| s.exit_code());
            registry.0.lock().unwrap().remove(&waiter_id);
            let _ = code_tx.send(code);
        });

        let reader_id = id.clone();
        std::thread::spawn(move || {
            let mut buf = [0u8; 8192];
            while let Ok(n) = reader.read(&mut buf) {
                if n == 0 {
                    break;
                }
                let params = json!({"pty_id": reader_id, "data_b64": B64.encode(&buf[..n])});
                // blocking_send: ถ้า client อ่านไม่ทัน PTY จะถูกชะลอตาม ไม่กินหน่วยความจำเพิ่ม
                if out.blocking_send(notification("pty.output", params)).is_err() {
                    return;
                }
            }
            let code = code_rx.recv().ok().flatten();
            let _ = out.blocking_send(notification(
                "pty.exit",
                json!({"pty_id": reader_id, "exit_code": code}),
            ));
        });
        Ok(json!({"pty_id": id}))
    }

    pub fn write(&self, p: WriteParams) -> Res<()> {
        let mut map = self.0.lock().unwrap();
        let pty = local(map.get_mut(&p.pty_id), &p.pty_id)?;
        pty.writer.write_all(p.data.as_bytes())?;
        Ok(pty.writer.flush()?)
    }

    pub fn resize(&self, p: ResizeParams) -> Res<()> {
        let mut map = self.0.lock().unwrap();
        let pty = local(map.get_mut(&p.pty_id), &p.pty_id)?;
        Ok(pty.master.resize(PtySize {
            rows: p.rows,
            cols: p.cols,
            pixel_width: 0,
            pixel_height: 0,
        })?)
    }

    /// สั่งฆ่า — thread ที่รออยู่จะเก็บกวาดและส่ง pty.exit เอง
    pub fn close(&self, p: CloseParams) -> Res<()> {
        let mut map = self.0.lock().unwrap();
        let pty = local(map.get_mut(&p.pty_id), &p.pty_id)?;
        // กับดัก: WinChildKiller::kill ของ portable-pty กลับผลของ TerminateProcess — ฆ่าสำเร็จกลับได้ Err
        // (เช่น os error 10035 ที่ค้างจากงานอื่น) จึงไม่เชื่อผลของมัน; ฆ่าไม่สำเร็จจริง pty.exit จะไม่มา ผู้ใช้เห็นเอง
        let _ = pty.killer.kill();
        Ok(())
    }

    /// ฆ่า terminal ของโปรเซสนี้ทั้งหมด แล้วคืน id ของ terminal ที่อยู่ใน helper ให้ผู้เรียกสั่งปิดต่อ
    pub fn close_all(&self) -> Vec<String> {
        let mut owner = Vec::new();
        for (id, entry) in self.0.lock().unwrap().iter_mut() {
            match entry {
                Entry::Local(pty) => {
                    let _ = pty.killer.kill();
                }
                Entry::Owner => owner.push(id.clone()),
            }
        }
        owner
    }
}

fn local<'a>(entry: Option<&'a mut Entry>, id: &str) -> Res<&'a mut Pty> {
    match entry {
        Some(Entry::Local(pty)) => Ok(pty),
        _ => Err(not_found(id)),
    }
}

/// PTY ที่เปิดแล้วพร้อมโปรเซส shell — ConPTY หรือ WinPTY ก็ได้รูปเดียวกัน
pub(crate) struct Started {
    pub master: Box<dyn MasterPty + Send>,
    pub reader: Box<dyn Read + Send>,
    pub writer: Box<dyn Write + Send>,
    pub child: Box<dyn Child + Send + Sync>,
}

fn start(size: PtySize, shell: &str, cwd: Option<&str>, mode: Mode) -> Res<Started> {
    #[cfg(windows)]
    let (shell, path) = (
        crate::platform::win::terminal_program(shell),
        crate::platform::win::terminal_path(mode),
    );
    // ConPTY มีตั้งแต่ Windows 10 1809 / Server 2019 — เครื่องเก่ากว่า (เช่น Server 2016) ใช้ WinPTY แทน
    #[cfg(windows)]
    if !crate::legacy_pty::has_conpty() {
        return Ok(crate::legacy_pty::open(size, &shell, cwd, path.as_deref())?);
    }
    #[cfg(not(windows))]
    let _ = mode;
    let pair = native_pty_system().openpty(size)?;
    let mut cmd = CommandBuilder::new(shell);
    if let Some(cwd) = cwd {
        cmd.cwd(cwd);
    }
    cmd.env("TERM", "xterm-256color");
    #[cfg(windows)]
    if let Some(path) = path {
        cmd.env("Path", path);
    }
    let child = pair.slave.spawn_command(cmd)?;
    drop(pair.slave);
    Ok(Started {
        reader: pair.master.try_clone_reader()?,
        writer: pair.master.take_writer()?,
        master: pair.master,
        child,
    })
}

fn default_shell() -> String {
    if cfg!(windows) {
        "powershell.exe".into()
    } else {
        std::env::var("SHELL").unwrap_or_else(|_| "sh".into())
    }
}
