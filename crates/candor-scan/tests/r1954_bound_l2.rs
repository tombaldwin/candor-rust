//! Lane rust-bound-l2 — SOUNDNESS R1954 / R1966 / R1976 / R1989 / R1968: a `dispatchesOn` key names the trait that DECLARES the member.
//! Fixtures are the checked-in cells `soundness/cfgtwin/fx/l2k` (one hop, the cell's own `Cargo.lock`), `l2w` (consumer -> producer with
//! its own `Cargo.lock` -> dependency, W-g5), `lb3`, `bm1` and `wg5g`: every `_e` probe writes its marker when run (`run.py --exec`) and
//! every `_q` probe is pure. The `main` column of `soundness/cfgtwin/expect.tsv` records what each test fails on. `CANDOR_R1954_OFF=1`,
//! `CANDOR_R1989_OFF=1` and `CANDOR_R1968_OFF=1` restore main's answer (asserted below), the proof that the fixtures reach the change.

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
    let d = std::env::temp_dir().join(format!("candor-l2-{}-{}", std::process::id(), N.fetch_add(1, Ordering::SeqCst)));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    let out = d.join("out");
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_candor-scan"));
    cmd.arg(dir).arg("--out").arg(out.join("r"));
    for k in ["CANDOR_DEPS", "CANDOR_CONFIG", "CANDOR_POLICY", "CANDOR_R1954_OFF", "CANDOR_R1989_OFF", "CANDOR_R1968_OFF", "CANDOR_BOUNDSET_OFF"] {
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
    let keep = std::env::temp_dir().join(format!("candor-l2-rep-{}-{}", std::process::id(), N.fetch_add(1, Ordering::SeqCst)));
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

fn drop_reps(reps: &[PathBuf]) {
    for d in reps.iter().filter_map(|p| p.parent()) {
        let _ = std::fs::remove_dir_all(d);
    }
}

fn cell(c: &str, env: &[(&str, &str)]) -> Rows {
    let (r, reps) = scan(&fx(c), &[], env);
    drop_reps(&reps);
    r
}

/// The cell scanned CHAINED over its dependency's report (`run.py`'s `dep` chain).
fn chained(c: &str, dep: &str, env: &[(&str, &str)]) -> Rows {
    let (_, reps) = scan(&fx(c).join(dep), &[], env);
    let (r, r2) = scan(&fx(c), &reps, env);
    drop_reps(&reps);
    drop_reps(&r2);
    r
}

/// `run.py`'s `twohop` chain: the producer `mid` scanned over the dependency's report, the consumer (the cell) over both.
fn twohop(c: &str, dep: &str, mid: &str, env: &[(&str, &str)]) -> Rows {
    let (_, dreps) = scan(&fx(c).join(dep), &[], env);
    let (_, mreps) = scan(&fx(c).join(mid), &dreps, env);
    let all: Vec<PathBuf> = dreps.iter().chain(mreps.iter()).cloned().collect();
    let (r, r2) = scan(&fx(c), &all, env);
    drop_reps(&dreps);
    drop_reps(&mreps);
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
fn keys(r: &Rows, p: &str) -> Vec<String> {
    let mut k = row(r, p).keys;
    k.sort();
    k
}

const OFF: &[(&str, &str)] = &[("CANDOR_R1954_OFF", "1")];

#[test]
fn r1954_key_names_the_declaring_trait_from_the_lock_pinned_source() {
    let r = cell("l2k", &[]);
    let want = [
        ("l2k_suppath_e", "fxdep_l2kd#BaseS::s_fn"),          // `X: SubS` (`SubS: BaseS`), path spelling
        ("l2k_supmeth_e", "fxdep_l2kd#BaseS::s_go"),          // the method spelling
        ("l2k_proj_e", "fxdep_l2kd#BaseS::s_fn"),             // `where I::Item: SubS`, the projection spelling
        ("l2k_deep_e", "fxdep_l2kd#m::Deep::df"),             // two levels, a supertrait written by its module path
        ("l2k_far_e", "fxdep_l2kf#Far::ff"),                  // a supertrait of ANOTHER package, read in that package's source
        ("l2k_two_e", "fxdep_l2kd#SupA::sa"),                 // R1976: `LSub2: SupA + SupB`, only `SupA` declares `sa`
        ("l2k_mixed_e", "fxdep_l2kd#SupD::sf"),               // R1989: `X: Dt + Lt5`, `Lt5: SupD`
    ];
    for (p, k) in want {
        assert_eq!(keys(&r, p), vec![k.to_string()], "{p}: {:?}", row(&r, p));
    }
    // controls: a key on the declaring trait is untouched; a trait a macro may declare is a doubt, so its key is kept as minted
    assert_eq!(keys(&r, "l2k_flat_e"), vec!["fxdep_l2kd#Flat::f_fn".to_string()]);
    assert_eq!(keys(&r, "l2k_made_e"), vec!["fxdep_l2kd#OverMade::mk".to_string()]);
    // the switch restores the keys as minted, and so does a missing lock (bm1: the same `SubD: SupD` shape, no Cargo.lock)
    let o = cell("l2k", OFF);
    assert_eq!(keys(&o, "l2k_suppath_e"), vec!["fxdep_l2kd#SubS::s_fn".to_string()], "{:?}", row(&o, "l2k_suppath_e"));
    assert_eq!(keys(&o, "l2k_far_e"), vec!["fxdep_l2kd#OverFar::ff".to_string()]);
    assert!(keys(&o, "l2k_two_e").contains(&"fxdep_l2kd#SupB::sa".to_string()), "{:?}", row(&o, "l2k_two_e"));
    let b = cell("bm1", &[]);
    assert_eq!(keys(&b, "bm1_b5_depsup_e"), vec!["fxdep_bm1#SubD::sf".to_string()], "no lock: kept: {:?}", row(&b, "bm1_b5_depsup_e"));
}

#[test]
fn r1954_two_hop_consumer_joins_its_implementor_of_the_declaring_trait() {
    // W-g5: the consumer implements the DECLARING trait; the producer's key used to name the subtrait, so the join never matched and
    // a consumer whose implementor writes a file read `deny Fs` 0 two hops away
    let c = twohop("l2w", "fxdep_l2wd", "fxdep_l2wp", &[]);
    let o = twohop("l2w", "fxdep_l2wd", "fxdep_l2wp", OFF);
    for s in ["supmeth", "suppath", "proj", "deep", "deeppath", "far", "farpath", "reexp", "mixed", "gsuppath", "gproj"] {
        let (e, q) = (format!("l2w_{s}_e"), format!("l2w_{s}_q"));
        assert!(fs(&c, &e), "{e} not charged two hops away: {:?}", row(&c, &e));
        assert!(!fs(&c, &q), "{q} (pure twin) charged: {:?}", row(&c, &q));
        assert!(unknown(&c, &q) || !row(&c, &q).invisible.is_empty(), "{q} lost its disclosure: {:?}", row(&c, &q));
    }
    // the rewrite is what moved them: off, the subtrait-keyed spellings read no `Fs`
    for s in ["supmeth", "suppath", "deep", "far", "reexp"] {
        let e = format!("l2w_{s}_e");
        assert!(!fs(&o, &e), "{e} charged with the rewrite off: {:?}", row(&o, &e));
    }
    // `X: Dt + Lt5` is R1989's key (`SupD::sf`, beside `Dt::sf`): off together, the consumer reads no `Fs`
    let o2 = twohop("l2w", "fxdep_l2wd", "fxdep_l2wp", &[("CANDOR_R1954_OFF", "1"), ("CANDOR_R1989_OFF", "1")]);
    assert!(!fs(&o2, "l2w_mixed_e"), "{:?}", row(&o2, "l2w_mixed_e"));
    // controls charged on every arm: the flat member, the qself spelling that already names the declaring trait, and a member the
    // keyed trait declares itself (`OwnSub::o_go`, its supertrait `Own` declares one too)
    for p in ["l2w_flat_e", "l2w_supqself_e", "l2w_own_e"] {
        assert!(fs(&c, p) && fs(&o, p), "{p}: {:?} / off {:?}", row(&c, p), row(&o, p));
    }
}

#[test]
fn r1989_dependency_supertrait_of_a_local_bound_beside_a_dependency_bound() {
    // `T: dep::Dt + Lt5`, `Lt5: dep::SupD`, `T::sf()` with an in-crate implementor that writes: `[]` chained, `deny Unknown` 0 on main
    let r = chained("lb3", "fxdep_lb3", &[]);
    let x = row(&r, "lb3_depsup_e");
    assert!(unknown(&r, "lb3_depsup_e") && x.keys.contains(&"fxdep_lb3#SupD::sf".to_string()), "{x:?}");
    let o = chained("lb3", "fxdep_lb3", &[("CANDOR_R1989_OFF", "1")]);
    assert!(!unknown(&o, "lb3_depsup_e") && !row(&o, "lb3_depsup_e").keys.contains(&"fxdep_lb3#SupD::sf".to_string()), "{:?}", row(&o, "lb3_depsup_e"));
    // a std bound beside the local one is not admitted (no key can say a std trait declares it)
    let r = cell("lb3", &[]);
    assert!(fs(&r, "lb3_mixed_e"), "{:?}", row(&r, "lb3_mixed_e"));
}

#[test]
fn r1968_bound_brought_in_by_a_dependency_glob_is_keyed_or_disclosed() {
    // `use dep::*;` in the producer: the path, qself, projection and local-subtrait method spellings formed nothing (`-` chained)
    let c = twohop("wg5g", "fxdep_wg5gd", "fxdep_wg5gp", &[]);
    for p in ["wg5g_flatfn_e", "wg5g_supqself_e", "wg5g_lsdmeth_e"] {
        assert!(fs(&c, p), "{p}: {:?}", row(&c, p));
    }
    for p in ["wg5g_suppath_e", "wg5g_proj_e", "wg5g_flatfn_q", "wg5g_suppath_q", "wg5g_proj_q", "wg5g_lsdmeth_q"] {
        assert!(unknown(&c, p) && !fs(&c, &p.replace("_e", "_q")), "{p}: {:?}", row(&c, p));
    }
    let o = twohop("wg5g", "fxdep_wg5gd", "fxdep_wg5gp", &[("CANDOR_R1968_OFF", "1")]);
    for p in ["wg5g_flatfn_e", "wg5g_suppath_e", "wg5g_supqself_e", "wg5g_proj_e", "wg5g_lsdmeth_e"] {
        let x = row(&o, p);
        assert!(x.inferred.is_empty(), "{p} with R1968 off: {x:?}");
    }
}
