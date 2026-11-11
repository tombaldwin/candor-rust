//! SOUNDNESS R1685 — A TRAIT MEMBER DECLARED BY A MACRO IN THE TRAIT BODY, CALLED BY ITS PATH.
//!
//! time 0.3.53–0.3.55: `pub(crate) trait ComponentProvider { unimplemented_methods! { .. ("time") period => Period; .. } }`
//! and `impl ComponentProvider for Timestamp { fn period(&self, state) { let time = state.time.get_or_insert_with(..);
//! ComponentProvider::period(time, &mut ()) } }`. The trait's item list does not show `period` (an item-position macro), so
//! the PATH call reached no dispatch decision: no edge to `<Time as ComponentProvider>::period`, no hedge, on every arm
//! including published 0.41.3 — and the same for a TYPED first argument. A local `impl` declaring the member proves it
//! (E0407). EXECUTED (lane scratch `rustagent-r1682/f2`, `cargo run --example run`: `Time::period` wrote its file through
//! `Stamp::period`).
use std::path::{Path, PathBuf};
use std::process::Command;

fn bin() -> &'static str {
    env!("CARGO_BIN_EXE_candor-scan")
}

fn fixture(tag: &str, files: &[(&str, &str)]) -> PathBuf {
    let d = std::env::temp_dir().join(format!("candor-r1685-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(d.join("src")).unwrap();
    std::fs::write(d.join("Cargo.toml"), "[package]\nname = \"f2\"\nversion = \"0.1.0\"\nedition = \"2021\"\n[dependencies]\n").unwrap();
    for (p, s) in files {
        let f = d.join("src").join(p);
        std::fs::create_dir_all(f.parent().unwrap()).unwrap();
        std::fs::write(f, s).unwrap();
    }
    d
}

fn scan(dir: &Path, policy: Option<&str>, off: &[&str]) -> (i32, serde_json::Value) {
    let mut cmd = Command::new(bin());
    cmd.arg(dir.to_string_lossy().as_ref()).arg("--json");
    if let Some(p) = policy {
        let pf = dir.join("policy.txt");
        std::fs::write(&pf, p).unwrap();
        cmd.arg("--policy").arg(&pf);
    }
    for v in ["CANDOR_DEPS", "CANDOR_CONFIG", "CANDOR_POLICY", "CANDOR_R1685_OFF"] {
        cmd.env_remove(v);
    }
    for v in off {
        cmd.env(v, "1");
    }
    let out = cmd.output().expect("run candor-scan");
    let code = out.status.code().unwrap_or(-1);
    assert!((0..=2).contains(&code), "scan failed ({code}): {}", String::from_utf8_lossy(&out.stderr));
    (code, serde_json::from_slice(&out.stdout).expect("report json"))
}

fn field(v: &serde_json::Value, f: &str, key: &str) -> Vec<String> {
    let row = v["functions"].as_array().unwrap().iter().find(|r| r["fn"].as_str() == Some(f));
    let mut out: Vec<String> = row
        .and_then(|r| r[key].as_array())
        .map(|a| a.iter().filter_map(|e| e.as_str().map(str::to_string)).collect())
        .unwrap_or_default();
    out.sort();
    out
}

const F2: &[(&str, &str)] = &[
    ("lib.rs", r####"macro_rules! unimpl { ($($n:ident)*) => { $(fn $n(&self, _st: &mut ()) -> u8 { unimplemented!() })* } }
pub(crate) trait Prov { unimpl! { period } }
pub struct Time;
pub struct Date;
pub struct Stamp { cache: Option<Time> }
fn w() { std::fs::write(std::env::var("OUT").unwrap() + "/" + "time", b"x").unwrap(); }
impl Prov for Time { fn period(&self, _: &mut ()) -> u8 { w(); 1 } }
impl Prov for Date { fn period(&self, _: &mut ()) -> u8 { 2 } }
impl Stamp {
    fn time(&self) -> Time { Time }
}
impl Prov for Stamp {
    fn period(&self, _: &mut ()) -> u8 {
        let mut c: Option<Time> = None;
        let t = c.get_or_insert_with(|| self.time());
        Prov::period(t, &mut ())
    }
}
pub fn stamp_period_untyped() -> u8 { let s = Stamp { cache: None }; let _ = &s.cache; Prov::period(&s, &mut ()) }
pub fn typed_ufcs() -> u8 { let t = Time; Prov::period(&t, &mut ()) }
pub fn untyped_ufcs(x: &dyn std::any::Any) -> u8 { let t = x.downcast_ref::<Time>().unwrap(); Prov::period(t, &mut ()) }
pub fn untyped_ufcs2() -> u8 { let mut c: Option<Time> = None; let t = c.get_or_insert_with(|| Time); Prov::period(t, &mut ()) }
pub fn untyped_dot() -> u8 { let mut c: Option<Time> = None; let t = c.get_or_insert_with(|| Time); t.period(&mut ()) }
"####),
];

#[test]
fn r1685_a_path_call_to_a_macro_declared_trait_member_dispatches_over_the_impls() {
    let d = fixture("f2", F2);
    let (_, v) = scan(&d, None, &[]);
    for f in ["Stamp::period", "stamp_period_untyped", "untyped_ufcs", "untyped_ufcs2", "typed_ufcs"] {
        assert!(field(&v, f, "inferred").contains(&"Fs".to_string()), "`{f}`: {:?}", field(&v, f, "inferred"));
        let (code, _) = scan(&d, Some(&format!("deny Fs {f}\n")), &[]);
        assert_eq!(code, 1, "`deny Fs {f}` must fire");
    }
    let (_, off) = scan(&d, None, &["CANDOR_R1685_OFF"]);
    for f in ["Stamp::period", "stamp_period_untyped", "typed_ufcs"] {
        assert!(field(&off, f, "inferred").is_empty(), "the switch restores the silence on `{f}` (proves the fixture reaches the code)");
    }
}
