//! SOUNDNESS R1623 — the classifier half of the sqlx 0.9.0 Postgres / SQLite / pool rows (the scan half, with the gate flips and the
//! receiver typing, is `candor-scan/tests/r1623_sqlx_pg_sqlite.rs`). Every positive is EXECUTED (macOS, DYLD interposer, postgres:16 with
//! TLS and SCRAM, mysql:8, real sqlite files; handoff `rust-r1623/`); every negative is a neighbour the same run measured at 0 events.

use candor_classify::{also_effects, also_fs_kind, classify, classify_all};

fn all(p: &str) -> Option<Vec<&'static str>> {
    classify_all("sqlx", p)
}

#[test]
fn postgres_connect_spellings_are_db_with_net_ipc_rand_and_a_url_spelling_adds_env_and_fs() {
    // options already built: connect only
    for p in [
        "sqlx::PgConnection::connect_with", "sqlx::PgPool::connect_with", "sqlx::postgres::PgPoolOptions::connect_with",
        "sqlx::postgres::PgConnectOptions::connect", "sqlx::postgres::PgListener::connect_with",
    ] {
        assert_eq!(all(p), Some(vec!["Db", "Net", "Ipc", "Rand"]), "{p}");
    }
    // a URL is parsed inside the call
    for p in [
        "sqlx::PgConnection::connect", "sqlx::PgPool::connect", "sqlx::postgres::PgPoolOptions::connect",
        "sqlx::postgres::PgListener::connect",
    ] {
        let v = all(p).unwrap_or_default();
        assert_eq!(v[0], "Db", "{p}");
        for e in ["Net", "Ipc", "Rand", "Env", "Fs"] {
            assert!(v.contains(&e), "{p} lacks {e}: {v:?}");
        }
        assert_eq!(also_fs_kind("sqlx", p), vec!["read"], "{p}: the parse stats and opens ~/.pgpass read-only");
    }
}

#[test]
fn postgres_lazy_url_and_option_constructors_are_env_and_fs_with_no_connect() {
    for p in [
        "sqlx::PgPool::connect_lazy", "sqlx::postgres::PgPoolOptions::connect_lazy",
        "sqlx::postgres::PgConnectOptions::new", "sqlx::postgres::PgConnectOptions::new_without_pgpass",
        "sqlx::postgres::PgConnectOptions::default", "sqlx::postgres::PgConnectOptions::from_str",
        "sqlx::postgres::PgConnectOptions::from_url",
    ] {
        assert_eq!(all(p), Some(vec!["Env", "Fs"]), "{p}");
    }
    // EXECUTED 0 events: no URL, and the setters only fill a struct
    for p in [
        "sqlx::postgres::PgPoolOptions::connect_lazy_with", "sqlx::PgPool::connect_lazy_with",
        "sqlx::postgres::PgConnectOptions::host", "sqlx::postgres::PgConnectOptions::port",
        "sqlx::postgres::PgConnectOptions::ssl_mode", "sqlx::postgres::PgConnectOptions::username",
        "sqlx::postgres::PgPoolOptions::new", "sqlx::postgres::PgPoolOptions::max_connections",
    ] {
        assert!(classify("sqlx", p).is_none() && also_effects("sqlx", p).is_empty(), "{p}");
    }
    // another driver's `new` / `from_str` is not Postgres's
    for p in ["sqlx::sqlite::SqliteConnectOptions::new", "sqlx::mysql::MySqlConnectOptions::from_str", "sqlx::mysql::MySqlConnectOptions::new"] {
        assert!(classify("sqlx", p).is_none(), "{p}");
    }
}

#[test]
fn pool_acquire_class_carries_the_connect_effects_per_driver() {
    for p in ["sqlx::PgPool::acquire", "sqlx::PgPool::begin", "sqlx::postgres::PgPool::begin_with", "sqlx::MySqlPool::acquire", "sqlx::MySqlPool::begin"] {
        assert_eq!(all(p), Some(vec!["Db", "Net", "Ipc", "Rand"]), "{p}");
    }
    // the chain-root guess a `let p = PgPoolOptions::new().connect_lazy(u).unwrap(); p.acquire()` forms
    assert_eq!(all("sqlx::postgres::PgPoolOptions::acquire"), Some(vec!["Db", "Net", "Ipc", "Rand"]));
    // `try_acquire` returns an idle connection or None: EXECUTED 0 events cold and warm. `try_begin` sends BEGIN on a warm pool
    // (postgres `log_statement=all`) and is Db only.
    assert!(classify("sqlx", "sqlx::PgPool::try_acquire").is_none() && also_effects("sqlx", "sqlx::PgPool::try_acquire").is_empty());
    assert_eq!(all("sqlx::PgPool::try_begin"), Some(vec!["Db"]));
    assert_eq!(all("sqlx::postgres::PgPool::try_begin"), Some(vec!["Db"]));
    // a generic `Pool<DB>` names no driver: its acquire stays Db only (the effect set is the driver's)
    assert_eq!(all("sqlx::Pool::acquire"), Some(vec!["Db"]));
    assert!(classify("mycrate", "mycrate::PgPool::try_begin").is_none());
}

#[test]
fn sqlite_connect_is_fs_env_rand_and_never_net_or_ipc() {
    for p in [
        "sqlx::SqliteConnection::connect", "sqlx::SqliteConnection::connect_with", "sqlx::SqlitePool::connect",
        "sqlx::SqlitePool::connect_with", "sqlx::sqlite::SqlitePoolOptions::connect", "sqlx::sqlite::SqlitePoolOptions::connect_with",
        "sqlx::sqlite::SqliteConnectOptions::connect", "sqlx::SqlitePool::acquire", "sqlx::SqlitePool::begin",
        "sqlx::sqlite::SqlitePool::begin_with",
    ] {
        let v = all(p).unwrap_or_default();
        assert_eq!(v[0], "Db", "{p}");
        for e in ["Fs", "Env", "Rand"] {
            assert!(v.contains(&e), "{p} lacks {e}: {v:?}");
        }
        assert!(!v.contains(&"Net") && !v.contains(&"Ipc"), "{p}: sqlite opens a file, never a socket: {v:?}");
        // the direction depends on `mode=`: unknown, so the charging site records the poison marker, never a guess
        assert!(also_fs_kind("sqlx", p).is_empty(), "{p}");
    }
    // EXECUTED 0 events
    for p in [
        "sqlx::SqlitePool::connect_lazy", "sqlx::sqlite::SqlitePoolOptions::connect_lazy", "sqlx::sqlite::SqliteConnectOptions::new",
        "sqlx::sqlite::SqliteConnectOptions::from_str", "sqlx::sqlite::SqliteConnectOptions::filename", "sqlx::SqlitePool::try_acquire",
    ] {
        assert!(classify("sqlx", p).is_none() && also_effects("sqlx", p).is_empty(), "{p}");
    }
}

#[test]
fn r1618_mysql_connect_rows_are_unchanged() {
    for p in ["sqlx::MySqlConnection::connect", "sqlx::MySqlPool::connect_with", "sqlx::mysql::MySqlPoolOptions::connect"] {
        assert_eq!(all(p), Some(vec!["Db", "Net", "Ipc", "Rand"]), "{p}");
    }
    assert!(classify("sqlx", "sqlx::MySqlPool::connect_lazy").is_none());
}
