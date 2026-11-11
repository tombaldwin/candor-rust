//! SOUNDNESS R1661 — A `crate::`-ROOTED RECEIVER BASE MUST NOT FORM A DEPENDENCY PROVENANCE.
//!
//! `dep_value_provenance`'s field arm rooted a provenance at the first segment of a resolved base type, which for a
//! crate-anchored type is `crate`. The call site then pushed an inert `crate::<untyped>::..` marker that no dependency
//! answers, and `note_untyped_method` counted it as a formed call, so the R1095 `dispatch:untyped receiver` floor stood down
//! and the caller read ABSENT. EXECUTED (lane scratch `rustagent-stage1a/fx/m2x`, the binary writes the marker for every
//! spelling): `n::m2_qual_zq(s: &crate::m::S) { s.f.0.go() }` was ABSENT on published 0.41.3 and on 4dcfd9f; the bare
//! spelling in the declaring module and the `self` spelling disclosed. Fixture fns carry `_zq` because `deny` matches by prefix.

use std::path::{Path, PathBuf};
use std::process::Command;

fn bin() -> &'static str {
    env!("CARGO_BIN_EXE_candor-scan")
}

fn fixture(tag: &str, src: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("candor-r1661-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(d.join("src")).unwrap();
    std::fs::write(d.join("Cargo.toml"), "[package]\nname = \"fx\"\nversion = \"0.1.0\"\nedition = \"2021\"\n[dependencies]\n").unwrap();
    std::fs::write(d.join("src/lib.rs"), src).unwrap();
    d
}

fn scan(dir: &Path, policy: Option<&str>, off: bool) -> (i32, serde_json::Value) {
    let mut cmd = Command::new(bin());
    cmd.arg(dir.to_string_lossy().as_ref()).arg("--json");
    if let Some(p) = policy {
        let pf = dir.join("policy.txt");
        std::fs::write(&pf, p).unwrap();
        cmd.arg("--policy").arg(&pf);
    }
    cmd.env_remove("CANDOR_DEPS").env_remove("CANDOR_CONFIG").env_remove("CANDOR_POLICY").env_remove("CANDOR_R1661_OFF")
        .env_remove("CANDOR_R1692_OFF");
    // SOUNDNESS R1692 generalises this row: ANY `<untyped>` marker whose roots are no declared dependency now reaches the R1095
    // floor, so `CANDOR_R1661_OFF` alone no longer restores the silence (`r1692_subsumes_the_crate_rooted_marker` below). The
    // attribution arm turns both off.
    if off {
        cmd.env("CANDOR_R1661_OFF", "1").env("CANDOR_R1692_OFF", "1");
    }
    let out = cmd.output().expect("run candor-scan");
    let code = out.status.code().unwrap_or(-1);
    let refused_by_switch = off && policy.is_some() && code == 2;
    assert!(code == 0 || code == 1 || refused_by_switch, "scan failed ({code}): {}", String::from_utf8_lossy(&out.stderr));
    (code, serde_json::from_slice(&out.stdout).expect("report json"))
}

fn row<'a>(v: &'a serde_json::Value, f: &str) -> Option<&'a serde_json::Value> {
    v["functions"].as_array().unwrap().iter().find(|r| r["fn"].as_str() == Some(f))
}

fn inferred(v: &serde_json::Value, f: &str) -> Vec<String> {
    row(v, f).and_then(|r| r["inferred"].as_array()).map(|a| a.iter().filter_map(|e| e.as_str().map(str::to_string)).collect()).unwrap_or_default()
}

const SRC: &str = "pub struct Writer;\n\
impl Writer { pub fn go(&self) { std::fs::write(std::env::temp_dir().join(\"r1661\"), b\"x\").unwrap(); } }\n\
pub mod m { use super::*; pub struct S { pub f: (Writer, u8) }\n\
impl S { pub fn m2_self_zq(&self) { self.f.0.go(); } }\n\
pub fn m2_bare_zq(s: &S) { s.f.0.go(); } }\n\
pub mod n { pub fn m2_qual_zq(s: &crate::m::S) { s.f.0.go(); } }\n";

#[test]
fn r1661_the_crate_qualified_spelling_keeps_the_untyped_receiver_floor() {
    let d = fixture("m2x", SRC);
    let (_, on) = scan(&d, None, false);
    for f in ["m::S::m2_self_zq", "m::m2_bare_zq", "n::m2_qual_zq"] {
        assert_eq!(inferred(&on, f), ["Unknown"], "{f}: {on:#}");
        let why = row(&on, f).and_then(|r| r["unknownWhy"].as_array()).cloned().unwrap_or_default();
        assert!(why.iter().any(|w| w.as_str().is_some_and(|w| w.starts_with("dispatch:untyped receiver of `go`"))), "{f}: {on:#}");
    }
    let (rc, _) = scan(&d, Some("deny Unknown n::m2_qual_zq\n"), false);
    assert_eq!(rc, 1, "the disclosure reaches the gate");
    // The kill switch restores the silence on the qualified spelling only, and a gate under it is unanswerable.
    let (_, off) = scan(&d, None, true);
    assert!(row(&off, "n::m2_qual_zq").is_none(), "CANDOR_R1661_OFF restores the ABSENT row {off:#}");
    // SOUNDNESS R1657 — with origin qualification the bare spelling is `crate::m::S` too, so under CANDOR_R1661_OFF it forms
    // the same inert marker and is ABSENT; R1661 is what keeps both spellings disclosed.
    assert!(row(&off, "m::m2_bare_zq").is_none(), "R1657 anchors the bare spelling onto the R1661 path {off:#}");
    let (refused, _) = scan(&d, Some("deny Unknown n::m2_qual_zq\n"), true);
    assert_eq!(refused, 2);
    let _ = std::fs::remove_dir_all(&d);
}

#[test]
fn r1692_subsumes_the_crate_rooted_marker() {
    let d = fixture("m2x1692", SRC);
    let mut cmd = Command::new(bin());
    cmd.arg(d.to_string_lossy().as_ref()).arg("--json");
    for (k, _) in std::env::vars() {
        if k.starts_with("CANDOR_") {
            cmd.env_remove(k);
        }
    }
    cmd.env("CANDOR_R1661_OFF", "1");
    let out = cmd.output().expect("run candor-scan");
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).expect("report json");
    assert_eq!(inferred(&v, "n::m2_qual_zq"), ["Unknown"], "R1692 keeps the disclosure with R1661 off {v:#}");
    let _ = std::fs::remove_dir_all(&d);
}
