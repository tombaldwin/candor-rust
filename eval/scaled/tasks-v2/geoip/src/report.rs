use crate::service::GeoService;

/// Build a plain-text geo summary of a fixed set of IPs.
pub fn summary(svc: &GeoService) -> String {
    let mut out = String::new();
    for ip in ["203.0.113.1", "198.51.100.7", "192.0.2.5"] {
        if let Some(loc) = svc.locate(ip) {
            out.push_str(&format!("{ip}={loc}\n"));
        }
    }
    out
}
