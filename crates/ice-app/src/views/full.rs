//! 文本页的 B 区（源文）与 C 区（译文）：F-06~F-10。
//!
//! 绘制顺序（Tab 键的焦点遍历跟着它走）：工具条 → 状态条 → **本文件的 E 底栏** → 源文 → 译文。
//! 底栏由 `views::show` 在窗口层建，且必须建在 `CentralPanel` **之前**——`CentralPanel` 吃掉剩余
//! 矩形，之后再加的 `Panel` 收不到位置（P4 缺陷 D-1 就是这么把整条动作行裁出窗口的）。代价是
//! 底栏在焦点顺序上排到正文之前；翻译走裸 Enter 与 Cmd+Enter，不靠 Tab 走到那颗按钮。
//! 动作按钮住在这条自占一行的底栏里（REQ-013，v0.7）：上一版它浮在译文栏右下角，正文末尾要
//! 预留一条等高空白才不被压住，预留算错一点，长译文滚到底最后一行就看不见（D-9）。

use egui::{self, Align, Key, KeyboardShortcut, Layout, Modifiers, RichText};

use crate::state::{AppState, FailureAction, Tone};
use crate::system::clipboard::{self, CopyKind, EMPTY_TRANSLATION_HINT};
use crate::system::hotkeys::SOURCE_EDITOR_ID;
use crate::theme;

use super::Effects;

/// 底栏按钮的标准尺寸：等宽排列（F-09）
const BUTTON_SIZE: egui::Vec2 = egui::Vec2::new(84.0, 28.0);

/// 窄窗退让的第一档（REQ-013）：先降宽度、不降高度——用户说的是"把按钮调整小一点"，
/// 不是"小到点不到"。（`MIN_COMPACT` 那条"不许压缩控件"只约束简洁窗的动作行。）
const BUTTON_SIZE_NARROW: egui::Vec2 = egui::Vec2::new(64.0, 28.0);

/// 底栏的上下留白（px）
const BAR_MARGIN: f32 = 6.0;

/// 底栏按钮之间的间距（px）
const BAR_SPACING: f32 = 6.0;

/// 底栏左侧给字数文本留出的宽度：字数可以省略号截断，按钮不行（REQ-013 的退让顺序）
const CHAR_SLOT: f32 = 160.0;

/// 正文框的内边距（`Frame::group` 与内部定高算式共用的那一个数）
const GROUP_INNER_PAD: f32 = 8.0;

/// 两栏页脚行（徽标）的固定高度。
///
/// 徽标比纯文本高（内边距 4 + 小字号），空页脚又是 0 高——页脚高度各算各的，
/// 两栏的底边就会差出一截，真机走查里"两边窗口大小不一样"正是这么来的。
/// 固定成一行能装下徽标的高度，两栏底边才永远对齐。
const PANE_FOOTER_HEIGHT: f32 = 26.0;

/// 底栏的矩形 id：布局测试要拿它跟两栏的正文矩形比"互不重叠"
fn bottom_bar_id() -> egui::Id {
    egui::Id::NULL.with("ice-translator.bottom-bar")
}

/// 底栏上一帧实测到的矩形与它画的底色。
///
/// 底色也要记：这条栏挪到窗口层之后，`CentralPanel` 那层 `panel_fill` 不再兜在它下面，
/// 忘了自己填色就是在窗口上开个透出桌面的洞（用户 2026-10-28 第二次报的问题）。矩形与
/// 底色同批记下，断言才量的是同一条真机路径。
#[cfg(test)]
pub(crate) fn recorded_bar(ctx: &egui::Context) -> (egui::Rect, egui::Color32) {
    ctx.data_mut(|d| d.get_temp::<(egui::Rect, egui::Color32)>(bottom_bar_id()))
        .expect("底栏要记下自己的矩形与底色")
}

/// 两栏正文框（含标题下面那一整块圆角框）的矩形与底色
fn pane_frame_id(pane: &'static str) -> egui::Id {
    egui::Id::NULL.with("ice-translator.pane-frame").with(pane)
}

/// 栏目标题那一行的矩形，用来断言它真的居中在栏宽里
fn pane_title_id(pane: &'static str) -> egui::Id {
    egui::Id::NULL.with("ice-translator.pane-title").with(pane)
}

fn record_pane_frame(
    ctx: &egui::Context,
    pane: &'static str,
    rect: egui::Rect,
    fill: egui::Color32,
) {
    ctx.data_mut(|d| d.insert_temp(pane_frame_id(pane), (rect, fill)));
}

#[cfg(test)]
pub(crate) fn recorded_pane(
    ctx: &egui::Context,
    pane: &'static str,
) -> (egui::Rect, egui::Color32) {
    ctx.data_mut(|d| d.get_temp::<(egui::Rect, egui::Color32)>(pane_frame_id(pane)))
        .unwrap_or_else(|| panic!("{pane} 栏没有记下正文框"))
}

#[cfg(test)]
pub(crate) fn recorded_pane_title(ctx: &egui::Context, pane: &'static str) -> egui::Rect {
    ctx.data_mut(|d| d.get_temp::<egui::Rect>(pane_title_id(pane)))
        .unwrap_or_else(|| panic!("{pane} 栏没有记下标题行"))
}

/// 栏目标题（"源文""译文"）：整行在栏宽内居中，整行从左到右一道**连续**渐变。
///
/// egui 0.37 没有渐变文字这条路：一行 `Galley` 只有一个颜色（`Glyph` 不带色位，颜色在
/// 排版时就烘进 `Row::contents` 的网格里），`Shape` 里也没有裁剪节点。能借的是
/// `Painter`——它给每个形状记下当下的 `clip_rect`（`painter.rs:213-220`），于是同一行字
/// 可以画很多遍，每遍把 clip_rect 收到其中一个窄条上，那一遍用渐变的第 i 档色。
/// 窄条宽度由 `theme::title_bands` 定，两字标题也被切成十几档，看得出走色。
///
/// 逐字换色不是渐变（用户 2026-10-28 否掉的正是那个），所以这里**不**按字取色。
/// 字号取 `TextStyle::Body` 解析出来的值而不是写死 px，否则设置页的字号缩放（FR-2.3）
/// 对标题不起作用。
fn pane_title(ui: &mut egui::Ui, pane: &'static str, text: &str) {
    let font_id = ui.style().text_styles[&egui::TextStyle::Body].clone();
    let size = ui.ctx().fonts_mut(|f| {
        f.layout_no_wrap(text.to_owned(), font_id.clone(), egui::Color32::WHITE)
            .size()
    });
    let pad = title_inset(ui.available_width(), size.x);
    let origin = egui::pos2(ui.cursor().min.x + pad, ui.cursor().min.y);
    exact_row(ui, size.y, Layout::left_to_right(Align::Min), |_| {});

    let bands = theme::title_bands(size.x);
    let step = size.x / bands as f32;
    let outer = ui.clip_rect();
    for index in 0..bands {
        let left = origin.x + index as f32 * step;
        // 最后一条收到行尾，避免步长取整在右端留出一道没画到的缝
        let right = if index + 1 == bands {
            origin.x + size.x
        } else {
            left + step
        };
        let band = egui::Rect::from_min_max(
            egui::pos2(left, origin.y),
            egui::pos2(right, origin.y + size.y),
        );
        ui.set_clip_rect(band.intersect(outer));
        ui.painter().text(
            origin,
            egui::Align2::LEFT_TOP,
            text,
            font_id.clone(),
            theme::title_color(index as f32 / (bands - 1) as f32),
        );
    }
    ui.set_clip_rect(outer);
    // 记的是**字占的那一段**，不是整行：行矩形本来就是满栏宽，拿它比"居不居中"等于没量
    ui.ctx()
        .data_mut(|d| d.insert_temp(pane_title_id(pane), egui::Rect::from_min_size(origin, size)));
}

/// 居中的起笔留白：整行宽 `width` 放进宽 `available` 的栏里；放不下时不留（也不给负数，
/// 负数会把标题推出栏外）
fn title_inset(available: f32, width: f32) -> f32 {
    ((available - width) / 2.0).max(0.0)
}

/// 两栏各自的滚动容器。
///
/// 正文（源文编辑器 / 译文标签）都按内容自然高度布局，装不下的部分靠滚动看——
/// 不套滚动容器的话，18k 字符的粘贴会把源文框一路撑到窗口外（W-1.16 的"变形、贴底"），
/// 而且既裁不掉也滚不动。简洁窗的译文区复用同一个构造（REQ-005 要求同一手法）。
pub(crate) fn scroll_area(pane: &'static str) -> egui::ScrollArea {
    egui::ScrollArea::vertical()
        .id_salt(pane)
        // 撑满给定的正文矩形，不随内容收缩，两栏的框才有确定的高度
        .auto_shrink([false, false])
        // 可视区不许超过给它的矩形：`min_scrolled_size` 默认 64px，会把矮栏的视口抬到 64，
        // 多出来的那十几像素画到框外（简洁窗里正好压在底部动作行之下，最后一行永远看不见，
        // 用户 2026-10-27）。滚不动的风险由调用方的栏高守卫兜着，不靠这条默认值。
        .min_scrolled_height(0.0)
}

/// 把滚动容器的实测数据（可视矩形、内容尺寸、当前偏移）记在临时数据里，供布局测试断言。
///
/// 无头测试看不见像素，只能靠这三个数断言"内容确实比可视区高"和"滚轮给下去偏移真的动了"
/// ——W-1.16 的回归锁（源文框曾经既不裁剪也不能滚）。
pub(crate) fn record_scroll<R>(
    ctx: &egui::Context,
    pane: &'static str,
    output: &egui::containers::scroll_area::ScrollAreaOutput<R>,
) {
    ctx.data_mut(|d| {
        d.insert_temp(
            scroll_id(pane),
            (
                output.inner_rect,
                output.content_size,
                output.state.clamped_offset(),
            ),
        )
    });
}

pub(crate) fn scroll_id(pane: &'static str) -> egui::Id {
    egui::Id::NULL.with("ice-translator.scroll").with(pane)
}

/// 文本页的两栏正文。动作与副作用都在 `bottom_bar`（窗口层那条栏）里，这里只读 `state`。
pub fn show(ui: &mut egui::Ui, state: &mut AppState) {
    // 分两栏不能用 `Ui::columns`：它给每列的矩形用的是 `max_rect` 的底边，不看光标，
    // 底栏刚从光标矩形里扣掉那一条它看不见（实测正文底边压过条子上沿 5px）。
    // `horizontal` 系布局按 `available_size_before_wrap()` 分配，扣掉的高度真的传得进来。
    ui.horizontal_top(|ui| {
        let gap = ui.spacing().item_spacing.x;
        let half = (ui.available_width() - gap) / 2.0;
        let size = egui::vec2(half, ui.available_height());
        ui.allocate_ui_with_layout(size, egui::Layout::top_down(egui::Align::Min), |ui| {
            source_pane(ui, state)
        });
        ui.allocate_ui_with_layout(size, egui::Layout::top_down(egui::Align::Min), |ui| {
            translation_pane(ui, state)
        });
    });
}

/// 定高行：内容画进丢弃的子 Ui，光标按给定高度精确推进。
///
/// TextEdit 的外框、徽标的内边距、空页脚的实际高度都不可控（各差几个像素），
/// 只有行高定死，两栏的底边才永远对齐——真机走查里"两边窗口大小不一样"的根因。
fn exact_row(ui: &mut egui::Ui, height: f32, layout: Layout, add: impl FnOnce(&mut egui::Ui)) {
    let cursor = ui.cursor();
    let rect = egui::Rect::from_min_size(cursor.min, egui::vec2(cursor.width(), height));
    let mut child = ui.new_child(egui::UiBuilder::new().max_rect(rect).layout(layout));
    add(&mut child);
    ui.advance_cursor_after_rect(rect);
}

/// B 区：源文编辑器（F-06）。字数不再挂在它脚注上——v0.7 起住进底栏左侧（REQ-013）。
fn source_pane(ui: &mut egui::Ui, state: &mut AppState) {
    pane_title(ui, "source", "源文");
    let body = pane_body_height(ui);
    let cursor = ui.cursor();
    let body_rect = egui::Rect::from_min_size(cursor.min, egui::vec2(cursor.width(), body));
    let mut pane = ui.new_child(
        egui::UiBuilder::new()
            .max_rect(body_rect)
            .layout(Layout::top_down(Align::Min)),
    );
    // 外框由这一层画，编辑器自己不带框：TextEdit 的框跟着内容高度走，内容一溢出
    // 底边就被滚动容器裁掉（W-1.16 修复后的新毛病），译文栏的框却是定高的 Frame::group。
    // 焦点态取上一帧记下的事实——本帧的 has_focus 要等编辑器画完才知道。
    let stroke = if state.focus_in_source {
        pane.visuals().widgets.active.bg_stroke
    } else {
        pane.visuals().widgets.noninteractive.bg_stroke
    };
    let fill = theme::surface_fill(pane.visuals());
    egui::Frame::group(pane.style())
        .inner_margin(GROUP_INNER_PAD)
        .fill(fill)
        .stroke(stroke)
        .show(&mut pane, |ui| {
            ui.allocate_ui(
                egui::vec2(ui.available_width(), body - 2.0 * GROUP_INNER_PAD),
                |ui| {
                    let output = scroll_area("source").show(ui, |ui| {
                        egui::TextEdit::multiline(&mut state.source)
                            .id(egui::Id::unique(SOURCE_EDITOR_ID))
                            .frame(egui::Frame::NONE)
                            .desired_width(f32::INFINITY)
                            .min_size(egui::vec2(0.0, body - 2.0 * GROUP_INNER_PAD))
                            .hint_text("输入或粘贴文本（Enter 翻译，Shift+Enter 换行）")
                            // 裸 Enter 归"翻译"，换行只认 Shift+Enter（FR-1.1）
                            .return_key(KeyboardShortcut::new(Modifiers::SHIFT, Key::Enter))
                            .show(ui)
                    });
                    // 快捷键在绘制之前判定，所以这里记下的是给下一帧用的事实
                    state.focus_in_source = output.inner.response.has_focus();
                    record_scroll(ui.ctx(), "source", &output);
                },
            );
        });
    record_pane_frame(ui.ctx(), "source", body_rect, fill);
    ui.advance_cursor_after_rect(body_rect);

    state.sync_char_count();
}

/// 两栏共用的正文高度算法：标题行以下全部给正文框。
///
/// 底栏不在这里扣：它是窗口层的 `Panel::bottom`，在 `CentralPanel` 之前就把那一条占了，
/// 两栏拿到的可用高度本来就少了它。v0.7 第一版让两栏自己减 `bar_height`，扣的数与画的数
/// 差 3px 就把条子挤出窗口——交给 egui 排，"扣多少"和"画多高"才是同一个数（REQ-013）。
/// 徽标行也不在这里扣：它住进译文框内部（见 `translation_pane`），两栏的框因此都是
/// "标题以下满高"，底边天然对齐。
fn pane_body_height(ui: &egui::Ui) -> f32 {
    ui.available_height().max(80.0)
}

/// F-07：字数常驻；"检测：xx"只在源语言为自动检测时出现（FR-1.2）
pub fn char_count_text(state: &AppState) -> String {
    let mut text = format!("字数 {}", state.char_count);
    if state.config.translation.source.is_none() {
        if let Some(detected) = state.translation.as_ref().and_then(|r| r.detected) {
            text.push_str(&format!(" · 检测：{}", detected.display_name()));
        }
    }
    text
}

/// C 区：译文展示（F-08）+ 缓存与耗时标记（F-10）。动作按钮在底栏（F-09，REQ-013）。
fn translation_pane(ui: &mut egui::Ui, state: &mut AppState) {
    pane_title(ui, "translation", "译文");
    let body = pane_body_height(ui);
    let cursor = ui.cursor();
    let body_rect = egui::Rect::from_min_size(cursor.min, egui::vec2(cursor.width(), body));
    let mut boxed = ui.new_child(
        egui::UiBuilder::new()
            .max_rect(body_rect)
            .layout(Layout::top_down(Align::Min)),
    );
    let fill = theme::surface_fill(boxed.visuals());
    egui::Frame::group(boxed.style())
        .inner_margin(GROUP_INNER_PAD)
        .fill(fill)
        .show(&mut boxed, |ui| {
            // 徽标行住在框**里面**（2026-10-28）：挂在框外时，还没翻译的那 26px 就是
            // 正文框与底栏之间一段滚不到的空白，用户看到的正是"两个框下面空白太多"。
            // 定高这件事保留在框内——徽标有没有、有几枚，都不许把译文框顶高顶矮。
            let inner = body - 2.0 * GROUP_INNER_PAD;
            let text = (inner - ui.spacing().item_spacing.y - PANE_FOOTER_HEIGHT).max(0.0);
            // ScrollArea 的 auto_shrink(false) 会吃光"可用高度"而不是 set_min_height，
            // 不关进一个定高子 Ui，译文框就会比源文框高出一截
            ui.allocate_ui(egui::vec2(ui.available_width(), text), |ui| {
                let scrolled = scroll_area("translation").show(ui, |ui| {
                    translation_body(ui, state);
                });
                record_scroll(ui.ctx(), "translation", &scrolled);
            });
            exact_row(
                ui,
                PANE_FOOTER_HEIGHT,
                Layout::left_to_right(Align::Center),
                |ui| badge_row(ui, state),
            );
        });
    record_pane_frame(ui.ctx(), "translation", body_rect, fill);
    ui.advance_cursor_after_rect(body_rect);
}

/// 动作条里的三个常驻动作 + 失败动作（F-13）
#[derive(Clone, Copy)]
enum BarAction {
    Translate,
    Copy(CopyKind),
    Failure(FailureAction),
}

impl BarAction {
    fn label(self, busy: bool) -> &'static str {
        match self {
            BarAction::Translate => {
                if busy {
                    "翻译中…"
                } else {
                    "翻译"
                }
            }
            BarAction::Copy(kind) => kind.label(),
            BarAction::Failure(action) => action.label(),
        }
    }
}

/// 动作条上的按钮：三个常驻动作，失败时多一枚"去设置"之类（F-13）
fn bar_actions(state: &AppState) -> Vec<BarAction> {
    let mut actions: Vec<BarAction> = vec![
        BarAction::Translate,
        BarAction::Copy(CopyKind::Translation),
        BarAction::Copy(CopyKind::Bilingual),
    ];
    if let Some(action) = state.failure.as_ref().and_then(|f| f.action) {
        actions.push(BarAction::Failure(action));
    }
    actions
}

/// 底栏排布（REQ-013 的退让顺序）：标准档一排排得下就用标准档；排不下先把按钮调窄一档；
/// 还是排不下才换行。返回按钮尺寸与行数，`bar_height` 与 `bottom_bar` 都用它，
/// 所以"扣多少高度"和"画多高"是同一个数——上一版正是这两处各算各的才留出 20px 的空缝。
fn bar_layout(button_area_w: f32, count: usize) -> (egui::Vec2, usize) {
    for size in [BUTTON_SIZE, BUTTON_SIZE_NARROW] {
        if bar_rows(button_area_w, count, size) == 1 {
            return (size, 1);
        }
    }
    let size = BUTTON_SIZE_NARROW;
    (size, bar_rows(button_area_w, count, size))
}

fn bar_rows(button_area_w: f32, count: usize, size: egui::Vec2) -> usize {
    let usable = button_area_w.max(size.x);
    let per_row = ((usable + BAR_SPACING) / (size.x + BAR_SPACING))
        .floor()
        .max(1.0) as usize;
    count.div_ceil(per_row).max(1)
}

/// 底栏占多高：行数 × 按钮高 + 行距 + 上下留白。
fn bar_height(size: egui::Vec2, rows: usize) -> f32 {
    rows as f32 * size.y + (rows - 1) as f32 * BAR_SPACING + 2.0 * BAR_MARGIN
}

/// F-09 / REQ-013：E 动作底栏。`Panel::bottom` 自占一行，左侧字数、右侧按钮组。
///
/// 由 `views::show` 在**窗口层**调用（只有文本页有），不在 `full::show` 里：放进
/// `CentralPanel` 的话，它下面还压着那层框架的 8px 内边距，按钮就成了"上 7、下 15"
/// （用户 2026-10-28 那句"按钮上下留白距离不同"）。窗口层的条子与状态条相接、横向满窗，
/// 上下各留 `BAR_MARGIN`，REQ-013 那句"贴窗口下边"才真的成立。
///
/// **这条栏必须自己画底色**，不能像以前那样留空：挪到窗口层之后，它下面不再有
/// `CentralPanel` 那层 `panel_fill` 兜着，而 eframe 的默认 `clear_color` 是
/// `rgba(12,12,12,180)`（半透明深灰，`eframe/src/epi.rs:251`）——留空就是在那一条
/// 窗口区域上开个洞，直接透出桌面（用户 2026-10-28 第二次报的"这颜色怎么回事"）。
/// 工具条与状态条同样由 `Frame::side_top_panel` 画着 `panel_fill`，看不出这件事。
///
/// 从"浮层"改成"自占"的理由（用户 2026-10-27 那句"底部还是被遮住了"）：浮层不参与布局，
/// 正文必须靠预留空白躲它，预留公式与实画公式一旦不同步就又被压一行。底栏占位之后，
/// "最后一行被盖住"在结构上不成立，不需要任何预留。
///
/// 左右两块各用一个定长子 `Ui`，中间不会互相压：字数那格可以省略号截断，按钮那格不行。
/// 按钮按 `rev()` 排进 `right_to_left`，视觉顺序仍是「翻译」在最左。
pub(crate) fn bottom_bar(ui: &mut egui::Ui, state: &mut AppState, effects: &mut Effects) {
    let actions = bar_actions(state);
    let fill = ui.visuals().panel_fill;
    let frame = egui::Frame::NONE.fill(fill).inner_margin(BAR_MARGIN);
    let panel = egui::Panel::bottom("full_bottom_bar")
        .frame(frame)
        .show(ui, |ui| {
            let (size, rows) = bar_layout(ui.available_width() - CHAR_SLOT, actions.len());
            let height = bar_height(size, rows) - 2.0 * BAR_MARGIN;
            let rect = egui::Rect::from_min_size(
                ui.cursor().min,
                egui::vec2(ui.available_width(), height),
            );

            let mut left = ui.new_child(
                egui::UiBuilder::new()
                    .max_rect(egui::Rect::from_min_size(
                        rect.min,
                        egui::vec2(CHAR_SLOT, height),
                    ))
                    .layout(Layout::left_to_right(Align::Center)),
            );
            left.add_sized(
                [CHAR_SLOT - 8.0, size.y],
                egui::Label::new(RichText::new(char_count_text(state)).color(theme::MUTED))
                    .truncate(),
            );

            let mut right = ui.new_child(
                egui::UiBuilder::new()
                    .max_rect(egui::Rect::from_min_max(
                        egui::pos2(rect.min.x + CHAR_SLOT, rect.min.y),
                        rect.max,
                    ))
                    .layout(Layout::right_to_left(Align::Center)),
            );
            let busy = state.in_flight.is_some();
            for action in actions.iter().rev() {
                let response = right.add(egui::Button::new(action.label(busy)).min_size(size));
                if response.clicked() {
                    match *action {
                        BarAction::Translate => {
                            if !busy {
                                effects.request(state.submit());
                            }
                        }
                        BarAction::Copy(kind) => copy(state, effects, kind),
                        BarAction::Failure(action) => failure_action(state, effects, action),
                    }
                }
            }
            // 两块都画在丢弃的子 Ui 里，父级（面板）自己一条都没分配：不手工推进光标，
            // 面板高度就塌回"一个按钮"，换出来的第二排直接画到面板外面（与简洁窗同一个坑）
            ui.advance_cursor_after_rect(rect);
        });
    ui.ctx().data_mut(|d| {
        d.insert_temp(bottom_bar_id(), (panel.response.rect, fill));
    });
}

fn translation_body(ui: &mut egui::Ui, state: &AppState) {
    if let Some(failure) = &state.failure {
        // 失败时译文区改红字显示原因；源文一个字符都不动（FR-1.7）
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
        "译文会显示在这里"
    } else {
        "尚未翻译：按 Enter 或 Cmd+Enter"
    };
    ui.colored_label(theme::MUTED, hint);
}

/// F-10：`缓存` / `320ms` / `分 N 段` / `检测：英语`；缓存命中时不显示耗时（此时恒为 0，会误导）
fn badge_row(ui: &mut egui::Ui, state: &AppState) {
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 8.0;
        if state.in_flight.is_some() && state.translation.is_some() {
            badge(ui, theme::WARN, "更新中");
        }
        if state.failure.is_some() {
            return;
        }
        let Some(result) = &state.translation else {
            return;
        };
        if result.cache_hit {
            badge(ui, theme::OK, "缓存");
        } else {
            badge(ui, theme::MUTED, &format!("{}ms", result.elapsed_ms));
        }
        if result.segments > 1 {
            badge(ui, theme::MUTED, &format!("分 {} 段", result.segments));
        }
        if state.config.translation.source.is_none() {
            if let Some(detected) = result.detected {
                badge(
                    ui,
                    theme::MUTED,
                    &format!("检测：{}", detected.display_name()),
                );
            }
        }
    });
}

fn badge(ui: &mut egui::Ui, color: egui::Color32, text: &str) {
    egui::Frame::group(ui.style())
        .inner_margin(4.0)
        .fill(color.gamma_multiply(0.15))
        .show(ui, |ui| {
            ui.label(RichText::new(text).color(color).small());
        });
}

/// 两个复制按钮与 Cmd+Shift+C 共用这一条路径。
///
/// 空译文时只提示、不碰剪贴板——绝不静默清空用户剪贴板里原有的东西（REQ-003）。
/// 真正的写入由 `app.rs` 在帧末做，失败时由它给可读错误。
pub fn copy(state: &mut AppState, effects: &mut Effects, kind: CopyKind) {
    let translated = state
        .translation
        .as_ref()
        .map(|result| result.text.as_str())
        .unwrap_or_default();
    match clipboard::compose(kind, &state.source, translated) {
        Some(text) => effects.copy = Some(text),
        None => state.toast(Tone::Warn, EMPTY_TRANSLATION_HINT),
    }
}

/// 失败动作（REQ-004）：去设置 / 改用机翻并立即重译
pub fn failure_action(state: &mut AppState, effects: &mut Effects, action: FailureAction) {
    match action {
        FailureAction::GoSettings => state.go_to_key_settings(),
        FailureAction::UseMachine => effects.request(state.switch_to_machine()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::{Failure, KeyState};
    use ice_core::config::Config;
    use ice_core::glossary::Glossary;
    use ice_core::language::{EngineKind, Language};
    use ice_providers::error::ProviderError;
    use ice_providers::request::TranslationResult;

    fn state() -> AppState {
        let mut state = AppState::new(
            Config::default(),
            Glossary::default(),
            KeyState::Missing,
            None,
        );
        state.source = "hello world".into();
        state
    }

    fn translated(text: &str) -> TranslationResult {
        TranslationResult {
            text: text.into(),
            engine: EngineKind::Machine,
            detected: Some(Language::En),
            elapsed_ms: 320,
            cache_hit: false,
            segments: 1,
        }
    }

    fn raw_input(width: f32, height: f32) -> egui::RawInput {
        egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(width, height),
            )),
            ..Default::default()
        }
    }

    /// 无头骨架：与 `views::show` 里文本页同序——窗口层先建底栏，再进 `CentralPanel`。
    ///
    /// 少了这层，测的就不是真机那条路径。底栏"上 7、下 15"那 8px（用户 2026-10-28）只在
    /// `CentralPanel` 里才画得出来：直接调 `show` 时根本没有那层框架的 8px 下内边距。
    fn run_text_ui(ui: &mut egui::Ui, state: &mut AppState, effects: &mut Effects) {
        bottom_bar(ui, state, effects);
        egui::CentralPanel::default().show(ui, |ui| show(ui, state));
    }

    /// 跑若干帧文本页（宽 `width`、高 720），返回 Context。
    /// 两帧起步：底栏矩形与滚动容器的实测数据都是画完那一帧才记上的，
    /// 单帧读到的还是上一帧（也就是没有）的值。
    fn run_text_page(
        state: &mut AppState,
        effects: &mut Effects,
        width: f32,
        frames: usize,
    ) -> egui::Context {
        let ctx = egui::Context::default();
        let mut time = 0.0;
        for _ in 0..frames {
            let mut raw = raw_input(width, 720.0);
            raw.time = Some(time);
            time += 1.0 / 60.0;
            let output = ctx.run_ui(raw, |ui| run_text_ui(ui, state, effects));
            output.drop_without_applying_deltas();
        }
        ctx
    }

    /// 某栏滚动容器的可视矩形
    fn scroll_rect(ctx: &egui::Context, pane: &'static str) -> egui::Rect {
        ctx.data_mut(|d| d.get_temp::<(egui::Rect, egui::Vec2, egui::Vec2)>(scroll_id(pane)))
            .unwrap_or_else(|| panic!("{pane} 栏没有滚动容器"))
            .0
    }

    /// 按真实的底栏排布跑一帧文本页，返回 Context（两栏矩形从 `scroll_rect` 量）
    fn measure_columns(state: &mut AppState) -> egui::Context {
        let ctx = egui::Context::default();
        let mut effects = Effects::default();
        let output = ctx.run_ui(raw_input(640.0, MIN_BODY_HEIGHT), |ui| {
            run_text_ui(ui, state, &mut effects)
        });
        output.drop_without_applying_deltas();
        ctx
    }

    /// 取回某栏滚动容器的实测数据：可视高度 / 内容高度 / 当前偏移
    fn scroll_metrics(ctx: &egui::Context, pane: &'static str) -> ScrollMetrics {
        let (inner, content, offset) = ctx
            .data_mut(|d| d.get_temp::<(egui::Rect, egui::Vec2, egui::Vec2)>(scroll_id(pane)))
            .unwrap_or_else(|| panic!("{pane} 栏没有滚动容器"));
        ScrollMetrics {
            viewport: inner.height(),
            content: content.y,
            offset: offset.y,
        }
    }

    #[derive(Debug)]
    struct ScrollMetrics {
        viewport: f32,
        content: f32,
        offset: f32,
    }

    /// 18,000 字符的粘贴样本（W-1.16 用的就是 `/tmp/ice-18k-paste.txt` 这个量级）
    fn huge_text() -> String {
        let mut text = String::new();
        while text.chars().count() < 18_000 {
            text.push_str(
                "这是一段用来撑爆版面的中文长句，它没有任何实际含义，只是把字符数堆上去。",
            );
        }
        text
    }

    /// 把指针放在 `pointer` 处连发 8 帧小幅滚轮位移（按触控板处理，逐帧全额生效），
    /// 返回该栏滚动容器最后的偏移。
    fn wheel_scroll(
        state: &mut AppState,
        effects: &mut Effects,
        pane: &'static str,
        pointer: egui::Pos2,
    ) -> f32 {
        let ctx = egui::Context::default();
        let mut time = 0.0;
        for _ in 0..8 {
            let mut raw = raw_input(1080.0, 720.0);
            raw.time = Some(time);
            time += 1.0 / 60.0;
            raw.events.push(egui::Event::PointerMoved(pointer));
            raw.events.push(egui::Event::MouseWheel {
                unit: egui::MouseWheelUnit::Point,
                delta: egui::vec2(0.0, -5.0),
                phase: egui::TouchPhase::Move,
                source: egui::MouseWheelSource::Trackpad,
                modifiers: egui::Modifiers::NONE,
            });
            let output = ctx.run_ui(raw, |ui| run_text_ui(ui, state, effects));
            output.drop_without_applying_deltas();
        }
        scroll_metrics(&ctx, pane).offset
    }

    /// 最小窗口 640×420 扣掉工具条 56、状态条 28 之后，文本页那一整块（底栏 + 两栏 +
    /// `CentralPanel` 的上下内边距各 8）拿到的总高。骨架里底栏与中央栏都从这条数往下扣。
    const MIN_BODY_HEIGHT: f32 = 420.0 - 56.0 - 28.0;

    /// `CentralPanel::default()` 用的 `Frame::central_panel` 四边各 8px 内边距（egui
    /// `containers/frame.rs:191`）。底栏挪到窗口层之后，这 8px 落在**正文与底栏之间**，
    /// 不再压在底栏下面——压在下面时就是"按钮上下留白差 8px"那个缺陷。
    const CENTRAL_FRAME_PAD: f32 = 8.0;

    /// `Panel` 在靠正文那一侧的外边距里预留的分隔线，宽度取 `noninteractive.bg_stroke`。
    const PANEL_SEPARATOR: f32 = 1.0;

    /// 两栏正文框 `Frame::group` 的内边距（`source_pane` / `translation_pane` 里那个 8.0）。
    const GROUP_FRAME_PAD: f32 = 8.0;

    /// 两栏的正文框必须等大、顶边对齐，而且**自己带底色**。
    ///
    /// 2026-10-28 起量的是 `Frame::group` 那个框本身（`recorded_pane`），不再是里面的滚动
    /// 容器：徽标行搬进译文框内部之后两个容器的可视高度本来就差一条页脚，而用户看到的两个
    /// 框是等大的。有译文/无译文各跑一次，锁住"框高不随徽标有无变化"。
    /// 底色那两条对应同一天那句"这两个框怎么和软件其他的框颜色一样"。
    #[test]
    fn both_panes_get_the_same_box() {
        for with_translation in [true, false] {
            let mut state = state();
            if with_translation {
                state.translation = Some(translated("你好，世界"));
            }
            let ctx = measure_columns(&mut state);
            let (source, source_fill) = recorded_pane(&ctx, "source");
            let (translation, translation_fill) = recorded_pane(&ctx, "translation");
            assert!(
                (source.height() - translation.height()).abs() <= 0.5,
                "两个框不一样高（译文={with_translation}）：源文 {} 译文 {}",
                source.height(),
                translation.height()
            );
            assert!(
                (source.min.y - translation.min.y).abs() <= 0.5,
                "两个框顶边没对齐（译文={with_translation}）：源文 y={} 译文 y={}",
                source.min.y,
                translation.min.y
            );
            assert!(
                (source.width() - translation.width()).abs() <= 0.5,
                "两个框不一样宽（译文={with_translation}）：源文 {source:?} 译文 {translation:?}"
            );
            assert!(
                source.max.y <= MIN_BODY_HEIGHT + 0.5,
                "正文框被挤出栏外（y={}），最小窗口下会被裁掉",
                source.max.y
            );
            let canvas = ctx.global_style().visuals.panel_fill;
            for (pane, fill) in [("source", source_fill), ("translation", translation_fill)] {
                assert_ne!(fill, canvas, "{pane} 栏的框没填色，和窗口背景糊成一片");
                assert_eq!(fill.a(), 255, "{pane} 栏的框底色不该是半透明：{fill:?}");
            }
        }
    }

    /// 正文框要一直长到底栏边上（用户 2026-10-28："原文及译文框下面空白太多"）。
    ///
    /// 改之前实测那条空白是 36px（26 的页脚预留 + 8 的中央栏内边距 + 1 的分隔线），
    /// 而页脚在还没翻译时是一段滚不到的死白。徽标行搬进框内之后，框底到条子上沿之间
    /// 只该剩 `CentralPanel` 那 8px 内边距——多一像素都是有人又在中间垫了东西。
    #[test]
    fn the_pane_boxes_reach_down_to_the_bottom_bar() {
        let mut state = state();
        state.translation = Some(translated("你好，世界"));
        let mut effects = Effects::default();
        let ctx = run_text_page(&mut state, &mut effects, 1080.0, 2);
        let (bar, _) = recorded_bar(&ctx);
        for pane in ["source", "translation"] {
            let (frame, _) = recorded_pane(&ctx, pane);
            let gap = bar.min.y - frame.max.y;
            assert!(
                (0.0..=CENTRAL_FRAME_PAD + 0.5).contains(&gap),
                "{pane} 栏的框与底栏之间空出 {gap:.1}px（应为 {:.1}px 的中央栏内边距）：\
                 框底 {} 条子上沿 {}",
                CENTRAL_FRAME_PAD,
                frame.max.y,
                bar.min.y
            );
        }
    }

    /// 栏目标题居中在栏宽里，且不压进正文框（用户 2026-10-28 那句"居中更好"）。
    #[test]
    fn the_pane_titles_are_centered_over_their_pane() {
        let mut state = state();
        let mut effects = Effects::default();
        let ctx = run_text_page(&mut state, &mut effects, 1080.0, 2);
        for pane in ["source", "translation"] {
            let (frame, _) = recorded_pane(&ctx, pane);
            let title = recorded_pane_title(&ctx, pane);
            let left = title.min.x - frame.min.x;
            let right = frame.max.x - title.max.x;
            assert!(
                (left - right).abs() <= 2.0,
                "{pane} 栏的标题没居中：左边留 {left:.1}px，右边留 {right:.1}px"
            );
            assert!(
                title.max.y <= frame.min.y + 0.5,
                "{pane} 栏的标题压进了正文框：标题底边 {} 框顶边 {}",
                title.max.y,
                frame.min.y
            );
        }
    }

    /// 标题这一帧到底被画成什么样：直接数背景层里的文字形状。
    ///
    /// 锁的是"渐变"而不是"拼色"（用户 2026-10-28："你搞两个不同颜色的字放一起就叫艺术字
    /// 了？"）。逐字换色的写法只有两个字模形状；连续渐变是**同一行字画 N 遍、每遍裁到一条
    /// 窄带上**，所以这里要看到 N 个形状、每个的文字都是完整的"源文"，而且 N 条裁剪矩形
    /// 首尾相接铺满标题那一行。
    #[test]
    fn the_title_is_painted_as_clipped_bands_of_the_whole_word() {
        let mut state = state();
        let mut effects = Effects::default();
        let ctx = egui::Context::default();
        // 形状在这一帧的 `FullOutput.shapes` 里——`end_pass` 会把它们从 graphics 层抽走，
        // 所以不能像别的走查测试那样跑完再从 `ctx.graphics` 找。
        let mut shapes: Vec<egui::epaint::ClippedShape> = Vec::new();
        let mut time = 0.0;
        for _ in 0..2 {
            let mut raw = raw_input(1080.0, 720.0);
            raw.time = Some(time);
            time += 1.0 / 60.0;
            let mut output = ctx.run_ui(raw, |ui| run_text_ui(ui, &mut state, &mut effects));
            shapes = std::mem::take(&mut output.shapes);
            output.drop_without_applying_deltas();
        }

        for pane in ["source", "translation"] {
            let word = if pane == "source" { "源文" } else { "译文" };
            let title = recorded_pane_title(&ctx, pane);
            let bands: Vec<egui::Rect> = shapes
                .iter()
                .filter(|clipped| match &clipped.shape {
                    egui::Shape::Text(text) => text.galley.text().replace(' ', "") == word,
                    _ => false,
                })
                .map(|clipped| clipped.clip_rect)
                .collect();
            assert!(
                bands.len() > 6,
                "{pane} 栏的标题被画成 {} 段：一行字只画两三遍就是逐字换色，不是渐变",
                bands.len()
            );
            assert_eq!(
                bands.len(),
                theme::title_bands(title.width()),
                "{pane} 栏的标题段数和 `title_bands` 算出来的对不上"
            );
            // 每一条都裁在标题那一行里，且相邻条子的左沿就是上一条的右沿
            let mut left = title.min.x - 0.5;
            for clip in &bands {
                assert!(
                    (clip.min.x - left).abs() <= 0.6,
                    "{pane} 栏的色带之间有缝：上一条到 {left:.1}，这一条从 {:.1} 开始",
                    clip.min.x
                );
                assert!(
                    clip.max.x > clip.min.x && clip.min.y <= title.min.y + 0.5,
                    "{pane} 栏的色带矩形不像一条竖带：{clip:?}"
                );
                left = clip.max.x;
            }
            assert!(
                (left - title.max.x).abs() <= 0.6,
                "{pane} 栏的色带没铺满整行：铺到 {left:.1}，标题宽到 {:.1}",
                title.max.x
            );
        }
    }

    #[test]
    fn title_inset_centers_the_row_and_never_pushes_it_outside() {
        assert_eq!(title_inset(300.0, 100.0), 100.0);
        // 栏比标题还窄时不留负数（负数会把字推出栏外）
        assert_eq!(title_inset(50.0, 100.0), 0.0);
    }

    /// W-1.16：粘贴 18,000 字符之后，源文框必须老老实实待在自己那一栏里，
    /// 多出来的内容进滚动容器（修复前编辑器按内容高度一路撑到窗口外，既裁不掉也滚不动）。
    ///
    /// 断言落在滚动容器的实测数据上而不是栏目的 `min_rect`：两栏的正文都画进"丢弃的子 Ui"
    /// 再手工推进光标（底边对齐就是这么做的），子 Ui 里溢出多少，`min_rect` 一概看不见——
    /// 这正是 D-1/D-3 那批布局测试放走这个缺陷的原因。
    ///
    /// "两个框一样大"量的是 `recorded_pane`（用户看到的那个圆角框），不是滚动容器：
    /// 徽标行住进译文框内部之后，容器本来就比源文那个矮一条页脚（2026-10-28）。
    #[test]
    fn a_huge_paste_keeps_the_source_editor_inside_its_pane() {
        let mut state = state();
        state.source = huge_text();
        state.translation = Some(translated(&huge_text()));
        state.sync_char_count();
        let ctx = measure_columns(&mut state);
        let source = scroll_rect(&ctx, "source");
        let translation = scroll_rect(&ctx, "translation");
        let (source_box, _) = recorded_pane(&ctx, "source");
        let (translation_box, _) = recorded_pane(&ctx, "translation");

        assert!(
            (source_box.height() - translation_box.height()).abs() <= 0.5,
            "长文粘贴之后两个框不一样大：源文 {} 译文 {}",
            source_box.height(),
            translation_box.height()
        );
        for (pane, rect) in [("source", source), ("translation", translation)] {
            let metrics = scroll_metrics(&ctx, pane);
            assert!(
                metrics.content > metrics.viewport * 3.0,
                "{pane} 栏的内容应当远高于一屏，才谈得上滚动：{metrics:?}"
            );
            assert!(
                metrics.viewport > 0.0 && rect.max.y <= MIN_BODY_HEIGHT + 0.5,
                "{pane} 栏的可视窗口（{rect:?}）底边越出了栏目可用高 {MIN_BODY_HEIGHT}，框被内容撑出窗口了"
            );
        }
    }

    /// W-1.16：内容超出一屏时，两栏都要能用滚轮滚下去。
    /// 用户报的是"左边无法下拉、右边也没有下拉"，所以两栏都断言。
    #[test]
    fn both_panes_scroll_when_the_content_does_not_fit() {
        let mut state = state();
        state.source = huge_text();
        state.translation = Some(translated(&huge_text()));
        let mut effects = Effects::default();

        for (pane, x) in [("source", 0.25), ("translation", 0.75)] {
            let offset = wheel_scroll(
                &mut state,
                &mut effects,
                pane,
                egui::pos2(1080.0 * x, 360.0),
            );
            assert!(
                offset > 1.0,
                "{pane} 栏滚了 8 帧滚轮却一动不动（offset={offset}）"
            );
        }
    }

    /// REQ-013：底栏自占一行，排在两栏之后。四边都在窗口内，且不压任何一栏的正文可视区
    /// ——上一版它是浮层，正文靠预留躲它，预留公式与实画公式不同步就又盖住一行。
    ///
    /// 后两条锁的是 2026-10-28 那个缺陷（"按钮上下留白距离不同"）：底栏当年画在
    /// `CentralPanel` 里，被那层框架的 8px 下内边距顶离窗口底边，按钮变成上 6、下 14。
    /// 现在它是窗口层的栏，必须贴住底边、横满整窗，条子里外各留一份 `BAR_MARGIN`。
    #[test]
    fn the_bottom_bar_occupies_its_own_row_below_both_panes() {
        let mut state = state();
        state.translation = Some(translated("你好，世界"));
        state.sync_char_count();
        let mut effects = Effects::default();
        let ctx = run_text_page(&mut state, &mut effects, 1080.0, 2);
        let (bar, _) = recorded_bar(&ctx);
        let win = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1080.0, 720.0));

        assert!(
            bar.min.x >= win.min.x - 0.5 && bar.max.x <= win.max.x + 0.5,
            "底栏横向溢出窗口：{bar:?}（窗口 {win:?}）"
        );
        assert!(
            bar.max.y <= win.max.y + 0.5,
            "底栏被挤出窗口，窄窗里会看不见：{bar:?}"
        );
        for pane in ["source", "translation"] {
            let body = scroll_rect(&ctx, pane);
            assert!(
                body.max.y <= bar.min.y + 0.5,
                "{pane} 栏的可视区压进了底栏：正文底边 {} 条子上沿 {}",
                body.max.y,
                bar.min.y
            );
        }
        assert!(
            bar.min.x <= 0.5 && bar.max.x >= win.max.x - 0.5,
            "底栏没有横满整窗（{bar:?}），两侧会被页框内边距切出缺口"
        );
        assert!(
            bar.max.y >= win.max.y - 0.5,
            "底栏下方空出 {}px：它被容器的内边距顶离了窗口底边，按钮上下留白就不等了",
            win.max.y - bar.max.y
        );
        assert!(
            (bar.height() - (BUTTON_SIZE.y + 2.0 * BAR_MARGIN + PANEL_SEPARATOR)).abs() <= 0.5,
            "底栏不是'一排按钮 + 上下各一份留白 + 一条分隔线'：{bar:?}"
        );
    }

    /// 退让顺序是纯函数（与字体度量无关）：标准档排得下就用标准档；排不下先把整排调窄一档；
    /// 窄档还是排不下才换行。不许压字号，也不许横向溢出。
    #[test]
    fn the_bar_shrinks_before_it_wraps() {
        // 三枚按钮，标准档一行（3×84 + 2×6 = 264）要 264px 以上的按钮区
        assert_eq!(bar_layout(300.0, 3), (BUTTON_SIZE, 1));
        // 264 与 3×64+2×6=204 之间：标准档排不下，窄档一行放得下 → 降宽度不降高度
        assert_eq!(bar_layout(230.0, 3), (BUTTON_SIZE_NARROW, 1));
        // 连一枚窄按钮都排不下时一行只给一枚，行数跟着按钮数走，绝不给 0 行
        assert_eq!(bar_layout(10.0, 3), (BUTTON_SIZE_NARROW, 3));
        assert_eq!(
            bar_height(BUTTON_SIZE_NARROW, 3),
            3.0 * BUTTON_SIZE_NARROW.y + 2.0 * BAR_SPACING + 2.0 * BAR_MARGIN
        );
        // 四枚（失败时多一枚"去设置"）在窄档也排不下 → 换行，行高跟着长
        assert_eq!(bar_layout(230.0, 4), (BUTTON_SIZE_NARROW, 2));
        assert!(
            bar_height(BUTTON_SIZE_NARROW, 2) > bar_height(BUTTON_SIZE_NARROW, 1),
            "换行没有让底栏长高，第二排会被裁掉"
        );
    }

    /// 窄窗（400px）里底栏换行是**长高**，两栏正文跟着让出那几像素，谁也不溢出窗口
    #[test]
    fn a_narrow_window_wraps_the_bar_instead_of_overflowing() {
        let mut state = state();
        state.failure = Some(Failure {
            message: "API Key 无效".into(),
            action: Some(FailureAction::GoSettings),
        });
        let mut effects = Effects::default();
        let ctx = run_text_page(&mut state, &mut effects, 400.0, 2);
        let (bar, _) = recorded_bar(&ctx);

        assert!(
            bar.max.x <= 400.0 + 0.5,
            "窄窗里底栏横向溢出：{bar:?}（窗口宽 400）"
        );
        assert!(
            bar.height() > BUTTON_SIZE.y + 2.0 * BAR_MARGIN,
            "四枚按钮在 400px 宽应当换成两排：{bar:?}"
        );
        let body = scroll_rect(&ctx, "translation");
        assert!(
            body.max.y <= bar.min.y + 0.5,
            "底栏长高后压到了译文可视区：正文底边 {} 条子上沿 {}",
            body.max.y,
            bar.min.y
        );
    }

    /// 走查缺陷（用户 2026-10-27）：长译文滚到底，最后一行被右下角动作条盖住。
    ///
    /// v0.7 起底栏自占一行，所以这条断言的是结构而不是预留：滚到底时译文可视区的底边
    /// 就在底栏上沿之上，正文后面不许再垫任何"预留空白"（垫了就是永远滚不到的假滚动条）。
    #[test]
    fn the_last_line_stops_above_the_bottom_bar() {
        let mut state = state();
        state.translation = Some(translated(&huge_text()));
        state.sync_char_count();
        let mut effects = Effects::default();
        let ctx = egui::Context::default();
        let mut time = 0.0;
        for _ in 0..40 {
            let mut raw = raw_input(1080.0, 720.0);
            raw.time = Some(time);
            time += 1.0 / 60.0;
            raw.events
                .push(egui::Event::PointerMoved(egui::pos2(810.0, 360.0)));
            raw.events.push(egui::Event::MouseWheel {
                unit: egui::MouseWheelUnit::Point,
                delta: egui::vec2(0.0, -400.0),
                phase: egui::TouchPhase::Move,
                source: egui::MouseWheelSource::Trackpad,
                modifiers: egui::Modifiers::NONE,
            });
            ctx.run_ui(raw, |ui| run_text_ui(ui, &mut state, &mut effects))
                .drop_without_applying_deltas();
        }

        let (inner, content, offset) = ctx
            .data_mut(|d| {
                d.get_temp::<(egui::Rect, egui::Vec2, egui::Vec2)>(scroll_id("translation"))
            })
            .expect("译文栏没有滚动容器");
        let (bar, _) = recorded_bar(&ctx);

        // 先确认真滚到了底，否则下面那条"没被盖住"是白过的
        assert!(
            offset.y >= content.y - inner.height() - 0.5,
            "没滚到底：offset={} 内容={} 可视={}",
            offset.y,
            content.y,
            inner.height()
        );
        assert!(
            inner.max.y <= bar.min.y + 0.5,
            "译文可视区长进了底栏：视口底边 {} 条子上沿 {}",
            inner.max.y,
            bar.min.y
        );
        // 视口底边到条子上沿之间该有的是：译文栏 `Frame::group` 的下内边距、徽标行
        // （`PANE_FOOTER_HEIGHT`）和它上面那条行距，再加 `CentralPanel` 的下内边距与底栏的
        // 分隔线；再多的就是凭空留出的空白（滚不到的那段假滚动条）
        assert!(
            bar.min.y - inner.max.y
                <= PANE_FOOTER_HEIGHT
                    + 3.0
                    + GROUP_FRAME_PAD
                    + CENTRAL_FRAME_PAD
                    + PANEL_SEPARATOR
                    + 2.0,
            "正文与底栏之间空出 {:.1}px，比徽标行那一条还多",
            bar.min.y - inner.max.y
        );
    }

    fn run_full(state: &mut AppState, effects: &mut Effects) {
        let ctx = egui::Context::default();
        let output = ctx.run_ui(raw_input(1080.0, 720.0), |ui| {
            run_text_ui(ui, state, effects)
        });
        output.drop_without_applying_deltas();
    }

    #[test]
    fn copy_buttons_refuse_to_touch_the_clipboard_without_a_translation() {
        let mut state = state();
        let mut effects = Effects::default();
        for kind in [CopyKind::Translation, CopyKind::Bilingual] {
            copy(&mut state, &mut effects, kind);
        }
        assert!(effects.copy.is_none());
        assert_eq!(state.toasts.len(), 2);
        assert_eq!(state.toasts[0].text, EMPTY_TRANSLATION_HINT);
        assert_eq!(state.toasts[0].tone, Tone::Warn);
        // 源文不能被这次失败的动作改动
        assert_eq!(state.source, "hello world");
    }

    #[test]
    fn copy_translation_queues_only_the_translation() {
        let mut state = state();
        state.translation = Some(translated("你好，世界"));
        let mut effects = Effects::default();

        copy(&mut state, &mut effects, CopyKind::Translation);
        assert_eq!(effects.copy.as_deref(), Some("你好，世界"));
        assert!(state.toasts.is_empty(), "成功提示要等真的写进剪贴板再给");
    }

    #[test]
    fn copy_bilingual_queues_source_and_translation() {
        let mut state = state();
        state.translation = Some(translated("你好，世界"));
        let mut effects = Effects::default();

        copy(&mut state, &mut effects, CopyKind::Bilingual);
        assert_eq!(effects.copy.as_deref(), Some("hello world\n\n你好，世界"));
    }

    #[test]
    fn failure_action_go_settings_switches_tab_and_focuses_the_key_field() {
        let mut state = state();
        state.failure = Some(Failure {
            message: ProviderError::Unauthorized.to_string(),
            action: Some(FailureAction::GoSettings),
        });
        let mut effects = Effects::default();

        failure_action(&mut state, &mut effects, FailureAction::GoSettings);

        assert_eq!(state.tab, crate::state::Tab::Settings);
        assert!(state.focus_key_input);
        assert!(effects.job.is_none());
    }

    #[test]
    fn failure_action_use_machine_resubmits_with_the_machine_engine() {
        let mut state = state();
        state.engine = EngineKind::Ai;
        state.failure = Some(Failure {
            message: ProviderError::Timeout(15_000, 2).to_string(),
            action: Some(FailureAction::UseMachine),
        });
        let mut effects = Effects::default();

        failure_action(&mut state, &mut effects, FailureAction::UseMachine);

        let job = effects.job.expect("应立即重译");
        assert_eq!(job.engine, EngineKind::Machine);
        assert_eq!(job.request.text, "hello world");
        assert_eq!(state.engine, EngineKind::Machine);
        assert!(state.failure.is_none(), "重译时旧错误要清掉");
    }

    #[test]
    fn source_editor_focus_is_recorded_for_the_next_frame() {
        let mut state = state();
        let mut effects = Effects::default();
        run_full(&mut state, &mut effects);
        assert!(!state.focus_in_source, "刚打开时焦点不在源文里");

        // 模拟用户点进源文框：egui 的焦点是 memory 里的状态
        let ctx = egui::Context::default();
        ctx.memory_mut(|mem| mem.request_focus(egui::Id::unique(SOURCE_EDITOR_ID)));
        let output = ctx.run_ui(raw_input(1080.0, 720.0), |ui| {
            run_text_ui(ui, &mut state, &mut effects)
        });
        output.drop_without_applying_deltas();
        assert!(state.focus_in_source, "Enter 能不能翻译全靠这个标记");
    }

    #[test]
    fn char_count_shows_detected_language_only_in_auto_mode() {
        let mut state = state();
        state.sync_char_count();
        state.translation = Some(translated("你好"));
        assert_eq!(char_count_text(&state), "字数 11 · 检测：英语");

        state.config.translation.source = Some(Language::En);
        assert_eq!(char_count_text(&state), "字数 11");
    }

    #[test]
    fn a_cached_result_reports_the_cache_and_no_elapsed_time() {
        let mut state = state();
        state.translation = Some(TranslationResult::cached("你好", EngineKind::Machine));
        let mut effects = Effects::default();
        // 渲染不 panic 即可；标记内容由 result 字段决定，这里断言判定分支
        run_full(&mut state, &mut effects);
        let result = state.translation.as_ref().unwrap();
        assert!(result.cache_hit);
        assert_eq!(result.elapsed_ms, 0);
    }

    /// REQ-002 的 20,000 字符假设（spec 里标 HYPOTHESIS，实现第一天出结论）。
    /// 预算给到 100ms/帧：60fps 的单帧是 16.7ms，但无头这一帧包含首次字形栅格化，
    /// 留足余量；真实手感以用户粘贴 18k 字符的走查为准（W-2）。
    #[test]
    fn twenty_thousand_characters_layout_within_budget() {
        let mut state = state();
        // 中英混排，覆盖 CJK 字形与换行两条路径
        state.source = "the quick brown fox 跳过了懒狗。".repeat(1_100);
        assert!(state.source.chars().count() > 20_000);
        let mut effects = Effects::default();

        let started = std::time::Instant::now();
        run_full(&mut state, &mut effects);
        let elapsed = started.elapsed();

        state.sync_char_count();
        assert!(state.char_count > 20_000);
        println!("20k 字符单帧布局耗时：{elapsed:?}");
        assert!(
            elapsed.as_millis() < 100,
            "20k 字符单帧布局用了 {elapsed:?}，超过 100ms 预算（REQ-002 的 HYPOTHESIS 不成立）"
        );
    }
}
