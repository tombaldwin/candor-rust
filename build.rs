// Stamp the candor *source* version (short git hash) and pinned toolchain into the dylib at
// build time. Why: a baseline is only comparable to the exact engine that produced it, and the
// guard must compare against the engine THAT WILL RUN — i.e. the dylib's true build commit —
// not the source tree's current git HEAD. Those diverge the moment you `git pull` candor without
// rebuilding: HEAD moves ahead while the installed dylib stays old, and a HEAD-based version
// check silently masks a stale baseline (the exact situation found in a real consumer). The
// dylib carries its own version so `cargo-candor` and the report sidecar can report the truth.
use std::process::Command;

fn main() {
    let version = Command::new("git")
        .args(["rev-parse", "--short", "HEAD"])
        .output()
        .ok()
        .filter(|o| o.status.success())
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "unknown".to_string());
    println!("cargo:rustc-env=CANDOR_VERSION={version}");

    // Pinned toolchain, parsed from the rust-toolchain file (the channel = "..." line). The file
    // is named `rust-toolchain` here, but accept the `.toml` form too for portability.
    let toolchain = std::fs::read_to_string("rust-toolchain")
        .or_else(|_| std::fs::read_to_string("rust-toolchain.toml"))
        .ok()
        .and_then(|s| {
            s.lines()
                .find(|l| l.trim_start().starts_with("channel"))
                .and_then(|l| l.split('"').nth(1).map(str::to_string))
        })
        .unwrap_or_else(|| "unknown".to_string());
    println!("cargo:rustc-env=CANDOR_TOOLCHAIN={toolchain}");
    println!("cargo:rerun-if-changed=rust-toolchain");

    // Re-stamp when the commit changes: HEAD itself, and the branch ref HEAD points at (a plain
    // commit updates the ref, not HEAD). Missing paths (no .git, e.g. a packaged source) just
    // force a conservative rebuild rather than failing.
    println!("cargo:rerun-if-changed=.git/HEAD");
    // Also watch `packed-refs`: `git gc` (which runs automatically, and a `git pull` can trigger) moves
    // a branch ref from the loose `.git/refs/heads/<branch>` into `.git/packed-refs` and deletes the
    // loose file. A later `git pull` then updates `packed-refs` ONLY — `.git/HEAD` is unchanged (still
    // `ref: refs/heads/<branch>`) and the loose ref is gone — so without watching packed-refs the
    // version stamp goes stale exactly in the pull-without-rebuild case this exists to catch.
    println!("cargo:rerun-if-changed=.git/packed-refs");
    if let Ok(head) = std::fs::read_to_string(".git/HEAD") {
        if let Some(r) = head.strip_prefix("ref: ") {
            println!("cargo:rerun-if-changed=.git/{}", r.trim());
        }
    }
}
