//! 文本页的 B 区（源文）与 C 区（译文）：F-06~F-10。
//!
//! 绘制顺序即视觉顺序（工具条 → 源文 → 译文 → 底栏 → 状态条），Tab 键的焦点遍历才不会出现
//! "视觉在下面、焦点先到"的错位。动作按钮住在**自占一行的底栏**里（REQ-013，v0.7）：上一版
//! 它浮在译文栏右下角，正文末尾要预留一条等高空白才不被压住，预留算错一点，长译文滚到底
//! 最后一行就看不见（用户 2026-10-27 那轮的 D-9）。

use egui::{self, Align, Key, KeyboardShortcut, Layout, Modifiers, RichText};

use crate::state::{AppState, FailureAction, Tone};
use crate::system::clipboard::{self, CopyKind, EMPTY_TRANSLATION_HINT};
use crate::system::hotkeys::SOURCE_EDITOR_ID;
use crate::theme;

use super::Effects;

/// 底栏按钮的标准尺寸：等宽排列（F-09）
const BUTTON_SIZE: egui::Vec2 = egui::Vec2::new(84.0, 28.0);

/// 窄窗退让的第一档（REQ-013）：先降宽度、不降高度——用户说的是"把按钮调整小一点"，
/// 不是"小到点不到"。（`MIN_COMPACT` 那条"不许压缩控件"只约束简洁窗的动作行。）
const BUTTON_SIZE_NARROW: egui::Vec2 = egui::Vec2::new(64.0, 28.0);

/// 底栏的上下留白（px）
const BAR_MARGIN: f32 = 6.0;

/// 底栏按钮之间的间距（px）
const BAR_SPACING: f32 = 6.0;

/// 底栏左侧给字数文本留出的宽度：字数可以省略号截断，按钮不行（REQ-013 的退让顺序）
const CHAR_SLOT: f32 = 160.0;

/// 两栏页脚行（字数 / 徽标）的固定高度。
///
/// 徽标比纯文本高（内边距 4 + 小字号），空页脚又是 0 高——页脚高度各算各的，
/// 两栏的底边就会差出一截，真机走查里"两边窗口大小不一样"正是这么来的。
/// 固定成一行能装下徽标的高度，两栏底边才永远对齐。
const PANE_FOOTER_HEIGHT: f32 = 26.0;

/// 底栏的矩形 id：布局测试要拿它跟两栏的正文矩形比"互不重叠"
fn bottom_bar_id() -> egui::Id {
    egui::Id::NULL.with("ice-translator.bottom-bar")
}

/// 两栏各自的滚动容器。
///
/// 正文（源文编辑器 / 译文标签）都按内容自然高度布局，装不下的部分靠滚动看——
/// 不套滚动容器的话，18k 字符的粘贴会把源文框一路撑到窗口外（W-1.16 的"变形、贴底"），
/// 而且既裁不掉也滚不动。简洁窗的译文区复用同一个构造（REQ-005 要求同一手法）。
pub(crate) fn scroll_area(pane: &'static str) -> egui::ScrollArea {
    egui::ScrollArea::vertical()
        .id_salt(pane)
        // 撑满给定的正文矩形，不随内容收缩，两栏的框才有确定的高度
        .auto_shrink([false, false])
        // 可视区不许超过给它的矩形：`min_scrolled_size` 默认 64px，会把矮栏的视口抬到 64，
        // 多出来的那十几像素画到框外（简洁窗里正好压在底部动作行之下，最后一行永远看不见，
        // 用户 2026-10-27）。滚不动的风险由调用方的栏高守卫兜着，不靠这条默认值。
        .min_scrolled_height(0.0)
}

/// 把滚动容器的实测数据（可视矩形、内容尺寸、当前偏移）记在临时数据里，供布局测试断言。
///
/// 无头测试看不见像素，只能靠这三个数断言"内容确实比可视区高"和"滚轮给下去偏移真的动了"
/// ——W-1.16 的回归锁（源文框曾经既不裁剪也不能滚）。
pub(crate) fn record_scroll<R>(
    ctx: &egui::Context,
    pane: &'static str,
    output: &egui::containers::scroll_area::ScrollAreaOutput<R>,
) {
    ctx.data_mut(|d| {
        d.insert_temp(
            scroll_id(pane),
            (
                output.inner_rect,
                output.content_size,
                output.state.clamped_offset(),
            ),
        )
    });
}

pub(crate) fn scroll_id(pane: &'static str) -> egui::Id {
    egui::Id::NULL.with("ice-translator.scroll").with(pane)
}

pub fn show(ui: &mut egui::Ui, state: &mut AppState, effects: &mut Effects) {
    // 底栏先建：`Panel::bottom` 自下而上占位，两栏拿到的是**扣掉底栏之后**的剩余高度。
    // v0.7 第一版让两栏自己减 `bar_height`，扣的数与画的数差 3px 就把条子挤出窗口——
    // 交给 egui 排，"扣多少"和"画多高"才是同一个数（REQ-013 要的就是这个）。
    let actions = bar_actions(state);
    bottom_bar(ui, state, effects, &actions);
    // 分两栏不能用 `Ui::columns`：它给每列的矩形用的是 `max_rect` 的底边，不看光标，
    // 底栏刚从光标矩形里扣掉那一条它看不见（实测正文底边压过条子上沿 5px）。
    // `horizontal` 系布局按 `available_size_before_wrap()` 分配，扣掉的高度真的传得进来。
    ui.horizontal_top(|ui| {
        let gap = ui.spacing().item_spacing.x;
        let half = (ui.available_width() - gap) / 2.0;
        let size = egui::vec2(half, ui.available_height());
        ui.allocate_ui_with_layout(size, egui::Layout::top_down(egui::Align::Min), |ui| {
            source_pane(ui, state)
        });
        ui.allocate_ui_with_layout(size, egui::Layout::top_down(egui::Align::Min), |ui| {
            translation_pane(ui, state)
        });
    });
}

/// 定高行：内容画进丢弃的子 Ui，光标按给定高度精确推进。
///
/// TextEdit 的外框、徽标的内边距、空页脚的实际高度都不可控（各差几个像素），
/// 只有行高定死，两栏的底边才永远对齐——真机走查里"两边窗口大小不一样"的根因。
fn exact_row(ui: &mut egui::Ui, height: f32, layout: Layout, add: impl FnOnce(&mut egui::Ui)) {
    let cursor = ui.cursor();
    let rect = egui::Rect::from_min_size(cursor.min, egui::vec2(cursor.width(), height));
    let mut child = ui.new_child(egui::UiBuilder::new().max_rect(rect).layout(layout));
    add(&mut child);
    ui.advance_cursor_after_rect(rect);
}

/// B 区：源文编辑器（F-06）。字数不再挂在它脚注上——v0.7 起住进底栏左侧（REQ-013）。
fn source_pane(ui: &mut egui::Ui, state: &mut AppState) {
    ui.label(RichText::new("源文").strong());
    let body = pane_body_height(ui);
    let cursor = ui.cursor();
    let body_rect = egui::Rect::from_min_size(cursor.min, egui::vec2(cursor.width(), body));
    let mut pane = ui.new_child(
        egui::UiBuilder::new()
            .max_rect(body_rect)
            .layout(Layout::top_down(Align::Min)),
    );
    // 外框由这一层画，编辑器自己不带框：TextEdit 的框跟着内容高度走，内容一溢出
    // 底边就被滚动容器裁掉（W-1.16 修复后的新毛病），译文栏的框却是定高的 Frame::group。
    // 焦点态取上一帧记下的事实——本帧的 has_focus 要等编辑器画完才知道。
    let stroke = if state.focus_in_source {
        pane.visuals().widgets.active.bg_stroke
    } else {
        pane.visuals().widgets.noninteractive.bg_stroke
    };
    egui::Frame::group(pane.style())
        .inner_margin(8.0)
        .stroke(stroke)
        .show(&mut pane, |ui| {
            ui.allocate_ui(egui::vec2(ui.available_width(), body - 16.0), |ui| {
                let output = scroll_area("source").show(ui, |ui| {
                    egui::TextEdit::multiline(&mut state.source)
                        .id(egui::Id::unique(SOURCE_EDITOR_ID))
                        .frame(egui::Frame::NONE)
                        .desired_width(f32::INFINITY)
                        .min_size(egui::vec2(0.0, body - 16.0))
                        .hint_text("输入或粘贴文本（Enter 翻译，Shift+Enter 换行）")
                        // 裸 Enter 归"翻译"，换行只认 Shift+Enter（FR-1.1）
                        .return_key(KeyboardShortcut::new(Modifiers::SHIFT, Key::Enter))
                        .show(ui)
                });
                // 快捷键在绘制之前判定，所以这里记下的是给下一帧用的事实
                state.focus_in_source = output.inner.response.has_focus();
                record_scroll(ui.ctx(), "source", &output);
            });
        });
    ui.advance_cursor_after_rect(body_rect);

    state.sync_char_count();
}

/// 两栏共用的正文高度算法：扣掉行间距与固定页脚，剩下的全给正文。
///
/// 底栏不在这里扣：它是 `Panel::bottom`，`show` 里先建，两栏拿到的可用高度本来就少了它。
/// 页脚要扣：译文栏的徽标行是定高的，源文栏没有页脚也得跟着让出同一格，两个框才一样大。
fn pane_body_height(ui: &egui::Ui) -> f32 {
    (ui.available_height() - ui.spacing().item_spacing.y - PANE_FOOTER_HEIGHT).max(80.0)
}

/// F-07：字数常驻；"检测：xx"只在源语言为自动检测时出现（FR-1.2）
pub fn char_count_text(state: &AppState) -> String {
    let mut text = format!("字数 {}", state.char_count);
    if state.config.translation.source.is_none() {
        if let Some(detected) = state.translation.as_ref().and_then(|r| r.detected) {
            text.push_str(&format!(" · 检测：{}", detected.display_name()));
        }
    }
    text
}

/// C 区：译文展示（F-08）+ 缓存与耗时标记（F-10）。动作按钮在底栏（F-09，REQ-013）。
fn translation_pane(ui: &mut egui::Ui, state: &mut AppState) {
    ui.label(RichText::new("译文").strong());
    let body = pane_body_height(ui);
    let cursor = ui.cursor();
    let body_rect = egui::Rect::from_min_size(cursor.min, egui::vec2(cursor.width(), body));
    let mut boxed = ui.new_child(
        egui::UiBuilder::new()
            .max_rect(body_rect)
            .layout(Layout::top_down(Align::Min)),
    );
    egui::Frame::group(boxed.style())
        .inner_margin(8.0)
        .show(&mut boxed, |ui| {
            // ScrollArea 的 auto_shrink(false) 会吃光"可用高度"而不是 set_min_height，
            // 不关进一个定高子 Ui，译文框就会比源文框高出一截
            ui.allocate_ui(egui::vec2(ui.available_width(), body - 16.0), |ui| {
                let scrolled = scroll_area("translation").show(ui, |ui| {
                    translation_body(ui, state);
                });
                record_scroll(ui.ctx(), "translation", &scrolled);
            });
        });
    ui.advance_cursor_after_rect(body_rect);
    exact_row(
        ui,
        PANE_FOOTER_HEIGHT,
        Layout::left_to_right(Align::Center),
        |ui| badge_row(ui, state),
    );
}

/// 动作条里的三个常驻动作 + 失败动作（F-13）
#[derive(Clone, Copy)]
enum BarAction {
    Translate,
    Copy(CopyKind),
    Failure(FailureAction),
}

impl BarAction {
    fn label(self, busy: bool) -> &'static str {
        match self {
            BarAction::Translate => {
                if busy {
                    "翻译中…"
                } else {
                    "翻译"
                }
            }
            BarAction::Copy(kind) => kind.label(),
            BarAction::Failure(action) => action.label(),
        }
    }
}

/// 动作条上的按钮：三个常驻动作，失败时多一枚"去设置"之类（F-13）
fn bar_actions(state: &AppState) -> Vec<BarAction> {
    let mut actions: Vec<BarAction> = vec![
        BarAction::Translate,
        BarAction::Copy(CopyKind::Translation),
        BarAction::Copy(CopyKind::Bilingual),
    ];
    if let Some(action) = state.failure.as_ref().and_then(|f| f.action) {
        actions.push(BarAction::Failure(action));
    }
    actions
}

/// 底栏排布（REQ-013 的退让顺序）：标准档一排排得下就用标准档；排不下先把按钮调窄一档；
/// 还是排不下才换行。返回按钮尺寸与行数，`bar_height` 与 `bottom_bar` 都用它，
/// 所以"扣多少高度"和"画多高"是同一个数——上一版正是这两处各算各的才留出 20px 的空缝。
fn bar_layout(button_area_w: f32, count: usize) -> (egui::Vec2, usize) {
    for size in [BUTTON_SIZE, BUTTON_SIZE_NARROW] {
        if bar_rows(button_area_w, count, size) == 1 {
            return (size, 1);
        }
    }
    let size = BUTTON_SIZE_NARROW;
    (size, bar_rows(button_area_w, count, size))
}

fn bar_rows(button_area_w: f32, count: usize, size: egui::Vec2) -> usize {
    let usable = button_area_w.max(size.x);
    let per_row = ((usable + BAR_SPACING) / (size.x + BAR_SPACING))
        .floor()
        .max(1.0) as usize;
    count.div_ceil(per_row).max(1)
}

/// 底栏占多高：行数 × 按钮高 + 行距 + 上下留白。
fn bar_height(size: egui::Vec2, rows: usize) -> f32 {
    rows as f32 * size.y + (rows - 1) as f32 * BAR_SPACING + 2.0 * BAR_MARGIN
}

/// F-09 / REQ-013：底栏。`Panel::bottom` 自占一行，左侧字数、右侧按钮组。
///
/// 从"浮层"改成"自占"的理由（用户 2026-10-27 那句"底部还是被遮住了"）：浮层不参与布局，
/// 正文必须靠预留空白躲它，预留公式与实画公式一旦不同步就又被压一行。底栏占位之后，
/// "最后一行被盖住"在结构上不成立，不需要任何预留。
///
/// 左右两块各用一个定长子 `Ui`，中间不会互相压：字数那格可以省略号截断，按钮那格不行。
/// 按钮按 `rev()` 排进 `right_to_left`，视觉顺序仍是「翻译」在最左。
fn bottom_bar(
    ui: &mut egui::Ui,
    state: &mut AppState,
    effects: &mut Effects,
    actions: &[BarAction],
) {
    let panel = egui::Panel::bottom("full_bottom_bar")
        .frame(egui::Frame::NONE.inner_margin(BAR_MARGIN))
        .show(ui, |ui| {
            let (size, rows) = bar_layout(ui.available_width() - CHAR_SLOT, actions.len());
            let height = bar_height(size, rows) - 2.0 * BAR_MARGIN;
            let rect = egui::Rect::from_min_size(
                ui.cursor().min,
                egui::vec2(ui.available_width(), height),
            );

            let mut left = ui.new_child(
                egui::UiBuilder::new()
                    .max_rect(egui::Rect::from_min_size(
                        rect.min,
                        egui::vec2(CHAR_SLOT, height),
                    ))
                    .layout(Layout::left_to_right(Align::Center)),
            );
            left.add_sized(
                [CHAR_SLOT - 8.0, size.y],
                egui::Label::new(RichText::new(char_count_text(state)).color(theme::MUTED))
                    .truncate(),
            );

            let mut right = ui.new_child(
                egui::UiBuilder::new()
                    .max_rect(egui::Rect::from_min_max(
                        egui::pos2(rect.min.x + CHAR_SLOT, rect.min.y),
                        rect.max,
                    ))
                    .layout(Layout::right_to_left(Align::Center)),
            );
            let busy = state.in_flight.is_some();
            for action in actions.iter().rev() {
                let response = right.add(egui::Button::new(action.label(busy)).min_size(size));
                if response.clicked() {
                    match *action {
                        BarAction::Translate => {
                            if !busy {
                                effects.request(state.submit());
                            }
                        }
                        BarAction::Copy(kind) => copy(state, effects, kind),
                        BarAction::Failure(action) => failure_action(state, effects, action),
                    }
                }
            }
            // 两块都画在丢弃的子 Ui 里，父级（面板）自己一条都没分配：不手工推进光标，
            // 面板高度就塌回"一个按钮"，换出来的第二排直接画到面板外面（与简洁窗同一个坑）
            ui.advance_cursor_after_rect(rect);
        });
    ui.ctx()
        .data_mut(|d| d.insert_temp(bottom_bar_id(), panel.response.rect));
}

fn translation_body(ui: &mut egui::Ui, state: &AppState) {
    if let Some(failure) = &state.failure {
        // 失败时译文区改红字显示原因；源文一个字符都不动（FR-1.7）
        ui.add(
            egui::Label::new(RichText::new(&failure.message).color(theme::ERROR))
                .selectable(true)
                .wrap(),
        );
        return;
    }
    if let Some(result) = &state.translation {
        ui.add(
            egui::Label::new(RichText::new(&result.text))
                .selectable(true)
                .wrap(),
        );
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
        "译文会显示在这里"
    } else {
        "尚未翻译：按 Enter 或 Cmd+Enter"
    };
    ui.colored_label(theme::MUTED, hint);
}

/// F-10：`缓存` / `320ms` / `分 N 段` / `检测：英语`；缓存命中时不显示耗时（此时恒为 0，会误导）
fn badge_row(ui: &mut egui::Ui, state: &AppState) {
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 8.0;
        if state.in_flight.is_some() && state.translation.is_some() {
            badge(ui, theme::WARN, "更新中");
        }
        if state.failure.is_some() {
            return;
        }
        let Some(result) = &state.translation else {
            return;
        };
        if result.cache_hit {
            badge(ui, theme::OK, "缓存");
        } else {
            badge(ui, theme::MUTED, &format!("{}ms", result.elapsed_ms));
        }
        if result.segments > 1 {
            badge(ui, theme::MUTED, &format!("分 {} 段", result.segments));
        }
        if state.config.translation.source.is_none() {
            if let Some(detected) = result.detected {
                badge(
                    ui,
                    theme::MUTED,
                    &format!("检测：{}", detected.display_name()),
                );
            }
        }
    });
}

fn badge(ui: &mut egui::Ui, color: egui::Color32, text: &str) {
    egui::Frame::group(ui.style())
        .inner_margin(4.0)
        .fill(color.gamma_multiply(0.15))
        .show(ui, |ui| {
            ui.label(RichText::new(text).color(color).small());
        });
}

/// 两个复制按钮与 Cmd+Shift+C 共用这一条路径。
///
/// 空译文时只提示、不碰剪贴板——绝不静默清空用户剪贴板里原有的东西（REQ-003）。
/// 真正的写入由 `app.rs` 在帧末做，失败时由它给可读错误。
pub fn copy(state: &mut AppState, effects: &mut Effects, kind: CopyKind) {
    let translated = state
        .translation
        .as_ref()
        .map(|result| result.text.as_str())
        .unwrap_or_default();
    match clipboard::compose(kind, &state.source, translated) {
        Some(text) => effects.copy = Some(text),
        None => state.toast(Tone::Warn, EMPTY_TRANSLATION_HINT),
    }
}

/// 失败动作（REQ-004）：去设置 / 改用机翻并立即重译
pub fn failure_action(state: &mut AppState, effects: &mut Effects, action: FailureAction) {
    match action {
        FailureAction::GoSettings => state.go_to_key_settings(),
        FailureAction::UseMachine => effects.request(state.switch_to_machine()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::{Failure, KeyState};
    use ice_core::config::Config;
    use ice_core::glossary::Glossary;
    use ice_core::language::{EngineKind, Language};
    use ice_providers::error::ProviderError;
    use ice_providers::request::TranslationResult;

    fn state() -> AppState {
        let mut state = AppState::new(
            Config::default(),
            Glossary::default(),
            KeyState::Missing,
            None,
        );
        state.source = "hello world".into();
        state
    }

    fn translated(text: &str) -> TranslationResult {
        TranslationResult {
            text: text.into(),
            engine: EngineKind::Machine,
            detected: Some(Language::En),
            elapsed_ms: 320,
            cache_hit: false,
            segments: 1,
        }
    }

    fn raw_input(width: f32, height: f32) -> egui::RawInput {
        egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(width, height),
            )),
            ..Default::default()
        }
    }

    /// 跑若干帧文本页（宽 `width`、高 720），返回 Context。
    /// 两帧起步：底栏矩形与滚动容器的实测数据都是画完那一帧才记上的，
    /// 单帧读到的还是上一帧（也就是没有）的值。
    fn run_text_page(
        state: &mut AppState,
        effects: &mut Effects,
        width: f32,
        frames: usize,
    ) -> egui::Context {
        let ctx = egui::Context::default();
        let mut time = 0.0;
        for _ in 0..frames {
            let mut raw = raw_input(width, 720.0);
            raw.time = Some(time);
            time += 1.0 / 60.0;
            let output = ctx.run_ui(raw, |ui| show(ui, state, effects));
            output.drop_without_applying_deltas();
        }
        ctx
    }

    /// 底栏这一行占到的矩形（REQ-013 之后它自占位，布局断言就比它的上沿）
    fn bar_rect(ctx: &egui::Context) -> egui::Rect {
        ctx.data_mut(|d| d.get_temp::<egui::Rect>(bottom_bar_id()))
            .expect("底栏要记下自己的矩形")
    }

    /// 某栏滚动容器的可视矩形
    fn scroll_rect(ctx: &egui::Context, pane: &'static str) -> egui::Rect {
        ctx.data_mut(|d| d.get_temp::<(egui::Rect, egui::Vec2, egui::Vec2)>(scroll_id(pane)))
            .unwrap_or_else(|| panic!("{pane} 栏没有滚动容器"))
            .0
    }

    /// 按真实的底栏排布跑一帧文本页，返回 Context（两栏矩形从 `scroll_rect` 量）
    fn measure_columns(state: &mut AppState) -> egui::Context {
        let ctx = egui::Context::default();
        let mut effects = Effects::default();
        let output = ctx.run_ui(raw_input(640.0, MIN_BODY_HEIGHT), |ui| {
            show(ui, state, &mut effects)
        });
        output.drop_without_applying_deltas();
        ctx
    }

    /// 取回某栏滚动容器的实测数据：可视高度 / 内容高度 / 当前偏移
    fn scroll_metrics(ctx: &egui::Context, pane: &'static str) -> ScrollMetrics {
        let (inner, content, offset) = ctx
            .data_mut(|d| d.get_temp::<(egui::Rect, egui::Vec2, egui::Vec2)>(scroll_id(pane)))
            .unwrap_or_else(|| panic!("{pane} 栏没有滚动容器"));
        ScrollMetrics {
            viewport: inner.height(),
            content: content.y,
            offset: offset.y,
        }
    }

    #[derive(Debug)]
    struct ScrollMetrics {
        viewport: f32,
        content: f32,
        offset: f32,
    }

    /// 18,000 字符的粘贴样本（W-1.16 用的就是 `/tmp/ice-18k-paste.txt` 这个量级）
    fn huge_text() -> String {
        let mut text = String::new();
        while text.chars().count() < 18_000 {
            text.push_str(
                "这是一段用来撑爆版面的中文长句，它没有任何实际含义，只是把字符数堆上去。",
            );
        }
        text
    }

    /// 把指针放在 `pointer` 处连发 8 帧小幅滚轮位移（按触控板处理，逐帧全额生效），
    /// 返回该栏滚动容器最后的偏移。
    fn wheel_scroll(
        state: &mut AppState,
        effects: &mut Effects,
        pane: &'static str,
        pointer: egui::Pos2,
    ) -> f32 {
        let ctx = egui::Context::default();
        let mut time = 0.0;
        for _ in 0..8 {
            let mut raw = raw_input(1080.0, 720.0);
            raw.time = Some(time);
            time += 1.0 / 60.0;
            raw.events.push(egui::Event::PointerMoved(pointer));
            raw.events.push(egui::Event::MouseWheel {
                unit: egui::MouseWheelUnit::Point,
                delta: egui::vec2(0.0, -5.0),
                phase: egui::TouchPhase::Move,
                source: egui::MouseWheelSource::Trackpad,
                modifiers: egui::Modifiers::NONE,
            });
            let output = ctx.run_ui(raw, |ui| show(ui, state, effects));
            output.drop_without_applying_deltas();
        }
        scroll_metrics(&ctx, pane).offset
    }

    /// 最小窗口 640×420 扣掉工具条 56、状态条 28、CentralPanel 上下内边距各 8 之后，
    /// 文本页真正拿到的高度。
    const MIN_BODY_HEIGHT: f32 = 420.0 - 56.0 - 28.0 - 16.0;

    /// 两栏的正文框必须等大、顶边对齐。
    ///
    /// 译文栏下面还挂着徽标行（`exact_row` 定高），源文栏 v0.7 起没有页脚（字数住进底栏），
    /// 栏目的 `min_rect` 因此天然不等高——那不代表框不一样大。这里量两栏滚动容器的矩形：
    /// 它们才是用户看到的那两个框。故意各跑一次"有译文/无译文"，徽标行高度差也一并锁住。
    #[test]
    fn both_panes_get_the_same_box() {
        for with_translation in [true, false] {
            let mut state = state();
            if with_translation {
                state.translation = Some(translated("你好，世界"));
            }
            let ctx = measure_columns(&mut state);
            let source = scroll_rect(&ctx, "source");
            let translation = scroll_rect(&ctx, "translation");
            assert!(
                (source.height() - translation.height()).abs() <= 0.5,
                "两个框不一样高（译文={with_translation}）：源文 {} 译文 {}",
                source.height(),
                translation.height()
            );
            assert!(
                (source.min.y - translation.min.y).abs() <= 0.5,
                "两个框顶边没对齐（译文={with_translation}）：源文 y={} 译文 y={}",
                source.min.y,
                translation.min.y
            );
            assert!(
                source.max.y <= MIN_BODY_HEIGHT + 0.5,
                "页脚被挤出栏外（y={}），最小窗口下会被裁掉",
                source.max.y
            );
        }
    }

    /// W-1.16：粘贴 18,000 字符之后，源文框必须老老实实待在自己那一栏里，
    /// 多出来的内容进滚动容器（修复前编辑器按内容高度一路撑到窗口外，既裁不掉也滚不动）。
    ///
    /// 断言落在滚动容器的实测数据上而不是栏目的 `min_rect`：两栏的正文都画进"丢弃的子 Ui"
    /// 再手工推进光标（底边对齐就是这么做的），子 Ui 里溢出多少，`min_rect` 一概看不见——
    /// 这正是 D-1/D-3 那批布局测试放走这个缺陷的原因。
    #[test]
    fn a_huge_paste_keeps_the_source_editor_inside_its_pane() {
        let mut state = state();
        state.source = huge_text();
        state.translation = Some(translated(&huge_text()));
        state.sync_char_count();
        let ctx = measure_columns(&mut state);
        let source = scroll_rect(&ctx, "source");
        let translation = scroll_rect(&ctx, "translation");

        assert!(
            (source.height() - translation.height()).abs() <= 0.5,
            "长文粘贴之后两个框不一样大：源文 {} 译文 {}",
            source.height(),
            translation.height()
        );
        for (pane, rect) in [("source", source), ("translation", translation)] {
            let metrics = scroll_metrics(&ctx, pane);
            assert!(
                metrics.content > metrics.viewport * 3.0,
                "{pane} 栏的内容应当远高于一屏，才谈得上滚动：{metrics:?}"
            );
            assert!(
                metrics.viewport > 0.0 && rect.max.y <= MIN_BODY_HEIGHT + 0.5,
                "{pane} 栏的可视窗口（{rect:?}）底边越出了栏目可用高 {MIN_BODY_HEIGHT}，框被内容撑出窗口了"
            );
        }
    }

    /// W-1.16：内容超出一屏时，两栏都要能用滚轮滚下去。
    /// 用户报的是"左边无法下拉、右边也没有下拉"，所以两栏都断言。
    #[test]
    fn both_panes_scroll_when_the_content_does_not_fit() {
        let mut state = state();
        state.source = huge_text();
        state.translation = Some(translated(&huge_text()));
        let mut effects = Effects::default();

        for (pane, x) in [("source", 0.25), ("translation", 0.75)] {
            let offset = wheel_scroll(
                &mut state,
                &mut effects,
                pane,
                egui::pos2(1080.0 * x, 360.0),
            );
            assert!(
                offset > 1.0,
                "{pane} 栏滚了 8 帧滚轮却一动不动（offset={offset}）"
            );
        }
    }

    /// REQ-013：底栏自占一行，排在两栏之后。四边都在窗口内，且不压任何一栏的正文可视区
    /// ——上一版它是浮层，正文靠预留躲它，预留公式与实画公式不同步就又盖住一行。
    #[test]
    fn the_bottom_bar_occupies_its_own_row_below_both_panes() {
        let mut state = state();
        state.translation = Some(translated("你好，世界"));
        state.sync_char_count();
        let mut effects = Effects::default();
        let ctx = run_text_page(&mut state, &mut effects, 1080.0, 2);
        let bar = bar_rect(&ctx);
        let win = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1080.0, 720.0));

        assert!(
            bar.min.x >= win.min.x - 0.5 && bar.max.x <= win.max.x + 0.5,
            "底栏横向溢出窗口：{bar:?}（窗口 {win:?}）"
        );
        assert!(
            bar.max.y <= win.max.y + 0.5,
            "底栏被挤出窗口，窄窗里会看不见：{bar:?}"
        );
        for pane in ["source", "translation"] {
            let body = scroll_rect(&ctx, pane);
            assert!(
                body.max.y <= bar.min.y + 0.5,
                "{pane} 栏的可视区压进了底栏：正文底边 {} 条子上沿 {}",
                body.max.y,
                bar.min.y
            );
        }
    }

    /// 退让顺序是纯函数（与字体度量无关）：标准档排得下就用标准档；排不下先把整排调窄一档；
    /// 窄档还是排不下才换行。不许压字号，也不许横向溢出。
    #[test]
    fn the_bar_shrinks_before_it_wraps() {
        // 三枚按钮，标准档一行（3×84 + 2×6 = 264）要 264px 以上的按钮区
        assert_eq!(bar_layout(300.0, 3), (BUTTON_SIZE, 1));
        // 264 与 3×64+2×6=204 之间：标准档排不下，窄档一行放得下 → 降宽度不降高度
        assert_eq!(bar_layout(230.0, 3), (BUTTON_SIZE_NARROW, 1));
        // 连一枚窄按钮都排不下时一行只给一枚，行数跟着按钮数走，绝不给 0 行
        assert_eq!(bar_layout(10.0, 3), (BUTTON_SIZE_NARROW, 3));
        assert_eq!(
            bar_height(BUTTON_SIZE_NARROW, 3),
            3.0 * BUTTON_SIZE_NARROW.y + 2.0 * BAR_SPACING + 2.0 * BAR_MARGIN
        );
        // 四枚（失败时多一枚"去设置"）在窄档也排不下 → 换行，行高跟着长
        assert_eq!(bar_layout(230.0, 4), (BUTTON_SIZE_NARROW, 2));
        assert!(
            bar_height(BUTTON_SIZE_NARROW, 2) > bar_height(BUTTON_SIZE_NARROW, 1),
            "换行没有让底栏长高，第二排会被裁掉"
        );
    }

    /// 窄窗（400px）里底栏换行是**长高**，两栏正文跟着让出那几像素，谁也不溢出窗口
    #[test]
    fn a_narrow_window_wraps_the_bar_instead_of_overflowing() {
        let mut state = state();
        state.failure = Some(Failure {
            message: "API Key 无效".into(),
            action: Some(FailureAction::GoSettings),
        });
        let mut effects = Effects::default();
        let ctx = run_text_page(&mut state, &mut effects, 400.0, 2);
        let bar = bar_rect(&ctx);

        assert!(
            bar.max.x <= 400.0 + 0.5,
            "窄窗里底栏横向溢出：{bar:?}（窗口宽 400）"
        );
        assert!(
            bar.height() > BUTTON_SIZE.y + 2.0 * BAR_MARGIN,
            "四枚按钮在 400px 宽应当换成两排：{bar:?}"
        );
        let body = scroll_rect(&ctx, "translation");
        assert!(
            body.max.y <= bar.min.y + 0.5,
            "底栏长高后压到了译文可视区：正文底边 {} 条子上沿 {}",
            body.max.y,
            bar.min.y
        );
    }

    /// 走查缺陷（用户 2026-10-27）：长译文滚到底，最后一行被右下角动作条盖住。
    ///
    /// v0.7 起底栏自占一行，所以这条断言的是结构而不是预留：滚到底时译文可视区的底边
    /// 就在底栏上沿之上，正文后面不许再垫任何"预留空白"（垫了就是永远滚不到的假滚动条）。
    #[test]
    fn the_last_line_stops_above_the_bottom_bar() {
        let mut state = state();
        state.translation = Some(translated(&huge_text()));
        state.sync_char_count();
        let mut effects = Effects::default();
        let ctx = egui::Context::default();
        let mut time = 0.0;
        for _ in 0..40 {
            let mut raw = raw_input(1080.0, 720.0);
            raw.time = Some(time);
            time += 1.0 / 60.0;
            raw.events
                .push(egui::Event::PointerMoved(egui::pos2(810.0, 360.0)));
            raw.events.push(egui::Event::MouseWheel {
                unit: egui::MouseWheelUnit::Point,
                delta: egui::vec2(0.0, -400.0),
                phase: egui::TouchPhase::Move,
                source: egui::MouseWheelSource::Trackpad,
                modifiers: egui::Modifiers::NONE,
            });
            ctx.run_ui(raw, |ui| show(ui, &mut state, &mut effects))
                .drop_without_applying_deltas();
        }

        let (inner, content, offset) = ctx
            .data_mut(|d| {
                d.get_temp::<(egui::Rect, egui::Vec2, egui::Vec2)>(scroll_id("translation"))
            })
            .expect("译文栏没有滚动容器");
        let bar = bar_rect(&ctx);

        // 先确认真滚到了底，否则下面那条"没被盖住"是白过的
        assert!(
            offset.y >= content.y - inner.height() - 0.5,
            "没滚到底：offset={} 内容={} 可视={}",
            offset.y,
            content.y,
            inner.height()
        );
        assert!(
            inner.max.y <= bar.min.y + 0.5,
            "译文可视区长进了底栏：视口底边 {} 条子上沿 {}",
            inner.max.y,
            bar.min.y
        );
        // 视口底边到条子上沿之间只该有译文栏自己的徽标行（`PANE_FOOTER_HEIGHT`）、
        // 它上面一条行距（3px）和底栏的上内边距，合计 ≈35px；再多的就是凭空留出的空白
        assert!(
            bar.min.y - inner.max.y <= PANE_FOOTER_HEIGHT + 3.0 + BAR_MARGIN + 5.0,
            "正文与底栏之间空出 {:?}px，比徽标行那一条还多",
            bar.min.y - inner.max.y
        );
    }

    fn run_full(state: &mut AppState, effects: &mut Effects) {
        let ctx = egui::Context::default();
        let output = ctx.run_ui(raw_input(1080.0, 720.0), |ui| show(ui, state, effects));
        output.drop_without_applying_deltas();
    }

    #[test]
    fn copy_buttons_refuse_to_touch_the_clipboard_without_a_translation() {
        let mut state = state();
        let mut effects = Effects::default();
        for kind in [CopyKind::Translation, CopyKind::Bilingual] {
            copy(&mut state, &mut effects, kind);
        }
        assert!(effects.copy.is_none());
        assert_eq!(state.toasts.len(), 2);
        assert_eq!(state.toasts[0].text, EMPTY_TRANSLATION_HINT);
        assert_eq!(state.toasts[0].tone, Tone::Warn);
        // 源文不能被这次失败的动作改动
        assert_eq!(state.source, "hello world");
    }

    #[test]
    fn copy_translation_queues_only_the_translation() {
        let mut state = state();
        state.translation = Some(translated("你好，世界"));
        let mut effects = Effects::default();

        copy(&mut state, &mut effects, CopyKind::Translation);
        assert_eq!(effects.copy.as_deref(), Some("你好，世界"));
        assert!(state.toasts.is_empty(), "成功提示要等真的写进剪贴板再给");
    }

    #[test]
    fn copy_bilingual_queues_source_and_translation() {
        let mut state = state();
        state.translation = Some(translated("你好，世界"));
        let mut effects = Effects::default();

        copy(&mut state, &mut effects, CopyKind::Bilingual);
        assert_eq!(effects.copy.as_deref(), Some("hello world\n\n你好，世界"));
    }

    #[test]
    fn failure_action_go_settings_switches_tab_and_focuses_the_key_field() {
        let mut state = state();
        state.failure = Some(Failure {
            message: ProviderError::Unauthorized.to_string(),
            action: Some(FailureAction::GoSettings),
        });
        let mut effects = Effects::default();

        failure_action(&mut state, &mut effects, FailureAction::GoSettings);

        assert_eq!(state.tab, crate::state::Tab::Settings);
        assert!(state.focus_key_input);
        assert!(effects.job.is_none());
    }

    #[test]
    fn failure_action_use_machine_resubmits_with_the_machine_engine() {
        let mut state = state();
        state.engine = EngineKind::Ai;
        state.failure = Some(Failure {
            message: ProviderError::Timeout(15_000, 2).to_string(),
            action: Some(FailureAction::UseMachine),
        });
        let mut effects = Effects::default();

        failure_action(&mut state, &mut effects, FailureAction::UseMachine);

        let job = effects.job.expect("应立即重译");
        assert_eq!(job.engine, EngineKind::Machine);
        assert_eq!(job.request.text, "hello world");
        assert_eq!(state.engine, EngineKind::Machine);
        assert!(state.failure.is_none(), "重译时旧错误要清掉");
    }

    #[test]
    fn source_editor_focus_is_recorded_for_the_next_frame() {
        let mut state = state();
        let mut effects = Effects::default();
        run_full(&mut state, &mut effects);
        assert!(!state.focus_in_source, "刚打开时焦点不在源文里");

        // 模拟用户点进源文框：egui 的焦点是 memory 里的状态
        let ctx = egui::Context::default();
        ctx.memory_mut(|mem| mem.request_focus(egui::Id::unique(SOURCE_EDITOR_ID)));
        let output = ctx.run_ui(raw_input(1080.0, 720.0), |ui| {
            show(ui, &mut state, &mut effects)
        });
        output.drop_without_applying_deltas();
        assert!(state.focus_in_source, "Enter 能不能翻译全靠这个标记");
    }

    #[test]
    fn char_count_shows_detected_language_only_in_auto_mode() {
        let mut state = state();
        state.sync_char_count();
        state.translation = Some(translated("你好"));
        assert_eq!(char_count_text(&state), "字数 11 · 检测：英语");

        state.config.translation.source = Some(Language::En);
        assert_eq!(char_count_text(&state), "字数 11");
    }

    #[test]
    fn a_cached_result_reports_the_cache_and_no_elapsed_time() {
        let mut state = state();
        state.translation = Some(TranslationResult::cached("你好", EngineKind::Machine));
        let mut effects = Effects::default();
        // 渲染不 panic 即可；标记内容由 result 字段决定，这里断言判定分支
        run_full(&mut state, &mut effects);
        let result = state.translation.as_ref().unwrap();
        assert!(result.cache_hit);
        assert_eq!(result.elapsed_ms, 0);
    }

    /// REQ-002 的 20,000 字符假设（spec 里标 HYPOTHESIS，实现第一天出结论）。
    /// 预算给到 100ms/帧：60fps 的单帧是 16.7ms，但无头这一帧包含首次字形栅格化，
    /// 留足余量；真实手感以用户粘贴 18k 字符的走查为准（W-2）。
    #[test]
    fn twenty_thousand_characters_layout_within_budget() {
        let mut state = state();
        // 中英混排，覆盖 CJK 字形与换行两条路径
        state.source = "the quick brown fox 跳过了懒狗。".repeat(1_100);
        assert!(state.source.chars().count() > 20_000);
        let mut effects = Effects::default();

        let started = std::time::Instant::now();
        run_full(&mut state, &mut effects);
        let elapsed = started.elapsed();

        state.sync_char_count();
        assert!(state.char_count > 20_000);
        println!("20k 字符单帧布局耗时：{elapsed:?}");
        assert!(
            elapsed.as_millis() < 100,
            "20k 字符单帧布局用了 {elapsed:?}，超过 100ms 预算（REQ-002 的 HYPOTHESIS 不成立）"
        );
    }
}
