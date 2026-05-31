use crate::resolver::Resolver;

/// The lookup service. Holds the resolver and exposes the geo lookups used across the app.
pub struct GeoService {
    resolver: Resolver,
}

impl GeoService {
    pub fn new() -> Self {
        GeoService { resolver: Resolver::new() }
    }

    /// Locate a single IP.
    pub fn locate(&self, ip: &str) -> Option<String> {
        self.resolver.resolve(ip)
    }

    /// Locate several IPs.
    pub fn batch(&self, ips: &[&str]) -> Vec<Option<String>> {
        ips.iter().map(|ip| self.resolver.resolve(ip)).collect()
    }
}
