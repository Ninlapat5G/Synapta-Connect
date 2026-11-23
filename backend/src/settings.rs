//! ค่าตั้งที่เจ้าของเครื่องเปลี่ยนได้ตอนรัน (ผ่าน panel/ตัวติดตั้ง) — เก็บใน data_dir/settings.json
//!
//! ค่าตั้งของ "พอร์ต/โฟลเดอร์" ยังมาจาก command line (config.rs) เหมือนเดิม; ไฟล์นี้มีเฉพาะสิ่งที่ panel แก้ได้

use crate::{config::Ports, fsutil};
use serde::{Deserialize, Serialize};
use std::{
    io,
    path::{Path, PathBuf},
    sync::Mutex,
};

/// แหล่ง release เริ่มต้นที่ฝังตอน build (`release/source.txt`) — ว่าง = ต้องตั้งเอง
const DEFAULT_SOURCE: &str = include_str!("../release/source.txt");

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct UpdateSettings {
    /// เปิด = ตรวจและติดตั้งเอง; ปิด = เฉพาะที่เจ้าของกดใน panel
    pub auto_update: bool,
    /// `github:OWNER/REPO` | `https://…/` | `dir:PATH` (ว่าง = ใช้ค่าที่ฝังตอน build)
    pub source: String,
    pub web_source: String,
    pub check_interval_hours: u64,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct ScreenInput {
    /// เจ้าของอนุญาตให้จับภาพ/ส่ง input ผ่าน helper ใน session ของตัวเองหรือไม่ (ปิดเป็นค่าเริ่มต้น)
    pub enabled: bool,
}

/// คำสั่ง (exec / terminal) รันด้วยบัญชีไหน — ตัดสินใน run_as.rs
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct CommandSettings {
    /// เปิด = รันผ่าน helper ด้วยบัญชีเจ้าของเครื่อง (เหมือนพิมพ์ที่หน้าเครื่อง); ไม่มี helper = กลับไปใช้บัญชี service
    pub run_as_owner: bool,
    /// เปิด = ต้องได้สิทธิ์ administrator (helper ที่ยังไม่ elevated จะขอ UAC บนหน้าจอเครื่อง)
    pub administrator: bool,
}

impl Default for CommandSettings {
    fn default() -> Self {
        CommandSettings {
            run_as_owner: true,
            administrator: false,
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub update: UpdateSettings,
    pub screen_input: ScreenInput,
    pub commands: CommandSettings,
    /// บัญชี Windows ของเจ้าของเครื่อง (อ่าน local_token ได้, เป็นเจ้าของ helper)
    pub owner: Option<String>,
    pub network: NetworkSettings,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(default)]
pub struct NetworkSettings {
    pub ports: Option<Ports>,
    pub local_enabled: bool,
    pub lan_enabled: bool,
}

impl Default for NetworkSettings {
    fn default() -> Self {
        Self {
            ports: None,
            local_enabled: true,
            lan_enabled: false,
        }
    }
}

impl Default for UpdateSettings {
    fn default() -> Self {
        UpdateSettings {
            auto_update: true,
            source: String::new(),
            web_source: String::new(),
            check_interval_hours: 6,
        }
    }
}

impl Settings {
    /// แหล่ง release ที่ใช้จริง (ว่าง = ยังไม่ได้ตั้ง)
    pub fn effective_source(&self) -> String {
        match self.update.source.trim() {
            "" => DEFAULT_SOURCE.trim().to_string(),
            s => s.to_string(),
        }
    }

    pub fn effective_web_source(&self) -> String {
        if self.update.web_source.trim().is_empty() {
            self.effective_source()
        } else {
            self.update.web_source.trim().to_string()
        }
    }
}

pub struct SettingsStore {
    path: PathBuf,
    inner: Mutex<Settings>,
}

impl SettingsStore {
    pub fn load(data_dir: &Path) -> SettingsStore {
        let path = data_dir.join("settings.json");
        let inner = Mutex::new(fsutil::read_json(&path).unwrap_or_default());
        SettingsStore { path, inner }
    }

    pub fn get(&self) -> Settings {
        self.inner.lock().unwrap().clone()
    }

    /// สำเนาค่าตั้งปัจจุบันไปไว้ที่ `path` (สำรองก่อนอัปเดต)
    pub fn save_copy(&self, path: &Path) -> io::Result<()> {
        fsutil::write_json(path, &self.get())
    }

    /// ใส่ค่าตั้งคืนจากไฟล์สำรอง (ตัวติดตั้งกู้รุ่นเดิมแล้วส่งมาให้) — ไฟล์อ่านไม่ออก = ไม่เปลี่ยนอะไร
    pub fn restore_from(&self, backup: &Path) -> io::Result<()> {
        let restored: Settings = fsutil::read_json(backup).ok_or_else(|| io::Error::other("ไฟล์ค่าตั้งสำรองอ่านไม่ได้"))?;
        self.update(|s| {
            let network = s.network;
            *s = restored;
            s.network = network;
        })
        .map(|_| ())
    }

    /// แก้แล้วบันทึกทันที — บันทึกไม่ได้ = ไม่เปลี่ยนค่าในหน่วยความจำ (กันค่าบนดิสก์กับที่ใช้อยู่ไม่ตรงกัน)
    pub fn update(&self, change: impl FnOnce(&mut Settings)) -> io::Result<Settings> {
        let mut guard = self.inner.lock().unwrap();
        let mut next = guard.clone();
        change(&mut next);
        fsutil::write_json(&self.path, &next)?;
        *guard = next.clone();
        Ok(next)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_then_persist_and_partial_file_keeps_defaults() {
        let dir = std::env::temp_dir().join(format!("synapta-settings-{}", crate::hex::random_id(4)));
        std::fs::create_dir_all(&dir).unwrap();
        let store = SettingsStore::load(&dir);
        assert!(store.get().update.auto_update);
        assert!(!store.get().screen_input.enabled);
        assert!(store.get().commands.run_as_owner);
        assert!(!store.get().commands.administrator);

        store.update(|s| s.update.auto_update = false).unwrap();
        assert!(!SettingsStore::load(&dir).get().update.auto_update);

        // ไฟล์จากรุ่นเก่าที่มีไม่ครบทุกฟิลด์ต้องอ่านได้ (การย้ายรุ่น/rollback ไม่ทำให้ค่าหาย)
        std::fs::write(dir.join("settings.json"), r#"{"owner":"me"}"#).unwrap();
        let s = SettingsStore::load(&dir).get();
        assert_eq!(s.owner.as_deref(), Some("me"));
        assert!(s.update.auto_update);
        std::fs::remove_dir_all(dir).unwrap();
    }
}
