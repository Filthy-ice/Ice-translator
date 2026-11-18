//! 选区层（CHG-010 REQ-102 / REQ-104 / REQ-105）。
//!
//! **这个模块里不许有"问屏幕"的动作。** 一扇遮罩盖住的就是**一块屏**，所以本模块的坐标
//! 一律是"那块屏的矩形 + 该屏内的点"，它既不知道有几块屏、也不去读全局鼠标——那些都在
//! `app.rs` 与 `cover.rs` 那一侧。这么切有三个理由，每个都是这仓踩过的坑：
//! 1. 状态机与几何要能在无头里被断言（探针只能证明机制，证不了"擦过去不该选中"这类判断）；
//! 2. 副屏的**负坐标**（本机实测第二块屏挂在 `(-1920,-120)`）只有在"传进来的那块矩形"这条路上
//!    才是自然成立的；自己拿 `screen.size()` 拼一次，负 x 就从"不是 bug"变成"看不见的那一屏"；
//! 3. **点 → 物理像素的换算只许存在一处**（[`Field::to_pixels`]）。CHG-009 事实 22 已经演示过
//!    两处各除一次或都忘了除的现场：贴图大一倍、文字层整体错位。

use crate::system::annotate::{
    AnnoObject, BADGE_RADIUS, MOSAIC_BLOCK_PT, ObjectList, PALETTE, PALETTE_NAMES, Shape,
    TEXT_SIZE, THICKNESS, THICKNESS_NAMES, render,
};
use crate::system::bar::{self, GAP};
use crate::system::wake_root;
use egui::{Id, Pos2, Rect, Vec2, pos2};
use ice_core::document::PixelRect;
use ice_vision::paint::FileMetrics;

/// 一次拖选期间的临时状态。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Stage {
    /// 还没有按下
    Idle,
    /// 按下了、还在拖
    Dragging,
    /// 松手了，选区停在原地等确认（REQ-102 的那句"确认之前不落地"就靠这一格存在）
    Ready,
    /// 确认。终态：帧循环据此去裁、去落盘、去钉住
    Confirmed,
    /// 取消。终态：什么都不留
    Cancelled,
}

/// 一次按下落在哪。**这是全局唯一一处这个判定**（REQ-102：空白 / 框内 / 八个把手之一）。
///
/// 顺序是死的：把手优先于框内。反过来的话右下角那个点会先被判成"框内移动"，
/// 于是八个把手里靠内的那一圈永远点不到——而这种缺陷在无头里测得出来，别调手感。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Grab {
    /// 框外的空白处：按下就是"开始一个新的框"
    Blank,
    /// 框内：按下是"挪整个框"
    Inside,
    /// 八个把手之一，编号见 [`HANDLE_COUNT`] 与 [`handle_center`]
    Handle(u8),
}

/// 八个把手：0=nw 1=n 2=ne 3=e 4=se 5=s 6=sw 7=w。
pub const HANDLE_COUNT: u8 = 8;

/// 把手的命中半径（点）。**探针量到的是"事件进得来、判定按预期分岔"，不是手感**：
/// `CGEventPost` 的注入件按下坐标本身就偏上百个点，拿它判半径是伪证据（spec.md 事实 6）。
/// 这一档的最终值归走查 W-6.1，改动只许改这里。
pub const HANDLE_RADIUS: f32 = 10.0;

/// 把手画出来的边长（点）。**画的大小与命中的半径是两件事**：命中仍是 [`HANDLE_RADIUS`]
/// 那一档（W-6.1 定），这里只管它看起来多大。它有一半探在选区外，所以会吃掉 [`GAP`]
/// 那道缝——下面那条编译期断言就是不许它探进药丸里。
pub const HANDLE_SIZE: f32 = 6.0;

/// "按下到抬起没挪动多少算单击"的阈值（点）。REQ-108 的"单击截整窗"与 REQ-102 的
/// "拖动=自由矩形"共用这一条判据，两处不许各写一个数。
pub const CLICK_SLOP: f32 = 3.0;

/// 压暗那一档（选区之外整体乘到这个亮度）。55% 是 某截图工具 那个量级的默认（Q-005），
/// 只有用户的眼睛能定，改这一格不动任何逻辑。
pub const DIM: u8 = 140;

/// 认出的那扇窗的边色（REQ-108）。与选区的白边分开：白边是"我自己框的"，
/// 这一圈是"系统替我认的，单击就拿走"，两种含义不该用同一种颜色说。
pub const HOVER_EDGE: egui::Color32 = egui::Color32::from_rgb(90, 170, 255);

/// 一块屏 + 它的像素尺寸。**点↔像素的换算只能从这里过**（[`Field::to_pixels`]）。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Field {
    /// 那块屏在**它自己窗口内**的矩形（点）。遮罩开窗时窗口与屏对齐，所以就是 0..尺寸
    pub bounds: egui::Rect,
    /// 那块屏的物理像素尺寸（抓回来的冻结帧就是它）
    pub px: Vec2,
}

impl Field {
    pub fn new(bounds: egui::Rect, px: Vec2) -> Self {
        Self { bounds, px }
    }

    /// 这块屏上"每点多少像素"。**唯一一处**除法。
    ///
    /// 返回 `None` 表示这块屏的尺寸不可信（零宽/零高）——调用方 SHALL 当作
    /// "这台机器拿不到画面"处理，SHALL NOT 把 scale 默认成 1.0：那是 CHG-009 事实 22
    /// 那句"看着 1:1 像是普遍规律"的另一种写法。
    pub fn scale(&self) -> Option<f32> {
        let w = self.bounds.width();
        let h = self.bounds.height();
        if w <= 0.0 || h <= 0.0 || self.px.x <= 0.0 || self.px.y <= 0.0 {
            return None;
        }
        // 横向纵向各算一次再核对： Retina 竖屏或缩放不一致时这里就会分歧，
        // 分歧了宁可判"换算不可信"，也不要裁出一张压扁的图。
        let sx = self.px.x / w;
        let sy = self.px.y / h;
        // 横纵不一致（旋转屏、非等比缩放）时不猜：判"换算不可信"，让调用方去说人话
        if (sx - sy).abs() > 0.01 {
            return None;
        }
        Some(sx)
    }

    /// 点矩形 → 像素矩形。**裁剪要用的物理像素只许由这个函数产出**（REQ-104）。
    ///
    /// 夹在屏内由 [`Field::clamp`] 先做过，这里只负责换算与"至少 1 像素"的兜底：
    /// 拖出一条线的选区不该裁出 0 宽的文件，那种文件后面识别链会给出"未发现文字"，
    /// 用户看到的是一句与他动作对不上的话。
    pub fn to_pixels(self, sel: Rect) -> Option<PixelRect> {
        let scale = self.scale()?;
        let sel = self.clamp(sel);
        let x = (sel.min.x - self.bounds.min.x) * scale;
        let y = (sel.min.y - self.bounds.min.y) * scale;
        let right = (sel.max.x - self.bounds.min.x) * scale;
        let bottom = (sel.max.y - self.bounds.min.y) * scale;
        Some(PixelRect::new(
            x.max(0.0),
            y.max(0.0),
            (right - x).max(1.0),
            (bottom - y).max(1.0),
        ))
    }

    /// 把矩形夹进这块屏，并保证 min ≤ max（副屏负坐标也走这一条）。
    pub fn clamp(&self, r: egui::Rect) -> egui::Rect {
        let x = |v: f32| v.clamp(self.bounds.min.x, self.bounds.max.x);
        let y = |v: f32| v.clamp(self.bounds.min.y, self.bounds.max.y);
        let (a, b, c, d) = (x(r.min.x), y(r.min.y), x(r.max.x), y(r.max.y));
        Rect::from_min_max(pos2(a.min(c), b.min(d)), pos2(a.max(c), b.max(d)))
    }
}

/// 第 `index` 号把手在矩形上的位置（点）。
pub fn handle_center(rect: egui::Rect, index: u8) -> egui::Pos2 {
    let [x0, y0, x1, y1] = [rect.min.x, rect.min.y, rect.max.x, rect.max.y];
    let (mx, my) = ((x0 + x1) / 2.0, (y0 + y1) / 2.0);
    match index {
        0 => pos2(x0, y0),
        1 => pos2(mx, y0),
        2 => pos2(x1, y0),
        3 => pos2(x1, my),
        4 => pos2(x1, y1),
        5 => pos2(mx, y1),
        6 => pos2(x0, y1),
        _ => pos2(x0, my),
    }
}

/// 一次按下落在哪（[`Grab`] 的顺序纪律：把手 → 框内 → 空白）。
pub fn grab(rect: Option<egui::Rect>, at: egui::Pos2, radius: f32) -> Grab {
    let Some(rect) = rect else { return Grab::Blank };
    for index in 0..HANDLE_COUNT {
        let center = handle_center(rect, index);
        if (at.x - center.x).abs() <= radius && (at.y - center.y).abs() <= radius {
            return Grab::Handle(index);
        }
    }
    if rect.contains(at) {
        Grab::Inside
    } else {
        Grab::Blank
    }
}

/// 这一帧的按下算不算"单击"（没挪动）。REQ-108 用它决定"截认出的那扇窗"。
pub fn is_click(from: egui::Pos2, to: egui::Pos2) -> bool {
    (to - from).length() <= CLICK_SLOP
}

/// 选区那一次的完整状态。**只有状态与纯方法，不读输入也不写盘。**
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Selection {
    pub stage: Stage,
    /// 屏上的那块矩形（点）。`Ready`/`Confirmed` 时一定非空
    pub rect: Option<egui::Rect>,
    /// 按下那一点（拖新框与框内移动的锚点）。框内移动期间**不改写**：始终拿它和
    /// [`Self::grabbed`] 算绝对位移，按住的那一点才会一直贴在鼠标下。
    anchor: egui::Pos2,
    /// 这次按下抢到了什么
    grab: Grab,
    /// 这一帧有没有在拖
    dragging: bool,
    /// **框内按下那一刻**的矩形。移动永远从这份原始位置 +（当前点 − 按下点）算起，
    /// 不逐帧累加——累加写法在屏缘夹取之后会把抓取点漂掉（用户 2026-11-16 真机报的
    /// "左上角粘鼠标"就是把锚点记成了框角，D-43）。
    grabbed: Option<Rect>,
    /// **空白处按下之前**那块矩形。空白单击是"要鼠标底下那扇窗"（REQ-108），
    /// 不是"把上一个框弄没"——所以旧框先押着，真的拖起来了才丢。
    previous: Option<Rect>,
    /// 这次空白按下有没有真的拖出去过（超过 [`CLICK_SLOP`]）
    moved: bool,
}

impl Default for Selection {
    fn default() -> Self {
        Self {
            stage: Stage::Idle,
            rect: None,
            anchor: Pos2::ZERO,
            grab: Grab::Blank,
            dragging: false,
            grabbed: None,
            previous: None,
            moved: false,
        }
    }
}

impl Selection {
    /// 每次打开遮罩都从这一格起（Q-002：不记上一次的尺寸与位置）。
    pub fn fresh() -> Self {
        Self::default()
    }

    /// 按下。返回这次按下抢到了什么，帧循环据此决定这一帧要不要接管鼠标。
    pub fn press(&mut self, field: &Field, at: Pos2, radius: f32) -> Grab {
        let at = pos2(
            at.x.clamp(field.bounds.min.x, field.bounds.max.x),
            at.y.clamp(field.bounds.min.y, field.bounds.max.y),
        );
        self.grab = grab(self.rect, at, radius);
        self.anchor = at;
        self.dragging = true;
        self.stage = Stage::Dragging;
        match self.grab {
            // 空白处按下：旧框押着不丢，真拖出去了才算"新框顶掉旧框"
            Grab::Blank => {
                self.previous = self.rect;
                self.rect = Some(Rect::from_min_max(at, at));
                self.moved = false;
                self.grabbed = None;
            }
            // 框内按下：押住原始矩形，移动从它算绝对位移（抓取点不漂）
            Grab::Inside => {
                self.grabbed = self.rect;
            }
            Grab::Handle(_) => {
                self.grabbed = None;
            }
        }
        self.grab
    }

    /// 拖动中。`press` 之后每帧都可以调，一次算一步。
    pub fn drag(&mut self, field: &Field, to: Pos2) {
        if !self.dragging {
            return;
        }
        let to = pos2(
            to.x.clamp(field.bounds.min.x, field.bounds.max.x),
            to.y.clamp(field.bounds.min.y, field.bounds.max.y),
        );
        match self.grab {
            Grab::Blank => {
                // 没超过阈值的"拖"还是单击：不许一抖就把用户的旧框顶掉
                if !self.moved && is_click(self.anchor, to) {
                    return;
                }
                self.moved = true;
                self.rect = Some(Rect::from_two_pos(self.anchor, to));
            }
            Grab::Inside => {
                if let Some(base) = self.grabbed {
                    let size = base.size();
                    // 移动不许把框推出屏外：先算整块能放得下的范围，再平移。
                    // 从原始矩形算绝对位移：夹在屏缘再拖回来，抓取点仍贴在鼠标下。
                    let min_x = field.bounds.min.x;
                    let max_x = (field.bounds.max.x - size.x).max(min_x);
                    let min_y = field.bounds.min.y;
                    let max_y = (field.bounds.max.y - size.y).max(min_y);
                    let moved = pos2(
                        (base.min.x + (to.x - self.anchor.x)).clamp(min_x, max_x),
                        (base.min.y + (to.y - self.anchor.y)).clamp(min_y, max_y),
                    );
                    self.rect = Some(Rect::from_min_size(moved, size));
                }
            }
            Grab::Handle(index) => {
                if let Some(rect) = self.rect {
                    self.rect = Some(field.clamp(resize(rect, index, to)));
                }
            }
        }
    }

    /// 松手。三种落点：
    /// - 拖出了面积 → `Ready`（停在原地等确认，REQ-102"确认之前不落地"就靠这一格存在）；
    /// - 空白单击、没拖出面积 → 把押着的旧框还回来，回 `Ready`/`Idle`，并让调用方去看
    ///   那一扇认出的窗（[`Selection::take_window`]，REQ-108 的"单击截整窗"）；
    /// - 在已有框上单击（点把手/点框内）→ 保持原框，`Ready`。
    ///
    /// **抬起在屏外也算一次正常松手**——拖出去松手是常态，判成取消等于让用户白拖一次。
    pub fn release(&mut self, field: &Field, to: Pos2) -> Stage {
        self.dragging = false;
        self.drag(field, to);
        if self.grab == Grab::Blank && !self.moved {
            self.rect = self.previous;
        }
        // 退化成一条线/一个点的矩形不是选区
        self.rect = self
            .rect
            .filter(|rect| rect.width() > 0.0 && rect.height() > 0.0);
        self.stage = match self.rect {
            Some(_) => Stage::Ready,
            None => Stage::Idle,
        };
        self.stage
    }

    /// 这一次松手是不是"单击"（该去认那扇窗）。要在 [`Selection::release`] 之后问。
    pub fn clicked(&self) -> bool {
        self.grab == Grab::Blank && !self.moved
    }

    /// 外部（遮罩自己算好的）框直接进 `Ready`：确认时主窗用它，不再重算一遍。
    pub fn from_rect(field: &Field, rect: Rect) -> Selection {
        let mut sel = Selection::fresh();
        sel.rect = Some(field.clamp(rect));
        sel.stage = Stage::Ready;
        sel
    }

    /// 认出的那扇窗直接成为选区并确认（REQ-108 的"单击就走"）。
    pub fn take_window(&mut self, field: &Field, rect: Rect) -> Stage {
        self.rect = Some(field.clamp(rect));
        self.dragging = false;
        self.stage = Stage::Confirmed;
        self.stage
    }

    pub fn confirm(&mut self) -> Stage {
        if self.rect.is_some() {
            self.stage = Stage::Confirmed;
        }
        self.stage
    }

    /// 取消：**三个入口都汇到这里**（REQ-105），任何一条收尾都不许多做一件事。
    pub fn cancel(&mut self) {
        self.stage = Stage::Cancelled;
        self.rect = None;
        self.dragging = false;
        self.grab = Grab::Blank;
        self.grabbed = None;
    }

    /// 尺寸读数（`宽 × 高`，**像素**不是点）。
    ///
    /// 用户拿它判断"我框的是不是那块"，所以它必须与 [`Field::to_pixels`] 同源：
    /// 一个报点、一个裁像素，Retina 屏上就差一倍（H4 那一档本机量不到，判据先立住）。
    pub fn size_label(&self, field: &Field) -> Option<String> {
        let px = self.rect.and_then(|rect| field.to_pixels(rect))?;
        // 报的是**宽高**，不是左上角——上一版这里写成 `px.x × px.w`，测试当场抓到：
        // 一个 400×300 的框会念成"200 × 400"，用户照着这个数字判断自己框对了没有，就是假信息。
        Some(format!(
            "{} × {}",
            px.w.round().max(0.0) as i32,
            px.h.round().max(0.0) as i32
        ))
    }
}

/// 改大小：把手 `index` 跟着点走，**与它相对的那个角**钉死不动。
fn resize(rect: Rect, index: u8, to: Pos2) -> Rect {
    // 只推该动的那一两条边；**拖过对边就停住**（每根动的边都拿对面那条夹一下）。
    // 让它翻面会出两种读数相同、语义相反的东西：用户想放大，结果缩小，而框看起来还是"正常的一块"。
    let (mut x0, mut y0, mut x1, mut y1) = (rect.min.x, rect.min.y, rect.max.x, rect.max.y);
    match index {
        0 => {
            x0 = to.x.min(x1);
            y0 = to.y.min(y1);
        }
        1 => y0 = to.y.min(y1),
        2 => {
            x1 = to.x.max(x0);
            y0 = to.y.min(y1);
        }
        3 => x1 = to.x.max(x0),
        4 => {
            x1 = to.x.max(x0);
            y1 = to.y.max(y0);
        }
        5 => y1 = to.y.max(y0),
        6 => {
            x0 = to.x.min(x1);
            y1 = to.y.max(y0);
        }
        _ => x0 = to.x.min(x1),
    }
    Rect::from_min_max(pos2(x0, y0), pos2(x1, y1))
}

// 缝 `GAP`、翻上下 `BarSide`/`side()`、药丸矩形 `rect()`、两端留白 `SIDE_PAD`、颗间缝
// `BTN_GAP` 都在 `system/bar.rs` 那一份共用件里（CHG-011 抽公共：贴图那条药丸与遮罩这条
// 读同一套几何，各写一份迟早分岔）。这里只留遮罩自己的行高 `BAR_H`、线稿槽宽 `ICON_W`
// 与下面这一处调用，线稿 `icon_button` 的画法一字不动。

/// 这一刻药丸占的那一块矩形；**没有有效选区就没有药丸**。
///
/// 绘制与"按下落在哪"的判定都读这一处。两处各算一次迟早分岔——上一版正是分岔出来的：
/// 按下落在药丸上被当成"在空白处起新框"，框缩成一个点、药丸那一帧没了位置，
/// 那颗按钮再也收不到完整的按下—抬起。
///
/// 宽度**按内容定**（[`bar_width`]）：全部线稿颗 + 一句尺寸读数 + 两端留白，不再写死一个
/// 300——内容只占约一半、右边空一大截黑底，正是用户那句"按钮文字排列好丑"（2026-11-11）。
/// `label_w` 由 [`label_width`] 用绘制同一字体量出，按下判定与绘制各按自己那一刻的选区量，
/// 读的都是同一个 `next`，不会分岔。
fn bar_of(sel: &Selection, field: &Field, label_w: f32, tool_active: bool) -> Option<Rect> {
    let rect = sel
        .rect
        .filter(|rect| rect.width() > 0.0 && rect.height() > 0.0)?;
    let size = Vec2::new(bar_width(label_w, tool_active), BAR_H);
    Some(bar::rect(rect, size, bar::side(rect, size, field.bounds)))
}

/// 药丸宽 = 全部线稿颗（[`BAR_ICONS`]）+ 选中工具时两颗定宽下拉（[`OPT_W`]）+ 一句尺寸读数，
/// 再加上各项之间的 [`bar::BTN_GAP`] 与两端 [`bar::SIDE_PAD`]。项数按"线稿颗 + 下拉 + 读数"数，
/// 缝 = 项数 − 1；没有读数（屏比例不可信、量不出像素）时少一项、也少一条缝。与贴图那条
/// `width_for` 同一条"按内容定宽"的纪律，只是这边末颗是变宽的尺寸文字而非定宽文字按钮。
fn bar_width(label_w: f32, tool_active: bool) -> f32 {
    let icons = BAR_ICONS.len() as f32;
    let opts = if tool_active { 2.0 } else { 0.0 };
    let label = if label_w > 0.0 { 1.0 } else { 0.0 };
    let gaps = icons + opts + label - 1.0;
    2.0 * bar::SIDE_PAD + icons * ICON_W + opts * OPT_W + gaps * bar::BTN_GAP + label_w
}

/// 量一句尺寸读数在**绘制用的同一字体**（Body）下占多宽（点）。`ui.label` 默认就是 Body，
/// 这里用同一字体量，量出来的宽与画出来的宽同源——药丸才能恰好贴合内容。
fn label_width(ui: &egui::Ui, sel: &Selection, field: &Field) -> f32 {
    let Some(text) = sel.size_label(field) else {
        return 0.0;
    };
    let font = egui::TextStyle::Body.resolve(ui.style());
    ui.fonts_mut(|f| {
        f.layout_no_wrap(text, font, egui::Color32::TRANSPARENT)
            .size()
            .x
    })
}

// 三条常量的边界钉在**编译期**：写成运行期断言会被判"常量比较恒真"而形同虚设，
// 它要的效果从来不是"跑一下看看"，是"谁改坏了就编译不过"。
/// 压暗不许等于"没压"(255)，也不许等于"看不见原图"(0)；具体档位由走查 W-6.2 定（Q-005）。
const _: () = assert!(DIM >= 60 && DIM <= 200);
/// 单击阈值必须明显小于把手命中半径，否则"点在把手上"会先被判成单击，八个把手靠内那圈就废了。
const _: () = assert!(CLICK_SLOP > 0.0 && CLICK_SLOP < HANDLE_RADIUS);
/// 八个把手 = 4 角 + 4 边。改了它，[`handle_center`] 的编号与 [`grab`] 的命中会一起分岔。
const _: () = assert!(HANDLE_COUNT == 8);
/// 药丸不许被把手压住：把手有一半探在选区外，缝至少得有那一半宽。取等号时
/// 正下方那颗把手的外缘正好贴上药丸的上缘（相接、不相叠）；要把把手画大，缝也得跟着大。
const _: () = assert!(GAP >= HANDLE_SIZE / 2.0);

// ---- 遮罩那一扇窗（egui 侧） ----

/// 遮罩能对外发出的**全部意图**。一条枚举、一处映射——与贴图那一条纪律同形：
/// 键、按钮、右键若各写一套，出的就是"点按钮有效、按键没反应"那类只对一半的缺陷。
#[derive(Debug, Clone, PartialEq)]
pub enum MaskOut {
    /// 按对勾 / 回车 / 复制 / 保存：把这扇窗自己的选区交出去确认（点=那块屏的本地点坐标）。
    /// **选区状态不再回传主窗**——上一版每帧回传，两扇子窗都在往同一格写，
    /// 结果就是"拖动一闪一闪"和"在左边拖、右边的旧框自己跳出来"（用户 2026-11-09 晚原话）。
    Confirm { display: u8, rect: Rect },
    /// 复制那颗：同样先确认，差别在落点（钉住 vs 只写剪贴板）
    ConfirmCopy { display: u8, rect: Rect },
    /// 保存那颗
    ConfirmSave { display: u8, rect: Rect },
    /// esc 或点空白：什么都不留
    Cancel,
}

/// 遮罩窗的标题。**抬层那一步按它找窗**（`system::cover`），所以这一格只许有一处定义。
pub fn mask_title(display: u8) -> String {
    format!("IceTranslator 选区 {display}")
}

/// 光标此刻落在哪块屏：返回那块屏的序号（`-D` 那个 display）。
///
/// 输入是「全局光标 + 各屏的全局矩形」，纯函数，无头可断言（副屏负偏移那条照样成立）。
/// 认窗高亮只许发生在光标真所在的那块屏上——另一扇遮罩读不到指针就退回本地 (0,0)，
/// 拿那个点去认窗会把它左上角那扇窗点亮，正是用户那句「两块屏上的窗都亮了、只有周边灰」。
///
/// 光标问不到（`None`：非 mac、无头不在主线程）或落在屏间缝隙里 → `None`，
/// 调用方退回「各遮罩沿用各自的指针判法」（与今天一致，不许因为问不到就永不认窗）。
pub fn lit_display(cursor: Option<Pos2>, screens: &[(u8, Rect)]) -> Option<u8> {
    let point = cursor?;
    screens
        .iter()
        .find(|(_, rect)| rect.contains(point))
        .map(|(display, _)| *display)
}

fn slot_key(display: u8) -> Id {
    Id::NULL.with("ice-selection.mask.slot").with(display)
}

/// 「固定」热键（F3）在遮罩还开着时递进来的"替我按下对勾"信号。
///
/// 与"选区不回传主窗"不冲突：主窗只递一个**触发**，遮窗自己手里有没有真框自己判、
/// 自己把那块矩形交出去（走的还是按对勾/回车那同一条出口）。之所以不能主窗直接替它确认：
/// 那一刻哪块屏、框在哪，只有正在拖的那一扇遮窗知道。
fn confirm_key() -> Id {
    Id::NULL.with("ice-selection.confirm-request")
}

pub fn request_confirm(ctx: &egui::Context) {
    ctx.data_mut(|d| d.insert_temp(confirm_key(), true));
}

fn confirm_requested(ctx: &egui::Context) -> bool {
    ctx.data(|d| d.get_temp::<bool>(confirm_key()) == Some(true))
}

pub fn clear_confirm_request(ctx: &egui::Context) {
    ctx.data_mut(|d| d.remove::<bool>(confirm_key()));
}

/// 这扇遮罩自己的选区状态存在根上下文的 data 里，**按屏编号分开**。
/// 放这里而不是放 `AppState`：只有画它的那一帧会读写它，绕一圈主窗就会看到旧值（闪），
/// 而两扇窗同时写一份更会串（左屏拖动、右屏旧框跳出来）。
///
/// 格子里带一个轮号（D-42）：临时态不随遮罩关窗而清，确认那一支只把阶段推到 Confirmed、
/// 框还留在格子里，下一轮开遮罩读到的就是上一轮的框（用户 2026-11-10 原话「下次我打开
/// 还有上次的选框」）。读的时候轮号对不上就当空白，写的时候原地盖掉——与定格纹理那格
/// 同一个形状（D-41），每屏永远只有一格，不随轮数攒。
fn sel_key(display: u8) -> Id {
    Id::NULL.with("ice-selection.mask.sel").with(display)
}

/// 从那一格里读这一轮的选区。轮号对不上就是新一轮，读空白（D-42）。
/// 单独一个函数：判据直接拿真存储走一遍，不用在测试里抄一份读法。
fn load_selection(d: &egui::util::IdTypeMap, display: u8, frozen_id: u64) -> Selection {
    d.get_temp::<(u64, Selection)>(sel_key(display))
        .filter(|(old_gen, _)| *old_gen == frozen_id)
        .map(|(_, sel)| sel)
        .unwrap_or_default()
}

/// 这一轮**任一扇屏**已经拖出过框没有（REQ-108 的"用户还没有拖出选区"问的是人，
/// 不是屏）。存在根上下文、按轮号 keyed：A 屏画了框，B 屏的认窗高亮与单击截整窗
/// 都得停——不然鼠标挪到另一块屏还能点亮窗、单击还会把那一扇截走（D-45）。
/// 一轮之内只增不减：框一旦画出，到确认/esc 结束这一轮之前都算"已经有框"。
fn any_selection(root: &egui::Context, frozen_id: u64) -> bool {
    root.data(|d| {
        d.get_temp::<bool>(Id::NULL.with("ice-selection.any-sel").with(frozen_id))
            .unwrap_or(false)
    })
}

fn mark_selection(root: &egui::Context, frozen_id: u64) {
    root.data_mut(|d| d.insert_temp(Id::NULL.with("ice-selection.any-sel").with(frozen_id), true));
}

// ---- 标注层与工具态（CHG-014） ----

fn ann_key(display: u8) -> Id {
    Id::NULL.with("ice-selection.mask.ann").with(display)
}

/// 取这一屏这一轮的标注层；轮号对不上就换一张全新的透明层（照选区那格 D-42 同形）。
///
/// **存在根上下文的 data 里**：遮罩子窗画它、主窗确认时裁它，两边读的是同一份 `Arc`。
/// 不绕帧回传——贴图那条已经验证过，状态绕主窗一圈必闪，多子窗还会串格。
/// 每屏永远只有一格：新一轮原地顶掉旧的，不随轮数攒（一层就是几十 MB）。
pub fn annotation_objects(
    ctx: &egui::Context,
    display: u8,
    frozen_id: u64,
) -> std::sync::Arc<std::sync::Mutex<ObjectList>> {
    let key = ann_key(display);
    ctx.data_mut(|d| {
        if let Some((old_gen, objects)) =
            d.get_temp::<(u64, std::sync::Arc<std::sync::Mutex<ObjectList>>)>(key)
        {
            if old_gen == frozen_id {
                return objects;
            }
        }
        let fresh = std::sync::Arc::new(std::sync::Mutex::new(ObjectList::new()));
        d.insert_temp(key, (frozen_id, fresh.clone()));
        fresh
    })
}

/// 标注的显示纹理：**对象列表一变就重传一次**（缓存认 frozen_id + revision）。
/// 一件没画（revision 0）返回 None——不传纹理也不画，保证没动工具时与现状逐像素同。
/// 走对象引擎 `render`：逐件画几何＋对自身擦除痕迹减 alpha（H1 与旧像素同源）。
fn annotation_texture(
    child: &egui::Context,
    display: u8,
    frozen_id: u64,
    objects: &std::sync::Arc<std::sync::Mutex<ObjectList>>,
    background: &image::RgbaImage,
) -> Option<egui::TextureId> {
    let guard = objects.lock().ok()?;
    let rev = guard.revision;
    if rev == 0 {
        return None;
    }
    let key = Id::NULL.with("ice-selection.ann-tex").with(display);
    if let Some((old_gen, old_rev, old)) =
        child.data(|d| d.get_temp::<(u64, u64, egui::TextureHandle)>(key))
    {
        if old_gen == frozen_id && old_rev == rev {
            return Some(old.id());
        }
    }
    let Ok(metrics) = FileMetrics::open() else {
        return None;
    };
    let image0 = render(&guard.objects, background, &metrics);
    drop(guard);
    let (w, h) = (image0.width(), image0.height());
    let image = egui::ColorImage::from_rgba_unmultiplied([w as usize, h as usize], image0.as_raw());
    let texture = child.load_texture("ice-selection.ann", image, egui::TextureOptions::LINEAR);
    let id = texture.id();
    child.data_mut(|d| d.insert_temp(key, (frozen_id, rev, texture)));
    Some(id)
}

/// 药丸上的标注工具。一次只激活一件；再点当前这件或按 esc 回到选区态（REQ-006）。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Tool {
    /// 画框
    Rect,
    /// 带箭头的线
    Arrow,
    /// 直线
    Line,
    /// 铅笔（自由绘制）
    Pencil,
    /// 序号标（单击落一个，自动加一）
    Number,
    /// 文字（单击唤起输入，确认后落层）
    Text,
    /// 马赛克（拖区从冻结背景盒式打码）
    Mosaic,
    /// 橡皮（按像素擦痕迹）
    Eraser,
}

/// 药丸颗 → 它激活的工具；确认三颗不是工具，返回 None。
fn tool_of(icon: BarIcon) -> Option<Tool> {
    match icon {
        BarIcon::Rect => Some(Tool::Rect),
        BarIcon::Arrow => Some(Tool::Arrow),
        BarIcon::Line => Some(Tool::Line),
        BarIcon::Pencil => Some(Tool::Pencil),
        BarIcon::Number => Some(Tool::Number),
        BarIcon::Text => Some(Tool::Text),
        BarIcon::Mosaic => Some(Tool::Mosaic),
        BarIcon::Eraser => Some(Tool::Eraser),
        _ => None,
    }
}

/// 工具态那一扇窗自己的运行态：激活哪件 + 这一次手势的锚点与上一帧落点。
/// **与选区状态分开存**——工具手势不许碰 `next.rect`（互斥那条的一半就靠这个分离）。
/// `last` 给铅笔/橡皮用：它俩是拖到哪画到哪，每帧落上一帧到这一帧那一小段。
#[derive(Clone, PartialEq)]
struct ToolState {
    tool: Option<Tool>,
    anchor: Pos2,
    last: Pos2,
    dragging: bool,
    /// 序号标下一个要落的号：单击自增，新一轮从 1 起（某截图工具 默认同口径，
    /// 它的「从 0 开始」是可选开关）。
    next_number: u32,
    /// 文字工具开着的那条输入：落点 + 正在打的字。`None` = 没在输入。
    text_input: Option<(Pos2, String)>,
    /// 粗细档（REQ-003）：默认中档。运行态，不进 config；落层与预览同源读。
    thickness_idx: usize,
    /// 色板档（REQ-003）：默认红。马赛克与橡皮不读它（一个取背景色、一个只清 alpha）。
    color_idx: usize,
    /// 铅笔/马赛克这一次拖拽累积的点（点空间）：松手建**一个** Pencil 或 Mosaic 对象
    /// （CHG-016 T-002；马赛克 2026-11-18 改涂抹式，与铅笔同走这条累积）。
    stroke_points: Vec<Pos2>,
    /// 橡皮这一次拖拽累积的轨迹点（点空间）：松手对这些点做一次 erase_pen（一条撤销）。
    erase_points: Vec<Pos2>,
    /// 正在拖动改位置的既有对象下标（CHG-016 T-005）：按下命中已有对象时置上，
    /// 拖到哪挪到哪，松手清空。`None` = 不在拖既有对象（那就是在画新的）。
    moving: Option<usize>,
    /// 拖动既有对象时上一帧的落点（点空间），用来算每帧位移。
    move_last: Pos2,
}

impl Default for ToolState {
    fn default() -> Self {
        Self {
            tool: None,
            anchor: Pos2::ZERO,
            last: Pos2::ZERO,
            dragging: false,
            next_number: 1,
            text_input: None,
            thickness_idx: 1,
            color_idx: 0,
            stroke_points: Vec::new(),
            erase_points: Vec::new(),
            moving: None,
            move_last: Pos2::ZERO,
        }
    }
}

fn tool_key(display: u8) -> Id {
    Id::NULL.with("ice-selection.mask.tool").with(display)
}

fn load_tool_state(d: &egui::util::IdTypeMap, display: u8, frozen_id: u64) -> ToolState {
    d.get_temp::<(u64, ToolState)>(tool_key(display))
        .filter(|(old_gen, _)| *old_gen == frozen_id)
        .map(|(_, ts)| ts)
        .unwrap_or_default()
}

/// 工具手势的点 → 物理像素。**与裁剪同一条换算**（[`Field::to_pixels`]）：
/// 119 那版直线/箭头/铅笔/橡皮把点坐标直接喂给物理像素的层函数，1.0× 屏上看不出、
/// 2× 屏上会差一倍——八件工具的手势全收进这一个漏斗，不许再旁路。
fn point_to_px(field: Field, p: Pos2) -> Option<(f32, f32)> {
    let r = field.to_pixels(Rect::from_min_size(p, Vec2::ZERO))?;
    Some((r.x, r.y))
}

/// 序号标落一件：点换像素 → 建 Serial 对象（号取对象列表的 next_serial 并自增，默认 1 起）。
fn stamp_number(
    ts: &ToolState,
    objects: &std::sync::Arc<std::sync::Mutex<ObjectList>>,
    field: Field,
    at: Pos2,
) {
    let Some(scale) = field.scale() else { return };
    let Some(center) = point_to_px(field, at) else {
        return;
    };
    if let Ok(mut ol) = objects.lock() {
        ol.push_serial(
            center,
            BADGE_RADIUS[ts.thickness_idx] * scale,
            PALETTE[ts.color_idx],
        );
    }
}

/// 把开着的那条文字输入落成 Text 对象：取走 → 空白不落 → 点换像素 → 存文本与字号。
/// 字体读不出只 warn（NFR-3）。
fn commit_text_input(
    ts: &mut ToolState,
    objects: &std::sync::Arc<std::sync::Mutex<ObjectList>>,
    field: Field,
) {
    let Some((at, text)) = ts.text_input.take() else {
        return;
    };
    let text = text.trim();
    if text.is_empty() {
        return;
    }
    let Some(scale) = field.scale() else { return };
    let Some(p) = point_to_px(field, at) else {
        return;
    };
    if FileMetrics::open().is_err() {
        tracing::warn!("文字标注：本机字体读不出，这一条没落");
        return;
    }
    let owned = text.to_string();
    if let Ok(mut ol) = objects.lock() {
        ol.push(AnnoObject {
            shape: Shape::Text {
                at: p,
                text: owned,
                font_size: TEXT_SIZE[ts.thickness_idx] * scale,
                color: PALETTE[ts.color_idx],
            },
            erased: Vec::new(),
        });
    }
}

fn viewport_of(display: u8) -> egui::ViewportId {
    egui::ViewportId::from_hash_of(Id::NULL.with("ice-selection.mask").with(display))
}

/// 叫醒某一扇遮罩重画。光标跨屏时「认窗高亮的那扇」要换人，而子窗不会自己醒
/// （指针事件只到鼠标底下那一扇，另一扇收不到任何事件），得由主窗点名让它重画一次。
pub fn wake_mask(ctx: &egui::Context, display: u8) {
    ctx.request_repaint_of(viewport_of(display));
}

/// 取走这一扇遮罩攒下的意图（帧循环每帧一次）。
pub fn take_outs(ctx: &egui::Context, display: u8) -> Vec<MaskOut> {
    let key = slot_key(display);
    let slot = ctx.data_mut(|d| {
        d.get_temp::<std::sync::Arc<std::sync::Mutex<Vec<MaskOut>>>>(key)
            .unwrap_or_else(|| {
                let fresh = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
                d.insert_temp(key, fresh.clone());
                fresh
            })
    });
    slot.lock()
        .map(|mut q| std::mem::take(&mut *q))
        .unwrap_or_default()
}

fn slot_of(ctx: &egui::Context, display: u8) -> std::sync::Arc<std::sync::Mutex<Vec<MaskOut>>> {
    let key = slot_key(display);
    ctx.data_mut(|d| {
        d.get_temp::<std::sync::Arc<std::sync::Mutex<Vec<MaskOut>>>>(key)
            .unwrap_or_else(|| {
                let fresh = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
                d.insert_temp(key, fresh.clone());
                fresh
            })
    })
}

/// 一扇遮罩这一帧要画的东西。
pub struct MaskScreen {
    /// `-D` 的序号，用来命名窗口与配对
    pub display: u8,
    /// 那块屏的点尺寸
    pub points: Vec2,
    /// 那块屏的**物理像素**尺寸（= 冻结帧的真实尺寸）。标注层按它建，
    /// 手势的点坐标经 `Field::to_pixels` 换进来——与裁剪同一条换算，Retina 上不会差一倍。
    pub px: Vec2,
    /// 那块屏的全局左上角（与系统窗口表同一坐标系）。认窗靠它把全局矩形换回本地。
    pub origin: Pos2,
    /// 认窗那一份快照（整段遮罩期间不变，见 `system::windows::Snapshot`）
    pub windows: std::sync::Arc<crate::system::windows::Snapshot>,
    /// 冻结帧的纹理（**由根上下文加载**，与贴图那一条同一个做法）
    pub texture: egui::TextureId,
    /// 这一轮冻结的轮号。选区那格带着它存取：轮号一换就当空白（D-42）
    pub frozen_id: u64,
    /// 光标此刻是不是真在这块屏上（由帧循环问窗口服务器算出，见 [`lit_display`]）。
    /// **认窗高亮只在这块屏上做**：另一扇遮罩读不到指针会退回本地 (0,0)，拿那个点认窗
    /// 会把它左上角那扇窗点亮（用户那句「两块屏上的窗都亮了」）。问不到光标时帧循环给
    /// `true`，退回各窗自己的指针判法（与今天一致）。压暗那一档不受它影响。
    pub cursor_here: bool,
    /// 冻结帧原图（物理像素）。马赛克从它取色打码——与遮罩画的是同一份 `Arc`。
    pub frame: std::sync::Arc<image::RgbaImage>,
}

/// 画一扇遮罩。每帧对该 show 的每块屏各调一次（CHG-009 事实 25 那条：deferred viewport 是每帧 show）。
///
/// 压暗 = **整屏一张冻结图乘一个暗档**，选区那块再按原亮度重画一遍。
/// 这里刻意不用透明窗口、不用鼠标穿透：那两个属性正是 D-29 那次吞掉全部点击与键盘的元凶，
/// 而乘暗这一条路在探针里逐点核对过（红 `(220,30,30)` → `(99,13,13)`）。
pub fn show(ctx: &egui::Context, screen: &MaskScreen) {
    // `-D` 的序号。闭包要 `'static`，所以一进来就按值带走
    let display = screen.display;
    let viewport = viewport_of(display);
    let builder = egui::ViewportBuilder::default()
        .with_title(mask_title(display))
        .with_decorations(false)
        // 透明底是为了根除按 F1 那一瞬白屏（用户 2026-11-13 真机复报）。deferred 子视口拿不到
        // eframe 只给根窗那层"建窗先藏、第一帧画完再露"的防白闪（wgpu_integration 的 create_window），
        // 而不透明窗的默认原生底是白——窗先露、第一帧 wgpu 还没把冻结图盖上那一小段就是白。改成
        // 透明底，那一小段透出的是桌面（冻结图本就是桌面快照，几乎一样）；第一帧把冻结图不透明铺满
        // 整屏（下方 painter.image），稳态仍是整屏不透明。"先藏再露"这条路走不通：eframe 对不可见的
        // deferred 视口直接不跑 UI 闭包（wgpu_integration「Nothing will be shown」），藏了就永远画不出第一帧。
        .with_transparent(true)
        .with_has_shadow(false)
        // 这一档只是**下限**：真盖住 Dock 与菜单栏靠 `system::cover` 那一次原生抬层，
        // 头几帧幂等重下、收敛即收手。留着它是为了让抬层失败时退回"至少还置顶在普通窗之上"。
        .with_always_on_top()
        .with_position(egui::pos2(0.0, 0.0))
        .with_inner_size(screen.points);

    let texture = screen.texture;
    let points = screen.points;
    let px = screen.px;
    let origin = screen.origin;
    let frozen_id = screen.frozen_id;
    let cursor_here = screen.cursor_here;
    let windows = screen.windows.clone();
    let frame = screen.frame.clone();
    // 闭包要 'static：把这一帧要用的几个数**按值**带走，不带引用

    let root = ctx.clone();
    let slot = slot_of(ctx, display);

    ctx.show_viewport_deferred(viewport, builder, move |ui, _class| {
        // N0 读数：这一扇遮罩**真的画上第一帧**的时刻——白屏的终点就是这一行。
        // 按轮号作废，一轮一块屏只记一次；只读缓存，不参与任何判定。
        let paint_key = Id::NULL.with("ice-selection.first-paint").with(display);
        if ui.ctx().data(|d| d.get_temp::<u64>(paint_key)) != Some(frozen_id) {
            // `display` 这个名字在 `tracing` 的字段语法里被占用，先在宏外头落成整数
            let screen = display;
            tracing::debug!(
                screen,
                t_ms = crate::system::probe::now_ms(),
                "遮罩：第一帧已画"
            );
            ui.ctx().data_mut(|d| d.insert_temp(paint_key, frozen_id));
        }
        let child = ui.ctx().clone();
        let field = Field::new(Rect::from_min_size(Pos2::ZERO, points), points);
        // 这一扇窗自己的选区：**读写都在这一帧里完成**，不绕主窗一圈。
        // 轮号对不上就是新一轮截图，读空白（D-42：上一轮的框不许跟过来）
        let mut next = ui.ctx().data(|d| load_selection(d, display, frozen_id));
        let mut changed = false;
        // 标注层与工具态：都按 (屏, 轮号) 存取，与选区那格同形（D-42：新一轮读空白）。
        // 层从根上下文取——确认时主窗拿的是同一份 Arc，不绕帧回传。
        let ann_field = Field::new(Rect::from_min_size(Pos2::ZERO, points), px);
        let objects = annotation_objects(&root, display, frozen_id);
        let mut ts = ui.ctx().data(|d| load_tool_state(d, display, frozen_id));
        let tool_before = ts.clone();
        let here = ui.input(|i| i.pointer.interact_pos()).unwrap_or(Pos2::ZERO);
        let holding = ui.input(|i| i.pointer.button_down(egui::PointerButton::Primary));
        // 认窗（REQ-108）：**光标真在这块屏、这一轮还没框、也没在拖**的时候，鼠标底下那扇应用窗亮出来。
        // 已经有框就不再认——那会儿用户在调自己那个框，亮另一扇窗只会跟他抢注意力。
        // "还没框"问的是**这一轮所有屏**（any_selection）：A 屏画了框，B 屏也不许再认（D-45）。
        // `cursor_here` 把另一块屏挡在外面：那扇遮罩读不到指针，`here` 会是本地 (0,0)，
        // 不挡的话它左上角那扇窗会被点亮（用户那句「两块屏上的窗都亮了、只有周边灰」）。
        let hover = if cursor_here
            && next.rect.is_none()
            && !any_selection(&root, frozen_id)
            && !holding
            && ts.tool.is_none()
        {
            windows.candidate(origin, here, field.bounds)
        } else {
            None
        };
        let full = ui.max_rect();
        // 整屏压暗
        ui.painter().image(
            texture,
            full,
            Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)),
            egui::Color32::from_rgb(DIM, DIM, DIM),
        );
        // 认出的那扇窗：按原亮度亮出来 + 一圈强调色边。**不画把手**——它不是给用户调的，
        // 单击就整扇拿下（画了把手反而像在说"你可以拖它"）。
        if let (None, Some(rect)) = (next.rect, hover) {
            let uv = Rect::from_min_max(
                Pos2::new(rect.min.x / points.x, rect.min.y / points.y),
                Pos2::new(rect.max.x / points.x, rect.max.y / points.y),
            );
            ui.painter().image(texture, rect, uv, egui::Color32::WHITE);
            ui.painter().rect_stroke(
                rect,
                0.0,
                egui::Stroke::new(2.0, HOVER_EDGE),
                egui::StrokeKind::Inside,
            );
        }
        // 选区那块按原亮度 + 白边 + 八个把手
        if let Some(rect) = next.rect {
            let uv = Rect::from_min_max(
                Pos2::new(rect.min.x / points.x, rect.min.y / points.y),
                Pos2::new(rect.max.x / points.x, rect.max.y / points.y),
            );
            ui.painter().image(texture, rect, uv, egui::Color32::WHITE);
            ui.painter().rect_stroke(
                rect,
                0.0,
                egui::Stroke::new(1.0, egui::Color32::WHITE),
                egui::StrokeKind::Outside,
            );
            for handle in 0..HANDLE_COUNT {
                let center = handle_center(rect, handle);
                ui.painter().rect_filled(
                    Rect::from_center_size(center, Vec2::splat(HANDLE_SIZE)),
                    0.0,
                    egui::Color32::WHITE,
                );
            }
        }

        // 标注层盖在压暗与选区之上：笔迹是用户画的内容，不被暗档乘色。
        // 空层不传纹理也不画——没动工具时这一扇与现状逐像素同（REQ-001）。
        if let Some(tex) = annotation_texture(&child, display, frozen_id, &objects, &frame) {
            ui.painter().image(
                tex,
                full,
                Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)),
                egui::Color32::WHITE,
            );
        }

        // 键盘：esc 取消、回车确认、command+c／ctrl+c 复制这一张。收不收得到取决于应用是否前台——
        // 那一步在 `cover::focus`，头几帧重下、收敛即收手（实测激活要一两帧才落地，"发一次就当成功"是错的）。
        // **三个键都先过状态机再交意图**：esc 要把这一扇窗自己的框清掉，状态各窗自持，
        // 主窗不会替它清，留着它下一次开遮罩就带着上一次的框。
        let mut keys: Vec<MaskOut> = child.input(|i| {
            i.events
                .iter()
                .filter_map(|e| match e {
                    egui::Event::Key {
                        key: egui::Key::Escape,
                        pressed: true,
                        ..
                    } => {
                        // esc 三层由近及远：先收文字输入（不落字，esc 是取消键）、
                        // 再退工具（REQ-006，框还在）、选区态才是取消遮罩。
                        if ts.text_input.is_some() {
                            ts.text_input = None;
                            None
                        } else if ts.tool.is_some() {
                            ts.tool = None;
                            ts.dragging = false;
                            None
                        } else {
                            next.cancel();
                            Some(MaskOut::Cancel)
                        }
                    }
                    egui::Event::Key {
                        key: egui::Key::Enter,
                        pressed: true,
                        ..
                    } => {
                        // 输入开着：回车 = 把这条字落层，不是确认遮罩
                        if ts.text_input.is_some() {
                            commit_text_input(&mut ts, &objects, ann_field);
                            None
                        } else {
                            next.confirm();
                            next.rect.map(|rect| MaskOut::Confirm { display, rect })
                        }
                    }
                    // **command+c（mac）/ ctrl+c（其他）= 药丸上那颗「复制」按了一下**（某截图工具 同款，
                    // 用户 2026-11-11 第 5 条）。只在**手里真有框**时兑现：没框就什么都不做，
                    // 不许替用户猜一块全屏当"这张图"复制出去。
                    egui::Event::Key {
                        key: egui::Key::C,
                        pressed: true,
                        modifiers,
                        ..
                    } if (modifiers.command || modifiers.ctrl)
                        && !modifiers.alt
                        && !modifiers.shift
                        && next.rect.is_some() =>
                    {
                        // 复制的是落完字的成品：开着的输入先落层
                        commit_text_input(&mut ts, &objects, ann_field);
                        next.rect.map(|rect| MaskOut::ConfirmCopy { display, rect })
                    }
                    _ => None,
                })
                .collect()
        });
        // 「固定」热键（F3）按下来的那一帧：手里真有框的这一扇遮窗替用户按下对勾。
        // 并进上面同一个 `keys`，走同一条推意图的出口——不新开 queue.push / wake_root 的点。
        // 没框就留着这个请求（下一次有框再兑现），主窗那侧在新轮开始时清掉，不跨轮误触。
        if confirm_requested(&child) && next.rect.is_some() {
            clear_confirm_request(&child);
            commit_text_input(&mut ts, &objects, ann_field);
            next.confirm();
            if let Some(rect) = next.rect {
                keys.push(MaskOut::Confirm { display, rect });
            }
        }
        if !keys.is_empty() {
            changed = true;
            if let Ok(mut queue) = slot.lock() {
                queue.extend(keys);
            }
            wake_root(&root);
        }
        // 撤销（CHG-015 REQ-001）：command+z / ctrl+z 撤最近一笔标注。只动层 + 叫醒自己，
        // **不推意图**——撤销不是遮罩的收/发，主窗不该被打扰。文字输入开着时让给输入框
        // （TextEdit 自己吃 cmd+z）；repeat 滤掉长按连发，一下只撤一笔。
        let undo_pressed = child.input(|i| {
            i.events.iter().any(|e| {
                matches!(
                    e,
                    egui::Event::Key {
                        key: egui::Key::Z,
                        pressed: true,
                        repeat: false,
                        modifiers,
                        ..
                    } if (modifiers.command || modifiers.ctrl)
                        && !modifiers.alt
                        && !modifiers.shift
                )
            })
        });
        // 重做（CHG-016 REQ-003 / Q-003）：command+shift+z 或 ctrl+y 补回最近撤掉的一笔。
        let redo_pressed = child.input(|i| {
            i.events.iter().any(|e| {
                matches!(
                    e,
                    egui::Event::Key {
                        key: egui::Key::Z,
                        pressed: true,
                        repeat: false,
                        modifiers,
                        ..
                    } if (modifiers.command || modifiers.ctrl) && modifiers.shift
                ) || matches!(
                    e,
                    egui::Event::Key {
                        key: egui::Key::Y,
                        pressed: true,
                        repeat: false,
                        modifiers,
                        ..
                    } if modifiers.ctrl && !modifiers.shift
                )
            })
        });
        if ts.text_input.is_none() {
            if undo_pressed {
                if let Ok(mut ol) = objects.lock() {
                    if ol.undo() {
                        child.request_repaint();
                    }
                }
            } else if redo_pressed {
                if let Ok(mut ol) = objects.lock() {
                    if ol.redo() {
                        child.request_repaint();
                    }
                }
            }
        }

        // 文字输入框：**先画再派发手势**——「按下落在框外 = 确认这条输入」要先知道框在哪。
        // 键盘三键在上面已经先处理过（回车落字、esc 收输入），这里 TextEdit 只吃字符。
        let mut input_rect = None;
        if let Some((at, text)) = ts.text_input.as_mut() {
            let id = Id::NULL.with("ice-selection.text-input").with(display);
            let area = egui::Area::new(id)
                .fixed_pos(*at)
                .order(egui::Order::Foreground)
                .show(ui.ctx(), |ui| {
                    egui::Frame::NONE
                        .fill(egui::Color32::from_rgba_premultiplied(28, 28, 30, 235))
                        .stroke(egui::Stroke::new(1.0, egui::Color32::WHITE))
                        .inner_margin(egui::Margin::symmetric(8, 4))
                        .corner_radius(egui::CornerRadius::same(6))
                        .show(ui, |ui| {
                            let edit = egui::TextEdit::singleline(text)
                                .id(id.with("edit"))
                                .desired_width(180.0)
                                .hint_text("输入文字，回车落下去")
                                .show(ui);
                            if !edit.response.has_focus() {
                                edit.response.request_focus();
                            }
                        });
                });
            input_rect = Some(area.response.rect);
        }

        // 鼠标：按下落在哪 → 拖动 → 抬起。**状态就在这一扇窗自己身上读写**，
        // 不经过主窗一圈（上一版经过，于是"拖了没框"、"一闪一闪"、"左屏拖右屏跳框"）
        //
        // **落在药丸上的那一下不算框选手势。** 先前没有这一条：按下落在对勾上，`press` 先把
        // 旧框换成一个零尺寸的点，药丸那一块下一帧就没了位置，那颗按钮收不到完整的
        // "按下—抬起"——真机上的表现就是"点了对勾什么都不钉"（用户 2026-11-09 晚第二条原话）。
        let bar_now = bar_of(
            &next,
            &field,
            label_width(ui, &next, &field),
            ts.tool.is_some(),
        );
        let on_bar = bar_now.is_some_and(|bar| bar.contains(here));
        let pressed = ui.input(|i| i.pointer.button_pressed(egui::PointerButton::Primary));
        let released = ui.input(|i| i.pointer.button_released(egui::PointerButton::Primary));
        if ts.tool.is_none() {
            // 选区态：既有手势逐字不变（REQ-006 的"默认与现状一致"就靠这一支原样保留）
            if pressed && !on_bar {
                next.press(&field, here, HANDLE_RADIUS);
                changed = true;
            } else if ui.input(|i| i.pointer.button_down(egui::PointerButton::Primary)) {
                let before = next.rect;
                next.drag(&field, here);
                changed = next.rect != before;
            }
            // 抬起只在"真的拖过"之后交给状态机：没进 Dragging 的抬起（药丸上那一下）不许去动框，
            // 它会把 `previous` 那份旧值当结果还回来。
            if released && next.dragging {
                next.release(&field, here);
                // 空白单击 = 要鼠标底下那扇窗（REQ-108，用户原话"单击就相当于对整个应用界面
                // 一样的大小进行了截图"）。**认不出来就什么都不做**：`release` 已经把押着的旧框
                // 还回来了，不许顺手清掉它，也不许猜一个全屏框交出去。
                //
                // "手上还有没有框"这一条**只能在 release 之后问**：按下与抬起通常落在不同帧，
                // 按下那一帧已经把旧框押进 `previous`、把 `rect` 换成了一个零尺寸的点，
                // 在抬起那一帧的开头去看 `rect` 只会看到那个点——真机读数就是"单击认窗从不触发"。
                // `release` 之后 `rect` 恰好等于 gesture 之前的那份值，问它才是问对了东西。
                if next.clicked() && next.rect.is_none() && !any_selection(&root, frozen_id) {
                    if let Some(rect) = windows.candidate(origin, here, field.bounds) {
                        next.take_window(&field, rect);
                        tracing::info!("遮罩：单击认窗，整扇拿下 {rect:?}");
                        if let Ok(mut queue) = slot.lock() {
                            queue.push(MaskOut::Confirm { display, rect });
                        }
                        wake_root(&root);
                    }
                }
                changed = true;
            }
        } else {
            // 工具态：手势只动画笔，**不碰 next.rect**（互斥的另一半）。八件工具松手/单击
            // 建一个对象推进 ObjectList（CHG-016 T-002，取代旧"直接落像素"）。按下先命中测试：
            // 点落在已有对象上就抓起来拖（T-005 移动，一次拖动＝一条撤销），落在空白才画新的。
            // 只改对象列表 + 叫醒自己，不推意图、不叫主窗。
            let clamp_pt = |p: Pos2| {
                pos2(
                    p.x.clamp(ann_field.bounds.min.x, ann_field.bounds.max.x),
                    p.y.clamp(ann_field.bounds.min.y, ann_field.bounds.max.y),
                )
            };
            // 落在药丸 / 下拉弹出菜单 / 文字输入框这些浮层上的那一下不算画手势：
            // `is_pointer_over_egui` 认出指针正压在某个 Foreground 浮层上（画布是 Background
            // 根面板，不在此列）。少了这一条，从色板下拉里选个颜色会在菜单底下的画布上
            // 顺手点出一笔（REQ-003 换档换色不许污染画面）。
            if pressed && !on_bar && !ui.ctx().is_pointer_over_egui() {
                if let Some(rect) = input_rect {
                    // 输入开着：落在框外那一下 = 确认这条输入，这一下不起新手势；
                    // 落在框里交给 TextEdit（挪光标），不动工具态。
                    if !rect.contains(here) {
                        commit_text_input(&mut ts, &objects, ann_field);
                    }
                } else {
                    match ts.tool {
                        Some(Tool::Number) => {
                            stamp_number(&ts, &objects, ann_field, clamp_pt(here));
                        }
                        Some(Tool::Text) => {
                            // 上一条还没确认的先落，再在这一下的位置开新的
                            commit_text_input(&mut ts, &objects, ann_field);
                            ts.text_input = Some((clamp_pt(here), String::new()));
                        }
                        _ => {
                            // 先试命中已有对象：命中就抓起来拖（begin_move 压一条撤销快照）。
                            let hit = point_to_px(ann_field, clamp_pt(here))
                                .and_then(|pt| objects.lock().ok().and_then(|ol| ol.hit(pt, 8.0)));
                            if let Some(idx) = hit {
                                if let Ok(mut ol) = objects.lock() {
                                    ol.begin_move();
                                }
                                ts.moving = Some(idx);
                                ts.move_last = clamp_pt(here);
                            } else {
                                ts.anchor = clamp_pt(here);
                                ts.last = ts.anchor;
                                match ts.tool {
                                    Some(Tool::Pencil | Tool::Mosaic) => {
                                        ts.stroke_points = vec![ts.anchor]
                                    }
                                    Some(Tool::Eraser) => ts.erase_points = vec![ts.anchor],
                                    _ => {}
                                }
                            }
                            ts.dragging = true;
                        }
                    }
                }
            }
            // 拖拽中：抓着的既有对象→逐帧平移；铅笔/橡皮→累积点。
            if ts.dragging && ui.input(|i| i.pointer.button_down(egui::PointerButton::Primary)) {
                let to = clamp_pt(here);
                if let Some(idx) = ts.moving {
                    if let (Some(a), Some(b)) = (
                        point_to_px(ann_field, ts.move_last),
                        point_to_px(ann_field, to),
                    ) {
                        if let Ok(mut ol) = objects.lock() {
                            ol.move_object(idx, b.0 - a.0, b.1 - a.1);
                        }
                    }
                    ts.move_last = to;
                } else {
                    match ts.tool {
                        Some(Tool::Pencil | Tool::Mosaic) => ts.stroke_points.push(to),
                        Some(Tool::Eraser) => ts.erase_points.push(to),
                        _ => {}
                    }
                }
                ts.last = to;
            }
            if released && ts.dragging {
                ts.dragging = false;
                let a = ts.anchor;
                let b = clamp_pt(here);
                if ts.moving.take().is_some() {
                    // 拖动既有对象收口：位置已逐帧改好、begin_move 已压快照，这里只收尾。
                } else if let (Some(pa), Some(pb)) =
                    (point_to_px(ann_field, a), point_to_px(ann_field, b))
                {
                    if let Some(scale) = ann_field.scale() {
                        let width = THICKNESS[ts.thickness_idx] * scale;
                        let color = PALETTE[ts.color_idx];
                        match ts.tool {
                            Some(Tool::Rect) => {
                                let rect = ann_field.clamp(Rect::from_two_pos(a, b));
                                if rect.width() > 0.0 && rect.height() > 0.0 {
                                    if let Some(px_rect) = ann_field.to_pixels(rect) {
                                        if let Ok(mut ol) = objects.lock() {
                                            ol.push(AnnoObject {
                                                shape: Shape::Rect {
                                                    a: (px_rect.x, px_rect.y),
                                                    b: (
                                                        px_rect.x + px_rect.w,
                                                        px_rect.y + px_rect.h,
                                                    ),
                                                    width,
                                                    color,
                                                },
                                                erased: Vec::new(),
                                            });
                                        }
                                    }
                                }
                            }
                            Some(Tool::Line) => {
                                if let Ok(mut ol) = objects.lock() {
                                    ol.push(AnnoObject {
                                        shape: Shape::Line {
                                            a: pa,
                                            b: pb,
                                            width,
                                            color,
                                        },
                                        erased: Vec::new(),
                                    });
                                }
                            }
                            Some(Tool::Arrow) => {
                                if let Ok(mut ol) = objects.lock() {
                                    ol.push(AnnoObject {
                                        shape: Shape::Arrow {
                                            from: pa,
                                            to: pb,
                                            width,
                                            color,
                                        },
                                        erased: Vec::new(),
                                    });
                                }
                            }
                            Some(Tool::Pencil) => {
                                let pts: Vec<(f32, f32)> = ts
                                    .stroke_points
                                    .iter()
                                    .filter_map(|&p| point_to_px(ann_field, p))
                                    .collect();
                                ts.stroke_points.clear();
                                if !pts.is_empty() {
                                    if let Ok(mut ol) = objects.lock() {
                                        ol.push(AnnoObject {
                                            shape: Shape::Pencil {
                                                points: pts,
                                                width,
                                                color,
                                            },
                                            erased: Vec::new(),
                                        });
                                    }
                                }
                            }
                            Some(Tool::Eraser) => {
                                let pts: Vec<(f32, f32)> = ts
                                    .erase_points
                                    .iter()
                                    .filter_map(|&p| point_to_px(ann_field, p))
                                    .collect();
                                ts.erase_points.clear();
                                if !pts.is_empty() {
                                    if let Ok(mut ol) = objects.lock() {
                                        ol.erase_pen(pts, width * 3.0);
                                    }
                                }
                            }
                            Some(Tool::Mosaic) => {
                                // 涂抹式：整段轨迹转物理像素，建一个 Mosaic 对象（brush 轨迹＋块度）。
                                let pts: Vec<(f32, f32)> = ts
                                    .stroke_points
                                    .iter()
                                    .filter_map(|&p| point_to_px(ann_field, p))
                                    .collect();
                                ts.stroke_points.clear();
                                if !pts.is_empty() {
                                    let block = (MOSAIC_BLOCK_PT * scale).round().max(1.0) as u32;
                                    if let Ok(mut ol) = objects.lock() {
                                        ol.push(AnnoObject {
                                            shape: Shape::Mosaic {
                                                points: pts,
                                                width,
                                                block,
                                            },
                                            erased: Vec::new(),
                                        });
                                    }
                                }
                            }
                            _ => {}
                        }
                    }
                }
            }
            if ts.dragging {
                child.request_repaint();
            }
        }
        if changed {
            if next.rect.is_some() {
                mark_selection(&root, frozen_id);
            }
            ui.ctx()
                .data_mut(|d| d.insert_temp(sel_key(display), (frozen_id, next)));
            // 自己叫醒自己：指针事件只到这一扇子窗，不叫它重画，框就停在上一次画出来的样子
            // （"拖了没框/一闪一闪"的根在这儿，与绕不绕主窗无关）
            child.request_repaint();
            // 这里**不**叫主窗：拖动与悬停每帧都在变，主窗那一侧没有要跟着变的东西，
            // 每帧把它叫醒一次正是 D-31~D-35 那一族。要它醒的只有"推进槽里的那一刻"。
        }
        // 拖动中的矢量预览：与落层同一份几何、同一个色，松手落下去的就是看到的那个。
        // 抓着重画对象（ts.moving 有值）时不画新形状预览；铅笔/橡皮改成"累积后建对象"，
        // 不再实时落像素，所以这里补上它们的拖拽预览（铅笔=累积折线，橡皮=淡白擦痕）。
        if ts.dragging && ts.moving.is_none() {
            let c = PALETTE[ts.color_idx];
            let stroke = egui::Stroke::new(
                THICKNESS[ts.thickness_idx],
                egui::Color32::from_rgb(c[0], c[1], c[2]),
            );
            let painter = ui.painter();
            match ts.tool {
                Some(Tool::Rect) => {
                    let rect = ann_field.clamp(Rect::from_two_pos(ts.anchor, here));
                    painter.rect_stroke(rect, 0.0, stroke, egui::StrokeKind::Inside);
                }
                Some(Tool::Line) => {
                    painter.line_segment([ts.anchor, here], stroke);
                }
                Some(Tool::Arrow) => {
                    painter.line_segment([ts.anchor, here], stroke);
                    // 头的几何与落层读同一份（annotate::arrow_head），预览即所得
                    let [h1, h2] = crate::system::annotate::arrow_head(
                        (ts.anchor.x, ts.anchor.y),
                        (here.x, here.y),
                        THICKNESS[ts.thickness_idx],
                    );
                    painter.line_segment([here, pos2(h1.0, h1.1)], stroke);
                    painter.line_segment([here, pos2(h2.0, h2.1)], stroke);
                }
                Some(Tool::Pencil) => {
                    if ts.stroke_points.len() < 2 {
                        if let Some(&p) = ts.stroke_points.first() {
                            painter.circle_filled(
                                p,
                                THICKNESS[ts.thickness_idx] / 2.0,
                                egui::Color32::from_rgb(c[0], c[1], c[2]),
                            );
                        }
                    } else {
                        for pair in ts.stroke_points.windows(2) {
                            painter.line_segment([pair[0], pair[1]], stroke);
                        }
                    }
                }
                Some(Tool::Eraser) => {
                    let er = egui::Stroke::new(
                        THICKNESS[ts.thickness_idx] * 3.0,
                        egui::Color32::from_white_alpha(60),
                    );
                    if ts.erase_points.len() < 2 {
                        if let Some(&p) = ts.erase_points.first() {
                            painter.circle_filled(
                                p,
                                THICKNESS[ts.thickness_idx] * 1.5,
                                egui::Color32::from_white_alpha(60),
                            );
                        }
                    } else {
                        for pair in ts.erase_points.windows(2) {
                            painter.line_segment([pair[0], pair[1]], er);
                        }
                    }
                }
                Some(Tool::Mosaic) => {
                    // 涂抹式预览：淡白宽线沿笔迹示意"涂到哪打到哪"（不再是方框）。
                    let mo = egui::Stroke::new(
                        THICKNESS[ts.thickness_idx] * 3.0,
                        egui::Color32::from_white_alpha(40),
                    );
                    if ts.stroke_points.len() < 2 {
                        if let Some(&p) = ts.stroke_points.first() {
                            painter.circle_filled(
                                p,
                                THICKNESS[ts.thickness_idx] * 1.5,
                                egui::Color32::from_white_alpha(40),
                            );
                        }
                    } else {
                        for pair in ts.stroke_points.windows(2) {
                            painter.line_segment([pair[0], pair[1]], mo);
                        }
                    }
                }
                _ => {}
            }
        }

        // 笔刷光标反馈（用户 2026-11-18 要）：选中铅笔/橡皮/马赛克时，鼠标尖画一个跟粗细
        // 挂钩的圈，让人看得出要涂多宽。落在药丸/下拉浮层上、或指针不在画布内时不画。
        if matches!(ts.tool, Some(Tool::Pencil | Tool::Eraser | Tool::Mosaic))
            && !on_bar
            && !ui.ctx().is_pointer_over_egui()
            && ann_field.bounds.contains(here)
        {
            let r = match ts.tool {
                Some(Tool::Pencil) => THICKNESS[ts.thickness_idx] / 2.0,
                _ => THICKNESS[ts.thickness_idx] * 1.5,
            };
            ui.painter().circle_stroke(
                here,
                r.max(2.0),
                egui::Stroke::new(1.0, egui::Color32::from_white_alpha(210)),
            );
        }

        // 动作条：**圆角药丸，浮在选区下方，放不下翻上方**；不许铺满整幅图宽（REQ-103）。
        // 宽度按内容定（[`bar_width`]），版式照贴图那条已批准药丸（`pill.rs`）：两端让出
        // `SIDE_PAD`、颗间 `BTN_GAP`、整行在 `BAR_H` 里垂直居中——不再齐平顶格、右边空一大截。
        if let Some(bar) = bar_of(
            &next,
            &field,
            label_width(ui, &next, &field),
            ts.tool.is_some(),
        ) {
            // 用 Area 浮在遮罩之上：药丸的位置跟着选区算，不占窗口布局（占位会把图片挤走）。
            // Area 原点左移一格 SIDE_PAD，下面那一行就从药丸左缘往里 SIDE_PAD 处起摆。
            egui::Area::new(Id::NULL.with("ice-selection.bar"))
                .fixed_pos(bar.min + egui::vec2(bar::SIDE_PAD, 0.0))
                .order(egui::Order::Foreground)
                .show(ui.ctx(), |ui| {
                    // 底色就画在 `bar` 那一块。这里曾经写成 `Rect::from_min_size(Pos2::ZERO, bar.size())`，
                    // 而 Area 的 painter 用的是**全局**坐标——于是底色跑到屏幕左上角去了，
                    // 药丸本体只剩三颗线稿浮在画面上（真机逐点量到：(0,0) 处有一块 36 的暗档，
                    // 而选区下方那一块仍是 132 的画面对比）。`bar` 是全局矩形，与 Area 原点无关。
                    ui.painter().rect_filled(
                        bar,
                        bar.height() / 2.0,
                        egui::Color32::from_rgba_premultiplied(28, 28, 30, 235),
                    );
                    ui.spacing_mut().item_spacing.x = bar::BTN_GAP;
                    ui.allocate_ui_with_layout(
                        egui::vec2(bar.width() - 2.0 * bar::SIDE_PAD, bar.height()),
                        egui::Layout::left_to_right(egui::Align::Center),
                        |ui| {
                            // 工具颗在前、确认三颗在后：**画框 / 箭头 / 直线 / 铅笔 /
                            // 序号 / 文字 / 马赛克 / 橡皮 / 复制 / 保存 / 对勾**（对勾最后，
                            // 用户 2026-11-17）。对勾就是"完成并钉住"（他原话"点对勾和固定到
                            // 屏幕上是一样的"）；栏里不放「取消」——esc 与右键承担。图标全是
                            // **线稿**，不用字符（本仓出过字符显示成方块的缺陷）。
                            for icon in BAR_ICONS {
                                let active = tool_of(icon).is_some() && ts.tool == tool_of(icon);
                                if icon_button(ui, icon, active).clicked() {
                                    if let Some(t) = tool_of(icon) {
                                        // 工具开关只改这一扇窗自己的运行态：不推意图、不叫主窗
                                        // （推意图的出口只有确认三颗，形状核查盯死三个）。
                                        // 开着的文字输入先落层，不许跟着切换丢字。
                                        commit_text_input(&mut ts, &objects, ann_field);
                                        ts.tool = if ts.tool == Some(t) { None } else { Some(t) };
                                        ts.dragging = false;
                                        continue;
                                    }
                                    // 确认必须带上**这扇窗自己的框**：主窗不用再猜是哪块屏
                                    let out = match (icon, next.rect) {
                                        (BarIcon::Check, Some(rect)) => {
                                            MaskOut::Confirm { display, rect }
                                        }
                                        (BarIcon::Copy, Some(rect)) => {
                                            MaskOut::ConfirmCopy { display, rect }
                                        }
                                        (BarIcon::Save, Some(rect)) => {
                                            MaskOut::ConfirmSave { display, rect }
                                        }
                                        // 还没框就先不响应：不猜一个全屏框给用户，那比不响应更糟
                                        _ => continue,
                                    };
                                    // 成品里需要包含正在输入的那条字
                                    commit_text_input(&mut ts, &objects, ann_field);
                                    if let Ok(mut queue) = slot.lock() {
                                        queue.push(out);
                                    }
                                    wake_root(&root);
                                }
                            }
                            // 选中工具才长出粗细 / 色板两颗下拉（REQ-003）：换档换色，落层与
                            // 预览同源读 `ts.thickness_idx` / `ts.color_idx`。没选工具时不占位，
                            // 药丸宽度由 [`bar_width`] 的 `tool_active` 同步收回去。两颗定宽 OPT_W，
                            // 换档时药丸不跳宽。按钮套一层暗色 visuals 与药丸同底；弹出菜单跟随
                            // 应用主题（与贴图右键菜单一致，不另造一套）。
                            if ts.tool.is_some() {
                                let mut dark = ui.visuals().clone();
                                dark.widgets.inactive.bg_fill =
                                    egui::Color32::from_rgba_premultiplied(48, 48, 52, 235);
                                dark.widgets.inactive.weak_bg_fill =
                                    egui::Color32::from_rgba_premultiplied(48, 48, 52, 235);
                                dark.widgets.inactive.fg_stroke =
                                    egui::Stroke::new(1.0, egui::Color32::from_gray(235));
                                dark.widgets.hovered.bg_fill =
                                    egui::Color32::from_rgba_premultiplied(66, 66, 72, 245);
                                dark.widgets.hovered.weak_bg_fill =
                                    egui::Color32::from_rgba_premultiplied(66, 66, 72, 245);
                                // 暗色只套在这两颗下拉上：**必须走 `visuals_mut`**（clone-on-write，
                                // 只改这一个 Ui 及其子）。`ui.set_visuals` 看着像 Ui 的方法，实则
                                // Ui Deref 到 Context，调的是全局 `Context::set_visuals`——scope 拦不住，
                                // 会把整扇上下文的文字墨色改成浅灰，亮色主界面随后整排按钮字发白
                                // （D-46，真机像素量到 238 对 247 的底）。
                                ui.scope(|ui| {
                                    *ui.visuals_mut() = dark;
                                    egui::ComboBox::from_id_salt("ice-selection.thickness")
                                        .selected_text(THICKNESS_NAMES[ts.thickness_idx])
                                        .width(OPT_W)
                                        .show_ui(ui, |ui| {
                                            for (i, name) in THICKNESS_NAMES.iter().enumerate() {
                                                ui.selectable_value(
                                                    &mut ts.thickness_idx,
                                                    i,
                                                    *name,
                                                );
                                            }
                                        });
                                    egui::ComboBox::from_id_salt("ice-selection.color")
                                        .selected_text(PALETTE_NAMES[ts.color_idx])
                                        .width(OPT_W)
                                        .show_ui(ui, |ui| {
                                            for (i, name) in PALETTE_NAMES.iter().enumerate() {
                                                ui.selectable_value(&mut ts.color_idx, i, *name);
                                            }
                                        });
                                });
                            }
                            // 尺寸读数与裁剪同源（同一个 `Field`），这里只报一次。
                            // 字体就是 Body（[`label_width`] 量的也是它），画出来的宽与量出来的同源。
                            if let Some(text) = next.size_label(&field) {
                                ui.label(text);
                            }
                        },
                    );
                });
        }
        // 工具态写回：与选区那格同形（带轮号），变了才写、才叫醒自己。
        // **必须放在药丸循环之后**：工具开关在那循环里才拨动，写回若抢在它前面，
        // 拨动的结果存不进去、下一帧读回还是旧态——真机报的"工具按钮点了没反应"
        // 正是这条顺序错（D-44）。落层那一笔也在这里被看见：revision 变了下一帧重传纹理。
        if ts != tool_before {
            ui.ctx()
                .data_mut(|d| d.insert_temp(tool_key(display), (frozen_id, ts.clone())));
            child.request_repaint();
        }
    });
}

/// 栏里那几颗，顺序就是显示顺序。**对勾排在最后**（用户 2026-11-17 原话"把对号往最后放"）；
/// 栏里不放「取消」（esc 与右键承担）。加一颗要改这张表，宽度测试与形状核查会先响。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum BarIcon {
    /// 画框（标注工具；再点一次或 esc 退出）
    Rect,
    /// 带箭头的线
    Arrow,
    /// 直线
    Line,
    /// 铅笔
    Pencil,
    /// 序号标
    Number,
    /// 文字
    Text,
    /// 马赛克
    Mosaic,
    /// 橡皮
    Eraser,
    /// 完成 = 钉到屏幕上
    Check,
    Copy,
    Save,
}

pub const BAR_ICONS: [BarIcon; 11] = [
    BarIcon::Rect,
    BarIcon::Arrow,
    BarIcon::Line,
    BarIcon::Pencil,
    BarIcon::Number,
    BarIcon::Text,
    BarIcon::Mosaic,
    BarIcon::Eraser,
    BarIcon::Copy,
    BarIcon::Save,
    BarIcon::Check,
];

/// 一颗线稿图标按钮：自己画（两段线/两块圆角框/一个盘子加一支箭头），返回是否被点。
///
/// 为什么不用 `ui.button("✔")`：字符依赖字体，本仓因为字符图标出过显示成方块的缺陷（D-5 那一族）。
fn icon_button(ui: &mut egui::Ui, icon: BarIcon, active: bool) -> egui::Response {
    let size = egui::vec2(ICON_W, ICON_W);
    let (id, rect) = ui.allocate_space(size);
    let response = ui.interact(rect, id, egui::Sense::click());
    let rect = rect.shrink(4.0);
    let ink = if response.hovered() || active {
        egui::Color32::WHITE
    } else {
        egui::Color32::from_rgb(0xC8, 0xCA, 0xCF)
    };
    let stroke = egui::Stroke::new(1.6, ink);
    let painter = ui.painter();
    if active {
        // 激活中的工具：一圈淡底把它与"还没点"的颗分开。必须用 from_white_alpha——
        // 之前写成 from_rgba_premultiplied(255,255,255,30)：预乘 alpha 下色值(255)远大于
        // alpha(30)不合法，合成时颜色饱和成一整块实心白，把白色图标盖得看不见（用户 2026-11-18
        // 真机拍照报"选中工具纯白什么都看不到"）。from_white_alpha 才是正确的淡白。
        painter.rect_filled(rect.expand(3.0), 5.0, egui::Color32::from_white_alpha(30));
    }
    match icon {
        BarIcon::Rect => {
            // 方框线稿：与它落下去的形状同一个读法
            painter.rect_stroke(rect.shrink(1.0), 2.0, stroke, egui::StrokeKind::Inside);
        }
        BarIcon::Arrow => {
            // 斜方向箭头：左下到右上一支杆，尖端两段短线成头
            let a = egui::pos2(rect.min.x, rect.max.y);
            let b = egui::pos2(rect.max.x, rect.min.y);
            painter.line_segment([a, b], stroke);
            painter.line_segment(
                [b, egui::pos2(rect.max.x - rect.width() * 0.42, rect.min.y)],
                stroke,
            );
            painter.line_segment(
                [b, egui::pos2(rect.max.x, rect.min.y + rect.height() * 0.42)],
                stroke,
            );
        }
        BarIcon::Line => {
            // 斜方向直线：与箭头同一根杆，没有头
            painter.line_segment(
                [
                    egui::pos2(rect.min.x, rect.max.y),
                    egui::pos2(rect.max.x, rect.min.y),
                ],
                stroke,
            );
        }
        BarIcon::Pencil => {
            // 铅笔线稿：拉满整幅对角（之前偏短）、笔身够粗，尖头处加一条曲线示意"能随便画"。
            let p = |fx: f32, fy: f32| {
                egui::pos2(
                    rect.min.x + rect.width() * fx,
                    rect.min.y + rect.height() * fy,
                )
            };
            let tip = p(0.10, 0.90);
            let (cone_l, cone_r) = (p(0.326, 0.830), p(0.170, 0.674));
            let (back_l, back_r) = (p(0.844, 0.312), p(0.688, 0.156));
            // 笔身两条长边 + 尾封口
            painter.line_segment([cone_l, back_l], stroke);
            painter.line_segment([cone_r, back_r], stroke);
            painter.line_segment([back_l, back_r], stroke);
            // 削尖三角头两条斜边收到笔尖
            painter.line_segment([cone_l, tip], stroke);
            painter.line_segment([cone_r, tip], stroke);
            // 锥底与笔身分隔线
            painter.line_segment([cone_l, cone_r], stroke);
            // 铅芯：靠近尖端一小段填实
            painter.add(egui::Shape::convex_polygon(
                vec![tip, p(0.168, 0.876), p(0.121, 0.829)],
                ink,
                egui::Stroke::NONE,
            ));
        }
        BarIcon::Number => {
            // 序号标线稿：一个圆 + 里面两段线画的「1」（竖杆 + 顶部小旗），
            // 与它落下去的实心圆 + 白数字同一个读法。不用字符字形（依赖字体）。
            painter.circle_stroke(rect.center(), rect.width() * 0.46, stroke);
            let p = |fx: f32, fy: f32| {
                egui::pos2(
                    rect.min.x + rect.width() * fx,
                    rect.min.y + rect.height() * fy,
                )
            };
            painter.line_segment([p(0.50, 0.30), p(0.50, 0.72)], stroke);
            painter.line_segment([p(0.38, 0.40), p(0.50, 0.30)], stroke);
        }
        BarIcon::Text => {
            // 文字线稿：一个「T」——顶上一横 + 中间一竖
            let p = |fx: f32, fy: f32| {
                egui::pos2(
                    rect.min.x + rect.width() * fx,
                    rect.min.y + rect.height() * fy,
                )
            };
            painter.line_segment([p(0.22, 0.26), p(0.78, 0.26)], stroke);
            painter.line_segment([p(0.50, 0.26), p(0.50, 0.78)], stroke);
        }
        BarIcon::Mosaic => {
            // 马赛克线稿：2×2 棋盘格，对角两格实心、两格描边——
            // 与它落下去的"一块一色"像素化同一个读法。
            let (hw, hh) = (rect.width() / 2.0, rect.height() / 2.0);
            let cell = |cx: f32, cy: f32| {
                egui::Rect::from_min_size(
                    egui::pos2(rect.min.x + cx * hw, rect.min.y + cy * hh),
                    egui::vec2(hw, hh),
                )
            };
            painter.rect_filled(cell(0.0, 0.0), 0.0, ink);
            painter.rect_filled(cell(1.0, 1.0), 0.0, ink);
            painter.rect_stroke(cell(1.0, 0.0), 0.0, stroke, egui::StrokeKind::Inside);
            painter.rect_stroke(cell(0.0, 1.0), 0.0, stroke, egui::StrokeKind::Inside);
        }
        BarIcon::Eraser => {
            // 橡皮线稿：整体放大、白色笔身加长（实心橡皮头缩短、分隔线往尖端挪），底下保留一条横线。
            let p = |fx: f32, fy: f32| {
                egui::pos2(
                    rect.min.x + rect.width() * fx,
                    rect.min.y + rect.height() * fy,
                )
            };
            let (a, b, c, d) = (p(0.12, 0.56), p(0.56, 0.84), p(0.90, 0.44), p(0.46, 0.14));
            let (q1, q2) = (p(0.583, 0.224), p(0.243, 0.638));
            // 橡皮头（a-d 那一端）填实 + 描边——只占靠尖端一小截，白色笔身更长
            painter.add(egui::Shape::convex_polygon(vec![a, d, q1, q2], ink, stroke));
            // 整块橡皮轮廓
            painter.add(egui::Shape::convex_polygon(
                vec![a, b, c, d],
                egui::Color32::TRANSPARENT,
                stroke,
            ));
            // 底下那条横线（跟放大后的橡皮同宽）
            painter.line_segment([p(0.10, 0.92), p(0.80, 0.92)], stroke);
        }
        BarIcon::Check => {
            // 两段线，短的那段从左边起头——和 某截图工具 那颗对勾同一个读法
            let start = egui::pos2(rect.min.x, rect.center().y);
            let knee = egui::pos2(rect.min.x + rect.width() * 0.35, rect.max.y);
            let end = egui::pos2(rect.max.x, rect.min.y);
            // 两段线，短的那段从左边起头——和 某截图工具 那颗对勾同一个读法。
            // 不用 `Shape::convex_polyline`：0.37 没有它，且折线本来就两处端点。
            painter.line_segment([start, knee], stroke);
            painter.line_segment([knee, end], stroke);
        }
        BarIcon::Copy => {
            // 两框：后框偏右上、前框偏左下压住它一多半（>50%）；前框先实心填药丸底色再描边，
            // 后框被压部分的线条不透过前框（用户 2026-11-17 v0.2 裁定）。偏移取 0.20、框取 0.72：
            // 重叠率 (1-0.20/0.72)^2 ≈ 0.52 > 0.5，恰好"压住一多半"。
            let bw = rect.width() * 0.72;
            let bh = rect.height() * 0.72;
            let back = egui::Rect::from_min_size(
                egui::pos2(rect.min.x + rect.width() * 0.20, rect.min.y),
                egui::vec2(bw, bh),
            );
            let front = egui::Rect::from_min_size(
                egui::pos2(rect.min.x, rect.min.y + rect.height() * 0.20),
                egui::vec2(bw, bh),
            );
            painter.rect_stroke(back, 3.0, stroke, egui::StrokeKind::Inside);
            painter.rect_filled(
                front,
                3.0,
                egui::Color32::from_rgba_premultiplied(28, 28, 30, 235),
            );
            painter.rect_stroke(front, 3.0, stroke, egui::StrokeKind::Inside);
        }
        BarIcon::Save => {
            // 通用"下载/存到别处"：一支向下箭头（竖杆＋两段斜线成头）＋ 下方一条独立横线，
            // 不再是 U 形盘子（用户 2026-11-17 v0.2 裁定）。横线与箭头不相连。
            let cx = rect.center().x;
            let top = egui::pos2(cx, rect.min.y + rect.height() * 0.06);
            let tip = egui::pos2(cx, rect.min.y + rect.height() * 0.60);
            painter.line_segment([top, tip], stroke);
            painter.line_segment(
                [
                    egui::pos2(cx - rect.width() * 0.22, rect.min.y + rect.height() * 0.38),
                    tip,
                ],
                stroke,
            );
            painter.line_segment(
                [
                    egui::pos2(cx + rect.width() * 0.22, rect.min.y + rect.height() * 0.38),
                    tip,
                ],
                stroke,
            );
            let base_y = rect.min.y + rect.height() * 0.88;
            painter.line_segment(
                [
                    egui::pos2(rect.min.x + rect.width() * 0.12, base_y),
                    egui::pos2(rect.max.x - rect.width() * 0.12, base_y),
                ],
                stroke,
            );
        }
    }
    let tip = match icon {
        BarIcon::Rect => "在截图上画框（再点一次或按 esc 退出）",
        BarIcon::Arrow => "画带箭头的线（再点一次或按 esc 退出）",
        BarIcon::Line => "画直线（再点一次或按 esc 退出）",
        BarIcon::Pencil => "自由绘制（再点一次或按 esc 退出）",
        BarIcon::Number => "点一下落一个序号，自动加一（再点一次或按 esc 退出）",
        BarIcon::Text => "点一下输入文字，回车落下去（再点一次或按 esc 退出）",
        BarIcon::Mosaic => "拖一块区域打码（再点一次或按 esc 退出）",
        BarIcon::Eraser => "擦掉画上去的痕迹（再点一次或按 esc 退出）",
        BarIcon::Check => "钉到屏幕上（与按回车相同）",
        BarIcon::Copy => "复制这张图到剪贴板",
        BarIcon::Save => "存成文件",
    };
    response.on_hover_text(tip)
}

/// 动作条的行高（药丸）。**宽度不再写死**——按内容定（[`bar_width`]：全部线稿颗、一句尺寸
/// 读数、两端留白），它 SHALL NOT 跟着选区宽度变（用户那句"标注按钮怎么有背景"要的就是浮着
/// 的小条，不是一条 bar）。先前写死 300 而内容只占约一半，右边空一大截黑底，正是"排列好丑"那一条。
pub const BAR_H: f32 = 34.0;

/// 一颗线稿按钮的槽宽（点）。[`icon_button`] 占位与 [`bar_width`] 算宽都读这一处，
/// 改大改小两边一起动，不会分岔出"按钮挤出去 / 药丸留空"。
pub const ICON_W: f32 = 26.0;

/// 粗细 / 色板两个下拉的定宽（点）。选中工具才长出来（REQ-003），[`bar_width`] 按此加宽。
/// 定宽而非按内容：换档换色时药丸宽度不跳，两颗下拉始终对齐同一槽。
pub const OPT_W: f32 = 46.0;

#[cfg(test)]
mod tests {
    use super::*;
    use egui::vec2;

    /// 主屏：1920×1080 的点、2 倍像素（**故意用非 1.0 的比例**——本机在线两屏都是 1.0，
    /// 无头里再写 1.0 就等于"拿 1.0 的读数冒充 2.0"，那是 CHG-009 事实 22 犯过的错）。
    fn main_screen() -> Field {
        Field::new(
            Rect::from_min_max(pos2(0.0, 0.0), pos2(1920.0, 1080.0)),
            vec2(3840.0, 2160.0),
        )
    }

    /// 第二块屏：挂在左边、还往上多出 120（本机实测 `(-1920,-120) 1920x1200`）。
    /// 遮罩窗与屏对齐后窗口内是 0 起点，但**全局矩形**要能被同一套函数处理，所以这里
    /// 直接拿带负坐标的 bounds 喂进去——负 x 不许有任何特殊分支。
    fn left_screen_global() -> Field {
        Field::new(
            Rect::from_min_max(pos2(-1920.0, -120.0), pos2(0.0, 1080.0)),
            vec2(1920.0, 1200.0),
        )
    }

    fn drag_all_the_way(field: &Field, from: Pos2, to: Pos2) -> Selection {
        let mut sel = Selection::fresh();
        sel.press(field, from, HANDLE_RADIUS);
        sel.drag(field, to);
        sel.release(field, to);
        sel
    }

    /// 空白拖出新框，尺寸读数与矩形同源（同一个换算、同一份夹取）。
    #[test]
    fn a_blank_drag_makes_a_selection_whose_label_matches_its_pixels() {
        let field = main_screen();
        let sel = drag_all_the_way(&field, pos2(100.0, 100.0), pos2(300.0, 250.0));
        assert_eq!(
            sel.stage,
            Stage::Ready,
            "松手后必须停在原地等确认，不许自己落地"
        );
        let rect = sel.rect.expect("拖出来了就该有框");
        assert_eq!(
            (rect.min.x, rect.min.y, rect.max.x, rect.max.y),
            (100.0, 100.0, 300.0, 250.0)
        );
        // 200×150 点，2 倍屏 → 400×300 像素。读数报的是**像素**，不是点
        assert_eq!(sel.size_label(&field).as_deref(), Some("400 × 300"));
    }

    /// 越界一律夹在屏内：拖出去、拖过右缘、拖到屏外抬起，都只能是屏内那块。
    #[test]
    fn a_drag_that_leaves_the_screen_is_clamped_not_lost() {
        let field = main_screen();
        let sel = drag_all_the_way(&field, pos2(1800.0, 1000.0), pos2(5000.0, 5000.0));
        let rect = sel.rect.expect("抬起在屏外也算一次正常松手");
        assert_eq!(
            (rect.max.x, rect.max.y),
            (1920.0, 1080.0),
            "右下一角死死停在屏角"
        );
        assert_eq!(sel.size_label(&field).as_deref(), Some("240 × 160"));
    }

    /// 副屏的**负坐标**没有特殊分支：同一套夹取与换算在那块屏上照样成立。
    #[test]
    fn the_left_screen_with_negative_coordinates_uses_the_same_rules() {
        let field = left_screen_global();
        let sel = drag_all_the_way(&field, pos2(-1900.0, -100.0), pos2(-1700.0, 100.0));
        let rect = sel.rect.expect("负坐标那块屏照样能框");
        assert_eq!((rect.min.x, rect.min.y), (-1900.0, -100.0));
        // 1.0 倍屏：200×200 点 = 200×200 像素，且原点从**那块屏自己的左上角**量起
        assert_eq!(sel.size_label(&field).as_deref(), Some("200 × 200"));
        // 拖出这块屏的边界要夹住（它的顶边在全局 y=-120）
        let out = drag_all_the_way(&field, pos2(-1900.0, -110.0), pos2(-999999.0, -999999.0));
        assert_eq!(
            out.rect.map(|r| (r.min.x, r.min.y)),
            Some((-1920.0, -120.0))
        );
    }

    /// **八个把手各只动该动的那两条边**：右下角跟着走，左上角钉住不动。
    #[test]
    fn a_handle_moves_only_the_edges_it_should() {
        let field = main_screen();
        let base = Rect::from_min_max(pos2(100.0, 100.0), pos2(300.0, 200.0));
        let mut sel = Selection {
            stage: Stage::Ready,
            rect: Some(base),
            ..Selection::default()
        };
        assert_eq!(
            sel.press(&field, pos2(300.0, 200.0), HANDLE_RADIUS),
            Grab::Handle(4)
        );
        sel.drag(&field, pos2(360.0, 260.0));
        sel.release(&field, pos2(360.0, 260.0));
        assert_eq!(
            sel.rect,
            Some(Rect::from_min_max(pos2(100.0, 100.0), pos2(360.0, 260.0)))
        );

        // 只拖上边：x 不许动
        let mut sel = Selection {
            stage: Stage::Ready,
            rect: Some(base),
            ..Selection::default()
        };
        assert_eq!(
            sel.press(&field, pos2(200.0, 100.0), HANDLE_RADIUS),
            Grab::Handle(1)
        );
        sel.drag(&field, pos2(233.0, 60.0));
        assert_eq!(
            sel.rect,
            Some(Rect::from_min_max(pos2(100.0, 60.0), pos2(300.0, 200.0)))
        );
    }

    /// 框内拖动 = 平移：**尺寸一格不变**，也不许被推出屏外。
    #[test]
    fn dragging_inside_moves_the_box_and_keeps_its_size() {
        let field = main_screen();
        let base = Rect::from_min_max(pos2(1800.0, 1000.0), pos2(1900.0, 1070.0));
        let mut sel = Selection {
            stage: Stage::Ready,
            rect: Some(base),
            ..Selection::default()
        };
        assert_eq!(
            sel.press(&field, pos2(1850.0, 1030.0), HANDLE_RADIUS),
            Grab::Inside
        );
        sel.drag(&field, pos2(2400.0, 2400.0));
        let rect = sel.rect.expect("还在拖");
        assert_eq!(rect.size(), base.size(), "移动不许改尺寸");
        assert_eq!(
            rect.max,
            pos2(1920.0, 1080.0),
            "往屏外推要停在屏角，而不是把框推出可见区"
        );
    }

    /// D-43（用户 2026-11-16 真机报）：框内按住拖动，**按下那一点要一直贴在鼠标下**。
    /// 遮罩每帧调一次 drag，旧写法把锚点记成框的新左上角，第二帧起左上角就粘了鼠标；
    /// 只拖一步的旧用例抓不到，这条专门多帧连拖。夹到屏缘再拖回来，抓取点也不许漂。
    #[test]
    fn moving_inside_keeps_the_pressed_point_under_the_cursor_across_frames() {
        let field = main_screen();
        let base = Rect::from_min_max(pos2(100.0, 100.0), pos2(300.0, 200.0));
        let mut sel = Selection {
            stage: Stage::Ready,
            rect: Some(base),
            ..Selection::default()
        };
        let press_at = pos2(250.0, 180.0);
        let offset = press_at - base.min; // (150, 80)：抓取点相对框左上角的位置
        assert_eq!(sel.press(&field, press_at, HANDLE_RADIUS), Grab::Inside);

        // 连拖三帧（真机就是每帧一步）：每一步框左上角都该等于 鼠标 − 固定偏移
        for to in [pos2(260.0, 190.0), pos2(350.0, 260.0), pos2(400.0, 300.0)] {
            sel.drag(&field, to);
            let rect = sel.rect.expect("还在拖");
            assert_eq!(rect.size(), base.size(), "移动不许改尺寸");
            assert_eq!(rect.min, to - offset, "按下那一点必须一直贴在鼠标下");
        }

        // 推到屏缘夹住，再拖回来：绝对位移算法要把抓取点原样还回来，不许漂
        sel.drag(&field, pos2(1900.0, 1060.0));
        assert_eq!(sel.rect.unwrap().max, pos2(1920.0, 1080.0), "夹在屏角");
        sel.drag(&field, pos2(400.0, 300.0));
        assert_eq!(
            sel.rect.unwrap().min,
            pos2(400.0, 300.0) - offset,
            "从屏缘拖回来，抓取点仍贴在鼠标下"
        );

        assert_eq!(sel.release(&field, pos2(400.0, 300.0)), Stage::Ready);
        assert_eq!(sel.rect.unwrap().min, pos2(400.0, 300.0) - offset);
        assert_eq!(sel.rect.unwrap().size(), base.size());
    }

    /// 一次按下**只落一处判定**：把手优先于框内。反过来的话靠内那一圈把手永远点不到，
    /// 而这个缺陷在无头里就能测出来，不该留到手感走查。
    #[test]
    fn one_press_lands_on_exactly_one_of_the_three_cases() {
        let rect = Rect::from_min_max(pos2(100.0, 100.0), pos2(300.0, 200.0));
        // 右下角上：是把手 4，不是 Inside
        assert_eq!(
            grab(Some(rect), pos2(300.0, 200.0), HANDLE_RADIUS),
            Grab::Handle(4)
        );
        // 角内一格：仍然是把手（优先级）
        assert_eq!(
            grab(Some(rect), pos2(295.0, 195.0), HANDLE_RADIUS),
            Grab::Handle(4)
        );
        // 框的正中：Inside
        assert_eq!(
            grab(Some(rect), pos2(200.0, 150.0), HANDLE_RADIUS),
            Grab::Inside
        );
        // 框外：Blank
        assert_eq!(
            grab(Some(rect), pos2(400.0, 150.0), HANDLE_RADIUS),
            Grab::Blank
        );
        // 还没有框时任何东西都是 Blank
        assert_eq!(grab(None, pos2(100.0, 100.0), HANDLE_RADIUS), Grab::Blank);
        // 八个把手编号覆盖完整，且两两不同
        let seen: Vec<Grab> = (0..HANDLE_COUNT)
            .map(|i| grab(Some(rect), handle_center(rect, i), HANDLE_RADIUS))
            .collect();
        assert!(seen.iter().all(|g| matches!(g, Grab::Handle(_))));
        let ids: Vec<u8> = seen
            .iter()
            .filter_map(|g| match g {
                Grab::Handle(i) => Some(*i),
                _ => None,
            })
            .collect();
        assert_eq!(ids.len(), HANDLE_COUNT as usize);
        assert!(
            ids.iter().collect::<std::collections::HashSet<_>>().len() == HANDLE_COUNT as usize
        );
    }

    /// **单击认窗这一判只能在 release 之后问**——按下与抬起分属两帧，帧开头那份 `rect`
    /// 早就被按下换成了零尺寸的点。
    ///
    /// 真机读数：悬停高亮是对的（同一处 `candidate`），单击却永远不钉整扇窗。原因就是旧判定
    /// 在抬起那一帧的开头读 `rect.is_some()`，读到的是那个点 ⇒ 一律当成"手上还有框"。
    #[test]
    fn a_single_click_across_two_frames_still_asks_for_the_window_hit() {
        let field = main_screen();
        let at = pos2(1500.0, 300.0);

        let mut sel = Selection::fresh();
        sel.press(&field, at, HANDLE_RADIUS);
        assert!(
            sel.rect.is_some_and(|rect| rect.width() <= 0.0),
            "按下那一帧手上只有一个零尺寸的点"
        );
        sel.release(&field, at);
        assert!(sel.clicked(), "没拖过就是单击");
        assert!(
            sel.rect.is_none(),
            "release 把这次手势之前的那份值还回来了——本来就没有框"
        );

        // 代码形状：判定必须在 release 之后，且不许再回到"帧开头猜"
        let body = crate::system::implementation_lines(include_str!("selection.rs"));
        assert!(
            body.contains("next.clicked() && next.rect.is_none()"),
            "认窗那一下只能在 release 之后判定"
        );
        assert!(
            !body.contains("let had_box"),
            "不许在帧开头读 rect 来当'手上还有框'"
        );
    }

    /// **单击不许把上一个框弄没**：空白单击是"要鼠标底下那扇窗"（REQ-108），
    /// 不是"取消选中"。真取消走 esc 或那颗按钮（REQ-105）。
    #[test]
    fn a_click_on_blank_keeps_the_previous_box_for_the_window_hit() {
        let field = main_screen();
        let base = Rect::from_min_max(pos2(100.0, 100.0), pos2(300.0, 200.0));
        let mut sel = Selection {
            stage: Stage::Ready,
            rect: Some(base),
            ..Selection::default()
        };
        sel.press(&field, pos2(800.0, 700.0), HANDLE_RADIUS);
        sel.release(&field, pos2(801.0, 701.0));
        assert!(sel.clicked(), "没挪动就是单击，调用方要拿它去认窗");
        assert_eq!(sel.rect, Some(base), "旧框要原样回来");
    }

    /// 抖一下（超过阈值）就算拖动，不再当单击——两条的边界只有一条常量在管。
    #[test]
    fn crossing_the_slop_turns_a_click_into_a_drag() {
        let field = main_screen();
        let base = Rect::from_min_max(pos2(100.0, 100.0), pos2(300.0, 200.0));
        let mut sel = Selection {
            stage: Stage::Ready,
            rect: Some(base),
            ..Selection::default()
        };
        sel.press(&field, pos2(800.0, 700.0), HANDLE_RADIUS);
        sel.drag(&field, pos2(900.0, 800.0));
        sel.release(&field, pos2(900.0, 800.0));
        assert!(!sel.clicked());
        assert_eq!(
            sel.rect,
            Some(Rect::from_min_max(pos2(800.0, 700.0), pos2(900.0, 800.0)))
        );
    }

    /// 认出的那扇窗 → 直接 `Confirmed`，且夹在那块屏内（半屏在屏幕外的窗也截得对）。
    #[test]
    fn a_recognised_window_is_confirmed_and_clamped() {
        let field = main_screen();
        let mut sel = Selection::fresh();
        sel.take_window(
            &field,
            Rect::from_min_max(pos2(1800.0, 1000.0), pos2(4000.0, 4000.0)),
        );
        assert_eq!(sel.stage, Stage::Confirmed);
        assert_eq!(
            sel.rect,
            Some(Rect::from_min_max(
                pos2(1800.0, 1000.0),
                pos2(1920.0, 1080.0)
            ))
        );
    }

    /// 认窗高亮跟着光标走：光标在哪块屏就只亮哪块（用户那句「两块屏上的窗都亮了」的修法）。
    /// 纯函数，喂「全局光标 + 各屏全局矩形」，副屏负偏移那条照样成立。
    #[test]
    fn the_lit_display_is_the_one_containing_the_cursor() {
        let screens = [
            (1_u8, main_screen().bounds),
            (2_u8, left_screen_global().bounds),
        ];
        // 光标在主屏里：亮主屏
        assert_eq!(lit_display(Some(pos2(900.0, 500.0)), &screens), Some(1));
        // 光标在副屏里（负坐标）：亮副屏，不许因为 x 是负的就认不出
        assert_eq!(lit_display(Some(pos2(-1000.0, 300.0)), &screens), Some(2));
        // 问不到光标：None，调用方退回各窗自己的判法（不许因为问不到就永不认窗）
        assert_eq!(lit_display(None, &screens), None);
    }

    /// 光标落在两块屏之间的缝隙里：哪块都不亮（None），不许硬塞给最近的那块。
    #[test]
    fn a_cursor_in_the_gap_between_screens_lights_neither() {
        let screens = [
            (1_u8, Rect::from_min_max(pos2(0.0, 0.0), pos2(100.0, 100.0))),
            (
                2_u8,
                Rect::from_min_max(pos2(200.0, 0.0), pos2(300.0, 100.0)),
            ),
        ];
        assert_eq!(lit_display(Some(pos2(150.0, 50.0)), &screens), None);
        // 一块屏都没有时同样不认
        assert_eq!(lit_display(Some(pos2(50.0, 50.0)), &[]), None);
    }

    /// 形状核查：认窗高亮必须先问「光标真在这块屏」（`cursor_here`），不许拿本地 (0,0) 去认。
    /// 真机读数：双屏时没鼠标的那扇遮罩也亮窗——它读不到指针退回 (0,0)，认了左上角那扇。
    #[test]
    fn the_window_hover_is_gated_by_the_cursor_being_on_this_screen() {
        let body = crate::system::implementation_lines(include_str!("selection.rs"));
        assert!(
            body.contains("let hover = if cursor_here"),
            "认窗高亮只在光标真所在的那块屏上做，另一块不许亮"
        );
    }

    /// 取消是**一个出口**（esc、点遮罩外、点「取消」三条都汇到这儿）：框没了、状态是终态。
    #[test]
    fn cancelling_drops_the_box_and_lands_in_one_terminal_state() {
        let field = main_screen();
        let mut sel = drag_all_the_way(&field, pos2(10.0, 10.0), pos2(200.0, 300.0));
        assert_eq!(sel.stage, Stage::Ready);
        sel.cancel();
        assert_eq!(sel.stage, Stage::Cancelled);
        assert!(sel.rect.is_none());
        assert!(sel.size_label(&field).is_none(), "取消了就不该还有尺寸可报");
    }

    /// 没选区时 `confirm` 不许凭空造一个：确认要有东西可裁，否则历史目录里会多一张空气。
    #[test]
    fn confirming_an_empty_selection_does_nothing() {
        let field = main_screen();
        let mut sel = Selection::fresh();
        assert_eq!(sel.confirm(), Stage::Idle);
        assert!(sel.rect.is_none());
        // 取消之后也一样：终态不许被"确认"救回来
        sel.take_window(&field, Rect::from_min_max(pos2(0.0, 0.0), pos2(10.0, 10.0)));
        sel.cancel();
        assert_eq!(sel.confirm(), Stage::Cancelled);
    }

    /// **点→像素只有一处换算**（REQ-104 的判据形状）：尺寸、裁剪、以及"这块屏能不能算"
    /// 都走同一个函数。这里锁住"2 倍屏裁出的像素是点框的两倍"与"零尺寸屏不猜 1.0"。
    #[test]
    fn the_point_to_pixel_conversion_happens_once_and_refuses_to_guess() {
        let field = main_screen();
        assert_eq!(field.scale(), Some(2.0));
        let px = field
            .to_pixels(Rect::from_min_max(pos2(100.0, 200.0), pos2(300.0, 400.0)))
            .expect("换算得出");
        assert_eq!((px.x, px.y, px.w, px.h), (200.0, 400.0, 400.0, 400.0));

        // 零宽/零高的屏：不许把 scale 默认成 1.0（那是"看着 1:1 像普遍规律"的老错）
        assert_eq!(
            Field::new(
                Rect::from_min_max(pos2(0.0, 0.0), pos2(0.0, 0.0)),
                vec2(0.0, 0.0)
            )
            .scale(),
            None
        );
        // 横纵比例不一致（旋转/非等比）也判"不可信"，不挑一个轴用
        assert_eq!(
            Field::new(
                Rect::from_min_max(pos2(0.0, 0.0), pos2(1000.0, 1000.0)),
                vec2(2000.0, 1000.0)
            )
            .scale(),
            None
        );
    }

    /// 动作条：**放不下就翻到上方，两侧都不许压住选区**（REQ-103 那条断言）。
    #[test]
    fn the_action_bar_flips_above_when_it_would_not_fit_below() {
        let field = main_screen();
        let bar = vec2(240.0, 34.0);
        let high = Rect::from_min_max(pos2(100.0, 100.0), pos2(400.0, 200.0));
        assert_eq!(bar::side(high, bar, field.bounds), bar::BarSide::Below);
        let rect = bar::rect(high, bar, bar::BarSide::Below);
        assert_eq!(rect.min.y, high.max.y + GAP, "紧贴选区下方");
        assert!(!rect.intersects(high), "下方那一档不许压住选区");

        let low = Rect::from_min_max(pos2(100.0, 900.0), pos2(400.0, 1070.0));
        assert_eq!(
            bar::side(low, bar, field.bounds),
            bar::BarSide::Above,
            "下面放不下就翻上去"
        );
        let flipped = bar::rect(low, bar, bar::BarSide::Above);
        assert_eq!(flipped.max.y, low.min.y - GAP);
        assert!(!flipped.intersects(low), "翻上去那一档同样不许压住选区");
    }

    /// **药丸宽按内容定**：颗数读 BAR_ICONS（加颗这里跟着长）+ 尺寸读数 + 颗间缝 + 两端留白；
    /// 没有读数时少一颗少一条缝。锁住"不再写死 300"——一句常见读数（约 60 点）配出来的
    /// 药丸远窄于 300，右边不再空一大截。
    #[test]
    fn the_bar_width_hugs_its_content() {
        let n = BAR_ICONS.len() as f32;
        let no_label = 2.0 * bar::SIDE_PAD + n * ICON_W + (n - 1.0) * bar::BTN_GAP;
        assert_eq!(
            bar_width(0.0, false),
            no_label,
            "没读数：全部颗 + 颗间缝 + 两端留白"
        );
        assert_eq!(
            bar_width(60.0, false),
            no_label + 60.0 + bar::BTN_GAP,
            "多一句读数：多它本身的宽 + 多一条缝"
        );
        assert_ne!(
            bar_width(60.0, false),
            300.0,
            "宽是按内容算出来的，不是先前写死的 300（颗数多了它会跟着长）"
        );
        // 选中工具：两颗定宽下拉各占 OPT_W，且各多一条缝（REQ-003）。
        assert_eq!(
            bar_width(60.0, true),
            bar_width(60.0, false) + 2.0 * OPT_W + 2.0 * bar::BTN_GAP,
            "选中工具：药丸长出两颗下拉，各加一份宽与一条缝"
        );
        assert_eq!(
            bar_width(0.0, true),
            bar_width(0.0, false) + 2.0 * OPT_W + 2.0 * bar::BTN_GAP,
            "没读数时选中工具同样长出两颗下拉"
        );
    }

    /// 药丸宽**不随选区宽变**：窄框与宽框配同一句读数，药丸一样宽、都以选区中线对齐、都不压选区。
    #[test]
    fn the_bar_width_does_not_follow_the_selection() {
        let field = main_screen();
        let narrow = Selection::from_rect(
            &field,
            Rect::from_min_max(pos2(100.0, 100.0), pos2(140.0, 140.0)),
        );
        let wide = Selection::from_rect(
            &field,
            Rect::from_min_max(pos2(100.0, 100.0), pos2(900.0, 500.0)),
        );
        let a = bar_of(&narrow, &field, 60.0, false).expect("有框就有药丸");
        let b = bar_of(&wide, &field, 60.0, false).expect("有框就有药丸");
        assert_eq!(a.width(), b.width(), "同一句读数，窄框宽框配同一颗药丸");
        assert_eq!(a.width(), bar_width(60.0, false));
        assert_eq!(a.center().x, narrow.rect.unwrap().center().x, "中线对齐");
        assert_eq!(b.center().x, wide.rect.unwrap().center().x, "中线对齐");
        assert!(!a.intersects(narrow.rect.unwrap()) && !b.intersects(wide.rect.unwrap()));
    }

    /// **形状核查：那一行留白 + 居中，不再齐平顶格。** 锁住用户那句"排列好丑"的修法：
    /// Area 原点左移 SIDE_PAD、颗间 BTN_GAP、整行在 BAR_H 里 Align::Center 垂直居中。
    #[test]
    fn the_bar_row_is_inset_padded_and_centered() {
        let body = crate::system::implementation_lines(include_str!("selection.rs"));
        assert!(
            body.contains(".fixed_pos(bar.min + egui::vec2(bar::SIDE_PAD, 0.0))"),
            "那一行从药丸左缘往里 SIDE_PAD 起摆，不再齐平左缘"
        );
        assert!(
            body.contains("ui.spacing_mut().item_spacing.x = bar::BTN_GAP;"),
            "颗间缝读共用件 BTN_GAP，不用默认间距"
        );
        assert!(
            body.contains("egui::Layout::left_to_right(egui::Align::Center)"),
            "整行在行高里垂直居中，不再贴着药丸顶"
        );
        assert!(
            body.contains("bar.width() - 2.0 * bar::SIDE_PAD"),
            "行宽 = 药丸宽减两端留白，恰好贴合"
        );
    }

    /// **按下落在药丸上，不许当成"在空白处起一个新框"。**
    ///
    /// 分两步钉住。第一步是几何：药丸按定义就在选区外面（缝 3 点），所以那一点对状态机
    /// 来说是 `Grab::Blank`——照旧写法走一遍 `press`，旧框会被换成一个零尺寸的点，药丸下一帧
    /// 就没了位置，那颗按钮再也收不到完整的按下—抬起（真机读数：点了对勾什么都不钉）。
    /// 第二步是代码形状：鼠标那一段必须先问 `on_bar`，抬起也必须只认"真的拖过"的那次。
    #[test]
    fn a_press_on_the_bar_is_not_a_selection_gesture() {
        let field = main_screen();
        let box_rect = Rect::from_min_max(pos2(600.0, 300.0), pos2(900.0, 500.0));
        let sel = Selection::from_rect(&field, box_rect);
        let bar = bar_of(&sel, &field, 60.0, false).expect("有框就该有药丸");
        assert_eq!(bar.min.y, box_rect.max.y + GAP, "药丸浮在选区下方 3 点");
        assert!(!bar.intersects(box_rect), "药丸不许压住选区");

        // 反面写法照走一遍：那一下会被判成"空白处按下"，旧框当场被顶掉
        let mut naive = sel;
        let grab = naive.press(&field, bar.center(), HANDLE_RADIUS);
        assert_eq!(grab, Grab::Blank, "药丸那一点在选区外，状态机眼里就是空白");
        assert!(
            naive.rect.is_some_and(|r| r.width() <= 0.0),
            "这一条是在记录那个陷阱：不加护栏，旧框就是会被换成一个点"
        );

        // 代码形状：护栏必须在
        let body = crate::system::implementation_lines(include_str!("selection.rs"));
        assert!(
            body.contains("if pressed && !on_bar"),
            "落在药丸上的那一下不许进状态机"
        );
        assert!(
            body.contains("if released && next.dragging"),
            "没真的拖过的抬起不许把 previous 那份旧值当结果还回来"
        );
        assert!(
            body.contains("let on_bar = bar_now.is_some_and"),
            "按下落点的判定与绘制必须读同一块矩形"
        );
    }

    /// **形状核查：药丸的底色画在药丸那一块，不许画到屏幕角上。**
    ///
    /// `egui::Area` 里的 painter 用的是**全局**坐标，`fixed_pos` 只管排版不管画笔。
    /// 这一条先前写成 `Rect::from_min_size(Pos2::ZERO, bar.size())`，真机逐点量到的是：
    /// 选区下方那一块仍是画面色（132），而屏幕左上角 (0,0) 起 300×34 多出一块暗档（36）——
    /// 也就是"三颗线稿浮在画面上、底色跑到角上"。注释里讨论过反面写法不算，只看代码行。
    #[test]
    fn the_bar_background_is_painted_on_the_bar_itself() {
        let body = crate::system::implementation_lines(include_str!("selection.rs"));
        let from = body
            .find("ice-selection.bar")
            .expect("药丸那一块得有个出处");
        let fill = &body[from..];
        let fill = &fill[..fill.find("allocate_ui_with_layout").unwrap_or(fill.len())];
        assert!(fill.contains("rect_filled("), "底色那一笔就在这个块里");
        assert!(
            !fill.contains("Pos2::ZERO"),
            "底色不许用原点算：Area 的 painter 是全局坐标，原点=屏幕左上角"
        );
    }

    /// **形状核查：推进槽里的那一刻，必须点名叫主窗那一扇视口。**
    ///
    /// 子窗闭包里 `ctx.request_repaint()` 走的是"当前视口 id"，只会重画遮罩自己。
    /// 真机上"esc 按了选区没了遮罩还在""对勾点了什么都不钉"就是这个形状——意图已经在槽里，
    /// 主窗一次都没醒。所以三个出口都只能调 `wake_root`，一处不许漏，也不许多（拖动与悬停
    /// 每帧都在变，那些不许把主窗叫醒，D-31~D-35 那一族）。
    #[test]
    fn every_intent_outlet_wakes_the_root_viewport() {
        let body = crate::system::implementation_lines(include_str!("selection.rs"));
        assert_eq!(
            body.matches("wake_root(&root)").count(),
            3,
            "esc/回车、单击认窗、药丸确认三颗——三个推意图的出口，一个不多一个不少；工具颗只改本地态，不推意图"
        );
        assert!(
            !body.contains("root.request_repaint()"),
            "拿主窗的 ctx 调 request_repaint 只会叫醒当前这一扇子窗，是个假动作"
        );
        assert_eq!(
            body.matches("queue.push").count() + body.matches("queue.extend").count(),
            3,
            "推意图的地方每处都得配一次 wake_root，加了新出口要先改这条断言"
        );
    }

    /// **D-42：新一轮截图必须读到空白选区。**
    ///
    /// 临时态不随遮罩关窗而清：确认那一支只把阶段推到 Confirmed，框还留在格子里。
    /// 用户 2026-11-10 原话「下次我打开还有上次的选框」。拿真存储走一遍读法：
    /// 同轮还在（拖动时每帧读写靠它），换轮空白，换屏也不许借。
    #[test]
    fn a_new_capture_round_reads_a_blank_selection() {
        let mut map = egui::util::IdTypeMap::default();
        let field = main_screen();
        let old = Selection::from_rect(
            &field,
            Rect::from_min_max(pos2(100.0, 100.0), pos2(300.0, 200.0)),
        );
        map.insert_temp(sel_key(1), (10_u64, old));

        let same = load_selection(&map, 1, 10);
        assert_eq!(same.rect, old.rect, "同一轮里框要一直在，不然拖动每帧都丢");

        let fresh = load_selection(&map, 1, 11);
        assert!(fresh.rect.is_none(), "新一轮不许带着上一轮的框");
        assert_eq!(fresh.stage, Stage::Idle);

        let other = load_selection(&map, 2, 10);
        assert!(other.rect.is_none(), "另一块屏更不许借这一扇的框");
    }

    /// 形状核查：写回那一格必须盖上这一轮的轮号，读法必须过 `load_selection`。
    /// 两处各写一份迟早分岔——D-42 正是"读的时候没问轮号"分出来的。
    #[test]
    fn the_selection_slot_is_stamped_with_the_round_it_belongs_to() {
        let body = crate::system::implementation_lines(include_str!("selection.rs"));
        assert!(
            body.contains("insert_temp(sel_key(display), (frozen_id, next))"),
            "写回那一格要带上轮号"
        );
        assert!(
            body.contains("data(|d| load_selection(d, display, frozen_id))"),
            "帧里的读法必须走那一个函数"
        );
    }

    /// 形状核查：遮罩窗必须透明底，根除按 F1 那一瞬白屏（用户 2026-11-13 真机复报）。
    /// deferred 子视口没有 eframe 只给根窗那层"建窗先藏、画完再露"的防白闪，不透明窗默认
    /// 原生底是白，窗先露而第一帧冻结图还没盖上那一小段就是白。透明底让那一小段透出桌面而非白。
    #[test]
    fn the_mask_window_keeps_a_transparent_backing_to_kill_the_white_flash() {
        let body = crate::system::implementation_lines(include_str!("selection.rs"));
        assert!(
            body.contains(".with_transparent(true)"),
            "遮罩窗必须透明底，否则按 F1 那一瞬白屏会长回来"
        );
        assert!(
            !body.contains(".with_transparent(false)"),
            "遮罩窗不许回到不透明底——那正是白屏的来源"
        );
    }

    /// **标注对象列表按 (屏, 轮号) 存取**：同轮同一份（遮罩画的与主窗裁的是同一张），
    /// 换轮全新空列表（D-42 同形），换屏不借。拿真存储走一遍，不抄读法。
    #[test]
    fn the_annotation_objects_is_per_screen_and_per_round() {
        let ctx = egui::Context::default();
        let a = annotation_objects(&ctx, 1, 10);
        a.lock().unwrap().push(AnnoObject {
            shape: Shape::Line {
                a: (0.0, 0.0),
                b: (10.0, 10.0),
                width: 2.0,
                color: PALETTE[0],
            },
            erased: Vec::new(),
        });
        let same = annotation_objects(&ctx, 1, 10);
        assert!(
            std::sync::Arc::ptr_eq(&a, &same),
            "同轮必须取到同一份：主窗确认时裁的就是遮罩画的那张"
        );
        assert!(same.lock().unwrap().revision > 0);
        let fresh = annotation_objects(&ctx, 1, 11);
        assert!(!std::sync::Arc::ptr_eq(&a, &fresh), "换轮换一份新对象列表");
        assert_eq!(fresh.lock().unwrap().revision, 0, "新列表必须一件没有");
        let other = annotation_objects(&ctx, 2, 10);
        assert_eq!(
            other.lock().unwrap().revision,
            0,
            "另一块屏不许借这一扇的列表"
        );
    }

    /// 工具态同样按轮号作废：新一轮不许带着上一轮激活的工具。
    #[test]
    fn tool_state_reads_blank_on_a_new_round() {
        let mut map = egui::util::IdTypeMap::default();
        let ts = ToolState {
            tool: Some(Tool::Rect),
            anchor: pos2(5.0, 5.0),
            last: pos2(5.0, 5.0),
            dragging: false,
            next_number: 1,
            text_input: None,
            thickness_idx: 1,
            color_idx: 0,
            stroke_points: Vec::new(),
            erase_points: Vec::new(),
            moving: None,
            move_last: pos2(0.0, 0.0),
        };
        map.insert_temp(tool_key(1), (10_u64, ts));
        assert_eq!(load_tool_state(&map, 1, 10).tool, Some(Tool::Rect));
        assert_eq!(load_tool_state(&map, 1, 11).tool, None, "换轮读空白");
        assert_eq!(load_tool_state(&map, 2, 10).tool, None, "换屏不借");
    }

    /// **D-46 锁死：下拉那层暗色 visuals 只许套在 scope 里，不许漏到全局。**
    /// `ui.set_visuals` 看着是 Ui 的方法，实则 Ui Deref 到 Context，调的是全局
    /// `Context::set_visuals`——scope 拦不住，亮色主界面随后整排按钮字发白（真机像素
    /// 量到字 238 对底 247）。这里走 `visuals_mut`（clone-on-write）真隔离：scope 内读到
    /// 暗墨、scope 外与全局样式都保持原墨。
    #[test]
    fn the_dropdown_dark_visuals_stay_inside_their_scope() {
        let ctx = egui::Context::default();
        ctx.set_theme(egui::Theme::Light);
        let before = ctx
            .style_of(egui::Theme::Light)
            .visuals
            .widgets
            .inactive
            .fg_stroke
            .color;
        let mut inside = None;
        let mut after = None;
        let output = ctx.run_ui(egui::RawInput::default(), |ui| {
            ui.scope(|ui| {
                let mut dark = ui.visuals().clone();
                dark.widgets.inactive.fg_stroke =
                    egui::Stroke::new(1.0, egui::Color32::from_gray(235));
                *ui.visuals_mut() = dark;
                inside = Some(ui.visuals().widgets.inactive.fg_stroke.color);
            });
            after = Some(ui.visuals().widgets.inactive.fg_stroke.color);
        });
        output.drop_without_applying_deltas();
        assert_eq!(
            inside,
            Some(egui::Color32::from_gray(235)),
            "scope 内那两颗下拉要读到暗底浅字"
        );
        assert_eq!(after, Some(before), "scope 外不许被带偏");
        assert_eq!(
            ctx.style_of(egui::Theme::Light)
                .visuals
                .widgets
                .inactive
                .fg_stroke
                .color,
            before,
            "全局亮色样式的文字墨色不许被 scope 改写（D-46 的真病因）"
        );
    }

    /// **形状核查：粗细 / 色板落层与预览同源读运行态档位**（REQ-003）。
    /// 实现段里不许再出现写死的中档 / 红色字面量——那正是"换档换色不生效"的旧写法；
    /// 两颗下拉的 id 与读 `ts` 档位的出处都得在。
    #[test]
    fn thickness_and_color_read_the_live_tier_everywhere() {
        let body = crate::system::implementation_lines(include_str!("selection.rs"));
        for lit in [
            "THICKNESS[1]",
            "PALETTE[0]",
            "BADGE_RADIUS[1]",
            "TEXT_SIZE[1]",
        ] {
            assert!(
                !body.contains(lit),
                "实现段不许写死档位字面量 {lit}：落层与预览都得读 ts 的档"
            );
        }
        assert!(
            body.contains("THICKNESS[ts.thickness_idx]") && body.contains("PALETTE[ts.color_idx]"),
            "落层 / 预览 / 序号 / 文字都读运行态档位"
        );
        assert!(
            body.contains("BADGE_RADIUS[ts.thickness_idx]")
                && body.contains("TEXT_SIZE[ts.thickness_idx]"),
            "序号半径与文字字号也随粗细档走"
        );
        assert!(
            body.contains("ice-selection.thickness") && body.contains("ice-selection.color"),
            "药丸里得有粗细与色板两颗下拉"
        );
        assert!(
            body.contains("THICKNESS_NAMES[ts.thickness_idx]")
                && body.contains("PALETTE_NAMES[ts.color_idx]"),
            "下拉选中文字读档位名，与档位值同源（annotate 那一份常量）"
        );
        assert!(
            body.contains("if pressed && !on_bar && !ui.ctx().is_pointer_over_egui()"),
            "工具按下要避开浮层：从下拉里选色不许在菜单底下的画布上点出一笔"
        );
    }

    /// **形状核查：esc 先退工具、不取消遮罩**（REQ-006）。工具态那一支不许推 Cancel。
    #[test]
    fn escape_exits_the_tool_before_it_cancels_the_mask() {
        let body = crate::system::implementation_lines(include_str!("selection.rs"));
        assert!(
            body.contains("if ts.tool.is_some() {"),
            "esc 必须先问工具态：有工具先退工具，框还在"
        );
    }

    /// **形状核查：工具态与选区态互斥**——选区手势整支包在 `ts.tool.is_none()` 里，
    /// 工具手势那一支不碰 `next.rect`（REQ-006 的默认不变就靠这个分离）。
    #[test]
    fn tool_gestures_and_selection_gestures_are_mutually_exclusive() {
        let body = crate::system::implementation_lines(include_str!("selection.rs"));
        assert!(
            body.contains("if ts.tool.is_none() {"),
            "选区手势只在没有激活工具时走"
        );
        let from = body
            .find("ts.anchor = clamp_pt(here)")
            .expect("工具态那一支得有出处");
        let to = body
            .find("if ts != tool_before")
            .expect("工具态写回得有出处");
        let tool_branch = &body[from..to];
        assert!(
            !tool_branch.contains("next.press") && !tool_branch.contains("next.drag"),
            "工具手势不许进选区状态机"
        );
    }

    /// 动作条十一颗：前八颗是工具（框/箭头/直线/铅笔/序号/文字/马赛克/橡皮），
    /// 后三颗复制/保存/对勾——**对勾在最后**（用户 2026-11-17 裁定）。
    #[test]
    fn the_bar_lists_eight_tools_then_copy_save_check() {
        assert_eq!(BAR_ICONS.len(), 11);
        assert_eq!(BAR_ICONS[10], BarIcon::Check, "对勾必须排在最后");
        for icon in &BAR_ICONS[..8] {
            assert!(tool_of(*icon).is_some(), "{icon:?} 必须是工具");
        }
        assert_eq!(tool_of(BAR_ICONS[4]), Some(Tool::Number));
        assert_eq!(tool_of(BAR_ICONS[5]), Some(Tool::Text));
        assert_eq!(tool_of(BAR_ICONS[6]), Some(Tool::Mosaic));
        assert_eq!(tool_of(BAR_ICONS[7]), Some(Tool::Eraser));
        for icon in &BAR_ICONS[8..] {
            assert!(tool_of(*icon).is_none(), "{icon:?} 是出口动作，不是工具");
        }
    }

    /// 工具手势与裁剪同一条换算：2× 屏上点 → 物理像素翻倍，出屏的点夹回屏内
    /// 也走同一条（119 的隐患正是把点坐标直接喂给物理像素的层函数）。
    #[test]
    fn tool_points_convert_through_the_same_scale_as_the_crop() {
        let field = main_screen();
        assert_eq!(
            point_to_px(field, pos2(100.0, 100.0)),
            Some((200.0, 200.0)),
            "2× 屏上 100 点必须是 200 像素"
        );
        assert_eq!(point_to_px(field, pos2(0.0, 0.0)), Some((0.0, 0.0)));
        assert_eq!(
            point_to_px(field, pos2(-5.0, 2000.0)),
            Some((0.0, 2160.0)),
            "出屏的点要夹回屏内再换算"
        );
    }

    /// 序号从 1 起、同轮接着数、新一轮回到 1（某截图工具 默认同口径）。
    #[test]
    fn the_serial_counter_starts_at_one_and_resets_each_round() {
        assert_eq!(ToolState::default().next_number, 1);
        let mut map = egui::util::IdTypeMap::default();
        let ts = ToolState {
            next_number: 5,
            ..ToolState::default()
        };
        map.insert_temp(tool_key(1), (10_u64, ts));
        assert_eq!(
            load_tool_state(&map, 1, 10).next_number,
            5,
            "同一轮要接着数"
        );
        assert_eq!(
            load_tool_state(&map, 1, 11).next_number,
            1,
            "新一轮必须从 1 起"
        );
    }

    /// 文字落层只走 commit 这一条：落完输入即关；空白不落、输入也关。
    /// 字体读不出的机器上自动跳过（与 ice-vision 字体用例同一个门槛）。
    #[test]
    fn committing_text_input_stamps_once_and_closes_the_input() {
        let Ok(_m) = FileMetrics::open() else { return };
        let field = Field::new(
            Rect::from_min_max(pos2(0.0, 0.0), pos2(200.0, 100.0)),
            vec2(200.0, 100.0),
        );
        let objects = std::sync::Arc::new(std::sync::Mutex::new(ObjectList::new()));
        let mut ts = ToolState {
            tool: Some(Tool::Text),
            text_input: Some((pos2(10.0, 10.0), "你好".to_string())),
            ..ToolState::default()
        };
        commit_text_input(&mut ts, &objects, field);
        assert!(ts.text_input.is_none(), "落完输入必须关掉");
        assert_eq!(objects.lock().unwrap().revision, 1, "落一件加一");
        let mut ts2 = ToolState {
            tool: Some(Tool::Text),
            text_input: Some((pos2(10.0, 10.0), "   ".to_string())),
            ..ToolState::default()
        };
        commit_text_input(&mut ts2, &objects, field);
        assert!(ts2.text_input.is_none(), "空白也要关输入");
        assert_eq!(objects.lock().unwrap().revision, 1, "空白不许落层");
    }

    /// **形状核查：工具手势全走 point_to_px 漏斗**，不许再有点坐标直喂层函数的旁路
    /// （119 隐患的根因；与互斥核查圈同一段代码）。
    #[test]
    fn every_tool_gesture_goes_through_the_pixel_funnel() {
        let body = crate::system::implementation_lines(include_str!("selection.rs"));
        let from = body
            .find("ts.anchor = clamp_pt(here)")
            .expect("工具态那一支得有出处");
        let to = body
            .find("if ts != tool_before")
            .expect("工具态写回得有出处");
        let tool_branch = &body[from..to];
        assert!(
            tool_branch.contains("point_to_px(ann_field"),
            "工具手势的点必须经漏斗换成物理像素"
        );
        assert!(
            !tool_branch.contains("(ts.last.x, ts.last.y)"),
            "铅笔/橡皮不许把点坐标直接喂给层函数"
        );
        assert!(
            !tool_branch.contains("let (pa, pb) = ((a.x, a.y), (b.x, b.y))"),
            "松手落层不许把点坐标直接喂给层函数"
        );
    }

    /// **形状核查：工具态写回必须排在药丸开关之后**（D-44 的根因）。
    /// 写回抢在开关前面，拨动的结果存不进去，真机就是"工具按钮点了没反应"。
    #[test]
    fn the_tool_write_back_comes_after_the_bar_toggle() {
        let body = crate::system::implementation_lines(include_str!("selection.rs"));
        let toggle = body
            .find("ts.tool = if ts.tool == Some(t)")
            .expect("药丸里得有工具开关");
        let write_back = body
            .find("if ts != tool_before")
            .expect("工具态写回得有出处");
        assert!(
            write_back > toggle,
            "写回必须在开关之后，不然开关结果存不进去"
        );
    }

    /// D-45：「已经有框」这一轮全屏幕共享——任一屏画了框就算有，新一轮回到没有。
    /// 拿真存储走一遍，不抄读法。
    #[test]
    fn the_any_selection_flag_is_round_scoped_and_shared_across_screens() {
        let ctx = egui::Context::default();
        assert!(!any_selection(&ctx, 10), "新一轮开始没有框");
        mark_selection(&ctx, 10);
        assert!(any_selection(&ctx, 10), "任一屏画了框，这一轮就算有框");
        assert!(!any_selection(&ctx, 11), "换轮回到没有框");
    }

    /// 形状核查：认窗的两个出口——悬停高亮与单击截整窗——都必须问这一轮的
    /// 「已经有框」标记（D-45：A 屏画了框，B 屏不许再认、更不许单击截走）。
    #[test]
    fn both_window_recognition_gates_ask_the_round_flag() {
        let body = crate::system::implementation_lines(include_str!("selection.rs"));
        let n = body.matches("!any_selection(&root, frozen_id)").count();
        assert!(n >= 2, "悬停高亮与单击截整窗两处都要问标记，只找到 {n} 处");
    }
}
