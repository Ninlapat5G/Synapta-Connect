use super::{error::UpdateError, manifest, version::Version};
use base64::{Engine, engine::general_purpose::STANDARD};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{collections::BTreeMap, io::Read, path::Path};

pub const MANIFEST: &str = "panel-manifest.json";
pub const SIGNATURE: &str = "panel-manifest.json.sig";
pub const MAX_BYTES: u64 = 8 * 1024 * 1024;
pub const API: u32 = 1;
pub const BASELINE: &str = env!("SYNAPTA_PANEL_VERSION");

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Manifest {
    pub version: String,
    pub panel_api: u32,
    pub asset: String,
    pub sha256: String,
    pub size: u64,
    #[serde(default)]
    pub notes: String,
}

impl Manifest {
    pub fn asset_name(version: &str) -> String {
        format!("SynaptaConnect-Panel-v{version}.json")
    }

    pub fn verify(bytes: &[u8], signature: &str, key: &[u8; 32]) -> Result<Self, UpdateError> {
        manifest::verify_signature(bytes, signature, key)?;
        let info: Self = serde_json::from_slice(bytes).map_err(|e| invalid(format!("อ่าน manifest ของเว็บไม่ได้: {e}")))?;
        if Version::parse(&info.version).is_none_or(|version| info.asset != Self::asset_name(&version.to_string()))
            || info.panel_api == 0
            || info.size == 0
            || info.size > MAX_BYTES
            || manifest::parse_hex32(&info.sha256).is_none()
        {
            return Err(invalid("ข้อมูลรุ่น, API, ขนาด หรือ SHA-256 ของเว็บผิดรูป"));
        }
        Ok(info)
    }

    pub fn compatible(&self) -> Result<(), UpdateError> {
        if self.panel_api != API {
            return Err(UpdateError::new(
                "verify",
                "panel_incompatible",
                format!("เว็บต้องใช้ Panel API {} แต่ Agent นี้รองรับ API {API}", self.panel_api),
            ));
        }
        Ok(())
    }
}

pub struct Bundle {
    files: BTreeMap<String, Vec<u8>>,
}

impl Bundle {
    pub fn decode(bytes: &[u8]) -> Result<Self, UpdateError> {
        if bytes.len() as u64 > MAX_BYTES {
            return Err(invalid("แพ็กเกจเว็บใหญ่เกินกำหนด"));
        }
        let encoded: BTreeMap<String, String> =
            serde_json::from_slice(bytes).map_err(|e| invalid(format!("อ่านแพ็กเกจเว็บไม่ได้: {e}")))?;
        if encoded.len() > 256 {
            return Err(invalid("แพ็กเกจเว็บมีไฟล์มากเกินกำหนด"));
        }
        let mut files = BTreeMap::new();
        for (path, data) in encoded {
            if content_type(&path).is_none() {
                return Err(invalid(format!("แพ็กเกจมีชื่อไฟล์ไม่รองรับ: {path}")));
            }
            let data = STANDARD
                .decode(data)
                .map_err(|_| invalid("ข้อมูลไฟล์ภายในแพ็กเกจเว็บไม่ถูกต้อง"))?;
            files.insert(path, data);
        }
        let html_text = files
            .get("index.html")
            .and_then(|bytes| std::str::from_utf8(bytes).ok())
            .ok_or_else(|| invalid("เว็บไม่มี index.html แบบ UTF-8"))?;
        let mut script = false;
        let mut style = false;
        for part in html_text.split('"').skip(1).step_by(2) {
            if let Some(path) = part
                .strip_prefix(crate::panel::PATH)
                .filter(|path| path.starts_with("assets/"))
            {
                if !files.get(path).is_some_and(|data| !data.is_empty()) {
                    return Err(invalid(format!("index.html อ้างถึงไฟล์ที่ขาดหาย: {path}")));
                }
                script |= path.ends_with(".js");
                style |= path.ends_with(".css");
            }
        }
        if !script || !style {
            return Err(invalid("index.html ต้องโหลด JavaScript และ CSS จากเส้นทางของ panel"));
        }
        Ok(Self { files })
    }

    pub fn file(&self, path: &str) -> Option<(&'static str, Vec<u8>)> {
        Some((content_type(path)?, self.files.get(path)?.clone()))
    }
}

pub fn verify_payload(info: &Manifest, bytes: &[u8]) -> Result<Bundle, UpdateError> {
    if bytes.len() as u64 != info.size || crate::hex::to_hex(&Sha256::digest(bytes)) != info.sha256.to_ascii_lowercase()
    {
        return Err(UpdateError::new(
            "verify",
            "hash_mismatch",
            "แพ็กเกจเว็บไม่ตรงกับ SHA-256 หรือขนาดที่เซ็นไว้",
        ));
    }
    Bundle::decode(bytes)
}

pub fn collect(dir: &Path) -> anyhow::Result<Vec<u8>> {
    let mut files = BTreeMap::new();
    files.insert(
        "index.html".to_string(),
        STANDARD.encode(read_bounded(&dir.join("index.html"), MAX_BYTES)?),
    );
    for entry in std::fs::read_dir(dir.join("assets"))? {
        let entry = entry?;
        let name = entry
            .file_name()
            .into_string()
            .map_err(|_| anyhow::anyhow!("invalid asset filename"))?;
        let path = format!("assets/{name}");
        anyhow::ensure!(
            entry.file_type()?.is_file() && content_type(&path).is_some(),
            "unsupported panel asset: {path}"
        );
        files.insert(path, STANDARD.encode(read_bounded(&entry.path(), MAX_BYTES)?));
    }
    let bytes = serde_json::to_vec(&files)?;
    Bundle::decode(&bytes).map_err(|e| anyhow::anyhow!("{e}"))?;
    Ok(bytes)
}

pub fn read_bounded(path: &Path, max: u64) -> std::io::Result<Vec<u8>> {
    let mut bytes = Vec::new();
    std::fs::File::open(path)?.take(max + 1).read_to_end(&mut bytes)?;
    if bytes.len() as u64 > max {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "file exceeds size limit",
        ));
    }
    Ok(bytes)
}

fn content_type(path: &str) -> Option<&'static str> {
    if path == "index.html" {
        return Some("text/html; charset=utf-8");
    }
    let name = path.strip_prefix("assets/")?;
    let (stem, extension) = name.rsplit_once('.')?;
    if stem.is_empty()
        || stem.len() > 140
        || !stem
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
    {
        return None;
    }
    match extension {
        "js" => Some("text/javascript; charset=utf-8"),
        "css" => Some("text/css; charset=utf-8"),
        "woff2" => Some("font/woff2"),
        "webp" => Some("image/webp"),
        "png" => Some("image/png"),
        "jpg" | "jpeg" => Some("image/jpeg"),
        "svg" => Some("image/svg+xml"),
        _ => None,
    }
}

fn invalid(message: impl Into<String>) -> UpdateError {
    UpdateError::new("verify", "manifest_invalid", message)
}
