//! 图片页：P4 只画 `03-ui-design.md` §2「图片页」的静态版式（E/F/G/H/I 五区），
//! 实际能力在 P6/P7 接入 `ice-vision`。
//!
//! 这里刻意不放任何可点控件（REQ-001）：走查时点到没反应的按钮只会浪费验收时间，
//! 而且"未实现"的按钮本身也是一种误导——它承诺了当前版本并不存在的能力。

use egui::{self, Align, Layout, RichText};

use crate::state::AppState;
use crate::theme;

pub fn show(ui: &mut egui::Ui, state: &AppState) {
    ui.spacing_mut().item_spacing.y = 8.0;
    notice(ui);
    toolbar_frame(ui);
    egui::ScrollArea::vertical()
        .auto_shrink([false, false])
        .show(ui, |ui| {
            preview_frames(ui);
            result_frame(ui);
            progress_frame(ui);
        });
    // `state` 在 P4 的图片页上没有可改的东西，但签名要与其它三页一致（REQ-001），
    // 这里只读一个值，免得将来接线时忘了它本来就在这里。
    ui.colored_label(
        theme::MUTED,
        format!(
            "当前语言对：{} → {}",
            source_label(state),
            state.config.translation.target.display_name()
        ),
    );
}

fn source_label(state: &AppState) -> &str {
    state
        .config
        .translation
        .source
        .map(|lang| lang.display_name())
        .unwrap_or("自动检测")
}

fn notice(ui: &mut egui::Ui) {
    egui::Frame::group(ui.style())
        .fill(theme::ACCENT.gamma_multiply(0.12))
        .inner_margin(10.0)
        .show(ui, |ui| {
            ui.label(RichText::new("图片翻译尚未启用").strong());
            ui.label(
                "识别与译文回填将在后续版本接入 macOS Vision：全程本地，图片不会上传到任何服务。",
            );
        });
}

/// E 区：图片工具条的位置与内容（版式先行，控件在 P6 补）
fn toolbar_frame(ui: &mut egui::Ui) {
    region(ui, 56.0, |ui| {
        ui.horizontal(|ui| {
            for label in ["打开图片", "粘贴", "拖拽区"] {
                placeholder_chip(ui, label);
            }
            ui.separator();
            ui.colored_label(theme::MUTED, "模式：回填 / 覆盖层");
        });
    });
}

/// F/G 区：左原图、右结果，同尺寸对齐
fn preview_frames(ui: &mut egui::Ui) {
    ui.columns(2, |columns| {
        region(&mut columns[0], 180.0, |ui| {
            caption(ui, "F 原图预览");
            ui.colored_label(theme::MUTED, "叠加识别框，可点击选中");
        });
        region(&mut columns[1], 180.0, |ui| {
            caption(ui, "G 结果预览");
            ui.colored_label(theme::MUTED, "降级块用橙色描边标记");
        });
    });
}

/// H 区：识别结果列表（可编辑），P4 只给表头与两行样例占位
fn result_frame(ui: &mut egui::Ui) {
    region(ui, 96.0, |ui| {
        caption(ui, "H 识别结果");
        ui.horizontal(|ui| {
            ui.colored_label(theme::MUTED, "#");
            ui.colored_label(theme::MUTED, "置信度");
            ui.colored_label(theme::MUTED, "原文");
            ui.colored_label(theme::MUTED, "译文");
        });
        ui.colored_label(theme::MUTED, "（暂无内容）");
    });
}

/// I 区：进度与取消
fn progress_frame(ui: &mut egui::Ui) {
    region(ui, 36.0, |ui| {
        ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
            caption(ui, "I 进度");
            ui.colored_label(theme::MUTED, "空闲");
        });
    });
}

fn region(ui: &mut egui::Ui, height: f32, body: impl FnOnce(&mut egui::Ui)) {
    egui::Frame::group(ui.style())
        .inner_margin(8.0)
        .show(ui, |ui| {
            ui.set_min_height(height);
            ui.set_width(ui.available_width());
            body(ui);
        });
}

fn caption(ui: &mut egui::Ui, text: &str) {
    ui.label(RichText::new(text).strong());
}

/// 看着像按钮但不是按钮：用 `Frame` 画外形，避免产生可点的热区
fn placeholder_chip(ui: &mut egui::Ui, text: &str) {
    egui::Frame::group(ui.style())
        .inner_margin(egui::Margin::symmetric(10, 5))
        .fill(theme::MUTED.gamma_multiply(0.10))
        .show(ui, |ui| {
            ui.colored_label(theme::MUTED, text);
        });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::KeyState;
    use ice_core::config::Config;
    use ice_core::glossary::Glossary;
    use ice_core::language::Language;

    fn state() -> AppState {
        AppState::new(
            Config::default(),
            Glossary::default(),
            KeyState::Missing,
            None,
        )
    }

    /// 无头跑一帧：占位页的正确性就是"不 panic、不改状态"，没有可断言的输出文本
    fn render(state: &AppState) {
        let ctx = egui::Context::default();
        let raw = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(1080.0, 720.0),
            )),
            ..Default::default()
        };
        let output = ctx.run_ui(raw, |ui| {
            show(ui, state);
        });
        output.drop_without_applying_deltas();
    }

    #[test]
    fn placeholder_page_renders_the_five_regions_without_panicking() {
        let state = state();
        render(&state);
    }

    #[test]
    fn placeholder_page_never_changes_state() {
        let mut state = state();
        state.source = "不该被动".into();
        render(&state);
        assert_eq!(state.tab, crate::state::Tab::Text);
        assert_eq!(state.source, "不该被动");
    }

    #[test]
    fn language_pair_is_reflected_in_the_footnote() {
        let mut state = state();
        state.config.translation.source = Some(Language::Ja);
        state.config.translation.target = Language::En;
        assert_eq!(source_label(&state), "日语");
    }

    #[test]
    fn auto_detection_reads_as_auto() {
        assert_eq!(source_label(&state()), "自动检测");
    }
}
