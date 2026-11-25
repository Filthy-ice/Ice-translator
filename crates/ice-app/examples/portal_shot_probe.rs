//! 取证（CHG-017 T-001）：门户 `Screenshot.Screenshot` 到底给不给**我们这个应用**出图。
//!
//! 规格里那条假设是 H1：本机 `org.freedesktop.portal.Desktop` 上接口在场、权限库里
//! `screenshot/screenshot` 也有过条目，但"接口在场"不等于"我们的 app id 拿得到像素"。
//! REQ-002 的验收要求这条在真机上跑一次并留读数——门户不给图时 T-007 就要按 Q-1 的口径
//! 改成"隐藏截图入口 + 说明原因"，那是两种产品行为，不能靠推断选。
//!
//! 用法（Linux）：`cargo run -p ice-app --example portal_shot_probe`。
//! 首次调用会弹一次授权框（门户自己管，我们不存权限、不写配置）。
//!
//! 这条链的证据只打印**尺寸与存在性**：截图内容里是用户桌面，路径与像素都不进日志
//! （NFR-3 同族），读完立刻删掉临时文件。

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::mpsc;
use std::time::{Duration, Instant};

use zbus::MatchRule;
use zbus::blocking::MessageIterator;
use zbus::zvariant::{OwnedObjectPath, OwnedValue};

const PORTAL: &str = "org.freedesktop.portal.Desktop";
const PORTAL_PATH: &str = "/org/freedesktop/portal/desktop";
const SCREENSHOT_IFACE: &str = "org.freedesktop.portal.Screenshot";
const REQUEST_IFACE: &str = "org.freedesktop.portal.Request";

/// 门户回信：`(response, results)`；`response` 0 = 成功，1 = 用户取消，其它 = 失败。
/// 用 owned 类型：`Message::body().deserialize()` 要求产物是 `'static`
type Response = (u32, HashMap<String, OwnedValue>);

fn main() {
    let conn = match zbus::blocking::Connection::session() {
        Ok(conn) => conn,
        Err(err) => {
            println!("结论: 连不上会话总线（{err}）——本机的 D-Bus 会话没起来");
            std::process::exit(1);
        }
    };
    let rule = MatchRule::builder()
        .interface(REQUEST_IFACE)
        .expect("静态字符串不会错")
        .member("Response")
        .expect("静态字符串不会错")
        .build();
    let iter = match MessageIterator::for_match_rule(rule, &conn, None) {
        Ok(iter) => iter,
        Err(err) => {
            println!("结论: 订阅不上 Request.Response（{err}）");
            std::process::exit(1);
        }
    };
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        for msg in iter.flatten() {
            let body = msg.body();
            if let Ok(reply) = body.deserialize::<Response>() {
                let path = msg
                    .header()
                    .path()
                    .map(|p| p.as_str().to_string())
                    .unwrap_or_default();
                let _ = tx.send((path, reply));
            }
        }
    });

    let options: HashMap<&str, OwnedValue> = HashMap::new();
    let called = conn.call_method(
        Some(PORTAL),
        PORTAL_PATH,
        Some(SCREENSHOT_IFACE),
        "Screenshot",
        &(&"", &options),
    );
    let path: OwnedObjectPath = match called {
        Ok(reply) => match reply.body().deserialize::<OwnedObjectPath>() {
            Ok(path) => path,
            Err(err) => {
                println!("结论: Screenshot 的返回体解析失败（{err}）");
                std::process::exit(1);
            }
        },
        Err(err) => {
            println!("结论: 调用 Screenshot 失败（{err}）");
            std::process::exit(1);
        }
    };
    println!("请求已发出，等待门户回信（最多 90 秒，含授权框）: {path}");

    let started = Instant::now();
    loop {
        if started.elapsed() > Duration::from_secs(90) {
            println!("结论: 90 秒内门户没有回信（授权框可能还挂在屏幕上，或者门户根本没应答）");
            std::process::exit(1);
        }
        match rx.recv_timeout(Duration::from_secs(2)) {
            Ok((signal_path, (response, results))) => {
                if signal_path != path.as_str() {
                    println!("跳过别的请求的回信: {signal_path}");
                    continue;
                }
                report(response, &results);
                return;
            }
            Err(mpsc::RecvTimeoutError::Timeout) => continue,
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                println!("结论: 信号流断了，没拿到回信");
                std::process::exit(1);
            }
        }
    }
}

/// 只报尺寸与文件是否存在，然后把门户落的临时文件删掉（REQ-002"读完即释放"）
fn report(response: u32, results: &HashMap<String, OwnedValue>) {
    println!("结论: response={response}（0 成功 / 1 用户取消 / 其它失败）");
    if response != 0 {
        for (key, value) in results {
            println!("  {key} = {value:?}");
        }
        std::process::exit(1);
    }
    let uri = results
        .get("uri")
        .and_then(|v| String::try_from(v.clone()).ok());
    let Some(uri) = uri.as_deref() else {
        println!("  回信里没有 uri 字段，拿不到图");
        std::process::exit(1);
    };
    let Some(path) = uri_to_path(uri) else {
        println!("  uri 不是 file:// 形态（{uri}），本轮不尝试读取");
        std::process::exit(1);
    };
    // 这是门户自己落的临时文件名（不含画面内容），进本机日志可以接受；不进仓库证据文件
    println!("  uri = {uri}");
    match std::fs::metadata(&path) {
        Ok(meta) => match image::image_dimensions(&path) {
            Ok((w, h)) => println!("  读到像素: {w}x{h}，文件 {} 字节", meta.len()),
            Err(err) => println!("  文件 {} 字节但解不开: {err}", meta.len()),
        },
        Err(err) => println!("  按 uri 打不开文件: {err}"),
    }
    if let Err(err) = std::fs::remove_file(&path) {
        println!("  临时文件没能删掉（{err}），请手动清理");
    }
}

/// `file://` → 本地路径。门户回的是**百分号编码**的 URI（本机实测：
/// `file:///home/…/%E5%9B%BE%E7%89%87/Screenshot-4.png`），不解码就找不到文件
fn uri_to_path(uri: &str) -> Option<PathBuf> {
    let raw = uri.strip_prefix("file://")?;
    Some(PathBuf::from(percent_decode(raw)))
}

/// 只解 `%XX`（URI 里没有 query/fragment，也没有 `+` 表空格的约定）
fn percent_decode(input: &str) -> String {
    let bytes = input.as_bytes();
    let mut out: Vec<u8> = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            let hex = &input[i + 1..i + 3];
            if let Ok(byte) = u8::from_str_radix(hex, 16) {
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
