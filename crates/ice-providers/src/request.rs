use ice_core::language::{EngineKind, LangPair, Language};
use serde::{Deserialize, Serialize};

/// 一次翻译请求。`glossary_hint` 由调度层从术语表渲染而来并注入 AI 提示词；
/// 机翻通道的译后替换在调度层完成（FR-4.5）。
#[derive(Debug, Clone, PartialEq)]
pub struct TranslationRequest {
    pub text: String,
    pub pair: LangPair,
    pub glossary_hint: Option<String>,
    /// 上一次译文的尾巴，AI 分段翻译时用于保持语气与术语一致（FR-1.9）
    pub context_tail: Option<String>,
}

impl TranslationRequest {
    pub fn new(text: impl Into<String>, pair: LangPair) -> Self {
        Self {
            text: text.into(),
            pair,
            glossary_hint: None,
            context_tail: None,
        }
    }

    pub fn source_code_or_auto(&self) -> String {
        self.pair
            .source
            .map(|l| l.code().to_string())
            .unwrap_or_else(|| "auto".into())
    }

    pub fn is_empty(&self) -> bool {
        self.text.trim().is_empty()
    }

    pub fn char_count(&self) -> usize {
        self.text.chars().count()
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TranslationResult {
    pub text: String,
    pub engine: EngineKind,
    /// 自动检测到的源语言（FR-1.2 需在界面回显）
    pub detected: Option<Language>,
    pub elapsed_ms: u64,
    pub cache_hit: bool,
    /// 分段翻译时的段数，便于排查顺序错乱
    pub segments: usize,
}

impl TranslationResult {
    pub fn cached(text: impl Into<String>, engine: EngineKind) -> Self {
        Self {
            text: text.into(),
            engine,
            detected: None,
            elapsed_ms: 0,
            cache_hit: true,
            segments: 1,
        }
    }
}
