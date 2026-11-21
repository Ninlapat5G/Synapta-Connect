//! Windows service: จุดเข้าของ SCM, การเรียกตัวติดตั้ง และ log ของ service
//!
//! สอง service จาก exe ตัวเดียว (ดู docs/SERVICE_AND_UPDATE.md):
//!   SynaptaConnect         — agent (บัญชีเสมือน NT SERVICE\SynaptaConnect สิทธิ์ต่ำ, เริ่มเองตอนบูต, restart เองเมื่อล้ม)
//!   SynaptaConnectUpdater  — ตัวเปลี่ยนรุ่นแบบ SYSTEM ไม่มี listener: ตื่นตอนบูต (กู้ที่ค้าง) และเมื่อ agent เรียก แล้วจบเอง
//! ไฟล์: mod.rs (ไฟล์นี้) · host.rs (สั่ง/ถาม service จริง) · install.rs (ติดตั้ง/ถอน/สถานะ)

mod host;
pub mod install;

use crate::{
    config::{Config, service_data_dir},
    update::{
        Trigger,
        installer::{self, Layout, Timing},
        manifest,
    },
};
pub use host::{SystemHost, http_health};

pub fn start_agent() -> Result<(), String> {
    host::start_service(AGENT_SERVICE)
}

pub fn stop_agent() -> Result<(), String> {
    host::stop_service(AGENT_SERVICE)
}
use std::{
    ffi::OsString,
    path::{Path, PathBuf},
    sync::mpsc,
    time::Duration,
};
use windows_service::{
    define_windows_service,
    service::{ServiceControl, ServiceControlAccept, ServiceExitCode, ServiceState, ServiceStatus, ServiceType},
    service_control_handler::{self, ServiceControlHandlerResult, ServiceStatusHandle},
    service_dispatcher,
};

pub const AGENT_SERVICE: &str = "SynaptaConnect";
pub const UPDATER_SERVICE: &str = "SynaptaConnectUpdater";
/// บัญชีเสมือนของ agent — ไม่มีรหัสผ่าน ไม่ใช่ admin ไม่ใช่ SYSTEM เข้าถึงได้เฉพาะที่ถูกให้สิทธิ์ชื่อนี้
pub const AGENT_ACCOUNT: &str = r"NT SERVICE\SynaptaConnect";

/// agent เรียกตัวติดตั้งผ่าน SCM (สิทธิ์ START บน service นั้นถูกให้เฉพาะบัญชีของ agent ตอนติดตั้ง)
pub struct ServiceTrigger;

impl Trigger for ServiceTrigger {
    fn start_installer(&self) -> Result<(), String> {
        host::start_service(UPDATER_SERVICE)
    }
}

/// ตำแหน่งไฟล์ของระบบที่ติดตั้งแล้ว: โปรแกรมอยู่ข้าง exe ที่กำลังรัน, ข้อมูลใน ProgramData, พื้นที่ตัวติดตั้งแยกต่างหาก
pub fn system_layout() -> Layout {
    let exe = std::env::current_exe().unwrap_or_default();
    let data_dir = service_data_dir();
    Layout {
        install_dir: exe.parent().map(Path::to_path_buf).unwrap_or_default(),
        work_dir: data_dir.join("updater"),
        data_dir,
    }
}

// ---------- จุดเข้าของ SCM ----------

define_windows_service!(ffi_agent_main, agent_main);
define_windows_service!(ffi_updater_main, updater_main);

/// เรียกจาก `synapta-connect service run` — คืนเมื่อ service หยุด
pub fn run_agent_service() -> Result<(), windows_service::Error> {
    service_dispatcher::start(AGENT_SERVICE, ffi_agent_main)
}

/// เรียกจาก `synapta-connect updater run`
pub fn run_updater_service() -> Result<(), windows_service::Error> {
    service_dispatcher::start(UPDATER_SERVICE, ffi_updater_main)
}

fn report(handle: &ServiceStatusHandle, state: ServiceState, accept: ServiceControlAccept, exit: u32) {
    let _ = handle.set_service_status(ServiceStatus {
        service_type: ServiceType::OWN_PROCESS,
        current_state: state,
        controls_accepted: accept,
        exit_code: ServiceExitCode::Win32(exit),
        checkpoint: 0,
        wait_hint: Duration::from_secs(30),
        process_id: None,
    });
}

fn agent_main(_args: Vec<OsString>) {
    let config = Config::for_service();
    init_file_log(&config.data_dir, "agent.log");
    let (stop_tx, stop_rx) = mpsc::channel::<()>();
    let handle = match service_control_handler::register(AGENT_SERVICE, move |event| match event {
        ServiceControl::Stop | ServiceControl::Shutdown => {
            let _ = stop_tx.send(());
            ServiceControlHandlerResult::NoError
        }
        ServiceControl::Interrogate => ServiceControlHandlerResult::NoError,
        _ => ServiceControlHandlerResult::NotImplemented,
    }) {
        Ok(h) => h,
        Err(e) => return tracing::error!("ลงทะเบียน service handler ไม่ได้: {e}"),
    };
    report(&handle, ServiceState::StartPending, ServiceControlAccept::empty(), 0);

    let runtime = match tokio::runtime::Runtime::new() {
        Ok(r) => r,
        Err(e) => {
            tracing::error!("สร้าง runtime ไม่ได้: {e}");
            return report(&handle, ServiceState::Stopped, ServiceControlAccept::empty(), 1);
        }
    };
    let _ = rustls::crypto::ring::default_provider().install_default();
    crate::platform::init();
    report(
        &handle,
        ServiceState::Running,
        ServiceControlAccept::STOP | ServiceControlAccept::SHUTDOWN,
        0,
    );

    let result = runtime.block_on(crate::run_until(config, async {
        let _ = tokio::task::spawn_blocking(move || stop_rx.recv()).await;
    }));
    let exit = match result {
        Ok(()) => 0,
        Err(e) => {
            tracing::error!("agent หยุดทำงาน: {e:#}");
            1
        }
    };
    report(&handle, ServiceState::Stopped, ServiceControlAccept::empty(), exit);
}

fn updater_main(_args: Vec<OsString>) {
    let layout = system_layout();
    init_file_log(&layout.work_dir, "updater.log");
    // ไม่รับคำสั่งหยุดระหว่างทำงาน: การหยุดกลางการสลับไฟล์คือสิ่งที่ journal ไว้กู้ แต่ไม่ควรเกิดโดยตั้งใจ
    let handle = match service_control_handler::register(UPDATER_SERVICE, |event| match event {
        ServiceControl::Interrogate => ServiceControlHandlerResult::NoError,
        _ => ServiceControlHandlerResult::NotImplemented,
    }) {
        Ok(h) => h,
        Err(e) => return tracing::error!("ลงทะเบียน service handler ไม่ได้: {e}"),
    };
    report(&handle, ServiceState::Running, ServiceControlAccept::empty(), 0);

    let (host, timing) = (SystemHost, Timing::default());
    // ลำดับสำคัญ: กู้งานที่ถูกขัดจังหวะก่อน แล้วค่อยรับคำขอใหม่
    if let Some(r) = installer::recover(&layout, &host, &timing) {
        tracing::warn!(outcome = %r.outcome, version = %r.version, "กู้การอัปเดตที่ถูกขัดจังหวะ");
    }
    match manifest::embedded_pubkey() {
        Some(key) => {
            if let Some(r) = installer::apply_pending(&layout, &host, &key, &timing) {
                tracing::info!(outcome = %r.outcome, version = %r.version, error = ?r.error, "ผลการอัปเดต");
            }
        }
        None => tracing::error!("ไม่มี public key ของผู้เผยแพร่ในโปรแกรมนี้ — ไม่ติดตั้งอัปเดต"),
    }
    report(&handle, ServiceState::Stopped, ServiceControlAccept::empty(), 0);
}

/// log ลงไฟล์ (service ไม่มี console) — หมุนเมื่อเกิน 5 MB เก็บแค่ 1 ไฟล์เก่า
fn init_file_log(data_dir: &Path, name: &str) {
    let dir: PathBuf = data_dir.join("logs");
    let _ = std::fs::create_dir_all(&dir);
    let path = dir.join(name);
    if std::fs::metadata(&path).is_ok_and(|m| m.len() > 5 * 1024 * 1024) {
        let _ = std::fs::rename(&path, dir.join(format!("{name}.1")));
    }
    let Ok(file) = std::fs::OpenOptions::new().create(true).append(true).open(&path) else {
        return;
    };
    let _ = tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "synapta_connect=info".into()),
        )
        .with_ansi(false)
        .with_writer(std::sync::Mutex::new(file))
        .try_init();
}
