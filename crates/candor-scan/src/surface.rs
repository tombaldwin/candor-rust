//! Surface the single most SURPRISING transitive reach (the cold-repo hook).
//!
//! After the effect summary + κ ledger, candor-scan emits ONE more stderr line: the most surprising
//! transitive reach in the crate + a ready-to-run `candor path` command. See SURFACE-BEST-FIND-DESIGN.md.
//!
//! Fully deterministic — pure call-graph + name analysis, NO LLM. A CANDIDATE is a function `F` that
//! INHERITS an effect `E` (E ∈ inferred[F] but E ∉ direct[F]); we BFS to the nearest local direct SOURCE
//! `S` and score by how surprising the reach is (a benign-named function reaching a scary effect). The
//! find is never *wrong*: `candor path` re-derives the chain and the gate is ground truth. When nothing
//! clears the bar we emit an honest "nothing hidden" fallback — never a manufactured surprise.

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

/// A scored candidate reach.
struct Find {
    func: String,
    effect: &'static str,
    hops: usize,
    source: String,
    benign_token: String,
    score: i64,
}

fn is_test(qual: &str) -> bool {
    qual.contains("::tests::") || qual.contains("::test::")
}

/// BFS from `func` over `calls` (follow callees, shortest hops) to the nearest function `S` with
/// `effect` ∈ direct[S]. Returns (hops≥1, S). Only traverses through callees that transitively carry
/// the effect, so the frontier stays on-effect (matches `candor path`'s walk).
fn nearest_source<'a>(
    func: &'a str,
    effect: &str,
    direct: &'a HashMap<String, BTreeSet<&'static str>>,
    inferred: &'a HashMap<String, BTreeSet<&'static str>>,
    calls: &'a HashMap<String, BTreeSet<String>>,
) -> Option<(usize, &'a str)> {
    let mut seen: BTreeSet<&str> = BTreeSet::new();
    let mut q: VecDeque<(&str, usize)> = VecDeque::new();
    seen.insert(func);
    q.push_back((func, 0));
    while let Some((cur, d)) = q.pop_front() {
        // A direct source found at distance d≥1 is the nearest (BFS). The start `func` itself is an
        // INHERITED reach (E ∉ direct[func]) so it never matches at d==0.
        if d >= 1 && direct.get(cur).is_some_and(|s| s.contains(effect)) {
            return Some((d, cur));
        }
        if let Some(cs) = calls.get(cur) {
            for c in cs {
                let cc = c.as_str();
                if !seen.contains(cc) && inferred.get(cc).is_some_and(|s| s.contains(effect)) {
                    seen.insert(cc);
                    q.push_back((cc, d + 1));
                }
            }
        }
    }
    None
}

/// Compute the single most surprising reach. Returns `None` when there are ZERO effectful functions
/// (caller emits nothing); returns `Some(None)` when there were effectful functions but none cleared
/// the bar (caller emits the honest fallback); returns `Some(Some(find))` for the winning reach.
#[allow(clippy::type_complexity)]
fn best_find(
    inferred: &HashMap<String, BTreeSet<&'static str>>,
    direct: &HashMap<String, BTreeSet<&'static str>>,
    calls: &HashMap<String, BTreeSet<String>>,
) -> Option<Option<Find>> {
    // Any function carrying a real (non-Unknown) effect makes the crate "effectful" — governs
    // whether we emit the fallback vs nothing.
    let mut any_effectful = false;

    // Deterministic iteration: sort quals ascending so the tie-break (qual ascending) is stable and
    // HashMap order never leaks into the result.
    let mut quals: Vec<&String> = inferred.keys().collect();
    quals.sort();

    let mut best: Option<Find> = None;

    for f in quals {
        let inf = &inferred[f];
        if inf.iter().any(|e| *e != "Unknown") {
            any_effectful = true;
        }
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
        // Candidate effects: inherited (in inferred, not direct), not Unknown.
        let mut effects: Vec<&'static str> = inf.iter().copied().filter(|e| *e != "Unknown" && !dir.contains(e)).collect();
        effects.sort();
        for e in effects {
            let sal = salience(e);
            if sal == 0 {
                continue;
            }
            let Some((hops, s)) = nearest_source(f, e, direct, inferred, calls) else {
                continue; // no LOCAL direct source — nothing to show
            };
            let benign = has_token(f_leaf, BENIGN);
            let benignity = if benign.is_some() { 3 } else { 1 };
            let crossing = if module_of(s) != f_mod { 2 } else { 1 };
            let score = sal * benignity * hops_factor(hops) * crossing;
            if score == 0 {
                continue;
            }
            let cand = Find {
                func: f.clone(),
                effect: e,
                hops,
                source: s.to_string(),
                benign_token: benign.unwrap_or_default(),
                score,
            };
            // Tie-break: higher score, then fewer hops, then qual ascending. Quals are iterated
            // ascending and effects ascending, so a strict > keeps the first (smallest qual) winner.
            let better = match &best {
                None => true,
                Some(b) => {
                    cand.score > b.score
                        || (cand.score == b.score && cand.hops < b.hops)
                    // equal score & hops: earlier qual already seen (ascending iteration) → keep it.
                }
            };
            if better {
                best = Some(cand);
            }
        }
    }

    if !any_effectful {
        return None;
    }
    Some(best)
}

/// Emit the surface note to STDERR. `loc` maps qual → "file:line" for the source callout.
pub fn emit(
    inferred: &HashMap<String, BTreeSet<&'static str>>,
    direct: &HashMap<String, BTreeSet<&'static str>>,
    calls: &HashMap<String, BTreeSet<String>>,
    loc: &HashMap<String, String>,
) {
    match best_find(inferred, direct, calls) {
        None => {} // zero effectful functions — emit nothing
        Some(None) => {
            eprintln!("candor: nothing hidden — every effect sits where its name says it should.");
        }
        Some(Some(f)) => {
            let where_s = loc.get(&f.source).map(String::as_str).unwrap_or("?");
            let hop_word = if f.hops == 1 { "hop" } else { "hops" };
            let benign_note = if f.benign_token.is_empty() {
                String::new()
            } else {
                format!("          a \"{}\"-named function reaching {}.\n", f.benign_token, f.effect)
            };
            eprintln!(
                "candor: most surprising reach — `{}` performs {}, {} {} away via `{}` ({}).\n{}          →  candor path {} {}",
                f.func, f.effect, f.hops, hop_word, f.source, where_s, benign_note, f.func, f.effect
            );
        }
    }
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

        let got = best_find(&inferred, &direct, &calls).expect("effectful").expect("a winner");
        assert_eq!(got.func, "settings::Settings::load");
        assert_eq!(got.effect, "Net");
        assert_eq!(got.hops, 3);
        assert_eq!(got.source, "net_layer::do_send");
        assert_eq!(got.benign_token, "load");
    }

    #[test]
    fn fallback_when_nothing_qualifies() {
        // One effectful function, but it is a DIRECT source (not inherited) AND effecty-named — no
        // candidate qualifies → Some(None), the honest fallback.
        let mut direct: HashMap<String, BTreeSet<&'static str>> = HashMap::new();
        let mut inferred: HashMap<String, BTreeSet<&'static str>> = HashMap::new();
        let calls: HashMap<String, BTreeSet<String>> = HashMap::new();
        direct.insert("net::client::send".into(), set(&["Net"]));
        inferred.insert("net::client::send".into(), set(&["Net"]));

        let got = best_find(&inferred, &direct, &calls);
        assert!(matches!(got, Some(None)), "expected the honest fallback, got a winner");
    }

    #[test]
    fn nothing_when_no_effects() {
        // No non-Unknown effect anywhere → None (caller emits nothing at all).
        let direct: HashMap<String, BTreeSet<&'static str>> = HashMap::new();
        let mut inferred: HashMap<String, BTreeSet<&'static str>> = HashMap::new();
        let calls: HashMap<String, BTreeSet<String>> = HashMap::new();
        inferred.insert("util::parse".into(), set(&["Unknown"]));
        assert!(best_find(&inferred, &direct, &calls).is_none());
    }
}
