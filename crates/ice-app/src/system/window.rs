//! 取到本进程那一个原生 `NSWindow`（`system::alpha` 与 `system::titlebar` 共用）。
//!
//! 路数与 eframe 自己定位红绿灯时用的完全一致（`eframe/src/native/macos.rs`）：
//! `Frame::winit_window()` → `HasWindowHandle::window_handle()` →
//! `AppKitWindowHandle.ns_view`（**指的是 NSView，不是窗口**）→ `NSView::window()`。
//!
//! 这条 unsafe 只写一遍是有原因的：两处都要在帧循环里按当前形态往原生层下东西，
//! 一旦取窗口的写法分叉，出的就是"一边有效一边无效"这类查不出来的缺陷（D-9 同源）。
//!
//! 返回 `None` 的三种情况：非主线程（调用点错了）、拿不到句柄、view 还没有窗口
//! （建窗与首个 pass 的先后不由我们决定）。调用方按"这一帧没下成，下一帧再试"处理。
#![cfg(target_os = "macos")]

use objc2::MainThreadMarker;
use objc2::rc::Retained;
use objc2_app_kit::{NSView, NSWindow};
use raw_window_handle::{HasWindowHandle, RawWindowHandle};

pub fn ns_window(frame: &eframe::Frame) -> Option<Retained<NSWindow>> {
    // 只在主线程上碰 AppKit。`ui` 回调本身就在主线程，拿不到 marker 说明调用点错了。
    MainThreadMarker::new()?;

    let window = frame.winit_window()?;
    let handle = window.window_handle().ok()?;
    let RawWindowHandle::AppKit(appkit) = handle.as_raw() else {
        return None;
    };

    // SAFETY: `ns_view` 由 winit 从 `NSWindow.contentView` 取出，指针在窗口存活期内有效；
    // 上面刚确认过在主线程上。与 eframe `native/macos.rs` 的 `ns_view_from_handle` 同一个取法。
    let ns_view: &NSView = unsafe { &*appkit.ns_view.as_ptr().cast::<NSView>() };
    ns_view.window()
}
