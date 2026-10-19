use std::collections::HashMap;
use std::sync::Arc;
use std::time::Instant;

use ice_core::glossary::Glossary;
use ice_core::language::EngineKind;

use crate::cache::{CacheKey, CachedValue, TranslationCache};
use crate::engine::{TranslationEngine, unavailable_error};
use crate::error::{ProviderError, ProviderResult};
use crate::request::{TranslationRequest, TranslationResult};

/// 调度层：选引擎、查缓存、注入术语、译后替换。
///
/// 缓存里存的是**引擎原始输出**，术语替换在返回前执行，因此改术语表能立刻生效（FR-4.5），
/// 不必清空缓存。
pub struct Translator {
    engines: HashMap<EngineKind, Arc<dyn TranslationEngine>>,
    cache: TranslationCache,
    glossary: Glossary,
    glossary_version: u64,
}

impl Translator {
    pub fn new() -> Self {
        Self {
            engines: HashMap::new(),
            cache: TranslationCache::default(),
            glossary: Glossary::default(),
            glossary_version: 0,
        }
    }

    pub fn register(&mut self, engine: Arc<dyn TranslationEngine>) {
        self.engines.insert(engine.kind(), engine);
    }

    pub fn has_engine(&self, kind: EngineKind) -> bool {
        self.engines.contains_key(&kind)
    }

    pub fn set_glossary(&mut self, glossary: Glossary) {
        self.glossary = glossary;
        self.glossary_version = self.glossary_version.wrapping_add(1);
    }

    pub fn glossary(&self) -> &Glossary {
        &self.glossary
    }

    pub fn cache_hit_rate(&self) -> f32 {
        self.cache.hit_rate()
    }

    pub fn cached_len(&self) -> usize {
        self.cache.len()
    }

    pub fn clear_cache(&self) {
        self.cache.clear();
    }

    /// 引擎可用性，供界面在切换时给出提示（FR-1.7 / R-4）
    pub fn availability(&self) -> Vec<(EngineKind, bool, &'static str)> {
        EngineKind::all()
            .into_iter()
            .map(|kind| match self.engines.get(&kind) {
                None => (kind, false, "引擎未注册"),
                Some(engine) => (kind, engine.is_available(), ""),
            })
            .collect()
    }

    pub async fn translate(
        &self,
        kind: EngineKind,
        req: TranslationRequest,
    ) -> ProviderResult<TranslationResult> {
        if req.is_empty() {
            return Err(ProviderError::EmptyInput);
        }
        let engine = self
            .engines
            .get(&kind)
            .ok_or_else(|| unavailable_error(kind))?;
        if !engine.is_available() {
            return Err(unavailable_error(kind));
        }

        let mut effective = req;
        if kind == EngineKind::Ai {
            let hint = self.glossary.prompt_fragment();
            if !hint.is_empty() {
                effective.glossary_hint = Some(hint);
            }
        }

        let key = CacheKey {
            engine: kind,
            source: effective.source_code_or_auto(),
            target: effective.pair.target.code().to_string(),
            text: effective.text.clone(),
            salt: if kind == EngineKind::Ai {
                self.glossary_version
            } else {
                0
            },
        };

        if let Some(hit) = self.cache.get(&key) {
            return Ok(TranslationResult {
                text: self.finish_text(kind, hit.text),
                engine: kind,
                detected: hit.detected,
                elapsed_ms: 0,
                cache_hit: true,
                segments: 1,
            });
        }

        let started = Instant::now();
        let result = engine.translate(&effective).await?;
        let elapsed_ms = started.elapsed().as_millis() as u64;
        self.cache.insert(
            key,
            CachedValue {
                text: result.text.clone(),
                detected: result.detected,
            },
        );

        Ok(TranslationResult {
            text: self.finish_text(kind, result.text),
            engine: kind,
            detected: result.detected,
            elapsed_ms,
            cache_hit: false,
            segments: result.segments,
        })
    }

    fn finish_text(&self, kind: EngineKind, raw: String) -> String {
        if kind == EngineKind::Machine {
            self.glossary.post_substitute(&raw)
        } else {
            raw
        }
    }
}

impl Default for Translator {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::Ordering;
    use std::sync::{Arc, Mutex};

    use ice_core::glossary::Term;
    use ice_core::language::{LangPair, Language};

    use super::*;
    use crate::engine::mock::MockEngine;

    fn pair() -> LangPair {
        LangPair::auto_to(Language::ZhHans)
    }

    fn translator_with(
        kind: EngineKind,
        reply: &str,
    ) -> (Translator, Arc<std::sync::atomic::AtomicUsize>) {
        let mock = MockEngine::new(kind, reply);
        let calls = mock.calls();
        let mut t = Translator::new();
        t.register(Arc::new(mock));
        (t, calls)
    }

    #[tokio::test]
    async fn empty_input_never_reaches_engine() {
        let (t, calls) = translator_with(EngineKind::Machine, "x");
        let err = t
            .translate(EngineKind::Machine, TranslationRequest::new("   ", pair()))
            .await
            .unwrap_err();
        assert!(matches!(err, ProviderError::EmptyInput));
        assert_eq!(calls.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn unregistered_engine_reports_not_configured_for_ai() {
        let t = Translator::new();
        let err = t
            .translate(EngineKind::Ai, TranslationRequest::new("hi", pair()))
            .await
            .unwrap_err();
        assert!(matches!(err, ProviderError::NotConfigured));
    }

    #[tokio::test]
    async fn second_identical_request_hits_cache_and_skips_engine() {
        let (t, calls) = translator_with(EngineKind::Machine, "机翻");
        let first = t
            .translate(
                EngineKind::Machine,
                TranslationRequest::new("hello", pair()),
            )
            .await
            .unwrap();
        let second = t
            .translate(
                EngineKind::Machine,
                TranslationRequest::new("hello", pair()),
            )
            .await
            .unwrap();
        assert!(!first.cache_hit);
        assert!(second.cache_hit);
        assert_eq!(second.elapsed_ms, 0);
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        assert_eq!(first.text, second.text);
    }

    #[tokio::test]
    async fn different_engines_do_not_share_cache_entries() {
        let mut t = Translator::new();
        let machine = MockEngine::new(EngineKind::Machine, "机翻");
        let ai = MockEngine::new(EngineKind::Ai, "AI");
        let machine_calls = machine.calls();
        let ai_calls = ai.calls();
        t.register(Arc::new(machine));
        t.register(Arc::new(ai));

        let req = TranslationRequest::new("hello", pair());
        let m1 = t.translate(EngineKind::Machine, req.clone()).await.unwrap();
        let a1 = t.translate(EngineKind::Ai, req.clone()).await.unwrap();
        let m2 = t.translate(EngineKind::Machine, req).await.unwrap();
        assert!(m1.text.starts_with("机翻"));
        assert!(a1.text.starts_with("AI"));
        assert!(m2.cache_hit, "机翻第二次应命中自己的缓存");
        assert_eq!(machine_calls.load(Ordering::SeqCst), 1);
        assert_eq!(ai_calls.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn glossary_applies_to_cached_result_without_new_request() {
        let (mut t, calls) = translator_with(EngineKind::Machine, "deploy on Kubernetes");
        let req = TranslationRequest::new("deploy on Kubernetes", pair());
        let before = t.translate(EngineKind::Machine, req.clone()).await.unwrap();
        assert!(before.text.contains("Kubernetes"), "{}", before.text);

        let mut glossary = Glossary::default();
        glossary.upsert(Term::new("Kubernetes", "K8s"));
        t.set_glossary(glossary);

        let after = t.translate(EngineKind::Machine, req).await.unwrap();
        assert!(after.cache_hit, "应仍走缓存，不重复请求");
        assert!(
            after.text.ends_with("K8s"),
            "术语需即时生效: {}",
            after.text
        );
        assert_eq!(calls.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn ai_channel_receives_glossary_hint() {
        struct Spy(Arc<Mutex<Option<String>>>);

        #[async_trait::async_trait]
        impl TranslationEngine for Spy {
            fn kind(&self) -> EngineKind {
                EngineKind::Ai
            }

            async fn translate(
                &self,
                req: &TranslationRequest,
            ) -> ProviderResult<TranslationResult> {
                *self.0.lock().unwrap() = req.glossary_hint.clone();
                Ok(TranslationResult::cached("好的", EngineKind::Ai))
            }
        }

        let seen: Arc<Mutex<Option<String>>> = Arc::new(Mutex::new(None));
        let mut glossary = Glossary::default();
        glossary.upsert(Term::new("latency", "时延"));
        let mut t = Translator::new();
        t.set_glossary(glossary);
        t.register(Arc::new(Spy(seen.clone())));
        t.translate(
            EngineKind::Ai,
            TranslationRequest::new("high latency", pair()),
        )
        .await
        .unwrap();

        let hint = seen.lock().unwrap().clone().expect("AI 通道应收到提示词");
        assert!(hint.contains("latency => 时延"));
    }

    #[tokio::test]
    async fn unavailable_engine_is_refused_before_request() {
        struct Down;

        #[async_trait::async_trait]
        impl TranslationEngine for Down {
            fn kind(&self) -> EngineKind {
                EngineKind::Ai
            }

            fn is_available(&self) -> bool {
                false
            }

            async fn translate(
                &self,
                _req: &TranslationRequest,
            ) -> ProviderResult<TranslationResult> {
                unreachable!("不可用时不应被调用")
            }
        }

        let mut t = Translator::new();
        t.register(Arc::new(Down));
        let err = t
            .translate(EngineKind::Ai, TranslationRequest::new("hi", pair()))
            .await
            .unwrap_err();
        assert!(matches!(err, ProviderError::NotConfigured));
        assert!(!t.availability()[1].1);
    }
}
