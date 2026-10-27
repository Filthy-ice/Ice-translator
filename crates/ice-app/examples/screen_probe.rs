//! 一次性屏幕几何探针（不属于正式产物）：打印 `NSScreen.screens` 里每一块屏的
//! `frame` / `visibleFrame`（AppKit 左下原点），以及按 `system::screen::flip` 换算后的
//! egui/winit 坐标。双屏停靠的缺陷（D-8 与后续的接缝问题）要靠真实摆放定位，
//! 单测里的屏幕是我猜的。
//!
//! ```sh
//! cargo run -p ice-app --example screen_probe
//! ```

use objc2::MainThreadMarker;
use objc2_app_kit::NSScreen;

fn main() {
    let Some(mtm) = MainThreadMarker::new() else {
        eprintln!("拿不到主线程标记，探针无法读 NSScreen");
        return;
    };
    let screens = NSScreen::screens(mtm);
    let main_h = if screens.count() > 0 {
        screens.objectAtIndex(0).frame().size.height
    } else {
        0.0
    };
    println!(
        "屏幕数 = {}，主屏（index 0）高度 = {main_h}",
        screens.count()
    );
    // 桌面并集，与 `system::screen::union` 同一条公式：停靠只认这两条外缘
    let mut union = (f64::MAX, f64::MAX, f64::MIN, f64::MIN);
    for (i, screen) in screens.iter().enumerate() {
        let frame = screen.frame();
        let visible = screen.visibleFrame();
        let name = screen.localizedName();
        println!(
            "[{i}] {name}\n    AppKit  frame    x {:.1}..{:.1}  y {:.1}..{:.1}",
            frame.origin.x,
            frame.origin.x + frame.size.width,
            frame.origin.y,
            frame.origin.y + frame.size.height,
        );
        println!(
            "    AppKit  visible  x {:.1}..{:.1}  y {:.1}..{:.1}",
            visible.origin.x,
            visible.origin.x + visible.size.width,
            visible.origin.y,
            visible.origin.y + visible.size.height,
        );
        // 与 `system::screen::flip` 同一条公式：y_topleft = 主屏高 − 矩形高 − 原点 y
        let top = main_h - frame.size.height - frame.origin.y;
        let v_top = main_h - visible.size.height - visible.origin.y;
        println!(
            "    egui    frame    x {:.1}..{:.1}  y {:.1}..{:.1}",
            frame.origin.x,
            frame.origin.x + frame.size.width,
            top,
            top + frame.size.height,
        );
        println!(
            "    egui    visible  x {:.1}..{:.1}  y {:.1}..{:.1}",
            visible.origin.x,
            visible.origin.x + visible.size.width,
            v_top,
            v_top + visible.size.height,
        );
        union.0 = union.0.min(visible.origin.x);
        union.1 = union.1.min(v_top);
        union.2 = union.2.max(visible.origin.x + visible.size.width);
        union.3 = union.3.max(v_top + visible.size.height);
    }
    println!(
        "桌面并集（停靠外缘）x {:.1}..{:.1}  y {:.1}..{:.1}",
        union.0, union.2, union.1, union.3
    );
}
