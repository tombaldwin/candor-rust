//! SOUNDNESS R1666 (union half) — AN INTERFACE-UNION ROW MUST NOT BE SUPPRESSED WHILE IT CARRIES A REASON CLASS THE REAL ROW LACKS.
//!
//! The beside-union row is withheld when the real row at its hash already carries every effect, because `Unknown` covers any effect.
//! It does not cover a reason CLASS: with the real default body `Unknown[dispatch]` and the implementor `Unknown[indirect]`
//! (a stored callback), the union row was dropped and `deny Unknown[indirect]` read 1 -> 0 for a chained consumer. Pre-existing on
//! 4df3788 (a `dup()` ambiguity hedge on the default body suppresses it identically); the R1666 floor exposed it on object-0.24.0
//! `Object::symbol_map` by giving more default bodies an `Unknown` of their own.

use std::path::{Path, PathBuf};
use std::process::Command;

fn bin() -> &'static str {
    env!("CARGO_BIN_EXE_candor-scan")
}

fn fixture(tag: &str, src: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("candor-r1666-{tag}-{}", std::process::id()));
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
    cmd.env_remove("CANDOR_DEPS").env_remove("CANDOR_CONFIG").env_remove("CANDOR_POLICY").env_remove("CANDOR_R1666B_OFF");
    if off {
        cmd.env("CANDOR_R1666B_OFF", "1");
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

fn why(v: &serde_json::Value, f: &str) -> Vec<String> {
    row(v, f).and_then(|r| r["unknownWhy"].as_array()).map(|a| a.iter().filter_map(|e| e.as_str().map(str::to_string)).collect()).unwrap_or_default()
}

const SRC: &str = "pub struct E { pub cb: Box<dyn Fn()> }\n\
impl E { pub fn fire_zq(&self) { (self.cb)(); } }\n\
pub trait Tr { fn m_zq(&self, r: Result<u8, E>) -> Result<u8, E> { r.map_err(|e| { e.fire_zq(); e }) } }\n\
pub struct X { pub cb: Box<dyn Fn()> }\n\
impl Tr for X { fn m_zq(&self, r: Result<u8, E>) -> Result<u8, E> { (self.cb)(); r } }\n";

fn union_why(v: &serde_json::Value) -> Vec<String> {
    v["functions"].as_array().unwrap().iter().find(|r| r["fn"].as_str() == Some("Tr::m_zq") && r["interfaceUnion"].as_bool() == Some(true))
        .and_then(|r| r["unknownWhy"].as_array()).map(|a| a.iter().filter_map(|e| e.as_str().map(str::to_string)).collect()).unwrap_or_default()
}

#[test]
fn r1666_a_union_row_keeps_a_reason_class_the_real_row_lacks() {
    let d = fixture("uni", SRC);
    let (_, on) = scan(&d, None, false);
    // The real default body is `dispatch` only (the untyped-receiver floor), the implementor is a stored callback.
    assert!(why(&on, "Tr::m_zq").iter().any(|w| w.starts_with("dispatch:untyped receiver of `fire_zq`")), "{on:#}");
    assert!(union_why(&on).iter().any(|w| w.starts_with("callback:")), "the union row keeps its `indirect` class {on:#}");
    let (_, off) = scan(&d, None, true);
    assert!(union_why(&off).is_empty(), "CANDOR_R1666B_OFF restores the suppression {off:#}");
    let _ = std::fs::remove_dir_all(&d);
}
