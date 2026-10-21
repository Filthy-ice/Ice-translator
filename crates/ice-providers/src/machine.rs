//! 机翻通道（FR-1.3、FR-1.7、NFR-1）。
//!
//! 零配置可用：不需要 Key，直接 GET 公共端点。选型与失效条件见 `docs/adr/ADR-004-machine-translate-endpoint.md`。
//! 单次查询有长度上限（实测 500 字符），因此超长文本先保护再切块，逐块请求后按序拼回。

use std::time::{Duration, Instant};

use async_trait::async_trait;
use ice_core::config::Config;
use ice_core::language::{EngineKind, Language};
use serde::Deserialize;

use crate::engine::TranslationEngine;
use crate::error::{ProviderError, ProviderResult, from_reqwest, from_status, should_retry};
use crate::protect::{chunk_chars, protect, restore};
use crate::request::{TranslationRequest, TranslationResult};

/// MyMemory 实测上限 500 字符，留 50 字符余量给占位符与语言参数
pub const MACHINE_QUERY_CHARS: usize = 450;
pub const DEFAULT_ENDPOINT: &str = "https://api.mymemory.translated.net/get";
/// 网络类失败的额外重试次数（首次之外）
const EXTRA_ATTEMPTS: u32 = 2;

#[derive(Debug, Clone)]
pub struct MachineTranslateEngine {
    client: reqwest::Client,
    endpoint: String,
    timeout: Duration,
}

impl MachineTranslateEngine {
    /// 用应用配置里的超时构造默认通道
    pub fn from_config(cfg: &Config) -> Self {
        Self::with_endpoint(
            DEFAULT_ENDPOINT,
            Duration::from_millis(cfg.translation.timeout_ms),
        )
    }

    /// 显式指定端点与超时：测试指向本地假服务，也便于将来换端点
    pub fn with_endpoint(endpoint: impl Into<String>, timeout: Duration) -> Self {
        let endpoint = endpoint.into();
        Self {
            // 不跟随重定向：让 3xx 变成可读的失败而不是悄悄换地址
            client: reqwest::Client::builder()
                .timeout(timeout)
                .redirect(reqwest::redirect::Policy::none())
                .build()
                .unwrap_or_default(),
            endpoint,
            timeout,
        }
    }

    /// 源语言：显式指定则照用（此时不回显"检测到"，因为并没有检测）；
    /// "自动检测"用本地字符集粗判（FR-1.2）。端点不接受 `auto`，判不出来时按英文请求，
    /// 并把回显留空——不谎称检测到了什么。
    fn source_language(&self, req: &TranslationRequest) -> (Language, Option<Language>) {
        if let Some(source) = req.pair.source {
            return (source, None);
        }
        match ice_core::detect(&req.text).filter(|d| d.confidence >= 0.3) {
            Some(detection) => (detection.language, Some(detection.language)),
            None => (Language::En, None),
        }
    }

    /// 一次 GET。响应状态既可能在 HTTP 层，也可能写在响应体里（实测超限时 HTTP 仍是 200）。
    /// `retries` 是本次调用之前已经重试的次数，只用于让超时错误能说出"试过几次"。
    async fn translate_chunk(
        &self,
        text: &str,
        langpair: &str,
        attempts: u32,
    ) -> ProviderResult<String> {
        let response = self
            .client
            .get(&self.endpoint)
            .query(&[("q", text), ("langpair", langpair)])
            .send()
            .await
            .map_err(|e| from_reqwest(&e, self.timeout, attempts))?;
        let status = response.status();
        let body = response
            .text()
            .await
            .map_err(|e| from_reqwest(&e, self.timeout, attempts))?;
        if !status.is_success() {
            return Err(from_status(status.as_u16(), &body));
        }
        let parsed: MyMemoryResponse = serde_json::from_str(&body)
            .map_err(|e| ProviderError::Decode(format!("机翻响应不是预期 JSON：{e}")))?;
        parsed.into_translation()
    }
}

/// `responseStatus` 实测既可能是数字 `200` 也可能是字符串 `"403"`
#[derive(Debug, Deserialize)]
struct MyMemoryResponse {
    #[serde(rename = "responseData")]
    data: ResponseData,
    #[serde(rename = "responseStatus", default)]
    status: serde_json::Value,
    #[serde(rename = "responseDetails", default)]
    details: String,
}

#[derive(Debug, Deserialize)]
struct ResponseData {
    #[serde(rename = "translatedText", default)]
    translated: String,
}

fn status_code(value: &serde_json::Value) -> Option<u16> {
    match value {
        serde_json::Value::Number(n) => n.as_u64().map(|v| v as u16),
        serde_json::Value::String(s) => s.trim().parse().ok(),
        _ => None,
    }
}

impl MyMemoryResponse {
    fn into_translation(self) -> ProviderResult<String> {
        match status_code(&self.status) {
            Some(200) => {}
            Some(other) => {
                let reason = if self.details.trim().is_empty() {
                    self.data.translated
                } else {
                    self.details
                };
                return Err(ProviderError::Status(other, reason));
            }
            // 状态字段缺失或形态不认识：按解析失败处理，不猜成功
            None => {
                return Err(ProviderError::Decode(format!(
                    "机翻响应缺少可识别的 responseStatus：{:#?}",
                    self.status
                )));
            }
        }
        if self.data.translated.trim().is_empty() {
            return Err(ProviderError::Empty);
        }
        Ok(self.data.translated)
    }
}

#[async_trait]
impl TranslationEngine for MachineTranslateEngine {
    fn kind(&self) -> EngineKind {
        EngineKind::Machine
    }

    async fn translate(&self, req: &TranslationRequest) -> ProviderResult<TranslationResult> {
        if req.is_empty() {
            return Err(ProviderError::EmptyInput);
        }
        let target = req.pair.target.code().to_string();
        let (source, detected) = self.source_language(req);
        // 端点硬要求两端语言不同。"自动检测"判出与目标同语言时（中文→简体中文最常见），
        // 发过去必被拒，所以就地拦下：不发这次请求，也不把端点的英文原文抛给用户
        if source == req.pair.target {
            return Err(ProviderError::SameLanguage(source.display_name()));
        }
        let langpair = format!("{}|{}", source.code(), target);

        let protected = protect(&req.text);
        let chunks = chunk_chars(&protected.template, MACHINE_QUERY_CHARS);
        let started = Instant::now();

        let mut joined = String::new();
        for chunk in &chunks {
            let mut retries = 0u32;
            let text = loop {
                match self.translate_chunk(chunk, &langpair, retries).await {
                    Ok(text) => break text,
                    Err(err) if should_retry(&err) && retries < EXTRA_ATTEMPTS => {
                        retries += 1;
                        tracing::debug!("机翻第 {retries} 次重试，原因：{err}");
                    }
                    Err(err) => return Err(err),
                }
            };
            // 块与块直接相接：切块本身是无损切分（protect 的 chunk 测试保证每块拼回等于整篇）
            joined.push_str(&text);
        }

        let restored = restore(&joined, &protected.originals);
        if !restored.lost.is_empty() {
            return Err(ProviderError::Decode(format!(
                "机翻改写了占位符，未能还原的内容编号：{:?}",
                restored.lost
            )));
        }
        Ok(TranslationResult {
            text: restored.text,
            engine: EngineKind::Machine,
            detected,
            elapsed_ms: started.elapsed().as_millis() as u64,
            cache_hit: false,
            segments: chunks.len(),
        })
    }
}

#[cfg(test)]
mod tests {
    use ice_core::language::LangPair;

    use super::*;
    use crate::fake_server::{closed_url, serve, serve_hanging, wait_for_requests};

    fn engine(fake: &str) -> MachineTranslateEngine {
        MachineTranslateEngine::with_endpoint(fake, Duration::from_millis(300))
    }

    fn ok(text: &str) -> String {
        format!(
            r#"{{"responseData":{{"translatedText":"{text}"}},"responseStatus":200,"responseDetails":""}}"#
        )
    }

    #[tokio::test]
    async fn explicit_source_is_used_and_not_echoed_as_detection() {
        let fake = serve(vec![ok("你好")]);
        let result = engine(&fake.url)
            .translate(&TranslationRequest::new(
                "hello",
                LangPair::explicit(Language::En, Language::ZhHans),
            ))
            .await
            .expect("应成功");
        assert_eq!(result.text, "你好");
        // 用户已经指定源语言，界面不该假装"检测到英文"
        assert_eq!(result.detected, None);
        assert_eq!(result.segments, 1);
        let sent = wait_for_requests(&fake, 1).join("\n");
        assert!(sent.contains("langpair=en%7Czh-CN"), "{sent}");
    }

    #[tokio::test]
    async fn auto_source_uses_local_detection() {
        let fake = serve(vec![ok("ok")]);
        let result = engine(&fake.url)
            .translate(&TranslationRequest::new(
                "今天天气很好，我们出去玩",
                LangPair::auto_to(Language::En),
            ))
            .await
            .expect("应成功");
        assert_eq!(result.detected, Some(Language::ZhHans));
        let sent = wait_for_requests(&fake, 1).join("\n");
        assert!(sent.contains("langpair=zh-CN%7Cen"), "{sent}");
    }

    #[tokio::test]
    async fn undetectable_source_falls_back_to_english_without_echo() {
        let fake = serve(vec![ok("ok")]);
        let result = engine(&fake.url)
            .translate(&TranslationRequest::new(
                "1234567890 !@#$%^&*()",
                LangPair::auto_to(Language::ZhHans),
            ))
            .await
            .expect("应成功");
        assert_eq!(result.detected, None);
        let sent = wait_for_requests(&fake, 1).join("\n");
        assert!(sent.contains("langpair=en%7Czh-CN"), "{sent}");
    }

    /// 从假服务收到的请求行里取出 `q` 参数的原文
    fn query_of(request: &str) -> String {
        let target = request
            .split_whitespace()
            .nth(1)
            .unwrap_or_default()
            .to_string();
        let encoded = target
            .split('?')
            .nth(1)
            .unwrap_or_default()
            .split('&')
            .find(|kv| kv.starts_with("q="))
            .unwrap_or("q=")
            .trim_start_matches("q=");
        urlencoding::decode(encoded)
            .map(String::from)
            .unwrap_or_default()
    }

    #[tokio::test]
    async fn long_text_is_chunked_under_the_limit_and_rejoined() {
        let fake = serve(vec![ok("第一段"), ok("第二段")]);
        let text = "句子。".repeat(200); // 800 字符，超过 450 上限
        let result = engine(&fake.url)
            .translate(&TranslationRequest::new(
                text.clone(),
                LangPair::explicit(Language::ZhHans, Language::En),
            ))
            .await
            .expect("超长文本应分块成功");
        assert_eq!(result.segments, 2);
        assert_eq!(result.text, "第一段第二段");
        let requests = wait_for_requests(&fake, 2);
        let queries: Vec<String> = requests.iter().map(|r| query_of(r)).collect();
        for query in &queries {
            let count = query.chars().count();
            assert!(
                count <= MACHINE_QUERY_CHARS,
                "单块 {count} 字符超过端点上限 {MACHINE_QUERY_CHARS}"
            );
        }
        // 切块无损：拼回去必须等于原文
        assert_eq!(queries.concat(), text);
    }

    #[tokio::test]
    async fn body_level_error_is_not_mistaken_for_translation() {
        let body = r#"{"responseData":{"translatedText":"QUERY LENGTH LIMIT EXCEEDED. MAX ALLOWED QUERY : 500 CHARS"},"responseStatus":"403","responseDetails":"QUERY LENGTH LIMIT EXCEEDED. MAX ALLOWED QUERY : 500 CHARS"}"#.to_string();
        let fake = serve(vec![body]);
        let err = engine(&fake.url)
            .translate(&TranslationRequest::new(
                "hello",
                LangPair::explicit(Language::En, Language::ZhHans),
            ))
            .await
            .expect_err("响应体里的 403 必须报失败");
        assert!(
            matches!(&err, ProviderError::Status(403, reason) if reason.contains("QUERY LENGTH")),
            "{err:?}"
        );
    }

    #[tokio::test]
    async fn empty_translation_is_a_failure_not_empty_success() {
        let fake = serve(vec![ok("   ")]);
        let err = engine(&fake.url)
            .translate(&TranslationRequest::new(
                "hi",
                LangPair::explicit(Language::En, Language::ZhHans),
            ))
            .await
            .unwrap_err();
        assert!(matches!(err, ProviderError::Empty), "{err:?}");
    }

    #[tokio::test]
    async fn unrecognized_status_shape_is_decode_failure() {
        let fake = serve(vec![
            r#"{"responseData":{"translatedText":"x"},"responseStatus":"OK"}"#.to_string(),
        ]);
        let err = engine(&fake.url)
            .translate(&TranslationRequest::new(
                "hi",
                LangPair::explicit(Language::En, Language::ZhHans),
            ))
            .await
            .unwrap_err();
        assert!(matches!(err, ProviderError::Decode(_)), "{err:?}");
    }

    #[tokio::test]
    async fn timeout_is_reported_with_retry_count() {
        let fake = serve_hanging(Duration::from_millis(400));
        let err = engine(&fake.url)
            .translate(&TranslationRequest::new(
                "hello",
                LangPair::explicit(Language::En, Language::ZhHans),
            ))
            .await
            .unwrap_err();
        assert!(
            matches!(&err, ProviderError::Timeout(ms, attempts) if *ms == 300 && *attempts == EXTRA_ATTEMPTS),
            "{err:?}"
        );
        assert!(err.retryable());
        // 首次 + 2 次重试
        assert_eq!(wait_for_requests(&fake, 3).len(), 3);
    }

    /// 端点要求两端语言不同（实测同语言对返回 HTTP 200 + `responseStatus: 403`
    /// + `PLEASE SELECT TWO DISTINCT LANGUAGES`）。
    ///
    /// 用连不上的地址来证明"根本没发请求"：真发出去了就是 `Http`，被拦下来才是 `SameLanguage`。
    #[tokio::test]
    async fn same_source_and_target_is_refused_before_any_request() {
        let err = engine(&closed_url())
            .translate(&TranslationRequest::new(
                "你好世界",
                LangPair::explicit(Language::ZhHans, Language::ZhHans),
            ))
            .await
            .unwrap_err();
        assert!(matches!(err, ProviderError::SameLanguage(_)), "{err:?}");
        let text = err.to_string();
        assert!(text.contains("相同"), "{text}");
        assert!(text.contains("简体中文"), "{text}");
        assert!(!text.contains("你好世界"), "错误里不得带源文: {text}");
        assert!(!err.retryable(), "得换目标语言，重试没有意义");
    }

    /// 真实触发路径：源文是中文、目标也是简体中文，源语言留在"自动检测"
    #[tokio::test]
    async fn auto_detected_source_matching_target_is_also_refused() {
        let err = engine(&closed_url())
            .translate(&TranslationRequest::new(
                "这是一段应当被检测为中文的文本",
                LangPair::auto_to(Language::ZhHans),
            ))
            .await
            .unwrap_err();
        assert!(matches!(err, ProviderError::SameLanguage(_)), "{err:?}");
    }

    #[tokio::test]
    async fn connection_refused_says_unreachable_not_rejected() {
        let err = engine(&closed_url())
            .translate(&TranslationRequest::new(
                "hello",
                LangPair::explicit(Language::En, Language::ZhHans),
            ))
            .await
            .unwrap_err();
        let text = err.to_string();
        assert!(text.contains("不可达"), "{text}");
        assert!(!text.contains("hello"), "错误里不得带源文: {text}");
    }

    #[tokio::test]
    async fn protected_content_survives_a_service_that_mangles_numbers() {
        // 假服务把占位符原样带回，同时改写了其它内容
        let fake = serve(vec![ok("看 [[0]] 与 [[1]] 的说明")]);
        let result = engine(&fake.url)
            .translate(&TranslationRequest::new(
                "看 https://a.example.com 与 12 MB 的说明",
                LangPair::explicit(Language::En, Language::ZhHans),
            ))
            .await
            .expect("应成功");
        assert_eq!(result.text, "看 https://a.example.com 与 12 MB 的说明");
    }
}
