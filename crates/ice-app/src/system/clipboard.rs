//! 剪贴板（FR-1.5，REQ-003；图片那两条出入口属 CHG-005 REQ-001 与 CHG-006 REQ-008）。
//!
//! 组装内容与判据是纯函数，可单测；真正读写走 arboard，只在真机验证。
//! 四条通道各管一个方向：文本写（`write`）、文本读（`read`，服务于"剪贴板即翻"）、
//! 图片读（`read_image`，「粘贴」入口）与图片写（`write_image`，「复制图片」出口）。

use std::borrow::Cow;

use image::RgbaImage;

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
    let mut clipboard = arboard::Clipboard::new().map_err(unavailable)?;
    clipboard.set_text(text.to_string()).map_err(refused)
}

/// 「复制图片」那颗按钮的悬停提示与成功提示条各一句（Q-004）。
///
/// 两句都写在这一层，是因为它们说的是**同一个系统行为**：arboard 在 macOS 上的实现先
/// `clipboard.clear()` 再 `writeObjects`（`arboard-3.7.0/src/platform/osx.rs:347`），所以这一步
/// 必然抹掉剪贴板里原有的文本。这是所有 macOS App 的共性，不是本应用的缺陷；要做的只有一件
/// ——在用户点下去**之前**就说清（hover），点完再确认一次（提示条）。SHALL NOT 先读原文本、
/// 写完再偷偷还原：那会在用户没要求的时候两次改写剪贴板，还可能把几 MB 的旧图留在内存里。
pub const COPY_IMAGE_HOVER: &str = "复制图片会覆盖剪贴板里的文本（macOS 写图先清空剪贴板）";

/// 复制成功那条提示。按既有 `TOAST_LIFETIME`（1s）淡出、没有关闭按钮
pub const COPY_IMAGE_DONE: &str = "图片已复制；剪贴板里原来的文本已被替换";

/// 像素数与尺寸对不上时那句话。正常输入走不到这里（`RgbaImage` 自己保证这个等式），
/// 走到了就是缓冲在别处被截断过——那句话说的是"没法复制"，不带尺寸也不带任何字节。
pub const IMAGE_SIZE_TEXT: &str = "复制失败：这张图的像素数与它的尺寸对不上";

/// 把一张成品图写进剪贴板（REQ-008 的第二条出路）。
///
/// 交出去的是**裸 RGBA 像素**，由系统自己编成 TIFF/PNG：`arboard::set_image` 收的就是
/// `ImageData`，字节序按 `CGImageAlphaInfo::Last` = 直通 RGBA，与 `RgbaImage` 一致，
/// 所以这一路不经过 `ice_vision::export::encode`（那一层只服务落盘）。
///
/// 写之前不动剪贴板、写之后不还原（见 [`COPY_IMAGE_HOVER`]）。
pub fn write_image(image: &RgbaImage) -> Result<(), String> {
    let (width, height) = (image.width() as usize, image.height() as usize);
    let bytes = image.as_raw();
    // 与读那一侧同一条纪律：字节数对不上就不交出去。跨进外部 API 之前把这条等式断言一次，
    // 免得一段缺尾像素的缓冲被系统按声称的尺寸解释
    if !rgba_matches(width, height, bytes.len()) {
        return Err(IMAGE_SIZE_TEXT.to_string());
    }
    let mut clipboard = arboard::Clipboard::new().map_err(unavailable)?;
    clipboard
        .set_image(arboard::ImageData {
            width,
            height,
            bytes: Cow::Owned(bytes.clone()),
        })
        .map_err(refused)
}

/// 两句失败文案的唯一出处：拿不到剪贴板句柄、与拿到了但写不进去。文本与图片两条写入共用，
/// 免得"同一个故障两种说法"。转发的 `e` 是 arboard 的错误名（`ContentNotAvailable` 那一类），
/// 不含用户内容——真要打日志也只会有这一句（NFR-3）。
fn unavailable(error: impl std::fmt::Display) -> String {
    format!("剪贴板不可用：{error}")
}

fn refused(error: impl std::fmt::Display) -> String {
    format!("写入剪贴板失败：{error}")
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

/// 剪贴板里的一张图（CHG-005 REQ-001 的「粘贴」入口）。返回 `(RGBA 字节, 宽, 高)`。
///
/// arboard 在 macOS 上已经把剪贴板里的 TIFF/PNG 解成 **RGBA8 裸像素**
/// （`arboard-3.7.0/src/platform/osx.rs:224-247`，`bytes: rgba.into_raw()`），所以这里交出去的是
/// 像素缓冲而不是编码字节：调用方走 `ice_vision::import::admit_rgba`，不该再喂 `admit` 去解码。
///
/// 读不到（没有剪贴板句柄、里面根本不是图片、或字节数对不上 `宽×高×4`）一律 `None`。
/// 最后那条校验是boundary 检查：剪贴板是外部输入，字节与声称的尺寸不符时宁可当"没有图"，
/// 也不让一段缺尾像素的缓冲被当成完整图片送去识别。
pub fn read_image() -> Option<(Vec<u8>, usize, usize)> {
    let mut clipboard = arboard::Clipboard::new().ok()?;
    let image = clipboard.get_image().ok()?;
    validate_rgba(image.bytes.into_owned(), image.width, image.height)
}

/// 尺寸与字节数对得上才放行；`None` 表示这对数值不构成一张合法的 RGBA 图。
fn validate_rgba(bytes: Vec<u8>, width: usize, height: usize) -> Option<(Vec<u8>, usize, usize)> {
    rgba_matches(width, height, bytes.len()).then_some((bytes, width, height))
}

/// `宽 × 高 × 4` 等不等这一段字节的长度；任何一步溢出都算不等。
///
/// 读与写两侧共用这一条：读那一侧剪贴板是外部输入，写那一侧是我们要把缓冲交给一个会按
/// 声称尺寸解释它的外部 API。两侧的纪律是同一句——对不上就不交出去。
fn rgba_matches(width: usize, height: usize, len: usize) -> bool {
    width
        .checked_mul(height)
        .and_then(|pixels| pixels.checked_mul(4))
        == Some(len)
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

    /// 一张合法的 RGBA 缓冲：字节数必须正好是 宽×高×4，多了少了都不放行
    #[test]
    fn clipboard_pixels_are_accepted_only_when_the_count_matches() {
        let ok = validate_rgba(vec![0u8; 4 * 3 * 2], 3, 2).expect("3x2 RGBA = 24 字节");
        assert_eq!((ok.1, ok.2), (3, 2));
        assert_eq!(ok.0.len(), 24);
        assert!(
            validate_rgba(vec![0u8; 23], 3, 2).is_none(),
            "少一个字节就是坏数据"
        );
        assert!(
            validate_rgba(vec![0u8; 25], 3, 2).is_none(),
            "多出来的字节没有归属"
        );
        // 0 宽高是合法的空图，不是崩溃；0×0×4 = 0 字节
        assert!(validate_rgba(Vec::new(), 0, 0).is_some());
    }

    /// 写那一侧走的是同一条判据（`write_image` 里那道门）：字节数对不上就不交出去。
    /// 单独测这一层是因为 `RgbaImage` 自己保证那个等式，从它身上构造不出反例——
    /// 反例只能按数字造，而数字正是跨进 arboard 之前唯一还能拦住的东西。
    #[test]
    fn the_write_side_refuses_the_same_mismatched_buffers() {
        assert!(rgba_matches(3, 2, 24));
        assert!(!rgba_matches(3, 2, 23), "少一个字节就是坏数据");
        assert!(!rgba_matches(3, 2, 25), "多出来的字节没有归属");
        assert!(rgba_matches(0, 0, 0), "空图合法，不是崩溃");
        // 溢出那一档：`usize::MAX × 2` 绕回来是 0，不许因为"正好等于 len"而放行
        assert!(!rgba_matches(usize::MAX, 2, 0));

        let image = RgbaImage::new(3, 2);
        assert!(
            rgba_matches(
                image.width() as usize,
                image.height() as usize,
                image.as_raw().len()
            ),
            "一张真图的缓冲必须对得上，否则「复制图片」恒失败"
        );
    }

    /// Q-004 那两句：点之前（hover）与点之后（提示条）都得说到"文本被换掉"，而且不是同一句。
    /// 两句写在这一层，是因为它们讲的是同一个系统行为——界面与帧末各抄一份，
    /// 迟早有一份忘了提，用户就会在 Cmd+V 之后才发现刚才那段文字没了。
    #[test]
    fn both_copy_image_sentences_say_the_text_is_replaced() {
        assert!(
            COPY_IMAGE_HOVER.contains("覆盖剪贴板里的文本"),
            "点之前那句没把后果说在前头：{COPY_IMAGE_HOVER}"
        );
        assert!(
            COPY_IMAGE_DONE.contains("文本已被替换"),
            "点之后那句没确认这件事：{COPY_IMAGE_DONE}"
        );
        assert_ne!(
            COPY_IMAGE_HOVER, COPY_IMAGE_DONE,
            "两句一字不差，用户看不出哪句是提醒哪句是结果"
        );
        // Q-004 明写 SHALL NOT 备份还原，所以文案里不许出现这类承诺
        for text in [COPY_IMAGE_HOVER, COPY_IMAGE_DONE] {
            assert!(
                !text.contains("还原") && !text.contains("恢复"),
                "{text} 许了一个不做的承诺"
            );
        }
    }
}
