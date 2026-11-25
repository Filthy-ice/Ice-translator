//! 自绘标题栏（Linux 档，2026-11-26 用户"让标题栏和界面一致啊！"）。
//!
//! 为什么必须自绘：Linux 的标题栏归窗口管理器画，GNOME/Mutter 按**系统**主题给它上色，
//! 我们打的 `_GTK_THEME_VARIANT` 它不认（实测父框上根本没有这个属性）。于是"应用深色 +
//! 系统浅色"时标题栏永远白——三个档位里总有一边对不上。唯一能保证一致的做法：
//! 去掉系统装饰，这条带子由我们用 egui 画，颜色和内容区同一个出处。
//!
//! 只画三样：拖动区、标题、三颗按钮（最小化 / 切形态 / 关闭）。mac 档不开这个模块：
//! 那边的标题栏是 AppKit 的，`background::sync_titlebar` 已经让它跟主题走。
#![cfg(target_os = "linux")]

use eframe::egui::{self, CursorIcon, Pos2, Rect, ResizeDirection, Sense, Vec2, ViewportCommand};

use super::Effects;
use crate::theme;

/// 标题栏高度。32 与 mac 标题栏同高（`theme.rs` 提到的那 32pt），换平台时视觉一致。
pub const HEIGHT: f32 = 32.0;
/// 右侧三颗按钮的方块边长
const BUTTON: f32 = 24.0;
/// 按钮之间的间距
const GAP: f32 = 6.0;

/// 画一条标题栏并把交互写进 `effects`。返回它占掉的高度（调用方据此排正文）。
pub fn show(ui: &mut egui::Ui, effects: &mut Effects, compact: bool) {
    let fill = theme::surface_fill(ui.visuals());
    // 整窗先铺一层底：去掉系统装饰后没有不透明 backing，任何没画到的像素都会透出桌面/
    // 后面的窗口（用户 2026-11-26 报的"中间有条缝"就是标题栏与正文之间那一条没被覆盖）。
    ui.painter()
        .rect_filled(ui.max_rect(), egui::CornerRadius::ZERO, fill);
    let width = ui.available_width();
    let (rect, response) =
        ui.allocate_exact_size(Vec2::new(width, HEIGHT), Sense::click_and_drag());
    ui.painter()
        .rect_filled(rect, egui::CornerRadius::ZERO, fill);

    // 按钮从右往左排：✕ / ▢ / —
    let mut right = rect.max.x - GAP;
    let mut button = |ui: &mut egui::Ui, glyph: &str| -> egui::Response {
        let r = Rect::from_min_size(
            Pos2::new(right - BUTTON, rect.center().y - BUTTON / 2.0),
            Vec2::splat(BUTTON),
        );
        right = r.min.x - GAP;
        let resp = ui.interact(r, ui.scope_id().with(glyph), Sense::click());
        let color = if resp.hovered() {
            ui.visuals().widgets.hovered.fg_stroke.color
        } else {
            ui.visuals().weak_text_color()
        };
        ui.painter().text(
            r.center(),
            egui::Align2::CENTER_CENTER,
            glyph,
            egui::FontId::proportional(13.0),
            color,
        );
        resp
    };
    let close = button(ui, "✕");
    let toggle = button(ui, "▢");
    let minimize = button(ui, "—");

    if close.clicked() {
        // 与系统关闭按钮同一条路：交给帧循环的关闭处理（收进托盘 / 最小化）
        ui.ctx().send_viewport_cmd(ViewportCommand::Close);
    }
    if toggle.clicked() {
        effects.toggle_mode = true;
    }
    if minimize.clicked() {
        ui.ctx().send_viewport_cmd(ViewportCommand::Minimized(true));
    }

    // 标题居中（不抢按钮那一块）
    if width > 160.0 {
        ui.painter().text(
            Pos2::new(rect.center().x, rect.center().y),
            egui::Align2::CENTER_CENTER,
            "Ice Translator",
            egui::FontId::proportional(13.0),
            ui.visuals().strong_text_color(),
        );
    }

    // 拖动：按在带上（按钮之外的区域）拖 → 交原生窗口拖拽；双击 → 铺满/还原
    let on_buttons = close.hovered() || toggle.hovered() || minimize.hovered();
    if !on_buttons {
        if response.drag_started() {
            effects.start_drag = true;
        }
        if response.double_clicked() {
            // 精简窗双击 = 切到全量（再双击回到精简，尺寸原样回来）。
            // **不走 `toggle_fill`**：那条路的"还原"固定按 `UiMode::Full` 算尺寸，
            // 在精简窗里双击两次会变成全量窗那么大（2026-11-26 用户实测）。
            if compact {
                effects.toggle_mode = true;
            } else {
                effects.toggle_fill = true;
            }
        }
    }

    resize_handles(ui);
}

/// 边缘缩放把手（2026-11-26 用户"补上边缘缩放把手"）。
///
/// 去掉系统装饰后，窗口边缘的缩放归我们：三条边 + 两个下角各铺一条 8px 的透明热区，
/// 按下即 `BeginResize`（winit 的 `drag_resize_window`，与系统拖拽同一套），悬停换光标。
/// 上边不铺：那一条是标题栏的拖动区。
fn resize_handles(ui: &mut egui::Ui) {
    const EDGE: f32 = 8.0;
    const CORNER: f32 = 14.0;
    let full = ui.max_rect();
    let zones = [
        (
            Rect::from_min_max(
                Pos2::new(full.min.x, full.min.y + CORNER),
                Pos2::new(full.min.x + EDGE, full.max.y - CORNER),
            ),
            ResizeDirection::West,
            CursorIcon::ResizeHorizontal,
        ),
        (
            Rect::from_min_max(
                Pos2::new(full.max.x - EDGE, full.min.y + CORNER),
                Pos2::new(full.max.x, full.max.y - CORNER),
            ),
            ResizeDirection::East,
            CursorIcon::ResizeHorizontal,
        ),
        (
            Rect::from_min_max(
                Pos2::new(full.min.x + CORNER, full.max.y - EDGE),
                Pos2::new(full.max.x - CORNER, full.max.y),
            ),
            ResizeDirection::South,
            CursorIcon::ResizeVertical,
        ),
        (
            Rect::from_min_max(
                Pos2::new(full.min.x, full.max.y - CORNER),
                Pos2::new(full.min.x + CORNER, full.max.y),
            ),
            ResizeDirection::SouthWest,
            CursorIcon::ResizeSouthWest,
        ),
        (
            Rect::from_min_max(
                Pos2::new(full.max.x - CORNER, full.max.y - CORNER),
                full.max,
            ),
            ResizeDirection::SouthEast,
            CursorIcon::ResizeSouthEast,
        ),
    ];
    for (index, (zone, direction, cursor)) in zones.into_iter().enumerate() {
        let resp = ui.interact(zone, ui.scope_id().with(("resize", index)), Sense::drag());
        if resp.hovered() || resp.dragged() {
            ui.ctx().set_cursor_icon(cursor);
        }
        if resp.drag_started() {
            ui.ctx()
                .send_viewport_cmd(ViewportCommand::BeginResize(direction));
        }
    }
}
