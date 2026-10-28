//! 真实第三方契约验证，默认忽略：只有 `cargo test -p ice-providers -- --ignored` 才发请求。
//!
//! 本地假服务能证明"解析、重试、错误映射按设计走"，证不了"MyMemory 与 DeepSeek 现在仍这样应答"。
//! 这些测试就是对那些真实契约的证据（CHG-002 验证层次表里的 `sandbox` 层）。
//! 内容都是本文件自带的示例文本，不含任何用户数据。

use std::time::Duration;

use ice_core::config::{AiConfig, AiProvider, Config, KEYCHAIN_ACCOUNT, KEYCHAIN_SERVICE};
use ice_core::language::{EngineKind, LangPair, Language};
use ice_providers::{
    AiEngine, MachineTranslateEngine, TranslationEngine, TranslationRequest, Translator,
};

const TIMEOUT: Duration = Duration::from_secs(15);

/// Key 只从钥匙串取（ADR-005）。本文件是 `ice-providers` 里唯一能碰到 keyring 的地方——
/// 它跑在 dev-dependency 下，生产代码不读钥匙串。
fn api_key() -> String {
    let entry =
        keyring::Entry::new(KEYCHAIN_SERVICE, KEYCHAIN_ACCOUNT).expect("钥匙串条目应可寻址");
    entry
        .get_password()
        .expect("钥匙串里没有 IceTranslator/ai-api-key；请先运行 security add-generic-password -a ai-api-key -s IceTranslator -w")
}

/// 用预设（地址与模型都留空取默认）构造真实 DeepSeek 通道，分段上限由调用方决定
fn deepseek(segment_chars: usize) -> AiEngine {
    AiEngine::new(
        AiConfig {
            provider: AiProvider::DeepSeek,
            key_configured: true,
            ..Default::default()
        },
        api_key(),
        TIMEOUT,
        segment_chars,
    )
}

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

/// AI：DeepSeek 真实契约。
#[tokio::test]
#[ignore = "会向 AI 服务发真实请求，可能消耗账号额度，需显式 --ignored"]
async fn ai_live_deepseek() {
    let engine = deepseek(3500);
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

/// NFR-1 的**正式判定**：机翻冷路径 P50 < 800ms（20 次采样）+ 热路径（缓存命中）< 5ms（10 次）。
///
/// P3 收口时只有单次样本（731ms），P5 又见过 2,133ms 的抖动，所以"NFR-1 达标"一直没被判定过，
/// 挂在 `07-acceptance.md` §P4 遗留 ⑤，采样脚本本身挂在 P8 的 TC-P-1。这条把两笔账一起结掉：
/// 同一句 200 字符英文连发 20 次（每次都真发请求，引擎层没有缓存），报出全部样本与 P50/P90/极差；
/// 再把同一句交给带缓存的 `Translator` 跑 11 次，第一次必然穿透、后 10 次必然命中，量的是命中那 10 次。
///
/// **每次冷采样之间停 `PACING_MS`**，这不是照顾本机的网络，而是照顾端点的配额：2026-10-28 实测
/// 不限速连发时，第 1 批 20/20 成功、第 2 批 16/20、第 3 批 0/20 全被 HTTP 429 拒掉。429 在
/// `error.rs` 里刻意**不自动重试**（立刻重试只会放大限流），所以脚本越急促，测到的越是配额而不是延迟。
/// 真实用户也不会一秒内点二十次翻译，加了停顿反而更贴近 NFR-1 想量的那个数字。
///
/// 抖动本身不判失败——免费公开端点的延迟不是本应用能控制的，写死 P99 会让这条测试变成每天随机红。
/// 真出现整批慢（成功率不足）时**判为"本次测不出"**而不是悄悄通过。
#[tokio::test]
#[ignore = "会向第三方公开服务发 20+11 次真实请求（带限速停顿），需显式 --ignored"]
async fn machine_latency_p50_over_twenty_samples() {
    const COLD_SAMPLES: usize = 20;
    const HOT_SAMPLES: usize = 10;
    const P50_BUDGET_MS: u64 = 800;
    const HOT_BUDGET_MS: u64 = 5;
    const MIN_SUCCESS: usize = 15;
    const PACING_MS: u64 = 1_500;

    let engine = MachineTranslateEngine::from_config(&Config::default());
    let text = "The caching layer invalidates entries by shard, and the strategy must survive a rolling restart. ".repeat(2);
    let chars = text.chars().count();
    assert!(
        (180..230).contains(&chars),
        "TC-P-1 规定采样用约 200 字符的一段，实测 {chars}"
    );
    let request = || {
        TranslationRequest::new(
            text.clone(),
            LangPair::explicit(Language::En, Language::ZhHans),
        )
    };

    let mut cold: Vec<u64> = Vec::with_capacity(COLD_SAMPLES);
    let mut failed = 0usize;
    let mut limited = 0usize;
    for i in 0..COLD_SAMPLES {
        if i > 0 {
            tokio::time::sleep(Duration::from_millis(PACING_MS)).await;
        }
        let result = engine.translate(&request()).await;
        match result {
            Ok(r) if !r.text.trim().is_empty() => cold.push(r.elapsed_ms),
            other => {
                failed += 1;
                let message = other
                    .err()
                    .map_or_else(|| String::from("空译文"), |e| e.to_string());
                if message.contains("429") {
                    limited += 1;
                }
                println!("冷启动第 {i} 次未取得译文：{message}");
            }
        }
    }

    if cold.len() < MIN_SUCCESS {
        panic!(
            "{COLD_SAMPLES} 次里只成功 {} 次（失败 {failed}，其中被限流 {limited} 次），端点这次不可靠，NFR-1 判定不成立",
            cold.len()
        );
    }
    cold.sort_unstable();
    let n = cold.len();
    let p50 = if n % 2 == 0 {
        (cold[n / 2 - 1] + cold[n / 2]) / 2
    } else {
        cold[n / 2]
    };
    let p90 = cold[((n as f64 * 0.9).ceil() as usize)
        .saturating_sub(1)
        .min(n - 1)];

    println!(
        "NFR-1 机翻冷路径（{chars} 字符，每次间隔 {PACING_MS}ms）：成功 {n}/{COLD_SAMPLES}（失败 {failed}，限流 {limited}）｜样本 {cold:?}｜P50 {p50}ms｜P90 {p90}ms｜最快 {}ms｜最慢 {}ms",
        cold[0],
        cold[n - 1]
    );
    assert!(
        p50 < P50_BUDGET_MS,
        "P50 {p50}ms 未达 NFR-1 的 <{P50_BUDGET_MS}ms（P90 {p90}ms、最慢 {}ms）",
        cold[n - 1]
    );

    let mut translator = Translator::new();
    translator.register(std::sync::Arc::new(engine));
    let mut hot: Vec<u64> = Vec::with_capacity(HOT_SAMPLES + 1);
    for i in 0..=HOT_SAMPLES {
        if i == 0 {
            // 只有会真发请求的那一次（第 0 次穿透）需要限速，后面全是本地命中、不打网络
            tokio::time::sleep(Duration::from_millis(PACING_MS)).await;
        }
        let started = std::time::Instant::now();
        let result = translator.translate(EngineKind::Machine, request()).await;
        let ms = started.elapsed().as_millis() as u64;
        if result.is_err() {
            panic!(
                "缓存通道第 {i} 次失败（{ms}ms）：{:?}——第 0 次是真实穿透，端点此时不可用则热路径判不出来",
                result.err()
            );
        }
        hot.push(ms);
    }
    let first_is_cold_miss = hot[0];
    let mut hit = hot[1..].to_vec();
    hit.sort_unstable();
    let hit_p50 = hit[hit.len() / 2];
    let hit_max = *hit.last().unwrap_or(&0);
    println!(
        "NFR-1 机翻译热路径（缓存命中 {HOT_SAMPLES} 次，已去掉第一次穿透的 {first_is_cold_miss}ms）：样本 {hit:?}｜P50 {hit_p50}ms｜最慢 {hit_max}ms"
    );
    assert!(
        hit_p50 < HOT_BUDGET_MS,
        "缓存命中 P50 {hit_p50}ms 未达 TC-P-1 的 <{HOT_BUDGET_MS}ms"
    );
}

/// AI 长文提速的真实证据（FR-1.9，2026-10-28 用户要求"并行翻译后拼接，别忘了接口中关闭思考"）。
///
/// 三段证据，都能在真实应答里读到：
/// 1. **关闭思考的字段被端点认**。带 `thinking: {"type":"disabled"}`（拼法见
///    `AiProvider::thinking_off`）的请求返回 200 且有译文——严格校验请求体的服务会直接 400，
///    字段拼错在这里就藏不住；
/// 2. **超长文本确实被切成多段**（把分段上限调到 300 字符逼出多段）；
/// 3. **多段确实同时在飞**：整篇耗时必须小于"段数 × 单段耗时"。串行实现是逐段相加再加开销，
///    必然越过这条线，所以这不是掐表猜的，是有判据的。
#[tokio::test]
#[ignore = "会向 AI 服务发真实请求，可能消耗账号额度，需显式 --ignored"]
async fn ai_live_long_text_is_split_and_parallel() {
    let engine = deepseek(300);
    let sentence = "The quick brown fox jumps over the lazy dog near the riverbank. ";
    let long_text = sentence.repeat(12); // 约 860 字符，300 上限下必然多段
    assert!(
        long_text.chars().count() > 700,
        "样例本身要先够长，否则测不到分段"
    );

    let single = engine
        .translate(&TranslationRequest::new(
            sentence.trim_end(),
            LangPair::explicit(Language::En, Language::ZhHans),
        ))
        .await
        .expect("单段请求应成功");
    let long = engine
        .translate(&TranslationRequest::new(
            long_text.clone(),
            LangPair::explicit(Language::En, Language::ZhHans),
        ))
        .await
        .expect("超长文本应分段成功");

    println!(
        "AI 单段：{}ms；AI 长文：{} 段、{}ms（串行上界 {}ms）",
        single.elapsed_ms,
        long.segments,
        long.elapsed_ms,
        single.elapsed_ms.saturating_mul(long.segments as u64)
    );
    assert!(long.segments >= 2, "超上限文本必须分段，实际 {long:?}");
    assert!(!long.text.trim().is_empty());
    assert!(
        long.elapsed_ms < single.elapsed_ms * long.segments as u64,
        "{} 段用了 {}ms，而单段耗时 {}ms——并行没生效",
        long.segments,
        long.elapsed_ms,
        single.elapsed_ms
    );
}
