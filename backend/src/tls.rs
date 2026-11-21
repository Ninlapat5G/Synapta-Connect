//! TLS 1.3 + mutual TLS: ตัวตนของเครื่องปลายทาง (host key), allowlist ของ client, และ accept loop
//!
//! ทั้งสองฝั่งใช้ใบรับรอง self-signed ที่มี Ed25519 key — ไม่มี CA; ความเชื่อใจมาจากการ "ปักหมุด key":
//!  - client ปักหมุด host key ที่ได้ตอนจับคู่ (เทียบ key ไม่ใช่ชื่อหรือวันหมดอายุของใบรับรอง)
//!  - agent ยอมเฉพาะใบรับรองของ client ที่ key อยู่ใน allowlist (paired_devices.json) ไม่งั้นตัดตั้งแต่ handshake
//!
//! client ที่ยังไม่มีใบรับรองเชื่อมต่อได้ (เพื่อจับคู่) แต่ถูกจำกัดให้เข้าได้เฉพาะ route สาธารณะ (ดู http.rs)

use crate::{app::App, auth::KILL_ALL, hex::to_hex};
use axum::{Extension, Router, extract::ConnectInfo};
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD as B64URL};
use hyper_util::{
    rt::{TokioExecutor, TokioIo},
    server::conn::auto,
    service::TowerToHyperService,
};
use rustls::{
    CertificateError, DigitallySignedStruct, DistinguishedName, Error as TlsError, ServerConfig, SignatureScheme,
    crypto::{CryptoProvider, WebPkiSupportedAlgorithms, ring},
    pki_types::{CertificateDer, PrivateKeyDer, UnixTime, pem::PemObject},
    server::danger::{ClientCertVerified, ClientCertVerifier},
};
use sha2::{Digest, Sha256};
use std::{collections::HashMap, net::IpAddr, path::Path, sync::Arc, time::Duration};
use tokio::{net::TcpListener, sync::Semaphore};
use tokio_rustls::TlsAcceptor;

const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(5);
/// อายุรวมสูงสุดของ connection ที่ไม่มีใบรับรอง (ใช้แค่ hello / จับคู่ ซึ่งจบใน request เดียว)
const ANONYMOUS_LIFETIME: Duration = Duration::from_secs(10);
/// connection ของอุปกรณ์ที่จับคู่แล้วพร้อมกันสูงสุด
const MAX_CONNECTIONS: usize = 256;
/// connection ที่ยังไม่ยืนยันตัว (กำลัง handshake หรือไม่มีใบรับรอง) ทั้งระบบ / ต่อ IP ต้นทาง
const MAX_PRE_AUTH: usize = 64;
const MAX_PRE_AUTH_PER_IP: usize = 8;
const ED25519_OID: &str = "1.3.101.112";

/// ตัวตนของเครื่องนี้: คู่กุญแจ Ed25519 + ใบรับรอง self-signed (สร้างครั้งเดียว เก็บใน data_dir)
pub struct HostIdentity {
    cert: CertificateDer<'static>,
    key: PrivateKeyDer<'static>,
    pub public_key: [u8; 32],
}

/// public key Ed25519 ดิบ 32 ไบต์ของใบรับรอง (None = ไม่ใช่ Ed25519 หรืออ่านไม่ได้)
pub fn ed25519_key(cert: &CertificateDer<'_>) -> Option<[u8; 32]> {
    let (_, parsed) = x509_parser::parse_x509_certificate(cert).ok()?;
    let spki = parsed.public_key();
    if spki.algorithm.algorithm.to_id_string() != ED25519_OID {
        return None;
    }
    spki.subject_public_key.data.as_ref().try_into().ok()
}

/// `SHA256:<hex>` ของ public key ดิบ — ค่านี้คือ `host_pubkey_fingerprint` ใน beacon
pub fn fingerprint(public_key: &[u8; 32]) -> String {
    format!("SHA256:{}", to_hex(&Sha256::digest(public_key)))
}

impl HostIdentity {
    pub fn load_or_create(dir: &Path) -> anyhow::Result<HostIdentity> {
        let (cert_file, key_file) = (dir.join("host.crt"), dir.join("host.key"));
        if !(cert_file.exists() && key_file.exists()) {
            let key = rcgen::KeyPair::generate_for(&rcgen::PKCS_ED25519)?;
            let cert = rcgen::CertificateParams::new(vec!["synapta-connect".to_string()])?.self_signed(&key)?;
            std::fs::create_dir_all(dir)?;
            crate::platform::write_private(&key_file, key.serialize_pem().as_bytes())?;
            std::fs::write(&cert_file, cert.pem())?;
        }
        let cert = CertificateDer::from_pem_file(&cert_file)?;
        let key = PrivateKeyDer::from_pem_file(&key_file)?;
        let public_key = ed25519_key(&cert)
            .ok_or_else(|| anyhow::anyhow!("host.crt ไม่ใช่ Ed25519 — ลบ host.crt/host.key แล้วเปิดใหม่"))?;
        Ok(HostIdentity { cert, key, public_key })
    }

    pub fn fingerprint(&self) -> String {
        fingerprint(&self.public_key)
    }

    /// base64url ของ public key ดิบ (ค่าที่ส่งให้ client ปักหมุดตอนจับคู่)
    pub fn public_key_b64(&self) -> String {
        B64URL.encode(self.public_key)
    }
}

/// ยอมเฉพาะใบรับรอง client ที่ key อยู่ใน allowlist; ไม่แสดงใบรับรองเลยก็ผ่านได้ (ไปจับคู่)
struct Allowlist {
    app: App,
    algorithms: WebPkiSupportedAlgorithms,
}

// ClientCertVerifier ต้องเป็น Debug แต่ AppState มี secret/handle ที่ไม่ควรพิมพ์
impl std::fmt::Debug for Allowlist {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Allowlist")
    }
}

impl ClientCertVerifier for Allowlist {
    fn offer_client_auth(&self) -> bool {
        true
    }

    fn client_auth_mandatory(&self) -> bool {
        false
    }

    fn root_hint_subjects(&self) -> &[DistinguishedName] {
        &[]
    }

    fn verify_client_cert(
        &self,
        end_entity: &CertificateDer<'_>,
        _intermediates: &[CertificateDer<'_>],
        _now: UnixTime,
    ) -> Result<ClientCertVerified, TlsError> {
        let key = ed25519_key(end_entity).ok_or(TlsError::InvalidCertificate(CertificateError::BadEncoding))?;
        match self.app.auth.device_for_key(&key) {
            Some(_) => Ok(ClientCertVerified::assertion()),
            None => Err(TlsError::InvalidCertificate(CertificateError::UnknownIssuer)),
        }
    }

    // เปิดเฉพาะ TLS 1.3 จึงไม่มีทางมาถึงฟังก์ชันนี้ — ปฏิเสธไว้เผื่อ
    fn verify_tls12_signature(
        &self,
        _: &[u8],
        _: &CertificateDer<'_>,
        _: &DigitallySignedStruct,
    ) -> Result<rustls::client::danger::HandshakeSignatureValid, TlsError> {
        Err(TlsError::PeerIncompatible(rustls::PeerIncompatible::Tls12NotOffered))
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<rustls::client::danger::HandshakeSignatureValid, TlsError> {
        rustls::crypto::verify_tls13_signature(message, cert, dss, &self.algorithms)
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.algorithms.supported_schemes()
    }
}

pub fn server_config(app: &App) -> anyhow::Result<Arc<ServerConfig>> {
    let provider: Arc<CryptoProvider> = Arc::new(ring::default_provider());
    let verifier = Allowlist {
        app: app.clone(),
        algorithms: provider.signature_verification_algorithms,
    };
    let mut config = ServerConfig::builder_with_provider(provider)
        .with_protocol_versions(&[&rustls::version::TLS13])?
        .with_client_cert_verifier(Arc::new(verifier))
        .with_single_cert(vec![app.host.cert.clone()], app.host.key.clone_key())?;
    // http/1.1 อย่างเดียว: WebSocket ผ่าน h2 (RFC 8441) client ส่วนใหญ่ไม่รองรับ
    config.alpn_protocols = vec![b"http/1.1".to_vec()];
    Ok(Arc::new(config))
}

/// ตัวตนของ client ที่ผูกกับ connection นี้ (None = ไม่ได้แสดงใบรับรอง ใช้ได้แค่ route สาธารณะ)
#[derive(Clone)]
pub struct ClientIdentity(pub Option<String>);

/// รอจนกว่า connection นี้ต้องถูกตัด: อุปกรณ์ของมันถูก revoke หรือมีคำสั่งตัดทั้งหมด
/// (WebSocket หลัง upgrade เป็นของ task แยกที่ไม่ตายตามเมื่อ connection ถูกทิ้ง จึงต้องเรียกฟังก์ชันนี้เองใน ws_session ด้วย)
pub async fn wait_for_kill(app: &App, device: &Option<String>, mut kills: tokio::sync::broadcast::Receiver<String>) {
    use tokio::sync::broadcast::error::RecvError;
    loop {
        match kills.recv().await {
            Ok(id) if id == KILL_ALL || Some(&id) == device.as_ref() => return,
            Ok(_) => {}
            // พลาดข้อความไปบางส่วน: เช็คตรง ๆ ว่าอุปกรณ์ยังอยู่ใน allowlist ไหม
            Err(RecvError::Lagged(_)) if device.as_ref().is_some_and(|d| !app.auth.is_active(d)) => return,
            Err(RecvError::Lagged(_)) => {}
            Err(RecvError::Closed) => std::future::pending::<()>().await,
        }
    }
}

/// ช่อง connection ที่ "ยังไม่รู้ว่าเป็นใคร" (กำลัง handshake หรือไม่มีใบรับรอง) — แยกจากช่องของอุปกรณ์ที่จับคู่แล้ว
/// และจำกัดต่อ IP ต้นทาง ไม่งั้นคนเดียวเปิด connection เปล่าค้างไว้ก็แย่งช่องของเจ้าของเครื่องจนต่อไม่ได้
#[derive(Default)]
struct PreAuth(std::sync::Mutex<HashMap<IpAddr, usize>>);

struct PreAuthSlot {
    owner: Arc<PreAuth>,
    ip: IpAddr,
}

impl PreAuth {
    fn try_enter(self: &Arc<Self>, ip: IpAddr) -> Option<PreAuthSlot> {
        let mut by_ip = self.0.lock().unwrap();
        if by_ip.values().sum::<usize>() >= MAX_PRE_AUTH || by_ip.get(&ip).copied().unwrap_or(0) >= MAX_PRE_AUTH_PER_IP
        {
            return None;
        }
        *by_ip.entry(ip).or_insert(0) += 1;
        Some(PreAuthSlot {
            owner: self.clone(),
            ip,
        })
    }
}

impl Drop for PreAuthSlot {
    fn drop(&mut self) {
        let mut by_ip = self.owner.0.lock().unwrap();
        if let Some(n) = by_ip.get_mut(&self.ip) {
            *n -= 1;
            if *n == 0 {
                by_ip.remove(&self.ip);
            }
        }
    }
}

/// รับ connection TLS แล้วส่งเข้า router — วนจนโปรแกรมปิด
pub async fn serve(listener: TcpListener, config: Arc<ServerConfig>, router: Router, app: App) {
    let acceptor = TlsAcceptor::from(config);
    let slots = Arc::new(Semaphore::new(MAX_CONNECTIONS));
    let pre_auth = Arc::new(PreAuth::default());
    loop {
        let (tcp, peer) = match listener.accept().await {
            Ok(accepted) => accepted,
            Err(e) => {
                tracing::warn!("accept ล้มเหลว: {e}");
                tokio::time::sleep(Duration::from_millis(100)).await;
                continue;
            }
        };
        let Some(pre_slot) = pre_auth.try_enter(peer.ip()) else {
            continue; // IP นี้ (หรือทั้งระบบ) มี connection ที่ยังไม่ยืนยันตัวเต็มแล้ว
        };
        let (acceptor, router, app, slots) = (acceptor.clone(), router.clone(), app.clone(), slots.clone());
        tokio::spawn(async move {
            let Ok(Ok(tls)) = tokio::time::timeout(HANDSHAKE_TIMEOUT, acceptor.accept(tcp)).await else {
                return;
            };
            let device = tls
                .get_ref()
                .1
                .peer_certificates()
                .and_then(|chain| chain.first())
                .and_then(ed25519_key)
                .and_then(|key| app.auth.device_for_key(&key));
            // อุปกรณ์ที่จับคู่แล้วย้ายไปใช้ช่องของตัวเอง; คนไม่มีใบรับรองถือช่อง pre-auth ต่อไปจนจบ
            let (_pre_slot, _slot) = match &device {
                Some(_) => match slots.try_acquire_owned() {
                    Ok(slot) => (None, Some(slot)),
                    Err(_) => return,
                },
                None => (Some(pre_slot), None),
            };
            let router = router
                .layer(Extension(ClientIdentity(device.clone())))
                .layer(Extension(ConnectInfo(peer)));
            let mut builder = auto::Builder::new(TokioExecutor::new());
            if device.is_none() {
                // hello/จับคู่ ใช้ครั้งเดียวจบ — ไม่ keep-alive และมีอายุรวมจำกัด กัน connection เปล่าค้างยึดช่อง
                builder.http1().keep_alive(false);
            }
            let conn = builder.serve_connection_with_upgrades(TokioIo::new(tls), TowerToHyperService::new(router));
            tokio::pin!(conn);
            let kills = app.auth.subscribe_kills();
            let anonymous = device.is_none();
            let lifetime = async {
                if anonymous {
                    tokio::time::sleep(ANONYMOUS_LIFETIME).await;
                } else {
                    std::future::pending::<()>().await;
                }
            };
            tokio::select! {
                _ = conn.as_mut() => {}
                _ = lifetime => {}
                _ = wait_for_kill(&app, &device, kills) => tracing::info!(%peer, "ตัด connection"),
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pre_auth_slots_are_capped_per_ip_and_released_on_drop() {
        let limiter = Arc::new(PreAuth::default());
        let attacker: IpAddr = "10.0.0.1".parse().unwrap();
        let owner: IpAddr = "10.0.0.2".parse().unwrap();
        let held: Vec<_> = (0..MAX_PRE_AUTH_PER_IP)
            .map(|_| limiter.try_enter(attacker).unwrap())
            .collect();
        assert!(limiter.try_enter(attacker).is_none(), "IP เดียวยึดได้ไม่เกินเพดานต่อ IP");
        assert!(limiter.try_enter(owner).is_some(), "IP อื่นต้องยังเข้าได้");
        drop(held);
        assert!(limiter.try_enter(attacker).is_some(), "คืนช่องเมื่อ connection จบ");
    }
}
