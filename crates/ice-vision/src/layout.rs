//! S2 坐标换算与 S3 行合并 —— 纯函数层，不碰系统框架，任何平台都能编译与测试。
//!
//! [`RowObservation`] 是框架与算法之间唯一的接缝：`vision_sys` 把 ObjC 对象拆成它，
//! 本模块从此只认纯 Rust。坐标原点约定见 `ice_core::document::PixelRect`（左上、y 向下），
//! 而 Vision 给的是左下原点的归一化框，翻转由 [`flip_rect`] 负责。

use ice_core::document::{PixelRect, TextBlock};

/// Vision 的归一化框：原点在**左下**，x/y/w/h 均相对图像宽高（正常在 0~1，偶有极小越界）。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct NormalizedRect {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

/// 一行识别结果（框架无关的中间态，只在 `vision_sys` 与本模块之间出现）。
#[derive(Debug, Clone, PartialEq)]
pub struct RowObservation {
    pub text: String,
    pub normalized_box: NormalizedRect,
    /// 第一名候选的置信度（0~1）。
    pub confidence: f32,
    /// 框架返回的行序，用于确定性破平与 [`BlockDraft::source_rows`] 回溯。
    pub row_index: usize,
}

/// 归一化左下框 → 像素左上框，并做保守夹取。
///
/// `y_px = (1 − y − h) × H`，**不是** `H − y×H`：后者漏掉 `− h`，会把整行往上挪一个行高。
/// 越界时收窄宽高而不是挪 origin —— P7 要按这个框擦像素，宁可少擦几像素也不能擦到框外。
/// 宽高基准是入场缩放后的尺寸（`OcrDocument.width/height`），不是原图。
pub fn flip_rect(rect: NormalizedRect, img_w: u32, img_h: u32) -> PixelRect {
    let w = img_w as f32;
    let h = img_h as f32;
    let x = (rect.x * w).clamp(0.0, w);
    let y = ((1.0 - rect.y - rect.h) * h).clamp(0.0, h);
    PixelRect::new(
        x,
        y,
        (rect.w * w).max(0.0).min(w - x),
        (rect.h * h).max(0.0).min(h - y),
    )
}

/// S3 合并参数（docs/04 §2 的原值；本包不预先调参，真机判定属 REQ-008 第二层）。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MergeParams {
    /// ① 左边界对齐容差 = 该系数 × 平均字高。
    pub align_tol_factor: f32,
    /// ② 垂直间隙上限 = 该系数 × 行高。
    pub row_gap_factor: f32,
    /// ③ 水平重叠下限 = 该系数 × 较窄一行的宽。
    pub overlap_min: f32,
    /// ④ 「已填满」判定 = 行宽达到该系数 × 参考宽度即视为段落结尾，不再合并。
    pub fill_ratio_para_break: f32,
    /// 硬上限：单块最多几行。
    pub max_rows: usize,
    /// 硬上限：单块最多几字符（按 `chars()` 计，不是字节）。
    pub max_chars: usize,
}

impl MergeParams {
    /// docs/04 §2 的原值，本包的缺省参数。
    pub const DEFAULT: Self = Self {
        align_tol_factor: 0.6,
        row_gap_factor: 1.3,
        overlap_min: 0.4,
        fill_ratio_para_break: 0.85,
        max_rows: 12,
        max_chars: 1200,
    };
}

impl Default for MergeParams {
    fn default() -> Self {
        Self::DEFAULT
    }
}

/// 合并出来的一个块草稿；`source_rows` 是 REQ-005 拆分回原行的唯一依据。
#[derive(Debug, Clone, PartialEq)]
pub struct BlockDraft {
    pub text: String,
    pub bbox: PixelRect,
    /// 按字符数加权的置信度。
    pub confidence: f32,
    pub source_rows: Vec<usize>,
}

impl BlockDraft {
    /// 落成 `TextBlock`：id 由调用方按顺序给，合并/拆分产生的新 id 不在此管。
    /// 行号在视觉层内部是 `usize`（就是 `Vec` 的下标），进数据结构时收成 `u32`。
    pub fn into_block(self, id: u32) -> TextBlock {
        TextBlock::with_rows(
            id,
            self.text,
            self.bbox,
            self.confidence,
            self.source_rows.into_iter().map(|row| row as u32).collect(),
        )
    }
}

/// 一条行观察换算到像素并打上表格列标记后的内部形态。
#[derive(Debug, Clone)]
struct PlacedRow {
    row_index: usize,
    text: String,
    /// `chars()` 计数，不是字节数——中文一行 20 字节不是 20 字。
    chars: usize,
    confidence: f32,
    rect: PixelRect,
    /// 所在 y 带被判定为表格时的列号；`None` 表示非表格行。
    column: Option<usize>,
}

/// 正在生长的块：判据④要块左边界与参考宽度，两条硬上限要行数与字符数，都在这里备齐。
struct BlockAccum {
    members: Vec<PlacedRow>,
    left: f32,
    max_w: f32,
    chars: usize,
}

impl BlockAccum {
    fn new(row: PlacedRow) -> Self {
        let (left, max_w, chars) = (row.rect.x, row.rect.w, row.chars);
        Self {
            members: vec![row],
            left,
            max_w,
            chars,
        }
    }

    fn push(&mut self, row: PlacedRow) {
        self.left = self.left.min(row.rect.x);
        self.max_w = self.max_w.max(row.rect.w);
        self.chars += row.chars;
        self.members.push(row);
    }

    fn last(&self) -> &PlacedRow {
        self.members.last().expect("块至少有一行")
    }
}

/// S3：行 → 语义段。五条判据全部满足才合并，另加两条硬上限（docs/04 §2）。
///
/// 零 IO、零 unsafe、零平台依赖：`vision_sys` 把框架对象拆成 `RowObservation` 之后，
/// 分组这件事就完全可以在任何机器上跑测试（REQ-004 的验证一条都不需要真机）。
/// 输入顺序不影响结果——内部按"自上而下、自左而右"重排，平局用 `row_index` 决胜。
pub fn merge_rows(
    rows: &[RowObservation],
    params: &MergeParams,
    img_w: u32,
    img_h: u32,
) -> Vec<BlockDraft> {
    if rows.is_empty() {
        return Vec::new();
    }
    let mut placed: Vec<PlacedRow> = rows
        .iter()
        .map(|row| PlacedRow {
            row_index: row.row_index,
            text: row.text.clone(),
            chars: row.text.chars().count(),
            confidence: row.confidence,
            rect: flip_rect(row.normalized_box, img_w, img_h),
            column: None,
        })
        .collect();

    // 确定性：`f32::total_cmp` 先 y 后 x，完全相等用 row_index 破平，不用哈希迭代序。
    placed.sort_by(|a, b| {
        a.rect
            .y
            .total_cmp(&b.rect.y)
            .then(a.rect.x.total_cmp(&b.rect.x))
            .then(a.row_index.cmp(&b.row_index))
    });
    assign_table_columns(&mut placed);

    // 单行块时"块内最大行宽"就是上一行自己，`a.w < 0.85 × a.w` 恒不成立，
    // 判据④会在每个段落开头失效——所以那时退化为全图最大行宽（docs/04 §2 判据④）。
    let global_max_w = placed
        .iter()
        .map(|row| row.rect.w)
        .fold(f32::NEG_INFINITY, f32::max);

    let mut blocks: Vec<BlockDraft> = Vec::new();
    let mut iter = placed.into_iter();
    // 上面已保证 `rows` 非空，这里必然拿得到第一行。
    let mut current = BlockAccum::new(iter.next().expect("rows 非空"));
    for row in iter {
        if can_merge(&current, &row, params, global_max_w) {
            current.push(row);
        } else {
            blocks.push(finish(current));
            current = BlockAccum::new(row);
        }
    }
    blocks.push(finish(current));
    blocks
}

/// 五条判据 + 两条硬上限，全过才允许把 `row` 接进 `acc`。
fn can_merge(acc: &BlockAccum, row: &PlacedRow, params: &MergeParams, global_max_w: f32) -> bool {
    let prev = acc.last();
    if acc.members.len() >= params.max_rows || acc.chars + row.chars > params.max_chars {
        return false;
    }
    // ⑤ 表格防护：表格列与非表格行、以及不同列之间，一律不合并。
    if prev.column != row.column {
        return false;
    }
    let avg_h = (prev.rect.h + row.rect.h) / 2.0;
    // ① 左边界对齐（相邻两行）。
    if (prev.rect.x - row.rect.x).abs() >= params.align_tol_factor * avg_h {
        return false;
    }
    // ② 垂直间隙：`vertical_gap` 给 −1 表示两行上下重叠，属乱序或表格，不靠它合并。
    let gap = prev.rect.vertical_gap(row.rect);
    if gap < 0.0 || gap >= params.row_gap_factor * avg_h {
        return false;
    }
    // ③ 水平投影重叠。
    let overlap = prev.rect.right().min(row.rect.right()) - prev.rect.x.max(row.rect.x);
    if overlap <= params.overlap_min * prev.rect.w.min(row.rect.w) {
        return false;
    }
    // ④ 新段落：相对**块左边界**缩进，且上一行没写满参考宽度。
    let ref_w = if acc.members.len() >= 2 {
        acc.max_w
    } else {
        global_max_w
    };
    let indented = row.rect.x - acc.left > params.align_tol_factor * avg_h;
    let prev_unfilled = prev.rect.w < params.fill_ratio_para_break * ref_w;
    if indented && prev_unfilled {
        return false;
    }
    true
}

/// 把按 y 带切出表格列写回各行。
///
/// 带 = 垂直方向连成一片的若干行（后一行的顶边落在已并入行的底边之下）。带内按 x 排序，
/// 出现"下一行的左边落在当前组右边之外"即另起一列；组数 ≥2 才判表格（docs/04 §2 判据⑤）。
fn assign_table_columns(placed: &mut [PlacedRow]) {
    let mut start = 0usize;
    while start < placed.len() {
        let mut end = start + 1;
        let mut band_bottom = placed[start].rect.bottom();
        while end < placed.len() && placed[end].rect.y < band_bottom {
            band_bottom = band_bottom.max(placed[end].rect.bottom());
            end += 1;
        }
        let mut band: Vec<usize> = (start..end).collect();
        band.sort_by(|&i, &j| {
            placed[i]
                .rect
                .x
                .total_cmp(&placed[j].rect.x)
                .then(placed[i].row_index.cmp(&placed[j].row_index))
        });
        let mut groups: Vec<(usize, usize)> = Vec::new(); // (placed 下标, 列号)
        let mut right = f32::NEG_INFINITY;
        let mut column = 0usize;
        for &index in &band {
            if placed[index].rect.x > right {
                column += 1;
                right = placed[index].rect.right();
            } else {
                right = right.max(placed[index].rect.right());
            }
            groups.push((index, column - 1));
        }
        let is_table = column >= 2;
        for (index, group) in groups {
            placed[index].column = is_table.then_some(group);
        }
        start = end;
    }
}

/// 收块：文本按行序换行拼接、框取 union、置信度按字符数加权、行号原样带出。
fn finish(acc: BlockAccum) -> BlockDraft {
    let texts: Vec<String> = acc.members.iter().map(|row| row.text.clone()).collect();
    let mut bbox = acc.members[0].rect;
    let mut weighted = 0.0_f32;
    for row in &acc.members {
        bbox = bbox.union(row.rect);
        weighted += row.confidence * row.chars as f32;
    }
    // 全空文本时退化成简单平均，避免 0/0 出一个 NaN。
    let confidence = if acc.chars > 0 {
        weighted / acc.chars as f32
    } else {
        acc.members.iter().map(|row| row.confidence).sum::<f32>() / acc.members.len() as f32
    };
    BlockDraft {
        text: texts.join("\n"),
        bbox,
        confidence,
        source_rows: acc.members.iter().map(|row| row.row_index).collect(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const W: u32 = 100;
    const H: u32 = 200;

    fn norm(x: f32, y: f32, w: f32, h: f32) -> NormalizedRect {
        NormalizedRect { x, y, w, h }
    }

    /// 像素级判等：`0.9f32` 本身就不是 0.9，归一化算出 4e-6 的残差属预期。
    /// 这里要锁的是公式形状（差一个 `− h` 就是 20px 级别），不是十进制精确值，
    /// 所以容差取 1e-3 像素——远小于任何一次擦像素/画字的粒度。
    fn approx(actual: f32, expected: f32) {
        assert!(
            (actual - expected).abs() < 1e-3,
            "期望 {expected}，实际 {actual}"
        );
    }

    #[test]
    fn a_box_at_normalized_zero_lands_on_the_bottom_row() {
        let px = flip_rect(norm(0.0, 0.0, 1.0, 0.1), W, H);
        approx(px.x, 0.0);
        approx(px.w, 100.0);
        approx(px.y, 180.0);
        approx(px.bottom(), 200.0);
    }

    #[test]
    fn a_box_at_normalized_one_lands_on_the_top_row() {
        let px = flip_rect(norm(0.0, 0.9, 1.0, 0.1), W, H);
        approx(px.y, 0.0);
        approx(px.bottom(), 20.0);
    }

    /// 典型错写法 `H − y×H` 必须被这条区分出来：只要 `h > 0`，两种算法结果不同。
    #[test]
    fn flipping_subtracts_the_box_height_not_just_the_origin() {
        let px = flip_rect(norm(0.0, 0.2, 1.0, 0.5), W, H);
        approx(px.y, 60.0);
        let wrong = H as f32 - 0.2 * H as f32;
        assert!(
            (px.y - wrong).abs() > 1.0,
            "错写法 {wrong} 与正确结果 {} 没拉开差距",
            px.y
        );
    }

    #[test]
    fn x_and_w_scale_by_image_width_independently_of_height() {
        let px = flip_rect(norm(0.25, 0.4, 0.5, 0.2), W, H);
        approx(px.x, 25.0);
        approx(px.w, 50.0);
        // y 轴用 H=200：(1 − 0.4 − 0.2) × 200 = 80，宽 0.2 × 200 = 40。
        approx(px.y, 80.0);
        approx(px.h, 40.0);
    }

    #[test]
    fn the_pixel_size_follows_the_size_given_not_any_other_size() {
        let small = flip_rect(norm(0.5, 0.5, 0.25, 0.25), 100, 100);
        let large = flip_rect(norm(0.5, 0.5, 0.25, 0.25), 1000, 1000);
        approx(large.x / small.x, 10.0);
        approx(large.w / small.w, 10.0);
    }

    #[test]
    fn out_of_range_boxes_are_clipped_inward_instead_of_moved() {
        let px = flip_rect(norm(-0.01, -0.02, 1.2, 1.1), W, H);
        approx(px.x, 0.0);
        approx(px.y, 0.0);
        assert!(px.w <= W as f32 && px.h <= H as f32);
        assert!(px.right() <= W as f32 && px.bottom() <= H as f32);
    }

    #[test]
    fn zero_sized_boxes_stay_zero_and_never_negative() {
        let px = flip_rect(norm(0.5, 0.5, 0.0, 0.0), W, H);
        approx(px.w, 0.0);
        approx(px.h, 0.0);
        assert!(px.x >= 0.0 && px.y >= 0.0);
    }

    #[test]
    fn flipping_keeps_top_below_bottom_for_every_row() {
        for i in 0..20 {
            let y = i as f32 * 0.05;
            let px = flip_rect(norm(0.0, y, 1.0, 0.04), W, H);
            assert!(px.y <= px.bottom(), "y {} 翻转后顶边跑到了底边下面", y);
            assert!(px.y >= 0.0 && px.bottom() <= H as f32);
        }
    }

    /// S3 合并判据的用例。坐标一律用**像素**写（左上原点，y 向下），helper 再折成 Vision 的
    /// 归一化左下框——这样每条判据的几何意图一眼可见，且顺带反复走一遍 `flip_rect`。
    mod merge {
        use super::*;

        const PW: u32 = 800;
        const PH: u32 = 600;

        /// `index` 直接当 `row_index`；`y` 是像素顶边。
        fn line(index: usize, text: &str, x: f32, y: f32, w: f32, h: f32) -> RowObservation {
            line_conf(index, text, x, y, w, h, 0.9)
        }

        fn line_conf(
            index: usize,
            text: &str,
            x: f32,
            y: f32,
            w: f32,
            h: f32,
            confidence: f32,
        ) -> RowObservation {
            RowObservation {
                text: text.to_string(),
                normalized_box: norm(
                    x / PW as f32,
                    (PH as f32 - y - h) / PH as f32,
                    w / PW as f32,
                    h / PH as f32,
                ),
                confidence,
                row_index: index,
            }
        }

        fn merge(rows: &[RowObservation]) -> Vec<BlockDraft> {
            merge_rows(rows, &MergeParams::DEFAULT, PW, PH)
        }

        /// 一段三行的正文：左边界齐、行距 10px（< 1.3×行高）、水平完全重叠。
        fn paragraph() -> Vec<RowObservation> {
            vec![
                line(0, "第一行文字", 100.0, 100.0, 600.0, 30.0),
                line(1, "第二行文字", 100.0, 140.0, 600.0, 30.0),
                line(2, "第三行文字", 100.0, 180.0, 400.0, 30.0),
            ]
        }

        #[test]
        fn a_clean_paragraph_becomes_one_block() {
            let blocks = merge(&paragraph());
            assert_eq!(blocks.len(), 1);
            assert_eq!(blocks[0].text, "第一行文字\n第二行文字\n第三行文字");
            assert_eq!(blocks[0].source_rows, vec![0, 1, 2]);
        }

        #[test]
        fn empty_input_produces_no_blocks() {
            assert!(merge(&[]).is_empty());
        }

        /// 判据①：左边界差 40px，容差 0.6×行高 = 18px。
        #[test]
        fn misaligned_left_edge_starts_a_new_block() {
            let rows = vec![
                line(0, "甲", 100.0, 100.0, 600.0, 30.0),
                line(1, "乙", 140.0, 140.0, 600.0, 30.0),
            ];
            assert_eq!(merge(&rows).len(), 2);
        }

        /// 判据①的边界两侧（容差 0.6×80 = 48）：47px 合、49px 分。
        #[test]
        fn the_alignment_threshold_is_the_only_difference() {
            let inside = vec![
                line(0, "甲", 100.0, 100.0, 300.0, 80.0),
                line(1, "乙", 147.0, 185.0, 300.0, 80.0),
            ];
            let outside = vec![
                line(0, "甲", 100.0, 100.0, 300.0, 80.0),
                line(1, "乙", 149.0, 185.0, 300.0, 80.0),
            ];
            assert_eq!(merge(&inside).len(), 1);
            assert_eq!(merge(&outside).len(), 2);
        }

        /// 判据②：间隙 70px > 1.3×行高 39px。
        #[test]
        fn too_much_vertical_gap_breaks_the_paragraph() {
            let rows = vec![
                line(0, "甲", 100.0, 100.0, 600.0, 30.0),
                line(1, "乙", 100.0, 200.0, 600.0, 30.0),
            ];
            assert_eq!(merge(&rows).len(), 2);
        }

        /// 判据②的反面补一条上下重叠：`vertical_gap` 返回 −1，重叠行不靠它合并。
        #[test]
        fn vertically_overlapping_rows_do_not_merge() {
            let rows = vec![
                line(0, "甲", 100.0, 100.0, 600.0, 60.0),
                line(1, "乙", 100.0, 120.0, 600.0, 60.0),
            ];
            assert_eq!(merge(&rows).len(), 2);
        }

        /// 判据③：两条又高又窄、横向错开的行——①②都过，只有投影重叠不够。
        #[test]
        fn insufficient_horizontal_overlap_breaks_the_paragraph() {
            let rows = vec![
                line(0, "甲", 100.0, 100.0, 10.0, 60.0),
                line(1, "乙", 120.0, 170.0, 10.0, 60.0),
            ];
            assert_eq!(merge(&rows).len(), 2);
            let enough = vec![
                line(0, "甲", 100.0, 100.0, 10.0, 60.0),
                line(1, "乙", 105.0, 170.0, 10.0, 60.0),
            ];
            assert_eq!(merge(&enough).len(), 1);
        }

        /// 判据④：第三行相对块左边界缩进 50px（容差 48），差别只在上一行写没写满。
        #[test]
        fn indented_row_after_an_unfilled_line_starts_a_new_paragraph() {
            let rows = vec![
                line(0, "甲", 100.0, 100.0, 600.0, 80.0),
                line(1, "乙", 140.0, 185.0, 200.0, 80.0),
                line(2, "丙", 150.0, 270.0, 200.0, 80.0),
            ];
            let blocks = merge(&rows);
            assert_eq!(blocks.len(), 2);
            assert_eq!(blocks[0].source_rows, vec![0, 1]);
            assert_eq!(blocks[1].source_rows, vec![2]);
        }

        /// 判据④的反面：上一行已填满参考宽度 85%，缩进不再算新段落（可能是居中换行）。
        #[test]
        fn indented_row_after_a_full_line_stays_in_the_same_paragraph() {
            let rows = vec![
                line(0, "甲", 100.0, 100.0, 600.0, 80.0),
                line(1, "乙", 140.0, 185.0, 600.0, 80.0),
                line(2, "丙", 150.0, 270.0, 200.0, 80.0),
            ];
            let blocks = merge(&rows);
            assert_eq!(blocks.len(), 1);
            assert_eq!(blocks[0].source_rows, vec![0, 1, 2]);
        }

        /// 判据⑤：两行三列的表，六个单元格各自成块，绝不跨列。
        #[test]
        fn table_rows_never_merge_across_columns() {
            let rows = vec![
                line(0, "A1", 50.0, 100.0, 150.0, 40.0),
                line(1, "B1", 300.0, 100.0, 150.0, 40.0),
                line(2, "C1", 600.0, 100.0, 150.0, 40.0),
                line(3, "A2", 50.0, 150.0, 150.0, 40.0),
                line(4, "B2", 300.0, 150.0, 150.0, 40.0),
                line(5, "C2", 600.0, 150.0, 150.0, 40.0),
            ];
            let blocks = merge(&rows);
            assert_eq!(blocks.len(), rows.len());
            for block in &blocks {
                // 每块只有一行；若哪天允许同行合并，也必须落在同一列（x 相同）。
                assert_eq!(block.source_rows.len(), 1);
            }
        }

        /// 硬上限一：13 行齐正文，第 13 行必须另起一块。
        #[test]
        fn the_twelfth_row_is_the_last_one_in_a_block() {
            let rows: Vec<RowObservation> = (0..13)
                .map(|i| line(i, "行", 100.0, 100.0 + i as f32 * 40.0, 600.0, 30.0))
                .collect();
            let blocks = merge(&rows);
            assert_eq!(blocks.len(), 2);
            assert_eq!(blocks[0].source_rows.len(), MergeParams::DEFAULT.max_rows);
            assert_eq!(blocks[1].source_rows, vec![12]);
        }

        /// 硬上限二：700 + 700 = 1400 字 > 1200，几何上再像同一段也要断。
        #[test]
        fn the_character_cap_breaks_before_the_merge() {
            let long = "字".repeat(700);
            let rows = vec![
                line(0, &long, 100.0, 100.0, 600.0, 30.0),
                line(1, &long, 100.0, 140.0, 600.0, 30.0),
            ];
            let blocks = merge(&rows);
            assert_eq!(blocks.len(), 2);
            let exact = "字".repeat(600);
            let fits = vec![
                line(0, &exact, 100.0, 100.0, 600.0, 30.0),
                line(1, &exact, 100.0, 140.0, 600.0, 30.0),
            ];
            assert_eq!(merge(&fits).len(), 1);
        }

        /// 确定性：同一输入跑两次必须逐字节相同，且打乱输入顺序也得到同一分组。
        #[test]
        fn the_grouping_is_deterministic_and_order_independent() {
            let rows = paragraph();
            assert_eq!(merge(&rows), merge(&rows));
            let mut shuffled = rows.clone();
            shuffled.reverse();
            shuffled.push(line(9, "第零行", 100.0, 60.0, 600.0, 30.0));
            let blocks = merge(&shuffled);
            assert_eq!(blocks.len(), 1);
            // 行号按像素顺序落位，不受输入顺序影响。
            assert_eq!(blocks[0].source_rows, vec![9, 0, 1, 2]);
        }

        #[test]
        fn the_block_box_covers_every_member_row() {
            let blocks = merge(&paragraph());
            let box_ = blocks[0].bbox;
            assert!(box_.x <= 100.0);
            assert!(box_.y <= 100.0);
            assert!(box_.right() >= 700.0);
            assert!(box_.bottom() >= 210.0);
        }

        /// 加权而非简单平均：1 字 conf 0.2 + 40 字 conf 0.9 ≈ 0.883，离 0.55 很远。
        #[test]
        fn the_block_confidence_is_weighted_by_characters() {
            let rows = vec![
                line_conf(0, "字", 100.0, 100.0, 300.0, 80.0, 0.2),
                line_conf(1, &"字".repeat(40), 100.0, 185.0, 600.0, 80.0, 0.9),
            ];
            let blocks = merge(&rows);
            assert_eq!(blocks.len(), 1);
            approx(blocks[0].confidence, 0.8829);
            assert!(
                (blocks[0].confidence - 0.55).abs() > 0.1,
                "加权结果与简单平均分不开"
            );
        }
    }
}
