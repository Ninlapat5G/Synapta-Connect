use serde_json::{Value, json};
use std::{
    net::{IpAddr, SocketAddr, TcpListener, UdpSocket},
    path::PathBuf,
    time::Duration,
};
use synapta_connect::{
    config::{Config, Ports},
    hex::random_id,
    panel_https, run_until,
};
use tokio::sync::oneshot;

struct Fixture {
    dir: PathBuf,
    config: Config,
}

impl Fixture {
    fn new() -> Self {
        let dir = std::env::temp_dir().join(format!("synapta-panel-net-{}", random_id(8)));
        std::fs::create_dir_all(&dir).unwrap();
        let a = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let b = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let udp = UdpSocket::bind(("127.0.0.1", 0)).unwrap();
        let config = Config {
            port: a.local_addr().unwrap().port(),
            local_port: b.local_addr().unwrap().port(),
            discovery_port: udp.local_addr().unwrap().port(),
            data_dir: dir.clone(),
            ..Config::default()
        };
        Self { dir, config }
    }

    async fn start(
        &self,
        client: &reqwest::Client,
    ) -> (String, oneshot::Sender<()>, tokio::task::JoinHandle<anyhow::Result<()>>) {
        let (stop, shutdown) = oneshot::channel();
        let task = tokio::spawn(run_until(self.config.clone(), async {
            let _ = shutdown.await;
        }));
        let base = self.base();
        tokio::time::timeout(Duration::from_secs(15), async {
            loop {
                if client
                    .get(format!("{base}/api/v1/local/health"))
                    .send()
                    .await
                    .is_ok_and(|r| r.status().is_success())
                {
                    break;
                }
                assert!(!task.is_finished(), "agent stopped during startup");
                tokio::time::sleep(Duration::from_millis(50)).await;
            }
        })
        .await
        .unwrap();
        (
            std::fs::read_to_string(self.dir.join("local_token")).unwrap(),
            stop,
            task,
        )
    }

    fn base(&self) -> String {
        format!("http://127.0.0.1:{}", self.config.local_port)
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

fn client() -> reqwest::ClientBuilder {
    let _ = rustls::crypto::ring::default_provider().install_default();
    reqwest::Client::builder().no_proxy().timeout(Duration::from_secs(3))
}

async fn code(client: &reqwest::Client, base: &str, native: &str, query: &str) -> Value {
    client
        .post(format!("{base}/api/v1/local/panel-session{query}"))
        .header("X-Synapta-Local", native)
        .send()
        .await
        .unwrap()
        .error_for_status()
        .unwrap()
        .json::<Value>()
        .await
        .unwrap()["code"]
        .clone()
}

fn exchange(client: &reqwest::Client, base: &str, code: &Value) -> reqwest::RequestBuilder {
    client
        .post(format!("{base}/api/v1/panel/exchange"))
        .json(&json!({ "code": code }))
}

async fn exchanged(client: &reqwest::Client, base: &str, code: &Value) -> String {
    exchange(client, base, code)
        .send()
        .await
        .unwrap()
        .error_for_status()
        .unwrap()
        .json::<Value>()
        .await
        .unwrap()["token"]
        .as_str()
        .unwrap()
        .to_string()
}

fn settings(
    client: &reqwest::Client,
    base: &str,
    token: &str,
    p: Ports,
    local: bool,
    lan: bool,
) -> reqwest::RequestBuilder {
    client
        .put(format!("{base}/api/v1/local/network"))
        .header("X-Synapta-Local", token)
        .json(&json!({
            "port": p.port, "local_port": p.local_port, "discovery_port": p.discovery_port,
            "local_enabled": local, "lan_enabled": lan,
        }))
}

#[tokio::test]
async fn ports_move_together_and_local_access_can_be_restored() {
    let fixture = Fixture::new();
    let client = client().build().unwrap();
    let (native, stop, task) = fixture.start(&client).await;
    let before = fixture.base();
    let opening = code(&client, &before, &native, "").await;
    let token = exchanged(&client, &before, &opening).await;
    let page = client
        .get(format!("{before}/synaptaconnect"))
        .send()
        .await
        .unwrap()
        .text()
        .await
        .unwrap();
    assert!(page.contains("/synaptaconnect/assets/"));

    let occupied = TcpListener::bind(("127.0.0.1", 0)).unwrap();
    let conflict = Ports {
        local_port: occupied.local_addr().unwrap().port(),
        ..fixture.config.ports()
    };
    assert_eq!(
        settings(&client, &before, &token, conflict, true, false)
            .send()
            .await
            .unwrap()
            .status(),
        409
    );
    assert_eq!(
        client
            .get(format!("{before}/synaptaconnect"))
            .send()
            .await
            .unwrap()
            .status(),
        200
    );

    let next_fixture = Fixture::new();
    let next = next_fixture.config.ports();
    let update = settings(&client, &before, &token, next, true, false)
        .send()
        .await
        .unwrap()
        .error_for_status()
        .unwrap()
        .json::<Value>()
        .await
        .unwrap();
    assert_eq!(update["local_port"], next.local_port);
    let after = next_fixture.base();
    assert!(
        client
            .get(format!("{before}/synaptaconnect"))
            .send()
            .await
            .map_or(true, |response| response.status() == 403)
    );
    let token = exchanged(&client, &after, &update["code"]).await;
    assert!(
        TcpListener::bind(("127.0.0.1", next.port)).is_err(),
        "Agent TCP is bound at the new port"
    );
    let probe = tokio::net::UdpSocket::bind(("127.0.0.1", 0)).await.unwrap();
    probe
        .send_to(br#"{"probe":"synapta-connect"}"#, ("127.0.0.1", next.discovery_port))
        .await
        .unwrap();
    let mut bytes = [0; 512];
    let (size, _) = tokio::time::timeout(Duration::from_secs(2), probe.recv_from(&mut bytes))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        serde_json::from_slice::<Value>(&bytes[..size]).unwrap()["port"],
        next.port
    );

    assert_eq!(
        settings(&client, &after, &token, next, false, false)
            .send()
            .await
            .unwrap()
            .status(),
        200
    );
    assert_eq!(
        client
            .get(format!("{after}/synaptaconnect"))
            .send()
            .await
            .unwrap()
            .status(),
        403
    );
    assert_eq!(
        client
            .get(format!("{after}/api/v1/local/status"))
            .header("X-Synapta-Local", &token)
            .send()
            .await
            .unwrap()
            .status(),
        403
    );
    assert_eq!(
        client
            .get(format!("{after}/api/v1/local/health"))
            .send()
            .await
            .unwrap()
            .status(),
        200
    );
    assert_eq!(
        client
            .post(format!("{after}/api/v1/local/enable-panel"))
            .header("X-Synapta-Local", &native)
            .send()
            .await
            .unwrap()
            .status(),
        200
    );
    assert_eq!(
        client
            .get(format!("{after}/synaptaconnect"))
            .send()
            .await
            .unwrap()
            .status(),
        200
    );
    assert_eq!(
        synapta_connect::settings::SettingsStore::load(&fixture.dir)
            .get()
            .network
            .ports,
        Some(next)
    );
    stop.send(()).unwrap();
    task.await.unwrap().unwrap();
}

#[tokio::test]
async fn lan_https_uses_its_own_codes_and_denies_native_routes() {
    let Some(ip) = synapta_connect::netif::list().into_iter().find_map(|entry| {
        if let IpAddr::V4(ip) = entry.ip
            && entry.kind == "physical"
            && ip.is_private()
        {
            Some(ip)
        } else {
            None
        }
    }) else {
        eprintln!("LAN socket test skipped: no private physical IPv4");
        return;
    };
    let fixture = Fixture::new();
    let rcgen::CertifiedKey { cert, signing_key } =
        rcgen::generate_simple_self_signed(vec!["synapta-test.lan".to_string(), ip.to_string()]).unwrap();
    std::fs::write(fixture.dir.join(panel_https::CERT_FILE), cert.pem()).unwrap();
    std::fs::write(fixture.dir.join(panel_https::KEY_FILE), signing_key.serialize_pem()).unwrap();
    let client = client()
        .add_root_certificate(reqwest::Certificate::from_pem(cert.pem().as_bytes()).unwrap())
        .resolve("synapta-test.lan", SocketAddr::new(ip.into(), 0))
        .build()
        .unwrap();
    let (native, stop, task) = fixture.start(&client).await;
    let local = fixture.base();
    let lan = format!("https://synapta-test.lan:{}", fixture.config.local_port);
    let direct_ip = format!("https://{ip}:{}", fixture.config.local_port);
    assert!(client.get(format!("{lan}/synaptaconnect")).send().await.is_err());
    assert_eq!(
        settings(&client, &local, &native, fixture.config.ports(), true, true)
            .send()
            .await
            .unwrap()
            .status(),
        200
    );
    for base in [&lan, &direct_ip] {
        assert_eq!(
            client
                .get(format!("{base}/synaptaconnect"))
                .send()
                .await
                .unwrap()
                .status(),
            200
        );
    }
    let host_port = fixture.config.local_port;
    assert!(
        client
            .get(format!("http://{ip}:{host_port}/synaptaconnect"))
            .send()
            .await
            .is_err()
    );
    let local_code = code(&client, &local, &native, "").await;
    assert_eq!(exchange(&client, &lan, &local_code).send().await.unwrap().status(), 403);
    let lan_code = code(&client, &local, &native, "?lan=synapta-test.lan").await;
    assert_eq!(
        exchange(&client, &direct_ip, &lan_code).send().await.unwrap().status(),
        403
    );
    let lan_token = exchanged(&client, &lan, &lan_code).await;
    assert_eq!(exchange(&client, &lan, &lan_code).send().await.unwrap().status(), 403);
    assert_eq!(
        client
            .get(format!("{lan}/api/v1/local/health"))
            .send()
            .await
            .unwrap()
            .status(),
        404
    );
    assert_eq!(
        client
            .post(format!("{lan}/api/v1/local/enable-panel"))
            .header("X-Synapta-Local", &native)
            .send()
            .await
            .unwrap()
            .status(),
        404
    );
    for invalid in [&native, &exchanged(&client, &local, &local_code).await] {
        assert_eq!(
            client
                .get(format!("{lan}/api/v1/local/status"))
                .header("X-Synapta-Local", invalid)
                .send()
                .await
                .unwrap()
                .status(),
            403
        );
    }
    let request = || {
        client
            .get(format!("{lan}/api/v1/local/status"))
            .header("X-Synapta-Local", &lan_token)
    };
    assert_eq!(
        request()
            .header("Origin", "https://invalid.example")
            .send()
            .await
            .unwrap()
            .status(),
        403
    );
    assert_eq!(
        request()
            .header("Host", "invalid.example")
            .send()
            .await
            .unwrap()
            .status(),
        403
    );
    assert_eq!(request().header("Origin", &lan).send().await.unwrap().status(), 200);
    assert_eq!(
        settings(&client, &local, &native, fixture.config.ports(), true, false)
            .send()
            .await
            .unwrap()
            .status(),
        200
    );
    assert!(request().send().await.is_err());
    stop.send(()).unwrap();
    task.await.unwrap().unwrap();
}
