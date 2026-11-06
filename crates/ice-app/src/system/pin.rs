//! 贴图窗（CHG-009 REQ-003/REQ-004）。
//!
//! **这一格的地基是 2026-11-05 那晚仓外探针量出来的**（事实 17/25/26/27/29/32/33）：
//! 子 viewport 里那张纹供得出来、画的就是我们算的那块几何、无边框子窗收得到键盘且只给
//! 焦点那一张、同时钉 8 张各自出图。所以这个模块不是"猜一个形态再改"，是把读数控成代码。
//!
//! 三条从读数里出来的硬规矩，写在这里而不是散在绘制里：
//! 1. **带子是窗口的一部分**，按钮 SHALL NOT 浮在图片上——用户那句"右下角有漂浮在图片
//!    外侧的翻译按钮"是字面意思，图上压控件就违背它。
//! 2. **收起带子 = 把窗口本身缩到图片那一块**（`ViewportCommand::InnerSize`）。实测过
//!    只画不缩的那一档：那条带子不画东西、不透明（采样 `#0800`）、**还照吃命中**——
//!    一条隐形陷阱。所以"画但不看得见"这种收法禁止出现。
//! 3. **位置只从 `ViewportInfo::outer_rect` 读**，本模块 SHALL NOT 自己累计第二份坐标
//!    （拖动、系统搬窗、隐藏再回来三种情况它都跟着，事实 32）。
//!
//! `scale` 那一处除法是全局唯一出处（像素 → 点），窗口尺寸、带子重算、文字层坐标三处
//! 都读同一个结果；各自再除一次或都忘了除，就是"贴图比用户框的大一倍"那种现场（H9）。

use egui::{Pos2, Rect, Response, Sense, Ui, Vec2};

/// 带子的高度：一颗按钮（与 E 区那排同源，同一个字号加同一套留白）加上下各 8 留白。
///
/// 为什么是常量而不是按内容量：收起时窗口要缩到**图片那一块**，展开时窗口是
/// `图片 + BAND_H`——这个差值必须是确定的数，否则"缩窗口"那一刻算出来的高度和实际
/// 摆出来的带子对不上，就会留下几像素的死区（就是第 2 条硬规矩要防的东西）。
pub const BAND_H: f32 = 34.0;

/// 带子里那四颗，**顺序就是显示顺序**。
///
/// 一处声明、长度锁 4：用户明说"只要四项"（Q-006 默认 翻译 / 复制图像 / 保存图像 / 关闭），
/// 带子和右键菜单读同一张表。加第五项要改这一处，而 `exactly_four_actions` 那条断言会先响。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PinAction {
    Translate,
    CopyImage,
    SaveImage,
    Close,
}

/// 四颗按钮的表。带子里画它，右键菜单也列它——**不是两套清单**。
pub const ACTIONS: [PinAction; 4] = [
    PinAction::Translate,
    PinAction::CopyImage,
    PinAction::SaveImage,
    PinAction::Close,
];

impl PinAction {
    /// 按钮上那两个字的来源。文案是一次性资产（`docs/05` §6 那条），带子与菜单共用。
    pub fn label(self) -> &'static str {
        match self {
            PinAction::Translate => "翻译",
            PinAction::CopyImage => "复制",
            PinAction::SaveImage => "保存",
            PinAction::Close => "关",
        }
    }

    /// 右键菜单里要说全的名字（带子上挤成两个字，菜单里不必省）。
    pub fn menu_label(self) -> &'static str {
        match self {
            PinAction::Translate => "翻译这张图",
            PinAction::CopyImage => "复制图像",
            PinAction::SaveImage => "保存图像",
            PinAction::Close => "关闭这张贴图",
        }
    }
}

/// 一张贴图该占的窗口尺寸与两块矩形。**这条 REQ 唯一能被无头锁住的部分**。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PinLayout {
    /// 窗口内容尺寸（点）。收起时它等于图片那块，展开时多出 `BAND_H`
    pub window_size: Vec2,
    /// 图片画在哪块（点，左上角是窗口原点）
    pub image_rect: Rect,
    /// 带子占的那块。收起时它整块落在窗口之外，宽度为 0——**不是"在里面但没画"**
    pub band_rect: Rect,
}

/// 像素尺寸 + 开合 + 那张屏的 `pixels_per_point` → 窗口几何。
///
/// `image_px` 是**像素**（PNG 读出来的原尺寸），`scale` 是那张贴图所在屏的 ppp。
/// 除法只在这一处发生，产出的三个矩形全是点。
pub fn pin_layout(image_px: Vec2, band_open: bool, scale: f32) -> PinLayout {
    // 唯一一处除法。ppp 拿不到（0 或负）时按 1.0 兜住——那不是"截错了"，
    // 而是这一帧还没有屏信息，宁可 1:1 也不能算出 0 尺寸的窗口
    let scale = if scale > 0.0 { scale } else { 1.0 };
    let size = image_px / scale;
    let image_rect = Rect::from_min_size(Pos2::ZERO, size);
    let window_size = if band_open {
        Vec2::new(size.x, size.y + BAND_H)
    } else {
        size
    };
    let band_rect = if band_open {
        Rect::from_min_size(Pos2::new(0.0, size.y), Vec2::new(size.x, BAND_H))
    } else {
        // 收起：整块挪到窗口之外并压成 0 宽。命中测试因此永远碰不到它（REQ-004 末句
        // "不可见就是不可点，不画灰着的假按钮"）
        Rect::from_min_size(Pos2::new(window_size.x, window_size.y), Vec2::ZERO)
    };
    PinLayout {
        window_size,
        image_rect,
        band_rect,
    }
}

/// 带子里那一排按钮画进 `ui`，按下的动作推进 `sink`。
///
/// **带子收起时这个函数根本不会被调**（窗口没那块地方），所以这里不需要再判一次开合；
/// 反过来，如果哪天有人给"收起"的贴图也画了这几颗，`band_rect` 的宽度是 0，按钮挤不进
/// 命中区——两种防线都留着。
pub fn band_buttons(ui: &mut Ui, sink: &mut Vec<PinAction>) {
    ui.horizontal(|ui| {
        for action in ACTIONS {
            // 「关」按下去就是把这张从状态里摘掉，别的什么都不动（历史文件照旧）
            if ui.button(action.label()).clicked() {
                sink.push(action);
            }
        }
    });
}

/// 图片本体那块的命中意图：**按住就能拖**（REQ-004 第一条）。
///
/// 首选 `with_movable_by_background(true)`——实测无边框子窗整张能拖着走，拖到的位置
/// `outer_rect` 读得到（事实 29/32），不需要我们自己接拖动事件算增量；这条只负责"这块
/// 区域该被当作窗口本体而不是控件"，真的拖动由系统做。
pub fn image_area(ui: &mut Ui, size: Vec2) -> Response {
    ui.allocate_rect(Rect::from_min_size(Pos2::ZERO, size), Sense::drag())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn px(w: f32, h: f32) -> Vec2 {
        Vec2::new(w, h)
    }

    fn at(x: f32, y: f32) -> Pos2 {
        Pos2::new(x, y)
    }

    /// 展开：窗口 = 图片 + 一条带子，带子在图片**下面**，图片那块一个像素都没被挤
    #[test]
    fn the_open_band_sits_below_the_image_and_grows_the_window() {
        let l = pin_layout(px(512.0, 360.0), true, 1.0);
        assert_eq!(l.window_size, px(512.0, 360.0 + BAND_H));
        assert_eq!(l.image_rect.max, at(512.0, 360.0));
        assert_eq!(l.band_rect.min, at(0.0, 360.0), "带子必须紧贴图片下缘");
        assert_eq!(l.band_rect.height(), BAND_H);
        assert_eq!(l.band_rect.width(), 512.0, "带子与图片同宽");
        // 两块不许重叠：图上压控件就违背"漂浮在图片外侧"那句原话
        assert!(l.image_rect.max.y <= l.band_rect.min.y);
    }

    /// 收起：**窗口本身**缩到图片那一块，带子整块到窗口之外且宽度为 0。
    /// 这条断言挡的是"画但不看得见"那种收法——实测它会留下一条不吃绘制、照吃命中的死区。
    #[test]
    fn a_closed_band_shrinks_the_window_itself_and_leaves_no_dead_strip() {
        let open = pin_layout(px(512.0, 360.0), true, 1.0);
        let closed = pin_layout(px(512.0, 360.0), false, 1.0);
        assert_eq!(closed.window_size, px(512.0, 360.0), "收起 = 缩到图片本身");
        assert_eq!(open.window_size.y - closed.window_size.y, BAND_H);
        // 图片那块不许因为收带子而挪动或变形（往下收，不把图挪走）
        assert_eq!(closed.image_rect, open.image_rect);
        assert_eq!(closed.band_rect.width(), 0.0);
        assert!(closed.band_rect.min.x >= closed.window_size.x);
        assert!(closed.band_rect.min.y >= closed.window_size.y);
    }

    /// `scale = 2.0` 那一档：**点边长是像素的一半**，而且只在这一处除。
    /// 这条**不声称证明了真机行为**（H9 要一块 HiDPI 屏，归 W-5.7），只证明换算只有一个出处。
    #[test]
    fn the_pixel_to_point_division_happens_exactly_once() {
        let l = pin_layout(px(1000.0, 800.0), true, 2.0);
        assert_eq!(
            l.image_rect.max,
            at(500.0, 400.0),
            "2 倍屏上贴图不该有一倍大"
        );
        assert_eq!(
            l.window_size.y,
            400.0 + BAND_H,
            "带子那一段是点，不再除一次"
        );
        assert_eq!(l.band_rect.min.y, 400.0);
        assert_eq!(l.band_rect.height(), BAND_H);
    }

    /// ppp 拿不到时按 1.0 兜：宁可 1:1，也不能算出 0 尺寸或负尺寸的窗口
    #[test]
    fn a_missing_scale_falls_back_to_one_not_to_zero() {
        for bad in [0.0, -1.0, -0.0] {
            let l = pin_layout(px(200.0, 100.0), true, bad);
            assert_eq!(l.window_size, px(200.0, 100.0 + BAND_H), "ppp={bad}");
            assert!(l.window_size.x > 0.0 && l.window_size.y > 0.0);
        }
    }

    /// **右键菜单与带子读同一张表，而且恰好四项**（用户明说"只要四项"，
    /// 加第五项要改这一处，而这条断言先响）。
    #[test]
    fn exactly_four_actions_shared_by_the_band_and_the_menu() {
        assert_eq!(ACTIONS.len(), 4, "带子与菜单的动作表被改长了");
        let labels: Vec<&str> = ACTIONS.iter().map(|a| a.label()).collect();
        let menus: Vec<&str> = ACTIONS.iter().map(|a| a.menu_label()).collect();
        for l in &labels {
            assert!(
                labels.iter().filter(|x| *x == l).count() == 1,
                "带子上有两颗同名：{l}"
            );
        }
        for m in &menus {
            assert!(
                menus.iter().filter(|x| *x == m).count() == 1,
                "菜单里有两项同名：{m}"
            );
        }
        // 四件事各归一件，不许有第五件（某截图工具 那种十几项不是他要的）
        assert_eq!(
            labels,
            vec!["翻译", "复制", "保存", "关"],
            "带子上的字改了要顺着文案四条线一起改（docs/05 §6）"
        );
    }

    /// 「关」只是这张贴图的事：它的名字里不许混进"退出应用"那种含义。
    #[test]
    fn the_close_action_is_only_about_this_pin() {
        assert_eq!(PinAction::Close.menu_label(), "关闭这张贴图");
        assert!(!PinAction::Close.menu_label().contains("退出"));
    }

    /// 窗口尺寸随图片尺寸线性走，且**左上角永远是原点**：位置不由这里算，
    /// 只能从 `ViewportInfo::outer_rect` 读（本模块出现第二份坐标累计就是缺陷）。
    #[test]
    fn the_layout_never_invents_a_position() {
        for (w, h) in [(100.0, 50.0), (512.0, 360.0), (1920.0, 1080.0)] {
            let l = pin_layout(px(w, h), true, 1.0);
            assert_eq!(l.image_rect.min, Pos2::ZERO, "几何里不许出现第二个坐标源");
            assert_eq!(l.window_size, px(w, h + BAND_H));
        }
    }
}
