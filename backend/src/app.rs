//! สถานะที่ใช้ร่วมกันทั้งโปรแกรม

use crate::{
    auth::{Auth, load_or_create_node_id},
    config::{Config, Mode},
    display::Display,
    network::Network,
    pairing::Handshakes,
    panel::PanelSessions,
    settings::SettingsStore,
    tls::HostIdentity,
    transfer::Transfers,
    update::{NoInstaller, UpdateEnv, UpdateManager, manifest, version::Version, web::WebUpdater},
};
use std::{sync::Arc, time::Instant};
use tokio::sync::Semaphore;

/// MJPEG stream พร้อมกันได้กี่อัน (แต่ละอันกิน CPU จับภาพ+เข้ารหัสต่อเนื่อง)
const MAX_SCREEN_STREAMS: usize = 3;

pub struct AppState {
    pub config: Config,
    /// `sc-node-xxxxxxxxxxxx` — ชื่อที่ประกาศใน beacon (ไม่ใช่ความลับ)
    pub node_id: String,
    /// ความลับที่หน้าจัดการบนเครื่องต้องแนบ (header X-Synapta-Local) — สุ่มใหม่ทุกครั้งที่เริ่ม เก็บใน
    /// data_dir/local_token ที่จำกัดสิทธิ์ไว้ ให้ process ของผู้ใช้อื่นบนเครื่องเดียวกันอ่าน OTP / เปิดโหมดจับคู่ไม่ได้
    pub local_token: String,
    pub host: HostIdentity,
    pub auth: Auth,
    pub handshakes: Handshakes,
    pub transfers: Transfers,
    pub screen_streams: Arc<Semaphore>,
    pub started: Instant,
    /// ค่าตั้งที่เจ้าของแก้ผ่าน panel (อัปเดต, สิทธิ์จับภาพ/input)
    pub settings: Arc<SettingsStore>,
    pub updates: Arc<UpdateManager>,
    /// ตัวส่งงานจับภาพ/input ไปยังที่ที่ทำได้ (ในโปรเซส หรือ helper)
    pub display: Display,
    pub panel: PanelSessions,
    pub network: Network,
    pub web_updates: Arc<WebUpdater>,
    pub(crate) sdp_ota: crate::sdp_ota::Ota,
}

pub type App = Arc<AppState>;

/// ค่าอัปเดตของโปรแกรมจริง: public key ที่ฝังตอน build, ตัวติดตั้งจริงเฉพาะโหมด service
fn production_update_env(mode: Mode) -> UpdateEnv {
    #[cfg(windows)]
    let trigger: Arc<dyn crate::update::Trigger> = match mode {
        Mode::Service => Arc::new(crate::winsvc::ServiceTrigger),
        Mode::Console | Mode::Portable => Arc::new(NoInstaller),
    };
    #[cfg(not(windows))]
    let trigger: Arc<dyn crate::update::Trigger> = Arc::new(NoInstaller);
    let _ = mode;
    UpdateEnv {
        pubkey: manifest::embedded_pubkey(),
        trigger,
        current: Version::current(),
    }
}

impl AppState {
    pub fn new(config: Config) -> anyhow::Result<App> {
        let env = production_update_env(config.mode);
        AppState::new_with_update_env(config, env)
    }

    /// เทสต์ใช้ช่องนี้ใส่ public key / ตัวติดตั้งปลอม
    pub fn new_with_update_env(mut config: Config, update_env: UpdateEnv) -> anyhow::Result<App> {
        // เทียบ root แบบ canonical ตั้งแต่ต้น: ถ้า root ไม่มีอยู่ ให้ล้มตอนเริ่มดีกว่าปล่อยให้ไม่จำกัดเงียบ ๆ
        config.file_roots = config
            .file_roots
            .iter()
            .map(|r| {
                r.canonicalize()
                    .map_err(|e| anyhow::anyhow!("--file-root {}: {e}", r.display()))
            })
            .collect::<Result<_, _>>()?;
        let portable = config.mode == Mode::Portable;
        let local_token = crate::hex::random_id(32);
        std::fs::create_dir_all(&config.data_dir)?;
        let settings = Arc::new(SettingsStore::load(&config.data_dir));
        let network = Network::load(&config.data_dir, settings.get().network, config.ports())?;
        // เจ้าของเครื่อง (บัญชีที่ติดตั้งระบุไว้) ต้องอ่าน token ได้เพื่อเปิด panel — คนอื่นอ่านไม่ได้
        let readers: Vec<String> = settings.get().owner.into_iter().collect();
        if !portable {
            crate::platform::write_private_for(&config.data_dir.join("local_token"), local_token.as_bytes(), &readers)?;
        }
        let web_updates = Arc::new(WebUpdater::new(&config.data_dir, settings.clone(), update_env.pubkey));
        let updates = Arc::new(UpdateManager::new(
            update_env,
            settings.clone(),
            config.data_dir.clone(),
        ));
        Ok(Arc::new(AppState {
            started: Instant::now(),
            display: Display::new(config.mode),
            panel: PanelSessions::load(&config.data_dir),
            network,
            web_updates,
            settings,
            updates,
            local_token,
            node_id: if portable {
                format!("sc-node-{}", crate::hex::random_id(6))
            } else {
                load_or_create_node_id(&config.data_dir)?
            },
            host: if portable {
                HostIdentity::ephemeral()?
            } else {
                HostIdentity::load_or_create(&config.data_dir)?
            },
            auth: if portable {
                Auth::ephemeral()
            } else {
                Auth::load(&config.data_dir)?
            },
            handshakes: Handshakes::default(),
            transfers: Transfers::new(config.file_roots.clone()),
            screen_streams: Arc::new(Semaphore::new(MAX_SCREEN_STREAMS)),
            sdp_ota: crate::sdp_ota::Ota::new(),
            config,
        }))
    }
}
