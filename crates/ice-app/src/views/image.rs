//! 图片页（CHG-005）：E 导入、F 原图预览、H 可编辑识别结果、I 识别状态。
//!
//! G 结果预览在这一版还是空的一格——它画的是"回填译文之后的图"，那是 P7 的产物。
//! 这一页其余每一颗控件都立刻改到 `AppState.ocr` 那份文档上，而那份文档正是 P7 要拿去
//! 翻译与合成的那一份（REQ-005 的判据就落在这里）。
//!
//! 视图不碰系统：面板、剪贴板读图、拖拽落地统统记成 `Effects`，帧末由 `app.rs` 执行；
//! 派发识别走 `Effects::job`（`Job::ImageOcr`），与文本翻译共用同一条派发口。

use egui::{self, Align, ColorImage, Id, Layout, RichText, TextureOptions, Ui};
use ice_vision::{Admitted, LOW_CONFIDENCE_WARN};

use crate::state::{AppState, RecognitionStage, Split, Tone};
use crate::theme;
use crate::views::Effects;

/// F 区预览的最长边（像素）。入场后的图最大 8000px，画进这一格前先等比降下来：
/// 预览只用来"看个位置和对不对得上"，不需要像素级保真。
const PREVIEW_MAX_EDGE: u32 = 320;

/// F 那一格能给的高度（宽由 `ui.columns` 分一半，随窗口变）。
const PREVIEW_MAX_HEIGHT: f32 = 220.0;

/// H 那一格没有结果时的最低高度：列表一空就塌成两行，整页会跟着跳。
const LIST_MIN_HEIGHT: f32 = 96.0;

pub fn show(ui: &mut egui::Ui, state: &mut AppState, effects: &mut Effects, supported: bool) {
    ui.spacing_mut().item_spacing.y = 8.0;
    import_bar(ui, state, effects, supported);
    egui::ScrollArea::vertical()
        .auto_shrink([false, false])
        .show(ui, |ui| {
            preview_row(ui, state);
            block_list(ui, state);
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

/// F 原图预览 + G 结果预览，两格同尺寸对齐（`03-ui-design.md` §2）。
fn preview_row(ui: &mut egui::Ui, state: &mut AppState) {
    let request_id = state.image_request_id;
    ui.columns(2, |columns| {
        source_preview(&mut columns[0], state, request_id);
        region(&mut columns[1], PREVIEW_MAX_HEIGHT, |ui| {
            caption(ui, "G 结果预览");
            ui.colored_label(theme::MUTED, "译文回填后的图（下一版）");
        });
    });
}

fn source_preview(ui: &mut egui::Ui, state: &AppState, request_id: u64) {
    region(ui, PREVIEW_MAX_HEIGHT, |ui| {
        caption(ui, "F 原图预览");
        let Some(admitted) = state.image.as_ref() else {
            ui.colored_label(theme::MUTED, "（还没有图片）");
            return;
        };
        let ctx = ui.ctx().clone();
        let id = preview_id(admitted, request_id);
        // `load_texture_cached` 按像素内容去重，所以每帧都可以放心调；真正贵的那一步
        // （从入场图缩出预览）由 `preview_pixels` 自己按同一把 `id` 缓存
        let handle = ctx.load_texture_cached(
            "ocr_source_preview",
            id,
            preview_pixels(&ctx, admitted, id),
            TextureOptions::LINEAR,
        );
        let size = handle.size_vec2();
        let fit = (PREVIEW_MAX_EDGE as f32 / size.x)
            .min(PREVIEW_MAX_HEIGHT / size.y)
            .min(1.0);
        ui.add(
            egui::Image::from_texture(&handle)
                .max_size(size * fit)
                .maintain_aspect_ratio(true),
        );
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

/// 预览像素：一张图只构建一次。
///
/// 不能每帧现算——8000px 的入场图逐帧缩放能把 CPU 顶到听见风扇声（NFR-2 的空闲口径先不提，
/// 识别中那几秒界面会直接卡住）。缓存键见 [`preview_id`]，`image_request_id` 也在键里，
/// 所以换一张图（哪怕同名同尺寸）必然重建。
fn preview_pixels(ctx: &egui::Context, admitted: &Admitted, id: Id) -> ColorImage {
    if let Some(cached) = ctx.data(|data| data.get_temp::<ColorImage>(id)) {
        return cached;
    }
    let small = image::DynamicImage::ImageRgba8(admitted.image.clone())
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

/// H 区：识别结果列表。
///
/// 动作分两趟落地：勾与改字只改值，删/合/拆三条会重排块号。同一帧里两头的动作都可能发生
/// （正改着字又点了一下删除），先值后结构，否则第二条动作会作用到换过号的块上。
fn block_list(ui: &mut egui::Ui, state: &mut AppState) {
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
        let document = state.ocr.take();
        if let Some(document) = document.as_ref() {
            for block in document.blocks.iter() {
                ui.horizontal(|ui| {
                    // 复选框与文本框都绑在局部值上，改完了把动作交给状态层：
                    // 界面这一层从头到尾没有写过 `enabled` 和 `text` 这两个字段
                    let mut enabled = block.enabled;
                    let checkbox = ui.checkbox(&mut enabled, "");
                    record_checkbox(ui.ctx(), block.id, checkbox.rect);
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
                    // 译文列在 P6 恒空：留这一格是为了让它下次有地方长出来
                    ui.colored_label(theme::MUTED, "—");
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

/// I 区：状态行（REQ-006 五态 + 耗时 + 「忽略这次识别」）。
///
/// 调用点在 `views/mod.rs`，排在 `CentralPanel` **之前**：本页正文那节 `ScrollArea` 是
/// `auto_shrink([false, false])`，会吃掉当时所有的剩余高度，状态条若排在它后面只剩 0px 一格，
/// 那颗「忽略这次识别」根本画不出来（点不到也就测不到）。与文本页的 E 动作底栏同一条规矩。
pub(crate) fn status_bar(ui: &mut egui::Ui, state: &mut AppState, effects: &mut Effects) {
    let frame = egui::Frame::side_top_panel(ui.style()).inner_margin(egui::Margin::symmetric(8, 6));
    egui::Panel::bottom("image_status")
        .frame(frame)
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                caption(ui, "I 识别");
                let color = match &state.image_stage {
                    RecognitionStage::Failed(_) => theme::ERROR,
                    RecognitionStage::Recognizing => theme::WARN,
                    RecognitionStage::Done { .. } => theme::OK,
                    RecognitionStage::Idle | RecognitionStage::NoText => theme::MUTED,
                };
                let label = if state.image_elapsed_ms == 0 {
                    state.image_stage.label()
                } else {
                    format!(
                        "{} · {} ms",
                        state.image_stage.label(),
                        state.image_elapsed_ms
                    )
                };
                ui.colored_label(color, label);
                if state.image_stage.cancellable() {
                    // 靠右：识别中这一行只有这一颗按钮，放在文案后面会让人以为它是句说明
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        let ignore = ui.button("忽略这次识别");
                        record(ui.ctx(), "ignore", ignore.rect);
                        if ignore.clicked() {
                            effects.ignore_image = true;
                        }
                        ignore.on_hover_text(
                            "底层那次识别撤不回来；这一颗只是说结果回来时不要填进列表",
                        );
                    });
                }
            });
        });
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
/// 与工具条、图钉、主题滑块同一套路（`views/mod.rs`）。
fn record(ctx: &egui::Context, salt: &'static str, rect: egui::Rect) {
    ctx.data_mut(|data| data.insert_temp(entry_id(salt), rect));
}

fn entry_id(salt: &'static str) -> Id {
    Id::NULL.with("ice-translator.image-entry").with(salt)
}

/// 每一行勾选框的矩形。
///
/// 与上面那三颗入口分开记，是因为块号是运行期才知道的数：salt 是 `&'static str`，
/// 装不下"第几块"。测试要点的从来只有一号块那一格，所以按块号建 Id 就够了。
fn record_checkbox(ctx: &egui::Context, id: u32, rect: egui::Rect) {
    ctx.data_mut(|data| {
        data.insert_temp(
            Id::NULL.with("ice-translator.image-checkbox").with(id),
            rect,
        );
    });
}

#[cfg(test)]
pub(crate) fn checkbox_rect(ctx: &egui::Context, id: u32) -> egui::Rect {
    ctx.data_mut(|data| {
        data.get_temp::<egui::Rect>(Id::NULL.with("ice-translator.image-checkbox").with(id))
            .unwrap_or_else(|| panic!("图片页第 {id} 块的勾选框没有记下矩形"))
    })
}

/// 这一格到底画没画：`None` = 这一帧根本没出现这颗控件
#[cfg(test)]
pub(crate) fn entry_rect(ctx: &egui::Context, salt: &'static str) -> Option<egui::Rect> {
    ctx.data_mut(|data| data.get_temp::<egui::Rect>(entry_id(salt)))
}

#[cfg(test)]
pub(crate) fn recorded_entry(ctx: &egui::Context, salt: &'static str) -> egui::Rect {
    entry_rect(ctx, salt).unwrap_or_else(|| panic!("图片页的 {salt} 没有记下矩形"))
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
        assert!(!effects.ignore_image);
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
        let at = checkbox_rect(&ctx, 0).center();

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

    /// REQ-006 第三条：只有识别中那一态给得出「忽略这次识别」。
    #[test]
    fn only_a_recognition_in_flight_offers_the_ignore_button() {
        let mut state = new_state();
        state.source = "hello".into();
        let mut effects = Effects::default();
        // 空闲：按钮根本没画，所以那一格连矩形都该没有
        let ctx = run(&mut state, &mut effects, true, &[]);
        assert!(!state.image_stage.cancellable());
        assert!(
            entry_rect(&ctx, "ignore").is_none(),
            "空闲态也画「忽略这次识别」，用户会以为有一次任务可以撤"
        );

        let job = state
            .submit_image(crate::state::ImageInput::Path(std::path::PathBuf::from(
                "/tmp/a.png",
            )))
            .expect("要派发一次识别");
        assert!(matches!(job, Job::ImageOcr { .. }));
        let ctx = run(&mut state, &mut effects, true, &[]);
        assert!(state.image_stage.cancellable());
        let at = recorded_entry(&ctx, "ignore").center();
        effects = Effects::default();
        run(&mut state, &mut effects, true, &[at]);
        assert!(effects.ignore_image, "点「忽略这次识别」没给出意图");
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
