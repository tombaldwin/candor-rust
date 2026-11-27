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

// ---------------------------------------------------------------------------------------------------------------------------------
// Lane rust-floor-hedges-2 — SOUNDNESS R1932, R1934, R1935.

#[test]
fn r1932_an_anchored_use_of_a_cfg_twin_reads_the_declared_arm_beside_the_reexport_arm() {
    let r = scan("fl32");
    for p in ["fl32_mod_e", "fl32_body_e", "fl32_path_e"] {
        assert!(has(&r, p, "Fs"), "{p}: the declared fn arm's write must be read through the anchored `use`: {r:?}");
    }
    // the other orientation (re-export arm writes) and a plain single-arm facade keep reading as before; the pure twins stay pure
    for p in ["fl32r_e", "fl32fac_e", "fl32sb_e"] {
        assert!(has(&r, p, "Fs"), "{p}: {r:?}");
    }
    for p in ["fl32_q", "fl32fac_q", "fl32sb_q"] {
        assert!(pure(&r, p), "{p}: {r:?}");
    }
}

/// Raw `functions[]` rows of `dir`, optionally chained against report files (`CANDOR_DEPS`).
fn raw(dir: &Path, deps: &[PathBuf]) -> (Vec<PathBuf>, BTreeMap<String, serde_json::Value>) {
    let d = std::env::temp_dir().join(format!("candor-floor2-{}-{}", std::process::id(), N.fetch_add(1, Ordering::SeqCst)));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_candor-scan"));
    cmd.arg(dir).arg("--out").arg(d.join("r"));
    for k in ["CANDOR_DEPS", "CANDOR_CONFIG", "CANDOR_POLICY"] {
        cmd.env_remove(k);
    }
    if !deps.is_empty() {
        cmd.env("CANDOR_DEPS", std::env::join_paths(deps).unwrap());
    }
    cmd.env("CANDOR_CACHE_DIR", d.join("c"));
    let o = cmd.output().expect("run candor-scan");
    let rc = o.status.code().unwrap_or(-1);
    assert!(rc == 0 || rc == 1, "scan failed: {}", String::from_utf8_lossy(&o.stderr));
    let mut reports = Vec::new();
    let mut rows = BTreeMap::new();
    for e in std::fs::read_dir(&d).unwrap() {
        let p = e.unwrap().path();
        if p.is_file() && p.to_string_lossy().ends_with(".scan.json") {
            let v: serde_json::Value = serde_json::from_slice(&std::fs::read(&p).unwrap()).unwrap();
            for f in v["functions"].as_array().unwrap() {
                rows.insert(f["fn"].as_str().unwrap().to_string(), f.clone());
            }
            reports.push(p);
        }
    }
    if reports.is_empty() {
        // `--out <dir>/r` writes `<dir>/r.<pkg>.scan.json` beside the directory, or inside it: look one level down too
        for e in std::fs::read_dir(d.join("r")).into_iter().flatten().flatten() {
            let p = e.path();
            if p.to_string_lossy().ends_with(".scan.json") {
                let v: serde_json::Value = serde_json::from_slice(&std::fs::read(&p).unwrap()).unwrap();
                for f in v["functions"].as_array().unwrap() {
                    rows.insert(f["fn"].as_str().unwrap().to_string(), f.clone());
                }
                reports.push(p);
            }
        }
    }
    (reports, rows)
}

fn strs(v: &serde_json::Value, k: &str) -> Vec<String> {
    v[k].as_array().map(|a| a.iter().filter_map(|x| x.as_str().map(String::from)).collect()).unwrap_or_default()
}

#[test]
fn r1934_a_member_called_on_an_associated_type_of_a_dependency_bound_generic_is_disclosed() {
    let cell = fx("fl34");
    // UNCHAINED: the unit carries the dependency in `invisible` and the obligation-1 key, exactly as the qself spelling does
    let (_, rows) = raw(&cell, &[]);
    for g in ["g_fl34::assoc2", "g_fl34::assoc_q"] {
        let r = rows.get(g).unwrap_or_else(|| panic!("{g} must have a row: {rows:?}"));
        assert_eq!(strs(r, "invisible"), ["fxdep_fl34"], "{g}: {r}");
        assert_eq!(strs(r, "dispatchesOn"), ["fxdep_fl34#HasA::A::f"], "{g}: {r}");
    }
    // two dependency bounds: which trait declares `A` is not known, so the site is hedged with a reason that says so
    let two = &rows["g_fl34::assoc_two"];
    assert!(strs(two, "inferred").contains(&"Unknown".to_string()) && strs(two, "unknownWhy").iter().any(|w| w == "dispatch:T::A.f"), "{two}");
    // CHAINED against the dependency's own report: the join is live and the unit reads Unknown, as the qself spelling does
    let (dep_reports, _) = raw(&cell.join("fxdep_fl34"), &[]);
    assert!(!dep_reports.is_empty(), "the dependency must produce a report");
    let (_, chained) = raw(&cell, &dep_reports);
    for g in ["g_fl34::assoc2", "g_fl34::assoc_q", "g_fl34::assoc_sup"] {
        assert!(strs(&chained[g], "inferred").contains(&"Unknown".to_string()), "{g} must not read pure when chained: {}", chained[g]);
    }
}

#[test]
fn r1935_a_method_call_on_a_type_only_hidden_macro_text_declares_is_disclosed() {
    let r = scan("fl35");
    for p in ["fl35_let_e", "fl35_ann_e", "fl35_use_e", "fl35_ufcs_e"] {
        assert!(has(&r, p, "Unknown") && why(&r, p, "macro:module items hidden"), "{p}: {r:?}");
    }
    assert!(pure(&r, "fl35_q"), "a type in a visible module the text does not declare stays clean: {r:?}");
}

#[test]
fn round2_an_out_of_line_module_named_in_macro_text_is_read_not_hedged() {
    let r = scan("fl35o");
    assert!(has(&r, "fl35o_let_e", "Fs"), "the file the engine reads supplies the effect: {r:?}");
    assert!(!why(&r, "fl35o_let_e", "macro:module items hidden"), "no false hidden-module reason: {r:?}");
    assert!(pure(&r, "fl35o_q"), "{r:?}");
}

#[test]
fn round2_a_projection_a_non_dependency_bound_may_declare_is_hedged_not_keyed() {
    let (_, rows) = raw(&fx("fl34i"), &[]);
    let r = &rows["g_fl34i::iter"];
    assert!(strs(r, "unknownWhy").iter().any(|w| w == "dispatch:T::Item.f"), "{r}");
    // R1940: the only key is the one the WHERE bound names (`Af`, which declares `f`); never `HasA::Item::f` (HasA does not declare `Item`)
    assert!(strs(r, "dispatchesOn").iter().all(|k| k == "fxdep_fl34i#Af::f"), "no key may name a member the dependency trait does not declare: {r}");
}

#[test]
fn r1940_a_member_called_on_a_projection_a_std_only_bound_generic_bounds_by_where_is_read() {
    // `T: Iterator` (a std-only bound on `T`) with the member's trait bounding the PROJECTION. Every `_e` probe writes when run
    // (`fl40` in `expect.tsv`), and every one read ABSENT on v0.43.0 and main. A LOCAL bound trait dispatches over its implementors.
    let r = scan("fl40");
    for p in [
        "fl40_loc_e", "fl40_inl_e", "fl40_into_e", "fl40_deref_e", "fl40_impl_e", "fl40_two_e", "fl40_bind_e", "fl40_sup_e", "fl40_qs0_e",
        "fl40_recv_e",
    ] {
        assert!(has(&r, p, "Fs"), "{p}: the in-crate implementor's write must be read: {r:?}");
    }
    // the written `Item = QuietL` binding TYPES the call, so the pure twin stays pure (a resolution, not a union)
    assert!(pure(&r, "fl40_bind_q"), "fl40_bind_q: {r:?}");
    // a dependency bound whose member list is unknown is keyed like the qself spelling, unchained, and Unknown when chained
    let cell = fx("fl40");
    let (_, rows) = raw(&cell, &[]);
    for g in ["g_fl40::std_dep", "g_fl40::ctl_dep_qself"] {
        let r = rows.get(g).unwrap_or_else(|| panic!("{g} must have a row: {rows:?}"));
        assert_eq!(strs(r, "invisible"), ["fxdep_fl40"], "{g}: {r}");
        assert_eq!(strs(r, "dispatchesOn"), ["fxdep_fl40#Af::f"], "{g}: {r}");
    }
    let (dep_reports, _) = raw(&cell.join("fxdep_fl40"), &[]);
    let (_, chained) = raw(&cell, &dep_reports);
    for g in ["g_fl40::std_dep", "g_fl40::dep_clone", "g_fl40::ctl_dep_qself"] {
        assert!(strs(&chained[g], "inferred").contains(&"Unknown".to_string()), "{g} must not read pure when chained: {}", chained[g]);
    }
    // a dependency bound beside a local one is disclosed beside the local answer (Fs + Unknown), never keyed to a guess
    let both = &rows["g_fl40::loc_dep"];
    assert!(strs(both, "inferred").contains(&"Fs".to_string()) && strs(both, "unknownWhy").iter().any(|w| w == "dispatch:T::Item.lf"), "{both}");
}

#[test]
fn r1913_a_std_path_call_naming_a_sealed_std_trait_member_the_module_imports_is_not_joined_to_a_dependency_impl() {
    // `OsStr::from_bytes(..)`: std's sealed `OsStrExt::from_bytes` runs where that trait is IMPORTED, the dependency's `FromB::from_bytes`
    // (implemented for `OsStr` there) where only it is. `fl41_std_q` and `fl41_alias_q` run std's (pure, executed); on v0.43.0 and main the chained
    // join charged the dependency's write to them (a fabrication). `fl41_dep_e` runs the dependency's and must keep its charge.
    let cell = fx("fl41");
    let (dep_reports, _) = raw(&cell.join("fxdep_fl41"), &[]);
    assert!(!dep_reports.is_empty(), "the dependency must produce a report");
    let (_, chained) = raw(&cell, &dep_reports);
    let inf = |g: &str| chained.get(g).map(|r| strs(r, "inferred")).unwrap_or_default(); // a pure unit has no row
    assert!(inf("a_fl41::dep_e").contains(&"Fs".to_string()), "the dependency's member runs here and must be charged: {chained:?}");
    for g in ["b_fl41::std_q", "d_fl41::alias_q"] {
        assert!(!inf(g).contains(&"Fs".to_string()), "{g}: std's sealed member is the one imported, so the dependency's write must not be charged: {chained:?}");
    }
    // a GLOB of the std module is not an explicit import of the trait: the join stays (the conservative direction, as before)
    assert!(inf("c_fl41::glob_q").contains(&"Fs".to_string()), "a glob does not prove the std trait is the one in scope: {chained:?}");
}

#[test]
fn r1913_round2_the_withholding_needs_the_call_type_and_the_modules_own_import() {
    // Fable's `fr13` cell: every `_e` runs the DEPENDENCY's impl (executed write) while a sealed std trait is imported somewhere else — for another TYPE
    // (`PathBuf`), through a crate `pub use` re-export, in the PARENT module, or in a sibling — and must stay charged chained; the `_q` removal targets run
    // std's (pure) and must not be.
    let cell = fx("fr13");
    let (dep_reports, _) = raw(&cell.join("fxdep_fr13"), &[]);
    assert!(!dep_reports.is_empty(), "the dependency must produce a report");
    let (_, chained) = raw(&cell, &dep_reports);
    let fs = |g: &str| chained.get(g).is_some_and(|r| strs(r, "inferred").contains(&"Fs".to_string()));
    for g in ["a_fr13::e", "g_fr13::e", "i_fr13::e", "j_fr13::e", "p_fr13::e", "o_fr13::inner_fr13::e", "h_fr13::e"] {
        assert!(fs(g), "{g}: the dependency's member runs and must be charged: {chained:?}");
    }
    for g in ["p_fr13::inner2_fr13::q"] {
        assert!(!fs(g), "{g}: std's sealed member is imported by THIS module/fn, so the dependency join must not charge it: {chained:?}");
    }
}

#[test]
fn r1913_round3_only_a_module_level_import_counts_not_a_body_scoped_one() {
    // `fr13k`: a std `use` inside an inner block, a closure or a nested fn does not cover a call outside it, so the dependency's impl (executed write)
    // must stay charged; a module-level grouped / `pub(crate)` / re-exported import runs std's (pure) and is removed. `fr13l`: SliceIndex is unstable.
    let cell = fx("fr13k");
    let (dep_reports, _) = raw(&cell.join("fxdep_fr13k"), &[]);
    let (_, chained) = raw(&cell, &dep_reports);
    let fs = |g: &str| chained.get(g).is_some_and(|r| strs(r, "inferred").contains(&"Fs".to_string()));
    for g in ["k1_fr13k::e", "k2_fr13k::e", "k3_fr13k::e", "k4_fr13k::e", "k6_fr13k::inner::e"] {
        assert!(fs(g), "{g}: the dependency's member runs and must be charged: {chained:?}");
    }
    for g in ["k9_fr13k::q", "k10_fr13k::q", "k11_fr13k::q"] {
        assert!(!fs(g), "{g}: std's sealed member is imported at module level: {chained:?}");
    }
}
