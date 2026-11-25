//! 取证（CHG-017 T-002）：同一时刻，门户截图 vs X11 `GetImage(root)` 的像素统计对比。
//!
//! 规格里的推断是"XWayland 的 root 只合成 X11 客户端，Wayland 原生应用拍不到"。
//! 这里把两张图都取回来，按像素统计给出差异比例——差异出现的位置就是 GetImage 缺的那部分。
//!
//! 用法：`cargo run -p ice-app --example grab_source_compare`（Linux + 会话总线）。
//! 只打印尺寸与统计，不落任何图片；门户那张读完即删。
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::mpsc;
use std::time::{Duration, Instant};

use x11rb::connection::Connection;
use x11rb::protocol::xproto::{ConnectionExt as _, ImageFormat};
use zbus::MatchRule;
use zbus::blocking::MessageIterator;
use zbus::zvariant::{OwnedObjectPath, OwnedValue};

fn main() {
    let portal = match portal_shot() {
        Some(img) => img,
        None => {
            println!("门户没拿到图，对比做不了");
            std::process::exit(1);
        }
    };
    let x11 = match root_image() {
        Some(img) => img,
        None => {
            println!("GetImage 失败");
            std::process::exit(1);
        }
    };
    println!(
        "尺寸：门户 {}x{} / GetImage {}x{}",
        portal.width(),
        portal.height(),
        x11.width(),
        x11.height()
    );
    let (w, h) = (
        portal.width().min(x11.width()),
        portal.height().min(x11.height()),
    );
    let mut sampled = 0u64;
    let mut differing = 0u64;
    let mut sum = 0u64;
    for y in (0..h).step_by(3) {
        for x in (0..w).step_by(3) {
            let a = portal.get_pixel(x, y);
            let b = x11.get_pixel(x, y);
            let d = (i32::from(a.0[0]) - i32::from(b.0[0])).abs()
                + (i32::from(a.0[1]) - i32::from(b.0[1])).abs()
                + (i32::from(a.0[2]) - i32::from(b.0[2])).abs();
            sampled += 1;
            sum += d as u64;
            if d > 24 {
                differing += 1;
            }
        }
    }
    println!(
        "抽样 {sampled} 点：平均通道差 {:.2}，显著不同 {differing} 点（{:.2}%）",
        sum as f64 / (sampled.max(1) as f64 * 3.0),
        differing as f64 * 100.0 / sampled.max(1) as f64
    );
}

/// 门户整屏截图 → RGBA
fn portal_shot() -> Option<image::RgbaImage> {
    let conn = zbus::blocking::Connection::session().ok()?;
    let rule = MatchRule::builder()
        .interface("org.freedesktop.portal.Request")
        .ok()?
        .member("Response")
        .ok()?
        .build();
    let iter = MessageIterator::for_match_rule(rule, &conn, Some(8)).ok()?;
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        for msg in iter.flatten() {
            let path = msg
                .header()
                .path()
                .map(|p| p.as_str().to_string())
                .unwrap_or_default();
            if let Ok(reply) = msg
                .body()
                .deserialize::<(u32, HashMap<String, OwnedValue>)>()
            {
                let _ = tx.send((path, reply));
            }
        }
    });
    let options: HashMap<&str, OwnedValue> = HashMap::from([(
        "interactive",
        zbus::zvariant::Value::from(false).try_into().ok()?,
    )]);
    let reply = conn
        .call_method(
            Some("org.freedesktop.portal.Desktop"),
            "/org/freedesktop/portal/desktop",
            Some("org.freedesktop.portal.Screenshot"),
            "Screenshot",
            &(&"", &options),
        )
        .ok()?;
    let request: OwnedObjectPath = reply.body().deserialize().ok()?;
    let started = Instant::now();
    while started.elapsed() < Duration::from_secs(30) {
        if let Ok((path, (code, results))) = rx.recv_timeout(Duration::from_secs(1)) {
            if path != request.as_str() || code != 0 {
                continue;
            }
            let uri = results
                .get("uri")
                .and_then(|v| String::try_from(v.clone()).ok())?;
            let file = PathBuf::from(percent_decode(uri.strip_prefix("file://")?));
            let image = image::open(&file).ok().map(|i| i.to_rgba8());
            std::fs::remove_file(&file).ok();
            return image;
        }
    }
    None
}

/// `GetImage` 整屏（root）。深度用 root 自己的，格式 Z_PIXMAP。
fn root_image() -> Option<image::RgbaImage> {
    let (conn, num) = x11rb::connect(None).ok()?;
    let screen = &conn.setup().roots[num];
    let (w, h) = (screen.width_in_pixels, screen.height_in_pixels);
    let reply = match conn.get_image(ImageFormat::Z_PIXMAP, screen.root, 0, 0, w, h, !0) {
        Ok(cookie) => match cookie.reply() {
            Ok(reply) => reply,
            Err(err) => {
                println!("GetImage 回信失败: {err}");
                return None;
            }
        },
        Err(err) => {
            println!("GetImage 发不出去: {err}");
            return None;
        }
    };
    let mut out = image::RgbaImage::new(u32::from(w), u32::from(h));
    let bytes = reply.data;
    let stride = usize::from(w) * 4;
    for y in 0..usize::from(h) {
        for x in 0..usize::from(w) {
            let at = y * stride + x * 4;
            if at + 3 < bytes.len() {
                out.put_pixel(
                    x as u32,
                    y as u32,
                    image::Rgba([bytes[at + 2], bytes[at + 1], bytes[at], 255]),
                );
            }
        }
    }
    Some(out)
}

fn percent_decode(input: &str) -> String {
    let bytes = input.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            if let Ok(byte) = u8::from_str_radix(&input[i + 1..i + 3], 16) {
                out.push(byte);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8(out).unwrap_or_else(|_| input.to_string())
}
