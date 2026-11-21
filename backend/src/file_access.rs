//! กฎเดียวของ "path ที่ client ส่งมาเข้าถึงไฟล์ไหนได้" สำหรับ transfer.* และ GET /files
//!
//! ทุก path ต้องผ่าน canonicalize (ตาม symlink/`.`/`..` แล้ว) ก่อนเทียบ `--file-root` — เทียบ string ดิบโดนเลี่ยงได้
//! ถ้าไม่ตั้ง `--file-root` เลยจะไม่จำกัดโฟลเดอร์ (เหตุผลเดียวกับ exec: session ที่ pair แล้วรันคำสั่งได้อยู่แล้ว)
//! แต่ยังปฏิเสธ `..` ใน path เขียน และไม่เขียนทะลุ symlink

use crate::rpc::{Res, RpcError};
use std::path::{Component, Path, PathBuf};

fn inside_roots(path: &Path, roots: &[PathBuf]) -> Res<()> {
    if roots.is_empty() || roots.iter().any(|r| path.starts_with(r)) {
        Ok(())
    } else {
        Err(RpcError::invalid("path อยู่นอกโฟลเดอร์ที่อนุญาต (--file-root)"))
    }
}

/// path ของไฟล์ที่จะอ่าน: ต้องมีอยู่ เป็นไฟล์ และอยู่ใน root — คืน path แบบ canonical
pub fn for_read(path: &str, roots: &[PathBuf]) -> Res<PathBuf> {
    let canonical = Path::new(path)
        .canonicalize()
        .map_err(|e| RpcError::invalid(format!("เปิด path ไม่ได้: {e}")))?;
    inside_roots(&canonical, roots)?;
    if !canonical.is_file() {
        return Err(RpcError::invalid("ไม่ใช่ไฟล์"));
    }
    Ok(canonical)
}

/// path ของไฟล์ที่จะเขียน (ไฟล์/โฟลเดอร์แม่ยังไม่ต้องมี): absolute, ไม่มี `..`, ไม่ใช่ symlink, อยู่ใน root
/// คืน path ที่ canonicalize ส่วนที่มีอยู่แล้ว — ยังไม่สร้างอะไรบนดิสก์ (ผู้เรียกสร้างโฟลเดอร์หลังผ่านการตรวจนี้)
pub fn for_write(path: &str, roots: &[PathBuf]) -> Res<PathBuf> {
    let given = Path::new(path);
    if !given.is_absolute() {
        return Err(RpcError::invalid("path ต้องเป็น absolute path"));
    }
    if given.components().any(|c| c == Component::ParentDir) {
        return Err(RpcError::invalid("path ห้ามมี .."));
    }
    if given.symlink_metadata().is_ok_and(|m| m.file_type().is_symlink()) {
        return Err(RpcError::invalid("ปลายทางเป็น symlink — ไม่เขียนทะลุ"));
    }
    // หาโฟลเดอร์แรกที่มีอยู่จริง แล้ว canonicalize เฉพาะส่วนนั้น ส่วนที่เหลือต่อท้ายตามเดิม
    let mut existing = given;
    let mut missing = Vec::new();
    while !existing.exists() {
        missing.push(existing.file_name().ok_or_else(|| RpcError::invalid("path ไม่ถูกต้อง"))?);
        existing = existing.parent().ok_or_else(|| RpcError::invalid("path ไม่ถูกต้อง"))?;
    }
    let mut resolved = existing.canonicalize()?;
    resolved.extend(missing.iter().rev());
    inside_roots(&resolved, roots)?;
    Ok(resolved)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sandbox() -> (PathBuf, Vec<PathBuf>) {
        let base = std::env::temp_dir().join(format!("synapta-fa-{}", crate::hex::random_id(4)));
        std::fs::create_dir_all(base.join("root")).unwrap();
        std::fs::write(base.join("outside.txt"), "x").unwrap();
        std::fs::write(base.join("root").join("in.txt"), "x").unwrap();
        let root = base.join("root").canonicalize().unwrap();
        (base.canonicalize().unwrap(), vec![root])
    }

    #[test]
    fn write_rejects_parent_dir_and_paths_outside_roots() {
        let (base, roots) = sandbox();
        let escape = format!("{}/root/../outside.txt", base.display());
        assert!(for_write(&escape, &roots).is_err(), "`..` ต้องถูกปฏิเสธ");
        assert!(for_write(&base.join("outside2.txt").to_string_lossy(), &roots).is_err());
        let ok = for_write(&base.join("root").join("new").join("a.txt").to_string_lossy(), &roots).unwrap();
        assert!(ok.ends_with("a.txt") && ok.starts_with(&roots[0]));
        assert!(!base.join("root").join("new").exists(), "การตรวจต้องไม่สร้างโฟลเดอร์");
    }

    #[test]
    fn read_rejects_outside_roots_but_allows_everything_without_roots() {
        let (base, roots) = sandbox();
        let outside = base.join("outside.txt").to_string_lossy().to_string();
        assert!(for_read(&outside, &roots).is_err());
        assert!(for_read(&base.join("root").join("in.txt").to_string_lossy(), &roots).is_ok());
        assert!(for_read(&outside, &[]).is_ok());
        // เข้าทาง `..` จากใน root ก็ต้องโดน canonicalize แล้วปฏิเสธ
        let sneaky = format!("{}/root/../outside.txt", base.display());
        assert!(for_read(&sneaky, &roots).is_err());
    }
}
