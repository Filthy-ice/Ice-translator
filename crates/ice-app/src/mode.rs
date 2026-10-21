//! 窗口形态（FR-3.1 双模式、FR-3.4 置顶、FR-3.6 位置记忆、FR-5.2 关窗常驻）。
//!
//! P4 只做即时切换与内容保留；形变动画、拖拽、贴边吸附属 P5（`CHG-004`）。
//! 命令的组装是纯函数，可无头断言（沿用 P1 的 `queued(ctx)` 手法）。

use std::time::{Duration, Instant};

use eframe::egui::{self, Pos2, Rect, Vec2, ViewportBuilder, ViewportCommand, WindowLevel};
use ice_core::config::{UiConfig, UiMode, WindowRect};

use crate::state::AppState;

/// 全量模式的最小内尺寸（`03-ui-design.md` §1）
pub const MIN_FULL_SIZE: Vec2 = Vec2::new(640.0, 420.0);

/// 隐藏期间的重绘心跳。
///
/// 最小化之后 eframe 不再跑 egui pass，只按 `MISSED_REDRAW_TIMEOUT`（250ms）调一次
/// `App::logic`（`update_logic_only`），`App::ui` 根本不执行——所以心跳必须由 `logic`
/// 里的 `request_repaint_after` 续上，否则没人再叫事件循环，从 Dock 叫回来时窗口就停在
/// 旧的那一帧上（缺陷 D-2）。
/// 250ms 与 eframe 自己的节流同档：再快也会被 `INVISIBLE_WINDOW_REPAINT_INTERVAL` 压回来。
pub const HIDDEN_HEARTBEAT: Duration = Duration::from_millis(250);

/// 某个形态对应的窗口命令。顺序有意义：先解除装饰再改尺寸，否则尺寸会被标题栏吃掉一次。
pub fn commands_for_mode(cfg: &UiConfig, mode: UiMode) -> Vec<ViewportCommand> {
    let rect = rect_for(cfg, mode);
    match mode {
        UiMode::Full => vec![
            ViewportCommand::Decorations(true),
            ViewportCommand::Resizable(true),
            ViewportCommand::MinInnerSize(MIN_FULL_SIZE),
            ViewportCommand::InnerSize(Vec2::new(rect.w, rect.h)),
        ],
        UiMode::Compact => vec![
            ViewportCommand::Decorations(false),
            ViewportCommand::Resizable(false),
            ViewportCommand::InnerSize(Vec2::new(rect.w, rect.h)),
            ViewportCommand::OuterPosition(Pos2::new(rect.x, rect.y)),
        ],
    }
}

pub const fn rect_for(cfg: &UiConfig, mode: UiMode) -> WindowRect {
    match mode {
        UiMode::Full => cfg.full_rect,
        UiMode::Compact => cfg.compact_rect,
    }
}

/// 启动形态直接由保存的模式决定，避免"先开大窗再缩成简洁窗"的闪烁（FR-3.1）
pub fn initial_viewport(cfg: &UiConfig) -> ViewportBuilder {
    let rect = rect_for(cfg, cfg.mode);
    let mut builder = ViewportBuilder::default()
        .with_title("Ice Translator")
        .with_inner_size([rect.w, rect.h])
        .with_position([rect.x, rect.y])
        .with_decorations(cfg.mode == UiMode::Full)
        .with_resizable(cfg.mode == UiMode::Full)
        .with_window_level(window_level(cfg.always_on_top));
    if cfg.mode == UiMode::Full {
        builder = builder.with_min_inner_size([MIN_FULL_SIZE.x, MIN_FULL_SIZE.y]);
    }
    builder
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
        assert!(commands.contains(&ViewportCommand::Resizable(false)));
        assert!(commands.contains(&ViewportCommand::InnerSize(Vec2::new(460.0, 96.0))));
        assert!(commands.contains(&ViewportCommand::OuterPosition(Pos2::new(1400.0, 320.0))));
        // 装饰要在尺寸之前解除
        assert!(
            commands
                .iter()
                .position(|c| *c == ViewportCommand::Decorations(false))
                .unwrap()
                < commands
                    .iter()
                    .position(|c| matches!(c, ViewportCommand::InnerSize(_)))
                    .unwrap()
        );
    }

    #[test]
    fn switching_back_to_full_restores_decorations_and_min_size() {
        let commands = commands_for_mode(&UiConfig::default(), UiMode::Full);
        assert!(commands.contains(&ViewportCommand::Decorations(true)));
        assert!(commands.contains(&ViewportCommand::Resizable(true)));
        assert!(commands.contains(&ViewportCommand::MinInnerSize(MIN_FULL_SIZE)));
        assert!(commands.contains(&ViewportCommand::InnerSize(Vec2::new(1080.0, 720.0))));
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
}
