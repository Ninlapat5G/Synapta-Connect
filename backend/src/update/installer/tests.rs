//! เทสต์การเปลี่ยนรุ่น/กู้คืน ด้วย Host ปลอมที่ "ทำตัวเป็น agent" ตามเนื้อหาของไฟล์ exe:
//! เนื้อหา `<รุ่น>|<โหมด>` — โหมด ok | unhealthy | flaky | nostart

use super::*;
use crate::update::manifest::testkit::Signer;
use std::sync::Mutex;

fn settings_path(l: &Layout) -> PathBuf {
    l.data_dir.join("settings.json")
}

struct FakeHost {
    layout: Layout,
    state: Mutex<(bool, u32)>, // (กำลังรัน, pid)
}

impl FakeHost {
    fn exe_text(&self) -> String {
        std::fs::read_to_string(self.layout.exe()).unwrap()
    }
    fn parts(&self) -> (String, String) {
        let text = self.exe_text();
        let (v, m) = text.split_once('|').unwrap();
        (v.into(), m.into())
    }
}

impl Host for FakeHost {
    fn stop_agent(&self) -> Result<(), String> {
        self.state.lock().unwrap().0 = false;
        Ok(())
    }
    fn start_agent(&self) -> Result<(), String> {
        let (version, mode) = self.parts();
        if mode == "nostart" {
            return Err("เริ่มไม่ขึ้น".into());
        }
        // รุ่นใหม่ย้ายรูปแบบ settings ตอนเริ่ม — ถ้ากู้คืนต้องได้ของเดิมกลับ
        if version != "0.3.0" {
            std::fs::write(settings_path(&self.layout), format!("migrated-by-{version}")).unwrap();
        }
        let mut st = self.state.lock().unwrap();
        *st = (true, st.1 + 1);
        Ok(())
    }
    fn health(&self) -> Result<Health, String> {
        let (version, mode) = self.parts();
        let mut st = self.state.lock().unwrap();
        if !st.0 || mode == "unhealthy" {
            return Err("ไม่ตอบ".into());
        }
        if mode == "flaky" {
            st.1 += 1; // ทำเหมือน crash แล้วถูกเริ่มใหม่ทุกครั้งที่ถาม
        }
        Ok(Health {
            version,
            pid: st.1,
            uptime_secs: 1,
        })
    }
}

struct Fixture {
    host: FakeHost,
    signer: Signer,
    root: PathBuf,
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

fn timing() -> Timing {
    Timing {
        health_timeout: Duration::from_millis(300),
        stable_for: Duration::from_millis(50),
        poll: Duration::from_millis(10),
    }
}

/// ติดตั้งรุ่น 0.3.0 ที่ทำงานอยู่ พร้อม settings.json เดิม
fn fixture() -> Fixture {
    let root = std::env::temp_dir().join(format!("synapta-inst-{}", crate::hex::random_id(4)));
    let layout = Layout {
        install_dir: root.join("program"),
        data_dir: root.join("data"),
        work_dir: root.join("work"),
    };
    for d in [&layout.install_dir, &layout.data_dir, &layout.work_dir] {
        std::fs::create_dir_all(d).unwrap();
    }
    std::fs::write(layout.exe(), "0.3.0|ok").unwrap();
    write_install_info(&layout, "0.3.0").unwrap();
    std::fs::write(settings_path(&layout), "OLD").unwrap();
    let host = FakeHost {
        layout,
        state: Mutex::new((true, 1)),
    };
    Fixture {
        host,
        signer: Signer::new(),
        root,
    }
}

impl Fixture {
    fn layout(&self) -> &Layout {
        &self.host.layout
    }
    /// วาง release ใน staging เหมือนที่ agent ทำ
    fn stage(&self, version: &str, payload: &str) {
        let (manifest, sig) = self.signer.release(version, payload.as_bytes());
        self.stage_raw(version, &manifest, &sig, payload.as_bytes());
    }
    fn stage_raw(&self, version: &str, manifest: &[u8], sig: &str, package: &[u8]) {
        let dir = self.layout().staging().join(version);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("manifest.json"), manifest).unwrap();
        std::fs::write(dir.join("manifest.json.sig"), sig).unwrap();
        std::fs::write(dir.join("package.exe"), package).unwrap();
        fsutil::write_json(
            &self.layout().request(),
            &Request {
                version: version.into(),
                id: crate::hex::random_id(4),
            },
        )
        .unwrap();
    }
    fn run(&self) -> UpdateResult {
        apply_pending(self.layout(), &self.host, &self.signer.public(), &timing()).expect("มีคำขอค้างอยู่")
    }
    fn settings(&self) -> String {
        std::fs::read_to_string(settings_path(self.layout())).unwrap()
    }
    /// รายชื่อไฟล์ทั้งหมดในที่ที่ agent คุม (ยกเว้นโฟลเดอร์ผลของตัวติดตั้งเอง)
    fn agent_area_files(&self) -> Vec<PathBuf> {
        fn walk(dir: &Path, skip: &Path, out: &mut Vec<PathBuf>) {
            for e in std::fs::read_dir(dir).into_iter().flatten().flatten() {
                let p = e.path();
                if p == skip {
                    continue;
                }
                if p.is_dir() {
                    walk(&p, skip, out);
                } else {
                    out.push(p);
                }
            }
        }
        let mut files = Vec::new();
        walk(&self.layout().data_dir, &self.layout().status_dir(), &mut files);
        files.sort();
        files
    }
    fn last_history(&self) -> Entry {
        History::new(&self.layout().data_dir).recent(1).remove(0)
    }
}

#[test]
fn successful_update_swaps_keeps_backup_and_records_everything() {
    let f = fixture();
    f.stage("0.3.1", "0.3.1|ok");
    let r = f.run();

    assert_eq!((r.outcome.as_str(), r.error), ("installed", None));
    assert_eq!(f.host.exe_text(), "0.3.1|ok");
    assert_eq!(installed_version(f.layout()).as_deref(), Some("0.3.1"));
    assert_eq!(f.settings(), "migrated-by-0.3.1");
    // เก็บตัวติดตั้งรุ่นเดิม + ค่าตั้งเดิมไว้
    let backup = f.layout().backups().join("0.3.0");
    assert_eq!(std::fs::read_to_string(backup.join(EXE_NAME)).unwrap(), "0.3.0|ok");
    assert!(!f.layout().journal().exists());
    // ตัวติดตั้งไม่ลบ/ไม่เขียนในที่ที่ agent คุม (staging ยังอยู่ — agent เก็บกวาดเอง) และผลอยู่ในที่ agent เขียนไม่ได้
    assert!(f.layout().request().exists() && f.layout().staging().join("0.3.1").exists());
    assert_eq!(f.last_history().outcome, "installed");
    assert!(f.layout().result().starts_with(f.layout().status_dir()) && f.layout().result().exists());
    // คำขอเดิมที่ยังค้างในโฟลเดอร์ (ตัวติดตั้งตื่นตอนบูตทุกครั้ง) ต้องไม่ถูกทำซ้ำ
    assert!(apply_pending(f.layout(), &f.host, &f.signer.public(), &timing()).is_none());
    assert_eq!(f.host.exe_text(), "0.3.1|ok");
}

#[test]
fn bad_signature_hash_or_version_never_touch_the_installed_files() {
    let f = fixture();
    let (manifest, sig) = f.signer.release("0.3.1", b"0.3.1|ok");

    // ลายเซ็นจากคีย์อื่น
    let other = Signer::new();
    f.stage_raw("0.3.1", &manifest, &other.sign_hex(&manifest), b"0.3.1|ok");
    assert_eq!(f.run().error.unwrap().code, "signature_invalid");

    // manifest ถูกต้อง แต่ตัวติดตั้งถูกสลับเนื้อ
    f.stage_raw("0.3.1", &manifest, &sig, b"0.3.1|evil");
    assert_eq!(f.run().error.unwrap().code, "hash_mismatch");

    // รุ่นเท่ากัน / ต่ำกว่า = downgrade
    for old in ["0.3.0", "0.2.9"] {
        f.stage(old, &format!("{old}|ok"));
        assert_eq!(f.run().error.unwrap().code, "version_not_newer", "{old}");
    }

    // manifest ของรุ่นอื่นถูกวางใต้ชื่อโฟลเดอร์ของอีกรุ่น (ลองหลอกให้ติดตั้งรุ่นที่ไม่ได้ขอ)
    let (m2, s2) = f.signer.release("0.9.0", b"0.9.0|ok");
    f.stage_raw("0.3.1", &m2, &s2, b"0.9.0|ok");
    assert_eq!(f.run().error.unwrap().code, "manifest_invalid");

    assert_eq!(f.host.exe_text(), "0.3.0|ok");
    assert_eq!(f.settings(), "OLD");
    assert_eq!(f.last_history().outcome, "failed");
    assert!(
        f.layout().backups().read_dir().is_err(),
        "ตรวจไม่ผ่านต้องไม่สำรอง/ไม่แตะอะไร"
    );
}

#[test]
fn new_version_that_fails_health_is_rolled_back_with_old_settings() {
    for (mode, expected_code) in [
        ("unhealthy", "health_check_failed"),
        ("nostart", "service_start_failed"),
        ("flaky", "health_check_failed"),
    ] {
        let f = fixture();
        f.stage("0.3.1", &format!("0.3.1|{mode}"));
        let before = f.agent_area_files();
        let r = f.run();
        assert_eq!(
            f.agent_area_files(),
            before,
            "ตัวติดตั้งต้องไม่สร้าง/ลบไฟล์ในที่ที่ agent คุม ({mode})"
        );

        assert_eq!(r.outcome, "rolled_back", "{mode}");
        assert_eq!(r.error.unwrap().code, expected_code, "{mode}");
        assert_eq!(f.host.exe_text(), "0.3.0|ok", "{mode}");
        assert_eq!(installed_version(f.layout()).as_deref(), Some("0.3.0"));
        // ตัวติดตั้งไม่แตะ settings.json (ค่าตั้งของ agent): agent รุ่นเดิมที่กลับมาใส่ค่าคืนเอง (ทดสอบใน update/tests.rs)
        let migrated = if mode == "nostart" { "OLD" } else { "migrated-by-0.3.1" };
        assert_eq!(f.settings(), migrated, "{mode}");
        assert!(
            f.host.health().is_ok_and(|h| h.version == "0.3.0"),
            "agent รุ่นเดิมต้องกลับมาทำงาน ({mode})"
        );
        assert!(!f.layout().journal().exists());
        assert_eq!(f.last_history().outcome, "rolled_back");
    }
}

#[test]
fn interruption_after_swap_is_recovered_on_next_start() {
    let f = fixture();
    f.stage("0.3.1", "0.3.1|ok");
    // จำลองไฟดับหลังสลับไฟล์ ก่อนตรวจสุขภาพ: ทำสามขั้นแรกของ apply() เอง แล้วไม่ทำต่อ
    let l = f.layout();
    let backup = l.backups().join("0.3.0");
    backup_current(l, &backup).unwrap();
    std::fs::copy(l.staging().join("0.3.1").join("package.exe"), l.exe_new()).unwrap();
    fsutil::write_json(
        &l.journal(),
        &Journal {
            from: "0.3.0".into(),
            to: "0.3.1".into(),
            state: "swapping".into(),
        },
    )
    .unwrap();
    f.host.stop_agent().unwrap();
    swap_in(l, "0.3.1").unwrap();
    std::fs::write(settings_path(l), "half-migrated").unwrap();
    assert_eq!(f.host.exe_text(), "0.3.1|ok");

    let r = recover(l, &f.host, &timing()).expect("มี journal ค้าง");
    assert_eq!(
        (r.outcome.as_str(), r.error.unwrap().code.as_str()),
        ("rolled_back", "interrupted_rolled_back")
    );
    assert_eq!(f.host.exe_text(), "0.3.0|ok");
    assert!(f.host.health().is_ok());
    assert!(!l.journal().exists());

    assert!(recover(l, &f.host, &timing()).is_none(), "ไม่มี journal = ไม่ทำอะไร");
}

#[test]
fn interruption_that_left_no_exe_at_all_is_recovered_from_backup() {
    let f = fixture();
    let l = f.layout();
    backup_current(l, &l.backups().join("0.3.0")).unwrap();
    fsutil::write_json(
        &l.journal(),
        &Journal {
            from: "0.3.0".into(),
            to: "0.3.1".into(),
            state: "swapping".into(),
        },
    )
    .unwrap();
    std::fs::rename(l.exe(), l.exe_old()).unwrap(); // ตายหลังย้ายไฟล์เดิม ก่อนวางไฟล์ใหม่
    assert!(!l.exe().exists());

    assert_eq!(recover(l, &f.host, &timing()).unwrap().outcome, "rolled_back");
    assert_eq!(f.host.exe_text(), "0.3.0|ok");
}

#[test]
fn unresolved_recovery_prevents_another_update_from_overwriting_the_journal() {
    let f = fixture();
    let layout = f.layout();
    f.stage("0.3.1", "0.3.1|ok");
    fsutil::write_json(
        &layout.journal(),
        &Journal {
            from: "0.2.9".into(),
            to: "0.3.0".into(),
            state: "verifying".into(),
        },
    )
    .unwrap();

    assert!(apply_pending(layout, &f.host, &f.signer.public(), &timing()).is_none());
    let journal: Journal = fsutil::read_json(&layout.journal()).unwrap();
    assert_eq!(journal.from, "0.2.9");
    assert_eq!(f.host.exe_text(), "0.3.0|ok");
    assert!(!layout.last_request().exists());
}

#[cfg(windows)]
#[test]
fn replaced_windows_image_can_remain_mapped_during_recovery() {
    use std::{os::windows::process::CommandExt, process::Command};
    let f = fixture();
    let layout = f.layout();
    let ping = crate::platform::win::system_tool("ping.exe");
    std::fs::copy(ping, layout.exe()).unwrap();
    std::fs::write(layout.exe_new(), "restored image").unwrap();
    let mut child = Command::new(layout.exe())
        .args(["-t", "127.0.0.1"])
        .creation_flags(0x0800_0000)
        .spawn()
        .unwrap();
    let displaced = retired_image(layout);
    let result = replace_executable(layout, &displaced);
    let still_running = child.try_wait().unwrap().is_none();
    let _ = child.kill();
    let _ = child.wait();

    result.unwrap();
    assert!(still_running);
    assert!(displaced.is_file());
    assert_eq!(std::fs::read_to_string(layout.exe()).unwrap(), "restored image");
}

#[test]
fn rollback_that_cannot_complete_is_reported_loudly_with_backup_path() {
    let f = fixture();
    let l = f.layout();
    fsutil::write_json(
        &l.journal(),
        &Journal {
            from: "0.3.0".into(),
            to: "0.3.1".into(),
            state: "verifying".into(),
        },
    )
    .unwrap();
    // ไม่มีสำรองเลย
    let r = recover(l, &f.host, &timing()).unwrap();
    let e = r.error.unwrap();
    assert_eq!((r.outcome.as_str(), e.code.as_str()), ("failed", "rollback_failed"));
    assert!(e.message.contains("backup"), "ต้องบอกที่เก็บตัวติดตั้งรุ่นเดิม: {}", e.message);
}

#[test]
fn only_the_three_newest_backups_are_kept() {
    let f = fixture();
    for v in ["0.1.0", "0.2.0", "0.10.0", "0.3.0", "0.9.0"] {
        std::fs::create_dir_all(f.layout().backups().join(v)).unwrap();
    }
    prune_backups(f.layout());
    let mut left: Vec<String> = f
        .layout()
        .backups()
        .read_dir()
        .unwrap()
        .map(|e| e.unwrap().file_name().into_string().unwrap())
        .collect();
    left.sort();
    assert_eq!(left, ["0.10.0", "0.3.0", "0.9.0"]); // เรียงตามเลขรุ่น ไม่ใช่ตัวอักษร
}

#[test]
fn no_request_means_nothing_happens() {
    let f = fixture();
    assert!(apply_pending(f.layout(), &f.host, &f.signer.public(), &timing()).is_none());
}

#[test]
fn oversized_package_is_never_copied_by_the_privileged_installer() {
    let f = fixture();
    let (manifest, sig) = f.signer.release("0.3.1", b"0.3.1|ok");
    // agent ที่ถูกยึดวางไฟล์ใหญ่กว่าที่ manifest (เซ็นแล้ว) ระบุ — ต้องไม่ถูกคัดลอกลง work_dir เลย
    f.stage_raw("0.3.1", &manifest, &sig, &vec![7u8; 5 * 1024 * 1024]);
    let r = f.run();
    assert_eq!(r.error.unwrap().code, "hash_mismatch");
    assert!(!f.layout().work_dir.join("incoming").join("package.exe").exists());
    assert_eq!(f.host.exe_text(), "0.3.0|ok");

    // ไฟล์ manifest ใหญ่ผิดปกติก็ถูกปฏิเสธก่อนอ่านทั้งไฟล์
    f.stage_raw("0.3.1", &vec![b'x'; 200 * 1024], &sig, b"0.3.1|ok");
    assert_eq!(r_code(&f), "apply_failed");
}

fn r_code(f: &Fixture) -> String {
    f.run().error.unwrap().code
}

#[test]
fn unknown_installed_version_refuses_instead_of_skipping_the_downgrade_check() {
    let f = fixture();
    std::fs::remove_file(f.layout().install_json()).unwrap();
    f.stage("0.3.1", "0.3.1|ok");
    let r = f.run();
    assert_eq!(
        (r.outcome.as_str(), r.error.unwrap().code.as_str()),
        ("failed", "apply_failed")
    );
    assert_eq!(f.host.exe_text(), "0.3.0|ok");
    assert!(f.layout().backups().read_dir().is_err(), "ไม่ได้สำรอง/ไม่ได้แตะอะไร");
}
