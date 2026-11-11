//! 两条药丸共用的几何 + 贴图自己的文字按钮（CHG-011 REQ-001 / REQ-002 / REQ-009）。
//!
//! 缝与翻转判据从 `selection.rs` 搬来共用：遮罩那条药丸与贴图这条药丸 SHALL 读同一套
//! 几何，各写一份迟早分岔（上一版遮罩正是分岔出"按下落在药丸上被当成起新框"那个陷阱）。
//! **线稿 `icon_button` 不在这儿**——改判后贴图消费文字按钮，线稿只剩遮罩一个消费者，
//! 留在 `selection.rs` 不动（REQ-002 改口记录）。
#![allow(dead_code)]
// 浮空药丸那条路（另起一扇透明窗 + 进程级 alpha）在真机上证伪了两件事：拖动跟随一卡一卡
// （独立窗跟不上系统拖窗）、进程级透明把主窗也变成半透明坏了截图观感。贴图暂时退回"带子是窗口
// 一部分"的旧形态，`selection.rs` 只共用这里的**几何**（GAP/side/rect）。下面 `text_button` /
// `paint_shadow` / 悬停那三个常量先**留仓不接线**，留给"把带子本身画成圆角浮起样子"的下一次尝试。

use egui::{Color32, Rect, Vec2, pos2};

/// 药丸与锚点之间的缝（点）。**3 是他给的数**（2026-11-09 晚原话「距离贴图上/在下
/// 大约 3px 的圆角的单独操作栏」，参照 某截图工具 / 某聊天软件截图）：贴着但不压住，一眼看得出
/// 这条药丸是给这块锚点用的。改这个数不动任何判定——两侧都从这一处读。
pub const GAP: f32 = 3.0;

/// 一颗文字按钮的槽宽（点）：两字标签 + 左右留白。药丸宽度按内容定（[`width_for`]），
/// SHALL NOT 跟着锚点宽走（他那句"太丑了"指的就是整宽一条 bar）。
pub const TEXT_BTN_W: f32 = 44.0;

/// 文字按钮之间的间距（点）与药丸两端留白（点）。[`width_for`] 与药丸那一行的版式
/// 都从这两处读，不各写一个数。
pub const BTN_GAP: f32 = 6.0;
pub const SIDE_PAD: f32 = 10.0;

/// 悬停那一颗垫的淡圆角底（REQ-009 / Q-005：白 alpha 26，约一成白叠在药丸底色上）。
/// 用 `from_rgba_unmultiplied_const` 而不是 `from_white_alpha`：后者非 const，这一格要留在
/// 常量出处。未直乘的白 255 + alpha 26 与 `from_white_alpha(26)` 是同一个色。
pub const HOVER_TINT: Color32 = Color32::from_rgba_unmultiplied_const(255, 255, 255, 26);

/// 悬停那一颗的上浮量（点，REQ-009 / Q-005）。**只改画不改命中**：命中矩形仍是未上浮
/// 的原槽位，否则鼠标追着一颗会跳的按钮点。
pub const HOVER_LIFT: f32 = 1.0;

/// 悬停淡底与药丸本体的圆角（点）。观感归 W-7.1 / W-7.4，改判走 revise。
pub const ROUNDING: f32 = 8.0;

/// 文字那颗的三种墨色。**药丸本体是深色那一档**（与遮罩那条已批准的药丸同一底色，浮在任何
/// 桌面上都读得清；[HOVER_TINT] 那"一成白"要叠在深色上才看得出来是"变淡"）。所以这里**不**跟
/// 应用深浅色主题走——它是浮在桌面上的独立物体，底色与遮罩那条同源。禁用态退一档灰。
pub const TEXT_REST: Color32 = Color32::from_rgb(0xC8, 0xCA, 0xCF);
pub const TEXT_HOVER: Color32 = Color32::from_rgb(0xFF, 0xFF, 0xFF);
pub const TEXT_DISABLED: Color32 = Color32::from_rgb(0x6E, 0x71, 0x76);

/// 药丸本体与悬停那颗共用的一道淡影（Q-002 / REQ-009）。**用 painter 画，不开整窗
/// `with_has_shadow`**：那是整扇窗的窗影，会把图片本体也套一圈，不是药丸的影。走与
/// `theme::pane_shadow` 同一条软边通道（`Shadow::as_shape` → `RectShape { blur_width }`），
/// 但更淡——这是浮在桌面小物件的影，不是正文框那一圈。阴影只此一处。
pub fn paint_shadow(painter: &egui::Painter, rect: Rect, rounding: impl Into<egui::CornerRadius>) {
    let shadow = egui::Shadow {
        offset: [0, 1],
        blur: 4,
        spread: 0,
        color: Color32::from_black_alpha(28),
    };
    painter.add(shadow.as_shape(rect, rounding));
}

/// 药丸放哪：**优先锚点下方，放不下翻到上方**（与遮罩那条同一套判据）。
///
/// `field` 是"那块屏的矩形"——本来 `selection.rs` 那份只读 `field.bounds.max.y`，所以这里
/// 直接收一个 [`Rect`]，贴图这边不必为了一个用不上的 `px` 字段去造一份假的 `Field`。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BarSide {
    Below,
    Above,
}

pub fn side(anchor: Rect, bar: Vec2, field: Rect) -> BarSide {
    let below = anchor.max.y + GAP + bar.y;
    if below <= field.max.y {
        BarSide::Below
    } else {
        BarSide::Above
    }
}

/// 药丸那一块的矩形。**任何一侧都不许压住锚点**——中线对齐，下方 = `max.y + GAP`、
/// 上方 = `min.y - GAP - bar.y`。锚点很小（药丸比锚点宽）时仍以中线对齐，两侧探出去
/// 的部分照样不压锚点（它本来就浮在桌面上）。
pub fn rect(anchor: Rect, bar: Vec2, side: BarSide) -> Rect {
    let center_x = (anchor.min.x + anchor.max.x) / 2.0;
    let min = pos2(
        center_x - bar.x / 2.0,
        match side {
            BarSide::Below => anchor.max.y + GAP,
            BarSide::Above => anchor.min.y - GAP - bar.y,
        },
    );
    Rect::from_min_size(min, bar)
}

/// 按内容定宽：`n` 颗文字按钮 + 它们之间的缝 + 两端留白。三颗 = 164 点。
/// 写成 `const fn` 是为了下面那条编译期断言——谁把共用件改成"跟锚点宽走"就编译不过。
pub const fn width_for(n: usize) -> f32 {
    n as f32 * TEXT_BTN_W + (n as f32 - 1.0) * BTN_GAP + 2.0 * SIDE_PAD
}

// 三颗文字药丸必须比遮罩那条（带一句尺寸读数 + 三颗线稿）窄：浮空小条不是一条 bar。
const _: () = assert!(width_for(3) < crate::system::selection::BAR_W);

/// 贴图药丸里的一颗文字按钮（REQ-002 / REQ-009）。
///
/// **平时只画字本身**（不画底、不画框、不画边）；悬停整颗垫一层 [`HOVER_TINT`] 淡圆角底、
/// 上浮 [`HOVER_LIFT`]、再画一道 [`paint_shadow`] 淡影。**命中矩形用未上浮的原槽位**
/// （`ui.interact` 拿原 rect，上浮只作用于画），否则鼠标追着一颗会跳的按钮点。
/// 禁用态那颗不进悬停支，只画灰字。字用 egui 文本绘制（不是字符字形当图标），不存在
/// D-5 那种"拿字符当图标出方块"的问题。
pub fn text_button(ui: &mut egui::Ui, label: &str) -> egui::Response {
    let height = ui.available_height();
    let (id, rect) = ui.allocate_space(egui::vec2(TEXT_BTN_W, height));
    let response = ui.interact(rect, id, egui::Sense::click());
    let enabled = response.enabled();
    let hovered = enabled && response.hovered();
    // 上浮只作用于画：命中拿的是上面那个未上浮的 rect
    let draw = if hovered {
        rect.translate(egui::vec2(0.0, -HOVER_LIFT))
    } else {
        rect
    };
    let painter = ui.painter();
    if hovered {
        paint_shadow(painter, draw, ROUNDING);
        painter.rect_filled(draw, ROUNDING, HOVER_TINT);
    }
    let color = if !enabled {
        TEXT_DISABLED
    } else if hovered {
        TEXT_HOVER
    } else {
        TEXT_REST
    };
    let font = egui::TextStyle::Button.resolve(ui.style());
    painter.text(
        draw.center(),
        egui::Align2::CENTER_CENTER,
        label,
        font,
        color,
    );
    response
}

#[cfg(test)]
mod tests {
    use super::*;
    use egui::vec2;

    fn field() -> Rect {
        Rect::from_min_max(pos2(0.0, 0.0), pos2(1920.0, 1080.0))
    }

    #[test]
    fn the_gap_is_the_three_points_he_gave() {
        assert_eq!(GAP, 3.0);
    }

    #[test]
    fn three_text_buttons_are_164_points_wide() {
        assert_eq!(width_for(3), 164.0);
        assert!(width_for(3) < crate::system::selection::BAR_W);
    }

    /// 宽度按内容定，**不随锚点宽变**：窄图与宽图配同一颗药丸。
    #[test]
    fn the_width_does_not_follow_the_anchor() {
        let bar = vec2(width_for(3), 34.0);
        let narrow = Rect::from_min_max(pos2(100.0, 100.0), pos2(140.0, 140.0));
        let wide = Rect::from_min_max(pos2(100.0, 100.0), pos2(900.0, 500.0));
        assert_eq!(rect(narrow, bar, BarSide::Below).width(), width_for(3));
        assert_eq!(rect(wide, bar, BarSide::Below).width(), width_for(3));
    }

    #[test]
    fn below_sits_three_points_under_and_never_touches_the_anchor() {
        let bar = vec2(width_for(3), 34.0);
        let anchor = Rect::from_min_max(pos2(100.0, 100.0), pos2(400.0, 200.0));
        assert_eq!(side(anchor, bar, field()), BarSide::Below);
        let r = rect(anchor, bar, BarSide::Below);
        assert_eq!(r.min.y, anchor.max.y + GAP);
        assert!(!r.intersects(anchor));
    }

    #[test]
    fn it_flips_above_when_below_would_not_fit_and_still_clears_the_anchor() {
        let bar = vec2(width_for(3), 34.0);
        let low = Rect::from_min_max(pos2(100.0, 1000.0), pos2(400.0, 1070.0));
        assert_eq!(side(low, bar, field()), BarSide::Above);
        let r = rect(low, bar, BarSide::Above);
        assert_eq!(r.max.y, low.min.y - GAP);
        assert!(!r.intersects(low));
    }

    /// 锚点很小（药丸比锚点宽）时仍以中线对齐，两侧探出去也不压锚点。
    #[test]
    fn a_tiny_anchor_still_centers_the_pill() {
        let bar = vec2(width_for(3), 34.0);
        let tiny = Rect::from_min_max(pos2(500.0, 300.0), pos2(520.0, 320.0));
        let r = rect(tiny, bar, BarSide::Below);
        assert_eq!(r.center().x, tiny.center().x);
        assert!(r.min.x < tiny.min.x && r.max.x > tiny.max.x, "两侧探出锚点");
        assert!(!r.intersects(tiny));
    }

    /// 跑一帧并把纹理增量吃掉（同 pin.rs 那条：不消费会在 drop 时 panic）。
    fn pass(ctx: &egui::Context, draw: impl FnMut(&mut egui::Ui)) {
        ctx.run_ui(egui::RawInput::default(), draw)
            .drop_without_applying_deltas();
    }

    /// 命中矩形是**未上浮的原槽位**：宽恒为 TEXT_BTN_W，与 HOVER_LIFT 取多少无关。
    /// 把 lift 改成 0 或 5，命中矩形都不动（浮起是观感，不是命中区移动）。
    #[test]
    fn the_hit_rect_is_the_un_lifted_slot() {
        let ctx = egui::Context::default();
        let mut hit = None;
        pass(&ctx, |ui| {
            ui.allocate_ui_with_layout(
                vec2(width_for(3), 34.0),
                egui::Layout::left_to_right(egui::Align::Center),
                |ui| {
                    let r = text_button(ui, "翻译");
                    hit = Some(r.rect);
                },
            );
        });
        let hit = hit.expect("画了一颗");
        assert_eq!(hit.width(), TEXT_BTN_W);
        assert_eq!(hit.height(), 34.0, "行高给多少那颗就占多少，不是上浮后的偏移");
    }

    /// 形状核查：平时那支不画底/框，悬停支读那三个常量各一次、命中与上浮分离。
    #[test]
    fn the_hover_branch_reads_the_three_constants_once_each() {
        let body = crate::system::implementation_lines(include_str!("bar.rs"));
        assert_eq!(body.matches("HOVER_TINT").count(), 2, "常量定义一处 + 悬停支用一处");
        assert_eq!(body.matches("HOVER_LIFT").count(), 2, "常量定义一处 + 悬停支用一处");
        assert!(body.contains("paint_shadow(painter, draw"), "悬停支调那一道共用影");
        assert!(
            body.contains("ui.interact(rect, id"),
            "命中拿的是未上浮的原 rect"
        );
        // 平时那支只画字：rect_filled 只在悬停支里出现一次
        assert_eq!(body.matches("rect_filled").count(), 1);
    }
}