//! SOUNDNESS R1617 — second effects for the Log rows (a dependency's own logging/tracing call is `Log` at the consumer's
//! call, SPEC.md:427-431) and the non-Log extras (postgres connect, fastrand/tempfile clock seed).
//!
//! Every positive here was EXECUTED with the effect observed inside the call window (a `log` logger + `tracing` subscriber
//! for `Log`, a DYLD interposer for the rest); every negative is the same type's measured 0-event neighbour. The negatives
//! carry the weight: a row keyed on the type alone would pass every positive.

use candor_classify::{also_effects, classify, classify_all, AlsoOn, ALSO_EFFECTS};

#[test]
fn postgres_family_verbs_carry_log_and_the_typed_family_does_not() {
    for (c, t) in [("tokio_postgres", "Client"), ("tokio_postgres", "Transaction"), ("postgres", "Client"), ("postgres", "Transaction")] {
        for m in ["query", "query_one", "query_opt", "query_raw", "execute", "batch_execute", "simple_query", "prepare",
                  "prepare_typed", "copy_in", "copy_out", "transaction"] {
            let p = format!("{c}::{t}::{m}");
            assert_eq!(classify_all(c, &p), Some(vec!["Db", "Log"]), "{p}");
        }
    }
    for p in ["tokio_postgres::Client::execute_raw", "tokio_postgres::Client::simple_query_raw", "postgres::Client::is_valid"] {
        let c = p.split("::").next().unwrap();
        assert_eq!(classify_all(c, p), Some(vec!["Db", "Log"]), "{p}");
    }
    // EXECUTED 0 records at DEBUG: the typed protocol path has no log site, so it stays a single-effect Db.
    for p in ["tokio_postgres::Client::query_typed", "tokio_postgres::Client::query_typed_raw", "tokio_postgres::Client::query_typed_one",
              "tokio_postgres::Client::execute_typed", "tokio_postgres::Transaction::query_typed", "postgres::Client::query_typed"] {
        let c = p.split("::").next().unwrap();
        assert_eq!(classify_all(c, p), Some(vec!["Db"]), "{p}: 0 log records");
    }
    // Idle neighbours charge nothing at all.
    for p in ["tokio_postgres::Client::is_closed", "tokio_postgres::Client::cancel_token", "tokio_postgres::Client::build_transaction"] {
        assert!(classify("tokio_postgres", p).is_none() && also_effects("tokio_postgres", p).is_empty(), "{p}");
    }
}

#[test]
fn deadpool_rows_cover_its_own_spellings_only() {
    for p in ["deadpool_postgres::Client::query", "deadpool_postgres::Client::prepare_cached", "deadpool_postgres::ClientWrapper::transaction",
              "deadpool_postgres::Transaction::commit", "deadpool_postgres::Transaction::prepare_typed_cached"] {
        assert_eq!(classify_all("deadpool_postgres", p), Some(vec!["Db", "Log"]), "{p}");
    }
    // The pool is lazy and `Pool::get` is not a classified path: no primary, so no second effect (the table never stands alone).
    assert!(classify_all("deadpool_postgres", "deadpool_postgres::Pool::get").is_none());
    assert!(also_effects("deadpool_postgres", "deadpool_postgres::Config::create_pool").is_empty());
}

#[test]
fn reqwest_execute_and_send_log_but_builders_do_not() {
    for p in ["reqwest::Client::execute", "reqwest::blocking::Client::execute", "reqwest::RequestBuilder::send", "reqwest::blocking::RequestBuilder::send"] {
        assert_eq!(classify_all("reqwest", p), Some(vec!["Net", "Log"]), "{p}");
    }
    // EXECUTED 0 records: building a request performs no I/O.
    assert_eq!(also_effects("reqwest", "reqwest::Client::get"), Vec::<&str>::new());
    assert!(!also_effects("reqwest", "reqwest::RequestBuilder::header").contains(&"Log"));
    // A look-alike in another crate is not claimed.
    assert!(also_effects("ureq", "ureq::Agent::execute").is_empty());
}

#[test]
fn postgres_connect_carries_net_ipc_rand_and_a_tls_handshake_does_not() {
    for p in ["tokio_postgres::Config::connect", "tokio_postgres::config::Config::connect", "tokio_postgres::connect"] {
        assert_eq!(classify_all("tokio_postgres", p), Some(vec!["Db", "Net", "Ipc", "Rand"]), "{p}");
    }
    for p in ["postgres::Config::connect", "postgres::Client::connect"] {
        assert_eq!(classify_all("postgres", p), Some(vec!["Db", "Net", "Ipc", "Rand"]), "{p}");
    }
    // `connect_raw` runs the handshake over the caller's stream: it dials nothing (not claimed). `TlsConnect::connect` upgrades a
    // stream already open. Neither is `tokio_postgres::connect`, which an `Exact` row must not reach by suffix.
    assert!(also_effects("tokio_postgres", "tokio_postgres::Config::connect_raw").is_empty());
    assert!(also_effects("tokio_postgres", "tokio_postgres::tls::TlsConnect::connect").is_empty());
    assert!(also_effects("tokio_postgres", "tokio_postgres::Config::host").is_empty());
}

#[test]
fn fastrand_and_tempfile_carry_the_first_use_clock_seed() {
    for p in ["fastrand::u32", "fastrand::seed", "fastrand::get_seed", "fastrand::alphabetic", "fastrand::global_rng::shuffle"] {
        assert_eq!(classify_all("fastrand", p), Some(vec!["Rand", "Clock"]), "{p}");
    }
    for p in ["fastrand::Rng::new", "fastrand::Rng::default"] {
        assert_eq!(classify_all("fastrand", p), Some(vec!["Rand", "Clock"]), "{p}");
    }
    // EXECUTED 0 clock reads: a seeded generator and an instance draw never touch the thread-local seed.
    assert!(also_effects("fastrand", "fastrand::Rng::u32").is_empty(), "an instance draw is not the free fn of the same name");
    assert!(also_effects("fastrand", "fastrand::Rng::usize").is_empty());
    assert!(classify("fastrand", "fastrand::Rng::with_seed").is_none());
    for p in ["tempfile::NamedTempFile::new", "tempfile::NamedTempFile::with_suffix_in", "tempfile::TempDir::new", "tempfile::TempDir::with_prefix",
              "tempfile::Builder::tempfile", "tempfile::Builder::make_in", "tempfile::tempfile", "tempfile::dir::tempdir_in"] {
        assert_eq!(classify_all("tempfile", p), Some(vec!["Fs", "Clock"]), "{p}");
    }
    // EXECUTED 0 clock reads: no name is generated.
    for p in ["tempfile::NamedTempFile::close", "tempfile::NamedTempFile::persist", "tempfile::NamedTempFile::into_file", "tempfile::TempDir::keep"] {
        assert!(!also_effects("tempfile", p).contains(&"Clock"), "{p}");
    }
}

#[test]
fn exact_and_free_fn_matchers_are_strict() {
    let exact = ALSO_EFFECTS.iter().find(|r| matches!(r.on, AlsoOn::Exact(_))).expect("an Exact row");
    assert!(exact.on.matches("tokio_postgres::connect") && !exact.on.matches("tokio_postgres::Config::connect"));
    let free = ALSO_EFFECTS.iter().find(|r| matches!(r.on, AlsoOn::FreeFn)).expect("a FreeFn row");
    assert!(free.on.matches("fastrand::u32") && free.on.matches("fastrand::global_rng::u32"));
    assert!(!free.on.matches("fastrand::Rng::u32"));
    assert!(!free.on.matches("fastrand::&Rng::eq"), "a trait-impl spelling is not a free fn (found in the corpus A/B)");
}
