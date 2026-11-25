//! โหมดจับคู่ (OTP) + รายการอุปกรณ์ที่จับคู่แล้ว (allowlist ของ public key) + การเก็บลงดิสก์
//!
//! อุปกรณ์ = public key Ed25519 ของ client; agent ไม่เก็บความลับใด ๆ ของ client
//! ตัวตนตอนใช้งานคือใบรับรอง mTLS ที่มี key นี้ (tls.rs ตรวจ) — ไม่มี token
//! ขั้นตอนพิสูจน์ว่ารู้ OTP (SPAKE2) อยู่ใน pairing.rs; ไฟล์นี้ถือแค่ "OTP ตัวไหนใช้ได้ และเดาผิดไปกี่ครั้ง"

use crate::hex::{random_id, to_hex};
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD as B64URL};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::HashMap,
    net::IpAddr,
    path::PathBuf,
    sync::Mutex,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};
use tokio::sync::broadcast;

const OTP_DIGITS: usize = 8;
const OTP_TTL_SECS: u64 = 120;
const MAX_WRONG_GUESSES: u32 = 5;
const MAX_RECENT_FAILURES: usize = 20;
const LOCKOUT: Duration = Duration::from_secs(60);
const PORTABLE_ATTACH_WINDOW: Duration = Duration::from_secs(30);
/// ข้อความใน kill channel ที่แปลว่า "ตัดทุก connection"
pub const KILL_ALL: &str = "*";

pub fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

#[derive(Clone, Serialize)]
pub struct OtpInfo {
    pub otp: String,
    pub expires_at: u64,
}

/// อุปกรณ์ที่จับคู่แล้ว — `id` คือ fingerprint ของ public key (sha256 ตัดเหลือ 32 ตัวอักษร hex)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Device {
    pub id: String,
    pub client_name: String,
    /// public key Ed25519 ดิบ 32 ไบต์ (base64url ไม่มี padding)
    pub public_key: String,
    pub created_at: u64,
    /// ค่าสดอยู่ในหน่วยความจำ; ในไฟล์เป็นค่า ณ ครั้งล่าสุดที่บันทึก
    #[serde(default)]
    pub last_seen: u64,
}

/// การเดา OTP ผิดหนึ่งครั้ง — ข้อมูลที่เจ้าของเครื่องใช้ตัดสินว่าต้องปิดโหมดจับคู่หรือไม่
#[derive(Debug, Clone, Serialize)]
pub struct GuessFailure {
    pub at: u64,
    pub ip: String,
    pub client_name: String,
}

#[derive(Debug, PartialEq)]
pub enum PairError {
    /// ไม่ได้เปิดโหมดจับคู่ หรือ OTP หมดอายุ/ถูกทำลายแล้ว
    Closed,
    Locked {
        retry_in_secs: u64,
    },
    /// OTP เปลี่ยนไประหว่างทำ handshake — ต้องเริ่มใหม่
    Stale,
    BadKey,
}

struct Pairing {
    otp: OtpInfo,
    /// เพิ่มทุกครั้งที่ออก OTP ใหม่ ใช้ตรวจว่า handshake ที่ค้างอยู่เป็นของ OTP ปัจจุบัน
    generation: u64,
}

struct State {
    /// None = ปิดรับจับคู่ (ค่าเริ่มต้น)
    pairing: Option<Pairing>,
    next_generation: u64,
    devices: HashMap<String, Device>,
    wrong_guesses: u32,
    locked_until: Option<Instant>,
    /// การเดาผิดล่าสุด (ไว้ให้เจ้าของเครื่องดูว่าใครกำลังเดา) — ใหม่สุดอยู่ท้าย
    recent_failures: Vec<GuessFailure>,
    portable_control: Option<PortableControl>,
}

enum PortableControl {
    Waiting { device: String, until: Instant },
    Connected(String),
}

pub struct Auth {
    state: Mutex<State>,
    devices_file: Option<PathBuf>,
    /// ส่ง device id ที่ถูก revoke (หรือ KILL_ALL) เพื่อให้ tls.rs ตัด connection ของมัน
    kill: broadcast::Sender<String>,
}

fn new_otp() -> OtpInfo {
    let n = rand::random::<u64>() % 10u64.pow(OTP_DIGITS as u32);
    OtpInfo {
        otp: format!("{n:0width$}", width = OTP_DIGITS),
        expires_at: now_secs() + OTP_TTL_SECS,
    }
}

/// id ของอุปกรณ์จาก public key ดิบ
pub fn device_id(public_key: &[u8]) -> String {
    to_hex(&Sha256::digest(public_key))[..32].to_string()
}

impl Auth {
    /// โหลดรายการอุปกรณ์จาก data_dir (ไฟล์ paired_devices.json)
    pub fn load(data_dir: &std::path::Path) -> std::io::Result<Auth> {
        std::fs::create_dir_all(data_dir)?;
        let devices_file = data_dir.join("paired_devices.json");
        let devices: Vec<Device> = std::fs::read(&devices_file)
            .ok()
            .and_then(|b| serde_json::from_slice(&b).ok())
            .unwrap_or_default();
        Ok(Self::new(Some(devices_file), devices))
    }

    pub(crate) fn ephemeral() -> Self {
        Self::new(None, Vec::new())
    }

    fn new(devices_file: Option<PathBuf>, devices: Vec<Device>) -> Self {
        Self {
            state: Mutex::new(State {
                pairing: None,
                next_generation: 1,
                devices: devices.into_iter().map(|d| (d.id.clone(), d)).collect(),
                wrong_guesses: 0,
                locked_until: None,
                recent_failures: Vec::new(),
                portable_control: None,
            }),
            devices_file,
            kill: broadcast::channel(16).0,
        }
    }

    // ---------- โหมดจับคู่ ----------

    /// เปิดโหมดจับคู่ด้วย OTP ใหม่ (ตัวเก่าใช้ไม่ได้ทันที)
    pub fn open_pairing(&self) -> OtpInfo {
        let mut st = self.state.lock().unwrap();
        Self::issue_otp(&mut st)
    }

    fn issue_otp(st: &mut State) -> OtpInfo {
        let otp = new_otp();
        let generation = st.next_generation;
        st.next_generation += 1;
        st.pairing = Some(Pairing {
            otp: otp.clone(),
            generation,
        });
        otp
    }

    pub(crate) fn portable_otp(&self) -> Option<OtpInfo> {
        let mut st = self.state.lock().unwrap();
        if let Some(PortableControl::Waiting { device, until }) = &st.portable_control
            && *until <= Instant::now()
        {
            let device = device.clone();
            st.portable_control = None;
            st.devices.remove(&device);
            let _ = self.kill.send(device);
        }
        if st.portable_control.is_some() || st.locked_until.is_some_and(|until| until > Instant::now()) {
            return None;
        }
        if let Some(pairing) = &st.pairing
            && pairing.otp.expires_at > now_secs()
        {
            return Some(pairing.otp.clone());
        }
        Some(Self::issue_otp(&mut st))
    }

    pub(crate) fn attach_portable(&self, device_id: &str) -> bool {
        let mut st = self.state.lock().unwrap();
        if matches!(&st.portable_control, Some(PortableControl::Waiting { device, until })
            if device == device_id && *until > Instant::now())
        {
            st.portable_control = Some(PortableControl::Connected(device_id.into()));
            return true;
        }
        false
    }

    pub(crate) fn portable_attached(&self, device_id: &str) -> bool {
        let st = self.state.lock().unwrap();
        matches!(&st.portable_control, Some(PortableControl::Connected(device)) if device == device_id)
    }

    pub fn close_pairing(&self) {
        self.state.lock().unwrap().pairing = None;
    }

    /// OTP ที่ยังใช้ได้ (None = ปิดโหมดจับคู่หรือหมดอายุ)
    pub fn current_otp(&self) -> Option<OtpInfo> {
        let st = self.state.lock().unwrap();
        st.pairing
            .as_ref()
            .map(|p| p.otp.clone())
            .filter(|o| o.expires_at > now_secs())
    }

    /// เริ่ม handshake: คืน (OTP, generation) ถ้าเปิดโหมดจับคู่อยู่และไม่ถูกล็อก
    pub fn begin_pairing(&self) -> Result<(String, u64), PairError> {
        let mut st = self.state.lock().unwrap();
        if let Some(until) = st.locked_until {
            let left = until.saturating_duration_since(Instant::now());
            if !left.is_zero() {
                return Err(PairError::Locked {
                    retry_in_secs: left.as_secs() + 1,
                });
            }
            st.locked_until = None;
        }
        match &st.pairing {
            Some(p) if p.otp.expires_at > now_secs() => Ok((p.otp.otp.clone(), p.generation)),
            _ => Err(PairError::Closed),
        }
    }

    /// บันทึกการเดาผิดหนึ่งครั้ง (นับทั้งเครื่อง) — ครบ 5 ครั้งทำลาย OTP ปิดโหมดจับคู่ และล็อก 60 วินาที
    /// ต่อให้เปิดโหมดจับคู่ใหม่ระหว่างล็อกก็ยังเดาไม่ได้
    pub fn report_wrong_guess(&self, ip: IpAddr, client_name: &str) {
        let mut st = self.state.lock().unwrap();
        st.recent_failures.push(GuessFailure {
            at: now_secs(),
            ip: ip.to_string(),
            client_name: client_name.chars().take(64).collect(),
        });
        if st.recent_failures.len() > MAX_RECENT_FAILURES {
            st.recent_failures.remove(0);
        }
        st.wrong_guesses += 1;
        if st.wrong_guesses >= MAX_WRONG_GUESSES {
            st.wrong_guesses = 0;
            st.pairing = None;
            st.locked_until = Some(Instant::now() + LOCKOUT);
            tracing::warn!("เดา OTP ผิดครบ {MAX_WRONG_GUESSES} ครั้ง: ทำลาย OTP และล็อกการจับคู่ 60 วินาที");
        }
    }

    /// handshake สำเร็จ: ลงทะเบียนอุปกรณ์ แล้วใช้ OTP หมด (ปิดโหมดจับคู่)
    pub fn complete_pairing(&self, generation: u64, public_key: &str, client_name: &str) -> Result<Device, PairError> {
        let raw = B64URL
            .decode(public_key)
            .ok()
            .filter(|k| k.len() == 32)
            .ok_or(PairError::BadKey)?;
        let mut st = self.state.lock().unwrap();
        match &st.pairing {
            Some(p) if p.generation == generation && p.otp.expires_at > now_secs() => {}
            _ => return Err(PairError::Stale),
        }
        st.pairing = None;
        st.wrong_guesses = 0;
        let device = Device {
            id: device_id(&raw),
            client_name: client_name.chars().take(64).collect(),
            public_key: public_key.to_string(),
            created_at: now_secs(),
            last_seen: now_secs(),
        };
        st.devices.insert(device.id.clone(), device.clone());
        if self.devices_file.is_none() {
            st.portable_control = Some(PortableControl::Waiting {
                device: device.id.clone(),
                until: Instant::now() + PORTABLE_ATTACH_WINDOW,
            });
        }
        self.save(&st);
        Ok(device)
    }

    // ---------- อุปกรณ์ที่จับคู่แล้ว ----------

    /// id ของอุปกรณ์ที่ถือ public key นี้ (None = ไม่อยู่ใน allowlist)
    pub fn device_for_key(&self, public_key: &[u8; 32]) -> Option<String> {
        let id = device_id(public_key);
        let mut st = self.state.lock().unwrap();
        st.devices.get_mut(&id)?.last_seen = now_secs();
        Some(id)
    }

    pub fn is_active(&self, device_id: &str) -> bool {
        self.state.lock().unwrap().devices.contains_key(device_id)
    }

    pub fn list(&self) -> Vec<Device> {
        let mut v: Vec<Device> = self.state.lock().unwrap().devices.values().cloned().collect();
        v.sort_by_key(|d| d.created_at);
        v
    }

    /// ลบออกจาก allowlist และตัด connection ที่เปิดอยู่ของอุปกรณ์นั้นทันที
    pub fn revoke(&self, device_id: &str) -> bool {
        let mut st = self.state.lock().unwrap();
        let removed = st.devices.remove(device_id).is_some();
        if removed {
            if self.devices_file.is_none() {
                st.portable_control = None;
            }
            self.save(&st);
            let _ = self.kill.send(device_id.to_string());
        }
        removed
    }

    /// ตัดทุก connection ที่เปิดอยู่ (ไม่ลบอุปกรณ์ — ต่อใหม่ได้) สำหรับปุ่ม "หยุดทันที"
    pub fn kill_all(&self) {
        let _ = self.kill.send(KILL_ALL.to_string());
    }

    /// สถานะการเดา OTP สำหรับหน้าจัดการ: ผิดไปกี่ครั้งในรอบนี้, ถูกล็อกอีกกี่วินาที, ใครเดา (IP + ชื่อที่แจ้ง)
    pub fn pairing_report(&self) -> serde_json::Value {
        let st = self.state.lock().unwrap();
        let locked_for = st
            .locked_until
            .map(|u| u.saturating_duration_since(Instant::now()).as_secs())
            .filter(|s| *s > 0);
        serde_json::json!({"wrong_guesses": st.wrong_guesses, "max_wrong_guesses": MAX_WRONG_GUESSES,
                           "locked_for_secs": locked_for, "recent_failures": st.recent_failures})
    }

    pub fn subscribe_kills(&self) -> broadcast::Receiver<String> {
        self.kill.subscribe()
    }

    // เขียนไฟล์ชั่วคราวแล้ว rename เพื่อไม่ให้ไฟล์พังถ้าไฟดับกลางทาง
    fn save(&self, st: &State) {
        let Some(devices_file) = &self.devices_file else { return };
        let devices: Vec<&Device> = st.devices.values().collect();
        let tmp = devices_file.with_extension("json.tmp");
        let result = serde_json::to_vec_pretty(&devices)
            .map_err(std::io::Error::other)
            .and_then(|b| std::fs::write(&tmp, b))
            .and_then(|_| std::fs::rename(&tmp, devices_file));
        if let Err(e) = result {
            // ไม่ทำให้ pair/revoke ล้ม — แต่ต้องรู้ว่ารายการบนดิสก์ไม่ตรงกับในหน่วยความจำ
            tracing::error!("บันทึก paired_devices.json ไม่สำเร็จ: {e}");
        }
    }
}

/// ชื่อ agent แบบสั้นที่ใช้ประกาศ: `sc-node-<12 hex>` (สุ่มครั้งเดียวต่อเครื่อง เก็บใน data_dir/device_id)
pub fn load_or_create_node_id(data_dir: &std::path::Path) -> std::io::Result<String> {
    let file = data_dir.join("device_id");
    match std::fs::read_to_string(&file) {
        Ok(s) if !s.trim().is_empty() => Ok(s.trim().to_string()),
        _ => {
            let id = format!("sc-node-{}", random_id(6));
            std::fs::create_dir_all(data_dir)?;
            std::fs::write(&file, &id)?;
            Ok(id)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn auth() -> Auth {
        let dir = std::env::temp_dir().join(format!("synapta-auth-{}", random_id(4)));
        Auth::load(&dir).unwrap()
    }

    #[test]
    fn portable_pairing_is_usable_only_for_one_live_controller() {
        let auth = Auth::ephemeral();
        let first = auth.portable_otp().unwrap();
        let (_, generation) = auth.begin_pairing().unwrap();
        let device = auth
            .complete_pairing(generation, &B64URL.encode([5_u8; 32]), "Client")
            .unwrap();
        assert!(auth.portable_otp().is_none());
        assert!(!auth.portable_attached(&device.id));
        assert!(!auth.attach_portable("another-device"));
        assert!(auth.attach_portable(&device.id));
        assert!(auth.portable_attached(&device.id));
        assert!(!auth.attach_portable(&device.id));
        assert!(auth.revoke(&device.id));
        assert!(!auth.is_active(&device.id));
        assert!(!auth.portable_attached(&device.id));
        assert!(auth.portable_otp().is_some());
        assert_ne!(auth.begin_pairing().unwrap().1, generation);
        assert!(!first.otp.is_empty());
    }

    fn key() -> String {
        B64URL.encode([7u8; 32])
    }

    #[test]
    fn pairing_is_closed_until_opened_and_otp_is_single_use() {
        let a = auth();
        assert_eq!(a.begin_pairing().unwrap_err(), PairError::Closed);
        a.open_pairing();
        let (_, generation) = a.begin_pairing().unwrap();
        a.complete_pairing(generation, &key(), "t").unwrap();
        assert_eq!(
            a.begin_pairing().unwrap_err(),
            PairError::Closed,
            "ใช้ OTP แล้วต้องปิดโหมดจับคู่"
        );
        assert!(a.is_active(&device_id(&[7u8; 32])));
    }

    #[test]
    fn five_wrong_guesses_destroy_otp_and_lock_even_after_reopening() {
        let a = auth();
        a.open_pairing();
        for _ in 0..5 {
            a.report_wrong_guess("10.0.0.9".parse().unwrap(), "x");
        }
        assert!(a.current_otp().is_none());
        a.open_pairing();
        assert!(matches!(a.begin_pairing(), Err(PairError::Locked { .. })));
    }

    #[test]
    fn handshake_from_an_older_otp_is_stale() {
        let a = auth();
        a.open_pairing();
        let (_, old) = a.begin_pairing().unwrap();
        a.open_pairing();
        assert_eq!(a.complete_pairing(old, &key(), "t").unwrap_err(), PairError::Stale);
    }

    #[test]
    fn revoke_removes_device_and_notifies() {
        let a = auth();
        a.open_pairing();
        let (_, generation) = a.begin_pairing().unwrap();
        let device = a.complete_pairing(generation, &key(), "t").unwrap();
        let mut kills = a.subscribe_kills();
        assert!(a.revoke(&device.id));
        assert!(!a.is_active(&device.id));
        assert_eq!(kills.try_recv().unwrap(), device.id);
    }
}
