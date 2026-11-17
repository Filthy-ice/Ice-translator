//! 标注层：冻结帧**物理像素**大小的透明 RGBA 光栅，工具往上落像素，橡皮按像素擦。
//!
//! 为什么是光栅不是矢量列表：橡皮要能"擦掉半条框、另半条还在"（用户 2026-11-17 原话
//! "难道不是截图上面加一个图层吗？这样橡皮不就能部分擦除了？"）。矢量列表只能整条删，
//! 落像素之后擦除就是 destination-out 笔刷，擦到哪块哪块透明，天然支持部分擦。
//!
//! 本模块**不碰 egui**：全是纯像素函数，无头测试直接断言落层形状。坐标一律是物理像素
//! （与冻结帧 `frame.image` 同尺寸），手势的点坐标由调用方拿真实 scale 换进来。

use ice_core::document::PixelRect;
use ice_vision::paint::{FileMetrics, draw_text};
use ice_vision::typeset::{Align, BlockPlan, Metrics, PlanKind, PlannedLine};
use image::RgbaImage;

/// 粗细三档（点）：细 / 中 / 粗。落层时乘那块屏的真实 scale 换成物理像素。
/// 默认中档；档位选择是运行态，不进 config（REQ-003）。
pub const THICKNESS: [f32; 3] = [2.0, 4.0, 7.0];

/// 粗细档的下拉文案，与 [`THICKNESS`] 同序（REQ-003 药丸下拉读这一份）。
pub const THICKNESS_NAMES: [&str; 3] = ["细", "中", "粗"];

/// 序号标半径三档（点）：与粗细同一个档位读法（REQ-003）。默认中档。
pub const BADGE_RADIUS: [f32; 3] = [10.0, 14.0, 18.0];

/// 文字字号三档（点）。默认中档；与序号标共用粗细档位的运行态（REQ-003）。
pub const TEXT_SIZE: [f32; 3] = [14.0, 20.0, 28.0];

/// 马赛克块度（点）：默认一档（REQ-002）。落层乘 scale 换成物理像素块。
pub const MOSAIC_BLOCK_PT: f32 = 12.0;

/// 色板六色，**默认红在最前**（用户 2026-11-17 裁定"默认红+少量可选色"）。
pub const PALETTE: [[u8; 4]; 6] = [
    [220, 40, 40, 255],
    [240, 180, 20, 255],
    [40, 110, 220, 255],
    [40, 170, 80, 255],
    [245, 245, 245, 255],
    [20, 20, 20, 255],
];

/// 色板六色的下拉文案，与 [`PALETTE`] 同序、红在最前（REQ-003）。
pub const PALETTE_NAMES: [&str; 6] = ["红", "黄", "蓝", "绿", "白", "黑"];

/// 撤销前像的一块**整数像素**矩形（层内坐标：左上角 + 宽高）。annotate 不碰 egui，
/// 不用 egui::Rect；落层包围盒夹到层内、按整像素抄前像，撤销时逐像素贴回。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct PxRect {
    pub x: u32,
    pub y: u32,
    pub w: u32,
    pub h: u32,
}

impl PxRect {
    fn bytes(self) -> usize {
        self.w as usize * self.h as usize * 4
    }
}

/// 一笔撤销的前像集合（REQ-001）。单击类工具一个 part；铅笔/橡皮一次拖拽累积多个 part
/// （每段落像素前先抄该段包围盒的前像），松手时整段算一条入栈。
pub struct UndoEntry {
    pub parts: Vec<(PxRect, RgbaImage)>,
}

impl UndoEntry {
    fn bytes(&self) -> usize {
        self.parts.iter().map(|(r, _)| r.bytes()).sum()
    }
}

/// 撤销栈双上限（REQ-001 / Q-003）：条数封 30、总字节封 64MB，超限从栈首丢最旧。
/// 一块 4K 整层约 33MB，整层快照 30 步就是 1GB 扛不住——所以只抄脏矩形前像，不快照整层。
pub const MAX_UNDO_ENTRIES: usize = 30;
pub const MAX_UNDO_BYTES: usize = 64 * 1024 * 1024;

/// 一层标注。`revision` 每落一笔加一：显示侧拿它判"要不要重传纹理"（照定格纹理那套缓存读法）。
pub struct Layer {
    pub image: RgbaImage,
    pub revision: u64,
    /// 已入栈的撤销条（栈顶 = 最近一笔）。
    undo_stack: Vec<UndoEntry>,
    /// 正在拖的那一段（铅笔/橡皮）：松手前累积各段前像，松手整段算一条。`None` = 不在拖拽态，
    /// 此时每个落层入口自成一条入栈。
    pending: Option<Vec<(PxRect, RgbaImage)>>,
}

impl Layer {
    pub fn new(width: u32, height: u32) -> Self {
        Self {
            image: RgbaImage::new(width, height),
            revision: 0,
            undo_stack: Vec::new(),
            pending: None,
        }
    }

    fn touch(&mut self) {
        self.revision = self.revision.wrapping_add(1);
    }

    /// 开始一段拖拽（铅笔/橡皮按下时调）：此后各段落层的前像累积进 pending，不各自入栈。
    pub fn begin_stroke(&mut self) {
        self.pending = Some(Vec::new());
    }

    /// 结束一段拖拽（松手时调）：把累积的各段前像合成一条入栈。空段不入栈、不动 revision。
    pub fn end_stroke(&mut self) {
        if let Some(parts) = self.pending.take() {
            if !parts.is_empty() {
                self.push_entry(UndoEntry { parts });
            }
        }
    }

    /// 撤销最近一笔：弹栈顶，把各 part 前像逐像素贴回，bump revision。空栈返回 false 不动层。
    /// **倒序贴回**：各段前像抄的是"落这一段之前"的层，后段的前像里带着前段的墨；
    /// 正序贴会把前段的墨在重叠区贴回来，倒序让最早那段的空白前像最后盖上去才是真还原。
    pub fn undo(&mut self) -> bool {
        let Some(entry) = self.undo_stack.pop() else {
            return false;
        };
        for (rect, before) in entry.parts.iter().rev() {
            for y in 0..rect.h {
                for x in 0..rect.w {
                    self.image
                        .put_pixel(rect.x + x, rect.y + y, *before.get_pixel(x, y));
                }
            }
        }
        self.touch();
        true
    }

    /// 抄一块包围盒的前像（f32 边界 → 夹到层内的整像素矩形）。落像素**之前**调。
    /// 零宽/零高（包围盒完全在层外）返回一个空 part，撤销时贴回零像素、无副作用。
    fn before_image(&self, bounds: (f32, f32, f32, f32)) -> (PxRect, RgbaImage) {
        let (w, h) = (self.image.width(), self.image.height());
        let x0 = (bounds.0.floor().max(0.0) as u32).min(w);
        let y0 = (bounds.1.floor().max(0.0) as u32).min(h);
        let x1 = (bounds.2.ceil().max(0.0) as u32).min(w);
        let y1 = (bounds.3.ceil().max(0.0) as u32).min(h);
        let (pw, ph) = (x1.saturating_sub(x0), y1.saturating_sub(y0));
        let rect = PxRect {
            x: x0,
            y: y0,
            w: pw,
            h: ph,
        };
        let img = if pw == 0 || ph == 0 {
            RgbaImage::new(0, 0)
        } else {
            image::imageops::crop_imm(&self.image, x0, y0, pw, ph).to_image()
        };
        (rect, img)
    }

    /// 落一笔的前像入栈：拖拽态累积进 pending，否则自成一条。
    fn record(&mut self, part: (PxRect, RgbaImage)) {
        if part.0.w == 0 || part.0.h == 0 {
            return;
        }
        match self.pending.as_mut() {
            Some(parts) => parts.push(part),
            None => self.push_entry(UndoEntry { parts: vec![part] }),
        }
    }

    /// 入栈一条，双上限从栈首丢最旧直到放得下（单条本身就超限时仍入栈——那是唯一能撤的那笔）。
    fn push_entry(&mut self, entry: UndoEntry) {
        let new_bytes = entry.bytes();
        let mut total: usize = self.undo_stack.iter().map(UndoEntry::bytes).sum();
        while !self.undo_stack.is_empty()
            && (self.undo_stack.len() >= MAX_UNDO_ENTRIES || total + new_bytes > MAX_UNDO_BYTES)
        {
            let old = self.undo_stack.remove(0);
            total -= old.bytes();
        }
        self.undo_stack.push(entry);
    }

    /// 落层统一包装：先抄包围盒前像，再落像素，再入栈、bump revision。
    /// 八件工具里"无早退、无返回值"的那几件走它，快照逻辑只此一份（REQ-001）。
    fn stroke(&mut self, bounds: (f32, f32, f32, f32), paint: impl FnOnce(&mut RgbaImage)) {
        let part = self.before_image(bounds);
        paint(&mut self.image);
        self.record(part);
        self.touch();
    }

    /// 矩形框：a、b 是对角两点，四条边各落一条线。
    pub fn rect_stroke(&mut self, a: (f32, f32), b: (f32, f32), width: f32, color: [u8; 4]) {
        let (x0, x1) = (a.0.min(b.0), a.0.max(b.0));
        let (y0, y1) = (a.1.min(b.1), a.1.max(b.1));
        let half = width / 2.0;
        self.stroke(segment_bounds(a, b, half), move |img| {
            for (p, q) in [
                ((x0, y0), (x1, y0)),
                ((x1, y0), (x1, y1)),
                ((x1, y1), (x0, y1)),
                ((x0, y1), (x0, y0)),
            ] {
                paint_segment(img, p, q, half, color);
            }
        });
    }

    pub fn line(&mut self, a: (f32, f32), b: (f32, f32), width: f32, color: [u8; 4]) {
        let half = width / 2.0;
        self.stroke(segment_bounds(a, b, half), move |img| {
            paint_segment(img, a, b, half, color);
        });
    }

    /// 带箭头的线：杆 + 两条头斜线。头的几何单独公开（[`arrow_head`]），
    /// 预览矢量与落层读同一份，拖的时候看到的箭头就是松手落下去的那个。
    pub fn arrow(&mut self, from: (f32, f32), to: (f32, f32), width: f32, color: [u8; 4]) {
        let half = width / 2.0;
        let [h1, h2] = arrow_head(from, to, width);
        // 包围盒要罩住杆与两条头斜线：取 from/to/h1/h2 四点的极值再外扩半宽。
        let bounds = (
            from.0.min(to.0).min(h1.0).min(h2.0) - half - 1.0,
            from.1.min(to.1).min(h1.1).min(h2.1) - half - 1.0,
            from.0.max(to.0).max(h1.0).max(h2.0) + half + 1.0,
            from.1.max(to.1).max(h1.1).max(h2.1) + half + 1.0,
        );
        self.stroke(bounds, move |img| {
            paint_segment(img, from, to, half, color);
            paint_segment(img, to, h1, half, color);
            paint_segment(img, to, h2, half, color);
        });
    }

    /// 铅笔：把拖过的点串成折线逐段落像素。
    pub fn pencil(&mut self, points: &[(f32, f32)], width: f32, color: [u8; 4]) {
        let half = width / 2.0;
        let bounds = points_bounds(points, half);
        let pts = points.to_vec();
        self.stroke(bounds, move |img| {
            for pair in pts.windows(2) {
                paint_segment(img, pair[0], pair[1], half, color);
            }
            if pts.len() == 1 {
                paint_segment(img, pts[0], pts[0], half, color);
            }
        });
    }

    /// 橡皮：destination-out 笔刷——只把已有的 alpha 往下减，不往上盖任何颜色。
    /// 擦过的地方变回透明，底下的冻结帧原样露出来；没擦到的笔画一个像素都不动。
    pub fn erase(&mut self, a: (f32, f32), b: (f32, f32), radius: f32) {
        self.stroke(segment_bounds(a, b, radius), move |img| {
            for_each_covered_pixel(img, a, b, radius, |img, x, y, cov| {
                let px = img.get_pixel_mut(x as u32, y as u32);
                px[3] = ((px[3] as f32) * (1.0 - cov)).round() as u8;
            });
        });
    }

    /// 序号标：实心圆 + 居中白数字（REQ-002）。数字走本机黑体光栅——与译文回填
    /// 同一支字体、同一条缺字形纪律，不联网、不引新字体依赖（停止条件 6）。
    pub fn number_badge(
        &mut self,
        center: (f32, f32),
        radius: f32,
        number: u32,
        color: [u8; 4],
        metrics: &FileMetrics,
    ) {
        let font_size = radius * 1.2;
        let line_h = metrics.line_height(font_size);
        let area = PixelRect::new(
            center.0 - radius,
            center.1 - line_h / 2.0,
            radius * 2.0,
            line_h,
        );
        // 包围盒罩住实心圆与居中数字两块：圆 center±radius、数字 area，取并集再外扩一格。
        let bounds = (
            (center.0 - radius - 1.0).min(area.x),
            (center.1 - radius - 1.0).min(area.y),
            (center.0 + radius + 1.0).max(area.right()),
            (center.1 + radius + 1.0).max(area.bottom()),
        );
        let label = number.to_string();
        self.stroke(bounds, move |img| {
            paint_segment(img, center, center, radius, color);
            stamp_line(
                img,
                area,
                &label,
                font_size,
                Align::Center,
                [255, 255, 255, 255],
                metrics,
            );
        });
    }

    /// 文字：落点起笔写一行（REQ-002）。空白不落、不 touch；返回缺字形数——
    /// 只交数不交内容（NFR-3）。
    pub fn text(
        &mut self,
        at: (f32, f32),
        text: &str,
        font_size: f32,
        color: [u8; 4],
        metrics: &FileMetrics,
    ) -> usize {
        let text = text.trim();
        if text.is_empty() {
            return 0;
        }
        let line_h = metrics.line_height(font_size);
        let width = metrics.text_width(text, font_size);
        let area = PixelRect::new(at.0, at.1, width.max(1.0), line_h);
        let part = self.before_image((
            area.x - 1.0,
            area.y - 1.0,
            area.right() + 1.0,
            area.bottom() + 1.0,
        ));
        let missing = stamp_line(
            &mut self.image,
            area,
            text,
            font_size,
            Align::Left,
            color,
            metrics,
        );
        self.record(part);
        self.touch();
        missing
    }

    /// 马赛克：拖区按块度盒式平均，从冻结背景取色、**不透明**写层（REQ-002）。
    /// 块格对齐区域左上角，一块一色——出来是像素化不是模糊（某截图工具「马赛克」档同读法）。
    /// 零宽/零高的区不落、不 touch。
    pub fn mosaic(&mut self, background: &RgbaImage, a: (f32, f32), b: (f32, f32), block: u32) {
        let block = block.max(1) as i64;
        let (x0, x1) = (a.0.min(b.0).floor() as i64, a.0.max(b.0).ceil() as i64);
        let (y0, y1) = (a.1.min(b.1).floor() as i64, a.1.max(b.1).ceil() as i64);
        if x1 <= x0 || y1 <= y0 {
            return;
        }
        let part = self.before_image((x0 as f32, y0 as f32, x1 as f32, y1 as f32));
        let (bw, bh) = (background.width() as i64, background.height() as i64);
        let (lw, lh) = (self.image.width() as i64, self.image.height() as i64);
        let mut cy = y0;
        while cy < y1 {
            let mut cx = x0;
            while cx < x1 {
                // 采样范围 = 块格 ∩ 拖区 ∩ 背景；边缘块按实际覆盖取平均
                let (sx0, sy0) = (cx.max(0), cy.max(0));
                let (sx1, sy1) = ((cx + block).min(x1).min(bw), (cy + block).min(y1).min(bh));
                if sx1 > sx0 && sy1 > sy0 {
                    let mut sum = [0u64; 3];
                    let mut n = 0u64;
                    for y in sy0..sy1 {
                        for x in sx0..sx1 {
                            let p = background.get_pixel(x as u32, y as u32);
                            for c in 0..3 {
                                sum[c] += p[c] as u64;
                            }
                            n += 1;
                        }
                    }
                    let avg = image::Rgba([
                        (sum[0] / n) as u8,
                        (sum[1] / n) as u8,
                        (sum[2] / n) as u8,
                        255,
                    ]);
                    let (dx1, dy1) = ((cx + block).min(x1).min(lw), (cy + block).min(y1).min(lh));
                    for y in sy0..dy1 {
                        for x in sx0..dx1 {
                            self.image.put_pixel(x as u32, y as u32, avg);
                        }
                    }
                }
                cx += block;
            }
            cy += block;
        }
        self.record(part);
        self.touch();
    }
}

/// 单行文字落层：序号标的数字与文字工具共用这一条（同一支字体、同一条缺字形纪律）。
/// 计划只有一行，对齐与字色由调用方定；绘制走 ice-vision 现成的光栅，本模块不另起一套。
fn stamp_line(
    img: &mut RgbaImage,
    area: PixelRect,
    text: &str,
    font_size: f32,
    align: Align,
    color: [u8; 4],
    metrics: &FileMetrics,
) -> usize {
    let width = metrics.text_width(text, font_size);
    let plan = BlockPlan {
        kind: PlanKind::InPlace,
        font_size,
        lines: vec![PlannedLine {
            text: text.to_string(),
            width,
        }],
        align,
        degraded: false,
        text_color: image::Rgba(color),
        plate: None,
        area,
    };
    draw_text(img, &plan, metrics)
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

/// 一条线段（半径 radius 的圆头笔刷）落像素的包围盒，与 [`for_each_covered_pixel`] 的
/// 遍历范围同式（min−radius−1 .. max+radius+1），撤销前像照这块抄，一个像素不漏。
fn segment_bounds(a: (f32, f32), b: (f32, f32), radius: f32) -> (f32, f32, f32, f32) {
    (
        a.0.min(b.0) - radius - 1.0,
        a.1.min(b.1) - radius - 1.0,
        a.0.max(b.0) + radius + 1.0,
        a.1.max(b.1) + radius + 1.0,
    )
}

/// 一串点（铅笔折线）的包围盒：各点极值外扩半宽 + 一格。空串给一个零盒（不落像素）。
fn points_bounds(points: &[(f32, f32)], half: f32) -> (f32, f32, f32, f32) {
    let Some(first) = points.first() else {
        return (0.0, 0.0, 0.0, 0.0);
    };
    let (mut x0, mut y0, mut x1, mut y1) = (first.0, first.1, first.0, first.1);
    for &(x, y) in points {
        x0 = x0.min(x);
        y0 = y0.min(y);
        x1 = x1.max(x);
        y1 = y1.max(y);
    }
    (
        x0 - half - 1.0,
        y0 - half - 1.0,
        x1 + half + 1.0,
        y1 + half + 1.0,
    )
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

    /// 序号标：圆内是色底、中心有白字墨迹、圆外全透明。字体读不出的机器上
    /// 这一条自动跳过（与 ice-vision 的字体用例同一个门槛）。
    #[test]
    fn number_badge_paints_a_disk_with_a_white_digit() {
        let Ok(m) = FileMetrics::open() else { return };
        let mut l = Layer::new(64, 64);
        l.number_badge((32.0, 32.0), 14.0, 1, RED, &m);
        let rim = l.image.get_pixel(32, 20);
        assert!(
            rim[3] > 200 && rim[0] > 150 && rim[1] < 100,
            "圆内上缘应是红底，读到的 {rim:?}"
        );
        let white_ink = l
            .image
            .enumerate_pixels()
            .filter(|(x, y, _)| (24..40).contains(x) && (24..40).contains(y))
            .filter(|(_, _, p)| p[3] > 200 && p[0] > 200 && p[1] > 200 && p[2] > 200)
            .count();
        assert!(white_ink > 0, "圆心一带必须有白数字的墨迹");
        assert_eq!(alpha(&l, 32, 5), 0, "圆外必须透明");
        assert_eq!(l.revision, 1);
    }

    #[test]
    fn text_stamps_ink_and_blank_text_touches_nothing() {
        let Ok(m) = FileMetrics::open() else { return };
        let mut l = Layer::new(200, 64);
        assert_eq!(l.text((10.0, 10.0), "   ", 20.0, RED, &m), 0);
        assert_eq!(l.revision, 0, "空白输入不许 touch");
        let missing = l.text((10.0, 10.0), "标注", 20.0, RED, &m);
        assert_eq!(missing, 0, "黑体画得出这两个字");
        assert_eq!(l.revision, 1);
        assert!(
            l.image.pixels().any(|p| p[3] > 200 && p[0] > 150),
            "得有红墨落下去"
        );
    }

    /// 马赛克：块内一色（盒式平均）、不透明、区外一个像素不动。
    #[test]
    fn mosaic_averages_blocks_and_stays_opaque() {
        let mut bg = RgbaImage::new(64, 64);
        for y in 0..64 {
            for x in 0..64 {
                let v = if x < 32 { 0 } else { 255 };
                bg.put_pixel(x, y, image::Rgba([v, v, v, 255]));
            }
        }
        let mut l = Layer::new(64, 64);
        l.mosaic(&bg, (16.0, 16.0), (48.0, 48.0), 8);
        assert_eq!(*l.image.get_pixel(20, 20), image::Rgba([0, 0, 0, 255]));
        assert_eq!(
            *l.image.get_pixel(36, 20),
            image::Rgba([255, 255, 255, 255])
        );
        assert_eq!(alpha(&l, 8, 8), 0, "区外必须透明");
        assert_eq!(l.revision, 1);
        // 块格横跨黑白交界时，块内是平均灰而不是条纹
        let mut l2 = Layer::new(64, 64);
        l2.mosaic(&bg, (24.0, 24.0), (40.0, 40.0), 16);
        let p = l2.image.get_pixel(28, 28);
        assert!(
            p[0] > 60 && p[0] < 200,
            "跨交界的块应是平均灰，读到的 {p:?}"
        );
    }

    #[test]
    fn mosaic_on_a_zero_width_rect_touches_nothing() {
        let bg = RgbaImage::from_pixel(32, 32, image::Rgba([10, 10, 10, 255]));
        let mut l = Layer::new(32, 32);
        l.mosaic(&bg, (8.0, 8.0), (8.0, 20.0), 4);
        assert_eq!(l.revision, 0);
        assert!(l.image.pixels().all(|p| p[3] == 0));
    }

    /// REQ-003：换粗细档，落层线宽跟着变（同一条线，粗档不透明像素更多）。
    #[test]
    fn thicker_tier_paints_a_wider_line() {
        let count = |w: f32| {
            let mut l = Layer::new(64, 64);
            l.line((8.0, 32.0), (56.0, 32.0), w, PALETTE[0]);
            l.image.pixels().filter(|p| p[3] > 0).count()
        };
        assert!(
            count(THICKNESS[2]) > count(THICKNESS[1]) && count(THICKNESS[1]) > count(THICKNESS[0]),
            "细<中<粗三档落层宽度必须递增"
        );
    }

    /// REQ-003：换色板，落层颜色跟着变；默认档（0）就是红。
    #[test]
    fn palette_tier_changes_the_painted_color_and_default_is_red() {
        assert_eq!(PALETTE[0], [220, 40, 40, 255], "默认红在最前");
        let mid = |c: [u8; 4]| {
            let mut l = Layer::new(64, 64);
            l.line((8.0, 32.0), (56.0, 32.0), THICKNESS[1], c);
            *l.image.get_pixel(32, 32)
        };
        let red = mid(PALETTE[0]);
        let blue = mid(PALETTE[2]);
        assert_eq!(red.0, [220, 40, 40, 255]);
        assert_eq!(blue.0, [40, 110, 220, 255]);
        assert_ne!(red, blue);
    }

    /// REQ-003：下拉文案与档值同序同长（名字表错位＝下拉选的是另一档）。
    #[test]
    fn tier_names_line_up_with_tier_values() {
        assert_eq!(THICKNESS_NAMES.len(), THICKNESS.len());
        assert_eq!(PALETTE_NAMES.len(), PALETTE.len());
        assert_eq!(THICKNESS_NAMES[1], "中", "默认中档");
        assert_eq!(PALETTE_NAMES[0], "红", "默认红");
    }

    /// REQ-001：撤销把整层恢复到落笔前**逐像素一致**（含 alpha=0 的还原），
    /// revision 加一让显示侧重传纹理；再撤一次空栈返回 false、层一个像素不动。
    #[test]
    fn undo_restores_the_layer_pixel_exact() {
        let mut l = Layer::new(64, 64);
        let before = l.image.clone();
        l.line((10.0, 32.0), (54.0, 32.0), 4.0, RED);
        assert_ne!(l.image, before);
        let rev = l.revision;
        assert!(l.undo());
        assert_eq!(l.image, before, "撤销必须逐像素回到落笔前");
        assert_eq!(l.revision, rev + 1);
        let after_undo = l.image.clone();
        assert!(!l.undo(), "空栈撤销返回 false");
        assert_eq!(l.image, after_undo);
        assert_eq!(l.revision, rev + 1, "空栈撤销不许动 revision");
    }

    /// REQ-001：铅笔一次拖拽各段累积前像，松手整段算**一条**——一次撤销全段消失。
    #[test]
    fn one_drag_is_one_undo_entry() {
        let mut l = Layer::new(64, 64);
        let before = l.image.clone();
        l.begin_stroke();
        l.pencil(&[(5.0, 5.0), (20.0, 30.0)], 3.0, RED);
        l.pencil(&[(20.0, 30.0), (40.0, 10.0)], 3.0, RED);
        l.pencil(&[(40.0, 10.0), (55.0, 45.0)], 3.0, RED);
        l.end_stroke();
        assert!(l.undo(), "三段拖拽只入一条，一次就该撤掉");
        assert_eq!(l.image, before, "整段拖拽一次撤销全回到空白");
        assert!(!l.undo(), "不许剩第二条");
    }

    /// REQ-001：橡皮擦掉的 alpha 也能撤回来（前像抄的是擦之前的墨）。
    #[test]
    fn undo_restores_erased_ink() {
        let mut l = Layer::new(64, 64);
        l.line((10.0, 32.0), (54.0, 32.0), 4.0, RED);
        let after_line = l.image.clone();
        l.erase((10.0, 32.0), (30.0, 32.0), 6.0);
        assert_eq!(alpha(&l, 20, 32), 0);
        assert!(l.undo());
        assert_eq!(l.image, after_line, "撤销把擦掉的那半墨原样还回来");
        assert!(alpha(&l, 20, 32) > 200);
    }

    /// REQ-001 / Q-003：条数封 30——第 31 笔入栈时最旧那条被丢，只能撤 30 次。
    #[test]
    fn entry_cap_keeps_only_thirty() {
        let mut l = Layer::new(64, 64);
        for i in 0..31 {
            let y = 2.0 + i as f32 * 2.0;
            l.line((2.0, y), (60.0, y), 1.0, RED);
        }
        let mut n = 0;
        while l.undo() {
            n += 1;
            assert!(n <= MAX_UNDO_ENTRIES, "撤销次数不许超过条数上限");
        }
        assert_eq!(n, MAX_UNDO_ENTRIES);
        assert!(alpha(&l, 30, 2) > 0, "最旧那笔已被丢出栈，撤不掉、墨还在");
    }

    /// REQ-001 / Q-003：字节封 64MB——整层大快照入栈超限时从栈首丢最旧。
    /// 2048² 一层 16MB，五条整层前像 80MB > 64MB，入第五条时丢最旧，栈里剩四条。
    #[test]
    fn byte_cap_drops_the_oldest_snapshots() {
        let mut l = Layer::new(2048, 2048);
        for _ in 0..5 {
            l.rect_stroke((0.0, 0.0), (2047.0, 2047.0), 2.0, RED);
        }
        let mut n = 0;
        while l.undo() {
            n += 1;
        }
        assert_eq!(n, 4, "64MB 装得下四条 16MB，第五条挤掉最旧");
    }

    /// REQ-001：空拖拽（按下没画就松手）不入栈、不动 revision。
    #[test]
    fn an_empty_drag_pushes_nothing() {
        let mut l = Layer::new(32, 32);
        l.begin_stroke();
        l.end_stroke();
        assert_eq!(l.revision, 0);
        assert!(!l.undo());
    }
}
