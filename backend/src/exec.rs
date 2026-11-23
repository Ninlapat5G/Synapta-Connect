//! รันคำสั่งแบบ single-shot: ส่งคืน stdout / stderr / exit code

use crate::rpc::{Res, RpcError};
use crate::{config::Mode, kill_tree::TreeGuard};
use serde::{Deserialize, Serialize};
use std::{collections::HashMap, process::Stdio, time::Duration};
use tokio::{io::AsyncReadExt, process::Command};

const DEFAULT_TIMEOUT_MS: u64 = 60_000;
const MAX_TIMEOUT_MS: u64 = 600_000;
/// ผลลัพธ์ต่อ stream ที่เก็บ — เกินนี้ตัดทิ้งและบอกใน `truncated`
const MAX_OUTPUT: usize = 1024 * 1024;

#[derive(Deserialize)]
pub struct Params {
    pub command: String,
    /// powershell (โดยปริยายบน Windows) | cmd | sh (โดยปริยายบน Unix) | bash
    pub shell: Option<String>,
    pub cwd: Option<String>,
    pub timeout_ms: Option<u64>,
    pub env: Option<HashMap<String, String>>,
    /// ข้อความที่จะส่งเข้า stdin ของคำสั่ง
    pub stdin: Option<String>,
}

#[derive(Serialize)]
pub struct Output {
    stdout: String,
    stderr: String,
    /// None = ถูกฆ่าด้วย timeout หรือ signal
    exit_code: Option<i32>,
    timed_out: bool,
    truncated: bool,
    duration_ms: u128,
}

/// (โปรแกรม, อาร์กิวเมนต์) ที่จะรันคำสั่งใน shell ที่เลือก
///
/// กับดัก: บน Windows stdout ของ cmd/PowerShell ที่ต่อท่ออยู่ใช้ codepage ของเครื่อง (เช่น 874 บนเครื่องไทย)
/// ไม่ใช่ UTF-8 ทำให้ภาษาไทยเพี้ยน จึงบังคับ UTF-8 นำหน้าคำสั่งเสมอ
fn shell_invocation(shell: Option<&str>, command: &str) -> Res<(String, Vec<String>)> {
    let s = |v: &[&str]| v.iter().map(|x| x.to_string()).collect::<Vec<_>>();
    Ok(
        match (
            shell.unwrap_or(if cfg!(windows) { "powershell" } else { "sh" }),
            cfg!(windows),
        ) {
            ("powershell", true) => (
                "powershell.exe".into(),
                [
                    s(&["-NoProfile", "-NonInteractive", "-Command"]),
                    vec![format!("[Console]::OutputEncoding=[Text.Encoding]::UTF8; {command}")],
                ]
                .concat(),
            ),
            ("cmd", true) => (
                "cmd.exe".into(),
                [s(&["/D", "/C"]), vec![format!("chcp 65001>nul & {command}")]].concat(),
            ),
            ("sh", false) => ("sh".into(), [s(&["-c"]), vec![command.to_string()]].concat()),
            ("bash", _) => ("bash".into(), [s(&["-c"]), vec![command.to_string()]].concat()),
            (other, _) => return Err(RpcError::invalid(format!("shell '{other}' ใช้ไม่ได้บน OS นี้"))),
        },
    )
}

/// อ่าน stream จนจบ เก็บแค่ MAX_OUTPUT ไบต์แรก แต่ยังอ่านต่อเพื่อไม่ให้โปรเซสค้างเพราะท่อเต็ม
async fn collect(mut stream: impl tokio::io::AsyncRead + Unpin) -> (Vec<u8>, bool) {
    let (mut kept, mut truncated, mut buf) = (Vec::new(), false, [0u8; 16 * 1024]);
    while let Ok(n) = stream.read(&mut buf).await {
        if n == 0 {
            break;
        }
        let room = MAX_OUTPUT.saturating_sub(kept.len());
        truncated |= n > room;
        kept.extend_from_slice(&buf[..n.min(room)]);
    }
    (kept, truncated)
}

pub async fn run(p: Params, mode: Mode) -> Res<Output> {
    let (program, args) = shell_invocation(p.shell.as_deref(), &p.command)?;
    tracing::info!(command = %p.command.chars().take(200).collect::<String>(), "exec.run");

    #[cfg(windows)]
    let mut cmd = Command::new(crate::platform::win::terminal_program(&program));
    #[cfg(not(windows))]
    let mut cmd = {
        let _ = mode;
        Command::new(program)
    };
    cmd.args(args)
        .stdin(if p.stdin.is_some() {
            Stdio::piped()
        } else {
            Stdio::null()
        })
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    if let Some(cwd) = &p.cwd {
        cmd.current_dir(cwd);
    }
    #[cfg(windows)]
    if let Some(path) = crate::platform::win::terminal_path(mode) {
        cmd.env("Path", path);
    }
    if let Some(env) = &p.env {
        cmd.envs(env);
    }
    #[cfg(windows)]
    cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW: ไม่ให้หน้าต่าง console โผล่บนจอเครื่องปลายทาง
    #[cfg(unix)]
    cmd.process_group(0); // กลุ่มของตัวเอง เพื่อให้ kill_tree ฆ่าทั้งต้นไม้ได้

    let started = std::time::Instant::now();
    let mut child = cmd
        .spawn()
        .map_err(|e| RpcError::new(crate::rpc::INTERNAL, format!("เริ่มคำสั่งไม่ได้: {e}")))?;
    // ถูกทิ้งเมื่อ timeout หรือเมื่อ connection หลุดกลางทาง (future ถูกยกเลิก) = ฆ่าทั้งต้นไม้ ไม่ใช่แค่ shell
    let tree = TreeGuard::attach(&child);
    if let (Some(text), Some(mut stdin)) = (p.stdin, child.stdin.take()) {
        tokio::spawn(async move {
            let _ = tokio::io::AsyncWriteExt::write_all(&mut stdin, text.as_bytes()).await;
        });
    }
    let out = tokio::spawn(collect(child.stdout.take().unwrap()));
    let err = tokio::spawn(collect(child.stderr.take().unwrap()));

    let limit = Duration::from_millis(p.timeout_ms.unwrap_or(DEFAULT_TIMEOUT_MS).min(MAX_TIMEOUT_MS));
    let (exit_code, timed_out) = match tokio::time::timeout(limit, child.wait()).await {
        Ok(status) => {
            tree.release(); // จบเองตามปกติ: ไม่ฆ่าโปรแกรมที่คำสั่งตั้งใจเปิดทิ้งไว้
            (status?.code(), false)
        }
        Err(_) => {
            drop(tree);
            let _ = child.kill().await; // ฆ่าซ้ำเผื่อผูก job ไม่สำเร็จ และเก็บกวาดสถานะของ shell
            (None, true)
        }
    };
    // โปรเซสลูกที่ยังถือท่ออยู่ทำให้ reader ไม่จบ — รอแค่ 1 วินาทีแล้วเอาเท่าที่อ่านได้
    let grace = Duration::from_secs(1);
    let (stdout, t1) = tokio::time::timeout(grace, out)
        .await
        .ok()
        .and_then(Result::ok)
        .unwrap_or_default();
    let (stderr, t2) = tokio::time::timeout(grace, err)
        .await
        .ok()
        .and_then(Result::ok)
        .unwrap_or_default();
    Ok(Output {
        stdout: String::from_utf8_lossy(&stdout).into_owned(),
        stderr: String::from_utf8_lossy(&stderr).into_owned(),
        exit_code,
        timed_out,
        truncated: t1 || t2,
        duration_ms: started.elapsed().as_millis(),
    })
}

#[cfg(all(test, windows))]
mod tests {
    use super::*;

    #[tokio::test]
    async fn windows_shells_use_the_current_machine_path_in_service_mode() {
        let expected = crate::platform::win::terminal_path(Mode::Service).expect("Windows machine PATH");
        for (shell, command) in [("powershell", "$env:Path"), ("cmd", "set PATH")] {
            let params = |env| Params {
                command: command.into(),
                shell: Some(shell.into()),
                cwd: None,
                timeout_ms: None,
                env,
                stdin: None,
            };
            let read_path = |result: Output| -> String {
                assert_eq!(result.exit_code, Some(0), "{shell}: {}", result.stderr);
                let output = result.stdout.trim_end_matches(['\r', '\n']);
                if shell == "cmd" {
                    output
                        .lines()
                        .filter_map(|line| line.split_once('='))
                        .find_map(|(key, value)| key.eq_ignore_ascii_case("PATH").then_some(value))
                        .expect("cmd PATH")
                        .into()
                } else {
                    output.into()
                }
            };
            assert_eq!(
                read_path(run(params(None), Mode::Service).await.unwrap()),
                expected.to_string_lossy()
            );
            let override_path = r"C:\Synapta Test\request-path";
            let env = HashMap::from([("PATH".into(), override_path.into())]);
            assert_eq!(
                read_path(run(params(Some(env)), Mode::Service).await.unwrap()),
                override_path
            );
        }
    }
}
