//! Lane rust-bound-l1a — SOUNDNESS R1952, R1957 (N1), R1958 (N2), R1959 (N3), R1960 (N4), R1961 (N5): the path spellings of a trait member
//! reached through a bound or a projection that formed NOTHING (no edge, no key, no `invisible`) where a sibling spelling of the same call
//! charged or disclosed it. Fixtures are the checked-in attack matrix (`soundness/cfgtwin/fx`, cells `bm1`, `bm2`, `bd1`, `wg1`, `wg2`, `wg3`,
//! `wg5`, `la1`); every `_e` probe writes its marker when run (`run.py --exec`) and every `_q` probe is pure. Every probe asserted here reads
//! `-` on v0.43.0 AND on main (the `pub` / `main` columns of `soundness/cfgtwin/expect.tsv`), so each test fails on the code it was written
//! against. Every change is ADDITIVE: an edge, a key or a hedge at a site that had none.

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
    let d = std::env::temp_dir().join(format!("candor-l1a-{}-{}", std::process::id(), N.fetch_add(1, Ordering::SeqCst)));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    let out = d.join("out");
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_candor-scan"));
    cmd.arg(dir).arg("--out").arg(out.join("r"));
    for k in ["CANDOR_DEPS", "CANDOR_CONFIG", "CANDOR_POLICY", "CANDOR_R1657BP_OFF", "CANDOR_R1757_OFF"] {
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
    let keep = std::env::temp_dir().join(format!("candor-l1a-rep-{}-{}", std::process::id(), N.fetch_add(1, Ordering::SeqCst)));
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
fn r1957_assoc_fn_declared_in_a_local_supertrait_dispatches_over_the_bound() {
    // `T::lsf()` under `T: LSub`, `trait LSub: LSup`, `fn lsf();` in LSup: bounded CHA over LSub's implementors, keyed on the DECLARING trait
    let r = cell("bm1");
    assert!(fs(&r, "bm1_b5_localsup_e") && key(&r, "bm1_b5_localsup_e", "cf_bm1#LSup::lsf"), "bm1: {:?}", row(&r, "bm1_b5_localsup_e"));
    // the `where`, impl-level and generic-METHOD spellings (Fable bd1)
    let r = cell("bd1");
    for p in ["bd1_where_e", "bd1_implgen_e", "bd1_method_gen_e"] {
        assert!(fs(&r, p) && key(&r, p, "cf_bd1#LSup_bd1::lsf"), "{p}: {:?}", row(&r, p));
    }
    // two supertrait hops, and the projection form (`type A: LSub;`, `T::A::lsf()`)
    let r = cell("la1");
    for p in ["la1_n1two_e", "la1_n1proj_e"] {
        assert!(fs(&r, p) && key(&r, p, "cf_la1#LSup::lsf"), "{p}: {:?}", row(&r, p));
    }
    // a blanket impl of the bound, of the declaring trait, of both (W-g2): the method spelling's answer, through R1723's blanket arm
    let r = cell("wg2");
    for p in ["wg2_blanketB_e", "wg2_blanketD_e", "wg2_blanketBoth_e"] {
        assert!(fs(&r, p), "{p}: {:?}", row(&r, p));
    }
    // thirteen implementors (W-g1): over the bounded-CHA cap, the true hedge with the key, never silence
    let r = cell("wg1");
    assert!(unknown(&r, "wg1_suppath_e") && key(&r, "wg1_suppath_e", "cf_wg1#LSup::lsf"), "wg1: {:?}", row(&r, "wg1_suppath_e"));
}

#[test]
fn r1958_dependency_supertrait_of_a_local_bound_is_keyed_and_disclosed() {
    // `T::sf()` under `T: LSubD`, `trait LSubD: dep::SupD {}`: the method spelling's key, and its hedge (this crate implements SupD itself)
    let r = cell("bm1");
    let x = row(&r, "bm1_b5_localsub_depsup_e");
    assert!(key(&r, "bm1_b5_localsub_depsup_e", "fxdep_bm1#SupD::sf") && x.invisible.iter().any(|i| i == "fxdep_bm1"), "bm1: {x:?}");
    assert!(unknown(&r, "bm1_b5_localsub_depsup_e"), "bm1 hedge: {x:?}");
    let r = cell("la1");
    assert!(key(&r, "la1_n2proj_e", "fxdep_la1#SupD::sf"), "la1_n2proj_e: {:?}", row(&r, "la1_n2proj_e"));
    // the GUARD: a second LOCAL bound declares `sf`, so the dependency supertrait (which does not) must not be keyed
    let g = row(&r, "la1_n2guard_e");
    assert!(fs(&r, "la1_n2guard_e") && !g.keys.iter().any(|k| k.contains("NoSf")), "la1_n2guard_e: {g:?}");
}

#[test]
fn r1959_qualified_projection_is_read_through_the_written_trait() {
    // `<T as HasL>::A::lf()` with `HasL { type A: Lt; }`
    let r = cell("bm2");
    assert!(fs(&r, "bm2_localdecl_qa_e") && key(&r, "bm2_localdecl_qa_e", "cf_bm2#Lt::lf"), "bm2: {:?}", row(&r, "bm2_localdecl_qa_e"));
    let r = cell("la1");
    // through a STD trait with a `where` bound on the projection, and through a trait that is not a DIRECT bound of `T`
    for p in ["la1_qa_std_e", "la1_qa_sup_e", "la1_qa_bind_e", "la1_qa_where_e"] {
        assert!(fs(&r, p), "{p}: {:?}", row(&r, p));
    }
    // a DEPENDENCY trait keeps the qself route's key (the trait that declares `A`)
    assert!(key(&r, "la1_qa_dep_e", "fxdep_la1#HasA::A::f"), "la1_qa_dep_e: {:?}", row(&r, "la1_qa_dep_e"));
    // thirteen implementors: the hedge
    let r = cell("wg1");
    assert!(unknown(&r, "wg1_qa_e"), "wg1_qa_e: {:?}", row(&r, "wg1_qa_e"));
}

#[test]
fn r1960_qualified_std_trait_member_charges_the_caller_like_ufcs() {
    // `<T as Iterator>::next(&mut t)` called with an effectful iterator: the caller is charged, as for `Iterator::next(&mut t)` and `t.next()`
    let r = cell("bm1");
    assert!(fs(&r, "bm1_b3_qself_next_e"), "bm1: {:?}", row(&r, "bm1_b3_qself_next_e"));
    // the I/O twin `<R as std::io::Read>::read(&mut r, ..)` (R1318's UFCS hook)
    let r = cell("la1");
    assert!(fs(&r, "la1_qself_read_e") && fs(&r, "la1_ufcs_read_e"), "la1: {:?} / {:?}", row(&r, "la1_qself_read_e"), row(&r, "la1_ufcs_read_e"));
}

#[test]
fn r1961_written_std_binding_enters_the_std_type_route() {
    // `T: Iterator<Item = String>`, `use dep::Dt; T::Item::f()` runs the dependency's `impl Dt for String`: chained, the join answers it as it
    // answers the concrete spelling `String::f()`. Unchained the marker adds nothing (the published behaviour).
    let c = chained("la1", "fxdep_la1");
    assert!(fs(&c, "la1_bindstd_e") && fs(&c, "la1_concrete_e"), "chained: {:?} / {:?}", row(&c, "la1_bindstd_e"), row(&c, "la1_concrete_e"));
    let r = cell("la1");
    assert!(!fs(&r, "la1_bindstd_e"), "unchained: {:?}", row(&r, "la1_bindstd_e"));
}

#[test]
fn r1952_dependency_trait_named_like_a_std_trait_is_a_dependency_bound() {
    let r = cell("wg3");
    for (p, k) in [("wg3_err_proj_e", "fxdep_wg3#Error::ef"), ("wg3_read_proj_e", "fxdep_wg3#Read::rf"), ("wg3_ptr_proj_e", "fxdep_wg3#Pointee::pf")] {
        let x = row(&r, p);
        assert!(key(&r, p, k) && x.invisible.iter().any(|i| i == "fxdep_wg3"), "{p}: {x:?}");
    }
    let r = cell("la1");
    for p in ["la1_r1952_bare_e", "la1_r1952_bare2_e"] {
        assert!(key(&r, p, "fxdep_la1#Error::ef"), "{p}: {:?}", row(&r, p));
    }
}

#[test]
fn r1957_r1958_keys_join_a_two_hop_consumer() {
    // W-g5: the consumer implements the DECLARING traits effectfully; the producer is generic over a subtrait. The producer's new keys
    // (`LSup::lsf` for `X::lsf()` under `X: LSub`; `BaseS::s_fn` for `X::s_fn()` under `X: LSubD`, `LSubD: dep::BaseS`) are what the
    // consumer's chained join charges. Both read `-` chained on v0.43.0 and main.
    let base = fx("wg5");
    let (_, d) = scan(&base.join("fxdep_wg5d"), &[], &[]);
    let (_, m) = scan(&base.join("fxdep_wg5p"), &d, &[]);
    let all: Vec<PathBuf> = d.iter().chain(m.iter()).cloned().collect();
    let (c, c2) = scan(&base, &all, &[]);
    drop_reps(&all);
    drop_reps(&c2);
    for p in ["wg5_lsup_e", "wg5_lsdpath_e"] {
        assert!(fs(&c, p), "{p}: {:?}", row(&c, p));
    }
}

#[test]
fn round2_a_supertrait_is_classified_by_its_written_root_not_its_leaf() {
    // Fable bl1: `trait LSub: fxdep::Sup {}` beside a crate-local `m::Sup` whose implementor writes. `g::<Quiet>()` runs the DEPENDENCY's
    // pure `lsf` (executed: none). Round 1 dispatched over the LOCAL `Sup` and charged `Fs` with a key naming it — a fabrication.
    let r = cell("bl1");
    for p in ["bl1_collide_q", "bl1_collide_e"] {
        let x = row(&r, p);
        assert!(!fs(&r, p), "{p} charged through the crate-local trait of the same leaf: {x:?}");
        assert!(!x.keys.iter().any(|k| k.contains("m_bl1")), "{p} keyed the local trait: {x:?}");
        assert!(key(&r, p, "fxdep_bl1#Sup::lsf"), "{p}: the dependency supertrait is keyed: {x:?}");
    }
    // the std twin: no key naming the local `m::Default`
    let r = cell("bl1b");
    let x = row(&r, "bl1b_stdcollide_q");
    assert!(!x.keys.iter().any(|k| k.contains("m_bl1b")), "bl1b: {x:?}");
    // bl2: the dependency supertrait written by PATH with no `use` is keyed (it formed nothing in round 1)
    let r = cell("bl2");
    for p in ["bl2_inherit_e", "bl2_declares_e"] {
        assert!(key(&r, p, "fxdep_bl2#SupD::sf"), "{p}: {:?}", row(&r, p));
    }
    // la2: a supertrait rooted at a module this module declares (`use id_private::IdPrivate; trait Id: IdPrivate {}`) stays LOCAL
    let r = cell("la2");
    for p in ["la2_localmod_proj_e", "la2_localmod_path_e"] {
        assert!(fs(&r, p) && key(&r, p, "cf_la2#id_private::IdPrivate::new_la2"), "{p}: {:?}", row(&r, p));
    }
    let c = chained("bl2", "fxdep_bl2");
    for p in ["bl2_inherit_e", "bl2_declares_e"] {
        assert!(fs(&c, p), "{p} chained: {:?}", row(&c, p));
    }
}

#[test]
fn round3_a_supertrait_is_local_only_when_its_written_path_names_the_declaration() {
    // Fable bl7: `mod reexp { pub use fxdep::Sup; } trait LSubR: reexp::Sup {}` beside a local `inner::Sup` whose implementor writes. The
    // path's ROOT is a local module but it NAMES the dependency trait; round 2 read it as the local one and charged an executed-pure probe.
    let r = cell("bl7");
    for p in ["bl7_reexp_q", "bl7_reexp_e"] {
        let x = row(&r, p);
        assert!(!fs(&r, p) && !x.keys.iter().any(|k| k.contains("inner_bl7")), "{p} charged through the local `inner::Sup`: {x:?}");
        assert!(unknown(&r, p), "{p} must be disclosed: {x:?}");
    }
    // paths that DO name the local declaration keep the resolution: through `self::`, `use super::inner`, and a parent glob
    for p in ["bl7_self_e", "bl7_moduse_e", "bl7_glob_e"] {
        assert!(fs(&r, p) && key(&r, p, "cf_bl7#inner_bl7::Sup::lsf"), "{p}: {:?}", row(&r, p));
    }
}
