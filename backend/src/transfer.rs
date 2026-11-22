//! ส่งไฟล์เป็นก้อน (chunk) พร้อมตรวจ SHA-256 และต่อจากจุดที่ค้างได้
//!
//! อัปโหลด: transfer.begin → PUT /api/v1/transfers/{id}?offset=N (ซ้ำจนครบ) → transfer.finish
//! ไฟล์ที่ยังไม่ครบอยู่ที่ `<path>.synapta-part`; ความคืบหน้า = ขนาดไฟล์นั้น จึงต่อได้แม้ agent restart
//! (client เรียก transfer.begin ด้วยพารามิเตอร์เดิมแล้วได้ `offset` ที่ต้องส่งต่อ)
//! ดาวน์โหลด: transfer.stat ขอ size+sha256 แล้ว GET /api/v1/files?path=... (รองรับ Range)

use crate::file_access;
use crate::hex::{random_id, to_hex};
use crate::rpc::{CONFLICT, Res, RpcError, parse};
use serde::Deserialize;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    collections::HashMap,
    io::Read,
    path::{Path, PathBuf},
    sync::Mutex,
    time::{Duration, Instant},
};

/// ขนาด chunk ที่แนะนำ (client ใช้ 64 KiB – 1 MiB ได้ ตัวเลขนี้คือค่ากลางที่ดีบน LAN/VPN)
pub const CHUNK_SIZE: u64 = 512 * 1024;
pub const MAX_CHUNK: usize = 1024 * 1024;
/// งานอัปโหลดที่ไม่มี chunk เข้ามานานเท่านี้ถือว่าทิ้งแล้ว — งานใหม่ที่ operation_id ต่างกันเขียนทับที่เดิมได้
const STALE_UPLOAD: Duration = Duration::from_secs(120);

struct Upload {
    path: PathBuf,
    size: u64,
    sha256: String,
    /// รหัสงานจาก client (SynaptaCode ส่งมาตั้งแต่ v1.0.101) — None = client รุ่นเก่า
    operation_id: Option<String>,
    touched: Instant,
}

pub struct Transfers {
    /// โฟลเดอร์ที่อนุญาต (canonical; ว่าง = ไม่จำกัด) ดู file_access.rs
    roots: Vec<PathBuf>,
    uploads: Mutex<HashMap<String, Upload>>,
}

#[derive(Deserialize)]
struct BeginParams {
    path: String,
    size: u64,
    sha256: String,
    #[serde(default)]
    overwrite: bool,
    operation_id: Option<String>,
}

#[derive(Deserialize)]
struct IdParams {
    transfer_id: String,
}

#[derive(Deserialize)]
struct StatParams {
    path: String,
}

fn part_path(path: &Path) -> PathBuf {
    let mut name = path.as_os_str().to_owned();
    name.push(".synapta-part");
    name.into()
}

fn part_len(path: &Path) -> u64 {
    std::fs::metadata(part_path(path)).map(|m| m.len()).unwrap_or(0)
}

/// SHA-256 ของไฟล์ (อ่านเป็นสตรีม ไม่โหลดทั้งไฟล์เข้า RAM)
fn hash_file(path: &Path) -> std::io::Result<String> {
    let mut file = std::fs::File::open(path)?;
    let (mut hasher, mut buf) = (Sha256::new(), vec![0u8; 256 * 1024]);
    loop {
        let n = file.read(&mut buf)?;
        if n == 0 {
            return Ok(to_hex(&hasher.finalize()));
        }
        hasher.update(&buf[..n]);
    }
}

/// เทียบปลายทางเดียวกันไหม — Windows ไม่สนตัวพิมพ์
fn same_target(a: &Path, b: &Path) -> bool {
    if cfg!(windows) {
        a.to_string_lossy().to_lowercase() == b.to_string_lossy().to_lowercase()
    } else {
        a == b
    }
}

fn unknown(id: &str) -> RpcError {
    RpcError::new(
        CONFLICT,
        format!("ไม่รู้จัก transfer '{id}' — เรียก transfer.begin ใหม่ (agent อาจ restart)"),
    )
}

impl Transfers {
    pub fn new(roots: Vec<PathBuf>) -> Transfers {
        Transfers {
            roots,
            uploads: Mutex::new(HashMap::new()),
        }
    }

    pub async fn rpc(&self, method: &str, params: Value) -> Res<Value> {
        match method {
            "transfer.begin" => self.begin(parse(params)?),
            "transfer.finish" => self.finish(parse::<IdParams>(params)?.transfer_id).await,
            "transfer.abort" => self.abort(&parse::<IdParams>(params)?.transfer_id),
            "transfer.stat" => stat(file_access::for_read(&parse::<StatParams>(params)?.path, &self.roots)?).await,
            _ => unreachable!("dispatch ส่งมาเฉพาะ transfer.*"),
        }
    }

    fn begin(&self, p: BeginParams) -> Res<Value> {
        let path = file_access::for_write(&p.path, &self.roots)?;
        if p.sha256.len() != 64 || !p.sha256.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Err(RpcError::invalid("sha256 ต้องเป็น hex 64 ตัวอักษร"));
        }
        if p.operation_id.as_ref().is_some_and(|op| op.is_empty() || op.len() > 64) {
            return Err(RpcError::invalid("operation_id ต้องยาว 1-64 ตัวอักษร"));
        }
        // ถือ lock ตั้งแต่ตรวจจนบันทึกงาน — สอง begin ที่มาพร้อมกันต้องเห็นกันเสมอ
        let mut uploads = self.uploads.lock().unwrap();
        claim_target(&mut uploads, &path, p.operation_id.as_deref())?;
        if path.is_dir() || (path.exists() && !p.overwrite) {
            return Err(RpcError::new(CONFLICT, "ปลายทางมีอยู่แล้ว (ส่ง overwrite:true ถ้าต้องการทับ)"));
        }
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        // ไฟล์ค้างที่ใหญ่กว่าไฟล์จริง = ของคนละงาน เริ่มใหม่
        let mut offset = part_len(&path);
        if offset > p.size {
            std::fs::remove_file(part_path(&path))?;
            offset = 0;
        }
        std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(part_path(&path))?;

        let id = random_id(8);
        uploads.insert(
            id.clone(),
            Upload {
                path,
                size: p.size,
                sha256: p.sha256.to_lowercase(),
                operation_id: p.operation_id,
                touched: Instant::now(),
            },
        );
        Ok(json!({"transfer_id": id, "offset": offset, "chunk_size": CHUNK_SIZE,
                  "max_chunk_size": MAX_CHUNK, "resumed": offset > 0}))
    }

    /// เขียน chunk ต่อท้ายไฟล์ — offset ต้องตรงกับที่ได้รับไปแล้วพอดี (ส่งซ้ำ/ข้ามไม่ได้)
    pub async fn put_chunk(&self, id: &str, offset: u64, data: Vec<u8>) -> Res<Value> {
        let path = {
            let mut map = self.uploads.lock().unwrap();
            let up = map.get_mut(id).ok_or_else(|| unknown(id))?;
            up.touched = Instant::now();
            if offset + data.len() as u64 > up.size {
                return Err(RpcError::invalid("chunk เกินขนาดไฟล์ที่ประกาศไว้"));
            }
            up.path.clone()
        };
        tokio::task::spawn_blocking(move || -> Res<Value> {
            let have = part_len(&path);
            if have != offset {
                return Err(RpcError::new(CONFLICT, format!("offset ไม่ตรง: ต้องส่งต่อที่ {have}")));
            }
            let mut f = std::fs::OpenOptions::new().append(true).open(part_path(&path))?;
            std::io::Write::write_all(&mut f, &data)?;
            Ok(json!({"offset": have + data.len() as u64}))
        })
        .await?
    }

    async fn finish(&self, id: String) -> Res<Value> {
        let (path, size, expected) = {
            let map = self.uploads.lock().unwrap();
            let up = map.get(&id).ok_or_else(|| unknown(&id))?;
            (up.path.clone(), up.size, up.sha256.clone())
        };
        // (transfer นี้จบแล้วหรือยัง, ผลลัพธ์) — ยังไม่ครบ = ให้ส่งต่อด้วย id เดิมได้
        let (finished, result) = tokio::task::spawn_blocking(move || -> (bool, Res<Value>) {
            let part = part_path(&path);
            let have = part_len(&path);
            if have != size {
                return (
                    false,
                    Err(RpcError::new(CONFLICT, format!("ยังไม่ครบ: ได้ {have} จาก {size} ไบต์"))),
                );
            }
            let actual = match hash_file(&part) {
                Ok(h) => h,
                Err(e) => return (false, Err(e.into())),
            };
            if actual != expected {
                // ไฟล์ที่ checksum ผิดห้ามอยู่ให้ resume ต่อ ไม่งั้นจะผิดซ้ำทุกครั้ง
                let _ = std::fs::remove_file(&part);
                let msg = format!("checksum ไม่ตรง (ได้ {actual}) — ลบไฟล์ค้างแล้ว ให้ transfer.begin ใหม่แล้วส่งทั้งไฟล์");
                return (true, Err(RpcError::new(CONFLICT, msg)));
            }
            match std::fs::rename(&part, &path) {
                Ok(()) => (true, Ok(json!({"path": path, "size": size, "sha256": actual}))),
                Err(e) => (false, Err(e.into())),
            }
        })
        .await?;
        if finished {
            self.uploads.lock().unwrap().remove(&id);
        }
        result
    }

    fn abort(&self, id: &str) -> Res<Value> {
        let up = self.uploads.lock().unwrap().remove(id).ok_or_else(|| unknown(id))?;
        let _ = std::fs::remove_file(part_path(&up.path));
        Ok(json!({"aborted": true}))
    }
}

/// กันสองงานเขียน `<path>.synapta-part` ไฟล์เดียวกันพร้อมกัน (ไฟล์จะปนกันจน checksum ผิดทั้งคู่)
/// - operation_id เดียวกัน = client ลองใหม่หลังสัญญาณหลุด: ทิ้ง transfer_id เก่า ใช้ไฟล์ค้างต่อ
/// - operation_id ต่างกันและงานเดิมยังเดินอยู่ = ปฏิเสธ; งานเดิมเงียบเกิน STALE_UPLOAD = ยึดต่อได้
/// - ฝั่งใดไม่มี operation_id (client รุ่นเก่า) = ทำแบบเดิม ไม่ตรวจ เพื่อไม่ทำให้การ resume ของรุ่นเก่าพัง
fn claim_target(uploads: &mut HashMap<String, Upload>, path: &Path, operation: Option<&str>) -> Res<()> {
    let Some(operation) = operation else { return Ok(()) };
    let mut replaced = Vec::new();
    for (id, up) in uploads.iter() {
        let Some(other) = up.operation_id.as_deref() else {
            continue;
        };
        if !same_target(&up.path, path) {
            continue;
        }
        if other == operation || up.touched.elapsed() >= STALE_UPLOAD {
            replaced.push(id.clone());
        } else {
            return Err(RpcError::new(CONFLICT, "มีอีกงานกำลังส่งไฟล์ไปที่ปลายทางนี้ — รอให้เสร็จก่อน"));
        }
    }
    for id in replaced {
        uploads.remove(&id);
    }
    Ok(())
}

/// `path` ผ่าน file_access::for_read มาแล้ว (canonical, เป็นไฟล์, อยู่ใน root)
async fn stat(path: PathBuf) -> Res<Value> {
    tokio::task::spawn_blocking(move || -> Res<Value> {
        let meta = std::fs::metadata(&path)?;
        let modified = meta
            .modified()
            .ok()
            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|d| d.as_secs());
        Ok(json!({"size": meta.len(), "sha256": hash_file(&path)?, "modified": modified}))
    })
    .await?
}
