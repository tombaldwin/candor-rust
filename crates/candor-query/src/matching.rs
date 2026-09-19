//! Function-name matching (the tiered query rules) and small shared helpers.

use crate::*;

/// Match a function name against a query. EXACT-wins: if some candidate equals `q` verbatim, only
/// exact names match (`show foo` returns `foo`, not `foobar`); otherwise fall back to substring so a
/// partial query still searches. `exact_exists` is precomputed over the candidate set.
/// Name-match tier: 3 = exact, 2 = SEGMENT-SUFFIX (`Pricing::quote` matches `pricing::Pricing::quote`
/// but NOT `…::quote_bulk` — the boundary before the query must be `::`), 1 = substring, 0 = none.
/// Queries resolve at the BEST tier any candidate reaches, so a partial-but-segment-precise name no
/// longer silently widens to substring cousins (found by the speed-eval red-team: `whatif
/// Pricing::quote` seeded the blast radius from `quote_bulk` too).
pub(crate) fn match_tier(name: &str, q: &str) -> u8 {
    if q.is_empty() {
        // An empty query matches NOTHING. Without this `name.contains("")` is true for every function,
        // so an unset `$FN` (e.g. `whatif <prefix> "" Net`) selected the WHOLE graph and reported the
        // entire codebase as the blast radius with exit 0 — a false-clean answer for an unspecified edit.
        return 0;
    }
    if name == q {
        3
    } else if name.ends_with(q)
        && (name[..name.len() - q.len()].ends_with("::") || name[..name.len() - q.len()].ends_with('.'))
    {
        // the boundary before the query must be a SEGMENT boundary in the report's own naming —
        // `::` (Rust) or `.` (JVM/TS/Swift/fleet reports read by this same binary)
        2
    } else if name.contains(q) {
        1
    } else {
        0
    }
}

/// Split a qualified name on the report's own separator: `::` when present, else `.` — the one
/// query binary serves every engine's reports (Swift/JVM/TS names are dot-separated; the GRDB
/// interop probe found `map` lumping 731 Swift functions into `(root)`).
pub(crate) fn name_segments(name: &str) -> Vec<&str> {
    if name.contains("::") {
        name.split("::").collect()
    } else {
        name.split('.').collect()
    }
}

/// The best tier `q` reaches over the candidate names (0 = no match anywhere).
pub(crate) fn best_tier<'a>(names: impl Iterator<Item = &'a str>, q: &str) -> u8 {
    names.map(|n| match_tier(n, q)).max().unwrap_or(0)
}

pub(crate) fn q_match(name: &str, q: &str, tier: u8) -> bool {
    tier > 0 && match_tier(name, q) >= tier
}

/// The bare method name / declaring type of a qual or of a `dispatch:OWNER.member` detail. Used ONLY by
/// the dispatch-frontier, to match a confirmed reacher against a dispatch source's owner.
///
/// SPLIT ON THE LAST `::` **OR** `.`, WHICHEVER ENDS LATER, AND THE `::` HALF IS LOAD-BEARING. §4 pins
/// the dispatch DETAIL as dotted `owner.member` in every engine, but the other side of this comparison
/// is a REACHER's qual in its own engine's spelling — and this consumer reads reports from candor-java
/// and candor-ts (`p.Type.member`) *and* from candor-scan (`mod::Type::member`). Dot-only, a rust qual
/// `I7::op` has no dot at all, so `simple_method` returned the WHOLE STRING, `by_method` was keyed on
/// `I7::op`, and a lookup of `op` could never hit: `possibleViaUnknownDispatch` came back `[]` for every
/// rust-produced report. §3.1 rules that a dropped frontier entry is a false all-clear — a consumer
/// reads the empty list as "no function may reach the target through an unresolved dispatch" — which is
/// the same direction the dot-free guard in `callers.rs` exists to stop, one spelling over.
///
/// It was UNREACHABLE until SOUNDNESS R485 and that is why it survived: candor-scan's only dispatch
/// reason was the dot-free `dispatch:untyped cross-package receiver`, which takes the over-list branch
/// before this is consulted, so the dotted path was exercised exclusively by java/ts reports whose quals
/// are dotted anyway. R485 made the scanner emit `dispatch:<Trait>.<method>` — 28,311 such reasons over
/// 1,608 crates.io crates — and the dotted path became rust's normal case overnight. The comment one
/// function down still says "this engine writes no hierarchy sidecar of its own, so every hierarchy it
/// walks came from candor-java or candor-ts"; that is still true of the HIERARCHY and is exactly the
/// assumption that made the qual spelling look like someone else's problem.
///
/// A qual with NO separator (a free fn `go`) yields itself, so it can match `dispatch:X.go` without
/// being an override of anything. That is an OVER-LIST, which is the direction §3.1 requires: the
/// frontier asserts nothing into `transitive`, so a spurious entry costs precision and a dropped one is
/// a false all-clear. The dotted engines already behaved this way for an unqualified name.
fn last_sep(f: &str) -> Option<(usize, usize)> {
    let dot = f.rfind('.').map(|i| (i, 1));
    let colons = f.rfind("::").map(|i| (i, 2));
    match (dot, colons) {
        (Some(d), Some(c)) => Some(if d.0 > c.0 { d } else { c }),
        (a, b) => a.or(b),
    }
}

pub(crate) fn simple_method(f: &str) -> &str {
    last_sep(f).map(|(i, w)| &f[i + w..]).unwrap_or(f)
}

pub(crate) fn declaring_type(f: &str) -> &str {
    last_sep(f).map(|(i, _)| &f[..i]).unwrap_or(f)
}

/// The answer to a subtype question, ⟨0.26⟩ THREE-VALUED because the sidecar format now distinguishes
/// what it could not before. SPEC §2.2 makes the KEY SET the manifest: a producer emits a key for every
/// type it indexed, `[]` included, so a type with NO key is one the pass never looked at.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Subtype {
    Yes,
    No,
    /// The walk left the indexed set. NOT a failed test — an unasked question, and per §3.1 an
    /// unanswerable condition must be disclosed rather than scored as a failed one.
    Unanswerable,
}

/// Reflexive+transitive subtype test over the hierarchy sidecar.
///
/// This engine writes no hierarchy sidecar of its own (candor-scan emits none), so every hierarchy it
/// walks came from candor-java or candor-ts — which is exactly why the tri-state matters here: the
/// producer's completeness is not this engine's to assume. `hier.get(t)` returning `None` used to skip
/// the frame silently, so "indexed, no supertypes" and "never analysed" both fell through to `false`.
/// That is a positive claim about a type nobody analysed, and in the dispatch frontier it removes a
/// reacher from a disclosure with no diagnostic — measured in both producer engines as `[]` where the
/// control gives the dispatching function.
///
/// A POSITIVE DOMINATES: reaching `owner` down one branch is `Yes` even if another branch ran off the
/// indexed set. The relation is established, and an unknown branch cannot un-establish it.
pub(crate) fn subtype_of(ty: &str, owner: &str, hier: &BTreeMap<String, Vec<String>>) -> Subtype {
    if ty == owner {
        return Subtype::Yes;
    }
    let mut saw_unindexed = false;
    let mut seen: HashSet<&str> = HashSet::new();
    let mut stack: Vec<&str> = vec![ty];
    while let Some(t) = stack.pop() {
        let Some(sups) = hier.get(t) else {
            saw_unindexed = true;
            continue;
        };
        for s in sups {
            if s == owner {
                return Subtype::Yes;
            }
            if seen.insert(s.as_str()) {
                stack.push(s.as_str());
            }
        }
    }
    if saw_unindexed { Subtype::Unanswerable } else { Subtype::No }
}

/// The two-valued form used by the dispatch frontier. `Unanswerable` collapses to TRUE — over-list, never
/// drop — which is the direction §2.2 ⟨0.26⟩ requires and the opposite of what absence used to do. It is
/// also the direction this frontier already takes one rung up: with NO sidecar at all the subtype test is
/// unanswerable and the ruling is to over-list, so partial information must not be worse than none.
pub(crate) fn is_subtype_of(ty: &str, owner: &str, hier: &BTreeMap<String, Vec<String>>) -> bool {
    subtype_of(ty, owner, hier) != Subtype::No
}

/// Normalize a function path for layer derivation: a UFCS trait-impl path `<Type as Trait>::method`
/// becomes the impl's `Type` (whose module is the real layer), not the literal `<Type as Trait` token.
/// Other names pass through. (Lint-backend names use this `def_path_str` form; scan names don't.)
pub(crate) fn norm_name(name: &str) -> &str {
    match name.strip_prefix('<') {
        Some(rest) => rest.split(" as ").next().unwrap_or(rest),
        None => name,
    }
}

/// The number of leading `::` segments shared by EVERY function name — the codebase root, so the next
/// segment is the architectural "layer" (`pgman::app::…` → `app`; a multi-crate report → the crate).
pub(crate) fn common_prefix_len(names: &[&String]) -> usize {
    let mut prefix: Option<Vec<&str>> = None;
    for n in names {
        let segs: Vec<&str> = name_segments(norm_name(n)); // already a Vec — no redundant .to_vec()
        match &mut prefix {
            None => prefix = Some(segs),
            Some(p) => {
                let mut i = 0;
                while i < p.len() && i < segs.len() && p[i] == segs[i] {
                    i += 1;
                }
                p.truncate(i);
            }
        }
    }
    prefix.map(|p| p.len()).unwrap_or(0)
}

/// The layer a function belongs to: the MODULE segment after the common root prefix. A free function at
/// the root (`pgman::main`) has no module beyond the crate, so it buckets into `(root)` rather than
/// becoming its own pseudo-layer — the layer is `segs[prefix_len]` only when a leaf follows it.
pub(crate) fn layer_of(name: &str, prefix_len: usize) -> String {
    let segs: Vec<&str> = name_segments(norm_name(name));
    if prefix_len + 1 < segs.len() {
        segs[prefix_len].to_string()
    } else {
        "(root)".to_string()
    }
}

// ── small helpers ───────────────────────────────────────────────────────────────────────────────

pub(crate) fn sorted(v: &[String]) -> Vec<String> {
    let mut out = v.to_vec();
    out.sort();
    out
}

pub(crate) fn q_or(s: &str) -> &str {
    if s.is_empty() { "?" } else { s }
}

/// SOUNDNESS R507 (rust's half of [[R497]], fixed in candor-java `92994fd`) — resolve a ONE-FUNCTION
/// selector for a verb that ANSWERS ABOUT THE FUNCTION IT PICKED, and REFUSE rather than substitute a
/// subject. `Ok(entry)` is the single function the question is about; `Err(2)` means the verb must
/// return 2 having already said why.
///
/// MEASURED PRE-FIX on this engine (a two-type fixture scanned by candor-scan, both functions in one
/// report, `show` and `callers` given the IDENTICAL selector on the IDENTICAL report answering
/// correctly — the verb is the only thing that differed):
///
/// ```text
///   path   Provider::resolve_credentials Exec
///     -> creds::InstanceProvider::resolve_credentials does not perform Exec (inferred: ["Clock"])   exit 0
///   impact Provider::resolve_credentials
///     -> `creds::InstanceProvider::resolve_credentials` … 0 effectful functions transitively call it  exit 0
/// ```
///
/// `creds::Provider::resolve_credentials` performs `Exec` and has 2 transitive callers. Both answers
/// were therefore FALSE NEGATIVES ON THE REAL QUESTION, not merely answers about the wrong subject —
/// and a negative is a claim in this contract (§3.1), so a negative about a substituted subject is a
/// FABRICATED claim.
///
/// TWO STACKED DEFECTS, and neither half alone is the fix:
///
/// 1. **The match was not SEGMENT-ANCHORED.** The old resolution in both verbs was
///    `find(func == q).or_else(find(func.contains(q)))` — so `Provider::resolve_credentials` matched
///    INSIDE the longer identifier `InstanceProvider::resolve_credentials`. [`match_tier`] has encoded
///    this engine's anchored ladder since the speed-eval red-team, and `show`/`callers`/`whatif`/`fix`
///    all route through it; `path` and `impact` never did. **The helper existing is not the same as the
///    call site using it** — that was the premise this row's brief got wrong, and it is the reason the
///    class survived a cross-engine review that cited this very file.
/// 2. **With several candidates it PICKED instead of refusing.** Anchoring alone leaves genuine
///    ambiguity silent: on the same fixture `path resolve_credentials Exec` is a segment-anchored
///    (tier-2) match on BOTH functions, and pre-fix answered the same confident negative about the one
///    that sorts first. Refusing alone would reject a question that has exactly one right answer.
///
/// THE ASYMMETRY THAT LET IT SURVIVE: both verbs ALREADY refused at exit 2 when ZERO functions matched.
/// Only MANY was answered silently. The family has also already ruled this class one ARGUMENT over —
/// candor-swift grew a guard for `path`'s EFFECT argument after `path caller Fsz` printed "caller does
/// not perform Fsz" at exit 0, and this engine carries that guard too (see `cmd_path`'s KNOWN_EFFECTS
/// check). The guard was built for argument 2 and never for argument 1.
///
/// AMBIGUITY IS COUNTED OVER DISTINCT NAMES, not over rows: a report SET unions siblings, so the same
/// qual can arrive more than once and a row count would refuse a question that has one answer.
///
/// VERB-SWEEP BOUNDARY (audited by grepping every selector-taking verb in this crate, not drawn around
/// the verb the row was filed against): `show` (`show.rs`), `callers`/`callers_via_callgraph`
/// (`callers.rs`) and `whatif` (`policy.rs`) answer over the WHOLE best-tier set, so many matches WIDEN
/// an answer rather than substituting its subject; `fix` (`fix.rs`) is anchored and picks, but PREFERS a
/// tier match that performs the effect, so it cannot emit "nothing to hoist" while a sibling match
/// performs it — deliberately left alone, and it must stay byte-aligned with java/ts/swift. `reachable`,
/// `rewire`, `gains`, `receipt`, `diff`, `containment`, `blindspots`, `map`, `tour`, `gate`, `fix-gate`
/// and `unverified` take no function selector at all. `path` and `impact` were the only two call sites
/// of an unanchored `.func.contains(<selector>)` in this crate, and the only two that both pick
/// arbitrarily AND phrase their answer as a determined negative about the picked name.
pub(crate) fn select_one<'a>(entries: &'a [ReportEntry], q: &str, verb: &str) -> Result<&'a ReportEntry, i32> {
    let tier = best_tier(entries.iter().map(|e| e.func.as_str()), q);
    if tier == 0 {
        // UNCHANGED WORDING: this arm already existed in both verbs and is pinned by the suite.
        eprintln!("candor-query {verb}: no function matching '{q}'");
        return Err(2);
    }
    let mut hits: Vec<&ReportEntry> = Vec::new();
    for e in entries {
        if q_match(&e.func, q, tier) && !hits.iter().any(|h| h.func == e.func) {
            hits.push(e);
        }
    }
    if let [only] = hits[..] {
        return Ok(only);
    }
    // REFUSE, and NAME THE CANDIDATES: the remedy is that the user's next command is one of these lines
    // pasted back. Capped, because a one-segment selector over a large report can match hundreds and an
    // unreadable refusal is a refusal people work around.
    let mut names: Vec<&str> = hits.iter().map(|e| e.func.as_str()).collect();
    names.sort_unstable();
    eprintln!(
        "candor-query {verb}: `{q}` is AMBIGUOUS — {} functions match it equally well. This verb answers \
         ABOUT ONE function, so picking one would state a fact about a function you did not ask about. \
         Re-run with one of:",
        names.len()
    );
    for n in names.iter().take(12) {
        eprintln!("    {n}");
    }
    if names.len() > 12 {
        eprintln!("    … and {} more (`candor-query show {q}` lists them all)", names.len() - 12);
    }
    Err(2)
}
