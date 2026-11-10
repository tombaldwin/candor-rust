//! SOUNDNESS R1612 — a call's SECOND effects (`ALSO_EFFECTS`), charged beside its primary `classify` answer.
//!
//! Each positive is EXECUTED under a DYLD interposer (macOS) with the second effect observed inside the call window; each
//! pure neighbour is the same type's idle window (0 events). The pure neighbours matter more than usual: a row keyed on
//! the type alone would pass the positives.

use candor_classify::{also_effects, also_fs_kind, also_hides_locator, classify, classify_all, classify_rendered, ALSO_EFFECTS};

#[test]
fn r1599_four_rule_rows_carry_their_second_effect() {
    // backoff: entropy (primary) AND the clock (the max_elapsed_time check). EXECUTED 1 getentropy + 1 clock_gettime.
    for p in ["backoff::ExponentialBackoff::next_backoff", "backoff::exponential::ExponentialBackoff::next_backoff"] {
        assert_eq!(classify_all("backoff", p), Some(vec!["Rand", "Clock"]), "{p}");
    }
    // grep_cli: PATH (primary) AND a stat per PATH directory. EXECUTED getenv(PATH) + stat(/usr/bin/gzip).
    for m in ["associate", "try_associate"] {
        let p = format!("grep_cli::DecompressionMatcherBuilder::{m}");
        assert_eq!(classify_all("grep_cli", &p), Some(vec!["Env", "Fs"]), "{p}");
    }
    // time: the clock (primary) AND the zone database. EXECUTED (TZ unset) clock_gettime + access(/etc/localtime).
    assert_eq!(
        classify_all("time", "time::utc_offset::UtcOffset::current_local_offset"),
        Some(vec!["Clock", "Fs"])
    );
    // portable_pty: the environment snapshot (primary) AND the passwd shell's access(2) when SHELL is unset.
    for m in ["new", "from_argv", "new_default_prog"] {
        let p = format!("portable_pty::CommandBuilder::{m}");
        assert_eq!(classify_all("portable_pty", &p), Some(vec!["Env", "Fs"]), "{p}");
        assert_eq!(classify_rendered("portable_pty", &p).as_deref(), Some("Env+Fs"));
    }
}

#[test]
fn pure_neighbours_and_lookalikes_get_no_second_effect() {
    // Same types, idle windows (EXECUTED 0 events).
    assert!(also_effects("backoff", "backoff::ExponentialBackoffBuilder::new").is_empty());
    assert!(also_effects("grep_cli", "grep_cli::DecompressionMatcherBuilder::new").is_empty());
    assert!(also_effects("time", "time::UtcOffset::from_hms").is_empty());
    assert!(also_effects("portable_pty", "portable_pty::CommandBuilder::arg").is_empty());
    // The primary's neighbours that ARE charged keep exactly one effect: a second effect is per method, not per type.
    assert_eq!(classify_all("backoff", "backoff::ExponentialBackoff::reset"), Some(vec!["Clock"]));
    // A different crate with the same type and method names is not claimed.
    assert!(also_effects("tokio_retry", "tokio_retry::ExponentialBackoff::next_backoff").is_empty());
    assert!(also_effects("my_pty", "my_pty::CommandBuilder::new").is_empty());
    // `Leaf` rows need a whole segment: `fooclient_tls` is not `client_tls`.
    assert!(also_effects("tungstenite", "tungstenite::fooclient_tls").is_empty());
}

#[test]
fn a_second_effect_never_replaces_or_stands_without_its_primary() {
    for row in ALSO_EFFECTS {
        assert!(!row.row.is_empty() && !row.evidence.is_empty(), "every row names its evidence");
        for e in row.effects {
            assert!(candor_report::EFFECTS.contains(e) && *e != "Unknown", "{} names a §1 effect, not {e}", row.row);
        }
    }
    // A path `classify` does not charge gets no second effect, whatever the table says (the κ ledger's
    // "was this call classified" stays the primary's question).
    assert_eq!(classify("tungstenite", "tungstenite::connect"), Some("Net"));
    assert!(also_effects("tungstenite", "tungstenite::connect").is_empty(), "R338's stated residual");
    for p in ["tungstenite::client_tls", "tungstenite::client_tls_with_config"] {
        assert_eq!(classify_all("tungstenite", p), Some(vec!["Net", "Fs"]), "{p}: added, never substituted");
    }
}

#[test]
fn locator_bearing_second_effects_are_marked_incomplete() {
    for e in ["Fs", "Net", "Llm", "Exec", "Db"] {
        assert!(also_hides_locator(e), "{e} has a §2 surface the call's literal does not name");
    }
    for e in ["Env", "Clock", "Rand", "Log", "Ipc", "Clipboard"] {
        assert!(!also_hides_locator(e), "{e} has no surface");
    }
}

#[test]
fn a_second_fs_declares_its_direction_from_the_executed_window() {
    // Every Fs row's window showed only stat/access/read-only opens, so each declares `read`; none may claim more.
    for (c, p) in [
        ("grep_cli", "grep_cli::DecompressionMatcherBuilder::associate"),
        ("time", "time::UtcOffset::current_local_offset"),
        ("portable_pty", "portable_pty::CommandBuilder::new"),
        ("tungstenite", "tungstenite::client_tls"),
        ("elasticsearch", "elasticsearch::Transport::single_node"),
    ] {
        assert_eq!(also_fs_kind(c, p), vec!["read"], "{p}");
    }
    assert!(also_fs_kind("backoff", "backoff::ExponentialBackoff::next_backoff").is_empty(), "no Fs, no direction");
}
