//! ส่งไฟล์เป็นก้อน (chunk) พร้อมตรวจ SHA-256 และต่อจากจุดที่ค้างได้
//!
//! อัปโหลด: transfer.begin → PUT /api/v1/transfers/{id}?offset=N (ซ้ำจนครบ) → transfer.finish
//! ไฟล์ที่ยังไม่ครบอยู่ที่ `<path>.synapta-part`; ความคืบหน้า = ขนาดไฟล์นั้น จึงต่อได้แม้ agent restart
//! (client เรียก transfer.begin ด้วยพารามิเตอร์เดิมแล้วได้ `offset` ที่ต้องส่งต่อ)
//! ดาวน์โหลด: transfer.stat ขอ size+sha256 แล้ว GET /api/v1/files?path=... (รองรับ Range)
//!
//! โหมดขนาน (`parallel: true` ใน transfer.begin, ตั้งแต่ 0.3.18): client ส่งหลาย chunk พร้อมกันแบบ IDM
//! chunk เริ่มที่ขอบ `UNIT` ยาวเท่าไรก็ได้ภายในเพดานที่ transfer_plan.rs คำนวณจากเครื่องตอนนั้น
//! เขียนลงตำแหน่งของมันใน `.synapta-part` และจดว่าได้ช่วงไหนแล้วใน `<path>.synapta-part.map`
//! จึงต่อได้แม้ agent restart · ดาวน์โหลดแบบขนานใช้ Range หลายช่วงพร้อมกันได้อยู่แล้ว

use crate::hex::{random_id, to_hex};
use crate::rpc::{CONFLICT, Res, RpcError, parse};
use crate::transfer_plan::{self, UNIT};
use crate::{file_access, fsutil};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    collections::HashMap,
    fs::File,
    io::Read,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

/// ขนาด chunk ที่แนะนำ (client ใช้ 64 KiB – 1 MiB ได้ ตัวเลขนี้คือค่ากลางที่ดีบน LAN/VPN)
pub const CHUNK_SIZE: u64 = 512 * 1024;
pub const MAX_CHUNK: usize = 1024 * 1024;
/// body ใหญ่สุดที่ PUT /api/v1/transfers รับ (http.rs ตั้งตามนี้)
pub const MAX_BODY: usize = transfer_plan::MAX_CHUNK as usize;
/// งานอัปโหลดที่ไม่มี chunk เข้ามานานเท่านี้ถือว่าทิ้งแล้ว — งานใหม่ที่ operation_id ต่างกันเขียนทับที่เดิมได้
const STALE_UPLOAD: Duration = Duration::from_secs(120);

struct Upload {
    path: PathBuf,
    size: u64,
    sha256: String,
    /// รหัสงานจาก client (SynaptaCode ส่งมาตั้งแต่ v1.0.101) — None = client รุ่นเก่า
    operation_id: Option<String>,
    touched: Instant,
    /// Some = โหมดขนาน (chunk มาไม่เรียงได้) · None = โหมดเดิม ต่อท้ายทีละก้อน
    ranges: Option<Arc<Mutex<Ranges>>>,
}

/// ช่วงไหนของโหมดขนานได้แล้วบ้าง (ทีละ `UNIT`) — เก็บเป็นไฟล์ `.synapta-part.map` คู่กับไฟล์ค้าง
/// มี size/sha256 กำกับ เพื่อไม่เอาไฟล์ค้างของอีกไฟล์ที่ชื่อเดียวกันมาต่อ
#[derive(Serialize, Deserialize)]
struct Ranges {
    size: u64,
    sha256: String,
    unit: u64,
    /// bitmap เป็น hex: bit i = ได้ unit i แล้ว (ไฟล์ 100 GB = map ราว 25 KB)
    done: String,
    #[serde(skip)]
    bits: Vec<u8>,
}

impl Ranges {
    fn new(size: u64, sha256: &str) -> Ranges {
        let units = size.div_ceil(UNIT) as usize;
        Ranges {
            size,
            sha256: sha256.to_string(),
            unit: UNIT,
            done: String::new(),
            bits: vec![0; units.div_ceil(8)],
        }
    }

    fn save(&mut self, path: &Path) -> std::io::Result<()> {
        self.done = to_hex(&self.bits);
        fsutil::atomic_write(
            &map_path(path),
            &serde_json::to_vec(self).map_err(std::io::Error::other)?,
        )
    }

    fn load(path: &Path) -> Option<Ranges> {
        let mut saved = fsutil::read_json::<Ranges>(&map_path(path))?;
        saved.bits = (0..saved.done.len() / 2)
            .map(|i| u8::from_str_radix(saved.done.get(i * 2..i * 2 + 2)?, 16).ok())
            .collect::<Option<_>>()?;
        Some(saved)
    }

    fn chunks(&self) -> u64 {
        self.size.div_ceil(self.unit)
    }

    fn has(&self, i: u64) -> bool {
        self.bits[(i / 8) as usize] & (1 << (i % 8)) != 0
    }

    fn mark(&mut self, i: u64) {
        self.bits[(i / 8) as usize] |= 1 << (i % 8);
    }

    /// ขนาดของ unit ที่ i (unit สุดท้ายอาจสั้นกว่า)
    fn len_of(&self, i: u64) -> u64 {
        self.unit.min(self.size - i * self.unit)
    }

    fn received_bytes(&self) -> u64 {
        (0..self.chunks())
            .filter(|&i| self.has(i))
            .map(|i| self.len_of(i))
            .sum()
    }

    fn complete(&self) -> bool {
        (0..self.chunks()).all(|i| self.has(i))
    }

    /// ช่วงไบต์ที่ยังขาด `[start, end)` รวมช่วงที่ติดกันแล้ว
    fn missing(&self) -> Vec<[u64; 2]> {
        let mut out: Vec<[u64; 2]> = Vec::new();
        for i in (0..self.chunks()).filter(|&i| !self.has(i)) {
            let (start, end) = (i * self.unit, i * self.unit + self.len_of(i));
            match out.last_mut() {
                Some(last) if last[1] == start => last[1] = end,
                _ => out.push([start, end]),
            }
        }
        out
    }

    /// ต่อจากไฟล์ค้างที่มีอยู่: map ของงานเดียวกัน = ใช้ต่อ · ไม่มี map แต่มีไฟล์ค้างของโหมดเดิม
    /// (ข้อมูลเรียงจากต้นไฟล์) = chunk ที่อยู่ในส่วนนั้นครบแล้ว · นอกนั้น = เริ่มใหม่
    fn resume(path: &Path, size: u64, sha256: &str) -> Ranges {
        let fresh = Ranges::new(size, sha256);
        if let Some(saved) = Ranges::load(path) {
            let same_job = saved.size == size
                && saved.sha256 == sha256
                && saved.unit == UNIT
                && saved.bits.len() == fresh.bits.len();
            return if same_job { saved } else { fresh };
        }
        let mut ranges = fresh;
        let have = part_len(path).min(size);
        for i in 0..ranges.chunks() {
            if i * ranges.unit + ranges.len_of(i) <= have {
                ranges.mark(i);
            }
        }
        ranges
    }
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
    /// client ส่ง chunk หลายก้อนพร้อมกันได้ (ดูหัวไฟล์)
    #[serde(default)]
    parallel: bool,
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

fn map_path(path: &Path) -> PathBuf {
    let mut name = part_path(path).into_os_string();
    name.push(".map");
    name.into()
}

/// ทิ้งไฟล์ค้างทั้งสองไฟล์ของปลายทางนี้
fn remove_partial(path: &Path) {
    let _ = std::fs::remove_file(part_path(path));
    let _ = std::fs::remove_file(map_path(path));
}

/// เขียน `data` ที่ตำแหน่ง `offset` โดยไม่ใช้ cursor ร่วม — หลาย chunk เขียนไฟล์เดียวพร้อมกันได้
#[cfg(windows)]
fn write_at(file: &File, mut data: &[u8], mut offset: u64) -> std::io::Result<()> {
    use std::os::windows::fs::FileExt;
    while !data.is_empty() {
        let n = file.seek_write(data, offset)?;
        if n == 0 {
            return Err(std::io::ErrorKind::WriteZero.into());
        }
        data = &data[n..];
        offset += n as u64;
    }
    Ok(())
}

#[cfg(unix)]
fn write_at(file: &File, data: &[u8], offset: u64) -> std::io::Result<()> {
    std::os::unix::fs::FileExt::write_all_at(file, data, offset)
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
        let sha256 = p.sha256.to_lowercase();
        check_free_space(&path, p.size)?;
        let id = random_id(8);
        let mut upload = Upload {
            path: path.clone(),
            size: p.size,
            sha256: sha256.clone(),
            operation_id: p.operation_id,
            touched: Instant::now(),
            ranges: None,
        };
        let reply = if p.parallel {
            let mut ranges = Ranges::resume(&path, p.size, &sha256);
            std::fs::OpenOptions::new()
                .create(true)
                .write(true)
                .truncate(false)
                .open(part_path(&path))?;
            ranges.save(&path)?;
            let active = uploads
                .values()
                .filter(|up| up.ranges.is_some() && up.touched.elapsed() < STALE_UPLOAD)
                .count();
            let plan = transfer_plan::plan(transfer_plan::available_ram(), active);
            let missing = ranges.missing();
            // client 1.0.108 ใช้ chunk_size เป็นขอบ chunk และตรวจว่าทุกช่วงที่ขาดเริ่มที่ขอบนั้น
            // ช่วงที่ขาดเริ่มที่ขอบ UNIT ได้ (ต่อจากโหมดเดิมหรือ client ที่ใช้ chunk ขนาดอื่น) — ไม่ตรงขอบก็เสนอ UNIT แทน
            let chunk_size = if missing.iter().all(|r| r[0].is_multiple_of(plan.chunk_size)) {
                plan.chunk_size
            } else {
                UNIT
            };
            let received = ranges.received_bytes();
            let reply = json!({"transfer_id": id, "mode": "parallel", "unit": UNIT, "chunk_size": chunk_size,
                               "max_chunk_size": plan.max_chunk_size, "max_parallel": plan.max_parallel,
                               "missing": missing, "received": received, "resumed": received > 0});
            upload.ranges = Some(Arc::new(Mutex::new(ranges)));
            reply
        } else {
            // ไฟล์ค้างของโหมดขนาน (มี map) อาจมีช่องว่างกลางไฟล์ ต่อท้ายไม่ได้ · ใหญ่กว่าไฟล์จริง = ของคนละงาน
            let mut offset = part_len(&path);
            if offset > p.size || map_path(&path).exists() {
                remove_partial(&path);
                offset = 0;
            }
            std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(part_path(&path))?;
            json!({"transfer_id": id, "offset": offset, "chunk_size": CHUNK_SIZE,
                   "max_chunk_size": MAX_CHUNK, "resumed": offset > 0})
        };
        uploads.insert(id, upload);
        Ok(reply)
    }

    /// โหมดเดิม: เขียน chunk ต่อท้ายไฟล์ — offset ต้องตรงกับที่ได้รับไปแล้วพอดี (ส่งซ้ำ/ข้ามไม่ได้)
    /// โหมดขนาน: เขียน chunk ลงตำแหน่งของมัน มาไม่เรียงและส่งซ้ำได้
    pub async fn put_chunk(&self, id: &str, offset: u64, data: Vec<u8>) -> Res<Value> {
        let (path, ranges) = {
            let mut map = self.uploads.lock().unwrap();
            let up = map.get_mut(id).ok_or_else(|| unknown(id))?;
            up.touched = Instant::now();
            if offset + data.len() as u64 > up.size {
                return Err(RpcError::invalid("chunk เกินขนาดไฟล์ที่ประกาศไว้"));
            }
            (up.path.clone(), up.ranges.clone())
        };
        if let Some(ranges) = ranges {
            return tokio::task::spawn_blocking(move || put_range(&path, &ranges, offset, &data)).await?;
        }
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
        let (path, size, expected, ranges) = {
            let map = self.uploads.lock().unwrap();
            let up = map.get(&id).ok_or_else(|| unknown(&id))?;
            (up.path.clone(), up.size, up.sha256.clone(), up.ranges.clone())
        };
        // (transfer นี้จบแล้วหรือยัง, ผลลัพธ์) — ยังไม่ครบ = ให้ส่งต่อด้วย id เดิมได้
        let (finished, result) = tokio::task::spawn_blocking(move || -> (bool, Res<Value>) {
            let part = part_path(&path);
            if let Some(ranges) = &ranges {
                let ranges = ranges.lock().unwrap();
                if !ranges.complete() {
                    let msg = format!("ยังไม่ครบ: ได้ {} จาก {size} ไบต์", ranges.received_bytes());
                    return (false, Err(RpcError::new(CONFLICT, msg)));
                }
            }
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
                remove_partial(&path);
                let msg = format!("checksum ไม่ตรง (ได้ {actual}) — ลบไฟล์ค้างแล้ว ให้ transfer.begin ใหม่แล้วส่งทั้งไฟล์");
                return (true, Err(RpcError::new(CONFLICT, msg)));
            }
            match std::fs::rename(&part, &path) {
                Ok(()) => {
                    let _ = std::fs::remove_file(map_path(&path));
                    (true, Ok(json!({"path": path, "size": size, "sha256": actual})))
                }
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
        remove_partial(&up.path);
        Ok(json!({"aborted": true}))
    }
}

/// chunk ของโหมดขนาน: เริ่มที่ขอบ UNIT และยาวเป็นผลคูณของ UNIT (ยกเว้น chunk ที่จบท้ายไฟล์)
/// เขียนข้อมูลก่อนแล้วค่อยจดลง map — ดับกลางทางแล้วแค่ต้องส่ง chunk นั้นซ้ำ map ไม่มีวันอ้างว่าได้ช่วงที่ยังไม่ได้เขียน
fn put_range(path: &Path, ranges: &Mutex<Ranges>, offset: u64, data: &[u8]) -> Res<Value> {
    let (first, last) = {
        let ranges = ranges.lock().unwrap();
        let end = offset + data.len() as u64;
        let whole_units = (data.len() as u64).is_multiple_of(ranges.unit) || end == ranges.size;
        if data.is_empty() || !offset.is_multiple_of(ranges.unit) || !whole_units {
            let msg = format!(
                "โหมดขนาน: chunk ต้องเริ่มที่ขอบ {0} ไบต์และยาวเป็นผลคูณของ {0} ไบต์",
                ranges.unit
            );
            return Err(RpcError::invalid(msg));
        }
        (offset / ranges.unit, end.div_ceil(ranges.unit))
    };
    let file = std::fs::OpenOptions::new().write(true).open(part_path(path))?;
    write_at(&file, data, offset)?;
    let mut ranges = ranges.lock().unwrap();
    (first..last).for_each(|i| ranges.mark(i));
    ranges.save(path)?;
    Ok(json!({"offset": offset + data.len() as u64, "received": ranges.received_bytes()}))
}

/// ปฏิเสธตั้งแต่ begin ถ้าดิสก์ปลายทางไม่พอ — ไม่งั้นจะรู้ตอนส่งไปแล้วหลาย GB
/// นับส่วนที่ได้แล้วใน `.synapta-part` ว่าไม่ต้องใช้พื้นที่เพิ่ม
fn check_free_space(path: &Path, size: u64) -> Res<()> {
    let Some(free) = path.parent().and_then(transfer_plan::free_space) else {
        return Ok(());
    };
    let need = size.saturating_sub(part_len(path));
    if need > free {
        let msg = format!("พื้นที่ดิสก์ปลายทางไม่พอ: ต้องใช้อีก {need} ไบต์ เหลือ {free} ไบต์");
        return Err(RpcError::new(CONFLICT, msg));
    }
    Ok(())
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
