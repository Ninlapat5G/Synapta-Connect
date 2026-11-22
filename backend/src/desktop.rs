use serde_json::Value;
use std::sync::{
    Once,
    atomic::{AtomicBool, Ordering},
};
use synapta_connect::{desktop_ipc, helper, open_panel, winsvc};
use tauri::Manager;

#[tauri::command]
async fn agent_request(method: String, path: String, body: Option<Value>) -> Result<desktop_ipc::Reply, String> {
    let status = method == "GET" && path == "/api/v1/local/status";
    match desktop_ipc::request(method, path, body).await {
        Ok(reply) => {
            if status && reply.status == 200 {
                static HELPER: Once = Once::new();
                HELPER.call_once(helper::start_in_this_session);
            }
            Ok(reply)
        }
        Err(error) if error.kind() == std::io::ErrorKind::PermissionDenied => Ok(desktop_ipc::Reply {
            status: 403,
            data: serde_json::json!({"error": {"message": "ตรวจสอบสิทธิ์บัญชี Windows ที่ตั้งไว้กับ Agent"}}),
        }),
        Err(error) => Err(format!("เชื่อมต่อ Agent ไม่ได้: {error}")),
    }
}

#[tauri::command]
async fn service_status() -> Result<Value, String> {
    tauri::async_runtime::spawn_blocking(winsvc::desktop_status)
        .await
        .map_err(|error| error.to_string())
}

#[tauri::command]
async fn service_control(action: String) -> Result<(), String> {
    static BUSY: AtomicBool = AtomicBool::new(false);
    tauri::async_runtime::spawn_blocking(move || {
        BUSY.compare_exchange(false, true, Ordering::Acquire, Ordering::Relaxed)
            .map_err(|_| "กำลังควบคุม Agent อยู่")?;
        let result = winsvc::desktop_control(&action);
        BUSY.store(false, Ordering::Release);
        result
    })
    .await
    .map_err(|error| error.to_string())?
}

#[tauri::command]
async fn open_web() -> Result<(), String> {
    open_panel::run(None, None, false, false, false).await.map(|_| ())
}

pub fn run() -> tauri::Result<()> {
    use windows::Win32::System::Console::{FreeConsole, GetConsoleProcessList};

    // Leave a parent terminal alone and detach the private console created by a shortcut.
    if unsafe { GetConsoleProcessList(&mut [0; 2]) } == 1 {
        unsafe {
            let _ = FreeConsole();
        }
    }
    tauri::Builder::default()
        .setup(|app| {
            // The desktop shortcut uses "Run: Minimized" to hide the console, and Windows applies that state to the
            // first ShowWindow of this process too, so the app opened minimized and looked like it never started.
            // The window is created hidden (tauri.conf.json) and shown here, then restored and focused explicitly.
            if let Some(window) = app.get_webview_window("main") {
                let _ = window.show();
                let _ = window.unminimize();
                let _ = window.set_focus();
            }
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            agent_request,
            service_status,
            service_control,
            open_web
        ])
        .run(tauri::generate_context!())
}
