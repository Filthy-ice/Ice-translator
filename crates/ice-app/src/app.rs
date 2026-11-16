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
use ice_vision::export;
use tokio::runtime::Runtime;
use tokio::sync::mpsc;

use crate::jobs::{self, JobRunner, OutcomeReceiver};
use crate::mode;
use crate::state::{self, AppState, KeyState, Tab, Tone, TrayState};
use crate::system::alpha;
use crate::system::background;
use crate::system::capture::{self, HISTORY_DAYS, HISTORY_MAX};
use crate::system::clipboard::{self, CopyKind};
use crate::system::dblclick;
use crate::system::hotkeys::{self, Action};
use crate::system::mouse;
use crate::system::panel;
use crate::system::screen;
use crate::system::secrets;
use crate::system::single_instance::InstanceLock;
use crate::system::titlebar;
use crate::system::tray;
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
    /// 保存面板在等谁的回信（见 [`PendingSave`]）
    /// 只给测试用：无头环境里没有真的 `NSScreen` 可问，屏列表由测试喂进来
    /// （与 `JobRunner::set_capture_runner` 同一条理由：会问到真屏幕的路径不许被测试撞上）
    #[cfg(test)]
    test_screens: Option<Vec<crate::state::ScreenSpec>>,
    pending_save: PendingSave,
    /// 单实例锁。`on_exit` 里显式释放，进程被杀时靠 `Drop`（REQ-009）
    lock: Option<InstanceLock>,
    key_probe: KeyProbeReceiver,
    /// 「保存」面板那头的发信端（CHG-006 REQ-008）。回信里只有路径，收件在 `save_rx`
    save_tx: panel::ReplySender,
    /// 保存面板回信的接收端，由 `poll_save` 每帧读空
    save_rx: panel::ReplyReceiver,
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
    /// N0 只读探针的上一份读数（见 `system::probe`）：只用于"变了才记"，不参与任何判定。
    window_probe: Option<crate::system::probe::WindowProbe>,
    /// 同上，停靠链那一组。
    dock_probe: Option<crate::system::probe::DockProbe>,
    /// 截图段末尾（派发 + `probe::SPAN_MS`）：只有段内的帧停顿才算线索。
    probe_span_until: Option<u64>,
    /// 上一帧的时刻，用来量"这一帧离上一帧多久"。
    probe_frame_at: Option<u64>,
    /// 上一帧下到原生层的整窗不透明度（FR-3.7，见 `apply_native_alpha`）。
    /// `None` = 还没下过或上一次没走通。
    native_alpha: Option<f32>,
    /// 上一帧下到原生层的整窗底色（含标题栏那条带子，见 `apply_native_background`）；
    /// `None` 的语义同 `native_alpha`。
    native_background: Option<egui::Color32>,
    /// 上一帧读回来的"标题栏有没有被压平"（D-26，见 `system::background::sync_titlebar`）。
    /// `None` = 还没读到过（非 macOS / 还没句柄 / 第一帧）。
    native_flat_titlebar: Option<bool>,
    /// 上一帧标题栏绿键的改道结果（见 `apply_zoom_hook`）。`None` = 还没下过。
    zoom_outcome: Option<titlebar::Outcome>,
    /// 上一帧标题栏双击监视器的装载结果（见 `apply_dblclick_hook`）。`None` = 还没下过。
    dblclick_outcome: Option<dblclick::Outcome>,
    /// 热键的 handler 与首轮注册只在启动后做一次
    hotkeys_started: bool,
    /// 根窗"按回不透明"的命令还要不要再发一次。CHG-011 开了进程级透明让药丸浮空，但它连主窗
    /// 一起变半透明（事实 20）；这一格把根窗单独发回 `Transparent(false)`。换形态时 winit 换窗、
    /// 透明跟着回到 native 的 true，所以要重发——`apply_native_background` 探到换窗就把它置回真。
    root_opaque_pending: bool,
    /// 截图历史目录，启动时解析一次。
    ///
    /// **为什么留成一份值而不是每次现问系统**：老化和「清除所有截图」都会**真删文件**。
    /// 无头测试必须能把它换到一个临时目录里去——不然跑一条断言就顺手删了用户截过的图。
    /// 这与剪贴板、面板、钥匙串那三条注入口是同一条纪律（`app_with_history` 就是同一手法）。
    shots_dir: Option<std::path::PathBuf>,
}

impl IceApp {
    // 参数就是这么多：装配根本来就要把引擎、运行时、历史、配置路径、锁、Key 探测、
    // 截图目录一起交进来（NFR-6 的依赖方向要求这些都在外面造好再注入）。
    // 包成一个 struct 只会多一层没有信息量的中间类型。
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        mut state: AppState,
        runtime: Runtime,
        translator: Translator,
        history: HistoryStore,
        config_path: PathBuf,
        lock: Option<InstanceLock>,
        key_probe: KeyProbeReceiver,
        // 截图历史目录。**由调用方给**而不是在这里现问系统：老化和一键清除都会真删文件，
        // 只要有一次忘记注入，测试就会删掉用户真的截图历史——参数化之后这种事编译不过
        shots_dir: Option<PathBuf>,
    ) -> Self {
        let (mut runner, outcomes) = JobRunner::new(translator);
        let (save_tx, save_rx) = panel::channel();
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
            #[cfg(test)]
            test_screens: None,
            pending_save: PendingSave::None,
            hotkeys_started: false,
            root_opaque_pending: true,
            shots_dir,
            lock,
            key_probe,
            save_tx,
            save_rx,
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
            window_probe: None,
            dock_probe: None,
            probe_span_until: None,
            probe_frame_at: None,
            native_alpha: None,
            native_background: None,
            native_flat_titlebar: None,
            zoom_outcome: None,
            dblclick_outcome: None,
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
            // 全局热键那三把（CHG-009 REQ-006）。截图那一把与图片页那颗「框选贴图」
            // **落同一个意图**，派发只在帧末一处做；「固定」在没有"框选但没钉住"的那张时
            // 什么都不做（不弹一句"没有可固定的"，也不拿历史里更早那张顶上去——那会把
            // 两把键混成一把）；「打开上一次」沿历史从新到旧**依次往回翻**（CHG-012 改口
            // CHG-009 REQ-006"取最近那张"：每按一次新钉一张更早的、累积，翻到头说一句）
            Action::Capture => effects.capture = true,
            Action::PinLast => {
                // 「固定」与「对勾」同义（本文件那条注释 + 用户原话）。遮罩还开着时，F3 就
                // 递一个"替我按下对勾"给遮窗——那一刻框在哪只有那一扇知道，选区不回传主窗，
                // 主窗只触发。遮罩没开时**什么都不做**（REQ-006"没有就什么都不做"；上限去掉后
                // 不再有"确认了没钉上"的那一格，用户 2026-11-16 拍板保持这一档）。
                if self.state.capturing() {
                    crate::system::selection::request_confirm(ctx);
                    // 让遮窗这一帧就重画一次去消费这个请求，不然要等它下一次恰好醒
                    ctx.request_repaint();
                }
            }
            Action::ReopenLast => {
                // 读盘在帧末这一层，状态层不碰 fs（CHG-009 REQ-005 分层纪律）。两档分流照
                // CHG-012 REQ-002：空→"还没有截图记录"；翻到头（next_older=None）→"没有更早的
                // 截图了"且不推进锚点。钉上才由 browse_older 记锚点、打来源标记。
                let shots = self
                    .shots_dir
                    .clone()
                    .map(|dir| capture::list(&dir))
                    .unwrap_or_default();
                if shots.is_empty() {
                    self.f2_notice("还没有截图记录");
                } else {
                    match capture::next_older(&shots, self.state.browse_anchor.as_deref()) {
                        None => self.f2_notice("没有更早的截图了"),
                        Some(shot) => {
                            self.state.browse_older(shot.path.clone());
                        }
                    }
                }
            }
            Action::HideWindow => self.hide(ctx),
            Action::RevealWindow => self.reveal(ctx),
            // 托盘菜单「设置」：收着就先唤回，再开设置页。兑现复用 `open_settings`
            // （工具条与「去设置」那一条走的同一个入口），不在托盘里另写第二条。
            Action::OpenSettings => {
                self.reveal(ctx);
                effects.open_settings = true;
            }
            // 托盘菜单「清除历史截图」（用户 2026-11-13 原话）。这条确认框只有全量窗会画
            // （`confirm` 挂在 `views::show`，精简窗只画 toasts），所以借"打开设置页"把窗落到
            // 能画它的宿主，再把设置页那颗「清除所有截图」**已有**的 `Confirm::ClearShots` 二次
            // 确认原样弹上来——不新增第二条删除实现、不新增第二种弹框。确认→ `clear_shots` 效果。
            Action::ClearShots => {
                self.reveal(ctx);
                effects.open_settings = true;
                self.state.confirm = Some(state::Confirm::ClearShots);
            }
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

    /// 切换置顶（K-09）。快捷键、工具条上那枚图钉（`views::pin_toggle`）与设置页那一格共用这一条路径。
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

    /// 收进后台（Cmd+W / 关闭按钮）。去哪儿由 `state.tray` 决定，见 `mode::hide`；
    /// 降级档的恢复判定与心跳见 `watch_visibility`。
    fn hide(&mut self, ctx: &egui::Context) {
        let tray = self.state.tray;
        mode::hide(ctx, &mut self.state, tray);
        // 托盘那一支用不上这个闸门（唤回只有"点那枚图标"一条路），降级支要用：
        // 记的是"收起来的时刻"，`watch_visibility` 按 `state.tray` 决定读不读它。
        self.reveal_guard.on_hide(Instant::now());
    }

    /// 托盘唤回（左键，或菜单第一项）。已经可见时什么都不做：重发几何会把用户刚铺满的
    /// 窗口缩回去，而那一次点击只是想确认软件还在。
    fn reveal(&mut self, ctx: &egui::Context) {
        if !self.state.hidden {
            return;
        }
        // 浮空提示是"收着时"那条路；唤回后主窗自己能画 toasts，把浮空那条收掉免得两扇叠着。
        self.state.float_hint = None;
        let tray = self.state.tray;
        mode::reveal(ctx, &mut self.state, &mut self.pending_geometry, tray);
    }

    /// F2 那两句"没得翻"的提示走哪条路：主窗看得见就进 toasts（画在主窗里），收着就进
    /// 浮空窗（独立一扇，后台也看得见，用户 2026-11-16）。两条不叠加，唤回时浮空那条收掉。
    fn f2_notice(&mut self, text: &str) {
        if self.state.hidden {
            self.state.set_float_hint(Tone::Info, text);
        } else {
            self.state.toast(Tone::Info, text);
        }
    }

    /// 全局热键：装上 handler、按需注册、把命中的那一下送进**既有的 `Action` 分派**。
    ///
    /// 挂在 `logic()` 而不是 `ui()`：窗口收起时 `ui()` 一帧都不跑，挂在那儿等于"只有窗口
    /// 看得见时才听得到热键"（D-21 那一笔账）。这里也不立任何心跳——实测热键事件本身就能
    /// 把停摆的循环叫回一帧（回调里那句 `request_repaint`，事实 38）
    fn pump_hotkeys(&mut self, ctx: &egui::Context, effects: &mut Effects) {
        if !crate::system::global_hotkeys::SUPPORTED {
            return;
        }
        if !self.hotkeys_started {
            self.hotkeys_started = true;
            crate::system::global_hotkeys::arm(ctx);
            if let Err(failure) = crate::system::global_hotkeys::apply(&self.state.config.hotkey) {
                // 启动时就抢不到键：说清哪一把、并告诉他还能用按钮
                self.state.toast(Tone::Warn, failure.explain());
            }
        }
        if let Some(action) = crate::system::global_hotkeys::take_action() {
            self.run(action, ctx, effects);
        }
    }

    /// 菜单栏托盘的泵：定档、核对、把 `outcome` 交回**既有**的 `run` 分派。
    /// 只由 `App::logic` 调（见那里的注释）——那是可见与收起两条路都会跑到的钩子。
    ///
    /// `scale` 与 `outcome` 由调用方给：真机那一次来自 `eframe::Frame` 与
    /// `tray::take_outcome()`，测试给一份固定的，才不必去碰进程级那个意图槽——那个槽被两个
    /// 并行测试踩到就是随机红。
    ///
    /// 三个决定各有一条理由：
    /// - **只在没定档时装**（`Unknown`）。装上之后每帧只做核对；装不上之后就停在
    ///   `Unavailable`，不再每帧重造状态项——反复造会在菜单栏里长出第二枚图标（REQ-001）。
    /// - **结果变了才说话**（与 `apply_zoom_hook`、`apply_dblclick_hook` 同一手法）：
    ///   逐帧记会把日志刷满。macOS 上装不上要给用户一条提示条，因为那一档的关窗语义
    ///   跟着换回"收进后台"，界面得说实话（REQ-005、REQ-006）。
    /// - **意图走 `run(action)`**，不在这外面另写一套兑现。四条意图与快捷键、设置页、
    ///   绿键落在同一批动作上，`Action::Quit` 必须是既有的那一条（TC-5.4-b）。
    fn pump_tray(
        &mut self,
        scale: f64,
        outcome: Option<tray::Outcome>,
        ctx: &egui::Context,
        effects: &mut Effects,
    ) {
        let next = match self.state.tray {
            TrayState::Unknown => tray::install(self.state.mode(), ctx, scale),
            TrayState::Available => tray::sync(self.state.mode()),
            TrayState::Unavailable => TrayState::Unavailable,
        };
        if next != self.state.tray {
            tracing::info!(?next, "托盘这一档定下来了");
            if tray::SUPPORTED && next == TrayState::Unavailable {
                self.state.toast(
                    Tone::Warn,
                    "菜单栏图标没能装上：关窗会收进后台，Dock 磁贴也留在原地",
                );
            }
            self.state.tray = next;
        }
        if let Some(outcome) = outcome {
            // D-21 的取证点：这一条打出来，说明"点击→意图→logic"整条管道是通的，
            // 问题只剩兑现那一层；打不出来，才是管道断了。真机上只有这一条能分辨两种病因。
            tracing::info!(?outcome, "托盘交来一件事");
            self.run(tray::action_of(outcome), ctx, effects);
        }
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
    /// macOS 那边进的是嵌套事件循环），所以"松手了没有"要**另外问**：`held_for` 给的是
    /// 硬件此刻的按键状态（`system::mouse`），问不到（`None`）才退回纯几何静止判定。
    ///
    /// **为什么非问不可**（缺陷 D-16，2026-10-28 用户原话"虽然我将其拖动到边上，但我还没有
    /// 松手，他就着急往里面收入，导致我只要一直不松手，鼠标和界面的距离会越来越大"）：
    /// 只看"几何连续 150ms 不变"的话，用户拖到边上**停住但没松手**就被判成松手，窗口当场被
    /// 吸附+藏到边外，而 AppKit 的拖拽还在跟手——于是窗口从指针底下滑走，按住越久差得越远。
    ///
    /// `area` 由调用方注入（同 `watch_dock` 的理由：真机要读屏幕，测试给一块固定的桌面）
    fn settle_drag(
        &mut self,
        ctx: &egui::Context,
        area_for: impl Fn() -> Option<Rect>,
        held_for: impl Fn() -> Option<bool>,
    ) {
        let outer = ctx.input(|i| i.viewport().outer_rect);
        self.observe_native_move(outer);
        if self.drag_settler.is_armed() && held_for() == Some(true) {
            // 还按着：几何再静也一个结论都不下，把静止计时重新起算，并留住下一帧
            self.drag_settler.postpone(Instant::now());
            ctx.request_repaint_after(mode::DRAG_SETTLE);
            return;
        }
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
            // 铺满那一支重发的是**当初那块可用区**（`state.fill`），不是从配置里现推的位置：
            // 槽里记的还是铺满之前的几何，现推会把窗口拽回铺满前，再叠上没落定的尺寸（D-24）。
            let commands = match self.state.fill {
                Some(area) => mode::fill_commands(area, target),
                None => mode::resend_commands(&self.state.config.ui, self.state.mode(), target),
            };
            tracing::debug!(
                w = target.x,
                h = target.y,
                filled = self.state.fill.is_some(),
                "尺寸未落定，重发尺寸与原点请求"
            );
            for command in commands {
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

    /// N0 只读探针（四条读数的口径与"为什么是这几个数"在 `system::probe` 的模块说明里）。
    ///
    /// `want` 是按同一组输入**重算**一遍 `mode::dim_alpha`，不从 `ui` 里那份缓存取：探针不许
    /// 反过来影响判定，也不许因为借用 `self.state` 而卡住后面那次对 `self.window_probe` 的可变借用。
    fn trace_window(&mut self, ctx: &egui::Context, frame: &eframe::Frame) {
        let now = crate::system::probe::now_ms();
        let in_span = crate::system::probe::in_span(self.probe_span_until, now);
        crate::system::probe::note_frame_gap(&mut self.probe_frame_at, now, in_span);
        let mode = self.state.mode();
        let focused = ctx.input(|i| i.viewport().focused) == Some(true);
        let (dim_enabled, unfocused_alpha) = (
            self.state.config.ui.dim_when_unfocused,
            self.state.config.ui.unfocused_alpha,
        );
        let capturing = self.state.capturing();
        let want = mode::dim_alpha(mode, focused, dim_enabled, unfocused_alpha);
        let min = mode::min_size_for(mode);
        // `native_alpha` 是"这一档真的落到原生层"那份缓存（失败时写 `None`，下一帧重试）
        let alpha_native = self.native_alpha == Some(want);
        let min_native = crate::system::probe::content_min_size(frame);
        // 窗真没了没有、alpha 实际是多少：以原生回读为准，不以"我们发过命令"为准
        let (native_visible, native_alpha) = match crate::system::probe::native_visibility(frame) {
            Some((visible, alpha)) => (Some(visible), Some(alpha)),
            None => (None, None),
        };
        crate::system::probe::note_window(
            &mut self.window_probe,
            crate::system::probe::WindowProbe {
                mode,
                focused,
                capturing,
                want,
                alpha_native,
                min_native,
                min_wanted: [min.x, min.y],
                native_visible,
                native_alpha,
            },
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
        if !needs_native_update(self.native_alpha, want) {
            return true;
        }
        let applied = alpha::set(frame, want);
        self.native_alpha = applied.then_some(want);
        // 值真的变了才记：这条是"标题栏到底有没有跟着淡"的唯一现场证据——无头测试拿不到
        // 窗口句柄，走没走到原生层只能在这里看。失败侧只警告一次（`needs_native_update`
        // 对失败不缓存，每帧都会重试，逐次打日志会刷屏，与 `system::screen` 同一手法）。
        if applied {
            tracing::debug!(alpha = want, native = true, "整窗不透明度下发");
        } else if !ALPHA_FALLBACK_WARNED.swap(true, std::sync::atomic::Ordering::Relaxed) {
            tracing::warn!("原生整窗不透明度下不发，失焦只淡窗内底色（标题栏不会跟着淡）");
        }
        applied
    }

    /// 把整窗底色下到位——**包括 AppKit 画的那条 32pt 标题栏**（用户 2026-11-04："标题栏的颜色
    /// 和界面不统一，有色差"）。色差就是这么进来的：上一轮只把冷灰白写进 egui 的
    /// `window_fill`/`panel_fill`，而窗体顶上那一条压根不在 egui 的绘制清单里。
    ///
    /// 色值只有一个出处：`theme::window_base`。原生这一句与内容区必须拿到同一个数，
    /// 两级分层（整窗基底 / 正文框浅一档）的全部细节在 `system::background`。
    ///
    /// 缓存与重试沿用 `apply_native_alpha` 那一套；差别是这条没有回退路径——下不去就是
    /// 标题栏跟着系统底色，界面里没有第二处可改口，所以也不发警告。
    fn apply_native_background(&mut self, frame: &eframe::Frame, dark: bool) {
        // 压平标题栏排在底色之前，而且**每帧读回来核对**：`titlebarAppearsTransparent` 为假时
        // 那 32pt 由 `NSThemeFrame` 按系统 chrome 画（真机量下来是 `#FFFFFF`，正文是 `#F7_F9_FC`
        // 的基底），底色下发得再准也只改到内容区——D-23 那一版就是这么"半改完"的（D-26）。
        let flat = background::sync_titlebar(frame);
        if flat != self.native_flat_titlebar {
            tracing::debug!(flat = ?flat, "标题栏平铺核对");
            self.native_flat_titlebar = flat;
        }
        // 根窗按回不透明：CHG-011 的进程级透明会连主窗一起变半透明（事实 20），这里把主窗
        // 单独设 opaque，药丸那扇独立窗不受影响。**放在底色那道"变了才发"的早退之前**——
        // 底色没变时窗口也可能因换形态丢了 opaque，这一句每帧核对不能跟着底色一起被跳过。
        if background::sync_opaque(frame) == Some(false) {
            tracing::debug!("根窗 opaque 没设上（拿不到句柄/非主线程），下一帧再试");
        }
        if flat == Some(false) {
            // 刚被换回来的那扇窗（换形态时 winit 换窗口，属性回到系统默认）或这一次没压成：
            // 底色那份缓存不作数，逼下一句重发一次。否则新窗口上是"平铺了、但底色还是默认"
            self.native_background = None;
            // 换窗也把"透明"带回成了 native 的 true → 主窗又半透明；标记要重发一次不透明命令
            self.root_opaque_pending = true;
        }

        let want = theme::window_base(dark);
        if !needs_native_update(self.native_background, want) {
            return;
        }
        let applied = background::set(frame, want);
        self.native_background = applied.then_some(want);
        if applied {
            tracing::debug!(?want, native = true, "整窗底色下发");
        }
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
                    tracing::warn!("接不上标题栏绿键，从精简窗切回全量只剩 Cmd+Shift+M");
                }
            }
        }
    }

    /// 帧首取走标题栏双击（任务 #25，用户"双击标题栏：铺满整屏 + 再双击还原"）。
    ///
    /// 与 `poll_titlebar_click` 同一形状：只把"被双击了"翻译成一条 effect，
    /// 铺满还是还原全部在 `toggle_fill` 里判，原生那条路径上不做任何业务判断。
    /// 取走这件事本身无条件执行——不收的话标记会一直挂着，等窗口回来时凭空多铺一次。
    fn poll_dblclick(&mut self, effects: &mut Effects) {
        if dblclick::take_double_click() && !self.state.hidden {
            effects.toggle_fill = true;
        }
    }

    /// 铺满 ↔ 还原（任务 #25）。两条路径都只发尺寸与原点，且都上护栏。
    ///
    /// 护栏是必须的：命令是异步请求，还原那一帧量到的**还是整屏**，`capture` 那时刚解除抑制，
    /// 不拦就会把整屏尺寸当成"用户选的尺寸"写进 `full_rect`——D-6 换了一个入口重犯。
    fn toggle_fill(&mut self, ctx: &egui::Context) {
        let (Some(inner), Some(outer)) = (
            ctx.input(|i| i.viewport().inner_rect),
            ctx.input(|i| i.viewport().outer_rect),
        ) else {
            // 几何还没报到（建窗当帧）：这一帧既算不出装饰差值也定不了铺满目标，下一帧再来
            return;
        };
        let (commands, target, fill) = if self.state.fill.is_some() {
            (
                mode::restore_commands(&self.state.config.ui),
                mode::target_size(&self.state.config.ui, UiMode::Full),
                None,
            )
        } else {
            // 可用区按**窗口所在那块屏**取（`work_area`），不是整个桌面并集：两块屏的用户
            // 双击要铺满的是自己眼前那块，`desktop` 的并集会把窗口拉过接缝、跨到另一屏上。
            let Some(area) = screen::work_area(ctx, outer) else {
                return;
            };
            let size = mode::fill_size(area, inner.size(), outer.size());
            (mode::fill_commands(area, size), size, Some(area))
        };
        if fill_is_no_op(fill.is_some(), target, &self.state.config.ui) {
            return;
        }
        self.state.fill = fill;
        for command in commands {
            ctx.send_viewport_cmd(command);
        }
        self.pending_geometry.set(target, Instant::now());
        // `area` 是这一句里最有用的数：D-24 现场"下发的目标比实测大一截"到底大在哪个屏、
        // 哪块可用区上，只看尺寸看不出来。只记矩形，不记屏名也不记任何用户内容（NFR-3）。
        tracing::debug!(
            filled = fill.is_some(),
            w = target.x,
            h = target.y,
            area = ?fill,
            "双击标题栏：下发铺满/还原的几何命令"
        );
        ctx.request_repaint();
    }

    /// 只在**全量窗**盯着标题栏双击（任务 #25；悬浮条铺满整屏不是它该做的事）。
    /// 原生细节、吞哪些落点、以及"双击之后立刻按住拖动会失效"这条代价见 `system::dblclick`。
    ///
    /// 每帧核对现状，**只在结果变了才记日志**（与 `apply_zoom_hook` 同一手法）：
    /// 装没装上是真机上唯一能区分"双击没被拦"和"拦了但铺满命令没发出去"的现场证据。
    fn apply_dblclick_hook(&mut self, frame: &eframe::Frame, ctx: &egui::Context, want: bool) {
        let outcome = dblclick::sync(frame, ctx, want);
        if self.dblclick_outcome == Some(outcome) {
            return;
        }
        self.dblclick_outcome = Some(outcome);
        match outcome {
            dblclick::Outcome::Watching => {
                tracing::debug!("标题栏双击改道：铺满整屏 / 再双击还原")
            }
            dblclick::Outcome::Idle => tracing::debug!("标题栏双击交回系统默认（简洁窗）"),
            dblclick::Outcome::Unavailable => {
                if !DBLCLICK_UNWATCHED_WARNED.swap(true, std::sync::atomic::Ordering::Relaxed) {
                    tracing::warn!("拦不下标题栏双击，双击仍是系统设置里那一档（默认多为最小化）");
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
        // N0 只读探针：这一串布尔里任何一个假掉，后面整段都不会动（口径见 `system::probe`）。
        // 排在两个早退之前，正因为"没走进后面的代码"才是这次要看的东西。
        let docked = mode::is_docked(edge, self.state.mode());
        crate::system::probe::note_dock(
            &mut self.dock_probe,
            crate::system::probe::DockProbe {
                edge,
                docked,
                needs_land: docked && self.dock_anchor.is_none(),
                shown: self.dock_shown,
                latched: self.dock_latched,
                armed: self.drag_settler.is_armed(),
                hidden: self.state.hidden,
            },
        );
        if !docked {
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

        // 指针在屏幕坐标系里的位置：**先问窗口服务器**。`hover_pos()` 量的是"事件投递给我们了
        // 没有"，应用不在前台时它会说指针不见了，而用户的鼠标还压在条上——收回计时就是这么
        // 提前跑起来的（D-22，症状与判据见 `mode::DOCK_HOVER_PAD`）。
        // 问不到（非 macOS、无头测试不在主线程）退回那一份代理量，它是窗口内局部坐标，
        // 加上外框原点搬到同一套坐标里。
        let cursor = mouse::cursor().or_else(|| {
            ctx.input(|i| i.pointer.hover_pos())
                .map(|at| at + outer.min.to_vec2())
        });
        // 创口带判定用窗口内坐标：唤出与收回都会整条挪窗，指针在窗口里的相对位置才是热区本身
        let local = Rect::from_min_size(Pos2::ZERO, inner.size());
        let pointer = cursor
            .filter(|at| window.contains(*at))
            .map(|at| Pos2::new(at.x - outer.min.x, at.y - outer.min.y));
        // "走开了"要走出外扩那一圈才算：正好压在边线上、鼠标还在去的路上不算走开
        let away = mode::walked_away(window, cursor);
        if self.dock_shown {
            if !away || self.state.focus_in_source || ctx.input(|i| i.pointer.any_down()) {
                // 指针还在附近，或正在输入行里打字、按住窗口拖到了边外（选译文选出了界）：
                // 那几种情况下"指针离开"是操作的一部分，不是"不看我了"（REQ-003 的例外）
                self.dock_left_at = None;
            } else {
                let now = Instant::now();
                let since = *self.dock_left_at.get_or_insert(now);
                let gone = now.duration_since(since);
                if gone >= mode::DOCK_AUTO_HIDE {
                    let Some(area) = area_for() else {
                        return;
                    };
                    tracing::debug!(?cursor, ?window, "停靠收回：指针已走出热区一圈");
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

    /// 「保存」面板的回信（CHG-006 REQ-008）。
    ///
    /// 落盘就在这一帧做完，不另开一支任务：编码一张图是一次性的几十到几百毫秒，由用户显式点
    /// 出来，换不来"第四条 Job + 与成品换代赛跑"那套机制。一次读空，不留到下一帧
    /// （窗口一收进后台 `ui` 就停了，留着可能永远没人读）。
    fn poll_save(&mut self) {
        while let Ok(reply) = self.save_rx.try_recv() {
            match reply {
                panel::Reply::Picked(path) => {
                    // 面板开着的那几秒里可能又点了一颗别的保存、也可能把那张贴图关了：
                    // 按**当初那一份**去存，来源没了就照"没了"说，不拿最后一张顶缸
                    let pending = std::mem::replace(&mut self.pending_save, PendingSave::None);
                    let (tone, text) = match pending {
                        PendingSave::Pin(id) => save_pin_image(&self.state, id, &path),
                        PendingSave::Shot(source) => save_shot_copy(&source, &path),
                        // 没有来源的回信无处可存：不碰盘，也不拿提示打扰用户
                        PendingSave::None => continue,
                    };
                    self.state.toast(tone, text);
                }
                // 取消是用户自己的选择，不需要一句"你取消了"来打扰——但**来源要清掉**：
                // 留着它就等于认定"下一份成品该存给它",那才是真出错
                panel::Reply::Cancelled => self.pending_save = PendingSave::None,
                panel::Reply::Failed => {
                    self.pending_save = PendingSave::None;
                    self.state
                        .toast(Tone::Error, "保存面板没有回应；请再点一次「保存」")
                }
            }
        }
    }

    /// 隐藏期间的看护（缺陷 D-2）——**只服务降级档**（`TrayState::Unavailable`，收起来 = 最小化）。
    ///
    /// 必须挂在 `App::logic` 上：最小化之后 eframe 不再跑 egui pass，只按
    /// `MISSED_REDRAW_TIMEOUT` 调 `logic`（`update_logic_only`），`ui` 一帧都不执行。
    /// 之前把判定和心跳写在 `ui` 里，窗口一收起来就全断了——从 Dock 叫回来时没有任何
    /// 一帧去发现"回来了"，画面就停在旧的那一帧上。
    /// `run_logic` 会刷新 `viewport()` 里的几何与焦点，也会把这里发的命令带出去，够用。
    ///
    /// 托盘那一档（`Available`）整条都跳过：窗口是我们自己 `Visible(false)` 隐的，
    /// "回来了"由用户点那枚图标说了才算，不需要在这儿猜；两条并行唤醒路径留着，
    /// 出的就是"图标点一下窗口弹两回"这类只对一半的缺陷（REQ-006）。
    fn watch_visibility(&mut self, ctx: &egui::Context) {
        if self.state.tray == TrayState::Available {
            return;
        }
        let (inner_rect, focused) = ctx.input(|i| (i.viewport().inner_rect, i.viewport().focused));
        if self
            .reveal_guard
            .check(self.state.hidden, Instant::now(), inner_rect, focused)
        {
            let tray = self.state.tray;
            mode::reveal(ctx, &mut self.state, &mut self.pending_geometry, tray);
        }
        if self.state.hidden {
            ctx.request_repaint_after(mode::HIDDEN_HEARTBEAT);
        }
    }

    /// 帧末续期：只给有观众的帧续（REQ-007）。
    ///
    /// 隐藏期一次都不发：窗口不可见时每续一次期就多跑一次 pass，而这段时间唯一该发生的
    /// 唤醒是"托盘被点了"——那一下自己会 `request_repaint`（`system::tray` 的回调），
    /// eframe 那侧本来也会给不可见窗兜底跑一次 pass，所以不需要常驻心跳。
    /// 提示条因此停在原样，唤回后接着淡出，用户看不见中间这几帧。
    fn renew_repaints(&mut self, ctx: &egui::Context) {
        if self.state.hidden {
            return;
        }
        if self.state.tick_toasts() {
            ctx.request_repaint();
        }
        if !self.state.toasts.is_empty() {
            // 淡出要一帧一帧才看得出动：按 `TOAST_FADE_STEP`（16ms，1s 约 60 帧）续重绘，
            // 提示条一消失就停——不常驻重绘，空闲时 CPU 才能压到 1% 以下（NFR-2）
            ctx.request_repaint_after(state::TOAST_FADE_STEP);
        }
    }

    /// 历史老化：一次截图之后顺带跑一遍，**不开定时器**（REQ-008：没人看的时候不需要老化，
    /// 帧循环里少一件常驻的事也是好的）。删的是真删，不标记。
    fn age_shots(&mut self) {
        let Some(dir) = self.shots_dir.clone() else {
            return;
        };
        let shots = capture::list(&dir);
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or_default();
        for path in capture::retain(&shots, now, HISTORY_DAYS, HISTORY_MAX) {
            // 删不掉就说一句：悄悄留着会让用户以为"清除"没生效
            if let Err(error) = std::fs::remove_file(&path) {
                // 只记错误类别：io::Error 的 Display 可能带路径，而那个路径就是用户内容
                tracing::warn!(kind = ?error.kind(), "截图历史有一条没删掉");
            }
        }
    }

    /// 把贴图那几扇窗画出来，并取走它们各自攒下的意图。
    ///
    /// 先 `clone` 一遍清单再遍历：处理一条意图会改 `state.pins`（关一张就摘一条），
    /// 边遍历边改是借位检查器最爱抓的事；这里宁可多拷几份 `PathBuf`。
    /// 遮罩那一组窗（CHG-010 REQ-101/102/103）：每块屏一扇、每帧幂等重下抬层与激活，
    /// 再取走它们攒下的意图。
    ///
    /// 开哪几扇窗、抬到哪一层、这一刻是不是前台，都是"这一帧的屏幕事实"，
    /// 所以放在帧循环而不是回信里（`apply_outcome` 只改状态，REQ-005 那条分层纪律）。
    fn draw_capture(&mut self, ctx: &egui::Context, effects: &mut Effects) {
        if !self.state.capturing() {
            return;
        }
        let Some(capture) = self.state.capture.clone() else {
            return;
        };
        let mut outs: Vec<crate::system::selection::MaskOut> = Vec::new();
        // 认窗那一份快照：整段遮罩期间只问系统一次（画面已冻结，问第二遍只会认出图里
        // 不存在的窗）。问的动作放在帧循环而不是回信里——它要主线程，且属于"这一刻的屏幕事实"
        let windows = match capture.windows.clone() {
            Some(snapshot) => snapshot,
            None => {
                let snapshot = std::sync::Arc::new(crate::system::windows::Snapshot::take());
                tracing::info!(
                    t_ms = crate::system::probe::now_ms(),
                    "认窗：在线 {} 扇，自家归属名认作 {}",
                    snapshot.entries.len(),
                    snapshot.self_owner
                );
                if let Some(live) = self.state.capture.as_mut() {
                    live.windows = Some(snapshot.clone());
                }
                snapshot
            }
        };
        // 光标此刻在哪块屏：问窗口服务器（与事件投递无关，D-16/D-22 同一条纪律）。
        // 认窗高亮只在那块屏上做——另一扇遮罩读不到指针会退回本地 (0,0)，不挡的话它左上角
        // 那扇窗会被点亮（用户那句「两块屏上的窗都亮了、只有周边灰」）。问不到（非 mac、
        // 无头不在主线程）退回 None：各遮罩沿用各自的指针判法，与今天一致。
        let screens: Vec<(u8, Rect)> = capture
            .frames
            .iter()
            .map(|f| (f.display, Rect::from_min_size(f.origin, f.points)))
            .collect();
        let lit = crate::system::selection::lit_display(mouse::cursor(), &screens);
        if lit != capture.lit {
            if let Some(live) = self.state.capture.as_mut() {
                live.lit = lit;
            }
            // 跨屏那一刻：亮的那扇要换人。子窗不会自己醒（指针只到鼠标底下那一扇），
            // 主窗点名让每一扇都重画一次，带上各自新的 cursor_here。
            for frame in capture.frames.iter() {
                crate::system::selection::wake_mask(ctx, frame.display);
            }
        }
        // 原生那两下（抬层 / 激活）只在头几帧重下。**配额是真的会减的**：减到 0 或已收敛
        // 就一次都不再问，主窗也跟着睡——每帧枚举窗口、重设 frame 正是 D-31~D-34 把主线程
        // 拖死的那条路（`Capture::push_frames`）
        let pushing = capture.push_frames > 0;
        let mut covers = true;
        for frame in capture.frames.iter() {
            let title = crate::system::selection::mask_title(frame.display);
            crate::system::selection::show(
                ctx,
                &crate::system::selection::MaskScreen {
                    display: frame.display,
                    points: frame.points,
                    origin: frame.origin,
                    windows: windows.clone(),
                    texture: freeze_texture(ctx, frame, capture.frozen_id),
                    frozen_id: capture.frozen_id,
                    cursor_here: match lit {
                        Some(d) => d == frame.display,
                        None => true,
                    },
                },
            );
            if pushing {
                // 幂等重下：实测 AppKit 与窗口服务器会短时不一致，而且框架自己也会按
                // viewport 设置重摆这扇窗（与 D-26/D-27 同族）
                covers &= crate::system::cover::cover_display(&title, frame.appkit).covers;
            }
            outs.extend(crate::system::selection::take_outs(ctx, frame.display));
        }
        let mut key = self.state.capture_has_keyboard();
        if pushing {
            self.state.set_raised(covers);
            // esc 收不收得到取决于我们是不是前台（事实 9/16）。激活是异步的，
            // 所以这几帧里重下，读回来的值进状态、不当成功记账
            let focus = crate::system::cover::focus(&crate::system::selection::mask_title(
                capture.frames.first().map_or(1, |f| f.display),
            ));
            if key != focus.key {
                ctx.request_repaint();
            }
            key = focus.key;
            self.state.capture_key = focus.key;
            let settled = covers && key;
            self.state.consume_push_frame(settled);
            let left = self.state.capture.as_ref().map_or(0, |c| c.push_frames);
            if settled {
                tracing::info!("遮罩：抬层与激活已收敛，原生那两下收手");
            } else if left == 0 {
                tracing::warn!(
                    "遮罩：重下 {} 帧仍没收敛（盖住={} 键盘={}），停手不再骚扰窗口服务器",
                    crate::state::PUSH_FRAMES,
                    covers,
                    key
                );
            } else {
                // 还没收敛就继续要帧：主窗一睡，这两下再也不会重试
                ctx.request_repaint();
            }
        }
        for out in outs {
            self.apply_mask_out(out, ctx, effects);
        }
        // 遮罩开着就一直醒着问光标：鼠标跨屏时指针事件只到鼠标底下那一扇遮罩，主窗收不到
        // 任何事件，不自己叫帧就永远停在第一帧算出的那块亮屏。这一段只在截图期间存在
        // （遮罩一关 `capturing()` 就假），每帧问光标的成本与停靠那条路相同（D-16/D-22）。
        ctx.request_repaint();
    }

    /// 遮罩那四颗 + esc 的落点。**五个入口汇到同一个出口**（REQ-105），
    /// 而"确认"这一件事只有一条路：裁 → 才第一次写盘。
    fn apply_mask_out(
        &mut self,
        out: crate::system::selection::MaskOut,
        ctx: &egui::Context,
        effects: &mut Effects,
    ) {
        use crate::system::selection::MaskOut;
        match out {
            // 确认三兄弟：都带"哪块屏 + 那个框"，主窗不再猜（上一版猜过一次就串了屏）
            MaskOut::Confirm { display, rect } => {
                tracing::info!("遮罩：对勾确认");
                if self.focus_capture(display, rect) {
                    self.confirm_capture();
                }
            }
            MaskOut::ConfirmCopy { display, rect } => {
                tracing::info!("遮罩：复制");
                if self.focus_capture(display, rect) {
                    let path = self.confirm_capture();
                    if let Some(path) = path {
                        effects.copy_shot = Some(path);
                    }
                }
            }
            MaskOut::ConfirmSave { display, rect } => {
                tracing::info!("遮罩：保存");
                if self.focus_capture(display, rect) {
                    if let Some(path) = self.confirm_capture() {
                        let name = path
                            .file_name()
                            .map(|n| n.to_string_lossy().to_string())
                            .unwrap_or_else(|| "截图.png".to_string());
                        if crate::system::panel::request_save(
                            &self.save_tx,
                            ctx,
                            &name,
                            &crate::system::panel::SAVE_TYPES,
                        ) {
                            self.pending_save = PendingSave::Shot(path);
                        } else {
                            self.state.toast(
                                Tone::Error,
                                "这台机器弹不出系统保存面板；可以用「复制」再粘到别处",
                            );
                        }
                    }
                }
            }
            MaskOut::Cancel => {
                tracing::info!("遮罩：取消");
                self.state
                    .end_capture(crate::state::CaptureStage::Cancelled);
            }
        }
    }

    /// 把"哪块屏 + 那个框"记进这次截图，好让裁剪知道去哪张图上裁。
    /// 屏序号找不到就是遮罩与冻结帧对不上号 —— 那必须说，不许悄悄裁第一块屏。
    fn focus_capture(&mut self, display: u8, rect: egui::Rect) -> bool {
        let Some(capture) = self.state.capture.as_mut() else {
            return false;
        };
        let Some(index) = capture.frames.iter().position(|f| f.display == display) else {
            tracing::warn!("遮罩交来的屏序号不在这次的冻结帧里");
            return false;
        };
        capture.screen = index;
        let Some(field) = capture.field() else {
            return false;
        };
        let sel = crate::system::selection::Selection::from_rect(&field, rect);
        // 选区与"它归哪块屏"只在状态层这一处写，测试走的也是同一个入口
        self.state.apply_selection(index, sel);
        true
    }

    /// 确认 = 裁 → **到这里才第一次写盘**（REQ-102：确认之前盘上不许有东西）。
    /// 裁不出来就说那一句并关掉遮罩——留一扇截不出东西的遮罩在屏上比一次失败更糟。
    fn confirm_capture(&mut self) -> Option<std::path::PathBuf> {
        let dir = self.shots_dir.clone()?;
        let Some(image) = self.state.crop_selection() else {
            tracing::warn!("确认时裁不出图：遮罩没收到选区（拖动没进来的话就是这一格）");
            return None;
        };
        let seq = self.state.capture_seq + 1;
        match capture::save_shot(&dir, &image, seq) {
            Ok(path) => {
                self.state.capture_seq += 1;
                // 确认即收尾：像素从状态里撤掉、遮罩那几扇窗跟着关，再走 CHG-009 那条钉图链
                // （REQ-104）。**「对勾」与「固定」是同一件事**（用户 2026-11-09 晚原话），
                // 确认这一刻就钉上，不留"已确认还没钉"那一格（上限已去掉，钉图必成）
                self.state.pin_confirmed_capture(path.clone());
                self.age_shots();
                Some(path)
            }
            Err(kind) => {
                let text = capture::Outcome::Failed(kind)
                    .explain()
                    .unwrap_or("这张没存下来")
                    .to_string();
                self.state
                    .end_capture(crate::state::CaptureStage::Failed(text.clone()));
                self.state.toast(Tone::Error, text);
                None
            }
        }
    }

    fn draw_pins(&mut self, ctx: &egui::Context, effects: &mut Effects) {
        if self.state.pins.is_empty() {
            return;
        }
        // 主窗这一帧读一次可用区（走 NSScreen）并缓存，供各贴图的子回调现取药丸位置——
        // 免每帧每张各走一次原生（D-31~D-35 那一族的教训）
        if let Some(area) = screen::desktop(ctx) {
            crate::system::pill::cache_screen(ctx, area);
        }
        // 首用提示只挂在**第一张**贴图的药丸上（REQ-005：第一次钉上那一张弹一次，钉第二张不再弹）。
        // 提示是全局一格状态（`AppState.pin_hint`），不属于某张 Pin；这里按"状态在 Showing"算出
        // 这一帧的视图（勾没勾），循环里只发给第一张。药丸与贴图窗都不读 AppState。
        let first_id = self.state.pins.first().map(|p| p.id);
        let hint_view = match self.state.pin_hint {
            crate::state::PinHint::Showing { dont_show_again } => Some(dont_show_again),
            _ => None,
        };
        for pin in self.state.pins.clone() {
            let hint = if Some(pin.id) == first_id {
                hint_view
            } else {
                None
            };
            // 贴图窗只画图片那块；带子展开时它这一帧的回调把自己位置现推给药丸窗
            crate::system::pin::show(ctx, &pin, hint);
            // 进度话由状态算出来再传给那扇药丸窗（贴图与药丸都不读 AppState，不存全局进度）。
            // 收起那一帧起不再 show ⇒ egui 删掉药丸窗，不需单独发关闭命令
            if pin.band_open {
                crate::system::pill::show(ctx, pin.id, pin_band_line(&pin), hint);
                // 两扇窗都建出来、且药丸**已被摆到过一次正确位置**之后，才用原生 `addChildWindow:`
                // 把药丸绑成贴图的子窗——绑那一刻抓的是相对偏移，没摆对就绑会把错偏移锁死。
                // 之后系统按固定偏移带着药丸走，拖动零延迟（治跟随的根本办法）。
                if !crate::system::pill::is_attached(ctx, pin.id)
                    && crate::system::pill::is_positioned(ctx, pin.id)
                    && crate::system::attach::attach_pill_to_pin(pin.id)
                {
                    crate::system::pill::set_attached(ctx, pin.id);
                }
            } else {
                // 带子收起 = 药丸窗没了；先解绑（窗真被销毁时这句是空操作），再清掉"已绑/已摆位"标记，
                // 重开那扇新药丸时重新摆位、重新绑
                crate::system::attach::detach_pill_from_pin(pin.id);
                crate::system::pill::clear_attached(ctx, pin.id);
                crate::system::pill::clear_positioned(ctx, pin.id);
            }
            for intent in crate::system::pin::take_intents(ctx, pin.id) {
                self.apply_pin_intent(pin.id, intent, effects);
            }
        }
        // 首用提示那两条意图（勾选翻一下 / 知道了）走**全局**槽，落到状态层就地做完——
        // 它们只改 `pin_hint` 与那一个配置 bool，不碰剪贴板/网络/原生面板，所以不进 Effects。
        for intent in crate::system::pill::take_hint_intents(ctx) {
            match intent {
                crate::system::pill::HintIntent::ToggleDontShow => {
                    self.state.toggle_pin_hint_dont_show();
                }
                crate::system::pill::HintIntent::Ack { dont_show_again } => {
                    self.state.ack_pin_hint(dont_show_again);
                }
            }
        }
    }

    /// 一张贴图发出来的意图落到**既有**通道上（本包不开第二条实现）。
    ///
    /// 关这张与收放带子是纯状态的事，就地做完；复制、保存、翻译只出意图——它们要碰
    /// 剪贴板、原生面板和网络，那些都在 `run_effects` 里，视图这一层一个字都不碰系统。
    fn apply_pin_intent(
        &mut self,
        id: u64,
        intent: crate::system::pin::PinIntent,
        effects: &mut Effects,
    ) {
        use crate::system::pin::PinIntent;
        match intent {
            PinIntent::Close => {
                self.state.close_pin(id);
            }
            PinIntent::CopyImage => effects.copy_pin = Some(id),
            PinIntent::SaveImage => effects.save_pin = Some(id),
            PinIntent::ToggleBand => {
                let open = self
                    .state
                    .pins
                    .iter()
                    .find(|p| p.id == id)
                    .is_some_and(|p| !p.band_open);
                self.state.set_pin_band(id, open);
            }
            // 右键那颗阴影勾选：只翻状态，真正下到原生窗由贴图窗那一帧幂等重下
            PinIntent::ToggleShadow => {
                self.state.toggle_pin_shadow(id);
            }
            // T-007 的前半：这颗按下才走链（识别 → 翻译 → 回填 → 成品原地替换）
            // 这颗按下走链：整条链（翻译→回填→成品）在 T-007 后半接，这一格先把意图
            // 原样带出去——**不在这里吞掉它**，吞掉的意图会让矩阵缺一格而没人发现
            PinIntent::Translate => effects.translate_pin = Some(id),
            // 文字层那一次拖选收尾（REQ-005）。空表也要写：它就是"取消选中"
            PinIntent::Select(ids) => {
                self.state.set_pin_selection(id, ids);
            }
            // 裸 `C` 的分派点。判据只有这一条：这张自己有没有选中文字。
            // 两条出路各回一条**既有**接缝（字走 `effects.copy`、图走 `effects.copy_pin`），
            // 不开第三条剪贴板实现——剪贴板那条通道上已经栽过一次"两个落点"（REQ-003）。
            PinIntent::Copy => {
                let selected = self
                    .state
                    .pins
                    .iter()
                    .find(|pin| pin.id == id)
                    .and_then(|pin| pin.selection_text());
                match selected {
                    Some(text) => effects.copy = Some(text),
                    None => effects.copy_pin = Some(id),
                }
            }
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
        // 带子里那颗「翻译」按下才走链（REQ-001 主干）。派发不出去时**不补一句空话**：
        // 带子里那一格本来就在写这张自己的状态（识别中 / 未发现文字 / 翻好了几块），
        // 再说一句"现在不能翻"只会与它对不上。
        if let Some(pin) = effects.translate_pin {
            if let Some(job) = self.state.submit_pin_translate(pin) {
                self.runner.submit(&self.runtime, job, ctx);
            }
        }
        // 「框选贴图」那颗与全局截图热键**落同一格**（形状核查也锁这一条：派发只这一处）。
        // CHG-013 改口 CHG-009 REQ-010：发起框选**不再收起自家窗**（用户 2026-11-13
        // "不用将我们的软件收起/透明"）。自家窗全程保持原样、遮罩盖在其上；可见则按当前
        // 不透明状态进冻结帧（登记为已知限制，不想被拍由用户自行管理可见性）。
        if effects.capture {
            // N0 时间轴的起点：白屏落在哪一段，靠这几个毫秒点对差值
            tracing::info!(
                t_ms = crate::system::probe::now_ms(),
                "截图：这一帧派发抓屏任务"
            );
            // 从这一刻起开一段"看帧停顿"的窗口：平时空闲几秒不重画是正常的，只有这一段里的停顿才是线索
            self.probe_span_until =
                Some(crate::system::probe::now_ms() + crate::system::probe::SPAN_MS);
            // 新轮开始：把上一轮可能残留的"F3 请确认"清掉，免得这一轮刚框好就被上一次的触发误钉。
            crate::system::selection::clear_confirm_request(ctx);
            // 屏列表只能由帧循环读（`NSScreen` 要在主线程）：抓哪几块、每块多大，
            // 都在这一下定下来交给任务，回信再按它配对（CHG-010 REQ-102"每块屏一扇遮罩"）。
            let screens = capture_screens(self);
            if let Some(job) = self.state.submit_capture(screens) {
                // CHG-013：不再收起自家窗，直接派发；收尾叫醒帧循环由子进程那条线程做。
                self.runner.submit(&self.runtime, job, ctx);
            }
        }
        // 一条派发口：贴图们"译文落好了、欠一张成品"的合成都走这里。
        // 成品的派发不写在回信落地里，是为了让 `apply_outcome` 只做状态（REQ-005 分层）
        for job in effects
            .job
            .into_iter()
            .chain(self.state.take_due_pin_composes())
        {
            self.runner.submit(&self.runtime, job, ctx);
        }
        // 「框选贴图」那颗与全局截图热键**落这一格**（同一个 `Effects::capture`）：
        // 派发只在这一处，两条入口不可能走偏成两套行为。
        if effects.rebind_hotkeys && crate::system::global_hotkeys::SUPPORTED {
            if let Err(failure) = crate::system::global_hotkeys::apply(&self.state.config.hotkey) {
                self.state.toast(Tone::Error, failure.explain());
            }
        }
        // 回信那三件事的另外两件：跑一次历史老化（截图真的回来了才跑，不开定时器）
        if self.state.capture_applied.take().is_some() {
            self.age_shots();
        }
        // 第三件：给"刚建好的每张贴图"派一次**只识别**。三种来源（截图回信、「固定」、
        // 「打开上一次」）都汇到这同一个出口，所以"钉上去之后做什么"只有一份代码
        for pin in std::mem::take(&mut self.state.pins_new) {
            if let Some(job) = self.state.submit_pin_recognize(pin) {
                self.runner.submit(&self.runtime, job, ctx);
            }
        }
        // 贴图那两条出路（CHG-009 REQ-003/004）：**复用上面那两条同一把函数**，
        // 只换像素来源——从这张自己的文件读回（`pin::pixels_of`），长期留一份 RGBA 是六张
        // 几百 MB，而复制/保存都是用户显式点一下，一次读盘换不来第四条任务那套机制。
        if let Some(path) = effects.copy_shot {
            let written = image::ImageReader::open(&path)
                .ok()
                .and_then(|r| r.decode().ok())
                .map(|img| clipboard::write_image(&img.to_rgba8()));
            match written {
                Some(Ok(())) => {
                    self.state.toast(Tone::Info, clipboard::COPY_IMAGE_DONE);
                    // 遮罩已经关了，焦点没处可还：这一步之后把应用藏起来，
                    // 让键盘回到他刚才那个应用。**这一格是实测缺口**：hide 之后
                    // 前台是不是原来那个应用，今天没量成（见 REQ-102/W-6.6），
                    // 所以只做一次、不写成"一定还得回去"
                    crate::system::cover::step_back();
                }
                Some(Err(reason)) => self.state.toast(Tone::Error, reason),
                None => self.state.toast(Tone::Warn, "这张读不回来，复制没做成"),
            }
        }
        if let Some(id) = effects.copy_pin {
            let written = self
                .state
                .pins
                .iter()
                .find(|pin| pin.id == id)
                .and_then(crate::system::pin::displayed)
                .map(|display| clipboard::write_image(display.image()));
            match written {
                Some(Ok(())) => self.state.toast(Tone::Info, clipboard::COPY_IMAGE_DONE),
                Some(Err(reason)) => self.state.toast(Tone::Error, reason),
                None => self.state.toast(Tone::Warn, PIN_GONE_TEXT),
            }
        }
        if let Some(id) = effects.save_pin {
            let default_name = crate::system::pin::default_save_name(id);
            // 面板要的是"把名字交出去"，落盘等回信：这一支与图片页那条共用同一个回信通道，
            // 差别只在回信来时该存哪一份像素（见 `pending_save`）
            if !panel::request_save(&self.save_tx, ctx, &default_name, &panel::SAVE_TYPES) {
                self.state.toast(
                    Tone::Error,
                    "这台机器弹不出系统保存面板；可以用「复制」再粘到别处",
                );
            } else {
                self.pending_save = PendingSave::Pin(id);
            }
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
                    UiMode::Compact => "已切到精简窗口",
                },
            );
        }
        if effects.open_settings {
            self.open_settings(ctx);
        }
        if effects.toggle_topmost {
            self.toggle_topmost(ctx);
        }
        if effects.toggle_fill {
            self.toggle_fill(ctx);
        }
        if effects.clear_shots {
            // 只删截图，不动文本历史；返回的是**真删掉的张数**——提示条说"清了 37 张"
            // 比"已清除"有用得多（用户由此知道原来攒了多少）
            match self.shots_dir.clone() {
                Some(dir) => {
                    let removed = capture::clear_all(&dir);
                    // 盘上的图没了，F2 游标也得归零：不然锚点还停在旧图上，清完再截、
                    // 按 F2 会从旧位置接着翻甚至翻不动（用户 2026-11-16 真机报的那一次）
                    self.state.browse_anchor = None;
                    let line = if removed == 0 {
                        "截图历史已经是空的".to_string()
                    } else {
                        format!("已清除 {removed} 张截图")
                    };
                    self.state.toast(Tone::Info, line);
                }
                None => self
                    .state
                    .toast(Tone::Error, "找不到截图历史所在的位置，没能清除"),
            }
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

/// 保存面板回来那条路径 → 一次落盘（REQ-008）。返回要给用户看的那句话与它的颜色。
///
/// 写成**自由函数、只借看 `AppState`**，为的是让两件事在形状上就成立，而不是靠记得：
/// ① 写失败时成品仍在内存——它从头到尾没拿到 `&mut AppState`，谈不上清空识别结果或译文，
///    重试入口就是那颗还在原地的「保存」；
/// ② 这一段能无头断言（`IceApp` 要 eframe 的运行时才造得出来，函数不要）。
///
/// 路径按 Q-005 收口（没写扩展名补 `.png`），格式仍由扩展名判；成功那句里**不带路径**，
/// 与 `export` 那两句失败文案同一条口径（NFR-3：路径会进日志，那是用户内容）。
/// 「保存」面板回信时，该存哪一份像素。
///
/// 面板是异步的：从"点了保存"到"选了位置"之间可能隔几秒，这期间用户可能又点了别张贴图的
/// 保存，也可能把那张关掉了。所以**来源要在按下那一刻定下来**，回信时按那一份去存。
#[derive(Debug, Clone, PartialEq, Eq, Default)]
enum PendingSave {
    /// 没有面板在等回信
    #[default]
    None,
    /// 某张贴图（此刻屏上显示的那一张）
    Pin(u64),
    /// 遮罩里刚确认的那一张：已经在历史目录里了，这里只是**再复制一份**到用户挑的位置。
    /// 为什么不重画一遍：那份像素已经落过盘，重画会多出第二个"写文件"的地方（REQ-104）
    Shot(std::path::PathBuf),
}

/// 贴图没了的时候要说的那一句：这时候盘上那个文件不是屏幕上他刚看到的东西，
/// 说"已保存"是骗人。
const PIN_GONE_TEXT: &str = "那张贴图已经不在了，这张没保存";

/// 存一张贴图：**同一把 `export::save`、同一套格式判定**，只有像素来源不同。
/// 一张贴图带子里那一行该说的话：识别段优先（"识别中…"、"未发现文字"），
/// 其次翻译段（"翻译中 3/7"、"翻好 5 块，失败 2 块"）。两段都空闲就不说——
/// 带子本来就窄，没事的时候不许占着那一格说话。
/// 要点名抓哪几块屏、每块屏多大。**每次截图前重新读**，不缓存：
/// 用户改分辨率今天实测过两次（1920×1200 → 2240×1400，位置也从 `(-1920,-120)` 变成
/// `(-2240,-320)`），缓存一帧就会把遮罩开在一块已经不存在的屏上。
///
/// 屏序号按 `NSScreen` 列表的位置从 1 编（`-D1` 是主屏），并用**像素尺寸回核**绑定
/// （见 `features/CHG-010/design.md` 那节追加）：两屏像素尺寸完全相同时这一核对分辨不出来，
/// 那种情况下宁可只盖主屏并说一句，也不猜。
fn capture_screens(_app: &IceApp) -> Vec<crate::state::ScreenSpec> {
    // 无头测试里没有真 `NSScreen` 可问，也不该让测试去猜这台机器插了几块屏：
    // 屏列表由测试喂（与 `set_capture_runner` 同一条理由）
    #[cfg(test)]
    if let Some(seeded) = &_app.test_screens {
        return seeded.clone();
    }
    let probed = crate::system::screen::probe().unwrap_or_default();
    // 翻坐标系要主屏高度，而主屏就是列表第一块（`NSScreen.screens[0]`）。
    // 只在这里翻一次：抬层用 AppKit 那份、认窗用左上原点那份，两份都跟着屏规格走。
    let main_height = probed.first().map_or(0.0, |screen| screen.frame.h);
    probed
        .into_iter()
        .enumerate()
        .map(|(index, screen)| {
            let appkit = egui::Rect::from_min_size(
                egui::pos2(screen.frame.x, screen.frame.y),
                egui::vec2(screen.frame.w, screen.frame.h),
            );
            crate::state::ScreenSpec {
                display: index as u8 + 1,
                origin: crate::system::screen::flip(screen.frame, main_height).min,
                appkit,
            }
        })
        .collect()
}

/// 带子右半边那一格：这张自己的进度，加一句"现在按 `C` 会得到什么"（REQ-005 末段）。
///
/// 进度那条**识别段优先占位**——识别没回来时说"可选文字"是假话（REQ-005 原话）。
/// 那一句 `C` 只在文字层真长出来之后才说；识别失败或"未发现文字"时文字层缺席，
/// 这一格就一个字都不多说，免得用户按出一张图还以为按坏了。
fn pin_band_line(pin: &crate::state::Pin) -> Option<String> {
    let mut parts: Vec<String> = Vec::new();
    if !matches!(pin.stage, crate::state::RecognitionStage::Idle) {
        parts.push(pin.stage.label());
    }
    if !matches!(pin.translate_stage, crate::state::TranslateStage::Idle) {
        parts.push(pin.translate_stage.label());
    }
    if pin.text_ready() {
        // 这句要短：带子只有 34pt 高、宽度跟着图走，窄图上长句会被裁掉。
        // "怎么拖选"那句挪到悬停提示里（`pin::show` 那一处），这里只说"按 C 会给什么"。
        parts.push(if pin.selected.is_empty() {
            "按 C 复制这张图".to_string()
        } else {
            format!("已选 {} 段 · 按 C 复制文字", pin.selected.len())
        });
    }
    if parts.is_empty() {
        None
    } else {
        Some(parts.join(" · "))
    }
}

/// 遮罩里那张冻结帧的纹理。**按 (截图序号, 屏序号) 缓存**：一张一块屏就是几十 MB 的像素，
/// 每帧重传会把这一秒的成本抬起来；换一次截图（新 id）自然作废，不留到下一轮。
fn freeze_texture(
    ctx: &egui::Context,
    frame: &crate::state::Freeze,
    frozen_id: u64,
) -> egui::TextureId {
    let (w, h) = (frame.image.width(), frame.image.height());
    // 缓存认的是**这一轮冻结的号**，不是像素尺寸。同一块屏每轮抓出来尺寸都一样，
    // 拿尺寸当内容的凭证就会把上一轮的画面留下来（缺陷 D-41：遮罩里那颗钟停在 20:28，
    // 而这一刻是 20:51；裁出去的是新像素，屏幕上却是旧的——用户框的是他看不见的东西）。
    // 一轮一次重传，不是每帧一次：`frozen_id` 不变时下面那条命中就走完。
    let key = egui::Id::NULL
        .with("ice-selection.freeze")
        .with(frame.display);
    if let Some((old_gen, old)) = ctx.data(|d| d.get_temp::<(u64, egui::TextureHandle)>(key)) {
        if old_gen == frozen_id {
            return old.id();
        }
    }
    // 这里要的是**四个一组的字节**，不是 Color32 数组——那个签名挨着的 `from_rgb`
    // 也一样，传错编译不过（探针那一轮我就是传错 uv 量出一个假结论，同族的错）
    let at = std::time::Instant::now();
    let mut bytes = Vec::with_capacity((w * h * 4) as usize);
    for pixel in frame.image.pixels() {
        bytes.extend_from_slice(&pixel.0);
    }
    let image = egui::ColorImage::from_rgba_unmultiplied([w as usize, h as usize], &bytes);
    let texture = ctx.load_texture("ice-selection.freeze", image, egui::TextureOptions::LINEAR);
    let id = texture.id();
    // N0 读数：这一轮这块屏的纹理准备耗时。它是主线程上的活，白屏若卡在这里会直接显出来。
    tracing::debug!(
        screen = frame.display,
        ms = at.elapsed().as_millis() as u64,
        px = (w * h) as u64,
        t_ms = crate::system::probe::now_ms(),
        "冻结帧纹理已备好"
    );
    // 键里不带轮号：新一轮把旧那一格**顶掉**，纹理才不会一轮攒一张
    ctx.data_mut(|d| d.insert_temp(key, (frozen_id, texture)));
    id
}

/// 把历史目录里那一份**原样复制**到用户挑的位置。
/// 不重画一遍：像素已经在盘上了，再走一次编码就是第二个"写文件"的地方（REQ-104）。
fn save_shot_copy(source: &std::path::Path, picked: &str) -> (Tone, String) {
    match std::fs::copy(source, picked) {
        Ok(_) => (Tone::Info, "已保存这张截图".to_string()),
        Err(error) => (Tone::Error, format!("保存失败：{error}")),
    }
}

fn save_pin_image(state: &AppState, id: u64, picked: &str) -> (Tone, String) {
    let Some(pin) = state.pins.iter().find(|pin| pin.id == id) else {
        return (Tone::Warn, PIN_GONE_TEXT.to_string());
    };
    let Some(display) = crate::system::pin::displayed(pin) else {
        // 文件被老化删了或读不开：与"没有成品"同一档，别假装写成功了
        return (Tone::Warn, PIN_GONE_TEXT.to_string());
    };
    let path = panel::resolved_save_path(picked);
    let format = export::format_of(&path);
    match export::save(display.image(), &path, format) {
        Ok(()) if format == export::Format::Jpeg && export::has_transparency(display.image()) => (
            Tone::Warn,
            "已保存成 JPEG；透明的地方按白色合成了（JPEG 没有透明通道）".to_string(),
        ),
        Ok(()) => (Tone::Info, "已保存这张贴图".to_string()),
        Err(error) => (Tone::Error, error.to_string()),
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

/// 原生整窗 alpha 走不通的警告只发一次（回退本身每帧都要重试，见 `needs_native_update`）。
static ALPHA_FALLBACK_WARNED: std::sync::atomic::AtomicBool =
    std::sync::atomic::AtomicBool::new(false);

/// 标题栏绿键接不上只发一次警告（`apply_zoom_hook`；接不上时每帧都会再试一次）
static ZOOM_UNHOOKED_WARNED: std::sync::atomic::AtomicBool =
    std::sync::atomic::AtomicBool::new(false);

/// 标题栏双击拦不住也只发一次警告（`apply_dblclick_hook`；监视器每帧都会再试装一次）
static DBLCLICK_UNWATCHED_WARNED: std::sync::atomic::AtomicBool =
    std::sync::atomic::AtomicBool::new(false);

/// 这一次双击是不是空操作（`toggle_fill` 的判定核心，抽出来才能无头断言）。
///
/// `filling` 是这一次判完之后**要不要把窗口标成铺满**，`target` 是即将下发的内尺寸。
/// 还原一侧永远不空：把窗口送回槽里记着的那组几何就是它的目的，量对不对都由它兜。
/// 铺满只在"槽里记的已经是这个整屏尺寸"时空转——典型成因是上一轮铺满被原生拒绝、护栏在
/// D-10 第二段把整屏采纳进了槽。此时命令发出去窗口一动不动，`state.fill` 却成了 `Some`、
/// `capture` 跟着停写槽，下一次双击就变成"还原到一个整屏的槽"，用户看不到任何反应。
fn fill_is_no_op(filling: bool, target: egui::Vec2, cfg: &UiConfig) -> bool {
    filling && target == mode::target_size(cfg, UiMode::Full)
}

/// 这一档**整窗属性**要不要真的下发给窗口服务器（`apply_native_alpha` 与
/// `apply_native_background` 共用的判定核心，抽出来才能无头断言）。
///
/// `applied` 是上一帧**成功**下到的值，`None` 表示还没下过或上次没走通。相同就跳过：
/// `setAlphaValue:` / `setBackgroundColor:` 一旦每帧都发，AppKit 每帧都会把整个窗口标脏
/// 重合成一遍，空闲 CPU 直接顶穿 NFR-2 的 1% 上限——悬浮窗停在桌面上不动时也应该几乎不耗电。
fn needs_native_update<T: PartialEq>(applied: Option<T>, want: T) -> bool {
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
    /// CHG-011 三格透明开关里的**清屏那一格**（事实 19）：eframe 默认返回 `(12,12,12,180)`，
    /// 带七成不透明，而 wgpu 那条路对**每一扇窗**都用这一份——不覆写，药丸那道缝里就是"七成
    /// 不透明的深灰混在桌面上"那一层暗纱（探针第 3 档读到 `(89,6,2)`）。覆成全透明之后，主窗与
    /// 贴图窗的实心**只能靠每帧铺满不透明底色**保证（`apply_native_background` 那一格），
    /// 这正是停止条件 8 要逐点量的东西——不许假设。
    fn clear_color(&self, _visuals: &egui::Visuals) -> [f32; 4] {
        [0.0; 4]
    }

    /// 窗口收起来之后 eframe 只调这一个钩子（见 `watch_visibility`）；可见帧里它同样会跑
    /// （`epi_integration.rs:357`，就在 `ui()` 之前），所以"不管窗口在不在都得盯着"的事归这里。
    fn logic(&mut self, ctx: &egui::Context, frame: &mut eframe::Frame) {
        self.watch_visibility(ctx);
        self.poll_clipboard(ctx, clipboard::read);
        // 托盘的泵挂在这一条钩子上，**不能挂在 `ui()`**：窗口不可见时 eframe 走的是 `!show_ui`
        // 那一支（`glow_integration.rs:708`），`ui()` 一次都不会被调。挂在 `ui()` 里等于"只在
        // 窗口已经看得见的时候才监听唤醒入口"——收进托盘之后那一点永远没人取，左键唤不回、
        // 菜单四项点了也不动（D-21，用户 2026-11-04："关闭后，点击托盘还是没反应"）。
        let scale = frame
            .winit_window()
            .map_or(1.0, |window| window.scale_factor());
        let mut effects = Effects::default();
        self.pump_hotkeys(ctx, &mut effects);
        self.pump_tray(scale, tray::take_outcome(), ctx, &mut effects);
        self.run_effects(ctx, effects);
        // 浮空提示活着且主窗收着：每帧补一次"隐藏也画"的请求，并按淡出步长续期。
        // RequestPaintWhileHidden 画完一帧就被 eframe 消费掉，所以必须每帧重发——
        // 只发一次的话第二帧起又回到"不跑 pass"，浮窗就停在那儿不淡也不收。
        if self.state.hidden && self.state.float_hint.is_some() {
            ctx.send_viewport_cmd(egui::ViewportCommand::RequestPaintWhileHidden);
            ctx.request_repaint_after(state::TOAST_FADE_STEP);
        }
    }

    fn ui(&mut self, ui: &mut egui::Ui, frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        theme::apply(&ctx, &self.state.config.ui);
        // 整窗底色（含原生标题栏那条带子）跟着**这一帧生效的那套主题**走：`ui.visuals()` 就是
        // egui 解析完 `theme_preference` 之后真正在用的那套，与内容区同一个出处、同一个时刻
        self.apply_native_background(frame, ui.visuals().dark_mode);
        // CHG-011：进程级透明让主窗也变半透明（事实 20），把**根窗**单独发回不透明。
        // 只在需要时发（首帧 + 换形态换窗后），不每帧骚扰。药丸那扇独立窗不受这条影响，仍浮空。
        if self.root_opaque_pending {
            ctx.send_viewport_cmd(egui::ViewportCommand::Transparent(false));
            self.root_opaque_pending = false;
        }

        jobs::drain(&mut self.outcomes, &mut self.state, &self.history);
        if let Ok(probe) = self.key_probe.try_recv() {
            self.apply_key_probe(probe);
        }
        self.poll_save();

        let focus_in_source = self.state.focus_in_source;
        let actions = ctx.input(|i| hotkeys::action_of(i, focus_in_source));
        // 窗口关闭按钮：Cmd+Q 之后不再拦截，其余情况一律收进后台（FR-5.2）
        let close_requested = ctx.input(|i| i.viewport().close_requested());

        let mut effects = Effects::default();
        self.poll_titlebar_click(&mut effects);
        self.poll_dblclick(&mut effects);
        for action in actions {
            self.run(action, &ctx, &mut effects);
        }

        if close_requested && !self.state.quitting {
            self.hide(&ctx);
        }

        self.refresh_geometry(&ctx);
        self.trace_geometry(&ctx);
        self.trace_window(&ctx, frame);
        self.settle_drag(&ctx, || screen::desktop(&ctx), mouse::left_button_down);
        self.watch_dock(&ctx, || screen::desktop(&ctx));
        self.rescue_offscreen(&ctx, |outer| screen::work_area(&ctx, outer));

        let ai_available = self.ai_available();
        // 双击标题栏的改道只属于全量窗：形态在这里，判定也只在这一句
        self.apply_dblclick_hook(frame, &ctx, self.state.mode() == UiMode::Full);
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

        // 贴图窗（CHG-009 REQ-003/004）：**每帧为每一张活着的贴图各 show 一次**。
        // 只在第一帧调一次的写法实测会翻车（事实 25）：窗口下一帧就被 egui 以
        // `it was never used this pass` 删掉。反过来，"关掉一张"的实现就是这一帧起不再 show 它。
        self.draw_capture(&ctx, &mut effects);
        self.draw_pins(&ctx, &mut effects);

        // 浮空提示：先 tick（到期就收），再 show（活着才画，None 那一帧窗被 egui 撤掉）。
        self.state.tick_float_hint();
        crate::system::hint::show(&ctx, self.state.float_hint.as_ref());

        self.run_effects(&ctx, effects);
        self.renew_repaints(&ctx);
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
        let Job::Translation {
            id,
            engine,
            request,
        } = &job
        else {
            panic!("文本页的 Translate 只会派发文本任务");
        };
        assert_eq!(*engine, EngineKind::Machine);
        assert_eq!(request.text, "hello");
        assert_eq!(app.state.in_flight, Some(*id));
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

    /// 关窗去哪儿由 `state.tray` 定，两支都要"进程不退、内容原样"（TC-5.4-c）。
    /// 唤回走的是同一个 `run` 分派：托盘那枚图标发的 `Action::RevealWindow` 与这里同一条路。
    #[test]
    fn hide_window_keeps_the_process_and_the_content() {
        for (tray, tucked_away) in [
            (TrayState::Available, egui::ViewportCommand::Visible(false)),
            (
                TrayState::Unavailable,
                egui::ViewportCommand::Minimized(true),
            ),
        ] {
            let dir = tempfile::tempdir().unwrap();
            let mut state = plain_state();
            state.source = "别丢我".into();
            state.tray = tray;
            let mut app = app(dir.path(), state);
            let c = ctx();
            let mut effects = Effects::default();

            app.run(Action::HideWindow, &c, &mut effects);

            let commands = c.viewport(|vp| vp.commands.clone());
            assert!(commands.contains(&egui::ViewportCommand::CancelClose));
            assert!(
                commands.contains(&tucked_away),
                "{tray:?} 少发了 {tucked_away:?}"
            );
            assert!(app.state.hidden);
            assert!(!app.state.quitting, "隐藏不是退出");
            assert_eq!(app.state.source, "别丢我", "收起来不许动内容一个字");

            app.run(Action::RevealWindow, &c, &mut effects);
            assert!(!app.state.hidden, "{tray:?} 这一档唤不回来");
            let commands = c.viewport(|vp| vp.commands.clone());
            assert!(
                !commands.contains(&egui::ViewportCommand::Close),
                "收起来与唤回全程都不该发 Close"
            );

            // 已经可见时再来一次是空操作：那一下常常只是用户想确认软件还在，
            // 重发几何会把刚铺满的窗口缩回去
            let before = c.viewport(|vp| vp.commands.len());
            app.run(Action::RevealWindow, &c, &mut effects);
            assert_eq!(
                c.viewport(|vp| vp.commands.len()),
                before,
                "第二次唤回多发命令了"
            );
        }
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

    /// 数这一帧里所有 `request_repaint*`。egui 0.37 的重绘观察者每一次请求都会触发
    /// （连"不会让下次重绘提前"的那些也算），正好用来数"到底要了几帧"。
    fn repaint_counter(ctx: &egui::Context) -> std::sync::Arc<std::sync::Mutex<Vec<Duration>>> {
        let seen: std::sync::Arc<std::sync::Mutex<Vec<Duration>>> = std::sync::Arc::default();
        let sink = std::sync::Arc::clone(&seen);
        ctx.set_repaint_observer(move |info| sink.lock().unwrap().push(info.delay));
        seen
    }

    /// REQ-007 的无头那半：观众不在场时一帧都不许多要。
    /// 每续一次期 eframe 就多跑一整个 pass，提示条因此停在原样、唤回后接着淡出——
    /// 中间那几帧本来就没人看。
    #[test]
    fn a_hidden_frame_asks_for_no_repaint_even_with_a_live_toast() {
        let dir = tempfile::tempdir().unwrap();
        let mut app = app(dir.path(), plain_state());
        app.state.toast(Tone::Info, "还没淡完");
        let c = ctx();
        let seen = repaint_counter(&c);

        app.renew_repaints(&c);
        assert_eq!(
            *seen.lock().unwrap(),
            [state::TOAST_FADE_STEP],
            "可见帧按 16ms 一档续淡出，不是零延迟的忙轮询"
        );

        app.state.hidden = true;
        app.renew_repaints(&c);
        app.renew_repaints(&c);
        assert_eq!(seen.lock().unwrap().len(), 1, "隐藏期两次续期一次都不许发");
    }

    /// F2"没得翻"那两句的分流（用户 2026-11-16）：主窗看得见走 toasts，收着走浮空窗；
    /// 唤回时浮空那条收掉，免得主窗 toasts 与浮窗两扇叠着说同一句。
    #[test]
    fn f2_notices_route_by_visibility_and_reveal_drops_the_float() {
        let dir = tempfile::tempdir().unwrap();
        let mut app = app(dir.path(), plain_state());
        let c = ctx();

        app.f2_notice("还没有截图记录");
        assert_eq!(app.state.toasts.len(), 1, "可见时走主窗 toasts");
        assert!(app.state.float_hint.is_none());

        app.state.hidden = true;
        app.f2_notice("没有更早的截图了");
        assert_eq!(app.state.toasts.len(), 1, "收着时不再往主窗 toasts 里塞");
        assert_eq!(
            app.state.float_hint.as_ref().unwrap().text,
            "没有更早的截图了"
        );

        app.reveal(&c);
        assert!(app.state.float_hint.is_none(), "唤回把浮空那条收掉");
    }

    /// 托盘装上以后，唤醒只剩"点那枚图标"一条路：`watch_visibility` 整条跳过，
    /// 既不发降级档的 250ms 心跳，也不让 `RevealGuard` 靠几何与焦点猜"回来了没有"。
    /// 两条并行唤醒路径留着出的就是"点一下弹两回"（REQ-006）；降级档那条由
    /// `restoring_from_the_dock_is_noticed_by_the_logic_hook` 继续守着。
    #[test]
    fn the_tray_branch_leaves_no_second_wakeup_path() {
        let dir = tempfile::tempdir().unwrap();
        let mut app = app(dir.path(), plain_state());
        app.state.tray = TrayState::Available;
        let c = ctx();
        let mut effects = Effects::default();
        app.run(Action::HideWindow, &c, &mut effects);

        // 喂进"窗口回来了"的证据，并让静默期真的过掉：降级档在这一步就会自己弹回来，
        // 少了这句等待，这条测试删掉早退也照样绿
        feed_viewport(
            &c,
            Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::Vec2::new(1080.0, 720.0),
            )),
            Some(true),
        );
        std::thread::sleep(mode::REVEAL_GRACE + Duration::from_millis(50));
        let seen = repaint_counter(&c);
        let before = seen.lock().unwrap().len();

        app.watch_visibility(&c);

        assert!(
            app.state.hidden,
            "唤回只由托盘那一下决定，几何与焦点都只是猜测"
        );
        assert_eq!(
            seen.lock().unwrap().len(),
            before,
            "托盘档不许有心跳：隐藏期唯一的唤醒源是回调里那一次 request_repaint"
        );
    }

    /// 收起期交来的意图必须被兑现（D-21 的另一半）。
    ///
    /// 挂在 `ui()` 上的写法**在测试里看不出任何异常**——测试直接调实现，永远"看得见"，
    /// 真机上却是点了没反应：eframe 在窗口不可见时根本不调 `ui()`。所以这条钉的是
    /// `pump_tray`（`logic()` 里唯一那一句）：给它一份意图，它必须把窗口叫回来。
    /// 「设置」那一项多一条顺序要求：先唤回、再开设置页，两件都得发生。
    #[test]
    fn an_intent_handed_over_while_hidden_still_opens_the_window() {
        let dir = tempfile::tempdir().unwrap();
        let mut app = app(dir.path(), plain_state());
        app.state.tray = TrayState::Available;
        let c = ctx();
        app.run(Action::HideWindow, &c, &mut Effects::default());
        assert!(app.state.hidden, "先要真的收起来，这条测的才是收起期那一路");

        let mut effects = Effects::default();
        app.pump_tray(2.0, Some(tray::Outcome::Settings), &c, &mut effects);
        assert!(
            !app.state.hidden,
            "收起期的意图没被兑现，窗口停在看不见的那一侧"
        );
        assert!(effects.open_settings, "菜单「设置」没把设置页开起来");

        // 左键那一支只唤回，不许顺手开设置页
        app.run(Action::HideWindow, &c, &mut Effects::default());
        let mut effects = Effects::default();
        app.pump_tray(2.0, Some(tray::Outcome::Reveal), &c, &mut effects);
        assert!(!app.state.hidden, "左键唤回没兑现");
        assert!(!effects.open_settings, "左键不该开设置页");
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
        let mut app = without_real_capture(IceApp::new(
            state,
            runtime,
            translator,
            HistoryStore::at(dir.path().join("history.jsonl")),
            dir.path().join("config.toml"),
            Some(lock),
            idle_key_probe(),
            Some(dir.path().join("shots")),
        ));

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

    /// 每个测试构造出来的 `IceApp` 都过这一手：框选的执行体换成桩。
    ///
    /// `screencapture -i` 会把全屏选区拍到屏幕上并**阻塞等人画完**——我引进过一条断言
    /// 让它真跑起来了，全量测试从 4 秒变成 348 秒，还可能在你屏幕上闪过一次选区。
    /// 所以这不是"测试想要快"，而是测试不该有权限操作你的屏幕。
    fn without_real_capture(mut app: IceApp) -> IceApp {
        app.runner
            .set_capture_runner(|_path, _seq| capture::Outcome::Cancelled);
        // 冻结那一支同样要接缝：默认给"两块屏"的形状（本机实测：主屏 + 左置那块），
        // 于是无头测试既不真抓屏、也不依赖测试机到底插了几块屏
        app.test_screens = Some(vec![
            crate::state::ScreenSpec {
                display: 1,
                appkit: egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1920.0, 1080.0)),
                origin: egui::Pos2::ZERO,
            },
            crate::state::ScreenSpec {
                display: 2,
                appkit: egui::Rect::from_min_size(
                    egui::pos2(-2240.0, -320.0),
                    egui::vec2(2240.0, 1400.0),
                ),
                origin: egui::pos2(-2240.0, 0.0),
            },
        ]);
        app.runner.set_freeze_runner(|_path, _seq, display| {
            Some(crate::state::Freeze {
                display,
                points: egui::vec2(1920.0, 1080.0),
                appkit: egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1920.0, 1080.0)),
                origin: egui::Pos2::ZERO,
                image: std::sync::Arc::new(image::RgbaImage::from_pixel(
                    8,
                    4,
                    image::Rgba([1, 2, 3, 255]),
                )),
            })
        });
        app
    }

    fn app_with_history(dir: &std::path::Path, state: AppState, history: HistoryStore) -> IceApp {
        let translator = build_translator(&state.config, None);
        // 测试构造出的每一个 `IceApp` 都过这一手：截图目录指到临时目录、框选执行体换成桩。
        // 老化与清除都会**真删文件**，指到系统里那个真实目录就等于跑一条断言删掉用户截过的图；
        // 而真框选会把全屏选区拍到屏幕上等人画完（见 `JobRunner::set_capture_runner`）
        without_real_capture(IceApp::new(
            state,
            runtime(),
            translator,
            history,
            dir.join("config.toml"),
            None,
            idle_key_probe(),
            Some(dir.join("shots")),
        ))
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

        let mut app = without_real_capture(IceApp::new(
            state,
            runtime(),
            translator,
            HistoryStore::at(dir.path().join("history.jsonl")),
            dir.path().join("config.toml"),
            None,
            idle_key_probe(),
            Some(dir.path().join("shots")),
        ));

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
        without_real_capture(IceApp::new(
            state,
            runtime(),
            translator,
            HistoryStore::at(dir.join("history.jsonl")),
            dir.join("config.toml"),
            None,
            idle_key_probe(),
            Some(dir.join("shots")),
        ))
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

        let mut app = without_real_capture(IceApp::new(
            state,
            runtime(),
            translator,
            HistoryStore::at(dir.path().join("history.jsonl")),
            dir.path().join("config.toml"),
            None,
            idle_key_probe(),
            Some(dir.path().join("shots")),
        ));
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
            app.settle_drag(&c, || Some(area()), || None);
        }
        assert!(!app.drag_settler.is_armed(), "窗口停着不算拖动");

        // 推到离右边缘 10px 处（阈值 40）：没有 pointer 事件、没有 StartDrag，只有位置变了
        let near_edge = Rect::from_min_size(Pos2::new(area().max.x - SIZE.x - 10.0, 400.0), SIZE);
        feed_geometry(&c, Some(near_edge), Some(true));
        app.settle_drag(&c, || Some(area()), || None);
        assert!(
            app.drag_settler.is_armed(),
            "原生标题栏的拖动没人 arm，吸附与停靠就一次都不会跑"
        );

        std::thread::sleep(mode::DRAG_SETTLE + std::time::Duration::from_millis(30));
        feed_geometry(&c, Some(near_edge), Some(true));
        app.settle_drag(&c, || Some(area()), || None);
        assert_eq!(app.state.config.ui.docked_edge, DockEdge::Right);
    }

    /// D-16 的锁：拖到边上**按住不放**不许收，松手之后才收。
    ///
    /// 2026-10-28 用户原话"虽然我将其拖动到边上，但我还没有松手，他就着急往里面收入，
    /// 导致我只要一直不松手，鼠标和界面的距离会越来越大"。故障的形状是：几何停住 150ms
    /// 就被当成松手，吸附把窗口从指针底下搬走，而 AppKit 的拖拽还在跟手。
    /// 这里两段几何完全一样，唯一变量是 `held_for` 说没说"还按着"。
    #[test]
    fn a_held_drag_stays_put_until_the_button_is_released() {
        let dir = tempfile::tempdir().unwrap();
        let mut app = app(dir.path(), compact_state());
        let c = ctx();
        let free = Rect::from_min_size(Pos2::new(600.0, 400.0), SIZE);
        for _ in 0..=STILL_BEFORE_DRAG {
            feed_geometry(&c, Some(free), Some(true));
            app.settle_drag(&c, || Some(area()), || None);
        }

        let near_edge = Rect::from_min_size(Pos2::new(area().max.x - SIZE.x - 10.0, 400.0), SIZE);
        feed_geometry(&c, Some(near_edge), Some(true));
        app.settle_drag(&c, || Some(area()), || Some(true));
        assert!(app.drag_settler.is_armed(), "拖到边上要开始等松手");

        // 停住超过两个静止阈值，但**手还按着**：一个结论都不许下
        std::thread::sleep(mode::DRAG_SETTLE * 2);
        for _ in 0..3 {
            feed_geometry(&c, Some(near_edge), Some(true));
            app.settle_drag(&c, || Some(area()), || Some(true));
        }
        assert_eq!(
            app.state.config.ui.docked_edge,
            DockEdge::None,
            "还没松手就收，窗口会从指针底下滑走"
        );
        assert!(
            !queued(&c)
                .iter()
                .any(|cmd| matches!(cmd, ViewportCommand::OuterPosition(_))),
            "按住期间不许下发任何挪窗命令"
        );
        assert!(
            app.drag_settler.is_armed(),
            "按住不动期间判定要一直挂着，且每帧续重绘"
        );

        // 松手后第一帧：整段按住期间没喂过几何（按住那几帧直接 return 了），这一帧只用来
        // 把松手处的几何立成参照，所以还不该收——延迟就是松手后约一个 `DRAG_SETTLE`。
        feed_geometry(&c, Some(near_edge), Some(true));
        app.settle_drag(&c, || Some(area()), || Some(false));
        assert_eq!(
            app.state.config.ui.docked_edge,
            DockEdge::None,
            "松手的第一帧只立参照，不该当场收"
        );

        // 参照之后停满一个阈值：照常吸附+停靠
        std::thread::sleep(mode::DRAG_SETTLE + std::time::Duration::from_millis(30));
        feed_geometry(&c, Some(near_edge), Some(true));
        app.settle_drag(&c, || Some(area()), || Some(false));
        assert_eq!(
            app.state.config.ui.docked_edge,
            DockEdge::Right,
            "松手了就必须收，不能因为问过硬件就永远不吸附"
        );
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
            app.settle_drag(&c, || Some(area()), || None);
        }

        let shown = Rect::from_min_size(Pos2::new(SHOWN_X, 400.0), SIZE);
        app.place_compact(&c, shown.min, DockEdge::Right);
        feed_geometry(&c, Some(shown), Some(true));
        app.settle_drag(&c, || Some(area()), || None);
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
            app.settle_drag(&c, || Some(area()), || None);
        }
        let dragged = Rect::from_min_size(Pos2::new(SHOWN_X - 120.0, 400.0), SIZE);
        feed_geometry(&c, Some(dragged), Some(true));
        app.settle_drag(&c, || Some(area()), || None);
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
                app.settle_drag(&c, || Some(area()), || None);
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

    /// NFR-2 的护栏：整窗 alpha 与整窗底色每帧都发一次系统调用，AppKit 就每帧重合成整个窗口。
    /// 悬浮窗绝大多数时间是"停在桌面上没人碰"，那一档不能变，所以同值必须跳过。
    /// 上次没走通（`None`）要重试——第一帧拿不到窗口句柄是正常时序，不是失败终局。
    #[test]
    fn an_unchanged_window_property_is_not_resent() {
        assert!(
            !needs_native_update(Some(0.85), 0.85),
            "同一档重复下发，空闲帧全在付重合成的钱"
        );
        assert!(
            !needs_native_update(Some(1.0), 1.0),
            "全量窗整场都是 100%，一次都不该发"
        );
        assert!(
            needs_native_update(Some(0.85), 1.0),
            "焦点回来了必须写回 100%，不然窗体永远停在半透明"
        );
        assert!(
            needs_native_update(None, 0.85),
            "从没成功下过（含首帧拿不到句柄）要重试，不能永久退回"
        );
        // 底色走的是同一条判定：深浅切换要重写一次，同档反复核对一个字都不发
        assert!(
            needs_native_update(
                Some(crate::theme::window_base(false)),
                crate::theme::window_base(true)
            ),
            "主题翻了标题栏还是旧底色，色差就又回来了"
        );
        assert!(
            !needs_native_update(
                Some(crate::theme::window_base(false)),
                crate::theme::window_base(false)
            ),
            "同一套主题每帧重发底色，等于每帧把整窗标脏"
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

    /// 任务 #25（用户"双击标题栏：铺满整屏 + 再双击还原"）：原生监视器的那一笔必须落到
    /// `effects.toggle_fill` 上，而且**一次双击只兑现一次**——与 `poll_titlebar_click`
    /// 同一条纪律。取走标记是无条件的：窗口收起时不兑现，也不许留给下一帧。
    #[test]
    fn a_title_bar_double_click_queues_the_fill_effect_once() {
        let dir = tempfile::tempdir().unwrap();
        let mut app = app(dir.path(), plain_state());

        dblclick::note_double_click();
        let mut effects = Effects::default();
        app.poll_dblclick(&mut effects);
        assert!(effects.toggle_fill, "双击没接进 effects，标题栏白挨一下");

        let mut second = Effects::default();
        app.poll_dblclick(&mut second);
        assert!(
            !second.toggle_fill,
            "同一次双击兑了两次：铺满和还原互相抵消"
        );

        dblclick::note_double_click();
        app.state.hidden = true;
        let mut hidden = Effects::default();
        app.poll_dblclick(&mut hidden);
        assert!(!hidden.toggle_fill, "窗口收起来了还铺什么满");
        assert!(
            !dblclick::take_double_click(),
            "标记要一并清掉，否则窗口回来那一帧凭空铺满一次"
        );
    }

    /// 喂一帧铺满判定要的输入：内框、外框（两者之差就是这套装饰的厚度，本机实测 32pt）、
    /// 以及整块屏的尺寸。无头测试里 `screen::probe()` 一定拿不到屏幕列表（不在主线程），
    /// 可用区走的正是 `fallback_area` 那条近似（顶部内缩 25pt，Dock 拿不到所以不缩底）。
    fn feed_window(ctx: &egui::Context, inner: Rect, outer: Rect, monitor: egui::Vec2) {
        let info = egui::ViewportInfo {
            inner_rect: Some(inner),
            outer_rect: Some(outer),
            focused: Some(true),
            monitor_size: Some(monitor),
            ..Default::default()
        };
        let mut raw = egui::RawInput::default();
        raw.viewports.insert(egui::ViewportId::ROOT, info);
        ctx.run_ui(raw, |_| {}).drop_without_applying_deltas();
    }

    /// 1440×900 的屏上，一个 1080×720（外框 752 高）的全量窗被双击：
    /// 要铺到可用区 (0,25)-(1440,900)，也就是内尺寸 1440×(875-32)=1440×843。
    /// 护栏同时上膛——还原那一帧量到的还是整屏，不拦就被写进 `full_rect`（D-6 换个入口）。
    #[test]
    fn a_double_click_fills_the_work_area_and_arms_the_guard() {
        let dir = tempfile::tempdir().unwrap();
        let mut app = app(dir.path(), plain_state());
        let c = ctx();
        feed_window(
            &c,
            Rect::from_min_size(Pos2::new(120.0, 110.0), egui::Vec2::new(1080.0, 720.0)),
            Rect::from_min_size(Pos2::new(120.0, 78.0), egui::Vec2::new(1080.0, 752.0)),
            egui::Vec2::new(1440.0, 900.0),
        );

        let effects = Effects {
            toggle_fill: true,
            ..Effects::default()
        };
        app.run_effects(&c, effects);

        let commands = queued(&c);
        assert!(
            commands.contains(&ViewportCommand::InnerSize(egui::Vec2::new(1440.0, 843.0))),
            "没减掉 32pt 装饰条，或者铺的是整块 frame 而不是可用区：{commands:?}"
        );
        assert!(
            commands.contains(&ViewportCommand::OuterPosition(Pos2::new(0.0, 25.0))),
            "原点要落在可用区的顶，菜单栏下面：{commands:?}"
        );
        assert!(app.state.fill.is_some());
        assert!(
            app.pending_geometry.is_pending(),
            "铺满的目标没进护栏，还原时拦不住实时尺寸"
        );
        assert_eq!(
            (
                app.state.config.ui.full_rect.w,
                app.state.config.ui.full_rect.h
            ),
            (1080.0, 720.0),
            "铺满当帧就把实时尺寸写进了槽"
        );
    }

    /// D-24（用户 2026-11-04："双击放大界面还不是全屏"）：铺满没铺到位时护栏要照样量、
    /// 照样重发，而**重发的必须是当初那块可用区**。上一版 `capture` 在铺满期间整个函数
    /// 直接返回（为了不写槽），于是连测量一起跳过——命令被原生砍掉一截没人发现，也不会自愈；
    /// 而还原那一支反倒每次都精确落位，两条路只有一条有护栏就是这里的成因。
    #[test]
    fn a_fill_that_lands_short_is_replayed_with_the_same_area() {
        let dir = tempfile::tempdir().unwrap();
        let mut app = app(dir.path(), plain_state());
        let c = ctx();
        let area = Rect::from_min_size(Pos2::new(0.0, 25.0), egui::Vec2::new(1440.0, 875.0));
        let target = egui::Vec2::new(1440.0, 843.0);
        app.state.fill = Some(area);
        app.pending_geometry.set(target, Instant::now());
        // 现场：宽铺满了，高度只到 661——正是要被重发的那种"矮了一截"
        feed_window(
            &c,
            Rect::from_min_size(Pos2::new(0.0, 57.0), egui::Vec2::new(1440.0, 661.0)),
            Rect::from_min_size(Pos2::new(0.0, 25.0), egui::Vec2::new(1440.0, 693.0)),
            egui::Vec2::new(1440.0, 900.0),
        );
        app.pending_geometry
            .set_stage_clock_for_tests(Instant::now() - mode::MODE_GEOMETRY_GRACE, false);

        app.refresh_geometry(&c);

        let commands = queued(&c);
        assert!(
            commands.contains(&ViewportCommand::InnerSize(target)),
            "第一段宽限期到期没重发尺寸：{commands:?}"
        );
        assert!(
            commands.contains(&ViewportCommand::OuterPosition(area.min)),
            "重发要铺回当初那块可用区，拿配置里铺满之前的位置会把窗口拽回原处：{commands:?}"
        );
        assert_eq!(
            (
                app.state.config.ui.full_rect.w,
                app.state.config.ui.full_rect.h
            ),
            (1080.0, 720.0),
            "护栏照量，但铺满期间实时尺寸一个字都不许进槽"
        );
    }

    /// 再双击一次还原：发的是槽里记着的那组几何（不是可用区、也不是当前实时尺寸），
    /// 并且不顺带重发装饰与下限。
    #[test]
    fn a_second_double_click_gives_the_remembered_rect_back() {
        let dir = tempfile::tempdir().unwrap();
        let mut app = app(dir.path(), plain_state());
        let c = ctx();
        let rect = app.state.config.ui.full_rect;
        app.state.fill = Some(Rect::from_min_size(
            Pos2::new(0.0, 25.0),
            egui::Vec2::new(1440.0, 875.0),
        ));
        // 实时几何是"已经铺满"的那一档：还原请求发出去之后它还没变，正是护栏要挡的那几帧
        feed_window(
            &c,
            Rect::from_min_size(Pos2::new(0.0, 57.0), egui::Vec2::new(1440.0, 843.0)),
            Rect::from_min_size(Pos2::new(0.0, 25.0), egui::Vec2::new(1440.0, 875.0)),
            egui::Vec2::new(1440.0, 900.0),
        );

        app.toggle_fill(&c);

        let commands = queued(&c);
        assert!(
            commands.contains(&ViewportCommand::InnerSize(egui::Vec2::new(rect.w, rect.h))),
            "还原发的不是槽里那组尺寸：{commands:?}"
        );
        assert!(commands.contains(&ViewportCommand::OuterPosition(Pos2::new(rect.x, rect.y))));
        assert!(
            !commands.contains(&ViewportCommand::Decorations(true)),
            "铺满没改过装饰，还原不该顺带重发：{commands:?}"
        );
        assert!(app.state.fill.is_none(), "还原完就不该还说自己是铺满的");
        assert!(
            app.pending_geometry.is_pending(),
            "还原也是一组异步请求，护栏要跟着上（D-6）"
        );
    }

    /// 上一轮铺满被原生拒绝、护栏在 D-10 第二段把整屏尺寸采纳进了槽之后，再双击"铺满"
    /// 是空操作：命令发出去窗口一动不动，`state.fill` 却成了 `Some`、`capture` 跟着停写槽，
    /// 下一次双击就变成"还原到一个整屏的槽"，用户看到的是第一下没反应。
    #[test]
    fn filling_to_the_size_already_recorded_is_refused() {
        let already_filled = UiConfig {
            full_rect: WindowRect {
                x: 0.0,
                y: 25.0,
                w: 1440.0,
                h: 843.0,
            },
            ..UiConfig::default()
        };
        let target = egui::Vec2::new(1440.0, 843.0);
        assert!(fill_is_no_op(true, target, &already_filled));
        assert!(
            !fill_is_no_op(true, target, &UiConfig::default()),
            "槽里记的还是常规尺寸，这一刀要照做"
        );
        assert!(
            !fill_is_no_op(false, target, &already_filled),
            "还原永远不是空操作：送回槽里的几何就是它的目的"
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

    /// 置顶的两条入口共用同一条路径：界面那枚图钉（`views::pin_toggle`，2026-10-29 从原生
    /// 标题栏挪进工具条）写的是 `effects.toggle_topmost`，`Cmd+Shift+P` 走
    /// `Action::ToggleTopmost`，两条最后都落到 `toggle_topmost` 这一个方法上：
    /// 写回配置 + 下发 WindowLevel + 一条提示，谁点的都一样。
    #[test]
    fn the_pin_effect_and_the_hotkey_share_one_path() {
        let dir = tempfile::tempdir().unwrap();
        let mut app = app(dir.path(), compact_state());
        let c = ctx();

        // 图钉那一格点击在 view 侧置的就是这个字段
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
        assert_eq!(
            queued(&c)
                .iter()
                .filter(|cmd| matches!(cmd, ViewportCommand::WindowLevel(_)))
                .count(),
            1,
            "一次点击下发了不止一次层级，置顶本身要只有一个出处"
        );

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

    /// 一张带真实像素文件的贴图（保存两条出路的共同前置）。`alpha` 用来造"带透明"与
    /// "整张不透明"两种图，那句 JPEG 提示只该在前一种上出现。临时目录要跟着状态一起活着。
    fn with_pin_picture(alpha: u8) -> (AppState, tempfile::TempDir) {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("pin.png");
        image::RgbaImage::from_pixel(8, 8, image::Rgba([255, 255, 255, alpha]))
            .save(&file)
            .unwrap();
        let mut state = plain_state();
        state.push_pin(file, None);
        (state, dir)
    }

    /// REQ-008 的失败那一档：给一句说明原因的话、**来源仍在**、重试入口就是那颗按钮。
    /// 写失败只回一句话，不碰状态——贴图还在屏上，才有的重试。
    #[test]
    fn a_failed_save_names_the_problem_and_keeps_everything() {
        let (state, dir) = with_pin_picture(255);

        // 父目录不存在（磁盘/权限那一档）。**不能拿临时目录本身当靶子**：它没有扩展名，
        // 按 Q-005 会被补成 `<临时目录>.png`——那是它旁边的一个新文件，写得成。
        let missing = dir.path().join("没有这一层").join("成品.png");
        let picked = missing.to_string_lossy().to_string();
        let (tone, text) = save_pin_image(&state, 1, &picked);
        assert_eq!(tone, Tone::Error, "写不出去却说成了别的档");
        assert!(
            text.contains(export::SAVE_IO_TEXT),
            "那句话没说明原因：{text}"
        );
        assert!(!text.contains(&picked), "文案里带出了路径：{text}");
        assert!(!missing.exists(), "失败了却不许留下半个文件");
        assert_eq!(state.pins.len(), 1, "写失败却把贴图清了，那就没得重试");
    }

    /// 面板回来那条路径落盘的就是屏上那张的编码字节（补扩展名那一档在
    /// `panel::resolved_save_path` 自己的测试里钉着，这里不重复）。
    #[test]
    fn a_save_lands_on_the_picked_path_as_the_encoded_displayed_picture() {
        let (state, dir) = with_pin_picture(255);

        let named = dir.path().join("成品.png");
        let (tone, text) = save_pin_image(&state, 1, &named.to_string_lossy());
        assert_eq!(tone, Tone::Info);
        assert_eq!(text, "已保存这张贴图", "{text}");
        let displayed = crate::system::pin::displayed(&state.pins[0]).expect("屏上得有图");
        assert_eq!(
            std::fs::read(&named).unwrap(),
            export::encode(displayed.image(), export::Format::Png).unwrap(),
            "落盘的不是编码出来的那一份字节"
        );
    }

    /// JPEG 那句提示只在**真有透明**时出现（Q-005）：整张不透明时提它是吓人，
    /// 而真带透明时不提，用户拿到的就是一张白底图却不知道为什么。
    #[test]
    fn the_jpeg_hint_appears_only_when_the_picture_has_transparency() {
        let (state, dir) = with_pin_picture(128);
        let picked = dir.path().join("成品.jpg").to_string_lossy().to_string();

        let (tone, text) = save_pin_image(&state, 1, &picked);
        assert_eq!(tone, Tone::Warn, "带透明存成 JPEG 该提一句");
        assert!(
            text.contains("白色") && text.contains("JPEG"),
            "那句话没说清透明的地方去了哪儿：{text}"
        );

        let (opaque, dir) = with_pin_picture(255);
        let picked = dir.path().join("不透明.jpg").to_string_lossy().to_string();
        let (tone, text) = save_pin_image(&opaque, 1, &picked);
        assert_eq!(tone, Tone::Info);
        assert_eq!(text, "已保存这张贴图", "整张不透明却提了透明：{text}");
    }

    /// 存一张贴图：走的是**同一把** `export::save`，只是像素来自这张自己的文件。
    /// 贴图没了/文件读不开都说同一句"不在了"——不许假装写成功。
    #[test]
    fn saving_a_pin_writes_the_same_export_seam_and_refuses_a_gone_one() {
        let dir = tempfile::tempdir().unwrap();
        let mut state = plain_state();
        let shot = dir.path().join("shot.png");
        image::RgbImage::from_pixel(8, 6, image::Rgb([3, 4, 5]))
            .save(&shot)
            .unwrap();
        state.push_pin(shot, None);

        let out = dir.path().join("out.png");
        let (tone, text) = save_pin_image(&state, 1, out.to_str().unwrap());
        assert_eq!(tone, Tone::Info, "{text}");
        assert!(out.exists(), "文件没落盘却说已保存：{text}");
        assert_eq!(text, "已保存这张贴图");

        // 号不在了：不写文件、说一句"没了"，不是"已保存"
        let gone = dir.path().join("gone.png");
        let (tone, text) = save_pin_image(&state, 99, gone.to_str().unwrap());
        assert_eq!(tone, Tone::Warn);
        assert_eq!(text, PIN_GONE_TEXT);
        assert!(!gone.exists());

        // 贴图还在但文件被老化删了：同一档，不造第二套失败话
        let mut orphan = plain_state();
        let missing = dir.path().join("aged-out.png");
        orphan.push_pin(missing.clone(), None);
        let (tone, text) =
            save_pin_image(&orphan, 1, dir.path().join("never.png").to_str().unwrap());
        assert_eq!(tone, Tone::Warn);
        assert_eq!(text, PIN_GONE_TEXT);
    }

    /// 带子那一格要说清"现在按 `C` 会得到什么"，但**只在文字层真长出来之后说**
    /// （REQ-005：识别没回来时说"可选文字"是假话；识别失败时一个字都不许多说）。
    #[test]
    fn the_band_says_what_c_will_give_and_only_after_the_text_layer_grew() {
        let mut state = plain_state();
        state.push_pin(std::path::PathBuf::from("/tmp/a.png"), None);
        let pin = state.pins.first_mut().unwrap();

        pin.stage = crate::state::RecognitionStage::Recognizing;
        let line = pin_band_line(pin).expect("识别中要说一句");
        assert!(line.contains("识别中"), "{line}");
        assert!(
            !line.contains("按 C"),
            "识别在飞时不许说按 C 会得到什么：{line}"
        );

        pin.document = Some(crate::system::pin::probe_document());
        pin.stage = crate::state::RecognitionStage::Done { blocks: 2 };
        let line = pin_band_line(pin).unwrap();
        assert_eq!(
            line, "完成 2 块 · 按 C 复制这张图",
            "带子那格要短，长句在窄图上会被裁：{line}"
        );

        pin.selected = vec![0, 1];
        let line = pin_band_line(pin).unwrap();
        assert_eq!(line, "完成 2 块 · 已选 2 段 · 按 C 复制文字", "{line}");

        pin.selected = Vec::new();
        pin.stage = crate::state::RecognitionStage::NoText;
        let line = pin_band_line(pin).unwrap();
        assert!(
            !line.contains("按 C"),
            "这张没出字就不许留一句会用错的提示：{line}"
        );
    }

    /// 裸 `C` 的分派点：**有选中走文本那条接缝，没选中走图像那条**。
    /// 两条都是既有的（`effects.copy` / `effects.copy_pin`），这里不开第三条剪贴板实现。
    #[test]
    fn bare_c_dispatches_between_text_and_image_by_selection() {
        let dir = tempfile::tempdir().unwrap();
        let mut state = plain_state();
        state.push_pin(dir.path().join("a.png"), None);
        {
            let pin = state.pins.first_mut().unwrap();
            pin.stage = crate::state::RecognitionStage::Done { blocks: 2 };
            pin.document = Some(crate::system::pin::probe_document());
        }
        let mut app = app(dir.path(), state);

        let mut none = Effects::default();
        app.apply_pin_intent(1, crate::system::pin::PinIntent::Copy, &mut none);
        assert_eq!(none.copy_pin, Some(1), "没选中就是复制这张图");
        assert!(none.copy.is_none());

        app.state.set_pin_selection(1, vec![1, 0]);
        let mut picked = Effects::default();
        app.apply_pin_intent(1, crate::system::pin::PinIntent::Copy, &mut picked);
        assert_eq!(picked.copy_pin, None, "有选中时不许同时发图像那条");
        assert_eq!(
            picked.copy.as_deref(),
            Some("上面那一行\n下面那一行"),
            "倒着拖也要按阅读顺序交出去"
        );
    }

    /// 拖选收尾写进的是**这一张**的选中集合：另一张不受影响（六张贴图各说各的）。
    /// 空表是一次写入——它就是取消选中。
    #[test]
    fn a_drag_selection_lands_on_its_own_pin_only() {
        let dir = tempfile::tempdir().unwrap();
        let mut state = plain_state();
        state.push_pin(dir.path().join("a.png"), None);
        state.push_pin(dir.path().join("b.png"), None);
        state.set_pin_selection(1, vec![0, 1]);
        state.set_pin_selection(2, vec![0]);
        assert_eq!(state.pins[0].selected, vec![0, 1]);
        assert_eq!(state.pins[1].selected, vec![0]);
        let mut app = app(dir.path(), state);
        let mut effects = Effects::default();
        app.apply_pin_intent(
            1,
            crate::system::pin::PinIntent::Select(Vec::new()),
            &mut effects,
        );
        assert!(app.state.pins[0].selected.is_empty(), "取消选中要落下去");
        assert_eq!(app.state.pins[1].selected, vec![0], "另一张不受影响");
        assert!(
            effects.copy.is_none() && effects.copy_pin.is_none(),
            "拖选不碰剪贴板"
        );
    }

    /// 面板开着的那几秒里，来源要在按下那一刻定下来：回信按那一份存，
    /// 取消或面板没回音都要把它清干净——留着就等于认定"下一份是给它的"。
    #[test]
    fn the_save_source_is_decided_at_press_time_and_cleared_on_cancel() {
        let mut state = plain_state();
        state.push_pin(std::path::PathBuf::from("/tmp/a.png"), None);
        state.push_pin(std::path::PathBuf::from("/tmp/b.png"), None);
        let mut app = app(std::env::temp_dir().as_path(), state);
        assert_eq!(app.pending_save, PendingSave::None);

        // 两张先后点保存：以最后那一次为准（面板只有一张在用户手上）
        app.pending_save = PendingSave::Pin(1);
        app.pending_save = PendingSave::Pin(2);
        assert_eq!(app.pending_save, PendingSave::Pin(2));

        // 取消那一路必须归零
        app.pending_save = PendingSave::None;
        assert_eq!(app.pending_save, PendingSave::None);
    }

    /// esc 关这张贴图 = **只把它从"该被 show 的集合"里摘掉**，历史目录里那张原图
    /// 一个字都不动（REQ-004 第一条）。关掉之后再点一次不许凭空报错说"已关闭"。
    #[test]
    fn closing_a_pin_with_escape_only_drops_the_window() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("esc.png");
        image::RgbImage::from_pixel(6, 4, image::Rgb([7, 8, 9]))
            .save(&file)
            .unwrap();
        let before = std::fs::read(&file).unwrap();

        let mut state = plain_state();
        let pin = state.push_pin(file.clone(), None);
        let mut app = app(dir.path(), state);
        let mut effects = Effects::default();

        app.apply_pin_intent(pin, crate::system::pin::PinIntent::Close, &mut effects);
        assert!(app.state.pins.is_empty());
        assert_eq!(
            std::fs::read(&file).unwrap(),
            before,
            "关一张贴图动了历史文件"
        );
        // 关掉这一张不碰剪贴板也不派任何任务
        assert!(effects.copy_pin.is_none() && effects.job.is_none());

        // 再关一次：那张本来就不在了，界面上不该说"已关闭"
        let mut again = Effects::default();
        app.apply_pin_intent(pin, crate::system::pin::PinIntent::Close, &mut again);
        assert!(again.copy_pin.is_none());
    }

    /// 空格只收放这一张的带子，别把别人的也收了（多张同时在屏是合法情况）。
    #[test]
    fn the_space_key_toggles_the_band_of_that_pin_only() {
        let dir = tempfile::tempdir().unwrap();
        let mut state = plain_state();
        for name in ["a.png", "b.png"] {
            state.push_pin(dir.path().join(name), None);
        }
        let mut app = app(dir.path(), state);
        let mut effects = Effects::default();

        app.apply_pin_intent(2, crate::system::pin::PinIntent::ToggleBand, &mut effects);
        assert!(!app.state.pins[1].band_open, "第二张的带子没收起来");
        assert!(app.state.pins[0].band_open, "第一张被牵连了");
    }

    /// 托盘「清除历史截图」那一项落到的动作：当帧**不自己删、不自己弹新框**，而是唤回 +
    /// 借"打开设置页"把确认框落到能画它的全量窗宿主上 + 把设置页那颗**已有**的
    /// `Confirm::ClearShots` 二次确认弹上来。真删要等用户点确认走 `clear_shots`，没点之前一张都不许动。
    #[test]
    fn tray_clear_shots_arms_the_existing_confirm_instead_of_deleting_directly() {
        let dir = tempfile::tempdir().unwrap();
        let shots = dir.path().join("shots");
        std::fs::create_dir_all(&shots).unwrap();
        std::fs::File::create(shots.join("a.png")).unwrap();

        let mut app = app(dir.path(), plain_state());
        let mut effects = Effects::default();
        app.run(Action::ClearShots, &ctx(), &mut effects);

        assert_eq!(
            app.state.confirm,
            Some(state::Confirm::ClearShots),
            "托盘清除要先弹那个已有的二次确认"
        );
        assert!(
            effects.open_settings,
            "确认框只在全量窗画，得借开设置页把它落到宿主上"
        );
        assert!(shots.join("a.png").exists(), "没点确认之前一张都不许删");
    }

    /// 「清除所有截图」删的是真的文件，报的是**真删掉的张数**，而且**不动文本历史**
    /// （两个"历史"是两个东西，一键误删另一个是事故）。
    #[test]
    fn clearing_shots_removes_the_files_and_leaves_the_text_history_alone() {
        let dir = tempfile::tempdir().unwrap();
        let shots = dir.path().join("shots");
        std::fs::create_dir_all(&shots).unwrap();
        for name in ["1.png", "2.png", "3.png"] {
            std::fs::File::create(shots.join(name)).unwrap();
        }
        let text_history = dir.path().join("history.jsonl");
        std::fs::write(&text_history, b"some record").unwrap();

        let mut app = app(dir.path(), plain_state());
        let effects = Effects {
            clear_shots: true,
            ..Default::default()
        };
        let ctx = ctx();
        app.run_effects(&ctx, effects);

        assert_eq!(
            std::fs::read_dir(&shots).unwrap().count(),
            0,
            "三张截图没被清掉"
        );
        assert!(std::fs::exists(&text_history).unwrap(), "文本历史被牵连了");
        let toasts: Vec<String> = app
            .state
            .toasts
            .iter()
            .map(|toast| toast.text.clone())
            .collect();
        assert!(
            toasts.iter().any(|line| line.contains("3 张")),
            "提示条要说清删了几张：{toasts:?}"
        );
    }

    /// 清截图需要把 F2 游标一起归零（用户 2026-11-16 真机报"清完再截 F2 不管用"）：
    /// 盘上的图删了，锚点还停在旧路径上，下一次 F2 会从旧位置接着翻甚至翻不动。
    /// 清除之后游标必须回 `None`＝从最新重新起翻。
    #[test]
    fn clearing_shots_also_resets_the_f2_browse_cursor() {
        let dir = tempfile::tempdir().unwrap();
        let shots = dir.path().join("shots");
        std::fs::create_dir_all(&shots).unwrap();
        std::fs::File::create(shots.join("1.png")).unwrap();

        let mut app = app(dir.path(), plain_state());
        app.state.browse_older(shots.join("1.png"));
        assert!(app.state.browse_anchor.is_some(), "先真的把游标停在一张上");

        let effects = Effects {
            clear_shots: true,
            ..Default::default()
        };
        app.run_effects(&ctx(), effects);

        assert_eq!(
            std::fs::read_dir(&shots).unwrap().count(),
            0,
            "本地文件要真删掉"
        );
        assert_eq!(
            app.state.browse_anchor, None,
            "清完游标不归零，下一次 F2 还会卡在旧位置"
        );
    }

    /// 空历史也要有话说（"已经是空的"），而不是静悄悄——静悄悄会被当成"清除没生效"。
    #[test]
    fn an_empty_shot_history_says_so_instead_of_staying_silent() {
        let dir = tempfile::tempdir().unwrap();
        let shots = dir.path().join("shots");
        std::fs::create_dir_all(&shots).unwrap();
        let mut app = app(dir.path(), plain_state());
        let effects = Effects {
            clear_shots: true,
            ..Default::default()
        };
        app.run_effects(&ctx(), effects);
        let lines: Vec<String> = app
            .state
            .toasts
            .iter()
            .map(|toast| toast.text.clone())
            .collect();
        assert!(
            lines.iter().any(|line| line.contains("空的")),
            "一张都没有时也要回一句：{lines:?}"
        );
    }

    /// 截图**不再收起自家窗**（CHG-013 改口 CHG-009 REQ-010，用户"不用将我们的软件收起/透明"）。
    /// 没有意图时一家窗都不收；真有意图时也一家都不收——自家窗全程保持原样，遮罩盖在其上。
    /// 这条同时给一个我自己引进过的错误锁门：把 `if effects.capture` 守卫写丢会每帧发起框选。
    #[test]
    fn capture_never_hides_our_own_window() {
        let dir = tempfile::tempdir().unwrap();
        let mut app = app(dir.path(), plain_state());
        app.state.push_pin(dir.path().join("a.png"), None);
        let c = ctx();

        app.run_effects(&c, Effects::default());
        assert_eq!(app.state.capture_stage, crate::state::CaptureStage::Idle);
        assert!(
            !queued(&c)
                .iter()
                .any(|command| matches!(command, ViewportCommand::Visible(false))),
            "没有意图却收起了窗：{:?}",
            queued(&c)
        );

        // 真有意图时也不收：CHG-013 起截图全程不收起自家窗
        let effects = Effects {
            capture: true,
            ..Default::default()
        };
        app.run_effects(&c, effects);
        assert!(
            !queued(&c)
                .iter()
                .any(|command| matches!(command, ViewportCommand::Visible(false))),
            "截图收起了自家窗：回到被改口的老路（CHG-009 REQ-010）了：{:?}",
            queued(&c)
        );
        assert_eq!(
            app.state.capture_stage,
            crate::state::CaptureStage::Capturing
        );
    }

    /// 遮罩开着、一块 40×30 的冻结帧、选区还没框。**认窗那份快照喂空的**：
    /// 真去读在线窗口表就把这台机器的窗请进测试了，而这两条要锁的事与窗口表无关。
    fn frozen_app(dir: &std::path::Path) -> IceApp {
        let mut app = app(dir, plain_state());
        app.state.capture = Some(crate::state::Capture {
            frozen_id: 1,
            frames: vec![crate::state::Freeze {
                display: 1,
                points: egui::vec2(40.0, 30.0),
                appkit: egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(40.0, 30.0)),
                origin: egui::Pos2::ZERO,
                image: std::sync::Arc::new(image::RgbaImage::from_pixel(
                    40,
                    30,
                    image::Rgba([9, 8, 7, 255]),
                )),
            }],
            screen: 0,
            sel: crate::system::selection::Selection::fresh(),
            raised: true,
            push_frames: crate::state::PUSH_FRAMES,
            windows: Some(std::sync::Arc::new(
                crate::system::windows::Snapshot::default(),
            )),
            lit: None,
        });
        app.state.capture_stage = crate::state::CaptureStage::Frozen;
        app
    }

    /// 确认之后**遮罩必须关、图必须钉上去**。这两件事我都漏过：裁完落盘就 return，
    /// 状态里的 `capture` 还在，于是屏上那扇遮罩一动不动，用户以为"点对勾没反应"
    /// （他 2026-11-09 晚原话："点了对号也不固定图片到屏幕上也不退出截图模式"）。
    /// 「对勾」与「固定」是同一件事，所以钉上去之后那一次"只识别"也要派得出去。
    #[test]
    fn confirming_closes_the_mask_and_pins_the_shot() {
        let dir = tempfile::tempdir().unwrap();
        let mut app = frozen_app(dir.path());
        assert!(app.focus_capture(
            1,
            egui::Rect::from_min_max(egui::pos2(2.0, 3.0), egui::pos2(22.0, 18.0))
        ));

        let path = app.confirm_capture().expect("确认该裁得出一张");
        assert!(path.exists(), "确认之前不许落盘，确认之后必须落盘");
        assert!(
            app.state.capture.is_none(),
            "确认之后遮罩还留着：用户会以为对勾没生效"
        );
        assert!(!app.state.capturing());
        assert_eq!(app.state.pins.len(), 1, "对勾就是固定，不许只落盘不钉");
        assert_eq!(
            app.state.pins_new.len(),
            1,
            "钉上去之后那一次只识别要派得出去（与另两条来源同一个出口）"
        );
        assert_eq!(app.state.capture_stage, crate::state::CaptureStage::Pinned);
    }

    /// 抬层/激活那两下**只在头几帧重下**，而且配额由帧循环真的减掉。
    /// 上一版帧循环读了 `push_frames > 0` 却从不减它，于是遮罩开着就每帧枚举窗口、
    /// 重设 frame，把主线程拖死（D-31~D-34）。这条锁的是**接线**，不是那个减法本身
    /// （减法自己另有 `the_native_repush_quota_is_spent_and_then_stops`）。
    #[test]
    fn the_frame_loop_spends_the_native_repush_quota() {
        let dir = tempfile::tempdir().unwrap();
        let mut app = frozen_app(dir.path());
        let c = ctx();
        let before = app.state.capture.as_ref().unwrap().push_frames;
        // 遮罩要画在真的一个 egui pass 里（deferred viewport 当场就要字体），
        // 裸 Context 上直接调会在 egui 内部 panic，那不是本条要测的事
        c.run_ui(egui::RawInput::default(), |_ui| {
            app.draw_capture(&c, &mut Effects::default());
        })
        .drop_without_applying_deltas();
        assert_eq!(
            app.state.capture.as_ref().unwrap().push_frames,
            before - 1,
            "跑了一帧却没减配额：遮罩会永远每帧重下原生命令"
        );

        // 配额用尽之后那一帧：不再减（不许回绕）
        app.state.capture.as_mut().unwrap().push_frames = 0;
        c.run_ui(egui::RawInput::default(), |_ui| {
            app.draw_capture(&c, &mut Effects::default());
        })
        .drop_without_applying_deltas();
        assert_eq!(app.state.capture.as_ref().unwrap().push_frames, 0);
    }
}
