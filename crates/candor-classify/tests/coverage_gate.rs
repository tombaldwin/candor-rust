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
//! open.tsv` — NOT asserted here (asserting it would just make every future `cargo test` fail on a
//! pre-existing backlog); it is enforced separately, as a "may shrink, must never grow" ratchet, by the
//! weekly `coverage-gate-refresh` workflow re-running the generator against fresh crates.io sources.
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
