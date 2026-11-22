//! ติดตั้ง / ถอนการติดตั้ง / ดูสถานะ — `synapta-connect install|uninstall|service status` (ต้องรันด้วยสิทธิ์ Administrator)
//!
//! สิ่งที่ติดตั้ง: โปรแกรมใน Program Files, ข้อมูลใน ProgramData (ACL จำกัด), service 2 ตัว (mod.rs), กฎ firewall
//! (เฉพาะเครือข่าย private/domain), และ helper ของ session ผู้ใช้ (Run key) — ทุกอย่างถอนได้ด้วย `uninstall`
//! ล้มกลางทาง = ถอนของที่ลงไปแล้วออกให้หมด ไม่ทิ้งเครื่องไว้ในสภาพครึ่ง ๆ กลาง ๆ
//! exit code: 0 สำเร็จ · 3 ไม่ได้รันด้วยสิทธิ์ Administrator · 4 ล้มเหลว (ถอนที่ลงไปแล้วออกแล้ว)

use super::{AGENT_ACCOUNT, AGENT_SERVICE, SystemHost, UPDATER_SERVICE, host};
use crate::{
    cli::InstallOptions,
    config::service_data_dir,
    panel_https,
    settings::SettingsStore,
    update::{
        installer::{self, EXE_NAME, Host},
        version::Version,
    },
};
use serde_json::{Value, json};
use std::{
    os::windows::process::CommandExt,
    path::{Path, PathBuf},
    process::Command,
    time::{Duration, Instant},
};
use windows_service::{
    service::{
        ServiceAccess, ServiceAction, ServiceActionType, ServiceErrorControl, ServiceFailureActions,
        ServiceFailureResetPeriod, ServiceInfo, ServiceSidType, ServiceStartType, ServiceType,
    },
    service_manager::{ServiceManager, ServiceManagerAccess},
};

pub const EXIT_OK: i32 = 0;
pub const EXIT_NOT_ADMIN: i32 = 3;
pub const EXIT_FAILED: i32 = 4;

const FIREWALL_TCP: &str = "Synapta Connect (API)";
const FIREWALL_UDP: &str = "Synapta Connect (Discovery)";
const RUN_KEY: &str = r"HKLM\SOFTWARE\Microsoft\Windows\CurrentVersion\Run";
const RUN_VALUE: &str = "SynaptaConnectHelper";
pub(super) const FIREWALL_MARKER: &str = "service-firewall-v2";
const RECOVERY_MARKER: &str = "service-recovery-v1";
const WATCHDOG_TASK: &str = "SynaptaConnect Health Monitor";

/// ที่ติดตั้งโปรแกรม — %ProgramFiles%\SynaptaConnect
pub fn install_dir() -> PathBuf {
    std::env::var_os("ProgramFiles")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(r"C:\Program Files"))
        .join("SynaptaConnect")
}

/// สิ่งที่จะทำ (ใช้กับ `install --plan` โดยไม่แก้เครื่อง)
pub fn plan(opts: &InstallOptions) -> Vec<(&'static str, String)> {
    let (dir, data) = (install_dir(), service_data_dir());
    vec![
        ("copy_program", format!("คัดลอกโปรแกรมไป {}", dir.display())),
        (
            "create_services",
            format!(
                "สร้าง service {AGENT_SERVICE} (บัญชี {AGENT_ACCOUNT} สิทธิ์ต่ำ, เริ่มเองตอนบูต) และ {UPDATER_SERVICE} (ตัวติดตั้งอัปเดต)"
            ),
        ),
        (
            "data_acl",
            format!("สร้าง {} จำกัดสิทธิ์: service, SYSTEM, Administrators", data.display()),
        ),
        (
            "settings",
            format!(
                "บันทึกค่าตั้ง: เจ้าของ={}, อัปเดตอัตโนมัติ={}, จับภาพ/input={}",
                opts.owner,
                on_off(opts.auto_update),
                on_off(opts.allow_screen_input)
            ),
        ),
        (
            "firewall",
            format!(
                "อนุญาต SynaptaConnect บน private/domain: Agent TCP {}, discovery UDP {}, panel TCP {} (LAN {})",
                opts.ports.port,
                opts.ports.discovery_port,
                opts.ports.local_port,
                on_off(opts.allow_lan_panel)
            ),
        ),
        (
            "helper_autostart",
            "ลงทะเบียน helper (เริ่มตอนผู้ใช้ล็อกอิน ทำงานเฉพาะของเจ้าของ)".into(),
        ),
        (
            "start",
            if opts.start {
                format!("เริ่ม {AGENT_SERVICE} และตรวจสุขภาพ")
            } else {
                "ไม่เริ่ม service (--no-start)".into()
            },
        ),
    ]
}

fn on_off(v: bool) -> &'static str {
    if v { "เปิด" } else { "ปิด" }
}

// ---------- รายงานผล ----------

struct Out {
    json: bool,
}

impl Out {
    fn event(&self, step: &str, status: &str, detail: &str) {
        if self.json {
            println!("{}", json!({"step": step, "status": status, "detail": detail}));
        } else {
            let mark = match status {
                "ok" => "[ผ่าน]",
                "error" => "[ล้ม ]",
                "warn" => "[เตือน]",
                _ => "[....]",
            };
            println!("{mark} {step}: {detail}");
        }
    }

    fn result(&self, ok: bool, detail: &str) {
        if self.json {
            println!(
                "{}",
                json!({"result": if ok { "ok" } else { "error" }, "detail": detail})
            );
        } else {
            println!("{detail}");
        }
    }
}

fn step<T>(out: &Out, name: &str, f: impl FnOnce() -> Result<T, String>) -> Result<T, String> {
    out.event(name, "start", "");
    f().inspect(|_| out.event(name, "ok", ""))
        .inspect_err(|e| out.event(name, "error", e))
}

// ---------- เรียกเครื่องมือของระบบ ----------

fn run_tool(program: &str, args: &[&str]) -> Result<String, String> {
    // path เต็มใน System32 เสมอ (ตัวติดตั้งรันเป็น Administrator — ห้ามเรียกผ่าน PATH)
    let path = if program == "powershell.exe" {
        crate::platform::win::system_tool(r"WindowsPowerShell\v1.0\powershell.exe")
    } else {
        crate::platform::win::system_tool(program)
    };
    let output = Command::new(path)
        .args(args)
        .creation_flags(0x0800_0000) // CREATE_NO_WINDOW
        .output()
        .map_err(|e| format!("{program}: {e}"))?;
    let text = String::from_utf8_lossy(&output.stdout).to_string();
    if output.status.success() {
        Ok(text)
    } else {
        Err(format!(
            "{program} {} → {} {}",
            args.join(" "),
            output.status,
            text.trim()
        ))
    }
}

pub(super) fn recovery_ready(data_dir: &Path) -> bool {
    data_dir.join("updater-status").join(RECOVERY_MARKER).is_file()
}

pub(super) fn configure_recovery(dir: &Path, data: &Path) -> Result<(), String> {
    let agent = host::open(AGENT_SERVICE, ServiceAccess::CHANGE_CONFIG)?;
    agent
        .update_failure_actions(ServiceFailureActions {
            reset_period: ServiceFailureResetPeriod::After(Duration::from_secs(86400)),
            reboot_msg: None,
            command: None,
            actions: Some(vec![
                ServiceAction {
                    action_type: ServiceActionType::Restart,
                    delay: Duration::from_secs(10),
                },
                ServiceAction {
                    action_type: ServiceActionType::Restart,
                    delay: Duration::from_secs(30),
                },
                ServiceAction {
                    action_type: ServiceActionType::Restart,
                    delay: Duration::from_secs(60),
                },
                ServiceAction {
                    action_type: ServiceActionType::None,
                    delay: Duration::ZERO,
                },
            ]),
        })
        .map_err(|error| error.to_string())?;
    agent
        .set_failure_actions_on_non_crash_failures(true)
        .map_err(|error| error.to_string())?;
    let command = format!("\"{}\" watchdog", dir.join(EXE_NAME).display());
    run_tool(
        "schtasks.exe",
        &[
            "/Create",
            "/SC",
            "MINUTE",
            "/MO",
            "1",
            "/RU",
            "SYSTEM",
            "/TN",
            WATCHDOG_TASK,
            "/TR",
            &command,
            "/F",
        ],
    )?;
    std::fs::write(data.join("updater-status").join(RECOVERY_MARKER), b"1").map_err(|error| error.to_string())
}

fn grant(path: &Path, grants: &[&str]) -> Result<(), String> {
    let p = path.to_string_lossy();
    let mut args = vec![&*p, "/inheritance:r", "/grant:r"];
    args.extend_from_slice(grants);
    run_tool("icacls.exe", &args).map(|_| ())
}

// ---------- ติดตั้ง ----------

pub fn install(opts: &InstallOptions) -> i32 {
    let out = Out { json: opts.json };
    if !is_elevated::is_elevated() {
        out.result(false, "ต้องรันด้วยสิทธิ์ Administrator (คลิกขวา > Run as administrator)");
        return EXIT_NOT_ADMIN;
    }
    let dir = install_dir();
    // Rollback owns only the installation that passed preflight and created this directory.
    if let Err(e) = step(&out, "preflight", || {
        for name in [AGENT_SERVICE, UPDATER_SERVICE] {
            if host::service_exists(name)? {
                return Err(format!("มี service {name} อยู่แล้ว — ใช้ตัวอัปเดตเพื่อเปลี่ยนรุ่น"));
            }
        }
        std::fs::create_dir(&dir).map_err(|e| format!("สร้าง {} ไม่ได้: {e}", dir.display()))
    }) {
        out.result(false, &format!("ติดตั้งไม่สำเร็จ: {e}"));
        return EXIT_FAILED;
    }
    let keep_data_on_failure = service_data_dir().exists();
    match do_install(opts, &out, &dir) {
        Ok(msg) => {
            out.result(true, &msg);
            EXIT_OK
        }
        Err(e) => {
            out.event("rollback", "start", "ถอนสิ่งที่ลงไปแล้ว");
            uninstall_steps(&out, keep_data_on_failure);
            out.result(false, &format!("ติดตั้งไม่สำเร็จ: {e}"));
            EXIT_FAILED
        }
    }
}

fn do_install(opts: &InstallOptions, out: &Out, dir: &Path) -> Result<String, String> {
    let data = service_data_dir();
    let exe = dir.join(EXE_NAME);
    let work = data.join("updater");

    step(out, "copy_program", || {
        let me = std::env::current_exe().map_err(|e| e.to_string())?;
        std::fs::copy(&me, &exe).map_err(|e| format!("คัดลอก {} ไม่ได้: {e}", me.display()))?;
        Ok(())
    })?;

    step(out, "create_services", || create_services(&exe))?;

    step(out, "data_acl", || {
        std::fs::create_dir_all(&work).map_err(|e| e.to_string())?;
        // ข้อมูลของ agent: บัญชี service เขียนได้ (สืบทอดลงไฟล์ข้างใน) ไม่มีผู้ใช้ทั่วไป
        // Allow creation at the root and modification of ordinary children. The root
        // itself and the separately protected updater directories must stay in place.
        let agent_root = format!("{AGENT_ACCOUNT}:(RD,WD,AD,REA,WEA,X,RA,WA,RC,S)");
        let agent_children = format!("{AGENT_ACCOUNT}:(OI)(CI)(IO)M");
        grant(
            &data,
            &[
                &agent_root,
                &agent_children,
                "*S-1-5-18:(OI)(CI)F",
                "*S-1-5-32-544:(OI)(CI)F",
            ],
        )?;
        // ผลของตัวติดตั้ง: agent อ่านได้อย่างเดียว — ตัวติดตั้ง (SYSTEM) เขียนเฉพาะที่นี่/ที่ของตัวเอง ไม่เขียนในที่ที่ agent คุม
        let status = data.join("updater-status");
        std::fs::create_dir_all(&status).map_err(|e| e.to_string())?;
        let agent_read = format!("{AGENT_ACCOUNT}:(OI)(CI)RX");
        grant(
            &status,
            &[&agent_read, "*S-1-5-18:(OI)(CI)F", "*S-1-5-32-544:(OI)(CI)F"],
        )?;
        // พื้นที่ตัวติดตั้ง (journal, สำรอง, สำเนาที่ตรวจแล้ว): agent เข้าไม่ได้ — ไม่ให้ agent ที่ถูกยึดแก้ของที่ใช้ตัดสินความปลอดภัย
        grant(&work, &["*S-1-5-18:(OI)(CI)F", "*S-1-5-32-544:(OI)(CI)F"])
    })?;

    step(out, "token_file", || {
        // สร้าง local_token ว่างพร้อม ACL ไว้ก่อน (service สิทธิ์ต่ำตั้ง ACL เองไม่ได้ — มันแค่เติมเนื้อหา)
        // ชื่อเจ้าของผิด = ล้มตรงนี้พร้อมบอกสาเหตุ ไม่ใช่รู้ตอน panel เปิดไม่ได้
        let token = data.join("local_token");
        std::fs::write(&token, b"").map_err(|e| e.to_string())?;
        let agent_full = format!("{AGENT_ACCOUNT}:F");
        let owner_read = format!("{}:R", opts.owner);
        grant(&token, &[&agent_full, "*S-1-5-18:F", "*S-1-5-32-544:F", &owner_read]).map_err(|e| {
            format!(
                "ให้เจ้าของ '{}' อ่าน local_token ไม่ได้ (ชื่อบัญชีถูกต้องไหม? ใช้ --owner ชื่อเครื่อง\\ผู้ใช้): {e}",
                opts.owner
            )
        })?;
        let port_file = data.join("panel-port");
        std::fs::write(&port_file, opts.ports.local_port.to_string()).map_err(|e| e.to_string())?;
        grant(
            &port_file,
            &[&agent_full, "*S-1-5-18:F", "*S-1-5-32-544:F", &owner_read],
        )
    })?;

    step(out, "settings", || {
        if let Some((cert, key)) = &opts.panel_tls {
            panel_https::Server::from_files(cert, key).map_err(|e| format!("ใบรับรอง panel: {e:#}"))?;
            for (source, name) in [(cert, panel_https::CERT_FILE), (key, panel_https::KEY_FILE)] {
                let destination = data.join(name);
                let bytes = std::fs::read(source).map_err(|e| e.to_string())?;
                crate::fsutil::atomic_write(&destination, &bytes).map_err(|e| e.to_string())?;
            }
        }
        installer::write_install_info(&layout_for(dir, &data, &work), &Version::current().to_string())
            .map_err(|e| e.to_string())?;
        SettingsStore::load(&data)
            .update(|s| {
                s.owner = Some(opts.owner.clone());
                s.update.auto_update = opts.auto_update;
                s.update.source = opts.update_source.clone();
                s.update.web_source = opts.web_update_source.clone();
                s.screen_input.enabled = opts.allow_screen_input;
                s.network.ports = Some(opts.ports);
                s.network.lan_enabled = opts.allow_lan_panel;
            })
            .map(|_| ())
            .map_err(|e| e.to_string())
    })?;

    step(out, "firewall", || configure_firewall(&layout_for(dir, &data, &work)))?;
    step(out, "recovery", || configure_recovery(dir, &data))?;

    step(out, "helper_autostart", || {
        let command = format!("\"{}\" helper", exe.display());
        run_tool(
            "reg.exe",
            &["add", RUN_KEY, "/v", RUN_VALUE, "/t", "REG_SZ", "/d", &command, "/f"],
        )
        .map(|_| ())
    })?;

    if !opts.start {
        return Ok("ติดตั้งเสร็จ (ยังไม่ได้เริ่ม service — เริ่มด้วย `synapta-connect service start`)".into());
    }
    step(out, "start", || {
        host::start_service(AGENT_SERVICE)?;
        wait_healthy(Duration::from_secs(40)).map(|_| ())
    })?;

    // ตรวจว่าเจ้าของอ่าน local_token ได้จริง — ไม่ได้ = เปิด panel ไม่ได้ (เตือน ไม่ถึงกับล้มการติดตั้ง)
    let acl = run_tool("icacls.exe", &[&data.join("local_token").to_string_lossy()]).unwrap_or_default();
    let owner_name = opts.owner.rsplit('\\').next().unwrap_or(&opts.owner).to_lowercase();
    if !acl.to_lowercase().contains(&owner_name) {
        out.event(
            "owner_token_access",
            "warn",
            &format!("บัญชี '{}' ยังไม่มีสิทธิ์อ่าน local_token — ตรวจชื่อบัญชีเจ้าของ", opts.owner),
        );
    }
    Ok(format!(
        "ติดตั้งเสร็จ: service {AGENT_SERVICE} ทำงานอยู่ — เปิดหน้าจัดการด้วย `synapta-connect panel`"
    ))
}

fn layout_for(install: &Path, data: &Path, work: &Path) -> installer::Layout {
    installer::Layout {
        install_dir: install.into(),
        data_dir: data.into(),
        work_dir: work.into(),
    }
}

pub(super) fn configure_firewall(layout: &installer::Layout) -> Result<(), String> {
    let program = format!("program={}", layout.exe().display());
    let service = format!("service={AGENT_SERVICE}");
    for (name, proto) in [(FIREWALL_TCP, "TCP"), (FIREWALL_UDP, "UDP")] {
        let name = format!("name={name}");
        let protocol = format!("protocol={proto}");
        let options = [&*service, "localport=any", "profile=private,domain", "action=allow"];
        let mut change = vec![
            "advfirewall",
            "firewall",
            "set",
            "rule",
            &*name,
            "dir=in",
            &*program,
            "new",
        ];
        change.extend(options);
        if run_tool("netsh.exe", &change).is_err() {
            let mut add = vec![
                "advfirewall",
                "firewall",
                "add",
                "rule",
                &*name,
                "dir=in",
                &*program,
                &*protocol,
            ];
            add.extend(options);
            run_tool("netsh.exe", &add)?;
        }
    }
    crate::fsutil::atomic_write(&layout.status_dir().join(FIREWALL_MARKER), b"2").map_err(|e| e.to_string())
}

fn wait_healthy(timeout: Duration) -> Result<Value, String> {
    let deadline = Instant::now() + timeout;
    loop {
        match SystemHost.health() {
            Ok(h) => return Ok(json!(h)),
            Err(e) if Instant::now() >= deadline => {
                return Err(format!("service ไม่ตอบ health ภายใน {} วินาที: {e}", timeout.as_secs()));
            }
            Err(_) => std::thread::sleep(Duration::from_secs(1)),
        }
    }
}

fn create_services(exe: &Path) -> Result<(), String> {
    let manager = ServiceManager::local_computer(
        None::<&str>,
        ServiceManagerAccess::CONNECT | ServiceManagerAccess::CREATE_SERVICE,
    )
    .map_err(|e| format!("เชื่อม SCM ไม่ได้: {e}"))?;
    let info = |name: &str, display: &str, args: &[&str], account: Option<&str>| ServiceInfo {
        name: name.into(),
        display_name: display.into(),
        service_type: ServiceType::OWN_PROCESS,
        start_type: ServiceStartType::AutoStart,
        error_control: ServiceErrorControl::Normal,
        executable_path: exe.into(),
        launch_arguments: args.iter().map(Into::into).collect(),
        dependencies: vec![],
        account_name: account.map(Into::into),
        account_password: None,
    };
    let access = ServiceAccess::CHANGE_CONFIG
        | ServiceAccess::QUERY_STATUS
        | ServiceAccess::QUERY_CONFIG
        | ServiceAccess::START
        | ServiceAccess::STOP
        | ServiceAccess::DELETE;
    let e = |what: &'static str| move |err: windows_service::Error| format!("{what}: {err}");

    let agent = manager
        .create_service(
            &info(
                AGENT_SERVICE,
                "Synapta Connect Agent",
                &["service", "run"],
                Some(AGENT_ACCOUNT),
            ),
            access,
        )
        .map_err(e("สร้าง service agent"))?;
    agent
        .set_description("ให้ Synapta Code เชื่อมต่อ/ควบคุมเครื่องนี้ผ่านการจับคู่และ mTLS (สิทธิ์ต่ำ) — ปิดได้จาก services.msc หรือ synapta-connect uninstall")
        .map_err(e("ตั้งคำอธิบาย"))?;
    agent
        .set_config_service_sid_info(ServiceSidType::Unrestricted)
        .map_err(e("ตั้ง SID ของ service"))?;

    let updater = manager
        .create_service(
            &info(UPDATER_SERVICE, "Synapta Connect Updater", &["updater", "run"], None),
            access,
        )
        .map_err(e("สร้าง service ตัวติดตั้ง"))?;
    updater
        .set_description("เปลี่ยนรุ่น Synapta Connect (ตรวจลายเซ็น สำรอง กู้คืนเมื่อล้ม) — ทำงานตอนบูตและเมื่อ agent เรียก แล้วจบเอง")
        .map_err(e("ตั้งคำอธิบาย"))?;

    // ให้บัญชีของ agent "เริ่ม" ตัวติดตั้งได้ (และดูสถานะ) เท่านั้น — ไม่ให้หยุด/แก้ config
    let sid = run_tool("sc.exe", &["showsid", AGENT_SERVICE])?
        .split_whitespace()
        .find(|t| t.starts_with("S-1-5-80-"))
        .map(str::to_string)
        .ok_or("อ่าน SID ของ service agent ไม่ได้")?;
    let sddl = format!("D:(A;;CCLCSWRPWPDTLOCRRC;;;SY)(A;;CCDCLCSWRPWPDTLOCRSDRCWDWO;;;BA)(A;;RPLCRC;;;{sid})");
    run_tool("sc.exe", &["sdset", UPDATER_SERVICE, &sddl]).map(|_| ())
}

// ---------- ถอนการติดตั้ง ----------

pub fn uninstall(keep_data: bool, json: bool) -> i32 {
    let out = Out { json };
    if !is_elevated::is_elevated() {
        out.result(false, "ต้องรันด้วยสิทธิ์ Administrator (คลิกขวา > Run as administrator)");
        return EXIT_NOT_ADMIN;
    }
    let failures = uninstall_steps(&out, keep_data);
    if failures == 0 {
        out.result(true, "ถอนการติดตั้งเสร็จ");
        EXIT_OK
    } else {
        out.result(
            false,
            &format!("ถอนไม่ครบ {failures} ขั้น (ดูรายการด้านบน) — สั่ง uninstall ซ้ำได้"),
        );
        EXIT_FAILED
    }
}

/// ทำทุกขั้นแม้บางขั้นล้ม (ถอนครึ่งทางแย่กว่าถอนไม่ครบแล้วบอก) — คืนจำนวนขั้นที่ล้ม; ขั้นที่ "ไม่มีอะไรให้ถอน" ไม่นับว่าล้ม
fn uninstall_steps(out: &Out, keep_data: bool) -> usize {
    let (dir, data) = (install_dir(), service_data_dir());
    let exe = dir.join(EXE_NAME);
    let mut failures = 0;
    let mut run = |name: &str, f: &mut dyn FnMut() -> Result<(), String>| {
        if step(out, name, f).is_err() {
            failures += 1;
        }
    };

    run("stop_services", &mut || {
        for name in [UPDATER_SERVICE, AGENT_SERVICE] {
            if host::open(name, ServiceAccess::QUERY_STATUS).is_ok() {
                host::stop_service(name)?;
            }
        }
        Ok(())
    });
    run("stop_clients", &mut || stop_clients(&exe));
    run("watchdog", &mut || {
        if run_tool("schtasks.exe", &["/Query", "/TN", WATCHDOG_TASK]).is_ok() {
            run_tool("schtasks.exe", &["/Delete", "/TN", WATCHDOG_TASK, "/F"])?;
        }
        Ok(())
    });
    run("delete_services", &mut || {
        for name in [AGENT_SERVICE, UPDATER_SERVICE] {
            if let Ok(service) = host::open(name, ServiceAccess::DELETE) {
                service.delete().map_err(|e| format!("ลบ {name} ไม่ได้: {e}"))?;
            }
        }
        Ok(())
    });
    run("firewall", &mut || {
        // ไม่มีกฎ = netsh คืน error — ไม่ใช่ความล้มเหลวของการถอน
        for name in [FIREWALL_TCP, FIREWALL_UDP] {
            let _ = run_tool(
                "netsh.exe",
                &["advfirewall", "firewall", "delete", "rule", &format!("name={name}")],
            );
        }
        Ok(())
    });
    run("helper_autostart", &mut || {
        let _ = run_tool("reg.exe", &["delete", RUN_KEY, "/v", RUN_VALUE, "/f"]);
        Ok(())
    });
    run("shortcuts", &mut crate::shortcut::remove_for_all_users);
    run("data", &mut || {
        if keep_data {
            // เก็บอุปกรณ์ที่จับคู่/กุญแจ host ไว้ใช้ตอนติดตั้งใหม่ — ลบเฉพาะความลับชั่วคราวและงานค้าง
            for name in ["local_token", "panel_sessions.json", "panel-port"] {
                let _ = std::fs::remove_file(data.join(name));
            }
            for name in ["staging", "updater", "updater-status"] {
                let _ = std::fs::remove_dir_all(data.join(name));
            }
            return Ok(());
        }
        match std::fs::remove_dir_all(&data) {
            Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(format!("ลบ {} ไม่ได้: {e}", data.display())),
            _ => Ok(()),
        }
    });
    run("program_files", &mut || remove_program_dir(&dir));
    failures
}

/// ปิด helper และ Desktop ที่ใช้ exe ที่กำลังจะถูกแทนที่หรือถอนการติดตั้ง
fn stop_clients(exe: &Path) -> Result<(), String> {
    let script = format!(
        "$ErrorActionPreference = 'Stop'; Get-CimInstance Win32_Process -Filter \"Name='{EXE_NAME}' OR Name LIKE 'synapta-connect.exe.%'\" | Where-Object {{ ($_.ExecutablePath -eq '{0}' -or $_.ExecutablePath -like '{0}.*') -and $_.ProcessId -ne {1} -and $_.CommandLine -match '\\s(helper|desktop|panel\\s+--gui)\\s*$' }} | ForEach-Object {{ Stop-Process -Id $_.ProcessId -Force }}",
        exe.display().to_string().replace('\'', "''"),
        std::process::id(),
    );
    run_tool(
        "powershell.exe",
        &["-NoProfile", "-NonInteractive", "-Command", &script],
    )
    .map(|_| ())
}

// ---------- อัปเกรดทับ ----------

/// `upgrade`: ใช้ exe ตัวที่รันอยู่นี้แทนตัวที่ติดตั้งไว้ โดยคง service, ข้อมูล, การจับคู่ และค่าตั้งทั้งหมดไว้
///
/// เป็นทางของตัวติดตั้ง/ผู้ดูแลเมื่อยังไม่มีแหล่งอัปเดตที่เซ็นไว้ (ตัวอัปเดตอัตโนมัติยังเป็นทางหลัก)
/// รุ่นใหม่ไม่ขึ้นภายใน 40 วินาที = ใส่ exe เดิมคืนแล้วเริ่ม service ใหม่ ไม่ทิ้งเครื่องไว้โดยไม่มี agent
pub fn upgrade(json: bool) -> i32 {
    let out = Out { json };
    if !is_elevated::is_elevated() {
        out.result(false, "ต้องรันด้วยสิทธิ์ Administrator (คลิกขวา > Run as administrator)");
        return EXIT_NOT_ADMIN;
    }
    let (dir, data) = (install_dir(), service_data_dir());
    let exe = dir.join(EXE_NAME);
    let backup = dir.join(format!("synapta-connect.previous-{}.exe", crate::hex::random_id(8)));
    let layout = layout_for(&dir, &data, &data.join("updater"));
    let current = Version::current();

    let preflight = step(&out, "preflight", || {
        if !host::service_exists(AGENT_SERVICE)? || !exe.is_file() {
            return Err("ยังไม่ได้ติดตั้งบนเครื่องนี้ — ใช้ install".into());
        }
        host::wait_until_stopped(UPDATER_SERVICE, Duration::from_secs(90))?;
        let me = std::env::current_exe().map_err(|e| e.to_string())?;
        if same_file(&me, &exe) {
            return Err("กำลังรันจากตัวที่ติดตั้งอยู่ — เรียก upgrade จากไฟล์รุ่นใหม่ที่ดาวน์โหลดมา".into());
        }
        let installed = SystemHost
            .health()
            .ok()
            .map(|h| h.version)
            .or_else(|| installer::installed_version(&layout))
            .and_then(|v| Version::parse(&v))
            .ok_or("ไม่ทราบรุ่นที่ติดตั้งอยู่")?;
        if installed > current {
            return Err(format!("เครื่องนี้มีรุ่นที่ใหม่กว่า ({installed}) อยู่แล้ว"));
        }
        Ok((me, installed))
    });
    let (me, installed) = match preflight {
        Ok(paths) => paths,
        Err(e) => {
            out.result(false, &format!("อัปเกรดไม่สำเร็จ: {e}"));
            return EXIT_FAILED;
        }
    };

    let timing = installer::Timing::default();
    let swapped = step(&out, "stop_services", || host::stop_service(AGENT_SERVICE))
        .and_then(|_| step(&out, "stop_clients", || stop_clients(&exe)))
        .and_then(|_| {
            step(&out, "backup", || {
                std::fs::copy(&exe, &backup)
                    .map(|_| ())
                    .map_err(|e| format!("สำรอง {} ไม่ได้: {e}", exe.display()))
            })
        })
        .and_then(|_| {
            step(&out, "archive_recovery", || {
                installer::clear_manual_recovery(&layout).map_err(|e| e.to_string())
            })
        })
        .and_then(|_| {
            step(&out, "copy_program", || {
                installer::replace_manually(&layout, &me).map_err(|e| e.to_string())
            })
        });
    if let Err(e) = swapped {
        if !exe.is_file() && backup.is_file() {
            let _ = installer::replace_manually(&layout, &backup);
        }
        let _ = host::start_service(AGENT_SERVICE);
        out.result(false, &format!("อัปเกรดไม่สำเร็จ ใช้รุ่นเดิมต่อ: {e}"));
        return EXIT_FAILED;
    }

    let started = step(&out, "start", || {
        installer::write_install_info(&layout, &current.to_string()).map_err(|e| e.to_string())?;
        host::start_service(AGENT_SERVICE)?;
        installer::wait_healthy(&SystemHost, &timing, &current.to_string(), true).map_err(|e| e.message)
    });
    if let Err(e) = started {
        out.event("rollback", "start", "ใส่รุ่นเดิมคืน");
        let restored = host::stop_service(AGENT_SERVICE)
            .and_then(|_| installer::replace_manually(&layout, &backup).map_err(|e| e.to_string()))
            .and_then(|_| installer::write_install_info(&layout, &installed.to_string()).map_err(|e| e.to_string()))
            .and_then(|_| host::start_service(AGENT_SERVICE))
            .and_then(|_| {
                installer::wait_healthy(&SystemHost, &timing, &installed.to_string(), false).map_err(|e| e.message)
            });
        let detail = match restored {
            Ok(()) => format!("อัปเกรดไม่สำเร็จ กลับไปใช้รุ่นเดิมแล้ว: {e}"),
            Err(r) => format!(
                "อัปเกรดไม่สำเร็จ และใส่รุ่นเดิมคืนไม่ได้ ({r}) — ไฟล์เดิมอยู่ที่ {}: {e}",
                backup.display()
            ),
        };
        out.result(false, &detail);
        return EXIT_FAILED;
    }
    if let Err(e) = installer::record_manual_upgrade(&layout, installed.to_string(), current.to_string()) {
        out.event("update_status", "warn", &e.to_string());
    }
    let _ = std::fs::remove_file(&backup);
    out.result(true, &format!("อัปเกรดเป็น {current} เสร็จ — การจับคู่และค่าตั้งเดิมยังอยู่"));
    EXIT_OK
}

fn same_file(a: &Path, b: &Path) -> bool {
    match (a.canonicalize(), b.canonicalize()) {
        (Ok(a), Ok(b)) => a == b,
        _ => false,
    }
}

/// ลบโฟลเดอร์โปรแกรม; ถ้ากำลังรันจากในโฟลเดอร์นั้นเอง (ลบ exe ตัวเองไม่ได้) ให้ cmd ลบให้หลังโปรแกรมนี้ปิด
fn remove_program_dir(dir: &Path) -> Result<(), String> {
    if !dir.exists() {
        return Ok(());
    }
    let running_inside = std::env::current_exe().is_ok_and(|me| me.starts_with(dir));
    if !running_inside {
        return std::fs::remove_dir_all(dir).map_err(|e| format!("ลบ {} ไม่ได้: {e}", dir.display()));
    }
    let command = format!("ping 127.0.0.1 -n 4 >nul & rmdir /s /q \"{}\"", dir.display());
    Command::new(crate::platform::win::system_tool("cmd.exe"))
        .args(["/C", &command])
        .creation_flags(0x0800_0000 | 0x0000_0008) // CREATE_NO_WINDOW | DETACHED_PROCESS
        .spawn()
        .map(|_| ())
        .map_err(|e| format!("ตั้งเวลาลบ {} ไม่ได้: {e}", dir.display()))
}

// ---------- สถานะ ----------

/// `service status`: ไม่ต้องใช้สิทธิ์ admin (อ่านอย่างเดียว) — คืน exit 0 ถ้า agent ทำงานอยู่
pub fn status(json: bool) -> i32 {
    let state_of = |name: &str| -> Value {
        match host::open(name, ServiceAccess::QUERY_STATUS | ServiceAccess::QUERY_CONFIG) {
            Err(_) => json!({"installed": false}),
            Ok(s) => json!({
                "installed": true,
                "state": s.query_status().map(|st| format!("{:?}", st.current_state)).unwrap_or_else(|e| e.to_string()),
                "start_type": s.query_config().map(|c| format!("{:?}", c.start_type)).ok(),
                "account": s.query_config().ok().and_then(|c| c.account_name.map(|a| a.to_string_lossy().into_owned())),
            }),
        }
    };
    let (dir, data) = (install_dir(), service_data_dir());
    let layout = layout_for(&dir, &data, &data.join("updater"));
    let health = SystemHost.health();
    let report = json!({
        "agent": state_of(AGENT_SERVICE),
        "updater": state_of(UPDATER_SERVICE),
        "installed_version": installer::installed_version(&layout),
        "health": health.as_ref().ok(),
        "health_error": health.as_ref().err(),
    });
    if json {
        println!("{report}");
    } else {
        println!("{}", serde_json::to_string_pretty(&report).unwrap_or_default());
    }
    if report["health"].is_null() { 1 } else { 0 }
}
