//! 外观（FR-2.4 主题三态、FR-2.3 字号缩放、`03-ui-design.md` §1 的配色）。
//!
//! 颜色只在这里定义，各 view 不自带色值，免得改一次主题要翻四个文件。

use std::sync::Arc;

use egui::text::LayoutJob;
use egui::{Color32, FontId, Galley, RichText, Stroke, ThemePreference, Ui};
use ice_core::config::{Theme, UiConfig};

use crate::state::Tone;

/// 界面缩放的合法区间（与 `UiConfig::font_scale` 的约定一致）
pub const MIN_FONT_SCALE: f32 = 0.8;
pub const MAX_FONT_SCALE: f32 = 2.0;

/// 主色：按钮、选中态、链接
pub const ACCENT: Color32 = Color32::from_rgb(74, 132, 214);
/// 栏目标题渐变的中间色（`ACCENT` → 这里 → `TITLE_END`）
pub const TITLE_MID: Color32 = Color32::from_rgb(150, 92, 205);
/// 栏目标题渐变的最后一档
pub const TITLE_END: Color32 = Color32::from_rgb(226, 96, 150);
/// 渐变一条色带的宽度（px）——决定"源文""译文"这种两字标题被切成几段重画
pub const TITLE_BAND_PX: f32 = 3.0;
/// 错误红：翻译失败、钥匙串不可用
pub const ERROR: Color32 = Color32::from_rgb(214, 69, 65);
/// 降级黄：配置损坏启动、AI 超时改走机翻、AI 未配置
pub const WARN: Color32 = Color32::from_rgb(206, 145, 46);
/// 成功绿：复制完成、Key 保存成功
pub const OK: Color32 = Color32::from_rgb(76, 154, 96);
/// 次要文字：字数统计、耗时、缓存标记
pub const MUTED: Color32 = Color32::from_gray(128);

// ---- 冷灰白基底（只管浅色那一套，深色不动）----
//
// 用户 2026-11-04："界面基底(包括标题栏底色)能用冷灰白色最好，不要使用暖色，看起来好过时。"
// egui 自带的亮色是**纯灰**（gray(248)/gray(230)/gray(190)）——纯灰本身不暖，但它在暖光屏幕、
// 壁纸反光和 macOS 那层桌面模糊一起读出来就是"泛黄、上一代软件"。这里同一档亮度、
// 每一级都往蓝里偏 3~6，界面立刻变"冷白"。
//
// 只改**底与线**，不改文字色与 `ACCENT` 那一族：文字和主色是另一件事（主题包 #28 的边界）。

/// 界面底：中央面板、标题栏、窗底（原 gray(248)）
pub const COOL_BASE: Color32 = Color32::from_rgb(245, 248, 251);
/// 正文框底：源文与译文那一格的"可书写表面"（原 gray(255)）
pub const COOL_SURFACE: Color32 = Color32::from_rgb(251, 253, 255);
/// 比底深一档的填充：按钮、复选框、滑块槽（原 gray(230)）
pub const COOL_RAISED: Color32 = Color32::from_rgb(228, 233, 239);
/// 悬停那一档（比 `COOL_RAISED` 再深）
pub const COOL_HOVER: Color32 = Color32::from_rgb(217, 224, 233);
/// 按下那一档（再深一档，配合 `expansion` 的内缩就是"陷下去"）
pub const COOL_PRESSED: Color32 = Color32::from_rgb(203, 212, 224);
/// 分隔线与控件描边（原 gray(190)）
pub const COOL_EDGE: Color32 = Color32::from_rgb(197, 205, 214);

/// **整窗**的基底色：原生标题栏那条带子与 egui 的内容区共用这一个出处。
///
/// 分层只有两级（规矩与病因见 `system::background`）：基底铺满整窗，输入框与译文框用同色系
/// 浅一档的 `extreme_bg_color`。深色那一档不再造一个新数，直接读 egui 自己 `Visuals::dark()`
/// 的 `window_fill`——内容区画什么，标题栏就得是什么；两处各自持色就是"半改完"的来源。
///
/// 将来五套预设色（任务 #28）只喂这一个字段给那两处，别在 view 里另配底色。
pub fn window_base(dark: bool) -> Color32 {
    if dark {
        egui::Visuals::dark().window_fill
    } else {
        COOL_BASE
    }
}

/// 滑块那颗圆疙瘩的三层色：接触投影、背光本体、受光面。
///
/// 用户 2026-11-04："开关那个圆疙瘩能做立体感更好了。" egui 0.37 没有渐变图元（`Shape`
/// 里找不到径向渐变），球感只能靠**同色系三档圆盘错位叠**：本体暗一档、受光面亮一档并往上偏，
/// 本体在下缘露出一圈背光——那条不对称就是"凸起来"的全部来源。
///
/// 颜色定在这里而不是画它的地方，规矩同 [`COOL_BASE`]：各 view 不自带色值。
pub struct KnobTones {
    pub drop: Color32,
    pub body: Color32,
    pub lit: Color32,
}

pub const fn knob_tones(dark: bool) -> KnobTones {
    if dark {
        KnobTones {
            drop: Color32::from_black_alpha(140),
            body: Color32::from_gray(118),
            lit: Color32::from_gray(182),
        }
    } else {
        KnobTones {
            drop: Color32::from_black_alpha(44),
            body: Color32::from_rgb(222, 228, 236),
            lit: COOL_SURFACE,
        }
    }
}

/// 正文框底那一圈淡阴影（用户 2026-11-04："淡淡的阴影就行，你看别人的软件立体感好强"）。
///
/// 用 `Frame::shadow` 而不是自己叠矩形：egui 0.37 的阴影会走 `RectShape { blur_width }`
/// 那条软边通道（`Shape` 枚举里**没有** `ShadowShape` 这个变体，它被包进 `Shape::Vec` 交出来），
/// 手画的偏移矩形只有硬边。偏移往下 2 档、模糊 6 档、
/// alpha 34——再深就不是"淡"，是两条框各自浮起来一块黑板。
pub const fn pane_shadow(dark: bool) -> egui::Shadow {
    egui::Shadow {
        // `Shadow` 在 0.37 用整数档（`offset: [i8; 2]`、`blur/spread: u8`），没有半像素可调
        offset: [0, 2],
        blur: 6,
        spread: 0,
        color: Color32::from_black_alpha(if dark { 90 } else { 34 }),
    }
}

/// 两端色之间按 `t`∈[0,1] 插值（逐分量取整，短渐变用不着走线性色彩空间）
pub fn mix(from: Color32, to: Color32, t: f32) -> Color32 {
    let t = t.clamp(0.0, 1.0);
    let lerp = |a: u8, b: u8| (a as f32 + (b as f32 - a as f32) * t).round() as u8;
    Color32::from_rgb(
        lerp(from.r(), to.r()),
        lerp(from.g(), to.g()),
        lerp(from.b(), to.b()),
    )
}

/// 栏目标题渐变上 `t` 位置的色（`t`∈[0,1]，由调用方按"这一条色带在整行里的位置"算出来）。
///
/// 三段而不是两端：一行只有"源文"两个字时，两个端点色之间线性插值看起来就是"一个蓝字
/// 一个紫字"，那是拼色不是渐变（用户 2026-10-28 那句"你搞两个不同颜色的字放一起就叫艺术字
/// 了？"）。加一个中间色，颜色在整行宽度上先走蓝→紫、再走紫→玫，两端读者能看出走向。
pub fn title_color(t: f32) -> Color32 {
    let t = t.clamp(0.0, 1.0);
    if t < 0.5 {
        mix(ACCENT, TITLE_MID, t * 2.0)
    } else {
        mix(TITLE_MID, TITLE_END, (t - 0.5) * 2.0)
    }
}

/// 一行标题切成几条色带：每条宽 [`TITLE_BAND_PX`]，最少 8 条（再少就看得出台阶），
/// 最多 32 条（两个字用不着几十遍重画）。
pub fn title_bands(width: f32) -> usize {
    ((width / TITLE_BAND_PX).ceil() as usize).clamp(8, 32)
}

/// 只按 `factor` 缩放 rgb 三个分量，alpha 原样保留。
///
/// `Color32::gamma_multiply` 把 alpha 也一起乘了——直接拿它调控件底色会把不透明改成
/// 半透明，而半透明的填充正是一类反复复发的毛病（D-12：那条带变成透出桌面的洞）。
fn scale_rgb(color: Color32, factor: f32) -> Color32 {
    let scaled = color.gamma_multiply(factor);
    Color32::from_rgb(scaled.r(), scaled.g(), scaled.b())
}

/// 两个正文框（源文 / 译文）的底色：egui 给"可书写表面"用的那个颜色。
///
/// 用户 2026-10-28："这两个框怎么和软件其他的框颜色一样？"——`Frame::group` 本身不带
/// 底色（只有一圈 stroke），框里框外都是 `panel_fill`，两个正文区就融进了背景。
pub fn surface_fill(visuals: &egui::Visuals) -> Color32 {
    visuals.text_edit_bg_color()
}

/// 一个「正文框」该长什么样：**底色与阴影在同一处定义**——全量窗那两个框与精简窗那两条带共用它。
///
/// 为什么必须连底色一起给，而不是只给调用方补一个阴影：`Frame::group` 的 `fill` 是**透明的**
/// （`egui-0.37.0/src/containers/frame.rs:163`，`NONE` 那份常量就是 `Color32::TRANSPARENT`）。
/// 只挂阴影不铺底，那圈软阴影就从盒子的**底下透上来糊满整格**——用户 2026-11-05 看到的
/// "整个都是灰呼呼的，和全量界面的文本框区别太大了"就是这么来的，缺一层绘制而不是审美差异。
/// 底色取 [`surface_fill`]（"可书写表面"那一档近白），于是三件事一次配齐：**白卡、描边、投影**。
///
/// 描边留给调用方覆盖（源文框要跟着焦点换色），几何也留给调用方（两种形态的内边距与圆角本就不同）
/// ——**共用的是"长什么样"，不是"占多大地方"**，这条界线别抹掉。
pub fn pane_frame(style: &egui::Style) -> egui::Frame {
    egui::Frame::group(style)
        .fill(surface_fill(&style.visuals))
        .shadow(pane_shadow(style.visuals.dark_mode))
}

pub const fn tone_color(tone: Tone) -> Color32 {
    match tone {
        Tone::Info => ACCENT,
        Tone::Warn => WARN,
        Tone::Error => ERROR,
    }
}

pub const fn preference(theme: Theme) -> ThemePreference {
    match theme {
        Theme::System => ThemePreference::System,
        Theme::Light => ThemePreference::Light,
        Theme::Dark => ThemePreference::Dark,
    }
}

/// 字号缩放：以 egui 的默认字号为基准乘系数，亮暗两套样式都改。
///
/// 基准每帧重取而不是在现有值上乘，否则连续两帧就把字号乘了两次。
pub fn apply_font_scale(ctx: &egui::Context, scale: f32) {
    let scale = scale.clamp(MIN_FONT_SCALE, MAX_FONT_SCALE);
    let base = egui::style::default_text_styles();
    ctx.all_styles_mut(move |style| {
        style.text_styles = base
            .iter()
            .map(|(name, font)| {
                (
                    name.clone(),
                    FontId::new(font.size * scale, font.family.clone()),
                )
            })
            .collect();
    });
}

/// 边框线的颜色：浅色近黑、深色淡灰（用户 2026-10-28："浅色模式下应该有黑色边框，
/// 深色模式下应该有淡灰色边框"）。
///
/// 控件三态与整条工具条的分隔线共用这一处出处——分隔线另配一套色，就会出现"按钮有框、
/// 栏没有框"那种半改完的状态（同一批走查里他们连着提了这两条）。
pub const fn edge_color(dark: bool) -> Color32 {
    if dark {
        Color32::from_gray(120) // 淡灰边框
    } else {
        Color32::from_rgb(28, 32, 38) // 近黑边框，跟着冷灰白基底偏一点蓝
    }
}

/// 控件的边框与悬停反馈（用户 2026-10-28："浅色模式下应该有黑色边框，深色模式下应该有
/// 淡灰色边框；浅色鼠标放上去下陷整体变深颜色，深色鼠标放上去颜色变浅"）。
///
/// 根因：egui 亮色主题下 `widgets.inactive.bg_stroke` 就是 `Stroke::NONE`
/// （`style.rs:1846` 那套 `Widgets::light()`），按钮只有一层灰底、没有边框，
/// 悬停才浮现一条 gray(105)。这里把三态各配一条实线，并把悬停/按下做成"内缩 + 变深"：
/// `expansion` 取负是**纯绘制层**的内缩，不参与布局，看起来就是按下去了一点。
///
/// 基准一律取 egui 自带的那套 `Widgets::dark()`，不是当前样式里的值——`apply` 每帧都跑，
/// 在现有颜色上乘就等于每帧加深一次（跟上面字号缩放同一个坑）。浅色那一支不乘基准：
/// 基底已经是冷灰白，悬停/按下就得在同一色系里往深走，[`COOL_HOVER`] 与 [`COOL_PRESSED`]
/// 是从 [`COOL_RAISED`] 往下两档；乘出来的纯灰放冷色旁边更难看。
pub fn apply_widget_contrast(style: &mut egui::Style) {
    let dark = style.visuals.dark_mode;
    let (hover_fill, pressed_fill) = if dark {
        let base = egui::style::Widgets::dark().inactive.weak_bg_fill;
        (scale_rgb(base, 1.55), scale_rgb(base, 1.9))
    } else {
        (COOL_HOVER, COOL_PRESSED)
    };
    let edge = edge_color(dark);
    let widgets = &mut style.visuals.widgets;
    widgets.inactive.bg_stroke = Stroke::new(1.0, edge);
    widgets.hovered.bg_stroke = Stroke::new(1.0, press_edge(dark, 190));
    widgets.hovered.weak_bg_fill = hover_fill;
    widgets.hovered.bg_fill = hover_fill;
    widgets.hovered.expansion = -1.0;
    widgets.active.bg_stroke = Stroke::new(1.0, press_edge(dark, 220));
    widgets.active.weak_bg_fill = pressed_fill;
    widgets.active.bg_fill = pressed_fill;
    widgets.active.expansion = -2.0;
    widgets.open.bg_stroke = Stroke::new(1.0, edge);
}

/// 悬停与按下那两条描边：深色往上提亮一档（190 → 220 就是"越按越亮"那两个字的方向），
/// 浅色一律压黑——冷灰白底上只有黑线才读得出"这里有个框"。
const fn press_edge(dark: bool, lightened: u8) -> Color32 {
    if dark {
        Color32::from_gray(lightened)
    } else {
        Color32::BLACK
    }
}

/// 冷灰白基底：把浅色那一套的**底与线**换成 [`COOL_*`] 那几档。
///
/// 深色一套原样不动（`dark_mode` 一判就退出）。每一处都是**赋值**，不是 in-place 调亮调暗——
/// `apply` 每帧都跑，任何"在当前值上再动一下"都会逐帧漂移（字号、行距、控件色三个坑同一个）。
///
/// 这一处只管得到 **egui 画的那部分**（内容区与自绘的工具条 `views::window_bar`）。窗口顶上那
/// 32pt 标题栏归 AppKit 的 `NSThemeFrame` 画，改 `Visuals` 碰不到它——2026-11-04 那句"标题栏的
/// 颜色和界面不统一，有色差"就差在这里。整窗那一级由 [`window_base`] + `system::background` 补齐。
pub fn apply_cool_light(style: &mut egui::Style) {
    if style.visuals.dark_mode {
        return;
    }
    let visuals = &mut style.visuals;
    visuals.window_fill = COOL_BASE;
    visuals.panel_fill = COOL_BASE;
    visuals.extreme_bg_color = COOL_SURFACE;
    visuals.faint_bg_color = COOL_RAISED;
    visuals.window_stroke = Stroke::new(1.0, COOL_EDGE);
    let widgets = &mut visuals.widgets;
    widgets.noninteractive.bg_fill = COOL_BASE;
    widgets.noninteractive.weak_bg_fill = COOL_BASE;
    widgets.noninteractive.bg_stroke = Stroke::new(1.0, COOL_EDGE);
    widgets.inactive.bg_fill = COOL_RAISED;
    widgets.inactive.weak_bg_fill = COOL_RAISED;
    widgets.open.bg_fill = COOL_RAISED;
    widgets.open.weak_bg_fill = COOL_RAISED;
}

/// 每帧同步一次外观设置（改动即时生效，无需"应用"按钮）
pub fn apply(ctx: &egui::Context, ui: &UiConfig) {
    ctx.set_theme(preference(ui.theme));
    apply_font_scale(ctx, ui.font_scale);
    let line_spacing = ui.line_spacing;
    ctx.all_styles_mut(move |style| {
        // 先定基底再定控件三态：`set_theme` 每帧把样式换回 egui 自带那套，冷色必须跟着重打一遍
        apply_cool_light(style);
        apply_widget_contrast(style);
        // 赋值不是累加：`apply` 每帧都跑，`+=` 会每帧加一遍（和字号那个坑同一个）。
        // 这一项只被 TextEdit 那条路读（`text_edit/builder.rs:563`），`Label` 不认，
        // 所以译文那一侧由 [`PaneText`] 补挂同样的行高。
        style.spacing.extra_text_line_spacing = line_spacing;
    });
}

/// 一栏正文（源文 / 译文）的排版参数：字号倍率、字间距、行间距。
///
/// 三个数分别落在三个不同的地方，这是 egui 0.37 的实情，不是这里绕圈子：
/// 字号走 `FontId`；行间距在 `Spacing::extra_text_line_spacing`（只有 TextEdit 自动吃），
/// 对 `Label` 必须换成每个 section 上的绝对 `format.line_height`；字间距则两边都要手工挂
/// `format.extra_letter_spacing`——`Spacing` 里根本没有这一项。
///
/// 抽成一个类型是为了让**源文框和译文框长得一样**：两处各算各的，改一个忘另一个，
/// 就会出现"同样的行距，一个框松一个框挤"（用户 2026-10-28 那句"字间距和行间距太小了，
/// 看起来有点乱"，两栏不一致会比一致更乱）。
pub struct PaneText {
    pub font: FontId,
    pub letter_spacing: f32,
    pub line_height: f32,
}

impl PaneText {
    /// `pane_scale` 是这一栏自己的倍率，乘在**已经乘过界面 `font_scale`** 的 Body 字号上。
    ///
    /// 取 `ui.style()` 里当前的 Body，而不是取 `default_text_styles()` 的基准：界面缩放那一步
    /// 每帧已经做完了，这里再乘基准就等于把 `font_scale` 丢掉。
    pub fn new(ui: &Ui, config: &UiConfig, pane_scale: f32) -> Self {
        let base = &ui.style().text_styles[&egui::TextStyle::Body];
        let font = FontId::new(base.size * pane_scale, base.family.clone());
        let line_height = {
            let extra = ui.spacing().extra_text_line_spacing;
            ui.fonts_mut(|fonts| fonts.row_height(&font)) + extra
        };
        Self {
            font,
            letter_spacing: config.letter_spacing,
            line_height,
        }
    }

    /// 译文 / 错误提示这类 `Label` 正文。颜色留给调用方决定（错误红、普通正文各不同）。
    pub fn rich(&self, text: &str) -> RichText {
        RichText::new(text)
            .font(self.font.clone())
            .extra_letter_spacing(self.letter_spacing)
            .line_height(Some(self.line_height))
    }

    /// 源文 `TextEdit` 的自定义排版器：在 egui 默认那份之上多挂字间距。
    ///
    /// 默认那份（`text_edit/builder.rs:576-588`）的每个字段都得照抄，少一个就是新 bug：
    /// `keep_trailing_whitespace` 掉了打字时光标会跳，`halign` 掉了居中栏会左对齐，
    /// `line_height` 用绝对值，漏挂就等于把行间距设置作废。
    /// 颜色故意留 `PLACEHOLDER`——TextEdit 绘制时会替换成主题的正文色（`painter.rs:525`），
    /// 写死色值会让深色模式下第一行译文变黑底黑字。
    pub fn layout(&self, ui: &Ui, text: &str, wrap_width: f32) -> Arc<Galley> {
        let mut job = LayoutJob::simple(
            text.to_owned(),
            self.font.clone(),
            Color32::PLACEHOLDER,
            wrap_width,
        );
        job.halign = ui.layout().align2().x();
        job.keep_trailing_whitespace = true;
        for section in &mut job.sections {
            section.format.line_height = Some(self.line_height);
            section.format.extra_letter_spacing = self.letter_spacing;
        }
        ui.fonts_mut(|fonts| fonts.layout_job(job))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `all_styles_mut` 亮暗两套都改，断言任一套即可；`Theme` 要写全，避免与 config 的同名类型混淆
    fn style(ctx: &egui::Context) -> std::sync::Arc<egui::Style> {
        ctx.style_of(egui::Theme::Light)
    }

    fn body_size(ctx: &egui::Context) -> f32 {
        style(ctx).text_styles[&egui::TextStyle::Body].size
    }

    fn heading_size(ctx: &egui::Context) -> f32 {
        style(ctx).text_styles[&egui::TextStyle::Heading].size
    }

    #[test]
    fn theme_maps_to_the_three_preferences() {
        assert_eq!(preference(Theme::System), ThemePreference::System);
        assert_eq!(preference(Theme::Light), ThemePreference::Light);
        assert_eq!(preference(Theme::Dark), ThemePreference::Dark);
    }

    #[test]
    fn font_scale_multiplies_the_egui_defaults() {
        let ctx = egui::Context::default();
        apply_font_scale(&ctx, 1.5);
        let body = body_size(&ctx);
        let heading = heading_size(&ctx);
        assert_eq!(body, 13.0 * 1.5);
        assert_eq!(heading, 18.0 * 1.5);
    }

    #[test]
    fn font_scale_is_idempotent_across_frames() {
        let ctx = egui::Context::default();
        for _ in 0..3 {
            apply_font_scale(&ctx, 2.0);
        }
        assert_eq!(body_size(&ctx), 26.0);
    }

    #[test]
    fn font_scale_is_clamped_to_the_legal_range() {
        let ctx = egui::Context::default();
        apply_font_scale(&ctx, 0.1);
        assert_eq!(body_size(&ctx), 13.0 * 0.8);
        apply_font_scale(&ctx, 9.0);
        assert_eq!(body_size(&ctx), 13.0 * 2.0);
    }

    #[test]
    fn monospace_keeps_its_font_family() {
        let ctx = egui::Context::default();
        apply_font_scale(&ctx, 1.2);
        let family = style(&ctx).text_styles[&egui::TextStyle::Monospace]
            .family
            .clone();
        assert_eq!(family, egui::FontFamily::Monospace);
    }

    #[test]
    fn every_tone_has_its_own_color() {
        let colors = [
            tone_color(Tone::Info),
            tone_color(Tone::Warn),
            tone_color(Tone::Error),
        ];
        assert_ne!(colors[0], colors[1]);
        assert_ne!(colors[1], colors[2]);
        assert_ne!(colors[0], colors[2]);
    }

    #[test]
    fn apply_sets_theme_and_scale_together() {
        let ctx = egui::Context::default();
        let ui = UiConfig {
            theme: Theme::Dark,
            font_scale: 1.25,
            ..Default::default()
        };
        apply(&ctx, &ui);
        assert_eq!(body_size(&ctx), 13.0 * 1.25);
    }

    /// `Theme::System` 不是一种具体的样子，两套都要成立，所以直接拿 egui 的两套 Visuals
    fn visuals(theme: Theme) -> egui::Visuals {
        match theme {
            Theme::Dark => egui::Visuals::dark(),
            _ => egui::Visuals::light(),
        }
    }

    /// 正文框的底色只能取 egui 自己那个"可书写表面"的令牌，不许在这里新调一个颜色。
    ///
    /// 用户 2026-10-28 提的"这两个框怎么和软件其他的框颜色一样"要在**配色**上动，那是主题
    /// 那一包的事（还没到）；这里只允许"框和画布用的令牌不一样"这一条。
    #[test]
    fn surface_fill_is_eguis_own_input_token() {
        for theme in [Theme::Light, Theme::Dark] {
            let v = visuals(theme);
            let surface = surface_fill(&v);
            assert_eq!(surface, v.text_edit_bg_color());
            assert_eq!(surface.a(), 255, "正文框底色不许半透明：{surface:?}");
            assert_ne!(
                surface, v.panel_fill,
                "{theme:?} 下正文框和画布同色（用户 2026-10-28 的那句抱怨）"
            );
        }
    }

    /// 三档渐变：两个端点之外还要有一个中间色，且全程不许跑出三档锚点围出的区间。
    #[test]
    fn title_ramp_walks_three_stops() {
        assert_eq!(title_color(0.0), ACCENT);
        assert_eq!(title_color(0.5), TITLE_MID);
        assert_eq!(title_color(1.0), TITLE_END);
        let anchors = [ACCENT, TITLE_MID, TITLE_END];
        let at = |c: Color32, channel: u8| match channel {
            0 => c.r(),
            1 => c.g(),
            _ => c.b(),
        };
        for step in 0..=20 {
            let t = step as f32 / 20.0;
            for channel in 0..3 {
                let lo = anchors.iter().map(|c| at(*c, channel)).min().unwrap();
                let hi = anchors.iter().map(|c| at(*c, channel)).max().unwrap();
                let v = at(title_color(t), channel);
                assert!(
                    lo <= v && v <= hi,
                    "t={t:.2} 第 {channel} 个分量跑出锚点区间 {lo}..{hi}：{v}"
                );
            }
        }
    }

    /// 一行两个字也要**连续**地变色——这就是"逐字换色"和"渐变"的分别。
    ///
    /// 用户 2026-10-28："你搞两个不同颜色的字放一起就叫艺术字了？"按色带档数取样，
    /// 色阶必须明显多于字数、相邻档也真的在变，否则画出来就是两块拼色。
    #[test]
    fn the_title_ramp_has_many_more_steps_than_characters() {
        let bands = title_bands(2.0 * 13.0); // "源文"两个字、字号 13
        assert!(bands > 6, "两个字只切成 {bands} 档，太少");
        let colors: Vec<Color32> = (0..bands)
            .map(|index| title_color(index as f32 / (bands - 1) as f32))
            .collect();
        let changed = colors.windows(2).filter(|pair| pair[0] != pair[1]).count();
        assert!(
            changed >= bands - 2,
            "{bands} 档里只有 {changed} 档真的变色，看上去是拼色不是渐变：{colors:?}"
        );
    }

    #[test]
    fn title_bands_are_clamped() {
        assert_eq!(title_bands(0.0), 8, "再窄也切到 8 档，否则台阶看得见");
        assert_eq!(title_bands(600.0), 32, "一行标题不许切成几十遍重画");
        assert_eq!(
            title_bands(30.0),
            (30.0 / TITLE_BAND_PX).ceil() as usize,
            "越过下限之后每 {}px 一档",
            TITLE_BAND_PX
        );
    }

    #[test]
    fn mix_interpolates_each_channel_and_clamps_t() {
        let from = Color32::from_rgb(0, 100, 200);
        let to = Color32::from_rgb(100, 200, 0);
        assert_eq!(mix(from, to, 0.0), from);
        assert_eq!(mix(from, to, 1.0), to);
        assert_eq!(mix(from, to, 0.5), Color32::from_rgb(50, 150, 100));
        assert_eq!(
            mix(from, to, -3.0),
            from,
            "t 越界要夹住，不许算出区间外的颜色"
        );
        assert_eq!(mix(from, to, 4.0), to);
    }

    fn style_for(dark: bool) -> egui::Style {
        egui::Style {
            visuals: if dark {
                egui::Visuals::dark()
            } else {
                egui::Visuals::light()
            },
            ..Default::default()
        }
    }

    /// 亮色近黑边框、暗色淡灰边框（用户 2026-10-28："浅色模式下应该有黑色边框，
    /// 深色模式下应该有淡灰色边框"）。egui 亮色主题的 `inactive.bg_stroke` 是 NONE，
    /// 这条断言在改之前根本过不了。
    #[test]
    fn widgets_get_a_visible_border_in_both_themes() {
        for dark in [false, true] {
            let mut style = style_for(dark);
            apply_widget_contrast(&mut style);
            let stroke = style.visuals.widgets.inactive.bg_stroke;
            assert!(stroke.width >= 1.0, "按钮边框不该是 0 宽：{stroke:?}");
            assert_ne!(stroke.color, Color32::TRANSPARENT, "按钮边框不该没有颜色");
            assert_eq!(
                stroke.color.r() < 100,
                !dark,
                "{dark} 主题的边框该往哪边靠：亮色近黑、暗色淡灰，实际 {stroke:?}"
            );
        }
    }

    #[test]
    fn hover_sinks_and_darkens_in_light_and_lightens_in_dark() {
        let light_stock = egui::style::Widgets::light().inactive.weak_bg_fill;
        let mut light = style_for(false);
        apply_widget_contrast(&mut light);
        let hovered = light.visuals.widgets.hovered.weak_bg_fill;
        assert!(
            hovered.r() < light_stock.r(),
            "亮色悬停要整体变深：{light_stock:?} → {hovered:?}"
        );
        assert!(
            light.visuals.widgets.hovered.expansion < 0.0,
            "悬停要内缩，看着才像按下去"
        );

        let dark_stock = egui::style::Widgets::dark().inactive.weak_bg_fill;
        let mut dark = style_for(true);
        apply_widget_contrast(&mut dark);
        assert!(
            dark.visuals.widgets.hovered.weak_bg_fill.r() > dark_stock.r(),
            "暗色悬停要变浅"
        );
    }

    /// `apply` 每帧都跑：基准必须是 egui 自带那套而不是当前样式里的值，
    /// 否则一帧深一点，跑一会儿就把按钮熬成黑洞（跟字号缩放同一个坑）。
    #[test]
    fn widget_contrast_is_idempotent_across_frames() {
        let mut style = style_for(false);
        apply_widget_contrast(&mut style);
        let once = (
            style.visuals.widgets.hovered.weak_bg_fill,
            style.visuals.widgets.inactive.bg_stroke.color,
        );
        for _ in 0..5 {
            apply_widget_contrast(&mut style);
        }
        assert_eq!(
            (
                style.visuals.widgets.hovered.weak_bg_fill,
                style.visuals.widgets.inactive.bg_stroke.color
            ),
            once,
            "每帧重跑把控件颜色越改越深了"
        );
    }

    /// 用户 2026-11-04："界面基底(包括标题栏底色)能用冷灰白色最好，不要使用暖色，看起来好过时。"
    /// 这条量三件事：浅色那一套的**底、线、控件三态**都落在 `COOL_*` 上；深色一套一个字不动
    /// （本轮只改浅色，深浅色共用的其余配色仍属主题包 #28）；`apply` 每帧重跑不许把底色越跑越深。
    ///
    /// 读的是 `style_of(Theme::Light)` / `style_of(Theme::Dark)` 而不是"当前生效那套"：
    /// egui 0.37 把两套样式**并存**在 `Options::{light,dark}_style` 里，`set_theme` 只改
    /// `theme_preference`（选哪套），而 `all_styles_mut` 同时改两套。所以"浅色被改了、深色没被
    /// 顺手改"这件事必须分两套各读各的才量得出来。
    #[test]
    fn the_cool_base_replaces_eguis_greys_only_in_light_mode() {
        let applied = |theme: Theme| {
            let ctx = egui::Context::default();
            let ui = UiConfig {
                theme,
                ..Default::default()
            };
            for _ in 0..3 {
                apply(&ctx, &ui);
            }
            ctx
        };

        let ctx = applied(Theme::Light);
        let light = ctx.style_of(egui::Theme::Light).visuals.clone();
        assert_eq!(light.window_fill, COOL_BASE);
        assert_eq!(
            light.panel_fill, COOL_BASE,
            "自绘工具条与内容区必须同一个底，否则界面自己先分成两块"
        );
        assert_eq!(light.extreme_bg_color, COOL_SURFACE);
        assert_eq!(light.widgets.inactive.bg_fill, COOL_RAISED);
        assert_eq!(light.widgets.hovered.weak_bg_fill, COOL_HOVER);
        assert_eq!(light.widgets.active.weak_bg_fill, COOL_PRESSED);
        assert_eq!(light.widgets.noninteractive.bg_stroke.color, COOL_EDGE);
        assert_eq!(light.widgets.inactive.bg_stroke.color, edge_color(false));
        // "冷"是个可量的方向：底与线每一档的蓝分量都要压过红分量
        for swatch in [
            COOL_BASE,
            COOL_SURFACE,
            COOL_RAISED,
            COOL_HOVER,
            COOL_PRESSED,
            COOL_EDGE,
        ] {
            assert!(swatch.b() > swatch.r(), "{swatch:?} 不冷");
        }
        // 三帧之后还是同一组值：`apply` 每帧都跑，任何"在当前值上再动一下"都会逐帧漂移
        let again = applied(Theme::Light);
        assert_eq!(
            again.style_of(egui::Theme::Light).visuals.window_fill,
            light.window_fill
        );

        let dark = ctx.style_of(egui::Theme::Dark).visuals.clone();
        let stock = egui::Visuals::dark();
        assert_eq!(dark.window_fill, stock.window_fill, "深色那一套被顺手改了");
        assert_eq!(dark.panel_fill, stock.panel_fill);
        assert_eq!(dark.extreme_bg_color, stock.extreme_bg_color);
        // 深色那一支的悬停/按下仍走 2026-10-28 那条"变浅一档"的乘算，不许被冷灰白那两档顶掉
        assert_ne!(dark.widgets.hovered.weak_bg_fill, COOL_HOVER);
        assert_ne!(dark.widgets.active.weak_bg_fill, COOL_PRESSED);
    }

    /// 用户 2026-11-04 定的分层规矩：**基底一个色铺满整窗，正文框才是第二级**。
    /// 这条锁的是"两处各自持色"——上一轮只把冷色写进 egui，顶上那 32pt 归 AppKit 画，
    /// 于是同一个主题长出两块底色（"标题栏的颜色和界面不统一，有色差"）。
    ///
    /// 三个判据：① `window_base` 与当帧样式里的 `window_fill`/`panel_fill` 逐字相等（深浅两套
    /// 各判一次，原生那一侧取的就是这个数）；② 正文框色与基底**不相等**（两级要看得出分别是两级）；
    /// ③ 基底与正文面各只有一个出处，不是随手两个数。
    #[test]
    fn the_whole_window_has_one_base_color() {
        for dark in [false, true] {
            let ctx = egui::Context::default();
            let ui = UiConfig {
                theme: if dark { Theme::Dark } else { Theme::Light },
                ..Default::default()
            };
            apply(&ctx, &ui);
            let visuals = ctx
                .style_of(if dark {
                    egui::Theme::Dark
                } else {
                    egui::Theme::Light
                })
                .visuals
                .clone();
            let base = window_base(dark);
            assert_eq!(
                visuals.window_fill, base,
                "{dark}：内容区的底不是整窗那个底"
            );
            assert_eq!(
                visuals.panel_fill, base,
                "{dark}：面板带的底自己另取了一个色"
            );
            assert_ne!(
                visuals.extreme_bg_color, base,
                "{dark}：输入框/译文框和整窗同色，两级就塌成一级"
            );
        }
    }

    /// 用户 2026-11-04："淡淡的阴影就行。" 这条锁四个方向：有模糊（不是硬边）、只往下沉、
    /// 浅到不会让两个框各自浮成一块黑板、`spread` 恒 0（非 0 就是向外涨出一圈实色描边）。
    /// 「正文框」的底色与阴影必须**一起**给（2026-11-05 用户复报"精简界面…现在倒好整个都是
    /// 灰呼呼的，和全量界面的文本框区别太大了"）。病根是 `Frame::group` 的 `fill` 是透明的：
    /// 只挂阴影不铺底，软阴影会从盒子底下透上来糊在格子里。所以这条锁的是"缺一层绘制"这件事，
    /// 不是某个色值——三处调用点（全量两个框、精简两条带）共用这个函数。
    #[test]
    fn a_pane_frame_carries_its_surface_color_together_with_its_shadow() {
        for dark in [false, true] {
            let style = style_for(dark);
            let frame = pane_frame(&style);
            assert_eq!(
                frame.fill,
                surface_fill(&style.visuals),
                "底色没铺上，阴影就会从盒底透上来"
            );
            assert_ne!(
                frame.fill,
                Color32::TRANSPARENT,
                "`Frame::group` 自己是透明的——这一层不铺就等于什么都没画"
            );
            assert_ne!(
                frame.fill, style.visuals.panel_fill,
                "底色与整窗基底同色的话，框就只剩一圈阴影，正是用户报的那个样子"
            );
            assert_eq!(
                frame.shadow,
                pane_shadow(dark),
                "阴影不与底色同处定义，两处就会各自漂走"
            );
        }
    }

    #[test]
    fn the_pane_shadow_is_soft_and_stays_faint() {
        let light = pane_shadow(false);
        assert_eq!(light.offset, [0, 2], "只许往下沉，横向偏移会读成框自己歪了");
        assert!(
            light.blur >= 4,
            "blur {} 以下为硬边，不是软阴影",
            light.blur
        );
        assert_eq!(light.spread, 0, "spread 非 0 会向外涨成一圈实色");
        assert!(
            light.color.a() <= 40,
            "alpha {} 已经不是'淡'，两个框会各自浮起来一块黑板",
            light.color.a()
        );
        let dark = pane_shadow(true);
        assert!(
            dark.color.a() > light.color.a(),
            "深色底上同样浅的 alpha 根本读不出来"
        );
        assert_eq!(dark.offset, light.offset, "两个形态的投影方向不许分叉");
    }

    /// 行间距也是每帧同步的一项：`apply` 里必须是赋值。写成 `+=` 的话跑三帧就变成三倍，
    /// 悬浮窗那两条带会被顶穿（用户 2026-10-28 之后这条设置真的进了排版）。
    #[test]
    fn line_spacing_is_applied_once_per_frame() {
        let ctx = egui::Context::default();
        let config = UiConfig {
            line_spacing: 5.0,
            ..Default::default()
        };
        for _ in 0..3 {
            apply(&ctx, &config);
        }
        assert_eq!(style(&ctx).spacing.extra_text_line_spacing, 5.0);
    }

    /// 在跑起来的帧里造一份 `PaneText`：它的三个数分别取自 style、spacing 和字体度量，
    /// 手搓一个 `Style` 就测不到"每帧同步之后正文到底读到什么"。
    fn pane_text(config: &UiConfig, pane_scale: f32) -> PaneText {
        let ctx = egui::Context::default();
        apply(&ctx, config);
        let mut made = None;
        ctx.run_ui(egui::RawInput::default(), |ui| {
            made = Some(PaneText::new(ui, config, pane_scale));
        })
        .drop_without_applying_deltas();
        made.expect("那一帧没跑出 PaneText")
    }

    /// 正文倍率乘在**已经乘过界面字号**的 Body 上：两档是相乘关系，不是二选一。
    /// 用户要的是"原文和译文字体大小也需要能调整"，而整界面缩放那档（FR-2.3）不能因此失效。
    #[test]
    fn pane_font_multiplies_the_ui_scale_rather_than_replacing_it() {
        let config = UiConfig {
            font_scale: 1.6,
            ..Default::default()
        };
        let pane = pane_text(&config, 1.5);
        assert!(
            (pane.font.size - 13.0 * 1.6 * 1.5).abs() < 0.01,
            "正文应当是界面字号 × 分栏倍率，实际 {}",
            pane.font.size
        );
        assert_eq!(pane.letter_spacing, config.letter_spacing);
    }

    /// 行间距真的进到行高里，而且加的是"这一栏字号"的行高之上
    #[test]
    fn pane_line_height_follows_spacing_and_font_size() {
        let tight = UiConfig {
            line_spacing: 0.0,
            ..Default::default()
        };
        let loose = UiConfig {
            line_spacing: 10.0,
            ..Default::default()
        };
        let base = pane_text(&tight, 1.0).line_height;
        let with_gap = pane_text(&loose, 1.0).line_height;
        assert!(
            (with_gap - base - 10.0).abs() < 0.01,
            "行间距应当正好多出 10px，实际 {base} → {with_gap}"
        );
        assert!(
            base > 13.0,
            "行高比字号还小，说明取的不是字体度量而是字号本身：{base}"
        );
        let big = pane_text(&tight, 2.0).line_height;
        assert!(big > base, "正文放大两倍行高却没变：{base} → {big}");
    }

    /// 字间距必须进到排版结果里。`Spacing` 没有这一项，只能靠 `format.extra_letter_spacing`，
    /// 挂错了 egui 也不报错——所以这里量同一句话排出来的宽度，非零字间距必须更宽。
    #[test]
    fn letter_spacing_reaches_the_laid_out_line() {
        let width_at = |spacing: f32| {
            let config = UiConfig {
                letter_spacing: spacing,
                ..Default::default()
            };
            let ctx = egui::Context::default();
            apply(&ctx, &config);
            let mut width = None;
            ctx.run_ui(egui::RawInput::default(), |ui| {
                let pane = PaneText::new(ui, &config, 1.0);
                // 不换行地排一句中文，宽度就是所有字的推进量之和
                width = Some(pane.layout(ui, "中文翻译测试", f32::INFINITY).size().x);
            })
            .drop_without_applying_deltas();
            width.expect("那一帧没排出文字")
        };
        let tight = width_at(0.0);
        let loose = width_at(3.0);
        assert!(tight > 0.0, "没排版出任何东西");
        assert!(
            loose > tight,
            "字间距 3px 之后一句六个字还是一样宽：{tight} → {loose}"
        );
        // 6 个字之间 5 个缝隙，每个 3px：多出来的宽度应当接近 15px
        let extra = loose - tight;
        assert!(
            extra > 10.0 && extra < 20.0,
            "多出的宽度不像按缝隙算的：{extra}"
        );
    }
}
