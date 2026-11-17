//! I1 stage 3 — SOUNDNESS R1784 (a `macro_rules!` another macro's expansion DEFINES was never expanded) and R1813 (an `impl` a
//! BODY-position invocation emits was in no index: no unit, and its type published CLOSED with no `deref`). Each shape below was
//! EXECUTED (`rustagent-i1s3/fx/p2`, `fx/ref`, `fx/ch4`: the program writes its marker) and read ABSENT, with no `Unknown` and
//! no `invisible`, on published 0.42.0 and on main `7425d9eb`. Every effectful probe has a pure twin of a unique name that must
//! stay uncharged. The kill switches (`CANDOR_I1S3_MDEF_OFF`, `CANDOR_I1S3_BMDEF_OFF`, `CANDOR_I1S3_BODY_OFF`) restore the old
//! reading, which is what gives each assertion teeth.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::Command;

const W: &str = "let _ = std::fs::write(\"/tmp/i1s3-never-run\", b\"x\");";

fn tmpdir(tag: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("candor-i1s3-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(d.join("src")).unwrap();
    d
}

fn put(dir: &Path, rel: &str, body: &str) {
    let p = dir.join(rel);
    std::fs::create_dir_all(p.parent().unwrap()).unwrap();
    std::fs::write(p, body.replace("EFF", W)).unwrap();
}

fn krate(dir: &Path, name: &str, lib: &str) {
    put(dir, "Cargo.toml", &format!("[package]\nname = \"{name}\"\nversion = \"0.1.0\"\nedition = \"2021\"\n"));
    put(dir, "src/lib.rs", &format!("#![allow(dead_code, unused_macros)]\n{lib}"));
}

/// fn -> inferred
type Rows = BTreeMap<String, Vec<String>>;

fn scan_json(dir: &Path, env: &[(&str, &str)], deps: Option<&Path>) -> serde_json::Value {
    let out = dir.join("out");
    let _ = std::fs::remove_dir_all(&out);
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_candor-scan"));
    cmd.arg(dir.to_string_lossy().as_ref()).arg("--out").arg(out.join("r"));
    for k in [
        "CANDOR_DEPS",
        "CANDOR_CONFIG",
        "CANDOR_POLICY",
        "CANDOR_I1S3_MDEF_OFF",
        "CANDOR_I1S3_BMDEF_OFF",
        "CANDOR_I1S3_BODY_OFF",
    ] {
        cmd.env_remove(k);
    }
    if let Some(d) = deps {
        cmd.env("CANDOR_DEPS", d);
    }
    for (k, v) in env {
        cmd.env(k, v);
    }
    let o = cmd.output().expect("run candor-scan");
    assert!(o.status.code().is_some_and(|c| c == 0 || c == 1), "scan failed: {}", String::from_utf8_lossy(&o.stderr));
    for e in std::fs::read_dir(&out).unwrap() {
        let p = e.unwrap().path();
        let n = p.file_name().unwrap().to_string_lossy().to_string();
        if n.ends_with(".json") && !n.ends_with(".callgraph.json") && !n.ends_with(".locs.json") && !n.ends_with(".hierarchy.json") {
            return serde_json::from_slice(&std::fs::read(&p).unwrap()).unwrap();
        }
    }
    panic!("no report in {}", out.display());
}

fn rows(v: &serde_json::Value) -> Rows {
    let mut r = Rows::new();
    for f in v["functions"].as_array().unwrap() {
        let inf = f["inferred"].as_array().map(|a| a.iter().filter_map(|e| e.as_str().map(str::to_string)).collect()).unwrap_or_default();
        r.insert(f["fn"].as_str().unwrap().to_string(), inf);
    }
    r
}

fn has(r: &Rows, f: &str, e: &str) -> bool {
    r.get(f).is_some_and(|v| v.iter().any(|x| x == e))
}

#[test]
fn a_macro_a_macro_defines_is_expanded_where_it_is_in_textual_scope() {
    let d = tmpdir("mdef");
    krate(
        &d,
        "i1s3mdef",
        "macro_rules! def_fn { () => { macro_rules! mk_fn { ($n:ident) => { pub fn $n() { EFF } }; } }; }\n\
         def_fn!();\nmk_fn!(leaf_it);\npub fn md_item() { leaf_it() }\n\
         macro_rules! def_fnq { () => { macro_rules! mk_fnq { ($n:ident) => { pub fn $n() -> u8 { 5 } }; } }; }\n\
         def_fnq!();\nmk_fnq!(leaf_itq);\npub fn zq_md_item() { leaf_itq(); }\n\
         macro_rules! def_eff { () => { macro_rules! mk_eff { () => { EFF }; } }; }\n\
         def_eff!();\npub fn md_stmt() { mk_eff!(); }\n\
         pub struct MI;\n\
         macro_rules! def_meth { () => { macro_rules! mk_meth { () => { pub fn mi_go(&self) { EFF } }; } }; }\n\
         def_meth!();\nimpl MI { mk_meth!(); }\npub fn md_implitem() { MI.mi_go() }\n\
         macro_rules! def_two { () => { macro_rules! def_two_b { () => { macro_rules! mk_two { () => { EFF }; } }; } }; }\n\
         def_two!();\ndef_two_b!();\npub fn md_nested() { mk_two!(); }\n",
    );
    let r = rows(&scan_json(&d, &[], None));
    for p in ["md_item", "md_stmt", "md_implitem", "md_nested"] {
        assert!(has(&r, p, "Fs"), "{p} must read Fs: {:?}", r.get(p));
    }
    assert!(!has(&r, "zq_md_item", "Fs") && !has(&r, "zq_md_item", "Unknown"), "the pure twin is charged: {:?}", r.get("zq_md_item"));
    let off = rows(&scan_json(&d, &[("CANDOR_I1S3_MDEF_OFF", "1")], None));
    for p in ["md_item", "md_stmt", "md_nested"] {
        assert!(!has(&off, p, "Fs"), "{p}: the kill switch must restore the old (absent) reading: {:?}", off.get(p));
    }
    let _ = std::fs::remove_dir_all(&d);
}

#[test]
fn a_macro_defined_by_a_body_position_expansion_is_in_scope_for_the_rest_of_the_block() {
    let d = tmpdir("bmdef");
    krate(
        &d,
        "i1s3bmdef",
        "macro_rules! bm_def { () => { macro_rules! bm_eff { () => { EFF }; } }; }\n\
         pub fn bm_stmt() { bm_def!(); bm_eff!(); }\n\
         macro_rules! bm_defq { () => { macro_rules! bm_effq { () => { let _ = 8u8; }; } }; }\n\
         pub fn zq_bm_stmt() { bm_defq!(); bm_effq!(); }\n",
    );
    let r = rows(&scan_json(&d, &[], None));
    assert!(has(&r, "bm_stmt", "Fs"), "bm_stmt must read Fs: {:?}", r.get("bm_stmt"));
    assert!(r.get("zq_bm_stmt").is_none_or(|v| v.is_empty()), "the pure twin is charged: {:?}", r.get("zq_bm_stmt"));
    let off = rows(&scan_json(&d, &[("CANDOR_I1S3_BMDEF_OFF", "1")], None));
    assert!(!has(&off, "bm_stmt", "Fs"), "the kill switch must restore the old reading: {:?}", off.get("bm_stmt"));
    let _ = std::fs::remove_dir_all(&d);
}

#[test]
fn cfg_twin_definitions_a_macro_emits_disclose_rather_than_pick_an_arm() {
    let d = tmpdir("twin");
    krate(
        &d,
        "i1s3twin",
        "macro_rules! cfg_u { ($($i:item)*) => { $( #[cfg(unix)] $i )* } }\n\
         macro_rules! cfg_nu { ($($i:item)*) => { $( #[cfg(not(unix))] $i )* } }\n\
         cfg_u! { macro_rules! leaf_mt { () => { { EFF } } } }\n\
         cfg_nu! { macro_rules! leaf_mt { () => { () } } }\n\
         pub fn md_twin() { leaf_mt!(); }\n",
    );
    let r = rows(&scan_json(&d, &[], None));
    assert!(has(&r, "md_twin", "Fs") || has(&r, "md_twin", "Unknown"), "md_twin must not read pure: {:?}", r.get("md_twin"));
    let _ = std::fs::remove_dir_all(&d);
}

#[test]
fn an_impl_a_body_position_invocation_emits_is_a_unit_and_a_refused_one_discloses() {
    let d = tmpdir("body");
    krate(
        &d,
        "i1s3body",
        "pub struct TB; pub struct TBQ; pub struct TC; pub struct RU;\n\
         macro_rules! mk_inh { ($t:ident) => { impl $t { pub fn tb_go(&self) { EFF } } }; }\n\
         macro_rules! mk_inhq { ($t:ident) => { impl $t { pub fn tbq_go(&self) -> u8 { 2 } } }; }\n\
         macro_rules! mk_inhc { ($t:ident) => { impl $t { pub fn tc_go(&self) { EFF } } }; }\n\
         macro_rules! mk_inhu { ($t:ident) => { impl $t { pub fn ru_go(&self) { EFF } } }; }\n\
         fn host_a() { mk_inh!(TB); }\nfn host_q() { mk_inhq!(TBQ); }\nconst _: () = { mk_inhc!(TC); };\n\
         fn host_u() { use std::fmt::Write as _; mk_inhu!(RU); }\n\
         pub fn body_inh() { TB.tb_go() }\npub fn zq_body_inh() { TBQ.tbq_go(); }\npub fn body_const() { TC.tc_go() }\n\
         pub fn body_refused() { RU.ru_go() }\n\
         pub trait GoT { fn go_t(&self); }\npub struct TTp; impl GoT for TTp { fn go_t(&self) {} }\npub struct TT;\n\
         macro_rules! mk_tr { ($t:ident) => { impl GoT for $t { fn go_t(&self) { EFF } } }; }\n\
         fn host_t() { mk_tr!(TT); }\npub fn body_trait() { TT.go_t() }\n",
    );
    let r = rows(&scan_json(&d, &[], None));
    for p in ["body_inh", "body_const", "body_trait"] {
        assert!(has(&r, p, "Fs"), "{p} must read Fs: {:?}", r.get(p));
    }
    assert!(r.get("zq_body_inh").is_none_or(|v| v.is_empty()), "the pure twin is charged: {:?}", r.get("zq_body_inh"));
    // the body writes a `use`: module scope is not where the impl's names resolve, so it is not hoisted, and it discloses
    assert!(has(&r, "body_refused", "Unknown"), "a refused hoist must disclose: {:?}", r.get("body_refused"));
    let off = rows(&scan_json(&d, &[("CANDOR_I1S3_BODY_OFF", "1")], None));
    for p in ["body_inh", "body_const", "body_trait", "body_refused"] {
        assert!(!has(&off, p, "Fs") && !has(&off, p, "Unknown"), "{p}: the kill switch must restore the old reading: {:?}", off.get(p));
    }
    let _ = std::fs::remove_dir_all(&d);
}

#[test]
fn a_deref_a_body_position_invocation_emits_is_published_and_a_chained_consumer_follows_it() {
    let root = tmpdir("chain");
    let prod = root.join("prod");
    krate(
        &prod,
        "i1s3prod",
        "pub struct InnerE; impl InnerE { pub fn leak_e(&self) { EFF } }\npub struct WrapE(pub InnerE);\n\
         macro_rules! mk_e { ($t:ident, $u:ident) => { impl std::ops::Deref for $t { type Target = $u; fn deref(&self) -> &$u { &self.0 } } }; }\n\
         fn _f() { mk_e!(WrapE, InnerE); }\npub fn carrier() {}\n",
    );
    let pv_off = scan_json(&prod, &[("CANDOR_I1S3_BODY_OFF", "1")], None);
    assert!(pv_off["typeSurface"]["types"]["i1s3prod#WrapE"]["deref"].is_null(), "the kill switch must restore the old surface");
    // scanned last, so `prod/out` holds the default report the consumer chains over
    let pv = scan_json(&prod, &[], None);
    let te = &pv["typeSurface"]["types"]["i1s3prod#WrapE"];
    assert_eq!(te["deref"].as_str(), Some("i1s3prod#InnerE"), "WrapE must publish its deref: {te}");
    let app = root.join("app");
    put(
        &app,
        "Cargo.toml",
        "[package]\nname = \"i1s3app\"\nversion = \"0.1.0\"\nedition = \"2021\"\n[dependencies]\ni1s3prod = { path = \"../prod\" }\n",
    );
    put(&app, "src/lib.rs", "pub fn c_e2(w: &i1s3prod::WrapE) { i1s3prod::carrier(); w.leak_e() }\n");
    let r = rows(&scan_json(&app, &[], Some(&prod.join("out"))));
    assert!(has(&r, "c_e2", "Fs"), "the chained consumer must read Fs through the published deref: {:?}", r.get("c_e2"));
    let _ = std::fs::remove_dir_all(&root);
}
