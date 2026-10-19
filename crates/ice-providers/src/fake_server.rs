//! 测试用的本地假 HTTP 服务：不依赖第三方包，也不碰网络出口。
//!
//! 只提供引擎测试需要的能力：按顺序应答、挂起不答、以及一个确定没人监听的端口。

use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

#[derive(Clone)]
pub struct Fake {
    pub url: String,
    pub requests: Arc<Mutex<Vec<String>>>,
    pub closed: Arc<AtomicBool>,
}

/// 读完一个请求（含 body），返回原始文本
fn read_request(stream: &mut TcpStream) -> String {
    let mut buf = Vec::new();
    let mut byte = [0u8; 1];
    while stream.read(&mut byte).unwrap_or(0) == 1 {
        buf.push(byte[0]);
        if buf.ends_with(b"\r\n\r\n") {
            break;
        }
    }
    let head = String::from_utf8_lossy(&buf).to_lowercase();
    let length: usize = head
        .lines()
        .find_map(|line| line.strip_prefix("content-length:"))
        .and_then(|v| v.trim().parse().ok())
        .unwrap_or(0);
    let mut body = Vec::new();
    let _ = stream.take(length as u64).read_to_end(&mut body);
    let mut text = String::from_utf8_lossy(&buf).to_string();
    text.push_str(&String::from_utf8_lossy(&body));
    text
}

fn respond(stream: &mut TcpStream, status: &str, body: &str) {
    let packet = format!(
        "HTTP/1.1 {status}\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
        body.len()
    );
    let _ = stream.write_all(packet.as_bytes());
    let _ = stream.flush();
}

/// 按顺序把 `responses` 逐个作为指定状态码的响应发出，用尽后线程退出。
fn spawn_pairs(items: Vec<(u16, String)>, hang: bool, delay: Duration) -> Fake {
    let listener = TcpListener::bind("127.0.0.1:0").expect("假服务需要能绑定回环地址");
    let url = format!("http://{}", listener.local_addr().expect("已绑定"));
    let fake = Fake {
        url,
        requests: Arc::new(Mutex::new(Vec::new())),
        closed: Arc::new(AtomicBool::new(false)),
    };
    let requests = fake.requests.clone();
    let closed = fake.closed.clone();
    thread::spawn(move || {
        if hang {
            // 持续接受连接、读完请求但不回应，直到进程结束
            loop {
                let Ok((mut stream, _)) = listener.accept() else {
                    break;
                };
                let text = read_request(&mut stream);
                requests.lock().expect("假服务记录锁").push(text);
                thread::sleep(delay);
            }
        } else {
            for (code, response) in items {
                let Ok((mut stream, _)) = listener.accept() else {
                    break;
                };
                let text = read_request(&mut stream);
                requests.lock().expect("假服务记录锁").push(text);
                respond(&mut stream, &status_line(code), &response);
            }
        }
        closed.store(true, Ordering::SeqCst);
    });
    fake
}

/// 状态行需要原因短语，写死常用的几个，其余用通用文案
fn status_line(code: u16) -> String {
    let reason = match code {
        200 => "OK",
        400 => "Bad Request",
        401 => "Unauthorized",
        403 => "Forbidden",
        429 => "Too Many Requests",
        500 => "Internal Server Error",
        _ => "Error",
    };
    format!("{code} {reason}")
}

/// 按顺序把 `responses` 逐个作为 200 响应发出，用尽后线程退出。
pub fn serve(responses: Vec<String>) -> Fake {
    spawn_pairs(
        responses
            .into_iter()
            .map(|body| (200u16, body))
            .collect::<Vec<_>>(),
        false,
        Duration::ZERO,
    )
}

/// 指定状态码的响应，用于验证 401/429/5xx 的错误映射
pub fn serve_status(items: Vec<(u16, String)>) -> Fake {
    spawn_pairs(items, false, Duration::ZERO)
}

/// 接受连接但不回内容，用于验证超时与重试
pub fn serve_hanging(delay: Duration) -> Fake {
    spawn_pairs(Vec::new(), true, delay)
}

/// 一个确定没有监听者的地址（绑定后立即释放），用于验证连接失败
pub fn closed_url() -> String {
    let listener = TcpListener::bind("127.0.0.1:0").expect("需要一个空闲端口");
    let addr = listener.local_addr().expect("已绑定");
    drop(listener);
    format!("http://{addr}")
}

/// 等待假服务收到 `n` 条请求，避免测试靠 sleep 猜时间
pub fn wait_for_requests(fake: &Fake, n: usize) -> Vec<String> {
    for _ in 0..200 {
        let got = fake.requests.lock().expect("假服务记录锁").len();
        if got >= n {
            return fake.requests.lock().expect("假服务记录锁").clone();
        }
        if fake.closed.load(Ordering::SeqCst) {
            break;
        }
        thread::sleep(Duration::from_millis(10));
    }
    fake.requests.lock().expect("假服务记录锁").clone()
}
