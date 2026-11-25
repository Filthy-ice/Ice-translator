//! Linux 档托盘：手写 StatusNotifierItem（CHG-017 T-012，design D-7）。
//!
//! 宿主实测（2026-11-25）：`org.kde.StatusNotifierWatcher` 由 gnome-shell 持有，
//! `IsStatusNotifierHostRegistered = true`（AppIndicator 那一套扩展在场），所以 SNI 挂上去就能显示。
//!
//! 三处形状：
//!
//! 1. **图标走 `IconName` + `IconPixmap` 两份**：主题里装着 `ice-translator.png` 时用名字；
//!    同时给一份 ARGB32 位图，扩展不查主题也能画。
//! 2. **意图只走 `tray::mark`**：与 mac 的 `NSStatusItem` 落同一个 `PENDING` 槽，
//!    `App::logic` 的 `pump_tray` 一字不改（D-7 的"既有链路自动点亮"）。
//! 3. **菜单是 `com.canonical.dbusmenu`**，五项与 `tray::menu_titles` 一一对应；
//!    点选中只做 `mark`，不另开分派路径（与 mac 的 `popUpStatusItemMenu:` 同一条纪律）。
#![cfg(target_os = "linux")]

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use ice_core::config::UiMode;
use zbus::blocking::Connection;
use zbus::zvariant::{OwnedObjectPath, OwnedValue, Structure, Value};

use super::tray::{self, Outcome};
use crate::state::TrayState;

const ITEM_PATH: &str = "/StatusNotifierItem";
const MENU_PATH: &str = "/MenuBar";
const WATCHER: &str = "org.kde.StatusNotifierWatcher";
const WATCHER_PATH: &str = "/StatusNotifierWatcher";
const TITLE: &str = "Ice 翻译";
/// 符号名：`symbolic` 目录里那一支，GNOME 按面板前景色自动上色（与 mac 的
/// `setTemplate(true)` 同等效果）。用全彩应用图标会在深色面板上糊成一团。
const ICON_NAME: &str = "ice-translator-symbolic";

/// 菜单五项与意图的对应（id 从 1 起，顺序与 `tray::menu_titles` 一致）
fn outcome_of(id: i32) -> Option<Outcome> {
    match id {
        1 => Some(Outcome::Reveal),
        2 => Some(Outcome::ToggleMode),
        3 => Some(Outcome::Settings),
        4 => Some(Outcome::ClearShots),
        5 => Some(Outcome::Quit),
        _ => None,
    }
}

/// 常驻的那一份：当前形态与图标档位
struct Tray {
    state: Arc<Mutex<UiMode>>,
    /// 这条连接必须**活到进程结束**：名字、对象服务器与挂在 watcher 上的登记都长在它身上。
    /// 第一版把连接留在 `spawn` 里就返回了，名字在函数退出那一刻被释放——日志写着
    /// "Available" 而菜单栏什么都没有（2026-11-25 真机自查）。
    _conn: Connection,
}

static TRAY: Mutex<Option<Tray>> = Mutex::new(None);

/// 装托盘。已经有就不再装（`install` 是幂等的，与 mac 那一支同一条契约）。
pub fn install(mode: UiMode, ctx: &egui::Context, scale: f64) -> TrayState {
    if let Ok(slot) = TRAY.lock() {
        if slot.is_some() {
            return TrayState::Available;
        }
    }
    // 唤醒源与 mac 共用：`mark` 里要靠它叫醒停摆的帧循环
    super::tray::remember_context(ctx);
    let _ = scale; // 图标走主题里的符号图，档位不再参与（位图那条已撤）
    match spawn(mode) {
        Ok(tray) => {
            if let Ok(mut slot) = TRAY.lock() {
                *slot = Some(tray);
            }
            TrayState::Available
        }
        Err(err) => {
            tracing::warn!("托盘没装上（{err}），这一档按最小化降级");
            TrayState::Unavailable
        }
    }
}

/// 每帧核对。形态变了就更新菜单文案（菜单是"点开时现读"的，所以只改状态就够）。
pub fn sync(mode: UiMode) -> TrayState {
    let Ok(slot) = TRAY.lock() else {
        return TrayState::Unavailable;
    };
    let Some(tray) = slot.as_ref() else {
        return TrayState::Unavailable;
    };
    if let Ok(mut current) = tray.state.lock() {
        *current = mode;
    }
    TrayState::Available
}

/// 这一档没有"激活策略"这件事：Linux 的窗口没有 Dock 磁贴，收进托盘时不需要让任何图标消失。
/// 返回 `true` 是因为调用方把它当"该做的那件事做成了没有"——这里没有失败面，
/// 返回 `false` 只会让 `mode::hide` 打一条属于 mac 的假警告（2026-11-25 真机日志里出现过）。
pub const fn set_policy(_regular: bool) -> bool {
    true
}

fn spawn(mode: UiMode) -> Result<Tray, String> {
    let conn = Connection::session().map_err(|e| e.to_string())?;
    let service = format!("org.kde.StatusNotifierItem-{}-1", std::process::id());
    // 阻塞 API 的 `request_name` 成功即"拿到了这个名字"；被别人占着就是 Err
    conn.request_name(service.as_str())
        .map_err(|e| e.to_string())?;
    let state = Arc::new(Mutex::new(mode));
    let server = conn.object_server();
    server.at(ITEM_PATH, Sni).map_err(|e| e.to_string())?;
    server
        .at(
            MENU_PATH,
            Menu {
                state: Arc::clone(&state),
            },
        )
        .map_err(|e| e.to_string())?;
    // 显式放开对象服务器的借用：下面还要把 `conn` 移进 `Tray` 里养着
    drop(server);
    // 挂到 watcher 上：传服务名，watcher 会回来找 `/StatusNotifierItem`
    conn.call_method(
        Some(WATCHER),
        WATCHER_PATH,
        Some(WATCHER),
        "RegisterStatusNotifierItem",
        &(service.as_str(),),
    )
    .map_err(|e| e.to_string())?;
    Ok(Tray { state, _conn: conn })
}

/// `org.kde.StatusNotifierItem` 那一面。只做到"显示 + 左键唤回"，右键菜单由 Menu 承担。
struct Sni;

#[zbus::interface(name = "org.kde.StatusNotifierItem")]
impl Sni {
    #[zbus(property)]
    fn id(&self) -> &str {
        "ice-translator"
    }

    #[zbus(property)]
    fn category(&self) -> &str {
        "ApplicationStatus"
    }

    #[zbus(property)]
    fn status(&self) -> &str {
        "Active"
    }

    #[zbus(property)]
    fn title(&self) -> &str {
        TITLE
    }

    #[zbus(property)]
    fn icon_name(&self) -> &str {
        ICON_NAME
    }

    /// 位图交空列表：AppIndicator 扩展会优先用 `IconPixmap`，而一张黑色字形位图在深色面板上
    /// 等于看不见。符号名那条路由主题按前景色上色（同 mac 的模板图），所以这里只留名字。
    #[zbus(property)]
    fn icon_pixmap(&self) -> Vec<(i32, i32, Vec<u8>)> {
        Vec::new()
    }

    #[zbus(property)]
    fn menu(&self) -> OwnedObjectPath {
        OwnedObjectPath::try_from(MENU_PATH).expect("静态路径合法")
    }

    #[zbus(property)]
    fn item_is_menu(&self) -> bool {
        false
    }

    #[zbus(property)]
    fn window_id(&self) -> i32 {
        0
    }

    #[zbus(property)]
    fn icon_theme_path(&self) -> &str {
        ""
    }

    fn activate(&self, _x: i32, _y: i32) {
        tray::mark(Outcome::Reveal);
    }

    fn secondary_activate(&self, _x: i32, _y: i32) {
        tray::mark(Outcome::ToggleMode);
    }

    fn context_menu(&self, _x: i32, _y: i32) {}
}

/// `com.canonical.dbusmenu` 那一面：五项菜单
struct Menu {
    state: Arc<Mutex<UiMode>>,
}

/// 一项的完整布局结构：`(id, 属性, 子项)`
type Props = HashMap<String, Value<'static>>;
type Layout = (i32, Props, Vec<Value<'static>>);

fn props_of(label: Option<&str>) -> Props {
    let mut props: Props = HashMap::new();
    match label {
        Some(label) => {
            props.insert("label".to_string(), Value::from(label.to_string()));
        }
        None => {
            props.insert("children-display".to_string(), Value::from("submenu"));
        }
    }
    props.insert("enabled".to_string(), Value::from(true));
    props.insert("visible".to_string(), Value::from(true));
    props
}

fn layout_of(id: i32, label: &str) -> Layout {
    (id, props_of(Some(label)), Vec::new())
}

#[zbus::interface(name = "com.canonical.dbusmenu")]
impl Menu {
    #[zbus(property)]
    fn version(&self) -> u32 {
        3
    }

    #[zbus(property)]
    fn text_direction(&self) -> &str {
        "ltr"
    }

    #[zbus(property)]
    fn status(&self) -> &str {
        "normal"
    }

    #[zbus(property)]
    fn icon_theme_path(&self) -> &str {
        ""
    }

    /// 点选：只立意图，兑现交给 `App::logic` 的泵（与 mac 的回调同一条纪律）
    fn event(&self, id: i32, event_id: String, _data: OwnedValue, _timestamp: u32) {
        if event_id != "clicked" {
            return;
        }
        if let Some(outcome) = outcome_of(id) {
            tray::mark(outcome);
        }
    }

    /// 菜单结构。五项没有子菜单，`recursion_depth` 按"一次全给"处理。
    fn get_layout(
        &self,
        parent_id: i32,
        _recursion_depth: i32,
        _property_names: Vec<String>,
    ) -> (u32, Layout) {
        let mode = self.state.lock().map(|mode| *mode).unwrap_or(UiMode::Full);
        let titles = tray::menu_titles(mode);
        if parent_id != 0 {
            let label = match parent_id {
                1..=5 => titles[(parent_id - 1) as usize],
                _ => "",
            };
            return (1, layout_of(parent_id, label));
        }
        let children: Vec<Value<'static>> = titles
            .iter()
            .enumerate()
            .map(|(index, label)| {
                let (id, props, kids) = layout_of(index as i32 + 1, label);
                Value::Structure(Structure::from((id, props, kids)))
            })
            .collect();
        (1, (0, props_of(None), children))
    }

    fn get_group_properties(
        &self,
        ids: Vec<i32>,
        _property_names: Vec<String>,
    ) -> Vec<(i32, Props)> {
        let mode = self.state.lock().map(|mode| *mode).unwrap_or(UiMode::Full);
        let titles = tray::menu_titles(mode);
        ids.into_iter()
            .filter_map(|id| match id {
                0 => Some((0, props_of(None))),
                1..=5 => Some((id, props_of(Some(titles[(id - 1) as usize])))),
                _ => None,
            })
            .collect()
    }

    fn about_to_show(&self, _id: i32) -> bool {
        true
    }
}
