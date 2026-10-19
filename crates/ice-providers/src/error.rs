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
