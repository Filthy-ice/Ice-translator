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
    /// 切换置顶（K-09）。快捷键那条路径直接在 `app.rs::run` 里做，这里给标题栏图钉用
    ///（变更 I：简洁窗动作行那颗文字开关删了，界面侧只剩 `system::pin` 一个入口）
    pub toggle_topmost: bool,
    /// 退出程序（REQ-011）：界面侧只剩设置页那一颗按钮（用户 2026-10-28 变更 J 把悬浮窗
    /// 右下角那颗也删了："我既然用悬浮窗了，大概率是长期使用了"），另加 `Cmd+Q`。
    /// 走的都是 `Action::Quit` 这一条路径，不另开第二条退出实现。
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
    toasts(&ctx, state);
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

/// 主题滑动开关。用户 2026-10-28 变更 J 的原话："放在右下做成一个类似于开关左右推动的效果…
/// 你可以做成开关盖住太阳，露出月亮，就是暗色，如果盖住月亮，露出太阳就是浅色，在中间就是
/// 自动。而且全量界面也要有这个开关，不需要提示，看到的人都懂。也在右下角。"
///
/// 所以判据是**露出来的那一侧**，不是滑块所在的一侧：滑块压在马上（左端）时太阳被盖住、
/// 月亮露着 = 深色；滑块压在右端时反过来 = 浅色；停在中间两枚都露一半 = 自动。
///
/// 三条口径：
/// ① 三态仍是既有的 `config::Theme::{Light, Dark, System}`，这里只写
///    `state.config.ui.theme` + 标脏，生效仍在 `theme::apply`（`app.rs` 每帧调）——
///    与设置页外观分区那组单选、与那颗框都是**同一个配置项**，不另开第二套深浅色实现；
/// ② **没有悬停提示、没有文字**（"不需要提示，看到的人都懂"），变更 F 那档「自动」两字
///    随之作废；
/// ③ 太阳/月亮是矢量线稿，不是 `☀`/`☾` 字符——egui 自带字体没这些字形，真机落成豆腐块
///    （P4 缺陷 D-5 那条教训）。
///
/// `salt` 用来把矩形记进 `Context::data`，让两种形态各自的测试能同时找到那格。
pub(crate) const THEME_SWITCH_W: f32 = 40.0;
/// 轨道高 = 滑块直径 + 一道笔画，竖向不撑高所在那一行（同一行还有约 28px 的按钮）。
pub(crate) const THEME_SWITCH_H: f32 = 20.0;
/// 滑块半径：太阳光芒外沿 7.6、月亮半径 7，8px 的实心圆刚好把它们整个盖住——
/// "盖住"是这块控件的全部语义，盖不住就等于没做到。
pub(crate) const KNOB_RADIUS: f32 = 8.0;

/// 三档在轨道上的位置（0 = 左端盖住太阳，1 = 右端盖住月亮）
pub(crate) fn theme_slot(theme: ice_core::config::Theme) -> f32 {
    use ice_core::config::Theme;
    match theme {
        Theme::Dark => 0.0,
        Theme::System => 0.5,
        Theme::Light => 1.0,
    }
}

/// 轨道上某一点落在哪一档：取最近的一档，所以左四分之一是深色、右四分之一是浅色、
/// 中间一半都是自动。拖动时不会在两档边界抖——指针要退回 0.25 才换档。
pub(crate) fn theme_at(t: f32) -> ice_core::config::Theme {
    use ice_core::config::Theme;
    if t < 0.25 {
        Theme::Dark
    } else if t > 0.75 {
        Theme::Light
    } else {
        Theme::System
    }
}

fn theme_switch_id(salt: &'static str) -> egui::Id {
    egui::Id::NULL
        .with("ice-translator.theme-switch")
        .with(salt)
}

pub(crate) fn theme_switch(
    ui: &mut egui::Ui,
    state: &mut AppState,
    salt: &'static str,
) -> egui::Rect {
    // 只占位不响应（`allocate_at_least` 会自带一次 interact，两次点击判定会让同一枚
    // 指针被两个响应者各消费一遍）；真正的命中交给下面那个带稳定 `Id` 的 `interact`
    let (_auto_id, rect) = ui.allocate_space(egui::vec2(THEME_SWITCH_W, THEME_SWITCH_H));
    let visuals = ui.visuals();
    let painter = ui.painter_at(rect);
    let center_y = rect.center().y;
    // 两端滑块的圆心：滑块贴着轨道两头，所以内缩一个半径
    let left_x = rect.min.x + KNOB_RADIUS;
    let right_x = rect.max.x - KNOB_RADIUS;
    let icon_stroke = egui::Stroke::new(1.5, visuals.widgets.inactive.fg_stroke.color);

    painter.rect_filled(
        rect,
        rect.height() / 2.0,
        visuals.widgets.noninteractive.bg_fill,
    );
    painter.rect_stroke(
        rect,
        rect.height() / 2.0,
        visuals.widgets.noninteractive.bg_stroke,
        egui::StrokeKind::Inside,
    );
    draw_sun(&painter, egui::pos2(left_x, center_y), icon_stroke);
    draw_moon(&painter, egui::pos2(right_x, center_y), icon_stroke);
    // 滑块最后画：它是不透明的那块圆，盖住哪一侧就露出另一侧
    let t = theme_slot(state.config.ui.theme);
    let knob = egui::pos2(left_x + (right_x - left_x) * t, center_y);
    painter.circle_filled(knob, KNOB_RADIUS, visuals.widgets.inactive.bg_fill);
    painter.circle_stroke(knob, KNOB_RADIUS, icon_stroke);

    let response = ui.interact(rect, theme_switch_id(salt), egui::Sense::click_and_drag());
    if response.dragged() || response.clicked() {
        if let Some(pos) = response.interact_pointer_pos() {
            let t = ((pos.x - rect.min.x) / rect.width().max(1.0)).clamp(0.0, 1.0);
            let next = theme_at(t);
            if next != state.config.ui.theme {
                state.config.ui.theme = next;
                state.mark_config_dirty();
            }
        }
    }
    ui.ctx()
        .data_mut(|d| d.insert_temp(theme_switch_id(salt), rect));
    rect
}

#[cfg(test)]
pub(crate) fn recorded_theme_switch(ctx: &egui::Context, salt: &'static str) -> egui::Rect {
    ctx.data_mut(|d| d.get_temp::<egui::Rect>(theme_switch_id(salt)))
        .unwrap_or_else(|| panic!("{salt} 那一侧的主题开关没有记下矩形"))
}

/// 一段圆弧采样成折线（epaint 0.37 没有圆弧图元，只能自己按角度取点）
fn arc_points(
    center: egui::Pos2,
    radius: f32,
    from: f32,
    to: f32,
    steps: usize,
) -> Vec<egui::Pos2> {
    (0..=steps)
        .map(|i| {
            let angle = from + (to - from) * i as f32 / steps as f32;
            center + egui::vec2(angle.cos(), angle.sin()) * radius
        })
        .collect()
}

/// 太阳：一圈描边的圆 + 八道光芒。全是线稿，与月亮同一套笔画粗细。
fn draw_sun(painter: &egui::Painter, at: egui::Pos2, stroke: egui::Stroke) {
    painter.circle_stroke(at, 3.6, stroke);
    for index in 0..8 {
        let angle = std::f32::consts::TAU * index as f32 / 8.0;
        let dir = egui::vec2(angle.cos(), angle.sin());
        painter.line_segment([at + dir * 5.4, at + dir * 7.6], stroke);
    }
}

/// 月亮：月牙轮廓 = 外弧 + 内弧，两段弧共用两个尖端。
///
/// 只描边不填充：`PathShape` 的 `fill` 只支持凸多边形（`path_shape.rs:16`），月牙不是凸的，
/// 填出来会画歪。外弧取整圆的左侧 230°，内弧的圆心右移 `BIAS`、半径取到同样穿过那两个
/// 尖端，两弧之间剩下的就是月牙。
fn draw_moon(painter: &egui::Painter, at: egui::Pos2, stroke: egui::Stroke) {
    const TIP: f32 = 65.0_f32.to_radians();
    const R: f32 = 7.0;
    const BIAS: f32 = 3.2;
    const STEPS: usize = 20;
    let inner_center = at + egui::vec2(BIAS, 0.0);
    // `+y` 朝下，所以这是月牙朝下的那个尖端；它的极角就是内弧的端点角
    let tip = at + egui::vec2(R * TIP.cos(), R * TIP.sin());
    let offset = tip - inner_center;
    let mut points = arc_points(at, R, TIP, std::f32::consts::TAU - TIP, STEPS);
    points.extend(arc_points(
        inner_center,
        offset.length(),
        std::f32::consts::TAU - offset.angle(),
        offset.angle(),
        STEPS,
    ));
    painter.add(egui::Shape::Path(egui::epaint::PathShape::closed_line(
        points, stroke,
    )));
}

/// 提示条：**窗口正中**，最多三条，出生即开始线性淡出，`TOAST_LIFETIME`（500ms）到点消失。
///
/// 用户 2026-10-28 变更 I 的原话："所有的提示类弹窗都是中心弹窗500ms内淡化消失，
/// 不应该有关闭的操作和按钮。"三条各自对应一处删除：
/// ① 不再挂在顶部（原来全量窗贴顶 12px、简洁窗另算一套越过输入行的偏移）而落在正中；
/// ② 不再四秒常驻，整条 lifetime 就是那次淡出；
/// ③ 那颗 `×` 连同 `AppState::dismiss_toast` 一起没了，Area 也改成 `interactable(false)`——
///    不只是"少画一颗按钮"，而是**这一层根本不参与命中测试**，提示条盖住的按钮照样能点。
///
/// 淡的是整块（底色、描边、投影、文字一起乘同一个 alpha），所以不会出现"框还在、字先没了"。
/// 二次确认框（`confirm`）不在这条规矩里：它要用户做决定，不是"说一声就走"的提示。
pub(crate) fn toasts(ctx: &egui::Context, state: &mut AppState) {
    if state.toasts.is_empty() {
        return;
    }
    let band = egui::Area::new(egui::Id::unique("toasts"))
        .anchor(Align2::CENTER_CENTER, [0.0, 0.0])
        .order(Order::Foreground)
        .interactable(false)
        .show(ctx, |ui| {
            ui.vertical(|ui| {
                for toast in state.toasts.iter() {
                    let alpha = toast.opacity();
                    // `multiply_with_opacity` 一次处理底色、描边、投影（egui 自己那三行就是它）
                    egui::Frame::popup(ui.style())
                        .multiply_with_opacity(alpha)
                        .show(ui, |ui| {
                            ui.label(
                                RichText::new(&toast.text)
                                    .color(theme::tone_color(toast.tone).gamma_multiply(alpha)),
                            );
                        });
                }
            });
        })
        .response
        .rect;
    ctx.data_mut(|d| d.insert_temp(toast_band_id(), band));
}

/// 提示条那一带的矩形，只给无头断言用（判"是不是落在窗口正中"、"点上去会不会把它点掉"）
#[cfg(test)]
pub(crate) fn recorded_toast_band(ctx: &egui::Context) -> Option<egui::Rect> {
    ctx.data_mut(|d| d.get_temp::<egui::Rect>(toast_band_id()))
}

fn toast_band_id() -> egui::Id {
    egui::Id::NULL.with("ice-translator.toast-band")
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
        // 直接把时间戳拨过整条 lifetime，免得测试真等那 500ms
        state.toasts[0].shown_at -= crate::state::TOAST_LIFETIME * 2;
        assert!(state.tick_toasts());
        assert!(state.toasts.is_empty());
    }

    /// 变更 I 的前半句："所有的提示类弹窗都是中心弹窗"（用户 2026-10-28）。
    /// 判据落在实测矩形上：提示条那一带的中心就是窗口中心——横竖都居中，
    /// 不再贴顶 12px，也不再为简洁窗另算一套"越过输入行"的偏移。
    #[test]
    fn toasts_are_painted_at_the_window_centre() {
        let (_dir, store) = temp_history();
        let mut state = state();
        state.toast(Tone::Info, "已复制到剪贴板");
        let mut effects = Effects::default();
        let screen = egui::vec2(1080.0, 720.0);
        let ctx = run_frame(&mut state, &mut effects, &store, screen, 2);

        let band = recorded_toast_band(&ctx).expect("提示条那一带要记录下来");
        let centre = band.center();
        let want = egui::pos2(screen.x / 2.0, screen.y / 2.0);
        assert!(
            centre.distance(want) <= 1.0,
            "提示条没落在窗口正中：量到 {centre:?}，期望 {want:?}（带子 {band:?}）"
        );
    }

    /// 变更 I 的后半句："不应该有关闭的操作和按钮"。
    /// 这里不数按钮，量的是**行为**：把鼠标移到提示条正中间按下再松开，那条还在——
    /// 提示条只剩"自己到点淡出"这一种结束方式。将来谁再补一颗 `×`，这条会红。
    #[test]
    fn a_click_on_a_toast_does_not_dismiss_it() {
        let (_dir, store) = temp_history();
        let mut state = state();
        state.toast(Tone::Info, "已复制到剪贴板");
        let mut effects = Effects::default();
        let screen = egui::vec2(1080.0, 720.0);
        let ctx = run_frame(&mut state, &mut effects, &store, screen, 2);
        let at = recorded_toast_band(&ctx)
            .expect("先量到提示条那一带")
            .center();

        run_click(&mut state, &mut effects, &store, screen, at);
        assert_eq!(
            state.toasts.len(),
            1,
            "点在提示条上就把它点掉了——变更 I 要求提示类弹窗没有关闭操作"
        );
    }

    /// `run_frame` 只喂空事件，这里要的是"真的点一下"，所以单独一个入口
    ///（与 `run_shapes` 同一先例：不为一条测试去改大家都用的那个）。
    fn run_click(
        state: &mut AppState,
        effects: &mut Effects,
        history: &HistoryStore,
        screen: egui::Vec2,
        at: egui::Pos2,
    ) -> egui::Context {
        let ctx = egui::Context::default();
        let mut time = 1.0;
        let steps = [
            vec![],
            vec![egui::Event::PointerMoved(at)],
            vec![egui::Event::PointerButton {
                pos: at,
                button: egui::PointerButton::Primary,
                pressed: true,
                modifiers: egui::Modifiers::NONE,
            }],
            vec![egui::Event::PointerButton {
                pos: at,
                button: egui::PointerButton::Primary,
                pressed: false,
                modifiers: egui::Modifiers::NONE,
            }],
        ];
        for events in steps {
            let mut raw = egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, screen)),
                ..Default::default()
            };
            raw.time = Some(time);
            time += 1.0 / 60.0;
            raw.events = events;
            let output = ctx.run_ui(raw, |ui| {
                show(ui, state, effects, history, false);
            });
            output.drop_without_applying_deltas();
        }
        ctx
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
