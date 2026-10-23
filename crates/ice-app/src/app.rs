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
use std::time::{Duration, Instant};

use eframe::egui::{self, Pos2, Rect, Vec2, ViewportCommand};
use ice_core::config::{Config, DockEdge, UiMode};
use ice_core::history::HistoryStore;
use ice_providers::dispatcher::Translator;
use ice_providers::{AiEngine, MachineTranslateEngine};
use tokio::runtime::Runtime;
use tokio::sync::mpsc;

use crate::jobs::{self, JobRunner, OutcomeReceiver};
use crate::mode;
use crate::state::{self, AppState, KeyState, Tab, Tone};
use crate::system::clipboard::{self, CopyKind};
use crate::system::hotkeys::{self, Action};
use crate::system::screen;
use crate::system::secrets;
use crate::system::single_instance::InstanceLock;
use crate::theme;
use crate::views::{self, Effects};

/// 配置落盘的最短间隔。
///
/// 拖动窗口时 `mode::capture` 每帧都会把配置标脏，逐帧原子写盘没有意义；
/// 半秒去抖一次，退出与 Cmd+Q 走 `force`，不会丢改动。
const CONFIG_SAVE_INTERVAL: Duration = Duration::from_millis(500);

/// 后台钥匙串探测的结果回收端（容量 1：一次启动只探一次）
pub type KeyProbeReceiver = mpsc::Receiver<Result<Option<String>, KeyState>>;

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
    /// 停靠那一刻下发的窗口原点，拖离判定的锚点（REQ-003 第三条）。
    ///
    /// 不能取 `config.ui.compact_rect`：`mode::capture` 每帧按实时几何改写它，
    /// 拖动途中锚点就被冲掉、位移恒为 0，于是谁都拖不离。
    dock_anchor: Option<Pos2>,
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
        // 启动时就带着停靠态：锚点取上次落盘的原点——`place_compact` 停靠那次是强制落盘，
        // 写进去的正是停靠后的几何，两者相等（拖动途中被杀的那次例外，无所谓）。
        let dock_anchor = (state.config.ui.docked_edge != DockEdge::None).then(|| {
            let rect = state.config.ui.compact_rect;
            Pos2::new(rect.x, rect.y)
        });
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
            dock_anchor,
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
        mode::capture(ctx, &mut self.state);
        self.flush_config(true);
        mode::quit(ctx, &mut self.state);
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
    fn settle_drag(&mut self, ctx: &egui::Context) {
        let outer = ctx.input(|i| i.viewport().outer_rect);
        match self.drag_settler.feed(outer, Instant::now()) {
            Some(settled) => {
                let area = (self.state.mode() == UiMode::Compact)
                    .then(|| screen::work_area(ctx, settled))
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

    /// `area` 为 `None` 表示这次不吸附（全量窗有系统标题栏，或拿不到屏幕可用区）：只落盘。
    ///
    /// 可用区由调用方注入而不是在这里读屏幕，是为了让吸附决策可以无头断言——
    /// `screen::work_area` 在真机上会读到**这台机器**的屏幕，测试没法固定期望值。
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
        ctx.request_repaint();
    }

    /// 把简洁窗挪到一个新位置：发命令 + 写回配置 + 立即落盘（REQ-009）。
    ///
    /// 落盘走 `force`：拖动结束后没有任何东西再请求重绘，脏标记挂着不动，强杀就丢。
    ///
    /// 停靠态的锚点在这里一起维护：`Some(edge)` 就把锚点搬到刚下发的那个原点，
    /// 解除停靠则清空——这是"拖离"唯一可信的参照（`compact_rect` 每帧被 `capture` 改写）。
    fn place_compact(&mut self, ctx: &egui::Context, position: Pos2, edge: DockEdge) {
        ctx.send_viewport_cmd(ViewportCommand::OuterPosition(position));
        self.state.config.ui.compact_rect.x = position.x;
        self.state.config.ui.compact_rect.y = position.y;
        self.state.config.ui.docked_edge = edge;
        self.dock_anchor = (edge != DockEdge::None).then_some(position);
        self.state.mark_config_dirty();
        self.flush_config(true);
    }

    /// 简洁窗的"设置"：瞬时切回全量并直接落到设置页（REQ-005）。
    ///
    /// 不能复用 `Effects::toggle_mode`：那是"切换"，在全量窗里点设置会反被切成简洁窗。
    fn open_settings(&mut self, ctx: &egui::Context) {
        self.drag_settler.disarm();
        if self.state.mode() != UiMode::Full {
            mode::apply(ctx, &mut self.state, UiMode::Full);
        }
        self.state.tab = Tab::Settings;
        ctx.request_repaint();
    }

    /// 点停靠指示：换下一条边，并立刻把窗口移过去（K-07、REQ-003）。
    ///
    /// 这次移动是程序驱动的，不是用户拖动，所以先把静止判定解除——
    /// 否则 150ms 后 `settle_drag` 会把它当成一次松手，又吸附一遍。
    fn cycle_dock(&mut self, ctx: &egui::Context) {
        let edge = mode::cycle_edge(self.state.config.ui.docked_edge);
        self.state.config.ui.docked_edge = edge;
        self.drag_settler.disarm();
        let outer = ctx.input(|i| i.viewport().outer_rect);
        let area = outer.and_then(|outer| screen::work_area(ctx, outer));
        self.move_to_edge(ctx, edge, outer, area);
        self.state
            .toast(Tone::Info, views::compact::dock_label(edge, false));
    }

    /// 把简洁窗移到某条停靠边。拿不到真实外框或屏幕可用区时只记下新的停靠边、不发命令：
    /// 发一条按猜测算出来的 `OuterPosition` 比什么都不做更糟（窗口会跳到错的地方），
    /// 而下一次形态切换会按配置里的边重新落位。
    fn move_to_edge(
        &mut self,
        ctx: &egui::Context,
        edge: DockEdge,
        outer: Option<Rect>,
        area: Option<Rect>,
    ) {
        let (Some(outer), Some(area)) = (outer, area) else {
            self.state.mark_config_dirty();
            // 用户点了一下却没有任何后续帧时，脏标记会一直挂着，强杀就丢
            self.flush_config(true);
            return;
        };
        let size = self.state.config.ui.compact_rect;
        let window = Rect::from_min_size(outer.min, Vec2::new(size.w, size.h));
        let placed = mode::dock(window, area, edge, self.state.config.ui.dock_sliver_px);
        self.place_compact(ctx, placed.min, edge);
        ctx.request_repaint();
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
            mode::reveal(ctx, &mut self.state);
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
                Ok(()) => self.state.toast(Tone::Info, "已复制到剪贴板"),
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
            let mode = mode::toggle(ctx, &mut self.state);
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
        if effects.cycle_dock {
            self.cycle_dock(ctx);
        }
        if effects.clear_history {
            self.state.clear_history(&self.history);
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
        if !force && self.last_save.elapsed() < CONFIG_SAVE_INTERVAL {
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

/// `KeyState::Error` 里的文案已经是给用户看的，这里只是为了不把整个枚举 `Display` 出来
fn reason_text(state: &KeyState) -> String {
    match state {
        KeyState::Error(reason) => reason.clone(),
        other => other.label().to_string(),
    }
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
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
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
        for action in actions {
            self.run(action, &ctx, &mut effects);
        }

        if close_requested && !self.state.quitting {
            self.hide(&ctx);
        }

        mode::capture(&ctx, &mut self.state);
        self.settle_drag(&ctx);

        let ai_available = self.ai_available();
        match self.state.mode() {
            UiMode::Full => views::show(
                ui,
                &mut self.state,
                &mut effects,
                &self.history,
                ai_available,
            ),
            UiMode::Compact => {
                views::compact::show(ui, &mut self.state, &mut effects, ai_available)
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

    /// 简洁窗常驻置顶，用户往往正在别的应用里干活、顺手就来抓它——那一次窗口是**没有焦点**的，
    /// 而 egui-winit 的 `StartDrag` 被 `window.has_focus()` 挡着（`lib.rs:1961-1968`），
    /// 不先补一条 `Focus` 就表现为"把手拖不动"（走查里报的第一条缺陷）。
    #[test]
    fn an_unfocused_window_gets_focus_before_the_native_drag() {
        let dir = tempfile::tempdir().unwrap();
        let mut app = app(dir.path(), compact_state());
        let c = ctx();
        feed_geometry(
            &c,
            Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(460.0, 96.0))),
            Some(false),
        );

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

    #[test]
    fn a_focused_window_does_not_get_a_redundant_focus_command() {
        let dir = tempfile::tempdir().unwrap();
        let mut app = app(dir.path(), compact_state());
        let c = ctx();
        feed_geometry(
            &c,
            Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(460.0, 96.0))),
            Some(true),
        );

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
        let settled = Rect::from_min_size(
            Pos2::new(1440.0 - 30.0 - 460.0, 400.0),
            Vec2::new(460.0, 96.0),
        );

        app.apply_snap(&c, settled, Some(area()));

        let expected = 1440.0 - 6.0;
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
        state.config.ui.compact_rect.x = 1440.0 - 6.0;
        state.config.ui.compact_rect.y = 400.0;
        let mut app = app(dir.path(), state);
        let c = ctx();
        let settled = Rect::from_min_size(Pos2::new(1440.0 - 6.0, 400.0), Vec2::new(460.0, 96.0));

        app.apply_snap(&c, settled, Some(area()));

        assert!(queued(&c).is_empty(), "已经在那儿了，不该再发几何命令");
        assert_eq!(app.state.config.ui.compact_rect.x, settled.min.x);
    }

    /// TC-3.5-d（接线）：停靠后从 6px 热区向内拖过阈值 → 就地解除停靠，停在松手处并落盘
    #[test]
    fn dragging_a_docked_window_inward_releases_it_and_keeps_the_position() {
        let dir = tempfile::tempdir().unwrap();
        let mut app = app(dir.path(), compact_state());
        let c = ctx();
        app.apply_snap(
            &c,
            Rect::from_min_size(
                Pos2::new(1440.0 - 30.0 - 460.0, 400.0),
                Vec2::new(460.0, 96.0),
            ),
            Some(area()),
        );
        let docked = 1440.0 - 6.0;
        assert_eq!(app.state.config.ui.docked_edge, DockEdge::Right);
        assert_eq!(app.dock_anchor, Some(Pos2::new(docked, 400.0)));

        let dragged = Rect::from_min_size(Pos2::new(docked - 200.0, 400.0), Vec2::new(460.0, 96.0));
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
        let docked = 1440.0 - 6.0;
        app.apply_snap(
            &c,
            Rect::from_min_size(
                Pos2::new(1440.0 - 30.0 - 460.0, 400.0),
                Vec2::new(460.0, 96.0),
            ),
            Some(area()),
        );

        // 拖动途中的一帧：真实几何是"已经往里走了 100px"，capture 把它写进配置
        let in_flight =
            Rect::from_min_size(Pos2::new(docked - 100.0, 400.0), Vec2::new(460.0, 96.0));
        feed_geometry(&c, Some(in_flight), Some(true));
        mode::capture(&c, &mut app.state);
        assert_eq!(
            app.state.config.ui.compact_rect.x, in_flight.min.x,
            "前提：capture 确实把 compact_rect 冲成了途中位置"
        );
        assert_eq!(app.dock_anchor, Some(Pos2::new(docked, 400.0)), "锚点不动");

        // 静止在"向内 200px"处：仍要判出拖离
        let dragged = Rect::from_min_size(Pos2::new(docked - 200.0, 400.0), Vec2::new(460.0, 96.0));
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
        let docked = 1440.0 - 6.0;
        app.apply_snap(
            &c,
            Rect::from_min_size(
                Pos2::new(1440.0 - 30.0 - 460.0, 400.0),
                Vec2::new(460.0, 96.0),
            ),
            Some(area()),
        );

        let slid = Rect::from_min_size(Pos2::new(docked, 600.0), Vec2::new(460.0, 96.0));
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
        let settled = Rect::from_min_size(
            Pos2::new(1440.0 - 30.0 - 460.0, 400.0),
            Vec2::new(460.0, 96.0),
        );

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
        let lost = Rect::from_min_size(Pos2::new(5000.0, 4000.0), Vec2::new(460.0, 96.0));

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

    /// 点击停靠指示：换下一条边并立刻把窗口移过去，同时落盘（K-07）
    #[test]
    fn moving_to_an_edge_places_the_window_and_saves_it() {
        let dir = tempfile::tempdir().unwrap();
        let mut app = app(dir.path(), compact_state());
        let c = ctx();
        let outer = Rect::from_min_size(Pos2::new(500.0, 400.0), Vec2::new(460.0, 96.0));

        app.move_to_edge(&c, DockEdge::Right, Some(outer), Some(area()));

        let expected = 1440.0 - 6.0;
        assert!(queued(&c).contains(&ViewportCommand::OuterPosition(Pos2::new(expected, 400.0))));
        assert_eq!(app.state.config.ui.docked_edge, DockEdge::Right);
        assert_eq!(app.state.config.ui.compact_rect.x, expected);
        assert!(!app.state.config_dirty);
    }

    /// 拿不到真实几何时只记下新的停靠边，不发按猜测算出来的 `OuterPosition`
    #[test]
    fn cycling_the_dock_edge_without_geometry_only_records_the_new_edge() {
        let dir = tempfile::tempdir().unwrap();
        let mut state = compact_state();
        state.config.ui.docked_edge = DockEdge::Right;
        let mut app = app(dir.path(), state);
        let c = ctx();

        let effects = Effects {
            cycle_dock: true,
            ..Default::default()
        };
        app.run_effects(&c, effects);

        assert_eq!(
            app.state.config.ui.docked_edge,
            DockEdge::Bottom,
            "停靠右边 → 下一条是下边"
        );
        assert!(
            !queued(&c)
                .iter()
                .any(|cmd| matches!(cmd, ViewportCommand::OuterPosition(_))),
            "没有真实几何就不该猜一个位置出来"
        );
        assert!(!app.state.config_dirty, "换了边就要落盘");
        assert!(!app.drag_settler.is_armed(), "程序驱动的移动不是用户拖动");
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

    /// 动作行的置顶按钮与 Cmd+Shift+T 共用同一条路径：写回配置 + 下发 WindowLevel + 提示
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
}
