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
//!
//! ⟨0.28⟩ **AND "EVERY ADVISORY VERB" WAS ITSELF THE SCOPING MISTAKE THIS MODULE WAS WRITTEN TO STOP
//! HAPPENING AGAIN.** The header above says this is read for every ADVISORY verb, and the DESCRIPTIVE
//! verbs — the ones that answer a question rather than render a verdict — were never added. SPEC §2
//! ⟨0.28⟩ corrects the clause to the condition that makes it true: the obligation binds *"any verb whose
//! output could be read as a NEGATIVE FINDING about the code — a verdict, an empty result set, or a zero
//! count"*. An empty result set is exactly what these verbs produce. MEASURED on the release build over
//! a report declaring `analyzed.count: 0` and a non-empty `unanalyzed` — the standard post-failure
//! artifact — every one of them answered flat, at exit 0, with no hedge on either channel:
//!
//! ```text
//!   blindspots   {"sources":[],"totalUnknown":0}      ← "no blind spots", over a report whose own
//!   containment  {"ambient":{},"contained":[]}          manifest names a file it could not read
//!   reachable    {"effects":{},"entryPoints":0}
//!   map          {}
//!   tour         {"reaches":[]}
//!   where Fs     {"directly":[],"inherited":[]}
//! ```
//!
//! A consumer cannot tell *nobody performs `Fs`* from *nothing was examined*. Same module, same two
//! channels, same no-op-when-complete rule — [`ReportCompleteness::must_hedge`] is the trigger a
//! descriptive verb asks, because its answer is not a verdict and `incomplete()` alone is the wrong
//! question (see that method).
//!
//! ⟨0.28⟩ **AND `analyzed.count: 0` IS THE SECOND CAUSE, WHICH THIS MODULE DID NOT READ AT ALL.** SPEC
//! §2: *"a report-consuming verb MUST re-disclose a non-empty `unanalyzed`, **and an `analyzed.count` of
//! 0**, on the same terms."* A report that judged nothing carries no `unanalyzed` — there is no unread
//! FILE to name, the scan simply reached no conclusion — so the manifest reader saw a complete report
//! and the six verbs above answered `{}` over it just the same. [`ReportCompleteness::judged_nothing`]
//! is that arm, kept OUT of `incomplete()` on purpose (below), because ⟨0.24⟩ fixes count-0's exit code
//! and `incomplete()` is what two verbs compute theirs from.

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

/// The ⟨0.28⟩ disclosure keys, for `#[serde(flatten)]` into a verb's own document struct — see
/// [`ReportCompleteness::fields`], which is the only thing that builds one. Constructed ONLY when there
/// is something to disclose, so `flatten`ing it is still a no-op on a complete report (`incomplete` is
/// unconditional here precisely because the struct itself is the `Option`).
#[derive(serde::Serialize)]
pub(crate) struct CompletenessFields {
    pub(crate) incomplete: bool,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub(crate) unanalyzed: Vec<candor_report::UnanalyzedUnit>,
    #[serde(rename = "judgedNothing", skip_serializing_if = "Vec::is_empty")]
    pub(crate) judged_nothing: Vec<String>,
}

/// The manifest as far as it could be READ, unioned across the reports under a locator.
pub(crate) struct ReportCompleteness {
    pub(crate) unanalyzed: Vec<candor_report::UnanalyzedUnit>,
    pub(crate) unreadable: Vec<Unreadable>,
    /// ⟨0.28⟩ The reports under this locator that say they **JUDGED NOTHING** — SPEC §2's
    /// `analyzed.count == 0` row, decided per file by [`candor_report::report_judged_nothing`] (the same
    /// predicate `gate --report` and candor-scan's chained join use, so it cannot drift between them).
    ///
    /// A THIRD CAUSE, NOT A THIRD SPELLING OF THE FIRST. `unanalyzed` names source the scan could not
    /// READ; this is a scan that read whatever it read and reached no conclusion about any of it, so
    /// there is no file to name and the manifest is legitimately absent. Both make an empty answer
    /// unsupportable, and only the union of the two covers the post-failure artifact (which carries
    /// both) and the facade/`pub use` report (which carries only this).
    pub(crate) judged_nothing: Vec<String>,
}

impl ReportCompleteness {
    /// Is the universe this verb reasoned over known-partial? Either arm suppresses `ok`.
    ///
    /// ⟨0.28⟩ **`judged_nothing` IS DELIBERATELY NOT AN ARM OF THIS PREDICATE**, and the reason is an
    /// exit code. `unverified --strict` and `fix-gate --strict` compute theirs from this method
    /// ([`crate::unverified::unverified_exit`], [`crate::fix::fix_gate_exit`]) and answer 2 —
    /// *"the gate refuses over these bytes, so do I"* — when it is true. But ⟨0.24⟩ ruled count-0 the
    /// other way for exactly those bytes: it is *"A DISCLOSURE, NOT AN EXIT CODE … the exit code and the
    /// verdict document are UNCHANGED"*, because `gate --report` exits 0 over a facade package and a
    /// verb that exited 2 there would claim it got LESS far than the gate on identical input — the
    /// mirror of the over-claim the strict exit exists to prevent. So the count-0 cause reaches the two
    /// DISCLOSURE channels via [`Self::must_hedge`] and stops at the exit code.
    pub(crate) fn incomplete(&self) -> bool {
        !self.unanalyzed.is_empty() || !self.unreadable.is_empty()
    }

    /// ⟨0.28⟩ **Is there anything at all to disclose — the trigger for an ANSWER, where
    /// [`Self::incomplete`] is the trigger for a VERDICT.**
    ///
    /// SPEC §2 ⟨0.28⟩ binds the re-disclosure to *"any verb whose output could be read as a negative
    /// finding about the code — a verdict, an empty result set, or a zero count"*, and adds
    /// `analyzed.count: 0` to `unanalyzed` as a cause. A descriptive verb asks THIS: its empty set is a
    /// negative finding under both causes, and it has no exit code for the distinction above to matter
    /// to. `write_json`/`print_note` are keyed on it too, so a caller cannot get the JSON half's trigger
    /// and the prose half's trigger to disagree — the mutant that survived the whole suite (`ec1a441`)
    /// was exactly one channel going quiet.
    pub(crate) fn must_hedge(&self) -> bool {
        self.incomplete() || !self.judged_nothing.is_empty()
    }

    /// How many units the reports say were not analysed — readable manifest entries plus files whose
    /// manifest could not be read at all.
    pub(crate) fn units(&self) -> usize {
        self.unanalyzed.len() + self.unreadable.len()
    }

    /// What `candor-query gate --report` does over THESE SAME BYTES, as one sentence for a caller's
    /// `tail` — and it is a method rather than a fixed string because the two causes get opposite
    /// answers, which the first draft of this rung got wrong in prose.
    ///
    /// Every pre-⟨0.28⟩ caller closes its note with *"`gate --report` exits 2 over these bytes"*, which
    /// is true of `unanalyzed`: §3.3 makes an incomplete analysis of the target's own code one of the
    /// gate's two exit-2 causes. It is FALSE of `analyzed.count: 0`. ⟨0.24⟩ ruled that one explicitly the
    /// other way — *"A DISCLOSURE, NOT AN EXIT CODE"* — so the gate exits 0 over a judged-nothing report
    /// and a note claiming otherwise sends the reader to a CI job that will pass and tell them this
    /// warning was noise. Which is worse than saying nothing: it is the disclosure discrediting itself.
    ///
    /// The count-0 sentence is the more urgent one anyway, and says so: nothing downstream fails closed
    /// on these bytes, so this note is the only thing standing between the reader and an empty answer.
    pub(crate) fn gate_line(&self) -> &'static str {
        if self.incomplete() {
            "`gate --report` exits 2 over these bytes."
        } else {
            "NOTHING DOWNSTREAM WILL CATCH THIS FOR YOU — `gate --report` exits 0 over a judged-nothing \
             report (⟨0.24⟩: a disclosure, not an exit code), so this note is the whole of the warning."
        }
    }

    /// Union in a SECOND locator's manifest, for a verb that reads two — `containment <baseline>`, whose
    /// answer is a DIFFERENCE and is therefore unsound if either side is partial, and in opposite
    /// directions: a leak living in an unread file of the CURRENT tree is missed (a false all-clear),
    /// while one living in an unread file of the BASELINE reads as newly appeared (a fabricated leak,
    /// at exit 1). One `ReportCompleteness` rather than two notes, because `write_json` writes fixed key
    /// names and calling it twice would have the second locator's manifest overwrite the first's.
    pub(crate) fn absorb(&mut self, other: ReportCompleteness) {
        self.unanalyzed.extend(other.unanalyzed);
        self.unreadable.extend(other.unreadable);
        self.judged_nothing.extend(other.judged_nothing);
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
        let Some(f) = self.fields() else { return };
        let serde_json::Value::Object(f) = serde_json::to_value(f).unwrap() else { return };
        for (k, v) in f {
            out[k] = v;
        }
    }

    /// The SAME key set as [`Self::write_json`], as a `#[serde(flatten)]`-able struct — for a verb whose
    /// document is a typed `Serialize` rather than a [`serde_json::Value`]. `None` on a complete report.
    ///
    /// **THIS EXISTS BECAUSE `to_value` IS NOT ORDER-PRESERVING AND THAT BROKE THE CONTROL.** The first
    /// version of this rung routed `where` and `blindspots` through
    /// `serde_json::to_value(struct)` so it could call `write_json`, and `serde_json::Map` is a
    /// `BTreeMap` — so an ORDINARY run over an INTACT report, where this module is supposed to be a
    /// no-op, came back re-sorted: `{effect, directly, inherited}` → `{directly, effect, inherited}`, and
    /// every `blindspots` source `{fn, why, reaches, affected}` → `{affected, fn, reaches, why}`.
    /// Measured by diffing both verbs' output over an intact report before and after; nothing else in the
    /// suite would have shown it, because every assertion on these documents reads keys by name. A
    /// disclosure rung that silently reformats the answers it is disclosing about has changed the thing
    /// it promised not to touch.
    ///
    /// So the key set is still defined ONCE, here, and `write_json` is now a caller of it — the two
    /// attachment styles cannot drift into two different manifests.
    pub(crate) fn fields(&self) -> Option<CompletenessFields> {
        if !self.must_hedge() {
            return None;
        }
        Some(CompletenessFields {
            incomplete: true,
            unanalyzed: self.unanalyzed.clone(),
            // ⟨0.28⟩ `incomplete: true` is the flag EITHER cause raises — a consumer that only branches
            // on it is safe under both — and this names WHICH reports judged nothing, because the two
            // causes want different repairs: `unanalyzed` wants a scan that can READ a file, this wants
            // a scan that reached a conclusion. Omitted when empty, so a document raised by `unanalyzed`
            // alone stays byte-identical to a pre-⟨0.28⟩ one.
            judged_nothing: self.judged_nothing.clone(),
        })
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

    /// [`Self::write_note`] against a caller-supplied sink, so a test can assert the human channel is
    /// silent on a complete report. That assertion cannot be made through `print_note`/`eprint_note`,
    /// and the mutant this module's header describes — the whole JSON fix kept, the printed line
    /// deleted — survived the entire suite precisely because nothing could see this channel.
    #[cfg(test)]
    pub(crate) fn write_note_for_test(&self, w: &mut dyn std::io::Write, so_what: &str, tail: &str) {
        let _ = self.write_note(w, so_what, tail);
    }

    /// ONE prose implementation, sink-parameterised. Two copies of this text is exactly how the family
    /// arrived at two element rules for the manifest reader (`93cef40`).
    fn write_note(&self, w: &mut dyn std::io::Write, so_what: &str, tail: &str) -> std::io::Result<()> {
        if !self.must_hedge() {
            return Ok(());
        }
        // ⟨0.28⟩ The unanalyzed-only sentence is UNCHANGED, character for character: that is the case
        // every existing caller was measured and reviewed on, and the count-0 arm is additive.
        let head = match (self.incomplete(), self.judged_nothing.len()) {
            (true, 0) => format!(
                "the report(s) under this locator declare {} unit(s) candor could not analyze,",
                self.units()
            ),
            (true, n) => format!(
                "the report(s) under this locator declare {} unit(s) candor could not analyze, and {n} \
                 report(s) that judged nothing at all,",
                self.units()
            ),
            (false, n) => format!(
                "{n} report(s) under this locator say they JUDGED NOTHING (`analyzed.count: 0`),"
            ),
        };
        writeln!(w, "  ⚠ INCOMPLETE — {head}")?;
        writeln!(w, "      so {so_what}:")?;
        for u in &self.unanalyzed {
            writeln!(w, "      {} — {}", u.path, u.reason)?;
        }
        for u in &self.unreadable {
            writeln!(w, "      {} — its `unanalyzed` manifest could not be read (see above)", u.path)?;
        }
        for p in &self.judged_nothing {
            writeln!(
                w,
                "      {p} — `analyzed.count: 0`: this report judged NOTHING, so it names no function \
                 at all and its silence is not a purity claim"
            )?;
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
    let mut out =
        ReportCompleteness { unanalyzed: Vec::new(), unreadable: Vec::new(), judged_nothing: Vec::new() };
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
                // NOTHING ELSE IS ASKED OF THIS FILE. `Corrupt` covers unparsable TEXT as well as an
                // unreadable `unanalyzed` key, and `report_judged_nothing` also fails closed on
                // unparsable text — so asking both would list one file twice, under two causes, for one
                // fault. The `unreadable` arm is the stronger and more actionable of the two here.
                out.unreadable.push(Unreadable { path: p, key_present: true });
                continue;
            }
        }
        // ⟨0.28⟩ PER FILE and via the SHARED predicate, for the reason gate.rs gives on its own copy of
        // this decision: a locator naming several members must disclose EACH silent one by name, and the
        // rule that decides "silent" is the one `gate --report` and candor-scan's chained join already
        // use, so a report cannot be judged-nothing on one route and not the other. Reading the file a
        // second time is the price of keeping the predicate in one place — cheap against six call sites
        // each threading raw report text.
        if candor_report::report_judged_nothing(&text) {
            out.judged_nothing.push(p);
        }
    }
    out
}
