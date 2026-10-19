use serde::{Deserialize, Serialize};

/// 图片像素坐标原点：左上角，y 向下。视觉层负责把系统的左下原点转换过来。
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct PixelPoint {
    pub x: f32,
    pub y: f32,
}

impl PixelPoint {
    pub const fn new(x: f32, y: f32) -> Self {
        Self { x, y }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct PixelRect {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

impl PixelRect {
    pub const fn new(x: f32, y: f32, w: f32, h: f32) -> Self {
        Self { x, y, w, h }
    }

    pub const fn right(self) -> f32 {
        self.x + self.w
    }

    pub const fn bottom(self) -> f32 {
        self.y + self.h
    }

    pub const fn center_x(self) -> f32 {
        self.x + self.w / 2.0
    }

    pub const fn area(self) -> f32 {
        (self.w.max(0.0)) * (self.h.max(0.0))
    }

    pub fn intersects_horizontal(self, other: Self) -> bool {
        self.x < other.right() && other.x < self.right()
    }

    /// 垂直方向上的间隙（用于判断两块是否属于同一段落）
    pub fn vertical_gap(self, other: Self) -> f32 {
        if self.bottom() <= other.y {
            other.y - self.bottom()
        } else if other.bottom() <= self.y {
            self.y - other.bottom()
        } else {
            -1.0
        }
    }

    /// 返回包含两个矩形的最小外接矩形
    pub fn union(self, other: Self) -> Self {
        let x = self.x.min(other.x);
        let y = self.y.min(other.y);
        Self::new(
            x,
            y,
            self.right().max(other.right()) - x,
            self.bottom().max(other.bottom()) - y,
        )
    }

    pub fn scaled(self, factor: f32) -> Self {
        Self::new(self.x, self.y, self.w * factor, self.h * factor)
    }
}

/// OCR 出的一个文字块，可带翻译结果。FR-2.2 / FR-2.3
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TextBlock {
    pub id: u32,
    pub text: String,
    pub bbox: PixelRect,
    /// 旋转/倾斜文字的原始四点轮廓；None 表示水平
    pub polygon: Option<[PixelPoint; 4]>,
    pub confidence: f32,
    pub translation: Option<String>,
    /// 回填时因空间不足而降级为覆盖层（FR-2.7）
    pub degraded: bool,
    /// 用户在识别结果列表里勾掉后为 false，不参与翻译与合成
    pub enabled: bool,
}

impl TextBlock {
    pub fn new(id: u32, text: impl Into<String>, bbox: PixelRect, confidence: f32) -> Self {
        Self {
            id,
            text: text.into(),
            bbox,
            polygon: None,
            confidence,
            translation: None,
            degraded: false,
            enabled: true,
        }
    }

    /// 估计字号：块高的 0.75 倍，实测比直接用框高更接近视觉字号
    pub fn estimated_font_size(&self) -> f32 {
        (self.bbox.h * 0.75).max(8.0)
    }
}

/// 一张图的 OCR 结果集合。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct OcrDocument {
    pub width: u32,
    pub height: u32,
    pub blocks: Vec<TextBlock>,
    pub source_label: String,
}

impl OcrDocument {
    pub fn empty(width: u32, height: u32) -> Self {
        Self {
            width,
            height,
            blocks: Vec::new(),
            source_label: String::new(),
        }
    }

    pub fn enabled_blocks(&self) -> impl Iterator<Item = &TextBlock> {
        self.blocks
            .iter()
            .filter(|b| b.enabled && !b.text.trim().is_empty())
    }

    pub fn merged_text(&self) -> String {
        self.enabled_blocks()
            .map(|b| b.text.as_str())
            .collect::<Vec<_>>()
            .join("\n")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rect_union_covers_both() {
        let a = PixelRect::new(10.0, 10.0, 20.0, 10.0);
        let b = PixelRect::new(50.0, 40.0, 30.0, 15.0);
        let u = a.union(b);
        assert_eq!((u.x, u.y, u.right(), u.bottom()), (10.0, 10.0, 80.0, 55.0));
    }

    #[test]
    fn vertical_gap_is_negative_when_overlapping_rows() {
        let a = PixelRect::new(0.0, 0.0, 100.0, 20.0);
        let b = PixelRect::new(10.0, 10.0, 100.0, 20.0);
        assert_eq!(a.vertical_gap(b), -1.0);
        let c = PixelRect::new(10.0, 30.0, 100.0, 20.0);
        assert_eq!(a.vertical_gap(c), 10.0);
    }

    #[test]
    fn font_size_never_below_floor() {
        let tiny = TextBlock::new(0, "x", PixelRect::new(0.0, 0.0, 4.0, 4.0), 0.9);
        assert_eq!(tiny.estimated_font_size(), 8.0);
    }

    #[test]
    fn merged_text_skips_disabled_and_blank_blocks() {
        let mut doc = OcrDocument::empty(100, 100);
        doc.blocks.push(TextBlock::new(
            0,
            "A",
            PixelRect::new(0.0, 0.0, 10.0, 10.0),
            0.9,
        ));
        let mut blank = TextBlock::new(1, "  ", PixelRect::new(0.0, 0.0, 10.0, 10.0), 0.5);
        blank.enabled = false;
        doc.blocks.push(blank);
        doc.blocks.push(TextBlock::new(
            2,
            "B",
            PixelRect::new(0.0, 0.0, 10.0, 10.0),
            0.9,
        ));
        assert_eq!(doc.merged_text(), "A\nB");
    }
}
