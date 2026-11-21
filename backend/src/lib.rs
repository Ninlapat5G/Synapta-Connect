//! Synapta Connect agent — ตัวรับคำสั่งบนเครื่องปลายทาง (ดู docs/API.md)

pub mod api;
pub mod app;
pub mod auth;
pub mod cli;
pub mod config;
pub mod discovery;
pub mod display;
pub mod exec;
pub mod file_access;
pub mod fsutil;
pub mod helper;
pub mod hex;
pub mod http;
pub mod input;
pub mod kill_tree;
pub mod local;
pub mod netif;
pub mod open_panel;
pub mod pairing;
pub mod panel;
pub mod platform;
pub mod profile;
pub mod proof;
pub mod pty;
pub mod rpc;
pub mod screen;
pub mod settings;
pub mod tls;
pub mod transfer;
pub mod update;
#[cfg(windows)]
pub mod winsvc;

use config::{Config, Mode};
use std::{future::Future, net::SocketAddr};

/// รัน agent จนกว่าจะกด Ctrl+C
pub async fn run(config: Config) -> anyhow::Result<()> {
    run_until(config, async {
        let _ = tokio::signal::ctrl_c().await;
    })
    .await
}

/// รัน agent จนกว่า `shutdown` จะเสร็จ (service ใช้สัญญาณหยุดจาก SCM)
pub async fn run_until(config: Config, shutdown: impl Future<Output = ()>) -> anyhow::Result<()> {
    let app = app::AppState::new(config)?;
    let tls_listener = tokio::net::TcpListener::bind(("0.0.0.0", app.config.port)).await?;
    let local_listener = tokio::net::TcpListener::bind(("127.0.0.1", app.config.local_port)).await?;
    let _discovery = discovery::start(&app).await;

    match app.config.mode {
        Mode::Console => announce_console(&app),
        Mode::Service => {
            // service ไม่เปิดโหมดจับคู่เอง (เจ้าของสั่งที่ panel เท่านั้น) และไม่มีใครเห็น console
            tracing::info!(version = env!("CARGO_PKG_VERSION"), host_key = %app.host.fingerprint(), "service เริ่มทำงาน");
            app.updates.ingest_result(); // ผลของการอัปเดตที่เพิ่งจบ (รวมกรณีเพิ่งถูกกู้คืน)
            tokio::spawn(app.updates.clone().run_scheduler());
        }
    }

    let local = axum::serve(
        local_listener,
        local::router(app.clone()).into_make_service_with_connect_info::<SocketAddr>(),
    );
    let tls_config = tls::server_config(&app)?;
    tokio::select! {
        r = local => r?,
        _ = tls::serve(tls_listener, tls_config, http::router(app.clone()), app) => {}
        _ = shutdown => {}
    }
    Ok(())
}

fn announce_console(app: &app::App) {
    // println ไม่ใช่ tracing: OTP ต้องแสดงให้เจ้าของเครื่องเห็นแต่ไม่ควรลงไฟล์ log
    let otp = app.auth.open_pairing(); // เปิดเอเจนต์ด้วยมือ = เจ้าของเครื่องอยู่หน้าจอ จึงเปิดโหมดจับคู่ให้ครั้งแรก
    println!("Synapta Connect agent {} ({})", app.config.name, app.node_id);
    println!("  host key: {}", app.host.fingerprint());
    for i in netif::list() {
        println!(
            "  https://{}  [{} / {}]",
            SocketAddr::new(i.ip, app.config.port),
            i.name,
            i.kind
        );
    }
    println!(
        "  หน้าจัดการบนเครื่องนี้: http://127.0.0.1:{}/api/v1/local/status  (แนบ header X-Synapta-Local = ข้อความใน {})",
        app.config.local_port,
        app.config.data_dir.join("local_token").display()
    );
    println!("  OTP สำหรับจับคู่: {}  (หมดอายุใน 2 นาที)", otp.otp);
}
