//! Linux 档的原生窗口命令（CHG-017 T-010 的地基）。
//!
//! 这里只放**同一条 X 连接的取法**与"下一条命令、读回来核对"的原语，业务分档留在
//! `alpha`/`cover`/`mouse`/`windows` 各自的模块里——与 `window.rs`（mac）的定位一致：
//! 取窗口的写法只写一遍，否则出的就是"一边有效一边无效"那类查不出来的缺陷（D-9 同源）。
//!
//! 四条形状是这台机器（GNOME 50 + XWayland，2026-11-25）逼出来的：
//!
//! 1. **从 `Frame` 拿 XID 只认 Xlib/Xcb 两种句柄**。Wayland 原生窗口没有窗口号，
//!    那一档一律 `None`——不猜、不退回"随便找个窗"。
//! 2. **只碰本进程自己的窗**（`_NET_WM_PID` == 我们的 pid）。同一个 root 下坐着整个桌面
//!    的窗，按标题找窗而不校 pid，就等于把别人的窗口全屏化。
//! 3. **连接只建一条、放着复用**。这几个入口在帧循环里被调，每帧建一条 X 连接
//!    （socket + 认证）比命令本身贵两个数量级。连接真断了这台机器也就没有可操作的面了。
//! 4. **下完必须读回来**。写没写成以窗口服务器的现状为准，不以"我们调用过"为准
//!    （与 mac 侧 `alpha`/`titlebar` 同一条纪律）。
#![cfg(target_os = "linux")]

use std::sync::Mutex;
use std::sync::atomic::{AtomicU32, Ordering};

use eframe::egui;
use raw_window_handle::{HasWindowHandle, RawWindowHandle};
use x11rb::connection::Connection;
use x11rb::protocol::xproto::{
    AtomEnum, ClientMessageEvent, ConnectionExt as _, EventMask, PropMode, Window,
};
use x11rb::rust_connection::RustConnection;

/// 缓存的连接与它握手时选定的屏（`default_screen`）
static CONN: Mutex<Option<(RustConnection, usize)>> = Mutex::new(None);

/// 一条命令的通用形状：拿缓存的连接 + root 窗跑一次。
///
/// 闭包返回 `Option`：`None` 可能是"窗不在""属性没有"这类正常答案，也可能是连接断了。
/// 这里不分辨——X 服务器断掉时本进程的窗口已经没了，再重连也没有可操作的对象。
fn with_root<T>(f: impl FnOnce(&RustConnection, Window) -> Option<T>) -> Option<T> {
    // `try_lock` 而不是 `lock`：这把锁**不可重入**，而"在闭包里又调一次公开入口"是很容易
    // 犯的错（第一版就是这么把整条测试挂到超时的）。拿不到锁＝这一刻不问窗口服务器，
    // 交回 `None` 让调用方按"这一帧没下成"处理——帧循环本来就每帧幂等重下
    // （与 `cover_display` 的同一条纪律），而一个桌面程序最不能有的行为是挂住。
    let mut guard = CONN.try_lock().ok()?;
    if guard.is_none() {
        let (conn, screen_num) = x11rb::connect(None).ok()?;
        *guard = Some((conn, screen_num));
    }
    let (conn, screen_num) = guard.as_ref()?;
    let root = conn.setup().roots[*screen_num].root;
    f(conn, root)
}

/// 最近一次见到的主窗 XID。`alpha::set` 每帧都经过 [`xid`]，顺手记下来给门户当 parent_window
/// （`FileChooser` 要一个真实存在的 `x11:0x…`，空串会让后端关联失败）。
static MAIN: AtomicU32 = AtomicU32::new(0);
/// 主窗的 parent_window 标识（门户口径）。还没见过主窗时返回空串（门户那一侧会退回无父窗）。
pub fn parent_window() -> String {
    match MAIN.load(Ordering::Relaxed) {
        0 => String::new(),
        id => format!("x11:{id:#x}"),
    }
}

/// 本进程主窗的 XID。Wayland 原生句柄、或还没建出窗口时返回 `None`。
pub fn xid(frame: &eframe::Frame) -> Option<Window> {
    let window = frame.winit_window()?;
    let handle = window.window_handle().ok()?;
    let window = match handle.as_raw() {
        RawWindowHandle::Xlib(x) => Window::try_from(x.window).ok()?,
        RawWindowHandle::Xcb(x) => x.window.get(),
        _ => return None,
    };
    MAIN.store(window, Ordering::Relaxed);
    Some(window)
}

/// 窗口管理器套在客户窗外面的那层框：`(左, 上, 右, 下)`（像素）。
///
/// **`None` = 这一帧还量不出来**（还没被 reparent 进框里）。GNOME/XWayland 下我们的窗被套进
/// `mutter-x11-frames` 的框：客户窗 460×160、框 510×247、客户窗偏 `+25+62`（本机实测）。
/// 建窗后约 1.4 秒框才出现，这之前的 `QueryTree(client).parent` 还是 root——那时交回 `None`，
/// 让停靠判据等下一帧，而不是按"没有框"算一遍（那一下会把锚点定死，收位永远偏 25px）。
///
/// 0.5 秒缓存：这几个入口都在帧循环里，`QueryTree + 4 次往返` 不该每帧都做。
/// 框边距 `(左, 上, 右, 下)`
type Extents = (f32, f32, f32, f32);

#[cfg_attr(test, allow(dead_code))]
pub fn frame_extents() -> Option<Extents> {
    static CACHE: Mutex<Option<(std::time::Instant, Extents)>> = Mutex::new(None);
    if let Ok(cache) = CACHE.lock() {
        if let Some((at, value)) = cache.as_ref() {
            if at.elapsed() < std::time::Duration::from_millis(500) {
                return Some(*value);
            }
        }
    }
    let measured = measure_extents()?;
    if let Ok(mut cache) = CACHE.lock() {
        *cache = Some((std::time::Instant::now(), measured));
    }
    Some(measured)
}

fn measure_extents() -> Option<Extents> {
    let client = MAIN.load(Ordering::Relaxed);
    if client == 0 {
        return None;
    }
    with_root(|conn, root| {
        let tree = conn.query_tree(client).ok()?.reply().ok()?;
        let parent = tree.parent;
        if parent == root || parent == 0 {
            // 还没套框：**不是"没有框"**，是"还不知道"，交给下一帧
            return None;
        }
        let frame_pos = conn
            .translate_coordinates(parent, root, 0, 0)
            .ok()?
            .reply()
            .ok()?;
        let frame_geom = conn.get_geometry(parent).ok()?.reply().ok()?;
        let client_pos = conn
            .translate_coordinates(client, root, 0, 0)
            .ok()?
            .reply()
            .ok()?;
        let client_geom = conn.get_geometry(client).ok()?.reply().ok()?;
        let left = f32::from(client_pos.dst_x - frame_pos.dst_x);
        let top = f32::from(client_pos.dst_y - frame_pos.dst_y);
        let right = f32::from(frame_geom.width) - f32::from(client_geom.width) - left;
        let bottom = f32::from(frame_geom.height) - f32::from(client_geom.height) - top;
        Some((left.max(0.0), top.max(0.0), right.max(0.0), bottom.max(0.0)))
    })
}

/// 告诉窗口管理器"这扇窗是深色还是浅色"（`_GTK_THEME_VARIANT`）。
///
/// Linux 的标题栏归窗口管理器画（Mutter 按自己的主题配色），我们只改了 egui 那一层；
/// 不打这个属性，会出现实测里那句"标题栏白、界面黑"（2026-11-26 用户报的第 1/2 条）。
/// 每帧调不贵：上次下发的值记着，变了才发。
pub fn set_theme_variant(dark: bool) {
    static LAST: Mutex<Option<bool>> = Mutex::new(None);
    {
        let Ok(mut last) = LAST.lock() else { return };
        if *last == Some(dark) {
            return;
        }
        *last = Some(dark);
    }
    let client = MAIN.load(Ordering::Relaxed);
    if client == 0 {
        return;
    }
    let _ = with_root(|conn, _root| {
        let names = atoms(conn, &["_GTK_THEME_VARIANT", "UTF8_STRING"])?;
        let value: &[u8] = if dark { b"dark" } else { b"light" };
        conn.change_property(
            PropMode::REPLACE,
            client,
            names[0],
            names[1],
            8,
            value.len() as u32,
            value,
        )
        .ok()?;
        conn.flush().ok()?;
        Some(())
    });
}

/// 给窗口套一个圆角形状（X11 的 SHAPE 扩展）。
///
/// 为什么不用"透明像素"：这台机器的 wgpu X11 表面不支持透明合成（启动日志那条
/// `does not support CompositeAlphaMode`），画成透明角也只会显示成黑角。SHAPE 掩码是
/// 窗口服务器层面的裁剪，合成器照样按圆形边缘裁，且与主题无关。
///
/// 只在尺寸变化时重下一次（掩码是按像素画的，尺寸变了必须重画）。
pub fn apply_rounded_corners(radius: u16) {
    use x11rb::protocol::shape::{ConnectionExt as _, SK, SO};
    static LAST: Mutex<Option<(u16, u16, u16)>> = Mutex::new(None);
    let client = MAIN.load(Ordering::Relaxed);
    if client == 0 {
        return;
    }
    let _ = with_root(|conn, _root| {
        let geom = conn.get_geometry(client).ok()?.reply().ok()?;
        let (w, h) = (geom.width, geom.height);
        if w == 0 || h == 0 {
            return None;
        }
        {
            // 只**读**缓存；写必须在真的下成功之后（第一版先写后做：`with_root` 的 try_lock
            // 抢不到锁时也会记成"已应用"，圆角就永远不再重试）
            let Ok(last) = LAST.lock() else {
                return None;
            };
            if *last == Some((w, h, radius)) {
                return Some(());
            }
        }
        let r = radius.min(w / 2).min(h / 2);
        let pixmap = conn.generate_id().ok()?;
        conn.create_pixmap(1, pixmap, client, w, h).ok()?;
        let gc = conn.generate_id().ok()?;
        let aux = x11rb::protocol::xproto::CreateGCAux::new()
            .foreground(0u32)
            .background(0u32);
        conn.create_gc(gc, pixmap, &aux).ok()?;
        // 先整块清零（pixmap 内容未定义），再用前景 1 画出圆角矩形
        conn.poly_fill_rectangle(
            pixmap,
            gc,
            &[x11rb::protocol::xproto::Rectangle {
                x: 0,
                y: 0,
                width: w,
                height: h,
            }],
        )
        .ok()?;
        let set = x11rb::protocol::xproto::ChangeGCAux::new().foreground(1u32);
        conn.change_gc(gc, &set).ok()?;
        conn.poly_fill_rectangle(
            pixmap,
            gc,
            &[
                x11rb::protocol::xproto::Rectangle {
                    x: r as i16,
                    y: 0,
                    width: w.saturating_sub(2 * r),
                    height: h,
                },
                x11rb::protocol::xproto::Rectangle {
                    x: 0,
                    y: r as i16,
                    width: w,
                    height: h.saturating_sub(2 * r),
                },
            ],
        )
        .ok()?;
        let arc = |x: i16, y: i16| x11rb::protocol::xproto::Arc {
            x,
            y,
            width: 2 * r,
            height: 2 * r,
            angle1: 0,
            angle2: 360 * 64,
        };
        let rr = r as i16;
        conn.poly_fill_arc(
            pixmap,
            gc,
            &[
                arc(0, 0),
                arc(w as i16 - 2 * rr, 0),
                arc(0, h as i16 - 2 * rr),
                arc(w as i16 - 2 * rr, h as i16 - 2 * rr),
            ],
        )
        .ok()?;
        conn.shape_mask(SO::SET, SK::BOUNDING, client, 0, 0, pixmap)
            .ok()?;
        conn.shape_mask(SO::SET, SK::CLIP, client, 0, 0, pixmap)
            .ok()?;
        conn.free_pixmap(pixmap).ok()?;
        conn.free_gc(gc).ok()?;
        conn.flush().ok()?;
        if let Ok(mut last) = LAST.lock() {
            *last = Some((w, h, radius));
        }
        Some(())
    });
}

/// 窗口在 root 坐标里的真实矩形 `(x, y, w, h)`。
///
/// `get_geometry` 给的是相对父窗的位置，家具窗（标题栏）一插就偏；所以位置走
/// `translate_coordinates(window → root)`，尺寸才用 `get_geometry`。
pub fn geometry(window: Window) -> Option<(i32, i32, u32, u32)> {
    with_root(|conn, root| geometry_of(conn, root, window))
}

/// 已经握着连接/root 时用它。**`with_root` 那把锁不可重入**：在它的闭包里再调一次
/// [`geometry`] 就是自己等自己（第一版就是这么挂住的，整条测试挂到超时）。
fn geometry_of(
    conn: &RustConnection,
    root: Window,
    window: Window,
) -> Option<(i32, i32, u32, u32)> {
    let geom = conn.get_geometry(window).ok()?.reply().ok()?;
    let pos = conn
        .translate_coordinates(window, root, 0, 0)
        .ok()?
        .reply()
        .ok()?;
    Some((
        i32::from(pos.dst_x),
        i32::from(pos.dst_y),
        u32::from(geom.width),
        u32::from(geom.height),
    ))
}

/// EWMH 的 `_NET_WM_STATE` 一次增删。`on = true` 是 `_NET_WM_STATE_ADD(1)`，否则 REMOVE(0)。
///
/// 消息发到 root、`source indication = 1`（应用自己发起的），这是 EWMH 对客户端的要求；
/// 发给窗口本身窗口管理器收不到。
fn set_state_raw(
    conn: &RustConnection,
    root: Window,
    window: Window,
    state: u32,
    on: bool,
) -> bool {
    let Some(atoms) = atoms(conn, &["_NET_WM_STATE"]) else {
        return false;
    };
    let message = ClientMessageEvent::new(
        32,
        window,
        atoms[0],
        xproto_data([u32::from(on), state, 0, 1, 0]),
    );
    let sent = conn
        .send_event(
            false,
            root,
            EventMask::SUBSTRUCTURE_REDIRECT | EventMask::SUBSTRUCTURE_NOTIFY,
            message,
        )
        .is_ok();
    sent && conn.flush().is_ok()
}

fn xproto_data(values: [u32; 5]) -> x11rb::protocol::xproto::ClientMessageData {
    x11rb::protocol::xproto::ClientMessageData::from(values)
}

/// 读回 `_NET_WM_STATE` 里有没有某个状态。**判断成功与否的唯一依据**。
pub fn has_state(window: Window, state: &str) -> Option<bool> {
    with_root(|conn, _root| {
        let names = atoms(conn, &["_NET_WM_STATE", state])?;
        let prop = conn
            .get_property(false, window, names[0], AtomEnum::ATOM, 0, 64)
            .ok()?
            .reply()
            .ok()?;
        let states: Vec<u32> = prop.value32()?.collect();
        Some(states.contains(&names[1]))
    })
}

/// 加/删一个 `_NET_WM_STATE` 并读回来。返回那一刻**窗口服务器认的**状态。
pub fn set_state(window: Window, state: &str, on: bool) -> bool {
    with_root(|conn, root| {
        let names = atoms(conn, &[state])?;
        if !set_state_raw(conn, root, window, names[0], on) {
            return None;
        }
        Some(())
    });
    has_state(window, state).unwrap_or(false) == on
}

/// 请窗口管理器把焦点给这扇窗（`_NET_ACTIVE_WINDOW`，源标记 1）。返回"发出去了没有"，
/// **成不成要看 [`active_window`]**——激活是异步的，发出去那一刻读到的一般还是旧值。
pub fn activate(window: Window) -> bool {
    with_root(|conn, root| {
        let names = atoms(conn, &["_NET_ACTIVE_WINDOW"])?;
        let message = ClientMessageEvent::new(32, window, names[0], xproto_data([1, 0, 0, 1, 0]));
        let sent = conn
            .send_event(
                false,
                root,
                EventMask::SUBSTRUCTURE_REDIRECT | EventMask::SUBSTRUCTURE_NOTIFY,
                message,
            )
            .is_ok();
        (sent && conn.flush().is_ok()).then_some(())
    })
    .is_some()
}

/// 窗口管理器认的那个"活动窗"
pub fn active_window() -> Option<Window> {
    with_root(|conn, root| {
        let names = atoms(conn, &["_NET_ACTIVE_WINDOW"])?;
        let prop = conn
            .get_property(false, root, names[0], AtomEnum::WINDOW, 0, 1)
            .ok()?
            .reply()
            .ok()?;
        prop.value32()?.next()
    })
}

/// 键盘焦点此刻在哪扇窗上（`GetInputFocus`，够用来回答"esc 进不进得来"）
pub fn input_focus() -> Option<Window> {
    with_root(|conn, _root| {
        let reply = conn.get_input_focus().ok()?.reply().ok()?;
        // 1 = PointerRoot，0 = None：都不是"某扇窗拿着焦点"
        (reply.focus != 0 && reply.focus != 1).then_some(reply.focus)
    })
}

/// 整窗不透明度（`_NET_WM_WINDOW_OPACITY`，32 位 CARDINAL，`0..=0xffff_ffff`）。
/// 返回读回来的那一档；`None` = 写不进去或读不出来。
pub fn opacity(window: Window) -> Option<f32> {
    with_root(|conn, _root| {
        let names = atoms(conn, &["_NET_WM_WINDOW_OPACITY"])?;
        let prop = conn
            .get_property(false, window, names[0], AtomEnum::CARDINAL, 0, 1)
            .ok()?
            .reply()
            .ok()?;
        let raw = prop.value32()?.next()?;
        Some(raw as f32 / u32::MAX as f32)
    })
}

/// 写整窗不透明度并读回来
pub fn set_opacity(window: Window, alpha: f32) -> Option<f32> {
    let raw = (alpha.clamp(0.0, 1.0) * u32::MAX as f32).round() as u32;
    with_root(|conn, _root| {
        let names = atoms(conn, &["_NET_WM_WINDOW_OPACITY"])?;
        conn.change_property(
            PropMode::REPLACE,
            window,
            names[0],
            AtomEnum::CARDINAL,
            32,
            1,
            &raw.to_ne_bytes(),
        )
        .ok()?;
        conn.flush().ok()?;
        Some(())
    });
    opacity(window)
}

/// 指着屏幕的那支光标：root 坐标位置与按键掩码（掩码第 8 位＝左键）。
///
/// 这条比 `hover_pos` 可靠的地方与 mac 的 `+mouseLocation` 同族：它读的是窗口服务器里的
/// 当前状态，与事件投递无关——投递一停，`hover_pos` 就"指针不见了"，而停靠的收回计时
/// 正挂在它身上（D-16 的同一条教训）。
pub fn pointer() -> Option<(i32, i32, u16)> {
    with_root(|conn, root| {
        let reply = conn.query_pointer(root).ok()?.reply().ok()?;
        Some((
            i32::from(reply.root_x),
            i32::from(reply.root_y),
            u16::from(reply.mask),
        ))
    })
}

/// 按键掩码里"左键按着"那一位（X11 的 `Button1Mask`）
pub const BUTTON1_MASK: u16 = 1 << 8;

/// 这扇窗是不是**本进程**的（`_NET_WM_PID`）。只读，用来在动别人的窗之前挡一下。
pub fn owned_by_us(window: Window) -> bool {
    with_root(|conn, _root| Some(pid_of(conn, window) == Some(std::process::id()))).unwrap_or(false)
}

/// 同上：握着连接时用这个，别再进 [`owned_by_us`]（那把锁不可重入）
fn pid_of(conn: &RustConnection, window: Window) -> Option<u32> {
    let names = atoms(conn, &["_NET_WM_PID"])?;
    let prop = conn
        .get_property(false, window, names[0], AtomEnum::CARDINAL, 0, 1)
        .ok()?
        .reply()
        .ok()?;
    prop.value32()?.next()
}

/// 按标题找**本进程自己**的一扇窗。同名取最后一个（与我们刚开的那扇对齐，与 mac 侧
/// `find_by_title` 同一条做法）。
pub fn find_own_window(title: &str) -> Option<Window> {
    with_root(|conn, root| {
        let tree = conn.query_tree(root).ok()?.reply().ok()?;
        let mut found = None;
        let ours = std::process::id();
        for &child in &tree.children {
            if pid_of(conn, child) != Some(ours) {
                continue;
            }
            if read_name(conn, child).as_deref() == Some(title) {
                found = Some(child);
            }
        }
        found
    })
}

/// 从前往后的窗口表（认窗那一格，T-013）。
///
/// 数据源是 `_NET_CLIENT_LIST_STACKING`：EWMH 明令它是**从后往前**的叠放顺序，这里倒过来。
/// 只列这条表里的窗而不是 root 的全部子窗（本机实测 root 下 36 个子窗、这条表只有 2 条）——
/// 子窗里混着窗口管理器的家具与 input-only 窗，没有叠放顺序可用，硬凑出来的顺序会让
/// "鼠标底下那扇"认错（mac 侧"永远命中全屏 Dock 窗"那类症状的 Linux 版）。
///
/// 代价与 mac 侧同一条：**override-redirect 的窗不在表里**（工具提示、某些浮层不被认），
/// 认不到时拖选照常，不是死路。
pub fn stacked_entries() -> Vec<crate::system::windows::Entry> {
    with_root(|conn, root| {
        let names = atoms(conn, &["_NET_CLIENT_LIST_STACKING"])?;
        let prop = conn
            .get_property(false, root, names[0], AtomEnum::WINDOW, 0, 512)
            .ok()?
            .reply()
            .ok()?;
        let mut order: Vec<Window> = prop.value32()?.collect();
        order.reverse(); // EWMH 是从后往前，我们要从前往后
        // 只读这张表会漏窗（本机实测：表里 2 条，root 下 36 个子窗）。漏掉的那些没有叠放
        // 顺序可用，一律排到表尾（＝"在已知窗的下面"）——认窗取的是第一个命中的窗，
        // 顺序错只会让"鼠标底下那扇"退化成下面那扇，不会认错屏外的窗。
        if let Ok(Ok(tree)) = conn.query_tree(root).map(|c| c.reply()) {
            for child in tree.children {
                if !order.contains(&child)
                    && pid_of(conn, child) != Some(std::process::id())
                    && is_normal_window(conn, child)
                {
                    order.push(child);
                }
            }
        }
        let mut out = Vec::new();
        let ours = std::process::id();
        for window in order {
            if pid_of(conn, window) == Some(ours) {
                continue;
            }
            if !is_normal_window(conn, window) {
                continue;
            }
            let Some((x, y, w, h)) = geometry_of(conn, root, window) else {
                continue;
            };
            out.push(crate::system::windows::Entry {
                rect: egui::Rect::from_min_size(
                    egui::pos2(x as f32, y as f32),
                    egui::vec2(w as f32, h as f32),
                ),
                layer: 0,
                owner: class_of(conn, window).unwrap_or_default(),
            });
        }
        Some(out)
    })
    .unwrap_or_default()
}

/// 普通应用窗那一档：`_NET_WM_WINDOW_TYPE` 是 NORMAL / DIALOG / UTILITY（或没写）才算。
/// dock、desktop、toolbar、menu、splash 一律排掉——和 mac 侧"只认层级 0"同一个理由。
fn is_normal_window(conn: &RustConnection, window: Window) -> bool {
    let Some(names) = atoms(
        conn,
        &[
            "_NET_WM_WINDOW_TYPE",
            "_NET_WM_WINDOW_TYPE_NORMAL",
            "_NET_WM_WINDOW_TYPE_DIALOG",
            "_NET_WM_WINDOW_TYPE_UTILITY",
        ],
    ) else {
        return true;
    };
    let Ok(cookie) = conn.get_property(false, window, names[0], AtomEnum::ATOM, 0, 16) else {
        return true;
    };
    let Ok(prop) = cookie.reply() else {
        return true;
    };
    let Some(mut types) = prop.value32() else {
        return true; // 没写类型：按普通窗处理（与窗口管理器一致）
    };
    let accepted = &names[1..];
    types.any(|kind| accepted.contains(&kind))
}

/// `WM_CLASS` 的类名（第二段，即 `Exec` 名那种写法）。取不到就是空串，认窗时会当成别人家的窗。
fn class_of(conn: &RustConnection, window: Window) -> Option<String> {
    let prop = conn
        .get_property(false, window, AtomEnum::WM_CLASS, AtomEnum::STRING, 0, 256)
        .ok()?
        .reply()
        .ok()?;
    let text = String::from_utf8_lossy(&prop.value).into_owned();
    text.split('\0')
        .rfind(|s| !s.is_empty())
        .map(str::to_string)
}

/// 一扇窗的 `_NET_WM_NAME`（失败退回 `WM_NAME`）。只给"按标题找自己的窗"用。
fn read_name(conn: &RustConnection, window: Window) -> Option<String> {
    let names = atoms(conn, &["_NET_WM_NAME", "UTF8_STRING"])?;
    let utf8 = conn
        .get_property(false, window, names[0], names[1], 0, 1024)
        .ok()
        .and_then(|cookie| cookie.reply().ok());
    if let Some(prop) = utf8
        && !prop.value.is_empty()
    {
        return Some(String::from_utf8_lossy(&prop.value).into_owned());
    }
    let prop = conn
        .get_property(false, window, AtomEnum::WM_NAME, AtomEnum::STRING, 0, 1024)
        .ok()?
        .reply()
        .ok()?;
    (!prop.value.is_empty()).then(|| String::from_utf8_lossy(&prop.value).into_owned())
}

/// 把几个 atom 名字一次换成 atom 编号（一次 `InternAtom` 一个，数量固定所以不走批量接口）
fn atoms(conn: &RustConnection, names: &[&str]) -> Option<Vec<u32>> {
    names
        .iter()
        .map(|name| {
            conn.intern_atom(false, name.as_bytes())
                .ok()?
                .reply()
                .ok()
                .map(|r| r.atom)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 缓存连接的那条路在真机上走得通：拿 root 尺寸当探针（不依赖任何窗口）
    #[test]
    fn the_cached_connection_can_answer_a_root_query() {
        let size = with_root(|conn, root| {
            let geom = conn.get_geometry(root).ok()?.reply().ok()?;
            Some((u32::from(geom.width), u32::from(geom.height)))
        });
        let Some((w, h)) = size else {
            return; // headless：没有 X 就没有这一格
        };
        assert!(w > 0 && h > 0);
    }

    /// 我们的窗口一定带着 pid；拿 root 去问必须答"不是我们的"
    #[test]
    fn a_windows_pid_check_never_claims_foreign_windows() {
        // 注意：先放开锁再问（`with_root` 的闭包里再调公开入口就是自己等自己）
        let root = with_root(|_, root| Some(root));
        let Some(root) = root else {
            return; // headless
        };
        assert!(!owned_by_us(root), "root 窗是窗口管理器的，不是我们的");
    }
}
