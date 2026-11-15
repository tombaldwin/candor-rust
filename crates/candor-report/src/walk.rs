//! SOUNDNESS R1765 — THE ONE ANSWER TO "IS THIS DIRECTORY CARGO'S BUILD OUTPUT?", shared by every source walk.
//!
//! Every walk in this workspace used to prune a directory NAMED `target` at ANY depth. Cargo's build directory is one
//! such directory, but a source module can be one too: cc 1.4.7's `mod target;` lives in `src/target/`, and rustc
//! compiles it. The walk dropped it before any exclusion was recorded, so its functions were absent from `functions[]`
//! AND from `excluded` — a silence, not a disclosure. Six walks carried the rule (the scan's file walk and its
//! path-shape filter, two manifest walks, the dependency source index, the dependency probe, and `candor-query state`).
//!
//! What makes a directory build output is what CARGO says, and it says it three ways:
//!
//! * **`target/` BESIDE A `Cargo.toml`** — Cargo's default build directory is `target/` in the directory holding the
//!   workspace (or package) manifest. Matched by name plus that one manifest stat, before any descent, so a warm
//!   checkout's large `target/` is skipped without walking it. NOT "depth 1 of the scan root": a scan root need not be
//!   a package root (`candor-scan cc/src` is a legal manifest-less scan), and there `src/target/` sits at depth 1 and is a
//!   module (Fable E3 on R1765's first fix, which pruned it silently). Anywhere else in the walk a directory holding a
//!   `Cargo.toml` is a nested package the walks already prune, so the rule only ever fires at a package root.
//! * **a directory holding `CACHEDIR.TAG`** — Cargo writes that file (the Cache Directory Tagging spec) into every
//!   build directory it creates, whatever its name and wherever it sits, so a `CARGO_TARGET_DIR` or
//!   `build.target-dir` inside the tree is recognised once anything has been built into it. Source does not carry the
//!   tag: the spec defines it as "this directory holds regenerable cache data".
//! * **a configured target dir** — the caller passes the directories `CARGO_TARGET_DIR`, `CARGO_BUILD_TARGET_DIR` or a
//!   `build.target-dir` setting resolve to, for the case where one exists but carries no tag yet.
//!
//! A `target` directory anywhere else is ordinary source and is walked. WHICH WAY THAT FAILS: a stale build directory
//! that carries no tag, sits beside no manifest and is not configured (a vendored or copied tree with the tag stripped,
//! or a manifest-less scan root holding an old untagged `target/`) is read as SOURCE — an OVER-report of code rustc never
//! compiles, under a path that names `target::debug::` on its face. The opposite choice, pruning by name, is a SILENCE
//! over a module rustc does compile (this row). Over-report is the direction this family accepts.

use std::path::{Path, PathBuf};

/// The file Cargo (and every tool following the Cache Directory Tagging spec) writes at the root of a build directory.
pub const CACHEDIR_TAG: &str = "CACHEDIR.TAG";

/// Is `dir` — a directory entry `depth` levels below the scan root (the root itself is depth 0) — Cargo build output?
/// `configured` is the set of target directories configuration names, canonicalized by the caller (empty is fine).
pub fn is_build_output_dir(dir: &Path, depth: usize, configured: &[PathBuf]) -> bool {
    if depth == 0 {
        return false;
    }
    if dir.file_name().is_some_and(|n| n == "target") && dir.parent().is_some_and(|p| p.join("Cargo.toml").is_file()) {
        return true;
    }
    if dir.join(CACHEDIR_TAG).is_file() {
        return true;
    }
    !configured.is_empty()
        && std::fs::canonicalize(dir).is_ok_and(|c| configured.contains(&c))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("candor-report-walk-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn a_nested_target_directory_is_source_and_the_root_one_is_build_output() {
        let d = tmp("depth");
        std::fs::create_dir_all(d.join("target")).unwrap();
        std::fs::create_dir_all(d.join("src/target")).unwrap();
        assert!(!is_build_output_dir(&d.join("target"), 1, &[]), "no manifest beside it: not known build output by name");
        std::fs::write(d.join("Cargo.toml"), "[package]\nname = \"x\"\n").unwrap();
        assert!(is_build_output_dir(&d.join("target"), 1, &[]), "<root>/target beside Cargo.toml is Cargo's default build dir");
        assert!(!is_build_output_dir(&d.join("src/target"), 2, &[]), "src/target is a module directory (cc's `mod target;`)");
        assert!(!is_build_output_dir(&d.join("src/target"), 1, &[]), "and so it is when `src` is the scan root (Fable E3)");
        assert!(!is_build_output_dir(&d, 0, &[]), "the scan root is never pruned");
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn a_tagged_or_configured_directory_is_build_output_whatever_its_name() {
        let d = tmp("tag");
        std::fs::create_dir_all(d.join("build/out")).unwrap();
        std::fs::create_dir_all(d.join("src/target")).unwrap();
        std::fs::create_dir_all(d.join("cfg-out")).unwrap();
        assert!(!is_build_output_dir(&d.join("build/out"), 2, &[]));
        std::fs::write(d.join("build/out").join(CACHEDIR_TAG), "Signature: 8a477f597d28d172789f06886806bc55\n").unwrap();
        assert!(is_build_output_dir(&d.join("build/out"), 2, &[]), "a CARGO_TARGET_DIR Cargo built into carries the tag");
        let cfg = vec![std::fs::canonicalize(d.join("cfg-out")).unwrap()];
        assert!(is_build_output_dir(&d.join("cfg-out"), 1, &cfg), "a configured target dir with no tag yet");
        assert!(!is_build_output_dir(&d.join("src/target"), 2, &cfg), "configuration names one directory, not a name");
        let _ = std::fs::remove_dir_all(&d);
    }
}
