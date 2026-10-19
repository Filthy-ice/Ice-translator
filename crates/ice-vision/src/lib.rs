//! Ice Translator 视觉层：图片 OCR 与译文回填。
//!
//! 图片处理必须完全在本机完成（NFR-3），本 crate 不发起任何网络请求。

pub mod composite;
pub mod error;
pub mod ocr;

pub use composite::{CompositeOptions, CompositeOutput, Renderer};
pub use error::{VisionError, VisionResult};
pub use ocr::{OcrEngine, OcrProgress};

/// 当前平台是否具备原生 OCR 能力（NFR-7 的降级依据）
pub const fn native_ocr_supported() -> bool {
    cfg!(target_os = "macos")
}
