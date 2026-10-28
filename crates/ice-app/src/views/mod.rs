//! 全量界面的窗口骨架与页签分发。
//!
//! REQ-001 的四区里，工具条（A）与状态条（D）在这里，源文（B）/译文（C）两栏在 `full.rs`；
//! E 动作底栏的画法在 `full::bottom_bar`，但**调用**也在这里——它是窗口层的一条栏，
//! 必须排在 `CentralPanel` 之前。其余三个页签各自一个文件，签名统一为
//! `show(ui, &mut AppState, &mut Effects)`。简洁悬浮窗是另一套骨架，在 `compact.rs`。

use egui::{self, Align, Align2, Layout, Order, RichText};
use ice_core::history::HistoryStore;
use ice_core::language::EngineKind;

use crate::state::{AppState, Confirm, FailureAction, Job, Tab};
use crate::theme;

pub mod compact;
pub mod full;
pub mod history;
pub mod image;
pub mod settings;

/// 工具条的上下内边距（横向沿用 egui 面板自己的 8px）。
///
/// 这条栏以前是 `exact_size(56.0)` 钉死的（`03-ui-design.md` §1 那个 56px），里面却只有
/// 一行约 20px 的控件：多出来的三十几像素全在空转，正文被整块顶下去（用户 2026-10-28
/// "明明只占用一行，但是好远"）。现在栏高按内容自适应，这一条是它唯一的高度来源，
/// 字号缩放（`ui.font_scale` 到 2.0）时栏子也跟着长，不再溢出。
///
/// 类型是 `i8` 不是 `f32`：egui 0.37 把 `Margin` 收成了整数（`epaint/src/margin.rs:44`）。
const TOOLBAR_Y_MARGIN: i8 = 6;

/// 状态条的高度（`03-ui-design.md` §1）
pub const STATUS_HEIGHT: f32 = 28.0;

/// 全量模式提示条离窗口顶部的距离（简洁窗按输入行的实际底边算，见 `compact::toast_offset`）
pub const TOAST_OFFSET: f32 = 12.0;

/// 一帧里 view 想让外部世界做的事。
///
/// view 只改 `AppState`，副作用记在这里、帧末由 `app.rs` 统一执行；这样 view 函数能无头调用，
/// 不会碰到真实剪贴板、钥匙串或网络（REQ-003/REQ-005/REQ-006 的验证层次要求）。
#[derive(Debug, Default)]
pub struct Effects {
    /// 本帧要派发的翻译任务（界面上同一时刻只允许一个）
    pub job: Option<Job>,
    /// 要写进剪贴板的文本（已按"复制译文/复制对照"组装好）；`None` 表示本帧不碰剪贴板
    pub copy: Option<String>,
    /// 要存进钥匙串的 Key 明文；只在内存里传一次，不落盘不进日志
    pub save_key: Option<String>,
    /// 要清除钥匙串条目
    pub delete_key: bool,
    /// AI 配置或 Key 变了：重建 `Translator`
    pub rebuild_translator: bool,
    /// 术语增删改：把新表推给调度层（`Translator::set_glossary`），已缓存的结果也会重新后处理
    pub sync_glossary: bool,
    /// 切到简洁模式（F-05）
    pub toggle_mode: bool,
    /// 二次确认后清空历史
    pub clear_history: bool,
    /// 简洁窗把手被按下：交原生窗口拖拽（`ViewportCommand::StartDrag`，REQ-002）
    pub start_drag: bool,
    /// 简洁窗的"设置"：切回全量并打开设置页（REQ-005）
    pub open_settings: bool,
    /// 切换置顶（K-09）。快捷键那条路径直接在 `app.rs::run` 里做，这里给界面按钮用
    pub toggle_topmost: bool,
    /// 退出程序（REQ-011）：两种形态各有一颗看得见的按钮，走的都是 `Action::Quit` 那一条路径
    pub quit: bool,
    /// 把两个尺寸槽恢复成默认值（REQ-011 落点②：尺寸被历史版本写坏了，程序不自动改，用户显式恢复）
    pub reset_geometry: bool,
}

impl Effects {
    pub fn request(&mut self, job: Option<Job>) {
        if job.is_some() {
            self.job = job;
        }
    }
}

/// `ai_available` 由调用方从配置与钥匙串状态推出来（`config.ai.is_ready() && key.configured()`），
/// 而不是每帧去锁 `Translator` 问一次。
pub fn show(
    ui: &mut egui::Ui,
    state: &mut AppState,
    effects: &mut Effects,
    history: &HistoryStore,
    ai_available: bool,
) {
    toolbar(ui, state, effects, ai_available);
    status_bar(ui, state, ai_available);
    // E 动作底栏（REQ-013）：文本页专属，建在 `CentralPanel` 之前才拿得到位置（`CentralPanel`
    // 吃掉剩余矩形）。放在窗口层而不是页内，是为了让它贴住状态条——落在 `CentralPanel` 里时，
    // 它下方还压着那层框架的 8px 内边距，按钮上下留白就差了 8px（用户 2026-10-28 报的缺陷）。
    if state.tab == Tab::Text {
        full::bottom_bar(ui, state, effects);
    }

    egui::CentralPanel::default().show(ui, |ui| match state.tab {
        Tab::Text => full::show(ui, state),
        Tab::Image => image::show(ui, state),
        Tab::History => history::show(ui, state, history),
        Tab::Settings => settings::show(ui, state, effects, ai_available),
    });

    let ctx = ui.ctx().clone();
    toasts(&ctx, state, TOAST_OFFSET);
    confirm(&ctx, state, effects);
}

/// A 区：页签（F-01）+ 语言（F-02/F-03）+ 引擎（F-04）+ 悬浮（F-05）
///
/// 与正文的分界交给 `Panel` 自己画的那条分隔线（`show_separator_line` 默认开，取
/// `widgets.noninteractive.bg_stroke`）——底栏那条走的是同一段代码、同一个颜色，两栏的
/// 边界感自然一致。这里**不再额外套一圈 stroke**：2026-10-28 加过一圈 `edge_color` 边框，
/// 用户当场退回"太明显了，突兀，和底部栏的分割线一样颜色就行，而且两侧不需要线条"。
/// 栏高囤积才是"界限不明显"的真凶，那条由 `TOOLBAR_Y_MARGIN` 管。
fn toolbar(ui: &mut egui::Ui, state: &mut AppState, effects: &mut Effects, ai_available: bool) {
    let frame = egui::Frame::side_top_panel(ui.style())
        .inner_margin(egui::Margin::symmetric(8, TOOLBAR_Y_MARGIN));
    egui::Panel::top("toolbar").frame(frame).show(ui, |ui| {
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 8.0;
            tabs(ui, state);
            ui.separator();
            source_language(ui, state);
            target_language(ui, state, effects);
            ui.separator();
            engine_picker(ui, state, effects, ai_available);
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if ui
                    .button("悬浮窗")
                    .on_hover_text("切到简洁悬浮窗（Cmd+Shift+M）")
                    .clicked()
                {
                    effects.toggle_mode = true;
                }
            });
        });
    });
}

fn tabs(ui: &mut egui::Ui, state: &mut AppState) {
    for tab in Tab::all() {
        if ui.selectable_label(state.tab == tab, tab.label()).clicked() {
            // 切页签不重建数据、不触发请求：各页状态都常驻 AppState
            state.tab = tab;
        }
    }
}

fn source_language(ui: &mut egui::Ui, state: &mut AppState) {
    let selected = state.config.translation.source;
    let languages = state.visible_languages();
    let label = selected
        .map(|lang| lang.display_name().to_string())
        .unwrap_or_else(|| "自动检测".to_string());
    egui::ComboBox::from_id_salt("source_language")
        .selected_text(label)
        .show_ui(ui, |ui| {
            if ui
                .selectable_label(selected.is_none(), "自动检测")
                .clicked()
            {
                state.set_source_language(None);
            }
            for lang in languages {
                if ui
                    .selectable_label(selected == Some(lang), lang.display_name())
                    .clicked()
                {
                    state.set_source_language(Some(lang));
                }
            }
        });
}

fn target_language(ui: &mut egui::Ui, state: &mut AppState, effects: &mut Effects) {
    let selected = state.config.translation.target;
    let languages = state.visible_languages();
    egui::ComboBox::from_id_salt("target_language")
        .selected_text(selected.display_name())
        .show_ui(ui, |ui| {
            for lang in languages {
                if ui
                    .selectable_label(selected == lang, lang.display_name())
                    .clicked()
                {
                    // 切目标语言自动重译（FR-1.4、FR-1.3）
                    effects.request(state.set_target(lang));
                }
            }
        });
}

/// 引擎分段（F-04）。全量工具条与简洁窗动作行共用同一套：`Button::selectable` 的选中态、
/// AI 未配置时禁用并给说明，简洁窗不再新造一份（REQ-005）。
///
/// 返回两个分段按钮合起来的矩形：简洁窗的布局断言要拿它确认把手没压在分段上。
pub(crate) fn engine_picker(
    ui: &mut egui::Ui,
    state: &mut AppState,
    effects: &mut Effects,
    ai_available: bool,
) -> egui::Rect {
    let mut rect = egui::Rect::NOTHING;
    for kind in EngineKind::all() {
        let enabled = kind != EngineKind::Ai || ai_available;
        let mut button = ui.add_enabled(
            enabled,
            egui::Button::selectable(state.engine == kind, kind.display_name()),
        );
        if !enabled {
            button =
                button.on_disabled_hover_text("AI 未配置：请在设置页填地址、模型并保存 API Key");
        }
        rect = rect.union(button.rect);
        if button.clicked() {
            effects.request(state.set_engine(kind));
        }
    }
    rect
}

/// D 区：状态条（F-11）。三态——就绪 / 翻译中 / 错误；错误态带"去设置"入口。
fn status_bar(ui: &mut egui::Ui, state: &mut AppState, ai_available: bool) {
    egui::Panel::bottom("status")
        .exact_size(STATUS_HEIGHT)
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                let color = if state.failure.is_some() {
                    theme::ERROR
                } else if state.in_flight.is_some() {
                    theme::WARN
                } else {
                    theme::MUTED
                };
                ui.colored_label(color, state.status_text(ai_available));
                if state.failure.as_ref().and_then(|f| f.action) == Some(FailureAction::GoSettings)
                    && ui.small_button("去设置").clicked()
                {
                    state.go_to_key_settings();
                }
            });
        });
}

/// 提示条：顶部居中，最多三条，四秒自动消失，也可以点掉。
///
/// `offset_y` 是离窗口顶部的距离：全量窗给 `TOAST_OFFSET`，简洁窗要越过输入行——
/// 那一行会随内容长高（REQ-005 的"位置不遮住输入行文字"）。
pub(crate) fn toasts(ctx: &egui::Context, state: &mut AppState, offset_y: f32) {
    if state.toasts.is_empty() {
        return;
    }
    let mut dismissed = None;
    egui::Area::new(egui::Id::unique("toasts"))
        .anchor(Align2::CENTER_TOP, [0.0, offset_y])
        .order(Order::Foreground)
        .interactable(true)
        .show(ctx, |ui| {
            ui.vertical(|ui| {
                for (index, toast) in state.toasts.iter().enumerate() {
                    egui::Frame::popup(ui.style()).show(ui, |ui| {
                        // 关闭符排在文字后面同一行：竖排会把它甩到文字下方，像个孤立的方块
                        ui.horizontal(|ui| {
                            ui.spacing_mut().item_spacing.x = 6.0;
                            ui.label(
                                RichText::new(&toast.text).color(theme::tone_color(toast.tone)),
                            );
                            // 关闭符号只用 Latin-1 的 ×：⌗ ✕ ▣ 这类装饰字形既不在 egui 内置字体里，
                            // macOS 的系统字体回退也找不到，真机上会显示成豆腐块（走查发现）
                            if ui.small_button("×").clicked() {
                                dismissed = Some(index);
                            }
                        });
                    });
                }
            });
        });
    if let Some(index) = dismissed {
        state.dismiss_toast(index);
    }
}

/// 破坏性操作的二次确认（清空历史、清除 Key）
fn confirm(ctx: &egui::Context, state: &mut AppState, effects: &mut Effects) {
    let Some(kind) = state.confirm else {
        return;
    };
    let mut confirmed = None;
    egui::Area::new(egui::Id::unique("confirm"))
        .anchor(Align2::CENTER_CENTER, [0.0, 0.0])
        .order(Order::Foreground)
        .interactable(true)
        .show(ctx, |ui| {
            egui::Frame::popup(ui.style())
                .inner_margin(16.0)
                .show(ui, |ui| {
                    ui.set_min_width(320.0);
                    ui.label(RichText::new(kind.prompt()).strong());
                    ui.add_space(12.0);
                    ui.horizontal(|ui| {
                        if ui.button(kind.confirm_label()).clicked() {
                            confirmed = Some(true);
                        }
                        if ui.button("取消").clicked() {
                            confirmed = Some(false);
                        }
                    });
                });
        });
    match confirmed {
        Some(true) => {
            state.confirm = None;
            match kind {
                Confirm::ClearHistory => effects.clear_history = true,
                Confirm::ClearKey => effects.delete_key = true,
            }
        }
        Some(false) => state.confirm = None,
        None => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::{KeyState, Tone};
    use ice_core::config::Config;
    use ice_core::glossary::Glossary;

    fn state() -> AppState {
        AppState::new(
            Config::default(),
            Glossary::default(),
            KeyState::Missing,
            None,
        )
    }

    /// 无头跑 `frames` 帧。`ctx.run_ui` 交出来的就是 eframe 在 `App::ui` 里给的那个根 `Ui`，
    /// 所以 view 的绘制路径与真机完全一致；`screen_rect` 用来覆盖窄宽布局。
    /// 控件矩形是画完那一帧才记上的，要读它至少跑两帧。
    fn run_frame(
        state: &mut AppState,
        effects: &mut Effects,
        history: &HistoryStore,
        screen: egui::Vec2,
        frames: usize,
    ) -> egui::Context {
        let ctx = egui::Context::default();
        let mut time = 0.0;
        for _ in 0..frames {
            let mut raw = egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, screen)),
                ..Default::default()
            };
            raw.time = Some(time);
            time += 1.0 / 60.0;
            let output = ctx.run_ui(raw, |ui| {
                show(ui, state, effects, history, false);
            });
            output.drop_without_applying_deltas();
        }
        ctx
    }

    fn temp_history() -> (tempfile::TempDir, HistoryStore) {
        let dir = tempfile::tempdir().unwrap();
        let store = HistoryStore::at(dir.path().join("history.jsonl"));
        (dir, store)
    }

    #[test]
    fn every_tab_renders_without_panicking_at_the_minimum_size() {
        let (_dir, store) = temp_history();
        for tab in Tab::all() {
            let mut state = state();
            state.tab = tab;
            let mut effects = Effects::default();
            run_frame(
                &mut state,
                &mut effects,
                &store,
                egui::vec2(640.0, 420.0),
                1,
            );
            assert_eq!(state.tab, tab, "渲染过程不该把页签改掉");
            assert!(effects.job.is_none(), "切到 {tab:?} 不该发起请求");
        }
    }

    /// 用户 2026-10-28 报的两条缺陷一起锁在这里，量的是 `views::show` 这条真机路径：
    /// ① "按钮上下留白距离不同"——底栏画在 `CentralPanel` 里时被那层 8px 下内边距顶离窗口
    /// 底边（实测上 7、下 15），挪到窗口层后必须贴住状态条上沿、横向满窗；
    /// ② "这颜色怎么回事"——挪走之后它下面不再有 `panel_fill` 兜着，`Frame::NONE` 就等于
    /// 在窗口上开个透出桌面的洞（eframe 默认 `clear_color` 是半透明深灰）。
    #[test]
    fn the_text_page_bottom_bar_hugs_the_status_bar() {
        let (_dir, store) = temp_history();
        let mut state = state();
        let mut effects = Effects::default();
        let screen = egui::vec2(1080.0, 720.0);
        let ctx = run_frame(&mut state, &mut effects, &store, screen, 2);
        let (bar, fill) = full::recorded_bar(&ctx);

        let status_top = screen.y - STATUS_HEIGHT;
        assert!(
            (bar.max.y - status_top).abs() <= 0.5,
            "底栏与状态条之间空出 {:.1}px，按钮的上下留白就差这么多：{bar:?}",
            status_top - bar.max.y
        );
        assert!(
            bar.min.x <= 0.5 && bar.max.x >= screen.x - 0.5,
            "底栏没横满整窗（{bar:?}），左右各被页框内边距切掉一块"
        );
        let style = ctx.global_style();
        assert_eq!(
            fill, style.visuals.panel_fill,
            "底栏没自己画装饰条的底色，那条区域会透出桌面（D-12）"
        );
        assert_eq!(fill.a(), 255, "装饰条的底色必须不透明：{fill:?}");
    }

    /// 另一件只量"画没画"的事：把最后一帧的绘制清单拿出来。
    /// 现有的 `run_frame` 只交回 `Context`（控件矩形是记在临时数据里的），而边框线是形状，
    /// 只能从 shapes 读——所以这条单独一个入口，不去改动上面那些测试。
    fn run_shapes(
        state: &mut AppState,
        effects: &mut Effects,
        history: &HistoryStore,
        screen: egui::Vec2,
    ) -> (egui::Context, Vec<egui::epaint::ClippedShape>) {
        let ctx = egui::Context::default();
        let mut time = 0.0;
        let mut shapes = Vec::new();
        for _ in 0..2 {
            let mut raw = egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, screen)),
                ..Default::default()
            };
            raw.time = Some(time);
            time += 1.0 / 60.0;
            let mut output = ctx.run_ui(raw, |ui| {
                show(ui, state, effects, history, false);
            });
            shapes = std::mem::take(&mut output.shapes);
            output.drop_without_applying_deltas();
        }
        (ctx, shapes)
    }

    /// 工具条与正文的分界**只有一条线**，而且与底栏那条同源：两者都由 `Panel` 自己画
    /// （`show_separator_line` 默认开，取 `widgets.noninteractive.bg_stroke`）。
    ///
    /// 用户 2026-10-28 对着截图退回的原话："我看到你改动的菜单栏界限了，太明显了，突兀，
    /// 和底部栏的分割线一样颜色就行，而且两侧不需要线条。"所以这条锁三件事：顶区有一条
    /// 横贯整窗的分隔线、它的颜色等于底栏那条、顶区没有第二圈边框。
    /// 前一行"界限不明显"的真凶是栏高囤积（82px），那条由 `the_toolbar_takes_one_row_...` 管。
    #[test]
    fn the_toolbar_separator_matches_the_bottom_bar_and_has_no_frame() {
        let (_dir, store) = temp_history();
        let mut state = state();
        let mut effects = Effects::default();
        let screen = egui::vec2(1080.0, 720.0);
        let (ctx, shapes) = run_shapes(&mut state, &mut effects, &store, screen);

        // 横贯整窗的水平线段：(所在高度, 线色)
        let mut lines: Vec<(f32, egui::Color32)> = shapes
            .iter()
            .filter_map(|clipped| match &clipped.shape {
                egui::Shape::LineSegment { points, stroke } => {
                    let (a, b) = (points[0], points[1]);
                    (a.y == b.y && a.x <= 0.5 && b.x >= screen.x - 0.5)
                        .then_some((a.y, stroke.color))
                }
                _ => None,
            })
            .collect();
        lines.sort_by(|a, b| a.0.total_cmp(&b.0));

        let top: Vec<_> = lines.iter().filter(|(y, _)| *y < 100.0).collect();
        assert_eq!(top.len(), 1, "工具条下沿该只有一条分隔线，实得 {lines:?}");
        let bottom: Vec<_> = lines
            .iter()
            .filter(|(y, _)| *y > screen.y - 120.0)
            .collect();
        assert!(
            !bottom.is_empty(),
            "底栏那条分隔线没找到，无法比对：{lines:?}"
        );
        assert_eq!(
            top[0].1, bottom[0].1,
            "两条分隔线颜色不同——工具条自己另画了一色"
        );
        assert_eq!(
            top[0].1,
            ctx.global_style()
                .visuals
                .widgets
                .noninteractive
                .bg_stroke
                .color,
            "分隔线必须走 egui 那一处出处，别再各配一套色"
        );

        let edge = theme::edge_color(ctx.global_style().visuals.dark_mode);
        let framed = shapes.iter().any(|clipped| match &clipped.shape {
            egui::Shape::Rect(rect) => {
                rect.stroke.color == edge
                    && rect.stroke.width >= 1.0
                    && rect.rect.min.x <= 0.5
                    && rect.rect.max.x >= screen.x - 0.5
                    && rect.rect.min.y <= 1.0
            }
            _ => false,
        });
        assert!(!framed, "工具条又套上一圈整窗边框了：两侧不该有线");
    }

    /// 用户 2026-10-28："明明只占用一行，但是好远"。以前工具条 `exact_size(56)` 钉死，
    /// 里面只有一行约 20px 的控件，正文框上沿被顶到 **82px**（实测，把 `exact_size` 加回去
    /// 跑这条就能看到）。改成按内容自适应之后是 61px：栏高 = 一行 + 上下各 6px。
    /// 这条锁的是"栏不再囤空转"，阈值给 64px：留 3px 给字形度量的零头，但 56 那一档必然过不了。
    #[test]
    fn the_toolbar_takes_one_row_so_the_panes_sit_right_under_it() {
        let (_dir, store) = temp_history();
        let mut state = state();
        let mut effects = Effects::default();
        let ctx = run_frame(
            &mut state,
            &mut effects,
            &store,
            egui::vec2(1080.0, 720.0),
            2,
        );
        let (source, _) = full::recorded_pane(&ctx, "source");
        assert!(
            source.min.y <= 64.0,
            "正文框上沿跑到 {:.1}px，工具条又囤出空转了（钉死 56px 那一版是 82px）",
            source.min.y
        );
    }

    #[test]
    fn rendering_does_not_invent_requests_or_side_effects() {
        let (_dir, store) = temp_history();
        let mut state = state();
        let mut effects = Effects::default();
        run_frame(
            &mut state,
            &mut effects,
            &store,
            egui::vec2(1080.0, 720.0),
            1,
        );
        assert!(effects.job.is_none());
        assert!(effects.copy.is_none());
        assert!(effects.save_key.is_none());
        assert!(!effects.delete_key);
        assert!(!effects.toggle_mode);
        assert!(!effects.start_drag);
        assert!(!effects.open_settings);
        assert!(!effects.toggle_topmost);
    }

    #[test]
    fn tab_state_survives_a_round_trip_through_every_tab() {
        let (_dir, store) = temp_history();
        let mut state = state();
        state.source = "hello".into();
        state.history_query = "关键词".into();
        state.term_source = "pod".into();
        let mut effects = Effects::default();

        for tab in Tab::all() {
            state.tab = tab;
            run_frame(
                &mut state,
                &mut effects,
                &store,
                egui::vec2(1080.0, 720.0),
                1,
            );
        }

        assert_eq!(state.source, "hello");
        assert_eq!(state.history_query, "关键词");
        assert_eq!(state.term_source, "pod");
        assert!(effects.job.is_none());
    }

    #[test]
    fn toast_is_queued_and_expires_on_its_own() {
        let mut state = state();
        state.toast(Tone::Info, "已复制");
        assert_eq!(state.toasts.len(), 1);
        assert!(!state.tick_toasts());
        assert_eq!(state.toasts.len(), 1);
        // 直接把时间戳拨到过期之后，免得测试真等四秒
        state.toasts[0].shown_at -= crate::state::TOAST_VISIBLE * 2;
        assert!(state.tick_toasts());
        assert!(state.toasts.is_empty());
    }

    #[test]
    fn at_most_three_toasts_are_kept() {
        let mut state = state();
        for index in 0..5 {
            state.toast(Tone::Info, format!("第 {index} 条"));
        }
        assert_eq!(state.toasts.len(), 3);
        assert_eq!(state.toasts[0].text, "第 2 条");
    }
}
