use std::collections::HashMap;
use std::time::SystemTime;

/// A template engine with a TTL'd snippet cache.
pub struct Engine {
    snippets: HashMap<String, (String, SystemTime)>,
    ttl_secs: u64,
}

impl Engine {
    pub fn new() -> Self {
        let now = SystemTime::now();
        let mut snippets = HashMap::new();
        snippets.insert("year".to_string(), ("2026".to_string(), now));
        snippets.insert("brand".to_string(), ("Acme".to_string(), now));
        Engine { snippets, ttl_secs: 120 }
    }

    /// Expand a single `{{token}}` to its snippet text, if cached and fresh.
    pub fn expand(&self, token: &str) -> Option<String> {
        let (text, stored) = self.snippets.get(token)?;
        let age = SystemTime::now().duration_since(*stored).ok()?;
        if age.as_secs() <= self.ttl_secs {
            Some(text.clone())
        } else {
            None
        }
    }
}
