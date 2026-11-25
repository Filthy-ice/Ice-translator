//! xdg-desktop-portal 的两条调用（截图、另存为）——**一条会话连接 + 一条订阅**共用。
//!
//! 这里的形状与两份证据绑在一起，改之前先读：
//!
//! 1. **连接必须活得比一次调用长**（2026-11-25 真机缺陷）：订阅挂在连接上，每张截图重建一条
//!    连接，就是"第一次出图、之后每次白等满超时"（`elapsed_ms=30001 screens=0`）。
//! 2. **一条常驻订阅线程**：`blocking::MessageIterator::next()` 没有超时
//!    （`zbus-5.13.2/src/blocking/message_iterator.rs:112` 就是裸 `block_on`），每次调用起一条
//!    线程就会留下唤不醒的线程。
//! 3. **超时那一路仍要清文件**：发起方放弃后槽位留着、只收走回信通道，订阅线程收到晚到的
//!    回信时按 REQ-002"读完即释放"把门户落的图删掉。

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Sender};
use std::time::Duration;

use zbus::MatchRule;
use zbus::blocking::MessageIterator;
use zbus::zvariant::{OwnedObjectPath, OwnedValue};

const DESKTOP: &str = "org.freedesktop.portal.Desktop";
const DESKTOP_PATH: &str = "/org/freedesktop/portal/desktop";
const SCREENSHOT_IFACE: &str = "org.freedesktop.portal.Screenshot";
const REQUEST_IFACE: &str = "org.freedesktop.portal.Request";

/// 静默抓屏的等待上限。本机实测端到端 0.5 秒（V-005），这 30 秒是给门户自己弹的
/// 授权框留的——把上限压到秒级会把"用户正在点允许"判成失败。
const SILENT_WAIT: Duration = Duration::from_secs(30);
/// 交互那一档等的是人画框，不是机器干活
const INTERACTIVE_WAIT: Duration = Duration::from_secs(300);
/// "另存为"同样在等人点确认
const PICK_WAIT: Duration = Duration::from_secs(300);

/// 门户一次调用的四种真实结局。**这里刻意不出现 `Outcome`**：搬到哪个落点是
/// [`super::settle`] 的事，那一格要能无头测。
pub enum Reply {
    /// 门户落的图在哪（门户自己起的名字，不含画面内容）
    Image(PathBuf),
    /// `response=1`：用户在门户那一侧放弃了
    Cancelled,
    /// 会话总线连不上、门户没注册、调用本身报错
    Unreachable,
    /// 门户应了却给不出图：拒绝授权、超时、`uri` 不是 `file://`、非 0/1 的 response
    NoPixels,
}

/// `(response, results)`。用 owned 类型：`body().deserialize()` 要求产物是 `'static`
type Response = (u32, HashMap<String, OwnedValue>);

/// 一个在飞的请求：路径 + 回信通道。通道被发起方收走（超时）后置 `None`，
/// 订阅线程收到这种槽位的回信时只清文件。
struct Pending {
    path: String,
    tx: Option<Sender<Response>>,
}

/// 在飞的请求。同一时刻只允许一个（`state::submit_capture` 那格闸门保证），
/// 上限留 8 是因为超时放弃的那一路要活到门户晚到的回信把它清掉。
static PENDING: Mutex<Vec<Pending>> = Mutex::new(Vec::new());
/// 订阅线程是否在场（见模块头第 2 条）。断线时由那条线程自己清零，
/// 否则总线一死，之后的每一次截图都要白等满超时才失败。
static SUBSCRIBED: AtomicBool = AtomicBool::new(false);

/// 会话总线那条连接。**必须活得比一次调用长**：订阅（`MessageIterator`）挂在它身上，
/// 每来一张截图重建一条连接，第一次能出图、之后每次都要白等满超时——
/// 2026-11-25 真机就是这个形状（`elapsed_ms=30001`、`screens=0`，第二次起全失败）。
static CONN: Mutex<Option<zbus::blocking::Connection>> = Mutex::new(None);

fn session() -> Option<zbus::blocking::Connection> {
    let mut slot = CONN.lock().ok()?;
    if slot.is_none() {
        *slot = Some(zbus::blocking::Connection::session().ok()?);
    }
    slot.clone()
}

/// 连接坏了（总线重启过）时扔进垃圾桶，下一次调用重新连
fn drop_session() {
    if let Ok(mut slot) = CONN.lock() {
        *slot = None;
    }
}

pub fn screenshot(interactive: bool) -> Reply {
    let options: HashMap<&str, OwnedValue> =
        HashMap::from([("interactive", OwnedValue::from(interactive))]);
    let budget = if interactive {
        INTERACTIVE_WAIT
    } else {
        SILENT_WAIT
    };
    match exchange(SCREENSHOT_IFACE, "Screenshot", budget, &(&"", &options)) {
        Some(response) => verdict(response),
        None => Reply::Unreachable,
    }
}

/// 一次"另存为"的四种真实结局
pub enum SaveReply {
    Picked(PathBuf),
    Cancelled,
    /// 门户缺席或调用失败
    Unreachable,
    /// 门户应了却没给可用路径（缺 `uris`、`uris` 为空、不是 `file://`）
    Failed,
}

/// 门户的"另存为"（CHG-017 T-014，`org.freedesktop.portal.FileChooser.SaveFile`）。
///
/// **`parent` 必须是一个真实存在的 `x11:0x…`**：空串时后端报
/// "Failed to associate portal window with parent window"，对话框与我们的窗口脱钩。
/// 回信实测可达（2026-11-25 深夜，用户点取消 → `code=1`、`uris=[]`）——前一版
/// "收不到回信"的结论是探针从没让对话框真正结束，不是门户的问题。
pub fn save_file(parent: &str, default_name: &str) -> SaveReply {
    // handle_token 只许 `[A-Za-z_][A-Za-z0-9_]*`（实测带 `-` 会被拒：Invalid token）
    static SEQ: Mutex<u32> = Mutex::new(0);
    let token = match SEQ.lock() {
        Ok(mut seq) => {
            *seq += 1;
            format!("icesave{}x{}", std::process::id(), *seq)
        }
        Err(_) => format!("icesave{}", std::process::id()),
    };
    let options: HashMap<&str, OwnedValue> = HashMap::from([
        ("handle_token", owned_string(token)),
        ("current_name", owned_string(default_name)),
        ("modal", OwnedValue::from(true)),
    ]);
    let title = "选择译文图片保存到哪里";
    match exchange(
        "org.freedesktop.portal.FileChooser",
        "SaveFile",
        PICK_WAIT,
        &(parent, title, &options),
    ) {
        Some((0, results)) => match results.get("uris").and_then(uris_of) {
            Some(list) if !list.is_empty() => SaveReply::Picked(list[0].clone()),
            _ => SaveReply::Failed,
        },
        Some((1, _)) => SaveReply::Cancelled,
        Some(_) => SaveReply::Failed,
        None => SaveReply::Unreachable,
    }
}

/// `OwnedValue` 只对 HashMap/Optional 实现了 `From`；字符串走 `Value` 转一次
fn owned_string(text: impl Into<String>) -> OwnedValue {
    let value = zbus::zvariant::Value::from(text.into());
    OwnedValue::try_from(value).expect("字符串总能转成 OwnedValue")
}

/// `a{sv}` 里的 `uris`（`as` 数组）→ 本地路径列表
fn uris_of(value: &OwnedValue) -> Option<Vec<PathBuf>> {
    let list: Vec<String> = Vec::<String>::try_from(value.clone()).ok()?;
    Some(list.into_iter().filter_map(|u| uri_to_path(&u)).collect())
}

/// 门户调用的公共骨架：拿常驻连接 → 确保订阅 → 发请求 → 带上限等这条请求的回信。
/// 返回 `None` = 连门户都搭不上（调用方各自解释成 `Unreachable`）。
fn exchange<B: serde::Serialize + zbus::zvariant::DynamicType>(
    interface: &str,
    method: &str,
    budget: Duration,
    body: &B,
) -> Option<Response> {
    let conn = session()?;
    if let Err(_reply) = ensure_subscriber(&conn) {
        drop_session();
        return None;
    }
    let called = conn.call_method(Some(DESKTOP), DESKTOP_PATH, Some(interface), method, body);
    let request: OwnedObjectPath = match called {
        Ok(reply) => reply.body().deserialize::<OwnedObjectPath>().ok()?,
        Err(_) => {
            // 发不出去多半是总线没了：丢掉这条连接，下一次从新连接开始
            drop_session();
            return None;
        }
    };
    let (tx, rx) = mpsc::channel();
    if !remember(request.as_str(), tx) {
        return None;
    }
    match rx.recv_timeout(budget) {
        Ok(response) => Some(response),
        Err(_) => match rx.try_recv() {
            // 回信与截止线贴在一起时先把它捡回来：那张图已经落在用户的目录里，
            // 判成超时不捡 = 把桌面内容留在我们管不到的地方
            Ok(response) => Some(response),
            Err(_) => {
                abandon(request.as_str());
                None
            }
        },
    }
}

/// 门户的 `(response, results)` → 四种结局。`0` 却给不出 `file://` 也算拿不到图，
/// 不算成功——判据的最后一格始终在 [`super::settle`] 那边（文件在不在、解不解得开）。
fn verdict(response: Response) -> Reply {
    let (code, results) = response;
    match code {
        0 => uri_path(&results).map_or(Reply::NoPixels, Reply::Image),
        1 => Reply::Cancelled,
        _ => Reply::NoPixels,
    }
}

/// 起那条常驻订阅线程。已经在场直接返回；建订阅失败（连不上总线、门户不响应）交回
/// `Unreachable` 而不是让调用方看到 panic——那正是 `explain()` 那句"先用系统截图再点「粘贴」"
/// 要兜住的一档。
fn ensure_subscriber(conn: &zbus::blocking::Connection) -> Result<(), Reply> {
    // 先占位再起线程：两个调用同时进来时，第二条线程会把同一条订阅重复挂上总线
    if SUBSCRIBED.swap(true, Ordering::SeqCst) {
        return Ok(());
    }
    let rule = MatchRule::builder()
        .interface(REQUEST_IFACE)
        .expect("静态字符串不会错")
        .member("Response")
        .expect("静态字符串不会错")
        .build();
    let iter = match MessageIterator::for_match_rule(rule, conn, Some(8)) {
        Ok(iter) => iter,
        Err(_) => {
            SUBSCRIBED.store(false, Ordering::SeqCst);
            return Err(Reply::Unreachable);
        }
    };
    std::thread::spawn(move || listen(iter));
    Ok(())
}

fn listen(iter: MessageIterator) {
    // 读到错误就当连接不可信、直接收线程：`blocking` 的迭代器出错后继续 `next()`
    // 只会同样出错，留在这里就是空转。退出即清占位，下一次调用重新起一条。
    for msg in iter {
        let Ok(msg) = msg else { break };
        // 只认得出「哪个请求的回信」这一层，画面内容不进日志（NFR-3 同族）
        let Some(path) = msg.header().path().map(|p| p.as_str().to_string()) else {
            continue;
        };
        let Ok(response) = msg.body().deserialize::<Response>() else {
            continue;
        };
        deliver(&path, response);
    }
    SUBSCRIBED.store(false, Ordering::SeqCst);
}

/// 记下这个请求。槽位满了就挤掉最老的一个（那条晚到的回信就没人清了，
/// 而它只可能是"用户早就忘了那个授权框"留下的一张门户自己的图）。
fn remember(path: &str, tx: Sender<Response>) -> bool {
    let Ok(mut pending) = PENDING.lock() else {
        return false;
    };
    if pending.len() >= 8 {
        pending.remove(0);
    }
    pending.push(Pending {
        path: path.to_string(),
        tx: Some(tx),
    });
    true
}

/// 发起方不等了：收走通道，槽位留着给晚到的回信清文件
fn abandon(path: &str) {
    if let Ok(mut pending) = PENDING.lock() {
        if let Some(at) = pending.iter_mut().find(|p| p.path == path) {
            at.tx = None;
        }
    }
}

/// 回信交给等它的那一个。返回 false = 这不是我们的请求（别人的、或已被超时那一路处理过），
/// **此时绝不许动它的文件**——那是别人家的请求，删它等于替别人清桌面。
fn deliver(path: &str, response: Response) -> bool {
    // 一次锁内摘走槽位：`position` 的下标跨两次 `lock()` 就可能是别人的位置
    let slot = PENDING.lock().ok().and_then(|mut pending| {
        pending
            .iter()
            .position(|p| p.path == path)
            .map(|at| pending.remove(at))
    });
    let Some(Pending { tx, .. }) = slot else {
        return false;
    };
    match tx {
        Some(tx) => tx.send(response).is_ok(),
        None => {
            release(&response.1);
            true
        }
    }
}

/// 按 REQ-002"读完即释放"把门户落的图删掉（超时那一路的收尾）
fn release(results: &HashMap<String, OwnedValue>) {
    if let Some(path) = uri_path(results) {
        std::fs::remove_file(path).ok();
    }
}

/// `results.uri` → 本地路径。门户回的是百分号编码的 URI（本机实测
/// `file:///home/…/%E5%9B%BE%E7%89%87/Screenshot-5.png`），不解码就找不到文件
fn uri_path(results: &HashMap<String, OwnedValue>) -> Option<PathBuf> {
    let uri = results.get("uri")?;
    let uri = String::try_from(uri.clone()).ok()?;
    uri_to_path(&uri)
}

fn percent_decode(input: &str) -> String {
    let bytes = input.as_bytes();
    let mut out: Vec<u8> = Vec::with_capacity(bytes.len());
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

fn uri_to_path(uri: &str) -> Option<PathBuf> {
    let raw = uri.strip_prefix("file://")?;
    Some(PathBuf::from(percent_decode(raw)))
}

#[cfg(test)]
#[test]
fn a_percent_encoded_portal_uri_points_at_the_real_file() {
    // 本机实测的形态（V-005）：目录名是中文，门户回的是编码后的 URI
    let path = uri_to_path("file:///home/u/%E5%9B%BE%E7%89%87/Screenshot-5.png").unwrap();
    assert_eq!(path, PathBuf::from("/home/u/图片/Screenshot-5.png"));
    // 不是 file:// 就不去猜——那种形态本轮没在本机出现过，读它等于读一个不存在的路径
    assert!(uri_to_path("http://example.com/x.png").is_none());
}
