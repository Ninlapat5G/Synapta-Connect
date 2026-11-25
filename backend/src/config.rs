//! ค่าตั้งต้นของ agent — อ่านจาก command line เท่านั้น (ไม่มีไฟล์ config)

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Ports {
    pub port: u16,
    pub discovery_port: u16,
    pub local_port: u16,
}

impl Default for Ports {
    fn default() -> Self {
        Self {
            port: 52401,
            discovery_port: 52400,
            local_port: 52402,
        }
    }
}

impl Ports {
    pub fn validate(self) -> Result<Self, &'static str> {
        if self.port == 0 || self.discovery_port == 0 || self.local_port == 0 || self.port == self.local_port {
            return Err("ใช้พอร์ต 1–65535 และกำหนดพอร์ต TCP ของ Agent กับ panel ให้ต่างกัน");
        }
        Ok(self)
    }
}

pub fn panel_port(data_dir: &Path) -> Option<u16> {
    std::fs::read_to_string(data_dir.join("panel-port"))
        .ok()?
        .trim()
        .parse::<u16>()
        .ok()
        .filter(|port| *port > 0)
}

/// console = เปิดมือในเทอร์มินัล (พฤติกรรม Phase 0 เดิม) · service = รันเป็น Windows service เบื้องหลัง
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    Console,
    Service,
    Portable,
}

pub const PORTABLE_DISCOVERY_PORT: u16 = 52403;

#[derive(Clone, Debug)]
pub struct Config {
    pub mode: Mode,
    /// พอร์ต TCP ของ API — TLS 1.3 เท่านั้น (HTTPS + WSS) ไม่มี plaintext
    pub port: u16,
    /// พอร์ต UDP สำหรับ beacon / probe
    pub discovery_port: u16,
    /// พอร์ต HTTP บน 127.0.0.1 สำหรับหน้าจัดการบนเครื่องนี้เอง (/api/v1/local/*) — ไม่เปิดออกเครือข่าย
    pub local_port: u16,
    /// ที่เก็บ host key, device id และรายการอุปกรณ์ที่จับคู่แล้ว
    pub data_dir: PathBuf,
    /// ชื่อที่แสดงตอนค้นหา (โดยปริยายคือ hostname)
    pub name: String,
    /// โฟลเดอร์ที่ transfer/files เข้าถึงได้ (ว่าง = ไม่จำกัด) — เทียบหลัง canonicalize เสมอ
    pub file_roots: Vec<PathBuf>,
}

impl Config {
    pub fn ports(&self) -> Ports {
        Ports {
            port: self.port,
            discovery_port: self.discovery_port,
            local_port: self.local_port,
        }
    }

    /// ค่าตั้งของ service: ข้อมูลอยู่ใน %ProgramData%\SynaptaConnect (ใช้ร่วมกับ panel/ตัวติดตั้ง) พอร์ตเป็นค่าเริ่มต้นเสมอ
    pub fn for_service() -> Config {
        Config {
            mode: Mode::Service,
            data_dir: service_data_dir(),
            ..Config::default()
        }
    }

    /// `--port N --discovery-port N --local-port N --data-dir PATH --name TEXT --file-root PATH (ซ้ำได้)`
    pub fn from_flags(args: impl IntoIterator<Item = String>) -> Result<Config, String> {
        let mut cfg = Config::default();
        let mut args = args.into_iter();
        while let Some(flag) = args.next() {
            let value = args.next().ok_or(format!("{flag} ต้องมีค่าตามหลัง"))?;
            let number = |name: &str| value.parse::<u16>().map_err(|_| format!("{name} ไม่ใช่ตัวเลข"));
            match flag.as_str() {
                "--port" => cfg.port = number("--port")?,
                "--discovery-port" => cfg.discovery_port = number("--discovery-port")?,
                "--local-port" => cfg.local_port = number("--local-port")?,
                "--data-dir" => cfg.data_dir = value.into(),
                "--name" => cfg.name = value,
                "--file-root" => cfg.file_roots.push(value.into()),
                other => return Err(format!("ไม่รู้จักตัวเลือก {other}")),
            }
        }
        cfg.ports().validate()?;
        Ok(cfg)
    }
}

impl Default for Config {
    fn default() -> Self {
        let ports = Ports::default();
        Config {
            mode: Mode::Console,
            port: ports.port,
            discovery_port: ports.discovery_port,
            local_port: ports.local_port,
            data_dir: default_data_dir(),
            name: sysinfo::System::host_name().unwrap_or_else(|_| "unknown-host".into()),
            file_roots: Vec::new(),
        }
    }
}

fn default_data_dir() -> PathBuf {
    // Windows: %LOCALAPPDATA% (ของ SYSTEM คือ systemprofile), Unix: ~/.local/share
    let base = std::env::var_os("LOCALAPPDATA")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".local/share")))
        .unwrap_or_else(|| PathBuf::from("."));
    base.join("SynaptaConnect")
}

/// ข้อมูลของ service (ProgramData) — ตัวติดตั้ง, panel และ service ต้องเห็นที่เดียวกัน
pub fn service_data_dir() -> PathBuf {
    std::env::var_os("ProgramData")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(r"C:\ProgramData"))
        .join("SynaptaConnect")
}
