//! 应用侧的系统集成：钥匙串、剪贴板、快捷键、文件面板、单实例锁、屏幕可用区、整窗底色与不透明度、标题栏绿键、Dock 磁贴图标、菜单栏托盘、鼠标按键现场状态、区域截图。

pub mod alpha;
pub mod attach;
pub mod background;
pub mod bar;
pub mod capture;
pub mod clipboard;
pub mod cover;
pub mod dblclick;
pub mod dock;
pub mod global_hotkeys;
pub mod hotkeys;
pub mod mouse;
pub mod panel;
pub mod pill;
pub mod pin;
pub mod probe;
pub mod screen;
pub mod secrets;
pub mod selection;
pub mod shadow;
pub mod single_instance;
pub mod titlebar;
pub mod tray;
#[cfg(target_os = "macos")]
pub mod window;
pub mod windows;

/// 叫醒**主窗那一扇视口**。子窗（贴图窗、遮罩窗）里推完意图之后必须调它。
///
/// 为什么不能直接用 `ctx.request_repaint()`：egui 按"当前视口 id"派发重画，子窗闭包里
/// 那句只会重画**子窗自己**，主窗一次都不会醒。真机读数是同一形状：
/// 遮罩那边"esc 按了选区没了遮罩还在、对勾点了什么都不钉"；贴图那边"点关闭要么没反应、
/// 要么二十来秒才关上"——那二十秒就是主窗下一次恰好重画的时刻。意图已经进槽里了，没人来取。
pub(crate) fn wake_root(ctx: &egui::Context) {
    ctx.request_repaint_of(egui::ViewportId::ROOT);
}

/// 形状核查看的那一份：**只留代码行，注释去掉，测试那一段整块不算**。
///
/// 各模块的形状核查要看的是"代码里到底写了什么"，不是注释里讨论了什么——本仓的注释专门
/// 解释过为什么**不**走某条写法（背景拖动、`Pos2::ZERO` 那种底色、拿 `request_repaint`
/// 当叫醒主窗），测试的断言消息里也写着这些反面例子；把它们一起数进去，核查就自己骗自己了。
///
/// 判据是"**第一个 `#[cfg(test)]` 之前**"，所以本仓的约定跟着定下来：**测试与只给测试用的
///
/// **这条读法不适用于 `app.rs`**：那个文件里 `#[cfg(test)]` 也落在结构体字段与语句上
/// （`test_screens` 那一格），照本读法会把它后面的真代码一起切掉。要核 `app.rs` 里的某一处，
/// 就**只读那一处自己的文本**（见 `state.rs` 的 `each_capture_carries_its_own_generation…`
/// 怎么取 `fn freeze_texture` 那一段），别拿整文件截断去读它。
/// 夹具一律放在文件末尾**。`selection.rs` 原先把测试夹在中间（几何之后、遮罩窗之前），
/// 那一档会让"之后"的真代码整体看不见，2026-11-09 已把它搬回末尾。
#[cfg(test)]
pub(crate) fn implementation_lines(source: &str) -> String {
    source
        .split("#[cfg(test)]")
        .next()
        .unwrap_or(source)
        .lines()
        .filter(|line| !line.trim_start().starts_with("//"))
        .collect::<Vec<_>>()
        .join("\n")
}
