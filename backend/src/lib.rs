//! Synapta Connect agent — ตัวรับคำสั่งบนเครื่องปลายทาง (ดู docs/API.md)

pub mod api;
pub mod app;
pub mod auth;
pub mod browse;
pub mod cli;
pub mod config;
#[cfg(windows)]
pub mod desktop_ipc;
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
#[cfg(windows)]
mod legacy_pty;
pub mod local;
pub mod netif;
pub mod network;
pub mod open_panel;
pub mod pairing;
pub mod panel;
pub mod panel_https;
pub mod platform;
pub mod profile;
pub mod proof;
pub mod pty;
pub mod rpc;
pub mod screen;
pub mod settings;
#[cfg(windows)]
pub mod shortcut;
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
    let _ = rustls::crypto::ring::default_provider().install_default();
    let app = app::AppState::new(config)?;
    #[cfg(windows)]
    legacy_pty::set_data_dir(&app.config.data_dir);
    let listeners = network::Listeners::start(&app).await?;

    match app.config.mode {
        Mode::Console => announce_console(&app),
        Mode::Service => {
            // service ไม่เปิดโหมดจับคู่เอง (เจ้าของสั่งที่ panel เท่านั้น) และไม่มีใครเห็น console
            tracing::info!(version = env!("CARGO_PKG_VERSION"), host_key = %app.host.fingerprint(), "service เริ่มทำงาน");
            app.updates.ingest_result(); // ผลของการอัปเดตที่เพิ่งจบ (รวมกรณีเพิ่งถูกกู้คืน)
            tokio::spawn(app.updates.clone().run_scheduler());
            tokio::spawn(app.web_updates.clone().run_scheduler());
            #[cfg(windows)]
            tokio::spawn(winsvc::migrate_firewall(app.config.data_dir.clone()));
        }
    }

    let control = async {
        #[cfg(windows)]
        if app.config.mode == Mode::Service {
            desktop_ipc::serve(app.clone()).await;
        }
        std::future::pending::<()>().await;
    };
    tokio::select! {
        result = listeners.supervise(app.clone()) => result,
        _ = control => unreachable!(),
        _ = shutdown => Ok(()),
    }
}

fn announce_console(app: &app::App) {
    // println ไม่ใช่ tracing: OTP ต้องแสดงให้เจ้าของเครื่องเห็นแต่ไม่ควรลงไฟล์ log
    let otp = app.auth.open_pairing(); // เปิดเอเจนต์ด้วยมือ = เจ้าของเครื่องอยู่หน้าจอ จึงเปิดโหมดจับคู่ให้ครั้งแรก
    println!("Synapta Connect agent {} ({})", app.config.name, app.node_id);
    println!("  host key: {}", app.host.fingerprint());
    for i in netif::list() {
        println!(
            "  https://{}  [{} / {}]",
            SocketAddr::new(i.ip, app.network.state().ports.port),
            i.name,
            i.kind
        );
    }
    println!(
        "  หน้าจัดการบนเครื่องนี้: http://127.0.0.1:{}/synaptaconnect  (เปิดด้วย synapta-connect panel)",
        app.network.state().ports.local_port,
    );
    println!("  OTP สำหรับจับคู่: {}  (หมดอายุใน 2 นาที)", otp.otp);
}
