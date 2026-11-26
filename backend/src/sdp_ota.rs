//! อัปเดตเฟิร์มแวร์บอร์ด SDP (ESP32) ผ่าน OTA — RPC `sdp.ota.*` เรียก CLI ของ SDP checkout ที่เจ้าของเครื่องตั้งไว้
//! client ส่งได้แค่ ID; path ของ python/สคริปต์/กุญแจ/ไฟล์เฟิร์มแวร์มาจาก config ฝั่งเครื่องเท่านั้น (คู่มือ docs/SDP_OTA.md)

use crate::{
    app::App,
    config::Mode,
    rpc::{BUSY, NOT_AVAILABLE, Res, RpcError, parse},
    update::version::Version,
};
use serde::Deserialize;
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    net::Ipv4Addr,
    path::{Path, PathBuf},
    process::Stdio,
    time::Duration,
};
use tokio::{io::AsyncReadExt, process::Command, sync::Mutex};

const MODULE_ID: &str = "sdp-ota";
const ENTRYPOINT: &str = "sdp_ota.py";
const MAX_OUTPUT: usize = 32 * 1024;

/// อ่าน config ใหม่ทุก RPC เพื่อให้อัปเดตโมดูล SDP แล้วใช้ได้ทันทีโดยไม่ต้องรีสตาร์ต Connect
pub(crate) struct Ota {
    config_path: Option<PathBuf>,
    /// อัปโหลดได้ทีละงาน — สองงานพร้อมกันจะแย่งบอร์ดและผลยืนยันจะสับสน
    write: Mutex<()>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Config {
    sdp_root: PathBuf,
    python: PathBuf,
    artifact_root: Option<PathBuf>,
    #[serde(default)]
    allow_firmware_write: bool,
    #[serde(default)]
    write_clients: Vec<String>,
    #[serde(default)]
    boards: BTreeMap<String, Board>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Board {
    host: Ipv4Addr,
    project_id: String,
    key_file: PathBuf,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Manifest {
    schema_version: u32,
    id: String,
    version: String,
    entrypoint: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ModuleParams {}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct StatusParams {
    device_id: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct UploadParams {
    device_id: String,
    project_id: String,
    build_id: String,
}

struct Module {
    config: Config,
    script: PathBuf,
    version: String,
}

impl Ota {
    pub(crate) fn new() -> Self {
        Self {
            config_path: std::env::var_os("SYNAPTA_SDP_OTA_CONFIG").map(PathBuf::from),
            write: Mutex::new(()),
        }
    }

    pub(crate) async fn rpc(&self, app: &App, client: &str, method: &str, params: Value) -> Res<Value> {
        let kills = app.auth.subscribe_kills();
        if !app.auth.is_active(client) || (app.config.mode == Mode::Portable && !app.auth.portable_attached(client)) {
            return Err(unavailable("Active Connect pairing required"));
        }
        // pairing ถูกถอนหรือปิดเซสชัน Portable กลางทาง = ทิ้ง future → kill_on_drop ฆ่า CLI ที่รันอยู่
        let device = Some(client.to_owned());
        let revoked = crate::tls::wait_for_kill(app, &device, kills);
        tokio::select! {
            biased;
            _ = revoked => Err(unavailable("Connect pairing ended")),
            result = self.call(client, method, params) => result,
        }
    }

    async fn call(&self, client: &str, method: &str, params: Value) -> Res<Value> {
        if method == "sdp.ota.module" {
            parse::<ModuleParams>(params)?;
            let Some(path) = &self.config_path else {
                return Ok(json!({"id": MODULE_ID, "status": "unconfigured", "version": null, "firmwareWrite": false}));
            };
            return Ok(match Module::load(path).await {
                Ok(module) => json!({"id": MODULE_ID, "status": "ready", "schemaVersion": 1,
                    "version": module.version, "firmwareWrite": module.config.can_write(client)}),
                Err(error) => json!({"id": MODULE_ID, "status": "unavailable", "version": null,
                    "firmwareWrite": false, "error": error.message}),
            });
        }

        let path = self
            .config_path
            .as_deref()
            .ok_or_else(|| unavailable("SDP OTA is not configured"))?;
        let module = Module::load(path).await?;
        if method == "sdp.ota.status" {
            let request: StatusParams = parse(params)?;
            return module.status(&request.device_id).await;
        }

        let request: UploadParams = parse(params)?;
        if !module.config.can_write(client) {
            tracing::warn!(session = %client, "SDP OTA firmware write denied");
            return Err(unavailable("SDP OTA firmware write is disabled for this pairing"));
        }
        let _guard = self
            .write
            .try_lock()
            .map_err(|_| RpcError::new(BUSY, "An SDP firmware upload is already running"))?;
        module.upload(client, request).await
    }
}

impl Config {
    fn can_write(&self, client: &str) -> bool {
        self.allow_firmware_write && self.write_clients.iter().any(|id| id == client)
    }
}

impl Module {
    /// ตรวจ config + manifest; สคริปต์ต้องอยู่ใต้ sdpRoot จริงหลัง canonicalize (กัน symlink พาออกนอก checkout)
    async fn load(path: &Path) -> Res<Self> {
        let config_path = existing_file(path, "SDP OTA configuration is unavailable").await?;
        let mut config: Config = read_json(&config_path, "Invalid SDP OTA configuration").await?;
        config.python = existing_file(&config.python, "Configured Python executable is unavailable").await?;
        let root = existing_directory(&config.sdp_root, "Trusted SDP checkout is unavailable").await?;
        let dir = root.join("tools").join(MODULE_ID);
        let manifest_file = contained_file(&root, &dir.join("module.json"), "SDP OTA manifest is unavailable").await?;
        let manifest: Manifest = read_json(&manifest_file, "Invalid SDP OTA manifest").await?;
        if manifest.schema_version != 1
            || manifest.id != MODULE_ID
            || manifest.entrypoint != ENTRYPOINT
            || Version::parse(&manifest.version).is_none()
        {
            return Err(unavailable("Unsupported SDP OTA manifest"));
        }
        let script = contained_file(&root, &dir.join(ENTRYPOINT), "SDP OTA entrypoint is unavailable").await?;
        Ok(Self {
            config,
            script,
            version: manifest.version,
        })
    }

    fn board(&self, id: &str) -> Res<&Board> {
        if id.len() != 16 || !id.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b)) {
            return Err(RpcError::invalid(
                "deviceId must be 16 lowercase hexadecimal characters",
            ));
        }
        let board = self
            .config
            .boards
            .get(id)
            .ok_or_else(|| unavailable("SDP board is not configured on this host"))?;
        if !identifier(&board.project_id) {
            return Err(unavailable("Invalid SDP board project configuration"));
        }
        Ok(board)
    }

    async fn command(&self, operation: &str, board: &Board) -> Res<Command> {
        let key = existing_file(&board.key_file, "Configured SDP pairing credential is unavailable").await?;
        let mut command = Command::new(&self.config.python);
        command
            .arg("-I")
            .arg("-B")
            .arg(&self.script)
            .arg(operation)
            .arg("--host")
            .arg(board.host.to_string())
            .arg("--key-file")
            .arg(key);
        Ok(command)
    }

    async fn status(&self, id: &str) -> Res<Value> {
        let board = self.board(id)?;
        let (code, output) = run(self.command("status", board).await?, Duration::from_secs(12))
            .await
            .map_err(|_| unavailable("SDP OTA status failed"))?;
        let [status] = output.as_slice() else {
            return Err(unavailable("Invalid SDP OTA status response"));
        };
        let build = string(status, "runningBuildId");
        let state = string(status, "bootState");
        if code != Some(0)
            || string(status, "type") != "status"
            || string(status, "deviceId") != id
            || string(status, "projectId") != board.project_id
            || !identifier(build)
            || !matches!(state, "confirmed" | "pending" | "rolled_back")
        {
            return Err(unavailable("SDP OTA status did not verify the configured board"));
        }
        Ok(json!({"type": "status", "deviceId": id, "projectId": board.project_id,
            "runningBuildId": build, "bootState": state}))
    }

    async fn upload(&self, client: &str, request: UploadParams) -> Res<Value> {
        let board = self.board(&request.device_id)?;
        if request.project_id != board.project_id || !identifier(&request.build_id) {
            return Err(RpcError::invalid(
                "projectId must match the configured board and buildId must be valid",
            ));
        }
        let root = self
            .config
            .artifact_root
            .as_deref()
            .ok_or_else(|| unavailable("SDP OTA artifact root is not configured"))?;
        let root = existing_directory(root, "SDP OTA artifact root is unavailable").await?;
        let image = contained_file(
            &root,
            &root
                .join(&request.project_id)
                .join(&request.build_id)
                .join("sdp_ota.bin"),
            "Requested SDP firmware artifact is unavailable",
        )
        .await?;
        let mut command = self.command("upload", board).await?;
        command
            .arg("--device-id")
            .arg(&request.device_id)
            .arg("--project-id")
            .arg(&request.project_id)
            .arg("--build-id")
            .arg(&request.build_id)
            .arg("--image")
            .arg(image);
        tracing::info!(session = %client, device = %request.device_id, project = %request.project_id,
            build = %request.build_id, "SDP OTA firmware upload started");
        let reply = match run(command, Duration::from_secs(150)).await {
            Ok((code, records)) => upload_reply(&request, code, &records),
            Err(failure) => upload_unknown(&request, failure),
        };
        tracing::info!(session = %client, device = %request.device_id, project = %request.project_id,
            build = %request.build_id, outcome = string(&reply["result"], "outcome"), "SDP OTA firmware upload finished");
        Ok(reply)
    }
}

/// ผลจะเป็น verified/rolled_back ได้ก็ต่อเมื่อ exit code, บอร์ด, project และ build ตรงกับที่สั่งทุกข้อ ไม่งั้นเป็น unknown
fn upload_reply(request: &UploadParams, code: Option<i32>, records: &[Value]) -> Value {
    let Some((result, events)) = records.split_last() else {
        return upload_unknown(request, "Missing OTA result");
    };
    let outcome = string(result, "outcome");
    let expected_exit = match outcome {
        "verified" => 0,
        "failed" => 1,
        "rolled_back" => 2,
        "unknown" => 3,
        _ => return upload_unknown(request, "Invalid OTA outcome"),
    };
    let running = result.get("runningBuildId").and_then(Value::as_str);
    if code != Some(expected_exit)
        || string(result, "type") != "result"
        || string(result, "deviceId") != request.device_id
        || string(result, "projectId") != request.project_id
        || string(result, "buildId") != request.build_id
        || running.is_some_and(|b| !identifier(b))
        || (outcome == "verified" && running != Some(request.build_id.as_str()))
        || (outcome == "rolled_back" && (running.is_none() || running == Some(request.build_id.as_str())))
    {
        return upload_unknown(request, "OTA response did not match the requested board and build");
    }
    let progress: Vec<Value> = events
        .iter()
        .filter_map(|event| {
            if string(event, "type") != "progress" {
                return None;
            }
            let stage = string(event, "stage");
            match stage {
                "checking_device" | "verifying" => Some(json!({"type": "progress", "stage": stage})),
                "uploading" => {
                    let sent = event.get("bytesSent")?.as_u64()?;
                    let total = event.get("bytesTotal")?.as_u64()?;
                    (total > 0 && sent <= total).then(|| {
                        json!({"type": "progress", "stage": stage,
                    "bytesSent": sent, "bytesTotal": total})
                    })
                }
                _ => None,
            }
        })
        .collect();
    // CLI diagnostics may include private local paths. Return a stable outcome instead.
    let mut result = json!({"type": "result", "outcome": outcome, "deviceId": request.device_id,
        "projectId": request.project_id, "buildId": request.build_id, "runningBuildId": running});
    if outcome == "failed" {
        result["error"] = json!("SDP OTA reported a failure");
    }
    if outcome == "unknown" {
        result["error"] = json!("The board's post-reboot build could not be verified");
    }
    json!({"progress": progress, "result": result})
}

fn upload_unknown(request: &UploadParams, error: &str) -> Value {
    json!({"progress": [], "result": {"type": "result", "outcome": "unknown", "deviceId": request.device_id,
        "projectId": request.project_id, "buildId": request.build_id, "runningBuildId": null, "error": error}})
}

/// รัน CLI แล้วอ่าน stdout เป็น NDJSON (จำกัดขนาดและเวลา) — stderr ทิ้งเพราะอาจมี path ส่วนตัว
async fn run(mut command: Command, timeout: Duration) -> Result<(Option<i32>, Vec<Value>), &'static str> {
    let mut child = command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .kill_on_drop(true)
        .spawn()
        .map_err(|_| "Could not start the SDP OTA CLI")?;
    let mut stdout = child
        .stdout
        .take()
        .expect("stdout is piped")
        .take((MAX_OUTPUT + 1) as u64);
    tokio::time::timeout(timeout, async {
        let mut output = Vec::new();
        stdout
            .read_to_end(&mut output)
            .await
            .map_err(|_| "Could not read the SDP OTA CLI response")?;
        if output.len() > MAX_OUTPUT {
            return Err("SDP OTA CLI response exceeded its limit");
        }
        let code = child
            .wait()
            .await
            .map_err(|_| "Could not read the SDP OTA CLI exit status")?
            .code();
        let records = output
            .split(|byte| *byte == b'\n')
            .filter(|line| !line.is_empty())
            .map(serde_json::from_slice)
            .collect::<Result<Vec<_>, _>>()
            .map_err(|_| "SDP OTA CLI did not return valid NDJSON")?;
        Ok((code, records))
    })
    .await
    .map_err(|_| "SDP OTA CLI timed out")?
}

async fn read_json<T: serde::de::DeserializeOwned>(path: &Path, error: &str) -> Res<T> {
    let file = tokio::fs::File::open(path).await.map_err(|_| unavailable(error))?;
    let mut bytes = Vec::new();
    file.take(64 * 1024 + 1)
        .read_to_end(&mut bytes)
        .await
        .map_err(|_| unavailable(error))?;
    if bytes.len() > 64 * 1024 {
        return Err(unavailable(error));
    }
    serde_json::from_slice(&bytes).map_err(|_| unavailable(error))
}

async fn existing_file(path: &Path, error: &str) -> Res<PathBuf> {
    let path = canonical(path, error).await?;
    if !tokio::fs::metadata(&path).await.is_ok_and(|m| m.is_file()) {
        return Err(unavailable(error));
    }
    Ok(path)
}

async fn existing_directory(path: &Path, error: &str) -> Res<PathBuf> {
    let path = canonical(path, error).await?;
    if !tokio::fs::metadata(&path).await.is_ok_and(|m| m.is_dir()) {
        return Err(unavailable(error));
    }
    Ok(path)
}

async fn canonical(path: &Path, error: &str) -> Res<PathBuf> {
    if !path.is_absolute() {
        return Err(unavailable(error));
    }
    tokio::fs::canonicalize(path).await.map_err(|_| unavailable(error))
}

async fn contained_file(root: &Path, file: &Path, error: &str) -> Res<PathBuf> {
    let file = existing_file(file, error).await?;
    if !file.starts_with(root) {
        return Err(unavailable(error));
    }
    Ok(file)
}

/// กติกาเดียวกับ IDENTIFIER ของ SDP CLI / SynaptaCode และกันชื่ออุปกรณ์สงวนของ Windows เพราะใช้เป็นชื่อโฟลเดอร์
fn identifier(text: &str) -> bool {
    let bytes = text.as_bytes();
    let alnum = |b: u8| b.is_ascii_lowercase() || b.is_ascii_digit();
    let reserved = matches!(text, "con" | "prn" | "aux" | "nul")
        || (bytes.len() == 4 && (text.starts_with("com") || text.starts_with("lpt")) && bytes[3].is_ascii_digit());
    !reserved
        && bytes.len() <= 96
        && bytes.first().is_some_and(|b| alnum(*b))
        && bytes.iter().all(|b| alnum(*b) || matches!(b, b'-' | b'_'))
}

fn string<'a>(value: &'a Value, key: &str) -> &'a str {
    value.get(key).and_then(Value::as_str).unwrap_or_default()
}

fn unavailable(message: &str) -> RpcError {
    RpcError::new(NOT_AVAILABLE, message)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{api::Ctx, app::AppState};
    use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};

    const BOARD: &str = "1234567890abcdef";

    #[test]
    fn sdp_ota_result_requires_matching_identity_build_and_exit_code() {
        let request = UploadParams {
            device_id: BOARD.into(),
            project_id: "project".into(),
            build_id: "latest".into(),
        };
        let valid = json!({"type": "result", "outcome": "verified", "deviceId": BOARD, "projectId": "project",
            "buildId": "latest", "runningBuildId": "latest"});
        assert_eq!(
            upload_reply(&request, Some(0), std::slice::from_ref(&valid))["result"]["outcome"],
            "verified"
        );
        assert_eq!(
            upload_reply(&request, Some(2), std::slice::from_ref(&valid))["result"]["outcome"],
            "unknown"
        );
        for field in ["deviceId", "projectId", "buildId", "runningBuildId"] {
            let mut mismatched = valid.clone();
            mismatched[field] = json!("different");
            assert_eq!(
                upload_reply(&request, Some(0), &[mismatched])["result"]["outcome"],
                "unknown"
            );
        }
        let mut rolled_back = valid;
        rolled_back["outcome"] = json!("rolled_back");
        rolled_back["runningBuildId"] = json!("previous");
        assert_eq!(
            upload_reply(&request, Some(2), &[rolled_back])["result"]["outcome"],
            "rolled_back"
        );
    }

    #[tokio::test]
    #[ignore = "requires SYNAPTA_SDP_TEST_PYTHON pointing to Python 3.11+"]
    async fn sdp_ota_fake_cli_checks_live_manifest_pairing_and_firmware_permission() {
        fake_cli(Mode::Console).await;
        fake_cli(Mode::Portable).await;
    }

    async fn fake_cli(mode: Mode) {
        let python = std::env::var_os("SYNAPTA_SDP_TEST_PYTHON").expect("set SYNAPTA_SDP_TEST_PYTHON");
        let root = std::env::temp_dir().join(format!("synapta-ota-test-{}", crate::hex::random_id(8)));
        let tools = root.join("trusted checkout/tools/sdp-ota");
        let artifacts = root.join("artifacts");
        let build = artifacts.join("project/latest");
        std::fs::create_dir_all(&tools).unwrap();
        std::fs::create_dir_all(&build).unwrap();
        let manifest = tools.join("module.json");
        let marker = tools.join("write-called");
        let key = root.join("fake-key");
        std::fs::write(&key, "test credential").unwrap();
        std::fs::write(build.join("sdp_ota.bin"), "stub image").unwrap();
        std::fs::write(
            tools.join(ENTRYPOINT),
            r#"
import argparse
import json
from pathlib import Path

parser = argparse.ArgumentParser()
parser.add_argument('operation', choices=['status', 'upload'])
parser.add_argument('--host', required=True)
parser.add_argument('--key-file', type=Path, required=True)
parser.add_argument('--device-id')
parser.add_argument('--project-id')
parser.add_argument('--build-id')
parser.add_argument('--image', type=Path)
args = parser.parse_args()
assert args.host == '192.0.2.42' and args.key_file.read_text() == 'test credential'
if args.operation == 'status':
    print(json.dumps(dict(type='status', deviceId='1234567890abcdef', projectId='project',
                          bootState='confirmed', runningBuildId='previous', challenge='private')))
else:
    assert args.image.read_text() == 'stub image'
    Path(__file__).with_name('write-called').write_text('yes')
    print(json.dumps(dict(type='progress', stage='uploading', bytesSent=10, bytesTotal=10)))
    print(json.dumps(dict(type='result', outcome='verified', deviceId=args.device_id,
                          projectId=args.project_id, buildId=args.build_id, runningBuildId=args.build_id)))
"#,
        )
        .unwrap();
        let mut app = AppState::new(crate::config::Config {
            mode,
            data_dir: root.join("agent"),
            ..Default::default()
        })
        .unwrap();
        let path = root.join("ota.json");
        let state = std::sync::Arc::get_mut(&mut app).unwrap();
        state.sdp_ota.config_path = Some(path.clone());
        if mode == Mode::Portable {
            state.auth.portable_otp().unwrap();
        } else {
            state.auth.open_pairing();
        }
        let (_, generation) = state.auth.begin_pairing().unwrap();
        let keypair = rcgen::KeyPair::generate_for(&rcgen::PKCS_ED25519).unwrap();
        let public = URL_SAFE_NO_PAD.encode(keypair.public_key_raw());
        let client = state.auth.complete_pairing(generation, &public, "test").unwrap().id;
        let unpaired = Ctx {
            session_id: "other".into(),
            ws: None,
        };
        assert!(
            crate::api::dispatch(&app, &unpaired, "sdp.ota.module", Value::Null)
                .await
                .is_err()
        );
        let ctx = Ctx {
            session_id: client.clone(),
            ws: None,
        };
        let mut cfg = json!({"sdpRoot": root.join("trusted checkout"), "python": PathBuf::from(python),
            "artifactRoot": artifacts, "boards": {(BOARD): {"host": "192.0.2.42", "keyFile": key, "projectId": "project"}}});
        let mut module_manifest =
            json!({"schemaVersion": 1, "id": MODULE_ID, "version": "1.0.0", "entrypoint": ENTRYPOINT});
        crate::fsutil::write_json(&path, &cfg).unwrap();
        crate::fsutil::write_json(&manifest, &module_manifest).unwrap();
        let call = |method, params| crate::api::dispatch(&app, &ctx, method, params);
        let args = json!({"deviceId": BOARD, "projectId": "project", "buildId": "latest"});
        if mode == Mode::Portable {
            assert!(call("sdp.ota.module", Value::Null).await.is_err());
            assert!(app.auth.attach_portable(&client));
        }
        let module = call("sdp.ota.module", Value::Null).await.unwrap();
        assert_eq!(module["version"], "1.0.0");
        assert_eq!(module["firmwareWrite"], false);
        assert!(call("sdp.ota.upload", args.clone()).await.is_err());
        assert!(!marker.exists());
        let status = call("sdp.ota.status", json!({"deviceId": BOARD})).await.unwrap();
        assert_eq!(status["runningBuildId"], "previous");
        assert!(status.get("challenge").is_none());
        cfg["allowFirmwareWrite"] = json!(true);
        crate::fsutil::write_json(&path, &cfg).unwrap();
        assert!(call("sdp.ota.upload", args.clone()).await.is_err());
        cfg["writeClients"] = json!([client]);
        crate::fsutil::write_json(&path, &cfg).unwrap();
        let mut injected = args.clone();
        injected["image"] = json!("arbitrary-path");
        assert_eq!(
            call("sdp.ota.upload", injected).await.unwrap_err().code,
            crate::rpc::INVALID_PARAMS
        );
        assert!(!marker.exists());
        let response = call("sdp.ota.upload", args).await.unwrap();
        assert_eq!(response["result"]["outcome"], "verified");
        assert_eq!(response["progress"][0]["bytesSent"], 10);
        assert!(marker.exists());
        module_manifest["version"] = json!("1.0.1");
        crate::fsutil::write_json(&manifest, &module_manifest).unwrap();
        assert_eq!(call("sdp.ota.module", Value::Null).await.unwrap()["version"], "1.0.1");
        module_manifest["entrypoint"] = json!("../other.py");
        crate::fsutil::write_json(&manifest, &module_manifest).unwrap();
        assert_eq!(
            call("sdp.ota.module", Value::Null).await.unwrap()["status"],
            "unavailable"
        );
        app.auth.revoke(&ctx.session_id);
        assert!(call("sdp.ota.module", Value::Null).await.is_err());
        std::fs::remove_dir_all(root).unwrap();
    }
}
