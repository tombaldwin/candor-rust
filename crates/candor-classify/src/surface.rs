//! The SURPRISE heuristic — the single (or top-N) most surprising transitive reach in a crate (the
//! cold-repo hook). SHARED so candor-scan's scan-time note and candor-query's `tour` verb CANNOT
//! drift: this crate hosts the one implementation, both callers delegate here.
//!
//! Fully deterministic — pure call-graph + name analysis, NO LLM. A CANDIDATE is a function `F` that
//! INHERITS an effect `E` (E ∈ inferred[F] but E ∉ direct[F]); we BFS to the nearest local direct SOURCE
//! `S` and score by how surprising the reach is (a benign-named function reaching a scary effect). The
//! find is never *wrong*: `candor path` re-derives the chain and the gate is ground truth. When nothing
//! clears the bar the caller emits an honest "nothing hidden" fallback — never a manufactured surprise.
//!
//! Generic over the effect element type (`E: AsRef<str>`), so BOTH candor-scan's
//! `BTreeSet<&'static str>` maps AND candor-query's `BTreeSet<String>` maps drive the SAME code.

use std::collections::{BTreeSet, HashMap, VecDeque};

/// Name tokens that read as local / pure / config — a function whose leaf is named like this reaching a
/// scary effect is the core surprise signal.
const BENIGN: &[&str] = &[
    "settings", "config", "conf", "options", "opts", "util", "utils", "helper", "helpers", "model",
    "models", "dto", "entity", "format", "fmt", "parse", "get", "load", "new", "default", "validate",
    "valid", "render", "view", "build", "builder", "item", "entry", "record", "state", "context",
    "ctx", "info", "meta", "data", "value", "node", "field", "name", "key", "id", "path", "kind",
    "type", "status", "check", "init", "setup",
];

/// Name tokens that are effect-suggestive — a function in/near an effect-flavored context reaching that
/// effect is EXPECTED, not surprising, so we EXCLUDE it.
const EFFECTY: &[&str] = &[
    "fetch", "http", "https", "client", "api", "sync", "request", "req", "download", "upload", "query",
    "sql", "store", "save", "persist", "connect", "conn", "socket", "send", "recv", "read", "write",
    "open", "file", "fs", "io", "net", "tcp", "udp", "dns", "url", "host", "port", "cmd", "command",
    "shell", "process", "proc", "exec", "spawn", "env", "clock", "time", "now", "rand", "random",
    "log", "logger", "trace", "db",
];

/// Split a qualified name (or a leaf) into lowercase tokens on `_`, `::` and camelCase boundaries.
fn tokenize(name: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut prev_lower = false;
    for ch in name.chars() {
        if ch == '_' || ch == ':' {
            if !cur.is_empty() {
                out.push(std::mem::take(&mut cur));
            }
            prev_lower = false;
            continue;
        }
        // camelCase boundary: a lower/digit followed by an upper starts a new token.
        if ch.is_uppercase() && prev_lower && !cur.is_empty() {
            out.push(std::mem::take(&mut cur));
        }
        cur.push(ch.to_ascii_lowercase());
        prev_lower = ch.is_lowercase() || ch.is_ascii_digit();
    }
    if !cur.is_empty() {
        out.push(cur);
    }
    out
}

/// The leaf (final `::` segment) of a qualified name.
fn leaf(qual: &str) -> &str {
    qual.rsplit("::").next().unwrap_or(qual)
}

/// The module portion of a qualified name (everything before the leaf).
fn module_of(qual: &str) -> &str {
    match qual.rfind("::") {
        Some(i) => &qual[..i],
        None => "",
    }
}

fn has_token(name: &str, lexicon: &[&str]) -> Option<String> {
    tokenize(name).into_iter().find(|t| lexicon.contains(&t.as_str()))
}

/// Salience of an effect — the boundary/security-relevant effects a reviewer cares about score higher.
fn salience(effect: &str) -> i64 {
    match effect {
        "Net" | "Exec" | "Db" | "Ipc" => 5,
        "Fs" | "Env" => 3,
        "Clock" | "Log" | "Rand" => 1,
        _ => 0,
    }
}

fn hops_factor(hops: usize) -> i64 {
    match hops {
        1 => 2,
        2..=4 => 3,
        5..=6 => 2,
        _ => 1, // ≥7 (hops is always ≥1 for an inherited reach)
    }
}

/// A scored candidate reach. Generic over the caller's effect-string type via owned `String` fields, so
/// the SAME struct serves candor-scan (`&'static str` maps) and candor-query (`String` maps).
pub struct Find {
    pub func: String,
    pub effect: String,
    pub hops: usize,
    pub source: String,
    /// "file:line" of the effect SOURCE, resolved from the caller's `loc` map ("" when absent).
    pub source_loc: String,
    /// The benign leaf token that made the reach surprising ("" when the leaf isn't benign-named).
    pub benign_token: String,
    pub score: i64,
}

fn is_test(qual: &str) -> bool {
    qual.contains("::tests::") || qual.contains("::test::")
}

/// Does a set contain the effect `e`? Membership via `AsRef<str>` so it works over both
/// `BTreeSet<&'static str>` and `BTreeSet<String>`.
fn set_has<E: AsRef<str>>(set: &BTreeSet<E>, e: &str) -> bool {
    set.iter().any(|x| x.as_ref() == e)
}

/// BFS from `func` over `calls` (follow callees, shortest hops) to the nearest function `S` with
/// `effect` ∈ direct[S]. Returns (hops≥1, S). Only traverses through callees that transitively carry
/// the effect, so the frontier stays on-effect (matches `candor path`'s walk).
fn nearest_source<'a, E: AsRef<str>>(
    func: &'a str,
    effect: &str,
    direct: &'a HashMap<String, BTreeSet<E>>,
    inferred: &'a HashMap<String, BTreeSet<E>>,
    calls: &'a HashMap<String, BTreeSet<String>>,
) -> Option<(usize, &'a str)> {
    let mut seen: BTreeSet<&str> = BTreeSet::new();
    let mut q: VecDeque<(&str, usize)> = VecDeque::new();
    seen.insert(func);
    q.push_back((func, 0));
    while let Some((cur, d)) = q.pop_front() {
        // A direct source found at distance d≥1 is the nearest (BFS). The start `func` itself is an
        // INHERITED reach (E ∉ direct[func]) so it never matches at d==0.
        if d >= 1 && direct.get(cur).is_some_and(|s| set_has(s, effect)) {
            return Some((d, cur));
        }
        if let Some(cs) = calls.get(cur) {
            for c in cs {
                let cc = c.as_str();
                if !seen.contains(cc) && inferred.get(cc).is_some_and(|s| set_has(s, effect)) {
                    seen.insert(cc);
                    q.push_back((cc, d + 1));
                }
            }
        }
    }
    None
}

/// Compute the top-`top_n` most surprising reaches, most-surprising first. DEDUPED by function — each
/// function appears at most once (its single highest-scoring reach). The list is empty when nothing
/// clears the bar (the caller decides whether to emit the honest fallback vs nothing, using
/// [`any_effectful`]).
///
/// Ranking (the tie-break, applied to the whole candidate pool before the per-function dedup + take):
/// score DESC → hops ASC → qualified name ASC. With `top_n == 1` the result is BYTE-IDENTICAL to the old
/// scan-time `best_find` (conformance PART 4f + the candor-scan tests pin this).
pub fn best_finds<E: AsRef<str>>(
    inferred: &HashMap<String, BTreeSet<E>>,
    direct: &HashMap<String, BTreeSet<E>>,
    calls: &HashMap<String, BTreeSet<String>>,
    loc: &HashMap<String, String>,
    top_n: usize,
) -> Vec<Find> {
    // Deterministic iteration: sort quals ascending so the tie-break (qual ascending) is stable and
    // HashMap order never leaks into the result.
    let mut quals: Vec<&String> = inferred.keys().collect();
    quals.sort();

    let mut cands: Vec<Find> = Vec::new();

    for f in quals {
        let inf = &inferred[f];
        if is_test(f) {
            continue;
        }
        let f_leaf = leaf(f);
        let f_mod = module_of(f);
        // EXCLUDE the whole function if its leaf OR module reads effecty — its reach is obvious.
        if has_token(f_leaf, EFFECTY).is_some() || has_token(f_mod, EFFECTY).is_some() {
            continue;
        }
        let empty = BTreeSet::new();
        let dir = direct.get(f).unwrap_or(&empty);
        // Candidate effects: inherited (in inferred, not direct), not Unknown. Sorted ascending so the
        // per-function winner is deterministic.
        let mut effects: Vec<String> = inf
            .iter()
            .map(|e| e.as_ref().to_string())
            .filter(|e| e != "Unknown" && !set_has(dir, e))
            .collect();
        effects.sort();
        for e in effects {
            let sal = salience(&e);
            if sal == 0 {
                continue;
            }
            let Some((hops, s)) = nearest_source(f, &e, direct, inferred, calls) else {
                continue; // no LOCAL direct source — nothing to show
            };
            let benign = has_token(f_leaf, BENIGN);
            let benignity = if benign.is_some() { 3 } else { 1 };
            let crossing = if module_of(s) != f_mod { 2 } else { 1 };
            let score = sal * benignity * hops_factor(hops) * crossing;
            if score == 0 {
                continue;
            }
            cands.push(Find {
                func: f.clone(),
                effect: e,
                hops,
                source: s.to_string(),
                source_loc: loc.get(s).cloned().unwrap_or_default(),
                benign_token: benign.unwrap_or_default(),
                score,
            });
        }
    }

    // Rank the whole pool: score DESC, hops ASC, qual ASC. Quals were iterated ascending and effects
    // ascending, so on a full tie the first-pushed (smallest qual) candidate sorts first — matching the
    // old `best_find`'s "keep the earliest winner on an exact tie".
    cands.sort_by(|a, b| {
        b.score
            .cmp(&a.score)
            .then_with(|| a.hops.cmp(&b.hops))
            .then_with(|| a.func.cmp(&b.func))
    });

    // DEDUP by function — each function appears at most once (its single highest-scoring reach, which is
    // the first one seen in the ranked order). Then take up to `top_n` distinct functions.
    let mut seen_fns: BTreeSet<String> = BTreeSet::new();
    let mut out: Vec<Find> = Vec::new();
    for c in cands {
        if out.len() >= top_n {
            break;
        }
        if seen_fns.insert(c.func.clone()) {
            out.push(c);
        }
    }
    out
}

/// Is the crate EFFECTFUL — does ANY function carry a real (non-Unknown) effect? Governs whether the
/// caller emits the honest "nothing hidden" fallback (effectful, but nothing clears the bar) vs nothing
/// at all (a genuinely effect-free crate).
pub fn any_effectful<E: AsRef<str>>(inferred: &HashMap<String, BTreeSet<E>>) -> bool {
    inferred.values().any(|s| s.iter().any(|e| e.as_ref() != "Unknown"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn set(items: &[&'static str]) -> BTreeSet<&'static str> {
        items.iter().copied().collect()
    }
    fn sset(items: &[&str]) -> BTreeSet<String> {
        items.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn tokenize_splits_all_boundaries() {
        assert_eq!(tokenize("settings::Settings::needsUpdate"), vec!["settings", "settings", "needs", "update"]);
        assert_eq!(tokenize("api_client::latest_version"), vec!["api", "client", "latest", "version"]);
    }

    #[test]
    fn benign_deep_inherited_beats_shallow_effecty() {
        // Graph:
        //   settings::Settings::load  (benign leaf "load")  -inherits-> Net, 3 hops
        //     -> core::refresh -> core::sync_state -> net_layer::do_send (direct Net)
        //   api::fetch  (effecty leaf "fetch") -inherits-> Net, 1 hop  (EXCLUDED — effecty)
        //     -> net_layer::do_send
        let mut direct: HashMap<String, BTreeSet<&'static str>> = HashMap::new();
        let mut inferred: HashMap<String, BTreeSet<&'static str>> = HashMap::new();
        let mut calls: HashMap<String, BTreeSet<String>> = HashMap::new();
        let loc: HashMap<String, String> = HashMap::new();

        direct.insert("net_layer::do_send".into(), set(&["Net"]));
        inferred.insert("net_layer::do_send".into(), set(&["Net"]));

        inferred.insert("core::sync_state".into(), set(&["Net"]));
        calls.insert("core::sync_state".into(), sset(&["net_layer::do_send"]));

        inferred.insert("core::refresh".into(), set(&["Net"]));
        calls.insert("core::refresh".into(), sset(&["core::sync_state"]));

        // benign candidate: settings::Settings::load, 3 hops to source.
        inferred.insert("settings::Settings::load".into(), set(&["Net"]));
        calls.insert("settings::Settings::load".into(), sset(&["core::refresh"]));

        // effecty candidate: api::fetch, 1 hop — must be excluded by the EFFECTY leaf/module.
        inferred.insert("api::fetch".into(), set(&["Net"]));
        calls.insert("api::fetch".into(), sset(&["net_layer::do_send"]));

        assert!(any_effectful(&inferred));
        let got = best_finds(&inferred, &direct, &calls, &loc, 1);
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].func, "settings::Settings::load");
        assert_eq!(got[0].effect, "Net");
        assert_eq!(got[0].hops, 3);
        assert_eq!(got[0].source, "net_layer::do_send");
        assert_eq!(got[0].benign_token, "load");
    }

    #[test]
    fn dedup_one_row_per_function_top_n() {
        // Two distinct benign candidates reach Net at different depths; the top-N lists each ONCE,
        // ranked, and top_n caps the list. Uses String maps (the candor-query element type) to exercise
        // the generic path. (The `sync_state` intermediary is EFFECTY-named — `sync` — so it's excluded,
        // making the surprising set exactly {settings::Settings::load, model::render}.)
        let mut direct: HashMap<String, BTreeSet<String>> = HashMap::new();
        let mut inferred: HashMap<String, BTreeSet<String>> = HashMap::new();
        let mut calls: HashMap<String, BTreeSet<String>> = HashMap::new();
        let loc: HashMap<String, String> = HashMap::new();

        direct.insert("net_layer::do_send".into(), sset(&["Net"]));
        inferred.insert("net_layer::do_send".into(), sset(&["Net"]));

        // settings::Settings::load — 3 hops (benign, deep → higher score). All intermediaries are
        // EFFECTY-named (`sync_state` → sync, `download_step` → download) so they don't add rows.
        inferred.insert("core::sync_state".into(), sset(&["Net"]));
        calls.insert("core::sync_state".into(), sset(&["net_layer::do_send"]));
        inferred.insert("core::download_step".into(), sset(&["Net"]));
        calls.insert("core::download_step".into(), sset(&["core::sync_state"]));
        inferred.insert("settings::Settings::load".into(), sset(&["Net"]));
        calls.insert("settings::Settings::load".into(), sset(&["core::download_step"]));

        // model::render — 1 hop (benign, shallow → lower score).
        inferred.insert("model::render".into(), sset(&["Net"]));
        calls.insert("model::render".into(), sset(&["net_layer::do_send"]));

        let got = best_finds(&inferred, &direct, &calls, &loc, 10);
        assert_eq!(got.len(), 2, "two distinct benign functions, one row each");
        assert_eq!(got[0].func, "settings::Settings::load"); // deeper reach ranks first
        assert_eq!(got[1].func, "model::render");
        // top_n caps the list.
        assert_eq!(best_finds(&inferred, &direct, &calls, &loc, 1).len(), 1);
        // and no function is listed twice.
        assert_eq!(got.iter().map(|f| &f.func).collect::<BTreeSet<_>>().len(), got.len());
    }

    #[test]
    fn fallback_when_nothing_qualifies() {
        // One effectful function, but it is a DIRECT source (not inherited) AND effecty-named — no
        // candidate qualifies → an empty list, yet the crate IS effectful (caller emits the fallback).
        let mut direct: HashMap<String, BTreeSet<&'static str>> = HashMap::new();
        let mut inferred: HashMap<String, BTreeSet<&'static str>> = HashMap::new();
        let calls: HashMap<String, BTreeSet<String>> = HashMap::new();
        let loc: HashMap<String, String> = HashMap::new();
        direct.insert("net::client::send".into(), set(&["Net"]));
        inferred.insert("net::client::send".into(), set(&["Net"]));

        assert!(best_finds(&inferred, &direct, &calls, &loc, 1).is_empty());
        assert!(any_effectful(&inferred), "effectful, so the caller emits the honest fallback");
    }

    #[test]
    fn nothing_when_no_effects() {
        // No non-Unknown effect anywhere → empty list AND not effectful (caller emits nothing at all).
        let direct: HashMap<String, BTreeSet<&'static str>> = HashMap::new();
        let mut inferred: HashMap<String, BTreeSet<&'static str>> = HashMap::new();
        let calls: HashMap<String, BTreeSet<String>> = HashMap::new();
        let loc: HashMap<String, String> = HashMap::new();
        inferred.insert("util::parse".into(), set(&["Unknown"]));
        assert!(best_finds(&inferred, &direct, &calls, &loc, 1).is_empty());
        assert!(!any_effectful(&inferred));
    }
}
