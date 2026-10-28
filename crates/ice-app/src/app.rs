//! 帧循环：`eframe::App` 的实现，把快捷键、页签绘制、任务回收与 `ViewportCommand` 收在一处。
//!
//! 一帧的顺序是固定的，改动前先想清楚依赖：
//! **回收结果 → 判定快捷键 → 绘制 → 执行副作用 → 落盘**。
//! 快捷键必须在绘制之前判定，因为它读的是"上一帧源文框有没有焦点"（`state.focus_in_source`），
//! 而这一帧的焦点要等 `TextEdit` 画完才知道。
//!
//! 窗口收进后台之后 eframe 只调 `logic`，`ui` 一帧都不跑（缺陷 D-2 的第二轮教训），
//! 所以"隐藏期间要一直活着"的逻辑只能放 `logic`，见 `IceApp::watch_visibility`。

use std::path::PathBuf;
use std::time::Instant;

use eframe::egui::{self, Pos2, Rect, ViewportCommand};
use ice_core::config::{Config, DockEdge, UiConfig, UiMode, WindowRect};
use ice_core::history::HistoryStore;
use ice_providers::dispatcher::Translator;
use ice_providers::{AiEngine, MachineTranslateEngine};
use tokio::runtime::Runtime;
use tokio::sync::mpsc;

use crate::jobs::{self, JobRunner, OutcomeReceiver};
use crate::mode;
use crate::state::{self, AppState, KeyState, Tab, Tone};
use crate::system::alpha;
use crate::system::clipboard::{self, CopyKind};
use crate::system::hotkeys::{self, Action};
use crate::system::screen;
use crate::system::secrets;
use crate::system::single_instance::InstanceLock;
use crate::system::titlebar;
use crate::theme;
use crate::views::{self, Effects};

/// 后台钥匙串探测的结果回收端（容量 1：一次启动只探一次）
pub type KeyProbeReceiver = mpsc::Receiver<Result<Option<String>, KeyState>>;

/// 几何跳变多少像素才算"窗口动了"（`observe_native_move`）。`SETTLE_TOLERANCE` 那 0.5px
/// 是给静止判定用的亚像素抖动留的，认拖动要更宽一点：Retina 缩放下换一次屏幕就是 1px。
const NATIVE_MOVE_PX: f32 = 2.0;

/// 连续几帧没动才算"窗口停着"，也用于判定我们自己下发的那次移动已经结束。
/// 取 3 而不是 1：启动初期尺寸/位置会逐帧收敛，中间任何一帧的静止都不能当成"拖动开始了"。
const STILL_BEFORE_DRAG: u32 = 3;

/// 在后台线程读钥匙串，读完后请求重绘。
///
/// 这一步必须离开建窗路径、也离开 UI 线程：macOS 对不由本程序写入的条目会弹授权框，
/// `get_password()` 会一直阻塞到用户点完。放在建窗之前的话，窗口根本出不来，
/// 用户只看到一个来路不明的弹窗，表现为"启动卡死"。
pub fn spawn_key_probe(runtime: &Runtime, ctx: &egui::Context) -> KeyProbeReceiver {
    spawn_probe_with(runtime, ctx, secrets::read)
}

/// 读取函数做成参数，测试就能不碰真钥匙串地验证"结果确实送到了帧循环"。
fn spawn_probe_with(
    runtime: &Runtime,
    ctx: &egui::Context,
    read: impl FnOnce() -> Result<Option<String>, KeyState> + Send + 'static,
) -> KeyProbeReceiver {
    let (tx, rx) = mpsc::channel(1);
    let ctx = ctx.clone();
    runtime.spawn_blocking(move || {
        // 必须用 blocking_send：`send` 是 async 的，在这里只会造出一个没人 await 的 future
        let _ = tx.blocking_send(read());
        // 探测期间界面可能一直没有输入事件，不主动请求就不会重绘，"未知"会挂在状态条上
        ctx.request_repaint();
    });
    rx
}

pub struct IceApp {
    pub state: AppState,
    runtime: Runtime,
    runner: JobRunner,
    outcomes: OutcomeReceiver,
    history: HistoryStore,
    config_path: PathBuf,
    /// 单实例锁。`on_exit` 里显式释放，进程被杀时靠 `Drop`（REQ-009）
    lock: Option<InstanceLock>,
    key_probe: KeyProbeReceiver,
    last_save: Instant,
    /// 恢复判定的去抖（`mode::REVEAL_GRACE`）
    reveal_guard: mode::RevealGuard,
    /// 拖动/缩放静止判定（`mode::DRAG_SETTLE`）：松手后跑一次吸附与落盘
    drag_settler: mode::DragSettler,
    /// 上一帧读到的窗口外框，用来认出"几何自己动了"（原生标题栏那条拖动路径，见
    /// `observe_native_move`）
    observed_outer: Option<Rect>,
    /// 连续几帧没变的帧数。我们自己下发的那次移动要到这儿才算结束（`commanded` 的兜底）。
    still_frames: u32,
    /// 最近一次由本应用下发的窗口原点（吸附、落位、唤出、收回都走 `place_compact`）。
    /// 在它落地之前读到的跳变都是自己的，不能当成用户拖动。
    commanded: Option<Pos2>,
    /// 停靠那一刻下发的窗口原点，拖离判定的锚点（REQ-003 第三条）。
    ///
    /// 不能取 `config.ui.compact_rect`：`mode::capture` 每帧按实时几何改写它，
    /// 拖动途中锚点就被冲掉、位移恒为 0，于是谁都拖不离。
    dock_anchor: Option<Pos2>,
    /// 停靠态当前是"唤出"（true）还是"收起"（false）。运行时状态，一律不落盘（REQ-003）。
    dock_shown: bool,
    /// 指针离开窗口的时刻，`DOCK_AUTO_HIDE` 的计时起点
    dock_left_at: Option<Instant>,
    /// 闸门：true 表示"指针还欠一次离开创口带"，不许唤出。
    ///
    /// 没有它，收回后指针正停在那 3px 上就会立刻再弹出来，来回抖；用户把窗口拖到边上松手时
    /// 指针也正压在边线上，不置闸门的话停靠看起来"没生效"。
    dock_latched: bool,
    /// 待生效的窗口内尺寸（缺陷 D-6 的护栏，见 `mode::PendingGeometry`）
    pending_geometry: mode::PendingGeometry,
    /// 剪贴板即翻的判定器（FR-3.8）：记住上次内容与"本应用自己写过什么"
    clipboard_watch: clipboard::Watcher,
    /// 启动后的离屏校验跑过了没有（REQ-009）。一次就够：`screen::work_area` 要走 NSScreen。
    geometry_rescued: bool,
    /// 上一帧量到的窗口内尺寸与装饰条厚度（缺陷 D-14 的取证，见 `trace_geometry`）
    geometry_seen: Option<(egui::Vec2, f32)>,
    /// 上一帧下到原生层的整窗不透明度（FR-3.7，见 `apply_native_alpha`）。
    /// `None` = 还没下过或上一次没走通。
    native_alpha: Option<f32>,
    /// 上一帧标题栏绿键的改道结果（见 `apply_zoom_hook`）。`None` = 还没下过。
    zoom_outcome: Option<titlebar::Outcome>,
}

impl IceApp {
    pub fn new(
        mut state: AppState,
        runtime: Runtime,
        translator: Translator,
        history: HistoryStore,
        config_path: PathBuf,
        lock: Option<InstanceLock>,
        key_probe: KeyProbeReceiver,
    ) -> Self {
        let (mut runner, outcomes) = JobRunner::new(translator);
        // config.toml 里的术语必须在第一次派发前就进引擎，否则要等用户改一次术语才生效（FR-4.5）
        runner.set_glossary(state.glossary.clone());
        // 启动时的一次性提示（配置损坏降级、旧术语表导入）用提示条给，不进日志以外就没了
        if let Some(notice) = state.startup_notice.take() {
            state.toast(Tone::Warn, notice);
        }
        state.reload_history(&history);
        // 带着停靠态启动：锚点**不给值**，让 `watch_dock` 的第一帧去重新落位（`land_dock`）。
        // 照抄盘上的原点会出问题：上一次可能就是在"唤出位"被强杀的，那样窗口会以停靠之名
        // 整条可见地挂着，而用户以为它收起来了（REQ-003 的启动落位）。
        let docked = mode::is_docked(state.config.ui.docked_edge, state.config.ui.mode);
        Self {
            state,
            runtime,
            runner,
            outcomes,
            history,
            config_path,
            lock,
            key_probe,
            last_save: Instant::now(),
            reveal_guard: mode::RevealGuard::default(),
            drag_settler: mode::DragSettler::default(),
            observed_outer: None,
            still_frames: 0,
            commanded: None,
            dock_anchor: None,
            dock_shown: false,
            dock_left_at: None,
            // 收起态起步就把闸门关上：指针可能正停在那 3px 上，不等它离开一次就唤出等于闪一下
            dock_latched: docked,
            pending_geometry: mode::PendingGeometry::default(),
            clipboard_watch: clipboard::Watcher::default(),
            geometry_rescued: false,
            geometry_seen: None,
            native_alpha: None,
            zoom_outcome: None,
        }
    }

    /// AI 是否可用：配置齐备且钥匙串里确实有 Key。
    /// 每帧算一次比去锁 `Translator` 问一遍便宜，也不会因为重建时序出现假阴性。
    pub fn ai_available(&self) -> bool {
        self.state.config.ai.is_ready() && self.state.key.configured()
    }

    /// 状态条与工具条用的可用性判定，与 `Translator::availability()` 语义一致
    fn run(&mut self, action: Action, ctx: &egui::Context, effects: &mut Effects) {
        match action {
            Action::Translate => effects.request(self.state.submit()),
            Action::ToggleMode => effects.toggle_mode = true,
            Action::CopyTranslation => {
                views::full::copy(&mut self.state, effects, CopyKind::Translation)
            }
            Action::ToggleTopmost => self.toggle_topmost(ctx),
            Action::HideWindow => self.hide(ctx),
            Action::Quit => self.quit(ctx),
            // 换行由 `TextEdit` 自己插入（它的 `return_key` 就是 Shift+Enter），应用不参与
            Action::Newline => {}
            // Esc 只在简洁窗清空输入行；全量窗 Esc 不绑定任何动作（与 P4 一致）
            Action::ClearInput => {
                if self.state.mode() == UiMode::Compact {
                    self.state.clear_input();
                }
            }
        }
    }

    /// 切换置顶（K-09）。快捷键与简洁窗动作行的按钮共用这一条路径。
    fn toggle_topmost(&mut self, ctx: &egui::Context) {
        let on = self.state.toggle_topmost();
        mode::apply_topmost(ctx, &mut self.state);
        self.state
            .toast(Tone::Info, if on { "已置顶" } else { "已取消置顶" });
    }

    /// Cmd+Q：先落盘再关窗，`on_exit` 里还会再存一次（幂等）
    fn quit(&mut self, ctx: &egui::Context) {
        mode::capture(ctx, &mut self.state, &mut self.pending_geometry);
        self.flush_config(true);
        mode::quit(ctx, &mut self.state, &mut self.pending_geometry);
    }

    /// 收进后台（Cmd+W / 关闭按钮）。恢复判定与心跳见 `watch_visibility`。
    fn hide(&mut self, ctx: &egui::Context) {
        mode::hide(ctx, &mut self.state);
        self.reveal_guard.on_hide(Instant::now());
    }

    /// 把手按下：窗口拖拽交给原生（REQ-002）。
    ///
    /// `StartDrag` 在窗口没有焦点时会被 egui-winit 直接丢掉
    /// （`egui-winit-0.37.0/src/lib.rs:1961-1968` 的 `window.has_focus()` 判定），
    /// 所以未聚焦时先补一条 `Focus`。简洁窗是常驻置顶的小条，用户常常正在别的应用里
    /// 干活、顺手就来抓它——那一次恰好是没有焦点的，少这条命令表现为"拖不动"。
    fn start_drag(&mut self, ctx: &egui::Context) {
        if ctx.input(|i| i.viewport().focused) != Some(true) {
            ctx.send_viewport_cmd(ViewportCommand::Focus);
        }
        ctx.send_viewport_cmd(ViewportCommand::StartDrag);
        self.drag_settler.arm(Instant::now());
    }

    /// 拖动/缩放静止之后跑一次：吸附（REQ-003）+ 立即落盘（REQ-009）。
    ///
    /// 原生拖拽期间 egui 收不到 pointer 事件（`StartDrag` 把控制权交给了 AppKit，
    /// macOS 那边进的是嵌套事件循环），"松手"这个时刻应用侧无从得知，
    /// 只能靠 `DragSettler` 的静止判定。
    /// `area` 由调用方注入（同 `watch_dock` 的理由：真机要读屏幕，测试给一块固定的桌面）
    fn settle_drag(&mut self, ctx: &egui::Context, area_for: impl Fn() -> Option<Rect>) {
        let outer = ctx.input(|i| i.viewport().outer_rect);
        self.observe_native_move(outer);
        match self.drag_settler.feed(outer, Instant::now()) {
            Some(settled) => {
                let area = (self.state.mode() == UiMode::Compact)
                    .then(area_for)
                    .flatten();
                self.apply_snap(ctx, settled, area);
            }
            // 还在等静止：续一帧重绘。原生拖拽结束后事件循环未必还有别的理由再画一帧，
            // 不续的话"静止满 150ms"永远等不到，吸附一次都不会发生。
            None if self.drag_settler.is_armed() => {
                ctx.request_repaint_after(mode::DRAG_SETTLE);
            }
            None => {}
        }
    }

    /// 认出"用户抓的是原生标题栏"这条拖动路径，补上静止判定的启动（Q-013）。
    ///
    /// 抓原生标题栏不会经过这里的把手，也就没人 `arm`：AppKit 自己把窗口搬走，应用侧
    /// 只看到几何在动。而吸附、停靠、拖离解除全挂在 `DragSettler` 上，不 arm 就一次都不跑。
    ///
    /// **误判的代价比漏判大**：`show_dock` 把窗口唤出到贴着边缘的位置后，如果那次移动被
    /// 当成用户拖动，150ms 后的 `apply_snap` 会看到"窗口离右边不到阈值"再 `dock` 一次，
    /// 刚弹出来又缩回去。所以本应用自己下发的移动要排除：`place_compact` 记下目标原点，
    /// 几何到达目标（或连续几帧不再变化）之前不 arm。`was_still` 那条是把启动初期
    /// 一帧一帧收敛的几何变化也挡掉——那时窗口还没停过。
    fn observe_native_move(&mut self, outer: Option<Rect>) {
        let Some(rect) = outer else {
            self.observed_outer = None;
            self.still_frames = 0;
            return;
        };
        let prev = self.observed_outer.replace(rect);
        let moved = prev.is_some_and(|p| (p.min - rect.min).length() > NATIVE_MOVE_PX);
        let was_still = self.still_frames >= STILL_BEFORE_DRAG;
        self.still_frames = if moved { 0 } else { self.still_frames + 1 };

        if let Some(target) = self.commanded {
            let arrived = (target - rect.min).length() <= NATIVE_MOVE_PX;
            if arrived || self.still_frames >= STILL_BEFORE_DRAG {
                self.commanded = None;
            }
            return;
        }
        if moved
            && was_still
            && !self.drag_settler.is_armed()
            && !self.pending_geometry.is_pending()
            && self.state.mode() == UiMode::Compact
            && !self.state.hidden
        {
            self.drag_settler.arm(Instant::now());
        }
    }

    /// `area` 为 `None` 表示这次不吸附（全量窗有系统标题栏，或拿不到屏幕可用区）：只落盘。
    ///
    /// 可用区由调用方注入而不是在这里读屏幕，是为了让吸附决策可以无头断言——
    /// `screen::desktop` 在真机上会读到**这台机器**的屏幕，测试没法固定期望值。
    fn apply_snap(&mut self, ctx: &egui::Context, settled: Rect, area: Option<Rect>) {
        let Some(area) = area else {
            self.flush_config(true);
            return;
        };
        // 松手时窗口整个落在可用区之外（外接屏拔掉、休眠唤醒后的旧坐标）：夹回来，
        // 别让用户去 Dock 里找窗口。REQ-009 的启动路径与插拔恢复在 T-010 接上，
        // 这里先覆盖"拖完就丢"这一种。
        if !area.intersect(settled).is_positive() {
            let fitted = mode::fit_into_area(settled, area);
            self.place_compact(ctx, fitted.min, self.state.config.ui.docked_edge);
            return;
        }
        // 已停靠且从热区向内拖过阈值 → 就地解除停靠，停在松手处（REQ-003 第三条、TC-3.5-d）。
        // 必须排在 `snap` 之前：藏出去的窗口在停靠那条边上的几何距离恒为约 -(size − sliver)
        // 的大负数、且总是四边最小，先跑 `snap` 会立刻把它吸回原边，拖离永远判不出来。
        let docked = self.state.config.ui.docked_edge;
        if let Some(anchor) = self.dock_anchor.filter(|_| docked != DockEdge::None) {
            if mode::released(
                docked,
                anchor,
                settled.min,
                self.state.config.ui.snap_threshold_px,
            ) {
                self.place_compact(ctx, settled.min, DockEdge::None);
                ctx.request_repaint();
                return;
            }
        }
        let (snapped, edge) = mode::snap(
            settled,
            area,
            self.state.config.ui.snap_threshold_px,
            self.state.config.ui.dock_sliver_px,
        );
        // 吸附结果与当前几何一致时一个命令都不发：否则程序自己挪的那一次又会被下一轮
        // 判定当成"移动"，150ms 后再吸附一次，无限循环且每轮写盘（REQ-009 的节流被打穿）
        let already_there =
            (snapped.min - settled.min).length() <= 0.5 && edge == self.state.config.ui.docked_edge;
        if already_there {
            self.flush_config(true);
            return;
        }
        self.place_compact(ctx, snapped.min, edge);
        // 闸门在吸附落位处置起（REQ-003）：用户把窗口推到边上松手时指针就压在屏幕最外沿，
        // 不置闸门的话同一帧的 `watch_dock` 会立刻判定"进入热区"再弹回去，停靠看起来没生效。
        // 解除靠指针离开创口带，那是 `watch_dock` 的事。
        if mode::is_docked(edge, self.state.mode()) {
            self.dock_shown = false;
            self.dock_latched = true;
        }
        ctx.request_repaint();
    }

    /// 把简洁窗挪到一个新位置：发命令 + 写回配置 + 立即落盘（REQ-009）。
    ///
    /// 落盘走 `force`：拖动结束后没有任何东西再请求重绘，脏标记挂着不动，强杀就丢。
    ///
    /// 停靠态的锚点在这里一起维护：`Some(edge)` 就把锚点搬到刚下发的那个原点，
    /// 解除停靠则清空——这是"拖离"唯一可信的参照（`compact_rect` 每帧被 `capture` 改写）。
    ///
    /// 层级也在这里对齐：停靠边的唯一改动入口就是它，吸附落位要升到置顶层级
    /// （AppKit 的 `.floating`，Q-012），解除停靠要降回用户的置顶开关，两处都不用等到下一帧。
    fn place_compact(&mut self, ctx: &egui::Context, position: Pos2, edge: DockEdge) {
        tracing::debug!(x = position.x, y = position.y, ?edge, "下发窗口原点");
        ctx.send_viewport_cmd(ViewportCommand::OuterPosition(position));
        // 记下这次是"自己要求的移动"，`observe_native_move` 在它落地前不认成用户拖动
        self.commanded = Some(position);
        self.state.config.ui.compact_rect.x = position.x;
        self.state.config.ui.compact_rect.y = position.y;
        self.state.config.ui.docked_edge = edge;
        self.dock_anchor = (edge != DockEdge::None).then_some(position);
        mode::apply_topmost(ctx, &mut self.state);
        self.state.mark_config_dirty();
        self.flush_config(true);
    }

    /// 简洁窗的"设置"：瞬时切回全量并直接落到设置页（REQ-005）。
    ///
    /// 不能复用 `Effects::toggle_mode`：那是"切换"，在全量窗里点设置会反被切成简洁窗。
    fn open_settings(&mut self, ctx: &egui::Context) {
        self.drag_settler.disarm();
        if self.state.mode() != UiMode::Full {
            mode::apply(
                ctx,
                &mut self.state,
                UiMode::Full,
                &mut self.pending_geometry,
            );
        }
        self.state.tab = Tab::Settings;
        ctx.request_repaint();
    }

    /// 恢复默认窗口尺寸（REQ-011）。设置页那颗按钮是全应用唯一"程序替用户改几何"的入口，
    /// 存在的理由就是 D-10：尺寸槽可能被某个版本的切换写坏，而用户自己缩不动它。
    ///
    /// 不走 `mode::apply`：它开头先 `capture`，会把正要被覆盖的坏尺寸再记回槽里一次。
    /// 这里直接下发配置，然后按 `pending` 的规矩把目标尺寸登记，落定后由 `capture` 自然写回。
    ///
    /// 停靠一并清掉：收位是按旧尺寸算的，尺寸变了还留着旧锚点，下一帧会把窗口顶出屏幕。
    fn reset_window_sizes(&mut self, ctx: &egui::Context) {
        let current_mode = self.state.mode();
        let defaults = UiConfig::default();
        let monitor = screen::work_area(
            ctx,
            // 读不到当前几何时按主屏选：给 `select` 一个落在原点的小矩形，别喂 NaN
            ctx.input(|i| i.viewport().outer_rect)
                .unwrap_or_else(|| Rect::from_min_size(Pos2::ZERO, egui::vec2(1.0, 1.0))),
        )
        .map(|area| WindowRect {
            x: area.min.x,
            y: area.min.y,
            w: area.width(),
            h: area.height(),
        });
        // 拿不到可用区（无头测试、探针失败）时只换尺寸、位置留在原处，别把窗口甩到屏幕外
        let keep = mode::rect_for(&self.state.config.ui, current_mode);
        self.state.config.ui.full_rect = restored_rect(defaults.full_rect, keep, monitor);
        self.state.config.ui.compact_rect = restored_rect(defaults.compact_rect, keep, monitor);
        self.state.config.ui.docked_edge = DockEdge::None;
        self.dock_anchor = None;
        self.state.mark_config_dirty();
        self.flush_config(true);
        for command in mode::commands_for_mode(&self.state.config.ui, current_mode) {
            ctx.send_viewport_cmd(command);
        }
        self.pending_geometry.set(
            mode::target_size(&self.state.config.ui, current_mode),
            Instant::now(),
        );
        mode::apply_topmost(ctx, &mut self.state);
        self.state.toast(Tone::Info, "已恢复默认窗口尺寸");
        ctx.request_repaint();
    }

    /// 每帧的几何回写（FR-3.6）：`capture` 把实时几何记回当前形态的配置槽，
    /// 切换的那组命令**落定**的那一帧强制落盘（REQ-004）。
    ///
    /// 为什么要有"落定"这一步：`InnerSize` 是异步请求，把刚下发的目标值当已生效值写盘
    /// 就是把还没发生的事记进配置（D-6 的原罪）；而 `pending` 被清空恰好意味着真实几何
    /// 已经量到，此时槽里存的正是窗口现在的样子。SIGTERM/强杀不走 `on_exit`（P1 实测），
    /// 所以不能等退出再存。
    ///
    /// v0.7（D-10）加进来的中间一步：第一轮宽限期到期还没落定时重发一次尺寸请求。
    /// 重发后要再等一个宽限期才能判第二轮，而 egui 是"有事才画"的——不续一帧重绘，
    /// 第二轮到期永远不会被 `capture` 量到，护栏就停在"无限期拦着"那一档。
    fn refresh_geometry(&mut self, ctx: &egui::Context) {
        let was_pending = self.pending_geometry.is_pending();
        mode::capture(ctx, &mut self.state, &mut self.pending_geometry);
        if let Some(target) = self.pending_geometry.take_retry() {
            tracing::debug!(w = target.x, h = target.y, "尺寸未落定，重发尺寸与原点请求");
            for command in mode::resend_commands(&self.state.config.ui, self.state.mode(), target) {
                ctx.send_viewport_cmd(command);
            }
            ctx.request_repaint_after(mode::MODE_GEOMETRY_GRACE);
            return;
        }
        if was_pending && !self.pending_geometry.is_pending() {
            self.flush_config(true);
        }
    }

    /// 缺陷 D-14 的取证（用户 2026-10-28："拖动时候变大，停下后又恢复原来的大小"，屏幕中间
    /// 也这样，两种形态都中招）。
    ///
    /// 静态读代码排除了我们自己：`InnerSize` 全应用只有两个发出处——形态切换那组命令
    /// （`commands_for_mode`）与切换未落定时的一次重发（`resend_commands`），拖动落定后的
    /// 吸附只发 `OuterPosition`，而那条路径对全量窗连吸附都不做（`apply_snap` 拿 `area=None`
    /// 直接落盘返回）。所以要看的是现场：**实测内尺寸到底动没动**。
    ///
    /// 两件事分别对应两种"看起来变大"：
    /// - `dw`/`dh` 非零 = 窗口内容区真的被改大了（谁改的？不是我们，就是原生层）。
    /// - `chrome` 变了 = 外框与内框的差（标题栏那条装饰）在跳，整窗因此变大。
    ///
    /// 只记数字，不记路径也不记任何用户内容（NFR-3）。挂在 `debug` 级：默认 `info` 不输出，
    /// 排查时用 `RUST_LOG=ice_translator=debug` 打开。
    fn trace_geometry(&mut self, ctx: &egui::Context) {
        let (inner, outer) = ctx.input(|i| (i.viewport().inner_rect, i.viewport().outer_rect));
        let (Some(inner), Some(outer)) = (inner, outer) else {
            return;
        };
        let size = inner.size();
        let chrome = outer.height() - inner.height();
        let previous = self.geometry_seen.replace((size, chrome));
        let Some((before, before_chrome)) = previous else {
            tracing::debug!(
                mode = ?self.state.config.ui.mode,
                w = size.x,
                h = size.y,
                chrome = chrome,
                x = outer.min.x,
                y = outer.min.y,
                "窗口几何基线"
            );
            return;
        };
        if (before - size).length() <= 0.5 && (before_chrome - chrome).abs() <= 0.5 {
            return;
        }
        tracing::debug!(
            mode = ?self.state.config.ui.mode,
            w = size.x,
            h = size.y,
            dw = size.x - before.x,
            dh = size.y - before.y,
            chrome = chrome,
            dchrome = chrome - before_chrome,
            x = outer.min.x,
            y = outer.min.y,
            "窗口几何变化"
        );
    }

    /// 把整窗不透明度下到位（FR-3.7），返回**原生层是否接住了这一档**。
    ///
    /// 为什么放在帧循环而不是视图里：`views::compact` 只管画，够不着 `eframe::Frame`；
    /// 而"窗口该多透明"是形态 + 焦点 + 配置三件事的判定，帧循环本来就同时拿着这三样
    /// （见 `ui` 里的那次 `mode::dim_alpha`）。
    ///
    /// 返回 false 时缓存写成 `None`，下一帧重试：第一帧可能还拿不到窗口句柄
    /// （eframe 建窗与首个 pass 的先后不由我们决定），一次失败不能永久退回。
    fn apply_native_alpha(&mut self, frame: &eframe::Frame, want: f32) -> bool {
        if !needs_alpha_update(self.native_alpha, want) {
            return true;
        }
        let applied = alpha::set(frame, want);
        self.native_alpha = applied.then_some(want);
        // 值真的变了才记：这条是"标题栏到底有没有跟着淡"的唯一现场证据——无头测试拿不到
        // 窗口句柄，走没走到原生层只能在这里看。失败侧只警告一次（`needs_alpha_update`
        // 对失败不缓存，每帧都会重试，逐次打日志会刷屏，与 `system::screen` 同一手法）。
        if applied {
            tracing::debug!(alpha = want, native = true, "整窗不透明度下发");
        } else if !ALPHA_FALLBACK_WARNED.swap(true, std::sync::atomic::Ordering::Relaxed) {
            tracing::warn!("原生整窗不透明度下不发，失焦只淡窗内底色（标题栏不会跟着淡）");
        }
        applied
    }

    /// 帧首取走标题栏绿键的点击（用户 2026-10-28："悬浮窗更换全量的按钮可以不要，
    /// 将标题栏最大化功能替换为切换全量"）。
    ///
    /// 这里只把"被点了"翻译成 `effects.toggle_mode = true`——与 `Action::ToggleMode`
    /// （`Cmd+Shift+M`）落在同一个字段上，切换本身一个实现，不在原生路径上重做一遍判断。
    fn poll_titlebar_click(&mut self, effects: &mut Effects) {
        if titlebar::take_click() {
            effects.toggle_mode = true;
        }
    }

    /// 把标题栏绿键按形态接到该接的人身上：简洁窗归我们（点它切全量），
    /// 全量窗交回 AppKit（点它最大化）。原生细节与四条非显然的约束见 `system::titlebar`。
    ///
    /// 每帧都问一遍按钮现状，但**只在结果变了才记日志**：这一句在画帧路径上，
    /// 逐帧记会把 `/tmp/ice.log` 刷满（与 `apply_native_alpha` 同一手法）。
    fn apply_zoom_hook(&mut self, frame: &eframe::Frame, want: bool) {
        let outcome = titlebar::sync(frame, want);
        if self.zoom_outcome == Some(outcome) {
            return;
        }
        self.zoom_outcome = Some(outcome);
        match outcome {
            titlebar::Outcome::Hooked => tracing::debug!("标题栏绿键改道：点击切到全量窗"),
            titlebar::Outcome::Released => {
                tracing::debug!("标题栏绿键交回 AppKit：点击是最大化")
            }
            titlebar::Outcome::Unavailable => {
                if !ZOOM_UNHOOKED_WARNED.swap(true, std::sync::atomic::Ordering::Relaxed) {
                    tracing::warn!("接不上标题栏绿键，从悬浮窗切回全量只剩 Cmd+Shift+M");
                }
            }
        }
    }

    /// 启动后的离屏校验（REQ-009）：盘上记的几何整块落在当前可用区之外（外接屏拔掉、
    /// 分辨率变了）时把它夹回可用区并立即落盘，不让窗口"消失得找不回来"（FR-3.6 的验收）。
    ///
    /// 只判一次：`work_area` 要走 NSScreen，空闲帧不该为它付钱。停靠态不归这条管——
    /// 按设计就有 `w − sliver` 在区外（左停靠必然 x 为负），拿整窗判据去夹等于每次启动
    /// 都替用户解了停靠；那一支由 `watch_dock` 的第一帧 `land_dock` 负责重落位，
    /// 而它是按**当前**可用区算收起位的，屏幕没了自然会落到还在的那块屏的边上。
    fn rescue_offscreen(&mut self, ctx: &egui::Context, area_for: impl Fn(Rect) -> Option<Rect>) {
        if self.geometry_rescued || self.state.hidden {
            return;
        }
        if mode::is_docked(self.state.config.ui.docked_edge, self.state.mode()) {
            return;
        }
        let (Some(outer), Some(inner)) = (
            ctx.input(|i| i.viewport().outer_rect),
            ctx.input(|i| i.viewport().inner_rect),
        ) else {
            // 几何还没报到（建窗当帧），这一帧判不了，下一帧再来
            return;
        };
        self.geometry_rescued = true;
        let Some(area) = area_for(outer) else {
            return;
        };
        let window = Rect::from_min_size(outer.min, inner.size());
        if area.intersect(window).is_positive() {
            return;
        }
        let fitted = mode::fit_into_area(window, area);
        tracing::warn!(
            from_x = window.min.x as i32,
            from_y = window.min.y as i32,
            to_x = fitted.min.x as i32,
            to_y = fitted.min.y as i32,
            "启动几何整块在可用区外，已夹回"
        );
        if self.state.mode() == UiMode::Compact {
            self.place_compact(ctx, fitted.min, DockEdge::None);
        } else {
            ctx.send_viewport_cmd(ViewportCommand::OuterPosition(fitted.min));
            self.state.config.ui.full_rect.x = fitted.min.x;
            self.state.config.ui.full_rect.y = fitted.min.y;
            self.state.mark_config_dirty();
            self.flush_config(true);
        }
        ctx.request_repaint();
    }

    /// 停靠状态的每帧看护（REQ-003）：悬停唤出、`DOCK_AUTO_HIDE` 后自动收回、闸门、启动落位。
    ///
    /// 挂在 `settle_drag` 之后：吸附可能刚刚才改动停靠边，同一帧要用改动后的判定。
    /// 挂在 `ui` 而不是 `logic`：唤出/收回的输入是 pointer 事件，隐藏期间根本没有指针。
    /// winit 在鼠标进入窗口时自己会唤醒事件循环，收起态**不需要**常驻重绘心跳（NFR-2）。
    ///
    /// `area_for` 是桌面边界的取法（同 `apply_snap` 的注入理由）：真机走 `screen::desktop`，
    /// 测试给一块固定的屏。它只在需要落位的那几帧被调用——`NSScreen::screens` 每次都要
    /// 构造数组，不该在无停靠态的帧上白跑。
    ///
    /// 它不收窗口矩形做参数：停靠的边界是**整个桌面**的外缘，跟窗口此刻在哪块屏无关。
    /// 之前按单屏可用区判（D-8）出过两次事故——收起位的中心天然在所有屏之外，选屏退回主屏
    /// 会把唤出搬到另一块屏；而按所在屏算出的"边"可能是两块屏的接缝，藏进去既盖不住、
    /// `released` 又永远量不出向内的位移，停靠状态解不开。
    fn watch_dock(&mut self, ctx: &egui::Context, area_for: impl Fn() -> Option<Rect>) {
        let edge = self.state.config.ui.docked_edge;
        if !mode::is_docked(edge, self.state.mode()) {
            self.clear_dock_runtime(ctx);
            return;
        }
        if self.drag_settler.is_armed() {
            // 原生拖拽期间 egui 收不到 pointer 事件（AppKit 进了嵌套事件循环），判了也是错的
            return;
        }
        let (Some(outer), Some(inner)) = (
            ctx.input(|i| i.viewport().outer_rect),
            ctx.input(|i| i.viewport().inner_rect),
        ) else {
            // 读不到几何（还没建好、或正在最小化）：这一帧什么都判不了
            return;
        };
        let window = Rect::from_min_size(outer.min, inner.size());

        // 锚点缺失 = 这一次"停靠"还没真正落到位：带着停靠态启动，或切进简洁形态时盘上存的
        // 是唤出位的 x（上一次在唤出态被强杀）。按配置里的边重算收起位再下发，幂等。
        if self.dock_anchor.is_none() {
            let Some(area) = area_for() else {
                return;
            };
            self.land_dock(ctx, window, area, edge);
            return;
        }

        let pointer = ctx.input(|i| i.pointer.hover_pos());
        if self.dock_shown {
            match pointer {
                Some(_) => self.dock_left_at = None,
                None if self.state.focus_in_source || ctx.input(|i| i.pointer.any_down()) => {
                    // 正在输入行里打字，或按住窗口拖到了边外（选译文选出了界）：
                    // 那两种情况下"指针离开"是操作的一部分，不是"不看我了"（REQ-003 的例外）
                    self.dock_left_at = None;
                }
                None => {
                    let now = Instant::now();
                    let since = *self.dock_left_at.get_or_insert(now);
                    let gone = now.duration_since(since);
                    if gone >= mode::DOCK_AUTO_HIDE {
                        let Some(area) = area_for() else {
                            return;
                        };
                        self.hide_dock(ctx, window, area, edge);
                        // 同一帧就跑到底部的开闸判定：此刻指针还在窗口外，会被当成"已离开创口带"
                        // 而立刻把闸门解开，而窗口刚跳到收起位——下一帧指针落在那条 3px 上就会
                        // 马上再弹出去。闸门必须活到下一帧。
                        return;
                    } else {
                        // 一次性续期：收回计时是这里唯一需要"没人动鼠标也要醒"的东西
                        ctx.request_repaint_after(mode::DOCK_AUTO_HIDE - gone);
                    }
                }
            }
        } else {
            self.dock_left_at = None;
            if pointer.is_some() && !self.dock_latched {
                let Some(area) = area_for() else {
                    return;
                };
                self.show_dock(ctx, window, area, edge);
                return;
            }
        }

        // 闸门解除：指针得先离开创口带（不在窗口里也算离开），下一次进入才允许再唤出
        if self.dock_latched {
            let local = Rect::from_min_size(Pos2::ZERO, inner.size());
            let outside_band = match pointer {
                None => true,
                Some(pos) => {
                    !mode::dock_band_contains(local, edge, pos, self.state.config.ui.dock_sliver_px)
                }
            };
            if outside_band {
                self.dock_latched = false;
            }
        }
    }

    /// 按配置里的停靠边把悬浮条收到位（REQ-003 的"启动落位"，也覆盖形态切换后进来的那一次）。
    ///
    /// 已经在位时只补锚点、一个命令都不发：程序自己挪的那一次会被静止判定当成松手，
    /// 白写一次盘。
    fn land_dock(&mut self, ctx: &egui::Context, window: Rect, area: Rect, edge: DockEdge) {
        self.drag_settler.disarm();
        let tucked = mode::dock(window, area, edge, self.state.config.ui.dock_sliver_px);
        if (tucked.min - window.min).length() <= 0.5 {
            self.dock_anchor = Some(tucked.min);
        } else {
            self.place_compact(ctx, tucked.min, edge);
            ctx.request_repaint();
        }
        self.dock_shown = false;
        self.dock_latched = true;
    }

    /// 悬停唤出：整条回到可用区内，锚点跟着搬到唤出位（TC-3.5-f）。**不改停靠边**。
    fn show_dock(&mut self, ctx: &egui::Context, window: Rect, area: Rect, edge: DockEdge) {
        let shown = mode::reveal_rect(window, area, edge);
        self.drag_settler.disarm();
        self.place_compact(ctx, shown.min, edge);
        self.dock_shown = true;
        self.dock_left_at = None;
        ctx.request_repaint();
    }

    /// 自动收回：藏回边外只留那条热区，并关上闸门（TC-3.5-g）。
    ///
    /// 不关闸门的话，收回瞬间指针恰好还压在热区上 → 下一帧又唤出 → 500ms 后又收回，来回抖。
    fn hide_dock(&mut self, ctx: &egui::Context, window: Rect, area: Rect, edge: DockEdge) {
        let tucked = mode::dock(window, area, edge, self.state.config.ui.dock_sliver_px);
        self.drag_settler.disarm();
        self.place_compact(ctx, tucked.min, edge);
        self.dock_shown = false;
        self.dock_latched = true;
        self.dock_left_at = None;
        ctx.request_repaint();
    }

    /// 不再停靠：清掉运行时状态，并把窗口层级交还给用户的置顶开关。
    ///
    /// 悬浮条期间设的置顶层级会留在 AppKit 上。不补发这一条的话，用户解除停靠之后
    /// 窗口仍然压在所有应用上面，而他以为自己在"置顶"开关上看到的是关闭（Q-012）。
    fn clear_dock_runtime(&mut self, ctx: &egui::Context) {
        let had_state = self.dock_shown
            || self.dock_anchor.is_some()
            || self.dock_left_at.is_some()
            || self.dock_latched;
        self.dock_shown = false;
        self.dock_anchor = None;
        self.dock_left_at = None;
        self.dock_latched = false;
        if had_state {
            mode::apply_topmost(ctx, &mut self.state);
        }
    }

    /// 剪贴板即翻（FR-3.8、REQ-008）。
    ///
    /// 挂在 `logic` 而不是 `ui`：`logic` 每帧都跑（`epi_integration.rs:357` 在 `App::ui`
    /// 之前调它，最小化/被遮挡时走 `update_logic_only` 也调），把"续期"放这里轮询才不会
    /// 随绘制停摆。开关关闭时一个计时器都不立，空闲行为与 P4 的实测一致（NFR-2）。
    /// 窗口收起时既不轮询也不把窗口叫回来（Q-005）。
    ///
    /// 读取动作是注入进来的：真机走 arboard，测试给一个固定内容的闭包——
    /// 与 `watch_dock` 注入 `area_for` 同一个理由，进程级的剪贴板没法在 headless 下验证。
    fn poll_clipboard(&mut self, ctx: &egui::Context, read: impl FnOnce() -> Option<String>) {
        if !self.state.config.ui.clipboard_watch || self.state.hidden {
            return;
        }
        ctx.request_repaint_after(clipboard::CLIPBOARD_POLL);
        // 上一次请求还在飞：这一次不消费，700ms 后再看——把源文换掉却不发请求会留下
        // "源文是新的、译文是旧的"这种误导性画面
        if self.state.in_flight.is_some() {
            return;
        }
        // 焦点判定用的是上一帧绘结束时记下的状态：`logic` 跑在绘制之前，拿不到本帧的
        let Some(text) = self
            .clipboard_watch
            .observe(read(), self.state.focus_in_source)
        else {
            return;
        };
        self.state.source = text;
        // 走与手动提交完全相同的路径，REQ-006 的自动反向因此同样生效
        let Some(job) = self.state.submit() else {
            return;
        };
        self.runner.submit(&self.runtime, job, ctx);
        self.state.toast(Tone::Info, "已用剪贴板新内容自动翻译");
    }

    /// 隐藏期间的看护（缺陷 D-2）。
    ///
    /// 必须挂在 `App::logic` 上：最小化之后 eframe 不再跑 egui pass，只按
    /// `MISSED_REDRAW_TIMEOUT` 调 `logic`（`update_logic_only`），`ui` 一帧都不执行。
    /// 之前把判定和心跳写在 `ui` 里，窗口一收起来就全断了——从 Dock 叫回来时没有任何
    /// 一帧去发现"回来了"，画面就停在旧的那一帧上。
    /// `run_logic` 会刷新 `viewport()` 里的几何与焦点，也会把这里发的命令带出去，够用。
    fn watch_visibility(&mut self, ctx: &egui::Context) {
        let (inner_rect, focused) = ctx.input(|i| (i.viewport().inner_rect, i.viewport().focused));
        if self
            .reveal_guard
            .check(self.state.hidden, Instant::now(), inner_rect, focused)
        {
            mode::reveal(ctx, &mut self.state, &mut self.pending_geometry);
        }
        if self.state.hidden {
            ctx.request_repaint_after(mode::HIDDEN_HEARTBEAT);
        }
    }

    /// 帧末统一执行副作用：view 只改 `AppState`，碰系统的地方全在这里（REQ-005 的验证层次）
    fn run_effects(&mut self, ctx: &egui::Context, effects: Effects) {
        // 顺序有意义：术语与引擎重建要先于本帧的任务派发，否则新任务会用旧参数跑一遍
        if effects.sync_glossary {
            self.runner.set_glossary(self.state.glossary.clone());
        }
        if effects.rebuild_translator {
            self.rebuild_translator();
        }
        if let Some(job) = effects.job {
            self.runner.submit(&self.runtime, job, ctx);
        }
        if let Some(text) = effects.copy {
            match clipboard::write(&text) {
                Ok(()) => {
                    // 即翻开着时这一笔会被下一次轮询读到；记下它，"复制译文"才不会
                    // 立刻拿译文再翻一次（REQ-008 第二条）
                    self.clipboard_watch.note_written(&text);
                    self.state.toast(Tone::Info, "已复制到剪贴板");
                }
                Err(reason) => self.state.toast(Tone::Error, format!("复制失败：{reason}")),
            }
        }
        if let Some(key) = effects.save_key {
            self.save_key(key);
        }
        if effects.delete_key {
            self.delete_key();
        }
        if effects.toggle_mode {
            let mode = mode::toggle(ctx, &mut self.state, &mut self.pending_geometry);
            self.state.toast(
                Tone::Info,
                match mode {
                    UiMode::Full => "已切到全量窗口",
                    UiMode::Compact => "已切到简洁悬浮窗",
                },
            );
        }
        if effects.open_settings {
            self.open_settings(ctx);
        }
        if effects.toggle_topmost {
            self.toggle_topmost(ctx);
        }
        if effects.clear_history {
            self.state.clear_history(&self.history);
        }
        // REQ-011：两颗新按钮都接现成的路径，不另开实现。
        // 退出走 `Action::Quit` 那一条（`capture` → 强制落盘 → `mode::quit`），
        // 恢复尺寸是唯一一处"程序替用户改配置"，所以只按一次、且立即落盘。
        if effects.quit {
            self.run(Action::Quit, ctx, &mut Effects::default());
        }
        if effects.reset_geometry {
            self.reset_window_sizes(ctx);
        }
        // 拖拽命令排在最后：同一帧里若还有几何命令（形态切换、换停靠边），
        // 得先让它们进队，`StartDrag` 才不会把窗口从一个正在变化的状态里抓走
        if effects.start_drag {
            self.start_drag(ctx);
        }
    }

    /// 写钥匙串。明文只在这一处出现，日志里只有长度与哈希前缀（FR-4.2、ADR-005）。
    fn save_key(&mut self, key: String) {
        match secrets::write(&key) {
            Ok(()) => {
                self.state.config.ai.key_configured = true;
                self.state.mark_config_dirty();
                self.state.key = secrets::probe();
                if let KeyState::Configured { len, hash8 } = &self.state.key {
                    tracing::info!(key_len = len, key_hash8 = hash8, "API Key 已存入钥匙串");
                }
                self.state.toast(Tone::Info, "API Key 已存入钥匙串");
                self.rebuild_translator();
            }
            Err(reason) => {
                self.state.key = reason.clone();
                self.state.toast(
                    Tone::Error,
                    format!("Key 没能存进钥匙串：{}", reason_text(&reason)),
                );
            }
        }
    }

    fn delete_key(&mut self) {
        match secrets::delete() {
            Ok(()) => {
                self.state.config.ai.key_configured = false;
                self.state.mark_config_dirty();
                self.state.key = secrets::probe();
                self.state
                    .toast(Tone::Info, "已清除钥匙串里的 API Key，机翻不受影响");
                self.rebuild_translator();
            }
            Err(reason) => {
                self.state.key = reason.clone();
                self.state
                    .toast(Tone::Error, format!("清除失败：{}", reason_text(&reason)));
            }
        }
    }

    /// AI 配置或 Key 变了就整套重建。读不到 Key 时降级成"只有机翻"，界面照常可用。
    pub fn rebuild_translator(&mut self) {
        let key = match secrets::read() {
            Ok(key) => key,
            Err(reason) => {
                self.state.key = reason;
                None
            }
        };
        let translator = build_translator(&self.state.config, key);
        self.install_translator(translator);
    }

    /// 换整套引擎。旧实例连同它内部的术语表一起被丢掉，所以每次都要重灌一遍。
    fn install_translator(&mut self, translator: Translator) {
        self.runner.replace_translator(translator);
        self.runner.set_glossary(self.state.glossary.clone());
    }

    /// 后台探测回来后对齐状态。钥匙串是事实来源，config 里的 `key_configured` 只是缓存标记。
    fn apply_key_probe(&mut self, probe: Result<Option<String>, KeyState>) {
        match probe {
            Ok(Some(key)) => {
                let (len, hash8) = secrets::fingerprint(&key);
                tracing::info!(key_len = len, key_hash8 = hash8, "读到 API Key");
                self.state.key = KeyState::Configured { len, hash8 };
                if !self.state.config.ai.key_configured {
                    self.state.config.ai.key_configured = true;
                    self.state.mark_config_dirty();
                }
                let translator = build_translator(&self.state.config, Some(key));
                self.install_translator(translator);
            }
            Ok(None) => {
                self.state.key = KeyState::Missing;
                // 用户在"钥匙串访问"里手动删过条目
                if self.state.config.ai.key_configured {
                    self.state.config.ai.key_configured = false;
                    self.state.mark_config_dirty();
                }
            }
            // 钥匙串被锁或权限被拒：AI 通道不可用，机翻照常，应用继续跑（REQ-005）
            Err(reason) => {
                tracing::warn!("钥匙串不可用，AI 通道本次不启用");
                let text = format!("钥匙串不可用：{}", reason_text(&reason));
                self.state.key = reason;
                self.state.toast(Tone::Warn, text);
            }
        }
    }

    /// 落盘配置。`force` 用于退出与 Cmd+Q；失败时把脏标记放回去，下一次还会重试。
    pub fn flush_config(&mut self, force: bool) {
        if !self.state.config_dirty {
            return;
        }
        if !force && self.last_save.elapsed() < mode::CONFIG_SAVE_INTERVAL {
            return;
        }
        self.last_save = Instant::now();
        self.state.config_dirty = false;
        if let Err(err) = self.state.config.save_at(&self.config_path) {
            self.state.config_dirty = true;
            tracing::error!("配置保存失败: {err}");
            self.state
                .toast(Tone::Error, format!("配置未能保存：{err}"));
        }
    }

    /// 退出：先存配置，再放锁。顺序不能反——放锁之后另一个实例就能起来写同一份文件。
    pub fn shutdown(&mut self) {
        self.flush_config(true);
        if let Some(lock) = self.lock.take() {
            lock.release();
        }
    }
}

/// 恢复默认尺寸时位置怎么定（`reset_window_sizes` 的一条规则，抽出来才能两头都断言）：
/// 拿得到可用区就在其中居中，拿不到就留在原处——无头测试与探针失败的那些帧里，
/// 宁可不挪位置，也不能把窗口甩到屏幕外。
fn restored_rect(default: WindowRect, keep: WindowRect, monitor: Option<WindowRect>) -> WindowRect {
    match monitor {
        Some(m) => WindowRect::centered_in(&m, default.w, default.h),
        None => WindowRect {
            x: keep.x,
            y: keep.y,
            ..default
        },
    }
}

/// `KeyState::Error` 里的文案已经是给用户看的，这里只是为了不把整个枚举 `Display` 出来
fn reason_text(state: &KeyState) -> String {
    match state {
        KeyState::Error(reason) => reason.clone(),
        other => other.label().to_string(),
    }
}

/// 原生整窗 alpha 走不通的警告只发一次（回退本身每帧都要重试，见 `needs_alpha_update`）。
static ALPHA_FALLBACK_WARNED: std::sync::atomic::AtomicBool =
    std::sync::atomic::AtomicBool::new(false);

/// 标题栏绿键接不上只发一次警告（`apply_zoom_hook`；接不上时每帧都会再试一次）
static ZOOM_UNHOOKED_WARNED: std::sync::atomic::AtomicBool =
    std::sync::atomic::AtomicBool::new(false);

/// 这一档不透明度要不要真的下发给窗口服务器（`apply_native_alpha` 的判定核心，抽出来才能无头断言）。
///
/// `applied` 是上一帧**成功**下到的值，`None` 表示还没下过或上次没走通。相同就跳过：
/// `setAlphaValue:` 一旦每帧都发，AppKit 每帧都会把整个窗口标脏重合成一遍，
/// 空闲 CPU 直接顶穿 NFR-2 的 1% 上限——悬浮窗停在桌面上不动时也应该几乎不耗电。
fn needs_alpha_update(applied: Option<f32>, want: f32) -> bool {
    applied != Some(want)
}

/// 按当前配置装配引擎：机翻恒在，AI 只在"配置齐备且拿到了 Key"时注册（REQ-005、R-4）。
pub fn build_translator(config: &Config, key: Option<String>) -> Translator {
    let mut translator = Translator::new();
    translator.register(std::sync::Arc::new(MachineTranslateEngine::from_config(
        config,
    )));
    let key = key.unwrap_or_default();
    if !key.is_empty() && config.ai.is_ready() {
        translator.register(std::sync::Arc::new(AiEngine::from_config(config, key)));
    }
    translator
}

impl eframe::App for IceApp {
    /// 窗口收起来之后 eframe 只调这一个钩子（见 `watch_visibility`）
    fn logic(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.watch_visibility(ctx);
        self.poll_clipboard(ctx, clipboard::read);
    }

    fn ui(&mut self, ui: &mut egui::Ui, frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        theme::apply(&ctx, &self.state.config.ui);

        jobs::drain(&mut self.outcomes, &mut self.state, &self.history);
        if let Ok(probe) = self.key_probe.try_recv() {
            self.apply_key_probe(probe);
        }

        let focus_in_source = self.state.focus_in_source;
        let actions = ctx.input(|i| hotkeys::action_of(i, focus_in_source));
        // 窗口关闭按钮：Cmd+Q 之后不再拦截，其余情况一律收进后台（FR-5.2）
        let close_requested = ctx.input(|i| i.viewport().close_requested());

        let mut effects = Effects::default();
        self.poll_titlebar_click(&mut effects);
        for action in actions {
            self.run(action, &ctx, &mut effects);
        }

        if close_requested && !self.state.quitting {
            self.hide(&ctx);
        }

        self.refresh_geometry(&ctx);
        self.trace_geometry(&ctx);
        self.settle_drag(&ctx, || screen::desktop(&ctx));
        self.watch_dock(&ctx, || screen::desktop(&ctx));
        self.rescue_offscreen(&ctx, |outer| screen::work_area(&ctx, outer));

        let ai_available = self.ai_available();
        match self.state.mode() {
            UiMode::Full => {
                // 全量窗永不半透明（REQ-007 第四条），但这一档要在这里显式写回 100%：
                // 上一帧可能正是失焦的简洁窗，`native_alpha` 还停在 0.85 那一档上
                self.apply_native_alpha(frame, 1.0);
                // 绿键也一并交回 AppKit：全量窗里它还是最大化（改道只属于悬浮窗）
                self.apply_zoom_hook(frame, false);
                views::show(
                    ui,
                    &mut self.state,
                    &mut effects,
                    &self.history,
                    ai_available,
                )
            }
            UiMode::Compact => {
                // 用户 2026-10-28 把动作行里那颗「全量」按钮删了，切回全量改由绿键承担
                self.apply_zoom_hook(frame, true);
                // 失焦半透明（REQ-007）：焦点是窗口级事实，由帧循环读给视图，视图自己不去问
                let focused = ctx.input(|i| i.viewport().focused) == Some(true);
                let want = mode::dim_alpha(
                    UiMode::Compact,
                    focused,
                    self.state.config.ui.dim_when_unfocused,
                    self.state.config.ui.unfocused_alpha,
                );
                // 两条路径二选一，不能叠加（叠了会淡成 want²）。原生那条能让标题栏一起变淡
                // （FR-3.7 用户走查点名的就是"标题栏为什么不跟着透"），走通了就不再动 egui 底色。
                let native = self.apply_native_alpha(frame, want);
                let dim = if native { 1.0 } else { want };
                views::compact::show(ui, &mut self.state, &mut effects, ai_available, dim)
            }
        }

        self.run_effects(&ctx, effects);

        if self.state.tick_toasts() {
            ctx.request_repaint();
        }
        if !self.state.toasts.is_empty() {
            // 让提示条到点自己消失：不常驻重绘，空闲时 CPU 才能压到 1% 以下（NFR-2）
            ctx.request_repaint_after(state::TOAST_VISIBLE);
        }
        self.flush_config(false);
    }

    fn on_exit(&mut self) {
        self.shutdown();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    use crate::state::Job;
    use crate::stub::StubEngine;
    use crate::system::single_instance;
    use crate::views::Effects;
    use ice_core::config::{AiProvider, UiConfig};
    use ice_core::glossary::{Glossary, Term};
    use ice_core::language::EngineKind;
    use ice_providers::request::TranslationResult;

    fn ctx() -> egui::Context {
        egui::Context::default()
    }

    fn runtime() -> Runtime {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap()
    }

    /// 测试绝不碰真钥匙串：给一个已经关闭的通道，等价于"探测还没回来"
    fn idle_key_probe() -> KeyProbeReceiver {
        let (tx, rx) = mpsc::channel(1);
        drop(tx);
        rx
    }

    fn app(dir: &std::path::Path, state: AppState) -> IceApp {
        app_with_history(dir, state, HistoryStore::at(dir.join("history.jsonl")))
    }

    fn plain_state() -> AppState {
        AppState::new(
            Config::default(),
            Glossary::default(),
            KeyState::Missing,
            None,
        )
    }

    #[test]
    fn machine_engine_is_always_registered_and_ai_needs_both_config_and_key() {
        let config = Config::default();
        let translator = build_translator(&config, Some("sk-test".into()));
        assert!(translator.has_engine(EngineKind::Machine));
        assert!(
            !translator.has_engine(EngineKind::Ai),
            "Custom 且地址为空 → 不该注册 AI"
        );

        let mut config = Config::default();
        config.ai.provider = AiProvider::DeepSeek;
        config.ai.key_configured = true;
        assert!(config.ai.is_ready());

        let with_key = build_translator(&config, Some("sk-test".into()));
        assert!(with_key.has_engine(EngineKind::Ai));

        let without_key = build_translator(&config, None);
        assert!(
            !without_key.has_engine(EngineKind::Ai),
            "配置齐备但钥匙串里没有 Key，也不能注册 AI"
        );
    }

    #[test]
    fn translate_action_queues_a_job_instead_of_calling_the_network() {
        let dir = tempfile::tempdir().unwrap();
        let mut state = plain_state();
        state.source = "hello".into();
        let mut app = app(dir.path(), state);
        let mut effects = Effects::default();

        app.run(Action::Translate, &ctx(), &mut effects);

        let job: Job = effects.job.expect("有源文就该派发");
        assert_eq!(job.engine, EngineKind::Machine);
        assert_eq!(job.request.text, "hello");
        assert_eq!(app.state.in_flight, Some(job.id));
    }

    #[test]
    fn translate_on_an_empty_source_queues_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let mut app = app(dir.path(), plain_state());
        let mut effects = Effects::default();
        app.run(Action::Translate, &ctx(), &mut effects);
        assert!(effects.job.is_none());
        assert!(app.state.in_flight.is_none());
    }

    #[test]
    fn hide_window_keeps_the_process_and_the_content() {
        let dir = tempfile::tempdir().unwrap();
        let mut state = plain_state();
        state.source = "别丢我".into();
        let mut app = app(dir.path(), state);
        let c = ctx();
        let mut effects = Effects::default();

        app.run(Action::HideWindow, &c, &mut effects);

        let commands = c.viewport(|vp| vp.commands.clone());
        assert!(commands.contains(&egui::ViewportCommand::CancelClose));
        assert!(commands.contains(&egui::ViewportCommand::Minimized(true)));
        assert!(app.state.hidden);
        assert!(!app.state.quitting, "隐藏不是退出");
        assert_eq!(app.state.source, "别丢我");
    }

    /// 把窗口的几何与焦点喂给无头 `Context`，等价于 `update_viewport_info` 每帧的刷新
    fn feed_viewport(ctx: &egui::Context, inner_rect: Option<egui::Rect>, focused: Option<bool>) {
        let info = egui::ViewportInfo {
            inner_rect,
            focused,
            ..Default::default()
        };
        let mut raw = egui::RawInput::default();
        raw.viewports.insert(egui::ViewportId::ROOT, info);
        ctx.run_ui(raw, |_| {}).drop_without_applying_deltas();
    }

    /// 缺陷 D-2 第二轮的真正成因：隐藏期间 eframe 只调 `App::logic`、`App::ui` 一帧不跑，
    /// 判定挂在 `ui` 上等于没写。这条测试走的就是 `logic` 的实现 `watch_visibility`。
    #[test]
    fn restoring_from_the_dock_is_noticed_by_the_logic_hook() {
        let dir = tempfile::tempdir().unwrap();
        let mut app = app(dir.path(), plain_state());
        let c = ctx();
        let mut effects = Effects::default();
        app.run(Action::HideWindow, &c, &mut effects);
        assert!(app.state.hidden);

        // 最小化中：几何塌成 None、焦点也没了
        feed_viewport(&c, None, Some(false));
        app.watch_visibility(&c);
        assert!(app.state.hidden, "还收着，不该自己弹回来");

        // 用户点 Dock 叫回来：几何与焦点都回来了，但静默期未过（收起那一帧就是这个样子）
        let rect = Some(egui::Rect::from_min_size(
            egui::Pos2::ZERO,
            egui::Vec2::new(1080.0, 720.0),
        ));
        feed_viewport(&c, rect, Some(true));
        app.watch_visibility(&c);
        assert!(
            app.state.hidden,
            "静默期内不判恢复，Cmd+W 才不会被自己弹回来"
        );

        std::thread::sleep(mode::REVEAL_GRACE + Duration::from_millis(50));
        app.watch_visibility(&c);

        assert!(!app.state.hidden, "恢复判定没跑，窗口会停在旧画面上（D-2）");
        let commands = c.viewport(|vp| vp.commands.clone());
        assert!(commands.contains(&egui::ViewportCommand::Minimized(false)));
        assert!(
            commands.contains(&egui::ViewportCommand::Decorations(true)),
            "当前形态的几何要整套重发，逼 winit 同步 surface 尺寸"
        );
    }

    #[test]
    fn quit_writes_the_config_before_releasing_the_lock() {
        let dir = tempfile::tempdir().unwrap();
        let lock = single_instance::acquire(dir.path()).expect("拿到锁");
        let lock_path = lock.path().to_path_buf();

        let mut state = plain_state();
        state.config.ui.always_on_top = true;
        state.config.translation.target = ice_core::language::Language::Ja;
        state.mark_config_dirty();

        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let translator = build_translator(&state.config, None);
        let mut app = IceApp::new(
            state,
            runtime,
            translator,
            HistoryStore::at(dir.path().join("history.jsonl")),
            dir.path().join("config.toml"),
            Some(lock),
            idle_key_probe(),
        );

        app.shutdown();

        assert!(!lock_path.exists(), "退出后锁必须放掉");
        let (saved, _) = Config::load_at(&dir.path().join("config.toml")).unwrap();
        assert!(saved.ui.always_on_top);
        assert_eq!(saved.translation.target, ice_core::language::Language::Ja);
    }

    #[test]
    fn config_writes_are_debounced_but_a_forced_save_always_happens() {
        let dir = tempfile::tempdir().unwrap();
        let mut app = app(dir.path(), plain_state());
        let path = dir.path().join("config.toml");

        app.state.mark_config_dirty();
        app.flush_config(false);
        assert!(!path.exists(), "刚存过就又被标脏，半秒内不该再写一次");
        assert!(app.state.config_dirty, "脏标记要留着");

        app.flush_config(true);
        assert!(path.exists());
        assert!(!app.state.config_dirty);
    }

    #[test]
    fn a_failed_save_keeps_the_dirty_flag_for_the_next_try() {
        let dir = tempfile::tempdir().unwrap();
        let mut app = app(dir.path(), plain_state());
        // 指向一个不可能写成功的路径：父目录是个普通文件
        std::fs::write(dir.path().join("blocker"), b"x").unwrap();
        app.config_path = dir.path().join("blocker").join("config.toml");
        app.state.mark_config_dirty();

        app.flush_config(true);

        assert!(app.state.config_dirty, "没存成就要重试");
        assert_eq!(app.state.toasts.len(), 1);
        assert_eq!(app.state.toasts[0].tone, Tone::Error);
    }

    #[test]
    fn startup_notice_becomes_a_toast_and_history_is_loaded_once() {
        let dir = tempfile::tempdir().unwrap();
        let store = HistoryStore::at(dir.path().join("history.jsonl"));
        store
            .append(&ice_core::history::HistoryRecord {
                ts: 1_760_000_000,
                engine: EngineKind::Machine,
                detected: None,
                target: ice_core::language::Language::ZhHans,
                source_text: "hello".into(),
                result_text: "你好".into(),
                elapsed_ms: 120,
            })
            .unwrap();

        let state = AppState::new(
            Config::default(),
            Glossary::default(),
            KeyState::Missing,
            Some("配置文件解析失败，本次以默认配置启动".into()),
        );
        let app = app_with_history(dir.path(), state, store);

        assert_eq!(app.state.toasts.len(), 1);
        assert_eq!(app.state.toasts[0].tone, Tone::Warn);
        assert!(app.state.startup_notice.is_none());
        assert_eq!(app.state.history_count, 1, "状态条上的历史条数启动时就要对");
        assert_eq!(app.state.tab, Tab::Text);
    }

    fn app_with_history(dir: &std::path::Path, state: AppState, history: HistoryStore) -> IceApp {
        let translator = build_translator(&state.config, None);
        IceApp::new(
            state,
            runtime(),
            translator,
            history,
            dir.join("config.toml"),
            None,
            idle_key_probe(),
        )
    }

    /// FR-4.5：config.toml 里的术语在**第一次**派发前就要进引擎，
    /// 不能等用户在设置页改一次术语才生效。
    #[test]
    fn glossary_from_config_reaches_the_engine_on_the_first_job() {
        let dir = tempfile::tempdir().unwrap();
        let config = Config {
            terms: vec![Term::new("pod", "舱体")],
            ..Default::default()
        };
        let mut state = AppState::new(
            config.clone(),
            Glossary::from_config(&config),
            KeyState::Missing,
            None,
        );
        state.engine = EngineKind::Ai;
        state.source = "the pod".into();

        let stub = StubEngine::replying(EngineKind::Ai, "舱体");
        let mut translator = Translator::new();
        translator.register(std::sync::Arc::new(stub.clone()));

        let mut app = IceApp::new(
            state,
            runtime(),
            translator,
            HistoryStore::at(dir.path().join("history.jsonl")),
            dir.path().join("config.toml"),
            None,
            idle_key_probe(),
        );

        let ctx = ctx();
        let job = app.state.submit().expect("有源文就该能提交");
        app.runner.submit(&app.runtime, job, &ctx);
        let outcome = app.runtime.block_on(async { app.outcomes.recv().await });
        assert!(outcome.is_some(), "任务要能跑完");

        let hint = stub
            .last_request()
            .and_then(|req| req.glossary_hint)
            .expect("启动就该带上 config 里的术语");
        assert!(hint.contains("pod => 舱体"), "{hint}");
    }

    /// 装了假引擎的 app：剪贴板即翻要能看到"请求真的派发出去了"，
    /// 而 `app()` 那条夹具走的是真机翻引擎（会打网络）。
    fn app_with_stub(dir: &std::path::Path, state: AppState, stub: &StubEngine) -> IceApp {
        let mut translator = Translator::new();
        translator.register(std::sync::Arc::new(stub.clone()));
        IceApp::new(
            state,
            runtime(),
            translator,
            HistoryStore::at(dir.join("history.jsonl")),
            dir.join("config.toml"),
            None,
            idle_key_probe(),
        )
    }

    fn watch_state() -> AppState {
        let mut config = Config::default();
        config.ui.clipboard_watch = true;
        AppState::new(config, Glossary::default(), KeyState::Missing, None)
    }

    /// REQ-008 接线：轮询读到外部新内容 → 填进源文 → 走与手动提交同一条派发路径。
    /// 剪贴板读取是注入的闭包，测试不碰进程级剪贴板。
    #[test]
    fn the_clipboard_poll_fills_the_source_and_dispatches_a_job() {
        let dir = tempfile::tempdir().unwrap();
        let stub = StubEngine::replying(EngineKind::Machine, "你好，世界");
        let handle = stub.clone();
        let mut app = app_with_stub(dir.path(), watch_state(), &stub);
        let c = ctx();

        app.poll_clipboard(&c, || Some("hello world".into()));

        assert_eq!(app.state.source, "hello world");
        let id = app.state.request_id;
        assert!(app.state.in_flight.is_some(), "读到新内容就该派发");
        assert_eq!(app.state.toasts.len(), 1);
        assert_eq!(app.state.toasts[0].text, "已用剪贴板新内容自动翻译");

        // 上一次请求还在飞：连读都不读，700ms 后的下一帧自然再看
        let mut reads = 0usize;
        app.poll_clipboard(&c, || {
            reads += 1;
            Some("second thought".into())
        });
        assert_eq!(reads, 0, "在途不该再发第二个请求");
        assert_eq!(
            app.state.source, "hello world",
            "换了源文却不发请求会留下误导性画面"
        );

        // 飞着的请求跑完并落到界面（`recv` 会把这一条取走，所以要 `apply` 而不是 `drain`）
        let outcome = app.runtime.block_on(async { app.outcomes.recv().await });
        let applied = jobs::apply(&mut app.state, &app.history, outcome.expect("任务要跑完"));
        assert!(applied, "结果对得上 request_id，不该当过期丢掉");
        assert_eq!(handle.calls(), 1);
        assert_eq!(
            handle.last_request().map(|req| req.text),
            Some("hello world".into())
        );

        // 同一段内容再被读到一次：不是新东西，不许叠第二个请求（REQ-008 第一条）
        app.poll_clipboard(&c, || Some("hello world".into()));
        assert_eq!(app.state.request_id, id, "内容没变不该再翻一遍");
        assert!(app.state.in_flight.is_none());

        // 换一段真的新内容：立刻又翻
        app.poll_clipboard(&c, || Some("good morning".into()));
        assert_eq!(app.state.request_id, id + 1);
        assert_eq!(app.state.source, "good morning");
    }

    /// "本应用复制译文"那一笔会被下一次轮询读到，不认它就会出现"复制→立刻又翻一次"的循环。
    #[test]
    fn the_poll_ignores_the_text_the_app_itself_just_copied() {
        let dir = tempfile::tempdir().unwrap();
        let stub = StubEngine::replying(EngineKind::Machine, "你好，世界");
        let mut app = app_with_stub(dir.path(), watch_state(), &stub);
        let c = ctx();

        app.clipboard_watch.note_written("你好，世界");
        let mut reads = 0usize;
        app.poll_clipboard(&c, || {
            reads += 1;
            Some("你好，世界".into())
        });

        assert_eq!(reads, 1, "开着就要继续读，只是这一笔不当新内容");
        assert!(app.state.source.is_empty(), "自己写的译文不该变成源文");
        assert!(app.state.in_flight.is_none());
        assert_eq!(stub.calls(), 0);
    }

    /// 开关关着（默认值）和窗口收起时：连剪贴板都不去读一下（NFR-3、Q-005）。
    /// 计数闭包就是探针——"没填源文"可能只是内容恰好没变化，"一次都没读"才是完全不碰。
    #[test]
    fn a_disabled_or_hidden_watch_never_reads_the_clipboard() {
        let dir = tempfile::tempdir().unwrap();
        let stub = StubEngine::replying(EngineKind::Machine, "x");
        let c = ctx();
        let mut app = app_with_stub(dir.path(), plain_state(), &stub);

        let mut reads = 0usize;
        app.poll_clipboard(&c, || {
            reads += 1;
            Some("hello world".into())
        });

        assert_eq!(reads, 0, "开关关闭（默认值）时一次都不该读");
        assert!(app.state.source.is_empty());
        assert!(app.state.in_flight.is_none());

        let mut app = app_with_stub(dir.path(), watch_state(), &stub);
        app.state.hidden = true;
        let mut reads = 0usize;
        app.poll_clipboard(&c, || {
            reads += 1;
            Some("hello world".into())
        });

        assert_eq!(reads, 0, "收起的窗口同样不轮询");
        assert!(app.state.source.is_empty());
    }

    #[test]
    fn the_probe_result_actually_arrives_over_the_channel() {
        let runtime = runtime();
        let mut rx = spawn_probe_with(&runtime, &ctx(), || Ok(Some("sk-test-value".into())));
        let probe = runtime
            .block_on(async { rx.recv().await })
            .expect("探测结果要真的送到帧循环");
        assert_eq!(probe.unwrap().as_deref(), Some("sk-test-value"));
    }

    /// 探测没回来之前不能声称 AI 就绪，也不能改 config 里上一次的缓存标记。
    #[test]
    fn a_pending_key_probe_leaves_ai_unavailable_until_it_lands() {
        let dir = tempfile::tempdir().unwrap();
        let mut config = Config::default();
        config.ai.provider = AiProvider::DeepSeek;
        config.ai.key_configured = true;
        let state = AppState::new(config, Glossary::default(), KeyState::Unknown, None);
        let mut app = app(dir.path(), state);

        assert_eq!(app.state.key, KeyState::Unknown);
        assert!(!app.ai_available(), "钥匙串还没读完，不能算就绪");

        app.apply_key_probe(Ok(Some("sk-test-value".into())));

        assert!(app.ai_available());
        assert!(matches!(
            app.state.key,
            KeyState::Configured { len: 13, .. }
        ));
        assert!(app.state.config.ai.key_configured);
        assert!(
            !app.state.config_dirty,
            "标记本来就和钥匙串一致，不该白写一次盘"
        );
    }

    /// 用户在"钥匙串访问"里手动删过条目：以钥匙串为准，并把标记写回去。
    #[test]
    fn the_probe_overrides_a_stale_configured_flag() {
        let dir = tempfile::tempdir().unwrap();
        let mut config = Config::default();
        config.ai.provider = AiProvider::DeepSeek;
        config.ai.key_configured = true;
        let state = AppState::new(config, Glossary::default(), KeyState::Unknown, None);
        let mut app = app(dir.path(), state);

        app.apply_key_probe(Ok(None));

        assert_eq!(app.state.key, KeyState::Missing);
        assert!(!app.state.config.ai.key_configured);
        assert!(app.state.config_dirty, "对齐结果要落盘");
        assert!(!app.ai_available());
    }

    /// 钥匙串被锁或权限被拒：给一条可见提示，机翻照常，应用不退出（REQ-005）。
    #[test]
    fn an_unavailable_keychain_degrades_to_a_toast() {
        let dir = tempfile::tempdir().unwrap();
        let state = AppState::new(
            Config::default(),
            Glossary::default(),
            KeyState::Unknown,
            None,
        );
        let mut app = app(dir.path(), state);

        app.apply_key_probe(Err(KeyState::Error("钥匙串被锁定".into())));

        assert!(!app.ai_available());
        assert_eq!(
            app.state.key,
            KeyState::Error("钥匙串被锁定".into()),
            "设置页要显示失败原因"
        );
        assert!(
            app.state
                .toasts
                .iter()
                .any(|t| t.tone == Tone::Warn && t.text.contains("钥匙串不可用"))
        );
    }

    /// 换了整套引擎之后术语表会随旧实例一起消失，必须重灌。
    #[test]
    fn rebuilding_the_translator_keeps_the_glossary() {
        let dir = tempfile::tempdir().unwrap();
        let config = Config {
            terms: vec![Term::new("pod", "舱体")],
            ..Default::default()
        };
        let mut state = AppState::new(
            config.clone(),
            Glossary::from_config(&config),
            KeyState::Missing,
            None,
        );
        state.engine = EngineKind::Ai;
        state.source = "the pod".into();

        let stub = StubEngine::replying(EngineKind::Ai, "舱体");
        let mut translator = Translator::new();
        translator.register(std::sync::Arc::new(stub.clone()));

        let mut app = IceApp::new(
            state,
            runtime(),
            translator,
            HistoryStore::at(dir.path().join("history.jsonl")),
            dir.path().join("config.toml"),
            None,
            idle_key_probe(),
        );
        app.state.glossary = Glossary::from_config(&config);
        let mut replacement = Translator::new();
        replacement.register(std::sync::Arc::new(stub.clone()));
        app.install_translator(replacement);

        let ctx = ctx();
        let job = app.state.submit().expect("有源文就该能提交");
        app.runner.submit(&app.runtime, job, &ctx);
        let _ = app.runtime.block_on(async { app.outcomes.recv().await });

        let hint = stub
            .last_request()
            .and_then(|req| req.glossary_hint)
            .expect("重建之后术语仍要生效");
        assert!(hint.contains("pod => 舱体"), "{hint}");
    }

    #[test]
    fn ai_availability_needs_config_and_a_key_not_just_one_of_them() {
        let dir = tempfile::tempdir().unwrap();
        let mut config = Config::default();
        config.ai.provider = AiProvider::DeepSeek;
        let state = AppState::new(config, Glossary::default(), KeyState::Missing, None);
        let mut app = app(dir.path(), state);
        assert!(!app.ai_available(), "配置齐了但缺 Key");

        app.state.key = KeyState::Configured {
            len: 51,
            hash8: "a1b2c3d4".into(),
        };
        app.state.config.ai.key_configured = true;
        assert!(app.ai_available());

        // 两处标记必须一致：`save_key` / `delete_key` / `prepare` 都会同时写，任一处缺失都不算就绪
        app.state.config.ai.key_configured = false;
        assert!(!app.ai_available(), "config 里的标记被清掉就不算就绪");

        app.state.config.ai.key_configured = true;
        app.state.key = KeyState::Missing;
        assert!(!app.ai_available(), "钥匙串里没有 Key 就不算就绪");
    }

    // ---- CHG-004：简洁窗的接线（REQ-002 拖拽 / REQ-003 吸附 / REQ-005 清空与设置）----

    fn queued(ctx: &egui::Context) -> Vec<ViewportCommand> {
        ctx.viewport(|vp| vp.commands.clone())
    }

    /// 1440×900 的屏，菜单栏 25pt 在上、Dock 70pt 在下（与 `mode.rs` 的吸附测试同一块屏）
    fn area() -> Rect {
        Rect::from_min_max(Pos2::new(0.0, 25.0), Pos2::new(1440.0, 830.0))
    }

    /// 简洁窗的默认尺寸（与 `UiConfig::default()` 的 `compact_rect` 一致）
    const SIZE: egui::Vec2 = egui::Vec2::new(460.0, 160.0);
    /// 收起后留在屏幕上的热区宽度，与 `UiConfig::dock_sliver_px` 的默认值同源
    const SLIVER: f32 = 3.0;
    /// 右停靠的两个原点 x：收起位只留热区，唤出位整条在屏内
    const TUCKED_X: f32 = 1440.0 - SLIVER;
    const SHOWN_X: f32 = 1440.0 - 460.0;
    /// 热区中心的指针位置（窗口本地坐标；收起态只有左边那 `SLIVER` 像素在屏上）
    const STRIP: Pos2 = Pos2::new(SLIVER / 2.0, 60.0);

    fn compact_state() -> AppState {
        let config = Config {
            ui: UiConfig {
                mode: UiMode::Compact,
                ..UiConfig::default()
            },
            ..Default::default()
        };
        let mut state = AppState::new(config, Glossary::default(), KeyState::Missing, None);
        state.source = "hello".into();
        state
    }

    /// 把窗口的几何与焦点喂给无头 `Context`，等价于 `update_viewport_info` 每帧的刷新
    fn feed_geometry(ctx: &egui::Context, outer: Option<Rect>, focused: Option<bool>) {
        let info = egui::ViewportInfo {
            inner_rect: outer,
            outer_rect: outer,
            focused,
            ..Default::default()
        };
        let mut raw = egui::RawInput::default();
        raw.viewports.insert(egui::ViewportId::ROOT, info);
        ctx.run_ui(raw, |_| {}).drop_without_applying_deltas();
    }

    /// 喂一帧停靠看护要的输入：窗口几何 + 指针（`None` 等价于 winit 的 `CursorLeft` →
    /// `Event::PointerGone` → `hover_pos()` 为 None）+ 指针是否按着不放
    fn feed_pointer(ctx: &egui::Context, window: Rect, pointer: Option<Pos2>, held: bool) {
        let info = egui::ViewportInfo {
            inner_rect: Some(window),
            outer_rect: Some(window),
            focused: Some(true),
            ..Default::default()
        };
        let mut raw = egui::RawInput::default();
        raw.viewports.insert(egui::ViewportId::ROOT, info);
        raw.events.push(match pointer {
            Some(pos) => egui::Event::PointerMoved(pos),
            None => egui::Event::PointerGone,
        });
        if held {
            raw.events.push(egui::Event::PointerButton {
                pos: pointer.unwrap_or(Pos2::ZERO),
                button: egui::PointerButton::Primary,
                pressed: true,
                modifiers: egui::Modifiers::NONE,
            });
        }
        ctx.run_ui(raw, |_| {}).drop_without_applying_deltas();
    }

    /// 已经吸在右边、当前是收起态的 app：走真实的吸附路径落位，锚点与闸门都由 `apply_snap` 置好
    fn docked_app(dir: &std::path::Path, c: &egui::Context) -> IceApp {
        let mut app = app(dir, compact_state());
        app.apply_snap(
            c,
            Rect::from_min_size(Pos2::new(SHOWN_X + 30.0, 400.0), SIZE),
            Some(area()),
        );
        assert_eq!(app.state.config.ui.docked_edge, DockEdge::Right);
        assert!(!app.dock_shown, "吸附落位就是收起态");
        app
    }

    /// 看护一帧：注入固定桌面边界，真机上这里才是 `screen::desktop`
    /// （单屏时并集就等于那块屏的可见区，测试里两个数是同一组）
    fn watch(app: &mut IceApp, c: &egui::Context) {
        app.watch_dock(c, || Some(area()));
    }

    /// 简洁窗常驻置顶，用户往往正在别的应用里干活、顺手就来抓它——那一次窗口是**没有焦点**的，
    /// 而 egui-winit 的 `StartDrag` 被 `window.has_focus()` 挡着（`lib.rs:1961-1968`），
    /// 不先补一条 `Focus` 就表现为"把手拖不动"（走查里报的第一条缺陷）。
    #[test]
    fn an_unfocused_window_gets_focus_before_the_native_drag() {
        let dir = tempfile::tempdir().unwrap();
        let mut app = app(dir.path(), compact_state());
        let c = ctx();
        feed_geometry(&c, Some(Rect::from_min_size(Pos2::ZERO, SIZE)), Some(false));

        let effects = Effects {
            start_drag: true,
            ..Default::default()
        };
        app.run_effects(&c, effects);

        let commands = queued(&c);
        let focus = commands
            .iter()
            .position(|cmd| *cmd == ViewportCommand::Focus)
            .expect("未聚焦时必须先补 Focus，否则 StartDrag 会被平台丢掉");
        let drag = commands
            .iter()
            .position(|cmd| *cmd == ViewportCommand::StartDrag)
            .expect("把手按下要真的把拖拽交给原生");
        assert!(focus < drag, "Focus 必须排在 StartDrag 之前");
        assert!(app.drag_settler.is_armed(), "松手判定要从这一刻开始计时");
    }

    /// Q-013 之后窗口带原生标题栏，用户更多时候抓的是它——那条路径不经过把手、
    /// 也就没人发 `StartDrag`，应用侧只看得到几何在动。吸附必须照样跑。
    #[test]
    fn a_native_titlebar_drag_arms_the_settler_and_snaps() {
        let dir = tempfile::tempdir().unwrap();
        let mut app = app(dir.path(), compact_state());
        let c = ctx();
        let free = Rect::from_min_size(Pos2::new(600.0, 400.0), SIZE);
        for _ in 0..=STILL_BEFORE_DRAG {
            feed_geometry(&c, Some(free), Some(true));
            app.settle_drag(&c, || Some(area()));
        }
        assert!(!app.drag_settler.is_armed(), "窗口停着不算拖动");

        // 推到离右边缘 10px 处（阈值 40）：没有 pointer 事件、没有 StartDrag，只有位置变了
        let near_edge = Rect::from_min_size(Pos2::new(area().max.x - SIZE.x - 10.0, 400.0), SIZE);
        feed_geometry(&c, Some(near_edge), Some(true));
        app.settle_drag(&c, || Some(area()));
        assert!(
            app.drag_settler.is_armed(),
            "原生标题栏的拖动没人 arm，吸附与停靠就一次都不会跑"
        );

        std::thread::sleep(mode::DRAG_SETTLE + std::time::Duration::from_millis(30));
        feed_geometry(&c, Some(near_edge), Some(true));
        app.settle_drag(&c, || Some(area()));
        assert_eq!(app.state.config.ui.docked_edge, DockEdge::Right);
    }

    /// 反过来：我们自己下发的那次移动不能当成用户拖动。`show_dock` 把窗口挪到贴着边缘的
    /// 唤出位，若那被认成拖动，150ms 后的 `apply_snap` 会看到"离右边不到阈值"再 `dock` 一次，
    /// 刚弹出来又缩回去——唤出看起来失效。
    #[test]
    fn a_move_we_asked_for_ourselves_does_not_arm_the_settler() {
        let dir = tempfile::tempdir().unwrap();
        let mut app = app(dir.path(), compact_state());
        let c = ctx();
        let free = Rect::from_min_size(Pos2::new(600.0, 400.0), SIZE);
        for _ in 0..=STILL_BEFORE_DRAG {
            feed_geometry(&c, Some(free), Some(true));
            app.settle_drag(&c, || Some(area()));
        }

        let shown = Rect::from_min_size(Pos2::new(SHOWN_X, 400.0), SIZE);
        app.place_compact(&c, shown.min, DockEdge::Right);
        feed_geometry(&c, Some(shown), Some(true));
        app.settle_drag(&c, || Some(area()));
        assert!(
            !app.drag_settler.is_armed(),
            "自己要求的移动到了位，不该被当成拖动"
        );
        assert!(
            app.commanded.is_none(),
            "到达目标即清空，下一次变动就是用户拖的"
        );

        // 停稳之后再动一次：这回必须 arm，否则用户真的拖标题栏时又没吸附了
        for _ in 0..STILL_BEFORE_DRAG {
            feed_geometry(&c, Some(shown), Some(true));
            app.settle_drag(&c, || Some(area()));
        }
        let dragged = Rect::from_min_size(Pos2::new(SHOWN_X - 120.0, 400.0), SIZE);
        feed_geometry(&c, Some(dragged), Some(true));
        app.settle_drag(&c, || Some(area()));
        assert!(
            app.drag_settler.is_armed(),
            "门禁放开之后要立刻认出下一次拖动"
        );
    }

    #[test]
    fn a_focused_window_does_not_get_a_redundant_focus_command() {
        let dir = tempfile::tempdir().unwrap();
        let mut app = app(dir.path(), compact_state());
        let c = ctx();
        feed_geometry(&c, Some(Rect::from_min_size(Pos2::ZERO, SIZE)), Some(true));

        let effects = Effects {
            start_drag: true,
            ..Default::default()
        };
        app.run_effects(&c, effects);

        let commands = queued(&c);
        assert!(commands.contains(&ViewportCommand::StartDrag));
        assert!(!commands.contains(&ViewportCommand::Focus));
    }

    /// 松手静止后按注入的可用区吸附，并把结果立即写进配置与磁盘（REQ-003 + REQ-009）
    #[test]
    fn a_settled_drag_snaps_to_the_nearest_edge_and_persists() {
        let dir = tempfile::tempdir().unwrap();
        let mut app = app(dir.path(), compact_state());
        let c = ctx();
        let settled = Rect::from_min_size(Pos2::new(1440.0 - 30.0 - 460.0, 400.0), SIZE);

        app.apply_snap(&c, settled, Some(area()));

        let expected = TUCKED_X;
        assert_eq!(app.state.config.ui.docked_edge, DockEdge::Right);
        assert_eq!(app.state.config.ui.compact_rect.x, expected);
        assert!(queued(&c).contains(&ViewportCommand::OuterPosition(Pos2::new(expected, 400.0))));
        assert!(!app.state.config_dirty, "吸附完要立即落盘，不留脏标记");
        let saved = Config::load_at(&dir.path().join("config.toml"))
            .expect("配置要能读回来")
            .0;
        assert_eq!(
            saved.ui.compact_rect.x, expected,
            "盘上的几何要是吸附后的那个"
        );
        assert_eq!(saved.ui.docked_edge, DockEdge::Right);
    }

    /// 已经停在目标位置上：一个命令都不发、一次盘都不写。
    /// 否则程序自己挪的那一次会被下一轮静止判定当成"用户又拖了"，每 150ms 无限重算。
    #[test]
    fn snapping_again_from_the_docked_position_is_a_no_op() {
        let dir = tempfile::tempdir().unwrap();
        let mut state = compact_state();
        state.config.ui.docked_edge = DockEdge::Right;
        state.config.ui.compact_rect.x = TUCKED_X;
        state.config.ui.compact_rect.y = 400.0;
        let mut app = app(dir.path(), state);
        let c = ctx();
        let settled = Rect::from_min_size(Pos2::new(TUCKED_X, 400.0), SIZE);

        app.apply_snap(&c, settled, Some(area()));

        assert!(queued(&c).is_empty(), "已经在那儿了，不该再发几何命令");
        assert_eq!(app.state.config.ui.compact_rect.x, settled.min.x);
    }

    /// 缺陷 D-14 的第一道锁：拖动途中一条**尺寸**命令都不许发。
    ///
    /// 用户 2026-10-28："拖动时候变大，停下后又恢复原来的大小"，屏幕中间也这样，两种形态都
    /// 中招。`InnerSize` 只要出现在拖动中途，原生层就会把窗口拽回配置里的那个尺寸，观感正是
    /// "拖一下大一下、停下来又缩回去"。这里按帧喂移动中的外框（等价于 AppKit 自己搬窗口），
    /// 逐帧检查发出去的命令：尺寸、下限、装饰、可否缩放四种都不许碰，位置命令（吸附要用的
    /// `OuterPosition`）不在禁止之列。
    #[test]
    fn a_drag_never_asks_for_a_new_size_in_either_mode() {
        let cases = [
            (UiMode::Full, egui::vec2(1080.0, 720.0)),
            (UiMode::Compact, SIZE),
        ];
        for (mode, size) in cases {
            let dir = tempfile::tempdir().unwrap();
            let mut state = plain_state();
            state.config.ui.mode = mode;
            let mut app = app(dir.path(), state);
            let c = ctx();

            for step in 0..8 {
                let outer = Rect::from_min_size(Pos2::new(300.0 + step as f32 * 60.0, 300.0), size);
                feed_geometry(&c, Some(outer), Some(true));
                app.refresh_geometry(&c);
                app.settle_drag(&c, || Some(area()));
                for command in queued(&c) {
                    assert!(
                        !matches!(
                            command,
                            ViewportCommand::InnerSize(_)
                                | ViewportCommand::MinInnerSize(_)
                                | ViewportCommand::Decorations(_)
                                | ViewportCommand::Resizable(_)
                        ),
                        "{mode:?} 拖动第 {step} 帧发了尺寸类命令：{command:?}"
                    );
                }
            }
        }
    }

    /// 缺陷 D-14 的第二道锁：系统自己改了窗口尺寸时，我们记下来，但不跟它抢。
    ///
    /// 拖动途中原生层可能改内尺寸（平铺、缩放动画、用户抓到了边）。这一帧之后
    /// `capture` 要把真实尺寸写回本槽——配置得跟着现实走；同时**一条 `InnerSize` 都不发**，
    /// 否则就成了"系统改大、我们拽回、系统再改大"的来回抖。
    #[test]
    fn a_size_the_system_changed_is_recorded_not_fought() {
        let dir = tempfile::tempdir().unwrap();
        let mut app = app(dir.path(), plain_state());
        let c = ctx();
        let normal = Rect::from_min_size(Pos2::new(300.0, 300.0), egui::vec2(1080.0, 720.0));
        feed_geometry(&c, Some(normal), Some(true));
        app.refresh_geometry(&c);

        let grown = Rect::from_min_size(Pos2::new(360.0, 300.0), egui::vec2(1400.0, 900.0));
        feed_geometry(&c, Some(grown), Some(true));
        app.refresh_geometry(&c);

        assert_eq!(
            (
                app.state.config.ui.full_rect.w,
                app.state.config.ui.full_rect.h
            ),
            (1400.0, 900.0),
            "原生层改过的尺寸要记进配置，不能留着旧值"
        );
        assert!(
            !queued(&c)
                .iter()
                .any(|cmd| matches!(cmd, ViewportCommand::InnerSize(_))),
            "系统刚改完尺寸我们就发一条拽回去，窗口就来回抖"
        );
    }

    /// NFR-2 的护栏：整窗 alpha 每帧都发一次系统调用，AppKit 就每帧重合成整个窗口。
    /// 悬浮窗绝大多数时间是"停在桌面上没人碰"，那一档不能变，所以同值必须跳过。
    /// 上次没走通（`None`）要重试——第一帧拿不到窗口句柄是正常时序，不是失败终局。
    #[test]
    fn an_unchanged_window_alpha_is_not_resent() {
        assert!(
            !needs_alpha_update(Some(0.85), 0.85),
            "同一档重复下发，空闲帧全在付重合成的钱"
        );
        assert!(
            !needs_alpha_update(Some(1.0), 1.0),
            "全量窗整场都是 100%，一次都不该发"
        );
        assert!(
            needs_alpha_update(Some(0.85), 1.0),
            "焦点回来了必须写回 100%，不然窗体永远停在半透明"
        );
        assert!(
            needs_alpha_update(None, 0.85),
            "从没成功下过（含首帧拿不到句柄）要重试，不能永久退回"
        );
    }

    /// TC-3.5-d（接线）：停靠后从 6px 热区向内拖过阈值 → 就地解除停靠，停在松手处并落盘
    #[test]
    fn dragging_a_docked_window_inward_releases_it_and_keeps_the_position() {
        let dir = tempfile::tempdir().unwrap();
        let mut app = app(dir.path(), compact_state());
        let c = ctx();
        app.apply_snap(
            &c,
            Rect::from_min_size(Pos2::new(1440.0 - 30.0 - 460.0, 400.0), SIZE),
            Some(area()),
        );
        let docked = TUCKED_X;
        assert_eq!(app.state.config.ui.docked_edge, DockEdge::Right);
        assert_eq!(app.dock_anchor, Some(Pos2::new(docked, 400.0)));

        let dragged = Rect::from_min_size(Pos2::new(docked - 200.0, 400.0), SIZE);
        app.apply_snap(&c, dragged, Some(area()));

        assert_eq!(
            app.state.config.ui.docked_edge,
            DockEdge::None,
            "向内拖过阈值要恢复自由浮动"
        );
        assert_eq!(
            app.state.config.ui.compact_rect.x, dragged.min.x,
            "停在松手处"
        );
        assert!(queued(&c).contains(&ViewportCommand::OuterPosition(dragged.min)));
        assert_eq!(app.dock_anchor, None, "解除停靠要把锚点一起清掉");
        assert!(
            !app.state.config_dirty,
            "解除停靠也算一次几何定案，立即落盘"
        );
    }

    /// 拖离的参照必须是"停靠那一刻下发的原点"，不能是 `compact_rect`：
    /// `mode::capture` 每帧把实时几何写进 `compact_rect`，拖动途中它就等于窗口当前位置，
    /// 拿它当锚点位移恒为 0，四条边谁都拖不离（v0.3 的左/上就是这样）。
    #[test]
    fn the_release_reads_the_anchor_even_after_capture_moved_the_config_rect() {
        let dir = tempfile::tempdir().unwrap();
        let mut app = app(dir.path(), compact_state());
        let c = ctx();
        let docked = TUCKED_X;
        app.apply_snap(
            &c,
            Rect::from_min_size(Pos2::new(1440.0 - 30.0 - 460.0, 400.0), SIZE),
            Some(area()),
        );

        // 拖动途中的一帧：真实几何是"已经往里走了 100px"，capture 把它写进配置
        let in_flight = Rect::from_min_size(Pos2::new(docked - 100.0, 400.0), SIZE);
        feed_geometry(&c, Some(in_flight), Some(true));
        mode::capture(&c, &mut app.state, &mut app.pending_geometry);
        assert_eq!(
            app.state.config.ui.compact_rect.x, in_flight.min.x,
            "前提：capture 确实把 compact_rect 冲成了途中位置"
        );
        assert_eq!(app.dock_anchor, Some(Pos2::new(docked, 400.0)), "锚点不动");

        // 静止在"向内 200px"处：仍要判出拖离
        let dragged = Rect::from_min_size(Pos2::new(docked - 200.0, 400.0), SIZE);
        app.apply_snap(&c, dragged, Some(area()));
        assert_eq!(
            app.state.config.ui.docked_edge,
            DockEdge::None,
            "锚点被 capture 冲掉也不许影响拖离判定"
        );
    }

    /// 沿交叉轴拖 200px 是"贴着边滑动"，不是拖离：停靠态保留，一个命令都不发
    #[test]
    fn sliding_along_the_docked_edge_keeps_the_window_docked() {
        let dir = tempfile::tempdir().unwrap();
        let mut app = app(dir.path(), compact_state());
        let c = ctx();
        let docked = TUCKED_X;
        app.apply_snap(
            &c,
            Rect::from_min_size(Pos2::new(1440.0 - 30.0 - 460.0, 400.0), SIZE),
            Some(area()),
        );

        let slid = Rect::from_min_size(Pos2::new(docked, 600.0), SIZE);
        app.apply_snap(&c, slid, Some(area()));

        assert_eq!(
            app.state.config.ui.docked_edge,
            DockEdge::Right,
            "仍然停在右边"
        );
        assert!(
            !queued(&c).contains(&ViewportCommand::OuterPosition(slid.min)),
            "吸附结果就是松手位置，不必再挪"
        );
        assert_eq!(app.dock_anchor.unwrap().x, docked, "released 只看停靠轴");
    }

    /// 全量窗有系统标题栏，不参与吸附：静止之后只落盘
    #[test]
    fn a_full_window_never_snaps() {
        let dir = tempfile::tempdir().unwrap();
        let mut app = app(dir.path(), plain_state());
        let c = ctx();
        let settled = Rect::from_min_size(Pos2::new(1440.0 - 30.0 - 460.0, 400.0), SIZE);

        app.apply_snap(&c, settled, None);

        assert!(queued(&c).is_empty());
        assert_eq!(app.state.config.ui.docked_edge, DockEdge::None);
    }

    /// 拖到屏幕外松手（外接屏拔掉、休眠唤醒后的旧坐标）：夹回可用区，不许把窗口弄丢
    #[test]
    fn a_window_released_off_screen_is_pulled_back_into_the_work_area() {
        let dir = tempfile::tempdir().unwrap();
        let mut app = app(dir.path(), compact_state());
        let c = ctx();
        let lost = Rect::from_min_size(Pos2::new(5000.0, 4000.0), SIZE);

        app.apply_snap(&c, lost, Some(area()));

        let rect = app.state.config.ui.compact_rect;
        assert!(
            rect.x >= 0.0 && rect.y >= 25.0,
            "夹回后不该有屏幕外坐标：{rect:?}"
        );
        assert!(
            rect.x + rect.w <= 1440.0 && rect.y + rect.h <= 830.0,
            "整块都要回到可用区里：{rect:?}"
        );
    }

    /// REQ-009 的启动支：盘上记的坐标整块落在可用区外（外接屏拔掉、分辨率变了）→
    /// 第一帧就夹回来并立即落盘；判一次就够，第二帧不再去问屏幕。
    #[test]
    fn a_window_saved_off_screen_is_pulled_back_on_the_first_frame() {
        let dir = tempfile::tempdir().unwrap();
        let mut state = compact_state();
        state.config.ui.compact_rect.x = 5000.0;
        state.config.ui.compact_rect.y = 4000.0;
        let mut app = app(dir.path(), state);
        let c = ctx();
        let path = dir.path().join("config.toml");
        feed_geometry(
            &c,
            Some(Rect::from_min_size(Pos2::new(5000.0, 4000.0), SIZE)),
            Some(true),
        );

        app.rescue_offscreen(&c, |_| Some(area()));

        let rect = app.state.config.ui.compact_rect;
        assert!(
            rect.y >= 25.0
                && rect.x >= 0.0
                && rect.x + rect.w <= 1440.0
                && rect.y + rect.h <= 830.0,
            "夹回后整块都要在可用区里：{rect:?}"
        );
        assert!(
            queued(&c).contains(&ViewportCommand::OuterPosition(Pos2::new(rect.x, rect.y))),
            "夹回要真的下发给窗口，不只是改配置"
        );
        let (saved, _) = Config::load_at(&path).expect("夹回当帧就落盘，强杀也找得回来");
        assert_eq!(saved.ui.compact_rect, rect);

        let probes = std::cell::Cell::new(0usize);
        app.rescue_offscreen(&c, |_| {
            probes.set(probes.get() + 1);
            Some(area())
        });
        assert_eq!(
            probes.into_inner(),
            0,
            "`work_area` 要走 NSScreen，启动校验只跑一帧"
        );
    }

    /// 上一条的例外（REQ-009 v0.5 新增）：左停靠按设计就有 `w − sliver` 在区外（x 为负）。
    /// 拿整窗判据去夹它，等于每次启动都替用户解了停靠。
    #[test]
    fn a_docked_window_is_never_rescued_back_into_the_area() {
        let dir = tempfile::tempdir().unwrap();
        let c = ctx();
        let mut app = docked_app(dir.path(), &c);
        app.state.config.ui.docked_edge = DockEdge::Left;
        app.state.config.ui.compact_rect.x = -457.0;
        feed_geometry(
            &c,
            Some(Rect::from_min_size(Pos2::new(-457.0, 400.0), SIZE)),
            Some(true),
        );

        let probes = std::cell::Cell::new(0usize);
        app.rescue_offscreen(&c, |_| {
            probes.set(probes.get() + 1);
            Some(area())
        });

        assert_eq!(
            probes.into_inner(),
            0,
            "停靠态交给 `land_dock` 重落位，不跑整窗判据"
        );
        assert_eq!(
            app.state.config.ui.compact_rect.x, -457.0,
            "不该被拉回可用区"
        );
    }

    /// REQ-004：切换下发的是一组**异步请求**，落定的那一帧才把真实几何写盘并立即存。
    /// 落定之前不能把目标值当已生效值写（D-6），也不该抢着落盘。
    #[test]
    fn the_switched_geometry_is_saved_as_soon_as_it_lands() {
        let dir = tempfile::tempdir().unwrap();
        let mut app = app(dir.path(), compact_state());
        let c = ctx();
        let path = dir.path().join("config.toml");

        mode::apply(
            &c,
            &mut app.state,
            UiMode::Compact,
            &mut app.pending_geometry,
        );
        assert!(app.pending_geometry.is_pending());

        // 切换后的第一帧：窗口还是全量的大数据
        feed_geometry(
            &c,
            Some(Rect::from_min_size(
                Pos2::new(300.0, 200.0),
                egui::Vec2::new(642.0, 422.0),
            )),
            Some(true),
        );
        app.refresh_geometry(&c);
        let rect = app.state.config.ui.compact_rect;
        assert_eq!(rect.w, 460.0, "护栏内不把过渡尺寸写进槽");
        assert_eq!(rect.x, 300.0, "位置轴照写，用户可能同一瞬间在拖窗");
        assert!(!path.exists(), "还没落定，不该抢着落盘");

        // 窗口跟上了目标尺寸
        feed_geometry(
            &c,
            Some(Rect::from_min_size(Pos2::new(300.0, 200.0), SIZE)),
            Some(true),
        );
        app.refresh_geometry(&c);

        assert!(!app.pending_geometry.is_pending(), "命中目标尺寸即解除护栏");
        let (saved, _) = Config::load_at(&path).expect("落定的那一帧立即落盘");
        assert_eq!(saved.ui.mode, UiMode::Compact);
        assert_eq!(saved.ui.compact_rect, rect);
    }

    /// D-10 的回归锁：一次"始终没落定"的切换结束后，两个尺寸槽不许变得一字不差。
    ///
    /// 走查时盘上的坏形状是 `full_rect` 与 `compact_rect` 四项全等（984×632）——护栏
    /// "到期即放行"把大窗尺寸抄进了简洁槽，之后 `measured == target` 每帧命中，坏状态锁死。
    /// 现在第二段到期改为不采纳：槽里的尺寸保持切换前的值，只有位置照写。
    #[test]
    fn the_two_slots_never_end_up_equal_after_a_failed_switch() {
        let dir = tempfile::tempdir().unwrap();
        let mut app = app(dir.path(), plain_state());
        let c = ctx();
        let big = Rect::from_min_size(Pos2::new(-1758.0, 380.0), egui::Vec2::new(984.0, 632.0));

        // 全量窗停在 984×632，然后切到简洁——原生始终不缩小
        feed_geometry(&c, Some(big), Some(true));
        app.refresh_geometry(&c);
        mode::apply(
            &c,
            &mut app.state,
            UiMode::Compact,
            &mut app.pending_geometry,
        );
        assert!(app.pending_geometry.is_pending());

        feed_geometry(&c, Some(big), Some(true));
        app.refresh_geometry(&c);
        let after_first_frame = app.state.config.ui.compact_rect;
        assert_eq!(
            (after_first_frame.w, after_first_frame.h),
            (460.0, 160.0),
            "宽限期内的过渡尺寸不进槽"
        );

        // 直接推到"重发已交、第二轮也到期"，再喂同一帧大尺寸
        app.pending_geometry
            .set_stage_clock_for_tests(Instant::now() - mode::MODE_GEOMETRY_GRACE * 2, true);
        app.refresh_geometry(&c);
        assert!(!app.pending_geometry.is_pending(), "第二段到期后护栏收工");
        let compact = app.state.config.ui.compact_rect;
        let full = app.state.config.ui.full_rect;
        assert_eq!((compact.w, compact.h), (460.0, 160.0), "实测大尺寸没被采纳");
        assert_eq!((full.w, full.h), (984.0, 632.0), "全量槽照实记自己的尺寸");
        assert_ne!(compact, full, "两槽不许变得一字不差——那正是 D-10 的指纹");

        // 之后窗口一直停在大尺寸：尺寸轴保持静默
        for _ in 0..3 {
            feed_geometry(&c, Some(big), Some(true));
            app.refresh_geometry(&c);
        }
        assert_eq!(
            app.state.config.ui.compact_rect, compact,
            "静默基线期间不再有写回"
        );
    }

    /// 第一段到期的重发（REQ-004 v0.7）：帧循环要真的把第二组命令发出去，而且只发一次。
    ///
    /// 判据写成"正常落定各恰好 1 条、没落定的路径至多 2 条且第二组出现在宽限期之后"，
    /// 与 spec 那条一改二的断言同一形状。
    #[test]
    fn a_first_stage_expiry_resends_the_size_and_position_once() {
        let dir = tempfile::tempdir().unwrap();
        let mut app = app(dir.path(), plain_state());
        let c = ctx();
        let big = Rect::from_min_size(Pos2::new(300.0, 200.0), egui::Vec2::new(984.0, 632.0));
        let target = mode::target_size(&app.state.config.ui, UiMode::Compact);

        mode::apply(
            &c,
            &mut app.state,
            UiMode::Compact,
            &mut app.pending_geometry,
        );
        let after_switch = queued(&c);
        assert_eq!(
            size_cmds(&after_switch, target),
            1,
            "切换那一组里尺寸命令恰好一条"
        );

        feed_geometry(&c, Some(big), Some(true));
        app.refresh_geometry(&c);
        assert_eq!(
            size_cmds(&queued(&c), target),
            0,
            "宽限期没到不该发第二组（`feed_geometry` 会像原生层那样取走上一帧的命令，这里数的是当帧新发的）"
        );

        app.pending_geometry
            .set_stage_clock_for_tests(Instant::now() - mode::MODE_GEOMETRY_GRACE * 2, false);
        app.refresh_geometry(&c);
        let after_retry = queued(&c);
        assert_eq!(
            size_cmds(&after_retry, target),
            1,
            "第一段到期：重发一条尺寸"
        );
        assert_eq!(
            after_retry.len(),
            2,
            "重发只补尺寸与位置两条，不整套重发：{after_retry:?}"
        );

        // 第二轮里再喂几帧，不许逐帧重发
        for _ in 0..3 {
            feed_geometry(&c, Some(big), Some(true));
            app.refresh_geometry(&c);
        }
        assert_eq!(size_cmds(&queued(&c), target), 0, "一轮宽限期只重发一次");
    }

    /// 尺寸命令计数（REQ-004 的"各恰好 1 条"要按命令种类数，不是按整套数）
    fn size_cmds(cmds: &[ViewportCommand], target: egui::Vec2) -> usize {
        cmds.iter()
            .filter(|cmd| **cmd == ViewportCommand::InnerSize(target))
            .count()
    }

    /// REQ-009 的节流：拖动结束后由 `capture` 自己续一帧重绘（那口钟在无头环境观测不到），
    /// 这里断言它的后果——脏掉的几何最迟在一个节流窗口后出现在盘上，不等 `on_exit`。
    #[test]
    fn a_geometry_change_is_written_within_the_throttle_window() {
        let dir = tempfile::tempdir().unwrap();
        let mut app = app(dir.path(), compact_state());
        let c = ctx();
        let path = dir.path().join("config.toml");

        feed_geometry(
            &c,
            Some(Rect::from_min_size(Pos2::new(120.0, 300.0), SIZE)),
            Some(true),
        );
        mode::capture(&c, &mut app.state, &mut app.pending_geometry);
        assert!(app.state.config_dirty, "几何一变就该标脏");

        app.flush_config(false);
        assert!(!path.exists(), "刚存过，节流窗口内不重复写盘");

        std::thread::sleep(mode::CONFIG_SAVE_INTERVAL + Duration::from_millis(20));
        app.flush_config(false);

        let (saved, _) = Config::load_at(&path).expect("节流窗口一过就该落盘");
        assert_eq!(saved.ui.compact_rect.x, 120.0);
        assert!(!app.state.config_dirty);
    }

    /// REQ-004 的边界：窗口收起时按 `Cmd+Shift+M` 只改配置里的目标形态——
    /// 一条几何命令都不发，也不该把窗口叫回来。
    #[test]
    fn a_switch_while_hidden_only_updates_the_config() {
        let dir = tempfile::tempdir().unwrap();
        let mut app = app(dir.path(), compact_state());
        let c = ctx();
        app.state.hidden = true;
        let mut effects = Effects::default();

        app.run(Action::ToggleMode, &c, &mut effects);
        app.run_effects(&c, effects);

        assert_eq!(
            app.state.mode(),
            UiMode::Full,
            "目标形态要记下，Dock 叫回时按它设几何"
        );
        assert!(app.state.config_dirty);
        assert!(app.state.hidden, "切换不是唤起窗口");
        assert!(
            queued(&c).is_empty(),
            "隐藏期一条几何命令都不发：{:?}",
            queued(&c)
        );
    }

    /// 变更 G（用户 2026-10-28："将标题栏最大化功能替换为切换全量"）：原生绿键的点击
    /// 必须落到与 `Cmd+Shift+M` **同一个** `effects` 字段上，而且一次点击只兑现一次。
    ///
    /// 无头拿不到 `NSWindow`，所以这条测的是"点击 → 切换"这段接线（`poll_titlebar_click`），
    /// 不是 AppKit 那一头（`sync` 走没走到绿键只能看 `/tmp/ice.log` 的那行 debug）。
    #[test]
    fn a_title_bar_click_toggles_through_the_same_effect_as_the_shortcut() {
        let dir = tempfile::tempdir().unwrap();
        let mut app = app(dir.path(), plain_state());

        titlebar::mark_click();
        let mut effects = Effects::default();
        app.poll_titlebar_click(&mut effects);
        assert!(
            effects.toggle_mode,
            "绿键的点击没接进 effects，标题栏那颗按钮点了没反应"
        );

        let mut second = Effects::default();
        app.poll_titlebar_click(&mut second);
        assert!(!second.toggle_mode, "同一次点击被兑了两次，形态会被切回去");

        let mut via_key = Effects::default();
        app.run(Action::ToggleMode, &ctx(), &mut via_key);
        assert_eq!(
            via_key.toggle_mode, effects.toggle_mode,
            "快捷键与绿键不是同一条路径"
        );
    }

    /// REQ-003：光标推到屏幕边上那条热区 → 整条唤出，锚点跟到唤出位，停靠边不动。
    /// 吸附落位时闸门是关着的，所以先用一帧"指针在窗口外"把它打开——这也证明开闸不需要挪窗口。
    #[test]
    fn hovering_the_strip_reveals_the_docked_window() {
        let dir = tempfile::tempdir().unwrap();
        let c = ctx();
        let mut app = docked_app(dir.path(), &c);
        let tucked = Rect::from_min_size(Pos2::new(TUCKED_X, 400.0), SIZE);

        feed_pointer(&c, tucked, None, false);
        watch(&mut app, &c);
        assert!(!app.dock_latched, "指针离开创口带就该开闸");
        assert!(queued(&c).is_empty(), "开闸那一帧什么都不做");

        feed_pointer(&c, tucked, Some(STRIP), false);
        watch(&mut app, &c);

        let commands = queued(&c);
        assert!(
            commands.contains(&ViewportCommand::OuterPosition(Pos2::new(SHOWN_X, 400.0))),
            "唤出要整条回到屏内：{commands:?}"
        );
        assert!(app.dock_shown);
        assert_eq!(app.dock_anchor, Some(Pos2::new(SHOWN_X, 400.0)));
        assert_eq!(
            app.state.config.ui.docked_edge,
            DockEdge::Right,
            "唤出只改运行时状态，不改停靠边"
        );
    }

    /// D-8 的接线侧：看护帧落位用的是注入进来的**整个桌面**边界，不是窗口所在那块屏。
    /// 单屏时两个数值相同，所以这条专门给一块"比屏大"的边界——窗口停在两块屏的接缝上，
    /// 按所在屏算的右外缘就是那条缝（收到 x≈−3：既盖不住，`released` 又永远量不出向内位移、
    /// 停靠解不开）；按桌面算才拿得到真正的外缘 1917。
    #[test]
    fn the_dock_watcher_lands_on_the_desktop_edge_not_the_screen_edge() {
        let dir = tempfile::tempdir().unwrap();
        let c = ctx();
        let mut app = docked_app(dir.path(), &c);
        app.dock_anchor = None;
        let at_seam = Rect::from_min_size(Pos2::new(-230.0, 400.0), SIZE);
        feed_geometry(&c, Some(at_seam), Some(true));

        let desktop = Rect::from_min_max(Pos2::new(-1920.0, 25.0), Pos2::new(1920.0, 1016.0));
        app.watch_dock(&c, || Some(desktop));

        assert_eq!(
            app.state.config.ui.compact_rect.x,
            desktop.max.x - SLIVER,
            "收起到桌面右外缘，不是接缝"
        );
        assert_eq!(app.dock_anchor, Some(Pos2::new(1917.0, 400.0)));
    }

    /// Q-012：停靠态那 3px 必须收得到鼠标事件，所以层级强制浮在其他应用之上——
    /// 尤其要覆盖**用户关掉置顶**的那一种（走查那次正是 `always_on_top = false`，
    /// 藏出去的窗口被别的 app 压住，用户"再也找不到"）。
    /// egui 0.37 没有 `WindowLevel::Floating`，`AlwaysOnTop` 在 macOS 上正是 AppKit 的 `.floating`。
    #[test]
    fn docking_lifts_the_strip_even_with_the_topmost_switch_off() {
        let dir = tempfile::tempdir().unwrap();
        let mut state = compact_state();
        state.config.ui.always_on_top = false;
        let mut app = app(dir.path(), state);
        let c = ctx();

        app.apply_snap(
            &c,
            Rect::from_min_size(Pos2::new(SHOWN_X + 30.0, 400.0), SIZE),
            Some(area()),
        );

        assert!(
            queued(&c).contains(&ViewportCommand::WindowLevel(
                egui::WindowLevel::AlwaysOnTop
            )),
            "吸附落位那帧就要把窗口浮起来，否则热区接不到鼠标"
        );
        assert!(
            !app.state.config.ui.always_on_top,
            "置顶偏好不因停靠被改写，指示仍如实显示"
        );
    }

    /// 唤出后指针离开满 `DOCK_AUTO_HIDE` → 收回；收回时指针恰好还压在 3px 上，
    /// 闸门必须挡住下一次唤出，否则就是"弹出来→收回去"来回抖（TC-3.5-g）
    #[test]
    fn it_retracts_after_the_pointer_leaves_and_does_not_bounce_back() {
        let dir = tempfile::tempdir().unwrap();
        let c = ctx();
        let mut app = docked_app(dir.path(), &c);
        let tucked = Rect::from_min_size(Pos2::new(TUCKED_X, 400.0), SIZE);
        let shown = Rect::from_min_size(Pos2::new(SHOWN_X, 400.0), SIZE);
        app.dock_latched = false;
        app.dock_shown = true;
        app.dock_anchor = Some(shown.min);

        // 还没到点：不许收
        app.dock_left_at = Some(Instant::now());
        feed_pointer(&c, shown, None, false);
        watch(&mut app, &c);
        assert!(app.dock_shown, "计时没到就收会把用户正在看的译文抢走");
        assert!(queued(&c).is_empty());

        // 到点：收回，并关上闸门
        app.dock_left_at = Some(Instant::now() - mode::DOCK_AUTO_HIDE);
        feed_pointer(&c, shown, None, false);
        watch(&mut app, &c);
        assert!(
            queued(&c).contains(&ViewportCommand::OuterPosition(Pos2::new(TUCKED_X, 400.0))),
            "收回要发收起位的命令"
        );
        assert!(!app.dock_shown);
        assert!(app.dock_latched, "收回要关闸");

        // 指针仍在那 3px 上：一帧都不该再动
        feed_pointer(&c, tucked, Some(STRIP), false);
        watch(&mut app, &c);
        assert!(queued(&c).is_empty(), "闸门没开就唤出，看起来就是在边上抖");
        assert!(app.dock_latched, "还在创口带里，闸门不解");
    }

    /// 正在输入行里打字、或鼠标还按着（拖窗/选字拖出了界）时不收回：
    /// 那两种"指针离开"是操作的一部分，不是"不看我了"（REQ-003 的两条例外）
    #[test]
    fn typing_or_a_held_button_keeps_the_window_out() {
        let dir = tempfile::tempdir().unwrap();
        let c = ctx();
        let shown = Rect::from_min_size(Pos2::new(SHOWN_X, 400.0), SIZE);

        let mut app = docked_app(dir.path(), &c);
        app.dock_shown = true;
        app.dock_anchor = Some(shown.min);
        app.state.focus_in_source = true;
        feed_pointer(&c, shown, None, false);
        watch(&mut app, &c);
        assert!(app.dock_shown, "光标在输入行里时不许收回");
        assert!(app.dock_left_at.is_none(), "例外那一帧要重置计时，别攒着");

        let mut app = docked_app(dir.path(), &c);
        app.dock_shown = true;
        app.dock_anchor = Some(shown.min);
        feed_pointer(&c, shown, None, true);
        watch(&mut app, &c);
        assert!(app.dock_shown, "按住不放时不许收回");
        assert!(queued(&c).is_empty());
    }

    /// 带着停靠态启动：盘上存的可能是上次的**唤出位**（在唤出态被强杀），
    /// 照搬会让窗口以停靠之名整条挂着。锚点缺失 → 第一帧看护重算收起到位，且不唤出
    #[test]
    fn a_docked_start_lands_at_the_tucked_position_instead_of_the_saved_one() {
        let dir = tempfile::tempdir().unwrap();
        let mut state = compact_state();
        state.config.ui.docked_edge = DockEdge::Right;
        state.config.ui.compact_rect.x = SHOWN_X;
        state.config.ui.compact_rect.y = 400.0;
        let mut app = app(dir.path(), state);
        let c = ctx();
        assert_eq!(app.dock_anchor, None, "启动时锚点必须留空，交给落位那一步");

        feed_pointer(
            &c,
            Rect::from_min_size(Pos2::new(SHOWN_X, 400.0), SIZE),
            None,
            false,
        );
        watch(&mut app, &c);

        assert!(queued(&c).contains(&ViewportCommand::OuterPosition(Pos2::new(TUCKED_X, 400.0))));
        assert_eq!(
            app.state.config.ui.compact_rect.x, TUCKED_X,
            "落位要写回配置并落盘"
        );
        assert!(!app.dock_shown);
        assert!(app.dock_latched, "落位那一帧指针可能就压在边上，闸门要关上");
    }

    /// 落位是幂等的：已经在收起位时只补锚点，一个命令都不发（否则每次启动都白挪一次）
    #[test]
    fn landing_on_the_position_it_already_holds_sends_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let mut state = compact_state();
        state.config.ui.docked_edge = DockEdge::Right;
        state.config.ui.compact_rect.x = TUCKED_X;
        state.config.ui.compact_rect.y = 400.0;
        let mut app = app(dir.path(), state);
        let c = ctx();
        let tucked = Rect::from_min_size(Pos2::new(TUCKED_X, 400.0), SIZE);

        feed_pointer(&c, tucked, None, false);
        watch(&mut app, &c);

        assert!(queued(&c).is_empty(), "已经在那儿了，不该再发命令");
        assert_eq!(app.dock_anchor, Some(tucked.min));
    }

    /// 解除停靠（向内拖过阈值）之后：层级要还给用户的置顶开关，运行时状态全清。
    /// 这里置顶是关着的，所以恢复后必须是 `Normal`——否则窗口会一直压在所有应用上面，
    /// 而用户在开关上看到的是"未置顶"（Q-012 的另一半）
    #[test]
    fn releasing_the_dock_gives_the_level_back_to_the_topmost_switch() {
        let dir = tempfile::tempdir().unwrap();
        let mut state = compact_state();
        state.config.ui.always_on_top = false;
        let mut app = app(dir.path(), state);
        let c = ctx();
        app.apply_snap(
            &c,
            Rect::from_min_size(Pos2::new(SHOWN_X + 30.0, 400.0), SIZE),
            Some(area()),
        );
        assert!(queued(&c).contains(&ViewportCommand::WindowLevel(
            egui::WindowLevel::AlwaysOnTop
        )));
        app.dock_shown = true;

        let free = Rect::from_min_size(Pos2::new(TUCKED_X - 200.0, 400.0), SIZE);
        app.apply_snap(&c, free, Some(area()));
        assert_eq!(app.state.config.ui.docked_edge, DockEdge::None);
        assert!(
            queued(&c).contains(&ViewportCommand::WindowLevel(egui::WindowLevel::Normal)),
            "解除停靠要把层级交还给置顶开关（这里是关）"
        );

        feed_pointer(&c, free, Some(Pos2::new(100.0, 60.0)), false);
        watch(&mut app, &c);
        assert!(!app.dock_shown);
        assert_eq!(app.dock_anchor, None);
        assert!(!app.dock_latched);
    }

    /// 简洁窗里 Esc 清空输入行，并把旧译文与在飞任务一起作废
    #[test]
    fn escape_in_the_compact_window_clears_the_input_and_the_result() {
        let dir = tempfile::tempdir().unwrap();
        let mut app = app(dir.path(), compact_state());
        app.state.translation = Some(TranslationResult::cached("你好", EngineKind::Machine));
        let c = ctx();
        let mut effects = Effects::default();

        app.run(Action::ClearInput, &c, &mut effects);

        assert!(app.state.source.is_empty());
        assert!(app.state.translation.is_none(), "旧译文留着会误导");
        assert!(
            app.state.in_flight.is_none(),
            "在飞的任务要作废，否则清空后半秒译文又自己冒出来"
        );
        assert!(effects.job.is_none(), "清空不发请求");
    }

    /// 全量窗里 Esc 仍不绑定任何动作（与 P4 一致）
    #[test]
    fn escape_does_nothing_in_the_full_window() {
        let dir = tempfile::tempdir().unwrap();
        let mut app = app(dir.path(), plain_state());
        app.state.source = "别清我".into();
        let mut effects = Effects::default();
        app.run(Action::ClearInput, &ctx(), &mut effects);
        assert_eq!(app.state.source, "别清我", "全量窗里 Esc 不做任何事");
    }

    /// 简洁窗的"设置"：瞬时切回全量并直接落到设置页（REQ-005）。
    /// 不复用 `toggle_mode`：那是"切换"，在全量窗里点设置会反被切成简洁窗。
    #[test]
    fn the_compact_settings_button_switches_to_full_and_lands_on_settings() {
        let dir = tempfile::tempdir().unwrap();
        let mut app = app(dir.path(), compact_state());
        let c = ctx();

        let effects = Effects {
            open_settings: true,
            ..Default::default()
        };
        app.run_effects(&c, effects);

        assert_eq!(app.state.mode(), UiMode::Full);
        assert_eq!(app.state.tab, Tab::Settings);
        assert!(queued(&c).contains(&ViewportCommand::Decorations(true)));
        assert!(!app.drag_settler.is_armed());
    }

    /// 动作行的置顶按钮与 Cmd+Shift+P 共用同一条路径：写回配置 + 下发 WindowLevel + 提示
    #[test]
    fn the_topmost_button_and_the_hotkey_share_one_path() {
        let dir = tempfile::tempdir().unwrap();
        let mut app = app(dir.path(), compact_state());
        let c = ctx();

        let effects = Effects {
            toggle_topmost: true,
            ..Default::default()
        };
        app.run_effects(&c, effects);

        assert!(app.state.config.ui.always_on_top);
        assert!(queued(&c).contains(&ViewportCommand::WindowLevel(
            egui::WindowLevel::AlwaysOnTop
        )));
        assert_eq!(app.state.toasts.len(), 1);

        let mut effects = Effects::default();
        app.run(Action::ToggleTopmost, &c, &mut effects);
        assert!(
            !app.state.config.ui.always_on_top,
            "快捷键那条路径同样能切回来"
        );
    }

    /// REQ-011 落点②的接线：设置页那颗按钮置的意图，到这里必须真的把两个槽、停靠状态、
    /// 盘上的文件和待生效几何一起收拾干净。D-10 走查时盘上那两个槽一字不差（984×632），
    /// 这就是用户唯一能自救的出口，所以断言按"恢复之后还找不回默认值就是没恢复"来写。
    #[test]
    fn the_reset_button_restores_both_slots_and_flushes_them() {
        let dir = tempfile::tempdir().unwrap();
        let mut state = plain_state();
        let broken = WindowRect {
            x: 300.0,
            y: 200.0,
            w: 984.0,
            h: 632.0,
        };
        state.config.ui.full_rect = broken;
        state.config.ui.compact_rect = broken;
        state.config.ui.docked_edge = DockEdge::Left;
        let mut app = app(dir.path(), state);
        app.dock_anchor = Some(Pos2::new(-981.0, 200.0));
        let c = ctx();
        let defaults = UiConfig::default();

        let effects = Effects {
            reset_geometry: true,
            ..Default::default()
        };
        app.run_effects(&c, effects);

        let ui = &app.state.config.ui;
        assert_eq!(
            (ui.full_rect.w, ui.full_rect.h),
            (defaults.full_rect.w, defaults.full_rect.h),
            "全量槽没回到 1080×720"
        );
        assert_eq!(
            (ui.compact_rect.w, ui.compact_rect.h),
            (defaults.compact_rect.w, defaults.compact_rect.h),
            "简洁槽没回到 460×160"
        );
        assert_ne!(
            ui.full_rect, ui.compact_rect,
            "按完还是 D-10 那个指纹，等于白按"
        );
        assert_eq!(ui.docked_edge, DockEdge::None, "收位是按旧尺寸算的");
        assert!(
            app.dock_anchor.is_none(),
            "旧锚点留着下一帧会把窗口顶出屏幕"
        );

        // 立即落盘：这一步之后强杀也不该回到坏值
        let (saved, _) =
            Config::load_at(&dir.path().join("config.toml")).expect("恢复当帧就落盘，不等节流窗口");
        assert_eq!(saved.ui.full_rect, ui.full_rect);
        assert_eq!(saved.ui.compact_rect, ui.compact_rect);

        let target = mode::target_size(&app.state.config.ui, app.state.mode());
        assert_eq!(
            size_cmds(&queued(&c), target),
            1,
            "按当前形态下发了目标尺寸"
        );
        assert!(
            app.pending_geometry.is_pending(),
            "恢复后的尺寸同样要走护栏，否则落定前的实测值会把它冲掉"
        );
        assert_eq!(app.state.toasts.len(), 1, "{}", app.state.toasts[0].text);
    }

    /// 居中那条规则两头都要断言：无头测试与探针失败帧里根本没有可用区，
    /// 这时挪位置比不挪更糟（用户下一次连窗口都找不到）。
    #[test]
    fn the_restored_size_is_centered_when_the_area_is_known() {
        let defaults = UiConfig::default();
        let keep = WindowRect {
            x: 300.0,
            y: 200.0,
            w: 984.0,
            h: 632.0,
        };
        let area = WindowRect {
            x: 0.0,
            y: 25.0,
            w: 1440.0,
            h: 805.0,
        };

        let restored = restored_rect(defaults.compact_rect, keep, Some(area));
        assert_eq!(
            (restored.w, restored.h),
            (defaults.compact_rect.w, defaults.compact_rect.h)
        );
        let expected =
            WindowRect::centered_in(&area, defaults.compact_rect.w, defaults.compact_rect.h);
        assert_eq!(
            (restored.x, restored.y),
            (expected.x, expected.y),
            "拿得到可用区就居中在它里面"
        );

        let headless = restored_rect(defaults.compact_rect, keep, None);
        assert_eq!(
            (headless.x, headless.y),
            (300.0, 200.0),
            "拿不到可用区时位置留在原处，只有尺寸换回默认"
        );
        assert_eq!((headless.w, headless.h), (460.0, 160.0));
    }
}
