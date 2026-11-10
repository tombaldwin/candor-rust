//! SOUNDNESS R1609, R1616 — three spellings whose RECEIVER's type never reached the classifier or the dispatch route, each
//! BUILT AND EXECUTED in the lane (scratchpad `rustagent-recv/fx/t9` and `fx/c`, DYLD interposer for the dependency cases):
//!
//!   * R1609 — `let f = h.gen::<F>(); f.all()` over `fn gen<T: Default>(&self) -> T`: the binding dispatched over the BOUND
//!     (`Default`, no `all`) and the caller read ABSENT over a write. The turbofish's type now types it; a turbofish naming
//!     the caller's own generic `B` dispatches over `B`'s bounds beside the callee's (bitflags' `from_str::<B>` shape).
//!   * R1616 (UFCS) — `<backoff::ExponentialBackoff as Backoff>::next_backoff(b)` formed only the trait path, which no rule
//!     claims: ABSENT over 1 clock read + 1 getentropy, while `b.next_backoff()` charged `[Clock, Rand]`.
//!   * R1616 (provided member) — `chrono::Local.from_utc_datetime(n)` reached the classifier as `chrono::Local::
//!     from_utc_datetime`, a `TimeZone` PROVIDED member no row named: ABSENT over 2 clock reads + `TZ` + /etc/localtime.
//!
//! Each `*_OFF` switch restores the old reading and the defect cases must then read silent again — the proof that the
//! fixtures reach the change. Pure controls must stay absent.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Command;

const LOCAL: &str = r#"
#![allow(dead_code)]
fn hit(n: &str) { let _ = std::fs::write(format!("/tmp/r1609-never-run-{n}"), "x"); }
pub struct H;
#[derive(Default)] pub struct F(u8);
#[derive(Default)] pub struct Q(u8);
impl H {
    pub fn new() -> Self { H }
    pub fn gen<T: Default>(&self) -> T { T::default() }
    pub fn mk<T: Default>(&self) -> Result<T, String> { Ok(T::default()) }
    pub fn me(self) -> Self { self }
}
impl F { pub fn all(self) { hit("F.all") } pub fn me2(self) -> Self { self } }
impl Q { pub fn all(self) {} }
pub trait Gen { fn make<T: Default>(&self) -> T; }
impl Gen for H { fn make<T: Default>(&self) -> T { T::default() } }
pub trait Flags { fn bits(&self) -> u8; }
impl Flags for F { fn bits(&self) -> u8 { hit("F.bits"); 1 } }
impl Flags for Q { fn bits(&self) -> u8 { 0 } }

pub fn t_let() { let f = H::new().gen::<F>(); f.all() }
pub fn t_inline() { H::new().gen::<F>().all() }
pub fn t_param(h: &H) { let f = h.gen::<F>(); f.all() }
pub fn t_param_inline(h: &H) { h.gen::<F>().all() }
pub fn t_try(h: &H) -> Result<(), String> { let f = h.mk::<F>()?; f.all(); Ok(()) }
pub fn t_mid_chain() { let f = H::new().me().gen::<F>().me2(); f.all() }
pub fn t_dyn_trait(g: &impl Gen) { let f = g.make::<F>(); f.all() }
pub fn t_caller_generic<B: Flags + Default>(h: &H) -> u8 { let b = h.gen::<B>(); b.bits() }
pub fn c_q(h: &H) { let q = h.gen::<Q>(); q.all() }
pub fn c_q_inline() { H::new().gen::<Q>().all() }
pub fn c_std_parse() -> u32 { "5".parse::<u32>().unwrap_or(0) }
"#;

const DEPS: &str = r#"
use backoff::backoff::Backoff;
use chrono::TimeZone;
pub fn f_bo_next_ufcs(b: &mut backoff::ExponentialBackoff) -> Option<std::time::Duration> { <backoff::ExponentialBackoff as Backoff>::next_backoff(b) }
pub fn f_bo_ufcs_default() -> backoff::ExponentialBackoff { <backoff::ExponentialBackoff as Default>::default() }
pub fn f_bo_ufcs_reset(b: &mut backoff::ExponentialBackoff) { <backoff::ExponentialBackoff as Backoff>::reset(b) }
pub fn f_tz_from_utc(n: &chrono::NaiveDateTime) -> chrono::DateTime<chrono::Local> { chrono::Local.from_utc_datetime(n) }
pub fn f_tz_ts_opt() -> bool { chrono::Local.timestamp_opt(1_600_000_000, 0).single().is_some() }
pub fn f_tz_ymd_hms() -> bool { chrono::Local.with_ymd_and_hms(2020, 1, 2, 3, 4, 5).single().is_some() }
pub fn f_tz_from_local(n: &chrono::NaiveDateTime) -> bool { chrono::Local.from_local_datetime(n).single().is_some() }
pub fn c_tz_utc(n: &chrono::NaiveDateTime) -> chrono::DateTime<chrono::Utc> { chrono::Utc.from_utc_datetime(n) }
pub fn c_tz_utc_ts() -> bool { chrono::Utc.timestamp_opt(1_600_000_000, 0).single().is_some() }
pub fn c_bo_ufcs_zero() -> Option<std::time::Duration> { let mut z = backoff::backoff::Zero {}; <backoff::backoff::Zero as Backoff>::next_backoff(&mut z) }
pub fn c_bo_ufcs_clone(b: &backoff::ExponentialBackoff) -> backoff::ExponentialBackoff { <backoff::ExponentialBackoff as Clone>::clone(b) }
"#;

fn fixture(tag: &str, deps: &str, src: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("candor-r1609-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(d.join("src")).unwrap();
    std::fs::write(
        d.join("Cargo.toml"),
        format!("[package]\nname = \"fx\"\nversion = \"0.1.0\"\nedition = \"2021\"\n[dependencies]\n{deps}"),
    )
    .unwrap();
    std::fs::write(d.join("src/lib.rs"), src).unwrap();
    d
}

fn scan(dir: &Path, off: &[&str]) -> HashMap<String, Vec<String>> {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_candor-scan"));
    cmd.arg(dir.to_string_lossy().as_ref()).arg("--json");
    for k in ["CANDOR_DEPS", "CANDOR_CONFIG", "CANDOR_POLICY", "CANDOR_R1609_OFF", "CANDOR_R1616U_OFF"] {
        cmd.env_remove(k);
    }
    for k in off {
        cmd.env(k, "1");
    }
    let out = cmd.output().expect("run candor-scan");
    assert!(out.status.code().is_some_and(|c| c == 0 || c == 1), "scan failed: {}", String::from_utf8_lossy(&out.stderr));
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).expect("report json");
    v["functions"]
        .as_array()
        .unwrap()
        .iter()
        .map(|f| {
            let mut e: Vec<String> = f["inferred"].as_array().unwrap().iter().filter_map(|x| x.as_str().map(str::to_string)).collect();
            e.sort();
            (f["fn"].as_str().unwrap().to_string(), e)
        })
        .collect()
}

fn inf(rows: &HashMap<String, Vec<String>>, f: &str) -> Vec<String> {
    rows.get(f).cloned().unwrap_or_default()
}

fn v(x: &[&str]) -> Vec<String> {
    x.iter().map(|s| s.to_string()).collect()
}

const R1609_DEFECT: &[&str] = &["t_let", "t_inline", "t_param", "t_param_inline", "t_try", "t_mid_chain", "t_dyn_trait"];

#[test]
fn r1609_a_method_turbofish_types_the_generic_return() {
    let d = fixture("tf", "", LOCAL);
    let on = scan(&d, &[]);
    for f in R1609_DEFECT {
        assert_eq!(inf(&on, f), v(&["Fs"]), "{f}: `F::all` runs");
    }
    // the turbofish names the CALLER's generic: `B`'s bounds dispatch (bounded CHA over the local `Flags` implementors)
    assert_eq!(inf(&on, "t_caller_generic"), v(&["Fs"]), "t_caller_generic: `B: Flags` reaches `F::bits`");
    for f in ["c_q", "c_q_inline", "c_std_parse"] {
        assert!(inf(&on, f).is_empty(), "{f}: a pure turbofish target must not gain a charge: {:?}", inf(&on, f));
    }
    // reach: with the switch off, every defect case is not charged `Fs` again (silent, or the old floor)
    let off = scan(&d, &["CANDOR_R1609_OFF"]);
    for f in R1609_DEFECT.iter().chain(&["t_caller_generic"]) {
        assert!(!inf(&off, f).contains(&"Fs".to_string()), "{f}: CANDOR_R1609_OFF must restore the old reading");
    }
}

#[test]
fn r1616_ufcs_on_a_dependency_type_and_chrono_local_provided_members_are_charged() {
    let d = fixture("dep", "backoff = \"0.4\"\nchrono = \"0.4\"\n", DEPS);
    let on = scan(&d, &[]);
    assert_eq!(inf(&on, "f_bo_next_ufcs"), v(&["Clock", "Rand"]));
    assert_eq!(inf(&on, "f_bo_ufcs_default"), v(&["Clock"]));
    assert_eq!(inf(&on, "f_bo_ufcs_reset"), v(&["Clock"]));
    for f in ["f_tz_from_utc", "f_tz_ts_opt", "f_tz_ymd_hms", "f_tz_from_local"] {
        assert_eq!(inf(&on, f), v(&["Clock"]), "{f}");
    }
    for f in ["c_tz_utc", "c_tz_utc_ts", "c_bo_ufcs_zero", "c_bo_ufcs_clone"] {
        assert!(inf(&on, f).is_empty(), "{f}: pure control charged {:?}", inf(&on, f));
    }
    let off = scan(&d, &["CANDOR_R1616U_OFF"]);
    for f in ["f_bo_next_ufcs", "f_bo_ufcs_default", "f_bo_ufcs_reset"] {
        assert!(inf(&off, f).is_empty(), "{f}: CANDOR_R1616U_OFF must restore the trait-path-only reading");
    }
}
