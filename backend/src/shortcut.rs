//! ไอคอน "Synapta Connect" บน Desktop และ Start Menu ของเจ้าของเครื่อง
//!
//! ไอคอนรัน `synapta-connect.exe panel --gui` เพื่อเปิด Desktop ในรุ่นที่มี Tauri
//! จึงไม่ต้องคัดลอกคำสั่งไปวางใน PowerShell และไม่ได้ลดความปลอดภัยลง
//! helper สร้างให้ตอนเริ่ม (helper รันด้วยบัญชีเจ้าของ จึงเขียนโฟลเดอร์ของเจ้าของได้ และได้ทั้งเครื่องที่อัปเดตอัตโนมัติ
//! ซึ่งไม่ผ่านขั้น install) — สร้างครั้งเดียว ถ้าเจ้าของลบทิ้งเองจะไม่สร้างซ้ำ

use base64::Engine;
use std::{os::windows::process::CommandExt, path::Path, process::Command};

const LINK: &str = "Synapta Connect.lnk";
/// มีไฟล์นี้ = เคยสร้างไอคอนให้ผู้ใช้นี้แล้ว
const MARKER: &str = "shortcut-created";

fn marker_path() -> Option<std::path::PathBuf> {
    Some(
        Path::new(&std::env::var_os("LOCALAPPDATA")?)
            .join("SynaptaConnect")
            .join(MARKER),
    )
}

/// สคริปต์ส่งแบบ -EncodedCommand (UTF-16 base64) — path ที่มีช่องว่างหรือภาษาไทยไม่ถูก cmd/PowerShell ตีความผิด
fn powershell(script: &str) -> Result<(), String> {
    let utf16: Vec<u8> = script.encode_utf16().flat_map(u16::to_le_bytes).collect();
    let encoded = base64::engine::general_purpose::STANDARD.encode(utf16);
    let status = Command::new(crate::platform::win::system_tool(
        r"WindowsPowerShell\v1.0\powershell.exe",
    ))
    .args(["-NoProfile", "-NonInteractive", "-EncodedCommand", &encoded])
    .creation_flags(0x0800_0000) // CREATE_NO_WINDOW
    .status()
    .map_err(|e| e.to_string())?;
    status
        .success()
        .then_some(())
        .ok_or_else(|| format!("powershell จบด้วย {status}"))
}

/// สร้างไอคอนให้ผู้ใช้ที่รันโปรเซสนี้ (เรียกจาก helper หลังยืนยันว่าเป็นเจ้าของ) — ล้มเหลวก็แค่บันทึก log
pub fn ensure_for_current_user() {
    let Some(marker) = marker_path() else { return };
    let Ok(exe) = std::env::current_exe() else { return };
    // เฉพาะตัวที่ติดตั้งแล้ว — exe ที่รันจากโฟลเดอร์ build/ดาวน์โหลดไม่ควรทิ้งไอคอนชี้ไปที่ชั่วคราว
    if marker.exists() || !exe.starts_with(crate::winsvc::install::install_dir()) {
        return;
    }
    let exe = exe.display().to_string().replace('\'', "''");
    let script = format!(
        "$ErrorActionPreference = 'Stop'; $shell = New-Object -ComObject WScript.Shell; \
         foreach ($dir in [Environment]::GetFolderPath('Programs'), [Environment]::GetFolderPath('Desktop')) {{ \
           $link = $shell.CreateShortcut((Join-Path $dir '{LINK}')); $link.TargetPath = '{exe}'; \
           $link.Arguments = 'panel --gui'; $link.WindowStyle = 7; $link.IconLocation = '{exe},0'; \
           $link.Description = 'เปิดหน้าจัดการ Synapta Connect'; $link.Save() }}"
    );
    match powershell(&script) {
        Ok(()) => {
            let _ = std::fs::create_dir_all(marker.parent().unwrap_or(&marker));
            let _ = std::fs::write(&marker, b"");
        }
        Err(e) => tracing::warn!("สร้างไอคอน Synapta Connect ไม่ได้: {e}"),
    }
}

/// ลบไอคอนของทุกผู้ใช้ (ตอนถอนการติดตั้ง ซึ่งรันเป็น Administrator) — โปรไฟล์ไล่จาก ProfileList
pub fn remove_for_all_users() -> Result<(), String> {
    powershell(&format!(
        "Get-ChildItem 'HKLM:\\SOFTWARE\\Microsoft\\Windows NT\\CurrentVersion\\ProfileList' | ForEach-Object {{ \
           $profileDir = (Get-ItemProperty $_.PSPath).ProfileImagePath; if (-not $profileDir) {{ return }} \
           foreach ($file in \"$profileDir\\Desktop\\{LINK}\", \
             \"$profileDir\\AppData\\Roaming\\Microsoft\\Windows\\Start Menu\\Programs\\{LINK}\", \
             \"$profileDir\\AppData\\Local\\SynaptaConnect\\{MARKER}\") {{ \
             Remove-Item -LiteralPath $file -Force -ErrorAction SilentlyContinue }} }}"
    ))
}
