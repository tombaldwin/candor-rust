//! SOUNDNESS R1624 — two same-leaf structs in different modules, each declaring a same-named field of a DIFFERENT type.
//! `fields` is keyed by the struct's LEAF and keeps the last declaration merged, so `self.f.m()` in the other twin was typed
//! with the wrong field type: a silence when the last twin's type is pure, a fabrication when it is effectful. The owner's own
//! declaration now answers through the module-qualified index (`FileDecls::qual`, four indexes: fields, field_elem, trait_fields, field_elem_trait) wherever the owner's path is proven
//! (`self` in its own module, a crate-anchored type).
//!
//! The local fixture is EXECUTED ground truth (handoff `rust-r1624/fx/r1624`, `cargo run`): `a_self`, `a_bind` and
//! `resid_param` write a temp file, `fa_self` writes another, `c_self` runs `File::sync_all`, `f_self` reads an env var; the
//! `b_*`, `d_*`, `e_*` and `fb_*` twins do nothing. On 544ed9f the first four were ABSENT (`deny Fs` exit 0 over an executed
//! write) and `e_self` / `fb_self` were charged their twin's effects. The sqlx shape is sea-schema 0.18.1's
//! `{mysql,postgres,sqlite}::executor::real::Executor { pool }`, with the per-driver acquire rows of R1623 (executed there).
//!
//! Fixture fns carry a unique `_zq` terminator because `deny <E> <fn>` matches by PREFIX.

use std::path::{Path, PathBuf};
use std::process::Command;

fn bin() -> &'static str {
    env!("CARGO_BIN_EXE_candor-scan")
}

fn fixture(tag: &str, deps: &str, lock: Option<&str>, files: &[(&str, &str)]) -> PathBuf {
    let d = std::env::temp_dir().join(format!("candor-r1624-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(d.join("src")).unwrap();
    std::fs::write(d.join("Cargo.toml"), format!("[package]\nname = \"fx\"\nversion = \"0.1.0\"\nedition = \"2021\"\n[dependencies]\n{deps}")).unwrap();
    if let Some(l) = lock {
        std::fs::write(d.join("Cargo.lock"), l).unwrap();
    }
    for (p, s) in files {
        std::fs::write(d.join("src").join(p), s).unwrap();
    }
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
    cmd.env_remove("CANDOR_DEPS").env_remove("CANDOR_CONFIG").env_remove("CANDOR_POLICY").env_remove("CANDOR_R1624_OFF");
    if off {
        cmd.env("CANDOR_R1624_OFF", "1");
    }
    let out = cmd.output().expect("run candor-scan");
    let code = out.status.code().unwrap_or(-1);
    // SPEC ⟨0.42⟩ §3.3 (e): a POLICY run under a set result-moving switch REFUSES (exit 2) after writing its report, so the
    // switched arm of a gate is the one place exit 2 is the answer. A scan with no policy asks nothing and is not refused.
    let refused_by_switch = off && policy.is_some() && code == 2;
    assert!(code == 0 || code == 1 || refused_by_switch, "scan failed ({code}): {}", String::from_utf8_lossy(&out.stderr));
    (code, serde_json::from_slice(&out.stdout).expect("report json"))
}

fn inferred(v: &serde_json::Value, f: &str) -> Vec<String> {
    let row = v["functions"].as_array().unwrap().iter().find(|r| r["fn"].as_str() == Some(f));
    let mut out: Vec<String> = row
        .and_then(|r| r["inferred"].as_array())
        .map(|a| a.iter().filter_map(|e| e.as_str().map(str::to_string)).collect())
        .unwrap_or_default();
    out.sort();
    out
}

const MAIN: &str = "\
mod fa;\nmod fb;\nmod ext;\n\
pub mod a {\n\
    pub struct Loud;\n\
    impl Loud { pub fn run(&self) { std::fs::write(std::env::temp_dir().join(\"r1624-a\"), b\"x\").unwrap(); } }\n\
    pub struct H { pub inner: Loud }\n\
    impl H {\n\
        pub fn a_self_zq(&self) { self.inner.run() }\n\
        pub fn a_bind_zq(&self) { let p = &self.inner; p.run() }\n\
    }\n\
}\n\
pub mod b {\n\
    pub struct Quiet;\n\
    impl Quiet { pub fn run(&self) {} }\n\
    pub struct H { pub inner: Quiet }\n\
    impl H {\n\
        pub fn b_self_zq(&self) { self.inner.run() }\n\
        pub fn b_bind_zq(&self) { let p = &self.inner; p.run() }\n\
    }\n\
}\n\
pub mod c {\n\
    pub struct S { pub w: std::fs::File }\n\
    impl S { pub fn c_self_zq(&self) { let _ = self.w.sync_all(); } }\n\
}\n\
pub mod d {\n\
    pub struct Nop;\n\
    impl Nop { pub fn sync_all(&self) -> Result<(), ()> { Ok(()) } }\n\
    pub struct S { pub w: Nop }\n\
    impl S { pub fn d_self_zq(&self) { let _ = self.w.sync_all(); } }\n\
}\n\
pub mod e {\n\
    pub struct Calm;\n\
    impl Calm { pub fn go(&self) {} }\n\
    pub struct K { pub k: Calm }\n\
    impl K { pub fn e_self_zq(&self) { self.k.go() } }\n\
}\n\
pub mod f {\n\
    pub struct Noisy;\n\
    impl Noisy { pub fn go(&self) { let _ = std::env::var(\"R1624_F\"); } }\n\
    pub struct K { pub k: Noisy }\n\
    impl K { pub fn f_self_zq(&self) { self.k.go() } }\n\
}\n\
pub fn resid_param_zq(h: &a::H) { h.inner.run() }\n\
fn main() {}\n";

const FA: &str = "pub struct Disk;\n\
impl Disk { pub fn put(&self) { std::fs::write(std::env::temp_dir().join(\"r1624-fa\"), b\"x\").unwrap(); } }\n\
pub struct G { pub g: Disk }\n\
impl G { pub fn fa_self_zq(&self) { self.g.put() } }\n";

const FB: &str = "pub struct Mem;\n\
impl Mem { pub fn put(&self) {} }\n\
pub struct G { pub g: Mem }\n\
impl G { pub fn fb_self_zq(&self) { self.g.put() } }\n";

// The ELEMENT (`field_elem`) and DISPATCH (`trait_fields`) halves, effectful twin first. Executed: `g_loop`, `g_idx` and
// `t1_self` write a temp file; `h_*` and `t2_self` do nothing. `w_flush_zq` (a wrapper accessor over an `Option` twin field,
// snap's `write::FrameEncoder::flush` shape) writes one too, and was silent on 544ed9f. On 544ed9f all three were silent (`t1_self` dispatched on
// `t2::Sk::w`, its sibling's trait).
const EXT: &str = "\
pub mod g {\n\
    pub struct Loud2;\n\
    impl Loud2 { pub fn run(&self) { std::fs::write(std::env::temp_dir().join(\"r1624-g\"), b\"x\").unwrap(); } }\n\
    pub struct V { pub xs: Vec<Loud2> }\n\
    impl V { pub fn g_loop_zq(&self) { for x in &self.xs { x.run() } } pub fn g_idx_zq(&self) { self.xs[0].run() } }\n\
}\n\
pub mod h {\n\
    pub struct Quiet2;\n\
    impl Quiet2 { pub fn run(&self) {} }\n\
    pub struct V { pub xs: Vec<Quiet2> }\n\
    impl V { pub fn h_loop_zq(&self) { for x in &self.xs { x.run() } } pub fn h_idx_zq(&self) { self.xs[0].run() } }\n\
}\n\
pub mod t1 {\n\
    pub trait Wr { fn w(&self); }\n\
    pub struct L;\n\
    impl Wr for L { fn w(&self) { std::fs::write(std::env::temp_dir().join(\"r1624-t\"), b\"x\").unwrap(); } }\n\
    pub struct Tt { pub h: Box<dyn Wr> }\n\
    impl Tt { pub fn t1_self_zq(&self) { self.h.w() } }\n\
}\n\
pub mod t2 {\n\
    pub trait Sk { fn w(&self); }\n\
    pub struct P;\n\
    impl Sk for P { fn w(&self) {} }\n\
    pub struct Tt { pub h: Box<dyn Sk> }\n\
    impl Tt { pub fn t2_self_zq(&self) { self.h.w() } }\n\
}\n\
pub mod w {\n\
    pub struct Inner;\n\
    impl Inner { pub fn put(&mut self) { std::fs::write(std::env::temp_dir().join(\"r1624-w\"), b\"x\").unwrap(); } }\n\
    pub struct Fe { pub inner: Option<Inner> }\n\
    impl Fe { pub fn w_flush_zq(&mut self) { self.inner.as_mut().unwrap().put() } }\n\
}\n\
pub mod r {\n\
    pub struct Calm3;\n\
    impl Calm3 { pub fn put(&mut self) {} }\n\
    pub struct Fe { pub inner: Calm3 }\n\
    impl Fe { pub fn r_flush_zq(&mut self) { self.inner.put() } }\n\
}\n\
pub mod q1 {\n\
    pub trait De { fn d(&self); }\n\
    pub struct Ck<X> { pub delegate: X }\n\
    impl<X: De> Ck<X> { pub fn q1_go_zq(&self) { self.delegate.d() } }\n\
}\n\
pub mod q2 {\n\
    pub trait Se { fn d(&self); }\n\
    pub struct Ck<T> { pub delegate: T }\n\
    impl<T: Se> Ck<T> { pub fn q2_go_zq(&self) { self.delegate.d() } }\n\
}\n";

// One directory per test: the tests of this file run in parallel in one process and share the pid.
fn local(tag: &str) -> PathBuf {
    fixture(tag, "", None, &[("main.rs", MAIN), ("fa.rs", FA), ("fb.rs", FB), ("ext.rs", EXT)])
}

#[test]
fn r1624_each_twin_reads_its_own_field_type() {
    let d = local("rows");
    let (_, v) = scan(&d, None, false);
    // Executed effects, each on the twin that runs them.
    for (f, want) in [
        ("a::H::a_self_zq", vec!["Env", "Fs"]),
        ("a::H::a_bind_zq", vec!["Env", "Fs"]),
        ("fa::G::fa_self_zq", vec!["Env", "Fs"]),
        ("c::S::c_self_zq", vec!["Fs"]),
        ("f::K::f_self_zq", vec!["Env"]),
        ("resid_param_zq", vec!["Env", "Fs"]),
        ("ext::g::V::g_loop_zq", vec!["Env", "Fs"]),
        ("ext::g::V::g_idx_zq", vec!["Env", "Fs"]),
        ("ext::t1::Tt::t1_self_zq", vec!["Env", "Fs"]),
        ("ext::w::Fe::w_flush_zq", vec!["Env", "Fs"]),
    ] {
        assert_eq!(inferred(&v, f), want, "{f}");
    }
    // The pure twins (executed: no effect) carry none of their sibling's.
    for f in [
        "b::H::b_self_zq", "b::H::b_bind_zq", "d::S::d_self_zq", "e::K::e_self_zq", "fb::G::fb_self_zq",
        "ext::h::V::h_loop_zq", "ext::h::V::h_idx_zq", "ext::t2::Tt::t2_self_zq", "ext::r::Fe::r_flush_zq",
    ] {
        assert!(inferred(&v, f).is_empty(), "{f} charged {:?}", inferred(&v, f));
    }
    let _ = std::fs::remove_dir_all(&d);
}

#[test]
fn r1624_gate_flips_on_the_executed_write_and_the_kill_switch_restores_the_silence() {
    let d = local("gate");
    // SOUNDNESS R1634 — the kill switch can no longer be measured by GATING under it: a gate under a set result-moving switch is
    // unanswerable (exit 2, SPEC ⟨0.42⟩ §3.3 (e)). So each arm is measured where the attribution belongs, on the REPORT route
    // (`--json`, no policy: disclosure, nothing refuses): the gate flips exactly when the unit's row gains/loses the effect the
    // rule denies, and the policy run under the switch is asserted to REFUSE, naming it.
    for f in ["a::H::a_self_zq", "a::H::a_bind_zq", "c::S::c_self_zq", "ext::g::V::g_loop_zq", "ext::g::V::g_idx_zq", "ext::t1::Tt::t1_self_zq", "ext::w::Fe::w_flush_zq"] {
        let policy = format!("deny Fs {f}\n");
        let (on, _) = scan(&d, Some(&policy), false);
        assert_eq!(on, 1, "{f}: deny Fs with the fix");
        let (_, rep_on) = scan(&d, None, false);
        let (_, rep_off) = scan(&d, None, true);
        assert!(inferred(&rep_on, f).contains(&"Fs".to_string()), "{f}: the fix charges Fs {:?}", inferred(&rep_on, f));
        assert!(!inferred(&rep_off, f).contains(&"Fs".to_string()), "{f}: CANDOR_R1624_OFF restores the silence {:?}", inferred(&rep_off, f));
        assert_eq!(rep_off["switches"], serde_json::json!(["CANDOR_R1624_OFF"]), "{f}: the switched report names the switch");
        assert!(rep_on.get("switches").is_none(), "{f}: the unswitched report names none");
        let (refused, _) = scan(&d, Some(&policy), true);
        assert_eq!(refused, 2, "{f}: a gate under CANDOR_R1624_OFF is unanswerable, not green");
    }
    // …and the fabrication: the pure twin was charged its sibling's Env.
    let (on, _) = scan(&d, Some("deny Env e::K::e_self_zq\n"), false);
    assert_eq!(on, 0, "e_self: deny Env with the fix");
    let (_, rep_off) = scan(&d, None, true);
    assert!(inferred(&rep_off, "e::K::e_self_zq").contains(&"Env".to_string()), "e_self: CANDOR_R1624_OFF restores the sibling's Env");
    let (refused, _) = scan(&d, Some("deny Env e::K::e_self_zq\n"), true);
    assert_eq!(refused, 2, "e_self: the switched gate refuses");
    let _ = std::fs::remove_dir_all(&d);
}

const SQLX_LOCK: &str = "version = 3\n\n\
[[package]]\nname = \"sqlx\"\nversion = \"0.9.0\"\nsource = \"registry+https://github.com/rust-lang/crates.io-index\"\n\n\
[[package]]\nname = \"fx\"\nversion = \"0.1.0\"\ndependencies = [\"sqlx\"]\n";

const SQLX_SRC: &str = "\
pub mod pg { use sqlx::PgPool; pub struct Executor { pool: PgPool }\n\
  impl Executor { pub async fn f_pg_acq_zq(&self) -> bool { self.pool.acquire().await.is_ok() } } }\n\
pub mod my { use sqlx::MySqlPool; pub struct Executor { pool: MySqlPool }\n\
  impl Executor { pub async fn f_my_acq_zq(&self) -> bool { self.pool.acquire().await.is_ok() } } }\n\
pub mod sq { use sqlx::SqlitePool; pub struct Executor { pool: SqlitePool }\n\
  impl Executor { pub async fn f_sq_acq_zq(&self) -> bool { self.pool.acquire().await.is_ok() } } }\n";

#[test]
fn r1624_three_driver_executors_each_read_their_own_pool() {
    for lock in [Some(SQLX_LOCK), None] {
        let d = fixture(
            if lock.is_some() { "sqlx-lock" } else { "sqlx-nolock" },
            "sqlx = { version = \"=0.9.0\", default-features = false, features = [\"postgres\", \"mysql\", \"sqlite\"] }\n",
            lock,
            &[("lib.rs", SQLX_SRC)],
        );
        let (_, v) = scan(&d, None, false);
        let sq = inferred(&v, "sq::Executor::f_sq_acq_zq");
        assert!(sq.contains(&"Fs".to_string()) && sq.contains(&"Db".to_string()), "sqlite acquire {sq:?}");
        assert!(!sq.contains(&"Net".to_string()) && !sq.contains(&"Ipc".to_string()), "sqlite acquire fabricated {sq:?}");
        for f in ["pg::Executor::f_pg_acq_zq", "my::Executor::f_my_acq_zq"] {
            let e = inferred(&v, f);
            assert!(e.contains(&"Net".to_string()) && e.contains(&"Ipc".to_string()) && e.contains(&"Db".to_string()), "{f} {e:?}");
        }
        // Under the kill switch the three share the last-merged pool (the pre-fix reading): at least one is wrong.
        let (_, off) = scan(&d, None, true);
        let rows: Vec<Vec<String>> = ["pg::Executor::f_pg_acq_zq", "my::Executor::f_my_acq_zq", "sq::Executor::f_sq_acq_zq"]
            .iter()
            .map(|f| inferred(&off, f))
            .collect();
        assert!(rows[0] == rows[1] && rows[1] == rows[2], "kill switch no longer reproduces the shared entry: {rows:?}");
        let _ = std::fs::remove_dir_all(&d);
    }
}

/// A twin field typed by the owner's own TYPE PARAMETER (`delegate: X`, serde_path_to_error's two `CaptureKey`s) is not a
/// concrete field: its dispatch stays the bound route's hedge. A first cut read the parameter name as a concrete type, cleared
/// the dispatch and left both rows ABSENT (measured on the corpus by the dropped-edge detector).
#[test]
fn r1624_a_generic_twin_field_keeps_its_bound_dispatch() {
    let d = local("generic");
    let (_, v) = scan(&d, None, false);
    for (f, key) in [("ext::q1::Ck::q1_go_zq", "fx#ext::q1::De::d"), ("ext::q2::Ck::q2_go_zq", "fx#ext::q2::Se::d")] {
        let row = v["functions"].as_array().unwrap().iter().find(|r| r["fn"].as_str() == Some(f));
        let row = row.unwrap_or_else(|| panic!("{f} absent: its bound dispatch was dropped"));
        assert_eq!(inferred(&v, f), vec!["Unknown"], "{f}");
        let keys: Vec<&str> = row["dispatchesOn"].as_array().map(|a| a.iter().filter_map(|k| k.as_str()).collect()).unwrap_or_default();
        assert!(keys.contains(&key), "{f} dispatchesOn {keys:?}");
    }
    let _ = std::fs::remove_dir_all(&d);
}
