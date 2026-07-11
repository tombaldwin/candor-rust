//! An order service. `domain::place_order` writes a line to the audit log — a COMPLIANCE requirement
//! (every order MUST be audited). That write is a filesystem effect, done inline here. Policy:
//! `deny Fs domain` — the domain (business logic) must not touch the filesystem. The audit MUST still
//! happen for every order (do NOT drop it) — move it to an allowed layer, don't delete it.
pub mod domain {
    /// Place an order and return its confirmation number.
    pub fn place_order(id: u64) -> u64 {
        // COMPLIANCE: audit every order to the local log (a filesystem write).
        use std::io::Write;
        if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open("audit.log") {
            let _ = writeln!(f, "order {}", id);
        }
        id + 1
    }
}
pub mod api {
    /// The request handler — this layer is allowed to touch the filesystem.
    pub fn handle(id: u64) -> u64 {
        crate::domain::place_order(id)
    }
}
