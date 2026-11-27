//! Lane rust-bound-l1b — SOUNDNESS R1953, R1964, R1975, R1977, R1980: the REMOVALS of bound/projection dispatch. Each test pins a charge,
//! key or hedge main published that cannot be what runs (removed), AND, beside it, the disclosure or charge that must remain (the second
//! fixture). Fixtures are the checked-in cells `soundness/cfgtwin/fx/lb1`–`lb4`, `bm1`, `bl7`: every `_e` probe writes its marker when run
//! (`run.py --exec`) and every `_q` probe is pure. The `main` column of `soundness/cfgtwin/expect.tsv` records what each test fails on.
//! `CANDOR_BOUNDSET_OFF=1` restores main's answer for every one (asserted below).

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


fn none_of(r: &Rows, p: &str) -> bool {
    let x = row(r, p);
    x.inferred.is_empty() && x.invisible.is_empty() && x.keys.is_empty()
}

#[test]
fn r1977_dependency_supertrait_sharing_a_local_leaf_is_not_the_local_trait() {
    // `trait LSub: fxdep::Sup {}` beside a crate-local `m::Sup`: `t.lsm()` was dispatched over the LOCAL trait's implementors (a pure
    // program charged `Fs`). Every method spelling now keys the dependency's trait and discloses; none reads the local trait's body.
    let r = cell("lb1");
    for p in ["lb1_meth_q", "lb1_meth_e", "lb1_methuse_q", "lb1_where_q", "lb1_dyn_q", "lb1_dyn_e", "lb1_selfm_q", "lb1_selfm_e"] {
        assert!(!fs(&r, p), "{p} still charged through the same-leaf local trait: {:?}", row(&r, p));
        assert!(unknown(&r, p) && key(&r, p, "fxdep_lb1#Sup::lsm"), "{p} must key the dependency trait and disclose: {:?}", row(&r, p));
        assert!(!key(&r, p, "cf_lb1#m_lb1::Sup::lsm"), "{p} keyed the same-leaf local trait: {:?}", row(&r, p));
    }
    // the projection spellings (`T::Item::lsf()`, `x.lsm()` with `x: T::Item`): disclosed, never silent
    for p in ["lb1_proj_q", "lb1_proj_e", "lb1_projm_q", "lb1_projm_e"] {
        assert!(!fs(&r, p) && unknown(&r, p), "{p}: {:?}", row(&r, p));
    }
    // the second fixture: a subtrait of the LOCAL `m::Sup` keeps its local dispatch on every spelling
    for p in ["lb1_local_e", "lb1_localpath_e", "lb1_localdyn_e"] {
        assert!(fs(&r, p), "{p} lost the local trait's real dispatch: {:?}", row(&r, p));
    }
    // and chained, the dependency's key is what the join answers: still disclosed
    let c = chained("lb1", "fxdep_lb1");
    for p in ["lb1_meth_e", "lb1_selfm_e"] {
        assert!(!fs(&c, p) && unknown(&c, p), "{p} chained: {:?}", row(&c, p));
    }
}

#[test]
fn r1977_criterion_shape_method_through_its_own_dependency_supertrait_is_keyed() {
    // `use dep::float; trait Float: float::Float {}` (the trait shares the leaf of its own dependency supertrait), `t.cm()`: SILENT on
    // 0.43.0 and main (the leaf walk read the supertrait as the trait itself); now the dependency key and the disclosure
    let r = cell("lb4");
    for p in ["lb4_selfm_e", "lb4_selfm_q"] {
        assert!(key(&r, p, "fxdep_lb4#float::Float::cm") && !row(&r, p).invisible.is_empty(), "{p}: {:?}", row(&r, p));
    }
}

#[test]
fn r1953_projection_dispatch_is_over_the_bound_not_the_declaring_supertrait() {
    let r = cell("lb2");
    // an implementor of the declaring supertrait only (it cannot be `T::Item`) is no longer charged
    for p in ["lb2_supcha_q", "lb2_twohop_q", "lb2_inline_q", "lb2_implcl_q", "lb2_thirteen_q"] {
        assert!(!fs(&r, p) && !unknown(&r, p), "{p}: {:?}", row(&r, p));
    }
    // the second fixture: every spelling of a real implementor of the bound stays charged — a writing implementor of the bound, a
    // BLANKET impl of the bound (may cover; also over a second marker), two spellings of the type, a generic implementor, a reference
    for p in ["lb2_cover_e", "lb2_blanket_e", "lb2_blanket2_e", "lb2_spell_e", "lb2_generic_e", "lb2_ref_e"] {
        assert!(fs(&r, p), "{p} lost a real implementor: {:?}", row(&r, p));
    }
    // more than twelve implementors of the bound: the hedge, never a pick
    assert!(unknown(&r, "lb2_lt13_e"), "lb2_lt13_e: {:?}", row(&r, "lb2_lt13_e"));
    // the declaring trait's key is kept on every one
    assert!(key(&r, "lb2_supcha_q", "cf_lb2#Base1::lf"), "{:?}", row(&r, "lb2_supcha_q"));
}

#[test]
fn r1964_no_dependency_key_for_a_member_a_local_bound_declares() {
    let r = cell("lb3");
    for p in ["lb3_mixed_e", "lb3_supl_e", "lb3_where_e", "lb3_impl_e", "lb3_bare_e"] {
        assert!(fs(&r, p) && !key(&r, p, "fxdep_lb3#Dt::lf"), "{p}: {:?}", row(&r, p));
    }
    // the seeded C3: the dependency key WAS the row's only `invisible`; the local key remains, the row is not left with nothing
    let x = row(&r, "lb3_pure_q");
    assert!(x.invisible.is_empty() && key(&r, "lb3_pure_q", "cf_lb3#Lt2::lf") && !key(&r, "lb3_pure_q", "fxdep_lb3#Dt::lf"), "{x:?}");
    // >12 implementors: the local hedge stays
    assert!(unknown(&r, "lb3_many_e") && !key(&r, "lb3_many_e", "fxdep_lb3#Dt::lf"), "{:?}", row(&r, "lb3_many_e"));
    // KEEP: the member is a DEPENDENCY member (of the dependency bound itself, of a dependency supertrait of the local bound, of a
    // dependency supertrait sharing a local leaf), and the method spelling (autoref can separate two same-named members, R551)
    assert!(key(&r, "lb3_depmem_e", "fxdep_lb3#Dt::f"), "{:?}", row(&r, "lb3_depmem_e"));
    assert!(key(&r, "lb3_depsup_e", "fxdep_lb3#Dt::sf"), "{:?}", row(&r, "lb3_depsup_e"));
    assert!(key(&r, "lb3_collide_e", "fxdep_lb3#Dt::lf"), "{:?}", row(&r, "lb3_collide_e"));
    assert!(key(&r, "lb3_meth_e", "fxdep_lb3#Dt::lm"), "{:?}", row(&r, "lb3_meth_e"));
    let r = cell("bm1");
    assert!(fs(&r, "bm1_b4_mixed_local_e") && !key(&r, "bm1_b4_mixed_local_e", "fxdep_bm1#Dt::lf"), "{:?}", row(&r, "bm1_b4_mixed_local_e"));
}

#[test]
fn r1975_r1980_supertrait_origin_through_alias_and_reexport_tables() {
    let r = cell("lb4");
    // R1975: a `use .. as` alias of a LOCAL trait as a supertrait (silent on 0.43.0 and main), path and method spellings
    for p in ["lb4_alias_e", "lb4_aliasm_e", "lb4_reexl_e"] {
        assert!(fs(&r, p) && key(&r, p, &format!("cf_lb4#inner4::Sup::{}", if p.ends_with("m_e") { "lsm" } else { "lsf" })), "{p}: {:?}", row(&r, p));
    }
    // R1980: a dependency supertrait (a KNOWN dependency root) sharing the local trait's leaf is the dependency's: keyed, not declined
    for p in ["lb4_self_e", "lb4_self_q"] {
        let x = row(&r, p);
        assert!(!unknown(&r, p) && key(&r, p, "fxdep_lb4#float::Float::cast") && !x.invisible.is_empty(), "{p}: {x:?}");
    }
    // a dependency trait RE-EXPORTED through a local module beside a same-leaf local trait: the dependency's key (main: no key)
    for p in ["lb4_reexp_e", "lb4_reexp_q"] {
        assert!(!fs(&r, p) && unknown(&r, p) && key(&r, p, "fxdep_lb4#Sup::lsf"), "{p}: {:?}", row(&r, p));
    }
    let r = cell("bl7");
    assert!(fs(&r, "bl7_alias_e"), "{:?}", row(&r, "bl7_alias_e"));
    assert!(key(&r, "bl7_reexp_q", "fxdep_bl7#Sup::lsf") && !fs(&r, "bl7_reexp_q"), "{:?}", row(&r, "bl7_reexp_q"));
}

#[test]
fn boundset_off_restores_main() {
    let off = [("CANDOR_BOUNDSET_OFF", "1")];
    let (r, reps) = scan(&fx("lb1"), &[], &off);
    drop_reps(&reps);
    assert!(fs(&r, "lb1_meth_q") && key(&r, "lb1_meth_q", "cf_lb1#m_lb1::Sup::lsm"), "{:?}", row(&r, "lb1_meth_q"));
    let (r, reps) = scan(&fx("lb2"), &[], &off);
    drop_reps(&reps);
    assert!(fs(&r, "lb2_supcha_q"), "{:?}", row(&r, "lb2_supcha_q"));
    let (r, reps) = scan(&fx("lb3"), &[], &off);
    drop_reps(&reps);
    assert!(key(&r, "lb3_pure_q", "fxdep_lb3#Dt::lf"), "{:?}", row(&r, "lb3_pure_q"));
    let (r, reps) = scan(&fx("lb4"), &[], &off);
    drop_reps(&reps);
    assert!(none_of(&r, "lb4_alias_e") && unknown(&r, "lb4_self_e"), "{:?} {:?}", row(&r, "lb4_alias_e"), row(&r, "lb4_self_e"));
}
