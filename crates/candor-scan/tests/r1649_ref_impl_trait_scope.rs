//! SOUNDNESS R1649 — a typed method call on `T` read the impl that claimed the tail `T::m`, not the one rustc runs.
//!
//! sqlx-postgres `impl Executor for &mut PgListener { fn prepare_with(self, ..) { self.connection().await?.prepare_with(..) } }`
//! imports only `Executor`, so rustc's MIR calls `<&mut PgConnection as Executor>::prepare_with` (module-keyed, R1034). The
//! engine resolved the tail `PgConnection::prepare_with` to `impl AnyConnectionBackend for PgConnection`'s method, because the
//! `&mut T` impl's fallback is not filed where another impl already claims the tail. Executed fixture rustagent-r474/fx/r1649:
//! `listener::prep` writes and read ABSENT; the pure twin `listener2::prep2` read the other impl's `Fs`.
//!
//! The second half is the silence the resolution exposed: a stream macro's body (`try_stream! { .. }`) is a block of
//! statements, and the expression-list walk read none of it (sqlx-postgres `<&mut PgConnection as Executor>::fetch_many`
//! read `[]` on every arm). Executed fixture rustagent-r474/fx/r1649s (async-stream): both writing streams write, the twin
//! does not. Fixture fns carry a unique `_zq` terminator because `deny <E> <fn>` matches by PREFIX.

use std::path::{Path, PathBuf};
use std::process::Command;

fn bin() -> &'static str {
    env!("CARGO_BIN_EXE_candor-scan")
}

fn fixture(tag: &str, deps: &str, files: &[(&str, &str)]) -> PathBuf {
    let d = std::env::temp_dir().join(format!("candor-r1649-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(d.join("src")).unwrap();
    std::fs::write(d.join("Cargo.toml"), format!("[package]\nname = \"{tag}\"\nversion = \"0.1.0\"\nedition = \"2021\"\n{deps}")).unwrap();
    for (n, s) in files {
        std::fs::write(d.join("src").join(n), s).unwrap();
    }
    d
}

fn scan(dir: &Path, off: bool) -> serde_json::Value {
    let mut cmd = Command::new(bin());
    cmd.arg(dir.to_string_lossy().as_ref()).arg("--json");
    cmd.env_remove("CANDOR_DEPS").env_remove("CANDOR_CONFIG").env_remove("CANDOR_POLICY").env_remove("CANDOR_R1649_OFF");
    if off {
        cmd.env("CANDOR_R1649_OFF", "1");
    }
    let o = cmd.output().expect("run candor-scan");
    let code = o.status.code().unwrap_or(-1);
    assert!(code == 0 || code == 1, "scan failed ({code}): {}", String::from_utf8_lossy(&o.stderr));
    serde_json::from_slice(&o.stdout).expect("report json")
}

fn inferred(v: &serde_json::Value, f: &str) -> Vec<String> {
    let mut out: Vec<String> = v["functions"]
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r["fn"].as_str() == Some(f))
        .and_then(|r| r["inferred"].as_array())
        .map(|a| a.iter().filter_map(|e| e.as_str().map(str::to_string)).collect())
        .unwrap_or_default();
    out.sort();
    out
}

const LIB: &str = "pub mod any;\npub mod any2;\npub mod executor;\npub mod executor2;\npub mod listener;\npub mod listener2;\n\
pub trait Exec { fn prep(self, q: &str) -> usize; }\n\
pub trait AnyBackend { fn prep(&mut self, q: &str) -> usize; }\n\
pub struct Conn { pub path: String }\n\
pub struct Listener { pub conn: Conn }\n\
impl Listener { pub fn connection(&mut self) -> &mut Conn { &mut self.conn } }\n\
pub trait Exec2 { fn prep2(self, q: &str) -> usize; }\n\
pub trait AnyBackend2 { fn prep2(&mut self, q: &str) -> usize; }\n\
pub struct Conn2 { pub path: String }\n\
pub struct Listener2 { pub conn: Conn2 }\n\
impl Listener2 { pub fn connection(&mut self) -> &mut Conn2 { &mut self.conn } }\n";

#[test]
fn r1649_ref_impl_whose_trait_is_in_scope_wins_the_tail() {
    let d = fixture(
        "lsn",
        "",
        &[
            ("lib.rs", LIB),
            ("executor.rs", "use crate::{Conn, Exec};\nimpl<'c> Exec for &'c mut Conn {\n    fn prep(self, q: &str) -> usize { std::fs::write(&self.path, q).unwrap(); 1 }\n}\n"),
            ("any.rs", "use crate::{AnyBackend, Conn};\nimpl AnyBackend for Conn {\n    fn prep(&mut self, _q: &str) -> usize { 0 }\n}\n"),
            ("listener.rs", "use crate::{Exec, Listener};\nimpl<'c> Exec for &'c mut Listener {\n    fn prep(self, q: &str) -> usize { self.connection().prep(q) }\n}\n"),
            ("executor2.rs", "use crate::{Conn2, Exec2};\nimpl<'c> Exec2 for &'c mut Conn2 { fn prep2(self, _q: &str) -> usize { 0 } }\n"),
            ("any2.rs", "use crate::{AnyBackend2, Conn2};\nimpl AnyBackend2 for Conn2 { fn prep2(&mut self, q: &str) -> usize { std::fs::write(&self.path, q).unwrap(); 1 } }\n"),
            ("listener2.rs", "use crate::{Exec2, Listener2};\nimpl<'c> Exec2 for &'c mut Listener2 {\n    fn prep2(self, q: &str) -> usize { self.connection().prep2(q) }\n}\n"),
        ],
    );
    let v = scan(&d, false);
    assert_eq!(inferred(&v, "listener::prep"), vec!["Fs"], "the `&mut Conn` impl runs (executed):\n{v:#}");
    assert!(inferred(&v, "listener2::prep2").is_empty(), "pure twin: `AnyBackend2` is not in scope there:\n{v:#}");
    let off = scan(&d, true);
    assert!(inferred(&off, "listener::prep").is_empty(), "CANDOR_R1649_OFF restores the old reading:\n{off:#}");
    assert_eq!(inferred(&off, "listener2::prep2"), vec!["Fs"]);
}

#[test]
fn r1649_a_glob_leaves_the_old_reading() {
    // A glob may bring the claimant's trait into scope, so rustc's choice is not determined here and nothing moves: the
    // row reads exactly what it reads under CANDOR_R1649_OFF. (A first cut UNIONED the `&T` impl in on "may be in scope"
    // and added 400 edges rustc's MIR does not make; see rustagent-r474/ab1649/mc.tsv.)
    let d = fixture(
        "lsnglob",
        "",
        &[
            ("lib.rs", LIB),
            ("executor.rs", "use crate::{Conn, Exec};\nimpl<'c> Exec for &'c mut Conn {\n    fn prep(self, q: &str) -> usize { std::fs::write(&self.path, q).unwrap(); 1 }\n}\n"),
            ("any.rs", "use crate::{AnyBackend, Conn};\nimpl AnyBackend for Conn {\n    fn prep(&mut self, _q: &str) -> usize { std::env::var(\"X\").map(|_| 0).unwrap_or(0) }\n}\n"),
            ("listener.rs", "use crate::executor::*;\nuse crate::{Exec, Listener};\nimpl<'c> Exec for &'c mut Listener {\n    fn prep(self, q: &str) -> usize { self.connection().prep(q) }\n}\n"),
            ("executor2.rs", ""),
            ("any2.rs", ""),
            ("listener2.rs", ""),
        ],
    );
    let v = scan(&d, false);
    let off = scan(&d, true);
    assert_eq!(inferred(&v, "listener::prep"), inferred(&off, "listener::prep"), "{v:#}");
}

#[test]
fn r1649_stream_macro_body_is_walked() {
    let d = fixture(
        "strm",
        "[dependencies]\nasync-stream = \"0.3\"\nfutures-core = \"0.3\"\n",
        &[(
            "lib.rs",
            "use async_stream::{stream, try_stream};\nuse futures_core::stream::Stream;\n\
             pub fn s_try_write_zq(p: String) -> impl Stream<Item = std::io::Result<u8>> {\n    try_stream! {\n        std::fs::write(&p, b\"x\")?;\n        yield 1u8;\n    }\n}\n\
             pub fn s_plain_write_zq(p: String) -> impl Stream<Item = u8> {\n    stream! {\n        std::fs::write(&p, b\"y\").unwrap();\n        yield 2u8;\n    }\n}\n\
             pub fn s_quiet_zq(n: u8) -> impl Stream<Item = u8> {\n    stream! {\n        let m = n + 1;\n        yield m;\n    }\n}\n",
        )],
    );
    let v = scan(&d, false);
    assert_eq!(inferred(&v, "s_try_write_zq"), vec!["Fs"], "{v:#}");
    assert_eq!(inferred(&v, "s_plain_write_zq"), vec!["Fs"], "{v:#}");
    assert!(inferred(&v, "s_quiet_zq").is_empty(), "pure twin:\n{v:#}");
    let off = scan(&d, true);
    assert!(inferred(&off, "s_try_write_zq").is_empty(), "{off:#}");
}
