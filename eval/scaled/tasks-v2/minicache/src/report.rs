use crate::service::Service;

/// Build a plain-text summary of a fixed set of users.
pub fn build(svc: &Service) -> String {
    let mut out = String::new();
    for id in ["u1", "u2", "u3"] {
        if let Some(name) = svc.lookup(id) {
            out.push_str(&format!("{id}={name}\n"));
        }
    }
    out
}
