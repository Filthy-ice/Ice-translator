//! 整窗背景色（用户 2026-11-04："标题栏的颜色和界面不统一，有色差。你应该将颜色应用到整个界面，
//! 只不过输入和译文框是对应的浅色/半透色。"）。
//!
//! 色差不是 egui 里两处色值打架，而是**画那两条带子的根本不是同一个绘制者**：内容区由 egui 画，
//! 而标题栏那 32pt（连同红绿灯）由 AppKit 的 `NSThemeFrame` 画，压根不在 egui 的绘制清单里——
//! `system::alpha` 当年撞上的就是同一堵墙（"标题栏为什么不能跟着失焦半透明"）。改 `Visuals`
//! 永远改不到它，改多少次都还是两块色。
//!
//! 能让整扇窗户（含标题栏）用同一个底色的公开 API 只有一个：`NSWindow.setBackgroundColor:`。
//! 但**光有这一句改不到标题栏**（D-23 那一版就错在这里，把"它写的是窗口的底"当成了
//! "那条带子自然跟着变"）。真机取像素量下来（2026-11-05，精简窗 401×193，聚焦态）：
//! `y=0..30` 整条是 **`#FFFFFF`**，`y=31` 一道 `#E8_E8_E8` 分隔线，`y=32` 往下才是 `#F7_F9_FC`
//! 的基底——标题栏那一条由 `NSThemeFrame` 按**系统 chrome** 画，它不画窗口的底。
//!
//! 所以"整窗一个底色"是**两件事**，缺一件就是"半改完"：
//!
//! 1. [`set`]：把基底写进 `NSWindow.backgroundColor`（内容区之外那层的颜色，也是标题栏透出来
//!    要看到的那个色）；
//! 2. [`sync_titlebar`]：`titlebarAppearsTransparent = true`，让标题栏**不再画自己那块底**，
//!    于是窗口那一层透出来，与正文同一个色。红绿灯、拖动、绿键改道都不受影响——
//!    不画背景不等于不响应事件，内容视图也没铺上去（没开 `fullSizeContentView`），
//!    那 32pt 仍然归标题栏拖。
//!
//! 两级分层的规矩因此是：**基底**（`theme::window_base` 一个色值 → 原生窗口底色 + 标题栏平铺 +
//! egui 的 `window_fill`/`panel_fill`，四处同一个出处）；**正文面**（输入框与译文框用同色系
//! 浅一档的 `extreme_bg_color`，淡阴影挂在它们身上）。
//!
//! 这条对**将来换肤（任务 #28）是前置条件而不是偏好**：五套预设色只要喂 `window_base` 这一个
//! 字段，四个去处自动一起变。反过来，任何"这块单独配个色"或"标题栏另外画一次"的写法，
//! 都会在换肤时把这道色差按主题数放大成五份。
//!
//! 与 `system::alpha` 共用 `super::window::ns_window`。`set` 由调用方按"值变了才下发"调；
//! `sync_titlebar` **每帧读回来核对**——换形态时 winit 会换窗口（`system::dblclick` 早就记下这条
//! 事实），属性跟着回到系统默认，缓存一个"我设过了"就会让色差在下一次切换后悄悄回来。
//! 返回 `false` / `None` 表示这条路走不通（非 macOS、拿不到窗口句柄、不在主线程），调用方
//! **不缓存**，下一帧重试（第一帧可能还没有窗口）。

use eframe::egui::Color32;

/// 把整窗底色设为 `color`（含原生标题栏那条带子，前提是先 [`sync_titlebar`] 压平了那条带子）。
/// 返回是否真的下到了原生层。
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

/// 让标题栏**不再画自己那块底**，从而透出窗口的底色（[`set`] 写进去的那一层）。
/// 返回读回来的那一档；`None` = 这条路走不通（非 macOS / 没句柄 / 不在主线程）。
///
/// 为什么必须每帧核对而不是设一次：换形态时 winit 会换窗口（`system::dblclick` 记下过这条事实，
/// 它为此存的是被监听那块窗口的指针），`titlebarAppearsTransparent` 跟着回到系统默认。
/// 只设一次的话，色差会在第一次形态切换之后悄悄长回来，而缓存里还写着"我已经设过了"。
///
/// 已经是平铺的那一帧只读不写——这条 API 每次写都会让 `NSThemeFrame` 重画一次那 32pt，
/// 稳态下每帧一次消息就够。
#[cfg(target_os = "macos")]
pub fn sync_titlebar(frame: &eframe::Frame) -> Option<bool> {
    let ns_window = super::window::ns_window(frame)?;
    if ns_window.titlebarAppearsTransparent() {
        return Some(true);
    }
    ns_window.setTitlebarAppearsTransparent(true);
    // 读回来交出去：写没写成以窗口的现状为准，不以我们"调用过"为准（与 `system::titlebar`
    // 那枚绿灯同一纪律）。理论上是 true，真不是的话调用方会看到 `Some(false)` 并重来一帧。
    Some(ns_window.titlebarAppearsTransparent())
}

/// 非 macOS 没有标题栏底色这个概念（那条带子归窗口管理器画），恒 `None`，
/// 界面层什么都不做（NFR-7：不许因为原生没有就退化成第二套画法）。
#[cfg(not(target_os = "macos"))]
pub fn sync_titlebar(_frame: &eframe::Frame) -> Option<bool> {
    None
}
