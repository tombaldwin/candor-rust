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
    /// ⟨0.28⟩ SPEC §2 row 3, pinned verbatim in the rung that introduced it:
    /// `"noManifest": [ "<report path>", … ]  // consulted reports carrying no `analyzed` key`.
    #[serde(rename = "noManifest", skip_serializing_if = "Vec::is_empty")]
    pub(crate) no_manifest: Vec<String>,
    /// ⟨0.32⟩ The exclusion CLASSES the producing scan never opened, on the machine channel — the same
    /// wire spelling candor-ts publishes. Written ONLY on a run this verb ARMED (see
    /// [`ReportCompleteness::unread_armed`]), which is why it can be a plain `skip_serializing_if`
    /// field: on every other verb and every unarmed run the list is empty and the document is
    /// byte-identical to its pre-rung form.
    ///
    /// IT CARRIES THE CAUSE, and that is not decoration. `incomplete: true` alone tells an agent the
    /// answer is partial and nothing about WHY — and on this rung the repair is specific and cheap
    /// (re-run the producing scan WITH this policy), where the `unanalyzed` repair is not. The gate's
    /// own verdict document does not carry this key, because §3.1 makes it byte-equal to the scan
    /// route's; an advisory document is under no such constraint, and its reader has no stderr.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub(crate) unread: Vec<String>,
    /// ⟨R152⟩ SPEC §3.3.1 ⟨0.32⟩'s refusal-marker reason(s) — see [`ReportCompleteness::refused`]. A
    /// rust-only key today, on the same terms `unread` already ships on: `incomplete: true` alone would
    /// tell a `--json` consumer the answer is partial and say nothing about why, and this is the one
    /// cause in this struct whose reason is not recoverable from the report bytes at all (the whole
    /// point of a marker — see `refusal_marker_for`'s doc).
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub(crate) refused: Vec<String>,
}

/// ⟨R54⟩ [`CompletenessFields`]'s SAME key set, `baseline`-prefixed — for a two-locator verb whose
/// BASELINE side must be disclosed SEPARATELY from its CURRENT side (SOUNDNESS.md R54: `diff`). This is
/// not a new spelling: `gains --json` has published exactly this shape since ⟨0.28⟩
/// (`baselineIncomplete`/`baselineUnanalyzed`/`baselineJudgedNothing`/`baselineNoManifest`, built by
/// hand in `diff.rs`'s `attach_manifest`) precisely because a bare `incomplete: true` cannot say WHICH
/// of two reports was partial, and *the baseline was partial* / *the current scan was partial* license
/// opposite readings of the same comparison. This struct makes that shape reusable by a TYPED document
/// (`#[serde(flatten)]`, [`ReportCompleteness::baseline_fields`]) rather than a `serde_json::Value` one:
/// see [`ReportCompleteness::fields`]'s own doc for why `to_value` is the wrong route for a struct whose
/// field ORDER is part of the byte-identical contract.
#[derive(serde::Serialize)]
pub(crate) struct BaselineCompletenessFields {
    #[serde(rename = "baselineIncomplete")]
    pub(crate) incomplete: bool,
    #[serde(rename = "baselineUnanalyzed", skip_serializing_if = "Vec::is_empty")]
    pub(crate) unanalyzed: Vec<candor_report::UnanalyzedUnit>,
    #[serde(rename = "baselineJudgedNothing", skip_serializing_if = "Vec::is_empty")]
    pub(crate) judged_nothing: Vec<String>,
    #[serde(rename = "baselineNoManifest", skip_serializing_if = "Vec::is_empty")]
    pub(crate) no_manifest: Vec<String>,
    /// See [`CompletenessFields::unread`] — carried here on the same terms (empty and omitted unless
    /// this verb ever arms `unread_armed` on the baseline side, which none does today) so a future
    /// policy-carrying two-locator verb inherits the wire key rather than needing a second rung to add it.
    #[serde(rename = "baselineUnread", skip_serializing_if = "Vec::is_empty")]
    pub(crate) unread: Vec<String>,
    /// See [`CompletenessFields::refused`] — carried here on the same terms.
    #[serde(rename = "baselineRefused", skip_serializing_if = "Vec::is_empty")]
    pub(crate) refused: Vec<String>,
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
    /// ⟨0.28⟩ SPEC §2 — **THE THIRD ROW IS NOT THE FIRST ROW.** The reports under this locator that
    /// carry **NO `analyzed` KEY AT ALL** — §2's row 3, a pre-⟨0.21⟩ producer.
    ///
    /// MEASURED on this engine 2026-08-13 over `{"candor":…,"functions":[]}` with no `analyzed` key:
    /// every query verb listed the file under `judgedNothing` and the note said it *"say[s] they JUDGED
    /// NOTHING (`analyzed.count: 0`)"*. **The report declares nothing.** The HEDGE is the right
    /// direction — row 3's own instruction is *no manifest, no claim* — but the disclosure is FALSE, and
    /// this family rates a false disclosure worse than a missing one (§3.4's `net-partner` finding: an
    /// engine reported "ignoring unknown config key" *while honouring it*).
    ///
    /// A SEPARATE FIELD, NOT A RE-LABEL, because `judgedNothing` is pinned to *"reports declaring
    /// `analyzed.count: 0`"*: filing row 3 there makes one key mean two things and loses the distinction
    /// the table exists to draw. The REPAIRS differ — row 1 wants a scan that reaches a conclusion, row 3
    /// wants a producer that emits a manifest at all.
    ///
    /// It raises [`Self::must_hedge`] exactly as its two siblings do and, like them, stops at the exit
    /// code: [`Self::incomplete`] does not read it.
    pub(crate) no_manifest: Vec<String>,
    /// ⟨0.30⟩ The peek's findings carried by the reports under this locator — functions OUTSIDE the
    /// scan's scope performing an effect the policy DENIES. An arm of [`Self::incomplete`] because
    /// ⟨0.24⟩ binds it: *"AN ADVISORY VERB MUST NEVER BE LESS SENSITIVE TO INCOMPLETENESS THAN THE GATE
    /// OVER THE SAME BYTES"*, and *"THE SAME RULE BINDS EVERY ADVISORY VERB THAT ANSWERS `ok` —
    /// `unverified`, `fix-gate`, and any later sibling"*. ⟨0.30⟩ made the gate exit 2 on this cause and
    /// left these verbs behind: MEASURED, `gate --report` exited 2 over a report whose peek had resolved
    /// a denied effect while `unverified --strict` printed *"every function in a pure/deny layer is
    /// PROVABLY clean ✓"* at exit 0 — the rung's own false all-clear, moved sideways into the sibling.
    pub(crate) out_of_scope: Vec<candor_report::OutOfScopeFinding>,
    /// ⟨0.32⟩ The exclusion CLASSES the producing scan never opened — `excluded[]` entries that are
    /// neither `peeked` nor `judgedElsewhere`, read off the SAME key and through the SAME reader
    /// `gate --report` uses ([`candor_report::report_excluded`]). The sibling of `out_of_scope` and the
    /// other half of one rung: that one is what the peek FOUND, this is what nothing ever opened.
    ///
    /// **COLLECTED HERE, ARMED BY THE VERB** — see [`Self::unread_armed`] and [`arm_unread`]. This
    /// function reads a report locator and holds no policy, and the condition is about the policy in
    /// force NOW.
    pub(crate) unread: Vec<String>,
    /// ⟨0.32⟩ Has the calling verb decided that THIS run's policy makes [`Self::unread`] matter?
    ///
    /// **THE CONDITION IS THE QUESTION BEING ASKED, NEVER THE PRODUCER'S HISTORY** — only a
    /// `deny`/`pure` rule's answer depends on code outside the scan's scope, so `allow`/`forbid`/`only`/
    /// `layer` must cost an unread class nothing. Held as its own flag rather than inferred from
    /// `unread` being non-empty so that *"no policy was given"* and *"this policy denies nothing"*
    /// cannot be confused with *"the producer read everything"*.
    ///
    /// **AND IT IS WHY THIS IS NOT AN UNCONDITIONAL ARM *OF THE EXIT CODE*.** An unread class rides
    /// almost every report a bare `candor-scan <dir> --out r` writes — any tree with a build script,
    /// tests, benches or examples — and a `--strict` verb that exited 2 on every one of them would be
    /// MORE pessimistic than the gate, which ⟨0.24⟩ forbids in the same breath as the under-claim.
    ///
    /// ⟨0.32⟩ **IT IS NOT WHAT THE *DISCLOSURE* IS KEYED ON — SEE [`Self::must_hedge`], WHICH READS
    /// [`Self::unread`] DIRECTLY.** The paragraph above used to end *"the descriptive verbs never arm
    /// it … they carry no policy, so there is no question whose answer could depend on the unread
    /// code"*, and that was ruled WRONG four-way on 2026-08-24; the ruling is on `must_hedge`.
    pub(crate) unread_armed: bool,
    /// ⟨0.33⟩ RAW MATERIALS for [`arm_unasked_rules`] — one entry per report under the locator: did THAT
    /// report have any `excluded[]` entry that was `peeked` and NOT `judgedElsewhere` (the SPEC §2
    /// ⟨0.33⟩ precondition), and what deny set did its peek run under (`None` = `scannedUnder` ABSENT,
    /// which is the EMPTY SET for the subset test, never a licence).
    ///
    /// Collected PER REPORT rather than pre-unioned, for the same reason `gate --report` computes
    /// [`crate::gate`]'s equivalent per report: `scannedUnder` and `peeked` are facts about ONE producing
    /// scan, and unioning them before comparing would let a policy-scanned report's deny set answer for a
    /// no-policy sibling's peeked classes.
    ///
    /// ⟨0.34⟩ the THIRD element is that report's own declared `candor.spec` ([`candor_report::report_spec`],
    /// verbatim, empty for a pre-spec-field producer) — carried alongside rather than re-read later,
    /// for the reason `any_peeked`/`scanned_under` are captured here instead of re-parsing `text`: a
    /// second read of one report is how a fact drifts from the one [`arm_unasked_rules`] compares it
    /// against.
    pub(crate) scanned_under_facts: Vec<(bool, Option<Vec<String>>, String)>,
    /// ⟨0.33⟩ The rules THIS run's policy holds that some report's peek was never asked about (SPEC §2
    /// ⟨0.33⟩) — canonical, deduplicated, code-point sorted. Empty until [`arm_unasked_rules`] is called;
    /// mirrors [`Self::unread_armed`]'s split between raw collection and policy-scoped arming, but needs
    /// no separate `_armed` flag because the value ITSELF is structurally empty whenever this run's own
    /// deny set is empty (an allow/forbid/only-only policy, or no policy at all) — the over-charge control
    /// falls out of the computation rather than needing a second conjunct.
    pub(crate) unasked_rules: Vec<String>,
    /// ⟨0.34⟩ **NAMES THE CAUSE, NEVER MOVES THE VERDICT.** `true` when [`Self::unasked_rules`] is
    /// non-empty AND every report that contributed to it predates ⟨0.33⟩ ([`candor_report::spec_predates`]
    /// against `"0.33"`) — i.e. the gap is fully explained by producers that could not yet have written
    /// `scannedUnder` at all, never by one that ran under a genuinely different or narrower deny set. A
    /// SINGLE ≥⟨0.33⟩ contributing report is enough to keep this `false`, because for that report the
    /// omission is real: it COULD have recorded covering this run's rules and did not. See
    /// [`arm_unasked_rules`] for the per-report accounting this is built from.
    pub(crate) unasked_rules_predates_033: bool,
    /// ⟨R152⟩ SPEC §3.3.1 ⟨0.32⟩: *"A consumer resolving a `--report` locator MUST consult the marker
    /// and refuse (exit 2) when one is present for the reports it is about to read."* `gate --report`
    /// has honoured this since ⟨0.32⟩ ([`crate::gate`]'s own `load_gate_report`); every OTHER reader of
    /// this locator did not, because `refusal_marker_for` had exactly one caller in the whole repo. A
    /// refused prefix is bytes the gate has disowned — certifying them here would be MORE confident
    /// than the gate over the SAME locator, which is the over-claim ⟨0.24⟩'s "an advisory verb may be
    /// LESS certain than the gate, never more" already forbids for every other cause in this struct.
    ///
    /// One entry per locator that carried a marker (0 or 1 for a one-locator verb, up to 2 for
    /// `containment`/`diff`'s current+baseline pair via [`Self::absorb`]) — a `Vec`, not an `Option`,
    /// for the same reason `unread`/`unanalyzed` are: the shape [`Self::absorb`] already `.extend()`s.
    pub(crate) refused: Vec<candor_report::RefusalMarker>,
}

/// ⟨0.32⟩ **ARM THE UNREAD-CLASS CAUSE FOR THIS RUN'S POLICY** — the one place the condition is applied
/// on the advisory route, so the three verbs that carry a policy cannot answer it three ways.
///
/// **APPLIED ONCE TO THE VALUE**, exactly as `cmd_gate` applies it to `rep.unpeeked`: this object feeds
/// the exit code, the JSON document and the prose note, and a condition stated at only one of them lets
/// them disagree about one run. That split is not hypothetical — it has now been found in three engines,
/// most recently on candor-scan's own `--gate-json` (a document reading `"ok": false, "incomplete":
/// true` beside exit 0).
///
/// `p.rules` IS THE DENY LIST AND `pure` IS IN IT — the parser records a `pure` line as a rule with an
/// EMPTY effect list (§2.2 ⟨0.30⟩). Reading the question off a flattened set of effect NAMES would get
/// nothing from that and let the STRICTEST policy the grammar has disarm the rung; measured four-way on
/// the scan route once already, which is why the conformance arm carries a `pure` row.
pub(crate) fn arm_unread(mut c: ReportCompleteness, p: &candor_classify::policy::ParsedPolicy) -> ReportCompleteness {
    if p.rules.is_empty() {
        // CLEARED, not merely left unarmed: nothing downstream may read a list this run decided is not
        // a question, and the document key is built off the same vector.
        c.unread.clear();
    }
    c.unread_armed = !c.unread.is_empty();
    c
}

/// ⟨0.33⟩ **ARM THE CROSS-POLICY CAUSE FOR THIS RUN'S POLICY** (SPEC §2 ⟨0.33⟩) — the one place the
/// condition is applied on the advisory route, beside [`arm_unread`] for the identical reason: `gate
/// --report` binds every advisory verb that answers `ok` to be at least as pessimistic as it is over the
/// same bytes (⟨0.24⟩), and a second computation of that relation is how it has drifted before (PART 67).
///
/// **PER REPORT, NEVER OVER THE UNION** — the same rule `Query::unaskedRules`/`candor-query gate --report`
/// applies: `scannedUnder` and `peeked` are facts about ONE producing scan, so unioning [`Self::
/// scanned_under_facts`] first would let a policy-scanned report's deny set answer for a no-policy
/// sibling's peeked classes.
///
/// **STRUCTURALLY EMPTY, not by a conjunct.** `own` is the canonical form of `p.rules`; when it is empty
/// (an `allow`/`forbid`/`only`-only policy denies nothing, so its canonical set is empty and a subset of
/// everything) the loop below contributes nothing, and a report with no `any_peeked` fact never
/// contributes either — both of ⟨0.33⟩'s over-charge controls fall out of the computation rather than
/// needing a second `if`.
pub(crate) fn arm_unasked_rules(
    mut c: ReportCompleteness,
    p: &candor_classify::policy::ParsedPolicy,
) -> ReportCompleteness {
    let own = candor_classify::policy::canonical_deny_set(&p.rules);
    let mut missing: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
    // ⟨0.34⟩ SPEC §2 ⟨0.34⟩ — did ANY report ≥⟨0.33⟩ ALSO fail to cover a rule that ended up in
    // `missing`? One such report is enough: for it the gap is real (it could have recorded covering
    // this run's rules and did not), so a message naming "before producers recorded their deny set"
    // would be false of it even if every OTHER contributing report predates the rung. Tracked beside
    // the union rather than derived from it afterward, because `missing` alone has already forgotten
    // which report each entry came from — the same reason `scanned_under_facts` is per-report above.
    let mut missing_from_current_spec = false;
    if !own.is_empty() {
        for (any_peeked, scanned_under, spec) in &c.scanned_under_facts {
            if !*any_peeked {
                continue;
            }
            let theirs: std::collections::BTreeSet<&str> =
                scanned_under.as_deref().unwrap_or(&[]).iter().map(String::as_str).collect();
            let predates = candor_report::spec_predates(spec, "0.33");  // ⟨0.33⟩ — the RUNG this comparison names, not a version that bumps
            for r in &own {
                if !theirs.contains(r.as_str()) {
                    missing.insert(r.clone());
                    if !predates {
                        missing_from_current_spec = true;
                    }
                }
            }
        }
    }
    c.unasked_rules = missing.into_iter().collect();
    c.unasked_rules_predates_033 = !c.unasked_rules.is_empty() && !missing_from_current_spec;
    c
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
    ///
    /// ⟨0.32⟩ **`unread_armed` IS AN ARM, and it is the same MUST arriving one shape over.**
    /// `gate --report` refuses over a class the producing scan never opened, so a `--strict` verb over
    /// those bytes must not certify. MEASURED on the release build at `ab505c0`, the moment the gate
    /// route gained the rule and stopped there, over the PART 62 rust fixture (an unreadable `build.rs`
    /// running `curl`, scanned with no policy, gated under `deny Exec`):
    ///
    /// ```text
    ///   gate --report N --policy P            exit 2   {"ok": false, "incomplete": true}
    ///   fix-gate   --report N --policy P -s   exit 0   {"ok": true, "remedies": []}
    ///   unverified --report N --policy P -s   exit 0   {"ok": true, "unverified": []}
    /// ```
    ///
    /// Closing a cause on the gate and leaving its siblings is how the ⟨0.30⟩ half of this same rung
    /// drifted first (`out_of_scope`, one line up). Twice says the ARM is what a new verdict cause
    /// needs, not a comment telling the next author to remember.
    ///
    /// **AND `unverified`'S ANSWER LOOKED RIGHT FOR THE WRONG REASON.** Over a fixture whose functions
    /// carry `Unknown`, it exited 1 on the holes it found and read as a refusal; over the same tree with
    /// no hole in it, it answered `{"ok": true, "unverified": []}` at 0. A non-zero exit reached by a
    /// different finding is not this relation being satisfied — which is why the pinned row's fixture
    /// has every finding set empty and the unread class as the only thing that can move a verb.
    pub(crate) fn incomplete(&self) -> bool {
        !self.unanalyzed.is_empty()
            || !self.unreadable.is_empty()
            || !self.out_of_scope.is_empty()
            || self.unread_armed
            // ⟨0.33⟩ THE FOURTH CAUSE (SPEC §2 ⟨0.33⟩) — a class the producing scan READ, but under a
            // DIFFERENT deny set than this run holds. Needs no separate `_armed` flag the way `unread`
            // does: `arm_unasked_rules` already leaves this structurally empty whenever the policy in
            // force asks nothing (see that function).
            || !self.unasked_rules.is_empty()
            // ⟨R152⟩ SPEC §3.3.1 ⟨0.32⟩ — see the field doc on `refused`. `gate --report` refuses
            // UNCONDITIONALLY over a marked prefix; joining this arm here, rather than adding a second
            // exit-code check beside the five above, is what makes a `--strict` advisory verb's "2" mean
            // the SAME thing `gate --report`'s "2" means for every cause, this one included.
            || !self.refused.is_empty()
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
    ///
    /// ⟨0.28⟩ `no_manifest` (SPEC §2 row 3) is an arm of THIS and not of [`Self::incomplete`], for the
    /// identical reason `judged_nothing` is: the gate exits 0 over a manifest-less report too, so a verb
    /// exiting 2 there would claim it got LESS far than the gate on the same bytes. The row-3 split
    /// re-routes a hedge that was already happening; it must not also move an exit code.
    ///
    /// ⟨0.32⟩ **AND [`Self::unread`] IS AN ARM OF THIS, *UNARMED* — RULED 2026-08-24 AFTER A FOUR-WAY
    /// DIVERGENCE. DO NOT RE-LITIGATE IT HERE.** Over a report whose `excluded` names a class the scan
    /// never opened, `tour` printed the bare *"nothing hidden — every effect sits where its name says it
    /// should"* at exit 0 in candor-rust, candor-ts and candor-swift, while candor-java hedged and named
    /// the class. **candor-java was right.**
    ///
    /// **IT IS A DISCLOSURE, NOT A VERDICT, AND IT MUST NOT MOVE AN EXIT CODE** — which is why the arm is
    /// here and NOT on [`Self::incomplete`]. ⟨0.24⟩'s advisory-verb pessimism MUST binds verbs that
    /// answer `ok`; `tour` answers none and has no exit-code obligation, so that clause does not reach
    /// it. What reaches it is SPEC §2 ⟨0.28⟩, which widens the re-disclosure MUST to *"any verb whose
    /// output could be read as a negative finding about the code — a verdict, an empty result set, or a
    /// zero count"*, and SPEC §3.1 ⟨0.18⟩, which already forbids THIS EXACT SENTENCE over a ≥⅓-Unknown
    /// graph. An unread exclusion class is the same ignorance arriving by a different route, and the ⅓
    /// threshold structurally CANNOT see it: an unread unit contributes no entry, so it moves neither
    /// the numerator nor the denominator.
    ///
    /// **AND THE ARGUMENT THAT KEPT IT OUT WAS THE WRONG WAY ROUND.** Three engines reasoned *"these
    /// verbs carry no policy, so there is no question whose answer could depend on the unread code"*.
    /// The condition ⟨0.32⟩ states is the QUESTION IN FORCE, and a verb with no policy is not asking a
    /// NARROWER question than `deny Exec` — it is asking the widest one there is, the whole effect
    /// surface. A `deny`/`pure` rule's answer can depend on unread code, so [`arm_unread`] arms; an
    /// `allow`/`forbid`/`only`/`layer` policy's answer cannot, so it CLEARS the list and this arm goes
    /// quiet with it; a descriptive verb's answer always can.
    ///
    /// **THE TRIGGER IS THE GATE'S, MINUS THE POLICY CONDITION**: `peeked == false` with no
    /// `judgedElsewhere`, off the same key through the same reader, `count` IGNORED (measured
    /// 2026-08-24 — all four gates refuse over a `count: 0` unread class and certify over a
    /// `judgedElsewhere: true` one). One matcher, so a report that earns an unhedged `tour` is exactly a
    /// report `gate --report` can certify. The NOISE objection — this fires on nearly every no-policy
    /// report — is real, and it is answered by the REMEDY rather than by silence: scan with the policy,
    /// the peek reads the class, `peeked` turns true and the hedge goes away.
    ///
    /// **KNOWN RESIDUAL, stated rather than asserted away:** `peeked: true` means the class was READ,
    /// not ANALYZED — the peek looks only for effects the PRODUCER's policy denied — so an undenied
    /// effect inside a peeked class is still outside `tour`'s graph and outside this hedge. That is the
    /// gate's residual too (SPEC §2 ⟨0.32⟩ files it against a report recording its own deny set), and
    /// closing it is a rung, not a fix.
    pub(crate) fn must_hedge(&self) -> bool {
        self.incomplete()
            || !self.judged_nothing.is_empty()
            || !self.no_manifest.is_empty()
            || !self.unread.is_empty()
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
    ///
    /// ⟨0.28⟩ **AND A ROW-3-ONLY HEDGE GETS THE SAME EXIT REPORTED WITHOUT THE WRONG NOUN.** The gate
    /// exits 0 over a manifest-less report too (its own note names both conditions — *"`analyzed.count`
    /// is 0, or absent with no entries"*), so the urgency is identical; but calling the report
    /// *judged-nothing* in a sentence printed under the row-3 disclosure would re-assert, in prose, the
    /// exact claim the split was made to stop making.
    pub(crate) fn gate_line(&self) -> &'static str {
        if self.incomplete() {
            "`gate --report` exits 2 over these bytes."
        } else if !self.unread.is_empty() {
            // ⟨0.32⟩ THE UNARMED UNREAD CAUSE GETS ITS OWN SENTENCE, because both of the ones below are
            // FALSE of it in opposite directions. "exits 2 over these bytes" is unqualified and this verb
            // holds no policy to say it under; "exits 0 over a judged-nothing report" names a cause that
            // is not present and sends the reader to a CI job that will pass. `gate --report` can only
            // ever evaluate a deny-family rule — measured 2026-08-24, all four engines refuse an
            // `allow`-only policy as NO RULES and a `forbid` rule as unevaluable on this route — so the
            // exit is a certainty once a policy exists, and the gap is only that none does here.
            "`gate --report` exits 2 over these bytes under any policy it can evaluate (they are all \
             `deny`/`pure`), and this verb holds none — so NOTHING DOWNSTREAM IS FAILING CLOSED ON IT \
             HERE and this note is the whole of the warning."
        } else if self.judged_nothing.is_empty() {
            "NOTHING DOWNSTREAM WILL CATCH THIS FOR YOU — `gate --report` exits 0 over a report carrying \
             no `analyzed` manifest (⟨0.24⟩: a disclosure, not an exit code), so this note is the whole \
             of the warning."
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
        self.no_manifest.extend(other.no_manifest);
        // ⟨0.32⟩ …and the unread classes, with the ARMING ORed rather than replaced: a baseline armed
        // under this run's policy stays armed after the union, and an unarmed side cannot disarm an
        // armed one. `containment` is the only caller, and its answer is a DIFFERENCE — unsound if
        // either side is partial.
        self.unread.extend(other.unread);
        self.unread_armed |= other.unread_armed;
        // ⟨0.33⟩ …and the cross-policy raw materials + any already-armed value, on the same terms:
        // `containment` never arms either cause today, so this is symmetry with `unread` above rather
        // than a reachable path, and it is what keeps a future armed caller from silently losing one side
        // of a diff.
        self.scanned_under_facts.extend(other.scanned_under_facts);
        // ⟨0.34⟩ computed BEFORE the union below overwrites the per-side emptiness this needs: a plain
        // `|=` would let one side's `true` (its OWN `unasked_rules` was small and fully pre-⟨0.33⟩) paper
        // over the other side's `false` (a genuine ≥⟨0.33⟩ mismatch), on a MERGED list neither side's flag
        // was computed against. A side that contributed NOTHING (`unasked_rules` empty, flag `false` by
        // [`arm_unasked_rules`]'s invariant) must not veto the other's true answer either — hence the
        // match on which side(s) actually had something to say, not a blanket AND. Symmetry with
        // `unread_armed` above, not a reachable path: `containment` never arms `unasked_rules` today.
        self.unasked_rules_predates_033 = match
            (!self.unasked_rules.is_empty(), !other.unasked_rules.is_empty())
        {
            (false, false) => false,
            (true, false) => self.unasked_rules_predates_033,
            (false, true) => other.unasked_rules_predates_033,
            (true, true) => self.unasked_rules_predates_033 && other.unasked_rules_predates_033,
        };
        self.unasked_rules.extend(other.unasked_rules);
        // ⟨R152⟩ …and any refusal marker either locator carried — plain `.extend()`, like `unread` and
        // `unanalyzed` above: a baseline that is refused is exactly as disqualifying as a current tree
        // that is, and `containment`'s answer is a DIFFERENCE, unsound if either side is disowned.
        self.refused.extend(other.refused);
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
            // ⟨0.28⟩ SPEC §2 row 3, pinned as `noManifest` in the rung that introduced it. Its own key
            // rather than a third member of `judgedNothing`, because that key is defined as "reports
            // declaring `analyzed.count: 0`" and a row-3 report declares nothing — and because the two
            // send the reader to different repairs. Omitted when empty like the other two, so a document
            // raised by either sibling alone is byte-identical to its pre-row-3 form.
            no_manifest: self.no_manifest.clone(),
            // ⟨0.32⟩ The classes nothing opened, on the machine channel. Empty on every unarmed run —
            // which is every descriptive verb and every policy with no deny rule — so a document raised
            // by any pre-⟨0.32⟩ cause alone stays byte-identical to its pre-rung form.
            //
            // ⟨0.32⟩ **STILL `unread_armed` AND DELIBERATELY NOT `!unread.is_empty()`, even though the
            // PROSE note moved.** This is a WIRE key, and the only engine that publishes it is this one
            // (measured 2026-08-24: `fix-gate --json` over one unread report — rust `{"incomplete":
            // true, "unread": […]}`, java and swift `{"incomplete": true}` alone). Widening it to the
            // descriptive documents would mint a fifth key set on the six verbs where three engines
            // publish none, which is §3.3.1's *"four independent guesses with a conformance failure
            // scheduled"*. The descriptive hedge raises `incomplete: true` and adds NO key of its own —
            // the rule this module already applies to `unreadable`, and the one ts and swift state for
            // themselves. Closing the rust-only ADVISORY key four-way is a separate rung.
            unread: if self.unread_armed { self.unread.clone() } else { Vec::new() },
            // ⟨R152⟩ Unconditional, unlike `unread` above: this cause needs no policy-scoped arming —
            // `refused` is a raw fact about the LOCATOR (SPEC §3.3.1 ⟨0.32⟩), not a question whose
            // relevance depends on what the run's policy asks. See [`Self::refused`]'s field doc.
            refused: self.refused.iter().map(|m| m.reason.clone()).collect(),
        })
    }

    /// ⟨R54⟩ [`Self::fields`], `baseline`-prefixed — see [`BaselineCompletenessFields`] for why this is
    /// a reused shape and not a new one. Same trigger (`must_hedge`), same no-op-when-complete rule, so
    /// a two-locator verb's intact side stays byte-identical to a one-locator verb's.
    pub(crate) fn baseline_fields(&self) -> Option<BaselineCompletenessFields> {
        if !self.must_hedge() {
            return None;
        }
        Some(BaselineCompletenessFields {
            incomplete: true,
            unanalyzed: self.unanalyzed.clone(),
            judged_nothing: self.judged_nothing.clone(),
            no_manifest: self.no_manifest.clone(),
            unread: if self.unread_armed { self.unread.clone() } else { Vec::new() },
            refused: self.refused.iter().map(|m| m.reason.clone()).collect(),
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
        //
        // ⟨0.28⟩ …and SPEC §2's THIRD ROW gets its OWN clause, appended, for the same reason: the
        // sentence above was FALSE of it. A manifest-less report does not "say it judged nothing" — it
        // says nothing, and a reader sent to re-run a scan that already reached a conclusion goes to the
        // wrong repair. Appended rather than folded into the existing arms so the two measured wordings
        // stay character-for-character what they were when no row-3 report is present.
        //
        // ⟨0.32⟩ **AND THE FIRST ARM ASKS `units()`, NOT `incomplete()`.** Those are different questions
        // and the gap between them is a sentence that says nothing: `incomplete()` has counted the two
        // SCOPE causes since ⟨0.30⟩ while this head was built from the MANIFEST rows alone, so a note
        // whose ONLY cause is out-of-scope or unread code came out as *"declare 0 unit(s) candor could
        // not analyze"* — a hedge that names no cause, which is the deleted-disclosure defect arriving
        // inside the disclosure. Latent while the unread-class rule was gated on the producer's history;
        // reachable on nearly every no-policy report the moment it was not. candor-java measured the
        // same line on the same rung.
        let mut head = match (self.units() > 0, self.judged_nothing.len()) {
            (true, 0) => format!(
                "the report(s) under this locator declare {} unit(s) candor could not analyze,",
                self.units()
            ),
            (true, n) => format!(
                "the report(s) under this locator declare {} unit(s) candor could not analyze, and {n} \
                 report(s) that judged nothing at all,",
                self.units()
            ),
            // Reachable only with a row-3 report in hand: `must_hedge` gated the early return above, and
            // with no unanalyzed unit, no unreadable file and no count-0 report, `no_manifest` is the
            // only arm left that could have raised it.
            (false, 0) => String::new(),
            (false, n) => format!(
                "{n} report(s) under this locator say they JUDGED NOTHING (`analyzed.count: 0`),"
            ),
        };
        // ONE CLAUSE PER CAUSE, appended by one rule: the `alone` wording when nothing precedes it (a
        // clause has to be a sentence on its own), the `joined` wording otherwise — and the joined form
        // eats the preceding clause comma so the whole reads `…, and N …,`. Written once because the
        // row-3 block below was copied twice before this rung and the copies drifted in their verb.
        fn append(head: &mut String, alone: String, joined: String) {
            if head.is_empty() {
                *head = alone;
            } else {
                head.pop(); // the clause comma, so the joined sentence reads `…, and N report(s) …,`
                head.push_str(&joined);
            }
        }
        // ⟨R152⟩ SPEC §3.3.1 ⟨0.32⟩ — placed FIRST among the appended clauses, ahead of every cause a
        // producing scan merely DECLARED, because this one says the producing RUN itself never
        // completed: nothing else in `head` is trustworthy context for it, so it leads.
        if let n @ 1.. = self.refused.len() {
            append(
                &mut head,
                format!(
                    "{n} report(s) under this locator are marked REFUSED (SPEC §3.3.1 ⟨0.32⟩) — the \
                     most recent producing scan over them refused and never completed,"
                ),
                format!(", and {n} report(s) under this locator marked REFUSED,"),
            );
        }
        if let n @ 1.. = self.no_manifest.len() {
            append(
                &mut head,
                format!(
                    "{n} report(s) under this locator carry NO `analyzed` manifest at all (SPEC §2 row \
                     3, a pre-⟨0.21⟩ producer),"
                ),
                format!(", and {n} report(s) carrying NO `analyzed` manifest at all,"),
            );
        }
        // ⟨0.30⟩ THE PEEK'S FINDINGS — an arm of `incomplete()` since that rung, and named here since
        // ⟨0.32⟩ made the omission reachable.
        if let n @ 1.. = self.out_of_scope.len() {
            append(
                &mut head,
                format!(
                    "the report(s) under this locator name {n} function(s) OUTSIDE the scan's scope \
                     performing an effect the producing scan's policy DENIED,"
                ),
                format!(
                    ", and {n} function(s) OUTSIDE the scan's scope performing a DENIED effect,"
                ),
            );
        }
        // ⟨0.32⟩ …and the classes nothing OPENED, on `unread` DIRECTLY rather than on `unread_armed` —
        // see `must_hedge`, which carries the ruling. A policy with no deny rule still reaches nothing
        // here, because `arm_unread` CLEARS the list rather than merely leaving it unarmed.
        if !self.unread.is_empty() {
            let n = self.unread.len();
            append(
                &mut head,
                format!(
                    "the report(s) under this locator declare {n} exclusion class(es) the scan did NOT \
                     READ (`excluded[].peeked: false`),"
                ),
                format!(", and {n} exclusion class(es) the scan did NOT READ,"),
            );
        }
        // ⟨0.33⟩ …and the rules THIS policy holds that some peeked class's producer was never asked about
        // (SPEC §2 ⟨0.33⟩). Needs no `_armed` check the way `unread` does above — `arm_unasked_rules`
        // already leaves this structurally empty whenever the policy in force asks nothing.
        //
        // ⟨0.34⟩ TWO SENTENCES FOR ONE CAUSE, chosen by `unasked_rules_predates_033` — SAME verdict, SAME
        // exit, ONLY the head sentence's wording moves. When every contributing report predates ⟨0.33⟩ the
        // "does not cover" phrasing is misleading: it reads as "a producer chose a different policy", and
        // the true statement is "no producer here could yet WRITE the policy it peeked under". The `else`
        // arm is character-for-character the pre-⟨0.34⟩ text — the control this rung ships with.
        if !self.unasked_rules.is_empty() {
            let n = self.unasked_rules.len();
            if self.unasked_rules_predates_033 {
                append(
                    &mut head,
                    format!(
                        "the report(s) under this locator predate ⟨0.33⟩ — before a producing scan \
                         recorded the deny set its peek ran under — so they cannot say whether {n} \
                         rule(s) of THIS policy were ever asked,"
                    ),
                    format!(
                        ", and {n} rule(s) of THIS policy the report(s) — from before ⟨0.33⟩ — cannot say \
                         they were asked about,"
                    ),
                );
            } else {
                append(
                    &mut head,
                    format!(
                        "the report(s) under this locator were peeked under a deny set that does not cover \
                         {n} rule(s) of THIS policy,"
                    ),
                    format!(
                        ", and {n} rule(s) of THIS policy the report(s) were peeked under a DIFFERENT deny \
                         set,"
                    ),
                );
            }
        }
        writeln!(w, "  ⚠ INCOMPLETE — {head}")?;
        writeln!(w, "      so {so_what}:")?;
        for m in &self.refused {
            writeln!(
                w,
                "      {} — REFUSED: {}. `candor-query gate --report` refuses (exit 2) over these bytes \
                 unconditionally (SPEC §3.3.1 ⟨0.32⟩); re-scan to clear the marker",
                m.prefix, m.reason
            )?;
        }
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
        for p in &self.no_manifest {
            writeln!(
                w,
                "      {p} — NO `analyzed` manifest at all (SPEC §2 row 3, a pre-⟨0.21⟩ producer): it \
                 DECLARES nothing about what was judged, so its silence licenses no purity claim \
                 either. Re-scan with a current engine so the report carries its manifest"
            )?;
        }
        for o in &self.out_of_scope {
            writeln!(
                w,
                "      {} — OUTSIDE the producing scan's scope: it performs {}, and the gate did not \
                 judge it",
                o.func,
                o.effects.join(", ")
            )?;
        }
        for c in &self.unread {
            // ONE FACT SENTENCE, TWO REMEDIES. The fact is the same on both routes and stays
            // character-for-character what the armed route was measured on; the REPAIR is not — an
            // armed run already holds the policy to re-scan with, and a descriptive verb holds none, so
            // telling it to re-run "WITH this policy" names a thing the reader does not have.
            let remedy = if self.unread_armed {
                "Re-run the producing scan WITH this policy (candor-scan <dir> --policy <p>)"
            } else {
                "Re-run the producing scan WITH a `deny`/`pure` policy so the peek reads it \
                 (candor-scan <dir> --policy <p>)"
            };
            writeln!(
                w,
                "      {c} — this exclusion class went UNREAD (`excluded[].peeked: false`): its \
                 effects are absent because nothing looked, not because there are none. {remedy}"
            )?;
        }
        if !self.unasked_rules.is_empty() {
            // ⟨0.34⟩ ONE FACT, TWO REMEDIES — same split as `unread`'s `remedy` above, and for the same
            // reason: the CAUSE differs, so the REPAIR does. `else` is the pre-⟨0.34⟩ sentence, unchanged.
            let cause_and_remedy = if self.unasked_rules_predates_033 {
                "these reports were produced before ⟨0.33⟩, when a producing scan did not yet record the \
                 deny set its peek ran under — so an empty finding there cannot be read as an answer to \
                 THIS policy's question. Re-scan with a 0.33+ engine under THE SAME policy this verb is \
                 applying (candor-scan <dir> --policy <p>) — not merely under a policy"
            } else {
                "never asked of the excluded files a peeked class's producer read: the empty finding \
                 there answers a DIFFERENT question, not this one. Re-run the producing scan under THE \
                 SAME policy this verb is applying (candor-scan <dir> --policy <p>) — not merely under a \
                 policy"
            };
            writeln!(w, "      {} — {cause_and_remedy}", self.unasked_rules.join(", "))?;
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
    let mut out = ReportCompleteness {
        unanalyzed: Vec::new(),
        unreadable: Vec::new(),
        judged_nothing: Vec::new(),
        no_manifest: Vec::new(),
        out_of_scope: Vec::new(),
        unread: Vec::new(),
        unread_armed: false,
        scanned_under_facts: Vec::new(),
        unasked_rules: Vec::new(),
        unasked_rules_predates_033: false,
        refused: Vec::new(),
    };
    // ⟨R152⟩ SPEC §3.3.1 ⟨0.32⟩ — see the field doc on `refused`. Checked ONCE, against the LOCATOR,
    // exactly as `gate --report`'s own `load_gate_report` checks it — never per glob'd report file,
    // because the marker is a fact about the PREFIX's most recent producing run, not about any one file
    // under it (a `.refused.json` sits BESIDE the reports, never inside their glob).
    if let Some(m) = candor_report::refusal_marker_for(prefix) {
        out.refused.push(m);
    }
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
        // ⟨0.30⟩ the peek's findings, read as strictly as `unanalyzed` above — an unreadable key is
        // corrupt input, never its permissive empty value, because non-emptiness is a fail-closed trigger.
        match candor_report::report_out_of_scope(&text) {
            candor_report::KeyRead::Present(o) => out.out_of_scope.extend(o),
            candor_report::KeyRead::Absent => {}
            candor_report::KeyRead::Corrupt => {
                out.unreadable.push(Unreadable { path: p, key_present: true });
                continue;
            }
        }
        // ⟨0.32⟩ …and the SCOPE the producer recorded, off the SAME key and through the SAME reader
        // `load_gate_report` uses (`candor_report::report_excluded`) — shared rather than re-spelled,
        // because two readings of one flag is exactly how the two arms of ⟨0.30⟩ drifted. The FILTER is
        // the gate's too: `peeked` says the producer opened the class, `judged_elsewhere` is the
        // producer's own carve-out for a derived copy of code this same scan already judged.
        //
        // CORRUPT RIDES `unreadable`, as the `out_of_scope` block above does it and for its reason: an
        // `excluded` key coerced to `[]` is the claim "this scan excluded nothing" — the safe-LOOKING
        // value — and here it would silently DELETE an arm rather than raise one. The gate refuses over
        // the same bytes naming the key, so an advisory verb that read them leniently would be LESS
        // pessimistic than the gate (SPEC §3.2).
        //
        // COLLECTED UNCONDITIONALLY; whether it MATTERS is `arm_unread`'s decision, because it turns on
        // the policy in force and this function holds none.
        // ⟨0.33⟩ THE PRECONDITION of the cross-policy cause (SPEC §2 ⟨0.33⟩): did ANY class of THIS report
        // get read (`peeked`) and not carved out as `judgedElsewhere`? Captured here, before `x` is
        // consumed below, because it is the SAME `excluded` read `unread` already does — a second parse
        // of one key is how the two would drift.
        let mut any_peeked = false;
        match candor_report::report_excluded(&text) {
            candor_report::KeyRead::Present(x) => {
                any_peeked = x.iter().any(|e| e.peeked && !e.judged_elsewhere);
                out.unread.extend(
                    x.into_iter().filter(|e| !e.peeked && !e.judged_elsewhere).map(|e| e.class),
                );
            }
            candor_report::KeyRead::Absent => {}
            candor_report::KeyRead::Corrupt => {
                out.unreadable.push(Unreadable { path: p, key_present: true });
                continue;
            }
        }
        // ⟨0.33⟩ …and THE QUESTION THIS REPORT'S PEEK WAS PUT — read as strictly as `excluded`/
        // `outOfScope` above, and for the identical fail-open reason: a garbled `scannedUnder` coerced to
        // its permissive value would MANUFACTURE coverage this report's producer never claimed. Recorded
        // per report (never unioned yet) for [`arm_unasked_rules`] to consume once this run's policy is
        // known.
        // ⟨0.34⟩ this report's own declared contract version, carried alongside the fact it qualifies —
        // see the field doc on `scanned_under_facts` for why it travels with the tuple instead of being
        // re-read from `text` later, once `arm_unasked_rules` knows which rules are missing.
        let spec = candor_report::report_spec(&text);
        match candor_report::report_scanned_under(&text) {
            candor_report::KeyRead::Present(d) => out.scanned_under_facts.push((any_peeked, Some(d), spec)),
            candor_report::KeyRead::Absent => out.scanned_under_facts.push((any_peeked, None, spec)),
            candor_report::KeyRead::Corrupt => {
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
        //
        // ⟨0.28⟩ …AND THEN SPLIT BY WHICH ROW OF SPEC §2's TABLE IT IS, which is a SECOND question asked
        // of the same file, never an edit to the answer above. `report_judged_nothing` decides COVERAGE
        // on two other routes (candor-scan's chained join, `gate --report`), where a manifest-less
        // report must keep granting none — row 3's own instruction is *no manifest, no claim*. Flipping
        // it here to correct a LABEL would make every pre-⟨0.21⟩ report read as covered: a silent
        // under-report introduced by a disclosure fix. So the hedge stands and only its KEY is chosen,
        // by the disclosure-only `report_has_no_manifest`.
        if candor_report::report_judged_nothing(&text) {
            if candor_report::report_has_no_manifest(&text) {
                out.no_manifest.push(p);
            } else {
                out.judged_nothing.push(p);
            }
        }
    }
    out
}
