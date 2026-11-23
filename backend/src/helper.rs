//! helper ใน session ของเจ้าของเครื่อง (`synapta-connect helper`) — มือและตาของ service ที่ทำงานใน Session 0
//!
//! ทำงานด้วยสิทธิ์ของผู้ใช้ที่ล็อกอิน (เริ่มจาก HKLM\...\Run ตอนล็อกอิน) ต่อเข้า service ที่ loopback ด้วย local_token
//! ผู้ใช้ที่ไม่ใช่เจ้าของอ่านไฟล์ token ไม่ได้ → helper ของเขาเลิกทำงานเงียบ ๆ จึงไม่มีทางเป็นตาของ service ได้
//! ตัวตนของ helper = สิทธิ์อ่านไฟล์ token (ACL) ไม่ใช่สิ่งที่ helper อ้างเอง; รับเฉพาะ method จับภาพ/input (display.rs)
//! และคำสั่งที่รันด้วยบัญชีเจ้าของ (run_as.rs)

use crate::{
    config::{Mode, Ports, panel_port, service_data_dir},
    display::{self, HelperInfo},
    exec, proof, pty,
    rpc::{Res, RpcError, parse, to_value},
    run_as,
};
use futures_util::{SinkExt, StreamExt};
use serde_json::{Value, json};
use std::time::Duration;
use tokio::sync::mpsc;
use tokio_tungstenite::tungstenite::{Message, client::IntoClientRequest};

const RETRY_MIN: Duration = Duration::from_secs(5);
const RETRY_MAX: Duration = Duration::from_secs(60);

/// รันจนกว่าจะอ่าน token ไม่ได้ (ไม่ใช่เจ้าของ / ถอนการติดตั้งแล้ว)
pub async fn run() -> anyhow::Result<()> {
    #[cfg(windows)]
    if !claim_session(handed_off()) {
        return Ok(()); // มี helper ใน session นี้อยู่แล้ว
    }
    let data_dir = service_data_dir();
    let token_file = data_dir.join("local_token");
    // อ่าน token ได้ = เป็นเจ้าของ จึงสร้างไอคอนเปิด panel และไอคอนมุมจอให้ (ทำเบื้องหลัง ไม่ให้ช้า helper)
    #[cfg(windows)]
    if std::fs::File::open(&token_file).is_ok() {
        std::thread::spawn(crate::shortcut::ensure_for_current_user);
        crate::tray::start();
    }
    let result = serve(&data_dir, &token_file).await;
    #[cfg(windows)]
    crate::tray::stop();
    result
}

async fn serve(data_dir: &std::path::Path, token_file: &std::path::Path) -> anyhow::Result<()> {
    let mut wait = RETRY_MIN;
    loop {
        #[cfg(windows)]
        if newer_version_installed() {
            // เอาไอคอนมุมจอออกก่อน ตัวใหม่จะใส่ของตัวเอง — ไม่ให้มีสองไอคอนซ้อนกัน
            crate::tray::stop();
            start_in_this_session_as(true);
            return Ok(());
        }
        // อ่านใหม่ทุกรอบ: service สุ่ม token ใหม่ทุกครั้งที่เริ่ม
        let Ok(token) = std::fs::read_to_string(token_file) else {
            return Ok(()); // ไม่มีสิทธิ์อ่าน = ผู้ใช้นี้ไม่ใช่เจ้าของ (หรือยังไม่ได้ติดตั้ง) ไม่ใช่ความผิดพลาด
        };
        if !proof::valid_local_token(token.trim()) {
            tokio::time::sleep(RETRY_MIN).await;
            continue;
        }
        let port = panel_port(data_dir).unwrap_or_else(|| Ports::default().local_port);
        match session(port, token.trim()).await {
            Ok(()) => wait = RETRY_MIN, // เคยต่อได้แล้วหลุด (service รีสตาร์ท) ลองใหม่ถี่ ๆ
            Err(e) => {
                tracing::debug!("helper ต่อ service ไม่ได้: {e}");
                wait = (wait * 2).min(RETRY_MAX);
            }
        }
        let jitter = Duration::from_millis(u64::from(rand::random::<u16>() % 1000));
        tokio::time::sleep(wait + jitter).await;
    }
}

/// ต่อ agent แล้วทำงานให้จนหลุด — **ตรวจก่อนว่าฝั่งที่ฟังคือ agent จริง** แล้วจึงส่งอะไรออกไปหรือรับคำสั่งใด ๆ:
/// ส่ง nonce + HMAC ที่ผูกกับ token (ไม่ส่ง token), แล้วต้องได้ HMAC ของฝั่ง agent กลับมาในคำตอบการอัปเกรดที่ตรงกัน
/// ไม่ตรง = ตัดทันที (ผู้ใช้อื่นที่ยึดพอร์ต 52402 ตอน service หยุด สั่งจับภาพ/คลิก/พิมพ์ใน session ของเจ้าของไม่ได้)
pub async fn session(port: u16, token: &str) -> anyhow::Result<()> {
    let nonce = crate::hex::random_id(16);
    let mut request = format!("ws://127.0.0.1:{port}/api/v1/local/helper").into_client_request()?;
    request.headers_mut().insert("x-synapta-nonce", nonce.parse()?);
    request
        .headers_mut()
        .insert("x-synapta-proof", proof::make(token, proof::HELPER, &nonce).parse()?);
    let (socket, response) =
        tokio::time::timeout(Duration::from_secs(8), tokio_tungstenite::connect_async(request)).await??;
    let server_proof = response
        .headers()
        .get("x-synapta-server-proof")
        .and_then(|v| v.to_str().ok())
        .unwrap_or_default();
    anyhow::ensure!(
        proof::check(token, proof::SERVER, &nonce, server_proof),
        "ฝั่งที่ฟังพอร์ตไม่ใช่ agent จริง (พิสูจน์ตัวไม่ผ่าน) — ไม่ทำงานให้"
    );
    let (mut sink, mut stream) = socket.split();

    let me = HelperInfo {
        user: std::env::var("USERNAME")
            .or_else(|_| std::env::var("USER"))
            .unwrap_or_default(),
        session: session_id(),
        elevated: is_elevated::is_elevated(),
    };
    sink.send(Message::Text(json!({"hello": me}).to_string().into()))
        .await?;
    #[cfg(windows)]
    let _online = Online::mark();

    // งานแต่ละตัวทำแยกกัน (จับภาพช้าต้องไม่ขวางการคลิก) ผลกลับผ่านคิวเดียวไปยังตัวเขียน socket
    // output ของ terminal ก็ออกทางคิวนี้ (notification ไม่มี id)
    let (out, mut replies) = mpsc::channel::<String>(16);
    // terminal ที่เปิดให้ service ตายไปพร้อม connection นี้ → ปิดทั้งหมดเมื่อ session จบ
    let ptys = PtyCleanup(pty::Registry::default());
    loop {
        tokio::select! {
            msg = stream.next() => match msg {
                Some(Ok(Message::Text(text))) => {
                    let (out, ptys) = (out.clone(), ptys.0.clone());
                    tokio::spawn(async move {
                        let reply = handle(&text, &ptys, &out).await;
                        let _ = out.send(reply).await;
                    });
                }
                Some(Ok(Message::Ping(p))) => sink.send(Message::Pong(p)).await?,
                Some(Ok(Message::Close(_))) | Some(Err(_)) | None => return Ok(()),
                _ => {}
            },
            Some(reply) = replies.recv() => sink.send(Message::Text(reply.into())).await?,
        }
    }
}

/// สถานะบนไอคอนมุมจอ: ต่อ Agent ได้ตลอดอายุของค่านี้ หลุดเมื่อ session จบไม่ว่าทางไหน
#[cfg(windows)]
struct Online;

#[cfg(windows)]
impl Online {
    fn mark() -> Online {
        crate::tray::set_online(true);
        Online
    }
}

#[cfg(windows)]
impl Drop for Online {
    fn drop(&mut self) {
        crate::tray::set_online(false);
    }
}

struct PtyCleanup(pty::Registry);

impl Drop for PtyCleanup {
    fn drop(&mut self) {
        self.0.close_all();
    }
}

async fn handle(text: &str, ptys: &pty::Registry, out: &mpsc::Sender<String>) -> String {
    let v: Value = serde_json::from_str(text).unwrap_or_default();
    let id = v["id"].clone();
    let method = v["method"].as_str().unwrap_or_default();
    let params = v["params"].clone();
    let result = if display::is_display_method(method) {
        display::execute(method, params).await
    } else if run_as::is_command_method(method) {
        command(method, params, ptys, out).await
    } else {
        Err(RpcError::new(
            crate::rpc::METHOD_NOT_FOUND,
            format!("helper ไม่รับ method '{method}'"),
        ))
    };
    match result {
        Ok(r) => json!({"id": id, "result": r}),
        Err(e) => json!({"id": id, "error": e}),
    }
    .to_string()
}

/// คำสั่งที่ service ส่งมาให้รันด้วยบัญชีเจ้าของ — Mode::Console = PATH ของผู้ใช้นี้ (รวมที่เพิ่งติดตั้งหลังล็อกอิน)
async fn command(method: &str, params: Value, ptys: &pty::Registry, out: &mpsc::Sender<String>) -> Res<Value> {
    let ok = || Ok(json!({"ok": true}));
    match method {
        "exec.run" => to_value(exec::run(parse(params)?, Mode::Console).await?),
        "pty.open" => {
            // id มาจาก service ที่จองเส้นทาง output ไว้แล้ว — รับเฉพาะรูปแบบที่ service สร้าง
            let id = params["pty_id"].as_str().unwrap_or_default().to_string();
            if id.is_empty() || id.len() > 32 || !id.bytes().all(|b| b.is_ascii_hexdigit()) {
                return Err(RpcError::invalid("pty_id ไม่ถูกต้อง"));
            }
            ptys.open_as(id, parse(params["open"].clone())?, out.clone(), Mode::Console)
        }
        "pty.write" => ptys.write(parse(params)?).and_then(|_| ok()),
        "pty.resize" => ptys.resize(parse(params)?).and_then(|_| ok()),
        "pty.close" => ptys.close(parse(params)?).and_then(|_| ok()),
        "helper.elevate" => elevate(),
        other => Err(RpcError::new(
            crate::rpc::METHOD_NOT_FOUND,
            format!("helper ไม่รับ method '{other}'"),
        )),
    }
}

/// เริ่ม helper ตัวใหม่ด้วยสิทธิ์ administrator (UAC) แล้วตัวนี้ออก — ตัวใหม่รอ mutex ของ session จนตัวนี้ออก
#[cfg(windows)]
fn elevate() -> Res<Value> {
    if is_elevated::is_elevated() {
        return Ok(json!({"elevated": true}));
    }
    run_elevated_helper()?;
    std::thread::spawn(|| {
        std::thread::sleep(Duration::from_millis(500)); // ให้คำตอบถึง service ก่อน
        crate::tray::stop();
        std::process::exit(0);
    });
    Ok(json!({"elevated": true}))
}

#[cfg(not(windows))]
fn elevate() -> Res<Value> {
    Err(RpcError::new(
        crate::rpc::NOT_AVAILABLE,
        "ขอสิทธิ์ administrator ได้เฉพาะบน Windows",
    ))
}

/// เปิดหน้าต่าง UAC แล้วรอจนเจ้าของตอบ (ShellExecuteEx "runas")
#[cfg(windows)]
fn run_elevated_helper() -> Res<()> {
    use windows::{
        Win32::{
            Foundation::ERROR_CANCELLED,
            UI::{
                Shell::{SEE_MASK_NOASYNC, SHELLEXECUTEINFOW, ShellExecuteExW},
                WindowsAndMessaging::SW_HIDE,
            },
        },
        core::{HSTRING, PCWSTR, w},
    };
    let exe = std::env::current_exe()?;
    let file = HSTRING::from(exe.as_os_str());
    let args = HSTRING::from(format!("helper {HANDOFF_ARG}"));
    let mut info = SHELLEXECUTEINFOW {
        cbSize: size_of::<SHELLEXECUTEINFOW>() as u32,
        fMask: SEE_MASK_NOASYNC,
        lpVerb: w!("runas"),
        lpFile: PCWSTR(file.as_ptr()),
        lpParameters: PCWSTR(args.as_ptr()),
        nShow: SW_HIDE.0,
        ..Default::default()
    };
    match unsafe { ShellExecuteExW(&mut info) } {
        Ok(()) => Ok(()),
        Err(e) if e.code() == ERROR_CANCELLED.to_hresult() => Err(RpcError::new(
            crate::rpc::INTERNAL,
            "เจ้าของเครื่องไม่ได้กดยืนยัน UAC — คำสั่งยังไม่ได้สิทธิ์ administrator",
        )),
        Err(e) => Err(RpcError::new(
            crate::rpc::INTERNAL,
            format!("ขอสิทธิ์ administrator ไม่ได้: {e}"),
        )),
    }
}

/// helper ได้ทีละตัวต่อ session — service ปฏิเสธตัวที่สอง ซึ่งจะวนต่อใหม่ไปเรื่อย ๆ จึงเลิกตั้งแต่ต้น
/// (mutex ใน namespace `Local\` แยกตาม session; handle ไม่ปิด ถือไว้จนโปรเซสจบ)
/// `wait`: ตัวที่รับช่วงจาก helper รุ่นเก่ารอให้ตัวเก่าออกก่อน แทนที่จะเลิกทันที
#[cfg(windows)]
fn claim_session(wait: bool) -> bool {
    let wait = Duration::from_secs(if wait { 15 } else { 0 });
    // HANDLE ที่ได้ถูกทิ้งไว้เปิดโดยตั้งใจ: mutex ต้องอยู่จนโปรเซสนี้จบ
    claim_mutex(windows::core::w!(r"Local\SynaptaConnectHelper"), wait).is_some()
}

/// None = มีตัวอื่นถือ mutex อยู่จนหมดเวลารอ · สร้าง mutex ไม่ได้เลยถือว่าได้ (แย่สุดคือมีตัวสำรองรออยู่)
#[cfg(windows)]
fn claim_mutex(name: windows::core::PCWSTR, wait: Duration) -> Option<windows::Win32::Foundation::HANDLE> {
    use windows::Win32::{
        Foundation::{CloseHandle, ERROR_ALREADY_EXISTS, GetLastError, HANDLE},
        System::Threading::CreateMutexW,
    };
    let deadline = std::time::Instant::now() + wait;
    loop {
        match unsafe { CreateMutexW(None, false, name) } {
            Ok(handle) => {
                if unsafe { GetLastError() } != ERROR_ALREADY_EXISTS {
                    return Some(handle);
                }
                // ต้องปิด handle ที่ได้มา ไม่งั้น mutex ไม่มีวันหายแม้ตัวเก่าจะออกไปแล้ว
                unsafe {
                    let _ = CloseHandle(handle);
                }
            }
            Err(_) => return Some(HANDLE::default()),
        }
        if std::time::Instant::now() >= deadline {
            return None;
        }
        std::thread::sleep(Duration::from_millis(250));
    }
}

/// ตัวแปรที่ helper รุ่นเก่าใส่ให้ตัวที่รับช่วง — ตัวที่รับช่วงแล้วไม่ส่งต่ออีก จึงไม่มีทางวนสร้างโปรเซสไม่รู้จบ
const HANDOFF_ENV: &str = "SYNAPTA_HELPER_HANDOFF";

/// แบบอาร์กิวเมนต์ — ตัวที่เริ่มผ่าน UAC ไม่ได้สืบทอด environment ของผู้เรียก
const HANDOFF_ARG: &str = "--handoff";

fn handed_off() -> bool {
    std::env::var_os(HANDOFF_ENV).is_some() || std::env::args().any(|a| a == HANDOFF_ARG)
}

/// helper อยู่ยาวตลอดการล็อกอิน แต่ตัวอัปเดตสลับไฟล์ใต้มันได้ — ตัวเก่าจะรันโค้ดเก่าต่อไปจนล็อกเอาต์
/// (input/จับภาพ/ไอคอนไม่ได้ของที่แก้ในรุ่นใหม่) จึงต้องเปลี่ยนตัวเองเป็นรุ่นที่ติดตั้งอยู่
#[cfg(windows)]
fn newer_version_installed() -> bool {
    let dir = crate::winsvc::install::install_dir();
    let installed_here = std::env::current_exe().is_ok_and(|exe| exe.starts_with(&dir));
    installed_here
        && !handed_off()
        && crate::update::installer::installed_version_from(&dir.join("install.json"))
            .is_some_and(|version| version != env!("CARGO_PKG_VERSION"))
}

/// เริ่ม helper ของ service ใน session นี้ (ตัวที่ติดตั้งไว้ใน Program Files) — มีอยู่แล้วตัวใหม่จะเลิกเอง
///
/// Run key เริ่ม helper ตอนล็อกอินเท่านั้น ถ้าติดตั้งขณะเจ้าของล็อกอินค้างอยู่ จะไม่มี helper จนล็อกอินรอบหน้า
/// `panel` ซึ่งรันด้วยบัญชีเจ้าของใน session นั้นอยู่แล้วจึงเรียกตัวนี้แทน
#[cfg(windows)]
pub fn start_in_this_session() {
    start_in_this_session_as(false);
}

#[cfg(windows)]
fn start_in_this_session_as(handoff: bool) {
    use std::os::windows::process::CommandExt;
    const DETACHED_PROCESS: u32 = 0x0000_0008;
    const CREATE_NEW_PROCESS_GROUP: u32 = 0x0000_0200;
    let exe = crate::winsvc::install::install_dir().join(crate::update::installer::EXE_NAME);
    if !exe.is_file() {
        return;
    }
    // กับดัก: stdio ต้องเป็น null — ถ้าสืบทอด pipe ของผู้เรียก (เช่นตัวติดตั้งที่อ่าน output ของ `panel` จนจบ)
    // helper ที่อยู่ยาวจะถือ pipe ไว้ ทำให้ผู้เรียกรอไม่มีวันจบ
    let mut command = std::process::Command::new(exe);
    if handoff {
        command.env(HANDOFF_ENV, "1");
    } else {
        command.env_remove(HANDOFF_ENV);
    }
    let _ = command
        .arg("helper")
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .creation_flags(DETACHED_PROCESS | CREATE_NEW_PROCESS_GROUP)
        .spawn();
}

#[cfg(windows)]
fn session_id() -> u32 {
    use windows::Win32::System::{RemoteDesktop::ProcessIdToSessionId, Threading::GetCurrentProcessId};
    let mut session = 0u32;
    unsafe {
        let _ = ProcessIdToSessionId(GetCurrentProcessId(), &mut session);
    }
    session
}

#[cfg(not(windows))]
fn session_id() -> u32 {
    0
}

#[cfg(all(test, windows))]
mod tests {
    use super::*;
    use windows::{
        Win32::Foundation::CloseHandle,
        core::{HSTRING, PCWSTR},
    };

    #[test]
    fn handoff_waits_for_the_old_helper_to_release_the_session() {
        let name = HSTRING::from(format!(r"Local\SynaptaConnectHelperTest-{}", crate::hex::random_id(8)));
        let old = claim_mutex(PCWSTR(name.as_ptr()), Duration::ZERO).expect("first claim");
        assert!(
            claim_mutex(PCWSTR(name.as_ptr()), Duration::ZERO).is_none(),
            "a second helper must not run"
        );

        let waiting = std::thread::spawn({
            let name = name.clone();
            move || claim_mutex(PCWSTR(name.as_ptr()), Duration::from_secs(5)).is_some()
        });
        std::thread::sleep(Duration::from_millis(600));
        unsafe {
            let _ = CloseHandle(old);
        }
        assert!(
            waiting.join().unwrap(),
            "the new helper takes over once the old one exits"
        );
    }
}
