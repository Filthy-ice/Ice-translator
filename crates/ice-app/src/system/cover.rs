//! 把一扇窗盖到整块屏上（CHG-010 REQ-101 的那一次原生抬层，**全仓唯一出处**）。
//!
//! 为什么要有这个模块，是一条量出来的事实（spec.md 事实 8）：egui 的置顶档是窗口层级 3，
//! 而这台机器上 **Dock 是 20、菜单栏是 24**。于是一扇"置顶"的无边框窗在屏幕下缘会被 Dock
//! 压住一截、在上缘被菜单栏压住一截——对贴图那叫"看起来没置顶"（缺陷 D-30），对截图遮罩
//! 那更严重：遮罩盖不住那两条，"整屏定格、桌面不可操作"这句话就是假的。
//!
//! ## 三条从读数里出来的写法，都不显然
//!
//! 1. **每帧幂等重下，不缓存"我已经抬过了"**。实测：一次 `setLevel:` + `setFrame:` 之后，
//!    AppKit 那侧立刻是新值，而窗口服务器那侧要到**下一轮 runloop**（约 0.01~0.02 秒）才跟上，
//!    而在真实应用里还多一个变量——框架每帧会按它自己的 viewport 设置重新摆一次窗。
//!    这与本仓既有的两条同族（缺陷 D-26 标题栏底色、D-27 Dock 图标："只在启动时下发一次的
//!    原生命令会回到默认"）。所以这里只提供"看一眼、不对就下"的函数，**不返回"已处理过"这种状态**。
//! 2. **抬完必须读回来**，而且读的是**实际给的那一块**，不是我请求的那一块：AppKit 会把没抬层的
//!    窗夹进 `visibleFrame`（实测 `0,63,1920x953`），拿请求值去换算就是"裁出来的图整体偏 63 像素"
//!    那一类查不出根的缺陷（CHG-009 事实 22 同一形状）。读回来的那块交给调用方当**唯一**坐标基准。
//! 3. **不拿 `Fullscreen(true)` 顶替**：那是切一个 Space、带切换动画，为截图这一件事动系统级状态
//!    （REQ-101 明令）。egui 0.37 的 `WindowLevel` 只有三档、`ViewportCommand` 里没有"焦点/更高层级"，
//!    所以这一跳只能落在原生，这里就是它落的地方。
//!
//! ## 服务器侧读数为什么没进实现
//!
//! 理想做法是每帧比对窗口服务器的读数（`CGWindowListCopyWindowInfo`）直到两边一致。那要多开一个
//! crate feature（`objc2-core-graphics/CGWindow`——包体已在依赖图里，但会**改动 `cargo tree` 基线**，
//! 而 V-006 那条依赖零增量的判据正是拿它逐行比对的）。这一版先按"每帧幂等重下 + AppKit 回读"实现，
//! 把"抬了仍然露出 Dock"这一格明确交给走查 **W-6.1**：他真的看见 Dock 压在遮罩上，再回来开那个
//! feature 并同步改 V-006 的基线——**不假装这一半已经验过**。

#[cfg(target_os = "macos")]
use objc2::MainThreadMarker;
#[cfg(target_os = "macos")]
use objc2::rc::Retained;
#[cfg(target_os = "macos")]
use objc2_app_kit::{NSApplication, NSWindow};

/// 一次"盖住整块屏"的请求结果。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Cover {
    /// 窗此刻实际的矩形（**全局点坐标**，AppKit 那侧读回来的）
    pub frame: egui::Rect,
    /// 这一刻是不是已经盖住了（frame 与请求的那块一致即算，层级由系统给）
    pub covers: bool,
}

/// 把标题为 `title` 的那扇窗抬到 Dock 与菜单栏之上，并对齐 `target` 那块屏。
///
/// 幂等：已经对了就只读不回写。**每帧都可以调**。拿不到（非 mac、不在主线程、还没建出那扇窗）
/// 一律 `covers = false`，调用方按"这一档只盖住了可用区"处理并**在界面上说一句**
/// （REQ-101：不许静默当成功）。
#[cfg(target_os = "macos")]
pub fn cover_display(title: &str, target: egui::Rect) -> Cover {
    let Some(mtm) = MainThreadMarker::new() else {
        return Cover {
            frame: target,
            covers: false,
        };
    };
    let Some(window) = find_by_title(&mtm, title) else {
        return Cover {
            frame: target,
            covers: false,
        };
    };
    let want = rect_of(target);
    let now = window.frame();
    let level = cover_level();
    if now != want || window.level() != level {
        // 顺序有意义：先抬层再改框。反过来时 AppKit 会按"普通窗"把新框夹回 visibleFrame，
        // 实测那种做法留下的就是 `0,31,1920x1080` 这种"高对上了、上缘还差一条"的半吊子形状。
        window.setLevel(level);
        window.setFrame_display(want, false);
        window.setLevel(level);
    }
    let after = window.frame();
    Cover {
        frame: egui::Rect::from_min_max(
            egui::pos2(after.origin.x as f32, after.origin.y as f32),
            egui::pos2(
                (after.origin.x + after.size.width) as f32,
                (after.origin.y + after.size.height) as f32,
            ),
        ),
        covers: after == want,
    }
}

#[cfg(not(target_os = "macos"))]
pub fn cover_display(_title: &str, target: egui::Rect) -> Cover {
    // 非 mac 拿不到这一跳。整个截图入口在那一侧本来就是 `Unsupported`（`capability()` 那一处），
    // 这里返回 false 只是保证"任何平台都不许把没做到的说成做到了"。
    Cover {
        frame: target,
        covers: false,
    }
}

/// 把应用提到前台，并把那扇 `title` 窗设为键盘窗——**esc 能不能进来就取决于这一句**
/// （spec.md 事实 9 与 16：不激活时注入的 esc 一个事件都进不来）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Focus {
    pub active: bool,
    pub key: bool,
}

/// 激活是异步的：实测发出去那一刻读到的是 `active=false`，约 0.1 秒后才翻成 true。
/// 所以这个函数**每帧调不贵**，而调用方 SHALL 拿返回值核对，SHALL NOT "发一次就当成功"
/// （REQ-102；与本仓"回调改了状态要自己叫一次循环"那条纪律同族——动作发了不等于状态变了）。
#[cfg(target_os = "macos")]
pub fn focus(title: &str) -> Focus {
    use objc2_app_kit::NSApplicationActivationPolicy;
    let Some(mtm) = MainThreadMarker::new() else {
        return Focus {
            active: false,
            key: false,
        };
    };
    let app = NSApplication::sharedApplication(mtm);
    if app.activationPolicy() != NSApplicationActivationPolicy::Regular {
        app.setActivationPolicy(NSApplicationActivationPolicy::Regular);
    }
    app.activate();
    let key = find_by_title(&mtm, title).is_some_and(|w| {
        if !w.isKeyWindow() {
            w.makeKeyAndOrderFront(None);
        }
        w.isKeyWindow()
    });
    Focus {
        active: app.isActive(),
        key,
    }
}

#[cfg(not(target_os = "macos"))]
pub fn focus(_title: &str) -> Focus {
    Focus {
        active: false,
        key: false,
    }
}

/// 收尾那一下：把应用藏起来，让键盘焦点回到他刚才那个应用。
///
/// **这一格是诚实的未完成**：`hide` 之后焦点到底回不回得去，我在这台机器上没量成
/// （临时探针自己根本没能把自己激活，那次的 `key=false` 不算数据）。所以这里只提供动作，
/// 不承诺"一定还得回去"；量出来是什么就写进什么，写在 W-6.6。
#[cfg(target_os = "macos")]
pub fn step_back() {
    let Some(mtm) = MainThreadMarker::new() else {
        return;
    };
    NSApplication::sharedApplication(mtm).hide(None);
}

#[cfg(not(target_os = "macos"))]
pub fn step_back() {}

#[cfg(target_os = "macos")]
fn find_by_title(
    mtm: &objc2::MainThreadMarker,
    title: &str,
) -> Option<objc2::rc::Retained<objc2_app_kit::NSWindow>> {
    use objc2_app_kit::NSApplication;
    use objc2_foundation::NSString;
    let app = NSApplication::sharedApplication(*mtm);
    let want = NSString::from_str(title);
    // 同名窗取**最后一个**：AppKit 的列表从前往后排，遮罩是我们刚开的那一扇。
    let mut found: Option<Retained<NSWindow>> = None;
    for window in app.windows().iter() {
        if window.title() == want {
            found = Some(window.clone());
        }
    }
    found
}

#[cfg(target_os = "macos")]
fn rect_of(r: egui::Rect) -> objc2_foundation::NSRect {
    // AppKit 的窗口坐标是**左下原点**、全局点坐标（`CGWindowList` 与 egui 是左上原点）。
    // 这里不做任何翻转：传进来的 `target` 已经是"那扇窗该在的 AppKit 矩形"
    // （由 `screen::displays()` 按 AppKit 口径给出）。翻两次就是遮罩跑到别块屏上那种缺陷。
    objc2_foundation::NSRect {
        origin: objc2_foundation::NSPoint {
            x: r.min.x as f64,
            y: r.min.y as f64,
        },
        size: objc2_foundation::NSSize {
            width: r.width() as f64,
            height: r.height() as f64,
        },
    }
}

/// 遮罩该待的窗口层级。**`NSWindowLevel` 在 objc2 里就是 `isize` 的别名**，所以这里直接给数。
///
/// 25 = 菜单栏那一档(24) + 1：实测这一档同时盖得住 Dock(20) 与菜单栏(24)（`layer=25`）。
/// 不用 screenSaver(1000)：那是给屏保的，会把系统自己的浮层一起压下去。
#[cfg(target_os = "macos")]
const fn cover_level() -> objc2_app_kit::NSWindowLevel {
    25
}
