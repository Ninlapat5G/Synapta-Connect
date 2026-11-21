//! Terminal สำรองด้วย WinPTY สำหรับ Windows ที่ไม่มี ConPTY (ก่อน 10 1809 / Server 2019 เช่น Server 2016)
//!
//! winpty.dll กับ winpty-agent.exe (จาก node-pty 1.1.0, winpty 0.4.4-dev, MIT — ดู vendor/winpty/LICENSE)
//! ฝังมาใน exe แล้วแตกไว้ใน data-dir ของ agent ตอนใช้ครั้งแรก: อัปเดต backend จึงพาไฟล์ชุดใหม่มาเอง
//! และไม่มีขั้นติดตั้งเพิ่ม โฟลเดอร์นั้นเขียนได้เฉพาะบัญชีที่รัน agent, SYSTEM และ Administrators
//! (service: ACL จากตัวติดตั้ง · console: โปรไฟล์ของผู้ใช้เอง) ผู้ใช้อื่นจึงวาง DLL ปลอมให้ agent โหลดไม่ได้
//! และทุกครั้งที่เปิดจะเทียบไฟล์บนดิสก์กับของที่ฝังไว้ก่อนโหลด

use crate::pty::Started;
use anyhow::{Context, Result, bail};
use portable_pty::{Child, ChildKiller, ExitStatus, MasterPty, PtySize};
use sha2::{Digest, Sha256};
use shared_library::shared_library;
use std::{
    ffi::{OsStr, OsString, c_void},
    fs::{File, OpenOptions},
    io::{self, Read},
    os::windows::{
        ffi::{OsStrExt, OsStringExt},
        io::{AsRawHandle, FromRawHandle, RawHandle},
    },
    path::{Path, PathBuf},
    ptr,
    sync::{Arc, Mutex, OnceLock},
};

const WINPTY_DLL: &[u8] = include_bytes!("../vendor/winpty/winpty.dll");
const WINPTY_AGENT: &[u8] = include_bytes!("../vendor/winpty/winpty-agent.exe");
/// winpty.dll หา winpty-agent.exe จากโฟลเดอร์เดียวกับตัวเอง — ต้องอยู่คู่กันเสมอ
const FILES: [(&str, &[u8]); 2] = [("winpty.dll", WINPTY_DLL), ("winpty-agent.exe", WINPTY_AGENT)];

/// WINPTY_SPAWN_FLAG_AUTO_SHUTDOWN: shell จบ → agent ปิดตัว → pipe output ได้ EOF
const SPAWN_AUTO_SHUTDOWN: u64 = 1;
const WAIT_OBJECT_0: u32 = 0;
const WAIT_TIMEOUT: u32 = 258;
const INFINITE: u32 = u32::MAX;

static DATA_DIR: OnceLock<PathBuf> = OnceLock::new();

type Ptr = *mut c_void;

shared_library!(Api,
    pub fn winpty_config_new(flags: u64, error: *mut Ptr) -> Ptr,
    pub fn winpty_config_free(config: Ptr),
    pub fn winpty_config_set_initial_size(config: Ptr, cols: i32, rows: i32),
    pub fn winpty_open(config: Ptr, error: *mut Ptr) -> Ptr,
    pub fn winpty_free(pty: Ptr),
    pub fn winpty_conin_name(pty: Ptr) -> *const u16,
    pub fn winpty_conout_name(pty: Ptr) -> *const u16,
    pub fn winpty_spawn_config_new(flags: u64, app: *const u16, cmdline: *const u16, cwd: *const u16, env: *const u16, error: *mut Ptr) -> Ptr,
    pub fn winpty_spawn_config_free(config: Ptr),
    pub fn winpty_spawn(pty: Ptr, config: Ptr, process: *mut Ptr, thread: *mut Ptr, create_error: *mut u32, error: *mut Ptr) -> i32,
    pub fn winpty_set_size(pty: Ptr, cols: i32, rows: i32, error: *mut Ptr) -> i32,
);

unsafe extern "system" {
    fn GetModuleHandleW(name: *const u16) -> Ptr;
    fn GetProcAddress(module: Ptr, name: *const u8) -> Ptr;
    fn WaitForSingleObject(handle: RawHandle, millis: u32) -> u32;
    fn GetExitCodeProcess(handle: RawHandle, code: *mut u32) -> i32;
    fn GetProcessId(handle: RawHandle) -> u32;
    fn TerminateProcess(handle: RawHandle, code: u32) -> i32;
    fn CloseHandle(handle: Ptr) -> i32;
}

/// บอกที่แตกไฟล์ WinPTY (data-dir ของ agent) — เรียกครั้งเดียวตอนเริ่ม agent
pub fn set_data_dir(dir: &Path) {
    let _ = DATA_DIR.set(dir.to_path_buf());
}

/// Windows รุ่นนี้มี ConPTY ไหม (kernel32!CreatePseudoConsole)
pub fn has_conpty() -> bool {
    let kernel32 = wide("kernel32.dll");
    unsafe {
        !GetProcAddress(
            GetModuleHandleW(kernel32.as_ptr()),
            c"CreatePseudoConsole".as_ptr().cast(),
        )
        .is_null()
    }
}

/// เปิด shell ใน WinPTY
pub fn open(size: PtySize, shell: &str, cwd: Option<&str>) -> Result<Started> {
    let data = DATA_DIR.get().context("ยังไม่ได้กำหนดโฟลเดอร์ข้อมูลของ WinPTY")?;
    open_from(&unpack(data)?, size, shell, cwd)
}

/// แตกไฟล์ลง `<data>\winpty\<hash>\` (ชื่อตาม hash: agent รุ่นใหม่ไม่ต้องเขียนทับไฟล์ที่ PTY เดิมยังเปิดอยู่)
fn unpack(data: &Path) -> Result<PathBuf> {
    let mut hash = Sha256::new();
    for (_, bytes) in FILES {
        hash.update(bytes);
    }
    let dir = data.join("winpty").join(crate::hex::to_hex(&hash.finalize()[..8]));
    std::fs::create_dir_all(&dir).with_context(|| format!("สร้าง {} ไม่ได้", dir.display()))?;
    for (name, bytes) in FILES {
        let path = dir.join(name);
        if std::fs::read(&path).is_ok_and(|on_disk| on_disk == bytes) {
            continue;
        }
        crate::fsutil::atomic_write(&path, bytes).with_context(|| format!("เขียน {} ไม่ได้", path.display()))?;
    }
    Ok(dir)
}

fn open_from(dir: &Path, size: PtySize, shell: &str, cwd: Option<&str>) -> Result<Started> {
    // shell อยู่ในอัญประกาศของ command line — อัญประกาศหรือ NUL ข้างในจะทำให้ CreateProcess แยกคำผิด
    if shell.contains(['"', '\0']) || cwd.is_some_and(|dir| dir.contains('\0')) {
        bail!("shell หรือโฟลเดอร์เริ่มต้นมีอักขระที่ใช้ไม่ได้");
    }
    let api = Api::open(&dir.join("winpty.dll")).map_err(|e| anyhow::anyhow!("โหลด WinPTY ไม่ได้: {e:?}"))?;
    let session = Session::start(api, size)?;

    let command = wide(format!("\"{shell}\""));
    let cwd = cwd.map(wide);
    let child = session.spawn(&command, cwd.as_deref())?;

    let (conin, conout) = unsafe {
        let conin = (session.api.winpty_conin_name)(session.pty);
        let conout = (session.api.winpty_conout_name)(session.pty);
        if conin.is_null() || conout.is_null() {
            bail!("WinPTY ไม่ได้ให้ชื่อ pipe");
        }
        (from_wide(conin), from_wide(conout))
    };
    let writer = OpenOptions::new()
        .write(true)
        .open(conin)
        .context("เปิด pipe ขาเข้าของ WinPTY")?;
    let reader = OpenOptions::new()
        .read(true)
        .open(conout)
        .context("เปิด pipe ขาออกของ WinPTY")?;

    let session = Arc::new(session);
    Ok(Started {
        master: Box::new(Master(session.clone())),
        reader: Box::new(Reader {
            pipe: reader,
            _session: session,
        }),
        writer: Box::new(writer),
        child: Box::new(child),
    })
}

/// winpty_t หนึ่งตัว — ปิดเมื่อทั้ง master และ reader เลิกใช้ (Arc)
struct Session {
    api: Api,
    pty: Ptr,
    size: Mutex<PtySize>,
}

// WinPTY ระบุว่า winpty_t ใช้ข้าม thread ได้ ส่วน Api เป็นแค่ตาราง function pointer ของ DLL ที่ยังโหลดอยู่
unsafe impl Send for Session {}
unsafe impl Sync for Session {}

impl Session {
    fn start(api: Api, size: PtySize) -> Result<Session> {
        unsafe {
            let config = (api.winpty_config_new)(0, ptr::null_mut());
            if config.is_null() {
                bail!("สร้างค่าตั้ง WinPTY ไม่ได้");
            }
            (api.winpty_config_set_initial_size)(config, size.cols.into(), size.rows.into());
            let pty = (api.winpty_open)(config, ptr::null_mut());
            (api.winpty_config_free)(config);
            if pty.is_null() {
                bail!("เริ่ม winpty-agent ไม่ได้");
            }
            Ok(Session {
                api,
                pty,
                size: Mutex::new(size),
            })
        }
    }

    fn spawn(&self, command: &[u16], cwd: Option<&[u16]>) -> Result<Process> {
        unsafe {
            let config = (self.api.winpty_spawn_config_new)(
                SPAWN_AUTO_SHUTDOWN,
                ptr::null(),
                command.as_ptr(),
                cwd.map_or(ptr::null(), |dir| dir.as_ptr()),
                ptr::null(), // env: ใช้ของ agent
                ptr::null_mut(),
            );
            if config.is_null() {
                bail!("สร้างค่าตั้งโปรเซสของ WinPTY ไม่ได้");
            }
            let (mut process, mut thread, mut create_error) = (ptr::null_mut(), ptr::null_mut(), 0);
            let ok = (self.api.winpty_spawn)(
                self.pty,
                config,
                &mut process,
                &mut thread,
                &mut create_error,
                ptr::null_mut(),
            );
            (self.api.winpty_spawn_config_free)(config);
            if !thread.is_null() {
                CloseHandle(thread);
            }
            if ok == 0 || process.is_null() {
                bail!("เปิด shell ใน WinPTY ไม่ได้ (CreateProcess error {create_error})");
            }
            Ok(Process(Arc::new(File::from_raw_handle(process))))
        }
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        unsafe { (self.api.winpty_free)(self.pty) }
    }
}

struct Master(Arc<Session>);

impl MasterPty for Master {
    fn resize(&self, size: PtySize) -> Result<()> {
        let session = &self.0;
        let ok =
            unsafe { (session.api.winpty_set_size)(session.pty, size.cols.into(), size.rows.into(), ptr::null_mut()) };
        if ok == 0 {
            bail!("ปรับขนาด WinPTY ไม่ได้");
        }
        *session.size.lock().unwrap() = size;
        Ok(())
    }

    fn get_size(&self) -> Result<PtySize> {
        Ok(*self.0.size.lock().unwrap())
    }

    // pty.rs รับ reader/writer ไปตั้งแต่ตอนเปิดแล้ว ไม่ขอซ้ำ
    fn try_clone_reader(&self) -> Result<Box<dyn Read + Send>> {
        bail!("WinPTY ให้ reader ได้ครั้งเดียว")
    }

    fn take_writer(&self) -> Result<Box<dyn io::Write + Send>> {
        bail!("WinPTY ให้ writer ได้ครั้งเดียว")
    }
}

/// pipe ขาออก — ถือ Session ไว้ให้ agent อยู่จนอ่าน output ก้อนสุดท้ายเสร็จ
struct Reader {
    pipe: File,
    _session: Arc<Session>,
}

impl Read for Reader {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        self.pipe.read(buf)
    }
}

/// handle ของโปรเซส shell (File ใช้ปิด handle ตอน drop)
#[derive(Debug)]
struct Process(Arc<File>);

impl ChildKiller for Process {
    fn kill(&mut self) -> io::Result<()> {
        if unsafe { TerminateProcess(self.0.as_raw_handle(), 1) } == 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(())
    }

    fn clone_killer(&self) -> Box<dyn ChildKiller + Send + Sync> {
        Box::new(Process(self.0.clone()))
    }
}

impl Child for Process {
    fn try_wait(&mut self) -> io::Result<Option<ExitStatus>> {
        match unsafe { WaitForSingleObject(self.0.as_raw_handle(), 0) } {
            WAIT_OBJECT_0 => {
                let mut code = 0;
                if unsafe { GetExitCodeProcess(self.0.as_raw_handle(), &mut code) } == 0 {
                    return Err(io::Error::last_os_error());
                }
                Ok(Some(ExitStatus::with_exit_code(code)))
            }
            WAIT_TIMEOUT => Ok(None),
            _ => Err(io::Error::last_os_error()),
        }
    }

    fn wait(&mut self) -> io::Result<ExitStatus> {
        if unsafe { WaitForSingleObject(self.0.as_raw_handle(), INFINITE) } != WAIT_OBJECT_0 {
            return Err(io::Error::last_os_error());
        }
        self.try_wait()?.ok_or_else(|| io::Error::other("โปรเซสยังไม่จบ"))
    }

    fn process_id(&self) -> Option<u32> {
        Some(unsafe { GetProcessId(self.0.as_raw_handle()) })
    }

    fn as_raw_handle(&self) -> Option<RawHandle> {
        Some(self.0.as_raw_handle())
    }
}

fn wide(text: impl AsRef<OsStr>) -> Vec<u16> {
    text.as_ref().encode_wide().chain(Some(0)).collect()
}

unsafe fn from_wide(text: *const u16) -> PathBuf {
    let mut len = 0;
    while unsafe { *text.add(len) } != 0 {
        len += 1;
    }
    OsString::from_wide(unsafe { std::slice::from_raw_parts(text, len) }).into()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    // เครื่อง dev มี ConPTY อยู่แล้ว เทสต์นี้จึงเรียก WinPTY ตรง ๆ เพื่อให้เส้นทางของ Server 2016 ถูกรันจริง
    #[test]
    fn winpty_runs_shell_resizes_and_exits() {
        let data = std::env::temp_dir().join(format!("synapta-winpty-{}", crate::hex::random_id(4)));
        let dir = unpack(&data).unwrap();
        // แตกซ้ำต้องใช้ไฟล์เดิมได้ (ไม่เขียนทับ)
        assert_eq!(unpack(&data).unwrap(), dir);

        let mut pty = open_from(&dir, PtySize::default(), "cmd.exe", None).unwrap();
        pty.master
            .resize(PtySize {
                rows: 30,
                cols: 100,
                pixel_width: 0,
                pixel_height: 0,
            })
            .unwrap();
        assert_eq!(pty.master.get_size().unwrap().cols, 100);
        pty.writer.write_all(b"echo SYNAPTA_LEGACY_TEST\r\nexit 7\r\n").unwrap();
        pty.writer.flush().unwrap();
        let mut output = String::new();
        let _ = pty.reader.read_to_string(&mut output);
        assert!(output.contains("SYNAPTA_LEGACY_TEST"), "{output}");
        assert_eq!(pty.child.wait().unwrap().exit_code(), 7);
        drop(pty);
        let _ = std::fs::remove_dir_all(&data);
    }

    #[test]
    fn winpty_kill_ends_shell() {
        let data = std::env::temp_dir().join(format!("synapta-winpty-{}", crate::hex::random_id(4)));
        let mut pty = open_from(&unpack(&data).unwrap(), PtySize::default(), "cmd.exe", None).unwrap();
        pty.child.clone_killer().kill().unwrap();
        assert!(!pty.child.wait().unwrap().success());
        drop(pty);
        let _ = std::fs::remove_dir_all(&data);
    }

    #[test]
    fn rejects_quote_in_shell() {
        let err = open_from(Path::new("unused"), PtySize::default(), "cmd.exe\" /c calc", None)
            .err()
            .unwrap();
        assert!(err.to_string().contains("อักขระ"));
    }
}
