use async_trait::async_trait;
use ice_core::language::EngineKind;

use crate::error::{ProviderError, ProviderResult};
use crate::request::{TranslationRequest, TranslationResult};

/// 一个可调用的翻译通道。实现必须是无状态可并发的（引擎实例被 UI 跨线程共享）。
#[async_trait]
pub trait TranslationEngine: Send + Sync {
    fn kind(&self) -> EngineKind;

    async fn translate(&self, req: &TranslationRequest) -> ProviderResult<TranslationResult>;

    /// 服务是否可用（例如 AI 通道缺 Key 时返回 false，界面据此提示去设置页）
    fn is_available(&self) -> bool {
        true
    }
}

/// 供上层做降级决策：不可用时给出原因
pub fn unavailable_error(engine: EngineKind) -> ProviderError {
    match engine {
        EngineKind::Machine => ProviderError::Http("机翻服务暂时不可用".into()),
        EngineKind::Ai => ProviderError::NotConfigured,
    }
}

#[cfg(test)]
pub mod mock {
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::{Arc, Mutex};

    use ice_core::language::Language;

    use super::*;

    /// 记录调用次数的假引擎，用于验证缓存与调度行为
    pub struct MockEngine {
        kind: EngineKind,
        calls: Arc<AtomicUsize>,
        reply: Mutex<String>,
        delay: std::time::Duration,
    }

    impl MockEngine {
        pub fn new(kind: EngineKind, reply: impl Into<String>) -> Self {
            Self {
                kind,
                calls: Arc::new(AtomicUsize::new(0)),
                reply: Mutex::new(reply.into()),
                delay: std::time::Duration::ZERO,
            }
        }

        pub fn calls(&self) -> Arc<AtomicUsize> {
            self.calls.clone()
        }
    }

    #[async_trait]
    impl TranslationEngine for MockEngine {
        fn kind(&self) -> EngineKind {
            self.kind
        }

        async fn translate(&self, req: &TranslationRequest) -> ProviderResult<TranslationResult> {
            if !self.delay.is_zero() {
                tokio::time::sleep(self.delay).await;
            }
            self.calls.fetch_add(1, Ordering::SeqCst);
            let reply = self.reply.lock().unwrap().clone();
            Ok(TranslationResult {
                text: format!("{reply}:{}", req.text),
                engine: self.kind,
                detected: Some(Language::En),
                elapsed_ms: 1,
                cache_hit: false,
                segments: 1,
            })
        }
    }
}
