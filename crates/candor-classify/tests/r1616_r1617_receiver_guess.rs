//! SOUNDNESS R1616 (provided member) and R1617(4) — the classifier's half of two receiver-typing rows.
//!
//! R1616: `TimeZone`'s PROVIDED members on chrono's `Local` call the `offset_from_*` lookups `Local` implements. EXECUTED
//! (rustagent-recv `fx/c`, DYLD interposer), each name on a valid input: 2 clock reads + getenv(TZ) + open(/etc/localtime);
//! `Utc` and `FixedOffset`, the same members, 0 events.
//!
//! R1617(4): `also_effects_by_method` is the union the scan charges for a dependency builder-chain call whose step type it
//! could only GUESS (the chain's root). It must name every row's method on any type of that crate, and nothing else.

use candor_classify::{also_effects, also_effects_by_method, classify};

#[test]
fn r1616_local_provided_timezone_members_read_the_clock() {
    for m in ["from_utc_datetime", "from_utc_date", "from_local_datetime", "from_local_date", "with_ymd_and_hms", "ymd",
              "ymd_opt", "yo", "yo_opt", "isoywd", "isoywd_opt", "timestamp", "timestamp_opt", "timestamp_millis",
              "timestamp_millis_opt", "timestamp_nanos", "timestamp_micros", "datetime_from_str"] {
        assert_eq!(classify("chrono", &format!("chrono::Local::{m}")), Some("Clock"), "{m}");
        assert_eq!(classify("chrono", &format!("chrono::offset::Local::{m}")), Some("Clock"), "{m}");
        for ty in ["Utc", "FixedOffset", "offset::Utc", "NaiveDateTime"] {
            assert_eq!(classify("chrono", &format!("chrono::{ty}::{m}")), None, "{ty}::{m}");
        }
        assert_eq!(classify("mycrate", &format!("mycrate::Local::{m}")), None, "{m}: another crate's Local");
    }
    // neighbours that are not provided members reaching a lookup
    for m in ["from_offset", "now_or_never", "fix", "name"] {
        assert_eq!(classify("chrono", &format!("chrono::Local::{m}")), None, "{m}");
    }
}

#[test]
fn r1617_the_by_method_union_is_the_crates_rows_for_that_method_on_any_type() {
    // backoff's row is keyed `ExponentialBackoff::next_backoff` (+Clock): a guessed root type still names it by method
    assert_eq!(also_effects_by_method("backoff", "next_backoff"), vec!["Clock"]);
    assert!(also_effects("backoff", "backoff::ExponentialBackoffBuilder::next_backoff").is_empty(), "the type-keyed lookup misses the guess");
    // nothing for a method no row names, another crate, or a free-function (`Leaf`) row
    assert!(also_effects_by_method("backoff", "reset").is_empty());
    assert!(also_effects_by_method("rand", "next_backoff").is_empty());
    assert!(also_effects_by_method("grep_cli", "new").is_empty());
}
