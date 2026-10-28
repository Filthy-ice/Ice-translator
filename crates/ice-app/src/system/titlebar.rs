//! 标题栏绿键改道（用户 2026-10-28："悬浮窗更换全量的按钮可以不要，将标题栏最大化功能
//! 替换为切换全量。"）。
//!
//! 简洁窗动作行里那颗「全量」按钮删掉之后，"看得见的切回全量入口"只剩标题栏那颗绿灯
//! （`NSWindowButton::ZoomButton`）。AppKit 默认把它的 target-action 接到窗口的 zoom /
//! 全屏动作上，而**这一对是可以覆盖的**：取到那颗按钮，`target` 换成运行时用
//! `define_class!` 注册的一个 `NSObject` 子类，`action` 换成只有一个方法的选择器，
//! 方法体只做一件事——把"被点了"记进 `CLICKED`。帧循环下一帧取走它，置
//! `Effects::toggle_mode`，走的仍是 `Cmd+Shift+M`（`Action::ToggleMode`）那一条路径，
//! 不另开第二条切换实现。
//!
//! 四件不显然的事：
//!
//! 1. **每帧从按钮本身读现状**（`action()` / `target()`），一致就什么都不写。
//!    AppKit 会在一些路径上重新接线（窗口成为 key、进出全屏、`updateZoomButton`），
//!    缓存"我接过了"会让改道悄悄失效；而这里的状态是能问出来的，问了比记着可靠。
//!    一次查询是两次消息发送，只在真正画帧的帧里发生（NFR-2 的空闲 1% 不受影响）。
//! 2. **target 必须自己强引用住**。`NSControl.target` 是 `unsafe_unretained`，AppKit 不
//!    retain 它；我们一放手，按钮里剩的就是野指针，点一下是访问已释放对象。所以那个
//!    `Retained<ZoomTarget>` 存在 `WIRED` 里，活到还原那一刻。
//! 3. **只在简洁窗改道，切回全量窗必须还原**——两种形态共用同一个 `NSWindow`。
//!    还原用的是改道前从按钮上读到的那对 target/action，不是猜的默认值；不还原文全量窗
//!    的绿键也变成"切换形态"，而那颗在 full 里本该还是最大化（任务 #25 的双击铺满还要用它）。
//! 4. **悬停提示要一起改**。绿灯的行为变了，AppKit 原本给它的 tooltip 说的是"缩放"，
//!    不改口就是界面在撒谎；AppKit 也会在 hover 时重写 tooltip，所以这一项跟着第 1 条
//!    每帧核对。
//!
//! 取窗口与 `system::alpha` 共用 `super::window::ns_window`。

use std::sync::atomic::{AtomicBool, Ordering};

/// 绿键被点了。写它的是 AppKit 的 action 方法（主线程），读它的是帧循环（同一线程）；
/// 用原子量只是为了让这两处代码能分属两个模块、并让测试能模拟一次点击。
static CLICKED: AtomicBool = AtomicBool::new(false);

/// 这一帧把绿键改道成功没有（只给调用方记日志用，值变了才记，空闲帧不刷屏）
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Outcome {
    /// 归我们：点它是切换形态（简洁窗）
    Hooked,
    /// 交回 AppKit：点它是最大化（全量窗）
    Released,
    /// 这条路走不通（非 macOS、拿不到窗口句柄、这颗窗没有绿键）
    Unavailable,
}

/// 记一次"绿键被点了"。ObjC 侧的 action 方法调它，测试也从这里模拟点击。
pub fn mark_click() {
    CLICKED.store(true, Ordering::SeqCst);
}

/// 取走"绿键被点了"这个事实。一帧内点多次只算一次：第二次点的时候形态已经在切了，
/// 再切回去不是用户要的。
pub fn take_click() -> bool {
    CLICKED.swap(false, Ordering::SeqCst)
}

#[cfg(target_os = "macos")]
mod macos {
    use std::cell::RefCell;
    use std::ffi::c_void;

    use objc2::rc::{Allocated, Retained};
    use objc2::runtime::{AnyObject, Sel};
    use objc2::{AnyThread, define_class, msg_send, sel};
    use objc2_app_kit::{NSButton, NSWindowButton};
    use objc2_foundation::{NSObject, NSString};

    use super::{Outcome, mark_click};

    /// 改道后 `action` 的那颗选择器：读到它就是"现在归我们"的指纹。
    /// （不做 `const`：`Sel::register` 在 objc2 0.6 的默认 feature 下不是 const fn）
    fn ours() -> Sel {
        sel!(iceToggleFullFromTitleBar:)
    }

    /// 绿灯被点时悬停提示说的话（行为变了，文案也得跟着变，否则界面在撒谎）。
    /// 带着快捷键一起说：动作行里那颗「全量」按钮删掉后，这行提示就是"还能用键盘切"的
    /// 唯一现场说明（原来那颗的 hover 就写着这句）。
    const HINT: &str = "切换到全量窗口（Cmd+Shift+M）";

    define_class!(
        /// 绿键的新 target。它只有一个方法，方法体只有 `mark_click()` 一行——
        /// "要不要切换、切成什么"全部留在帧循环里，与快捷键共用同一条 effects 路径。
        #[unsafe(super(NSObject))]
        #[name = "IceTranslatorTitleBarTarget"]
        struct ZoomTarget;

        impl ZoomTarget {
            // 选择器首词 `ice` 不属于任何 Objective-C 方法族，`define_class!` 自己推断；
            // 再手写 `#[unsafe(method_family = none)]` 会和 `#[unsafe(method(...))]` 冲突，直接编译失败。
            #[unsafe(method(iceToggleFullFromTitleBar:))]
            fn toggle_full(&self, _sender: &AnyObject) {
                mark_click();
            }
        }
    );

    impl ZoomTarget {
        /// `alloc` + `init`：没有实例变量，状态全在 `CLICKED` 那个全局原子里
        fn new() -> Retained<Self> {
            let this: Allocated<Self> = Self::alloc();
            let this = this.set_ivars(());
            unsafe { msg_send![super(this), init] }
        }
    }

    /// 改道前那颗按钮自带的 target/action，还原时原样交回去；
    /// `target` 存在的另一个理由是它是那个新对象的**唯一强引用**
    struct Wired {
        target: Retained<ZoomTarget>,
        original_target: Option<Retained<AnyObject>>,
        original_action: Option<Sel>,
    }

    thread_local! {
        static WIRED: RefCell<Option<Wired>> = const { RefCell::new(None) };
    }

    /// 这颗按钮现在是不是已经归我们：action 是我们的那颗选择器，且 target 还是我们那个对象
    fn is_ours(button: &NSButton) -> bool {
        if button.action() != Some(ours()) {
            return false;
        }
        WIRED.with(|wired| {
            wired.borrow().as_ref().is_some_and(|state| {
                let mine = Retained::as_ptr(&state.target).cast::<c_void>();
                button
                    .target()
                    .is_some_and(|t| Retained::as_ptr(&t).cast::<c_void>() == mine)
            })
        })
    }

    /// 记下改道前读到的那对 target/action，再把按钮接到我们自己身上
    fn hook(button: &NSButton) {
        let original_target = button.target();
        let original_action = button.action();
        let target = ZoomTarget::new();
        // SAFETY: `setTarget:` 是 `unsafe_unretained` 赋值，强引用由 `WIRED` 持有，
        // 在按钮的生命周期内不会被释放；`setAction:` 收的是运行时已注册的选择器常量。
        unsafe {
            button.setTarget(Some(&target));
            button.setAction(Some(ours()));
        }
        WIRED.with(|wired| {
            *wired.borrow_mut() = Some(Wired {
                target,
                original_target,
                original_action,
            })
        });
    }

    /// 交回 AppKit：用改道前记下的那一对；没记过（不该发生）就摘干净，
    /// 至少不会留下"target 已经没了"的悬空 action
    fn release(button: &NSButton) {
        let state = WIRED.with(|wired| wired.borrow_mut().take());
        // SAFETY: 传回的 target 是刚取出的那个强引用（在本次调用期内活着），
        // action 是 AppKit 原先自己挂上去的那颗选择器。
        unsafe {
            match state {
                Some(wired) => {
                    button.setTarget(wired.original_target.as_deref());
                    button.setAction(wired.original_action);
                }
                None => {
                    button.setTarget(None);
                    button.setAction(None);
                }
            }
        }
        button.setToolTip(None);
    }

    /// 悬停提示对不上就补一次（AppKit 在 hover 时会把标准按钮的 tooltip 改回"缩放"）
    fn ensure_hint(button: &NSButton) {
        let hint = NSString::from_str(HINT);
        if button.toolTip().is_some_and(|tip| *tip == *hint) {
            return;
        }
        button.setToolTip(Some(&hint));
    }

    pub fn sync(frame: &eframe::Frame, want: bool) -> Outcome {
        let Some(window) = crate::system::window::ns_window(frame) else {
            return Outcome::Unavailable;
        };
        // 这颗窗没有绿键时不报错，交回调用方记一次警告
        let Some(button) = window.standardWindowButton(NSWindowButton::ZoomButton) else {
            return Outcome::Unavailable;
        };

        if want {
            if !is_ours(&button) {
                hook(&button);
            }
            ensure_hint(&button);
            Outcome::Hooked
        } else {
            if is_ours(&button) {
                release(&button);
            }
            Outcome::Released
        }
    }
}

#[cfg(target_os = "macos")]
pub use macos::sync;

/// 非 macOS 没有这颗按钮的 target-action 可改，恒返回 `Unavailable`。
/// 那一侧切回全量只剩 `Cmd+Shift+M`，与本应用"macOS 优先"的口径一致。
#[cfg(not(target_os = "macos"))]
pub fn sync(_frame: &eframe::Frame, _want: bool) -> Outcome {
    Outcome::Unavailable
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 一次点击只兑现一次切换：取走即清零，否则同一刀会被切两遍
    #[test]
    fn one_click_is_handed_over_exactly_once() {
        assert!(!take_click(), "上一处的残留会让第一次点击凭空多切一刀");
        mark_click();
        mark_click();
        assert!(take_click());
        assert!(!take_click());
    }
}
