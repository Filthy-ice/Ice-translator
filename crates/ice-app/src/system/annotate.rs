//! 标注层：冻结帧**物理像素**大小的透明 RGBA 光栅，工具往上落像素，橡皮按像素擦。
//!
//! 为什么是光栅不是矢量列表：橡皮要能"擦掉半条框、另半条还在"（用户 2026-11-17 原话
//! "难道不是截图上面加一个图层吗？这样橡皮不就能部分擦除了？"）。矢量列表只能整条删，
//! 落像素之后擦除就是 destination-out 笔刷，擦到哪块哪块透明，天然支持部分擦。
//!
//! 本模块**不碰 egui**：全是纯像素函数，无头测试直接断言落层形状。坐标一律是物理像素
//! （与冻结帧 `frame.image` 同尺寸），手势的点坐标由调用方拿真实 scale 换进来。

use image::RgbaImage;

/// 粗细三档（点）：细 / 中 / 粗。落层时乘那块屏的真实 scale 换成物理像素。
/// 默认中档；档位选择是运行态，不进 config（REQ-003）。
pub const THICKNESS: [f32; 3] = [2.0, 4.0, 7.0];

/// 色板六色，**默认红在最前**（用户 2026-11-17 裁定"默认红+少量可选色"）。
pub const PALETTE: [[u8; 4]; 6] = [
    [220, 40, 40, 255],
    [240, 180, 20, 255],
    [40, 110, 220, 255],
    [40, 170, 80, 255],
    [245, 245, 245, 255],
    [20, 20, 20, 255],
];

/// 一层标注。`revision` 每落一笔加一：显示侧拿它判"要不要重传纹理"（照定格纹理那套缓存读法）。
pub struct Layer {
    pub image: RgbaImage,
    pub revision: u64,
}

impl Layer {
    pub fn new(width: u32, height: u32) -> Self {
        Self {
            image: RgbaImage::new(width, height),
            revision: 0,
        }
    }

    fn touch(&mut self) {
        self.revision = self.revision.wrapping_add(1);
    }

    /// 矩形框：a、b 是对角两点，四条边各落一条线。
    pub fn rect_stroke(&mut self, a: (f32, f32), b: (f32, f32), width: f32, color: [u8; 4]) {
        let (x0, x1) = (a.0.min(b.0), a.0.max(b.0));
        let (y0, y1) = (a.1.min(b.1), a.1.max(b.1));
        for (p, q) in [
            ((x0, y0), (x1, y0)),
            ((x1, y0), (x1, y1)),
            ((x1, y1), (x0, y1)),
            ((x0, y1), (x0, y0)),
        ] {
            paint_segment(&mut self.image, p, q, width / 2.0, color);
        }
        self.touch();
    }

    pub fn line(&mut self, a: (f32, f32), b: (f32, f32), width: f32, color: [u8; 4]) {
        paint_segment(&mut self.image, a, b, width / 2.0, color);
        self.touch();
    }

    /// 带箭头的线：杆 + 两条头斜线。头的几何单独公开（[`arrow_head`]），
    /// 预览矢量与落层读同一份，拖的时候看到的箭头就是松手落下去的那个。
    pub fn arrow(&mut self, from: (f32, f32), to: (f32, f32), width: f32, color: [u8; 4]) {
        paint_segment(&mut self.image, from, to, width / 2.0, color);
        let [h1, h2] = arrow_head(from, to, width);
        paint_segment(&mut self.image, to, h1, width / 2.0, color);
        paint_segment(&mut self.image, to, h2, width / 2.0, color);
        self.touch();
    }

    /// 铅笔：把拖过的点串成折线逐段落像素。
    pub fn pencil(&mut self, points: &[(f32, f32)], width: f32, color: [u8; 4]) {
        for pair in points.windows(2) {
            paint_segment(&mut self.image, pair[0], pair[1], width / 2.0, color);
        }
        if points.len() == 1 {
            paint_segment(&mut self.image, points[0], points[0], width / 2.0, color);
        }
        self.touch();
    }

    /// 橡皮：destination-out 笔刷——只把已有的 alpha 往下减，不往上盖任何颜色。
    /// 擦过的地方变回透明，底下的冻结帧原样露出来；没擦到的笔画一个像素都不动。
    pub fn erase(&mut self, a: (f32, f32), b: (f32, f32), radius: f32) {
        for_each_covered_pixel(&mut self.image, a, b, radius, |img, x, y, cov| {
            let px = img.get_pixel_mut(x as u32, y as u32);
            px[3] = ((px[3] as f32) * (1.0 - cov)).round() as u8;
        });
        self.touch();
    }
}

/// 箭头两条头斜线的落点（相对尖端 to）。杆方向的反向偏转 ±30°，头长随线宽走、
/// 有个下限——细线也得看得见箭头。预览与落层共用这一份几何。
pub fn arrow_head(from: (f32, f32), to: (f32, f32), width: f32) -> [(f32, f32); 2] {
    let dx = to.0 - from.0;
    let dy = to.1 - from.1;
    let len = (dx * dx + dy * dy).sqrt();
    if len < 1e-3 {
        return [to, to];
    }
    let (ux, uy) = (dx / len, dy / len);
    let head = (width * 4.0).max(12.0);
    let cos = 0.866_f32; // cos(30°)
    let sin = 0.5_f32;
    [
        (
            to.0 - head * (ux * cos - uy * sin),
            to.1 - head * (uy * cos + ux * sin),
        ),
        (
            to.0 - head * (ux * cos + uy * sin),
            to.1 - head * (uy * cos - ux * sin),
        ),
    ]
}

/// 把标注层按 alpha 盖进裁图：`x0/y0` 是裁区在整帧里的物理像素左上角。
/// **三出口（钉住/复制/保存）唯一的合成点**在 `state.rs::crop_selection` 调它，
/// 层外的像素一个都不许动（测试逐像素盯这条）。
pub fn composite_into(base: &mut RgbaImage, layer: &RgbaImage, x0: u32, y0: u32) {
    let w = base.width().min(layer.width().saturating_sub(x0));
    let h = base.height().min(layer.height().saturating_sub(y0));
    for y in 0..h {
        for x in 0..w {
            let src = layer.get_pixel(x0 + x, y0 + y);
            if src[3] == 0 {
                continue;
            }
            let dst = base.get_pixel_mut(x, y);
            blend_straight(dst, *src, 1.0);
        }
    }
}

/// straight-alpha 源色按覆盖率盖到目标像素上（目标也是 straight alpha，image crate 的约定）。
fn blend_straight(dst: &mut image::Rgba<u8>, src: image::Rgba<u8>, coverage: f32) {
    let sa = (src[3] as f32 / 255.0) * coverage.clamp(0.0, 1.0);
    if sa <= 0.0 {
        return;
    }
    let da = dst[3] as f32 / 255.0;
    let oa = sa + da * (1.0 - sa);
    for c in 0..3 {
        let s = src[c] as f32 / 255.0;
        let d = dst[c] as f32 / 255.0;
        dst[c] = (((s * sa + d * da * (1.0 - sa)) / oa) * 255.0).round() as u8;
    }
    dst[3] = (oa * 255.0).round() as u8;
}

/// 一条线段按"像素中心到线段的距离"落覆盖率：距离在半宽内全盖，边缘半像素渐变，
/// 出来是抗锯齿的圆头线。框/直线/箭头/铅笔全走这一条，粗细与手感同源。
fn paint_segment(img: &mut RgbaImage, a: (f32, f32), b: (f32, f32), half: f32, color: [u8; 4]) {
    let src = image::Rgba(color);
    for_each_covered_pixel(img, a, b, half, |img, x, y, cov| {
        blend_straight(img.get_pixel_mut(x as u32, y as u32), src, cov);
    });
}

fn for_each_covered_pixel(
    img: &mut RgbaImage,
    a: (f32, f32),
    b: (f32, f32),
    radius: f32,
    mut f: impl FnMut(&mut RgbaImage, i32, i32, f32),
) {
    let min_x = (a.0.min(b.0) - radius - 1.0).floor() as i32;
    let max_x = (a.0.max(b.0) + radius + 1.0).ceil() as i32;
    let min_y = (a.1.min(b.1) - radius - 1.0).floor() as i32;
    let max_y = (a.1.max(b.1) + radius + 1.0).ceil() as i32;
    let (w, h) = (img.width() as i32, img.height() as i32);
    for y in min_y.max(0)..=max_y.min(h - 1) {
        for x in min_x.max(0)..=max_x.min(w - 1) {
            let d = dist_to_segment((x as f32 + 0.5, y as f32 + 0.5), a, b);
            let cov = (radius + 0.5 - d).clamp(0.0, 1.0);
            if cov > 0.0 {
                f(img, x, y, cov);
            }
        }
    }
}

fn dist_to_segment(p: (f32, f32), a: (f32, f32), b: (f32, f32)) -> f32 {
    let ab = (b.0 - a.0, b.1 - a.1);
    let ap = (p.0 - a.0, p.1 - a.1);
    let len2 = ab.0 * ab.0 + ab.1 * ab.1;
    let t = if len2 < 1e-9 {
        0.0
    } else {
        ((ap.0 * ab.0 + ap.1 * ab.1) / len2).clamp(0.0, 1.0)
    };
    let dx = p.0 - (a.0 + ab.0 * t);
    let dy = p.1 - (a.1 + ab.1 * t);
    (dx * dx + dy * dy).sqrt()
}

#[cfg(test)]
mod tests {
    use super::*;

    const RED: [u8; 4] = [220, 40, 40, 255];

    fn alpha(l: &Layer, x: u32, y: u32) -> u8 {
        l.image.get_pixel(x, y)[3]
    }

    #[test]
    fn rect_paints_four_edges_and_leaves_interior_transparent() {
        let mut l = Layer::new(64, 64);
        l.rect_stroke((10.0, 10.0), (50.0, 40.0), 3.0, RED);
        for (x, y) in [(30, 10), (30, 40), (10, 25), (50, 25)] {
            assert!(alpha(&l, x, y) > 200, "边中点 ({x},{y}) 应不透明");
        }
        assert_eq!(alpha(&l, 30, 25), 0, "框内部必须透明");
        assert_eq!(alpha(&l, 5, 5), 0, "框外必须透明");
        assert_eq!(l.revision, 1);
    }

    #[test]
    fn erase_half_a_line_leaves_the_other_half() {
        let mut l = Layer::new(64, 64);
        l.line((10.0, 32.0), (54.0, 32.0), 4.0, RED);
        l.erase((10.0, 32.0), (30.0, 32.0), 6.0);
        assert_eq!(alpha(&l, 20, 32), 0, "擦过的那半必须全透明");
        assert!(alpha(&l, 45, 32) > 200, "没擦到的那半必须还在");
        assert_eq!(l.revision, 2);
    }

    #[test]
    fn composite_leaves_background_untouched_where_layer_is_transparent() {
        let mut base = RgbaImage::from_pixel(64, 64, image::Rgba([30, 60, 120, 255]));
        let mut l = Layer::new(64, 64);
        l.rect_stroke((10.0, 10.0), (50.0, 40.0), 3.0, RED);
        composite_into(&mut base, &l.image, 0, 0);
        assert_eq!(
            *base.get_pixel(30, 25),
            image::Rgba([30, 60, 120, 255]),
            "层透明处背景必须逐像素不变"
        );
        assert_ne!(*base.get_pixel(30, 10), image::Rgba([30, 60, 120, 255]));
    }

    #[test]
    fn composite_offset_only_touches_the_crop_region() {
        let mut base = RgbaImage::from_pixel(16, 16, image::Rgba([9, 9, 9, 255]));
        let mut l = Layer::new(64, 64);
        l.line((20.0, 20.0), (40.0, 20.0), 4.0, RED);
        composite_into(&mut base, &l.image, 16, 16);
        assert!(base.get_pixel(8, 4)[0] > 100, "裁区内 (20-16,20-16) 应带红");
        assert_eq!(*base.get_pixel(0, 0), image::Rgba([9, 9, 9, 255]));
    }

    #[test]
    fn arrow_head_is_symmetric_and_shaft_reaches_the_tip() {
        let from = (10.0, 10.0);
        let to = (50.0, 50.0);
        let [h1, h2] = arrow_head(from, to, 3.0);
        let d1 = ((h1.0 - to.0).powi(2) + (h1.1 - to.1).powi(2)).sqrt();
        let d2 = ((h2.0 - to.0).powi(2) + (h2.1 - to.1).powi(2)).sqrt();
        assert!((d1 - d2).abs() < 1e-3, "两条头斜线必须等长");
        assert!(d1 > 10.0, "头长要有下限");
        let mut l = Layer::new(64, 64);
        l.arrow(from, to, 3.0, RED);
        assert!(alpha(&l, 48, 48) > 200, "尖端附近必须有墨");
        assert!(alpha(&l, 30, 30) > 200, "杆中点必须有墨");
    }

    #[test]
    fn pencil_paints_every_vertex_of_the_polyline() {
        let mut l = Layer::new(64, 64);
        l.pencil(&[(5.0, 5.0), (20.0, 30.0), (40.0, 10.0)], 3.0, RED);
        for (x, y) in [(5, 5), (20, 30), (40, 10)] {
            assert!(alpha(&l, x, y) > 200, "折线顶点 ({x},{y}) 应有墨");
        }
    }

    #[test]
    fn erase_on_empty_layer_changes_nothing_visible() {
        let mut l = Layer::new(32, 32);
        l.erase((0.0, 0.0), (31.0, 31.0), 8.0);
        assert!(l.image.pixels().all(|p| p[3] == 0));
    }
}
