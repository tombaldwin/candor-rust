//! SOUNDNESS R1658 / R1659 — a generic FIELD accessor (`impl<F> W<F> { fn get(&mut self) -> &mut F }`) reached as
//! `h.w.get().<method>()` lost the receiver's type in two ways, both ABSENT (a §2 purity claim over a real effect) on
//! published 0.41.3 and EXECUTED (rustagent-fxg/fx/fxg, fx/fxg2: every `*_read` / `*_poke` fixture fn reads or writes a
//! real file when run).
//!
//!  * R1658 — the field's written argument is a std I/O handle (`u::U<std::fs::File>`): the field branch of
//!    `impl_generic_return` asked R452's gate for a crate `File::read`, which cannot exist. Resolved: `Fs`.
//!  * R1659 — the accessor's owner has a same-leaf TWIN (`a::W<F>`, `b::W<X, F>`), so `file_decl_fact` withdraws the leaf-keyed
//!    position and the read returned `None` as if there were no accessor. Disclosed: `Unknown` (`ambiguous:`), not resolved —
//!    the resolution is the value-typing restructure's stage 3.
//!
//! Fixture fns carry a unique `_zq` terminator because `deny <E> <fn>` matches by PREFIX.

use std::path::{Path, PathBuf};
use std::process::Command;

fn bin() -> &'static str {
    env!("CARGO_BIN_EXE_candor-scan")
}

fn fixture(tag: &str, src: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("candor-r1658-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(d.join("src")).unwrap();
    std::fs::write(d.join("Cargo.toml"), "[package]\nname = \"fa\"\nversion = \"0.1.0\"\nedition = \"2021\"\n").unwrap();
    std::fs::write(d.join("src/lib.rs"), src).unwrap();
    d
}

fn scan(dir: &Path, off: Option<&str>) -> serde_json::Value {
    let mut cmd = Command::new(bin());
    cmd.arg(dir.to_string_lossy().as_ref()).arg("--json");
    cmd.env_remove("CANDOR_DEPS").env_remove("CANDOR_CONFIG").env_remove("CANDOR_POLICY");
    cmd.env_remove("CANDOR_R1658_OFF").env_remove("CANDOR_R1659_OFF");
    if let Some(sw) = off {
        cmd.env(sw, "1");
    }
    let o = cmd.output().expect("run candor-scan");
    let code = o.status.code().unwrap_or(-1);
    assert!(code == 0 || code == 1, "scan failed ({code}): {}", String::from_utf8_lossy(&o.stderr));
    serde_json::from_slice(&o.stdout).expect("report json")
}

fn gate(dir: &Path, rule: &str) -> i32 {
    let pol = dir.join("policy.txt");
    std::fs::write(&pol, format!("{rule}\n")).unwrap();
    let o = Command::new(bin())
        .arg(dir.to_string_lossy().as_ref())
        .args(["--policy", pol.to_string_lossy().as_ref(), "--out", dir.join("r").to_string_lossy().as_ref()])
        .env_remove("CANDOR_DEPS").env_remove("CANDOR_CONFIG").env_remove("CANDOR_POLICY")
        .output()
        .expect("run candor-scan");
    o.status.code().unwrap_or(-1)
}

fn row<'a>(v: &'a serde_json::Value, f: &str) -> Option<&'a serde_json::Value> {
    v["functions"].as_array().unwrap().iter().find(|r| r["fn"].as_str() == Some(f))
}

fn inferred(v: &serde_json::Value, f: &str) -> Vec<String> {
    let mut out: Vec<String> = row(v, f)
        .and_then(|r| r["inferred"].as_array())
        .map(|a| a.iter().filter_map(|e| e.as_str().map(str::to_string)).collect())
        .unwrap_or_default();
    out.sort();
    out
}

fn why(v: &serde_json::Value, f: &str) -> Vec<String> {
    row(v, f)
        .and_then(|r| r["unknownWhy"].as_array())
        .map(|a| a.iter().filter_map(|e| e.as_str().map(str::to_string)).collect())
        .unwrap_or_default()
}

const SRC: &str = "\
use std::io::Read;\n\
pub mod a { pub struct W<F> { pub f: F } impl<F> W<F> { pub fn get(&mut self) -> &mut F { &mut self.f } } }\n\
pub mod b { pub struct W<X, F> { pub x: X, pub f: F } impl<X, F> W<X, F> { pub fn get(&mut self) -> &mut F { &mut self.f } } }\n\
pub mod u { pub struct U<F> { pub f: F } impl<F> U<F> { pub fn get(&mut self) -> &mut F { &mut self.f } } }\n\
pub struct Eff;\n\
impl Eff { pub fn poke_zq(&mut self) { std::fs::write(\"/tmp/r1659_x\", b\"x\").unwrap(); } }\n\
pub struct Quiet;\n\
impl Quiet { pub fn noop_zq(&mut self) -> u8 { 1 } }\n\
pub struct HFile { pub w: a::W<std::fs::File> }\n\
pub struct HUFile { pub w: u::U<std::fs::File> }\n\
pub struct HUVec { pub w: u::U<Vec<u8>> }\n\
pub struct HEff { pub w: a::W<Eff> }\n\
pub struct HUEff { pub w: u::U<Eff> }\n\
pub struct HQuiet { pub w: a::W<Quiet> }\n\
pub struct HUQuiet { pub w: u::U<Quiet> }\n\
pub fn uniq_read_zq(h: &mut HUFile, buf: &mut [u8]) -> usize { h.w.get().read(buf).unwrap() }\n\
pub fn uniq_len_zq(h: &mut HUVec) -> usize { h.w.get().len() }\n\
pub fn twin_read_zq(h: &mut HFile, buf: &mut [u8]) -> usize { h.w.get().read(buf).unwrap() }\n\
pub fn twin_poke_zq(h: &mut HEff) { h.w.get().poke_zq() }\n\
pub fn uniq_poke_zq(h: &mut HUEff) { h.w.get().poke_zq() }\n\
pub fn twin_quiet_zq(h: &mut HQuiet) -> u8 { h.w.get().noop_zq() }\n\
pub fn uniq_quiet_zq(h: &mut HUQuiet) -> u8 { h.w.get().noop_zq() }\n";

#[test]
fn r1658_unique_owner_over_a_std_handle_charges_fs() {
    let d = fixture("h", SRC);
    let on = scan(&d, None);
    assert_eq!(inferred(&on, "uniq_read_zq"), ["Fs"], "{on}");
    // the same shape over a NON-handle argument stays pure — the resolution adds `Fs` only where the argument is a handle
    assert!(row(&on, "uniq_len_zq").is_none(), "pure unique twin must stay absent: {on}");
    // single variable: the switch removes exactly the row (published 0.41.3 reads it ABSENT)
    let off = scan(&d, Some("CANDOR_R1658_OFF"));
    assert!(row(&off, "uniq_read_zq").is_none(), "R1658 off must reproduce the silence: {off}");
    assert_eq!(gate(&d, "deny Fs uniq_read_zq"), 1);
    assert_eq!(gate(&d, "deny Fs uniq_len_zq"), 0);
}

#[test]
fn r1659_twin_owner_withdrawn_position_discloses() {
    let d = fixture("t", SRC);
    let on = scan(&d, None);
    for f in ["twin_poke_zq", "twin_read_zq"] {
        assert_eq!(inferred(&on, f), ["Unknown"], "{f}: {on}");
        let w = why(&on, f);
        assert!(w.len() == 1 && w[0].starts_with("ambiguous:") && w[0].contains("W::get") && w[0].contains("withdrawn"), "{f}: {w:?}");
    }
    // the unique-owner twin of the same body is RESOLVED, not hedged
    assert_eq!(inferred(&on, "uniq_poke_zq"), ["Fs"], "{on}");
    assert!(why(&on, "uniq_poke_zq").is_empty());
    // a unique pure twin stays absent (its name is unique, so the prefix gate cannot read another row)
    assert!(row(&on, "uniq_quiet_zq").is_none(), "{on}");
    // gate flips: published reads ABSENT / exit 0
    let off = scan(&d, Some("CANDOR_R1659_OFF"));
    assert!(row(&off, "twin_poke_zq").is_none(), "R1659 off must reproduce the silence: {off}");
    assert_eq!(gate(&d, "deny Fs Unknown twin_poke_zq"), 1);
    assert_eq!(gate(&d, "deny Unknown twin_poke_zq"), 1);
    assert_eq!(gate(&d, "deny Fs Unknown uniq_quiet_zq"), 0);
}
