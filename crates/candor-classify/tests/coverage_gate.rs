//! THE COMPLETENESS GATE for `CALIBRATED_CRATES` (see `eval/coverage-gate/generate.py` for the full
//! design, and REPORT — the commit that added this file — for the measurement behind it).
//!
//! THE PROBLEM this closes: `crates/candor-scan/src/scan.rs`'s coverage ledger is deliberately
//! crate-level — once a crate is in `CALIBRATED_CRATES`, an unmatched path is a CLAIM of reviewed
//! purity, with no `coverage.uncovered` disclosure. Calibrating a crate converts "no rule" into
//! "checked, and it's pure". Nothing previously enforced that the verb table backing that claim was
//! actually complete; ten real, silent under-reports (`ignore::Walk::new`, `diesel::Connection::
//! establish`, five rusqlite constructors, `git2::Submodule::clone`/`::update`, `mongodb::Client::
//! with_options`, `mysql`/`mysql_async::Conn::new`, `sea_orm::Database::connect_proxy`,
//! `tokio_postgres::Config::connect_raw`, nine tungstenite handshake functions, `aws_config::
//! load_from_env`) shipped from exactly this gap before commit 19ce144 closed the ten known instances.
//! This test closes the GENERATOR, not another instance: it is a differential between what
//! `candor-scan` finds by REAL local call-graph analysis when it self-scans a calibrated crate's own
//! vendored source (an oracle independent of whether the entry point itself has a top-level rule — see
//! generate.py's header for the empirical proof) and what `classify()` recognizes for the SAME entry
//! point spelled the way an external consumer would call it. `covered.tsv` is every point where the two
//! agreed at generation time; this test is the promise that agreement doesn't quietly regress.
//!
//! THE GATE MUST BE ABLE TO FAIL. Proof (see the commit): removing the `ignore::Walk::new`/
//! `ignore::Walk::from_iter` rule from `classify()` makes this test fail, printing exactly:
//!   ignore::Walk::new  (was Env,Fs,Log at generation time)
//!   ignore::Walk::from_iter is not itself in covered.tsv (self-scan found it only `invisible`, not a
//!   confirmed effect — see generate.py's header on why a bare `invisible` doesn't qualify), so only
//!   `Walk::new` regresses — restoring the rule turns the test green again.
//!
//! **THE EQUALITY FIX (this commit).** The gate above only ever asserted `classify(krate, path).
//! is_some()` — presence, not agreement. A rule NARROWED to a still-non-`None` but WRONG effect (e.g.
//! `async_nats::connect`'s `Some("Net")` changed to `Some("Log")`) passed this gate silently: a `deny
//! Net` policy would then wave through code opening a NATS connection, which is exactly the class this
//! gate exists to stop. Column 3 (`self_scan_effects`, formerly this file's only "effects" column)
//! CANNOT be used as the equality target — it is a DIFFERENT oracle's superset, not classify()'s own
//! answer: self-scan's real call-graph walk of `async_nats::connect` legitimately finds `Fs,Log,Net,
//! Rand,Unknown` reachable (auth touches a creds file, connecting logs, etc.), so `Log` was ALREADY a
//! member of that set before this fix existed — a membership check against column 3 would have missed
//! the reviewer's exact Net->Log mutation too. It would also have gone the other way: self-scan's set
//! for `async_nats::Consumer::request_batch` is `Log` alone while classify() correctly returns `Net`
//! for it, so membership-testing column 3 flags today's genuinely-correct row as a false regression.
//! Column 4 (`classified_as`) instead records what `classify()` ITSELF returned for that exact
//! consumer_path at generation time, and this test now asserts EXACT equality against it.
//!
//! THIS IS A HARD GATE ON EVERY ROW IN `covered.tsv`, not a ratchet: every one of them is something
//! `classify()` ALREADY recognizes today, so the only way this test fails is a REGRESSION (a rule
//! narrowed, retargeted to a different effect, or removed). The RATCHET of entries self-scan found
//! effectful that no rule (or `REVIEWED_PURE_ENTRIES`) recognizes yet lives in `eval/coverage-gate/
//! open.tsv` — its EFFECTS are not asserted here (that would make every `cargo test` fail on a
//! pre-existing backlog); growth is caught by the weekly `coverage-gate-refresh` workflow re-running the
//! generator against fresh crates.io sources. What IS asserted here, since SOUNDNESS R1528 step 3, is
//! that every open.tsv row is either pre-ruling backlog (`grandfathered.tsv`) or carries a recorded
//! verdict (`OPEN_ENTRY_VERDICTS`) — see the second half of this file.
//!
//! `REVIEWED_PURE_ENTRIES` (crates/candor-classify/src/lib.rs, beside `REVIEWED_PURE_CRATES`) is the
//! escape hatch for an `open.tsv` row a human has since read and found genuinely pure: add it there
//! (with the SAME evidence bar `REVIEWED_PURE_CRATES` documents) and the refresh workflow's ratchet
//! stops counting it, without inventing a classify() rule for something that performs no effect. It
//! doubles as the escape hatch here for a `covered.tsv` row whose classify() rule a human later removed
//! in favor of a `REVIEWED_PURE_ENTRIES` verdict, without regenerating the (now-stale) manifest row.

use candor_classify::{classify, REVIEWED_PURE_ENTRIES};

#[test]
fn calibrated_crates_completeness_gate() {
    let manifest = include_str!("../../../eval/coverage-gate/covered.tsv");
    let mut failures = Vec::new();
    let mut checked = 0usize;

    for line in manifest.lines() {
        if line.starts_with('#') || line.trim().is_empty() {
            continue;
        }
        let mut cols = line.split('\t');
        let (Some(krate), Some(path), Some(self_scan_effects), Some(classified_as)) =
            (cols.next(), cols.next(), cols.next(), cols.next())
        else {
            panic!("malformed covered.tsv row (expected 4 tab-separated columns): {line:?}");
        };
        checked += 1;
        let actual = classify(krate, path);
        let still_agrees = actual == Some(classified_as) || REVIEWED_PURE_ENTRIES.contains(&(krate, path));
        if !still_agrees {
            let now = match actual {
                Some(eff) => format!(
                    "now classify() -> Some({eff:?}), not the recorded {classified_as:?} — a rule was \
                     NARROWED to a different effect, not just removed"
                ),
                None => format!(
                    "now classify() -> None (recorded as {classified_as:?} at generation time), and not \
                     in REVIEWED_PURE_ENTRIES"
                ),
            };
            failures.push(format!(
                "{krate}\t{path}\t{now} (self-scan found {self_scan_effects} reachable at generation time)"
            ));
        }
    }

    // A near-empty manifest would make this test pass vacuously — assert it actually loaded real rows,
    // so a path/build mistake (e.g. include_str!'s relative path silently resolving to an empty file)
    // fails loudly instead of the gate quietly asserting nothing.
    assert!(
        checked > 500,
        "expected several hundred rows from eval/coverage-gate/covered.tsv, got {checked} — \
         the manifest failed to load (check the include_str! path) or was truncated"
    );

    assert!(
        failures.is_empty(),
        "{} of {checked} previously-covered CALIBRATED_CRATES entry point(s) no longer classify to the \
         SAME effect they were recorded with (a rule was narrowed, retargeted, or removed — or a crate \
         version bump in the checked-in manifest no longer matches reality; regenerate via `python3 \
         eval/coverage-gate/generate.py` and review the diff). Each of these WAS reachable to a real \
         effect via real local call-graph analysis of the crate's own source (candor-scan self-scanning \
         it, not a guess) at generation time, and classify() agreed on a SPECIFIC effect for it then:\n{}",
        failures.len(),
        failures.join("\n")
    );
}

// ---------------------------------------------------------------------------------------------------
// SOUNDNESS R1528 step 3 — EVERY `open.tsv` ROW CARRIES A VERDICT, OR IS PRE-RULING BACKLOG.
//
// The doc comment above says the ratchet is "NOT asserted here" and is enforced weekly. That was the
// gap: the weekly job fails on a grown row, and the red was cleared by SPLICING the row into
// `open.tsv` — 108 rows since 10-08, with no rule, no read and no filed row — after which nothing
// anywhere distinguished a triaged row from a copied one. Tom's ruling (2026-11-08): a newly grown
// row clears the red only with a recorded verdict. The rows already in `open.tsv` at the ruling are
// `grandfathered.tsv`, tracked as a population by R1528 and allowed only to shrink; every OTHER row
// must be in `OPEN_ENTRY_VERDICTS` (a filed row) — a rule or a `REVIEWED_PURE_ENTRIES` read take the
// row out of `open.tsv` on regeneration, so they need no record here.
// ---------------------------------------------------------------------------------------------------

use candor_classify::{CALIBRATED_CRATES, OPEN_ENTRY_VERDICTS};
use std::collections::{BTreeSet, HashMap};

/// The size of `grandfathered.tsv` when the ruling was implemented (the checked-in `open.tsv`, 776
/// rows, minus the 64 R1528-step-2 found UNREACHABLE; 694 after the 18 reachable rusqlite rows got a classify
/// rule, R1528; 619 after the 75 ureq 3.4.2 rows got a rule, a read or a verdict; 550 after the 69 chrono / tracing-subscriber / jiff rows did; 474 after the 76 sea_orm 2.0.4 rows did; 369 after the 105 async_nats 0.50.0 rows did; 300 after the 69 rdkafka 0.39.0 rows did; 218 after the 44 mongodb 3.9.1 and 38 mysql_async 0.37.1 rows did; 152 after the 23 aws_config 1.12.0, 22 sqlx_core 0.9.0 and 21 redis 1.7.1 rows did). It may go DOWN — lower this number when it
/// does, so the slack cannot be spent — and must never go up: a row added to the backlog is a verdict
/// skipped. Raising it is visible in review, which is the point.
const GRANDFATHERED_CEILING: usize = 152;

/// (crate, entry) of every data row of an `open.tsv`-shaped manifest (crate first, entry last).
fn manifest_keys(text: &str) -> Vec<(String, String)> {
    text.lines()
        .filter(|l| !l.starts_with('#') && !l.trim().is_empty())
        .map(|l| {
            let cols: Vec<&str> = l.split('\t').collect();
            assert!(cols.len() >= 2, "malformed manifest row: {l:?}");
            (cols[0].to_string(), cols[cols.len() - 1].to_string())
        })
        .collect()
}

/// The verdict-register check, as a pure function of its three inputs so the test below can prove it
/// FAILS on a seeded grown row (the vacuity rule: a check never shown to fail is not evidence).
fn verdict_problems(
    open: &[(String, String)],
    grandfathered: &[(String, String)],
    verdicts: &[(&str, &str, &str)],
) -> Vec<String> {
    let mut problems = Vec::new();
    let open_set: BTreeSet<&(String, String)> = open.iter().collect();
    let gf: BTreeSet<(&str, &str)> = grandfathered.iter().map(|(c, e)| (c.as_str(), e.as_str())).collect();
    let mut seen: HashMap<(&str, &str), &str> = HashMap::new();
    for &(krate, entry, verdict) in verdicts {
        if seen.insert((krate, entry), verdict).is_some() {
            problems.push(format!("duplicate verdict for {krate}\t{entry}"));
        }
        if !CALIBRATED_CRATES.contains(&krate) {
            problems.push(format!("verdict names {krate:?}, which is not a CALIBRATED_CRATES entry"));
        }
        let well_formed = verdict
            .split_once(':')
            .is_some_and(|(row, kind)| {
                row.len() > 1
                    && row.starts_with('R')
                    && row[1..].chars().all(|c| c.is_ascii_digit())
                    && matches!(kind, "silent" | "overcharge" | "conditional")
            });
        if !well_formed {
            problems.push(format!(
                "{krate}\t{entry}: verdict {verdict:?} is not `R<id>:silent|overcharge|conditional`"
            ));
        }
        if !open_set.contains(&(krate.to_string(), entry.to_string())) {
            problems.push(format!(
                "{krate}\t{entry}: has a verdict but is not in open.tsv — drop the stale verdict (a rule or a \
                 REVIEWED_PURE_ENTRIES read already took the row out)"
            ));
        }
    }
    for (krate, entry) in grandfathered {
        if !open_set.contains(&(krate.clone(), entry.clone())) {
            problems.push(format!(
                "{krate}\t{entry}: is in grandfathered.tsv but no longer in open.tsv — remove it from \
                 grandfathered.tsv (and lower GRANDFATHERED_CEILING), or a later regrowth would clear unread"
            ));
        }
    }
    for (krate, entry) in open {
        if !gf.contains(&(krate.as_str(), entry.as_str())) && !seen.contains_key(&(krate.as_str(), entry.as_str())) {
            problems.push(format!(
                "{krate}\t{entry}: is in open.tsv with NO VERDICT. A row added since the ruling needs one of: \
                 a classify() rule (the row then moves to covered.tsv), a REVIEWED_PURE_ENTRIES read, or a \
                 filed SOUNDNESS row recorded in OPEN_ENTRY_VERDICTS as `R<id>:silent|overcharge|conditional`. \
                 Splicing the row in is not a verdict."
            ));
        }
    }
    problems
}

#[test]
fn every_open_row_has_a_verdict_or_is_pre_ruling_backlog() {
    let open = manifest_keys(include_str!("../../../eval/coverage-gate/open.tsv"));
    let grandfathered = manifest_keys(include_str!("../../../eval/coverage-gate/grandfathered.tsv"));
    assert!(open.len() > 300, "open.tsv loaded {} rows — truncated or the include_str! path moved", open.len());
    assert!(
        grandfathered.len() <= GRANDFATHERED_CEILING,
        "grandfathered.tsv has {} rows, above its ceiling {GRANDFATHERED_CEILING}: the pre-ruling backlog may \
         only shrink. A new row needs a verdict (see OPEN_ENTRY_VERDICTS), not a backlog slot.",
        grandfathered.len()
    );
    let problems = verdict_problems(&open, &grandfathered, OPEN_ENTRY_VERDICTS);
    assert!(problems.is_empty(), "coverage-gate verdict register:\n  {}", problems.join("\n  "));
}

/// THE CHECK CAN FAIL — seeded, in both directions, on the real manifests. A row spliced into open.tsv
/// with no verdict must be named; the same row with a well-formed verdict must clear; a malformed or
/// stale verdict must be named. Without this, a register that loaded nothing would pass.
#[test]
fn the_verdict_check_fails_on_a_seeded_unverdicted_row() {
    let mut open = manifest_keys(include_str!("../../../eval/coverage-gate/open.tsv"));
    let grandfathered = manifest_keys(include_str!("../../../eval/coverage-gate/grandfathered.tsv"));
    let seeded = ("chrono".to_string(), "chrono::seeded::Grown::row".to_string());
    open.push(seeded.clone());
    let red = verdict_problems(&open, &grandfathered, OPEN_ENTRY_VERDICTS);
    assert!(
        red.iter().any(|p| p.contains("chrono::seeded::Grown::row") && p.contains("NO VERDICT")),
        "a spliced row with no verdict was NOT reported: {red:?}"
    );
    let with: Vec<(&str, &str, &str)> = OPEN_ENTRY_VERDICTS
        .iter()
        .copied()
        .chain([("chrono", "chrono::seeded::Grown::row", "R1528:silent")])
        .collect();
    assert!(verdict_problems(&open, &grandfathered, &with).is_empty(), "a verdicted row must clear");
    let bad: Vec<(&str, &str, &str)> = vec![
        ("chrono", "chrono::seeded::Grown::row", "looked at it, fine"),
        ("chrono", "chrono::not::in::open", "R1528:silent"),
    ];
    let named = verdict_problems(&open, &grandfathered, &bad);
    assert!(named.iter().any(|p| p.contains("is not `R<id>")), "a prose verdict must be refused: {named:?}");
    assert!(named.iter().any(|p| p.contains("not in open.tsv")), "a stale verdict must be refused: {named:?}");
}

/// SOUNDNESS R1528 — THE `"0.39"` vs `"0.39.0"` COMPARATOR, FIXED NOW THAT THE ROWS HAVE RULES.
///
/// `calibration_exceeded` used to fall through to `resolved != ceiling` when the numeric parts tie, so the
/// two-component ceiling `("rusqlite", "0.39")` read the very release it was reviewed against (`0.39.0`) as
/// BEYOND review: an unmatched rusqlite call rejoined the coverage ledger and the consumer carried
/// `invisible: [rusqlite]`. That over-disclosure was the only thing disclosing the 18 reachable rusqlite rows
/// that had no rule; fixing it first would have re-granted the exemption over them (a consumer calling
/// `query_row_and_then` dropped out of `functions[]`). The rows now classify `Db`, `open.tsv` holds none, and
/// trailing zero components are padding. A pre-release/build suffix still means "not the reviewed artifact".
/// This pins the corrected answers (it FAILS on the old comparator: `0.39.0` read as exceeded) and the
/// precondition that made the fix safe.
#[test]
fn ceiling_comparator_treats_trailing_zeros_as_padding_and_open_rusqlite_stays_empty() {
    use candor_classify::calibration_exceeded as ex;
    // The defect itself: the reviewed release, spelled with the padding cargo always writes.
    assert!(!ex("rusqlite", "0.39.0"), "0.39.0 is the release the two-component ceiling 0.39 was reviewed against");
    assert!(!ex("rusqlite", "0.39"));
    // Everything that really is beyond review still is.
    assert!(ex("rusqlite", "0.39.1"));
    assert!(ex("rusqlite", "0.40.0"));
    assert!(ex("rusqlite", "0.39.0-rc.1"), "a pre-release is not the reviewed artifact");
    assert!(ex("rusqlite", "0.39.0+build.7"));
    assert!(!ex("rusqlite", "0.38.9"));
    // Three-component ceilings answer as before, including a ceiling that is itself a pre-release.
    assert!(!ex("tempfile", "3.27.0") && ex("tempfile", "3.27.1") && ex("tempfile", "3.28.0"));
    assert!(!ex("elasticsearch", "8.19.0-alpha.1"));
    assert!(ex("elasticsearch", "8.19.0"), "the release is not the alpha that was reviewed");
    assert!(ex("elasticsearch", "8.19.0-alpha.2"));
    assert!(!ex("not_a_calibrated_crate", "9.9.9"));
    // The precondition: the exemption is only safe over a crate whose reachable rows all have a rule or a
    // verdict. A rusqlite row that regrows into `open.tsv` must be resolved BEFORE it reaches a consumer.
    let open = include_str!("../../../eval/coverage-gate/open.tsv");
    let rusqlite_open = open.lines().filter(|l| l.starts_with("rusqlite\t")).count();
    assert_eq!(
        rusqlite_open, 0,
        "a rusqlite row is back in open.tsv while rusqlite 0.39.0 holds the calibration exemption: the \
         consumer is told nothing about it (SOUNDNESS R1528). Give it a rule or a verdict."
    );
}
