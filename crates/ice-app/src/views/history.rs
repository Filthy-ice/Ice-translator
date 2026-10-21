//! 历史页（FR-4.4、REQ-006）。
//!
//! 搜索即重查、点一条只回填不重译。文件损坏或缺失时显示"历史不可用 + 原因"，
//! 翻译照常进行——历史是附属功能，不能让它把主流程带崩。

use egui::{self, Align, Layout, RichText};
use ice_core::history::{HISTORY_CAPACITY, HistoryRecord, HistoryStore};

use crate::state::{AppState, Confirm, Tone};
use crate::theme;

/// 一行历史的高度：时间戳 + 引擎 + 单行预览
const ROW_HEIGHT: f32 = 24.0;
/// 预览截断长度，避免长文本把行撑爆（列表里只需要认出是哪条）
const PREVIEW_CHARS: usize = 60;

pub fn show(ui: &mut egui::Ui, state: &mut AppState, history: &HistoryStore) {
    ui.spacing_mut().item_spacing.y = 8.0;
    toolbar(ui, state, history);
    if let Some(reason) = &state.history_error {
        ui.colored_label(theme::ERROR, reason);
    }
    list(ui, state);
}

fn toolbar(ui: &mut egui::Ui, state: &mut AppState, history: &HistoryStore) {
    ui.horizontal(|ui| {
        ui.label(RichText::new("搜索").strong());
        let response = ui.add(
            egui::TextEdit::singleline(&mut state.history_query)
                .id(egui::Id::unique("history_query"))
                .desired_width(280.0)
                .hint_text("按源文或译文的子串匹配"),
        );
        // 每敲一个字就重查一次：历史最多 500 行，重读文件的开销远小于加一层防抖状态机
        if response.changed() {
            state.reload_history(history);
        }
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            ui.colored_label(
                theme::MUTED,
                format!("共 {} 条（上限 {HISTORY_CAPACITY}）", state.history_count),
            );
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

fn list(ui: &mut egui::Ui, state: &mut AppState) {
    if state.history_rows.is_empty() {
        ui.colored_label(
            theme::MUTED,
            if state.history_query.trim().is_empty() {
                "还没有历史：翻译成功后会自动记在这里"
            } else {
                "没有匹配的历史"
            },
        );
        return;
    }
    let width = ui.available_width();
    egui::ScrollArea::vertical()
        .auto_shrink([false, false])
        .show(ui, |ui| {
            // 先取出要回填的下标：闭包里不能同时借用 state 的两半
            let mut picked = None;
            for (index, row) in state.history_rows.iter().enumerate() {
                let text = row_text(row);
                if ui
                    .add(
                        egui::Button::new(RichText::new(text))
                            .min_size(egui::vec2(width, ROW_HEIGHT)),
                    )
                    .on_hover_text("回填到文本页，不会自动翻译")
                    .clicked()
                {
                    picked = Some(index);
                }
            }
            if let Some(index) = picked {
                load(state, index);
            }
        });
}

/// 点一条历史：回填源文与译文并切到文本页，**不发请求**（REQ-006）
pub fn load(state: &mut AppState, index: usize) {
    if state.load_history_row(index) {
        state.toast(Tone::Info, "已回填到文本页，未自动翻译");
    }
}

/// 一行文案：`时间 · 引擎 · 源→目标 · 预览`。全部来自已落盘的记录，不查网络。
pub fn row_text(row: &HistoryRecord) -> String {
    let stamp = stamp_of(row.ts)
        .map(|s| format_stamp(&s))
        .unwrap_or_else(|| "--".to_string());
    let detected = row
        .detected
        .map(|lang| lang.display_name().to_string())
        .unwrap_or_else(|| "自动".to_string());
    let preview = one_line(&row.source_text, PREVIEW_CHARS);
    format!(
        "{stamp}  {}  {detected} → {}  {preview}",
        row.engine.display_name(),
        row.target.display_name()
    )
}

/// 去掉换行与制表符：列表是单行的，原文里的换行会把行高撑乱
fn one_line(text: &str, chars: usize) -> String {
    let mut out: String = text
        .chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .take(chars)
        .collect();
    if text.chars().count() > chars {
        out.push('…');
    }
    out.trim().to_string()
}

/// 本地日历时间。`libc` 已是 unix 侧依赖，用它拿时区偏移；
/// 非 unix 平台没有时区库，按 UTC 显示（P8 的跨平台检查会覆盖这条）。
pub fn stamp_of(ts: i64) -> Option<Stamp> {
    let local = ts.checked_add(local_offset_secs(ts)?)?;
    let days = local.div_euclid(86_400);
    let secs = local.rem_euclid(86_400);
    let (year, month, day) = civil_from_days(days);
    Some(Stamp {
        year,
        month,
        day,
        hour: (secs / 3_600) as u32,
        minute: ((secs % 3_600) / 60) as u32,
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Stamp {
    pub year: i32,
    pub month: u32,
    pub day: u32,
    pub hour: u32,
    pub minute: u32,
}

pub fn format_stamp(stamp: &Stamp) -> String {
    format!(
        "{:04}-{:02}-{:02} {:02}:{:02}",
        stamp.year, stamp.month, stamp.day, stamp.hour, stamp.minute
    )
}

// `time_t` 与 `tm_gmtoff` 在 64 位平台上就是 i64，转换看着多余；32 位平台上它们是 i32，
// 去掉转换就编译不过。项目要跨平台，所以按 32 位写死并对本平台的告警放行。
#[cfg(unix)]
#[allow(clippy::useless_conversion)]
fn local_offset_secs(ts: i64) -> Option<i64> {
    let time: libc::time_t = ts.try_into().ok()?;
    let mut tm: libc::tm = unsafe { std::mem::zeroed() };
    if unsafe { libc::localtime_r(&time, &mut tm) }.is_null() {
        return None;
    }
    Some(i64::from(tm.tm_gmtoff))
}

#[cfg(not(unix))]
fn local_offset_secs(_ts: i64) -> Option<i64> {
    Some(0)
}

/// 天数 → 公历年月日（Howard Hinnant 的 `civil_from_days`，1970-01-01 为第 0 天）
fn civil_from_days(days: i64) -> (i32, u32, u32) {
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    ((y + i64::from(m <= 2)) as i32, m, d)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::{KeyState, Tab};
    use ice_core::config::Config;
    use ice_core::glossary::Glossary;
    use ice_core::language::{EngineKind, Language};

    fn state() -> AppState {
        AppState::new(
            Config::default(),
            Glossary::default(),
            KeyState::Missing,
            None,
        )
    }

    fn record(source: &str, translation: &str) -> HistoryRecord {
        HistoryRecord {
            ts: 1_760_000_000,
            engine: EngineKind::Machine,
            detected: Some(Language::En),
            target: Language::ZhHans,
            source_text: source.into(),
            result_text: translation.into(),
            elapsed_ms: 220,
        }
    }

    fn temp_store() -> (tempfile::TempDir, HistoryStore) {
        let dir = tempfile::tempdir().unwrap();
        let store = HistoryStore::at(dir.path().join("history.jsonl"));
        (dir, store)
    }

    fn render(state: &mut AppState, store: &HistoryStore) {
        let ctx = egui::Context::default();
        let raw = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(1080.0, 720.0),
            )),
            ..Default::default()
        };
        let output = ctx.run_ui(raw, |ui| show(ui, state, store));
        output.drop_without_applying_deltas();
    }

    #[test]
    fn rendering_the_history_page_does_not_submit_anything() {
        let (_dir, store) = temp_store();
        store.append(&record("hello", "你好")).unwrap();
        let mut state = state();
        state.reload_history(&store);
        assert_eq!(state.history_count, 1);

        render(&mut state, &store);
        assert_eq!(state.tab, Tab::Text, "渲染本身不该切页");
        assert!(state.in_flight.is_none());
        assert!(state.translation.is_none());
    }

    #[test]
    fn picking_a_row_fills_the_text_page_without_translating() {
        let mut state = state();
        state.history_rows = vec![record("install the package", "安装该包")];

        load(&mut state, 0);

        assert_eq!(state.source, "install the package");
        assert_eq!(
            state.translation.as_ref().map(|r| r.text.as_str()),
            Some("安装该包")
        );
        assert_eq!(state.tab, Tab::Text);
        assert!(state.in_flight.is_none(), "回填绝不自动发请求");
        assert_eq!(state.toasts[0].tone, Tone::Info);
    }

    #[test]
    fn picking_an_absent_row_is_a_no_op() {
        let mut state = state();
        state.source = "保持原样".into();
        load(&mut state, 7);
        assert_eq!(state.source, "保持原样");
        assert!(state.toasts.is_empty());
    }

    #[test]
    fn corrupt_history_lines_are_skipped_and_the_page_stays_usable() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("history.jsonl");
        std::fs::write(&path, "这不是 JSON\n").unwrap();
        let store = HistoryStore::at(path);

        let mut state = state();
        state.reload_history(&store);
        // ice-core 的约定：坏行跳过并留 debug 日志，一行坏不该让整页报错
        assert!(state.history_rows.is_empty());
        assert!(state.history_error.is_none());
        state.source = "hello".into();
        assert!(state.submit().is_some());
    }

    #[test]
    fn an_unreadable_history_file_reports_a_reason() {
        let dir = tempfile::tempdir().unwrap();
        // 用目录冒充历史文件：能打开但读不出内容
        let path = dir.path().join("history.jsonl");
        std::fs::create_dir(&path).unwrap();
        let store = HistoryStore::at(path);

        let mut state = state();
        state.reload_history(&store);
        let reason = state.history_error.clone().expect("要给出可读原因");
        assert!(reason.contains("历史不可用"), "{reason}");
        assert!(state.history_rows.is_empty());
        // 历史坏了不影响翻译：源文与提交路径都还在
        state.source = "hello".into();
        assert!(state.submit().is_some());
    }

    #[test]
    fn row_text_is_single_line_and_truncated() {
        let long = "a\nb\tc".to_string() + &"长".repeat(200);
        let text = row_text(&record(&long, "x"));
        assert!(!text.contains('\n'), "{text}");
        assert!(text.contains('…'), "{text}");
        assert!(text.chars().count() < 140, "{text}");
        assert!(text.contains("机翻"), "{text}");
        assert!(text.contains("英语 → 简体中文"), "{text}");
    }

    #[test]
    fn control_characters_are_flattened_not_rendered() {
        assert_eq!(one_line("a\u{7}b", 10), "a b");
        assert_eq!(one_line("  前后空白  ", 10), "前后空白");
    }

    #[test]
    fn civil_from_days_matches_known_dates() {
        assert_eq!(civil_from_days(0), (1970, 1, 1));
        assert_eq!(civil_from_days(-1), (1969, 12, 31));
        // 2026-10-20 距 1970-01-01 共 20746 天
        assert_eq!(civil_from_days(20_746), (2026, 10, 20));
        assert_eq!(civil_from_days(18_262), (2020, 1, 1));
    }

    #[test]
    fn stamp_formatting_is_zero_padded() {
        let stamp = Stamp {
            year: 2026,
            month: 3,
            day: 5,
            hour: 9,
            minute: 7,
        };
        assert_eq!(format_stamp(&stamp), "2026-03-05 09:07");
    }

    #[test]
    fn unix_epoch_zero_is_a_valid_stamp() {
        let stamp = stamp_of(0).expect("纪元 0 必须能显示");
        assert!(stamp.hour < 24 && stamp.minute < 60);
        assert_eq!(format_stamp(&stamp).len(), 16);
    }
}
