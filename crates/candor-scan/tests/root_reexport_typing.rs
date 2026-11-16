//! A type written through a crate-ROOT local re-export of a dependency type (`pub use crate::ident::Ident;` at the root,
//! `pub use rdep::Ident;` in `ident`), spelled `crate::Ident` or bare under a root glob, named no type: the receiver
//! counted as typed, the formed call named nothing, and a method call that writes a file read ABSENT with no `invisible`
//! (EXECUTED in `rustagent-i1s2b/fx/fxre`; on published 0.42.0 too). The re-export table's root key answers it now.
//! `CANDOR_ROOT_REEXPORT1_OFF` restores the old reading, which is what gives each assertion teeth.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::Command;

const W: &str = "let _ = std::fs::write(\"/tmp/rr1-never-run\", b\"x\");";

fn tmpdir(tag: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("candor-rr1-{tag}-{}", std::process::id()));
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
    for k in ["CANDOR_DEPS", "CANDOR_CONFIG", "CANDOR_POLICY", "CANDOR_ROOT_REEXPORT1_OFF", "CANDOR_I1S2_XCFG_OFF"] {
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
fn a_type_written_through_a_root_reexport_of_a_dependency_type_discloses_the_dependency() {
    let root = tmpdir("root");
    let dep = root.join("rdep");
    put(&dep, "Cargo.toml", "[package]\nname = \"rdep\"\nversion = \"0.1.0\"\nedition = \"2021\"\n");
    put(&dep, "src/lib.rs", "pub struct Ident { pub v: u8 }\nimpl Ident { pub fn touch(&self) { EFF } }\n");
    let app = root.join("app");
    put(
        &app,
        "Cargo.toml",
        "[package]\nname = \"rr1app\"\nversion = \"0.1.0\"\nedition = \"2021\"\n[dependencies]\nrdep = { path = \"../rdep\" }\n",
    );
    put(
        &app,
        "src/lib.rs",
        "#![allow(unused)]\nmod ident;\npub use crate::ident::Ident;\npub mod item;\npub mod glb;\npub mod own;\n\
         mod ext;\npub use crate::ext::B;\npub mod gen;\n",
    );
    // CONTROL (bstr 1.13.0's shape, first A/B of this change): the root re-exports a FN `B`, and a struct's GENERIC
    // PARAMETER is also `B` — recorded bare, like any name. The field is not the fn: its trait dispatch keeps its hedge.
    put(&app, "src/ext.rs", "#[allow(non_snake_case)]\npub fn B(x: &[u8]) -> &[u8] { x }\n");
    put(
        &app,
        "src/gen.rs",
        "use std::io;\npub struct Lines<B> { buf: B }\n\
         impl<B: io::BufRead> Lines<B> { pub fn next_line(&mut self) -> usize { let mut v = vec![]; self.buf.read_until(b'\\n', &mut v).unwrap_or(0) } }\n",
    );
    put(&app, "src/ident.rs", "pub use rdep::Ident;\n");
    put(
        &app,
        "src/item.rs",
        "fn mk() -> rdep::Ident { rdep::Ident { v: 1 } }\n\
         pub struct H { pub ident: crate::Ident }\n\
         pub fn field_e() { H { ident: mk() }.ident.touch() }\n\
         pub fn let_e() { let i: crate::Ident = mk(); i.touch() }\n\
         fn p(i: &crate::Ident) { i.touch() }\n\
         pub fn param_e() { p(&mk()) }\n",
    );
    // the bare spelling under a root glob — syn's `use super::*;` beside `pub struct UseName { pub ident: Ident }`
    put(
        &app,
        "src/glb.rs",
        "use crate::*;\npub struct G { pub ident: Ident }\nimpl G { pub fn go(&self) { self.ident.touch() } }\n",
    );
    // CONTROL: a module's OWN `Ident` beside the root's — recorded anchored, so the root's re-export must not take it. Its
    // method has its own name, so a by-leaf join to `touch` cannot charge the probes above (or this one) by accident.
    put(
        &app,
        "src/own.rs",
        "pub struct Ident { pub v: u8 }\nimpl Ident { pub fn touch_own(&self) { EFF } }\n\
         pub struct O { pub ident: Ident }\npub fn own_e() { O { ident: Ident { v: 1 } }.ident.touch_own() }\n",
    );
    let on = scan(&app, &[]);
    let off = scan(&app, &[("CANDOR_ROOT_REEXPORT1_OFF", "1")]);
    for f in ["item::field_e", "item::let_e", "item::p", "glb::G::go"] {
        let r = on.get(f).unwrap_or_else(|| panic!("{f}: absent with the fix: {on:?}"));
        assert!(r.1.contains(&"rdep".to_string()), "{f}: must name the dependency: {on:?}");
        // teeth: without the lookup the same call names nothing
        assert!(off.get(f).is_none_or(|r| r.0.is_empty() && r.1.is_empty()), "{f}: the kill switch must restore the old reading: {off:?}");
    }
    let gen = on.get("gen::Lines::next_line").map(|r| r.0.clone()).unwrap_or_default();
    assert!(gen.contains(&"Unknown".to_string()), "a generic parameter named like a root fn must keep its dispatch hedge: {on:?}");
    // the module's own type keeps its own charge and is never sent to the dependency
    assert_eq!(on.get("own::own_e").map(|r| r.0.clone()), Some(vec!["Fs".to_string()]), "{on:?}");
    assert!(!on["own::own_e"].1.contains(&"rdep".to_string()), "{on:?}");
}
