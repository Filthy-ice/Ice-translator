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

/// 简洁模式的最小内尺寸（FR-3.2 改：可缩放，下限 320×96，无上限）
pub const MIN_COMPACT_SIZE: Vec2 = Vec2::new(MIN_COMPACT_W, MIN_COMPACT_H);

/// 隐藏期间的重绘心跳。
///
/// 最小化之后 eframe 不再跑 egui pass，只按 `MISSED_REDRAW_TIMEOUT`（250ms）调一次
/// `App::logic`（`update_logic_only`），`App::ui` 根本不执行——所以心跳必须由 `logic`
/// 里的 `request_repaint_after` 续上，否则没人再叫事件循环，从 Dock 叫回来时窗口就停在
/// 旧的那一帧上（缺陷 D-2）。
/// 250ms 与 eframe 自己的节流同档：再快也会被 `INVISIBLE_WINDOW_REPAINT_INTERVAL` 压回来。
pub const HIDDEN_HEARTBEAT: Duration = Duration::from_millis(250);

/// 某个形态对应的窗口命令。顺序有意义：先解除装饰、再改最小尺寸约束，最后才改尺寸。
///
/// `InnerSize` 走 `window.request_inner_size`，平台认为该尺寸不合法时**直接返回当前尺寸并
/// 忽略请求**（`egui-winit-0.37.0/src/lib.rs:1969-1985`）。P4 的简洁分支从不改 `MinInnerSize`，
/// 于是全量留下的 640×420 会把 460×96 夹掉——命令发了、窗口没变小，这就是走查缺陷 W-2.1。
pub fn commands_for_mode(cfg: &UiConfig, mode: UiMode) -> Vec<ViewportCommand> {
    let rect = rect_for(cfg, mode);
    match mode {
        UiMode::Full => vec![
            ViewportCommand::Decorations(true),
            ViewportCommand::Resizable(true),
            ViewportCommand::MinInnerSize(MIN_FULL_SIZE),
            ViewportCommand::InnerSize(Vec2::new(rect.w, rect.h)),
        ],
        UiMode::Compact => {
            let rect = clamp_compact_rect(rect);
            vec![
                ViewportCommand::Decorations(false),
                // 用户 2026-10-22 决定：简洁窗也要能鼠标拖动缩放（P4 是 `false`）
                ViewportCommand::Resizable(true),
                ViewportCommand::MinInnerSize(MIN_COMPACT_SIZE),
                ViewportCommand::InnerSize(Vec2::new(rect.w, rect.h)),
                ViewportCommand::OuterPosition(Pos2::new(rect.x, rect.y)),
            ]
        }
    }
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

/// 四边吸附（FR-3.5、REQ-003）：松手后窗口离哪条边最近就贴哪条，四边都够远则自由浮动。
///
/// 判定是纯函数：输入窗口外框、屏幕可用区、阈值与热区宽度，输出新矩形与停靠边。
/// `area` 来自 `system::screen::work_area`（去掉菜单栏与 Dock 的那块）。
///
/// 四条公式逐字照抄 `spec.md` REQ-003 与 `docs/03-ui-design.md` §3 的伪代码：
///
/// ```text
/// 右 x = area.right  - w + sliver      左 x = area.x      - w + sliver
/// 下 y = area.bottom - h + sliver      上 y = area.y      - h + sliver
/// ```
///
/// **注意这四条并不对称**：左/上把窗口大部分推出屏幕、只留 `sliver` 宽的可见热区
/// （与"保留 6px 可见热区供拖回"、TC-3.5-d"热区消失"一致）；而右/下按同一公式算出来的是
/// 窗口**超出**可用区 `sliver` 像素（几乎全可见）。基线文档三处都是这么写的，
/// 所以这里按字面实现，不自行改成"四边都只露 6px"——那属需求变更，要重新走批准。
///
/// 夹取只分两种（设计里"吸附后再跑一次 `fit_into_area`"与 TC-3.5-a 的字面期望冲突，
/// 全量夹取会把 sliver 也夹没，四条公式全部失效）：
/// - **交叉轴**（贴左右时夹 y、贴上下时夹 x）全量夹进可用区，不许窗口斜着飘出去；
/// - **停靠轴**只在窗口比可用区还长时夹到起点，此时 `area.max - size` 是负数，
///   照抄会把窗口甩到屏幕外（TC-3.5-e 要的正是"不产生负坐标、夹在可视区内"）。
pub fn snap(window: Rect, area: Rect, threshold: f32, sliver: f32) -> (Rect, DockEdge) {
    let candidates = [
        (DockEdge::Left, window.min.x - area.min.x),
        (DockEdge::Top, window.min.y - area.min.y),
        (DockEdge::Right, area.max.x - window.max.x),
        (DockEdge::Bottom, area.max.y - window.max.y),
    ];
    let (edge, distance) = candidates
        .into_iter()
        .min_by(|a, b| a.1.total_cmp(&b.1))
        .expect("四条边总有最小值");
    // 四边都够远（或距离不可比）：自由浮动，位置一个像素都不动。
    // 用 `partial_cmp` 而不是 `>=`：NaN 与阈值不可比，此时也必须停在原地。
    let Some(Ordering::Less) = distance.partial_cmp(&threshold) else {
        return (window, DockEdge::None);
    };
    (dock(window, area, edge, sliver), edge)
}

/// 把窗口贴到指定边。`snap` 与"点击停靠指示换边"共用这一条路径（REQ-003）。
pub fn dock(window: Rect, area: Rect, edge: DockEdge, sliver: f32) -> Rect {
    let size = window.size();
    let (x, y) = match edge {
        DockEdge::Right => (
            docked_axis(area.max.x - size.x + sliver, size.x, area.min.x, area.max.x),
            cross_axis(window.min.y, size.y, area.min.y, area.max.y),
        ),
        DockEdge::Left => (
            docked_axis(area.min.x - size.x + sliver, size.x, area.min.x, area.max.x),
            cross_axis(window.min.y, size.y, area.min.y, area.max.y),
        ),
        DockEdge::Bottom => (
            cross_axis(window.min.x, size.x, area.min.x, area.max.x),
            docked_axis(area.max.y - size.y + sliver, size.y, area.min.y, area.max.y),
        ),
        DockEdge::Top => (
            cross_axis(window.min.x, size.x, area.min.x, area.max.x),
            docked_axis(area.min.y - size.y + sliver, size.y, area.min.y, area.max.y),
        ),
        DockEdge::None => return window,
    };
    Rect::from_min_size(Pos2::new(x, y), size)
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

/// 停靠指示点一下换下一条边：右 → 下 → 左 → 上 → 右，自由浮动先进右（REQ-003）
pub const fn cycle_edge(edge: DockEdge) -> DockEdge {
    match edge {
        DockEdge::Right => DockEdge::Bottom,
        DockEdge::Bottom => DockEdge::Left,
        DockEdge::Left => DockEdge::Top,
        DockEdge::Top => DockEdge::Right,
        DockEdge::None => DockEdge::Right,
    }
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
        .with_decorations(cfg.mode == UiMode::Full)
        .with_resizable(true)
        .with_min_inner_size([min.x, min.y])
        .with_window_level(window_level(cfg.always_on_top))
}

pub const fn window_level(always_on_top: bool) -> WindowLevel {
    if always_on_top {
        WindowLevel::AlwaysOnTop
    } else {
        WindowLevel::Normal
    }
}

/// 切到指定形态。源文、译文、语言对、页签一律保留，且不重发请求（FR-3.3）。
pub fn apply(ctx: &egui::Context, state: &mut AppState, mode: UiMode) {
    // 先把当前几何记回配置，否则切回来时会跳到上一次的旧位置
    capture(ctx, state);
    state.config.ui.mode = mode;
    state.mark_config_dirty();
    for command in commands_for_mode(&state.config.ui, mode) {
        ctx.send_viewport_cmd(command);
    }
    apply_topmost(ctx, state);
    ctx.request_repaint();
}

pub fn toggle(ctx: &egui::Context, state: &mut AppState) -> UiMode {
    let mode = state.toggle_mode();
    apply(ctx, state, mode);
    mode
}

pub fn apply_topmost(ctx: &egui::Context, state: &mut AppState) {
    ctx.send_viewport_cmd(ViewportCommand::WindowLevel(window_level(
        state.config.ui.always_on_top,
    )));
}

/// 记住窗口位置与大小（FR-3.6）。屏幕拔掉后旧坐标会落到可用区外，启动时回屏幕中心。
pub fn capture(ctx: &egui::Context, state: &mut AppState) {
    let (inner, outer) = ctx.input(|i| {
        let viewport = i.viewport();
        (viewport.inner_rect, viewport.outer_rect)
    });
    let Some(rect) = capture_rect(inner, outer) else {
        return;
    };
    let mode = state.config.ui.mode;
    let slot = if mode == UiMode::Full {
        &mut state.config.ui.full_rect
    } else {
        &mut state.config.ui.compact_rect
    };
    if *slot != rect {
        *slot = rect;
        state.mark_config_dirty();
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
pub fn reveal(ctx: &egui::Context, state: &mut AppState) {
    state.hidden = false;
    tracing::debug!(mode = ?state.config.ui.mode, "窗口恢复可见，重发几何");
    // 与 hide 的 `Minimized(true)` 对上：AppKit 那边若还认为自己是最小化的，
    // 这一步才真的把它叫回来；已经可见时是空操作。
    ctx.send_viewport_cmd(ViewportCommand::Minimized(false));
    for command in commands_for_mode(&state.config.ui, state.config.ui.mode) {
        ctx.send_viewport_cmd(command);
    }
    apply_topmost(ctx, state);
    ctx.request_repaint();
}

/// Cmd+Q：先落盘再让窗口真的关掉。`on_exit` 里还会再存一次，这里保证顺序。
pub fn quit(ctx: &egui::Context, state: &mut AppState) {
    capture(ctx, state);
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

    #[test]
    fn switching_to_compact_queues_a_borderless_small_window() {
        let cfg = UiConfig::default();
        let commands = commands_for_mode(&cfg, UiMode::Compact);
        assert!(commands.contains(&ViewportCommand::Decorations(false)));
        assert!(
            commands.contains(&ViewportCommand::Resizable(true)),
            "简洁窗要能鼠标缩放（用户 2026-10-22 决定），P4 的 Resizable(false) 是缺陷 W-2.1 的一半"
        );
        assert!(commands.contains(&ViewportCommand::MinInnerSize(MIN_COMPACT_SIZE)));
        assert!(commands.contains(&ViewportCommand::InnerSize(Vec2::new(460.0, 96.0))));
        assert!(commands.contains(&ViewportCommand::OuterPosition(Pos2::new(1400.0, 320.0))));
        // 装饰要在尺寸之前解除
        let inner_size = commands
            .iter()
            .position(|c| matches!(c, ViewportCommand::InnerSize(_)))
            .unwrap();
        assert!(
            commands
                .iter()
                .position(|c| *c == ViewportCommand::Decorations(false))
                .unwrap()
                < inner_size
        );
        // 约束也要在尺寸之前改：反过来 460×96 会被全量留下的 640×420 静默夹掉（W-2.1 的根因）
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

    /// 两个形态各带自己的下限：切到简洁时不得把 640×420 带过去，切回全量时不得把 320×96 留下。
    /// 这条锁的是"约束跟着形态走"，写错就是 W-2.1 复发（窗口缩不下去 / 全量窗能缩到看不见控件）。
    #[test]
    fn each_mode_carries_its_own_minimum_and_never_inherits_the_other() {
        let cfg = UiConfig::default();
        let compact = commands_for_mode(&cfg, UiMode::Compact);
        assert!(compact.contains(&ViewportCommand::MinInnerSize(MIN_COMPACT_SIZE)));
        assert!(
            !compact.contains(&ViewportCommand::MinInnerSize(MIN_FULL_SIZE)),
            "简洁窗带着 640×420 的下限就永远缩不到 460×96"
        );

        let full = commands_for_mode(&cfg, UiMode::Full);
        assert!(full.contains(&ViewportCommand::MinInnerSize(MIN_FULL_SIZE)));
        assert!(
            !full.contains(&ViewportCommand::MinInnerSize(MIN_COMPACT_SIZE)),
            "全量窗只留 320×96 的下限就能被缩到控件叠在一起"
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

    /// 下限只在"太小"时生效：合法尺寸原样通过，大尺寸不夹小（REQ-001"不设上限"）
    #[test]
    fn clamping_a_compact_rect_only_lifts_the_too_small_axes() {
        let normal = WindowRect {
            x: 1400.0,
            y: 320.0,
            w: 460.0,
            h: 96.0,
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
        assert_eq!((clamped.w, clamped.h), (MIN_COMPACT_W, 96.0));
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

        let mode = toggle(&ctx, &mut state);

        assert_eq!(mode, UiMode::Compact);
        assert_eq!(state.mode(), UiMode::Compact);
        assert_eq!(
            (state.source.clone(), state.translation.clone(), state.tab),
            before
        );
        assert!(state.config_dirty);
        assert!(state.in_flight.is_none(), "切换形态不得发起请求");
        assert!(queued(&ctx).contains(&ViewportCommand::Decorations(false)));
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
        assert!(!builder.decorations.unwrap_or(true));
        assert_eq!(builder.inner_size, Some(Vec2::new(460.0, 96.0)));
        assert!(
            builder.resizable.unwrap_or(false),
            "以简洁形态启动时也要能缩放，否则重启一次就把 REQ-001 丢了"
        );
        assert_eq!(builder.min_inner_size, Some(MIN_COMPACT_SIZE));

        let full = UiConfig {
            mode: UiMode::Full,
            ..UiConfig::default()
        };
        let builder = initial_viewport(&full);
        assert!(builder.decorations.unwrap_or(false));
        assert_eq!(builder.min_inner_size, Some(MIN_FULL_SIZE));
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
        reveal(&ctx, &mut state);
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

        reveal(&ctx, &mut state);

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
        quit(&ctx, &mut state);
        assert!(state.quitting);
        assert!(queued(&ctx).contains(&ViewportCommand::Close));
    }

    /// 1440×900 的屏，菜单栏 25pt 在上、Dock 70pt 在下（与 `system::screen` 的测试同一块屏）
    fn area() -> Rect {
        Rect::from_min_max(Pos2::new(0.0, 25.0), Pos2::new(1440.0, 830.0))
    }

    /// 简洁窗的默认尺寸（`UiConfig::default().compact_rect`）
    const COMPACT: Vec2 = Vec2::new(460.0, 96.0);

    fn at(x: f32, y: f32) -> Rect {
        Rect::from_min_size(Pos2::new(x, y), COMPACT)
    }

    /// TC-3.5-a：距右边缘 30px（阈值 40）→ 停靠右边，`x = area.right − w + sliver`
    #[test]
    fn snapping_to_the_right_edge_uses_the_specified_formula() {
        let window = at(1440.0 - 30.0 - 460.0, 400.0);
        let (rect, edge) = snap(window, area(), 40.0, 6.0);
        assert_eq!(edge, DockEdge::Right);
        assert_eq!(rect.min.x, 1440.0 - 460.0 + 6.0);
        assert_eq!(rect.min.y, 400.0, "交叉轴上本来就在可用区内，不该动");
    }

    /// TC-3.5-b：左/上/下各 30px，分别停靠对应边。
    /// 左与上按基线公式把窗口大部分推出屏幕，只留 6px 可见热区（拖回来的抓手）。
    #[test]
    fn the_other_three_edges_snap_the_same_way() {
        let (rect, edge) = snap(at(30.0, 400.0), area(), 40.0, 6.0);
        assert_eq!(edge, DockEdge::Left);
        assert_eq!(rect.min.x, 0.0 - 460.0 + 6.0);
        assert_eq!(rect.max.x, 6.0, "可用区里只露出 6px 热区");

        let (rect, edge) = snap(at(700.0, 25.0 + 30.0), area(), 40.0, 6.0);
        assert_eq!(edge, DockEdge::Top);
        assert_eq!(rect.min.y, 25.0 - 96.0 + 6.0);
        assert_eq!(rect.max.y, 31.0, "菜单栏之下只露出 6px 热区");

        let (rect, edge) = snap(at(700.0, 830.0 - 30.0 - 96.0), area(), 40.0, 6.0);
        assert_eq!(edge, DockEdge::Bottom);
        assert_eq!(rect.min.y, 830.0 - 96.0 + 6.0);
    }

    /// TC-3.5-c：四边都够远 → 自由浮动，位置一个像素都不动
    #[test]
    fn a_window_far_from_every_edge_stays_where_it_is() {
        let window = at(500.0, 400.0);
        let (rect, edge) = snap(window, area(), 40.0, 6.0);
        assert_eq!(edge, DockEdge::None);
        assert_eq!(rect, window);
    }

    /// TC-3.5-d：已停靠右边、又被拖离 200px → 恢复自由浮动，不自动吸回原边
    #[test]
    fn dragging_a_docked_window_away_releases_it() {
        let (docked, edge) = snap(at(1440.0 - 30.0 - 460.0, 400.0), area(), 40.0, 6.0);
        assert_eq!(edge, DockEdge::Right);
        let dragged = Rect::from_min_size(Pos2::new(docked.min.x - 200.0, 400.0), COMPACT);
        let (rect, edge) = snap(dragged, area(), 40.0, 6.0);
        assert_eq!(edge, DockEdge::None);
        assert_eq!(rect, dragged, "松手位置原样保留");
    }

    /// TC-3.5-e：窗口比屏幕可用区还高 → 不产生负坐标，夹在可视区内。
    /// 这是"停靠轴也要夹"的唯一情形；其余情况一律保留热区（见 `snap` 的文档）。
    #[test]
    fn a_window_taller_than_the_work_area_never_gets_negative_coordinates() {
        let tall = Rect::from_min_size(Pos2::new(700.0, 800.0), Vec2::new(460.0, 900.0));
        let (rect, edge) = snap(tall, area(), 40.0, 6.0);
        assert_eq!(edge, DockEdge::Bottom);
        assert_eq!(
            rect.min.y, 25.0,
            "夹到可用区起点，不是 area.bottom − 900 + 6 那个负数"
        );
        assert!(rect.min.y >= area().min.y);

        let wide = Rect::from_min_size(Pos2::new(1400.0, 400.0), Vec2::new(1600.0, 96.0));
        let (rect, edge) = snap(wide, area(), 40.0, 6.0);
        assert_eq!(edge, DockEdge::Right);
        assert_eq!(rect.min.x, 0.0);
    }

    /// 交叉轴永远夹在可用区内：贴着右边停靠时 y 不许留在屏幕外。
    ///
    /// 走 `dock` 而不是 `snap`：`snap` 取的是四条边里**带符号**距离最小的那条，
    /// 窗口远在屏幕下方时 `d_bottom` 是个大负数，胜出的一定是底边，测不到右边的交叉轴。
    #[test]
    fn docking_clamps_the_cross_axis_into_the_work_area() {
        let x = 1440.0 - 460.0 + 6.0;
        let rect = dock(at(x, 2000.0), area(), DockEdge::Right, 6.0);
        assert_eq!(rect.min.y, 830.0 - 96.0, "越下要夹回可用区底边");
        assert_eq!(rect.min.x, x, "停靠轴本身不动");

        let rect = dock(at(x, -500.0), area(), DockEdge::Right, 6.0);
        assert_eq!(rect.min.y, 25.0, "越上要夹回可用区顶边");

        let rect = dock(at(2000.0, 400.0), area(), DockEdge::Top, 6.0);
        assert_eq!(rect.min.x, 1440.0 - 460.0, "贴上下边时夹的是 x");
    }

    /// 已经停在目标位置的窗口再吸一次，结果不变：吸附必须幂等，
    /// 否则 `DragSettler` 返回后写完盘、下一帧又"变了"，就会无限重算
    #[test]
    fn snapping_an_already_docked_window_changes_nothing() {
        let (once, edge) = snap(at(1440.0 - 30.0 - 460.0, 400.0), area(), 40.0, 6.0);
        assert_eq!(edge, DockEdge::Right);
        let (twice, edge) = snap(once, area(), 40.0, 6.0);
        assert_eq!(edge, DockEdge::Right);
        assert_eq!(twice, once);
    }

    #[test]
    fn cycling_the_dock_edge_walks_right_bottom_left_top() {
        assert_eq!(cycle_edge(DockEdge::None), DockEdge::Right);
        assert_eq!(cycle_edge(DockEdge::Right), DockEdge::Bottom);
        assert_eq!(cycle_edge(DockEdge::Bottom), DockEdge::Left);
        assert_eq!(cycle_edge(DockEdge::Left), DockEdge::Top);
        assert_eq!(cycle_edge(DockEdge::Top), DockEdge::Right);
    }

    /// `dock(None)` 是空操作：自由浮动的窗口不该被"停靠"到任何地方
    #[test]
    fn docking_to_no_edge_leaves_the_window_alone() {
        let window = at(500.0, 400.0);
        assert_eq!(dock(window, area(), DockEdge::None, 6.0), window);
    }

    #[test]
    fn fitting_into_the_work_area_handles_all_five_cases() {
        let area = area();
        let inside = at(100.0, 100.0);
        assert_eq!(fit_into_area(inside, area), inside, "本来就在里面，不动");

        let too_far_left = at(-500.0, 100.0);
        assert_eq!(fit_into_area(too_far_left, area).min.x, 0.0);

        let too_far_down = at(100.0, 900.0);
        assert_eq!(fit_into_area(too_far_down, area).min.y, 830.0 - 96.0);

        let too_wide = Rect::from_min_size(Pos2::new(-200.0, 100.0), Vec2::new(2000.0, 96.0));
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
}
