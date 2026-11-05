//! S7 绘制：把 `typeset` 的计划变成像素（CHG-006 REQ-003 / REQ-005 / REQ-006）。
//!
//! 本包**唯一**读系统字体文件的地方，也是**唯一**逐像素写的地方。两件事各有一条收拢的理由：
//! 字体在不在、face 号对不对是运行环境问题，必须集中成一句 `VisionError::Font`，而不是散在合成链
//! 各处；像素写一旦散开，就会有人在第二个模块里发明第二套擦除口径或第二套混合公式。这里不发网络、
//! 不落文件、不碰 UI crate（NFR-6），并且**只往调用方给的画布上写**——"输入不可变"（REQ-003）是
//! `composite.rs` 那句 `source.clone()` 的责任，本模块拿到什么画布就写什么画布。
//!
//! 四件不显然的事：
//!
//! 1. **缓存的是字节，不是 `Face`**。字体字节进 `OnceLock<Vec<u8>>`（一次读盘，之后每次合成都免费），
//!    `Face` 由 [`FileMetrics::open`] 现场解析。`Face` 的 `Sync` 是上游的实现细节而不是承诺，把它塞进
//!    `static` 等于替别人许一个没人保证的愿；而 `FileMetrics` 是**一次合成一个实例**，只在同一个
//!    `spawn_blocking` 里活着，所以既不跨线程共享，也不会在排版测量循环里反复解析一支十几兆的 `.ttc`。
//! 2. **缺字形不画豆腐块**（ADR-003 那条纪律的像素侧）：主字体没有就问后备，两支都没有就跳过那个字符、
//!    不占推进宽度、只把计数交出去。日志与界面只许拿到一个数——是哪个字进了日志，等于把用户内容写进
//!    日志（NFR-3）。
//! 3. **三档擦除不能合成一档**：`Uniform` 用 `bg` 实心填框内、采样带一个像素都不动（改了它就把"背景
//!    证据"擦掉了，第二个块的 `corners` 会读到假背景）；`Gradient` 用四角双线性插值填满框内、再羽化
//!    接缝，SHALL NOT 用一条均值色糊过去——那正是补丁感的来源；`Complex` 一个像素都不擦。
//! 4. **"量出来的宽"与"画出去的宽"是同一条**：[`FileMetrics::text_width`] 和绘制时挪笔用的都是
//!    [`advance_of`]，所以 `typeset` 计划里的行宽不需要、也不可能在绘制侧被算成第二个数。

use std::sync::OnceLock;

use ab_glyph_rasterizer::{Rasterizer, point};
use ice_core::document::PixelRect;
use ice_core::font::{GOTHIC_PATH, HEITI_PATH, HEITI_SC_FACE_INDEX};
use image::{Rgba, RgbaImage};
use owned_ttf_parser::{Face, GlyphId, OutlineBuilder};

use crate::bg::{BackgroundEstimate, BackgroundKind, Span, corner_mean, inset};
use crate::error::{VisionError, VisionResult};
use crate::typeset::{BlockPlan, Metrics, PlanKind};

/// 字形轮廓相对行盒向外的余量（字号的比例）。
///
/// 两个理由：真实字体的笔画可以微微越过 em 盒（斜体尤其），照行盒大小建栅格会把越出的那一截裁掉；
/// 而 `Rasterizer::for_each_pixel_2d` 按宽度取模遍历，**宽度为 0 会除零**，所以下限夹到 1 像素——
/// 那个 `.max(1.0)` 不是防御性写法，是这一层能跑的前提。
const INK_PAD_RATIO: f32 = 0.25;
/// 羽化那一圈的宽度：docs/04 §4 第 6 条"再加 1px 模糊边缘"
const FEATHER_PX: i64 = 1;

/// 主字体字节。读不到就存空表，让 `Face::parse` 自然失败、在 [`FileMetrics::open`] 一处出口报成
/// `VisionError::Font`；`OnceLock` 只填一次，所以磁盘故障不会被每个块重试一遍。
static HEITI_BYTES: OnceLock<Vec<u8>> = OnceLock::new();
/// 后备字体字节。**读不到不是错误**：少一支字体只是"某些字符没人能画"，计数照实报。
static GOTHIC_BYTES: OnceLock<Vec<u8>> = OnceLock::new();

fn heiti_bytes() -> &'static [u8] {
    HEITI_BYTES.get_or_init(|| std::fs::read(HEITI_PATH).unwrap_or_default())
}

fn gothic_bytes() -> &'static [u8] {
    GOTHIC_BYTES.get_or_init(|| std::fs::read(GOTHIC_PATH).unwrap_or_default())
}

/// 解析一支字体的一面。错误文字里只有路径与面号，没有任何用户内容（NFR-3）。
fn face_from<'a>(bytes: &'a [u8], index: u32, path: &str) -> VisionResult<Face<'a>> {
    Face::parse(bytes, index)
        .map_err(|err| VisionError::Font(format!("{path} 第 {index} 面读不出来：{err}")))
}

/// 真字体那把尺子，实现 [`typeset::Metrics`]。一次合成开一个实例，实例内不重复解析。
pub struct FileMetrics {
    heiti: Face<'static>,
    gothic: Option<Face<'static>>,
}

impl FileMetrics {
    /// 打开本机字体。主字体读不到或 face 号对不上 → `VisionError::Font`（不 panic，也不静默换一支）。
    ///
    /// 后备读不到只让 `gothic` 为空——那是覆盖面变窄，不是不能工作。
    pub fn open() -> VisionResult<FileMetrics> {
        let heiti = face_from(heiti_bytes(), HEITI_SC_FACE_INDEX, HEITI_PATH)?;
        let gothic = face_from(gothic_bytes(), 0, GOTHIC_PATH).ok();
        Ok(FileMetrics { heiti, gothic })
    }

    /// 这个字由哪一面画、字形号是多少；`None` 就是"两支本机字体都没有这个字"。
    ///
    /// 判"缺不缺字形"全仓只有这一条（[`count_missing`] 与 [`draw_text`] 都问它），所以界面上那句
    /// "该块有 N 个字符本机字体没有"与实际画出来的东西不会各说各话。
    fn glyph(&self, ch: char) -> Option<(&Face<'static>, GlyphId)> {
        if let Some(gid) = self.heiti.glyph_index(ch) {
            return Some((&self.heiti, gid));
        }
        let face = self.gothic.as_ref()?;
        face.glyph_index(ch).map(|gid| (face, gid))
    }

    /// 推进宽度（像素）。两支都画不出的是 **0**——不占位，给一个不存在的字留白比留白更难看。
    fn advance(&self, ch: char, font_size: f32) -> f32 {
        self.glyph(ch)
            .map_or(0.0, |(face, gid)| advance_of(face, gid, font_size))
    }

    /// 基线在行盒里往下多少。**固定用主字体的 `ascender`**：行与行的相对位置不许取决于某个字符
    /// 恰好落进了后备字体，否则中英混排（黑体没有的那几个字）会在同一行里抖两下。
    fn baseline_offset(&self, font_size: f32) -> f32 {
        self.heiti.ascender() as f32 * scale(&self.heiti, font_size)
    }
}

impl Metrics for FileMetrics {
    /// 逐字符推进宽度求和。与绘制侧挪笔同一条（见模块注释第 4 条）；空串、以及整串都缺字形的串
    /// 都是 0.0——量不出宽度就报量不出，不猜一个。
    fn text_width(&self, text: &str, font_size: f32) -> f32 {
        text.chars().map(|ch| self.advance(ch, font_size)).sum()
    }
}

/// 字体单位 → 像素的比例。`units_per_em` 为 0（表残缺）时按 1 算，不让一次除法变成 `inf`。
fn scale(face: &Face, font_size: f32) -> f32 {
    font_size / f32::from(face.units_per_em().max(1))
}

/// 某个字形在某个字号下占多宽。主/后备都走这一条，所以混排一行的总宽与逐字描出来的位置一致。
///
/// 0.25.1 的 `Face` 没有按 ppem 取度量的 `glyph_metrics`，只有字体单位的 `glyph_hor_advance`；
/// 于是自己除 `units_per_em`——这与 [`scale`] 是同一个除法，度量侧和绘制侧因此不会算出两个数。
fn advance_of(face: &Face, gid: GlyphId, font_size: f32) -> f32 {
    face.glyph_hor_advance(gid).unwrap_or(0) as f32 * scale(face, font_size)
}

/// 两支本机字体都画不出的字符数。**只有计数**——是哪个字不许出现在任何出口（NFR-3）。
pub fn count_missing(text: &str, metrics: &FileMetrics) -> usize {
    text.chars()
        .filter(|ch| metrics.glyph(*ch).is_none())
        .count()
}

/// 把一块计划里的字画进画布，返回**没画出来的字符数**（`Skip` 恒为 0，因为它一个字都不该写）。
///
/// 不做裁切：`ReplaceOnly` 那一档是用户显式说了"别铺底板"，越框也要把字写完（FR-2.7 的验收原文是
/// "无文字被裁切丢失"）；越出去的像素只受画布边界约束。
pub fn draw_text(canvas: &mut RgbaImage, plan: &BlockPlan, metrics: &FileMetrics) -> usize {
    if plan.kind == PlanKind::Skip {
        return 0;
    }
    let line_height = metrics.line_height(plan.font_size);
    let mut missing = 0;
    for (index, line) in plan.lines.iter().enumerate() {
        let Some(cell) = plan.line_box(index, line_height) else {
            break;
        };
        missing += draw_line(
            canvas,
            &cell,
            &line.text,
            plan.text_color,
            metrics,
            plan.font_size,
        );
    }
    missing
}

/// 一行：按行盒外扩 [`INK_PAD_RATIO`] 建栅格，逐字描轮廓，再把 coverage 当 alpha 混进画布。
///
/// 栅格按行建（不是整图）：一整张 4000×3000 的图配一个 `Rasterizer` 是 48MB 的浮点缓冲，而一行
/// 只有 `行宽 × 1.25 字号`。行数是几，分配就是几。
fn draw_line(
    canvas: &mut RgbaImage,
    cell: &PixelRect,
    text: &str,
    color: Rgba<u8>,
    metrics: &FileMetrics,
    font_size: f32,
) -> usize {
    let pad = (font_size * INK_PAD_RATIO).ceil().max(1.0) as i64;
    let x0 = cell.x.floor() as i64 - pad;
    let y0 = cell.y.floor() as i64 - pad;
    let width = (cell.w.ceil() as i64 + pad * 2).max(1) as usize;
    let height = (cell.h.ceil() as i64 + pad * 2).max(1) as usize;

    // 栅格坐标是"相对这一格左上角"：绝对像素 = 原点 + 栅格坐标
    let baseline = cell.y + metrics.baseline_offset(font_size) - y0 as f32;
    let mut raster = Rasterizer::new(width, height);
    let mut pen_x = cell.x;
    let mut missing = 0;
    for ch in text.chars() {
        let Some((face, gid)) = metrics.glyph(ch) else {
            // 缺字形：不画豆腐块，也不挪笔（模块注释第 2 条）
            missing += 1;
            continue;
        };
        let step = advance_of(face, gid, font_size);
        let mut builder = ToPixels {
            raster: &mut raster,
            scale: scale(face, font_size),
            pen_x: pen_x - x0 as f32,
            baseline,
            cursor: (0.0, 0.0),
            start: (0.0, 0.0),
        };
        face.outline_glyph(gid, &mut builder);
        pen_x += step;
    }

    let (w, h) = (canvas.width() as i64, canvas.height() as i64);
    raster.for_each_pixel_2d(|gx, gy, coverage| {
        let (x, y) = (x0 + gx as i64, y0 + gy as i64);
        if x < 0 || y < 0 || x >= w || y >= h {
            return;
        }
        blend(canvas.get_pixel_mut(x as u32, y as u32), color, coverage);
    });
    missing
}

/// src-over：coverage 当几何覆盖率，再乘字色自带的 alpha。
///
/// alpha 也按 over 合成——透明底 PNG 那一类用例里，把一格 α=0 写成 α=255 等于凭空给原图加一块
/// 不透明补丁。coverage 为 0（栅格里绝大多数像素）直接返回：既不写也不改值，省下整趟空转的除法。
fn blend(dst: &mut Rgba<u8>, src: Rgba<u8>, coverage: f32) {
    let a = coverage.clamp(0.0, 1.0) * (f32::from(src.0[3]) / 255.0);
    if a == 0.0 {
        return;
    }
    let da = f32::from(dst.0[3]) / 255.0;
    let out_a = a + da * (1.0 - a);
    for slot in 0..3 {
        let over = f32::from(src.0[slot]) * a + f32::from(dst.0[slot]) * da * (1.0 - a);
        dst.0[slot] = (over / out_a).round().clamp(0.0, 255.0) as u8;
    }
    dst.0[3] = (out_a * 255.0).round() as u8;
}

/// 按三档擦掉一块原文。**只有 `InPlace` 的块该走到这里**（调用方按 `plan.kind` 分派），`Complex`
/// 在这里再挡一道：它是 ADR-003 的兜底，也是 L-1 那条"照片背景不擦"的边界。
///
/// 写的范围就是 [`bg::inset`] 给的那段像素区间——与估计背景时"框覆盖哪些像素"共用一条口径，两处
/// 不许有第二种说法。区间为空（0 宽块、框完全在图外）什么都不写。
pub fn erase(canvas: &mut RgbaImage, rect: &PixelRect, estimate: &BackgroundEstimate) {
    let span = inset(canvas, rect);
    if span.0 >= span.2 || span.1 >= span.3 {
        return;
    }
    match estimate.kind {
        BackgroundKind::Uniform => fill(canvas, span, estimate.bg),
        BackgroundKind::Gradient => {
            fill_bilinear(canvas, span, &estimate.corners);
            feather(canvas, span);
        }
        BackgroundKind::Complex => {}
    }
}

/// 覆盖层底板：填满块框，**不擦任何像素**（REQ-006"不修改原图像素"里，盖住不等于擦掉——底下那一层
/// 原图一个字节都没变，只是被挡住了）。
///
/// 直写而不混合：`typeset::plate_color` 把底板 alpha 钉死成 255，这里若再走 src-over，底下透明的
/// 原图会混进来，"底色与字色对比度 ≥ 4.5"那条判据就只在纸面上成立。
pub fn overlay_plate(canvas: &mut RgbaImage, rect: &PixelRect, plate: Rgba<u8>) {
    fill(canvas, inset(canvas, rect), plate);
}

fn fill(canvas: &mut RgbaImage, span: Span, color: Rgba<u8>) {
    for y in span.1..span.3 {
        for x in span.0..span.2 {
            canvas.put_pixel(x as u32, y as u32, color);
        }
    }
}

/// 四角双线性插值填满框内。`tx`/`ty` 归一化成"首像素 0、末像素 1"，所以左右两列拿到的就是
/// `corners` 里那两列的颜色——`Gradient` 的验收说"擦完仍是渐变且角色对得上"，靠的就是这一条。
fn fill_bilinear(canvas: &mut RgbaImage, span: Span, corners: &[Rgba<u8>; 4]) {
    let columns = (span.2 - span.0).max(2) as f32 - 1.0;
    let rows = (span.3 - span.1).max(2) as f32 - 1.0;
    for y in span.1..span.3 {
        for x in span.0..span.2 {
            let tx = (x - span.0) as f32 / columns;
            let ty = (y - span.1) as f32 / rows;
            canvas.put_pixel(x as u32, y as u32, bilinear(corners, tx, ty));
        }
    }
}

/// 左上、右上、左下、右下按 `tx`（横向）与 `ty`（纵向）插值。四通道同插：alpha 也是背景证据的一部分。
fn bilinear(corners: &[Rgba<u8>; 4], tx: f32, ty: f32) -> Rgba<u8> {
    let top = lerp(&corners[0], &corners[1], tx);
    let bottom = lerp(&corners[2], &corners[3], tx);
    lerp(&top, &bottom, ty)
}

fn lerp(a: &Rgba<u8>, b: &Rgba<u8>, t: f32) -> Rgba<u8> {
    let channel = |x: u8, y: u8| {
        (f32::from(x) + (f32::from(y) - f32::from(x)) * t)
            .round()
            .clamp(0.0, 255.0) as u8
    };
    Rgba([
        channel(a.0[0], b.0[0]),
        channel(a.0[1], b.0[1]),
        channel(a.0[2], b.0[2]),
        channel(a.0[3], b.0[3]),
    ])
}

/// 擦除区**外扩 1px 的那一圈**做 3×3 均值：接缝那一行同时看到填进去的渐变和带里的原背景，平均一次
/// 就把"边框"变成"过渡"。
///
/// 三个讲究：
/// - 先读完整个圈再写（`means` 快照）。边读边写会让圈内的像素看到彼此的新值，一圈糊下来颜色会朝
///   一个方向漂移——那不是我要的第三条效果。
/// - 只碰外扩那一圈。圈内是刚填好的渐变，再平均一次就把它糊成灰斑；圈外隔两格以外一个字都不动，
///   那是别的块要采的背景证据。
/// - 3×3 用的是 [`bg::corner_mean`]，与采四角同一个均值口径；全仓不出现第二种"邻域平均"。
fn feather(canvas: &mut RgbaImage, span: Span) {
    let outer = (
        span.0 - FEATHER_PX,
        span.1 - FEATHER_PX,
        span.2 + FEATHER_PX,
        span.3 + FEATHER_PX,
    );
    let (w, h) = (canvas.width() as i64, canvas.height() as i64);
    let mut means: Vec<(u32, u32, Rgba<u8>)> = Vec::new();
    for y in outer.1..outer.3 {
        for x in outer.0..outer.2 {
            let inside_box = x >= span.0 && x < span.2 && y >= span.1 && y < span.3;
            if inside_box || x < 0 || y < 0 || x >= w || y >= h {
                continue;
            }
            means.push((x as u32, y as u32, corner_mean(canvas, x, y)));
        }
    }
    for (x, y, color) in means {
        canvas.put_pixel(x, y, color);
    }
}

/// `ttf_parser` 给的是字体单位、y 轴向上；栅格是像素、y 轴向下。这一层只做这一个仿射。
///
/// 与 `examples/self_metric.rs` 里那份同名 builder 是两份实现，这是**已知重复**：那份是 P6 度量的
/// 取证工具，它的逐字节结果已经进了账本（24 轮相同），改走生产路径会让那张对照表失去可比性；
/// T-011 的 `compose_metric.rs` 直接用本模块之后，那份就该退役。
struct ToPixels<'r> {
    raster: &'r mut Rasterizer,
    scale: f32,
    pen_x: f32,
    baseline: f32,
    cursor: (f32, f32),
    start: (f32, f32),
}

impl ToPixels<'_> {
    fn at(&self, x: f32, y: f32) -> (f32, f32) {
        (self.pen_x + x * self.scale, self.baseline - y * self.scale)
    }
}

impl OutlineBuilder for ToPixels<'_> {
    fn move_to(&mut self, x: f32, y: f32) {
        self.start = self.at(x, y);
        self.cursor = self.start;
    }

    fn line_to(&mut self, x: f32, y: f32) {
        let to = self.at(x, y);
        self.raster
            .draw_line(point(self.cursor.0, self.cursor.1), point(to.0, to.1));
        self.cursor = to;
    }

    fn quad_to(&mut self, x1: f32, y1: f32, x: f32, y: f32) {
        let control = self.at(x1, y1);
        let to = self.at(x, y);
        self.raster.draw_quad(
            point(self.cursor.0, self.cursor.1),
            point(control.0, control.1),
            point(to.0, to.1),
        );
        self.cursor = to;
    }

    fn curve_to(&mut self, x1: f32, y1: f32, x2: f32, y2: f32, x: f32, y: f32) {
        let c1 = self.at(x1, y1);
        let c2 = self.at(x2, y2);
        let to = self.at(x, y);
        self.raster.draw_cubic(
            point(self.cursor.0, self.cursor.1),
            point(c1.0, c1.1),
            point(c2.0, c2.1),
            point(to.0, to.1),
        );
        self.cursor = to;
    }

    fn close(&mut self) {
        self.raster.draw_line(
            point(self.cursor.0, self.cursor.1),
            point(self.start.0, self.start.1),
        );
        self.cursor = self.start;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::typeset::{Align, PlannedLine};

    const DARK: Rgba<u8> = Rgba([10, 10, 10, 255]);
    const WHITE: Rgba<u8> = Rgba([250, 250, 250, 255]);
    /// 框里残留的原文笔画：既不是背景色也不是擦除色，"被盖掉"这件事才在逐像素比较里数得出来
    const INK: Rgba<u8> = Rgba([128, 128, 128, 255]);
    /// Unicode 没分配的码位：两支字体都不可能有它的字形，它也不会碰巧被画成一个方块
    const NO_GLYPH: char = '\u{0378}';

    fn canvas(w: u32, h: u32, color: Rgba<u8>) -> RgbaImage {
        RgbaImage::from_pixel(w, h, color)
    }

    fn estimate(kind: BackgroundKind, bg: Rgba<u8>, corners: [Rgba<u8>; 4]) -> BackgroundEstimate {
        BackgroundEstimate {
            bg,
            fg: WHITE,
            variance: 0.0,
            kind,
            corners,
        }
    }

    /// 一块只有一行的计划。绘制侧只关心"落在哪一格、什么色、画不画得出来"，版式判据在 `typeset` 测。
    fn one_line(text: &str, area: PixelRect, kind: PlanKind) -> BlockPlan {
        BlockPlan {
            kind,
            font_size: 20.0,
            lines: vec![PlannedLine {
                text: text.to_string(),
                width: area.w,
            }],
            align: Align::Left,
            degraded: false,
            text_color: WHITE,
            plate: None,
            area,
        }
    }

    /// 图里有几个像素与上一秒不同
    fn changed(image: &RgbaImage, before: &RgbaImage) -> usize {
        image
            .pixels()
            .zip(before.pixels())
            .filter(|(a, b)| a.0 != b.0)
            .count()
    }

    #[test]
    fn uniform_erase_fills_the_box_and_nothing_out_it() {
        let rect = PixelRect::new(10.0, 10.0, 10.0, 10.0);
        let mut image = canvas(30, 30, DARK);
        // 框内先塞两个"字"：擦除要能把它们全部盖掉
        image.put_pixel(12, 12, INK);
        image.put_pixel(17, 18, INK);
        let before = image.clone();

        erase(
            &mut image,
            &rect,
            &estimate(BackgroundKind::Uniform, WHITE, [DARK; 4]),
        );

        for y in 10..20 {
            for x in 10..20 {
                assert_eq!(image.get_pixel(x, y), &WHITE, "框内 ({x},{y}) 没擦干净");
            }
        }
        // 采样带那一圈一个像素都不动：改了它，第二个块的 corners 会读到假背景
        assert_eq!(changed(&image, &before), 10 * 10, "只该动框内那 100 格");
        for edge in [(9, 15), (20, 15), (15, 9), (15, 20)] {
            assert_eq!(
                image.get_pixel(edge.0, edge.1),
                &DARK,
                "带上的 {edge:?} 被动了"
            );
        }
    }

    #[test]
    fn gradient_erase_stays_a_gradient_and_keeps_the_corner_roles() {
        let rect = PixelRect::new(5.0, 5.0, 10.0, 10.0);
        // 左黑右白的横向渐变：上下两对角同色，所以插值只随 x 变
        let corners = [DARK, WHITE, DARK, WHITE];
        let mut image = canvas(25, 25, DARK);
        erase(
            &mut image,
            &rect,
            &estimate(BackgroundKind::Gradient, DARK, corners),
        );

        let row: Vec<u8> = (5..15).map(|x| image.get_pixel(x, 9).0[0]).collect();
        assert_eq!(row[0], 10, "左列该等于左上的角色");
        assert_eq!(row[9], 250, "右列该等于右上的角色");
        assert!(
            row.windows(2).all(|pair| pair[1] >= pair[0]),
            "中间不是单调过渡：{row:?}"
        );
        // SHALL NOT 用一条均值色糊过去：一行里出现的色数就是证据
        let distinct: std::collections::HashSet<u8> = row.iter().copied().collect();
        assert!(distinct.len() >= 8, "渐变被填成了 {} 条色", distinct.len());
        // 这一对角不随 y 变（横向渐变），所以同一列上下两格同色
        assert_eq!(image.get_pixel(8, 6), image.get_pixel(8, 13));
    }

    #[test]
    fn feather_blurs_only_the_ring_and_the_mean_is_hand_computable() {
        let rect = PixelRect::new(5.0, 5.0, 10.0, 10.0);
        let mut image = canvas(25, 25, DARK);
        // 四角都是白的 ⇒ 框内填成全白。圈上 (4,9) 的 3×3 里只有一列（x=5）在框内，
        // 于是它的均值是 (3×250 + 6×10) / 9 = 90——这个数手算得出，所以它锁得住实现。
        erase(
            &mut image,
            &rect,
            &estimate(BackgroundKind::Gradient, DARK, [WHITE; 4]),
        );

        assert_eq!(
            image.get_pixel(4, 9),
            &Rgba([90, 90, 90, 255]),
            "圈上的值不是 3×3 均值"
        );
        assert_eq!(
            image.get_pixel(15, 9),
            &Rgba([90, 90, 90, 255]),
            "右半边同理"
        );
        // 隔两格以外不许动
        assert_eq!(image.get_pixel(3, 9), &DARK);
        assert_eq!(image.get_pixel(16, 9), &DARK);
        // 圈内不许被再平均一次：那是刚填好的渐变本身
        assert_eq!(image.get_pixel(5, 5), &WHITE);
        assert_eq!(image.get_pixel(14, 14), &WHITE);
    }

    #[test]
    fn complex_erase_writes_a_single_pixel_less_than_nothing() {
        let mut image = canvas(20, 20, DARK);
        image.put_pixel(9, 9, WHITE);
        let before = image.clone();
        erase(
            &mut image,
            &PixelRect::new(5.0, 5.0, 10.0, 10.0),
            &estimate(BackgroundKind::Complex, DARK, [DARK; 4]),
        );
        assert_eq!(changed(&image, &before), 0, "Complex 档动了一个像素");
    }

    #[test]
    fn a_rect_covering_no_pixels_writes_nothing() {
        // 0 宽的框（Vision 偶发的退化框）、完全在图外的框、整个在负坐标的框：三样都不许越界写
        let mut image = canvas(12, 12, DARK);
        let before = image.clone();
        for rect in [
            PixelRect::new(4.0, 4.0, 0.0, 10.0),
            PixelRect::new(40.0, 40.0, 10.0, 10.0),
            PixelRect::new(-6.0, -6.0, 3.0, 3.0),
        ] {
            erase(
                &mut image,
                &rect,
                &estimate(BackgroundKind::Uniform, WHITE, [DARK; 4]),
            );
            erase(
                &mut image,
                &rect,
                &estimate(BackgroundKind::Gradient, DARK, [WHITE; 4]),
            );
            overlay_plate(&mut image, &rect, WHITE);
        }
        assert_eq!(changed(&image, &before), 0, "空区间写进了像素");
    }

    #[test]
    fn the_plate_covers_the_box_and_leaves_everything_else_byte_identical() {
        let rect = PixelRect::new(2.0, 2.0, 6.0, 6.0);
        // 透明底：底板要是不透明的白，而不是与 α=0 混出来的东西
        let mut image = canvas(12, 12, Rgba([0, 0, 0, 0]));
        overlay_plate(&mut image, &rect, WHITE);
        for y in 2..8 {
            for x in 2..8 {
                assert_eq!(image.get_pixel(x, y), &WHITE);
            }
        }
        assert_eq!(image.get_pixel(1, 4), &Rgba([0, 0, 0, 0]), "板外一格被动了");
        assert_eq!(image.get_pixel(8, 4), &Rgba([0, 0, 0, 0]), "板外一格被动了");
    }

    #[test]
    fn blending_is_src_over_including_the_alpha_channel() {
        // 不透明底 + 半覆盖 ⇒ 恰好是两色中点，alpha 仍是全不透明
        let mut opaque = Rgba([0, 0, 0, 255]);
        blend(&mut opaque, WHITE, 0.5);
        assert_eq!(opaque, Rgba([125, 125, 125, 255]));

        // 满覆盖 ⇒ 就是字色本身，不留下底下那层
        let mut dark = Rgba([0, 0, 0, 255]);
        blend(&mut dark, WHITE, 1.0);
        assert_eq!(dark, WHITE);

        // 透明底 + 满覆盖的字 ⇒ 那一格从此有这个字的颜色与不透明度（透明 PNG 上回填的常见情形）
        let mut clear = Rgba([7, 7, 7, 0]);
        blend(&mut clear, DARK, 1.0);
        assert_eq!(clear, DARK);

        // 零覆盖：整格不动，也不把 α 从 0 抬起来
        let mut untouched = Rgba([7, 7, 7, 0]);
        blend(&mut untouched, WHITE, 0.0);
        assert_eq!(untouched, Rgba([7, 7, 7, 0]));
    }

    /// 只有 mac 编译面一定有那两支字体；Linux 上 `open()` 会如实报 `Font`，字形的事测不了（NFR-7 只承诺 S 档）
    #[cfg(target_os = "macos")]
    mod heiti {
        use super::*;

        /// 字号 20 ⇒ 行盒 25。这一条把"行高用比例而不是字面上下伸部"钉在真字体上。
        fn laid(text: &str, kind: PlanKind) -> BlockPlan {
            one_line(text, PixelRect::new(0.0, 0.0, 200.0, 25.0), kind)
        }

        #[test]
        fn the_open_handle_is_the_only_place_a_missing_font_becomes_an_error() {
            assert!(FileMetrics::open().is_ok());
            // 空字节、乱 face 号都只该变成一条 Font 错误，不许 panic
            assert!(matches!(
                face_from(&[], 0, "空的"),
                Err(VisionError::Font(_))
            ));
            assert!(matches!(
                face_from(heiti_bytes(), 9_999, HEITI_PATH),
                Err(VisionError::Font(_))
            ));
        }

        #[test]
        fn a_han_glyph_is_measured_and_drawn_with_the_same_ruler() {
            let metrics = FileMetrics::open().expect("本机有黑体-简");
            let one = metrics.text_width("文", 20.0);
            assert!(
                (one - 20.0).abs() < 0.6,
                "全角字的推进宽度应等于 em，实测 {one}"
            );
            assert_eq!(metrics.text_width("文文", 20.0), one * 2.0);
            assert!(metrics.text_width("i", 20.0) < one, "窄字占了一个全角的位");
            assert_eq!(metrics.text_width("", 20.0), 0.0);
            assert_eq!(metrics.line_height(20.0), 25.0);

            let mut image = canvas(60, 30, DARK);
            let before = image.clone();
            assert_eq!(
                draw_text(&mut image, &laid("文", PlanKind::InPlace), &metrics),
                0
            );
            assert!(changed(&image, &before) > 0, "一个字都没画出来");

            // 墨必须落在计划那一格里：横向不许越过一个字的推进宽度，纵向不许掉出行盒
            let mut right = 0u32;
            let mut bottom = 0u32;
            let mut left = u32::MAX;
            for (x, y, pixel) in image.enumerate_pixels() {
                if *pixel != DARK {
                    right = right.max(x);
                    bottom = bottom.max(y);
                    left = left.min(x);
                }
            }
            assert!(
                left <= 2 && right <= 21 && bottom <= 25,
                "字跑出了计划那一格：左缘 {left} 右缘 {right} 底缘 {bottom}"
            );
        }

        #[test]
        fn a_glyph_nobody_has_is_counted_not_painted() {
            let metrics = FileMetrics::open().expect("本机有黑体-简");
            let gone = NO_GLYPH.to_string();
            assert_eq!(count_missing("文中没有这个字", &metrics), 0);
            assert_eq!(count_missing(&gone, &metrics), 1);

            // 两支都没有：不画豆腐块、不占位，计数如实带回
            let mut image = canvas(60, 30, DARK);
            let before = image.clone();
            assert_eq!(
                draw_text(&mut image, &laid(&gone, PlanKind::InPlace), &metrics),
                1
            );
            assert_eq!(changed(&image, &before), 0, "缺字形被画成了东西");
        }

        #[test]
        fn skip_draws_nothing_and_the_overlay_layer_still_does() {
            let metrics = FileMetrics::open().expect("本机有黑体-简");
            let mut image = canvas(60, 30, DARK);
            let before = image.clone();
            assert_eq!(
                draw_text(&mut image, &laid("文", PlanKind::Skip), &metrics),
                0
            );
            assert_eq!(changed(&image, &before), 0, "Skip 那块也写了像素");

            // 覆盖层照画：底板由调用方先铺（`overlay_plate`），这里只管字
            assert_eq!(
                draw_text(&mut image, &laid("文", PlanKind::Overlay), &metrics),
                0
            );
            assert!(changed(&image, &before) > 0, "覆盖层那一块没画上字");
        }

        #[test]
        fn the_fallback_face_lets_a_hangul_line_be_measured_and_drawn() {
            let metrics = FileMetrics::open().expect("本机有黑体-简");
            // 黑体没有谚文（`ice_core::font` 模块注释里那条），后备 AppleGothic 有：
            // 两支字体合起来必须量得出宽度、画得出色，否则 ko/ru 那两类图会一片空白。
            let text = "이미지";
            assert_eq!(count_missing(text, &metrics), 0);
            assert!(metrics.text_width(text, 20.0) > 0.0);

            let mut image = canvas(120, 30, DARK);
            let before = image.clone();
            assert_eq!(
                draw_text(&mut image, &laid(text, PlanKind::InPlace), &metrics),
                0
            );
            assert!(changed(&image, &before) > 0, "谚文整行没画出来");
        }
    }
}
