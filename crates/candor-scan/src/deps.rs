//! Everything Cargo/dependency-shaped: Cargo.toml reading (line-based on purpose),
//! dependency-report chaining (`--deps`), and the registry-source locator.

use crate::*;

/// candor-SCAN ONLY: builder-ENTRY points whose effect the typed classifier deliberately defers to a
/// terminal VERB. `duct::cmd!(...).run()` is the canonical case — `cmd!`/`cmd` only BUILD an Expression;
/// the spawn is at `.run()`/`.read()`/`.start()`. The DEEP engine types the receiver and catches the verb,
/// so candor-classify keeps the entry pure for PRECISION (lib.rs duct rule + its `cmd → None` test). But
/// the SYNTACTIC scanner can't type a builder chain — least of all through the `cmd!` MACRO whose result
/// is opaque — so the verb's effect is dropped and the program reads silent-pure (a real under-report
/// found by the real-world dynamic oracle; the same macro-blindness family as the log/tracing macros).
/// Classify the ENTRY as the crate's whole effect: a safe OVER-approximation (candor's never-under-report
/// bias), scoped to candor-scan so the deep engine stays precise. Both engines still agree on the
/// function's effect when the builder is actually run (the overwhelmingly common case).
pub(crate) fn scan_builder_entry_effect(_cr: &str, path: &str) -> Option<&'static str> {
    // A DATA TABLE the real-world oracle DRIVES: builder-chain ENTRY paths whose effect candor-classify
    // keys on a TERMINAL VERB the syntactic scanner can't reach (it can't type the chain). Add a row when
    // the oracle proves a verb-keyed crate under-reports here. Entries are exact ENTRY paths — NOT the
    // terminal verbs (those stay candor-classify's job for the typed deep engine, which stays precise).
    const ENTRIES: &[(&str, &str)] = &[
        // duct — `cmd!`/`sh!`/`cmd`/`sh` build; `.run()/.read()/.start()` execute (found 2026-06-17).
        ("duct::cmd", "Exec"),
        ("duct::sh", "Exec"),
        // ureq — `get/post/...` build a Request; `.call()` performs the Net (found 2026-06-17, net_ureq).
        ("ureq::get", "Net"),
        ("ureq::post", "Net"),
        ("ureq::put", "Net"),
        ("ureq::delete", "Net"),
        ("ureq::head", "Net"),
        ("ureq::patch", "Net"),
        ("ureq::request", "Net"),
        // sqlx — `query*()` build; `.execute()/.fetch_*()` round-trip (found 2026-06-17, recall corpus).
        ("sqlx::query", "Db"),
        ("sqlx::query_as", "Db"),
        ("sqlx::query_scalar", "Db"),
        ("sqlx::query_with", "Db"),
        ("sqlx::query_as_with", "Db"),
        // diesel — `sql_query()` builds raw SQL; `.execute()/.load()` round-trips (found 2026-06-17).
        ("diesel::sql_query", "Db"),
    ];
    ENTRIES.iter().find(|(p, _)| *p == path).map(|(_, eff)| *eff)
}

/// A loaded sibling-report function: the effects + literal surfaces a consumer's call inherits.
///
/// EVERY FIELD IS A SET, AND THAT IS LOAD-BEARING RATHER THAN TIDY. `apply_dep_fn` folds all eight
/// into `BTreeSet`s, so the join's result is invariant under the ORDER and the MULTIPLICITY of every
/// one of them — the serialisation carries no information the consumer can use.
///
/// It is now what makes `union_with` below TOTAL: two entries colliding under one index key are
/// unioned field-by-field rather than one being chosen or the key withdrawn (see the merge site and
/// candor-spec/ENTRY-COLLISION-DECISION.md). Being sets is what makes that union associative,
/// commutative and idempotent, so the index is invariant under the ORDER the reports are loaded in —
/// which is precisely the property java's `deny Fs` flip lacked: there, renaming a dep report file
/// changed the effect the consumer saw.
///
/// Historically this doc argued the same point about `PartialEq`: withdrawal exempted an identical
/// restatement, that exemption was decided by `PartialEq`, and derived on a `Vec` it would have been
/// order-sensitive — so two entries stating one claim in different orders read as a DISAGREEMENT and
/// the key was withdrawn, a purity claim under ⟨0.21⟩. That hazard is now structural rather than
/// guarded: nothing is withdrawn, so no serialisation accident can manufacture one.
#[derive(Clone, Default, PartialEq)]
pub(crate) struct DepFn {
    pub(crate) effects: BTreeSet<&'static str>,
    pub(crate) hosts: BTreeSet<String>,
    pub(crate) cmds: BTreeSet<String>,
    pub(crate) paths: BTreeSet<String>,
    pub(crate) tables: BTreeSet<String>,
    /// Blind crates the dep fn (transitively) reaches — its report's `invisible`. Carried across the join
    /// so a consumer inherits the disclosure (sweep [8]): else a dep that floored an unmodeled crate read
    /// as plain pure at the chain boundary, dropping the per-fn honesty caveat.
    pub(crate) invisible: BTreeSet<String>,
    /// Effects whose surface the dep fn left masking-incomplete — carried so a benign literal in the
    /// consumer can't mask the dep's invisible forbidden endpoint across the join (sweep [30]).
    pub(crate) incomplete: BTreeSet<&'static str>,
    /// The dep fn's own `unknownWhy` — carried so its `Unknown` does not arrive at the consumer with the
    /// REASON CLASS stripped off. Verbatim, not re-derived: the strings are already the canonical §4 ⟨0.7⟩
    /// vocabulary (a conforming producer wrote them) and `dispatch:<owner>.<member>` carries a NORMATIVE
    /// detail that a consumer needs to resolve overrides — re-deriving would destroy exactly that.
    pub(crate) unknown_why: BTreeSet<String>,
}

impl DepFn {
    /// Fold another entry for the SAME index key into this one — the family-wide entry-collision rule
    /// (candor-spec/ENTRY-COLLISION-DECISION.md), replacing withdrawal.
    ///
    /// EVERY FIELD, NOT JUST `effects`. That is the measured half of the decision: withdrawal discarded
    /// the whole entry, and the κ ledger (`invisible`) and the call edges (`calls`, applied by the
    /// caller) disagree far more often than the effects do — 30/37/273 and 57/120/326 against
    /// `inferred`'s 2/8/113 on candor-rust/pgman/ebman. A union that covered only `effects` would keep
    /// closing the purity claim while still dropping the disclosure that says what was not analyzed.
    ///
    /// EXHAUSTIVE BY DESTRUCTURING, so a field added later cannot silently opt out of the union and
    /// re-open the vein — the compiler names it here. This is the same reason the fields are sets: the
    /// invariant belongs in a place a later edit has to walk past, not in a comment.
    pub(crate) fn union_with(&mut self, other: &DepFn) {
        let DepFn { effects, hosts, cmds, paths, tables, invisible, incomplete, unknown_why } = other;
        self.effects.extend(effects.iter().copied());
        self.hosts.extend(hosts.iter().cloned());
        self.cmds.extend(cmds.iter().cloned());
        self.paths.extend(paths.iter().cloned());
        self.tables.extend(tables.iter().cloned());
        self.invisible.extend(invisible.iter().cloned());
        self.incomplete.extend(incomplete.iter().copied());
        self.unknown_why.extend(unknown_why.iter().cloned());
    }
}

/// The mutable per-function surface maps a chained dep entry is folded into. Exists only so
/// `apply_dep_fn` can be the ONE apply site without a ten-argument signature — every field is the
/// caller's own map, borrowed for the length of one fold.
pub(crate) struct DepSink<'a> {
    pub(crate) direct: &'a mut HashMap<String, BTreeSet<&'static str>>,
    pub(crate) hosts: &'a mut HashMap<String, BTreeSet<String>>,
    pub(crate) cmds: &'a mut HashMap<String, BTreeSet<String>>,
    pub(crate) paths: &'a mut HashMap<String, BTreeSet<String>>,
    pub(crate) tables: &'a mut HashMap<String, BTreeSet<String>>,
    pub(crate) incomplete: &'a mut HashMap<String, BTreeSet<&'static str>>,
    pub(crate) unknown_why: &'a mut HashMap<String, BTreeSet<String>>,
    pub(crate) blind_direct: &'a mut HashMap<String, BTreeSet<String>>,
    pub(crate) dep_invisible: &'a mut BTreeSet<String>,
    /// Callers whose `Unknown` arrived through this join with no reason the dependency recorded — the
    /// ONE case where a fn carries `Unknown` in `direct` and legitimately has no `unknownWhy`. The
    /// report writer's §4 invariant reads this to tell that case apart from a genuine marker gap; see
    /// `apply_dep_fn` and the `debug_assert` in `scan_one`.
    pub(crate) unknown_via_dep: &'a mut BTreeSet<String>,
}

/// THE ONE PLACE a chained dep entry's surfaces are charged to a calling function.
///
/// It exists because there were THREE, and they had drifted. candor-java shipped this vein's sibling
/// with `crossDepJoin` reproducing `inheritDepFn` line for line; the copies drifted until the ⟨0.19⟩
/// reason class reached the hand-off sites and NOT the ordinary call path, so a shipped,
/// conformance-pinned gate was silently inert (`6ab26e4`, fixed by deleting the copy). rust's three
/// copies had drifted the same way, in the disclosure direction: the cross-crate DROP-GLUE join
/// carried only `effects` + `paths`, and the dep-LAZY join carried no `invisible` and no `incomplete`.
///
/// Every surface belongs to the caller for the same reason the effects do. A dep's `Drop::drop` body
/// and a dep's lazy initializer both RUN in the calling function's scope — so the hosts they contact,
/// the tables they touch, the masking-incompleteness they leave (sweep [30]) and the blind crates they
/// reach (sweep [8]) are all as much the caller's as the effect is. A join that carries the effect and
/// drops the `incomplete` beside it lets a benign literal in the CONSUMER certify a surface the
/// dependency already declared uncertifiable.
pub(crate) fn apply_dep_fn(de: &DepFn, caller: &str, s: DepSink<'_>) {
    let ext = |m: &mut HashMap<String, BTreeSet<String>>, v: &BTreeSet<String>| {
        if !v.is_empty() {
            m.entry(caller.to_string()).or_default().extend(v.iter().cloned());
        }
    };
    for e in &de.effects {
        s.direct.entry(caller.to_string()).or_default().insert(e);
    }
    // AN `Unknown` MUST ARRIVE WITH THE MARKER THAT SAYS SO. SPEC §4 requires `unknownWhy` on any fn that
    // introduces `Unknown` DIRECTLY, and this join writes straight into `direct` — so the caller IS the
    // source, with no callee entry in this report to inherit a reason from. Without this the ⟨0.19⟩ class
    // was simply lost at the boundary: the gate's "an Unknown with no recorded reason is `unresolved`"
    // fallback then answered with the catch-all, so `deny E Unknown[dispatch]` / `[indirect]` — the
    // class-targeted policies the Unknown-ratchet is adopted with — read GREEN over a dependency whose own
    // report named the class. candor-ts `4dad22d` is the same drift one repo over.
    //
    // …BUT A REASON THE DEPENDENCY DID NOT GIVE IS NOT INVENTED HERE. The reasonless case used to be
    // filled with `callback:chained dependency declared Unknown without a reason`, on the argument that
    // `callback:` is the §4 vocabulary's residual bucket and that failing closed beats an empty field.
    // Both halves are wrong. `callback:` is not a residual bucket — §4 defines it as an unresolved
    // higher-order / owner-less INVOCATION, a claim about code, and nothing here observed one. And the
    // field is not a hole when empty: §6.2 states that "a function whose `Unknown` carries no recorded
    // reason is treated as `unresolved`" — the catch-all that stays inside `Unknown[*]` and
    // `Unknown[dynamic]` and that this engine's own under-gating lint tells a policy author to keep. So
    // the tag added no disclosure; it REPLACED the right class with a wrong one, and
    // `deny E Unknown[unresolved]` read GREEN on rust while firing on java, ts and swift over
    // byte-identical input. All three leave this to the §6.2 fallback (swift attaches a `dep:`
    // provenance pointer and documents that it projects to `unresolved` — same class, and its comment
    // gives the reason that decides this one: a class the chained arm carries and the single-tree
    // control does not is a divergence whichever way round it points).
    if de.effects.contains(&"Unknown") {
        if de.unknown_why.is_empty() {
            s.unknown_via_dep.insert(caller.to_string());
        } else {
            s.unknown_why.entry(caller.to_string()).or_default().extend(de.unknown_why.iter().cloned());
        }
    }
    ext(s.hosts, &de.hosts);
    ext(s.cmds, &de.cmds);
    ext(s.paths, &de.paths);
    ext(s.tables, &de.tables);
    // sweep [8]: inherit the dep fn's blind-crate disclosure so a consumer's pure verdict stays
    // qualified across the chain boundary (else the dep's floored reach reads as plain pure here).
    // `blind_direct` is per-fn; `dep_invisible` keeps the crate alive through the `global_blind`
    // filter, which only knows crates this scan saw a call into directly.
    ext(s.blind_direct, &de.invisible);
    s.dep_invisible.extend(de.invisible.iter().cloned());
    // sweep [30]: inherit masking-incompleteness so a benign literal here can't certify the dep's
    // invisible runtime endpoint.
    if !de.incomplete.is_empty() {
        s.incomplete.entry(caller.to_string()).or_default().extend(de.incomplete.iter().copied());
    }
}

/// The CANDOR_DEPS index: `crate#leaf`, `crate#tail2` and `crate#<full qual>` keys (UNAMBIGUOUS only
/// — a key two dep functions share is dropped, the same under-report-don't-guess rule as
/// `resolve_target`), plus
/// the covered crate set. A report whose producing version differs from this binary's is
/// DOWNGRADED to `Unknown` rather than silently trusted (spec §2.1).
#[derive(Default)]
pub(crate) struct DepIndex {
    pub(crate) by_key: HashMap<String, DepFn>,
    pub(crate) crates: std::collections::HashSet<String>,
    /// Crates whose loaded report(s) include at least one the §2.1 staleness gate REFUSED TO TRUST.
    ///
    /// `crates` answers "is there a report to join against" — a stale report still has entries, and
    /// downgrading them to `Unknown` is only possible if the join still fires, so the join gate must
    /// keep using `crates`. It must NOT also answer "is this crate COVERED for the κ ledger", because
    /// coverage is a claim that the report's SILENCE is informative: §2 chaining rule 3 makes an absent
    /// entry a purity claim, and the ledger exemption is what stops the blind-spot disclosure from
    /// saying otherwise. Granting that to a report we just refused to believe means every function the
    /// stale report does not mention reads as a confident purity claim, with no `invisible`, on the
    /// authority of the distrusted report — the candor-ts `651c9f9` shape.
    ///
    /// Conservative on conflict: if ANY report covering a crate is stale the crate is untrusted, since
    /// a fresh report for part of a crate cannot vouch for the part the stale one covered.
    pub(crate) untrusted: std::collections::HashSet<String>,
    /// ⟨0.21⟩ Crates whose loaded report DECLARES ITSELF INCOMPLETE — a non-empty `unanalyzed`, i.e. the
    /// producing scan named source it could not analyze. The same door as `untrusted`, one step earlier:
    /// staleness asks whether to believe what a report SAYS, completeness asks whether its SILENCE means
    /// anything. §2 chaining rule 3 turns silence into a purity claim, and a report that never read some of
    /// its own source is not entitled to that — chaining it was strictly WORSE than not chaining it, since
    /// the dependency's own gate refuses to certify itself over unanalyzed code (`--gate-json` exits 2 for
    /// precisely this) and the consumer was certifying one on its behalf.
    ///
    /// **THE TREATMENT DIFFERS FROM STALENESS, and the difference is the whole point.** A stale report's
    /// entries are assertions from a build this engine will not repeat, so they are DOWNGRADED to
    /// `Unknown`. An incomplete report's entries were derived from source it DID read and are TRUE, so
    /// they are kept exactly as they are — effects, literal surfaces, reason classes and all — and only
    /// COVERAGE is withheld. Strictly additive: an answered key still answers, an unanswered one falls
    /// back to the κ ledger's `invisible` hedge, and no effect is ever removed.
    ///
    /// Distinct from [`DepFn::incomplete`], which is the ⟨sweep 30⟩ MASKING-incompleteness of a single
    /// entry's literal surface. This one is a property of the whole REPORT.
    ///
    /// Ported from candor-ts `21277eb` (java `d1d3045`, swift `74cd8f1`); rust was the last engine
    /// gating coverage on staleness alone.
    ///
    /// CONSERVATIVE ON CONFLICT, and NOT subtracted the way swift's `incompletePkgs` is — for the same
    /// rust-specific reason `63bbe87` refused to align the fresh-vs-stale rule. See the fixture
    /// `a_crate_chained_both_complete_and_incomplete_keeps_its_blind_spot_disclosure`.
    pub(crate) incomplete_pkgs: std::collections::HashSet<String>,
    /// ⟨0.24⟩ Crates whose loaded report says it JUDGED NOTHING — ⟨0.21⟩ `analyzed.count` is 0 (see
    /// [`candor_report::claims_to_have_judged_nothing`] for the predicate and its shape table). **THE THIRD
    /// ANSWER TO "MAY THIS REPORT'S SILENCE SPEAK?"**, after staleness (§2.1, [`DepIndex::untrusted`]) and
    /// incompleteness (⟨0.21⟩, [`DepIndex::incomplete_pkgs`]). Coverage is the single mechanism that turns
    /// a report's silence into a purity claim, so all three live on the same door.
    ///
    /// A count-0 report bought a consumer MORE confidence than not chaining the package at all: the caller
    /// dropped out of `functions` — a ⟨0.21⟩ positive purity claim — with no `invisible`, no
    /// `coverage.uncovered`, no verdict caveat and no `--gate-json` coverage block, while the SAME scan
    /// with nothing chained disclosed all four. The empty report carries no effects, so this arm cannot
    /// itself trip a gate; what it deleted is the DISCLOSURE, which is why the fix restores that channel
    /// and does not manufacture a verdict.
    ///
    /// Treatment follows [`DepIndex::incomplete_pkgs`], not [`DepIndex::untrusted`]: the crate stays
    /// CHAINED (its keys are still asked, so a contradictory count-0-with-entries report still answers)
    /// and only coverage is withheld. Entries are never touched, so this is strictly additive — it can
    /// only add a hedge, never remove an effect.
    ///
    /// CONSERVATIVE ON CONFLICT, like its two neighbours here and UNLIKE candor-swift, which subtracts
    /// (`unjudgedPkgs.subtract(coveredPkgs)`) so a crate chained once as judged and once as judged-nothing
    /// keeps the earned claim. Both readings are defensible — swift's is that a count-0 report makes no
    /// claim in either direction and so should be a no-op beside a real one; this engine's is the one its
    /// other two refusal sets already take, that a partial report cannot vouch for the part another report
    /// covered, and the cost of being wrong is one extra hedge rather than one missing one. Pinned by
    /// `a_crate_chained_both_judged_and_unjudged_keeps_its_blind_spot_disclosure`; flipping it is a
    /// one-line subtraction at the end of the load, exactly as `incomplete_pkgs` documents.
    pub(crate) judged_nothing_pkgs: std::collections::HashSet<String>,
    /// ⟨typeSurface.returns⟩ `{crate}#{fn qual}` -> `{crate}#{type qual}`, merged across every loaded
    /// report. Lets a consumer type a receiver bound from a dependency FACTORY, which is otherwise
    /// impossible: a pure factory is absent from the report entirely, so there is no entry on which a
    /// return type could have been carried. Same never-guess rule as `by_key` — two reports publishing
    /// DIFFERENT types for one fn id drop the key rather than pick, because a wrong receiver type
    /// FABRICATES where a missing one merely misses.
    pub(crate) returns: HashMap<String, String>,
}

/// ⟨0.21⟩ Does this dep report DECLARE ITSELF INCOMPLETE — does its `unanalyzed` manifest name source the
/// producing scan could not analyze? See [`DepIndex::incomplete_pkgs`] for why it costs coverage.
///
/// **ABSENT means COMPLETE, PRESENT-BUT-MALFORMED means INCOMPLETE.** `candor_report` omits the key
/// entirely when the manifest is empty (`skip_serializing_if = "Vec::is_empty"`), so absence is this
/// engine's own way of saying "I read everything" — reading it as incompleteness would hedge every report
/// ever written, including every one in this repo's own fixtures. Anything else — a non-empty array, a
/// `null`, a string, an object — is a completeness claim that cannot be read, and a claim that cannot be
/// read is not a claim: it fails CLOSED. So the only two shapes that buy coverage are an absent key and an
/// explicitly empty array: a denylist of proven-safe shapes, never an allowlist of rejected ones.
///
/// (candor-java `d1d3045` reads malformed the same way. candor-ts `scan.mjs:625` and candor-swift
/// `Deps.swift` both fail OPEN on a non-array — `Array.isArray(d.unanalyzed) && …` and `as? [Any] ?? []`
/// — which is a row for those repos, not a divergence any conformance PART can see, since no conforming
/// producer emits the shape.)
pub(crate) fn declares_itself_incomplete(v: &serde_json::Value) -> bool {
    match v.get("unanalyzed") {
        None => false,
        Some(u) => !u.as_array().is_some_and(|a| a.is_empty()),
    }
}

pub(crate) fn load_dep_reports(spec: Option<&str>) -> DepIndex {
    let mut idx = DepIndex::default();
    let Some(spec) = spec else { return idx };
    // Canonical-path dedup: the same report loaded twice used to self-collide on every key and be
    // dropped as 'ambiguous', silently killing the chain (review: --deps + CANDOR_DEPS=.candor/deps
    // — the natural combination — did exactly that). The entry union has since made that particular
    // accident harmless (unioning an entry with itself is the identity), so this is now a COST guard
    // rather than a correctness one: parsing and folding every report twice is pure waste. Kept for
    // that reason, and because a reader should not have to re-derive which of the two it is.
    // Directories walk RECURSIVELY: --deps writes one subdirectory per name@version.
    let mut files: Vec<std::path::PathBuf> = Vec::new();
    let mut seen_files: std::collections::HashSet<std::path::PathBuf> = std::collections::HashSet::new();
    let mut push_file = |f: std::path::PathBuf, files: &mut Vec<std::path::PathBuf>| {
        let canon = std::fs::canonicalize(&f).unwrap_or(f);
        if seen_files.insert(canon.clone()) {
            files.push(canon);
        }
    };
    // SPLIT ON WHITESPACE, COLON *AND* COMMA — SPEC §3.4 says `deps`/`CANDOR_DEPS` is "whitespace-separated
    // report paths", and this engine split on `:` alone. A two-path spec therefore arrived as ONE token,
    // matched no file, and rust chained NOTHING while printing "CANDOR_DEPS entry not found, skipped" and
    // the ordinary uncovered-package hedge — indistinguishable, in the report, from having been handed no
    // reports at all.
    //
    // FOUND THROUGH A WAIVER THAT NAMED THE WRONG CAUSE. Conformance PART 26's `stale_beside` arm passes
    // `"<trusted> <stale>"`, and rust's waiver for it read "the key is withdrawn, the effect is gone and
    // the package is re-declared uncovered" — a precise diagnosis of a mechanism that was not running. The
    // arm had been measuring rust-with-nothing-chained since it was written. Measured, same fixture:
    //
    //     space-separated   go -> []                    nothing chained (this bug)
    //     colon-separated   go -> ['Exec','Unknown']     the entry union, working
    //
    // NOT the cardinal sin — the package is disclosed `invisible` and stderr names the skip, so the
    // failure is loud. It is a conformance defect, and it was hiding behind a waiver that read as a
    // soundness finding, which is worse than either on its own.
    //
    // candor-java has documented `space/colon/comma` since its loader was written; matching it here makes
    // the family convention one rule rather than three. Colon and comma are supersets of the spec, not
    // substitutes for it: an existing colon-separated spec keeps working, which is what every rust
    // fixture and this engine's own `--deps` output use.
    for tok in spec.split([' ', '\t', '\n', '\r', ':', ',']).filter(|t| !t.is_empty()) {
        let p = Path::new(tok);
        if p.is_dir() {
            for e in walkdir::WalkDir::new(p).into_iter().filter_map(Result::ok) {
                let f = e.path();
                let name = f.file_name().and_then(|n| n.to_str()).unwrap_or("");
                if f.is_file() && name.ends_with(".json") && !name.contains("callgraph") {
                    push_file(f.to_path_buf(), &mut files);
                }
            }
        } else if p.is_file() {
            push_file(p.to_path_buf(), &mut files);
        } else {
            // ⟨0.27⟩ SPEC §2: A CONFIGURED DEP THAT CANNOT BE READ IS UNEVALUABLE, NOT REDUCED COVERAGE.
            // Skipping it continued the run, and the caller of that dep then serialised `inferred: []` —
            // a ⟨0.21⟩ purity claim, published in the REPORT, about a function whose dependency the
            // operator configured precisely so it would not be one. The coverage note travels on stderr;
            // the claim travels in the artifact a chained consumer and `gate --report` actually read.
            //
            // java and swift already refused here; this engine and ts continued. One config, two
            // meanings, on a condition CI meets routinely — a dep not yet scanned, a path that moved.
            eprintln!("candor-scan: CANDOR_DEPS names {tok} but it is not a readable file or directory —");
            eprintln!("        failing (exit 2, unevaluable). A configured dep that is not there is not");
            eprintln!("        reduced coverage: its callers would serialise `inferred: []`, which is a");
            eprintln!("        purity claim about code this scan never saw. Scan that dependency, or");
            eprintln!("        remove it from the `deps` config / CANDOR_DEPS.");
            // THROUGH THE SINK-AWARE EXIT, not `process::exit`. A raw exit leaves `--gate-json -` with
            // ZERO bytes on stdout and a file sink holding the armed PLACEHOLDER rather than this
            // reason — the machine channel getting nothing on the very cause the operator configured.
            // Measured against java on the same input: 280 bytes there, 0 here.
            crate::gate::exit2_refused(format!("configured dependency {tok} is not a readable file or directory"));
        }
    }
    let my_version = format!("scan-{}", env!("CARGO_PKG_VERSION"));
    // NOTE: there is no `ambiguous` set any more. The ENTRY index unions colliding keys and withdraws
    // nothing, so nothing needs remembering as poisoned. `ret_ambiguous` below is the RETURN-TYPE index,
    // which is a different question and still withdraws: guessing a receiver type fabricates a call
    // target, whereas the fallback is half 1's disclosure rather than silence.
    let mut ret_ambiguous: std::collections::HashSet<String> = std::collections::HashSet::new();
    for f in &files {
        // ⟨0.27⟩ THE SAME RULE AS THE TOKEN ARM ABOVE, and it was missing here. SPEC §2 binds the
        // configured case with one sentence — a dep path that "does not exist OR CANNOT BE READ MUST
        // exit 2, naming it" — and the 0.27 work implemented only the first half. A path that resolved
        // to a file which then failed to open, or held malformed JSON, was SKIPPED at exit 0, so the
        // caller of that dep serialised `inferred: []`: the ⟨0.21⟩ purity claim the token arm exists to
        // prevent, reached by a different door.
        //
        // Found by the 0.27 go/no-go panel, which read this engine's own changelog claim ("a configured
        // dep that cannot be read now refuses") and tested it rather than believing it. java and swift
        // refused on both halves already; this made the family 2-v-2 on a MUST.
        let Ok(text) = std::fs::read_to_string(f) else {
            eprintln!("candor-scan: CANDOR_DEPS report {} could not be read —", f.display());
            eprintln!("        failing (exit 2, unevaluable). A configured dep this scan cannot read is");
            eprintln!("        not reduced coverage: its callers would serialise `inferred: []`, a purity");
            eprintln!("        claim about code this scan never saw.");
            crate::gate::exit2_refused(format!("configured dependency report {} could not be read", f.display()));
        };
        let Ok(v) = serde_json::from_str::<serde_json::Value>(&text) else {
            eprintln!("candor-scan: CANDOR_DEPS report {} is not valid JSON —", f.display());
            eprintln!("        failing (exit 2, unevaluable). Same reason as an unreadable one: a report");
            eprintln!("        that cannot be parsed makes no claim, and continuing would publish one.");
            crate::gate::exit2_refused(format!("configured dependency report {} is not valid JSON", f.display()));
        };
        // v0.2+ envelope or the v0.1 bare array; the producing version comes from the envelope.
        let version = v.pointer("/candor/version").and_then(|x| x.as_str()).unwrap_or("");
        let stale = version != my_version;
        // ⟨0.21⟩ …and a report that names source it could not analyze grants no coverage either (see
        // `DepIndex::incomplete_pkgs`). STALENESS OUTRANKS IT — a report this engine already refuses to
        // believe cannot be trusted about its own completeness either, and it has lost its coverage
        // anyway, so its `unanalyzed` buys it nothing beyond the downgrade it already has and the two
        // stderr disclosures stay disjoint instead of both naming the same crate. That precedence lives
        // in ONE place, `cover`'s branch order, and is pinned by
        // `a_stale_report_is_not_also_counted_incomplete`. It was ALSO written here as `!stale && …` —
        // and the mutation round showed that conjunct failing nothing, because the `else if` already
        // decides it. A guard that cannot be detected needs a test; a guard that costs nothing needs
        // deleting (standing bar item 8c), and this one was the second.
        let incomplete = declares_itself_incomplete(&v);
        let Some(fns) = v.get("functions").and_then(|x| x.as_array()).or_else(|| v.as_array()) else { continue };
        // ⟨0.24⟩ …and neither does a report that JUDGED NOTHING (`analyzed.count: 0`) — the third answer
        // to "may this report's silence speak?", ranked BELOW the other two for the same reason
        // incompleteness ranks below staleness: the three stderr disclosures stay disjoint and the
        // precedence lives in one place, `cover`'s branch order. Keyed on the INTEGER and never on
        // `fns.is_empty()` — the two are the same shape on the wire and only the count separates a facade
        // from a legitimately all-pure crate; `fns` enters ONLY as SPEC §2's manifest-less third row.
        let judged_nothing = candor_report::claims_to_have_judged_nothing(&v, !fns.is_empty());
        // ⟨typeSurface.returns⟩ Merge this report's published return types. GATED ON `!stale` for the
        // same reason the effects are: a report from a different producer version is not trusted, and a
        // type surface read off one would silently key the consumer through a claim we just refused to
        // believe. (The trap costs three false measurements a day when forgotten — dep reports must be
        // regenerated with the binary under test.)
        if !stale {
            if let Some(ts) = v.pointer("/typeSurface/returns").and_then(|x| x.as_object()) {
                for (k, val) in ts {
                    let Some(t) = val.as_str() else { continue };
                    if ret_ambiguous.contains(k) {
                        continue;
                    }
                    match idx.returns.get(k) {
                        // Two reports claim DIFFERENT types for one fn id — drop it. Never guess a
                        // receiver type: a wrong one fabricates, a missing one falls back to half 1.
                        Some(prev) if prev != t => {
                            idx.returns.remove(k);
                            ret_ambiguous.insert(k.clone());
                        }
                        Some(_) => {}
                        None => {
                            idx.returns.insert(k.clone(), t.to_string());
                        }
                    }
                }
            }
        }
        // THE SECOND CONJUNCT OF SPEC §6.2 ⟨0.24⟩'s CONTRIBUTION RULE: an entry's `Unknown` is
        // UNACCOUNTED only when NEITHER its own tags NOR its published `calls` chain explain it.
        //
        // `unknownWhy` is DIRECT-ONLY by design (§4: a reason names a site in the function's OWN body),
        // so a dep entry whose `Unknown` is purely INHERITED correctly carries no reason — and it is
        // indistinguishable, field for field, from one nothing accounted for. Charging both to the
        // catch-all is the NAIVE form §6.2 names and the changelog calibrates: "contributing `unresolved`
        // to one whose `Unknown` is correctly classified at the callee is the mirror fabrication. A fix
        // that trades one for the other is not a fix."
        //
        // MEASURED HERE BEFORE THIS PASS EXISTED, on TRUSTED reports, with exactly the control §6.2
        // prescribes — a fresh dependency whose `Unknown` is explained once via its own tag and once via
        // a `calls` edge. rust passed the tag arm and failed the edge arm: scanning candor-scan over its
        // own 173-report dep tree, 8 caller functions were charged `unresolved`, and all 8 trace to three
        // `syn` entries (`parse::F::parse2`, `mac::Macro::parse_body_with`,
        // `attr::Attribute::parse_nested_meta`) whose `Unknown` syn's OWN `calls` chain explains 2–5 hops
        // down, at `error::Error::new`, as `ambiguous:same-name local defs` — class `dispatch`, not
        // `unresolved`. A fabricated class, not a missing one.
        //
        // The resolution is a least fixpoint over the report's own graph, and it is a DENYLIST: it carves
        // out the entries a reason demonstrably reaches and leaves everything else on the conservative
        // catch-all. An allowlist here — "explain only the shapes I thought of" — would silently return
        // every shape it forgot to the fabrication.
        //
        // GATED ON `!stale` for the same reason the type surface is: §2.1 refuses to believe a distrusted
        // report's effects, so it must not believe its `calls` either. A stale entry keeps the reasonless
        // treatment, which is the whole point of the downgrade.
        let mut chain_why: std::collections::HashMap<String, BTreeSet<String>> = Default::default();
        if !stale {
            let mut own: std::collections::HashMap<&str, BTreeSet<String>> = Default::default();
            let mut edges: std::collections::HashMap<&str, Vec<&str>> = Default::default();
            for e in fns {
                let Some(q) = e.get("fn").and_then(|x| x.as_str()) else { continue };
                let why: BTreeSet<String> = e
                    .get("unknownWhy")
                    .and_then(|x| x.as_array())
                    .into_iter()
                    .flatten()
                    .filter_map(|s| s.as_str().map(str::to_string))
                    .collect();
                if !why.is_empty() {
                    own.insert(q, why);
                }
                let cs: Vec<&str> = e
                    .get("calls")
                    .and_then(|x| x.as_array())
                    .into_iter()
                    .flatten()
                    .filter_map(|s| s.as_str())
                    .collect();
                if !cs.is_empty() {
                    edges.insert(q, cs);
                }
            }
            // Only worth a fixpoint if the report records BOTH reasons and edges to carry them along.
            if !own.is_empty() && !edges.is_empty() {
                let mut acc: std::collections::HashMap<&str, BTreeSet<String>> = own.clone();
                // Bounded like every other propagation here: the set only grows, over a finite node set,
                // so the loop terminates; the cap is belt-and-braces against a pathological report.
                for _ in 0..64 {
                    let mut moved = false;
                    for (q, cs) in &edges {
                        let mut merged = acc.get(q).cloned().unwrap_or_default();
                        let before = merged.len();
                        for c in cs {
                            if let Some(w) = acc.get(c) {
                                merged.extend(w.iter().cloned());
                            }
                        }
                        if merged.len() != before {
                            acc.insert(q, merged);
                            moved = true;
                        }
                    }
                    if !moved {
                        break;
                    }
                }
                // Keep only the entries this pass ADDS — one whose own tag already answers is untouched,
                // so the verbatim direct reason still wins and this can only ever fill a blank.
                for (q, w) in acc {
                    if !own.contains_key(q) {
                        chain_why.insert(q.to_string(), w);
                    }
                }
            }
        }
        // The crate(s) a report COVERS, for the §7.14 ledger exemption (§2 chaining rule 3): the
        // AUTHORITATIVE claim is the envelope's `package` (or the JVM-shape `packages`) field — an
        // EMPTY report ({functions: []}) is an all-pure purity claim for that package, covered and
        // never a κ blind spot. Keyed on the envelope so the exemption doesn't depend on the file
        // NAME or on any join firing (found live: an empty chained report named outside the
        // `….<crate>.scan.json` shape still drew a "classifier doesn't cover" line here while candor-java and
        // candor-ts correctly stayed quiet). A hyphenated package name also registers in Rust ident
        // form (`dep-c` → `dep_c`), the form call paths carry.
        //
        // A STALE report registers the crate here TOO — the join must still fire so its entries can be
        // charged `Unknown` — but every name it registers is ALSO recorded `untrusted`, so the ledger
        // exemption below does not treat the distrusted report's silence as a purity claim.
        //
        // ⟨0.21⟩ …AND SO DOES A REPORT THAT DECLARES ITSELF INCOMPLETE, into `incomplete_pkgs`. THE ONE
        // registration closure is what makes that a fix rather than a no-op wearing a fix's clothes:
        // candor-java found coverage anchored TWICE (a file-level envelope registration and an entry-hash
        // fallback) and gating one of them failed NOTHING. rust has FOUR registration sites — the
        // envelope `package`, the JVM-shape `packages[]`, the filename fallback and each entry's `hash`
        // prefix — and all four go through here, so there is exactly one place to gate. Counted, not
        // assumed: `cover(` is the only writer of `crates`/`untrusted`/`incomplete_pkgs`/
        // `judged_nothing_pkgs` in this file, and `coverage_has_exactly_one_anchor_and_exactly_one_consumer`
        // fails if a fifth site ever appears.
        //
        // ⟨0.24⟩ …AND SO DOES A REPORT THAT JUDGED NOTHING, into `judged_nothing_pkgs`. It rides the SAME
        // closure for exactly the reason the trap names: a count-0 report reaches the entry loop with no
        // entries, so the `hash`-prefix anchor never fires for it and gating only that one would have been
        // a no-op wearing a fix's clothes — the envelope `package` and the FILENAME fallback are the two
        // anchors that actually carry this shape. Gating the closure gates all four at once.
        let cover = |name: String, idx: &mut DepIndex| {
            if stale {
                idx.untrusted.insert(name.clone());
            } else if incomplete {
                idx.incomplete_pkgs.insert(name.clone());
            } else if judged_nothing {
                idx.judged_nothing_pkgs.insert(name.clone());
            }
            idx.crates.insert(name);
        };
        for pkg in v
            .get("package")
            .and_then(|x| x.as_str())
            .into_iter()
            .chain(v.get("packages").and_then(|x| x.as_array()).into_iter().flatten().filter_map(|x| x.as_str()))
            .map(str::to_string)
            .collect::<Vec<_>>()
        {
            cover(pkg.replace('-', "_"), &mut idx);
            cover(pkg, &mut idx);
        }
        // Filename fallback (`report.<crate>.scan.json`) for pre-`package` reports, and the default
        // crate attribution for entries carrying no `hash` prefix.
        let file_crate = f
            .file_name()
            .and_then(|n| n.to_str())
            .and_then(|n| n.strip_suffix(".scan.json"))
            .and_then(|n| n.rsplit('.').next())
            .map(str::to_string);
        // Register the crate at FILE level too: an all-pure crate's report has zero entries, and that
        // emptiness is its honest claim — the crate is covered, not invisible.
        if let Some(c) = &file_crate {
            cover(c.clone(), &mut idx);
        }
        for e in fns {
            let Some(qual) = e.get("fn").and_then(|x| x.as_str()) else { continue };
            let krate = e
                .get("hash")
                .and_then(|x| x.as_str())
                .and_then(|h| h.split_once('#'))
                .map(|(c, _)| c.to_string())
                .or_else(|| file_crate.clone());
            let Some(krate) = krate else { continue };
            cover(krate.clone(), &mut idx);
            let mut de = DepFn::default();
            if stale {
                de.effects.insert("Unknown"); // §2.1: a different producer version is not trusted
                // NO REASON, DELIBERATELY — §6.2 classes it `unresolved`, which is what java, ts and
                // swift all land on and what the spec prescribes for an Unknown with no recorded reason.
                // This carried `callback:chained report from a different producer version`, which classes
                // `indirect`: a claim that some higher-order invocation could not be resolved, when what
                // actually happened is that a report failed a version check. The distrust is a property
                // of the REPORT, so there is no call site to name and no single-tree control to agree
                // with — three engines and §6.2 are the evidence. The prose lives on stderr instead
                // (see the disclosure below), the channel ts and swift already use for this and the one
                // rust had nothing on.
                //
                // ⟨0.24⟩ SPEC §4 NOW REGISTERS `dep:<hash>` AND `dep-stale:<pkg>` AS PERMANENT KINDS —
                // not migration ones — and §6.2 holds swift's per-ENTRY form up as the correct shape.
                // The blocker on following swift here is no longer the spec, it is the SHIPPED
                // conformance harness: PART 10's `CANON` is still the four kinds with `ambiguous` in a
                // tolerated set, and `dep-stale` is in neither, so emitting it scores a hard DIVERGE
                // today. Reasonless staleness is still classed correctly on this engine — `apply_dep_fn`
                // routes it through `unknown_via_dep`, which CONTRIBUTES `unresolved` at the source
                // (scan.rs) rather than waiting for the join's absence arm. What the missing token costs
                // is only the REPORT's ability to say "Unknown, and nothing explained it" beside a
                // reason it does have; see the residual note at that contribution site.
            } else {
                for s in e.get("inferred").and_then(|x| x.as_array()).into_iter().flatten() {
                    if let Some(s) = s.as_str() {
                        // unknown vocabulary (a future spec's effect) is honestly Unknown
                        de.effects.insert(candor_classify::cap_from_name(s).unwrap_or("Unknown"));
                    }
                }
                let strs = |k: &str| -> BTreeSet<String> {
                    e.get(k)
                        .and_then(|x| x.as_array())
                        .into_iter()
                        .flatten()
                        .filter_map(|s| s.as_str().map(str::to_string))
                        .collect()
                };
                de.hosts = strs("hosts");
                de.cmds = strs("cmds");
                de.paths = strs("paths");
                de.tables = strs("tables");
                de.invisible = strs("invisible"); // sweep [8]: carry the blind-crate disclosure across the join
                // The reason class travels with the `Unknown` it explains. Verbatim — see `DepFn`.
                de.unknown_why = strs("unknownWhy");
                // …and when the entry's own tags say nothing, the reason its published `calls` chain
                // reaches (the fixpoint above). Still verbatim: these are the dependency's own canonical
                // §4 strings, moved along an edge the dependency itself published, never re-derived.
                // Only fills a blank — an entry with a direct tag keeps exactly the bytes it shipped.
                if de.unknown_why.is_empty() {
                    if let Some(w) = chain_why.get(qual) {
                        de.unknown_why = w.clone();
                    }
                }
                // sweep [30]: carry masking-incompleteness (mapped to the static effect alphabet).
                for s in e.get("incomplete").and_then(|x| x.as_array()).into_iter().flatten() {
                    if let Some(eff) = s.as_str().and_then(candor_classify::cap_from_name) {
                        de.incomplete.insert(eff);
                    }
                }
            }
            // THREE key shapes per entry: leaf, qualified tail2, and the FULL qual. The full qual is
            // the precise one — `deplib#sync::Client::fetch` — and it exists so a join that already
            // KNOWS its target exactly can ask for exactly that instead of settling for tail2, where
            // `sync::Client` and `mock::Client` are the same key. Nothing published a qualified type id
            // before because the index could not answer one (DEP-RECEIVER-TYPING-DESIGN.md
            // "BLOCKING PREREQUISITE"); that is what this key is for.
            //
            // ADDITIVE, and the DEDUP is what makes it so: for a 1- or 2-segment qual the full qual IS
            // the leaf/tail2 string, so pushing it again would collide with itself and the never-guess
            // rule below would REMOVE the key that already worked — a silent under-report manufactured
            // by an "additive" change. A ≥3-segment full qual can never collide with another entry's
            // leaf (1 segment) or tail2 (2 segments), so no existing key is put at risk either.
            let mut keys: Vec<String> = Vec::with_capacity(3);
            let push_key = |k: String, keys: &mut Vec<String>| {
                if !keys.contains(&k) {
                    keys.push(k);
                }
            };
            push_key(format!("{krate}#{}", qual.rsplit("::").next().unwrap_or(qual)), &mut keys);
            if let Some(t2) = tail2(qual) {
                push_key(format!("{krate}#{t2}"), &mut keys);
            }
            push_key(format!("{krate}#{qual}"), &mut keys);
            for k in keys {
                // TWO ENTRIES UNDER ONE KEY ARE UNIONED — never withdrawn, never picked between.
                // Decided family-wide in candor-spec/ENTRY-COLLISION-DECISION.md after measuring all four
                // engines against three real `.candor/deps` trees, and this is the arm that was unsound.
                //
                // WHAT THIS REPLACES: withdrawal. Two entries disagreeing under one key used to REMOVE the
                // key, so the calling function vanished from `functions` entirely — which under ⟨0.21⟩ is
                // a positive claim of purity, the cardinal sin. Named live instance on one of the
                // most-depended-upon crates there is:
                //
                //     hyper#client::conn::http1::Builder::handshake  ['Log'] @0.14.32  vs  [] @1.9.0
                //
                // Both hyper versions are legitimately in the tree (cargo permits semver-major
                // duplicates), rust withdrew the key, and the consumer read it as ABSENT = pure.
                //
                // WHY UNION IS NOT A HEDGE HERE. The objection to unioning is that two entries under one
                // key may be two DIFFERENT functions that merely collide, so the union charges one's
                // effects to the other — a fabrication, and that is exactly why this withdrew. Measured,
                // that objection describes NOTHING in the corpus: every one of the 123 disagreements
                // across candor-rust/pgman/ebman is one function at two VERSIONS of one crate
                // (thiserror-impl 1.x/2.x, rustix 0.38/1.1, http 0.2/1.4, hyper 0.14/1.9). For a version
                // pair the union is not an over-approximation, it is the CORRECT answer: both bodies are
                // in the build, the package-scoped key cannot express which one a caller resolves to, so
                // the runtime may execute either and their union is simply what the key means. Total cost
                // of the union across all three corpora: SEVEN effect-items, to close 123 purity claims.
                //
                // WITHDRAWING COST MORE THAN THE EFFECTS, which is what made this a whole-entry fix rather
                // than an effects one. The key carries the κ ledger and the call edges too, and both
                // disagree far more often than `inferred` does (`invisible` 30/37/273, `calls` 57/120/326
                // against `inferred`'s 2/8/113). Withdrawal discarded all of it at once — including the
                // coverage disclosure whose entire job is to say what was NOT analyzed. `direct` and
                // `unknownWhy` union at zero measured cost in every corpus: one side is always a subset of
                // the other, so the union is just "the one that said something".
                //
                // THE IDENTICAL-RESTATEMENT EXEMPTION IS NOW SUBSUMED rather than special-cased. It used to
                // be a separate `prev == &de` branch guarding against a dep directory holding two copies of
                // one report; unioning a set with itself is already the identity, so the case needs no arm.
                // That is also why the `PartialEq`-on-`BTreeSet` reasoning in `DepFn`'s doc comment is no
                // longer load-bearing for soundness — a producer serialising a vector in a different order
                // can no longer manufacture a "disagreement" that withdraws a key.
                idx.by_key.entry(k).or_default().union_with(&de);
            }
        }
    }
    // THE STALENESS DISCLOSURE, on the channel that can carry prose. The §2.1 downgrade puts `Unknown`
    // on every entry of an untrusted report and withholds its coverage, but until now rust said so
    // NOWHERE a reader could see: no report field names the report, and the reason field is the wrong
    // place (a raw reason is gate vocabulary — see the `stale` arm above). candor-ts and candor-swift
    // both print exactly this line; rust was the only engine silent about it.
    if !idx.untrusted.is_empty() {
        let mut names: Vec<&str> = idx.untrusted.iter().map(String::as_str).collect();
        names.sort_unstable();
        eprintln!(
            "candor-scan: {} chained dependency report(s) were produced by a DIFFERENT engine build — \
             downgraded to Unknown and granted no coverage (§2.1): {}",
            names.len(),
            names.join(", ")
        );
    }
    // ⟨0.21⟩ The completeness disclosure, on the same channel and for the same reason: the withheld
    // coverage is visible in the report as an `invisible` hedge and a κ-ledger row, but nothing there
    // says WHY a crate with a chained report is being hedged. `cargo test` and the four-way conformance
    // suite read the report and the exit code, not this (standing-bar item 7g).
    if !idx.incomplete_pkgs.is_empty() {
        let mut names: Vec<&str> = idx.incomplete_pkgs.iter().map(String::as_str).collect();
        names.sort_unstable();
        eprintln!(
            "candor-scan: {} chained dependency report(s) declare source they could not analyze (⟨0.21⟩ \
             `unanalyzed`) — their entries are KEPT unchanged, but they grant NO coverage, so a key they \
             do not answer discloses instead of reading pure: {}",
            names.len(),
            names.join(", ")
        );
    }
    // ⟨0.24⟩ The judged-nothing disclosure, third on the same channel. The REMEDY is named because a
    // count-0 report is almost always a build artifact rather than a real answer — a facade crate of
    // `pub use`es, a platform stub, an aggregation target — and the reader's next question is "so what do
    // I do about it", which the other two lines answer implicitly (rebuild / fix the parse error) and
    // this one does not.
    if !idx.judged_nothing_pkgs.is_empty() {
        let mut names: Vec<&str> = idx.judged_nothing_pkgs.iter().map(String::as_str).collect();
        names.sort_unstable();
        eprintln!(
            "candor-scan: {} chained dependency report(s) judged NOTHING (⟨0.24⟩ `analyzed.count` is 0, \
             absent-with-no-functions, or unreadable) — they grant NO coverage, so a call into them \
             discloses exactly as if the report had not been chained at all. Usually a facade or \
             re-export-only crate: scan what it re-exports: {}",
            names.len(),
            names.join(", ")
        );
    }
    idx
}

// ── shared Cargo.toml line primitives (line-based on purpose — no toml dependency) ────────────────
// The ONE place table-header and scalar parsing live, so a manifest-syntax quirk (`[ spaced ]`
// headers, a trailing `# comment`) is handled once across the three readers below rather than
// drifting between them.

/// A `[section]` header line → its inner name, surrounding spaces tolerated (`[ workspace ]` →
/// "workspace"); None for any non-header line.
pub(crate) fn toml_section(line: &str) -> Option<&str> {
    let l = line.trim();
    Some(l.strip_prefix('[')?.strip_suffix(']')?.trim())
}

/// A scalar `key = "value"` / `key = value` on this line — `key` matched as the WHOLE key (then `=`),
/// the value quote-trimmed and an out-of-quotes trailing `# comment` stripped. None if not this key.
pub(crate) fn toml_scalar<'a>(line: &'a str, key: &str) -> Option<&'a str> {
    let rest = line.trim().strip_prefix(key)?.trim_start().strip_prefix('=')?.trim();
    Some(if let Some(q) = rest.strip_prefix('"') {
        q.split('"').next().unwrap_or(q)
    } else {
        rest.split('#').next().unwrap_or(rest).trim()
    })
}

/// Dependency names declared by EVERY Cargo.toml under the scan root (a workspace's members each
/// declare their own — review: reading only the root manifest left member-declared deps invisible
/// to the κ ledger on the most common project layout), normalized to crate-root form (`-` -> `_`).
/// dev-/build-dependencies are the harness's and the build script's universe, not the crate's
/// runtime one — excluded, like tests/ and build.rs.
pub(crate) fn cargo_deps(dir: &str) -> (std::collections::HashSet<String>, HashMap<String, String>) {
    let mut out = std::collections::HashSet::new();
    let mut renames = HashMap::new();
    // Honour the SAME nested-package rule as the source walk (filter_entry above): a subdir with its
    // own Cargo.toml is a different package whose deps are ITS universe, not this crate's — scan_target
    // scans it separately. Without this, a nested fixture/path-dep's deps polluted the parent's κ
    // ledger (the source walk skips the nested sources, so the two had drifted out of agreement).
    for entry in walkdir::WalkDir::new(dir)
        .into_iter()
        .filter_entry(|e| {
            if e.depth() == 0 || !e.file_type().is_dir() {
                return true;
            }
            let name = e.file_name().to_str().unwrap_or("");
            if name == "target" || (name.starts_with('.') && name != "." && name != "..") {
                return false;
            }
            !e.path().join("Cargo.toml").is_file()
        })
        .filter_map(Result::ok)
    {
        let p = entry.path();
        if p.file_name().and_then(|n| n.to_str()) != Some("Cargo.toml") {
            continue;
        }
        if let Ok(text) = std::fs::read_to_string(p) {
            cargo_toml_deps(&text, &mut out, &mut renames);
        }
    }
    (out, renames)
}

/// One manifest's dependency names, all four header forms: `[dependencies]` /
/// `[workspace.dependencies]` / `[target.….dependencies]` sections, and the table-header
/// declarations `[dependencies.name]` / `[target.….dependencies.name]` (review: the old
/// `ends_with("dependencies]")` gate made the header-form branch unreachable — a table-header
/// dep was invisible to the ledger, execution-verified).
pub(crate) fn cargo_toml_deps(
    text: &str,
    out: &mut std::collections::HashSet<String>,
    renames: &mut HashMap<String, String>,
) {
    // A dependency RENAME (`tui-common = { package = "tb-tui-common" }`) means the manifest KEY is
    // what the code imports while the registry/report knows the REAL package — without the map,
    // --deps scanned the real crate and the join/ledger missed it under the key (found live on
    // ebman: tui_common stayed "invisible" with its report sitting right there).
    // Match `package` only as a KEY (`{ … package = "real" }`), not as a substring of a dependency
    // KEY (`my-package = "1.2"` previously parsed its own version as a rename target) or a value:
    // `package` must sit at a token boundary and be followed by `=`.
    let pkg_re = |l: &str| -> Option<String> {
        let bytes = l.as_bytes();
        let mut search = 0;
        while let Some(rel) = l[search..].find("package") {
            let i = search + rel;
            let boundary = i == 0 || matches!(bytes[i - 1], b'{' | b',' | b' ' | b'\t');
            if boundary {
                if let Some(rest) = l[i + "package".len()..].trim_start().strip_prefix('=') {
                    if let Some(rest) = rest.trim_start().strip_prefix('"') {
                        return rest.split('"').next().map(|s| s.replace('-', "_"));
                    }
                }
            }
            search = i + "package".len();
        }
        None
    };
    let mut in_deps = false;
    let mut header_key: Option<String> = None; // the `[dependencies.name]` we're inside, if any
    for line in text.lines() {
        let l = line.trim();
        if let Some(inner) = toml_section(line) {
            let harness = inner.contains("dev-dependencies") || inner.contains("build-dependencies");
            in_deps = !harness && (inner == "dependencies" || inner.ends_with(".dependencies"));
            header_key = None;
            if !harness && !in_deps {
                let name = inner
                    .rfind(".dependencies.")
                    .map(|i| &inner[i + ".dependencies.".len()..])
                    .or_else(|| inner.strip_prefix("dependencies."));
                if let Some(name) = name {
                    if !name.is_empty() && !name.contains('.') {
                        let key = name.trim_matches('"').replace('-', "_");
                        out.insert(key.clone());
                        header_key = Some(key);
                    }
                }
            }
            continue;
        }
        if l.is_empty() || l.starts_with('#') {
            continue;
        }
        // inside a `[dependencies.name]` table: a `package = "real"` line is the rename
        if let Some(key) = &header_key {
            if l.starts_with("package") {
                if let Some(real) = pkg_re(l) {
                    renames.insert(key.clone(), real);
                }
            }
            continue;
        }
        if !in_deps {
            continue;
        }
        if let Some(name) = l.split('=').next() {
            let name = name.trim().trim_matches('"');
            if !name.is_empty() {
                let key = name.replace('-', "_");
                // A rename only appears in an INLINE TABLE value (`key = { … package = "real" }`),
                // never as a bare `package = "0.1"` (which is a dependency NAMED package) — so search
                // only inside the braces.
                if let Some(brace) = l.find('{') {
                    if let Some(real) = pkg_re(&l[brace..]) {
                        if real != key {
                            renames.insert(key.clone(), real);
                        }
                    }
                }
                out.insert(key);
            }
        }
    }
}

/// The cargo registry source roots (`~/.cargo/registry/src/<index-hash>/`), where unbuilt
/// dependency sources live. CARGO_HOME is honoured.
pub(crate) fn dirs_cargo_registry_src() -> Vec<std::path::PathBuf> {
    let home = std::env::var("CARGO_HOME")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|_| Path::new(&std::env::var("HOME").unwrap_or_default()).join(".cargo"));
    std::fs::read_dir(home.join("registry").join("src"))
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.is_dir())
        .collect()
}

pub(crate) fn read_crate_name(root: &Path) -> Option<String> {
    let txt = std::fs::read_to_string(root.join("Cargo.toml")).ok()?;
    let mut in_package = false;
    for line in txt.lines() {
        if let Some(section) = toml_section(line) {
            in_package = section == "package"; // only [package]'s `name` is the crate name
            continue;
        }
        // `name` inside `[package]` only (a `name =` in `[[bin]]`/`[dependencies]` is not the crate name).
        if in_package {
            if let Some(v) = toml_scalar(line, "name") {
                return Some(v.replace('-', "_"));
            }
        }
    }
    None
}

/// The string entries of `key = [ ... ]` inside `[table]` — line-based (the manifest subset that
/// matters), multi-line arrays included. No TOML dependency, same trade as the parsers above.
pub(crate) fn toml_string_array(txt: &str, table: &str, key: &str) -> Vec<String> {
    let (mut in_table, mut collecting) = (false, false);
    let mut out = Vec::new();
    for line in txt.lines() {
        let l = line.trim();
        if !collecting {
            if let Some(section) = toml_section(line) {
                in_table = section == table;
                continue;
            }
        }
        if !in_table {
            continue;
        }
        let rest = if let Some(r) = l.strip_prefix(key) {
            let r = r.trim_start();
            let Some(r) = r.strip_prefix('=') else { continue };
            collecting = true;
            r
        } else if collecting {
            l
        } else {
            continue;
        };
        let mut parts = rest.split('"');
        parts.next();
        while let Some(s) = parts.next() {
            out.push(s.to_string());
            if parts.next().is_none() {
                break;
            }
        }
        if rest.contains(']') {
            collecting = false;
        }
    }
    out
}

/// True if the manifest declares a `[workspace]` table at all (distinct from "has members"): a
/// workspace root with zero RESOLVED members must warn, not silently fall through to a single-crate
/// scan whose nested-package filter then prunes every member into an empty report.
pub(crate) fn has_workspace_table(root: &Path) -> bool {
    std::fs::read_to_string(root.join("Cargo.toml"))
        .map(|t| t.lines().any(|l| l.trim() == "[workspace]"))
        .unwrap_or(false)
}

/// Member directories of the root manifest's `[workspace]`, joined to `root`, honouring `exclude`,
/// expanding globs (a bare `*` = root's immediate children, `prefix/*` = a dir's children), and
/// DEDUPLICATED (a member listed explicitly AND matched by a glob otherwise scans/prints twice).
/// Empty when there is no `members` key. A `*`-pattern this simple matcher can't expand is WARNED,
/// never silently dropped (a dropped member yields a vacuous gate, the §6.2 forbidden state).
pub(crate) fn workspace_members(root: &Path) -> Vec<String> {
    let Ok(txt) = std::fs::read_to_string(root.join("Cargo.toml")) else { return Vec::new() };
    let members = toml_string_array(&txt, "workspace", "members");
    if members.is_empty() {
        return Vec::new();
    }
    let exclude = toml_string_array(&txt, "workspace", "exclude");
    // Expand a `<base>/*` (base "" for a bare `*`) to its child dirs carrying a Cargo.toml.
    let expand = |base: &str| -> Vec<String> {
        let dir = if base.is_empty() { root.to_path_buf() } else { root.join(base) };
        let mut found: Vec<String> = std::fs::read_dir(dir)
            .into_iter()
            .flatten()
            .filter_map(Result::ok)
            .filter(|e| e.path().join("Cargo.toml").is_file())
            .map(|e| {
                let n = e.file_name().to_string_lossy().into_owned();
                if base.is_empty() { n } else { format!("{base}/{n}") }
            })
            .collect();
        found.sort();
        found
    };
    let mut rels: Vec<String> = Vec::new();
    for m in members {
        if m == "*" {
            rels.extend(expand(""));
        } else if let Some(base) = m.strip_suffix("/*") {
            rels.extend(expand(base));
        } else if m.contains('*') {
            eprintln!("candor-scan: workspace member glob `{m}` is not a trailing `*` — not expanded; \
                       scan its crates directly or list them explicitly");
        } else if root.join(&m).join("Cargo.toml").is_file() {
            rels.push(m);
        }
    }
    rels.retain(|m| !exclude.iter().any(|e| m == e || m.starts_with(&format!("{e}/"))));
    rels.sort();
    rels.dedup();
    rels.into_iter().map(|m| root.join(m).to_string_lossy().into_owned()).collect()
}
