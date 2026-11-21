//! อ่านคำสั่งจาก command line: โหมด console เดิม (มีแต่ --flag) หรือคำสั่งย่อย (install, uninstall, service …)
//! ไฟล์นี้แค่แปลง argv เป็น `Command` — ตัวทำงานจริงอยู่ที่ main.rs / winsvc/

use crate::{config::Config, update::source::Source};

#[derive(Debug, Clone)]
pub struct InstallOptions {
    /// บัญชี Windows ของเจ้าของเครื่อง (อ่าน local_token เพื่อเปิด panel, เป็นเจ้าของ helper)
    pub owner: String,
    pub auto_update: bool,
    /// ว่าง = ใช้ค่าที่ฝังตอน build
    pub update_source: String,
    pub allow_screen_input: bool,
    pub start: bool,
    pub json: bool,
}

#[derive(Debug)]
pub enum Command {
    /// เปิดมือในเทอร์มินัล (Phase 0 เดิม)
    Agent(Config),
    ServiceRun,
    UpdaterRun,
    Install {
        options: InstallOptions,
        plan_only: bool,
    },
    Uninstall {
        keep_data: bool,
        json: bool,
    },
    ServiceStart,
    ServiceStop,
    ServiceStatus {
        json: bool,
    },
    /// เปิดหน้าจัดการในเบราว์เซอร์ (ไม่ต้องใช้สิทธิ์ admin ขอแค่เป็นเจ้าของเครื่อง)
    Panel {
        local_port: Option<u16>,
    },
    /// ตัวช่วยจับภาพ/ส่ง input ใน session ผู้ใช้
    Helper,
    Version,
}

pub const USAGE: &str = "การใช้งาน:
  synapta-connect [--port N] [--discovery-port N] [--local-port N] [--data-dir PATH] [--name TEXT] [--file-root PATH]
      เปิด agent ในเทอร์มินัล (ไม่ใช่ service)
  synapta-connect install [--plan] [--json] [--auto-update on|off] [--update-source SRC] [--allow-screen-input] [--owner USER] [--no-start]
      ติดตั้งเป็น Windows service (ต้องรันด้วยสิทธิ์ Administrator)
  synapta-connect uninstall [--keep-data] [--json]
  synapta-connect service start|stop|status [--json]
  synapta-connect panel        เปิดหน้าจัดการในเบราว์เซอร์
  synapta-connect --version";

pub fn parse(args: Vec<String>) -> Result<Command, String> {
    let Some(first) = args.first().map(String::as_str) else {
        return Ok(Command::Agent(Config::default()));
    };
    let rest = &args[1..];
    match first {
        "--version" | "version" => Ok(Command::Version),
        "service" => match rest.first().map(String::as_str) {
            Some("run") => Ok(Command::ServiceRun),
            Some("start") => Ok(Command::ServiceStart),
            Some("stop") => Ok(Command::ServiceStop),
            Some("status") => Ok(Command::ServiceStatus {
                json: rest.iter().any(|a| a == "--json"),
            }),
            _ => Err("service ต้องตามด้วย run | start | stop | status".into()),
        },
        "updater" if rest.first().map(String::as_str) == Some("run") => Ok(Command::UpdaterRun),
        "helper" => Ok(Command::Helper),
        "panel" => {
            let mut port = None;
            let mut it = rest.iter();
            while let Some(flag) = it.next() {
                match flag.as_str() {
                    "--local-port" => {
                        port = Some(
                            it.next()
                                .and_then(|v| v.parse().ok())
                                .ok_or("--local-port ต้องตามด้วยตัวเลข")?,
                        )
                    }
                    other => return Err(format!("ไม่รู้จักตัวเลือก {other}")),
                }
            }
            Ok(Command::Panel { local_port: port })
        }
        "uninstall" => {
            let mut keep_data = false;
            let mut json = false;
            for a in rest {
                match a.as_str() {
                    "--keep-data" => keep_data = true,
                    "--json" => json = true,
                    other => return Err(format!("ไม่รู้จักตัวเลือก {other}")),
                }
            }
            Ok(Command::Uninstall { keep_data, json })
        }
        "install" => parse_install(rest),
        flag if flag.starts_with("--") => Config::from_flags(args.clone()).map(Command::Agent),
        other => Err(format!("ไม่รู้จักคำสั่ง '{other}'")),
    }
}

fn parse_install(args: &[String]) -> Result<Command, String> {
    let mut o = InstallOptions {
        owner: crate::platform::account().unwrap_or_default(),
        auto_update: true,
        update_source: String::new(),
        allow_screen_input: false,
        start: true,
        json: false,
    };
    let mut plan_only = false;
    let mut it = args.iter();
    while let Some(flag) = it.next() {
        let mut value = |name: &str| it.next().cloned().ok_or(format!("{name} ต้องมีค่าตามหลัง"));
        match flag.as_str() {
            "--plan" => plan_only = true,
            "--json" => o.json = true,
            "--no-start" => o.start = false,
            "--allow-screen-input" => o.allow_screen_input = true,
            "--owner" => o.owner = value("--owner")?,
            "--auto-update" => {
                o.auto_update = match value("--auto-update")?.as_str() {
                    "on" => true,
                    "off" => false,
                    _ => return Err("--auto-update รับ on หรือ off".into()),
                }
            }
            "--update-source" => {
                let src = value("--update-source")?;
                Source::parse(&src)?;
                o.update_source = src;
            }
            other => return Err(format!("ไม่รู้จักตัวเลือก {other}")),
        }
    }
    if o.owner.trim().is_empty() {
        return Err("ระบุเจ้าของเครื่องด้วย --owner <ชื่อผู้ใช้ Windows> (หาบัญชีปัจจุบันไม่ได้)".into());
    }
    // ชื่อนี้ถูกต่อเป็น "<ชื่อ>:R" ส่งให้ icacls — ห้ามมีอักขระที่เปลี่ยนความหมายของ ACL (: ( ) , ฯลฯ)
    let valid_owner = o.owner.chars().count() <= 128
        && o.owner
            .chars()
            .all(|c| c.is_alphanumeric() || matches!(c, '_' | '.' | ' ' | '@' | '\\' | '-' | '$'));
    if !valid_owner {
        return Err("--owner ใช้ได้เฉพาะตัวอักษร ตัวเลข _ . - @ $ ช่องว่าง และ \\ (ไม่เกิน 128 ตัว)".into());
    }
    Ok(Command::Install { options: o, plan_only })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(args: &[&str]) -> Result<Command, String> {
        parse(args.iter().map(|s| s.to_string()).collect())
    }

    #[test]
    fn no_args_and_flags_start_the_console_agent_as_before() {
        assert!(matches!(p(&[]), Ok(Command::Agent(_))));
        match p(&["--port", "1234", "--name", "x"]) {
            Ok(Command::Agent(c)) => assert_eq!((c.port, c.name.as_str()), (1234, "x")),
            other => panic!("{other:?}"),
        }
        assert!(p(&["--bogus", "1"]).is_err());
    }

    #[test]
    fn subcommands_parse_and_reject_unknown_options() {
        assert!(matches!(p(&["service", "run"]), Ok(Command::ServiceRun)));
        assert!(matches!(
            p(&["service", "status", "--json"]),
            Ok(Command::ServiceStatus { json: true })
        ));
        assert!(matches!(p(&["updater", "run"]), Ok(Command::UpdaterRun)));
        assert!(matches!(
            p(&["uninstall", "--keep-data"]),
            Ok(Command::Uninstall {
                keep_data: true,
                json: false
            })
        ));
        assert!(matches!(
            p(&["panel", "--local-port", "9"]),
            Ok(Command::Panel { local_port: Some(9) })
        ));
        for bad in [
            &["service"][..],
            &["updater"],
            &["uninstall", "--x"],
            &["panel", "--local-port"],
            &["nope"],
        ] {
            assert!(p(bad).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn install_options_are_validated() {
        let Ok(Command::Install { options, plan_only }) = p(&[
            "install",
            "--plan",
            "--owner",
            "PC\\me",
            "--auto-update",
            "off",
            "--update-source",
            "github:a/b",
            "--allow-screen-input",
            "--no-start",
        ]) else {
            panic!("ต้อง parse ได้")
        };
        assert!(plan_only && !options.auto_update && options.allow_screen_input && !options.start);
        assert_eq!(
            (options.owner.as_str(), options.update_source.as_str()),
            ("PC\\me", "github:a/b")
        );

        assert!(p(&["install", "--owner", "me", "--auto-update", "maybe"]).is_err());
        assert!(p(&["install", "--owner", "me", "--update-source", "http://insecure/"]).is_err());
        assert!(p(&["install", "--owner"]).is_err());
        // อักขระที่เปลี่ยนความหมายของ ACL / คำสั่ง ต้องถูกปฏิเสธที่ฝั่ง CLI (ไม่เชื่อค่าที่ frontend ส่งมา)
        for bad in ["me:F", "me(OI)", "a,b", "x;y", "me'", "me\"", "me|x"] {
            assert!(p(&["install", "--owner", bad]).is_err(), "{bad}");
        }
        assert!(p(&["install", "--owner", "DESKTOP-1\\สมชาย"]).is_ok());
        assert!(p(&["install", "--owner", "me", "--update-source", "https://x/a b/"]).is_err());
    }
}
