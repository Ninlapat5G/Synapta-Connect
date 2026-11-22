//! ฝัง public key ของผู้เผยแพร่เข้าโปรแกรม: ตัวแปร SYNAPTA_RELEASE_PUBKEY (hex 64 ตัว) ถ้ามี ไม่งั้นไฟล์ release/pubkey.hex
//! (ให้ CI/ผู้เผยแพร่ใส่กุญแจตอน build โดยไม่ต้องแก้ไฟล์ใน repo; รุ่นทดสอบใช้กุญแจทดสอบได้โดยไม่ปนกับของจริง)

fn main() {
    #[cfg(feature = "desktop")]
    tauri_build::try_build(
        tauri_build::Attributes::new()
            .windows_attributes(
                tauri_build::WindowsAttributes::new()
                    .window_icon_path("../installer/windows/assets/synapta-connect.ico"),
            )
            .app_manifest(tauri_build::AppManifest::new().commands(&[
                "agent_request",
                "service_status",
                "service_control",
                "open_web",
            ])),
    )
    .expect("build desktop configuration");

    println!("cargo:rerun-if-env-changed=SYNAPTA_RELEASE_PUBKEY");
    println!("cargo:rerun-if-changed=../frontend/dist");
    println!("cargo:rerun-if-changed=../frontend/package.json");
    println!("cargo:rerun-if-changed=release/pubkey.hex");
    assert!(
        std::path::Path::new("../frontend/dist/index.html").is_file(),
        "Build the panel first: npm ci --prefix ../frontend && npm run build --prefix ../frontend"
    );
    let panel_package = std::fs::read_to_string("../frontend/package.json").expect("missing frontend/package.json");
    let panel_version = panel_package
        .lines()
        .find_map(|line| line.trim().strip_prefix("\"version\": \""))
        .and_then(|tail| tail.split_once('"'))
        .map(|(version, _)| version)
        .expect("set the panel version in frontend/package.json");
    println!("cargo:rustc-env=SYNAPTA_PANEL_VERSION={panel_version}");
    let key = std::env::var("SYNAPTA_RELEASE_PUBKEY")
        .unwrap_or_else(|_| std::fs::read_to_string("release/pubkey.hex").unwrap_or_default());
    let key = key.trim();
    let valid = key.len() == 64 && key.bytes().all(|b| b.is_ascii_hexdigit());
    assert!(valid, "public key ของผู้เผยแพร่ต้องเป็น hex 64 ตัว (ดู release/README.md)");
    let out = std::path::PathBuf::from(std::env::var("OUT_DIR").unwrap()).join("pubkey.hex");
    std::fs::write(out, key).unwrap();
}
