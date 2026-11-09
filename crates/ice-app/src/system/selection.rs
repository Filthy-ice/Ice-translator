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

use egui::{Id, Pos2, Rect, Vec2, pos2};
use ice_core::document::PixelRect;

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

/// "按下到抬起没挪动多少算单击"的阈值（点）。REQ-108 的"单击截整窗"与 REQ-102 的
/// "拖动=自由矩形"共用这一条判据，两处不许各写一个数。
pub const CLICK_SLOP: f32 = 3.0;

/// 压暗那一档（选区之外整体乘到这个亮度）。55% 是 某截图工具 那个量级的默认（Q-005），
/// 只有用户的眼睛能定，改这一格不动任何逻辑。
pub const DIM: u8 = 140;

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
    /// 按下那一点（拖新框与移动的锚点）
    anchor: egui::Pos2,
    /// 这次按下抢到了什么
    grab: Grab,
    /// 这一帧有没有在拖
    dragging: bool,
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
            }
            Grab::Inside | Grab::Handle(_) => {}
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
                if !self.moved && (to - self.anchor).length() <= CLICK_SLOP {
                    return;
                }
                self.moved = true;
                self.rect = Some(Rect::from_two_pos(self.anchor, to));
            }
            Grab::Inside => {
                if let Some(rect) = self.rect {
                    let size = rect.size();
                    // 移动不许把框推出屏外：先算整块能放得下的范围，再平移
                    let min_x = field.bounds.min.x;
                    let max_x = (field.bounds.max.x - size.x).max(min_x);
                    let min_y = field.bounds.min.y;
                    let max_y = (field.bounds.max.y - size.y).max(min_y);
                    let moved = pos2(
                        (rect.min.x + (to.x - self.anchor.x)).clamp(min_x, max_x),
                        (rect.min.y + (to.y - self.anchor.y)).clamp(min_y, max_y),
                    );
                    self.rect = Some(Rect::from_min_size(moved, size));
                    self.anchor = moved;
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

/// 动作条放哪：**优先选区下方，放不下翻到上方**（REQ-103 的那条断言）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BarSide {
    Below,
    Above,
}

pub fn bar_side(sel: egui::Rect, bar: Vec2, field: &Field) -> BarSide {
    let below = sel.max.y + GAP + bar.y;
    if below <= field.bounds.max.y {
        BarSide::Below
    } else {
        BarSide::Above
    }
}

/// 动作条与选区之间的缝（点）。
pub const GAP: f32 = 8.0;

/// 动作条那一块的矩形。**任何一侧都不许压住选区**——那条断言在下面这条函数里就是
/// "矩形与选区不相交"，不是"看起来没压"。
pub fn bar_rect(sel: egui::Rect, bar: Vec2, side: BarSide) -> egui::Rect {
    let center_x = (sel.min.x + sel.max.x) / 2.0;
    let min = pos2(
        center_x - bar.x / 2.0,
        match side {
            BarSide::Below => sel.max.y + GAP,
            BarSide::Above => sel.min.y - GAP - bar.y,
        },
    );
    Rect::from_min_size(min, bar)
}

// 三条常量的边界钉在**编译期**：写成运行期断言会被判"常量比较恒真"而形同虚设，
// 它要的效果从来不是"跑一下看看"，是"谁改坏了就编译不过"。
/// 压暗不许等于"没压"(255)，也不许等于"看不见原图"(0)；具体档位由走查 W-6.2 定（Q-005）。
const _: () = assert!(DIM >= 60 && DIM <= 200);
/// 单击阈值必须明显小于把手命中半径，否则"点在把手上"会先被判成单击，八个把手靠内那圈就废了。
const _: () = assert!(CLICK_SLOP > 0.0 && CLICK_SLOP < HANDLE_RADIUS);
/// 八个把手 = 4 角 + 4 边。改了它，[`handle_center`] 的编号与 [`grab`] 的命中会一起分岔。
const _: () = assert!(HANDLE_COUNT == 8);

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
        assert_eq!(bar_side(high, bar, &field), BarSide::Below);
        let rect = bar_rect(high, bar, BarSide::Below);
        assert_eq!(rect.min.y, high.max.y + GAP, "紧贴选区下方");
        assert!(!rect.intersects(high), "下方那一档不许压住选区");

        let low = Rect::from_min_max(pos2(100.0, 900.0), pos2(400.0, 1070.0));
        assert_eq!(
            bar_side(low, bar, &field),
            BarSide::Above,
            "下面放不下就翻上去"
        );
        let flipped = bar_rect(low, bar, BarSide::Above);
        assert_eq!(flipped.max.y, low.min.y - GAP);
        assert!(!flipped.intersects(low), "翻上去那一档同样不许压住选区");
    }
}

// ---- 遮罩那一扇窗（egui 侧） ----

/// 遮罩能对外发出的**全部意图**。一条枚举、一处映射——与贴图那一条纪律同形：
/// 键、按钮、右键若各写一套，出的就是"点按钮有效、按键没反应"那类只对一半的缺陷。
#[derive(Debug, Clone, PartialEq)]
pub enum MaskOut {
    /// 这一帧的按下/拖动改写了选区。带屏序号：**选区只在那一块屏内**（Q-004）
    Select { screen: usize, sel: Selection },
    /// 动作条那颗「钉住」或回车：确认 = 裁 → 落盘 → 走贴图链
    Pin,
    /// 动作条那颗「复制」：确认但不钉（F3「固定」的落点就是这一档）
    Copy,
    /// 动作条那颗「保存」：同上，走原生保存面板那条既有接缝
    Save,
    /// esc 或动作条那颗「取消」：什么都不留
    Cancel,
}

/// 遮罩窗的标题。**抬层那一步按它找窗**（`system::cover`），所以这一格只许有一处定义。
pub fn mask_title(display: u8) -> String {
    format!("IceTranslator 选区 {display}")
}

fn slot_key(display: u8) -> Id {
    Id::NULL.with("ice-selection.mask.slot").with(display)
}

fn viewport_of(display: u8) -> egui::ViewportId {
    egui::ViewportId::from_hash_of(Id::NULL.with("ice-selection.mask").with(display))
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
    /// 第几块屏（也是 `Selection` 的坐标归属）
    pub screen: usize,
    /// `-D` 的序号，用来命名窗口与配对
    pub display: u8,
    /// 那块屏的点尺寸
    pub points: Vec2,
    /// 冻结帧的纹理（**由根上下文加载**，与贴图那一条同一个做法）
    pub texture: egui::TextureId,
}

/// 画一扇遮罩。每帧对该 show 的每块屏各调一次（CHG-009 事实 25 那条：deferred viewport 是每帧 show）。
///
/// 压暗 = **整屏一张冻结图乘一个暗档**，选区那块再按原亮度重画一遍。
/// 这里刻意不用透明窗口、不用鼠标穿透：那两个属性正是 D-29 那次吞掉全部点击与键盘的元凶，
/// 而乘暗这一条路在探针里逐点核对过（红 `(220,30,30)` → `(99,13,13)`）。
pub fn show(ctx: &egui::Context, screen: &MaskScreen, sel: Selection) {
    let viewport = viewport_of(screen.display);
    let builder = egui::ViewportBuilder::default()
        .with_title(mask_title(screen.display))
        .with_decorations(false)
        .with_transparent(false)
        .with_has_shadow(false)
        // 这一档只是**下限**：真盖住 Dock 与菜单栏靠 `system::cover` 那一次原生抬层，
        // 每帧幂等重下。留着它是为了让抬层失败时退回"至少还置顶在普通窗之上"。
        .with_always_on_top()
        .with_position(egui::pos2(0.0, 0.0))
        .with_inner_size(screen.points);

    let texture = screen.texture;
    let points = screen.points;
    let index = screen.screen;
    let root = ctx.clone();
    let slot = slot_of(ctx, screen.display);

    ctx.show_viewport_deferred(viewport, builder, move |ui, _class| {
        let child = ui.ctx().clone();
        let field = Field::new(Rect::from_min_size(Pos2::ZERO, points), points);
        let full = ui.max_rect();
        // 整屏压暗
        ui.painter().image(
            texture,
            full,
            Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)),
            egui::Color32::from_rgb(DIM, DIM, DIM),
        );
        // 选区那块按原亮度 + 白边 + 八个把手
        if let Some(rect) = sel.rect {
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
                    Rect::from_center_size(center, Vec2::splat(6.0)),
                    0.0,
                    egui::Color32::WHITE,
                );
            }
        }

        // 键盘：esc 取消、回车钉住。收不收得到取决于应用是否前台——那一步在 `cover::focus`，
        // 每帧幂等重下（实测激活要一两帧才落地，"发一次就当成功"是错的）。
        let keys: Vec<MaskOut> = child.input(|i| {
            i.events
                .iter()
                .filter_map(|e| match e {
                    egui::Event::Key {
                        key: egui::Key::Escape,
                        pressed: true,
                        ..
                    } => Some(MaskOut::Cancel),
                    egui::Event::Key {
                        key: egui::Key::Enter,
                        pressed: true,
                        ..
                    } => Some(MaskOut::Pin),
                    _ => None,
                })
                .collect()
        });
        if !keys.is_empty() {
            if let Ok(mut queue) = slot.lock() {
                queue.extend(keys);
            }
            root.request_repaint();
        }

        // 鼠标：按下落在哪 → 拖动 → 抬起。这一帧算出的新状态原样交回（`Selection` 是 Copy）
        let pressed = ui.input(|i| i.pointer.button_pressed(egui::PointerButton::Primary));
        let released = ui.input(|i| i.pointer.button_released(egui::PointerButton::Primary));
        let here = ui.input(|i| i.pointer.interact_pos()).unwrap_or(Pos2::ZERO);
        let mut next = sel;
        let mut changed = false;
        if pressed {
            next.press(&field, here, HANDLE_RADIUS);
            changed = true;
        } else if ui.input(|i| i.pointer.button_down(egui::PointerButton::Primary)) {
            next.drag(&field, here);
            changed = true;
        }
        if released {
            next.release(&field, here);
            changed = true;
        }
        if changed {
            if let Ok(mut queue) = slot.lock() {
                queue.push(MaskOut::Select {
                    screen: index,
                    sel: next,
                });
            }
            // **这一句是 D-32 的正解**。选区存在主窗那侧，而指针事件只叫醒这扇子窗：
            // 不叫主窗，主循环就停在"上一次画出来的那一帧"，于是按下/拖动算了半天
            // 也没人把结果写回去 —— 用户看到的就是"鼠标拖了，什么框都没有"。
            root.request_repaint();
        }

        // 动作条：**圆角药丸，浮在选区下方，放不下翻上方**；不许铺满整幅图宽（REQ-103）
        if let Some(rect) = next.rect.filter(|r| r.width() > 0.0 && r.height() > 0.0) {
            let size = Vec2::new(BAR_W, BAR_H);
            let side = bar_side(rect, size, &field);
            let bar = bar_rect(rect, size, side);
            let _ = side;
            // 用 Area 浮在遮罩之上：药丸的位置跟着选区算，不占窗口布局（占位会把图片挤走）
            egui::Area::new(Id::NULL.with("ice-selection.bar"))
                .fixed_pos(bar.min)
                .order(egui::Order::Foreground)
                .show(ui.ctx(), |ui| {
                    ui.painter().rect_filled(
                        Rect::from_min_size(Pos2::ZERO, bar.size()),
                        bar.height() / 2.0,
                        egui::Color32::from_rgba_premultiplied(28, 28, 30, 235),
                    );
                    ui.horizontal(|ui| {
                        // 三颗：**对勾 / 复制 / 保存**。对勾就是"完成并钉住"（他原话
                        // "点对勾和固定到屏幕上是一样的"）；栏里不放「取消」——esc 与右键
                        // 承担。图标全是**线稿**，不用字符（本仓出过字符显示成方块的缺陷）。
                        for icon in BAR_ICONS {
                            if icon_button(ui, icon).clicked() {
                                let out = match icon {
                                    BarIcon::Check => MaskOut::Pin,
                                    BarIcon::Copy => MaskOut::Copy,
                                    BarIcon::Save => MaskOut::Save,
                                };
                                if let Ok(mut queue) = slot.lock() {
                                    queue.push(out);
                                }
                                root.request_repaint();
                            }
                        }
                        // 尺寸读数与裁剪同源（同一个 `Field`），这里只报一次
                        if let Some(text) = next.size_label(&field) {
                            ui.label(text);
                        }
                    });
                });
        }
    });
}

/// 栏里那三颗，顺序就是显示顺序（用户 2026-11-09 晚逐条定下：对勾取代「钉住」、
/// 栏里不放「取消」）。加一颗要改这张表，`the_bar_has_three_icons_in_this_order` 会先响。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum BarIcon {
    /// 完成 = 钉到屏幕上
    Check,
    Copy,
    Save,
}

pub const BAR_ICONS: [BarIcon; 3] = [BarIcon::Check, BarIcon::Copy, BarIcon::Save];

/// 一颗线稿图标按钮：自己画（两段线/两块圆角框/一个盘子加一支箭头），返回是否被点。
///
/// 为什么不用 `ui.button("✔")`：字符依赖字体，本仓因为字符图标出过显示成方块的缺陷（D-5 那一族）。
fn icon_button(ui: &mut egui::Ui, icon: BarIcon) -> egui::Response {
    let size = egui::vec2(26.0, 26.0);
    let (id, rect) = ui.allocate_space(size);
    let response = ui.interact(rect, id, egui::Sense::click());
    let rect = rect.shrink(4.0);
    let ink = if response.hovered() {
        egui::Color32::WHITE
    } else {
        egui::Color32::from_rgb(0xC8, 0xCA, 0xCF)
    };
    let stroke = egui::Stroke::new(1.6, ink);
    let painter = ui.painter();
    match icon {
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
            let back = egui::Rect::from_min_size(
                egui::pos2(rect.min.x, rect.min.y),
                egui::vec2(rect.width() * 0.62, rect.height() * 0.74),
            );
            let front = back.translate(egui::vec2(rect.width() * 0.3, rect.height() * 0.26));
            painter.rect_stroke(front, 3.0, stroke, egui::StrokeKind::Inside);
            painter.rect_stroke(back, 3.0, stroke, egui::StrokeKind::Inside);
        }
        BarIcon::Save => {
            // 一支向下的箭头落进一个盘子（"存到别处"的通行读法）
            let top = egui::pos2(rect.center().x, rect.min.y);
            let bottom = egui::pos2(rect.center().x, rect.center().y + rect.height() * 0.12);
            painter.line_segment([top, bottom], stroke);
            painter.line_segment(
                [
                    egui::pos2(
                        rect.min.x + rect.width() * 0.24,
                        rect.min.y + rect.height() * 0.3,
                    ),
                    bottom,
                ],
                stroke,
            );
            painter.line_segment(
                [
                    egui::pos2(
                        rect.max.x - rect.width() * 0.24,
                        rect.min.y + rect.height() * 0.3,
                    ),
                    bottom,
                ],
                stroke,
            );
            painter.line_segment(
                [
                    egui::pos2(rect.min.x, rect.center().y),
                    egui::pos2(rect.min.x, rect.max.y),
                ],
                stroke,
            );
            painter.line_segment(
                [
                    egui::pos2(rect.min.x, rect.max.y),
                    egui::pos2(rect.max.x, rect.max.y),
                ],
                stroke,
            );
            painter.line_segment(
                [
                    egui::pos2(rect.max.x, rect.max.y),
                    egui::pos2(rect.max.x, rect.center().y),
                ],
                stroke,
            );
        }
    }
    let tip = match icon {
        BarIcon::Check => "钉到屏幕上（与按回车相同）",
        BarIcon::Copy => "复制这张图到剪贴板",
        BarIcon::Save => "存成文件",
    };
    response.on_hover_text(tip)
}

/// 动作条的尺寸（药丸）。宽度按四颗按钮 + 一句尺寸读数估的固定值：
/// 它 SHALL NOT 跟着选区宽度变（用户那句"标注按钮怎么有背景"要的就是浮着的小条，不是一条 bar）。
pub const BAR_W: f32 = 300.0;
pub const BAR_H: f32 = 34.0;
