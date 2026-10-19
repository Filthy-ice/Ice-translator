//! 源语言粗判，用于 FR-1.2 的"自动检测"与界面回显。
//!
//! 只做字符集统计，不做词表匹配：目标是"绝大多数情况判对 + 给出可信度"，
//! 而不是取代服务端检测。机翻通道自带检测结果时优先采用服务端返回值。

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
    latin: u32,
    latin_extended: u32,
    traditional_cjk: u32,
}

/// 繁体专用字形样本（简化字中不出现的写法），用于区分 zh-Hans / zh-Hant。
/// 刻意不含 問/理/策 这类两岸同形的字。
const TRADITIONAL_SAMPLE: &str = "個們來時區書東車馬鳥語誰點裏後應開關長對話證訴處題緩經樣發說請實優會備選傳達運動務續內頁顏顯鐵";

/// 一个字符的"信息权重"：一个汉字约等于两三个拉丁字母承载的内容，
/// 不加权会让中英混排的句子被字母数量压过。
const CJK_WEIGHT: f32 = 2.0;
const LATIN_WEIGHT: f32 = 1.0;
const SYLLABARY_WEIGHT: f32 = 1.15;

pub fn detect(text: &str) -> Option<Detection> {
    let mut t = Tally::default();
    for ch in text.chars() {
        let code = ch as u32;
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
        } else if ch.is_ascii_alphabetic() {
            t.latin += 1;
        } else if (0x00C0..=0x024F).contains(&code) {
            t.latin += 1;
            t.latin_extended += 1;
        }
    }

    let total_weight = t.cjk as f32 * CJK_WEIGHT
        + t.kana as f32 * CJK_WEIGHT
        + t.hangul as f32 * CJK_WEIGHT
        + t.latin as f32 * LATIN_WEIGHT
        + (t.cyrillic + t.arabic + t.hebrew + t.thai + t.greek) as f32 * SYLLABARY_WEIGHT;
    if total_weight <= 0.0 {
        return None;
    }

    // 假名/谚文一旦出现即定性：日文必然混用汉字，韩文的谚文不会出现在别种语言里
    if t.kana > 0 {
        return Some(award(
            Language::Ja,
            (t.kana + t.cjk) as f32 * CJK_WEIGHT,
            total_weight,
        ));
    }
    if t.hangul > 0 {
        return Some(award(
            Language::Ko,
            t.hangul as f32 * CJK_WEIGHT,
            total_weight,
        ));
    }

    let ranked = [
        (Language::ZhHans, t.cjk as f32 * CJK_WEIGHT),
        (Language::En, t.latin as f32 * LATIN_WEIGHT),
        (Language::Ru, t.cyrillic as f32 * SYLLABARY_WEIGHT),
        (Language::Ar, t.arabic as f32 * SYLLABARY_WEIGHT),
        (Language::Th, t.thai as f32 * SYLLABARY_WEIGHT),
        (Language::El, t.greek as f32 * SYLLABARY_WEIGHT),
    ];
    let (mut language, best) = ranked
        .into_iter()
        .filter(|(_, w)| *w > 0.0)
        .max_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal))?;

    if language == Language::ZhHans {
        let ratio = t.traditional_cjk as f32 / (t.cjk.max(1)) as f32;
        if ratio > 0.15 {
            language = Language::ZhHant;
        }
        return Some(award(language, best, total_weight));
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
}
