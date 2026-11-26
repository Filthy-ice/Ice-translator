//! 取证（LNX-002）：门户 `GlobalShortcuts.CreateSession` 在本机是否可用。
//!
//! **只建会话、不绑键**：`BindShortcuts` 会弹一次系统授权框，而 `CreateSession` 不会。
//! 用法：`cargo run -p ice-app --example portal_shortcuts_probe`。
use std::collections::HashMap;
use std::sync::mpsc;
use std::time::{Duration, Instant};

use zbus::MatchRule;
use zbus::blocking::MessageIterator;
use zbus::zvariant::{OwnedObjectPath, OwnedValue};

fn main() {
    let conn = zbus::blocking::Connection::session().expect("session bus");
    // 先订阅再调用：回信可能先于我们拿到 request 路径
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
            if let Ok(reply) = msg
                .body()
                .deserialize::<(u32, HashMap<String, OwnedValue>)>()
            {
                let _ = tx.send((path, reply));
            }
        }
    });
    let options: HashMap<&str, OwnedValue> = HashMap::from([
        (
            "handle_token",
            zbus::zvariant::Value::from("iceprobecreater".to_string())
                .try_into()
                .unwrap(),
        ),
        (
            "session_handle_token",
            zbus::zvariant::Value::from("iceprobecreate".to_string())
                .try_into()
                .unwrap(),
        ),
    ]);
    let reply = conn
        .call_method(
            Some("org.freedesktop.portal.Desktop"),
            "/org/freedesktop/portal/desktop",
            Some("org.freedesktop.portal.GlobalShortcuts"),
            "CreateSession",
            &(&options,),
        )
        .expect("CreateSession");
    let request: OwnedObjectPath = reply.body().deserialize().expect("request path");
    println!("CreateSession 已受理: {request}");
    let started = Instant::now();
    while started.elapsed() < Duration::from_secs(10) {
        if let Ok((path, (code, results))) = rx.recv_timeout(Duration::from_secs(1)) {
            if path == request.as_str() {
                let session = results
                    .get("session_handle")
                    .and_then(|v| String::try_from(v.clone()).ok())
                    .unwrap_or_default();
                println!("response={code} session_handle={session}");
                return;
            }
        }
    }
    println!("10 秒内没等到回信");
}
