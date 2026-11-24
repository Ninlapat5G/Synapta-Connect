//! สเปกเครื่อง + ระดับสิทธิ์ของโปรเซสที่รัน agent

use crate::{netif, rpc::Res};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    sync::atomic::{AtomicBool, Ordering},
    time::Duration,
};
use sysinfo::{
    Disks, MINIMUM_CPU_UPDATE_INTERVAL, Pid, ProcessRefreshKind, ProcessesToUpdate, System, UpdateKind, Users,
};

#[derive(Deserialize, Default)]
pub struct Params {
    /// ส่ง environment variables ของ agent ด้วย (อาจมีความลับ จึงไม่ส่งโดยปริยาย)
    #[serde(default)]
    include_env: bool,
}

#[derive(Serialize)]
pub struct Privilege {
    /// standard | administrator | system | service (Windows) / standard | root (Unix)
    pub level: &'static str,
    pub user: String,
}

pub async fn get(p: Params) -> Res<Value> {
    // sysinfo ต้องวัด CPU สองรอบห่างกันถึงจะได้ค่า usage — ทำใน thread แยกไม่ให้บล็อก runtime
    let mut profile = tokio::task::spawn_blocking(collect).await??;
    if p.include_env {
        profile["env"] = json!(masked_env(std::env::vars()));
    }
    Ok(profile)
}

/// ค่าที่ปลอดภัยจะส่งตามจริง ที่เหลือส่งแค่ชื่อพร้อม "***" — ผลลัพธ์นี้ไปถึง LLM และ transcript ได้
/// ถ้ารั่ว API key / รหัสผ่านใน env จะหลุดไปด้วย (ต้องการค่าจริงให้ใช้ exec.run อ่านเองโดยรู้ตัว)
fn masked_env(vars: impl Iterator<Item = (String, String)>) -> BTreeMap<String, String> {
    const SAFE: [&str; 6] = ["PATH", "USERPROFILE", "HOME", "SHELL", "LANG", "OS"];
    vars.map(|(k, v)| {
        let shown = if SAFE.contains(&k.to_uppercase().as_str()) {
            v
        } else {
            "***".into()
        };
        (k, shown)
    })
    .collect()
}

fn collect() -> Res<Value> {
    let mut sys = System::new()?;
    sys.refresh_cpu_all();
    sys.refresh_memory();
    std::thread::sleep(MINIMUM_CPU_UPDATE_INTERVAL);
    sys.refresh_cpu_usage();

    let disks = disks_with_timeout();

    Ok(json!({
        "hostname": System::host_name().ok(),
        "os": {"name": System::name().ok(), "version": System::long_os_version().ok(),
               "kernel": System::kernel_version().ok(), "arch": System::cpu_arch()},
        "cpu": {"model": sys.cpus().first().map(|c| c.brand().to_string()),
                "physical_cores": System::physical_core_count().ok(),
                "logical_cores": sys.cpus().len(),
                "usage_percent": sys.global_cpu_usage()},
        "memory": {"total_bytes": sys.total_memory(), "available_bytes": sys.available_memory()},
        "disks_complete": disks.is_some(),
        "disks": disks.unwrap_or_default(),
        "network": netif::list(),
        "privilege": privilege()?,
        "agent": {"version": env!("CARGO_PKG_VERSION"), "pid": std::process::id(),
                  "mode": if is_service_account() { "service" } else { "console" }},
    }))
}

/// How long `system.profile` waits for the disk list before replying without it.
const DISK_TIMEOUT: Duration = Duration::from_secs(3);
/// A disk probe thread is still running (possibly stuck forever on a hung drive).
static DISK_PROBE_RUNNING: AtomicBool = AtomicBool::new(false);

/// Disk list, or None when it did not finish in time.
///
/// On app01 a USB stick hung so that GetVolumeInformationW on its drive never returned (even with
/// SEM_FAILCRITICALERRORS, and the thread could not be killed). The probe ran inline, so every `system.profile`
/// call (connect_info) hung until the client timed out and leaked one more stuck thread. Now the probe runs in its
/// own thread with a deadline, and no new probe starts while an old one is still stuck.
fn disks_with_timeout() -> Option<Vec<Value>> {
    if DISK_PROBE_RUNNING.swap(true, Ordering::AcqRel) {
        return None;
    }
    let (sender, receiver) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let disks = list_disks();
        DISK_PROBE_RUNNING.store(false, Ordering::Release);
        let _ = sender.send(disks);
    });
    receiver.recv_timeout(DISK_TIMEOUT).ok().flatten()
}

fn list_disks() -> Option<Vec<Value>> {
    let disks = Disks::new_with_refreshed_list().ok()?;
    Some(
        disks
            .iter()
            .map(|d| {
                json!({"name": d.name().to_string_lossy(), "mount": d.mount_point(),
                       "file_system": d.file_system().to_string_lossy(),
                       "total_bytes": d.total_space(), "available_bytes": d.available_space(),
                       "removable": d.is_removable()})
            })
            .collect(),
    )
}

pub fn privilege() -> Res<Privilege> {
    let mut sys = System::new()?;
    let pid = Pid::from_u32(std::process::id());
    sys.refresh_processes_specifics(
        ProcessesToUpdate::Some(&[pid]),
        true,
        ProcessRefreshKind::nothing().with_user(UpdateKind::OnlyIfNotSet),
    );
    let uid = sys.process(pid).and_then(|p| p.user_id());
    let users = Users::new_with_refreshed_list()?;
    let user = uid
        .and_then(|u| users.get_user_by_id(u))
        .map(|u| u.name().to_string())
        .or_else(|| std::env::var("USERNAME").or_else(|_| std::env::var("USER")).ok())
        .unwrap_or_else(|| "unknown".into());

    // บัญชีเสมือนของ service (`NT SERVICE\SynaptaConnect`): Windows ให้ token แบบไม่แยก UAC จึงถูกนับว่า
    // "elevated" ทั้งที่ไม่ใช่ผู้ดูแล — ต้องแยกก่อน ไม่งั้นรายงานเป็น administrator ผิดความจริง (เจอบน Server 2016)
    if is_service_account() {
        return Ok(Privilege {
            level: "service",
            user: crate::platform::account().unwrap_or(user),
        });
    }
    // S-1-5-18 คือ SID ของบัญชี SYSTEM บน Windows
    let is_system = uid.is_some_and(|u| u.to_string() == "S-1-5-18");
    let level = match (is_system, is_elevated::is_elevated(), cfg!(windows)) {
        (true, _, _) => "system",
        (false, true, true) => "administrator",
        (false, true, false) => "root",
        _ => "standard",
    };
    Ok(Privilege { level, user })
}

/// agent รันด้วยบัญชีเสมือนของ Windows service (`NT SERVICE\...`)
fn is_service_account() -> bool {
    static SERVICE: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *SERVICE
        .get_or_init(|| crate::platform::account().is_some_and(|name| name.to_lowercase().starts_with("nt service\\")))
}

#[cfg(test)]
mod tests {
    use super::masked_env;

    #[test]
    fn env_hides_everything_except_the_safe_list() {
        let vars = [("Path", "C:\\bin"), ("OPENAI_API_KEY", "sk-secret"), ("LANG", "th_TH")]
            .into_iter()
            .map(|(k, v)| (k.to_string(), v.to_string()));
        let env = masked_env(vars);
        assert_eq!(env["Path"], "C:\\bin");
        assert_eq!(env["LANG"], "th_TH");
        assert_eq!(env["OPENAI_API_KEY"], "***");
    }
}
