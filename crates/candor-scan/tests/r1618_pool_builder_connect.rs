//! SOUNDNESS R1618 — `deadpool_postgres::Pool::{get, timeout_get}`, `build_transaction().start()` and the sqlx MySQL connect
//! spellings, charged at the scan's main path, with `deny <E>` going 0 -> 1 on code that really does it and a pure neighbour
//! staying at 0, in BOTH the lock arm (a `Cargo.lock` pins the calibrated versions) and the lockless arm.
//!
//! EXECUTED (macOS, DYLD interposer, postgres:16 / mysql:8): a cold deadpool `get()` = connect + 1 getentropy, warm = 0; `start()` =
//! BEGIN + 1 `log` record; every sqlx MySQL connect spelling = connect + 5 getentropy (default TLS arm). The `connect_with`
//! spellings were ABSENT from `functions[]` before this row. Fixture fns carry unique terminators because `deny <E> <fn>` matches
//! by PREFIX.

use std::path::{Path, PathBuf};
use std::process::Command;

fn bin() -> &'static str {
    env!("CARGO_BIN_EXE_candor-scan")
}

const LOCK: &str = "version = 3\n\n\
[[package]]\nname = \"deadpool-postgres\"\nversion = \"0.14.2\"\nsource = \"registry+https://github.com/rust-lang/crates.io-index\"\n\n\
[[package]]\nname = \"tokio-postgres\"\nversion = \"0.7.18\"\nsource = \"registry+https://github.com/rust-lang/crates.io-index\"\n\n\
[[package]]\nname = \"postgres\"\nversion = \"0.19.14\"\nsource = \"registry+https://github.com/rust-lang/crates.io-index\"\n\n\
[[package]]\nname = \"sqlx\"\nversion = \"0.9.0\"\nsource = \"registry+https://github.com/rust-lang/crates.io-index\"\n\n\
[[package]]\nname = \"fx\"\nversion = \"0.1.0\"\ndependencies = [\"deadpool-postgres\", \"postgres\", \"sqlx\", \"tokio-postgres\"]\n";

fn fixture(tag: &str, lock: bool, src: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("candor-r1618-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(d.join("src")).unwrap();
    std::fs::write(
        d.join("Cargo.toml"),
        "[package]\nname = \"fx\"\nversion = \"0.1.0\"\nedition = \"2021\"\n[dependencies]\ndeadpool-postgres = \"=0.14.2\"\ntokio-postgres = \"=0.7.18\"\npostgres = \"=0.19.14\"\nsqlx = { version = \"=0.9.0\", default-features = false, features = [\"mysql\"] }\n",
    )
    .unwrap();
    if lock {
        std::fs::write(d.join("Cargo.lock"), LOCK).unwrap();
    }
    std::fs::write(d.join("src/lib.rs"), src).unwrap();
    d
}

fn scan(dir: &Path, policy: Option<&str>) -> (i32, serde_json::Value) {
    let mut cmd = Command::new(bin());
    cmd.arg(dir.to_string_lossy().as_ref()).arg("--json");
    let pf = dir.join("policy.txt");
    if let Some(p) = policy {
        std::fs::write(&pf, p).unwrap();
        cmd.arg("--policy").arg(&pf);
    }
    let out = cmd.env_remove("CANDOR_DEPS").env_remove("CANDOR_CONFIG").env_remove("CANDOR_POLICY").output().expect("run candor-scan");
    let code = out.status.code().unwrap_or(-1);
    assert!(code == 0 || code == 1, "scan failed ({code}): {}", String::from_utf8_lossy(&out.stderr));
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

const SRC: &str = "\
pub async fn f_pool_get_z(p: &deadpool_postgres::Pool) { let _ = p.get().await; }\n\
pub async fn f_pool_tget_z(p: &deadpool_postgres::Pool) { let _ = p.timeout_get(&deadpool_postgres::Timeouts::new()).await; }\n\
pub fn c_pool_status_z(p: &deadpool_postgres::Pool) { let _ = p.status(); }\n\
pub fn c_pool_resize_z(p: &deadpool_postgres::Pool) { p.resize(2); }\n\
pub async fn f_tx_start_z(c: &mut tokio_postgres::Client) { let _ = c.build_transaction().start().await; }\n\
pub async fn f_tx_iso_z(c: &mut tokio_postgres::Client) { let _ = c.build_transaction().isolation_level(tokio_postgres::IsolationLevel::Serializable).start().await; }\n\
pub fn f_tx_sync_z(c: &mut postgres::Client) { let _ = c.build_transaction().start(); }\n\
pub async fn f_tx_dp_z(c: &mut deadpool_postgres::Object) { let _ = c.build_transaction().start().await; }\n\
pub fn c_tx_build_z(c: &mut tokio_postgres::Client) { let _ = c.build_transaction().read_only(true); }\n\
pub async fn f_sx_conn_z(u: &str) { use sqlx::Connection; let _ = sqlx::MySqlConnection::connect(u).await; }\n\
pub async fn f_sx_cwith_z(o: &sqlx::mysql::MySqlConnectOptions) { use sqlx::Connection; let _ = sqlx::MySqlConnection::connect_with(o).await; }\n\
pub async fn f_sx_pool_z(u: &str) { let _ = sqlx::MySqlPool::connect(u).await; }\n\
pub async fn f_sx_pwith_z(o: sqlx::mysql::MySqlConnectOptions) { let _ = sqlx::MySqlPool::connect_with(o).await; }\n\
pub async fn f_sx_popts_z(u: &str) { let _ = sqlx::mysql::MySqlPoolOptions::new().connect(u).await; }\n\
pub async fn f_sx_popw_z(o: sqlx::mysql::MySqlConnectOptions) { let _ = sqlx::mysql::MySqlPoolOptions::new().connect_with(o).await; }\n\
pub fn c_sx_lazy_z(u: &str) { let _ = sqlx::MySqlPool::connect_lazy(u); }\n\
pub fn c_sx_opts_z() -> sqlx::mysql::MySqlConnectOptions { sqlx::mysql::MySqlConnectOptions::new().host(\"h\") }\n";

fn check(lock: bool) {
    let d = fixture(if lock { "lock" } else { "nolock" }, lock, SRC);
    let (_, v) = scan(&d, None);
    let s = |x: &[&str]| x.iter().map(|e| e.to_string()).collect::<Vec<_>>();
    // (1) the pool
    for f in ["f_pool_get_z", "f_pool_tget_z"] {
        assert_eq!(field(&v, f, "inferred"), s(&["Db", "Ipc", "Net", "Rand"]), "{f} lock={lock}:\n{v:#}");
        // a Net second effect has a destination the call's literal does not name: marked, never certified
        assert_eq!(field(&v, f, "incomplete"), s(&["Net"]), "{f}");
    }
    // (2) the transaction builder
    for f in ["f_tx_start_z", "f_tx_iso_z", "f_tx_sync_z", "f_tx_dp_z"] {
        assert_eq!(field(&v, f, "inferred"), s(&["Db", "Log"]), "{f} lock={lock}:\n{v:#}");
    }
    // (3) sqlx MySQL: connect_with was ABSENT
    for f in ["f_sx_conn_z", "f_sx_cwith_z", "f_sx_pool_z", "f_sx_pwith_z", "f_sx_popts_z", "f_sx_popw_z"] {
        assert_eq!(field(&v, f, "inferred"), s(&["Db", "Ipc", "Net", "Rand"]), "{f} lock={lock}:\n{v:#}");
        assert!(!field(&v, f, "inferred").contains(&"Fs".to_string()), "no blanket Fs for ssl-ca");
    }
    for f in ["c_pool_status_z", "c_pool_resize_z", "c_tx_build_z", "c_sx_lazy_z", "c_sx_opts_z"] {
        assert!(field(&v, f, "inferred").is_empty(), "{f} is a pure neighbour (lock={lock}):\n{v:#}");
    }
    // the gate flips: `deny <E>` exits 1 over the code that really does it, 0 over its neighbour
    for (e, f) in [
        ("Db", "f_pool_get_z"), ("Net", "f_pool_get_z"), ("Rand", "f_pool_get_z"), ("Ipc", "f_pool_get_z"),
        ("Db", "f_pool_tget_z"), ("Net", "f_pool_tget_z"),
        ("Db", "f_tx_start_z"), ("Log", "f_tx_start_z"), ("Db", "f_tx_iso_z"), ("Db", "f_tx_sync_z"), ("Log", "f_tx_sync_z"),
        ("Db", "f_tx_dp_z"), ("Log", "f_tx_dp_z"),
        ("Db", "f_sx_cwith_z"), ("Net", "f_sx_cwith_z"), ("Rand", "f_sx_cwith_z"), ("Net", "f_sx_conn_z"), ("Rand", "f_sx_conn_z"),
        ("Db", "f_sx_pwith_z"), ("Net", "f_sx_pool_z"), ("Db", "f_sx_popw_z"), ("Net", "f_sx_popts_z"),
    ] {
        let (code, _) = scan(&d, Some(&format!("deny {e} {f}\n")));
        assert_eq!(code, 1, "deny {e} {f} (lock={lock})");
    }
    for (e, f) in [
        ("Db", "c_pool_status_z"), ("Net", "c_pool_resize_z"), ("Db", "c_tx_build_z"), ("Log", "c_tx_build_z"),
        ("Db", "c_sx_lazy_z"), ("Net", "c_sx_lazy_z"), ("Db", "c_sx_opts_z"),
        ("Net", "f_tx_start_z"), ("Fs", "f_sx_conn_z"),
    ] {
        let (code, _) = scan(&d, Some(&format!("deny {e} {f}\n")));
        assert_eq!(code, 0, "deny {e} {f} must stay clean (lock={lock})");
    }
    let _ = std::fs::remove_dir_all(&d);
}

#[test]
fn pool_get_builder_start_and_sqlx_connect_flip_their_gates_with_a_lock() {
    check(true);
}

#[test]
fn pool_get_builder_start_and_sqlx_connect_flip_their_gates_without_a_lock() {
    check(false);
}
