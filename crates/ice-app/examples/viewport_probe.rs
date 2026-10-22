//! CHG-004 T-001 的实测探针 v3（一次性，不进正式产物）。
//!
//! v1 已定：H1 无边框可边缘缩放通过；H3 整窗透明通过；H2 逐帧 `InnerSize` 补间**不通过**
//! （60fps 发命令，200ms 里只有 4~5 次落到窗口上，并复现 ~1.0s 事件循环停摆）。
//! v2 已定：A2（降到 30fps）同样不通过；C（窗内补间）通过——14 帧 / 204ms / 16.5ms±1ms。
//!
//! v3 只补一件事：v2 的 C 按钮把终点写死成 COMPACT，「复原」是单发命令，所以用户只看到
//! "缩下去有动画、弹回来是瞬移"。v3 让 C **双向对称**：
//! - 静止时窗口就是当前形态的精确 rect（全量 640×420@(200,160) / 简洁 460×96@(900,700)）；
//! - 点任一方向按钮 → 命令① 立刻把窗口设成两端 rect 的**外接矩形**（透明，看不见），
//!   随后每帧只插值**画出来的内容矩形**（位置 + 尺寸），t≥1 时命令② 收到精确终点；
//! - 一次动画最多 2 条几何命令（反向重入才会到 3 条），日志里用 `cmds=` 累计可核。
//!
//! 另留「瞬移」按钮作无动画对照，即 v2 里被判定为"直接卡到大"的那种效果。
//!
//! 跑法：
//! ```sh
//! cargo run -p ice-app --example viewport_probe 2>&1 \
//!   | tee features/CHG-004/evidence/T-001-probe-v3.log
//! ```

use eframe::egui;
use egui::{Align, Color32, Layout, Pos2, Rect, StrokeKind, Vec2, ViewportCommand};

/// 补间时长，与 design.md 的 `MORPH_DURATION` 一致
const TWEEN_SECS: f64 = 0.2;
/// 简洁窗默认尺寸（`03-ui-design.md` §3）
const COMPACT: Vec2 = Vec2::new(460.0, 96.0);
/// 探针初始尺寸
const FULL: Vec2 = Vec2::new(640.0, 420.0);

/// 全量形态的屏幕矩形（与 `UiConfig` 默认 window_rect 同量级）
fn full_screen() -> Rect {
    Rect::from_min_size(Pos2::new(200.0, 160.0), FULL)
}

/// 简洁形态的屏幕矩形（右下靠边，模拟贴边悬浮条）
fn compact_screen() -> Rect {
    Rect::from_min_size(Pos2::new(900.0, 700.0), COMPACT)
}

fn main() -> eframe::Result {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .with_target(false)
        .with_writer(std::io::stderr)
        .init();

    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("CHG-004 viewport probe v3")
            .with_transparent(true)
            .with_decorations(false)
            .with_resizable(true)
            .with_min_inner_size(Vec2::ZERO)
            .with_inner_size(FULL)
            .with_position(full_screen().min),
        ..Default::default()
    };
    eframe::run_native(
        "viewport_probe",
        options,
        Box::new(|_cc| Ok(Box::new(Probe::default()))),
    )
}

struct Probe {
    morph: Option<Morph>,
    last: Option<Geom>,
    frames: u32,
    /// 已发出的几何命令条数（一次动画应 ≤3）
    cmds: u32,
    /// 静止时窗口应处的屏幕矩形
    current: Rect,
    /// 补间中：窗口内坐标系里画出来的内容矩形
    panel: Option<Rect>,
}

impl Default for Probe {
    fn default() -> Self {
        Self {
            morph: None,
            last: None,
            frames: 0,
            cmds: 0,
            current: full_screen(),
            panel: None,
        }
    }
}

#[derive(Debug, Clone, Copy)]
struct Morph {
    start: f64,
    from: Rect,
    to: Rect,
    union: Rect,
}

#[derive(Debug, Clone, Copy, PartialEq)]
struct Geom {
    inner: Vec2,
    outer_pos: Pos2,
    focused: bool,
}

/// smoothstep，与 v1/v2 一致，保证三次实测的曲线可比
fn ease(t: f64) -> f32 {
    let t = t.clamp(0.0, 1.0);
    (t * t * (3.0 - 2.0 * t)) as f32
}

fn lerp_rect(from: Rect, to: Rect, e: f32) -> Rect {
    Rect {
        min: from.min + (to.min - from.min) * e,
        max: from.max + (to.max - from.max) * e,
    }
}

impl eframe::App for Probe {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        let now = ctx.input(|i| i.time);
        let info = ctx.input(|i| i.viewport().clone());
        let geom = Geom {
            inner: info.inner_rect.map(|r| r.size()).unwrap_or_default(),
            outer_pos: info
                .outer_rect
                .map(|r| r.min)
                .unwrap_or(Pos2::new(0.0, 0.0)),
            focused: info.focused.unwrap_or(false),
        };
        if Some(geom) != self.last {
            tracing::info!(
                "geom frame={} inner={:.0}x{:.0} outer=({:.0},{:.0}) focused={} cmds={}",
                self.frames,
                geom.inner.x,
                geom.inner.y,
                geom.outer_pos.x,
                geom.outer_pos.y,
                geom.focused,
                self.cmds,
            );
            self.last = Some(geom);
        }
        self.frames += 1;

        if let Some(m) = self.morph {
            let t = (now - m.start) / TWEEN_SECS;
            let e = ease(t);
            let content = lerp_rect(m.from, m.to, e);
            // 窗口内坐标 = 内容屏幕坐标 - 窗口真实原点（每帧重算，
            // 这样命令① 异步落地的那几帧也不会让内容弹一下）
            self.panel = Some(Rect::from_min_size(
                content.min - geom.outer_pos.to_vec2(),
                content.size(),
            ));
            tracing::info!(
                "C t={t:.3} frame={} content={:.0}x{:.0}@({:.0},{:.0}) window={:.0}x{:.0}@({:.0},{:.0}) cmds={}",
                self.frames,
                content.width(),
                content.height(),
                content.min.x,
                content.min.y,
                geom.inner.x,
                geom.inner.y,
                geom.outer_pos.x,
                geom.outer_pos.y,
                self.cmds,
            );
            if t >= 1.0 {
                // 命令②：一次性收到精确终点。透明区域本来就看不见，这一跳应当无感。
                self.cmds += 1;
                tracing::info!(
                    "C cmd② frame={} union={:.0}x{:.0} → InnerSize({:.0}x{:.0}) OuterPosition({:.0},{:.0}) 用时 {:.0}ms",
                    self.frames,
                    m.union.width(),
                    m.union.height(),
                    m.to.width(),
                    m.to.height(),
                    m.to.min.x,
                    m.to.min.y,
                    (now - m.start) * 1e3,
                );
                ctx.send_viewport_cmd(ViewportCommand::InnerSize(m.to.size()));
                ctx.send_viewport_cmd(ViewportCommand::OuterPosition(m.to.min));
                self.current = m.to;
                self.morph = None;
                self.panel = None;
            } else {
                ctx.request_repaint();
            }
        }

        // 透明背景：策略 C 全靠这个把"窗口比内容大"藏起来
        egui::CentralPanel::default()
            .frame(egui::Frame::NONE.fill(Color32::TRANSPARENT))
            .show(ui, |ui| {
                let full = ui.max_rect();
                let panel = self.panel.unwrap_or(full);

                // 窗口真实边界画一层极淡的框，方便肉眼确认透明余量有多大
                ui.painter()
                    .rect_stroke(full, 0.0, (1.0, Color32::from_gray(90)), StrokeKind::Middle);
                ui.painter().rect_filled(
                    panel,
                    10.0,
                    if self.panel.is_some() {
                        Color32::from_rgb(40, 70, 110)
                    } else {
                        Color32::from_rgb(45, 48, 54)
                    },
                );

                let tall = panel.height() > 150.0;
                let mut inner = ui.new_child(
                    egui::UiBuilder::new()
                        .max_rect(panel.shrink(10.0))
                        .layout(Layout::top_down(Align::Min)),
                );
                inner.label("CHG-004 probe v3");
                if tall {
                    inner.label(format!(
                        "window {:.0}×{:.0} @({:.0},{:.0}) focused {} cmds {}",
                        geom.inner.x, geom.inner.y, geom.outer_pos.x, geom.outer_pos.y,
                        geom.focused, self.cmds
                    ));
                }
                inner.horizontal_wrapped(|ui| {
                    if ui
                        .button("全量 → 简洁（C）")
                        .on_hover_text("内容缩到 460×96，窗口先变成两端的外接矩形，末帧再收准")
                        .clicked()
                    {
                        self.start_morph(compact_screen(), now, &ctx);
                    }
                    if ui
                        .button("简洁 → 全量（C）")
                        .on_hover_text("同一条 C，方向相反：内容从 460×96 长回 640×420")
                        .clicked()
                    {
                        self.start_morph(full_screen(), now, &ctx);
                    }
                    if ui
                        .button("瞬移（无动画，对照）")
                        .on_hover_text("单发几何命令，即 v2 里被判定为「直接卡到大」的效果")
                        .clicked()
                    {
                        let to = if self.current == full_screen() {
                            compact_screen()
                        } else {
                            full_screen()
                        };
                        self.morph = None;
                        self.panel = None;
                        self.cmds += 1;
                        tracing::info!(
                            "JUMP frame={} → {:.0}x{:.0}@({:.0},{:.0})",
                            self.frames,
                            to.width(),
                            to.height(),
                            to.min.x,
                            to.min.y,
                        );
                        ctx.send_viewport_cmd(ViewportCommand::InnerSize(to.size()));
                        ctx.send_viewport_cmd(ViewportCommand::OuterPosition(to.min));
                        self.current = to;
                    }
                });
                if tall {
                    inner.separator();
                    inner.label("两个方向都点一遍：应都是 200ms 连续形变，末帧那一下看不见。");
                    inner.label("再点「瞬移」对照，确认差别就是有没有动画。");
                }
            });
    }
}

impl Probe {
    /// 命令①：窗口瞬时变成两端的外接矩形（透明 → 不可见），随后只在窗口内插值内容矩形。
    fn start_morph(&mut self, to: Rect, now: f64, ctx: &egui::Context) {
        let from = self.current;
        let u = from.union(to);
        self.cmds += 1;
        tracing::info!(
            "--- C cmd① frame={} from {:.0}x{:.0}@({:.0},{:.0}) → to {:.0}x{:.0}@({:.0},{:.0}) union {:.0}x{:.0}@({:.0},{:.0})",
            self.frames,
            from.width(),
            from.height(),
            from.min.x,
            from.min.y,
            to.width(),
            to.height(),
            to.min.x,
            to.min.y,
            u.width(),
            u.height(),
            u.min.x,
            u.min.y,
        );
        ctx.send_viewport_cmd(ViewportCommand::InnerSize(u.size()));
        ctx.send_viewport_cmd(ViewportCommand::OuterPosition(u.min));
        self.morph = Some(Morph {
            start: now,
            from,
            to,
            union: u,
        });
        self.panel = Some(Rect::from_min_size(from.min, from.size()));
    }
}
