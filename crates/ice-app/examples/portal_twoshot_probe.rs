//! 取证（CHG-017 修复回归）：**同一条会话连接 + 一次订阅**，连拍两张。
//!
//! 2026-11-25 真机的缺陷是：每张截图新建一条 D-Bus 连接、订阅却挂在那条马上被丢的连接上，
//! 于是第一张出图、之后每次都白等满 30 秒（日志 `elapsed_ms=30001 screens=0`）。
//! 本探针按修好后的形状跑两次，两次都必须拿到 `response=0` 与一个可读的 uri。
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::mpsc;
use std::time::{Duration, Instant};

use zbus::MatchRule;
use zbus::blocking::MessageIterator;
use zbus::zvariant::{OwnedObjectPath, OwnedValue};

type Response = (u32, HashMap<String, OwnedValue>);

fn main() {
    let conn = zbus::blocking::Connection::session().expect("session bus");
    let rule = MatchRule::builder()
        .interface("org.freedesktop.portal.Request")
        .expect("static")
        .member("Response")
        .expect("static")
        .build();
    let iter = MessageIterator::for_match_rule(rule, &conn, Some(8)).expect("subscribe");
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        for msg in iter.flatten() {
            let path = msg
                .header()
                .path()
                .map(|p| p.as_str().to_string())
                .unwrap_or_default();
            if let Ok(reply) = msg.body().deserialize::<Response>() {
                let _ = tx.send((path, reply));
            }
        }
    });
    for round in 1..=2 {
        let options: HashMap<&str, OwnedValue> =
            HashMap::from([("interactive", OwnedValue::from(false))]);
        let called = conn.call_method(
            Some("org.freedesktop.portal.Desktop"),
            "/org/freedesktop/portal/desktop",
            Some("org.freedesktop.portal.Screenshot"),
            "Screenshot",
            &(&"", &options),
        );
        let request: OwnedObjectPath = called.expect("call").body().deserialize().expect("path");
        let started = Instant::now();
        let mut verdict = String::from("超时");
        while started.elapsed() < Duration::from_secs(30) {
            match rx.recv_timeout(Duration::from_secs(1)) {
                Ok((path, (code, results))) if path == request.as_str() => {
                    let uri = results
                        .get("uri")
                        .and_then(|v| String::try_from(v.clone()).ok())
                        .unwrap_or_default();
                    let file = uri
                        .strip_prefix("file://")
                        .map(percent_decode)
                        .map(PathBuf::from);
                    let dims = file.as_ref().and_then(|p| image::image_dimensions(p).ok());
                    if let Some(p) = &file {
                        std::fs::remove_file(p).ok();
                    }
                    verdict = format!("response={code} uri_ok={} 尺寸={dims:?}", !uri.is_empty());
                    break;
                }
                Ok(_) => continue,
                Err(mpsc::RecvTimeoutError::Timeout) => continue,
                Err(_) => {
                    verdict = "订阅断了".into();
                    break;
                }
            }
        }
        println!(
            "第 {round} 张: {verdict}，耗时 {} ms",
            started.elapsed().as_millis()
        );
    }
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
