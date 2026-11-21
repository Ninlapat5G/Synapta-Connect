//! หน้าจัดการบนเครื่องนี้ (`http://127.0.0.1:52402`) — สำหรับเจ้าของเครื่อง/panel: โหมดจับคู่ OTP, revoke, หยุดทันที,
//! สถานะ service, อัปเดต, สวิตช์จับภาพ/input และตัวรับ helper — สัญญาอยู่ที่ docs/API.md §6 และ docs/PANEL_API.md
//! เสิร์ฟบน loopback เป็น HTTP ธรรมดา ไม่ออกเครือข่าย (จึงไม่ต้องใช้ใบรับรองที่เบราว์เซอร์จะเตือน)

use crate::{
    app::App,
    netif, panel, profile, proof,
    update::{Refused, source::Source},
};
use axum::{
    Json, Router,
    extract::{ConnectInfo, Path, Query, Request, State, ws::WebSocketUpgrade},
    http::{HeaderMap, StatusCode, header},
    middleware::{self, Next},
    response::{IntoResponse, Response},
    routing::{get, post, put},
};
use serde::Deserialize;
use serde_json::{Value, json};
use std::{collections::HashMap, net::SocketAddr};

pub fn router(app: App) -> Router {
    // ต้องมี token (local_token ของ CLI/ helper หรือ token ของ panel)
    let owner = Router::new()
        .route("/api/v1/local/status", get(status))
        .route("/api/v1/local/pairing/open", post(open_pairing))
        .route("/api/v1/local/pairing/close", post(close_pairing))
        .route("/api/v1/local/devices/{id}/revoke", post(revoke))
        .route("/api/v1/local/kill-all", post(kill_all))
        .route("/api/v1/local/update", get(update_status))
        .route("/api/v1/local/update/history", get(update_history))
        .route("/api/v1/local/update/check", post(update_check))
        .route("/api/v1/local/update/apply", post(update_apply))
        .route("/api/v1/local/update/retry", post(update_retry))
        .route("/api/v1/local/update/settings", put(update_settings))
        .route("/api/v1/local/screen-input", get(screen_input).put(set_screen_input))
        .route("/api/v1/local/panel-session", post(panel_session))
        .layer(middleware::from_fn_with_state(app.clone(), token_guard));

    // ไม่ต้องมี token แต่ผ่านด่าน loopback เหมือนกัน: หน้าเว็บ panel (ไฟล์ static) + แลกรหัส + health
    Router::new()
        .merge(owner)
        .route("/api/v1/local/health", get(health))
        // helper พิสูจน์ตัวด้วย HMAC (ไม่ส่ง token ออกสาย) จึงไม่ผ่าน token_guard — ดู helper_socket
        .route("/api/v1/local/helper", get(helper_socket))
        .route("/api/v1/panel/exchange", post(panel_exchange))
        .route("/panel", get(panel::index))
        .route("/panel/", get(panel::index))
        .route("/panel/{*path}", get(panel::file))
        .layer(middleware::from_fn(loopback_guard))
        .with_state(app)
}

// ---------- ด่าน ----------

/// ด่านชั้นแรกของทุก route: (1) peer ต้องเป็น loopback (2) ไม่มี header ของ proxy — Tailscale serve / reverse proxy
/// ต่อเข้ามาจาก 127.0.0.1 เหมือนกัน (3) Host ต้องเป็นชื่อ loopback กันเว็บที่ทำ DNS rebinding
/// (4) ถ้ามี Origin ต้องเป็น origin เดียวกับ Host (panel ของเราเอง) — เว็บอื่นยิงข้ามมาไม่ได้
async fn loopback_guard(ConnectInfo(peer): ConnectInfo<SocketAddr>, req: Request, next: Next) -> Response {
    let h: &HeaderMap = req.headers();
    let proxied = ["x-forwarded-for", "forwarded", "x-real-ip", "via"]
        .iter()
        .any(|k| h.contains_key(*k));
    let host = h.get(header::HOST).and_then(|v| v.to_str().ok()).unwrap_or("");
    let host_name = host.rsplit_once(':').map_or(host, |(name, _port)| name);
    let host_ok = matches!(host_name, "127.0.0.1" | "localhost" | "[::1]");
    let origin_ok = h
        .get(header::ORIGIN)
        .is_none_or(|o| o.to_str().ok() == Some(&format!("http://{host}")));
    if !peer.ip().is_loopback() || proxied || !host_ok || !origin_ok {
        return StatusCode::FORBIDDEN.into_response();
    }
    next.run(req).await
}

/// เทียบแบบไม่หยุดเมื่อเจอไบต์แรกที่ต่าง
fn same_secret(a: &[u8], b: &[u8]) -> bool {
    a.len() == b.len() && a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

/// token ที่รับได้: local_token จากไฟล์ (เจ้าของ/Administrators) หรือ token ที่ panel แลกมา
fn token_ok(app: &App, headers: &HeaderMap) -> bool {
    let supplied = headers.get("x-synapta-local").map_or(&b""[..], |v| v.as_bytes());
    same_secret(supplied, app.local_token.as_bytes())
        || std::str::from_utf8(supplied).is_ok_and(|t| app.panel.is_valid(t))
}

/// endpoint พวกนี้เปิดเผย OTP และสั่ง revoke จึงต้องมาจากเจ้าของเครื่องจริง ๆ — token กัน process ของ "ผู้ใช้อื่นบนเครื่องเดียวกัน"
/// (loopback เปิดให้ทุก process ต่อได้; ไม่มี token ผู้ใช้ธรรมดาจะอ่าน OTP ไปจับคู่แล้วได้ exec ด้วยสิทธิ์ของ agent)
async fn token_guard(State(app): State<App>, req: Request, next: Next) -> Response {
    if !token_ok(&app, req.headers()) {
        return StatusCode::FORBIDDEN.into_response();
    }
    next.run(req).await
}

fn error(status: StatusCode, code: &str, message: &str) -> Response {
    (status, Json(json!({"error": {"code": code, "message": message}}))).into_response()
}

// ---------- สถานะและโหมดจับคู่ (เดิม) ----------

async fn status(State(app): State<App>) -> Json<Value> {
    // otp เป็น null เมื่อปิดโหมดจับคู่ — UI ต้องเรียก pairing/open เพื่อเปิด
    let otp = app.auth.current_otp();
    let privilege = tokio::task::spawn_blocking(profile::privilege)
        .await
        .ok()
        .and_then(Result::ok);
    Json(json!({
        "device_id": app.node_id, "name": app.config.name,
        "host_pubkey_fingerprint": app.host.fingerprint(),
        "pairing_mode": otp.is_some(),
        "otp": otp.as_ref().map(|o| &o.otp), "otp_expires_at": otp.as_ref().map(|o| o.expires_at),
        "devices": app.auth.list(), "addresses": netif::list(),
        "pairing": app.auth.pairing_report(),
        "agent": {
            "version": env!("CARGO_PKG_VERSION"),
            "mode": if app.config.mode == crate::config::Mode::Service { "service" } else { "console" },
            "account": crate::platform::account(),
            "privilege": privilege.as_ref().map(|p| p.level),
            "pid": std::process::id(), "uptime_secs": app.started.elapsed().as_secs(),
        },
        "screen_input": screen_input_json(&app),
    }))
}

/// เปิดโหมดจับคู่ (หรือออก OTP ใหม่ถ้าเปิดอยู่แล้ว)
async fn open_pairing(State(app): State<App>) -> Json<Value> {
    tracing::info!("เจ้าของเครื่องเปิดโหมดจับคู่");
    Json(json!(app.auth.open_pairing()))
}

async fn close_pairing(State(app): State<App>) -> Json<Value> {
    app.auth.close_pairing();
    Json(json!({"pairing_mode": false}))
}

async fn revoke(State(app): State<App>, Path(id): Path<String>) -> Json<Value> {
    tracing::info!(target = %id, "device.revoke (local)");
    Json(json!({"revoked": app.auth.revoke(&id)}))
}

/// "หยุดทันที": ตัดทุก connection ที่เปิดอยู่ (อุปกรณ์ยังจับคู่อยู่ ต่อใหม่ได้)
async fn kill_all(State(app): State<App>) -> Json<Value> {
    tracing::warn!("เจ้าของเครื่องสั่งตัดทุก connection");
    app.auth.kill_all();
    Json(json!({"ok": true}))
}

/// ไม่ต้องมี token (ตัวติดตั้ง/CLI ถามได้) แต่ถ้าแนบ `?nonce=<hex>` จะมี `proof` = HMAC(local_token, nonce|รุ่น|pid)
/// ให้ผู้ถามที่ถือ token ตรวจได้ว่าคนตอบคือ agent จริง ไม่ใช่โปรเซสอื่นที่ยึดพอร์ตไว้ (ดู proof.rs)
async fn health(State(app): State<App>, Query(q): Query<HashMap<String, String>>) -> Json<Value> {
    let (version, pid) = (env!("CARGO_PKG_VERSION"), std::process::id());
    let proof = q
        .get("nonce")
        .filter(|n| n.len() <= 64 && n.bytes().all(|b| b.is_ascii_hexdigit()))
        .map(|n| proof::make(&app.local_token, proof::HEALTH, &proof::health_message(n, version, pid)));
    Json(json!({"ok": true, "version": version, "pid": pid,
                "uptime_secs": app.started.elapsed().as_secs(), "proof": proof}))
}

// ---------- panel ----------

async fn panel_session(State(app): State<App>) -> Json<Value> {
    Json(json!({"code": app.panel.issue_code(), "expires_in": 60}))
}

#[derive(Deserialize)]
struct Exchange {
    code: String,
}

async fn panel_exchange(State(app): State<App>, Json(body): Json<Exchange>) -> Response {
    match app.panel.exchange(&body.code) {
        Some((token, expires_at)) => Json(json!({"token": token, "expires_at": expires_at})).into_response(),
        None => StatusCode::FORBIDDEN.into_response(),
    }
}

// ---------- อัปเดต ----------

async fn update_status(State(app): State<App>) -> Json<Value> {
    Json(app.updates.status())
}

async fn update_history(State(app): State<App>, Query(q): Query<HashMap<String, String>>) -> Json<Value> {
    let limit = q.get("limit").and_then(|v| v.parse().ok()).unwrap_or(100);
    Json(json!(app.updates.history(limit)))
}

fn refused(r: Refused) -> Response {
    match r {
        Refused::Busy => error(StatusCode::CONFLICT, "busy", "กำลังทำงานอัปเดตอื่นอยู่ — รอให้เสร็จก่อน"),
        Refused::RateLimited => error(StatusCode::TOO_MANY_REQUESTS, "rate_limited", "ตรวจถี่เกินไป — รอสักครู่"),
        Refused::Invalid(m) => error(StatusCode::BAD_REQUEST, "invalid_request", &m),
    }
}

/// งานยาว (เครือข่าย/ดาวน์โหลด) ทำเบื้องหลัง — ตอบ 202 พร้อมสถานะตอนนี้ให้ panel poll ต่อ
fn accepted(app: &App) -> Response {
    (StatusCode::ACCEPTED, Json(app.updates.status())).into_response()
}

async fn update_check(State(app): State<App>) -> Response {
    let job = match app.updates.begin_check(true) {
        Ok(job) => job,
        Err(r) => return refused(r),
    };
    let mgr = app.updates.clone();
    tokio::spawn(async move { mgr.run_check(job).await });
    accepted(&app)
}

#[derive(Deserialize)]
struct ApplyBody {
    version: String,
}

async fn update_apply(State(app): State<App>, Json(body): Json<ApplyBody>) -> Response {
    start_apply(&app, &body.version)
}

/// ลองใหม่ = ติดตั้งรุ่นที่ตรวจพบอยู่ (เฉพาะเมื่อมีความล้มเหลวให้ลองใหม่) พร้อมเริ่มนับครั้งใหม่
async fn update_retry(State(app): State<App>) -> Response {
    let status = app.updates.status();
    if status["can_retry"] != json!(true) {
        return error(StatusCode::BAD_REQUEST, "invalid_request", "ไม่มีงานอัปเดตที่ล้มเหลวให้ลองใหม่");
    }
    let version = status["available"]["version"].as_str().unwrap_or_default().to_string();
    start_apply(&app, &version)
}

fn start_apply(app: &App, version: &str) -> Response {
    tracing::info!(version, "เจ้าของสั่งติดตั้งอัปเดต");
    let job = match app.updates.begin_apply(version, true) {
        Ok(job) => job,
        Err(r) => return refused(r),
    };
    let mgr = app.updates.clone();
    tokio::spawn(async move { mgr.run_apply(job).await });
    accepted(app)
}

#[derive(Deserialize)]
struct UpdateSettingsBody {
    auto_update: bool,
    source: Option<String>,
}

async fn update_settings(State(app): State<App>, Json(body): Json<UpdateSettingsBody>) -> Response {
    // "" = กลับไปใช้แหล่งที่ฝังตอน build; แหล่งแบบโฟลเดอร์ในเครื่อง (dir:) ตั้งผ่าน panel ไม่ได้ (ใช้ทดสอบ ผ่าน settings.json ของผู้ดูแล)
    if let Some(src) = body.source.as_deref().filter(|s| !s.trim().is_empty()) {
        match Source::parse(src) {
            Ok(Source::Dir(_)) | Err(_) => {
                return error(
                    StatusCode::BAD_REQUEST,
                    "bad_source",
                    "รับเฉพาะ github:OWNER/REPO หรือ https://…/",
                );
            }
            Ok(_) => {}
        }
    }
    tracing::info!(auto_update = body.auto_update, "เจ้าของเปลี่ยนค่าตั้งอัปเดต");
    match app
        .updates
        .set_settings(body.auto_update, body.source.map(|s| s.trim().to_string()))
    {
        Ok(()) => Json(app.updates.status()).into_response(),
        Err(e) => error(
            StatusCode::INTERNAL_SERVER_ERROR,
            "save_failed",
            &format!("บันทึกค่าตั้งไม่ได้: {e}"),
        ),
    }
}

// ---------- จับภาพ/ส่ง input ----------

fn screen_input_json(app: &App) -> Value {
    let helper = app.display.helper_info();
    json!({
        "enabled": app.settings.get().screen_input.enabled,
        "helper_connected": helper.is_some(),
        "helper_user": helper.as_ref().map(|h| &h.user),
        "helper_session": helper.as_ref().map(|h| h.session),
    })
}

async fn screen_input(State(app): State<App>) -> Json<Value> {
    Json(screen_input_json(&app))
}

#[derive(Deserialize)]
struct EnabledBody {
    enabled: bool,
}

async fn set_screen_input(State(app): State<App>, Json(body): Json<EnabledBody>) -> Response {
    tracing::warn!(enabled = body.enabled, "เจ้าของเปลี่ยนสิทธิ์จับภาพ/ส่ง input");
    match app.settings.update(|s| s.screen_input.enabled = body.enabled) {
        Ok(_) => Json(screen_input_json(&app)).into_response(),
        Err(e) => error(
            StatusCode::INTERNAL_SERVER_ERROR,
            "save_failed",
            &format!("บันทึกค่าตั้งไม่ได้: {e}"),
        ),
    }
}

/// helper ต่อเข้ามา: ตัวตนผูกกับการรู้ local_token (ไฟล์ที่เฉพาะเจ้าของอ่านได้) แต่ **ไม่ส่ง token ทางสาย** —
/// helper ส่ง nonce + HMAC(token, "helper-hello", nonce) แล้ว agent ตอบ HMAC(token, "helper-server", nonce) กลับในหัวข้อ
/// ของการอัปเกรด เพื่อให้ helper ตรวจว่าคนฟังคือ agent จริง (กันโปรเซสอื่นที่ยึดพอร์ตตอน service ไม่ทำงาน
/// หลอกให้ helper ส่งภาพหน้าจอ/ทำคำสั่งคลิก-พิมพ์ให้) · nonce ใช้ได้ครั้งเดียว (กัน replay ที่ดักเห็นจาก loopback)
async fn helper_socket(State(app): State<App>, headers: HeaderMap, ws: WebSocketUpgrade) -> Response {
    let text = |name: &str| headers.get(name).and_then(|v| v.to_str().ok()).unwrap_or("");
    let (nonce, given) = (text("x-synapta-nonce"), text("x-synapta-proof"));
    let nonce_ok = (16..=64).contains(&nonce.len()) && nonce.bytes().all(|b| b.is_ascii_hexdigit());
    if !nonce_ok || !proof::check(&app.local_token, proof::HELPER, nonce, given) || !app.display.accept_nonce(nonce) {
        return StatusCode::FORBIDDEN.into_response();
    }
    let server_proof = proof::make(&app.local_token, proof::SERVER, nonce);
    let mut response = ws.on_upgrade(move |socket| async move { app.display.serve_helper(socket).await });
    if let Ok(value) = server_proof.parse() {
        response.headers_mut().insert("x-synapta-server-proof", value);
    }
    response
}
