//! The infrastructure layer (a separate crate). The domain must not depend on this.
pub fn save(record: &str) {
    let _ = std::fs::write("/tmp/infra_store", record);
}
