//! เครื่องมือของผู้เผยแพร่ (ไม่ถูกติดตั้งให้ผู้ใช้): สร้างกุญแจเซ็น, ทำ release (manifest + ลายเซ็น), ตรวจ release
//!
//!   synapta-release keygen --out DIR
//!   synapta-release package --exe PATH --version X.Y.Z --key PRIVATE_KEY_FILE --out DIR [--notes TEXT]
//!   synapta-release verify --dir DIR --pubkey HEX
//!   synapta-release package-panel --web DIR --version X.Y.Z --key PRIVATE_KEY_FILE --out DIR [--panel-api N] [--notes TEXT]
//!   synapta-release verify-panel --dir DIR --pubkey HEX
//!
//! private key อยู่ในไฟล์เดียว (hex ของ PKCS#8) เก็บนอก repo — ใครถือไฟล์นี้ = ออกอัปเดตให้ทุกเครื่องได้

use ring::{
    rand::SystemRandom,
    signature::{Ed25519KeyPair, KeyPair},
};
use serde_json::json;
use sha2::{Digest, Sha256};
use std::{path::PathBuf, process::Command};
use synapta_connect::{
    hex::to_hex,
    update::{
        manifest::{self, Manifest, PLATFORM, decode_hex, parse_hex32},
        version::Version,
        web_package::{self, Manifest as PanelManifest},
    },
};

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let result = match args.first().map(String::as_str) {
        Some("keygen") => keygen(&args[1..]),
        Some("package") => package(&args[1..]),
        Some("verify") => verify(&args[1..]),
        Some("package-panel") => package_panel(&args[1..]),
        Some("verify-panel") => verify_panel(&args[1..]),
        _ => Err("คำสั่ง: keygen | package | verify | package-panel | verify-panel".into()),
    };
    if let Err(e) = result {
        eprintln!("ผิดพลาด: {e}");
        std::process::exit(1);
    }
}

fn flag(args: &[String], name: &str) -> Result<String, String> {
    args.iter()
        .position(|a| a == name)
        .and_then(|i| args.get(i + 1).cloned())
        .ok_or(format!("ต้องมี {name} <ค่า>"))
}

fn keygen(args: &[String]) -> Result<(), String> {
    let out = PathBuf::from(flag(args, "--out")?);
    std::fs::create_dir_all(&out).map_err(|e| e.to_string())?;
    let file = out.join("synapta-release.key");
    if file.exists() {
        return Err(format!("{} มีอยู่แล้ว — ไม่เขียนทับกุญแจเดิม", file.display()));
    }
    let pkcs8 = Ed25519KeyPair::generate_pkcs8(&SystemRandom::new()).map_err(|e| e.to_string())?;
    let pair = Ed25519KeyPair::from_pkcs8(pkcs8.as_ref()).map_err(|e| e.to_string())?;
    synapta_connect::platform::write_private(&file, to_hex(pkcs8.as_ref()).as_bytes()).map_err(|e| e.to_string())?;
    println!("สร้างกุญแจเซ็นที่ {} (เก็บนอก repo สำรองไว้ 2 ที่ ห้ามส่งต่อ)", file.display());
    println!(
        "public key (ใส่ใน backend/release/pubkey.hex แล้ว build ใหม่):\n{}",
        to_hex(pair.public_key().as_ref())
    );
    Ok(())
}

fn load_key(path: &str) -> Result<Ed25519KeyPair, String> {
    let text = std::fs::read_to_string(path).map_err(|e| format!("อ่านกุญแจไม่ได้: {e}"))?;
    let bytes = decode_hex(text.trim()).ok_or("ไฟล์กุญแจไม่ใช่ hex")?;
    Ed25519KeyPair::from_pkcs8(&bytes).map_err(|e| format!("กุญแจไม่ถูกต้อง: {e}"))
}

fn package(args: &[String]) -> Result<(), String> {
    let exe = PathBuf::from(flag(args, "--exe")?);
    let version = flag(args, "--version")?;
    let out = PathBuf::from(flag(args, "--out")?);
    let key = load_key(&flag(args, "--key")?)?;
    let notes = flag(args, "--notes").unwrap_or_default();
    Version::parse(&version).ok_or("--version ต้องเป็นรูป x.y.z")?;

    // exe ต้องรายงานเลขรุ่นตรงกับที่จะประกาศ — ตัวติดตั้งใช้เลขนี้ตัดสินว่ารุ่นใหม่เริ่มขึ้นจริง (health check)
    let reported = Command::new(&exe)
        .arg("--version")
        .output()
        .map_err(|e| format!("รัน {} ไม่ได้: {e}", exe.display()))?;
    let reported = String::from_utf8_lossy(&reported.stdout).trim().to_string();
    // ข้ามได้เฉพาะตอนทดสอบการกู้คืน (ทำ release ปลอมที่ตั้งใจให้ใช้ไม่ได้) — ห้ามใช้กับ release จริง
    let skip_check = args.iter().any(|a| a == "--skip-version-check-for-tests");
    if !skip_check && reported != format!("synapta-connect {version}") {
        return Err(format!(
            "exe รายงานว่า '{reported}' แต่ประกาศรุ่น {version} — build ใหม่หรือแก้ --version"
        ));
    }

    let bytes = std::fs::read(&exe).map_err(|e| e.to_string())?;
    let asset = Manifest::asset_name(&version);
    let manifest = json!({
        "version": version, "asset": asset, "sha256": to_hex(&Sha256::digest(&bytes)), "size": bytes.len(),
        "platform": PLATFORM, "notes": notes,
        "published_at": utc_now_iso(),
    })
    .to_string();
    std::fs::create_dir_all(&out).map_err(|e| e.to_string())?;
    std::fs::write(out.join(&asset), &bytes).map_err(|e| e.to_string())?;
    std::fs::write(out.join("manifest.json"), &manifest).map_err(|e| e.to_string())?;
    std::fs::write(
        out.join("manifest.json.sig"),
        to_hex(key.sign(manifest.as_bytes()).as_ref()),
    )
    .map_err(|e| e.to_string())?;

    // ตรวจกลับด้วยกุญแจสาธารณะของตัวเองก่อนบอกว่าเสร็จ
    let public: [u8; 32] = key.public_key().as_ref().try_into().map_err(|_| "public key ยาวผิด")?;
    manifest::verify(
        manifest.as_bytes(),
        &std::fs::read_to_string(out.join("manifest.json.sig")).unwrap_or_default(),
        &public,
    )
    .map_err(|e| e.to_string())?;
    println!(
        "สร้าง release {version} ที่ {} — อัปโหลด 3 ไฟล์นี้เป็น assets ของ GitHub Release:",
        out.display()
    );
    println!("  {asset}\n  manifest.json\n  manifest.json.sig");
    Ok(())
}

/// เวลา UTC แบบ ISO-8601 โดยไม่ต้องพึ่ง crate วันที่ (ใช้แค่แสดงผลใน panel)
fn utc_now_iso() -> String {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs());
    let (days, rem) = (secs / 86400, secs % 86400);
    // civil-from-days (Howard Hinnant)
    let z = days as i64 + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    format!(
        "{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}Z",
        rem / 3600,
        rem % 3600 / 60,
        rem % 60
    )
}

fn verify(args: &[String]) -> Result<(), String> {
    let dir = PathBuf::from(flag(args, "--dir")?);
    let public = parse_hex32(&flag(args, "--pubkey")?).ok_or("--pubkey ต้องเป็น hex 64 ตัว")?;
    let manifest_bytes = std::fs::read(dir.join("manifest.json")).map_err(|e| e.to_string())?;
    let signature = std::fs::read_to_string(dir.join("manifest.json.sig")).map_err(|e| e.to_string())?;
    let m = manifest::verify(&manifest_bytes, &signature, &public).map_err(|e| e.to_string())?;
    let bytes = std::fs::read(dir.join(&m.asset)).map_err(|e| e.to_string())?;
    if bytes.len() as u64 != m.size || to_hex(&Sha256::digest(&bytes)) != m.sha256 {
        return Err("ตัวติดตั้งไม่ตรงกับ manifest".into());
    }
    println!("release {} ถูกต้อง (ลายเซ็น, SHA-256, ขนาด)", m.version);
    Ok(())
}

fn package_panel(args: &[String]) -> Result<(), String> {
    let dir = PathBuf::from(flag(args, "--web")?);
    let out = PathBuf::from(flag(args, "--out")?);
    let version = flag(args, "--version")?;
    let parsed = Version::parse(&version).ok_or("--version ต้องเป็นรูป x.y.z")?;
    if version != parsed.to_string() {
        return Err("เลขรุ่นต้องใช้รูป x.y.z แบบไม่มีเลขศูนย์นำหน้า".into());
    }
    let api = if args.iter().any(|arg| arg == "--panel-api") {
        flag(args, "--panel-api")?
            .parse::<u32>()
            .map_err(|_| "--panel-api ต้องเป็นจำนวนเต็มบวก")?
    } else {
        web_package::API
    };
    let key = load_key(&flag(args, "--key")?)?;
    let bytes = web_package::collect(&dir).map_err(|e| format!("{e:#}"))?;
    let manifest = PanelManifest {
        asset: PanelManifest::asset_name(&version),
        version,
        panel_api: api,
        sha256: to_hex(&Sha256::digest(&bytes)),
        size: bytes.len() as u64,
        notes: flag(args, "--notes").unwrap_or_default(),
    };
    let manifest_bytes = serde_json::to_vec(&manifest).map_err(|e| e.to_string())?;
    let signature = to_hex(key.sign(&manifest_bytes).as_ref());
    let public = key.public_key().as_ref().try_into().map_err(|_| "public key ยาวผิด")?;
    PanelManifest::verify(&manifest_bytes, &signature, public).map_err(|e| e.to_string())?;
    std::fs::create_dir_all(&out).map_err(|e| e.to_string())?;
    for (name, contents) in [
        (manifest.asset.as_str(), bytes.as_slice()),
        (web_package::SIGNATURE, signature.as_bytes()),
        (web_package::MANIFEST, manifest_bytes.as_slice()),
    ] {
        synapta_connect::fsutil::atomic_write(&out.join(name), contents).map_err(|e| e.to_string())?;
    }
    println!("เว็บรุ่น {} · Panel API {} · {}", manifest.version, api, out.display());
    Ok(())
}

fn verify_panel(args: &[String]) -> Result<(), String> {
    let dir = PathBuf::from(flag(args, "--dir")?);
    let public = parse_hex32(&flag(args, "--pubkey")?).ok_or("--pubkey ต้องเป็น hex 64 ตัว")?;
    let limit = manifest::MAX_MANIFEST_BYTES as u64;
    let bytes = web_package::read_bounded(&dir.join(web_package::MANIFEST), limit).map_err(|e| e.to_string())?;
    let sig = web_package::read_bounded(&dir.join(web_package::SIGNATURE), 256).map_err(|e| e.to_string())?;
    let m = PanelManifest::verify(&bytes, std::str::from_utf8(&sig).map_err(|e| e.to_string())?, &public)
        .map_err(|e| e.to_string())?;
    let bytes = web_package::read_bounded(&dir.join(&m.asset), m.size).map_err(|e| e.to_string())?;
    web_package::verify_payload(&m, &bytes).map_err(|e| e.to_string())?;
    println!(
        "เว็บรุ่น {} ถูกต้อง (Ed25519, SHA-256, ขนาด และโครงไฟล์) · Panel API {}",
        m.version, m.panel_api
    );
    Ok(())
}
