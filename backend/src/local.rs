//! หน้าจัดการบนเครื่องนี้ (`http://127.0.0.1:52402`) — สำหรับเจ้าของเครื่อง/panel: โหมดจับคู่ OTP, revoke, หยุดทันที,
//! สถานะ service, อัปเดต, สวิตช์จับภาพ/input และตัวรับ helper — สัญญาอยู่ที่ docs/API.md §6 และ docs/PANEL_API.md
//! เสิร์ฟบน loopback เป็น HTTP ธรรมดา ไม่ออกเครือข่าย (จึงไม่ต้องใช้ใบรับรองที่เบราว์เซอร์จะเตือน)

use crate::{
    app::App,
    netif,
    network::NetworkRequest,
    panel::{self, Scope},
    panel_https::ListenerPort,
    profile, proof,
    update::{Refused, source::Source, web::Action},
};
use axum::{
    Extension, Json, Router,
    extract::{ConnectInfo, Path, Query, Request, State, ws::WebSocketUpgrade},
    http::{HeaderMap, HeaderValue, StatusCode, header, uri::Authority},
    middleware::{self, Next},
    response::{IntoResponse, Response},
    routing::{get, post, put},
};
use serde::Deserialize;
use serde_json::{Value, json};
use std::{collections::HashMap, net::SocketAddr};

pub fn router(app: App) -> Router {
    Router::new()
        .merge(management(app.clone()))
        .merge(panel_routes(app.clone()))
        .route("/api/v1/local/health", get(health))
        .route("/api/v1/local/helper", get(helper_socket))
        .merge(
            Router::new()
                .route("/api/v1/local/panel-session", post(panel_session))
                .route("/api/v1/local/enable-panel", post(enable_panel))
                .route("/api/v1/local/recover-web", post(recover_web))
                .route_layer(middleware::from_fn_with_state(app.clone(), native_guard)),
        )
        .layer(Extension(Scope::Local))
        .layer(middleware::from_fn_with_state(app.clone(), loopback_guard))
        .layer(middleware::from_fn(private_response))
        .with_state(app)
}

pub fn lan_router(app: App) -> Router {
    management(app.clone())
        .merge(panel_routes(app.clone()))
        .layer(middleware::from_fn_with_state(app.clone(), lan_guard))
        .layer(middleware::from_fn(private_response))
        .with_state(app)
}

fn management(app: App) -> Router<App> {
    Router::new()
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
        .route("/api/v1/local/web-update", get(web_status))
        .route("/api/v1/local/web-update/check", post(web_check))
        .route("/api/v1/local/web-update/apply", post(web_apply))
        .route("/api/v1/local/web-update/retry", post(web_retry))
        .route("/api/v1/local/web-update/rollback", post(web_rollback))
        .route("/api/v1/local/screen-input", get(screen_input).put(set_screen_input))
        .route("/api/v1/local/commands", get(commands).put(set_commands))
        .route("/api/v1/local/network", get(network_status).put(set_network))
        .route("/api/v1/local/panel-reload", post(panel_reload))
        .layer(middleware::from_fn_with_state(app, token_guard))
}

#[cfg(windows)]
pub(crate) fn desktop_router(app: App) -> Router {
    management(app.clone())
        .route("/api/v1/local/health", get(health))
        .route("/api/v1/local/recover-web", post(recover_web))
        .route("/api/v1/local/restart-web", post(restart_web))
        .layer(Extension(Scope::Local))
        .with_state(app)
}

#[cfg(windows)]
async fn restart_web(State(app): State<App>) -> Response {
    match app.network.restart_panel().await {
        Ok(()) => Json(network_json(&app)).into_response(),
        Err(message) => error(StatusCode::SERVICE_UNAVAILABLE, "web_restart_failed", &message),
    }
}

fn panel_routes(app: App) -> Router<App> {
    Router::new()
        .route("/api/v1/panel/exchange", post(panel_exchange))
        .route("/panel", get(panel::index))
        .route("/panel/", get(panel::index))
        .route("/synaptaconnect", get(panel::index))
        .route(panel::PATH, get(panel::index))
        .route("/synaptaconnect/{*path}", get(panel::file))
        .layer(middleware::from_fn_with_state(app, channel_guard))
}

async fn private_response(req: Request, next: Next) -> Response {
    let mut response = next.run(req).await;
    response
        .headers_mut()
        .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    response
        .headers_mut()
        .insert(header::REFERRER_POLICY, HeaderValue::from_static("no-referrer"));
    response
}

// ---------- ด่าน ----------

/// ด่านชั้นแรกของทุก route: (1) peer ต้องเป็น loopback (2) ไม่มี header ของ proxy — Tailscale serve / reverse proxy
/// ต่อเข้ามาจาก 127.0.0.1 เหมือนกัน (3) Host ต้องเป็นชื่อ loopback กันเว็บที่ทำ DNS rebinding
/// (4) ถ้ามี Origin ต้องเป็น origin เดียวกับ Host (panel ของเราเอง) — เว็บอื่นยิงข้ามมาไม่ได้
async fn loopback_guard(
    State(app): State<App>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    req: Request,
    next: Next,
) -> Response {
    let h: &HeaderMap = req.headers();
    let port = req.extensions().get::<ListenerPort>().map(|port| port.0);
    let ok =
        request_host(h, "http", port).is_some_and(|host| matches!(host.host(), "127.0.0.1" | "localhost" | "[::1]"));
    if !peer.ip().is_loopback() || !ok || port.is_some_and(|port| port != app.network.state().ports.local_port) {
        return StatusCode::FORBIDDEN.into_response();
    }
    next.run(req).await
}

fn request_host(headers: &HeaderMap, scheme: &str, local_port: Option<u16>) -> Option<Authority> {
    if [
        "x-forwarded-for",
        "x-forwarded-host",
        "x-forwarded-proto",
        "x-forwarded-port",
        "forwarded",
        "x-real-ip",
        "via",
    ]
    .iter()
    .any(|key| headers.contains_key(*key))
        || headers.get_all(header::HOST).iter().count() != 1
        || headers.get_all(header::ORIGIN).iter().count() > 1
    {
        return None;
    }
    let host = headers.get(header::HOST)?.to_str().ok()?;
    let authority: Authority = host.parse().ok()?;
    let default_port = if scheme == "https" { 443 } else { 80 };
    if authority.as_str().contains('@')
        || local_port.is_some_and(|p| p != authority.port_u16().unwrap_or(default_port))
        || headers
            .get(header::ORIGIN)
            .is_some_and(|origin| origin.to_str().ok() != Some(&format!("{scheme}://{host}")))
    {
        return None;
    }
    Some(authority)
}

async fn lan_guard(
    State(app): State<App>,
    Extension(ListenerPort(port)): Extension<ListenerPort>,
    mut req: Request,
    next: Next,
) -> Response {
    let state = app.network.state();
    let Some(host) = request_host(req.headers(), "https", Some(port)).map(|name| name.host().to_ascii_lowercase())
    else {
        return StatusCode::FORBIDDEN.into_response();
    };
    if !state.lan_enabled
        || port != state.ports.local_port
        || !app.network.lan.as_ref().is_some_and(|lan| lan.hosts.contains(&host))
    {
        return StatusCode::FORBIDDEN.into_response();
    }
    req.extensions_mut().insert(Scope::Lan(host));
    next.run(req).await
}

fn channel_enabled(app: &App, scope: &Scope) -> bool {
    let settings = app.network.state();
    match scope {
        Scope::Local => settings.local_enabled,
        Scope::Lan(_) => settings.lan_enabled,
    }
}

async fn channel_guard(
    State(app): State<App>,
    Extension(scope): Extension<Scope>,
    req: Request,
    next: Next,
) -> Response {
    if !channel_enabled(&app, &scope) {
        return StatusCode::FORBIDDEN.into_response();
    }
    next.run(req).await
}

/// เทียบแบบไม่หยุดเมื่อเจอไบต์แรกที่ต่าง
fn same_secret(a: &[u8], b: &[u8]) -> bool {
    a.len() == b.len() && a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

/// token ที่รับได้: local_token จากไฟล์ (เจ้าของ/Administrators) หรือ token ที่ panel แลกมา
fn native_token(app: &App, headers: &HeaderMap) -> bool {
    headers
        .get("x-synapta-local")
        .is_some_and(|v| same_secret(v.as_bytes(), app.local_token.as_bytes()))
}

async fn native_guard(State(app): State<App>, req: Request, next: Next) -> Response {
    if req.headers().contains_key(header::ORIGIN) || !(native_token(&app, req.headers()) || native_proof(&app, &req)) {
        return StatusCode::FORBIDDEN.into_response();
    }
    next.run(req).await
}

fn native_proof(app: &App, req: &Request) -> bool {
    let text = |name| req.headers().get(name).and_then(|value| value.to_str().ok());
    let (Some(time), Some(nonce), Some(proof), Some(target)) = (
        text("x-synapta-time"),
        text("x-synapta-nonce"),
        text("x-synapta-proof"),
        req.uri().path_and_query(),
    ) else {
        return false;
    };
    if time
        .parse::<u64>()
        .ok()
        .is_none_or(|time| time.abs_diff(crate::auth::now_secs()) > 15)
        || nonce.len() != 32
        || !nonce.bytes().all(|b| b.is_ascii_hexdigit())
    {
        return false;
    }
    let message = crate::proof::control_message(time, nonce, target.as_str());
    crate::proof::check(&app.local_token, crate::proof::PANEL_CONTROL, &message, proof)
        && app.display.accept_nonce(nonce)
}

/// endpoint พวกนี้เปิดเผย OTP และสั่ง revoke จึงต้องมาจากเจ้าของเครื่องจริง ๆ — token กัน process ของ "ผู้ใช้อื่นบนเครื่องเดียวกัน"
/// (loopback เปิดให้ทุก process ต่อได้; ไม่มี token ผู้ใช้ธรรมดาจะอ่าน OTP ไปจับคู่แล้วได้ exec ด้วยสิทธิ์ของ agent)
async fn token_guard(State(app): State<App>, Extension(scope): Extension<Scope>, req: Request, next: Next) -> Response {
    let native =
        scope == Scope::Local && !req.headers().contains_key(header::ORIGIN) && native_token(&app, req.headers());
    let panel = channel_enabled(&app, &scope)
        && req
            .headers()
            .get("x-synapta-local")
            .and_then(|v| v.to_str().ok())
            .is_some_and(|token| app.panel.valid_for(token, &scope));
    if !native && !panel {
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
        "commands": commands_json(&app),
        "network": network_json(&app),
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

async fn panel_session(State(app): State<App>, Query(query): Query<HashMap<String, String>>) -> Response {
    let scope = match query.get("lan") {
        None => Scope::Local,
        Some(host) => {
            let host = host.to_ascii_lowercase();
            if !app.network.lan.as_ref().is_some_and(|lan| lan.hosts.contains(&host)) {
                return error(
                    StatusCode::BAD_REQUEST,
                    "bad_panel_host",
                    "ชื่อนี้ไม่มีในใบรับรอง HTTPS ของ panel",
                );
            }
            Scope::Lan(host)
        }
    };
    if !channel_enabled(&app, &scope) {
        return error(StatusCode::FORBIDDEN, "channel_disabled", "ช่องทางเข้าหน้าจัดการนี้ปิดอยู่");
    }
    Json(json!({"code": app.panel.issue_for(scope), "expires_in": 60})).into_response()
}

#[derive(Deserialize)]
struct Exchange {
    code: String,
}

async fn panel_exchange(
    State(app): State<App>,
    Extension(scope): Extension<Scope>,
    Json(body): Json<Exchange>,
) -> Response {
    match app.panel.exchange_for(&body.code, &scope) {
        Some((token, expires_at)) => Json(json!({"token": token, "expires_at": expires_at})).into_response(),
        None => StatusCode::FORBIDDEN.into_response(),
    }
}

async fn panel_reload(State(app): State<App>, Extension(scope): Extension<Scope>) -> Json<Value> {
    Json(json!({"code": app.panel.issue_for(scope)}))
}

fn network_json(app: &App) -> Value {
    let state = app.network.state();
    json!({
        "port": state.ports.port, "discovery_port": state.ports.discovery_port, "local_port": state.ports.local_port,
        "local_enabled": state.local_enabled, "lan_enabled": state.lan_enabled,
        "lan_ready": app.network.lan.is_some(),
        "lan_hosts": app.network.lan.as_ref().map_or(&[][..], |lan| &lan.hosts[..]),
        "web_listener": app.network.panel_health(),
    })
}

async fn network_status(State(app): State<App>) -> Json<Value> {
    Json(network_json(&app))
}

async fn set_network(
    State(app): State<App>,
    Extension(scope): Extension<Scope>,
    Json(body): Json<NetworkRequest>,
) -> Response {
    let old_port = app.network.state().ports.local_port;
    if let Err(message) = app.network.change(body).await {
        return error(StatusCode::CONFLICT, "network_change_failed", &message);
    }
    let mut value = network_json(&app);
    if old_port != body.ports.local_port && channel_enabled(&app, &scope) {
        value["code"] = json!(app.panel.issue_for(scope));
    }
    Json(value).into_response()
}

async fn enable_panel(State(app): State<App>) -> Response {
    let mut next = app.network.state();
    next.local_enabled = true;
    next.lan_enabled &= app.network.lan.is_some();
    match app.network.change(next).await {
        Ok(()) => Json(network_json(&app)).into_response(),
        Err(message) => error(StatusCode::CONFLICT, "network_change_failed", &message),
    }
}

// ---------- อัปเดต ----------

async fn update_status(State(app): State<App>) -> Json<Value> {
    let mut state = app.updates.status();
    state["web"] = app.web_updates.status();
    state["web_source"] = json!(app.settings.get().update.web_source);
    Json(state)
}

async fn recover_web(State(app): State<App>) -> Response {
    match app.web_updates.recover() {
        Ok(()) => Json(app.web_updates.status()).into_response(),
        Err(e) => error(StatusCode::CONFLICT, &e.code, &e.message),
    }
}

async fn web_status(State(app): State<App>) -> Json<Value> {
    Json(app.web_updates.status())
}

fn start_web(app: &App, action: Action) -> Response {
    match app.web_updates.start(action) {
        Ok(()) => (StatusCode::ACCEPTED, Json(app.web_updates.status())).into_response(),
        Err(e) => refused(e),
    }
}

async fn web_check(State(app): State<App>) -> Response {
    start_web(&app, Action::Check)
}

async fn web_apply(State(app): State<App>, Json(body): Json<ApplyBody>) -> Response {
    start_web(&app, Action::Apply(body.version))
}

async fn web_retry(State(app): State<App>) -> Response {
    start_web(&app, Action::Retry)
}

async fn web_rollback(State(app): State<App>) -> Response {
    start_web(&app, Action::Rollback)
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
    web_source: Option<String>,
}

async fn update_settings(State(app): State<App>, Json(body): Json<UpdateSettingsBody>) -> Response {
    // "" = กลับไปใช้แหล่งที่ฝังตอน build; แหล่งแบบโฟลเดอร์ในเครื่อง (dir:) ตั้งผ่าน panel ไม่ได้ (ใช้ทดสอบ ผ่าน settings.json ของผู้ดูแล)
    for src in [body.source.as_deref(), body.web_source.as_deref()]
        .into_iter()
        .flatten()
        .filter(|s| !s.trim().is_empty())
    {
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
    match app.updates.set_sources(
        body.auto_update,
        body.source.map(|s| s.trim().to_string()),
        body.web_source.map(|s| s.trim().to_string()),
    ) {
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

// ---------- คำสั่งรันด้วยบัญชีไหน (run_as.rs) ----------

fn commands_json(app: &App) -> Value {
    let settings = app.settings.get().commands;
    let helper = app.display.helper_info();
    json!({
        "run_as_owner": settings.run_as_owner,
        "administrator": settings.administrator,
        "helper_connected": helper.is_some(),
        "helper_user": helper.as_ref().map(|h| &h.user),
        "helper_elevated": helper.as_ref().map(|h| h.elevated),
    })
}

async fn commands(State(app): State<App>) -> Json<Value> {
    Json(commands_json(&app))
}

#[derive(Deserialize)]
struct CommandsBody {
    run_as_owner: Option<bool>,
    administrator: Option<bool>,
}

async fn set_commands(State(app): State<App>, Json(body): Json<CommandsBody>) -> Response {
    tracing::warn!(run_as_owner = ?body.run_as_owner, administrator = ?body.administrator, "เจ้าของเปลี่ยนบัญชีที่ใช้รันคำสั่ง");
    let saved = app.settings.update(|s| {
        if let Some(on) = body.run_as_owner {
            s.commands.run_as_owner = on;
        }
        if let Some(on) = body.administrator {
            s.commands.administrator = on;
        }
    });
    match saved {
        Ok(settings) => {
            // เจ้าของเพิ่งเปิดที่หน้าเครื่อง — ขอ UAC ตอนนี้เลย ไม่ต้องรอคำสั่งแรกจากระยะไกล (ซึ่งไม่มีใครกดยืนยันให้)
            let needs_uac = settings.commands.run_as_owner
                && settings.commands.administrator
                && app.display.helper_info().is_some_and(|h| !h.elevated);
            if needs_uac {
                let app = app.clone();
                tokio::spawn(async move {
                    if let Err(e) = crate::run_as::elevate(&app).await {
                        tracing::warn!("ขอสิทธิ์ administrator ไม่สำเร็จ: {}", e.message);
                    }
                });
            }
            Json(commands_json(&app)).into_response()
        }
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
    let mut port = app.network.watch_panel_port();
    let mut response = ws.on_upgrade(move |socket| async move {
        tokio::select! {
            _ = app.display.serve_helper(socket) => {}
            _ = port.changed() => {}
        }
    });
    if let Ok(value) = server_proof.parse() {
        response.headers_mut().insert("x-synapta-server-proof", value);
    }
    response
}
