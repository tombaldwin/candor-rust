//! ⟨0.41⟩ `accept` — A KNOWN VIOLATION, WRITTEN DOWN WHERE A REVIEWER READS IT (SPEC §6.2 ⟨0.41⟩).
//!
//! **THE CARDINAL DIRECTION HERE IS ACCEPTING A VIOLATION THAT IS NEW.** Every decision below is made so that
//! doubt leaves the row RED: an entry names ONE producing rule LINE, ONE exact `fn` and the effects it covers;
//! a twin it cannot tell apart is accepted by nothing; an `Unknown` whose classes are not determinable is not
//! covered by any filter; a nested unit is accepted only on evidence read from THIS run. A bug in this file may
//! cost a false red; it must not cost a false green.
//!
//! **THE SEAM.** [`settle`] is a pure function of `(entries, rows, run facts)`. It is deliberately NOT inside
//! [`crate::gate::gate`]: the scan route gates a workspace one MEMBER at a time and concatenates the rows, so a
//! match made inside `gate()` would call an entry written for member A stale in member B's call, and could not
//! see a twin that lives in a different member. Both routes call `gate()` (which only stamps each policy row
//! with its producing line), then hand the rows of the WHOLE run here — one implementation, so the two routes
//! cannot disagree about an acceptance over the same rows (§3.1 byte-equality).

use crate::policy::{AcceptEntry, UNKNOWN};
use candor_report::{AcceptedItem, GateViolation};
use std::collections::{BTreeMap, BTreeSet};

/// What the CALLER knows about the run that a list of rows cannot say.
pub struct RunFacts<'a> {
    /// EVERY unit of the run (not just the violating ones), as `(name a policy matches and a row carries,
    /// identity)`. Two units sharing a name with different identities are `(hash, fn)` twins (§3.1 ⟨0.40⟩), and an
    /// entry on that name accepts NEITHER. The identity is only ever compared for equality.
    pub units: &'a [(String, String)],
    /// [`crate::policy::rule_key`] of every rule line this route could NOT evaluate (§3.1 `unevaluated`): `forbid`
    /// and `only` on `gate --report`, an unanswerable filter, a withheld pair. An entry on such a rule is `unjudged`
    /// — nothing can be said either way — and is never stale.
    pub unevaluated: &'a BTreeSet<String>,
}

/// What [`settle`] decided.
#[derive(Default, Debug)]
pub struct Settled {
    /// Every input row that was NOT accepted, in input order. Includes rows no `accept` line could touch.
    pub remaining: Vec<GateViolation>,
    /// One `AS-EFF-012` row per STALE entry (`ok: false`, exit 1). Kept apart from `remaining` so a PROVISIONAL
    /// per-member call (scan route) can ignore them: staleness is a property of the whole run.
    pub stale: Vec<GateViolation>,
    /// The verdict's `accepted` list, sorted by `line`, then `fn`, then `hash` (code point).
    pub accepted: Vec<AcceptedItem>,
    /// How many ROWS were taken out of `violations` (for the human channel's count).
    pub accepted_rows: usize,
    /// One human sentence per entry that is `unjudged`, `ambiguous`, `exceeded` — each carrying a remedy.
    /// Unprefixed: the caller prefixes its own binary name.
    pub notes: Vec<String>,
}

const DENY: &str = "AS-EFF-006";
const FORBID: &str = "AS-EFF-009";

/// The unit a positional unit is lexically nested in, when its name says so: `f::{closure#0}` is inside `f`,
/// `f::{closure#0}::{closure#1}` inside `f::{closure#0}`. Read off the NAME, and only ever used together with the
/// two conditions the clause pins (the enclosing unit's own row is accepted by the same entry in this run, and
/// the nested row's effects are a subset), so it trusts no engine's attribution model.
fn enclosing_of(func: &str) -> Option<&str> {
    if !func.ends_with('}') {
        return None;
    }
    let at = func.rfind("::{")?;
    let seg = &func[at + 2..];
    crate::policy::is_positional_key(seg).then(|| &func[..at])
}

fn covers(e: &AcceptEntry, row: &GateViolation) -> bool {
    for x in &row.effects {
        if x == UNKNOWN {
            // Covered only by an `Unknown[F]` whose class list contains EVERY class of the row's `reasonClass`
            // (all classes on the function). A row whose classes are not determinable is covered by NO filter.
            let Some(cs) = &e.classes else { return false };
            if !e.effects.contains(UNKNOWN) || row.reason_class.is_empty() {
                return false;
            }
            if !row.reason_class.iter().all(|c| cs.contains(c)) {
                return false;
            }
        } else if !e.effects.contains(x.as_str()) {
            return false;
        }
    }
    true
}

fn item(e: &AcceptEntry, rule: &str, func: &str, hash: &str, effects: Vec<String>, status: &str, enclosing: &str) -> AcceptedItem {
    AcceptedItem {
        rule: rule.to_string(),
        func: func.to_string(),
        hash: hash.to_string(),
        effects,
        line: e.line,
        policy_rule: e.rule_key.clone(),
        because: e.because.clone(),
        status: status.to_string(),
        enclosing: enclosing.to_string(),
    }
}

/// Decide a whole run's rows against its `accept` entries. `rows` is every violation of the run (policy rows carry
/// their producing line in [`GateViolation::rule_key`]; every other row passes through untouched).
pub fn settle(entries: &[AcceptEntry], rows: Vec<GateViolation>, facts: &RunFacts) -> Settled {
    let mut out = Settled::default();
    if entries.is_empty() {
        out.remaining = rows;
        return out;
    }
    // (hash, fn) TWINS: >1 distinct identity under one name.
    let mut by_name: BTreeMap<&str, BTreeSet<&str>> = BTreeMap::new();
    for (n, id) in facts.units {
        by_name.entry(n.as_str()).or_default().insert(id.as_str());
    }
    let twin = |name: &str| by_name.get(name).is_some_and(|s| s.len() > 1);
    let usable = |e: &AcceptEntry| !facts.unevaluated.contains(&e.rule_key) && !twin(&e.func);
    let policy_row = |r: &GateViolation| !r.rule_key.is_empty() && (r.rule == DENY || r.rule == FORBID);

    let mut acc = vec![false; rows.len()];
    // per entry (by index): the rows it accepted directly; (nested row, enclosing row) pairs.
    let mut direct: Vec<Vec<usize>> = vec![Vec::new(); entries.len()];
    let mut nested: Vec<Vec<(usize, usize)>> = vec![Vec::new(); entries.len()];

    // ---- direct: the entry names this row's producing LINE and this row's exact `fn`, and covers its effects.
    // Per producing line, BEFORE any de-duplication of identical `{rule, fn, effects}` rows: a row two lines
    // produce is two rows here, each accepted only by an entry naming ITS line.
    for (i, r) in rows.iter().enumerate() {
        if !policy_row(r) {
            continue;
        }
        for (k, e) in entries.iter().enumerate() {
            if usable(e) && e.rule_key == r.rule_key && e.func == r.func && covers(e, r) {
                acc[i] = true;
                direct[k].push(i);
            }
        }
    }
    // ---- the nested-unit rule: a row on a unit V keyed positionally and lexically inside U is accepted by an
    // entry on U iff that entry accepts U's own row under the SAME rule line in THIS run, and V's row effects
    // are a subset of U's. Both conditions are read from this run; an engine that does not carry V's effect onto
    // U leaves V red — fail-closed, and disclosed by the red row itself.
    for (i, v) in rows.iter().enumerate() {
        if acc[i] || !policy_row(v) {
            continue;
        }
        let mut cur: &str = &v.func;
        for _ in 0..8 {
            let Some(enc) = enclosing_of(cur) else { break };
            for (k, e) in entries.iter().enumerate() {
                if !usable(e) || e.rule_key != v.rule_key || e.func != enc {
                    continue;
                }
                for &u in &direct[k] {
                    let ur = &rows[u];
                    if ur.func == enc && ur.rule_key == v.rule_key && v.effects.iter().all(|x| ur.effects.contains(x)) {
                        acc[i] = true;
                        nested[k].push((i, u));
                    }
                }
            }
            if acc[i] {
                break;
            }
            cur = enc; // a closure inside a closure: climb to the next positional parent
        }
    }
    out.accepted_rows = acc.iter().filter(|a| **a).count();
    let remaining_idx: Vec<usize> = (0..rows.len()).filter(|&i| !acc[i]).collect();

    // ---- statuses, stale rows.
    let mut items: Vec<AcceptedItem> = Vec::new();
    for (k, e) in entries.iter().enumerate() {
        let code = if e.forbid { FORBID } else { DENY };
        let names: Vec<String> = e.effects.iter().map(|s| s.to_string()).collect();
        if facts.unevaluated.contains(&e.rule_key) {
            items.push(item(e, code, &e.func, "", names, "unjudged", ""));
            out.notes.push(format!(
                "policy line {} (`{}`) is UNJUDGED — its rule `{}` could not be evaluated on this route, so nothing was \
                 accepted by it. Gate at scan time (`candor-scan <dir> --policy …`) to judge it.",
                e.line, e.text.trim(), e.rule_key
            ));
            continue;
        }
        if twin(&e.func) {
            items.push(item(e, code, &e.func, "", names, "ambiguous", ""));
            out.notes.push(format!(
                "policy line {} (`{}`) is AMBIGUOUS — two units in this run are named `{}` with different identities, \
                 so it accepts NEITHER. Gate the modules separately (one policy file per separately-gated module) so \
                 each name is one unit.",
                e.line, e.text.trim(), e.func
            ));
            continue;
        }
        if !direct[k].is_empty() {
            // The row this entry accepted. Several rows (identical duplicate lines) are picked deterministically,
            // so the two routes — which accumulate in different orders — carry the same item.
            let &first = direct[k].iter().min_by_key(|&&i| (rows[i].hash.clone(), rows[i].effects.clone())).unwrap();
            let r = &rows[first];
            let mut eff = r.effects.clone();
            eff.sort();
            items.push(item(e, &r.rule, &r.func, &r.hash, eff, "accepted", ""));
            for &(vi, _) in &nested[k] {
                let v = &rows[vi];
                let mut eff = v.effects.clone();
                eff.sort();
                items.push(item(e, &v.rule, &v.func, &v.hash, eff, "accepted", &e.func));
            }
            continue;
        }
        let over = remaining_idx.iter().map(|&i| &rows[i]).find(|r| policy_row(r) && r.rule_key == e.rule_key && r.func == e.func);
        if let Some(r) = over {
            items.push(item(e, code, &e.func, "", names, "exceeded", ""));
            out.notes.push(format!(
                "policy line {} (`{}`) no longer covers `{}`: it now performs {{ {} }} under `{}`, more than the {{ {} }} \
                 accepted. The violation stays red. Review the new behaviour; if it is intended, widen the entry to list \
                 it, otherwise fix the code.",
                e.line, e.text.trim(), e.func, r.effects.join(", "), e.rule_key, e.effects.iter().copied().collect::<Vec<_>>().join(", ")
            ));
            continue;
        }
        // STALE: its rule was evaluated, and it accepted nothing and is not exceeded.
        out.stale.push(GateViolation {
            rule: "AS-EFF-012".into(),
            func: e.func.clone(),
            effects: names.clone(),
            detail: format!(
                "`{}` has a STALE acceptance — policy line {} (`{}`) matched no violation of `{}`. Delete line {}: left \
                 in place it would accept whatever a later edit puts under that key",
                e.func, e.line, e.text.trim(), e.rule_key, e.line
            ),
            ..Default::default()
        });
    }
    items.sort_by(|a, b| (a.line, &a.func, &a.hash).cmp(&(b.line, &b.func, &b.hash)));
    out.accepted = items;
    out.remaining = rows.into_iter().enumerate().filter(|(i, _)| !acc[*i]).map(|(_, r)| r).collect();
    out
}

/// The human-channel line SPEC §6.2 ⟨0.41⟩ requires on EVERY run with at least one accepted row (*"211 violation(s)
/// accepted by policy; 0 new"*) — never only on request: a gate that says `ok` over 211 accepted violations without
/// the number is the silence this directive exists to avoid. `new` is what is still in `violations`.
pub fn announce(accepted_rows: usize, new: usize) -> Option<String> {
    (accepted_rows > 0).then(|| format!("{accepted_rows} violation(s) accepted by policy; {new} new"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::policy::parse_policy_silent;

    fn row(fnn: &str, key: &str, eff: &[&str], hash: &str, rc: &[&str]) -> GateViolation {
        GateViolation {
            rule: if eff.is_empty() { FORBID } else { DENY }.into(),
            func: fnn.into(),
            hash: hash.into(),
            effects: eff.iter().map(|s| s.to_string()).collect(),
            reason_class: rc.iter().map(|s| s.to_string()).collect(),
            rule_key: key.into(),
            ..Default::default()
        }
    }
    fn entries(pol: &str) -> Vec<AcceptEntry> {
        parse_policy_silent(pol, &Default::default()).accepts
    }
    fn run<'a>(units: &'a [(String, String)], uneval: &'a BTreeSet<String>) -> RunFacts<'a> {
        RunFacts { units, unevaluated: uneval }
    }
    fn u(n: &str, id: &str) -> (String, String) {
        (n.to_string(), id.to_string())
    }

    #[test]
    fn grammar_pins_every_malformed_form_to_ignored() {
        let bad = [
            "deny Fs dao\naccept dao.a Fs for deny Fs dao",                                // no because
            "deny Fs dao\naccept dao.a Fs for deny Fs dao because",                        // empty reason
            "deny Fs dao\naccept dao.a Fs for deny Fs dao because #123",                   // comment ate it
            "deny Fs dao\naccept dao.a Fs deny Fs dao because x",                          // no `for`
            "deny Fs dao\naccept dao.a for deny Fs dao because x",                         // deny without effects
            "deny Fs dao\naccept dao.a Fs for deny Fs app because x",                      // names no rule
            "deny Fs dao\naccept dao.a Net[unknown-host] for deny Fs dao because x",       // bracket on non-Unknown
            "deny Unknown dao\naccept dao.a Unknown for deny Unknown dao because x",       // bare Unknown
            "deny Unknown dao\naccept dao.a Unknown[*] for deny Unknown dao because x",
            "deny Unknown dao\naccept dao.a Unknown[dynamic] for deny Unknown dao because x",
            // `{closure#N}` is UNWRITABLE here — the `#` begins a comment, so the line is cut before `for` and is
            // ignored for that reason (§6.2: "which §6.2's comment rule makes unwritable anyway"). This engine has no
            // writable positional spelling; `is_positional_key` is exercised directly below and by the nested-unit rule.
            "deny Fs dao\naccept dao.f::{closure#0} Fs for deny Fs dao because x",
            "deny Fs dao\nallow Net in dao ok.example\naccept dao.a Net for allow Net in dao ok.example because x",
            "forbid dao -> infra\naccept dao.a Fs for forbid dao -> infra because x",       // forbid takes none
            "deny Fs dao\naccept dao.a Fs for deny Fs dao extra because x",                // not the raw tokens
            "deny Fs dao extra\naccept dao.a Fs for deny Fs dao because x",                // consumed tokens only
        ];
        for pol in bad {
            let p = parse_policy_silent(pol, &Default::default());
            assert!(p.accepts.is_empty(), "must NOT become an entry: {pol:?}");
            assert!(p.errors.iter().any(|e| e.token == "accept" && !e.fatal), "must be listed as ignored: {pol:?}");
        }
        // a typo'd effect or class token is the policy ERROR, exactly like `deny Nett app`
        for pol in [
            "deny Fs dao\naccept dao.a Fss for deny Fs dao because x",
            "deny Unknown dao\naccept dao.a Unknown[dispatchh] for deny Unknown dao because x",
        ] {
            let p = parse_policy_silent(pol, &Default::default());
            assert!(p.accepts.is_empty());
            assert!(!p.fatal_messages().is_empty(), "must be fatal: {pol:?}");
        }
    }

    #[test]
    fn grammar_reads_the_good_forms() {
        let p = parse_policy_silent(
            "accept dao.a Fs for deny   Fs\tdao extra because known since 2026 # why\n\
             deny Fs dao extra\n\
             pure parse\n\
             forbid dao -> infra\n\
             accept parse.x Clock Rand for pure parse because x\n\
             accept dao.b for forbid dao -> infra because y\n\
             deny Unknown[dispatch,native] dao\n\
             accept dao.c Unknown[dispatch] for deny Unknown[dispatch,native] dao because z\n",
            &Default::default(),
        );
        assert!(p.errors.is_empty(), "{:?}", p.errors.iter().map(|e| &e.message).collect::<Vec<_>>());
        assert_eq!(p.accepts.len(), 4);
        // an entry may name a rule that comes AFTER it, and the key is the RAW tokens single-spaced
        assert_eq!(p.accepts[0].rule_key, "deny Fs dao extra");
        assert_eq!(p.accepts[0].because, "known since 2026");
        assert_eq!(p.accepts[0].line, 1);
        assert!(p.accepts[2].forbid && p.accepts[2].effects.is_empty());
        assert_eq!(p.accepts[3].classes, Some(["dispatch".to_string()].into_iter().collect()));
        // directives, not rules
        assert_eq!(p.rules.len(), 3);
    }

    #[test]
    fn only_accept_lines_is_still_zero_rules() {
        let p = parse_policy_silent("accept dao.a Fs for deny Fs dao because x\n", &Default::default());
        assert!(p.rules.is_empty() && p.allow_rules.is_empty() && p.layer_rules.is_empty() && p.only_rules.is_empty());
    }

    #[test]
    fn exact_fn_per_line_effect_subset() {
        let es = entries("deny Fs dao\ndeny Fs\naccept dao.a Fs for deny Fs dao because x\n");
        let units = [u("dao.a", "p#dao.a"), u("dao.ab", "p#dao.ab")];
        let none = BTreeSet::new();
        let rows = vec![
            row("dao.a", "deny Fs dao", &["Fs"], "p#dao.a", &[]),
            row("dao.a", "deny Fs", &["Fs"], "p#dao.a", &[]),            // the SECOND line's row: not named
            row("dao.ab", "deny Fs dao", &["Fs"], "p#dao.ab", &[]),      // prefix: not named
        ];
        let s = settle(&es, rows, &run(&units, &none));
        assert_eq!(s.accepted_rows, 1);
        assert_eq!(s.remaining.len(), 2);
        assert!(s.remaining.iter().all(|r| !(r.func == "dao.a" && r.rule_key == "deny Fs dao")));
        assert!(s.stale.is_empty());
        assert_eq!(s.accepted.len(), 1);
        assert_eq!(s.accepted[0].status, "accepted");
        // more effects than accepted: exceeded, row stays WHOLE, never also stale
        let es = entries("deny Fs Rand dao\naccept dao.a Fs for deny Fs Rand dao because x\n");
        let rows = vec![row("dao.a", "deny Fs Rand dao", &["Fs", "Rand"], "p#dao.a", &[])];
        let s = settle(&es, rows, &run(&units, &none));
        assert_eq!(s.accepted_rows, 0);
        assert_eq!(s.remaining.len(), 1);
        assert!(s.stale.is_empty());
        assert_eq!(s.accepted[0].status, "exceeded");
        assert!(s.notes[0].contains("widen the entry"));
    }

    #[test]
    fn twins_unjudged_stale_and_unknown_classes() {
        let none = BTreeSet::new();
        // twins: same name, different identity -> NEITHER accepted
        let es = entries("deny Fs dao\naccept dao.t Fs for deny Fs dao because x\n");
        let units = [u("dao.t", "p#dao.t"), u("dao.t", "q#dao.t")];
        let rows = vec![row("dao.t", "deny Fs dao", &["Fs"], "p#dao.t", &[]), row("dao.t", "deny Fs dao", &["Fs"], "q#dao.t", &[])];
        let s = settle(&es, rows, &run(&units, &none));
        assert_eq!((s.accepted_rows, s.remaining.len(), s.stale.len()), (0, 2, 0));
        assert_eq!(s.accepted[0].status, "ambiguous");
        // unevaluated rule: unjudged, never stale
        let units = [u("dao.a", "p#dao.a")];
        let uneval: BTreeSet<String> = ["deny Fs dao".to_string()].into_iter().collect();
        let s = settle(&es_for("dao.a"), vec![], &run(&units, &uneval));
        assert_eq!(s.accepted[0].status, "unjudged");
        assert!(s.stale.is_empty());
        // stale: rule evaluated, row gone
        let s = settle(&es_for("dao.a"), vec![], &run(&units, &none));
        assert_eq!(s.stale.len(), 1);
        assert_eq!(s.stale[0].rule, "AS-EFF-012");
        assert!(s.stale[0].detail.contains("Delete line 2"));
        // Unknown: covered only by a filter containing EVERY class of the row; undeterminable classes covered by none
        let es = entries("deny Unknown dao\naccept dao.c Unknown[dispatch] for deny Unknown dao because x\n");
        let units = [u("dao.c", "p#dao.c")];
        let ok = settle(&es, vec![row("dao.c", "deny Unknown dao", &["Unknown"], "p#dao.c", &["dispatch"])], &run(&units, &none));
        assert_eq!(ok.accepted_rows, 1);
        let moved = settle(&es, vec![row("dao.c", "deny Unknown dao", &["Unknown"], "p#dao.c", &["dispatch", "native"])], &run(&units, &none));
        assert_eq!((moved.accepted_rows, moved.remaining.len()), (0, 1));
        assert_eq!(moved.accepted[0].status, "exceeded");
        let blind = settle(&es, vec![row("dao.c", "deny Unknown dao", &["Unknown"], "p#dao.c", &[])], &run(&units, &none));
        assert_eq!(blind.accepted_rows, 0, "an Unknown whose classes are not determinable is covered by NO filter");
    }

    fn es_for(f: &str) -> Vec<AcceptEntry> {
        entries(&format!("deny Fs dao\naccept {f} Fs for deny Fs dao because x\n"))
    }

    #[test]
    fn nested_units_need_the_enclosing_row_and_a_subset() {
        let none = BTreeSet::new();
        let es = es_for("dao::a");
        let units = [u("dao::a", "p#dao::a"), u("dao::a::{closure#0}", "p#dao::a::{closure#0}")];
        let a = row("dao::a", "deny Fs dao", &["Fs"], "p#dao::a", &[]);
        let v = row("dao::a::{closure#0}", "deny Fs dao", &["Fs"], "p#dao::a::{closure#0}", &[]);
        let s = settle(&es, vec![a.clone(), v.clone()], &run(&units, &none));
        assert_eq!((s.accepted_rows, s.remaining.len()), (2, 0));
        assert_eq!(s.accepted.len(), 2);
        assert_eq!(s.accepted[1].enclosing, "dao::a");
        // the nested row does MORE than the enclosing row -> stays red
        let v2 = row("dao::a::{closure#0}", "deny Fs dao", &["Fs", "Clock"], "p#dao::a::{closure#0}", &[]);
        let s = settle(&es, vec![a.clone(), v2], &run(&units, &none));
        assert_eq!((s.accepted_rows, s.remaining.len()), (1, 1));
        // the enclosing row is not accepted (engine did not carry the effect onto it) -> nested stays red
        let s = settle(&es, vec![v], &run(&units, &none));
        assert_eq!((s.accepted_rows, s.remaining.len()), (0, 1));
        assert_eq!(s.stale.len(), 1, "nothing accepted and nothing exceeded");
    }

    #[test]
    fn positional_spellings() {
        for f in ["a::{closure#0}", "a::{closure#12}::{closure#3}", "x::{async_block#1}", "<T as U>::m::{constant#0}"] {
            assert!(crate::policy::is_positional_key(f), "{f}");
        }
        for f in ["a::closure", "a::{closure}", "a::{closure#}", "a::{#0}", "dao::a", "a::b#1", "{closure#0"] {
            assert!(!crate::policy::is_positional_key(f), "{f}");
        }
    }
}
