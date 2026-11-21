//! สถานะที่ต้องจำข้ามการรีสตาร์ท: ลองรุ่นเป้าหมายไปกี่ครั้ง ลองได้อีกเมื่อไร ผลตรวจล่าสุด และงานที่ส่งให้ตัวติดตั้งแล้ว
//!
//! กฎลองใหม่อยู่ที่นี่ที่เดียว: ต่อรุ่นเป้าหมายลองอัตโนมัติได้ MAX_ATTEMPTS ครั้ง ห่างกัน 1 ชม. แล้ว 6 ชม.
//! ความผิดแบบถาวร (ลายเซ็น/manifest ผิด) นับครบทันที — เจ้าของยังสั่งลองเองได้ (เคลียร์ตัวนับ)

use super::error::UpdateError;
use crate::{auth::now_secs, fsutil};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

pub const MAX_ATTEMPTS: u32 = 3;
const BACKOFF_SECS: [u64; 2] = [3600, 6 * 3600];

/// release ที่ตรวจพบว่าใหม่กว่ารุ่นที่ติดตั้ง (แสดงใน panel)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Available {
    pub version: String,
    pub notes: String,
    pub published_at: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Pending {
    pub version: String,
    pub requested_at: u64,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct UpdateState {
    /// รุ่นที่กำลังลองอยู่ (ตัวนับด้านล่างเป็นของรุ่นนี้)
    pub target: Option<String>,
    pub attempts: u32,
    pub next_attempt_at: Option<u64>,
    pub last_error: Option<UpdateError>,
    pub last_error_at: Option<u64>,
    pub last_check_at: Option<u64>,
    pub available: Option<Available>,
    /// รหัสผลล่าสุดของตัวติดตั้งที่อ่านแล้ว (ไฟล์ผลอยู่ในที่ที่ agent ลบไม่ได้ จึงต้องจำเอง)
    pub seen_result: Option<String>,
    /// ส่งให้ตัวติดตั้งแล้วแต่ยังไม่รู้ผล — เริ่ม agent ใหม่แล้วใช้ตัดสินว่าสำเร็จหรือไม่
    pub pending: Option<Pending>,
}

impl UpdateState {
    pub fn record_failure(&mut self, version: &str, error: UpdateError) {
        if self.target.as_deref() != Some(version) {
            self.target = Some(version.into());
            self.attempts = 0;
        }
        self.attempts = if error.is_permanent() {
            MAX_ATTEMPTS
        } else {
            self.attempts + 1
        };
        self.next_attempt_at = (self.attempts < MAX_ATTEMPTS)
            .then(|| now_secs() + BACKOFF_SECS[(self.attempts as usize - 1).min(BACKOFF_SECS.len() - 1)]);
        self.last_error = Some(error);
        self.last_error_at = Some(now_secs());
    }

    /// ลองรุ่นนี้ให้ระบบเริ่มเองได้หรือยัง (รุ่นใหม่ที่ไม่เคยลอง = ได้เสมอ)
    pub fn may_auto_try(&self, version: &str) -> bool {
        self.target.as_deref() != Some(version)
            || (self.attempts < MAX_ATTEMPTS && self.next_attempt_at.is_none_or(|t| now_secs() >= t))
    }

    /// เจ้าของสั่งลองเอง = เริ่มนับใหม่
    pub fn reset_for_manual(&mut self, version: &str) {
        self.target = Some(version.into());
        self.attempts = 0;
        self.next_attempt_at = None;
    }

    pub fn record_success(&mut self) {
        *self = UpdateState {
            last_check_at: self.last_check_at,
            seen_result: self.seen_result.take(),
            ..UpdateState::default()
        };
    }
}

pub struct StateFile(PathBuf);

impl StateFile {
    pub fn new(data_dir: &std::path::Path) -> StateFile {
        StateFile(data_dir.join("update-state.json"))
    }
    pub fn load(&self) -> UpdateState {
        fsutil::read_json(&self.0).unwrap_or_default()
    }
    pub fn save(&self, state: &UpdateState) {
        if let Err(e) = fsutil::write_json(&self.0, state) {
            tracing::error!("บันทึก update-state.json ไม่สำเร็จ: {e}");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn transient() -> UpdateError {
        UpdateError::new("download", "download_failed", "x")
    }

    #[test]
    fn auto_retry_is_bounded_and_backed_off() {
        let mut s = UpdateState::default();
        assert!(s.may_auto_try("1.0.1"));
        s.record_failure("1.0.1", transient());
        assert_eq!(s.attempts, 1);
        assert!(!s.may_auto_try("1.0.1"), "ต้องรอ backoff");
        assert!(s.may_auto_try("1.0.2"), "รุ่นใหม่เริ่มนับใหม่");

        s.next_attempt_at = Some(0); // ข้ามเวลารอ
        assert!(s.may_auto_try("1.0.1"));
        s.record_failure("1.0.1", transient());
        s.next_attempt_at = Some(0);
        s.record_failure("1.0.1", transient());
        assert_eq!(s.attempts, MAX_ATTEMPTS);
        assert_eq!(s.next_attempt_at, None);
        assert!(!s.may_auto_try("1.0.1"), "ครบเพดานแล้วหยุดเอง");

        s.reset_for_manual("1.0.1");
        assert!(s.may_auto_try("1.0.1"));
    }

    #[test]
    fn permanent_errors_use_up_the_quota_at_once() {
        let mut s = UpdateState::default();
        s.record_failure("1.0.1", UpdateError::new("verify", "signature_invalid", "x"));
        assert_eq!(s.attempts, MAX_ATTEMPTS);
        assert!(!s.may_auto_try("1.0.1"));
    }

    #[test]
    fn state_survives_restart_and_success_clears_errors_but_keeps_last_check() {
        let dir = std::env::temp_dir().join(format!("synapta-state-{}", crate::hex::random_id(4)));
        std::fs::create_dir_all(&dir).unwrap();
        let file = StateFile::new(&dir);
        let mut s = file.load();
        s.last_check_at = Some(5);
        s.record_failure("1.0.1", transient());
        file.save(&s);
        let mut loaded = file.load();
        assert_eq!(loaded.attempts, 1);
        loaded.record_success();
        assert_eq!(
            (loaded.attempts, loaded.last_error.is_none(), loaded.last_check_at),
            (0, true, Some(5))
        );
        std::fs::remove_dir_all(dir).unwrap();
    }
}
