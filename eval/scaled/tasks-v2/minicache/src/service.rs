use crate::cache::Cache;

/// The service layer. Holds the cache and exposes lookups used across the app.
pub struct Service {
    cache: Cache,
}

impl Service {
    pub fn new() -> Self {
        let mut cache = Cache::new();
        cache.put("u1", "Alice".into());
        cache.put("u2", "Bob".into());
        Service { cache }
    }

    /// Look up one user by id (cache-backed).
    pub fn lookup(&self, id: &str) -> Option<String> {
        self.cache.get(id)
    }

    /// Look up several users by id.
    pub fn batch(&self, ids: &[&str]) -> Vec<Option<String>> {
        ids.iter().map(|id| self.cache.get(id)).collect()
    }
}
