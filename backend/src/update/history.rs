//! ประวัติการอัปเดต (JSON lines ใน data_dir/update-history.jsonl) — panel แสดงเป็นรายการ "ใหม่สุดก่อน"
//!
//! มีสองผู้เขียน: agent (ตรวจ/ดาวน์โหลด) และตัวติดตั้งแบบ SYSTEM (เปลี่ยนรุ่น/กู้คืน) — เขียนแบบ append ทีละบรรทัด
//! outcome: up_to_date | available | staged | installed | failed | rolled_back

use crate::auth::now_secs;
use serde::{Deserialize, Serialize};
use std::{io::Write, path::PathBuf};

const KEEP: usize = 200;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Entry {
    pub at: u64,
    pub from: String,
    pub to: Option<String>,
    pub outcome: String,
    pub stage: Option<String>,
    pub code: Option<String>,
    pub message: Option<String>,
    pub attempt: u32,
}

impl Entry {
    pub fn new(from: &str, to: Option<&str>, outcome: &str) -> Entry {
        Entry {
            at: now_secs(),
            from: from.into(),
            to: to.map(Into::into),
            outcome: outcome.into(),
            stage: None,
            code: None,
            message: None,
            attempt: 0,
        }
    }
    pub fn with_error(mut self, e: &super::error::UpdateError, attempt: u32) -> Entry {
        self.stage = Some(e.stage.clone());
        self.code = Some(e.code.clone());
        self.message = Some(e.message.clone());
        self.attempt = attempt;
        self
    }
}

pub struct History {
    path: PathBuf,
    /// ไฟล์ของอีกฝั่งที่อ่านรวมด้วย (อ่านอย่างเดียว) — ไม่เขียนข้ามที่ของกันและกัน
    also_read: Option<PathBuf>,
}

impl History {
    /// ฝั่ง agent: เขียนของตัวเอง อ่านรวมกับของตัวติดตั้ง
    pub fn new(data_dir: &std::path::Path) -> History {
        History {
            path: data_dir.join("update-history.jsonl"),
            also_read: Some(data_dir.join("updater-status").join("history.jsonl")),
        }
    }

    /// ฝั่งตัวติดตั้ง: เขียนในพื้นที่ที่ agent เขียนไม่ได้เท่านั้น
    pub fn installer(status_dir: &std::path::Path) -> History {
        History {
            path: status_dir.join("history.jsonl"),
            also_read: None,
        }
    }

    pub fn append(&self, entry: &Entry) {
        let line = serde_json::to_string(entry).unwrap_or_default();
        let written = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.path)
            .and_then(|mut f| writeln!(f, "{line}"));
        if let Err(e) = written {
            tracing::error!("บันทึกประวัติอัปเดตไม่สำเร็จ: {e}");
        }
        self.trim();
    }

    /// ใหม่สุดก่อน; บรรทัดที่อ่านไม่ออก (เช่นเขียนค้างตอนไฟดับ) ข้ามไป
    pub fn recent(&self, limit: usize) -> Vec<Entry> {
        let mut entries: Vec<Entry> = std::iter::once(&self.path)
            .chain(self.also_read.iter())
            .flat_map(|p| {
                std::fs::read_to_string(p)
                    .unwrap_or_default()
                    .lines()
                    .map(String::from)
                    .collect::<Vec<_>>()
            })
            .filter_map(|l| serde_json::from_str(&l).ok())
            .collect();
        // เวลาเท่ากันให้ลำดับเดิมในไฟล์ (stable) — เรียงเก่า→ใหม่ แล้วกลับ
        entries.sort_by_key(|e| e.at);
        entries.reverse();
        entries.truncate(limit);
        entries
    }

    fn trim(&self) {
        let text = std::fs::read_to_string(&self.path).unwrap_or_default();
        let lines: Vec<&str> = text.lines().collect();
        if lines.len() > KEEP * 2 {
            let kept = lines[lines.len() - KEEP..].join("\n") + "\n";
            let _ = crate::fsutil::atomic_write(&self.path, kept.as_bytes());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn appends_newest_first_skips_torn_lines_and_trims() {
        let dir = std::env::temp_dir().join(format!("synapta-hist-{}", crate::hex::random_id(4)));
        std::fs::create_dir_all(&dir).unwrap();
        let h = History::new(&dir);
        assert!(h.recent(10).is_empty());
        h.append(&Entry::new("0.3.0", Some("0.3.1"), "staged"));
        std::fs::OpenOptions::new()
            .append(true)
            .open(dir.join("update-history.jsonl"))
            .unwrap()
            .write_all(b"{\"at\":1,\"fro")
            .unwrap();
        std::fs::OpenOptions::new()
            .append(true)
            .open(dir.join("update-history.jsonl"))
            .unwrap()
            .write_all(b"\n")
            .unwrap();
        h.append(&Entry::new("0.3.0", Some("0.3.1"), "installed"));
        let recent = h.recent(10);
        assert_eq!(
            recent.iter().map(|e| e.outcome.as_str()).collect::<Vec<_>>(),
            ["installed", "staged"]
        );

        for _ in 0..(KEEP * 2 + 5) {
            h.append(&Entry::new("0.3.0", None, "up_to_date"));
        }
        assert!(h.recent(10_000).len() <= KEEP * 2, "ต้องไม่โตไม่จำกัด");
        std::fs::remove_dir_all(dir).unwrap();
    }
}
