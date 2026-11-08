//! SOUNDNESS R1528: the open.tsv rows that were executed or verified true and were silent at
//! consumers. This file pins each rule through the public `classify` API next to a same-crate
//! pure neighbour, so a regression that drops a rule (or widens one onto a pure neighbour) fails
//! here. Executed evidence: ~/candor-handoff/rust-r1528/rules/executed-logs/.

use candor_classify::classify;

#[test]
fn r1528_rules_fire_on_the_verified_rows() {
    assert_eq!(classify("tempfile", "tempfile::NamedTempFile::into_file"), Some("Fs"));
    for v in ["with_prefix", "with_prefix_in", "with_suffix", "with_suffix_in"] {
        assert_eq!(classify("tempfile", &format!("tempfile::TempDir::{v}")), Some("Fs"), "{v}");
    }
    assert_eq!(classify("chrono", "chrono::Utc::today"), Some("Clock"));
    assert_eq!(classify("chrono", "chrono::Local::today"), Some("Clock"));
    assert_eq!(classify("tokio_postgres", "tokio_postgres::Client::cancel_query"), Some("Net"));
    assert_eq!(classify("tokio_postgres", "tokio_postgres::Transaction::cancel_query"), Some("Net"));
    assert_eq!(classify("postgres", "postgres::Client::is_valid"), Some("Db"));
    for p in [
        "rdkafka::Client::fetch_group_list",
        "rdkafka::BaseConsumer::fetch_group_list",
        "rdkafka::StreamConsumer::fetch_group_list",
    ] {
        assert_eq!(classify("rdkafka", p), Some("Net"), "{p}");
    }
    assert_eq!(classify("tracing_subscriber", "tracing_subscriber::fmt::Layer::new"), Some("Env"));
}

#[test]
fn r1528_rules_do_not_reach_pure_neighbours() {
    assert_eq!(classify("tempfile", "tempfile::NamedTempFile::path"), None);
    assert_eq!(classify("tempfile", "tempfile::TempDir::into_path"), None);
    assert_eq!(classify("chrono", "chrono::NaiveDate::today"), None);
    assert_eq!(classify("chrono", "chrono::DateTime::parse_from_rfc2822"), None);
    assert_eq!(classify("tokio_postgres", "tokio_postgres::Client::is_closed"), None);
    assert_eq!(classify("tokio_postgres", "tokio_postgres::Client::is_valid"), None);
    assert_eq!(classify("tracing_subscriber", "tracing_subscriber::fmt::Layer::with_writer"), None);
}
