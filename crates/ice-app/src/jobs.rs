//! 翻译与识别任务的派发与回收（架构 §4 的并发模型：UI 线程绝不 `await`）。
//!
//! `Translator` 的缓存与术语表都在实例内部，`set_glossary` 又要 `&mut self`，
//! 所以共享句柄是 `Arc<tokio::sync::Mutex<Translator>>`：任务在 `await` 期间持锁，
//! 界面上的术语改动用 `try_lock` 立即生效，锁被占住时挂起、下一次派发前先补上。
//!
//! 图片识别走另一条线程池入口：`runtime.spawn_blocking`。整段（读盘解码 → 缩放 → Vision
//! 调用 → 行合并）都待在同一条工作线程里，`Retained` 出不了 `ice-vision` 的 `vision_sys`
//! （ADR-002 的硬边界，docs/05 那条"objc 类 `!Send` ⇒ 一次调用整段待在同一线程"）。
//!
//! P7 补上图片链的另两段，各按自己的性质选入口：逐块翻译还是 async、还是 `await` 网络，但
//! **一批 N 块串行**（I-04"行间不并发"）；合成与识别同族，是一次占住一条工作线程的同步计算。

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Instant;

use egui::Context;
use ice_core::document::OcrDocument;
use ice_core::glossary::Glossary;
use ice_core::history::HistoryStore;
use ice_core::language::EngineKind;
use ice_providers::dispatcher::Translator;
use ice_providers::error::ProviderError;
use ice_providers::request::TranslationRequest;
use ice_vision::composite::Renderer;
use ice_vision::error::VisionError;
use ice_vision::import::{Admitted, admit_path, admit_rgba};
use ice_vision::typeset::CompositeMode;
use image::RgbaImage;
use tokio::runtime::Runtime;
use tokio::sync::{Mutex, mpsc};

use crate::state::{AppState, ImageInput, ImageRecognition, Job, Outcome, RequestId};

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
    ///
    /// 四条分支用同一个 `tx`、同一个"结果到达就重绘一次"的收尾，但线程入口不同：
    /// 翻译与逐块翻译是 async（要 `await` 网络），识别与合成是 blocking（识别整段占住一条
    /// 工作线程且 objc 对象一个都不许离开那段，ADR-002；合成是一次同步的 CPU 活）。
    pub fn submit(&mut self, runtime: &Runtime, job: Job, ctx: &Context) {
        match job {
            Job::Translation {
                id,
                engine,
                request,
            } => self.submit_translation(runtime, id, engine, request, ctx),
            Job::ImageOcr { id, input } => self.submit_image(runtime, id, input, ctx),
            Job::ImageTranslate {
                id,
                engine,
                blocks,
                cancel,
            } => self.submit_image_translate(runtime, id, engine, blocks, cancel, ctx),
            Job::ImageCompose {
                id,
                source,
                document,
                mode,
            } => self.submit_image_compose(runtime, id, source, document, mode, ctx),
        }
    }

    fn submit_translation(
        &mut self,
        runtime: &Runtime,
        id: RequestId,
        engine: EngineKind,
        request: TranslationRequest,
        ctx: &Context,
    ) {
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
            let result = guard.translate(engine, request.clone()).await;
            // 先放锁再发送：界面收到结果后可能立刻改术语，不该被自己堵住
            drop(guard);

            // `try_send` 而不是 `send().await`：通道满了就意味着界面已经不再回收结果
            //（退出中），此时 await 会永远等不到重绘。界面上同一时刻只允许一个请求在飞，
            // 容量 8 在正常运行中不可能被填满。
            let sent = tx
                .try_send(Outcome::Translation { id, engine, result })
                .is_ok();
            if sent {
                ctx.request_repaint();
            }
        });
    }

    /// 识别段：入场（读盘/解码/缩放）+ S1 Vision + S2/S3 行合并，整段一条 `spawn_blocking`。
    ///
    /// 输入为 `ImageInput::Clipboard` 时像素已由主线程的 arboard 取出，这里只做长度校验后
    /// 拼成 `RgbaImage`；`Path` 则连读盘都在这条线程上（几百 MB 的截图也不能卡界面）。
    fn submit_image(&mut self, runtime: &Runtime, id: RequestId, input: ImageInput, ctx: &Context) {
        let tx = self.tx.clone();
        let ctx = ctx.clone();

        runtime.spawn_blocking(move || {
            let result = admit_input(input).and_then(|admitted| {
                let (rows, document, progress) = ice_vision::recognize_with_rows(&admitted.image)?;
                Ok(ImageRecognition {
                    admitted,
                    rows,
                    document,
                    elapsed_ms: progress.elapsed_ms,
                })
            });
            let sent = tx.try_send(Outcome::Image { id, result }).is_ok();
            if sent {
                ctx.request_repaint();
            }
        });
    }

    /// 逐块翻译段：N 块 = N 次请求，**串行**，一块落地才发下一块（docs/03 I-04 原文"行间不并发"，
    /// Q-002 裁定沿用；机翻端点的免费额度按请求数算）。
    ///
    /// 三条不显然的：
    /// 1. **整批持同一把锁**。中途换引擎或改术语会让同一批的前半与后半走两套参数，那时候
    ///    "H 区这 N 行出自同一个引擎、同一份术语表"就不再成立。挂起的改动留到下一次派发补
    ///    （与单条翻译同一条纪律），代价是这段时间界面上的术语改动不落进正在跑的这批。
    /// 2. **一块的 `Err` 只记在它自己头上**：不回滚前面已落地的 `Ok`，也不静默改走另一个引擎
    ///    （D-20 那条：换通道是业务决定，交回用户）。
    /// 3. **停止只在块边界生效**。在飞的那一次请求撤不回，所以点停止最少还要等一块的时间——
    ///    界面因此不许说"已撤回"，只许说"已停止这次处理，译文还在"。
    fn submit_image_translate(
        &mut self,
        runtime: &Runtime,
        id: RequestId,
        engine: EngineKind,
        blocks: Vec<(u32, TranslationRequest)>,
        cancel: Arc<AtomicBool>,
        ctx: &Context,
    ) {
        let shared = Arc::clone(&self.shared);
        let tx = self.tx.clone();
        let ctx = ctx.clone();
        let glossary = self.pending_glossary.take();
        let replacement = self.pending_translator.take();
        let total = blocks.len();

        runtime.spawn(async move {
            let started = Instant::now();
            let mut guard = shared.lock().await;
            if let Some(translator) = replacement {
                *guard = translator;
            }
            if let Some(glossary) = glossary {
                guard.set_glossary(glossary);
            }

            let mut results = Vec::with_capacity(total);
            let mut cancelled = false;
            for (block_id, request) in blocks {
                if cancel.load(Ordering::Relaxed) {
                    cancelled = true;
                    break;
                }
                let result = guard.translate(engine, request).await;
                results.push((block_id, result.map(|translated| translated.text)));
                // 每块报一次进度：20 个块要等十几秒，"翻译中 7/20"是这段唯一能自证还在走的东西。
                // 发不出去就跳过这一次——通道满意味着界面已经不回收结果了，要紧的是最后那条。
                if tx
                    .try_send(Outcome::ImageTranslateProgress {
                        id,
                        done: results.len(),
                        total,
                    })
                    .is_ok()
                {
                    ctx.request_repaint();
                }
            }
            // 先放锁再发送：界面收到结果后可能立刻改术语，不该被自己堵住
            drop(guard);

            let elapsed_ms = started.elapsed().as_millis() as u64;
            let sent = tx
                .try_send(Outcome::ImageTranslated {
                    id,
                    results,
                    cancelled,
                    elapsed_ms,
                })
                .is_ok();
            if sent {
                ctx.request_repaint();
            }
        });
    }

    /// 合成段：一次同步计算，整段待在同一条 `spawn_blocking` 线程上（字形光栅化是 CPU 活）。
    ///
    ///  Renderer 用的选项一律是默认值：那四个数（下限比例、留白、采样比例、是否允许改判）是
    /// 判据的一部分，不是用户偏好，`config.toml` 里没有它们（docs/04 §3）。
    /// 成品只有一个结局：整张图画完（`Ok`）或者什么都没有（`Err`）。中间态不往外发，
    /// 所以界面上永远不会出现画了一半的那张图（REQ-009 第四条）。
    fn submit_image_compose(
        &mut self,
        runtime: &Runtime,
        id: RequestId,
        source: RgbaImage,
        document: OcrDocument,
        mode: CompositeMode,
        ctx: &Context,
    ) {
        let tx = self.tx.clone();
        let ctx = ctx.clone();

        runtime.spawn_blocking(move || {
            let started = Instant::now();
            let result = Renderer::default().compose(&source, &document, mode);
            let elapsed_ms = started.elapsed().as_millis() as u64;
            let sent = tx
                .try_send(Outcome::ImageComposed {
                    id,
                    result,
                    elapsed_ms,
                })
                .is_ok();
            if sent {
                ctx.request_repaint();
            }
        });
    }
}

/// 把任务原料变成入场后的图。剪贴板那一支的字节长度已由 `system::clipboard::read_image` 校验，
/// 这里再校验一次不是为了防主线程，而是 `RgbaImage::from_raw` 本身对长度不符就返回 `None`。
fn admit_input(input: ImageInput) -> Result<Admitted, VisionError> {
    match input {
        ImageInput::Path(path) => admit_path(&path),
        ImageInput::Clipboard {
            bytes,
            width,
            height,
        } => {
            let image = RgbaImage::from_raw(width as u32, height as u32, bytes)
                .ok_or_else(|| VisionError::Decode(CLIPBOARD_SIZE_TEXT.to_string()))?;
            Ok(admit_rgba(image, CLIPBOARD_LABEL))
        }
    }
}

/// 剪贴板来源在界面上的名字（`Admitted.label` 只给显示用，不进日志、不进历史）
const CLIPBOARD_LABEL: &str = "剪贴板";

/// 像素数与宽高对不上时给人的那句话——与文件解码失败分开写，否则用户会以为图坏了
const CLIPBOARD_SIZE_TEXT: &str = "剪贴板里的图片尺寸与像素数不匹配，请重新复制";

/// 把一条结果落到界面上。返回 `false` 表示这是过期结果，已按各自那一侧的序号丢弃（REQ-002、REQ-006）。
pub fn apply(state: &mut AppState, history: &HistoryStore, outcome: Outcome) -> bool {
    log_outcome(&outcome);

    let accepted = state.apply_outcome(outcome);
    if !accepted {
        return false;
    }
    // 缓存命中不重复写历史：那条记录在第一次翻译时已经写过了。
    // 图片链的逐块译文也不写历史：历史表那一行的语义是"用户在文本页打下的源文 → 它的译文"，
    // 而图片的块号只跟这一次导入有关系，把 N 行记进去下次既查不回也认不出是哪张图。
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

/// 日志只记变体名、状态码与长度：错误文本里可能带端点信息，源文、译文与识别出的文字一律不进日志（NFR-3）。
fn log_outcome(outcome: &Outcome) {
    match outcome {
        Outcome::Translation { id, result, .. } => match result {
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
        },
        Outcome::Image { id, result } => match result {
            // 图片侧的口径与 `ocr.rs` 那条 debug 一致：只有行数、块数、耗时和尺寸
            Ok(recognition) => tracing::info!(
                request_id = id,
                rows = recognition.rows.len(),
                blocks = recognition.document.blocks.len(),
                elapsed_ms = recognition.elapsed_ms,
                width = recognition.admitted.width,
                height = recognition.admitted.height,
                scaled = recognition.admitted.scaled,
                "本地识别完成"
            ),
            Err(err) => tracing::warn!(
                request_id = id,
                error = vision_variant_name(err),
                "本地识别失败"
            ),
        },
        // 进度不记日志：每块一条会把真正那条结局淹掉，而它能说的只有"还在走"
        Outcome::ImageTranslateProgress { .. } => {}
        Outcome::ImageTranslated {
            id,
            results,
            cancelled,
            elapsed_ms,
        } => {
            // 只有块号、计数、耗时与变体名：识别出的文字与译文一律不进日志（NFR-3）
            tracing::info!(
                request_id = id,
                blocks = results.len(),
                failed = results.iter().filter(|(_, result)| result.is_err()).count(),
                cancelled,
                elapsed_ms,
                "逐块翻译结束"
            );
        }
        Outcome::ImageComposed {
            id,
            result,
            elapsed_ms,
        } => match result {
            Ok(composed) => tracing::info!(
                request_id = id,
                replaced = composed.replaced,
                overlaid = composed.overlaid,
                skipped = composed.skipped,
                degraded = composed.degraded(),
                missing_glyphs = composed
                    .missing_glyphs
                    .iter()
                    .map(|(_, count)| count)
                    .sum::<usize>(),
                elapsed_ms,
                "本地合成完成"
            ),
            Err(err) => tracing::warn!(
                request_id = id,
                error = vision_variant_name(err),
                "本地合成失败"
            ),
        },
    }
}

/// 图片侧的变体名：错误文本里带 `code=`/`desc_len=` 之类排障字段，那些留在 `ice-vision` 自己的日志里，
/// 这一层只给一个能对齐 `VisionError` 定义的名字。
fn vision_variant_name(err: &VisionError) -> &'static str {
    match err {
        VisionError::UnsupportedPlatform => "UnsupportedPlatform",
        VisionError::Decode(_) => "Decode",
        VisionError::TooLarge(..) => "TooLarge",
        VisionError::Recognition(_) => "Recognition",
        VisionError::NoTextFound => "NoTextFound",
        VisionError::Composite(_) => "Composite",
        VisionError::Font(_) => "Font",
        VisionError::NotImplemented(_) => "NotImplemented",
    }
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
    use crate::state::{ComposeStage, KeyState, RecognitionStage, TranslateStage};
    use crate::stub::{StubBehavior, StubEngine, StubError};
    use ice_core::document::{PixelRect, TextBlock};
    use ice_vision::composite::Composed;

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

        let stale = Outcome::Translation {
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

    // ---- 逐块翻译那一段（CHG-006 T-007）----

    impl Harness {
        /// 一份"识别完了"的状态：入场图 + 每块一行原文，逐块翻译的前置条件就此成立。
        fn with_blocks(&mut self, texts: &[&str]) {
            self.state.image = Some(Admitted {
                image: RgbaImage::from_pixel(600, 400, image::Rgba([255, 255, 255, 255])),
                scaled: false,
                width: 600,
                height: 400,
                label: "sample.png".to_string(),
            });
            let mut document = OcrDocument::empty(600, 400);
            document.blocks = texts
                .iter()
                .enumerate()
                .map(|(index, text)| {
                    TextBlock::new(
                        index as u32,
                        *text,
                        PixelRect::new(10.0, index as f32 * 30.0, 300.0, 24.0),
                        0.9,
                    )
                })
                .collect();
            self.state.ocr = Some(document);
            self.state.image_stage = RecognitionStage::Done {
                blocks: texts.len(),
            };
        }

        /// 派发一批逐块翻译，把中间每条进度留下、最后那条终结结果单独给出来。
        fn drain_translate(&mut self) -> (Vec<(usize, usize)>, Outcome) {
            let mut progress = Vec::new();
            loop {
                let outcome = self
                    .runtime
                    .block_on(async { self.rx.recv().await.expect("结果应当回来") });
                match outcome {
                    Outcome::ImageTranslateProgress { done, total, .. } => {
                        progress.push((done, total));
                    }
                    other => return (progress, other),
                }
            }
        }
    }

    /// REQ-001 第一条 + 第二条：一块一次请求，中间那块失败只记在它自己头上，
    /// 前面已落地的译文不回滚，后面那块照旧翻出来。
    #[test]
    fn one_request_per_block_and_one_failure_taints_only_that_block() {
        let stub =
            StubEngine::replying_per_text(vec![("第二块".to_string(), Err(StubError::Timeout))]);
        let mut harness = Harness::new(vec![Arc::new(stub.clone())]);
        harness.with_blocks(&["第一块", "第二块", "第三块"]);
        let ctx = Context::default();
        let job = harness
            .state
            .submit_image_translate(&[])
            .expect("三块该派发得出");
        harness.runner.submit(&harness.runtime, job, &ctx);
        let (progress, outcome) = harness.drain_translate();
        assert!(apply(&mut harness.state, &harness.history, outcome));

        assert_eq!(stub.calls(), 3, "三块就是三次请求，不多也不少");
        assert_eq!(
            progress,
            vec![(1, 3), (2, 3), (3, 3)],
            "进度要一块一跳且带上总数"
        );

        let blocks = &harness.state.ocr.as_ref().expect("文档还在").blocks;
        assert_eq!(blocks[0].translation.as_deref(), Some("译文·第一块"));
        assert_eq!(blocks[1].translation, None, "失败的那块不该有一条假译文");
        assert_eq!(blocks[2].translation.as_deref(), Some("译文·第三块"));
        assert_eq!(
            harness.state.failed_blocks,
            vec![1],
            "失败台账只记那一块，重试才只重发它"
        );
        assert!(matches!(
            harness.state.translate_stage,
            TranslateStage::Done { .. }
        ));
    }

    /// REQ-001 第三条：一批在飞时不派发第二批——第二次派发给不出任务，
    /// 而不是"把第一批顶掉"（顶掉意味着用户看到的译文来自两次请求）。
    #[test]
    fn a_second_batch_does_not_dispatch_while_the_first_is_in_flight() {
        let stub = StubEngine::replying_per_text(Vec::new()).with_delay(Duration::from_millis(60));
        let mut harness = Harness::new(vec![Arc::new(stub.clone())]);
        harness.with_blocks(&["第一块", "第二块"]);
        let ctx = Context::default();
        let job = harness.state.submit_image_translate(&[]).unwrap();
        harness.runner.submit(&harness.runtime, job, &ctx);
        assert!(
            harness.state.submit_image_translate(&[]).is_none(),
            "第一批还在飞就派出了第二批"
        );
        // 收尾：把在飞的这批读完，免得状态留在"翻译中"影响下一次派发
        let (_, outcome) = harness.drain_translate();
        apply(&mut harness.state, &harness.history, outcome);
    }

    /// 停止只在块边界生效（REQ-009）：已请求的那一块会跑完，之后的块一次都不发；
    /// 已翻好的译文留在原处。
    ///
    /// 为什么是"两块"而不是"一块"：进度是在一块翻完那一刻发出来的，而工作循环发完就接着
    /// 请求下一块——主线程看到那条进度时，第二次的请求已经在飞了。那一次撤不回（REQ-009 的
    /// 原话），所以块边界停下来的代价最多是一块，一条都不许多。
    #[test]
    fn a_stop_at_the_block_boundary_keeps_what_was_already_translated() {
        let stub = StubEngine::replying_per_text(Vec::new()).with_delay(Duration::from_millis(200));
        let mut harness = Harness::new(vec![Arc::new(stub.clone())]);
        harness.with_blocks(&["第一块", "第二块", "第三块"]);
        let ctx = Context::default();
        let job = harness.state.submit_image_translate(&[]).unwrap();
        let cancel = match &job {
            Job::ImageTranslate { cancel, .. } => Arc::clone(cancel),
            other => panic!("这条测试要的是逐块翻译那一支，实际是 {other:?}"),
        };
        harness.runner.submit(&harness.runtime, job, &ctx);

        // 第一块的进度回来时按下停止
        let first = harness
            .runtime
            .block_on(async { harness.rx.recv().await.expect("进度应当回来") });
        assert!(matches!(
            first,
            Outcome::ImageTranslateProgress { done: 1, .. }
        ));
        cancel.store(true, Ordering::Relaxed);

        let (rest, outcome) = harness.drain_translate();
        assert_eq!(
            rest,
            vec![(2, 3)],
            "块边界之后只该剩那一次在飞的请求，第三块一次都不发"
        );
        assert_eq!(stub.calls(), 2, "停下之后还多打了接口");
        let results = match &outcome {
            Outcome::ImageTranslated {
                results, cancelled, ..
            } => {
                assert!(*cancelled, "带停止标记的那一批要如实标出来");
                results
            }
            other => panic!("最后一条该是逐块翻译的结果，实际是 {other:?}"),
        };
        assert_eq!(results.len(), 2, "只有真发过请求的那两块有结果");
        assert!(apply(&mut harness.state, &harness.history, outcome));
        assert_eq!(
            harness.state.translate_stage,
            TranslateStage::Stopped { ok: 2, failed: 0 },
            "停在块边界要说'已停止'，并带着已经落地的块数"
        );
        let blocks = &harness.state.ocr.as_ref().unwrap().blocks;
        assert_eq!(
            blocks[0].translation.as_deref(),
            Some("译文·第一块"),
            "已翻好的那块不许因为停止而消失"
        );
        assert_eq!(
            blocks[1].translation.as_deref(),
            Some("译文·第二块"),
            "在飞那一次的译文照样落地"
        );
        assert_eq!(blocks[2].translation, None, "没发过请求的块不该有条假译文");
    }

    // ---- 合成那一段（CHG-006 T-008）----

    /// 合成只有一个结局：整张图画完（`Ok`）或者什么都没有。中间态不往外发，
    /// 所以界面上永远不会出现画了一半的那张图（REQ-009 第四条）。
    #[test]
    fn a_compose_job_returns_one_whole_picture_and_no_intermediate_frames() {
        let mut harness = Harness::new(vec![Arc::new(StubEngine::replying(
            EngineKind::Machine,
            "你好",
        ))]);
        harness.with_blocks(&["第一块", "第二块"]);
        for block in &mut harness.state.ocr.as_mut().unwrap().blocks {
            block.translation = Some("回填文字".to_string());
        }
        let ctx = Context::default();
        let job = harness
            .state
            .submit_image_compose()
            .expect("有译文就该派发得出合成");
        harness.runner.submit(&harness.runtime, job, &ctx);

        let outcome = harness
            .runtime
            .block_on(async { harness.rx.recv().await.expect("成品应当回来") });
        assert!(
            matches!(outcome, Outcome::ImageComposed { .. }),
            "合成段中间不发东西：{outcome:?}"
        );
        assert!(harness.rx.try_recv().is_err(), "一次合成只给一条结果");
        assert!(apply(&mut harness.state, &harness.history, outcome));
        assert!(
            matches!(harness.state.compose_stage, ComposeStage::Done),
            "成品落地后合成段该说完成，实际是 {:?}",
            harness.state.compose_stage
        );
        let composed = harness.state.composed.expect("成品要落地");
        assert_eq!(
            (composed.image.width(), composed.image.height()),
            (600, 400)
        );
        assert_eq!(
            composed.replaced + composed.overlaid + composed.skipped,
            2,
            "计数要和块数对上：{:?}",
            composed.per_block
        );
    }

    /// 换图之后，上一张图的成品与在飞的合成都不该再落回界面（三个序号各自过期）。
    #[test]
    fn a_stale_compose_result_does_not_replace_the_new_picture() {
        let mut harness = Harness::new(vec![Arc::new(StubEngine::replying(
            EngineKind::Machine,
            "你好",
        ))]);
        harness.with_blocks(&["第一块"]);
        harness.state.ocr.as_mut().unwrap().blocks[0].translation = Some("你好".to_string());
        let stale = Outcome::ImageComposed {
            id: harness.state.compose_request_id + 99,
            result: Ok(Composed {
                image: RgbaImage::from_pixel(4, 4, image::Rgba([0, 0, 0, 255])),
                per_block: Vec::new(),
                replaced: 1,
                overlaid: 0,
                skipped: 0,
                degraded_blocks: Vec::new(),
                missing_glyphs: Vec::new(),
            }),
            elapsed_ms: 5,
        };
        assert!(
            !apply(&mut harness.state, &harness.history, stale),
            "对不上号的成品落进了界面"
        );
        assert!(harness.state.composed.is_none());
        assert!(matches!(
            harness.state.compose_stage,
            ComposeStage::Idle | ComposeStage::Done
        ));
    }
}
