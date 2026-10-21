//! 离线验证用的假引擎（`#[cfg(test)]` 专用）。
//!
//! `ice-providers` 里的 `MockEngine` 是 `#[cfg(test)]` 私有的、且只能回固定文本，
//! 无法注入 `ProviderError`；REQ-004 的错误映射需要更强的假引擎，所以界面层自带一份。
//! 它可以设定回复文本、延迟、要抛的错误变体，并记录调用次数与最后一次收到的请求
//! （用来断言术语提示词确实注入了）。

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use ice_core::language::{EngineKind, Language};
use ice_providers::engine::TranslationEngine;
use ice_providers::error::ProviderError;
use ice_providers::request::{TranslationRequest, TranslationResult};

/// 要注入的错误变体。`ProviderError` 不是 `Clone`，所以这里存"怎么造"而不是"造好的"。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StubError {
    NotImplemented,
    Http,
    Timeout,
    Status,
    Unauthorized,
    RateLimited,
    NotConfigured,
    Decode,
    Empty,
    EmptyInput,
    SameLanguage,
}

impl StubError {
    pub fn build(self) -> ProviderError {
        match self {
            StubError::NotImplemented => ProviderError::NotImplemented("stub"),
            StubError::Http => ProviderError::Http("无法连接 stub.example：网络不可达".into()),
            StubError::Timeout => ProviderError::Timeout(15_000, 2),
            StubError::Status => ProviderError::Status(503, "service unavailable".into()),
            StubError::Unauthorized => ProviderError::Unauthorized,
            StubError::RateLimited => ProviderError::RateLimited,
            StubError::NotConfigured => ProviderError::NotConfigured,
            StubError::Decode => ProviderError::Decode("缺少 choices 字段".into()),
            StubError::Empty => ProviderError::Empty,
            StubError::EmptyInput => ProviderError::EmptyInput,
            StubError::SameLanguage => ProviderError::SameLanguage("简体中文"),
        }
    }

    pub const fn all() -> [StubError; 11] {
        [
            StubError::NotImplemented,
            StubError::Http,
            StubError::Timeout,
            StubError::Status,
            StubError::Unauthorized,
            StubError::RateLimited,
            StubError::NotConfigured,
            StubError::Decode,
            StubError::Empty,
            StubError::EmptyInput,
            StubError::SameLanguage,
        ]
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum StubBehavior {
    Ok(String),
    Err(StubError),
}

/// 可克隆：注册进 `Translator` 的是 `Arc<dyn TranslationEngine>`，
/// 测试留一份克隆当句柄，才能读回调用次数与最后一次请求。
#[derive(Clone)]
pub struct StubEngine {
    kind: EngineKind,
    behavior: StubBehavior,
    delay: Duration,
    calls: Arc<AtomicUsize>,
    last_request: Arc<Mutex<Option<TranslationRequest>>>,
}

impl StubEngine {
    pub fn new(kind: EngineKind, behavior: StubBehavior) -> Self {
        Self {
            kind,
            behavior,
            delay: Duration::ZERO,
            calls: Arc::new(AtomicUsize::new(0)),
            last_request: Arc::new(Mutex::new(None)),
        }
    }

    pub fn replying(kind: EngineKind, text: impl Into<String>) -> Self {
        Self::new(kind, StubBehavior::Ok(text.into()))
    }

    pub fn failing(kind: EngineKind, error: StubError) -> Self {
        Self::new(kind, StubBehavior::Err(error))
    }

    pub fn with_delay(mut self, delay: Duration) -> Self {
        self.delay = delay;
        self
    }

    pub fn calls(&self) -> usize {
        self.calls.load(Ordering::SeqCst)
    }

    pub fn last_request(&self) -> Option<TranslationRequest> {
        self.last_request.lock().expect("stub mutex").clone()
    }
}

#[async_trait]
impl TranslationEngine for StubEngine {
    fn kind(&self) -> EngineKind {
        self.kind
    }

    async fn translate(
        &self,
        req: &TranslationRequest,
    ) -> Result<TranslationResult, ProviderError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        *self.last_request.lock().expect("stub mutex") = Some(req.clone());
        if self.delay > Duration::ZERO {
            tokio::time::sleep(self.delay).await;
        }
        match &self.behavior {
            StubBehavior::Ok(text) => Ok(TranslationResult {
                text: text.clone(),
                engine: self.kind,
                // 显式指定源语言时不回显"检测到"（FR-1.2）
                detected: req.pair.source.is_none().then_some(Language::En),
                elapsed_ms: 12,
                cache_hit: false,
                segments: 1,
            }),
            StubBehavior::Err(error) => Err(error.build()),
        }
    }
}
