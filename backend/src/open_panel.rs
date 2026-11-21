//! `synapta-connect panel` — เปิดหน้าจัดการในเบราว์เซอร์ด้วยรหัสครั้งเดียว (ไม่เอา token ถาวรไปไว้ใน URL)
//!
//! ต้องอ่าน local_token ได้ = ต้องเป็นเจ้าของเครื่อง (หรือ Administrator) — ผู้ใช้อื่นทำไม่ได้ตั้งแต่ขั้นนี้
//! ลำดับหา token: ของ service (ProgramData) ก่อน แล้วของโหมด console (LOCALAPPDATA)

use crate::config::{Config, service_data_dir};
use std::path::PathBuf;

pub async fn run(local_port: Option<u16>) -> Result<String, String> {
    let port = local_port.unwrap_or_else(|| Config::default().local_port);
    let tokens: Vec<String> = token_candidates()
        .into_iter()
        .filter_map(|f| std::fs::read_to_string(f).ok())
        .map(|t| t.trim().to_string())
        .filter(|t| crate::proof::valid_local_token(t))
        .collect();
    if tokens.is_empty() {
        return Err(
            "อ่าน local_token ไม่ได้ — ต้องเป็นเจ้าของเครื่องที่ตั้งไว้ตอนติดตั้ง (หรือ Administrator) และ service ต้องกำลังทำงาน".into(),
        );
    }

    let _ = rustls::crypto::ring::default_provider().install_default();
    let token = verified_token(port, &tokens).await?;
    let reply: serde_json::Value = reqwest::Client::new()
        .post(format!("http://127.0.0.1:{port}/api/v1/local/panel-session"))
        .header("X-Synapta-Local", &token)
        .send()
        .await
        .and_then(|r| r.error_for_status())
        .map_err(|e| {
            format!("ติดต่อ agent ที่พอร์ต {port} ไม่ได้ (service ทำงานอยู่หรือไม่? `synapta-connect service status`): {e}")
        })?
        .json()
        .await
        .map_err(|e| format!("คำตอบจาก agent อ่านไม่ได้: {e}"))?;
    let code = reply["code"].as_str().ok_or("agent ไม่ส่งรหัสกลับมา")?;

    let url = format!("http://127.0.0.1:{port}/panel/#code={code}");
    open_browser(&url)?;
    Ok("เปิด panel ในเบราว์เซอร์แล้ว (ลิงก์ใช้ได้ครั้งเดียว 60 วินาที) — ถ้าไม่เปิดเอง สั่งคำสั่งนี้ใหม่".into())
}

fn token_candidates() -> Vec<PathBuf> {
    vec![
        service_data_dir().join("local_token"),
        Config::default().data_dir.join("local_token"),
    ]
}

fn open_browser(url: &str) -> Result<(), String> {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        // `start "" URL` — รหัสเป็น hex ล้วน ไม่มีอักขระที่ cmd ตีความ
        std::process::Command::new(crate::platform::win::system_tool("cmd.exe"))
            .args(["/C", "start", "", url])
            .creation_flags(0x0800_0000) // CREATE_NO_WINDOW
            .spawn()
            .map(|_| ())
            .map_err(|e| format!("เปิดเบราว์เซอร์ไม่ได้: {e}"))
    }
    #[cfg(not(windows))]
    {
        println!("{url}");
        Ok(())
    }
}

/// หา token ที่ "ผู้ฟังที่พอร์ตนั้นพิสูจน์ได้ว่ารู้" ก่อนส่ง token ไปให้ — โปรเซสอื่นที่ยึดพอร์ต (ตอน service หยุด) จะไม่ได้ token ของเรา
async fn verified_token(port: u16, tokens: &[String]) -> Result<String, String> {
    let client = reqwest::Client::new();
    let unreachable = |e: reqwest::Error| {
        format!("ติดต่อ agent ที่พอร์ต {port} ไม่ได้ (service ทำงานอยู่หรือไม่? `synapta-connect service status`): {e}")
    };
    for token in tokens {
        let nonce = crate::hex::random_id(16);
        let reply: serde_json::Value = client
            .get(format!("http://127.0.0.1:{port}/api/v1/local/health?nonce={nonce}"))
            .send()
            .await
            .map_err(unreachable)?
            .json()
            .await
            .map_err(|e| format!("คำตอบจาก agent อ่านไม่ได้: {e}"))?;
        let message = crate::proof::health_message(
            &nonce,
            reply["version"].as_str().unwrap_or_default(),
            reply["pid"].as_u64().unwrap_or_default() as u32,
        );
        if crate::proof::check(
            token,
            crate::proof::HEALTH,
            &message,
            reply["proof"].as_str().unwrap_or_default(),
        ) {
            return Ok(token.clone());
        }
    }
    Err("ผู้ฟังที่พอร์ตนี้พิสูจน์ไม่ได้ว่าเป็น agent ของเครื่องนี้ — ไม่ส่ง token ให้ (มีโปรแกรมอื่นใช้พอร์ตอยู่หรือ service ยังไม่ทำงาน?)".into())
}
