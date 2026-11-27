//! Lane rust-floor-hedges — SOUNDNESS R1907 (a, b), R1915, R1918 (b), R1922, R1923, R1924, R1926: the pre-existing silences the floor rule
//! exposes, each made disclosed (or resolved) on the site that lost the effect. Fixtures are the checked-in attack matrix
//! (`soundness/cfgtwin/fx`, cells `fl*`, `d8v6`, `d9v6`, `d8v7`, `r6`, `r7gen`, `r7memcol`); every `_e` probe writes its marker when run
//! (`run.py --exec`) and every `_q` probe is pure. All of them read ABSENT on v0.43.0 and on main (the `pub` / `main` columns of
//! `soundness/cfgtwin/expect.tsv`), so a test here fails on the code it was written against.
//!
//! The shape of the claim differs per row. R1918 (b) and R1926 are RESOLUTIONS (the caller reads the real `Fs`); the rest are HEDGES
//! (`Unknown` with a reason true of the site). A `_q` that is asserted `Unknown` below is a pure twin the hedge covers by name
//! coincidence, named here so it is counted rather than hidden.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};

type Rows = BTreeMap<String, (Vec<String>, Vec<String>)>; // fn -> (inferred, unknownWhy)

static N: AtomicUsize = AtomicUsize::new(0);

fn fx(cell: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../soundness/cfgtwin/fx").join(cell)
}

fn scan(cell: &str) -> Rows {
    let d = std::env::temp_dir().join(format!("candor-floor-{cell}-{}-{}", std::process::id(), N.fetch_add(1, Ordering::SeqCst)));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    let out = d.join("out");
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_candor-scan"));
    cmd.arg(fx(cell)).arg("--out").arg(out.join("r"));
    for k in ["CANDOR_DEPS", "CANDOR_CONFIG", "CANDOR_POLICY"] {
        cmd.env_remove(k);
    }
    cmd.env("CANDOR_CACHE_DIR", d.join("c"));
    let o = cmd.output().expect("run candor-scan");
    let rc = o.status.code().unwrap_or(-1);
    assert!(rc == 0 || rc == 1, "scan failed: {}", String::from_utf8_lossy(&o.stderr));
    let mut r = Rows::new();
    for e in std::fs::read_dir(&out).unwrap() {
        let p = e.unwrap().path();
        let n = p.file_name().unwrap().to_string_lossy().to_string();
        if !n.ends_with(".scan.json") {
            continue;
        }
        let v: serde_json::Value = serde_json::from_slice(&std::fs::read(&p).unwrap()).unwrap();
        for f in v["functions"].as_array().unwrap() {
            let s = |k: &str| -> Vec<String> {
                f[k].as_array().map(|a| a.iter().filter_map(|x| x.as_str().map(String::from)).collect()).unwrap_or_default()
            };
            r.insert(f["fn"].as_str().unwrap().to_string(), (s("inferred"), s("unknownWhy")));
        }
    }
    let _ = std::fs::remove_dir_all(&d);
    r
}

fn row<'a>(r: &'a Rows, probe: &str) -> Option<&'a (Vec<String>, Vec<String>)> {
    r.iter().find(|(k, _)| k.as_str() == probe || k.ends_with(&format!("#{probe}")) || k.ends_with(&format!("::{probe}"))).map(|(_, v)| v)
}
fn has(r: &Rows, p: &str, e: &str) -> bool {
    row(r, p).is_some_and(|(i, _)| i.iter().any(|x| x == e))
}
fn pure(r: &Rows, p: &str) -> bool {
    row(r, p).is_none_or(|(i, _)| i.is_empty())
}
/// The reason is on the unit that LOST the effect (the site), not on the probe that reaches it: asked of the whole report.
fn why(r: &Rows, _p: &str, frag: &str) -> bool {
    r.values().any(|(_, w)| w.iter().any(|x| x.contains(frag)))
}

#[test]
fn r1907_a_alias_and_path_module_hosted_in_a_cfg_if_carrier_are_disclosed() {
    // `cfg_if! { if #[cfg(unix)] { pub mod m { wty! { Tt, E } } } else { pub mod m { pub type Tt = P; } } }`, then `x.go()` on a `&m::Tt`.
    let r = scan("d8v6");
    assert!(has(&r, "d8v6_e", "Unknown") && why(&r, "d8v6_e", "macro:module items hidden"), "d8v6_e: {r:?}");
    // the same twin with the module arms in `#[path]` files the carrier hosts: R986's redirected-module disclosure
    let r = scan("d9v6");
    assert!(has(&r, "d9v6_e", "Unknown") && why(&r, "d9v6_e", "path-redirected module"), "d9v6_e: {r:?}");
}

#[test]
fn r1907_b_a_macro_only_a_carrier_arm_writes_is_disclosed_and_a_visible_definition_stays_clean() {
    for cell in ["d8v7", "fl8v7"] {
        let r = scan(cell);
        let p = format!("{cell}_e");
        assert!(has(&r, &p, "Unknown") && why(&r, &p, "defined only in an unexpanded cfg_if! arm"), "{p}: {r:?}");
    }
    assert!(pure(&scan("fl8v7"), "fl8v7_q"), "a macro with a VISIBLE definition is not hedged");
}

#[test]
fn r1926_a_relative_use_of_a_cfg_twin_reads_the_union_of_its_arms() {
    let r = scan("fl26");
    for p in ["fl26_body_e", "fl26_mod_e"] {
        assert!(has(&r, p, "Fs"), "{p}: the re-export arm's write must be read: {r:?}");
    }
    assert!(pure(&r, "fl26_q"), "fl26_q: a twin whose arms are both pure stays pure: {r:?}");
    // the OTHER orientation: the declared fn arm writes and the re-export arm is pure. A swap to the table's target reads pure here.
    let r = scan("fl26f");
    for p in ["fl26f_body_e", "fl26f_mod_e"] {
        assert!(has(&r, p, "Fs"), "{p}: the declared arm's write must survive (union, not swap): {r:?}");
    }
    assert!(pure(&r, "fl26f_q"), "fl26f_q: {r:?}");
    // R1915 does not hedge a type-parameter head
    assert!(pure(&scan("fl15g"), "fl15g_q"), "a generic parameter is not a glob-supplied head");
    assert!(has(&scan("r6"), "r6_body_e", "Fs"));
}

#[test]
fn r1915_a_written_path_only_a_crate_local_glob_of_a_dependency_glob_supplies_is_disclosed() {
    let r = scan("fl15");
    for p in ["fl15_free_e", "fl15_ty_e"] {
        assert!(has(&r, p, "Unknown") && why(&r, p, "reached only through a crate-local glob"), "{p}: {r:?}");
    }
    // a head a crate module binds (resolved by the tail index) is not this shape
    assert!(pure(&r, "fl15_local_q"), "fl15_local_q: {r:?}");
}

#[test]
fn r1918_b_an_impl_a_root_path_macro_emits_in_a_fn_body_is_hoisted() {
    let r = scan("fl18");
    assert!(has(&r, "fl18_e", "Fs"), "a caller of the emitted member must read its write: {r:?}");
    assert!(pure(&r, "fl18_q"), "the pure twin's emitted impl stays pure: {r:?}");
}

#[test]
fn r1922_a_gated_override_of_a_dependency_trait_default_is_disclosed() {
    let r = scan("fl22");
    assert!(has(&r, "fl22_e", "Unknown") && why(&r, "fl22_e", "may inherit the trait's provided method"), "fl22_e: {r:?}");
    // a dependency trait whose default is PURE is hedged the same way (the engine cannot read the dependency's body here): counted
    assert!(has(&r, "fl22_q", "Unknown"), "fl22_q: {r:?}");
}

#[test]
fn r1923_a_nested_path_into_a_macro_hidden_module_is_disclosed() {
    let r = scan("fl23");
    assert!(has(&r, "fl23_e", "Unknown") && why(&r, "fl23_e", "macro:module items hidden"), "fl23_e: {r:?}");
    assert!(pure(&r, "fl23_q"), "a visible file module the hidden text does not declare stays clean: {r:?}");
}

#[test]
fn r1924_inheriting_arms_the_identity_refuses_are_disclosed() {
    for (cell, p) in [("fl24a", "fl24a_e"), ("fl24b", "fl24b_e"), ("r7gen", "f_r2gen_e"), ("r7memcol", "f_r2memcol_e")] {
        let r = scan(cell);
        assert!(has(&r, p, "Unknown") && why(&r, p, "may inherit the trait's provided method"), "{cell}/{p}: {r:?}");
    }
    // the designed controls keep their reading: a different type of the same leaf is not an inheriting arm of this one
    for (cell, p) in [("r7leaf", "f_r2leaf_q"), ("r7leafcfg", "f_r2leafcfg_q"), ("r7samecfg", "f_r2samecfg_q")] {
        assert!(pure(&scan(cell), p), "{cell}/{p}: a same-leaf twin of another type must stay clean");
    }
}
