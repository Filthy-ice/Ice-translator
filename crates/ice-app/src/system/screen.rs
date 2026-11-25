//! 屏幕可用区探针（CHG-004 REQ-003、FR-5.3）。
//!
//! 吸附要贴的是"去掉菜单栏与 Dock 的那块"，而 egui 拿不到它：`ViewportInfo` 只有
//! `monitor_size`（整块屏幕的逻辑尺寸，含菜单栏与 Dock），winit 0.30 的 `MonitorHandle`
//! 也只暴露 `size()`/`scale_factor()`/`position()`。macOS 上问 AppKit 的
//! `NSScreen.visibleFrame`（它的定义就是可用区），其余平台退回 `monitor_size` 并把上边界
//! 内缩 `MENU_BAR_PX` 当近似值。
//!
//! **两个取法对应两件事**（D-8 之后的分工）：
//! - `work_area`：某**一块屏**的可用区，只有离屏校验（`rescue_offscreen`）用它——
//!   判"这个窗口还在不在屏上"必须按窗口所在的那块屏。
//! - `desktop`：所有屏可用区的**并集**，停靠/吸附/唤出用它。停靠的语义是"藏到桌面外面"，
//!   而只有并集的外缘才是桌面外缘；两块并排放的屏中间那条缝不属于任何外缘，把窗口藏进去
//!   只是藏到另一块屏上（既盖不住、热区也回不来，`released` 还会永远算不出向内的位移）。
//!
//! 坐标换算、并集与选屏是纯函数，可无头单测；只有 `probe()` 碰平台 API。

use std::sync::atomic::{AtomicBool, Ordering};

use eframe::egui::{self, Pos2, Rect, Vec2};

/// 非 macOS 平台对菜单栏高度的近似（pt）。macOS 菜单栏在 1x 缩放下是 25pt。
pub const MENU_BAR_PX: f32 = 25.0;

/// AppKit 坐标系里的一个矩形：原点在**主屏左下角**，y 向上。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RawRect {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

/// 一块屏幕的两个矩形：`frame` 是整块屏，`visible` 是去掉菜单栏与 Dock 的可用区。
/// 两个都是 AppKit 坐标系，换算交给 `flip`。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RawScreen {
    pub frame: RawRect,
    pub visible: RawRect,
}

/// AppKit（左下原点、y 向上）→ egui/winit（左上原点、y 向下）。
///
/// 公式必须与 winit 的一致，否则吸附位置会整体偏移一个屏幕高度：
/// `y_topleft = 主屏高度 - rect.height - rect.origin.y`，x 不变
/// （`winit-0.30.13/src/platform_impl/macos/monitor.rs:401-409`）。
/// `main_height` 取的是**主屏**的整屏高度，与这块矩形落在哪台显示器上无关——winit 那边
/// 用的是 `CGDisplay::main()`，注释明写"屏幕坐标是相对主显示器的"。
pub fn flip(rect: RawRect, main_height: f32) -> Rect {
    Rect::from_min_size(
        Pos2::new(rect.x, main_height - rect.h - rect.y),
        Vec2::new(rect.w, rect.h),
    )
}

/// AppKit 坐标系里的一个**点** → egui/winit 坐标系。
///
/// 与 [`flip`] 同一个原点、同一个基准高度，但矩形那条要减去自身高度、点这条不要：
/// `y_topleft = 主屏高度 - y_cocoa`。光标位置用的是这一条。
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
pub const fn flip_point(cocoa: (f32, f32), main_height: f32) -> Pos2 {
    Pos2::new(cocoa.0, main_height - cocoa.1)
}

/// 窗口管理器套在客户窗外的框边距 `(左, 上, 右, 下)`；`None` = 这一帧还量不出来。
///
/// mac 上 egui 的 `outer_rect` 本来就含标题栏，没有这一层（恒 0）；Linux 上 `outer_rect`
/// 是客户窗，而用户看见、拖的是外面那层框。**测试里恒 0**：无头断言喂的是合成矩形。
pub fn frame_extents() -> Option<(f32, f32, f32, f32)> {
    #[cfg(all(target_os = "linux", not(test)))]
    {
        crate::system::window_x11::frame_extents()
    }
    #[cfg(not(all(target_os = "linux", not(test))))]
    {
        Some((0.0, 0.0, 0.0, 0.0))
    }
}

/// 客户窗矩形 → 连框在内的"看得见"的矩形。`None` = 框还量不出来（调用方这一帧别落位）。
pub fn frame_rect(client: Rect) -> Option<Rect> {
    // Linux 档从 2026-11-26 起**不再有系统装饰**（标题栏自绘，见 `views::chrome`）：
    // 量不到框就是"本来没有框"，直接交回客户窗矩形。mac 那边 `frame_extents` 恒 0，
    // 这条同样成立。
    let Some((left, top, right, bottom)) = frame_extents() else {
        return Some(client);
    };
    Some(Rect::from_min_max(
        Pos2::new(client.min.x - left, client.min.y - top),
        Pos2::new(client.max.x + right, client.max.y + bottom),
    ))
}

/// 上面那个的逆：框矩形的左上角 → 客户窗该去的左上角（`OuterPosition` 收的是客户窗坐标）
pub fn client_origin(frame_min: Pos2) -> Pos2 {
    match frame_extents() {
        Some((left, top, _, _)) => Pos2::new(frame_min.x + left, frame_min.y + top),
        None => frame_min,
    }
}

/// 选出窗口所在屏幕的可用区，返回值已换算到 egui 坐标系。
///
/// 判据是**窗口中心**落在哪块屏的 `frame` 里；一块都不含（窗口被拖到屏幕外，或外接屏
/// 拔掉后旧坐标失效）则退回主屏——AppKit 的 `screens[0]` 就是主显示器。
/// 屏幕列表为空时返回 `None`，由调用方走 `fallback_area`。
pub fn select(screens: &[RawScreen], window: Rect) -> Option<Rect> {
    let main = screens.first()?;
    let main_height = main.frame.h;
    let center = window.center();
    let screen = screens
        .iter()
        .find(|s| flip(s.frame, main_height).contains(center))
        .unwrap_or(main);
    Some(flip(screen.visible, main_height))
}

/// 整个桌面的边界：所有屏幕可用区的**并集**（egui 坐标系）。
///
/// 并集是外接矩形，两块屏之间有缝隙时它也会把缝隙圈进来——这正是停靠要的：
/// 唯一能"藏出去"的边界是桌面的最外缘，接缝不是。屏幕列表为空返回 `None`。
pub fn union(screens: &[RawScreen]) -> Option<Rect> {
    let main_height = screens.first()?.frame.h;
    screens
        .iter()
        .map(|s| flip(s.visible, main_height))
        .reduce(|acc, r| {
            Rect::from_min_max(
                Pos2::new(acc.min.x.min(r.min.x), acc.min.y.min(r.min.y)),
                Pos2::new(acc.max.x.max(r.max.x), acc.max.y.max(r.max.y)),
            )
        })
}

/// 探针不可用时的近似可用区：整块屏幕减去顶部菜单栏。
///
/// `monitor_size` 不带原点，多屏下无法知道这块屏在哪，只能按 (0,0) 起算；Dock 的位置与
/// 自动隐藏状态也拿不到。所以这条路径下的吸附边界是近似值（设计里已记明，精确判定属
/// P8 的跨平台核查 NFR-7）。
pub fn fallback_area(monitor_size: Option<Vec2>) -> Option<Rect> {
    let size = monitor_size?;
    Some(Rect::from_min_max(
        Pos2::new(0.0, MENU_BAR_PX),
        Pos2::new(size.x, size.y),
    ))
}

static FALLBACK_WARNED: AtomicBool = AtomicBool::new(false);

/// 回退只警告一次。屏幕拔掉之后吸附判定会连续触发，逐次打日志会刷屏（`warn_once` 语义）。
fn warn_fallback_once() {
    if !FALLBACK_WARNED.swap(true, Ordering::Relaxed) {
        tracing::warn!(
            menu_bar_px = MENU_BAR_PX,
            "拿不到 AppKit 屏幕可用区，退回 monitor_size 近似值；吸附边界不精确"
        );
    }
}

/// 当前窗口所在屏幕的可用区（egui 坐标系）。
///
/// `window` 传窗口的外框矩形：离屏校验要的是"窗口在屏幕上的位置"，不是内容区。
/// 探针失败或屏幕列表为空时退回近似值，并只警告一次。
/// **停靠相关的路径不要用这个**，用 `desktop`。
pub fn work_area(ctx: &egui::Context, window: Rect) -> Option<Rect> {
    if let Some(area) = probe().and_then(|screens| select(&screens, window)) {
        return Some(area);
    }
    let area = fallback_area(ctx.input(|i| i.viewport().monitor_size))?;
    warn_fallback_once();
    Some(area)
}

/// 整个桌面的边界（egui 坐标系），停靠与吸附用。
///
/// 与 `work_area` 的区别就是不看窗口在哪块屏：停靠的边界是桌面的外缘，
/// 而藏出去的窗口（`x = area.x − w + sliver`）中心必然在所有屏之外，
/// 按屏选会退回主屏、把双屏用户的唤出/收回跳到另一块屏上（D-8）。
/// 探针失败时退回 `fallback_area`，与 `work_area` 共用同一条 warn-once。
pub fn desktop(ctx: &egui::Context) -> Option<Rect> {
    if let Some(area) = probe().and_then(|screens| union(&screens)) {
        return Some(area);
    }
    let area = fallback_area(ctx.input(|i| i.viewport().monitor_size))?;
    warn_fallback_once();
    Some(area)
}

/// 读取所有屏幕的 `frame` 与 `visibleFrame`。列表为空或不在主线程时返回 `None`。
#[cfg(target_os = "macos")]
pub fn probe() -> Option<Vec<RawScreen>> {
    use objc2::MainThreadMarker;
    use objc2_app_kit::NSScreen;
    use objc2_foundation::NSRect;

    const fn raw(rect: NSRect) -> RawRect {
        RawRect {
            x: rect.origin.x as f32,
            y: rect.origin.y as f32,
            w: rect.size.width as f32,
            h: rect.size.height as f32,
        }
    }

    // egui 的 `ui`/`logic` 回调都在主线程上；拿不到 marker 说明调用点错了，按探针失败处理
    let mtm = MainThreadMarker::new()?;
    let screens = NSScreen::screens(mtm);
    let mut out = Vec::with_capacity(screens.count());
    for screen in screens.iter() {
        out.push(RawScreen {
            frame: raw(screen.frame()),
            visible: raw(screen.visibleFrame()),
        });
    }
    (!out.is_empty()).then_some(out)
}

/// 非 macOS 非 Linux：没有可问的平台 API，一律走 `fallback_area`。
#[cfg(not(any(target_os = "macos", target_os = "linux")))]
pub const fn probe() -> Option<Vec<RawScreen>> {
    None
}

/// Linux 档（CHG-017 T-008）：问 X，不问 AppKit。
///
/// 本机实测（2026-11-25，examples/x11_geometry_probe）：
///
/// ```text
/// RandR 1.5，在用的屏 1 块：primary=true x=0 y=0 w=2560 h=1440
/// _NET_WORKAREA: [0, 32, 2560, 1349]
/// ```
///
/// 也就是 design D-9 记下的那个形状：**XWayland 把桌面并集报成一整块虚拟屏**，
/// 顶栏 32px 靠 `_NET_WORKAREA` 才看得出来。所以 Linux 档拿不到"每块屏各自的位置"，
/// 单虚拟屏语义下 `select`/`union` 仍然给出对的结果——这是 XWayland 的边界，不是这里的缺陷。
///
/// 坐标从左上原点换回 `RawScreen` 要的那套左下原点（mac 的形状），换算见 [`x11_geometry`]；
/// 三个公共位（`flip`/`select`/`union`）因此**一行都不用改**。
#[cfg(target_os = "linux")]
pub fn probe() -> Option<Vec<RawScreen>> {
    // 测试里认线程自己的那一份：Linux 上没有 mac `MainThreadMarker` 那种"测试线程拿不到"
    // 的天然隔离，真去问 X 会让"1440×900 的假屏"那类无头断言跟着本机显示器变
    #[cfg(test)]
    if let Some(pinned) = TEST_GEOMETRY.with(|slot| slot.borrow().clone()) {
        return Some(pinned);
    }
    x11_geometry::screens()
}

/// 无头测试给"这一条线程"钉一块假屏（见 [`probe`]）。**线程隔离**：测试各跑各的线程，
/// 一个测试钉的几何不许漏进另一个；生产构建里根本没有这个入口。
#[cfg(all(test, target_os = "linux"))]
pub fn pin_geometry_for_test(screens: Vec<RawScreen>) {
    TEST_GEOMETRY.with(|slot| *slot.borrow_mut() = Some(screens));
}

#[cfg(all(test, target_os = "linux"))]
thread_local! {
    static TEST_GEOMETRY: std::cell::RefCell<Option<Vec<RawScreen>>> =
        const { std::cell::RefCell::new(None) };
}

/// X 那几格几何的读取与缓存（Linux 档，T-008）。
///
/// 为什么带 1 秒缓存：`work_area`/`desktop` 在帧循环里被调，每帧建一条 X 连接
/// 等于把吸附判定的心跳挂在 socket 上；而屏幕布局一秒内的变化人眼看不出，
/// 判据要的只是"别拿拔掉显示器之前的旧坐标"。
#[cfg(target_os = "linux")]
mod x11_geometry {
    use std::sync::Mutex;
    use std::time::{Duration, Instant};

    use x11rb::connection::Connection;
    use x11rb::protocol::randr::ConnectionExt as _;
    use x11rb::protocol::xproto::{AtomEnum, ConnectionExt as _};
    use x11rb::rust_connection::RustConnection;

    use super::{RawRect, RawScreen};

    const TTL: Duration = Duration::from_secs(1);

    /// 顶栏高度（px）：本机实测 `_NET_WORKAREA` 从 y=32 起。只在 `_NET_WORKAREA`
    /// 缺席时当保守值用——不拿 mac 的 25，那是 AppKit 的菜单栏。
    const TOP_BAR_PX: i32 = 32;

    /// X11 的左上原点矩形（root 坐标）
    #[derive(Clone, Copy, Debug, PartialEq)]
    struct Geom {
        x: i32,
        y: i32,
        w: i32,
        h: i32,
    }

    impl Geom {
        /// 交集为空（两块屏与工作区不相交）时返回 None，由调用方改用整块屏
        fn intersect(self, other: Geom) -> Option<Geom> {
            let x = self.x.max(other.x);
            let y = self.y.max(other.y);
            let right = (self.x + self.w).min(other.x + other.w);
            let bottom = (self.y + self.h).min(other.y + other.h);
            (right > x && bottom > y).then_some(Geom {
                x,
                y,
                w: right - x,
                h: bottom - y,
            })
        }

        /// 左上原点 → AppKit 那套左下原点（`flip` 的逆）：`y_cocoa = 基准高 - h - y`
        fn to_raw(self, main_height: i32) -> RawRect {
            let h = self.h as f32;
            RawRect {
                x: self.x as f32,
                y: (main_height - self.h - self.y) as f32,
                w: self.w as f32,
                h,
            }
        }
    }

    struct Cached {
        at: Instant,
        screens: Option<Vec<RawScreen>>,
    }

    static CACHE: Mutex<Option<Cached>> = Mutex::new(None);

    pub(super) fn screens() -> Option<Vec<RawScreen>> {
        if let Ok(cache) = CACHE.lock() {
            if let Some(entry) = cache.as_ref() {
                if entry.at.elapsed() < TTL {
                    return entry.screens.clone();
                }
            }
        }
        let fresh = read().map(into_raw);
        if let Ok(mut cache) = CACHE.lock() {
            *cache = Some(Cached {
                at: Instant::now(),
                screens: fresh.clone(),
            });
        }
        fresh
    }

    /// 连一次 X，把三格全读回来。任何一格失败都落到**更保守的那一格**而不是整条失败：
    /// root 尺寸永远拿得到（它来自连接握手），所以"这台机器有几块屏"至少有一份答案。
    fn read() -> Option<Vec<(Geom, Geom)>> {
        let (conn, screen_num) = x11rb::connect(None).ok()?;
        let screen = &conn.setup().roots[screen_num];
        let root = screen.root;
        let fallback = Geom {
            x: 0,
            y: 0,
            w: i32::from(screen.width_in_pixels),
            h: i32::from(screen.height_in_pixels),
        };
        let mut frames = monitors(&conn, root).unwrap_or_else(|| vec![fallback]);
        if frames.is_empty() {
            frames.push(fallback);
        }
        let work = work_area(&conn, root);
        let out = frames
            .into_iter()
            .map(|frame| {
                let visible = work.and_then(|w| frame.intersect(w)).unwrap_or(Geom {
                    y: frame.y + TOP_BAR_PX.min(frame.h - 1).max(0),
                    h: (frame.h - TOP_BAR_PX).max(1),
                    ..frame
                });
                (frame, visible)
            })
            .collect();
        Some(out)
    }

    /// RandR 1.5 的 monitor 列表。版本不够或扩展不在（老 X 服务器）就交回 `None`，
    /// 由调用方用 root 尺寸兜底——这里**不**退回去解析 crtc，那条路更繁琐且给的是同一份信息。
    fn monitors(conn: &RustConnection, root: u32) -> Option<Vec<Geom>> {
        let version = conn.randr_query_version(1, 5).ok()?.reply().ok()?;
        if (version.major_version, version.minor_version) < (1, 5) {
            return None;
        }
        let reply = conn.randr_get_monitors(root, true).ok()?.reply().ok()?;
        let list: Vec<Geom> = reply
            .monitors
            .iter()
            .map(|m| Geom {
                x: i32::from(m.x),
                y: i32::from(m.y),
                w: i32::from(m.width),
                h: i32::from(m.height),
            })
            .collect();
        // 主屏排第一：`select`/`union` 都拿 `screens[0].frame.h` 当坐标基准，
        // 而 X11 的 root 原点就在主屏左上角
        let primary_first = reply.monitors.iter().position(|m| m.primary).map(|at| {
            let mut list = list.clone();
            let primary = list.remove(at);
            list.insert(0, primary);
            list
        });
        Some(primary_first.unwrap_or(list))
    }

    /// `_NET_WORKAREA` 第一个桌面的四元组 `[x, y, w, h]`。它在 EWMH 里是**整块桌面**的可用区，
    /// 不是每屏一份，所以调用方拿它和每块屏求交。
    fn work_area(conn: &RustConnection, root: u32) -> Option<Geom> {
        let atom = conn
            .intern_atom(false, b"_NET_WORKAREA")
            .ok()?
            .reply()
            .ok()?
            .atom;
        let prop = conn
            .get_property(false, root, atom, AtomEnum::CARDINAL, 0, 4)
            .ok()?
            .reply()
            .ok()?;
        let values: Vec<u32> = prop.value32()?.collect();
        let [x, y, w, h] = values.get(0..4)? else {
            return None;
        };
        (w > &0 && h > &0).then(|| Geom {
            x: i32::try_from(*x).unwrap_or(0),
            y: i32::try_from(*y).unwrap_or(0),
            w: i32::try_from(*w).unwrap_or(0),
            h: i32::try_from(*h).unwrap_or(0),
        })
    }

    /// 基准高取第一块（主屏）的整屏高：`select`/`union` 都按 `screens[0].frame.h` 换算
    fn into_raw(pairs: Vec<(Geom, Geom)>) -> Vec<RawScreen> {
        let Some((first, _)) = pairs.first() else {
            return Vec::new();
        };
        let main_height = first.h;
        pairs
            .into_iter()
            .map(|(frame, visible)| RawScreen {
                frame: frame.to_raw(main_height),
                visible: visible.to_raw(main_height),
            })
            .collect()
    }

    #[cfg(test)]
    pub(super) mod tests {
        use super::*;

        /// 一块屏 + 全局工作区 → `read()` 之后那张 `RawScreen` 的形状。
        /// 走的是同一对 `intersect` / `into_raw`，所以断言的是真换算而不是抄一遍公式。
        pub fn raw_pair(frame_w: i32, frame_h: i32, work: (i32, i32, i32, i32)) -> RawScreen {
            let frame = Geom {
                x: 0,
                y: 0,
                w: frame_w,
                h: frame_h,
            };
            let work = Geom {
                x: work.0,
                y: work.1,
                w: work.2,
                h: work.3,
            };
            let visible = frame.intersect(work).unwrap_or(Geom {
                y: frame.y + TOP_BAR_PX.min(frame.h - 1).max(0),
                h: (frame.h - TOP_BAR_PX).max(1),
                ..frame
            });
            into_raw(vec![(frame, visible)]).remove(0)
        }

        /// 每块屏与全局工作区的交集（None = 不相交）
        pub fn slices(
            frames: &[(i32, i32, i32, i32)],
            work: (i32, i32, i32, i32),
        ) -> Vec<Option<(i32, i32, i32, i32)>> {
            let work = Geom {
                x: work.0,
                y: work.1,
                w: work.2,
                h: work.3,
            };
            frames
                .iter()
                .map(|f| {
                    let frame = Geom {
                        x: f.0,
                        y: f.1,
                        w: f.2,
                        h: f.3,
                    };
                    frame.intersect(work).map(|g| (g.x, g.y, g.w, g.h))
                })
                .collect()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const fn raw(x: f32, y: f32, w: f32, h: f32) -> RawRect {
        RawRect { x, y, w, h }
    }

    /// 主屏 1440×900，菜单栏 25pt 在上、Dock 70pt 在下（AppKit 坐标系里 Dock 占 y∈[0,70)）
    fn main_screen() -> RawScreen {
        RawScreen {
            frame: raw(0.0, 0.0, 1440.0, 900.0),
            visible: raw(0.0, 70.0, 1440.0, 805.0),
        }
    }

    #[test]
    fn flipping_the_main_screen_moves_the_origin_to_the_top_left() {
        let area = flip(main_screen().visible, 900.0);
        assert_eq!(area.min, Pos2::new(0.0, 25.0), "菜单栏在上方 25pt");
        assert_eq!(area.max, Pos2::new(1440.0, 830.0), "Dock 在下方 70pt");
    }

    /// 整块屏幕（没有菜单栏也没有 Dock 时）翻转后应当正好铺满 (0,0)-(w,h)
    #[test]
    fn flipping_a_full_frame_covers_the_screen() {
        let area = flip(main_screen().frame, 900.0);
        assert_eq!(
            area,
            Rect::from_min_size(Pos2::ZERO, Vec2::new(1440.0, 900.0))
        );
    }

    /// 副屏在主屏右侧且更高：AppKit 的 y 是 0，翻转后 top 是**负数**。
    /// 这条锁的是"`main_height` 取主屏高度而不是这块屏自己的高度"——取错就会整体偏移 180px。
    #[test]
    fn a_taller_secondary_screen_to_the_right_gets_a_negative_top() {
        let secondary = RawScreen {
            frame: raw(1440.0, 0.0, 1920.0, 1080.0),
            visible: raw(1440.0, 70.0, 1920.0, 985.0),
        };
        let frame = flip(secondary.frame, 900.0);
        assert_eq!(frame.min, Pos2::new(1440.0, -180.0));
        assert_eq!(frame.max, Pos2::new(3360.0, 900.0));

        let area = flip(secondary.visible, 900.0);
        assert_eq!(
            area.min,
            Pos2::new(1440.0, -155.0),
            "副屏菜单栏在它自己的顶部：整屏 top -180 再加 25pt"
        );
        assert_eq!(area.max, Pos2::new(3360.0, 830.0), "Dock 仍在它自己的底部");
    }

    /// 副屏在主屏下方：AppKit 的 y 是负的，翻转后 top 正好接在主屏底边
    #[test]
    fn a_secondary_screen_below_the_main_one_lands_under_it() {
        let below = RawScreen {
            frame: raw(0.0, -1080.0, 1920.0, 1080.0),
            visible: raw(0.0, -1080.0, 1920.0, 1055.0),
        };
        let frame = flip(below.frame, 900.0);
        assert_eq!(frame.min, Pos2::new(0.0, 900.0));
        assert_eq!(frame.max, Pos2::new(1920.0, 1980.0));

        let area = flip(below.visible, 900.0);
        assert_eq!(area.min, Pos2::new(0.0, 925.0), "菜单栏在副屏自己的顶部");
    }

    /// 窗口中心落在哪块屏，就用哪块屏的可用区（多屏吸附的前提）
    #[test]
    fn the_screen_containing_the_window_center_wins() {
        let secondary = RawScreen {
            frame: raw(1440.0, 0.0, 1920.0, 1080.0),
            visible: raw(1440.0, 70.0, 1920.0, 985.0),
        };
        let screens = [main_screen(), secondary];

        let on_main = Rect::from_min_size(Pos2::new(200.0, 300.0), Vec2::new(460.0, 96.0));
        assert_eq!(
            select(&screens, on_main).unwrap().max,
            Pos2::new(1440.0, 830.0)
        );

        let on_secondary = Rect::from_min_size(Pos2::new(2000.0, 100.0), Vec2::new(460.0, 96.0));
        assert_eq!(
            select(&screens, on_secondary).unwrap().max,
            Pos2::new(3360.0, 830.0)
        );
    }

    /// 窗口被拖到所有屏幕之外（外接屏拔掉后的旧坐标就是这种）：退回主屏，不能返回 `None`
    /// 让调用方无路可走——REQ-009 要求"不把窗口弄丢"
    #[test]
    fn a_window_outside_every_screen_falls_back_to_the_main_one() {
        let screens = [main_screen()];
        let off = Rect::from_min_size(Pos2::new(5000.0, 4000.0), Vec2::new(460.0, 96.0));
        let area = select(&screens, off).unwrap();
        assert_eq!(area.min, Pos2::new(0.0, 25.0));
        assert_eq!(area.max, Pos2::new(1440.0, 830.0));
    }

    #[test]
    fn an_empty_screen_list_selects_nothing() {
        let window = Rect::from_min_size(Pos2::ZERO, Vec2::new(460.0, 96.0));
        assert_eq!(select(&[], window), None);
    }

    /// 光标是一个**点**，翻转不减高度：主屏左下角 (0,0) 在 egui 里是 (0, 主屏高)，
    /// 左上角 (0,900) 是 (0,0)。停靠的"指针在不在窗口里"就靠这一条搬到同一套坐标。
    #[test]
    fn a_point_flips_without_subtracting_its_own_height() {
        assert_eq!(flip_point((0.0, 0.0), 900.0), Pos2::new(0.0, 900.0));
        assert_eq!(flip_point((0.0, 900.0), 900.0), Pos2::new(0.0, 0.0));
        assert_eq!(flip_point((700.0, 300.0), 900.0), Pos2::new(700.0, 600.0));
    }

    #[test]
    fn the_fallback_area_subtracts_an_approximate_menu_bar() {
        let area = fallback_area(Some(Vec2::new(1440.0, 900.0))).unwrap();
        assert_eq!(area.min, Pos2::new(0.0, MENU_BAR_PX));
        assert_eq!(area.max, Pos2::new(1440.0, 900.0));
        assert_eq!(fallback_area(None), None, "拿不到屏幕尺寸时不猜");
    }

    /// D-8 与"接缝不吸附"的共同根因：两块并排放的屏，桌面外缘是**最左与最右**，
    /// 中间那条缝不属于任何外缘。并集给出的正是这两个外缘，
    /// 于是 `snap` 再也看不到 x=0 这条线，藏不进另一块屏、也解不开的死状态就不会产生。
    #[test]
    fn the_desktop_union_spans_both_displays_and_skips_the_seam() {
        let main_right = RawScreen {
            frame: raw(0.0, 0.0, 1920.0, 1080.0),
            visible: raw(0.0, 25.0, 1920.0, 1055.0),
        };
        let left = RawScreen {
            frame: raw(-1920.0, 0.0, 1920.0, 1080.0),
            visible: raw(-1920.0, 25.0, 1920.0, 1055.0),
        };
        let area = union(&[main_right, left]).unwrap();
        assert_eq!(area.min.x, -1920.0, "桌面左外缘在副屏那边");
        assert_eq!(area.max.x, 1920.0, "桌面右外缘在主屏那边");
        assert_eq!(area.min.y, 0.0, "两块屏的顶齐平（主屏高度为翻转基准）");
        assert_eq!(area.max.y, 1055.0);
    }

    /// 主屏更高、副屏更矮时，并集的 y 取两者之外沿：交叉轴夹取只会更松，不会把窗口夹丢
    #[test]
    fn a_shorter_neighbour_widens_the_union_without_losing_an_edge() {
        let main = main_screen(); // 1440×900，可见区 egui y 25..830
        let small = RawScreen {
            frame: raw(1440.0, 0.0, 1024.0, 768.0),
            visible: raw(1440.0, 0.0, 1024.0, 743.0),
        };
        let area = union(&[main, small]).unwrap();
        assert_eq!(area.min.x, 0.0);
        assert_eq!(area.max.x, 2464.0, "右外缘在那块矮屏上");
        assert_eq!(area.min.y, 25.0);
        assert_eq!(area.max.y, 900.0, "矮屏没有 Dock，它的底边就是主屏底边");
    }

    #[test]
    fn a_single_display_unions_to_its_own_visible_area() {
        let main = main_screen();
        assert_eq!(union(&[main]), Some(flip(main.visible, 900.0)));
        assert_eq!(union(&[]), None);
    }

    // ---- Linux 档：X11 左上原点 → RawScreen 的换算（T-008）----
    // 数字就是本机实测那一组（examples/x11_geometry_probe，2026-11-25）：
    // 一块 2560×1440 的虚拟屏，`_NET_WORKAREA = [0, 32, 2560, 1349]`。

    #[cfg(target_os = "linux")]
    #[test]
    fn the_measured_x11_rects_flip_back_to_the_numbers_we_saw() {
        use super::x11_geometry::tests::*;
        let screens = raw_pair(2560, 1440, (0, 32, 2560, 1349));
        assert_eq!(
            flip(screens.frame, 1440.0),
            Rect::from_min_size(Pos2::ZERO, Vec2::new(2560.0, 1440.0)),
            "整屏那一格必须原样翻回去"
        );
        let visible = flip(screens.visible, 1440.0);
        assert_eq!(visible.min, Pos2::new(0.0, 32.0), "顶栏 32px 是实测的");
        assert_eq!(
            visible.max,
            Pos2::new(2560.0, 1381.0),
            "底边 1440-32-1349=59"
        );
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn a_work_area_off_screen_leaves_the_frame_minus_the_top_bar() {
        use super::x11_geometry::tests::*;
        // 工作区与这块屏不相交（例如工作区只报了副屏）：不许把可见区算成空的
        let screens = raw_pair(1920, 1080, (1920, 0, 1920, 1080));
        let visible = flip(screens.visible, 1080.0);
        assert_eq!(visible.min.y, 32.0);
        assert_eq!(visible.max.y, 1080.0);
        assert_eq!(visible.max.x, 1920.0, "不相交时按整屏宽度算");
    }

    /// 真机冒烟：本机真去问一次 X。有 X 才跑（返回 None 就跳过），断言的是形状而不是数字——
    /// 数字随显示器变，形状（可见区必须落在整屏里）不该变。
    #[cfg(target_os = "linux")]
    #[test]
    fn the_live_x11_probe_keeps_every_visible_rect_inside_its_frame() {
        let Some(screens) = probe() else {
            return; // headless：没有 X 就没有这一格
        };
        assert!(!screens.is_empty());
        let main_height = screens[0].frame.h;
        for screen in &screens {
            let frame = flip(screen.frame, main_height);
            let visible = flip(screen.visible, main_height);
            assert!(
                frame.contains_rect(visible),
                "可用区不在这块屏里：frame={frame:?} visible={visible:?}"
            );
        }
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn two_real_x11_monitors_get_each_their_own_slice_of_the_work_area() {
        use super::x11_geometry::tests::*;
        // 真 X11 双屏（不是本机这台 XWayland）：工作区是整块桌面的
        let slices = slices(
            &[(0, 0, 1920, 1080), (1920, 0, 2560, 1440)],
            (0, 32, 4480, 1408),
        );
        assert_eq!(slices[0], Some((0, 32, 1920, 1048)));
        assert_eq!(slices[1], Some((1920, 32, 2560, 1408)));
    }
}
