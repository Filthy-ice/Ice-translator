//! 不可译内容保护（FR-1.6）与占位符安全的长度切块。
//!
//! 请求前把 URL、邮箱、代码块、标识符、带单位数字与日期替换成 `[[n]]` 占位符，
//! 收到译文后按占位符原样还原。保护、还原与切块都是纯函数，两条通道共用；
//! 切块放在这里是因为它不得把占位符从中间劈开。

use std::sync::OnceLock;

use regex::Regex;

/// 按优先级排列的保护规则：靠前的规则先占位，其后规则不得切开已保护区间。
/// 最后一条把原文里本就存在的 `[[3]]` 也保护起来，避免与我们的编号碰撞。
///
/// 标识符只认"看得出是标识符"的写法（驼峰多段、下划线连接），
/// 单个首字母大写的普通英文词（Kubernetes）不保护：它既会误伤句首单词，
/// 机翻本来也几乎不会改动这类专有名词。
const PATTERNS: &[&str] = &[
    r"(?s)```.+?```",
    r"`[^`\n]+`",
    r#"[A-Za-z][A-Za-z0-9+.-]*://[^\s<>"']+"#,
    r"[A-Za-z0-9._%+-]+@[A-Za-z0-9.-]+\.[A-Za-z]{2,}",
    r"\b\d{4}-\d{2}-\d{2}([ T]\d{2}:\d{2}(:\d{2})?)?\b",
    r"\b\d+(?:[.,]\d+)?\s?(?:MB|GB|TB|KB|ms|s|min|h|d|px|pt|km|kg|kcal|%|元|美元|欧元)\b",
    r"\b[A-Z][a-z0-9]+(?:[A-Z][a-z0-9]+)+\b",
    r"\b[a-z][a-z0-9]*(?:[A-Z][a-z0-9]*)+\b",
    r"\b[a-z0-9]+(?:_[a-z0-9]+)+\b",
    r"\[\[\d+\]\]",
];

const SENTENCE_ENDS: [char; 7] = ['。', '！', '？', '；', '.', '?', ';'];

fn regexes() -> &'static [Regex] {
    static RE: OnceLock<Vec<Regex>> = OnceLock::new();
    RE.get_or_init(|| {
        PATTERNS
            .iter()
            .map(|p| Regex::new(p).expect("保护规则必须是合法正则"))
            .collect()
    })
}

fn placeholder_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"\[\[(\d+)\]\]").expect("占位符规则必须是合法正则"))
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct ProtectedText {
    /// 含 `[[n]]` 占位符、待提交的文本
    pub template: String,
    /// 下标即占位符编号，值为需要原样保留的原文片段
    pub originals: Vec<String>,
}

impl ProtectedText {
    pub fn is_empty(&self) -> bool {
        self.originals.is_empty()
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Restored {
    pub text: String,
    /// 译文中丢失的占位符编号；上层据此判定"没能逐字节还原"
    pub lost: Vec<usize>,
}

/// 收集命中区间（重叠时先命中的规则胜出），按偏移升序拼出模板。
pub fn protect(text: &str) -> ProtectedText {
    let mut spans: Vec<(usize, usize)> = Vec::new();
    for re in regexes() {
        for m in re.find_iter(text) {
            let (s, e) = (m.start(), m.end());
            if spans.iter().any(|(ps, pe)| s < *pe && e > *ps) {
                continue;
            }
            spans.push((s, e));
        }
    }
    spans.sort_by_key(|(s, _)| *s);

    let mut template = String::with_capacity(text.len());
    let mut originals = Vec::new();
    let mut cursor = 0;
    for (s, e) in spans {
        template.push_str(&text[cursor..s]);
        template.push_str(&format!("[[{}]]", originals.len()));
        originals.push(text[s..e].to_string());
        cursor = e;
    }
    template.push_str(&text[cursor..]);
    ProtectedText {
        template,
        originals,
    }
}

/// 按占位符字面匹配还原，不依赖位置顺序，因此模型调整句序也能还原。
/// 占位符在译文中消失时，把对应原文追加到末尾并登记 `lost`，
/// 供上层报失败，而不是静默返回残缺文本。
pub fn restore(translated: &str, originals: &[String]) -> Restored {
    let mut text = String::with_capacity(translated.len());
    let mut seen = vec![false; originals.len()];
    let mut last = 0;
    for cap in placeholder_re().captures_iter(translated) {
        let whole = cap.get(0).expect("整条匹配必然存在");
        text.push_str(&translated[last..whole.start()]);
        last = whole.end();
        let index = cap[1].parse::<usize>().ok();
        match index.and_then(|n| originals.get(n)) {
            Some(original) => {
                text.push_str(original);
                seen[index.expect("上一步已解析成功")] = true;
            }
            // 编号不在本次文本的表里：原样留着，交给上层判断
            None => text.push_str(whole.as_str()),
        }
    }
    text.push_str(&translated[last..]);

    let lost: Vec<usize> = seen
        .iter()
        .enumerate()
        .filter(|(_, found)| !**found)
        .map(|(i, _)| i)
        .collect();
    for i in &lost {
        text.push('\n');
        text.push_str(&originals[*i]);
    }
    Restored { text, lost }
}

fn char_len(s: &str) -> usize {
    s.chars().count()
}

/// 第 `max` 个字符对应的字节下标（`chars().count()` 语义，不是字节数）。
fn byte_at_char(s: &str, max: usize) -> usize {
    s.char_indices().nth(max).map(|(i, _)| i).unwrap_or(s.len())
}

/// 在 `text[..hard]` 内找最晚的自然断点，返回其后一位作为切点。
/// 断点必须是字符边界：'。' 占 3 字节，`i + 1` 会把切点落进字符内部。
fn natural_cut(text: &str, hard: usize) -> Option<usize> {
    let head = &text[..hard];
    if let Some(i) = head.rfind('\n') {
        return Some(i + '\n'.len_utf8());
    }
    SENTENCE_ENDS
        .iter()
        .filter_map(|p| head.rfind(*p).map(|i| (i, *p)))
        .map(|(i, p)| i + p.len_utf8())
        .max()
}

/// 切点若落在占位符内部，先尽量后退到占位符起点；退到 0 时改为吃到占位符结束。
fn avoid_splitting_placeholder(text: &str, cut: usize) -> usize {
    let Some(open) = text[..cut].rfind("[[") else {
        return cut;
    };
    if text[open..cut].contains("]]") {
        return cut;
    }
    let closed = text[open..].find("]]").map(|i| open + i + 2);
    if open == 0 {
        closed.unwrap_or(cut)
    } else {
        open
    }
}

/// 选一个切点：优先自然断点，但断点太靠前会切出几乎没有内容的碎块——
/// 多花一次请求，还打断上下文，所以宁可硬切到上限。
fn choose_cut(rest: &str, hard: usize) -> usize {
    let natural = natural_cut(rest, hard).unwrap_or(hard);
    let candidate = if natural * 2 >= hard { natural } else { hard };
    let cut = avoid_splitting_placeholder(rest, candidate);
    if cut == 0 || cut >= rest.len() {
        hard
    } else {
        cut
    }
}

/// 按字符数切块，优先在换行与句末断开，且绝不劈开 `[[n]]` 占位符。
/// 单块内容本身超长（如整段代码块已被占位符替换，一般不会发生）时按长度硬切，保证能推进。
pub fn chunk_chars(text: &str, max: usize) -> Vec<String> {
    let max = max.max(1);
    let mut out = Vec::new();
    let mut rest = text;
    while !rest.is_empty() {
        if char_len(rest) <= max {
            out.push(rest.to_string());
            break;
        }
        let cut = choose_cut(rest, byte_at_char(rest, max));
        out.push(rest[..cut].to_string());
        rest = &rest[cut..];
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn table(text: &str) -> Vec<String> {
        protect(text).originals
    }

    #[test]
    fn protects_the_declared_kinds_of_untranslatable_content() {
        let src = "看 https://example.com/a?b=1 与 me@example.com，x = 12 MB，日期 2026-10-19，类名 MyTranslateEngine，方法 getUserInfo，字段 user_id";
        let found = table(src);
        assert!(found.iter().any(|s| s.starts_with("https://")), "{found:?}");
        assert!(found.iter().any(|s| s.contains('@')), "{found:?}");
        assert!(found.iter().any(|s| s == "12 MB"), "{found:?}");
        assert!(found.iter().any(|s| s == "2026-10-19"), "{found:?}");
        assert!(found.iter().any(|s| s == "MyTranslateEngine"), "{found:?}");
        assert!(found.iter().any(|s| s == "getUserInfo"), "{found:?}");
        assert!(found.iter().any(|s| s == "user_id"), "{found:?}");
        // 保护过的文本原样回传时必须逐字节等于原文
        let p = protect(src);
        assert_eq!(restore(&p.template, &p.originals).text, src);
    }

    #[test]
    fn fenced_code_block_is_protected_as_a_whole() {
        let src = "例子：```rust\nlet url = \"https://inside.example\";\n```完毕";
        let p = protect(src);
        assert_eq!(p.originals.len(), 1, "{:?}", p.originals);
        assert!(p.originals[0].contains("```rust"));
        assert!(!p.template.contains("https://inside.example"));
    }

    #[test]
    fn identity_roundtrip_keeps_every_byte() {
        let src = "访问 https://a.example.com 或 mail@x.io，容量 512 GB，类名 MyTranslateEngine，版本 v2 于 2026-10-19 发布";
        let p = protect(src);
        // 用原文里的片段冒充"译文"，逐字节还原应回到原样
        let fake = p.template.clone();
        let back = restore(&fake, &p.originals);
        assert_eq!(back.text, src);
        assert!(back.lost.is_empty());
    }

    #[test]
    fn same_identifier_is_restored_everywhere() {
        let src = "MyTranslateEngine 与 MyTranslateEngine 集群";
        let p = protect(src);
        assert_eq!(p.originals.len(), 2);
        let back = restore(&p.template, &p.originals);
        assert_eq!(back.text, src);
    }

    #[test]
    fn ordinary_capitalized_words_are_not_protected() {
        // 保护规则不该把句首普通英文词也锁死，否则英文原文会被切成一堆占位符
        let p = protect("The Engine is running");
        assert!(p.originals.is_empty(), "{:?}", p.originals);
        assert!(!p.template.contains("[["), "{}", p.template);
    }

    #[test]
    fn plain_text_gets_no_placeholder() {
        let src = "今天天气不错，我们出去走走吧";
        let p = protect(src);
        assert_eq!(p.template, src);
        assert!(p.is_empty());
    }

    #[test]
    fn missing_placeholder_is_reported_as_lost() {
        let p = protect("参考 https://a.example.com 与 12 MB");
        assert_eq!(p.template, "参考 [[0]] 与 [[1]]");
        // 模型把第一个占位符吞了：必须报丢失，并把原文补回末尾而不是静默丢弃
        let back = restore("参见 [[1]]", &p.originals);
        assert_eq!(back.lost, vec![0], "丢失的占位符需报出来");
        assert!(
            back.text.contains("12 MB"),
            "在位内容应正常还原: {}",
            back.text
        );
        assert!(
            back.text.contains("https://a.example.com"),
            "丢失内容不得凭空消失: {}",
            back.text
        );
    }

    #[test]
    fn reordered_placeholders_still_restore() {
        let p = protect("先 12 MB 再 https://a.example.com");
        let back = restore("顺序调整：[[1]]、[[0]]", &p.originals);
        assert!(back.lost.is_empty());
        assert!(
            back.text.contains("https://a.example.com、12 MB"),
            "{}",
            back.text
        );
    }

    #[test]
    fn existing_bracket_placeholder_in_source_is_escaped() {
        let src = "文档里的 [[0]] 不是我们的占位符";
        let p = protect(src);
        assert_eq!(p.originals, vec!["[[0]]".to_string()]);
        assert_eq!(restore(&p.template, &p.originals).text, src);
    }

    #[test]
    fn chunks_respect_char_limit_and_order() {
        let src = "句子。".repeat(200); // 800 字符
        let chunks = chunk_chars(&src, 450);
        assert!(chunks.len() >= 2);
        assert!(chunks.iter().all(|c| char_len(c) <= 450));
        assert_eq!(chunks.concat(), src, "切块必须无损且保序");
    }

    #[test]
    fn chunk_never_splits_a_placeholder() {
        // 长前缀把占位符推到切点附近
        let src = format!("{}[[0]]尾巴", "a".repeat(448));
        let p = protect(&src);
        let chunks = chunk_chars(&p.template, 450);
        for c in &chunks {
            assert!(!c.ends_with('['), "切点落在占位符内部: {c:?}");
        }
        let joined = chunks.concat();
        assert_eq!(restore(&joined, &p.originals).text, src);
    }

    #[test]
    fn oversized_code_block_never_reaches_the_chunk_limit() {
        let src = format!("前言```text\n{}\n```后记", "x".repeat(2000));
        let p = protect(&src);
        assert!(char_len(&p.template) < 100, "整段代码块应被单个占位符替掉");
        assert_eq!(chunk_chars(&p.template, 450).len(), 1);
        assert_eq!(restore(&p.template, &p.originals).text, src);
    }

    #[test]
    fn no_near_empty_chunks() {
        // 换行紧贴开头时不能切出 "x\n" 这种碎块：白花一次请求
        let src = format!("x\n{}", "字".repeat(500));
        let chunks = chunk_chars(&src, 100);
        let sizes: Vec<usize> = chunks.iter().map(|c| char_len(c)).collect();
        assert!(
            sizes[..sizes.len() - 1].iter().all(|n| *n == 100),
            "除末尾余量外每段都应吃满额度: {sizes:?}"
        );
        assert!(
            chunks.iter().all(|c| !c.trim().is_empty()),
            "不得出现纯空白碎块: {chunks:?}"
        );
        assert_eq!(chunks.concat(), src);
    }

    #[test]
    fn prefer_line_break_over_hard_cut() {
        let src = "短行\n".repeat(120); // 每行 4 个字符
        let chunks = chunk_chars(&src, 450);
        assert!(chunks.iter().all(|c| c.ends_with('\n')));
        assert_eq!(chunks.concat(), src);
    }
}
