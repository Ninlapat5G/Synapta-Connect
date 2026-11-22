//! ทดสอบ agent ผ่านเครือข่ายจริง (TLS 1.3 + mTLS บนพอร์ตสุ่มของ loopback) โดยทำตัวเป็น client อิสระ:
//! สร้างกุญแจ/ใบรับรองเอง, ปักหมุด host key, ทำ SPAKE2 เอง — เพื่อให้เทสต์ยืนยันสัญญาของโปรโตคอล ไม่ใช่โค้ดของ agent
//! ไม่ครอบคลุม screen.* / input.* ในชุดปกติเพราะต้องมีหน้าจอและเมาส์จริง (มี test แบบ #[ignore] ให้รันมือ)

use base64::{
    Engine,
    engine::general_purpose::{STANDARD as B64, URL_SAFE_NO_PAD as B64URL},
};
use futures_util::{SinkExt, StreamExt};
use hmac::{Hmac, KeyInit, Mac};
use rustls::{
    ClientConfig, DigitallySignedStruct, Error as TlsError, SignatureScheme,
    client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier},
    crypto::{WebPkiSupportedAlgorithms, ring},
    pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer, ServerName, UnixTime},
};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use spake2::{Ed25519Group, Identity, Password, Spake2};
use std::{net::SocketAddr, path::PathBuf, sync::Arc, time::Duration};
use synapta_connect::{
    app::{App, AppState},
    config::Config,
    hex::{random_id, to_hex},
    http, local, tls,
};
use tokio_tungstenite::tungstenite::Message;

const CLIENT_ID: &[u8] = b"synapta-connect client";
const HOST_ID: &[u8] = b"synapta-connect host";

/// client ปักหมุด host key: ยอมเฉพาะเซิร์ฟเวอร์ที่ public key ตรงกับที่รู้ (ไม่สนชื่อ/วันหมดอายุของใบรับรอง)
#[derive(Debug)]
struct PinnedHost {
    key: [u8; 32],
    algorithms: WebPkiSupportedAlgorithms,
}

impl ServerCertVerifier for PinnedHost {
    fn verify_server_cert(
        &self,
        end_entity: &CertificateDer<'_>,
        _: &[CertificateDer<'_>],
        _: &ServerName<'_>,
        _: &[u8],
        _: UnixTime,
    ) -> Result<ServerCertVerified, TlsError> {
        match tls::ed25519_key(end_entity) {
            Some(k) if k == self.key => Ok(ServerCertVerified::assertion()),
            _ => Err(TlsError::General("host key ไม่ตรงกับที่ปักหมุดไว้".into())),
        }
    }

    fn verify_tls12_signature(
        &self,
        _: &[u8],
        _: &CertificateDer<'_>,
        _: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, TlsError> {
        Err(TlsError::General("tls12".into()))
    }

    fn verify_tls13_signature(
        &self,
        m: &[u8],
        c: &CertificateDer<'_>,
        d: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, TlsError> {
        rustls::crypto::verify_tls13_signature(m, c, d, &self.algorithms)
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.algorithms.supported_schemes()
    }
}

struct Identity25519 {
    key: rcgen::KeyPair,
    cert: CertificateDer<'static>,
}

fn new_identity() -> Identity25519 {
    let key = rcgen::KeyPair::generate_for(&rcgen::PKCS_ED25519).unwrap();
    let cert = rcgen::CertificateParams::new(vec!["client".to_string()])
        .unwrap()
        .self_signed(&key)
        .unwrap();
    Identity25519 {
        cert: cert.der().clone(),
        key,
    }
}

impl Identity25519 {
    fn public_key(&self) -> Vec<u8> {
        self.key.public_key_raw().to_vec()
    }
}

struct Agent {
    app: App,
    port: u16,
    local_base: String,
    me: Identity25519,
    dir: PathBuf,
}

fn client_config(pin: [u8; 32], identity: Option<&Identity25519>) -> ClientConfig {
    let provider = Arc::new(ring::default_provider());
    let builder = ClientConfig::builder_with_provider(provider.clone())
        .with_protocol_versions(&[&rustls::version::TLS13])
        .unwrap()
        .dangerous()
        .with_custom_certificate_verifier(Arc::new(PinnedHost {
            key: pin,
            algorithms: provider.signature_verification_algorithms,
        }));
    let mut config = match identity {
        Some(id) => builder
            .with_client_auth_cert(
                vec![id.cert.clone()],
                PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(id.key.serialize_der())),
            )
            .unwrap(),
        None => builder.with_no_client_auth(),
    };
    config.alpn_protocols = vec![b"http/1.1".to_vec()];
    config
}

async fn start_with(file_roots: Vec<PathBuf>) -> Agent {
    synapta_connect::platform::init(); // เหมือน main: ต้องมีก่อนจับภาพ/จำลองเมาส์
    let _ = ring::default_provider().install_default(); // reqwest (no-provider) ต้องมี provider เริ่มต้น แม้ client ธรรมดา
    let dir = std::env::temp_dir().join(format!("synapta-test-{}", random_id(4)));
    let config = Config {
        mode: synapta_connect::config::Mode::Console,
        port: 0,
        discovery_port: 0,
        local_port: 0,
        data_dir: dir.clone(),
        name: "test".into(),
        file_roots,
    };
    let app = AppState::new(config).unwrap();

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let server_config = tls::server_config(&app).unwrap();
    tokio::spawn(tls::serve(
        listener,
        server_config,
        http::router(app.clone()),
        app.clone(),
    ));

    let local = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let local_base = format!("http://{}", local.local_addr().unwrap());
    let local_router = local::router(app.clone()).into_make_service_with_connect_info::<SocketAddr>();
    tokio::spawn(async move { axum::serve(local, local_router).await.unwrap() });

    Agent {
        app,
        port,
        local_base,
        me: new_identity(),
        dir,
    }
}

async fn start() -> Agent {
    start_with(vec![]).await
}

/// เปิดโหมดจับคู่แล้วจับคู่ตัวเองสำเร็จ
async fn start_paired() -> Agent {
    let a = start().await;
    let otp = a.app.auth.open_pairing().otp;
    a.pair(&otp, a.app.host.public_key).await.unwrap();
    a
}

fn mac(key: &[u8], label: &[u8], host: &[u8], client: &[u8]) -> Hmac<Sha256> {
    let mut m = Hmac::<Sha256>::new_from_slice(key).unwrap();
    m.update(label);
    m.update(host);
    m.update(client);
    m
}

impl Agent {
    fn url(&self, path: &str) -> String {
        format!("https://127.0.0.1:{}{path}", self.port)
    }

    fn http(&self, identity: Option<&Identity25519>) -> reqwest::Client {
        reqwest::Client::builder()
            .use_preconfigured_tls(client_config(self.app.host.public_key, identity))
            .build()
            .unwrap()
    }

    /// client ของอุปกรณ์ที่จับคู่แล้ว (แสดงใบรับรอง)
    fn authed(&self) -> reqwest::Client {
        self.http(Some(&self.me))
    }

    /// จับคู่ด้วย SPAKE2 — `host_pubkey_seen` คือ host key ที่ client เห็นบน TLS (ปกติเท่ากับของจริง)
    /// Err = (status, body) ของขั้นที่ล้ม
    async fn pair(&self, otp: &str, host_pubkey_seen: [u8; 32]) -> Result<Value, (u16, Value)> {
        let anon = self.http(None);
        let (state, client_msg) = Spake2::<Ed25519Group>::start_a(
            &Password::new(otp.as_bytes()),
            &Identity::new(CLIENT_ID),
            &Identity::new(HOST_ID),
        );
        let client_pub = self.me.public_key();
        let url = self.url("/api/v1/auth/spake2-handshake");
        let post = |body: Value| {
            let (anon, url) = (anon.clone(), url.clone());
            async move {
                let res = anon.post(url).json(&body).send().await.unwrap();
                let status = res.status().as_u16();
                let body: Value = res.json().await.unwrap();
                if status == 200 { Ok(body) } else { Err((status, body)) }
            }
        };
        let r1 = post(json!({"step": 1, "client_msg": B64.encode(client_msg), "client_pubkey": B64URL.encode(&client_pub), "client_name": "test-client"})).await?;
        let server_msg = B64.decode(r1["server_msg"].as_str().unwrap()).unwrap();
        let key = state.finish(&server_msg).unwrap();
        let confirm = mac(&key, CLIENT_ID, &host_pubkey_seen, &client_pub)
            .finalize()
            .into_bytes();
        let r2 =
            post(json!({"step": 2, "handshake_id": r1["handshake_id"], "client_confirm": B64.encode(confirm)})).await?;
        // client ต้องตรวจฝั่ง agent ด้วย (พิสูจน์ว่า agent รู้ OTP และเห็น key เดียวกัน)
        let host_confirm = B64.decode(r2["host_confirm"].as_str().unwrap()).unwrap();
        mac(&key, HOST_ID, &host_pubkey_seen, &client_pub)
            .verify_slice(&host_confirm)
            .expect("host_confirm ไม่ถูกต้อง");
        assert_eq!(r2["host_pubkey"], B64URL.encode(host_pubkey_seen));
        Ok(r2)
    }

    async fn rpc_raw(&self, method: &str, params: Value) -> Value {
        let body = json!({"jsonrpc": "2.0", "id": 1, "method": method, "params": params});
        self.authed()
            .post(self.url("/api/v1/rpc"))
            .json(&body)
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap()
    }

    /// result ของ method (panic ถ้าเป็น error)
    async fn call(&self, method: &str, params: Value) -> Value {
        let r = self.rpc_raw(method, params).await;
        assert!(r.get("error").is_none(), "{method} ล้มเหลว: {r}");
        r["result"].clone()
    }

    async fn ws(&self) -> tokio_tungstenite::WebSocketStream<tokio_rustls::client::TlsStream<tokio::net::TcpStream>> {
        let tcp = tokio::net::TcpStream::connect(("127.0.0.1", self.port)).await.unwrap();
        let connector =
            tokio_rustls::TlsConnector::from(Arc::new(client_config(self.app.host.public_key, Some(&self.me))));
        let tls = connector
            .connect(ServerName::try_from("synapta-connect").unwrap(), tcp)
            .await
            .unwrap();
        tokio_tungstenite::client_async(format!("wss://127.0.0.1:{}/api/v1/ws", self.port), tls)
            .await
            .unwrap()
            .0
    }

    /// URL ดาวน์โหลดไฟล์ (path ถูก encode ให้)
    fn files_url(&self, path: &str) -> reqwest::Url {
        reqwest::Url::parse_with_params(&self.url("/api/v1/files"), [("path", path)]).unwrap()
    }

    fn device_id(&self) -> String {
        synapta_connect::auth::device_id(&self.me.public_key())
    }

    async fn local(&self, method: reqwest::Method, path: &str) -> reqwest::Response {
        reqwest::Client::new()
            .request(method, format!("{}{path}", self.local_base))
            .header("x-synapta-local", &self.app.local_token)
            .send()
            .await
            .unwrap()
    }
}

impl Drop for Agent {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

#[tokio::test]
async fn spake2_pairing_then_mtls_and_revoke() {
    let a = start().await;
    let anon = a.http(None);

    // ไม่ต้องมีใบรับรองก็เรียก hello ได้ และได้ fingerprint ไว้ปักหมุด
    let hello: Value = anon
        .get(a.url("/api/v1/hello"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(hello["host_pubkey_fingerprint"], a.app.host.fingerprint());
    assert_eq!(hello["pairing_mode"], false);

    // route ที่ต้อง login: ไม่มีใบรับรอง = 401
    let res = anon.post(a.url("/api/v1/rpc")).body("{}").send().await.unwrap();
    assert_eq!(res.status(), 401);

    // โหมดจับคู่ปิดอยู่ตั้งแต่เริ่ม
    let (status, body) = a.pair("12345678", a.app.host.public_key).await.unwrap_err();
    assert_eq!(
        (status, body["error"]["code"].as_str().unwrap()),
        (403, "pairing_closed")
    );

    let otp = a.app.auth.open_pairing().otp;
    let paired = a.pair(&otp, a.app.host.public_key).await.unwrap();
    assert_eq!(paired["device_id"], a.device_id());
    assert!(a.app.auth.current_otp().is_none(), "จับคู่สำเร็จแล้วต้องปิดโหมดจับคู่");
    assert!(a.dir.join("paired_devices.json").exists());

    // OTP ใช้ซ้ำไม่ได้
    assert_eq!(a.pair(&otp, a.app.host.public_key).await.unwrap_err().0, 403);

    assert_eq!(a.call("rpc.ping", json!({})).await["pong"], true);

    // อุปกรณ์ที่ไม่เคยจับคู่: ถูกตัดตั้งแต่ TLS handshake
    let stranger = new_identity();
    let res = a.http(Some(&stranger)).get(a.url("/api/v1/hello")).send().await;
    assert!(res.is_err(), "ใบรับรองที่ไม่อยู่ใน allowlist ต้องเชื่อมต่อไม่ได้");

    // client ที่ปักหมุด host key ผิดต้องไม่ยอมคุย (กัน MITM)
    let wrong_pin = reqwest::Client::builder()
        .use_preconfigured_tls(client_config([9u8; 32], None))
        .build()
        .unwrap();
    assert!(wrong_pin.get(a.url("/api/v1/hello")).send().await.is_err());

    // revoke = ลบออกจาก allowlist แล้วใช้ต่อไม่ได้ทันที
    assert_eq!(
        a.call("session.revoke", json!({"session_id": a.device_id()})).await["revoked"],
        true
    );
    assert!(a.authed().get(a.url("/api/v1/hello")).send().await.is_err());
}

#[tokio::test]
async fn five_wrong_guesses_destroy_otp_and_lock_pairing() {
    let a = start().await;
    a.app.auth.open_pairing();
    for _ in 0..5 {
        assert_eq!(a.pair("00000000", a.app.host.public_key).await.unwrap_err().0, 403);
    }
    assert!(a.app.auth.current_otp().is_none(), "เดาผิดครบ 5 ครั้งต้องทำลาย OTP");
    // เปิดโหมดจับคู่ใหม่ระหว่างล็อกก็ยังเดาไม่ได้
    let fresh = a.app.auth.open_pairing().otp;
    assert_eq!(a.pair(&fresh, a.app.host.public_key).await.unwrap_err().0, 429);
}

#[tokio::test]
async fn pairing_fails_when_client_saw_a_different_host_key() {
    // จำลองตัวกลางที่สวม TLS: client เห็น host key ไม่ตรงกับที่ agent ใช้จริง → confirm ไม่ผ่านแม้ OTP ถูก
    let a = start().await;
    let otp = a.app.auth.open_pairing().otp;
    assert_eq!(a.pair(&otp, [5u8; 32]).await.unwrap_err().0, 403);
    assert!(a.app.auth.list().is_empty(), "ต้องไม่มีอุปกรณ์ถูกลงทะเบียน");
}

#[tokio::test]
async fn browsers_are_rejected() {
    let a = start_paired().await;
    let res = a
        .authed()
        .post(a.url("/api/v1/rpc"))
        .header("origin", "https://evil.example")
        .body("{}")
        .send()
        .await
        .unwrap();
    assert_eq!(res.status(), 403);
}

#[tokio::test]
async fn exec_and_profile() {
    let a = start_paired().await;
    let r = a.call("exec.run", json!({"command": "echo hello"})).await;
    assert_eq!(r["stdout"].as_str().unwrap().trim(), "hello");
    assert_eq!(r["exit_code"], 0);

    let r = a.call("exec.run", json!({"command": "exit 3"})).await;
    assert_eq!(r["exit_code"], 3);

    // ภาษาไทยต้องไม่เพี้ยนผ่าน codepage ของ Windows
    let r = a.call("exec.run", json!({"command": "echo สวัสดี"})).await;
    assert_eq!(r["stdout"].as_str().unwrap().trim(), "สวัสดี");

    let sleep = if cfg!(windows) { "Start-Sleep 5" } else { "sleep 5" };
    let r = a.call("exec.run", json!({"command": sleep, "timeout_ms": 300})).await;
    assert_eq!(r["timed_out"], true);

    let p = a.call("system.profile", json!({"include_env": true})).await;
    assert!(p["cpu"]["logical_cores"].as_u64().unwrap() > 0);
    assert!(p["memory"]["total_bytes"].as_u64().unwrap() > 0);
    assert!(["standard", "administrator", "system", "root"].contains(&p["privilege"]["level"].as_str().unwrap()));
    let env = p["env"].as_object().unwrap();
    assert!(!env.is_empty());
    assert!(env.values().any(|v| v == "***"), "ตัวแปรนอกรายการปลอดภัยต้องถูกปิดบัง");
}

/// Windows: หลัง timeout โปรเซสลูกของ shell (ping ที่เปิดจาก cmd) ต้องไม่ค้างอยู่
#[cfg(windows)]
#[tokio::test]
async fn exec_timeout_kills_child_processes_of_the_shell() {
    let a = start_paired().await;

    let pid_file = a.dir.join("ping.pid");
    let path = pid_file.display().to_string().replace('\'', "''");
    let spawn = format!(
        "$ping = Start-Process \"$env:SystemRoot\\System32\\PING.EXE\" -ArgumentList '-n','37','127.0.0.1' -WindowStyle Hidden -PassThru; [IO.File]::WriteAllText('{path}', [string]$ping.Id)"
    );
    let take_pid = || std::fs::read_to_string(&pid_file).unwrap().parse::<u32>().unwrap();
    let inspect = |pid| {
        format!(
            "$p = Get-Process -Id {pid} -ErrorAction SilentlyContinue; if ($p -and $p.ProcessName -eq 'PING') {{ Stop-Process -InputObject $p -Force; 'alive' }}"
        )
    };

    let r = a
        .call(
            "exec.run",
            json!({"command": format!("{spawn}; Start-Sleep -Seconds 30"), "timeout_ms": 3000}),
        )
        .await;
    assert_eq!(r["timed_out"], true);
    let left = a.call("exec.run", json!({"command": inspect(take_pid())})).await;
    assert_eq!(left["stdout"].as_str().unwrap().trim(), "", "ping ลูกต้องถูกฆ่าไปด้วย");

    std::fs::remove_file(&pid_file).unwrap();
    let started = a.call("exec.run", json!({"command": spawn})).await;
    assert_eq!(started["exit_code"], 0, "{started}");
    let alive = a.call("exec.run", json!({"command": inspect(take_pid())})).await;
    assert_eq!(
        alive["stdout"].as_str().unwrap().trim(),
        "alive",
        "โปรแกรมที่คำสั่งเปิดทิ้งไว้ต้องยังอยู่"
    );
}

#[tokio::test]
async fn upload_resumes_verifies_checksum_and_downloads_by_range() {
    let a = start_paired().await;
    let http = a.authed();
    let data: Vec<u8> = (0..1_500_000u32).map(|i| (i * 31 % 251) as u8).collect();
    let sha = to_hex(&Sha256::digest(&data));
    let dest = a.dir.join("incoming").join("big.bin");
    let dest_str = dest.to_string_lossy().to_string();
    let chunk = 512 * 1024;

    let begin = |sha: String| {
        a.call(
            "transfer.begin",
            json!({"path": dest_str, "size": data.len(), "sha256": sha}),
        )
    };
    let put = |id: String, offset: usize, end: usize| {
        http.put(a.url(&format!("/api/v1/transfers/{id}?offset={offset}")))
            .body(data[offset..end].to_vec())
            .send()
    };

    let first = begin(sha.clone()).await;
    assert_eq!(first["offset"], 0);
    let id = first["transfer_id"].as_str().unwrap().to_string();
    assert_eq!(put(id.clone(), 0, chunk).await.unwrap().status(), 200);

    // สัญญาณหลุดแล้วเริ่มใหม่: ต้องได้ offset ที่ค้างอยู่
    let again = begin(sha.clone()).await;
    assert_eq!(again["offset"], chunk);
    assert_eq!(again["resumed"], true);
    let id = again["transfer_id"].as_str().unwrap().to_string();

    // ส่งข้าม offset ไม่ได้
    assert_eq!(put(id.clone(), chunk * 2, data.len()).await.unwrap().status(), 409);
    assert_eq!(put(id.clone(), chunk, chunk * 2).await.unwrap().status(), 200);
    assert_eq!(put(id.clone(), chunk * 2, data.len()).await.unwrap().status(), 200);

    let done = a.call("transfer.finish", json!({"transfer_id": id})).await;
    assert_eq!(done["sha256"], sha);
    assert_eq!(std::fs::read(&dest).unwrap(), data);

    // ดาวน์โหลดเฉพาะช่วง (Range) — ใช้ต่อดาวน์โหลดที่ค้าง
    let res = http
        .get(a.files_url(&dest_str))
        .header("range", "bytes=10-19")
        .send()
        .await
        .unwrap();
    assert_eq!(res.status(), 206);
    assert_eq!(res.bytes().await.unwrap().as_ref(), &data[10..20]);
    assert_eq!(a.call("transfer.stat", json!({"path": dest_str})).await["sha256"], sha);

    // checksum ไม่ตรง: ต้องปฏิเสธและลบไฟล์ค้าง
    let bad_dest = a.dir.join("bad.bin").to_string_lossy().to_string();
    let b = a
        .call(
            "transfer.begin",
            json!({"path": bad_dest, "size": 4, "sha256": "0".repeat(64)}),
        )
        .await;
    let id = b["transfer_id"].as_str().unwrap().to_string();
    let res = http
        .put(a.url(&format!("/api/v1/transfers/{id}?offset=0")))
        .body("abcd")
        .send()
        .await
        .unwrap();
    assert_eq!(res.status(), 200);
    let r = a.rpc_raw("transfer.finish", json!({"transfer_id": id})).await;
    assert!(r["error"]["message"].as_str().unwrap().contains("checksum"));
    assert!(!std::path::Path::new(&bad_dest).exists());
    assert!(!std::path::Path::new(&format!("{bad_dest}.synapta-part")).exists());
}

#[tokio::test]
async fn concurrent_uploads_to_one_destination_are_refused_by_operation_id() {
    let a = start_paired().await;
    let dest = a.dir.join("shared.bin").to_string_lossy().to_string();
    let begin = |op: Option<&str>| {
        let mut params = json!({"path": dest, "size": 4, "sha256": "0".repeat(64)});
        if let Some(op) = op {
            params["operation_id"] = json!(op);
        }
        a.rpc_raw("transfer.begin", params)
    };
    let first = begin(Some("a1")).await;
    assert!(first.get("error").is_none(), "{first}");
    // งานอื่นเขียนที่เดียวกันระหว่างงานแรกยังเดิน = ปฏิเสธ
    let other = begin(Some("b2")).await;
    assert_eq!(other["error"]["code"], -32002, "{other}");
    // งานเดิมลองใหม่ = ได้ และ transfer_id เก่าใช้ไม่ได้แล้ว
    let retry = begin(Some("a1")).await;
    assert!(retry.get("error").is_none(), "{retry}");
    let old = first["result"]["transfer_id"].as_str().unwrap();
    let r = a.rpc_raw("transfer.abort", json!({"transfer_id": old})).await;
    assert!(r.get("error").is_some(), "transfer_id ของรอบก่อนต้องถูกทิ้ง: {r}");
    // client รุ่นเก่า (ไม่มี operation_id) ทำงานแบบเดิม
    assert!(begin(None).await.get("error").is_none());
}

#[tokio::test]
async fn paired_device_can_read_update_status_and_is_refused_unknown_versions() {
    let a = start_paired().await;
    let status = a.call("update.status", json!({})).await;
    assert_eq!(status["current_version"], env!("CARGO_PKG_VERSION"));
    assert!(status.get("web").is_some());
    // ยังไม่ได้ตรวจพบรุ่นนี้ = ติดตั้งไม่ได้ (กันสั่งติดตั้งรุ่นที่ไม่ได้มาจาก release ที่ตรวจลายเซ็นแล้ว)
    let r = a.rpc_raw("update.install", json!({"version": "9.9.9"})).await;
    assert_eq!(r["error"]["code"], -32602, "{r}");
}

#[tokio::test]
async fn open_path_lists_roots_folders_and_previews_files() {
    let a = start_paired().await;
    let roots = a.call("file.open_path", json!({})).await;
    assert_eq!(roots["kind"], "roots");
    assert!(!roots["entries"].as_array().unwrap().is_empty());

    let folder = a.dir.join("browse");
    std::fs::create_dir_all(folder.join("sub")).unwrap();
    std::fs::write(folder.join("note.txt"), "hello").unwrap();
    let listing = a
        .call(
            "file.open_path",
            json!({"path": folder.to_string_lossy(), "limit": 100}),
        )
        .await;
    assert_eq!(listing["kind"], "directory");
    let names: Vec<_> = listing["entries"]
        .as_array()
        .unwrap()
        .iter()
        .map(|e| e["name"].clone())
        .collect();
    assert_eq!(names, [json!("sub"), json!("note.txt")]);

    let file = listing["entries"][1]["path"].as_str().unwrap();
    let opened = a.call("file.open_path", json!({"path": file, "preview": true})).await;
    assert_eq!(opened["kind"], "file");
    assert_eq!(opened["size"], 5);
    assert_eq!(opened["text_preview"], "hello");
}

#[tokio::test]
async fn file_access_is_confined_to_roots_after_canonicalize() {
    let base = std::env::temp_dir().join(format!("synapta-roots-{}", random_id(4)));
    std::fs::create_dir_all(base.join("root")).unwrap();
    std::fs::write(base.join("secret.txt"), "secret").unwrap();
    let a = start_with(vec![base.join("root")]).await;
    let otp = a.app.auth.open_pairing().otp;
    a.pair(&otp, a.app.host.public_key).await.unwrap();

    let sha = "0".repeat(64);
    // เขียนออกนอก root ด้วย `..`
    let escape = format!("{}/root/../escaped.txt", base.display());
    let r = a
        .rpc_raw("transfer.begin", json!({"path": escape, "size": 1, "sha256": sha}))
        .await;
    assert!(r.get("error").is_some(), "`..` ต้องถูกปฏิเสธ: {r}");
    assert!(!base.join("escaped.txt.synapta-part").exists());
    // อ่านนอก root (ทั้ง RPC และ GET /files)
    let outside = base.join("secret.txt").to_string_lossy().to_string();
    assert!(
        a.rpc_raw("transfer.stat", json!({"path": outside}))
            .await
            .get("error")
            .is_some()
    );
    let res = a.authed().get(a.files_url(&outside)).send().await.unwrap();
    assert_eq!(res.status(), 400);
    // ใน root ใช้ได้
    let inside = base.join("root").join("ok.bin").to_string_lossy().to_string();
    assert!(
        a.rpc_raw("transfer.begin", json!({"path": inside, "size": 1, "sha256": sha}))
            .await
            .get("error")
            .is_none()
    );
    let _ = std::fs::remove_dir_all(&base);
}

#[tokio::test]
async fn pty_over_wss_and_closes_on_revoke() {
    let a = start_paired().await;

    // pty ผ่าน HTTP ต้องบอกชัดว่าใช้ไม่ได้
    assert!(
        a.rpc_raw("pty.open", json!({})).await["error"]["message"]
            .as_str()
            .unwrap()
            .contains("WebSocket")
    );

    let mut ws = a.ws().await;
    let send = |id: u32, method: &str, params: Value| {
        Message::Text(
            json!({"jsonrpc":"2.0","id":id,"method":method,"params":params})
                .to_string()
                .into(),
        )
    };
    ws.send(send(1, "pty.open", json!({}))).await.unwrap();

    let (mut pty_id, mut output, mut exit_seen) = (None::<String>, String::new(), false);
    let deadline = tokio::time::Instant::now() + Duration::from_secs(30);
    while !exit_seen {
        let msg = tokio::time::timeout_at(deadline, ws.next())
            .await
            .expect("หมดเวลารอ pty")
            .unwrap()
            .unwrap();
        let Message::Text(text) = msg else { continue };
        let v: Value = serde_json::from_str(&text).unwrap();
        match v["method"].as_str() {
            Some("pty.output") => {
                let chunk = String::from_utf8_lossy(&B64.decode(v["params"]["data_b64"].as_str().unwrap()).unwrap())
                    .into_owned();
                // ConPTY ถามตำแหน่งเคอร์เซอร์ (ESC[6n) แล้วรอคำตอบก่อนเริ่ม shell — xterm.js ตอบให้เอง แต่ test ต้องตอบเอง
                if chunk.contains("\x1b[6n") {
                    // pty_id รู้แล้วเสมอ เพราะ output มาหลังคำตอบของ pty.open
                    ws.send(send(3, "pty.write", json!({"pty_id": pty_id, "data": "\x1b[1;1R"})))
                        .await
                        .unwrap();
                }
                output.push_str(&chunk);
            }
            Some("pty.exit") => exit_seen = true,
            _ if v["id"] == 1 => {
                pty_id = Some(
                    v["result"]["pty_id"]
                        .as_str()
                        .unwrap_or_else(|| panic!("pty.open: {v}"))
                        .to_string(),
                );
                ws.send(send(
                    2,
                    "pty.write",
                    json!({"pty_id": pty_id, "data": "echo synapta-pty-ok\rexit\r"}),
                ))
                .await
                .unwrap();
            }
            _ => {}
        }
    }
    assert!(output.contains("synapta-pty-ok"), "output ที่ได้: {output}");

    // เปิด pty ค้างไว้ แล้ว revoke → connection ต้องถูกตัด
    let mut ws2 = a.ws().await;
    ws2.send(send(1, "pty.open", json!({}))).await.unwrap();
    a.call("session.revoke", json!({"session_id": a.device_id()})).await;
    let closed = tokio::time::timeout(Duration::from_secs(10), async {
        while ws2.next().await.is_some_and(|m| m.is_ok()) {}
    })
    .await;
    assert!(closed.is_ok(), "connection ต้องถูกตัดหลัง revoke");
}

#[tokio::test]
async fn local_panel_guards_and_controls() {
    let a = start_paired().await;
    let url = format!("{}/api/v1/local/status", a.local_base);
    let plain = reqwest::Client::new();
    assert_eq!(
        plain.get(&url).send().await.unwrap().status(),
        403,
        "ไม่มี X-Synapta-Local"
    );
    // process อื่นบนเครื่องที่เดา token ไม่ได้ (เช่น ใส่ "1") ต้องอ่าน OTP ไม่ได้
    let guess = plain.get(&url).header("x-synapta-local", "1").send().await.unwrap();
    assert_eq!(guess.status(), 403, "token ผิดต้องถูกปฏิเสธ");
    let on_disk = std::fs::read_to_string(a.dir.join("local_token")).unwrap();
    assert_eq!(on_disk, a.app.local_token, "token ต้องอ่านได้จากไฟล์ใน data_dir");

    let status = a.local(reqwest::Method::GET, "/api/v1/local/status").await;
    assert_eq!(status.status(), 200);
    let status: Value = status.json().await.unwrap();
    assert!(status["otp"].is_null(), "จับคู่เสร็จแล้วโหมดจับคู่ต้องปิด");
    assert_eq!(status["devices"].as_array().unwrap().len(), 1);

    let rebinding = plain
        .get(&url)
        .header("x-synapta-local", &a.app.local_token)
        .header("host", "evil.example")
        .send()
        .await
        .unwrap();
    assert_eq!(rebinding.status(), 403, "Host ที่ไม่ใช่ loopback (DNS rebinding)");
    let proxied = plain
        .get(&url)
        .header("x-synapta-local", &a.app.local_token)
        .header("x-forwarded-for", "10.0.0.5")
        .send()
        .await
        .unwrap();
    assert_eq!(proxied.status(), 403, "ผ่าน proxy");
    let res = plain
        .get(&url)
        .header("x-synapta-local", &a.app.local_token)
        .header("origin", "http://evil.example")
        .send()
        .await
        .unwrap();
    assert!(
        res.headers().get("access-control-allow-origin").is_none(),
        "ต้องไม่มี CORS"
    );

    // เปิด/ปิดโหมดจับคู่
    let open: Value = a
        .local(reqwest::Method::POST, "/api/v1/local/pairing/open")
        .await
        .json()
        .await
        .unwrap();
    assert_eq!(open["otp"].as_str().unwrap().len(), 8);
    assert!(a.app.auth.current_otp().is_some());
    a.local(reqwest::Method::POST, "/api/v1/local/pairing/close").await;
    assert!(a.app.auth.current_otp().is_none());

    // "หยุดทันที": ตัด connection แต่อุปกรณ์ยังจับคู่อยู่ ต่อใหม่ได้
    let mut ws = a.ws().await;
    a.local(reqwest::Method::POST, "/api/v1/local/kill-all").await;
    let closed = tokio::time::timeout(Duration::from_secs(10), async {
        while ws.next().await.is_some_and(|m| m.is_ok()) {}
    })
    .await;
    assert!(closed.is_ok(), "kill-all ต้องตัด connection");
    assert_eq!(a.call("rpc.ping", json!({})).await["pong"], true);

    // revoke จากเครื่องนี้
    let id = a.device_id();
    let revoked: Value = a
        .local(reqwest::Method::POST, &format!("/api/v1/local/devices/{id}/revoke"))
        .await
        .json()
        .await
        .unwrap();
    assert_eq!(revoked["revoked"], true);
    assert!(a.authed().get(a.url("/api/v1/hello")).send().await.is_err());
}

#[tokio::test]
#[ignore = "ต้องมีหน้าจอ/เมาส์จริง (ขยับเมาส์เครื่องนี้) — รัน: cargo test --test api -- --ignored --nocapture"]
async fn screen_snapshot_stream_and_input_coordinates_agree() {
    use enigo::{Enigo, Mouse, Settings};
    let a = start_paired().await;
    let list = a.call("screen.list", json!({})).await;
    let primary = list["monitors"]
        .as_array()
        .unwrap()
        .iter()
        .find(|m| m["is_primary"] == true)
        .unwrap()
        .clone();
    let (pw, ph) = (primary["width"].as_u64().unwrap(), primary["height"].as_u64().unwrap());
    println!("จอหลัก {pw}x{ph} scale_factor={}", primary["scale_factor"]);

    for (format, magic) in [("png", &b"\x89PNG"[..]), ("webp", b"RIFF"), ("jpeg", b"\xff\xd8")] {
        let s = a.call("screen.snapshot", json!({"format": format})).await;
        let bytes = B64.decode(s["data_b64"].as_str().unwrap()).unwrap();
        assert!(bytes.starts_with(magic), "{format} ไม่ใช่ไฟล์ภาพที่ถูกต้อง");
        assert_eq!((s["width"].as_u64().unwrap(), s["height"].as_u64().unwrap()), (pw, ph));
        println!("{format}: {} KB", bytes.len() / 1024);
    }
    let small = a
        .call("screen.snapshot", json!({"format": "jpeg", "max_width": 640}))
        .await;
    assert_eq!(small["width"], 640);
    assert!((small["scale"].as_f64().unwrap() - pw as f64 / 640.0).abs() < 0.01);

    // เมาส์ต้องไปอยู่ที่พิกเซลเดียวกับที่ภาพจับได้ (กับดัก DPI scaling)
    let enigo = Enigo::new(&Settings::default()).unwrap();
    assert_eq!(
        enigo.main_display().unwrap(),
        (pw as i32, ph as i32),
        "เมาส์กับภาพต้องใช้ระบบพิกเซลเดียวกัน"
    );
    let original = enigo.location().unwrap();
    a.call("input.mouse_move", json!({"x": 200, "y": 150})).await;
    assert_eq!(enigo.location().unwrap(), (200, 150));
    a.call("input.mouse_move", json!({"x": original.0, "y": original.1}))
        .await;

    // stream: ได้เฟรม MJPEG แรกจริง
    let mut res = a
        .authed()
        .get(a.url("/api/v1/screen/stream?fps=5&max_width=640"))
        .send()
        .await
        .unwrap();
    assert_eq!(res.status(), 200);
    let first = res.chunk().await.unwrap().unwrap();
    assert!(first.starts_with(b"--frame"));
    println!("เฟรมแรกของ stream {} KB", first.len() / 1024);
}

#[tokio::test]
#[ignore = "ต้องมี multicast บนเครื่อง/เครือข่ายจริง — รัน: cargo test --test api -- --ignored --nocapture"]
async fn mdns_and_udp_beacon_announce_only_basic_identity() {
    let dir = std::env::temp_dir().join(format!("synapta-mdns-{}", random_id(4)));
    let config = Config {
        mode: synapta_connect::config::Mode::Console,
        port: 52499,
        discovery_port: 52498,
        local_port: 0,
        data_dir: dir.clone(),
        name: "mdns-test".into(),
        file_roots: vec![],
    };
    let app = AppState::new(config).unwrap();
    let _discovery = synapta_connect::discovery::start(&app).await;

    // UDP probe ตอบด้วย beacon ที่มีแค่ตัวตนพื้นฐาน
    let sock = tokio::net::UdpSocket::bind("127.0.0.1:0").await.unwrap();
    sock.send_to(br#"{"probe":"synapta-connect"}"#, "127.0.0.1:52498")
        .await
        .unwrap();
    let mut buf = [0u8; 1024];
    let (n, _) = tokio::time::timeout(Duration::from_secs(5), sock.recv_from(&mut buf))
        .await
        .unwrap()
        .unwrap();
    let beacon: Value = serde_json::from_slice(&buf[..n]).unwrap();
    let mut keys: Vec<_> = beacon.as_object().unwrap().keys().cloned().collect();
    keys.sort();
    assert_eq!(
        keys,
        [
            "device_id",
            "host_pubkey_fingerprint",
            "pairing_mode",
            "port",
            "protocol"
        ]
    );
    assert_eq!(beacon["host_pubkey_fingerprint"], app.host.fingerprint());

    let found = tokio::task::spawn_blocking(|| {
        let daemon = mdns_sd::ServiceDaemon::new().unwrap();
        let events = daemon.browse("_synapta-connect._tcp.local.").unwrap();
        let deadline = std::time::Instant::now() + Duration::from_secs(15);
        while let Ok(event) = events.recv_timeout(deadline.saturating_duration_since(std::time::Instant::now())) {
            if let mdns_sd::ServiceEvent::ServiceResolved(info) = event
                && info.get_port() == 52499
            {
                return Some(info.get_addresses().len());
            }
        }
        None
    })
    .await
    .unwrap();
    println!("พบผ่าน mDNS: addresses={found:?}");
    assert!(found.is_some(), "ไม่พบ service ผ่าน mDNS ใน 15 วินาที");
    let _ = std::fs::remove_dir_all(dir);
}

/// ขั้น 1 ของ SPAKE2 อย่างเดียว (ไม่ทำต่อ) — คืน HTTP status ไว้ทดสอบเพดาน handshake ค้าง
async fn step1_only(a: &Agent, otp: &str) -> u16 {
    let (_, msg) = Spake2::<Ed25519Group>::start_a(
        &Password::new(otp.as_bytes()),
        &Identity::new(CLIENT_ID),
        &Identity::new(HOST_ID),
    );
    let body = json!({"step": 1, "client_msg": B64.encode(msg), "client_pubkey": B64URL.encode(a.me.public_key()), "client_name": "spam"});
    a.http(None)
        .post(a.url("/api/v1/auth/spake2-handshake"))
        .json(&body)
        .send()
        .await
        .unwrap()
        .status()
        .as_u16()
}

#[tokio::test]
async fn handshake_spam_is_rejected_not_evicting_and_wrong_guesses_are_reported() {
    let a = start().await;
    let otp = a.app.auth.open_pairing().otp;
    // เดาผิด → หน้าจัดการเห็นจำนวนและ IP ต้นทาง
    let _ = a.pair("00000000", a.app.host.public_key).await;
    let status: Value = a
        .local(reqwest::Method::GET, "/api/v1/local/status")
        .await
        .json()
        .await
        .unwrap();
    assert_eq!(status["pairing"]["wrong_guesses"], 1);
    assert_eq!(status["pairing"]["recent_failures"][0]["ip"], "127.0.0.1");

    // IP เดียวค้าง handshake ได้ 2 อัน ตัวที่ 3 ถูกปฏิเสธ (ไม่ใช่ทิ้งตัวเก่า)
    assert_eq!(step1_only(&a, &otp).await, 200);
    assert_eq!(step1_only(&a, &otp).await, 200);
    assert_eq!(step1_only(&a, &otp).await, 429);
}

#[tokio::test]
async fn a_device_can_only_revoke_itself() {
    let a = start_paired().await;
    // ลงทะเบียนอุปกรณ์ที่สองตรง ๆ ผ่าน auth (ไม่ต้องทำ TLS อีกชุด)
    let (otp_gen, other_key) = (a.app.auth.open_pairing(), B64URL.encode([9u8; 32]));
    let _ = otp_gen;
    let (_, generation) = a.app.auth.begin_pairing().unwrap();
    let other = a.app.auth.complete_pairing(generation, &other_key, "other").unwrap();

    let r = a.rpc_raw("session.revoke", json!({"session_id": other.id})).await;
    assert!(
        r["error"]["message"].as_str().unwrap().contains("ตัวเอง"),
        "ต้องปฏิเสธ: {r}"
    );
    assert!(a.app.auth.is_active(&other.id), "อุปกรณ์อื่นต้องยังอยู่");

    // ไม่ใส่ session_id = เพิกถอนตัวเอง
    assert_eq!(a.call("session.revoke", json!({})).await["revoked"], true);
    assert!(!a.app.auth.is_active(&a.device_id()));
}

#[tokio::test]
async fn idle_pre_auth_connections_cannot_lock_out_paired_devices() {
    use tokio::io::AsyncReadExt;
    let a = start_paired().await;
    // ผู้โจมตีเปิด TCP เปล่าค้างไว้จากหนึ่ง IP (127.0.0.1) จนเต็มเพดานต่อ IP (8)
    let mut held = Vec::new();
    for _ in 0..8 {
        held.push(tokio::net::TcpStream::connect(("127.0.0.1", a.port)).await.unwrap());
    }
    // ตัวที่ 9 ต้องถูกตัดทิ้งทันที (อ่านได้ EOF/error ไม่ใช่ค้าง)
    let mut ninth = tokio::net::TcpStream::connect(("127.0.0.1", a.port)).await.unwrap();
    let mut buf = [0u8; 1];
    let closed = tokio::time::timeout(Duration::from_secs(3), ninth.read(&mut buf)).await;
    assert!(
        matches!(closed, Ok(Ok(0)) | Ok(Err(_))),
        "connection ที่เกินเพดานต่อ IP ต้องถูกปิด"
    );

    // เครื่องที่จับคู่แล้วจาก IP อื่น (127.0.0.2) ยังต่อได้ตามปกติ
    let owner = reqwest::Client::builder()
        .use_preconfigured_tls(client_config(a.app.host.public_key, Some(&a.me)))
        .local_address("127.0.0.2".parse::<std::net::IpAddr>().unwrap())
        .build()
        .unwrap();
    let hello = owner.get(a.url("/api/v1/hello")).send().await.unwrap();
    assert_eq!(hello.status(), 200);
    drop(held);
}
