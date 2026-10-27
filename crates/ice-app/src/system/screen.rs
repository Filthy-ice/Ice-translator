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

/// 非 macOS 平台没有 `NSScreen`，一律走 `fallback_area`。
#[cfg(not(target_os = "macos"))]
pub const fn probe() -> Option<Vec<RawScreen>> {
    None
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
}
