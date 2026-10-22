//! 文本页的 B 区（源文）与 C 区（译文）：F-06~F-10。
//!
//! 绘制顺序即视觉顺序（工具条 → 源文 → 译文 → 状态条），Tab 键的焦点遍历才不会出现
//! "视觉在下面、焦点先到"的错位。动作条常驻译文栏内右下角，焦点排在译文之后。

use egui::{self, Align, Key, KeyboardShortcut, Layout, Modifiers, RichText};

use crate::state::{AppState, FailureAction, Tone};
use crate::system::clipboard::{self, CopyKind, EMPTY_TRANSLATION_HINT};
use crate::system::hotkeys::SOURCE_EDITOR_ID;
use crate::theme;

use super::Effects;

/// 悬浮动作条里按钮的最小尺寸：等宽排列，间距 6px（F-09）
const BUTTON_SIZE: egui::Vec2 = egui::Vec2::new(84.0, 28.0);

/// 悬浮动作条外框的内边距（px）
const BAR_MARGIN: f32 = 6.0;

/// 两栏页脚行（字数 / 徽标）的固定高度。
///
/// 徽标比纯文本高（内边距 4 + 小字号），空页脚又是 0 高——页脚高度各算各的，
/// 两栏的底边就会差出一截，真机走查里"两边窗口大小不一样"正是这么来的。
/// 固定成一行能装下徽标的高度，两栏底边才永远对齐。
const PANE_FOOTER_HEIGHT: f32 = 26.0;

/// 悬浮动作条的 id：不挂在任何 Ui 的谱系下，下一帧和布局测试才能用同一个 id 取回它的矩形。
fn bar_id() -> egui::Id {
    egui::Id::NULL.with("ice-translator.floating-actions")
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
    // 两栏按可用高度撑满，动作条悬浮在译文栏内部，不再单独占一行（F-09 悬浮化）
    ui.columns(2, |columns| {
        source_pane(&mut columns[0], state);
        translation_pane(&mut columns[1], state, effects);
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

/// B 区：源文编辑器（F-06）+ 字数与检测语言（F-07）
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
    exact_row(
        ui,
        PANE_FOOTER_HEIGHT,
        Layout::right_to_left(Align::Center),
        |ui| {
            ui.colored_label(theme::MUTED, char_count_text(state));
        },
    );
}

/// 两栏共用的正文高度算法：扣掉行间距与固定页脚，剩下的全给正文
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

/// C 区：译文展示（F-08）+ 缓存与耗时标记（F-10）+ 悬浮动作条（F-09）
fn translation_pane(ui: &mut egui::Ui, state: &mut AppState, effects: &mut Effects) {
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
                let inner = ui.max_rect();
                let scrolled = scroll_area("translation").show(ui, |ui| {
                    translation_body(ui, state);
                });
                record_scroll(ui.ctx(), "translation", &scrolled);
                floating_actions(ui, state, effects, inner);
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

/// F-09 动作条：自带外框，常驻译文框内右下角，不隐藏也不浮动。
///
/// 第二轮走查（2026-10-21）用户先要"悬浮浮出"，看到实物后改主意："干脆不要收起了，
/// 就放在右下角不动"。外框不用 popup（它的阴影会糊到译文框边框上，"边框有点遮挡"），
/// 改成填充＋描边、无阴影，并与译文框边框留出 `BAR_MARGIN` 的间隙。
fn floating_actions(
    ui: &mut egui::Ui,
    state: &mut AppState,
    effects: &mut Effects,
    inner: egui::Rect,
) {
    let ctx = ui.ctx().clone();
    let id = bar_id();

    let mut actions: Vec<BarAction> = vec![
        BarAction::Translate,
        BarAction::Copy(CopyKind::Translation),
        BarAction::Copy(CopyKind::Bilingual),
    ];
    if let Some(action) = state.failure.as_ref().and_then(|f| f.action) {
        actions.push(BarAction::Failure(action));
    }

    let spacing = 6.0;
    let outer = inner.expand(BAR_MARGIN);
    ctx.data_mut(|d| d.insert_temp(id.with("frame"), outer));

    // 放不下就换行，窄栏里也不许横向溢出译文框
    let max_content_w = (inner.width() - 2.0 * BAR_MARGIN).max(BUTTON_SIZE.x);
    let per_row = ((max_content_w + spacing) / (BUTTON_SIZE.x + spacing))
        .floor()
        .max(1.0) as usize;
    let rows = actions.len().div_ceil(per_row);
    let widest = actions.len().min(per_row);
    let content_w = widest as f32 * BUTTON_SIZE.x + (widest - 1) as f32 * spacing;
    let content_h = rows as f32 * BUTTON_SIZE.y + (rows - 1) as f32 * spacing;

    // 右下角固定位置：外框四边都离译文框边框 BAR_MARGIN，不压边
    let content_rect = egui::Rect::from_min_size(
        egui::pos2(
            inner.right() - BAR_MARGIN - content_w,
            inner.bottom() - BAR_MARGIN - content_h,
        ),
        egui::vec2(content_w, content_h),
    );
    let frame = egui::Frame::group(ui.style())
        .fill(ui.visuals().window_fill())
        .stroke(ui.visuals().window_stroke())
        .corner_radius(egui::CornerRadius::same(8))
        .inner_margin(egui::Margin::same(BAR_MARGIN as i8));
    ui.painter().add(frame.paint(content_rect));
    ctx.data_mut(|d| d.insert_temp(id, frame.outer_rect(content_rect)));

    // 按钮放进一个丢弃的子 Ui：`put` 会推进光标并把矩形并进父 Ui，
    // 动作条的矩形就会把译文框（乃至整栏）撑高
    let mut bar_ui = ui.new_child(egui::UiBuilder::new().max_rect(content_rect).layout(
        egui::Layout::centered_and_justified(egui::Direction::TopDown),
    ));
    for (index, action) in actions.iter().enumerate() {
        let rect = egui::Rect::from_min_size(
            content_rect.min
                + egui::vec2(
                    (index % per_row) as f32 * (BUTTON_SIZE.x + spacing),
                    (index / per_row) as f32 * (BUTTON_SIZE.y + spacing),
                ),
            BUTTON_SIZE,
        );
        let busy = state.in_flight.is_some();
        let response = bar_ui.put(
            rect,
            egui::Button::new(action.label(busy)).min_size(BUTTON_SIZE),
        );
        if response.clicked() {
            match action {
                BarAction::Translate => {
                    if !busy {
                        effects.request(state.submit());
                    }
                }
                BarAction::Copy(kind) => copy(state, effects, *kind),
                BarAction::Failure(action) => failure_action(state, effects, *action),
            }
        }
    }
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

    /// 跑若干帧文本页，返回 Context 与译文框的外矩形（动作条每帧记在临时数据里）
    fn run_text_page(
        state: &mut AppState,
        effects: &mut Effects,
        width: f32,
        frames: usize,
    ) -> (egui::Context, egui::Rect) {
        let ctx = egui::Context::default();
        let mut time = 0.0;
        for _ in 0..frames {
            let mut raw = raw_input(width, 720.0);
            raw.time = Some(time);
            time += 1.0 / 60.0;
            let output = ctx.run_ui(raw, |ui| show(ui, state, effects));
            output.drop_without_applying_deltas();
        }
        let frame = ctx
            .data_mut(|d| d.get_temp::<egui::Rect>(bar_id().with("frame")))
            .expect("译文框要记下自己的矩形");
        (ctx, frame)
    }

    fn bar_rect(ctx: &egui::Context) -> egui::Rect {
        ctx.data_mut(|d| d.get_temp::<egui::Rect>(bar_id()))
            .expect("动作条要记下自己的矩形")
    }

    /// 量出两栏各自画到的矩形
    fn measure_columns(
        state: &mut AppState,
        effects: &mut Effects,
    ) -> (egui::Context, egui::Rect, egui::Rect) {
        let ctx = egui::Context::default();
        let mut rects = (egui::Rect::NOTHING, egui::Rect::NOTHING);
        let output = ctx.run_ui(raw_input(640.0, MIN_BODY_HEIGHT), |ui| {
            ui.columns(2, |columns| {
                source_pane(&mut columns[0], state);
                translation_pane(&mut columns[1], state, effects);
                rects = (columns[0].min_rect(), columns[1].min_rect());
            });
        });
        output.drop_without_applying_deltas();
        (ctx, rects.0, rects.1)
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

    /// 两栏的底边必须对齐。
    ///
    /// 源文页脚是纯文本、译文页脚是徽标（更高），译文没出来时页脚又是空的——
    /// 页脚高度各算各的，两栏就一高一矮，真机走查截图里"两边窗口大小不一样"
    /// 正是这么来的。这里故意放上译文让徽标出现，高度差才暴露得出来。
    #[test]
    fn both_panes_end_at_the_same_line() {
        let mut state = state();
        state.translation = Some(translated("你好，世界"));
        let mut effects = Effects::default();
        let (_ctx, source, translation) = measure_columns(&mut state, &mut effects);
        assert!(
            (source.max.y - translation.max.y).abs() <= 0.5,
            "两栏底边没对齐：源文 y={} 译文 y={}",
            source.max.y,
            translation.max.y
        );
        assert!(
            source.max.y <= MIN_BODY_HEIGHT + 0.5,
            "页脚被挤出栏外（y={}），最小窗口下会被裁掉",
            source.max.y
        );
    }

    /// 没有译文时译文页脚是空的，两栏底边同样要对齐
    #[test]
    fn both_panes_align_even_with_an_empty_translation_footer() {
        let mut state = state();
        let mut effects = Effects::default();
        let (_ctx, source, translation) = measure_columns(&mut state, &mut effects);
        assert!(
            (source.max.y - translation.max.y).abs() <= 0.5,
            "源文 y={} 译文 y={}",
            source.max.y,
            translation.max.y
        );
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
        let mut effects = Effects::default();
        let (ctx, source, translation) = measure_columns(&mut state, &mut effects);

        assert!(
            source.max.y <= MIN_BODY_HEIGHT + 0.5,
            "页脚被挤出栏外（y={}），最小窗口下会被裁掉",
            source.max.y
        );
        assert!(
            (source.max.y - translation.max.y).abs() <= 0.5,
            "长文粘贴之后两栏底边错位：源文 y={} 译文 y={}",
            source.max.y,
            translation.max.y
        );
        for (pane, column) in [("source", source), ("translation", translation)] {
            let metrics = scroll_metrics(&ctx, pane);
            assert!(
                metrics.content > metrics.viewport * 3.0,
                "{pane} 栏的内容应当远高于一屏，才谈得上滚动：{metrics:?}"
            );
            assert!(
                metrics.viewport > 0.0 && metrics.viewport <= column.height() + 0.5,
                "{pane} 栏的可视窗口（{}px）超过了栏目高度（{}px），框被内容撑变形了",
                metrics.viewport,
                column.height()
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

    /// 动作条常驻译文框内右下角，且四边都不压译文框的边框（第二轮走查："边框有点遮挡"）
    #[test]
    fn the_action_bar_sits_in_the_corner_without_touching_the_pane_border() {
        let mut state = state();
        let mut effects = Effects::default();
        let (ctx, frame) = run_text_page(&mut state, &mut effects, 1080.0, 1);
        let bar = bar_rect(&ctx);
        assert!(
            bar.right() <= frame.right() - 4.0 && bar.bottom() <= frame.bottom() - 4.0,
            "动作条压到译文框边框上了：bar={bar:?} frame={frame:?}"
        );
        assert!(
            bar.left() >= frame.left() && bar.top() >= frame.top(),
            "动作条越出译文框：bar={bar:?} frame={frame:?}"
        );
    }

    /// 最小窗口下四个按钮一行放不下，要在译文框内换行而不是横向溢出
    #[test]
    fn the_action_bar_wraps_inside_a_narrow_pane() {
        let mut state = state();
        state.failure = Some(Failure {
            message: "API Key 无效".into(),
            action: Some(FailureAction::GoSettings),
        });
        let mut effects = Effects::default();
        let (ctx, frame) = run_text_page(&mut state, &mut effects, 640.0, 1);
        let bar = bar_rect(&ctx);
        assert!(
            bar.left() >= frame.left() - 0.5 && bar.right() <= frame.right() + 0.5,
            "窄栏里动作条横向溢出译文框：bar={bar:?} frame={frame:?}"
        );
        assert!(
            bar.height() > BUTTON_SIZE.y + 6.0,
            "四个按钮应当换行成两排：bar={bar:?}"
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
