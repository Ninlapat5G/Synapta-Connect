//! การจับคู่ด้วย SPAKE2 — OTP เป็นรหัสผ่านร่วม ไม่เคยวิ่งบนสาย
//!
//! POST /api/v1/auth/spake2-handshake (สองขั้น)
//!
//! ขั้น 1: client ส่ง `{step:1, client_msg, client_pubkey, client_name}` → agent ตอบ `{handshake_id, server_msg}`
//! ทั้งสองฝั่งได้กุญแจร่วม K ก็ต่อเมื่อใช้ OTP ตรงกัน (ตัวกลางดักอ่านข้อความทั้งสองก็เดา OTP ออฟไลน์ไม่ได้)
//!
//! ขั้น 2: client ส่ง `{step:2, handshake_id, client_confirm}` โดย
//! `client_confirm = HMAC-SHA256(K, "synapta-connect client" ‖ host_pubkey ‖ client_pubkey)`
//! agent ตรวจแล้วตอบ `{device_id, host_pubkey, host_pubkey_fingerprint, host_confirm}` โดย
//! `host_confirm = HMAC-SHA256(K, "synapta-connect host" ‖ host_pubkey ‖ client_pubkey)`
//!
//! host_pubkey ในสูตรคือ key ที่แต่ละฝั่งเห็นจริงบน TLS — ถ้ามีตัวกลางสวม TLS key ไม่ตรงกัน confirm จะไม่ผ่าน
//! จึงผูกการจับคู่เข้ากับ host key ที่ client จะปักหมุดจริง ๆ
//!
//! การเดา OTP หนึ่งครั้ง = หนึ่งครั้งที่ขั้น 2 ไม่ผ่าน (นับใน auth.rs: ครบ 5 ครั้งทำลาย OTP)

use crate::{app::App, auth::PairError, hex::random_id};
use axum::{
    Json,
    extract::{ConnectInfo, State},
    http::StatusCode,
    response::{IntoResponse, Response},
};
use base64::{
    Engine,
    engine::general_purpose::{STANDARD as B64, URL_SAFE_NO_PAD as B64URL},
};
use hmac::{Hmac, KeyInit, Mac};
use serde::Deserialize;
use serde_json::{Value, json};
use sha2::Sha256;
use spake2::{Ed25519Group, Identity, Password, Spake2};
use std::{
    collections::HashMap,
    net::{IpAddr, SocketAddr},
    sync::Mutex,
    time::{Duration, Instant},
};

const HANDSHAKE_TTL: Duration = Duration::from_secs(30);
/// handshake ที่ค้างพร้อมกันสูงสุด (คนเริ่มแล้วไม่จบไม่กินหน่วยความจำไม่จำกัด)
const MAX_PENDING: usize = 8;
/// ต่อ IP ต้นทาง — กันคนเดียวยิงขั้น 1 รัว ๆ จนดันการจับคู่ของเจ้าของออก (เต็มแล้ว 'ปฏิเสธตัวใหม่' ไม่ใช่ 'ทิ้งตัวเก่า')
const MAX_PENDING_PER_IP: usize = 2;
const CLIENT_ID: &[u8] = b"synapta-connect client";
const HOST_ID: &[u8] = b"synapta-connect host";

struct Pending {
    ip: IpAddr,
    key: Vec<u8>,
    generation: u64,
    client_pubkey: String,
    client_name: String,
    started: Instant,
}

#[derive(Default)]
pub struct Handshakes(Mutex<HashMap<String, Pending>>);

#[derive(Deserialize)]
pub struct Body {
    step: u8,
    client_msg: Option<String>,
    client_pubkey: Option<String>,
    client_name: Option<String>,
    handshake_id: Option<String>,
    client_confirm: Option<String>,
}

fn fail(status: StatusCode, code: &str, message: &str) -> Response {
    (status, Json(json!({"error": {"code": code, "message": message}}))).into_response()
}

fn pair_error(e: PairError) -> Response {
    match e {
        PairError::Closed => fail(StatusCode::FORBIDDEN, "pairing_closed", "โหมดจับคู่ปิดอยู่ (ให้เจ้าของเครื่องเปิด Pairing Mode)"),
        PairError::Locked { retry_in_secs } => (
            StatusCode::TOO_MANY_REQUESTS,
            Json(json!({"error": {"code": "locked", "message": "เดา OTP ผิดหลายครั้ง ถูกล็อกชั่วคราว"}, "retry_in_secs": retry_in_secs})),
        )
            .into_response(),
        PairError::Stale => fail(StatusCode::CONFLICT, "pairing_changed", "OTP เปลี่ยนระหว่างทาง ให้เริ่มจับคู่ใหม่"),
        PairError::BadKey => fail(StatusCode::BAD_REQUEST, "bad_key", "client_pubkey ต้องเป็น Ed25519 32 ไบต์แบบ base64url"),
    }
}

/// HMAC ยืนยัน: label ‖ host_pubkey ‖ client_pubkey
fn confirmation(key: &[u8], label: &[u8], host_pubkey: &[u8], client_pubkey: &[u8]) -> Hmac<Sha256> {
    let mut mac = Hmac::<Sha256>::new_from_slice(key).expect("HMAC รับกุญแจความยาวใดก็ได้");
    mac.update(label);
    mac.update(host_pubkey);
    mac.update(client_pubkey);
    mac
}

pub async fn handshake(
    State(app): State<App>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    Json(body): Json<Body>,
) -> Response {
    match body.step {
        1 => step1(&app, peer.ip(), body),
        2 => step2(&app, peer.ip(), body),
        _ => fail(StatusCode::BAD_REQUEST, "bad_step", "step ต้องเป็น 1 หรือ 2"),
    }
}

/// Some(คำตอบ) ถ้าเริ่ม handshake ใหม่ไม่ได้: IP นี้ค้างอยู่แล้ว 2 อัน หรือทั้งเครื่องเต็ม 8 อัน
/// ปฏิเสธ "ตัวใหม่" ไม่ทิ้ง "ตัวเก่า" — ไม่งั้นคนยิงรัวดันการจับคู่ของเจ้าของที่กำลังทำอยู่ทิ้งได้
fn busy_response(pending: &mut HashMap<String, Pending>, ip: IpAddr) -> Option<Response> {
    pending.retain(|_, p| p.started.elapsed() < HANDSHAKE_TTL);
    let from_ip = pending.values().filter(|p| p.ip == ip).count();
    (from_ip >= MAX_PENDING_PER_IP || pending.len() >= MAX_PENDING).then(|| {
        fail(
            StatusCode::TOO_MANY_REQUESTS,
            "busy",
            "มี handshake ค้างอยู่มากเกินไป รอสักครู่แล้วลองใหม่",
        )
    })
}

fn step1(app: &App, ip: IpAddr, body: Body) -> Response {
    let (Some(client_msg), Some(client_pubkey), Some(client_name)) =
        (body.client_msg, body.client_pubkey, body.client_name)
    else {
        return fail(
            StatusCode::BAD_REQUEST,
            "missing_field",
            "step 1 ต้องมี client_msg, client_pubkey, client_name",
        );
    };
    let valid_key = B64URL.decode(&client_pubkey).is_ok_and(|k| k.len() == 32);
    let Some(client_msg) = B64.decode(client_msg).ok().filter(|_| valid_key) else {
        return pair_error(PairError::BadKey);
    };
    // ตรวจเพดานก่อนเริ่มคำนวณ SPAKE (กิน CPU) — คนยิงรัวต้องไม่ทำให้เครื่องช้า
    if let Some(busy) = busy_response(&mut app.handshakes.0.lock().unwrap(), ip) {
        return busy;
    }
    let (otp, generation) = match app.auth.begin_pairing() {
        Ok(v) => v,
        Err(e) => return pair_error(e),
    };
    let (state, server_msg) = Spake2::<Ed25519Group>::start_b(
        &Password::new(otp.as_bytes()),
        &Identity::new(CLIENT_ID),
        &Identity::new(HOST_ID),
    );
    let Ok(key) = state.finish(&client_msg) else {
        return fail(
            StatusCode::BAD_REQUEST,
            "bad_client_msg",
            "client_msg ไม่ใช่ข้อความ SPAKE2 ที่ถูกต้อง",
        );
    };
    let id = random_id(16);
    let mut pending = app.handshakes.0.lock().unwrap();
    if let Some(busy) = busy_response(&mut pending, ip) {
        return busy;
    }
    pending.insert(
        id.clone(),
        Pending {
            ip,
            key,
            generation,
            client_pubkey,
            client_name,
            started: Instant::now(),
        },
    );
    Json(json!({"handshake_id": id, "server_msg": B64.encode(server_msg)})).into_response()
}

fn step2(app: &App, ip: IpAddr, body: Body) -> Response {
    let (Some(id), Some(confirm)) = (body.handshake_id, body.client_confirm) else {
        return fail(
            StatusCode::BAD_REQUEST,
            "missing_field",
            "step 2 ต้องมี handshake_id และ client_confirm",
        );
    };
    // ใช้ได้ครั้งเดียวเสมอ ไม่ว่าผลจะเป็นอย่างไร
    let pending = app.handshakes.0.lock().unwrap().remove(&id);
    let Some(p) = pending.filter(|p| p.started.elapsed() < HANDSHAKE_TTL) else {
        return fail(
            StatusCode::NOT_FOUND,
            "unknown_handshake",
            "ไม่พบ handshake นี้หรือหมดเวลา (30 วินาที) ให้เริ่มใหม่",
        );
    };
    let client_raw = B64URL.decode(&p.client_pubkey).unwrap_or_default();
    let supplied = B64.decode(confirm).unwrap_or_default();
    let expected = confirmation(&p.key, CLIENT_ID, &app.host.public_key, &client_raw);
    // verify_slice เทียบแบบ constant-time
    if expected.verify_slice(&supplied).is_err() {
        tracing::warn!(client = %p.client_name, "จับคู่ล้มเหลว: OTP ไม่ตรงหรือ TLS host key ที่ client เห็นไม่ตรง");
        app.auth.report_wrong_guess(ip, &p.client_name);
        return fail(StatusCode::FORBIDDEN, "invalid_otp", "OTP ไม่ถูกต้อง");
    }
    let device = match app
        .auth
        .complete_pairing(p.generation, &p.client_pubkey, &p.client_name)
    {
        Ok(d) => d,
        Err(e) => return pair_error(e),
    };
    tracing::info!(device = %device.id, client = %device.client_name, "จับคู่สำเร็จ");
    let host_confirm = confirmation(&p.key, HOST_ID, &app.host.public_key, &client_raw)
        .finalize()
        .into_bytes();
    let reply: Value = json!({
        "device_id": device.id,
        "host_pubkey": app.host.public_key_b64(),
        "host_pubkey_fingerprint": app.host.fingerprint(),
        "host_confirm": B64.encode(host_confirm),
    });
    Json(reply).into_response()
}
