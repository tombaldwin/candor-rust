use crate::service::GeoService;

/// Handle `GET /geo/:ip` — returns a JSON object for one IP.
pub fn lookup_one(svc: &GeoService, ip: &str) -> String {
    match svc.locate(ip) {
        Some(loc) => format!("{{\"loc\":\"{loc}\"}}"),
        None => "{\"error\":\"unknown\"}".to_string(),
    }
}

/// Handle `GET /geo?ips=...` — returns a JSON array of locations.
pub fn lookup_many(svc: &GeoService, ips: &[&str]) -> String {
    let locs: Vec<String> = svc.batch(ips).into_iter().flatten().collect();
    format!("[{}]", locs.join(","))
}
