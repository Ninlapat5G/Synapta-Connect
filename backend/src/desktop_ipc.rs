use crate::{app::App, config::service_data_dir, proof};
use axum::{
    Router,
    body::{Body, to_bytes},
    http::{Method, Request, header},
};
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use serde_json::Value;
use std::{
    io,
    os::windows::io::{FromRawHandle, IntoRawHandle, OwnedHandle},
    time::Duration,
};
use tokio::{
    io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt},
    net::windows::named_pipe::{NamedPipeClient, NamedPipeServer, ServerOptions},
    task::JoinSet,
    time::{sleep, timeout},
};
use tower::ServiceExt;
use windows::{
    Win32::{
        Foundation::{ERROR_PIPE_BUSY, HLOCAL, LocalFree},
        Security::{
            Authorization::{ConvertStringSecurityDescriptorToSecurityDescriptorW, SDDL_REVISION_1},
            PSECURITY_DESCRIPTOR, SECURITY_ATTRIBUTES,
        },
        Storage::FileSystem::{
            CreateFileW, FILE_FLAG_OVERLAPPED, FILE_FLAGS_AND_ATTRIBUTES, FILE_SHARE_MODE, OPEN_EXISTING,
        },
    },
    core::w,
};

const PIPE: &str = r"\\.\pipe\SynaptaConnect.Control.v1";
const MAX_REQUEST: usize = 64 * 1024;
const MAX_RESPONSE: usize = 2 * 1024 * 1024;
const TIMEOUT: Duration = Duration::from_secs(8);
const SERVER: &str = "desktop-server";
const REQUEST: &str = "desktop-request";
const RESPONSE: &str = "desktop-response";

fn pipe_name(token: &str) -> String {
    let id = proof::make(token, "desktop-pipe", "");
    format!("{PIPE}.{}", &id[..32])
}

#[derive(Serialize, Deserialize)]
pub struct Reply {
    pub status: u16,
    pub data: Value,
}

#[derive(Serialize, Deserialize)]
struct Frame {
    text: String,
    mac: String,
}

impl Frame {
    fn sign(token: &str, label: &str, context: &str, text: String) -> Self {
        let mac = proof::make(token, label, &format!("{context}|{text}"));
        Self { text, mac }
    }

    fn verify(&self, token: &str, label: &str, context: &str) -> io::Result<()> {
        if proof::check(token, label, &format!("{context}|{}", self.text), &self.mac) {
            Ok(())
        } else {
            Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "ยืนยันตัวตน Desktop หรือ Agent ไม่สำเร็จ",
            ))
        }
    }
}

#[derive(Serialize, Deserialize)]
struct Call {
    method: String,
    path: String,
    body: Option<Value>,
}

// Only these panel actions can cross the browser-to-native boundary.
fn allowed(method: &str, path: &str) -> bool {
    let base = "/api/v1/local/";
    let Some(path) = path.strip_prefix(base) else {
        return false;
    };
    match (method, path) {
        ("GET", "health" | "status" | "update" | "web-update" | "network" | "screen-input")
        | ("PUT", "update/settings" | "screen-input" | "network")
        | (
            "POST",
            "pairing/open"
            | "pairing/close"
            | "kill-all"
            | "update/check"
            | "update/apply"
            | "update/retry"
            | "web-update/check"
            | "web-update/apply"
            | "web-update/retry"
            | "web-update/rollback"
            | "recover-web"
            | "restart-web",
        ) => true,
        ("POST", path) => path
            .strip_prefix("devices/")
            .and_then(|p| p.strip_suffix("/revoke"))
            .is_some_and(|id| {
                !id.is_empty() && id.len() <= 128 && id.bytes().all(|b| b.is_ascii_alphanumeric() || b"-_".contains(&b))
            }),
        _ => false,
    }
}

async fn read<T: DeserializeOwned>(pipe: &mut (impl AsyncRead + Unpin), max: usize) -> io::Result<T> {
    let length = pipe.read_u32().await? as usize;
    if length > max {
        return Err(io::ErrorKind::InvalidData.into());
    }
    let mut bytes = vec![0; length];
    pipe.read_exact(&mut bytes).await?;
    Ok(serde_json::from_slice(&bytes)?)
}

async fn write(pipe: &mut (impl AsyncWrite + Unpin), value: &impl Serialize, max: usize) -> io::Result<()> {
    let bytes = serde_json::to_vec(value)?;
    if bytes.len() > max {
        return Err(io::ErrorKind::InvalidData.into());
    }
    pipe.write_u32(bytes.len() as u32).await?;
    pipe.write_all(&bytes).await
}

fn fresh_nonce(nonce: &str) -> bool {
    nonce.len() == 32 && nonce.bytes().all(|b| b.is_ascii_hexdigit())
}

fn create(name: &str, first: bool) -> io::Result<NamedPipeServer> {
    let mut descriptor = PSECURITY_DESCRIPTOR::default();
    // Authenticated local users can open a client, but only the server can create other instances.
    // Management still requires the secret restricted to the configured owner and administrators.
    unsafe {
        ConvertStringSecurityDescriptorToSecurityDescriptorW(
            w!("D:P(A;;GA;;;SY)(A;;GA;;;BA)(A;;GA;;;OW)(A;;0x12019b;;;AU)"),
            SDDL_REVISION_1,
            &mut descriptor,
            None,
        )?;
    }
    let mut attributes = SECURITY_ATTRIBUTES {
        nLength: std::mem::size_of::<SECURITY_ATTRIBUTES>() as u32,
        lpSecurityDescriptor: descriptor.0,
        bInheritHandle: false.into(),
    };
    let server = unsafe {
        ServerOptions::new()
            .first_pipe_instance(first)
            .reject_remote_clients(true)
            .create_with_security_attributes_raw(name, (&mut attributes as *mut SECURITY_ATTRIBUTES).cast())
    };
    unsafe {
        let _ = LocalFree(Some(HLOCAL(descriptor.0)));
    }
    server
}

async fn dispatch(router: Router, token: &str, call: Call) -> io::Result<Reply> {
    if !allowed(&call.method, &call.path) {
        return Ok(Reply {
            status: 403,
            data: serde_json::json!({"error": {"message": "คำสั่งนี้ใช้จาก Desktop ไม่ได้"}}),
        });
    }
    if call.method != "GET" {
        tracing::info!(method = %call.method, path = %call.path, "Desktop control request");
    }
    let mut request = Request::builder()
        .method(call.method.parse::<Method>().map_err(io::Error::other)?)
        .uri(call.path)
        .header("x-synapta-local", token);
    let body = if let Some(value) = call.body {
        request = request.header(header::CONTENT_TYPE, "application/json");
        Body::from(serde_json::to_vec(&value)?)
    } else {
        Body::empty()
    };
    let response = router
        .oneshot(request.body(body).map_err(io::Error::other)?)
        .await
        .expect("infallible router");
    let status = response.status().as_u16();
    let bytes = to_bytes(response.into_body(), MAX_RESPONSE)
        .await
        .map_err(io::Error::other)?;
    let data = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
    Ok(Reply { status, data })
}

async fn session(mut pipe: NamedPipeServer, router: Router, app: App) -> io::Result<()> {
    let client_nonce: String = read(&mut pipe, 128).await?;
    if !fresh_nonce(&client_nonce) {
        return Err(io::ErrorKind::InvalidData.into());
    }

    let token = &app.local_token;
    let server_nonce = crate::hex::random_id(16);
    write(
        &mut pipe,
        &Frame::sign(token, SERVER, &client_nonce, server_nonce.clone()),
        256,
    )
    .await?;
    let call: Frame = read(&mut pipe, MAX_REQUEST).await?;
    call.verify(token, REQUEST, &server_nonce)?;
    let reply = dispatch(router, token, serde_json::from_str(&call.text)?).await?;
    write(
        &mut pipe,
        &Frame::sign(token, RESPONSE, &call.mac, serde_json::to_string(&reply)?),
        MAX_RESPONSE,
    )
    .await?;
    // Wait until the client consumed the response before Windows closes the server end.
    let _ = pipe.read_u8().await;
    Ok(())
}

async fn listen(app: &App, router: &Router) -> io::Result<()> {
    let name = pipe_name(&app.local_token);
    let mut next = create(&name, true)?;
    let mut clients = JoinSet::new();
    loop {
        tokio::select! {
            result = next.connect(), if clients.len() < 12 => {
                result?;
                let pipe = std::mem::replace(&mut next, create(&name, false)?);
                let (router, app) = (router.clone(), app.clone());
                clients.spawn(async move { let _ = timeout(TIMEOUT, session(pipe, router, app)).await; });
            }
            _ = clients.join_next(), if !clients.is_empty() => {}
        }
    }
}

pub async fn serve(app: App) {
    let router = crate::local::desktop_router(app.clone());
    loop {
        if let Err(error) = listen(&app, &router).await {
            tracing::warn!(%error, "Desktop control is unavailable");
        }
        sleep(Duration::from_secs(10)).await;
    }
}

async fn open(name: &str) -> io::Result<NamedPipeClient> {
    let name = windows::core::HSTRING::from(name);
    loop {
        // Kept in its own scope: a raw HANDLE is not Send, so it must not live across the await.
        {
            let opened = unsafe {
                CreateFileW(
                    &name,
                    0x12019b,
                    FILE_SHARE_MODE(0),
                    None,
                    OPEN_EXISTING,
                    // SECURITY_SQOS_PRESENT | SECURITY_IDENTIFICATION prevents server impersonation.
                    FILE_FLAG_OVERLAPPED | FILE_FLAGS_AND_ATTRIBUTES(0x0011_0000),
                    None,
                )
            };
            // The Agent keeps one listening instance and creates the next only after accepting a
            // client. The panel sends two requests per poll, so the second one regularly hit
            // ERROR_PIPE_BUSY and the Desktop flashed "cannot connect to Agent". Busy means the
            // Agent is alive: wait for the next instance (the caller's timeout bounds this loop).
            match opened {
                Ok(handle) => {
                    let file = unsafe { OwnedHandle::from_raw_handle(handle.0) };
                    // The returned pipe owns the handle. Convert before handing it to Tokio.
                    return unsafe { NamedPipeClient::from_raw_handle(file.into_raw_handle()) };
                }
                Err(error) if error.code() == ERROR_PIPE_BUSY.to_hresult() => {}
                Err(error) => return Err(io::Error::other(error)),
            }
        }
        sleep(Duration::from_millis(20)).await;
    }
}

pub async fn request(method: String, path: String, body: Option<Value>) -> io::Result<Reply> {
    if !allowed(&method, &path) {
        return Err(io::Error::new(io::ErrorKind::InvalidInput, "คำสั่งนี้ใช้จาก Desktop ไม่ได้"));
    }
    let token = tokio::fs::read_to_string(service_data_dir().join("local_token")).await?;
    let token = token.trim();
    if !proof::valid_local_token(token) {
        return Err(io::ErrorKind::NotConnected.into());
    }
    timeout(TIMEOUT, async {
        let mut pipe = open(&pipe_name(token)).await?;
        let client_nonce = crate::hex::random_id(16);
        write(&mut pipe, &client_nonce, 128).await?;
        let hello: Frame = read(&mut pipe, 256).await?;
        hello.verify(token, SERVER, &client_nonce)?;
        let call = Call { method, path, body };
        let call = Frame::sign(token, REQUEST, &hello.text, serde_json::to_string(&call)?);
        write(&mut pipe, &call, MAX_REQUEST).await?;
        let result: Frame = read(&mut pipe, MAX_RESPONSE).await?;
        result.verify(token, RESPONSE, &call.mac)?;
        let _ = pipe.write_u8(0).await;
        Ok(serde_json::from_str(&result.text)?)
    })
    .await?
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_non_panel_paths_and_binds_messages_to_the_connection() {
        assert!(allowed("POST", "/api/v1/local/devices/sc-a123/revoke"));
        assert!(!allowed("POST", "/api/v1/local/devices/../revoke"));
        assert!(!allowed("GET", "/api/v1/local/status?extra=1"));
        assert!(!allowed("POST", "/v1/terminal.exec"));

        let frame = Frame::sign("token", REQUEST, "connection-a", "payload".into());
        assert!(frame.verify("token", REQUEST, "connection-a").is_ok());
        assert!(frame.verify("token", REQUEST, "connection-b").is_err());
        assert!(frame.verify("token", RESPONSE, "connection-a").is_err());
    }

    #[tokio::test]
    async fn local_pipe_supports_authenticated_accounts_and_multiple_instances() {
        let name = format!("{PIPE}.test-{}", crate::hex::random_id(8));
        let mut listener = create(&name, true).unwrap();
        async {
            let mut client = open(&name).await.unwrap();
            listener.connect().await.unwrap();
            let _next = create(&name, false).unwrap();
            write(&mut client, &"test", 128).await.unwrap();
            assert_eq!(read::<String>(&mut listener, 128).await.unwrap(), "test");
            write(&mut listener, &"ready", 128).await.unwrap();
            assert_eq!(read::<String>(&mut client, 128).await.unwrap(), "ready");
        }
        .await;
    }

    #[tokio::test]
    async fn waits_for_the_next_instance_instead_of_failing_when_busy() {
        let name = format!("{PIPE}.test-{}", crate::hex::random_id(8));
        let _listener = create(&name, true).unwrap();
        let _first = open(&name).await.unwrap();
        let second = tokio::spawn({
            let name = name.clone();
            async move { open(&name).await }
        });
        sleep(Duration::from_millis(150)).await;
        assert!(
            !second.is_finished(),
            "the only instance is taken, so the second client must wait"
        );
        let _next = create(&name, false).unwrap();
        let second = timeout(Duration::from_secs(2), second).await.unwrap().unwrap();
        assert!(second.is_ok());
    }
}
