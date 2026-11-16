//! SOUNDNESS R1807 — A CHAINED ROW AND A CURATED ENTRY THAT ANSWER ONE CALL ARE UNIONED, on every spelling.
//!
//! With a dependency CHAINED, a call the curated classifier answers (`fetch_all` → `Db` on a `sqlx_core` path) never read the
//! dependency's row on the TYPED route: `sqlx_core::raw_sql::RawC::fetch_all(..)` over a dependency whose `fetch_all` writes a
//! file read `[Db]`, and `deny Fs` exited 0 over code that writes (executed, rustagent-r1807 fx/m2, on 0.42.0 and a1812e43
//! alike). The row's `Unknown` and its `dispatchesOn` follow-on were dropped the same way. The untyped spelling of the same
//! member did the opposite (the row won, the curated `Db` was dropped). Five sites, four answers; now one: the union.
//!
//! The dependency is a stub named `sqlx_core` (the classifier calibrates that crate by NAME, and charges the `fetch_all` verb
//! `Db`). Its members:
//!   * `RawC::fetch_all` writes a file: row `[Fs]`.
//!   * `RawU::fetch_all` dispatches on `Sink::put`, which the CONSUMER implements with a file write (`WSink`): row `[Unknown]` +
//!     `dispatchesOn`, and the §4 obligation-3 follow-on reaches the consumer's `Fs`.
//!   * `RawV::fetch_all` dispatches on `Sink2::put2`, which nobody implements: row `[Unknown]` alone (a disclosure cell; the hedge
//!     is the only thing the row adds, so a "concrete union, hedge dropped" rule reads clean everywhere except here).
//!   * `a::Raw::fetch_all` writes; `b::Raw::fetch_all` is pure: same tail, different types (a sibling must not be charged).
//!   * `RawB::step` yields a `RawC`; `RawB::fetch_all` itself is pure (the builder-chain guess).
//!
//! Every assertion names the arm. `CANDOR_R1807_OFF=1` is the calibration: each effectful case loses what R1807 adds. Fixture
//! fns end `_zq` because `deny <E> <fn>` matches by PREFIX.

use std::path::{Path, PathBuf};
use std::process::Command;

fn bin() -> &'static str {
    env!("CARGO_BIN_EXE_candor-scan")
}

fn tmpdir(tag: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("candor-r1807-{tag}-{}", std::process::id()));
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
    "CANDOR_DEPS",
    "CANDOR_CONFIG",
    "CANDOR_POLICY",
    "CANDOR_R1787_OFF",
    "CANDOR_R1807_OFF",
    "CANDOR_R1807_INSTR",
    "CANDOR_R1807_SEED_HEDGE",
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

/// Every row of the report over `dir` chained on `deps`, with `env` set: fn → (`inferred`, `unknownWhy`). A switched arm is read
/// from the report (a result-moving switch makes a GATE refuse).
fn rows(dir: &Path, deps: Option<&Path>, env: &[(&str, &str)]) -> std::collections::HashMap<String, (Vec<String>, Vec<String>)> {
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
        "pub trait Sink { fn put(&self, p: &str); }\n\
         pub trait Sink2 { fn put2(&self, p: &str); }\n\
         pub mod raw_sql {\n\
          use crate::{Sink, Sink2};\n\
          pub struct RawC(pub String);\n\
          pub fn raw_c(s: &str) -> RawC { RawC(s.to_string()) }\n\
          impl RawC { pub fn new(s: &str) -> RawC { RawC(s.to_string()) } pub fn fetch_all(self, p: &str) { std::fs::write(p, self.0).unwrap(); } }\n\
          pub struct RawU(pub String);\n\
          pub fn raw_u(s: &str) -> RawU { RawU(s.to_string()) }\n\
          impl RawU { pub fn fetch_all(self, k: &dyn Sink, p: &str) { k.put(p) } }\n\
          pub struct RawV(pub String);\n\
          pub fn raw_v(s: &str) -> RawV { RawV(s.to_string()) }\n\
          impl RawV { pub fn fetch_all(self, k: &dyn Sink2, p: &str) { k.put2(p) } }\n\
          pub struct RawB(pub String);\n\
          impl RawB { pub fn new(s: &str) -> RawB { RawB(s.to_string()) } pub fn step(self) -> RawC { RawC(self.0) } pub fn fetch_all(self, p: &str) -> usize { self.0.len() + p.len() } }\n\
         }\n\
         pub mod a { pub struct Raw(pub String); pub fn raw_a(s: &str) -> Raw { Raw(s.to_string()) } impl Raw { pub fn fetch_all(self, p: &str) { std::fs::write(p, self.0).unwrap(); } } }\n\
         pub mod b { pub struct Raw(pub String); pub fn raw_b(s: &str) -> Raw { Raw(s.to_string()) } impl Raw { pub fn fetch_all(self, p: &str) -> usize { self.0.len() + p.len() } } }\n",
    );
    let reps = root.join("reps");
    std::fs::create_dir_all(&reps).unwrap();
    let o = scan_cmd(&dep, None, &[]).arg("--out").arg(reps.join("sqlx_core")).output().unwrap();
    assert!(o.status.code().is_some_and(|c| (0..=2).contains(&c)), "{}", String::from_utf8_lossy(&o.stderr));
    let app = pkg(
        &root,
        "app",
        &["sqlx_core"],
        "use sqlx_core::raw_sql::{raw_c, raw_u, raw_v, RawB, RawC, RawU, RawV};\n\
         use sqlx_core::Sink2;\n\
         pub struct WSink;\n\
         impl sqlx_core::Sink for WSink { fn put(&self, p: &str) { std::fs::write(format!(\"{p}.sink\"), \"s\").unwrap(); } }\n\
         pub fn c_ufcs_zq(p: &str) { RawC::fetch_all(raw_c(\"q\"), p) }\n\
         pub fn c_annot_zq(p: &str) { let r: RawC = raw_c(\"q\"); r.fetch_all(p) }\n\
         pub fn c_param_zq(r: RawC, p: &str) { r.fetch_all(p) }\n\
         pub fn c_ctor_zq(p: &str) { RawC::new(\"q\").fetch_all(p) }\n\
         pub fn c_untyped_zq(p: &str) { let r = raw_c(\"q\"); r.fetch_all(p) }\n\
         pub fn c_inline_zq(p: &str) { raw_c(\"q\").fetch_all(p) }\n\
         pub fn u_param_zq(r: RawU, p: &str) { r.fetch_all(&WSink, p) }\n\
         pub fn u_inline_zq(p: &str) { raw_u(\"q\").fetch_all(&WSink, p) }\n\
         pub fn v_param_zq(r: RawV, k: &dyn Sink2, p: &str) { r.fetch_all(k, p) }\n\
         pub fn v_inline_zq(k: &dyn Sink2, p: &str) { raw_v(\"q\").fetch_all(k, p) }\n\
         pub fn ca_ufcs_zq(p: &str) { sqlx_core::a::Raw::fetch_all(sqlx_core::a::raw_a(\"q\"), p) }\n\
         pub fn cb_ufcs_zq(p: &str) -> usize { sqlx_core::b::Raw::fetch_all(sqlx_core::b::raw_b(\"q\"), p) }\n\
         pub fn cb_annot_zq(p: &str) -> usize { let r: sqlx_core::b::Raw = sqlx_core::b::raw_b(\"q\"); r.fetch_all(p) }\n\
         pub fn b_chain_zq(p: &str) { RawB::new(\"q\").step().fetch_all(p) }\n\
         pub fn b_direct_zq(p: &str) -> usize { RawB::new(\"q\").fetch_all(p) }\n\
         #[cfg(unix)]\nuse sqlx_core::raw_sql::RawC as CfgT;\n\
         #[cfg(not(unix))]\nuse sqlx_core::raw_sql::RawB as CfgT;\n\
         pub fn c_cfg_zq(r: CfgT, p: &str) { let _ = r.fetch_all(p); }\n",
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

#[test]
fn every_spelling_of_a_classified_chained_call_carries_the_union() {
    let (_root, app, reps) = world("union");
    let on = rows(&app, Some(&reps), &[]);
    let off = rows(&app, Some(&reps), &[("CANDOR_R1807_OFF", "1")]);
    let get = |m: &std::collections::HashMap<String, (Vec<String>, Vec<String>)>, f: &str| m.get(f).cloned().unwrap_or_default();
    // the concrete member, every spelling: the curated `Db` AND the row's `Fs`
    for f in ["c_ufcs_zq", "c_annot_zq", "c_param_zq", "c_ctor_zq", "c_untyped_zq", "c_inline_zq", "ca_ufcs_zq", "b_chain_zq", "c_cfg_zq"] {
        assert_eq!(get(&on, f).0, set(&["Db", "Fs"]), "{f}: union of the curated Db and the row's Fs");
    }
    // the calibration: the typed spellings (and the chain guess, and the cfg arm) lose the row's Fs with R1807 off; the untyped
    // spellings lose the curated Db
    for f in ["c_ufcs_zq", "c_annot_zq", "c_param_zq", "c_ctor_zq", "ca_ufcs_zq", "b_chain_zq", "c_cfg_zq"] {
        assert_eq!(get(&off, f).0, set(&["Db"]), "{f}: R1807 off is the curated-wins answer");
    }
    for f in ["c_untyped_zq", "c_inline_zq"] {
        assert_eq!(get(&off, f).0, set(&["Fs"]), "{f}: R1807 off is the row-wins answer");
    }
    // the Unknown-only row through the consumer's own implementor: Db, the follow-on's Fs, and the row's hedge with its reason
    for f in ["u_param_zq", "u_inline_zq"] {
        let (inf, why) = get(&on, f);
        assert_eq!(inf, set(&["Db", "Fs", "Unknown"]), "{f}: union, with the dispatch follow-on");
        assert!(why.iter().any(|w| w == "dispatch:Sink.put"), "{f}: the row's reason is carried: {why:?}");
    }
    assert_eq!(get(&off, "u_param_zq").0, set(&["Db"]), "u_param: off drops the row, its hedge and its follow-on");
    assert_eq!(get(&off, "u_inline_zq").0, set(&["Db", "Unknown"]), "u_inline: off never followed the dispatch");
    // the hedge-only row: exactly `Unknown` added, with its reason; nothing executes behind it
    for f in ["v_param_zq", "v_inline_zq"] {
        let (inf, why) = get(&on, f);
        assert_eq!(inf, set(&["Db", "Unknown"]), "{f}: the row's hedge is unioned, nothing else");
        assert!(why.iter().any(|w| w == "dispatch:Sink2.put2"), "{f}: {why:?}");
    }
    assert_eq!(get(&off, "v_param_zq").0, set(&["Db"]), "v_param: off withdrew the hedge");
    // the sibling and the pure base: the curated charge alone, never a same-tail sibling's Fs
    for f in ["cb_ufcs_zq", "cb_annot_zq", "b_direct_zq"] {
        assert_eq!(get(&on, f).0, set(&["Db"]), "{f}: no sibling's Fs");
        assert_eq!(get(&on, f), get(&off, f), "{f}: R1807 moves nothing on a pure member");
    }
}

#[test]
fn the_gate_flips_over_code_that_writes_and_stays_green_over_code_that_does_not() {
    let (_root, app, reps) = world("gate");
    for f in ["c_ufcs_zq", "c_param_zq", "u_param_zq", "ca_ufcs_zq", "b_chain_zq", "c_cfg_zq"] {
        assert_eq!(deny(&app, "Fs", f, Some(&reps)), 1, "{f}: deny Fs fires over the executed write");
    }
    for f in ["c_untyped_zq", "c_param_zq"] {
        assert_eq!(deny(&app, "Db", f, Some(&reps)), 1, "{f}: deny Db fires (the curated charge stands)");
    }
    assert_eq!(deny(&app, "Fs Unknown", "v_param_zq", Some(&reps)), 1, "v: the hedge reaches deny Fs Unknown");
    assert_eq!(deny(&app, "Fs", "v_param_zq", Some(&reps)), 0, "v: and nothing concrete is fabricated");
    for f in ["cb_ufcs_zq", "cb_annot_zq", "b_direct_zq"] {
        assert_eq!(deny(&app, "Fs Unknown", f, Some(&reps)), 0, "{f}: pure twin");
    }
}

#[test]
fn a_seeded_hedge_drop_is_visible_to_the_content_assertion() {
    // the instrument calibration Fable's C1 asks for: a rule that unions the row's concrete effects but drops its `Unknown`
    // reads `deny Fs` = 1 on `u` (the follow-on still reaches the consumer's Fs), so a gate-pair check cannot see it; the
    // content check can
    let (_root, app, reps) = world("seed");
    let seeded = rows(&app, Some(&reps), &[("CANDOR_R1807_SEED_HEDGE", "1")]);
    let u = seeded.get("u_param_zq").cloned().unwrap_or_default().0;
    assert!(u.contains(&"Fs".to_string()) && !u.contains(&"Unknown".to_string()), "seed: u keeps Fs and loses the hedge: {u:?}");
    let v = seeded.get("v_param_zq").cloned().unwrap_or_default().0;
    assert_eq!(v, set(&["Db"]), "seed: v loses its only addition");
}

#[test]
fn a_stale_chained_report_unions_its_downgrade_and_no_concrete_effect() {
    let (_root, app, reps) = world("stale");
    // rewrite the producing version: §2.1 downgrades every row to `Unknown` + `dep-stale:`
    let stale = reps.parent().unwrap().join("reps-stale");
    std::fs::create_dir_all(&stale).unwrap();
    for e in std::fs::read_dir(&reps).unwrap() {
        let p = e.unwrap().path();
        let mut v: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(&p).unwrap()).unwrap();
        if let Some(c) = v.get_mut("candor").and_then(|c| c.as_object_mut()) {
            c.insert("version".into(), serde_json::Value::String("0.0.1-stale".into()));
        }
        std::fs::write(stale.join(p.file_name().unwrap()), v.to_string()).unwrap();
    }
    let on = rows(&app, Some(&stale), &[]);
    for f in ["c_ufcs_zq", "c_param_zq", "u_param_zq"] {
        let (inf, why) = on.get(f).cloned().unwrap_or_default();
        assert_eq!(inf, set(&["Db", "Unknown"]), "{f}: the curated Db stays, the stale row adds only its downgrade");
        assert!(why.iter().any(|w| w == "dep-stale:sqlx_core"), "{f}: {why:?}");
    }
}
