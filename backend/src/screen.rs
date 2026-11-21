//! จับภาพหน้าจอ/หน้าต่าง: snapshot (ให้ AI) และ MJPEG stream (ให้คนดูสด)
//!
//! พิกัด: ภาพที่ส่งออกอาจถูกย่อ (max_width) — ผลลัพธ์บอก origin_x/origin_y และ scale ไว้ให้แปลงกลับ:
//!   screen_x = origin_x + pixel_x * scale   (ใช้กับ input.* ซึ่งรับพิกัดของ virtual desktop)

use crate::rpc::{Res, RpcError};
use axum::body::Body;
use base64::{Engine, engine::general_purpose::STANDARD as B64};
use image::{ExtendedColorType, ImageEncoder, RgbaImage, imageops::FilterType};
use serde::Deserialize;
use serde_json::{Value, json};
use std::{
    io::Cursor,
    time::{Duration, Instant},
};
use xcap::{Monitor, Window};

const MAX_FPS: u32 = 15;

#[derive(Deserialize, Clone, Copy, Default, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum Format {
    #[default]
    Webp,
    Png,
    Jpeg,
}

/// ใช้ร่วมกันทั้ง screen.snapshot (JSON) และ /v1/screen/stream (query string) — ทุกช่องเป็น optional
/// เลือกเป้าหมาย: window_* อย่างใดอย่างหนึ่ง, หรือ monitor (id จาก screen.list), ไม่ใส่เลย = จอหลัก
#[derive(Deserialize, Clone, Default)]
pub struct CaptureParams {
    pub monitor: Option<u32>,
    pub window_id: Option<u32>,
    pub window_pid: Option<u32>,
    /// ชื่อหน้าต่างบางส่วน (ไม่สนตัวพิมพ์)
    pub window_title: Option<String>,
    #[serde(default)]
    pub format: Format,
    pub max_width: Option<u32>,
    /// 1-100 ใช้กับ jpeg (stream ใช้ jpeg เสมอ)
    pub quality: Option<u8>,
    /// เฉพาะ stream
    pub fps: Option<u32>,
}

struct Frame {
    image: RgbaImage,
    origin: (i32, i32),
    source_width: u32,
}

fn capture(p: &CaptureParams) -> Res<Frame> {
    let by_window = p.window_id.is_some() || p.window_pid.is_some() || p.window_title.is_some();
    let (image, origin) = if by_window {
        let title = p.window_title.as_ref().map(|t| t.to_lowercase());
        let window = Window::all()?
            .into_iter()
            .filter(|w| !w.is_minimized().unwrap_or(false))
            .find(|w| {
                p.window_id.is_none_or(|id| w.id().ok() == Some(id))
                    && p.window_pid.is_none_or(|pid| w.pid().ok() == Some(pid))
                    && title
                        .as_ref()
                        .is_none_or(|t| w.title().is_ok_and(|x| x.to_lowercase().contains(t)))
            })
            .ok_or_else(|| RpcError::invalid("ไม่พบหน้าต่างที่ตรงเงื่อนไข (หน้าต่างที่ย่ออยู่จะไม่ถูกนับ)"))?;
        (window.capture_image()?, (window.x()?, window.y()?))
    } else {
        let monitors = Monitor::all()?;
        let monitor = match p.monitor {
            Some(id) => monitors.into_iter().find(|m| m.id().ok() == Some(id)),
            None => {
                let mut all = monitors;
                let primary = all.iter().position(|m| m.is_primary().unwrap_or(false));
                (!all.is_empty()).then(|| all.swap_remove(primary.unwrap_or(0)))
            }
        }
        .ok_or_else(|| RpcError::invalid("ไม่พบจอที่ระบุ"))?;
        (monitor.capture_image()?, (monitor.x()?, monitor.y()?))
    };
    let source_width = image.width();
    let image = match p.max_width {
        Some(w) if w > 0 && w < source_width => {
            let h = (image.height() as u64 * w as u64 / source_width as u64).max(1) as u32;
            image::imageops::resize(&image, w, h, FilterType::Triangle)
        }
        _ => image,
    };
    Ok(Frame {
        image,
        origin,
        source_width,
    })
}

fn encode(img: &RgbaImage, format: Format, quality: Option<u8>) -> Res<Vec<u8>> {
    let (w, h) = img.dimensions();
    let mut out = Cursor::new(Vec::new());
    match format {
        Format::Png => {
            image::codecs::png::PngEncoder::new(&mut out).write_image(img, w, h, ExtendedColorType::Rgba8)?
        }
        // WebP ของ crate image เป็น lossless เท่านั้น — ไฟล์ใหญ่กว่า jpeg แต่ตัวหนังสือคมสำหรับ Vision Model
        Format::Webp => {
            image::codecs::webp::WebPEncoder::new_lossless(&mut out).write_image(img, w, h, ExtendedColorType::Rgba8)?
        }
        Format::Jpeg => {
            let rgb = image::DynamicImage::ImageRgba8(img.clone()).to_rgb8();
            image::codecs::jpeg::JpegEncoder::new_with_quality(&mut out, quality.unwrap_or(75).clamp(1, 100))
                .write_image(&rgb, w, h, ExtendedColorType::Rgb8)?
        }
    }
    Ok(out.into_inner())
}

pub async fn snapshot(p: CaptureParams) -> Res<Value> {
    tokio::task::spawn_blocking(move || {
        let frame = capture(&p)?;
        let (width, height) = frame.image.dimensions();
        let bytes = encode(&frame.image, p.format, p.quality)?;
        let mime = match p.format {
            Format::Webp => "image/webp",
            Format::Png => "image/png",
            Format::Jpeg => "image/jpeg",
        };
        Ok(
            json!({"format": p.format_name(), "mime": mime, "width": width, "height": height,
                  "origin_x": frame.origin.0, "origin_y": frame.origin.1,
                  "scale": frame.source_width as f64 / width as f64,
                  "data_b64": B64.encode(bytes)}),
        )
    })
    .await?
}

impl CaptureParams {
    fn format_name(&self) -> &'static str {
        match self.format {
            Format::Webp => "webp",
            Format::Png => "png",
            Format::Jpeg => "jpeg",
        }
    }
}

pub fn list() -> Res<Value> {
    let monitors: Vec<Value> = Monitor::all()?
        .iter()
        .map(|m| {
            json!({"id": m.id().ok(), "name": m.friendly_name().or_else(|_| m.name()).ok(),
                   "x": m.x().ok(), "y": m.y().ok(), "width": m.width().ok(), "height": m.height().ok(),
                   "scale_factor": m.scale_factor().ok(), "is_primary": m.is_primary().ok()})
        })
        .collect();
    let windows: Vec<Value> = Window::all()?
        .iter()
        .filter(|w| !w.title().unwrap_or_default().is_empty())
        .map(|w| {
            json!({"id": w.id().ok(), "pid": w.pid().ok(), "app": w.app_name().ok(), "title": w.title().ok(),
                   "x": w.x().ok(), "y": w.y().ok(), "width": w.width().ok(), "height": w.height().ok(),
                   "minimized": w.is_minimized().ok(), "focused": w.is_focused().ok()})
        })
        .collect();
    Ok(json!({"monitors": monitors, "windows": windows}))
}

/// สร้างตัวอ่านภาพ MJPEG; `permit` ถูกถือไว้จนกว่า stream จบ (จำกัดจำนวน stream พร้อมกัน)
/// `keep_going` คืน false เมื่อ session ถูก revoke → หยุดส่งทันที
pub fn mjpeg_body(
    p: CaptureParams,
    permit: tokio::sync::OwnedSemaphorePermit,
    keep_going: impl Fn() -> bool + Send + 'static,
) -> Res<Body> {
    capture(&p)?; // ลองจับหนึ่งรอบก่อน เพื่อให้เป้าหมายผิดตอบเป็น error ปกติ ไม่ใช่ stream ว่าง
    let interval = Duration::from_secs_f64(1.0 / p.fps.unwrap_or(5).clamp(1, MAX_FPS) as f64);
    let p = CaptureParams {
        max_width: p.max_width.or(Some(1280)),
        ..p
    };
    let (tx, rx) = tokio::sync::mpsc::channel::<Vec<u8>>(2);
    tokio::task::spawn_blocking(move || {
        let _permit = permit;
        while keep_going() {
            let started = Instant::now();
            let Ok(frame) = capture(&p) else { break };
            let Ok(jpeg) = encode(&frame.image, Format::Jpeg, p.quality.or(Some(60))) else {
                break;
            };
            let mut part = format!(
                "--frame\r\nContent-Type: image/jpeg\r\nContent-Length: {}\r\n\r\n",
                jpeg.len()
            )
            .into_bytes();
            part.extend_from_slice(&jpeg);
            part.extend_from_slice(b"\r\n");
            // client ปิดการเชื่อมต่อ → rx ถูกทิ้ง → send ล้มเหลว → เลิกจับภาพ
            if tx.blocking_send(part).is_err() {
                break;
            }
            std::thread::sleep(interval.saturating_sub(started.elapsed()));
        }
    });
    let stream = futures_util::stream::unfold(rx, |mut rx| async move {
        rx.recv().await.map(|b| (Ok::<_, std::convert::Infallible>(b), rx))
    });
    Ok(Body::from_stream(stream))
}
