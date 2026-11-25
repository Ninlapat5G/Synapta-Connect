//! จับภาพ/ส่ง input: ตัดสินว่า "ใครเป็นคนทำ" — จุดเดียวที่ screen.* / input.* ผ่าน
//!
//! console: ทำใน process นี้เลย (รันในเซสชันผู้ใช้อยู่แล้ว เหมือน Phase 0)
//! service: service อยู่ Session 0 จับภาพ/ส่ง input ไม่ได้ และไม่ควรได้ — งานถูกส่งต่อให้ helper (`synapta-connect helper`)
//! ที่รันใน session ของเจ้าของเครื่อง ด้วยสิทธิ์ของเจ้าของ และต่อเข้ามาหา service เองด้วย local_token
//! (อ่านได้เฉพาะเจ้าของ/Administrators) เจ้าของปิดสวิตช์ใน panel = งานทั้งหมดถูกปฏิเสธทันที
//!
//! `Display` ถือช่องทางไปหา helper ตัวเดียวของเครื่อง — คำสั่ง exec/terminal ที่รันด้วยบัญชีเจ้าของ (run_as.rs)
//! ก็ส่งผ่านช่องนี้ (`helper_call`) และ output ของ terminal ที่ helper ส่งกลับถูกส่งต่อด้วย `route_pty`

use crate::{
    app::App,
    config::Mode,
    input, pty,
    rpc::{Res, RpcError, notification, parse},
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
/// ต้องต่ำกว่าเพดาน 15 วินาทีของ SynaptaCode ไม่งั้นผู้ใช้ได้ "timeout" แทนข้อความที่บอกสาเหตุ
const HELPER_TIMEOUT: Duration = Duration::from_secs(12);

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
    /// token ของ helper มีสิทธิ์ administrator เต็ม (helper ก่อน 0.3.14 ไม่ส่ง = false)
    #[serde(default)]
    pub elevated: bool,
    /// รุ่นของ helper — ว่าง = ก่อน 0.3.15 (helper 0.3.13 ลงไปรันคำสั่งให้ไม่ได้)
    #[serde(default)]
    pub version: String,
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

/// terminal ที่ helper เปิดให้ client หนึ่ง: output ส่งกลับทาง `out` และลบออกจาก `registry` ของ client เมื่อจบ
struct PtyRoute {
    out: mpsc::Sender<String>,
    registry: pty::Registry,
}

/// client อ่าน output ไม่ทันนานกว่านี้ = ตัด terminal นั้น (ไม่ให้ client ช้าตัวเดียวขวางงานอื่นของ helper)
const ROUTE_SEND_TIMEOUT: Duration = Duration::from_secs(5);

pub struct Display {
    mode: Mode,
    helper: Mutex<Option<Link>>,
    used_nonces: Mutex<std::collections::VecDeque<String>>,
    pty_routes: Mutex<HashMap<String, PtyRoute>>,
}

const SCREEN_INPUT: &str = "จับภาพ/ส่ง input";

/// `what` = งานที่ทำไม่ได้ ขึ้นต้นข้อความให้ผู้ใช้รู้ว่าอะไรพัง
fn unavailable(what: &str, why: &str) -> RpcError {
    RpcError::new(SCREEN_INPUT_UNAVAILABLE, format!("{what} ไม่ได้: {why}"))
}

impl Display {
    pub fn new(mode: Mode) -> Display {
        Display {
            mode,
            helper: Mutex::new(None),
            used_nonces: Mutex::new(Default::default()),
            pty_routes: Mutex::new(HashMap::new()),
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
        if self.mode != Mode::Service {
            return execute(method, params).await;
        }
        if !app.settings.get().screen_input.enabled {
            return Err(unavailable(
                SCREEN_INPUT,
                "เจ้าของเครื่องยังไม่ได้อนุญาต (เปิดที่ panel > การจับภาพ/ส่ง input)",
            ));
        }
        self.helper_call(method, params, HELPER_TIMEOUT, SCREEN_INPUT).await
    }

    /// ส่งงานให้ helper โดยไม่ตรวจสวิตช์ใด ๆ — ผู้เรียกตัดสินสิทธิ์เองก่อน (display: screen_input, run_as: commands)
    pub async fn helper_call(&self, method: &str, params: Value, timeout: Duration, what: &str) -> Res<Value> {
        let jobs = self
            .helper
            .lock()
            .unwrap()
            .as_ref()
            .map(|l| l.jobs.clone())
            .ok_or_else(|| {
                unavailable(
                    what,
                    "ไม่มี helper ใน session ของเจ้าของเครื่อง (เจ้าของยังไม่ได้ล็อกอิน หรือ helper ไม่ทำงาน)",
                )
            })?;
        let (reply, answer) = oneshot::channel();
        let job = Job {
            method: method.into(),
            params,
            reply,
        };
        // The deadline covers queueing too: when the link to the helper stalls, its queue (8 jobs) fills and
        // `send` waits with no end, so a call used to hang past its timeout.
        let request = async {
            jobs.send(job)
                .await
                .map_err(|_| unavailable(what, "helper หลุดการเชื่อมต่อ"))?;
            answer
                .await
                .unwrap_or_else(|_| Err(unavailable(what, "helper หลุดการเชื่อมต่อระหว่างทำงาน")))
        };
        tokio::time::timeout(timeout, request)
            .await
            .unwrap_or_else(|_| Err(unavailable(what, "helper ตอบช้าเกินกำหนด")))
    }

    /// ให้ output ของ terminal `pty_id` ที่ helper เปิด ไปถึง client เจ้าของ terminal
    pub fn route_pty(&self, pty_id: &str, out: mpsc::Sender<String>, registry: pty::Registry) {
        self.pty_routes
            .lock()
            .unwrap()
            .insert(pty_id.to_string(), PtyRoute { out, registry });
    }

    /// เลิกส่ง output ของ `pty_id` และลบออกจากตารางของ client
    pub fn unroute_pty(&self, pty_id: &str) {
        if let Some(route) = self.pty_routes.lock().unwrap().remove(pty_id) {
            route.registry.forget(pty_id);
        }
    }

    /// ส่ง notification ของ terminal จาก helper ต่อให้ client — pty.exit = ปิดเส้นทางนั้น
    async fn forward_pty(&self, method: &str, text: String, params: &Value) {
        let Some(id) = params["pty_id"].as_str() else { return };
        let exit = method == "pty.exit";
        let Some(out) = self.pty_routes.lock().unwrap().get(id).map(|r| r.out.clone()) else {
            return;
        };
        if exit {
            self.unroute_pty(id);
        }
        let sent = tokio::time::timeout(ROUTE_SEND_TIMEOUT, out.send(text)).await;
        if !exit && !matches!(sent, Ok(Ok(()))) {
            // client ปิดไปแล้วหรืออ่านไม่ทัน: ปิด terminal ที่ helper แทนการค้างรอ
            self.unroute_pty(id);
            let jobs = self.helper.lock().unwrap().as_ref().map(|l| l.jobs.clone());
            if let Some(jobs) = jobs {
                let (reply, _) = oneshot::channel();
                let params = json!({"pty_id": id});
                let _ = jobs.try_send(Job {
                    method: "pty.close".into(),
                    params,
                    reply,
                });
            }
        }
    }

    /// helper หลุด = terminal ทุกอันที่มันเปิดตายไปด้วย — บอก client ให้ครบ
    fn end_all_ptys(&self) {
        let routes: Vec<(String, PtyRoute)> = self.pty_routes.lock().unwrap().drain().collect();
        for (id, route) in routes {
            route.registry.forget(&id);
            let _ = route
                .out
                .try_send(notification("pty.exit", json!({"pty_id": id, "exit_code": null})));
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
                        let _ = job.reply.send(Err(unavailable("ส่งงานให้ helper", "helper หลุดการเชื่อมต่อ")));
                        break;
                    }
                    pending.insert(next_id, job.reply);
                }
                msg = stream.next() => match msg {
                    Some(Ok(Message::Text(text))) => {
                        let Ok(v) = serde_json::from_str::<Value>(&text) else { continue };
                        // notification (ไม่มี id) จาก helper มีแค่ output/จบของ terminal
                        if v["id"].is_null() {
                            if let Some(method @ ("pty.output" | "pty.exit")) = v["method"].as_str() {
                                self.forward_pty(method, text.to_string(), &v["params"]).await;
                            }
                            continue;
                        }
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
                    // Forget jobs whose caller already timed out, or a helper that never answers grows this map forever.
                    pending.retain(|_, reply| !reply.is_closed());
                    if sink.send(Message::Ping(Vec::new().into())).await.is_err() {
                        break;
                    }
                }
            }
        }
        // pending ที่ค้างอยู่ถูก drop → call() ได้ "helper หลุด" ทันที
        self.end_all_ptys();
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
