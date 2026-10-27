//! 窗口形态（FR-3.1 双模式、FR-3.4 置顶、FR-3.6 位置记忆、FR-5.2 关窗常驻），
//! 以及 CHG-004 加进来的贴边吸附（FR-3.5）与拖动松手判定。
//!
//! 切换是瞬时的：FR-3.9 在 2026-10-22 由"180~220ms 形变动画"改为即时切换（用户决定，
//! 真机上补间会把事件循环卡住约 1s，见 `CHG-004/design.md`）。
//! 命令的组装与吸附几何都是纯函数，可无头断言（沿用 P1 的 `queued(ctx)` 手法）。

use std::cmp::Ordering;
use std::time::{Duration, Instant};

use eframe::egui::{self, Pos2, Rect, Vec2, ViewportBuilder, ViewportCommand, WindowLevel};
use ice_core::config::{
    DockEdge, MIN_COMPACT_H, MIN_COMPACT_W, MIN_FULL_H, MIN_FULL_W, UiConfig, UiMode, WindowRect,
};

use crate::state::AppState;

/// 全量模式的最小内尺寸（`03-ui-design.md` §1）。数值出处是 `ice-core::config`，
/// 与 `Config::sanitized` 共用同一份定义，两边不会各写各的字面量。
pub const MIN_FULL_SIZE: Vec2 = Vec2::new(MIN_FULL_W, MIN_FULL_H);

/// 简洁模式的最小内尺寸（FR-3.2 改：可缩放，下限 320×160，无上限）
pub const MIN_COMPACT_SIZE: Vec2 = Vec2::new(MIN_COMPACT_W, MIN_COMPACT_H);

/// 隐藏期间的重绘心跳。
///
/// 最小化之后 eframe 不再跑 egui pass，只按 `MISSED_REDRAW_TIMEOUT`（250ms）调一次
/// `App::logic`（`update_logic_only`），`App::ui` 根本不执行——所以心跳必须由 `logic`
/// 里的 `request_repaint_after` 续上，否则没人再叫事件循环，从 Dock 叫回来时窗口就停在
/// 旧的那一帧上（缺陷 D-2）。
/// 250ms 与 eframe 自己的节流同档：再快也会被 `INVISIBLE_WINDOW_REPAINT_INTERVAL` 压回来。
pub const HIDDEN_HEARTBEAT: Duration = Duration::from_millis(250);

/// 某个形态对应的窗口命令。顺序有意义：装饰与约束先落，最后才改尺寸与原点。
///
/// `InnerSize` 走 `window.request_inner_size`，平台认为该尺寸不合法时**直接返回当前尺寸并
/// 忽略请求**（`egui-winit-0.37.0/src/lib.rs:1969-1985`）。P4 的简洁分支从不改 `MinInnerSize`，
/// 于是全量留下的 640×420 会把 460×130 夹掉——命令发了、窗口没变小，这就是走查缺陷 W-2.1。
///
/// 两种形态各发**恰好一条** `InnerSize` 与一条 `OuterPosition`（REQ-004）：切换是瞬时的，
/// 没有逐帧补间，多发一条就会多发一次原生 resize 事件。`OuterPosition` 对全量形态也下发
/// 是 CHG-004 的改动——从贴边停靠的悬浮条切回全量时不下发的话，窗口会留在屏幕边缘那条位置
/// 上，与 FR-3.6"记住位置尺寸"矛盾。
pub fn commands_for_mode(cfg: &UiConfig, mode: UiMode) -> Vec<ViewportCommand> {
    let rect = rect_for(cfg, mode);
    // 用户 2026-10-27 决定（Q-013）：简洁窗也要有原生顶部栏与最小化/关闭按钮。
    // 装饰两种形态都开，形态差别回到尺寸与下限上。
    let min_size = min_size_for(mode);
    vec![
        ViewportCommand::Decorations(true),
        ViewportCommand::Resizable(true),
        ViewportCommand::MinInnerSize(min_size),
        ViewportCommand::InnerSize(target_size(cfg, mode)),
        ViewportCommand::OuterPosition(Pos2::new(rect.x, rect.y)),
    ]
}

/// 该形态即将下发的目标内尺寸（简洁形态已经过下限夹取）。
///
/// 单列一个函数不是为了复用而复用：`apply` 要拿它记"待生效尺寸"（缺陷 D-6 的护栏），
/// 而那个值必须与命令里实际发出去的一字不差，否则护栏判的是另一个尺寸。
pub fn target_size(cfg: &UiConfig, mode: UiMode) -> Vec2 {
    let rect = match mode {
        UiMode::Full => rect_for(cfg, mode),
        UiMode::Compact => clamp_compact_rect(rect_for(cfg, mode)),
    };
    Vec2::new(rect.w, rect.h)
}

pub const fn rect_for(cfg: &UiConfig, mode: UiMode) -> WindowRect {
    match mode {
        UiMode::Full => cfg.full_rect,
        UiMode::Compact => cfg.compact_rect,
    }
}

/// 该形态的最小内尺寸。两个形态各带自己的下限，切换时谁也不把值留给对方。
pub const fn min_size_for(mode: UiMode) -> Vec2 {
    match mode {
        UiMode::Full => MIN_FULL_SIZE,
        UiMode::Compact => MIN_COMPACT_SIZE,
    }
}

/// 把小于下限的简洁窗尺寸抬到下限，位置不动。
///
/// `Config::sanitized` 在加载时已经夹过一次，这里是下发命令前的第二道：配置结构体可以被
/// 代码直接改（测试、以及后续 REQ-003 的停靠重算），不能假定它一定经过 `sanitized`。
/// 夹完再发 `InnerSize`，否则请求会被原生约束静默忽略，写回配置的又是另一个值。
pub fn clamp_compact_rect(rect: WindowRect) -> WindowRect {
    WindowRect {
        w: rect.w.max(MIN_COMPACT_W),
        h: rect.h.max(MIN_COMPACT_H),
        ..rect
    }
}

/// 唤出后指针离开多久自动收回（REQ-003、TC-3.5-g）。
///
/// 不能太短：从窗口移开鼠标去点别处的那零点几秒就会被当成"不看了"，窗口在使用者眼皮底下
/// 消失。500ms 比一次自然的移动停留长，又短到不会被当成"忘了收"。
pub const DOCK_AUTO_HIDE: Duration = Duration::from_millis(500);

/// 左右吸附（FR-3.5、REQ-003 v0.5）：松手后窗口离**左或右**哪条边近就贴哪条，两条都够远则自由浮动。
///
/// 判定是纯函数：输入窗口外框、可用区、阈值与热区宽度，输出新矩形与停靠边。
/// `area` 来自 `system::screen::desktop`——**整个桌面**所有屏可见区的并集，不是某一块屏
/// （D-8：按单屏判会把双屏用户的窗口藏进接缝，既盖不住也解不开）。
///
/// 两条公式逐字照抄 `spec.md` REQ-003 与 `docs/03-ui-design.md` §3：
///
/// ```text
/// 右 x = area.right - sliver      左 x = area.x - w + sliver
/// ```
///
/// **把窗口藏到那条边之外，可视区内只留 `sliver`（默认 3px）宽的一条热区**。3px 贴在屏幕
/// 最外沿，光标推到边必然落进它——这是"悬停唤出"的几何前提。
///
/// 上/下两条边整个从判定里删掉（v0.4 的四边对称作废，Q-010）：藏进菜单栏或 Dock 之后，
/// 那 3px 不在用户会去悬停的位置，"鼠标放上去自动出来"在那里不成立。
///
/// 夹取只分两种（吸附后再跑一次 `fit_into_area` 会把刚留出来的 sliver 一起夹没）：
/// - **交叉轴**（贴左右时夹 y）全量夹进可用区，不许窗口斜着飘出去；
/// - **停靠轴**只在窗口宽度超过可用区时把起点夹到 `area.min`，此时 `area.max - size` 是负数，
///   照抄会把窗口甩到屏幕另一侧之外（TC-3.5-e 要的正是拦住这一种）。
pub fn snap(window: Rect, area: Rect, threshold: f32, sliver: f32) -> (Rect, DockEdge) {
    let candidates = [
        (DockEdge::Left, window.min.x - area.min.x),
        (DockEdge::Right, area.max.x - window.max.x),
    ];
    let (edge, distance) = candidates
        .into_iter()
        .min_by(|a, b| a.1.total_cmp(&b.1))
        .expect("两条边总有最小值");
    // 两条边都够远（或距离不可比）：自由浮动，位置一个像素都不动。
    // 用 `partial_cmp` 而不是 `>=`：NaN 与阈值不可比，此时也必须停在原地。
    let Some(Ordering::Less) = distance.partial_cmp(&threshold) else {
        return (window, DockEdge::None);
    };
    (dock(window, area, edge, sliver), edge)
}

/// 把窗口藏到指定边外，只留 `sliver` 的可见热区。吸附与启动重落位共用这一条路径（REQ-003）。
pub fn dock(window: Rect, area: Rect, edge: DockEdge, sliver: f32) -> Rect {
    let size = window.size();
    let x = match edge {
        DockEdge::Right => docked_axis(area.max.x - sliver, size.x, area.min.x, area.max.x),
        DockEdge::Left => docked_axis(area.min.x - size.x + sliver, size.x, area.min.x, area.max.x),
        // `Top`/`Bottom` 运行时不再产生（`snap` 只出左/右，配置读到即在 `sanitized` 里置 `None`），
        // 这里保持全匹配、原样返回：越界停靠比不停靠更糟，宁可不动。
        _ => return window,
    };
    Rect::from_min_size(
        Pos2::new(x, cross_axis(window.min.y, size.y, area.min.y, area.max.y)),
        size,
    )
}

/// 悬停唤出位（REQ-003、TC-3.5-f）：整条边齐、窗口完整可见，**不改停靠边**。
///
/// 右 → `x = area.max.x - w`；左 → `x = area.min.x`。y 沿用现值并夹进可用区。
/// 唤出只是运行时状态（`app.rs` 的 `dock_shown`），盘上记的仍是停靠边：强杀后重启会由
/// REQ-003 的启动落位重新按 `dock(...)` 收回去，不会以"停靠"之名整条可见地挂着。
pub fn reveal_rect(window: Rect, area: Rect, edge: DockEdge) -> Rect {
    let size = window.size();
    let x = match edge {
        DockEdge::Right => area.max.x - size.x,
        DockEdge::Left => area.min.x,
        _ => return window,
    };
    Rect::from_min_size(
        Pos2::new(x, cross_axis(window.min.y, size.y, area.min.y, area.max.y)),
        size,
    )
}

/// 指针（窗口内坐标）是否落在**创口带**里：停靠热区再沿停靠轴向屏幕内扩 `sliver` 的那一条。
///
/// 闸门（`dock_latched`）的唯一解除条件是"指针离开创口带后再次进入"。判定用窗口内坐标而不是
/// 屏幕坐标，因为唤出与收回都会整条挪动窗口，而指针在窗口里的相对位置恰好是热区本身。
/// `sliver * 2.0` 的余量是给 Retina 亚像素和"贴着边一点点"的手抖留的：只算 3px 的话
/// 光标停在热区边缘会在一半像素上反复进出，唤出/收回又抖起来。
pub fn dock_band_contains(window: Rect, edge: DockEdge, pointer: Pos2, sliver: f32) -> bool {
    let band = sliver * 2.0;
    match edge {
        DockEdge::Right => pointer.x - window.min.x <= band,
        DockEdge::Left => window.max.x - pointer.x <= band,
        _ => false,
    }
}

/// 已停靠时判"拖离"（REQ-003 第三条、TC-3.5-d）：松手原点相对**停靠锚点**沿停靠轴
/// 向屏幕内移动的距离 ≥ 阈值即解除停靠。
///
/// 不能共用 `snap` 里那个"窗口到边线的几何距离"：窗口藏到边外之后，那条边的距离恒为
/// 约 `-(size - sliver)`（460 宽的窗、sliver 3 时约 -457），总是两边里最小的，
/// `snap` 会把"向内拖 200px"重新吸回原边，拖离永远不成立。
///
/// `anchor` 必须是停靠那一刻下发的那个原点，不能取 `config.ui.compact_rect`——
/// `capture` 每帧按实时几何改写它，拖动途中就被冲掉、位移恒为 0。锚点本身在吸附、唤出、
/// 收回三处跟着搬（REQ-003）：不搬唤出位的话，"从唤出态向内拖"量不出真实位移。
///
/// 只算停靠轴：交叉轴（沿边滑动）的位移不判拖离，停靠态应当保留。
pub fn released(edge: DockEdge, anchor: Pos2, settled: Pos2, threshold: f32) -> bool {
    let inward = match edge {
        DockEdge::Right => anchor.x - settled.x,
        DockEdge::Left => settled.x - anchor.x,
        // 上/下停靠已随 v0.5 下线，保留分支只为 `DockEdge` 全匹配
        _ => return false,
    };
    matches!(
        inward.partial_cmp(&threshold),
        Some(Ordering::Greater) | Some(Ordering::Equal)
    )
}

/// 交叉轴：整条夹进可用区，窗口不许斜着飘出去
fn cross_axis(pos: f32, size: f32, lo: f32, hi: f32) -> f32 {
    if size > hi - lo {
        lo
    } else {
        pos.clamp(lo, hi - size)
    }
}

/// 停靠轴：只在窗口比可用区还长时夹到起点，其余情况保留 `dock` 算出的热区/外溢
fn docked_axis(pos: f32, size: f32, lo: f32, hi: f32) -> f32 {
    if size > hi - lo { lo } else { pos }
}

/// 离屏夹回（REQ-009）：宽高超过可用区时先夹到可用区大小，再把位置夹进
/// `[area.min, area.max - size]`。用于启动路径与"休眠唤醒/插拔后坐标失效"，
/// **不用于** `snap` 之后——见 `snap` 的文档，全量夹取会把停靠热区夹没。
pub fn fit_into_area(rect: Rect, area: Rect) -> Rect {
    let size = Vec2::new(
        rect.width().min(area.width()).max(0.0),
        rect.height().min(area.height()).max(0.0),
    );
    Rect::from_min_size(
        Pos2::new(
            cross_axis(rect.min.x, size.x, area.min.x, area.max.x),
            cross_axis(rect.min.y, size.y, area.min.y, area.max.y),
        ),
        size,
    )
}

/// 拖动/缩放静止判定阈值。
///
/// 原生拖拽期间 egui 收不到任何 pointer 事件（`StartDrag` 把控制权交给了 AppKit），
/// "松手"这个时刻应用侧无从得知，只能看几何：连续这么久不变就当松手了。
/// 150ms 比一帧长得多、又短到用户感觉不出延迟。
pub const DRAG_SETTLE: Duration = Duration::from_millis(150);

/// 几何比较的容差（px）。`outer_rect` 每帧回读，缩放因子换算会带来亚像素抖动，
/// 不容差就永远判不出"静止"。
const SETTLE_TOLERANCE: f32 = 0.5;

/// 静止判定器：`start_drag` 发出后 `arm`，此后每帧 `feed` 一次窗口外框，
/// 连续 `DRAG_SETTLE` 不变就返回最终矩形（**只返回一次**，随后自动解除）。
///
/// 返回后必须解除，否则吸附把窗口挪到新位置 → 几何又变了 → 150ms 后再返回一次，
/// 如此往复每 150ms 写一次盘（REQ-009 的落盘节流会被打穿）。
#[derive(Debug, Default)]
pub struct DragSettler {
    armed_at: Option<Instant>,
    last: Option<Rect>,
    last_move: Option<Instant>,
}

impl DragSettler {
    pub fn arm(&mut self, now: Instant) {
        *self = Self {
            armed_at: Some(now),
            last: None,
            last_move: None,
        };
    }

    pub fn feed(&mut self, outer: Option<Rect>, now: Instant) -> Option<Rect> {
        // 不在拖动过程中：既不判静止，也不刷新任何计时
        self.armed_at?;
        let Some(rect) = outer else {
            // 读不到几何就无从判断有没有在动，按"还在动"处理并刷新计时：
            // 沿用旧计时会在拖动中途判出静止，把窗口当场吸到边上
            self.last_move = Some(now);
            return None;
        };
        let settled = match (self.last, self.last_move) {
            (Some(prev), Some(at)) => {
                if moved(prev, rect) {
                    self.last_move = Some(now);
                    false
                } else {
                    now.duration_since(at) >= DRAG_SETTLE
                }
            }
            // 刚 arm 的第一帧：把当前几何当基准，从这一刻开始计时
            _ => {
                self.last_move = Some(now);
                false
            }
        };
        self.last = Some(rect);
        if !settled {
            return None;
        }
        *self = Self::default();
        Some(rect)
    }

    /// arm 之后一直没等到静止（例如切了形态）：调用方可以主动放弃这次判定
    pub fn disarm(&mut self) {
        *self = Self::default();
    }

    /// 还在等静止。`app.rs` 靠它决定要不要续一帧重绘：
    /// 原生拖拽结束后事件循环可能没有别的理由再画一帧，不续就永远等不到"静止满 150ms"。
    pub const fn is_armed(&self) -> bool {
        self.armed_at.is_some()
    }
}

fn moved(prev: Rect, next: Rect) -> bool {
    (prev.min - next.min).length() > SETTLE_TOLERANCE
        || (prev.size() - next.size()).length() > SETTLE_TOLERANCE
}

/// 启动形态直接由保存的模式决定，避免"先开大窗再缩成简洁窗"的闪烁（FR-3.1）
pub fn initial_viewport(cfg: &UiConfig) -> ViewportBuilder {
    let rect = rect_for(cfg, cfg.mode);
    let min = min_size_for(cfg.mode);
    ViewportBuilder::default()
        .with_title("Ice Translator")
        .with_inner_size([rect.w, rect.h])
        .with_position([rect.x, rect.y])
        .with_decorations(true)
        .with_resizable(true)
        // 失焦半透明（REQ-007）要靠**窗口**的 alpha 通道，而 `ViewportCommand::Transparent`
        // 在 macOS 上不保证重建 surface，所以这个属性只能在创建期给（T-001 的 H3 实测成立：
        // 透明区不吞点击、也不影响绘制，底色不透明时观感与不设一致）。
        .with_transparent(true)
        .with_min_inner_size([min.x, min.y])
        .with_window_level(level_for_dock(
            is_docked(cfg.docked_edge, cfg.mode),
            cfg.always_on_top,
        ))
}

/// 失焦时窗体底色的不透明度（FR-3.7、REQ-007）。
///
/// 只有"简洁形态 + 没有焦点 + 用户没关这个开关"才降到 85%，其余一律 100%：
/// 全量窗是要盯着看内容的，不该因为点了一下别处就变淡。
pub const fn dim_alpha(mode: UiMode, focused: bool, dim_enabled: bool) -> f32 {
    if matches!(mode, UiMode::Compact) && !focused && dim_enabled {
        DIM_ALPHA
    } else {
        1.0
    }
}

/// 失焦半透明降到的不透明度（"透 15%"）
const DIM_ALPHA: f32 = 0.85;

pub const fn window_level(always_on_top: bool) -> WindowLevel {
    if always_on_top {
        WindowLevel::AlwaysOnTop
    } else {
        WindowLevel::Normal
    }
}

/// 当前配置是不是处于停靠态（REQ-003）。
///
/// 停靠只在简洁形态成立：全量窗带着系统标题栏，藏到边外既唤不出来也拖不动，
/// 而且从贴边的悬浮条切回全量时窗口必须回到 `full_rect`，不能被当成"还停着"。
pub const fn is_docked(edge: DockEdge, mode: UiMode) -> bool {
    matches!(mode, UiMode::Compact) && matches!(edge, DockEdge::Left | DockEdge::Right)
}

/// 停靠态的窗口层级（REQ-003、Q-012）：只要停在左/右边上就一律浮在其他应用之上，
/// 不论用户的 `always_on_top` 为何值。
///
/// 那 3px 热区贴在屏幕最外沿，`Normal` 层级的窗口会被别的 app 压住，鼠标推到边上根本
/// 点不到它、也收不到进入事件——悬浮条就再也唤不出来（缺陷 D-7 的成因之一）。解除停靠后
/// 恢复用户设定。
///
/// 一处事实更正：spec 与 design 写的 `WindowLevel::Floating` 在 egui 0.37 里**没有这个变体**
/// （`egui-0.37.0/src/viewport.rs:969-974` 只有 `Normal`/`AlwaysOnBottom`/`AlwaysOnTop`）。
/// 要的是 AppKit 那个 `.floating` 层级，而 winit 正是把 `AlwaysOnTop` 映射到它
/// （`egui-winit-0.37.0/src/lib.rs:2086-2090` → `winit::window::WindowLevel::AlwaysOnTop`），
/// 所以这里取 `AlwaysOnTop`，行为与规格描述一致。
pub const fn level_for_dock(docked: bool, always_on_top: bool) -> WindowLevel {
    if docked {
        WindowLevel::AlwaysOnTop
    } else {
        window_level(always_on_top)
    }
}

/// 待生效几何的宽限期（缺陷 D-6）。
///
/// 探针实测单发几何命令 **20ms** 落定（`features/CHG-004/evidence/T-001-probe-v3.log`），
/// 这里给 50 倍余量：足够挡住切换后的过渡帧，又不至于在原生就是不肯给那个尺寸时
/// 永久挡着用户手动缩放写盘。
pub const MODE_GEOMETRY_GRACE: Duration = Duration::from_millis(1000);

/// 判定"目标尺寸已经落定"的容差（px）。
const PENDING_SIZE_TOLERANCE: f32 = 2.0;

/// 形态切换后"已经发出、还没落到窗口上"的目标内尺寸（缺陷 D-6 的护栏，由 `IceApp` 持有）。
///
/// `commands_for_mode` 下发的是一组**请求**（`InnerSize` 走 winit 的 `request_inner_size`，
/// 异步），而 `capture` 每帧把实时几何写回**当前形态**的配置槽。切换那一帧之后 `mode` 已经是
/// Compact、窗口却还是全量的大数据，于是 `compact_rect` 被写成 642×422，此后再没有任何人发
/// 缩小请求——简洁窗就此永久停在大数据上（2026-10-23 走查实证：盘上 `compact_rect` 的宽高与
/// `full_rect` 一字不差；反证：全新启动、不经形态切换时那个默认尺寸不会被改写）。
///
/// 护栏只挡**尺寸轴**：位置照写，用户可能在切换的同一瞬间拖动窗口。
///
/// ## 三段（v0.7，缺陷 D-10）
///
/// v0.6 只有一个 `target`，宽限期一过就无条件放行——于是"原生没给那个尺寸"这种最坏情况里，
/// 第一个量到的**别的形态**的尺寸被当成"用户选的尺寸"永久写进本槽；两槽一旦相等，下一帧
/// `measured == target` 恒成立，`landed` 分支每帧命中，护栏形同不存在，坏状态不可自愈。
/// 现在改成"重发一次 → 再把当前值当成静默起点"：
#[derive(Debug)]
struct Stage {
    target: Vec2,
    since: Instant,
    /// 第一段到期时是否已经把重发请求交给过调用方
    resent: bool,
}

#[derive(Debug, Default)]
pub struct PendingGeometry {
    stage: Option<Stage>,
    /// 第二段到期时量到的实测尺寸：它成为"窗口没动"的参照，与它相同就不写盘
    baseline: Option<Vec2>,
    /// 等调用方取走的重发目标（第一段到期的那一帧立起）
    retry: Option<Vec2>,
}

impl PendingGeometry {
    /// 下发一组几何命令时记下目标内尺寸。新的切换无条件作废上一段的基线。
    pub fn set(&mut self, target: Vec2, now: Instant) {
        self.stage = Some(Stage {
            target,
            since: now,
            resent: false,
        });
        self.baseline = None;
        self.retry = None;
    }

    /// 还有没有"发出去了、窗口还没跟上"的尺寸。帧循环用它判断切换是否已经落定（REQ-004）。
    pub const fn is_pending(&self) -> bool {
        self.stage.is_some()
    }

    /// 第一段到期后交给调用方重发的那组命令的目标尺寸；取走即清空，一轮只给一次。
    ///
    /// 重发不能在这里自己发（`PendingGeometry` 拿不到 `Context`，也不该拿）。
    pub fn take_retry(&mut self) -> Option<Vec2> {
        self.retry.take()
    }

    /// 测试专用：拨护栏里的计时，替掉"真等两个宽限期"。
    ///
    /// 挪 `since` 而不是直接塞状态，`capture` 里那句 `Instant::now()` 才仍按正常路径判到期。
    /// `resent = false` 用来演练第一段到期（该重发），`true` 用来演练第二段到期（该拒绝采纳）。
    #[cfg(test)]
    pub(crate) fn set_stage_clock_for_tests(&mut self, since: Instant, resent: bool) {
        if let Some(stage) = self.stage.as_mut() {
            stage.since = since;
            stage.resent = resent;
        }
        if !resent {
            self.retry = None;
        }
    }

    /// 这一帧量到的内尺寸能不能写进配置的尺寸轴。
    ///
    /// 四种情形：
    /// 1. 命中目标（容差内）→ 落定，清状态，放行。
    /// 2. 没命中、宽限期也没到 → 拦住（`false`），沿用槽里的旧尺寸。
    /// 3. 没命中、第一段到期 → 立起 `retry` 让调用方重发一次，重置计时，这一帧仍拦住。
    /// 4. 没命中、第二段到期 → **不采纳实测值**。把它记成 `baseline`、清掉 `stage`、记一条
    ///    WARN，此后与 `baseline` 相同的帧一律不写（窗口停在哪儿都不算用户的意思），
    ///    只有量到与 `baseline` 不同的尺寸才认定是用户亲手在拉窗口，交还尺寸轴所有权。
    pub fn size_axis_writable(&mut self, measured: Vec2, now: Instant) -> bool {
        if let Some(base) = self.baseline {
            let user_moved = (measured - base).length() > PENDING_SIZE_TOLERANCE;
            if user_moved {
                self.baseline = None;
            }
            return user_moved;
        }
        let Some(stage) = self.stage.take() else {
            return true;
        };
        if (measured - stage.target).length() <= PENDING_SIZE_TOLERANCE {
            return true;
        }
        if now.duration_since(stage.since) >= MODE_GEOMETRY_GRACE {
            if stage.resent {
                self.retry = None;
                self.baseline = Some(measured);
            } else {
                self.stage = Some(Stage {
                    target: stage.target,
                    since: now,
                    resent: true,
                });
                self.retry = Some(stage.target);
            }
        } else {
            self.stage = Some(stage);
        }
        false
    }
}

/// 重发用的命令（REQ-004 v0.7 的"至多两组"）。
///
/// 只重发尺寸与位置：`Decorations`/`Resizable`/`MinInnerSize` 在切换那一组里已经生效、
/// 也不随尺寸变，整套重发等于把"各至多 2 条"变成"各 2 条外加三条无意义的重复"。
/// 位置取槽里当前值（`capture` 刚写过），免得重发把用户这一秒拖到的位置又拽回去。
pub fn resend_commands(cfg: &UiConfig, mode: UiMode, target: Vec2) -> Vec<ViewportCommand> {
    let rect = rect_for(cfg, mode);
    vec![
        ViewportCommand::InnerSize(target),
        ViewportCommand::OuterPosition(Pos2::new(rect.x, rect.y)),
    ]
}

/// 切到指定形态。源文、译文、语言对、页签一律保留，且不重发请求（FR-3.3）。
///
/// 窗口收起时（`state.hidden`）只改配置里的目标形态，一条几何命令都不发，也不把窗口
/// 叫回来（REQ-004 的边界）：全局热键 `Cmd+Shift+M` 在收起态照样按得到，按完弹回来的是
/// 目标形态的几何——`reveal` 本来就会按当前 `config.ui.mode` 整套重发一次。
pub fn apply(
    ctx: &egui::Context,
    state: &mut AppState,
    mode: UiMode,
    pending: &mut PendingGeometry,
) {
    if state.hidden {
        state.config.ui.mode = mode;
        state.mark_config_dirty();
        return;
    }
    // 先把当前几何记回配置，否则切回来时会跳到上一次的旧位置
    capture(ctx, state, pending);
    state.config.ui.mode = mode;
    state.mark_config_dirty();
    let target = target_size(&state.config.ui, mode);
    for command in commands_for_mode(&state.config.ui, mode) {
        ctx.send_viewport_cmd(command);
    }
    pending.set(target, Instant::now());
    apply_topmost(ctx, state);
    ctx.request_repaint();
}

pub fn toggle(ctx: &egui::Context, state: &mut AppState, pending: &mut PendingGeometry) -> UiMode {
    let mode = state.toggle_mode();
    apply(ctx, state, mode, pending);
    mode
}

/// 下发当前应该有的窗口层级：停靠态强制 `AlwaysOnTop`（AppKit 的 `.floating`，Q-012），其余按用户的置顶开关（FR-3.4）。
pub fn apply_topmost(ctx: &egui::Context, state: &mut AppState) {
    ctx.send_viewport_cmd(ViewportCommand::WindowLevel(level_for_dock(
        is_docked(state.config.ui.docked_edge, state.config.ui.mode),
        state.config.ui.always_on_top,
    )));
}

/// 配置落盘的最短间隔（`app::flush_config` 的节流窗口）。
///
/// 拖动窗口时 `capture` 每帧都会把配置标脏，逐帧原子写盘没有意义；半秒去抖一次，
/// 退出与 Cmd+Q 走 `force`，不会丢改动。放在这里是因为 `capture` 也要它：
/// 几何最后一次变化后要自己把"500ms 后再来一帧"续上，节流窗口不能两处各写一个数。
pub const CONFIG_SAVE_INTERVAL: Duration = Duration::from_millis(500);

/// 记住窗口位置与大小（FR-3.6）。屏幕拔掉后旧坐标会落到可用区外，启动时回屏幕中心。
///
/// `pending` 是 D-6 的护栏：切换后还没落定的那几帧只写位置轴，尺寸轴沿用槽里的旧值。
///
/// 几何一变就立一个 `CONFIG_SAVE_INTERVAL` 的重绘钟（REQ-009）：`flush_config(false)`
/// 只在有帧跑的时候才被调用，而拖动结束后没有任何东西再请求重绘——脏标记会一直挂着，
/// 强杀就丢（SIGTERM 不走 `on_exit`，P1 实测）。
pub fn capture(ctx: &egui::Context, state: &mut AppState, pending: &mut PendingGeometry) {
    let (inner, outer) = ctx.input(|i| {
        let viewport = i.viewport();
        (viewport.inner_rect, viewport.outer_rect)
    });
    let Some(rect) = capture_rect(inner, outer) else {
        return;
    };
    let mode = state.config.ui.mode;
    // 日志要用的两个数在写回之前取，写回后它们就是"被保住的那个尺寸轴"了
    let target = target_size(&state.config.ui, mode);
    let slot = if mode == UiMode::Full {
        &mut state.config.ui.full_rect
    } else {
        &mut state.config.ui.compact_rect
    };
    let kept = Vec2::new(slot.w, slot.h);
    let measured = Vec2::new(rect.w, rect.h);
    let was_pending = pending.is_pending();
    let writable = pending.size_axis_writable(measured, Instant::now());
    if was_pending && !writable && !pending.is_pending() {
        // 第二段到期：护栏从"先挡一挡"改判成"不采纳"。这是这次切换唯一一次说明，
        // 只打尺寸数字——路径与任何用户内容不进日志（NFR-3）。
        tracing::warn!(
            "尺寸请求两轮宽限期都没落定，本槽保持 {}×{}（目标 {}×{}，实测 {}×{}），此后只有你手动缩放才会改尺寸",
            kept.x,
            kept.y,
            target.x,
            target.y,
            measured.x,
            measured.y
        );
    }
    let rect = if writable {
        rect
    } else {
        keep_pending_size(rect, *slot)
    };
    if *slot != rect {
        *slot = rect;
        state.mark_config_dirty();
        ctx.request_repaint_after(CONFIG_SAVE_INTERVAL);
    }
}

/// 护栏生效时的写回值：位置用这一帧量到的，尺寸沿用槽里的（也就是刚下发的那个目标）。
fn keep_pending_size(measured: WindowRect, previous: WindowRect) -> WindowRect {
    WindowRect {
        w: previous.w,
        h: previous.h,
        ..measured
    }
}

/// 位置取外框（含阴影/标题栏的原点），尺寸取内框（`with_inner_size` 的语义）。
/// 任一还没有值时（第一帧）返回 `None`，不覆盖已保存的配置。
///
/// 外框缺失时不能退而用内框原点：那会把标题栏高度算进位置，每启动一次窗口就往下挪一截。
fn capture_rect(inner: Option<Rect>, outer: Option<Rect>) -> Option<WindowRect> {
    let size = inner?.size();
    let origin = outer?.min;
    Some(WindowRect {
        x: origin.x,
        y: origin.y,
        w: size.x,
        h: size.y,
    })
}

/// 关窗常驻（FR-5.2）：不退出进程，只把窗口收起来。
///
/// 用最小化而不是 `Visible(false)`：eframe 0.37 从不把 macOS 的 `Reopen` 事件传上来，
/// 真隐藏之后点 Dock 图标无法恢复，而单实例锁又会让"重开一个"直接退出。
/// 最小化态由 AppKit 自己在 Dock 里留缩略图，点一下就回来，不需要应用侧代码。
pub fn hide(ctx: &egui::Context, state: &mut AppState) {
    // 由窗口关闭按钮触发时，必须在同一帧里撤销关闭，否则 eframe 会直接结束 run loop
    ctx.send_viewport_cmd(ViewportCommand::CancelClose);
    ctx.send_viewport_cmd(ViewportCommand::Minimized(true));
    state.hidden = true;
    tracing::debug!("收进后台（最小化，进程不退出）");
    ctx.request_repaint();
}

/// 窗口是不是已经从最小化回来了。
///
/// 不能看 `viewport().minimized`：`egui_winit::update_viewport_info` 只在 `is_init` 时填这个字段
/// ——运行时去问 `window.is_minimized()` 在 macOS 上会死锁（emilk/egui#3494）。于是我们自己发的
/// `Minimized(true)` 之后，它永远停在 `Some(true)`，拿它判恢复的话一次都不会成立。
///
/// 但同一个函数会按 `is_minimized()` 决定要不要填 `inner_rect`：最小化时是 `None`，
/// 恢复后才有值。所以改用"有没有真实几何"来判断——这个字段运行时是刷新的。
pub const fn is_revealed(inner_rect: Option<Rect>) -> bool {
    inner_rect.is_some()
}

/// 隐藏后的判恢复静默期（缺陷 D-2 第二轮）。
///
/// 收起的那一帧和 macOS 最小化动画期间，`focused` 还是 `Some(true)`、`inner_rect` 也还在，
/// 拿它们当"回来了"的证据就会把刚收起来的窗口当场弹回去（Cmd+W 路径尤其明显）。
/// 所以隐藏之后这段时间内一律不判恢复；超过之后宁可误判一次重发几何（窗口本来就可见时
/// `reveal` 是幂等的），也不能冻住。
pub const REVEAL_GRACE: Duration = Duration::from_millis(800);

/// 恢复判定的去抖器。每帧喂一次窗口状态，返回"这一帧该不该 `reveal`"。
///
/// 判据有两条，静默期一过任意一条成立就算恢复：
/// - **几何回来了**（`inner_rect` 有值）：最硬的证据；
/// - **窗口拿到焦点**：`inner_rect` 在 macOS 上可能卡住不回来（`update_viewport_info`
///   按 `is_minimized()` 决定填不填，而那个值运行时不可靠），但 `focused` 每帧都刷新，
///   最小化的窗口不可能是 key window，所以焦点回来就等于窗口回来了。
#[derive(Debug, Default)]
pub struct RevealGuard {
    hidden_at: Option<Instant>,
    /// 上一次看到的 (有没有几何, 有没有焦点)，只在变化时打日志，避免心跳刷屏
    last_seen: Option<(bool, bool)>,
}

impl RevealGuard {
    /// 收起窗口的那一刻调用，重新开始计时
    pub fn on_hide(&mut self, now: Instant) {
        self.hidden_at = Some(now);
        self.last_seen = None;
    }

    pub fn check(
        &mut self,
        hidden: bool,
        now: Instant,
        inner_rect: Option<Rect>,
        focused: Option<bool>,
    ) -> bool {
        let Some(hidden_at) = self.hidden_at.filter(|_| hidden) else {
            *self = Self::default();
            return false;
        };
        let revealed = is_revealed(inner_rect);
        let focused = focused == Some(true);
        let seen = (revealed, focused);
        if self.last_seen.replace(seen) != Some(seen) {
            tracing::debug!(
                revealed,
                focused,
                elapsed_ms = now.duration_since(hidden_at).as_millis() as u64,
                "隐藏期间的窗口状态变了"
            );
        }
        now.duration_since(hidden_at) >= REVEAL_GRACE && (revealed || focused)
    }
}

/// 窗口重新可见：清标记，并把当前形态的几何命令再发一次（进程从未退出，内容原样）。
///
/// 重发几何不是为了改窗口，是为了逼 winit 再走一次 surface 尺寸同步：最小化期间
/// `inner_rect` 是 `None`，恢复时若那次 `Resized` 没被处理，GL surface 会停在旧尺寸上，
/// 画面就成了一张被非等比拉伸的旧图——文字变窄变扁、鼠标放上去也没有任何反馈。
pub fn reveal(ctx: &egui::Context, state: &mut AppState, pending: &mut PendingGeometry) {
    state.hidden = false;
    tracing::debug!(mode = ?state.config.ui.mode, "窗口恢复可见，重发几何");
    // 与 hide 的 `Minimized(true)` 对上：AppKit 那边若还认为自己是最小化的，
    // 这一步才真的把它叫回来；已经可见时是空操作。
    ctx.send_viewport_cmd(ViewportCommand::Minimized(false));
    let mode = state.config.ui.mode;
    let target = target_size(&state.config.ui, mode);
    for command in commands_for_mode(&state.config.ui, mode) {
        ctx.send_viewport_cmd(command);
    }
    // 重发同样是一组异步请求，护栏要跟着重新上（D-6）
    pending.set(target, Instant::now());
    apply_topmost(ctx, state);
    ctx.request_repaint();
}

/// Cmd+Q：先落盘再让窗口真的关掉。`on_exit` 里还会再存一次，这里保证顺序。
pub fn quit(ctx: &egui::Context, state: &mut AppState, pending: &mut PendingGeometry) {
    capture(ctx, state, pending);
    state.quitting = true;
    ctx.send_viewport_cmd(ViewportCommand::Close);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::KeyState;
    use ice_core::config::Config;
    use ice_core::glossary::Glossary;

    fn queued(ctx: &egui::Context) -> Vec<ViewportCommand> {
        ctx.viewport(|vp| vp.commands.clone())
    }

    fn state() -> AppState {
        let mut state = AppState::new(
            Config::default(),
            Glossary::default(),
            KeyState::Missing,
            None,
        );
        state.source = "hello".into();
        state.translation = Some(ice_providers::request::TranslationResult::cached(
            "你好",
            ice_core::language::EngineKind::Machine,
        ));
        state
    }

    /// Q-013（用户 2026-10-27）：简洁窗同样带原生顶部栏与最小化/关闭按钮，
    /// 形态差别只剩尺寸与下限；装饰仍要排在 `InnerSize` 之前（原生属性，分帧设会闪）。
    #[test]
    fn switching_to_compact_queues_a_small_titled_window() {
        let cfg = UiConfig::default();
        let commands = commands_for_mode(&cfg, UiMode::Compact);
        assert!(commands.contains(&ViewportCommand::Decorations(true)));
        assert!(
            commands.contains(&ViewportCommand::Resizable(true)),
            "简洁窗要能鼠标缩放（用户 2026-10-22 决定），P4 的 Resizable(false) 是缺陷 W-2.1 的一半"
        );
        assert!(commands.contains(&ViewportCommand::MinInnerSize(MIN_COMPACT_SIZE)));
        assert!(commands.contains(&ViewportCommand::InnerSize(Vec2::new(460.0, 160.0))));
        assert!(commands.contains(&ViewportCommand::OuterPosition(Pos2::new(1400.0, 320.0))));
        // 装饰与几何同帧下发，且排在尺寸之前
        let inner_size = commands
            .iter()
            .position(|c| matches!(c, ViewportCommand::InnerSize(_)))
            .unwrap();
        assert!(
            commands
                .iter()
                .position(|c| *c == ViewportCommand::Decorations(true))
                .unwrap()
                < inner_size
        );
        // 约束也要在尺寸之前改：反过来 460×130 会被全量留下的 640×420 静默夹掉（W-2.1 的根因）
        assert!(
            commands
                .iter()
                .position(|c| *c == ViewportCommand::MinInnerSize(MIN_COMPACT_SIZE))
                .unwrap()
                < inner_size,
            "MinInnerSize 必须排在 InnerSize 之前"
        );
        assert!(
            commands
                .iter()
                .position(|c| *c == ViewportCommand::Resizable(true))
                .unwrap()
                < inner_size,
            "Resizable 也必须排在 InnerSize 之前，否则不可缩放状态下请求会被忽略"
        );
    }

    /// 两个形态各带自己的下限：切到简洁时不得把 640×420 带过去，切回全量时不得把 320×160 留下。
    /// 这条锁的是"约束跟着形态走"，写错就是 W-2.1 复发（窗口缩不下去 / 全量窗能缩到看不见控件）。
    #[test]
    fn each_mode_carries_its_own_minimum_and_never_inherits_the_other() {
        let cfg = UiConfig::default();
        let compact = commands_for_mode(&cfg, UiMode::Compact);
        assert!(compact.contains(&ViewportCommand::MinInnerSize(MIN_COMPACT_SIZE)));
        assert!(
            !compact.contains(&ViewportCommand::MinInnerSize(MIN_FULL_SIZE)),
            "简洁窗带着 640×420 的下限就永远缩不到 460×160"
        );

        let full = commands_for_mode(&cfg, UiMode::Full);
        assert!(full.contains(&ViewportCommand::MinInnerSize(MIN_FULL_SIZE)));
        assert!(
            !full.contains(&ViewportCommand::MinInnerSize(MIN_COMPACT_SIZE)),
            "全量窗只留 320×160 的下限就能被缩到控件叠在一起"
        );
        assert_eq!(min_size_for(UiMode::Full), MIN_FULL_SIZE);
        assert_eq!(min_size_for(UiMode::Compact), MIN_COMPACT_SIZE);
    }

    #[test]
    fn switching_back_to_full_restores_decorations_and_min_size() {
        let commands = commands_for_mode(&UiConfig::default(), UiMode::Full);
        assert!(commands.contains(&ViewportCommand::Decorations(true)));
        assert!(commands.contains(&ViewportCommand::Resizable(true)));
        assert!(commands.contains(&ViewportCommand::MinInnerSize(MIN_FULL_SIZE)));
        assert!(commands.contains(&ViewportCommand::InnerSize(Vec2::new(1080.0, 720.0))));
    }

    /// REQ-004：切换是瞬时的，一次切换每种几何命令**恰好一条**。
    /// 多发一条 `InnerSize` 就是实现悄悄退回逐帧补间（真机实测会被原生夹成 ~1s 停摆）；
    /// 少发一条 `OuterPosition` 就是从贴边的悬浮条切回全量时窗口留在屏幕边缘。
    #[test]
    fn a_switch_sends_exactly_one_of_each_geometry_command() {
        let cfg = UiConfig::default();
        for mode in [UiMode::Full, UiMode::Compact] {
            let commands = commands_for_mode(&cfg, mode);
            let count =
                |pred: fn(&ViewportCommand) -> bool| commands.iter().filter(|c| pred(c)).count();
            assert_eq!(
                count(|c| { matches!(c, ViewportCommand::InnerSize(_)) }),
                1,
                "{mode:?} 一次切换发了多条尺寸命令：{commands:?}"
            );
            assert_eq!(
                count(|c| { matches!(c, ViewportCommand::OuterPosition(_)) }),
                1,
                "{mode:?} 的原点命令应当恰好一条：{commands:?}"
            );
            assert_eq!(
                count(|c| { matches!(c, ViewportCommand::Decorations(_)) }),
                1,
                "{mode:?} 的装饰命令应当恰好一条：{commands:?}"
            );

            let rect = rect_for(&cfg, mode);
            assert!(
                commands.contains(&ViewportCommand::OuterPosition(Pos2::new(rect.x, rect.y))),
                "{mode:?} 要回到自己槽里记着的那个原点"
            );

            // 装饰必须排在尺寸之前：反了会有一帧按错误的可用高度绘制，观感就是"内容闪一下"
            let decorations = commands
                .iter()
                .position(|c| matches!(c, ViewportCommand::Decorations(_)))
                .unwrap();
            let inner_size = commands
                .iter()
                .position(|c| matches!(c, ViewportCommand::InnerSize(_)))
                .unwrap();
            assert!(
                decorations < inner_size,
                "{mode:?} 的装饰命令排在尺寸命令之后：{commands:?}"
            );
        }
    }

    /// 下限只在"太小"时生效：合法尺寸原样通过，大尺寸不夹小（REQ-001"不设上限"）
    #[test]
    fn clamping_a_compact_rect_only_lifts_the_too_small_axes() {
        let normal = WindowRect {
            x: 1400.0,
            y: 320.0,
            w: 460.0,
            h: 160.0,
        };
        assert_eq!(clamp_compact_rect(normal), normal, "合法尺寸不动");

        let bigger = WindowRect {
            w: 900.0,
            h: 400.0,
            ..normal
        };
        assert_eq!(clamp_compact_rect(bigger), bigger, "无上限，不夹小");

        let too_narrow = WindowRect { w: 100.0, ..normal };
        let clamped = clamp_compact_rect(too_narrow);
        assert_eq!((clamped.w, clamped.h), (MIN_COMPACT_W, 160.0));
        assert_eq!((clamped.x, clamped.y), (1400.0, 320.0), "位置不参与夹取");

        let too_short = WindowRect { h: 40.0, ..normal };
        let clamped = clamp_compact_rect(too_short);
        assert_eq!((clamped.w, clamped.h), (460.0, MIN_COMPACT_H));
    }

    /// 配置里的尺寸小于下限时，下发的 `InnerSize` 必须是夹过的那个值：
    /// 直接发 100×40 会被原生约束静默忽略，窗口停在旧尺寸，而写回配置的却是 100×40
    #[test]
    fn an_undersized_configured_rect_is_clamped_before_it_reaches_the_window() {
        let cfg = UiConfig {
            compact_rect: WindowRect {
                x: 10.0,
                y: 20.0,
                w: 100.0,
                h: 40.0,
            },
            ..UiConfig::default()
        };
        let commands = commands_for_mode(&cfg, UiMode::Compact);
        assert!(commands.contains(&ViewportCommand::InnerSize(Vec2::new(
            MIN_COMPACT_W,
            MIN_COMPACT_H
        ))));
        assert!(commands.contains(&ViewportCommand::OuterPosition(Pos2::new(10.0, 20.0))));
    }

    #[test]
    fn toggling_mode_keeps_content_and_marks_config_dirty() {
        let ctx = egui::Context::default();
        let mut state = state();
        let before = (state.source.clone(), state.translation.clone(), state.tab);

        let mode = toggle(&ctx, &mut state, &mut PendingGeometry::default());

        assert_eq!(mode, UiMode::Compact);
        assert_eq!(state.mode(), UiMode::Compact);
        assert_eq!(
            (state.source.clone(), state.translation.clone(), state.tab),
            before
        );
        assert!(state.config_dirty);
        assert!(state.in_flight.is_none(), "切换形态不得发起请求");
        assert!(queued(&ctx).contains(&ViewportCommand::Decorations(true)));
    }

    #[test]
    fn topmost_maps_to_window_level_and_is_persisted() {
        let ctx = egui::Context::default();
        let mut state = state();
        state.toggle_topmost();
        apply_topmost(&ctx, &mut state);
        assert!(queued(&ctx).contains(&ViewportCommand::WindowLevel(WindowLevel::AlwaysOnTop)));
        assert!(state.config.ui.always_on_top);

        state.toggle_topmost();
        apply_topmost(&ctx, &mut state);
        assert!(queued(&ctx).contains(&ViewportCommand::WindowLevel(WindowLevel::Normal)));
        assert!(!state.config.ui.always_on_top);
    }

    #[test]
    fn initial_viewport_follows_the_saved_mode() {
        let compact = UiConfig {
            mode: UiMode::Compact,
            ..UiConfig::default()
        };
        let builder = initial_viewport(&compact);
        assert_eq!(
            builder.decorations,
            Some(true),
            "Q-013：简洁形态启动也带原生顶部栏（最小化与关闭按钮在栏上）"
        );
        assert_eq!(builder.inner_size, Some(Vec2::new(460.0, 160.0)));
        assert!(
            builder.resizable.unwrap_or(false),
            "以简洁形态启动时也要能缩放，否则重启一次就把 REQ-001 丢了"
        );
        assert_eq!(builder.min_inner_size, Some(MIN_COMPACT_SIZE));
        assert_eq!(
            builder.transparent,
            Some(true),
            "失焦半透明靠窗口 alpha 通道，这个属性只能在创建期给（REQ-007）"
        );

        let full = UiConfig {
            mode: UiMode::Full,
            ..UiConfig::default()
        };
        let builder = initial_viewport(&full);
        assert!(builder.decorations.unwrap_or(false));
        assert_eq!(builder.min_inner_size, Some(MIN_FULL_SIZE));
        assert_eq!(
            builder.transparent,
            Some(true),
            "两种形态同一个窗口，surface 只能建一次"
        );
    }

    /// REQ-007 的四组组合：只有"简洁 + 失焦 + 开关开着"才降透明度
    #[test]
    fn only_an_unfocused_compact_window_dims() {
        assert_eq!(dim_alpha(UiMode::Compact, false, true), DIM_ALPHA);
        assert_eq!(dim_alpha(UiMode::Compact, true, true), 1.0, "聚焦时恒定");
        assert_eq!(
            dim_alpha(UiMode::Compact, false, false),
            1.0,
            "开关关了就不降"
        );
        assert_eq!(
            dim_alpha(UiMode::Full, false, true),
            1.0,
            "全量窗从不半透明"
        );
    }

    #[test]
    fn capture_ignores_the_first_frame_where_geometry_is_unknown() {
        assert_eq!(capture_rect(None, None), None);
        assert_eq!(
            capture_rect(
                Some(Rect::from_min_size(
                    Pos2::new(10.0, 20.0),
                    Vec2::new(800.0, 600.0)
                )),
                None
            ),
            None
        );
    }

    #[test]
    fn capture_takes_position_from_outer_and_size_from_inner() {
        let inner = Rect::from_min_size(Pos2::new(10.0, 42.0), Vec2::new(800.0, 600.0));
        let outer = Rect::from_min_size(Pos2::new(10.0, 20.0), Vec2::new(800.0, 622.0));
        let rect = capture_rect(Some(inner), Some(outer)).unwrap();
        assert_eq!(rect.x, 10.0);
        assert_eq!(rect.y, 20.0);
        assert_eq!((rect.w, rect.h), (800.0, 600.0));
    }

    #[test]
    fn hide_cancels_close_and_minimizes() {
        let ctx = egui::Context::default();
        let mut state = state();
        hide(&ctx, &mut state);
        let commands = queued(&ctx);
        assert!(commands.contains(&ViewportCommand::CancelClose));
        assert!(commands.contains(&ViewportCommand::Minimized(true)));
        assert!(!commands.contains(&ViewportCommand::Close));
        assert!(state.hidden);
        // 常驻期间内容原样
        assert_eq!(state.source, "hello");
        reveal(&ctx, &mut state, &mut PendingGeometry::default());
        assert!(!state.hidden);
    }

    /// macOS 上 `viewport().minimized` 运行时永不刷新，恢复判定必须看几何。
    /// 这条测试锁的是"为什么不用 minimized"，写错一次就要在真机上再冻一次窗口。
    #[test]
    fn reveal_is_judged_by_geometry_not_by_a_flag_macos_never_refreshes() {
        assert!(!is_revealed(None), "最小化期间 inner_rect 是 None");
        let rect = Some(Rect::from_min_size(Pos2::ZERO, MIN_FULL_SIZE));
        assert!(is_revealed(rect), "几何回来了就是恢复了");
    }

    fn geometry() -> Option<Rect> {
        Some(Rect::from_min_size(Pos2::ZERO, MIN_FULL_SIZE))
    }

    /// 收起那一帧窗口还带着几何、还带着焦点；静默期内不能判恢复，
    /// 否则 Cmd+W 会被自己"恢复"回来（D-2 第二轮的副作用防线）
    #[test]
    fn the_hide_frame_itself_never_counts_as_a_reveal() {
        let mut guard = RevealGuard::default();
        let t0 = Instant::now();
        guard.on_hide(t0);
        assert!(!guard.check(true, t0, geometry(), Some(true)));
        assert!(!guard.check(true, t0 + REVEAL_GRACE / 2, geometry(), Some(true)));
    }

    /// `inner_rect` 在 macOS 上可能卡住不回来，这时焦点是唯一还活着的证据
    #[test]
    fn focus_reveals_even_when_the_geometry_never_comes_back() {
        let mut guard = RevealGuard::default();
        let t0 = Instant::now();
        guard.on_hide(t0);
        assert!(
            !guard.check(true, t0 + REVEAL_GRACE, None, Some(false)),
            "最小化中不算"
        );
        assert!(guard.check(true, t0 + REVEAL_GRACE * 2, None, Some(true)));
    }

    /// 几何回来了就是恢复了，不需要窗口拿到焦点（用户可能从 Dock 点完就切走了）
    #[test]
    fn geometry_coming_back_reveals_on_its_own() {
        let mut guard = RevealGuard::default();
        let t0 = Instant::now();
        guard.on_hide(t0);
        assert!(!guard.check(true, t0 + REVEAL_GRACE / 2, geometry(), Some(false)));
        assert!(guard.check(true, t0 + REVEAL_GRACE * 2, geometry(), Some(false)));
    }

    /// 窗口可见时判据一律不成立，且把去抖器清干净（下一次隐藏从头计时）
    #[test]
    fn a_visible_window_never_triggers_a_reveal() {
        let mut guard = RevealGuard::default();
        let now = Instant::now();
        guard.on_hide(now);
        assert!(!guard.check(false, now + REVEAL_GRACE * 2, geometry(), Some(true)));
        assert!(!guard.check(false, now + REVEAL_GRACE * 3, geometry(), Some(true)));
    }

    /// 恢复时必须把当前形态的几何命令整套再发一次：不是为了改窗口，
    /// 是为了逼 winit 同步 GL surface 尺寸，否则画面会停在被拉伸的旧帧上
    #[test]
    fn reveal_resends_the_geometry_of_the_current_mode() {
        let ctx = egui::Context::default();
        let mut state = state();
        state.hidden = true;

        reveal(&ctx, &mut state, &mut PendingGeometry::default());

        let commands = queued(&ctx);
        for expected in commands_for_mode(&state.config.ui, state.config.ui.mode) {
            assert!(commands.contains(&expected), "少发了 {expected:?}");
        }
        assert!(
            commands.contains(&ViewportCommand::Minimized(false)),
            "要显式解除最小化，AppKit 那边可能还认为窗口是收起来的"
        );
        assert!(
            commands.contains(&ViewportCommand::WindowLevel(window_level(
                state.config.ui.always_on_top
            ))),
            "置顶状态也要一起恢复"
        );
        assert!(!state.hidden);
    }

    #[test]
    fn quit_sets_the_flag_and_really_closes() {
        let ctx = egui::Context::default();
        let mut state = state();
        quit(&ctx, &mut state, &mut PendingGeometry::default());
        assert!(state.quitting);
        assert!(queued(&ctx).contains(&ViewportCommand::Close));
    }

    /// 1440×900 的屏，菜单栏 25pt 在上、Dock 70pt 在下（与 `system::screen` 的测试同一块屏）
    fn area() -> Rect {
        Rect::from_min_max(Pos2::new(0.0, 25.0), Pos2::new(1440.0, 830.0))
    }

    /// 简洁窗的默认尺寸（`UiConfig::default().compact_rect`）
    const COMPACT: Vec2 = Vec2::new(460.0, 160.0);

    /// 停靠热区宽度：与 `UiConfig::default().dock_sliver_px` 同值（v0.5 由 6.0 改成 3.0，Q-010）
    const SLIVER: f32 = 3.0;

    fn at(x: f32, y: f32) -> Rect {
        Rect::from_min_size(Pos2::new(x, y), COMPACT)
    }

    /// 接缝不是边（D-8 的后半）：`area` 给的是**整个桌面**的并集，两块屏中间那条缝
    /// 因此不在它的边界上。窗口停在缝附近时两边都远 → 自由浮动；
    /// 而"记了停靠却藏不住、`released` 又永远量不出向内位移"的死状态从源头就不会产生。
    #[test]
    fn a_display_seam_is_not_a_dockable_edge() {
        // 左屏 x∈[-1920,0]、右屏（主屏）x∈[0,1920] 的可见区并集
        let desktop = Rect::from_min_max(Pos2::new(-1920.0, 25.0), Pos2::new(1920.0, 1016.0));
        let at_seam = at(-230.0, 400.0);
        assert!(
            desktop.contains(at_seam.center()),
            "测试前提：窗口跨在接缝上，两块屏都沾到"
        );
        let (kept, edge) = snap(at_seam, desktop, 40.0, SLIVER);
        assert_eq!(edge, DockEdge::None, "距两条外缘各 1690px，不该吸到缝上");
        assert_eq!(kept, at_seam, "自由浮动时位置一个像素都不动");

        let right = at(1920.0 - 30.0 - 460.0, 400.0);
        let (tucked, edge) = snap(right, desktop, 40.0, SLIVER);
        assert_eq!(edge, DockEdge::Right);
        assert_eq!(tucked.min.x, 1917.0, "收起到桌面右外缘，只留 3px");
        assert_eq!(reveal_rect(tucked, desktop, DockEdge::Right).min.x, 1460.0);
    }

    /// TC-3.5-a：距右边缘 30px（阈值 40）→ 停靠右边，`x = area.right − sliver`
    #[test]
    fn snapping_to_the_right_edge_uses_the_specified_formula() {
        let window = at(1440.0 - 30.0 - 460.0, 400.0);
        let (rect, edge) = snap(window, area(), 40.0, SLIVER);
        assert_eq!(edge, DockEdge::Right);
        assert_eq!(rect.min.x, 1440.0 - SLIVER);
        assert_eq!(
            area().max.x - rect.min.x,
            SLIVER,
            "可视区内只露出 3px 热区——光标推到屏幕边必然落进它"
        );
        assert!(
            rect.max.x > area().max.x,
            "窗口本体藏在边外，这是机制不是越界"
        );
        assert_eq!(rect.min.y, 400.0, "交叉轴上本来就在可用区内，不该动");
    }

    /// TC-3.5-b：距左边缘 30px → 停靠左边，`x = area.x − w + sliver`（必然是负坐标）
    #[test]
    fn snapping_to_the_left_edge_mirrors_the_right_one() {
        let (rect, edge) = snap(at(30.0, 400.0), area(), 40.0, SLIVER);
        assert_eq!(edge, DockEdge::Left);
        assert_eq!(rect.min.x, 0.0 - 460.0 + SLIVER);
        assert_eq!(rect.max.x, SLIVER, "可用区里只露出 3px 热区");
        assert!(
            rect.min.x < 0.0,
            "左停靠必然产生负坐标，那正是要藏起来的部分"
        );
    }

    /// TC-3.5-b 的另一半（v0.5 砍掉两条边）：**贴着上/下边也不许吸附**。
    /// 藏进菜单栏或 Dock 之后那 3px 不在用户会去悬停的位置，"鼠标放上去自动出来"在那里不成立。
    #[test]
    fn the_top_and_bottom_edges_never_snap() {
        for window in [at(700.0, 25.0 + 30.0), at(700.0, 830.0 - 30.0 - COMPACT.y)] {
            let (rect, edge) = snap(window, area(), 40.0, SLIVER);
            assert_eq!(edge, DockEdge::None, "上/下边够近也不吸：{window:?}");
            assert_eq!(rect, window, "位置一个像素都不动");
        }
        // 左右都够远：自由浮动
        let window = at(500.0, 400.0);
        let (rect, edge) = snap(window, area(), 40.0, SLIVER);
        assert_eq!(edge, DockEdge::None);
        assert_eq!(rect, window);
    }

    /// TC-3.5-d：已停靠右边、又向内拖 200px → `released` 判出解除停靠
    #[test]
    fn dragging_a_docked_window_inward_releases_it() {
        let (docked, edge) = snap(at(1440.0 - 30.0 - 460.0, 400.0), area(), 40.0, SLIVER);
        assert_eq!(edge, DockEdge::Right);
        let anchor = docked.min;
        let dragged = at(anchor.x - 200.0, 400.0);
        assert!(
            released(edge, anchor, dragged.min, 40.0),
            "向内 200px ≥ 阈值，必须判拖离"
        );
        assert!(
            !released(edge, anchor, at(anchor.x - 20.0, 400.0).min, 40.0),
            "20px 还没过阈值，仍算停靠"
        );
        assert!(
            !released(edge, anchor, anchor, 40.0),
            "原地松手（位移 0）不算拖离"
        );
    }

    /// 左右两条边都判得出拖离，方向相反；沿边滑动不算
    #[test]
    fn both_dock_edges_release_toward_the_screen_interior() {
        // (停靠边, 锚点, 向内拖 200px 后的原点)
        let cases = [
            (
                DockEdge::Right,
                Pos2::new(600.0, 400.0),
                Pos2::new(400.0, 400.0),
            ),
            (
                DockEdge::Left,
                Pos2::new(600.0, 400.0),
                Pos2::new(800.0, 400.0),
            ),
        ];
        for (edge, anchor, inward) in cases {
            assert!(
                released(edge, anchor, inward, 40.0),
                "{edge:?} 沿停靠轴向内 200px 应判拖离"
            );
            let outward = anchor + (anchor - inward);
            assert!(
                !released(edge, anchor, outward, 40.0),
                "{edge:?} 往边外推不是拖离"
            );
            // 沿交叉轴拖 200px 是"贴着边滑动"，停靠态要留着
            let along = Pos2::new(anchor.x, anchor.y + 200.0);
            assert!(
                !released(edge, anchor, along, 40.0),
                "{edge:?} 的交叉轴位移不该解除停靠"
            );
        }
        // 已下线的两条边（只为反序列化留在枚举里）一律判不出拖离
        for edge in [DockEdge::None, DockEdge::Top, DockEdge::Bottom] {
            assert!(!released(edge, Pos2::ZERO, Pos2::new(900.0, 900.0), 40.0));
        }
    }

    /// 为什么必须有 `released`：藏出去之后停靠那条边的几何距离恒为大负数、且总是两边最小，
    /// 只跑 `snap` 会把"向内拖 200px"重新吸回原边。谁想把它"简化"回单一几何判据，先过这条。
    #[test]
    fn geometry_alone_would_snap_a_dragged_away_window_back() {
        let (docked, _) = snap(at(1440.0 - 30.0 - 460.0, 400.0), area(), 40.0, SLIVER);
        let dragged = at(docked.min.x - 200.0, 400.0);
        let (rect, edge) = snap(dragged, area(), 40.0, SLIVER);
        assert_eq!(edge, DockEdge::Right, "几何判据仍然判成停靠");
        assert_eq!(rect.min.x, 1440.0 - SLIVER, "并且把窗口吸回边外");
    }

    /// TC-3.5-e：窗口比屏幕可用区还宽 → 停靠轴的起点夹到 `area.min`（不是
    /// `area.max − size` 那个负数，照抄会把窗口甩到屏幕另一侧之外），交叉轴全量夹回可视区。
    /// 这是"停靠轴也要夹"的唯一情形；其余情况一律保留热区（见 `snap` 的文档）。
    #[test]
    fn a_window_wider_than_the_work_area_gets_clamped_to_the_area_start() {
        let wide = Rect::from_min_size(Pos2::new(1400.0, 400.0), Vec2::new(1600.0, 130.0));
        let (rect, edge) = snap(wide, area(), 40.0, SLIVER);
        assert_eq!(edge, DockEdge::Right);
        assert_eq!(
            rect.min.x, 0.0,
            "夹到可用区起点，不是 area.right − 1600 + 3 那个负数"
        );
        assert!(
            area().max.x - rect.min.x >= SLIVER,
            "夹完仍要留得住那 3px 热区"
        );

        // 窗口比可用区还高：交叉轴（y）整条夹回，起点同样是 area.min
        let tall = Rect::from_min_size(Pos2::new(1410.0, 800.0), Vec2::new(460.0, 900.0));
        let (rect, edge) = snap(tall, area(), 40.0, SLIVER);
        assert_eq!(edge, DockEdge::Right);
        assert_eq!(rect.min.y, 25.0);
        assert_eq!(rect.min.x, 1440.0 - SLIVER, "交叉轴夹取不许动停靠轴");
    }

    /// 交叉轴永远夹在可用区内：贴着右边停靠时 y 不许留在屏幕外。
    #[test]
    fn docking_clamps_the_cross_axis_into_the_work_area() {
        let x = 1440.0 - SLIVER;
        let rect = dock(at(x, 2000.0), area(), DockEdge::Right, SLIVER);
        assert_eq!(rect.min.y, 830.0 - COMPACT.y, "越下要夹回可用区底边");
        assert_eq!(rect.min.x, x, "停靠轴本身不动");

        let rect = dock(at(x, -500.0), area(), DockEdge::Right, SLIVER);
        assert_eq!(rect.min.y, 25.0, "越上要夹回可用区顶边");

        let rect = dock(at(-457.0, 2000.0), area(), DockEdge::Left, SLIVER);
        assert_eq!(rect.min.x, -457.0, "左停靠的负坐标不受交叉轴影响");
        assert_eq!(rect.min.y, 830.0 - COMPACT.y);
    }

    /// TC-3.5-f：唤出位就是"齐边整窗可见"，右/左各一条公式；唤出后热区不再只露 3px
    #[test]
    fn revealing_slides_the_whole_window_back_into_the_area() {
        let tucked = dock(at(0.0, 400.0), area(), DockEdge::Right, SLIVER);
        let shown = reveal_rect(tucked, area(), DockEdge::Right);
        assert_eq!(shown.min.x, 1440.0 - 460.0, "右唤出：x = area.right − w");
        assert_eq!(shown.max.x, 1440.0, "整条窗口都在可用区里");
        assert_eq!(shown.min.y, tucked.min.y, "唤出只动停靠轴");

        let tucked = dock(at(0.0, 400.0), area(), DockEdge::Left, SLIVER);
        let shown = reveal_rect(tucked, area(), DockEdge::Left);
        assert_eq!(shown.min.x, 0.0, "左唤出：x = area.min");
        assert_eq!(shown.width(), tucked.width(), "唤出不改尺寸");

        // 唤出位同样夹交叉轴（窗口停在可用区下方的极端情形）
        let tucked = dock(at(0.0, 2000.0), area(), DockEdge::Right, SLIVER);
        let shown = reveal_rect(tucked, area(), DockEdge::Right);
        assert_eq!(shown.min.y, 830.0 - COMPACT.y);

        // 非停靠边是空操作
        let window = at(500.0, 400.0);
        assert_eq!(reveal_rect(window, area(), DockEdge::None), window);
    }

    /// 唤出↔收回是一对可逆的落位：从停靠位唤出、再按同一条边停靠回去，几何必须回到原样。
    /// 这条锁的是"收回用的是唤出后的矩形"这一实现事实——否则每悬停一次窗口就往外多挪 3px。
    #[test]
    fn retracting_after_a_reveal_returns_to_the_same_tucked_rect() {
        let tucked = dock(at(700.0, 400.0), area(), DockEdge::Right, SLIVER);
        let shown = reveal_rect(tucked, area(), DockEdge::Right);
        assert_eq!(dock(shown, area(), DockEdge::Right, SLIVER), tucked);

        let tucked = dock(at(700.0, 400.0), area(), DockEdge::Left, SLIVER);
        let shown = reveal_rect(tucked, area(), DockEdge::Left);
        assert_eq!(dock(shown, area(), DockEdge::Left, SLIVER), tucked);
    }

    /// 创口带（闸门的解除区）：热区 + 向内 `sliver`，坐标是窗口内的 egui 逻辑点
    #[test]
    fn the_sliver_band_is_the_hot_zone_plus_one_sliver_inward() {
        let window = at(0.0, 0.0);
        for (x, expected) in [
            (0.0, true),
            (4.0, true),
            (6.0, true),
            (6.1, false),
            (200.0, false),
        ] {
            assert_eq!(
                dock_band_contains(window, DockEdge::Right, Pos2::new(x, 60.0), SLIVER),
                expected,
                "右停靠：窗口内 x = {x} 该{}",
                if expected { "算" } else { "不算" }
            );
        }
        // 左停靠：带在窗口的右端（屏幕左边那条）
        assert!(dock_band_contains(
            window,
            DockEdge::Left,
            Pos2::new(460.0 - 4.0, 60.0),
            SLIVER
        ));
        assert!(!dock_band_contains(
            window,
            DockEdge::Left,
            Pos2::new(200.0, 60.0),
            SLIVER
        ));
        assert!(!dock_band_contains(
            window,
            DockEdge::None,
            Pos2::ZERO,
            SLIVER
        ));
    }

    /// TC-3.5-h：停靠态只在简洁形态成立，且不论 `always_on_top` 一律置顶（Q-012）
    #[test]
    fn a_docked_strip_is_always_floating() {
        assert!(is_docked(DockEdge::Right, UiMode::Compact));
        assert!(is_docked(DockEdge::Left, UiMode::Compact));
        // 全量窗带着标题栏，藏到边外既唤不出来也拖不动
        assert!(!is_docked(DockEdge::Right, UiMode::Full));
        // 上/下已下线；自由浮动不算停靠
        assert!(!is_docked(DockEdge::Top, UiMode::Compact));
        assert!(!is_docked(DockEdge::Bottom, UiMode::Compact));
        assert!(!is_docked(DockEdge::None, UiMode::Compact));

        for topmost in [true, false] {
            assert_eq!(level_for_dock(true, topmost), WindowLevel::AlwaysOnTop);
            assert_eq!(level_for_dock(false, topmost), window_level(topmost));
        }
    }

    /// 已经停在目标位置的窗口再吸一次，结果不变：吸附必须幂等，
    /// 否则 `DragSettler` 返回后写完盘、下一帧又"变了"，就会无限重算
    #[test]
    fn snapping_an_already_docked_window_changes_nothing() {
        let (once, edge) = snap(at(1440.0 - 30.0 - 460.0, 400.0), area(), 40.0, SLIVER);
        assert_eq!(edge, DockEdge::Right);
        let (twice, edge) = snap(once, area(), 40.0, SLIVER);
        assert_eq!(edge, DockEdge::Right);
        assert_eq!(twice, once);
    }

    /// `dock(None)` 是空操作：自由浮动的窗口不该被"停靠"到任何地方
    #[test]
    fn docking_to_no_edge_leaves_the_window_alone() {
        let window = at(500.0, 400.0);
        assert_eq!(dock(window, area(), DockEdge::None, SLIVER), window);
        // 已下线的两条边同样不动：停靠比不停靠更糟的时候，宁可什么都不做
        assert_eq!(dock(window, area(), DockEdge::Top, SLIVER), window);
        assert_eq!(dock(window, area(), DockEdge::Bottom, SLIVER), window);
    }

    #[test]
    fn fitting_into_the_work_area_handles_all_five_cases() {
        let area = area();
        let inside = at(100.0, 100.0);
        assert_eq!(fit_into_area(inside, area), inside, "本来就在里面，不动");

        let too_far_left = at(-500.0, 100.0);
        assert_eq!(fit_into_area(too_far_left, area).min.x, 0.0);

        let too_far_down = at(100.0, 900.0);
        assert_eq!(fit_into_area(too_far_down, area).min.y, 830.0 - COMPACT.y);

        let too_wide = Rect::from_min_size(Pos2::new(-200.0, 100.0), Vec2::new(2000.0, 130.0));
        let fitted = fit_into_area(too_wide, area);
        assert_eq!(fitted.width(), 1440.0, "先夹到可用区宽度");
        assert_eq!(fitted.min.x, 0.0);

        let too_tall = Rect::from_min_size(Pos2::new(100.0, -200.0), Vec2::new(460.0, 1000.0));
        let fitted = fit_into_area(too_tall, area);
        assert_eq!(fitted.height(), 805.0);
        assert_eq!(fitted.min.y, 25.0);
    }

    /// 未 arm 时一律不返回：没有拖动就没有"松手"
    #[test]
    fn an_unarmed_settler_never_fires() {
        let mut settler = DragSettler::default();
        let t0 = Instant::now();
        for step in 0..10 {
            assert_eq!(settler.feed(Some(at(100.0 + step as f32, 100.0)), t0), None);
        }
    }

    /// 移动过程中不返回；几何读不到的那几帧也不能拿旧计时判静止
    #[test]
    fn a_moving_window_is_not_settled() {
        let mut settler = DragSettler::default();
        let t0 = Instant::now();
        settler.arm(t0);
        for step in 1..=8 {
            let rect = at(100.0 + step as f32 * 20.0, 100.0);
            assert_eq!(settler.feed(Some(rect), t0 + DRAG_SETTLE * step), None);
        }
        let resting = at(260.0, 100.0);
        assert_eq!(settler.feed(None, t0 + DRAG_SETTLE * 9), None);
        assert_eq!(settler.feed(Some(resting), t0 + DRAG_SETTLE * 9), None);
        assert_eq!(
            settler.feed(Some(resting), t0 + DRAG_SETTLE * 9 + DRAG_SETTLE / 2),
            None,
            "读不到几何之后要重新等满一个静止窗口，否则会在拖动中途吸附"
        );
        assert_eq!(
            settler.feed(Some(resting), t0 + DRAG_SETTLE * 10),
            Some(resting)
        );
    }

    /// 静止不足 `DRAG_SETTLE` 不返回；到了就返回最终矩形，且只返回一次
    #[test]
    fn a_still_window_settles_exactly_once() {
        let mut settler = DragSettler::default();
        let t0 = Instant::now();
        let final_rect = at(700.0, 300.0);
        settler.arm(t0);
        assert_eq!(settler.feed(Some(final_rect), t0), None);
        assert_eq!(
            settler.feed(Some(final_rect), t0 + DRAG_SETTLE / 2),
            None,
            "才静止 75ms，还在阈值内"
        );
        assert_eq!(
            settler.feed(Some(final_rect), t0 + DRAG_SETTLE),
            Some(final_rect)
        );
        assert_eq!(
            settler.feed(Some(final_rect), t0 + DRAG_SETTLE * 4),
            None,
            "已经返回过一次，不能每 150ms 再返回一次（会打穿落盘节流）"
        );
    }

    /// 静止到一半又动了：重新计时，不许提前返回
    #[test]
    fn moving_again_restarts_the_settle_timer() {
        let mut settler = DragSettler::default();
        let t0 = Instant::now();
        settler.arm(t0);
        assert_eq!(settler.feed(Some(at(100.0, 100.0)), t0), None);
        assert_eq!(
            settler.feed(Some(at(100.0, 100.0)), t0 + DRAG_SETTLE * 3 / 4),
            None
        );
        assert_eq!(
            settler.feed(Some(at(140.0, 100.0)), t0 + DRAG_SETTLE),
            None,
            "又动了，计时归零"
        );
        assert_eq!(
            settler.feed(Some(at(140.0, 100.0)), t0 + DRAG_SETTLE * 2),
            Some(at(140.0, 100.0))
        );
    }

    /// 亚像素抖动不算移动：`outer_rect` 每帧回读，缩放换算会带来零点几像素的误差，
    /// 不容差的话窗口永远"还在动"，吸附一次都不会发生
    #[test]
    fn sub_pixel_jitter_counts_as_still() {
        let mut settler = DragSettler::default();
        let t0 = Instant::now();
        settler.arm(t0);
        assert_eq!(settler.feed(Some(at(100.0, 100.0)), t0), None);
        assert_eq!(
            settler.feed(Some(at(100.3, 100.2)), t0 + DRAG_SETTLE),
            Some(at(100.3, 100.2))
        );
    }

    /// D-6：切换后待生效的那几帧，尺寸轴不许被实时几何冲掉；命中目标才放行。
    ///
    /// 真机走查的故障形状：从 1080×720 切到简洁，`compact_rect` 被写成 642×422
    /// 且再没人发缩小请求，简洁窗就此永久是大窗。
    #[test]
    fn a_pending_switch_blocks_the_size_axis_until_it_lands() {
        let t0 = Instant::now();
        let target = Vec2::new(460.0, 160.0);
        let mut pending = PendingGeometry::default();
        assert!(
            pending.size_axis_writable(Vec2::new(1080.0, 720.0), t0),
            "没有待生效尺寸时照常写回，手动缩放不受影响"
        );

        pending.set(target, t0);
        assert_eq!(
            pending.stage.as_ref().map(|stage| stage.target),
            Some(target)
        );
        assert!(
            !pending.size_axis_writable(Vec2::new(642.0, 422.0), t0 + Duration::from_millis(16)),
            "切换后第一帧量到的还是旧尺寸，写回就是 D-6"
        );
        assert!(
            !pending.size_axis_writable(Vec2::new(642.0, 422.0), t0 + MODE_GEOMETRY_GRACE / 2),
            "一直没落定时宽限期还要挡着"
        );
        assert!(
            pending.size_axis_writable(target, t0 + MODE_GEOMETRY_GRACE / 2),
            "落定即放行"
        );
        assert_eq!(
            pending.stage.as_ref().map(|stage| stage.target),
            None,
            "放行后护栏自己解除"
        );
    }

    /// D-10 第一段到期：那一帧**仍不放行**，改为把一次重发交给调用方。
    ///
    /// v0.6 在这里直接放行，于是别的形态的尺寸被当成"用户选的尺寸"写进本槽——这就是
    /// 盘上两槽一字不差、且再也自愈不了的起点。
    #[test]
    fn the_first_expiry_hands_back_exactly_one_retry() {
        let t0 = Instant::now();
        let target = Vec2::new(460.0, 160.0);
        let foreign = Vec2::new(984.0, 632.0);
        let mut pending = PendingGeometry::default();
        pending.set(target, t0);

        assert!(
            !pending.size_axis_writable(foreign, t0 + MODE_GEOMETRY_GRACE),
            "到期那一帧不写回"
        );
        assert_eq!(pending.take_retry(), Some(target), "把重发目标交给调用方");
        assert_eq!(pending.take_retry(), None, "一轮只给一次");
        assert!(pending.is_pending(), "第二轮宽限期重新计时");
        assert!(
            !pending.size_axis_writable(foreign, t0 + MODE_GEOMETRY_GRACE * 3 / 2),
            "第二轮里继续拦着"
        );
    }

    /// D-10 第二段到期：不采纳实测尺寸，把它当成"窗口没动"的静默参照。
    ///
    /// 三条断言对应 spec 里那三段：② 之后连续几帧同样的实测值一律不写盘；③ 与参照不同的
    /// 值（用户自己在拉窗口）要被放行，手动缩放的所有权不能被护栏吞掉。
    #[test]
    fn a_never_landing_switch_refuses_to_adopt_the_measured_size() {
        let t0 = Instant::now();
        let target = Vec2::new(460.0, 160.0);
        let foreign = Vec2::new(984.0, 632.0);
        let mut pending = PendingGeometry::default();
        pending.set(target, t0);

        assert!(!pending.size_axis_writable(foreign, t0 + MODE_GEOMETRY_GRACE));
        let _ = pending.take_retry();
        assert!(
            !pending.size_axis_writable(foreign, t0 + MODE_GEOMETRY_GRACE * 2),
            "第二段到期也不写"
        );
        assert!(!pending.is_pending(), "护栏收工，不再每帧拦位置以外的东西");
        assert_eq!(pending.baseline, Some(foreign), "实测值成为静默参照");

        for frame in 1..5u32 {
            assert!(
                !pending.size_axis_writable(foreign, t0 + MODE_GEOMETRY_GRACE * 2 + ms(frame * 16)),
                "窗口停在哪儿都不算用户的意思"
            );
        }

        // ③ 用户手动缩放：与参照不同即交还尺寸轴所有权，且只交还这一次
        assert!(
            pending.size_axis_writable(Vec2::new(700.0, 500.0), t0 + MODE_GEOMETRY_GRACE * 3),
            "手动缩放不被吞"
        );
        assert_eq!(pending.baseline, None, "参照用完即废");
        assert!(
            pending.size_axis_writable(Vec2::new(700.0, 500.0), t0 + MODE_GEOMETRY_GRACE * 4),
            "交还之后与切换前一致"
        );

        // 下一次切换无条件作废残留参照
        pending.baseline = Some(foreign);
        pending.set(Vec2::new(1080.0, 720.0), t0 + MODE_GEOMETRY_GRACE * 5);
        assert_eq!(pending.baseline, None, "新切换不继承上一段的参照");
    }

    /// 参照判据用的是同一把容差尺：亚像素抖动不该被当成"用户拉了窗口"
    ///
    /// 位移按欧氏长度判，所以抖动只加在一条轴上——两轴各抖一个容差，合起来是 √2 倍容差，
    /// 那就不是"容差内"了。
    #[test]
    fn a_jitter_of_the_same_size_stays_silent() {
        let t0 = Instant::now();
        let mut pending = PendingGeometry {
            baseline: Some(Vec2::new(984.0, 632.0)),
            ..Default::default()
        };
        assert!(
            !pending.size_axis_writable(Vec2::new(984.0 + PENDING_SIZE_TOLERANCE * 0.9, 632.0), t0),
            "容差内不算手动缩放"
        );
        assert!(
            pending.size_axis_writable(Vec2::new(984.0, 632.0 + PENDING_SIZE_TOLERANCE * 2.0), t0),
            "超出容差就放行"
        );
    }

    fn ms(frame: u32) -> Duration {
        Duration::from_millis(u64::from(frame))
    }

    /// 护栏生效时写回的是"新位置 + 旧尺寸"：位置轴必须照常写，
    /// 用户可能在切换的同一瞬间就把窗口拖走了
    #[test]
    fn a_suppressed_capture_still_records_the_position() {
        let measured = WindowRect {
            x: 900.0,
            y: 120.0,
            w: 642.0,
            h: 422.0,
        };
        let previous = WindowRect {
            x: 1400.0,
            y: 320.0,
            w: 460.0,
            h: 160.0,
        };
        let kept = keep_pending_size(measured, previous);
        assert_eq!((kept.x, kept.y), (900.0, 120.0), "位置照写");
        assert_eq!((kept.w, kept.h), (460.0, 160.0), "尺寸沿用刚下发的目标");
    }

    /// `apply` 必须把即将下发的那个 `InnerSize` 记进护栏——记了别的值等于没记
    #[test]
    fn apply_records_exactly_the_size_it_queued() {
        let ctx = egui::Context::default();
        let mut state = state();
        let mut pending = PendingGeometry::default();

        apply(&ctx, &mut state, UiMode::Compact, &mut pending);

        assert_eq!(
            pending.stage.as_ref().map(|stage| stage.target),
            Some(Vec2::new(460.0, 160.0))
        );
        assert!(queued(&ctx).contains(&ViewportCommand::InnerSize(Vec2::new(460.0, 160.0))));
    }

    /// 带着停靠态启动时窗口层级就要是 `AlwaysOnTop`，不能等第一帧的 `watch_dock` 再补：
    /// `always_on_top=false` 时那 3px 会压在别的 app 下面，鼠标推不到（Q-012、缺陷 D-7）
    #[test]
    fn a_docked_startup_begins_floating() {
        let docked = UiConfig {
            mode: UiMode::Compact,
            docked_edge: DockEdge::Right,
            always_on_top: false,
            ..UiConfig::default()
        };
        assert_eq!(
            initial_viewport(&docked).window_level,
            Some(WindowLevel::AlwaysOnTop)
        );

        let undocked = UiConfig {
            mode: UiMode::Compact,
            always_on_top: false,
            ..UiConfig::default()
        };
        assert_eq!(
            initial_viewport(&undocked).window_level,
            Some(WindowLevel::Normal)
        );
    }
}
