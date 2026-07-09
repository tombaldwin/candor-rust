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

/// The bare method name / declaring type of a `mod.Type.member` qual (split on the last `.`). Used by
/// the dispatch-frontier to match a confirmed reacher against a `dispatch:OWNER.member` owner.
pub(crate) fn simple_method(f: &str) -> &str {
    f.rfind('.').map(|i| &f[i + 1..]).unwrap_or(f)
}

pub(crate) fn declaring_type(f: &str) -> &str {
    f.rfind('.').map(|i| &f[..i]).unwrap_or(f)
}

/// Reflexive+transitive subtype test over the hierarchy sidecar.
pub(crate) fn is_subtype_of(ty: &str, owner: &str, hier: &BTreeMap<String, Vec<String>>) -> bool {
    if ty == owner {
        return true;
    }
    let mut seen: HashSet<&str> = HashSet::new();
    let mut stack: Vec<&str> = vec![ty];
    while let Some(t) = stack.pop() {
        if let Some(sups) = hier.get(t) {
            for s in sups {
                if s == owner {
                    return true;
                }
                if seen.insert(s.as_str()) {
                    stack.push(s.as_str());
                }
            }
        }
    }
    false
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

/// (prefix, query, want_json) from `[prefix, query, 0|1]`.
pub(crate) fn three(args: &[String]) -> Option<(&str, &str, bool)> {
    match args {
        [a, b, c, ..] => Some((a.as_str(), b.as_str(), c == "1")),
        _ => None,
    }
}

/// (prefix, want_json) from `[prefix, 0|1]`.
pub(crate) fn two(args: &[String]) -> Option<(&str, bool)> {
    match args {
        [a, b, ..] => Some((a.as_str(), b == "1")),
        _ => None,
    }
}
