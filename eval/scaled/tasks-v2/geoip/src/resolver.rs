use std::collections::HashMap;
use std::time::SystemTime;

/// An in-memory geo-IP resolver with a TTL cache.
pub struct Resolver {
    map: HashMap<String, (String, SystemTime)>,
    ttl_secs: u64,
}

impl Resolver {
    pub fn new() -> Self {
        let now = SystemTime::now();
        let mut map = HashMap::new();
        map.insert("203.0.113.1".to_string(), ("Berlin, DE".to_string(), now));
        map.insert("198.51.100.7".to_string(), ("Tokyo, JP".to_string(), now));
        Resolver { map, ttl_secs: 300 }
    }

    /// Resolve an IP to a location. Returns the cached location if present and not past its TTL.
    pub fn resolve(&self, ip: &str) -> Option<String> {
        let (loc, stored) = self.map.get(ip)?;
        let age = SystemTime::now().duration_since(*stored).ok()?;
        if age.as_secs() <= self.ttl_secs {
            Some(loc.clone())
        } else {
            None
        }
    }
}
