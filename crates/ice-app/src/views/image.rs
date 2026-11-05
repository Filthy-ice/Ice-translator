//! 图片页（CHG-005 + CHG-006）：E 导入与四条动作、F 原图预览（识别框可点）、G 成品预览、
//! H 可编辑识别结果与译文列、I 三段状态与那颗共用「停止」。
//!
//! 每一颗控件都立刻改到 `AppState` 那份文档上，而那份文档正是要拿去翻译与合成的那一份
//! （REQ-005 的判据就落在这里）；翻译段逐块写回同一份文档的 `translation`，合成段读它出一张成品，
//! 两条出路（保存 / 复制图片）再把那张成品交出去。
//!
//! 视图不碰系统：面板、剪贴板读写图、拖拽落地统统记成 `Effects`，帧末由 `app.rs` 执行；
//! 识别、逐块翻译、合成三段派发都走 `Effects::job` 那同一条口。本页对 `system` 的唯一依赖是
//! 两句**文案常量**（Q-004 那两句要说的事只有一处出处），没有一次系统调用。

use egui::{self, Color32, ColorImage, Id, RichText, Sense, TextureOptions, Ui};
use ice_core::document::{OcrDocument, PixelRect};
use ice_vision::composite::Composed;
use ice_vision::typeset::CompositeMode;
use ice_vision::{Admitted, LOW_CONFIDENCE_WARN};
use image::RgbaImage;

use crate::state::{AppState, ComposeStage, Phase, RecognitionStage, Split, Tone, TranslateStage};
use crate::system::clipboard;
use crate::theme;
use crate::views::Effects;

/// F 区与 G 区预览的最长边（像素）。入场后的图最大 8000px，画进这两格前先等比降下来：
/// 预览只用来"看个位置和对不对得上"，不需要像素级保真。
const PREVIEW_MAX_EDGE: u32 = 320;

/// F 那一格能给的高度（宽由 `ui.columns` 分一半，随窗口变）。
const PREVIEW_MAX_HEIGHT: f32 = 220.0;

/// H 那一格没有结果时的最低高度：列表一空就塌成两行，整页会跟着跳。
const LIST_MIN_HEIGHT: f32 = 96.0;

/// 画法单选三档，与 [`CompositeMode`] 一一对应。顺序就是界面上的顺序。
const MODES: &[(CompositeMode, &str, &str)] = &[
    (
        CompositeMode::Auto,
        "自动",
        "逐块看背景：干净底色就地回填，花背景铺底板覆盖",
    ),
    (
        CompositeMode::ReplaceOnly,
        "就地回填",
        "强制擦掉原字再写译文；放不下就允许越框，不裁字",
    ),
    (
        CompositeMode::OverlayAll,
        "铺底板",
        "一个像素都不擦，在块框上盖一块底板再写译文",
    ),
];

/// 「保存」那颗的悬停提示。三件事各半句：格式由扩展名定、位置由用户在系统面板里选、
/// 存的是**当前 G 区那一张**（合成在飞时那一格显示的是上一张成品，门也照样开着——
/// 所见即所存，见 [`AppState::export_gate`]）。
const SAVE_HINT: &str = "存成 PNG 或 JPEG（扩展名决定格式）；存的是 G 区现在这一张";

pub fn show(ui: &mut egui::Ui, state: &mut AppState, effects: &mut Effects, supported: bool) {
    ui.spacing_mut().item_spacing.y = 8.0;
    import_bar(ui, state, effects, supported);
    action_bar(ui, state, effects);
    egui::ScrollArea::vertical()
        .auto_shrink([false, false])
        .show(ui, |ui| {
            preview_row(ui, state);
            block_list(ui, state, effects);
        });
}

/// 平台没有本地识别时给四颗入口的那句话（REQ-007）。
///
/// 说清是"这台机器不行"而不是"这个版本没做"——入口置灰还在原地的全部意义就在这里。
const UNSUPPORTED_HINT: &str = "当前平台没有本地文字识别；文本翻译不受影响";

/// E 区：四条入口汇进同一次派发（REQ-001）。
///
/// 「拖拽」那一格不是按钮——整个窗口都是拖放区，落地在 `app.rs` 的帧循环里读
/// `dropped_files`，这里只写一句怎么用。
fn import_bar(ui: &mut egui::Ui, state: &mut AppState, effects: &mut Effects, supported: bool) {
    egui::Frame::group(ui.style())
        .inner_margin(8.0)
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                let open = ui.add_enabled(supported, egui::Button::new("打开图片"));
                record(ui.ctx(), "open", open.rect);
                if open.clicked() {
                    effects.open_panel = true;
                }
                open.on_hover_text(if supported {
                    "选一张本地图，全程不上传"
                } else {
                    UNSUPPORTED_HINT
                });
                let paste = ui.add_enabled(supported, egui::Button::new("粘贴"));
                record(ui.ctx(), "paste", paste.rect);
                if paste.clicked() {
                    effects.paste_image = true;
                }
                paste.on_hover_text(if supported {
                    "用剪贴板里的图片（Cmd+C 复制的那张）"
                } else {
                    UNSUPPORTED_HINT
                });
                ui.colored_label(theme::MUTED, "或把图片直接拖到窗口里");
            });
            ui.horizontal(|ui| {
                ui.colored_label(theme::MUTED, "路径");
                let response = ui.add(
                    egui::TextEdit::singleline(&mut state.image_path_input)
                        .hint_text("例如 ~/Desktop/shot.png")
                        .desired_width(ui.available_width() - 64.0),
                );
                // 手输路径的人十有八九直接按回车，不给这条路等于逼他用鼠标再点一次
                let entered =
                    response.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                let submit = ui.add_enabled(supported, egui::Button::new("识别"));
                record(ui.ctx(), "submit", submit.rect);
                let pressed = submit.clicked();
                submit.on_hover_text(if supported {
                    "对这一张图跑一次本地识别"
                } else {
                    UNSUPPORTED_HINT
                });
                if supported && (pressed || entered) {
                    if let Some(job) = state.submit_image_path() {
                        effects.request(Some(job));
                        state.image_path_input.clear();
                    }
                }
            });
        });
}

/// E 区第二排：两条后续动作 + 画法单选（CHG-006 REQ-001 / REQ-006）。
///
/// 前置条件全在状态层（`translate_gate` / `compose_gate`），界面只做两件事：灰掉、把那句话说出来。
/// **禁用而不是消失**（REQ-007 的口径）：入口留在原地，用户才知道这一步是"还没到"，不是"没做"。
fn action_bar(ui: &mut egui::Ui, state: &mut AppState, effects: &mut Effects) {
    let translate_blocker = state.translate_gate();
    let compose_blocker = state.compose_gate();
    let export_blocker = state.export_gate();
    // 台账为空时这一颗说的是"没有可重试的"；否则跟着翻译段走（在飞就等一等的说法一样）
    let retry_blocker = if state.failed_blocks.is_empty() {
        Some("这一批没有失败块")
    } else {
        translate_blocker
    };
    egui::Frame::group(ui.style())
        .inner_margin(8.0)
        .show(ui, |ui| {
            ui.horizontal_wrapped(|ui| {
                let translate =
                    ui.add_enabled(translate_blocker.is_none(), egui::Button::new("翻译全部"));
                record(ui.ctx(), "translate", translate.rect);
                if translate.clicked() {
                    effects.request(state.submit_image_translate(&[]));
                }
                translate.on_hover_text(
                    translate_blocker.unwrap_or("一块一次请求，一块失败不牵连别块；勾掉的不翻"),
                );

                let compose =
                    ui.add_enabled(compose_blocker.is_none(), egui::Button::new("生成图片"));
                record(ui.ctx(), "compose", compose.rect);
                if compose.clicked() {
                    effects.request(state.submit_image_compose());
                }
                compose.on_hover_text(
                    compose_blocker
                        .unwrap_or("擦原字、画译文，全在内存里的副本上做；原图一个字节都不动"),
                );

                let retry =
                    ui.add_enabled(retry_blocker.is_none(), egui::Button::new("重试失败块"));
                record(ui.ctx(), "retry", retry.rect);
                if retry.clicked() {
                    effects.request(state.submit_image_retry_failed());
                }
                retry.on_hover_text(retry_blocker.unwrap_or("只重发失败的那几块，已翻好的不动"));

                // 两条出路（REQ-008）共用一道门：手上得有一张成品。写出去的动作都在帧末
                // （面板 / 剪贴板），这里只记意图，所以这两颗在本页是无头可测的。
                let save = ui.add_enabled(export_blocker.is_none(), egui::Button::new("保存"));
                record(ui.ctx(), "save", save.rect);
                if save.clicked() {
                    effects.save_image = true;
                }
                save.on_hover_text(export_blocker.unwrap_or(SAVE_HINT));

                let copy = ui.add_enabled(export_blocker.is_none(), egui::Button::new("复制图片"));
                record(ui.ctx(), "copy_image", copy.rect);
                if copy.clicked() {
                    effects.copy_image = true;
                }
                // Q-004：那句"会覆盖剪贴板里的文本"必须在点下去**之前**看得见，
                // 所以它是 hover 的正文，不是复制成功之后才补的一句
                copy.on_hover_text(export_blocker.unwrap_or(clipboard::COPY_IMAGE_HOVER));

                ui.colored_label(theme::MUTED, "画法");
                for (mode, name, hint) in MODES {
                    let chosen = ui.radio_value(&mut state.compose_mode, *mode, *name);
                    record(ui.ctx(), mode_salt(*mode), chosen.rect);
                    chosen.on_hover_text(format!("{hint}；只改下一次生成，不重新识别也不重新翻译"));
                }
            });
        });
}

/// 三档单选各自那格的登记名（无头测试要点哪一档就取哪一格）。
fn mode_salt(mode: CompositeMode) -> &'static str {
    match mode {
        CompositeMode::Auto => "mode_auto",
        CompositeMode::ReplaceOnly => "mode_replace",
        CompositeMode::OverlayAll => "mode_overlay",
    }
}

/// F 原图预览 + G 结果预览，两格同尺寸对齐（`03-ui-design.md` §2）。
fn preview_row(ui: &mut egui::Ui, state: &mut AppState) {
    let image_id = state.image_request_id;
    let compose_id = state.compose_request_id;
    let mut selected: Option<Option<u32>> = None;
    ui.columns(2, |columns| {
        source_preview(&mut columns[0], state, image_id, &mut selected);
        result_preview(&mut columns[1], state, compose_id);
    });
    if let Some(picked) = selected {
        state.selected_block = picked;
    }
}

/// F 区：入场图 + 识别框叠加，点中一个框就把那一块交给 H 区（REQ-007 的"可点选中"）。
///
/// `selected` 是往外带的那一笔：画框与命中测试要借 `state`（只读），写 `selected_block` 得等
/// 借用结束，所以这里只把结论装进出参，落回状态由 [`preview_row`] 做。
fn source_preview(
    ui: &mut egui::Ui,
    state: &AppState,
    request_id: u64,
    selected: &mut Option<Option<u32>>,
) {
    region(ui, PREVIEW_MAX_HEIGHT, |ui| {
        caption(ui, "F 原图预览");
        let Some(admitted) = state.image.as_ref() else {
            ui.colored_label(theme::MUTED, "（还没有图片）");
            return;
        };
        let id = preview_id(admitted, request_id);
        let painted = preview_painted(ui, &admitted.image, id, "ocr_source_preview");
        // 框按**入场图**的像素坐标等比映射到这一格，与纹理同一个缩放基准
        let boxes = recognition_boxes(
            painted,
            (admitted.width, admitted.height),
            state.ocr.as_ref(),
        );
        for (block_id, rect) in &boxes {
            let color = if state.selected_block == Some(*block_id) {
                theme::ACCENT
            } else {
                theme::MUTED
            };
            ui.painter().rect_stroke(
                *rect,
                0.0,
                egui::Stroke::new(1.0, color),
                egui::StrokeKind::Inside,
            );
            record_cell(ui.ctx(), "box", *block_id, *rect);
        }
        let hit = ui.interact(
            painted,
            Id::NULL.with("ice-translator.image-boxes"),
            Sense::click(),
        );
        if hit.clicked() {
            *selected = Some(pick_block(&boxes, hit.hover_pos()));
        }
        ui.colored_label(
            theme::MUTED,
            format!("识别用尺寸：{} × {} px", admitted.width, admitted.height),
        );
        if admitted.scaled {
            // Q-007：缩放不能静默，否则用户会把"图被缩过"当成识别不准的原因
            ui.colored_label(
                theme::WARN,
                RichText::new("已缩放：原图超出 8000px，按等比缩小后识别").strong(),
            );
        }
    });
}

/// G 区：成品预览（REQ-007）。
///
/// 与 F 区同尺寸同一缩放基准；`degraded` 那几块按 spec 的原话描橙（用 `theme::WARN`，
/// 与置信度那条 ⚠ 同一个颜色出处，不再抄一个橙）。
fn result_preview(ui: &mut egui::Ui, state: &AppState, request_id: u64) {
    region(ui, PREVIEW_MAX_HEIGHT, |ui| {
        caption(ui, "G 结果预览");
        let Some(composed) = state.composed.as_ref() else {
            // 还没有成品时说的是"缺哪一步"，而不是空白一格（REQ-006 后半句）
            ui.colored_label(
                theme::MUTED,
                state.compose_gate().unwrap_or("（这一次没有生成成品）"),
            );
            return;
        };
        let id = composed_preview_id(composed, request_id);
        let painted = preview_painted(ui, &composed.image, id, "ocr_result_preview");
        let boxes = recognition_boxes(
            painted,
            (composed.image.width(), composed.image.height()),
            state.ocr.as_ref(),
        );
        for block_id in &composed.degraded_blocks {
            let Some((_, rect)) = boxes.iter().find(|(id, _)| id == block_id) else {
                continue;
            };
            ui.painter().rect_stroke(
                *rect,
                0.0,
                egui::Stroke::new(1.5, theme::WARN),
                egui::StrokeKind::Inside,
            );
        }
        ui.colored_label(
            theme::MUTED,
            format!(
                "{} · 回填 {} 覆盖 {} 跳过 {}",
                composed.mode_label(),
                composed.replaced,
                composed.overlaid,
                composed.skipped
            ),
        );
        if composed.degraded() > 0 {
            ui.colored_label(
                theme::WARN,
                RichText::new(format!(
                    "降级 {} 块（橙框）：缩到字号下限仍放不下",
                    composed.degraded()
                ))
                .strong(),
            );
        }
        // 只报计数：是哪个字不进界面也不进日志（NFR-3）
        if !composed.missing_glyphs.is_empty() {
            let count: usize = composed.missing_glyphs.iter().map(|(_, n)| n).sum();
            ui.colored_label(
                theme::WARN,
                format!("缺字形 {count} 个：本机两种字体都画不出，该处留空"),
            );
        }
    });
}

/// 一张预览图画进这一格之后实际占到的矩形。F 与 G 共用这一条：两格的缩放基准必须一致
/// （REQ-007"同尺寸等比"），各自写一遍缩放迟早会走偏。
///
/// `load_texture_cached` 按像素内容去重，所以每帧都可以放心调；真正贵的那一步（从大图缩出
/// 预览）由 [`preview_pixels`] 自己按同一把 `id` 缓存。
fn preview_painted(ui: &mut egui::Ui, source: &RgbaImage, id: Id, name: &str) -> egui::Rect {
    let ctx = ui.ctx();
    let handle = ctx.load_texture_cached(
        name,
        id,
        preview_pixels(ctx, source, id),
        TextureOptions::LINEAR,
    );
    ui.add(
        egui::Image::from_texture(&handle)
            .max_size(handle.size_vec2() * fit_scale(&handle))
            .maintain_aspect_ratio(true),
    )
    .rect
}

/// 预览的等比缩放比：既不超过 `PREVIEW_MAX_EDGE`/`PREVIEW_MAX_HEIGHT`，也不放大回去。
fn fit_scale(handle: &egui::TextureHandle) -> f32 {
    let size = handle.size_vec2();
    (PREVIEW_MAX_EDGE as f32 / size.x)
        .min(PREVIEW_MAX_HEIGHT / size.y)
        .min(1.0)
}

/// 预览像素：一张图只构建一次。
///
/// 不能每帧现算——8000px 的入场图与同尺寸的成品逐帧缩放能把 CPU 顶到听见风扇声（NFR-2 的空闲
/// 口径先不提，处理中那几秒界面会直接卡住）。缓存键由调用方给，见 [`preview_id`] 与
/// [`composed_preview_id`]。
fn preview_pixels(ctx: &egui::Context, source: &RgbaImage, id: Id) -> ColorImage {
    if let Some(cached) = ctx.data(|data| data.get_temp::<ColorImage>(id)) {
        return cached;
    }
    let small = image::DynamicImage::ImageRgba8(source.clone())
        .thumbnail(PREVIEW_MAX_EDGE, PREVIEW_MAX_EDGE)
        .into_rgba8();
    let size = [small.width() as usize, small.height() as usize];
    let image = ColorImage::from_rgba_unmultiplied(size, small.as_raw());
    ctx.data_mut(|data| data.insert_temp(id, image.clone()));
    image
}

fn preview_id(admitted: &Admitted, request_id: u64) -> Id {
    Id::NULL
        .with("ice-translator.image-preview")
        .with(&admitted.label)
        .with(admitted.width)
        .with(admitted.height)
        .with(admitted.scaled)
        .with(request_id)
}

/// 成品那一格的缓存键。
///
/// `compose_request_id` 必须在键里：成品与原图**同尺寸**，只按尺寸建键的话同一张窗口尺度下
/// 的新成品会被缓存吃掉，用户盯着旧的那张看（REQ-009 的"换代"就白做了）。
fn composed_preview_id(composed: &Composed, request_id: u64) -> Id {
    Id::NULL
        .with("ice-translator.result-preview")
        .with(composed.image.width())
        .with(composed.image.height())
        .with(composed.replaced)
        .with(composed.overlaid)
        .with(request_id)
}

/// 识别框：入场图像素坐标 → 这一格里实际画出来的那块矩形。
///
/// 预览是缩过的，所以只有一个比例；框只用来"点得中、看得出差不多"，像素级对位看的是成品图，
/// 不是这一格。
fn recognition_boxes(
    painted: egui::Rect,
    image_size: (u32, u32),
    document: Option<&OcrDocument>,
) -> Vec<(u32, egui::Rect)> {
    let Some(document) = document else {
        return Vec::new();
    };
    if image_size.0 == 0 || image_size.1 == 0 {
        return Vec::new();
    }
    document
        .blocks
        .iter()
        .map(|block| (block.id, scale_box(painted, image_size, &block.bbox)))
        .collect()
}

fn scale_box(painted: egui::Rect, image_size: (u32, u32), bbox: &PixelRect) -> egui::Rect {
    let scale = egui::vec2(
        painted.width() / image_size.0 as f32,
        painted.height() / image_size.1 as f32,
    );
    egui::Rect::from_min_size(
        painted.min + egui::vec2(bbox.x * scale.x, bbox.y * scale.y),
        egui::vec2(bbox.w * scale.x, bbox.h * scale.y),
    )
}

/// 点中了哪一块：命中的框里取**面积最小**的那个（嵌套框时点到的是里面那一块）。
/// 没有命中或没点到任何地方都是 `None`——`None` 也是有效结论：取消高亮。
fn pick_block(boxes: &[(u32, egui::Rect)], pos: Option<egui::Pos2>) -> Option<u32> {
    let pos = pos?;
    boxes
        .iter()
        .filter(|(_, rect)| rect.contains(pos))
        .min_by(|(_, a), (_, b)| a.area().partial_cmp(&b.area()).unwrap())
        .map(|(id, _)| *id)
}

/// 绘制阶段只记动作、退出对文档的借用之后再逐条执行；分成两种枚举是因为它们落地的时机不同，
/// 而不是因为形状不同——见 [`block_list`]。
enum ValueAction {
    Enable(u32, bool),
    EditText(u32, String),
}

enum ShapeAction {
    Remove(u32),
    Merge(u32),
    Split(u32),
}

/// H 区那一格译文的宽度，与原文那格同宽（两列并排，一宽一窄会把行撑歪）。
const TRANSLATION_CELL: f32 = 200.0;

/// H 区：识别结果列表，右边跟着译文列。
///
/// 动作分三趟落地：勾与改字只改值，删/合/拆三条会重排块号，重试那一颗要派发任务。同一帧里
/// 几头的动作都可能发生（正改着字又点了一下删除），所以先值、后结构，派发排在最后——
/// 它读的必须是排完号的那份文档，否则重试的是换过号的另一块。
fn block_list(ui: &mut egui::Ui, state: &mut AppState, effects: &mut Effects) {
    region(ui, LIST_MIN_HEIGHT, |ui| {
        caption(ui, "H 识别结果");
        let count = state
            .ocr
            .as_ref()
            .map(|document| document.blocks.len())
            .unwrap_or(0);
        if count == 0 {
            ui.colored_label(
                theme::MUTED,
                if state.ocr.is_some() {
                    "这张图里没有合并成块的文字"
                } else {
                    "（还没有识别结果）"
                },
            );
            return;
        }
        ui.horizontal(|ui| {
            for title in ["选", "#", "置信度", "原文", "译文"] {
                ui.colored_label(theme::MUTED, title);
            }
        });

        let mut value_actions: Vec<ValueAction> = Vec::new();
        let mut shape_actions: Vec<ShapeAction> = Vec::new();
        let mut retry: Option<u32> = None;
        let document = state.ocr.take();
        if let Some(document) = document.as_ref() {
            for block in document.blocks.iter() {
                let highlighted = state.selected_block == Some(block.id);
                let failed = state.failed_blocks.contains(&block.id);
                let row = ui.horizontal(|ui| {
                    // 复选框与文本框都绑在局部值上，改完了把动作交给状态层：
                    // 界面这一层从头到尾没有写过 `enabled` 和 `text` 这两个字段
                    let mut enabled = block.enabled;
                    let checkbox = ui.checkbox(&mut enabled, "");
                    record_cell(ui.ctx(), "checkbox", block.id, checkbox.rect);
                    if enabled != block.enabled {
                        value_actions.push(ValueAction::Enable(block.id, enabled));
                    }
                    ui.label(format!("#{}", block.id + 1));
                    let confidence = format!("{:.2}", block.confidence);
                    // 低置信度必须一眼看得见（docs/06 IMG-11）：这一行后面可能要用户改字，
                    // 看不出来就等于让他拿一条错的结果去翻译
                    if block.confidence < LOW_CONFIDENCE_WARN {
                        ui.colored_label(
                            theme::WARN,
                            RichText::new(format!("{confidence} ⚠")).strong(),
                        );
                    } else {
                        ui.colored_label(theme::MUTED, confidence);
                    }
                    let mut draft = block.text.clone();
                    ui.add(
                        egui::TextEdit::singleline(&mut draft)
                            .id_salt(("ocr_block_edit", block.id))
                            .desired_width(200.0),
                    );
                    if draft != block.text {
                        value_actions.push(ValueAction::EditText(block.id, draft));
                    }
                    // 译文列（REQ-001）：翻好的给译文，失败的给一颗只重发这一块的按钮，
                    // 没翻的留一格空位。三种情况占同一格，列表不因为一批结果跳行
                    match block.translation.as_deref() {
                        Some(text) => {
                            let cell = ui.add_sized(
                                [TRANSLATION_CELL, ui.spacing().interact_size.y],
                                egui::Label::new(text).truncate(),
                            );
                            cell.on_hover_text(text);
                        }
                        None if failed => {
                            ui.colored_label(theme::ERROR, "翻失败");
                            let again = ui.small_button("重试这一块");
                            record_cell(ui.ctx(), "retry", block.id, again.rect);
                            if again.clicked() {
                                retry = Some(block.id);
                            }
                        }
                        None => {
                            ui.colored_label(theme::MUTED, "—");
                        }
                    }
                    if ui.small_button("合并↓").clicked() {
                        shape_actions.push(ShapeAction::Merge(block.id));
                    }
                    if ui.small_button("拆分").clicked() {
                        shape_actions.push(ShapeAction::Split(block.id));
                    }
                    if ui.small_button("删除").clicked() {
                        shape_actions.push(ShapeAction::Remove(block.id));
                    }
                });
                let rect = row.response.rect;
                record_cell(ui.ctx(), "row", block.id, rect);
                if highlighted {
                    // F 区点中的那一块在这里认出来：描一圈 + 滚进视野（REQ-007）
                    ui.painter().rect_stroke(
                        rect,
                        2.0,
                        egui::Stroke::new(1.0, theme::ACCENT),
                        egui::StrokeKind::Inside,
                    );
                    row.response.scroll_to_me(Some(egui::Align::Center));
                }
            }
        }
        state.ocr = document;

        for action in value_actions {
            match action {
                ValueAction::Enable(id, enabled) => {
                    state.set_block_enabled(id, enabled);
                }
                ValueAction::EditText(id, text) => {
                    state.edit_block_text(id, text);
                }
            }
        }

        let mut notice: Option<(Tone, String)> = None;
        for action in shape_actions {
            match action {
                ShapeAction::Remove(id) => {
                    state.remove_block(id);
                }
                ShapeAction::Merge(id) => {
                    state.merge_block(id);
                }
                ShapeAction::Split(id) => {
                    notice = Some(match state.split_block(id) {
                        Split::Restored(pieces) => {
                            (Tone::Info, format!("已按识别结果拆成 {pieces} 块"))
                        }
                        Split::KeptInFirst { pieces } => (
                            Tone::Warn,
                            format!(
                                "这块改过文字：原文整段留在第 1 块，其余 {} 块为空，请核对",
                                pieces - 1
                            ),
                        ),
                        Split::Nothing => (Tone::Info, "这块拆不回：来源行不足两行".to_string()),
                    });
                }
            }
        }
        if let Some((tone, text)) = notice {
            state.toast(tone, text);
        }
        // 派发排在值与结构两趟之后：重试读的是排完号、改完字的那一份文档
        if let Some(id) = retry {
            effects.request(state.submit_image_translate(&[id]));
        }
        // I 区那句"完成 N 块"与这里的条数是同一个数，列表一变它就得跟着变
        if matches!(state.image_stage, RecognitionStage::Done { .. }) {
            state.image_stage = RecognitionStage::Done {
                blocks: count_after(state),
            };
        }
        ui.colored_label(
            theme::MUTED,
            format!(
                "参与翻译的文本：{} 字（已勾掉 {} 块）",
                state.image_merged_text().chars().count(),
                state
                    .ocr
                    .as_ref()
                    .map(|document| document.blocks.iter().filter(|b| !b.enabled).count())
                    .unwrap_or(0)
            ),
        );
    });
}

fn count_after(state: &AppState) -> usize {
    state
        .ocr
        .as_ref()
        .map(|document| document.blocks.len())
        .unwrap_or(0)
}

/// I 区：三段状态（识别 / 翻译 / 合成）+ 那颗按当前段改变含义的「停止」（REQ-009）。
///
/// 调用点在 `views/mod.rs`，排在 `CentralPanel` **之前**：本页正文那节 `ScrollArea` 是
/// `auto_shrink([false, false])`，会吃掉当时所有的剩余高度，状态条若排在它后面只剩 0px 一格，
/// 那颗「停止」根本画不出来（点不到也就测不到）。与文本页的 E 动作底栏同一条规矩。
///
/// 三段各写各的，不合成一句"处理完成"：一次合成失败不该把已经翻好的译文也说成失败的。
/// 三行都包在 `horizontal_wrapped` 里——字号放大到 2.0 时这一条会排满，被裁掉的那一段
/// 等于没有状态（宁可换行）。
pub(crate) fn status_bar(ui: &mut egui::Ui, state: &mut AppState, effects: &mut Effects) {
    let stop_hint = state.stop_hint();
    super::window_bar("image_status", egui::Margin::symmetric(8, 6), ui, |ui| {
        ui.horizontal_wrapped(|ui| {
            caption(ui, "I 处理");
            stage_cell(
                ui,
                "识别",
                &state.image_stage.label(),
                state.image_stage.phase(),
                state.image_elapsed_ms,
            );
            ui.separator();
            stage_cell(
                ui,
                "翻译",
                &translate_line(state),
                state.translate_stage.phase(),
                state.translate_elapsed_ms,
            );
            ui.separator();
            stage_cell(
                ui,
                "合成",
                &compose_line(state),
                state.compose_stage.phase(),
                state.compose_elapsed_ms,
            );
            // 「停止」钉在这一行的右边缘，三段共用一颗。滑块 2026-11-04 挪去状态栏那一行之后，
            // 这条子带回到"有按钮才存在"：条件包住整条带没有副作用了（包住滑块才会——
            // 那会让这一页在某些帧根本没有滑块，见 `views::window_bar` 上面那段历史）。
            if let Some(hint) = stop_hint {
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let stop = ui.button("停止");
                    record(ui.ctx(), "stop", stop.rect);
                    if stop.clicked() {
                        effects.stop_image = true;
                    }
                    // hover 那句随段而变：只有翻译段是真停，另两段是丢弃结果
                    stop.on_hover_text(hint);
                });
            }
        });
    });
}

/// 一段的一行：名字弱色 + 那一档的话按 `Phase` 着色 + 耗时（跑过一次才附）。
fn stage_cell(ui: &mut egui::Ui, name: &str, line: &str, phase: Phase, elapsed_ms: u64) {
    ui.colored_label(theme::MUTED, name);
    let text = if elapsed_ms == 0 {
        line.to_string()
    } else {
        format!("{line} · {elapsed_ms} ms")
    };
    ui.colored_label(phase_color(phase), text);
}

/// 四档共用一个颜色出处：进行中是琥珀（还没结果，但别当成坏了），有结果是绿，坏了才是红。
fn phase_color(phase: Phase) -> Color32 {
    match phase {
        Phase::Idle => theme::MUTED,
        Phase::Busy => theme::WARN,
        Phase::Done => theme::OK,
        Phase::Bad => theme::ERROR,
    }
}

/// 翻译段那一行的说法：空闲且缺前置条件时说的是缺哪一步，与 E 区那颗置灰同源（REQ-006 后半句）。
///
/// 界面两处读同一句，才不会出现"按钮灰着、状态行却说空闲"。
/// 只有 `Idle` 才让位给那句前置条件话：段子在跑、在飞、已经坏了，说的都是这一段自己发生了什么，
/// 这时按钮是不是灰着与这行无关。
fn translate_line(state: &AppState) -> String {
    match (&state.translate_stage, state.translate_gate()) {
        (TranslateStage::Idle, Some(blocker)) => blocker.to_string(),
        (stage, _) => stage.label(),
    }
}

/// 合成段那一行的说法，与上一条同一个规矩。
fn compose_line(state: &AppState) -> String {
    match (&state.compose_stage, state.compose_gate()) {
        (ComposeStage::Idle, Some(blocker)) => blocker.to_string(),
        (stage, _) => stage.label(),
    }
}

fn region(ui: &mut egui::Ui, min_height: f32, body: impl FnOnce(&mut Ui)) {
    egui::Frame::group(ui.style())
        .inner_margin(8.0)
        .show(ui, |ui| {
            // 预览格没有图时也要占住那一栏的高度：两列一空一满会让整页跟着跳
            ui.set_min_height(min_height);
            ui.set_width(ui.available_width());
            body(ui);
        });
}

fn caption(ui: &mut egui::Ui, text: &str) {
    ui.label(RichText::new(text).strong());
}

/// 把控件矩形交给 `Context::data`，让无头测试能拿到"打开图片"到底画在哪一格。
/// 登记与取回都在 `views` 那张共用表里（为什么要登记见 `views::rect_id`），本页占 `image` 区。
fn record(ctx: &egui::Context, salt: &str, rect: egui::Rect) {
    super::record_rect(ctx, "image", salt, rect);
}

/// 块号登记的键。块号是运行期才知道的数，`salt` 那个参数装不下"第几块"，所以拼进同一张
/// 登记表的另一个命名空间里。`kind` 分四格是因为同一块在同一帧里有四个矩形——测试要点
/// "那一行的重试"就不能拿到勾选框的位置。
fn cell_id(kind: &str, id: u32) -> Id {
    super::rect_id("image-cell", &format!("{kind}#{id}"))
}

/// 按块号登记的格子：F 区的识别框、H 区的整行、勾选框、重试那颗。
fn record_cell(ctx: &egui::Context, kind: &str, id: u32, rect: egui::Rect) {
    ctx.data_mut(|data| data.insert_temp(cell_id(kind, id), rect));
}

#[cfg(test)]
pub(crate) fn cell_rect(ctx: &egui::Context, kind: &str, id: u32) -> egui::Rect {
    ctx.data_mut(|data| {
        data.get_temp::<egui::Rect>(cell_id(kind, id))
            .unwrap_or_else(|| panic!("图片页第 {id} 块的 {kind} 格没有记下矩形"))
    })
}

/// 这一格到底画没画：`None` = 这一帧根本没出现这颗控件
#[cfg(test)]
pub(crate) fn entry_rect(ctx: &egui::Context, salt: &str) -> Option<egui::Rect> {
    super::rect_slot(ctx, "image", salt)
}

#[cfg(test)]
pub(crate) fn recorded_entry(ctx: &egui::Context, salt: &str) -> egui::Rect {
    super::recorded_rect(ctx, "image", salt)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::{Job, KeyState};
    use ice_core::config::Config;
    use ice_core::document::{OcrDocument, PixelRect, TextBlock};
    use ice_core::glossary::Glossary;
    use ice_vision::layout::RowObservation;
    use image::RgbaImage;

    fn new_state() -> AppState {
        AppState::new(
            Config::default(),
            Glossary::default(),
            KeyState::Missing,
            None,
        )
    }

    fn block(id: u32, text: &str, confidence: f32) -> TextBlock {
        TextBlock::with_rows(
            id,
            text,
            PixelRect::new(10.0, id as f32 * 20.0, 200.0, 18.0),
            confidence,
            vec![id * 2, id * 2 + 1],
        )
    }

    fn doc(texts: &[(&str, f32)]) -> OcrDocument {
        let mut document = OcrDocument::empty(600, 400);
        document.blocks = texts
            .iter()
            .enumerate()
            .map(|(index, (text, confidence))| block(index as u32, text, *confidence))
            .collect();
        document
    }

    fn rows(texts: &[&str]) -> Vec<RowObservation> {
        texts
            .iter()
            .enumerate()
            .map(|(index, text)| RowObservation {
                text: (*text).to_string(),
                normalized_box: ice_vision::NormalizedRect {
                    x: 0.1,
                    y: 0.1 + index as f32 * 0.2,
                    w: 0.5,
                    h: 0.1,
                },
                confidence: 0.9,
                row_index: index,
            })
            .collect()
    }

    /// 一帧一帧喂输入：第一轮空事件（让控件把矩形记下来），第二轮才是点击。
    /// 顺带把第二轮画出来的图元交回去——"这句话到底有没有画到屏幕上"只能从 shapes 里读。
    fn run(
        state: &mut AppState,
        effects: &mut Effects,
        supported: bool,
        clicks: &[egui::Pos2],
    ) -> egui::Context {
        let ctx = egui::Context::default();
        run_with(&ctx, state, effects, supported, clicks);
        ctx
    }

    fn run_with(
        ctx: &egui::Context,
        state: &mut AppState,
        effects: &mut Effects,
        supported: bool,
        clicks: &[egui::Pos2],
    ) -> Vec<egui::epaint::ClippedShape> {
        let screen = egui::vec2(1080.0, 720.0);
        let mut time = 0.0;
        let mut shapes = Vec::new();
        for round in 0..2 {
            let events: Vec<egui::Event> = if round == 0 {
                Vec::new()
            } else {
                clicks
                    .iter()
                    .flat_map(|at| {
                        [
                            egui::Event::PointerMoved(*at),
                            egui::Event::PointerButton {
                                pos: *at,
                                button: egui::PointerButton::Primary,
                                pressed: true,
                                modifiers: egui::Modifiers::NONE,
                            },
                            egui::Event::PointerButton {
                                pos: *at,
                                button: egui::PointerButton::Primary,
                                pressed: false,
                                modifiers: egui::Modifiers::NONE,
                            },
                        ]
                    })
                    .collect()
            };
            let mut raw = egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, screen)),
                ..Default::default()
            };
            raw.time = Some(time);
            time += 1.0 / 60.0;
            raw.events = events;
            let mut output = ctx.run_ui(raw, |ui| {
                // 与 `views/mod.rs` 同一套组合：先立底栏，再画正文
                status_bar(ui, state, effects);
                show(ui, state, effects, supported);
            });
            shapes = std::mem::take(&mut output.shapes);
            output.drop_without_applying_deltas();
        }
        shapes
    }

    /// 画面上排出去过的文字（每枚 `TextShape` 带着它排版时的原文）。
    /// 断言走这一条，而不是 `AppState` 里的某个字段——后者只证明数据在，不证明用户看得见。
    fn painted_text(shapes: &[egui::epaint::ClippedShape]) -> String {
        let mut text = String::new();
        for clipped in shapes {
            if let egui::Shape::Text(shape) = &clipped.shape {
                text.push_str(&shape.galley.job.text);
                text.push('\n');
            }
        }
        text
    }

    /// P4 那两条以"这一页没有可点控件"为前提的占位测试到此作废：
    /// 现在断的是反过来的事——三颗入口都画出来了，点下去各自给出自己的那条意图。
    #[test]
    fn each_entry_gives_its_own_intent_and_nothing_else() {
        for (salt, want) in [("open", "open_panel"), ("paste", "paste_image")] {
            let mut state = new_state();
            let mut effects = Effects::default();
            let ctx = run(&mut state, &mut effects, true, &[]);
            let at = recorded_entry(&ctx, salt).center();
            effects = Effects::default();
            run(&mut state, &mut effects, true, &[at]);
            let fired = match want {
                "open_panel" => effects.open_panel,
                _ => effects.paste_image,
            };
            assert!(fired, "点 {salt} 没有给出 {want}");
            let other = match want {
                "open_panel" => effects.paste_image,
                _ => effects.open_panel,
            };
            assert!(
                !other,
                "点 {salt} 顺带把另一条入口也触发了，两颗按钮响应区重叠"
            );
            assert!(effects.job.is_none(), "光点入口不该派发识别");
        }
    }

    /// 「识别」是这一页唯一直接派发任务的一颗：它给出的是 `Job::ImageOcr`，
    /// 而路径来自输入框，不在帧末之外另读一次磁盘。
    #[test]
    fn the_path_entry_dispatches_an_image_job() {
        let mut state = new_state();
        state.image_path_input = "/tmp/shot.png".into();
        let mut effects = Effects::default();
        let ctx = run(&mut state, &mut effects, true, &[]);
        let at = recorded_entry(&ctx, "submit").center();
        effects = Effects::default();
        run(&mut state, &mut effects, true, &[at]);
        let Some(Job::ImageOcr { input, .. }) = effects.job.as_ref() else {
            panic!("点「识别」没有派发识别任务：{:?}", effects.job);
        };
        assert_eq!(
            input,
            &crate::state::ImageInput::Path(std::path::PathBuf::from("/tmp/shot.png"))
        );
        assert_eq!(state.image_stage, RecognitionStage::Recognizing);
    }

    /// 空路径按下「识别」不该凭空造一个任务，也不该把状态推到识别中
    #[test]
    fn an_empty_path_box_dispatches_nothing() {
        let mut state = new_state();
        let mut effects = Effects::default();
        let ctx = run(&mut state, &mut effects, true, &[]);
        let at = recorded_entry(&ctx, "submit").center();
        run(&mut state, &mut effects, true, &[at]);
        assert!(effects.job.is_none());
        assert_eq!(state.image_stage, RecognitionStage::Idle);
    }

    #[test]
    fn rendering_does_not_invent_jobs_or_effects() {
        let mut state = new_state();
        state.ocr = Some(doc(&[("hello", 0.9)]));
        let mut effects = Effects::default();
        run(&mut state, &mut effects, true, &[]);
        assert!(effects.job.is_none(), "光是渲染不该派发任务");
        assert!(!effects.open_panel);
        assert!(!effects.paste_image);
        assert!(!effects.stop_image);
    }

    /// REQ-007 的无头那一半：不支持的平台上三颗入口**还在页面上**（记得到矩形），
    /// 但点下去一条意图都不给。
    #[test]
    fn unsupported_platform_keeps_the_entries_but_stops_the_clicks() {
        for salt in ["open", "paste", "submit"] {
            let mut state = new_state();
            state.image_path_input = "/tmp/a.png".into();
            let mut effects = Effects::default();
            let ctx = run(&mut state, &mut effects, false, &[]);
            let rect = recorded_entry(&ctx, salt);
            assert!(
                rect.width() > 1.0 && rect.height() > 1.0,
                "{salt} 在不支持的平台上被藏起来了（{rect:?}）——置灰才是 REQ-007 要的"
            );
            run(&mut state, &mut effects, false, &[rect.center()]);
            assert!(!effects.open_panel && !effects.paste_image);
            assert!(effects.job.is_none(), "{salt} 被禁了还能派发");
            assert_eq!(state.image_stage, RecognitionStage::Idle);
        }
    }

    /// 已缩放必须写在脸上（Q-007 不静默）：断的是那一格**画出来的字**，不是字段值。
    #[test]
    fn a_scaled_image_is_announced_next_to_its_size() {
        let mut state = new_state();
        state.image = Some(admit(8, 6, true));
        let mut effects = Effects::default();
        let shapes = run_with(
            &egui::Context::default(),
            &mut state,
            &mut effects,
            true,
            &[],
        );
        let text = painted_text(&shapes);
        assert!(
            text.contains("已缩放"),
            "入场图带缩放标记，界面上却没说：{text:?}"
        );
        assert!(
            text.contains("8 × 6 px"),
            "尺寸那句要跟着缩放说明一起出现：{text:?}"
        );

        // 没缩放的图不该冒出这句——否则警告就成了噪声
        let mut unscaled = new_state();
        unscaled.image = Some(admit(8, 6, false));
        let mut effects = Effects::default();
        let shapes = run_with(
            &egui::Context::default(),
            &mut unscaled,
            &mut effects,
            true,
            &[],
        );
        assert!(
            !painted_text(&shapes).contains("已缩放"),
            "没缩放也提示，等于把警告讲便宜了"
        );
    }

    fn admit(width: u32, height: u32, scaled: bool) -> Admitted {
        Admitted {
            image: RgbaImage::from_pixel(width, height, image::Rgba([255, 255, 255, 255])),
            scaled,
            width,
            height,
            label: "sample.png".to_string(),
        }
    }

    /// 一张"已经翻好"的图：入场图 + 每块都带译文，合成段的前置条件就此成立。
    /// 给 E 区那颗「生成图片」和 G 区那一格用——它们的条件全在状态层，这里只造数据。
    fn translated_state(pairs: &[(&str, &str)]) -> AppState {
        let mut state = new_state();
        state.image = Some(admit(600, 400, false));
        let mut document = OcrDocument::empty(600, 400);
        document.blocks = pairs
            .iter()
            .enumerate()
            .map(|(index, (text, translation))| {
                let mut block = block(index as u32, text, 0.9);
                block.translation = Some((*translation).to_string());
                block
            })
            .collect();
        state.ocr = Some(document);
        state.image_stage = RecognitionStage::Done {
            blocks: pairs.len(),
        };
        state
    }

    /// 一块带译文的结果，用来测 G 区那一格。
    fn composed(blocks: &[(u32, bool)]) -> Composed {
        let mut per_block = Vec::new();
        let mut degraded_blocks = Vec::new();
        let mut replaced = 0;
        for (id, degraded) in blocks {
            per_block.push((*id, ice_vision::typeset::PlanKind::InPlace));
            replaced += 1;
            if *degraded {
                degraded_blocks.push(*id);
            }
        }
        Composed {
            image: RgbaImage::from_pixel(600, 400, image::Rgba([255, 255, 255, 255])),
            per_block,
            replaced,
            overlaid: 0,
            skipped: 0,
            degraded_blocks,
            missing_glyphs: Vec::new(),
        }
    }

    /// 五种动作都作用在**将要交给 P7 的那份文档**上，并立刻反映在 `merged_text()`（REQ-005）。
    #[test]
    fn every_edit_action_moves_the_document_that_p7_will_receive() {
        let mut state = new_state();
        state.ocr = Some(doc(&[("第一块", 0.9), ("第二块", 0.9), ("第三块", 0.4)]));
        assert_eq!(state.image_merged_text(), "第一块\n第二块\n第三块");

        assert!(state.set_block_enabled(1, false));
        assert_eq!(state.image_merged_text(), "第一块\n第三块");
        assert!(state.set_block_enabled(1, true), "勾选是可以勾回来的");
        assert_eq!(state.image_merged_text(), "第一块\n第二块\n第三块");

        assert!(state.edit_block_text(0, "改成这样".to_string()));
        assert_eq!(state.image_merged_text(), "改成这样\n第二块\n第三块");

        // 合并那一步光比 `merged_text` 看不出差别（拼接符本来就是 `\n`），要比的是块数与块内容
        assert!(state.merge_block(1));
        let blocks = &state.ocr.as_ref().unwrap().blocks;
        assert_eq!(blocks.len(), 2, "合并后少一块");
        assert_eq!(blocks[1].text, "第二块\n第三块");
        assert_eq!(state.image_merged_text(), "改成这样\n第二块\n第三块");
        assert!(
            !state.merge_block(1),
            "最后一块下面没有邻居，合并该拒绝而不是把自己并给自己"
        );

        assert!(state.remove_block(0));
        assert_eq!(state.image_merged_text(), "第二块\n第三块");
        assert_eq!(
            state
                .ocr
                .as_ref()
                .unwrap()
                .blocks
                .iter()
                .map(|b| b.id)
                .collect::<Vec<_>>(),
            vec![0],
            "删完要重排块号，列表里不能留空洞"
        );
    }

    /// 勾选这一条走的是状态层方法，不是界面自己改字段（REQ-005：`views::image` 只调这些动作）。
    /// 无头测试只能从外面看结果：点一号块的勾选框，改动落在**那一块**的 `enabled` 上，
    /// 块仍然在列表里（勾选和删除是两件事），而 `merged_text()` 当场少一行。
    #[test]
    fn clicking_a_row_checkbox_routes_through_the_state_action() {
        let mut state = new_state();
        state.ocr = Some(doc(&[("第一块", 0.9), ("第二块", 0.9)]));
        state.image_stage = RecognitionStage::Done { blocks: 2 };
        let mut effects = Effects::default();
        let ctx = run(&mut state, &mut effects, true, &[]);
        let at = cell_rect(&ctx, "checkbox", 0).center();

        let mut effects = Effects::default();
        run(&mut state, &mut effects, true, &[at]);
        let blocks = &state.ocr.as_ref().expect("勾选不该把文档清空").blocks;
        assert_eq!(blocks.len(), 2, "勾选不是删除，块还得留在列表里");
        assert!(!blocks[0].enabled, "点了一号格的勾选框却没关掉它");
        assert!(blocks[1].enabled, "点一号格顺带把另一行也勾掉了");
        assert_eq!(state.image_merged_text(), "第二块");
        assert!(effects.job.is_none(), "改勾选不该顺手派发翻译或识别");

        let mut effects = Effects::default();
        run(&mut state, &mut effects, true, &[at]);
        assert!(
            state.ocr.as_ref().unwrap().blocks[0].enabled,
            "同一颗再点一次要勾得回来——关掉即删数据的话这里就回不去了"
        );
        assert_eq!(state.image_merged_text(), "第一块\n第二块");
    }

    /// 合并后拆分：文字没动过就逐行还原（Q-004 的第一档）。
    #[test]
    fn a_merge_then_split_with_untouched_text_restores_each_row() {
        let mut state = new_state();
        let mut document = doc(&[("上行", 0.9), ("下行", 0.8)]);
        document.blocks[0].source_rows = vec![0];
        document.blocks[1].source_rows = vec![1];
        state.ocr = Some(document);
        state.image_rows = rows(&["上行", "下行"]);

        assert!(state.merge_block(0));
        assert_eq!(state.image_merged_text(), "上行\n下行");
        assert_eq!(state.split_block(0), Split::Restored(2));
        assert_eq!(state.image_merged_text(), "上行\n下行");
        let blocks = &state.ocr.as_ref().unwrap().blocks;
        assert_eq!(blocks.len(), 2);
        assert_eq!(blocks[0].source_rows, vec![0]);
        assert_eq!(blocks[1].source_rows, vec![1]);
    }

    /// 改过文字再拆：不猜字符比例，整段留在第一块，其余给空串（Q-004 的第二档）。
    /// 空串块被 `enabled_blocks` 天然跳过，所以不会把空块送进 P7。
    #[test]
    fn a_split_of_edited_text_keeps_everything_in_the_first_piece() {
        let mut state = new_state();
        let mut document = doc(&[("上行", 0.9)]);
        document.blocks[0].source_rows = vec![0, 1];
        state.ocr = Some(document);
        state.image_rows = rows(&["上行", "下行"]);

        assert_eq!(state.split_block(0), Split::KeptInFirst { pieces: 2 });
        let blocks = &state.ocr.as_ref().unwrap().blocks;
        assert_eq!(blocks[0].text, "上行", "整段留在第一块");
        assert_eq!(blocks[1].text, "");
        assert_eq!(state.image_merged_text(), "上行");
    }

    /// 来源行不足两行（或换过图没重认）就拆不动，界面该给一句"拆不回"而不是静默
    #[test]
    fn a_block_without_two_source_rows_cannot_be_split() {
        let mut state = new_state();
        let mut document = doc(&[("只有一行", 0.9)]);
        document.blocks[0].source_rows = vec![3];
        state.ocr = Some(document);
        state.image_rows = rows(&["第 0 行", "第 1 行", "第 2 行", "第 3 行"]);
        assert_eq!(state.split_block(0), Split::Nothing);

        state.image_rows.clear();
        let mut other = doc(&[("两行", 0.9)]);
        other.blocks[0].source_rows = vec![9, 10];
        state.ocr = Some(other);
        assert_eq!(state.split_block(0), Split::Nothing, "行号越界时不许硬拆");
    }

    #[test]
    fn the_status_line_names_all_five_stages_and_the_elapsed() {
        let mut state = new_state();
        assert_eq!(state.image_stage.label(), "空闲");
        state.image_elapsed_ms = 812;
        state.image_stage = RecognitionStage::Done { blocks: 7 };
        assert_eq!(state.image_stage.label(), "完成 7 块");
        state.image_stage = RecognitionStage::NoText;
        assert_eq!(state.image_stage.label(), "未发现文字");
        state.image_stage = RecognitionStage::Failed("本机识别失败，请重试或换一张图".to_string());
        assert!(state.image_stage.label().starts_with("识别失败："));
    }

    /// REQ-009 那颗共用的「停止」：三段都不在飞时它**不存在**，任何一段在飞时它都在，
    /// 而且三段用的是同一颗——按下去给出的意图相同，含义由状态层按段决定。
    #[test]
    fn the_stop_button_exists_only_while_a_segment_is_in_flight() {
        // 空闲：那一格根本没画，用户不会以为有一次任务可以撤
        let mut state = new_state();
        let mut effects = Effects::default();
        let ctx = run(&mut state, &mut effects, true, &[]);
        assert!(
            entry_rect(&ctx, "stop").is_none(),
            "三段都空闲也画「停止」，等于凭空造了一次任务"
        );

        // 识别段在飞
        let mut state = new_state();
        let job = state
            .submit_image(crate::state::ImageInput::Path(std::path::PathBuf::from(
                "/tmp/a.png",
            )))
            .expect("要派发一次识别");
        assert!(matches!(job, Job::ImageOcr { .. }));
        let mut effects = Effects::default();
        let ctx = run(&mut state, &mut effects, true, &[]);
        let at = recorded_entry(&ctx, "stop").center();
        effects = Effects::default();
        run(&mut state, &mut effects, true, &[at]);
        assert!(effects.stop_image, "点「停止」没给出意图");
        assert!(
            matches!(state.image_stage, RecognitionStage::Recognizing),
            "视图不该自己改段的状态（帧末才动手）"
        );

        // 翻译段在飞
        let mut state = translated_state(&[("hello", "你好")]);
        state.ocr.as_mut().unwrap().blocks[0].translation = None;
        state
            .submit_image_translate(&[])
            .expect("有文字就该派发得出逐块翻译");
        let mut effects = Effects::default();
        let ctx = run(&mut state, &mut effects, true, &[]);
        assert!(
            entry_rect(&ctx, "stop").is_some(),
            "翻译段在飞时「停止」不见了"
        );

        // 合成段在飞
        let mut state = translated_state(&[("hello", "你好")]);
        state
            .submit_image_compose()
            .expect("有译文就该派发得出合成");
        let mut effects = Effects::default();
        let ctx = run(&mut state, &mut effects, true, &[]);
        assert!(
            entry_rect(&ctx, "stop").is_some(),
            "合成段在飞时「停止」不见了"
        );
    }

    /// 一颗按钮对三段，所以那句话必须跟着段变（REQ-009 的"不谎称撤回"）。
    /// 断的是状态层给的原文：界面只是把它交给 hover，文案本身只有一处来源。
    #[test]
    fn the_stop_hint_names_the_segment_actually_in_flight() {
        let mut state = new_state();
        assert_eq!(state.stop_hint(), None, "空闲时不给文案");

        state
            .submit_image(crate::state::ImageInput::Path(std::path::PathBuf::from(
                "/tmp/a.png",
            )))
            .expect("要派发一次识别");
        assert!(
            state.stop_hint().unwrap_or_default().contains("撤不回来"),
            "识别段要说清这只是丢弃结果：{:?}",
            state.stop_hint()
        );

        // 换图把识别段记过期之后再派翻译：两条 hint 不能同时成立，所以这里读的是翻译段那句
        let mut state = translated_state(&[("hello", "你好")]);
        state.ocr.as_mut().unwrap().blocks[0].translation = None;
        state.submit_image_translate(&[]).expect("要派发出一批翻译");
        assert!(
            state.stop_hint().unwrap_or_default().contains("块边界"),
            "翻译段要说清停在块边界、已翻好的留下：{:?}",
            state.stop_hint()
        );

        let mut state = translated_state(&[("hello", "你好")]);
        state.submit_image_compose().expect("要派发一次合成");
        assert!(
            state.stop_hint().unwrap_or_default().contains("撤不回来"),
            "合成段与识别段同一口径，不许说成已停止：{:?}",
            state.stop_hint()
        );
    }

    /// 列表一变，I 区那句"完成 N 块"的 N 必须跟着变：两处不同源就是当面自相矛盾。
    /// 断的是**下一帧画出来的那句**，不是 `image_stage` 字段——刷新动作发生在 `block_list` 里。
    #[test]
    fn removing_a_block_updates_the_done_count_in_the_status_line() {
        let mut state = new_state();
        state.ocr = Some(doc(&[("a", 0.9), ("b", 0.9)]));
        state.image_stage = RecognitionStage::Done { blocks: 2 };
        let mut effects = Effects::default();
        let shapes = run_with(
            &egui::Context::default(),
            &mut state,
            &mut effects,
            true,
            &[],
        );
        let first_text = painted_text(&shapes);
        assert!(first_text.contains("完成 2 块"), "第一帧：{first_text:?}");

        assert!(state.remove_block(0));
        let mut effects = Effects::default();
        let shapes = run_with(
            &egui::Context::default(),
            &mut state,
            &mut effects,
            true,
            &[],
        );
        let text = painted_text(&shapes);
        assert!(
            text.contains("完成 1 块"),
            "列表只剩一块，状态行还报 2：{text:?}"
        );
        assert!(!text.contains("完成 2 块"));
    }

    /// 低置信度那一行要看得见（docs/06 IMG-11）：⚠ 是画出来的，不是藏在字段里的。
    #[test]
    fn a_low_confidence_row_is_marked_and_a_high_one_is_not() {
        let mut state = new_state();
        state.ocr = Some(doc(&[
            ("这一行很稳", 0.99),
            ("这一行可疑", LOW_CONFIDENCE_WARN - 0.01),
        ]));
        let mut effects = Effects::default();
        let shapes = run_with(
            &egui::Context::default(),
            &mut state,
            &mut effects,
            true,
            &[],
        );
        let text = painted_text(&shapes);
        // 每一枚 label / TextEdit 各自是一个 galley，所以"同一 UI 行"里的字段会散成不同的行；
        // 这里比的是整页文本里出现过哪几个置信度串
        assert!(
            text.contains("0.49 ⚠"),
            "低于警戒线 {LOW_CONFIDENCE_WARN} 的那行没有带上 ⚠：{text:?}"
        );
        assert!(
            !text.contains("0.99 ⚠"),
            "警戒线以上的那行不该跟着响：{text:?}"
        );
        assert!(text.contains("0.99"), "两行都该画出来：{text:?}");
    }

    /// 前置条件矩阵（REQ-006 后半句 + REQ-007）：条件不满足时那颗入口**还在页面上**、灰着，
    /// 而缺的那一步写在 I 区状态行里。五颗各测自己那一格，不写成"整页都灰"这种糊断言。
    #[test]
    fn each_greyed_entry_leaves_the_page_intact_and_the_status_line_names_the_missing_step() {
        // 只识别完、一块译文都没有：翻译那颗能点，合成那颗灰着并说出缺哪一步
        let mut state = new_state();
        state.image = Some(admit(600, 400, false));
        state.ocr = Some(doc(&[("hello", 0.9)]));
        state.image_stage = RecognitionStage::Done { blocks: 1 };
        let mut effects = Effects::default();
        let ctx = egui::Context::default();
        let shapes = run_with(&ctx, &mut state, &mut effects, true, &[]);
        let text = painted_text(&shapes);
        assert!(
            text.contains("先翻出至少一块译文"),
            "合成缺的那一步没写进状态行：{text:?}"
        );
        assert!(
            !text.contains("先识别出至少一块文字"),
            "已经有文字块，别再报那一步：{text:?}"
        );
        for salt in ["translate", "compose", "retry", "save", "copy_image"] {
            let rect = recorded_entry(&ctx, salt);
            assert!(
                rect.width() > 1.0 && rect.height() > 1.0,
                "{salt} 在条件不满足时消失了（{rect:?}）——置灰才是 REQ-007 要的"
            );
        }
        // 灰着的那几颗点下去一条意图都不给：合成/重试不给任务，保存/复制不给那两条出路
        let greyed = [
            recorded_entry(&ctx, "compose").center(),
            recorded_entry(&ctx, "retry").center(),
            recorded_entry(&ctx, "save").center(),
            recorded_entry(&ctx, "copy_image").center(),
        ];
        let mut effects = Effects::default();
        run(&mut state, &mut effects, true, &greyed);
        assert!(
            effects.job.is_none(),
            "条件不满足的合成/重试却派发了任务：{:?}",
            effects.job
        );
        assert!(
            !effects.save_image && !effects.copy_image,
            "没有成品却给出了保存/复制的意图：save={} copy={}",
            effects.save_image,
            effects.copy_image
        );

        // 连文字块都没有：翻译那颗也说得出缺哪一步
        let mut bare = new_state();
        bare.image = Some(admit(600, 400, false));
        let mut effects = Effects::default();
        let shapes = run_with(
            &egui::Context::default(),
            &mut bare,
            &mut effects,
            true,
            &[],
        );
        let text = painted_text(&shapes);
        assert!(
            text.contains("先识别出至少一块文字"),
            "翻译段空着却要说出空的原因：{text}"
        );

        // 图都没导入：合成那颗的条件是"先导入一张图"
        let mut empty = new_state();
        let mut effects = Effects::default();
        let shapes = run_with(
            &egui::Context::default(),
            &mut empty,
            &mut effects,
            true,
            &[],
        );
        let text = painted_text(&shapes);
        assert!(
            text.contains("先导入一张图"),
            "G 区空着时要说缺的是图：{text}"
        );
    }

    /// 条件齐了，三颗就都能派发，而且各派各的那一支任务。
    #[test]
    fn every_enabled_entry_dispatches_its_own_job() {
        // 「翻译全部」：整批一次派发，块号取自文档
        let mut state = translated_state(&[("hello", "你好"), ("world", "世界")]);
        state
            .ocr
            .as_mut()
            .unwrap()
            .blocks
            .iter_mut()
            .for_each(|b| b.translation = None);
        state.translate_stage = TranslateStage::Idle;
        let mut effects = Effects::default();
        let ctx = run(&mut state, &mut effects, true, &[]);
        run(
            &mut state,
            &mut effects,
            true,
            &[recorded_entry(&ctx, "translate").center()],
        );
        let Some(Job::ImageTranslate { blocks, .. }) = effects.job.as_ref() else {
            panic!("条件齐了却没派发出逐块翻译：{:?}", effects.job);
        };
        assert_eq!(
            blocks.iter().map(|(id, _)| *id).collect::<Vec<_>>(),
            vec![0, 1],
            "整批翻译少了一块"
        );

        // 「生成图片」：给的是合成那一支
        let mut state = translated_state(&[("hello", "你好")]);
        let mut effects = Effects::default();
        let ctx = run(&mut state, &mut effects, true, &[]);
        run(
            &mut state,
            &mut effects,
            true,
            &[recorded_entry(&ctx, "compose").center()],
        );
        assert!(
            matches!(effects.job, Some(Job::ImageCompose { .. })),
            "条件齐了却没派发合成：{:?}",
            effects.job
        );

        // 「重试失败块」：只给台账里那几块，不是整批
        let mut state = translated_state(&[("hello", "你好"), ("world", "世界")]);
        state.failed_blocks = vec![1];
        let mut effects = Effects::default();
        let ctx = run(&mut state, &mut effects, true, &[]);
        run(
            &mut state,
            &mut effects,
            true,
            &[recorded_entry(&ctx, "retry").center()],
        );
        let Some(Job::ImageTranslate { blocks, .. }) = effects.job.as_ref() else {
            panic!("台账里有失败块却没派发重试：{:?}", effects.job);
        };
        assert_eq!(
            blocks.iter().map(|(id, _)| *id).collect::<Vec<_>>(),
            vec![1],
            "重试把整批重翻了——已翻好的那块不该再动"
        );
    }

    /// 台账为空时那颗「重试失败块」绝不能变成"整批重翻"：空块号列表在
    /// [`AppState::submit_image_translate`] 里正是那个意思，所以这一格要有自己的一道门。
    #[test]
    fn an_empty_failure_ledger_retries_nothing() {
        let mut state = translated_state(&[("hello", "你好")]);
        state.failed_blocks.clear();
        let mut effects = Effects::default();
        assert!(state.submit_image_retry_failed().is_none());

        // 界面上同理：那一颗这时候是灰的，点下去不派发
        let ctx = run(&mut state, &mut effects, true, &[]);
        let at = recorded_entry(&ctx, "retry").center();
        let mut effects = Effects::default();
        run(&mut state, &mut effects, true, &[at]);
        assert!(
            effects.job.is_none(),
            "空台账点重试派发了任务：{:?}",
            effects.job
        );
    }

    /// 两条出路（REQ-008）：有成品时两颗各给各的意图、互不牵连，而且**谁都不派发任务**——
    /// 保存与复制都在帧末碰系统（面板 / 剪贴板），视图这一层只记意图，所以这两颗在无头里能测到底。
    /// 点完成品还得在（那两条出路都是"交出去一份拷贝"，不是"消费掉它"）。
    ///
    /// Q-004 那句"会覆盖剪贴板里的文本"本身锁在 `system::clipboard` 的测试里，这里锁的是接线：
    /// 那颗按钮的 hover 直接引用同一个常量，界面没有第二份文案。
    #[test]
    fn the_two_export_entries_give_their_own_intent_and_never_a_job() {
        for (salt, saving) in [("save", true), ("copy_image", false)] {
            let mut state = translated_state(&[("hello", "你好")]);
            state.composed = Some(composed(&[(0, false)]));
            let mut effects = Effects::default();
            let ctx = run(&mut state, &mut effects, true, &[]);
            let at = recorded_entry(&ctx, salt).center();
            effects = Effects::default();
            run(&mut state, &mut effects, true, &[at]);
            assert_eq!(effects.save_image, saving, "点 {salt} 给的保存意图不对");
            assert_eq!(effects.copy_image, !saving, "点 {salt} 给的复制意图不对");
            assert!(
                effects.job.is_none(),
                "点 {salt} 派发了任务：{:?}",
                effects.job
            );
            assert!(state.composed.is_some(), "点 {salt} 把成品抹掉了");
        }

        // 缺成品时那两颗灰着，说的是状态层给的那一句（同一个出处，界面不抄第二份）
        let mut bare = translated_state(&[("hello", "你好")]);
        assert_eq!(bare.export_gate(), Some("先生成一张成品图"));
        bare.composed = Some(composed(&[(0, false)]));
        assert_eq!(bare.export_gate(), None, "有成品了那道门就该开");

        // 合成在飞时门照样开着：G 区显示的是上一张成品，存出去的就是它（所见即所存）
        let mut composing = translated_state(&[("hello", "你好")]);
        composing.composed = Some(composed(&[(0, false)]));
        composing
            .submit_image_compose()
            .expect("有译文就该派发得出合成");
        assert_eq!(composing.export_gate(), None, "合成在飞时两条出路被挡了");
    }

    /// 画法单选只改下一次生成用哪一档：不派发、不重认、不重翻，已有的成品也留着（REQ-006）。
    #[test]
    fn picking_a_compose_mode_changes_only_the_mode() {
        let mut state = translated_state(&[("hello", "你好")]);
        state.compose_request_id = 4;
        state.composed = Some(composed(&[(0, false)]));
        let mut effects = Effects::default();
        let ctx = run(&mut state, &mut effects, true, &[]);
        let before = (
            state.image_request_id,
            state.translate_request_id,
            state.compose_request_id,
        );
        assert_eq!(state.compose_mode, CompositeMode::Auto);
        let at = recorded_entry(&ctx, "mode_overlay").center();
        let mut effects = Effects::default();
        run(&mut state, &mut effects, true, &[at]);
        assert_eq!(
            state.compose_mode,
            CompositeMode::OverlayAll,
            "点「铺底板」没换档"
        );
        assert!(effects.job.is_none(), "换画法不该派发任何任务");
        assert!(state.composed.is_some(), "换画法不该把已有成品抹掉");
        assert_eq!(
            (
                state.image_request_id,
                state.translate_request_id,
                state.compose_request_id
            ),
            before,
            "换画法重新起了某一段的序号，等于偷偷重跑了一次"
        );
        assert_eq!(
            state.ocr.as_ref().unwrap().blocks[0].translation.as_deref(),
            Some("你好"),
            "换画法不该动已经翻好的译文"
        );
    }

    /// 换一次生成就要换一张纹理：成品与原图同尺寸，键里少了序号就会被缓存吃掉，
    /// 用户在 G 区盯着上一张看（REQ-009 第四条的"换代"白做）。
    #[test]
    fn the_result_texture_key_follows_the_compose_request() {
        let one = composed(&[(0, false)]);
        let same = composed(&[(0, false)]);
        assert_eq!(
            composed_preview_id(&one, 3),
            composed_preview_id(&same, 3),
            "同一张成品每帧都要重建纹理，白白多一次上传"
        );
        assert_ne!(
            composed_preview_id(&one, 3),
            composed_preview_id(&one, 4),
            "新一次的成品与上一次同键，G 区会一直显示旧的那张"
        );
    }

    /// 降级块要看得见（FR-2.7）：断的是那一圈橙**画没画出来**，不是 `degraded` 有没有值。
    #[test]
    fn a_degraded_block_is_outlined_and_a_normal_one_is_not() {
        let mut degraded = translated_state(&[("hello", "你好"), ("world", "世界")]);
        degraded.composed = Some(composed(&[(0, true), (1, false)]));
        let mut effects = Effects::default();
        let shapes = run_with(
            &egui::Context::default(),
            &mut degraded,
            &mut effects,
            true,
            &[],
        );
        assert!(has_warn_outline(&shapes), "有降级块却没画出橙描边");
        assert!(
            painted_text(&shapes).contains("降级 1 块"),
            "橙描边要配一句数得出来的话：{:?}",
            painted_text(&shapes)
        );

        let mut clean = translated_state(&[("hello", "你好"), ("world", "世界")]);
        clean.composed = Some(composed(&[(0, false), (1, false)]));
        let mut effects = Effects::default();
        let shapes = run_with(
            &egui::Context::default(),
            &mut clean,
            &mut effects,
            true,
            &[],
        );
        assert!(
            !has_warn_outline(&shapes),
            "没有降级块也描橙，警告就成了噪声"
        );
    }

    /// F 区点一个识别框 → 选中的是**那一块**，而且这一笔不派发任何任务（REQ-007 的"点框定位"）。
    #[test]
    fn clicking_a_recognition_box_selects_that_row_and_dispatches_nothing() {
        let mut state = new_state();
        state.image = Some(admit(600, 400, false));
        state.ocr = Some(doc(&[("第一块", 0.9), ("第二块", 0.9)]));
        state.image_stage = RecognitionStage::Done { blocks: 2 };
        let mut effects = Effects::default();
        let ctx = run(&mut state, &mut effects, true, &[]);
        assert_eq!(state.selected_block, None, "还没点就选中了");
        let at = cell_rect(&ctx, "box", 1).center();
        let mut effects = Effects::default();
        run(&mut state, &mut effects, true, &[at]);
        assert_eq!(
            state.selected_block,
            Some(1),
            "点二号框没有选中那一块，实际是 {:?}",
            state.selected_block
        );
        assert!(effects.job.is_none(), "点框不该派发任务");

        // 点预览里、框外面的那一片：取消高亮，而不是停在最后一块上。
        // 点在预览格之外不算这一笔（那里是正文的空白，不该顺手改选中），所以取框下方那片留白
        let mut outside = new_state();
        outside.image = Some(admit(600, 400, false));
        outside.ocr = Some(doc(&[("第一块", 0.9), ("第二块", 0.9)]));
        outside.image_stage = RecognitionStage::Done { blocks: 2 };
        outside.selected_block = Some(0);
        let mut effects = Effects::default();
        let ctx = run(&mut outside, &mut effects, true, &[]);
        let box0 = cell_rect(&ctx, "box", 0);
        let box1 = cell_rect(&ctx, "box", 1);
        let at = egui::pos2(
            box1.center().x,
            box1.bottom() + (box1.height() + 4.0).max(8.0),
        );
        assert!(
            at.y > box0.bottom() && !box1.contains(at),
            "这一笔要落在预览内、框外：{at:?}"
        );
        let mut effects = Effects::default();
        run(&mut outside, &mut effects, true, &[at]);
        assert_eq!(
            outside.selected_block, None,
            "点到框外还留着选中，描圈就赖在那一行上不走了"
        );
    }

    /// 列表里那一颗「重试这一块」只重发它所在的那一块（REQ-001 第二条）。
    /// 那一颗只在"这块没有译文且在本批失败台账里"时出现，所以先把第二块做成那个形状。
    #[test]
    fn the_row_retry_re_sends_only_that_block() {
        let mut state = translated_state(&[("hello", "你好"), ("world", "世界")]);
        state.ocr.as_mut().unwrap().blocks[1].translation = None;
        state.failed_blocks = vec![1];
        let mut effects = Effects::default();
        let ctx = run(&mut state, &mut effects, true, &[]);
        let at = cell_rect(&ctx, "retry", 1).center();
        let mut effects = Effects::default();
        run(&mut state, &mut effects, true, &[at]);
        let Some(Job::ImageTranslate { blocks, .. }) = effects.job.as_ref() else {
            panic!("点行内重试没派发任务：{:?}", effects.job);
        };
        assert_eq!(
            blocks.iter().map(|(id, _)| *id).collect::<Vec<_>>(),
            vec![1],
            "行内重试把别块也发出去了"
        );
    }

    /// 形状像描边的图元里有没有那一圈橙（`theme::WARN` 的 1.5px 矩形边框）。
    fn has_warn_outline(shapes: &[egui::epaint::ClippedShape]) -> bool {
        shapes.iter().any(|clipped| {
            let egui::Shape::Rect(rect) = &clipped.shape else {
                return false;
            };
            rect.stroke.color == theme::WARN && rect.stroke.width > 0.0
        })
    }

    #[test]
    fn the_unsupported_hint_says_what_is_missing_and_what_is_not() {
        assert!(
            UNSUPPORTED_HINT.contains("当前平台"),
            "要说清是这台机器不行，而不是这个版本没做"
        );
        assert!(
            UNSUPPORTED_HINT.contains("文本翻译"),
            "顺手要说一句文本功能不受影响（REQ-007）"
        );
    }
}
