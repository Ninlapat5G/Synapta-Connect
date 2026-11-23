//! ตาราง method ของ RPC — จุดเดียวที่บอกว่า agent มีคำสั่งอะไรบ้าง (รายละเอียดอยู่ใน docs/API.md)
//!
//! ใช้ร่วมกันระหว่าง POST /v1/rpc และ WebSocket /v1/ws

use crate::{
    app::App,
    browse, display, exec, netif, profile, pty,
    rpc::{METHOD_NOT_FOUND, NOT_AVAILABLE, Res, RpcError, parse, to_value},
};
use serde::Deserialize;
use serde_json::{Value, json};
use tokio::sync::mpsc::Sender;

/// ส่วนที่มีเฉพาะเมื่อเรียกผ่าน WebSocket: ช่องส่ง notification กลับ + PTY ของ connection นี้
pub struct WsCtx {
    pub out: Sender<String>,
    pub ptys: pty::Registry,
}

pub struct Ctx {
    pub session_id: String,
    pub ws: Option<WsCtx>,
}

#[derive(Deserialize)]
struct RevokeParams {
    /// ไม่ใส่ = ตัวเอง; ใส่ได้เฉพาะ id ของตัวเอง
    session_id: Option<String>,
}

fn ok() -> Res<Value> {
    Ok(json!({"ok": true}))
}

pub async fn dispatch(app: &App, ctx: &Ctx, method: &str, params: Value) -> Res<Value> {
    let ws = || {
        ctx.ws
            .as_ref()
            .ok_or_else(|| RpcError::new(NOT_AVAILABLE, format!("{method} ใช้ได้เฉพาะผ่าน WebSocket (/v1/ws)")))
    };
    // บันทึกว่าใครสั่งอะไร (เฉพาะชื่อ method — ไม่บันทึก params ที่อาจมีความลับ; exec.run บันทึกคำสั่งเองใน exec.rs)
    if !matches!(method, "rpc.ping" | "pty.write" | "pty.resize" | "input.mouse_move") {
        tracing::info!(session = %ctx.session_id, method, "rpc");
    }
    match method {
        "rpc.ping" => Ok(json!({"pong": true})),

        "system.profile" => profile::get(parse(params)?).await,
        "system.network" => to_value(netif::list()),

        "exec.run" => to_value(exec::run(parse(params)?, app.config.mode).await?),

        "pty.open" => {
            let ws = ws()?;
            ws.ptys.open(parse(params)?, ws.out.clone(), app.config.mode)
        }
        "pty.write" => ws()?.ptys.write(parse(params)?).and_then(|_| ok()),
        "pty.resize" => ws()?.ptys.resize(parse(params)?).and_then(|_| ok()),
        "pty.close" => ws()?.ptys.close(parse(params)?).and_then(|_| ok()),

        "file.open_path" => browse::open_path(parse(params)?, &app.config.file_roots).await,

        "update.status" | "update.check" | "update.install" => crate::update::remote::rpc(app, method, params).await,

        "transfer.begin" | "transfer.finish" | "transfer.abort" | "transfer.stat" => {
            app.transfers.rpc(method, params).await
        }

        m if display::is_display_method(m) => {
            let result = app.display.call(app, m, params).await;
            if m.starts_with("input.")
                && m != "input.mouse_move"
                && let Err(error) = &result
                && error.code == display::SCREEN_INPUT_UNAVAILABLE
            {
                tracing::warn!(session = %ctx.session_id, method = m, reason = %error.message, "input unavailable");
            }
            result
        }

        "session.list" => Ok(json!({"current": ctx.session_id, "sessions": app.auth.list()})),
        "session.revoke" => {
            // อุปกรณ์หนึ่งเพิกถอนอุปกรณ์อื่นไม่ได้ — ไม่งั้นอุปกรณ์ที่ถูกขโมยไปตัดเจ้าของตัวจริงออกได้
            // (เพิกถอนตัวอื่นทำที่หน้าจัดการบนเครื่องปลายทาง /api/v1/local/devices/{id}/revoke)
            let target = parse::<RevokeParams>(params)?
                .session_id
                .unwrap_or_else(|| ctx.session_id.clone());
            if target != ctx.session_id {
                return Err(RpcError::invalid(
                    "เพิกถอนได้เฉพาะอุปกรณ์ของตัวเอง — อุปกรณ์อื่นให้เจ้าของเครื่องปลายทางเพิกถอนที่หน้าจัดการ",
                ));
            }
            tracing::info!(device = %ctx.session_id, "session.revoke (ตัวเอง)");
            Ok(json!({"revoked": app.auth.revoke(&target)}))
        }

        other => Err(RpcError::new(METHOD_NOT_FOUND, format!("ไม่มี method '{other}'"))),
    }
}
