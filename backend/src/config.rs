//! ค่าตั้งต้นของ agent — อ่านจาก command line เท่านั้น (ไม่มีไฟล์ config)

use std::path::PathBuf;

/// console = เปิดมือในเทอร์มินัล (พฤติกรรม Phase 0 เดิม) · service = รันเป็น Windows service เบื้องหลัง
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    Console,
    Service,
}

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
        Ok(cfg)
    }
}

impl Default for Config {
    fn default() -> Self {
        Config {
            mode: Mode::Console,
            port: 52401,
            discovery_port: 52400,
            local_port: 52402,
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
