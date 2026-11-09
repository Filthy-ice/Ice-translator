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
    /// 该块由识别结果的第几行合并而来（FR-2.3 的"拆分回原行"靠它）。
    /// 空表示来源行未知：手动合并、外部构造的块都可能是空。
    ///
    /// 运行态字段，不进配置 schema：`default` 让改动前序列化的数据仍可反序列化。
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub source_rows: Vec<u32>,
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
            source_rows: Vec::new(),
        }
    }

    /// 带来源行的构造器：视觉层合并出来的块走这条（FR-2.3 拆分回原行的信息入口）。
    pub fn with_rows(
        id: u32,
        text: impl Into<String>,
        bbox: PixelRect,
        confidence: f32,
        source_rows: Vec<u32>,
    ) -> Self {
        Self {
            source_rows,
            ..Self::new(id, text, bbox, confidence)
        }
    }

    /// 估计字号：块高的 0.75 倍，实测比直接用框高更接近视觉字号
    pub fn estimated_font_size(&self) -> f32 {
        (self.bbox.h * 0.75).max(8.0)
    }

    /// 这块有没有**可画的**译文：没翻、翻失败（`None`）与翻出来是空白都不算。
    ///
    /// 「能不能生成成品」的前置条件与版式计划里的 `Skip` 判据是同一条，所以收在这里一处：
    /// 两处各写各的，就会出现"界面认为可以画，画出来那块却是空的"。
    pub fn translated_text(&self) -> Option<&str> {
        self.translation
            .as_deref()
            .map(str::trim)
            .filter(|text| !text.is_empty())
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

    /// 把选中的几块拼成一段文字，**按阅读顺序**（先比上缘、同一行再比左缘）。
    ///
    /// 返回 `None` 而不是空串：贴图那边要拿"有没有选中"当分派依据（复制字还是复制图），
    /// 一个空串会让"没选中"和"选中了一块空文字"这两种状态长得一模一样。
    /// 只认块 id 集合里**确实存在且非空白**的块——识别没出的块选不中，选中的块被老化
    /// 清掉文档也不会交出一段来历不明的文字。
    pub fn joined_text(&self, ids: &[u32]) -> Option<String> {
        let mut picked: Vec<&TextBlock> = ids
            .iter()
            .filter_map(|id| self.blocks.iter().find(|b| b.id == *id))
            // 与贴图那一侧的命中判定同一条口径：勾掉的块与空白块**都选不中**，
            // 所以也不该被这段拼接悄悄带出去（两处各判一半就是"复制出来的多了没选的那行"）
            .filter(|b| b.enabled && !b.text.trim().is_empty())
            .collect();
        if picked.is_empty() {
            return None;
        }
        picked.sort_by(|a, b| {
            a.bbox
                .y
                .partial_cmp(&b.bbox.y)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then(
                    a.bbox
                        .x
                        .partial_cmp(&b.bbox.x)
                        .unwrap_or(std::cmp::Ordering::Equal),
                )
                .then(a.id.cmp(&b.id))
        });
        picked.dedup_by(|a, b| a.id == b.id);
        Some(
            picked
                .into_iter()
                .map(|b| b.text.trim().to_string())
                .collect::<Vec<_>>()
                .join("\n"),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 一块可识别的文字（贴图文字层的用例都从这里造，别处不重复这套字段）
    fn block(id: u32, text: &str, x: f32, y: f32, w: f32, h: f32) -> TextBlock {
        TextBlock {
            id,
            text: text.to_string(),
            bbox: PixelRect::new(x, y, w, h),
            polygon: None,
            confidence: 0.9,
            translation: None,
            degraded: false,
            enabled: true,
            source_rows: vec![],
        }
    }

    fn doc(blocks: Vec<TextBlock>) -> OcrDocument {
        OcrDocument {
            width: 1000,
            height: 800,
            blocks,
            source_label: "probe".to_string(),
        }
    }

    /// 拖选出来的文字按**阅读顺序**拼：先比上缘、同一行再比左缘，与传入的块 id 次序无关
    /// （用户是从下往上倒着拖的，粘出去的字不该倒着读）。
    #[test]
    fn joined_text_follows_reading_order_not_click_order() {
        let document = doc(vec![
            block(1, "第一行", 0.0, 0.0, 200.0, 20.0),
            block(2, "第二行", 0.0, 30.0, 200.0, 20.0),
            block(3, "第三行", 0.0, 60.0, 200.0, 20.0),
        ]);
        assert_eq!(
            document.joined_text(&[3, 1, 2]).as_deref(),
            Some("第一行\n第二行\n第三行")
        );
        // 同一行里两块：左边的先出来
        let same_row = doc(vec![
            block(1, "右块", 300.0, 0.0, 100.0, 20.0),
            block(2, "左块", 10.0, 0.0, 100.0, 20.0),
        ]);
        assert_eq!(same_row.joined_text(&[1, 2]).as_deref(), Some("左块\n右块"));
    }

    /// **空选择交出 `None`，不交出空串**：贴图那边拿"有没有值"分派复制字还是复制图，
    /// 一个空串会让"没选中"与"选中了一块空文字"长得一模一样。
    #[test]
    fn an_empty_selection_is_none_rather_than_an_empty_string() {
        let mut off = block(3, "被勾掉的", 0.0, 80.0, 50.0, 10.0);
        off.enabled = false;
        let document = doc(vec![
            block(1, "有字", 0.0, 0.0, 50.0, 10.0),
            block(2, "   ", 0.0, 40.0, 50.0, 10.0),
            off,
        ]);
        assert_eq!(document.joined_text(&[]), None);
        // 只选中空白块 / 被勾掉的块：仍然是"没选中"
        assert_eq!(document.joined_text(&[2]), None);
        assert_eq!(document.joined_text(&[3]), None);
        // id 不在文档里（比如文档被新一轮识别换掉了）也不交出别处的文字
        assert_eq!(document.joined_text(&[99]), None);
        assert_eq!(document.joined_text(&[1, 1]).as_deref(), Some("有字"));
    }

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
