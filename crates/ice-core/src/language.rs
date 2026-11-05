use serde::{Deserialize, Serialize};
use std::fmt;

/// 翻译引擎通道。FR-1.3
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum EngineKind {
    /// 传统机器翻译：零配置、快、不消耗 AI 额度
    Machine,
    /// 大模型翻译：质量高、需要用户自备服务参数
    Ai,
}

impl EngineKind {
    pub const fn all() -> [EngineKind; 2] {
        [EngineKind::Machine, EngineKind::Ai]
    }

    pub fn display_name(self) -> &'static str {
        match self {
            EngineKind::Machine => "机翻",
            EngineKind::Ai => "AI",
        }
    }
}

impl fmt::Display for EngineKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.display_name())
    }
}

/// 支持的语言。`code()` 为对外服务协议使用的语言码。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Language {
    ZhHans,
    ZhHant,
    En,
    Ja,
    Ko,
    Fr,
    De,
    Es,
    Pt,
    It,
    Ru,
    Vi,
    Th,
    Id,
    Ar,
    El,
}

impl Language {
    pub const fn code(self) -> &'static str {
        match self {
            Language::ZhHans => "zh-CN",
            Language::ZhHant => "zh-TW",
            Language::En => "en",
            Language::Ja => "ja",
            Language::Ko => "ko",
            Language::Fr => "fr",
            Language::De => "de",
            Language::Es => "es",
            Language::Pt => "pt",
            Language::It => "it",
            Language::Ru => "ru",
            Language::Vi => "vi",
            Language::Th => "th",
            Language::Id => "id",
            Language::Ar => "ar",
            Language::El => "el",
        }
    }

    pub const fn display_name(self) -> &'static str {
        match self {
            Language::ZhHans => "简体中文",
            Language::ZhHant => "繁体中文",
            Language::En => "英语",
            Language::Ja => "日语",
            Language::Ko => "韩语",
            Language::Fr => "法语",
            Language::De => "德语",
            Language::Es => "西班牙语",
            Language::Pt => "葡萄牙语",
            Language::It => "意大利语",
            Language::Ru => "俄语",
            Language::Vi => "越南语",
            Language::Th => "泰语",
            Language::Id => "印尼语",
            Language::Ar => "阿拉伯语",
            Language::El => "希腊语",
        }
    }

    pub fn from_code(code: &str) -> Option<Language> {
        let normalized = code.trim().replace('_', "-").to_lowercase();
        let aliased = match normalized.as_str() {
            "zh" | "zh-cn" | "zh-hans" | "zh-hans-cn" | "hans" | "cn" => Some(Language::ZhHans),
            "zh-tw" | "zh-hant" | "zh-hant-tw" | "hant" | "tw" => Some(Language::ZhHant),
            "jp" | "jpn" => Some(Language::Ja),
            "kr" | "kor" => Some(Language::Ko),
            _ => None,
        };
        if aliased.is_some() {
            return aliased;
        }
        let subtag = normalized.split('-').next().unwrap_or("");
        Language::all()
            .iter()
            .copied()
            .find(|l| l.code() == normalized || l.code().split('-').next() == Some(subtag))
    }

    pub const fn all() -> &'static [Language] {
        &[
            Language::ZhHans,
            Language::ZhHant,
            Language::En,
            Language::Ja,
            Language::Ko,
            Language::Fr,
            Language::De,
            Language::Es,
            Language::Pt,
            Language::It,
            Language::Ru,
            Language::Vi,
            Language::Th,
            Language::Id,
            Language::Ar,
            Language::El,
        ]
    }

    /// 界面上默认露出的语言（FR-1.4：中英日常用，其余在设置里启用）
    pub const fn preferred() -> &'static [Language] {
        &[Language::ZhHans, Language::En, Language::Ja]
    }

    /// 是否需要按字符断行（中日文可任意位置换行）
    pub const fn breaks_by_char(self) -> bool {
        matches!(self, Language::ZhHans | Language::ZhHant | Language::Ja)
    }

    /// 从区域文字估算的宽度系数：用于回填时的字号自适应（FR-2.7）。
    /// 拉丁语系同等信息量通常比中文长 1.5~2.2 倍。
    pub const fn width_factor(self) -> f32 {
        match self {
            Language::ZhHans | Language::ZhHant | Language::Ja => 0.55,
            Language::Ko => 0.8,
            Language::De | Language::Fr | Language::Es | Language::Pt | Language::It => 1.9,
            Language::Ru => 1.7,
            Language::En => 1.5,
            Language::Vi | Language::Id => 1.6,
            Language::Th => 1.2,
            Language::Ar => 1.1,
            Language::El => 1.4,
        }
    }
}

impl fmt::Display for Language {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.display_name())
    }
}

/// 语言对。`source == None` 表示自动检测（FR-1.2）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct LangPair {
    pub source: Option<Language>,
    pub target: Language,
}

impl LangPair {
    pub const fn auto_to(target: Language) -> Self {
        Self {
            source: None,
            target,
        }
    }

    pub const fn explicit(source: Language, target: Language) -> Self {
        Self {
            source: Some(source),
            target,
        }
    }
}

impl Default for LangPair {
    fn default() -> Self {
        Self::auto_to(Language::ZhHans)
    }
}

/// 汉字区（简繁同一区，扩展 A 另算）：`detect` 数 cjk 用的是它，断行判"逐字成行"也用它。
pub fn is_han(code: u32) -> bool {
    (0x4E00..=0x9FFF).contains(&code) || (0x3400..=0x4DBF).contains(&code)
}

/// 假名区（平假名/片假名 + 半角片假名）。
pub fn is_kana(code: u32) -> bool {
    (0x3040..=0x30FF).contains(&code) || (0x31F0..=0x31FF).contains(&code)
}

/// 谚文区（音节块 + 初声终声字母）。
pub fn is_hangul(code: u32) -> bool {
    (0xAC00..=0xD7AF).contains(&code) || (0x1100..=0x11FF).contains(&code)
}

/// 这个字符能不能在任意位置换行——`Language::breaks_by_char` 的**字符级孪生**。
///
/// 为什么需要孪生而不是直接用它：译文回填那条链（`ice-vision::typeset`）里没有任何
/// `Language` 可问——`compose` 不收语言参数，用户选的语向也推不出块内文字的真实书写系统
/// （中英混排的块常见）。断行只能逐字符判，而判据必须与"中日按字符"那条口径一致，
/// 所以两处共用这一条：汉字、假名逐字，谚文按词（谚文文本以空格分词，与拉丁同一条路）。
pub fn breaks_lines_per_char(ch: char) -> bool {
    let code = ch as u32;
    is_han(code) || is_kana(code)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn code_roundtrip() {
        for lang in Language::all() {
            assert_eq!(Language::from_code(lang.code()), Some(*lang), "{lang}");
        }
    }

    #[test]
    fn from_code_normalizes_case_and_underscore() {
        assert_eq!(Language::from_code("ZH_CN"), Some(Language::ZhHans));
        assert_eq!(Language::from_code(" zh-Hans "), Some(Language::ZhHans));
        assert_eq!(Language::from_code("en-GB"), Some(Language::En));
        assert_eq!(Language::from_code("klingon"), None);
    }

    #[test]
    fn preferred_is_subset_of_all() {
        for lang in Language::preferred() {
            assert!(Language::all().contains(lang));
        }
    }
}
