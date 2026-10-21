//! Ice Translator 领域层。
//!
//! 本 crate 只放数据模型与纯逻辑，禁止依赖 UI、网络、OCR crate（NFR-6）。

pub mod config;
pub mod detect;
pub mod document;
pub mod error;
pub mod glossary;
pub mod history;
pub mod language;

pub use config::Config;
pub use detect::{Detection, detect};
pub use document::{OcrDocument, PixelPoint, PixelRect, TextBlock};
pub use error::CoreError;
pub use glossary::{Glossary, LegacyImport, Term};
pub use history::{HistoryRecord, HistoryStore};
pub use language::{EngineKind, LangPair, Language};
