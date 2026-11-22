//! สั่งอัปเดตจากอุปกรณ์ที่จับคู่แล้ว (RPC `update.*`) — ให้ SynaptaCode อัปเดตเครื่องปลายทางได้โดยไม่ต้องไปกดที่เครื่อง
//!
//! ใช้ด่านและงานเดียวกับปุ่มใน panel (UpdateManager) จึงได้การตรวจลายเซ็น/SHA-256/กู้คืนครบเหมือนกัน
//! ไม่เพิ่มสิทธิ์: อุปกรณ์ที่จับคู่รัน exec ได้อยู่แล้ว และติดตั้งได้แค่แพ็กเกจที่ผู้เผยแพร่เซ็น

use super::Refused;
use crate::{
    app::App,
    rpc::{BUSY, INVALID_PARAMS, Res, RpcError, parse},
};
use serde::Deserialize;
use serde_json::{Value, json};

#[derive(Deserialize)]
struct InstallParams {
    /// ไม่ใส่ = ตรวจหารุ่นล่าสุดก่อนแล้วติดตั้งรุ่นนั้น
    version: Option<String>,
}

fn refused(r: Refused) -> RpcError {
    match r {
        Refused::Busy => RpcError::new(BUSY, "กำลังทำงานอัปเดตอื่นอยู่ — รอให้เสร็จก่อน"),
        Refused::RateLimited => RpcError::new(BUSY, "ตรวจถี่เกินไป — รอสักครู่"),
        Refused::Invalid(m) => RpcError::new(INVALID_PARAMS, m),
    }
}

/// สถานะเดียวกับที่ panel เห็น (รวมสถานะเว็บ panel)
fn status(app: &App) -> Value {
    let mut state = app.updates.status();
    state["web"] = app.web_updates.status();
    state
}

pub async fn rpc(app: &App, method: &str, params: Value) -> Res<Value> {
    match method {
        "update.status" => Ok(status(app)),
        "update.check" => {
            // เช็กอย่างเดียว: ผลไม่ทำให้ตัวจับเวลาติดตั้ง และไม่เลื่อนรอบตรวจปกติ (ดู UpdateState::remote_found)
            // ตอบทันที งานเครือข่ายทำเบื้องหลัง — client poll update.status ต่อ
            let job = app.updates.begin_remote_check().map_err(refused)?;
            let mgr = app.updates.clone();
            tokio::spawn(async move { mgr.run_check(job).await });
            Ok(status(app))
        }
        "update.install" => install(app, parse(params)?).await,
        _ => unreachable!("dispatch ส่งมาเฉพาะ update.*"),
    }
}

/// ตรวจ (ถ้าไม่ระบุรุ่น) แล้วเริ่มดาวน์โหลด/ติดตั้งเบื้องหลัง — คำตอบกลับมาก่อน service เริ่มใหม่
/// หลังจากนั้นการเชื่อมต่อจะหลุดราว 10-40 วินาที แล้ว client เช็กผลด้วย update.status หรือ system.profile
async fn install(app: &App, p: InstallParams) -> Res<Value> {
    let version = match p.version {
        Some(version) => version,
        None => {
            // ตรวจเพิ่งไปเมื่อกี้ (RateLimited) ก็ใช้ผลรอบนั้นได้
            match app.updates.check(true).await {
                Ok(()) | Err(Refused::RateLimited) => {}
                Err(r) => return Err(refused(r)),
            }
            let state = app.updates.status();
            let Some(version) = state["available"]["version"].as_str() else {
                return Ok(json!({"started": false, "status": status(app)}));
            };
            version.to_string()
        }
    };
    tracing::info!(version, "อุปกรณ์ที่จับคู่สั่งติดตั้งอัปเดต");
    let job = app.updates.begin_apply(&version, true).map_err(refused)?;
    let mgr = app.updates.clone();
    tokio::spawn(async move { mgr.run_apply(job).await });
    Ok(json!({"started": true, "version": version, "status": status(app)}))
}
