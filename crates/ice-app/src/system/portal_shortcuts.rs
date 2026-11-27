//! Wayland 档的全局热键：`xdg-desktop-portal` 的 `GlobalShortcuts`（LNX-002）。
//!
//! 为什么必须有这条：LNX-001 的 Linux 热键是 `XGrabKey` 抓 root，而 **mutter 只在焦点是
//! X11 窗口时才把按键交给 XWayland**。焦点在 DING 桌面、GNOME 原生应用时那一下根本到不了
//! 我们手里（用户 2026-11-26："点桌面再按 F1 没反应，点 Qoder 就正常"）。
//! 门户的 `GlobalShortcuts` 由合成器自己派发，与焦点在谁那里无关。
//!
//! 形状与 `system::portal`（截图那条）一致，四条纪律照搬：
//!
//! 1. **一条常驻连接**：会话与订阅挂在它身上，随调用建连接会让第二次起全部失效
//!    （LNX-001 在截图链上踩过：第二张起永远超时）。
//! 2. **绑定是异步的、且会弹一次授权框**：`CreateSession` 很快（不弹框），
//!    `BindShortcuts` 要等用户点"允许"，所以绑定放在工作线程里，`apply` 只回"开始绑了没有"。
//! 3. **命中只写同一个 `PENDING`**：与 mac / `XGrabKey` 共用一条分派，
//!    写入动作在 `global_hotkeys::note_id`，这里不自己开路径。
//! 4. **失败如实记**：门户缺席/被拒/触发被占 → 记 `FAILED` 并 warn；界面那句"只能用图片页
//!    那颗按钮"仍然是正确的降级出口。
#![cfg(target_os = "linux")]

use std::collections::HashMap;
use std::sync::Mutex;
use std::sync::mpsc;
use std::time::{Duration, Instant};

use ice_core::config::{BaseKey, Combo, HotkeyConfig, is_unbound, parse_combo};
use zbus::MatchRule;
use zbus::blocking::{Connection, MessageIterator};
use zbus::zvariant::{OwnedObjectPath, OwnedValue};

use super::global_hotkeys::HotkeyFailure;

const PORTAL: &str = "org.freedesktop.portal.Desktop";
const PORTAL_PATH: &str = "/org/freedesktop/portal/desktop";
const SHORTCUTS_IFACE: &str = "org.freedesktop.portal.GlobalShortcuts";
const REQUEST_IFACE: &str = "org.freedesktop.portal.Request";

/// 三条动作的门户口径 id。与 mac 的 `ID_CAPTURE/ID_PIN_LAST/ID_REOPEN` 一一对应，
/// 命中后由 `global_hotkeys::note_id` 翻成同一个 `PENDING` 码。
const IDS: [(&str, &str); 3] = [
    ("capture", "区域截图"),
    ("pin_last", "固定最近一次框选"),
    ("reopen", "打开上一次截图"),
];

/// 一条要绑的键：门户 id、触发描述、给授权框看的名字
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Planned {
    pub id: &'static str,
    pub trigger: String,
    pub description: &'static str,
}

/// 已经绑定/正在绑的那条工作线程的投递口
static WORKER: Mutex<Option<mpsc::Sender<Vec<Planned>>>> = Mutex::new(None);

/// 最近一次绑定失败的原因（界面想提示时可取；取走即清）
static FAILED: Mutex<Option<HotkeyFailure>> = Mutex::new(None);

/// 取走"上次绑定失败"的那一格（`None` = 没失败过）
pub fn take_failure() -> Option<HotkeyFailure> {
    FAILED.lock().ok().and_then(|mut slot| slot.take())
}

/// 触发描述：XDG shortcuts 的格式（`F1`、`CTRL+SHIFT+F1`、`LOGO+F3`）。
///
/// 修饰键顺序固定 `LOGO+CTRL+ALT+SHIFT`（门户与各家桌面对顺序不敏感，但固定下来单测才写得死）。
pub(super) fn trigger_of(combo: &Combo) -> Option<String> {
    let base = match combo.base {
        BaseKey::Function(n) if (1..=12).contains(&n) => format!("F{n}"),
        BaseKey::Letter(ch) => ch.to_ascii_uppercase().to_string(),
        BaseKey::Digit(ch) => ch.to_string(),
        _ => return None,
    };
    let mut parts: Vec<&str> = Vec::new();
    if combo.cmd {
        parts.push("LOGO");
    }
    if combo.control {
        parts.push("CTRL");
    }
    if combo.option {
        parts.push("ALT");
    }
    if combo.shift {
        parts.push("SHIFT");
    }
    parts.push(&base);
    Some(parts.join("+"))
}

/// 配置 → 要绑的那几条。**纯函数**：未绑定的跳过、认不出的跳过、两条撞同一个触发串在这里
/// 就报 `SelfConflict`（与 `registration_plan` 同一条口径）。
fn plan_of(config: &HotkeyConfig) -> Result<Vec<Planned>, HotkeyFailure> {
    let raw = [
        ("capture", &config.capture),
        ("pin_last", &config.pin_last),
        ("reopen", &config.reopen),
    ];
    let mut plan: Vec<Planned> = Vec::new();
    for (id, value) in raw {
        if is_unbound(value) {
            continue;
        }
        let Some(combo) = parse_combo(value) else {
            tracing::warn!(hotkey = id, "快捷键认不出来，这一把不绑");
            continue;
        };
        let Some(trigger) = trigger_of(&combo) else {
            tracing::warn!(hotkey = id, "快捷键用的键门户不支持，这一把不绑");
            continue;
        };
        if let Some(other) = plan.iter().find(|p| p.trigger == trigger) {
            tracing::warn!(
                hotkey = id,
                conflicts = other.id,
                "两条动作撞在同一个组合上"
            );
            return Err(HotkeyFailure::SelfConflict);
        }
        let description = IDS
            .iter()
            .find(|(known, _)| *known == id)
            .map(|(_, d)| *d)
            .unwrap_or("Ice Translator");
        plan.push(Planned {
            id,
            trigger,
            description,
        });
    }
    Ok(plan)
}

/// 绑（或改绑）三条热键。返回 `Ok` = 已经交给工作线程去绑了。
pub fn apply(config: &HotkeyConfig) -> Result<(), HotkeyFailure> {
    let plan = plan_of(config)?;
    {
        let Ok(slot) = WORKER.lock() else {
            return Err(HotkeyFailure::Unavailable);
        };
        if let Some(tx) = slot.as_ref() {
            // 已经起过：改绑交给那条线程（它会先注销旧的再绑新的）
            return tx.send(plan).map_err(|_| HotkeyFailure::Unavailable);
        }
    }
    let conn = Connection::session().map_err(|err| {
        tracing::warn!("连不上会话总线（{err}），全局热键走不了门户");
        HotkeyFailure::Unavailable
    })?;
    // 非沙箱二进制没有身份，门户会回 NotAllowed: An app id is required。
    // 先在 host portal 里用桌面文件 id 注册，后续调用才认这个应用。
    if let Err(err) = conn.call_method(
        Some(PORTAL),
        PORTAL_PATH,
        Some("org.freedesktop.host.portal.Registry"),
        "Register",
        &("ice-translator", HashMap::<&str, OwnedValue>::new()),
    ) {
        tracing::warn!("host portal 注册失败（{err}），继续尝试建会话");
    }
    let session = create_session(&conn).map_err(|err| {
        tracing::warn!("门户 GlobalShortcuts 建会话失败（{err}）");
        HotkeyFailure::Unavailable
    })?;
    listen_activated(conn.clone(), session.clone());
    let (tx, rx) = mpsc::channel::<Vec<Planned>>();
    let worker_conn = conn.clone();
    let worker_session = session.clone();
    let first = plan.clone();
    std::thread::spawn(move || {
        let mut pending_plan = Some(first);
        loop {
            let plan = match pending_plan.take() {
                Some(plan) => plan,
                None => match rx.recv() {
                    Ok(plan) => plan,
                    Err(_) => return,
                },
            };
            if let Err(err) = bind(&worker_conn, &worker_session, &plan) {
                tracing::warn!(
                    ?err,
                    "门户绑定全局热键失败（授权被拒、或触发被别的应用占着）"
                );
                if let Ok(mut slot) = FAILED.lock() {
                    *slot = Some(HotkeyFailure::Unavailable);
                }
            } else {
                tracing::info!(hotkeys = plan.len(), "门户全局热键已绑定");
            }
        }
    });
    if let Ok(mut slot) = WORKER.lock() {
        *slot = Some(tx);
    }
    Ok(())
}

/// `CreateSession`：不弹框，回 `session_handle`（`o`）
fn create_session(conn: &Connection) -> Result<OwnedObjectPath, String> {
    let token = format!("icehotkeys{}", std::process::id());
    let options: HashMap<&str, OwnedValue> = HashMap::from([
        ("handle_token", owned(&format!("{token}r"))),
        ("session_handle_token", owned(&token)),
    ]);
    let reply = conn
        .call_method(
            Some(PORTAL),
            PORTAL_PATH,
            Some(SHORTCUTS_IFACE),
            "CreateSession",
            &(&options,),
        )
        .map_err(|e| e.to_string())?;
    let request: OwnedObjectPath = reply.body().deserialize().map_err(|e| e.to_string())?;
    let (code, results) = wait_response(conn, request.as_str(), Duration::from_secs(10))?;
    if code != 0 {
        return Err(format!("CreateSession response={code}"));
    }
    let handle = results
        .get("session_handle")
        .and_then(|v| String::try_from(v.clone()).ok())
        .ok_or_else(|| "CreateSession 没回 session_handle".to_string())?;
    OwnedObjectPath::try_from(handle).map_err(|e| e.to_string())
}

/// `BindShortcuts`：会弹一次授权框，最长等 120 秒
fn bind(conn: &Connection, session: &OwnedObjectPath, plan: &[Planned]) -> Result<(), String> {
    if plan.is_empty() {
        return Ok(());
    }
    let shortcuts: Vec<(&str, HashMap<&str, OwnedValue>)> = plan
        .iter()
        .map(|p| {
            let props: HashMap<&str, OwnedValue> = HashMap::from([
                ("description", owned(p.description)),
                ("preferred_trigger", owned(&p.trigger)),
            ]);
            (p.id, props)
        })
        .collect();
    let options: HashMap<&str, OwnedValue> = HashMap::new();
    let reply = conn
        .call_method(
            Some(PORTAL),
            PORTAL_PATH,
            Some(SHORTCUTS_IFACE),
            "BindShortcuts",
            &(session, &shortcuts, &"", &options),
        )
        .map_err(|e| e.to_string())?;
    let request: OwnedObjectPath = reply.body().deserialize().map_err(|e| e.to_string())?;
    let (code, _results) = wait_response(conn, request.as_str(), Duration::from_secs(120))?;
    if code == 0 {
        Ok(())
    } else {
        Err(format!("BindShortcuts response={code}"))
    }
}

/// 订阅 `Activated`：命中只写同一个 `PENDING`（经 `global_hotkeys::note_id`）
fn listen_activated(conn: Connection, session: OwnedObjectPath) {
    let rule = match MatchRule::builder()
        .interface(SHORTCUTS_IFACE)
        .and_then(|b| b.member("Activated"))
    {
        Ok(builder) => builder.build(),
        Err(err) => {
            tracing::warn!("订阅 Activated 的规则拼不出来（{err}）");
            return;
        }
    };
    let Ok(iter) = MessageIterator::for_match_rule(rule, &conn, Some(8)) else {
        tracing::warn!("订阅不上 GlobalShortcuts.Activated");
        return;
    };
    let session_text = session.as_str().to_string();
    std::thread::spawn(move || {
        for msg in iter.flatten() {
            let active = match msg
                .body()
                .deserialize::<(OwnedObjectPath, String, u64, HashMap<String, OwnedValue>)>()
            {
                Ok(active) => active,
                Err(_) => continue,
            };
            if active.0.as_str() != session_text {
                continue;
            }
            super::global_hotkeys::note_id(&active.1);
        }
    });
}

type Response = (u32, HashMap<String, OwnedValue>);

/// 等某一条请求的 `Response`。**订阅先于调用**才不会有回信丢失的窗口，所以这里临时起一个
/// 迭代器、按请求路径过滤。
fn wait_response(
    conn: &Connection,
    request_path: &str,
    budget: Duration,
) -> Result<Response, String> {
    let rule = MatchRule::builder()
        .interface(REQUEST_IFACE)
        .and_then(|b| b.member("Response"))
        .map_err(|e| e.to_string())?
        .build();
    let iter = MessageIterator::for_match_rule(rule, conn, Some(8)).map_err(|e| e.to_string())?;
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
    let started = Instant::now();
    while started.elapsed() < budget {
        match rx.recv_timeout(Duration::from_millis(200)) {
            Ok((path, reply)) if path == request_path => return Ok(reply),
            Ok(_) => continue,
            Err(mpsc::RecvTimeoutError::Timeout) => continue,
            Err(mpsc::RecvTimeoutError::Disconnected) => break,
        }
    }
    Err(format!("{budget:?} 内没等到 {request_path} 的回信"))
}

/// `OwnedValue` 只对 HashMap/Optional 实现了 `From`；字符串走 `Value` 转一次
fn owned(text: &str) -> OwnedValue {
    let value = zbus::zvariant::Value::from(text.to_string());
    OwnedValue::try_from(value).expect("字符串总能转成 OwnedValue")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn combo(raw: &str) -> Combo {
        parse_combo(raw).expect("测试串都该能解析")
    }

    /// 裸键：只出键名
    #[test]
    fn a_bare_function_key_formats_as_its_name() {
        assert_eq!(trigger_of(&combo("f1")).as_deref(), Some("F1"));
        assert_eq!(trigger_of(&combo("f12")).as_deref(), Some("F12"));
    }

    /// 修饰键顺序固定 LOGO+CTRL+ALT+SHIFT，字母大写
    #[test]
    fn modifiers_come_in_a_fixed_order_and_letters_are_uppercase() {
        assert_eq!(
            trigger_of(&combo("shift+ctrl+alt+cmd+f2")).as_deref(),
            Some("LOGO+CTRL+ALT+SHIFT+F2")
        );
        assert_eq!(trigger_of(&combo("ctrl+a")).as_deref(), Some("CTRL+A"));
        assert_eq!(trigger_of(&combo("alt+7")).as_deref(), Some("ALT+7"));
    }

    /// 配置 → 计划：未绑定的跳过，三条都在时顺序固定
    #[test]
    fn the_plan_skips_unbound_entries() {
        let config = HotkeyConfig {
            capture: "f1".into(),
            pin_last: "".into(),
            reopen: "f2".into(),
        };
        let plan = plan_of(&config).expect("不撞车");
        assert_eq!(plan.len(), 2);
        assert_eq!(plan[0].id, "capture");
        assert_eq!(plan[0].trigger, "F1");
        assert_eq!(plan[1].id, "reopen");
    }

    /// 两条撞同一个触发串 → SelfConflict（到门户那一步才发现就没有这句能行动的话了）
    #[test]
    fn two_actions_on_one_trigger_are_a_self_conflict() {
        let config = HotkeyConfig {
            capture: "f3".into(),
            pin_last: "f3".into(),
            reopen: "f2".into(),
        };
        assert_eq!(plan_of(&config), Err(HotkeyFailure::SelfConflict));
    }

    /// 认不出的组合不绑、也不报错（与 `registration_plan` 同一条口径）
    #[test]
    fn an_unparsable_combo_is_skipped_not_fatal() {
        let config = HotkeyConfig {
            capture: "nonsense".into(),
            pin_last: "f2".into(),
            reopen: "f3".into(),
        };
        let plan = plan_of(&config).expect("跳过不是失败");
        assert_eq!(plan.len(), 2);
    }
}
