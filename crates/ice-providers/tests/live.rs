//! 真实第三方契约验证，默认忽略：只有 `cargo test -p ice-providers -- --ignored` 才发请求。
//!
//! 本地假服务能证明"解析、重试、错误映射按设计走"，证不了"MyMemory 与 DeepSeek 现在仍这样应答"。
//! 这两条测试就是对那两个真实契约的证据（CHG-002 验证层次表里的 `sandbox` 层）。
//! 内容都是本文件自带的示例文本，不含任何用户数据。

use std::time::Duration;

use ice_core::config::{AiConfig, AiProvider, Config, KEYCHAIN_ACCOUNT, KEYCHAIN_SERVICE};
use ice_core::language::{LangPair, Language};
use ice_providers::{AiEngine, MachineTranslateEngine, TranslationEngine, TranslationRequest};

const TIMEOUT: Duration = Duration::from_secs(15);

/// 机翻：短文本走一次真实 GET
#[tokio::test]
#[ignore = "会向第三方公开服务发真实请求，需显式 --ignored"]
async fn machine_live_short() {
    let engine = MachineTranslateEngine::from_config(&Config::default());
    let result = engine
        .translate(&TranslationRequest::new(
            "hello world",
            LangPair::explicit(Language::En, Language::ZhHans),
        ))
        .await
        .expect("真实机翻端点应可应答");
    println!(
        "机翻短文本：{:?} ({}ms, {}段)",
        result.text, result.elapsed_ms, result.segments
    );
    assert!(!result.text.trim().is_empty());
    assert_eq!(result.segments, 1);
}

/// 机翻：超过单次查询上限（`MACHINE_QUERY_CHARS` = 450）的文本必须靠切块成功，而不是收到响应体里的错误
#[tokio::test]
#[ignore = "会向第三方公开服务发真实请求，需显式 --ignored"]
async fn machine_live_long_text_is_chunked() {
    let engine = MachineTranslateEngine::from_config(&Config::default());
    let text = "This sentence is repeated to exceed the query limit. ".repeat(20);
    assert!(text.chars().count() > 700, "样例本身要先够长");
    let result = engine
        .translate(&TranslationRequest::new(
            text,
            LangPair::explicit(Language::En, Language::ZhHans),
        ))
        .await
        .expect("超长文本应被切成多块并拼回");
    println!(
        "机翻长文本：{} 段，译文 {} 字符 ({}ms)",
        result.segments,
        result.text.chars().count(),
        result.elapsed_ms
    );
    assert!(result.segments >= 2, "超上限文本必须分段");
    assert!(!result.text.contains("QUERY LENGTH"), "{}", result.text);
}

/// AI：DeepSeek 真实契约。Key 只从钥匙串取（ADR-005），本测试是 `ice-providers` 里
/// 唯一能碰到 keyring 的地方——它跑在 dev-dependency 下，生产代码不读钥匙串。
#[tokio::test]
#[ignore = "会向 AI 服务发真实请求，可能消耗账号额度，需显式 --ignored"]
async fn ai_live_deepseek() {
    let entry =
        keyring::Entry::new(KEYCHAIN_SERVICE, KEYCHAIN_ACCOUNT).expect("钥匙串条目应可寻址");
    let api_key = entry
        .get_password()
        .expect("钥匙串里没有 IceTranslator/ai-api-key；请先运行 security add-generic-password -a ai-api-key -s IceTranslator -w");

    let engine = AiEngine::new(
        AiConfig {
            provider: AiProvider::DeepSeek,
            key_configured: true,
            ..Default::default()
        },
        api_key,
        TIMEOUT,
        3500,
    );
    assert!(engine.is_available(), "预设 + Key 齐备时应可用");

    let result = engine
        .translate(&TranslationRequest::new(
            "This is a live contract check for the AI channel.",
            LangPair::explicit(Language::En, Language::ZhHans),
        ))
        .await
        .expect("DeepSeek 端点应返回译文");
    println!(
        "AI 译文：{} ({}ms, {}段)",
        result.text, result.elapsed_ms, result.segments
    );
    assert!(!result.text.trim().is_empty());

    // 不可译内容必须逐字节回来：这条同时验证 protect→模型→restore 的真实链路
    let protected = engine
        .translate(&TranslationRequest::new(
            "参考 https://a.example.com 与 12 MB 的说明，接口名 getUserInfo。",
            LangPair::auto_to(Language::En),
        ))
        .await
        .expect("带占位符的文本应能还原");
    println!("AI 保护内容：{}", protected.text);
    assert!(
        protected.text.contains("https://a.example.com"),
        "{}",
        protected.text
    );
    assert!(protected.text.contains("12 MB"), "{}", protected.text);
    assert!(protected.text.contains("getUserInfo"), "{}", protected.text);
}
