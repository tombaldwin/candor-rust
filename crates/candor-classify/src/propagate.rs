//! The transitive least fixed point over a call graph, for string-set facts:
//!
//!   acc[f] = direct[f] ∪ ⋃_{c ∈ calls[f]} acc[c]
//!
//! Set union is monotone and confluent, so the result is independent of the order in which functions
//! are relaxed. A worklist drives it: when `acc[c]` grows, exactly `c`'s callers are re-enqueued via a
//! callee→callers reverse index, giving amortized O(V + E · facts) instead of the naive sweep's O(V²)
//! on a long chain.
//!
//! IT LIVES HERE, NOT IN THE SCANNER, BECAUSE TWO LAYERS NEED THE SAME REACH. candor-scan computes the
//! reason-class accumulator over its in-memory call graph so the `deny E Unknown[class]` gate can scope
//! an Unknown by reason; candor-query recomputes it from a report's `calls` edges so `unverified
//! --class` filters over the same set. A gate and the disclosure that explains it must agree about what
//! a function's reason classes ARE — two fixpoints that can drift apart is its own defect.

use std::collections::{BTreeSet, HashMap, HashSet, VecDeque};

/// callee → its callers (reverse of `calls`), restricted to callers present in `all`. The naive sweep
/// only ever relaxes `f ∈ all`, so a caller outside `all` must NOT be re-enqueued — else the worklist
/// would grow an `acc` entry the sweep never created, diverging from the reference output. Enforced
/// here (a filter), not merely assumed. Built once per propagation.
fn caller_index<'a>(
    calls: &'a HashMap<String, BTreeSet<String>>,
    in_all: &HashSet<&str>,
) -> HashMap<&'a str, Vec<&'a str>> {
    let mut rev: HashMap<&str, Vec<&str>> = HashMap::new();
    for (f, cs) in calls {
        if !in_all.contains(f.as_str()) {
            continue; // the naive `for f in all` sweep never relaxes this caller
        }
        for c in cs {
            rev.entry(c.as_str()).or_default().push(f.as_str());
        }
    }
    rev
}

/// Propagate string facts (hosts, commands, paths, tables, blind crates, reason classes) to the least
/// fixed point over `calls`. Only functions that actually gain a fact get an `acc` entry — a function
/// with no facts anywhere in its reach is ABSENT from the result, not present-and-empty.
pub fn propagate_str(
    direct: &HashMap<String, BTreeSet<String>>,
    calls: &HashMap<String, BTreeSet<String>>,
    all: &[String],
) -> HashMap<String, BTreeSet<String>> {
    let mut acc = direct.clone();
    let in_all: HashSet<&str> = all.iter().map(String::as_str).collect();
    let rev = caller_index(calls, &in_all);

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
