//! ส่งไฟล์แบบขนานได้แค่ไหน — คำนวณจาก RAM ที่ว่างตอนนี้และจำนวนงานที่กำลังส่งอยู่ (ไม่ใช้ตัวเลขตายตัว)
//! และพื้นที่ว่างของดิสก์ปลายทาง ส่วนความเร็วเน็ตเป็นหน้าที่ของ client ที่วัดเองแล้วปรับจำนวน request ภายในเพดานนี้

use std::path::Path;

/// ความละเอียดของ map ความคืบหน้า — chunk ทุกก้อนต้องเริ่มที่ขอบนี้
pub const UNIT: u64 = 1024 * 1024;
/// chunk ใหญ่สุดที่รับได้ไม่ว่าเครื่องจะมี RAM เท่าไร (http.rs ตั้ง body limit ตามนี้)
pub const MAX_CHUNK: u64 = 16 * UNIT;
/// SynaptaCode 1.0.108 ยอมรับ `chunk_size` ไม่เกิน 4 MiB — ค่าเริ่มต้นที่เสนอจึงไม่เกินนี้
const LEGACY_CHUNK: u64 = 4 * UNIT;
const MIN_PARALLEL: u64 = 2;
/// TLS รับ handshake ที่ยังไม่จบได้ 8 ต่อ IP (tls.rs) — client ต้องเพิ่ม request ทีละตัว ไม่เปิดพร้อมกันทีเดียว
const MAX_PARALLEL: u64 = 16;

pub struct Plan {
    /// ขนาด chunk เริ่มต้นที่เสนอ (ผลคูณของ UNIT)
    pub chunk_size: u64,
    /// chunk ใหญ่สุดที่ client ขยายไปได้
    pub max_chunk_size: u64,
    /// request พร้อมกันสูงสุด
    pub max_parallel: u64,
}

/// `active` = งานอัปโหลดแบบขนานที่ยังเดินอยู่ (ไม่นับงานนี้)
///
/// agent รับ body ทั้งก้อนเข้า RAM ก่อนเขียน — งานหนึ่งใช้ได้ถึง chunk × parallel จึงแบ่งงบจาก RAM ว่าง:
/// ไม่เกิน 1/32 ของ RAM ว่าง (16-256 MiB) หารด้วยจำนวนงานที่ส่งพร้อมกัน
pub fn plan(available_ram: u64, active: usize) -> Plan {
    let budget = (available_ram / 32 / (active as u64 + 1)).clamp(16 * UNIT, 256 * UNIT);
    let max_chunk_size = (budget / 8 / UNIT * UNIT).clamp(UNIT, MAX_CHUNK);
    Plan {
        chunk_size: max_chunk_size.min(LEGACY_CHUNK),
        max_chunk_size,
        max_parallel: (budget / max_chunk_size).clamp(MIN_PARALLEL, MAX_PARALLEL),
    }
}

/// RAM ที่ว่างตอนนี้ — อ่านไม่ได้ให้ถือว่าน้อย (ได้งบขั้นต่ำ)
pub fn available_ram() -> u64 {
    sysinfo::System::new()
        .map(|mut sys| {
            sys.refresh_memory();
            sys.available_memory()
        })
        .unwrap_or(0)
}

/// พื้นที่ว่างของไดรฟ์ที่ `dir` อยู่ — None = อ่านไม่ได้ (ไม่ขวางการส่ง)
#[cfg(windows)]
pub fn free_space(dir: &Path) -> Option<u64> {
    use std::os::windows::ffi::OsStrExt;
    use windows::{Win32::Storage::FileSystem::GetDiskFreeSpaceExW, core::PCWSTR};
    let wide: Vec<u16> = dir.as_os_str().encode_wide().chain([0]).collect();
    let mut free = 0u64;
    unsafe { GetDiskFreeSpaceExW(PCWSTR(wide.as_ptr()), Some(&mut free), None, None) }.ok()?;
    Some(free)
}

#[cfg(not(windows))]
pub fn free_space(_dir: &Path) -> Option<u64> {
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    const GIB: u64 = 1024 * 1024 * 1024;

    #[test]
    fn plan_scales_with_free_ram_and_shares_it_between_uploads() {
        let big = plan(32 * GIB, 0);
        assert_eq!((big.max_chunk_size, big.max_parallel), (16 * UNIT, 16));
        assert_eq!(big.chunk_size, 4 * UNIT);

        let small = plan(512 * 1024 * 1024, 0);
        assert_eq!((small.max_chunk_size, small.max_parallel), (2 * UNIT, 8));

        // งานที่สองได้งบครึ่งหนึ่ง
        let shared = plan(4 * GIB, 1);
        let alone = plan(4 * GIB, 0);
        assert!(shared.max_chunk_size * shared.max_parallel < alone.max_chunk_size * alone.max_parallel);

        // อ่าน RAM ไม่ได้ = ยังส่งได้ด้วยงบขั้นต่ำ
        let unknown = plan(0, 0);
        assert!(unknown.max_parallel >= MIN_PARALLEL && unknown.max_chunk_size >= UNIT);
    }

    #[cfg(windows)]
    #[test]
    fn free_space_reads_the_drive_and_ram_is_known() {
        assert!(free_space(&std::env::temp_dir()).is_some_and(|free| free > 0));
        assert!(available_ram() > 0);
    }
}
