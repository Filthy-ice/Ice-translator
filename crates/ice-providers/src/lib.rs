//! Ice Translator 翻译引擎层。
//!
//! 网络与引擎实现在此，不依赖任何 UI crate（NFR-6）。

pub mod cache;
pub mod dispatcher;
pub mod engine;
pub mod error;
pub mod request;

pub use cache::{CacheKey, CachedValue, TranslationCache};
pub use dispatcher::Translator;
pub use engine::TranslationEngine;
pub use error::{ProviderError, ProviderResult};
pub use request::{TranslationRequest, TranslationResult};
