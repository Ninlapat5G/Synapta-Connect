//! ส่วนที่ขึ้นกับ OS: การตั้งค่าโปรเซสตอนเริ่ม (Windows) และการเขียนไฟล์ลับ

#[cfg(windows)]
pub mod win {
    use std::{
        collections::HashSet,
        ffi::{OsString, c_void},
        os::windows::ffi::OsStringExt,
    };

    unsafe extern "system" {
        fn SetConsoleOutputCP(code_page: u32) -> i32;
        fn GetModuleHandleW(name: *const u16) -> *mut c_void;
        fn LoadLibraryExW(name: *const u16, file: *mut c_void, flags: u32) -> *mut c_void;
        fn GetProcAddress(module: *mut c_void, name: *const u8) -> *mut c_void;
    }

    #[link(name = "user32")]
    unsafe extern "system" {
        fn MessageBoxW(window: *mut c_void, text: *const u16, caption: *const u16, kind: u32) -> i32;
    }

    #[link(name = "advapi32")]
    unsafe extern "system" {
        fn RegGetValueW(
            key: isize,
            subkey: *const u16,
            value: *const u16,
            flags: u32,
            value_type: *mut u32,
            data: *mut c_void,
            bytes: *mut u32,
        ) -> i32;
    }

    const LOAD_LIBRARY_SEARCH_SYSTEM32: u32 = 0x800;
    const HKEY_CURRENT_USER: isize = 0x8000_0001u32 as i32 as isize;
    const HKEY_LOCAL_MACHINE: isize = 0x8000_0002u32 as i32 as isize;
    const RRF_RT_REG_SZ: u32 = 0x0002;
    const RRF_RT_REG_EXPAND_SZ: u32 = 0x0004;

    /// กล่องข้อความแจ้งผิดพลาด — ฟอนต์ระบบแสดงภาษาไทยได้ ต่างจาก console ของ Server 2016 ที่ขึ้นเป็นกล่องสี่เหลี่ยม
    pub fn message_box(title: &str, text: &str) {
        const MB_ICONWARNING: u32 = 0x30;
        unsafe {
            MessageBoxW(
                std::ptr::null_mut(),
                wide(text).as_ptr(),
                wide(title).as_ptr(),
                MB_ICONWARNING,
            )
        };
    }

    pub fn init() {
        // กับดักที่เคยเจอจริง: จอ 1920x1080 ที่ตั้ง scale 125% — โปรเซสที่ไม่ประกาศ DPI-aware จะเห็นจอเป็น
        // 1536x864 ตอนจำลองเมาส์ แต่ภาพที่จับได้เป็น 1920x1080 จริง → AI คลิกจุดจากภาพแล้วพลาดไป 25%
        set_dpi_aware();
        // console ใช้ codepage ท้องถิ่น (เช่น 874) ทำให้ภาษาไทยใน OTP/log เพี้ยน — บังคับ UTF-8
        unsafe { SetConsoleOutputCP(65001) };
    }

    /// ประกาศ per-monitor DPI ให้ทั้งภาพและเมาส์ใช้ pixel จริงเหมือนกัน
    ///
    /// กับดัก: ห้ามลิงก์ SetProcessDpiAwarenessContext แบบตรง — ฟังก์ชันนี้มีตั้งแต่ Windows 10 1703
    /// ถ้าอยู่ใน import table, Windows Server 2016 (1607) จะไม่ยอมเปิด exe เลย ("entry point not found")
    /// จึงหาแบบ dynamic: v2 (-4) ถ้ามี ไม่งั้นใช้ SetProcessDpiAwareness(2) ของ shcore (per-monitor v1)
    fn set_dpi_aware() {
        let user32 = wide("user32.dll");
        let shcore = wide("shcore.dll");
        unsafe {
            let set_context = GetProcAddress(
                GetModuleHandleW(user32.as_ptr()),
                c"SetProcessDpiAwarenessContext".as_ptr().cast(),
            );
            if !set_context.is_null() {
                let f: unsafe extern "system" fn(isize) -> i32 = std::mem::transmute(set_context);
                f(-4);
                return;
            }
            let module = LoadLibraryExW(shcore.as_ptr(), std::ptr::null_mut(), LOAD_LIBRARY_SEARCH_SYSTEM32);
            if module.is_null() {
                return;
            }
            let set_awareness = GetProcAddress(module, c"SetProcessDpiAwareness".as_ptr().cast());
            if !set_awareness.is_null() {
                let f: unsafe extern "system" fn(i32) -> i32 = std::mem::transmute(set_awareness);
                f(2);
            }
        }
    }

    fn wide(text: &str) -> Vec<u16> {
        text.encode_utf16().chain(Some(0)).collect()
    }

    /// เครื่องมือของระบบด้วย path เต็มใน System32 — ไม่พึ่ง PATH/โฟลเดอร์ปัจจุบัน (กัน exe ปลอมชื่อเดียวกันที่ผู้ใช้ทั่วไปวางไว้
    /// ถูกเรียกด้วยสิทธิ์ของ service หรือของตัวติดตั้งที่รันเป็น Administrator)
    pub fn system_tool(name: &str) -> std::path::PathBuf {
        let root = std::env::var_os("SystemRoot").unwrap_or_else(|| r"C:\Windows".into());
        std::path::Path::new(&root).join("System32").join(name)
    }

    pub fn terminal_program(shell: &str) -> OsString {
        match shell.to_ascii_lowercase().as_str() {
            "cmd" | "cmd.exe" => system_tool("cmd.exe").into(),
            "powershell" | "powershell.exe" => system_tool(r"WindowsPowerShell\v1.0\powershell.exe").into(),
            _ => shell.into(),
        }
    }

    pub fn terminal_path(mode: crate::config::Mode) -> Option<OsString> {
        let machine = registry_path(
            HKEY_LOCAL_MACHINE,
            r"SYSTEM\CurrentControlSet\Control\Session Manager\Environment",
        );
        if mode == crate::config::Mode::Service {
            return machine;
        }

        // Keep the launching shell's PATH precedence (for example, an activated virtual environment).
        let paths = [
            std::env::var_os("Path"),
            machine,
            registry_path(HKEY_CURRENT_USER, "Environment"),
        ];
        let mut seen = HashSet::new();
        std::env::join_paths(
            paths
                .iter()
                .flatten()
                .flat_map(std::env::split_paths)
                .filter(|path| seen.insert(path.as_os_str().to_ascii_lowercase())),
        )
        .ok()
    }

    fn registry_path(key: isize, subkey: &str) -> Option<OsString> {
        // Windows limits an environment variable to 32,767 characters.
        let mut path = vec![0u16; 32_768];
        let mut bytes = (path.len() * size_of::<u16>()) as u32;
        let status = unsafe {
            RegGetValueW(
                key,
                wide(subkey).as_ptr(),
                wide("Path").as_ptr(),
                RRF_RT_REG_SZ | RRF_RT_REG_EXPAND_SZ,
                std::ptr::null_mut(),
                path.as_mut_ptr().cast(),
                &mut bytes,
            )
        };
        if status != 0 {
            return None;
        }
        let end = path.iter().position(|&unit| unit == 0)?;
        Some(OsString::from_wide(&path[..end]))
    }

    /// บัญชีที่โปรเซสนี้รันอยู่ในรูป `DOMAIN\user` (service บัญชีเสมือนได้ `nt service\synaptaconnect`)
    pub fn current_account() -> Option<String> {
        use std::os::windows::process::CommandExt;
        let out = std::process::Command::new(system_tool("whoami.exe"))
            .creation_flags(0x0800_0000) // CREATE_NO_WINDOW
            .output()
            .ok()?;
        let name = String::from_utf8_lossy(&out.stdout).trim().to_string();
        (out.status.success() && !name.is_empty()).then_some(name)
    }

    fn icacls(path: &std::path::Path, args: &[&str]) -> bool {
        use std::os::windows::process::CommandExt;
        std::process::Command::new(system_tool("icacls.exe"))
            .arg(path)
            .args(args)
            .creation_flags(0x0800_0000) // CREATE_NO_WINDOW
            .output()
            .is_ok_and(|o| o.status.success())
    }

    /// จำกัดไฟล์ให้บัญชีที่รัน + SYSTEM + Administrators (อ่านเขียนได้) และ `readers` (อ่านอย่างเดียว) เท่านั้น
    /// ตัดสิทธิ์ที่สืบทอดจากโฟลเดอร์แม่ ทำไม่สำเร็จก็แค่เตือน — ไฟล์ยังได้ ACL ของโฟลเดอร์ตามปกติ
    pub fn restrict_acl(path: &std::path::Path, readers: &[String]) {
        let me = current_account().or_else(|| std::env::var("USERNAME").ok());
        let Some(me) = me else { return };
        let owner_grant = format!("{me}:F");
        if !icacls(
            path,
            &[
                "/inheritance:r",
                "/grant:r",
                &owner_grant,
                "*S-1-5-18:F",
                "*S-1-5-32-544:F",
            ],
        ) {
            tracing::warn!(
                "ตั้ง ACL ให้ {} ไม่สำเร็จ — ตรวจสิทธิ์ของโฟลเดอร์ data-dir ด้วยตนเอง",
                path.display()
            );
            return;
        }
        for reader in readers {
            // แยกคำสั่งต่อคน: ชื่อบัญชีที่ผิดคนเดียวต้องไม่ทำให้สิทธิ์ของคนอื่นหาย
            if !icacls(path, &["/grant", &format!("{reader}:R")]) {
                tracing::warn!(
                    "ให้ {reader} อ่าน {} ไม่สำเร็จ — ตรวจชื่อบัญชีเจ้าของใน settings.json",
                    path.display()
                );
            }
        }
    }
}

/// บัญชีที่โปรเซสนี้รันอยู่ (แสดงใน panel) — ไม่รู้ = None
pub fn account() -> Option<String> {
    #[cfg(windows)]
    return win::current_account();
    #[cfg(not(windows))]
    return std::env::var("USER").ok();
}

/// ทำครั้งเดียวตอนเริ่มโปรแกรม
pub fn init() {
    #[cfg(windows)]
    win::init();
}

/// เขียนไฟล์ลับ (host key, local token) ให้อ่านได้เฉพาะเจ้าของ — Unix: 0600, Windows: ACL เฉพาะบัญชีที่รัน/SYSTEM/Administrators
pub fn write_private(path: &std::path::Path, data: &[u8]) -> std::io::Result<()> {
    write_private_for(path, data, &[])
}

/// เหมือน write_private แต่ให้ `readers` (บัญชี Windows) อ่านได้ด้วย — ใช้กับ local_token ที่เจ้าของเครื่องต้องอ่านเพื่อเปิด panel
/// (Unix: ไม่ใช้ — service ไม่ใช่โหมดของ Unix)
pub fn write_private_for(path: &std::path::Path, data: &[u8], readers: &[String]) -> std::io::Result<()> {
    #[cfg(unix)]
    {
        let _ = readers;
        use std::os::unix::fs::OpenOptionsExt;
        let mut f = std::fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .mode(0o600)
            .open(path)?;
        std::io::Write::write_all(&mut f, data)
    }
    #[cfg(windows)]
    {
        // ไฟล์มีอยู่แล้ว = เขียนทับเนื้อหาโดยคง ACL เดิม: service รันด้วยบัญชีสิทธิ์ต่ำ แก้ ACL ไม่ได้แน่นอนทุกกรณี
        // ตัวติดตั้ง (Administrator) จึงสร้างไฟล์ local_token พร้อม ACL ที่ถูกต้องไว้ก่อน แล้ว service แค่เติมเนื้อหาใหม่ทุกครั้งที่เริ่ม
        let existed = path.exists();
        std::fs::write(path, data)?;
        if !existed {
            win::restrict_acl(path, readers);
        }
        Ok(())
    }
}
