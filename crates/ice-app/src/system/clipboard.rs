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

/// 剪贴板轮询间隔（REQ-008）。arboard 3.7 只有 `get_text()`，没有 changeCount 之类的
/// 版本接口，所以只能定时读全文比对；典型文本下是亚毫秒级，700ms 让空闲开销压到不可见。
pub const CLIPBOARD_POLL: std::time::Duration = std::time::Duration::from_millis(700);

/// 读一次剪贴板文本。读不到（没有剪贴板句柄、或里面根本不是文本，例如图片）返回 `None`，
/// 由 `Watcher::observe` 按"什么都不做"处理（REQ-008 第三条）。
pub fn read() -> Option<String> {
    let mut clipboard = arboard::Clipboard::new().ok()?;
    clipboard.get_text().ok()
}

/// "剪贴板即翻"的判定器（FR-3.8）。
///
/// 只有状态和判据是纯的：`observe` 收"这次读到了什么"和"输入行有没有焦点"，返回该不该填。
/// 真正的 `read()` 留在外面，由 `app.rs` 的帧循环调用，这样五种故障模式都能无头单测。
#[derive(Debug, Default)]
pub struct Watcher {
    /// 上一次看到的文本，用来判"内容变了没有"
    last: Option<String>,
    /// 最近一次由本应用写出去的文本，用来判"这次变化是不是自己干的"
    ours: Option<String>,
}

impl Watcher {
    /// 一次轮询的判定：返回 `Some(text)` 表示"该填进输入行并提交"。
    ///
    /// 六条判据按故障模式排序，每一条都对应一种真实会发生的错（见 `CHG-004/design.md`
    /// REQ-008）：读失败、空文本、内容没变、是自己写的、用户正在输入框里打字。
    pub fn observe(&mut self, text: Option<String>, input_focused: bool) -> Option<String> {
        let text = text?;
        if text.trim().is_empty() {
            return None;
        }
        if self.last.as_deref() == Some(text.as_str()) {
            return None;
        }
        if self.ours.as_deref() == Some(text.as_str()) {
            // 自己刚复制出去的译文/对照：当成"见过"，不当成用户的新输入
            self.last = Some(text);
            return None;
        }
        // 焦点占用时同样要更新 `last`：不更新的话，用户一失焦就会突然被灌进一段
        // 早就躺在剪贴板里的旧内容（REQ-008 第四条的"等到不再持有焦点才填"不等于补填旧账）
        self.last = Some(text.clone());
        if input_focused {
            return None;
        }
        let (len, hash8) = super::secrets::fingerprint(&text);
        // 只记长度与哈希前缀，剪贴板原文一个字符都不进日志（NFR-3 的同一口径）
        tracing::debug!(
            clip_len = len,
            clip_hash8 = hash8,
            "剪贴板新内容将自动填入并提交"
        );
        Some(text)
    }

    /// 记下"这是应用自己刚写出去的"，下一次轮询看到它不会当成外部新内容
    pub fn note_written(&mut self, text: &str) {
        self.ours = Some(text.to_string());
    }
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

    /// REQ-008 的七条判据，逐条锁一个真实故障模式
    #[test]
    fn the_watcher_fills_only_on_external_new_text() {
        let mut w = Watcher::default();

        // 1 读失败（或不是文本）：静默
        assert_eq!(w.observe(None, false), None);
        // 2 空/纯空白
        assert_eq!(w.observe(Some("   ".into()), false), None);
        // 3 第一次出现的新文本 → 填
        assert_eq!(
            w.observe(Some("hello".into()), false).as_deref(),
            Some("hello")
        );
        // 4 同一段再来一次：内容没变
        assert_eq!(
            w.observe(Some("hello".into()), false),
            None,
            "重复轮询不能重复翻译"
        );
        // 5 应用自己写的
        w.note_written("你好");
        assert_eq!(
            w.observe(Some("你好".into()), false),
            None,
            "复制译文不能立刻拿译文再翻一次"
        );
        // 6 用户正在输入框里打字：不填，但内容要记为"见过"
        assert_eq!(w.observe(Some("world".into()), true), None);
        // 7 上一条的延续：失焦之后也**不补填**那段旧内容
        assert_eq!(
            w.observe(Some("world".into()), false),
            None,
            "焦点释放后补灌旧内容会把用户刚打的字顶掉"
        );
        // 失焦之后真正出现的新文本才填
        assert_eq!(
            w.observe(Some("new text".into()), false).as_deref(),
            Some("new text")
        );
    }

    /// 自己写的记录只保一次：用户随后在别处复制了同一段文字时不该再吞掉，
    /// 但判据是"内容比对"而不是"次数"——所以换一段就照常填
    #[test]
    fn the_watcher_compares_content_not_how_many_times_it_has_fired() {
        let mut w = Watcher::default();
        w.note_written("译文 A");
        assert_eq!(
            w.observe(Some("译文 B".into()), false).as_deref(),
            Some("译文 B")
        );
        assert_eq!(
            w.observe(Some("译文 A".into()), false),
            None,
            "仍是自己写的那一段"
        );
    }
}
