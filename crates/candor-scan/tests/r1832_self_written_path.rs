//! SOUNDNESS R1832 — `Self` INSIDE `impl LocalTrait for dep::Type` NAMES THE WRITTEN PATH, AS THE `use`-IMPORT SPELLING ALREADY DID.
//! SOUNDNESS R1833 — A NESTED fn's CALLABLE PARAMETER IS HEDGED AS A TOP-LEVEL fn's IS.
//! SOUNDNESS R1835 — AN UNCLASSIFIED JOIN DOES NOT TAKE A SAME-TAIL SIBLING TYPE's ROW.
//!
//! R1832: `impl Go for sqlx_core::d::Lone { fn go(self, p) { Self::tally_zz(self, p) } }` bound `Self` to the bare leaf `Lone`,
//! so the call named no unit and no dependency: `[]` chained over an executed read+write (fable-r1814merge f3,
//! rustagent-r1832 g2), on 0.42.0 and main. The same impl written `use sqlx_core::d::Lone; impl Go for Lone` read `Env,Fs`.
//!
//! R1833: `{ fn app<F: FnOnce(&str)>(f: F, p: &str) { f(p) } app(dep::wr_zz, p) }` read ABSENT (chained and unchained) over an
//! executed write; the same helper at the top level reads `Unknown` (`callback:`).
//!
//! R1835: `sqlx_core::c::Raw::tally_zz(..)` joined `a::Raw::tally_zz` (an inherent that only writes) by the tail `Raw::tally_zz`
//! and read `[Fs]`, while the `Tally` default that runs also reads the environment: `deny Env` 0.
//!
//! Each `CANDOR_R183x_OFF=1` restores the old ROW and is asserted here as the calibration (a gate run under a result-moving
//! switch refuses with exit 2, SPEC §3.4, so the OFF arms are read as rows, not gates). Fixture fns end `_zq` because
//! `deny <E> <fn>` matches by PREFIX.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Command;

fn bin() -> &'static str {
    env!("CARGO_BIN_EXE_candor-scan")
}

fn tmpdir(tag: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("candor-r1832-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn write(p: &Path, s: &str) {
    std::fs::create_dir_all(p.parent().unwrap()).unwrap();
    std::fs::write(p, s).unwrap();
}

fn pkg(root: &Path, name: &str, deps: &[&str], src: &str) -> PathBuf {
    let d = root.join(name);
    let deps: String = deps.iter().map(|x| format!("{x} = {{ path = \"../{x}\" }}\n")).collect();
    write(&d.join("Cargo.toml"), &format!("[package]\nname = \"{name}\"\nversion = \"0.1.0\"\nedition = \"2021\"\n[dependencies]\n{deps}"));
    write(&d.join("src/lib.rs"), src);
    d
}

const SWITCHES: [&str; 6] = ["CANDOR_DEPS", "CANDOR_CONFIG", "CANDOR_POLICY", "CANDOR_R1832_OFF", "CANDOR_R1833_OFF", "CANDOR_R1835_OFF"];

fn scan_cmd(dir: &Path, deps: Option<&Path>, env: &[(&str, &str)]) -> Command {
    let mut cmd = Command::new(bin());
    cmd.arg(dir.to_string_lossy().as_ref());
    for v in SWITCHES {
        cmd.env_remove(v);
    }
    if let Some(d) = deps {
        cmd.env("CANDOR_DEPS", d);
    }
    for (k, v) in env {
        cmd.env(k, v);
    }
    cmd
}

/// `deny <effects> <f>` over `dir` chained on `deps`: the exit code.
fn deny(dir: &Path, effects: &str, f: &str, deps: Option<&Path>, env: &[(&str, &str)]) -> i32 {
    let pol = dir.join(format!("policy-{}-{f}.txt", effects.replace(' ', "-")));
    write(&pol, &format!("deny {effects} {f}\n"));
    let mut cmd = scan_cmd(dir, deps, env);
    cmd.arg("--json").arg("--policy").arg(&pol);
    let o = cmd.output().expect("run candor-scan");
    let c = o.status.code().unwrap_or(-1);
    assert!(c == 0 || c == 1, "scan failed ({c}): {}", String::from_utf8_lossy(&o.stderr));
    c
}

type Rows = HashMap<String, (Vec<String>, Vec<String>)>;

/// Every row of the report: fn leaf → (`inferred`, `unknownWhy`). An ABSENT fn is absent from the map.
fn rows(dir: &Path, deps: Option<&Path>, env: &[(&str, &str)]) -> Rows {
    let mut cmd = scan_cmd(dir, deps, env);
    cmd.arg("--json");
    let o = cmd.output().expect("run candor-scan");
    let v: serde_json::Value = serde_json::from_slice(&o.stdout).expect("report json");
    let strs = |x: &serde_json::Value| -> Vec<String> {
        x.as_array().map(|a| a.iter().filter_map(|e| e.as_str().map(str::to_string)).collect()).unwrap_or_default()
    };
    v["functions"].as_array().unwrap().iter()
        .map(|r| {
            let f = r["fn"].as_str().unwrap_or("");
            (f.rsplit("::").next().unwrap_or(f).to_string(), (strs(&r["inferred"]), strs(&r["unknownWhy"])))
        })
        .collect()
}

fn eff<'a>(r: &'a Rows, f: &str) -> &'a [String] {
    r.get(f).map(|x| x.0.as_slice()).unwrap_or(&[])
}

fn concrete(r: &Rows, f: &str) -> Vec<String> {
    eff(r, f).iter().filter(|e| *e != "Unknown").cloned().collect()
}

/// The dependency, scanned into `reps`, plus the consumer `app` over `src`.
fn world(tag: &str, app_src: &str) -> (PathBuf, PathBuf) {
    let root = tmpdir(tag);
    let dep = pkg(
        &root,
        "sqlx_core",
        &[],
        "pub trait Fetch: Sized { fn fetch_all(self, p: &str) { let v = std::env::var(\"R1832_MARK\").unwrap_or_default(); std::fs::write(format!(\"{p}.fetch\"), v).unwrap(); } }\n\
         pub trait Tally: Sized { fn tally_zz(self, p: &str) { let v = std::env::var(\"R1832_MARK\").unwrap_or_default(); std::fs::write(format!(\"{p}.tally\"), v).unwrap(); } }\n\
         pub trait Quiet: Sized { fn hush_zz(self, p: &str) -> usize { p.len() } }\n\
         pub trait Mk { fn mk_zz(p: &str) { std::fs::write(format!(\"{p}.mk\"), \"m\").unwrap(); } }\n\
         pub fn wr_zz(p: &str) { std::fs::write(format!(\"{p}.wr\"), \"w\").unwrap(); }\n\
         pub mod d { pub struct Lone(pub String); pub fn lone(s: &str) -> Lone { Lone(s.to_string()) }\n\
          impl crate::Tally for Lone {} impl crate::Fetch for Lone {} impl crate::Quiet for Lone {} impl crate::Mk for Lone {} }\n\
         pub mod g { pub struct Gen<T>(pub T); impl<T> crate::Tally for Gen<T> {} }\n\
         pub mod a { pub struct Raw(pub String); pub fn raw_a(s: &str) -> Raw { Raw(s.to_string()) }\n\
          impl Raw { pub fn tally_zz(&self, p: &str) { std::fs::write(format!(\"{p}.a\"), &self.0).unwrap(); } } }\n\
         pub mod c { pub struct Raw(pub String); pub fn raw_c(s: &str) -> Raw { Raw(s.to_string()) } impl crate::Tally for Raw {} }\n",
    );
    let reps = root.join("reps");
    std::fs::create_dir_all(&reps).unwrap();
    let o = scan_cmd(&dep, None, &[]).arg("--out").arg(reps.join("sqlx_core")).output().unwrap();
    assert!(o.status.code().is_some_and(|c| (0..=2).contains(&c)), "{}", String::from_utf8_lossy(&o.stderr));
    let app = pkg(&root, "app", &["sqlx_core"], app_src);
    (app, reps)
}

const R1832_APP: &str = "#![allow(unused_imports)]\n\
    mod am {\n\
     use sqlx_core::{Tally, Fetch, Quiet, Mk};\n\
     pub trait A: Sized { fn a_path(self, p: &str); fn a_cls(self, p: &str); fn a_qself(self, p: &str); fn a_fnref(self, p: &str);\n\
      fn a_assoc(self, p: &str); fn a_sib(self, p: &str); fn a_pure(self, p: &str) -> usize; }\n\
     impl A for sqlx_core::d::Lone {\n\
      fn a_path(self, p: &str) { Self::tally_zz(self, p) }\n\
      fn a_cls(self, p: &str) { Self::fetch_all(self, p) }\n\
      fn a_qself(self, p: &str) { <Self>::tally_zz(self, p) }\n\
      fn a_fnref(self, p: &str) { let f = Self::tally_zz; f(self, p) }\n\
      fn a_assoc(self, p: &str) { Self::mk_zz(p) }\n\
      fn a_sib(self, p: &str) { Self::a_path(self, p) }\n\
      fn a_pure(self, p: &str) -> usize { Self::hush_zz(self, p) }\n\
     }\n\
    }\n\
    pub use am::A;\n\
    mod em { use sqlx_core::d; use sqlx_core::Tally; pub trait E: Sized { fn e_path(self, p: &str); } impl E for d::Lone { fn e_path(self, p: &str) { Self::tally_zz(self, p) } } }\n\
    pub use em::E;\n\
    mod dm { use sqlx_core::Tally; pub trait D: Sized { fn d_path(self, p: &str); } impl D for sqlx_core::g::Gen<u8> { fn d_path(self, p: &str) { Self::tally_zz(self, p) } } }\n\
    pub use dm::D;\n\
    pub mod lm { pub struct Loc2; impl Loc2 { pub fn hit2(p: &str) { std::fs::write(format!(\"{p}.hit2\"), \"h\").unwrap(); } } }\n\
    pub trait L1 { fn l_crate(p: &str); }\n\
    impl L1 for crate::lm::Loc2 { fn l_crate(p: &str) { Self::hit2(p) } }\n\
    pub mod sm { pub trait L3 { fn l_super(p: &str); } impl L3 for super::lm::Loc2 { fn l_super(p: &str) { Self::hit2(p) } } }\n\
    pub use sm::L3;\n\
    pub fn s_path_zq(p: &str) { A::a_path(sqlx_core::d::lone(\"q\"), p) }\n\
    pub fn s_cls_zq(p: &str) { A::a_cls(sqlx_core::d::lone(\"q\"), p) }\n\
    pub fn s_qself_zq(p: &str) { A::a_qself(sqlx_core::d::lone(\"q\"), p) }\n\
    pub fn s_fnref_zq(p: &str) { A::a_fnref(sqlx_core::d::lone(\"q\"), p) }\n\
    pub fn s_assoc_zq(p: &str) { A::a_assoc(sqlx_core::d::lone(\"q\"), p) }\n\
    pub fn s_sib_zq(p: &str) { A::a_sib(sqlx_core::d::lone(\"q\"), p) }\n\
    pub fn s_mod_zq(p: &str) { E::e_path(sqlx_core::d::lone(\"q\"), p) }\n\
    pub fn s_gen_zq(p: &str) { D::d_path(sqlx_core::g::Gen(0u8), p) }\n\
    pub fn s_nested_zq(p: &str) { trait Nn { fn nn(self, p: &str); } impl Nn for sqlx_core::d::Lone { fn nn(self, p: &str) { use sqlx_core::Tally; Self::tally_zz(self, p) } } Nn::nn(sqlx_core::d::lone(\"q\"), p) }\n\
    pub fn s_pure_zq(p: &str) -> usize { A::a_pure(sqlx_core::d::lone(\"q\"), p) }\n\
    pub trait Rr { fn r_a(p: &str); fn r_b(p: &str); }\n\
    impl Rr for core::ops::Range<u8> { fn r_a(p: &str) { Self::r_b(p) } fn r_b(p: &str) { std::fs::write(format!(\"{p}.rb\"), \"b\").unwrap(); } }\n\
    pub fn s_coresib_zq(p: &str) { <core::ops::Range<u8> as Rr>::r_a(p) }\n\
    mod gm { use sqlx_core::*; pub trait G9: Sized { fn g9_a(self, p: &str); } impl G9 for d::Lone { fn g9_a(self, p: &str) { Self::tally_zz(self, p) } } }\n\
    pub use gm::G9;\n\
    mod shm { use sqlx_core::d::Lone as Range; pub trait S11 { fn s11_a(p: &str); fn s11_b(p: &str); }\n\
     impl S11 for std::ops::Range<u8> { fn s11_a(p: &str) { Self::s11_b(p) } fn s11_b(p: &str) { std::fs::write(format!(\"{p}.s11\"), \"b\").unwrap(); } } }\n\
    pub use shm::S11;\n\
    pub fn s_glob_zq(p: &str) { G9::g9_a(sqlx_core::d::lone(\"q\"), p) }\n\
    pub fn s_shadow_zq(p: &str) { <std::ops::Range<u8> as S11>::s11_a(p) }\n\
    pub fn l_crate_zq(p: &str) { <crate::lm::Loc2 as L1>::l_crate(p) }\n\
    pub fn l_super_zq(p: &str) { <crate::lm::Loc2 as L3>::l_super(p) }\n";

#[test]
fn r1832_self_in_an_impl_written_on_a_dependency_path_reaches_the_dependency() {
    let (app, reps) = world("self", R1832_APP);
    let on = rows(&app, Some(&reps), &[]);
    let off = rows(&app, Some(&reps), &[("CANDOR_R1832_OFF", "1")]);
    // every one of these writes a file (and reads the environment) when run
    for f in ["s_path_zq", "s_cls_zq", "s_qself_zq", "s_fnref_zq", "s_sib_zq", "s_mod_zq", "s_gen_zq", "s_nested_zq", "s_glob_zq"] {
        let e = eff(&on, f);
        assert!(e.iter().any(|x| x == "Fs") && e.iter().any(|x| x == "Env"), "{f}: {e:?} (R1832 must reach the trait default)");
        assert!(!eff(&off, f).iter().any(|x| x == "Fs"), "{f}: the OFF switch must restore the silence ({:?})", eff(&off, f));
    }
    assert!(eff(&on, "s_assoc_zq").iter().any(|x| x == "Fs"), "s_assoc_zq: {:?}", eff(&on, "s_assoc_zq"));
    assert!(eff(&on, "s_cls_zq").iter().any(|x| x == "Db"), "the classifier still answers the classified member");
    // the gate flips on code that really performs the effect
    assert_eq!(deny(&app, "Fs", "s_path_zq", Some(&reps), &[]), 1);
    // the pure twin is never charged a concrete effect
    assert!(concrete(&on, "s_pure_zq").is_empty(), "s_pure_zq: {:?}", eff(&on, "s_pure_zq"));
    // a std/core-rooted self type keeps the leaf binding: `Self::r_b` still reaches the impl's own sibling (a first cut
    // bound `core::ops::Range` and the sibling edge was lost in every position, fx/g5 `core_*`)
    assert!(eff(&on, "s_coresib_zq").iter().any(|x| x == "Fs"), "s_coresib_zq: {:?}", eff(&on, "s_coresib_zq"));
    // …and where a `use` binds that leaf to ANOTHER type, the bare leaf (the impl's own unit key) is bound instead
    assert!(eff(&on, "s_shadow_zq").iter().any(|x| x == "Fs"), "s_shadow_zq: {:?}", eff(&on, "s_shadow_zq"));
    assert!(!eff(&off, "s_shadow_zq").iter().any(|x| x == "Fs"), "s_shadow_zq OFF: {:?}", eff(&off, "s_shadow_zq"));
    // local types written with a path are unchanged: `Self::hit2` still reaches the local body
    for f in ["l_crate_zq", "l_super_zq"] {
        assert_eq!(eff(&on, f), eff(&off, f), "{f}");
        assert!(eff(&on, f).iter().any(|x| x == "Fs"), "{f}: {:?}", eff(&on, f));
    }
}

const R1833_APP: &str = "#![allow(unused_imports)]\n\
    fn loc_eff(p: &str) { std::fs::write(format!(\"{p}.loc\"), \"l\").unwrap(); }\n\
    pub fn top_app<R, F: FnOnce(&str) -> R>(f: F, p: &str) -> R { f(p) }\n\
    pub fn n_free_zq(p: &str) { fn ap<R, F: FnOnce(&str) -> R>(f: F, p: &str) -> R { f(p) } ap(sqlx_core::wr_zz, p) }\n\
    pub fn n_meth_zq(p: &str) { fn ap<R, F: FnOnce(sqlx_core::d::Lone, &str) -> R>(f: F, p: &str) -> R { f(sqlx_core::d::lone(\"q\"), p) } ap(sqlx_core::d::Lone::tally_zz, p) }\n\
    pub fn n_ptr_zq(p: &str) { fn ap(f: fn(&str), p: &str) { f(p) } ap(sqlx_core::wr_zz, p) }\n\
    pub fn n_dyn_zq(p: &str) { fn ap(f: &dyn Fn(&str), p: &str) { f(p) } ap(&sqlx_core::wr_zz, p) }\n\
    pub fn n_local_zq(p: &str) { fn ap<R, F: FnOnce(&str) -> R>(f: F, p: &str) -> R { f(p) } ap(crate::loc_eff, p) }\n\
    pub fn n_nocall_zq(p: &str) -> usize { fn ap<F: FnOnce(&str)>(_f: F, p: &str) -> usize { p.len() } ap(sqlx_core::wr_zz, p) }\n\
    pub fn t_free_zq(p: &str) { crate::top_app(sqlx_core::wr_zz, p) }\n\
    pub fn n_opt_zq(p: &str) { fn ap(f: Option<fn(&str)>, p: &str) { if let Some(g) = f { g(p) } } ap(Some(sqlx_core::wr_zz), p) }\n\
    pub fn n_optgen_zq(p: &str) { fn ap<F: FnOnce(&str)>(f: Option<F>, p: &str) { match f { Some(g) => g(p), None => {} } } ap(Some(sqlx_core::wr_zz), p) }\n\
    pub fn n_optbox_zq(p: &str) { fn ap(f: Option<Box<dyn Fn(&str)>>, p: &str) { if let Some(g) = f { g(p) } } ap(Some(Box::new(sqlx_core::wr_zz)), p) }\n\
    pub fn n_tuple_zq(p: &str) { fn ap<F: FnOnce(&str)>((f, q): (F, &str)) { f(q) } ap((sqlx_core::wr_zz, p)) }\n\
    pub fn top_tuple<F: FnOnce(&str)>((f, q): (F, &str)) { f(q) }\n\
    pub fn t_tuple_zq(p: &str) { crate::top_tuple((sqlx_core::wr_zz, p)) }\n";

#[test]
fn r1833_a_nested_fn_invoking_its_callable_parameter_discloses() {
    let (app, reps) = world("nested", R1833_APP);
    for deps in [Some(reps.as_path()), None] {
        let on = rows(&app, deps, &[]);
        let off = rows(&app, deps, &[("CANDOR_R1833_OFF", "1")]);
        // each one writes a file when run
        for f in ["n_free_zq", "n_meth_zq", "n_ptr_zq", "n_dyn_zq", "n_local_zq", "n_opt_zq", "n_optgen_zq", "n_optbox_zq", "n_tuple_zq", "top_tuple"] {
            let (e, why) = on.get(f).cloned().unwrap_or_default();
            assert!(e.iter().any(|x| x == "Unknown"), "{f} ({}): {e:?}", deps.is_some());
            assert!(why.iter().any(|w| w.starts_with("callback:")), "{f}: {why:?}");
            assert!(!eff(&off, f).iter().any(|x| x == "Unknown"), "{f}: OFF must restore the old answer ({:?})", eff(&off, f));
        }
        // the top-level tuple-pattern helper's caller inherits the disclosure
        assert!(eff(&on, "t_tuple_zq").iter().any(|x| x == "Unknown"), "t_tuple_zq: {:?}", eff(&on, "t_tuple_zq"));
        // a callable parameter that is never invoked is nothing to disclose
        assert!(eff(&on, "n_nocall_zq").is_empty(), "n_nocall_zq: {:?}", eff(&on, "n_nocall_zq"));
        // parity with the top-level helper
        assert!(eff(&on, "t_free_zq").iter().any(|x| x == "Unknown"));
    }
    assert_eq!(deny(&app, "Fs Unknown", "n_free_zq", Some(&reps), &[]), 1);
}

const R1835_APP: &str = "#![allow(unused_imports)]\n\
    use sqlx_core::Tally;\n\
    pub fn c_path_zq(p: &str) { sqlx_core::c::Raw::tally_zz(sqlx_core::c::raw_c(\"q\"), p) }\n\
    pub fn c_annot_zq(p: &str) { let r: sqlx_core::c::Raw = sqlx_core::c::raw_c(\"q\"); r.tally_zz(p) }\n\
    pub fn a_inh_zq(p: &str) { sqlx_core::a::Raw::tally_zz(&sqlx_core::a::raw_a(\"q\"), p) }\n";

#[test]
fn r1835_an_unclassified_join_does_not_take_a_same_tail_siblings_row() {
    let (app, reps) = world("sibling", R1835_APP);
    let on = rows(&app, Some(&reps), &[]);
    let off = rows(&app, Some(&reps), &[("CANDOR_R1835_OFF", "1")]);
    for f in ["c_path_zq", "c_annot_zq"] {
        // the `Tally` default runs: it reads the environment and writes
        let e = eff(&on, f);
        assert!(e.iter().any(|x| x == "Env") && e.iter().any(|x| x == "Fs"), "{f}: {e:?}");
        assert!(!eff(&off, f).iter().any(|x| x == "Env"), "{f}: OFF must restore the sibling's row ({:?})", eff(&off, f));
        assert_eq!(deny(&app, "Env", f, Some(&reps), &[]), 1, "{f}");
    }
    // the inherent that really owns the tail keeps its own row, and gains nothing
    assert_eq!(eff(&on, "a_inh_zq"), eff(&off, "a_inh_zq"));
    assert!(!eff(&on, "a_inh_zq").iter().any(|x| x == "Env"), "a_inh_zq: {:?}", eff(&on, "a_inh_zq"));
}
