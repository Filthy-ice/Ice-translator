//! 标题栏双击观测（任务 #25，用户"双击标题栏：铺满整屏 + 再双击还原"）。
//!
//! 双击标题栏这件事**在 egui 里收不到**：两种形态都带原生标题栏（Q-013），那条 32pt 高的
//! 带子是 AppKit 的 `NSTitlebarView` 在画，它自己接管 `mouseDown` 用来拖窗（同一颗带子让
//! 画进标题栏的控件点不动——标题栏里那颗原生图钉就是靠这条路才收得到点击，2026-10-29 已连
//! 根撤掉）。系统默认动作还由用户在
//! 系统设置里选的档决定（默认常常是"最小化"），那不是我们能改的公开 API。
//!
//! 公开 API 里唯一能既看到又拦下这一次点击的位置，是
//! `NSEvent::addLocalMonitorForEventsMatchingMask_handler:`——本地监视器在 AppKit 派发
//! **之前**收到本进程的事件，返回 `null` 就是把这一次派发取消。做法因此是**只看双击、只吞双击**，
//! 且只吞"标题栏中间那段"：
//!
//! - 单击、拖动、`mouseMoved` 一律不碰（掩码只有 LeftMouseDown / LeftMouseUp），拖窗那条路
//!   留在原处；
//! - `clickCount >= 2` 才算，且必须落在 [`title_band`] 算出的带子里——左端扣掉红绿灯
//!   （那三颗也是按钮，吞掉它们第二下的 `mouseDown` 就变成"关不掉窗口"），右端只留
//!   [`EDGE_PAD`] 一道余量（标题栏里的原生 accessory 图钉 2026-10-29 撤了，右边缘不再有
//!   会收点击的东西），下边界取内容区
//!   的顶，**内容区里的任何双击（译文里连点两下选词、输入框里双击粘贴）都不受影响**；
//! - 只认装监视器那一刻记下的那颗窗口（[`WATCHED`] 比 `event.window()` 的裸指针）。
//!
//! 代价写清楚：**双击之后立刻按住拖动**这条路会失效（第二下的按下事件被吞，拖拽不会开始）。
//! 这是拦下游走必经的取舍；真机上如果觉得碍事，回退方案见 `06-test-cases.md` W-2.37。
//!
//! 监视器只在**全量窗**装着（`sync` 的 `want` 由帧循环按形态给）：悬浮窗是贴边长条，铺满整屏
//! 不是它该做的事。取不到原生窗口时返回 [`Outcome::Unavailable`]，界面行为退回系统默认，
//! 不假装有效。与 `super::titlebar` 同一纪律：每帧核对现状，只在结果变化时
//! 由调用方记日志。

use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

/// 带子两端各留的余量（pt）：贴着红绿灯边沿的那几像素不算"标题栏中间"。
/// 判据卡在毫厘之间出的就是"那颗按钮点不动"这类查不出来的缺陷。
pub const EDGE_PAD: f64 = 6.0;

/// 标题栏里合格的双击落点区间（**窗口坐标系**：原点在左下角，y 向上）。
///
/// 之所以是四个数而不是"高度 + 两个边距"：上边界是窗口顶、下边界是内容区顶，两者都来自
/// 当帧问出来的真实几何。将来若改成无边框或 `fullSizeContentView`，这条带子自己会塌成
/// 空区间，判据跟着失效而不是继续误吞事件。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Band {
    pub left: f64,
    pub right: f64,
    pub bottom: f64,
    pub top: f64,
}

/// 算出那颗窗口的标题栏带子。
///
/// `window_w / window_h` 是**外框**尺寸，`content_h` 是内容视图的高度（外框减标题栏），
/// `lights_right` 是红绿灯整组的最右边缘。右端不再让格子出去：标题栏里那颗原生图钉
/// 2026-10-29 挪进了 egui 工具条（用户指定放在「悬浮窗」二字左侧），那条 accessory 撤了，
/// 右边缘到窗口边界之间再没有会收点击的东西。
///
/// 带子可能是**空**的（窗宽比红绿灯禁区还窄，或无边框时 `content_h == window_h`）：空区间让
/// [`in_band`] 恒假，也就是"什么都不吞"——判不过的方向是保守的。
pub const fn title_band(window_w: f64, window_h: f64, content_h: f64, lights_right: f64) -> Band {
    Band {
        left: lights_right + EDGE_PAD,
        right: window_w - EDGE_PAD,
        bottom: content_h,
        top: window_h,
    }
}

/// 这一点是不是"标题栏中间的双击落点"。
///
/// 空带子（零高或零宽）一次都不含：贴着边线的那一像素算在里面，但"上下重合"或"左右交叉"
/// 说明判据本身已经不成立（无边框、窗宽小于两端禁区之和），此时什么都不吞才是安全方向。
pub const fn in_band(x: f64, y: f64, band: Band) -> bool {
    band.left < band.right
        && band.bottom < band.top
        && x >= band.left
        && x <= band.right
        && y >= band.bottom
        && y <= band.top
}

/// 一次合格的标题栏双击。写它的是本地监视器里的 block（主线程、AppKit 派发路径上），
/// 读它的是帧循环；用原子量只是为了让这两处代码分属两个模块，并让测试能模拟一次双击。
static DOUBLED: AtomicBool = AtomicBool::new(false);

/// 装监视器那一刻记下的那颗窗口（裸指针），0 表示没装。
/// 只在主线程读写；用原子量是为了让那个 block 不必捕获任何 Rust 引用。
static WATCHED: AtomicUsize = AtomicUsize::new(0);

/// 记一次合格双击。ObjC 侧的 block 调它，测试也从这里模拟。
pub fn note_double_click() {
    DOUBLED.store(true, Ordering::SeqCst);
}

/// 取走"标题栏被双击了"这个事实。一帧内多次只算一次：第二下的时候铺满请求已经在路上了。
pub fn take_double_click() -> bool {
    DOUBLED.swap(false, Ordering::SeqCst)
}

/// 这颗窗口是不是当前被盯着的那颗。
fn is_watched(ptr: usize) -> bool {
    ptr != 0 && WATCHED.load(Ordering::SeqCst) == ptr
}

/// 这一帧该不该盯着标题栏双击（与 `titlebar::Outcome` 同一形状，方便调用方按变化记日志）
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Outcome {
    /// 监视器已装上：标题栏中间的双击归我们
    Watching,
    /// 监视器已撤下（悬浮窗）：双击回到系统默认
    Idle,
    /// 这条路走不通（非 macOS、拿不到窗口句柄、装不上监视器）
    Unavailable,
}

#[cfg(target_os = "macos")]
mod macos {
    use std::cell::RefCell;
    use std::ffi::c_void;
    use std::ptr::{NonNull, null_mut};
    use std::sync::atomic::Ordering;

    use block2::RcBlock;
    use objc2::MainThreadMarker;
    use objc2::rc::Retained;
    use objc2::runtime::AnyObject;
    use objc2_app_kit::{NSEvent, NSEventMask, NSView, NSWindow, NSWindowButton};

    use super::{Band, Outcome, WATCHED, in_band, is_watched, note_double_click, title_band};

    /// 红绿灯是三颗独立的按钮，禁区取它们的最右边缘
    const TRAFFIC: [NSWindowButton; 3] = [
        NSWindowButton::CloseButton,
        NSWindowButton::MiniaturizeButton,
        NSWindowButton::ZoomButton,
    ];

    struct Watch {
        /// AppKit 还给我们的凭据，`removeMonitor:` 要它
        token: Retained<AnyObject>,
        /// block 必须由我们持有：`addLocalMonitor...` **不** retain 传进去的 block，
        /// 一放手 ObjC 侧剩的就是野 block（与 `titlebar::Wired` 存住 target 同一条理由）
        _handler: RcBlock<dyn Fn(NonNull<NSEvent>) -> *mut NSEvent>,
    }

    thread_local! {
        static WATCH: RefCell<Option<Watch>> = const { RefCell::new(None) };
    }

    /// 红绿灯整组的最右边缘（窗口坐标系的 x：标题栏视图与窗口同 x 原点，所以按钮自己
    /// 坐标系里的 x 可以直接比）。问不到就当 0——禁区只剩 `EDGE_PAD`，方向是**少吞**。
    fn lights_right(window: &NSWindow) -> f64 {
        TRAFFIC
            .iter()
            .filter_map(|button| window.standardWindowButton(*button))
            .map(|button| {
                let frame = button.frame();
                frame.origin.x + frame.size.width
            })
            .fold(0.0, f64::max)
    }

    /// 当帧问出来的带子。内容视图还没有（第一帧）时返回 `None`，本帧什么都不吞。
    fn band_of(window: &NSWindow) -> Option<Band> {
        let frame = window.frame();
        let content: Retained<NSView> = window.contentView()?;
        Some(title_band(
            frame.size.width,
            frame.size.height,
            content.frame().size.height,
            lights_right(window),
        ))
    }

    /// block 体：判定落点，命中就记一笔并吞掉这次派发。
    ///
    /// 这里**不许 panic**——它跑在 ObjC 的调用栈上，展开跨过 FFI 边界没有定义。所以整段
    /// 只发消息、比数值：不索引、不 `unwrap`、不做会溢出或除零的算术。取不到主线程标记
    /// 就当"不是我们的双击"（fail-closed 的方向是不吞，宁可让系统默认动作演一遍）。
    fn qualifies(raw: NonNull<NSEvent>) -> bool {
        // SAFETY: 本地监视器给出的指针在本次调用期间有效，只读、不留存。
        let event = unsafe { raw.as_ref() };
        if event.clickCount() < 2 {
            return false;
        }
        // `NSEvent::window:` 要主线程标记：事件派发本身就在主线程上，拿不到说明这条路径
        // 被挪到了别的线程，那种情况下不判比误判安全。
        let Some(mtm) = MainThreadMarker::new() else {
            return false;
        };
        let Some(window) = event.window(mtm) else {
            return false;
        };
        let watched = Retained::as_ptr(&window).cast::<c_void>() as usize;
        if !is_watched(watched) {
            return false;
        }
        let location = event.locationInWindow();
        band_of(&window).is_some_and(|band| in_band(location.x, location.y, band))
    }

    pub fn sync(frame: &eframe::Frame, want: bool) -> Outcome {
        let Some(window) = crate::system::window::ns_window(frame) else {
            return Outcome::Unavailable;
        };
        let ptr = Retained::as_ptr(&window).cast::<c_void>() as usize;
        let installed = WATCH.with(|watch| watch.borrow().is_some());

        if !want {
            if installed {
                uninstall();
            }
            return Outcome::Idle;
        }
        if installed {
            if is_watched(ptr) {
                return Outcome::Watching;
            }
            // 窗口换了（不该发生，但按现状而不是按记忆判定才有这一支）：先撤再装
            uninstall();
        }
        install(ptr)
    }

    fn install(ptr: usize) -> Outcome {
        let mask = NSEventMask::LeftMouseDown | NSEventMask::LeftMouseUp;
        let handler: RcBlock<dyn Fn(NonNull<NSEvent>) -> *mut NSEvent> =
            RcBlock::new(|raw: NonNull<NSEvent>| -> *mut NSEvent {
                if qualifies(raw) {
                    note_double_click();
                    // 返回 null = 这次派发取消：系统的"双击标题栏"动作不会再演一遍
                    return null_mut();
                }
                raw.as_ptr()
            });
        // SAFETY: 类方法，只在主线程调用（`sync` 站在 `ui` 帧路径上）；block 是刚造的
        // `'static` 对象，并由下面的 `WATCH` 强持有到 `removeMonitor:` 之后。
        // 返回 `None` 是文档里允许的结果（AppKit 拒绝登记），不是 panic 的理由。
        let token =
            unsafe { NSEvent::addLocalMonitorForEventsMatchingMask_handler(mask, &handler) };
        let Some(token) = token else {
            // 装不上就不假装有效：指针清零、双击回到系统默认动作。
            WATCHED.store(0, Ordering::SeqCst);
            return Outcome::Unavailable;
        };
        WATCHED.store(ptr, Ordering::SeqCst);
        WATCH.with(|watch| {
            *watch.borrow_mut() = Some(Watch {
                token,
                _handler: handler,
            })
        });
        Outcome::Watching
    }

    fn uninstall() {
        let state = WATCH.with(|watch| watch.borrow_mut().take());
        WATCHED.store(0, Ordering::SeqCst);
        if let Some(watch) = state {
            // SAFETY: `token` 正是 `addLocalMonitor...` 还给我们的那个凭据；`removeMonitor:`
            // 之后 AppKit 不再调用 block，`_handler` 随作用域一起释放是安全的。
            unsafe { NSEvent::removeMonitor(&watch.token) };
        }
    }
}

#[cfg(target_os = "macos")]
pub use macos::sync;

/// 非 macOS 没有可拦的标题栏双击（本应用 macOS 优先，NFR-7 只保证那一侧能编译）。
#[cfg(not(target_os = "macos"))]
pub const fn sync(_frame: &eframe::Frame, _want: bool) -> Outcome {
    Outcome::Unavailable
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 本机观测：带装饰的窗口外框比内容高 32pt（`07-acceptance.md` 的几何基线）
    const CHROME: f64 = 32.0;
    const LIGHTS: f64 = 70.0;

    fn band(w: f64, h: f64) -> Band {
        title_band(w, h, h - CHROME, LIGHTS)
    }

    /// 正中间那一下算——铺满/还原的触发点就是这个
    #[test]
    fn a_click_in_the_middle_of_the_title_bar_counts() {
        let band = band(1920.0, 1080.0);
        assert_eq!(band.bottom, 1048.0, "下边界是内容区的顶，不是窗口中线");
        assert_eq!(band.top, 1080.0);
        assert!(in_band(960.0, 1064.0, band));
        assert!(in_band(band.left, 1048.0, band), "边界本身算在里面");
        assert!(in_band(band.right, 1080.0, band));
    }

    /// 红绿灯那三颗与贴边那几像素不算：吞掉第二下的 `mouseDown` 会让它们点不动
    #[test]
    fn clicks_over_the_buttons_are_left_alone() {
        let band = band(1920.0, 1080.0);
        assert!(!in_band(12.0, 1064.0, band), "关闭那颗");
        assert!(!in_band(68.0, 1064.0, band), "绿灯贴着禁区边，也不吞");
        assert_eq!(
            band.right, 1_914.0,
            "右端只剩一道 `EDGE_PAD`，标题栏里已没有别的控件"
        );
        assert!(
            !in_band(1_919.0, 1064.0, band),
            "贴着窗口右边缘那一下留给拖窗口"
        );
    }

    /// 内容区里的一切都不受影响（译文里连点两下选词、输入框里双击粘贴）
    #[test]
    fn nothing_inside_the_content_area_is_swallowed() {
        let band = band(1920.0, 1080.0);
        assert!(!in_band(960.0, 1047.0, band), "差 1pt 进了内容区，就不许吞");
        assert!(!in_band(960.0, 500.0, band));
        assert!(!in_band(960.0, 0.0, band));
    }

    /// 判不过的方向必须是"什么都不吞"：窗宽小于两端禁区之和、或没有标题栏
    /// （`content_h == window_h`）时带子是空的，一次也命中不了
    #[test]
    fn an_impossible_band_swallows_nothing() {
        let too_narrow = band(80.0, 400.0);
        assert!(too_narrow.right < too_narrow.left, "两端禁区已经重叠");
        for x in [0.0, 40.0, 74.0, 1_000.0] {
            assert!(!in_band(x, 380.0, too_narrow), "{x}");
        }
        let borderless = title_band(1920.0, 1080.0, 1080.0, LIGHTS);
        assert_eq!(borderless.bottom, borderless.top, "没有标题栏带子");
        assert!(!in_band(960.0, 1080.0, borderless));
    }

    /// 一次双击只兑现一次：取走即清零，否则同一刀会铺满又还原
    #[test]
    fn one_double_click_is_handed_over_exactly_once() {
        assert!(
            !take_double_click(),
            "上一处的残留会让第一次双击凭空多切一刀"
        );
        note_double_click();
        note_double_click();
        assert!(take_double_click());
        assert!(!take_double_click());
    }

    /// 监视器只认装它那一刻那颗窗口：别的窗口的事件不判
    #[test]
    fn only_the_watched_window_counts() {
        assert!(!is_watched(0), "0 是'没装'，不能匹配任何指针");
        WATCHED.store(0x1234, Ordering::SeqCst);
        assert!(is_watched(0x1234));
        assert!(!is_watched(0x5678));
        WATCHED.store(0, Ordering::SeqCst);
        assert!(!is_watched(0x1234), "撤下之后残留的指针不能再算数");
    }
}
