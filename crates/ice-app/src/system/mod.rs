//! 应用侧的系统集成：钥匙串、剪贴板、快捷键、单实例锁、屏幕可用区、整窗不透明度、标题栏绿键。

pub mod alpha;
pub mod clipboard;
pub mod hotkeys;
pub mod screen;
pub mod secrets;
pub mod single_instance;
pub mod titlebar;
#[cfg(target_os = "macos")]
pub mod window;
