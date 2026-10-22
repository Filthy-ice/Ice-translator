//! 简洁悬浮窗（FR-3.2、REQ-002 可拖可缩、REQ-005 与全量窗同源）。
//!
//! 自上而下四块：把手条 → 输入行 → 动作行（底）→ 译文区（中央）。
//!
//! **把手为什么要显式矩形**：边框窗的标题栏是原生给的，简洁窗 `Decorations(false)` 之后
//! 一处都没有，而窗口本身又是常驻置顶的小条——用户抓不到它就只能去 Dock 里找。
//! 把手不能靠"盖在控件上面"来做：egui 的命中测试按绘制顺序取最后一个矩形，
//! 压上去的把手会把下面的按钮一起吃掉。这里改成三块**互不重叠**的空白区当把手
//! （顶部把手条、动作行左右两组按钮之间的空隙、译文正文之下的空白带），
//! 与 egui 自己的 `Window` 标题栏避开关闭按钮是同一个思路。
//!
//! 把手只用 `Sense::drag()` 而不是 `click_and_drag()`：后者要等指针移动超过
//! `max_click_dist`（6px）才认定是拖拽，而 REQ-002 要求"按下即发 `StartDrag`"。
//! 把手上没有点击语义，丢掉 click 不损失任何东西。

use egui::{
    self, Align, CentralPanel, CursorIcon, Key, KeyboardShortcut, Layout, Modifiers, Pos2, Rect,
    RichText, Sense, TextStyle, Vec2,
};
use ice_core::config::DockEdge;

use crate::state::AppState;
use crate::system::clipboard::CopyKind;
use crate::system::hotkeys::SOURCE_EDITOR_ID;
use crate::theme;

use super::{Effects, engine_picker, full, toasts};

/// 顶部把手条高度。`Frame::NONE` 不带内边距，这 10px 全是可拖区。
const HANDLE_HEIGHT: f32 = 10.0;
/// 输入行高度（与 P4 的简洁窗一致，单行 `TextEdit` 加上下 2px 内边距刚好装得下）
const INPUT_HEIGHT: f32 = 34.0;
/// 动作行高度
const ACTION_HEIGHT: f32 = 28.0;
/// 译文区底部把手带高度。96px 的最小窗里，正文只剩 96-10-34-28-4=20px，
/// 再厚就看不见译文了。
const TAIL_HEIGHT: f32 = 8.0;
/// 正文可视高度低于这个值时不画底部把手（窗口被压到极限时优先保译文）
const MIN_BODY_HEIGHT: f32 = 20.0;
/// 窄标签阈值：窗口内宽小于它时，动作行的文字标签换成单字，给引擎分段腾地方
const NARROW_WIDTH: f32 = 380.0;
/// 动作行空隙把手与左右两组按钮之间留的距离，避免指针落在按钮边缘时误判成拖窗
const GAP_PADDING: f32 = 4.0;

/// 译文区滚动容器的 id（`full::scroll_area` 按名字区分两栏）
const TRANSLATION_PANE: &str = "compact_translation";

/// 提示条离窗口顶部的距离：越过把手条与输入行，不遮住正在输入的字（REQ-005）
pub const TOAST_OFFSET: f32 = HANDLE_HEIGHT + INPUT_HEIGHT + 4.0;

pub fn show(ui: &mut egui::Ui, state: &mut AppState, effects: &mut Effects, ai_available: bool) {
    handle_bar(ui, effects);
    input_row(ui, state);
    action_row(ui, state, effects, ai_available);
    translation_pane(ui, state, effects);

    let ctx = ui.ctx().clone();
    toasts(&ctx, state, TOAST_OFFSET);
}

/// 把控件矩形记在临时数据里，供无头布局测试断言。
///
/// 看不见像素的测试只能靠这些数判断"控件有没有溢出窗口""把手有没有压住按钮"——
/// P4 的 D-1/D-3 那批缺陷正是版面越界而测试无从发现。
fn record(ctx: &egui::Context, salt: &str, rect: Rect) {
    ctx.data_mut(|d| d.insert_temp(rect_id(salt), rect));
}

fn rect_id(salt: &str) -> egui::Id {
    egui::Id::NULL.with("ice-translator.compact").with(salt)
}

/// 一块空白当拖拽把手：按下即请求原生窗口拖拽，指针悬停时变成抓手。
fn drag_handle(ui: &egui::Ui, salt: &str, rect: Rect, effects: &mut Effects) {
    if rect.is_negative() || rect.width() < 1.0 || rect.height() < 1.0 {
        return;
    }
    let response = ui
        .interact(rect, egui::Id::unique(salt), Sense::drag())
        .on_hover_cursor(CursorIcon::Grab);
    if response.drag_started() {
        effects.start_drag = true;
    }
}

/// 顶部把手条。
///
/// 不用 `Frame::side_top_panel`：它自带 `symmetric(8, 2)` 的内边距，10px 的面板只剩 6px
/// 可拖区。改成无边框＋面板底色，视觉上与下面几块连成一片，整条都是把手。
fn handle_bar(ui: &mut egui::Ui, effects: &mut Effects) {
    let fill = ui.style().visuals.panel_fill;
    egui::Panel::top("compact_handle")
        .exact_size(HANDLE_HEIGHT)
        .frame(egui::Frame::NONE.fill(fill))
        .show(ui, |ui| {
            let rect = ui.max_rect();
            record(ui.ctx(), "handle_top", rect);
            // 中间一小段圆条：纯装饰，告诉用户这里能拖
            let grip = Rect::from_center_size(rect.center(), Vec2::new(36.0, 4.0));
            ui.painter().rect_filled(
                grip,
                egui::CornerRadius::same(2),
                theme::MUTED.gamma_multiply(0.45),
            );
            drag_handle(ui, "compact_handle_top", rect, effects);
        });
}

/// 输入行：单行编辑，裸 Enter 翻译、Shift+Enter 不換行（单行没有换行）、Esc 清空。
fn input_row(ui: &mut egui::Ui, state: &mut AppState) {
    egui::Panel::top("compact_source")
        .exact_size(INPUT_HEIGHT)
        .show(ui, |ui| {
            record(ui.ctx(), "input", ui.max_rect());
            let output = egui::TextEdit::singleline(&mut state.source)
                .id(egui::Id::unique(SOURCE_EDITOR_ID))
                .desired_width(f32::INFINITY)
                .hint_text("输入或粘贴，Enter 翻译，Esc 清空")
                .return_key(KeyboardShortcut::new(Modifiers::SHIFT, Key::Enter))
                .show(ui);
            // 快捷键在绘制之前判定，这里记下的是给下一帧用的事实
            state.focus_in_source = output.response.has_focus();
            state.sync_char_count();
        });
}

/// 动作行：左侧「全量 / 复制 / 设置 / 机翻 / AI」，右侧「置顶 / 停靠」，
/// 中间的空隙是第三块把手。
///
/// 整行降到 `TextStyle::Small` 并把按钮内边距压到 (4,1)：320px 的最小窗里，
/// 七个控件按默认字号要 300px 以上，会把右侧那两个挤出窗口（REQ-005"不溢出、不重叠"）。
fn action_row(ui: &mut egui::Ui, state: &mut AppState, effects: &mut Effects, ai_available: bool) {
    egui::Panel::bottom("compact_bar")
        .exact_size(ACTION_HEIGHT)
        .show(ui, |ui| {
            ui.style_mut().override_text_style = Some(TextStyle::Small);
            ui.style_mut().spacing.button_padding = Vec2::new(4.0, 1.0);
            ui.style_mut().spacing.item_spacing.x = 4.0;

            let row = ui.max_rect();
            let ctx = ui.ctx().clone();
            record(&ctx, "actions", row);
            let narrow = row.width() < NARROW_WIDTH;

            let mut left_end = row.min.x;
            let mut right_start = row.max.x;
            ui.horizontal(|ui| {
                let full_mode = ui
                    .small_button("全量")
                    .on_hover_text("切回全量界面（Cmd+Shift+M）");
                record(&ctx, "btn_full", full_mode.rect);
                if full_mode.clicked() {
                    effects.toggle_mode = true;
                }

                let copy = ui
                    .small_button("复制")
                    .on_hover_text("复制译文（Cmd+Shift+C）");
                record(&ctx, "btn_copy", copy.rect);
                if copy.clicked() {
                    full::copy(state, effects, CopyKind::Translation);
                }

                let settings = ui.small_button("设置").on_hover_text("打开设置页");
                record(&ctx, "btn_settings", settings.rect);
                if settings.clicked() {
                    effects.open_settings = true;
                }

                // 引擎分段与全量工具条共用同一个实现（REQ-005：AI 未配置时同样禁用并给说明）
                let engine_start = ui.min_rect().max.x;
                engine_picker(ui, state, effects, ai_available);
                record(
                    &ctx,
                    "engine",
                    Rect::from_min_max(
                        Pos2::new(engine_start, row.min.y),
                        Pos2::new(ui.min_rect().max.x, row.max.y),
                    ),
                );

                left_end = ui.min_rect().max.x;
                record(
                    &ctx,
                    "actions_left",
                    Rect::from_min_max(row.min, Pos2::new(left_end, row.max.y)),
                );

                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    let top = ui
                        .selectable_label(state.config.ui.always_on_top, topmost_label(narrow))
                        .on_hover_text("窗口置顶（Cmd+Shift+T）");
                    record(&ctx, "btn_top", top.rect);
                    if top.clicked() {
                        effects.toggle_topmost = true;
                    }

                    let docked = state.config.ui.docked_edge;
                    let dock = ui
                        .selectable_label(docked != DockEdge::None, dock_label(docked, narrow))
                        .on_hover_text("切换停靠边：窗口贴到屏幕那一侧");
                    record(&ctx, "btn_dock", dock.rect);
                    if dock.clicked() {
                        effects.cycle_dock = true;
                    }

                    right_start = ui.min_rect().min.x;
                });
                record(
                    &ctx,
                    "actions_right",
                    Rect::from_min_max(Pos2::new(right_start, row.min.y), row.max),
                );
            });

            let gap = Rect::from_min_max(
                Pos2::new(left_end + GAP_PADDING, row.min.y),
                Pos2::new(right_start - GAP_PADDING, row.max.y),
            );
            record(&ctx, "bar_gap", gap);
            drag_handle(ui, "compact_handle_bar", gap, effects);
        });
}

/// 停靠指示的文案。窄窗下省掉"停靠："前缀，只留方位字。
pub fn dock_label(edge: DockEdge, narrow: bool) -> &'static str {
    match edge {
        DockEdge::Left => {
            if narrow {
                "左"
            } else {
                "停靠：左"
            }
        }
        DockEdge::Right => {
            if narrow {
                "右"
            } else {
                "停靠：右"
            }
        }
        DockEdge::Top => {
            if narrow {
                "上"
            } else {
                "停靠：上"
            }
        }
        DockEdge::Bottom => {
            if narrow {
                "下"
            } else {
                "停靠：下"
            }
        }
        DockEdge::None => "自由",
    }
}

pub const fn topmost_label(narrow: bool) -> &'static str {
    if narrow { "顶" } else { "置顶" }
}

/// 译文区：定高滚动容器 + 底部把手带。
///
/// 正文与把手带分成两块矩形手工排（同 `full.rs::exact_row` 的做法）而不是把手画进滚动容器里：
/// 滚动容器里的任何东西都算"内容高度"，把手会把内容撑得永远比可视区高，
/// 于是滚动条常驻、滚下去却什么也没有（P4 缺陷 D-4 的同一个坑）。
fn translation_pane(ui: &mut egui::Ui, state: &mut AppState, effects: &mut Effects) {
    CentralPanel::default()
        .frame(egui::Frame::central_panel(ui.style()).inner_margin(egui::Margin::symmetric(8, 2)))
        .show(ui, |ui| {
            let pane = ui.max_rect();
            record(ui.ctx(), "translation", pane);
            if pane.height() < MIN_BODY_HEIGHT {
                return;
            }
            let split = pane.max.y - TAIL_HEIGHT;
            let body_rect = Rect::from_min_max(pane.min, Pos2::new(pane.max.x, split));
            let tail_rect = Rect::from_min_max(Pos2::new(pane.min.x, split), pane.max);

            let mut child = ui.new_child(
                egui::UiBuilder::new()
                    .max_rect(body_rect)
                    .layout(Layout::top_down(Align::Min)),
            );
            let output = full::scroll_area(TRANSLATION_PANE).show(&mut child, |ui| {
                body(ui, state);
            });
            full::record_scroll(ui.ctx(), TRANSLATION_PANE, &output);

            record(ui.ctx(), "handle_tail", tail_rect);
            drag_handle(ui, "compact_handle_tail", tail_rect, effects);
            ui.advance_cursor_after_rect(pane);
        });
}

/// 译文正文。四种状态与全量窗一致（失败红字 / 译文可选中 / 翻译中 / 空态提示），
/// 只是不画徽标行——96px 的窗里放不下，耗时与缓存标记回全量窗看。
fn body(ui: &mut egui::Ui, state: &AppState) {
    if let Some(failure) = &state.failure {
        ui.add(
            egui::Label::new(RichText::new(&failure.message).color(theme::ERROR))
                .selectable(true)
                .wrap(),
        );
        return;
    }
    if let Some(result) = &state.translation {
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing.x = 6.0;
            ui.add(
                egui::Label::new(RichText::new(&result.text))
                    .selectable(true)
                    .wrap(),
            );
            ui.label(
                RichText::new(result.engine.display_name())
                    .color(theme::MUTED)
                    .small(),
            );
        });
        return;
    }
    if state.in_flight.is_some() {
        ui.horizontal(|ui| {
            ui.spinner();
            ui.colored_label(theme::WARN, "翻译中…");
        });
        return;
    }
    let hint = if state.source.trim().is_empty() {
        "译文显示在这里"
    } else {
        "按 Enter 翻译"
    };
    ui.colored_label(theme::MUTED, hint);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::{Failure, KeyState};
    use crate::views::Effects;
    use ice_core::config::Config;
    use ice_core::glossary::Glossary;
    use ice_core::language::EngineKind;
    use ice_providers::request::TranslationResult;

    /// 最小简洁窗（FR-3.2 的下限）：所有布局断言都在这个尺寸下做，装得下就一定装得下更大的
    fn window() -> Rect {
        Rect::from_min_size(Pos2::ZERO, Vec2::new(320.0, 96.0))
    }

    fn state() -> AppState {
        let mut state = AppState::new(
            Config::default(),
            Glossary::default(),
            KeyState::Missing,
            None,
        );
        state.source = "hello".into();
        state
    }

    fn translated(text: &str) -> TranslationResult {
        TranslationResult {
            text: text.into(),
            engine: EngineKind::Machine,
            detected: None,
            elapsed_ms: 320,
            cache_hit: false,
            segments: 1,
        }
    }

    struct Harness {
        ctx: egui::Context,
        size: Vec2,
        time: f64,
    }

    impl Harness {
        fn new(size: Vec2) -> Self {
            Self {
                ctx: egui::Context::default(),
                size,
                time: 0.0,
            }
        }

        fn step(
            &mut self,
            state: &mut AppState,
            effects: &mut Effects,
            ai_available: bool,
            events: Vec<egui::Event>,
        ) {
            let mut raw = egui::RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, self.size)),
                events,
                ..Default::default()
            };
            raw.time = Some(self.time);
            self.time += 1.0 / 60.0;
            let output = self
                .ctx
                .run_ui(raw, |ui| show(ui, state, effects, ai_available));
            output.drop_without_applying_deltas();
        }

        /// 命中测试用的是**上一帧**记下的控件矩形，点击之前必须先空跑两帧，
        /// 否则第一帧上任何控件都点不中
        fn warm(&mut self, state: &mut AppState, effects: &mut Effects, ai_available: bool) {
            for _ in 0..2 {
                self.step(state, effects, ai_available, vec![]);
            }
        }

        fn click(
            &mut self,
            state: &mut AppState,
            effects: &mut Effects,
            ai_available: bool,
            at: Pos2,
        ) {
            let button = |pressed| egui::Event::PointerButton {
                pos: at,
                button: egui::PointerButton::Primary,
                pressed,
                modifiers: egui::Modifiers::NONE,
            };
            self.step(
                state,
                effects,
                ai_available,
                vec![egui::Event::PointerMoved(at), button(true)],
            );
            self.step(state, effects, ai_available, vec![button(false)]);
        }
    }

    fn rect(ctx: &egui::Context, salt: &str) -> Rect {
        ctx.data_mut(|d| d.get_temp::<Rect>(rect_id(salt)))
            .unwrap_or_else(|| panic!("没有记下 {salt} 的矩形"))
    }

    fn scroll_offset(ctx: &egui::Context) -> f32 {
        let (_, _, offset) = ctx
            .data_mut(|d| d.get_temp::<(Rect, Vec2, Vec2)>(full::scroll_id(TRANSLATION_PANE)))
            .expect("译文区要有滚动容器");
        offset.y
    }

    /// 96px 的最小窗里，四块面板 + 所有按钮 + 两块把手都必须待在窗口内，且互不重叠。
    /// 这条锁的是 REQ-005 的"不溢出、不重叠"：动作行按默认字号排会到 300px 以上，
    /// 右侧那两个按钮就被挤出窗口（P4 的 D-1 同类缺陷）。
    #[test]
    fn every_control_fits_the_minimum_compact_window() {
        let mut h = Harness::new(window().size());
        let mut state = state();
        state.translation = Some(translated("你好"));
        let mut effects = Effects::default();
        h.warm(&mut state, &mut effects, true);

        let win = window();
        for salt in [
            "handle_top",
            "input",
            "actions",
            "actions_left",
            "actions_right",
            "bar_gap",
            "btn_full",
            "btn_copy",
            "btn_settings",
            "btn_top",
            "btn_dock",
            "engine",
            "translation",
            "handle_tail",
        ] {
            let r = rect(&h.ctx, salt);
            assert!(
                r.min.x >= win.min.x - 0.5
                    && r.min.y >= win.min.y - 0.5
                    && r.max.x <= win.max.x + 0.5
                    && r.max.y <= win.max.y + 0.5,
                "{salt} 越出窗口：{r:?}（窗口 {win:?}）"
            );
        }

        let gap = rect(&h.ctx, "bar_gap");
        assert!(
            gap.width() >= 0.0,
            "动作行左右两组按钮已经贴到一起，中间放不下把手：{gap:?}"
        );

        // 同一行里的控件两两不重叠（把手更不能压住按钮，否则点按钮会变成拖窗）
        let row = [
            rect(&h.ctx, "btn_full"),
            rect(&h.ctx, "btn_copy"),
            rect(&h.ctx, "btn_settings"),
            rect(&h.ctx, "engine"),
            gap,
            rect(&h.ctx, "actions_right"),
        ];
        for (i, a) in row.iter().enumerate() {
            for b in row.iter().skip(i + 1) {
                assert!(
                    a.intersect(*b).is_negative() || a.intersect(*b).width() <= 0.5,
                    "动作行里有两个控件重叠了：{a:?} 与 {b:?}"
                );
            }
        }
    }

    /// 四块自上而下堆叠：把手条 → 输入行 → 译文区 → 动作行（底）
    #[test]
    fn the_four_bands_are_stacked_without_gaps_or_overlaps() {
        let mut h = Harness::new(window().size());
        let mut state = state();
        let mut effects = Effects::default();
        h.warm(&mut state, &mut effects, false);

        let handle = rect(&h.ctx, "handle_top");
        let input = rect(&h.ctx, "input");
        let translation = rect(&h.ctx, "translation");
        let actions = rect(&h.ctx, "actions");
        assert!(handle.bottom() <= input.top() + 0.5);
        assert!(input.bottom() <= translation.top() + 0.5);
        assert!(translation.bottom() <= actions.top() + 0.5);
        assert!(
            actions.bottom() <= window().max.y + 0.5,
            "动作行被挤出窗口，最小窗下会看不见"
        );
    }

    /// 三块把手各自都要能触发原生拖拽，其余地方一个都不能
    fn drag_probe(at: Pos2) -> bool {
        let mut h = Harness::new(window().size());
        let mut state = state();
        let mut effects = Effects::default();
        h.warm(&mut state, &mut effects, false);
        h.step(
            &mut state,
            &mut effects,
            false,
            vec![
                egui::Event::PointerMoved(at),
                egui::Event::PointerButton {
                    pos: at,
                    button: egui::PointerButton::Primary,
                    pressed: true,
                    modifiers: egui::Modifiers::NONE,
                },
            ],
        );
        effects.start_drag
    }

    #[test]
    fn the_handles_start_a_native_drag_and_nothing_else_does() {
        let mut h = Harness::new(window().size());
        let mut state = state();
        let mut effects = Effects::default();
        h.warm(&mut state, &mut effects, false);
        let (handle, input, gap, tail) = (
            rect(&h.ctx, "handle_top"),
            rect(&h.ctx, "input"),
            rect(&h.ctx, "bar_gap"),
            rect(&h.ctx, "handle_tail"),
        );

        assert!(drag_probe(handle.center()), "顶部把手条拖不动");
        assert!(drag_probe(gap.center()), "动作行中间的把手拖不动");
        assert!(drag_probe(tail.center()), "译文底部的把手拖不动");
        assert!(
            !drag_probe(input.center()),
            "输入行被当成把手，用户就没法点进去打字了"
        );
    }

    /// REQ-005：简洁窗里能直接换引擎，且 AI 未配置时点了不派发请求（与全量窗同源）
    #[test]
    fn the_engine_segment_switches_engines_and_retranslates() {
        let mut h = Harness::new(window().size());
        let mut state = state();
        let mut effects = Effects::default();
        h.warm(&mut state, &mut effects, true);
        let engine = rect(&h.ctx, "engine");
        // 分段里第一个是「机翻」、第二个是「AI」，点右半边落在 AI 上
        let ai_at = Pos2::new(engine.max.x - 4.0, engine.center().y);

        h.click(&mut state, &mut effects, true, ai_at);

        assert_eq!(state.engine, EngineKind::Ai);
        let job = effects.job.expect("切到 AI 应当立刻用新引擎重译");
        assert_eq!(job.engine, EngineKind::Ai);
    }

    #[test]
    fn ai_is_disabled_when_it_is_not_configured() {
        let mut h = Harness::new(window().size());
        let mut state = state();
        let mut effects = Effects::default();
        h.warm(&mut state, &mut effects, false);
        let engine = rect(&h.ctx, "engine");
        h.click(
            &mut state,
            &mut effects,
            false,
            Pos2::new(engine.max.x - 4.0, engine.center().y),
        );
        assert_eq!(state.engine, EngineKind::Machine, "AI 不可用时不该切过去");
        assert!(effects.job.is_none());
    }

    #[test]
    fn the_settings_button_asks_to_open_the_settings_tab() {
        let mut h = Harness::new(window().size());
        let mut state = state();
        let mut effects = Effects::default();
        h.warm(&mut state, &mut effects, false);
        let at = rect(&h.ctx, "btn_settings").center();
        h.click(&mut state, &mut effects, false, at);
        assert!(effects.open_settings);
        assert!(!effects.toggle_mode, "设置走自己的通道，不是普通的形态切换");
    }

    #[test]
    fn the_dock_and_topmost_indicators_are_buttons() {
        let mut h = Harness::new(window().size());
        let mut state = state();
        let mut effects = Effects::default();
        h.warm(&mut state, &mut effects, false);
        h.click(
            &mut state,
            &mut effects,
            false,
            rect(&h.ctx, "btn_dock").center(),
        );
        assert!(effects.cycle_dock);

        let mut effects = Effects::default();
        h.click(
            &mut state,
            &mut effects,
            false,
            rect(&h.ctx, "btn_top").center(),
        );
        assert!(effects.toggle_topmost);
    }

    #[test]
    fn the_copy_button_queues_the_translation_without_touching_the_clipboard() {
        let mut h = Harness::new(window().size());
        let mut state = state();
        state.translation = Some(translated("你好"));
        let mut effects = Effects::default();
        h.warm(&mut state, &mut effects, false);
        h.click(
            &mut state,
            &mut effects,
            false,
            rect(&h.ctx, "btn_copy").center(),
        );
        assert_eq!(effects.copy.as_deref(), Some("你好"));
    }

    /// 走查缺陷：简洁窗里的译文滚不动。定高滚动容器 + 滚轮 → 偏移必须真的变
    #[test]
    fn a_long_translation_scrolls_inside_the_compact_pane() {
        let mut h = Harness::new(window().size());
        let mut state = state();
        state.translation = Some(translated(&"一段很长的译文。".repeat(80)));
        let mut effects = Effects::default();
        h.warm(&mut state, &mut effects, false);
        let pane = rect(&h.ctx, "translation");
        let before = scroll_offset(&h.ctx);

        for _ in 0..8 {
            h.step(
                &mut state,
                &mut effects,
                false,
                vec![
                    egui::Event::PointerMoved(pane.center()),
                    egui::Event::MouseWheel {
                        unit: egui::MouseWheelUnit::Point,
                        delta: Vec2::new(0.0, -5.0),
                        phase: egui::TouchPhase::Move,
                        source: egui::MouseWheelSource::Trackpad,
                        modifiers: egui::Modifiers::NONE,
                    },
                ],
            );
        }

        assert!(
            scroll_offset(&h.ctx) > before,
            "滚轮给下去偏移没动：译文区仍然滚不动"
        );
    }

    /// 内容不超高时不该出现"能滚但什么也没有"的假滚动条（底部把手带不算内容高度）
    #[test]
    fn a_short_translation_does_not_invent_a_scrollbar() {
        let mut h = Harness::new(window().size());
        let mut state = state();
        state.translation = Some(translated("你好"));
        let mut effects = Effects::default();
        h.warm(&mut state, &mut effects, false);
        let (viewport, content, _) = h
            .ctx
            .data_mut(|d| d.get_temp::<(Rect, Vec2, Vec2)>(full::scroll_id(TRANSLATION_PANE)))
            .expect("译文区要有滚动容器");
        assert!(
            content.y <= viewport.height() + 0.5,
            "内容 {content:?} 比可视区 {viewport:?} 还高，会常驻一条没用的滚动条"
        );
    }

    #[test]
    fn narrow_windows_use_the_short_labels() {
        assert_eq!(dock_label(DockEdge::Right, true), "右");
        assert_eq!(dock_label(DockEdge::Right, false), "停靠：右");
        assert_eq!(dock_label(DockEdge::Left, false), "停靠：左");
        assert_eq!(dock_label(DockEdge::Top, false), "停靠：上");
        assert_eq!(dock_label(DockEdge::Bottom, false), "停靠：下");
        assert_eq!(dock_label(DockEdge::None, true), "自由");
        assert_eq!(dock_label(DockEdge::None, false), "自由");
        assert_eq!(topmost_label(true), "顶");
        assert_eq!(topmost_label(false), "置顶");
    }

    /// 窗口拉宽之后标签恢复全称，且仍然装得下
    #[test]
    fn a_wider_window_gets_the_full_labels_and_still_does_not_overflow() {
        let mut h = Harness::new(Vec2::new(560.0, 160.0));
        let mut state = state();
        let mut effects = Effects::default();
        h.warm(&mut state, &mut effects, false);
        assert_eq!(rect(&h.ctx, "actions").width() + 16.0, 560.0);
        let gap = rect(&h.ctx, "bar_gap");
        assert!(gap.width() > 20.0, "宽窗下把手应当更宽，实际 {gap:?}");
    }

    /// 失败态在简洁窗里同样显示红字原因，且不动源文（FR-1.7）
    #[test]
    fn a_failure_shows_its_reason_and_keeps_the_source() {
        let mut h = Harness::new(window().size());
        let mut state = state();
        state.failure = Some(Failure {
            message: "AI 未配置".into(),
            action: None,
        });
        let mut effects = Effects::default();
        h.step(&mut state, &mut effects, false, vec![]);
        assert_eq!(state.source, "hello");
        assert!(effects.job.is_none(), "渲染失败态不该自己重发请求");
    }
}
