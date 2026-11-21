//! แหล่ง release: GitHub Releases, โฟลเดอร์ HTTPS ใดก็ได้, หรือโฟลเดอร์ในเครื่อง (ไว้ทดสอบโดยไม่ต้องมี repo จริง)
//!
//! ทุกแหล่งมีไฟล์ชุดเดียวกัน: `manifest.json`, `manifest.json.sig`, และตัวติดตั้งตามชื่อใน manifest
//! ไฟล์นี้แค่ "เอาไฟล์มา" — ไม่เชื่อเนื้อหา การตรวจลายเซ็น/hash/เวอร์ชันอยู่ที่ manifest.rs และ installer.rs

use super::{error::UpdateError, manifest::MAX_MANIFEST_BYTES};
use serde_json::Value;
use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    time::Duration,
};
use tokio::io::AsyncWriteExt;

pub const MANIFEST_FILE: &str = "manifest.json";
pub const SIGNATURE_FILE: &str = "manifest.json.sig";

#[derive(Debug, Clone, PartialEq)]
pub enum Source {
    GitHub {
        owner: String,
        repo: String,
    },
    /// URL ของโฟลเดอร์ (ลงท้าย `/`) ที่มีไฟล์ครบชุด
    Https(String),
    Dir(PathBuf),
}

pub struct Fetched {
    pub manifest: Vec<u8>,
    pub signature: String,
}

fn unreachable(stage: &str, message: impl Into<String>) -> UpdateError {
    UpdateError::new(stage, "source_unreachable", message)
}

impl Source {
    /// `github:OWNER/REPO` | `https://host/path/` | `dir:PATH`
    pub fn parse(text: &str) -> Result<Source, String> {
        let text = text.trim();
        if let Some(rest) = text.strip_prefix("github:") {
            let (owner, repo) = rest.split_once('/').ok_or("ต้องเป็น github:OWNER/REPO")?;
            let ok = |s: &str, max: usize, extra: &[char]| {
                !s.is_empty()
                    && s.len() <= max
                    && !s.starts_with('.')
                    && s.chars().all(|c| c.is_ascii_alphanumeric() || extra.contains(&c))
            };
            if !ok(owner, 39, &['-']) || !ok(repo, 100, &['-', '_', '.']) {
                return Err("ชื่อ owner/repo ของ GitHub ไม่ถูกต้อง".into());
            }
            Ok(Source::GitHub {
                owner: owner.into(),
                repo: repo.into(),
            })
        } else if text.starts_with("https://") && text.len() > "https://".len() {
            // ASCII ที่พิมพ์ได้เท่านั้น ไม่มีช่องว่าง/อัญประกาศ/อักขระที่ shell หรือ URL ตีความ — ค่านี้ไปถึง settings.json และ URL ที่ SYSTEM ไม่แตะ
            // แต่ถูกพิมพ์กลับใน panel/คำสั่ง จึงไม่รับอะไรที่ไม่จำเป็น
            if !text
                .bytes()
                .all(|b| b.is_ascii_graphic() && !b"\"'<>\\^`{|}".contains(&b))
            {
                return Err("URL ต้องเป็น ASCII ไม่มีช่องว่างหรืออักขระพิเศษ".into());
            }
            Ok(Source::Https(if text.ends_with('/') {
                text.into()
            } else {
                format!("{text}/")
            }))
        } else if let Some(path) = text.strip_prefix("dir:") {
            Ok(Source::Dir(path.into()))
        } else {
            Err("รูปแบบที่รับ: github:OWNER/REPO, https://…/ (ต้อง https) หรือ dir:PATH".into())
        }
    }

    pub fn display(&self) -> String {
        match self {
            Source::GitHub { owner, repo } => format!("github:{owner}/{repo}"),
            Source::Https(url) => url.clone(),
            Source::Dir(path) => format!("dir:{}", path.display()),
        }
    }

    pub async fn fetch_manifest(&self) -> Result<Fetched, UpdateError> {
        let manifest = self.fetch_small(MANIFEST_FILE).await?;
        let signature = self.fetch_small(SIGNATURE_FILE).await?;
        let signature = String::from_utf8(signature).map_err(|_| unreachable("check", "ไฟล์ลายเซ็นไม่ใช่ข้อความ"))?;
        Ok(Fetched { manifest, signature })
    }

    /// ดาวน์โหลดตัวติดตั้งลง `dest` (ไม่เกิน `max_bytes`) — ไม่เชื่อ Content-Length; นับไบต์จริงที่รับ
    pub async fn download(&self, name: &str, dest: &Path, max_bytes: u64) -> Result<(), UpdateError> {
        let fail = |m: String| UpdateError::new("download", "download_failed", m);
        match self {
            Source::Dir(dir) => {
                let from = dir.join(name);
                let len = std::fs::metadata(&from)
                    .map_err(|e| fail(format!("ไม่พบ {name}: {e}")))?
                    .len();
                if len > max_bytes {
                    return Err(fail("ไฟล์ใหญ่เกินกำหนด".into()));
                }
                std::fs::copy(&from, dest).map_err(|e| fail(format!("คัดลอกไม่ได้: {e}")))?;
                Ok(())
            }
            _ => {
                let url = self.url_of(name).await?;
                let mut res = http_client(Duration::from_secs(900))?
                    .get(url)
                    .send()
                    .await
                    .and_then(|r| r.error_for_status())
                    .map_err(|e| fail(format!("ดาวน์โหลดไม่สำเร็จ: {e}")))?;
                let mut file = tokio::fs::File::create(dest)
                    .await
                    .map_err(|e| fail(format!("สร้างไฟล์ชั่วคราวไม่ได้: {e}")))?;
                let mut total = 0u64;
                while let Some(chunk) = res
                    .chunk()
                    .await
                    .map_err(|e| fail(format!("สายหลุดระหว่างดาวน์โหลด: {e}")))?
                {
                    total += chunk.len() as u64;
                    if total > max_bytes {
                        return Err(fail("ไฟล์ใหญ่เกินกำหนด".into()));
                    }
                    file.write_all(&chunk)
                        .await
                        .map_err(|e| fail(format!("เขียนไฟล์ไม่ได้: {e}")))?;
                }
                file.flush().await.map_err(|e| fail(format!("เขียนไฟล์ไม่ได้: {e}")))
            }
        }
    }

    async fn fetch_small(&self, name: &str) -> Result<Vec<u8>, UpdateError> {
        if let Source::Dir(dir) = self {
            let path = dir.join(name);
            let bytes =
                std::fs::read(&path).map_err(|e| unreachable("check", format!("อ่าน {} ไม่ได้: {e}", path.display())))?;
            return if bytes.len() > MAX_MANIFEST_BYTES {
                Err(UpdateError::new("check", "manifest_invalid", "ไฟล์ใหญ่ผิดปกติ"))
            } else {
                Ok(bytes)
            };
        }
        let url = self.url_of(name).await?;
        let mut res = http_client(Duration::from_secs(30))?
            .get(url)
            .send()
            .await
            .and_then(|r| r.error_for_status())
            .map_err(|e| unreachable("check", format!("ต่อแหล่ง release ไม่ได้: {e}")))?;
        let mut body = Vec::new();
        while let Some(chunk) = res
            .chunk()
            .await
            .map_err(|e| unreachable("check", format!("อ่านข้อมูลไม่ครบ: {e}")))?
        {
            body.extend_from_slice(&chunk);
            if body.len() > MAX_MANIFEST_BYTES {
                return Err(UpdateError::new("check", "manifest_invalid", "ไฟล์ใหญ่ผิดปกติ"));
            }
        }
        Ok(body)
    }

    /// URL ของไฟล์ในแหล่งที่เป็นเครือข่าย
    async fn url_of(&self, name: &str) -> Result<String, UpdateError> {
        match self {
            Source::Https(base) => Ok(format!("{base}{name}")),
            Source::GitHub { owner, repo } => {
                let api = format!("https://api.github.com/repos/{owner}/{repo}/releases/latest");
                let release: Value = http_client(Duration::from_secs(30))?
                    .get(api)
                    .header("Accept", "application/vnd.github+json")
                    .send()
                    .await
                    .and_then(|r| r.error_for_status())
                    .map_err(|e| unreachable("check", format!("ไม่พบ release ล่าสุดของ {owner}/{repo}: {e}")))?
                    .json()
                    .await
                    .map_err(|e| unreachable("check", format!("คำตอบจาก GitHub อ่านไม่ได้: {e}")))?;
                github_assets(&release, owner, repo)
                    .remove(name)
                    .ok_or_else(|| unreachable("check", format!("release ล่าสุดไม่มีไฟล์ {name}")))
            }
            Source::Dir(_) => unreachable!("dir ไม่ใช้ URL"),
        }
    }
}

/// ชื่อไฟล์ → URL จากคำตอบ `releases/latest` — รับเฉพาะ URL ที่อยู่ใต้ repo นั้นจริง
/// (กัน release ที่ชี้ asset ไปโฮสต์อื่น) และไม่รับ draft/prerelease
fn github_assets(release: &Value, owner: &str, repo: &str) -> HashMap<String, String> {
    let prefix = format!("https://github.com/{owner}/{repo}/releases/download/");
    if release["draft"].as_bool() == Some(true) || release["prerelease"].as_bool() == Some(true) {
        return HashMap::new();
    }
    release["assets"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|a| {
            Some((
                a["name"].as_str()?.to_string(),
                a["browser_download_url"].as_str()?.to_string(),
            ))
        })
        .filter(|(_, url)| url.starts_with(&prefix))
        .collect()
}

fn http_client(timeout: Duration) -> Result<reqwest::Client, UpdateError> {
    // reqwest (rustls แบบไม่มี provider) ต้องมี provider เริ่มต้นหนึ่งตัวต่อโปรเซส — ติดตั้งซ้ำไม่เป็นไร
    let _ = rustls::crypto::ring::default_provider().install_default();
    reqwest::Client::builder()
        .https_only(true)
        .user_agent("SynaptaConnect-Updater")
        .connect_timeout(Duration::from_secs(10))
        .timeout(timeout)
        .build()
        .map_err(|e| unreachable("check", format!("สร้าง HTTP client ไม่ได้: {e}")))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn parses_sources_and_rejects_bad_ones() {
        assert_eq!(
            Source::parse("github:me/Synapta-Connect"),
            Ok(Source::GitHub {
                owner: "me".into(),
                repo: "Synapta-Connect".into()
            })
        );
        assert_eq!(
            Source::parse("https://example.com/r"),
            Ok(Source::Https("https://example.com/r/".into()))
        );
        assert_eq!(Source::parse("dir:C:\\x"), Ok(Source::Dir("C:\\x".into())));
        for bad in [
            "",
            "http://example.com/",
            "github:me",
            "github:me/",
            "github:/repo",
            "github:../x/y",
            "github:me/a/b",
            "github:me/repo name",
            "ftp://x",
            "https://",
            "https://x/a b/",
            "https://x/\"y",
            "https://x/ไทย/",
            "https://x/a;b|c`d/",
        ] {
            assert!(Source::parse(bad).is_err(), "{bad}");
        }
        assert_eq!(Source::parse("github:me/repo").unwrap().display(), "github:me/repo");
    }

    #[test]
    fn github_assets_only_accept_urls_under_the_repo_and_stable_releases() {
        let release = json!({"draft": false, "prerelease": false, "assets": [
            {"name": "manifest.json", "browser_download_url": "https://github.com/me/repo/releases/download/v1/manifest.json"},
            {"name": "evil.exe", "browser_download_url": "https://evil.example/evil.exe"},
            {"name": "other.exe", "browser_download_url": "https://github.com/other/repo/releases/download/v1/other.exe"},
        ]});
        let assets = github_assets(&release, "me", "repo");
        assert_eq!(assets.len(), 1);
        assert!(assets.contains_key("manifest.json"));

        for flag in ["draft", "prerelease"] {
            let mut r = release.clone();
            r[flag] = json!(true);
            assert!(github_assets(&r, "me", "repo").is_empty(), "{flag}");
        }
    }

    #[tokio::test]
    async fn dir_source_fetches_manifest_and_asset_and_reports_missing_files() {
        let dir = std::env::temp_dir().join(format!("synapta-src-{}", crate::hex::random_id(4)));
        std::fs::create_dir_all(&dir).unwrap();
        let src = Source::Dir(dir.clone());
        assert_eq!(src.fetch_manifest().await.err().unwrap().code, "source_unreachable");

        std::fs::write(dir.join(MANIFEST_FILE), b"{}").unwrap();
        std::fs::write(dir.join(SIGNATURE_FILE), b"abcd\n").unwrap();
        std::fs::write(dir.join("pkg.exe"), vec![7u8; 100]).unwrap();
        let fetched = src.fetch_manifest().await.unwrap();
        assert_eq!((fetched.manifest, fetched.signature.trim()), (b"{}".to_vec(), "abcd"));

        let out = dir.join("out.exe");
        src.download("pkg.exe", &out, 100).await.unwrap();
        assert_eq!(std::fs::read(&out).unwrap().len(), 100);
        assert_eq!(
            src.download("pkg.exe", &out, 99).await.unwrap_err().code,
            "download_failed"
        );
        assert_eq!(
            src.download("nope.exe", &out, 100).await.unwrap_err().code,
            "download_failed"
        );
        std::fs::remove_dir_all(dir).unwrap();
    }
}
