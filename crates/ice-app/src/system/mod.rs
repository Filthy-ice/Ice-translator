//! 应用侧的系统集成：钥匙串、剪贴板、快捷键、文件面板、单实例锁、屏幕可用区、整窗底色与不透明度、标题栏绿键、菜单栏托盘、鼠标按键现场状态。

pub mod alpha;
pub mod background;
pub mod clipboard;
pub mod dblclick;
pub mod hotkeys;
pub mod mouse;
pub mod panel;
pub mod screen;
pub mod secrets;
pub mod single_instance;
pub mod titlebar;
pub mod tray;
#[cfg(target_os = "macos")]
pub mod window;
