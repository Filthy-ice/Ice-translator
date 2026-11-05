//! S5 背景估计：一块文字"底下是什么颜色、能不能就地擦掉"（CHG-006 REQ-002）。
//!
//! 这个模块**零 IO、零 `unsafe`、不碰任何 UI crate**（NFR-6），于是三档判定这件事能在无头里
//! 逐条钉死——它也是回填管线里唯一"只看像素、不改像素"的一段：正因为不改像素，判据才全部可测。
//!
//! 三件不显然的事：
//!
//! 1. **采样带不含框内**。把文字本身算进"背景"，深色模式截图必错（`docs/06` 的 IMG-6 专打这个）：
//!    黑底白字的框内一半像素是白的，众数会被字形拉走，擦出来就是一块灰斑。
//! 2. **档位边界归哪一档是定死的**，不留给人再解释一遍：`0.15` 属 `Gradient`（"≥"那一侧）、
//!    `0.35` 也属 `Gradient`（`Complex` 用严格大于）。绘制侧不许再定义一次。
//! 3. **拿不准的方向永远是"不擦"**。带内样本太少、贴角只剩一条线，判 `Complex`——擦错了没人能把
//!    原图那块像素找回来，不擦最多是原文还留在那儿（ADR-003 的同一兜底方向）。

use ice_core::document::PixelRect;
use image::{Rgba, RgbaImage};

use crate::composite::CompositeOptions;

/// 带内像素与 `bg` 的切比雪夫距离超过这个数，就算"不是背景"
pub const BG_DISTANCE_THRESHOLD: i64 = 12;
/// 框内取前景那一簇的下界：距离 ≥ 框内最大距离 × 这个比例
pub const FG_CLUSTER_RATIO: f64 = 0.6;
/// 采样带至少要这么多像素，不足判 `Complex`
pub const MIN_BAND_PIXELS: usize = 16;
/// 占比低于它 = 纯色背景
pub const UNIFORM_VARIANCE_MAX: f64 = 0.15;
/// 占比高于它 = 复杂背景（等于它仍属 `Gradient`）
pub const COMPLEX_VARIANCE_MIN: f64 = 0.35;
/// 角均值的取样半径：1 → 3×3
pub const CORNER_RADIUS: i64 = 1;
/// 像素栅格上的闭开区间 `(x0, y0, x1, y1)`，`x1/y1` 不含
pub type Span = (i64, i64, i64, i64);

/// 量化：每通道取高 4bit，RGB 共 16³ = 4096 桶。用定长数组而不是 `HashMap`——除了快，
/// 更要紧的是**迭代序不是哈希序**，"同分时取先遇到的那个"才有确定含义。
const BUCKETS: usize = 4096;
const QUANT_SHIFT: u8 = 4;

fn bucket(pixel: &Rgba<u8>) -> usize {
    (((pixel.0[0] as usize) >> QUANT_SHIFT) << 8)
        | (((pixel.0[1] as usize) >> QUANT_SHIFT) << 4)
        | ((pixel.0[2] as usize) >> QUANT_SHIFT)
}

/// 两个颜色的切比雪夫距离（最大通道差）：按人眼的方式判"差多少"——
/// 一个通道差得远就显眼，不必三通道都差。`typeset` 判框内哪一圈是字用的也是它，
/// 于是"什么算跟背景不一样"全仓只有这一条判据。
pub(crate) fn distance(a: &Rgba<u8>, b: &Rgba<u8>) -> i64 {
    (0..3)
        .map(|i| (a.0[i] as i64 - b.0[i] as i64).abs())
        .max()
        .unwrap_or(0)
}

/// alpha 权重。透明像素按 `1/255` 而不是 `0` 计：一整带全透明（透明底 PNG 是 `docs/06`
/// 五类用例之一）时若都按 0 计，所有桶权重同为 0，众数就选不出任何东西；按 1 计仍能选出桶，
/// 桶内均值退化成普通均值，alpha 如实是 0。
fn weight(pixel: &Rgba<u8>) -> f64 {
    f64::from(pixel.0[3].max(1)) / 255.0
}

/// 一个量化桶的累加器。颜色取桶内加权均值而不是桶心还原：这样"纯色带的 `bg` 等于那个色"
/// 是字面成立，而不是"差 8 以内"。
#[derive(Default, Clone, Copy)]
struct Stat {
    w: f64,
    rs: f64,
    gs: f64,
    bs: f64,
    asum: f64,
    count: u64,
    /// 行主序扫描里第一次撞进这个桶的位置，同分时的裁决依据
    first: usize,
}

impl Stat {
    fn add(&mut self, pixel: &Rgba<u8>) {
        let w = weight(pixel);
        self.w += w;
        self.rs += w * f64::from(pixel.0[0]);
        self.gs += w * f64::from(pixel.0[1]);
        self.bs += w * f64::from(pixel.0[2]);
        self.asum += w * f64::from(pixel.0[3]);
        self.count += 1;
    }

    /// `add` 过就有 `w > 0`（权重下界 1/255），所以这里不必再兜一次 0 除。
    fn color(&self) -> Rgba<u8> {
        Rgba([
            channel(self.rs, self.w),
            channel(self.gs, self.w),
            channel(self.bs, self.w),
            channel(self.asum, self.w),
        ])
    }
}

fn channel(sum: f64, divisor: f64) -> u8 {
    (sum / divisor).clamp(0.0, 255.0).round() as u8
}

/// 背景能不能就地擦
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BackgroundKind {
    /// 纯色：`bg` 实心填框内
    Uniform,
    /// 渐变/缓变：四个 `corners` 双线性插值填框内
    Gradient,
    /// 照片、噪声、样本只剩一条边：**一个像素都不擦**
    Complex,
}

/// 一个块的背景证据。全部在**擦除之前**算完，`corners` 尤其——后擦的块会把先擦的块的角读成
/// 假背景（两框重叠是真实场景，不是防御性假设，所以合成侧要先批量估计再动手改像素）。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BackgroundEstimate {
    /// 采样带的量化众数（桶内加权均值），alpha 如实带出来
    pub bg: Rgba<u8>,
    /// 框内前景那一簇的众数——回填的字号颜色从这里取（FR-2.8 的"白字仍是白"可自动那半）
    pub fg: Rgba<u8>,
    /// 带内与 `bg` 距离超过 [`BG_DISTANCE_THRESHOLD`] 的像素占比
    pub variance: f64,
    pub kind: BackgroundKind,
    /// 采样带外框的四角各 3×3 均值：左上、右上、左下、右下
    pub corners: [Rgba<u8>; 4],
}

/// 占比 → 档位。边界两处都归 `Gradient`，所以这里是 `<` 与 `<=` 的不对称写法。
pub const fn classify(variance: f64) -> BackgroundKind {
    if variance < UNIFORM_VARIANCE_MAX {
        BackgroundKind::Uniform
    } else if variance <= COMPLEX_VARIANCE_MIN {
        BackgroundKind::Gradient
    } else {
        BackgroundKind::Complex
    }
}

/// 估计一块框的背景。**不返回错误**：贴边就把外框夹进图像继续，样本不足判 `Complex`。
pub fn estimate(
    image: &RgbaImage,
    rect: &PixelRect,
    options: &CompositeOptions,
) -> BackgroundEstimate {
    // 设计里"按 padding/2 再试一次"那一支没落码：外扩量越小那一圈越窄，样本数只减不增，
    // 重试永远救不回来。直接走"仍不足判 Complex"，逐位结果与写了重试的完全一致。
    let (band, outer) = band_pixels(image, rect, options.padding.max(0.0));
    let inner = inset(image, rect);
    let (bg, variance) = if band.is_empty() {
        // 连一条样本都没有（整张图就是那块框）：`bg` 只能给框内均值，占比记满 1.0，
        // 档位随后被下面的样本数压成 `Complex`——数字不撒谎，方向是"不擦"
        (span_mean(image, inner), 1.0)
    } else {
        let bg = mode_of(&band);
        let outliers = band
            .iter()
            .filter(|pixel| distance(pixel, &bg) > BG_DISTANCE_THRESHOLD)
            .count();
        (bg, outliers as f64 / band.len() as f64)
    };
    let kind = if band.len() < MIN_BAND_PIXELS {
        BackgroundKind::Complex
    } else {
        classify(variance)
    };

    BackgroundEstimate {
        bg,
        fg: foreground(image, inner, bg),
        variance,
        kind,
        corners: corners_of(image, outer),
    }
}

/// 框内**首行**文字的横向范围 `(左缘, 右缘)`（像素栅格坐标，右缘不含）。
///
/// 为什么这件事归看像素的这一层：REQ-004 那句"依据首行左边界与块框中心的相对位置推断左/中/右"
/// 里的"首行左边界"只有看像素才拿得到——`TextBlock` 存的是**整块**的外接框，行与行的几何在
/// CHG-005 合并成块之后就没了（`source_rows` 只留行号）。返回值交给纯函数
/// `typeset::infer_align`，于是对齐判据本身仍能在无头里用四个数钉死。
///
/// 判"这是字"沿用同一个 [`BG_DISTANCE_THRESHOLD`]：与 `bg` 的切比雪夫距离超过它就算墨迹。
/// 从最上面一行有墨迹的行开始，连续往下走到第一条没有墨迹的行为止。
/// 返回 `None` 表示框内一个墨迹像素都没有（空白块），调用方按左对齐处理。
pub fn first_line_extent(image: &RgbaImage, rect: &PixelRect, bg: Rgba<u8>) -> Option<(f32, f32)> {
    let (x0, y0, x1, y1) = inset(image, rect);
    let mut extent: Option<(f32, f32)> = None;
    for y in y0..y1 {
        let row = ink_extent_in_row(image, x0, x1, y, bg);
        match (row, extent) {
            (Some(row), Some((left, right))) => extent = Some((left.min(row.0), right.max(row.1))),
            (Some(row), None) => extent = Some(row),
            // 已经起手、这一行却空了：首行到此为止
            (None, Some(_)) => break,
            (None, None) => {}
        }
    }
    extent
}

/// 一行里墨迹的 `(最左, 最右 + 1)`；整行没有墨迹返回 `None`
fn ink_extent_in_row(
    image: &RgbaImage,
    x0: i64,
    x1: i64,
    y: i64,
    bg: Rgba<u8>,
) -> Option<(f32, f32)> {
    let mut row: Option<(f32, f32)> = None;
    for x in x0..x1 {
        let pixel = *image.get_pixel(x as u32, y as u32);
        if distance(&pixel, &bg) > BG_DISTANCE_THRESHOLD {
            row = match row {
                None => Some((x as f32, (x + 1) as f32)),
                Some((left, right)) => Some((left, right.max((x + 1) as f32))),
            };
        }
    }
    row
}

/// 框内那一簇前景的众数：离 `bg` 最远的那批里再取多数。
///
/// 先求框内最大距离、再按 `≥ 最大距离 × FG_CLUSTER_RATIO` 取一簇，而不是写死一个距离阈值：
/// 深灰底浅灰字（差 30）与白底黑字（差 255）用同一个比例都能选出字色，写死阈值就选不出前者。
fn foreground(image: &RgbaImage, inner: Span, bg: Rgba<u8>) -> Rgba<u8> {
    let mut pixels: Vec<(Rgba<u8>, i64)> = Vec::new();
    for y in inner.1..inner.3 {
        for x in inner.0..inner.2 {
            let pixel = *image.get_pixel(x as u32, y as u32);
            pixels.push((pixel, distance(&pixel, &bg)));
        }
    }
    let farthest = pixels.iter().map(|(_, d)| *d).max().unwrap_or(0);
    if farthest == 0 {
        // 框内和背景一模一样（空白块）：前景就是背景，合成侧不该把它当"字色"用
        return bg;
    }
    let floor = farthest as f64 * FG_CLUSTER_RATIO;
    let cluster: Vec<Rgba<u8>> = pixels
        .into_iter()
        .filter(|(_, d)| *d as f64 >= floor)
        .map(|(pixel, _)| pixel)
        .collect();
    mode_of(&cluster)
}

/// 桶内众数：权重最大的桶；同分时取扫描序里**先遇到**的那个（行主序，因此可复现）
fn mode_of(pixels: &[Rgba<u8>]) -> Rgba<u8> {
    let mut buckets = vec![Stat::default(); BUCKETS];
    for (order, pixel) in pixels.iter().enumerate() {
        let stat = &mut buckets[bucket(pixel)];
        if stat.count == 0 {
            stat.first = order;
        }
        stat.add(pixel);
    }
    let mut best: Option<&Stat> = None;
    for stat in &buckets {
        if stat.count == 0 {
            continue;
        }
        best = match best {
            None => Some(stat),
            Some(winner) if stat.w > winner.w => Some(stat),
            Some(winner) if stat.w == winner.w && stat.first < winner.first => Some(stat),
            Some(winner) => Some(winner),
        };
    }
    // 非空像素表一定落进某个桶：两个调用方都只在取到像素时才进来
    best.map(Stat::color).unwrap_or(Rgba([0, 0, 0, 255]))
}

/// 矩形在像素栅格上的闭开区间，已夹进图像范围。
///
/// 取 `floor`/`ceil` 而不是 `round`：估计的框必须**盖住** Vision 给的那块文字，宁可多算一圈也不
/// 许少算一行——少算的那一行擦不掉，会变成译文旁边留着半个字。`typeset` 找首行墨迹用的也是这
/// 个区间，两处对"框覆盖哪些像素"不许有两种说法。
pub(crate) fn inset(image: &RgbaImage, rect: &PixelRect) -> Span {
    let (w, h) = (image.width() as f32, image.height() as f32);
    let x0 = rect.x.floor().max(0.0).min(w) as i64;
    let y0 = rect.y.floor().max(0.0).min(h) as i64;
    let x1 = (rect.x + rect.w).ceil().min(w).max(x0 as f32) as i64;
    let y1 = (rect.y + rect.h).ceil().min(h).max(y0 as f32) as i64;
    (x0, y0, x1, y1)
}

/// 外扩 `padding` 那一圈（不含框内），连同外框一起返回
fn band_pixels(image: &RgbaImage, rect: &PixelRect, padding: f32) -> (Vec<Rgba<u8>>, Span) {
    let inner = inset(image, rect);
    let (w, h) = (image.width() as i64, image.height() as i64);
    let pad = padding.round().max(0.0) as i64;
    let outer = (
        (inner.0 - pad).max(0),
        (inner.1 - pad).max(0),
        (inner.2 + pad).min(w),
        (inner.3 + pad).min(h),
    );
    let mut band = Vec::new();
    for y in outer.1..outer.3 {
        for x in outer.0..outer.2 {
            let inside_box = x >= inner.0 && x < inner.2 && y >= inner.1 && y < inner.3;
            if !inside_box {
                band.push(*image.get_pixel(x as u32, y as u32));
            }
        }
    }
    (band, outer)
}

/// 外框四角各 3×3 均值（左上、右上、左下、右下）——`Gradient` 双线性填充要的这四个数。
fn corners_of(image: &RgbaImage, outer: Span) -> [Rgba<u8>; 4] {
    let (x0, y0, x1, y1) = outer;
    // 外框退化成 0 宽时（整张图就是那块框）四个角落在同一像素上，是如实结果不是错
    let right = (x1 - 1).max(x0);
    let bottom = (y1 - 1).max(y0);
    [
        corner_mean(image, x0, y0),
        corner_mean(image, right, y0),
        corner_mean(image, x0, bottom),
        corner_mean(image, right, bottom),
    ]
}

/// 以 `(cx, cy)` 为中心、半径 [`CORNER_RADIUS`] 的方块均值（贴边时自己收缩）。
///
/// `paint` 羽化擦除接缝那一圈用的也是它——全仓只许有一种"邻域 3×3 均值"。
pub(crate) fn corner_mean(image: &RgbaImage, cx: i64, cy: i64) -> Rgba<u8> {
    let (w, h) = (image.width() as i64, image.height() as i64);
    if w == 0 || h == 0 {
        return Rgba([0, 0, 0, 255]);
    }
    let x0 = (cx - CORNER_RADIUS).clamp(0, w - 1);
    let y0 = (cy - CORNER_RADIUS).clamp(0, h - 1);
    let x1 = (cx + CORNER_RADIUS).clamp(0, w - 1) + 1;
    let y1 = (cy + CORNER_RADIUS).clamp(0, h - 1) + 1;
    span_mean(image, (x0, y0, x1, y1))
}

/// 一段区间内像素的普通均值；空区间（0×0 的图、框完全在图外）给不透明的黑
fn span_mean(image: &RgbaImage, span: Span) -> Rgba<u8> {
    let mut sum = [0.0f64; 4];
    let mut count = 0u64;
    for y in span.1..span.3 {
        for x in span.0..span.2 {
            let pixel = image.get_pixel(x as u32, y as u32);
            for (slot, value) in sum.iter_mut().zip(&pixel.0) {
                *slot += f64::from(*value);
            }
            count += 1;
        }
    }
    if count == 0 {
        return Rgba([0, 0, 0, 255]);
    }
    Rgba(sum.map(|value| channel_round(value, count)))
}

fn channel_round(sum: f64, count: u64) -> u8 {
    (sum / count as f64).clamp(0.0, 255.0).round() as u8
}

/// WCAG 相对亮度。REQ-006 的覆盖层底色用同一个函数，不在绘制侧再写一份公式。
pub fn relative_luminance(color: &Rgba<u8>) -> f64 {
    let [r, g, b, _] = color.0;
    let linear = |v: u8| {
        let v = f64::from(v) / 255.0;
        if v <= 0.04045 {
            v / 12.92
        } else {
            ((v + 0.055) / 1.055).powf(2.4)
        }
    };
    0.2126 * linear(r) + 0.7152 * linear(g) + 0.0722 * linear(b)
}

/// WCAG 对比度，恒 ≥ 1
pub fn contrast(a: &Rgba<u8>, b: &Rgba<u8>) -> f64 {
    let (la, lb) = (relative_luminance(a), relative_luminance(b));
    let (lighter, darker) = if la >= lb { (la, lb) } else { (lb, la) };
    (lighter + 0.05) / (darker + 0.05)
}

#[cfg(test)]
mod tests {
    //! 三档判定的无头锁。每条都只喂合成像素、只看返回值，一次 IO 都没有——
    //! 于是"深色截图不会被擦成灰斑"这件事从"要肉眼验"降级成"跑一次测试"。
    use super::*;

    const DARK: Rgba<u8> = Rgba([12, 16, 22, 255]);
    const WHITE: Rgba<u8> = Rgba([255, 255, 255, 255]);

    fn solid(w: u32, h: u32, color: Rgba<u8>) -> RgbaImage {
        RgbaImage::from_pixel(w, h, color)
    }

    fn gray(v: u8) -> Rgba<u8> {
        Rgba([v, v, v, 255])
    }

    fn options(padding: f32) -> CompositeOptions {
        CompositeOptions {
            padding,
            ..Default::default()
        }
    }

    /// 三个参数摊平成"这块框、这点外扩"，调用点短到一行读完
    fn probe(image: &RgbaImage, rect: [f32; 4], padding: f32) -> BackgroundEstimate {
        let [x, y, w, h] = rect;
        estimate(image, &PixelRect::new(x, y, w, h), &options(padding))
    }

    /// 把框内画成竖条纹白字：框内近半像素是白的，剩下的才是被测对象。
    fn striped(mut image: RgbaImage, x: u32, y: u32, w: u32, h: u32) -> RgbaImage {
        for row in y..y + h {
            for column in (x..x + w).step_by(2) {
                image.put_pixel(column, row, WHITE);
            }
        }
        image
    }

    /// 纯色带 → `Uniform`，而且 `bg` **等于**那个色（不是"差 8 以内"）：桶内取加权均值
    /// 而不是桶心还原就是为了这一条。四角同理，四角是 `Gradient` 唯一的填充原料。
    #[test]
    fn a_uniform_band_returns_the_exact_color() {
        let color = Rgba([37, 89, 201, 255]);
        let image = solid(60, 40, color);
        let got = probe(&image, [20.0, 15.0, 20.0, 10.0], 2.0);

        assert_eq!(got.kind, BackgroundKind::Uniform);
        assert_eq!(got.bg, color);
        assert_eq!(got.variance, 0.0);
        assert_eq!(got.corners, [color; 4]);
    }

    /// 白字写在深底上：`bg` 还是深底（采样带不含框内这一条的全部意义）、`fg` 还是白
    /// （FR-2.8 验收"回填后文字颜色仍为白"里**可自动的那一半**，另一半"真图上"仍属 W-4.4）。
    #[test]
    fn white_on_dark_reads_the_background_and_the_ink_apart() {
        let image = striped(solid(60, 40, DARK), 20, 15, 20, 10);
        let got = probe(&image, [20.0, 15.0, 20.0, 10.0], 2.0);

        assert_eq!(got.bg, DARK);
        assert_eq!(got.fg, WHITE);
        assert_eq!(got.kind, BackgroundKind::Uniform);
    }

    /// 反过来的浅底深字也选出字色：`≥ 最大距离 × 0.6` 是比例，两头都成立；
    /// 写死一个距离阈值就只能选中一头。
    #[test]
    fn black_on_light_reads_back_as_black() {
        let mut image = solid(60, 40, WHITE);
        for row in 15..25u32 {
            for column in (20..40u32).step_by(3) {
                image.put_pixel(column, row, DARK);
            }
        }
        let got = probe(&image, [20.0, 15.0, 20.0, 10.0], 2.0);

        assert_eq!(got.fg, DARK);
    }

    /// 缓变梯度 → `Gradient`，占比落在中段。
    ///
    /// 构造：横向每 2px 升一级（`100 + x/2`），块框贴左、外扩 16px 后采样带覆盖 `x 0..52`
    /// 即 26 个灰阶，跨两个量化桶——第二桶（112..125 那 14 级）比第一桶（100..111 那 12 级）多，
    /// `bg = 119`，只有左端 100..106 这 7 级差过 12：占比 = 448/1728 ≈ 0.26，夹在 0.15 与 0.35
    /// 之间。梯度再陡一档就成 `Complex`，那是"该改覆盖层"的信号，不是这条用例。
    #[test]
    fn a_gentle_ramp_lands_in_the_gradient_tier() {
        let mut image = RgbaImage::new(100, 40);
        for y in 0..40u32 {
            for x in 0..100u32 {
                image.put_pixel(x, y, gray(100 + (x / 2) as u8));
            }
        }
        let got = probe(&image, [0.0, 18.0, 36.0, 4.0], 16.0);

        assert_eq!(got.kind, BackgroundKind::Gradient);
        assert!(
            (0.15..=0.35).contains(&got.variance),
            "占比该落在梯度档：{}",
            got.variance
        );
        assert!(
            (112..=126).contains(&got.bg[0]),
            "众数该落在梯度中段：{:?}",
            got.bg
        );
        assert!(got.corners[0][0] < got.corners[1][0], "左上该比右上暗");
        assert!(got.corners[2][0] < got.corners[3][0], "左下该比右下暗");
    }

    /// 棋盘格 → `Complex`：相邻像素黑白交替，一半像素必然与众数差过阈值。
    #[test]
    fn a_checkerboard_is_complex() {
        let mut image = RgbaImage::new(60, 40);
        for y in 0..40u32 {
            for x in 0..60u32 {
                image.put_pixel(x, y, gray(if (x + y) % 2 == 0 { 250 } else { 5 }));
            }
        }
        let got = probe(&image, [20.0, 15.0, 20.0, 10.0], 3.0);

        assert_eq!(got.kind, BackgroundKind::Complex);
        assert!(got.variance > COMPLEX_VARIANCE_MIN);
    }

    /// 两处边界各测两侧，档位在这里写死并归档——之后绘制侧不许再解释一遍。
    #[test]
    fn the_tier_boundaries_are_pinned_on_both_sides() {
        assert_eq!(
            classify(UNIFORM_VARIANCE_MAX - 0.001),
            BackgroundKind::Uniform
        );
        assert_eq!(classify(UNIFORM_VARIANCE_MAX), BackgroundKind::Gradient);
        assert_eq!(classify(COMPLEX_VARIANCE_MIN), BackgroundKind::Gradient);
        assert_eq!(
            classify(COMPLEX_VARIANCE_MIN + 0.001),
            BackgroundKind::Complex
        );
    }

    /// 样本不足优先于占比：带内一片纯色、占比算出来是 0.0，但只有 8 个像素，判 `Complex`。
    #[test]
    fn a_starved_band_is_complex_even_when_it_is_flat() {
        let image = solid(3, 3, DARK);
        let got = probe(&image, [1.0, 1.0, 1.0, 1.0], 2.0);

        assert_eq!(got.kind, BackgroundKind::Complex);
        assert_eq!(got.variance, 0.0, "占比如实是 0，档位仍要压住它");
    }

    /// 整张图就是那块框（外扩后夹回图像内，带为空）：不 panic、不返回错误，`variance` 记满，
    /// `bg` 退化成框内均值——而它因为档位是 `Complex` 不会参与擦除。
    #[test]
    fn a_box_covering_the_whole_image_has_no_band_and_stays_complex() {
        let mut image = RgbaImage::new(8, 8);
        for y in 0..8u32 {
            for x in 0..8u32 {
                image.put_pixel(x, y, gray(if y < 4 { 200 } else { 100 }));
            }
        }
        let got = probe(&image, [0.0, 0.0, 8.0, 8.0], 2.0);

        assert_eq!(got.kind, BackgroundKind::Complex);
        assert_eq!(got.variance, 1.0);
        assert_eq!(got.bg, gray(150));
    }

    /// 贴左缘的块：外框被夹进图像（左边没有 2px 可给）仍取得到一圈样本，正常出档位。
    #[test]
    fn a_box_glued_to_the_left_edge_still_gets_a_band() {
        let image = solid(40, 40, DARK);
        let got = probe(&image, [0.0, 12.0, 10.0, 8.0], 2.0);

        assert_eq!(got.kind, BackgroundKind::Uniform);
        assert_eq!(got.bg, DARK);
    }

    /// alpha 加权：占多数的全是透明像素时它们不该投出背景。
    /// 285 个 alpha=0 的浅像素对 15 个不透明的深像素——按个数浅的赢（19:1），按 alpha 加权
    /// 算深的赢（15 : 1.12）。透明像素不是"这块屏幕底下的颜色"。
    #[test]
    fn a_transparent_majority_does_not_become_the_background() {
        let mut image = solid(40, 40, Rgba([250, 250, 250, 0]));
        for x in 15..30u32 {
            image.put_pixel(x, 10, Rgba([20, 20, 20, 255]));
        }
        let got = probe(&image, [15.0, 15.0, 10.0, 10.0], 5.0);

        assert_eq!(got.bg, Rgba([20, 20, 20, 255]));
        assert_eq!(got.kind, BackgroundKind::Complex, "透明确实没法就地擦");
    }

    /// 同分时取行主序里先遇到的那个桶——把上下两条色带对调，众数跟着换，证明裁决依据是扫描序，
    /// 不是桶号大小也不是哈希序。这一条是"度量可复现"的地基，与 `merge_rows` 的 `total_cmp` 同源。
    #[test]
    fn a_histogram_tie_is_broken_by_row_major_scan_order() {
        // 4×8 的图，块框占中间四行；采样带只剩 y=0,1 与 y=6,7 这两截，各 8 个像素——恰好打平
        let bg_of = |top: u8, bottom: u8| -> Rgba<u8> {
            let mut image = solid(4, 8, gray(top));
            for y in 6..8u32 {
                for x in 0..4u32 {
                    image.put_pixel(x, y, gray(bottom));
                }
            }
            probe(&image, [0.0, 2.0, 4.0, 4.0], 2.0).bg
        };

        assert_eq!(bg_of(200, 220), gray(200), "上带是 200，先被扫到的是它");
        assert_eq!(bg_of(220, 200), gray(220), "上下对调，众数跟着换");
    }

    /// `corners` 读的是带（框外），所以只改框内动不到四角；同一张图重复调用逐字段相等（纯函数）。
    /// 反过来也说明合成侧为什么必须**先把所有块的估计算完**再动手擦：先擦的那一块会把相邻块的
    /// 四角改成假背景。
    #[test]
    fn the_corners_come_from_the_band_and_the_call_is_pure() {
        let image = striped(solid(60, 40, DARK), 20, 15, 20, 10);
        let rect = PixelRect::new(20.0, 15.0, 20.0, 10.0);
        let first = estimate(&image, &rect, &options(2.0));

        assert_eq!(
            estimate(&image, &rect, &options(2.0)),
            first,
            "同一张图同一块框，两次调用必须逐字段相等"
        );

        let mut repainted = image.clone();
        for y in 15..25u32 {
            for x in 20..40u32 {
                repainted.put_pixel(x, y, Rgba([200, 30, 30, 255]));
            }
        }
        assert_eq!(
            estimate(&repainted, &rect, &options(2.0)).corners,
            first.corners,
            "只改框内不该动到四角——角在带里"
        );
    }

    /// WCAG 两个函数给 REQ-006 复用，端点值对齐标准：黑 0 / 白 1 / 与顺序无关且恒 ≥ 1。
    #[test]
    fn the_wcag_helpers_hit_the_known_endpoints() {
        assert!((relative_luminance(&WHITE) - 1.0).abs() < 1e-9);
        assert!(relative_luminance(&Rgba([0, 0, 0, 255])).abs() < 1e-9);
        let ratio = contrast(&WHITE, &DARK);
        assert!(ratio > 15.0 && ratio <= 21.0, "白字深底该接近 21:1");
        assert_eq!(contrast(&DARK, &WHITE), ratio);
    }
}
