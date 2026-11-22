//! ไอคอน Synapta Connect ที่มุมขวาล่างของ Windows (notification area) ในบัญชีเจ้าของเครื่อง
//!
//! helper เป็นเจ้าของไอคอน เพราะอยู่ใน session ของเจ้าของตลอดการล็อกอิน (เริ่มจาก Run key)
//! คลิกซ้าย = เปิดแอป Synapta Connect · คลิกขวา = เมนูสั่งงานเร็ว ซึ่งส่งผ่านท่อเดียวกับ Desktop (`desktop_ipc`)
//! จึงทำได้เท่าที่ Desktop ทำได้พอดี และต้องอ่าน local_token ได้ (เฉพาะเจ้าของ) เหมือนกัน

use crate::desktop_ipc;
use serde_json::{Value, json};
use std::{
    sync::atomic::{AtomicBool, AtomicIsize, AtomicU32, Ordering},
    time::{Duration, Instant},
};
use windows::{
    Win32::{
        Foundation::{HWND, LPARAM, LRESULT, POINT, WPARAM},
        System::LibraryLoader::GetModuleHandleW,
        UI::{
            Shell::{
                ExtractIconExW, NIF_ICON, NIF_INFO, NIF_MESSAGE, NIF_TIP, NIIF_INFO, NIIF_WARNING, NIM_ADD, NIM_DELETE,
                NIM_MODIFY, NOTIFY_ICON_INFOTIP_FLAGS, NOTIFYICONDATAW, Shell_NotifyIconW,
            },
            WindowsAndMessaging::{
                AllowSetForegroundWindow, AppendMenuW, CreatePopupMenu, CreateWindowExW, DefWindowProcW, DestroyMenu,
                DestroyWindow, DispatchMessageW, FindWindowExW, GetClassNameW, GetCursorPos, GetMessageW, HICON, IDYES,
                IsIconic, MB_ICONWARNING, MB_SETFOREGROUND, MB_YESNO, MENU_ITEM_FLAGS, MF_CHECKED, MF_GRAYED,
                MF_SEPARATOR, MF_STRING, MSG, MessageBoxW, PostMessageW, PostQuitMessage, RegisterClassW,
                RegisterWindowMessageW, SW_RESTORE, SendMessageW, SetForegroundWindow, SetMenuDefaultItem, ShowWindow,
                TPM_NONOTIFY, TPM_RETURNCMD, TPM_RIGHTBUTTON, TrackPopupMenu, TranslateMessage, WINDOW_EX_STYLE,
                WINDOW_STYLE, WM_APP, WM_CLOSE, WM_CONTEXTMENU, WM_DESTROY, WM_LBUTTONUP, WM_NULL, WM_RBUTTONUP,
                WNDCLASSW,
            },
        },
    },
    core::{HSTRING, PCWSTR, w},
};

/// ชื่อหน้าต่างของแอป Desktop (tauri.conf.json) — ใช้หาหน้าต่างที่เปิดอยู่แล้วแทนการเปิดซ้ำ
const DESKTOP_TITLE: PCWSTR = w!("Synapta Connect");
const WM_TRAY: u32 = WM_APP + 1;
const ICON_ID: u32 = 1;

const ID_OPEN: u32 = 1;
const ID_WEB: u32 = 2;
const ID_SCREEN: u32 = 3;
const ID_KILL: u32 = 4;
const ID_UPDATE: u32 = 5;

static WINDOW: AtomicIsize = AtomicIsize::new(0);
static ICON: AtomicIsize = AtomicIsize::new(0);
static ONLINE: AtomicBool = AtomicBool::new(false);
/// Explorer ส่งข้อความนี้เมื่อ taskbar ถูกสร้างใหม่ (explorer ล่มแล้วเริ่มใหม่) — ต้องใส่ไอคอนกลับเอง
static TASKBAR_CREATED: AtomicU32 = AtomicU32::new(0);

/// แสดงไอคอนในเธรดของตัวเอง (ต้องมี message loop) — helper เรียกครั้งเดียวหลังยืนยันว่าเป็นเจ้าของ
pub fn start() {
    std::thread::spawn(|| {
        if let Err(error) = run() {
            tracing::warn!(%error, "แสดงไอคอนที่มุมจอไม่ได้");
        }
    });
}

/// เอาไอคอนออกก่อนโปรเซสจบ ไม่งั้นไอคอนผีค้างอยู่จนกว่าจะเอาเมาส์ไปชี้
pub fn stop() {
    let window = WINDOW.swap(0, Ordering::AcqRel);
    if window != 0 {
        unsafe {
            SendMessageW(HWND(window as _), WM_CLOSE, None, None);
        }
    }
}

/// สถานะ Agent ที่แสดงตอนเอาเมาส์ชี้ไอคอน — helper บอกเมื่อต่อ/หลุดจาก Agent
pub fn set_online(online: bool) {
    if ONLINE.swap(online, Ordering::AcqRel) != online {
        let window = WINDOW.load(Ordering::Acquire);
        if window != 0 {
            let mut data = icon_data(HWND(window as _));
            data.uFlags = NIF_TIP;
            unsafe {
                let _ = Shell_NotifyIconW(NIM_MODIFY, &data);
            }
        }
    }
}

/// ดึงหน้าต่าง Desktop ที่เปิดอยู่ขึ้นมาข้างหน้า — false ถ้ายังไม่มีหน้าต่างเปิด
pub fn focus_desktop() -> bool {
    let mut after = None;
    loop {
        let Ok(window) = (unsafe { FindWindowExW(None, after, PCWSTR::null(), DESKTOP_TITLE) }) else {
            return false;
        };
        // MessageBox ของโปรแกรมใช้ชื่อเดียวกัน (class #32770) — ไม่ใช่หน้าต่างแอป
        let mut class = [0u16; 16];
        let length = unsafe { GetClassNameW(window, &mut class) }.max(0) as usize;
        if String::from_utf16_lossy(&class[..length]) != "#32770" {
            unsafe {
                if IsIconic(window).as_bool() {
                    let _ = ShowWindow(window, SW_RESTORE);
                }
                let _ = SetForegroundWindow(window);
            }
            return true;
        }
        after = Some(window);
    }
}

fn run() -> windows::core::Result<()> {
    unsafe {
        let instance = GetModuleHandleW(None)?;
        let class = w!("SynaptaConnectTray");
        let window_class = WNDCLASSW {
            lpfnWndProc: Some(window_proc),
            hInstance: instance.into(),
            lpszClassName: class,
            ..Default::default()
        };
        RegisterClassW(&window_class);
        TASKBAR_CREATED.store(RegisterWindowMessageW(w!("TaskbarCreated")), Ordering::Release);
        // หน้าต่างธรรมดาที่ไม่แสดง (ไม่ใช่ message-only) เพราะต้องได้รับ TaskbarCreated ที่ส่งถึงหน้าต่างระดับบนสุด
        let window = CreateWindowExW(
            WINDOW_EX_STYLE(0),
            class,
            w!("Synapta Connect Tray"),
            WINDOW_STYLE(0),
            0,
            0,
            0,
            0,
            None,
            None,
            Some(instance.into()),
            None,
        )?;
        ICON.store(load_icon().0 as isize, Ordering::Release);
        WINDOW.store(window.0 as isize, Ordering::Release);
        add_icon(window);

        let mut message = MSG::default();
        while GetMessageW(&mut message, None, 0, 0).as_bool() {
            let _ = TranslateMessage(&message);
            DispatchMessageW(&message);
        }
    }
    Ok(())
}

fn load_icon() -> HICON {
    let mut small = HICON::default();
    if let Ok(exe) = std::env::current_exe() {
        unsafe {
            ExtractIconExW(&HSTRING::from(exe.as_os_str()), 0, None, Some(&mut small), 1);
        }
    }
    small
}

fn icon_data(window: HWND) -> NOTIFYICONDATAW {
    let mut data = NOTIFYICONDATAW {
        cbSize: std::mem::size_of::<NOTIFYICONDATAW>() as u32,
        hWnd: window,
        uID: ICON_ID,
        uFlags: NIF_ICON | NIF_MESSAGE | NIF_TIP,
        uCallbackMessage: WM_TRAY,
        hIcon: HICON(ICON.load(Ordering::Acquire) as _),
        ..Default::default()
    };
    let status = if ONLINE.load(Ordering::Acquire) {
        "Agent ทำงานอยู่"
    } else {
        "กำลังรอ Agent"
    };
    fill(
        &mut data.szTip,
        &format!("Synapta Connect {} — {status}", env!("CARGO_PKG_VERSION")),
    );
    data
}

fn add_icon(window: HWND) {
    unsafe {
        let _ = Shell_NotifyIconW(NIM_ADD, &icon_data(window));
    }
}

/// ข้อความแจ้งเตือนเล็กเหนือไอคอน — ใช้บอกผลของคำสั่งจากเมนู
fn notify(title: &str, text: &str, warning: bool) {
    let window = WINDOW.load(Ordering::Acquire);
    if window == 0 {
        return;
    }
    let mut data = icon_data(HWND(window as _));
    data.uFlags = NIF_INFO;
    data.dwInfoFlags = if warning { NIIF_WARNING } else { NIIF_INFO } | NOTIFY_ICON_INFOTIP_FLAGS(0);
    fill(&mut data.szInfoTitle, title);
    fill(&mut data.szInfo, text);
    unsafe {
        let _ = Shell_NotifyIconW(NIM_MODIFY, &data);
    }
}

/// คัดข้อความลงช่องขนาดคงที่ของ Windows (ตัดส่วนเกิน และเหลือที่ให้ตัวจบสตริง)
fn fill(target: &mut [u16], text: &str) {
    let units: Vec<u16> = text.encode_utf16().take(target.len() - 1).collect();
    target[..units.len()].copy_from_slice(&units);
    target[units.len()] = 0;
}

unsafe extern "system" fn window_proc(window: HWND, message: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    match message {
        WM_TRAY => {
            match lparam.0 as u32 {
                WM_LBUTTONUP => open_desktop(),
                WM_RBUTTONUP | WM_CONTEXTMENU => show_menu(window),
                _ => {}
            }
            LRESULT(0)
        }
        WM_CLOSE => {
            unsafe {
                let _ = DestroyWindow(window);
            }
            LRESULT(0)
        }
        WM_DESTROY => {
            unsafe {
                let _ = Shell_NotifyIconW(NIM_DELETE, &icon_data(window));
                PostQuitMessage(0);
            }
            LRESULT(0)
        }
        _ if message != 0 && message == TASKBAR_CREATED.load(Ordering::Acquire) => {
            add_icon(window);
            LRESULT(0)
        }
        _ => unsafe { DefWindowProcW(window, message, wparam, lparam) },
    }
}

fn show_menu(window: HWND) {
    // อ่านสถานะสวิตช์ก่อนแสดงเมนู ให้เครื่องหมายถูกตรงกับของจริง (Agent ไม่ตอบ = กดไม่ได้)
    let screen_input = call("GET", "screen-input", None)
        .ok()
        .and_then(|reply| reply["enabled"].as_bool());
    let agent = if screen_input.is_some() {
        MF_STRING
    } else {
        MF_STRING | MF_GRAYED
    };
    unsafe {
        let Ok(menu) = CreatePopupMenu() else { return };
        let add = |flags: MENU_ITEM_FLAGS, id: u32, text: &str| {
            let _ = AppendMenuW(menu, flags, id as usize, &HSTRING::from(text));
        };
        add(MF_STRING, ID_OPEN, "เปิด Synapta Connect");
        add(MF_STRING, ID_WEB, "เปิด Web Panel ในเบราว์เซอร์");
        add(MF_SEPARATOR, 0, "");
        let checked = if screen_input == Some(true) {
            MF_CHECKED
        } else {
            MENU_ITEM_FLAGS(0)
        };
        add(agent | checked, ID_SCREEN, "อนุญาตให้จับภาพและส่ง input");
        add(agent, ID_KILL, "ตัดการเชื่อมต่อทั้งหมด");
        add(agent, ID_UPDATE, "ตรวจอัปเดต");
        let _ = SetMenuDefaultItem(menu, ID_OPEN, 0);

        let mut point = POINT::default();
        let _ = GetCursorPos(&mut point);
        // เมนูของไอคอนมุมจอต้องให้หน้าต่างเจ้าของอยู่หน้าสุดก่อน ไม่งั้นคลิกที่อื่นแล้วเมนูไม่ปิด
        let _ = SetForegroundWindow(window);
        let chosen = TrackPopupMenu(
            menu,
            TPM_RIGHTBUTTON | TPM_RETURNCMD | TPM_NONOTIFY,
            point.x,
            point.y,
            None,
            window,
            None,
        )
        .0 as u32;
        let _ = PostMessageW(Some(window), WM_NULL, WPARAM(0), LPARAM(0));
        let _ = DestroyMenu(menu);
        run_command(chosen, screen_input);
    }
}

fn run_command(id: u32, screen_input: Option<bool>) {
    match id {
        ID_OPEN => open_desktop(),
        ID_WEB => background(|| {
            if let Err(error) = block_on(crate::open_panel::run(None, None, false, false, false)) {
                notify("เปิด Web Panel ไม่สำเร็จ", &error, true);
            }
        }),
        ID_SCREEN => {
            let enable = screen_input != Some(true);
            background(
                move || match call("PUT", "screen-input", Some(json!({ "enabled": enable }))) {
                    Ok(_) if enable => notify("Synapta Connect", "อนุญาตให้จับภาพและส่ง input แล้ว", false),
                    Ok(_) => notify("Synapta Connect", "ปิดการจับภาพและส่ง input แล้ว", false),
                    Err(error) => notify("เปลี่ยนสิทธิ์ไม่สำเร็จ", &error, true),
                },
            );
        }
        ID_KILL if confirm("ตัดการเชื่อมต่อของทุกอุปกรณ์ที่ต่ออยู่ตอนนี้?") =>
        {
            background(|| match call("POST", "kill-all", None) {
                Ok(_) => notify("Synapta Connect", "ตัดการเชื่อมต่อทั้งหมดแล้ว", false),
                Err(error) => notify("ตัดการเชื่อมต่อไม่สำเร็จ", &error, true),
            });
        }
        ID_UPDATE => background(check_update),
        _ => {}
    }
}

fn check_update() {
    if let Err(error) = call("POST", "update/check", None) {
        // กำลังตรวจอยู่แล้วก็ยังรอผลรอบนั้นต่อได้
        tracing::debug!(%error, "สั่งตรวจอัปเดตจากเมนูไอคอน");
    }
    let deadline = Instant::now() + Duration::from_secs(90);
    loop {
        std::thread::sleep(Duration::from_secs(2));
        let status = match call("GET", "update", None) {
            Ok(status) => status,
            Err(error) => return notify("ตรวจอัปเดตไม่สำเร็จ", &error, true),
        };
        match status["state"].as_str().unwrap_or_default() {
            "checking" if Instant::now() < deadline => continue,
            "update_available" => {
                let version = status["available"]["version"].as_str().unwrap_or("ใหม่");
                return notify(
                    "มีอัปเดต Synapta Connect",
                    &format!("รุ่น {version} พร้อมติดตั้ง เปิด Synapta Connect แล้วไปที่หน้าอัปเดต"),
                    false,
                );
            }
            "up_to_date" => return notify("Synapta Connect", "เป็นรุ่นล่าสุดแล้ว", false),
            _ => {
                let message = status["last_error"]["message"].as_str().unwrap_or("ยังไม่ได้ผลการตรวจ");
                return notify("ตรวจอัปเดตไม่สำเร็จ", message, true);
            }
        }
    }
}

/// เปิดแอป Desktop — ถ้าเปิดอยู่แล้วดึงหน้าต่างเดิมขึ้นมาแทนการเปิดซ้ำ
fn open_desktop() {
    if focus_desktop() {
        return;
    }
    use std::os::windows::process::CommandExt;
    const DETACHED_PROCESS: u32 = 0x0000_0008;
    let exe = crate::winsvc::install::install_dir().join(crate::update::installer::EXE_NAME);
    unsafe {
        // คลิกไอคอนทำให้โปรเซสนี้มีสิทธิ์ดึงหน้าต่างขึ้นหน้า — ส่งสิทธิ์ต่อให้แอปที่กำลังจะเปิด
        let _ = AllowSetForegroundWindow(u32::MAX);
    }
    if let Err(error) = std::process::Command::new(exe)
        .arg("desktop")
        .creation_flags(DETACHED_PROCESS)
        .spawn()
    {
        notify("เปิด Synapta Connect ไม่สำเร็จ", &error.to_string(), true);
    }
}

fn confirm(question: &str) -> bool {
    unsafe {
        MessageBoxW(
            None,
            &HSTRING::from(question),
            w!("Synapta Connect"),
            MB_YESNO | MB_ICONWARNING | MB_SETFOREGROUND,
        ) == IDYES
    }
}

/// คำสั่งที่ต้องรอ Agent ไม่ทำในเธรดของไอคอน ไม่งั้นเมนูและการคลิกค้างระหว่างรอ
fn background(job: impl FnOnce() + Send + 'static) {
    std::thread::spawn(job);
}

fn block_on<T>(future: impl Future<Output = T>) -> T {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("สร้าง tokio runtime ไม่ได้")
        .block_on(future)
}

/// เรียก API ของ Agent ผ่านท่อเดียวกับแอป Desktop
fn call(method: &str, path: &str, body: Option<Value>) -> Result<Value, String> {
    let reply = block_on(desktop_ipc::request(
        method.into(),
        format!("/api/v1/local/{path}"),
        body,
    ))
    .map_err(|error| format!("เชื่อมต่อ Agent ไม่ได้: {error}"))?;
    if reply.status >= 400 {
        let message = reply.data["error"]["message"].as_str().unwrap_or("Agent ปฏิเสธคำขอ");
        return Err(message.to_string());
    }
    Ok(reply.data)
}

#[cfg(test)]
mod tests {
    use super::fill;

    #[test]
    fn long_thai_text_is_cut_to_fit_and_stays_terminated() {
        let mut target = [1u16; 8];
        fill(&mut target, "ตัดการเชื่อมต่อทั้งหมดแล้ว");
        assert_eq!(target[7], 0);
        assert_eq!(String::from_utf16(&target[..7]).unwrap(), "ตัดการเ");
    }
}
