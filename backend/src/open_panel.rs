//! `synapta-connect panel` — เปิดหน้าจัดการในเบราว์เซอร์ด้วยรหัสครั้งเดียว (ไม่เอา token ถาวรไปไว้ใน URL)
//!
//! ต้องอ่าน local_token ได้ = ต้องเป็นเจ้าของเครื่อง (หรือ Administrator) — ผู้ใช้อื่นทำไม่ได้ตั้งแต่ขั้นนี้
//! ลำดับหา token: ของ service (ProgramData) ก่อน แล้วของโหมด console (LOCALAPPDATA)

use crate::config::{Config, panel_port, service_data_dir};

pub async fn run(
    local_port: Option<u16>,
    lan: Option<&str>,
    enable_local: bool,
    recover_web: bool,
    link_only: bool,
) -> Result<String, String> {
    let config = Config::default();
    let _ = rustls::crypto::ring::default_provider().install_default();
    let client = reqwest::Client::builder()
        .no_proxy()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(std::time::Duration::from_secs(5))
        .build()
        .map_err(|e| e.to_string())?;
    let mut found_secret = false;
    for (dir, is_service) in [(service_data_dir(), true), (config.data_dir, false)] {
        let Ok(token) = std::fs::read_to_string(dir.join("local_token")) else {
            continue;
        };
        let token = token.trim();
        if !crate::proof::valid_local_token(token) {
            continue;
        }
        found_secret = true;
        let port = local_port.or_else(|| panel_port(&dir)).unwrap_or(config.local_port);
        if !verified(&client, port, token).await {
            continue;
        }
        // อ่าน token ของ service ได้ = เป็นเจ้าของ จึงเป็นจังหวะที่ helper ใน session นี้ควรทำงาน
        #[cfg(windows)]
        if is_service {
            crate::helper::start_in_this_session();
        }
        #[cfg(not(windows))]
        let _ = is_service;
        if enable_local {
            control(&client, port, token, "enable-panel", None).await?;
        }
        if recover_web {
            control(&client, port, token, "recover-web", None).await?;
        }
        let reply = control(&client, port, token, "panel-session", lan).await?;
        let code = reply["code"]
            .as_str()
            .filter(|code| code.len() == 32 && code.bytes().all(|b| b.is_ascii_hexdigit()))
            .ok_or("agent ไม่ส่งรหัสเปิดหน้าเว็บที่ใช้ได้กลับมา")?;
        if let Some(host) = lan {
            return Ok(format!("https://{host}:{port}{}#code={code}", crate::panel::PATH));
        }
        let url = format!("http://127.0.0.1:{port}{}#code={code}", crate::panel::PATH);
        if link_only {
            return Ok(url);
        }
        open_browser(&url)?;
        // พิมพ์ลิงก์ไว้ด้วย: ถ้าเบราว์เซอร์ไม่ขึ้น ยังคัดลอกไปเปิดเองได้
        return Ok(format!(
            "เปิด panel ในเบราว์เซอร์แล้ว (ลิงก์ใช้ได้ครั้งเดียว 60 วินาที)\nถ้าเบราว์เซอร์ไม่ขึ้น เปิดลิงก์นี้: {url}"
        ));
    }
    if !found_secret {
        return Err("อ่าน local_token ไม่ได้ — ต้องเป็นเจ้าของเครื่องที่ตั้งไว้ตอนติดตั้ง (หรือ Administrator)".into());
    }
    Err("ติดต่อ agent ไม่ได้หรือผู้ฟังพิสูจน์ตัวตนไม่ผ่าน ตรวจว่า service ทำงานอยู่และใช้พอร์ตที่ถูกต้อง".into())
}

async fn control(
    client: &reqwest::Client,
    port: u16,
    token: &str,
    endpoint: &str,
    lan: Option<&str>,
) -> Result<serde_json::Value, String> {
    let mut url =
        reqwest::Url::parse(&format!("http://127.0.0.1:{port}/api/v1/local/{endpoint}")).map_err(|e| e.to_string())?;
    if let Some(host) = lan {
        url.query_pairs_mut().append_pair("lan", host);
    }
    let time = crate::auth::now_secs().to_string();
    let nonce = crate::hex::random_id(16);
    let target = match url.query() {
        Some(query) => format!("{}?{query}", url.path()),
        None => url.path().to_string(),
    };
    let message = crate::proof::control_message(&time, &nonce, &target);
    let response = client
        .post(url)
        .header("X-Synapta-Time", time)
        .header("X-Synapta-Nonce", nonce)
        .header(
            "X-Synapta-Proof",
            crate::proof::make(token, crate::proof::PANEL_CONTROL, &message),
        )
        .send()
        .await
        .map_err(|e| e.to_string())?;
    let ok = response.status().is_success();
    let value: serde_json::Value = response
        .json()
        .await
        .map_err(|e| format!("อ่านคำตอบจาก agent ไม่ได้: {e}"))?;
    if !ok {
        return Err(value["error"]["message"].as_str().unwrap_or("Agent ปฏิเสธคำขอ").into());
    }
    Ok(value)
}

fn open_browser(url: &str) -> Result<(), String> {
    #[cfg(windows)]
    {
        // ใช้เบราว์เซอร์เริ่มต้นเฉพาะเมื่อผู้ใช้เลือกเบราว์เซอร์รุ่นใหม่ไว้จริง ไม่งั้นเปิด Chrome/Edge/Firefox ตรง ๆ
        // (ดู default_browser_usable — บน Server 2016 ค่าเริ่มต้นใช้ไม่ได้ทั้งแบบ IE และแบบ Edge รุ่นเก่า)
        if !default_browser_usable() {
            let Some(browser) = modern_browser() else {
                if http_handler_is_ie() {
                    return Err(
                        "เครื่องนี้มีแค่ Internet Explorer ซึ่งเปิดหน้าจัดการไม่ได้ — ติดตั้ง Chrome, Edge หรือ Firefox \
                         แล้วรัน `synapta-connect panel` อีกครั้ง"
                            .into(),
                    );
                }
                return start_default(url);
            };
            return std::process::Command::new(browser)
                .arg(url)
                .spawn()
                .map(|_| ())
                .map_err(|e| format!("เปิดเบราว์เซอร์ไม่ได้: {e}"));
        }
        start_default(url)
    }
    #[cfg(not(windows))]
    {
        println!("{url}");
        Ok(())
    }
}

/// ค่าในรีจิสทรีผ่าน reg.exe (ไม่มี = None)
#[cfg(windows)]
fn registry_value(key: &str, value: Option<&str>) -> Option<String> {
    use std::os::windows::process::CommandExt;
    let mut args = vec!["query", key];
    match value {
        Some(name) => args.extend(["/v", name]),
        None => args.push("/ve"),
    }
    let out = std::process::Command::new(crate::platform::win::system_tool("reg.exe"))
        .args(&args)
        .creation_flags(0x0800_0000) // CREATE_NO_WINDOW
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    // บรรทัดผล: "    <ชื่อ>    REG_SZ    <ค่า>"
    let text = String::from_utf8_lossy(&out.stdout).into_owned();
    text.lines()
        .find_map(|line| line.split_once("REG_SZ").map(|(_, v)| v.trim().to_string()))
        .filter(|v| !v.is_empty())
}

/// `start "" URL` — รหัสเป็น hex ล้วน ไม่มีอักขระที่ cmd ตีความ
#[cfg(windows)]
fn start_default(url: &str) -> Result<(), String> {
    use std::os::windows::process::CommandExt;
    std::process::Command::new(crate::platform::win::system_tool("cmd.exe"))
        .args(["/C", "start", "", url])
        .creation_flags(0x0800_0000) // CREATE_NO_WINDOW
        .spawn()
        .map(|_| ())
        .map_err(|e| format!("เปิดเบราว์เซอร์ไม่ได้: {e}"))
}

/// เบราว์เซอร์เริ่มต้นเปิด panel ได้จริงไหม
///
/// บั๊กจริงบน app01 (Server 2016, v0.3.6): UserChoice เป็น `AppXq0fevzme2pys62n3e0fbqa7peapykr8v` (Edge รุ่นเก่าแบบ UWP)
/// ซึ่งไม่มีบน Server — `start URL` จึงได้หน้าต่าง "How do you want to open this?" แทนเบราว์เซอร์
/// ผู้ใช้ไม่ทันเลือก รหัส 60 วินาทีก็หมด หน้า panel เลยดูเหมือนคำสั่งยืนยันสิทธิ์ใช้ไม่ได้
/// จึงนับว่า "ใช้ได้" เฉพาะ ProgId ของเบราว์เซอร์ที่ติดตั้งแบบปกติ (ไม่ใช่ IE และไม่ใช่ AppX)
#[cfg(windows)]
fn default_browser_usable() -> bool {
    match registry_value(
        r"HKCU\Software\Microsoft\Windows\Shell\Associations\UrlAssociations\http\UserChoice",
        Some("ProgId"),
    ) {
        Some(prog_id) => !prog_id.eq_ignore_ascii_case("IE.HTTP") && !prog_id.to_ascii_lowercase().starts_with("appx"),
        None => !http_handler_is_ie(),
    }
}

/// ตัวเปิด http ของทั้งเครื่องเป็น IE (หรือไม่มี) — ค่าเริ่มต้นของ Server 2016
#[cfg(windows)]
fn http_handler_is_ie() -> bool {
    registry_value(r"HKCR\http\shell\open\command", None).is_none_or(|cmd| cmd.to_lowercase().contains("iexplore"))
}

/// เบราว์เซอร์รุ่นใหม่ที่ลงทะเบียนใน App Paths (ตัวติดตั้งของเบราว์เซอร์เขียนไว้ทั้งแบบเครื่องและแบบผู้ใช้)
#[cfg(windows)]
fn modern_browser() -> Option<std::path::PathBuf> {
    ["chrome.exe", "msedge.exe", "firefox.exe"].iter().find_map(|exe| {
        ["HKLM", "HKCU"].iter().find_map(|hive| {
            let key = format!(r"{hive}\SOFTWARE\Microsoft\Windows\CurrentVersion\App Paths\{exe}");
            registry_value(&key, None)
                .map(|path| std::path::PathBuf::from(path.trim_matches('"')))
                .filter(|path| path.is_file())
        })
    })
}

/// หา token ที่ "ผู้ฟังที่พอร์ตนั้นพิสูจน์ได้ว่ารู้" ก่อนส่ง token ไปให้ — โปรเซสอื่นที่ยึดพอร์ต (ตอน service หยุด) จะไม่ได้ token ของเรา
async fn verified(client: &reqwest::Client, port: u16, token: &str) -> bool {
    let nonce = crate::hex::random_id(16);
    let Ok(response) = client
        .get(format!("http://127.0.0.1:{port}/api/v1/local/health?nonce={nonce}"))
        .send()
        .await
    else {
        return false;
    };
    let Ok(reply) = response.json::<serde_json::Value>().await else {
        return false;
    };
    let message = crate::proof::health_message(
        &nonce,
        reply["version"].as_str().unwrap_or_default(),
        reply["pid"].as_u64().unwrap_or_default() as u32,
    );
    crate::proof::check(
        token,
        crate::proof::HEALTH,
        &message,
        reply["proof"].as_str().unwrap_or_default(),
    )
}
