//! ทดสอบเครื่องมือผู้เผยแพร่กับ exe จริงที่ build จาก repo นี้: keygen → package → verify และกรณีผิดพลาดที่ต้องไม่ผ่าน

use std::{path::Path, process::Command};

const RELEASE: &str = env!("CARGO_BIN_EXE_synapta-release");
const AGENT: &str = env!("CARGO_BIN_EXE_synapta-connect");

fn release(args: &[&str]) -> (bool, String) {
    let out = Command::new(RELEASE).args(args).output().unwrap();
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    (out.status.success(), text)
}

fn path(p: &Path) -> &str {
    p.to_str().unwrap()
}

#[test]
fn package_and_verify_roundtrip_and_failures() {
    let root = std::env::temp_dir().join(format!("synapta-reltool-{}", synapta_connect::hex::random_id(4)));
    let (keys, out) = (root.join("keys"), root.join("out"));
    let version = env!("CARGO_PKG_VERSION");

    let (ok, text) = release(&["keygen", "--out", path(&keys)]);
    assert!(ok, "{text}");
    let pubkey = text.lines().last().unwrap().trim().to_string();
    assert_eq!(pubkey.len(), 64);
    let key = keys.join("synapta-release.key");
    assert!(!release(&["keygen", "--out", path(&keys)]).0, "ต้องไม่เขียนทับกุญแจเดิม");

    // ประกาศเลขรุ่นที่ไม่ตรงกับตัว exe ต้องถูกปฏิเสธ (ตัวติดตั้งตรวจรุ่นใหม่จากเลขนี้)
    let (ok, text) = release(&[
        "package",
        "--exe",
        AGENT,
        "--version",
        "9.9.9",
        "--key",
        path(&key),
        "--out",
        path(&out),
    ]);
    assert!(!ok && text.contains("รายงานว่า"), "{text}");

    let (ok, text) = release(&[
        "package",
        "--exe",
        AGENT,
        "--version",
        version,
        "--key",
        path(&key),
        "--out",
        path(&out),
        "--notes",
        "ทดสอบ",
    ]);
    assert!(ok, "{text}");
    let asset = out.join(format!("SynaptaConnect-v{version}.exe"));
    assert!(asset.exists() && out.join("manifest.json").exists() && out.join("manifest.json.sig").exists());

    let (ok, text) = release(&["verify", "--dir", path(&out), "--pubkey", &pubkey]);
    assert!(ok, "{text}");
    assert!(
        !release(&["verify", "--dir", path(&out), "--pubkey", &"11".repeat(32)]).0,
        "คีย์อื่นต้องไม่ผ่าน"
    );

    // แก้ตัวติดตั้งหนึ่งไบต์ → ไม่ผ่าน
    let mut bytes = std::fs::read(&asset).unwrap();
    bytes[100] ^= 0xff;
    std::fs::write(&asset, bytes).unwrap();
    assert!(!release(&["verify", "--dir", path(&out), "--pubkey", &pubkey]).0);

    std::fs::remove_dir_all(root).unwrap();
}
