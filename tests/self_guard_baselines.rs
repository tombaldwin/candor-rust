//! SOUNDNESS R1633 — every workspace member has a committed Self-guard baseline.
//!
//! CI's Self-guard runs `CANDOR_BASELINE=.candor/baseline cargo dylint --lib-path … --workspace`, and the
//! engine looks each unit up as `.candor/baseline.<crate>.<kind>.json` (`<kind>` is the crate type: `Rlib`
//! for a library target, `Executable` for a binary). A unit with no such file is compared against NOTHING.
//! The step turns that into a red build, but only on CI; this test makes a new member (or a new target on
//! an existing one) fail locally, and checks that `.candor/.gitignore` allow-lists the file — an
//! un-allow-listed baseline is written by `snapshot`, never committed, and the gap reappears on CI.
use std::path::Path;

fn members(root: &Path) -> Vec<String> {
    let manifest = std::fs::read_to_string(root.join("Cargo.toml")).unwrap();
    let line = manifest.lines().find(|l| l.trim_start().starts_with("members")).expect("workspace members line");
    line.split('"').skip(1).step_by(2).map(str::to_string).collect()
}

fn package_name(dir: &Path) -> String {
    let m = std::fs::read_to_string(dir.join("Cargo.toml")).unwrap();
    let l = m.lines().find(|l| l.trim_start().starts_with("name") && l.contains('"')).expect("package name");
    l.split('"').nth(1).unwrap().replace('-', "_")
}

#[test]
fn every_workspace_unit_has_a_committed_self_guard_baseline() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let ignore = std::fs::read_to_string(root.join(".candor/.gitignore")).unwrap();
    let mut units = 0;
    for member in members(root) {
        let dir = root.join(&member);
        let name = package_name(&dir);
        for (src, kind) in [("src/lib.rs", "Rlib"), ("src/main.rs", "Executable")] {
            if !dir.join(src).is_file() {
                continue;
            }
            units += 1;
            let file = format!("baseline.{name}.{kind}.json");
            let body = std::fs::read_to_string(root.join(".candor").join(&file))
                .unwrap_or_else(|_| panic!("{member}: no .candor/{file} — Self-guard would compare this unit against nothing"));
            let v: serde_json::Value = serde_json::from_str(&body).unwrap_or_else(|e| panic!(".candor/{file} is not JSON: {e}"));
            assert!(v["functions"].is_array(), ".candor/{file} has no `functions` array");
            assert!(ignore.lines().any(|l| l == format!("!{file}")), ".candor/.gitignore does not allow-list {file}");
        }
    }
    assert!(units >= 6, "found only {units} workspace units — the member parse is broken, not the baselines");
}
