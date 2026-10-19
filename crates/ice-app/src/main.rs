//! Ice Translator 客户端入口。
//!
//! P1 骨架：验证"同一窗口在全量/简洁两种形态间切换 + 置顶 + 无边框"这条路走得通（ADR-001）。

use eframe::egui::{self, ViewportBuilder, ViewportCommand, WindowLevel};
use ice_core::config::{Config, UiMode};

struct IceApp {
    config: Config,
    mode: UiMode,
    always_on_top: bool,
    warning: Option<String>,
    /// ViewportCommand 异步生效，几何要在下一帧才读得到真实值
    report_geometry: bool,
}

impl IceApp {
    fn new(config: Config, warning: Option<String>) -> Self {
        let mode = config.ui.mode;
        let always_on_top = config.ui.always_on_top;
        Self {
            config,
            mode,
            always_on_top,
            warning,
            report_geometry: true,
        }
    }

    fn mark_geometry_dirty(&mut self, ctx: &egui::Context) {
        self.report_geometry = true;
        ctx.request_repaint();
    }

    fn apply_mode(&mut self, ctx: &egui::Context, target: UiMode) {
        self.mode = target;
        self.config.ui.mode = target;
        match target {
            UiMode::Full => {
                let rect = self.config.ui.full_rect;
                ctx.send_viewport_cmd(ViewportCommand::Decorations(true));
                ctx.send_viewport_cmd(ViewportCommand::InnerSize(egui::vec2(rect.w, rect.h)));
                ctx.send_viewport_cmd(ViewportCommand::Resizable(true));
            }
            UiMode::Compact => {
                let rect = self.config.ui.compact_rect;
                ctx.send_viewport_cmd(ViewportCommand::Decorations(false));
                ctx.send_viewport_cmd(ViewportCommand::Resizable(false));
                ctx.send_viewport_cmd(ViewportCommand::InnerSize(egui::vec2(rect.w, rect.h)));
                ctx.send_viewport_cmd(ViewportCommand::OuterPosition(egui::pos2(rect.x, rect.y)));
            }
        }
        self.apply_topmost(ctx);
    }

    fn apply_topmost(&mut self, ctx: &egui::Context) {
        self.config.ui.always_on_top = self.always_on_top;
        ctx.send_viewport_cmd(ViewportCommand::WindowLevel(if self.always_on_top {
            WindowLevel::AlwaysOnTop
        } else {
            WindowLevel::Normal
        }));
        self.mark_geometry_dirty(ctx);
    }
}

impl eframe::App for IceApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();

        if self.report_geometry {
            self.report_geometry = false;
            let (inner, outer, scale) = ctx.input(|i| {
                let vp = i.viewport();
                (
                    vp.inner_rect.map(|r| r.size()),
                    vp.outer_rect.map(|r| r.size()),
                    vp.native_pixels_per_point,
                )
            });
            tracing::info!(
                "窗口几何 mode={:?} 置顶={} inner={inner:?} outer={outer:?} pixels_per_point={scale:?}",
                self.mode,
                self.always_on_top,
            );
        }

        if let Some(warning) = &self.warning {
            egui::Panel::top("warning").show(ui, |ui| {
                ui.colored_label(egui::Color32::from_rgb(210, 140, 40), warning);
            });
        }

        egui::CentralPanel::default().show(ui, |ui| match self.mode {
            UiMode::Full => {
                ui.heading("Ice Translator");
                ui.label("工程骨架已就绪：领域层 / 引擎层 / 视觉层 / 界面层。");
                ui.separator();
                ui.horizontal(|ui| {
                    if ui.button("切换到简洁模式").clicked() {
                        self.apply_mode(&ctx, UiMode::Compact);
                    }
                    if ui
                        .button(if self.always_on_top {
                            "关闭置顶"
                        } else {
                            "开启置顶"
                        })
                        .clicked()
                    {
                        self.always_on_top = !self.always_on_top;
                        self.apply_topmost(&ctx);
                    }
                });
                ui.separator();
                ui.label(format!(
                    "默认引擎 {} · 目标语言 {} · 置顶 {}",
                    self.config.translation.default_engine,
                    self.config.translation.target,
                    if self.always_on_top { "开" } else { "关" }
                ));
            }
            UiMode::Compact => {
                ui.horizontal(|ui| {
                    ui.label("Ice Translator");
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui.selectable_label(self.always_on_top, "⌃ 置顶").clicked() {
                            self.always_on_top = !self.always_on_top;
                            self.apply_topmost(&ctx);
                        }
                        if ui.button("▣ 全量").clicked() {
                            self.apply_mode(&ctx, UiMode::Full);
                        }
                    });
                });
                ui.label("简洁模式骨架 · 划词翻译与贴边吸附在 P5 接入");
            }
        });
    }

    fn on_exit(&mut self) {
        self.config.ui.mode = self.mode;
        self.config.ui.always_on_top = self.always_on_top;
        if let Err(error) = self.config.save() {
            tracing::error!("配置未能保存: {error}");
        }
    }
}

fn main() -> eframe::Result {
    let filter = tracing_subscriber::EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info"));
    tracing_subscriber::fmt().with_env_filter(filter).init();

    let (config, warning) = Config::load().unwrap_or_else(|error| {
        tracing::error!("配置读取失败，以默认配置启动: {error}");
        (Config::default(), None)
    });

    // 启动形态直接由保存的模式决定，避免"先开大窗再缩成简洁窗"的闪烁（FR-3.1）
    let ui_cfg = config.ui;
    let rect = if ui_cfg.mode == UiMode::Compact {
        ui_cfg.compact_rect
    } else {
        ui_cfg.full_rect
    };
    let mut builder = ViewportBuilder::default()
        .with_title("Ice Translator")
        .with_inner_size([rect.w, rect.h])
        .with_position((rect.x, rect.y))
        .with_decorations(ui_cfg.mode == UiMode::Full)
        .with_window_level(if ui_cfg.always_on_top {
            WindowLevel::AlwaysOnTop
        } else {
            WindowLevel::Normal
        });
    if ui_cfg.mode == UiMode::Full {
        builder = builder.with_min_inner_size([640.0, 420.0]);
    }

    let options = eframe::NativeOptions {
        viewport: builder,
        ..Default::default()
    };

    eframe::run_native(
        "Ice Translator",
        options,
        Box::new(move |_cc| Ok(Box::new(IceApp::new(config, warning)))),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use eframe::egui;

    fn queued(ctx: &egui::Context) -> Vec<ViewportCommand> {
        ctx.viewport(|vp| vp.commands.clone())
    }

    /// ADR-001 的核心假设：单窗口两种形态只靠 ViewportCommand 就能表达，不需要多视口。
    #[test]
    fn switching_to_compact_queues_borderless_small_window() {
        let ctx = egui::Context::default();
        let mut app = IceApp::new(Config::default(), None);
        app.apply_mode(&ctx, UiMode::Compact);

        assert_eq!(app.mode, UiMode::Compact);
        let cmds = queued(&ctx);
        assert!(cmds.contains(&ViewportCommand::Decorations(false)));
        assert!(cmds.contains(&ViewportCommand::Resizable(false)));
        assert!(cmds.contains(&ViewportCommand::InnerSize(egui::vec2(460.0, 96.0))));
        assert!(cmds.contains(&ViewportCommand::OuterPosition(egui::pos2(1400.0, 320.0))));
    }

    #[test]
    fn switching_back_to_full_restores_decorations_and_size() {
        let ctx = egui::Context::default();
        let mut app = IceApp::new(Config::default(), None);
        app.apply_mode(&ctx, UiMode::Compact);
        app.apply_mode(&ctx, UiMode::Full);

        let cmds = queued(&ctx);
        assert!(cmds.contains(&ViewportCommand::Decorations(true)));
        assert!(cmds.contains(&ViewportCommand::Resizable(true)));
        assert!(cmds.contains(&ViewportCommand::InnerSize(egui::vec2(1080.0, 720.0))));
    }

    #[test]
    fn topmost_maps_to_always_on_top_window_level() {
        let ctx = egui::Context::default();
        let mut app = IceApp::new(Config::default(), None);
        app.always_on_top = true;
        app.apply_topmost(&ctx);

        assert!(queued(&ctx).contains(&ViewportCommand::WindowLevel(WindowLevel::AlwaysOnTop)));
        assert!(app.config.ui.always_on_top);
    }
}
