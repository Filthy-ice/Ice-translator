//! 整窗不透明度（FR-3.7 的原生路径，2026-10-28 补）。
//!
//! 失焦半透明原本只有 `views::compact::apply_dim` 一条路：它乘的是 egui 那几层**底色**的
//! alpha，作用域封顶在内容区。用户走查点出的正是这条路的天花板——"标题栏为什么不能跟着
//! 失焦半透明"：标题栏（连同红绿灯按钮）由 AppKit 的 `NSThemeFrame` 绘制，压根不在 egui
//! 的绘制清单里，应用层改 `Visuals` 改不到它。
//!
//! 能让标题栏一起变淡的公开 API 只有一个：`NSWindow.alphaValue`。它作用于整个窗口表面，
//! 边框、标题栏、按钮全都跟着；代价是文字也一并变淡（`apply_dim` 当年特意只改底色）。
//! 这笔交换值不值由用户判，所以同一批把"透多少"做成了设置页里的滑杆。
//!
//! 拿窗口的路数与 eframe 自己定位红绿灯时用的完全一致（`eframe/src/native/macos.rs`）：
//! `Frame::winit_window()` → `HasWindowHandle::window_handle()` →
//! `AppKitWindowHandle.ns_view`（**指的是 NSView，不是窗口**）→ `NSView::window()`。
//! 非 macOS 平台没有等价且安全的"整窗 alpha"命令（`ViewportCommand` 里没有这一项），
//! 一律返回 false，由调用方退回 `apply_dim`。

/// 把整窗的不透明度设为 `alpha`（1.0 不透明，0.0 看不见）。
///
/// 返回**是否真的下到了原生层**：`false` 表示这条路走不通（非 macOS、拿不到窗口句柄、
/// 不在主线程），调用方应退回 egui 侧的 `apply_dim`。不 panic、不重试、不打日志——
/// 这个函数每帧都可能被调，噪声留给调用方按"值变了"去记。
#[cfg(target_os = "macos")]
pub fn set(frame: &eframe::Frame, alpha: f32) -> bool {
    use objc2::MainThreadMarker;
    use objc2_app_kit::NSView;
    use raw_window_handle::{HasWindowHandle, RawWindowHandle};

    // 只在主线程上碰 AppKit。`ui` 回调本身就在主线程，拿不到 marker 说明调用点错了。
    if MainThreadMarker::new().is_none() {
        return false;
    }

    let Some(window) = frame.winit_window() else {
        return false;
    };
    let Ok(handle) = window.window_handle() else {
        return false;
    };
    let RawWindowHandle::AppKit(appkit) = handle.as_raw() else {
        return false;
    };

    // SAFETY: `ns_view` 由 winit 从 `NSWindow.contentView` 取出，指针在窗口存活期内有效；
    // 上面刚确认过在主线程上。与 eframe `native/macos.rs` 的 `ns_view_from_handle` 同一个取法。
    let ns_view: &NSView = unsafe { &*appkit.ns_view.as_ptr().cast::<NSView>() };
    let Some(ns_window) = ns_view.window() else {
        return false;
    };
    // `setAlphaValue:` 是安全方法（无所有权转移，参数是 CGFloat）。
    // CGFloat 在 64 位平台上就是 f64（`objc2-core-foundation/src/geometry.rs:18`），
    // 目标机全是 arm64/x86_64，直接转；不为此多引一个 crate。
    ns_window.setAlphaValue(alpha as f64);
    true
}

/// 非 macOS 没有整窗 alpha 的公开入口，恒返回 false，让调用方走 `apply_dim`。
#[cfg(not(target_os = "macos"))]
pub fn set(_frame: &eframe::Frame, _alpha: f32) -> bool {
    false
}
