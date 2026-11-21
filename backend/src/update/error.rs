//! ความล้มเหลวของการอัปเดต: `code` ให้ panel ทำปุ่ม/คำแนะนำ, `message` เป็นภาษาไทยพร้อมแสดง
//! ตารางความหมายของ code อยู่ที่ docs/PANEL_API.md §3 — เพิ่ม code ใหม่ต้องแก้ตารางนั้นด้วย

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct UpdateError {
    pub code: String,
    /// ขั้นที่ล้ม: check | download | verify | apply | health | rollback
    pub stage: String,
    pub message: String,
}

impl UpdateError {
    pub fn new(stage: &str, code: &str, message: impl Into<String>) -> UpdateError {
        UpdateError {
            code: code.into(),
            stage: stage.into(),
            message: message.into(),
        }
    }

    /// ความผิดที่ลองซ้ำก็ได้ผลเหมือนเดิม — นับเป็นใช้โควตาลองใหม่หมดทันที (เจ้าของยังสั่งลองเองได้)
    pub fn is_permanent(&self) -> bool {
        matches!(
            self.code.as_str(),
            "signature_invalid" | "manifest_invalid" | "version_not_newer" | "not_configured"
        )
    }
}

impl std::fmt::Display for UpdateError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{} ({}): {}", self.code, self.stage, self.message)
    }
}
