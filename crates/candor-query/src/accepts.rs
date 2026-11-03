//! ⟨0.41⟩ `candor-query accepts [--report <locator>] [--policy <file>] [--json] [--adopt --because <reason>]` — refresh
//! (or adopt) a policy's `accept` lines as a PROPOSED policy (SPEC §6.2 ⟨0.41⟩).
//!
//! **THE CARDINAL DIRECTION IS ACCEPTING A VIOLATION THAT IS NEW, AND THIS VERB CAN ONLY ACCEPT LESS.** So it decides
//! nothing itself. Which entry is stale, accepted, exceeded, ambiguous, unjudged or ignored, which rows an accepted
//! entry took, which rows are left and which rule line produced each — every one of those is read off THIS ENGINE'S
//! OWN GATE ([`gate_core`], the function `candor-query gate` runs, over the same bytes), and the proposal is then
//! gated again by that same function to say what committing it does (`after`). There is no second matcher here that
//! could drift from the gate's: "the same parse and the same matching" holds by construction.
//!
//! It writes NO FILE — not the policy, not a temp file, not a sink. The proposal goes to stdout (or, under `--json`,
//! a plan whose `text` is those bytes) and the operator diffs and commits it. That is also why the policy TEXT is
//! passed to the gate rather than a path: nothing is ever created for the gate to read.
//!
//! Every line the verb does not change is kept BYTE FOR BYTE, terminator included, so the file is split here at the
//! same terminators the gate's own reader splits at (LF, CRLF and a lone CR — the gate's `line` numbers must be this
//! verb's line numbers) and each line keeps the bytes it came with.

use crate::*;
use candor_classify::policy::{is_positional_key, rule_key, AcceptEntry};
use candor_report::{AcceptedItem, GateViolation};

const MARK: &str = "# candor-accepts:";
const REPLACE: &str = " — REPLACE the line above with: ";
const RULE_KINDS: [&str; 5] = ["deny", "pure", "forbid", "allow", "only"];
const USAGE: &str = "usage: candor-query accepts [--report <locator>] [--policy <file>] [--json] [--adopt --because <reason>]";

/// One input line: the bytes it came with, its terminator, and the text the gate reads (a leading BOM removed).
struct Ln {
    body: Vec<u8>,
    term: Vec<u8>,
    text: String,
}

fn split_lines(b: &[u8]) -> Vec<Ln> {
    let mut out: Vec<Ln> = Vec::new();
    let (mut start, n) = (0usize, b.len());
    let mut i = 0;
    let mk = |body: &[u8], term: &[u8], first: bool| {
        let mut t = String::from_utf8_lossy(body).into_owned();
        if first && let Some(rest) = t.strip_prefix('\u{feff}') {
            t = rest.to_string();
        }
        Ln { body: body.to_vec(), term: term.to_vec(), text: t }
    };
    while i < n {
        let c = b[i];
        if c == b'\n' || c == b'\r' {
            let tl = if c == b'\r' && i + 1 < n && b[i + 1] == b'\n' { 2 } else { 1 };
            out.push(mk(&b[start..i], &b[i..i + tl], out.is_empty()));
            i += tl;
            start = i;
        } else {
            i += 1;
        }
    }
    if start < n {
        out.push(mk(&b[start..n], &[], out.is_empty()));
    }
    out
}

fn is_ws(c: char) -> bool {
    matches!(c, ' ' | '\t' | '\n' | '\x0b' | '\x0c' | '\r')
}

/// The gate's own lexing of one policy line: the text before the first `#`, split at ASCII whitespace.
fn lex(text: &str) -> Vec<&str> {
    text.split('#').next().unwrap_or("").split(is_ws).filter(|s| !s.is_empty()).collect()
}

/// A line whose first non-blank characters are the verb's own comment marker — dropped and regenerated.
fn is_mark(text: &str) -> bool {
    text.trim_start_matches([' ', '\t']).starts_with(MARK)
}

/// ` #…` — the line's own trailing comment from its `#`, to carry onto a rewritten line; empty when there is none.
fn comment_of(text: &str) -> String {
    text.find('#').map(|i| format!(" {}", &text[i..])).unwrap_or_default()
}

/// Effect tokens in Unicode code-point order, an `Unknown` written as `Unknown[<its classes>]`.
fn canon(conc: &BTreeSet<String>, classes: Option<&BTreeSet<String>>) -> Vec<String> {
    let mut t: Vec<String> = conc.iter().cloned().collect();
    if let Some(cs) = classes {
        t.push(format!("Unknown[{}]", cs.iter().cloned().collect::<Vec<_>>().join(",")));
    }
    t.sort();
    t
}

fn accept_text(func: &str, effects: &[String], rule: &str, because: &str) -> String {
    let mut parts: Vec<&str> = vec!["accept", func];
    parts.extend(effects.iter().map(String::as_str));
    parts.extend(["for", rule, "because", because]);
    parts.join(" ")
}

fn policy_row(r: &GateViolation) -> bool {
    !r.rule_key.is_empty() && (r.rule == "AS-EFF-006" || r.rule == "AS-EFF-009")
}

/// Union of a set of rows' effects: concrete tokens, and whether any is `Unknown`, with the classes the rows carry.
fn union_effects<'a>(rows: impl Iterator<Item = &'a GateViolation>) -> (BTreeSet<String>, bool, BTreeSet<String>) {
    let (mut conc, mut unk, mut cls) = (BTreeSet::new(), false, BTreeSet::new());
    for r in rows {
        for x in &r.effects {
            if x == "Unknown" {
                unk = true;
            } else {
                conc.insert(x.clone());
            }
        }
        cls.extend(r.reason_class.iter().cloned());
    }
    (conc, unk, cls)
}

#[derive(Serialize)]
struct RemovedItem {
    line: usize,
    #[serde(rename = "fn")]
    func: String,
    #[serde(rename = "policyRule")]
    policy_rule: String,
    because: String,
}
#[derive(Serialize)]
struct NarrowedItem {
    line: usize,
    #[serde(rename = "fn")]
    func: String,
    #[serde(rename = "policyRule")]
    policy_rule: String,
    because: String,
    from: Vec<String>,
    to: Vec<String>,
    text: String,
}
#[derive(Serialize)]
struct ProposalItem {
    line: usize,
    #[serde(rename = "fn")]
    func: String,
    #[serde(rename = "policyRule")]
    policy_rule: String,
    because: String,
    adds: Vec<String>,
    text: String,
}
#[derive(Serialize)]
struct CreatedItem {
    #[serde(rename = "fn")]
    func: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    hash: String,
    #[serde(rename = "policyRule")]
    policy_rule: String,
    because: String,
    effects: Vec<String>,
    text: String,
}
#[derive(Serialize)]
struct UnnamedItem {
    #[serde(rename = "fn")]
    func: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    hash: String,
    #[serde(rename = "policyRule")]
    policy_rule: String,
    effects: Vec<String>,
}
#[derive(Serialize)]
struct UnjudgedItem {
    line: usize,
    #[serde(rename = "fn")]
    func: String,
    #[serde(rename = "policyRule")]
    policy_rule: String,
}
#[derive(Serialize)]
struct IgnoredItem {
    line: usize,
}
#[derive(Serialize, PartialEq, Eq)]
struct AfterItem {
    exit: i32,
    violations: usize,
    accepted: usize,
}
#[derive(Serialize)]
struct Plan {
    mode: &'static str,
    text: String,
    removed: Vec<RemovedItem>,
    narrowed: Vec<NarrowedItem>,
    proposals: Vec<ProposalItem>,
    #[serde(skip_serializing_if = "Option::is_none")]
    created: Option<Vec<CreatedItem>>,
    unnamed: Vec<UnnamedItem>,
    unjudged: Vec<UnjudgedItem>,
    ignored: Vec<IgnoredItem>,
    after: AfterItem,
}

/// `--because`: ONE argument, lexed into tokens by ASCII whitespace; at least one token; no `#` (the comment rule would
/// cut the reason short). Returns the single-spaced reason.
fn check_reason(raw: &str) -> Result<String, String> {
    if raw.contains('#') {
        return Err("--because must not contain `#` — a `#` begins a comment on a policy line, so it would cut the \
                    reason you wrote short. Reword it (the issue number without the hash, say)"
            .to_string());
    }
    let toks: Vec<&str> = raw.split(is_ws).filter(|s| !s.is_empty()).collect();
    if toks.is_empty() {
        return Err("--because needs a reason — a value with at least one token".to_string());
    }
    Ok(toks.join(" "))
}

fn usage(why: &str) -> i32 {
    eprintln!("candor-query accepts: {why}\n  {USAGE}");
    2
}

/// `accept` written as a verb: the line kind, one letter short. A usage error that names the verb meant.
pub(crate) fn cmd_accept_typo(_args: &[String]) -> i32 {
    eprintln!(
        "candor-query: `accept` is a policy LINE kind, not a verb — did you mean `accepts`?\n  {USAGE}\n  \
         (`accepts` prints a proposed policy that refreshes the `accept` lines; it writes no file)"
    );
    2
}

pub(crate) fn cmd_accepts(args: &[String]) -> i32 {
    let (mut report, mut policy, mut want_json, mut adopt, mut because_raw) =
        (None::<String>, None::<String>, false, false, None::<String>);
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--json" => want_json = true,
            "--adopt" => adopt = true,
            "--report" => match args.get(i + 1).filter(|v| v.as_str() == "-" || !v.starts_with('-')) {
                Some(v) => {
                    report = Some(resolve_locator(v));
                    i += 1;
                }
                None => return usage("--report requires a <locator> argument"),
            },
            "--policy" => match args.get(i + 1).filter(|v| v.as_str() == "-" || !v.starts_with('-')) {
                Some(v) => {
                    policy = Some(v.clone());
                    i += 1;
                }
                None => return usage("--policy requires a <file> argument"),
            },
            // ONE argument, whatever it looks like: the reason is free text.
            "--because" => match args.get(i + 1) {
                Some(v) => {
                    because_raw = Some(v.clone());
                    i += 1;
                }
                None => return usage("--because requires a reason argument"),
            },
            other if other.starts_with('-') && other.len() > 1 => {
                return usage(&format!(
                    "unknown flag `{other}` — `accepts` takes --report, --policy, --json, and --adopt with --because \
                     (it is advisory: there is no --strict, and it writes no file)"
                ));
            }
            other => return usage(&format!("unexpected argument `{other}` — `accepts` takes no positional argument")),
        }
        i += 1;
    }
    if adopt && because_raw.is_none() {
        return usage(
            "--adopt needs --because <reason>: an accepted violation carries a reason a person wrote, and this verb \
             will not compose one",
        );
    }
    if !adopt && because_raw.is_some() {
        return usage("--because only means something with --adopt (a refresh never writes a reason)");
    }
    let because = match because_raw.as_deref().map(check_reason).transpose() {
        Ok(b) => b,
        Err(why) => return usage(&why),
    };

    // ---- the policy (the gate's own ladder) and its bytes ---------------------------------------------------------
    let Some(policy_path) = resolve_policy_path(policy) else {
        return usage("a policy is required — pass `--policy <file>`, set CANDOR_POLICY, or add a `policy` key to .candor/config");
    };
    let raw = match std::fs::read(&policy_path) {
        Ok(b) => b,
        Err(e) => {
            eprintln!("candor-query accepts: cannot read policy {policy_path} ({e}) — no proposal.");
            return 2;
        }
    };
    let Ok(policy_text) = String::from_utf8(raw.clone()) else {
        eprintln!("candor-query accepts: cannot read policy {policy_path} (not valid UTF-8) — no proposal.");
        return 2;
    };
    let lines = split_lines(&raw);

    // ---- 1. the gate over the input bytes ---------------------------------------------------------------------------
    let first = match gate_core(&policy_text, &policy_path, report.clone(), CoreMode::Accepts { verbose: true }) {
        Ok(c) => c,
        Err(stop) => {
            eprintln!("candor-query accepts: the gate refuses this policy and report (exit {}) — no proposal. The policy was not changed.", stop.code);
            if stop.no_rules {
                eprintln!(
                    "candor-query accepts: this policy holds no rules. If it is empty because you ran \
                     `candor-query accepts > {policy_path}`, a shell redirect onto the policy EMPTIES it before the verb \
                     reads it — the policy file is now empty; restore it (e.g. `git checkout -- {policy_path}`) and \
                     write the proposal to ANOTHER path, then diff."
                );
            }
            return 2;
        }
    };
    if run_is_incomplete(&first.rep, &first.unasked_rules) {
        eprintln!(
            "candor-query accepts: the analysis is INCOMPLETE (a unit it did not read makes its entries look stale, and \
             pruning them would delete the reasons for code that still exists) — no proposal. The policy was not changed."
        );
        return 2;
    }

    // ---- 2. what each entry is, from the gate's own settlement ------------------------------------------------------
    let aux = first.aux.as_ref();
    let rows_left: &[GateViolation] = aux.map(|a| a.remaining.as_slice()).unwrap_or(&first.violations);
    let mut status: BTreeMap<usize, String> = BTreeMap::new();
    for it in first.accepted.as_deref().unwrap_or(&[]).iter().filter(|it| it.enclosing.is_empty()) {
        status.insert(it.line, it.status.clone());
    }
    let stale: BTreeSet<usize> = aux.map(|a| a.stale_lines.iter().copied().collect()).unwrap_or_default();
    // Only an `accept` line the gate ignored (malformed, or naming a rule line that no longer parses).
    let ignored: BTreeSet<usize> = first
        .ignored
        .iter()
        .filter(|g| lex(&g.text).first() == Some(&"accept"))
        .map(|g| g.line)
        .collect();

    let (mut removed, mut narrowed, mut proposals, mut unjudged) = (Vec::new(), Vec::new(), Vec::new(), Vec::new());
    let mut new_text: BTreeMap<usize, String> = BTreeMap::new();
    let mut mark_after: BTreeMap<usize, String> = BTreeMap::new();
    let mut notes: Vec<String> = Vec::new();
    let mut entries: Vec<&AcceptEntry> = first.p.accepts.iter().collect();
    entries.sort_by_key(|e| e.line);
    for e in entries {
        let st = status.get(&e.line).map(String::as_str).unwrap_or(if stale.contains(&e.line) { "stale" } else { "kept" });
        let src = lines.get(e.line.wrapping_sub(1)).map(|l| l.text.as_str()).unwrap_or("");
        match st {
            "stale" => removed.push(RemovedItem {
                line: e.line,
                func: e.func.clone(),
                policy_rule: e.rule_key.clone(),
                because: e.because.clone(),
            }),
            "accepted" => {
                let direct = aux.and_then(|a| a.direct_rows.iter().find(|(l, _)| *l == e.line)).map(|(_, r)| r.as_slice());
                let Some(direct) = direct else { continue };
                let (conc, unk, cls) = union_effects(direct.iter());
                // An `Unknown` row accepted by an entry always has classes (an undeterminable one is covered by no filter).
                let cls = if unk && cls.is_empty() { e.classes.clone().unwrap_or_default() } else { cls };
                let to = canon(&conc, unk.then_some(&cls));
                let from_conc: BTreeSet<String> =
                    e.effects.iter().filter(|x| **x != "Unknown").map(|x| x.to_string()).collect();
                let from = canon(&from_conc, e.classes.as_ref());
                if to.iter().collect::<BTreeSet<_>>() != from.iter().collect::<BTreeSet<_>>() {
                    let text = accept_text(&e.func, &to, &e.rule_key, &e.because) + &comment_of(src);
                    narrowed.push(NarrowedItem {
                        line: e.line,
                        func: e.func.clone(),
                        policy_rule: e.rule_key.clone(),
                        because: e.because.clone(),
                        from,
                        to,
                        text: text.clone(),
                    });
                    new_text.insert(e.line, text);
                }
            }
            "exceeded" => {
                let mine: Vec<&GateViolation> =
                    rows_left.iter().filter(|r| policy_row(r) && r.rule_key == e.rule_key && r.func == e.func).collect();
                let (conc, unk, cls) = union_effects(mine.iter().copied());
                if unk && cls.is_empty() {
                    // The row's classes are not determinable: no line could name them, so no wider line is proposed.
                    notes.push(format!(
                        "line {} (`{}`) is EXCEEDED by an `Unknown` whose reason classes this report does not carry — \
                         no wider line can be written for it; kept as is, and the row stays red.",
                        e.line, e.func
                    ));
                    continue;
                }
                let prop = canon(&conc, unk.then_some(&cls));
                let mut adds: Vec<String> = conc.iter().filter(|x| !e.effects.contains(x.as_str())).cloned().collect();
                if unk {
                    let extra: Vec<String> = cls
                        .iter()
                        .filter(|c| e.classes.as_ref().is_none_or(|have| !have.contains(*c)))
                        .cloned()
                        .collect();
                    if !extra.is_empty() {
                        adds.push(format!("Unknown[{}]", extra.join(",")));
                    }
                }
                adds.sort();
                let text = accept_text(&e.func, &prop, &e.rule_key, &e.because) + &comment_of(src);
                mark_after.insert(e.line, format!("{MARK} exceeded (adds {}){REPLACE}{text}", adds.join(" ")));
                proposals.push(ProposalItem {
                    line: e.line,
                    func: e.func.clone(),
                    policy_rule: e.rule_key.clone(),
                    because: e.because.clone(),
                    adds,
                    text,
                });
            }
            "unjudged" => unjudged.push(UnjudgedItem { line: e.line, func: e.func.clone(), policy_rule: e.rule_key.clone() }),
            _ => {} // ambiguous, or a state the gate did not name: kept byte for byte
        }
    }
    let removed_lines: BTreeSet<usize> = removed.iter().map(|r| r.line).collect();

    // ---- 3. the rows no entry names (`unnamed`; `--adopt` writes a line for each) -----------------------------------------
    let mut rule_line: BTreeMap<String, usize> = BTreeMap::new();
    for (n, l) in lines.iter().enumerate() {
        let t = lex(&l.text);
        if t.first().is_some_and(|k| RULE_KINDS.contains(k)) {
            let cut = l.text.split('#').next().unwrap_or("").trim_matches(is_ws);
            rule_line.entry(rule_key(cut)).or_insert(n + 1);
        }
    }
    let twins = candor_classify::accept::twin_names(&run_units(&first.sig));
    struct Cand {
        line: usize,
        func: String,
        hash: String,
        rule: String,
        effects: Vec<String>,
    }
    let mut cands: Vec<Cand> = Vec::new();
    let mut seen: BTreeSet<(String, String)> = BTreeSet::new();
    for r in rows_left.iter().filter(|r| policy_row(r)) {
        if first.p.accepts.iter().any(|e| e.func == r.func && e.rule_key == r.rule_key) {
            continue; // an entry names it already: an exceeded row is PROPOSED, never adopted
        }
        if !seen.insert((r.rule_key.clone(), r.func.clone())) || twins.contains(&r.func) {
            continue;
        }
        if is_positional_key(&r.func) || r.func.chars().any(|c| c == '#' || is_ws(c)) {
            continue; // a line the gate would not honour: a nested unit rides its enclosing entry instead
        }
        let (conc, unk, cls) = union_effects(std::iter::once(r));
        if unk && cls.is_empty() {
            continue;
        }
        cands.push(Cand {
            line: rule_line.get(&r.rule_key).copied().unwrap_or(usize::MAX),
            func: r.func.clone(),
            hash: r.hash.clone(),
            rule: r.rule_key.clone(),
            effects: canon(&conc, unk.then_some(&cls)),
        });
    }
    cands.sort_by(|a, b| (a.line, &a.func).cmp(&(b.line, &b.func)));
    let unnamed: Vec<UnnamedItem> = cands
        .iter()
        .map(|c| UnnamedItem { func: c.func.clone(), hash: c.hash.clone(), policy_rule: c.rule.clone(), effects: c.effects.clone() })
        .collect();
    let created: Vec<CreatedItem> = match (adopt, &because) {
        (true, Some(b)) => cands
            .iter()
            .map(|c| CreatedItem {
                func: c.func.clone(),
                hash: c.hash.clone(),
                policy_rule: c.rule.clone(),
                because: b.clone(),
                effects: c.effects.clone(),
                text: accept_text(&c.func, &c.effects, &c.rule, b),
            })
            .collect(),
        _ => Vec::new(),
    };

    // ---- 4. the proposed text ---------------------------------------------------------------------------------------------
    let mut body: Vec<u8> = Vec::new();
    for (n, l) in lines.iter().enumerate() {
        let ln = n + 1;
        if is_mark(&l.text) || removed_lines.contains(&ln) {
            continue;
        }
        match new_text.get(&ln) {
            Some(t) => body.extend_from_slice(t.as_bytes()),
            None => body.extend_from_slice(&l.body),
        }
        body.extend_from_slice(&l.term);
        if let Some(m) = mark_after.get(&ln) {
            if l.term.is_empty() {
                body.push(b'\n');
            }
            body.extend_from_slice(m.as_bytes());
            body.extend_from_slice(&l.term);
        }
    }
    if !created.is_empty() {
        if body.last().is_some_and(|b| *b != b'\n' && *b != b'\r') {
            body.push(b'\n');
        }
        for c in &created {
            body.extend_from_slice(c.text.as_bytes());
            body.push(b'\n');
        }
    }
    let text = String::from_utf8_lossy(&body).into_owned();

    // ---- 5. `after`: the gate's own verdict over the proposal ---------------------------------------------------------------
    let second = match gate_core(&text, &policy_path, report, CoreMode::Accepts { verbose: false }) {
        Ok(c) => c,
        Err(stop) => {
            eprintln!(
                "candor-query accepts: the gate REFUSES the proposal it would have printed ({}) — printing nothing. This \
                 is a defect in `accepts`, not in your policy; the policy was not changed.",
                stop.why
            );
            return 2;
        }
    };
    // `after` describes the DOCUMENT the gate writes over the proposal. A sole refusal (nothing fired and a rule could not
    // be evaluated) writes a refusal document, which carries no `accepted` list and no violations; any other ending
    // carries the settlement the run made.
    let tail = tail_kind(&second.violations, &second.refused, &second.rep, &second.unasked_rules);
    let document_carries_accepted = tail != Tail::SoleRefusal;
    let after = AfterItem {
        exit: tail.exit(),
        violations: second.violations.len(),
        accepted: if document_carries_accepted {
            second.accepted.as_deref().unwrap_or(&[]).iter().filter(|i: &&AcceptedItem| i.status == "accepted").count()
        } else {
            0
        },
    };

    // ---- 6. output ------------------------------------------------------------------------------------------------------
    use std::io::Write;
    let ignored_out: Vec<IgnoredItem> = ignored.iter().map(|&line| IgnoredItem { line }).collect();
    let counts = format!(
        "{} stale line(s) removed, {} narrowed, {} exceeded (proposed in a comment, NOT accepted), {} created; {} violation(s) \
         no entry names; after the proposal the gate exits {} with {} violation(s), {} accepted.",
        removed.len(),
        narrowed.len(),
        proposals.len(),
        created.len(),
        unnamed.len(),
        after.exit,
        after.violations,
        after.accepted
    );
    let (n_unjudged, ignored_lines) = (unjudged.len(), ignored.clone());
    let unjudged_say: Vec<String> = unjudged
        .iter()
        .map(|u| {
            format!(
                "candor-query accepts: line {} (`{}` under `{}`) kept UNJUDGED — this route cannot evaluate that rule, so nothing is known about the entry. Gate at scan time.",
                u.line, u.func, u.policy_rule
            )
        })
        .collect();
    let mode = if adopt { "adopt" } else { "refresh" };
    let stdout = std::io::stdout();
    let mut out = stdout.lock();
    let wrote = if want_json {
        let plan = Plan {
            mode,
            text,
            removed,
            narrowed,
            proposals,
            created: adopt.then_some(created),
            unnamed,
            unjudged,
            ignored: ignored_out,
            after,
        };
        match serde_json::to_string_pretty(&plan) {
            Ok(j) => out.write_all(format!("{j}\n").as_bytes()),
            Err(e) => {
                eprintln!("candor-query accepts: could not serialize the plan ({e})");
                return 2;
            }
        }
    } else {
        out.write_all(&body)
    };
    let _ = out.flush();
    if let Err(e) = wrote {
        eprintln!("candor-query accepts: could not write the proposal to stdout ({e})");
        return 2;
    }
    eprintln!("candor-query accepts ({mode}): {counts}");
    for n in &notes {
        eprintln!("candor-query accepts: {n}");
    }
    debug_assert_eq!(n_unjudged, unjudged_say.len());
    for u in &unjudged_say {
        eprintln!("{u}");
    }
    for l in ignored_lines {
        eprintln!("candor-query accepts: line {l} is IGNORED by the gate and kept as written — repairing it is yours.");
    }
    eprintln!(
        "candor-query accepts: nothing was written. To take the proposal: write it to ANOTHER path (not onto the policy — a \
         shell redirect onto it empties it first), then `diff {policy_path} <that path>` and commit."
    );
    0
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every byte of the input lands in exactly one body or terminator, and the split is the gate's own (LF, CRLF and a
    /// lone CR each end a line) — the gate's `line` numbers must be this verb's.
    #[test]
    fn split_keeps_every_byte_and_splits_where_the_gate_does() {
        let src = b"a\r\nb\rc\n\nd";
        let ls = split_lines(src);
        let joined: Vec<u8> = ls.iter().flat_map(|l| l.body.iter().chain(l.term.iter()).copied()).collect();
        assert_eq!(joined, src);
        let texts: Vec<&str> = ls.iter().map(|l| l.text.as_str()).collect();
        assert_eq!(texts, ["a", "b", "c", "", "d"]);
        assert_eq!(ls[0].term, b"\r\n");
        assert_eq!(ls[1].term, b"\r");
        assert!(ls[4].term.is_empty(), "a last line with no terminator stays without one");
    }

    #[test]
    fn a_leading_bom_is_kept_in_the_bytes_and_dropped_from_the_text_the_gate_reads() {
        let ls = split_lines("\u{feff}deny Fs\nx".as_bytes());
        assert_eq!(ls[0].text, "deny Fs");
        assert!(ls[0].body.starts_with("\u{feff}".as_bytes()));
        assert_eq!(ls[1].text, "x");
    }

    #[test]
    fn effect_tokens_are_in_code_point_order_with_unknown_carrying_its_classes() {
        let conc: BTreeSet<String> = ["Fs", "Clock"].iter().map(|s| s.to_string()).collect();
        let cls: BTreeSet<String> = ["native", "dispatch"].iter().map(|s| s.to_string()).collect();
        assert_eq!(canon(&conc, Some(&cls)), ["Clock", "Fs", "Unknown[dispatch,native]"]);
        assert_eq!(canon(&conc, None), ["Clock", "Fs"]);
    }

    #[test]
    fn the_reason_is_one_argument_lexed_by_ascii_whitespace_with_no_hash() {
        assert_eq!(check_reason("  a \t b\n").unwrap(), "a b");
        assert!(check_reason("  ").is_err());
        assert!(check_reason("see #1").is_err());
    }

    #[test]
    fn marker_lines_are_recognised_only_after_blanks_and_a_comment_keeps_its_own_hash() {
        assert!(is_mark("   \t# candor-accepts: exceeded"));
        assert!(!is_mark("accept x # candor-accepts:"));
        assert_eq!(comment_of("accept a Fs for r because x  # why"), " # why");
        assert_eq!(comment_of("accept a Fs for r because x"), "");
    }
}
