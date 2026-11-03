//! S0 入场：拖拽 / 粘贴 / 面板 / 手输路径四个入口汇到同一条解码链（CHG-005 REQ-001）。
//!
//! 这里只负责"把原料变成一张能用的 `RgbaImage`"，不认识 Vision、不认识界面：
//! 解码、缩放、贴标签三件事，四个入口拿到的错误类型与文案完全一致。

use std::path::Path;

use image::{DynamicImage, RgbaImage};

use crate::error::{VisionError, VisionResult};
use crate::ocr::MAX_EDGE_PX;

/// 档一：原料根本没到手。路径不存在、是目录、没权限都归这一档。
pub const NOT_FOUND_TEXT: &str = "找不到文件：路径不存在、是个目录，或没有读取权限";

/// 档二：到手了但读不懂。workspace 那份 `image` 只开了 png 与 jpeg 两个 feature（Q-005 选 (a)），
/// 剪贴板那条走 `admit_rgba`（arboard 已经把 TIFF 解成 RGBA），所以这里不承诺别的格式。
pub const DECODE_TEXT: &str = "图片格式不支持或已损坏（本版本支持 PNG / JPEG，以及剪贴板里的图片）";

/// 一张进入识别流程的图片：像素 + 是否被缩放过 + 尺寸 + 来源标签。
#[derive(Debug, Clone)]
pub struct Admitted {
    pub image: RgbaImage,
    /// `true` 表示原图任一边超 `MAX_EDGE_PX`、已等比缩小。**必须让用户看得见**（Q-007 不静默）。
    pub scaled: bool,
    /// 缩放后的宽高，也就是后续一切坐标换算的基准（REQ-003）。
    pub width: u32,
    pub height: u32,
    /// 文件名之类，只给界面显示用；识别结果本身不依赖它。
    pub label: String,
}

/// 字节 → 图片：面板与拖拽之后、以及任何"手上只有文件内容"的入口。
pub fn admit(bytes: &[u8], label: impl Into<String>) -> VisionResult<Admitted> {
    let decoded =
        image::load_from_memory(bytes).map_err(|_| VisionError::Decode(DECODE_TEXT.to_string()))?;
    Ok(admit_rgba(decoded.into_rgba8(), label))
}

/// 像素缓冲 → 图片：剪贴板入口（`arboard::get_image()` 给的就是 RGBA）。
pub fn admit_rgba(image: RgbaImage, label: impl Into<String>) -> Admitted {
    let oversize = image.width() > MAX_EDGE_PX || image.height() > MAX_EDGE_PX;
    // 唯一的缩放分支：超上限是**缩小后继续**而不是拒绝，所以这里永不返回 `TooLarge`
    //（那一档留给"缩放后仍无法处理"的极端情况）。
    //
    // 用的是 `DynamicImage::thumbnail` 而不是 `imageops::thumbnail`：后者按给定宽高**硬拉伸**，
    // 长图会被压成方图；前者才等比 fitting。
    let image = if oversize {
        DynamicImage::ImageRgba8(image)
            .thumbnail(MAX_EDGE_PX, MAX_EDGE_PX)
            .into_rgba8()
    } else {
        image
    };
    let (width, height) = (image.width(), image.height());
    Admitted {
        image,
        scaled: oversize,
        width,
        height,
        label: label.into(),
    }
}

/// 路径 → 图片：拖拽与手输路径共用。读盘失败只报"找不到文件"那一档，不透出 OS 原文。
pub fn admit_path(path: &Path) -> VisionResult<Admitted> {
    let label = path
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default();
    let bytes = std::fs::read(path).map_err(|_| VisionError::Decode(NOT_FOUND_TEXT.to_string()))?;
    admit(&bytes, label)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    fn png(width: u32, height: u32) -> Vec<u8> {
        let image = DynamicImage::ImageRgba8(RgbaImage::from_pixel(
            width,
            height,
            image::Rgba([240, 240, 240, 255]),
        ));
        let mut buf = Vec::new();
        image
            .write_to(&mut Cursor::new(&mut buf), image::ImageFormat::Png)
            .expect("自绘 PNG 编码");
        buf
    }

    #[test]
    fn bytes_and_rgba_land_on_the_same_shape() {
        let from_bytes = admit(&png(320, 200), "截图.png").expect("字节入场");
        let from_rgba = admit_rgba(
            RgbaImage::from_pixel(320, 200, image::Rgba([240, 240, 240, 255])),
            "剪贴板",
        );
        assert_eq!(
            (from_bytes.width, from_bytes.height, from_bytes.scaled),
            (from_rgba.width, from_rgba.height, from_rgba.scaled)
        );
        assert_eq!((from_bytes.width, from_bytes.height), (320, 200));
        assert!(!from_bytes.scaled, "没超上限不该标已缩放");
        assert_eq!(from_bytes.label, "截图.png");
    }

    /// 超过上限必须缩，而且**缩放后的尺寸就是文档尺寸**（REQ-003 的换算基准由这条锁住）。
    #[test]
    fn an_oversize_image_is_scaled_and_the_reported_size_is_the_scaled_one() {
        let admitted = admit(&png(8100, 100), "长图").expect("超宽图入场");
        assert!(admitted.scaled, "超上限必须标已缩放，不能静默");
        assert!(admitted.width <= MAX_EDGE_PX && admitted.height <= MAX_EDGE_PX);
        assert_eq!(admitted.width, admitted.image.width());
        assert_eq!(admitted.height, admitted.image.height());
        // 等比：8100×100 → 长边收到 8000，短边同步缩到约 99。
        let ratio = admitted.width as f32 / admitted.height as f32;
        assert!(
            ratio > 80.0 && ratio < 83.0,
            "等比被破坏，实际宽高比 {ratio}"
        );

        // 另一头：竖长图同样只收长边，不收成方图。
        let tall = admit(&png(200, 9000), "长截图").expect("超高图入场");
        assert!(tall.scaled);
        assert!(tall.height <= MAX_EDGE_PX && tall.width <= MAX_EDGE_PX);
        let tall_ratio = tall.width as f32 / tall.height as f32;
        assert!(tall_ratio < 0.03, "竖图被拉成了方图，宽高比 {tall_ratio}");
    }

    /// 两个入口拿到同一个错误类型，但文案分两档：找不到文件 ≠ 解不开。
    #[test]
    fn the_two_failure_kinds_share_the_error_type_but_not_the_text() {
        let missing = admit_path(Path::new("/definitely/not/here.png")).unwrap_err();
        let broken = admit(&[0u8, 1, 2, 3], "坏字节").unwrap_err();
        assert!(matches!(missing, VisionError::Decode(_)));
        assert!(matches!(broken, VisionError::Decode(_)));
        assert_ne!(missing.to_string(), broken.to_string());
        assert!(missing.to_string().contains("找不到文件"));
        assert!(broken.to_string().contains("PNG"));
    }

    /// 目录也归"找不到文件"这一档——它是原料没到手，不是格式问题。
    #[test]
    fn a_directory_reads_as_not_found() {
        let dir = std::env::temp_dir();
        let error = admit_path(&dir).unwrap_err();
        assert!(error.to_string().contains("找不到文件"), "{error}");
    }
}
