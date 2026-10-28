//! 设置页（FR-4.1~FR-4.5、REQ-005）。
//!
//! 分四组：AI 服务 / 翻译默认值 / 外观 / 数据（含术语分区 F-12）。
//! 没有"保存"按钮：任何改动即 `mark_config_dirty()`，帧末由 `app.rs` 原子落盘。
//!
//! Key 的硬约束（FR-4.2、ADR-005）：明文只活在 `state.key_input` 这一个内存缓冲里，
//! 输入框是 `password`，界面**不回显任何片段**（连后 4 位也不给），长度与哈希前缀只进日志。

use egui::{self, Align, Layout, RichText};
use ice_core::config::{AiProvider, MAX_UNFOCUSED_ALPHA, MIN_UNFOCUSED_ALPHA, Theme};
use ice_core::history::HistoryStore;
use ice_core::language::{EngineKind, Language};

use crate::state::{AppState, Confirm, KeyState, Tone};
use crate::theme::{self, MAX_FONT_SCALE, MIN_FONT_SCALE};

use super::Effects;

/// Key 输入框的稳定 Id：错误动作"去设置"要把焦点送到这里（REQ-004）
pub const KEY_INPUT_ID: &str = "ai_key_input";
const FIELD_WIDTH: f32 = 320.0;

pub fn show(ui: &mut egui::Ui, state: &mut AppState, effects: &mut Effects, ai_available: bool) {
    egui::ScrollArea::vertical()
        .auto_shrink([false, false])
        .show(ui, |ui| {
            ui.spacing_mut().item_spacing.y = 10.0;
            ai_group(ui, state, effects, ai_available);
            window_group(ui, effects);
            defaults_group(ui, state, effects);
            appearance_group(ui, state);
            data_group(ui, state, effects);
        });
}

/// 窗口这一组（REQ-011）：两颗只有按钮、没有开关的"应急出口"。
///
/// 用户第二轮走查的原话是"简洁模式要能够缩小到最小地步，并且有退出按钮"——退出这件事
/// 在界面上本来只有 Cmd+Q 和 Activity，看不见就等于没有；而"收进后台"与"退出"长得一样
/// （都是窗口没了），所以这颗按钮旁边必须写明差别。
///
/// 「恢复默认窗口尺寸」管的是缺陷留下的坏数据（D-10 把两个尺寸槽写成一样）：
/// 程序不猜哪个值本该属于谁，也不自动改写，只有用户按下去才恢复。
fn window_group(ui: &mut egui::Ui, effects: &mut Effects) {
    let ctx = ui.ctx().clone();
    group(ui, "窗口", |ui| {
        ui.horizontal(|ui| {
            let reset = ui.button("恢复默认窗口尺寸");
            record(&ctx, "btn_reset", reset.rect);
            ui.label(
                RichText::new("两种形态各回到默认尺寸并居中（窗口被历史版本写坏过时用）")
                    .color(theme::MUTED),
            );
            if reset.clicked() {
                effects.reset_geometry = true;
            }
        });
        ui.horizontal(|ui| {
            let quit = ui.button("退出程序");
            record(&ctx, "btn_quit", quit.rect);
            ui.label(RichText::new("红灯 / Cmd+W 是收进后台，不是退出").color(theme::MUTED));
            if quit.clicked() {
                effects.quit = true;
            }
        });
    });
}

/// 把控件矩形记在临时数据里，供无头测试命中点击（与 `compact::record` 同一套办法）：
/// 这两颗按钮的行为契约是"置一个意图"，只能真的点一下才知道接没接上。
fn record(ctx: &egui::Context, salt: &str, rect: egui::Rect) {
    ctx.data_mut(|d| d.insert_temp(rect_id(salt), rect));
}

fn rect_id(salt: &str) -> egui::Id {
    egui::Id::NULL.with("ice-translator.settings").with(salt)
}

fn group(ui: &mut egui::Ui, title: &str, body: impl FnOnce(&mut egui::Ui)) {
    ui.group(|ui| {
        ui.set_width(ui.available_width());
        ui.label(RichText::new(title).strong().size(15.0));
        ui.add_space(4.0);
        body(ui);
    });
    ui.add_space(4.0);
}

/// 一行"标签 + 控件"，标签列等宽，控件才不会左对齐得歪歪扭扭
fn row(ui: &mut egui::Ui, label: &str, body: impl FnOnce(&mut egui::Ui)) {
    ui.horizontal(|ui| {
        ui.label(RichText::new(label).color(theme::MUTED));
        ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
            ui.set_min_width(FIELD_WIDTH);
            body(ui);
        });
    });
}

// ---- 1. AI 服务 ----

fn ai_group(ui: &mut egui::Ui, state: &mut AppState, effects: &mut Effects, ai_available: bool) {
    group(ui, "AI 服务", |ui| {
        provider_picker(ui, state, effects);
        base_url_field(ui, state, effects);
        model_field(ui, state, effects);
        key_fields(ui, state, effects);
        ui.add_space(2.0);
        ui.colored_label(
            if ai_available { theme::OK } else { theme::WARN },
            if ai_available {
                "AI 通道可用"
            } else {
                "AI 未配置：选好供应商、保存 Key，工具条上的 AI 才能选"
            },
        );
    });
}

fn provider_picker(ui: &mut egui::Ui, state: &mut AppState, effects: &mut Effects) {
    let selected = state.config.ai.provider;
    row(ui, "供应商", |ui| {
        egui::ComboBox::from_id_salt("ai_provider")
            .selected_text(selected.display_name())
            .show_ui(ui, |ui| {
                for provider in AiProvider::all() {
                    if ui
                        .selectable_label(selected == provider, provider.display_name())
                        .clicked()
                    {
                        select_provider(state, effects, provider);
                    }
                }
            });
        ui.label(
            RichText::new("预设只是少打字，地址与模型都还能手改；选完即生效，这一栏没有保存按钮")
                .color(theme::MUTED)
                .small(),
        );
    });
}

/// 选预设：把 `preset_base_url()` / `preset_model()` 填进**编辑缓冲**（就是 `cfg.ai` 那两个字段）。
/// "手填优先于预设"的取值规则由 `AiConfig::effective_*` 保证，UI 不重复实现（REQ-005）。
pub fn select_provider(state: &mut AppState, effects: &mut Effects, provider: AiProvider) {
    state.config.ai.provider = provider;
    state.config.ai.base_url = provider.preset_base_url().to_string();
    state.config.ai.model = provider.preset_model().to_string();
    state.mark_config_dirty();
    effects.rebuild_translator = true;
    if provider.needs_manual_model() {
        state.toast(Tone::Info, "这一家没有默认模型名，请填写模型名或接入点 ID");
    }
}

fn base_url_field(ui: &mut egui::Ui, state: &mut AppState, effects: &mut Effects) {
    row(ui, "接口地址", |ui| {
        let response = ui.add(
            egui::TextEdit::singleline(&mut state.config.ai.base_url)
                .id(egui::Id::unique("ai_base_url"))
                .desired_width(FIELD_WIDTH)
                .hint_text("https://api.example.com/v1"),
        );
        if response.changed() {
            state.mark_config_dirty();
        }
        // 每敲一个字符就重建引擎没有意义，等焦点离开再重建
        if response.lost_focus() {
            effects.rebuild_translator = true;
        }
    });
    row(ui, "", |ui| {
        ui.label(
            RichText::new(format!("实际请求：{}", state.config.ai.endpoint()))
                .color(theme::MUTED)
                .small(),
        );
    });
}

fn model_field(ui: &mut egui::Ui, state: &mut AppState, effects: &mut Effects) {
    row(ui, "模型名", |ui| {
        let response = ui.add(
            egui::TextEdit::singleline(&mut state.config.ai.model)
                .id(egui::Id::unique("ai_model"))
                .desired_width(FIELD_WIDTH)
                .hint_text("留空则用供应商预设"),
        );
        if response.changed() {
            state.mark_config_dirty();
        }
        if response.lost_focus() {
            effects.rebuild_translator = true;
        }
    });
}

fn key_fields(ui: &mut egui::Ui, state: &mut AppState, effects: &mut Effects) {
    row(ui, "API Key", |ui| {
        let response = ui.add(
            egui::TextEdit::singleline(&mut state.key_input)
                .id(egui::Id::unique(KEY_INPUT_ID))
                .password(true)
                .desired_width(FIELD_WIDTH)
                .hint_text("粘贴后点保存，明文只进钥匙串"),
        );
        // 错误动作跳过来时把焦点直接放这里，省得用户再找一次
        if std::mem::take(&mut state.focus_key_input) {
            response.request_focus();
        }
        let filled = !state.key_input.trim().is_empty();
        if ui
            .add_enabled(filled, egui::Button::new("保存 Key"))
            .on_disabled_hover_text("先粘贴 Key")
            .clicked()
        {
            save_key(state, effects);
        }
        if ui
            .add_enabled(state.key.configured(), egui::Button::new("清除"))
            .on_hover_text("会先要你确认")
            .clicked()
        {
            state.confirm = Some(Confirm::ClearKey);
        }
    });
    row(ui, "", |ui| {
        // 只显示"已配置"与长度：不回显任何可还原片段（FR-4.2）
        match &state.key {
            KeyState::Configured { len, .. } => {
                ui.colored_label(theme::OK, format!("已配置（{len} 位，存在系统钥匙串）"));
            }
            KeyState::Missing => {
                ui.colored_label(theme::MUTED, "未配置");
            }
            KeyState::Unknown => {
                ui.colored_label(theme::MUTED, "尚未读取");
            }
            KeyState::Error(reason) => {
                ui.add(
                    egui::Label::new(
                        RichText::new(format!("钥匙串不可用：{reason}")).color(theme::ERROR),
                    )
                    .wrap(),
                );
            }
        }
    });
}

/// 保存 Key：只把明文交给 `effects`，真正的钥匙串写入与 `key_configured` 落盘由 `app.rs` 做，
/// 失败时由它给出可读错误（REQ-005 的验证层次要求 view 不碰系统）。
pub fn save_key(state: &mut AppState, effects: &mut Effects) {
    let key = state.key_input.trim();
    if key.is_empty() {
        return;
    }
    effects.save_key = Some(key.to_string());
    effects.rebuild_translator = true;
    // 立刻清空内存缓冲：界面上不留可读的明文，也不给"再点一次"的机会
    state.key_input.clear();
}

// ---- 2. 翻译默认值 ----

fn defaults_group(ui: &mut egui::Ui, state: &mut AppState, effects: &mut Effects) {
    group(ui, "翻译默认值", |ui| {
        default_engine(ui, state, effects);
        default_pair(ui, state, effects);
        extra_languages(ui, state);
    });
}

/// 默认引擎改动：写进配置，同时作用于当前会话——改完就用新引擎重译，省得再去工具条点一次
pub fn select_default_engine(state: &mut AppState, effects: &mut Effects, kind: EngineKind) {
    state.config.translation.default_engine = kind;
    state.mark_config_dirty();
    effects.request(state.set_engine(kind));
}

fn default_engine(ui: &mut egui::Ui, state: &mut AppState, effects: &mut Effects) {
    let selected = state.config.translation.default_engine;
    row(ui, "默认引擎", |ui| {
        egui::ComboBox::from_id_salt("default_engine")
            .selected_text(selected.display_name())
            .show_ui(ui, |ui| {
                for kind in EngineKind::all() {
                    if ui
                        .selectable_label(selected == kind, kind.display_name())
                        .clicked()
                    {
                        select_default_engine(state, effects, kind);
                    }
                }
            });
        ui.label(
            RichText::new("同时切换当前引擎并重译")
                .color(theme::MUTED)
                .small(),
        );
    });
}

fn default_pair(ui: &mut egui::Ui, state: &mut AppState, effects: &mut Effects) {
    let source = state.config.translation.source;
    let target = state.config.translation.target;

    row(ui, "默认源语言", |ui| {
        let languages = state.visible_languages();
        egui::ComboBox::from_id_salt("default_source")
            .selected_text(source.map(|lang| lang.display_name()).unwrap_or("自动检测"))
            .show_ui(ui, |ui| {
                if ui.selectable_label(source.is_none(), "自动检测").clicked() {
                    state.set_source_language(None);
                }
                for lang in languages {
                    if ui
                        .selectable_label(source == Some(lang), lang.display_name())
                        .clicked()
                    {
                        state.set_source_language(Some(lang));
                    }
                }
            });
    });

    row(ui, "默认目标语言", |ui| {
        let languages = state.visible_languages();
        egui::ComboBox::from_id_salt("default_target")
            .selected_text(target.display_name())
            .show_ui(ui, |ui| {
                for lang in languages {
                    if ui
                        .selectable_label(target == lang, lang.display_name())
                        .clicked()
                    {
                        // FR-1.4：记住最近使用；FR-1.3：切了就用当前源文重译
                        effects.request(state.set_target(lang));
                    }
                }
            });
    });
}

/// 常用三语（中/英/日）恒在列表里，其余按需启用（FR-1.4）
fn extra_languages(ui: &mut egui::Ui, state: &mut AppState) {
    row(ui, "更多语言", |ui| {
        ui.vertical(|ui| {
            for lang in optional_languages() {
                let mut enabled = state.config.translation.extra_languages.contains(&lang);
                if ui.checkbox(&mut enabled, lang.display_name()).changed() {
                    if enabled {
                        state.config.translation.extra_languages.push(lang);
                    } else {
                        state
                            .config
                            .translation
                            .extra_languages
                            .retain(|l| *l != lang);
                    }
                    state.mark_config_dirty();
                }
            }
        });
    });
}

/// 可勾选的语言 = 全部语言减去常驻的常用语言。纯函数，便于断言。
pub fn optional_languages() -> Vec<Language> {
    Language::all()
        .iter()
        .copied()
        .filter(|lang| !Language::preferred().contains(lang))
        .collect()
}

// ---- 3. 外观 ----

fn appearance_group(ui: &mut egui::Ui, state: &mut AppState) {
    group(ui, "外观", |ui| {
        let selected = state.config.ui.theme;
        row(ui, "主题", |ui| {
            for theme in [Theme::System, Theme::Light, Theme::Dark] {
                if ui
                    .selectable_label(selected == theme, theme_label(theme))
                    .clicked()
                {
                    state.config.ui.theme = theme;
                    state.mark_config_dirty();
                }
            }
            // 生效在 `theme::apply`（app.rs 每帧调）：这里只写配置，view 不碰 Context
        });
        row(ui, "字号", |ui| {
            if ui
                .add(
                    egui::Slider::new(
                        &mut state.config.ui.font_scale,
                        MIN_FONT_SCALE..=MAX_FONT_SCALE,
                    )
                    .text("倍率")
                    .fixed_decimals(1),
                )
                .changed()
            {
                // 滑杆本身会夹在区间内，这里再夹一次是防坏配置
                state.config.ui.font_scale = state
                    .config
                    .ui
                    .font_scale
                    .clamp(MIN_FONT_SCALE, MAX_FONT_SCALE);
                state.mark_config_dirty();
            }
        });
        row(ui, "窗口", |ui| {
            if ui
                .checkbox(
                    &mut state.config.ui.always_on_top,
                    "始终置顶（Cmd+Shift+P）",
                )
                .changed()
            {
                state.mark_config_dirty();
            }
            // FR-3.7（K-08）：开关在 P4 就有字段，本包才真的接线到绘制上
            if ui
                .checkbox(
                    &mut state.config.ui.dim_when_unfocused,
                    "悬浮窗失焦时半透明",
                )
                .on_hover_text(
                    "只在简洁悬浮窗生效：没有焦点时整块窗（含标题栏）透出桌面，聚焦后恢复 100%",
                )
                .changed()
            {
                state.mark_config_dirty();
            }
            // FR-3.8：这是"应用在后台读用户剪贴板"，开关文案必须明写它会做什么（REQ-008 隐私条）
            if ui
                .checkbox(&mut state.config.ui.clipboard_watch, "剪贴板即翻")
                .on_hover_text("开启后会读取你复制的文本并自动翻译；关闭时完全不读取剪贴板")
                .changed()
            {
                state.mark_config_dirty();
            }
        });
        // FR-3.7 的可调档（2026-10-28，用户走查："透明度需要能够在设置中调节"）。
        // 配置里存的是不透明度 0~1，滑杆给人读的是百分数：一次换算，1% 的档位两边都对得准，
        // 不必为界面再加一个字段。开关关掉时滑杆置灰——那一档已经不生效了，拖着没意义。
        row(ui, "半透明", |ui| {
            let mut percent = state.config.ui.unfocused_alpha * 100.0;
            let slider = egui::Slider::new(
                &mut percent,
                MIN_UNFOCUSED_ALPHA * 100.0..=MAX_UNFOCUSED_ALPHA * 100.0,
            )
            .text("不透明度")
            .suffix("%")
            .step_by(1.0)
            .fixed_decimals(0);
            if ui
                .add_enabled(state.config.ui.dim_when_unfocused, slider)
                .changed()
            {
                // 与字号那一条同样的护栏：滑杆自己会夹，这里再夹一次是防坏配置带进来的越界值
                state.config.ui.unfocused_alpha =
                    (percent / 100.0).clamp(MIN_UNFOCUSED_ALPHA, MAX_UNFOCUSED_ALPHA);
                state.mark_config_dirty();
            }
        });
    });
}

pub const fn theme_label(theme: Theme) -> &'static str {
    match theme {
        Theme::System => "跟随系统",
        Theme::Light => "浅色",
        Theme::Dark => "深色",
    }
}

// ---- 4. 数据（术语 F-12 + 历史入口） ----

fn data_group(ui: &mut egui::Ui, state: &mut AppState, effects: &mut Effects) {
    group(ui, "术语表", |ui| {
        ui.colored_label(
            theme::MUTED,
            format!(
                "{} 条 · 存在 config.toml 里，机翻在译后替换，AI 在提示词里带上（改动立即生效，无需清缓存）",
                state.glossary.terms.len()
            ),
        );
        term_editor(ui, state, effects);
        term_list(ui, state, effects);
    });
    group(ui, "历史", |ui| {
        row(ui, "条数", |ui| {
            ui.label(format!("{} 条", state.history_count));
        });
        row(ui, "文件", |ui| {
            let path = HistoryStore::default_path()
                .map(|p| p.display().to_string())
                .unwrap_or_else(|err| format!("不可用：{err}"));
            ui.label(RichText::new(path).color(theme::MUTED).small());
        });
        row(ui, "", |ui| {
            if ui.button("前往历史页").clicked() {
                state.tab = crate::state::Tab::History;
            }
            if ui
                .button("清空历史")
                .on_hover_text("不可撤销，会先要你确认")
                .clicked()
            {
                state.confirm = Some(Confirm::ClearHistory);
            }
        });
    });
}

fn term_editor(ui: &mut egui::Ui, state: &mut AppState, effects: &mut Effects) {
    ui.horizontal(|ui| {
        ui.add(
            egui::TextEdit::singleline(&mut state.term_source)
                .id(egui::Id::unique("term_source"))
                .desired_width(180.0)
                .hint_text("原文术语"),
        );
        ui.label("→");
        ui.add(
            egui::TextEdit::singleline(&mut state.term_target)
                .id(egui::Id::unique("term_target"))
                .desired_width(180.0)
                .hint_text("译名"),
        );
        let ready = !state.term_source.trim().is_empty() && !state.term_target.trim().is_empty();
        if ui
            .add_enabled(ready, egui::Button::new("添加"))
            .on_disabled_hover_text("原文与译名都要填")
            .clicked()
        {
            add_term(state, effects);
        }
    });
}

/// 增删术语后必须把新表推给调度层，否则已缓存的机翻结果还会用旧译名（FR-4.5）
pub fn add_term(state: &mut AppState, effects: &mut Effects) {
    if let Some(term) = state.add_term() {
        effects.sync_glossary = true;
        state.toast(
            Tone::Info,
            format!("已添加术语：{} → {}（立即生效）", term.source, term.target),
        );
    }
}

pub fn remove_term(state: &mut AppState, effects: &mut Effects, source: &str) {
    if state.remove_term(source) {
        effects.sync_glossary = true;
        state.toast(Tone::Info, format!("已删除术语：{source}"));
    }
}

fn term_list(ui: &mut egui::Ui, state: &mut AppState, effects: &mut Effects) {
    if state.glossary.terms.is_empty() {
        ui.colored_label(theme::MUTED, "还没有术语");
        return;
    }
    // 先记下要删的那条：不能在迭代 `state.glossary.terms` 的同时改它
    let mut removal = None;
    for term in &state.glossary.terms {
        ui.horizontal(|ui| {
            ui.label(format!("{} → {}", term.source, term.target));
            if term.case_sensitive {
                ui.label(RichText::new("区分大小写").color(theme::MUTED).small());
            }
            if ui.small_button("删除").clicked() {
                removal = Some(term.source.clone());
            }
        });
    }
    if let Some(source) = removal {
        remove_term(state, effects, &source);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
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

    fn render(state: &mut AppState, effects: &mut Effects, ai_available: bool) {
        let ctx = egui::Context::default();
        let raw = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(1080.0, 900.0),
            )),
            ..Default::default()
        };
        let output = ctx.run_ui(raw, |ui| show(ui, state, effects, ai_available));
        output.drop_without_applying_deltas();
    }

    /// 点击要跨帧：命中测试用的是**上一帧**的控件矩形，所以 `ctx` 得留着连跑几帧，
    /// 不能像 `render` 那样一帧一个新建的 Context
    struct Harness {
        ctx: egui::Context,
        size: egui::Vec2,
    }

    impl Harness {
        fn new(size: egui::Vec2) -> Self {
            Self {
                ctx: egui::Context::default(),
                size,
            }
        }

        fn step(&mut self, state: &mut AppState, effects: &mut Effects, events: Vec<egui::Event>) {
            let raw = egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, self.size)),
                events,
                ..Default::default()
            };
            let output = self.ctx.run_ui(raw, |ui| show(ui, state, effects, false));
            output.drop_without_applying_deltas();
        }

        fn warm(&mut self, state: &mut AppState, effects: &mut Effects) {
            for _ in 0..2 {
                self.step(state, effects, vec![]);
            }
        }

        fn click(&mut self, state: &mut AppState, effects: &mut Effects, at: egui::Pos2) {
            let button = |pressed| egui::Event::PointerButton {
                pos: at,
                button: egui::PointerButton::Primary,
                pressed,
                modifiers: egui::Modifiers::NONE,
            };
            self.step(
                state,
                effects,
                vec![egui::Event::PointerMoved(at), button(true)],
            );
            self.step(state, effects, vec![button(false)]);
        }

        fn rect(&self, salt: &str) -> egui::Rect {
            self.ctx
                .data_mut(|d| d.get_temp::<egui::Rect>(rect_id(salt)))
                .unwrap_or_else(|| panic!("没有记下 {salt} 的矩形"))
        }
    }

    /// REQ-011：这两颗是"应急出口"，点下去只置一个意图，剩下的事由 `app.rs` 接。
    /// 断言的是接线，不是文案：文案改了测试不管，意图没置上就是用户按了没反应。
    #[test]
    fn the_window_group_buttons_set_their_intent() {
        let mut h = Harness::new(egui::vec2(1080.0, 900.0));
        let mut state = state();
        let mut effects = Effects::default();
        h.warm(&mut state, &mut effects);

        h.click(&mut state, &mut effects, h.rect("btn_reset").center());
        assert!(
            effects.reset_geometry,
            "点了「恢复默认窗口尺寸」没置恢复意图"
        );

        let mut effects = Effects::default();
        h.click(&mut state, &mut effects, h.rect("btn_quit").center());
        assert!(effects.quit, "点了「退出程序」没置退出意图");
        assert!(
            !effects.reset_geometry,
            "两颗按钮共用了同一个意图：恢复尺寸不该顺手退出，反之也一样"
        );
    }

    /// 全量窗最小宽度（640×420，`config::MIN_FULL_*`）下两颗按钮仍要在窗内。
    /// REQ-011 的动机就是"看不见等于没有"，被裁出窗口的那颗和没有是同一回事。
    #[test]
    fn the_window_group_buttons_survive_the_narrowest_full_window() {
        let size = egui::vec2(640.0, 420.0);
        let mut h = Harness::new(size);
        let mut state = state();
        let mut effects = Effects::default();
        h.warm(&mut state, &mut effects);

        let win = egui::Rect::from_min_size(egui::Pos2::ZERO, size);
        for salt in ["btn_reset", "btn_quit"] {
            let rect = h.rect(salt);
            assert!(
                win.contains_rect(rect),
                "{salt} 出界：{rect:?} 不在 {win:?} 内"
            );
        }
    }

    #[test]
    fn rendering_the_settings_page_has_no_side_effects() {
        let mut state = state();
        let mut effects = Effects::default();
        render(&mut state, &mut effects, false);
        assert!(effects.job.is_none());
        assert!(effects.save_key.is_none());
        assert!(!effects.delete_key);
        assert!(!effects.rebuild_translator);
        assert!(!effects.sync_glossary);
        assert!(!state.config_dirty, "光看设置页不该改配置");
    }

    #[test]
    fn provider_preset_fills_the_edit_buffers() {
        let mut state = state();
        let mut effects = Effects::default();
        select_provider(&mut state, &mut effects, AiProvider::DeepSeek);

        assert_eq!(state.config.ai.base_url, "https://api.deepseek.com");
        assert_eq!(state.config.ai.model, "deepseek-flash");
        assert!(state.config_dirty);
        assert!(effects.rebuild_translator);
        assert!(state.toasts.is_empty(), "有默认模型名就不该多嘴");
    }

    #[test]
    fn ark_preset_asks_for_a_model_name() {
        let mut state = state();
        let mut effects = Effects::default();
        select_provider(&mut state, &mut effects, AiProvider::VolcengineArk);

        assert!(state.config.ai.model.is_empty());
        assert_eq!(state.toasts.len(), 1);
        assert!(
            state.toasts[0].text.contains("接入点 ID"),
            "{}",
            state.toasts[0].text
        );
    }

    #[test]
    fn manual_values_survive_a_preset_selection_and_still_win() {
        let mut state = state();
        let mut effects = Effects::default();
        select_provider(&mut state, &mut effects, AiProvider::DeepSeek);
        state.config.ai.base_url = "https://proxy.example.com/v1".into();

        // 取值规则在 core：手填优先于预设
        assert_eq!(
            state.config.ai.effective_base_url(),
            "https://proxy.example.com/v1"
        );
        assert_eq!(state.config.ai.effective_model(), "deepseek-flash");
    }

    #[test]
    fn custom_provider_without_a_url_is_not_ready() {
        let state = state();
        assert_eq!(state.config.ai.provider, AiProvider::Custom);
        assert!(!state.config.ai.is_ready());
        assert!(state.config.ai.effective_base_url().is_empty());
    }

    #[test]
    fn saving_a_key_hands_over_the_plaintext_once_and_clears_the_buffer() {
        let mut state = state();
        state.key_input = "  sk-test-value  ".into();
        let mut effects = Effects::default();

        save_key(&mut state, &mut effects);

        assert_eq!(effects.save_key.as_deref(), Some("sk-test-value"));
        assert!(state.key_input.is_empty(), "界面上不能留明文");
        assert!(effects.rebuild_translator);
        assert!(!state.config_dirty, "key_configured 要等真写进钥匙串再改");
    }

    #[test]
    fn an_empty_key_is_never_handed_over() {
        let mut state = state();
        state.key_input = "   ".into();
        let mut effects = Effects::default();
        save_key(&mut state, &mut effects);
        assert!(effects.save_key.is_none());
        assert_eq!(state.key_input, "   ", "没保存就别把用户输入吃掉");
    }

    #[test]
    fn a_configured_key_shows_only_its_length() {
        let state = AppState::new(
            Config::default(),
            Glossary::default(),
            KeyState::Configured {
                len: 51,
                hash8: "a1b2c3d4".into(),
            },
            None,
        );
        assert!(state.key.configured());
        // 界面上出现的只有长度，不含哈希，更不含明文
        assert_eq!(state.key.label(), "已配置");
    }

    #[test]
    fn adding_a_term_pushes_the_new_glossary_to_the_dispatcher() {
        let mut state = state();
        state.term_source = "pod".into();
        state.term_target = "舱体".into();
        let mut effects = Effects::default();

        add_term(&mut state, &mut effects);

        assert!(effects.sync_glossary);
        assert!(state.config_dirty);
        assert_eq!(state.config.terms.len(), 1, "术语要写进 config.toml");
        assert_eq!(state.config.terms[0].source, "pod");
        assert_eq!(state.config.terms[0].target, "舱体");
        assert!(state.term_source.is_empty() && state.term_target.is_empty());
        assert_eq!(state.toasts.len(), 1);
    }

    #[test]
    fn an_incomplete_term_is_rejected_silently() {
        let mut state = state();
        state.term_source = "pod".into();
        let mut effects = Effects::default();
        add_term(&mut state, &mut effects);
        assert!(!effects.sync_glossary);
        assert!(state.config.terms.is_empty());
        assert!(state.toasts.is_empty());
    }

    #[test]
    fn removing_a_term_also_syncs_and_dirties_the_config() {
        let mut state = state();
        state.term_source = "pod".into();
        state.term_target = "舱体".into();
        let mut effects = Effects::default();
        add_term(&mut state, &mut effects);

        let mut effects = Effects::default();
        remove_term(&mut state, &mut effects, "pod");

        assert!(effects.sync_glossary);
        assert!(state.glossary.terms.is_empty());
        assert!(state.config.terms.is_empty());
    }

    #[test]
    fn optional_languages_exclude_the_always_visible_ones() {
        let optional = optional_languages();
        for lang in Language::preferred() {
            assert!(
                !optional.contains(lang),
                "{lang:?} 常驻，不该出现在勾选列表里"
            );
        }
        assert!(optional.contains(&Language::De) || optional.len() > 1);
        assert_eq!(
            optional.len(),
            Language::all().len() - Language::preferred().len()
        );
    }

    #[test]
    fn changing_the_default_engine_also_switches_the_live_one() {
        let mut state = state();
        state.source = "hello".into();
        let mut effects = Effects::default();

        select_default_engine(&mut state, &mut effects, EngineKind::Ai);

        let job = effects.job.expect("有源文就该重译");
        assert_eq!(job.engine, EngineKind::Ai);
        assert_eq!(state.engine, EngineKind::Ai);
        assert_eq!(state.config.translation.default_engine, EngineKind::Ai);
        assert!(state.config_dirty, "默认值是配置项，必须落盘");
    }

    #[test]
    fn theme_labels_cover_all_three_states() {
        assert_eq!(theme_label(Theme::System), "跟随系统");
        assert_eq!(theme_label(Theme::Light), "浅色");
        assert_eq!(theme_label(Theme::Dark), "深色");
    }

    #[test]
    fn the_whole_page_renders_with_a_key_configured_and_a_full_glossary() {
        let mut config = Config::default();
        config.ai = ice_core::config::AiConfig {
            provider: AiProvider::DeepSeek,
            key_configured: true,
            ..Default::default()
        };
        config.terms = vec![
            ice_core::glossary::Term::new("pod", "舱体"),
            ice_core::glossary::Term::new("node", "节点"),
        ];
        let mut state = AppState::new(
            config.clone(),
            Glossary::from_config(&config),
            KeyState::Configured {
                len: 51,
                hash8: "a1b2c3d4".into(),
            },
            None,
        );
        state.history_count = 42;
        let mut effects = Effects::default();
        render(&mut state, &mut effects, true);
        assert_eq!(state.glossary.terms.len(), 2, "渲染不该改术语表");
    }
}
