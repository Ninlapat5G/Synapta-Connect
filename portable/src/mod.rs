use crate::{
    app::{App, AppState},
    config::{Config, Mode, PORTABLE_DISCOVERY_PORT},
    network::Listeners,
    platform::win::system_tool,
};
use std::{path::PathBuf, time::Duration};

mod window;

struct SessionFolder(PathBuf);

impl Drop for SessionFolder {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

pub fn run() -> anyhow::Result<()> {
    crate::platform::init();
    let session_path = std::env::temp_dir().join(format!("SynaptaConnectPortable-{}", crate::hex::random_id(16)));
    std::fs::create_dir(&session_path)?;
    let folder = SessionFolder(session_path);
    let config = Config {
        mode: Mode::Portable,
        port: 0,
        discovery_port: PORTABLE_DISCOVERY_PORT,
        data_dir: folder.0.clone(),
        ..Config::default()
    };
    let runtime = tokio::runtime::Runtime::new()?;
    let (app, listeners) = runtime.block_on(start(config))?;
    let supervisor = runtime.spawn(listeners.supervise(app.clone()));
    let result = window::run(&app, &supervisor);
    for device in app.auth.list() {
        app.auth.revoke(&device.id);
    }
    let stopped = supervisor.is_finished();
    let network_result = if stopped {
        runtime.block_on(supervisor)?
    } else {
        supervisor.abort();
        Ok(())
    };
    runtime.shutdown_timeout(Duration::from_secs(2));
    result?;
    network_result
}

async fn start(config: Config) -> anyhow::Result<(App, Listeners)> {
    let _ = rustls::crypto::ring::default_provider().install_default();
    let app = AppState::new(config)?;
    crate::legacy_pty::set_data_dir(&app.config.data_dir);
    let listeners = Listeners::start(&app).await?;
    app.auth.portable_otp();
    Ok((app, listeners))
}
