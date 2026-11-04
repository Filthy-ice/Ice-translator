//! 菜单栏托盘与"关窗去哪儿"（CHG-008：FR-5.4 + FR-5.2 前半句改判）。
//!
//! 用户 2026-11-03 那条报告有两件事：窗口点关闭不是收进托盘，而是躺在程序坞里。
//! 本模块把这件事改成**关窗 = 收进菜单栏**，并且把 Dock 磁贴一起收掉（用户在三个档位里
//! 选的中间那一档：运行期常驻，不是"全程只做菜单栏应用"）。四件不显然的事：
//!
//! 1. **图标是我们自己画的单色线稿**，不是 Dock 那张彩色方砖的缩小版。托盘那枚按钮走的是
//!    `setTemplate:` 的语义——"拿 alpha 当蒙版、由系统按深浅色上色"，给它一张彩图就等于
//!    既要形状又要颜色，系统只能按 alpha 走，结果是一团灰（REQ-002）。
//! 2. **回调里只立意图，不兑现**。`define_class!` 出来的方法既拿不到 `AppState` 也不该拿：
//!    它把"用户点了什么"写进一个 `AtomicU8`，帧循环下一帧取走并走既有那条 `Action` 通道。
//!    这是 `system::titlebar`、`system::dblclick` 之后的第三个同形状实例，不是巧合——
//!    唤醒路径若各写各的，出的就是"菜单项生效、左键不生效"这类只对一半的缺陷。
//! 3. **`request_repaint()` 是整条链的承重墙**。窗口 `orderOut` 之后不再收 `RedrawRequested`，
//!    但 eframe 0.37 会给不可见窗直接跑一次 pass（`run.rs:244-250`，注释写明目的正是"确保
//!    `Visible(true)` 这类命令仍被处理"），节流到 100ms（`run.rs:26-28`）。所以回调里补一次
//!    重绘请求就够，**不需要**为唤醒常驻一个事件监视器（REQ-007）。
//! 4. **整套生效或整套不生效**（REQ-006，本包最硬的否定式判据）。`TrayState` 是唯一分叉点：
//!    关窗去向、唤回入口、设置页文案、心跳四个消费者都读它。混合状态 = 窗口 `Visible(false)`
//!    收走了、磁贴被 `.accessory` 关了、唤醒入口又没装上——用户只剩"杀掉进程"一条路，
//!    正是 2026-10-27 那句"我没有办法重启软件"。装不上就整套退回今天的最小化语义。
//!
//! 左键与右键分叉靠 `NSApplication::currentEvent().r#type()`：`sendActionOn` 把按钮的触发
//! 掩码设成"左键抬起 + 右键抬起"，两种抬起都落到同一个方法上，方法里看当前事件是哪一种。
//! 菜单**不常驻**挂在状态项上——`setMenu:` 挂上去之后左键就不走 action 了（spec Q-001），
//! 所以只在右键成立那一刻挂上、`performClick` 逼它弹出，出完立刻摘掉。

use std::sync::OnceLock;
use std::sync::atomic::{AtomicU8, Ordering};

use ice_core::config::UiMode;
use image::{ImageEncoder, Rgba, RgbaImage};

use crate::state::TrayState;
use crate::system::hotkeys::Action;

/// 本包这套能力走不走得通。Windows 的 `Shell_NotifyIcon`、Linux 的 `StatusNotifierItem`
/// 按 NFR-7 另立待办，不在这里假装支持。
pub const SUPPORTED: bool = cfg!(target_os = "macos");

/// 菜单第一项（左键立的是同一个意图，只是不经菜单）。
pub const SHOW: &str = "显示窗口";
/// 菜单第三项。它与设置页那颗、`Cmd+Q` 共指 `Action::Quit`，**不新增第二条退出实现**。
pub const QUIT: &str = "退出程序";

// ---- 意图槽：回调写、帧循环读 ----

const CODE_NONE: u8 = 0;
const CODE_REVEAL: u8 = 1;
const CODE_TOGGLE_MODE: u8 = 2;
const CODE_QUIT: u8 = 3;

/// 托盘被点出来的那件事。一个槽而不是三个布尔：一次回调只可能立一个意图，
/// 三个标记会允许"同时既要退出又要唤回"这种根本不该存在的组合。
static PENDING: AtomicU8 = AtomicU8::new(CODE_NONE);

/// 唤醒源。隐藏的窗口收不到输入事件，`mark()` 必须能自己把帧循环叫起来，
/// 所以这里存一份 `egui::Context`（克隆廉价，内部是 `Arc`）。
static CONTEXT: OnceLock<egui::Context> = OnceLock::new();

/// 托盘转交给帧循环的三件事之一。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Outcome {
    /// 左键、或菜单第一项：把窗口叫回来
    Reveal,
    /// 菜单第二项：切到另一侧形态（与 `Cmd+Shift+M` 落在同一条 `Action` 上）
    ToggleMode,
    /// 菜单第三项：退出（与设置页那颗、`Cmd+Q` 同一条）
    Quit,
}

impl Outcome {
    const fn code(self) -> u8 {
        match self {
            Outcome::Reveal => CODE_REVEAL,
            Outcome::ToggleMode => CODE_TOGGLE_MODE,
            Outcome::Quit => CODE_QUIT,
        }
    }

    const fn from_code(code: u8) -> Option<Self> {
        match code {
            CODE_REVEAL => Some(Outcome::Reveal),
            CODE_TOGGLE_MODE => Some(Outcome::ToggleMode),
            CODE_QUIT => Some(Outcome::Quit),
            _ => None,
        }
    }
}

/// 立一个意图并把帧循环叫醒。ObjC 方法体里只做这一件事。
pub fn mark(outcome: Outcome) {
    PENDING.store(outcome.code(), Ordering::SeqCst);
    if let Some(ctx) = CONTEXT.get() {
        ctx.request_repaint();
    }
}

/// 取走意图，取完清零。一帧内点多次只兑现一次：第二次点的时候上一件事已经在路上了
/// （与 `titlebar::take_click`、`dblclick::take_double_click` 同一纪律）。
pub fn take_outcome() -> Option<Outcome> {
    Outcome::from_code(PENDING.swap(CODE_NONE, Ordering::SeqCst))
}

/// 意图 → 既有动作通道。`Outcome::Quit` 必须映射到**已有**的 `Action::Quit`（TC-5.4-b）。
pub const fn action_of(outcome: Outcome) -> Action {
    match outcome {
        Outcome::Reveal => Action::RevealWindow,
        Outcome::ToggleMode => Action::ToggleMode,
        Outcome::Quit => Action::Quit,
    }
}

/// 菜单三项的文案（TC-5.4-b 的无头判据挂在这上面）。
///
/// 第二项说的是"切到**另一侧**"的名字：当前在全量窗就写「精简」，在简洁窗就写「全量」。
/// 写「悬浮窗」是 2026-10-29 已被用户改口的旧词（任务 #44）。
pub fn menu_titles(mode: UiMode) -> [&'static str; 3] {
    [
        SHOW,
        if mode == UiMode::Full {
            "精简"
        } else {
            "全量"
        },
        QUIT,
    ]
}

/// 设置页「退出程序」旁边那句说明（REQ-005）。文案与降级语义同处，`views` 只调用。
pub const fn hint_text(state: TrayState) -> &'static str {
    match state {
        TrayState::Available => "红灯 / Cmd+W 是收进托盘，不是退出",
        // 非 macOS 与"装不出来"共用这句：那是那一档真实发生的事，不许谎称有托盘
        TrayState::Unavailable | TrayState::Unknown => "红灯 / Cmd+W 是收进后台，不是退出",
    }
}

// ---- 图标（纯位图，不碰 AppKit，任何平台可编译可测）----

/// 托盘图标的档位：位图的**像素**高度。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Tier {
    /// @1x：22px
    One,
    /// @2x：44px
    Two,
}

impl Tier {
    /// 菜单栏那一格的高度（点）。图标按这个尺寸请求显示，位图按档位放大。
    pub const HEIGHT: f64 = 22.0;

    pub const fn px(self) -> u32 {
        match self {
            Tier::One => 22,
            Tier::Two => 44,
        }
    }

    /// 按后备比例因子选档。1.5 以上算视网膜——2.0/3.0 都够用，中间那些杂项比例宁可取大图
    /// （系统往下缩不会糊，往上放才会糊）。
    pub const fn from_scale(scale: f64) -> Tier {
        if scale > 1.5 { Tier::Two } else { Tier::One }
    }
}

/// 线稿上的一个点。**坐标系与 AppKit 一致**：原点在左下角，y 向上，单位是点。
#[derive(Clone, Copy)]
struct Pt {
    x: f64,
    y: f64,
}

const fn pt(x: f64, y: f64) -> Pt {
    Pt { x, y }
}

/// 一条笔画：从 `a` 到 `b`、宽 `width` 点的线段（圆头端点，按"点到线段距离"算覆盖）。
#[derive(Clone, Copy)]
struct Stroke {
    a: Pt,
    b: Pt,
    width: f64,
}

const fn seg(a: Pt, b: Pt, width: f64) -> Stroke {
    Stroke { a, b, width }
}

/// 边缘羽化宽度（**像素**，不是点）。半个像素，够把 22px 的折角抹平，又不会把留白吃掉。
const RAMP_PX: f64 = 0.5;

/// 这一张线稿：一个窗口轮廓 + 一只朝上的折角，读作"窗口收进上面那条菜单栏"。
///
/// 坐标都落在 22×22 的点格子里：轮廓宽 15、高 12.5，水平居中（`3.5 + 18.5 = 22`）；
/// 折角跨 5、落差 2.5，顶点在 `y = 18.5`。
///
/// TC-5.4-a 那四条钉的是**边界行为**（边长、纯黑、四角透明、包围盒居中且四周留白），
/// 不是这几个具体数值——这张表可以改，四条判据不跟着改。
const fn strokes() -> [Stroke; 6] {
    let pen = 1.0;
    [
        // 窗口轮廓（长方形四条边）
        seg(pt(3.5, 2.0), pt(18.5, 2.0), pen),
        seg(pt(18.5, 2.0), pt(18.5, 14.5), pen),
        seg(pt(18.5, 14.5), pt(3.5, 14.5), pen),
        seg(pt(3.5, 14.5), pt(3.5, 2.0), pen),
        // 朝上的折角（读作"往上收进菜单栏"）
        seg(pt(8.5, 16.0), pt(11.0, 18.5), pen),
        seg(pt(11.0, 18.5), pt(13.5, 16.0), pen),
    ]
}

/// 画这一档的位图：所有像素的 RGB 恒为纯黑，只有 alpha 有形状（模板图靠 alpha 上色）。
pub fn glyph(tier: Tier) -> RgbaImage {
    let side = tier.px();
    let scale = f64::from(side) / Tier::HEIGHT;
    let mut image = RgbaImage::from_pixel(side, side, Rgba([0, 0, 0, 0]));
    for stroke in strokes() {
        paint(&mut image, &stroke, scale);
    }
    image
}

/// 把一个笔画刷进位图。像素取**中心点**判定，重叠处 alpha 取较大值。
fn paint(image: &mut RgbaImage, stroke: &Stroke, scale: f64) {
    let a = pt(stroke.a.x * scale, stroke.a.y * scale);
    let b = pt(stroke.b.x * scale, stroke.b.y * scale);
    let radius = stroke.width * scale;
    let reach = radius + RAMP_PX;
    // 只扫这一笔的外接框，夹到画布内
    let x0 = (a.x.min(b.x) - reach).max(0.0) as u32;
    let x1 = (a.x.max(b.x) + reach).min(f64::from(image.width()) - 1.0) as u32;
    let y0 = (a.y.min(b.y) - reach).max(0.0) as u32;
    let y1 = (a.y.max(b.y) + reach).min(f64::from(image.height()) - 1.0) as u32;
    for row in y0..=y1 {
        for col in x0..=x1 {
            let px = f64::from(col) + 0.5;
            let py = f64::from(row) + 0.5;
            let dist_sq = distance_sq(px, py, a, b);
            if dist_sq > reach * reach {
                continue;
            }
            let dist = dist_sq.sqrt();
            let alpha = if dist <= radius {
                255.0
            } else {
                ((reach - dist) / RAMP_PX * 255.0).max(0.0)
            };
            let pixel = image.get_pixel_mut(col, row);
            *pixel = Rgba([0, 0, 0, pixel.0[3].max(alpha as u8)]);
        }
    }
}

/// 点到线段的**平方**距离（免开方，直接与半径的平方比）。
fn distance_sq(px: f64, py: f64, a: Pt, b: Pt) -> f64 {
    let dx = b.x - a.x;
    let dy = b.y - a.y;
    let len_sq = dx * dx + dy * dy;
    let (ox, oy) = if len_sq == 0.0 {
        (px - a.x, py - a.y)
    } else {
        let t = clamp01(((px - a.x) * dx + (py - a.y) * dy) / len_sq);
        (px - a.x - t * dx, py - a.y - t * dy)
    };
    capped_sq(ox, oy)
}

const fn clamp01(v: f64) -> f64 {
    if v < 0.0 {
        0.0
    } else if v > 1.0 {
        1.0
    } else {
        v
    }
}

/// 平方距离，且**不会溢出或算出 NaN**：任一方向的差超过 2 直接返回 9.0，那是一个必然大于
/// `reach²`（本包最大 6.25）的值，判据把它筛掉。剩下的分量都在 2 以内，平方相加最多 8——
/// 浮点这条路上没有上溢，也就没有"某条线段偶发画出满幅脏像素"这种查不出来的缺陷。
const fn capped_sq(dx: f64, dy: f64) -> f64 {
    if dx > 2.0 || dx < -2.0 || dy > 2.0 || dy < -2.0 {
        9.0
    } else {
        dx * dx + dy * dy
    }
}

/// 位图 → PNG 字节。mac 侧再走 `NSData` → `NSImage::initWithData:` → `setTemplate:`。
///
/// 编码成图片而不是塞裸像素：`NSImage::initWithData:` 认的是 PNG/JPEG 一类的**编码数据**。
pub fn glyph_bytes(tier: Tier) -> Vec<u8> {
    let image = glyph(tier);
    let mut out = Vec::new();
    // 写不进 `Vec` 只可能是内存耗尽，而那本来就走 abort；这里不为它加分支
    let _ = image::codecs::png::PngEncoder::new(&mut out).write_image(
        image.as_raw(),
        image.width(),
        image.height(),
        image::ExtendedColorType::Rgba8,
    );
    out
}

#[cfg(target_os = "macos")]
mod mac {
    use std::cell::{Cell, RefCell};
    use std::ffi::c_void;

    use objc2::rc::{Allocated, Retained};
    use objc2::runtime::{AnyObject, Sel};
    use objc2::{AnyThread, MainThreadMarker, MainThreadOnly, define_class, msg_send, sel};
    use objc2_app_kit::{
        NSApplication, NSApplicationActivationPolicy, NSEventMask, NSEventType, NSImage, NSMenu,
        NSMenuItem, NSStatusBar, NSStatusBarButton, NSStatusItem, NSVariableStatusItemLength,
    };
    use objc2_foundation::{NSData, NSObject, NSSize, NSString};

    use super::{CONTEXT, Outcome, Tier, glyph_bytes, mark, menu_titles};
    use crate::state::TrayState;
    use ice_core::config::UiMode;

    /// 左键与右键都落到这一个方法上，方法里再分叉。
    fn click_sel() -> Sel {
        sel!(iceTrayClicked:)
    }

    /// 菜单三项的选择器，与 `menu_titles` 一一对应。第一项与左键立**同一个**意图。
    fn menu_sels() -> [Sel; 3] {
        [
            sel!(iceTrayShowWindow:),
            sel!(iceTrayToggleMode:),
            sel!(iceTrayQuit:),
        ]
    }

    define_class!(
        /// 托盘那枚按钮与菜单三项共同的 target。四个方法体各自只有一行——
        /// "要不要唤回、唤成什么"全部留在帧循环里，与快捷键共用同一条 effects 路径。
        #[unsafe(super(NSObject))]
        #[name = "IceTranslatorTrayTarget"]
        struct TrayTarget;

        impl TrayTarget {
            // 选择器首词 `ice` 不属于任何 Objective-C 方法族，`define_class!` 自己推断；
            // 再手写 `#[unsafe(method_family = none)]` 会和 `#[unsafe(method(...))]` 冲突。
            #[unsafe(method(iceTrayClicked:))]
            fn clicked(&self, _sender: &AnyObject) {
                // `pop_menu` 合成那一次点击不再分叉：它唯一的作用是"让挂上去的菜单弹出来"，
                // 再立一次意图就成了每帧自激的循环。
                if INSIDE_SYNTHETIC_CLICK.with(|flag| flag.get()) {
                    return;
                }
                if current_event_is_right_click() {
                    // 挂菜单必须在帧循环里同步做（见 `pop_menu`），这里只登记"该弹一次"
                    WANT_MENU.with(|flag| flag.set(true));
                } else {
                    mark(Outcome::Reveal);
                }
            }

            #[unsafe(method(iceTrayShowWindow:))]
            fn show_window(&self, _sender: &AnyObject) {
                mark(Outcome::Reveal);
            }

            #[unsafe(method(iceTrayToggleMode:))]
            fn toggle_mode(&self, _sender: &AnyObject) {
                mark(Outcome::ToggleMode);
            }

            #[unsafe(method(iceTrayQuit:))]
            fn quit(&self, _sender: &AnyObject) {
                mark(Outcome::Quit);
            }
        }
    );

    impl TrayTarget {
        fn new() -> Retained<Self> {
            let this: Allocated<Self> = Self::alloc();
            let this = this.set_ivars(());
            // SAFETY: `init` 是 `NSObject` 的标准初始化器，`this` 刚由 `alloc` 造出来。
            unsafe { msg_send![super(this), init] }
        }
    }

    thread_local! {
        /// 造好的那枚状态项。`Retained` 必须由我们持有：状态项一旦没有强引用，
        /// AppKit 会把它从菜单栏撤掉，图标就地消失。
        static TRAY: RefCell<Option<Tray>> = const { RefCell::new(None) };
        /// 右键成立，该弹一次菜单（`sync` 取走并清零）
        static WANT_MENU: Cell<bool> = const { Cell::new(false) };
        /// 正在合成那一次点击（挡住它再立意图，见 `clicked`）
        static INSIDE_SYNTHETIC_CLICK: Cell<bool> = const { Cell::new(false) };
    }

    /// 装好的一切强引用。每个字段都有非显然的持有理由：
    /// - `item`：见上，松手图标就没了；
    /// - `target`：`NSControl.target` 与 `NSMenuItem.target` 都是**弱**引用（objc2 把
    ///   `setTarget:` 直接标成 weak property），我们一放手就是野指针，点一下访问已释放对象；
    /// - `menu`：状态项上的菜单是我们临时挂/摘的，不自己拿住就没有人拿住；
    /// - `image`：`setImage:` 只收引用，放手后按钮留下的是一张已被回收的图；
    /// - `button`：接线每帧都要问它，取一次存下来比每帧 `item.button(mtm)` 少一条消息；
    /// - `titles`：挂上去那三项用的字符串，留着才问得到"标题是不是已经对得上"——
    ///   每帧现造 `NSString` 是白白的常驻分配。
    struct Tray {
        item: Retained<NSStatusItem>,
        button: Retained<NSStatusBarButton>,
        image: Retained<NSImage>,
        target: Retained<TrayTarget>,
        menu: Retained<NSMenu>,
        items: [Retained<NSMenuItem>; 3],
        titles: [Retained<NSString>; 3],
        tier: Tier,
    }

    /// 这一档的位图做成一张模板 `NSImage`，并按**点**尺寸请求显示（视网膜下用的就是大图那张）。
    fn make_image(tier: Tier) -> Option<Retained<NSImage>> {
        let data = NSData::with_bytes(&glyph_bytes(tier));
        // `initWithData:` 的首参要求刚 alloc、尚未初始化的内存，所以这两步连着写、
        // 中间不 early-return。返回 `None` 是文档允许的结果（这张位图没能变成图标），
        // 不是 panic 的理由。
        let image = NSImage::initWithData(NSImage::alloc(), &data)?;
        image.setSize(NSSize::new(Tier::HEIGHT, Tier::HEIGHT));
        image.setTemplate(true);
        Some(image)
    }

    /// 这颗按钮现在是不是已经归我们：action 是我们的那颗选择器，且 target 还是我们那个对象。
    ///
    /// 与 `titlebar::is_ours` 同一纪律——**每帧从按钮本身读现状**，不缓存"我接过线了"：
    /// AppKit 会在别的路径上重新接线，缓存下去出的就是"看得见但点不动"的装饰。
    fn is_ours(button: &NSStatusBarButton, target: &Retained<TrayTarget>) -> bool {
        if button.action() != Some(click_sel()) {
            return false;
        }
        let mine = Retained::as_ptr(target).cast::<c_void>();
        button
            .target()
            .is_some_and(|t| Retained::as_ptr(&t).cast::<c_void>() == mine)
    }

    /// 接线：target/action 归我们，且**左键抬起与右键抬起都算**（默认只有左键）。
    fn hook(button: &NSStatusBarButton, target: &Retained<TrayTarget>) {
        // SAFETY: `setTarget:` 是 weak 赋值，强引用由 `TRAY` 里的 `Tray.target` 持有到
        // 进程结束；`setAction:` 收的是本文件里 `sel!` 出来的已注册选择器。
        unsafe {
            button.setTarget(Some(target));
            button.setAction(Some(click_sel()));
        }
        // 右键也要能触发同一个方法，否则分叉没有输入可分。
        button.sendActionOn(NSEventMask::LeftMouseUp | NSEventMask::RightMouseUp);
    }

    /// 当前事件是不是"右键抬起"。取不到就当**不是**：那一支只立唤回意图，
    /// 方向是少做而不是多做（菜单出不去只是不便，误挂菜单会让左键失去 action）。
    fn current_event_is_right_click() -> bool {
        let Some(mtm) = MainThreadMarker::new() else {
            return false;
        };
        NSApplication::sharedApplication(mtm)
            .currentEvent()
            .is_some_and(|event| event.r#type() == NSEventType::RightMouseUp)
    }

    /// 三项标题与当帧形态对不上才重写（全量 ↔ 简洁切换后第二项要改口）。
    fn ensure_titles(tray: &mut Tray, mode: UiMode) {
        let labels = menu_titles(mode);
        for ((item, last_written), label) in tray.items.iter_mut().zip(&mut tray.titles).zip(labels)
        {
            if *item.title() == **last_written {
                continue;
            }
            let text = NSString::from_str(label);
            item.setTitle(&text);
            // 两份必须一起改口：`item.title()` 比的是内容，`last_written` 记的是"我们最后写进去的"
            *last_written = text;
        }
    }

    /// 装（幂等）。已经装过就只做"该改的改"，绝不重复造状态项——
    /// 重复造会在菜单栏里长出第二枚图标（REQ-001 的负判据）。
    pub fn install(mode: UiMode, ctx: &egui::Context, scale: f64) -> TrayState {
        if CONTEXT.get().is_none() {
            let _ = CONTEXT.set(ctx.clone());
        }
        let Some(mtm) = MainThreadMarker::new() else {
            return TrayState::Unavailable;
        };
        let want = Tier::from_scale(scale);
        TRAY.with(|tray| {
            let mut slot = tray.borrow_mut();
            let Some(existing) = slot.as_mut() else {
                return match build(mtm, mode, want) {
                    Some(built) => {
                        *slot = Some(built);
                        TrayState::Available
                    }
                    None => TrayState::Unavailable,
                };
            };
            if existing.tier != want {
                // 屏幕换了、或第一帧还没拿到比例因子：换一档位图重设一次，不重建状态项
                if let Some(image) = make_image(want) {
                    existing.button.setImage(Some(&image));
                    existing.image = image;
                    existing.tier = want;
                }
            }
            TrayState::Available
        })
    }

    /// 每帧核对接线与菜单标题，并兑现右键那一次"临时挂菜单"。
    pub fn sync(mode: UiMode) -> TrayState {
        TRAY.with(|tray| {
            let mut slot = tray.borrow_mut();
            let Some(tray) = slot.as_mut() else {
                return TrayState::Unknown;
            };
            if !is_ours(&tray.button, &tray.target) {
                hook(&tray.button, &tray.target);
            }
            ensure_titles(tray, mode);
            if WANT_MENU.with(|flag| flag.replace(false)) {
                pop_menu(tray);
            }
            TrayState::Available
        })
    }

    /// 挂上菜单 → 按一次按钮让它弹出 → 立刻摘下。三步连在一起、都在主线程的一帧里做完：
    /// 菜单一常驻挂着，左键就不走 action 了（Q-001）。
    ///
    /// "真的弹得出来"只有真机能定论（W-2.40）：`performClick:` 是合成一次点击，而状态项按钮
    /// 在真实点击里靠自己的追踪循环弹菜单，合成这一次走不走同一条路，无头测问不出来。
    /// 真机上若弹不出，退路是把这一支换成
    /// `NSMenu::popUpMenuPositioningItem:atLocation:inView:`——左键唤回与三项动作都不受影响。
    fn pop_menu(tray: &Tray) {
        tray.item.setMenu(Some(&tray.menu));
        INSIDE_SYNTHETIC_CLICK.with(|flag| {
            flag.set(true);
            // SAFETY: `performClick:` 在主线程同步派发按钮动作；`sender` 传 `None`
            // 是文档允许的（我们的四个方法都不读发送者）。
            unsafe { tray.button.performClick(None) };
            flag.set(false);
        });
        tray.item.setMenu(None);
    }

    /// 切激活策略：`false` = `.accessory`（Dock 磁贴消失），`true` = `.regular`（回来）。
    ///
    /// 这一句必须在主线程、且 `Visible(true)` **之前**发生：winit 的 `set_visible(true)`
    /// 走 `makeKeyAndOrderFront`，而 `.accessory` 态的窗口能不能成为 key window 由策略决定。
    /// `activate()` 紧随其后（spec Q-004：唤回要真的走到用户面前）——真机上若嫌它抢焦点，
    /// 删这一句就够，不影响其它判据。
    pub fn set_policy(regular: bool) -> bool {
        // 菜单栏上没有那枚图标就不许关磁贴：唤醒入口都没装上，把 Dock 图标也收走等于
        // 把用户关在门外（REQ-006 那个"只剩杀掉进程一条路"的缺陷）。
        // 顺带让 `mode` 的两支在**无头测试**里只发命令、不碰 `NSApp`。
        if TRAY.with(|tray| tray.borrow().is_none()) {
            return false;
        }
        let Some(mtm) = MainThreadMarker::new() else {
            return false;
        };
        let app = NSApplication::sharedApplication(mtm);
        let policy = if regular {
            NSApplicationActivationPolicy::Regular
        } else {
            NSApplicationActivationPolicy::Accessory
        };
        let ok = app.setActivationPolicy(policy);
        if regular && ok {
            app.activate();
        }
        ok
    }

    /// 从零造出状态项：长度自适应、按钮接线、位图、菜单三项。
    /// 任何一环取不到就返回 `None`，由调用方整档退回 `Unavailable`（REQ-006 要求
    /// "整套不生效"，不许半装）。
    fn build(mtm: MainThreadMarker, mode: UiMode, tier: Tier) -> Option<Tray> {
        let item = NSStatusBar::systemStatusBar().statusItemWithLength(NSVariableStatusItemLength);
        let button = item.button(mtm)?;
        let image = make_image(tier)?;
        button.setImage(Some(&image));

        let target = TrayTarget::new();
        hook(&button, &target);

        let titles = menu_titles(mode).map(NSString::from_str);
        let empty = NSString::from_str("");
        let menu = NSMenu::new(mtm);
        // 关掉自动使能：它的判定要求条目有 `validateMenuItem:` 一类的宿主，而我们那个 target
        // 只有一个写意图的方法——开着它，三项可能整排被置灰。
        menu.setAutoenablesItems(false);
        let mut items: Vec<Retained<NSMenuItem>> = Vec::with_capacity(3);
        for (index, action) in menu_sels().into_iter().enumerate() {
            // SAFETY: `initWithTitle:action:keyEquivalent:` 是 `NSMenuItem` 的指定初始化器，
            // 首参要求刚 alloc、尚未初始化的内存（与上面 `make_image` 同一条纪律）。
            // 标题是刚造的 `NSString`，选择器是本文件里 `sel!` 出来的三颗已注册方法，
            // keyEquivalent 给空串（不抢系统按键）。
            let entry = unsafe {
                NSMenuItem::initWithTitle_action_keyEquivalent(
                    NSMenuItem::alloc(mtm),
                    &titles[index],
                    Some(action),
                    &empty,
                )
            };
            // SAFETY: 同 `hook`——`setTarget:` 是 weak，强引用在 `Tray.target` 里。
            unsafe { entry.setTarget(Some(&target)) };
            menu.addItem(&entry);
            items.push(entry);
        }
        Some(Tray {
            item,
            button,
            image,
            target,
            menu,
            // 上面正好挂三项，与 `menu_sels()` / `menu_titles()` 的 arity 同源
            items: items.try_into().ok()?,
            titles,
            tier,
        })
    }
}

#[cfg(target_os = "macos")]
pub use mac::{install, set_policy, sync};

/// 非 macOS 这一支只导出同名函数，恒 `Unavailable`：关窗因此整档退回今天的最小化语义，
/// 那一侧的编译面 SHALL NOT 引用 `mac` 里的任何一个 objc 符号（REQ-006）。
#[cfg(not(target_os = "macos"))]
pub fn install(_mode: UiMode, _ctx: &egui::Context, _scale: f64) -> TrayState {
    TrayState::Unavailable
}

#[cfg(not(target_os = "macos"))]
pub fn sync(_mode: UiMode) -> TrayState {
    TrayState::Unavailable
}

#[cfg(not(target_os = "macos"))]
pub const fn set_policy(_regular: bool) -> bool {
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 非透明像素的包围盒：`(最小列, 最大列, 最小行, 最大行)`
    fn bbox(image: &RgbaImage) -> (u32, u32, u32, u32) {
        let mut found: Option<(u32, u32, u32, u32)> = None;
        for (x, y, pixel) in image.enumerate_pixels() {
            if pixel.0[3] == 0 {
                continue;
            }
            found = Some(match found {
                Some((left, right, top, bottom)) => {
                    (left.min(x), right.max(x), top.min(y), bottom.max(y))
                }
                None => (x, x, y, y),
            });
        }
        found.expect("线稿不能是空的")
    }

    /// TC-5.4-a 的四条判据，@1x 与 @2x 各跑一遍。形状改了不必跟着改这四条断言。
    fn check(tier: Tier) {
        let image = glyph(tier);
        let side = tier.px();
        let last = side - 1;

        // ① 边长等于档位高度，而且是方的
        assert_eq!((image.width(), image.height()), (side, side));

        // ② 凡是不透明的像素，RGB 必须是纯黑：模板图靠 alpha 上色，出现彩色就是错的
        for (x, y, pixel) in image.enumerate_pixels() {
            if pixel.0[3] > 0 {
                assert_eq!(pixel.0, [0, 0, 0, pixel.0[3]], "({x},{y}) 带了颜色");
            }
        }

        // ③ 四角完全透明：图标不许顶到画布边角，那是"贴边的一团灰"的起点
        for (x, y) in [(0, 0), (last, 0), (0, last), (last, last)] {
            assert_eq!(image.get_pixel(x, y).0[3], 0, "角上 ({x},{y}) 被画到了");
        }

        // ④ 非透明包围盒水平居中（左右留白差 ≤1px），且四周各留 ≥1px
        let (left, right, top, bottom) = bbox(&image);
        assert!(left >= 1 && top >= 1, "左上留白被吃掉：{left},{top}");
        assert!(
            last - right >= 1 && last - bottom >= 1,
            "右下留白被吃掉：{right},{bottom}"
        );
        let imbalance = i64::from(last - right) - i64::from(left);
        assert!(imbalance.abs() <= 1, "线稿没居中：左右留白差 {imbalance}px");
    }

    #[test]
    fn the_glyph_is_a_black_alpha_mask_at_both_tiers() {
        check(Tier::One);
        check(Tier::Two);
    }

    /// 边长真的按档位走（@1x 22 / @2x 44），选档只看比例因子
    #[test]
    fn the_tier_follows_the_backing_scale() {
        assert_eq!(Tier::from_scale(1.0), Tier::One);
        assert_eq!(Tier::from_scale(1.5), Tier::One, "1.5 还不算视网膜");
        assert_eq!(Tier::from_scale(2.0), Tier::Two);
        assert_eq!(Tier::from_scale(3.0), Tier::Two);
        assert_eq!(Tier::One.px(), 22);
        assert_eq!(Tier::Two.px(), 44);
    }

    /// 交给 `NSImage::initWithData:` 的必须是**编码后的图片**，且尺寸就是那一档
    #[test]
    fn the_bytes_are_a_decodable_png_of_the_expected_side() {
        let bytes = glyph_bytes(Tier::One);
        let magic: [u8; 8] = [0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A];
        assert_eq!(&bytes[..8], &magic[..], "不是 PNG");
        let decoded = image::load_from_memory(&bytes).expect("托盘位图得是解得开的图");
        assert_eq!((decoded.width(), decoded.height()), (22, 22));
    }

    /// 第二项永远说"另一侧"的名字，且不得写已被改口的「悬浮窗」（任务 #44）
    #[test]
    fn the_second_menu_item_names_the_other_mode() {
        assert_eq!(menu_titles(UiMode::Full), ["显示窗口", "精简", "退出程序"]);
        assert_eq!(
            menu_titles(UiMode::Compact),
            ["显示窗口", "全量", "退出程序"]
        );
        for title in menu_titles(UiMode::Full)
            .iter()
            .chain(menu_titles(UiMode::Compact).iter())
        {
            assert!(!title.contains("悬浮窗"), "旧词又回来了：{title}");
        }
    }

    /// 三个意图走同一个槽：取走即清零、连点只兑现一次；且 `Quit` 必须映射到**既有**的
    /// `Action::Quit`——托盘不许是第二条退出实现（TC-5.4-b）。
    ///
    /// 几件事写在一个测试里是刻意的：槽是进程级的，拆成多个 `#[test]` 会被并行跑到，
    /// 彼此踩对方的残留。
    #[test]
    fn an_intent_is_handed_over_exactly_once_and_maps_onto_existing_actions() {
        assert_eq!(
            take_outcome(),
            None,
            "上一处的残留会让第一次点击凭空多切一刀"
        );
        for outcome in [Outcome::Reveal, Outcome::ToggleMode, Outcome::Quit] {
            mark(outcome);
            mark(outcome);
            assert_eq!(take_outcome(), Some(outcome));
            assert_eq!(take_outcome(), None, "取走之后必须清零");
        }
        assert_eq!(action_of(Outcome::Reveal), Action::RevealWindow);
        assert_eq!(action_of(Outcome::ToggleMode), Action::ToggleMode);
        assert_eq!(action_of(Outcome::Quit), Action::Quit);
    }

    /// 降级侧的文案要说"收进后台"（那是那一档真实发生的事），两支都不许出现"最小化"
    /// 这个词——它是用户最容易误解的说法，而界面从不按它执行（REQ-005）。
    #[test]
    fn the_settings_hint_matches_whichever_way_the_window_went() {
        assert_eq!(
            hint_text(TrayState::Available),
            "红灯 / Cmd+W 是收进托盘，不是退出"
        );
        for state in [TrayState::Unavailable, TrayState::Unknown] {
            let text = hint_text(state);
            assert_eq!(text, "红灯 / Cmd+W 是收进后台，不是退出");
            assert!(!text.contains("最小化"), "{text}");
        }
        assert_ne!(
            hint_text(TrayState::Available),
            hint_text(TrayState::Unavailable)
        );
    }

    /// 本包只承诺 macOS 这一侧（NFR-7：Windows/Linux 的托盘另立待办），不许把话说满
    #[test]
    fn support_is_declared_for_exactly_the_platform_we_verified() {
        assert_eq!(SUPPORTED, cfg!(target_os = "macos"));
    }
}
