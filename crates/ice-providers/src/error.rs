use thiserror::Error;

#[derive(Debug, Error)]
pub enum ProviderError {
    #[error("引擎尚未实现: {0}")]
    NotImplemented(&'static str),

    #[error("网络请求失败: {0}")]
    Http(String),

    #[error("请求超时（{0}ms），已重试 {1} 次")]
    Timeout(u64, u32),

    #[error("服务返回错误 {0}: {1}")]
    Status(u16, String),

    #[error("API Key 无效或被拒绝（HTTP 401）")]
    Unauthorized,

    #[error("被限流（HTTP 429），请稍后再试")]
    RateLimited,

    #[error("AI 服务未配置：请在设置中填写 base_url、模型名，并保存 API Key")]
    NotConfigured,

    #[error("响应格式无法解析: {0}")]
    Decode(String),

    #[error("返回了空译文")]
    Empty,

    #[error("输入为空")]
    EmptyInput,

    /// 机翻端点要求两端语言不同：同语言对必然被拒（实测 HTTP 200，体内 `responseStatus: 403`，
    /// 详情是英文的 `PLEASE SELECT TWO DISTINCT LANGUAGES`）。与其把那句英文抛给用户，
    /// 不如在发请求前就地判定——结果一样，但省一次往返且说人话。
    #[error("源语言与目标语言相同（都是{0}），不需要翻译；请换一个目标语言")]
    SameLanguage(&'static str),
}

pub type ProviderResult<T> = Result<T, ProviderError>;

impl ProviderError {
    /// 界面据此决定"可重试"还是"要改配置"
    pub fn retryable(&self) -> bool {
        matches!(
            self,
            ProviderError::Timeout(..) | ProviderError::RateLimited | ProviderError::Http(_)
        )
    }
}

/// 引擎内部自动重试的判据：**只有网络类失败**才重试。
///
/// 与 `retryable()` 的区别是有意为之：429 时界面上可以放"再试一次"按钮，
/// 但自动立刻重试只会放大限流；401 与解析失败重试也不会改变结果。
pub fn should_retry(err: &ProviderError) -> bool {
    matches!(err, ProviderError::Timeout(..) | ProviderError::Http(_))
}

/// reqwest 的错误文本里会带上完整请求 URL，而我们的查询串里就是用户的原文。
/// 错误会进日志也会显示在界面上，因此把 `?` 之后的查询段整段剥掉（NFR-3）。
fn scrub_queries(msg: &str) -> String {
    let mut out = String::with_capacity(msg.len());
    let mut chars = msg.chars().peekable();
    while let Some(c) = chars.next() {
        if c != '?' {
            out.push(c);
            continue;
        }
        while let Some(&next) = chars.peek() {
            if next == ' ' || next == ')' || next == '"' {
                break;
            }
            chars.next();
        }
    }
    out
}

/// 把 reqwest 的错误分门别类（FR-1.7）。连接失败必须说成"连不上"，
/// 不能与"服务端拒绝"混为一谈，否则用户会去查 Key 而问题在网络。
/// `retries` 是本次调用之前已经重试的次数。
pub fn from_reqwest(
    err: &reqwest::Error,
    timeout: std::time::Duration,
    retries: u32,
) -> ProviderError {
    if err.is_timeout() {
        return ProviderError::Timeout(timeout.as_millis() as u64, retries);
    }
    if err.is_connect() {
        // 只取主机名：不把 URL 的查询串（用户原文）写进错误
        let host = err
            .url()
            .and_then(|u| u.host_str())
            .unwrap_or("服务端")
            .to_string();
        return ProviderError::Http(format!(
            "无法连接 {host}：网络不可达或被中间网络设备拦截，不是服务端拒绝"
        ));
    }
    ProviderError::Http(scrub_queries(&err.to_string()))
}

/// HTTP 状态码映射；原因文本截断，避免把整个响应体塞进错误里
pub fn from_status(status: u16, body: &str) -> ProviderError {
    match status {
        401 => ProviderError::Unauthorized,
        429 => ProviderError::RateLimited,
        other => {
            let clipped: String = body.chars().take(200).collect();
            ProviderError::Status(other, clipped)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn query_string_is_scrubbed_from_error_text() {
        let raw = "error sending request for url (https://x.example/get?q=%E4%BD%A0%E5%A5%BD&langpair=en%7Czh)";
        let clean = scrub_queries(raw);
        assert!(!clean.contains("q="), "{clean}");
        assert!(clean.contains("x.example"), "{clean}");
    }

    #[test]
    fn status_codes_map_to_distinct_readable_errors() {
        assert!(matches!(from_status(401, ""), ProviderError::Unauthorized));
        assert!(matches!(from_status(429, ""), ProviderError::RateLimited));
        assert!(
            matches!(&from_status(403, "forbidden"), ProviderError::Status(403, t) if t.as_str() == "forbidden"),
            "403 不能冒充 401"
        );
    }

    #[test]
    fn long_error_bodies_are_clipped() {
        let body = "x".repeat(5000);
        let err = from_status(500, &body);
        let text = err.to_string();
        assert!(text.chars().count() < 400, "{text}");
    }
}
