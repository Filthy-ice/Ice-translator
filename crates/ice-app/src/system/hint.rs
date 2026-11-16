//! 后台也看得见的那条小提示（用户 2026-11-16：F2 翻到头 / 没记录时，主窗收着也要能看见）。
//!
//! 主窗收起来时 eframe 不跑 egui pass，`views::toasts` 那条画在主窗里的提示自然没人看。
//! 这一扇是**独立浮窗**：透明底、无边框、置顶、鼠标穿透，浮在屏幕上方居中，1s 线性淡出
//! （复用 `Toast` 的寿命与步长）。活着时每帧重 show 一次（与贴图窗同一条 deferred 纪律：
//! 只 show 一次下一帧就被 egui 以"这一 pass 没用过"撤掉），到期那一帧起不再 show，窗自己没。

use egui::{Align2, Context, Order, Pos2, Rect, RichText, Vec2, ViewportBuilder, ViewportId};

use crate::state::Toast;
use crate::theme;

/// 浮窗按多大的可见物体去量：给足余量，药丸居中画，宁可窗大一点也不把字裁掉。
const HINT_SIZE: Vec2 = Vec2::new(360.0, 56.0);

fn viewport_id() -> ViewportId {
    ViewportId::from_hash_of("ice-float-hint")
}

/// 屏幕上方居中的落点。拿不到桌面边界就退回一块假定桌面，宁可不完美也别 panic。
fn anchor(ctx: &Context) -> Pos2 {
    let desktop = crate::system::screen::desktop(ctx)
        .unwrap_or(Rect::from_min_size(Pos2::ZERO, Vec2::new(1280.0, 800.0)));
    Pos2::new(desktop.center().x - HINT_SIZE.x / 2.0, desktop.top() + 48.0)
}

/// 活着就每帧 show 一次；`hint` 为 `None` 时这一帧不 show，窗就被 egui 撤掉。
pub fn show(ctx: &Context, hint: Option<&Toast>) {
    let Some(hint) = hint else {
        return;
    };
    // 闭包要 `'static`：这一帧的三份读数按值带走，下一帧重新取（与贴图窗同一条纪律）。
    let alpha = hint.opacity();
    let tone = hint.tone;
    let text = hint.text.clone();
    let builder = ViewportBuilder::default()
        .with_inner_size(HINT_SIZE)
        .with_position(anchor(ctx))
        .with_decorations(false)
        .with_transparent(true)
        .with_has_shadow(false)
        .with_always_on_top()
        .with_mouse_passthrough(true);
    ctx.show_viewport_deferred(viewport_id(), builder, move |ui, _class| {
        // 透明窗不铺底色，只画居中那块药丸——与主窗 `views::toasts` 同一套 Frame::popup，
        // 整块（底色、描边、投影、文字）一起乘同一个 alpha，不会出现"框还在、字先没了"。
        let child = ui.ctx().clone();
        egui::Area::new(egui::Id::unique("ice-float-hint-band"))
            .anchor(Align2::CENTER_CENTER, [0.0, 0.0])
            .order(Order::Foreground)
            .interactable(false)
            .show(&child, |ui| {
                egui::Frame::popup(ui.style())
                    .multiply_with_opacity(alpha)
                    .show(ui, |ui| {
                        ui.label(
                            RichText::new(&text)
                                .color(theme::tone_color(tone).gamma_multiply(alpha)),
                        );
                    });
            });
    });
}
