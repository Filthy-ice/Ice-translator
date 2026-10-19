use std::fs::OpenOptions;
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::error::CoreError;
use crate::language::{EngineKind, Language};

pub const HISTORY_CAPACITY: usize = 500;

/// 一条翻译历史（FR-4.4）
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct HistoryRecord {
    /// unix 秒
    pub ts: i64,
    pub engine: EngineKind,
    pub detected: Option<Language>,
    pub target: Language,
    pub source_text: String,
    pub result_text: String,
    pub elapsed_ms: u64,
}

impl HistoryRecord {
    pub fn preview(&self, chars: usize) -> String {
        self.source_text.chars().take(chars).collect::<String>()
    }
}

/// JSON Lines 存储，追加写，可被文本编辑器直接打开。
#[derive(Debug, Clone, Default)]
pub struct HistoryStore {
    path: PathBuf,
}

impl HistoryStore {
    pub fn default_path() -> Result<PathBuf, CoreError> {
        Ok(crate::config::Config::config_dir()?.join("history.jsonl"))
    }

    pub fn open_default() -> Result<Self, CoreError> {
        Ok(Self {
            path: Self::default_path()?,
        })
    }

    pub fn at(path: impl Into<PathBuf>) -> Self {
        Self { path: path.into() }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn append(&self, record: &HistoryRecord) -> Result<(), CoreError> {
        if let Some(dir) = self.path.parent() {
            std::fs::create_dir_all(dir).map_err(CoreError::ConfigIo)?;
        }
        let mut file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.path)
            .map_err(|e| CoreError::History(e.to_string()))?;
        let mut line =
            serde_json::to_string(record).map_err(|e| CoreError::History(e.to_string()))?;
        line.push('\n');
        file.write_all(line.as_bytes())
            .map_err(|e| CoreError::History(e.to_string()))?;
        if self.count() > HISTORY_CAPACITY {
            self.trim_to(HISTORY_CAPACITY)?;
        }
        Ok(())
    }

    /// 返回最近 n 条，按时间倒序（最新在前）。坏行跳过而不是整体失败（NFR-2）。
    pub fn recent(&self, n: usize) -> Result<Vec<HistoryRecord>, CoreError> {
        let all = self.read_all()?;
        let start = all.len().saturating_sub(n);
        Ok(all[start..].iter().rev().cloned().collect())
    }

    pub fn search(&self, query: &str, limit: usize) -> Result<Vec<HistoryRecord>, CoreError> {
        if query.trim().is_empty() {
            return self.recent(limit);
        }
        let needle = query.trim().to_lowercase();
        let hits: Vec<HistoryRecord> = self
            .read_all()?
            .into_iter()
            .rev()
            .filter(|r| {
                r.source_text.to_lowercase().contains(&needle)
                    || r.result_text.to_lowercase().contains(&needle)
            })
            .take(limit)
            .collect();
        Ok(hits)
    }

    pub fn clear(&self) -> Result<(), CoreError> {
        if self.path.exists() {
            std::fs::write(&self.path, "").map_err(|e| CoreError::History(e.to_string()))?;
        }
        Ok(())
    }

    fn read_all(&self) -> Result<Vec<HistoryRecord>, CoreError> {
        if !self.path.exists() {
            return Ok(Vec::new());
        }
        let file = OpenOptions::new()
            .read(true)
            .open(&self.path)
            .map_err(|e| CoreError::History(e.to_string()))?;
        let reader = BufReader::new(file);
        let mut out = Vec::new();
        for line in reader.lines() {
            let line = line.map_err(|e| CoreError::History(e.to_string()))?;
            if line.trim().is_empty() {
                continue;
            }
            match serde_json::from_str::<HistoryRecord>(&line) {
                Ok(record) => out.push(record),
                Err(e) => tracing::debug!("跳过损坏的历史行: {e}"),
            }
        }
        Ok(out)
    }

    fn count(&self) -> usize {
        self.read_all().map(|v| v.len()).unwrap_or(0)
    }

    fn trim_to(&self, capacity: usize) -> Result<(), CoreError> {
        let all = self.read_all()?;
        if all.len() <= capacity {
            return Ok(());
        }
        let kept = &all[all.len() - capacity..];
        let mut buf = String::new();
        for record in kept {
            buf.push_str(
                &serde_json::to_string(record).map_err(|e| CoreError::History(e.to_string()))?,
            );
            buf.push('\n');
        }
        // 与配置同样的写临时再改名策略
        let tmp = self.path.with_extension("jsonl.tmp");
        std::fs::write(&tmp, buf).map_err(|e| CoreError::History(e.to_string()))?;
        std::fs::rename(&tmp, &self.path).map_err(|e| CoreError::History(e.to_string()))?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn record(ts: i64, text: &str) -> HistoryRecord {
        HistoryRecord {
            ts,
            engine: EngineKind::Machine,
            detected: Some(Language::En),
            target: Language::ZhHans,
            source_text: text.into(),
            result_text: format!("译:{text}"),
            elapsed_ms: 120,
        }
    }

    #[test]
    fn append_and_recent_is_newest_first() {
        let dir = tempfile::tempdir().unwrap();
        let store = HistoryStore::at(dir.path().join("h.jsonl"));
        store.append(&record(1, "first")).unwrap();
        store.append(&record(2, "second")).unwrap();
        let recent = store.recent(10).unwrap();
        assert_eq!(recent.len(), 2);
        assert_eq!(recent[0].source_text, "second");
    }

    #[test]
    fn trims_to_capacity_keeping_newest() {
        let dir = tempfile::tempdir().unwrap();
        let store = HistoryStore::at(dir.path().join("h.jsonl"));
        for i in 0..(HISTORY_CAPACITY as i64 + 30) {
            store.append(&record(i, &format!("t{i}"))).unwrap();
        }
        let all = store.recent(HISTORY_CAPACITY + 10).unwrap();
        assert_eq!(all.len(), HISTORY_CAPACITY);
        assert_eq!(
            all[0].source_text,
            format!("t{}", HISTORY_CAPACITY as i64 + 29)
        );
    }

    #[test]
    fn search_matches_both_sides_case_insensitively() {
        let dir = tempfile::tempdir().unwrap();
        let store = HistoryStore::at(dir.path().join("h.jsonl"));
        store.append(&record(1, "Hello World")).unwrap();
        store.append(&record(2, "unrelated")).unwrap();
        let hits = store.search("world", 10).unwrap();
        assert_eq!(hits.len(), 1);
        let by_result = store.search("译:unrelated", 10).unwrap();
        assert_eq!(by_result.len(), 1);
        assert_eq!(by_result[0].source_text, "unrelated");
    }

    #[test]
    fn corrupt_line_is_skipped_not_fatal() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("h.jsonl");
        let store = HistoryStore::at(path.clone());
        store.append(&record(1, "ok")).unwrap();
        std::fs::OpenOptions::new()
            .append(true)
            .open(&path)
            .unwrap()
            .write_all(b"{not json}\n")
            .unwrap();
        store.append(&record(2, "also ok")).unwrap();
        assert_eq!(store.recent(10).unwrap().len(), 2);
    }

    #[test]
    fn clear_empties_history() {
        let dir = tempfile::tempdir().unwrap();
        let store = HistoryStore::at(dir.path().join("h.jsonl"));
        store.append(&record(1, "x")).unwrap();
        store.clear().unwrap();
        assert!(store.recent(10).unwrap().is_empty());
    }
}
