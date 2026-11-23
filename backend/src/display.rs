//! จับภาพ/ส่ง input: ตัดสินว่า "ใครเป็นคนทำ" — จุดเดียวที่ screen.* / input.* ผ่าน
//!
//! console: ทำใน process นี้เลย (รันในเซสชันผู้ใช้อยู่แล้ว เหมือน Phase 0)
//! service: service อยู่ Session 0 จับภาพ/ส่ง input ไม่ได้ และไม่ควรได้ — งานถูกส่งต่อให้ helper (`synapta-connect helper`)
//! ที่รันใน session ของเจ้าของเครื่อง ด้วยสิทธิ์ของเจ้าของ และต่อเข้ามาหา service เองด้วย local_token
//! (อ่านได้เฉพาะเจ้าของ/Administrators) เจ้าของปิดสวิตช์ใน panel = งานทั้งหมดถูกปฏิเสธทันที

use crate::{
    app::App,
    config::Mode,
    input,
    rpc::{Res, RpcError, parse},
    screen,
};
use axum::extract::ws::{Message, WebSocket};
use futures_util::{SinkExt, StreamExt};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{collections::HashMap, sync::Mutex, time::Duration};
use tokio::sync::{mpsc, oneshot};

/// ใช้ไม่ได้จากสถานะ helper หรือเซสชัน Windows ข้อความภาษาไทยพร้อมแสดงให้ผู้ใช้ปลายทาง
pub const SCREEN_INPUT_UNAVAILABLE: i32 = -32004;
/// จับภาพความละเอียดสูงหลายจอใช้เวลาได้ — นานกว่านี้ถือว่า helper ค้าง
const HELPER_TIMEOUT: Duration = Duration::from_secs(20);

/// method ที่ helper ยอมทำ — ที่เหลือถูกปฏิเสธ แม้ service ถูกหลอกให้ส่งมา
pub fn is_display_method(method: &str) -> bool {
    matches!(
        method,
        "screen.list"
            | "screen.snapshot"
            | "input.mouse_move"
            | "input.click"
            | "input.scroll"
            | "input.type"
            | "input.key"
    )
}

/// ทำงานจริงใน process นี้ (ใช้ทั้งโหมด console และใน helper)
pub async fn execute(method: &str, params: Value) -> Res<Value> {
    match method {
        "screen.list" => tokio::task::spawn_blocking(screen::list).await?,
        "screen.snapshot" => screen::snapshot(parse(params)?).await,
        "input.mouse_move" | "input.click" | "input.scroll" | "input.type" | "input.key" => {
            input::rpc(method, params).await
        }
        other => Err(RpcError::new(
            crate::rpc::METHOD_NOT_FOUND,
            format!("helper ไม่รับ method '{other}'"),
        )),
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HelperInfo {
    pub user: String,
    pub session: u32,
}

struct Job {
    method: String,
    params: Value,
    reply: oneshot::Sender<Res<Value>>,
}

struct Link {
    jobs: mpsc::Sender<Job>,
    info: HelperInfo,
}

/// nonce ของ helper ที่เคยใช้แล้ว เก็บล่าสุดกี่ตัว (ผู้ดักเห็น nonce+proof ที่ loopback เล่นซ้ำไม่ได้)
const REMEMBERED_NONCES: usize = 256;

pub struct Display {
    mode: Mode,
    helper: Mutex<Option<Link>>,
    used_nonces: Mutex<std::collections::VecDeque<String>>,
}

fn unavailable(why: &str) -> RpcError {
    RpcError::new(SCREEN_INPUT_UNAVAILABLE, format!("จับภาพ/ส่ง input ไม่ได้: {why}"))
}

impl Display {
    pub fn new(mode: Mode) -> Display {
        Display {
            mode,
            helper: Mutex::new(None),
            used_nonces: Mutex::new(Default::default()),
        }
    }

    /// จด nonce ที่ใช้แล้ว — false ถ้าเคยใช้ (replay)
    pub fn accept_nonce(&self, nonce: &str) -> bool {
        let mut used = self.used_nonces.lock().unwrap();
        if used.iter().any(|n| n == nonce) {
            return false;
        }
        if used.len() >= REMEMBERED_NONCES {
            used.pop_front();
        }
        used.push_back(nonce.to_string());
        true
    }

    pub fn helper_info(&self) -> Option<HelperInfo> {
        self.helper.lock().unwrap().as_ref().map(|l| l.info.clone())
    }

    pub async fn call(&self, app: &App, method: &str, params: Value) -> Res<Value> {
        if self.mode == Mode::Console {
            return execute(method, params).await;
        }
        if !app.settings.get().screen_input.enabled {
            return Err(unavailable("เจ้าของเครื่องยังไม่ได้อนุญาต (เปิดที่ panel > การจับภาพ/ส่ง input)"));
        }
        let jobs = self
            .helper
            .lock()
            .unwrap()
            .as_ref()
            .map(|l| l.jobs.clone())
            .ok_or_else(|| {
                unavailable("ไม่มี helper ใน session ของเจ้าของเครื่อง (เจ้าของยังไม่ได้ล็อกอิน หรือ helper ไม่ทำงาน)")
            })?;
        let (reply, answer) = oneshot::channel();
        let job = Job {
            method: method.into(),
            params,
            reply,
        };
        jobs.send(job).await.map_err(|_| unavailable("helper หลุดการเชื่อมต่อ"))?;
        match tokio::time::timeout(HELPER_TIMEOUT, answer).await {
            Ok(Ok(result)) => result,
            Ok(Err(_)) => Err(unavailable("helper หลุดการเชื่อมต่อระหว่างทำงาน")),
            Err(_) => Err(unavailable("helper ตอบช้าเกินกำหนด")),
        }
    }

    /// ฝั่ง service ของ WebSocket ที่ helper ต่อเข้ามา — ทีละ helper (ตัวที่สองถูกปฏิเสธจนกว่าตัวแรกจะหลุด)
    pub async fn serve_helper(&self, socket: WebSocket) {
        let (mut sink, mut stream) = socket.split();
        let hello = tokio::time::timeout(Duration::from_secs(5), stream.next()).await;
        let info = match hello {
            Ok(Some(Ok(Message::Text(t)))) => serde_json::from_str::<Value>(&t)
                .ok()
                .and_then(|v| serde_json::from_value::<HelperInfo>(v["hello"].clone()).ok()),
            _ => None,
        };
        let Some(info) = info else { return };

        let (jobs, mut queue) = mpsc::channel::<Job>(8);
        {
            let mut slot = self.helper.lock().unwrap();
            if slot.is_some() {
                return;
            }
            tracing::info!(user = %info.user, session = info.session, "helper ต่อเข้ามา");
            *slot = Some(Link { jobs, info });
        }
        let _detach = Detach(&self.helper);

        let mut pending: HashMap<u64, oneshot::Sender<Res<Value>>> = HashMap::new();
        let mut next_id = 0u64;
        let mut ping = tokio::time::interval(Duration::from_secs(15)); // จับ helper ที่ตายเงียบ ๆ (ล็อกออฟ)
        loop {
            tokio::select! {
                job = queue.recv() => {
                    let Some(job) = job else { break };
                    next_id += 1;
                    let request = json!({"id": next_id, "method": job.method, "params": job.params}).to_string();
                    if sink.send(Message::Text(request.into())).await.is_err() {
                        let _ = job.reply.send(Err(unavailable("helper หลุดการเชื่อมต่อ")));
                        break;
                    }
                    pending.insert(next_id, job.reply);
                }
                msg = stream.next() => match msg {
                    Some(Ok(Message::Text(text))) => {
                        let Ok(v) = serde_json::from_str::<Value>(&text) else { continue };
                        let Some(reply) = v["id"].as_u64().and_then(|id| pending.remove(&id)) else { continue };
                        let result = match v.get("error") {
                            Some(e) if !e.is_null() => Err(RpcError::new(
                                e["code"].as_i64().unwrap_or(-32000) as i32,
                                e["message"].as_str().unwrap_or("helper ผิดพลาด"),
                            )),
                            _ => Ok(v["result"].clone()),
                        };
                        let _ = reply.send(result);
                    }
                    Some(Ok(Message::Close(_))) | Some(Err(_)) | None => break,
                    _ => {}
                },
                _ = ping.tick() => {
                    if sink.send(Message::Ping(Vec::new().into())).await.is_err() {
                        break;
                    }
                }
            }
        }
        // pending ที่ค้างอยู่ถูก drop → call() ได้ "helper หลุด" ทันที
    }
}

/// ปล่อยช่อง helper เมื่อ connection จบทุกกรณี (รวม future ถูกทิ้งกลางทาง)
struct Detach<'a>(&'a Mutex<Option<Link>>);

impl Drop for Detach<'_> {
    fn drop(&mut self) {
        *self.0.lock().unwrap() = None;
        tracing::info!("helper หลุดการเชื่อมต่อ");
    }
}
