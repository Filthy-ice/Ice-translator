//! 认窗（CHG-010 REQ-108）：把"鼠标底下那扇窗"算出来。
//!
//! 这一格**不读屏幕内容、不做对比度分析**：系统本来就给着一张在线窗口表
//! （矩形 + 层级 + 归属 + 从前往后的顺序），本机实测能读到别家应用的窗口边界。
//! 所以这里只做纯挑选，读取那张表的那一步在 `windows_native`，两者分开是为了
//! 让"排除自己、排除桌面、取第一条包含该点的"这些判断能在无头里被断言
//! —— 真机跑一次只给一个样本，而这几条判断恰恰是最容易只对一半的。

use egui::{Pos2, Rect};

/// 窗口表里的一条。`rect` 是**全局点坐标**（左上原点，与 `CGWindowList` 一致）。
#[derive(Debug, Clone, PartialEq)]
pub struct Entry {
    pub rect: Rect,
    /// 窗口层级：主屏菜单栏 24、Dock 20、普通窗 0、桌面与壁纸是负数
    pub layer: i64,
    /// 归属进程名。用来排除**我们自己**的窗（不排就永远命中自家遮罩）
    pub owner: String,
}

/// 我们自己的归属名（`ice-translator` 与贴图/遮罩的标题都归它）
pub const SELF_OWNER: &str = "ice-translator";

/// 取鼠标那一点上**最靠前的那扇窗**的矩形；`origin` 是这扇遮罩所在屏的全局左上角，
/// 返回值换算回**该屏的本地坐标**（选区与裁剪都在本地坐标里算）。
///
/// 四条判据都在这一个函数里，别处不许再各判一半：
/// - **顺序就是叠放顺序**（表从前往后给），第一个命中的就是屏幕上真看见的那扇；
/// - 负层级是桌面与壁纸，它们铺满整块屏 —— 不排掉的话永远"命中桌面"；
/// - 自家窗要排掉：遮罩本身、还开着的贴图、主窗都在这张表里；
/// - 零面积排掉（菜单栏上那些 0x0 的记账窗会混在里面）。
pub fn topmost_at(entries: &[Entry], origin: Pos2, point: Pos2, self_owner: &str) -> Option<Rect> {
    let global = point + origin.to_vec2();
    let hits = entries.iter().find(|entry| {
        entry.layer >= 0
            && entry.owner != self_owner
            && entry.rect.width() > 0.0
            && entry.rect.height() > 0.0
            && entry.rect.contains(global)
    })?;
    // 全局 → 本地：一扇窗可能只有一半在这块屏上，夹取由调用方按那块屏的矩形做
    let shifted = Rect::from_min_max(
        hits.rect.min - origin.to_vec2(),
        hits.rect.max - origin.to_vec2(),
    );
    (shifted.width() > 0.0 && shifted.height() > 0.0).then_some(shifted)
}

/// 把候选框夹进这块屏（点坐标，本地）。副屏在负方向，所以夹取的两端都得用调用给的矩形，
/// 不许写死 `0..尺寸`（那是"副屏永远看不见"的写法）。
pub fn clamp_to_screen(rect: Rect, screen: Rect) -> Rect {
    let x = |v: f32| v.clamp(screen.min.x, screen.max.x);
    let y = |v: f32| v.clamp(screen.min.y, screen.max.y);
    let (a, b, c, d) = (x(rect.min.x), y(rect.min.y), x(rect.max.x), y(rect.max.y));
    Rect::from_min_max(Pos2::new(a.min(c), b.min(d)), Pos2::new(a.max(c), b.max(d)))
}

/// 读系统的在线窗口表（`CGWindowListCopyWindowInfo`）——**这一步还没接上**。
///
/// 2026-11-09 深夜试过一版：`core-graphics` 那张表交回来的是**无类型**的
/// `CFArray<*const c_void>`，元素要按 `CFDictionary<CFString, CFType>` 再取
/// `kCGWindowBounds` 那个**嵌套字典**里的 X/Y/Width/Height；core-foundation 0.10 的
/// `ItemRef` 不直接给 `get`，得自己按规则包装。第一版在这里连续撞三类签名问题
/// （`let-else` 闭包、turbofish 的 `>>`、`ItemRef::get`），当场放弃 —— 不拿一段
/// 半通的 FFI 提交进你的待批队列，也不留一个坏树。
///
/// 下一步按这个顺序做：先只读**层级与归属**（都是顶层键，一条 CFNumber/CFString）
/// 把候选顺序跑通，再单独加 bounds 的嵌套取值；每一步都用真表跑一次、打印读数核对。
#[cfg(target_os = "macos")]
pub fn read() -> Vec<Entry> {
    Vec::new()
}

#[cfg(not(target_os = "macos"))]
pub fn read() -> Vec<Entry> {
    Vec::new()
}

#[cfg(test)]
mod tests {
    use super::*;
    use egui::pos2;

    fn rect(x0: f32, y0: f32, x1: f32, y1: f32) -> Rect {
        Rect::from_min_max(pos2(x0, y0), pos2(x1, y1))
    }

    fn entry(layer: i64, owner: &str, r: Rect) -> Entry {
        Entry {
            rect: r,
            layer,
            owner: owner.into(),
        }
    }

    /// 取的是**第一条**命中的：表从前往后给，第一条就是看得见那扇
    #[test]
    fn the_first_entry_containing_the_point_wins() {
        let entries = vec![
            entry(0, "Safari", rect(0.0, 0.0, 900.0, 700.0)),
            entry(0, "Finder", rect(0.0, 0.0, 1920.0, 1080.0)),
        ];
        assert_eq!(
            topmost_at(&entries, Pos2::ZERO, pos2(100.0, 100.0), SELF_OWNER),
            Some(rect(0.0, 0.0, 900.0, 700.0))
        );
        // 那一点只在后面那扇里：拿后面的
        assert_eq!(
            topmost_at(&entries, Pos2::ZERO, pos2(1200.0, 900.0), SELF_OWNER),
            Some(rect(0.0, 0.0, 1920.0, 1080.0))
        );
    }

    /// 自家那几扇（遮罩、贴图、主窗）**必须排掉**：不排的话鼠标永远命中遮罩自己，
    /// 而遮罩铺满整屏 —— 结果就是"单击永远截整屏"。
    #[test]
    fn our_own_windows_are_never_the_candidate() {
        let entries = vec![
            entry(25, SELF_OWNER, rect(0.0, 0.0, 1920.0, 1080.0)),
            entry(3, SELF_OWNER, rect(40.0, 40.0, 400.0, 300.0)),
            entry(0, "Xcode", rect(0.0, 0.0, 1200.0, 900.0)),
        ];
        assert_eq!(
            topmost_at(&entries, Pos2::ZERO, pos2(60.0, 60.0), SELF_OWNER),
            Some(rect(0.0, 0.0, 1200.0, 900.0))
        );
        assert_eq!(
            topmost_at(&entries[..2], Pos2::ZERO, pos2(60.0, 60.0), SELF_OWNER),
            None,
            "只剩自家窗时宁可不认，也不许把自家遮罩当目标"
        );
    }

    /// 桌面与壁纸是负层级且铺满整块屏；0x0 的记账窗也不能算。
    #[test]
    fn the_desktop_and_zero_sized_entries_are_skipped() {
        let entries = vec![
            entry(
                -2_147_483_623,
                "Dynamic Wallpaper",
                rect(0.0, 0.0, 1920.0, 1080.0),
            ),
            entry(0, "SomeAgent", rect(500.0, 500.0, 500.0, 560.0)),
        ];
        assert_eq!(
            topmost_at(&entries, Pos2::ZERO, pos2(600.0, 600.0), SELF_OWNER),
            None
        );
    }

    /// 副屏在**负方向**（本机实测左置那块在 `(-1920,-120)`）：候选框要换算成那块屏的
    /// 本地坐标，负偏移不许被当成"没有命中"。
    #[test]
    fn a_secondary_screen_at_negative_coordinates_is_converted_back_to_local() {
        let origin = pos2(-1920.0, -120.0);
        let entries = vec![entry(0, "IDEA", rect(-1900.0, -100.0, -1200.0, 500.0))];
        assert_eq!(
            topmost_at(&entries, origin, pos2(20.0, 20.0), SELF_OWNER),
            Some(rect(20.0, 20.0, 720.0, 620.0))
        );
    }

    /// 夹取两端都用调用给的屏矩形：写死 `0..尺寸` 在副屏上会把框整个推没。
    #[test]
    fn clamping_uses_the_given_screen_not_a_zero_origin_assumption() {
        let screen = rect(-1920.0, -120.0, 0.0, 1080.0);
        assert_eq!(
            clamp_to_screen(rect(-2100.0, -400.0, 900.0, 4000.0), screen),
            rect(-1920.0, -120.0, 0.0, 1080.0)
        );
        assert_eq!(
            clamp_to_screen(rect(-1500.0, 0.0, -1000.0, 300.0), screen).min,
            pos2(-1500.0, 0.0)
        );
    }
}
