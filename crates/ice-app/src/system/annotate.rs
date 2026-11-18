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
#[derive(Clone, Debug)]
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
/// 可克隆：确认截图时把层的所有权（像素＋撤销栈）clone 一份交给贴图（CHG-015 REQ-002），
/// 不借遮罩那扇随确认即清的 Arc。
#[derive(Clone, Debug)]
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
    /// 零宽/零高的区不落、不 touch。绘制本体抽到 [`mosaic_paint`]，与对象引擎共用一份。
    pub fn mosaic(&mut self, background: &RgbaImage, a: (f32, f32), b: (f32, f32), block: u32) {
        let (x0, x1) = (a.0.min(b.0).floor() as i64, a.0.max(b.0).ceil() as i64);
        let (y0, y1) = (a.1.min(b.1).floor() as i64, a.1.max(b.1).ceil() as i64);
        if x1 <= x0 || y1 <= y0 {
            return;
        }
        let part = self.before_image((x0 as f32, y0 as f32, x1 as f32, y1 as f32));
        mosaic_paint(&mut self.image, background, a, b, block);
        self.record(part);
        self.touch();
    }
}

/// 马赛克盒式平均落像素本体（无 undo 副作用，纯绘制）：拖区按 block 分格，每格取
/// 背景 ∩ 拖区 ∩ 画布 的实际覆盖平均成不透明色写回 img。零宽/零高直接返回。
/// 像素 Layer 的 mosaic 与对象引擎的 render 共用这一份（H1 逐像素同源）。
fn mosaic_paint(
    img: &mut RgbaImage,
    background: &RgbaImage,
    a: (f32, f32),
    b: (f32, f32),
    block: u32,
) {
    let block = block.max(1) as i64;
    let (x0, x1) = (a.0.min(b.0).floor() as i64, a.0.max(b.0).ceil() as i64);
    let (y0, y1) = (a.1.min(b.1).floor() as i64, a.1.max(b.1).ceil() as i64);
    if x1 <= x0 || y1 <= y0 {
        return;
    }
    let (bw, bh) = (background.width() as i64, background.height() as i64);
    let (lw, lh) = (img.width() as i64, img.height() as i64);
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
                        img.put_pixel(x as u32, y as u32, avg);
                    }
                }
            }
            cx += block;
        }
        cy += block;
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

// ── 对象模型引擎（CHG-016 T-001）──────────────────────────────────────────
// 事实源从"一层像素"换成"一组对象"：每件标注是带几何＋样式＋自身擦除痕迹的对象，
// 像素层降级为按对象列表重渲染的派生缓存。T-001 先立引擎与 render（上面那套像素 Layer
// 仍服务于现网，未接线前标 allow(dead_code)；T-002 把八件工具切到建对象、T-003 把撤销
// 切到对象快照后，本注释与 allow 移除、旧脏矩形前像那套删除）。
//
// 擦除是绑定到对象自身的透明痕迹（事实 18 用户实测：拖走被擦对象，缺口跟着挪）：
// render 逐件先画几何进临时 tile、再用该件 erased 对 tile 自身 destination-out 减 alpha，
// 最后合成进层——所以擦除天然随对象平移，且命中只看几何（REQ-004）。

/// 一件标注对象的几何＋样式（坐标物理像素，样式存具体值不是档索引）。
#[allow(dead_code)]
#[derive(Clone, Debug, PartialEq)]
pub enum Shape {
    Rect {
        a: (f32, f32),
        b: (f32, f32),
        width: f32,
        color: [u8; 4],
    },
    Arrow {
        from: (f32, f32),
        to: (f32, f32),
        width: f32,
        color: [u8; 4],
    },
    Line {
        a: (f32, f32),
        b: (f32, f32),
        width: f32,
        color: [u8; 4],
    },
    Pencil {
        points: Vec<(f32, f32)>,
        width: f32,
        color: [u8; 4],
    },
    Serial {
        center: (f32, f32),
        radius: f32,
        number: u32,
        color: [u8; 4],
    },
    Text {
        at: (f32, f32),
        text: String,
        font_size: f32,
        color: [u8; 4],
    },
    Mosaic {
        a: (f32, f32),
        b: (f32, f32),
        block: u32,
    },
}

/// 橡皮对某件对象划过的擦除痕迹：画笔轨迹 / 矩形区两模式（对应 某截图工具 PathMode）。
/// 与所属对象一起平移，故擦除跟着对象走（事实 18）。
#[allow(dead_code)]
#[derive(Clone, Debug, PartialEq)]
pub enum EraseStamp {
    Pen { points: Vec<(f32, f32)>, width: f32 },
    Rect { a: (f32, f32), b: (f32, f32) },
}

/// 一个标注对象：几何＋样式＋自身擦除痕迹。
#[allow(dead_code)]
#[derive(Clone, Debug, PartialEq)]
pub struct AnnoObject {
    pub shape: Shape,
    pub erased: Vec<EraseStamp>,
}

/// 对象列表＝标注唯一事实源。undo/redo 存对象快照（廉价：一组对象不是像素），
/// 取代像素层那套脏矩形前像（REQ-003）。next_serial 序号自增计数，跟着对象走。
#[derive(Clone, Debug)]
pub struct ObjectList {
    pub objects: Vec<AnnoObject>,
    pub revision: u64,
    pub next_serial: u32,
    undo: Vec<Vec<AnnoObject>>,
    redo: Vec<Vec<AnnoObject>>,
}

// 引擎已备齐但遮罩尚未接线（T-002 接线后移除本 allow）：现网仍走像素 Layer，
// 这套对象列表的 undo/erase/render 仅由无头测试驱动，非测试构建里暂视为 dead。
#[allow(dead_code)]
impl ObjectList {
    pub fn new() -> Self {
        Self {
            objects: Vec::new(),
            revision: 0,
            next_serial: 1,
            undo: Vec::new(),
            redo: Vec::new(),
        }
    }

    fn touch(&mut self) {
        self.revision = self.revision.wrapping_add(1);
    }

    /// 突变前压一份快照；新突变清空 redo（标准语义）；条数封 30（对象快照廉价，无字节上限）。
    fn snapshot(&mut self) {
        self.undo.push(self.objects.clone());
        if self.undo.len() > MAX_UNDO_ENTRIES {
            self.undo.remove(0);
        }
        self.redo.clear();
    }

    /// 建一个对象（八件落墨工具松手/单击时调）：压快照 → 入列表 → bump revision。
    pub fn push(&mut self, obj: AnnoObject) {
        self.snapshot();
        self.objects.push(obj);
        self.touch();
    }

    /// 序号标：用 next_serial 建 Serial 并自增（默认从 1 起，CHG-014 行为）。
    pub fn push_serial(&mut self, center: (f32, f32), radius: f32, color: [u8; 4]) {
        let number = self.next_serial;
        self.push(AnnoObject {
            shape: Shape::Serial {
                center,
                radius,
                number,
                color,
            },
            erased: Vec::new(),
        });
        self.next_serial += 1;
    }

    /// 橡皮画笔：把这段轨迹作为擦除痕迹追加到**笔刷压到的每个对象**上（一次调用＝一条快照）。
    /// 没压到任何对象则什么都不做（不空压快照）。痕迹是绝对坐标，随对象 translate 一起挪。
    pub fn erase_pen(&mut self, points: Vec<(f32, f32)>, width: f32) {
        let idxs = self.crossed(|o| o.crossed_by_pen(&points, width));
        if idxs.is_empty() {
            return;
        }
        self.snapshot();
        for i in idxs {
            self.objects[i].erased.push(EraseStamp::Pen {
                points: points.clone(),
                width,
            });
        }
        self.touch();
    }

    /// 橡皮矩形：把矩形区作为擦除痕迹追加到压到的每个对象上。
    pub fn erase_rect(&mut self, a: (f32, f32), b: (f32, f32)) {
        let idxs = self.crossed(|o| o.crossed_by_rect(a, b));
        if idxs.is_empty() {
            return;
        }
        self.snapshot();
        for i in idxs {
            self.objects[i].erased.push(EraseStamp::Rect { a, b });
        }
        self.touch();
    }

    fn crossed(&self, pred: impl Fn(&AnnoObject) -> bool) -> Vec<usize> {
        (0..self.objects.len())
            .filter(|&i| pred(&self.objects[i]))
            .collect()
    }

    /// 撤销：弹最近一份快照恢复，当前份压入 redo，重渲染靠 revision 变触发。空栈 false。
    pub fn undo(&mut self) -> bool {
        match self.undo.pop() {
            Some(prev) => {
                self.redo.push(self.objects.clone());
                self.objects = prev;
                self.touch();
                true
            }
            None => false,
        }
    }

    /// 重做：弹 redo 恢复，当前份压回 undo。空栈 false。
    pub fn redo(&mut self) -> bool {
        match self.redo.pop() {
            Some(next) => {
                self.undo.push(self.objects.clone());
                self.objects = next;
                self.touch();
                true
            }
            None => false,
        }
    }
}

#[allow(dead_code)]
impl AnnoObject {
    /// 整体平移：几何点与自身擦除痕迹一起挪（擦除跟着对象走，事实 18）。
    pub fn translate(&mut self, dx: f32, dy: f32) {
        let shift = |p: &mut (f32, f32)| {
            p.0 += dx;
            p.1 += dy;
        };
        match &mut self.shape {
            Shape::Rect { a, b, .. } => {
                shift(a);
                shift(b);
            }
            Shape::Arrow { from, to, .. } => {
                shift(from);
                shift(to);
            }
            Shape::Line { a, b, .. } => {
                shift(a);
                shift(b);
            }
            Shape::Pencil { points, .. } => {
                for p in points.iter_mut() {
                    shift(p);
                }
            }
            Shape::Serial { center, .. } => shift(center),
            Shape::Text { at, .. } => shift(at),
            Shape::Mosaic { a, b, .. } => {
                shift(a);
                shift(b);
            }
        }
        for s in &mut self.erased {
            match s {
                EraseStamp::Pen { points, .. } => {
                    for p in points.iter_mut() {
                        shift(p);
                    }
                }
                EraseStamp::Rect { a, b } => {
                    shift(a);
                    shift(b);
                }
            }
        }
    }

    /// 点到该对象"落墨几何"的距离（落在墨上＝0）。橡皮笔刷半径 ≥ 此距离即算压到这一件。
    /// 只看几何不看 erased（与 REQ-004 命中同口径）。
    pub fn hit_dist(&self, p: (f32, f32)) -> f32 {
        match &self.shape {
            Shape::Rect { a, b, width, .. } => {
                let (x0, x1) = (a.0.min(b.0), a.0.max(b.0));
                let (y0, y1) = (a.1.min(b.1), a.1.max(b.1));
                let d = dist_to_segment(p, (x0, y0), (x1, y0))
                    .min(dist_to_segment(p, (x1, y0), (x1, y1)))
                    .min(dist_to_segment(p, (x1, y1), (x0, y1)))
                    .min(dist_to_segment(p, (x0, y1), (x0, y0)));
                (d - width / 2.0).max(0.0)
            }
            Shape::Line { a, b, width, .. } => (dist_to_segment(p, *a, *b) - width / 2.0).max(0.0),
            Shape::Arrow {
                from, to, width, ..
            } => {
                let [h1, h2] = arrow_head(*from, *to, *width);
                let d = dist_to_segment(p, *from, *to)
                    .min(dist_to_segment(p, *to, h1))
                    .min(dist_to_segment(p, *to, h2));
                (d - width / 2.0).max(0.0)
            }
            Shape::Pencil { points, width, .. } => {
                let mut d = f32::INFINITY;
                if points.len() == 1 {
                    d = dist_to_segment(p, points[0], points[0]);
                } else {
                    for w in points.windows(2) {
                        d = d.min(dist_to_segment(p, w[0], w[1]));
                    }
                }
                (d - width / 2.0).max(0.0)
            }
            Shape::Serial { center, radius, .. } => (dist(p, *center) - radius).max(0.0),
            Shape::Text {
                at,
                text,
                font_size,
                ..
            } => {
                let w = text.chars().count() as f32 * font_size * 0.6;
                let (x0, y0, x1, y1) = (at.0, at.1, at.0 + w.max(1.0), at.1 + font_size);
                dist_to_rect(p, x0, y0, x1, y1)
            }
            Shape::Mosaic { a, b, .. } => {
                let (x0, x1) = (a.0.min(b.0), a.0.max(b.0));
                let (y0, y1) = (a.1.min(b.1), a.1.max(b.1));
                dist_to_rect(p, x0, y0, x1, y1)
            }
        }
    }

    /// 对象包围盒（AABB），矩形擦除区是否压到用它粗判。
    pub fn bbox(&self) -> (f32, f32, f32, f32) {
        let pts = match &self.shape {
            Shape::Rect { a, b, .. } | Shape::Line { a, b, .. } | Shape::Mosaic { a, b, .. } => {
                vec![*a, *b]
            }
            Shape::Arrow { from, to, .. } => vec![*from, *to],
            Shape::Pencil { points, .. } => points.clone(),
            Shape::Serial { center, radius, .. } => {
                vec![
                    (center.0 - radius, center.1 - radius),
                    (center.0 + radius, center.1 + radius),
                ]
            }
            Shape::Text {
                at,
                text,
                font_size,
                ..
            } => {
                let w = text.chars().count() as f32 * font_size * 0.6;
                vec![*at, (at.0 + w.max(1.0), at.1 + font_size)]
            }
        };
        let mut x0 = f32::INFINITY;
        let mut y0 = f32::INFINITY;
        let mut x1 = f32::NEG_INFINITY;
        let mut y1 = f32::NEG_INFINITY;
        for p in pts {
            x0 = x0.min(p.0);
            y0 = y0.min(p.1);
            x1 = x1.max(p.0);
            y1 = y1.max(p.1);
        }
        (x0, y0, x1, y1)
    }

    /// 画笔橡皮这段轨迹是否压到本对象：任一轨迹点离落墨几何 ≤ 半笔宽。
    pub fn crossed_by_pen(&self, points: &[(f32, f32)], width: f32) -> bool {
        let r = width / 2.0;
        points.iter().any(|&p| self.hit_dist(p) <= r)
    }

    /// 矩形橡皮区是否压到本对象：AABB 相交。
    pub fn crossed_by_rect(&self, a: (f32, f32), b: (f32, f32)) -> bool {
        let (ox0, oy0, ox1, oy1) = self.bbox();
        let (rx0, rx1) = (a.0.min(b.0), a.0.max(b.0));
        let (ry0, ry1) = (a.1.min(b.1), a.1.max(b.1));
        rx0 <= ox1 && rx1 >= ox0 && ry0 <= oy1 && ry1 >= oy0
    }
}

/// 两点欧氏距离。
#[allow(dead_code)]
fn dist(a: (f32, f32), b: (f32, f32)) -> f32 {
    ((a.0 - b.0).powi(2) + (a.1 - b.1).powi(2)).sqrt()
}

/// 点到矩形（x0,y0)-(x1,y1) 的距离：在内为 0，否则到最近边/角。
#[allow(dead_code)]
fn dist_to_rect(p: (f32, f32), x0: f32, y0: f32, x1: f32, y1: f32) -> f32 {
    let dx = (x0 - p.0).max(p.0 - x1).max(0.0);
    let dy = (y0 - p.1).max(p.1 - y1).max(0.0);
    (dx * dx + dy * dy).sqrt()
}

/// 按列表序逐件重渲染整层：每件先画几何进临时 tile、再用该件 erased 对 tile 自身
/// destination-out 减 alpha，最后合成进层。叠放序＝列表序＝落笔序（H1 与旧像素引擎逐像素同源）。
#[allow(dead_code)]
pub fn render(objects: &[AnnoObject], background: &RgbaImage, metrics: &FileMetrics) -> RgbaImage {
    let (w, h) = (background.width(), background.height());
    let mut layer = RgbaImage::new(w, h);
    let mut tile = RgbaImage::new(w, h);
    for obj in objects {
        for p in tile.pixels_mut() {
            *p = image::Rgba([0, 0, 0, 0]);
        }
        paint_shape(&mut tile, &obj.shape, background, metrics);
        for stamp in &obj.erased {
            punch_stamp(&mut tile, stamp);
        }
        composite_into(&mut layer, &tile, 0, 0);
    }
    layer
}

/// 单件对象的几何落像素（纯绘制，无 undo 副作用），复用既有光栅基元。
#[allow(dead_code)]
fn paint_shape(img: &mut RgbaImage, shape: &Shape, background: &RgbaImage, metrics: &FileMetrics) {
    match shape {
        Shape::Rect { a, b, width, color } => {
            let half = width / 2.0;
            let (x0, x1) = (a.0.min(b.0), a.0.max(b.0));
            let (y0, y1) = (a.1.min(b.1), a.1.max(b.1));
            for (p, q) in [
                ((x0, y0), (x1, y0)),
                ((x1, y0), (x1, y1)),
                ((x1, y1), (x0, y1)),
                ((x0, y1), (x0, y0)),
            ] {
                paint_segment(img, p, q, half, *color);
            }
        }
        Shape::Line { a, b, width, color } => {
            paint_segment(img, *a, *b, width / 2.0, *color);
        }
        Shape::Arrow {
            from,
            to,
            width,
            color,
        } => {
            let half = width / 2.0;
            let [h1, h2] = arrow_head(*from, *to, *width);
            paint_segment(img, *from, *to, half, *color);
            paint_segment(img, *to, h1, half, *color);
            paint_segment(img, *to, h2, half, *color);
        }
        Shape::Pencil {
            points,
            width,
            color,
        } => {
            let half = width / 2.0;
            for pair in points.windows(2) {
                paint_segment(img, pair[0], pair[1], half, *color);
            }
            if let Some(&p) = points.first() {
                if points.len() == 1 {
                    paint_segment(img, p, p, half, *color);
                }
            }
        }
        Shape::Serial {
            center,
            radius,
            number,
            color,
        } => {
            let font_size = radius * 1.2;
            let line_h = metrics.line_height(font_size);
            let area = PixelRect::new(
                center.0 - radius,
                center.1 - line_h / 2.0,
                radius * 2.0,
                line_h,
            );
            let label = number.to_string();
            paint_segment(img, *center, *center, *radius, *color);
            stamp_line(
                img,
                area,
                &label,
                font_size,
                Align::Center,
                [255, 255, 255, 255],
                metrics,
            );
        }
        Shape::Text {
            at,
            text,
            font_size,
            color,
        } => {
            let t = text.trim();
            if t.is_empty() {
                return;
            }
            let line_h = metrics.line_height(*font_size);
            let width = metrics.text_width(t, *font_size);
            let area = PixelRect::new(at.0, at.1, width.max(1.0), line_h);
            stamp_line(img, area, t, *font_size, Align::Left, *color, metrics);
        }
        Shape::Mosaic { a, b, block } => mosaic_paint(img, background, *a, *b, *block),
    }
}

/// 对 tile 自身按擦除痕迹 destination-out 减 alpha（复用 CHG-014 erase 那套覆盖率算法）。
#[allow(dead_code)]
fn punch_stamp(img: &mut RgbaImage, stamp: &EraseStamp) {
    match stamp {
        EraseStamp::Pen { points, width } => {
            let r = width / 2.0;
            let segs: Vec<((f32, f32), (f32, f32))> = if points.len() < 2 {
                points.iter().map(|&p| (p, p)).collect()
            } else {
                points.windows(2).map(|w| (w[0], w[1])).collect()
            };
            for (a, b) in segs {
                for_each_covered_pixel(img, a, b, r, |img, x, y, cov| {
                    let px = img.get_pixel_mut(x as u32, y as u32);
                    px[3] = ((px[3] as f32) * (1.0 - cov)).round() as u8;
                });
            }
        }
        EraseStamp::Rect { a, b } => {
            let (w, h) = (img.width(), img.height());
            let x0 = (a.0.min(b.0).floor().max(0.0) as u32).min(w);
            let x1 = (a.0.max(b.0).ceil().max(0.0) as u32).min(w);
            let y0 = (a.1.min(b.1).floor().max(0.0) as u32).min(h);
            let y1 = (a.1.max(b.1).ceil().max(0.0) as u32).min(h);
            for y in y0..y1 {
                for x in x0..x1 {
                    img.get_pixel_mut(x, y)[3] = 0;
                }
            }
        }
    }
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

    // ── 对象引擎（CHG-016 T-001）────────────────────────────────

    /// REQ-001：空对象列表渲染得全透明层，尺寸＝背景尺寸（物理像素）。
    #[test]
    fn render_empty_list_is_fully_transparent() {
        let Ok(m) = FileMetrics::open() else { return };
        let bg = RgbaImage::from_pixel(32, 32, image::Rgba([10, 20, 30, 255]));
        let layer = render(&[], &bg, &m);
        assert_eq!(layer.width(), 32);
        assert_eq!(layer.height(), 32);
        assert!(layer.pixels().all(|p| p[3] == 0), "空列表必须全透明");
    }

    /// H1：单件 Rect 的对象渲染与旧像素引擎 rect_stroke 逐像素一致。
    #[test]
    fn render_rect_matches_pixel_engine() {
        let Ok(m) = FileMetrics::open() else { return };
        let bg = RgbaImage::new(64, 64);
        let obj = AnnoObject {
            shape: Shape::Rect {
                a: (10.0, 10.0),
                b: (50.0, 40.0),
                width: 3.0,
                color: RED,
            },
            erased: vec![],
        };
        let rendered = render(&[obj], &bg, &m);
        let mut l = Layer::new(64, 64);
        l.rect_stroke((10.0, 10.0), (50.0, 40.0), 3.0, RED);
        assert_eq!(
            rendered, l.image,
            "对象渲染必须与旧像素引擎逐像素一致（H1）"
        );
    }

    /// 事实 18：擦除是对象自身的透明痕迹——擦半条线只擦那半条，且 translate 后缺口跟着挪。
    #[test]
    fn erased_punch_is_partial_and_follows_translate() {
        let Ok(m) = FileMetrics::open() else { return };
        let bg = RgbaImage::new(64, 64);
        let mut obj = AnnoObject {
            shape: Shape::Line {
                a: (10.0, 32.0),
                b: (54.0, 32.0),
                width: 4.0,
                color: RED,
            },
            erased: vec![EraseStamp::Pen {
                points: vec![(10.0, 32.0), (30.0, 32.0)],
                width: 12.0,
            }],
        };
        let layer = render(&[obj.clone()], &bg, &m);
        assert_eq!(layer.get_pixel(20, 32)[3], 0, "擦过的那半必须透明");
        assert!(layer.get_pixel(45, 32)[3] > 200, "没擦的那半必须还在");

        obj.translate(0.0, 10.0);
        let moved = render(&[obj], &bg, &m);
        assert_eq!(moved.get_pixel(20, 42)[3], 0, "擦除跟着对象挪到 y=42");
        assert!(moved.get_pixel(45, 42)[3] > 200, "没擦那半也挪到 y=42");
        assert_eq!(moved.get_pixel(20, 32)[3], 0, "原 y=32 整条已挪走、透明");
    }

    /// REQ-001：ObjectList push 即 bump revision，next_serial 默认从 1 起（CHG-014 行为）。
    #[test]
    fn object_list_push_bumps_revision_and_starts_serial_at_one() {
        let mut ol = ObjectList::new();
        assert_eq!(ol.revision, 0);
        assert_eq!(ol.next_serial, 1);
        ol.push(AnnoObject {
            shape: Shape::Line {
                a: (0.0, 0.0),
                b: (1.0, 1.0),
                width: 2.0,
                color: RED,
            },
            erased: vec![],
        });
        assert_eq!(ol.revision, 1);
        assert_eq!(ol.objects.len(), 1);
    }

    /// REQ-003：对象快照撤销/重做——undo 退一份、redo 进一份，空栈 false。
    #[test]
    fn object_list_undo_redo_roundtrip() {
        let mut ol = ObjectList::new();
        let line = |y| AnnoObject {
            shape: Shape::Line {
                a: (2.0, y),
                b: (60.0, y),
                width: 2.0,
                color: RED,
            },
            erased: vec![],
        };
        ol.push(line(10.0));
        ol.push(line(20.0));
        assert_eq!(ol.objects.len(), 2);
        assert!(ol.undo());
        assert_eq!(ol.objects.len(), 1, "撤销退掉最近一件");
        assert!(ol.undo());
        assert_eq!(ol.objects.len(), 0);
        assert!(!ol.undo(), "空栈撤销 false");
        assert!(ol.redo());
        assert_eq!(ol.objects.len(), 1, "重做补回一件");
        assert!(ol.redo());
        assert_eq!(ol.objects.len(), 2);
        assert!(!ol.redo(), "重做到头 false");
    }

    /// REQ-003：新一次突变清空 redo 栈（撤到底再画一笔，之前的重做作废）。
    #[test]
    fn new_mutation_clears_redo() {
        let mut ol = ObjectList::new();
        ol.push(AnnoObject {
            shape: Shape::Line {
                a: (2.0, 2.0),
                b: (8.0, 8.0),
                width: 2.0,
                color: RED,
            },
            erased: vec![],
        });
        ol.undo();
        assert!(ol.redo(), "先有可重做的");
        ol.undo();
        ol.push(AnnoObject {
            shape: Shape::Line {
                a: (3.0, 3.0),
                b: (9.0, 9.0),
                width: 2.0,
                color: RED,
            },
            erased: vec![],
        });
        assert!(!ol.redo(), "新突变后 redo 应清空");
    }

    /// REQ-005：橡皮画笔只给压到的对象加擦除痕迹，没压到的不动；序号自增走 push_serial。
    #[test]
    fn erase_pen_stamps_only_crossed_objects_and_serial_increments() {
        let mut ol = ObjectList::new();
        ol.push(AnnoObject {
            shape: Shape::Line {
                a: (10.0, 32.0),
                b: (54.0, 32.0),
                width: 4.0,
                color: RED,
            },
            erased: vec![],
        });
        ol.push(AnnoObject {
            shape: Shape::Rect {
                a: (5.0, 5.0),
                b: (15.0, 15.0),
                width: 3.0,
                color: RED,
            },
            erased: vec![],
        });
        // 沿那条线左半段擦：只压到线（objects[0]），没压到远处的小框
        ol.erase_pen(vec![(10.0, 32.0), (30.0, 32.0)], 12.0);
        assert_eq!(ol.objects[0].erased.len(), 1, "压到的线加了一条擦除痕迹");
        assert_eq!(ol.objects[1].erased.len(), 0, "没压到的小框不许加");
        // 擦除可撤销：undo 掉这条 erase
        assert!(ol.undo());
        assert_eq!(ol.objects[0].erased.len(), 0, "撤销把擦除痕迹退回");

        // 序号自增：连落两个，号 1、2，next_serial 到 3
        let mut ol2 = ObjectList::new();
        ol2.push_serial((20.0, 20.0), 14.0, RED);
        ol2.push_serial((40.0, 40.0), 14.0, RED);
        assert_eq!(ol2.next_serial, 3);
        match &ol2.objects[0].shape {
            Shape::Serial { number, .. } => assert_eq!(*number, 1),
            _ => panic!("应是 Serial"),
        }
        match &ol2.objects[1].shape {
            Shape::Serial { number, .. } => assert_eq!(*number, 2),
            _ => panic!("应是 Serial"),
        }
    }

    /// REQ-005：擦除痕迹渲染后，被擦段变透明、没擦段还在（端到端 render 验证 erase_pen 生效）。
    #[test]
    fn erase_pen_makes_crossed_segment_transparent_in_render() {
        let Ok(m) = FileMetrics::open() else { return };
        let bg = RgbaImage::new(64, 64);
        let mut ol = ObjectList::new();
        ol.push(AnnoObject {
            shape: Shape::Line {
                a: (10.0, 32.0),
                b: (54.0, 32.0),
                width: 4.0,
                color: RED,
            },
            erased: vec![],
        });
        ol.erase_pen(vec![(10.0, 32.0), (30.0, 32.0)], 12.0);
        let layer = render(&ol.objects, &bg, &m);
        assert_eq!(layer.get_pixel(20, 32)[3], 0, "擦过段透明");
        assert!(layer.get_pixel(45, 32)[3] > 200, "没擦段还在");
    }
}
