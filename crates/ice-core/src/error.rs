use thiserror::Error;

#[derive(Debug, Error)]
pub enum CoreError {
    #[error("配置目录不可用: {0}")]
    ConfigDir(String),

    #[error("配置读写失败: {0}")]
    ConfigIo(#[from] std::io::Error),

    #[error("配置文件格式错误，已回退到默认配置: {0}")]
    ConfigParse(String),

    #[error("历史记录读写失败: {0}")]
    History(String),

    #[error("不支持的语言代码: {0}")]
    UnknownLanguage(String),

    #[error("图片数据无效: {0}")]
    InvalidImage(String),
}

pub type CoreResult<T> = Result<T, CoreError>;
