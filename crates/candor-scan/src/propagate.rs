//! Transitive effect propagation to a fixed point over the local call graph.

use crate::*;

pub(crate) fn propagate(
    direct: &HashMap<String, BTreeSet<&'static str>>,
    calls: &HashMap<String, BTreeSet<String>>,
    all: &[String],
) -> HashMap<String, BTreeSet<&'static str>> {
    let mut acc = direct.clone();
    for f in all {
        acc.entry(f.clone()).or_default();
    }
    let mut changed = true;
    while changed {
        changed = false;
        for f in all {
            let add: BTreeSet<&'static str> = calls
                .get(f)
                .map(|cs| cs.iter().filter_map(|c| acc.get(c)).flatten().copied().collect())
                .unwrap_or_default();
            let e = acc.entry(f.clone()).or_default();
            let before = e.len();
            e.extend(add);
            if e.len() != before {
                changed = true;
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
    let mut changed = true;
    while changed {
        changed = false;
        for f in all {
            let add: BTreeSet<String> = calls
                .get(f)
                .map(|cs| cs.iter().filter_map(|c| acc.get(c)).flatten().cloned().collect())
                .unwrap_or_default();
            if add.is_empty() {
                continue;
            }
            let e = acc.entry(f.clone()).or_default();
            let before = e.len();
            e.extend(add);
            if e.len() != before {
                changed = true;
            }
        }
    }
    acc
}
