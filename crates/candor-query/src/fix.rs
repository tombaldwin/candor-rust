//! `candor fix` / `candor fix-gate` — the boundary fix (integrations/FIX-SPEC.md). When an edit makes a
//! function perform an effect its layer forbids, this computes the *architectural* remedy: where the effect
//! should live (hoist it to the nearest allowed-layer caller) and which functions become pure and thread the
//! value. Read-only over the same report + policy the gate uses; the inverse of `whatif`. `fix` answers for
//! one function; `fix-gate` computes a remedy for every deny/`pure` (AS-EFF-006) violation in a report, so
//! the edit-time loop can hand the agent the *fix*, not just the finding. Advisory structure, never syntax;
//! the gate re-scan remains the ground truth.

use crate::grammar::{parse, report_or_discover, Shape};
use crate::load::load_entries;
use crate::matching::{best_tier, q_match};
use candor_report::ReportEntry;
use std::collections::{BTreeMap, BTreeSet, HashMap, VecDeque};

/// The deny/`pure` scope (the "layer") that forbids `effect` at `fname`, or None if performing `effect`
/// there is allowed. Mirrors `whatif`'s violation predicate exactly (SPEC §6): a `deny` fires when it names
/// the effect; a `pure` rule (empty effects) forbids every real effect but not `Unknown`.
fn denied_layer(fname: &str, effect: &str, rules: &[candor_classify::policy::PolicyRule]) -> Option<String> {
    for rule in rules {
        let denies = if rule.effects.is_empty() {
            effect != candor_classify::policy::UNKNOWN
        } else {
            rule.effects.contains(effect)
        };
        let in_scope = rule
            .scope
            .as_deref()
            .is_none_or(|s| candor_classify::policy::scope_matches(fname, s));
        if denies && in_scope {
            return Some(rule.scope.clone().unwrap_or_default());
        }
    }
    None
}

/// ⟨0.24⟩ [`denied_layer`] asked of a REAL function with a REAL signature, so a narrowing filter is
/// answered rather than ignored.
///
/// `denied_layer` above takes a NAME and a HYPOTHETICAL effect — "if this function performed `Net`,
/// would its layer forbid it?" — which is the right question for `fix`'s climb, where the callers being
/// classified into the pure span and the hoist frontier are being asked about a layer, not about
/// evidence they carry. It cannot answer a `deny Net[unknown-host]` / `deny Unknown[reflect]` rule,
/// because the filter quantifies over the FUNCTION'S destination and reason classes and a hypothetical
/// has none.
///
/// `fix-gate`'s enumeration is not hypothetical: it walks the report's own entries and asks which
/// `(function, effect)` pairs TRIP a rule — the same question the gate answers, and it must not answer
/// it differently. MEASURED 2026-07-28: `deny Unknown[reflect]` over an `indirect` hole → the gate exits
/// 0 and `fix-gate` named a hoist remedy for a crossing that does not exist, which is a boundary
/// refactor proposed to an agent on the strength of a rule the operator narrowed to exclude it.
///
/// So this arm goes through [`candor_classify::gate::rule_hits`], the gate's own firing decision. A
/// WITHHELD filter counts as NOT charged: `fix-gate` is a remedy for a violation, and there is no
/// violation to remedy where the gate declined to evaluate — that fact travels on the gate's own
/// refusal, which is the document that exists to carry it.
fn denied_layer_evidenced(
    e: &ReportEntry,
    effect: &str,
    rules: &[candor_classify::policy::PolicyRule],
    reason_classes: Option<&BTreeSet<String>>,
) -> Option<String> {
    let effs: Vec<&str> = e.inferred.iter().map(String::as_str).collect();
    for rule in rules {
        let in_scope = rule
            .scope
            .as_deref()
            .is_none_or(|s| candor_classify::policy::scope_matches(&e.func, s));
        if !in_scope {
            continue;
        }
        if candor_classify::gate::rule_hits(rule, &effs, reason_classes, &e.net_class).hits.contains(&effect) {
            return Some(rule.scope.clone().unwrap_or_default());
        }
    }
    None
}

/// The computed remedy for one `(function, effect)` boundary crossing — the deterministic cut between
/// "must stay pure" (`denied_span`) and "may perform the effect" (`hoist_to`). Borrows the report.
pub(crate) struct RemedyPlan<'a> {
    func: &'a str,
    effect: &'a str,
    layer: String,
    sites: BTreeSet<&'a str>,
    denied_span: BTreeSet<&'a str>,
    hoist_to: BTreeSet<&'a str>,
    hoist_higher: BTreeSet<&'a str>,
    clean_hoist: bool,
    allow_edit: String,
}

impl RemedyPlan<'_> {
    fn clean_hoist(&self) -> bool {
        self.clean_hoist
    }
    /// A hoist frontier exists, but it's SANDWICHED — a forbidden fn calls into it, so hoisting there would
    /// leave that caller violating. Distinguishes the two no-clean-hoist shapes in the message.
    fn sandwiched(&self) -> bool {
        !self.clean_hoist && !self.hoist_to.is_empty()
    }
    /// A stable key so `fix-gate` collapses the many inheritors of one root cause (every function in the
    /// denied span carries the effect) to a single remedy: the plan is fixed by its effect, layer, site, and
    /// hoist target — not by which inheriting function tripped the gate.
    fn dedup_key(&self) -> String {
        format!(
            "{}|{}|{:?}|{:?}",
            self.effect,
            self.layer,
            self.sites.iter().collect::<Vec<_>>(),
            self.hoist_to.iter().collect::<Vec<_>>()
        )
    }
    fn to_json(&self) -> serde_json::Value {
        serde_json::json!({
            "fn": self.func,
            "effect": self.effect,
            "layer": self.layer,
            "cleanHoist": self.clean_hoist(),
            "site": self.sites.iter().collect::<Vec<_>>(),
            "deniedSpan": self.denied_span.iter().collect::<Vec<_>>(),
            "hoistTo": self.hoist_to.iter().collect::<Vec<_>>(),
            "hoistHigher": self.hoist_higher.iter().collect::<Vec<_>>(),
            "policyAlternative": self.allow_edit,
        })
    }
    fn render_text(&self, out: &mut String) {
        use std::fmt::Write;
        // An empty scope is a GLOBAL rule (`deny Exec` with no layer — forbid it program-wide); render a
        // real word, not the literal "this", which read like an unsubstituted template variable (#12a).
        let layer_label = if self.layer.is_empty() { "global".to_string() } else { format!("`{}`", self.layer) };
        let sitelist = if self.sites.is_empty() {
            "(not a local source — a cross-crate or Unknown effect)".to_string()
        } else {
            self.sites.iter().map(|x| format!("`{x}`")).collect::<Vec<_>>().join(", ")
        };
        let _ = writeln!(out, "candor fix — hoist {} out of the {layer_label} boundary\n", self.effect);
        let _ = writeln!(out, "  The violation: `{}` performs {}, which the {layer_label} layer forbids.", self.func, self.effect);
        let _ = writeln!(out, "  Performed directly at: {sitelist}");
        let span: Vec<_> = self.denied_span.iter().take(6).map(|x| format!("`{x}`")).collect();
        let more = if self.denied_span.len() > 6 { ", …" } else { "" };
        let _ = writeln!(out, "  Forbidden across {} function(s) in the layer (they inherited it): {}{more}", self.denied_span.len(), span.join(", "));
        let _ = writeln!(out);
        if self.clean_hoist() {
            let _ = writeln!(out, "  THE FIX — hoist the effect to the boundary:");
            let _ = writeln!(out, "    · Perform {} at: {}  (an allowed layer that already calls into the domain).", self.effect,
                self.hoist_to.iter().map(|x| format!("`{x}`")).collect::<Vec<_>>().join(", "));
            let _ = writeln!(out, "    · Pass the result down as a parameter; the {} function(s) above then stay pure.", self.denied_span.len());
            let _ = writeln!(out, "    · Re-run the gate — the {layer_label} blast radius for {} should be empty.", self.effect);
            if !self.hoist_higher.is_empty() {
                let tops: Vec<_> = self.hoist_higher.iter().take(4).map(|x| format!("`{x}`")).collect();
                let more = if self.hoist_higher.len() > 4 { ", …" } else { "" };
                let _ = writeln!(out, "    · TRADE-OFF — or hoist higher (up to {}{more}): the effect then originates further up,", tops.join(", "));
                let _ = writeln!(out, "      keeping the {} intervening allowed-layer function(s) pure too, at the cost of threading it through more signatures.", self.hoist_higher.len());
            }
            let _ = writeln!(out);
            let _ = writeln!(out, "  ALTERNATIVE — if the {layer_label} layer is MEANT to perform {}, it's a policy bug,", self.effect);
            let _ = writeln!(out, "  not a code one: relax the boundary with  `{}`.", self.allow_edit);
        } else {
            if self.sandwiched() {
                let _ = writeln!(out, "  NO CLEAN HOIST — the nearest allowed layer ({}) is itself CALLED BY a {}-forbidding layer,",
                    self.hoist_to.iter().map(|x| format!("`{x}`")).collect::<Vec<_>>().join(", "), self.effect);
                let _ = writeln!(out, "  so hoisting {} there would leave that caller violating (a forbidden layer sandwiching an allowed one).", self.effect);
            } else {
                let _ = writeln!(out, "  NO CLEAN HOIST — every caller up to the entry points is also in a {}-forbidding layer.", self.effect);
            }
            let _ = writeln!(out, "  Three ways to fix it:");
            let _ = writeln!(out, "    (a) HOIST TO A NEW ENTRY POINT (recommended) — add a thin function ABOVE the {layer_label} layer that");
            let _ = writeln!(out, "        performs {} and passes the result DOWN as plain DATA; the {layer_label} functions take it as a", self.effect);
            let _ = writeln!(out, "        parameter and become PROVABLY pure (candor verifies no effect — clean under any policy). candor");
            let _ = writeln!(out, "        says \"no clean hoist\" only because no allowed caller EXISTS yet — you can add one; simplest fix.");
            let _ = writeln!(out, "    (b) INJECT via a fn/closure — give the {layer_label} layer a FUNCTION/CLOSURE parameter, supplied by an");
            let _ = writeln!(out, "        allowed adapter. This clears `deny {}`, but candor can't see THROUGH the injected function, so it", self.effect);
            let _ = writeln!(out, "        reads the {layer_label} as Unknown — a hole a `deny {} Unknown` policy would still flag; prefer (a) for", self.effect);
            let _ = writeln!(out, "        provable purity. Do NOT use a trait/interface port: candor resolves the dispatch back to its");
            let _ = writeln!(out, "        {}-performing impl, so the {layer_label} still trips the gate.", self.effect);
            let _ = writeln!(out, "    (c) If the {layer_label} layer legitimately needs {}, relax the boundary:  `{}`.", self.effect, self.allow_edit);
        }
    }
}

/// The cut itself — pure over the report graph. `start` performs `effect` and sits in a deny-`effect` layer
/// (`layer`); `rev` is the callee→callers adjacency. Returns the site(s), the denied span, and the hoist
/// frontier. Shared by `cmd_fix` (one function) and `cmd_fix_gate` (every violation).
fn compute_remedy<'a>(
    by_name: &HashMap<&'a str, &'a ReportEntry>,
    rev: &BTreeMap<&'a str, Vec<&'a str>>,
    rules: &[candor_classify::policy::PolicyRule],
    start: &'a ReportEntry,
    effect: &'a str,
    layer: String,
) -> RemedyPlan<'a> {
    // direct site(s) S: BFS from `start` through effect-carrying callees to the DIRECT source(s).
    let mut sites: BTreeSet<&str> = BTreeSet::new();
    let mut fseen: BTreeSet<&str> = BTreeSet::new();
    let mut q: VecDeque<&str> = VecDeque::new();
    q.push_back(start.func.as_str());
    fseen.insert(start.func.as_str());
    while let Some(cur) = q.pop_front() {
        let Some(f) = by_name.get(cur) else { continue };
        if f.direct.iter().any(|e| e == effect) {
            sites.insert(cur);
        }
        for c in &f.calls {
            if let Some(cf) = by_name.get(c.as_str())
                && cf.inferred.iter().any(|e| e == effect)
                && fseen.insert(c.as_str())
            {
                q.push_back(c.as_str());
            }
        }
    }

    // ANCHOR on the site(s) (fall back to `start` for a cross-crate/Unknown source with no local site) and
    // walk UP: denied-layer effect-carriers are the pure span; the allowed-layer callers where the climb
    // stops are the hoist frontier. Site-anchored so the span is the SAME whichever inheriting function
    // triggered it (root-independent) — the inheritors of one crossing collapse to one identical remedy.
    let anchors: Vec<&str> = if sites.is_empty() { vec![start.func.as_str()] } else { sites.iter().copied().collect() };
    let mut denied_span: BTreeSet<&str> = BTreeSet::new();
    let mut hoist_to: BTreeSet<&str> = BTreeSet::new();
    let mut up: VecDeque<&str> = VecDeque::new();
    for &a in &anchors {
        if denied_layer(a, effect, rules).is_some() {
            denied_span.insert(a); // a site that is itself in the denied layer
        }
        up.push_back(a);
    }
    while let Some(cur) = up.pop_front() {
        if let Some(cs) = rev.get(cur) {
            for &caller in cs {
                // skip a caller that doesn't route the effect — INCLUDING one absent from the report (a pure
                // callgraph-only node never carries the effect). Matches candor-swift; avoids classifying a
                // pure node into the span/hoist. (/code-review — was `if let Some(ce) = … && !…`.)
                let Some(ce) = by_name.get(caller) else { continue };
                if !ce.inferred.iter().any(|e| e == effect) {
                    continue;
                }
                if denied_layer(caller, effect, rules).is_some() {
                    if denied_span.insert(caller) {
                        up.push_back(caller); // denied → part of the span; keep climbing
                    }
                } else {
                    hoist_to.insert(caller); // allowed → the boundary; the effect should originate here
                }
            }
        }
    }

    // higher hoist options: allowed-layer transitive callers of the minimal frontier that also route the
    // effect — the places you COULD originate it instead. Hoisting higher keeps the frontier pure too, at the
    // cost of threading the value through more signatures (FIX-SPEC: the trade-off, disclosed not hidden).
    // The SANDWICHED-layer check (/code-review): a hoist is CLEAN only if no forbidden function sits ABOVE
    // the frontier. If a denied fn calls into a hoist target (D1 → A, A the frontier), then hoisting the
    // effect to A leaves D1 still inheriting it — so it isn't a clean hoist. Detected in the same upward
    // climb that gathers `hoist_higher` (which collects the allowed ancestors).
    let mut hoist_higher: BTreeSet<&str> = BTreeSet::new();
    let mut sandwiched = false;
    let mut hq: VecDeque<&str> = hoist_to.iter().copied().collect();
    let mut hseen: BTreeSet<&str> = hoist_to.iter().copied().collect();
    while let Some(cur) = hq.pop_front() {
        if let Some(cs) = rev.get(cur) {
            for &caller in cs {
                let Some(ce) = by_name.get(caller) else { continue };
                if !ce.inferred.iter().any(|e| e == effect) {
                    continue;
                }
                if denied_layer(caller, effect, rules).is_some() {
                    sandwiched = true; // a forbidden fn calls into the frontier — hoisting there wouldn't clear it
                } else if hseen.insert(caller) {
                    hoist_higher.insert(caller);
                    hq.push_back(caller);
                }
            }
        }
    }
    let clean_hoist = !hoist_to.is_empty() && !sandwiched;

    let allow_edit = if layer.is_empty() {
        format!("allow {effect}")
    } else {
        format!("allow {effect} {layer}")
    };
    RemedyPlan { func: &start.func, effect, layer, sites, denied_span, hoist_to, hoist_higher, clean_hoist, allow_edit }
}

/// Read + parse a policy, loud-failing (exit 2) on an unreadable path — the same fail-loud contract as
/// `whatif`, so a typo'd policy never yields a confident plan against a silently-empty ruleset. Returns the
/// deny/`pure` rules on success.
/// Returns the resolved policy PATH beside the parse — the ⟨0.28⟩ zero-rule caveat names the whole
/// policy, so the callers need to know which file that was.
fn load_rules(policy_path: Option<String>) -> Result<(String, candor_classify::policy::ParsedPolicy), i32> {
    let policy_path = policy_path.or_else(|| std::env::var("CANDOR_POLICY").ok());
    let Some(pp) = policy_path else {
        eprintln!("candor fix: a policy is required (pass a policy file or set CANDOR_POLICY) — the fix is the refactor that restores the boundary the edit crossed.");
        return Err(2);
    };
    // ⟨0.24⟩ Through the SHARED loader (`crate::policy::load_policy_as_the_gate_does`): a remedy computed
    // from a rule the gate does not apply sends an agent to refactor a boundary the gate never asked
    // about. MEASURED — `deny Unknown[corp]` with `corp` aliased to a non-matching class produced a hoist
    // plan while the gate exited 0.
    crate::policy::load_policy_as_the_gate_does("fix", &pp).map(|p| (pp, p))
}

/// ⟨0.24⟩ **A REMEDY MUST NOT REST ON EVIDENCE THE GATE REFUSED TO READ** — SPEC §3.2, candor-spec
/// `4fd140c`: *"`fix-gate` MUST NOT offer a remedy premised on evidence the gate refused to read. A hoist
/// plan for a boundary the gate could not adjudicate is a confident instruction resting on a guess."*
///
/// The pairs come from [`crate::gate::unanswerable_pairs`] — `gate --report`'s OWN answerability
/// predicate, not a copy of it — so "which boundaries are off limits" is decided once for the gate and
/// both advisory verbs.
///
/// MEASURED on this engine before the ruling, over the R11 report (`hosts`, no `netClass`) under
/// `deny Net[unknown-host] app`: `gate --report` exited 2 and `fix-gate` printed `{"ok": true,
/// "remedies": []}`, exit 0 — an unqualified all-clear over bytes the gate declined to judge. The
/// remedy itself was already withheld (`denied_layer_evidenced` goes through the gate's `rule_hits`,
/// which WITHHOLDS the hit); what was missing was that the verb SAID SO, and `ok: true` said the
/// opposite. `fix` (one function) was worse: it went through the filter-BLIND `denied_layer` and printed
/// a full hoist plan for `app.noClass`.
///
/// **`ok` IS OMITTED, NOT SET TO FALSE** — the shape SPEC §3.2 `0075987` fixed for `whatif`, copied for
/// its REASONING and not its familiarity. `ok: true` asserts there is no boundary crossing, over a
/// boundary nothing adjudicated; `ok: false` would assert a crossing the analysis never found, which is
/// the fabrication mirror and worse than what it replaces. So the key goes away, `unevaluated` takes its
/// place, and a consumer writing `if (r.ok)` gets falsy and fails safe.
///
/// The two verbs below therefore call [`crate::gate::unanswerable_pairs`] directly, and this is where
/// the reasoning lives.
///
/// ⟨0.24⟩ The `unevaluated` disclosure for a remedy document — the gate's `[{rule, why}]` shape (SPEC
/// §3.1 `fc4b5f6`), one entry per RULE, never a second spelling.
fn unevaluated_json(unanswered: &[crate::gate::Unanswerable]) -> Vec<serde_json::Value> {
    let mut seen: BTreeSet<&str> = BTreeSet::new();
    unanswered
        .iter()
        .filter(|u| seen.insert(u.rule.as_str()))
        .map(|u| serde_json::json!({ "rule": u.rule, "why": u.why }))
        .collect()
}

/// Build the callee→callers adjacency from the embedded call lists.
fn reverse_graph(entries: &[ReportEntry]) -> BTreeMap<&str, Vec<&str>> {
    let mut rev: BTreeMap<&str, Vec<&str>> = BTreeMap::new();
    for e in entries {
        for c in &e.calls {
            rev.entry(c.as_str()).or_default().push(e.func.as_str());
        }
    }
    rev
}

pub(crate) fn cmd_fix(args: &[String]) -> i32 {
    let g = parse(args, Shape { verb_args: 2, sentinel: true, has_policy: true });
    let (Some(target), Some(effect)) = (g.positional.first().cloned(), g.positional.get(1).cloned()) else {
        eprintln!("usage: candor-query fix <fn> <Effect> [--report <locator>] [--policy <file>] [--json]");
        return 2;
    };
    let (target, effect) = (&target, effect.as_str());
    if candor_classify::cap_from_name(effect).is_none() && effect != "Unknown" {
        eprintln!("candor: unknown effect `{effect}` (expected a candor effect name, e.g. Net/Fs/Db/Exec, or Unknown)");
        return 2;
    }
    let Some(prefix) = report_or_discover(&g) else {
        eprintln!("candor: no report found (no --report and no .candor/ discovered) — scan the crate first.");
        return 2;
    };
    let prefix = &prefix;
    let want_json = g.want_json;
    let policy_path = g.policy.clone();
    let (pp_path, parsed) = match load_rules(policy_path) {
        Ok(p) => p,
        Err(c) => return c,
    };
    let rules = &parsed.rules;

    let entries = load_entries(prefix);
    if entries.is_empty() {
        eprintln!("candor fix: no report for `{prefix}` — scan the crate first.");
        return 2;
    }
    let by_name: HashMap<&str, &ReportEntry> = entries.iter().map(|e| (e.func.as_str(), e)).collect();

    // ⟨0.24⟩ WHAT THE PRODUCING SCAN COULD NOT SEE (SPEC §3.2 `ec1a441`, [`crate::completeness`]).
    // `fix` is NOT one of the verbs `ec1a441` names, because it answers no `ok` — but it is the same
    // harm one verb over, which is how `4fd140c` reached it too, and every one of its answers is a claim
    // over the report: *"does not perform E — nothing to hoist"* rests on an effect set accumulated over
    // the callgraph (a callee in an unread file contributes nothing), and a hoist plan names the CALLERS
    // to move the effect to (a caller in an unread file is missing from `site`/`hoistTo`).
    //
    // So the DISCLOSURE reaches every one of them — on stderr when stdout carries a document, since two
    // of the four answers are prose in BOTH modes — and `incomplete`/`unanalyzed` ride the documents. The
    // EXIT CODE stays 0: this verb answers no `ok` for `--strict` to follow, its sibling refusal branch
    // below deliberately exits 0 for the same reason (`4fd140c`), and a second, contradictory exit
    // policy inside one verb would say the gate's refusal is the milder finding. candor-ts agrees —
    // measured today, its `fix` exits 0 and emits no manifest at all on this path.
    let comp = crate::completeness::report_completeness(prefix);
    comp.warn_unreadable("fix");
    let (so_what, tail) = (
        "any remedy below is computed over a universe candor cannot fully see",
        "A callee in one of those contributes no effect here, and a caller in one is invisible to the \
         hoist. `gate --report` exits 2 over these bytes. Re-scan for a complete answer.",
    );
    if want_json {
        comp.eprint_note(so_what, tail);
    } else {
        comp.print_note(so_what, tail);
    }

    // Resolve `target` among the best-tier name matches, PREFERRING one that actually performs the effect —
    // so a bare leaf (`save`) resolves to the violating `Repo.save`, not a same-named pure `Cache.save` that
    // happens to sort first. (Must match candor-ts/candor-java/candor-swift exactly — a divergence here flips
    // a real remedy into a false "nothing to hoist" on some engines. /code-review.)
    let tier = best_tier(entries.iter().map(|e| e.func.as_str()), target);
    let matches: Vec<&ReportEntry> = entries.iter().filter(|e| tier > 0 && q_match(&e.func, target, tier)).collect();
    let Some(start) = matches
        .iter()
        .copied()
        .find(|e| e.inferred.iter().any(|x| x == effect))
        .or_else(|| matches.first().copied())
    else {
        eprintln!("candor fix: no function matching `{target}`.");
        return 2;
    };

    // ⟨0.28⟩ SPEC §2's zero-rule caveat. The clause names `whatif`/`fix-gate`/`unverified` — the verbs
    // that share the loader — and this verb shares it too (`load_rules` above): its every answer below
    // ("does not perform", "not forbidden", a hoist plan) is an answer RELATIVE TO A POLICY, and
    // relative to no rules there is no boundary to have crossed. Answering `crossing: false` here would
    // be a confident no-op verdict from a gate that asked nothing — the advisory verb more confident
    // than the gate, which refuses these bytes outright. The named-verb list is illustrative, not
    // exhaustive (the ⟨0.24⟩ sibling-scoping lesson, recorded in SPEC §3.2). Exit 0, unchanged: both
    // no-op arms below already answered 0, and the missing-function usage error above kept its 2.
    if crate::policy::policy_asked_nothing(&parsed) {
        crate::policy::emit_zero_rule_caveat("fix", &pp_path, want_json, &comp);
        return 0;
    }

    // ⟨0.28⟩ SPEC §3.1 pins `crossing`: a boolean, PRESENT EXACTLY WHEN THE VERB ANSWERED, absent when
    // it refused, `reason` on the `false` arm. This engine emitted no such key — it answered the
    // determined-negative arms as PROSE ON STDOUT UNDER `--json` (measured: `fix <fn> <Eff>` printed
    // "…the boundary isn't crossed, nothing to fix." as stdout's only content), which §3.3.1
    // independently forbids ("stdout MUST then be pure JSON"). The two arms below are ANSWERS, so under
    // `--json` each is now a document: `{fn, effect, crossing: false, reason}` with the completeness
    // fields riding it, `reason` the ts/swift token pair ("does-not-perform" / "not-forbidden"). The
    // refused arm above keeps NO `crossing` key (the MCP contract's check-`refused`-first ordering),
    // and the plan arm below gains `crossing: true`. Exit 0 on both `false` arms, unchanged.
    if !start.inferred.iter().any(|e| e == effect) {
        if want_json {
            let mut out = serde_json::json!({
                "fn": start.func,
                "effect": effect,
                "crossing": false,
                "reason": "does-not-perform",
            });
            comp.write_json(&mut out);
            println!("{}", serde_json::to_string_pretty(&out).unwrap());
            return 0;
        }
        println!("candor fix: `{}` does not perform {effect} — nothing to hoist.", start.func);
        return 0;
    }
    // ⟨0.24⟩ **THE GATE IS ASKED FIRST — SPEC §3.2 (`4fd140c`).** MEASURED here on the R11 report: over
    // `deny Net[unknown-host] app` with `app.noClass` carrying `hosts` and no `netClass`,
    // `gate --report` exits 2 refusing to judge it, and this verb printed a complete hoist plan —
    // `deniedSpan`, `site`, `policyAlternative`, exit 0 — because [`denied_layer`] asks whether the rule
    // NAMES the effect and cannot see the narrowing filter at all. That is the ruling's exact harm, one
    // verb over from where it was measured: a confident refactoring instruction resting on a guess.
    //
    // The pairs are the gate's own ([`crate::gate::unanswerable_pairs`]), matched on THIS function, so
    // the two cannot drift about which boundary is off limits.
    let sig = crate::gate::report_signature(&entries);
    let refused: Vec<_> =
        crate::gate::unanswerable_pairs(&parsed, &sig).into_iter().filter(|u| u.func == start.func).collect();
    if !refused.is_empty() {
        for u in &refused {
            eprintln!("candor fix: `{}` — {}", u.rule, u.why);
        }
        if want_json {
            // A machine consumer piping this to `jq` must get a DOCUMENT, not empty stdout — an empty
            // stdout is read as "no remedy needed", which is the confident answer this branch exists to
            // withhold. No plan keys and no `ok`: the verb computed nothing, and it says which rule
            // stopped it in the gate's own `unevaluated` shape.
            let mut out = serde_json::json!({
                "fn": start.func,
                "effect": effect,
                "unevaluated": unevaluated_json(&refused),
            });
            comp.write_json(&mut out);
            println!("{}", serde_json::to_string_pretty(&out).unwrap());
            return 0;
        }
        println!(
            "candor fix: `{}` performs {effect}, but `candor-query gate --report` CANNOT JUDGE it over \
             this report ({} rule(s) above went unevaluated) — so there is no remedy to compute. Hoisting \
             across a boundary nothing adjudicated would be a confident instruction resting on a guess: \
             gate at scan time, or use the unnarrowed rule.",
            start.func,
            refused.len()
        );
        return 0;
    }
    let Some(layer) = denied_layer(&start.func, effect, rules) else {
        // The other `crossing: false` arm — see the note above. "not-forbidden" is a claim the rule
        // fired and missed, which the answered-refused split above already guards: an unanswerable
        // rule on this function took the `unevaluated` arm before reaching here.
        if want_json {
            let mut out = serde_json::json!({
                "fn": start.func,
                "effect": effect,
                "crossing": false,
                "reason": "not-forbidden",
            });
            comp.write_json(&mut out);
            println!("{}", serde_json::to_string_pretty(&out).unwrap());
            return 0;
        }
        println!(
            "candor fix: `{}` performs {effect}, but no policy forbids it there — the boundary isn't crossed, nothing to fix.",
            start.func
        );
        return 0;
    };

    let rev = reverse_graph(&entries);
    let plan = compute_remedy(&by_name, &rev, rules, start, effect, layer);

    if want_json {
        let mut out = plan.to_json();
        // ⟨0.28⟩ `crossing: true` beside the plan — here in `fix` ONLY, never on `fix-gate`'s
        // `remedies` entries, whose shape §3.1 pins separately without it.
        out["crossing"] = serde_json::json!(true);
        comp.write_json(&mut out);
        println!("{}", serde_json::to_string_pretty(&out).unwrap());
    } else {
        let mut s = String::new();
        plan.render_text(&mut s);
        print!("{s}");
        println!("\n  (Advisory: candor names the shape, you write the code; the gate re-scan verifies the fix.)");
    }
    0
}

/// `fix-gate <prefix> [policy] [--json|0|1]` — a remedy for EVERY deny/`pure` boundary crossing in the
/// report. This is the loop's payoff: the edit-time gate blocks, and this hands the agent the fix for each
/// crossing. Scope is AS-EFF-006 (effect-in-forbidden-layer) only — the one refactor candor can compute;
/// allowlist/layering findings are a different shape and are left to the gate's own message. Advisory: the
/// gate re-scan stays the ground truth.
pub(crate) fn cmd_fix_gate(args: &[String]) -> i32 {
    let g = parse(args, Shape { verb_args: 0, sentinel: true, has_policy: true });
    let Some(prefix) = report_or_discover(&g) else {
        eprintln!("candor: no report found (no --report and no .candor/ discovered) — scan the crate first.");
        return 2;
    };
    let prefix = &prefix;
    let want_json = g.want_json;
    let policy_path = g.policy.clone();
    let (pp_path, parsed) = match load_rules(policy_path) {
        Ok(p) => p,
        Err(c) => return c,
    };
    let rules = &parsed.rules;

    // ⟨0.28⟩ THROUGH THE LOUD LOADER, NOT A BARE EMPTINESS CHECK — same repair as `cmd_unverified`,
    // where the full note lives. A judged-nothing report (`functions: []`, `analyzed.count: 0`) is a
    // DISCLOSURE, not an exit code (SPEC §2 ⟨0.24⟩): it answers below at exit 0 with `incomplete: true`
    // + `judgedNothing`, while a missing or net-corrupt report stays a loud exit 2 in the loader.
    let entries = match crate::load::load_entries_loud(prefix) {
        Ok(e) => e,
        Err(code) => return code,
    };
    let by_name: HashMap<&str, &ReportEntry> = entries.iter().map(|e| (e.func.as_str(), e)).collect();
    let rev = reverse_graph(&entries);

    // Every (function, effect) that trips a deny/pure rule → its remedy, collapsed to one plan per root
    // cause (dedup_key folds the inheritors of a single crossing together).
    // Iterate functions (and effects) in sorted order so the first-writer-wins `fn` representative of a
    // collapsed remedy is deterministic across engines (load_entries doesn't sort; java/swift/ts all iterate
    // a sorted key set). The BTreeMap already emits remedies in dedup-key order. (/code-review.)
    let mut sorted: Vec<&ReportEntry> = entries.iter().collect();
    sorted.sort_by(|a, b| a.func.cmp(&b.func));
    // ⟨0.24⟩ The transitive reason classes, over the report's own edges — the same fixpoint `gate
    // --report` and `unverified` run, because the ENUMERATION below must select the gate's violation set
    // and a narrowing `Unknown[…]` filter quantifies over exactly this. It is now literally the gate's
    // OWN accumulator (SPEC §3.2, `4fd140c`): this verb reads `gate --report`'s signature rather than
    // rebuilding one, and takes the ANSWERABILITY set off the same object.
    let sig = crate::gate::report_signature(&entries);
    let reason_acc = &sig.reason_classes;
    let mut unanswered = crate::gate::unanswerable_pairs(&parsed, &sig);
    // ⟨0.29⟩ …AND THE TWO WHOLE-POLICY KINDS. `unanswerable_pairs` walks `p.rules`, i.e. `deny` only, so a
    // policy whose rules were ALL `forbid` produced an EMPTY refusal set and this verb printed
    // *"no deny/pure boundary crossings in this report ✓"* at exit 0 — measured — over a policy nothing
    // had evaluated. `gate --report` refused the same policy correctly; the rule lived inline in the gate
    // and the advisory siblings reading the same report never saw it. `whole_policy_refusals` is now
    // shared, and these entries carry no `func` because the kind is unanswerable for the whole report,
    // not at one function.
    unanswered.extend(crate::gate::whole_policy_refusals(&parsed, &pp_path).into_iter().map(|u| {
        crate::gate::Unanswerable { rule: u.rule, func: String::new(), why: u.why }
    }));
    // ⟨0.24⟩ …and what the producing scan could not SEE AT ALL (SPEC §3.2 `ec1a441`, and
    // [`crate::completeness`] for the reasoning). Independent of `unanswered`: that is a function candor
    // analyzed and the gate could not JUDGE, this is source the scan never read, so there is no function
    // to name. It bites this verb twice — the crossing itself may be in an unread file, and a remedy is
    // a hoist to the nearest allowed-layer CALLER, which is missing from the plan exactly as a caller in
    // an unparsed file is missing from `whatif`'s blast radius. MEASURED on the release build over a
    // report declaring one `unanalyzed` unit and a `deny Net app` nothing violates:
    // `{"ok": true, "remedies": []}`, exit 0 under `--strict`, and the stdout line
    // *"no deny/pure boundary crossings in this report ✓"*.
    let comp = crate::completeness::report_completeness(prefix);
    comp.warn_unreadable("fix-gate");

    // ⟨0.28⟩ SPEC §2: a CONFIGURED policy that parsed to zero rules asked nothing — an empty `remedies`
    // beside `ok: true` here is a claim relative to a gate that never asked a question. The caveat
    // document replaces the result; the EXIT is unchanged (the same expression the result path
    // computes, over empty finding sets — zero rules can produce no plan and no unanswerable pair).
    if crate::policy::policy_asked_nothing(&parsed) {
        crate::policy::emit_zero_rule_caveat("fix-gate", &pp_path, want_json, &comp);
        return fix_gate_exit(g.strict, false, false, comp.incomplete());
    }

    let mut plans: BTreeMap<String, RemedyPlan> = BTreeMap::new();
    for e in sorted {
        let mut effs: Vec<&String> = e.inferred.iter().collect();
        effs.sort();
        for effect in effs {
            if let Some(layer) = denied_layer_evidenced(e, effect, rules, reason_acc.get(&crate::gate::entry_key(e))) {
                let plan = compute_remedy(&by_name, &rev, rules, e, effect, layer);
                plans.entry(plan.dedup_key()).or_insert(plan);
            }
        }
    }

    if want_json {
        let remedies: Vec<_> = plans.values().map(|p| p.to_json()).collect();
        let mut out = serde_json::json!({ "remedies": remedies });
        // ⟨0.24⟩ See [`unanswerable_for`]: `ok` is written ONLY where every rule was answerable — every
        // ordinary document, which therefore stays byte-identical to a pre-ruling one. Over a boundary
        // the gate refused, neither boolean is a statement, so the key is ABSENT and `unevaluated` says
        // which rule went unanswered and why.
        // ⟨0.24⟩ …and an INCOMPLETE report suppresses `ok` for the same reason (`ec1a441`), on the same
        // OMIT-don't-falsify rule: `ok: false` would assert a boundary crossing beside an empty
        // `remedies`. `unevaluated` is not exclusive with it — a report can be both refused-on and
        // incomplete, and each says something the other does not.
        // ⟨0.28⟩ `must_hedge`: count-0 withdraws `ok` too (SPEC §2), on the same OMIT-don't-falsify
        // rule, and in step with the prose branch below. The exit argument stays `incomplete()`.
        if unanswered.is_empty() && !comp.must_hedge() {
            out["ok"] = serde_json::json!(plans.is_empty());
        }
        if !unanswered.is_empty() {
            out["unevaluated"] = serde_json::json!(unevaluated_json(&unanswered));
        }
        comp.write_json(&mut out);
        println!("{}", serde_json::to_string_pretty(&out).unwrap());
        // Advisory by default (exit 0 — the agent fix-loop reads the remedy and edits); `--strict` makes
        // the exit code follow `ok`, so a CI job can REQUIRE zero outstanding crossings (mirrors
        // `unverified --strict`). exit 2 (no report / unreadable policy) already returned above.
        return fix_gate_exit(g.strict, !plans.is_empty(), !unanswered.is_empty(), comp.incomplete());
    }

    // ONE LINE PER RULE, matching the gate's own channel: the first function that defeats a rule is the
    // example, and 194 lines (measured on a stripped ebman report) would bury it. The per-FUNCTION list
    // is `unverified`'s job, where the ruling puts it.
    {
        let mut seen: BTreeSet<&str> = BTreeSet::new();
        for u in unanswered.iter().filter(|u| seen.insert(u.rule.as_str())) {
            let n = unanswered.iter().filter(|o| o.rule == u.rule).count();
            // ⟨0.29⟩ A WHOLE-POLICY REFUSAL HAS NO FUNCTION, and rendering its empty `func` as `` ` ` ``
            // printed "No remedy is computed for ``" — a bare empty identifier that reads as a bug in the
            // tool. That is the SAME rendering defect this rung already fixed in `unverified`, on the
            // sibling verb, and it sits on the first stop of the path every newly-failing AS-EFF-008 user
            // is pointed at ("→ candor-query fix-gate names the remedy for each"). Found by a
            // release-panel consumer-impact review, which is where a user-facing string of this kind is
            // supposed to be caught. When the refusal is whole-policy, name the RULE instead.
            let subject = if u.func.is_empty() {
                "this rule on any function".to_string()
            } else {
                format!("`{}`", u.func)
            };
            eprintln!(
                "candor fix-gate: `{}` — {} No remedy is computed for {}{}: a hoist plan for a boundary \
                 the gate could not adjudicate is a confident instruction resting on a guess. \
                 `candor unverified` names them all.",
                u.rule,
                u.why,
                subject,
                if n > 1 { format!(" or the {} other function(s) this rule cannot be evaluated on", n - 1) } else { String::new() }
            );
        }
    }
    // ⟨0.24⟩ THE HUMAN CHANNEL (SPEC §3.2 `ec1a441`) — printed BEFORE the verdict, because it qualifies
    // the remedies below as much as the all-clear. A mutant that kept the JSON fix and deleted only this
    // call survived the entire suite: the `✓` below IS the prose `ok: true`.
    comp.print_note(
        "the remedies below are computed over a universe candor cannot fully see",
        // ⟨0.28⟩ `gate_line()`, for the reason its doc gives: "exits 2" is FALSE of a judged-nothing-
        // only report, and a note that mis-states the gate discredits itself. Byte-identical on the
        // `unanalyzed` arm.
        &format!(
            "A crossing in one of those is INVISIBLE here, and so is a caller a hoist would target. \
             {} Re-scan for a complete answer.",
            comp.gate_line()
        ),
    );
    if plans.is_empty() {
        if unanswered.is_empty() && !comp.must_hedge() {
            println!("candor fix-gate: no deny/pure boundary crossings in this report ✓");
        } else {
            // NO `✓`. The tick is the same claim in prose, over a report the gate refused to judge or
            // was never shown all of. Both causes are named, because they are different repairs: one
            // wants a policy the gate can evaluate, the other wants a scan that reads every file.
            let mut why: Vec<String> = Vec::new();
            if !unanswered.is_empty() {
                why.push(format!("{} rule/function pair(s) went unevaluated (above)", unanswered.len()));
            }
            if comp.incomplete() {
                why.push(format!("{} unit(s) were never analyzed (above)", comp.units()));
            }
            // ⟨0.28⟩ THE THIRD CAUSE NEEDS ITS OWN CLAUSE OR THE SENTENCE READS `— , and …`. A count-0
            // report contributes no unanalyzed UNITS (`comp.units()` is 0), so widening the branch above
            // without widening its explanation would have produced an empty reason list under a
            // withheld `✓` — a withdrawal that declines to say what it is withdrawing for.
            if !comp.judged_nothing.is_empty() {
                why.push(format!(
                    "{} report(s) judged nothing at all (above)",
                    comp.judged_nothing.len()
                ));
            }
            // ⟨0.28⟩ …AND THE FOURTH, for the same reason the third needed one: SPEC §2's row 3 raises
            // `must_hedge` and contributes to NEITHER count above, so a row-3-only report would have
            // withheld the `✓` and then listed no reason for it. Its own clause, not a fourth spelling
            // of "judged nothing" — the report declares nothing at all.
            if !comp.no_manifest.is_empty() {
                why.push(format!(
                    "{} report(s) carry NO `analyzed` manifest at all (above)",
                    comp.no_manifest.len()
                ));
            }
            println!(
                "candor fix-gate: no deny/pure boundary crossings CAN BE COMPUTED from this report — \
                 {}. {}",
                why.join(", "),
                if unanswered.is_empty() {
                    comp.gate_line()
                } else {
                    "`candor-query gate --report` refuses over these bytes."
                }
            );
        }
        return fix_gate_exit(g.strict, false, !unanswered.is_empty(), comp.incomplete());
    }
    let n = plans.len();
    println!(
        "candor fix — {n} boundary {} for this change:\n",
        if n == 1 { "remedy" } else { "remedies" }
    );
    for (i, p) in plans.values().enumerate() {
        if i > 0 {
            println!("  ────────────────────────────────────────");
        }
        let mut s = String::new();
        p.render_text(&mut s);
        print!("{s}");
    }
    println!("\n  (Advisory: candor names the shape, you write the code; the gate re-scan verifies each fix.)");
    let rc = fix_gate_exit(g.strict, true, !unanswered.is_empty(), comp.incomplete());
    if g.strict {
        // `--strict` turns the advisory into a CI gate: a non-empty remedy set is a failure (exit 1), so a
        // job can REQUIRE the boundary be clean before merge (mirrors `unverified --strict`). Without it the
        // remedy prints and the run stays green — the agent-loop default.
        if rc == 2 && !unanswered.is_empty() {
            println!(
                "  (--strict: {n} outstanding boundary crossing(s), AND {} rule/function pair(s) the gate \
                 could not evaluate → exit 2, matching `gate --report`)",
                unanswered.len()
            );
        } else if rc == 2 {
            println!(
                "  (--strict: {n} outstanding boundary crossing(s), AND {} unit(s) the scan never \
                 analyzed → exit 2, matching `gate --report`)",
                comp.units()
            );
        } else {
            println!("  (--strict: {n} outstanding boundary crossing(s) → exit 1)");
        }
    }
    rc
}

/// ⟨0.24⟩ `fix-gate --strict`'s exit code, with the REFUSAL DOMINATING — SPEC §3.2 (`4fd140c`):
/// *"`--strict` exits 2, matching the gate."* Same argument as `unverified`'s
/// [`crate::unverified::unverified_exit`]: neither outcome here is certain, so the exit answers *did this
/// verb evaluate the policy you gave it?*, and answering 1 where the gate answered 2 claims it got
/// further than the gate on identical bytes. Without `--strict` the verb stays advisory at exit 0.
///
/// ⟨0.24⟩ An INCOMPLETE report joins the 2 (`ec1a441`), for the identical reason: `gate --report` exits
/// 2 over those bytes, so any smaller code claims this verb saw more than the gate did.
fn fix_gate_exit(strict: bool, any_plans: bool, any_unanswered: bool, incomplete: bool) -> i32 {
    match (strict, any_unanswered || incomplete, any_plans) {
        (true, true, _) => 2,
        (true, false, true) => 1,
        _ => 0,
    }
}
