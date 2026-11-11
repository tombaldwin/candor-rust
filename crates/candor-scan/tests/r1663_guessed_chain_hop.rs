//! SOUNDNESS R1663 — A CHAIN HOP THE WALK GUESSED (`x.sel().go()`, `sel` declared to return ANOTHER type) MUST NOT READ ABSENT.
//!
//! `resolve_recv_type_for`'s builder-chain walk assumes every step returns its receiver's type, so `x.sel().go()` formed the
//! typed call `A::go` on `x`'s own type. Where `A` declares no `go` that call names no unit and was DROPPED with no disclosure.
//! EXECUTED (lane scratch `rustagent-floors/fx/f63`, the binary writes the marker): `c::hop_zq` read ABSENT on published
//! 0.41.3 and on 4df3788, `deny Fs` and `deny Unknown` both 0. Now the guessed-hop miss is judged by the R1095 floor, with an
//! `Unknown`-only candidate counted (moka's `Cache::insert_with_hash` was hedged all the way down). Fixture fns carry `_zq`
//! because `deny` matches by prefix.

use std::path::{Path, PathBuf};
use std::process::Command;

fn bin() -> &'static str {
    env!("CARGO_BIN_EXE_candor-scan")
}

fn fixture(tag: &str, src: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("candor-r1663-{tag}-{}", std::process::id()));
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
    cmd.env_remove("CANDOR_DEPS").env_remove("CANDOR_CONFIG").env_remove("CANDOR_POLICY").env_remove("CANDOR_R1663_OFF");
    if off {
        cmd.env("CANDOR_R1663_OFF", "1");
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

// `b::B::go` writes; `b2::B` is a pure twin leaf (so `sel`'s declared `-> &B` is a twin and `impl_declared_return`'s second gate
// declines). `c::hop_zq` is the guessed hop; `c::twin_pure_zq` calls the pure twin directly; `c::builder_zq` is a real builder
// hop (`-> &Self`), which is NOT a guess; `c::hop_has_zq` hops to a base that declares the same-named method (a different row's
// fabrication, left alone here).
const SRC: &str = "pub mod b { pub struct B; impl B { pub fn go(&self) { std::fs::write(std::env::temp_dir().join(\"r1663\"), b\"x\").unwrap(); } } }\n\
pub mod b2 { pub struct B; impl B { pub fn go(&self) {} } }\n\
pub mod a { use crate::b::B; pub struct A { pub bb: B }\n\
impl A { pub fn sel(&self) -> &B { &self.bb } pub fn me(&self) -> &A { self } pub fn only_a_zq(&self) {} } }\n\
pub mod c {\n\
pub fn hop_zq(x: &crate::a::A) { x.sel().go(); }\n\
pub fn twin_pure_zq(x: &crate::b2::B) { x.go(); }\n\
pub fn builder_zq(x: &crate::a::A) { x.me().only_a_zq(); }\n\
}\n";

#[test]
fn r1663_a_guessed_hop_that_names_no_unit_discloses() {
    let d = fixture("hop", SRC);
    let (_, on) = scan(&d, None, false);
    assert_eq!(inferred(&on, "c::hop_zq"), ["Unknown"], "{on:#}");
    assert!(why(&on, "c::hop_zq").iter().any(|w| w.starts_with("dispatch:untyped receiver of `go`")), "{on:#}");
    // The pure twin and the real builder hop are not guesses.
    assert!(row(&on, "c::twin_pure_zq").is_none(), "{on:#}");
    assert!(row(&on, "c::builder_zq").is_none(), "{on:#}");
    let (rc, _) = scan(&d, Some("deny Unknown c::hop_zq\n"), false);
    assert_eq!(rc, 1, "the disclosure reaches the gate");
    let (rc_pure, _) = scan(&d, Some("deny Unknown c::twin_pure_zq\n"), false);
    assert_eq!(rc_pure, 0, "the pure twin stays quiet");
    // The kill switch restores the silence, and a gate under it is unanswerable.
    let (_, off) = scan(&d, None, true);
    assert!(row(&off, "c::hop_zq").is_none(), "CANDOR_R1663_OFF restores the ABSENT row {off:#}");
    let (refused, _) = scan(&d, Some("deny Unknown c::hop_zq\n"), true);
    assert_eq!(refused, 2);
    let _ = std::fs::remove_dir_all(&d);
}
