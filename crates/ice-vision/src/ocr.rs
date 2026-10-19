use ice_core::document::OcrDocument;
use image::RgbaImage;

use crate::error::{VisionError, VisionResult};

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct OcrProgress {
    pub blocks: usize,
    pub elapsed_ms: u64,
}

/// 文字识别通道。实现在 P6 接入 macOS Vision。
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

    fn recognize(&self, _image: &RgbaImage) -> VisionResult<OcrDocument> {
        Err(VisionError::NotImplemented("Vision 文字识别在 P6 阶段接入"))
    }
}

/// 当前平台上可用的识别引擎；无能力时返回 None，由界面提示降级（NFR-7）。
pub fn default_engine() -> Option<VisionOcr> {
    if crate::native_ocr_supported() {
        Some(VisionOcr)
    } else {
        None
    }
}
