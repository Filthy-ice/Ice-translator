use ice_core::document::{OcrDocument, TextBlock};
use image::RgbaImage;

use crate::error::VisionResult;
// 只有非 mac 那条分支用到具体错误类型，mac 上识别失败由 `vision_sys` 自己带出来。
#[cfg(not(target_os = "macos"))]
use crate::error::VisionError;
use crate::layout::{MergeParams, RowObservation, merge_rows};

/// 边长上限：任一边超过这个数就先等比缩小再识别（REQ-001）。
///
/// 这条不是"性能优化"而是防炸：`Accurate` 档的耗时与像素量强相关，一张 2 万像素的长截图
/// 能把识别拖到用户放弃。缩放后的尺寸就是 `OcrDocument.width/height`，后续坐标换算全按它算。
pub const MAX_EDGE_PX: u32 = 8000;

/// 识别结果列表里低于这条块级置信度的行带一个可见 ⚠（docs/06 IMG-11）。
///
/// Q-006 定的是常量而不是设置项：一个用户调不动、也解释不清的阈值比写死的更糟。
pub const LOW_CONFIDENCE_WARN: f32 = 0.5;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct OcrProgress {
    pub blocks: usize,
    pub elapsed_ms: u64,
}

/// 文字识别通道。
pub trait OcrEngine: Send + Sync {
    fn name(&self) -> &'static str;

    /// 同步阻塞接口：由上层放进 spawn_blocking 执行，避免卡住渲染线程。
    fn recognize(&self, image: &RgbaImage) -> VisionResult<OcrDocument>;

    fn is_supported(&self) -> bool {
        crate::native_ocr_supported()
    }
}

#[derive(Debug, Default, Clone, Copy)]
pub struct VisionOcr;

impl OcrEngine for VisionOcr {
    fn name(&self) -> &'static str {
        "macOS Vision"
    }

    /// S1（`vision_sys`）→ S2 + S3（`layout`）在本函数内串完，中间态不出 crate。
    ///
    /// `OcrDocument.source_label` 这里留空——引擎只认像素，"这张图从哪来"是导入侧
    /// （`import::Admitted.label`）的知识，由界面在落地时填。
    fn recognize(&self, image: &RgbaImage) -> VisionResult<OcrDocument> {
        recognize_image(image)
    }
}

#[cfg(target_os = "macos")]
fn recognize_image(image: &RgbaImage) -> VisionResult<OcrDocument> {
    Ok(recognize_with_rows(image)?.1)
}

/// 与 `VisionOcr::recognize` 同一条链，但把 S1 的行级观察与这一段用的时间一起交出来。
///
/// 为什么要单独开这一个口：FR-2.3 的"拆分块"要按行回溯，界面需要知道某个块由哪几行、
/// 每行的原文与本帧尺寸下的像素框是什么——而 `OcrDocument` 里只有合并后的块。
/// `OcrEngine::recognize` 的签名不许动（REQ-002 的边界），所以中间态走这个函数而不是走 trait。
/// S1→S2→S3 依然只在本 crate 内串联，`Retained` 依然出不了 `vision_sys`。
///
/// `OcrProgress` 在这里出现而不是在界面层计时：界面层拿不到"什么时候进的 Vision"，
/// 它自己计时测到的是整段任务（含读盘解码），那个数拿来判 FR-2.2 的耗时口径会偏大。
#[cfg(target_os = "macos")]
pub fn recognize_with_rows(
    image: &RgbaImage,
) -> VisionResult<(Vec<RowObservation>, OcrDocument, OcrProgress)> {
    let started = std::time::Instant::now();
    let rows = crate::vision_sys::recognize_rows(image)?;
    let drafts = merge_rows(&rows, &MergeParams::DEFAULT, image.width(), image.height());
    let blocks: Vec<TextBlock> = drafts
        .into_iter()
        .enumerate()
        .map(|(index, draft)| draft.into_block(index as u32))
        .collect();
    let elapsed_ms = started.elapsed().as_millis() as u64;
    // 只有这五个字段进日志（NFR-3 在图片侧的口径）：识别出的文本一个字都不落盘。
    tracing::debug!(
        rows = rows.len(),
        blocks = blocks.len(),
        elapsed_ms,
        width = image.width(),
        height = image.height(),
        "本地识别完成"
    );
    let mut document = OcrDocument::empty(image.width(), image.height());
    document.blocks = blocks;
    let progress = OcrProgress {
        blocks: document.blocks.len(),
        elapsed_ms,
    };
    Ok((rows, document, progress))
}

/// 非 macOS 连行级观察都不可能产出：与 `recognize` 同一个降级档（REQ-007）。
#[cfg(not(target_os = "macos"))]
pub fn recognize_with_rows(
    _image: &RgbaImage,
) -> VisionResult<(Vec<RowObservation>, OcrDocument, OcrProgress)> {
    Err(VisionError::UnsupportedPlatform)
}

#[cfg(not(target_os = "macos"))]
fn recognize_image(image: &RgbaImage) -> VisionResult<OcrDocument> {
    let _ = image;
    Err(VisionError::UnsupportedPlatform)
}

/// 当前平台上可用的识别引擎；无能力时返回 None，由界面提示降级（NFR-7）。
pub fn default_engine() -> Option<VisionOcr> {
    if crate::native_ocr_supported() {
        Some(VisionOcr)
    } else {
        None
    }
}

#[cfg(all(test, target_os = "macos"))]
mod tests {
    use super::*;
    use crate::error::VisionError;

    /// 真机冒烟：一张纯色空白图走完整条 CGImage → handler → perform 管道。
    ///
    /// 断言的不是识别质量，而是**管道没有以框架错误失败**——允许的结果只有两种：
    /// `Ok`（Vision 从噪声里读出了东西）或 `NoTextFound`（它没读出来，这是正常档）。
    /// 出现 `Recognition`/`Composite` 之类就说明 CGImage 构造或 handler 初始化真的坏了。
    /// 整模块 cfg 在 macOS 下：非 mac 目标根本没有 `vision_sys`。
    #[test]
    fn a_blank_image_travels_the_whole_pipeline_without_a_framework_error() {
        let image = RgbaImage::from_pixel(240, 120, image::Rgba([255, 255, 255, 255]));
        match VisionOcr.recognize(&image) {
            Ok(document) => {
                assert_eq!((document.width, document.height), (240, 120));
                for block in &document.blocks {
                    assert!(!block.source_rows.is_empty());
                }
            }
            Err(VisionError::NoTextFound) => {}
            Err(other) => panic!("识别管道以框架错误失败：{other:?}"),
        }
    }

    /// 文档尺寸用的是**入场后的尺寸**，与 `flip_rect` 的基准同源。
    #[test]
    fn the_document_size_is_the_admitted_size() {
        let image = RgbaImage::from_pixel(300, 150, image::Rgba([255, 255, 255, 255]));
        if let Ok(document) = VisionOcr.recognize(&image) {
            assert_eq!((document.width, document.height), (300, 150));
        }
    }

    /// 交回来的行必须真能对上块里的 `source_rows`：界面的"拆分回原行"就靠这层对应关系，
    /// 行号越界等于拆分时 panic 或静默丢字。
    #[test]
    fn the_rows_handed_back_are_the_ones_the_blocks_were_built_from() {
        let image = RgbaImage::from_pixel(320, 160, image::Rgba([255, 255, 255, 255]));
        let Ok((rows, document, progress)) = recognize_with_rows(&image) else {
            return; // 空白图认不出字是正常档（NoTextFound），不是这条断言的对象
        };
        // I 区那句"完成 N 块"读的是 `progress.blocks`，H 区列表读的是 `document.blocks`：
        // 两个数不同源就会当着我的面自相矛盾。
        assert_eq!(progress.blocks, document.blocks.len());
        let used: usize = document.blocks.iter().map(|b| b.source_rows.len()).sum();
        // 允许个别行没能进任何块（被合并判据挡住），但绝不能凭空多出行来
        assert!(
            used <= rows.len(),
            "块里引用了 {used} 行，可总共只交回来 {} 行",
            rows.len()
        );
        for block in &document.blocks {
            for &row in &block.source_rows {
                assert!(
                    (row as usize) < rows.len(),
                    "行号 {row} 越界，总共 {} 行",
                    rows.len()
                );
            }
        }
    }
}
