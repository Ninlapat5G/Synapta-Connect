//! ให้ client หาเครื่องนี้เจอเอง: mDNS + UDP broadcast beacon + UDP probe
//!
//! ขอบเขตที่ต้องรู้: mDNS และ broadcast วิ่งได้เฉพาะในวง L2 เดียวกัน (LAN/Wi-Fi) —
//! VPN แบบ L3 (Tailscale, WireGuard) ไม่ส่งต่อทั้งสองอย่าง จึงมี UDP probe:
//! client ส่ง unicast `{"probe":"synapta-connect"}` ไปที่ IP:discovery_port ของเครื่องนี้
//! (ผ่าน VPN ได้) แล้วได้ beacon ตอบกลับ; ถ้าไม่ได้เลยก็ต่อ IP:port ตรง ๆ (direct fallback)
//!
//! ใครบน LAN ก็อ่าน beacon ได้ จึงประกาศแค่ตัวตนพื้นฐานและชื่อที่แสดงตอนจับคู่ Portable

use crate::{app::App, netif};
use mdns_sd::{IfKind, ServiceDaemon, ServiceInfo};
use serde_json::{Value, json};
use std::{net::SocketAddr, time::Duration};
use tokio::{net::UdpSocket, task::JoinHandle};

const SERVICE_TYPE: &str = "_synapta-connect._tcp.local.";
const PROBE_MAGIC: &str = "synapta-connect";
const BEACON_INTERVAL: Duration = Duration::from_secs(3);

/// ต้องถือไว้ตลอดอายุโปรแกรม — ทิ้งเมื่อไหร่ mDNS หยุดประกาศ
pub struct Discovery {
    mdns: Option<ServiceDaemon>,
    probe: Option<JoinHandle<()>>,
    broadcast: JoinHandle<()>,
}

fn beacon(app: &App) -> Value {
    let pairing_mode = app.auth.current_otp().is_some();
    let portable = app.config.mode == crate::config::Mode::Portable;
    let mut beacon = json!({
        "protocol": "synapta-connect/v1",
        "device_id": app.node_id,
        "host_pubkey_fingerprint": app.host.fingerprint(),
        "port": app.network.state().ports.port,
        "pairing_mode": pairing_mode,
        "portable": portable,
    });
    if portable && pairing_mode {
        beacon["name"] = json!(app.config.name);
    }
    beacon
}

/// เริ่มประกาศตัวเอง; ล้มเหลวส่วนไหนก็แค่ log (agent ยังต่อด้วย IP ตรงได้)
pub async fn start(app: &App) -> Discovery {
    let port = app.network.state().ports.discovery_port;
    let socket = crate::network::bind_discovery(port)
        .await
        .inspect_err(|e| tracing::warn!("เปิด UDP {port} ไม่ได้: {e}"))
        .ok();
    start_with_socket(app, socket)
}

pub(crate) fn start_with_socket(app: &App, socket: Option<UdpSocket>) -> Discovery {
    let mdns = start_mdns(app).inspect_err(|e| tracing::warn!("mDNS ใช้ไม่ได้: {e}")).ok();
    Discovery {
        mdns,
        probe: socket.map(|sock| tokio::spawn(answer_probes(sock, app.clone()))),
        broadcast: tokio::spawn(broadcast_beacons(app.clone())),
    }
}

impl Discovery {
    pub fn set_probe(&mut self, app: &App, socket: UdpSocket) {
        if let Some(old) = self.probe.replace(tokio::spawn(answer_probes(socket, app.clone()))) {
            old.abort();
        }
    }

    pub fn refresh_mdns(&mut self, app: &App) {
        if let Some(previous) = self.mdns.take() {
            let _ = previous.shutdown();
        }
        self.mdns = start_mdns(app).inspect_err(|e| tracing::warn!("mDNS ใช้ไม่ได้: {e}")).ok();
    }
}

impl Drop for Discovery {
    fn drop(&mut self) {
        if let Some(daemon) = &self.mdns {
            let _ = daemon.shutdown();
        }
        if let Some(probe) = &self.probe {
            probe.abort();
        }
        self.broadcast.abort();
    }
}

fn start_mdns(app: &App) -> Result<ServiceDaemon, mdns_sd::Error> {
    let daemon = ServiceDaemon::new()?;
    // ไม่ประกาศออกทาง VPN/virtual adapter: record ของ mDNS มี IP ของ interface อื่นรวมอยู่ด้วย
    // (shortcut: ปิดตามรายชื่อ adapter ตอนเริ่ม — adapter VPN ที่เพิ่งต่อทีหลังยังไม่ถูกปิดจนกว่าจะ restart)
    for iface in netif::list().into_iter().filter(|i| i.kind != "physical") {
        daemon.disable_interface(IfKind::Name(iface.name))?;
    }
    daemon.disable_interface(IfKind::IPv6)?; // agent ฟังแค่ IPv4 ไม่ประกาศที่อยู่ที่ต่อไม่ได้
    let fingerprint = app.host.fingerprint();
    let props = [
        ("device_id", app.node_id.as_str()),
        ("fp", fingerprint.as_str()),
        ("v", "1"),
    ];
    // ที่อยู่ว่าง + enable_addr_auto = ใช้ IP ปัจจุบันของทุก interface และตามเมื่อ interface เปลี่ยน
    let info = ServiceInfo::new(
        SERVICE_TYPE,
        &app.node_id,
        &format!("{}.local.", app.node_id),
        "",
        app.network.state().ports.port,
        &props[..],
    )?
    .enable_addr_auto();
    daemon.register(info)?;
    Ok(daemon)
}

/// ตอบ probe ด้วย beacon แบบ unicast กลับไปหาผู้ถาม
async fn answer_probes(sock: UdpSocket, app: App) {
    let mut buf = [0u8; 256];
    loop {
        let Ok((len, from)) = sock.recv_from(&mut buf).await else {
            continue;
        };
        let is_probe = serde_json::from_slice::<Value>(&buf[..len]).is_ok_and(|v| v["probe"] == PROBE_MAGIC);
        if is_probe {
            let _ = sock.send_to(beacon(&app).to_string().as_bytes(), from).await;
        }
    }
}

/// ทุก 3 วินาที ส่ง beacon ออกทุก interface IPv4 แบบ physical (อ่านรายการใหม่ทุกรอบ เผื่อ interface เปลี่ยน)
async fn broadcast_beacons(app: App) {
    let mut tick = tokio::time::interval(BEACON_INTERVAL);
    loop {
        tick.tick().await;
        let payload = beacon(&app).to_string();
        let port = app.network.state().ports.discovery_port;
        for iface in netif::list().into_iter().filter(|i| i.kind == "physical") {
            let (std::net::IpAddr::V4(ip), Some(bcast)) = (iface.ip, iface.broadcast) else {
                continue;
            };
            // bind ที่ IP ของ interface นั้น เพื่อให้ OS ส่งออกทาง interface นั้นจริง ๆ
            let Ok(sock) = UdpSocket::bind(SocketAddr::from((ip, 0))).await else {
                continue;
            };
            if sock.set_broadcast(true).is_ok() {
                let _ = sock.send_to(payload.as_bytes(), (bcast, port)).await;
            }
        }
    }
}
