//! เส้นทาง HTTPS / WSS ทั้งหมดของพอร์ตหลัก (TLS 1.3 เท่านั้น)
//!
//!   ไม่ต้องมีใบรับรอง client: GET /api/v1/hello, POST /api/v1/auth/spake2-handshake
//!   ต้องมีใบรับรอง client ที่อยู่ใน allowlist (mTLS): POST /api/v1/rpc, GET /api/v1/ws,
//!       PUT /api/v1/transfers/{id}, GET /api/v1/files, GET /api/v1/screen/stream
//!   เฉพาะเครื่องนี้เอง (พอร์ต loopback แยก): ดู local.rs

use crate::{
    api::{Ctx, WsCtx, dispatch},
    app::App,
    file_access, pairing, pty,
    rpc::{self, BUSY, CONFLICT, INVALID_PARAMS, METHOD_NOT_FOUND, NOT_AVAILABLE, Request as RpcRequest, RpcError},
    screen,
    tls::ClientIdentity,
    transfer,
};
use axum::{
    Extension, Json, Router,
    body::Bytes,
    extract::{
        DefaultBodyLimit, Path, Query, Request, State,
        ws::{Message, WebSocket, WebSocketUpgrade},
    },
    http::{StatusCode, header},
    middleware::{self, Next},
    response::{IntoResponse, Response},
    routing::{get, post, put},
};
use futures_util::{SinkExt, StreamExt};
use serde::Deserialize;
use serde_json::{Value, json};
use std::{collections::HashMap, time::Duration};
use tokio::{sync::mpsc, task::JoinHandle};
use tower::ServiceExt;
use tower_http::services::ServeFile;

/// อุปกรณ์ที่ผ่านด่าน mTLS (id = fingerprint ของ public key) — handler ดึงจาก request extensions
#[derive(Clone)]
pub struct Verified(pub String);

/// code ของ RpcError → HTTP status (ใช้กับ endpoint ที่ไม่ใช่ JSON-RPC)
impl IntoResponse for RpcError {
    fn into_response(self) -> Response {
        let status = match self.code {
            INVALID_PARAMS | NOT_AVAILABLE => StatusCode::BAD_REQUEST,
            METHOD_NOT_FOUND => StatusCode::NOT_FOUND,
            CONFLICT => StatusCode::CONFLICT,
            BUSY => StatusCode::TOO_MANY_REQUESTS,
            _ => StatusCode::INTERNAL_SERVER_ERROR,
        };
        (status, Json(json!({"error": self}))).into_response()
    }
}

pub fn router(app: App) -> Router {
    let protected = Router::new()
        .route("/api/v1/rpc", post(http_rpc))
        .route("/api/v1/ws", get(ws_upgrade))
        // axum ตัด body ที่ 2 MB โดยปริยาย — chunk ของโหมดขนาน (transfer.rs) ใหญ่กว่านั้น
        .route(
            "/api/v1/transfers/{id}",
            put(put_chunk).layer(DefaultBodyLimit::max(transfer::MAX_BODY)),
        )
        .route("/api/v1/files", get(download))
        .route("/api/v1/screen/stream", get(screen_stream))
        .layer(middleware::from_fn_with_state(app.clone(), require_client));

    Router::new()
        .route("/api/v1/hello", get(hello))
        .route("/api/v1/auth/spake2-handshake", post(pairing::handshake))
        .merge(protected)
        // ไม่มี CORS โดยตั้งใจ: client ของเราไม่ใช่เบราว์เซอร์ และการยิงข้ามเว็บ (CSRF / DNS rebinding)
        // ต้องถูกตัดทิ้ง — เบราว์เซอร์ทุกตัวใส่ Origin ให้เอง จึงปฏิเสธทั้งหมดที่มี Origin
        .layer(middleware::from_fn(reject_browsers))
        .with_state(app)
}

// ---------- ด่านของ route ----------

async fn reject_browsers(req: Request, next: Next) -> Response {
    if req.headers().contains_key(header::ORIGIN) {
        return StatusCode::FORBIDDEN.into_response();
    }
    next.run(req).await
}

/// route ที่ต้อง login: connection นี้ต้องแสดงใบรับรองที่ key อยู่ใน allowlist (tls.rs ตรวจตอน handshake)
/// เช็ค is_active ซ้ำทุก request กันช่วงที่เพิ่ง revoke แต่ connection ยังไม่ทันถูกตัด
async fn require_client(
    State(app): State<App>,
    Extension(ClientIdentity(device)): Extension<ClientIdentity>,
    mut req: Request,
    next: Next,
) -> Response {
    match device.filter(|d| app.auth.is_active(d)) {
        Some(id) => {
            req.extensions_mut().insert(Verified(id));
            next.run(req).await
        }
        None => (
            StatusCode::UNAUTHORIZED,
            Json(json!({"error": {"code": "client_certificate_required",
                                  "message": "ต้องแสดงใบรับรอง client ของอุปกรณ์ที่จับคู่แล้ว (mTLS)"}})),
        )
            .into_response(),
    }
}

// ---------- ไม่ต้องมีใบรับรอง client ----------

/// ตอบแค่ตัวตนพื้นฐาน — คนแปลกหน้าบน LAN เรียกได้ จึงไม่บอก OS / IP ของ VPN / สิทธิ์
/// (รายละเอียดเหล่านั้นอยู่ใน system.profile หลัง login และ /api/v1/local/status)
async fn hello(State(app): State<App>) -> Json<Value> {
    Json(json!({
        "device_id": app.node_id, "name": app.config.name, "version": env!("CARGO_PKG_VERSION"),
        "api_version": 1, "port": app.network.state().ports.port,
        "host_pubkey_fingerprint": app.host.fingerprint(),
        "pairing_mode": app.auth.current_otp().is_some(),
    }))
}

// ---------- ต้องผ่าน mTLS ----------

async fn http_rpc(
    State(app): State<App>,
    Extension(Verified(device)): Extension<Verified>,
    body: Bytes,
) -> Json<Value> {
    let ctx = Ctx {
        session_id: device,
        ws: None,
    };
    Json(handle_message(&app, &ctx, &body).await)
}

/// แปลงข้อความ JSON-RPC หนึ่งก้อนเป็นคำตอบ (ใช้ร่วมกับ WebSocket)
async fn handle_message(app: &App, ctx: &Ctx, raw: &[u8]) -> Value {
    match serde_json::from_slice::<RpcRequest>(raw) {
        Ok(req) => rpc::response(req.id, dispatch(app, ctx, &req.method, req.params).await),
        Err(e) => rpc::response(None, Err(RpcError::new(-32700, format!("อ่าน JSON-RPC ไม่ได้: {e}")))),
    }
}

async fn ws_upgrade(
    State(app): State<App>,
    Extension(Verified(device)): Extension<Verified>,
    ws: WebSocketUpgrade,
) -> Response {
    ws.max_message_size(1024 * 1024)
        .on_upgrade(move |socket| ws_session(app, device, socket))
}

/// เก็บกวาดเมื่อ connection จบ — รวมกรณีที่ถูกตัดกลางคัน (revoke / kill-all) ซึ่ง future ถูกทิ้งโดยไม่รันโค้ดท้ายฟังก์ชัน
struct WsCleanup {
    app: App,
    ptys: pty::Registry,
    writer: JoinHandle<()>,
}

impl Drop for WsCleanup {
    fn drop(&mut self) {
        let in_helper = self.ptys.close_all();
        crate::run_as::close_owner_ptys(&self.app, in_helper);
        self.writer.abort();
    }
}

async fn ws_session(app: App, device: String, socket: WebSocket) {
    let (mut sink, mut stream) = socket.split();
    let (out, mut out_rx) = mpsc::channel::<String>(256);

    // ตัวเขียนตัวเดียวของ socket: คำตอบ + notification ของทุก request/PTY ผ่านคิวเดียวกัน
    let writer = tokio::spawn(async move {
        let mut ping = tokio::time::interval(Duration::from_secs(30)); // จับ connection ที่ตายเงียบ ๆ
        loop {
            let sent = tokio::select! {
                msg = out_rx.recv() => match msg {
                    Some(text) => sink.send(Message::Text(text.into())).await,
                    None => break,
                },
                _ = ping.tick() => sink.send(Message::Ping(Bytes::new())).await,
            };
            if sent.is_err() {
                break;
            }
        }
    });
    let cleanup = WsCleanup {
        app: app.clone(),
        ptys: pty::Registry::default(),
        writer,
    };

    let me = Some(device.clone());
    let kill = crate::tls::wait_for_kill(&app, &me, app.auth.subscribe_kills());
    tokio::pin!(kill);
    loop {
        let msg = tokio::select! {
            msg = stream.next() => msg,
            _ = &mut kill => break, // revoke / kill-all
        };
        let Some(Ok(msg)) = msg else { break };
        match msg {
            Message::Text(text) => {
                // แต่ละ request ทำงานแยก — exec ที่ช้าต้องไม่ขวาง pty.write
                let (app, out, ptys, device) = (app.clone(), out.clone(), cleanup.ptys.clone(), device.clone());
                tokio::spawn(async move {
                    let ctx = Ctx {
                        session_id: device,
                        ws: Some(WsCtx { out: out.clone(), ptys }),
                    };
                    let reply = handle_message(&app, &ctx, text.as_bytes()).await;
                    let _ = out.send(reply.to_string()).await;
                });
            }
            Message::Close(_) => break,
            _ => {} // ping/pong/binary: ไม่มีงานให้ทำ
        }
    }
}

#[derive(Deserialize)]
struct ChunkQuery {
    offset: u64,
}

async fn put_chunk(
    State(app): State<App>,
    Path(id): Path<String>,
    Query(q): Query<ChunkQuery>,
    body: Bytes,
) -> Result<Json<Value>, RpcError> {
    Ok(Json(app.transfers.put_chunk(&id, q.offset, body.to_vec()).await?))
}

async fn download(State(app): State<App>, Query(q): Query<HashMap<String, String>>, req: Request) -> Response {
    let Some(path) = q.get("path") else {
        return RpcError::invalid("ต้องมี ?path=").into_response();
    };
    let path = match file_access::for_read(path, &app.config.file_roots) {
        Ok(p) => p,
        Err(e) => return e.into_response(),
    };
    tracing::info!(device = ?req.extensions().get::<Verified>().map(|v| &v.0), path = %path.display(), "files.download");
    // ServeFile รองรับ Range (206) ให้ client ต่อดาวน์โหลดที่ค้างได้
    match ServeFile::new(path).oneshot(req).await {
        Ok(res) => res.into_response(),
        Err(e) => match e {},
    }
}

async fn screen_stream(
    State(app): State<App>,
    Extension(Verified(device)): Extension<Verified>,
    Query(p): Query<screen::CaptureParams>,
) -> Result<Response, RpcError> {
    if app.config.mode == crate::config::Mode::Service {
        // ต้องจับภาพผ่าน helper ใน session ผู้ใช้ ซึ่งยังไม่รองรับการถ่ายทอดต่อเนื่อง — ใช้ screen.snapshot ซ้ำแทน
        return Err(RpcError::new(
            NOT_AVAILABLE,
            "โหมด service ไม่รองรับ MJPEG stream — ใช้ screen.snapshot ซ้ำแทน",
        ));
    }
    let permit = app
        .screen_streams
        .clone()
        .try_acquire_owned()
        .map_err(|_| RpcError::new(BUSY, "stream พร้อมกันเต็มแล้ว"))?;
    tracing::info!(%device, "screen.stream");
    let body = tokio::task::spawn_blocking(move || screen::mjpeg_body(p, permit, move || app.auth.is_active(&device)))
        .await??;
    Ok((
        [(header::CONTENT_TYPE, "multipart/x-mixed-replace; boundary=frame")],
        body,
    )
        .into_response())
}
