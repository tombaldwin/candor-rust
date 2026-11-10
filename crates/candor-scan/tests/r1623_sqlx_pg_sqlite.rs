//! SOUNDNESS R1623 — sqlx 0.9.0 POSTGRES and SQLITE connect/acquire/options spellings (R1618's residual (c)), with the effect
//! EXECUTED (macOS DYLD interposer, postgres:16 with TLS on and SCRAM, mysql:8, real sqlite files; handoff `rust-r1623/`):
//!   * Postgres connect = 1 connect (TCP or AF_UNIX) + 1-6 getentropy; a URL spelling also reads 12 `PG*` variables and stats the
//!     unix-socket directories (and opens `~/.pgpass` with no password); `connect_lazy(url)` does ONLY the parse, 0 connections.
//!   * An acquire/begin on a COLD pool is the same connect; on a warm pool it does none of it (the stated over-charge).
//!   * SQLite connect opens the db file read-write, opens /dev/urandom and reads TMPDIR: NO Net, NO Ipc (copying the MySQL shape would
//!     fabricate). `connect_lazy` and `SqliteConnectOptions::{new, parse}` do nothing (0 events).
//!   * SILENT before this row: `PgPool::connect_lazy(u)` then `.acquire()` (receiver untyped: `connect_lazy` / `connect_with` were not
//!     constructor names), `PgListener::connect_with(&pool)`, `Pool::try_begin` (a warm pool sent BEGIN), and the option constructors
//!     / `parse::<PgConnectOptions>()` (Env + Fs).
//!
//! Fixture fns carry a unique `_zq` terminator because `deny <E> <fn>` matches by PREFIX. Both the lock arm (a `Cargo.lock` pins the
//! calibrated sqlx) and the lockless arm are asserted.

use std::path::{Path, PathBuf};
use std::process::Command;

fn bin() -> &'static str {
    env!("CARGO_BIN_EXE_candor-scan")
}

const LOCK: &str = "version = 3\n\n\
[[package]]\nname = \"sqlx\"\nversion = \"0.9.0\"\nsource = \"registry+https://github.com/rust-lang/crates.io-index\"\n\n\
[[package]]\nname = \"fx\"\nversion = \"0.1.0\"\ndependencies = [\"sqlx\"]\n";

fn fixture(tag: &str, lock: bool, src: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("candor-r1623-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(d.join("src")).unwrap();
    std::fs::write(
        d.join("Cargo.toml"),
        "[package]\nname = \"fx\"\nversion = \"0.1.0\"\nedition = \"2021\"\n[dependencies]\nsqlx = { version = \"=0.9.0\", default-features = false, features = [\"postgres\", \"mysql\", \"sqlite\"] }\n",
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
use sqlx::Connection;\n\
use sqlx::ConnectOptions;\n\
use std::str::FromStr;\n\
pub async fn f_pg_conn_zq(u: &str) { let _ = sqlx::PgConnection::connect(u).await; }\n\
pub async fn f_pg_cwith_zq(o: &sqlx::postgres::PgConnectOptions) { let _ = sqlx::PgConnection::connect_with(o).await; }\n\
pub async fn f_pg_pool_zq(u: &str) { let _ = sqlx::PgPool::connect(u).await; }\n\
pub async fn f_pg_pwith_zq(o: sqlx::postgres::PgConnectOptions) { let _ = sqlx::PgPool::connect_with(o).await; }\n\
pub async fn f_pg_popts_zq(u: &str) { let _ = sqlx::postgres::PgPoolOptions::new().max_connections(2).connect(u).await; }\n\
pub async fn f_pg_popw_zq(o: sqlx::postgres::PgConnectOptions) { let _ = sqlx::postgres::PgPoolOptions::new().connect_with(o).await; }\n\
pub async fn f_pg_optsconn_zq(o: sqlx::postgres::PgConnectOptions) { let _ = o.connect().await; }\n\
pub async fn f_pg_listen_zq(u: &str) { let _ = sqlx::postgres::PgListener::connect(u).await; }\n\
pub async fn f_pg_listenw_zq(p: &sqlx::PgPool) { let _ = sqlx::postgres::PgListener::connect_with(p).await; }\n\
pub fn f_pg_lazy_zq(u: &str) { let _ = sqlx::PgPool::connect_lazy(u); }\n\
pub fn f_pg_lazyopts_zq(u: &str) { let _ = sqlx::postgres::PgPoolOptions::new().connect_lazy(u); }\n\
pub fn c_pg_lazywith_zq(o: sqlx::postgres::PgConnectOptions) { let _ = sqlx::postgres::PgPoolOptions::new().connect_lazy_with(o); }\n\
pub async fn f_pg_lazyacq_zq(u: &str) { let p = sqlx::PgPool::connect_lazy(u).unwrap(); let _ = p.acquire().await; }\n\
pub async fn f_pg_lazybegin_zq(u: &str) { let p = sqlx::PgPool::connect_lazy(u).unwrap(); let _ = p.begin().await; }\n\
pub async fn f_pg_cwithacq_zq(o: sqlx::postgres::PgConnectOptions) { let p = sqlx::PgPool::connect_with(o).await.unwrap(); let _ = p.acquire().await; }\n\
pub async fn f_pg_lazyoptsacq_zq(u: &str) { let p = sqlx::postgres::PgPoolOptions::new().connect_lazy(u).unwrap(); let _ = p.acquire().await; }\n\
pub async fn f_pg_acq_zq(p: &sqlx::PgPool) { let _ = p.acquire().await; }\n\
pub async fn f_pg_begin_zq(p: &sqlx::PgPool) { let _ = p.begin().await; }\n\
pub async fn f_pg_trybegin_zq(p: &sqlx::PgPool) { let _ = p.try_begin().await; }\n\
pub fn c_pg_tryacq_zq(p: &sqlx::PgPool) { let _ = p.try_acquire(); }\n\
pub fn c_pg_size_zq(p: &sqlx::PgPool) -> u32 { p.size() }\n\
pub fn f_pg_new_zq() { let _ = sqlx::postgres::PgConnectOptions::new(); }\n\
pub fn f_pg_nopg_zq() { let _ = sqlx::postgres::PgConnectOptions::new_without_pgpass(); }\n\
pub fn f_pg_parse_zq(u: &str) { let _ = u.parse::<sqlx::postgres::PgConnectOptions>(); }\n\
pub fn f_pg_fromstr_zq(u: &str) { let _ = sqlx::postgres::PgConnectOptions::from_str(u); }\n\
pub fn c_pg_setters_zq(o: sqlx::postgres::PgConnectOptions) -> sqlx::postgres::PgConnectOptions { o.host(\"h\").port(5).username(\"u\") }\n\
pub async fn f_my_acq_zq(p: &sqlx::MySqlPool) { let _ = p.acquire().await; }\n\
pub async fn f_my_begin_zq(p: &sqlx::MySqlPool) { let _ = p.begin().await; }\n\
pub async fn f_sq_conn_zq(u: &str) { let _ = sqlx::SqliteConnection::connect(u).await; }\n\
pub async fn f_sq_cwith_zq(o: &sqlx::sqlite::SqliteConnectOptions) { let _ = sqlx::SqliteConnection::connect_with(o).await; }\n\
pub async fn f_sq_pool_zq(u: &str) { let _ = sqlx::SqlitePool::connect(u).await; }\n\
pub async fn f_sq_pwith_zq(o: sqlx::sqlite::SqliteConnectOptions) { let _ = sqlx::SqlitePool::connect_with(o).await; }\n\
pub async fn f_sq_popts_zq(u: &str) { let _ = sqlx::sqlite::SqlitePoolOptions::new().connect(u).await; }\n\
pub async fn f_sq_optsconn_zq(o: sqlx::sqlite::SqliteConnectOptions) { let _ = o.connect().await; }\n\
pub async fn f_sq_acq_zq(p: &sqlx::SqlitePool) { let _ = p.acquire().await; }\n\
pub async fn f_sq_lazyacq_zq(u: &str) { let p = sqlx::SqlitePool::connect_lazy(u).unwrap(); let _ = p.acquire().await; }\n\
pub fn c_sq_lazy_zq(u: &str) { let _ = sqlx::SqlitePool::connect_lazy(u); }\n\
pub fn c_sq_opts_zq() -> sqlx::sqlite::SqliteConnectOptions { sqlx::sqlite::SqliteConnectOptions::new().filename(\"x.db\") }\n\
pub fn c_sq_parse_zq(u: &str) { let _ = u.parse::<sqlx::sqlite::SqliteConnectOptions>(); }\n\
pub fn c_pure_zq(a: u32) -> u32 { a + 1 }\n";

const EXPECT: &[(&str, &[&str])] = &[
    ("f_pg_conn_zq", &["Db", "Env", "Fs", "Ipc", "Net", "Rand"]),
    ("f_pg_cwith_zq", &["Db", "Ipc", "Net", "Rand"]),
    ("f_pg_pool_zq", &["Db", "Env", "Fs", "Ipc", "Net", "Rand"]),
    ("f_pg_pwith_zq", &["Db", "Ipc", "Net", "Rand"]),
    ("f_pg_popts_zq", &["Db", "Env", "Fs", "Ipc", "Net", "Rand"]),
    ("f_pg_popw_zq", &["Db", "Ipc", "Net", "Rand"]),
    ("f_pg_optsconn_zq", &["Db", "Ipc", "Net", "Rand"]),
    ("f_pg_listen_zq", &["Db", "Env", "Fs", "Ipc", "Net", "Rand"]),
    ("f_pg_listenw_zq", &["Db", "Ipc", "Net", "Rand"]),
    ("f_pg_lazy_zq", &["Env", "Fs"]),
    ("f_pg_lazyopts_zq", &["Env", "Fs"]),
    ("c_pg_lazywith_zq", &[]),
    ("f_pg_lazyacq_zq", &["Db", "Env", "Fs", "Ipc", "Net", "Rand"]),
    ("f_pg_lazybegin_zq", &["Db", "Env", "Fs", "Ipc", "Net", "Rand"]),
    ("f_pg_cwithacq_zq", &["Db", "Ipc", "Net", "Rand"]),
    ("f_pg_lazyoptsacq_zq", &["Db", "Env", "Fs", "Ipc", "Net", "Rand"]),
    ("f_pg_acq_zq", &["Db", "Ipc", "Net", "Rand"]),
    ("f_pg_begin_zq", &["Db", "Ipc", "Net", "Rand"]),
    ("f_pg_trybegin_zq", &["Db"]),
    ("c_pg_tryacq_zq", &[]),
    ("c_pg_size_zq", &[]),
    ("f_pg_new_zq", &["Env", "Fs"]),
    ("f_pg_nopg_zq", &["Env", "Fs"]),
    ("f_pg_parse_zq", &["Env", "Fs"]),
    ("f_pg_fromstr_zq", &["Env", "Fs"]),
    ("c_pg_setters_zq", &[]),
    ("f_my_acq_zq", &["Db", "Ipc", "Net", "Rand"]),
    ("f_my_begin_zq", &["Db", "Ipc", "Net", "Rand"]),
    ("f_sq_conn_zq", &["Db", "Env", "Fs", "Rand"]),
    ("f_sq_cwith_zq", &["Db", "Env", "Fs", "Rand"]),
    ("f_sq_pool_zq", &["Db", "Env", "Fs", "Rand"]),
    ("f_sq_pwith_zq", &["Db", "Env", "Fs", "Rand"]),
    ("f_sq_popts_zq", &["Db", "Env", "Fs", "Rand"]),
    ("f_sq_optsconn_zq", &["Db", "Env", "Fs", "Rand"]),
    ("f_sq_acq_zq", &["Db", "Env", "Fs", "Rand"]),
    ("f_sq_lazyacq_zq", &["Db", "Env", "Fs", "Rand"]),
    ("c_sq_lazy_zq", &[]),
    ("c_sq_opts_zq", &[]),
    ("c_sq_parse_zq", &[]),
    ("c_pure_zq", &[]),
];

/// `deny <E> <fn>` over every effect the table says the fn performs exits 1; over every other effect of the six an executed run
/// showed it NOT performing it exits 0 (the SQLite arm has no Net/Ipc; a lazy constructor has no Db/Net; `try_begin` has only Db).
const GATED: &[&str] = &["Db", "Net", "Ipc", "Rand", "Fs", "Env"];

fn check(lock: bool) {
    let d = fixture(if lock { "lock" } else { "nolock" }, lock, SRC);
    let (_, v) = scan(&d, None);
    for (f, want) in EXPECT {
        let want: Vec<String> = want.iter().map(|e| e.to_string()).collect();
        assert_eq!(field(&v, f, "inferred"), want, "{f} lock={lock}:\n{v:#}");
    }
    // a second-effect `Net` / `Fs` has a locator the call does not show: marked, never certified by a sibling literal
    for f in ["f_pg_conn_zq", "f_pg_acq_zq", "f_pg_lazyacq_zq", "f_my_acq_zq"] {
        assert!(field(&v, f, "incomplete").contains(&"Net".to_string()), "{f} Net must be incomplete");
    }
    for f in ["f_sq_conn_zq", "f_sq_acq_zq", "f_pg_new_zq", "f_pg_lazy_zq"] {
        assert!(field(&v, f, "incomplete").contains(&"Fs".to_string()), "{f} Fs must be incomplete");
    }
    // the SQLite arm must never carry Net/Ipc (copying the MySQL shape would fabricate)
    for (f, want) in EXPECT {
        if f.contains("_sq_") {
            assert!(!want.contains(&"Net") && !want.contains(&"Ipc"), "{f}");
        }
    }
    // the gate flips, exhaustively over the six effects, for the fns whose set is the point of this row
    for f in [
        "f_pg_conn_zq", "f_pg_cwith_zq", "f_pg_pool_zq", "f_pg_pwith_zq", "f_pg_popts_zq", "f_pg_popw_zq", "f_pg_optsconn_zq",
        "f_pg_listen_zq", "f_pg_listenw_zq", "f_pg_lazy_zq", "f_pg_lazyopts_zq", "f_pg_lazyacq_zq", "f_pg_lazybegin_zq",
        "f_pg_lazyoptsacq_zq", "f_pg_acq_zq", "f_pg_trybegin_zq", "f_pg_new_zq", "f_pg_parse_zq", "f_pg_fromstr_zq",
        "f_my_acq_zq", "f_sq_conn_zq", "f_sq_pool_zq", "f_sq_optsconn_zq", "f_sq_acq_zq", "f_sq_lazyacq_zq",
        "c_pg_lazywith_zq", "c_pg_tryacq_zq", "c_pg_size_zq", "c_pg_setters_zq", "c_sq_lazy_zq", "c_sq_opts_zq", "c_sq_parse_zq",
        "c_pure_zq",
    ] {
        let want = EXPECT.iter().find(|(n, _)| *n == f).unwrap().1;
        for e in GATED {
            let (code, _) = scan(&d, Some(&format!("deny {e} {f}\n")));
            assert_eq!(code, i32::from(want.contains(e)), "deny {e} {f} (lock={lock})");
        }
    }
    let _ = std::fs::remove_dir_all(&d);
}

#[test]
fn sqlx_pg_sqlite_connect_acquire_and_options_flip_their_gates_with_a_lock() {
    check(true);
}

#[test]
fn sqlx_pg_sqlite_connect_acquire_and_options_flip_their_gates_without_a_lock() {
    check(false);
}
