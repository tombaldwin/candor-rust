//! SOUNDNESS R1528, the 2026-11-28 coverage-gate refresh (issue #18, run 45624099895): 34 public entry points of calibrated crates
//! grew in `open.tsv` without a verdict. 30 now carry a `classify()` rule (every one read in the published crate source, evidence
//! at each rule in `src/lib.rs`), 2 a `REVIEWED_PURE_ENTRIES` read (isahc, rusqlite), and 2 an `OPEN_ENTRY_VERDICTS` record (R1559, R1582).
//! Each rule is asserted in BOTH spellings the generator guesses (the crate-root alias and the module path) beside the pure
//! twin that shares its type or module and must stay uncharged.
use candor_classify::{classify, OPEN_ENTRY_VERDICTS, REVIEWED_PURE_ENTRIES};

fn charged(krate: &str, effect: &str, paths: &[&str]) {
    for p in paths {
        assert_eq!(classify(krate, p), Some(effect), "{p}");
    }
}

fn pure(krate: &str, paths: &[&str]) {
    for p in paths {
        assert_eq!(classify(krate, p), None, "a pure twin must stay uncharged: {p}");
    }
}

#[test]
fn r1528_2026_11_28_env_logger_reads_the_environment() {
    // `Builder::{from_env, from_default_env, parse_env, parse_default_env}` reach `Var::get` -> `env::var(RUST_LOG)`
    // (logger.rs:872); `Logger::from_*env`, `env_logger::builder` and `env_logger::from_env` are one call over `Builder::from_*`.
    for m in ["from_env", "from_default_env", "parse_env", "parse_default_env"] {
        charged("env_logger", "Env", &[&format!("env_logger::Builder::{m}"), &format!("env_logger::logger::Builder::{m}")]);
    }
    for m in ["from_env", "from_default_env"] {
        charged("env_logger", "Env", &[&format!("env_logger::Logger::{m}"), &format!("env_logger::logger::Logger::{m}")]);
    }
    charged(
        "env_logger",
        "Env",
        &["env_logger::builder", "env_logger::logger::builder", "env_logger::from_env", "env_logger::logger::from_env"],
    );
    // the neighbours that only build or name a variable read nothing; `init*` keeps its `Log`
    pure(
        "env_logger",
        &[
            "env_logger::Builder::new",
            "env_logger::Builder::build",
            "env_logger::Builder::filter_level",
            "env_logger::Builder::parse_filters",
            "env_logger::Builder::parse_write_style",
            "env_logger::Env::new",
            "env_logger::Env::filter",
            "env_logger::Logger::filter",
        ],
    );
    charged("env_logger", "Log", &["env_logger::init", "env_logger::Builder::init", "env_logger::try_init_from_env"]);
    // another crate's same-named builder is not claimed
    assert_eq!(classify("mycrate", "mycrate::Builder::from_env"), None);
}

#[test]
fn r1528_2026_11_28_diesel_sqlite_entries() {
    // sqlite3_{auto_extension,cancel_auto_extension,reset_auto_extension} change what every LATER connection runs (the same
    // process-global registry `rusqlite::register_auto_extension` is charged for); `batch_execute` is `sqlite3_exec`; the blob
    // opener is `sqlite3_blob_open`.
    for m in ["register_auto_extension", "cancel_auto_extension", "reset_auto_extension"] {
        charged("diesel", "Db", &[&format!("diesel::sqlite::{m}"), &format!("diesel::sqlite::auto_extension::{m}")]);
    }
    charged(
        "diesel",
        "Db",
        &[
            "diesel::sqlite::BorrowedSqliteConnection::batch_execute",
            "diesel::sqlite::connection::borrowed_connection::BorrowedSqliteConnection::batch_execute",
            "diesel::sqlite::SqliteConnection::get_read_only_blob",
            "diesel::sqlite::connection::sqlite_blob::SqliteConnection::get_read_only_blob",
        ],
    );
    pure(
        "diesel",
        &[
            "diesel::sqlite::BorrowedSqliteConnection::inner_mut",
            "diesel::sqlite::SqliteReadOnlyBlob::len",
            "diesel::sqlite::SqliteReadOnlyBlob::is_empty",
            "diesel::sqlite::SqliteConnection::set_instrumentation",
        ],
    );
}

#[test]
fn r1528_2026_11_28_grep_cli_ignore_lscolors() {
    // `build_by_magic` is `build`'s sibling: it opens the file, reads 8 bytes and spawns the decompressor on a magic match.
    charged(
        "grep_cli",
        "Exec",
        &["grep_cli::DecompressionReaderBuilder::build_by_magic", "grep_cli::decompress::DecompressionReaderBuilder::build_by_magic"],
    );
    pure("grep_cli", &["grep_cli::DecompressionReaderBuilder::new", "grep_cli::DecompressionMatcherBuilder::new"]);
    // `skip_stdout(true)` runs `same_file::Handle::stdout()` and `file.metadata()` (an fstat of descriptor 1).
    charged("ignore", "Fs", &["ignore::WalkBuilder::skip_stdout", "ignore::walk::WalkBuilder::skip_stdout"]);
    pure("ignore", &["ignore::WalkBuilder::hidden", "ignore::WalkBuilder::max_depth", "ignore::WalkBuilder::new"]);
    // `style_for_path` is `path.symlink_metadata()` then the metadata-taking variant: an lstat. The earlier rule read it PURE.
    charged("lscolors", "Fs", &["lscolors::LsColors::style_for_path"]);
    charged("lscolors", "Env", &["lscolors::LsColors::from_env"]);
    pure(
        "lscolors",
        &["lscolors::LsColors::from_string", "lscolors::LsColors::style_for_path_with_metadata", "lscolors::LsColors::style_for_str"],
    );
}

#[test]
fn r1528_2026_11_28_jiff_1_0_renames() {
    // jiff 1.0 renamed `Zoned` -> `ZonedDateTime` and `to_zoned`/`parse_zoned` -> `to_zoned_datetime`/`parse_zoned_datetime`;
    // the bodies are the 0.2 ones the existing rule reads (`tz::db().get(name)`, `<same>_with(crate::tz::db(), ..)`).
    charged(
        "jiff",
        "Fs",
        &[
            "jiff::ZonedDateTime::in_tz",
            "jiff::zoned::ZonedDateTime::in_tz",
            "jiff::BrokenDownTime::to_zoned_datetime",
            "jiff::fmt::strtime::BrokenDownTime::to_zoned_datetime",
            "jiff::DateTimeParser::parse_zoned_datetime",
            "jiff::fmt::temporal::DateTimeParser::parse_zoned_datetime",
        ],
    );
    // the 0.2 spellings keep theirs
    charged("jiff", "Fs", &["jiff::Zoned::in_tz", "jiff::BrokenDownTime::to_zoned", "jiff::DateTimeParser::parse_zoned"]);
    // the caller-supplied-database variant is the DATABASE's effect (R1559), not a rule
    pure(
        "jiff",
        &[
            "jiff::BrokenDownTime::to_zoned_datetime_with",
            "jiff::fmt::strtime::BrokenDownTime::to_zoned_datetime_with",
            "jiff::ZonedDateTime::with_time_zone",
            "jiff::ZonedDateTime::timestamp",
        ],
    );
}

#[test]
fn r1528_2026_11_28_notify_9_path_registration() {
    // notify 9 adds `Watcher::watch_with(path, config)` (`watch` is `watch_with(path, WatchPathConfig::new(mode))`) and
    // `Watcher::update_paths(ops)` (a batch of watch/unwatch ops): the same path registration `watch`/`unwatch` already charge.
    for w in ["INotifyWatcher", "FsEventWatcher", "KqueueWatcher", "PollWatcher", "ReadDirectoryChangesWatcher"] {
        charged("notify", "Fs", &[&format!("notify::{w}::watch_with")]);
    }
    for w in ["INotifyWatcher", "FsEventWatcher"] {
        charged("notify", "Fs", &[&format!("notify::{w}::update_paths")]);
    }
    charged(
        "notify",
        "Fs",
        &[
            "notify::inotify::INotifyWatcher::watch_with",
            "notify::fsevent::FsEventWatcher::update_paths",
            "notify::windows::ReadDirectoryChangesWatcher::watch_with",
            "notify::Watcher::watch_with",
            "notify::Watcher::update_paths",
        ],
    );
    pure("notify", &["notify::Config::default", "notify::WatchPathConfig::new", "notify::PollWatcher::watched_paths"]);
}

#[test]
fn r1528_2026_11_28_rusqlite_redis_sqlx() {
    // `Connection::file_control` -> `sqlite3_file_control` and `Connection::wal_checkpoint_v2` -> `sqlite3_wal_checkpoint_v2`:
    // both are on the `ffi` table already, their wrappers were not.
    charged(
        "rusqlite",
        "Db",
        &[
            "rusqlite::Connection::file_control",
            "rusqlite::Connection::wal_checkpoint_v2",
            "rusqlite::wal::Connection::wal_checkpoint_v2",
        ],
    );
    pure("rusqlite", &["rusqlite::Connection::is_autocommit", "rusqlite::Connection::changes", "rusqlite::wal::CheckpointMode::default"]);
    // redis `ClusterClient::get_async_generic_connection::<C>` is `ClusterConnection::new(&initial_nodes, params).await`, the
    // body of `get_async_connection_with_config`, which already carries `Db`.
    charged(
        "redis",
        "Db",
        &[
            "redis::ClusterClient::get_async_generic_connection",
            "redis::cluster_handling::client::ClusterClient::get_async_generic_connection",
        ],
    );
    pure("redis", &["redis::ClusterClient::new", "redis::cluster_handling::client::ClusterClientBuilder::retries"]);
    // sqlx-core `net::tls::connector(config)` builds the TLS connector: it reads the root, client-certificate and key FILES the
    // `TlsConfig` names (`CertificateInput::File` -> `fs::read`) and the OS trust store.
    charged("sqlx_core", "Fs", &["sqlx_core::net::tls::connector"]);
    // the handshake over a socket keeps its own `Net`; the connector build is not that
    charged("sqlx_core", "Net", &["sqlx_core::net::tls::handshake"]);
}

#[test]
fn r1528_2026_11_28_verdicts_and_reads() {
    for (krate, entry, verdict) in [
        ("jiff", "jiff::fmt::strtime::BrokenDownTime::to_zoned_datetime_with", "R1559:conditional"),
        ("rdkafka", "rdkafka::client::Client::sasl_set_credentials", "R1582:silent"),
    ] {
        assert!(OPEN_ENTRY_VERDICTS.contains(&(krate, entry, verdict)), "{entry}");
        assert_eq!(classify(krate, entry), None, "a verdict is not a rule: {entry}");
    }
    for (krate, entry) in [("isahc", "isahc::Builder::with_config"), ("rusqlite", "rusqlite::Connection::wal_hook")] {
        assert!(REVIEWED_PURE_ENTRIES.contains(&(krate, entry)), "{entry}");
        assert_eq!(classify(krate, entry), None, "a read is not a rule: {entry}");
    }
}
