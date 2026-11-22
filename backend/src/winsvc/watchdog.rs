use super::{AGENT_SERVICE, SystemHost, UPDATER_SERVICE, host, install};
use crate::update::installer::{EXE_NAME, Host};
use serde::{Deserialize, Serialize};
use std::time::{Duration, SystemTime};
use windows::Win32::{
    Foundation::CloseHandle,
    System::Threading::{OpenProcess, PROCESS_TERMINATE, TerminateProcess},
};
use windows_service::service::{ServiceAccess, ServiceState};

#[derive(Default, Deserialize, Serialize)]
struct Attempts {
    pid: u32,
    misses: u8,
    restarts: u8,
    checked_at: u64,
}

impl Attempts {
    fn missed(&mut self, pid: u32, now: u64) -> bool {
        if self.pid != pid || now.saturating_sub(self.checked_at) > 180 {
            self.pid = pid;
            self.misses = 0;
        }
        if now.saturating_sub(self.checked_at) < 40 {
            return false;
        }
        self.checked_at = now;
        self.misses = self.misses.saturating_add(1);
        self.misses >= 3 && self.restarts < 3
    }
}

pub fn watchdog() -> Result<(), String> {
    let layout = super::system_layout();
    if !is_elevated::is_elevated() || layout.install_dir != install::install_dir() {
        return Err("ให้ Windows เรียก health monitor จากโปรแกรมที่ติดตั้งไว้เท่านั้น".into());
    }
    if std::env::current_exe().as_deref().ok() != Some(layout.install_dir.join(EXE_NAME).as_path()) {
        return Err("ตำแหน่งโปรแกรม health monitor ไม่ถูกต้อง".into());
    }
    super::init_file_log(&layout.work_dir, "watchdog.log");
    let agent = host::open(AGENT_SERVICE, ServiceAccess::QUERY_STATUS)?;
    let updater = host::open(UPDATER_SERVICE, ServiceAccess::QUERY_STATUS)?;
    let running_pid = || {
        let status = agent.query_status().ok()?;
        (status.current_state == ServiceState::Running && host::state_of(&updater).ok()? == ServiceState::Stopped)
            .then_some(status.process_id)
            .flatten()
    };
    let Some(pid) = running_pid() else { return Ok(()) };
    let record = layout.work_dir.join("watchdog-state.json");
    if SystemHost.health().is_ok_and(|health| health.pid == pid) {
        let _ = std::fs::remove_file(&record);
        return Ok(());
    }
    if running_pid() != Some(pid) {
        return Ok(());
    }
    let now = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap_or(Duration::ZERO)
        .as_secs();
    let mut attempts: Attempts = std::fs::read(&record)
        .ok()
        .and_then(|bytes| serde_json::from_slice(&bytes).ok())
        .unwrap_or_default();
    let restart = attempts.missed(pid, now);
    crate::fsutil::write_json(&record, &attempts).map_err(|error| error.to_string())?;
    if !restart {
        return Ok(());
    }
    let process = unsafe { OpenProcess(PROCESS_TERMINATE, false, pid) }.map_err(|error| error.to_string())?;
    let result = (|| {
        if running_pid() != Some(pid) {
            return Ok(());
        }
        attempts.restarts += 1;
        attempts.misses = 0;
        crate::fsutil::write_json(&record, &attempts).map_err(|error| error.to_string())?;
        tracing::warn!(
            pid,
            attempt = attempts.restarts,
            "Agent did not answer three health checks; asking SCM to recover"
        );
        unsafe { TerminateProcess(process, 1) }.map_err(|error| error.to_string())
    })();
    unsafe {
        let _ = CloseHandle(process);
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn restart_requires_three_spaced_failures_and_respects_the_recovery_limit() {
        let mut state = Attempts::default();
        assert!(!state.missed(12, 1000));
        assert!(!state.missed(12, 1010));
        assert!(!state.missed(12, 1060));
        assert!(state.missed(12, 1120));
        assert!(!state.missed(13, 1180));
        state.restarts = 3;
        assert!(!state.missed(13, 1240));
        assert!(!state.missed(13, 1300));
    }
}
