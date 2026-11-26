//! Lane rust-macropath-l3 — R-BARE-TC (SOUNDNESS R1908, R1914, R1899). Fixtures are the checked-in attack matrix group E
//! (`soundness/macropath/fx/e1..e17`, executed ground truth in `soundness/macropath/expect.tsv`): a bare item-position
//! `thread_local!` beside a cfg pair `gated_eN` (both arms written in full). The pair reads `Unknown` exactly when the
//! invocation may be a DEPENDENCY's macro (hidden text, so one arm's items may be written there).
//!
//! - a glob never rebinds a std-prelude name in a crate that has std's macro prelude (E0659 executed): e1, e2, e7, e15, e17 read
//!   NO hold now; on main they carried one whose reason was false;
//! - the same glob does rebind `thread_local` in a `#![no_std]` crate, in the module that holds it or that a local glob of it
//!   reaches: e8 (kept), e9 (`use crate::prelude::*;`) and e10 (a glob a local macro emits) now hold;
//! - an explicit `use dep::thread_local;` (e5) and a cfg_attr no_std crate (e14, the build with no std) keep their hold.
//!
//! `CANDOR_R_BARE_TC_OFF=1` restores main's reading for every cell.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};

fn fx(cell: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../soundness/macropath/fx").join(cell)
}

static N: AtomicUsize = AtomicUsize::new(0);

/// `fn` -> `inferred`, from one scan in a directory of its own, removed afterwards.
fn scan(cell: &str, env: &[(&str, &str)]) -> BTreeMap<String, Vec<String>> {
    let d = std::env::temp_dir().join(format!("candor-baretc-{cell}-{}-{}", std::process::id(), N.fetch_add(1, Ordering::SeqCst)));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    let out = d.join("out");
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_candor-scan"));
    cmd.arg(fx(cell)).arg("--out").arg(out.join("r"));
    for k in ["CANDOR_DEPS", "CANDOR_CONFIG", "CANDOR_POLICY", "CANDOR_R_BARE_TC_OFF"] {
        cmd.env_remove(k);
    }
    for (k, v) in env {
        cmd.env(k, v);
    }
    cmd.env("CANDOR_CACHE_DIR", d.join("c"));
    let o = cmd.output().expect("run candor-scan");
    assert!(o.status.code().is_some_and(|c| c == 0 || c == 1), "scan failed: {}", String::from_utf8_lossy(&o.stderr));
    let mut r = BTreeMap::new();
    let mut reports = 0;
    for e in std::fs::read_dir(&out).unwrap() {
        let p = e.unwrap().path();
        let n = p.file_name().unwrap().to_string_lossy().to_string();
        if n.ends_with(".json") && !n.ends_with(".callgraph.json") && !n.ends_with(".locs.json") && !n.ends_with(".hierarchy.json") {
            let v: serde_json::Value = serde_json::from_slice(&std::fs::read(&p).unwrap()).unwrap();
            reports += 1;
            for f in v["functions"].as_array().unwrap() {
                let inferred = f["inferred"].as_array().map(|a| a.iter().filter_map(|e| e.as_str().map(str::to_string)).collect()).unwrap_or_default();
                r.insert(f["fn"].as_str().unwrap().to_string(), inferred);
            }
        }
    }
    let _ = std::fs::remove_dir_all(&d);
    assert!(reports > 0, "{cell}: the scan wrote no report (a fixture the engine did not reach reads as 'no hold')");
    r
}

/// Does the cell's `gated_<cell>` pair carry `Unknown`? A pair with no hold has no row at all, so a negative reading is vacuous
/// unless the same cell reads positive under the other setting: every test below asserts BOTH, which is what proves the fixture
/// reaches the code.
fn held(cell: &str, env: &[(&str, &str)]) -> bool {
    let probe = format!("gated_{cell}");
    scan(cell, env).iter().any(|(k, v)| (*k == probe || k.ends_with(&format!("::{probe}"))) && v.iter().any(|e| e == "Unknown"))
}

#[test]
fn a_glob_never_rebinds_a_std_prelude_name_in_a_crate_that_has_it() {
    // E0659 over every one of these (executed): the glob cannot supply `thread_local`, so no cfg arm hides behind it.
    for cell in ["e1", "e2", "e7", "e15", "e17"] {
        assert!(!held(cell, &[]), "{cell}: a glob hold with a false reason");
        // the same scan with the switch: main's reading holds (the instrument can fail; the cell is not hollow)
        assert!(held(cell, &[("CANDOR_R_BARE_TC_OFF", "1")]), "{cell}: main's reading did not hold, the cell proves nothing");
    }
}

#[test]
fn holds_that_are_true_stay_and_no_std_holds_are_per_module() {
    // explicit import, a no_std lib with a glob in the module, a cfg_attr(no_std) lib (the build without std)
    for cell in ["e5", "e8", "e14"] {
        assert!(held(cell, &[]), "{cell}: a true hold was lost");
        assert!(held(cell, &[("CANDOR_R_BARE_TC_OFF", "1")]), "{cell}: main's reading");
    }
    // E9 / E10: the glob reaches the module through a local glob (`use crate::prelude::*;`), or a macro emits it. Silent on main.
    for cell in ["e9", "e10"] {
        assert!(held(cell, &[]), "{cell}: the dependency's glob may supply thread_local; no hold is a silence");
        assert!(!held(cell, &[("CANDOR_R_BARE_TC_OFF", "1")]), "{cell}: main did not hold (the cell must show the silence it closes)");
    }
}

#[test]
fn cells_this_lane_does_not_move() {
    // E6: a glob in a module nobody globs; E16: a local thread_local definition (its body is read); E12: `write` (a core name);
    // E3/E4/E11/E13: a `#[macro_use] extern crate` in the ROOT, the invocation in another file (L3(c), not shipped).
    for cell in ["e6", "e12", "e16", "e3", "e4", "e11", "e13"] {
        let (on, off) = (scan(cell, &[]), scan(cell, &[("CANDOR_R_BARE_TC_OFF", "1")]));
        assert_eq!(on, off, "{cell}: the switch moved a cell L3 does not touch");
    }
}

// Design revision 2 (Fable's merge review): an explicit `use` is classified by where it LANDS, and `macro_use` is read through
// `cfg_attr`. E20..E31 are the executed cells (g5 g5c g5k g5x g5n g7 g5s g5t g5w g2 g2b g1).
#[test]
fn an_explicit_use_that_lands_on_a_dependency_re_export_holds() {
    // e20/e21/e22 (same-file prelude, three spellings, a no_std lib) and e29 (cfg_attr macro_use) hold on main too: they must not be lost.
    for cell in ["e20", "e21", "e22", "e29", "e30", "e31"] {
        assert!(held(cell, &[("CANDOR_R_BARE_TC_OFF", "1")]), "{cell}: main's reading did not hold, the cell proves nothing");
        assert!(held(cell, &[]), "{cell}: a hold main had was lost (an introduced 1 -> 0)");
    }
    // e23/e24/e25/e28 are silent on main (cross-file / `super::` / cfg twin) and now hold; the switch restores main's silence.
    for cell in ["e23", "e24", "e25", "e28"] {
        assert!(held(cell, &[]), "{cell}: the dependency's macro may bind; no hold is a silence");
        assert!(!held(cell, &[("CANDOR_R_BARE_TC_OFF", "1")]), "{cell}: main did not hold");
    }
}

#[test]
fn an_explicit_use_that_lands_on_the_toolchain_does_not_hold() {
    // e26: prelude.rs re-exports std's; e27: the same inside the file. std's binds, so a hold would carry a false reason.
    for cell in ["e26", "e27"] {
        assert!(!held(cell, &[]), "{cell}: a false-reason hold");
        assert_eq!(scan(cell, &[]), scan(cell, &[("CANDOR_R_BARE_TC_OFF", "1")]), "{cell}: the switch moved a control");
    }
}
