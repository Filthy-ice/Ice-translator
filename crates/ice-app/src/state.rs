//! 界面状态与状态转移。
//!
//! 这里只放纯逻辑，不碰 egui 绘制，因此 REQ-001~REQ-004、REQ-007 的行为都能无头断言。

use ice_core::config::{Config, UiMode};
use ice_core::glossary::{Glossary, Term};
use ice_core::history::{HISTORY_CAPACITY, HistoryRecord, HistoryStore};
use ice_core::language::{EngineKind, Language};
use ice_providers::error::ProviderError;
use ice_providers::request::{TranslationRequest, TranslationResult};

pub type RequestId = u64;

/// 全量模式的四个页签（`03-ui-design.md` §2 F-01）。术语表是设置页内的一个分区（F-12），不占页签。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tab {
    Text,
    Image,
    History,
    Settings,
}

impl Tab {
    pub const fn all() -> [Tab; 4] {
        [Tab::Text, Tab::Image, Tab::History, Tab::Settings]
    }

    pub const fn label(self) -> &'static str {
        match self {
            Tab::Text => "文本",
            Tab::Image => "图片",
            Tab::History => "历史",
            Tab::Settings => "设置",
        }
    }
}

/// 只有需要"给出动作按钮"的错误才映射到动作（架构 §5 的规则）
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FailureAction {
    GoSettings,
    UseMachine,
}

impl FailureAction {
    pub const fn label(self) -> &'static str {
        match self {
            FailureAction::GoSettings => "去设置",
            FailureAction::UseMachine => "改用机翻",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Failure {
    pub message: String,
    pub action: Option<FailureAction>,
}

/// 错误文案一律取自 `ProviderError` 的 `Display`，UI 不重新拼语义（FR-1.7）。
/// `current` 用来避免在机翻通道上给出"改用机翻"这种没有意义的动作。
pub fn failure_of(err: &ProviderError, current: EngineKind) -> Failure {
    let action = match err {
        ProviderError::Unauthorized | ProviderError::NotConfigured => {
            Some(FailureAction::GoSettings)
        }
        ProviderError::Timeout(..) if current == EngineKind::Ai => Some(FailureAction::UseMachine),
        _ => None,
    };
    Failure {
        message: err.to_string(),
        action,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tone {
    Info,
    Warn,
    Error,
}

/// 提示条自动消失的时长
pub const TOAST_VISIBLE: std::time::Duration = std::time::Duration::from_secs(4);

#[derive(Debug, Clone)]
pub struct Toast {
    pub tone: Tone,
    pub text: String,
    pub shown_at: std::time::Instant,
}

impl Toast {
    pub fn expired(&self) -> bool {
        self.shown_at.elapsed() >= TOAST_VISIBLE
    }
}

/// 破坏性操作的二次确认（`03-ui-design.md` §1）
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Confirm {
    ClearHistory,
    ClearKey,
}

impl Confirm {
    pub const fn prompt(self) -> &'static str {
        match self {
            Confirm::ClearHistory => "清空全部历史？此操作不可撤销。",
            Confirm::ClearKey => "清除钥匙串里的 API Key？清除后 AI 通道不可用。",
        }
    }

    pub const fn confirm_label(self) -> &'static str {
        match self {
            Confirm::ClearHistory => "确认清空",
            Confirm::ClearKey => "确认清除",
        }
    }
}

/// 界面上关于 Key 的全部可见信息。**不含明文，也不含任何可还原片段**（FR-4.2）。
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum KeyState {
    #[default]
    Unknown,
    Missing,
    Configured {
        len: usize,
        hash8: String,
    },
    Error(String),
}

impl KeyState {
    pub const fn configured(&self) -> bool {
        matches!(self, KeyState::Configured { .. })
    }

    pub const fn label(&self) -> &'static str {
        match self {
            KeyState::Configured { .. } => "已配置",
            KeyState::Missing => "未配置",
            KeyState::Unknown => "未知",
            KeyState::Error(_) => "读取失败",
        }
    }
}

/// 一次待派发的翻译任务
#[derive(Debug, Clone, PartialEq)]
pub struct Job {
    pub id: RequestId,
    pub engine: EngineKind,
    pub request: TranslationRequest,
}

/// 任务回来的结果。带 `id` 用于丢弃过期结果，带 `engine` 用于错误动作判定。
/// 不派生 `Clone`：`ProviderError` 不是 `Clone`，而结果本来也只该被消费一次。
#[derive(Debug)]
pub struct Outcome {
    pub id: RequestId,
    pub engine: EngineKind,
    pub result: Result<TranslationResult, ProviderError>,
}

/// 提示条最多同时显示这么多条，避免把界面挤没
const MAX_TOASTS: usize = 3;

pub struct AppState {
    pub config: Config,
    pub glossary: Glossary,
    pub tab: Tab,
    /// 当前选中的引擎（工具条 F-04）。与 `config.translation.default_engine`（设置页的默认值）分开：
    /// 随手切换不该改用户的默认配置，但退出时默认值要落盘（FR-1.3）。
    pub engine: EngineKind,
    pub source: String,
    /// `source` 的字符数缓存：只在字节长度变化时重算，避免每帧 O(n)（REQ-002 的 20k 字符要求）
    pub char_count: usize,
    source_bytes: usize,
    pub translation: Option<TranslationResult>,
    pub failure: Option<Failure>,
    pub in_flight: Option<RequestId>,
    pub request_id: RequestId,
    pub toasts: Vec<Toast>,
    /// 启动时的一次性提示（配置损坏降级、旧术语表导入条数）
    pub startup_notice: Option<String>,
    pub key: KeyState,
    /// 从错误动作跳到设置页后，把焦点定位到 Key 输入
    pub focus_key_input: bool,
    /// 上一帧源文编辑器是否持有焦点。快捷键在绘制之前判定，所以用的是上一帧的事实。
    pub focus_in_source: bool,
    pub history_query: String,
    pub history_rows: Vec<HistoryRecord>,
    pub history_error: Option<String>,
    pub history_count: usize,
    pub term_source: String,
    pub term_target: String,
    pub confirm: Option<Confirm>,
    /// Cmd+Q 走过一次：此后关闭请求不再被拦截（FR-5.2）
    pub quitting: bool,
    /// 窗口当前是否处于隐藏/最小化态
    pub hidden: bool,
    /// 有配置改动待落盘
    pub config_dirty: bool,
    /// 设置页里正在编辑的 Key 明文缓冲。只在内存，退出前不清空也不落盘、不进日志。
    pub key_input: String,
}

impl AppState {
    pub fn new(
        config: Config,
        glossary: Glossary,
        key: KeyState,
        startup_notice: Option<String>,
    ) -> Self {
        let engine = config.translation.default_engine;
        Self {
            config,
            glossary,
            tab: Tab::Text,
            engine,
            source: String::new(),
            char_count: 0,
            source_bytes: 0,
            translation: None,
            failure: None,
            in_flight: None,
            request_id: 0,
            toasts: Vec::new(),
            startup_notice,
            key,
            focus_key_input: false,
            focus_in_source: false,
            history_query: String::new(),
            history_rows: Vec::new(),
            history_error: None,
            history_count: 0,
            term_source: String::new(),
            term_target: String::new(),
            confirm: None,
            quitting: false,
            hidden: false,
            config_dirty: false,
            key_input: String::new(),
        }
    }

    pub fn mode(&self) -> UiMode {
        self.config.ui.mode
    }

    /// 每帧调一次：只在源文真的变了时重算字符数
    pub fn sync_char_count(&mut self) {
        if self.source.len() != self.source_bytes {
            self.source_bytes = self.source.len();
            self.char_count = self.source.chars().count();
        }
    }

    pub fn toast(&mut self, tone: Tone, text: impl Into<String>) {
        self.toasts.push(Toast {
            tone,
            text: text.into(),
            shown_at: std::time::Instant::now(),
        });
        if self.toasts.len() > MAX_TOASTS {
            let extra = self.toasts.len() - MAX_TOASTS;
            self.toasts.drain(0..extra);
        }
    }

    pub fn dismiss_toast(&mut self, index: usize) {
        if index < self.toasts.len() {
            self.toasts.remove(index);
        }
    }

    /// 每帧调一次：过期的提示条自动消失。返回是否有变化（决定要不要请求重绘）。
    pub fn tick_toasts(&mut self) -> bool {
        let before = self.toasts.len();
        self.toasts.retain(|toast| !toast.expired());
        self.toasts.len() != before
    }

    /// 提交一次翻译。返回 `None` 表示"不该发请求"（正在飞行中，或源文为空）。
    pub fn submit(&mut self) -> Option<Job> {
        if self.in_flight.is_some() {
            return None;
        }
        self.sync_char_count();
        if self.source.trim().is_empty() {
            // 空态：不发请求，也不留旧结果误导（REQ-002）
            self.translation = None;
            self.failure = None;
            return None;
        }
        self.request_id += 1;
        let id = self.request_id;
        self.in_flight = Some(id);
        self.failure = None;
        Some(Job {
            id,
            engine: self.engine,
            request: TranslationRequest::new(
                self.source.clone(),
                self.config.translation.lang_pair(),
            ),
        })
    }

    /// 应用一个任务结果。返回 `false` 表示这是过期结果，已按 `request_id` 丢弃（REQ-002）。
    pub fn apply_outcome(&mut self, outcome: Outcome) -> bool {
        if self.in_flight != Some(outcome.id) {
            return false;
        }
        self.in_flight = None;
        match outcome.result {
            Ok(result) => {
                self.failure = None;
                self.translation = Some(result);
            }
            Err(err) => {
                self.failure = Some(failure_of(&err, outcome.engine));
                // 译文区改为显示原因；源文一个字符都不动（FR-1.7）
                self.translation = None;
            }
        }
        true
    }

    /// 切换引擎：对当前源文自动重译（FR-1.3）
    pub fn set_engine(&mut self, kind: EngineKind) -> Option<Job> {
        self.engine = kind;
        self.submit()
    }

    /// 切换目标语言：记住最近使用 + 自动重译（FR-1.4、FR-1.3）
    pub fn set_target(&mut self, lang: Language) -> Option<Job> {
        self.config.translation.target = lang;
        self.config.translation.remember_target(lang);
        self.config_dirty = true;
        self.submit()
    }

    pub fn set_source_language(&mut self, lang: Option<Language>) {
        self.config.translation.source = lang;
        self.config_dirty = true;
    }

    /// 全量↔简洁切换：只改形态，内容一律保留、不重发请求（FR-3.3）
    pub fn toggle_mode(&mut self) -> UiMode {
        self.config.ui.mode = match self.config.ui.mode {
            UiMode::Full => UiMode::Compact,
            UiMode::Compact => UiMode::Full,
        };
        self.config_dirty = true;
        self.config.ui.mode
    }

    pub fn toggle_topmost(&mut self) -> bool {
        self.config.ui.always_on_top = !self.config.ui.always_on_top;
        self.config_dirty = true;
        self.config.ui.always_on_top
    }

    pub fn mark_config_dirty(&mut self) {
        self.config_dirty = true;
    }

    /// 错误动作：跳到设置页并把焦点定位到 Key 输入
    pub fn go_to_key_settings(&mut self) {
        self.tab = Tab::Settings;
        self.focus_key_input = true;
    }

    /// 错误动作：改用机翻并立即重译（FR-1.7）
    pub fn switch_to_machine(&mut self) -> Option<Job> {
        self.set_engine(EngineKind::Machine)
    }

    /// Esc：清空输入行以及它对应的结果（REQ-005）。
    ///
    /// 不发请求、不退出、不收窗。在飞的任务一并作废：`in_flight` 置空之后，
    /// 那次结果回来时会被 `apply_outcome` 当成过期结果丢掉——否则用户清空了输入行，
    /// 半秒后译文又自己冒出来。
    pub fn clear_input(&mut self) {
        self.source.clear();
        self.translation = None;
        self.failure = None;
        self.in_flight = None;
        self.sync_char_count();
    }

    pub fn visible_languages(&self) -> Vec<Language> {
        self.config.translation.visible_languages()
    }

    // ---- 历史（FR-4.4） ----

    pub fn reload_history(&mut self, store: &HistoryStore) {
        let query = self.history_query.trim();
        let loaded = if query.is_empty() {
            store.recent(HISTORY_CAPACITY)
        } else {
            store.search(query, HISTORY_CAPACITY)
        };
        match loaded {
            Ok(rows) => {
                self.history_count = rows.len();
                self.history_rows = rows;
                self.history_error = None;
            }
            Err(err) => {
                self.history_rows.clear();
                self.history_error = Some(format!("历史不可用：{err}"));
            }
        }
    }

    /// 翻译成功后追加一条历史（追加写，不重写整文件）
    pub fn record_history(&mut self, store: &HistoryStore, result: &TranslationResult) {
        let record = HistoryRecord {
            ts: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_secs() as i64)
                .unwrap_or_default(),
            engine: result.engine,
            detected: result.detected,
            target: self.config.translation.target,
            source_text: self.source.clone(),
            result_text: result.text.clone(),
            elapsed_ms: result.elapsed_ms,
        };
        if let Err(err) = store.append(&record) {
            self.history_error = Some(format!("历史写入失败：{err}"));
            return;
        }
        self.reload_history(store);
    }

    pub fn clear_history(&mut self, store: &HistoryStore) {
        match store.clear() {
            Ok(()) => {
                self.history_rows.clear();
                self.history_count = 0;
                self.history_error = None;
                self.toast(Tone::Info, "历史已清空");
            }
            Err(err) => self.history_error = Some(format!("清空历史失败：{err}")),
        }
    }

    /// 点一条历史：只回填内容，不自动发请求（REQ-006）
    pub fn load_history_row(&mut self, index: usize) -> bool {
        // 先克隆出来：下面要改 `self.source`，不能同时借着 `self.history_rows`
        let Some(row) = self.history_rows.get(index).cloned() else {
            return false;
        };
        self.source = row.source_text.clone();
        self.source_bytes = 0;
        self.sync_char_count();
        self.translation = Some(TranslationResult {
            text: row.result_text.clone(),
            engine: row.engine,
            detected: row.detected,
            elapsed_ms: row.elapsed_ms,
            cache_hit: false,
            segments: 1,
        });
        self.failure = None;
        self.tab = Tab::Text;
        true
    }

    // ---- 术语（FR-4.5） ----

    pub fn add_term(&mut self) -> Option<Term> {
        let source = self.term_source.trim();
        let target = self.term_target.trim();
        if source.is_empty() || target.is_empty() {
            return None;
        }
        let term = Term::new(source, target);
        self.glossary.upsert(term.clone());
        self.glossary.write_to_config(&mut self.config);
        self.term_source.clear();
        self.term_target.clear();
        self.config_dirty = true;
        Some(term)
    }

    pub fn remove_term(&mut self, source: &str) -> bool {
        let removed = self.glossary.remove(source);
        if removed {
            self.glossary.write_to_config(&mut self.config);
            self.config_dirty = true;
        }
        removed
    }

    /// 状态条（F-11）：就绪 / 错误 / 未配置三态
    pub fn status_text(&self, ai_available: bool) -> String {
        let engine_state = if ai_available {
            "机翻可用 · AI 可用"
        } else {
            "机翻可用 · AI 未配置"
        };
        let head = if let Some(failure) = &self.failure {
            format!("错误：{}", failure.message)
        } else if self.in_flight.is_some() {
            "翻译中…".to_string()
        } else {
            "就绪".to_string()
        };
        format!(
            "{head} · {engine_state} · 历史 {} 条 · Cmd+Enter 翻译",
            self.history_count
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ok(text: &str, engine: EngineKind) -> TranslationResult {
        TranslationResult {
            text: text.into(),
            engine,
            detected: Some(Language::En),
            elapsed_ms: 320,
            cache_hit: false,
            segments: 1,
        }
    }

    fn app_with_source(text: &str) -> AppState {
        let mut state = AppState::new(
            Config::default(),
            Glossary::default(),
            KeyState::Missing,
            None,
        );
        state.source = text.into();
        state
    }

    #[test]
    fn empty_source_does_not_submit_and_clears_stale_result() {
        let mut state = app_with_source("  \n ");
        state.translation = Some(ok("旧译文", EngineKind::Machine));
        assert!(state.submit().is_none());
        assert!(state.translation.is_none());
        assert_eq!(state.request_id, 0);
    }

    #[test]
    fn submit_while_in_flight_is_ignored() {
        let mut state = app_with_source("hello");
        let first = state.submit().expect("首次提交");
        assert_eq!(first.id, 1);
        assert!(state.submit().is_none(), "请求进行中不得叠加第二个请求");
        assert_eq!(state.request_id, 1);
    }

    #[test]
    fn stale_outcome_is_dropped_without_touching_visible_text() {
        let mut state = app_with_source("hello");
        let first = state.submit().unwrap();
        // 用户改了源文并再次提交（第一次的结果此时才回来）
        state.in_flight = None;
        state.source = "world".into();
        let second = state.submit().unwrap();
        assert_ne!(first.id, second.id);

        let stale = Outcome {
            id: first.id,
            engine: EngineKind::Machine,
            result: Ok(ok("旧结果", EngineKind::Machine)),
        };
        assert!(!state.apply_outcome(stale));
        assert!(state.translation.is_none(), "过期结果不得回写");
        assert_eq!(state.in_flight, Some(second.id), "新请求仍在飞行中");

        let fresh = Outcome {
            id: second.id,
            engine: EngineKind::Machine,
            result: Ok(ok("新结果", EngineKind::Machine)),
        };
        assert!(state.apply_outcome(fresh));
        assert_eq!(state.translation.unwrap().text, "新结果");
        assert_eq!(state.in_flight, None);
    }

    #[test]
    fn failure_keeps_source_and_maps_action() {
        let mut state = app_with_source("hello");
        let job = state.submit().unwrap();
        state.apply_outcome(Outcome {
            id: job.id,
            engine: EngineKind::Ai,
            result: Err(ProviderError::Unauthorized),
        });
        assert_eq!(state.source, "hello", "失败不得丢源文");
        let failure = state.failure.as_ref().expect("要有可读错误");
        assert_eq!(failure.message, ProviderError::Unauthorized.to_string());
        assert_eq!(failure.action, Some(FailureAction::GoSettings));
        assert!(state.translation.is_none());

        state.go_to_key_settings();
        assert_eq!(state.tab, Tab::Settings);
        assert!(state.focus_key_input);
    }

    #[test]
    fn timeout_on_ai_offers_machine_fallback_but_not_on_machine() {
        let err = ProviderError::Timeout(15_000, 2);
        let ai = failure_of(&err, EngineKind::Ai);
        assert_eq!(ai.action, Some(FailureAction::UseMachine));
        assert!(ai.message.contains("已重试 2 次"), "{}", ai.message);
        assert_eq!(failure_of(&err, EngineKind::Machine).action, None);
    }

    #[test]
    fn rate_limit_keeps_retry_possible_and_has_no_action() {
        let failure = failure_of(&ProviderError::RateLimited, EngineKind::Ai);
        assert_eq!(failure.action, None);
        assert_eq!(failure.message, ProviderError::RateLimited.to_string());
    }

    #[test]
    fn switching_engine_or_target_resubmits_current_source() {
        let mut state = app_with_source("hello");
        let id = state.submit().unwrap().id;
        state.apply_outcome(Outcome {
            id,
            engine: EngineKind::Machine,
            result: Ok(ok("你好", EngineKind::Machine)),
        });

        let job = state.set_engine(EngineKind::Ai).expect("切引擎要重译");
        assert_eq!(job.engine, EngineKind::Ai);
        assert_eq!(job.request.text, "hello");

        state.in_flight = None;
        let job = state.set_target(Language::Ja).expect("改目标语言要重译");
        assert_eq!(job.request.pair.target, Language::Ja);
        assert_eq!(state.config.translation.recent_targets[0], Language::Ja);
        assert!(state.config_dirty);
    }

    #[test]
    fn mode_toggle_preserves_content_and_does_not_request() {
        let mut state = app_with_source("hello");
        let id = state.submit().unwrap().id;
        state.apply_outcome(Outcome {
            id,
            engine: EngineKind::Machine,
            result: Ok(ok("你好", EngineKind::Machine)),
        });
        let id_before = state.request_id;

        for _ in 0..20 {
            state.toggle_mode();
        }

        assert_eq!(state.mode(), UiMode::Full);
        assert_eq!(state.source, "hello");
        assert_eq!(state.translation.unwrap().text, "你好");
        assert_eq!(state.request_id, id_before, "切换模式不得重复发请求");
    }

    #[test]
    fn char_count_is_cached_until_source_changes() {
        let mut state = app_with_source("你好世界");
        state.sync_char_count();
        assert_eq!(state.char_count, 4);
        state.sync_char_count();
        assert_eq!(state.char_count, 4);
        state.source.push_str("abc");
        state.sync_char_count();
        assert_eq!(state.char_count, 7);
    }

    #[test]
    fn term_edits_flow_into_config_and_mark_dirty() {
        let mut state = AppState::new(
            Config::default(),
            Glossary::default(),
            KeyState::Missing,
            None,
        );
        state.term_source = "pod".into();
        state.term_target = "Pod".into();
        let term = state.add_term().expect("术语要能新增");
        assert_eq!(term.source, "pod");
        assert_eq!(state.config.terms.len(), 1);
        assert!(state.config_dirty);
        assert!(state.term_source.is_empty(), "输入缓冲要清空");

        state.term_target = String::new();
        assert!(state.add_term().is_none(), "译名为空不新增");

        assert!(state.remove_term("POD"));
        assert!(state.config.terms.is_empty());
    }

    #[test]
    fn history_row_loads_without_submitting() {
        let dir = tempfile::tempdir().unwrap();
        let store = HistoryStore::at(dir.path().join("history.jsonl"));
        let mut state = AppState::new(
            Config::default(),
            Glossary::default(),
            KeyState::Missing,
            None,
        );
        state.source = "hello".into();
        let result = ok("你好", EngineKind::Machine);
        state.submit();
        state.record_history(&store, &result);
        assert_eq!(state.history_count, 1);

        state.source = String::new();
        state.translation = None;
        assert!(state.load_history_row(0));
        assert_eq!(state.source, "hello");
        assert_eq!(state.translation.unwrap().text, "你好");
        assert_eq!(state.tab, Tab::Text);
        assert_eq!(state.request_id, 1, "回显历史不得自动发请求");
    }

    #[test]
    fn corrupt_history_shows_reason_and_keeps_app_usable() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("history.jsonl");
        std::fs::write(&path, "{ 这不是 jsonl\n").unwrap();
        let store = HistoryStore::at(&path);
        let mut state = AppState::new(
            Config::default(),
            Glossary::default(),
            KeyState::Missing,
            None,
        );
        state.reload_history(&store);
        // 损坏行被跳过而不是致命错误（core 的既有行为），因此这里不该有 error
        assert!(state.history_error.is_none(), "{:?}", state.history_error);
        state.source = "hello".into();
        assert!(state.submit().is_some());
    }

    #[test]
    fn status_text_reflects_state_and_ai_availability() {
        let mut state = AppState::new(
            Config::default(),
            Glossary::default(),
            KeyState::Missing,
            None,
        );
        assert!(state.status_text(false).starts_with("就绪"));
        assert!(state.status_text(false).contains("AI 未配置"));
        assert!(state.status_text(true).contains("AI 可用"));

        state.failure = Some(failure_of(&ProviderError::RateLimited, EngineKind::Ai));
        assert!(state.status_text(true).starts_with("错误：被限流"));
    }
}
