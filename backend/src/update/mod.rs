//! การอัปเดตอัตโนมัติ — ฝั่ง agent (สิทธิ์ต่ำ): ตรวจ release, ดาวน์โหลด, ตรวจ, ส่งให้ตัวติดตั้ง, สรุปผลให้ panel
//!
//! agent ไม่เปลี่ยนไฟล์โปรแกรมเอง (ไม่มีสิทธิ์) — วางแพ็กเกจใน staging แล้วเรียก service ตัวติดตั้ง (installer.rs)
//! ซึ่งตรวจซ้ำทั้งหมดและเป็นผู้สลับไฟล์/กู้คืน การตรวจฝั่ง agent มีไว้ให้ตอบ panel เร็วและไม่ส่งของเสียไปให้ตัวติดตั้ง
//!
//! ไฟล์ในโฟลเดอร์นี้: source (เอาไฟล์จากไหน) · manifest (ลายเซ็น) · version · state (กฎลองใหม่) · history ·
//! installer (ฝั่งสิทธิ์สูง) · error (code ที่ panel เห็น) · ไฟล์นี้ = ตัวประสาน + scheduler

pub mod error;
pub mod history;
pub mod installer;
pub mod manifest;
pub mod source;
pub mod state;
pub mod version;

use crate::{auth::now_secs, fsutil, settings::SettingsStore};
use error::UpdateError;
use history::{Entry, History};
use installer::{Request, UpdateResult};
use serde_json::{Value, json};
use source::Source;
use state::{Available, MAX_ATTEMPTS, Pending, StateFile, UpdateState};
use std::{
    path::PathBuf,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};
use version::Version;

const MANUAL_CHECK_COOLDOWN: Duration = Duration::from_secs(15);
/// ส่งให้ตัวติดตั้งแล้วเงียบเกินนี้ = ตัวติดตั้งตายกลางทาง (ปกติเสร็จใน ~1 นาที)
const PENDING_TIMEOUT_SECS: u64 = 15 * 60;

/// ตัวเรียกตัวติดตั้งสิทธิ์สูง — Windows service จริงอยู่ที่ winsvc/, เทสต์ใช้ของปลอม
pub trait Trigger: Send + Sync {
    fn start_installer(&self) -> Result<(), String>;
}

/// โหมด console ไม่มีตัวติดตั้ง — ตรวจ release ได้แต่ติดตั้งไม่ได้
pub struct NoInstaller;

impl Trigger for NoInstaller {
    fn start_installer(&self) -> Result<(), String> {
        Err("ไม่ได้ติดตั้งเป็น service จึงไม่มีตัวติดตั้งอัตโนมัติ (ติดตั้งด้วย synapta-connect install)".into())
    }
}

pub struct UpdateEnv {
    /// None = build นี้ไม่มี public key ของผู้เผยแพร่ → ติดตั้งอัปเดตไม่ได้
    pub pubkey: Option<[u8; 32]>,
    pub trigger: Arc<dyn Trigger>,
    pub current: Version,
}

/// ปฏิเสธการสั่งที่ทำไม่ได้ ณ ตอนนี้ (panel แปลงเป็น 4xx)
#[derive(Debug, PartialEq)]
pub enum Refused {
    Busy,
    RateLimited,
    /// เหตุผลภาษาไทย
    Invalid(String),
}

/// งานที่ผ่านด่านตรวจแล้ว ถือ "ประตู" ไว้จนจบ — ทีละงานเท่านั้น
pub struct CheckJob {
    manual: bool,
    _gate: tokio::sync::OwnedMutexGuard<()>,
}

pub struct ApplyJob {
    version: String,
    _gate: tokio::sync::OwnedMutexGuard<()>,
}

pub struct UpdateManager {
    env: UpdateEnv,
    settings: Arc<SettingsStore>,
    data_dir: PathBuf,
    history: History,
    state_file: StateFile,
    state: Mutex<UpdateState>,
    /// ขั้นที่กำลังทำ (checking/downloading/verifying) — ไม่เก็บลงดิสก์
    activity: Mutex<Option<&'static str>>,
    busy: Arc<tokio::sync::Mutex<()>>,
    last_manual_check: Mutex<Option<Instant>>,
}

impl UpdateManager {
    pub fn new(env: UpdateEnv, settings: Arc<SettingsStore>, data_dir: PathBuf) -> UpdateManager {
        let state_file = StateFile::new(&data_dir);
        UpdateManager {
            state: Mutex::new(state_file.load()),
            history: History::new(&data_dir),
            state_file,
            env,
            settings,
            data_dir,
            activity: Mutex::new(None),
            busy: Arc::new(tokio::sync::Mutex::new(())),
            last_manual_check: Mutex::new(None),
        }
    }

    fn current(&self) -> String {
        self.env.current.to_string()
    }

    fn source(&self) -> Result<Source, UpdateError> {
        let text = self.settings.get().effective_source();
        let not_configured = |m: &str| UpdateError::new("check", "not_configured", m);
        if self.env.pubkey.is_none() {
            return Err(not_configured(
                "โปรแกรมรุ่นนี้ยังไม่มี public key ของผู้เผยแพร่ จึงตรวจลายเซ็น/ติดตั้งอัปเดตไม่ได้",
            ));
        }
        if text.is_empty() {
            return Err(not_configured("ยังไม่ได้ตั้งแหล่ง release สำหรับอัปเดต"));
        }
        Source::parse(&text).map_err(|e| not_configured(&e))
    }

    fn save(&self, change: impl FnOnce(&mut UpdateState)) {
        let mut st = self.state.lock().unwrap();
        change(&mut st);
        self.state_file.save(&st);
    }

    fn set_activity(&self, a: Option<&'static str>) {
        *self.activity.lock().unwrap() = a;
    }

    // ---------- สถานะสำหรับ panel ----------

    pub fn status(&self) -> Value {
        let st = self.state.lock().unwrap().clone();
        let settings = self.settings.get();
        let source = self.source();
        let activity = *self.activity.lock().unwrap();
        let state = if let Some(a) = activity {
            a
        } else if st.pending.is_some() {
            "applying"
        } else if let Some(e) = &st.last_error {
            match e.code.as_str() {
                "health_check_failed" | "service_start_failed" | "interrupted_rolled_back" => "rolled_back",
                _ => "failed",
            }
        } else if st.available.is_some() {
            "update_available"
        } else if source.as_ref().is_err_and(|e| e.code == "not_configured") {
            "not_configured"
        } else if !settings.update.auto_update {
            "disabled"
        } else if st.last_check_at.is_some() {
            "up_to_date"
        } else {
            "idle"
        };
        json!({
            "current_version": self.current(),
            "state": state,
            "auto_update": settings.update.auto_update,
            "source": settings.effective_source(),
            "last_check_at": st.last_check_at,
            "available": st.available,
            "last_error": st.last_error.as_ref().map(|e| json!({
                "code": e.code, "message": e.message, "stage": e.stage,
                "version": st.target, "at": st.last_error_at,
            })),
            "attempts": st.attempts, "max_attempts": MAX_ATTEMPTS, "next_retry_at": st.next_attempt_at,
            "can_retry": st.available.is_some() && st.last_error.as_ref().is_some_and(|e| e.stage != "check") && st.pending.is_none(),
            "history": self.history.recent(20),
        })
    }

    pub fn history(&self, limit: usize) -> Vec<Entry> {
        self.history.recent(limit.min(200))
    }

    pub fn set_settings(&self, auto_update: bool, source: Option<String>) -> std::io::Result<()> {
        self.settings
            .update(|s| {
                s.update.auto_update = auto_update;
                if let Some(src) = source {
                    s.update.source = src;
                }
            })
            .map(|_| ())
    }

    // ---------- การกระทำ ----------

    /// ตรวจ release ล่าสุด (ไม่ติดตั้ง) — สองจังหวะ: begin_* ปฏิเสธทันทีถ้าทำไม่ได้, run_* ทำงานจริง
    /// (แยกกันเพื่อให้ panel ได้ 429/409 ทันที แล้วปล่อยงานยาวไปทำเบื้องหลัง)
    pub fn begin_check(&self, manual: bool) -> Result<CheckJob, Refused> {
        if manual {
            let mut last = self.last_manual_check.lock().unwrap();
            if last.is_some_and(|t| t.elapsed() < MANUAL_CHECK_COOLDOWN) {
                return Err(Refused::RateLimited);
            }
            *last = Some(Instant::now());
        }
        let gate = self.busy.clone().try_lock_owned().map_err(|_| Refused::Busy)?;
        self.set_activity(Some("checking"));
        Ok(CheckJob { manual, _gate: gate })
    }

    pub async fn check(&self, manual: bool) -> Result<(), Refused> {
        let job = self.begin_check(manual)?;
        self.run_check(job).await;
        Ok(())
    }

    pub async fn run_check(&self, job: CheckJob) {
        let manual = job.manual;
        let outcome = self.fetch_verified().await;
        self.set_activity(None);

        let current = self.env.current;
        match outcome {
            Ok((m, _, _)) => {
                let newer = m.version() > current;
                let was_known = self
                    .state
                    .lock()
                    .unwrap()
                    .available
                    .as_ref()
                    .is_some_and(|a| a.version == m.version);
                self.save(|st| {
                    st.last_check_at = Some(now_secs());
                    if st.last_error.as_ref().is_some_and(|e| e.stage == "check") {
                        st.last_error = None; // ตรวจผ่านแล้ว ความผิดจากการตรวจครั้งก่อนไม่เกี่ยวอีก
                    }
                    st.available = newer.then(|| Available {
                        version: m.version.clone(),
                        notes: m.notes.clone(),
                        published_at: m.published_at.clone(),
                    });
                });
                if manual || !was_known {
                    let outcome = if newer { "available" } else { "up_to_date" };
                    self.history.append(&Entry::new(
                        &self.current(),
                        newer.then_some(m.version.as_str()),
                        outcome,
                    ));
                }
            }
            Err(e) if e.code == "not_configured" => {} // สถานะ not_configured แสดงเองจากค่าตั้ง ไม่ใช่ความล้มเหลว
            Err(e) => {
                let changed = self.state.lock().unwrap().last_error.as_ref() != Some(&e);
                self.save(|st| {
                    st.last_check_at = Some(now_secs());
                    st.last_error = Some(e.clone());
                    st.last_error_at = Some(now_secs());
                });
                if manual || changed {
                    self.history
                        .append(&Entry::new(&self.current(), None, "failed").with_error(&e, 0));
                }
            }
        }
    }

    /// ดาวน์โหลด → ตรวจ → ส่งให้ตัวติดตั้ง; `manual` = เจ้าของสั่ง (เริ่มนับครั้งใหม่)
    pub fn begin_apply(&self, version: &str, manual: bool) -> Result<ApplyJob, Refused> {
        let gate = self.busy.clone().try_lock_owned().map_err(|_| Refused::Busy)?;
        {
            let st = self.state.lock().unwrap();
            if st.pending.is_some() {
                return Err(Refused::Busy);
            }
            if st.available.as_ref().is_none_or(|a| a.version != version) {
                return Err(Refused::Invalid(format!(
                    "ไม่มี release รุ่น {version} ให้ติดตั้ง — ตรวจอัปเดตก่อน"
                )));
            }
            if !manual && !st.may_auto_try(version) {
                return Err(Refused::Invalid("รุ่นนี้ลองครบเพดานหรือยังไม่ถึงเวลาลองใหม่".into()));
            }
        }
        if manual {
            self.save(|st| st.reset_for_manual(version));
        }
        self.set_activity(Some("downloading"));
        Ok(ApplyJob {
            version: version.into(),
            _gate: gate,
        })
    }

    pub async fn apply(&self, version: &str, manual: bool) -> Result<(), Refused> {
        let job = self.begin_apply(version, manual)?;
        self.run_apply(job).await;
        Ok(())
    }

    pub async fn run_apply(&self, job: ApplyJob) {
        let version = job.version.as_str();
        let outcome = self.stage(version).await;
        self.set_activity(None);
        if let Err(e) = outcome {
            let _ = std::fs::remove_file(self.data_dir.join("settings.pre-update.json"));
            self.save(|st| st.record_failure(version, e.clone()));
            let attempt = self.state.lock().unwrap().attempts;
            self.history
                .append(&Entry::new(&self.current(), Some(version), "failed").with_error(&e, attempt));
        }
    }

    async fn fetch_verified(&self) -> Result<(manifest::Manifest, Vec<u8>, String), UpdateError> {
        let source = self.source()?;
        let fetched = source.fetch_manifest().await?;
        let pubkey = self.env.pubkey.expect("source() ตรวจแล้ว");
        let m = manifest::verify(&fetched.manifest, &fetched.signature, &pubkey)?;
        Ok((m, fetched.manifest, fetched.signature))
    }

    /// ขั้นดาวน์โหลด+ตรวจ+วางใน staging+เรียกตัวติดตั้ง
    async fn stage(&self, version: &str) -> Result<(), UpdateError> {
        let (m, manifest_bytes, signature) = self.fetch_verified().await?;
        if m.version != version {
            return Err(UpdateError::new(
                "check",
                "manifest_invalid",
                "release ล่าสุดเปลี่ยนรุ่นระหว่างทาง — ตรวจอัปเดตใหม่",
            ));
        }
        if m.version() <= self.env.current {
            return Err(UpdateError::new(
                "verify",
                "version_not_newer",
                "รุ่นที่เสนอไม่ใหม่กว่ารุ่นที่ติดตั้ง",
            ));
        }
        let io_err =
            |e: std::io::Error| UpdateError::new("download", "download_failed", format!("เขียน staging ไม่ได้: {e}"));
        let dir = self.data_dir.join("staging").join(version);
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).map_err(io_err)?;
        let package = dir.join("package.exe");
        self.source()?.download(&m.asset, &package, m.size).await?;

        self.set_activity(Some("verifying"));
        let (size, digest) = installer::hash_file(&package).map_err(io_err)?;
        if size != m.size || digest != m.sha256.to_lowercase() {
            let _ = std::fs::remove_dir_all(&dir);
            return Err(UpdateError::new(
                "verify",
                "hash_mismatch",
                "ตัวติดตั้งที่ดาวน์โหลดไม่ตรงกับ SHA-256/ขนาดใน manifest",
            ));
        }
        std::fs::write(dir.join("manifest.json"), &manifest_bytes).map_err(io_err)?;
        std::fs::write(dir.join("manifest.json.sig"), signature).map_err(io_err)?;
        // agent สำรองค่าตั้งของตัวเองก่อนส่งคำขอ — ตัวติดตั้งไม่แตะ settings (ดู ingest_result)
        self.settings
            .save_copy(&self.data_dir.join("settings.pre-update.json"))
            .map_err(io_err)?;
        fsutil::write_json(
            &self.data_dir.join("staging").join("request.json"),
            &Request {
                version: version.into(),
                id: crate::hex::random_id(8),
            },
        )
        .map_err(io_err)?;

        self.history
            .append(&Entry::new(&self.current(), Some(version), "staged"));
        self.save(|st| {
            st.pending = Some(Pending {
                version: version.into(),
                requested_at: now_secs(),
            })
        });
        self.env.trigger.start_installer().map_err(|e| {
            self.save(|st| st.pending = None);
            let _ = std::fs::remove_file(self.data_dir.join("staging").join("request.json"));
            UpdateError::new("apply", "updater_unavailable", format!("เรียกตัวติดตั้งไม่ได้: {e}"))
        })
    }

    /// อ่านผลที่ตัวติดตั้งทิ้งไว้ แล้วสรุปเป็นสถานะของ agent — เรียกเป็นระยะ และตอนเริ่มโปรแกรม
    pub fn ingest_result(&self) {
        let updater_status = self.data_dir.join("updater-status");
        let seen = self.state.lock().unwrap().seen_result.clone();
        let fresh = fsutil::read_json::<UpdateResult>(&updater_status.join("result.json"))
            .filter(|r| seen.as_deref() != Some(&r.id));
        if let Some(r) = fresh {
            // ตัวติดตั้งกู้รุ่นเดิมแล้ว = ค่าตั้งที่ agent สำรองไว้เองก่อนส่งคำขอต้องกลับมา (รุ่นใหม่อาจแก้ค่าตั้งไปแล้ว)
            // ตัวติดตั้งไม่แตะ settings เลย — สำรอง/ใส่คืนเป็นหน้าที่ของ agent ในที่ที่ agent คุมเอง
            let backup = self.data_dir.join("settings.pre-update.json");
            if r.outcome != "installed"
                && backup.exists()
                && let Err(e) = self.settings.restore_from(&backup)
            {
                tracing::error!("ใส่ค่าตั้งเดิมคืนไม่สำเร็จ: {e}");
            }
            let _ = std::fs::remove_file(&backup);
            let _ = std::fs::remove_dir_all(self.data_dir.join("staging")); // ที่ของ agent: เก็บกวาดเอง
            self.save(|st| {
                st.pending = None;
                st.seen_result = Some(r.id.clone());
                match r.error {
                    None => st.record_success(),
                    Some(e) => st.record_failure(&r.version, e),
                }
            });
            return;
        }
        // ตัวติดตั้งไม่ตอบเลยนานผิดปกติ: เลิกรอ แจ้งสาเหตุ (ไม่เดาว่าสำเร็จ)
        let stale = self
            .state
            .lock()
            .unwrap()
            .pending
            .clone()
            .filter(|p| now_secs() > p.requested_at + PENDING_TIMEOUT_SECS);
        if let Some(p) = stale {
            let e = UpdateError::new(
                "apply",
                "updater_unavailable",
                "ตัวติดตั้งไม่ตอบ (เกิน 15 นาที) — ตรวจ service SynaptaConnectUpdater",
            );
            self.save(|st| {
                st.pending = None;
                st.record_failure(&p.version, e.clone());
            });
            self.history
                .append(&Entry::new(&self.current(), Some(&p.version), "failed").with_error(&e, 0));
        }
    }

    // ---------- ตัวจับเวลา ----------

    /// ทำงานตลอดอายุ agent (เฉพาะโหมด service): รับผลจากตัวติดตั้ง, ตรวจตามรอบ, ติดตั้งเองถ้าเปิดไว้
    pub async fn run_scheduler(self: Arc<Self>) {
        tokio::time::sleep(Duration::from_secs(120)).await; // ให้ระบบนิ่งหลังบูตก่อนใช้เครือข่าย
        let mut tick = tokio::time::interval(Duration::from_secs(5));
        loop {
            tick.tick().await;
            self.ingest_result();
            let settings = self.settings.get();
            let (last_check, available) = {
                let st = self.state.lock().unwrap();
                (st.last_check_at, st.available.clone())
            };
            let due = last_check.is_none_or(|t| now_secs() >= t + settings.update.check_interval_hours.max(1) * 3600);
            if !settings.update.auto_update || self.source().is_err() {
                continue;
            }
            if due {
                let _ = self.check(false).await;
            }
            let available = self.state.lock().unwrap().available.clone().or(available);
            if let Some(a) = available {
                let _ = self.apply(&a.version, false).await; // ปฏิเสธเองถ้ายังไม่ถึงเวลา/ครบเพดาน
            }
        }
    }
}

#[cfg(test)]
mod tests;
