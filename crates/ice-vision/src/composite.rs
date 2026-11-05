//! S8 合成：把"识别结果 + 译文"变成一张成品图（CHG-006 REQ-003 / REQ-006）。
//!
//! 本模块自己**不判档、不排版、不描字形**——那三件事分别归 [`bg`]、[`typeset`]、[`paint`]。它只做
//! 三件更粗的事：把每块的结论收成一张施工清单、在**原图的副本**上分三段施工、把施工结果如实写进
//! [`Composed`]。四件不显然的：
//!
//! 1. **清单先于像素**。每块的背景证据（尤其那四个角）与首行墨迹范围都在动第一个像素之前算完。
//!    两框重叠是真实场景（图文混排的说明书），后算的块会把先擦的块的角读成假背景。
//! 2. **分三段施工，不逐块施工**（擦除 → 底板 → 写字）。docs/04 §4 第 8 条只说了前两段，第三段是
//!    同一条理由的续集：两块覆盖层相交时，逐块"铺完就写"会让后铺的底板埋掉先写的字。三段各自按
//!    块 id 升序走完，同一输入必然同一输出。
//! 3. **输入不可变**（REQ-003）。`source.clone()` 是全链唯一的一次克隆，此后每个写都只打在副本上；
//!    磁盘上那张原图本模块碰不到（导出另属 `export`，且只在用户显式选路径时写）。
//! 4. **擦除清单就是 `InPlace` 那一档**，`degraded` 不是独立的一道门。`spec.md` REQ-003 的边界那句是
//!    "`degraded` 块 SHALL NOT 被擦（它走覆盖层）"——括号给的是理由，而那个理由只在 `Auto` /
//!    `OverlayAll` 下成立（放不下就改判，degraded 必然与 Overlay 同来）。`ReplaceOnly` 是用户显式说了
//!    "别铺底板"，REQ-004 第 4 条给它留的出口是"照画、允许越框、仍标 degraded"，于是出现"既降级又是
//!    回填"的块：这一档不擦等于把新旧两层字叠在一起，与 FR-2.4 那句"擦除原文并绘制译文"正相反。
//!    本包按 `InPlace` 收口，`Skip` / `Overlay` 恒不擦。

use ice_core::document::{OcrDocument, PixelRect};
use image::RgbaImage;

use crate::bg::{self, BackgroundEstimate};
use crate::error::VisionResult;
use crate::paint::{self, FileMetrics};
use crate::typeset::{self, BlockPlan, CompositeMode, Metrics, PlanKind};

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

/// 一张成品，连同"每一块到底怎么画的"那份记录。
///
/// `per_block` 是记录的本体，四个计数里有三个只是从它数出来的（同一次遍历），所以界面读到的
/// 数字与实际画法不可能各说各话（REQ-006 那条"计数 SHALL 与实际画的块数一致"）。`degraded` 是
/// 唯一例外：降级说的是"放不下"，不是"怎么画"，那一列装不下它，因此单开一列块号。
#[derive(Debug, Clone)]
pub struct Composed {
    pub image: RgbaImage,
    /// 块 id → 画法，按 id 升序。"同一张图里混合两种画法"就靠它表达（Q-001）
    pub per_block: Vec<(u32, PlanKind)>,
    /// 就地擦除并回填的块数
    pub replaced: usize,
    /// 铺底板覆盖的块数
    pub overlaid: usize,
    /// 没译文、那块区域原样保留的块数
    pub skipped: usize,
    /// 缩到字号下限仍放不下的**块号**，升序（FR-2.7）。
    ///
    /// 单开一列而不是塞进 `per_block`：降级说的是"放不下"，不是"怎么画"。G 区要按 spec
    /// REQ-004 末句与 REQ-007 那句"橙色描边"画出**是哪几块**，只给一个数画不出来。
    pub degraded_blocks: Vec<u32>,
    /// 块 id → 两支本机字体都画不出的字符**数**。是哪个字不进任何出口（NFR-3）
    pub missing_glyphs: Vec<(u32, usize)>,
}

impl Composed {
    /// 降级块数。从那一列数出来，与 G 区的橙描边同源，不另记一份。
    ///
    /// 不是 `const fn`：`Vec::len` 到 1.87 才在常量上下文里稳定，本仓库的 MSRV 是 1.85。
    pub fn degraded(&self) -> usize {
        self.degraded_blocks.len()
    }

    /// 界面 I 区那句"这一张是怎么画的"。
    ///
    /// 词还是 FR-2.5 与 docs/03 I-02 那两个词，界面不再抄第三个字面量；退场的是"整图只能有一种
    /// 模式"那个承诺——混合时这里如实说"回填 + 覆盖层"。一块都没画（全 `Skip`）时说"未绘制"，
    /// 那张图与输入逐字节相同，这比硬挑一个词更诚实。
    pub const fn mode_label(&self) -> &'static str {
        match (self.replaced > 0, self.overlaid > 0) {
            (true, true) => "回填 + 覆盖层",
            (true, false) => "回填",
            (false, true) => "覆盖层",
            (false, false) => "未绘制",
        }
    }
}

/// 一块的施工单：动像素要用的东西一次备齐，三段各自只取自己那一列。
struct Job {
    id: u32,
    bbox: PixelRect,
    estimate: BackgroundEstimate,
    plan: BlockPlan,
}

/// 把译文画回图片。
#[derive(Debug, Default, Clone)]
pub struct Renderer {
    pub options: CompositeOptions,
}

impl Renderer {
    pub fn new(options: CompositeOptions) -> Self {
        Self { options }
    }

    /// 出一张成品图。
    ///
    /// 唯一的错误来路是本机字体读不到（[`FileMetrics::open`]）→ `VisionError::Font`，界面沿用
    /// 既有那句"本机处理失败，请重试"，已完成的识别与译文不消失。整段应当在 `spawn_blocking` 里
    /// 跑：字形光栅化是 CPU 活（design 的线程那条），本模块只负责不往外吐中间态。
    pub fn compose(
        &self,
        source: &RgbaImage,
        document: &OcrDocument,
        mode: CompositeMode,
    ) -> VisionResult<Composed> {
        let metrics = FileMetrics::open()?;
        let jobs = plan_all(source, document, mode, &metrics, &self.options);

        let mut canvas = source.clone();
        erase_all(&mut canvas, &jobs);
        plate_all(&mut canvas, &jobs);
        let missing_glyphs = draw_all(&mut canvas, &jobs, &metrics);

        // 四个计数一律从 `per_block` 数出来，不再另数一遍清单：界面读到的数字与画法记录
        // 因此不可能各说各话（REQ-006 那条"计数 SHALL 与实际画的块数一致"）
        let per_block: Vec<(u32, PlanKind)> =
            jobs.iter().map(|job| (job.id, job.plan.kind)).collect();
        Ok(Composed {
            image: canvas,
            replaced: count_kind(&per_block, PlanKind::InPlace),
            overlaid: count_kind(&per_block, PlanKind::Overlay),
            skipped: count_kind(&per_block, PlanKind::Skip),
            // 降级不是画法，`per_block` 那一列装不下它，只能回到计划里逐块收块号
            degraded_blocks: jobs
                .iter()
                .filter(|job| job.plan.degraded)
                .map(|job| job.id)
                .collect(),
            per_block,
            missing_glyphs,
        })
    }
}

/// S5 + S6：动第一个像素之前，把每块的背景、首行墨迹范围与版式计划一次算完。
///
/// `metrics` 是 trait 对象而不是具体的 `FileMetrics`：这张清单因此在无头里能用一把假尺钉死
/// （真字体那份由 [`Renderer::compose`] 现场开）。`source` 只读。
///
/// `enabled = false` 的块在这里就被挡掉（[`OcrDocument::enabled_blocks`]），于是它既不进擦除清单、
/// 也不进绘制清单，更不出现在 `per_block` 里——界面上勾掉一块，画面上就该当它不存在。
fn plan_all(
    source: &RgbaImage,
    document: &OcrDocument,
    mode: CompositeMode,
    metrics: &dyn Metrics,
    options: &CompositeOptions,
) -> Vec<Job> {
    let mut jobs: Vec<Job> = document
        .enabled_blocks()
        .map(|block| {
            let estimate = bg::estimate(source, &block.bbox, options);
            let first_line = bg::first_line_extent(source, &block.bbox, estimate.bg);
            Job {
                id: block.id,
                bbox: block.bbox,
                plan: typeset::plan(block, &estimate, metrics, options, mode, first_line),
                estimate,
            }
        })
        .collect();
    jobs.sort_by_key(|job| job.id);
    jobs
}

/// 第一段：只擦回填那一档，覆盖层与跳过的一个像素都不擦（ADR-003 的兜底就在这条 `if` 上）。
fn erase_all(canvas: &mut RgbaImage, jobs: &[Job]) {
    for job in jobs {
        if job.plan.kind == PlanKind::InPlace {
            paint::erase(canvas, &job.bbox, &job.estimate);
        }
    }
}

/// 第二段：覆盖层底板。它是"盖住"不是"擦掉"——底下那层原图一个字节都没变，只是被挡住。
fn plate_all(canvas: &mut RgbaImage, jobs: &[Job]) {
    for job in jobs {
        if job.plan.kind == PlanKind::Overlay {
            // `typeset::plan` 里 plate 与 Overlay 同生同灭，取不到底色那种情况不存在
            if let Some(plate) = job.plan.plate {
                paint::overlay_plate(canvas, &job.bbox, plate);
            }
        }
    }
}

/// 第三段：写字。返回逐块缺字形计数，只带非零的那些块（`Skip` 恒为 0，它一个字都不该写）。
fn draw_all(canvas: &mut RgbaImage, jobs: &[Job], metrics: &FileMetrics) -> Vec<(u32, usize)> {
    let mut missing: Vec<(u32, usize)> = Vec::new();
    for job in jobs {
        let gone = paint::draw_text(canvas, &job.plan, metrics);
        if gone > 0 {
            missing.push((job.id, gone));
        }
    }
    missing
}

fn count_kind(per_block: &[(u32, PlanKind)], kind: PlanKind) -> usize {
    per_block.iter().filter(|(_, k)| *k == kind).count()
}

#[cfg(test)]
mod tests {
    use super::*;
    use ice_core::document::TextBlock;
    use image::{Rgba, RgbaImage};

    const BG: Rgba<u8> = Rgba([240, 240, 240, 255]);
    /// 假"原文笔画"：与背景离得足够远，`bg::estimate` 会把它当前景
    const INK: Rgba<u8> = Rgba([20, 20, 20, 255]);

    /// 一把假尺（与 `typeset::tests` 同一条口径）：宽度 = 字数 × 字号，行高走 trait 默认的 1.25 倍
    struct Ruler;

    impl Metrics for Ruler {
        fn text_width(&self, text: &str, font_size: f32) -> f32 {
            text.chars().count() as f32 * font_size
        }
    }

    /// 一块干净浅底、框内有一行"原文"的图。背景必须是纯色，`Uniform` 档才会判成可回填。
    fn sheet(width: u32, height: u32) -> RgbaImage {
        RgbaImage::from_pixel(width, height, BG)
    }

    /// 在块框里画一条假原文：让 `fg`、`first_line_extent`、对齐推断都有东西可量
    fn with_ink(image: &mut RgbaImage, rect: &PixelRect) {
        let (x0, y0, x1, y1) = (
            rect.x as u32,
            (rect.y + rect.h * 0.25) as u32,
            rect.right() as u32,
            (rect.y + rect.h * 0.75) as u32,
        );
        for y in y0..y1 {
            for x in x0..x1 {
                image.put_pixel(x, y, INK);
            }
        }
    }

    fn block(id: u32, rect: PixelRect, translation: Option<&str>) -> TextBlock {
        let mut block = TextBlock::new(id, "原文", rect, 0.9);
        block.translation = translation.map(str::to_string);
        block
    }

    fn document(blocks: Vec<TextBlock>) -> OcrDocument {
        OcrDocument {
            width: 60,
            height: 60,
            blocks,
            source_label: String::new(),
        }
    }

    /// 一张图 + 一组块，块内已经画上假原文
    fn scene(blocks: &[TextBlock]) -> (RgbaImage, OcrDocument) {
        let mut image = sheet(60, 60);
        for block in blocks {
            with_ink(&mut image, &block.bbox);
        }
        (image, document(blocks.to_vec()))
    }

    /// 这块框里还剩多少个"跟背景不一样"的像素——擦干净了就是 0
    fn ink_in(image: &RgbaImage, rect: &PixelRect) -> usize {
        let mut count = 0;
        for y in rect.y as u32..rect.bottom() as u32 {
            for x in rect.x as u32..rect.right() as u32 {
                if image.get_pixel(x, y).0[..3] != BG.0[..3] {
                    count += 1;
                }
            }
        }
        count
    }

    fn kinds(jobs: &[Job]) -> Vec<(u32, PlanKind)> {
        jobs.iter().map(|job| (job.id, job.plan.kind)).collect()
    }

    #[test]
    fn every_enabled_block_gets_exactly_one_record_in_id_order() {
        let mut off = block(1, PixelRect::new(2.0, 2.0, 20.0, 16.0), Some("译文"));
        off.enabled = false;
        // 故意把 id 打乱交进去：清单必须自己排回升序，绘制次序才谈得上"可复现"
        let blocks = vec![
            block(4, PixelRect::new(26.0, 2.0, 20.0, 16.0), None),
            off,
            block(2, PixelRect::new(2.0, 24.0, 20.0, 16.0), Some("译文")),
        ];
        let (image, doc) = scene(&blocks);
        let jobs = plan_all(
            &image,
            &doc,
            CompositeMode::Auto,
            &Ruler,
            &CompositeOptions::default(),
        );

        assert_eq!(
            kinds(&jobs),
            vec![(2, PlanKind::InPlace), (4, PlanKind::Skip),]
        );
    }

    #[test]
    fn the_counts_are_counted_out_of_the_per_block_record() {
        let blocks = vec![
            block(1, PixelRect::new(2.0, 2.0, 20.0, 16.0), Some("译文")),
            block(2, PixelRect::new(2.0, 24.0, 20.0, 16.0), None),
        ];
        let (image, doc) = scene(&blocks);
        let jobs = plan_all(
            &image,
            &doc,
            CompositeMode::Auto,
            &Ruler,
            &CompositeOptions::default(),
        );
        let per_block = kinds(&jobs);

        assert_eq!(count_kind(&per_block, PlanKind::InPlace), 1);
        assert_eq!(count_kind(&per_block, PlanKind::Skip), 1);
        assert_eq!(count_kind(&per_block, PlanKind::Overlay), 0);
        // 三个数加起来等于记录长度：既没漏记一块，也没凭空多出一块
        assert_eq!(
            count_kind(&per_block, PlanKind::InPlace)
                + count_kind(&per_block, PlanKind::Overlay)
                + count_kind(&per_block, PlanKind::Skip),
            per_block.len()
        );
    }

    #[test]
    fn a_photo_like_background_forces_the_overlay_and_saves_the_pixels() {
        let mut image = sheet(60, 60);
        // 棋盘格：采样带的离群占比远超 0.35，判 `Complex`
        for y in 0..60 {
            for x in 0..60 {
                let dark = (x / 3 + y / 3) % 2 == 0;
                image.put_pixel(x, y, if dark { INK } else { BG });
            }
        }
        let rect = PixelRect::new(10.0, 10.0, 24.0, 16.0);
        let doc = document(vec![block(1, rect, Some("译文"))]);
        let before = image.clone();

        let jobs = plan_all(
            &image,
            &doc,
            CompositeMode::Auto,
            &Ruler,
            &CompositeOptions::default(),
        );
        assert_eq!(kinds(&jobs), vec![(1, PlanKind::Overlay)]);

        // `Complex` 那一档：擦与铺都不许动到框内原有的那些格子之外的东西
        let mut canvas = before.clone();
        erase_all(&mut canvas, &jobs);
        assert_eq!(changed(&canvas, &before), 0, "Complex 档一个像素都不该擦");
    }

    #[test]
    fn forced_overlay_leaves_everything_outside_the_boxes_untouched() {
        let blocks = vec![
            block(1, PixelRect::new(2.0, 2.0, 20.0, 16.0), Some("译文")),
            block(2, PixelRect::new(2.0, 24.0, 20.0, 16.0), Some("译文")),
        ];
        let (image, doc) = scene(&blocks);
        let jobs = plan_all(
            &image,
            &doc,
            CompositeMode::OverlayAll,
            &Ruler,
            &CompositeOptions::default(),
        );
        assert_eq!(
            kinds(&jobs),
            vec![(1, PlanKind::Overlay), (2, PlanKind::Overlay)]
        );

        let before = image.clone();
        let mut canvas = before.clone();
        erase_all(&mut canvas, &jobs);
        assert_eq!(changed(&canvas, &before), 0, "强制覆盖层不擦任何像素");
        plate_all(&mut canvas, &jobs);
        for y in 0..60 {
            for x in 0..60 {
                let inside_box = blocks.iter().any(|b| {
                    (x as f32) >= b.bbox.x
                        && (x as f32) < b.bbox.right()
                        && (y as f32) >= b.bbox.y
                        && (y as f32) < b.bbox.bottom()
                });
                if !inside_box {
                    assert_eq!(
                        canvas.get_pixel(x, y),
                        before.get_pixel(x, y),
                        "块外 ({x},{y}) 被铺底板动到了"
                    );
                }
            }
        }
    }

    #[test]
    fn a_degraded_backfill_still_gets_erased_under_replace_only() {
        // 窄框 + 长译文：缩到 55% 也放不下 ⇒ degraded。`ReplaceOnly` 不许改判覆盖层，
        // 于是这块既是 InPlace 又带 degraded——它照旧该擦，否则新旧两层字叠在一起。
        let rect = PixelRect::new(2.0, 2.0, 20.0, 16.0);
        let long = "这一段译文比这块框能装下的宽得多";
        let blocks = vec![block(1, rect, Some(long))];
        let (image, doc) = scene(&blocks);
        let jobs = plan_all(
            &image,
            &doc,
            CompositeMode::ReplaceOnly,
            &Ruler,
            &CompositeOptions::default(),
        );
        assert_eq!(jobs[0].plan.kind, PlanKind::InPlace);
        assert!(jobs[0].plan.degraded, "缩到底放不下就该标降级");

        let mut canvas = image.clone();
        erase_all(&mut canvas, &jobs);
        assert_eq!(ink_in(&canvas, &rect), 0, "降级的回填块也要把原文擦掉");
    }

    #[test]
    fn a_skipped_block_keeps_the_original_text_on_the_picture() {
        let rect = PixelRect::new(2.0, 24.0, 20.0, 16.0);
        let blocks = vec![block(1, rect, None)];
        let (image, doc) = scene(&blocks);
        let jobs = plan_all(
            &image,
            &doc,
            CompositeMode::Auto,
            &Ruler,
            &CompositeOptions::default(),
        );
        assert_eq!(kinds(&jobs), vec![(1, PlanKind::Skip)]);

        let before = image.clone();
        let mut canvas = before.clone();
        erase_all(&mut canvas, &jobs);
        plate_all(&mut canvas, &jobs);
        assert_eq!(
            changed(&canvas, &before),
            0,
            "没译文的块原样保留（擦了就永久丢信息）"
        );
        assert!(ink_in(&canvas, &rect) > 0);
    }

    #[test]
    fn a_plate_is_laid_after_every_block_has_been_erased() {
        // 一块覆盖层（铺底板）+ 一块回填（擦除），两框上下相接、下块的擦除区伸进上块的底板。
        // "先铺板再擦下一块"会把那块板擦掉一个洞；分段处理（擦除 → 底板）不会。
        // 这块板直接构造而不走 `plan_all`：一张图上很难同时有噪声背景和干净纯色背景，而这条要测的
        // 只是两段的先后，与谁判成哪一档无关。
        let plate_box = PixelRect::new(2.0, 2.0, 20.0, 16.0);
        let backfill = PixelRect::new(2.0, 12.0, 20.0, 16.0);
        const PLATE: Rgba<u8> = Rgba([40, 40, 40, 255]);
        let image = sheet(60, 60);
        let flat = bg::estimate(&image, &backfill, &CompositeOptions::default()).bg;
        let jobs = vec![
            job(1, plate_box, PlanKind::Overlay, Some(PLATE), flat),
            job(2, backfill, PlanKind::InPlace, None, flat),
        ];
        // 两框不相接的话，下面两条断言就都在空转
        assert!(plate_box.bottom() > backfill.y);
        assert_ne!(PLATE, flat, "底板色和擦除色相同，这条测不出次序");

        // 这一格落在两框的重叠区：底板铺得到、擦除也盖得住，两段的先后才在这里分得开
        let probe = (10u32, 14u32);
        let mut wrong = image.clone();
        plate_all(&mut wrong, &jobs);
        erase_all(&mut wrong, &jobs);
        assert_eq!(
            wrong.get_pixel(probe.0, probe.1),
            &flat,
            "构造不对：这一格本该被下一块的擦除盖到"
        );

        let mut staged = image.clone();
        erase_all(&mut staged, &jobs);
        plate_all(&mut staged, &jobs);
        assert_eq!(
            staged.get_pixel(probe.0, probe.1),
            &PLATE,
            "先全部擦除再铺底板，那块板不该有洞"
        );
    }

    /// 一张只填 `kind` / `plate` / `area` 的计划：这三段施工各取一列，其余字段用不上。
    fn job(
        id: u32,
        bbox: PixelRect,
        kind: PlanKind,
        plate: Option<Rgba<u8>>,
        bg_color: Rgba<u8>,
    ) -> Job {
        Job {
            id,
            bbox,
            estimate: BackgroundEstimate {
                bg: bg_color,
                fg: INK,
                variance: 0.0,
                kind: if plate.is_some() {
                    bg::BackgroundKind::Complex
                } else {
                    bg::BackgroundKind::Uniform
                },
                corners: [bg_color; 4],
            },
            plan: BlockPlan {
                kind,
                font_size: 12.0,
                lines: Vec::new(),
                align: typeset::Align::Left,
                degraded: false,
                text_color: INK,
                plate,
                area: bbox,
            },
        }
    }

    #[test]
    fn the_mode_word_names_whichever_layers_were_actually_used() {
        let composed = |replaced, overlaid| Composed {
            image: sheet(1, 1),
            per_block: Vec::new(),
            replaced,
            overlaid,
            skipped: 0,
            degraded_blocks: Vec::new(),
            missing_glyphs: Vec::new(),
        };
        assert_eq!(composed(3, 0).mode_label(), "回填");
        assert_eq!(composed(0, 2).mode_label(), "覆盖层");
        assert_eq!(composed(3, 2).mode_label(), "回填 + 覆盖层");
        assert_eq!(composed(0, 0).mode_label(), "未绘制");
    }

    fn changed(a: &RgbaImage, b: &RgbaImage) -> usize {
        a.pixels()
            .zip(b.pixels())
            .filter(|(x, y)| x.0 != y.0)
            .count()
    }

    /// 动像素那三段都要真字形，所以这几条随 [`crate::paint`] 的真字体用例一起只在 mac 面上跑
    /// （NFR-7 只承诺 S 档；清单与画法那几条判据在上面已经无头成立）。
    #[cfg(target_os = "macos")]
    mod composed_on_macos {
        use super::*;

        /// REQ-003 那条哈希判据的无头形状：副本逐像素相等 + 磁盘上那张 PNG 字节相等
        #[test]
        fn the_source_picture_is_not_touched_in_memory_or_on_disk() {
            let dir = tempfile::tempdir().expect("临时目录");
            let path = dir.path().join("原图.png");
            let blocks = vec![
                block(
                    1,
                    PixelRect::new(2.0, 2.0, 24.0, 16.0),
                    Some("本机识别不上传"),
                ),
                block(2, PixelRect::new(2.0, 30.0, 24.0, 16.0), Some("好")),
            ];
            let (image, doc) = scene(&blocks);
            image.save(&path).expect("PNG 写得出去");
            let bytes_before = std::fs::read(&path).expect("读得回那张 PNG");
            let source_before = image.clone();

            let composed = Renderer::default()
                .compose(&image, &doc, CompositeMode::Auto)
                .expect("mac 上字体在位，合成不该失败");

            assert!(
                changed(&composed.image, &source_before) > 0,
                "成品什么都没改？"
            );
            assert_eq!(
                changed(&image, &source_before),
                0,
                "传进来的那张原图被改到了"
            );
            assert_eq!(
                std::fs::read(&path).expect("再读一次"),
                bytes_before,
                "磁盘上那张 PNG 的字节变了"
            );
        }

        /// docs/04 §4 第 8 条：先全部擦除、再全部绘制。上下两块相接、下块的擦除区伸进上块那一行字
        /// 的下半截——逐块"擦完就画"会擦掉上块刚画好的一截，两段处理不会。
        ///
        /// 三条断言是一套：先量出"上块单独画出来那一截有多少点"，再证明错序把它归零（不然这条
        /// 测试就成了空转），最后证明生产次序把它原样留着。
        ///
        /// 上块的译文用汉字（字形几乎铺满整个 em 盒，下半截一定有点），下块只一个字且靠左，
        /// 取样带因此只可能落着上块的字。
        #[test]
        fn erasing_the_lower_block_does_not_wipe_the_line_above_it() {
            let upper = PixelRect::new(2.0, 2.0, 40.0, 20.0);
            let lower = PixelRect::new(2.0, 8.0, 40.0, 20.0);
            let mut image = sheet(60, 60);
            // 只在**上块**里画上假原文，而且压在 8 行以上：下块那一圈的采样带因此仍然干净，
            // 两块都判 Uniform（擦除是平的 `bg` 实心填），这条断言才是"擦掉 vs 没擦掉"两态
            for y in 3..6 {
                for x in 4..20 {
                    image.put_pixel(x, y, INK);
                }
            }
            let doc = document(vec![
                block(1, upper, Some("本机识别")),
                block(2, lower, Some("好")),
            ]);
            let renderer = Renderer::default();
            let metrics = FileMetrics::open().expect("字体在位");
            let jobs = plan_all(
                &image,
                &doc,
                CompositeMode::Auto,
                &metrics,
                &renderer.options,
            );
            assert_eq!(
                jobs.iter().map(|job| job.plan.kind).collect::<Vec<_>>(),
                vec![PlanKind::InPlace, PlanKind::InPlace]
            );

            // 上块那行字的右下角那一截：落在下块的擦除区里
            let band = PixelRect::new(30.0, 8.0, 10.0, 3.0);

            let mut only_upper = image.clone();
            erase_all(&mut only_upper, &jobs[..1]);
            paint::draw_text(&mut only_upper, &jobs[0].plan, &metrics);
            let drawn = ink_in(&only_upper, &band);
            assert!(drawn > 0, "参照图都没有墨迹，这条断言就成了空转");

            // 逐块处理（故意的错序）：擦下块会把上块刚画的下半截抹掉
            let mut one_by_one = image.clone();
            erase_all(&mut one_by_one, &jobs[..1]);
            paint::draw_text(&mut one_by_one, &jobs[0].plan, &metrics);
            erase_all(&mut one_by_one, &jobs[1..]);
            assert_eq!(
                ink_in(&one_by_one, &band),
                0,
                "构造不出错序的后果，说明两框本来不相接，测试要改"
            );

            // 分段处理（生产次序）：下块的擦除碰不到上块刚画好的字
            let mut staged = image.clone();
            erase_all(&mut staged, &jobs);
            plate_all(&mut staged, &jobs);
            for job in &jobs {
                paint::draw_text(&mut staged, &job.plan, &metrics);
            }
            assert_eq!(
                ink_in(&staged, &band),
                drawn,
                "先全部擦除再全部绘制之后，上块那一行的下半截应当与参照图逐点相同"
            );
        }

        /// 端到端：回填那块确实出现了新写的字，且 `Composed` 与 `per_block` 说的是同一件事
        #[test]
        fn a_backfilled_block_ends_up_with_ink_where_the_plan_says() {
            let rect = PixelRect::new(2.0, 2.0, 40.0, 20.0);
            let (image, doc) = scene(&[block(1, rect, Some("本机识别"))]);
            let composed = Renderer::default()
                .compose(&image, &doc, CompositeMode::Auto)
                .expect("mac 上字体在位");

            assert_eq!(composed.per_block, vec![(1, PlanKind::InPlace)]);
            assert_eq!(composed.replaced, 1);
            assert_eq!(composed.overlaid, 0);
            assert_eq!(composed.skipped, 0);
            assert!(ink_in(&composed.image, &rect) > 0, "原文擦掉了却没写进译文");
            // 黑体第 1 面覆盖这些汉字，所以这一段不该报缺字形
            assert!(composed.missing_glyphs.is_empty());
        }

        /// 两支字体都没有的字符：块照常画，只是那个字符不占位，计数如实带出来（REQ-005 / NFR-3）
        #[test]
        fn a_glyph_nobody_has_is_reported_as_a_count_only() {
            let rect = PixelRect::new(2.0, 2.0, 40.0, 20.0);
            let (image, doc) = scene(&[block(1, rect, Some("好\u{0378}"))]);
            let composed = Renderer::default()
                .compose(&image, &doc, CompositeMode::Auto)
                .expect("mac 上字体在位");
            assert_eq!(composed.missing_glyphs, vec![(1, 1)]);
        }
    }
}
