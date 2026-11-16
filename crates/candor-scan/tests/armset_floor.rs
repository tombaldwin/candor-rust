//! A type written through a crate-ROOT local re-export of a dependency type (`pub use crate::ident::Ident;` at the root,
//! `pub use rdep::Ident;` in `ident`), spelled `crate::Ident` or bare under a root glob, named no type: the receiver
//! counted as typed, the formed call named nothing, and a method call that writes a file read ABSENT with no `invisible`
//! (EXECUTED in `rustagent-i1s2b/fx/fxre`; on published 0.42.0 too). The re-export table's root key answers it now.
//! `CANDOR_ROOT_REEXPORT1_OFF` restores the old reading, which is what gives each assertion teeth.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::Command;

const W: &str = "let _ = std::fs::write(\"/tmp/arm-never-run\", b\"x\");";

fn tmpdir(tag: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("candor-arm-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(d.join("src")).unwrap();
    d
}

fn put(dir: &Path, rel: &str, body: &str) {
    let p = dir.join(rel);
    std::fs::create_dir_all(p.parent().unwrap()).unwrap();
    std::fs::write(p, body.replace("EFF", W)).unwrap();
}

/// fn -> (inferred, invisible)
type Rows = BTreeMap<String, (Vec<String>, Vec<String>)>;

fn scan(dir: &Path, env: &[(&str, &str)]) -> Rows {
    let out = dir.join("out");
    let _ = std::fs::remove_dir_all(&out);
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_candor-scan"));
    cmd.arg(dir.to_string_lossy().as_ref()).arg("--out").arg(out.join("r"));
    for k in ["CANDOR_DEPS", "CANDOR_CONFIG", "CANDOR_POLICY", "CANDOR_ARMSET_OFF", "CANDOR_I1S2_XCFG_OFF"] {
        cmd.env_remove(k);
    }
    for (k, v) in env {
        cmd.env(k, v);
    }
    let o = cmd.output().expect("run candor-scan");
    assert!(o.status.code().is_some_and(|c| c == 0 || c == 1), "scan failed: {}", String::from_utf8_lossy(&o.stderr));
    let mut rows = Rows::new();
    for e in std::fs::read_dir(&out).unwrap() {
        let p = e.unwrap().path();
        let n = p.file_name().unwrap().to_string_lossy().to_string();
        if !n.ends_with(".json") || n.ends_with(".callgraph.json") || n.ends_with(".locs.json") || n.ends_with(".hierarchy.json") {
            continue;
        }
        let v: serde_json::Value = serde_json::from_slice(&std::fs::read(&p).unwrap()).unwrap();
        let strs = |x: &serde_json::Value| -> Vec<String> {
            x.as_array().map(|a| a.iter().filter_map(|e| e.as_str().map(str::to_string)).collect()).unwrap_or_default()
        };
        for f in v["functions"].as_array().unwrap() {
            rows.insert(f["fn"].as_str().unwrap().to_string(), (strs(&f["inferred"]), strs(&f["invisible"])));
        }
    }
    rows
}

#[test]
fn an_untyped_receiver_over_cfg_arms_discloses_an_arm_that_is_not_provably_pure() {
    let root = tmpdir("root");
    let dep = root.join("adep");
    put(&dep, "Cargo.toml", "[package]\nname = \"adep\"\nversion = \"0.1.0\"\nedition = \"2021\"\n");
    put(&dep, "src/lib.rs", "pub struct Ext { pub v: u8 }\nimpl Ext { pub fn touch(&self) { EFF } }\n");
    let app = root.join("app");
    put(
        &app,
        "Cargo.toml",
        "[package]\nname = \"armapp\"\nversion = \"0.1.0\"\nedition = \"2021\"\n[dependencies]\nadep = { path = \"../adep\" }\n",
    );
    put(
        &app,
        "src/lib.rs",
        "#![allow(unused)]\npub mod vis;\n\
         pub struct Reg { v: vis::SelV, p: vis::SelP }\n\
         impl Reg {\n    pub fn new() -> Reg { Reg { v: vis::SelV::new(), p: vis::SelP::new() } }\n\
             pub(crate) fn v(&self) -> &vis::SelV { &self.v }\n    pub(crate) fn p(&self) -> &vis::SelP { &self.p }\n}\n\
         pub fn vis_e() -> usize { Reg::new().v().idv() }\n\
         pub fn pure_ctl() -> usize { Reg::new().p().idp() }\n",
    );
    put(&app, "src/vis/mod.rs", "#[cfg(unix)]\nmod a;\n#[cfg(unix)]\npub use self::a::*;\n#[cfg(windows)]\nmod b;\n#[cfg(windows)]\npub use self::b::*;\n");
    put(
        &app,
        "src/vis/a.rs",
        "pub struct SelV { id: usize }\nimpl SelV { pub fn new() -> SelV { SelV { id: 1 } } pub fn idv(&self) -> usize { adep::Ext { v: 1 }.touch(); self.id } }\n\
         pub struct SelP { id: usize }\nimpl SelP { pub fn new() -> SelP { SelP { id: 1 } } pub fn idp(&self) -> usize { self.id } }\n",
    );
    put(
        &app,
        "src/vis/b.rs",
        "pub struct SelV { id: usize }\nimpl SelV { pub fn new() -> SelV { SelV { id: 2 } } pub fn idv(&self) -> usize { self.id } }\n\
         pub struct SelP { id: usize }\nimpl SelP { pub fn new() -> SelP { SelP { id: 2 } } pub fn idp(&self) -> usize { self.id } }\n",
    );
    let on = scan(&app, &[]);
    let off = scan(&app, &[("CANDOR_ARMSET_OFF", "1")]);
    // the arm that runs (unix) writes through an unchained dependency: the call must disclose
    let v = on.get("vis_e").map(|r| r.0.clone()).unwrap_or_default();
    assert!(v.contains(&"Unknown".to_string()), "vis_e must disclose the not-provably-pure arm: {on:?}");
    // teeth: the old floor read it absent
    assert!(off.get("vis_e").is_none_or(|r| r.0.is_empty()), "the kill switch must restore the old floor: {off:?}");
    // CONTROL: every arm provably pure — nothing to disclose, nothing fabricated
    assert!(on.get("pure_ctl").is_none_or(|r| r.0.is_empty()), "an all-pure arm set must stay silent: {on:?}");
}
