//! AI 通道（FR-1.4、FR-1.5、FR-1.9、NFR-1）。
//!
//! 只走 OpenAI 兼容的 `POST /chat/completions`：供应商差异收敛在 `AiConfig` 的预设里
//! （base_url + 默认模型），本模块不认任何一家的私有字段。
//! Key 由构造参数传入——**本模块不读钥匙串、不读配置文件**（ADR-005 的归属层在 P4 的 ice-app）。

use std::hash::{Hash, Hasher};
use std::time::{Duration, Instant};

use async_trait::async_trait;
use ice_core::config::{AiConfig, Config};
use ice_core::language::EngineKind;
use serde::Deserialize;
use serde_json::{json, value::Value};

use crate::engine::TranslationEngine;
use crate::error::{ProviderError, ProviderResult, from_reqwest, from_status, should_retry};
use crate::protect::{chunk_chars, protect, restore};
use crate::request::{TranslationRequest, TranslationResult};

/// 网络类失败的额外重试次数（首次之外），与机翻通道同策略
const EXTRA_ATTEMPTS: u32 = 2;
/// 承接上文的长度：够定住语气与术语，又不至于每段都重放整篇
const CONTEXT_TAIL_CHARS: usize = 160;

#[derive(Clone)]
pub struct AiEngine {
    cfg: AiConfig,
    api_key: String,
    client: reqwest::Client,
    timeout: Duration,
    segment_chars: usize,
}

/// `Debug` 手写：`{:?}` 是最常见的泄露途径，Key 只打印成长度。
impl std::fmt::Debug for AiEngine {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AiEngine")
            .field("provider", &self.cfg.provider)
            .field("model", &self.cfg.effective_model())
            .field("endpoint", &self.cfg.endpoint())
            .field("key_len", &self.api_key.chars().count())
            .field("timeout_ms", &self.timeout.as_millis())
            .finish_non_exhaustive()
    }
}

/// 只用于日志关联的文本指纹：不承担完整性或安全校验，所以用标准库默认哈希即可。
fn text_fingerprint(text: &str) -> u64 {
    let mut hasher = std::hash::DefaultHasher::new();
    text.hash(&mut hasher);
    hasher.finish()
}

impl AiEngine {
    /// `segment_chars` 由调用方给（配置里的 `translation.ai_chunk_chars`），
    /// 测试里用小值就能验证多段行为。
    pub fn new(
        cfg: AiConfig,
        api_key: impl Into<String>,
        timeout: Duration,
        segment_chars: usize,
    ) -> Self {
        Self {
            client: reqwest::Client::builder()
                .timeout(timeout)
                .build()
                .unwrap_or_default(),
            api_key: api_key.into(),
            cfg,
            timeout,
            segment_chars: segment_chars.max(1),
        }
    }

    /// 用应用配置装配：AI 段与超时都来自 `config.toml`，端点在 `AiConfig::endpoint()` 归一化
    pub fn from_config(cfg: &Config, api_key: impl Into<String>) -> Self {
        Self::new(
            cfg.ai.clone(),
            api_key,
            Duration::from_millis(cfg.translation.timeout_ms),
            cfg.translation.ai_chunk_chars,
        )
    }

    fn system_prompt(&self, req: &TranslationRequest) -> String {
        let source = match req.pair.source {
            Some(s) => format!("把用户给出的{}原文", s.display_name()),
            None => "把用户给出的原文（语言由你自行判断）".to_string(),
        };
        let mut lines = vec![
            format!(
                "你是专业翻译引擎，{source}翻译成{}。",
                req.pair.target.display_name()
            ),
            "只输出译文本身：不要解释、不要加引号、不要包在代码块里。".to_string(),
            "原文中形如 [[0]]、[[17]] 的占位符代表不可翻译的内容，必须原样保留、不得增删或改写编号，每个占位符在译文中恰好出现一次。".to_string(),
            "保留原文的换行与段落结构。".to_string(),
        ];
        if let Some(hint) = req
            .glossary_hint
            .as_deref()
            .filter(|h| !h.trim().is_empty())
        {
            lines.push(format!("术语偏好（命中时必须采用）：{hint}"));
        }
        if let Some(tail) = req.context_tail.as_deref().filter(|t| !t.trim().is_empty()) {
            lines.push(format!(
                "承接上文（只用于统一语气与术语，不要翻译）：{tail}"
            ));
        }
        lines.join("\n")
    }

    /// 一次 chat/completions 调用。`retries` 只用于让超时错误能说出"已重试几次"。
    async fn request_once(&self, req: &TranslationRequest, retries: u32) -> ProviderResult<String> {
        let mut body = json!({
            "model": self.cfg.effective_model(),
            "messages": [
                { "role": "system", "content": self.system_prompt(req) },
                { "role": "user", "content": req.text },
            ],
        });
        // 部分供应商拒绝 temperature 为空值，缺省时整个字段不发
        if let Some(t) = self.cfg.temperature {
            body["temperature"] = Value::from(t);
        }

        let response = self
            .client
            .post(self.cfg.endpoint())
            .bearer_auth(self.api_key.trim())
            .json(&body)
            .send()
            .await
            .map_err(|e| from_reqwest(&e, self.timeout, retries))?;
        let status = response.status();
        let text = response
            .text()
            .await
            .map_err(|e| from_reqwest(&e, self.timeout, retries))?;
        if !status.is_success() {
            return Err(from_status(status.as_u16(), &text));
        }
        let parsed: ChatResponse = serde_json::from_str(&text)
            .map_err(|e| ProviderError::Decode(format!("AI 响应不是预期 JSON：{e}")))?;
        parsed.into_text()
    }

    async fn request_with_retry(&self, req: &TranslationRequest) -> ProviderResult<String> {
        let mut retries = 0u32;
        loop {
            match self.request_once(req, retries).await {
                Ok(text) => return Ok(text),
                Err(err) if should_retry(&err) && retries < EXTRA_ATTEMPTS => {
                    retries += 1;
                    tracing::debug!("AI 第 {retries} 次重试，原因：{err}");
                }
                Err(err) => return Err(err),
            }
        }
    }
}

#[derive(Debug, Deserialize)]
struct ChatResponse {
    #[serde(default)]
    choices: Vec<Choice>,
}

#[derive(Debug, Deserialize)]
struct Choice {
    #[serde(default)]
    message: Option<Message>,
}

#[derive(Debug, Deserialize)]
struct Message {
    #[serde(default)]
    content: Option<String>,
}

impl ChatResponse {
    /// 只报结构问题，不回显响应体：响应体里是用户的译文
    fn into_text(self) -> ProviderResult<String> {
        let content = self
            .choices
            .into_iter()
            .next()
            .and_then(|c| c.message)
            .and_then(|m| m.content)
            .ok_or_else(|| {
                ProviderError::Decode("AI 响应缺少 choices[0].message.content".into())
            })?;
        if content.trim().is_empty() {
            return Err(ProviderError::Empty);
        }
        Ok(content)
    }
}

#[async_trait]
impl TranslationEngine for AiEngine {
    fn kind(&self) -> EngineKind {
        EngineKind::Ai
    }

    /// 配置完整（地址+模型+Key 标记）且真拿到了 Key 才算可用。
    /// 调度层会先拦一次，这里是防止绕过调度层直接调用。
    fn is_available(&self) -> bool {
        self.cfg.is_ready() && !self.api_key.trim().is_empty()
    }

    async fn translate(&self, req: &TranslationRequest) -> ProviderResult<TranslationResult> {
        if req.is_empty() {
            return Err(ProviderError::EmptyInput);
        }
        if !self.is_available() {
            return Err(ProviderError::NotConfigured);
        }
        let protected = protect(&req.text);
        let segments = chunk_chars(&protected.template, self.segment_chars);
        let started = Instant::now();
        tracing::debug!(
            model = %self.cfg.effective_model(),
            len = req.char_count(),
            fp = %format!("{:08x}", text_fingerprint(&req.text) as u32),
            segments = segments.len(),
            "发起 AI 翻译"
        );

        let mut joined = String::new();
        let mut sent = req.clone();
        for (index, segment) in segments.iter().enumerate() {
            sent.text = segment.clone();
            // 第二段起带上上一段译文的尾巴，让分段读起来像一篇（FR-1.9）
            sent.context_tail = if index == 0 {
                req.context_tail.clone()
            } else {
                Some(tail_of(&joined))
            };
            let piece = self.request_with_retry(&sent).await?;
            let chunk_ended_with_newline = segment.ends_with('\n');
            joined.push_str(&piece);
            if chunk_ended_with_newline && !joined.ends_with('\n') {
                joined.push('\n');
            }
        }

        let restored = restore(&joined, &protected.originals);
        if !restored.lost.is_empty() {
            return Err(ProviderError::Decode(format!(
                "模型丢掉了占位符，未能还原的内容编号：{:?}",
                restored.lost
            )));
        }
        let detected = match req.pair.source {
            Some(_) => None,
            None => ice_core::detect(&req.text)
                .filter(|d| d.confidence >= 0.3)
                .map(|d| d.language),
        };
        Ok(TranslationResult {
            text: restored.text,
            engine: EngineKind::Ai,
            detected,
            elapsed_ms: started.elapsed().as_millis() as u64,
            cache_hit: false,
            segments: segments.len(),
        })
    }
}

/// 上一段译文的尾部，用于下一段的"承接上文"
fn tail_of(text: &str) -> String {
    let chars: Vec<char> = text.chars().collect();
    let start = chars.len().saturating_sub(CONTEXT_TAIL_CHARS);
    chars[start..].iter().collect()
}

#[cfg(test)]
mod tests {
    use ice_core::config::AiProvider;
    use ice_core::language::{LangPair, Language};
    use serde_json::Value;

    use super::*;
    use crate::fake_server::{serve, serve_status, wait_for_requests};

    fn cfg() -> AiConfig {
        AiConfig {
            provider: AiProvider::DeepSeek,
            key_configured: true,
            ..Default::default()
        }
    }

    /// 指向假服务的引擎：base_url 手填覆盖预设，分段上限由测试决定
    fn engine(fake: &str, segment_chars: usize) -> AiEngine {
        let cfg = AiConfig {
            provider: AiProvider::Custom,
            base_url: fake.into(),
            model: "test-model".into(),
            key_configured: true,
            ..Default::default()
        };
        AiEngine::new(
            cfg,
            "sk-test-key-1234567890",
            Duration::from_millis(300),
            segment_chars,
        )
    }

    fn reply(text: &str) -> String {
        format!(r#"{{"choices":[{{"message":{{"role":"assistant","content":"{text}"}}}}]}}"#)
    }

    fn request(text: &str, pair: LangPair) -> TranslationRequest {
        TranslationRequest::new(text, pair)
    }

    #[tokio::test]
    async fn translates_single_segment() {
        let fake = serve(vec![reply("你好世界")]);
        let result = engine(&fake.url, 3500)
            .translate(&request(
                "hello world",
                LangPair::explicit(Language::En, Language::ZhHans),
            ))
            .await
            .expect("应成功");
        assert_eq!(result.text, "你好世界");
        assert_eq!(result.engine, EngineKind::Ai);
        assert_eq!(result.segments, 1);
        assert_eq!(result.detected, None);
    }

    #[tokio::test]
    async fn request_body_follows_openai_shape() {
        let fake = serve(vec![reply("好")]);
        engine(&fake.url, 3500)
            .translate(&request(
                "hello",
                LangPair::explicit(Language::En, Language::ZhHans),
            ))
            .await
            .unwrap();
        let raw = wait_for_requests(&fake, 1)[0].clone();
        let body = raw.split("\r\n\r\n").nth(1).unwrap_or_default();
        let parsed: Value = serde_json::from_str(body).expect("请求体应为 JSON");
        assert_eq!(parsed["model"], "test-model");
        assert_eq!(parsed["messages"][0]["role"], "system");
        assert_eq!(parsed["messages"][1]["content"], "hello");
        // 未配置 temperature 时字段整个不发
        assert!(parsed.get("temperature").is_none(), "{parsed}");
        assert!(
            raw.to_lowercase()
                .contains("authorization: bearer sk-test-key")
        );
    }

    #[tokio::test]
    async fn temperature_is_sent_when_configured() {
        let fake = serve(vec![reply("好")]);
        let mut e = engine(&fake.url, 3500);
        e.cfg.temperature = Some(0.2);
        e.translate(&request(
            "hi",
            LangPair::explicit(Language::En, Language::ZhHans),
        ))
        .await
        .unwrap();
        let raw = wait_for_requests(&fake, 1)[0].clone();
        let body = raw.split("\r\n\r\n").nth(1).unwrap_or_default();
        let parsed: Value = serde_json::from_str(body).unwrap();
        // f32 0.2 展开成 f64 是 0.2000000029…，比数值而不是比字面量
        let t = parsed["temperature"].as_f64().expect("应带 temperature");
        assert!((t - 0.2).abs() < 1e-6, "{t}");
    }

    #[tokio::test]
    async fn glossary_and_target_appear_in_system_prompt() {
        let fake = serve(vec![reply("好")]);
        let e = engine(&fake.url, 3500);
        let mut req = request("hi", LangPair::explicit(Language::En, Language::ZhHans));
        req.glossary_hint = Some("engine → 引擎".into());
        e.translate(&req).await.unwrap();
        let raw = wait_for_requests(&fake, 1)[0].clone();
        let body = raw.split("\r\n\r\n").nth(1).unwrap_or_default();
        let parsed: Value = serde_json::from_str(body).unwrap();
        let prompt = parsed["messages"][0]["content"].as_str().unwrap();
        assert!(prompt.contains("简体中文"), "{prompt}");
        assert!(prompt.contains("engine → 引擎"), "{prompt}");
        assert!(prompt.contains("[[0]]"), "占位符规则必须写进提示词");
    }

    #[tokio::test]
    async fn long_text_is_split_and_second_segment_gets_context() {
        let fake = serve(vec![reply("第一段译文末尾是甲乙丙丁"), reply("第二段译文")]);
        // 上限设小，逼出两段
        let result = engine(&fake.url, 20)
            .translate(&request(
                &"一二三四五六七八九〇".repeat(4),
                LangPair::explicit(Language::ZhHans, Language::En),
            ))
            .await
            .expect("应分两段成功");
        assert_eq!(result.segments, 2);
        let requests = wait_for_requests(&fake, 2);
        let second = requests[1].split("\r\n\r\n").nth(1).unwrap_or_default();
        let parsed: Value = serde_json::from_str(second).unwrap();
        let prompt = parsed["messages"][0]["content"].as_str().unwrap();
        assert!(
            prompt.contains("承接上文") && prompt.contains("第一段译文末尾是甲乙丙丁"),
            "第二段应收到第一段译文尾巴: {prompt}"
        );
    }

    #[tokio::test]
    async fn segments_join_in_order_and_keep_the_line_break() {
        // "abcd\nefgh" 在 6 字符上限下切成 "abcd\n" 与 "efgh"
        let fake = serve(vec![reply("第一行"), reply("第二行")]);
        let result = engine(&fake.url, 6)
            .translate(&request(
                "abcd\nefgh",
                LangPair::explicit(Language::En, Language::ZhHans),
            ))
            .await
            .unwrap();
        assert_eq!(result.segments, 2);
        assert_eq!(
            result.text, "第一行\n第二行",
            "模型吞掉换行时不能把两段黏成一行"
        );
    }

    #[tokio::test]
    async fn second_segment_failure_returns_error_not_half_output() {
        let fake = serve(vec![
            reply("第一段"),
            r#"{"error":{"message":"boom"}}"#.into(),
        ]);
        let err = engine(&fake.url, 4)
            .translate(&request(
                "12345678",
                LangPair::explicit(Language::En, Language::ZhHans),
            ))
            .await
            .expect_err("任一段失败必须整体失败");
        assert!(matches!(err, ProviderError::Decode(_)), "{err:?}");
    }

    #[tokio::test]
    async fn auth_and_rate_limit_map_to_distinct_errors() {
        let fake = serve_status(vec![(401, r#"{"error":"bad key"}"#.into())]);
        let err = engine(&fake.url, 3500)
            .translate(&request(
                "hello 机密",
                LangPair::explicit(Language::En, Language::ZhHans),
            ))
            .await
            .expect_err("401 必须失败");
        assert!(matches!(err, ProviderError::Unauthorized), "{err:?}");
        assert!(!err.retryable(), "Key 无效不该自动重试");
        // 一次请求都不重试
        assert_eq!(wait_for_requests(&fake, 1).len(), 1);

        let fake = serve_status(vec![(429, "".into())]);
        let err = engine(&fake.url, 3500)
            .translate(&request(
                "hello",
                LangPair::explicit(Language::En, Language::ZhHans),
            ))
            .await
            .expect_err("429 必须失败");
        assert!(matches!(err, ProviderError::RateLimited), "{err:?}");

        let fake = serve_status(vec![(500, "backend exploded".into())]);
        let err = engine(&fake.url, 3500)
            .translate(&request(
                "hello",
                LangPair::explicit(Language::En, Language::ZhHans),
            ))
            .await
            .expect_err("5xx 必须失败");
        assert!(
            matches!(&err, ProviderError::Status(500, t) if t == "backend exploded"),
            "{err:?}"
        );
        // 状态码类失败重试也不会改变结果，一律不自动重试（REQ-005）
        assert!(!err.retryable(), "{err}");
        assert_eq!(wait_for_requests(&fake, 1).len(), 1);
    }

    #[tokio::test]
    async fn missing_placeholder_from_model_is_a_failure() {
        let fake = serve(vec![reply("看 与 的说明")]);
        let err = engine(&fake.url, 3500)
            .translate(&request(
                "看 https://a.example.com 与 12 MB 的说明",
                LangPair::explicit(Language::En, Language::ZhHans),
            ))
            .await
            .expect_err("占位符丢失必须报失败");
        assert!(matches!(err, ProviderError::Decode(_)), "{err:?}");
    }

    #[tokio::test]
    async fn unavailable_without_key_or_config() {
        let cfg = cfg();
        let no_key = AiEngine::new(cfg.clone(), "", Duration::from_millis(100), 3500);
        assert!(!no_key.is_available());
        let incomplete = AiConfig {
            provider: AiProvider::Custom,
            key_configured: true,
            ..Default::default()
        };
        assert!(
            !AiEngine::new(incomplete, "k", Duration::from_millis(100), 3500).is_available(),
            "Custom 没填地址时不可用"
        );
        let err = no_key
            .translate(&request(
                "hi",
                LangPair::explicit(Language::En, Language::ZhHans),
            ))
            .await
            .expect_err("不可用引擎不得发请求");
        assert!(matches!(err, ProviderError::NotConfigured), "{err:?}");
    }

    #[test]
    fn preset_endpoint_has_no_duplicate_suffix() {
        for p in AiProvider::all() {
            let cfg = AiConfig {
                provider: p,
                key_configured: !matches!(p, AiProvider::Custom),
                ..Default::default()
            };
            if p == AiProvider::Custom {
                assert!(cfg.endpoint().is_empty() || cfg.endpoint().ends_with("chat/completions"));
                continue;
            }
            let url = cfg.endpoint();
            assert_eq!(url.matches("/chat/completions").count(), 1, "{url}");
            assert!(url.starts_with("https://"), "{url}");
        }
    }

    #[test]
    fn debug_output_hides_the_key() {
        let e = AiEngine::new(
            cfg(),
            "sk-super-secret-value",
            Duration::from_millis(100),
            3500,
        );
        let text = format!("{e:?}");
        assert!(!text.contains("super-secret"), "{text}");
        assert!(text.contains("key_len"), "{text}");
    }

    #[test]
    fn fingerprint_is_stable_and_short() {
        let a = text_fingerprint("hello");
        assert_eq!(a, text_fingerprint("hello"));
        assert_ne!(a, text_fingerprint("hello!"));
        assert_eq!(format!("{:08x}", a as u32).len(), 8);
    }

    #[tokio::test]
    async fn error_text_never_carries_the_source_text() {
        let fake = serve(vec![r#"{"detail":"unexpected"}"#.into()]);
        let err = engine(&fake.url, 3500)
            .translate(&request(
                "机密合同编号内容",
                LangPair::explicit(Language::ZhHans, Language::En),
            ))
            .await
            .expect_err("缺 choices 应失败");
        let text = err.to_string();
        assert!(text.contains("choices"), "{text}");
        assert!(!text.contains("机密合同"), "错误文案不得带出源文: {text}");
    }
}
