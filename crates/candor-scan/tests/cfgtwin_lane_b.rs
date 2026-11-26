//! cfg-twin lane B — SOUNDNESS R1863, R1855, R1898. Fixtures are the checked-in attack matrix (`soundness/cfgtwin/fx`, sections E
//! and F); every `_e` probe writes its marker when run (`run.py --exec`) and every `_q` probe is pure or does not reach the write.
//!
//! - R1863 (a RESOLUTION): a hidden override of a member the visible own-cfg impl inherits stands a member on the impl; its body
//!   now reaches the trait's own member, so the effectful default is charged beside the `Unknown` (v0.43.0 and main read
//!   `Unknown` only, `deny Fs` exit 0).
//! - R1855 / R1898 (HEDGES): a source `macro_rules!` shadowing a same-named emitted one in another file, and an item-position
//!   invocation that bound the file's own later definition beside another file's earlier one, disclose `ambiguous:`.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::Command;

fn fx(cell: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../soundness/cfgtwin/fx").join(cell)
}

/// fn -> (inferred, unknownWhy)
type Rows = BTreeMap<String, (Vec<String>, Vec<String>)>;

fn strs(v: &serde_json::Value) -> Vec<String> {
    v.as_array().map(|a| a.iter().filter_map(|e| e.as_str().map(str::to_string)).collect()).unwrap_or_default()
}

/// One scan in a directory of its own: the test fn's name keys it, so two tests over one cell never share (or delete) a dir.
fn scan(cell: &str, who: &str) -> (Rows, i32) {
    let d = std::env::temp_dir().join(format!("candor-cfgtwinB-{who}-{cell}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    let out = d.join("out");
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_candor-scan"));
    cmd.arg(fx(cell)).arg("--out").arg(out.join("r"));
    for k in ["CANDOR_DEPS", "CANDOR_CONFIG", "CANDOR_POLICY", "CANDOR_I1S2_TRAITARM_OFF", "CANDOR_I1S3_SHADOW_OFF"] {
        cmd.env_remove(k);
    }
    cmd.env("CANDOR_CACHE_DIR", d.join("c"));
    let o = cmd.output().expect("run candor-scan");
    let rc = o.status.code().unwrap_or(-1);
    assert!(rc == 0 || rc == 1, "scan failed: {}", String::from_utf8_lossy(&o.stderr));
    let mut r = Rows::new();
    for e in std::fs::read_dir(&out).unwrap() {
        let p = e.unwrap().path();
        let n = p.file_name().unwrap().to_string_lossy().to_string();
        if n.ends_with(".json") && !n.ends_with(".callgraph.json") && !n.ends_with(".locs.json") && !n.ends_with(".hierarchy.json") {
            let v: serde_json::Value = serde_json::from_slice(&std::fs::read(&p).unwrap()).unwrap();
            for f in v["functions"].as_array().unwrap() {
                let k = f["fn"].as_str().unwrap().to_string();
                let ent = r.entry(k).or_default();
                ent.0.extend(strs(&f["inferred"]));
                ent.1.extend(strs(&f["unknownWhy"]));
            }
        }
    }
    let _ = std::fs::remove_dir_all(&d);
    (r, rc)
}

fn row<'a>(r: &'a Rows, probe: &str) -> Option<&'a (Vec<String>, Vec<String>)> {
    r.iter().find(|(k, _)| *k == probe).map(|(_, v)| v)
}
fn unknown(r: &Rows, p: &str) -> bool {
    row(r, p).is_some_and(|v| v.0.iter().any(|e| e == "Unknown"))
}
fn fs(r: &Rows, p: &str) -> bool {
    row(r, p).is_some_and(|v| v.0.iter().any(|e| e == "Fs"))
}
fn why(r: &Rows, p: &str, needle: &str) -> bool {
    row(r, p).is_some_and(|v| v.1.iter().any(|w| w.contains(needle)))
}

#[test]
fn r1863_the_stood_member_keeps_the_defaults_effect_beside_its_unknown() {
    // E1 / E3 / E4 / E9 / E1L: the default is effectful, the hidden override is pure (or both are effectful). Each read `Unknown`
    // ONLY before (`deny Fs` exit 0 over an executed write); now `Fs` and `Unknown` both.
    for cell in ["e1", "e1l", "e3", "e4", "e9"] {
        let (r, _) = scan(cell, "r1863a");
        let p = format!("{cell}_e");
        assert!(fs(&r, &p) && unknown(&r, &p), "{cell}: {:?}", row(&r, &p));
    }
}

#[test]
fn r1863_mirror_and_overriding_shapes_keep_their_unknown_and_charge_nothing_new() {
    // E2 / E2L: default PURE, hidden override effectful: `Unknown`, never silent, and no `Fs` invented from a pure default.
    for cell in ["e2", "e10", "e11"] {
        let (r, _) = scan(cell, "r1863b");
        let p = format!("{cell}_e");
        assert!(unknown(&r, &p) && !fs(&r, &p), "{cell}: {:?}", row(&r, &p));
    }
    // E5: the visible member overrides (pure) — no default is reached, the stood member is not made for it.
    let (r, _) = scan("e5", "r1863b");
    assert!(unknown(&r, "e5_e") && !fs(&r, "e5_e"), "{:?}", row(&r, "e5_e"));
    // E12: a STD trait has no local default unit: the stood member's `Unknown` is still the floor.
    for p in ["e12_e", "e12_b"] {
        let (r, _) = scan("e12", "r1863b");
        assert!(unknown(&r, p), "{p}: {:?}", row(&r, p));
    }
}

#[test]
fn r1863_the_gate_flips_over_the_write() {
    // `deny Fs` over the probe: exit 0 before this change (the row held `Unknown` alone), 1 now.
    let d = std::env::temp_dir().join(format!("candor-cfgtwinB-gate-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    std::fs::write(d.join("policy"), "deny Fs e1_e\n").unwrap();
    let o = Command::new(env!("CARGO_BIN_EXE_candor-scan"))
        .arg(fx("e1"))
        .arg("--out")
        .arg(d.join("out").join("r"))
        .arg("--policy")
        .arg(d.join("policy"))
        .env("CANDOR_CACHE_DIR", d.join("c"))
        .env_remove("CANDOR_POLICY")
        .env_remove("CANDOR_DEPS")
        .output()
        .unwrap();
    let _ = std::fs::remove_dir_all(&d);
    assert_eq!(o.status.code(), Some(1), "{}", String::from_utf8_lossy(&o.stderr));
}

#[test]
fn r1855_a_source_macro_shadowing_an_emitted_one_in_another_file_discloses() {
    // F1: `defs.rs` emits `kx` through a definer; main.rs defines a pure `kx` AFTER the first invocation. The source arm is the
    // one read; the emitted one (effectful) is what rustc expands at `f1_e`. F1Q is R208's stated over-hold on the same name.
    let (r, _) = scan("f1", "r1855");
    assert!(unknown(&r, "f1_e") && why(&r, "f1_e", "ambiguous:"), "{:?}", row(&r, "f1_e"));
    assert!(unknown(&r, "f1_q"), "{:?}", row(&r, "f1_q")); // over-hold, recorded
    // F2: source/source across files is R208's already and stays so.
    let (r, _) = scan("f2", "r1855");
    assert!(unknown(&r, "f2_e") && why(&r, "f2_e", "ambiguous:"), "{:?}", row(&r, "f2_e"));
}

#[test]
fn r1898_an_item_position_invocation_before_the_files_own_definition_discloses() {
    // F3: `mkf!(f3_e)` precedes main.rs's own `mkf` and follows `#[macro_use] mod defs;`, whose `mkf` writes.
    let (r, _) = scan("f3", "r1898");
    assert!(unknown(&r, "f3_e") && why(&r, "f3_e", "ambiguous:"), "{:?}", row(&r, "f3_e"));
    // the invocation AFTER the file's own definition binds it (textual scope), so `f3_q` is not touched
    assert!(!unknown(&r, "f3_q"), "{:?}", row(&r, "f3_q"));
}

#[test]
fn r1898_controls_a_once_defined_macro_is_not_disclosed() {
    // G1 has no macro at all; the scan must not file an `ambiguous:` on anything.
    let (r, _) = scan("g1", "r1898c");
    assert!(r.values().all(|v| !v.1.iter().any(|w| w.contains("ambiguous:"))), "{r:?}");
}
