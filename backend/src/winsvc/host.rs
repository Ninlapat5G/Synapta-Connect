//! สั่งและถามสถานะ service จริงผ่าน SCM + ตรวจสุขภาพ agent ผ่าน loopback — ฝั่งที่ตัวติดตั้งใช้ (installer::Host)

use super::AGENT_SERVICE;
use crate::{
    config::{Config, service_data_dir},
    proof,
    update::installer::{Health, Host},
};
use std::{
    io::{Read, Write},
    net::{SocketAddr, TcpStream},
    time::{Duration, Instant},
};
use windows_service::{
    service::{Service, ServiceAccess, ServiceState},
    service_manager::{ServiceManager, ServiceManagerAccess},
};

const WAIT: Duration = Duration::from_secs(30);
// HRESULT-ish: ERROR_SERVICE_ALREADY_RUNNING (1056) / ERROR_SERVICE_NOT_ACTIVE (1062)
const ALREADY_RUNNING: i32 = 1056;
const NOT_ACTIVE: i32 = 1062;
const DOES_NOT_EXIST: i32 = 1060;

pub fn open(name: &str, access: ServiceAccess) -> Result<Service, String> {
    ServiceManager::local_computer(None::<&str>, ServiceManagerAccess::CONNECT)
        .and_then(|m| m.open_service(name, access))
        .map_err(|e| format!("เปิด service {name} ไม่ได้: {e}"))
}

fn win32_code(e: &windows_service::Error) -> Option<i32> {
    match e {
        windows_service::Error::Winapi(io) => io.raw_os_error(),
        _ => None,
    }
}

pub fn service_exists(name: &str) -> Result<bool, String> {
    match ServiceManager::local_computer(None::<&str>, ServiceManagerAccess::CONNECT)
        .and_then(|m| m.open_service(name, ServiceAccess::QUERY_STATUS))
    {
        Ok(_) => Ok(true),
        Err(e) if win32_code(&e) == Some(DOES_NOT_EXIST) => Ok(false),
        Err(e) => Err(format!("ตรวจสอบ service {name} ไม่ได้: {e}")),
    }
}

pub fn state_of(service: &Service) -> Result<ServiceState, String> {
    service
        .query_status()
        .map(|s| s.current_state)
        .map_err(|e| e.to_string())
}

pub fn start_service(name: &str) -> Result<(), String> {
    let service = open(name, ServiceAccess::START | ServiceAccess::QUERY_STATUS)?;
    match service.start::<&str>(&[]) {
        Ok(()) => Ok(()),
        Err(e) if win32_code(&e) == Some(ALREADY_RUNNING) => Ok(()),
        Err(e) => Err(format!("เริ่ม {name} ไม่ได้: {e}")),
    }
}

/// หยุดแล้วรอจนหยุดจริง (หยุดอยู่แล้ว = สำเร็จ)
pub fn stop_service(name: &str) -> Result<(), String> {
    let service = open(name, ServiceAccess::STOP | ServiceAccess::QUERY_STATUS)?;
    match service.stop() {
        Ok(_) => {}
        Err(e) if win32_code(&e) == Some(NOT_ACTIVE) => return Ok(()),
        Err(e) => return Err(format!("หยุด {name} ไม่ได้: {e}")),
    }
    let deadline = Instant::now() + WAIT;
    while Instant::now() < deadline {
        if state_of(&service)? == ServiceState::Stopped {
            return Ok(());
        }
        std::thread::sleep(Duration::from_millis(250));
    }
    Err(format!("{name} ไม่หยุดภายใน {} วินาที", WAIT.as_secs()))
}

pub struct SystemHost;

impl Host for SystemHost {
    fn stop_agent(&self) -> Result<(), String> {
        stop_service(AGENT_SERVICE)
    }
    fn start_agent(&self) -> Result<(), String> {
        start_service(AGENT_SERVICE)
    }
    fn health(&self) -> Result<Health, String> {
        http_health(Config::default().local_port)
    }
}

/// ถามสุขภาพ agent ผ่าน loopback **พร้อมตรวจว่าคนตอบคือ agent จริง** (ต้องอ่าน local_token ได้ = Administrator/SYSTEM/เจ้าของ)
/// โปรเซสอื่นที่ยึดพอร์ต 52402 ไว้ตอบรุ่น/pid ปลอมได้ แต่ทำ HMAC ของ token ไม่ได้ → ตัวติดตั้งจึงไม่ถูกหลอกว่ารุ่นใหม่ขึ้นแล้ว
/// เรียกจากเธรดปกติ (ไม่ต้องมี async runtime)
pub fn http_health(port: u16) -> Result<Health, String> {
    let token = std::fs::read_to_string(service_data_dir().join("local_token"))
        .map_err(|e| format!("อ่าน local_token ไม่ได้ (ต้องเป็น Administrator/เจ้าของ): {e}"))?;
    let token = token.trim();
    if !proof::valid_local_token(token) {
        return Err("local_token ยังไม่พร้อมใช้งาน".into());
    }
    let nonce = crate::hex::random_id(16);
    let body = http_get(port, &format!("/api/v1/local/health?nonce={nonce}"))?;
    let v: serde_json::Value = serde_json::from_str(&body).map_err(|e| format!("อ่านผล health ไม่ได้: {e}"))?;
    let health: Health = serde_json::from_value(v.clone()).map_err(|e| format!("ผล health ไม่ครบ: {e}"))?;
    let given = v["proof"].as_str().unwrap_or_default();
    let message = proof::health_message(&nonce, &health.version, health.pid);
    if !proof::check(token, proof::HEALTH, &message, given) {
        return Err("ผู้ตอบที่พอร์ตนี้ไม่ใช่ agent จริง (พิสูจน์ตัวไม่ผ่าน)".into());
    }
    Ok(health)
}

fn http_get(port: u16, path: &str) -> Result<String, String> {
    let addr = SocketAddr::from(([127, 0, 0, 1], port));
    let mut stream = TcpStream::connect_timeout(&addr, Duration::from_secs(2)).map_err(|e| e.to_string())?;
    stream.set_read_timeout(Some(Duration::from_secs(3))).ok();
    stream.set_write_timeout(Some(Duration::from_secs(3))).ok();
    write!(
        stream,
        "GET {path} HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nConnection: close\r\n\r\n"
    )
    .map_err(|e| e.to_string())?;
    let mut raw = String::new();
    // เพดานอ่านกันผู้ตอบปลอมส่งไม่หยุด
    (&mut stream)
        .take(64 * 1024)
        .read_to_string(&mut raw)
        .map_err(|e| e.to_string())?;
    if !raw.starts_with("HTTP/1.1 200") {
        return Err(format!("health ตอบ {}", raw.lines().next().unwrap_or("")));
    }
    Ok(raw.split_once("\r\n\r\n").map_or("", |(_, b)| b).to_string())
}
