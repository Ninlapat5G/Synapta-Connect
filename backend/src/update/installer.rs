//! ตัวเปลี่ยนรุ่นที่มีสิทธิ์สูง (ทำงานใน service ตัวติดตั้งแบบ SYSTEM — ดู winsvc/) และการกู้คืน
//!
//! หลักการ: ไม่เชื่อสิ่งที่ agent (สิทธิ์ต่ำ) วางไว้ใน staging — คัดลอกเข้าที่ของตัวเองก่อน แล้วตรวจลายเซ็น/hash/เวอร์ชัน
//! ใหม่จากสำเนานั้น (กัน agent เปลี่ยนไฟล์หลังตรวจ) แม้ agent ถูกยึดก็ลงได้เฉพาะ release ที่ผู้เผยแพร่เซ็นและใหม่กว่าเท่านั้น
//!
//! ลำดับ: สำรองรุ่นเดิม+ค่าตั้ง → จดบันทึก (journal) → หยุด agent → สลับไฟล์ → เริ่ม agent → ตรวจสุขภาพ
//! ล้มเหลวที่ขั้นใดหลังจดบันทึก = กู้รุ่นเดิม; ถ้าถูกขัดจังหวะ (ไฟดับ) `recover()` ตอนเริ่มระบบทำเรื่องเดียวกัน
//! ทุกการสลับไฟล์เป็น rename (ไม่เขียนทับกลางไฟล์) จึงมีรุ่นที่สมบูรณ์อยู่ในดิสก์เสมอ

use super::{
    error::UpdateError,
    history::{Entry, History},
    manifest::{self, Manifest},
    version::Version,
};
use crate::{auth::now_secs, fsutil};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    io::Read,
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

pub const EXE_NAME: &str = "synapta-connect.exe";
const KEEP_BACKUPS: usize = 3;

pub struct Layout {
    /// โฟลเดอร์โปรแกรม (Program Files\SynaptaConnect) — agent เขียนไม่ได้
    pub install_dir: PathBuf,
    /// ข้อมูลของ agent (ProgramData\SynaptaConnect) — agent เขียนได้
    pub data_dir: PathBuf,
    /// พื้นที่เฉพาะตัวติดตั้ง (journal, สำรอง, สำเนาที่ตรวจแล้ว) — agent เข้าไม่ได้
    pub work_dir: PathBuf,
}

impl Layout {
    pub fn exe(&self) -> PathBuf {
        self.install_dir.join(EXE_NAME)
    }
    fn exe_old(&self) -> PathBuf {
        self.install_dir.join(format!("{EXE_NAME}.old"))
    }
    fn exe_new(&self) -> PathBuf {
        self.install_dir.join(format!("{EXE_NAME}.new"))
    }
    pub fn install_json(&self) -> PathBuf {
        self.install_dir.join("install.json")
    }
    pub fn staging(&self) -> PathBuf {
        self.data_dir.join("staging")
    }
    pub fn request(&self) -> PathBuf {
        self.staging().join("request.json")
    }
    /// ที่ที่ตัวติดตั้งเขียนผลให้ agent อ่าน — agent อ่านได้อย่างเดียว (ACL) ตัวติดตั้งจึงไม่ต้องเขียนในที่ที่ agent คุมเลย
    pub fn status_dir(&self) -> PathBuf {
        self.data_dir.join("updater-status")
    }
    pub fn result(&self) -> PathBuf {
        self.status_dir().join("result.json")
    }
    fn last_request(&self) -> PathBuf {
        self.work_dir.join("last_request")
    }
    fn journal(&self) -> PathBuf {
        self.work_dir.join("journal.json")
    }
    fn backups(&self) -> PathBuf {
        self.work_dir.join("backup")
    }
}

/// สิ่งที่ตัวติดตั้งต้องขอจากระบบปฏิบัติการ — แยกเป็น trait เพื่อทดสอบการกู้คืนโดยไม่ต้องมี service จริง
pub trait Host {
    /// หยุด agent และรอจนหยุดจริง (หยุดอยู่แล้ว = สำเร็จ)
    fn stop_agent(&self) -> Result<(), String>;
    fn start_agent(&self) -> Result<(), String>;
    fn health(&self) -> Result<Health, String>;
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Health {
    pub version: String,
    pub pid: u32,
    pub uptime_secs: u64,
}

pub struct Timing {
    pub health_timeout: Duration,
    pub stable_for: Duration,
    pub poll: Duration,
}

impl Default for Timing {
    fn default() -> Self {
        Timing {
            health_timeout: Duration::from_secs(45),
            stable_for: Duration::from_secs(10),
            poll: Duration::from_secs(1),
        }
    }
}

/// คำขอที่ agent วางให้ — เลขรุ่น + รหัสคำขอ (กันทำซ้ำ); ไฟล์อยู่ที่ staging/<รุ่น>/{manifest.json,manifest.json.sig,package.exe}
/// ตัวติดตั้ง "อ่านอย่างเดียว" จาก staging (ที่ที่ agent คุม) — ไม่ลบ ไม่เขียน เพื่อไม่ให้ agent ที่ถูกยึดใช้ junction/symlink
/// หลอกให้ SYSTEM แตะไฟล์ที่ไม่ใช่ของมัน (agent เก็บกวาด staging เอง)
#[derive(Serialize, Deserialize)]
pub struct Request {
    pub version: String,
    pub id: String,
}

/// ผลที่ agent อ่านไปอัปเดตสถานะ (installer เขียนประวัติเอง)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UpdateResult {
    /// รหัสผล (สุ่มใหม่ทุกครั้ง) — agent จำว่าอ่านผลไหนแล้ว เพราะลบไฟล์นี้ไม่ได้
    pub id: String,
    pub version: String,
    pub from: String,
    /// installed | rolled_back | failed
    pub outcome: String,
    pub error: Option<UpdateError>,
    pub at: u64,
}

#[derive(Serialize, Deserialize)]
struct InstallInfo {
    version: String,
}

#[derive(Serialize, Deserialize)]
struct Journal {
    from: String,
    to: String,
    /// swapping | verifying
    state: String,
}

pub fn installed_version(layout: &Layout) -> Option<String> {
    installed_version_from(&layout.install_json())
}

fn installed_version_from(path: &Path) -> Option<String> {
    fsutil::read_json::<InstallInfo>(path).map(|i| i.version)
}

pub fn write_install_info(layout: &Layout, version: &str) -> std::io::Result<()> {
    fsutil::write_json(
        &layout.install_json(),
        &InstallInfo {
            version: version.into(),
        },
    )
}

fn err(stage: &str, code: &str, message: impl Into<String>) -> UpdateError {
    UpdateError::new(stage, code, message)
}

/// ทำคำขอที่ค้างอยู่ (ถ้ามี) — None = ไม่มีคำขอ หรือเป็นคำขอที่ทำไปแล้ว
pub fn apply_pending(layout: &Layout, host: &dyn Host, pubkey: &[u8; 32], timing: &Timing) -> Option<UpdateResult> {
    if layout.journal().exists() {
        return None;
    }
    let request: Request = fsutil::read_json(&layout.request())?;
    // จำรหัสคำขอไว้ในที่ของเรา (ไม่ลบไฟล์ของ agent): ตัวติดตั้งตื่นตอนบูตทุกครั้ง ต้องไม่ทำคำขอเดิมซ้ำ
    if std::fs::read_to_string(layout.last_request()).is_ok_and(|id| id == request.id) {
        return None;
    }
    std::fs::create_dir_all(&layout.work_dir).ok()?;
    fsutil::atomic_write(&layout.last_request(), request.id.as_bytes()).ok()?;
    let Some(from) = installed_version(layout) else {
        // ไม่รู้รุ่นปัจจุบัน = ตรวจ downgrade ไม่ได้ จึงปฏิเสธ (ไม่ใช่ข้ามการตรวจ)
        let error = err(
            "verify",
            "apply_failed",
            "อ่านรุ่นที่ติดตั้งไม่ได้ (install.json หายหรือเสีย) — ไม่ติดตั้งอัปเดต",
        );
        let result = new_result(request.version, "unknown".into(), "failed", Some(error));
        record(layout, &result);
        return Some(result);
    };
    let outcome = apply(layout, host, pubkey, timing, &request.version, &from);
    let (kind, error) = match outcome {
        Ok(()) => ("installed", None),
        Err((kind, e)) => (kind, Some(e)),
    };
    let result = new_result(request.version, from, kind, error);
    record(layout, &result);
    Some(result)
}

fn new_result(version: String, from: String, outcome: &str, error: Option<UpdateError>) -> UpdateResult {
    UpdateResult {
        id: crate::hex::random_id(8),
        version,
        from,
        outcome: outcome.into(),
        error,
        at: now_secs(),
    }
}

/// กู้คืนถ้าครั้งก่อนถูกขัดจังหวะ — เรียกตอนตัวติดตั้งเริ่มทำงาน (เช่น หลังบูตเครื่อง) ก่อนรับคำขอใหม่
pub fn recover(layout: &Layout, host: &dyn Host, timing: &Timing) -> Option<UpdateResult> {
    // ลบไฟล์เก่าที่ติดค้างเพราะตอนนั้น service ตัวนี้ยังใช้ไฟล์อยู่
    if !layout.journal().exists() {
        let _ = std::fs::remove_file(layout.exe_old());
    }
    let journal: Journal = fsutil::read_json(&layout.journal())?;
    if Version::parse(&journal.from).is_none() || Version::parse(&journal.to).is_none() {
        tracing::error!("ไม่สามารถอ่านรุ่นจาก journal ของตัวอัปเดต");
        return None;
    }
    let error = err(
        "apply",
        "interrupted_rolled_back",
        "การเปลี่ยนรุ่นถูกขัดจังหวะ (ไฟดับหรือถูกปิดกลางทาง) — กู้รุ่นเดิมแล้ว",
    );
    let (kind, error) = match rollback(layout, host, timing, &journal) {
        Ok(()) => ("rolled_back", error),
        Err(e) => ("failed", e),
    };
    let result = new_result(journal.to, journal.from, kind, Some(error));
    record(layout, &result);
    Some(result)
}

fn record(layout: &Layout, result: &UpdateResult) {
    let mut entry = Entry::new(&result.from, Some(&result.version), &result.outcome);
    if let Some(e) = &result.error {
        entry = entry.with_error(e, 0);
    }
    let _ = std::fs::create_dir_all(layout.status_dir());
    History::installer(&layout.status_dir()).append(&entry);
    if let Err(e) = fsutil::write_json(&layout.result(), result) {
        tracing::error!("เขียนผลอัปเดตไม่สำเร็จ: {e}");
    }
}

/// Err((outcome, error)): outcome = "failed" (รุ่นเดิมไม่ถูกแตะ หรือกู้ไม่สำเร็จ) | "rolled_back"
fn apply(
    layout: &Layout,
    host: &dyn Host,
    pubkey: &[u8; 32],
    timing: &Timing,
    version: &str,
    from: &str,
) -> Result<(), (&'static str, UpdateError)> {
    let fail = |e: UpdateError| ("failed", e);

    // 1) คัดลอกเข้าที่ของเรา แล้วตรวจจากสำเนา
    let package = verified_copy(layout, pubkey, version, from).map_err(fail)?;

    // 2) สำรองรุ่นเดิม + ค่าตั้ง (ก่อนแตะอะไรทั้งนั้น)
    let backup = layout.backups().join(from);
    backup_current(layout, &backup).map_err(|e| fail(err("apply", "apply_failed", format!("สำรองรุ่นเดิมไม่ได้: {e}"))))?;

    // 3) เตรียมไฟล์ใหม่ข้าง ๆ ไฟล์จริง ก่อนหยุด agent เพื่อให้ช่วงที่ service ดับสั้นที่สุด
    stage_executable(&package, &layout.exe_new())
        .map_err(|e| fail(err("apply", "apply_failed", format!("วางไฟล์ใหม่ไม่ได้: {e}"))))?;
    clear_old_image(layout).map_err(|e| fail(err("apply", "apply_failed", format!("เก็บไฟล์รุ่นเดิมไม่ได้: {e}"))))?;

    // 4) จดบันทึกแล้วเริ่มสลับ — จากจุดนี้ ล้มเหลว/ถูกขัดจังหวะ = กู้คืน
    let mut journal = Journal {
        from: from.into(),
        to: version.into(),
        state: "swapping".into(),
    };
    fsutil::write_json(&layout.journal(), &journal)
        .map_err(|e| fail(err("apply", "apply_failed", format!("เขียน journal ไม่ได้: {e}"))))?;

    let swapped = host
        .stop_agent()
        .map_err(|e| err("apply", "apply_failed", format!("หยุด service ไม่ได้: {e}")))
        .and_then(|_| swap_in(layout, version));
    if let Err(e) = swapped {
        return Err(finish_with_rollback(layout, host, timing, &journal, e));
    }

    // 5) เริ่มรุ่นใหม่ แล้วตรวจว่าใช้งานได้จริงและไม่ล้มซ้ำ
    journal.state = "verifying".into();
    let _ = fsutil::write_json(&layout.journal(), &journal);
    let healthy = host
        .start_agent()
        .map_err(|e| err("health", "service_start_failed", format!("รุ่นใหม่เริ่มไม่ขึ้น: {e}")))
        .and_then(|_| wait_healthy(host, timing, version, true));
    match healthy {
        Ok(()) => {
            let _ = std::fs::remove_file(layout.journal());
            let _ = std::fs::remove_file(layout.exe_old()); // ล้มก็ไม่เป็นไร: recover() รอบหน้าลบให้
            prune_backups(layout);
            Ok(())
        }
        Err(e) => Err(finish_with_rollback(layout, host, timing, &journal, e)),
    }
}

/// คัดลอก staging → work_dir/incoming แล้วตรวจ ลายเซ็น/hash/ขนาด/เวอร์ชัน ของสำเนา; คืน path ของ exe ที่ตรวจแล้ว
fn verified_copy(layout: &Layout, pubkey: &[u8; 32], version: &str, from: &str) -> Result<PathBuf, UpdateError> {
    let stage = |m: String| err("verify", "apply_failed", m);
    if Version::parse(version).is_none() {
        return Err(err("verify", "manifest_invalid", "เลขรุ่นในคำขอผิดรูป"));
    }
    let src = layout.staging().join(version);
    let incoming = layout.work_dir.join("incoming");
    let _ = std::fs::remove_dir_all(&incoming);
    std::fs::create_dir_all(&incoming).map_err(|e| stage(format!("สร้างโฟลเดอร์ตรวจสอบไม่ได้: {e}")))?;
    // manifest/ลายเซ็นเล็กเสมอ — อ่านแบบมีเพดาน แล้วตรวจลายเซ็นก่อนแตะแพ็กเกจ (agent ใส่ไฟล์ใหญ่แค่ไหนก็ไม่ถูกคัดลอก)
    for name in ["manifest.json", "manifest.json.sig"] {
        let bytes = read_bounded(&src.join(name), manifest::MAX_MANIFEST_BYTES as u64)
            .map_err(|e| stage(format!("อ่าน {name} ใน staging ไม่ได้: {e}")))?;
        std::fs::write(incoming.join(name), bytes).map_err(|e| stage(e.to_string()))?;
    }
    let manifest_bytes = std::fs::read(incoming.join("manifest.json")).map_err(|e| stage(e.to_string()))?;
    let signature = std::fs::read_to_string(incoming.join("manifest.json.sig")).map_err(|e| stage(e.to_string()))?;
    let m: Manifest = manifest::verify(&manifest_bytes, &signature, pubkey)?;
    if m.version != version {
        return Err(err("verify", "manifest_invalid", "manifest เป็นคนละรุ่นกับที่ขอ"));
    }
    let installed = Version::parse(from);
    if installed.is_some_and(|cur| m.version() <= cur) {
        return Err(err(
            "verify",
            "version_not_newer",
            format!("รุ่น {version} ไม่ใหม่กว่ารุ่นที่ติดตั้ง ({from})"),
        ));
    }
    // ขนาดต้องเท่าที่ manifest (เซ็นแล้ว ≤ 150 MB) พอดีก่อนคัดลอก และคัดลอกแบบมีเพดานกันไฟล์โตระหว่างทาง —
    // ไม่งั้น agent ที่ถูกยึดทำให้ SYSTEM คัดลอกไฟล์หลาย GB จนไดรฟ์ระบบเต็มได้
    let package = incoming.join("package.exe");
    let too_big = || err("verify", "hash_mismatch", "ขนาดตัวติดตั้งไม่ตรงกับ manifest ที่เซ็นไว้");
    let mut source =
        std::fs::File::open(src.join("package.exe")).map_err(|e| stage(format!("ไม่พบ package.exe ใน staging: {e}")))?;
    if source.metadata().map_err(|e| stage(e.to_string()))?.len() != m.size {
        return Err(too_big());
    }
    let mut target = std::fs::File::create(&package).map_err(|e| stage(e.to_string()))?;
    let copied = std::io::copy(&mut (&mut source).take(m.size + 1), &mut target).map_err(|e| stage(e.to_string()))?;
    if copied != m.size {
        return Err(too_big());
    }
    drop(target);
    let (size, digest) = hash_file(&package).map_err(|e| stage(format!("อ่านตัวติดตั้งไม่ได้: {e}")))?;
    if size != m.size || digest != m.sha256.to_lowercase() {
        return Err(err(
            "verify",
            "hash_mismatch",
            "ตัวติดตั้งไม่ตรงกับ SHA-256/ขนาดใน manifest ที่เซ็นไว้",
        ));
    }
    Ok(package)
}

fn read_bounded(path: &Path, max: u64) -> std::io::Result<Vec<u8>> {
    let mut bytes = Vec::new();
    std::fs::File::open(path)?.take(max + 1).read_to_end(&mut bytes)?;
    if bytes.len() as u64 > max {
        return Err(std::io::Error::other("ไฟล์ใหญ่เกินกำหนด"));
    }
    Ok(bytes)
}

pub(super) fn hash_file(path: &Path) -> std::io::Result<(u64, String)> {
    let mut file = std::fs::File::open(path)?;
    let (mut hasher, mut buf, mut size) = (Sha256::new(), vec![0u8; 1 << 16], 0u64);
    loop {
        let n = file.read(&mut buf)?;
        if n == 0 {
            return Ok((size, crate::hex::to_hex(&hasher.finalize())));
        }
        hasher.update(&buf[..n]);
        size += n as u64;
    }
}

fn backup_current(layout: &Layout, backup: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(backup)?;
    std::fs::copy(layout.exe(), backup.join(EXE_NAME))?;
    std::fs::copy(layout.install_json(), backup.join("install.json"))?;
    Ok(())
}

fn stage_executable(from: &Path, to: &Path) -> std::io::Result<()> {
    std::fs::copy(from, to)?;
    std::fs::File::options().write(true).open(to)?.sync_all()
}

fn rename_image(from: &Path, to: &Path) -> std::io::Result<()> {
    for _attempt in 0..20 {
        match std::fs::rename(from, to) {
            #[cfg(windows)]
            Err(error) if _attempt < 19 && matches!(error.raw_os_error(), Some(5 | 32 | 33)) => {
                std::thread::sleep(Duration::from_millis(250));
            }
            result => return result,
        }
    }
    unreachable!()
}

/// Rename the running image out of the way. Windows can rename an image whose
/// process still holds it, while deleting or replacing it in place can fail.
fn replace_executable(layout: &Layout, displaced: &Path) -> std::io::Result<()> {
    let active = layout.exe();
    let had_active = match rename_image(&active, displaced) {
        Ok(()) => true,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => false,
        Err(error) => return Err(error),
    };
    if let Err(error) = rename_image(&layout.exe_new(), &active) {
        if had_active && let Err(restore) = rename_image(displaced, &active) {
            return Err(std::io::Error::other(format!(
                "{error}; คืนไฟล์จาก {} ไม่ได้: {restore}",
                displaced.display()
            )));
        }
        return Err(error);
    }
    Ok(())
}

fn retired_image(layout: &Layout) -> PathBuf {
    layout
        .install_dir
        .join(format!("{EXE_NAME}.retired-{}", crate::hex::random_id(8)))
}

#[cfg(windows)]
pub(crate) fn clear_manual_recovery(layout: &Layout) -> std::io::Result<()> {
    let journal = layout.journal();
    let archive = layout
        .work_dir
        .join(format!("journal.before-repair-{}.json", crate::hex::random_id(8)));
    match std::fs::rename(journal, archive) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        result => result,
    }
}

#[cfg(windows)]
pub(crate) fn replace_manually(layout: &Layout, image: &Path) -> std::io::Result<()> {
    stage_executable(image, &layout.exe_new())?;
    let displaced = retired_image(layout);
    replace_executable(layout, &displaced)?;
    let _ = std::fs::remove_file(displaced);
    Ok(())
}

#[cfg(windows)]
pub(crate) fn record_manual_upgrade(layout: &Layout, from: String, version: String) -> std::io::Result<()> {
    let result = new_result(version, from, "installed", None);
    std::fs::create_dir_all(layout.status_dir())?;
    fsutil::write_json(&layout.result(), &result)?;
    History::installer(&layout.status_dir()).append(&Entry::new(&result.from, Some(&result.version), &result.outcome));
    Ok(())
}

fn clear_old_image(layout: &Layout) -> std::io::Result<()> {
    let retired = format!("{EXE_NAME}.retired-");
    for entry in std::fs::read_dir(&layout.install_dir)?.flatten() {
        let name = entry.file_name();
        if name
            .to_string_lossy()
            .strip_prefix(&retired)
            .is_some_and(|suffix| suffix.len() == 16 && suffix.bytes().all(|byte| byte.is_ascii_hexdigit()))
        {
            let _ = std::fs::remove_file(entry.path());
        }
    }
    match std::fs::remove_file(layout.exe_old()) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        // Desktop can keep the previous image mapped while a later upgrade swaps the active file.
        Err(_) => rename_image(&layout.exe_old(), &retired_image(layout)),
    }
}

/// สลับ: exe → exe.old, exe.new → exe (ทั้งคู่เป็น rename) แล้วจดรุ่นใหม่
fn swap_in(layout: &Layout, version: &str) -> Result<(), UpdateError> {
    let fail = |m: String| err("apply", "apply_failed", m);
    replace_executable(layout, &layout.exe_old()).map_err(|e| fail(format!("สลับไฟล์โปรแกรมไม่ได้: {e}")))?;
    write_install_info(layout, version).map_err(|e| fail(format!("บันทึก install.json ไม่ได้: {e}")))
}

pub(crate) fn wait_healthy(
    host: &dyn Host,
    timing: &Timing,
    version: &str,
    require_stable: bool,
) -> Result<(), UpdateError> {
    let fail = |m: String| err("health", "health_check_failed", m);
    let deadline = Instant::now() + timing.health_timeout;
    let first = loop {
        match host.health() {
            Ok(h) if h.version == version => break h,
            Ok(h) if Instant::now() >= deadline => {
                return Err(fail(format!("service ตอบเป็นรุ่น {} ไม่ใช่ {version}", h.version)));
            }
            Err(e) if Instant::now() >= deadline => return Err(fail(format!("service ไม่ตอบภายในเวลาที่กำหนด: {e}"))),
            _ => std::thread::sleep(timing.poll),
        }
    };
    if !require_stable {
        return Ok(());
    }
    // ผ่านครั้งแรกยังไม่พอ: รอแล้วตรวจซ้ำ ต้องเป็นโปรเซสเดิม (ไม่ crash แล้วถูกเริ่มใหม่วน)
    std::thread::sleep(timing.stable_for);
    match host.health() {
        Ok(h) if h.version == version && h.pid == first.pid => Ok(()),
        Ok(_) => Err(fail("service รีสตาร์ทเองหลังเริ่มทำงาน (ไม่เสถียร)".into())),
        Err(e) => Err(fail(format!("service หยุดตอบหลังเริ่มทำงาน: {e}"))),
    }
}

/// กู้แล้วจบด้วยผล "rolled_back" ที่บอกสาเหตุเดิม หรือ "failed/rollback_failed" ถ้ากู้ไม่ได้
fn finish_with_rollback(
    layout: &Layout,
    host: &dyn Host,
    timing: &Timing,
    journal: &Journal,
    cause: UpdateError,
) -> (&'static str, UpdateError) {
    match rollback(layout, host, timing, journal) {
        Ok(()) => ("rolled_back", cause),
        Err(mut e) => {
            e.message = format!("{} · สาเหตุที่เริ่มกู้คืน: {}", e.message, cause.message);
            ("failed", e)
        }
    }
}

/// คืนรุ่นเดิม: ไฟล์ exe และ install.json จากสำรอง แล้วเริ่ม agent และตรวจสุขภาพ (ค่าตั้งของ agent ให้ agent ใส่คืนเอง — ตัวติดตั้งไม่แตะ)
/// ทำซ้ำได้ (idempotent) เพราะ recover() อาจเรียกหลังถูกขัดจังหวะที่จุดใดก็ได้
fn rollback(layout: &Layout, host: &dyn Host, timing: &Timing, journal: &Journal) -> Result<(), UpdateError> {
    let backup = layout.backups().join(&journal.from);
    let fail = |m: String| {
        err(
            "rollback",
            "rollback_failed",
            format!("กู้รุ่นเดิมไม่สำเร็จ: {m} — ตัวติดตั้งรุ่นเดิมสำรองไว้ที่ {}", backup.display()),
        )
    };
    let previous = installed_version_from(&backup.join("install.json"))
        .filter(|version| version == &journal.from)
        .ok_or_else(|| fail("ข้อมูลรุ่นในไฟล์สำรองไม่ตรงกับ journal".into()))?;
    stage_executable(&backup.join(EXE_NAME), &layout.exe_new()).map_err(|e| fail(format!("อ่านสำรองไม่ได้: {e}")))?;
    host.stop_agent().map_err(|e| fail(format!("หยุด service ไม่ได้: {e}")))?;
    let displaced = retired_image(layout);
    replace_executable(layout, &displaced).map_err(|e| fail(format!("วางไฟล์เดิมคืนไม่ได้: {e}")))?;
    write_install_info(layout, &previous).map_err(|e| fail(e.to_string()))?;
    let _ = std::fs::remove_file(displaced);
    let _ = std::fs::remove_file(layout.exe_old());

    host.start_agent()
        .map_err(|e| fail(format!("เริ่ม service รุ่นเดิมไม่ได้: {e}")))?;
    wait_healthy(host, timing, &journal.from, false).map_err(|e| fail(e.message))?;
    let _ = std::fs::remove_file(layout.journal());
    Ok(())
}

fn prune_backups(layout: &Layout) {
    let Ok(dir) = std::fs::read_dir(layout.backups()) else {
        return;
    };
    let mut versions: Vec<(Version, PathBuf)> = dir
        .flatten()
        .filter_map(|e| Some((Version::parse(e.file_name().to_str()?)?, e.path())))
        .collect();
    versions.sort();
    let excess = versions.len().saturating_sub(KEEP_BACKUPS);
    for (_, path) in versions.into_iter().take(excess) {
        let _ = std::fs::remove_dir_all(path);
    }
}

#[cfg(test)]
mod tests;
