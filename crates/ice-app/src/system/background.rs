//! 整窗背景色（用户 2026-11-04："标题栏的颜色和界面不统一，有色差。你应该将颜色应用到整个界面，
//! 只不过输入和译文框是对应的浅色/半透色。"）。
//!
//! 色差不是 egui 里两处色值打架，而是**画那两条带子的根本不是同一个绘制者**：内容区由 egui 画，
//! 而标题栏那 32pt（连同红绿灯）由 AppKit 的 `NSThemeFrame` 画，压根不在 egui 的绘制清单里——
//! `system::alpha` 当年撞上的就是同一堵墙（"标题栏为什么不能跟着失焦半透明"）。改 `Visuals`
//! 永远改不到它，改多少次都还是两块色。
//!
//! 能让整扇窗户（含标题栏）用同一个底色的公开 API 只有一个：`NSWindow.setBackgroundColor:`。
//! 它写的是窗口的底，`NSThemeFrame` 那条带子取的就是这一层。所以本应用的分层规矩是两级：
//!
//! 1. **基底**：[`theme::window_base`] 一个色值铺满整窗——原生这一句 + egui 的
//!    `window_fill`/`panel_fill`，两处同一个出处；
//! 2. **正文面**：输入框与译文框用同色系浅一档（`extreme_bg_color`），淡阴影挂在它们身上。
//!
//! 将来五套预设色（任务 #28）只需喂这一个"基底"字段给这两处；任何"这块单独配个色"的写法
//! 都会在这里重新长出同一道色差。
//!
//! 与 `system::alpha` 共用 `super::window::ns_window`，同一纪律：每帧核对、值变了才下发。
//! 返回 `false` 表示这条路走不通（非 macOS、拿不到窗口句柄、不在主线程），调用方**不缓存**，
//! 下一帧重试（第一帧可能还没有窗口）。

use eframe::egui::Color32;

/// 把整窗底色设为 `color`（含原生标题栏那条带子）。返回是否真的下到了原生层。
///
/// 不 panic、不重试、不打日志——这个函数每帧都会被调到，噪声留给调用方按"值变了"去记。
#[cfg(target_os = "macos")]
pub fn set(frame: &eframe::Frame, color: Color32) -> bool {
    let Some(ns_window) = super::window::ns_window(frame) else {
        return false;
    };
    // 分量是 0..=255 的整数，除以 255 之后必然落在 0..=1，构造不会失败。
    // 必须走 **sRGB** 那条：`Color32` 的分量就是 sRGB，而 `colorWithRed:...` 用的是
    // `deviceRGB`，在 P3 屏上会偏一档——那正好是要修的这个"色差"的另一种形态。
    // CGFloat 在 64 位目标机上就是 f64（`objc2-core-foundation/src/geometry.rs:18`）。
    let channel = |value: u8| f64::from(value) / 255.0;
    let ns_color = objc2_app_kit::NSColor::colorWithSRGBRed_green_blue_alpha(
        channel(color.r()),
        channel(color.g()),
        channel(color.b()),
        channel(color.a()),
    );
    ns_window.setBackgroundColor(Some(&ns_color));
    true
}

/// 非 macOS 没有等价入口（`ViewportCommand` 里没有"整窗底色"这一项），恒返回 false。
/// 那一侧的标题栏归窗口管理器画，色差不是我们能改的。
#[cfg(not(target_os = "macos"))]
pub fn set(_frame: &eframe::Frame, _color: Color32) -> bool {
    false
}
