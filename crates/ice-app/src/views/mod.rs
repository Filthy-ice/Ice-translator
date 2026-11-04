//! 全量界面的窗口骨架与页签分发。
//!
//! REQ-001 的四区里，工具条（A）与状态条（D）在这里，源文（B）/译文（C）两栏在 `full.rs`；
//! E 动作底栏的画法在 `full::bottom_bar`，但**调用**也在这里——它是窗口层的一条栏，
//! 必须排在 `CentralPanel` 之前。其余三个页签各自一个文件，签名统一为
//! `show(ui, &mut AppState, &mut Effects)`。简洁悬浮窗是另一套骨架，在 `compact.rs`。

use egui::{self, Align, Align2, Layout, Order, RichText};
use ice_core::history::HistoryStore;
use ice_core::language::EngineKind;

use crate::state::{AppState, Confirm, FailureAction, Job, Tab};
use crate::theme;

pub mod compact;
pub mod full;
pub mod history;
pub mod image;
pub mod settings;

/// 工具条的上下内边距（横向沿用 egui 面板自己的 8px）。
///
/// 这条栏以前是 `exact_size(56.0)` 钉死的（`03-ui-design.md` §1 那个 56px），里面却只有
/// 一行约 20px 的控件：多出来的三十几像素全在空转，正文被整块顶下去（用户 2026-10-28
/// "明明只占用一行，但是好远"）。现在栏高按内容自适应，这一条是它唯一的高度来源，
/// 字号缩放（`ui.font_scale` 到 2.0）时栏子也跟着长，不再溢出。
///
/// 类型是 `i8` 不是 `f32`：egui 0.37 把 `Margin` 收成了整数（`epaint/src/margin.rs:44`）。
const TOOLBAR_Y_MARGIN: i8 = 6;

/// 状态条的高度（`03-ui-design.md` §1）
pub const STATUS_HEIGHT: f32 = 28.0;

// ---- 控件命中矩形的登记处 ----

/// 一格里画了什么、画在哪儿：无头测试唯一能拿到的版面事实。
///
/// 看不见像素的测试只能靠这些矩形判断"控件有没有溢出窗口""把手有没有压住按钮""两颗控件
/// 的响应区重不重"——P4 的 D-1/D-3 那批缺陷正是版面越界而测试无从发现。这套办法原先在
/// `compact`/`settings`/`image` 三个文件里各抄了一份（同一个 `Id::NULL.with(前缀).with(salt)`
/// 加一次 `insert_temp`），2026-11-04 抽成这一处，四个宿主共用。
///
/// `namespace` 按宿主分（`compact`/`settings`/`image`/`toolbar`/`theme`/`pin`）：同一帧里
/// 不同栏的 `salt` 允许重名（两种形态都有 `pin`、三页都有主题滑块），不分区就会互相覆盖。
pub(crate) fn rect_id(namespace: &str, salt: &str) -> egui::Id {
    egui::Id::NULL
        .with("ice-translator.rect")
        .with(namespace)
        .with(salt)
}

/// 画的时候登记自己那一格。生产路径也在调用（不只是测试构建里才有），所以不带 `cfg(test)`。
pub(crate) fn record_rect(ctx: &egui::Context, namespace: &str, salt: &str, rect: egui::Rect) {
    ctx.data_mut(|data| data.insert_temp(rect_id(namespace, salt), rect));
}

/// 这一格到底画没画：`None` = 这一帧根本没出现这颗控件（反向断言要用它）。
#[cfg(test)]
pub(crate) fn rect_slot(ctx: &egui::Context, namespace: &str, salt: &str) -> Option<egui::Rect> {
    ctx.data_mut(|data| data.get_temp::<egui::Rect>(rect_id(namespace, salt)))
}

#[cfg(test)]
pub(crate) fn recorded_rect(ctx: &egui::Context, namespace: &str, salt: &str) -> egui::Rect {
    rect_slot(ctx, namespace, salt)
        .unwrap_or_else(|| panic!("{namespace} 里的 {salt} 没有记下矩形"))
}

// ---- 窗口层底栏的外壳 ----

/// 页面底栏共用的外壳：文本页 E 动作栏、图片页 I 状态栏（历史页没有底栏，滑块那颗在状态条）。
///
/// 两条几何规矩只在这里写一遍，两个宿主不再各抄一份：
/// ① 只能是 `Panel::bottom`，而且调用点排在 `CentralPanel` **之前**——中央区会吃掉当时所有
/// 的剩余矩形，画在它里面就被那层 8px 内边距顶离窗口下边（用户 2026-10-28 那句
/// "按钮上下留白距离不同"，实测上 7、下 15）。
/// ② 底色必须自己画 `panel_fill`——这一条下面不再有 `CentralPanel` 那层底色兜着，而 eframe
/// 默认 `clear_color` 是半透明深灰（`eframe/src/epi.rs:251`），留空等于在窗口上开个洞直接
/// 透出桌面（同一天那句"这颜色怎么回事"）。
pub(crate) fn window_bar(
    name: &str,
    margin: impl Into<egui::Margin>,
    ui: &mut egui::Ui,
    content: impl FnOnce(&mut egui::Ui),
) -> egui::Response {
    let frame = egui::Frame::NONE
        .fill(ui.visuals().panel_fill)
        .inner_margin(margin);
    egui::Panel::bottom(name)
        .frame(frame)
        .show(ui, |ui| content(ui))
        .response
}

/// 一帧里 view 想让外部世界做的事。
///
/// view 只改 `AppState`，副作用记在这里、帧末由 `app.rs` 统一执行；这样 view 函数能无头调用，
/// 不会碰到真实剪贴板、钥匙串或网络（REQ-003/REQ-005/REQ-006 的验证层次要求）。
#[derive(Debug, Default)]
pub struct Effects {
    /// 本帧要派发的翻译任务（界面上同一时刻只允许一个）
    pub job: Option<Job>,
    /// 要写进剪贴板的文本（已按"复制译文/复制对照"组装好）；`None` 表示本帧不碰剪贴板
    pub copy: Option<String>,
    /// 要存进钥匙串的 Key 明文；只在内存里传一次，不落盘不进日志
    pub save_key: Option<String>,
    /// 要清除钥匙串条目
    pub delete_key: bool,
    /// AI 配置或 Key 变了：重建 `Translator`
    pub rebuild_translator: bool,
    /// 术语增删改：把新表推给调度层（`Translator::set_glossary`），已缓存的结果也会重新后处理
    pub sync_glossary: bool,
    /// 切到简洁模式（F-05）
    pub toggle_mode: bool,
    /// 二次确认后清空历史
    pub clear_history: bool,
    /// 简洁窗把手被按下：交原生窗口拖拽（`ViewportCommand::StartDrag`，REQ-002）
    pub start_drag: bool,
    /// 简洁窗的"设置"：切回全量并打开设置页（REQ-005）
    pub open_settings: bool,
    /// 切换置顶（K-09）。快捷键那条路径直接在 `app.rs::run` 里做，这里给界面那枚图钉用
    ///（2026-10-29：`pin_toggle` 画在全量工具条与悬浮窗动作行里，原生标题栏那颗已撤）
    pub toggle_topmost: bool,
    /// 标题栏被双击（任务 #25）：铺满整屏 ↔ 还原。入口只有 `system::dblclick` 那个原生
    /// 监视器——窗内没有按钮，键盘上也没有对应快捷键（双击标题栏本来就是 macOS 的一个习惯，
    /// 只是默认动作由系统设置说了算，不是我们能改的公开 API）。
    pub toggle_fill: bool,
    /// 退出程序（REQ-011）：界面侧只剩设置页那一颗按钮（用户 2026-10-28 变更 J 把悬浮窗
    /// 右下角那颗也删了："我既然用悬浮窗了，大概率是长期使用了"），另加 `Cmd+Q`。
    /// 走的都是 `Action::Quit` 这一条路径，不另开第二条退出实现。
    pub quit: bool,
    /// 把两个尺寸槽恢复成默认值（REQ-011 落点②：尺寸被历史版本写坏了，程序不自动改，用户显式恢复）
    pub reset_geometry: bool,
    /// 「打开图片」：交出原生面板（CHG-005 REQ-001）。
    /// 记的是**意图**而不是路径——面板还没回来，帧末只负责把面板叫出来，
    /// 选中的路径由 `app.rs` 在后续帧里从回信通道读，视图永远碰不到 AppKit。
    pub open_panel: bool,
    /// 「粘贴」：帧末去剪贴板取一次图（CHG-005 REQ-001 第四条入口）。
    /// 与 `copy` 不同向：那条是写剪贴板，这条是读，而且只在用户按这颗钮时读一次，
    /// 不参与 `clipboard_watch` 的轮询（那是文本即翻，读到图片只会白读）。
    pub paste_image: bool,
    /// 「忽略这次识别」：只把在飞的那一次标记为过期，不撤回底层调用（Q-008）
    pub ignore_image: bool,
}

impl Effects {
    pub fn request(&mut self, job: Option<Job>) {
        if job.is_some() {
            self.job = job;
        }
    }
}

/// `ai_available` 由调用方从配置与钥匙串状态推出来（`config.ai.is_ready() && key.configured()`），
/// 而不是每帧去锁 `Translator` 问一次。
pub fn show(
    ui: &mut egui::Ui,
    state: &mut AppState,
    effects: &mut Effects,
    history: &HistoryStore,
    ai_available: bool,
) {
    toolbar(ui, state, effects, ai_available);
    status_bar(ui, state, ai_available);
    // E 动作底栏（REQ-013）：文本页专属，建在 `CentralPanel` 之前才拿得到位置（`CentralPanel`
    // 吃掉剩余矩形）。放在窗口层而不是页内，是为了让它贴住状态条——落在 `CentralPanel` 里时，
    // 它下方还压着那层框架的 8px 内边距，按钮上下留白就差了 8px（用户 2026-10-28 报的缺陷）。
    //
    // 这一层现在**只有两处宿主**：文本页那颗动作栏、图片页那颗"I 识别 + 忽略这次识别"。
    // 主题滑块 2026-11-04 挪进上面那条状态栏（`views::status_bar`），三页共用一行，历史页
    // 不再为它单开一条栏；设置页不画滑块，那一页自己有「主题」那一行。
    match state.tab {
        Tab::Text => full::bottom_bar(ui, state, effects),
        Tab::Image => image::status_bar(ui, state, effects),
        Tab::History | Tab::Settings => {}
    }

    egui::CentralPanel::default().show(ui, |ui| match state.tab {
        Tab::Text => full::show(ui, state),
        Tab::Image => image::show(ui, state, effects, ice_vision::native_ocr_supported()),
        Tab::History => history::show(ui, state, history),
        Tab::Settings => settings::show(ui, state, effects, ai_available),
    });

    let ctx = ui.ctx().clone();
    toasts(&ctx, state);
    confirm(&ctx, state, effects);
}

/// A 区：页签（F-01）+ 语言（F-02/F-03）+ 引擎（F-04）+ 悬浮（F-05）
///
/// 与正文的分界交给 `Panel` 自己画的那条分隔线（`show_separator_line` 默认开，取
/// `widgets.noninteractive.bg_stroke`）——底栏那条走的是同一段代码、同一个颜色，两栏的
/// 边界感自然一致。这里**不再额外套一圈 stroke**：2026-10-28 加过一圈 `edge_color` 边框，
/// 用户当场退回"太明显了，突兀，和底部栏的分割线一样颜色就行，而且两侧不需要线条"。
/// 栏高囤积才是"界限不明显"的真凶，那条由 `TOOLBAR_Y_MARGIN` 管。
fn toolbar(ui: &mut egui::Ui, state: &mut AppState, effects: &mut Effects, ai_available: bool) {
    let frame = egui::Frame::side_top_panel(ui.style())
        .inner_margin(egui::Margin::symmetric(8, TOOLBAR_Y_MARGIN));
    egui::Panel::top("toolbar").frame(frame).show(ui, |ui| {
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 8.0;
            tabs(ui, state);
            ui.separator();
            source_language(ui, state);
            target_language(ui, state, effects);
            ui.separator();
            engine_picker(ui, state, effects, ai_available);
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                let floating = ui
                    .button("精简")
                    .on_hover_text("切到精简窗口（Cmd+Shift+M）");
                record_rect(ui.ctx(), "toolbar", "floating", floating.rect);
                if floating.clicked() {
                    effects.toggle_mode = true;
                }
                // 用户 2026-10-29 指定"挪动到精简（悬浮窗）两个字左侧"。这条带子是
                // `right_to_left`，所以后加的落在左边——顺序反过来就跑到窗口右边缘去了。
                pin_toggle(ui, state, effects, "full");
            });
        });
    });
}

fn tabs(ui: &mut egui::Ui, state: &mut AppState) {
    for tab in Tab::all() {
        if ui.selectable_label(state.tab == tab, tab.label()).clicked() {
            // 切页签不重建数据、不触发请求：各页状态都常驻 AppState
            state.tab = tab;
        }
    }
}

fn source_language(ui: &mut egui::Ui, state: &mut AppState) {
    let selected = state.config.translation.source;
    let languages = state.visible_languages();
    let label = selected
        .map(|lang| lang.display_name().to_string())
        .unwrap_or_else(|| "自动检测".to_string());
    egui::ComboBox::from_id_salt("source_language")
        .selected_text(label)
        .show_ui(ui, |ui| {
            if ui
                .selectable_label(selected.is_none(), "自动检测")
                .clicked()
            {
                state.set_source_language(None);
            }
            for lang in languages {
                if ui
                    .selectable_label(selected == Some(lang), lang.display_name())
                    .clicked()
                {
                    state.set_source_language(Some(lang));
                }
            }
        });
}

fn target_language(ui: &mut egui::Ui, state: &mut AppState, effects: &mut Effects) {
    let selected = state.config.translation.target;
    let languages = state.visible_languages();
    egui::ComboBox::from_id_salt("target_language")
        .selected_text(selected.display_name())
        .show_ui(ui, |ui| {
            for lang in languages {
                if ui
                    .selectable_label(selected == lang, lang.display_name())
                    .clicked()
                {
                    // 切目标语言自动重译（FR-1.4、FR-1.3）
                    effects.request(state.set_target(lang));
                }
            }
        });
}

/// 引擎分段（F-04）。全量工具条与简洁窗动作行共用同一套：`Button::selectable` 的选中态、
/// AI 未配置时禁用并给说明，简洁窗不再新造一份（REQ-005）。
///
/// 返回两个分段按钮合起来的矩形：简洁窗的布局断言要拿它确认把手没压在分段上。
pub(crate) fn engine_picker(
    ui: &mut egui::Ui,
    state: &mut AppState,
    effects: &mut Effects,
    ai_available: bool,
) -> egui::Rect {
    let mut rect = egui::Rect::NOTHING;
    for kind in EngineKind::all() {
        let enabled = kind != EngineKind::Ai || ai_available;
        let mut button = ui.add_enabled(
            enabled,
            egui::Button::selectable(state.engine == kind, kind.display_name()),
        );
        if !enabled {
            button =
                button.on_disabled_hover_text("AI 未配置：请在设置页填地址、模型并保存 API Key");
        }
        rect = rect.union(button.rect);
        if button.clicked() {
            effects.request(state.set_engine(kind));
        }
    }
    rect
}

/// D 区：状态条（F-11）。三态——就绪 / 翻译中 / 错误；错误态带"去设置"入口。
///
/// 右端那颗主题滑块也住在这一行（用户 2026-11-04："历史没有这一行，如果放在同样的位置就很突兀，
/// 所以还是放在最下方吧，和状态栏一行"）。挪过来之后它有了**唯一一个宿主**：文本 / 图片 / 历史
/// 三页共用这一条 `Panel::bottom`，不再各建一条底栏去挂它——原来三条宿主栏里那三份
/// `right_to_left` 子带（以及历史页那条只为一颗滑块而存在的栏）随之删掉。
/// 设置页仍然不画：那一页自己有「主题」那一行，两颗并排会打架。
fn status_bar(ui: &mut egui::Ui, state: &mut AppState, ai_available: bool) {
    egui::Panel::bottom("status")
        .exact_size(STATUS_HEIGHT)
        .show(ui, |ui| {
            record_rect(ui.ctx(), "status", "band", ui.max_rect());
            ui.horizontal(|ui| {
                let color = if state.failure.is_some() {
                    theme::ERROR
                } else if state.in_flight.is_some() {
                    theme::WARN
                } else {
                    theme::MUTED
                };
                ui.colored_label(color, state.status_text(ai_available));
                if state.failure.as_ref().and_then(|f| f.action) == Some(FailureAction::GoSettings)
                    && ui.small_button("去设置").clicked()
                {
                    state.go_to_key_settings();
                }
                if state.tab != Tab::Settings {
                    // 状态文字是 LTR 排下来的，滑块要贴右缘就得另起一条 RTL 子带；
                    // 子带从当前光标起算，所以它排在"去设置"右边那一段空白里。
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        theme_switch(ui, state, "status");
                    });
                }
            });
        });
}

/// 主题滑动开关。用户 2026-10-28 变更 J 的原话："放在右下做成一个类似于开关左右推动的效果…
/// 你可以做成开关盖住太阳，露出月亮，就是暗色，如果盖住月亮，露出太阳就是浅色，在中间就是
/// 自动。而且全量界面也要有这个开关，不需要提示，看到的人都懂。也在右下角。"
///
/// 所以判据是**露出来的那一侧**，不是滑块所在的一侧：滑块压在马上（左端）时太阳被盖住、
/// 月亮露着 = 深色；滑块压在右端时反过来 = 浅色；停在中间两枚都露一半 = 自动。
///
/// 三条口径：
/// ① 三态仍是既有的 `config::Theme::{Light, Dark, System}`，这里只写
///    `state.config.ui.theme` + 标脏，生效仍在 `theme::apply`（`app.rs` 每帧调）——
///    与设置页外观分区那组单选、与那颗框都是**同一个配置项**，不另开第二套深浅色实现；
/// ② **没有悬停提示、没有文字**（"不需要提示，看到的人都懂"），变更 F 那档「自动」两字
///    随之作废；
/// ③ 太阳/月亮是矢量线稿，不是 `☀`/`☾` 字符——egui 自带字体没这些字形，真机落成豆腐块
///    （P4 缺陷 D-5 那条教训）。
///
/// `salt` 用来把矩形记进 `Context::data`，让两种形态各自的测试能同时找到那格。
pub(crate) const THEME_SWITCH_W: f32 = 40.0;
/// 轨道高 = 滑块直径 + 一道笔画，竖向不撑高所在那一行（同一行还有约 28px 的按钮）。
pub(crate) const THEME_SWITCH_H: f32 = 20.0;
/// 滑块半径：太阳光芒外沿 7.6、月亮半径 7，8px 的实心圆刚好把它们整个盖住——
/// "盖住"是这块控件的全部语义，盖不住就等于没做到。
pub(crate) const KNOB_RADIUS: f32 = 8.0;
/// 滑块离窗口右边缘的让位量（用户 2026-11-04："由于边框是圆角的，而且边框内部边上好像还有一圈
/// 看不见颜色的空气墙，主题切换按钮太贴边会导致浅色状态下，按钮会被覆盖一部分。精简窗也存在这个
/// 问题，是不是将其向窗口左侧挪动3-5px？"）。
///
/// 让位做在 `theme_switch` 自己身上（槽位比控件宽这一截），所以状态栏那一行与精简窗动作行
/// **两处一起生效**，不用在两个宿主里各减一次；要调手感只动这一个数。
///
/// 这一截只解决"离窗口边框太近"。用户 2026-11-04 看了第一版仍说"还是没有完全显示"，真机截图
/// 量出来啃掉那一小条的**不是窗口边缘**：轨道右端在 x=59、窗口边框在 x=73，中间空着 14px；
/// 被吃掉的是滑块自己那一头，规矩在 `KNOB_TRACK_GAP`。
pub(crate) const THEME_EDGE_INSET: f32 = 4.0;
/// 滑块圆心离轨道两端的**额外**让位（在 `KNOB_RADIUS` 之外）。
///
/// 原来圆心就贴着轨道端头内缩一个半径，滑块最外沿于是**正好落在轨道轮廓线上**：轨道那一圈
/// 描边 1px 画在矩形内侧，滑块那一圈 1.5px 描边画在圆周居中，两者在切点处叠在一起，轨道右端
/// 那半个圆帽被整个盖掉。深色下两圈颜色接近看不出来，浅色下轨道只剩描边可看（实测底色 249、
/// 轨道线 226），于是那枚滑块看着不像"在开关里面"，而像"把开关顶破了一角"——这就是
/// "按钮会被覆盖一部分"的真实机制。
///
/// 让开 3px 之后：滑块最外沿离轨道轮廓 2.25px（3 减掉描边的一半），轨道两端那两个月牙形的
/// 留白重新露出来，滑块回到轨道**内部**。三档的圆心与太阳/月亮的图案中心共用这一个函数
/// （`knob_center_x`），所以"盖住哪一侧"的语义没变：滑块半径 8 仍大于太阳光芒外沿 7.6
/// 与月亮半径 7。
pub(crate) const KNOB_TRACK_GAP: f32 = 3.0;

/// 三档在轨道上的位置（0 = 左端盖住太阳，1 = 右端盖住月亮）
pub(crate) fn theme_slot(theme: ice_core::config::Theme) -> f32 {
    use ice_core::config::Theme;
    match theme {
        Theme::Dark => 0.0,
        Theme::System => 0.5,
        Theme::Light => 1.0,
    }
}

/// 某档时滑块的圆心横坐标——同时也是那一侧图案（太阳/月亮）的圆心。
///
/// 画滑块、画图案、测试量位置都走这一个函数，免得"贴不贴轨道边"这件事有两份口径
/// （见 `KNOB_TRACK_GAP`）。
pub(crate) fn knob_center_x(rect: egui::Rect, theme: ice_core::config::Theme) -> f32 {
    let left = rect.min.x + KNOB_RADIUS + KNOB_TRACK_GAP;
    let right = rect.max.x - KNOB_RADIUS - KNOB_TRACK_GAP;
    left + (right - left) * theme_slot(theme)
}

/// 轨道上某一点落在哪一档：取最近的一档，所以左四分之一是深色、右四分之一是浅色、
/// 中间一半都是自动。拖动时不会在两档边界抖——指针要退回 0.25 才换档。
pub(crate) fn theme_at(t: f32) -> ice_core::config::Theme {
    use ice_core::config::Theme;
    if t < 0.25 {
        Theme::Dark
    } else if t > 0.75 {
        Theme::Light
    } else {
        Theme::System
    }
}

fn theme_switch_id(salt: &str) -> egui::Id {
    rect_id("theme", salt)
}

pub(crate) fn theme_switch(
    ui: &mut egui::Ui,
    state: &mut AppState,
    salt: &'static str,
) -> egui::Rect {
    // 只占位不响应（`allocate_at_least` 会自带一次 interact，两次点击判定会让同一枚
    // 指针被两个响应者各消费一遍）；真正的命中交给下面那个带稳定 `Id` 的 `interact`。
    //
    // 槽位比画出来的那一格宽 `THEME_EDGE_INSET`：`right_to_left` 的带子里 `allocate_space`
    // 从光标往左占，所以槽的**右**端就是原本贴窗口边缘那一头，让位留在右边，画与命中都只吃
    // 左边那 `THEME_SWITCH_W`。这颗控件只出现在 RTL 带子里（状态栏右端、精简窗动作行右端），
    // 换到 LTR 里让位会跑到左侧——加新宿主时注意这点。
    let (_auto_id, slot) = ui.allocate_space(egui::vec2(
        THEME_SWITCH_W + THEME_EDGE_INSET,
        THEME_SWITCH_H,
    ));
    let rect = egui::Rect::from_min_size(slot.min, egui::vec2(THEME_SWITCH_W, THEME_SWITCH_H));
    let visuals = ui.visuals();
    let painter = ui.painter_at(rect);
    let center_y = rect.center().y;
    // 两端图案的圆心 = 滑块在那两档时的圆心，同一个函数算出来，盖住的关系恒成立
    let left_x = knob_center_x(rect, ice_core::config::Theme::Dark);
    let right_x = knob_center_x(rect, ice_core::config::Theme::Light);
    let icon_stroke = egui::Stroke::new(1.5, visuals.widgets.inactive.fg_stroke.color);

    painter.rect_filled(
        rect,
        rect.height() / 2.0,
        visuals.widgets.noninteractive.bg_fill,
    );
    painter.rect_stroke(
        rect,
        rect.height() / 2.0,
        visuals.widgets.noninteractive.bg_stroke,
        egui::StrokeKind::Inside,
    );
    draw_sun(&painter, egui::pos2(left_x, center_y), icon_stroke);
    draw_moon(&painter, egui::pos2(right_x, center_y), icon_stroke);
    // 滑块最后画：它是不透明的那块圆，盖住哪一侧就露出另一侧
    let knob = egui::pos2(knob_center_x(rect, state.config.ui.theme), center_y);
    painter.circle_filled(knob, KNOB_RADIUS, visuals.widgets.inactive.bg_fill);
    painter.circle_stroke(knob, KNOB_RADIUS, icon_stroke);

    let response = ui.interact(rect, theme_switch_id(salt), egui::Sense::click_and_drag());
    if response.dragged() || response.clicked() {
        if let Some(pos) = response.interact_pointer_pos() {
            let t = ((pos.x - rect.min.x) / rect.width().max(1.0)).clamp(0.0, 1.0);
            let next = theme_at(t);
            if next != state.config.ui.theme {
                state.config.ui.theme = next;
                state.mark_config_dirty();
            }
        }
    }
    record_rect(ui.ctx(), "theme", salt, rect);
    rect
}

/// 置顶图钉那一格（FR-3.4 的可见入口）。
///
/// **2026-10-29 从原生标题栏挪进来**（用户走查原话："图钉？哪来的图钉？让你加也没见你加啊"，
/// 以及"全量窗挪动到精简（悬浮窗）两个字左侧，悬浮窗状态下，放在（右下角）颜色切换左边"）。
/// 原生那颗挂在 `NSTitlebarAccessoryViewController` 里，位置只有一个——窗口右边缘——挪不到
/// 用户指定的任何一处；而为了那一格要付的纪律（每帧核对是否还在、`Retained` 自持 target、
/// `lockFocus` 旋转、一次点击只兑一次）换来的观感是"右上角一个孤零零的小符号"。画在这里它和
/// 其他控件同族、跟着主题色走，置顶那条通道（`Effects::toggle_topmost`）一个字没变。
///
/// 图案是矢量线稿，不用 `📌` 那类字符（egui 字体没有，真机落豆腐块——P4 缺陷 D-5 的教训）。
/// 两态沿用原生那套语义：**未置顶 = 本色 + 斜躺 + 空心，置顶 = 强调色 + 立正 + 实心**。
pub(crate) const PIN_TOGGLE_W: f32 = 26.0;
/// 与主题滑块同高，两种形态右下角那一排里两枚并排时竖向不打架。
pub(crate) const PIN_TOGGLE_H: f32 = THEME_SWITCH_H;
/// 未置顶时整枚图钉绕中心躺过去的角度（度）：钉尖落到左下，与原生那颗 `pin` 的出厂方向一致。
const PIN_TILT_DEG: f32 = 45.0;

/// 悬停提示：说清现在什么状态、点一下会怎样、键盘上等价的那条是什么。
fn pin_tip(pinned: bool) -> &'static str {
    if pinned {
        "已置顶：窗口压在其他应用之上，点一下取消（Cmd+Shift+P）"
    } else {
        "置顶：窗口压在其他应用之上（Cmd+Shift+P）"
    }
}

fn pin_id(salt: &'static str) -> egui::Id {
    rect_id("pin", salt)
}

/// 画一枚图钉并接住点击。返回它的矩形，和 `theme_switch` 一样把矩形记进 `Context::data`
/// 让两种形态的测试都找得到那一格。
pub(crate) fn pin_toggle(
    ui: &mut egui::Ui,
    state: &mut AppState,
    effects: &mut Effects,
    salt: &'static str,
) -> egui::Rect {
    // 只占位不响应，真正的命中交给下面那个带稳定 `Id` 的 `interact`
    // （`allocate_space` 自带一次 interact，两次会让同一枚指针被两个响应者各消费一遍）
    let (_auto_id, rect) = ui.allocate_space(egui::vec2(PIN_TOGGLE_W, PIN_TOGGLE_H));
    let pinned = state.config.ui.always_on_top;
    let color = if pinned {
        theme::ACCENT
    } else {
        ui.visuals().widgets.inactive.fg_stroke.color
    };
    draw_pin(&ui.painter_at(rect), rect.center(), pinned, color);
    let response = ui.interact(rect, pin_id(salt), egui::Sense::click());
    if response.clicked() {
        effects.toggle_topmost = true;
    }
    response.on_hover_text(pin_tip(pinned));
    record_rect(ui.ctx(), "pin", salt, rect);
    rect
}

/// 圆头 + 一横底座 + 往下那根针。`pinned` 为假时整枚绕中心转 45° 躺下，头也只剩描边。
fn draw_pin(painter: &egui::Painter, at: egui::Pos2, pinned: bool, color: egui::Color32) {
    let stroke = egui::Stroke::new(1.5, color);
    let angle = if pinned {
        0.0
    } else {
        PIN_TILT_DEG.to_radians()
    };
    let (sin, cos) = angle.sin_cos();
    let spin = |p: egui::Vec2| egui::vec2(p.x * cos - p.y * sin, p.x * sin + p.y * cos);
    let point = |p: egui::Vec2| at + spin(p);
    let head = egui::vec2(0.0, -5.0);
    painter.line_segment(
        [point(egui::vec2(-3.4, -1.6)), point(egui::vec2(3.4, -1.6))],
        stroke,
    );
    painter.line_segment(
        [point(egui::vec2(0.0, -1.6)), point(egui::vec2(0.0, 6.2))],
        stroke,
    );
    if pinned {
        painter.circle_filled(point(head), 3.4, color);
    }
    painter.circle_stroke(point(head), 3.4, stroke);
}

#[cfg(test)]
pub(crate) fn recorded_pin(ctx: &egui::Context, salt: &'static str) -> egui::Rect {
    recorded_rect(ctx, "pin", salt)
}

/// 一段圆弧采样成折线（epaint 0.37 没有圆弧图元，只能自己按角度取点）
fn arc_points(
    center: egui::Pos2,
    radius: f32,
    from: f32,
    to: f32,
    steps: usize,
) -> Vec<egui::Pos2> {
    (0..=steps)
        .map(|i| {
            let angle = from + (to - from) * i as f32 / steps as f32;
            center + egui::vec2(angle.cos(), angle.sin()) * radius
        })
        .collect()
}

/// 太阳：一圈描边的圆 + 八道光芒。全是线稿，与月亮同一套笔画粗细。
fn draw_sun(painter: &egui::Painter, at: egui::Pos2, stroke: egui::Stroke) {
    painter.circle_stroke(at, 3.6, stroke);
    for index in 0..8 {
        let angle = std::f32::consts::TAU * index as f32 / 8.0;
        let dir = egui::vec2(angle.cos(), angle.sin());
        painter.line_segment([at + dir * 5.4, at + dir * 7.6], stroke);
    }
}

/// 月亮：月牙轮廓 = 外弧 + 内弧，两段弧各画一条**开放**折线，两端在尖端重合。
///
/// 只描边不填充：`PathShape` 的 `fill` 只支持凸多边形（`path_shape.rs:16`），月牙不是凸的，
/// 填出来会画歪。外弧取整圆的左侧 230°，内弧的圆心右移 `BIAS`、半径取到同样穿过那两个
/// 尖端，两弧之间剩下的就是月牙。
///
/// **为什么不能合成一条 `closed_line`**（2026-11-04 变更 S）：两弧在尖端只夹 27°，闭合成环
/// 之后那里就是一个 152.8° 的发卡弯。epaint 的 `add_line_loop`（`tessellator.rs:432`）对每个
/// 顶点做 miter join——法线取相邻两段法线的平均再除以长度平方，夹角越尖放大越多，152.8° 那一
/// 点放大到 4.25 倍，再叠上同方向的 feather 扩张，就沿角平分线甩出一条亮线。真机量到的是从尖端
/// 向右下 6.7px、亮度 214（和笔画本身一样亮）的斜线——用户看到的"月亮右上角有个斜杠"。
/// 它的长短方向还随滑块的亚像素位置变，所以"拉大一点就不见了"，不是消失是缩到半个像素。
/// 开放折线的首尾走 `add_line_path` 那一支，用各自那一段的法线，没有 miter，尖端恢复成
/// 1.5px 的平头。
fn draw_moon(painter: &egui::Painter, at: egui::Pos2, stroke: egui::Stroke) {
    const TIP: f32 = 65.0_f32.to_radians();
    const R: f32 = 7.0;
    const BIAS: f32 = 3.2;
    const STEPS: usize = 20;
    let inner_center = at + egui::vec2(BIAS, 0.0);
    // `+y` 朝下，所以这是月牙朝下的那个尖端；它的极角就是内弧的端点角
    let tip = at + egui::vec2(R * TIP.cos(), R * TIP.sin());
    let offset = tip - inner_center;
    let outer = arc_points(at, R, TIP, std::f32::consts::TAU - TIP, STEPS);
    let inner = arc_points(
        inner_center,
        offset.length(),
        std::f32::consts::TAU - offset.angle(),
        offset.angle(),
        STEPS,
    );
    for points in [outer, inner] {
        painter.add(egui::Shape::Path(egui::epaint::PathShape::line(
            points, stroke,
        )));
    }
}

/// 提示条：**窗口正中**，最多三条，出生即开始线性淡出，`TOAST_LIFETIME`（1s）到点消失。
///
/// 用户 2026-10-28 变更 I 的原话："所有的提示类弹窗都是中心弹窗500ms内淡化消失，
/// 不应该有关闭的操作和按钮。"三条各自对应一处删除：
/// ① 不再挂在顶部（原来全量窗贴顶 12px、简洁窗另算一套越过输入行的偏移）而落在正中；
/// ② 不再四秒常驻，整条 lifetime 就是那次淡出；
/// ③ 那颗 `×` 连同 `AppState::dismiss_toast` 一起没了，Area 也改成 `interactable(false)`——
///    不只是"少画一颗按钮"，而是**这一层根本不参与命中测试**，提示条盖住的按钮照样能点。
///
/// 2026-10-29 改判只动了时长：那句里的 500ms 太短，`TOAST_LIFETIME` 抬到 1s，其余三条不变。
///
/// 淡的是整块（底色、描边、投影、文字一起乘同一个 alpha），所以不会出现"框还在、字先没了"。
/// 二次确认框（`confirm`）不在这条规矩里：它要用户做决定，不是"说一声就走"的提示。
pub(crate) fn toasts(ctx: &egui::Context, state: &mut AppState) {
    if state.toasts.is_empty() {
        return;
    }
    let band = egui::Area::new(egui::Id::unique("toasts"))
        .anchor(Align2::CENTER_CENTER, [0.0, 0.0])
        .order(Order::Foreground)
        .interactable(false)
        .show(ctx, |ui| {
            ui.vertical(|ui| {
                for toast in state.toasts.iter() {
                    let alpha = toast.opacity();
                    // `multiply_with_opacity` 一次处理底色、描边、投影（egui 自己那三行就是它）
                    egui::Frame::popup(ui.style())
                        .multiply_with_opacity(alpha)
                        .show(ui, |ui| {
                            ui.label(
                                RichText::new(&toast.text)
                                    .color(theme::tone_color(toast.tone).gamma_multiply(alpha)),
                            );
                        });
                }
            });
        })
        .response
        .rect;
    ctx.data_mut(|d| d.insert_temp(toast_band_id(), band));
}

/// 提示条那一带的矩形，只给无头断言用（判"是不是落在窗口正中"、"点上去会不会把它点掉"）
#[cfg(test)]
pub(crate) fn recorded_toast_band(ctx: &egui::Context) -> Option<egui::Rect> {
    ctx.data_mut(|d| d.get_temp::<egui::Rect>(toast_band_id()))
}

fn toast_band_id() -> egui::Id {
    egui::Id::NULL.with("ice-translator.toast-band")
}

/// 破坏性操作的二次确认（清空历史、清除 Key）
fn confirm(ctx: &egui::Context, state: &mut AppState, effects: &mut Effects) {
    let Some(kind) = state.confirm else {
        return;
    };
    let mut confirmed = None;
    egui::Area::new(egui::Id::unique("confirm"))
        .anchor(Align2::CENTER_CENTER, [0.0, 0.0])
        .order(Order::Foreground)
        .interactable(true)
        .show(ctx, |ui| {
            egui::Frame::popup(ui.style())
                .inner_margin(16.0)
                .show(ui, |ui| {
                    ui.set_min_width(320.0);
                    ui.label(RichText::new(kind.prompt()).strong());
                    ui.add_space(12.0);
                    ui.horizontal(|ui| {
                        if ui.button(kind.confirm_label()).clicked() {
                            confirmed = Some(true);
                        }
                        if ui.button("取消").clicked() {
                            confirmed = Some(false);
                        }
                    });
                });
        });
    match confirmed {
        Some(true) => {
            state.confirm = None;
            match kind {
                Confirm::ClearHistory => effects.clear_history = true,
                Confirm::ClearKey => effects.delete_key = true,
            }
        }
        Some(false) => state.confirm = None,
        None => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::{KeyState, Tone};
    use ice_core::config::Config;
    use ice_core::glossary::Glossary;

    fn state() -> AppState {
        AppState::new(
            Config::default(),
            Glossary::default(),
            KeyState::Missing,
            None,
        )
    }

    /// 无头跑 `frames` 帧。`ctx.run_ui` 交出来的就是 eframe 在 `App::ui` 里给的那个根 `Ui`，
    /// 所以 view 的绘制路径与真机完全一致；`screen_rect` 用来覆盖窄宽布局。
    /// 控件矩形是画完那一帧才记上的，要读它至少跑两帧。
    fn run_frame(
        state: &mut AppState,
        effects: &mut Effects,
        history: &HistoryStore,
        screen: egui::Vec2,
        frames: usize,
    ) -> egui::Context {
        let ctx = egui::Context::default();
        let mut time = 0.0;
        for _ in 0..frames {
            let mut raw = egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, screen)),
                ..Default::default()
            };
            raw.time = Some(time);
            time += 1.0 / 60.0;
            let output = ctx.run_ui(raw, |ui| {
                show(ui, state, effects, history, false);
            });
            output.drop_without_applying_deltas();
        }
        ctx
    }

    fn temp_history() -> (tempfile::TempDir, HistoryStore) {
        let dir = tempfile::tempdir().unwrap();
        let store = HistoryStore::at(dir.path().join("history.jsonl"));
        (dir, store)
    }

    #[test]
    fn every_tab_renders_without_panicking_at_the_minimum_size() {
        let (_dir, store) = temp_history();
        for tab in Tab::all() {
            let mut state = state();
            state.tab = tab;
            let mut effects = Effects::default();
            run_frame(
                &mut state,
                &mut effects,
                &store,
                egui::vec2(640.0, 420.0),
                1,
            );
            assert_eq!(state.tab, tab, "渲染过程不该把页签改掉");
            assert!(effects.job.is_none(), "切到 {tab:?} 不该发起请求");
        }
    }

    /// 无头取出这一帧画布上的全部文字。位置类断言读控件矩形就够，**标签**类断言只能读这个
    /// ——`run_frame` 把 output 丢了，这里单独跑一遍不丢。
    fn drawn_texts(
        state: &mut AppState,
        effects: &mut Effects,
        store: &HistoryStore,
    ) -> Vec<String> {
        let ctx = egui::Context::default();
        let screen = egui::vec2(1080.0, 720.0);
        let mut shapes: Vec<egui::epaint::ClippedShape> = Vec::new();
        for frame in 0..2 {
            let mut raw = egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, screen)),
                ..Default::default()
            };
            raw.time = Some(frame as f64 / 60.0);
            let mut output = ctx.run_ui(raw, |ui| show(ui, state, effects, store, false));
            // 形状在这一帧的 output 里，`drop_without_applying_deltas` 之前要先把它们拿走
            shapes = std::mem::take(&mut output.shapes);
            output.drop_without_applying_deltas();
        }
        shapes
            .iter()
            .filter_map(|clipped| match &clipped.shape {
                egui::Shape::Text(text) => Some(text.galley.text().to_string()),
                _ => None,
            })
            .collect()
    }

    /// 用户 2026-11-03："还有悬浮窗三个字，改成精简两个字。"
    ///
    /// 那颗按钮的位置由 `the_pin_sits_left_of_the_floating_button_and_sets_the_same_intent`
    /// 锁着，这一条锁的是它**写着什么**：旧名字不许在任何一处残留（改判不是加别名）。
    #[test]
    fn the_mode_button_is_labeled_with_the_new_name_only() {
        let (_dir, store) = temp_history();
        let mut state = state();
        let mut effects = Effects::default();
        let texts = drawn_texts(&mut state, &mut effects, &store);
        let joined = texts.concat();

        assert!(
            texts.iter().any(|text| text.trim() == "精简"),
            "工具条上那颗形态按钮没写着「精简」：{texts:?}"
        );
        assert!(!joined.contains("悬浮窗"), "界面上还留着旧名字：{texts:?}");
    }

    /// 用户 2026-11-03："主题颜色切换图标除设置页都应该有" + 次日"还是放在最下方吧，和状态栏一行"。
    ///
    /// 两句合起来才是现在这条判据：三页都要有，而且都在**同一行**（状态栏），所以 salt 也只有一个
    /// `status`——原来三条宿主栏各一个 salt（`full`/`image`/`history`）是为了证明"每页自己那条栏里
    /// 画了一颗"，现在那条证明不再存在，改判据读同一格、逐页跑一帧。
    /// 设置页是反向的：那一页自己有「主题」那一行，再画一颗就是两个入口并排打架。
    #[test]
    fn the_theme_switch_is_drawn_on_every_tab_but_settings() {
        let (_dir, store) = temp_history();
        let screen = egui::vec2(900.0, 620.0);
        for tab in [Tab::Text, Tab::Image, Tab::History] {
            let mut state = state();
            state.tab = tab;
            let ctx = run_frame(&mut state, &mut Effects::default(), &store, screen, 2);
            let rect = rect_slot(&ctx, "theme", "status")
                .unwrap_or_else(|| panic!("{tab:?} 这一页的状态栏没画主题滑块"));
            assert!(
                rect.width() >= THEME_SWITCH_W - 0.5,
                "{tab:?} 的滑块被状态栏压窄了：{rect:?}"
            );
        }

        let mut state = state();
        state.tab = Tab::Settings;
        let ctx = run_frame(&mut state, &mut Effects::default(), &store, screen, 2);
        assert!(
            rect_slot(&ctx, "theme", "status").is_none(),
            "设置页又画了第二颗主题滑块，与那一行的「主题」并排"
        );
    }

    /// 用户 2026-11-04 那两句一起锁在这里：
    /// ① "历史没有这一行，如果放在同样的位置就很突兀，所以还是放在最下方吧，和状态栏一行"
    ///    ——判据是滑块落在状态栏那一格**之内**，同时**不在**文本页动作栏那一格之内（两条栏不重叠，
    ///    状态栏在动作栏下面，所以两个断言合起来才排除"还挂在动作栏右端"）。
    /// ② "边框是圆角的，而且边框内部边上好像还有一圈看不见颜色的空气墙，主题切换按钮太贴边会导致
    ///    浅色状态下，按钮会被覆盖一部分……向窗口左侧挪动3-5px"——判据是它的右端离状态栏内容区
    ///    右边缘正好让开 `THEME_EDGE_INSET`，不是"随便有点空隙"。
    /// ③ 老的点击判据跟着搬过来：点轨道左端（盖住太阳那一头）→ 深色 + 标脏。
    #[test]
    fn the_theme_switch_sits_in_the_status_row_and_offsets_from_the_edge() {
        let (_dir, store) = temp_history();
        let mut state = state();
        state.config.ui.theme = ice_core::config::Theme::System;
        let mut effects = Effects::default();
        let screen = egui::vec2(1080.0, 720.0);
        let ctx = run_frame(&mut state, &mut effects, &store, screen, 2);
        let band = recorded_rect(&ctx, "status", "band");
        let switch = recorded_rect(&ctx, "theme", "status");
        let (action_bar, _) = full::recorded_bar(&ctx);

        assert!(
            switch.min.y >= band.min.y - 0.5 && switch.max.y <= band.max.y + 0.5,
            "滑块不在状态栏那一行：{switch:?}（状态栏 {band:?}）"
        );
        assert!(
            switch.min.y >= action_bar.max.y - 0.5,
            "滑块还留在文本页动作栏那一行：{switch:?}（动作栏 {action_bar:?}）"
        );
        assert!(
            (band.max.x - THEME_EDGE_INSET - switch.max.x).abs() <= 0.5,
            "滑块没在右边缘让开 {THEME_EDGE_INSET}px：{switch:?}（状态栏 {band:?}）"
        );
        assert!(
            switch.width() >= THEME_SWITCH_W - 0.5,
            "滑块被状态栏压窄了：{switch:?}"
        );

        state.config.ui.theme = ice_core::config::Theme::System;
        state.config_dirty = false;
        run_click(
            &mut state,
            &mut effects,
            &store,
            screen,
            egui::pos2(switch.min.x + 2.0, switch.center().y),
        );
        assert_eq!(
            state.config.ui.theme,
            ice_core::config::Theme::Dark,
            "在状态栏那一点的滑块左端点下去，没切到深色"
        );
        assert!(state.config_dirty, "改了主题没置脏标记，配置不会落盘");
    }

    /// 用户 2026-11-04 看了让开窗口边缘那一版之后："还是没有完全显示，怎么回事"。
    ///
    /// 没治好是因为**病因找错了**。拿他那张 74×31 的截图逐像素量：轨道右端在 x=59，窗口右边框
    /// 在 x=73，中间空着 14px（8 面板内边距 + 4 让位 + 边框），窗口边缘根本没碰到它；被盖掉的
    /// 是轨道**自己**那半个圆帽——滑块圆心原来贴着端头内缩一个半径，于是滑块最外沿正好落在轨道
    /// 轮廓线上，而它的描边 1.5px 是居中的，等于又顶出去 0.75px。浅色下轨道只剩一圈线（实测底色
    /// 249、线 226），那圈线在端头被滑块的同色描边打断，看着就是"按钮缺了一角"。
    ///
    /// 判据因此是几何而不是观感：三档里滑块连描边一起都得待在轨道那一格**内部**，
    /// 两端各留出一截看得见的轨道。
    #[test]
    fn the_knob_stays_inside_the_track_at_both_ends() {
        let rect = egui::Rect::from_min_size(
            egui::pos2(120.0, 40.0),
            egui::vec2(THEME_SWITCH_W, THEME_SWITCH_H),
        );
        // 滑块那圈描边宽 1.5、画在圆周居中，所以外沿要算上半个
        let half_stroke = 0.75;
        let reach = KNOB_RADIUS + half_stroke;

        for theme in [
            ice_core::config::Theme::Dark,
            ice_core::config::Theme::System,
            ice_core::config::Theme::Light,
        ] {
            let knob = egui::Rect::from_center_size(
                egui::pos2(knob_center_x(rect, theme), rect.center().y),
                egui::vec2(2.0 * reach, 2.0 * reach),
            );
            assert!(
                rect.contains_rect(knob),
                "{theme:?} 档滑块连描边一起顶出了轨道那一格：{knob:?} 不在 {rect:?} 内——\
                 轨道端头那半个圆帽会被盖掉，浅色下看着就是按钮缺了一角"
            );
        }
    }

    /// 用户 2026-11-04："精简模式下，窗口小的时候，这个月亮右上角有个斜杠，拉大一点就不见了，
    /// 你看看这两张图的微小区别"。两张截图逐像素量出来，斜杠**一直都在**，只是长短随亚像素相位变：
    /// 窄窗（442px）那日从尖端 (422, 245) 向右下延伸到 (428, 248)，亮度 214，和笔画本身同色；
    /// 宽窗（500px）只到 (485, 329)，亮度 115，看着就"没了"。
    ///
    /// 病因是 `closed_line`：月牙外弧与内弧在尖端只夹 27°，闭合成环后那一点是 152.8° 的发卡弯，
    /// epaint 在那里做 miter join（`tessellator.rs:432`，法线 `(n0+n1)/2` 再除以长度平方），
    /// 放大 4.25 倍，再叠同向 feather，就甩出一条沿角平分线的亮线。
    ///
    /// 所以这条锁三件事：① 那一格里是**两条**折线（外弧 + 内弧），不是一条环；② 两条都不 `closed`；
    /// ③ 相邻段转角不超过 30°（外弧每段才转 12.1°，一旦有人把尖端接回成发卡弯，这个数直接飙到 152°），
    /// 并且所有点都落在月亮自己的 8.5px 半径内——斜杠那种 9.5px 的外甩藏不住。
    #[test]
    fn the_moon_is_two_open_arcs_so_its_tip_cannot_spike() {
        /// 相邻两段折线之间的最大转角（度）；零长段跳过，免得 `normalized()` 出 NaN。
        fn max_turn_deg(points: &[egui::Pos2]) -> f32 {
            points
                .windows(3)
                .filter_map(|w| {
                    let (a, b) = (w[1] - w[0], w[2] - w[1]);
                    (a.length_sq() > 1e-6 && b.length_sq() > 1e-6).then(|| {
                        a.normalized()
                            .dot(b.normalized())
                            .clamp(-1.0, 1.0)
                            .acos()
                            .to_degrees()
                    })
                })
                .fold(0.0, f32::max)
        }

        let (_dir, store) = temp_history();
        let mut state = state();
        let mut effects = Effects::default();
        let screen = egui::vec2(1080.0, 720.0);
        let (ctx, shapes) = run_shapes(&mut state, &mut effects, &store, screen);
        let switch = recorded_rect(&ctx, "theme", "status");
        let moon = egui::pos2(
            knob_center_x(switch, ice_core::config::Theme::Light),
            switch.center().y,
        );

        let arcs: Vec<&egui::epaint::PathShape> = shapes
            .iter()
            .filter_map(|clipped| match &clipped.shape {
                egui::Shape::Path(path)
                    if switch.contains(path.visual_bounding_rect().center()) =>
                {
                    Some(path)
                }
                _ => None,
            })
            .collect();

        assert_eq!(
            arcs.len(),
            2,
            "月亮该由外弧+内弧两条开放折线画成，实得 {} 条：{:?}",
            arcs.len(),
            arcs.iter()
                .map(|p| (p.closed, p.points.len()))
                .collect::<Vec<_>>()
        );
        for path in &arcs {
            assert!(
                !path.closed,
                "月牙又闭合成环了——尖端那个 152.8° 发卡弯会被 miter join 甩出一道斜杠"
            );
            let turn = max_turn_deg(&path.points);
            assert!(
                turn <= 30.0,
                "折线里藏着一个 {turn:.1}° 的急转弯，epaint 会在那里放大法线画出多余的线：{:?}",
                path.points.first()
            );
            let far = path
                .points
                .iter()
                .map(|p| p.distance(moon))
                .fold(0.0, f32::max);
            assert!(
                far <= 8.5,
                "月牙画到了离圆心 {far:.1}px 的地方（半径 7 + 半个笔画），超出月亮自己那一圈"
            );
        }
    }

    /// 用户 2026-10-28 报的两条缺陷一起锁在这里，量的是 `views::show` 这条真机路径：
    /// ① "按钮上下留白距离不同"——底栏画在 `CentralPanel` 里时被那层 8px 下内边距顶离窗口
    /// 底边（实测上 7、下 15），挪到窗口层后必须贴住状态条上沿、横向满窗；
    /// ② "这颜色怎么回事"——挪走之后它下面不再有 `panel_fill` 兜着，`Frame::NONE` 就等于
    /// 在窗口上开个透出桌面的洞（eframe 默认 `clear_color` 是半透明深灰）。
    #[test]
    fn the_text_page_bottom_bar_hugs_the_status_bar() {
        let (_dir, store) = temp_history();
        let mut state = state();
        let mut effects = Effects::default();
        let screen = egui::vec2(1080.0, 720.0);
        let ctx = run_frame(&mut state, &mut effects, &store, screen, 2);
        let (bar, fill) = full::recorded_bar(&ctx);

        let status_top = screen.y - STATUS_HEIGHT;
        assert!(
            (bar.max.y - status_top).abs() <= 0.5,
            "底栏与状态条之间空出 {:.1}px，按钮的上下留白就差这么多：{bar:?}",
            status_top - bar.max.y
        );
        assert!(
            bar.min.x <= 0.5 && bar.max.x >= screen.x - 0.5,
            "底栏没横满整窗（{bar:?}），左右各被页框内边距切掉一块"
        );
        let style = ctx.global_style();
        assert_eq!(
            fill, style.visuals.panel_fill,
            "底栏没自己画装饰条的底色，那条区域会透出桌面（D-12）"
        );
        assert_eq!(fill.a(), 255, "装饰条的底色必须不透明：{fill:?}");
    }

    /// 另一件只量"画没画"的事：把最后一帧的绘制清单拿出来。
    /// 现有的 `run_frame` 只交回 `Context`（控件矩形是记在临时数据里的），而边框线是形状，
    /// 只能从 shapes 读——所以这条单独一个入口，不去改动上面那些测试。
    fn run_shapes(
        state: &mut AppState,
        effects: &mut Effects,
        history: &HistoryStore,
        screen: egui::Vec2,
    ) -> (egui::Context, Vec<egui::epaint::ClippedShape>) {
        let ctx = egui::Context::default();
        let mut time = 0.0;
        let mut shapes = Vec::new();
        for _ in 0..2 {
            let mut raw = egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, screen)),
                ..Default::default()
            };
            raw.time = Some(time);
            time += 1.0 / 60.0;
            let mut output = ctx.run_ui(raw, |ui| {
                show(ui, state, effects, history, false);
            });
            shapes = std::mem::take(&mut output.shapes);
            output.drop_without_applying_deltas();
        }
        (ctx, shapes)
    }

    /// 工具条与正文的分界**只有一条线**，而且与底栏那条同源：两者都由 `Panel` 自己画
    /// （`show_separator_line` 默认开，取 `widgets.noninteractive.bg_stroke`）。
    ///
    /// 用户 2026-10-28 对着截图退回的原话："我看到你改动的菜单栏界限了，太明显了，突兀，
    /// 和底部栏的分割线一样颜色就行，而且两侧不需要线条。"所以这条锁三件事：顶区有一条
    /// 横贯整窗的分隔线、它的颜色等于底栏那条、顶区没有第二圈边框。
    /// 前一行"界限不明显"的真凶是栏高囤积（82px），那条由 `the_toolbar_takes_one_row_...` 管。
    #[test]
    fn the_toolbar_separator_matches_the_bottom_bar_and_has_no_frame() {
        let (_dir, store) = temp_history();
        let mut state = state();
        let mut effects = Effects::default();
        let screen = egui::vec2(1080.0, 720.0);
        let (ctx, shapes) = run_shapes(&mut state, &mut effects, &store, screen);

        // 横贯整窗的水平线段：(所在高度, 线色)
        let mut lines: Vec<(f32, egui::Color32)> = shapes
            .iter()
            .filter_map(|clipped| match &clipped.shape {
                egui::Shape::LineSegment { points, stroke } => {
                    let (a, b) = (points[0], points[1]);
                    (a.y == b.y && a.x <= 0.5 && b.x >= screen.x - 0.5)
                        .then_some((a.y, stroke.color))
                }
                _ => None,
            })
            .collect();
        lines.sort_by(|a, b| a.0.total_cmp(&b.0));

        let top: Vec<_> = lines.iter().filter(|(y, _)| *y < 100.0).collect();
        assert_eq!(top.len(), 1, "工具条下沿该只有一条分隔线，实得 {lines:?}");
        let bottom: Vec<_> = lines
            .iter()
            .filter(|(y, _)| *y > screen.y - 120.0)
            .collect();
        assert!(
            !bottom.is_empty(),
            "底栏那条分隔线没找到，无法比对：{lines:?}"
        );
        assert_eq!(
            top[0].1, bottom[0].1,
            "两条分隔线颜色不同——工具条自己另画了一色"
        );
        assert_eq!(
            top[0].1,
            ctx.global_style()
                .visuals
                .widgets
                .noninteractive
                .bg_stroke
                .color,
            "分隔线必须走 egui 那一处出处，别再各配一套色"
        );

        let edge = theme::edge_color(ctx.global_style().visuals.dark_mode);
        let framed = shapes.iter().any(|clipped| match &clipped.shape {
            egui::Shape::Rect(rect) => {
                rect.stroke.color == edge
                    && rect.stroke.width >= 1.0
                    && rect.rect.min.x <= 0.5
                    && rect.rect.max.x >= screen.x - 0.5
                    && rect.rect.min.y <= 1.0
            }
            _ => false,
        });
        assert!(!framed, "工具条又套上一圈整窗边框了：两侧不该有线");
    }

    /// 用户 2026-10-28："明明只占用一行，但是好远"。以前工具条 `exact_size(56)` 钉死，
    /// 里面只有一行约 20px 的控件，正文框上沿被顶到 **82px**（实测，把 `exact_size` 加回去
    /// 跑这条就能看到）。改成按内容自适应之后是 61px：栏高 = 一行 + 上下各 6px。
    /// 这条锁的是"栏不再囤空转"，阈值给 64px：留 3px 给字形度量的零头，但 56 那一档必然过不了。
    #[test]
    fn the_toolbar_takes_one_row_so_the_panes_sit_right_under_it() {
        let (_dir, store) = temp_history();
        let mut state = state();
        let mut effects = Effects::default();
        let ctx = run_frame(
            &mut state,
            &mut effects,
            &store,
            egui::vec2(1080.0, 720.0),
            2,
        );
        let (source, _) = full::recorded_pane(&ctx, "source");
        assert!(
            source.min.y <= 64.0,
            "正文框上沿跑到 {:.1}px，工具条又囤出空转了（钉死 56px 那一版是 82px）",
            source.min.y
        );
    }

    /// 图钉没有文字，能教用户那条快捷键的只剩悬停提示——所以**两态都必须带上它**。
    /// 原生那颗的同一件事由 `tip_for` 承担，模块删了，这条纪律跟着挪到这里。
    #[test]
    fn the_pin_tip_names_the_shortcut_in_both_states() {
        assert!(
            pin_tip(true).contains("Cmd+Shift+P") && pin_tip(false).contains("Cmd+Shift+P"),
            "图钉的悬停提示把快捷键的知识弄丢了"
        );
        assert_ne!(
            pin_tip(true),
            pin_tip(false),
            "两态提示一字不差，用户点下去之前看不出会往哪边切"
        );
    }

    /// 用户 2026-10-29 走查后指定的位置："全量窗挪动到精简（悬浮窗）两个字左侧"。
    /// 那条带子是 `right_to_left`，那颗形态按钮先占右边缘，图钉落在它左边才叫"左侧"；
    /// 置顶那条通道一个字没改——点的仍是 `effects.toggle_topmost`，与 `Cmd+Shift+P` 同一个字段。
    #[test]
    fn the_pin_sits_left_of_the_floating_button_and_sets_the_same_intent() {
        let (_dir, store) = temp_history();
        let mut state = state();
        let mut effects = Effects::default();
        let screen = egui::vec2(1080.0, 720.0);
        let ctx = run_frame(&mut state, &mut effects, &store, screen, 2);
        let floating = recorded_rect(&ctx, "toolbar", "floating");
        let pin = recorded_pin(&ctx, "full");

        assert!(
            pin.max.x <= floating.min.x + 0.5,
            "图钉没有落在「精简」左侧：{pin:?}（按钮 {floating:?}）"
        );
        assert!(
            pin.width() >= PIN_TOGGLE_W - 0.5,
            "图钉那一格被工具条压窄了：{pin:?}"
        );

        effects = Effects::default();
        run_click(
            &mut state,
            &mut effects,
            &store,
            screen,
            recorded_pin(&ctx, "full").center(),
        );
        assert!(
            effects.toggle_topmost,
            "点了图钉没有置置顶意图——置顶那条通道断在这里"
        );
        assert!(
            !effects.toggle_mode,
            "点图钉顺带把形态也切了，两块控件的响应区重叠了"
        );
    }

    #[test]
    fn rendering_does_not_invent_requests_or_side_effects() {
        let (_dir, store) = temp_history();
        let mut state = state();
        let mut effects = Effects::default();
        run_frame(
            &mut state,
            &mut effects,
            &store,
            egui::vec2(1080.0, 720.0),
            1,
        );
        assert!(effects.job.is_none());
        assert!(effects.copy.is_none());
        assert!(effects.save_key.is_none());
        assert!(!effects.delete_key);
        assert!(!effects.toggle_mode);
        assert!(!effects.start_drag);
        assert!(!effects.open_settings);
        assert!(!effects.toggle_topmost);
    }

    #[test]
    fn tab_state_survives_a_round_trip_through_every_tab() {
        let (_dir, store) = temp_history();
        let mut state = state();
        state.source = "hello".into();
        state.history_query = "关键词".into();
        state.term_source = "pod".into();
        let mut effects = Effects::default();

        for tab in Tab::all() {
            state.tab = tab;
            run_frame(
                &mut state,
                &mut effects,
                &store,
                egui::vec2(1080.0, 720.0),
                1,
            );
        }

        assert_eq!(state.source, "hello");
        assert_eq!(state.history_query, "关键词");
        assert_eq!(state.term_source, "pod");
        assert!(effects.job.is_none());
    }

    #[test]
    fn toast_is_queued_and_expires_on_its_own() {
        let mut state = state();
        state.toast(Tone::Info, "已复制");
        assert_eq!(state.toasts.len(), 1);
        assert!(!state.tick_toasts());
        assert_eq!(state.toasts.len(), 1);
        // 直接把时间戳拨过整条 lifetime，免得测试真等那一秒
        state.toasts[0].shown_at -= crate::state::TOAST_LIFETIME * 2;
        assert!(state.tick_toasts());
        assert!(state.toasts.is_empty());
    }

    /// 变更 I 的前半句："所有的提示类弹窗都是中心弹窗"（用户 2026-10-28）。
    /// 判据落在实测矩形上：提示条那一带的中心就是窗口中心——横竖都居中，
    /// 不再贴顶 12px，也不再为简洁窗另算一套"越过输入行"的偏移。
    #[test]
    fn toasts_are_painted_at_the_window_centre() {
        let (_dir, store) = temp_history();
        let mut state = state();
        state.toast(Tone::Info, "已复制到剪贴板");
        let mut effects = Effects::default();
        let screen = egui::vec2(1080.0, 720.0);
        let ctx = run_frame(&mut state, &mut effects, &store, screen, 2);

        let band = recorded_toast_band(&ctx).expect("提示条那一带要记录下来");
        let centre = band.center();
        let want = egui::pos2(screen.x / 2.0, screen.y / 2.0);
        assert!(
            centre.distance(want) <= 1.0,
            "提示条没落在窗口正中：量到 {centre:?}，期望 {want:?}（带子 {band:?}）"
        );
    }

    /// 变更 I 的后半句："不应该有关闭的操作和按钮"。
    /// 这里不数按钮，量的是**行为**：把鼠标移到提示条正中间按下再松开，那条还在——
    /// 提示条只剩"自己到点淡出"这一种结束方式。将来谁再补一颗 `×`，这条会红。
    #[test]
    fn a_click_on_a_toast_does_not_dismiss_it() {
        let (_dir, store) = temp_history();
        let mut state = state();
        state.toast(Tone::Info, "已复制到剪贴板");
        let mut effects = Effects::default();
        let screen = egui::vec2(1080.0, 720.0);
        let ctx = run_frame(&mut state, &mut effects, &store, screen, 2);
        let at = recorded_toast_band(&ctx)
            .expect("先量到提示条那一带")
            .center();

        run_click(&mut state, &mut effects, &store, screen, at);
        assert_eq!(
            state.toasts.len(),
            1,
            "点在提示条上就把它点掉了——变更 I 要求提示类弹窗没有关闭操作"
        );
    }

    /// `run_frame` 只喂空事件，这里要的是"真的点一下"，所以单独一个入口
    ///（与 `run_shapes` 同一先例：不为一条测试去改大家都用的那个）。
    fn run_click(
        state: &mut AppState,
        effects: &mut Effects,
        history: &HistoryStore,
        screen: egui::Vec2,
        at: egui::Pos2,
    ) -> egui::Context {
        let ctx = egui::Context::default();
        let mut time = 1.0;
        let steps = [
            vec![],
            vec![egui::Event::PointerMoved(at)],
            vec![egui::Event::PointerButton {
                pos: at,
                button: egui::PointerButton::Primary,
                pressed: true,
                modifiers: egui::Modifiers::NONE,
            }],
            vec![egui::Event::PointerButton {
                pos: at,
                button: egui::PointerButton::Primary,
                pressed: false,
                modifiers: egui::Modifiers::NONE,
            }],
        ];
        for events in steps {
            let mut raw = egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, screen)),
                ..Default::default()
            };
            raw.time = Some(time);
            time += 1.0 / 60.0;
            raw.events = events;
            let output = ctx.run_ui(raw, |ui| {
                show(ui, state, effects, history, false);
            });
            output.drop_without_applying_deltas();
        }
        ctx
    }

    #[test]
    fn at_most_three_toasts_are_kept() {
        let mut state = state();
        for index in 0..5 {
            state.toast(Tone::Info, format!("第 {index} 条"));
        }
        assert_eq!(state.toasts.len(), 3);
        assert_eq!(state.toasts[0].text, "第 2 条");
    }
}
