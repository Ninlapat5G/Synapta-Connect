//! ทดสอบฝั่ง service ผ่าน HTTP จริงบน loopback: การยืนยันตัวของ panel, API อัปเดต, และการส่งงานจับภาพ/input ให้ helper
//! (ส่วนที่ต้องสิทธิ์ admin — ติดตั้ง/ถอน service จริง — อยู่ที่ tests/admin/service-lifecycle.ps1)

use futures_util::{SinkExt, StreamExt};
use ring::{
    rand::SystemRandom,
    signature::{Ed25519KeyPair, KeyPair},
};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    net::SocketAddr,
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicU32, Ordering},
    },
    time::Duration,
};
use synapta_connect::{
    app::{App, AppState},
    config::{Config, Mode},
    hex::{random_id, to_hex},
    local, proof,
    update::{Trigger, UpdateEnv, version::Version, web_package},
};
use tokio_tungstenite::tungstenite::{Message, client::IntoClientRequest};

struct Calls(AtomicU32);

impl Trigger for Calls {
    fn start_installer(&self) -> Result<(), String> {
        self.0.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }
}

struct Svc {
    app: App,
    base: String,
    dir: PathBuf,
    key: Ed25519KeyPair,
    calls: Arc<Calls>,
    http: reqwest::Client,
}

impl Drop for Svc {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

async fn start(mode: Mode) -> Svc {
    let _ = rustls::crypto::ring::default_provider().install_default();
    let dir = std::env::temp_dir().join(format!("synapta-svc-{}", random_id(4)));
    let key =
        Ed25519KeyPair::from_pkcs8(Ed25519KeyPair::generate_pkcs8(&SystemRandom::new()).unwrap().as_ref()).unwrap();
    let calls = Arc::new(Calls(AtomicU32::new(0)));
    let env = UpdateEnv {
        pubkey: Some(key.public_key().as_ref().try_into().unwrap()),
        trigger: calls.clone(),
        current: Version(0, 3, 0),
    };
    let config = Config {
        mode,
        port: 0,
        discovery_port: 0,
        local_port: 0,
        data_dir: dir.join("data"),
        name: "svc-test".into(),
        file_roots: vec![],
    };
    let app = AppState::new_with_update_env(config, env).unwrap();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let router = local::router(app.clone()).into_make_service_with_connect_info::<SocketAddr>();
    tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
    Svc {
        app,
        base,
        dir,
        key,
        calls,
        http: reqwest::Client::new(),
    }
}

impl Svc {
    fn req(&self, method: reqwest::Method, path: &str, token: Option<&str>) -> reqwest::RequestBuilder {
        let r = self.http.request(method, format!("{}{path}", self.base));
        match token {
            Some(t) => r.header("X-Synapta-Local", t),
            None => r,
        }
    }
    fn file_token(&self) -> String {
        std::fs::read_to_string(self.dir.join("data").join("local_token")).unwrap()
    }
    async fn panel_token(&self) -> String {
        let code: Value = self
            .req(
                reqwest::Method::POST,
                "/api/v1/local/panel-session",
                Some(&self.file_token()),
            )
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        let exchanged: Value = self
            .req(reqwest::Method::POST, "/api/v1/panel/exchange", None)
            .json(&json!({"code": code["code"]}))
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        exchanged["token"].as_str().unwrap().to_string()
    }
    /// วาง release ที่เซ็นแล้วในโฟลเดอร์ แล้วชี้แหล่งอัปเดตไปที่นั่น
    fn publish(&self, version: &str, payload: &[u8]) {
        let release = self.dir.join("release");
        std::fs::create_dir_all(&release).unwrap();
        let asset = format!("SynaptaConnect-v{version}.exe");
        let manifest = json!({"version": version, "asset": asset, "sha256": to_hex(&Sha256::digest(payload)),
                              "size": payload.len(), "platform": "windows-x86_64", "notes": "ทดสอบ"})
        .to_string();
        std::fs::write(release.join("manifest.json"), &manifest).unwrap();
        std::fs::write(
            release.join("manifest.json.sig"),
            to_hex(self.key.sign(manifest.as_bytes()).as_ref()),
        )
        .unwrap();
        std::fs::write(release.join(asset), payload).unwrap();
        // แหล่งแบบ dir: ตั้งผ่าน panel ไม่ได้ (ตั้งใจ) — เทสต์เขียนค่าตั้งตรง ๆ เหมือนผู้ดูแล
        self.app
            .settings
            .update(|s| s.update.source = format!("dir:{}", release.display()))
            .unwrap();
    }
    async fn wait_state(&self, token: &str, want: &str) -> Value {
        self.wait_state_at("/api/v1/local/update", token, want).await
    }

    async fn wait_state_at(&self, path: &str, token: &str, want: &str) -> Value {
        for _ in 0..100 {
            let s: Value = self
                .req(reqwest::Method::GET, path, Some(token))
                .send()
                .await
                .unwrap()
                .json()
                .await
                .unwrap();
            if s["state"] == want {
                return s;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        panic!("state ไม่เป็น {want}");
    }
}

#[tokio::test]
async fn signed_vite_build_updates_and_rolls_back_over_panel_http() {
    let s = start(Mode::Service).await;
    let token = s.panel_token().await;
    let source = s.dir.join("web-release");
    std::fs::create_dir(&source).unwrap();
    let version = "99.0.0";
    let bytes = web_package::collect(&PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../frontend/dist")).unwrap();
    let asset = web_package::Manifest::asset_name(version);
    let manifest = serde_json::to_vec(&json!({
        "version": version, "panel_api": web_package::API, "asset": asset,
        "sha256": to_hex(&Sha256::digest(&bytes)), "size": bytes.len()
    }))
    .unwrap();
    std::fs::write(source.join(web_package::MANIFEST), &manifest).unwrap();
    std::fs::write(
        source.join(web_package::SIGNATURE),
        to_hex(s.key.sign(&manifest).as_ref()),
    )
    .unwrap();
    std::fs::write(source.join(&asset), bytes).unwrap();
    s.app
        .settings
        .update(|settings| settings.update.web_source = format!("dir:{}", source.display()))
        .unwrap();

    let endpoint = "/api/v1/local/web-update";
    let post = |action| s.req(reqwest::Method::POST, &format!("{endpoint}/{action}"), Some(&token));
    assert_eq!(post("check").send().await.unwrap().status(), 202);
    let available = s.wait_state_at(endpoint, &token, "update_available").await;
    assert_eq!(available["available"]["version"], version);
    assert_eq!(
        post("apply")
            .json(&json!({ "version": version }))
            .send()
            .await
            .unwrap()
            .status(),
        202
    );
    assert_eq!(
        s.wait_state_at(endpoint, &token, "up_to_date").await["current_version"],
        version
    );
    assert_eq!(s.calls.0.load(Ordering::SeqCst), 0, "the binary installer stays idle");
    let index = s
        .req(reqwest::Method::GET, "/synaptaconnect", None)
        .send()
        .await
        .unwrap()
        .text()
        .await
        .unwrap();
    assert!(index.contains("/synaptaconnect/assets/"));
    assert_eq!(post("rollback").send().await.unwrap().status(), 202);
    assert_eq!(
        s.wait_state_at(endpoint, &token, "rolled_back").await["current_version"],
        web_package::BASELINE
    );
}

#[tokio::test]
async fn panel_authentication_flow_and_guards() {
    let s = start(Mode::Service).await;
    let file = s.file_token();

    // ไม่มี token = ปฏิเสธทุก API; health เรียกได้ไม่ต้องมี token
    assert_eq!(
        s.req(reqwest::Method::GET, "/api/v1/local/status", None)
            .send()
            .await
            .unwrap()
            .status(),
        403
    );
    let health: Value = s
        .req(reqwest::Method::GET, "/api/v1/local/health", None)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(
        (health["ok"].as_bool(), health["version"].as_str()),
        (Some(true), Some(env!("CARGO_PKG_VERSION")))
    );

    // รหัสครั้งเดียว → token ของ panel; ใช้ซ้ำไม่ได้
    let code: Value = s
        .req(reqwest::Method::POST, "/api/v1/local/panel-session", Some(&file))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let exchange = |c: Value| {
        s.req(reqwest::Method::POST, "/api/v1/panel/exchange", None)
            .json(&json!({"code": c}))
    };
    let token = exchange(code["code"].clone())
        .send()
        .await
        .unwrap()
        .json::<Value>()
        .await
        .unwrap()["token"]
        .as_str()
        .unwrap()
        .to_string();
    assert_eq!(exchange(code["code"].clone()).send().await.unwrap().status(), 403);
    assert_eq!(exchange(json!("bogus")).send().await.unwrap().status(), 403);

    // token ของ panel ใช้กับ API ของเจ้าของได้ และสถานะมีฟิลด์ใหม่ตามสัญญา
    let status: Value = s
        .req(reqwest::Method::GET, "/api/v1/local/status", Some(&token))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(status["agent"]["mode"], "service");
    assert_eq!(status["agent"]["version"], env!("CARGO_PKG_VERSION"));
    assert_eq!(status["screen_input"]["enabled"], false);

    // DNS rebinding (Host แปลก) และเว็บอื่น (Origin ต่าง origin) ถูกตัด; Origin ของ panel เองผ่าน
    let evil_host = s
        .req(reqwest::Method::GET, "/api/v1/local/health", None)
        .header("Host", "evil.example");
    assert_eq!(evil_host.send().await.unwrap().status(), 403);
    let evil_origin = s
        .req(reqwest::Method::GET, "/api/v1/local/status", Some(&token))
        .header("Origin", "http://evil.example");
    assert_eq!(evil_origin.send().await.unwrap().status(), 403);
    let own = s
        .req(reqwest::Method::GET, "/api/v1/local/status", Some(&token))
        .header("Origin", s.base.clone());
    assert_eq!(own.send().await.unwrap().status(), 200);

    // หน้าเว็บ panel เสิร์ฟได้พร้อมหัวความปลอดภัย; ไฟล์ที่ไม่ได้ฝังไม่มี
    let page = s.req(reqwest::Method::GET, "/panel/", None).send().await.unwrap();
    assert_eq!(page.status(), 200);
    assert!(
        page.headers()["content-security-policy"]
            .to_str()
            .unwrap()
            .contains("default-src 'none'")
    );
    assert!(
        page.headers()["content-type"]
            .to_str()
            .unwrap()
            .starts_with("text/html")
    );
    assert_eq!(
        s.req(reqwest::Method::GET, "/panel/README.md", None)
            .send()
            .await
            .unwrap()
            .status(),
        404
    );
    // ออกนอกโฟลเดอร์ panel ไม่ได้ (URL ถูก normalize เป็น /Cargo.toml ซึ่งไม่มี route → ปฏิเสธ)
    let escape = s
        .req(reqwest::Method::GET, "/panel/../Cargo.toml", None)
        .send()
        .await
        .unwrap();
    assert!(matches!(escape.status().as_u16(), 403 | 404));
}

#[tokio::test]
async fn update_api_check_apply_retry_and_settings() {
    let s = start(Mode::Service).await;
    let token = s.panel_token().await;
    let t = Some(token.as_str());
    s.publish("0.3.1", b"new-version-exe");

    // ตรวจ: ตอบ 202 ทันที แล้ว poll เห็นรุ่นใหม่; เรียกซ้ำเร็ว ๆ ถูกจำกัดอัตรา
    let check = s
        .req(reqwest::Method::POST, "/api/v1/local/update/check", t)
        .send()
        .await
        .unwrap();
    assert_eq!(check.status(), 202);
    let status = s.wait_state(&token, "update_available").await;
    assert_eq!(
        (
            status["current_version"].as_str(),
            status["available"]["version"].as_str()
        ),
        (Some("0.3.0"), Some("0.3.1"))
    );
    assert_eq!(
        s.req(reqwest::Method::POST, "/api/v1/local/update/check", t)
            .send()
            .await
            .unwrap()
            .status(),
        429
    );

    // ติดตั้งรุ่นที่ไม่มี = 400; รุ่นที่มี = 202 แล้วตัวติดตั้งถูกเรียกหนึ่งครั้ง
    let apply = |v: &str| {
        s.req(reqwest::Method::POST, "/api/v1/local/update/apply", t)
            .json(&json!({"version": v}))
    };
    assert_eq!(apply("9.9.9").send().await.unwrap().status(), 400);
    assert_eq!(apply("0.3.1").send().await.unwrap().status(), 202);
    let status = s.wait_state(&token, "applying").await;
    assert_eq!(s.calls.0.load(Ordering::SeqCst), 1);
    assert!(
        status["history"]
            .as_array()
            .unwrap()
            .iter()
            .any(|h| h["outcome"] == "staged")
    );
    assert_eq!(apply("0.3.1").send().await.unwrap().status(), 409, "กำลังติดตั้งอยู่ ห้ามซ้อน");

    // ตัวติดตั้งรายงานว่ากู้รุ่นเดิม → panel เห็นสาเหตุ + ลองใหม่ได้
    let result = json!({"id": "r1", "version": "0.3.1", "from": "0.3.0", "outcome": "rolled_back", "at": 1,
        "error": {"code": "health_check_failed", "stage": "health", "message": "service ไม่ตอบ"}});
    let status_dir = s.dir.join("data").join("updater-status");
    std::fs::create_dir_all(&status_dir).unwrap();
    std::fs::write(status_dir.join("result.json"), result.to_string()).unwrap();
    s.app.updates.ingest_result();
    let status = s.wait_state(&token, "rolled_back").await;
    assert_eq!(
        (status["last_error"]["code"].as_str(), status["can_retry"].as_bool()),
        (Some("health_check_failed"), Some(true))
    );
    assert_eq!(
        s.req(reqwest::Method::POST, "/api/v1/local/update/retry", t)
            .send()
            .await
            .unwrap()
            .status(),
        202
    );
    s.wait_state(&token, "applying").await;
    assert_eq!(s.calls.0.load(Ordering::SeqCst), 2);

    let history: Value = s
        .req(reqwest::Method::GET, "/api/v1/local/update/history?limit=3", t)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(history.as_array().unwrap().len(), 3);

    // ไม่มีงานล้มเหลวให้ลองใหม่ = 400
    let fresh = start(Mode::Service).await;
    let ft = fresh.panel_token().await;
    assert_eq!(
        fresh
            .req(reqwest::Method::POST, "/api/v1/local/update/retry", Some(&ft))
            .send()
            .await
            .unwrap()
            .status(),
        400
    );

    // ค่าตั้ง: ปิดอัตโนมัติได้; แหล่งต้องเป็น github:/https:// เท่านั้น (dir: และ http:// ถูกปฏิเสธ)
    let put = |body: Value| {
        s.req(reqwest::Method::PUT, "/api/v1/local/update/settings", t)
            .json(&body)
    };
    for bad in ["dir:C:\\x", "http://x/", "ftp://x", "github:bad"] {
        let r = put(json!({"auto_update": true, "source": bad})).send().await.unwrap();
        assert_eq!(r.status(), 400, "{bad}");
        assert_eq!(r.json::<Value>().await.unwrap()["error"]["code"], "bad_source");
    }
    let ok: Value = put(json!({"auto_update": false, "source": "github:me/Synapta-Connect"}))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(
        (ok["auto_update"].as_bool(), ok["source"].as_str()),
        (Some(false), Some("github:me/Synapta-Connect"))
    );
    assert!(!s.app.settings.get().update.auto_update, "บันทึกลงค่าตั้งจริง");
}

/// helper ปลอมที่ต่อเข้ามาเพื่อทดสอบการส่งผลสำเร็จและข้อผิดพลาดกลับจาก helper
type HelperSocket = tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;

/// ขั้นจับมือของ helper: ส่ง nonce + proof (ไม่ส่ง token) → ได้ socket + หัวข้อ proof ของฝั่ง agent
async fn helper_handshake(
    s: &Svc,
    nonce: &str,
    proof_hex: &str,
) -> Result<
    (
        HelperSocket,
        tokio_tungstenite::tungstenite::handshake::client::Response,
    ),
    String,
> {
    let mut request = format!("ws{}/api/v1/local/helper", &s.base[4..])
        .into_client_request()
        .unwrap();
    request.headers_mut().insert("x-synapta-nonce", nonce.parse().unwrap());
    request
        .headers_mut()
        .insert("x-synapta-proof", proof_hex.parse().unwrap());
    tokio_tungstenite::connect_async(request)
        .await
        .map_err(|e| e.to_string())
}

/// `secret` = ความลับที่ helper อ้างว่ารู้ (ของจริงคือ local_token จากไฟล์)
async fn connect_helper(s: &Svc, secret: &str, user: &str) -> Result<tokio::task::JoinHandle<()>, String> {
    let nonce = random_id(16);
    let (socket, response) = helper_handshake(s, &nonce, &proof::make(secret, proof::HELPER, &nonce)).await?;
    let server_proof = response.headers()["x-synapta-server-proof"].to_str().unwrap();
    assert!(
        proof::check(secret, proof::SERVER, &nonce, server_proof),
        "agent ต้องพิสูจน์ตัวกลับ"
    );
    let (mut sink, mut stream) = socket.split();
    sink.send(Message::Text(
        json!({"hello": {"user": user, "session": 7}}).to_string().into(),
    ))
    .await
    .unwrap();
    Ok(tokio::spawn(async move {
        while let Some(Ok(msg)) = stream.next().await {
            if let Message::Text(text) = msg {
                let v: Value = serde_json::from_str(&text).unwrap();
                let reply = if v["method"] == "input.key" {
                    json!({"id": v["id"], "error": {
                        "code": -32004,
                        "message": "desktop ของ helper ไม่ได้รับอินพุตอยู่",
                    }})
                } else {
                    json!({"id": v["id"], "result": {"handled": v["method"]}})
                };
                if sink.send(Message::Text(reply.to_string().into())).await.is_err() {
                    break;
                }
            }
        }
    }))
}

async fn wait_helper(s: &Svc, connected: bool) {
    for _ in 0..100 {
        if s.app.display.helper_info().is_some() == connected {
            return;
        }
        tokio::time::sleep(Duration::from_millis(30)).await;
    }
    panic!("helper_connected ไม่เป็น {connected}");
}

#[tokio::test]
async fn screen_and_input_go_only_through_an_allowed_helper() {
    let s = start(Mode::Service).await;
    let call = |m: &'static str| {
        let app = s.app.clone();
        async move { app.display.call(&app, m, json!({})).await }
    };

    // 1) เจ้าของยังไม่อนุญาต → ปฏิเสธ แม้มี helper ต่ออยู่
    let helper = connect_helper(&s, &s.file_token(), "owner").await.unwrap();
    wait_helper(&s, true).await;
    let e = call("screen.snapshot").await.unwrap_err();
    assert_eq!(e.code, -32004);
    assert!(e.message.contains("ยังไม่ได้อนุญาต"), "{}", e.message);

    // 2) อนุญาตแล้ว → งานถึง helper และผลกลับมา
    let token = s.panel_token().await;
    let on = s
        .req(reqwest::Method::PUT, "/api/v1/local/screen-input", Some(&token))
        .json(&json!({"enabled": true}));
    let state: Value = on.send().await.unwrap().json().await.unwrap();
    assert_eq!(
        (
            state["enabled"].as_bool(),
            state["helper_connected"].as_bool(),
            state["helper_user"].as_str()
        ),
        (Some(true), Some(true), Some("owner"))
    );
    assert_eq!(call("input.click").await.unwrap(), json!({"handled": "input.click"}));
    let error = call("input.key").await.unwrap_err();
    assert_eq!(error.code, -32004);
    assert_eq!(error.message, "desktop ของ helper ไม่ได้รับอินพุตอยู่");

    // 3) helper ตัวที่สองถูกปฏิเสธ ตัวแรกยังเป็นของจริง
    let second = connect_helper(&s, &s.file_token(), "intruder").await.unwrap();
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert_eq!(s.app.display.helper_info().unwrap().user, "owner");
    second.abort();

    // 4) พิสูจน์ตัวด้วยความลับที่ไม่ใช่ local_token ไม่ได้ — token ของ panel, ค่ามั่ว
    //    (และ nonce ที่เคยใช้แล้วเล่นซ้ำไม่ได้ — ดู replayed_helper_handshake_is_refused)
    // token ของ panel เป็น helper ไม่ได้ (ตัวตนของ helper ผูกกับสิทธิ์อ่านไฟล์ token)
    assert!(connect_helper(&s, &token, "panel-as-helper").await.is_err());
    assert!(connect_helper(&s, "wrong", "x").await.is_err());

    // 5) เจ้าของปิดสวิตช์ → ปฏิเสธทันที; helper หลุด → บอกว่าไม่มี helper
    let off = s
        .req(reqwest::Method::PUT, "/api/v1/local/screen-input", Some(&token))
        .json(&json!({"enabled": false}));
    off.send().await.unwrap();
    assert!(call("input.click").await.unwrap_err().message.contains("ยังไม่ได้อนุญาต"));
    s.app.settings.update(|st| st.screen_input.enabled = true).unwrap();
    helper.abort();
    wait_helper(&s, false).await;
    assert!(call("screen.list").await.unwrap_err().message.contains("ไม่มี helper"));
}

#[tokio::test]
async fn helper_only_accepts_display_methods() {
    use synapta_connect::display::is_display_method;
    for m in [
        "screen.list",
        "screen.snapshot",
        "input.click",
        "input.key",
        "input.type",
        "input.scroll",
        "input.mouse_move",
    ] {
        assert!(is_display_method(m), "{m}");
    }
    for m in [
        "exec.run",
        "transfer.begin",
        "pty.open",
        "session.revoke",
        "system.profile",
        "",
    ] {
        assert!(!is_display_method(m), "{m}");
    }
}

#[tokio::test]
async fn replayed_helper_handshake_is_refused() {
    let s = start(Mode::Service).await;
    let (nonce, secret) = (random_id(16), s.file_token());
    let proof_hex = proof::make(&secret, proof::HELPER, &nonce);
    let first = helper_handshake(&s, &nonce, &proof_hex).await;
    assert!(first.is_ok());
    drop(first);
    // ผู้ดักเห็น nonce+proof ที่ loopback เล่นซ้ำเพื่อเป็น helper แทนไม่ได้
    assert!(helper_handshake(&s, &nonce, &proof_hex).await.is_err());
    // ไม่มี/สั้นเกิน/ไม่ใช่ hex = ปฏิเสธ
    for bad in ["", "abc", "zzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzz"] {
        assert!(
            helper_handshake(&s, bad, &proof::make(&secret, proof::HELPER, bad))
                .await
                .is_err(),
            "{bad:?}"
        );
    }
    // proof ของป้ายอื่น (เช่นของ health) ใช้แทนของ helper ไม่ได้
    let n2 = random_id(16);
    assert!(
        helper_handshake(&s, &n2, &proof::make(&secret, proof::HEALTH, &n2))
            .await
            .is_err()
    );
}

#[tokio::test]
async fn health_proves_identity_only_to_someone_who_knows_the_token() {
    let s = start(Mode::Service).await;
    let secret = s.file_token();
    let nonce = random_id(16);
    let get = |query: String| {
        let req = s.req(reqwest::Method::GET, &format!("/api/v1/local/health{query}"), None);
        async move { req.send().await.unwrap().json::<Value>().await.unwrap() }
    };
    let h = get(format!("?nonce={nonce}")).await;
    let (version, pid) = (h["version"].as_str().unwrap(), h["pid"].as_u64().unwrap() as u32);
    let message = proof::health_message(&nonce, version, pid);
    assert!(proof::check(
        &secret,
        proof::HEALTH,
        &message,
        h["proof"].as_str().unwrap()
    ));
    assert!(!proof::check(
        "not-the-token",
        proof::HEALTH,
        &message,
        h["proof"].as_str().unwrap()
    ));
    // ตอบของ nonce เก่าให้ nonce ใหม่ไม่ได้
    assert!(!proof::check(
        &secret,
        proof::HEALTH,
        &proof::health_message("00ff", version, pid),
        h["proof"].as_str().unwrap()
    ));
    // ไม่มี nonce / nonce ไม่ใช่ hex = ไม่มี proof
    assert!(get(String::new()).await["proof"].is_null());
    assert!(get("?nonce=not-hex!".into()).await["proof"].is_null());
}

/// ผู้ใช้อื่นที่ยึดพอร์ต 52402 ตอน service หยุดหลอก helper ของเจ้าของไม่ได้: helper ต้องตัดโดยไม่ส่ง/ทำอะไรเลย
#[allow(clippy::result_large_err)] // callback ของ tungstenite กำหนดชนิด Err มาให้
#[tokio::test]
async fn helper_refuses_to_serve_an_impostor_listener() {
    use tokio_tungstenite::tungstenite::handshake::server::{Request, Response};
    for header in [None, Some("deadbeef".repeat(8))] {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let (received, mut inbox) = tokio::sync::mpsc::unbounded_channel::<String>();
        let header2 = header.clone();
        tokio::spawn(async move {
            let (tcp, _) = listener.accept().await.unwrap();
            let callback = move |_: &Request, mut res: Response| {
                if let Some(h) = &header2 {
                    res.headers_mut().insert("x-synapta-server-proof", h.parse().unwrap());
                }
                Ok(res)
            };
            let Ok(mut ws) = tokio_tungstenite::accept_hdr_async(tcp, callback).await else {
                return;
            };
            // ปลอมเป็น agent: ส่งคำสั่งจับภาพทันที แล้วดูว่า helper ตอบ/ส่งอะไรกลับมาหรือไม่
            let _ = ws
                .send(Message::Text(
                    json!({"id": 1, "method": "screen.list"}).to_string().into(),
                ))
                .await;
            while let Some(Ok(m)) = ws.next().await {
                if let Message::Text(t) = m {
                    let _ = received.send(t.to_string());
                }
            }
        });
        let result = synapta_connect::helper::session(port, "the-real-local-token").await;
        assert!(result.is_err(), "helper ต้องปฏิเสธผู้ฟังปลอม (หัวข้อ={header:?})");
        tokio::time::sleep(Duration::from_millis(300)).await;
        assert!(inbox.try_recv().is_err(), "helper ต้องไม่ส่ง hello/ผลใด ๆ ให้ผู้ฟังปลอม");
    }
}

// ---------- คำสั่งรันด้วยบัญชีเจ้าของ (run_as.rs) ----------

/// ctx ของ client WebSocket หนึ่งตัว พร้อมช่องรับ notification (pty.output / pty.exit)
fn ws_ctx() -> (synapta_connect::api::Ctx, tokio::sync::mpsc::Receiver<String>) {
    let (out, inbox) = tokio::sync::mpsc::channel(256);
    let ctx = synapta_connect::api::Ctx {
        session_id: "client".into(),
        ws: Some(synapta_connect::api::WsCtx {
            out,
            ptys: synapta_connect::pty::Registry::default(),
        }),
    };
    (ctx, inbox)
}

/// helper ตัวจริง (โค้ดเดียวกับที่รันใน session เจ้าของ) ต่อเข้า service ที่ทดสอบ
fn real_helper(s: &Svc) -> tokio::task::JoinHandle<()> {
    let port: u16 = s.base.rsplit(':').next().unwrap().parse().unwrap();
    let token = s.file_token();
    tokio::spawn(async move {
        let _ = synapta_connect::helper::session(port, token.trim()).await;
    })
}

/// เก็บ output ของ terminal จนเจอ `needle` หรือหมดเวลา
async fn read_pty_until(inbox: &mut tokio::sync::mpsc::Receiver<String>, id: &str, needle: &str) -> String {
    use base64::Engine;
    let mut seen = String::new();
    let deadline = tokio::time::Instant::now() + Duration::from_secs(20);
    while !seen.contains(needle) {
        let left = deadline.saturating_duration_since(tokio::time::Instant::now());
        let Ok(Some(text)) = tokio::time::timeout(left, inbox.recv()).await else {
            panic!("ไม่เห็น {needle:?} ใน output ของ terminal: {seen:?}");
        };
        let v: Value = serde_json::from_str(&text).unwrap();
        if v["method"] == "pty.output" && v["params"]["pty_id"] == id {
            let bytes = base64::engine::general_purpose::STANDARD
                .decode(v["params"]["data_b64"].as_str().unwrap())
                .unwrap();
            seen.push_str(&String::from_utf8_lossy(&bytes));
        }
    }
    seen
}

#[tokio::test]
async fn commands_run_through_the_owner_helper_end_to_end() {
    use synapta_connect::api::dispatch;
    let s = start(Mode::Service).await;
    let helper = real_helper(&s);
    wait_helper(&s, true).await;
    let (ctx, mut inbox) = ws_ctx();

    // exec: helper รันจริงแล้วส่งผลกลับ พร้อมบอกบัญชีที่รัน
    let shell = if cfg!(windows) { "cmd" } else { "sh" };
    let result = dispatch(
        &s.app,
        &ctx,
        "exec.run",
        json!({"command": "echo owner-exec", "shell": shell}),
    )
    .await
    .unwrap();
    assert_eq!(result["exit_code"], 0, "{result}");
    assert!(result["stdout"].as_str().unwrap().contains("owner-exec"), "{result}");
    assert!(result["run_as"]["user"].is_string(), "{result}");

    // terminal: เปิดใน helper, output แรกต้องถึง client (เส้นทางจองไว้ก่อน), เขียน/ปิดผ่าน helper ได้
    let opened = dispatch(&s.app, &ctx, "pty.open", json!({"shell": shell}))
        .await
        .unwrap();
    let id = opened["pty_id"].as_str().unwrap().to_string();
    let ptys = &ctx.ws.as_ref().unwrap().ptys;
    assert_eq!(ptys.is_owner(&id), Some(true));
    // ConPTY ถามตำแหน่ง cursor (ESC[6n) แล้วรอคำตอบก่อนเริ่ม shell — ตอบแทน terminal จริง
    if cfg!(windows) {
        dispatch(&s.app, &ctx, "pty.write", json!({"pty_id": id, "data": "\u{1b}[1;1R"}))
            .await
            .unwrap();
    }
    dispatch(
        &s.app,
        &ctx,
        "pty.write",
        json!({"pty_id": id, "data": "echo pty-owner-ok\r\n"}),
    )
    .await
    .unwrap();
    read_pty_until(&mut inbox, &id, "pty-owner-ok").await;
    dispatch(&s.app, &ctx, "pty.close", json!({"pty_id": id}))
        .await
        .unwrap();
    let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
    while ptys.is_owner(&id).is_some() {
        assert!(
            tokio::time::Instant::now() < deadline,
            "pty.exit ต้องมาถึงและลบ terminal ออกจากตาราง"
        );
        tokio::time::sleep(Duration::from_millis(50)).await;
    }

    // client อื่นสั่งงาน terminal ที่ไม่ใช่ของตัวเองไม่ได้
    let reopened = dispatch(&s.app, &ctx, "pty.open", json!({"shell": shell}))
        .await
        .unwrap();
    let (other, _) = ws_ctx();
    let stolen = dispatch(
        &s.app,
        &other,
        "pty.write",
        json!({"pty_id": reopened["pty_id"], "data": "x"}),
    )
    .await;
    assert!(stolen.is_err(), "terminal ของคนอื่นต้องเขียนไม่ได้");

    // helper หลุด → terminal ที่ค้างอยู่ได้ pty.exit
    helper.abort();
    wait_helper(&s, false).await;
    assert_eq!(
        ctx.ws
            .as_ref()
            .unwrap()
            .ptys
            .is_owner(reopened["pty_id"].as_str().unwrap()),
        None
    );
}

#[tokio::test]
async fn command_account_follows_the_owner_settings() {
    use synapta_connect::api::dispatch;
    let s = start(Mode::Service).await;
    let (ctx, _inbox) = ws_ctx();
    let shell = if cfg!(windows) { "cmd" } else { "sh" };
    let exec = |ctx| dispatch(&s.app, ctx, "exec.run", json!({"command": "echo here", "shell": shell}));

    // ไม่มี helper → รันในโปรเซส service ได้ตามเดิม
    assert_eq!(exec(&ctx).await.unwrap()["exit_code"], 0);

    // ต้องการ administrator แต่ไม่มี helper → ปฏิเสธพร้อมเหตุผล ไม่แอบรันด้วยบัญชี service
    let token = s.panel_token().await;
    let state: Value = s
        .req(reqwest::Method::PUT, "/api/v1/local/commands", Some(&token))
        .json(&json!({"administrator": true}))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(
        (state["run_as_owner"].as_bool(), state["administrator"].as_bool()),
        (Some(true), Some(true))
    );
    let refused = exec(&ctx).await.unwrap_err();
    assert!(refused.message.contains("administrator"), "{}", refused.message);

    // helper ที่ไม่ใช่ administrator → service ขอให้ helper ยกสิทธิ์ (UAC) ก่อนรัน
    let helper = connect_helper(&s, &s.file_token(), "owner").await.unwrap();
    wait_helper(&s, true).await;
    let pending = exec(&ctx).await.unwrap_err(); // helper ปลอมตอบ elevate ได้ แต่ไม่มีตัว administrator ต่อเข้ามา
    assert!(pending.message.contains("administrator"), "{}", pending.message);

    // เจ้าของปิดการรันด้วยบัญชีเจ้าของ → กลับไปรันในโปรเซส service แม้มี helper
    s.app
        .settings
        .update(|st| {
            st.commands.run_as_owner = false;
            st.commands.administrator = false;
        })
        .unwrap();
    assert_eq!(exec(&ctx).await.unwrap()["exit_code"], 0);
    helper.abort();
}
