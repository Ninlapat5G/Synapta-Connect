use super::{
    Refused,
    error::UpdateError,
    history::{Entry, History},
    source::Source,
    version::Version,
    web_package::{self as package, BASELINE, Bundle, Manifest},
};
use crate::{auth::now_secs, fsutil, settings::SettingsStore};
use base64::{Engine, engine::general_purpose::STANDARD};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    path::{Path, PathBuf},
    sync::{Arc, Mutex, RwLock},
    time::Duration,
};

const ACTIVE: &str = "active.json";
const BACKUP: &str = "previous.json";
const BLOCKED: &str = "rolled-back.json";

#[derive(Clone, Serialize, Deserialize)]
struct Candidate {
    manifest: String,
    signature: String,
}

#[derive(Serialize, Deserialize)]
struct Saved {
    signed: Candidate,
    payload: String,
}

struct Installed {
    version: String,
    saved: Saved,
    bundle: Bundle,
}

#[derive(Default)]
struct Slots {
    current: Option<Arc<Installed>>,
    previous: Option<Arc<Installed>>,
}

#[derive(Clone)]
struct Available {
    info: Manifest,
    signed: Candidate,
    source: Source,
}

#[derive(Default)]
struct Progress {
    phase: Option<&'static str>,
    last_check: Option<u64>,
    available: Option<Available>,
    error: Option<UpdateError>,
}

pub enum Action {
    Check,
    Apply(String),
    Retry,
    Rollback,
}

pub struct WebUpdater {
    key: Option<[u8; 32]>,
    dir: PathBuf,
    settings: Arc<SettingsStore>,
    slots: RwLock<Slots>,
    progress: Mutex<Progress>,
    gate: Arc<tokio::sync::Mutex<()>>,
    history: History,
}

impl WebUpdater {
    pub fn new(data_dir: &Path, settings: Arc<SettingsStore>, key: Option<[u8; 32]>) -> Self {
        let dir = data_dir.join("web-update");
        let mut progress = Progress::default();
        let mut slots = Slots::default();
        if let Some(key) = &key {
            slots.previous = read_installed(&dir.join(BACKUP), key).ok().map(Arc::new);
            match read_installed(&dir.join(ACTIVE), key) {
                Ok(installed) => slots.current = Some(Arc::new(installed)),
                Err(error) if dir.join(ACTIVE).exists() => {
                    progress.phase = Some("rolled_back");
                    progress.error = Some(error);
                    slots.current = slots.previous.take();
                }
                Err(_) => {}
            }
        }
        Self {
            key,
            history: History::installer(&dir),
            dir,
            settings,
            slots: RwLock::new(slots),
            progress: Mutex::new(progress),
            gate: Arc::default(),
        }
    }

    pub fn file(&self, path: &str) -> Option<(&'static str, Vec<u8>)> {
        let slots = self.slots.read().unwrap();
        slots
            .current
            .as_ref()
            .and_then(|panel| panel.bundle.file(path))
            .or_else(|| {
                (path != "index.html").then_some(())?;
                slots.previous.as_ref()?.bundle.file(path)
            })
    }

    fn current(&self) -> String {
        self.slots
            .read()
            .unwrap()
            .current
            .as_ref()
            .map_or(BASELINE, |panel| panel.version.as_str())
            .to_owned()
    }

    fn source(&self) -> Result<Source, UpdateError> {
        let configured = self.settings.get();
        if self.key.is_none() {
            return Err(UpdateError::new("check", "not_configured", "ยังไม่ได้ฝังกุญแจของผู้เผยแพร่"));
        }
        Source::parse(&configured.effective_web_source())
            .map_err(|_| UpdateError::new("check", "not_configured", "ยังไม่ได้กำหนดแหล่งอัปเดตเว็บ"))
    }

    pub fn status(&self) -> Value {
        let current = self.current();
        let source = self.source();
        let progress = self.progress.lock().unwrap();
        let default = if source.is_err() {
            "not_configured"
        } else if self.settings.get().update.auto_update {
            "idle"
        } else {
            "disabled"
        };
        json!({
            "current_version": current,
            "state": progress.phase.unwrap_or(default),
            "source": source.as_ref().ok().map(Source::display),
            "last_check_at": progress.last_check,
            "available": progress.available.as_ref().map(|available| &available.info),
            "last_error": progress.error,
            "can_retry": progress.available.is_some() && progress.error.as_ref().is_some_and(|error| {
                !matches!(error.code.as_str(), "signature_invalid" | "manifest_invalid" | "panel_incompatible" | "version_not_newer")
            }),
            "can_rollback": self.slots.read().unwrap().current.is_some(),
            "history": self.history.recent(20),
        })
    }

    pub fn start(self: &Arc<Self>, action: Action) -> Result<(), Refused> {
        let gate = self.gate.clone().try_lock_owned().map_err(|_| Refused::Busy)?;
        let (result, phase) = {
            let progress = self.progress.lock().unwrap();
            match action {
                Action::Check => {
                    if progress.last_check.is_some_and(|at| now_secs().saturating_sub(at) < 15) {
                        return Err(Refused::RateLimited);
                    }
                    (None, "checking")
                }
                Action::Apply(ref version) => {
                    let found = progress
                        .available
                        .as_ref()
                        .filter(|a| &a.info.version == version)
                        .ok_or_else(|| Refused::Invalid("เว็บรุ่นนี้ยังไม่พร้อมติดตั้ง".into()))?;
                    (Some(found.clone()), "downloading")
                }
                Action::Retry => {
                    let found = progress
                        .available
                        .clone()
                        .ok_or_else(|| Refused::Invalid("ไม่มีอัปเดตเว็บให้ลองใหม่".into()))?;
                    (Some(found), "downloading")
                }
                Action::Rollback => {
                    if self.slots.read().unwrap().current.is_none() {
                        return Err(Refused::Invalid("กำลังใช้เว็บที่มากับ Agent อยู่แล้ว".into()));
                    }
                    (None, "applying")
                }
            }
        };
        self.progress.lock().unwrap().phase = Some(phase);
        let updater = self.clone();
        tokio::spawn(async move {
            let _gate = gate;
            match action {
                Action::Check => {
                    updater.check().await;
                }
                Action::Apply(_) | Action::Retry => {
                    updater.apply(result.expect("validated available")).await;
                }
                Action::Rollback => {
                    if let Err(error) = updater.rollback() {
                        updater.failure(error, None);
                    }
                }
            }
        });
        Ok(())
    }

    pub async fn run_scheduler(self: Arc<Self>) {
        tokio::time::sleep(Duration::from_secs(120)).await;
        let mut tick = tokio::time::interval(Duration::from_secs(30));
        loop {
            tick.tick().await;
            let pref = self.settings.get().update;
            let due = self
                .progress
                .lock()
                .unwrap()
                .last_check
                .is_none_or(|at| now_secs().saturating_sub(at) >= pref.check_interval_hours.clamp(1, 720) * 3600);
            if !pref.auto_update || !due || self.source().is_err() {
                continue;
            }
            let Ok(_gate) = self.gate.try_lock() else { continue };
            self.progress.lock().unwrap().phase = Some("checking");
            if let Some(available) = self.check().await {
                self.apply(available).await;
            }
        }
    }

    pub fn recover(&self) -> Result<(), UpdateError> {
        let _gate = self
            .gate
            .try_lock()
            .map_err(|_| UpdateError::new("rollback", "busy", "กำลังอัปเดตเว็บ"))?;
        self.rollback()
    }

    async fn check(&self) -> Option<Available> {
        {
            let mut progress = self.progress.lock().unwrap();
            progress.last_check = Some(now_secs());
            progress.available = None;
            progress.error = None;
        }
        match self.fetch().await {
            Ok(available) => {
                let mut progress = self.progress.lock().unwrap();
                progress.phase = Some(if available.is_some() {
                    "update_available"
                } else {
                    "up_to_date"
                });
                progress.available = available.clone();
                available
            }
            Err(error) => {
                self.failure(error, None);
                None
            }
        }
    }

    async fn fetch(&self) -> Result<Option<Available>, UpdateError> {
        let source = self.source()?;
        let bytes = source.fetch_small(package::MANIFEST).await?;
        let signature = source.fetch_small(package::SIGNATURE).await?;
        let signature = String::from_utf8(signature)
            .map_err(|_| UpdateError::new("verify", "signature_invalid", "ลายเซ็นเว็บผิดรูป"))?;
        let info = Manifest::verify(&bytes, &signature, &self.key.expect("checked source"))?;
        let current = Version::parse(&self.current()).expect("validated version");
        let blocked: Option<String> = fsutil::read_json(&self.dir.join(BLOCKED));
        if Version::parse(&info.version).is_some_and(|version| version <= current)
            || blocked
                .as_deref()
                .and_then(Version::parse)
                .is_some_and(|version| Some(version) >= Version::parse(&info.version))
        {
            return Ok(None);
        }
        info.compatible()?;
        Ok(Some(Available {
            info,
            signed: Candidate {
                manifest: String::from_utf8(bytes).expect("valid JSON manifest"),
                signature,
            },
            source,
        }))
    }

    async fn apply(&self, available: Available) {
        let version = available.info.version.clone();
        self.progress.lock().unwrap().phase = Some("downloading");
        match self.install(available).await {
            Ok(from) => {
                let mut progress = self.progress.lock().unwrap();
                progress.phase = Some("up_to_date");
                progress.available = None;
                progress.error = None;
                self.history.append(&Entry::new(&from, Some(&version), "installed"));
            }
            Err(error) => self.failure(error, Some(&version)),
        }
    }

    async fn install(&self, available: Available) -> Result<String, UpdateError> {
        let from = self.current();
        if Version::parse(&available.info.version) <= Version::parse(&from) {
            return Err(UpdateError::new("verify", "version_not_newer", "รุ่นเว็บต้องใหม่กว่ารุ่นปัจจุบัน"));
        }
        if self.source()? != available.source {
            return Err(UpdateError::new(
                "check",
                "source_changed",
                "แหล่งอัปเดตเว็บเปลี่ยนแล้ว กรุณาตรวจอีกครั้ง",
            ));
        }
        let info = available.info;
        info.compatible()?;
        std::fs::create_dir_all(&self.dir).map_err(save_error)?;
        let download = self.dir.join("download.tmp");
        let fetched = available
            .source
            .download(&info.asset, &download, info.size)
            .await
            .and_then(|_| package::read_bounded(&download, info.size).map_err(save_error));
        let _ = std::fs::remove_file(download);
        let bytes = fetched?;
        self.progress.lock().unwrap().phase = Some("verifying");
        let bundle = package::verify_payload(&info, &bytes)?;
        let saved = Saved {
            signed: available.signed,
            payload: STANDARD.encode(bytes),
        };
        self.progress.lock().unwrap().phase = Some("applying");
        let mut slots = self.slots.write().unwrap();
        if let Some(current) = slots.current.as_ref() {
            fsutil::write_json(&self.dir.join(BACKUP), &current.saved).map_err(save_error)?;
        } else if self.dir.join(BACKUP).exists() {
            std::fs::remove_file(self.dir.join(BACKUP)).map_err(save_error)?;
        }
        fsutil::write_json(&self.dir.join(ACTIVE), &saved).map_err(save_error)?;
        slots.previous = slots.current.take();
        slots.current = Some(Arc::new(Installed {
            version: info.version,
            saved,
            bundle,
        }));
        Ok(from)
    }

    fn rollback(&self) -> Result<(), UpdateError> {
        let from = self.current();
        let mut slots = self.slots.write().unwrap();
        if slots.current.is_none() {
            return Err(UpdateError::new("rollback", "no_web_rollback", "ไม่มีเว็บที่ติดตั้งเพิ่ม"));
        }
        fsutil::write_json(&self.dir.join(BLOCKED), &from).map_err(save_error)?;
        if let Some(previous) = slots.previous.as_ref() {
            fsutil::write_json(&self.dir.join(ACTIVE), &previous.saved).map_err(save_error)?;
        } else {
            std::fs::remove_file(self.dir.join(ACTIVE)).map_err(save_error)?;
        }
        // Keep the outgoing assets readable for browsers finishing requests from the previous HTML.
        let prior = slots.previous.take();
        slots.previous = slots.current.take();
        slots.current = prior;
        let to = slots
            .current
            .as_ref()
            .map_or(BASELINE, |panel| panel.version.as_str())
            .to_string();
        drop(slots);
        let _ = std::fs::remove_file(self.dir.join(BACKUP));
        let mut progress = self.progress.lock().unwrap();
        progress.available = None;
        progress.error = None;
        progress.phase = Some("rolled_back");
        self.history.append(&Entry::new(&from, Some(&to), "rolled_back"));
        Ok(())
    }

    fn failure(&self, error: UpdateError, version: Option<&str>) {
        let from = self.current();
        let mut progress = self.progress.lock().unwrap();
        progress.phase = Some(match error.code.as_str() {
            "panel_incompatible" => "incompatible",
            "not_configured" => "not_configured",
            _ => "failed",
        });
        if std::fs::create_dir_all(&self.dir).is_ok() {
            self.history
                .append(&Entry::new(&from, version, "failed").with_error(&error, 1));
        }
        progress.error = Some(error);
    }
}

fn read_installed(path: &Path, key: &[u8; 32]) -> Result<Installed, UpdateError> {
    let bytes = package::read_bounded(path, package::MAX_BYTES * 2).map_err(save_error)?;
    let invalid = || UpdateError::new("verify", "manifest_invalid", "แพ็กเกจเว็บบนดิสก์อ่านไม่ได้");
    let saved: Saved = serde_json::from_slice(&bytes).map_err(|_| invalid())?;
    let info = Manifest::verify(saved.signed.manifest.as_bytes(), &saved.signed.signature, key)?;
    info.compatible()?;
    let bytes = STANDARD.decode(&saved.payload).map_err(|_| invalid())?;
    let bundle = package::verify_payload(&info, &bytes)?;
    Ok(Installed {
        version: info.version,
        saved,
        bundle,
    })
}

fn save_error(error: std::io::Error) -> UpdateError {
    UpdateError::new(
        "apply",
        "web_storage_failed",
        format!("อ่านหรือบันทึกแพ็กเกจเว็บไม่ได้: {error}"),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{hex::random_id, update::manifest::testkit::Signer};
    use sha2::{Digest, Sha256};

    struct Fixture {
        dir: PathBuf,
        source: PathBuf,
        settings: Arc<SettingsStore>,
        signer: Signer,
    }

    impl Fixture {
        fn new() -> Self {
            let dir = std::env::temp_dir().join(format!("synapta-web-update-{}", random_id(8)));
            let source = dir.join("release");
            std::fs::create_dir_all(&source).unwrap();
            let settings = Arc::new(SettingsStore::load(&dir));
            settings
                .update(|settings| settings.update.web_source = format!("dir:{}", source.display()))
                .unwrap();
            Self {
                dir,
                source,
                settings,
                signer: Signer::new(),
            }
        }

        fn updater(&self) -> WebUpdater {
            WebUpdater::new(&self.dir, self.settings.clone(), Some(self.signer.public()))
        }

        fn publish(&self, version: &str, api: u32, payload: &[u8]) -> Manifest {
            let info = Manifest {
                version: version.into(),
                panel_api: api,
                asset: Manifest::asset_name(version),
                sha256: crate::hex::to_hex(&Sha256::digest(payload)),
                size: payload.len() as u64,
                notes: String::new(),
            };
            let bytes = serde_json::to_vec(&info).unwrap();
            std::fs::write(self.source.join(package::MANIFEST), &bytes).unwrap();
            std::fs::write(self.source.join(package::SIGNATURE), self.signer.sign_hex(&bytes)).unwrap();
            std::fs::write(self.source.join(&info.asset), payload).unwrap();
            info
        }
    }

    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.dir);
        }
    }

    fn payload(marker: &str) -> Vec<u8> {
        let html = format!(
            r#"<script src="/synaptaconnect/assets/{marker}.js"></script><link href="/synaptaconnect/assets/site.css">"#
        );
        serde_json::to_vec(&json!({
            "index.html": STANDARD.encode(html),
            format!("assets/{marker}.js"): STANDARD.encode("window.app = true"),
            "assets/site.css": STANDARD.encode("body { color: red }"),
        }))
        .unwrap()
    }

    #[tokio::test]
    async fn panel_rejects_bad_signature_hash_contract_and_missing_assets() {
        let fixture = Fixture::new();
        let updater = fixture.updater();
        let info = fixture.publish("90.0.1", package::API, &payload("signed"));
        let sig_file = fixture.source.join(package::SIGNATURE);
        let signature = std::fs::read(&sig_file).unwrap();
        std::fs::write(&sig_file, b"bad").unwrap();
        assert!(updater.check().await.is_none());
        assert_eq!(updater.status()["last_error"]["code"], "signature_invalid");
        std::fs::write(sig_file, signature).unwrap();
        let mut tampered = payload("signed");
        tampered[3] ^= 1;
        std::fs::write(fixture.source.join(info.asset), tampered).unwrap();
        updater.apply(updater.check().await.unwrap()).await;
        assert_eq!(updater.status()["last_error"]["code"], "hash_mismatch");
        assert_eq!(updater.current(), BASELINE);

        fixture.publish("90.0.2", package::API + 1, &payload("future"));
        assert!(updater.check().await.is_none());
        assert_eq!(updater.status()["last_error"]["code"], "panel_incompatible");
        fixture.publish(
            "90.0.3",
            package::API,
            br#"{"index.html":"PA==","assets/../escape.js":"YQ=="}"#,
        );
        updater.apply(updater.check().await.unwrap()).await;
        assert_eq!(updater.status()["last_error"]["code"], "manifest_invalid");
        assert_eq!(updater.current(), BASELINE);
        assert!(!fixture.dir.join("web-update").join(ACTIVE).exists());
    }

    #[tokio::test]
    async fn valid_panel_survives_restart_and_recovers_previous_or_embedded() {
        let fixture = Fixture::new();
        let updater = fixture.updater();
        fixture.publish("90.0.1", package::API, &payload("one"));
        updater.apply(updater.check().await.unwrap()).await;
        assert_eq!(updater.current(), "90.0.1");
        fixture.publish("90.0.2", package::API, &payload("two"));
        updater.apply(updater.check().await.unwrap()).await;
        assert_eq!(updater.current(), "90.0.2");
        assert!(updater.file("assets/one.js").is_some());
        assert!(updater.file("assets/two.js").is_some());

        let restarted = fixture.updater();
        assert_eq!(restarted.current(), "90.0.2");
        restarted.recover().unwrap();
        assert_eq!(restarted.current(), "90.0.1");
        assert!(
            restarted.check().await.is_none(),
            "manual rollback blocks the same bad release"
        );
        assert_eq!(fixture.updater().current(), "90.0.1");

        fixture.publish("90.0.3", package::API, &payload("three"));
        restarted.apply(restarted.check().await.unwrap()).await;
        let active = fixture.dir.join("web-update").join(ACTIVE);
        std::fs::write(&active, b"truncated").unwrap();
        let recovered = fixture.updater();
        assert_eq!(recovered.current(), "90.0.1");
        assert_eq!(recovered.status()["state"], "rolled_back");
        std::fs::write(fixture.dir.join("web-update").join(BACKUP), b"broken").unwrap();
        assert_eq!(fixture.updater().current(), BASELINE);
    }
}
