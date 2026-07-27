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
/// one of them — the serialisation carries no information the consumer can use. The index's
/// never-guess rule withdraws a key two entries DISAGREE under and exempts a restatement, and that
/// exemption is decided by `PartialEq`: derived on a `Vec` it is order- and duplicate-sensitive, so
/// two entries stating one claim in different orders read as a disagreement, the key is withdrawn,
/// and under ⟨0.21⟩ the consumer's silence is a purity claim — the cardinal sin `6f2210c` closed,
/// re-opened for any producer that happens to serialise a vector differently. Stating it in the TYPE
/// rather than in the comparison is what stops a field added later from re-opening it silently.
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
    // Canonical-path dedup: the same report loaded twice would self-collide on every key and be
    // dropped as 'ambiguous', silently killing the chain (review: --deps + CANDOR_DEPS=.candor/deps
    // — the natural combination — did exactly that). Directories walk RECURSIVELY: --deps writes
    // one subdirectory per name@version.
    let mut files: Vec<std::path::PathBuf> = Vec::new();
    let mut seen_files: std::collections::HashSet<std::path::PathBuf> = std::collections::HashSet::new();
    let mut push_file = |f: std::path::PathBuf, files: &mut Vec<std::path::PathBuf>| {
        let canon = std::fs::canonicalize(&f).unwrap_or(f);
        if seen_files.insert(canon.clone()) {
            files.push(canon);
        }
    };
    for tok in spec.split(':').filter(|t| !t.is_empty()) {
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
            eprintln!("candor-scan: CANDOR_DEPS entry not found, skipped: {tok}");
        }
    }
    let my_version = format!("scan-{}", env!("CARGO_PKG_VERSION"));
    let mut ambiguous: std::collections::HashSet<String> = std::collections::HashSet::new();
    let mut ret_ambiguous: std::collections::HashSet<String> = std::collections::HashSet::new();
    for f in &files {
        let Ok(text) = std::fs::read_to_string(f) else {
            eprintln!("candor-scan: CANDOR_DEPS report unreadable, skipped: {}", f.display());
            continue;
        };
        let Ok(v) = serde_json::from_str::<serde_json::Value>(&text) else {
            eprintln!("candor-scan: CANDOR_DEPS report unparsable, skipped: {}", f.display());
            continue;
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
        // assumed: `cover(` is the only writer of `crates`/`untrusted`/`incomplete_pkgs` in this file.
        let cover = |name: String, idx: &mut DepIndex| {
            if stale {
                idx.untrusted.insert(name.clone());
            } else if incomplete {
                idx.incomplete_pkgs.insert(name.clone());
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
                // rust had nothing on; PART 10 makes a `dep-stale:`-shaped kind a hard divergence, so
                // following them into the field itself is not open to this engine.
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
                if ambiguous.contains(&k) {
                    continue;
                }
                // Not the `entry` API: a collision REMOVES the key (and remembers it as ambiguous),
                // so the present-vs-absent branches move `k` into different maps — clippy's map_entry
                // rewrite (insert-or-modify in place) can't express the remove-on-collision.
                #[allow(clippy::map_entry)]
                if let Some(prev) = idx.by_key.get(&k) {
                    // THE NEVER-GUESS RULE IS ABOUT DISAGREEMENT, NOT REPETITION. Withdrawing a key two
                    // entries share is right when they say DIFFERENT things — there is no way to choose and a
                    // wrong choice fabricates. It is wrong when they are IDENTICAL: nothing is ambiguous, and
                    // withdrawing turns a resolved effect into an ABSENT one, which under ⟨0.21⟩ is a positive
                    // purity claim. MEASURED before the fix: one report chained gives the consumer `['Exec']`;
                    // the SAME report chained TWICE gives ABSENT with no coverage hedge — a cardinal sin
                    // reachable by the most ordinary accident there is, a dep directory holding two copies of
                    // one report. Found by candor-swift's fresh-vs-stale fixture, which hit it and flagged it
                    // for rust and java to check; java is CLEAN (its entry conflict is last-wins, so it keeps
                    // an answer), rust withdrew.
                    //
                    // DELIBERATELY ONLY THE IDENTICAL CASE. Entries that AGREE ON EFFECTS but differ in their
                    // literal surfaces are the majority of real collisions (1536 of 2041 on pgman's dep tree,
                    // 2255 of 3276 on ebman's) and merging those is arguably right too — but measured, it
                    // makes 24 of pgman's 200 functions and 108 of ebman's 544 newly carry `Unknown`, because
                    // the entries being recovered are ones the dependency itself could not resolve. That is a
                    // 12-20% disclosure increase and a design decision, not a tail on a bug fix; it is filed
                    // in the work queue with these numbers rather than taken here.
                    //
                    // AND "IDENTICAL" MEANS THE CLAIM, NOT ITS SERIALISATION — which is why every `DepFn`
                    // field is a `BTreeSet` (see the type). Derived `PartialEq` on a `Vec` compares
                    // element-wise and order-sensitively, so two entries stating one claim in different
                    // orders, or one of them restating a host, read as a DISAGREEMENT here and the key is
                    // withdrawn: the same cardinal sin this exemption exists to close, surviving for any
                    // producer that happens to order a vector differently. `apply_dep_fn` folds every field
                    // into a set, so set equality is not a relaxation of never-guess — two set-equal entries
                    // are operationally indistinguishable and there is nothing to choose between.
                    if prev == &de {
                        continue; // the same claim restated — keep the entry, withdraw nothing
                    }
                    idx.by_key.remove(&k); // two dep fns DISAGREE under one key — drop it, never guess
                    ambiguous.insert(k);
                } else {
                    idx.by_key.insert(k, de.clone());
                }
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
