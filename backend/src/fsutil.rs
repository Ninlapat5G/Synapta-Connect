//! อ่าน/เขียนไฟล์สถานะ (JSON) แบบไม่พังถ้าไฟดับกลางทาง

use serde::{Serialize, de::DeserializeOwned};
use std::{io, path::Path};

/// เขียนไฟล์ชั่วคราวแล้ว rename ทับ — ผู้อ่านเห็นไฟล์เก่าเต็มไฟล์หรือไฟล์ใหม่เต็มไฟล์เท่านั้น
pub fn atomic_write(path: &Path, data: &[u8]) -> io::Result<()> {
    let tmp = path.with_extension("tmp");
    let mut file = std::fs::File::create(&tmp)?;
    io::Write::write_all(&mut file, data)?;
    file.sync_all()?; // ไม่ sync = หลังไฟดับ rename อาจชี้ไปไฟล์ว่าง
    drop(file);
    std::fs::rename(&tmp, path)
}

pub fn write_json<T: Serialize>(path: &Path, value: &T) -> io::Result<()> {
    atomic_write(path, &serde_json::to_vec_pretty(value).map_err(io::Error::other)?)
}

/// ไฟล์ไม่มี/อ่านไม่ออก = None (ผู้เรียกใช้ค่าเริ่มต้น)
pub fn read_json<T: DeserializeOwned>(path: &Path) -> Option<T> {
    serde_json::from_slice(&std::fs::read(path).ok()?).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn json_roundtrip_and_missing_file() {
        let dir = std::env::temp_dir().join(format!("synapta-fsutil-{}", crate::hex::random_id(4)));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("a.json");
        assert_eq!(read_json::<Vec<u32>>(&file), None);
        write_json(&file, &vec![1, 2, 3]).unwrap();
        assert_eq!(read_json::<Vec<u32>>(&file), Some(vec![1, 2, 3]));
        std::fs::remove_dir_all(dir).unwrap();
    }
}
