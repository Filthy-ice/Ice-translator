//! Dock 磁贴上那枚应用图标（2026-11-05 缺陷 **D-27**）。
//!
//! 图标本体只有一个出处：`crate::app_icon`（那张 PNG 解出来、按苹果的图标网格留好白、
//! 降采样到磁贴够用的尺寸）。启动时 eframe 拿它调了一次 `setApplicationIconImage`，
//! 之后**没有任何人再调**。
//!
//! 而"关窗收进托盘"走的是把激活策略切成 `.accessory`（磁贴随之消失），唤回时切回 `.regular`
//! —— AppKit 在这一步**重建**磁贴，取的是本进程的 bundle 图标。我们跑的是没打包的裸二进制，
//! bundle 那一层没有图标，于是用户看到的是系统那张 "exec" 黑底绿字占位图（用户原话：
//! "软件刚重启是正常的，任务栏图标也正常，点击界面的关闭后，再点击托盘图标，界面打开后
//! 任务栏(程序坞)图标还是一个'exec'绿字黑底的图片"）。
//!
//! 所以**唤回那一次必须自己再交一次同一枚图标**。这里不重新解码、不重新缩放、不改留白比例：
//! 只把 `app_icon` 那份 RGBA 编成 `NSImage::initWithData:` 认的 PNG 字节，
//! 与 `system::tray` 那枚菜单栏图标同一条路子（那条也是"裸像素不认，只认编码数据"）。
//!
//! 非 macOS 一侧没有"运行中换 Dock 图标"这件事（任务栏图标走 `viewport.icon`，创建期一次性），
//! 因此这里恒返回 `false`，调用方据此什么都不做——**不许**为了"看起来有兜底"在别处再写一条
//! 平台分支。

/// 该不该在切激活策略之后重下图标。单独拎出来，是因为这一段里只有它是无头可断言的：
/// 真下图标要主线程上活着的 `NSApplication`，测试里没有那一样。
///
/// 判据只有一句：**切回 `.regular` 且真的切成了**才重下。收起（`.accessory`）那一次不重下——
/// 磁贴正要消失，给它刷一次内容既没用也没意义；而"没切成"更不许装作下过。
pub(crate) const fn wants_reinstall(regular: bool, switched: bool) -> bool {
    regular && switched
}

/// 把 `crate::app_icon` 那枚图标再下一次给 Dock 磁贴。返回是否真的下到了原生层。
///
/// 每一步都可能失败（拿不到主线程标记、图标解不出、编码失败、`NSImage` 造不出来），
/// 失败只返回 `false` 由调用方记一句——**不 panic**：这一刻是用户点了托盘要唤回界面，
/// 图标没换上不该让整个唤回动作崩掉。
#[cfg(target_os = "macos")]
pub fn apply() -> bool {
    use objc2::AnyThread;
    use objc2::MainThreadMarker;
    use objc2_app_kit::{NSApplication, NSImage};
    use objc2_foundation::NSData;

    let Some(mtm) = MainThreadMarker::new() else {
        return false;
    };
    let Some(tile) = crate::app_icon() else {
        return false;
    };
    let Ok(png) = png_bytes(&tile) else {
        return false;
    };
    let data = NSData::with_bytes(&png);
    let Some(image) = NSImage::initWithData(NSImage::alloc(), &data) else {
        return false;
    };
    let app = NSApplication::sharedApplication(mtm);
    // 这个 setter 之所以是 `unsafe`，文档给的唯一理由是形参 `None` 在某些系统上不被接受；
    // 这里交进去的是刚造出来的一张图，恒非 `None`。
    unsafe { app.setApplicationIconImage(Some(&image)) };
    true
}

/// RGBA 那份磁贴 → PNG 字节：`NSImage::initWithData:` 认编码数据，不认裸像素。
#[cfg(target_os = "macos")]
fn png_bytes(tile: &egui::IconData) -> image::ImageResult<Vec<u8>> {
    use image::{ExtendedColorType, ImageEncoder};
    let mut buffer = Vec::new();
    image::codecs::png::PngEncoder::new(&mut buffer).write_image(
        &tile.rgba,
        tile.width,
        tile.height,
        ExtendedColorType::Rgba8,
    )?;
    Ok(buffer)
}

/// 非 macOS：没有这件事，恒 `false`。
#[cfg(not(target_os = "macos"))]
pub fn apply() -> bool {
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 四种组合各判一次。这条判据不许漂成"两次都下"或"一次都不下"：前者在磁贴正要消失时
    /// 白刷一次，后者就是 D-27 现在的样子。
    #[test]
    fn the_icon_is_rehanded_only_when_the_policy_comes_back_to_regular() {
        assert!(
            wants_reinstall(true, true),
            "唤回成功却不重下图标——用户就看见系统那张 exec 占位图（D-27 原样复发）"
        );
        assert!(
            !wants_reinstall(false, true),
            "收起那一次不该重下：磁贴正要消失"
        );
        assert!(
            !wants_reinstall(true, false),
            "策略没切成时不该装作下成功了"
        );
        assert!(!wants_reinstall(false, false), "两头都没成，什么都不该做");
    }
}
