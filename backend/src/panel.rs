//! panel (หน้าจัดการบนเครื่อง): ไฟล์หน้าเว็บที่ฝังในโปรแกรม + การยืนยันตัวของเบราว์เซอร์
//!
//! เบราว์เซอร์อ่าน local_token ไม่ได้ (ไฟล์จำกัดสิทธิ์) — คำสั่ง `synapta-connect panel` (รันโดยเจ้าของ อ่านไฟล์ได้) ขอ "รหัสครั้งเดียว"
//! แล้วเปิดเบราว์เซอร์ด้วยรหัสนั้น หน้าเว็บแลกเป็น token อายุ 8 ชม. — token ถาวรไม่เคยเข้าเบราว์เซอร์/ประวัติ/URL
//! สัญญากับหน้าบ้านอยู่ที่ docs/PANEL_API.md §1

use crate::{auth::now_secs, hex::random_id};
use axum::{
    extract::{Path, State},
    http::{StatusCode, header},
    response::{IntoResponse, Response},
};
use rust_embed::RustEmbed;
use sha2::Digest;
use std::{collections::HashMap, sync::Mutex};

const CODE_TTL_SECS: u64 = 60;
const TOKEN_TTL_SECS: u64 = 8 * 3600;
/// กันกองไม่จำกัดถ้ามีใครขอรหัสรัว ๆ — เกินแล้วตัวเก่าสุดหลุดก่อน
const MAX_ENTRIES: usize = 16;

/// หน้าเว็บของ panel = โฟลเดอร์ frontend/ ตอน build (entry: index.html) ยกเว้นไฟล์ที่ไม่ใช่ของหน้าเว็บ
#[derive(RustEmbed)]
#[folder = "../frontend/dist/"]
struct Assets;

pub const PATH: &str = "/synaptaconnect/";

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Scope {
    Local,
    Lan(String),
}

/// token เก็บเป็น SHA-256 (ไม่เก็บตัวจริง) และรอดการรีสตาร์ทของ agent จนถึงเวลาหมดอายุ — ตอนอัปเดต service ต้องรีสตาร์ท
/// ถ้า token หายตามไปผู้ใช้จะไม่เห็นผลอัปเดต (สำเร็จ/กู้คืน) เลย; รหัสครั้งเดียว (60 วิ) ไม่เก็บลงดิสก์
#[derive(Default)]
pub struct PanelSessions {
    codes: Mutex<HashMap<String, (u64, Scope)>>,
    /// hash ของ token → เวลาหมดอายุ
    tokens: Mutex<HashMap<String, u64>>,
    /// ที่เก็บ hash (None = ไม่เก็บ ใช้ในเทสต์)
    file: Option<std::path::PathBuf>,
}

fn hash(token: &str) -> String {
    crate::hex::to_hex(&sha2::Sha256::digest(token.as_bytes()))
}

fn token_hash(token: &str, scope: &Scope) -> String {
    match scope {
        Scope::Local => hash(token),
        Scope::Lan(host) => hash(&format!("lan:{host}\0{token}")),
    }
}

/// ลบที่หมดอายุ แล้วถ้ายังล้นให้ตัดตัวที่ใกล้หมดอายุที่สุด
fn prune(map: &mut HashMap<String, u64>) {
    let now = now_secs();
    map.retain(|_, exp| *exp > now);
    while map.len() >= MAX_ENTRIES {
        let oldest = map.iter().min_by_key(|(_, exp)| **exp).map(|(k, _)| k.clone());
        match oldest {
            Some(k) => map.remove(&k),
            None => break,
        };
    }
}

impl PanelSessions {
    /// โหลด token ที่ยังไม่หมดอายุจาก data_dir/panel_sessions.json
    pub fn load(data_dir: &std::path::Path) -> PanelSessions {
        let file = data_dir.join("panel_sessions.json");
        let mut tokens: HashMap<String, u64> = crate::fsutil::read_json(&file).unwrap_or_default();
        let now = now_secs();
        tokens.retain(|_, exp| *exp > now);
        PanelSessions {
            tokens: Mutex::new(tokens),
            file: Some(file),
            ..PanelSessions::default()
        }
    }

    fn save(&self, tokens: &HashMap<String, u64>) {
        if let Some(file) = &self.file
            && let Err(e) = crate::fsutil::write_json(file, tokens)
        {
            tracing::error!("บันทึก panel_sessions.json ไม่สำเร็จ: {e}");
        }
    }

    /// รหัสครั้งเดียว อายุ 60 วินาที
    pub fn issue_code(&self) -> String {
        self.issue_for(Scope::Local)
    }

    pub fn issue_for(&self, scope: Scope) -> String {
        let mut codes = self.codes.lock().unwrap();
        codes.retain(|_, (expires, _)| *expires > now_secs());
        if codes.len() >= MAX_ENTRIES
            && let Some(oldest) = codes
                .iter()
                .min_by_key(|(_, (exp, _))| *exp)
                .map(|(key, _)| key.clone())
        {
            codes.remove(&oldest);
        }
        let code = random_id(16);
        codes.insert(code.clone(), (now_secs() + CODE_TTL_SECS, scope));
        code
    }

    /// แลกรหัส → (token, หมดอายุเมื่อ) — รหัสถูกใช้แล้วหมดทันทีไม่ว่าสำเร็จหรือไม่
    pub fn exchange(&self, code: &str) -> Option<(String, u64)> {
        self.exchange_for(code, &Scope::Local)
    }

    pub fn exchange_for(&self, code: &str, scope: &Scope) -> Option<(String, u64)> {
        let mut codes = self.codes.lock().unwrap();
        let (expires, issued_for) = codes.get(code)?;
        if expires <= &now_secs() || issued_for != scope {
            return None;
        }
        codes.remove(code);
        let mut tokens = self.tokens.lock().unwrap();
        prune(&mut tokens);
        let (token, until) = (random_id(32), now_secs() + TOKEN_TTL_SECS);
        tokens.insert(token_hash(&token, scope), until);
        self.save(&tokens);
        Some((token, until))
    }

    pub fn is_valid(&self, token: &str) -> bool {
        self.valid_for(token, &Scope::Local)
    }

    pub fn valid_for(&self, token: &str, scope: &Scope) -> bool {
        self.tokens
            .lock()
            .unwrap()
            .get(&token_hash(token, scope))
            .is_some_and(|exp| *exp > now_secs())
    }
}

pub async fn index(State(app): State<crate::app::App>) -> Response {
    serve_file(&app, "index.html")
}

pub async fn file(State(app): State<crate::app::App>, Path(path): Path<String>) -> Response {
    serve_file(&app, &path)
}

fn serve_file(app: &crate::app::App, path: &str) -> Response {
    let file = app
        .web_updates
        .file(path)
        .map(|(mime, bytes)| (mime.to_owned(), bytes))
        .or_else(|| Assets::get(path).map(|asset| (asset.metadata.mimetype().to_string(), asset.data.into_owned())));
    let Some((mime, bytes)) = file else {
        return StatusCode::NOT_FOUND.into_response();
    };
    // หน้าเว็บคุยได้เฉพาะ origin ตัวเอง และห้ามฝังใน frame — กัน XSS/clickjacking บนหน้าที่ถือ token
    (
        [
            (header::CONTENT_TYPE, mime),
            (
                header::CONTENT_SECURITY_POLICY,
                "default-src 'none'; script-src 'self'; style-src 'self'; img-src 'self' data:; \
                 font-src 'self'; connect-src 'self'; base-uri 'none'; form-action 'none'; frame-ancestors 'none'"
                    .into(),
            ),
            (header::X_CONTENT_TYPE_OPTIONS, "nosniff".into()),
            (header::CACHE_CONTROL, "no-store".into()),
            (header::REFERRER_POLICY, "no-referrer".into()),
        ],
        bytes,
    )
        .into_response()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn code_is_single_use_and_token_validates() {
        let s = PanelSessions::default();
        let code = s.issue_code();
        let (token, _) = s.exchange(&code).expect("แลกครั้งแรกได้");
        assert!(s.is_valid(&token));
        assert!(s.exchange(&code).is_none(), "รหัสใช้ได้ครั้งเดียว");
        assert!(s.exchange("nonsense").is_none());
        assert!(!s.is_valid("nonsense"));
    }

    #[test]
    fn expired_code_and_token_are_rejected_and_sessions_are_capped() {
        let s = PanelSessions::default();
        let code = s.issue_code();
        s.codes
            .lock()
            .unwrap()
            .insert(code.clone(), (now_secs() - 1, Scope::Local));
        assert!(s.exchange(&code).is_none());

        let (token, _) = s.exchange(&s.issue_code()).unwrap();
        s.tokens.lock().unwrap().insert(hash(&token), now_secs() - 1);
        assert!(!s.is_valid(&token));

        for _ in 0..100 {
            s.exchange(&s.issue_code());
        }
        assert!(s.tokens.lock().unwrap().len() <= MAX_ENTRIES);
    }

    #[test]
    fn tokens_survive_restart_as_hashes_only_and_expired_ones_are_dropped() {
        let dir = std::env::temp_dir().join(format!("synapta-panel-{}", random_id(4)));
        std::fs::create_dir_all(&dir).unwrap();
        let first = PanelSessions::load(&dir);
        let (token, _) = first.exchange(&first.issue_code()).unwrap();
        let (stale, _) = first.exchange(&first.issue_code()).unwrap();
        first.tokens.lock().unwrap().insert(hash(&stale), now_secs() - 1);
        first.save(&first.tokens.lock().unwrap());

        let on_disk = std::fs::read_to_string(dir.join("panel_sessions.json")).unwrap();
        assert!(!on_disk.contains(&token), "ต้องไม่เก็บ token ตัวจริง");

        let restarted = PanelSessions::load(&dir); // เหมือน agent เริ่มใหม่หลังอัปเดต
        assert!(restarted.is_valid(&token));
        assert!(!restarted.is_valid(&stale));
        assert!(!restarted.is_valid("guess"));
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn embedded_panel_has_an_entry_page_and_excludes_non_web_files() {
        assert!(Assets::get("index.html").is_some());
        assert!(Assets::get("README.md").is_none());
        assert!(Assets::get("preview.png").is_none());
    }
}
