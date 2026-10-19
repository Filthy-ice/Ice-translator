//! Ice Translator 翻译引擎层。
//!
//! 网络与引擎实现在此，不依赖任何 UI crate（NFR-6）。

pub mod ai;
pub mod cache;
pub mod dispatcher;
pub mod engine;
pub mod error;
pub mod machine;
pub mod protect;
pub mod request;

#[cfg(test)]
mod fake_server;

pub use ai::AiEngine;
pub use cache::{CacheKey, CachedValue, TranslationCache};
pub use dispatcher::Translator;
pub use engine::TranslationEngine;
pub use error::{ProviderError, ProviderResult};
pub use machine::{MACHINE_QUERY_CHARS, MachineTranslateEngine};
pub use protect::{ProtectedText, Restored, chunk_chars, protect, restore};
pub use request::{TranslationRequest, TranslationResult};
