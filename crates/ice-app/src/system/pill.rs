//! 贴图下方那扇**浮空药丸窗**（CHG-011 REQ-001 / REQ-003 / REQ-004 / REQ-005）。
//!
//! 为什么是**另一扇窗**而不是把贴图窗改透明（design 关键决定 1）：egui 0.37 没有逐像素命中，
//! `mouse_passthrough` 是整窗开关——同一扇窗改透明会在药丸两侧留下"看得见背后却点不着"的
//! 隐形陷阱（探针事实 14 实测透明区按整扇窗矩形吃点击）。另起一扇只比药丸本体大一圈阴影
//! 留白的窗，把死区压到最小，且那一圈紧贴一个看得见的物体。
//!
//! 三条硬规矩（从事实里出来，不是猜）：
//! 1. **位置命令只在这一处发**（事实 18：`pin.rs` 的形状核查明令 `OuterPosition` 出现 0 次）。
//!    药丸位置每帧从**所属贴图的 `outer_rect` 现推**，本模块不攒第二份坐标——攒了就回到
//!    CHG-010 那一轮"左屏拖右屏跳框"那个病。
//! 2. **不绕主窗**：位置由贴图窗自己的子回调直推（`send_viewport_cmd_to`），不走"子窗算→
//!    主窗存→下一帧传回"那条环（那条环必然一闪一闪，[[macos-carbon-hotkey-and-eframe-wake]]）。
//! 3. **命令只在真变了那一刻发**：每帧发就是每帧 `request_repaint_of` 把药丸窗叫醒，那是
//!    D-31~D-35 那一族（一次性原生动作改成每帧做之前必须重量代价）的 egui 版。缓存的是
//!    "上次发出去的值"，不是第二份位置权威。

use crate::system::{bar, pin, wake_root};
use egui::{Id, Rect, Vec2, pos2};

/// 药丸本体的高度（点）。与遮罩那条同一档（34），三颗文字按钮各占这一行。
pub const PILL_H: f32 = 34.0;

/// 药丸四周留给 painter 阴影的空白（点）。外接框 = 药丸本体四周各留这一格；
/// **这是死区的唯一来源**（design REQ-001 死区量级那条要写进 docs/05 的就是它）。
pub const SHADOW_PAD: f32 = 6.0;

/// 药丸那一扇视口的号——**唯一构造点**。贴图窗与药丸窗是两个不同的 id，esc 转交、
/// 位置命令都按这个号找窗。
pub fn viewport_id(pin_id: u64) -> egui::ViewportId {
    egui::ViewportId::from_hash_of(Id::NULL.with("ice-translator.pill").with(pin_id))
}

/// 这扇药丸原生窗的标题：带号，好让 `system::attach` 按标题把它和**它自己的**贴图配成对。
pub fn window_title(pin_id: u64) -> String {
    format!("贴图操作栏 {pin_id}")
}

/// 上一帧发出去的药丸窗左上角（点）。缓存"发出去的值"用于去重，不是位置权威。
fn pos_key(pin_id: u64) -> Id {
    Id::NULL.with("ice-translator.pill.pos").with(pin_id)
}

/// 这扇药丸**已被 macOS 绑成贴图窗的子窗没有**。绑上之后位置由系统按固定偏移带着走，
/// 我们就不再每帧推位置（再推会和系统抢）。带子收起/重开时这格清掉，药丸窗重建后重新绑。
fn attached_key(pin_id: u64) -> Id {
    Id::NULL.with("ice-translator.pill.attached").with(pin_id)
}

pub fn is_attached(ctx: &egui::Context, pin_id: u64) -> bool {
    ctx.data(|d| d.get_temp::<bool>(attached_key(pin_id)) == Some(true))
}

pub fn set_attached(ctx: &egui::Context, pin_id: u64) {
    ctx.data_mut(|d| d.insert_temp(attached_key(pin_id), true));
}

pub fn clear_attached(ctx: &egui::Context, pin_id: u64) {
    ctx.data_mut(|d| d.remove::<bool>(attached_key(pin_id)));
}

/// 主窗每帧写一次、子回调每帧读一次的可用区缓存（避免在 deferred 回调里逐帧走 NSScreen）。
fn screen_key() -> Id {
    Id::NULL.with("ice-translator.pill.screen")
}

/// 药丸本体的点尺寸：三颗文字按钮按内容定的宽 × 行高。
pub fn body_size() -> Vec2 {
    Vec2::new(bar::width_for(3), PILL_H)
}

/// 药丸窗的内容尺寸 = 本体 + 四周阴影留白。
pub fn window_size() -> Vec2 {
    let body = body_size();
    Vec2::new(body.x + 2.0 * SHADOW_PAD, body.y + 2.0 * SHADOW_PAD)
}

/// 药丸本体在**它自己那扇窗**里占的那块（窗原点起算，四周各让出 [`SHADOW_PAD`]）。
/// 绘制三颗按钮与那道影都相对这一块。
pub fn body_in_window() -> Rect {
    Rect::from_min_size(pos2(SHADOW_PAD, SHADOW_PAD), body_size())
}

/// 这一帧要不要给药丸窗发"挪位置"命令。**只在真的挪了 ≥0.5 点那一刻发一次**
/// （形状同 [`pin::inner_size_command`]：0.4 点的抖动不发，每帧发会把窗反复叫醒）。
pub fn position_command(previous: Option<egui::Pos2>, now: egui::Pos2) -> Option<egui::Pos2> {
    match previous {
        Some(prev) if prev == now => None,
        Some(prev) if (prev - now).length() < 0.5 => None,
        _ => Some(now),
    }
}

/// 从贴图的 `outer_rect` 现推药丸**窗**左上角（屏幕坐标）。
///
/// 纯函数，无头可断言：贴图窗（收起带子后）就是图片那一块，所以 `outer_rect` 即锚点；
/// 上下读共用件 `bar::side`（与遮罩同一套翻转判据），默认在图片**下方 3pt**、放不下翻**上方**；
/// 水平这维贴图这颗**靠右对齐**（药丸右边缘贴图片右边缘），不居中（居中是遮罩那颗的样子，两者分开）。
/// 窗左上角 = 药丸本体左上角再让出一格阴影留白。**只读 `outer_rect` 的几何、只调纯函数**，
/// 不许出现坐标字段读写（design REQ-004 把这条写成形状核查）。
pub fn pill_window_pos(outer_rect: Rect, screen: Rect) -> egui::Pos2 {
    let body = body_size();
    let side = bar::side(outer_rect, body, screen);
    let top = match side {
        bar::BarSide::Below => outer_rect.max.y + bar::GAP,
        bar::BarSide::Above => outer_rect.min.y - bar::GAP - body.y,
    };
    // 水平：贴图这颗**靠右**——药丸右边缘贴图片右边缘（某截图工具 那种操作条落图下方靠右），
    // 左边 = 图右缘 − 药丸宽。**不居中**（居中是遮罩那颗的已批准样子，两者分开，`bar::rect` 不改）。
    let left = outer_rect.max.x - body.x;
    pos2(left - SHADOW_PAD, top - SHADOW_PAD)
}

/// 贴图窗子回调每帧调它：把自己所在的 `outer_rect` 换算成药丸位置，**只在真变了那一刻**
/// 发给药丸窗那一扇（不绕主窗）。`screen` 是主窗这一帧缓存的可用区。
pub fn follow(root: &egui::Context, pin_id: u64, outer_rect: Rect, screen: Rect) {
    let now = pill_window_pos(outer_rect, screen);
    let previous = root.data(|d| d.get_temp::<egui::Pos2>(pos_key(pin_id)));
    if let Some(pos) = position_command(previous, now) {
        root.data_mut(|d| d.insert_temp(pos_key(pin_id), pos));
        root.send_viewport_cmd_to(
            viewport_id(pin_id),
            egui::ViewportCommand::OuterPosition(pos),
        );
    } else if previous.is_none() {
        // 第一次：记下发出去之前的期望值，即使窗还没建也不留"没人发过"的空档
        root.data_mut(|d| d.insert_temp(pos_key(pin_id), now));
    }
}

/// 主窗每帧写一次可用区，供各贴图子回调读取（免逐帧走 NSScreen）。
pub fn cache_screen(root: &egui::Context, screen: Rect) {
    root.data_mut(|d| d.insert_temp(screen_key(), screen));
}

/// 读主窗这一帧缓存的可用区；还没写过就返回 `None`（贴图回调这一帧不发位置命令，下一帧补上）。
pub fn screen_of(root: &egui::Context) -> Option<Rect> {
    root.data(|d| d.get_temp::<Rect>(screen_key()))
}

/// 药丸本体的底色：与遮罩那条**已批准的药丸同一档**（深色、约九分不透明）。浮在任何桌面上
/// 都读得清，[bar::HOVER_TINT] 那"一成白"叠在它上面才看得出是"变淡"。不跟应用主题走。
pub const PILL_BG: egui::Color32 = egui::Color32::from_rgba_premultiplied(28, 28, 30, 235);

/// 把这一帧这张药丸画出来并跟随。**每帧必须调**（`show_viewport_deferred` 不调那一帧起
/// 窗就被 egui 删掉，与 `pin::show` 同一条纪律）；收起带子那一帧起调用方不再调它，
/// 窗随之消失，不需要单独发关闭命令。
///
/// `line` 是这一帧这张的进度话（"识别中…" / "翻译中 3/7" / 按 `C` 会给什么），由帧循环从
/// 状态算好传进来——药丸不读 `AppState`，也不留一份全局可变的"当前进度"（六张各有各的）。
/// 它挂在整个药丸的悬停上，不再占宽度（REQ-002 / Q-003）。
pub fn show(ctx: &egui::Context, pin_id: u64, line: Option<String>) {
    let viewport = viewport_id(pin_id);
    let mut builder = egui::ViewportBuilder::default()
        .with_inner_size(window_size())
        .with_decorations(false)
        .with_transparent(true)
        // 事实 17：macOS 上透明窗必须关整窗窗影，否则 ghosting 残影；影改用 painter 画（Q-002）
        .with_has_shadow(false)
        .with_always_on_top()
        // 建出来不抢焦点：抢了会把用户正在打字那扇应用挤掉，且每钉一张抢一次（事实 17）
        .with_active(false)
        .with_title(window_title(pin_id));
    // 首帧创建时用缓存里的位置落位；之后位置只从 follow 来（builder 的位置改动 egui 不追）
    if let Some(at) = ctx.data(|d| d.get_temp::<egui::Pos2>(pos_key(pin_id))) {
        builder = builder.with_position(at);
    }

    let root = ctx.clone();
    ctx.show_viewport_deferred(viewport, builder, move |ui, _class| {
        let child = ui.ctx().clone();
        // esc 从药丸窗转交：**只取 esc 那一支**（空格与 C 仍归贴图窗，避免两扇窗各收一份）。
        // 推进所属那张贴图的槽，再点名叫醒主窗那一帧来取（子窗闭包里 request_repaint 只叫得醒自己）。
        let fired: Vec<pin::PinIntent> = child.input(|i| {
            i.events
                .iter()
                .filter_map(|e| match e {
                    egui::Event::Key {
                        key,
                        pressed: true,
                        repeat: false,
                        modifiers,
                        ..
                    } => match pin::key_intent(*key, *modifiers) {
                        Some(pin::PinIntent::Close) => Some(pin::PinIntent::Close),
                        _ => None,
                    },
                    _ => None,
                })
                .collect()
        });
        if !fired.is_empty() {
            pin::submit_intents(&root, pin_id, fired);
            wake_root(&root);
        }

        // 本体那一块：先铺那道淡影（Q-002，画在底色下缘露出一点），再涂药丸底色，再画那一排。
        // 没画的地方全透明（T-005 三格），圆角外与四周阴影留白就是那圈唯一的死区。
        let body = body_in_window();
        let painter = ui.painter();
        bar::paint_shadow(painter, body, bar::ROUNDING);
        painter.rect_filled(body, bar::ROUNDING, PILL_BG);
        // 整块药丸的悬停挂这句进度话（REQ-002 把它挪出宽度）。Sense::hover 只看不吃点击，
        // 点击仍归那一排里的某一颗——两者叠在同一块上不抢命中。
        let hover = ui.interact(
            body,
            Id::NULL.with("ice-pill.hover").with(pin_id),
            egui::Sense::hover(),
        );
        if let Some(text) = &line {
            hover.on_hover_text(text.clone());
        }
        // 那一排三颗用 Area 摆在 body 里（与遮罩那条同一手法：占绝对位置，不占窗口布局）。
        // 左右各让出 SIDE_PAD、颗间 BTN_GAP——这样摆出来的正好是 width_for(3) 那块宽。
        let mut local: Vec<pin::PinIntent> = Vec::new();
        egui::Area::new(Id::NULL.with("ice-pill.buttons").with(pin_id))
            .fixed_pos(body.min + egui::vec2(bar::SIDE_PAD, 0.0))
            .order(egui::Order::Middle)
            .show(ui.ctx(), |ui| {
                ui.spacing_mut().item_spacing.x = bar::BTN_GAP;
                ui.allocate_ui_with_layout(
                    Vec2::new(bar::width_for(3) - 2.0 * bar::SIDE_PAD, PILL_H),
                    egui::Layout::left_to_right(egui::Align::Center),
                    |ui| pin::pill_buttons(ui, &mut local),
                );
            });
        if !local.is_empty() {
            pin::submit_intents(&root, pin_id, local);
            wake_root(&root);
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use egui::Rect;

    fn screen() -> Rect {
        Rect::from_min_max(pos2(0.0, 0.0), pos2(1920.0, 1080.0))
    }

    #[test]
    fn the_window_is_the_body_plus_one_shadow_margin_all_around() {
        let w = window_size();
        let b = body_size();
        assert_eq!(w.x, b.x + 2.0 * SHADOW_PAD);
        assert_eq!(w.y, b.y + 2.0 * SHADOW_PAD);
        assert_eq!(b.x, bar::width_for(3));
        assert_eq!(body_in_window().min, pos2(SHADOW_PAD, SHADOW_PAD));
    }

    /// 位置命令只在真变了那一刻发：0.4 点抖动不发、恰好相等不发、挪 1 点才发一次。
    #[test]
    fn the_position_command_fires_only_on_a_real_move() {
        assert_eq!(position_command(Some(pos2(100.0, 200.0)), pos2(100.0, 200.0)), None);
        assert_eq!(
            position_command(Some(pos2(100.0, 200.0)), pos2(100.4, 200.0)),
            None,
            "0.4 点的抖动不该发一次移动"
        );
        assert_eq!(
            position_command(Some(pos2(100.0, 200.0)), pos2(100.0, 210.0)),
            Some(pos2(100.0, 210.0))
        );
        assert_eq!(position_command(None, pos2(100.0, 200.0)), Some(pos2(100.0, 200.0)));
    }

    /// 药丸默认在图片下方、离图 3pt、**右缘贴图片右缘**；翻到上方时以图片上缘为基准；窗左上角再让出阴影留白。
    #[test]
    fn the_pill_hangs_below_right_aligned_and_flips_above() {
        // 图片在屏幕中部：药丸在下、靠右
        let mid = Rect::from_min_max(pos2(300.0, 200.0), pos2(812.0, 560.0));
        let pos = pill_window_pos(mid, screen());
        assert_eq!(
            pos.x + SHADOW_PAD + bar::width_for(3),
            mid.max.x,
            "药丸右缘贴图片右缘（靠右，不居中）"
        );
        assert_eq!(pos.y + SHADOW_PAD, mid.max.y + bar::GAP, "紧贴图片下方 3pt");

        // 图片贴近屏底：放不下，翻到上方
        let low = Rect::from_min_max(pos2(300.0, 980.0), pos2(812.0, 1075.0));
        let above = pill_window_pos(low, screen());
        assert_eq!(above.y + SHADOW_PAD, low.min.y - bar::GAP - PILL_H, "翻到图片上方");
    }

    /// 形状核查（design REQ-004）：本模块 `OuterPosition` 恰好一次，且值只来自那个纯函数；
    /// `movable_by_background` 一次都不许出现。
    #[test]
    fn position_is_sent_from_exactly_one_place_and_only_from_the_pure_fn() {
        let body = crate::system::implementation_lines(include_str!("pill.rs"));
        assert_eq!(
            body.matches("OuterPosition").count(),
            1,
            "位置命令只能有一处（其余都不许往窗发位置）"
        );
        assert_eq!(
            body.matches("movable_by_background").count(),
            0,
            "药丸窗不拖，它跟着贴图走"
        );
        // 那一处发的是 follow 里现推的值：值来自 pill_window_pos，不自己读写字段
        assert!(body.contains("egui::ViewportCommand::OuterPosition(pos)"));
    }
}
