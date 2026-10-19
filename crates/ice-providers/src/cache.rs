use std::num::NonZeroUsize;
use std::sync::Mutex;

use ice_core::language::{EngineKind, Language};
use lru::LruCache;

/// 缓存键包含引擎与语言对：换引擎或换语言都必须重新翻译（FR-1.8）
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct CacheKey {
    pub engine: EngineKind,
    pub source: String,
    pub target: String,
    pub text: String,
    /// AI 通道用术语表版本号加盐：改术语后旧结果不再复用
    pub salt: u64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct CachedValue {
    pub text: String,
    pub detected: Option<Language>,
}

#[derive(Debug)]
pub struct TranslationCache {
    inner: Mutex<LruCache<CacheKey, CachedValue>>,
    hits: Mutex<usize>,
    misses: Mutex<usize>,
}

impl TranslationCache {
    pub fn with_capacity(items: usize) -> Self {
        let capacity = NonZeroUsize::new(items.max(1)).expect("capacity >= 1");
        Self {
            inner: Mutex::new(LruCache::new(capacity)),
            hits: Mutex::new(0),
            misses: Mutex::new(0),
        }
    }

    pub fn get(&self, key: &CacheKey) -> Option<CachedValue> {
        let mut guard = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        let found = guard.get(key).cloned();
        match found {
            Some(value) => {
                *self.hits.lock().unwrap_or_else(|e| e.into_inner()) += 1;
                Some(value)
            }
            None => {
                *self.misses.lock().unwrap_or_else(|e| e.into_inner()) += 1;
                None
            }
        }
    }

    pub fn insert(&self, key: CacheKey, value: CachedValue) {
        self.inner
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .put(key, value);
    }

    pub fn len(&self) -> usize {
        self.inner.lock().unwrap_or_else(|e| e.into_inner()).len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// 命中率，用于 NFR-1 的性能走查
    pub fn hit_rate(&self) -> f32 {
        let hits = *self.hits.lock().unwrap_or_else(|e| e.into_inner());
        let misses = *self.misses.lock().unwrap_or_else(|e| e.into_inner());
        let total = hits + misses;
        if total == 0 {
            0.0
        } else {
            hits as f32 / total as f32
        }
    }

    pub fn clear(&self) {
        self.inner.lock().unwrap_or_else(|e| e.into_inner()).clear();
    }
}

impl Default for TranslationCache {
    fn default() -> Self {
        Self::with_capacity(512)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(text: &str, engine: EngineKind) -> CacheKey {
        CacheKey {
            engine,
            source: "auto".into(),
            target: "zh-CN".into(),
            text: text.into(),
            salt: 0,
        }
    }

    #[test]
    fn hit_and_miss_counted() {
        let cache = TranslationCache::with_capacity(4);
        cache.insert(
            key("a", EngineKind::Machine),
            CachedValue {
                text: "甲".into(),
                detected: None,
            },
        );
        assert!(cache.get(&key("a", EngineKind::Machine)).is_some());
        assert!(cache.get(&key("b", EngineKind::Machine)).is_none());
        assert_eq!(cache.hit_rate(), 0.5);
    }

    #[test]
    fn different_engine_is_different_entry() {
        let cache = TranslationCache::with_capacity(4);
        cache.insert(
            key("a", EngineKind::Machine),
            CachedValue {
                text: "机翻".into(),
                detected: None,
            },
        );
        assert!(cache.get(&key("a", EngineKind::Ai)).is_none());
    }

    #[test]
    fn evicts_least_recently_used() {
        let cache = TranslationCache::with_capacity(2);
        cache.insert(
            key("a", EngineKind::Machine),
            CachedValue {
                text: "1".into(),
                detected: None,
            },
        );
        cache.insert(
            key("b", EngineKind::Machine),
            CachedValue {
                text: "2".into(),
                detected: None,
            },
        );
        // 访问 a 使 b 成为最久未使用
        cache.get(&key("a", EngineKind::Machine));
        cache.insert(
            key("c", EngineKind::Machine),
            CachedValue {
                text: "3".into(),
                detected: None,
            },
        );
        assert!(cache.get(&key("b", EngineKind::Machine)).is_none());
        assert!(cache.get(&key("a", EngineKind::Machine)).is_some());
        assert_eq!(cache.len(), 2);
    }
}
