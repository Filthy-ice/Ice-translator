use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::error::CoreError;

/// 一条术语（FR-4.5）
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Term {
    pub source: String,
    pub target: String,
    /// 拉丁文术语默认做大小写不敏感匹配；中文等无需区分
    #[serde(default)]
    pub case_sensitive: bool,
}

impl Term {
    pub fn new(source: impl Into<String>, target: impl Into<String>) -> Self {
        Self {
            source: source.into(),
            target: target.into(),
            case_sensitive: false,
        }
    }

    fn same_text(&self, other: &str) -> bool {
        if self.case_sensitive {
            self.source == other
        } else {
            self.source.to_lowercase() == other.to_lowercase()
        }
    }
}

fn same_source(a: &Term, b: &Term) -> bool {
    a.same_text(&b.source)
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Glossary {
    #[serde(default)]
    pub terms: Vec<Term>,
}

impl Glossary {
    pub const fn max_prompt_terms() -> usize {
        60
    }

    pub fn path() -> Result<PathBuf, CoreError> {
        Ok(crate::config::Config::config_dir()?.join("glossary.toml"))
    }

    pub fn load_at(path: &Path) -> Result<Self, CoreError> {
        if !path.exists() {
            return Ok(Self::default());
        }
        let raw = fs::read_to_string(path)?;
        // 术语表损坏时宁可返回空表，也不阻塞启动
        toml::from_str(&raw).map_err(|e| CoreError::ConfigParse(format!("术语表解析失败：{e}")))
    }

    pub fn load() -> Result<Self, CoreError> {
        Self::load_at(&Self::path()?)
    }

    pub fn save_at(&self, path: &Path) -> Result<(), CoreError> {
        if let Some(dir) = path.parent() {
            fs::create_dir_all(dir)?;
        }
        let rendered =
            toml::to_string_pretty(self).map_err(|e| CoreError::ConfigParse(e.to_string()))?;
        fs::write(path, rendered)?;
        Ok(())
    }

    pub fn save(&self) -> Result<PathBuf, CoreError> {
        let path = Self::path()?;
        self.save_at(&path)?;
        Ok(path)
    }

    pub fn upsert(&mut self, term: Term) {
        self.terms.retain(|existing| !same_source(existing, &term));
        self.terms.push(term);
        self.terms.sort_by(|a, b| a.source.cmp(&b.source));
    }

    pub fn remove(&mut self, source: &str) -> bool {
        let before = self.terms.len();
        self.terms.retain(|t| !t.same_text(source));
        self.terms.len() != before
    }

    pub fn lookup(&self, source: &str) -> Option<&Term> {
        self.terms.iter().find(|t| t.same_text(source))
    }

    /// 机翻通道的译后替换：术语原文若在译文中残留，强制换成指定译法。
    pub fn post_substitute(&self, translated: &str) -> String {
        let mut out = translated.to_string();
        for term in &self.terms {
            if term.source.is_empty() || term.source == term.target {
                continue;
            }
            out = if term.case_sensitive {
                out.replace(&term.source, &term.target)
            } else {
                replace_case_insensitive(&out, &term.source, &term.target)
            };
        }
        out
    }

    /// AI 通道的提示词片段。超长的术语表截断到前 N 条，避免吃掉上下文预算。
    pub fn prompt_fragment(&self) -> String {
        if self.terms.is_empty() {
            return String::new();
        }
        let mut lines = String::from("术语表（必须严格使用右侧译法）：\n");
        for term in self.terms.iter().take(Self::max_prompt_terms()) {
            lines.push_str(&format!("{} => {}\n", term.source, term.target));
        }
        lines
    }
}

fn replace_case_insensitive(haystack: &str, needle: &str, replacement: &str) -> String {
    let needle_chars: Vec<char> = needle.chars().flat_map(|c| c.to_lowercase()).collect();
    if needle_chars.is_empty() {
        return haystack.to_string();
    }
    let source_chars: Vec<char> = haystack.chars().collect();
    let step = needle.chars().count();
    let mut out = String::with_capacity(haystack.len());
    let mut i = 0usize;
    while i < source_chars.len() {
        // 逐字符小写后比较；某个字符展开成多个小写字符时按不匹配处理，避免错位
        let window: Vec<char> = source_chars[i..]
            .iter()
            .take(needle_chars.len())
            .flat_map(|c| c.to_lowercase())
            .take(needle_chars.len())
            .collect();
        let matched = window.len() == needle_chars.len() && window == needle_chars;
        if matched {
            out.push_str(replacement);
            i += step;
        } else {
            out.push(source_chars[i]);
            i += 1;
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn post_substitute_replaces_residual_source_case_insensitively() {
        let mut g = Glossary::default();
        g.upsert(Term::new("Kubernetes", "K8s 集群"));
        // 机翻常把专有名词原样留下
        assert_eq!(
            g.post_substitute("在 kubernetes 上部署"),
            "在 K8s 集群 上部署"
        );
    }

    #[test]
    fn case_sensitive_term_only_matches_exact() {
        let mut g = Glossary::default();
        g.terms.push(Term {
            source: "IO".into(),
            target: "输入输出".into(),
            case_sensitive: true,
        });
        assert_eq!(g.post_substitute("io 与 IO"), "io 与 输入输出");
    }

    #[test]
    fn upsert_is_idempotent_and_remove_works() {
        let mut g = Glossary::default();
        g.upsert(Term::new("pod", "Pod"));
        g.upsert(Term::new("POD", "容器组"));
        assert_eq!(g.terms.len(), 1);
        assert!(g.remove("POD"));
        assert!(!g.remove("pod"));
        assert!(g.terms.is_empty());
    }

    #[test]
    fn prompt_fragment_empty_when_no_terms() {
        assert!(Glossary::default().prompt_fragment().is_empty());
        let mut g = Glossary::default();
        g.upsert(Term::new("latency", "时延"));
        let frag = g.prompt_fragment();
        assert!(frag.contains("latency => 时延"));
    }

    #[test]
    fn prompt_fragment_caps_at_limit() {
        let mut g = Glossary::default();
        for i in 0..100 {
            g.upsert(Term::new(format!("term{i}"), format!("词{i}")));
        }
        let frag = g.prompt_fragment();
        assert_eq!(
            frag.lines().filter(|l| l.contains("=>")).count(),
            Glossary::max_prompt_terms()
        );
    }

    #[test]
    fn substitute_handles_cjk_and_repeated_occurrences() {
        let mut g = Glossary::default();
        g.upsert(Term::new("微服务", "microservice"));
        let out = g.post_substitute("微服务架构中的微服务");
        assert_eq!(out, "microservice架构中的microservice");
    }

    #[test]
    fn roundtrip_through_toml() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("glossary.toml");
        let mut g = Glossary::default();
        g.upsert(Term::new("throughput", "吞吐量"));
        g.save_at(&path).unwrap();
        assert_eq!(Glossary::load_at(&path).unwrap(), g);
    }

    #[test]
    fn replace_case_insensitive_does_not_cut_multibyte() {
        // needle 与 haystack 长度不同时的边界情况
        assert_eq!(replace_case_insensitive("Ökonom y", "ökon", "X"), "Xom y");
    }
}
