//! 剪贴板（FR-1.5，REQ-003）。
//!
//! 组装内容是纯函数，可单测；真正读写走 arboard，只在真机验证。

/// 两个按钮各对应一种写入内容
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CopyKind {
    /// 只写译文——这同时就是"一键把剪贴板替换为译文"
    Translation,
    /// 原文段、空行、译文段
    Bilingual,
}

impl CopyKind {
    pub const fn label(self) -> &'static str {
        match self {
            CopyKind::Translation => "复制译文",
            CopyKind::Bilingual => "复制对照",
        }
    }
}

/// 没有可复制内容时的统一提示
pub const EMPTY_TRANSLATION_HINT: &str = "没有可复制的译文";

/// 组装要写入剪贴板的文本。译文为空（含纯空白）时返回 `None`，调用方据此拒绝触碰剪贴板。
pub fn compose(kind: CopyKind, source: &str, translation: &str) -> Option<String> {
    let translation = translation.trim_end_matches(['\n', '\r']);
    if translation.trim().is_empty() {
        return None;
    }
    match kind {
        CopyKind::Translation => Some(translation.to_string()),
        // 原文里的换行原样保留，两段之间固定一个空行
        CopyKind::Bilingual => Some(format!(
            "{}\n\n{}",
            source.trim_end_matches(['\n', '\r']),
            translation
        )),
    }
}

pub fn write(text: &str) -> Result<(), String> {
    let mut clipboard = arboard::Clipboard::new().map_err(|e| format!("剪贴板不可用：{e}"))?;
    clipboard
        .set_text(text.to_string())
        .map_err(|e| format!("写入剪贴板失败：{e}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn translation_copy_writes_only_the_translation() {
        let out = compose(CopyKind::Translation, "hello world", "你好，世界").unwrap();
        assert_eq!(out, "你好，世界");
    }

    #[test]
    fn bilingual_copy_keeps_source_newlines_and_fixed_separator() {
        let out = compose(CopyKind::Bilingual, "line one\nline two", "第一行\n第二行").unwrap();
        assert_eq!(out, "line one\nline two\n\n第一行\n第二行");
    }

    #[test]
    fn empty_translation_never_touches_clipboard() {
        for translation in ["", "   ", "\n\n", "\t"] {
            assert_eq!(compose(CopyKind::Translation, "source", translation), None);
            assert_eq!(compose(CopyKind::Bilingual, "source", translation), None);
        }
    }

    #[test]
    fn trailing_newlines_from_the_provider_are_trimmed() {
        let out = compose(CopyKind::Translation, "a", "b\n\n").unwrap();
        assert_eq!(out, "b");
    }

    #[test]
    fn composing_is_idempotent() {
        let first = compose(CopyKind::Bilingual, "s", "t");
        assert_eq!(first, compose(CopyKind::Bilingual, "s", "t"));
    }
}
