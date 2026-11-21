use crate::{app::App, config::Ports, discovery, local, panel_https, settings::NetworkSettings, tls};
use serde::Deserialize;
use std::{
    net::Ipv4Addr,
    path::Path,
    sync::{Arc, OnceLock, RwLock},
};
use tokio::{
    net::{TcpListener, UdpSocket},
    sync::{mpsc, oneshot, watch},
    task::JoinHandle,
};

#[derive(Clone, Copy, Deserialize)]
pub struct NetworkRequest {
    #[serde(flatten)]
    pub ports: Ports,
    pub local_enabled: bool,
    pub lan_enabled: bool,
}

pub struct Network {
    state: RwLock<NetworkRequest>,
    pub lan: Option<panel_https::Server>,
    commands: OnceLock<mpsc::Sender<Change>>,
    panel_port: watch::Sender<u16>,
}

struct Change {
    next: NetworkRequest,
    reply: oneshot::Sender<Result<(), String>>,
}

impl Network {
    pub fn load(dir: &Path, settings: NetworkSettings, defaults: Ports) -> anyhow::Result<Self> {
        let ports = match settings.ports {
            Some(ports) => ports.validate().map_err(anyhow::Error::msg)?,
            None => defaults,
        };
        let lan = panel_https::Server::load(dir)
            .inspect_err(|error| tracing::error!("ใบรับรอง panel ใช้งานไม่ได้: {error:#}"))
            .ok()
            .flatten();
        Ok(Self {
            state: RwLock::new(NetworkRequest {
                ports,
                local_enabled: settings.local_enabled,
                lan_enabled: settings.lan_enabled,
            }),
            lan,
            commands: OnceLock::new(),
            panel_port: watch::channel(ports.local_port).0,
        })
    }

    pub fn state(&self) -> NetworkRequest {
        *self.state.read().unwrap()
    }

    pub fn watch_panel_port(&self) -> watch::Receiver<u16> {
        self.panel_port.subscribe()
    }

    pub async fn change(&self, next: NetworkRequest) -> Result<(), String> {
        next.ports.validate()?;
        if next.lan_enabled && self.lan.is_none() {
            return Err("ต้องติดตั้งใบรับรอง HTTPS และคีย์ที่ใช้ได้ก่อนเปิด LAN".into());
        }
        let sender = self.commands.get().ok_or("ตัวฟังเครือข่ายยังไม่พร้อม")?;
        let (reply, result) = oneshot::channel();
        sender
            .try_send(Change { next, reply })
            .map_err(|_| "กำลังใช้ค่าเครือข่ายอื่นอยู่")?;
        result.await.map_err(|_| "ตัวฟังเครือข่ายหยุดทำงาน")?
    }
}

struct Task(JoinHandle<()>);

impl Drop for Task {
    fn drop(&mut self) {
        self.0.abort();
    }
}

fn agent(app: &App, socket: TcpListener, config: &Arc<rustls::ServerConfig>) -> Task {
    Task(tokio::spawn(tls::serve(
        socket,
        config.clone(),
        crate::http::router(app.clone()),
        app.clone(),
    )))
}

fn panel(app: &App, socket: TcpListener, port: u16) -> Task {
    Task(tokio::spawn(panel_https::serve(
        socket,
        local::router(app.clone()),
        local::lan_router(app.clone()),
        app.clone(),
        port,
    )))
}

async fn bind_panel(app: &App, port: u16) -> std::io::Result<TcpListener> {
    let address = if app.network.lan.is_some() {
        Ipv4Addr::UNSPECIFIED
    } else {
        Ipv4Addr::LOCALHOST
    };
    bind_tcp(address, port).await
}

async fn bind_tcp(address: Ipv4Addr, port: u16) -> std::io::Result<TcpListener> {
    #[cfg(windows)]
    {
        let socket = bind_exclusive(address, port, socket2::Type::STREAM)?;
        socket.listen(128)?;
        TcpListener::from_std(socket.into())
    }
    #[cfg(not(windows))]
    TcpListener::bind((address, port)).await
}

pub(crate) async fn bind_discovery(port: u16) -> std::io::Result<UdpSocket> {
    #[cfg(windows)]
    {
        let socket = bind_exclusive(Ipv4Addr::UNSPECIFIED, port, socket2::Type::DGRAM)?;
        UdpSocket::from_std(socket.into())
    }
    #[cfg(not(windows))]
    UdpSocket::bind((Ipv4Addr::UNSPECIFIED, port)).await
}

#[cfg(windows)]
fn bind_exclusive(address: Ipv4Addr, port: u16, kind: socket2::Type) -> std::io::Result<socket2::Socket> {
    use std::os::windows::io::AsRawSocket;
    use windows::Win32::Networking::WinSock::{SO_EXCLUSIVEADDRUSE, SOCKET, SOL_SOCKET, WSAGetLastError, setsockopt};
    let socket = socket2::Socket::new(socket2::Domain::IPV4, kind, None)?;
    // A Winsock default wildcard binding can coexist with a later binding to a specific address.
    let enabled = 1_i32.to_ne_bytes();
    let result = unsafe {
        setsockopt(
            SOCKET(socket.as_raw_socket() as usize),
            SOL_SOCKET,
            SO_EXCLUSIVEADDRUSE,
            Some(&enabled),
        )
    };
    if result != 0 {
        return Err(std::io::Error::from_raw_os_error(unsafe { WSAGetLastError() }.0));
    }
    socket.set_nonblocking(true)?;
    socket.bind(&std::net::SocketAddr::from((address, port)).into())?;
    Ok(socket)
}

pub struct Listeners {
    agent: Task,
    panel: Task,
    discovery: discovery::Discovery,
    agent_tls: Arc<rustls::ServerConfig>,
    changes: mpsc::Receiver<Change>,
}

impl Listeners {
    pub async fn start(app: &App) -> anyhow::Result<Self> {
        let Ports {
            port,
            local_port,
            discovery_port,
        } = app.network.state().ports;
        let agent_socket = bind_tcp(Ipv4Addr::UNSPECIFIED, port).await?;
        let panel_socket = bind_panel(app, local_port).await?;
        let udp = bind_discovery(discovery_port)
            .await
            .inspect_err(|e| tracing::warn!("เปิด UDP {discovery_port} ไม่ได้: {e}"))
            .ok();
        let agent_tls = tls::server_config(app)?;
        let local_port = panel_socket.local_addr()?.port();
        {
            let mut current = app.network.state.write().unwrap();
            current.ports.local_port = local_port;
            current.ports.port = agent_socket.local_addr()?.port();
            if let Some(socket) = &udp {
                current.ports.discovery_port = socket.local_addr()?.port();
            }
        }
        app.network.panel_port.send_replace(local_port);
        publish_panel_port(app, local_port)?;
        let (sender, changes) = mpsc::channel(1);
        let _ = app.network.commands.set(sender);
        Ok(Self {
            agent: agent(app, agent_socket, &agent_tls),
            panel: panel(app, panel_socket, local_port),
            discovery: discovery::start_with_socket(app, udp),
            agent_tls,
            changes,
        })
    }

    pub async fn supervise(mut self, app: App) {
        while let Some(change) = self.changes.recv().await {
            let result = self.apply(&app, change.next).await;
            let _ = change.reply.send(result);
        }
    }

    async fn apply(&mut self, app: &App, next: NetworkRequest) -> Result<(), String> {
        let old = app.network.state();
        #[cfg(windows)]
        if app.config.mode == crate::config::Mode::Service
            && (old.ports != next.ports || next.lan_enabled && !old.lan_enabled)
            && !crate::winsvc::firewall_ready(&app.config.data_dir)
        {
            return Err("กำลังเตรียมกฎไฟร์วอลล์ของ Windows กรุณาลองใหม่หลัง service ตัวอัปเดตทำงานเสร็จ".into());
        }
        let bind_error = |port, error| format!("เปิดพอร์ต {port} ไม่ได้: {error}");
        let agent = if old.ports.port != next.ports.port {
            Some(
                bind_tcp(Ipv4Addr::UNSPECIFIED, next.ports.port)
                    .await
                    .map_err(|e| bind_error(next.ports.port, e))?,
            )
        } else {
            None
        };
        let panel = if old.ports.local_port != next.ports.local_port {
            Some(
                bind_panel(app, next.ports.local_port)
                    .await
                    .map_err(|e| bind_error(next.ports.local_port, e))?,
            )
        } else {
            None
        };
        let udp = if old.ports.discovery_port != next.ports.discovery_port {
            Some(
                bind_discovery(next.ports.discovery_port)
                    .await
                    .map_err(|e| bind_error(next.ports.discovery_port, e))?,
            )
        } else {
            None
        };

        if panel.is_some() {
            publish_panel_port(app, next.ports.local_port).map_err(|e| e.to_string())?;
        }
        if let Err(error) = app.settings.update(|settings| {
            settings.network = NetworkSettings {
                ports: Some(next.ports),
                local_enabled: next.local_enabled,
                lan_enabled: next.lan_enabled,
            }
        }) {
            if panel.is_some() {
                let _ = publish_panel_port(app, old.ports.local_port);
            }
            return Err(format!("บันทึกค่าเครือข่ายไม่ได้: {error}"));
        }
        *app.network.state.write().unwrap() = next;
        if let Some(socket) = agent {
            app.auth.kill_all();
            self.agent = self::agent(app, socket, &self.agent_tls);
        }
        if let Some(socket) = panel {
            self.panel = self::panel(app, socket, next.ports.local_port);
            app.network.panel_port.send_replace(next.ports.local_port);
        }
        if let Some(socket) = udp {
            self.discovery.set_probe(app, socket);
        }
        if old.ports.port != next.ports.port {
            self.discovery.refresh_mdns(app);
        }
        Ok(())
    }
}

fn publish_panel_port(app: &App, port: u16) -> std::io::Result<()> {
    let readers: Vec<_> = app.settings.get().owner.into_iter().collect();
    crate::platform::write_private_for(
        &app.config.data_dir.join("panel-port"),
        port.to_string().as_bytes(),
        &readers,
    )
}
