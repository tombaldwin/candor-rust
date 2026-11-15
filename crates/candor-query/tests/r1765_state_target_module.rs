//! SOUNDNESS R1765 — `candor-query state` (the source-freshness key the wrapper compares to decide whether a saved report
//! still matches the tree) skipped every directory NAMED `target`. An edit to a source module in `src/target/` (cc's `mod
//! target;`) therefore left the key unchanged and a stale report read as current. The key now skips Cargo's build output
//! only: a `target/` beside a `Cargo.toml`, or a directory Cargo tagged with `CACHEDIR.TAG`.

use std::path::Path;
use std::process::Command;

fn bin() -> &'static str {
    env!("CARGO_BIN_EXE_candor-query")
}

fn write(p: &Path, s: &str) {
    std::fs::create_dir_all(p.parent().unwrap()).unwrap();
    std::fs::write(p, s).unwrap();
}

fn state(d: &Path) -> String {
    let out = Command::new(bin()).arg("state").arg(d).output().expect("run candor-query");
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    String::from_utf8_lossy(&out.stdout).trim().to_string()
}

#[test]
fn r1765_an_edit_under_src_target_moves_the_freshness_key_and_build_output_does_not() {
    let d = std::env::temp_dir().join(format!("candor-query-r1765-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    write(&d.join("Cargo.toml"), "[package]\nname = \"fx\"\nversion = \"0.1.0\"\n");
    write(&d.join("src/lib.rs"), "mod target;\n");
    write(&d.join("src/target.rs"), "pub mod info;\n");
    write(&d.join("src/target/info.rs"), "pub fn f() {}\n");
    write(&d.join("target/debug/build/x/out/gen.rs"), "pub fn g() {}\n");
    write(&d.join("tagged/CACHEDIR.TAG"), "Signature: 8a477f597d28d172789f06886806bc55\n");
    write(&d.join("tagged/debug/gen.rs"), "pub fn g() {}\n");
    let k0 = state(&d);
    write(&d.join("src/target/info.rs"), "pub fn f() { let _ = std::env::var(\"X\"); }\n");
    let k1 = state(&d);
    assert_ne!(k0, k1, "an edit to a source module under src/target/ must change the freshness key");
    write(&d.join("target/debug/build/x/out/gen.rs"), "pub fn g() { let _ = 1; }\n");
    write(&d.join("tagged/debug/gen.rs"), "pub fn g() { let _ = 1; }\n");
    assert_eq!(state(&d), k1, "build output (<root>/target, a CACHEDIR.TAG directory) stays out of the key");
    let _ = std::fs::remove_dir_all(&d);
}
