//! พิสูจน์ว่า "อีกฝั่งของ loopback คือ agent จริง" โดยไม่ส่ง local_token ออกไปทางสาย
//!
//! พอร์ต 52402 บน 127.0.0.1 ใครบนเครื่องก็ bind ได้ตอน service ไม่ทำงาน (หยุด/crash/กำลังสลับไฟล์/ก่อนบูตเสร็จ)
//! ฝั่งที่ถือ token ต้องตรวจก่อนว่าคู่สนทนารู้ token จริง: ส่ง nonce สุ่มไป แล้วต้องได้ HMAC-SHA256(token, ป้าย ‖ ข้อความ) กลับมา
//! ผู้ฟังปลอมอ่านไฟล์ token ไม่ได้ (ACL) จึงตอบไม่ได้ — token ไม่ถูกส่งไปให้ผู้ฟังปลอมเลย (ต่างจากการแนบ header X-Synapta-Local)
//! ใช้กับ: helper ↔ agent (WebSocket), ตัวติดตั้งตรวจสุขภาพ, คำสั่ง `panel`/`service status`

use crate::hex::to_hex;
use hmac::{Hmac, KeyInit, Mac};
use sha2::Sha256;

/// ป้ายแยกบริบท: proof ของ helper เอาไปใช้เป็น proof ของ health ไม่ได้ และกลับกัน
pub const HELPER: &str = "helper-hello";
pub const SERVER: &str = "helper-server";
pub const HEALTH: &str = "health";
pub const PANEL_CONTROL: &str = "panel-control";

pub fn control_message(time: &str, nonce: &str, target: &str) -> String {
    format!("{time}|{nonce}|{target}")
}

pub fn valid_local_token(token: &str) -> bool {
    token.len() == 64 && token.bytes().all(|b| b.is_ascii_hexdigit())
}

fn mac(token: &str, label: &str, message: &str) -> Hmac<Sha256> {
    let mut m = Hmac::<Sha256>::new_from_slice(token.as_bytes()).expect("HMAC รับกุญแจความยาวใดก็ได้");
    m.update(label.as_bytes());
    m.update(&[0]);
    m.update(message.as_bytes());
    m
}

pub fn make(token: &str, label: &str, message: &str) -> String {
    to_hex(&mac(token, label, message).finalize().into_bytes())
}

/// ตรวจแบบเวลาคงที่
pub fn check(token: &str, label: &str, message: &str, given_hex: &str) -> bool {
    crate::update::manifest::decode_hex(given_hex)
        .is_some_and(|given| mac(token, label, message).verify_slice(&given).is_ok())
}

/// ข้อความที่ผูกกับคำตอบ health: nonce ของผู้ถาม + รุ่น + pid (ตอบซ้ำของเก่า/ปนรุ่นไม่ได้)
pub fn health_message(nonce: &str, version: &str, pid: u32) -> String {
    format!("{nonce}|{version}|{pid}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn proof_binds_token_label_and_message() {
        let p = make("secret", HELPER, "n1");
        assert!(check("secret", HELPER, "n1", &p));
        assert!(!check("other", HELPER, "n1", &p), "token ต่าง");
        assert!(!check("secret", SERVER, "n1", &p), "ป้ายต่าง (ใช้ข้ามบริบทไม่ได้)");
        assert!(!check("secret", HELPER, "n2", &p), "nonce ต่าง");
        assert!(!check("secret", HELPER, "n1", "zz"));
        assert!(!check("secret", HELPER, "n1", ""));
    }
}
