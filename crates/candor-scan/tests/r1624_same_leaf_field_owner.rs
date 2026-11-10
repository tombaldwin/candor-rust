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
    assert!(code == 0 || code == 1, "scan failed ({code}): {}", String::from_utf8_lossy(&out.stderr));
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
// `t1_self` write a temp file; `h_*` and `t2_self` do nothing. On 544ed9f all three were silent (`t1_self` dispatched on
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
    ] {
        assert_eq!(inferred(&v, f), want, "{f}");
    }
    // The pure twins (executed: no effect) carry none of their sibling's.
    for f in [
        "b::H::b_self_zq", "b::H::b_bind_zq", "d::S::d_self_zq", "e::K::e_self_zq", "fb::G::fb_self_zq",
        "ext::h::V::h_loop_zq", "ext::h::V::h_idx_zq", "ext::t2::Tt::t2_self_zq",
    ] {
        assert!(inferred(&v, f).is_empty(), "{f} charged {:?}", inferred(&v, f));
    }
    let _ = std::fs::remove_dir_all(&d);
}

#[test]
fn r1624_gate_flips_on_the_executed_write_and_the_kill_switch_restores_the_silence() {
    let d = local("gate");
    for f in ["a::H::a_self_zq", "a::H::a_bind_zq", "c::S::c_self_zq", "ext::g::V::g_loop_zq", "ext::g::V::g_idx_zq", "ext::t1::Tt::t1_self_zq"] {
        let (on, _) = scan(&d, Some(&format!("deny Fs {f}\n")), false);
        let (off, _) = scan(&d, Some(&format!("deny Fs {f}\n")), true);
        assert_eq!((on, off), (1, 0), "{f}: deny Fs with the fix / under CANDOR_R1624_OFF");
    }
    // …and the fabrication: the pure twin was charged its sibling's Env.
    let (on, _) = scan(&d, Some("deny Env e::K::e_self_zq\n"), false);
    let (off, _) = scan(&d, Some("deny Env e::K::e_self_zq\n"), true);
    assert_eq!((on, off), (0, 1), "e_self: deny Env with the fix / under CANDOR_R1624_OFF");
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
