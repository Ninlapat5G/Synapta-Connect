use super::{App, system_tool};
use std::os::windows::process::CommandExt;
use tokio::task::JoinHandle;
use windows::{
    Win32::{
        Foundation::{HWND, LPARAM, LRESULT, WPARAM},
        Graphics::Gdi::{DEFAULT_GUI_FONT, GetStockObject, HBRUSH},
        System::{LibraryLoader::GetModuleHandleW, Threading::CREATE_NO_WINDOW},
        UI::WindowsAndMessaging::*,
    },
    core::{PCWSTR, w},
};

const FIREWALL_MENU: usize = 0x1F10;
const TIMER_ID: usize = 1;
const PORTABLE_ICON: PCWSTR = PCWSTR(101_usize as *const u16);

fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain(Some(0)).collect()
}

pub(super) fn run(app: &App, supervisor: &JoinHandle<anyhow::Result<()>>) -> anyhow::Result<()> {
    let instance = unsafe { GetModuleHandleW(None)? }.into();
    let class = w!("SynaptaConnectPortableWindow");
    let title = w!("Synapta Connect Portable");
    let style = WS_CAPTION | WS_SYSMENU | WS_MINIMIZEBOX;
    let background = HBRUSH((windows::Win32::Graphics::Gdi::COLOR_WINDOW.0 as usize + 1) as _);
    let window = unsafe {
        let class_info = WNDCLASSW {
            hInstance: instance,
            hCursor: LoadCursorW(None, IDC_ARROW)?,
            hIcon: LoadIconW(Some(instance), PORTABLE_ICON)?,
            hbrBackground: background,
            lpfnWndProc: Some(window_proc),
            lpszClassName: class,
            ..Default::default()
        };
        if RegisterClassW(&class_info) == 0 {
            return Err(windows::core::Error::from_thread().into());
        }
        CreateWindowExW(
            WINDOW_EX_STYLE::default(),
            class,
            title,
            style,
            CW_USEDEFAULT,
            CW_USEDEFAULT,
            400,
            230,
            None,
            None,
            Some(instance),
            None,
        )?
    };
    let label_name = Control::label("ชื่อเครื่อง", 22, 18);
    let field_name = Control::field(&app.config.name, 22, 40);
    let label_code = Control::label("รหัสจับคู่", 22, 80);
    let field_code = Control::field("", 22, 102);
    let status = Control::label("", 22, 143);
    unsafe {
        label_name.create(window)?;
        field_name.create(window)?;
        label_code.create(window)?;
        let code = field_code.create(window)?;
        let status = status.create(window)?;
        let menu = GetSystemMenu(window, false);
        if !menu.is_invalid() {
            AppendMenuW(menu, MF_STRING, FIREWALL_MENU, w!("ตั้งค่า Windows Firewall"))?;
        }
        if SetTimer(Some(window), TIMER_ID, 250, None) == 0 {
            return Err(windows::core::Error::from_thread().into());
        }
        let mut display = (String::new(), String::new());
        refresh(app, code, status, &mut display)?;
        let _ = ShowWindow(window, SW_SHOW);
        let mut msg = MSG::default();
        loop {
            match GetMessageW(&mut msg, None, 0, 0).0 {
                -1 => return Err(windows::core::Error::from_thread().into()),
                0 => return Ok(()),
                _ => {}
            }
            if msg.message == WM_TIMER && msg.hwnd == window && msg.wParam.0 == TIMER_ID {
                if supervisor.is_finished() {
                    DestroyWindow(window)?;
                    return Ok(());
                }
                refresh(app, code, status, &mut display)?;
            }
            if !IsDialogMessageW(window, &msg).as_bool() {
                let _ = TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
        }
    }
}

fn refresh(app: &App, code: HWND, status: HWND, current: &mut (String, String)) -> windows::core::Result<()> {
    let otp = app.auth.portable_otp().map(|otp| otp.otp).unwrap_or_default();
    let message = app
        .auth
        .list()
        .into_iter()
        .next()
        .map(|device| {
            if app.auth.portable_attached(&device.id) {
                format!("กำลังเชื่อมต่อกับ {}", device.client_name)
            } else {
                format!("{} กำลังจับคู่", device.client_name)
            }
        })
        .unwrap_or_default();
    for (field, new, old) in [(code, otp, &mut current.0), (status, message, &mut current.1)] {
        if new != *old {
            unsafe { SetWindowTextW(field, PCWSTR(wide(&new).as_ptr()))? };
            *old = new;
        }
    }
    Ok(())
}

struct Control {
    class: PCWSTR,
    text: Vec<u16>,
    style: WINDOW_STYLE,
    x: i32,
    y: i32,
    height: i32,
}

impl Control {
    fn label(text: &str, x: i32, y: i32) -> Self {
        Self {
            class: w!("STATIC"),
            text: wide(text),
            style: WS_VISIBLE | WS_CHILD,
            x,
            y,
            height: 22,
        }
    }

    fn field(text: &str, x: i32, y: i32) -> Self {
        Self {
            class: w!("EDIT"),
            text: wide(text),
            style: WS_VISIBLE | WS_CHILD | WS_TABSTOP | WS_BORDER | WINDOW_STYLE((ES_READONLY | ES_AUTOHSCROLL) as u32),
            x,
            y,
            height: 27,
        }
    }

    unsafe fn create(self, parent: HWND) -> windows::core::Result<HWND> {
        unsafe {
            let hwnd = CreateWindowExW(
                WINDOW_EX_STYLE::default(),
                self.class,
                PCWSTR(self.text.as_ptr()),
                self.style,
                self.x,
                self.y,
                340,
                self.height,
                Some(parent),
                None,
                None,
                None,
            )?;
            SendMessageW(
                hwnd,
                WM_SETFONT,
                Some(WPARAM(GetStockObject(DEFAULT_GUI_FONT).0 as usize)),
                Some(LPARAM(1)),
            );
            Ok(hwnd)
        }
    }
}

unsafe extern "system" fn window_proc(window: HWND, message: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    unsafe {
        match message {
            WM_SYSCOMMAND if wparam.0 & 0xFFF0 == FIREWALL_MENU => {
                if let Err(error) = std::process::Command::new(system_tool("control.exe"))
                    .args(["/name", "Microsoft.WindowsFirewall"])
                    .creation_flags(CREATE_NO_WINDOW.0)
                    .spawn()
                {
                    crate::platform::win::message_box("Windows Firewall", &error.to_string());
                }
                LRESULT(0)
            }
            WM_DESTROY => {
                PostQuitMessage(0);
                LRESULT(0)
            }
            _ => DefWindowProcW(window, message, wparam, lparam),
        }
    }
}
