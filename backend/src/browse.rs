//! `file.open_path` — เปิดดูเครื่องปลายทาง: รายการไดรฟ์, รายการในโฟลเดอร์ (ทีละหน้า) หรือข้อมูล/พรีวิวของไฟล์
//!
//! สิทธิ์ใช้กฎเดียวกับ transfer.* (file_access.rs): ตั้ง `--file-root` ไว้ก็เห็นแค่ในนั้น
//! รูปผลลัพธ์ต้องตรงกับ `connectPathResult` ใน SynaptaCode (server/src/connect/schemas.ts) — ฟิลด์ที่ไม่มีค่าต้อง "ไม่ส่ง" ห้ามส่ง null

use crate::file_access::{self, display_path};
use crate::rpc::{Res, RpcError, to_value};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{
    io::Read,
    path::{Path, PathBuf},
};

/// จำนวนรายการต่อหน้าสูงสุด (SynaptaCode รับได้ไม่เกิน 100)
const MAX_PAGE: usize = 100;
/// อ่านต้นไฟล์มาทำพรีวิวไม่เกินเท่านี้ — ได้ไม่เกิน 16384 ตัวอักษร ต่ำกว่าเพดาน 32768 ของ SynaptaCode
const PREVIEW_BYTES: usize = 16 * 1024;

#[derive(Deserialize)]
pub struct OpenPathParams {
    /// ไม่ใส่หรือว่าง = รายการไดรฟ์ (หรือโฟลเดอร์ที่ --file-root อนุญาต)
    path: Option<String>,
    limit: Option<usize>,
    /// ค่าที่ได้จาก `next_cursor` ของหน้าก่อน
    cursor: Option<String>,
    #[serde(default)]
    preview: bool,
}

#[derive(Serialize, Debug)]
struct Entry {
    name: String,
    path: String,
    kind: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    size: Option<u64>,
}

#[derive(Serialize, Debug)]
#[serde(tag = "kind", rename_all = "lowercase")]
enum Opened {
    Roots {
        entries: Vec<Entry>,
    },
    Directory {
        path: String,
        parent: Option<String>,
        entries: Vec<Entry>,
        #[serde(skip_serializing_if = "Option::is_none")]
        next_cursor: Option<String>,
    },
    File {
        path: String,
        name: String,
        size: u64,
        #[serde(skip_serializing_if = "Option::is_none")]
        text_preview: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        preview_truncated: Option<bool>,
    },
}

pub async fn open_path(params: OpenPathParams, roots: &[PathBuf]) -> Res<Value> {
    let roots = roots.to_vec();
    let opened = tokio::task::spawn_blocking(move || open(params, &roots)).await??;
    to_value(opened)
}

fn open(p: OpenPathParams, roots: &[PathBuf]) -> Res<Opened> {
    let Some(path) = p.path.filter(|s| !s.is_empty()) else {
        return Ok(Opened::Roots {
            entries: top_level(roots),
        });
    };
    let canonical = file_access::for_browse(&path, roots)?;
    let meta = std::fs::metadata(&canonical)?;
    if meta.is_dir() {
        let limit = p.limit.unwrap_or(MAX_PAGE).clamp(1, MAX_PAGE);
        let offset = match p.cursor {
            Some(c) => c.parse::<usize>().map_err(|_| RpcError::invalid("cursor ไม่ถูกต้อง"))?,
            None => 0,
        };
        list_directory(&canonical, roots, offset, limit)
    } else if meta.is_file() {
        let (text_preview, preview_truncated) = match p.preview {
            true => preview(&canonical, meta.len())?,
            false => (None, None),
        };
        Ok(Opened::File {
            path: display_path(&canonical),
            name: canonical
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default(),
            size: meta.len(),
            text_preview,
            preview_truncated,
        })
    } else {
        Err(RpcError::invalid("ไม่ใช่ไฟล์หรือโฟลเดอร์"))
    }
}

/// จุดเริ่มเปิดดู: โฟลเดอร์ใน --file-root ถ้าตั้งไว้ ไม่งั้นทุกไดรฟ์ (Windows) หรือ `/`
fn top_level(roots: &[PathBuf]) -> Vec<Entry> {
    let dirs: Vec<PathBuf> = if !roots.is_empty() {
        roots.to_vec()
    } else if cfg!(windows) {
        (b'A'..=b'Z')
            .map(|d| PathBuf::from(format!("{}:\\", d as char)))
            .filter(|d| d.exists())
            .collect()
    } else {
        vec![PathBuf::from("/")]
    };
    dirs.iter()
        .map(|d| {
            let path = display_path(d);
            Entry {
                name: path.clone(),
                path,
                kind: "directory",
                size: None,
            }
        })
        .collect()
}

fn list_directory(dir: &Path, roots: &[PathBuf], offset: usize, limit: usize) -> Res<Opened> {
    let mut entries: Vec<Entry> = Vec::new();
    for item in std::fs::read_dir(dir)? {
        let Ok(item) = item else { continue };
        let full = item.path();
        // ตาม symlink/junction เพื่อบอกชนิดปลายทาง; stat ไม่ได้ (ไฟล์ระบบที่ถูกล็อก) ใช้ชนิดจาก read_dir แทน
        let (is_dir, size) = match std::fs::metadata(&full) {
            Ok(m) => (m.is_dir(), m.is_file().then_some(m.len())),
            Err(_) => match item.file_type() {
                Ok(t) => (t.is_dir(), None),
                Err(_) => continue,
            },
        };
        entries.push(Entry {
            name: item.file_name().to_string_lossy().into_owned(),
            path: display_path(&full),
            kind: if is_dir { "directory" } else { "file" },
            size: if is_dir { None } else { size },
        });
    }
    // โฟลเดอร์ก่อนไฟล์ แล้วเรียงชื่อไม่สนตัวพิมพ์ — ลำดับต้องคงที่ทุกหน้า cursor ถึงจะใช้ offset ได้
    entries.sort_by_cached_key(|e| (e.kind != "directory", e.name.to_lowercase(), e.name.clone()));
    let total = entries.len();
    let page: Vec<Entry> = entries.into_iter().skip(offset).take(limit).collect();
    let next_cursor = (offset + limit < total).then(|| (offset + limit).to_string());

    // ที่ root ที่อนุญาต (หรือรากไดรฟ์) ไม่มี "ขึ้นไป" — ไม่งั้น client จะเดินออกนอก root แล้วโดนปฏิเสธ
    let at_root = roots.iter().any(|r| r == dir);
    let parent = dir
        .parent()
        .filter(|_| !at_root)
        .filter(|p| roots.is_empty() || roots.iter().any(|r| p.starts_with(r)))
        .map(display_path);
    Ok(Opened::Directory {
        path: display_path(dir),
        parent,
        entries: page,
        next_cursor,
    })
}

/// คืน (พรีวิว, ถูกตัดไหม) — ไฟล์ที่ไม่ใช่ข้อความคืน (None, None)
fn preview(path: &Path, size: u64) -> Res<(Option<String>, Option<bool>)> {
    let mut head = Vec::with_capacity(PREVIEW_BYTES);
    std::fs::File::open(path)?
        .take(PREVIEW_BYTES as u64)
        .read_to_end(&mut head)?;
    Ok(match decode_text(&head) {
        Some(text) => (Some(text), Some(size > head.len() as u64)),
        None => (None, None),
    })
}

/// ถอดต้นไฟล์เป็นข้อความ: UTF-8 (มี/ไม่มี BOM) หรือ UTF-16 ที่มี BOM (ไฟล์ที่ PowerShell 5.1 เขียน)
/// มี NUL หรือถอดไม่ได้ = ไฟล์ binary
fn decode_text(bytes: &[u8]) -> Option<String> {
    let utf16 = |be: bool| {
        let units: Vec<u16> = bytes[2..]
            .as_chunks::<2>()
            .0
            .iter()
            .map(|&pair| {
                if be {
                    u16::from_be_bytes(pair)
                } else {
                    u16::from_le_bytes(pair)
                }
            })
            .collect();
        String::from_utf16_lossy(&units)
    };
    let text = match bytes {
        [0xFF, 0xFE, ..] => utf16(false),
        [0xFE, 0xFF, ..] => utf16(true),
        _ => {
            let body = bytes.strip_prefix(&[0xEF, 0xBB, 0xBF]).unwrap_or(bytes);
            match std::str::from_utf8(body) {
                Ok(s) => s.to_string(),
                // ตัวอักษรสุดท้ายโดนตัดกลางตัวตอนอ่าน 16 KiB — ตัดทิ้งเฉพาะตัวนั้น
                Err(e) if e.error_len().is_none() => std::str::from_utf8(&body[..e.valid_up_to()]).ok()?.to_string(),
                Err(_) => return None,
            }
        }
    };
    (!text.contains('\0')).then_some(text)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn params(path: Option<&Path>, cursor: Option<&str>, limit: usize, preview: bool) -> OpenPathParams {
        OpenPathParams {
            path: path.map(|p| p.to_string_lossy().into_owned()),
            limit: Some(limit),
            cursor: cursor.map(str::to_string),
            preview,
        }
    }

    fn sandbox() -> PathBuf {
        let base = std::env::temp_dir().join(format!("synapta-browse-{}", crate::hex::random_id(4)));
        std::fs::create_dir_all(base.join("Zeta")).unwrap();
        std::fs::create_dir_all(base.join("alpha")).unwrap();
        std::fs::write(base.join("b.txt"), "สวัสดี hello").unwrap();
        std::fs::write(base.join("A.bin"), [0u8, 1, 2, 3]).unwrap();
        base.canonicalize().unwrap()
    }

    #[test]
    fn lists_directories_first_and_pages_with_cursor() {
        let base = sandbox();
        let Opened::Directory {
            entries,
            next_cursor,
            path,
            ..
        } = open(params(Some(&base), None, 3, false), &[]).unwrap()
        else {
            panic!("ต้องเป็นโฟลเดอร์")
        };
        let names: Vec<_> = entries.iter().map(|e| e.name.as_str()).collect();
        assert_eq!(names, ["alpha", "Zeta", "A.bin"]);
        assert_eq!(entries[2].size, Some(4));
        assert!(!path.starts_with(r"\\?\"), "path ที่ส่งกลับต้องไม่มี prefix verbatim: {path}");
        let cursor = next_cursor.expect("ยังเหลืออีกหนึ่งรายการ");
        let Opened::Directory {
            entries, next_cursor, ..
        } = open(params(Some(&base), Some(&cursor), 3, false), &[]).unwrap()
        else {
            panic!()
        };
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].name, "b.txt");
        assert!(next_cursor.is_none());
        std::fs::remove_dir_all(base).unwrap();
    }

    #[test]
    fn previews_text_but_not_binary() {
        let base = sandbox();
        let Opened::File {
            text_preview,
            preview_truncated,
            size,
            ..
        } = open(params(Some(&base.join("b.txt")), None, 100, true), &[]).unwrap()
        else {
            panic!()
        };
        assert_eq!(text_preview.as_deref(), Some("สวัสดี hello"));
        assert_eq!(preview_truncated, Some(false));
        assert_eq!(size, "สวัสดี hello".len() as u64);
        let Opened::File { text_preview, .. } = open(params(Some(&base.join("A.bin")), None, 100, true), &[]).unwrap()
        else {
            panic!()
        };
        assert!(text_preview.is_none());
        std::fs::remove_dir_all(base).unwrap();
    }

    #[test]
    fn decodes_utf16_and_cut_utf8() {
        let mut le = vec![0xFF, 0xFE];
        le.extend("ไทย".encode_utf16().flat_map(u16::to_le_bytes));
        assert_eq!(decode_text(&le).as_deref(), Some("ไทย"));
        let cut = &"กข".as_bytes()[..4]; // ตัดกลางตัว "ข"
        assert_eq!(decode_text(cut).as_deref(), Some("ก"));
    }

    #[test]
    fn file_root_limits_listing_and_hides_parent() {
        let base = sandbox();
        let root = base.join("alpha").canonicalize().unwrap();
        let roots = vec![root.clone()];
        let Opened::Roots { entries } = open(params(None, None, 100, false), &roots).unwrap() else {
            panic!()
        };
        assert_eq!(entries.len(), 1);
        let Opened::Directory { parent, .. } = open(params(Some(&root), None, 100, false), &roots).unwrap() else {
            panic!()
        };
        assert!(parent.is_none(), "root ที่อนุญาตต้องไม่มี parent");
        assert!(open(params(Some(&base), None, 100, false), &roots).is_err());
        std::fs::remove_dir_all(base).unwrap();
    }

    #[test]
    fn serializes_without_nulls() {
        let base = sandbox();
        let value = to_value(open(params(Some(&base), None, 100, false), &[]).unwrap()).unwrap();
        assert_eq!(value["kind"], "directory");
        assert!(value.get("next_cursor").is_none());
        assert!(value["entries"][0].get("size").is_none(), "โฟลเดอร์ไม่มี size");
        std::fs::remove_dir_all(base).unwrap();
    }
}
