//! 源语言粗判，用于 FR-1.2 的"自动检测"与界面回显。
//!
//! 只做字符集统计，不做词表匹配：目标是"绝大多数情况判对 + 给出可信度"，
//! 而不是取代服务端检测。机翻通道自带检测结果时优先采用服务端返回值。
//!
//! **比较的是翻译单元（词）的数量，不是字符数量。**一个 `CrashLoopBackOff` 在屏幕上是
//! 16 个字母，在语义上是一个术语；一个汉字基本就是一个语素。按字符数比会让中文句子里的
//! 英文标识符成倍压过中文骨架（2026-10-28 复核实例见 `examples/detect_probe.rs`）。
//! "词"的切分仍然只看字符集（拉丁字母的连续段），不查任何词典，所以上面那句约束没变。

use crate::language::Language;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Detection {
    pub language: Language,
    /// 0.0~1.0。混合文本会明显低于单一字符集文本
    pub confidence: f32,
}

#[derive(Debug, Default, Clone, Copy)]
struct Tally {
    cjk: u32,
    kana: u32,
    hangul: u32,
    cyrillic: u32,
    arabic: u32,
    hebrew: u32,
    thai: u32,
    greek: u32,
    latin_extended: u32,
    /// 拉丁字母的连续段数量，含数字（`k8s`、`P99` 各算一个术语）
    latin_words: u32,
    traditional_cjk: u32,
}

/// 繁体专用字形样本（简化字中不出现的写法），用于区分 zh-Hans / zh-Hant。
/// 刻意不含 問/理/策 这类两岸同形的字。
const TRADITIONAL_SAMPLE: &str = "個們來時區書東車馬鳥語誰點裏後應開關長對話證訴處題緩經樣發說請實優會備選傳達運動務續內頁顏顯鐵";

/// 一个汉字承载的翻译单元数。中文平均词长 1.7 字，语素又多为单字，
/// 取 1.0 与"一个拉丁词"同价。
const HAN_UNITS: f32 = 1.0;
/// 一个拉丁词（连续字母段，夹数字也算一段）= 一个翻译单元。
const LATIN_WORD_UNITS: f32 = 1.0;
/// 假名是音拍文字，日语平均词长约 2.5 拍；谚文平均约 2 个音节成词。
const KANA_UNITS: f32 = 0.4;
const HANGUL_UNITS: f32 = 0.5;
/// 西里尔/阿拉伯/希伯来/泰/希腊按字母计，单词平均约 4~5 个字母。
/// 泰文没有词间空格，这一档是估的——它与拉丁混排时本就少见，不单独调。
const ALPHABETIC_UNITS: f32 = 0.25;

/// 连续的两个拉丁字母之间允许出现的字符（构成同一个术语）。
fn is_latin_letter(code: u32) -> bool {
    (0x41..=0x5A).contains(&code)
        || (0x61..=0x7A).contains(&code)
        || (0x00C0..=0x024F).contains(&code)
        || (0x1E00..=0x1EFF).contains(&code)
}

fn is_digit(code: u32) -> bool {
    (0x30..=0x39).contains(&code)
}

pub fn detect(text: &str) -> Option<Detection> {
    let mut t = Tally::default();
    let mut in_term = false;
    let mut term_has_letter = false;
    for ch in text.chars() {
        let code = ch as u32;
        if is_latin_letter(code) || is_digit(code) {
            // 术语切分：字母与数字连成一段（`k8s`、`P99`、`CrashLoopBackOff` 各一段），
            // 但只有含字母的那一段才算词——`300`、`2026` 不是。
            if !in_term {
                in_term = true;
                term_has_letter = false;
            }
            if is_latin_letter(code) {
                term_has_letter = true;
                if !(0x41..=0x5A).contains(&code) && !(0x61..=0x7A).contains(&code) {
                    t.latin_extended += 1;
                }
            }
        } else {
            if in_term {
                if term_has_letter {
                    t.latin_words += 1;
                }
                in_term = false;
            }
            if (0x4E00..=0x9FFF).contains(&code) || (0x3400..=0x4DBF).contains(&code) {
                t.cjk += 1;
                if TRADITIONAL_SAMPLE.contains(ch) {
                    t.traditional_cjk += 1;
                }
            } else if (0x3040..=0x30FF).contains(&code) || (0x31F0..=0x31FF).contains(&code) {
                t.kana += 1;
            } else if (0xAC00..=0xD7AF).contains(&code) || (0x1100..=0x11FF).contains(&code) {
                t.hangul += 1;
            } else if (0x0400..=0x04FF).contains(&code) {
                t.cyrillic += 1;
            } else if (0x0600..=0x06FF).contains(&code) {
                t.arabic += 1;
            } else if (0x0590..=0x05FF).contains(&code) {
                t.hebrew += 1;
            } else if (0x0E00..=0x0E7F).contains(&code) {
                t.thai += 1;
            } else if (0x0370..=0x03FF).contains(&code) {
                t.greek += 1;
            }
        }
    }
    if in_term && term_has_letter {
        t.latin_words += 1;
    }

    let han = t.cjk as f32 * HAN_UNITS;
    let latin = t.latin_words as f32 * LATIN_WORD_UNITS;
    let kana = t.kana as f32 * KANA_UNITS;
    let hangul = t.hangul as f32 * HANGUL_UNITS;
    let other = (t.cyrillic + t.arabic + t.hebrew + t.thai + t.greek) as f32 * ALPHABETIC_UNITS;
    let total_weight = han + latin + kana + hangul + other;
    if total_weight <= 0.0 {
        return None;
    }

    // 假名/谚文一旦出现即定性：日文必然混用汉字，韩文的谚文不会出现在别种语言里
    if t.kana > 0 {
        return Some(award(Language::Ja, kana + han, total_weight));
    }
    if t.hangul > 0 {
        return Some(award(Language::Ko, hangul, total_weight));
    }

    let ranked = [
        (Language::ZhHans, han),
        (Language::En, latin),
        (Language::Ru, t.cyrillic as f32 * ALPHABETIC_UNITS),
        (Language::Ar, t.arabic as f32 * ALPHABETIC_UNITS),
        (Language::Th, t.thai as f32 * ALPHABETIC_UNITS),
        (Language::El, t.greek as f32 * ALPHABETIC_UNITS),
    ];
    let (mut language, best) = ranked
        .into_iter()
        .filter(|(_, w)| *w > 0.0)
        .max_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal))?;

    // **中英词数相当时判中文**：英文技术术语被中文大量借用，反过来汉字几乎不会
    // 出现在英文句子里（引用一段中文报错除外）。所以平手归中文，拉丁要赢就得
    // **严格多过**汉字。只有一侧有量时不受这条影响。
    if language == Language::En && han > 0.0 && latin <= han {
        language = Language::ZhHans;
    }

    if language == Language::ZhHans {
        let ratio = t.traditional_cjk as f32 / (t.cjk.max(1)) as f32;
        if ratio > 0.15 {
            language = Language::ZhHant;
        }
        return Some(award(language, han, total_weight));
    }

    let mut detection = award(language, best, total_weight);
    // 拉丁字母带重音说明多半不是英语，但不臆断具体是哪种语言，改为压低置信度
    if language == Language::En && t.latin_extended > 0 {
        detection.confidence *= 0.75;
    }
    Some(detection)
}

fn award(language: Language, best_weight: f32, total_weight: f32) -> Detection {
    Detection {
        language,
        confidence: (best_weight / total_weight).clamp(0.0, 1.0),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_common_scripts() {
        let cases = [
            ("缓存失效策略需要按分片批处理", Language::ZhHans),
            ("The cache invalidation strategy", Language::En),
            ("お疲れ様でした、また明日", Language::Ja),
            ("안녕하세요 반갑습니다", Language::Ko),
            ("Шинный кэш устарел", Language::Ru),
            ("تخزين المؤقت", Language::Ar),
        ];
        for (text, expected) in cases {
            let d = detect(text).unwrap_or_else(|| panic!("应能检测: {text}"));
            assert_eq!(d.language, expected, "{text} 判成 {}", d.language);
            assert!(d.confidence > 0.6, "{text} 置信度过低: {}", d.confidence);
        }
    }

    #[test]
    fn digits_symbols_and_empty_are_not_guessed() {
        assert!(detect("2026-10-19 12:30:45").is_none());
        assert!(detect("....////----").is_none());
        assert!(detect("").is_none());
    }

    #[test]
    fn mixed_chinese_english_reads_as_chinese_with_lower_confidence() {
        let pure = detect("缓存失效策略需要按分片批处理才能生效").unwrap();
        let mixed = detect("缓存失效 cache invalidation 策略需要按分片批处理").unwrap();
        assert_eq!(mixed.language, Language::ZhHans);
        assert!(mixed.confidence < pure.confidence, "{mixed:?} vs {pure:?}");
    }

    #[test]
    fn mostly_english_with_one_cjk_term_reads_as_english() {
        let d = detect("flush 缓存 before running the benchmark again").unwrap();
        assert_eq!(d.language, Language::En);
    }

    #[test]
    fn japanese_with_kanji_still_reads_as_japanese() {
        let d = detect("メモリリークを調査します").unwrap();
        assert_eq!(d.language, Language::Ja);
    }

    #[test]
    fn traditional_chinese_separated_from_simplified() {
        let simp = detect("缓存失效策略需要更新").unwrap();
        let trad = detect("我們已經處理了這個問題，緩存失效策略需要更新").unwrap();
        assert_eq!(simp.language, Language::ZhHans);
        assert_eq!(trad.language, Language::ZhHant);
    }

    #[test]
    fn accented_latin_is_flagged_as_low_confidence() {
        let d = detect("L'invalidation du cache est nécessaire pour les objets").unwrap();
        assert_eq!(d.language, Language::En);
        assert!(d.confidence < 0.9, "重音文本不该报高置信: {}", d.confidence);
    }

    #[test]
    fn greek_wins_over_a_few_cjk_chars() {
        let d = detect("Αποθήκευση των δεδομένα").unwrap();
        assert_eq!(d.language, Language::El);
    }

    /// 2026-10-28 复核（TC-1.2-b）：真人会复制进输入框的混排实例，逐条锁定判定方向。
    /// 左侧标注的"期望"按**语法骨架**人工判定，不是按哪边字符多。
    #[test]
    fn mixed_instances_read_as_their_grammar_not_their_letter_count() {
        let cases = [
            (
                "请在 config.toml 里把 cache_ttl 改成 300 秒",
                Language::ZhHans,
            ),
            (
                "这个 K8s pod 一直 CrashLoopBackOff，node 也 NotReady",
                Language::ZhHans,
            ),
            (
                "麻烦确认下 SLA 里的 P99 是不是按自然月统计",
                Language::ZhHans,
            ),
            (
                "我看了下 PR 的 diff，有个 typo 在 config 里",
                Language::ZhHans,
            ),
            (
                "缓存失效 cache invalidation 策略需要按分片批处理",
                Language::ZhHans,
            ),
            (
                "flush 缓存 before running the benchmark again",
                Language::En,
            ),
            (
                "The error message says 连接超时 and nobody knows why",
                Language::En,
            ),
            ("please translate 这段话 into English", Language::En),
        ];
        for (text, expected) in cases {
            let d = detect(text).unwrap_or_else(|| panic!("应能检测: {text}"));
            assert_eq!(d.language, expected, "{text} 判成 {}", d.language);
        }
    }

    /// 平手归中文，拉丁要赢必须严格多过汉字——这条不对称是刻意的：
    /// 英文术语被中文借用是常态，汉字出现在英文句子里几乎是引用。
    #[test]
    fn a_tie_goes_to_chinese_and_english_needs_one_more_term_than_that() {
        let tie = detect("缓存 cache pool").unwrap();
        assert_eq!(tie.language, Language::ZhHans, "2 汉字 vs 2 词该判中文");
        let over = detect("缓存 cache pool now").unwrap();
        assert_eq!(over.language, Language::En, "3 词 > 2 汉字该判英文");
    }

    /// 术语按段数算，不按字母数算：一个超长标识符不等于十来个词。
    #[test]
    fn one_long_identifier_counts_as_one_term_however_many_letters_it_has() {
        let han_side = detect("这段 CrashLoopBackOff 还没修好").unwrap();
        assert_eq!(han_side.language, Language::ZhHans);
        assert!(
            han_side.confidence > 0.4,
            "一个术语不该把整句中文压到看不见: {}",
            han_side.confidence
        );
    }

    /// 数字不是词：纯参数仍然检不出，而不是判成英语。夹在中文里的 `p99` 算一个术语，
    /// 但它一个也压不过整句中文。
    #[test]
    fn digits_inside_a_term_are_not_a_word_on_their_own() {
        assert!(detect("300 60 1024").is_none());
        assert!(detect("2026-10-19 12:30:45").is_none());
        let mixed = detect("延迟 p99 是多少").unwrap();
        assert_eq!(mixed.language, Language::ZhHans);
    }
}
