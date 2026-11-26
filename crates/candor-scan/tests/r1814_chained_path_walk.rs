//! SOUNDNESS R1814 — AN UNCLASSIFIED PATH CALL INTO A CHAINED COVERED CRATE WALKS THE WRITTEN TYPE, AS ITS METHOD SPELLING DOES.
//! SOUNDNESS R1816 — A CLASSIFIED CALL WHOSE WALK IS STRUCTURAL (a renamed re-export the manifest does not key) DISCLOSES.
//!
//! R1814: `sqlx_core::d::Lone::tally_zz(lone("q"), p)`, where `tally_zz` is the PROVIDED body of a trait `d::Lone` implements and
//! it reads an env var and writes a file, asked only its own key (`Lone::tally_zz`), which no producer unit carries (a default is
//! keyed under the TRAIT), and the crate is covered: the consumer row was ABSENT, `deny Fs` and `deny Fs Unknown` both 0 over an
//! executed write, on 0.42.0 and 02d58c02 alike (EXECUTED rustagent-r1814 fx/z1 `u_*`, Fable's fx/m3 `r3_lonez_ufcs`), while
//! the method spelling `r.tally_zz(p)` walked and read `[Env, Fs]`.
//!
//! R1816: `pub use crate::k::Kit as CKit` publishes no `types` key for `re2::CKit`, so a classified `r.fetch_all(p)` on a
//! `CKit` (whose `fetch_all` is `Fetch`'s provided body, which writes) walked from an unkeyed node and read the curated `[Db]`
//! alone: `deny Fs Unknown` 0 over an executed write. A structural miss now ADDS `Unknown` for a classified call too.
//!
//! The dependency is a stub named `sqlx_core` (the classifier charges its `fetch_all` verb `Db`). `CANDOR_R1814_OFF=1` and
//! `CANDOR_R1816_OFF=1` are the calibrations. Fixture fns end `_zq` because `deny <E> <fn>` matches by PREFIX.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Command;

fn bin() -> &'static str {
    env!("CARGO_BIN_EXE_candor-scan")
}

fn tmpdir(tag: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("candor-r1814-{tag}-{}", std::process::id()));
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

const SWITCHES: [&str; 7] = [
    "CANDOR_DEPS", "CANDOR_CONFIG", "CANDOR_POLICY", "CANDOR_R1807_OFF", "CANDOR_R1814_OFF", "CANDOR_R1816_OFF", "CANDOR_R1818_OFF",
];

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
fn deny(dir: &Path, effects: &str, f: &str, deps: Option<&Path>) -> i32 {
    let pol = dir.join(format!("policy-{}-{f}.txt", effects.replace(' ', "-")));
    write(&pol, &format!("deny {effects} {f}\n"));
    let mut cmd = scan_cmd(dir, deps, &[]);
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

fn world(tag: &str) -> (PathBuf, PathBuf, PathBuf) {
    let root = tmpdir(tag);
    let dep = pkg(
        &root,
        "sqlx_core",
        &[],
        "pub trait Fetch: Sized { fn fetch_all(self, p: &str) { let v = std::env::var(\"R1814_MARK\").unwrap_or_default(); std::fs::write(format!(\"{p}.fetch\"), v).unwrap(); } }\n\
         pub trait Tally: Sized { fn tally_zz(self, p: &str) { let v = std::env::var(\"R1814_MARK\").unwrap_or_default(); std::fs::write(format!(\"{p}.tally\"), v).unwrap(); } }\n\
         pub trait Mk { fn mk_zz(p: &str) { std::fs::write(format!(\"{p}.mk\"), \"m\").unwrap(); } }\n\
         pub mod d { pub struct Lone(pub String); pub fn lone(s: &str) -> Lone { Lone(s.to_string()) }\n\
          impl crate::Tally for Lone {} impl crate::Mk for Lone {} impl Lone { pub fn still_zz(&self, p: &str) -> usize { p.len() } } }\n\
         pub mod g { pub struct Gen<T>(pub T); impl<T> crate::Tally for Gen<T> {} }\n\
         pub mod a { pub struct Raw(pub String); pub fn raw_a(s: &str) -> Raw { Raw(s.to_string()) }\n\
          impl Raw { pub fn tally_zz(&self, p: &str) { std::fs::write(format!(\"{p}.a\"), &self.0).unwrap(); } } }\n\
         pub mod k { pub struct Kit(pub String); pub fn kit(s: &str) -> Kit { Kit(s.to_string()) } impl crate::Fetch for Kit {} impl crate::Tally for Kit {} }\n\
         pub mod re2 { pub use crate::k::Kit as CKit; pub fn mk(s: &str) -> CKit { crate::k::Kit(s.to_string()) } }\n\
         pub mod q { pub struct Calm(pub String); pub fn calm(s: &str) -> Calm { Calm(s.to_string()) } impl Calm { pub fn still_zz(&self, p: &str) -> usize { p.len() } } }\n\
         pub mod re4 { pub use crate::q::Calm as RCalm; }\n\
         pub mod w { pub struct Wrong(pub String); impl Wrong { pub fn still_zz(&self, p: &str) -> usize { p.len() } } }\n\
         pub mod re6 { pub use crate::k::Kit as Wrong; pub fn mk(s: &str) -> Wrong { crate::k::Kit(s.to_string()) } }\n\
         pub mod sh { pub struct Shade(pub String); pub fn shade(s: &str) -> Shade { Shade(s.to_string()) }\n\
          impl Shade { pub fn tally_zz(self, p: &str) -> usize { p.len() } } impl crate::Tally for Shade {} }\n",
    );
    let reps = root.join("reps");
    std::fs::create_dir_all(&reps).unwrap();
    let o = scan_cmd(&dep, None, &[]).arg("--out").arg(reps.join("sqlx_core")).output().unwrap();
    assert!(o.status.code().is_some_and(|c| (0..=2).contains(&c)), "{}", String::from_utf8_lossy(&o.stderr));
    let app = pkg(
        &root,
        "app",
        &["sqlx_core"],
        "use sqlx_core::{Fetch, Tally, Mk};\n\
         pub fn u_path_zq(p: &str) { sqlx_core::d::Lone::tally_zz(sqlx_core::d::lone(\"q\"), p) }\n\
         pub fn u_use_zq(p: &str) { use sqlx_core::d::Lone; Lone::tally_zz(sqlx_core::d::lone(\"q\"), p) }\n\
         pub fn u_qself_zq(p: &str) { <sqlx_core::d::Lone>::tally_zz(sqlx_core::d::lone(\"q\"), p) }\n\
         pub fn u_assoc_zq(p: &str) { sqlx_core::d::Lone::mk_zz(p) }\n\
         pub fn u_turbo_zq(p: &str) { sqlx_core::g::Gen::<u8>::tally_zz(sqlx_core::g::Gen(0u8), p) }\n\
         pub fn u_method_zq(p: &str) { let r: sqlx_core::d::Lone = sqlx_core::d::lone(\"q\"); r.tally_zz(p) }\n\
         pub fn u_inh_zq(p: &str) { sqlx_core::a::Raw::tally_zz(&sqlx_core::a::raw_a(\"q\"), p) }\n\
         pub fn p_still_zq(p: &str) -> usize { sqlx_core::d::Lone::still_zz(&sqlx_core::d::lone(\"q\"), p) }\n\
         pub fn p_ctor_zq(p: &str) { let _ = sqlx_core::d::lone(p); }\n\
         pub fn f_shade_zq(p: &str) -> usize { sqlx_core::sh::Shade::tally_zz(sqlx_core::sh::shade(\"q\"), p) }\n\
         pub fn r_annot_zq(p: &str) { let r: sqlx_core::re2::CKit = sqlx_core::re2::mk(\"q\"); r.fetch_all(p) }\n\
         pub fn r_ufcs_zq(p: &str) { sqlx_core::re2::CKit::fetch_all(sqlx_core::re2::mk(\"q\"), p) }\n\
         pub fn r_kit_zq(p: &str) { let r: sqlx_core::k::Kit = sqlx_core::k::kit(\"q\"); r.fetch_all(p) }\n\
         pub fn rz_ufcs_zq(p: &str) { sqlx_core::re2::CKit::tally_zz(sqlx_core::re2::mk(\"q\"), p) }\n\
         pub fn w_cls_zq(p: &str) { sqlx_core::re6::Wrong::fetch_all(sqlx_core::re6::mk(\"q\"), p) }\n\
         pub fn w_annot_zq(p: &str) { let r: sqlx_core::re6::Wrong = sqlx_core::re6::mk(\"q\"); r.fetch_all(p) }\n\
         pub fn rp_ufcs_zq(p: &str) -> usize { sqlx_core::re4::RCalm::still_zz(&sqlx_core::q::calm(\"q\"), p) }\n",
    );
    write(
        &app.join("Cargo.lock"),
        "version = 3\n\n[[package]]\nname = \"app\"\nversion = \"0.1.0\"\ndependencies = [\"sqlx_core\"]\n\n[[package]]\nname = \"sqlx_core\"\nversion = \"0.1.0\"\n",
    );
    (root, app, reps)
}

fn set(v: &[&str]) -> Vec<String> {
    let mut s: Vec<String> = v.iter().map(|x| x.to_string()).collect();
    s.sort();
    s
}

fn inf(m: &Rows, f: &str) -> Option<Vec<String>> {
    m.get(f).map(|r| r.0.clone())
}

#[test]
fn an_unclassified_path_call_walks_to_the_trait_default_on_every_path_spelling() {
    let (_root, app, reps) = world("path");
    let on = rows(&app, Some(&reps), &[]);
    let off = rows(&app, Some(&reps), &[("CANDOR_R1814_OFF", "1")]);
    for f in ["u_path_zq", "u_use_zq", "u_qself_zq", "u_turbo_zq"] {
        assert_eq!(inf(&on, f), Some(set(&["Env", "Fs"])), "{f}: Tally's provided body, as the method spelling reads it");
        assert_eq!(inf(&off, f), None, "{f}: R1814 off is the silence (absent)");
    }
    assert_eq!(inf(&on, "u_assoc_zq"), Some(set(&["Fs"])), "an associated fn with no receiver: Mk's default");
    assert_eq!(inf(&off, "u_assoc_zq"), None, "assoc, off");
    // the controls: the method spelling and the inherent twin read the same on both arms
    assert_eq!(inf(&on, "u_method_zq"), Some(set(&["Env", "Fs"])), "method spelling");
    assert_eq!(inf(&on, "u_inh_zq"), Some(set(&["Fs"])), "inherent twin joins by key");
    for f in ["u_method_zq", "u_inh_zq"] {
        assert_eq!(inf(&on, f), inf(&off, f), "{f}: R1814 moves nothing that already joined");
    }
    // pure twins: an inherent pure member of a closed type, and a constructor, stay absent
    for f in ["p_still_zq", "p_ctor_zq"] {
        assert_eq!(inf(&on, f), None, "{f}: pure twin stays absent");
    }
    // SPEC.md:675's ACCEPTED over-charge: an inherent PURE `tally_zz` shadows Tally's default; the wire cannot tell it from an
    // inherited one, so the walk charges the default. The method spelling has always done this; a path call now does too. Pinned
    // so a later "fix" knows the spec priced it (removable only by a member manifest, rung B).
    assert_eq!(inf(&on, "f_shade_zq"), Some(set(&["Env", "Fs"])), "accepted over-charge (SPEC §2 ⟨0.40⟩)");
}

#[test]
fn a_classified_call_through_a_renamed_re_export_discloses() {
    let (_root, app, reps) = world("rename");
    let on = rows(&app, Some(&reps), &[]);
    let off = rows(&app, Some(&reps), &[("CANDOR_R1816_OFF", "1")]);
    for f in ["r_annot_zq", "r_ufcs_zq"] {
        let (i, why) = on.get(f).cloned().unwrap_or_default();
        assert_eq!(i, set(&["Db", "Unknown"]), "{f}: the curated Db and the structural miss's hedge");
        assert!(why.iter().any(|w| w == "dispatch:CKit.fetch_all"), "{f}: the reason names the member: {why:?}");
        assert_eq!(inf(&off, f), Some(set(&["Db"])), "{f}: R1816 off is the curated charge alone");
    }
    // the unrenamed spelling of the same type resolves (R1807's walk) and is not hedged
    assert_eq!(inf(&on, "r_kit_zq"), Some(set(&["Db", "Env", "Fs"])), "unrenamed control");
    // the UNCLASSIFIED renamed path call: walked from the unkeyed node (R1814), hedged by the in-scope `Tally`
    let (i, why) = on.get("rz_ufcs_zq").cloned().unwrap_or_default();
    assert_eq!(i, set(&["Unknown"]), "rz: disclosed");
    assert!(why.iter().any(|w| w == "dispatch:CKit.tally_zz"), "rz: {why:?}");
    assert_eq!(inf(&rows(&app, Some(&reps), &[("CANDOR_R1814_OFF", "1")]), "rz_ufcs_zq"), None, "rz, R1814 off: absent");
    // the renamed PURE twin: its start node is unkeyed, so the manifest cannot say what `RCalm` has — the structural miss
    // discloses on the path spelling as it does on the method spelling (an over-disclosure over a pure body, never a charge)
    assert_eq!(inf(&on, "rp_ufcs_zq"), Some(set(&["Unknown"])), "renamed pure twin: disclosed, not charged");
}

#[test]
fn the_gates_flip_over_the_executed_writes() {
    let (_root, app, reps) = world("gate");
    for f in ["u_path_zq", "u_use_zq", "u_assoc_zq"] {
        assert_eq!(deny(&app, "Fs", f, Some(&reps)), 1, "{f}: deny Fs fires");
    }
    for f in ["r_annot_zq", "r_ufcs_zq", "rz_ufcs_zq"] {
        assert_eq!(deny(&app, "Fs Unknown", f, Some(&reps)), 1, "{f}: deny Fs Unknown fires");
    }
    assert_eq!(deny(&app, "Fs Unknown", "p_still_zq", Some(&reps)), 0, "pure twin on a closed type stays green");
    assert_eq!(deny(&app, "Fs", "rp_ufcs_zq", Some(&reps)), 0, "renamed pure twin: nothing concrete is fabricated");
}

/// Fable's R1814 merge review, fx/f2 `gl` and fx/f1 `wrong`: a dependency whose every type is KIND-ONLY (a macro the producer
/// cannot expand writes its impls: a `#[macro_export]` macro invoked by its `crate::` path. Until I1 stage 3 this was a
/// macro-defined macro, which R1784 now expands, so the unseen expansion moved to a spelling the producer still cannot read), called from a CHILD module that sees the traits only through `use super::*` from the crate root — the
/// in-scope-trait reading is per file, so it never sees them there — and a same-leaf rename (`pub use crate::k::Kit as Wrong`
/// beside a pure `w::Wrong`) whose `type_key` leaf fallback lands on the wrong, closed node (SOUNDNESS R1818).
fn world_gl(tag: &str) -> (PathBuf, PathBuf, PathBuf) {
    let root = tmpdir(tag);
    let dep = pkg(
        &root,
        "sqlx_core",
        &[],
        "pub trait Fetch: Sized { fn fetch_all(self, p: &str) { let v = std::env::var(\"R1814_MARK\").unwrap_or_default(); std::fs::write(format!(\"{p}.fetch\"), v).unwrap(); } }\n\
         pub trait Tally: Sized { fn tally_zz(self, p: &str) { let v = std::env::var(\"R1814_MARK\").unwrap_or_default(); std::fs::write(format!(\"{p}.tally\"), v).unwrap(); } }\n\
         pub trait Quiet: Sized { fn hush_zz(self, p: &str) -> usize { p.len() } }\n\
         #[macro_export] macro_rules! gen_impls { ($t:ty) => { impl crate::Fetch for $t {} impl crate::Tally for $t {} impl crate::Quiet for $t {} impl $t { pub fn inh_zz(&self, p: &str) { std::fs::write(format!(\"{p}.inh\"), \"i\").unwrap(); } } }; }\n\
         pub mod km { pub struct Kmac(pub String); pub fn kmac(s: &str) -> Kmac { Kmac(s.to_string()) } crate::gen_impls!(Kmac); }\n\
         pub mod k { pub struct Kit(pub String); impl crate::Fetch for Kit {} impl crate::Tally for Kit {} impl crate::Quiet for Kit {} }\n\
         pub mod re2 { pub use crate::k::Kit as CKit; pub fn mk(s: &str) -> CKit { crate::k::Kit(s.to_string()) } }\n\
         pub mod d { pub struct Lone(pub String); pub fn lone(s: &str) -> Lone { Lone(s.to_string()) } impl crate::Tally for Lone {} impl crate::Fetch for Lone {} }\n",
    );
    let reps = root.join("reps");
    std::fs::create_dir_all(&reps).unwrap();
    let o = scan_cmd(&dep, None, &[]).arg("--out").arg(reps.join("sqlx_core")).output().unwrap();
    assert!(o.status.code().is_some_and(|c| (0..=2).contains(&c)), "{}", String::from_utf8_lossy(&o.stderr));
    let app = pkg(
        &root,
        "app",
        &["sqlx_core"],
        "#![allow(unused_imports)]\nuse sqlx_core::{Tally, Fetch, Quiet};\npub mod gl;\n",
    );
    write(
        &app.join("src/gl.rs"),
        "#![allow(unused_imports)]\nuse super::*;\n\
         pub fn gl_ufcs_zq(p: &str) { sqlx_core::re2::CKit::tally_zz(sqlx_core::re2::mk(\"q\"), p) }\n\
         pub fn gl_tannot_zq(p: &str) { let r: sqlx_core::re2::CKit = sqlx_core::re2::mk(\"q\"); r.tally_zz(p) }\n\
         pub fn gl_cls_zq(p: &str) { sqlx_core::re2::CKit::fetch_all(sqlx_core::re2::mk(\"q\"), p) }\n\
         pub fn gl_annot_zq(p: &str) { let r: sqlx_core::re2::CKit = sqlx_core::re2::mk(\"q\"); r.fetch_all(p) }\n\
         pub fn gl_lone_zq(p: &str) { sqlx_core::d::Lone::tally_zz(sqlx_core::d::lone(\"q\"), p) }\n\
         pub fn gl_kmac_ufcs_zq(p: &str) { sqlx_core::km::Kmac::tally_zz(sqlx_core::km::kmac(\"q\"), p) }\n\
         pub fn gl_kmac_cls_zq(p: &str) { sqlx_core::km::Kmac::fetch_all(sqlx_core::km::kmac(\"q\"), p) }\n\
         pub fn gl_kmac_annot_zq(p: &str) { let r: sqlx_core::km::Kmac = sqlx_core::km::kmac(\"q\"); r.fetch_all(p) }\n\
         pub fn gl_inh_zq(p: &str) { sqlx_core::km::Kmac::inh_zz(&sqlx_core::km::kmac(\"q\"), p) }\n",
    );
    write(
        &app.join("Cargo.lock"),
        "version = 3\n\n[[package]]\nname = \"app\"\nversion = \"0.1.0\"\ndependencies = [\"sqlx_core\"]\n\n[[package]]\nname = \"sqlx_core\"\nversion = \"0.1.0\"\n",
    );
    (root, app, reps)
}

#[test]
fn the_path_and_method_spellings_of_one_call_on_one_structural_node_disclose_alike() {
    let (_root, app, reps) = world_gl("gl");
    let on = rows(&app, Some(&reps), &[]);
    // the METHOD spelling on the unkeyed node was always disclosed; the PATH spelling now is too (it was ABSENT on 0.42.0
    // and main, and on the first R1814 cut, whose path carve-out dropped the structural-start hedge)
    // `gl_inh_zq`: an INHERENT member the unexpanded macro writes; no producer unit carries it (so no chained unit "publishes"
    // `inh_zz`), which is why the disclosure cannot be narrowed to members some chained unit names
    for f in ["gl_ufcs_zq", "gl_tannot_zq", "gl_lone_zq", "gl_kmac_ufcs_zq", "gl_inh_zq"] {
        let (i, _) = on.get(f).cloned().unwrap_or_default();
        assert!(i.contains(&"Unknown".to_string()), "{f}: the structural start discloses: {i:?}");
    }
    for f in ["gl_cls_zq", "gl_annot_zq", "gl_kmac_cls_zq", "gl_kmac_annot_zq"] {
        assert_eq!(inf(&on, f), Some(set(&["Db", "Unknown"])), "{f}: classified, structural start: the curated Db and the hedge");
    }
    let off = rows(&app, Some(&reps), &[("CANDOR_R1814_OFF", "1"), ("CANDOR_R1816_OFF", "1")]);
    assert_eq!(inf(&off, "gl_ufcs_zq"), None, "calibration: both off, the path spelling is the old silence");
    assert_eq!(inf(&off, "gl_cls_zq"), Some(set(&["Db"])), "calibration: both off, the curated Db alone");
    for f in ["gl_ufcs_zq", "gl_cls_zq", "gl_kmac_cls_zq"] {
        assert_eq!(deny(&app, "Fs Unknown", f, Some(&reps)), 1, "{f}: deny Fs Unknown fires over the executed write");
    }
}

#[test]
fn a_rename_onto_another_types_leaf_is_a_guessed_node_and_discloses() {
    let (_root, app, reps) = world("wrong");
    let on = rows(&app, Some(&reps), &[]);
    let off = rows(&app, Some(&reps), &[("CANDOR_R1818_OFF", "1")]);
    for f in ["w_cls_zq", "w_annot_zq"] {
        let (i, why) = on.get(f).cloned().unwrap_or_default();
        assert_eq!(i, set(&["Db", "Unknown"]), "{f}: the leaf fallback's node is a guess; its closed miss discloses");
        assert!(why.iter().any(|w| w == "dispatch:Wrong.fetch_all"), "{f}: {why:?}");
        assert_eq!(inf(&off, f), Some(set(&["Db"])), "{f}: R1818 off is the old silence");
        assert_eq!(deny(&app, "Fs Unknown", f, Some(&reps)), 1, "{f}: deny Fs Unknown fires");
    }
}
