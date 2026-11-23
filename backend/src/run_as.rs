//! คำสั่งจากระยะไกล (exec.run / pty.*) รันด้วยบัญชีไหน — จุดเดียวที่ตัดสินเรื่องนี้
//!
//! service รันเป็น `NT SERVICE\SynaptaConnect` ใน Session 0 ซึ่งไม่ใช่บัญชีของเจ้าของ: เขียนโฟลเดอร์ที่เจ้าของติดตั้งโปรแกรมไว้
//! ไม่ได้ และเห็นโปรไฟล์/ค่าตั้งคนละชุด — เคยทำให้ `flutter doctor` ค้างไม่รู้จบ (flutter.bat วนรอเปิดไฟล์ล็อกใน
//! `bin\cache` ที่บัญชี service เขียนไม่ได้) ทั้งที่หน้าเครื่องใช้ได้ปกติ
//!
//! ค่าเริ่มต้นจึงส่งคำสั่งให้ helper ใน session ของเจ้าของรันแทน = บัญชี, PATH และสิทธิ์เดียวกับหน้าเครื่อง
//! ไม่มี helper (ไม่มีใครล็อกอิน) → รันในบัญชี service เหมือนเดิม · ผลของ exec บอกเสมอว่ารันด้วยบัญชีไหน (`run_as`)
//!
//! ขอบเขตความปลอดภัย: ผู้ที่สั่ง exec ได้ = ทำได้ทุกอย่างที่เจ้าของทำได้ (เท่ากับสิทธิ์ควบคุมหน้าจอที่มีอยู่แล้ว)
//! เจ้าของปิดได้ที่ panel (`commands.run_as_owner`) และเลือกได้ว่าต้องเป็น administrator (`commands.administrator`)

use crate::{
    api::WsCtx,
    app::App,
    config::Mode,
    exec, hex,
    rpc::{INTERNAL, Res, RpcError, parse, to_value},
};
use serde_json::{Value, json};
use std::time::Duration;

/// method ที่ helper ยอมรันให้ในฐานะเจ้าของ (นอกจากจับภาพ/input) — ที่เหลือถูกปฏิเสธแม้ service ถูกหลอกให้ส่งมา
pub fn is_command_method(method: &str) -> bool {
    matches!(
        method,
        "exec.run" | "pty.open" | "pty.write" | "pty.resize" | "pty.close" | "helper.elevate"
    )
}

const WHAT: &str = "รันคำสั่งด้วยบัญชีเจ้าของเครื่อง";
/// เผื่อเวลาให้ helper ส่งผลกลับ นอกเหนือจากเวลาที่คำสั่งรันได้
const EXEC_SLACK: Duration = Duration::from_secs(15);
const PTY_TIMEOUT: Duration = Duration::from_secs(20);
/// เวลาให้เจ้าของกดยืนยัน UAC บนหน้าจอ แล้ว helper ตัวใหม่ (administrator) ต่อเข้ามา
const UAC_TIMEOUT: Duration = Duration::from_secs(150);
const ELEVATED_CONNECT_TIMEOUT: Duration = Duration::from_secs(30);

enum Target {
    /// โปรเซสนี้ (console = เจ้าของรันเองอยู่แล้ว · service = บัญชี service)
    Here,
    Owner,
}

async fn target(app: &App) -> Res<Target> {
    if app.config.mode == Mode::Console {
        return Ok(Target::Here);
    }
    let settings = app.settings.get().commands;
    if !settings.run_as_owner {
        return Ok(Target::Here);
    }
    let Some(helper) = app.display.helper_info() else {
        if settings.administrator {
            // บัญชี service ไม่ใช่ administrator — รันไปก็ไม่ตรงกับที่เจ้าของตั้งไว้
            return Err(RpcError::new(
                INTERNAL,
                "เจ้าของตั้งให้รันคำสั่งระดับ administrator แต่ไม่มีใครล็อกอินที่เครื่อง (ไม่มี helper) — ล็อกอินเครื่องปลายทางก่อน",
            ));
        }
        return Ok(Target::Here);
    };
    if helper.version.is_empty() {
        // helper รุ่นเก่าที่ค้างอยู่จากก่อนอัปเดต รันคำสั่งให้ไม่ได้ — ใช้บัญชี service ไปก่อนจนกว่ามันจะถูกแทน
        if settings.administrator {
            return Err(RpcError::new(
                INTERNAL,
                "helper บนเครื่องปลายทางยังเป็นรุ่นเก่า รันคำสั่งระดับ administrator ไม่ได้ — ล็อกเอาต์แล้วล็อกอินใหม่ที่เครื่องหนึ่งครั้ง",
            ));
        }
        return Ok(Target::Here);
    }
    if settings.administrator && !helper.elevated {
        elevate(app).await?;
    }
    Ok(Target::Owner)
}

/// ขอให้ helper เริ่มตัวเองใหม่ด้วยสิทธิ์ administrator (หน้าต่าง UAC ขึ้นบนเครื่องปลายทาง) แล้วรอตัวใหม่ต่อเข้ามา
pub async fn elevate(app: &App) -> Res<()> {
    // ทีละคำขอ: คำสั่งที่มาพร้อมกันรอผลของหน้าต่าง UAC เดียวกัน ไม่เปิดซ้อนหลายหน้าต่าง
    static ONE_AT_A_TIME: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());
    let _turn = ONE_AT_A_TIME.lock().await;
    let elevated = || app.display.helper_info().is_some_and(|h| h.elevated);
    if elevated() {
        return Ok(());
    }
    tracing::info!("ขอสิทธิ์ administrator ให้ helper (UAC บนเครื่องปลายทาง)");
    app.display
        .helper_call("helper.elevate", json!({}), UAC_TIMEOUT, "ขอสิทธิ์ administrator")
        .await?;
    let deadline = tokio::time::Instant::now() + ELEVATED_CONNECT_TIMEOUT;
    while tokio::time::Instant::now() < deadline {
        if elevated() {
            return Ok(());
        }
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
    Err(RpcError::new(
        INTERNAL,
        "ยืนยัน UAC แล้วแต่ helper ระดับ administrator ไม่ต่อเข้ามา — ลองสั่งใหม่อีกครั้ง",
    ))
}

pub async fn exec(app: &App, params: Value) -> Res<Value> {
    match target(app).await? {
        Target::Here => to_value(exec::run(parse(params)?, app.config.mode).await?),
        Target::Owner => {
            let request: exec::Params = parse(params.clone())?; // ตรวจรูปแบบก่อนรบกวน helper
            let wait = exec::time_limit(request.timeout_ms) + EXEC_SLACK;
            app.display.helper_call("exec.run", params, wait, WHAT).await
        }
    }
}

pub async fn pty_open(app: &App, ws: &WsCtx, params: Value) -> Res<Value> {
    match target(app).await? {
        Target::Here => ws.ptys.open(parse(params)?, ws.out.clone(), app.config.mode),
        Target::Owner => {
            ws.ptys.check_room()?;
            parse::<crate::pty::OpenParams>(params.clone())?;
            // จองเส้นทาง output ก่อนให้ helper เปิด — prompt แรกของ shell มาถึงเร็วมาก ห้ามหาย
            let id = hex::random_id(4);
            ws.ptys.adopt_owner(&id);
            app.display.route_pty(&id, ws.out.clone(), ws.ptys.clone());
            let request = json!({"pty_id": id, "open": params});
            match app.display.helper_call("pty.open", request, PTY_TIMEOUT, WHAT).await {
                Ok(_) => Ok(json!({"pty_id": id})),
                Err(e) => {
                    app.display.unroute_pty(&id);
                    Err(e)
                }
            }
        }
    }
}

/// pty.write / resize / close — ส่งไปที่ที่ terminal นั้นเปิดอยู่จริง
pub async fn pty_control(app: &App, ws: &WsCtx, method: &str, params: Value) -> Res<Value> {
    let id = params["pty_id"].as_str().unwrap_or_default();
    if ws.ptys.is_owner(id) == Some(true) {
        return app.display.helper_call(method, params, PTY_TIMEOUT, WHAT).await;
    }
    match method {
        "pty.write" => ws.ptys.write(parse(params)?),
        "pty.resize" => ws.ptys.resize(parse(params)?),
        _ => ws.ptys.close(parse(params)?),
    }?;
    Ok(json!({"ok": true}))
}

/// connection ของ client ปิด: ปิด terminal ที่ helper เปิดไว้ให้มันด้วย (ของโปรเซสนี้ Registry ฆ่าเองแล้ว)
pub fn close_owner_ptys(app: &App, ids: Vec<String>) {
    for id in ids {
        app.display.unroute_pty(&id);
        let app = app.clone();
        tokio::spawn(async move {
            let _ = app
                .display
                .helper_call("pty.close", json!({"pty_id": id}), PTY_TIMEOUT, WHAT)
                .await;
        });
    }
}
