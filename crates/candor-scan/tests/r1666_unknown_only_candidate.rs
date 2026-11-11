//! SOUNDNESS R1666 — THE R1095 UNTYPED-RECEIVER FLOOR MUST COUNT A CANDIDATE WHOSE ONLY EFFECT IS `Unknown`.
//!
//! `r.map_err(|e| { e.fire_zq(); e })`: the closure parameter is typed by no route, and the only crate method of that name,
//! `E::fire_zq`, calls a stored callback, so its report is `Unknown` and nothing else. The floor excluded such a candidate (the
//! std-trait names dominated its first measurement), so the caller read ABSENT. EXECUTED (lane scratch `rustagent-floors/fx/f66`,
//! the binary writes the marker through the stored callback): `caller_zq` was ABSENT on published 0.41.3 and on 791649c,
//! `deny Unknown` 0. Now it discloses, for a method std has no member of that name for (basic-toml's `Error::fix_offset`).

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
    cmd.env_remove("CANDOR_DEPS").env_remove("CANDOR_CONFIG").env_remove("CANDOR_POLICY").env_remove("CANDOR_R1666_OFF");
    if off {
        cmd.env("CANDOR_R1666_OFF", "1");
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

fn why(v: &serde_json::Value, f: &str) -> Vec<String> {
    row(v, f).and_then(|r| r["unknownWhy"].as_array()).map(|a| a.iter().filter_map(|e| e.as_str().map(str::to_string)).collect()).unwrap_or_default()
}

// `E::fire_zq` calls a stored callback (so it is `Unknown`-only); `E::calm_zq` is pure; `E::map` carries a std name.
const SRC: &str = "pub struct E { pub cb: Box<dyn Fn()> }\n\
impl E { pub fn fire_zq(&self) { (self.cb)(); } pub fn calm_zq(&self) { let _ = 1 + 1; } pub fn clone_w_zq(&self) { (self.cb)(); } pub fn len(&self) { (self.cb)(); } }\n\
pub fn mk() -> E { E { cb: Box::new(|| { std::fs::write(std::env::temp_dir().join(\"r1666\"), b\"x\").unwrap(); }) } }\n\
pub fn caller_zq(r: Result<u8, E>) -> Result<u8, E> { r.map_err(|e| { e.fire_zq(); e }) }\n\
pub fn pure_twin_zq(r: Result<u8, E>) -> Result<u8, E> { r.map_err(|e| { e.calm_zq(); e }) }\n\
pub fn std_named_zq(r: Result<u8, E>) -> Result<u8, E> { r.map_err(|e| { e.len(); e }) }\n";

#[test]
fn r1666_an_unknown_only_candidate_counts_for_a_name_std_lacks() {
    let d = fixture("unk", SRC);
    let (_, on) = scan(&d, None, false);
    assert_eq!(inferred(&on, "caller_zq"), ["Unknown"], "{on:#}");
    assert!(why(&on, "caller_zq").iter().any(|w| w.starts_with("dispatch:untyped receiver of `fire_zq`")), "{on:#}");
    // A pure candidate discloses nothing; a std-named method keeps the declined form's silence (the std-trait names it priced).
    assert!(row(&on, "pure_twin_zq").is_none(), "{on:#}");
    assert!(row(&on, "std_named_zq").is_none(), "{on:#}");
    let (rc, _) = scan(&d, Some("deny Unknown caller_zq\n"), false);
    assert_eq!(rc, 1, "the disclosure reaches the gate");
    let (rc_pure, _) = scan(&d, Some("deny Unknown pure_twin_zq\n"), false);
    assert_eq!(rc_pure, 0);
    let (_, off) = scan(&d, None, true);
    assert!(row(&off, "caller_zq").is_none(), "CANDOR_R1666_OFF restores the ABSENT row {off:#}");
    let (refused, _) = scan(&d, Some("deny Unknown caller_zq\n"), true);
    assert_eq!(refused, 2);
    let _ = std::fs::remove_dir_all(&d);
}
