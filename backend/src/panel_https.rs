use crate::{app::App, netif};
use anyhow::{Context, ensure};
use axum::{Extension, Router, extract::ConnectInfo};
use hyper_util::{
    rt::{TokioExecutor, TokioIo, TokioTimer},
    server::conn::auto::Builder,
    service::TowerToHyperService,
};
use rustls::{
    ServerConfig,
    crypto::ring,
    pki_types::{CertificateDer, PrivateKeyDer, pem::PemObject},
};
use std::{
    net::{IpAddr, Ipv4Addr},
    path::Path,
    sync::Arc,
    time::Duration,
};
use tokio::{net::TcpListener, sync::Semaphore, task::JoinSet, time::timeout};
use tokio_rustls::TlsAcceptor;
use x509_parser::{extensions::GeneralName, parse_x509_certificate};

pub const CERT_FILE: &str = "panel-tls.pem";
pub const KEY_FILE: &str = "panel-tls-key.pem";

pub struct Server {
    acceptor: TlsAcceptor,
    pub hosts: Vec<String>,
}

impl Server {
    pub fn load(dir: &Path) -> anyhow::Result<Option<Self>> {
        let cert = dir.join(CERT_FILE);
        let key = dir.join(KEY_FILE);
        if !cert.exists() && !key.exists() {
            return Ok(None);
        }
        Self::from_files(&cert, &key).map(Some)
    }

    pub fn from_files(cert: &Path, key: &Path) -> anyhow::Result<Self> {
        let certs: Vec<_> = CertificateDer::pem_file_iter(cert)?.collect::<Result<_, _>>()?;
        let (_, leaf) = parse_x509_certificate(certs.first().context("ไฟล์ไม่มีใบรับรอง")?.as_ref())
            .map_err(|_| anyhow::anyhow!("อ่านใบรับรอง HTTPS ไม่ได้"))?;
        ensure!(leaf.validity().is_valid(), "ใบรับรอง HTTPS หมดอายุหรือยังไม่เริ่มใช้");
        let names = leaf
            .subject_alternative_name()?
            .context("ใบรับรอง HTTPS ต้องมี Subject Alternative Name")?;
        let mut hosts = Vec::new();
        for name in &names.value.general_names {
            match name {
                GeneralName::DNSName(name) if valid_name(name) => hosts.push(name.to_ascii_lowercase()),
                GeneralName::IPAddress(ip) => {
                    if let Ok(ip) = <[u8; 4]>::try_from(*ip) {
                        let ip = Ipv4Addr::from(ip);
                        if !ip.is_loopback() && !ip.is_unspecified() {
                            hosts.push(ip.to_string());
                        }
                    }
                }
                _ => {}
            }
        }
        hosts.sort_unstable();
        hosts.dedup();
        ensure!(!hosts.is_empty(), "ใบรับรองต้องระบุ IPv4 หรือชื่อ DNS ที่ใช้กับ LAN");
        let mut tls = ServerConfig::builder_with_provider(Arc::new(ring::default_provider()))
            .with_protocol_versions(&[&rustls::version::TLS13])?
            .with_no_client_auth()
            .with_single_cert(certs, PrivateKeyDer::from_pem_file(key)?)?;
        tls.alpn_protocols = vec![b"http/1.1".to_vec()];
        Ok(Self {
            acceptor: TlsAcceptor::from(Arc::new(tls)),
            hosts,
        })
    }
}

fn valid_name(name: &str) -> bool {
    name.len() <= 253
        && !name.eq_ignore_ascii_case("localhost")
        && name.split('.').all(|label| {
            !label.is_empty()
                && label.len() <= 63
                && !label.starts_with('-')
                && !label.ends_with('-')
                && label.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
        })
}

#[derive(Clone, Copy)]
pub struct ListenerPort(pub u16);

pub async fn serve(listener: TcpListener, local: Router, lan: Router, app: App, port: u16) {
    let lan_slots = Arc::new(crate::tls::PreAuth::default());
    let local_slots = Arc::new(Semaphore::new(32));
    let mut connections = JoinSet::new();
    loop {
        let accepted = tokio::select! {
            result = listener.accept() => result,
            _ = connections.join_next(), if !connections.is_empty() => continue,
        };
        let Ok((tcp, peer)) = accepted else {
            tokio::time::sleep(Duration::from_millis(100)).await;
            continue;
        };
        let context = (Extension(ConnectInfo(peer)), Extension(ListenerPort(port)));
        if peer.ip().is_loopback() {
            let Ok(slot) = local_slots.clone().try_acquire_owned() else {
                continue;
            };
            let local = local.clone().layer(context);
            connections.spawn(async move {
                let _slot = slot;
                serve_http(tcp, local, true).await;
            });
        } else {
            let Some(server) = &app.network.lan else { continue };
            if !app.network.state().lan_enabled || !same_subnet(peer.ip(), tcp.local_addr().ok().map(|addr| addr.ip()))
            {
                continue;
            }
            let Some(slot) = lan_slots.try_enter(peer.ip()) else {
                continue;
            };
            let acceptor = server.acceptor.clone();
            let lan = lan.clone().layer(context);
            connections.spawn(async move {
                let _slot = slot;
                if let Ok(Ok(tls)) = timeout(Duration::from_secs(5), acceptor.accept(tcp)).await {
                    let _ = timeout(Duration::from_secs(30), serve_http(tls, lan, false)).await;
                }
            });
        }
    }
}

async fn serve_http(
    stream: impl tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin + Send + 'static,
    router: Router,
    local: bool,
) {
    let mut builder = Builder::new(TokioExecutor::new()).http1_only();
    builder
        .http1()
        .timer(TokioTimer::new())
        .header_read_timeout(Duration::from_secs(5))
        .keep_alive(local);
    let stream = TokioIo::new(stream);
    let service = TowerToHyperService::new(router);
    if local {
        let _ = builder.serve_connection_with_upgrades(stream, service).await;
    } else {
        let _ = builder.serve_connection(stream, service).await;
    }
}

fn same_subnet(peer: IpAddr, local: Option<IpAddr>) -> bool {
    let (IpAddr::V4(peer), Some(IpAddr::V4(local))) = (peer, local) else {
        return false;
    };
    if !peer.is_private() || !local.is_private() {
        return false;
    }
    netif::local_ipv4_mask(local)
        .is_some_and(|mask| u32::from(peer) & u32::from(mask) == u32::from(local) & u32::from(mask))
}
