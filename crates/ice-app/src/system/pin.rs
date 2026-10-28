//! 标题栏图钉（FR-3.4 置顶，2026-10-28 变更 I）。
//!
//! 用户原话："置顶不应该是文字，应该和大部分软件一样，可以在标题栏最右侧有一个类似于钉子的
//! 图标，点击置顶变成蓝色并且钉子变成垂直状态，则置顶。再点击变回来。全量界面也要有。"
//!
//! **"标题栏最右侧"这一格放不进 egui。** 两种形态的窗口都带原生标题栏（Q-013），那条 32px
//! 高的带子是 AppKit 的 `NSTitlebarView` 在画：把窗口改成 `fullSizeContentView` 让内容伸到
//! 它下面，点击仍然被标题栏吞掉（那条区域自己接管 `mouseDown` 用来拖窗），画上去的图钉点不动。
//! 公开 API 里唯一"既在标题栏里、又收得到点击"的位置是
//! `NSWindow.addTitlebarAccessoryViewController:` 配 `layoutAttribute = Right`——
//! 这就是 Safari、Xcode 那些右上角控件走的路。
//!
//! 图标用系统 SF Symbol `pin` / `pin.fill`（macOS 11+），不自己画位图：描边粗细、深浅色适配、
//! 视网膜缩放都由系统负责，跟红绿灯那颗的观感天然一致。图钉是 template 图，
//! 染色走 `NSButton.setContentTintColor:`（只对 borderless 按钮生效），所以"点一下变蓝"是
//! 系统给的颜色，不是我们挑的一支色笔。
//!
//! "躺下/立起来"：`pin` 出厂就是斜 45°（钉尖朝左下），置顶要垂直就把整张图**转 45°**。
//! 转换在 `pin_image` 里一次做完（`lockFocus` 画布 + `NSAffineTransform`），结果连同
//! `Look` 缓存起来，档位不变就不重画。**方向只有一个常数**（[`PIN_UPRIGHT_DEG`]）：
//! 真机上若转反了改这一个数，别在别处再乘一次。
//!
//! 点击怎么回到帧循环：那颗按钮的 target 是运行时注册的 `NSObject` 子类，方法体只做
//! `mark_click()`；帧循环 `take_click()` 取走后置 `Effects::toggle_topmost`——与
//! `Cmd+Shift+P`（`Action::ToggleTopmost`）**落在同一个字段上**，置顶本身一个实现。
//! 一次点击只兑一次，理由与标题栏绿键那条一样：第二次点的时候层级已经在切了。
//!
//! 三条跟 `system::titlebar` 同一来源的硬规矩（那里踩过的坑）：
//! 1. `NSControl.target` 是 `unsafe_unretained`，那个 `Retained<PinTarget>` 必须自己存着
//!    （[`MOUNTED`]），一放手按钮里剩的就是野指针；
//! 2. **每帧核对现状**：问窗口"我们的 accessory 还在不在"，不在就重新挂上去。AppKit 会在
//!    进出全屏等路径上重排标题栏，缓存"挂过了"会让图钉悄悄消失；
//! 3. 悬停提示跟着状态改口，界面不能撒谎。
//!
//! 取窗口句柄与 `system::alpha`、`system::titlebar` 共用 `super::window::ns_window`（只在主线程）。

use std::sync::atomic::{AtomicBool, Ordering};

/// 图钉被点了。写它的是 AppKit 的 action 方法（主线程），读它的是帧循环（同一线程）。
static CLICKED: AtomicBool = AtomicBool::new(false);

/// 记一次"图钉被点了"。ObjC 侧的 action 方法调它，测试也从这里模拟点击。
pub fn mark_click() {
    CLICKED.store(true, Ordering::SeqCst);
}

/// 取走"图钉被点了"这个事实。一帧内点多次只算一次。
pub fn take_click() -> bool {
    CLICKED.swap(false, Ordering::SeqCst)
}

/// 置顶时钉子要立起来转过的角度（度，AppKit 坐标下逆时针为正）。
/// `pin` 出厂是斜 45°，转这个数之后钉尖朝下。
pub const PIN_UPRIGHT_DEG: f64 = 45.0;

/// 图钉那一格从窗口右边缘往里占的宽度（pt），与 `macos::PIN_SIZE` 的宽**同一个数**。
///
/// 单列出来给 `system::dblclick` 用：算标题栏双击的落点区间时，右端必须把这一格让开——
/// 那颗图钉也是按钮，吞掉它第二下的 `mouseDown` 就变成"图钉点不动"（缺陷 D-9 那一类：
/// 两处各自写一遍尺寸，出的就是"一边有效一边无效"）。
pub const PIN_ZONE_W: f64 = 28.0;

/// 图钉该长什么样。**纯数据 + 纯函数**：真机上那颗按钮只是把这三项照抄一遍，
/// 而"置顶 = 蓝色 + 垂直"这条语义由 [`look_for`] 在无头侧锁住。
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Look {
    /// SF Symbol 名：未置顶是空心的 `pin`，置顶是实心的 `pin.fill`
    pub symbol: &'static str,
    /// 相对出厂角度转多少度
    pub tilt_deg: f64,
    /// 是否染成系统蓝
    pub tinted: bool,
}

/// 置顶 → 实心 + 立起来 + 蓝；未置顶 → 空心 + 躺着 + 跟随标题栏本色。
///
/// 三样一起变是有原因的：万一那 45° 在某台机器上转坏了（`pin_image` 会退回不转的那张），
/// 空心/实心与颜色这两档还分得清状态。
pub const fn look_for(pinned: bool) -> Look {
    if pinned {
        Look {
            symbol: "pin.fill",
            tilt_deg: PIN_UPRIGHT_DEG,
            tinted: true,
        }
    } else {
        Look {
            symbol: "pin",
            tilt_deg: 0.0,
            tinted: false,
        }
    }
}

/// 悬停提示：说清现在是什么状态、点一下会怎样、键盘上等价的那条是什么。
pub const fn tip_for(pinned: bool) -> &'static str {
    if pinned {
        "已置顶：窗口压在其他应用之上，点一下取消（Cmd+Shift+P）"
    } else {
        "置顶：窗口压在其他应用之上（Cmd+Shift+P）"
    }
}

/// 这一帧标题栏图钉挂上了没有（只给调用方记日志用，值变了才记）
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Outcome {
    /// 挂上了，点它是切置顶
    Mounted,
    /// 这条路走不通（非 macOS、拿不到窗口句柄、这台机器上没有 `pin` 这个符号）
    Unavailable,
}

#[cfg(target_os = "macos")]
mod macos {
    use std::cell::RefCell;
    use std::sync::atomic::{AtomicBool, Ordering};

    use objc2::rc::{Allocated, Retained};
    use objc2::runtime::AnyObject;
    use objc2::{AnyThread, MainThreadMarker, define_class, msg_send, sel};
    use objc2_app_kit::{
        NSAffineTransformNSAppKitAdditions, NSButton, NSCellImagePosition, NSColor,
        NSCompositingOperation, NSImage, NSLayoutAttribute, NSTitlebarAccessoryViewController,
        NSView, NSWindow,
    };
    use objc2_foundation::{NSAffineTransform, NSObject, NSPoint, NSRect, NSSize, NSString};

    use super::{Look, Outcome, PIN_ZONE_W, look_for, mark_click, tip_for};

    /// 图钉那一带的尺寸。宽度比图标多一点，点在边缘上不至于落到标题栏空白里被当成拖窗。
    /// 宽度用 [`PIN_ZONE_W`]：标题栏双击的落点区间要从右边缘让开这一格，两处共用一个数。
    const PIN_SIZE: NSSize = NSSize::new(PIN_ZONE_W, 22.0);
    /// 容器与图标共用同一个原点：按钮贴在容器左下角，高度差由标题栏自己居中
    const POS_ZERO: NSPoint = NSPoint::new(0.0, 0.0);

    define_class!(
        /// 图钉的 target。方法体只有 `mark_click()` 一行——"置顶还是取消"全部留在帧循环里，
        /// 与快捷键共用同一条 effects 路径。
        #[unsafe(super(NSObject))]
        #[name = "IceTranslatorPinTarget"]
        struct PinTarget;

        impl PinTarget {
            // 选择器首词 `ice` 不属于任何 Objective-C 方法族，`define_class!` 自己推断；
            // 再手写 `#[unsafe(method_family = none)]` 会和 `#[unsafe(method(...))]` 冲突。
            #[unsafe(method(iceTogglePinFromTitleBar:))]
            fn toggle_pin(&self, _sender: &AnyObject) {
                mark_click();
            }
        }
    );

    impl PinTarget {
        /// `alloc` + `init`：没有实例变量，状态全在 `CLICKED` 那个全局原子里
        fn new() -> Retained<Self> {
            let this: Allocated<Self> = Self::alloc();
            let this = this.set_ivars(());
            unsafe { msg_send![super(this), init] }
        }
    }

    /// 挂上去的那一套，连同**那个 target 的唯一强引用**
    struct Mounted {
        controller: Retained<NSTitlebarAccessoryViewController>,
        button: Retained<NSButton>,
        _target: Retained<PinTarget>,
        look: Look,
        tip: &'static str,
    }

    thread_local! {
        static MOUNTED: RefCell<Option<Mounted>> = const { RefCell::new(None) };
    }

    /// 转角度那次"转坏了"只报一次，别把日志刷满
    static FLAT_WARNED: AtomicBool = AtomicBool::new(false);

    /// 按 `Look` 造图标：取系统符号 → 需要时转 `tilt_deg` → 标成 template 让 `contentTintColor` 生效。
    ///
    /// `lockFocus`/`unlockFocus` 已被 Apple 标为 deprecated（它不建议用于"分辨率无关"的图）。
    /// 这里仍然用它，理由有三：换它推荐的两条路，一条要 `imageWithSize:flipped:drawingHandler:`
    /// （得开 `block2`，并且那块闭包必须逃逸、由 NSImage 长期持有），一条要自己造
    /// `NSBitmapImageRep` 与 `NSGraphicsContext`（多两个模块，代码翻倍）；而我们画的是一张
    /// 22pt 的小图标，`lockFocus` 按当前屏的倍率生成位图表示，观感与系统符号一致。
    /// 真的画空了（`representations` 为空）就退回未旋转的那张，见下面那次判空。
    #[allow(deprecated)]
    fn pin_image(look: Look, mtm: MainThreadMarker) -> Option<Retained<NSImage>> {
        let name = NSString::from_str(look.symbol);
        let label = NSString::from_str("置顶");
        let base =
            NSImage::imageWithSystemSymbolName_accessibilityDescription(&name, Some(&label))?;
        base.setTemplate(true);
        if look.tilt_deg == 0.0 {
            return Some(base);
        }
        let size = base.size();
        let canvas = NSImage::initWithSize(mtm.alloc(), size);
        canvas.lockFocus();
        let transform = NSAffineTransform::transform();
        transform.translateXBy_yBy(size.width / 2.0, size.height / 2.0);
        transform.rotateByDegrees(look.tilt_deg);
        transform.translateXBy_yBy(-size.width / 2.0, -size.height / 2.0);
        transform.concat();
        base.drawInRect_fromRect_operation_fraction(
            NSRect::new(POS_ZERO, size),
            NSRect::ZERO,
            NSCompositingOperation::Copy,
            1.0,
        );
        canvas.unlockFocus();
        canvas.setTemplate(true);
        if canvas.representations().is_empty() {
            // 一张都没画出来：宁可给用户一张躺着的实心蓝图钉，也不给一个看不见的按钮
            if !FLAT_WARNED.swap(true, Ordering::Relaxed) {
                tracing::warn!(
                    "标题栏图钉转角度没画出来，先用未旋转的那张（状态仍靠颜色与实心区分）"
                );
            }
            return Some(base);
        }
        Some(canvas)
    }

    /// 第一次挂：造那颗按钮、那个容器视图和 accessory 控制器，接到窗口右侧
    fn mount(window: &NSWindow, mtm: MainThreadMarker) -> Option<Mounted> {
        let frame = NSRect::new(POS_ZERO, PIN_SIZE);
        let button = NSButton::initWithFrame(mtm.alloc(), frame);
        // 无边框才有 contentTintColor 可用（Apple 的规矩：带框按钮不给你染色）
        button.setBordered(false);
        button.setTitle(&NSString::from_str(""));
        button.setImagePosition(NSCellImagePosition::ImageOnly);
        let target = PinTarget::new();
        // SAFETY: `setTarget:` 是 `unsafe_unretained` 赋值，强引用由 `MOUNTED` 持有；
        // `setAction:` 收的是 `define_class!` 刚注册过的那颗选择器。
        unsafe {
            button.setTarget(Some(&target));
            button.setAction(Some(sel!(iceTogglePinFromTitleBar:)));
        }

        let view = NSView::initWithFrame(mtm.alloc(), frame);
        view.addSubview(&button);

        let controller = NSTitlebarAccessoryViewController::new(mtm);
        controller.setView(&view);
        controller.setLayoutAttribute(NSLayoutAttribute::Right);
        window.addTitlebarAccessoryViewController(&controller);

        Some(Mounted {
            controller,
            button,
            _target: target,
            look: look_for(false),
            tip: tip_for(false),
        })
    }

    /// 按档位换图标与颜色（只在真的变了时做，见调用方）
    fn apply_look(button: &NSButton, look: Look, mtm: MainThreadMarker) {
        let Some(image) = pin_image(look, mtm) else {
            return;
        };
        button.setImage(Some(&image));
        if look.tinted {
            button.setContentTintColor(Some(&NSColor::systemBlueColor()));
        } else {
            // 交回系统默认色：深色标题栏上它自己会白，浅色上会黑，跟着主题走
            button.setContentTintColor(None);
        }
    }

    /// 我们的 accessory 还在窗口上吗（AppKit 重排标题栏时可能把它摘掉）
    fn still_attached(window: &NSWindow, mounted: &Mounted) -> bool {
        let mine = Retained::as_ptr(&mounted.controller).cast::<AnyObject>();
        window
            .titlebarAccessoryViewControllers()
            .iter()
            .any(|vc| Retained::as_ptr(&vc).cast::<AnyObject>() == mine)
    }

    pub fn sync(frame: &eframe::Frame, pinned: bool) -> Outcome {
        // 全程主线程：eframe 的 `update` 就在主线程上，拿不到 `mtm` 说明这条路走错了，
        // 与其硬来不如让图钉不出现（`Cmd+Shift+P` 与设置页那一格仍然管用）
        let Some(mtm) = MainThreadMarker::new() else {
            return Outcome::Unavailable;
        };
        let Some(window) = crate::system::window::ns_window(frame) else {
            return Outcome::Unavailable;
        };
        let look = look_for(pinned);
        let tip = tip_for(pinned);

        let mounted = MOUNTED.with(|slot| {
            let mut slot = slot.borrow_mut();
            if slot.is_none() {
                *slot = mount(&window, mtm);
            }
            let mounted = slot.as_mut()?;
            if !still_attached(&window, mounted) {
                window.addTitlebarAccessoryViewController(&mounted.controller);
            }
            if mounted.look != look {
                apply_look(&mounted.button, look, mtm);
                mounted.look = look;
            }
            if mounted.tip != tip {
                mounted.button.setToolTip(Some(&NSString::from_str(tip)));
                mounted.tip = tip;
            }
            Some(())
        });

        match mounted {
            Some(()) => Outcome::Mounted,
            None => Outcome::Unavailable,
        }
    }
}

#[cfg(target_os = "macos")]
pub use macos::sync;

/// 非 macOS 没有标题栏 accessory 这套（P8 跨平台复核）：恒 `Unavailable`。
/// 那一侧置顶只剩 `Cmd+Shift+P` 与设置页那一格。
#[cfg(not(target_os = "macos"))]
pub fn sync(_frame: &eframe::Frame, _pinned: bool) -> Outcome {
    Outcome::Unavailable
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 一次点击只兑现一次切换：取走即清零，否则同一刀会切两遍
    #[test]
    fn one_click_is_handed_over_exactly_once() {
        // `CLICKED` 是进程级的，测试并行跑时别的用例可能留了残留，先把它排空
        while take_click() {}
        mark_click();
        mark_click();
        assert!(take_click());
        assert!(!take_click());
    }

    /// 用户要的那句话本身就是判据：置顶 = 蓝色 + 垂直，未置顶 = 本色 + 躺着
    #[test]
    fn pinned_means_upright_and_blue() {
        let pinned = look_for(true);
        let free = look_for(false);
        assert!(pinned.tinted, "置顶要染成蓝色");
        assert!(!free.tinted, "未置顶用标题栏本色");
        assert_eq!(free.tilt_deg, 0.0, "未置顶就用系统那张躺着的 pin");
        assert!(
            pinned.tilt_deg.abs() > f64::EPSILON,
            "置顶必须把钉子立起来（转过的角度不能是 0）"
        );
        assert_eq!(
            pinned.tilt_deg, PIN_UPRIGHT_DEG,
            "角度只有这一个出处，别处不许再乘一次"
        );
        assert_eq!(
            pinned.symbol, "pin.fill",
            "立起来的同时换成实心，转坏了也分得出状态"
        );
        assert_eq!(free.symbol, "pin");
    }

    /// 档位与提示都只有两格，且一一对应（防止改了状态忘了改口）
    #[test]
    fn the_tip_matches_the_state() {
        assert_ne!(tip_for(true), tip_for(false));
        assert!(tip_for(true).contains("已置顶"));
        assert!(
            tip_for(false).contains("Cmd+Shift+P"),
            "两档都要说出键盘上等价的那条"
        );
        assert!(tip_for(true).contains("Cmd+Shift+P"));
    }
}
