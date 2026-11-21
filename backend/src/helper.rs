//! helper ใน session ของเจ้าของเครื่อง (`synapta-connect helper`) — มือและตาของ service ที่ทำงานใน Session 0
//!
//! ทำงานด้วยสิทธิ์ของผู้ใช้ที่ล็อกอิน (เริ่มจาก HKLM\...\Run ตอนล็อกอิน) ต่อเข้า service ที่ loopback ด้วย local_token
//! ผู้ใช้ที่ไม่ใช่เจ้าของอ่านไฟล์ token ไม่ได้ → helper ของเขาเลิกทำงานเงียบ ๆ จึงไม่มีทางเป็นตาของ service ได้
//! ตัวตนของ helper = สิทธิ์อ่านไฟล์ token (ACL) ไม่ใช่สิ่งที่ helper อ้างเอง; รับเฉพาะ method จับภาพ/input (display.rs)

use crate::{
    config::{Ports, panel_port, service_data_dir},
    display::{self, HelperInfo},
    proof,
    rpc::RpcError,
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
    if !claim_session() {
        return Ok(()); // มี helper ใน session นี้อยู่แล้ว
    }
    let data_dir = service_data_dir();
    let token_file = data_dir.join("local_token");
    let mut wait = RETRY_MIN;
    loop {
        // อ่านใหม่ทุกรอบ: service สุ่ม token ใหม่ทุกครั้งที่เริ่ม
        let Ok(token) = std::fs::read_to_string(&token_file) else {
            return Ok(()); // ไม่มีสิทธิ์อ่าน = ผู้ใช้นี้ไม่ใช่เจ้าของ (หรือยังไม่ได้ติดตั้ง) ไม่ใช่ความผิดพลาด
        };
        if !proof::valid_local_token(token.trim()) {
            tokio::time::sleep(RETRY_MIN).await;
            continue;
        }
        let port = panel_port(&data_dir).unwrap_or_else(|| Ports::default().local_port);
        match session(port, token.trim()).await {
            Ok(()) => wait = RETRY_MIN, // เคยต่อได้แล้วหลุด (service รีสตาร์ท) ลองใหม่ถี่ ๆ
            Err(e) => {
                tracing::debug!("helper ต่อ service ไม่ได้: {e}");
                wait = (wait * 2).min(RETRY_MAX);
            }
        }
        tokio::time::sleep(wait).await;
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
    let (socket, response) = tokio_tungstenite::connect_async(request).await?;
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
    };
    sink.send(Message::Text(json!({"hello": me}).to_string().into()))
        .await?;

    // งานแต่ละตัวทำแยกกัน (จับภาพช้าต้องไม่ขวางการคลิก) ผลกลับผ่านคิวเดียวไปยังตัวเขียน socket
    let (out, mut replies) = mpsc::channel::<String>(16);
    loop {
        tokio::select! {
            msg = stream.next() => match msg {
                Some(Ok(Message::Text(text))) => {
                    let out = out.clone();
                    tokio::spawn(async move { let _ = out.send(handle(&text).await).await; });
                }
                Some(Ok(Message::Ping(p))) => sink.send(Message::Pong(p)).await?,
                Some(Ok(Message::Close(_))) | Some(Err(_)) | None => return Ok(()),
                _ => {}
            },
            Some(reply) = replies.recv() => sink.send(Message::Text(reply.into())).await?,
        }
    }
}

async fn handle(text: &str) -> String {
    let v: Value = serde_json::from_str(text).unwrap_or_default();
    let id = v["id"].clone();
    let method = v["method"].as_str().unwrap_or_default();
    let result = if display::is_display_method(method) {
        display::execute(method, v["params"].clone()).await
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

/// helper ได้ทีละตัวต่อ session — service ปฏิเสธตัวที่สอง ซึ่งจะวนต่อใหม่ไปเรื่อย ๆ จึงเลิกตั้งแต่ต้น
/// (mutex ใน namespace `Local\` แยกตาม session; handle ไม่ปิด ถือไว้จนโปรเซสจบ)
#[cfg(windows)]
fn claim_session() -> bool {
    use windows::{
        Win32::{
            Foundation::{ERROR_ALREADY_EXISTS, GetLastError},
            System::Threading::CreateMutexW,
        },
        core::w,
    };
    match unsafe { CreateMutexW(None, false, w!("Local\\SynaptaConnectHelper")) } {
        Ok(_) => {
            let error = unsafe { GetLastError() };
            error != ERROR_ALREADY_EXISTS
        }
        Err(_) => true, // สร้าง mutex ไม่ได้ก็ยังทำงานได้ — แย่สุดคือมีตัวสำรองรออยู่
    }
}

/// เริ่ม helper ของ service ใน session นี้ (ตัวที่ติดตั้งไว้ใน Program Files) — มีอยู่แล้วตัวใหม่จะเลิกเอง
///
/// Run key เริ่ม helper ตอนล็อกอินเท่านั้น ถ้าติดตั้งขณะเจ้าของล็อกอินค้างอยู่ จะไม่มี helper จนล็อกอินรอบหน้า
/// `panel` ซึ่งรันด้วยบัญชีเจ้าของใน session นั้นอยู่แล้วจึงเรียกตัวนี้แทน
#[cfg(windows)]
pub fn start_in_this_session() {
    use std::os::windows::process::CommandExt;
    const DETACHED_PROCESS: u32 = 0x0000_0008;
    const CREATE_NEW_PROCESS_GROUP: u32 = 0x0000_0200;
    let exe = crate::winsvc::install::install_dir().join(crate::update::installer::EXE_NAME);
    if !exe.is_file() {
        return;
    }
    // กับดัก: stdio ต้องเป็น null — ถ้าสืบทอด pipe ของผู้เรียก (เช่นตัวติดตั้งที่อ่าน output ของ `panel` จนจบ)
    // helper ที่อยู่ยาวจะถือ pipe ไว้ ทำให้ผู้เรียกรอไม่มีวันจบ
    let _ = std::process::Command::new(exe)
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
