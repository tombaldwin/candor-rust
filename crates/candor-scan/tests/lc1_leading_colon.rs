//! Lane rust-lc1 — a supertrait written with a LEADING `::` names an EXTERN crate in edition 2018+ (`::fxdep_lc1::Sup`), never the same-named
//! LOCAL module's trait. Main read it as the local trait (`classify_super_reading` stripped the `::` and matched the local declaration), CHA'd the
//! local trait's implementors and charged an executed-pure program `Fs`. Edition 2015 is the other reading: `::m::Sup` is the crate ROOT's `m`, so the
//! local trait is what is named and its charge must stay. Fixtures: `soundness/cfgtwin/fx/lc1` (2021, executed ground truth) and `lc1e15` (2015).
//! `CANDOR_BOUNDSET_OFF=1` restores main's answer.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};

#[derive(Debug, Default, Clone)]
struct Row {
    inferred: Vec<String>,
    invisible: Vec<String>,
    keys: Vec<String>,
}
type Rows = BTreeMap<String, Row>;

static N: AtomicUsize = AtomicUsize::new(0);

fn fx(cell: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../soundness/cfgtwin/fx").join(cell)
}

/// Scan `dir` (with `deps`, report files, as `CANDOR_DEPS`); the rows, and the report files written (for a later chain).
fn scan(dir: &Path, deps: &[PathBuf], env: &[(&str, &str)]) -> (Rows, Vec<PathBuf>) {
    let d = std::env::temp_dir().join(format!("candor-l1b-{}-{}", std::process::id(), N.fetch_add(1, Ordering::SeqCst)));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    let out = d.join("out");
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_candor-scan"));
    cmd.arg(dir).arg("--out").arg(out.join("r"));
    for k in ["CANDOR_DEPS", "CANDOR_CONFIG", "CANDOR_POLICY", "CANDOR_R1657BP_OFF", "CANDOR_R1757_OFF", "CANDOR_BOUNDSET_OFF"] {
        cmd.env_remove(k);
    }
    if !deps.is_empty() {
        cmd.env("CANDOR_DEPS", std::env::join_paths(deps).unwrap());
    }
    for (k, v) in env {
        cmd.env(k, v);
    }
    cmd.env("CANDOR_CACHE_DIR", d.join("c"));
    let o = cmd.output().expect("run candor-scan");
    let rc = o.status.code().unwrap_or(-1);
    assert!(rc == 0 || rc == 1, "scan failed: {}", String::from_utf8_lossy(&o.stderr));
    let mut r = Rows::new();
    let mut reps = Vec::new();
    let keep = std::env::temp_dir().join(format!("candor-l1b-rep-{}-{}", std::process::id(), N.fetch_add(1, Ordering::SeqCst)));
    std::fs::create_dir_all(&keep).unwrap();
    for e in std::fs::read_dir(&out).unwrap() {
        let p = e.unwrap().path();
        let n = p.file_name().unwrap().to_string_lossy().to_string();
        if !n.ends_with(".scan.json") {
            continue;
        }
        let k = keep.join(&n);
        std::fs::copy(&p, &k).unwrap();
        reps.push(k);
        let v: serde_json::Value = serde_json::from_slice(&std::fs::read(&p).unwrap()).unwrap();
        for f in v["functions"].as_array().unwrap() {
            let s = |k: &str| -> Vec<String> {
                f[k].as_array().map(|a| a.iter().filter_map(|x| x.as_str().map(String::from)).collect()).unwrap_or_default()
            };
            r.insert(f["fn"].as_str().unwrap().to_string(), Row { inferred: s("inferred"), invisible: s("invisible"), keys: s("dispatchesOn") });
        }
    }
    let _ = std::fs::remove_dir_all(&d);
    (r, reps)
}

/// Remove the copied report files `scan` kept for a chain.
fn drop_reps(reps: &[PathBuf]) {
    for d in reps.iter().filter_map(|p| p.parent()) {
        let _ = std::fs::remove_dir_all(d);
    }
}

fn cell(c: &str) -> Rows {
    let (r, reps) = scan(&fx(c), &[], &[]);
    drop_reps(&reps);
    r
}

/// The cell scanned CHAINED over its dependency's report (`run.py`'s `dep` chain).
fn chained(c: &str, dep: &str) -> Rows {
    let (_, reps) = scan(&fx(c).join(dep), &[], &[]);
    let (r, r2) = scan(&fx(c), &reps, &[]);
    drop_reps(&reps);
    drop_reps(&r2);
    r
}

fn row(r: &Rows, probe: &str) -> Row {
    r.iter()
        .find(|(k, _)| k.as_str() == probe || k.ends_with(&format!("::{probe}")))
        .map(|(_, v)| v.clone())
        .unwrap_or_default()
}
fn fs(r: &Rows, p: &str) -> bool {
    row(r, p).inferred.iter().any(|x| x == "Fs")
}
fn unknown(r: &Rows, p: &str) -> bool {
    row(r, p).inferred.iter().any(|x| x == "Unknown")
}
fn key(r: &Rows, p: &str, k: &str) -> bool {
    row(r, p).keys.iter().any(|x| x == k)
}


#[test]
fn lc1_leading_colon_supertrait_is_the_extern_crate_not_the_local_module() {
    let un = cell("lc1");
    assert!(row(&un, "lc1_colon_q").invisible.iter().any(|x| x == "fxdep_lc1"), "{:?}", row(&un, "lc1_colon_q"));
    for r in [un, chained("lc1", "fxdep_lc1")] {
        // executed pure: the program instantiates only the DEPENDENCY's implementor, which does not write
        for p in ["lc1_colon_q", "lc1_colonm_q"] {
            assert!(!fs(&r, p), "{p}: the local trait's writing implementor must not be charged: {:?}", row(&r, p));
            assert!(unknown(&r, p), "{p}: the dependency trait is disclosed, not read pure: {:?}", row(&r, p));
        }
        assert!(key(&r, "lc1_colon_q", "fxdep_lc1#Sup::lsf"), "{:?}", row(&r, "lc1_colon_q"));
        assert!(key(&r, "lc1_colonm_q", "fxdep_lc1#Sup::lsm"), "{:?}", row(&r, "lc1_colonm_q"));
        // the same shape over a dependency implementor that writes: disclosed (the dependency's body is not here to read)
        for p in ["lc1_colon_e", "lc1_colonm_e"] {
            assert!(unknown(&r, p), "{p}: {:?}", row(&r, p));
        }
    }
}

#[test]
fn lc1_the_local_reading_stays_where_it_is_the_local_trait() {
    // the second fixture: the same shape WITHOUT the leading colon names the local trait, whose writing implementor is instantiated
    let r = cell("lc1");
    assert!(fs(&r, "lc1_local_e"), "{:?}", row(&r, "lc1_local_e"));
    // edition 2015: a leading `::` is the crate root, so `::fxdep_lc1e15::Sup` IS the local `mod fxdep_lc1e15`'s trait
    let r = cell("lc1e15");
    assert!(fs(&r, "lc1e15_root_e"), "{:?}", row(&r, "lc1e15_root_e"));
}

#[test]
fn lc1_boundset_off_restores_main() {
    let (r, reps) = scan(&fx("lc1"), &[], &[("CANDOR_BOUNDSET_OFF", "1")]);
    drop_reps(&reps);
    assert!(fs(&r, "lc1_colon_q"), "{:?}", row(&r, "lc1_colon_q"));
}
