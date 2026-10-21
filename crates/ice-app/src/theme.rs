//! 外观（FR-2.4 主题三态、FR-2.3 字号缩放、`03-ui-design.md` §1 的配色）。
//!
//! 颜色只在这里定义，各 view 不自带色值，免得改一次主题要翻四个文件。

use egui::{Color32, FontId, ThemePreference};
use ice_core::config::{Theme, UiConfig};

use crate::state::Tone;

/// 界面缩放的合法区间（与 `UiConfig::font_scale` 的约定一致）
pub const MIN_FONT_SCALE: f32 = 0.8;
pub const MAX_FONT_SCALE: f32 = 2.0;

/// 主色：按钮、选中态、链接
pub const ACCENT: Color32 = Color32::from_rgb(74, 132, 214);
/// 错误红：翻译失败、钥匙串不可用
pub const ERROR: Color32 = Color32::from_rgb(214, 69, 65);
/// 降级黄：配置损坏启动、AI 超时改走机翻、AI 未配置
pub const WARN: Color32 = Color32::from_rgb(206, 145, 46);
/// 成功绿：复制完成、Key 保存成功
pub const OK: Color32 = Color32::from_rgb(76, 154, 96);
/// 次要文字：字数统计、耗时、缓存标记
pub const MUTED: Color32 = Color32::from_gray(128);

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

/// 每帧同步一次外观设置（改动即时生效，无需"应用"按钮）
pub fn apply(ctx: &egui::Context, ui: &UiConfig) {
    ctx.set_theme(preference(ui.theme));
    apply_font_scale(ctx, ui.font_scale);
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
}
