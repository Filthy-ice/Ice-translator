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

use eframe::egui::{self, Align, Key, KeyboardShortcut, Layout, Modifiers, RichText};
use ice_core::config::{Config, UiMode};
use ice_core::history::HistoryStore;
use ice_providers::dispatcher::Translator;
use ice_providers::{AiEngine, MachineTranslateEngine};
use tokio::runtime::Runtime;
use tokio::sync::mpsc;

use crate::jobs::{self, JobRunner, OutcomeReceiver};
use crate::mode;
use crate::state::{self, AppState, KeyState, Tone};
use crate::system::clipboard::{self, CopyKind};
use crate::system::hotkeys::{self, Action, SOURCE_EDITOR_ID};
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
            Action::ToggleTopmost => {
                let on = self.state.toggle_topmost();
                mode::apply_topmost(ctx, &mut self.state);
                self.state
                    .toast(Tone::Info, if on { "已置顶" } else { "已取消置顶" });
            }
            Action::HideWindow => self.hide(ctx),
            Action::Quit => self.quit(ctx),
            // 换行由 `TextEdit` 自己插入（它的 `return_key` 就是 Shift+Enter），应用不参与
            Action::Newline => {}
        }
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
        if effects.clear_history {
            self.state.clear_history(&self.history);
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

        let ai_available = self.ai_available();
        match self.state.mode() {
            UiMode::Full => views::show(
                ui,
                &mut self.state,
                &mut effects,
                &self.history,
                ai_available,
            ),
            UiMode::Compact => compact(ui, &mut self.state, &mut effects),
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

/// 简洁形态（FR-3.2）。P4 只保证"内容可用 + 能切回来"；
/// 拖拽把手、贴边吸附、失焦半透明与形变动画属 P5（K-06~K-09、FR-3.5/3.7/3.9）。
fn compact(ui: &mut egui::Ui, state: &mut AppState, effects: &mut Effects) {
    egui::Panel::top("compact_source")
        .exact_size(34.0)
        .show(ui, |ui| {
            let output = egui::TextEdit::singleline(&mut state.source)
                .id(egui::Id::unique(SOURCE_EDITOR_ID))
                .desired_width(f32::INFINITY)
                .hint_text("输入或粘贴，Enter 翻译")
                .return_key(KeyboardShortcut::new(Modifiers::SHIFT, Key::Enter))
                .show(ui);
            state.focus_in_source = output.response.has_focus();
            state.sync_char_count();
        });

    egui::Panel::bottom("compact_bar")
        .exact_size(28.0)
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 8.0;
                if ui
                    .button("全量")
                    .on_hover_text("切回全量界面（Cmd+Shift+M）")
                    .clicked()
                {
                    effects.toggle_mode = true;
                }
                if ui.button("复制").clicked() {
                    views::full::copy(state, effects, CopyKind::Translation);
                }
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    ui.colored_label(theme::MUTED, state.engine.display_name());
                });
            });
        });

    egui::CentralPanel::default().show(ui, |ui| {
        if let Some(failure) = &state.failure {
            ui.add(egui::Label::new(RichText::new(&failure.message).color(theme::ERROR)).wrap());
        } else if let Some(result) = &state.translation {
            ui.add(egui::Label::new(&result.text).wrap());
        } else if state.in_flight.is_some() {
            ui.horizontal(|ui| {
                ui.spinner();
                ui.colored_label(theme::WARN, "翻译中…");
            });
        } else {
            ui.colored_label(theme::MUTED, "译文显示在这里");
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::{Job, Tab};
    use crate::stub::StubEngine;
    use crate::system::single_instance;
    use crate::views::Effects;
    use ice_core::config::AiProvider;
    use ice_core::glossary::{Glossary, Term};
    use ice_core::language::EngineKind;

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
}
