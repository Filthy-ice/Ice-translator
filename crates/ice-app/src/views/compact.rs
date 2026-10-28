//! 简洁悬浮窗（FR-3.2、REQ-002 可拖可缩、REQ-005 与全量窗同源）。
//!
//! 自上而下三块：输入带 → 动作行（底）→ 译文带，两条带各包一个圆角框、带间留 `BAND_GAP`。
//!
//! **两带的高度是 1:1 分摊出来的，不再随内容长高**（用户 2026-10-27 第二轮走查，REQ-012）：
//! 上一版"输入行按内容长到三行封顶"把译文区挤成一条缝，用户看到的是"译文只露半行"。
//! 现在窗口越高两带一起变高，内容装不下的那部分各自在框内滚。
//!
//! **顶部把手条已经删掉**（用户 2026-10-27，Q-013 之后）：窗口带原生标题栏，抓那条边
//! 比抓窗内 10px 的面板更直觉，窗内再画一根"小横杠"纯属重复。留下的两块把手是**看不见**的
//! 空白区（动作行行尾、译文正文之下 8px），它们只是补充：正文与按钮都不该被当成把手，
//! 而停靠态那几条边用户常常只露出几个像素。
//!
//! 把手不能靠"盖在控件上面"来做：egui 的命中测试按绘制顺序取最后一个矩形，
//! 压上去的把手会把下面的按钮一起吃掉。所以两块把手都取**互不重叠**的真空白，
//! 与 egui 自己的 `Window` 标题栏避开关闭按钮是同一个思路。
//!
//! 把手只用 `Sense::drag()` 而不是 `click_and_drag()`：后者要等指针移动超过
//! `max_click_dist`（6px）才认定是拖拽，而 REQ-002 要求"按下即发 `StartDrag`"。
//! 把手上没有点击语义，丢掉 click 不损失任何东西。
//!
//! 原生标题栏那条拖动路径不产生 `StartDrag`（AppKit 自己搬窗口），吸附由
//! `app::settle_drag` 观测几何跳变补上，不依赖这里的把手。

use egui::{
    self, Align, CentralPanel, CursorIcon, Key, KeyboardShortcut, Layout, Modifiers, Pos2, Rect,
    RichText, Sense, Vec2,
};

use crate::state::AppState;
use crate::system::clipboard::CopyKind;
use crate::system::hotkeys::SOURCE_EDITOR_ID;
use crate::theme;

use super::{Effects, engine_picker, full, toasts};

/// 输入带的最小内容高度：一行 `TextEdit` 加上下内边距。
/// REQ-012 里它是"两带均分不够时"的退让值——宁可译文带多拿，也不把输入框压到一行字以下。
const INPUT_HEIGHT: f32 = 34.0;
/// 两条带之间的空隙（REQ-012：≥5px，看得出是两块独立的东西）
pub const BAND_GAP: f32 = 5.0;
/// 带框上下内边距（`Frame::group` 的 `inner_margin`，`Margin` 在 egui 0.37 是整数）
const BAND_V_PAD: f32 = 4.0;
/// 一条带的外框比内容多出来的高度：`BAND_V_PAD` + 1px 描边。
///
/// 实测外框高 = 内容高 + 10（两带都是这个数），`band_heights` 必须按 10 而不是按 8 扣：
/// 少扣这两像素，两条带各多出一像素，叠到最矮那一档就够让译文带底边压过动作行顶边。
const BAND_OUTER_PAD: f32 = BAND_V_PAD + 1.0;
/// 译文区底部把手带高度。
/// 输入行长到三行时这一带会被挤没，`MIN_BODY_HEIGHT` 那条守卫兜着。
const TAIL_HEIGHT: f32 = 8.0;
/// 正文可视高度低于这个值时不画底部把手（窗口被压到极限时优先保译文）
const MIN_BODY_HEIGHT: f32 = 20.0;
/// 动作行空隙把手与按钮组之间留的距离，避免指针落在按钮边缘时误判成拖窗
const GAP_PADDING: f32 = 4.0;

/// 译文区滚动容器的 id（`full::scroll_area` 按名字区分两栏）
const TRANSLATION_PANE: &str = "compact_translation";

/// 输入框滚动容器的 id（`&'static str` 是给 `full::record_scroll` 用的）
const SOURCE_PANE: &str = "compact_source_pane";

/// 提示条与输入行底边的间距（REQ-005：不遮住正在输入的字）
const TOAST_GAP: f32 = 4.0;

pub fn show(
    ui: &mut egui::Ui,
    state: &mut AppState,
    effects: &mut Effects,
    ai_available: bool,
    dim: f32,
) {
    apply_dim(ui, dim);
    let ctx = ui.ctx().clone();
    // 动作行先建：`Panel::bottom` 自下而上占位，中央区拿到的可用高**就是**扣掉动作行之后的数。
    // 两带的分摊因此只看这一个实测值（REQ-012），不用再猜行高。
    action_row(ui, state, effects, ai_available);
    CentralPanel::default()
        .frame(egui::Frame::central_panel(ui.style()).inner_margin(0.0))
        .show(ui, |ui| {
            let (input_band, body_band) = band_heights(ui.available_height());
            input_row(ui, state, input_band);
            translation_pane(ui, state, effects, body_band);
        });

    toasts(&ctx, state, toast_offset(&ctx));
}

/// 两带的高度分摊（REQ-012）。纯函数：中央区（已扣掉动作行）的可用高进，两条带的**内容**高出。
///
/// 先给 1:1；`room` 小到两个半份撑不起"输入带 ≥ `INPUT_HEIGHT` 且译文带 ≥ `MIN_BODY_HEIGHT`"
/// 时**优先保译文带**（用户第二轮走查要解决的正是"译文只露半行"），输入带退回 `INPUT_HEIGHT` 那一档。
///
/// 减掉的是 2 个 `BAND_GAP`（一个画在两带之间，一个留在译文带与动作行之间当余气）和
/// 4 个 `BAND_OUTER_PAD`（两条带各自框比内容多出来的内边距 + 描边）。两条带都画在同一个
/// `Ui` 里，所以这个式子给出的就是**实际画出来的**比例，谁也不会多占一像素。
pub fn band_heights(available_h: f32) -> (f32, f32) {
    let room = (available_h - 2.0 * BAND_GAP - 4.0 * BAND_OUTER_PAD).max(0.0);
    let half = room / 2.0;
    if half >= INPUT_HEIGHT.max(MIN_BODY_HEIGHT) {
        return (half, half);
    }
    let input = INPUT_HEIGHT.min(room);
    (input, (room - input).max(0.0))
}

/// 提示条离窗口顶部的距离：越过输入带。带高每帧由 `band_heights` 决定，
/// 这个距离只能读它这一帧真正占到的底边，写死会把文字压在提示条下面。
fn toast_offset(ctx: &egui::Context) -> f32 {
    ctx.data_mut(|d| d.get_temp::<Rect>(rect_id("input")))
        .map(|band| band.max.y + TOAST_GAP)
        .unwrap_or(INPUT_HEIGHT + TOAST_GAP)
}

/// 失焦时把"构成窗体的那几层底色"一起降透明（REQ-007）。
///
/// 只改底色不改文字：观感要的是"整块窗体透出桌面"，而不是内容糊掉。窗口 surface 在创建时
/// 就带了 `with_transparent(true)`（`mode::initial_viewport`），这里填的 alpha 才有意义——
/// surface 不透明时再怎么乘 alpha 也透不出来。
///
/// **这是回退路径**。macOS 上帧循环走的是原生整窗 `NSWindow.alphaValue`（`system::alpha`），
/// 那条能让标题栏与红绿灯一起变淡（用户走查点名的就是"标题栏为什么不能跟着失焦半透明"），
/// 且已经生效时这里收到的是 1.0，即什么都不做——两条路径叠加会淡成 `want²`。
/// 只有拿不到原生窗口（非 macOS、无头）才由这条顶上。
fn apply_dim(ui: &mut egui::Ui, alpha: f32) {
    if alpha >= 1.0 {
        return;
    }
    let visuals = ui.visuals_mut();
    for fill in [
        &mut visuals.panel_fill,
        &mut visuals.window_fill,
        &mut visuals.extreme_bg_color,
        &mut visuals.widgets.noninteractive.bg_fill,
        &mut visuals.widgets.inactive.bg_fill,
    ] {
        *fill = fill.gamma_multiply(alpha);
    }
}

/// 把控件矩形记在临时数据里，供无头布局测试断言。
///
/// 看不见像素的测试只能靠这些数判断"控件有没有溢出窗口""把手有没有压住按钮"——
/// P4 的 D-1/D-3 那批缺陷正是版面越界而测试无从发现。
fn record(ctx: &egui::Context, salt: &str, rect: Rect) {
    ctx.data_mut(|d| d.insert_temp(rect_id(salt), rect));
}

fn rect_id(salt: &str) -> egui::Id {
    egui::Id::NULL.with("ice-translator.compact").with(salt)
}

/// 一块空白当拖拽把手：按下即请求原生窗口拖拽，指针悬停时变成抓手。
fn drag_handle(ui: &egui::Ui, salt: &str, rect: Rect, effects: &mut Effects) {
    if rect.is_negative() || rect.width() < 1.0 || rect.height() < 1.0 {
        return;
    }
    let response = ui
        .interact(rect, egui::Id::unique(salt), Sense::drag())
        .on_hover_cursor(CursorIcon::Grab);
    if response.drag_started() {
        effects.start_drag = true;
    }
}

/// 输入带：定高的多行编辑，高度由 `band_heights` 给（REQ-012），内容超出就在带内滚。
///
/// **三条实现前提一条都不能少**（都是实测坑，与高度上限无关）：
/// `TextEdit::multiline` 的默认最小高度是 4 行，不压 `desired_rows(1)` 就缩不回矮窗；
/// 外层带必须 `auto_shrink([false,false])`，`[_, true]` 会让内层收缩、把带子压回 25px；
/// `ScrollArea` 的 `min_scrolled_size` 默认 64px 比矮窗那一半还高，不显式
/// `min_scrolled_height(行高)`，`max_height` 直接被顶穿（实测封顶成了 64px）。
///
/// 键约定不变：裸 Enter 仍归"翻译"（`hotkeys::classify` 在绘制前判掉），
/// Shift+Enter 才是换行——多行 `TextEdit` 里 `return_key` 命中的那一下由 egui 自己
/// 插入 `"\n"`，未命中的 Enter 它不理会，所以两条都不需要改正本快捷键。
fn input_row(ui: &mut egui::Ui, state: &mut AppState, band: f32) {
    // 与 `TextEdit` 自己算行高用的同一条公式（`row_height + extra_text_line_spacing`）
    let line = ui.text_style_height(&egui::TextStyle::Body) + ui.spacing().extra_text_line_spacing;
    let ctx = ui.ctx().clone();

    let framed = band_frame(ui, |ui| {
        ui.allocate_ui(Vec2::new(ui.available_width(), band), |ui| {
            let output = egui::ScrollArea::vertical()
                .id_salt(SOURCE_PANE)
                .auto_shrink([false, false])
                .min_scrolled_height(line)
                .max_height(band)
                .show(ui, |ui| {
                    egui::TextEdit::multiline(&mut state.source)
                        .id(egui::Id::unique(SOURCE_EDITOR_ID))
                        .frame(egui::Frame::NONE)
                        .desired_rows(1)
                        .desired_width(f32::INFINITY)
                        .hint_text("输入或粘贴，Enter 翻译，Esc 清空")
                        .return_key(KeyboardShortcut::new(Modifiers::SHIFT, Key::Enter))
                        .show(ui)
                });
            full::record_scroll(&ctx, SOURCE_PANE, &output);
            // 快捷键在绘制之前判定，这里记下的是给下一帧用的事实
            state.focus_in_source = output.inner.response.has_focus();
            state.sync_char_count();
        });
    });
    // 提示条越过的是输入带的底边（不含空隙）
    record(&ctx, "input", framed.response.rect);
    // 带与带之间的空隙：`band_heights` 那 2 个 `BAND_GAP` 里的一个从这里画出去
    ui.add_space(BAND_GAP);
    record(&ctx, "input_band", framed.response.rect);
}

/// 一条带的圆角外框：与全量窗的源文栏同一套 `Frame::group`（圆角 + 描边），
/// 左右各留 4px 让描边不贴窗边，上下 `BAND_V_PAD` 内边距加 1px 描边（`band_heights` 按
/// `BAND_OUTER_PAD` 扣过 4 份）。
///
/// `Margin` 在 egui 0.37 是整数，`BAND_V_PAD` 那 4.0 就是这里的 `4`，两处要一起改。
fn band_frame<R>(
    ui: &mut egui::Ui,
    add: impl FnOnce(&mut egui::Ui) -> R,
) -> egui::InnerResponse<R> {
    egui::Frame::group(ui.style())
        .inner_margin(egui::Margin::symmetric(8, BAND_V_PAD as i8))
        .corner_radius(egui::CornerRadius::same(6))
        .outer_margin(egui::Margin::symmetric(4, 0))
        .show(ui, add)
}

/// 主题切换框的宽度。`13px` 字号下"两个汉字 + 左右内边距"正好是这个数，于是三态
/// （太阳 / 月亮 / 「自动」）占位一致——点到哪一档，右边那一排按钮都不会左右跳。
/// 字号缩放时带文字的档位会更宽一些，`min_size` 只保证不会更窄。
const THEME_TOGGLE_W: f32 = 42.0;

/// 一段圆弧采样成折线（epaint 0.37 没有圆弧图元，只能自己按角度取点）
fn arc_points(center: Pos2, radius: f32, from: f32, to: f32, steps: usize) -> Vec<Pos2> {
    (0..=steps)
        .map(|i| {
            let angle = from + (to - from) * i as f32 / steps as f32;
            center + Vec2::new(angle.cos(), angle.sin()) * radius
        })
        .collect()
}

/// 太阳：一圈描边的圆 + 八道光芒。全是线稿，与月亮同一套笔画粗细。
///
/// 不用 `☀` 这类字符：egui 自带字体里没有这些字形，真机上落成豆腐块
/// （P4 缺陷 D-5 就是装饰字符 `⌗` 把界面画花的）。
fn draw_sun(painter: &egui::Painter, at: Pos2, stroke: egui::Stroke) {
    painter.circle_stroke(at, 3.6, stroke);
    for index in 0..8 {
        let angle = std::f32::consts::TAU * index as f32 / 8.0;
        let dir = Vec2::new(angle.cos(), angle.sin());
        painter.line_segment([at + dir * 5.4, at + dir * 7.6], stroke);
    }
}

/// 月亮：月牙轮廓 = 外弧 + 内弧，两段弧共用两个尖端。
///
/// 只描边不填充：`PathShape` 的 `fill` 只支持凸多边形（`path_shape.rs:16`），月牙不是凸的，
/// 填出来会画歪。外弧取整圆的左侧 230°，内弧的圆心右移 `BIAS`、半径取到同样穿过那两个
/// 尖端，两弧之间剩下的就是月牙。
fn draw_moon(painter: &egui::Painter, at: Pos2, stroke: egui::Stroke) {
    const TIP: f32 = 65.0_f32.to_radians();
    const R: f32 = 7.0;
    const BIAS: f32 = 3.2;
    const STEPS: usize = 20;
    let inner_center = at + Vec2::new(BIAS, 0.0);
    // `+y` 朝下，所以这是月牙朝下的那个尖端；它的极角就是内弧的端点角
    let tip = at + Vec2::new(R * TIP.cos(), R * TIP.sin());
    let offset = tip - inner_center;
    let mut points = arc_points(at, R, TIP, std::f32::consts::TAU - TIP, STEPS);
    points.extend(arc_points(
        inner_center,
        offset.length(),
        std::f32::consts::TAU - offset.angle(),
        offset.angle(),
        STEPS,
    ));
    painter.add(egui::Shape::Path(egui::epaint::PathShape::closed_line(
        points, stroke,
    )));
}

/// 主题切换框（用户 2026-10-28：「左边加上 太阳/月亮 小图标切换框，标识浅色/深色模式…
/// 点击三次自动切换为自动两个字」）。三态就是 `Theme::{Light, Dark, System}`，
/// 单点顺序由 `config::Theme::next` 给出（浅色 → 深色 → 自动）；「自动」这一档
/// 按用户的要求隐去图案、只显示「自动」两个字。
///
/// 写的是 `state.config.ui.theme` + 脏标记，与设置页同一个通道：生效在 `theme::apply`
/// （`app.rs` 每帧调），view 不碰 `Context`。
fn theme_toggle(ui: &mut egui::Ui, state: &mut AppState) -> Rect {
    use ice_core::config::Theme;
    let current = state.config.ui.theme;
    let response = ui.add(
        egui::Button::new(if current == Theme::System {
            "自动"
        } else {
            ""
        })
        .min_size(Vec2::new(THEME_TOGGLE_W, 0.0)),
    );
    let response = match current {
        Theme::Light => response.on_hover_text("当前：浅色（点一下切到深色）"),
        Theme::Dark => response.on_hover_text("当前：深色（点一下切到自动）"),
        Theme::System => response.on_hover_text("当前：自动跟随系统（点一下切到浅色）"),
    };
    if current != Theme::System {
        let stroke = egui::Stroke::new(1.5, ui.visuals().widgets.inactive.fg_stroke.color);
        let painter = ui.painter_at(response.rect);
        match current {
            Theme::Light => draw_sun(&painter, response.rect.center(), stroke),
            Theme::Dark => draw_moon(&painter, response.rect.center(), stroke),
            Theme::System => {}
        }
    }
    if response.clicked() {
        state.config.ui.theme = current.next();
        state.mark_config_dirty();
    }
    response.rect
}

/// 动作行：「☀/☾/自动 / 全量 / 复制 / 设置 / 机翻|AI / 置顶 —— 退出（钉右）」，
/// 正常字号、正常按钮内边距，左排与「退出」之间剩下的真空白是第三块把手。
///
/// 「退出」为什么单独钉到行的右边缘（用户 2026-10-28："悬浮窗退出放在右下角"）：它是这一行
/// 唯一的"离开"动作，跟在 `置顶` 后面时和开关挤在一起。它在一条 `right_to_left` 子带里，
/// 子带的竖向范围取左排**实测**出来的那一格，所以行高仍由左排决定。
///
/// 高度不再 `exact_size`：这一行是自然高度，一行装得下就一行，装不下换行成两行。
/// 它是 `Panel::bottom`，占掉的高度由 egui 从中央区里扣，`band_heights` 拿到的可用高
/// 因此自动跟着行高变（REQ-001 的"缩放到任意合法尺寸三块都不被裁"）。
///
/// 也不再把字号和内边距压小（v0.4 那套三层收缩被 Q-011 推翻：用户 2026-10-23 的原话是
/// "悬浮窗也可以正常使用操作栏，没必要搞一个缩减版的操作栏，还不容易点到"）。
/// 换 `horizontal_wrapped` 而不是继续压小，技术上还有个好处：`wrapped` 布局自带
/// "永不超出可用宽度"，与字体度量无关——代价是最窄的窗里这一行可能占两行高，所以
/// 宽度下限按**度量**锁住（`the_action_row_stays_on_one_line_at_the_minimum_width`：
/// 320 那一档「退出」掉到第二行，用户 2026-10-28 要求"按钮不允许换行"，抬到 400；
/// 同日加了主题切换框后又抬到 420），高度下限抬到 160（REQ-012 两带 1:1 摊开后，
/// 130 那一档译文带连两行字都放不下）。
fn action_row(ui: &mut egui::Ui, state: &mut AppState, effects: &mut Effects, ai_available: bool) {
    let panel = egui::Panel::bottom("compact_bar").show(ui, |ui| {
        let available = ui.max_rect();
        let ctx = ui.ctx().clone();

        // 这一行"真的用了多宽"只能由控件矩形自己拼：`horizontal_wrapped` 会把整行
        // 的可用宽度分配到底，所以 `ui.min_rect()` 永远等于面板内容区，拿它算行尾空位
        // 会恒得零宽（实测 560px 宽的窗里内容只到 x=220，min_rect 却报到 552）。
        let mut left = Rect::NOTHING;
        ui.horizontal_wrapped(|ui| {
            // 行首的主题切换框（三态循环，见 `theme_toggle`）
            let theme_box = theme_toggle(ui, state);
            record(&ctx, "btn_theme", theme_box);
            left = left.union(theme_box);

            let full_mode = ui
                .button("全量")
                .on_hover_text("切回全量界面（Cmd+Shift+M）");
            record(&ctx, "btn_full", full_mode.rect);
            left = left.union(full_mode.rect);
            if full_mode.clicked() {
                effects.toggle_mode = true;
            }

            let copy = ui.button("复制").on_hover_text("复制译文（Cmd+Shift+C）");
            record(&ctx, "btn_copy", copy.rect);
            left = left.union(copy.rect);
            if copy.clicked() {
                full::copy(state, effects, CopyKind::Translation);
            }

            let settings = ui.button("设置").on_hover_text("打开设置页");
            record(&ctx, "btn_settings", settings.rect);
            left = left.union(settings.rect);
            if settings.clicked() {
                effects.open_settings = true;
            }

            // 引擎分段与全量工具条共用同一个实现（REQ-005：AI 未配置时同样禁用并给说明）。
            // 不包 `ui.horizontal`：那种子 `Ui` 会把行尾剩下的宽度整个吃掉，
            // 左排末尾就贴到面板右边、与钉右的「退出」之间不再有空白，把手没有立足之地。
            let engine = engine_picker(ui, state, effects, ai_available);
            record(&ctx, "engine", engine);
            left = left.union(engine);

            let top = ui
                .selectable_label(state.config.ui.always_on_top, "置顶")
                .on_hover_text("窗口置顶（Cmd+Shift+P）");
            record(&ctx, "btn_top", top.rect);
            left = left.union(top.rect);
            if top.clicked() {
                effects.toggle_topmost = true;
            }
        });

        // REQ-011：退出要看得见。红灯/`Cmd+W` 是收进后台（FR-5.2），不是退出，
        // 老用户在悬浮窗上找不到"到底怎么关干净"。点击走 `Action::Quit` 同一条路径。
        //
        // 与 `full::bottom_bar` 同一手法：一块 `right_to_left` 子带钉住右边缘，子带只吃
        // 左排那一格的高度，所以它不会把面板撑出第二行；两颗字也不允许被截断，故不给它
        // `truncate`。子带的左端取左排的起点，中间那段空白留给把手。
        let band = Rect::from_min_max(
            Pos2::new(left.min.x.max(available.min.x), left.min.y),
            Pos2::new(available.max.x, left.max.y),
        );
        let mut right = ui.new_child(
            egui::UiBuilder::new()
                .max_rect(band)
                .layout(Layout::right_to_left(Align::Center)),
        );
        let quit = right
            .button("退出")
            .on_hover_text("退出程序（收进后台请用红灯，Cmd+W）");
        record(&ctx, "btn_quit", quit.rect);
        if quit.clicked() {
            effects.quit = true;
        }

        let used = left.union(quit.rect);
        record(&ctx, "actions", used);
        // 把手吃的是左排与「退出」之间那段真的空白：两头各让开 4px，否则点按钮边缘
        // 会变成拖窗；挤到零宽时 `drag_handle` 自己跳过（宁少一块把手，也不压住按钮）
        let gap = Rect::from_min_max(
            Pos2::new(left.max.x + GAP_PADDING, used.min.y),
            Pos2::new(quit.rect.min.x - GAP_PADDING, used.max.y),
        );
        record(&ctx, "bar_gap", gap);
        drag_handle(ui, "compact_handle_bar", gap, effects);
    });
    // 行自己的矩形留给无头测试断言"动作行没被裁出窗口"
    record(ui.ctx(), "bar", panel.response.rect);
}

/// 译文带：`band_heights` 分到的那一半 + 底部把手带。
///
/// 正文与把手带分成两块矩形手工排（同 `full.rs::exact_row` 的做法）而不是把手画进滚动容器里：
/// 滚动容器里的任何东西都算"内容高度"，把手会把内容撑得永远比可视区高，
/// 于是滚动条常驻、滚下去却什么也没有（P4 缺陷 D-4 的同一个坑）。
///
/// 带框之外的空隙（中央区内剩的那 `BAND_GAP`）留在框下面，不画进框里：
/// REQ-012 要的是"看得出两块独立的东西"，不是框内多一段空白。
fn translation_pane(ui: &mut egui::Ui, state: &mut AppState, effects: &mut Effects, band: f32) {
    let ctx = ui.ctx().clone();
    let framed = band_frame(ui, |ui| {
        ui.allocate_ui(Vec2::new(ui.available_width(), band), |ui| {
            let pane = ui.max_rect();
            record(&ctx, "translation", pane);
            // 正文画进丢弃的子 Ui、把手只 `interact` 一块矩形，两者都不推进父级光标：
            // 不手工 `advance_cursor_after_rect`，带框的 `min_rect` 是空的，整条带会塌成
            // 26×10（实测：内层 `pane` 拿到的矩形是对的，塌的只有外框）。
            if pane.height() < MIN_BODY_HEIGHT {
                ui.advance_cursor_after_rect(pane);
                return;
            }
            let split = pane.max.y - TAIL_HEIGHT;
            let body_rect = Rect::from_min_max(pane.min, Pos2::new(pane.max.x, split));
            let tail_rect = Rect::from_min_max(Pos2::new(pane.min.x, split), pane.max);

            let mut child = ui.new_child(
                egui::UiBuilder::new()
                    .max_rect(body_rect)
                    .layout(Layout::top_down(Align::Min)),
            );
            let output = full::scroll_area(TRANSLATION_PANE).show(&mut child, |ui| {
                body(ui, state);
            });
            full::record_scroll(&ctx, TRANSLATION_PANE, &output);

            record(&ctx, "handle_tail", tail_rect);
            drag_handle(ui, "compact_handle_tail", tail_rect, effects);
            ui.advance_cursor_after_rect(pane);
        });
    });
    record(&ctx, "translation_band", framed.response.rect);
}

/// 译文正文。四种状态与全量窗一致（失败红字 / 译文可选中 / 翻译中 / 空态提示），
/// 只是不画徽标行——最小窗里正文只有几十像素高，先保译文，耗时与缓存标记回全量窗看。
///
/// 引擎名也不跟在译文后面（用户 2026-10-28："悬浮窗口时译文后面不应该有机翻两个字，在
/// 边框上就看的到"）：动作行那颗「机翻|AI」分段按钮写的就是当前引擎，同一件事不说两遍，
/// 而且那两个字还占掉译文的一行宽度。
fn body(ui: &mut egui::Ui, state: &AppState) {
    if let Some(failure) = &state.failure {
        ui.add(
            egui::Label::new(RichText::new(&failure.message).color(theme::ERROR))
                .selectable(true)
                .wrap(),
        );
        return;
    }
    if let Some(result) = &state.translation {
        ui.add(
            egui::Label::new(RichText::new(&result.text))
                .selectable(true)
                .wrap(),
        );
        return;
    }
    if state.in_flight.is_some() {
        ui.horizontal(|ui| {
            ui.spinner();
            ui.colored_label(theme::WARN, "翻译中…");
        });
        return;
    }
    let hint = if state.source.trim().is_empty() {
        "译文显示在这里"
    } else {
        "按 Enter 翻译"
    };
    ui.colored_label(theme::MUTED, hint);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::{Failure, KeyState};
    use crate::views::Effects;
    use ice_core::config::{Config, MIN_COMPACT_H, MIN_COMPACT_W, Theme};
    use ice_core::glossary::Glossary;
    use ice_core::language::EngineKind;
    use ice_providers::request::TranslationResult;

    /// 最小简洁窗（`MIN_COMPACT_SIZE`，2026-10-28 起 420×160）：所有布局断言都在这个尺寸下做，
    /// 装得下就一定装得下更大的。数值取自唯一出处，测试里不写字面量
    fn window() -> Rect {
        Rect::from_min_size(Pos2::ZERO, Vec2::new(MIN_COMPACT_W, MIN_COMPACT_H))
    }

    fn state() -> AppState {
        let mut state = AppState::new(
            Config::default(),
            Glossary::default(),
            KeyState::Missing,
            None,
        );
        state.source = "hello".into();
        state
    }

    fn translated(text: &str) -> TranslationResult {
        TranslationResult {
            text: text.into(),
            engine: EngineKind::Machine,
            detected: None,
            elapsed_ms: 320,
            cache_hit: false,
            segments: 1,
        }
    }

    struct Harness {
        ctx: egui::Context,
        size: Vec2,
        time: f64,
    }

    impl Harness {
        fn new(size: Vec2) -> Self {
            Self {
                ctx: egui::Context::default(),
                size,
                time: 0.0,
            }
        }

        fn step(
            &mut self,
            state: &mut AppState,
            effects: &mut Effects,
            ai_available: bool,
            events: Vec<egui::Event>,
        ) -> Vec<egui::epaint::ClippedShape> {
            let mut raw = egui::RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, self.size)),
                events,
                ..Default::default()
            };
            raw.time = Some(self.time);
            self.time += 1.0 / 60.0;
            let mut output = self
                .ctx
                .run_ui(raw, |ui| show(ui, state, effects, ai_available, 1.0));
            // 形状在这一帧的 `FullOutput.shapes` 里，`end_pass` 会把它们从 graphics 层抽走
            let shapes = std::mem::take(&mut output.shapes);
            output.drop_without_applying_deltas();
            shapes
        }

        /// 命中测试用的是**上一帧**记下的控件矩形，点击之前必须先空跑两帧，
        /// 否则第一帧上任何控件都点不中
        fn warm(&mut self, state: &mut AppState, effects: &mut Effects, ai_available: bool) {
            for _ in 0..2 {
                self.step(state, effects, ai_available, vec![]);
            }
        }

        fn click(
            &mut self,
            state: &mut AppState,
            effects: &mut Effects,
            ai_available: bool,
            at: Pos2,
        ) {
            let button = |pressed| egui::Event::PointerButton {
                pos: at,
                button: egui::PointerButton::Primary,
                pressed,
                modifiers: egui::Modifiers::NONE,
            };
            self.step(
                state,
                effects,
                ai_available,
                vec![egui::Event::PointerMoved(at), button(true)],
            );
            self.step(state, effects, ai_available, vec![button(false)]);
        }
    }

    fn rect(ctx: &egui::Context, salt: &str) -> Rect {
        ctx.data_mut(|d| d.get_temp::<Rect>(rect_id(salt)))
            .unwrap_or_else(|| panic!("没有记下 {salt} 的矩形"))
    }

    fn scroll_offset(ctx: &egui::Context) -> f32 {
        let (_, _, offset) = ctx
            .data_mut(|d| d.get_temp::<(Rect, Vec2, Vec2)>(full::scroll_id(TRANSLATION_PANE)))
            .expect("译文区要有滚动容器");
        offset.y
    }

    /// 走查缺陷（用户 2026-10-27）：长译文滚到底，最后一行被底部动作行挡住。
    ///
    /// 根因不在动作行（它是真的 `Panel::bottom`，中央栏拿到的剩余高度本来就避开了它），
    /// 而在滚动容器自己：`ScrollArea` 的 `min_scrolled_size` 默认 64px，矮栏会把可视区
    /// 抬到 64px 高——多出来的那 21px 画到框外、压在动作行底下，而最大偏移又是按这个
    /// 虚高的视口算的，所以最后十几像素怎么滚都滚不进来（实测视口底边 115.9，
    /// 而译文框底边 103.0、动作行顶边 108.0）。
    #[test]
    fn the_translation_viewport_never_grows_past_its_own_pane() {
        let mut state = state();
        // 三行源文（v0.7 起输入带定高，长文本在带内滚），把 160px 的窗挤成矮栏
        state.source = "Effective project efficiency depends less on working faster than on reducing avoidable friction across the entire delivery process. In many teams, delays are not caused by a lack of effort but by unclear ownership, inconsistent priorities, and information that is scattered across too many tools.".into();
        state.translation = Some(translated(&"可以帮助团队在小问题变成系统性风险之前发现它们。同时，效率不应仅通过产出体积来衡量。可持续性能需要现实的规划、透明的沟通和足够的缓冲来吸收意想不到的变化。当团队了解最重要的事，并花更少的时间协调不必要的工作时，他们可以交付更高质量的结果。".repeat(6)));
        state.sync_char_count();
        let mut effects = Effects::default();
        let mut h = Harness::new(Vec2::new(560.0, 160.0));
        h.warm(&mut state, &mut effects, false);
        let pane = rect(&h.ctx, "translation");
        let actions = rect(&h.ctx, "actions");
        for _ in 0..30 {
            h.step(
                &mut state,
                &mut effects,
                false,
                vec![
                    egui::Event::PointerMoved(pane.center()),
                    egui::Event::MouseWheel {
                        unit: egui::MouseWheelUnit::Point,
                        delta: egui::vec2(0.0, -400.0),
                        phase: egui::TouchPhase::Move,
                        source: egui::MouseWheelSource::Trackpad,
                        modifiers: egui::Modifiers::NONE,
                    },
                ],
            );
        }
        let (viewport, content, offset) = h
            .ctx
            .data_mut(|d| d.get_temp::<(Rect, Vec2, Vec2)>(full::scroll_id(TRANSLATION_PANE)))
            .unwrap();

        assert!(
            content.y > viewport.height(),
            "译文不够长，这条测试是白跑的：内容 {} 视口 {}",
            content.y,
            viewport.height()
        );
        assert!(
            viewport.max.y <= pane.max.y + 0.5,
            "译文可视区（底边 {}）长到译文框（底边 {}）之外了，多出来的部分会画在动作行之下",
            viewport.max.y,
            pane.max.y
        );
        assert!(
            offset.y >= content.y - viewport.height() - 0.5,
            "没滚到底：偏移 {} 内容 {} 视口 {}",
            offset.y,
            content.y,
            viewport.height()
        );
        // 滚到底时正文底边就是视口底边，它必须待在动作行之上
        assert!(
            viewport.max.y <= actions.min.y + 0.5,
            "最后一行压在动作行底下：正文底边 {} 动作行顶边 {}",
            viewport.max.y,
            actions.min.y
        );
    }

    /// 量两带各自的外框矩形（含带框内边距，不含带间空隙）
    fn bands_at(src: &str, width: f32) -> (Rect, Rect) {
        let mut h = Harness::new(Vec2::new(width, window().height()));
        let mut state = state();
        state.source = src.into();
        let mut effects = Effects::default();
        h.warm(&mut state, &mut effects, false);
        (rect(&h.ctx, "input_band"), rect(&h.ctx, "translation_band"))
    }

    /// `band_heights`（REQ-012 的纯函数部分）四条口径：够分就 1:1；撑不起两个半份时输入带
    /// 退到"一行字"那一档、余下全给译文带；可用高变少时两带跟着缩且不越界；付不起不给负数。
    #[test]
    fn the_two_bands_split_equally_and_fall_back_to_one_input_line() {
        // 够分的最低那一档：两条带各自正好"一行字"（`INPUT_HEIGHT` 是内容高，
        // 外框还要加 `BAND_OUTER_PAD` 的两倍），再加两个 `BAND_GAP`
        let even = 2.0 * INPUT_HEIGHT + 2.0 * BAND_GAP + 4.0 * BAND_OUTER_PAD;
        assert_eq!(
            band_heights(even),
            (INPUT_HEIGHT, INPUT_HEIGHT),
            "分界线那一点刚好两份都是'一行字'"
        );

        for extra in [20.0, 120.0, 600.0] {
            let (input, body) = band_heights(even + extra);
            assert_eq!(
                input, body,
                "{extra} 的富余没有 1:1 摊开：{input} vs {body}"
            );
        }

        let short = even - 20.0;
        let (input, body) = band_heights(short);
        assert_eq!(input, INPUT_HEIGHT, "输入带不再往上要");
        assert_eq!(
            body,
            short - 2.0 * BAND_GAP - 4.0 * BAND_OUTER_PAD - INPUT_HEIGHT,
            "剩下的都归译文带"
        );

        // 可用高变少（窄窗里动作行换行、或窗口被压矮）时，两带跟着一起缩，
        // 且**画出来的总高从不越过可用高**——这条才是"带子压在动作行上"的真正护栏
        let roomy = band_heights(400.0);
        let tight = band_heights(360.0);
        assert!(
            roomy.0 + roomy.1 > tight.0 + tight.1,
            "可用高少了 40px，两带却没跟着缩"
        );
        for available in [400.0, 360.0, 160.0, 120.0, 60.0] {
            let (input, body) = band_heights(available);
            assert!(
                input + body + 2.0 * BAND_GAP + 4.0 * BAND_OUTER_PAD <= available + 0.5,
                "{available}px 里画不下 {input}+{body} 两带"
            );
        }

        assert_eq!(band_heights(20.0), (0.0, 0.0), "付不起也不许给出负高度");
    }

    /// REQ-012：输入带不再随内容长高——多出来的源文在它自己的框里滚，译文带那一份不会被吃掉。
    ///
    /// 最窄（400，`MIN_COMPACT_SIZE`）和默认（460）两个宽度都跑一帧：两带等高是同一个式子给的，
    /// 但窄窗里动作行会不会换行、从中央区里扣掉多少，只有真画一帧才知道。
    #[test]
    fn the_input_band_keeps_its_share_when_the_source_gets_long() {
        for width in [400.0, 460.0] {
            let (short_input, short_body) = bands_at("hello", width);
            let (long_input, long_body) = bands_at("a\nb\nc\nd\ne\nf\ng\nh", width);

            assert!(
                (short_input.height() - long_input.height()).abs() < 0.5,
                "输入带跟着内容长高了（宽 {width}）：{} → {}",
                short_input.height(),
                long_input.height()
            );
            for (input, body) in [(short_input, short_body), (long_input, long_body)] {
                assert!(
                    (input.height() - body.height()).abs() <= 0.5,
                    "两带没等分（宽 {width}）：输入 {} 译文 {}",
                    input.height(),
                    body.height()
                );
                assert!(
                    body.height() >= MIN_BODY_HEIGHT,
                    "译文带放不下正文（宽 {width}）：{:?}",
                    body.height()
                );
            }
        }
    }

    /// 用户 2026-10-28："悬浮窗口时译文后面不应该有机翻两个字，在边框上就看的到"。
    ///
    /// 判据直接落在**这一帧真的画出来的文字形状**上：「机翻」只许出现一次，就是动作行那颗
    /// 分段按钮。译文后面再挂一个就变两次，而那条挂标还占掉译文的行宽。
    #[test]
    fn the_translation_band_does_not_repeat_the_engine_name() {
        let mut state = state();
        state.translation = Some(translated("你好，世界"));
        let mut effects = Effects::default();
        let mut harness = Harness::new(egui::vec2(460.0, 160.0));
        let shapes = harness.step(&mut state, &mut effects, true, vec![]);
        let texts: Vec<String> = shapes
            .iter()
            .filter_map(|clipped| match &clipped.shape {
                egui::Shape::Text(text) => Some(text.galley.text().replace(' ', "")),
                _ => None,
            })
            .collect();

        assert!(
            texts.iter().any(|text| text == "你好，世界"),
            "译文没画出来：{texts:?}"
        );
        assert_eq!(
            texts.iter().filter(|text| text.contains("机翻")).count(),
            1,
            "「机翻」在悬浮窗里出现了不止一次（动作行那颗按钮之外又来了一处）：{texts:?}"
        );
    }

    /// 封顶之后超出那几行靠滚轮看得到（与译文区同一个手法）
    #[test]
    fn a_source_beyond_the_cap_scrolls_inside_the_input_box() {
        let mut h = Harness::new(window().size());
        let mut state = state();
        state.source = "a\nb\nc\nd\ne\nf\ng\nh".into();
        let mut effects = Effects::default();
        h.warm(&mut state, &mut effects, false);

        let offset = |h: &Harness| {
            let (_, _, at) = h
                .ctx
                .data_mut(|d| d.get_temp::<(Rect, Vec2, Vec2)>(full::scroll_id(SOURCE_PANE)))
                .unwrap();
            at.y
        };
        let input = rect(&h.ctx, "input");
        let before = offset(&h);
        for _ in 0..8 {
            h.step(
                &mut state,
                &mut effects,
                false,
                vec![
                    egui::Event::PointerMoved(input.center()),
                    egui::Event::MouseWheel {
                        unit: egui::MouseWheelUnit::Point,
                        delta: Vec2::new(0.0, -5.0),
                        phase: egui::TouchPhase::Move,
                        source: egui::MouseWheelSource::Wheel,
                        modifiers: egui::Modifiers::NONE,
                    },
                ],
            );
        }
        assert!(
            offset(&h) > before,
            "输入框封顶后滚轮给不下去：{before} → {}",
            offset(&h)
        );
    }

    /// 最小窗（`MIN_COMPACT_SIZE`，2026-10-28 晚间起 420×160）里两条带、动作行、所有按钮
    /// 与把手都必须待在窗口内，且互不重叠。
    /// 这条锁的是 REQ-005 的"不溢出、不重叠"与 REQ-011 的"退出按钮也在窗内"：
    /// v0.5 起动作行按默认字号排，装不下靠换行而不是压小
    /// （P4 的 D-1 是"断言全绿而按钮被裁出窗口"）。
    #[test]
    fn every_control_fits_the_minimum_compact_window() {
        let mut h = Harness::new(window().size());
        let mut state = state();
        state.translation = Some(translated("你好"));
        let mut effects = Effects::default();
        h.warm(&mut state, &mut effects, true);

        let win = window();
        for salt in [
            "input",
            "input_band",
            "translation_band",
            "actions",
            "bar_gap",
            "btn_theme",
            "btn_full",
            "btn_copy",
            "btn_settings",
            "btn_top",
            "btn_quit",
            "engine",
            "translation",
            "handle_tail",
        ] {
            let r = rect(&h.ctx, salt);
            assert!(
                r.min.x >= win.min.x - 0.5
                    && r.min.y >= win.min.y - 0.5
                    && r.max.x <= win.max.x + 0.5
                    && r.max.y <= win.max.y + 0.5,
                "{salt} 越出窗口：{r:?}（窗口 {win:?}）"
            );
        }

        // 同一行里的控件两两不重叠（把手更不能压住按钮，否则点按钮会变成拖窗）
        let row = [
            rect(&h.ctx, "btn_theme"),
            rect(&h.ctx, "btn_full"),
            rect(&h.ctx, "btn_copy"),
            rect(&h.ctx, "btn_settings"),
            rect(&h.ctx, "engine"),
            rect(&h.ctx, "btn_top"),
            rect(&h.ctx, "btn_quit"),
            rect(&h.ctx, "bar_gap"),
        ];
        for (i, a) in row.iter().enumerate() {
            for b in row.iter().skip(i + 1) {
                let hit = a.intersect(*b);
                assert!(
                    hit.is_negative() || hit.width() <= 0.5 || hit.height() <= 0.5,
                    "动作行里有两个控件重叠了：{a:?} 与 {b:?}"
                );
            }
        }
    }

    /// Q-011：动作行不许再压字号。量的不是中文宽度（无头字体没有中文字形，量不准），
    /// 而是按钮高度——`interact_size.y`（20px）是 egui 给按钮的下限，
    /// v0.4 那套 `Small` 字号 + (4,1) 内边距一压就露馅。
    #[test]
    fn the_action_buttons_keep_the_normal_height() {
        let mut h = Harness::new(window().size());
        let mut state = state();
        let mut effects = Effects::default();
        h.warm(&mut state, &mut effects, true);
        for salt in [
            "btn_theme",
            "btn_full",
            "btn_copy",
            "btn_settings",
            "btn_top",
            "engine",
        ] {
            let r = rect(&h.ctx, salt);
            assert!(
                r.height() >= 20.0 - 0.5,
                "{salt} 的高度被压到 {:?}，按钮应当是可正常点击的尺寸",
                r.height()
            );
        }
    }

    /// 换行机制本身：宽度收到连"每个按钮的最小可点宽度"都放不下时，动作行要**换成两行**
    /// 而不是溢出窗口。这条量的是 200px（低于合法下限，用户拉不到，但机制得在极端下也成立），
    /// 只依赖与字体无关的 `interact_size.x`（40px）下限；合法下限处放不放得下一整行，
    /// 由 `the_action_row_stays_on_one_line_at_the_minimum_width` 按度量另锁一条。
    #[test]
    fn a_row_that_cannot_fit_wraps_instead_of_overflowing() {
        let size = Vec2::new(200.0, 200.0);
        let mut h = Harness::new(size);
        let mut state = state();
        let mut effects = Effects::default();
        h.warm(&mut state, &mut effects, true);

        let actions = rect(&h.ctx, "actions");
        assert!(
            actions.max.x <= size.x + 0.5,
            "动作行溢出到了窗口外：{actions:?}（窗口宽 {}）",
            size.x
        );
        // 两行 = 两个 `interact_size.y`（20px）以上；一行最多就是这个高度
        assert!(
            actions.height() >= 40.0,
            "放不下就该换成两行，实际高度 {:?}，动作行仍然只有一行高",
            actions.height()
        );
    }

    /// 顶部把手条删掉之后，窗内第一块就是输入行，它要贴到内容区顶边（Q-013）
    #[test]
    fn the_input_row_is_the_first_band_now_the_handle_bar_is_gone() {
        let mut h = Harness::new(window().size());
        let mut state = state();
        let mut effects = Effects::default();
        h.warm(&mut state, &mut effects, false);
        assert!(
            rect(&h.ctx, "input").min.y < 5.0,
            "输入行没顶到内容区上沿，那条 10px 的把手带还在：{:?}",
            rect(&h.ctx, "input")
        );
    }

    /// REQ-012：三块自上而下是**输入带 → 译文带 → 动作行（底）**，两条带之间、带与动作行之间
    /// 都要看得出 `BAND_GAP` 那么大的空隙，且谁也不许压到谁（旧版"贴着堆"就是被走查否掉的）。
    #[test]
    fn the_bands_are_separated_by_the_gap_and_never_overlap() {
        let mut h = Harness::new(window().size());
        let mut state = state();
        state.source = "a\nb\nc\nd\ne\nf".into();
        let mut effects = Effects::default();
        h.warm(&mut state, &mut effects, false);

        let input = rect(&h.ctx, "input_band");
        let body = rect(&h.ctx, "translation_band");
        let actions = rect(&h.ctx, "actions");
        let win = window();

        assert!(
            body.min.y - input.max.y >= BAND_GAP - 0.5,
            "两带之间只剩 {:?}px，看不出是两块独立的东西",
            body.min.y - input.max.y
        );
        assert!(
            actions.min.y - body.max.y >= BAND_GAP - 0.5,
            "译文带压在动作行上：间距 {:?}px",
            actions.min.y - body.max.y
        );
        for (a, b) in [(input, body), (body, actions), (input, actions)] {
            let hit = a.intersect(b);
            assert!(
                hit.is_negative() || hit.area() <= 0.5,
                "两块带重叠了：{a:?} 与 {b:?}"
            );
        }
        assert!(
            input.min.y >= win.min.y - 0.5 && actions.max.y <= win.max.y + 0.5,
            "带子越出窗口：输入 {input:?} 动作行 {actions:?}（窗口 {win:?}）"
        );
    }

    /// 两块看不见的把手各自都要能触发原生拖拽，其余地方一个都不能
    fn drag_probe(size: Vec2, at: Pos2) -> bool {
        let mut h = Harness::new(size);
        let mut state = state();
        let mut effects = Effects::default();
        h.warm(&mut state, &mut effects, false);
        h.step(
            &mut state,
            &mut effects,
            false,
            vec![
                egui::Event::PointerMoved(at),
                egui::Event::PointerButton {
                    pos: at,
                    button: egui::PointerButton::Primary,
                    pressed: true,
                    modifiers: egui::Modifiers::NONE,
                },
            ],
        );
        effects.start_drag
    }

    #[test]
    fn the_handles_start_a_native_drag_and_nothing_else_does() {
        let mut h = Harness::new(window().size());
        let mut state = state();
        let mut effects = Effects::default();
        h.warm(&mut state, &mut effects, false);
        let (input, tail, button) = (
            rect(&h.ctx, "input"),
            rect(&h.ctx, "handle_tail"),
            rect(&h.ctx, "btn_full"),
        );
        let size = window().size();

        assert!(drag_probe(size, tail.center()), "译文底部的把手拖不动");
        assert!(
            !drag_probe(size, input.center()),
            "输入行被当成把手，用户就没法点进去打字了"
        );
        assert!(
            !drag_probe(size, button.center()),
            "把手压住了按钮，点\"全量\"会变成拖窗"
        );
    }

    /// 左排与「退出」中间那段空白才是把手（「退出」钉右之后行尾没有空位了）。
    /// 只有在真的有空位时它才存在：窗口被压到连一整行按钮都放不下时
    /// （`the_action_row_stays_on_one_line_at_the_minimum_width` 管的就是不让这件事发生在合法
    /// 下限处），那里就没有把手，靠译文尾部那一块也还能拖。
    #[test]
    fn the_blank_between_the_row_and_the_quit_button_is_a_handle() {
        let size = Vec2::new(560.0, 160.0);
        let mut h = Harness::new(size);
        let mut state = state();
        let mut effects = Effects::default();
        h.warm(&mut state, &mut effects, false);
        let gap = rect(&h.ctx, "bar_gap");
        let quit = rect(&h.ctx, "btn_quit");
        let top = rect(&h.ctx, "btn_top");
        assert!(
            gap.width() >= 20.0,
            "宽窗下中间那段空白应当留下把手：{gap:?}"
        );
        assert!(
            gap.min.x >= top.max.x && gap.max.x <= quit.min.x,
            "把手跑出了\"左排之后、退出之前\"那一段：{gap:?}（置顶 {top:?}、退出 {quit:?}）"
        );
        assert!(drag_probe(size, gap.center()), "动作行中间的把手拖不动");

        let actions = rect(&h.ctx, "actions");
        assert!(
            actions.height() < 40.0,
            "560px 宽应当放得下一整行：{actions:?}"
        );
    }

    /// REQ-005：简洁窗里能直接换引擎，且 AI 未配置时点了不派发请求（与全量窗同源）
    #[test]
    fn the_engine_segment_switches_engines_and_retranslates() {
        let mut h = Harness::new(window().size());
        let mut state = state();
        let mut effects = Effects::default();
        h.warm(&mut state, &mut effects, true);
        let engine = rect(&h.ctx, "engine");
        // 分段里第一个是「机翻」、第二个是「AI」，点右半边落在 AI 上
        let ai_at = Pos2::new(engine.max.x - 4.0, engine.center().y);

        h.click(&mut state, &mut effects, true, ai_at);

        assert_eq!(state.engine, EngineKind::Ai);
        let job = effects.job.expect("切到 AI 应当立刻用新引擎重译");
        assert_eq!(job.engine, EngineKind::Ai);
    }

    #[test]
    fn ai_is_disabled_when_it_is_not_configured() {
        let mut h = Harness::new(window().size());
        let mut state = state();
        let mut effects = Effects::default();
        h.warm(&mut state, &mut effects, false);
        let engine = rect(&h.ctx, "engine");
        h.click(
            &mut state,
            &mut effects,
            false,
            Pos2::new(engine.max.x - 4.0, engine.center().y),
        );
        assert_eq!(state.engine, EngineKind::Machine, "AI 不可用时不该切过去");
        assert!(effects.job.is_none());
    }

    #[test]
    fn the_settings_button_asks_to_open_the_settings_tab() {
        let mut h = Harness::new(window().size());
        let mut state = state();
        let mut effects = Effects::default();
        h.warm(&mut state, &mut effects, false);
        let at = rect(&h.ctx, "btn_settings").center();
        h.click(&mut state, &mut effects, false, at);
        assert!(effects.open_settings);
        assert!(!effects.toggle_mode, "设置走自己的通道，不是普通的形态切换");
    }

    /// v0.5：停靠指示按钮下线（停靠由边缘自动判定，REQ-003），这一行只剩置顶一个开关
    #[test]
    fn the_topmost_indicator_is_a_button() {
        let mut h = Harness::new(window().size());
        let mut state = state();
        let mut effects = Effects::default();
        h.warm(&mut state, &mut effects, false);
        h.click(
            &mut state,
            &mut effects,
            false,
            rect(&h.ctx, "btn_top").center(),
        );
        assert!(effects.toggle_topmost);
    }

    /// REQ-011：退出在悬浮窗上看得见、点得动。它只置一个意图，怎么落盘怎么关窗是 `app.rs` 的事
    #[test]
    fn the_quit_button_is_in_the_action_row() {
        let mut h = Harness::new(window().size());
        let mut state = state();
        let mut effects = Effects::default();
        h.warm(&mut state, &mut effects, false);
        h.click(
            &mut state,
            &mut effects,
            false,
            rect(&h.ctx, "btn_quit").center(),
        );
        assert!(effects.quit, "点了\"退出\"没有置退出意图");
    }

    /// 用户 2026-10-28："悬浮窗退出放在右下角"。右下角 = 行的右边缘（这一行就是窗口的最底一块），
    /// 所以钉住的是两条：横向贴到内容区右边、纵向落在动作行那一格内。
    #[test]
    fn the_quit_button_is_pinned_to_the_bottom_right() {
        let mut h = Harness::new(window().size());
        let mut state = state();
        let mut effects = Effects::default();
        h.warm(&mut state, &mut effects, false);
        let quit = rect(&h.ctx, "btn_quit");
        let bar = rect(&h.ctx, "bar");
        let top = rect(&h.ctx, "btn_top");

        assert!(
            (bar.max.x - PANEL_X_MARGIN - quit.max.x).abs() <= 0.5,
            "\"退出\"没贴到行的右边缘：{quit:?}（行 {bar:?}）"
        );
        assert!(
            quit.min.y >= bar.min.y - 0.5 && quit.max.y <= bar.max.y + 0.5,
            "\"退出\"跑出了动作行那一格，不在右下角：{quit:?}（行 {bar:?}）"
        );
        assert!(
            quit.min.x > top.max.x,
            "\"退出\"还跟在\"置顶\"后面挨着，没有钉到右边：{top:?} → {quit:?}"
        );
    }

    /// 用户 2026-10-28："浅色/深色/自动三种状态，但是自动在深色后"——单点三次回到原点，
    /// 每次点下去都置脏标记（生效与落盘由 `app.rs` 按脏标记做，view 不碰 `Context`）。
    #[test]
    fn the_theme_toggle_cycles_light_dark_auto_and_back() {
        let mut h = Harness::new(window().size());
        let mut state = state();
        state.config.ui.theme = Theme::Light;
        let mut effects = Effects::default();
        h.warm(&mut state, &mut effects, false);
        let at = rect(&h.ctx, "btn_theme").center();

        let mut cycle = Vec::new();
        for _ in 0..3 {
            state.config_dirty = false;
            h.click(&mut state, &mut effects, false, at);
            cycle.push(state.config.ui.theme);
            assert!(state.config_dirty, "改了主题没置脏标记，配置不会落盘");
        }
        assert_eq!(
            cycle,
            [Theme::Dark, Theme::System, Theme::Light],
            "单点顺序不是 浅色→深色→自动→浅色"
        );
    }

    /// 「自动」这一档隐去图案、只显示"自动"两个字（用户 2026-10-28 原话），
    /// 另两档画的是矢量图案——图案一律不用 `☀`/`☾` 这类字符，egui 自带字体没有那些字形
    /// （P4 缺陷 D-5 就是装饰字符 `⌗` 落成豆腐块）。
    #[test]
    fn the_auto_state_shows_two_characters_and_the_other_two_show_icons() {
        let painted = |h: &mut Harness,
                       state: &mut AppState,
                       effects: &mut Effects,
                       theme: Theme|
         -> (Vec<String>, Vec<String>) {
            state.config.ui.theme = theme;
            let bar = rect(&h.ctx, "bar");
            let shapes = h.step(state, effects, false, vec![]);
            let text: Vec<String> = shapes
                .iter()
                .filter_map(|clipped| match &clipped.shape {
                    egui::Shape::Text(t) if t.pos.y >= bar.min.y && !t.galley.text().is_empty() => {
                        Some(t.galley.text().to_owned())
                    }
                    _ => None,
                })
                .collect();
            let icons: Vec<String> = shapes
                .iter()
                .filter(|clipped| {
                    // 只认切换框**自己那一格**里的矢量图形：整条动作行上还有一根横贯窗口的
                    // 分隔线（`LineSegment`），拿它当图案就算亮了
                    rect(&h.ctx, "btn_theme")
                        .contains(clipped.shape.visual_bounding_rect().center())
                        && matches!(
                            &clipped.shape,
                            egui::Shape::Circle(_)
                                | egui::Shape::Path(_)
                                | egui::Shape::LineSegment { .. }
                        )
                })
                .map(|clipped| format!("{:?}", clipped.shape))
                .collect();
            (text, icons)
        };

        let mut h = Harness::new(window().size());
        let mut state = state();
        let mut effects = Effects::default();
        h.warm(&mut state, &mut effects, false);

        let (text, icons) = painted(&mut h, &mut state, &mut effects, Theme::Light);
        assert!(!icons.is_empty(), "浅色档没画出太阳图案");
        assert!(!text.iter().any(|t| t == "自动"), "浅色档还带着\"自动\"");

        let (text, icons) = painted(&mut h, &mut state, &mut effects, Theme::Dark);
        assert!(
            icons.iter().any(|shape| shape.starts_with("Path(")),
            "深色档没画出月亮图案：{icons:?}"
        );
        assert!(!text.iter().any(|t| t == "自动"), "深色档还带着\"自动\"");

        let (text, icons) = painted(&mut h, &mut state, &mut effects, Theme::System);
        assert!(icons.is_empty(), "自动档按用户要求要隐去图案：{icons:?}");
        assert!(
            text.iter().any(|t| t == "自动"),
            "自动档没显示\"自动\"两个字：{text:?}"
        );
    }

    #[test]
    fn the_copy_button_queues_the_translation_without_touching_the_clipboard() {
        let mut h = Harness::new(window().size());
        let mut state = state();
        state.translation = Some(translated("你好"));
        let mut effects = Effects::default();
        h.warm(&mut state, &mut effects, false);
        h.click(
            &mut state,
            &mut effects,
            false,
            rect(&h.ctx, "btn_copy").center(),
        );
        assert_eq!(effects.copy.as_deref(), Some("你好"));
    }

    /// 走查缺陷：简洁窗里的译文滚不动。定高滚动容器 + 滚轮 → 偏移必须真的变
    #[test]
    fn a_long_translation_scrolls_inside_the_compact_pane() {
        let mut h = Harness::new(window().size());
        let mut state = state();
        state.translation = Some(translated(&"一段很长的译文。".repeat(80)));
        let mut effects = Effects::default();
        h.warm(&mut state, &mut effects, false);
        let pane = rect(&h.ctx, "translation");
        let before = scroll_offset(&h.ctx);

        for _ in 0..8 {
            h.step(
                &mut state,
                &mut effects,
                false,
                vec![
                    egui::Event::PointerMoved(pane.center()),
                    egui::Event::MouseWheel {
                        unit: egui::MouseWheelUnit::Point,
                        delta: Vec2::new(0.0, -5.0),
                        phase: egui::TouchPhase::Move,
                        source: egui::MouseWheelSource::Trackpad,
                        modifiers: egui::Modifiers::NONE,
                    },
                ],
            );
        }

        assert!(
            scroll_offset(&h.ctx) > before,
            "滚轮给下去偏移没动：译文区仍然滚不动"
        );
    }

    /// 内容不超高时不该出现"能滚但什么也没有"的假滚动条（底部把手带不算内容高度）
    #[test]
    fn a_short_translation_does_not_invent_a_scrollbar() {
        let mut h = Harness::new(window().size());
        let mut state = state();
        state.translation = Some(translated("你好"));
        let mut effects = Effects::default();
        h.warm(&mut state, &mut effects, false);
        let (viewport, content, _) = h
            .ctx
            .data_mut(|d| d.get_temp::<(Rect, Vec2, Vec2)>(full::scroll_id(TRANSLATION_PANE)))
            .expect("译文区要有滚动容器");
        assert!(
            content.y <= viewport.height() + 0.5,
            "内容 {content:?} 比可视区 {viewport:?} 还高，会常驻一条没用的滚动条"
        );
    }

    /// 窗口拉宽之后仍然是一行，且行尾空出把手
    #[test]
    fn a_wider_window_does_not_overflow_and_keeps_every_control() {
        let mut h = Harness::new(Vec2::new(560.0, 160.0));
        let mut state = state();
        let mut effects = Effects::default();
        h.warm(&mut state, &mut effects, false);
        let actions = rect(&h.ctx, "actions");
        assert!(
            actions.max.x <= 560.0 + 0.5,
            "宽窗里动作行溢出：{actions:?}"
        );
        for salt in [
            "btn_theme",
            "btn_full",
            "btn_copy",
            "btn_settings",
            "btn_top",
            "engine",
        ] {
            let r = rect(&h.ctx, salt);
            assert!(r.height() >= 20.0 - 0.5, "{salt} 在宽窗里被压小了：{r:?}");
        }
    }

    /// 失败态在简洁窗里同样显示红字原因，且不动源文（FR-1.7）
    #[test]
    fn a_failure_shows_its_reason_and_keeps_the_source() {
        let mut h = Harness::new(window().size());
        let mut state = state();
        state.failure = Some(Failure {
            message: "AI 未配置".into(),
            action: None,
        });
        let mut effects = Effects::default();
        h.step(&mut state, &mut effects, false, vec![]);
        assert_eq!(state.source, "hello");
        assert!(effects.job.is_none(), "渲染失败态不该自己重发请求");
    }

    /// 整宽字形：中日韩汉字与全角标点占满一个 `1em`，其余（拉丁字母、数字、半角符号）不是
    fn is_full_width(c: char) -> bool {
        matches!(
            c as u32,
            0x2E80..=0x9FFF | 0xAC00..=0xD7A3 | 0xF900..=0xFAFF | 0xFF00..=0xFF60
        )
    }

    /// `Panel::bottom` 默认框（`Frame::side_top_panel`）的左右内边距，egui 写死在
    /// `containers/frame.rs:187` 的 `Margin::symmetric(8, 2)` 里，不在 `Spacing` 里，只能自己记
    const PANEL_X_MARGIN: f32 = 8.0;

    /// 用户 2026-10-28："这个最小窗口太小了，按钮不允许换行"。
    ///
    /// 这条**不能**像别的布局断言那样"真画一帧量按钮的 y"：egui 自带的字体里没有中文字形
    /// （`epaint-0.37.0/src/text/text_layout.rs:2192` 就写着 "No bundled font has CJK glyphs"），
    /// 真机上的中文是 eframe 的 `system_fonts` 现找的系统字体（macOS 上是 PingFang）。无头跑
    /// 出来一个汉字只有 6.5px，真机是 13.0px——在 320 宽的窗里无头照样"一行装得下"，
    /// 那条断言锁不住任何东西（P4 的 D-1 是同一类盲区，只是方向相反）。
    ///
    /// 所以判据换成按度量算，字号与内边距都取当帧 `Style` 里的真值：
    /// - 汉字整宽，一个字 `1em`；其余按 `0.6em` 估（"AI" 在 Ubuntu-Light 里实测 11.56px，
    ///   估成 15.6px 是往宽里偏，宁可把下限要求算高）
    /// - 每颗按钮加左右 `button_padding.x`，再抬到 egui 的 `interact_size.x` 下限上
    ///   （`Button` 的 `min_size` 与样式下限取大，"AI" 那颗实际是 40px 而不是 31.6px）
    /// - 主题切换框量**实测**宽度，再与"两个汉字 + 内边距"取大：这一档显示的是「自动」，
    ///   无头量不到中文字形，真机上它比 `THEME_TOGGLE_W` 更宽就要靠这个 `max` 兜住
    /// - 相邻两项加 `item_spacing.x`，面板再加左右各 8
    ///
    /// 「退出」钉在右边缘之后，把手挪到左排与它中间那段空白，两头各要 `GAP_PADDING`，
    /// 所以宽度需求比"行尾留 4px"那一版又多 4px。
    ///
    /// 按钮文字从**这一帧真的画出来的**文字形状里取：改了文案、加了按钮，这条跟着变。
    /// 跑在 `Theme::Light`（图案档，框内没有文字）下，文字清单才仍然是那七颗按钮。
    #[test]
    fn the_action_row_stays_on_one_line_at_the_minimum_width() {
        let mut h = Harness::new(window().size());
        let mut state = state();
        state.config.ui.theme = Theme::Light;
        let mut effects = Effects::default();
        h.warm(&mut state, &mut effects, true);
        let shapes = h.step(&mut state, &mut effects, true, vec![]);

        let bar = rect(&h.ctx, "bar");
        let labels: Vec<String> = shapes
            .iter()
            .filter_map(|clipped| match &clipped.shape {
                // 空文字也要滤掉：主题框在图案档是"没有内容的按钮"，egui 仍然为它排了一个空 galley
                egui::Shape::Text(text)
                    if text.pos.y >= bar.min.y && !text.galley.text().is_empty() =>
                {
                    Some(text.galley.text().to_owned())
                }
                _ => None,
            })
            .collect();
        assert_eq!(
            labels,
            ["全量", "复制", "设置", "机翻", "AI", "置顶", "退出"],
            "动作行的按钮与文档记的不一致，下面的宽度就得重算"
        );

        let style = h.ctx.global_style();
        let px = style.text_styles[&egui::TextStyle::Button].size;
        let per_button = |label: &str| -> f32 {
            let text: f32 = label
                .chars()
                .map(|c| if is_full_width(c) { px } else { 0.6 * px })
                .sum::<f32>();
            (text + 2.0 * style.spacing.button_padding.x).max(style.spacing.interact_size.x)
        };
        let items: Vec<f32> = labels.iter().map(|label| per_button(label)).collect();
        let toggle_px = rect(&h.ctx, "btn_theme")
            .width()
            .max(2.0 * px + 2.0 * style.spacing.button_padding.x);
        let needed = items.iter().sum::<f32>()
            + toggle_px
            + items.len() as f32 * style.spacing.item_spacing.x
            + 2.0 * PANEL_X_MARGIN
            + 2.0 * GAP_PADDING;

        // 先自校：这套算法必须能解释用户看到的那次换行，并且解释"为什么下限要从 400 抬走"
        assert!(
            needed > 400.0,
            "按度量算加了主题框的这一行在 400 那一档还放得下（{needed} ≤ 400），\
             那'下限要抬'这件事就没被锁住"
        );
        assert!(
            needed <= window().width(),
            "最窄简洁窗放不下动作行：需要 {needed}，可用 {:?}",
            window().width()
        );
    }
}
