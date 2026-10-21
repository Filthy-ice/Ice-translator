//! 翻译任务的派发与回收（架构 §4 的并发模型：UI 线程绝不 `await`）。
//!
//! `Translator` 的缓存与术语表都在实例内部，`set_glossary` 又要 `&mut self`，
//! 所以共享句柄是 `Arc<tokio::sync::Mutex<Translator>>`：任务在 `await` 期间持锁，
//! 界面上的术语改动用 `try_lock` 立即生效，锁被占住时挂起、下一次派发前先补上。

use std::sync::Arc;

use egui::Context;
use ice_core::glossary::Glossary;
use ice_core::history::HistoryStore;
use ice_providers::dispatcher::Translator;
use ice_providers::error::ProviderError;
use tokio::runtime::Runtime;
use tokio::sync::{Mutex, mpsc};

use crate::state::{AppState, Job, Outcome};

/// 通道容量：界面上同一时刻只允许一个请求在飞，留一点余量给切换语言时的重译
const CHANNEL_CAPACITY: usize = 8;

pub type OutcomeReceiver = mpsc::Receiver<Outcome>;

pub struct JobRunner {
    shared: Arc<Mutex<Translator>>,
    tx: mpsc::Sender<Outcome>,
    pending_glossary: Option<Glossary>,
    pending_translator: Option<Translator>,
}

impl JobRunner {
    pub fn new(translator: Translator) -> (Self, OutcomeReceiver) {
        let (tx, rx) = mpsc::channel(CHANNEL_CAPACITY);
        (
            Self {
                shared: Arc::new(Mutex::new(translator)),
                tx,
                pending_glossary: None,
                pending_translator: None,
            },
            rx,
        )
    }

    /// 术语改动即时生效（REQ-006）：拿得到锁就马上写进去，拿不到就挂起到下一次派发。
    pub fn set_glossary(&mut self, glossary: Glossary) {
        match self.shared.try_lock() {
            Ok(mut translator) => translator.set_glossary(glossary),
            Err(_) => self.pending_glossary = Some(glossary),
        }
    }

    /// AI 配置或 Key 变化后换掉整套引擎。
    ///
    /// 重建会丢掉翻译缓存——这是有意的取舍：能触发重建的只有"用户改了供应商/模型/Key"，
    /// 此时旧缓存本来就不再对应同一套参数。术语表改动走 `set_glossary`，不动缓存。
    pub fn replace_translator(&mut self, translator: Translator) {
        match self.shared.try_lock() {
            Ok(mut guard) => *guard = translator,
            Err(_) => self.pending_translator = Some(translator),
        }
    }

    /// 派发一个任务。`ctx` 只用来在结果到达时请求重绘。
    pub fn submit(&mut self, runtime: &Runtime, job: Job, ctx: &Context) {
        let shared = Arc::clone(&self.shared);
        let tx = self.tx.clone();
        let ctx = ctx.clone();
        let glossary = self.pending_glossary.take();
        let replacement = self.pending_translator.take();

        runtime.spawn(async move {
            let mut guard = shared.lock().await;
            if let Some(translator) = replacement {
                *guard = translator;
            }
            if let Some(glossary) = glossary {
                guard.set_glossary(glossary);
            }
            let result = guard.translate(job.engine, job.request.clone()).await;
            // 先放锁再发送：界面收到结果后可能立刻改术语，不该被自己堵住
            drop(guard);

            // `try_send` 而不是 `send().await`：通道满了就意味着界面已经不再回收结果
            //（退出中），此时 await 会永远等不到重绘。界面上同一时刻只允许一个请求在飞，
            // 容量 8 在正常运行中不可能被填满。
            let sent = tx
                .try_send(Outcome {
                    id: job.id,
                    engine: job.engine,
                    result,
                })
                .is_ok();
            if sent {
                ctx.request_repaint();
            }
        });
    }
}

/// 把一条结果落到界面上。返回 `false` 表示这是过期结果，已按 `request_id` 丢弃（REQ-002）。
pub fn apply(state: &mut AppState, history: &HistoryStore, outcome: Outcome) -> bool {
    let id = outcome.id;
    // 日志只记变体名、状态码与长度：错误文本里可能带端点信息，源文与译文一律不进日志（NFR-3）
    match &outcome.result {
        Ok(result) => tracing::info!(
            request_id = id,
            engine = ?result.engine,
            chars = result.text.chars().count(),
            elapsed_ms = result.elapsed_ms,
            cache_hit = result.cache_hit,
            segments = result.segments,
            "翻译完成"
        ),
        Err(err) => tracing::warn!(
            request_id = id,
            error = variant_name(err),
            status = status_code(err).unwrap_or(0),
            "翻译失败"
        ),
    }

    let accepted = state.apply_outcome(outcome);
    if !accepted {
        return false;
    }
    // 缓存命中不重复写历史：那条记录在第一次翻译时已经写过了
    let fresh = state
        .translation
        .as_ref()
        .filter(|result| !result.cache_hit)
        .cloned();
    if let Some(result) = fresh {
        state.record_history(history, &result);
    }
    true
}

/// 帧首回收所有已到达的结果，返回本次应用的条数。
pub fn drain(rx: &mut OutcomeReceiver, state: &mut AppState, history: &HistoryStore) -> usize {
    let mut applied = 0;
    while let Ok(outcome) = rx.try_recv() {
        if apply(state, history, outcome) {
            applied += 1;
        }
    }
    applied
}

/// 只记变体名，不记内容
/// HTTP 状态码，非 HTTP 类错误返回 `None`。
///
/// 只记变体名的话，线上一条 `error="Status"` 无从下手：状态码不是秘密
/// （响应体已在 `from_status` 里截断，查询串已在 `from_reqwest` 里洗掉）。
fn status_code(err: &ProviderError) -> Option<u16> {
    match err {
        ProviderError::Status(code, _) => Some(*code),
        ProviderError::Unauthorized => Some(401),
        ProviderError::RateLimited => Some(429),
        _ => None,
    }
}

pub fn variant_name(err: &ProviderError) -> &'static str {
    match err {
        ProviderError::NotImplemented(_) => "NotImplemented",
        ProviderError::Http(_) => "Http",
        ProviderError::Timeout(..) => "Timeout",
        ProviderError::Status(..) => "Status",
        ProviderError::Unauthorized => "Unauthorized",
        ProviderError::RateLimited => "RateLimited",
        ProviderError::NotConfigured => "NotConfigured",
        ProviderError::Decode(_) => "Decode",
        ProviderError::Empty => "Empty",
        ProviderError::EmptyInput => "EmptyInput",
        ProviderError::SameLanguage(_) => "SameLanguage",
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use ice_core::config::Config;
    use ice_core::glossary::Term;
    use ice_core::language::EngineKind;

    use super::*;
    use crate::state::KeyState;
    use crate::stub::{StubBehavior, StubEngine, StubError};

    struct Harness {
        runtime: Runtime,
        runner: JobRunner,
        rx: OutcomeReceiver,
        state: AppState,
        history: HistoryStore,
        _dir: tempfile::TempDir,
    }

    impl Harness {
        fn new(engines: Vec<Arc<dyn ice_providers::engine::TranslationEngine>>) -> Self {
            let mut translator = Translator::new();
            for engine in engines {
                translator.register(engine);
            }
            let (runner, rx) = JobRunner::new(translator);
            let dir = tempfile::tempdir().unwrap();
            let mut state = AppState::new(
                Config::default(),
                Glossary::default(),
                KeyState::Missing,
                None,
            );
            state.source = "hello".to_string();
            Self {
                runtime: tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                    .unwrap(),
                runner,
                rx,
                state,
                history: HistoryStore::at(dir.path().join("history.jsonl")),
                _dir: dir,
            }
        }

        /// 提交当前源文并等结果真的回来
        fn run_once(&mut self, ctx: &Context) {
            let job = self.state.submit().expect("should submit");
            self.runner.submit(&self.runtime, job, ctx);
            let outcome = self
                .runtime
                .block_on(async { self.rx.recv().await.expect("outcome") });
            // recv 把结果取走了，这里直接 apply，等价于 drain 的单条路径
            apply(&mut self.state, &self.history, outcome);
        }
    }

    #[test]
    fn submitted_job_lands_in_state_and_history() {
        let stub = StubEngine::replying(EngineKind::Machine, "你好");
        let mut harness = Harness::new(vec![Arc::new(stub.clone())]);
        let ctx = Context::default();

        harness.run_once(&ctx);

        let translation = harness.state.translation.expect("translation");
        assert_eq!(translation.text, "你好");
        assert!(!translation.cache_hit);
        assert_eq!(harness.state.history_count, 1);
        assert_eq!(stub.calls(), 1);
    }

    #[test]
    fn stale_outcome_is_dropped_without_touching_the_ui() {
        let stub = StubEngine::replying(EngineKind::Machine, "你好");
        let mut harness = Harness::new(vec![Arc::new(stub.clone())]);

        let stale = Outcome {
            id: harness.state.request_id + 99,
            engine: EngineKind::Machine,
            result: Ok(ice_providers::request::TranslationResult::cached(
                "旧的",
                EngineKind::Machine,
            )),
        };
        assert!(!apply(&mut harness.state, &harness.history, stale));
        assert!(harness.state.translation.is_none());
        assert_eq!(harness.state.history_count, 0);
    }

    #[test]
    fn cache_hit_does_not_append_a_second_history_row() {
        let stub = StubEngine::replying(EngineKind::Machine, "你好");
        let mut harness = Harness::new(vec![Arc::new(stub.clone())]);
        let ctx = Context::default();

        harness.run_once(&ctx);
        // 同一份源文再翻一次：命中调度层缓存，不打网络也不重复写历史
        harness.state.translation = None;
        harness.run_once(&ctx);

        assert_eq!(stub.calls(), 1, "第二次应该走缓存");
        assert!(harness.state.translation.expect("translation").cache_hit);
        assert_eq!(harness.state.history_count, 1);
    }

    #[test]
    fn failure_is_mapped_and_source_survives() {
        let stub = StubEngine::failing(EngineKind::Ai, StubError::Unauthorized);
        let mut harness = Harness::new(vec![Arc::new(stub.clone())]);
        harness.state.engine = EngineKind::Ai;
        let ctx = Context::default();

        harness.run_once(&ctx);

        let failure = harness.state.failure.expect("failure");
        assert_eq!(failure.message, ProviderError::Unauthorized.to_string());
        assert_eq!(harness.state.source, "hello", "源文必须完好");
        assert_eq!(harness.state.history_count, 0);
    }

    #[test]
    fn every_error_variant_renders_a_message_and_never_panics() {
        for error in StubError::all() {
            let stub = StubEngine::failing(EngineKind::Machine, error);
            let mut harness = Harness::new(vec![Arc::new(stub)]);
            let ctx = Context::default();
            harness.run_once(&ctx);
            let failure = harness
                .state
                .failure
                .unwrap_or_else(|| panic!("{error:?} 没有失败态"));
            assert!(!failure.message.is_empty());
            assert_eq!(variant_name(&error.build()), variant_label(error));
        }
    }

    /// 光有变体名不够：`error="Status"` 到底是 400 还是 503，决定了下一步是改请求还是等一会
    #[test]
    fn status_code_is_logged_for_http_failures_only() {
        assert_eq!(status_code(&StubError::Status.build()), Some(503));
        assert_eq!(status_code(&StubError::Unauthorized.build()), Some(401));
        assert_eq!(status_code(&StubError::RateLimited.build()), Some(429));
        for error in [
            StubError::Timeout,
            StubError::Http,
            StubError::Decode,
            StubError::Empty,
            StubError::EmptyInput,
            StubError::NotConfigured,
            StubError::NotImplemented,
            StubError::SameLanguage,
        ] {
            assert_eq!(
                status_code(&error.build()),
                None,
                "{error:?} 不是 HTTP 状态"
            );
        }
    }

    /// 日志里的变体名必须与注入的变体一一对应，否则排障会被误导
    fn variant_label(error: StubError) -> &'static str {
        match error {
            StubError::NotImplemented => "NotImplemented",
            StubError::Http => "Http",
            StubError::Timeout => "Timeout",
            StubError::Status => "Status",
            StubError::Unauthorized => "Unauthorized",
            StubError::RateLimited => "RateLimited",
            StubError::NotConfigured => "NotConfigured",
            StubError::Decode => "Decode",
            StubError::Empty => "Empty",
            StubError::EmptyInput => "EmptyInput",
            StubError::SameLanguage => "SameLanguage",
        }
    }

    #[test]
    fn glossary_change_reaches_the_engine_prompt() {
        let stub = StubEngine::replying(EngineKind::Ai, "你好");
        let mut harness = Harness::new(vec![Arc::new(stub.clone())]);
        harness.state.engine = EngineKind::Ai;
        let ctx = Context::default();

        let mut glossary = Glossary::default();
        glossary.upsert(Term::new("hello", "你好"));
        harness.runner.set_glossary(glossary);
        harness.run_once(&ctx);

        let hint = stub
            .last_request()
            .and_then(|req| req.glossary_hint)
            .expect("AI 请求应带术语提示词");
        assert!(hint.contains("hello => 你好"), "{hint}");
    }

    #[test]
    fn replacing_the_translator_takes_effect_on_the_next_job() {
        let first = StubEngine::replying(EngineKind::Machine, "旧引擎");
        let mut harness = Harness::new(vec![Arc::new(first.clone())]);
        let ctx = Context::default();

        let mut translator = Translator::new();
        let second = StubEngine::new(EngineKind::Machine, StubBehavior::Ok("新引擎".to_string()));
        translator.register(Arc::new(second.clone()));
        harness.runner.replace_translator(translator);
        harness.run_once(&ctx);

        assert_eq!(
            harness.state.translation.expect("translation").text,
            "新引擎"
        );
        assert_eq!(first.calls(), 0);
        assert_eq!(second.calls(), 1);
    }

    #[test]
    fn a_slow_job_does_not_block_the_ui_thread() {
        let stub =
            StubEngine::replying(EngineKind::Machine, "你好").with_delay(Duration::from_millis(80));
        let mut harness = Harness::new(vec![Arc::new(stub.clone())]);
        let ctx = Context::default();

        let job = harness.state.submit().expect("should submit");
        harness.runner.submit(&harness.runtime, job, &ctx);
        // 派发立刻返回：界面线程没有被 await 卡住
        assert!(harness.rx.try_recv().is_err());
        assert_eq!(harness.state.in_flight, Some(1));

        let outcome = harness
            .runtime
            .block_on(async { harness.rx.recv().await.unwrap() });
        assert!(apply(&mut harness.state, &harness.history, outcome));
        assert_eq!(stub.calls(), 1);
    }
}
