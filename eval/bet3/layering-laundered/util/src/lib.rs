//! A generic utility crate. It happens to reach into `infra` — so anything depending on `util` for
//! `store` transitively depends on `infra`, a fact invisible at the `util::store` call site.
pub fn store(record: &str) { infra::save(record); }
