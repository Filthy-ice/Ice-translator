//! 物理鼠标的现场状态（2026-10-28 补，缺陷 D-16；2026-11-04 加光标位置，缺陷 D-22）。
//!
//! 两处需要它，都是同一件事：**egui 里的指针状态是"事件投递给我们了吗"的代理量，不是硬件现状**。
//! - 按键：原生拖拽期间 egui 收不到 pointer 事件。`ViewportCommand::StartDrag`
//!   和系统标题栏那两路拖动都进的是 AppKit 自己的嵌套事件循环，鼠标抬起那一下不会投递到本
//!   应用的 view，所以 `ctx.input(...)` 里的按键状态在整段拖动期间是**旧的**。
//! - 位置：应用不在前台时那一路投递会停，`hover_pos()` 说指针消失了，而鼠标还压在窗口上
//!   ——停靠的自动收回挂在它身上，出的就是"手还没到边，条先缩回去了"。
//!
//! 而吸附、停靠、拖离解除全挂在"松手了没有""指针还在不在"上。只看几何（`mode::DragSettler`）
//! 的旧判法会把"拖到边上停住但没松手"当成松手，当场把窗口从用户指针底下藏到边外——用户原话
//! "虽然我将其拖动到边上，但我还没有松手，他就着急往里面收入，导致我只要一直不松手，鼠标和界面的距离会
//! 越来越大"。这里问的是**硬件此刻的状态**，与事件投递无关，正好补上那一块。
//!
//! 返回 `None` 表示"问不出来"：非 macOS、或不在主线程上（调用点错了）。调用方必须把它当
//! "不知道"，退回原来的判法——**不许因为问不到就永远不吸附、也永远不收**。

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

/// Linux 档：问窗口服务器当前的按键掩码（`XQueryPointer` 的 `mask` 第 8 位＝左键）。
/// 与 mac 的 `+pressedMouseButtons` 同一个思路：读的是服务器里的当前状态，不是事件流。
#[cfg(target_os = "linux")]
pub fn left_button_down() -> Option<bool> {
    if cfg!(test) {
        // 测试里恒"不知道"：与 mac 上测试线程拿不到 `MainThreadMarker` 是同一个效果。
        // 真去问 X 会把跑测试那一刻的真实光标读进来，无头断言就被"他当时鼠标在哪"左右。
        return pinned_pointer().map(|(_, _, mask)| mask & super::window_x11::BUTTON1_MASK != 0);
    }
    super::window_x11::pointer().map(|(_, _, mask)| mask & super::window_x11::BUTTON1_MASK != 0)
}

#[cfg(not(any(target_os = "macos", target_os = "linux")))]
pub fn left_button_down() -> Option<bool> {
    None
}

/// 光标此刻在屏幕上的位置（egui 坐标系，与 `viewport().outer_rect` 同一原点）。
///
/// D-16 那条纪律的第二处应用：位置那一路投递一停，`hover_pos()` 就说指针消失了，而鼠标还压
/// 在窗口上——停靠的收回计时挂在它身上，出的就是用户 2026-11-04 那句"我鼠标还没有移到最左边/
/// 最右边的时候他又缩进去了"。`+mouseLocation` 读的是窗口服务器里的当前光标，与投递无关，
/// 也不需要任何权限。
///
/// 返回 `None` 表示问不出来：非 macOS、不在主线程（无头的帧测试正是这种）、或拿不到主屏高度。
/// 调用方必须把它当"不知道"，退回 `hover_pos`——**不许因为问不到就永远不收**。
#[cfg(target_os = "macos")]
pub fn cursor() -> Option<egui::Pos2> {
    MainThreadMarker::new()?;
    // 主屏高度是翻转的基准，与光标落在哪块屏无关（同 `screen::flip` 的取法）
    let main_height = crate::system::screen::probe()?.first()?.frame.h;
    let point = NSEvent::mouseLocation();
    Some(crate::system::screen::flip_point(
        (point.x as f32, point.y as f32),
        main_height,
    ))
}

/// Linux 档：`XQueryPointer` 给的是 root 坐标（左上原点），与 egui 的坐标系同一个原点，
/// 不需要 mac 那道翻转。问不到（没有 X）交回 `None`，调用方照旧退回事件代理量。
#[cfg(target_os = "linux")]
pub fn cursor() -> Option<egui::Pos2> {
    let (x, y, _) = if cfg!(test) {
        pinned_pointer()?
    } else {
        super::window_x11::pointer()?
    };
    Some(egui::pos2(x as f32, y as f32))
}

/// 测试里那条线程钉住的答案；生产构建恒 `None`（`cfg!(test)` 那两处就会直接去问 X）
#[cfg(target_os = "linux")]
fn pinned_pointer() -> Option<(i32, i32, u16)> {
    #[cfg(test)]
    {
        TEST_POINTER.with(|slot| (*slot.borrow()).flatten())
    }
    #[cfg(not(test))]
    {
        None
    }
}

#[cfg(all(test, target_os = "linux"))]
thread_local! {
    static TEST_POINTER: std::cell::RefCell<Option<Option<(i32, i32, u16)>>> =
        const { std::cell::RefCell::new(None) };
}

#[cfg(not(any(target_os = "macos", target_os = "linux")))]
pub const fn cursor() -> Option<egui::Pos2> {
    None
}

use eframe::egui;
#[cfg(target_os = "macos")]
use objc2::MainThreadMarker;
#[cfg(target_os = "macos")]
use objc2_app_kit::NSEvent;

#[cfg(test)]
mod tests {
    use super::*;

    /// 无头测试跑在工作线程上，`cursor()` 在那里必须问不出来——停靠那条链靠这个退回
    /// `hover_pos`，测试才测得到与今天一致的行为。
    #[test]
    fn the_cursor_is_unaskable_off_the_main_thread() {
        assert_eq!(cursor(), None, "工作线程上不许假装知道光标在哪");
    }
}
