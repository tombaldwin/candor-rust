//! SOUNDNESS R1528, the 2026-11-11 weekly coverage-gate refresh: 30 grown rows that executed fixtures showed to be real
//! effects now carry classify rules (the evidence is in the comment at each rule in `src/lib.rs`); the pure neighbours beside
//! each pin the carve-out so a widened rule fails here, and the verdicted-not-ruled chrono rows stay unclaimed (R1693).
use candor_classify::{also_effects, classify};

#[test]
fn coverage_gate_refresh_2026_11_11_rules() {
    // sync `postgres` wrappers (tokio_postgres had the rule; the sync crate's rows grew when it started reaching it)
    for p in ["Client::check_connection", "Client::execute_typed", "Client::query_typed_one", "Client::query_typed_opt",
              "Client::query_typed_raw", "Transaction::bind", "Transaction::commit", "Transaction::rollback",
              "Transaction::execute_typed", "Transaction::query_portal", "Transaction::query_portal_raw",
              "Transaction::query_typed_one", "Transaction::query_typed_opt", "Transaction::query_typed_raw",
              "Transaction::savepoint"] {
        assert_eq!(classify("postgres", &format!("postgres::{p}")), Some("Db"), "{p}");
    }
    for p in ["Client::is_closed", "Client::cancel_token", "Client::build_transaction", "Client::new"] {
        assert_eq!(classify("postgres", &format!("postgres::{p}")), None, "{p}");
    }
    assert_eq!(also_effects("postgres", "postgres::Transaction::commit"), vec!["Log"]);
    assert!(also_effects("postgres", "postgres::Transaction::query_typed_one").is_empty(), "0 records EXECUTED (R1617)");
    // sqlx migration runners, both the facade and the core spelling; `new` keeps Fs and `iter` stays pure
    for krate in ["sqlx", "sqlx_core"] {
        for m in ["run", "run_to", "skip", "undo", "run_direct"] {
            assert_eq!(classify(krate, &format!("{krate}::migrate::Migrator::{m}")), Some("Db"), "{krate} {m}");
        }
        assert_eq!(classify(krate, &format!("{krate}::migrate::Migrator::new")), Some("Fs"));
        for m in ["iter", "len", "set_locking", "dangerous_set_applied"] {
            assert_eq!(classify(krate, &format!("{krate}::migrate::Migrator::{m}")), None, "{krate} {m}");
        }
    }
    // redis sync PubSub command verbs (`get_message` stays the Net reader; the async pubsub module stays out)
    for m in ["subscribe", "psubscribe", "unsubscribe", "punsubscribe", "ping", "ping_message"] {
        assert_eq!(classify("redis", &format!("redis::PubSub::{m}")), Some("Db"), "{m}");
        assert_eq!(classify("redis", &format!("redis::connection::PubSub::{m}")), Some("Db"), "{m}");
        assert_eq!(classify("redis", &format!("redis::aio::pubsub::PubSub::{m}")), None, "async {m}");
    }
    assert_eq!(classify("redis", "redis::PubSub::get_message"), Some("Net"));
    assert_eq!(classify("redis", "redis::PubSub::get_message_timeout"), None);
    // mongodb `Aggregate` action; the builder-setter neighbours stay pure
    for m in ["execute", "batch"] {
        assert_eq!(classify("mongodb", &format!("mongodb::Aggregate::{m}")), Some("Db"), "{m}");
        assert_eq!(classify("mongodb", &format!("mongodb::action::aggregate::Aggregate::{m}")), Some("Db"), "{m}");
    }
    for m in ["session", "with_type", "hint", "allow_disk_use", "batch_size"] {
        assert_eq!(classify("mongodb", &format!("mongodb::Aggregate::{m}")), None, "{m}");
    }
    // async-nats `Stream::unpin_consumer` joins its pause/resume/reset siblings
    assert_eq!(classify("async_nats", "async_nats::jetstream::stream::Stream::unpin_consumer"), Some("Net"));
    // sea-orm `accepts` reads the PG* environment (and stats the socket dirs: the second effect)
    for p in ["sea_orm::SqlxPostgresConnector::accepts", "sea_orm::driver::sqlx_postgres::SqlxPostgresConnector::accepts"] {
        assert_eq!(classify("sea_orm", p), Some("Env"), "{p}");
        assert_eq!(also_effects("sea_orm", p), vec!["Fs"], "{p}");
    }
    assert_eq!(classify("sea_orm", "sea_orm::SqlxMySqlConnector::accepts"), None);
    // tracing-subscriber `OffsetTime::local_rfc_3339` = the time crate's local-offset probe (Clock + the zone-file access)
    for p in ["tracing_subscriber::fmt::time::OffsetTime::local_rfc_3339",
              "tracing_subscriber::fmt::time::time_crate::OffsetTime::local_rfc_3339"] {
        assert_eq!(classify("tracing_subscriber", p), Some("Clock"), "{p}");
        assert_eq!(also_effects("tracing_subscriber", p), vec!["Fs"], "{p}");
    }
    assert_eq!(classify("tracing_subscriber", "tracing_subscriber::fmt::time::OffsetTime::new"), None);
    // ureq `Agent::shutdown` (R1696): the consumer route to `TcpStream::shutdown`; the constructors stay as they were
    assert_eq!(classify("ureq", "ureq::Agent::shutdown"), Some("Net"));
    assert_eq!(classify("ureq", "ureq::agent::Agent::shutdown"), Some("Net"));
    assert_eq!(classify("ureq", "ureq::Agent::new_with_defaults"), Some("Env"));
    assert_eq!(classify("ureq", "ureq::config::Config::new_agent"), None, "R1694: pure, EXECUTED 0 events");
    assert_eq!(classify("ureq", "ureq::Body::read_to_string"), None, "R1695: conditional on the body's source");
    // the verdicted-not-ruled chrono rows stay unclaimed (R1693): a rule would charge every `Utc` consumer
    for p in ["chrono::Date::and_hms_opt", "chrono::DateTime::checked_add_days", "chrono::DateTime::with_time",
              "chrono::NaiveDateTime::parse_from_str"] {
        assert_eq!(classify("chrono", p), None, "{p}");
    }
}
