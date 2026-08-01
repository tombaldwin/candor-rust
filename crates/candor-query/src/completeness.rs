//! ⟨0.24⟩ **WHAT THE REPORTS UNDER A LOCATOR SAY THE PRODUCING SCAN COULD NOT SEE** (SPEC §2's
//! `unanalyzed` manifest), read for EVERY ADVISORY VERB — `whatif`, `unverified`, `fix-gate`, `fix`.
//!
//! **THE DEFECT** (SPEC §3.2 ⟨0.24⟩, candor-spec `0075987` then `ec1a441`). Over a report declaring
//! `unanalyzed`, an advisory verb answered `{"ok": true, …}`, exit 0, with a `✓` in prose and no
//! disclosure on ANY channel — while `candor-query gate --report` over the SAME bytes exits 2.
//!
//! **AND THE CLAUSE WAS FIRST SCOPED TO THE VERB ITS DEFECT WAS FOUND IN, WHICH IS THE REASON THIS
//! MODULE EXISTS.** `0075987` ruled it for `whatif`; this engine implemented it for `whatif`, inside
//! `whatif`'s own file, and `unverified`/`fix-gate`/`fix` contained not one occurrence of `incomplete`.
//! MEASURED here on the release build before the fix, on a report declaring one `unanalyzed` unit, NO
//! `Unknown` holes at all, and `deny Net app` that nothing violates:
//!
//! ```text
//!   gate --report        exit 2   ok:false  incomplete:true + manifest   ← correct
//!   unverified --strict  exit 0   {"ok": true, "unverified": []}
//!                        stdout:  "every function in a pure/deny layer is PROVABLY clean … ✓"
//!   fix-gate  --strict   exit 0   {"ok": true, "remedies": []}
//!                        stdout:  "no deny/pure boundary crossings in this report ✓"
//! ```
//!
//! "PROVABLY clean" over a report that declares source candor could not read. So the reading, the
//! document keys and the prose withdrawal live in ONE place, and a later sibling verb gets them by
//! calling this rather than by an author remembering the rule.
//!
//! **`ok` IS OMITTED, NOT SET TO FALSE**, and that distinction is the whole of the ruling. `ok: false`
//! on an advisory verb asserts *"a hole exists, here it is"* beside an empty array — a VIOLATION the
//! analysis never found, the fabrication mirror, and worse than the silence it replaces. So the field
//! goes away and `incomplete: true` + the manifest take its place: a consumer writing `if (r.ok)` gets a
//! falsy value and fails safe, one that looks further learns what was unread. Deliberately NOT the
//! refusal document's shape (`ok: false` + `refused: true`), where `ok: false` is *true* because the
//! gate did not pass — **a shape is copied for its reasoning, not for its familiarity**.
//!
//! **THE FINDINGS STILL SHIP.** A partial answer that says it is partial beats a refusal, and these are
//! the verbs consulted BEFORE an edit, where the alternative is the operator guessing.
//!
//! **AND THE DISCLOSURE MUST REACH EVERY CHANNEL THE VERB ANSWERS ON** (SPEC §3.2 `ec1a441`). This
//! engine built a mutant that kept the whole JSON fix and deleted only the printed human line, and it
//! survived the entire suite, because absence-asserts on `ok` cannot see the other channel. The prose
//! `✓` IS the prose `ok: true`; removing the JSON field while leaving that sentence standing MOVES the
//! false all-clear rather than removing it. [`ReportCompleteness::print_note`] is the human half and
//! every caller prints it BEFORE its verdict, because it qualifies the findings above as much as the
//! verdict below.

use crate::load::glob_reports;

/// A report whose completeness could not be established.
///
/// SPEC §2: *"a key that cannot be READ is corrupt input, never its empty value"* — and here the empty
/// value is exactly what licenses `ok`, so coercing it would convert corrupt input into the green
/// claim. The gate route REFUSES on both causes below (exit 2 — `strict!` in gate.rs for the key,
/// `hard_fail` for the file; the key arm measured 2026-07-28 on a report with the right shape and the
/// wrong field names). An advisory verb cannot refuse — a refusal sends the operator back to guessing,
/// which is the thing these verbs exist to replace — so it takes the same fail-safe posture through the
/// disclosure instead.
pub(crate) struct Unreadable {
    pub(crate) path: String,
    /// The file READ and its `unanalyzed` key is present-but-unparseable (as against: the file could
    /// not be read at all). Two different repairs, so two different sentences — "your `unanalyzed` key
    /// is not `[{path, reason}]`" is actionable where "this report did not load" sends the user to a
    /// scan they may not own.
    pub(crate) key_present: bool,
}

/// The manifest as far as it could be READ, unioned across the reports under a locator.
pub(crate) struct ReportCompleteness {
    pub(crate) unanalyzed: Vec<candor_report::UnanalyzedUnit>,
    pub(crate) unreadable: Vec<Unreadable>,
}

impl ReportCompleteness {
    /// Is the universe this verb reasoned over known-partial? Either arm suppresses `ok`.
    pub(crate) fn incomplete(&self) -> bool {
        !self.unanalyzed.is_empty() || !self.unreadable.is_empty()
    }

    /// How many units the reports say were not analysed — readable manifest entries plus files whose
    /// manifest could not be read at all.
    pub(crate) fn units(&self) -> usize {
        self.unanalyzed.len() + self.unreadable.len()
    }

    /// The stderr disclosure for a `unanalyzed` key that is present and unreadable. Named per file and
    /// actionable, for the reason gate.rs's `strict!` names the key rather than failing the whole load.
    pub(crate) fn warn_unreadable(&self, verb: &str) {
        for u in &self.unreadable {
            let p = &u.path;
            if u.key_present {
                eprintln!(
                    "candor {verb}: report {p} — the `unanalyzed` key is PRESENT but is not a list of \
                     `{{ path, reason }}` (SPEC §2). A key that cannot be READ is corrupt input, never \
                     its empty value, and here the empty value is what licenses `ok` — so this answer \
                     is reported INCOMPLETE. Fix the key, or re-run the scan that wrote it."
                );
            } else {
                eprintln!(
                    "candor {verb}: report {p} — could not be READ at all, so whether it declares \
                     unanalyzed source is unknown. `candor-query gate --report` refuses over this \
                     file, so this answer is reported INCOMPLETE rather than clean. Re-run the scan."
                );
            }
        }
    }

    /// The JSON half: `incomplete: true` + the manifest. The caller has ALREADY declined to write `ok`
    /// — this cannot remove a key it does not know the name of, and a caller that forgets would emit
    /// `ok` beside `incomplete`, which is the defect with a decoration.
    pub(crate) fn write_json(&self, out: &mut serde_json::Value) {
        if !self.incomplete() {
            return;
        }
        out["incomplete"] = serde_json::json!(true);
        if !self.unanalyzed.is_empty() {
            out["unanalyzed"] = serde_json::json!(self.unanalyzed);
        }
    }

    /// The HUMAN half — a no-op on a complete report, so an ordinary run stays byte-identical.
    ///
    /// `so_what` names what the reader must NOT read as complete and `tail` closes it, because the
    /// consequence differs per verb (`whatif` loses CALLERS from a blast radius; `unverified` cannot
    /// enumerate a function that is absent from `functions` at all) and a generic banner would be
    /// ignorable. The framing, the unit list and the fact that it is printed BEFORE the verdict are the
    /// parts that must not vary, so they are here.
    pub(crate) fn print_note(&self, so_what: &str, tail: &str) {
        let _ = self.write_note(&mut std::io::stdout(), so_what, tail);
    }

    /// [`Self::print_note`] on STDERR, for a verb whose stdout is a JSON document on this path — `fix`
    /// prints its "nothing to hoist" answer as prose in BOTH modes, so it needs the withdrawal in both,
    /// and prose written to stdout beside a document would corrupt the document.
    pub(crate) fn eprint_note(&self, so_what: &str, tail: &str) {
        let _ = self.write_note(&mut std::io::stderr(), so_what, tail);
    }

    /// ONE prose implementation, sink-parameterised. Two copies of this text is exactly how the family
    /// arrived at two element rules for the manifest reader (`93cef40`).
    fn write_note(&self, w: &mut dyn std::io::Write, so_what: &str, tail: &str) -> std::io::Result<()> {
        if !self.incomplete() {
            return Ok(());
        }
        writeln!(
            w,
            "  ⚠ INCOMPLETE — the report(s) under this locator declare {} unit(s) candor could not analyze,",
            self.units()
        )?;
        writeln!(w, "      so {so_what}:")?;
        for u in &self.unanalyzed {
            writeln!(w, "      {} — {}", u.path, u.reason)?;
        }
        for u in &self.unreadable {
            writeln!(w, "      {} — its `unanalyzed` manifest could not be read (see above)", u.path)?;
        }
        writeln!(w, "      {tail}")
    }
}

/// Read the ⟨0.21⟩ manifest off every report under `prefix`.
///
/// **AT LEAST AS PESSIMISTIC AS THE GATE, BY CONSTRUCTION** — SPEC §3.2 `93cef40`: *"whatever leniency
/// a reader applies, the advisory verb's incompleteness verdict must be at least as pessimistic as the
/// gate's over the same bytes."* candor-swift and candor-ts had implemented the reader twice with
/// different ELEMENT rules, and skipping an element makes the advisory verb read a SHORTER `unanalyzed`
/// list than the gate reads from the identical file. Here the relation is not maintained by agreement:
/// this is the SAME file set (`glob_reports`) and the SAME reader (`candor_report::report_unanalyzed`)
/// `load_gate_report` uses, and a malformed ELEMENT makes the whole key `Corrupt` in both — the gate
/// refuses, this reports incomplete. A file that cannot be READ is counted too, for the same relation:
/// the gate hard-fails on it, so an advisory verb that skipped it would answer clean over bytes the gate
/// declined to judge.
///
/// A locator matching NO report is NOT incomplete: an absent report is the ordinary pre-scan case every
/// caller already fails on before reaching here, and treating "no manifest" as "incomplete" would put
/// the hedge on every run and train the reader to ignore it — the same reason the vocabulary disclosure
/// is omitted when no alias was used.
pub(crate) fn report_completeness(prefix: &str) -> ReportCompleteness {
    let mut out = ReportCompleteness { unanalyzed: Vec::new(), unreadable: Vec::new() };
    for path in glob_reports(prefix) {
        let p = path.display().to_string();
        let Ok(text) = std::fs::read_to_string(&path) else {
            out.unreadable.push(Unreadable { path: p, key_present: false });
            continue;
        };
        match candor_report::report_unanalyzed(&text) {
            candor_report::KeyRead::Present(u) => out.unanalyzed.extend(u),
            candor_report::KeyRead::Absent => {}
            candor_report::KeyRead::Corrupt => {
                out.unreadable.push(Unreadable { path: p, key_present: true })
            }
        }
    }
    out
}
