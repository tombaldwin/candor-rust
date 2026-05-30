use crate::service::Service;

/// Handle `GET /user/:id` — returns a JSON object for one user.
pub fn get_one(svc: &Service, id: &str) -> String {
    match svc.lookup(id) {
        Some(name) => format!("{{\"name\":\"{name}\"}}"),
        None => "{\"error\":\"not found\"}".to_string(),
    }
}

/// Handle `GET /users?ids=...` — returns a JSON array of names.
pub fn get_many(svc: &Service, ids: &[&str]) -> String {
    let names: Vec<String> = svc.batch(ids).into_iter().flatten().collect();
    format!("[{}]", names.join(","))
}
