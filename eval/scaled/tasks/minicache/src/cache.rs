use std::collections::HashMap;
use std::time::SystemTime;

/// A simple in-memory TTL cache. Reads are cheap: a hash lookup plus a clock check for expiry.
pub struct Cache {
    map: HashMap<String, (String, SystemTime)>,
    ttl_secs: u64,
}

impl Cache {
    pub fn new() -> Self {
        Cache { map: HashMap::new(), ttl_secs: 60 }
    }

    /// Look up a key. Returns the value if present and not past its TTL.
    pub fn get(&self, key: &str) -> Option<String> {
        let (val, stored) = self.map.get(key)?;
        let age = SystemTime::now().duration_since(*stored).ok()?;
        if age.as_secs() <= self.ttl_secs {
            Some(val.clone())
        } else {
            None
        }
    }

    pub fn put(&mut self, key: &str, val: String) {
        self.map.insert(key.to_string(), (val, SystemTime::now()));
    }
}
