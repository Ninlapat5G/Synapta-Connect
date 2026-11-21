//! เทสต์ตัวประสานฝั่ง agent: ตรวจ → ดาวน์โหลด → ส่งให้ตัวติดตั้ง → รับผล — ใช้แหล่งแบบ dir: และตัวติดตั้งปลอม
//! (ไม่ต้องมี repo/เครือข่ายจริง) ส่วนการสลับไฟล์/กู้คืนทดสอบแยกใน installer/tests.rs

use super::*;
use manifest::testkit::Signer;
use std::sync::atomic::{AtomicU32, Ordering};

struct FakeTrigger {
    calls: AtomicU32,
    fail: bool,
}

impl Trigger for FakeTrigger {
    fn start_installer(&self) -> Result<(), String> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        if self.fail {
            Err("ไม่มี service".into())
        } else {
            Ok(())
        }
    }
}

struct Fixture {
    mgr: UpdateManager,
    trigger: Arc<FakeTrigger>,
    signer: Signer,
    root: PathBuf,
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

fn fixture_with(fail_trigger: bool, with_key: bool) -> Fixture {
    let root = std::env::temp_dir().join(format!("synapta-mgr-{}", crate::hex::random_id(4)));
    let (data, release) = (root.join("data"), root.join("release"));
    std::fs::create_dir_all(&data).unwrap();
    std::fs::create_dir_all(&release).unwrap();
    let signer = Signer::new();
    let trigger = Arc::new(FakeTrigger {
        calls: AtomicU32::new(0),
        fail: fail_trigger,
    });
    let settings = Arc::new(SettingsStore::load(&data));
    settings
        .update(|s| s.update.source = format!("dir:{}", release.display()))
        .unwrap();
    let env = UpdateEnv {
        pubkey: with_key.then(|| signer.public()),
        trigger: trigger.clone(),
        current: Version(0, 3, 0),
    };
    Fixture {
        mgr: UpdateManager::new(env, settings, data),
        trigger,
        signer,
        root,
    }
}

fn fixture() -> Fixture {
    fixture_with(false, true)
}

impl Fixture {
    fn publish(&self, version: &str, payload: &[u8]) {
        let (manifest, sig) = self.signer.release(version, payload);
        self.publish_raw(version, &manifest, &sig, payload);
    }
    fn publish_raw(&self, version: &str, manifest: &[u8], sig: &str, payload: &[u8]) {
        let dir = self.root.join("release");
        std::fs::write(dir.join("manifest.json"), manifest).unwrap();
        std::fs::write(dir.join("manifest.json.sig"), sig).unwrap();
        std::fs::write(dir.join(manifest::Manifest::asset_name(version)), payload).unwrap();
    }
    fn status(&self) -> Value {
        self.mgr.status()
    }
    fn state(&self) -> String {
        self.status()["state"].as_str().unwrap().to_string()
    }
    fn code(&self) -> String {
        self.status()["last_error"]["code"].as_str().unwrap_or("").to_string()
    }
    /// ทำตัวเป็นตัวติดตั้งที่รายงานผล
    fn installer_reports(&self, version: &str, outcome: &str, error: Option<UpdateError>) {
        let result = UpdateResult {
            id: crate::hex::random_id(4),
            version: version.into(),
            from: "0.3.0".into(),
            outcome: outcome.into(),
            error,
            at: now_secs(),
        };
        let status = self.root.join("data").join("updater-status");
        std::fs::create_dir_all(&status).unwrap();
        fsutil::write_json(&status.join("result.json"), &result).unwrap();
        self.mgr.ingest_result();
    }
}

#[tokio::test]
async fn check_reports_newer_release_and_ignores_same_or_older() {
    let f = fixture();
    assert_eq!(f.state(), "idle");

    f.publish("0.3.0", b"same");
    f.mgr.check(false).await.unwrap();
    assert_eq!(
        (f.state().as_str(), f.status()["available"].is_null()),
        ("up_to_date", true)
    );

    f.publish("0.3.1", b"new");
    f.mgr.check(false).await.unwrap();
    let s = f.status();
    assert_eq!(
        (s["state"].as_str(), s["available"]["version"].as_str()),
        (Some("update_available"), Some("0.3.1"))
    );
    assert_eq!(s["history"][0]["outcome"], "available");

    // ตรวจซ้ำอัตโนมัติเจอรุ่นเดิมอีก = ไม่เพิ่มประวัติซ้ำ
    let before = f.mgr.history(100).len();
    f.mgr.check(false).await.unwrap();
    assert_eq!(f.mgr.history(100).len(), before);
}

#[tokio::test]
async fn forged_or_broken_release_is_never_offered() {
    let f = fixture();
    let (manifest, _) = f.signer.release("0.3.1", b"new");
    f.publish_raw("0.3.1", &manifest, &Signer::new().sign_hex(&manifest), b"new");
    f.mgr.check(false).await.unwrap();
    assert_eq!((f.state().as_str(), f.code().as_str()), ("failed", "signature_invalid"));
    assert!(f.status()["available"].is_null());

    std::fs::remove_file(f.root.join("release").join("manifest.json")).unwrap();
    f.mgr.check(false).await.unwrap();
    assert_eq!(f.code(), "source_unreachable");
    assert_eq!(f.status()["last_error"]["stage"], "check");

    // ตรวจผ่านครั้งต่อไป ความผิดจากการตรวจก็หายเอง
    f.publish("0.3.0", b"same");
    f.mgr.check(false).await.unwrap();
    assert_eq!(f.state(), "up_to_date");
}

#[tokio::test]
async fn without_public_key_or_source_nothing_can_be_installed() {
    let f = fixture_with(false, false);
    f.publish("0.3.1", b"new");
    f.mgr.check(true).await.unwrap();
    assert_eq!(f.state(), "not_configured");
    assert!(f.status()["available"].is_null());

    // ลบแหล่งแล้วจะกลับไปใช้แหล่งที่ฝังใน release/source.txt — ไม่มีให้กลับ = ยังไม่ได้ตั้ง
    let f = fixture();
    f.mgr.set_settings(true, Some(String::new())).unwrap();
    if crate::settings::Settings::default().effective_source().is_empty() {
        assert_eq!(f.state(), "not_configured");
    }
}

#[tokio::test]
async fn apply_stages_verified_package_calls_installer_and_installer_result_updates_state() {
    let f = fixture();
    f.publish("0.3.1", b"new-exe");
    f.mgr.check(false).await.unwrap();
    f.mgr.apply("0.3.1", true).await.unwrap();

    assert_eq!(f.trigger.calls.load(Ordering::SeqCst), 1);
    assert_eq!(f.state(), "applying");
    let staging = f.root.join("data").join("staging");
    assert_eq!(
        std::fs::read(staging.join("0.3.1").join("package.exe")).unwrap(),
        b"new-exe"
    );
    assert!(staging.join("0.3.1").join("manifest.json.sig").exists() && staging.join("request.json").exists());

    // ตัวติดตั้งกู้รุ่นเดิมเพราะรุ่นใหม่ใช้ไม่ได้ → panel เห็นสาเหตุและลองใหม่ได้
    let cause = UpdateError::new("health", "health_check_failed", "service ไม่ตอบ");
    f.installer_reports("0.3.1", "rolled_back", Some(cause));
    let s = f.status();
    assert_eq!(
        (s["state"].as_str(), s["last_error"]["code"].as_str()),
        (Some("rolled_back"), Some("health_check_failed"))
    );
    assert_eq!(
        (s["attempts"].as_u64(), s["can_retry"].as_bool()),
        (Some(1), Some(true))
    );

    // ลองใหม่ด้วยมือ → เริ่มนับใหม่ → สำเร็จ
    f.mgr.apply("0.3.1", true).await.unwrap();
    assert_eq!(f.trigger.calls.load(Ordering::SeqCst), 2);
    f.installer_reports("0.3.1", "installed", None);
    let s = f.status();
    assert!(s["last_error"].is_null() && s["available"].is_null());
    assert_eq!(s["attempts"], 0);
}

#[tokio::test]
async fn tampered_download_is_rejected_before_installer_is_called() {
    let f = fixture();
    let (manifest, sig) = f.signer.release("0.3.1", b"honest");
    f.publish_raw("0.3.1", &manifest, &sig, b"hacked"); // ขนาดเท่ากัน เนื้อต่างกัน
    f.mgr.check(false).await.unwrap();
    f.mgr.apply("0.3.1", true).await.unwrap();
    assert_eq!(f.code(), "hash_mismatch");
    assert_eq!(f.trigger.calls.load(Ordering::SeqCst), 0);
    assert!(!f.root.join("data").join("staging").join("0.3.1").exists());
    assert_eq!(f.mgr.history(1)[0].outcome, "failed");

    // ไฟล์ใหญ่กว่าที่ manifest เซ็นไว้ถูกตัดตั้งแต่ตอนดาวน์โหลด
    f.publish_raw("0.3.1", &manifest, &sig, b"honest-but-much-longer");
    f.mgr.apply("0.3.1", true).await.unwrap();
    assert_eq!(f.code(), "download_failed");
    assert_eq!(f.trigger.calls.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn missing_installer_service_is_reported_and_auto_retry_stops_at_the_limit() {
    let f = fixture_with(true, true);
    f.publish("0.3.1", b"new");
    f.mgr.check(false).await.unwrap();

    f.mgr.apply("0.3.1", false).await.unwrap();
    assert_eq!(f.code(), "updater_unavailable");
    assert!(f.mgr.state.lock().unwrap().pending.is_none(), "ไม่มีงานค้างที่ตัวติดตั้ง");
    // ยังไม่ถึงเวลา backoff → ระบบลองเองไม่ได้ แต่เจ้าของสั่งเองได้
    assert!(matches!(f.mgr.apply("0.3.1", false).await, Err(Refused::Invalid(_))));
    f.mgr.apply("0.3.1", true).await.unwrap();
    assert_eq!(f.trigger.calls.load(Ordering::SeqCst), 2);

    // ข้าม backoff จนครบเพดาน
    for _ in 0..2 {
        f.mgr.save(|st| st.next_attempt_at = Some(0));
        let _ = f.mgr.apply("0.3.1", false).await;
    }
    assert_eq!(f.status()["attempts"], MAX_ATTEMPTS);
    f.mgr.save(|st| st.next_attempt_at = Some(0));
    assert!(
        matches!(f.mgr.apply("0.3.1", false).await, Err(Refused::Invalid(_))),
        "ครบเพดานแล้วระบบไม่ลองเอง"
    );
}

#[tokio::test]
async fn apply_is_refused_for_unknown_version_and_manual_check_is_rate_limited() {
    let f = fixture();
    f.publish("0.3.1", b"new");
    assert!(
        matches!(f.mgr.apply("0.3.1", true).await, Err(Refused::Invalid(_))),
        "ยังไม่เคยตรวจ"
    );
    f.mgr.check(true).await.unwrap();
    assert!(matches!(f.mgr.apply("9.9.9", true).await, Err(Refused::Invalid(_))));
    assert_eq!(f.mgr.check(true).await, Err(Refused::RateLimited));
    f.mgr.check(false).await.unwrap(); // อัตโนมัติไม่ติดเพดานของปุ่มกด
}

#[tokio::test]
async fn a_dead_installer_is_given_up_on_instead_of_waiting_forever() {
    let f = fixture();
    f.mgr.save(|st| {
        st.pending = Some(Pending {
            version: "0.3.1".into(),
            requested_at: now_secs() - PENDING_TIMEOUT_SECS - 1,
        })
    });
    assert_eq!(f.state(), "applying");
    f.mgr.ingest_result();
    assert_eq!(f.code(), "updater_unavailable");
    assert_ne!(f.state(), "applying");
}

#[tokio::test]
async fn settings_toggle_auto_update_and_persist() {
    let f = fixture();
    f.mgr.set_settings(false, None).unwrap();
    assert_eq!(f.state(), "disabled");
    assert_eq!(f.status()["auto_update"], false);
    // ปิดอัตโนมัติแล้วยังตรวจ/ติดตั้งด้วยมือได้
    f.publish("0.3.1", b"new");
    f.mgr.check(true).await.unwrap();
    assert_eq!(f.state(), "update_available");
}

#[tokio::test]
async fn rolled_back_result_restores_the_agents_own_settings_backup_once_and_cleans_up() {
    let f = fixture();
    f.publish("0.3.1", b"new");
    f.mgr.check(false).await.unwrap();
    f.mgr.apply("0.3.1", true).await.unwrap(); // agent สำรองค่าตั้งของตัวเองก่อนส่งคำขอ (auto_update=true)
    let backup = f.root.join("data").join("settings.pre-update.json");
    assert!(backup.exists());

    // "รุ่นใหม่" แก้ค่าตั้ง แล้วตัวติดตั้งรายงานว่ากู้รุ่นเดิม
    f.mgr.set_settings(false, None).unwrap();
    let cause = UpdateError::new("health", "health_check_failed", "x");
    f.installer_reports("0.3.1", "rolled_back", Some(cause));
    assert!(f.mgr.settings.get().update.auto_update, "ค่าตั้งเดิมต้องกลับมา");
    assert!(
        !backup.exists() && !f.root.join("data").join("staging").exists(),
        "agent เก็บกวาดของตัวเอง"
    );

    // อ่านผลเดิมซ้ำ (ไฟล์ลบไม่ได้) ต้องไม่ทับค่าที่เจ้าของตั้งใหม่ภายหลัง และไม่นับความล้มเหลวซ้ำ
    f.mgr.set_settings(false, None).unwrap();
    f.mgr.ingest_result();
    assert!(!f.mgr.settings.get().update.auto_update);
    assert_eq!(f.status()["attempts"], 1);
}

#[tokio::test]
async fn successful_update_keeps_new_settings_and_drops_the_backup() {
    let f = fixture();
    f.publish("0.3.1", b"new");
    f.mgr.check(false).await.unwrap();
    f.mgr.apply("0.3.1", true).await.unwrap();
    f.mgr.set_settings(false, None).unwrap(); // รุ่นใหม่ตั้งค่าเอง
    f.installer_reports("0.3.1", "installed", None);
    assert!(!f.mgr.settings.get().update.auto_update);
    assert!(!f.root.join("data").join("settings.pre-update.json").exists());
}
