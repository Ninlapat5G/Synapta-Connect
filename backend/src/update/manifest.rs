//! manifest ของ release + การตรวจลายเซ็น Ed25519
//!
//! ลายเซ็นครอบ "ไบต์ดิบของ manifest.json ทั้งไฟล์" (ไม่ canonicalize JSON) → ตรวจก่อน parse เสมอ
//! manifest ผูก: เลขรุ่น, ชื่อไฟล์, SHA-256 และขนาดของตัวติดตั้ง — แก้อย่างใดอย่างหนึ่งลายเซ็นไม่ผ่าน

use super::{error::UpdateError, version::Version};
use ring::signature::{ED25519, UnparsedPublicKey};
use serde::{Deserialize, Serialize};

/// ตัวติดตั้งใหญ่เกินนี้ถูกปฏิเสธก่อนดาวน์โหลด (exe ปัจจุบัน ~10 MB)
pub const MAX_PACKAGE_BYTES: u64 = 150 * 1024 * 1024;
pub const MAX_MANIFEST_BYTES: usize = 64 * 1024;
pub const PLATFORM: &str = "windows-x86_64";

/// public key ของผู้เผยแพร่ที่ฝังตอน build (build.rs: SYNAPTA_RELEASE_PUBKEY หรือ release/pubkey.hex); ศูนย์ล้วน = ยังไม่ตั้ง
const EMBEDDED_PUBKEY: &str = include_str!(concat!(env!("OUT_DIR"), "/pubkey.hex"));

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Manifest {
    pub version: String,
    /// ต้องเป็น `SynaptaConnect-v<version>.exe` พอดี
    pub asset: String,
    pub sha256: String,
    pub size: u64,
    pub platform: String,
    #[serde(default)]
    pub notes: String,
    #[serde(default)]
    pub published_at: String,
}

impl Manifest {
    pub fn version(&self) -> Version {
        Version::parse(&self.version).expect("verify() ตรวจเลขรุ่นแล้ว")
    }
    pub fn asset_name(version: &str) -> String {
        format!("SynaptaConnect-v{version}.exe")
    }
}

/// public key ที่ฝังไว้ — None เมื่อ build นี้ยังไม่ได้ตั้งค่า (ปิดการติดตั้งอัปเดต)
pub fn embedded_pubkey() -> Option<[u8; 32]> {
    configured_pubkey(EMBEDDED_PUBKEY)
}

/// ศูนย์ล้วนคือค่าว่างใน release/pubkey.hex
fn configured_pubkey(text: &str) -> Option<[u8; 32]> {
    let key = parse_hex32(text.trim())?;
    (key != [0u8; 32]).then_some(key)
}

pub fn parse_hex32(text: &str) -> Option<[u8; 32]> {
    decode_hex(text)?.try_into().ok()
}

pub fn decode_hex(text: &str) -> Option<Vec<u8>> {
    if !text.len().is_multiple_of(2) || !text.is_ascii() {
        return None;
    }
    (0..text.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&text[i..i + 2], 16).ok())
        .collect()
}

fn bad(code: &str, message: impl Into<String>) -> UpdateError {
    UpdateError::new("verify", code, message)
}

/// ตรวจลายเซ็นก่อน แล้วค่อยอ่านเนื้อ — manifest ที่ลายเซ็นไม่ผ่านไม่ถูก parse เลย
pub fn verify(manifest_bytes: &[u8], signature_hex: &str, pubkey: &[u8; 32]) -> Result<Manifest, UpdateError> {
    verify_signature(manifest_bytes, signature_hex, pubkey)?;
    let m: Manifest = serde_json::from_slice(manifest_bytes)
        .map_err(|e| bad("manifest_invalid", format!("อ่าน manifest ไม่ได้: {e}")))?;
    validate(m)
}

pub fn verify_signature(manifest_bytes: &[u8], signature_hex: &str, pubkey: &[u8; 32]) -> Result<(), UpdateError> {
    if manifest_bytes.len() > MAX_MANIFEST_BYTES {
        return Err(bad("manifest_invalid", "manifest ใหญ่เกินกำหนด"));
    }
    let signature = decode_hex(signature_hex.trim())
        .filter(|s| s.len() == 64)
        .ok_or_else(|| bad("signature_invalid", "ไฟล์ลายเซ็นผิดรูป — ไม่ติดตั้ง"))?;
    UnparsedPublicKey::new(&ED25519, pubkey)
        .verify(manifest_bytes, &signature)
        .map_err(|_| {
            bad(
                "signature_invalid",
                "ลายเซ็นของ release ไม่ตรงกับ public key ของผู้เผยแพร่ — ไม่ติดตั้ง",
            )
        })
}

fn validate(m: Manifest) -> Result<Manifest, UpdateError> {
    let version = Version::parse(&m.version)
        .ok_or_else(|| bad("manifest_invalid", format!("เลขรุ่น '{}' ไม่ใช่รูป x.y.z", m.version)))?;
    if m.asset != Manifest::asset_name(&version.to_string()) {
        return Err(bad("manifest_invalid", "ชื่อไฟล์ตัวติดตั้งไม่ตรงกับเลขรุ่น"));
    }
    if m.platform != PLATFORM {
        return Err(bad(
            "manifest_invalid",
            format!("release นี้สำหรับ '{}' ไม่ใช่ {PLATFORM}", m.platform),
        ));
    }
    if m.sha256.len() != 64 || decode_hex(&m.sha256).is_none() {
        return Err(bad("manifest_invalid", "sha256 ใน manifest ผิดรูป"));
    }
    if m.size == 0 || m.size > MAX_PACKAGE_BYTES {
        return Err(bad("manifest_invalid", "ขนาดตัวติดตั้งใน manifest ไม่สมเหตุสมผล"));
    }
    Ok(m)
}

#[cfg(test)]
pub(crate) mod testkit {
    //! คีย์/release ปลอมสำหรับเทสต์ — ใช้ร่วมกับเทสต์ของ source / installer / manager
    use super::*;
    use ring::{
        rand::SystemRandom,
        signature::{Ed25519KeyPair, KeyPair},
    };

    pub struct Signer(Ed25519KeyPair);

    impl Signer {
        pub fn new() -> Signer {
            let pkcs8 = Ed25519KeyPair::generate_pkcs8(&SystemRandom::new()).unwrap();
            Signer(Ed25519KeyPair::from_pkcs8(pkcs8.as_ref()).unwrap())
        }
        pub fn public(&self) -> [u8; 32] {
            self.0.public_key().as_ref().try_into().unwrap()
        }
        pub fn sign_hex(&self, data: &[u8]) -> String {
            crate::hex::to_hex(self.0.sign(data).as_ref())
        }
        /// manifest + ลายเซ็น สำหรับ "exe" ที่มีเนื้อหา `payload`
        pub fn release(&self, version: &str, payload: &[u8]) -> (Vec<u8>, String) {
            use sha2::{Digest, Sha256};
            let manifest = serde_json::json!({
                "version": version, "asset": Manifest::asset_name(version),
                "sha256": crate::hex::to_hex(&Sha256::digest(payload)), "size": payload.len(),
                "platform": PLATFORM, "notes": "test", "published_at": "2026-11-20T00:00:00Z",
            })
            .to_string()
            .into_bytes();
            let sig = self.sign_hex(&manifest);
            (manifest, sig)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{testkit::Signer, *};

    #[test]
    fn accepts_valid_and_rejects_every_kind_of_tampering() {
        let signer = Signer::new();
        let (manifest, sig) = signer.release("0.3.1", b"exe-bytes");
        let m = verify(&manifest, &sig, &signer.public()).unwrap();
        assert_eq!(m.version(), Version(0, 3, 1));

        // ลายเซ็นจากคีย์อื่น
        let other = Signer::new();
        assert_eq!(
            verify(&manifest, &sig, &other.public()).unwrap_err().code,
            "signature_invalid"
        );
        // แก้ manifest หนึ่งไบต์
        let mut tampered = manifest.clone();
        *tampered.iter_mut().find(|b| **b == b'1').unwrap() = b'2';
        assert_eq!(
            verify(&tampered, &sig, &signer.public()).unwrap_err().code,
            "signature_invalid"
        );
        // ลายเซ็นผิดรูป
        assert_eq!(
            verify(&manifest, "zz", &signer.public()).unwrap_err().code,
            "signature_invalid"
        );
    }

    #[test]
    fn signed_but_malformed_manifest_is_rejected() {
        let signer = Signer::new();
        let cases = [
            r#"{"version":"1.0.0","asset":"../evil.exe","sha256":"00","size":1,"platform":"windows-x86_64"}"#,
            r#"{"version":"1.0","asset":"SynaptaConnect-v1.0.exe","sha256":"","size":1,"platform":"windows-x86_64"}"#,
            r#"{"version":"1.0.0","asset":"SynaptaConnect-v1.0.0.exe","sha256":"zz","size":1,"platform":"windows-x86_64"}"#,
            r#"{"version":"1.0.0","asset":"SynaptaConnect-v1.0.0.exe","sha256":"","size":1,"platform":"linux"}"#,
            "not json",
        ];
        for body in cases {
            let sig = signer.sign_hex(body.as_bytes());
            let err = verify(body.as_bytes(), &sig, &signer.public()).unwrap_err();
            assert_eq!(err.code, "manifest_invalid", "{body}");
        }
    }

    #[test]
    fn placeholder_pubkey_means_not_configured() {
        assert_eq!(configured_pubkey(&"0".repeat(64)), None);
        assert_eq!(configured_pubkey(&"ab".repeat(32)), Some([0xab; 32]));
        assert_eq!(parse_hex32(&"ab".repeat(32)), Some([0xab; 32]));
        assert_eq!(parse_hex32("abc"), None);
    }
}
