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
        Color32::from_gray(30) // 近黑边框
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
/// 基准一律取 egui 自带的那套 `Widgets::{light,dark}()`，不是当前样式里的值——`apply`
/// 每帧都跑，在现有颜色上乘就等于每帧加深一次（跟上面字号缩放同一个坑）。
pub fn apply_widget_contrast(style: &mut egui::Style) {
    let dark = style.visuals.dark_mode;
    let base = if dark {
        egui::style::Widgets::dark()
    } else {
        egui::style::Widgets::light()
    };
    let edge = edge_color(dark);
    let hover_fill = scale_rgb(base.inactive.weak_bg_fill, if dark { 1.55 } else { 0.8 });
    let pressed_fill = scale_rgb(base.inactive.weak_bg_fill, if dark { 1.9 } else { 0.62 });
    let widgets = &mut style.visuals.widgets;
    widgets.inactive.bg_stroke = Stroke::new(1.0, edge);
    widgets.hovered.bg_stroke = Stroke::new(
        1.0,
        if dark {
            Color32::from_gray(190)
        } else {
            Color32::BLACK
        },
    );
    widgets.hovered.weak_bg_fill = hover_fill;
    widgets.hovered.bg_fill = hover_fill;
    widgets.hovered.expansion = -1.0;
    widgets.active.bg_stroke = Stroke::new(
        1.0,
        if dark {
            Color32::from_gray(220)
        } else {
            Color32::BLACK
        },
    );
    widgets.active.weak_bg_fill = pressed_fill;
    widgets.active.bg_fill = pressed_fill;
    widgets.active.expansion = -2.0;
    widgets.open.bg_stroke = Stroke::new(1.0, edge);
}

/// 每帧同步一次外观设置（改动即时生效，无需"应用"按钮）
pub fn apply(ctx: &egui::Context, ui: &UiConfig) {
    ctx.set_theme(preference(ui.theme));
    apply_font_scale(ctx, ui.font_scale);
    let line_spacing = ui.line_spacing;
    ctx.all_styles_mut(move |style| {
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
