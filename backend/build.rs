//! ฝัง public key ของผู้เผยแพร่เข้าโปรแกรม: ตัวแปร SYNAPTA_RELEASE_PUBKEY (hex 64 ตัว) ถ้ามี ไม่งั้นไฟล์ release/pubkey.hex
//! (ให้ CI/ผู้เผยแพร่ใส่กุญแจตอน build โดยไม่ต้องแก้ไฟล์ใน repo; รุ่นทดสอบใช้กุญแจทดสอบได้โดยไม่ปนกับของจริง)

fn main() {
    println!("cargo:rerun-if-env-changed=SYNAPTA_RELEASE_PUBKEY");
    println!("cargo:rerun-if-changed=../frontend");
    println!("cargo:rerun-if-changed=release/pubkey.hex");
    let key = std::env::var("SYNAPTA_RELEASE_PUBKEY")
        .unwrap_or_else(|_| std::fs::read_to_string("release/pubkey.hex").unwrap_or_default());
    let key = key.trim();
    let valid = key.len() == 64 && key.bytes().all(|b| b.is_ascii_hexdigit());
    assert!(valid, "public key ของผู้เผยแพร่ต้องเป็น hex 64 ตัว (ดู release/README.md)");
    let out = std::path::PathBuf::from(std::env::var("OUT_DIR").unwrap()).join("pubkey.hex");
    std::fs::write(out, key).unwrap();
}
