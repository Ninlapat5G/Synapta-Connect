//! จำลองเมาส์/คีย์บอร์ด — พิกัดเป็น pixel ของ virtual desktop (ดู screen.rs เรื่องการแปลงจากภาพ)

use crate::rpc::{Res, RpcError, parse};
use enigo::{Axis, Button, Coordinate, Direction, Enigo, Key, Keyboard, Mouse, Settings};
use serde::Deserialize;
use serde_json::{Value, json};
use std::{sync::Mutex, time::Duration};

/// ให้ทีละคำสั่ง — ไม่งั้นข้อความ/คลิกจากสอง session สลับกันกลางทาง
static INPUT_LOCK: Mutex<()> = Mutex::new(());

#[derive(Deserialize)]
struct MoveParams {
    x: i32,
    y: i32,
}

#[derive(Deserialize)]
struct ClickParams {
    /// ไม่ใส่ = คลิกตรงตำแหน่งเมาส์ปัจจุบัน
    x: Option<i32>,
    y: Option<i32>,
    /// left (โดยปริยาย) | right | middle
    button: Option<String>,
    #[serde(default)]
    double: bool,
}

#[derive(Deserialize)]
struct ScrollParams {
    /// บวก = เลื่อนลง/ขวา
    #[serde(default)]
    dy: i32,
    #[serde(default)]
    dx: i32,
}

#[derive(Deserialize)]
struct TypeParams {
    text: String,
}

#[derive(Deserialize)]
struct KeyParams {
    /// enter tab escape backspace delete space up down left right home end pageup pagedown f1-f12 หรืออักขระตัวเดียว
    key: String,
    /// ctrl alt shift meta (กดค้างระหว่างกดปุ่ม)
    #[serde(default)]
    modifiers: Vec<String>,
}

pub async fn rpc(method: &str, params: Value) -> Res<Value> {
    match method {
        "input.mouse_move" => {
            let p: MoveParams = parse(params)?;
            run(move |e| Ok(e.move_mouse(p.x, p.y, Coordinate::Abs)?)).await
        }
        "input.click" => {
            let p: ClickParams = parse(params)?;
            let button = match p.button.as_deref().unwrap_or("left") {
                "left" => Button::Left,
                "right" => Button::Right,
                "middle" => Button::Middle,
                other => return Err(RpcError::invalid(format!("button '{other}' ไม่รู้จัก"))),
            };
            run(move |e| {
                if let (Some(x), Some(y)) = (p.x, p.y) {
                    e.move_mouse(x, y, Coordinate::Abs)?;
                }
                e.button(button, Direction::Click)?;
                if p.double {
                    std::thread::sleep(Duration::from_millis(60));
                    e.button(button, Direction::Click)?;
                }
                Ok(())
            })
            .await
        }
        "input.scroll" => {
            let p: ScrollParams = parse(params)?;
            run(move |e| {
                if p.dy != 0 {
                    e.scroll(p.dy, Axis::Vertical)?;
                }
                if p.dx != 0 {
                    e.scroll(p.dx, Axis::Horizontal)?;
                }
                Ok(())
            })
            .await
        }
        "input.type" => {
            let p: TypeParams = parse(params)?;
            run(move |e| Ok(e.text(&p.text)?)).await
        }
        "input.key" => {
            let p: KeyParams = parse(params)?;
            let key = key_from_name(&p.key)?;
            let mods = p.modifiers.iter().map(|m| modifier(m)).collect::<Res<Vec<_>>>()?;
            run(move |e| {
                for m in &mods {
                    e.key(*m, Direction::Press)?;
                }
                let pressed = e.key(key, Direction::Click);
                // ปล่อย modifier เสมอ แม้กดปุ่มพลาด ไม่งั้น Ctrl/Alt ค้างบนเครื่องปลายทาง
                for m in mods.iter().rev() {
                    e.key(*m, Direction::Release)?;
                }
                Ok(pressed?)
            })
            .await
        }
        _ => unreachable!("dispatch ส่งมาเฉพาะ input.*"),
    }
}

type InputResult = Result<(), Box<dyn std::error::Error + Send + Sync>>;

async fn run(job: impl FnOnce(&mut Enigo) -> InputResult + Send + 'static) -> Res<Value> {
    tokio::task::spawn_blocking(move || -> Res<Value> {
        let _lock = INPUT_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let mut enigo = Enigo::new(&Settings::default())?;
        job(&mut enigo).map_err(|e| RpcError::new(crate::rpc::INTERNAL, e.to_string()))?;
        Ok(json!({"ok": true}))
    })
    .await?
}

fn modifier(name: &str) -> Res<Key> {
    Ok(match name {
        "ctrl" => Key::Control,
        "alt" => Key::Alt,
        "shift" => Key::Shift,
        "meta" => Key::Meta,
        other => return Err(RpcError::invalid(format!("modifier '{other}' ไม่รู้จัก"))),
    })
}

fn key_from_name(name: &str) -> Res<Key> {
    Ok(match name.to_lowercase().as_str() {
        "enter" | "return" => Key::Return,
        "tab" => Key::Tab,
        "escape" | "esc" => Key::Escape,
        "backspace" => Key::Backspace,
        "delete" => Key::Delete,
        "space" => Key::Space,
        "up" => Key::UpArrow,
        "down" => Key::DownArrow,
        "left" => Key::LeftArrow,
        "right" => Key::RightArrow,
        "home" => Key::Home,
        "end" => Key::End,
        "pageup" => Key::PageUp,
        "pagedown" => Key::PageDown,
        "f1" => Key::F1,
        "f2" => Key::F2,
        "f3" => Key::F3,
        "f4" => Key::F4,
        "f5" => Key::F5,
        "f6" => Key::F6,
        "f7" => Key::F7,
        "f8" => Key::F8,
        "f9" => Key::F9,
        "f10" => Key::F10,
        "f11" => Key::F11,
        "f12" => Key::F12,
        single if single.chars().count() == 1 => Key::Unicode(single.chars().next().unwrap()),
        other => return Err(RpcError::invalid(format!("key '{other}' ไม่รู้จัก"))),
    })
}
