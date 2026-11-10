//! SOUNDNESS R1618 — three silences found beside R1617, each EXECUTED (macOS, DYLD interposer / installed `log` logger, against
//! postgres:16 and mysql:8):
//!   (1) `deadpool_postgres::Pool::{get, timeout_get}` read pure; a COLD pool's `get` = connect + 1 getentropy (+ auth), a warm one 0.
//!   (2) `build_transaction().start()` (tokio_postgres, postgres, deadpool) read pure; it sends BEGIN and logs 1 record.
//!   (3) sqlx MySQL connect: `connect` was Db only, and `connect_with` (three spellings) read PURE; every spelling executed
//!       connect + 5 getentropy on the default TLS arm. `ssl-ca=<file>` opening a file (Fs) is NOT a row: see the row text.
//! The negatives are the same types' measured 0-event neighbours: a row keyed on the type alone would pass every positive.

use candor_classify::{also_effects, classify, classify_all};

#[test]
fn deadpool_pool_get_is_db_with_net_ipc_rand_and_the_pool_neighbours_are_pure() {
    for p in ["deadpool_postgres::Pool::get", "deadpool_postgres::Pool::timeout_get"] {
        assert_eq!(classify("deadpool_postgres", p), Some("Db"), "{p}");
        assert_eq!(classify_all("deadpool_postgres", p), Some(vec!["Db", "Net", "Ipc", "Rand"]), "{p}");
    }
    // EXECUTED 0 events: status / is_closed / manager / resize / close, and the config-only create_pool.
    for m in ["status", "is_closed", "manager", "resize", "close", "retain", "builder"] {
        let p = format!("deadpool_postgres::Pool::{m}");
        assert!(classify("deadpool_postgres", &p).is_none() && also_effects("deadpool_postgres", &p).is_empty(), "{p}");
    }
    assert!(also_effects("deadpool_postgres", "deadpool_postgres::Config::create_pool").is_empty());
    // Keyed on module + type + method: another crate's `Pool::get`, a different head, and a different method are not claimed.
    assert!(classify("r2d2", "r2d2::Pool::get").is_none());
    assert!(classify("deadpool", "deadpool::managed::Pool::get").is_none());
    assert!(classify("deadpool_postgres", "deadpool_postgres::Config::get").is_none());
    assert!(classify("deadpool_postgres", "deadpool_postgres::Pool::get_or_create").is_none());
}

#[test]
fn build_transaction_start_is_db_and_log_and_the_builder_setters_are_pure() {
    for p in [
        "tokio_postgres::Client::start", "tokio_postgres::TransactionBuilder::start",
        "postgres::Client::start", "postgres::TransactionBuilder::start",
        "deadpool_postgres::Client::start", "deadpool_postgres::Object::start", "deadpool_postgres::TransactionBuilder::start",
    ] {
        let c = p.split("::").next().unwrap();
        assert_eq!(classify_all(c, p), Some(vec!["Db", "Log"]), "{p}");
    }
    // EXECUTED 0 events / 0 records: the setters and `build_transaction` itself only configure the builder.
    for p in [
        "tokio_postgres::Client::isolation_level", "tokio_postgres::Client::read_only", "tokio_postgres::Client::deferrable",
        "tokio_postgres::Client::build_transaction", "postgres::Client::build_transaction",
        "tokio_postgres::TransactionBuilder::isolation_level",
    ] {
        let c = p.split("::").next().unwrap();
        assert!(classify(c, p).is_none() && also_effects(c, p).is_empty(), "{p}");
    }
    // A `start` on another crate's type, or on a type of these crates that is not a builder/client, is not claimed.
    assert!(classify("tokio", "tokio::runtime::Runtime::start").is_none());
    assert!(classify("tokio_postgres", "tokio_postgres::Config::start").is_none());
    assert!(classify("deadpool_postgres", "deadpool_postgres::Config::start").is_none());
}

#[test]
fn sqlx_mysql_connect_spellings_all_carry_db_net_ipc_rand_and_lazy_and_options_do_not() {
    for p in [
        "sqlx::MySqlConnection::connect", "sqlx::MySqlConnection::connect_with",
        "sqlx::MySqlPool::connect", "sqlx::MySqlPool::connect_with",
        "sqlx::mysql::MySqlPoolOptions::connect", "sqlx::mysql::MySqlPoolOptions::connect_with",
        "sqlx::mysql::MySqlConnectOptions::connect",
    ] {
        assert_eq!(classify_all("sqlx", p), Some(vec!["Db", "Net", "Ipc", "Rand"]), "{p}");
    }
    // `connect_with` was ABSENT (pure) before R1618 for the connection, the pool and the pool options.
    // EXECUTED 0 events: connect_lazy opens nothing (R1589); parsing / building options and `max_connections` do nothing.
    for p in [
        "sqlx::MySqlPool::connect_lazy", "sqlx::mysql::MySqlConnectOptions::new", "sqlx::mysql::MySqlConnectOptions::host",
        "sqlx::mysql::MySqlPoolOptions::new", "sqlx::mysql::MySqlPoolOptions::max_connections",
    ] {
        assert!(also_effects("sqlx", p).is_empty(), "{p}");
    }
    assert!(classify("sqlx", "sqlx::MySqlPool::connect_lazy").is_none());
    // A look-alike `connect_with` on a type that is not a connection / pool / pool options is not claimed.
    assert!(classify("sqlx", "sqlx::MySqlRow::connect_with").is_none());
    // The rows above are MySQL's. Postgres and sqlite were executed by R1623 (`r1623_sqlx_pg_sqlite.rs`) and carry their OWN
    // second effects: a MySQL row must never leak onto a driver it was not measured on (sqlite has no Net/Ipc).
    for p in ["sqlx::PgConnection::connect", "sqlx::PgPool::connect"] {
        let v = classify_all("sqlx", p).unwrap();
        assert!(v.contains(&"Net") && v.contains(&"Env"), "{p}: {v:?}");
    }
    let sq = classify_all("sqlx", "sqlx::SqliteConnection::connect").unwrap();
    assert!(!sq.contains(&"Net") && !sq.contains(&"Ipc"), "{sq:?}");
    // The ssl-ca Fs is URL-conditional and is NOT a table row: no Fs on any spelling.
    for p in ["sqlx::MySqlConnection::connect", "sqlx::MySqlPool::connect_with"] {
        assert!(!classify_all("sqlx", p).unwrap().contains(&"Fs"), "{p}");
    }
}
