//! 菜单栏托盘与"关窗去哪儿"（CHG-008：FR-5.4 + FR-5.2 前半句改判）。
//!
//! 用户 2026-11-03 那条报告有两件事：窗口点关闭不是收进托盘，而是躺在程序坞里。
//! 本模块把这件事改成**关窗 = 收进菜单栏**，并且把 Dock 磁贴一起收掉（用户在三个档位里
//! 选的中间那一档：运行期常驻，不是"全程只做菜单栏应用"）。四件不显然的事：
//!
//! 1. **图标是我们自己画的单色线稿**，不是 Dock 那张彩色方砖的缩小版。托盘那枚按钮走的是
//!    `setTemplate:` 的语义——"拿 alpha 当蒙版、由系统按深浅色上色"，给它一张彩图就等于
//!    既要形状又要颜色，系统只能按 alpha 走，结果是一团灰（REQ-002）。
//! 2. **左键与菜单项只立意图，右键的菜单就地弹出**。`define_class!` 出来的方法既拿不到
//!    `AppState` 也不该拿：那三件事把"用户点了什么"写进一个 `AtomicU8`，由 `app.rs` 取走并走
//!    既有那条 `Action` 通道——这是 `system::titlebar`、`system::dblclick` 之后的第三个同形状
//!    实例，不是巧合。但**菜单本身不等帧循环**：它是纯 AppKit，不需要 `AppState`，在回调里
//!    同步弹出反而少一层依赖（D-21 的另一半就丢在"等帧循环兑现"上，见第 3 条）。
//! 3. **泵必须挂在 `App::logic`，不能挂在 `App::ui`**。窗口 `orderOut` 之后 eframe 走的是
//!    `!show_ui` 那一支（`glow_integration.rs:708`），**只调 `logic()`、根本不调 `ui()`**；
//!    可见帧里 `logic()` 也照跑（`epi_integration.rs:357`），所以一帧一次、不多不少。
//!    装在 `ui()` 里等于"只在窗口已经看得见的时候才监听唤醒入口"——收进托盘之后意图永远躺在
//!    槽里，左键唤不回、右键也不出菜单，正是用户 2026-11-04 那句"关闭后点击托盘还是没反应"。
//!    `request_repaint()` 负责把睡着的事件循环叫醒（`run.rs:379` 对不可见窗节流到 100ms），
//!    为唤醒常驻一个事件监视器仍然不需要（REQ-007）。
//! 4. **整套生效或整套不生效**（REQ-006，本包最硬的否定式判据）。`TrayState` 是唯一分叉点：
//!    关窗去向、唤回入口、设置页文案、心跳四个消费者都读它。混合状态 = 窗口 `Visible(false)`
//!    收走了、磁贴被 `.accessory` 关了、唤醒入口又没装上——用户只剩"杀掉进程"一条路，
//!    正是 2026-10-27 那句"我没有办法重启软件"。装不上就整套退回今天的最小化语义。
//!
//! 左键与右键分叉靠 `NSApplication::currentEvent().r#type()`：`sendActionOn` 把按钮的触发
//! 掩码设成"左键抬起 + 右键抬起"，两种抬起都落到同一个方法上，方法里看当前事件是哪一种。
//! 菜单**从不常驻**挂在状态项上——`setMenu:` 挂上去之后左键就不走 action 了（spec Q-001），
//! 所以只用 `popUpStatusItemMenu:` 就地弹一次：它在追踪期间自己接管那枚状态项，返回后我们
//! 立刻把菜单摘干净并核对接线，左键那条路原样留着。

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
/// 菜单第三项。与设置页、「去设置」那一条共指 `Action::OpenSettings`，**不新增第二条实现**。
pub const SETTINGS: &str = "设置";
/// 菜单最后一项。它与设置页那颗、`Cmd+Q` 共指 `Action::Quit`，**不新增第二条退出实现**。
pub const QUIT: &str = "退出程序";

// ---- 意图槽：回调写、`App::logic` 读 ----

const CODE_NONE: u8 = 0;
const CODE_REVEAL: u8 = 1;
const CODE_TOGGLE_MODE: u8 = 2;
const CODE_QUIT: u8 = 3;
const CODE_SETTINGS: u8 = 4;

/// 托盘被点出来的那件事。一个槽而不是四个布尔：一次回调只可能立一个意图，
/// 四个标记会允许"同时既要退出又要唤回"这种根本不该存在的组合。
static PENDING: AtomicU8 = AtomicU8::new(CODE_NONE);

/// 唤醒源。隐藏的窗口收不到输入事件，`mark()` 必须能自己把事件循环叫起来，
/// 所以这里存一份 `egui::Context`（克隆廉价，内部是 `Arc`）。
static CONTEXT: OnceLock<egui::Context> = OnceLock::new();

/// 托盘转交给应用分派的四件事之一。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Outcome {
    /// 左键、或菜单第一项：把窗口叫回来
    Reveal,
    /// 菜单第二项：切到另一侧形态（与 `Cmd+Shift+M` 落在同一条 `Action` 上）
    ToggleMode,
    /// 菜单第三项：打开设置页（窗口收着时先唤回）
    Settings,
    /// 菜单最后一项：退出（与设置页那颗、`Cmd+Q` 同一条）
    Quit,
}

impl Outcome {
    const fn code(self) -> u8 {
        match self {
            Outcome::Reveal => CODE_REVEAL,
            Outcome::ToggleMode => CODE_TOGGLE_MODE,
            Outcome::Settings => CODE_SETTINGS,
            Outcome::Quit => CODE_QUIT,
        }
    }

    const fn from_code(code: u8) -> Option<Self> {
        match code {
            CODE_REVEAL => Some(Outcome::Reveal),
            CODE_TOGGLE_MODE => Some(Outcome::ToggleMode),
            CODE_SETTINGS => Some(Outcome::Settings),
            CODE_QUIT => Some(Outcome::Quit),
            _ => None,
        }
    }
}

/// 立一个意图并把事件循环叫醒。左键与三个菜单项的方法体里只做这一件事。
pub fn mark(outcome: Outcome) {
    PENDING.store(outcome.code(), Ordering::SeqCst);
    if let Some(ctx) = CONTEXT.get() {
        ctx.request_repaint();
    }
}

/// 取走意图，取完清零。一次泵里点多次只兑现一次：第二次点的时候上一件事已经在路上了
/// （与 `titlebar::take_click`、`dblclick::take_double_click` 同一纪律）。
pub fn take_outcome() -> Option<Outcome> {
    Outcome::from_code(PENDING.swap(CODE_NONE, Ordering::SeqCst))
}

/// 意图 → 既有动作通道。`Outcome::Quit` 必须映射到**已有**的 `Action::Quit`（TC-5.4-b）。
pub const fn action_of(outcome: Outcome) -> Action {
    match outcome {
        Outcome::Reveal => Action::RevealWindow,
        Outcome::ToggleMode => Action::ToggleMode,
        Outcome::Settings => Action::OpenSettings,
        Outcome::Quit => Action::Quit,
    }
}

/// 菜单四项的文案（TC-5.4-b 的无头判据挂在这上面）。
///
/// 第二项说的是"切到**另一侧**"的名字：当前在全量窗就写「精简」，在简洁窗就写「全量」。
/// 写「悬浮窗」是 2026-10-29 已被用户改口的旧词（任务 #44）。
pub fn menu_titles(mode: UiMode) -> [&'static str; 4] {
    [
        SHOW,
        if mode == UiMode::Full {
            "精简"
        } else {
            "全量"
        },
        SETTINGS,
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

/// 一只气泡的轮廓：外接矩形 + 圆角半径（22×22 点格子，y 向上）。
#[derive(Clone, Copy)]
struct Bubble {
    min: Pt,
    max: Pt,
    radius: f64,
}

/// 前一只：图标里左上那只有「文」的白气泡。
const FRONT: Bubble = Bubble {
    min: pt(2.6, 9.4),
    max: pt(11.2, 18.0),
    radius: 2.4,
};
/// 后一只：右下那只带「A」的蓝气泡。它落在前一只范围内的轮廓**不画**（见 `OVERLAP_GAP`），
/// 两只都画满就成一个"8"字，前后关系反而读不出来。
///
/// 位置照 `icon-1024.png` 量：往右挪约 8 点、往下挪约 2 点，两只只在边上交一小段——
/// 原图那两只就是并排贴着，不是叠成一团。
const BACK: Bubble = Bubble {
    min: pt(10.8, 7.2),
    max: pt(19.4, 15.8),
    radius: 2.4,
};
/// 前遮后的外放量：后一只的轮廓离前一只的边不到这个数就整段让位，留出一条缝。
/// 缝太宽，后一只就变成一根孤零零的括号；太窄，两只贴成一只。
const OVERLAP_GAP: f64 = 0.8;
/// 轮廓的笔宽（点）。1.2 在 22px 上是别的菜单栏图标那一档的粗细，再细就断。
const PEN: f64 = 1.2;
/// 尾巴比轮廓细一档：尾巴是"指向"，粗了就成了第二块主体。
const TAIL_PEN: f64 = 1.0;

/// 两条尾巴：前一只的尖朝左下、后一只的尖朝右下，与 `assets/icon/icon-1024.png` 同向。
///
/// 尖都落在底边的**一端**（前一只靠左、后一只靠右），这样才带得出方向；尖落在底边正中
/// 就成一个倒三角，读起来是"下面挂了个东西"，不是"这只气泡在说话"。
const fn tails() -> [Stroke; 4] {
    [
        seg(pt(3.4, FRONT.min.y), pt(4.2, 6.2), TAIL_PEN),
        seg(pt(4.2, 6.2), pt(6.6, FRONT.min.y), TAIL_PEN),
        seg(pt(15.0, BACK.min.y), pt(17.4, 4.4), TAIL_PEN),
        seg(pt(17.4, 4.4), pt(18.2, BACK.min.y), TAIL_PEN),
    ]
}

/// 把一只气泡按 `pad` 四周外扩，用来当另一只的遮挡范围。
const fn inflated(b: &Bubble, pad: f64) -> Bubble {
    Bubble {
        min: pt(b.min.x - pad, b.min.y - pad),
        max: pt(b.max.x + pad, b.max.y + pad),
        radius: b.radius + pad,
    }
}

/// 画这一档的位图：所有像素的 RGB 恒为纯黑，只有 alpha 有形状（模板图靠 alpha 上色）。
///
/// 形状是应用图标的双气泡轮廓，不是"随便找个简笔画顶上"——用户 2026-11-04 那句
/// "起码是我们原来图标的简笔画啊，这啥东西"。`assets/icon/icon-1024.png` 里那两只气泡
/// 是这软件唯一的身份，菜单栏那 22px 放不下「文」和「A」，但放得下它们的形状。
///
/// TC-5.4-a 那四条钉的是**边界行为**（边长、纯黑、四角透明、包围盒居中且四周留白），
/// 不是这几个具体数值——这张表可以改，四条判据不跟着改。
pub fn glyph(tier: Tier) -> RgbaImage {
    let side = tier.px();
    let scale = f64::from(side) / Tier::HEIGHT;
    let mut image = RgbaImage::from_pixel(side, side, Rgba([0, 0, 0, 0]));
    paint_bubble(
        &mut image,
        &BACK,
        scale,
        Some(&inflated(&FRONT, OVERLAP_GAP)),
    );
    for tail in tails() {
        paint(&mut image, &tail, scale);
    }
    paint_bubble(&mut image, &FRONT, scale, None);
    image
}

/// 把一个笔画刷进位图。像素取**中心点**判定，重叠处 alpha 取较大值。
fn paint(image: &mut RgbaImage, stroke: &Stroke, scale: f64) {
    let a = to_image(stroke.a, scale);
    let b = to_image(stroke.b, scale);
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

/// 线稿坐标 → 位图坐标。线稿按 AppKit 的习惯写（原点左下、y 向上），PNG 的第 0 行在**顶部**，
/// 所以落笔前翻一次。不翻的话"朝下的尾巴"会画成朝上——旧那张线稿就是这么错的。
fn to_image(p: Pt, scale: f64) -> Pt {
    pt(p.x * scale, (Tier::HEIGHT - p.y) * scale)
}

/// 一只气泡的轮廓刷进位图；`hide` 给了就把它范围内的像素全部让位（前一只遮后一只）。
///
/// 只扫这只形状自己的外接框，所以坐标分量都在画布尺寸以内，`hypot` 不可能溢出，
/// 也就不需要线段那一套 `capped_sq` 防护。
fn paint_bubble(image: &mut RgbaImage, bubble: &Bubble, scale: f64, hide: Option<&Bubble>) {
    let min = to_image(pt(bubble.min.x, bubble.max.y), scale);
    let max = to_image(pt(bubble.max.x, bubble.min.y), scale);
    let radius = bubble.radius * scale;
    let reach = PEN * scale * 0.5 + RAMP_PX;
    let mask = hide.map(|h| {
        let from = to_image(pt(h.min.x, h.max.y), scale);
        let to = to_image(pt(h.max.x, h.min.y), scale);
        Bubble {
            min: from,
            max: to,
            radius: h.radius * scale,
        }
    });
    let x0 = (min.x - reach).max(0.0) as u32;
    let x1 = (max.x + reach).min(f64::from(image.width()) - 1.0) as u32;
    let y0 = (min.y - reach).max(0.0) as u32;
    let y1 = (max.y + reach).min(f64::from(image.height()) - 1.0) as u32;
    for row in y0..=y1 {
        for col in x0..=x1 {
            let here = pt(f64::from(col) + 0.5, f64::from(row) + 0.5);
            if mask
                .as_ref()
                .is_some_and(|m| signed_border(&here, m) <= 0.0)
            {
                continue;
            }
            let dist = signed_border(&here, &Bubble { min, max, radius }).abs();
            let half = PEN * scale * 0.5;
            if dist > half + RAMP_PX {
                continue;
            }
            let alpha = if dist <= half {
                255.0
            } else {
                ((half + RAMP_PX - dist) / RAMP_PX * 255.0).max(0.0)
            };
            let pixel = image.get_pixel_mut(col, row);
            *pixel = Rgba([0, 0, 0, pixel.0[3].max(alpha as u8)]);
        }
    }
}

/// 点到圆角矩形**边界**的带符号距离：负=在里面，0=正上头，`abs()` 就是到轮廓的距离。
fn signed_border(p: &Pt, b: &Bubble) -> f64 {
    let hx = (b.max.x - b.min.x) * 0.5 - b.radius;
    let hy = (b.max.y - b.min.y) * 0.5 - b.radius;
    let qx = (p.x - (b.min.x + b.max.x) * 0.5).abs() - hx;
    let qy = (p.y - (b.min.y + b.max.y) * 0.5).abs() - hy;
    qx.max(0.0).hypot(qy.max(0.0)) + qx.max(qy).min(0.0) - b.radius
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
    use std::cell::RefCell;
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

    /// 菜单四项的选择器，与 `menu_titles` 一一对应。第一项与左键立**同一个**意图。
    fn menu_sels() -> [Sel; 4] {
        [
            sel!(iceTrayShowWindow:),
            sel!(iceTrayToggleMode:),
            sel!(iceTraySettings:),
            sel!(iceTrayQuit:),
        ]
    }

    define_class!(
        /// 托盘那枚按钮与菜单四项共同的 target。方法体各自只有一行——
        /// "要不要唤回、唤成什么"全部留在应用的分派里，与快捷键共用同一条 effects 路径。
        #[unsafe(super(NSObject))]
        #[name = "IceTranslatorTrayTarget"]
        struct TrayTarget;

        impl TrayTarget {
            // 选择器首词 `ice` 不属于任何 Objective-C 方法族，`define_class!` 自己推断；
            // 再手写 `#[unsafe(method_family = none)]` 会和 `#[unsafe(method(...))]` 冲突。
            #[unsafe(method(iceTrayClicked:))]
            fn clicked(&self, _sender: &AnyObject) {
                if current_event_is_right_click() {
                    // 右键就地弹菜单，**不等下一帧**：窗口收起来时 eframe 只跑 `App::logic`，
                    // 把"弹出"登记成旗子交给帧循环，等于交给了一个可能永远不来的读者（D-21）。
                    pop_menu();
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

            #[unsafe(method(iceTraySettings:))]
            fn settings(&self, _sender: &AnyObject) {
                mark(Outcome::Settings);
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
    }

    /// 装好的一切强引用。每个字段都有非显然的持有理由：
    /// - `item`：见上，松手图标就没了；
    /// - `target`：`NSControl.target` 与 `NSMenuItem.target` 都是**弱**引用（objc2 把
    ///   `setTarget:` 直接标成 weak property），我们一放手就是野指针，点一下访问已释放对象；
    /// - `menu`：状态项上的菜单是我们临时挂/摘的，不自己拿住就没有人拿住；
    /// - `image`：`setImage:` 只收引用，放手后按钮留下的是一张已被回收的图；
    /// - `button`：接线每帧都要问它，取一次存下来比每帧 `item.button(mtm)` 少一条消息；
    /// - `titles`：挂上去那四项用的字符串，留着才问得到"标题是不是已经对得上"——
    ///   每帧现造 `NSString` 是白白的常驻分配。
    struct Tray {
        item: Retained<NSStatusItem>,
        button: Retained<NSStatusBarButton>,
        image: Retained<NSImage>,
        target: Retained<TrayTarget>,
        menu: Retained<NSMenu>,
        items: [Retained<NSMenuItem>; 4],
        titles: [Retained<NSString>; 4],
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

    /// 每帧核对接线与菜单标题（`App::logic` 里跑，可见与收起两条路都会跑到）。
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
            TrayState::Available
        })
    }

    /// 右键：就地弹出那四项，弹完把菜单摘干净、接线再核一遍。
    ///
    /// 为什么不是"挂上 `menu` 再 `performClick:`"：那一支要等帧循环兑现，而窗口收起来时
    /// eframe 只跑 `App::logic`、`ui()` 里的一切都不来（D-21 就是这么丢的）。菜单是纯
    /// AppKit，不需要 `AppState`，在回调里同步走完就少一层依赖。
    ///
    /// `popUpStatusItemMenu:` 被标了 deprecated（苹果的说法是"改用 `menu` 属性"），但菜单
    /// 一常驻挂着左键就不走 action 了（Q-001）——这一支是"弹完把按钮还给我"的那个。
    ///
    /// 四份强引用先克隆、松开 `TRAY` 的借用再进循环：`popUpStatusItemMenu:` 是**同步阻塞**
    /// 的追踪循环，它在里面时再回头借同一个 `RefCell` 就是 double-borrow panic。
    fn pop_menu() {
        let handles = TRAY.with(|tray| {
            tray.borrow().as_ref().map(|tray| {
                (
                    tray.item.clone(),
                    tray.menu.clone(),
                    tray.button.clone(),
                    tray.target.clone(),
                )
            })
        });
        let Some((item, menu, button, target)) = handles else {
            return;
        };
        #[allow(deprecated)]
        item.popUpStatusItemMenu(&menu);
        item.setMenu(None);
        if !is_ours(&button, &target) {
            hook(&button, &target);
        }
    }

    /// 切激活策略：`false` = `.accessory`（Dock 磁贴消失），`true` = `.regular`（回来）。
    ///
    /// 这一句必须在主线程、且 `Visible(true)` **之前**发生：winit 的 `set_visible(true)`
    /// 走 `makeKeyAndOrderFront`，而 `.accessory` 态的窗口能不能成为 key window 由策略决定。
    ///
    /// 紧跟的是 `activateIgnoringOtherApps(true)` 而不是 `activate()`。后者的文档原话是
    /// "does not guarantee that the app will be activated at all"——它是**协作式**的，
    /// 别的应用正前台时系统可以直接拒绝（D-21 就是这个：托盘点了，图标亮了一下，窗口没回来）。
    /// 唤回是用户主动要求的，没有比这更明确的授权，所以用强制那一支；它确实被苹果标了
    /// deprecated（改用 `NSRunningApplication.activate(from:)`，那套要 macOS 14 且要
    /// `NSApplicationActivationReason`，我们的窗口本来就不是靠系统唤起来），故局部放行。
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
            #[allow(deprecated)]
            app.activateIgnoringOtherApps(true);
        }
        ok
    }

    /// 从零造出状态项：长度自适应、按钮接线、位图、菜单四项。
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
        // 只有一个写意图的方法——开着它，四项可能整排被置灰。
        menu.setAutoenablesItems(false);
        let mut items: Vec<Retained<NSMenuItem>> = Vec::with_capacity(4);
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
            // 上面正好挂四项，与 `menu_sels()` / `menu_titles()` 的 arity 同源
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

    /// 第二项永远说"另一侧"的名字，且不得写已被改口的「悬浮窗」（任务 #44）；
    /// 第三项是「设置」——用户 2026-11-04 点名要的两件事："右键起码的显示页面要有吧？打开设置要有吧？"
    #[test]
    fn the_second_menu_item_names_the_other_mode() {
        assert_eq!(
            menu_titles(UiMode::Full),
            ["显示窗口", "精简", "设置", "退出程序"]
        );
        assert_eq!(
            menu_titles(UiMode::Compact),
            ["显示窗口", "全量", "设置", "退出程序"]
        );
        for title in menu_titles(UiMode::Full)
            .iter()
            .chain(menu_titles(UiMode::Compact).iter())
        {
            assert!(!title.contains("悬浮窗"), "旧词又回来了：{title}");
        }
    }

    /// 四个意图走同一个槽：取走即清零、连点只兑现一次；且 `Quit` 必须映射到**既有**的
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
        for outcome in [
            Outcome::Reveal,
            Outcome::ToggleMode,
            Outcome::Settings,
            Outcome::Quit,
        ] {
            mark(outcome);
            mark(outcome);
            assert_eq!(take_outcome(), Some(outcome));
            assert_eq!(take_outcome(), None, "取走之后必须清零");
        }
        assert_eq!(action_of(Outcome::Reveal), Action::RevealWindow);
        assert_eq!(action_of(Outcome::ToggleMode), Action::ToggleMode);
        assert_eq!(action_of(Outcome::Settings), Action::OpenSettings);
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
