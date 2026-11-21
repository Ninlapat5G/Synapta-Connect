//! Interactive terminal (PTY) — ConPTY บน Windows (WinPTY บนรุ่นที่ไม่มี ConPTY), PTY ปกติบน Unix
//!
//! PTY เป็นของ WebSocket connection ที่เปิด: output ส่งกลับเป็น notification
//! `pty.output` / `pty.exit` และทุก PTY ถูกฆ่าเมื่อ connection ปิดหรือ session ถูก revoke

use crate::hex::random_id;
use crate::rpc::{CONFLICT, Res, RpcError, notification};
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

/// PTY ทั้งหมดของ connection เดียว
#[derive(Clone, Default)]
pub struct Registry(Arc<Mutex<HashMap<String, Pty>>>);

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
    pub fn open(&self, p: OpenParams, out: Sender<String>) -> Res<Value> {
        if self.0.lock().unwrap().len() >= MAX_PTYS_PER_CONNECTION {
            return Err(RpcError::new(CONFLICT, "เปิด pty ได้ไม่เกิน 8 อันต่อ connection"));
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
        } = start(size, &shell, p.cwd.as_deref())?;

        let id = random_id(4);
        let pty = Pty {
            writer,
            killer: child.clone_killer(),
            master,
        };
        self.0.lock().unwrap().insert(id.clone(), pty);

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
        let pty = map.get_mut(&p.pty_id).ok_or_else(|| not_found(&p.pty_id))?;
        pty.writer.write_all(p.data.as_bytes())?;
        Ok(pty.writer.flush()?)
    }

    pub fn resize(&self, p: ResizeParams) -> Res<()> {
        let map = self.0.lock().unwrap();
        let pty = map.get(&p.pty_id).ok_or_else(|| not_found(&p.pty_id))?;
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
        let pty = map.get_mut(&p.pty_id).ok_or_else(|| not_found(&p.pty_id))?;
        Ok(pty.killer.kill()?)
    }

    pub fn close_all(&self) {
        for pty in self.0.lock().unwrap().values_mut() {
            let _ = pty.killer.kill();
        }
    }
}

/// PTY ที่เปิดแล้วพร้อมโปรเซส shell — ConPTY หรือ WinPTY ก็ได้รูปเดียวกัน
pub(crate) struct Started {
    pub master: Box<dyn MasterPty + Send>,
    pub reader: Box<dyn Read + Send>,
    pub writer: Box<dyn Write + Send>,
    pub child: Box<dyn Child + Send + Sync>,
}

fn start(size: PtySize, shell: &str, cwd: Option<&str>) -> Res<Started> {
    // ConPTY มีตั้งแต่ Windows 10 1809 / Server 2019 — เครื่องเก่ากว่า (เช่น Server 2016) ใช้ WinPTY แทน
    #[cfg(windows)]
    if !crate::legacy_pty::has_conpty() {
        return Ok(crate::legacy_pty::open(size, shell, cwd)?);
    }
    let pair = native_pty_system().openpty(size)?;
    let mut cmd = CommandBuilder::new(shell);
    if let Some(cwd) = cwd {
        cmd.cwd(cwd);
    }
    cmd.env("TERM", "xterm-256color");
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
