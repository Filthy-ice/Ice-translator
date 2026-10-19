use thiserror::Error;

#[derive(Debug, Error)]
pub enum VisionError {
    #[error("当前平台不支持原生 OCR，请使用文本模式或切换到 macOS")]
    UnsupportedPlatform,

    #[error("图片解码失败: {0}")]
    Decode(String),

    #[error("图片过大（{0} 像素），超出可处理上限 {1}")]
    TooLarge(u64, u64),

    #[error("OCR 调用失败: {0}")]
    Recognition(String),

    #[error("未发现任何文字")]
    NoTextFound,

    #[error("图像合成失败: {0}")]
    Composite(String),

    #[error("找不到可用字体，无法渲染: {0}")]
    Font(String),

    #[error("该功能尚未实现: {0}")]
    NotImplemented(&'static str),
}

pub type VisionResult<T> = Result<T, VisionError>;
