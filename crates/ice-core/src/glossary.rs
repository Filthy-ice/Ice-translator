use std::fs;
use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::config::Config;

/// 一条术语（FR-4.5）
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Term {
    pub source: String,
    pub target: String,
    /// 拉丁文术语默认做大小写不敏感匹配；中文等无需区分。
    /// 默认值不写进 config.toml，让 `[[term]]` 保持需求 §4.5 的两行形态。
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
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

/// P2 时期术语表单独存 `glossary.toml`（`[[terms]]`）。CHG-003 起术语并入 `config.toml`，
/// 这里只负责把老文件读进来，绝不改写或删除它——那是用户自己的数据。
const LEGACY_FILE: &str = "glossary.toml";

#[derive(Debug, Clone, Default, Deserialize)]
pub struct LegacyGlossary {
    #[serde(default)]
    pub terms: Vec<Term>,
}

/// 一次性导入的结果：条数 + 给界面显示的提示（损坏/缺失时为 `None` 或说明文本）
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LegacyImport {
    pub imported: usize,
    pub notice: Option<String>,
}

impl Glossary {
    pub const fn max_prompt_terms() -> usize {
        60
    }

    pub fn from_config(cfg: &Config) -> Self {
        Self {
            terms: cfg.terms.clone(),
        }
    }

    /// 写回配置前先去重排序，保证同一份术语表渲染出的 `config.toml` 是稳定的
    pub fn write_to_config(&self, cfg: &mut Config) {
        let mut normalized = self.clone();
        normalized.normalize();
        cfg.terms = normalized.terms;
    }

    /// 从旧 `glossary.toml` 只读导入。仅当配置里还没有术语时才导，因此重复启动不会重复导入。
    pub fn import_legacy(cfg: &mut Config, dir: &Path) -> LegacyImport {
        if !cfg.terms.is_empty() {
            return LegacyImport::default();
        }
        let path = dir.join(LEGACY_FILE);
        if !path.exists() {
            return LegacyImport::default();
        }
        let raw = match fs::read_to_string(&path) {
            Ok(raw) => raw,
            Err(e) => {
                return LegacyImport {
                    imported: 0,
                    notice: Some(format!("旧术语表读取失败，已按空表启动：{e}")),
                };
            }
        };
        match toml::from_str::<LegacyGlossary>(&raw) {
            Ok(legacy) if legacy.terms.is_empty() => LegacyImport::default(),
            Ok(legacy) => {
                let mut glossary = Self {
                    terms: legacy.terms,
                };
                glossary.normalize();
                let imported = glossary.terms.len();
                cfg.terms = glossary.terms;
                LegacyImport {
                    imported,
                    notice: Some(format!("已从 glossary.toml 导入 {imported} 条术语")),
                }
            }
            // 术语表损坏时宁可空表启动，也不阻塞翻译（与 FR-4.3 同一取向）
            Err(e) => LegacyImport {
                imported: 0,
                notice: Some(format!("旧术语表解析失败，已按空表启动：{e}")),
            },
        }
    }

    /// 按源词排序并去掉重复项（同一源词保留最后一次写入的译法）
    pub fn normalize(&mut self) {
        let mut seen: Vec<Term> = Vec::with_capacity(self.terms.len());
        for term in self.terms.drain(..) {
            if let Some(slot) = seen.iter_mut().find(|t| same_source(t, &term)) {
                *slot = term;
            } else {
                seen.push(term);
            }
        }
        seen.sort_by(|a, b| a.source.cmp(&b.source));
        self.terms = seen;
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
    fn terms_render_as_term_tables_inside_config() {
        let mut cfg = Config::default();
        Glossary {
            terms: vec![
                Term::new("throughput", "吞吐量"),
                Term {
                    source: "IO".into(),
                    target: "输入输出".into(),
                    case_sensitive: true,
                },
            ],
        }
        .write_to_config(&mut cfg);

        let rendered = toml::to_string_pretty(&cfg).unwrap();
        assert!(rendered.contains("[[term]]"), "{rendered}");
        assert!(!rendered.contains("[[terms]]"), "{rendered}");
        // 默认值不落盘，`[[term]]` 保持需求 §4.5 的两行形态
        assert_eq!(rendered.matches("case_sensitive").count(), 1, "{rendered}");

        let back: Config = toml::from_str(&rendered).unwrap();
        assert_eq!(back, cfg);
        assert_eq!(Glossary::from_config(&back).terms.len(), 2);
    }

    #[test]
    fn write_to_config_dedupes_and_sorts() {
        let mut cfg = Config::default();
        Glossary {
            terms: vec![
                Term::new("zeta", "泽塔"),
                Term::new("alpha", "阿尔法"),
                Term::new("ZETA", "末位"),
            ],
        }
        .write_to_config(&mut cfg);

        let sources: Vec<&str> = cfg.terms.iter().map(|t| t.source.as_str()).collect();
        // 排序沿用 P2 的字节序（大写在小写前），去重才是大小写不敏感的
        assert_eq!(sources, ["ZETA", "alpha"]);
        assert_eq!(cfg.terms[0].target, "末位");
    }

    #[test]
    fn import_legacy_reads_old_file_without_touching_it() {
        let dir = tempfile::tempdir().unwrap();
        let legacy = dir.path().join("glossary.toml");
        fs::write(
            &legacy,
            "[[terms]]\nsource = \"pod\"\ntarget = \"Pod\"\n\n[[terms]]\nsource = \"IO\"\ntarget = \"输入输出\"\ncase_sensitive = true\n",
        )
        .unwrap();
        let before = fs::read(&legacy).unwrap();

        let mut cfg = Config::default();
        let outcome = Glossary::import_legacy(&mut cfg, dir.path());

        assert_eq!(outcome.imported, 2);
        assert!(outcome.notice.unwrap().contains("导入 2 条"));
        assert_eq!(cfg.terms.len(), 2);
        assert_eq!(
            Glossary::from_config(&cfg).lookup("POD").unwrap().target,
            "Pod"
        );
        // 用户的旧文件一个字节都不动
        assert_eq!(fs::read(&legacy).unwrap(), before);
    }

    #[test]
    fn import_legacy_is_idempotent_once_config_has_terms() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(
            dir.path().join("glossary.toml"),
            "[[terms]]\nsource = \"pod\"\ntarget = \"Pod\"\n",
        )
        .unwrap();

        let mut cfg = Config::default();
        assert_eq!(Glossary::import_legacy(&mut cfg, dir.path()).imported, 1);
        // 第二次启动：config 里已有术语，不再导入、不产生重复
        let second = Glossary::import_legacy(&mut cfg, dir.path());
        assert_eq!(second, LegacyImport::default());
        assert_eq!(cfg.terms.len(), 1);
    }

    #[test]
    fn import_legacy_degrades_on_corrupt_or_missing_file() {
        let dir = tempfile::tempdir().unwrap();
        let mut cfg = Config::default();
        assert_eq!(
            Glossary::import_legacy(&mut cfg, dir.path()),
            LegacyImport::default()
        );

        fs::write(dir.path().join("glossary.toml"), "terms = ][ 不是 toml").unwrap();
        let outcome = Glossary::import_legacy(&mut cfg, dir.path());
        assert_eq!(outcome.imported, 0);
        assert!(outcome.notice.unwrap().contains("解析失败"));
        assert!(cfg.terms.is_empty());
    }

    #[test]
    fn replace_case_insensitive_does_not_cut_multibyte() {
        // needle 与 haystack 长度不同时的边界情况
        assert_eq!(replace_case_insensitive("Ökonom y", "ökon", "X"), "Xom y");
    }
}
