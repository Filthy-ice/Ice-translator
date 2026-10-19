use ice_core::document::OcrDocument;
use image::RgbaImage;

use crate::error::{VisionError, VisionResult};

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CompositeOptions {
    /// 回填空间不足时自动降为覆盖层（FR-2.5 / FR-2.7）
    pub fallback_to_overlay: bool,
    /// 字号自适应下限，相对原字号的比例
    pub min_font_scale: f32,
    /// 文字块四周留白
    pub padding: f32,
    /// 背景色估计采用的边缘像素比例
    pub border_sample_ratio: f32,
}

impl Default for CompositeOptions {
    fn default() -> Self {
        Self {
            fallback_to_overlay: true,
            min_font_scale: 0.55,
            padding: 2.0,
            border_sample_ratio: 0.12,
        }
    }
}

#[derive(Debug, Clone)]
pub enum CompositeOutput {
    /// 擦除原文并回填译文（FR-2.4）
    Replaced {
        image: RgbaImage,
        replaced_blocks: usize,
    },
    /// 覆盖层模式（FR-2.5）
    Overlay {
        image: RgbaImage,
        overlaid_blocks: usize,
    },
}

impl CompositeOutput {
    pub fn into_image(self) -> RgbaImage {
        match self {
            CompositeOutput::Replaced { image, .. } | CompositeOutput::Overlay { image, .. } => {
                image
            }
        }
    }

    pub const fn mode_label(&self) -> &'static str {
        match self {
            CompositeOutput::Replaced { .. } => "回填",
            CompositeOutput::Overlay { .. } => "覆盖层",
        }
    }
}

/// 把译文画回图片。实现在 P7 完成。
#[derive(Debug, Default, Clone)]
pub struct Renderer {
    pub options: CompositeOptions,
}

impl Renderer {
    pub fn new(options: CompositeOptions) -> Self {
        Self { options }
    }

    pub fn compose(
        &self,
        _source: &RgbaImage,
        _document: &OcrDocument,
    ) -> VisionResult<CompositeOutput> {
        Err(VisionError::NotImplemented("译文回写在 P7 阶段接入"))
    }
}
