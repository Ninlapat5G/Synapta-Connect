use synapta_connect::cli::{self, Command};
use tracing_subscriber::EnvFilter;

#[cfg(all(windows, feature = "desktop"))]
mod desktop;

fn main() {
    let command = cli::parse(std::env::args().skip(1).collect()).unwrap_or_else(|e| {
        eprintln!("{e}\n\n{}", cli::USAGE);
        std::process::exit(2);
    });
    std::process::exit(run(command));
}

/// คืน exit code
fn run(command: Command) -> i32 {
    match command {
        Command::Version => {
            println!("synapta-connect {}", env!("CARGO_PKG_VERSION"));
            0
        }
        Command::Agent(config) => block_on(async {
            synapta_connect::platform::init();
            init_console_log();
            match synapta_connect::run(config).await {
                Ok(()) => 0,
                Err(e) => {
                    eprintln!("agent หยุดทำงาน: {e:#}");
                    1
                }
            }
        }),
        Command::Desktop => launch_desktop(),
        Command::Panel {
            local_port,
            lan,
            enable_local,
            recover_web,
            link_only,
            gui,
        } => {
            #[cfg(all(windows, feature = "desktop"))]
            if gui && local_port.is_none() && lan.is_none() && !enable_local && !recover_web && !link_only {
                return launch_desktop();
            }
            block_on(async {
                match synapta_connect::open_panel::run(local_port, lan.as_deref(), enable_local, recover_web, link_only)
                    .await
                {
                    Ok(msg) => {
                        println!("{msg}");
                        0
                    }
                    Err(e) => {
                        eprintln!("{e}");
                        #[cfg(windows)]
                        if gui {
                            synapta_connect::platform::win::message_box("Synapta Connect", &e);
                        }
                        #[cfg(not(windows))]
                        let _ = gui;
                        1
                    }
                }
            })
        }
        Command::Helper => block_on(async {
            synapta_connect::platform::init(); // DPI เหมือน agent: ภาพกับเมาส์ต้องใช้พิกเซลเดียวกัน
            synapta_connect::helper::run().await.map_or(1, |_| 0)
        }),
        other => windows_only(other),
    }
}

fn launch_desktop() -> i32 {
    #[cfg(all(windows, feature = "desktop"))]
    {
        match desktop::run() {
            Ok(()) => return 0,
            Err(error) => {
                eprintln!("{error}");
                synapta_connect::platform::win::message_box(
                    "Synapta Connect",
                    &format!("เปิด Desktop ไม่สำเร็จ: {error}\nลองเปิด Web Panel ด้วยคำสั่ง synapta-connect panel"),
                );
            }
        }
    }
    #[cfg(not(all(windows, feature = "desktop")))]
    eprintln!("รุ่นนี้ไม่มี Synapta Connect Desktop ให้ใช้คำสั่ง synapta-connect panel");
    1
}

fn block_on(future: impl std::future::Future<Output = i32>) -> i32 {
    tokio::runtime::Runtime::new()
        .expect("สร้าง tokio runtime ไม่ได้")
        .block_on(future)
}

fn init_console_log() {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::try_from_default_env().unwrap_or_else(|_| "synapta_connect=info".into()))
        .init();
}

#[cfg(windows)]
fn windows_only(command: Command) -> i32 {
    use synapta_connect::winsvc::{self, install};
    match command {
        Command::ServiceRun => winsvc::run_agent_service().map_or_else(service_start_error, |_| 0),
        Command::UpdaterRun => winsvc::run_updater_service().map_or_else(service_start_error, |_| 0),
        Command::Watchdog => report(winsvc::watchdog()),
        Command::Install { options, plan_only } => {
            if plan_only {
                for (step, detail) in install::plan(&options) {
                    if options.json {
                        println!("{}", serde_json::json!({"step": step, "detail": detail}));
                    } else {
                        println!("- {detail}");
                    }
                }
                return 0;
            }
            install::install(&options)
        }
        Command::Uninstall { keep_data, json } => install::uninstall(keep_data, json),
        Command::Upgrade { json } => install::upgrade(json),
        Command::ServiceStart => report(winsvc::start_agent()),
        Command::ServiceStop => report(winsvc::stop_agent()),
        Command::ServiceStatus { json } => install::status(json),
        _ => unreachable!("คำสั่งอื่นจัดการใน run()"),
    }
}

#[cfg(windows)]
fn report(result: Result<(), String>) -> i32 {
    match result {
        Ok(()) => 0,
        Err(e) => {
            eprintln!("{e}");
            1
        }
    }
}

#[cfg(windows)]
fn service_start_error(e: windows_service::Error) -> i32 {
    eprintln!("คำสั่งนี้ให้ Windows (SCM) เรียกเท่านั้น ไม่ใช่สั่งมือ: {e}");
    1
}

#[cfg(not(windows))]
fn windows_only(_: Command) -> i32 {
    eprintln!("คำสั่งนี้ใช้ได้เฉพาะ Windows");
    2
}
