//! Windows service: จุดเข้าของ SCM, การเรียกตัวติดตั้ง และ log ของ service
//!
//! สอง service จาก exe ตัวเดียว (ดู docs/SERVICE_AND_UPDATE.md):
//!   SynaptaConnect         — agent (บัญชีเสมือน NT SERVICE\SynaptaConnect สิทธิ์ต่ำ, เริ่มเองตอนบูต, restart เองเมื่อล้ม)
//!   SynaptaConnectUpdater  — ตัวเปลี่ยนรุ่นแบบ SYSTEM ไม่มี listener: ตื่นตอนบูต (กู้ที่ค้าง) และเมื่อ agent เรียก แล้วจบเอง
//! ไฟล์: mod.rs (ไฟล์นี้) · host.rs (สั่ง/ถาม service จริง) · install.rs (ติดตั้ง/ถอน/สถานะ)

mod host;
pub mod install;
mod watchdog;
pub use watchdog::watchdog;

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

pub fn desktop_status() -> serde_json::Value {
    use windows_service::service::ServiceAccess;
    let state = |name| match host::service_exists(name) {
        Ok(false) => "NotInstalled".into(),
        Err(_) => "Unavailable".into(),
        Ok(true) => host::open(name, ServiceAccess::QUERY_STATUS)
            .and_then(|service| host::state_of(&service))
            .map(|state| format!("{state:?}"))
            .unwrap_or_else(|_| "Unavailable".into()),
    };
    serde_json::json!({
        "agent": state(AGENT_SERVICE),
        "updater": state(UPDATER_SERVICE),
        "can_start": host::open(AGENT_SERVICE, ServiceAccess::START | ServiceAccess::QUERY_STATUS).is_ok(),
        "can_stop": host::open(AGENT_SERVICE, ServiceAccess::STOP | ServiceAccess::QUERY_STATUS).is_ok(),
        "account": crate::platform::account(),
        "desktop_version": env!("CARGO_PKG_VERSION"),
    })
}

pub fn desktop_control(action: &str) -> Result<(), String> {
    use windows_service::service::ServiceAccess;
    if let Ok(service) = host::open(UPDATER_SERVICE, ServiceAccess::QUERY_STATUS)
        && host::state_of(&service)? != ServiceState::Stopped
    {
        return Err("กำลังใช้ตัวติดตั้งอัปเดต กรุณารอให้ทำงานเสร็จก่อน".into());
    }
    match action {
        "start" => start_agent(),
        "stop" => stop_agent(),
        "restart" => {
            host::open(
                AGENT_SERVICE,
                ServiceAccess::START | ServiceAccess::STOP | ServiceAccess::QUERY_STATUS,
            )?;
            stop_agent()?;
            start_agent()
        }
        _ => return Err("คำสั่งควบคุม Agent ไม่ถูกต้อง".into()),
    }?;
    tracing::info!(%action, account = ?crate::platform::account(), "Desktop service control");
    Ok(())
}

pub fn firewall_ready(data_dir: &Path) -> bool {
    data_dir.join("updater-status").join(install::FIREWALL_MARKER).is_file()
}

pub async fn migrate_firewall(data_dir: PathBuf) {
    for _ in 0..6 {
        if firewall_ready(&data_dir) && install::recovery_ready(&data_dir) {
            return;
        }
        tokio::time::sleep(Duration::from_secs(15)).await;
        if let Err(error) = tokio::task::spawn_blocking(|| host::start_service(UPDATER_SERVICE))
            .await
            .unwrap_or_else(|e| Err(e.to_string()))
        {
            tracing::warn!("เริ่มตัวอัปเดตเพื่อปรับกฎไฟร์วอลล์ไม่ได้: {error}");
        }
    }
}
use std::{
    ffi::OsString,
    path::{Path, PathBuf},
    sync::Arc,
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
    let stop = Arc::new(tokio::sync::Notify::new());
    let signal = stop.clone();
    let handle = match service_control_handler::register(AGENT_SERVICE, move |event| match event {
        ServiceControl::Stop | ServiceControl::Shutdown => {
            signal.notify_one();
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
    crate::platform::init();
    report(
        &handle,
        ServiceState::Running,
        ServiceControlAccept::STOP | ServiceControlAccept::SHUTDOWN,
        0,
    );

    let result = runtime.block_on(crate::run_until(config, stop.notified()));
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
    if !firewall_ready(&layout.data_dir)
        && let Err(error) = install::configure_firewall(&layout)
    {
        tracing::error!("ปรับกฎไฟร์วอลล์ไม่ได้: {error}");
    }
    if !install::recovery_ready(&layout.data_dir)
        && let Err(error) = install::configure_recovery(&layout.install_dir, &layout.data_dir)
    {
        tracing::warn!(%error, "ตั้งค่า recovery ของ Agent ไม่สำเร็จ");
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
