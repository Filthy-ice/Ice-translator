//! 物理鼠标按键的现场状态（2026-10-28 补，缺陷 D-16）。
//!
//! 只此一处需要它：**原生拖拽期间 egui 收不到 pointer 事件**。`ViewportCommand::StartDrag`
//! 和系统标题栏那两路拖动都进的是 AppKit 自己的嵌套事件循环，鼠标抬起那一下不会投递到本
//! 应用的 view，所以 `ctx.input(...)` 里的按键状态在整段拖动期间是**旧的**。
//!
//! 而吸附、停靠、拖离解除全挂在"松手了没有"上。只看几何（`mode::DragSettler`）的旧判法会把
//! "拖到边上停住但没松手"当成松手，当场把窗口从用户指针底下藏到边外——用户原话"虽然我将其
//! 拖动到边上，但我还没有松手，他就着急往里面收入，导致我只要一直不松手，鼠标和界面的距离会
//! 越来越大"。这里问的是**硬件此刻的按键状态**，与事件投递无关，正好补上那一块。
//!
//! 返回 `None` 表示"问不出来"：非 macOS、或不在主线程上（调用点错了）。调用方必须把它当
//! "不知道"，退回纯时间判定——**不许因为问不到就永远不吸附**。

/// 位 0 = 左键；窗口只能被左键拖着走（右键/中间键那两路拖动 AppKit 不提供）。
/// 类型跟 `+pressedMouseButtons` 的返回一致，写死 `u64` 在 32 位上就不是同一个类型了。
#[cfg(target_os = "macos")]
const LEFT_BUTTON_BIT: objc2::ffi::NSUInteger = 1 << 0;

/// 左键此刻是否按着（与"事件有没有投递给我们"无关）。
#[cfg(target_os = "macos")]
pub fn left_button_down() -> Option<bool> {
    // 与 `system::window` 同一条规矩：只在主线程上碰 AppKit。
    // 无头的帧测试跑在工作线程上，这里自然返回 None，测试走注入的闭包而不是真硬件。
    MainThreadMarker::new()?;
    // `+pressedMouseButtons` 是类方法，读的是窗口服务器维护的按键掩码，无需任何权限。
    Some(NSEvent::pressedMouseButtons() & LEFT_BUTTON_BIT != 0)
}

/// 非 macOS 没有等价且免权限的"硬件按键状态"入口，恒返回 `None`，由调用方退回时间判定。
#[cfg(not(target_os = "macos"))]
pub fn left_button_down() -> Option<bool> {
    None
}

#[cfg(target_os = "macos")]
use objc2::MainThreadMarker;
#[cfg(target_os = "macos")]
use objc2_app_kit::NSEvent;
