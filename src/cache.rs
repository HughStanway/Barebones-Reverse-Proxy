use hyper::body::Bytes;
use hyper::{HeaderMap, StatusCode};
use lru::LruCache;
use std::num::NonZeroUsize;
use std::sync::RwLock;
use std::time::{Duration, Instant};

#[derive(Clone, Debug)]
pub struct CachedAsset {
    pub status: StatusCode,
    pub headers: HeaderMap,
    pub body: Bytes,
    pub created_at: Instant,
    pub ttl: Duration,
    pub size_bytes: usize,
}

impl CachedAsset {
    pub fn is_expired(&self) -> bool {
        self.created_at.elapsed() >= self.ttl
    }
}

pub struct LruCacheEngine {
    inner: RwLock<LruCache<String, CachedAsset>>,
    max_capacity_bytes: usize,
    max_file_size_bytes: usize,
    default_ttl: Duration,
    current_memory_bytes: RwLock<usize>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InsertResult {
    pub inserted: bool,
    pub bytes_inserted: usize,
    pub bytes_evicted: usize,
    pub items_evicted: usize,
    pub reason: Option<&'static str>,
}

impl LruCacheEngine {
    pub fn new(
        max_capacity_bytes: usize,
        max_file_size_bytes: usize,
        default_ttl_sec: u64,
    ) -> Self {
        let cap = NonZeroUsize::new(10_000).unwrap();
        LruCacheEngine {
            inner: RwLock::new(LruCache::new(cap)),
            max_capacity_bytes,
            max_file_size_bytes,
            default_ttl: Duration::from_secs(default_ttl_sec),
            current_memory_bytes: RwLock::new(0),
        }
    }

    pub fn get(&self, key: &str) -> Option<CachedAsset> {
        let mut guard = self.inner.write().ok()?;
        if let Some(item) = guard.get_mut(key) {
            if item.is_expired() {
                let size = item.size_bytes;
                guard.pop(key);
                if let Ok(mut mem) = self.current_memory_bytes.write() {
                    *mem = mem.saturating_sub(size);
                }
                None
            } else {
                Some(item.clone())
            }
        } else {
            None
        }
    }

    pub fn insert(
        &self,
        key: String,
        status: StatusCode,
        headers: HeaderMap,
        body: Bytes,
        ttl_override: Option<Duration>,
    ) -> InsertResult {
        let size_bytes = body.len();
        if size_bytes > self.max_file_size_bytes {
            return InsertResult {
                inserted: false,
                bytes_inserted: 0,
                bytes_evicted: 0,
                items_evicted: 0,
                reason: Some("exceeds_max_file_size"),
            };
        }

        if size_bytes > self.max_capacity_bytes {
            return InsertResult {
                inserted: false,
                bytes_inserted: 0,
                bytes_evicted: 0,
                items_evicted: 0,
                reason: Some("exceeds_max_capacity"),
            };
        }

        let ttl = ttl_override.unwrap_or(self.default_ttl);
        let asset = CachedAsset {
            status,
            headers,
            body,
            created_at: Instant::now(),
            ttl,
            size_bytes,
        };

        let mut guard = match self.inner.write() {
            Ok(g) => g,
            Err(_) => {
                return InsertResult {
                    inserted: false,
                    bytes_inserted: 0,
                    bytes_evicted: 0,
                    items_evicted: 0,
                    reason: Some("lock_error"),
                };
            }
        };
        let mut mem_guard = match self.current_memory_bytes.write() {
            Ok(g) => g,
            Err(_) => {
                return InsertResult {
                    inserted: false,
                    bytes_inserted: 0,
                    bytes_evicted: 0,
                    items_evicted: 0,
                    reason: Some("lock_error"),
                };
            }
        };

        let mut bytes_evicted = 0;
        let mut items_evicted = 0;

        while *mem_guard + size_bytes > self.max_capacity_bytes && !guard.is_empty() {
            if let Some((evicted_key, evicted_item)) = guard.pop_lru() {
                *mem_guard = mem_guard.saturating_sub(evicted_item.size_bytes);
                bytes_evicted += evicted_item.size_bytes;
                items_evicted += 1;
                crate::log_debug!(
                    "cache_eviction",
                    "evicted_key" => evicted_key,
                    "evicted_bytes" => evicted_item.size_bytes,
                    "current_memory_bytes" => *mem_guard
                );
            } else {
                break;
            }
        }

        if *mem_guard + size_bytes <= self.max_capacity_bytes {
            if let Some(old) = guard.put(key, asset) {
                *mem_guard = mem_guard.saturating_sub(old.size_bytes);
            }
            *mem_guard += size_bytes;
            InsertResult {
                inserted: true,
                bytes_inserted: size_bytes,
                bytes_evicted,
                items_evicted,
                reason: None,
            }
        } else {
            InsertResult {
                inserted: false,
                bytes_inserted: 0,
                bytes_evicted,
                items_evicted,
                reason: Some("capacity_full_after_eviction"),
            }
        }
    }

    pub fn current_memory_bytes(&self) -> usize {
        self.current_memory_bytes.read().map(|m| *m).unwrap_or(0)
    }

    pub fn entry_count(&self) -> usize {
        self.inner.read().map(|g| g.len()).unwrap_or(0)
    }

    pub fn max_capacity_bytes(&self) -> usize {
        self.max_capacity_bytes
    }

    pub fn max_file_size_bytes(&self) -> usize {
        self.max_file_size_bytes
    }
}

pub fn is_uncacheable_entrypoint(path: &str) -> bool {
    let lower_path = path.to_lowercase();
    lower_path.ends_with("sw.js")
        || lower_path.contains("service-worker")
        || lower_path.contains("registersw")
        || lower_path.contains("workbox")
        || lower_path.ends_with(".webmanifest")
        || lower_path.ends_with(".html")
        || lower_path == "/"
}

pub fn is_static_asset(path: &str, content_type: Option<&str>) -> bool {
    // NEVER cache Service Workers, PWA manifests, or HTML entrypoints as static assets!
    if is_uncacheable_entrypoint(path) {
        return false;
    }

    let lower_path = path.to_lowercase();
    if lower_path.ends_with(".css")
        || lower_path.ends_with(".js")
        || lower_path.ends_with(".png")
        || lower_path.ends_with(".jpg")
        || lower_path.ends_with(".jpeg")
        || lower_path.ends_with(".svg")
        || lower_path.ends_with(".ico")
        || lower_path.ends_with(".woff2")
        || lower_path.ends_with(".woff")
        || lower_path.ends_with(".ttf")
        || lower_path.ends_with(".webp")
        || lower_path.ends_with(".json")
        || lower_path.ends_with(".wasm")
    {
        return true;
    }

    if let Some(ct) = content_type {
        let ct_lower = ct.to_lowercase();
        if ct_lower.contains("text/html") {
            return false;
        }
        if ct_lower.contains("text/css")
            || ct_lower.contains("application/javascript")
            || ct_lower.contains("text/javascript")
            || ct_lower.contains("image/")
            || ct_lower.contains("font/")
            || ct_lower.contains("application/wasm")
        {
            return true;
        }
    }

    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_is_static_asset_service_worker_exclusions() {
        assert!(is_static_asset("/assets/app.css", None));
        assert!(is_static_asset("/assets/bundle-123.js", None));
        assert!(is_static_asset("/logo.png", None));

        // Exclusions: Service workers, PWA manifests, and HTML entry points
        assert!(!is_static_asset("/sw.js", None));
        assert!(!is_static_asset("/registerSW.js", None));
        assert!(!is_static_asset("/workbox-9c191d2f.js", None));
        assert!(!is_static_asset("/custom-service-worker.js", None));
        assert!(!is_static_asset("/manifest.webmanifest", None));
        assert!(!is_static_asset("/index.html", None));
        assert!(!is_static_asset("/", None));
        assert!(!is_static_asset("/page.html", Some("text/html")));
    }

    #[test]
    fn test_lru_cache_hit_and_miss() {
        let engine = LruCacheEngine::new(1024 * 1024, 100 * 1024, 300);
        let key = "example.local:/style.css".to_string();

        assert!(engine.get(&key).is_none());

        let headers = HeaderMap::new();
        let body = Bytes::from("body { color: red; }");
        let status = StatusCode::OK;

        assert!(
            engine
                .insert(key.clone(), status, headers, body.clone(), None)
                .inserted
        );

        let hit = engine.get(&key).expect("Expected cache HIT");
        assert_eq!(hit.status, StatusCode::OK);
        assert_eq!(hit.body, body);
    }

    #[test]
    fn test_lru_cache_eviction() {
        let engine = LruCacheEngine::new(50, 100, 300);
        let headers = HeaderMap::new();

        let res1 = engine.insert(
            "key1".to_string(),
            StatusCode::OK,
            headers.clone(),
            Bytes::from("12345678901234567890"),
            None,
        );
        assert!(res1.inserted);

        let res2 = engine.insert(
            "key2".to_string(),
            StatusCode::OK,
            headers.clone(),
            Bytes::from("12345678901234567890"),
            None,
        );
        assert!(res2.inserted);

        let res3 = engine.insert(
            "key3".to_string(),
            StatusCode::OK,
            headers,
            Bytes::from("12345678901234567890"),
            None,
        );
        assert!(res3.inserted);
        assert_eq!(res3.items_evicted, 1);
        assert_eq!(res3.bytes_evicted, 20);

        assert!(engine.get("key1").is_none());
        assert!(engine.get("key2").is_some());
        assert!(engine.get("key3").is_some());
    }
}
