//! 贴图窗四周那道窗影的原生开关（用户 2026-11-12："截图后图片要有阴影，右键要有关闭
//! 图片周围阴影的选项，不然不突出"）。
//!
//! 为什么落在原生：egui 0.37 的 `has_shadow` 只是 `ViewportBuilder` 的**创建期**字段，
//! `ViewportCommand` 里没有运行期改阴影这一项（egui-winit 每帧更新时把它解构成
//! `_has_shadow` 忽略掉，只在创建那扇窗时用一次）。所以"右键随时开关"这一跳只能落在
//! `NSWindow.setHasShadow:`，与 `cover`（抬层）、`background`（底色）同族——都是 egui
//! 够不到、只能原生补的那一格。
//!
//! 窗影画在窗口 bounds **之外**，所以"贴图窗恒等于图片"那条几何不动、也不多出吃命中的
//! 死区。这正是当初不用 painter 在图四周垫一圈透明边画影的理由：那圈边会把拖动/右键的
//! 命中区搅乱（pin.rs 硬规矩 2 那条隐形陷阱）。
//!
//! 这是对 CHG-011 Q-002"with_has_shadow(false) 这一格不动它"的改判：Q-002 当时说的是
//! **药丸**那道淡影（用 painter 画），并明确不去动贴图窗的窗影；现在用户要的是**图片本体**
//! 的窗影，两件事。改判出处与状态面见 `state.rs` 的 `Pin::shadow`。

/// 把标题为 `title` 的那扇窗的窗影设成 `want`。**幂等**：已经对了只读不写。
///
/// 每帧都可调（与 `cover::cover_display` 同一条纪律：不缓存"我设过了"——换屏/重建窗
/// 之后属性会回默认）。拿不到窗（非 mac、不在主线程、那扇窗还没建出来）一律返回
/// `false`，调用方下一帧再试，不假装设上了。
#[cfg(target_os = "macos")]
pub fn apply(title: &str, want: bool) -> bool {
    let Some(window) = super::attach::find_window(title) else {
        return false;
    };
    if window.hasShadow() != want {
        window.setHasShadow(want);
    }
    window.hasShadow() == want
}

/// 非 macOS 没有这条原生入口（`ViewportCommand` 里没有运行期窗影），恒 `false`。
/// 那一侧贴图窗创建时按 builder 的 `has_shadow` 给一次，运行期不再改（NFR-7：不假装做到）。
#[cfg(not(target_os = "macos"))]
pub fn apply(_title: &str, _want: bool) -> bool {
    false
}
