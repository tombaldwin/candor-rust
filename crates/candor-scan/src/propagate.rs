//! Transitive effect propagation to a fixed point over the local call graph.
//!
//! Both passes compute the SAME least fixed point:
//!   acc[f] = direct[f] ∪ ⋃_{c ∈ calls[f]} acc[c]
//! over a set-union (monotone, confluent) lattice, so the RESULT is independent of the order
//! in which functions are relaxed. A naive `while changed { for f in all { … } }` sweep is
//! correct but re-scans every function on every pass, so its cost is O(passes · V · Ē) and the
//! pass count equals the longest back-to-front dependency chain in declaration order — up to V
//! for a single deep `f0→f1→…→fN` chain, giving O(V²). Real crates converge in 2–4 passes
//! (shallow graphs, per-module scan), so the quadratic only bites on pathological long chains;
//! the WORKLIST below removes that cliff without changing the result: when `acc[c]` grows we
//! re-enqueue exactly `c`'s callers via a callee→callers reverse index, so each function is
//! reprocessed only when one of its callees actually gained an effect. Same least fixed point,
//! amortized O(V + E · effects) instead of O(V²).

use crate::*;

/// callee → its callers (reverse of `calls`), restricted to keys present in `all` (mirrors the
/// naive sweep, which only ever writes `acc` entries for `f ∈ all`). Built once per propagation.
fn caller_index(calls: &HashMap<String, BTreeSet<String>>) -> HashMap<&str, Vec<&str>> {
    let mut rev: HashMap<&str, Vec<&str>> = HashMap::new();
    for (f, cs) in calls {
        for c in cs {
            rev.entry(c.as_str()).or_default().push(f.as_str());
        }
    }
    rev
}

pub(crate) fn propagate(
    direct: &HashMap<String, BTreeSet<&'static str>>,
    calls: &HashMap<String, BTreeSet<String>>,
    all: &[String],
) -> HashMap<String, BTreeSet<&'static str>> {
    let mut acc = direct.clone();
    for f in all {
        acc.entry(f.clone()).or_default();
    }
    let rev = caller_index(calls);

    // Seed the worklist with every function: each must absorb its callees' current effects at
    // least once. Order doesn't affect the fixpoint; declaration order keeps behaviour familiar.
    let mut queue: VecDeque<String> = all.iter().cloned().collect();
    let mut queued: HashSet<String> = all.iter().cloned().collect();

    while let Some(f) = queue.pop_front() {
        queued.remove(&f);
        let add: BTreeSet<&'static str> = calls
            .get(&f)
            .map(|cs| cs.iter().filter_map(|c| acc.get(c)).flatten().copied().collect())
            .unwrap_or_default();
        let e = acc.entry(f.clone()).or_default();
        let before = e.len();
        e.extend(add);
        if e.len() != before {
            // `f` grew → its callers must re-absorb from it. Re-enqueue only those (dedup via `queued`).
            if let Some(callers) = rev.get(f.as_str()) {
                for &caller in callers {
                    if queued.insert(caller.to_string()) {
                        queue.push_back(caller.to_string());
                    }
                }
            }
        }
    }
    acc
}

pub(crate) fn propagate_str(
    direct: &HashMap<String, BTreeSet<String>>,
    calls: &HashMap<String, BTreeSet<String>>,
    all: &[String],
) -> HashMap<String, BTreeSet<String>> {
    let mut acc = direct.clone();
    let rev = caller_index(calls);

    let mut queue: VecDeque<String> = all.iter().cloned().collect();
    let mut queued: HashSet<String> = all.iter().cloned().collect();

    while let Some(f) = queue.pop_front() {
        queued.remove(&f);
        let add: BTreeSet<String> = calls
            .get(&f)
            .map(|cs| cs.iter().filter_map(|c| acc.get(c)).flatten().cloned().collect())
            .unwrap_or_default();
        if add.is_empty() {
            continue;
        }
        let e = acc.entry(f.clone()).or_default();
        let before = e.len();
        e.extend(add);
        if e.len() != before {
            if let Some(callers) = rev.get(f.as_str()) {
                for &caller in callers {
                    if queued.insert(caller.to_string()) {
                        queue.push_back(caller.to_string());
                    }
                }
            }
        }
    }
    acc
}
