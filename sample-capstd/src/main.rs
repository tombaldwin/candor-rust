//! cap-std conformance sample. candor recognises cap-std's capability types as real,
//! unforgeable declarations and its operations as the matching effect.
//!
//!   CANDOR_STRICT=1 cargo dylint --lib-path <…/libcandor@…dylib>
#![allow(dead_code)]

use cap_std::ambient_authority;
use cap_std::fs::Dir;

// ── CONFORMANT: holds a `Dir` capability, performs Fs *through* it. ───────────
// declared { Fs } (from `&Dir`) == inferred { Fs } (from `dir.read_to_string`).
fn read_config(dir: &Dir, name: &str) -> std::io::Result<String> {
    dir.read_to_string(name)
}

// ── VIOLATION: reaches for AMBIENT fs without holding any capability. ─────────
fn read_ambient(path: &str) -> std::io::Result<String> {
    std::fs::read_to_string(path) // inferred { Fs }, declared {} -> AS-EFF-001
}

fn main() -> std::io::Result<()> {
    let dir = Dir::open_ambient_dir(".", ambient_authority())?;
    let _ = read_config(&dir, "Cargo.toml");
    let _ = read_ambient("Cargo.toml");
    Ok(())
}
