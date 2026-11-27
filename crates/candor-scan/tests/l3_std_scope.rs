//! Lane rust-bound-l3 — SOUNDNESS R1951 / R1962 (additive) and R1956 / R1963 (removal): the std-type path rule
//! (`collector::r1757_std_type_dep_path`) decides a member a std trait declares by SCOPE, not by NAME, and a GLOB of a std module that
//! brings a sealed trait withholds the dependency join where std's member is the one that runs.
//!
//! Fixtures are the checked-in attack matrix (`soundness/cfgtwin/fx`, cells `l3s`, `l3e`, `l3g`, `bd2`, `bm3`, `fl41`, `fr13`, `fr13l`).
//! Every `_e` probe writes its marker when run (`run.py --exec`: the dependency's impl runs) and every `_q` probe is pure (std's member
//! runs). Every reading here is CHAINED — the fixture scanned with its own dependency's report — because the marker is inert unchained:
//! the published behaviour, and the reason the unchained corpus cannot see this rule at all. The `_e` assertions read `-` on v0.43.0 and on
//! main (`expect.tsv`'s `pub`/`main` columns), so they fail on the code they were written against.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};

type Rows = BTreeMap<String, (Vec<String>, Vec<String>)>; // fn -> (inferred, unknownWhy)

static N: AtomicUsize = AtomicUsize::new(0);

fn fx(cell: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../soundness/cfgtwin/fx").join(cell)
}

fn run(target: &Path, out: &Path, cache: &Path, deps: Option<&str>, env: &[(&str, &str)]) {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_candor-scan"));
    cmd.arg(target).arg("--out").arg(out.join("r"));
    for k in ["CANDOR_DEPS", "CANDOR_CONFIG", "CANDOR_POLICY", "CANDOR_R1913_OFF", "CANDOR_R1757_OFF"] {
        cmd.env_remove(k);
    }
    cmd.env("CANDOR_CACHE_DIR", cache);
    if let Some(d) = deps {
        cmd.env("CANDOR_DEPS", d);
    }
    for (k, v) in env {
        cmd.env(k, v);
    }
    let o = cmd.output().expect("run candor-scan");
    let rc = o.status.code().unwrap_or(-1);
    assert!(rc == 0 || rc == 1, "scan failed: {}", String::from_utf8_lossy(&o.stderr));
}

fn reports(out: &Path) -> Vec<PathBuf> {
    std::fs::read_dir(out)
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| p.file_name().unwrap().to_string_lossy().ends_with(".scan.json"))
        .collect()
}

/// The cell scanned CHAINED over its own dependency's report (`fxdep_<cell>`), as `run.py` does for a `dep` cell.
fn chained(cell: &str, env: &[(&str, &str)]) -> Rows {
    let d = std::env::temp_dir().join(format!("candor-l3-{cell}-{}-{}", std::process::id(), N.fetch_add(1, Ordering::SeqCst)));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    let dep_out = d.join("dep");
    run(&fx(cell).join(format!("fxdep_{cell}")), &dep_out, &d.join("c1"), None, env);
    let deps: Vec<String> = reports(&dep_out).iter().map(|p| p.to_string_lossy().to_string()).collect();
    assert!(!deps.is_empty(), "{cell}: the dependency produced no report");
    let out = d.join("out");
    run(&fx(cell), &out, &d.join("c2"), Some(&deps.join(":")), env);
    let mut r = Rows::new();
    for p in reports(&out) {
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
    r.iter().find(|(k, _)| k.as_str() == probe || k.ends_with(&format!("::{probe}"))).map(|(_, v)| v)
}
fn has(r: &Rows, p: &str, e: &str) -> bool {
    row(r, p).is_some_and(|(i, _)| i.iter().any(|x| x == e))
}

#[test]
fn r1951_a_std_trait_name_joins_the_dependency_where_no_std_trait_applies_in_scope() {
    let r = chained("l3s", &[]);
    for p in [
        "l3s_fromstr_dep_e",     // `String::from_str` with only a dependency `Parse` imported: FromStr is implemented for String, not in scope
        "l3s_default_notimpl_e", // `Instant::default()`: Default is in the prelude, not implemented for Instant (R1962)
        "l3s_get_pathbuf_e",     // `PathBuf::get`: SliceIndex's members are unstable, never a candidate
        "l3s_get_usize_e",       // `usize::get` (SliceIndex IS implemented for usize, but its `get` is unstable)
    ] {
        assert!(has(&r, p, "Fs"), "{p} must read the dependency's Fs chained: {:?}", row(&r, p));
    }
    // std's member runs (pure); the dependency still implements the same member for the type, so a carried marker would charge it
    for p in [
        "l3s_default_prelude_q",   // `String::default()`: Default (prelude) is implemented for String
        "l3s_fromstr_import_q",    // `use std::str::FromStr`
        "l3s_fromstr_fnbody_q",    // a fn-body `use std::str::FromStr`
        "l3s_fromstr_superglob_q", // `use super::*` of a module importing it
        "l3s_fromstr_reexport_q",  // `use crate::pre::FromStr` (a local re-export of std's)
        "l3s_fromstr_asunder_q",   // `use std::str::FromStr as _`
        "l3s_fromstr_rename_q",    // `use std::str::FromStr as Fs`
        "l3s_tryfrom_2021_q",      // TryFrom: the 2021 prelude, blanket impl
        "l3s_fromiter_2021_q",     // FromIterator: the 2021 prelude
    ] {
        assert!(!has(&r, p, "Fs"), "{p} runs std's member and must not be charged the dependency's: {:?}", row(&r, p));
    }
    // the dependency's impl that runs is PURE, so no entry joins: the miss is disclosed with a true reason
    assert!(has(&r, "l3s_default_pureimpl_q", "Unknown") && !has(&r, "l3s_default_pureimpl_q", "Fs"), "{:?}", row(&r, "l3s_default_pureimpl_q"));
}

#[test]
fn r1962_a_prelude_trait_of_a_later_edition_joins_only_in_an_older_package() {
    // edition 2018: FromIterator / TryFrom are not in the prelude, so the dependency's member runs
    let r = chained("l3e", &[]);
    assert!(has(&r, "l3e_fromiter_2018_e", "Fs"), "{:?}", row(&r, "l3e_fromiter_2018_e"));
    assert!(!has(&r, "l3e_fromiter_import_2018_q", "Fs"), "{:?}", row(&r, "l3e_fromiter_import_2018_q"));
    let r = chained("bd2", &[]);
    assert!(has(&r, "bd2_tryfrom_2018_e", "Fs"), "{:?}", row(&r, "bd2_tryfrom_2018_e"));
}

#[test]
fn r1951_design_cells_close() {
    let r = chained("bm3", &[]);
    for p in ["bm3_prelude_name_not_impl_e", "bm3_nonprelude_name_e", "bm3_dep_glob_prelude_name_e"] {
        assert!(has(&r, p, "Fs"), "{p}: {:?}", row(&r, p));
    }
    assert!(!has(&r, "bm3_std_trait_in_scope_q", "Fs"), "{:?}", row(&r, "bm3_std_trait_in_scope_q"));
    let r = chained("fl41", &[]);
    assert!(has(&r, "fl41_ordt_e", "Fs"), "{:?}", row(&r, "fl41_ordt_e"));
    let r = chained("fr13l", &[]);
    for p in ["fr13l_usize_e", "fr13l_range_e"] {
        assert!(has(&r, p, "Fs"), "{p}: {:?}", row(&r, p));
    }
    let r = chained("fr13", &[]);
    assert!(has(&r, "fr13_sliceidx_e", "Fs"), "{:?}", row(&r, "fr13_sliceidx_e"));
}
