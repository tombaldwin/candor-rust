//! Policy-facing commands: `parsepolicy`, `whatif`, `rewire`, `gate-verdict`.

use crate::*;

/// ⟨0.24⟩ **READ A POLICY THE WAY THE GATE READS IT.** The one loader for every verb that reasons about
/// a policy, so no verb can mean something different by the same file.
///
/// MEASURED 2026-07-28. `whatif`, `fix`/`fix-gate` and `unverified` all called bare `parse_policy`,
/// which loads NO `unknown-alias` vocabulary and reports NO policy errors. With
/// `unknown-alias corp = reflect` in the config beside the policy and `deny Unknown[corp] app.nat` over
/// a NATIVE-caused hole:
///
///   - the GATE resolves `corp` → `{reflect}`, the class does not match, exit **0**;
///   - `whatif app.nat Unknown` answered **"⚠ WOULD VIOLATE policy"** and printed the rule back as
///     `deny Unknown app.nat` — it had silently REWRITTEN the operator's rule to the widest form and was
///     showing the rewrite as if it were the rule;
///   - `fix-gate` named a hoist remedy for a violation the gate does not report;
///   - and `unverified` answered **"every function in a pure/deny layer is PROVABLY clean ✓"**, which is
///     the direction that matters: a hole is a function that PASSES its rule while being `Unknown`, so
///     widening the rule turned a real hole into a violation-that-isn't and **deleted it from the
///     disclosure.** An over-report in three verbs and a lost disclosure in the fourth, from one line.
///
/// §6.2 is explicit that the gate and the disclosure MUST apply the same rule, and these are the verbs an
/// agent consults BEFORE editing. Since ⟨0.24⟩ made an unrecognised token a POLICY ERROR there is a
/// second half: a verb that ignores `errors` answers from a rule the operator did not write, so the
/// refusal travels too.
///
/// The config is anchored to the POLICY file, never the CWD — SPEC §3.1's rule that vocabulary travels
/// with the policy that uses it, and the same anchor `cmd_gate` and `parsepolicy` already use.
pub(crate) fn load_policy_as_the_gate_does(
    verb: &str,
    policy_path: &str,
) -> Result<candor_classify::policy::ParsedPolicy, i32> {
    let Ok(text) = std::fs::read_to_string(policy_path) else {
        eprintln!("candor {verb}: policy `{policy_path}` could not be read — nothing computed (exit 2).");
        return Err(2);
    };
    let aliases = candor_classify::policy::discover_config_text(std::path::Path::new(policy_path))
        .map(|t| candor_classify::policy::parse_unknown_aliases(&t))
        .unwrap_or_default();
    let p = candor_classify::policy::parse_policy_with_aliases(&text, &aliases);
    // ⟨0.24⟩ FATAL errors only — `errors` now also carries the DROPPED-but-survivable lines
    // (`nonsense line`, a malformed `forbid`), which `parsepolicy` reports and no gate route refuses on.
    let fatal = p.fatal_messages();
    if !fatal.is_empty() {
        for e in &fatal {
            eprintln!("candor {verb}: policy error — {e}");
        }
        eprintln!(
            "candor {verb}: refusing to reason about a policy that cannot be honoured AS WRITTEN (exit \
             2). The gate refuses it too, and answering here from a rule the gate will not apply is the \
             worse failure: this is the verb consulted BEFORE the edit."
        );
        return Err(2);
    }
    Ok(p)
}

/// ⟨0.28⟩ SPEC §2: **AN ADVISORY VERB OVER A ZERO-RULE POLICY ANSWERS WITH THE CAVEAT DOCUMENT** —
/// result keys withheld, exit UNCHANGED. Did this configured policy parse to no rules at all?
///
/// §6.2 makes the same condition an exit-2 REFUSAL for the GATE (`ok: true` is a claim about the code
/// no such run is entitled to make); the advisory verbs share the loader and are ruled differently:
/// they set no verdict, so the refusal posture is the wrong import — what they produce is an answer
/// *relative to a policy*, and relative to no rules that answer is not a finding, it is an absence of
/// questions. So `unverified` does not emit an empty `unverified` list over a policy that asked
/// nothing, for the same reason ⟨0.27⟩'s refusal document must not carry `violations`. All three rule
/// vectors, for the reason the gate's own check gives: keying on `rules` alone would treat an
/// allow-only policy as empty.
pub(crate) fn policy_asked_nothing(p: &candor_classify::policy::ParsedPolicy) -> bool {
    p.rules.is_empty() && p.allow_rules.is_empty() && p.layer_rules.is_empty() && p.only_rules.is_empty()
}

/// The zero-rule caveat, on BOTH channels — SPEC §2 ⟨0.28⟩.
///
/// The machine document is the §3.1 `unevaluated` shape with the whole-policy entry — the SAME entry,
/// character for character, that this engine's two gate routes put on their zero-rule refusal
/// document, so the advisory caveat and the gate's refusal cannot drift about what went unevaluated
/// (inventing a second spelling is the mistake SPEC records making four times). No `ok`, no result
/// keys: a consumer branching on `r.ok` gets falsy and fails safe; one that looks further learns the
/// policy asked nothing. The report-completeness caveat rides the same document when it applies —
/// the two disclosures are independent and each says something the other does not.
///
/// The EXIT is the caller's and is UNCHANGED (⟨0.24⟩: count-0 reaches both disclosure channels and
/// stops at the exit code — the same standing ruling, one condition over).
pub(crate) fn emit_zero_rule_caveat(
    verb: &str,
    policy_path: &str,
    want_json: bool,
    comp: &crate::completeness::ReportCompleteness,
) {
    let sentence = format!(
        "candor {verb}: the policy at {policy_path} yielded NO RULES — every line was ignored, the \
         file is empty, or it holds only comments. A policy with no rules ASKS NOTHING, so this verb \
         has no answer to give relative to it: the result keys are withheld and this caveat stands in \
         their place (SPEC §2 ⟨0.28⟩). `gate` refuses outright over this policy (exit 2). If you did \
         not mean to gate, remove the policy configuration rather than pointing it at a file with no \
         rules in it."
    );
    if want_json {
        eprintln!("{sentence}");
        let mut out = serde_json::json!({
            "unevaluated": [ {
                "rule": format!("(entire policy {policy_path} — no rules parsed)"),
                "why": "the configured policy yielded zero rules, so nothing was evaluated and no \
                        rule can have passed",
            } ],
        });
        comp.write_json(&mut out);
        println!("{}", serde_json::to_string_pretty(&out).unwrap());
    } else {
        println!("{sentence}");
        comp.print_note(
            "this run's report is ALSO incomplete — the caveat above is not the only one",
            &format!("{} Re-scan for a complete answer.", comp.gate_line()),
        );
    }
}

// ── whatif ──────────────────────────────────────────────────────────────────────────────────────

// ⟨0.24⟩ `whatif`'s completeness read now lives in [`crate::completeness`], with the ruling's whole
// reasoning, because candor-spec `ec1a441` widened the clause `0075987` had scoped to THIS verb to
// every advisory verb — and the reason it had to be widened is that this engine implemented it here, in
// `whatif`'s own file, while `unverified.rs` and `fix.rs` contained not one occurrence of `incomplete`.
// `whatif`'s own stake is unchanged, and is why it was found here first: the `affected` set is a
// reverse-reachability closure over the callgraph, so a caller living in a file the scan could not parse
// is invisible to it — the blast radius is computed over a universe the analysis cannot fully see.

/// Segment-aware scope match, IDENTICAL to the lint's `scope_matches` (src/lib.rs) so a `whatif` verdict
/// matches what the policy gate would actually do — `domain` matches `app::domain::f` and `domain_logic`,
/// but NOT `subdomain`. Keep in lockstep with the lint.
/// `parsepolicy <file>` — dump the parsed CANDOR_POLICY as canonical JSON (deny/allow/forbid), using
/// the SHARED parser (`candor_classify::policy`, SPEC §6.2). Not a user workflow: it exists so the
/// cross-impl conformance suite can diff this engine's policy parse against the JVM engine and prove the
/// grammar means the same thing in both. A `pure` rule appears as a deny with empty `effects`; whole-unit
/// scope is the empty string (matching the JVM dump). Rules are sorted so the comparison is order-free.
pub(crate) fn cmd_parsepolicy(args: &[String]) -> i32 {
    let Some(path) = args.first() else {
        eprintln!("usage: candor-query parsepolicy <policy-file>");
        return 2;
    };
    let Ok(text) = std::fs::read_to_string(path) else {
        eprintln!("candor: cannot read policy {path}");
        return 2;
    };
    // ⟨0.19⟩ config-aware: discover `.candor/config` (or CANDOR_CONFIG) anchored to the policy file so an
    // `Unknown[<alias>]` resolves via a checked-in `unknown-alias` — the dump reflects real gate resolution,
    // and the four-way parsepolicy differential pins the expansion.
    let aliases = candor_classify::policy::discover_config_text(std::path::Path::new(path))
        .map(|t| candor_classify::policy::parse_unknown_aliases(&t))
        .unwrap_or_default();
    let p = candor_classify::policy::parse_policy_with_aliases(&text, &aliases);
    let mut deny: Vec<serde_json::Value> = p
        .rules
        .iter()
        .map(|r| {
            let mut m = serde_json::json!({
                "effects": r.effects.iter().copied().collect::<Vec<&str>>(),
                "scope": r.scope.as_deref().unwrap_or(""),
            });
            // Reason-scoped `Unknown[class…]`: emit sorted class tokens ONLY when the rule narrows Unknown,
            // so a bare `deny E`/`deny E Unknown` dump is byte-identical to pre-feature and the four-way
            // parsepolicy differential pins reason-class parsing across engines (matches candor-java).
            if !r.unknown_classes.is_empty() {
                let mut toks: Vec<&str> = r.unknown_classes.iter().map(|c| c.token()).collect();
                toks.sort_unstable(); // sort by TOKEN string (matches java's `.sorted()` on tokens)
                m["unknownClasses"] = serde_json::json!(toks);
            }
            // Net destination-class `Net[dest…]`: emit sorted dest tokens ONLY when the rule narrows Net, so a
            // bare `deny Net` dump is byte-identical and the four-way parsepolicy differential pins the
            // destination-class parsing across engines (matches candor-java).
            if !r.net_classes.is_empty() {
                let toks: Vec<&str> = r.net_classes.iter().map(String::as_str).collect(); // BTreeSet ⇒ sorted
                m["netClasses"] = serde_json::json!(toks);
            }
            m
        })
        .collect();
    let mut allow: Vec<serde_json::Value> = p
        .allow_rules
        .iter()
        .map(|r| {
            serde_json::json!({
                "effect": r.effect,
                "scope": r.scope.as_deref().unwrap_or(""),
                "values": r.literals.iter().map(String::as_str).collect::<Vec<&str>>(),
            })
        })
        .collect();
    let mut forbid: Vec<serde_json::Value> =
        p.layer_rules.iter().map(|r| serde_json::json!({ "from": r.from, "to": r.to })).collect();
    deny.sort_by_key(|v| v.to_string());
    allow.sort_by_key(|v| v.to_string());
    forbid.sort_by_key(|v| v.to_string());
    // ⟨0.29⟩ THE PERMISSION FORM RIDES THE WITNESS TOO. `parsepolicy` is the §6.2 GRAMMAR WITNESS — the
    // verb that exists so a consumer can diff what an engine made of a policy — so a rule kind missing
    // from it is a kind the diff cannot see. Omitting `only` here while candor-java emitted it made the
    // two disagree about what the same file MEANS, which is exactly what this verb is for catching.
    let mut only: Vec<serde_json::Value> = p
        .only_rules
        .iter()
        .map(|r| serde_json::json!({ "from": r.from, "to": r.to }))
        .collect();
    only.sort_by_key(|v| v.to_string());
    // ⟨0.24⟩ EVERY LINE THE PARSE DID NOT HONOUR (SPEC §3.1 `195d45a`/`901f14d`).
    //
    // MEASURED 2026-07-28 on the conformance battery: java 10, ts 4, **rust 0** — this verb emitted no
    // `errors` key at all. The facts existed and went to stderr as "ignoring policy rule …", so the one
    // verb that exists to let a consumer diff what an engine made of a policy answered with the
    // not-honoured half deleted. And it contradicted this engine's own gate, which REFUSES an
    // unrecognised class token while the parse narrowed it in silence — two answers to one question.
    //
    // NOT A REFUSAL. §3.1: *"`parsepolicy` MUST NOT REFUSE a policy it can read and cannot honour. It
    // REPORTS that parse, including what it could not honour."* So the exit code stays 0 and the errors
    // ride the document beside the rules that DID parse — including the fatal ones, which every gate
    // route refuses on and this verb is precisely the tool for diagnosing.
    //
    // OMITTED WHEN EMPTY, so a clean parse is byte-identical to the pre-rung dump and the four-way
    // parsepolicy differential does not move for any policy without an error.
    //
    // ORDER IS SOURCE ORDER — the rules above are sorted so the comparison is order-free, but an error
    // list is a reading of the FILE and the operator's next action is to go to those lines in order.
    let errors: Vec<serde_json::Value> = p
        .errors
        .iter()
        .map(|e| {
            serde_json::json!({
                "kind": e.kind,
                "token": e.token,
                "accepted": e.accepted,
                "rule": e.rule,
                "message": e.message,
            })
        })
        .collect();
    let mut doc = serde_json::json!({ "deny": deny, "allow": allow, "forbid": forbid, "only": only });
    if !errors.is_empty() {
        doc["errors"] = serde_json::Value::Array(errors);
    }
    println!("{doc}");
    0
}

/// `whatif <prefix> <fn> <Effect> [policy] [0|1]` — the PRE-EDIT verdict. Computes the blast radius of
/// introducing `Effect` into `fn` (the fn + every transitive caller, all of which would gain it), then —
/// given a policy — reports which of them would VIOLATE a `deny <Effect>` / `pure` boundary. Answers
/// "if I add a network call here, what happens and is it allowed?" BEFORE the edit, instead of edit →
/// run the gate → revert. Read-only over the call-graph sidecar + the policy file.
pub(crate) fn cmd_whatif(args: &[String]) -> i32 {
    let g = parse(args, Shape { verb_args: 2, sentinel: true, has_policy: true, verb: "whatif" });
    let (Some(target), Some(effect)) = (g.positional.first().cloned(), g.positional.get(1).cloned()) else {
        eprintln!("usage: candor-query whatif <fn> <Effect> [--report <locator>] [--policy <file>] [--json]");
        return 2;
    };
    let (target, effect) = (&target, &effect);
    // Validate the effect against the vocabulary: a typo'd/lowercase effect (`net`) matches no deny
    // rule and would print an authoritative-looking clean verdict — a false green light for the very
    // edit the policy forbids (/code-review). Reject it as a usage error, not a pass.
    if candor_classify::cap_from_name(effect).is_none() && effect.as_str() != "Unknown" {
        eprintln!("candor: unknown effect `{effect}` (expected a candor effect name, e.g. Net/Fs/Db/Exec, or Unknown)");
        return 2;
    }
    let Some(prefix) = report_or_discover(&g) else {
        eprintln!("candor: no report found (no --report and no .candor/ discovered) — scan the crate first.");
        return 2;
    };
    let prefix = &prefix;
    let want_json = g.want_json;
    // Policy: --policy / deprecated positional (both via `g`), else CANDOR_POLICY.
    let policy_path: Option<String> = g.policy.clone().or_else(|| std::env::var("CANDOR_POLICY").ok());

    let cg = load_callgraph(prefix);
    if cg.is_empty() {
        eprintln!("candor: no call-graph sidecar for `{prefix}` — scan the crate first.");
        return 2;
    }
    let mut rev: BTreeMap<&str, Vec<&str>> = BTreeMap::new();
    for (caller, callees) in &cg {
        for c in callees {
            rev.entry(c.as_str()).or_default().push(caller.as_str());
        }
    }
    let names: BTreeSet<&str> =
        cg.keys().map(|s| s.as_str()).chain(cg.values().flatten().map(|s| s.as_str())).collect();
    let tier = best_tier(names.iter().copied(), target);
    let targets: Vec<&str> = names.iter().copied().filter(|n| q_match(n, target, tier)).collect();
    if targets.is_empty() {
        eprintln!("candor: no function matching `{target}` in the call graph.");
        return 2;
    }
    // The affected set: the target(s) + every transitive caller — all gain `effect` after the edit.
    let mut affected: BTreeSet<&str> = targets.iter().copied().collect();
    let mut stack: Vec<&str> = targets.clone();
    while let Some(n) = stack.pop() {
        if let Some(cs) = rev.get(n) {
            for &c in cs {
                if affected.insert(c) {
                    stack.push(c);
                }
            }
        }
    }

    // The verdict: affected functions sitting in a `deny <effect>` / `pure` scope would violate.
    // Parsed by the SHARED canonical DSL parser (candor_classify::policy, SPEC §6.2) — the SAME one the
    // nightly gate uses — so the pre-edit verdict can never diverge from the real gate's. (Only the
    // deny/pure rules are simulated here; allow/forbid are not pre-edit effect-introduction concerns.)
    // A SPECIFIED-but-unreadable policy must FAIL LOUD, not silently yield ok:true — a typo'd
    // CANDOR_POLICY path otherwise reads as "no violations" and an agent proceeds with a forbidden
    // edit believing the boundary was checked (/code-review; mirrors cmd_diff's loud no-files check).
    //
    // ⟨0.24⟩ Through the SHARED loader, so `whatif`'s pre-edit verdict is computed from the same rules
    // the gate will apply: `unknown-alias` vocabulary resolved, policy errors refused. It used to call
    // bare `parse_policy`, which silently widened `deny Unknown[<alias>]` to a bare `deny Unknown` and
    // then printed the WIDENED rule back to the operator as the one that would be violated.
    let parsed = match policy_path.as_deref() {
        None => None,
        Some(p) => match load_policy_as_the_gate_does("whatif", p) {
            Ok(pp) => Some((p.to_string(), pp)),
            Err(code) => return code,
        },
    };
    let rules = parsed.as_ref().map(|(_, pp)| pp.rules.clone());
    // ⟨0.24⟩ `(fn, the rule's OWN source line, the condition that rule's verdict rests on)`.
    let mut violations: Vec<(&str, String, Option<String>)> = Vec::new();
    if let Some(rules) = &rules {
        for fname in &affected {
            for rule in rules {
                // Mirrors the gate's SEMANTICS §6 projection: `deny` fires only when the rule NAMES
                // the effect; `pure` forbids every real effect but not `Unknown` (the §4 visibility
                // marker — AS-EFF-003's concern; `deny Unknown` is the explicit strictness knob).
                // Kept in lockstep with the nightly gate (src/lib.rs) so the pre-edit verdict can
                // never diverge from the real gate's.
                let denies = if rule.effects.is_empty() {
                    effect != candor_classify::policy::UNKNOWN
                } else {
                    rule.effects.contains(effect.as_str())
                };
                let in_scope =
                    rule.scope.as_deref().is_none_or(|s| candor_classify::policy::scope_matches(fname, s));
                if denies && in_scope {
                    // ⟨0.24⟩ **THE RULE'S OWN LINE, VERBATIM.** This used to be REBUILT from `effects` +
                    // `scope` — and worse, from the effect being ASKED ABOUT rather than the rule's own
                    // set — so it showed the operator a rule they did not write. MEASURED 2026-07-28:
                    //
                    //   `deny Unknown[reflect] app.nat`  printed back as  `deny Unknown app.nat`
                    //   `deny Net[unknown-host] app`     printed back as  `deny Net app`
                    //   `deny Net Db  app`               printed back as  `deny Net app`
                    //
                    // The first two are the sharp ones: they show a NARROWED rule as the WIDE one, in the
                    // verb an agent reads before editing, so the operator's own scoping is invisible at
                    // exactly the moment they are deciding whether it protects them. `raw` is the line
                    // with its comment stripped and its ends trimmed — the policy as written.
                    violations.push((fname, rule.raw.clone(), narrowing_condition(rule, effect)));
                    break;
                }
            }
        }
    }

    // ⟨0.24⟩ Did the producing scan see all of the target's own source? SPEC §3.2 (`0075987`,
    // `ec1a441`) — see [`crate::completeness`] for why the answer is neither `ok: true` nor `ok: false`.
    //
    // ⟨0.33⟩ candor-spec PART 70: `whatif` read the bare, unarmed manifest here and never called
    // [`crate::completeness::arm_unread`]/[`crate::completeness::arm_unasked_rules`] — the SAME union
    // `unverified` and `fix`/`fix-gate` already apply to their own `parsed` policy (`unverified.rs`,
    // `fix.rs`). The ⟨0.32⟩ unread-class cause still reached `must_hedge()` unarmed (that predicate reads
    // `unread` directly — see its own doc), which is why PART 70 measured that cell OK without this call;
    // but ⟨0.33⟩'s cross-policy cause is populated ONLY by `arm_unasked_rules`, which nothing on this
    // route ever invoked, so a report `peeked: true` under a deny set narrower than THIS run's policy
    // could never raise it — `unasked_rules` stayed structurally empty forever and `ok` answered as if
    // the peek had been asked this policy's question. MEASURED: PART 70's cross-policy cell read
    // `ok: false` PRESENT where `incomplete: true` with `ok` OMITTED was required.
    //
    // Armed only when a policy was actually loaded, matching `diff`'s converse reasoning exactly: a
    // verb holding NO policy has no deny set for `arm_unasked_rules` to compare against (its `own` set
    // is structurally empty, same as `diff`'s), so skipping the call rather than calling it with an
    // empty policy changes nothing observable and avoids inventing a `ParsedPolicy` this route did not
    // parse.
    let comp = match parsed.as_ref() {
        Some((_, pp)) => crate::completeness::arm_unasked_rules(
            crate::completeness::arm_unread(crate::completeness::report_completeness(prefix), pp),
            pp,
        ),
        None => crate::completeness::report_completeness(prefix),
    };
    comp.warn_unreadable("whatif");

    // ⟨0.28⟩ SPEC §2: a CONFIGURED policy that parsed to zero rules asked nothing, so the pre-edit
    // verdict — and the blast radius it qualifies — is withheld in favour of the caveat document.
    // The exit is UNCHANGED (0: with no rules, no violation was ever recordable on this path).
    if let Some((pp_path, _)) = parsed.as_ref().filter(|(_, pp)| policy_asked_nothing(pp)) {
        emit_zero_rule_caveat("whatif", pp_path, want_json, &comp);
        return 0;
    }

    if want_json {
        let mut out = serde_json::json!({
            "of": targets,
            "effect": effect,
            "affected": affected.iter().collect::<Vec<_>>(),
            "violations": violations
                .iter()
                .map(|(f, r, cond)| {
                    let mut v = serde_json::json!({"fn": f, "rule": r});
                    // OMITTED unless the rule narrows the introduced effect, so every document from an
                    // unfiltered policy — which is nearly all of them — stays byte-identical.
                    if let Some(c) = cond {
                        v["conditional"] = serde_json::json!(c);
                    }
                    v
                })
                .collect::<Vec<_>>(),
        });
        // ⟨0.24⟩ SPEC §3.2 `0075987`. `ok` is written ONLY on a document whose `affected` set was
        // computed over a universe the analysis could see all of — every ordinary run, which therefore
        // stays byte-identical. Over an incomplete report the key is ABSENT (`if (r.ok)` is falsy and
        // fails safe) and `incomplete` + the manifest say what was unread instead.
        // ⟨0.28⟩ `must_hedge`, not `incomplete`: `analyzed.count: 0` withdraws `ok` on the same
        // terms (SPEC §2), and the two channels must move together — the prose `✓` below is keyed on
        // the same predicate, and a document saying `ok: true` under a note saying INCOMPLETE is the
        // split `ec1a441` ruled against. The EXIT CODE is untouched, here and there.
        if comp.must_hedge() {
            comp.write_json(&mut out);
        } else {
            out["ok"] = serde_json::json!(violations.is_empty());
        }
        println!("{}", serde_json::to_string_pretty(&out).unwrap());
        // THE EXIT CODE IS UNCHANGED, and deliberately: `0075987`'s remedy is the DOCUMENT's, and §3.3
        // enumerates the exit-2 causes for a GATE — this verb is not one, so minting a third here would
        // make an advisory pre-edit query fail a build. The shell reader keeps the violation count it
        // always had; the machine reader loses the `ok` it must not be given.
        return if violations.is_empty() { 0 } else { 1 };
    }

    println!("whatif: adding `{effect}` to `{}`", targets.join(", "));
    println!("  → propagates to {} function(s) (the blast radius):", affected.len());
    for f in &affected {
        println!("      {f}");
    }
    // ⟨0.24⟩ THE SAME HEDGE ON THE HUMAN CHANNEL (SPEC §3.2 `0075987`). The JSON reader loses `ok`; the
    // operator would otherwise read an unqualified blast radius and a `✓`, which is the same claim in
    // prose. Printed BEFORE the verdict, because it qualifies the `affected` list above as much as it
    // qualifies the verdict below — a caller in an unparsed file is missing from BOTH.
    comp.print_note(
        "the blast radius above is computed over a universe candor cannot fully see",
        "A caller living in one of those is INVISIBLE here. Re-scan for a complete answer.",
    );
    if rules.is_none() {
        println!("  (no policy given — pass a policy file or set CANDOR_POLICY for the gate verdict)");
        return 0;
    }
    if violations.is_empty() {
        // The `✓` is withheld on an incomplete report for the reason `ok` is: it is a claim over a set
        // known to be partial. The weaker sentence is not a hedge for its own sake — it is the only one
        // the input licenses.
        if comp.must_hedge() {
            println!(
                "  · nothing candor COULD SEE violates a `deny`/`pure` boundary — but see the INCOMPLETE \
                 note above; this is not an all-clear."
            );
        } else {
            println!("  ✓ within policy — this edit introduces no `deny`/`pure` boundary violation.");
        }
        0
    } else {
        println!("  ⚠ WOULD VIOLATE policy ({}) — run BEFORE the edit:", violations.len());
        for (f, r, cond) in &violations {
            println!("      [AS-EFF-006] `{f}`  (rule: `{r}`)");
            if let Some(c) = cond {
                println!("          …IF {c}.");
                println!("          This rule NARROWS, and the effect you have not written yet has no class to");
                println!("          match — candor charges it fail-closed rather than guessing which you'd add.");
            }
        }
        1
    }
}

/// ⟨0.24⟩ THE CONDITION A `whatif` VERDICT RESTS ON, when the matched rule NARROWS the effect being
/// introduced — `None` when it does not, which is the ordinary case.
///
/// **WHY THIS EXISTS AT ALL, AND WHY IT IS NOT A FILTER-AWARE MATCH.** Printing the rule VERBATIM is
/// strictly more truthful than rebuilding it, and it is also what makes an existing inaccuracy legible:
/// `whatif` answers a HYPOTHETICAL — "if this function performed `Net`, what happens?" — and a
/// `deny Net[unknown-host]` / `deny Unknown[reflect]` rule quantifies over the DESTINATION or REASON
/// CLASS of the effect you have not written yet. There is no class to match, so the question is
/// genuinely unanswerable, and `unverified`/`fix-gate`'s fix does not carry over: those two read a
/// signature that EXISTS.
///
/// Charging it is the right default for a pre-edit gate (fail-closed; the edit could land in any class),
/// and it stays. What was wrong was showing that unconditional verdict beside a rule reconstructed
/// WITHOUT its filter — the operator read a wide rule, got a wide answer, and never saw their own
/// narrowing. Printing `raw` without this would be worse still: the same unconditional verdict, now
/// attributed to the narrowed line, which reads as candor having evaluated a filter it did not.
///
/// §3.1 ⟨0.24⟩'s rule for exactly this shape is that **an unanswerable condition must be DISCLOSED,
/// never scored as a failed one**. So the verdict is unchanged and the condition rides beside it.
fn narrowing_condition(rule: &candor_classify::policy::PolicyRule, effect: &str) -> Option<String> {
    if effect == candor_classify::policy::UNKNOWN && !rule.unknown_classes.is_empty() {
        let mut t: Vec<&str> = rule.unknown_classes.iter().map(|c| c.token()).collect();
        t.sort_unstable();
        return Some(format!("the `Unknown` you introduce is of reason class {}", t.join(" / ")));
    }
    if effect == "Net" && !rule.net_classes.is_empty() {
        let t: Vec<&str> = rule.net_classes.iter().map(String::as_str).collect(); // BTreeSet ⇒ sorted
        return Some(format!("the `Net` you introduce reaches destination class {}", t.join(" / ")));
    }
    None
}

// ── rewire ──────────────────────────────────────────────────────────────────────────────────────

/// Per caller, the callees it had in the BASELINE call graph but no longer has now (the dropped edges).
pub(crate) fn dropped_edges<'a>(
    cur: &'a BTreeMap<String, Vec<String>>,
    base: &'a BTreeMap<String, Vec<String>>,
) -> BTreeMap<&'a str, Vec<&'a str>> {
    let mut dropped: BTreeMap<&str, Vec<&str>> = BTreeMap::new();
    for (caller, base_callees) in base {
        let now: BTreeSet<&str> =
            cur.get(caller).map(|v| v.iter().map(String::as_str).collect()).unwrap_or_default();
        let gone: Vec<&str> = base_callees.iter().map(String::as_str).filter(|c| !now.contains(c)).collect();
        if !gone.is_empty() {
            dropped.insert(caller.as_str(), gone);
        }
    }
    dropped
}

/// `rewire <cur_prefix> <base_prefix> [0|1]` — the de-wiring detector. Compares the current call graph to
/// a baseline and reports edges a function DROPPED — a call it made in the baseline and no longer makes.
/// The effect gate (`policy`/`whatif`) checks effect BOUNDARIES, not correctness, so it can be satisfied by
/// *disconnecting* functionality: an agent "fixes" a `deny Net api` violation by making `api::handle` stop
/// calling the pricing chain — the gate passes, the feature is broken. That removal is invisible to the
/// effect diff (a pure fn dropping a call changes no effect) but it IS in the call graph. This surfaces it:
/// a passing gate PLUS dropped edges = verify a fix didn't gut the feature. Reads the callgraph sidecars.
pub(crate) fn cmd_rewire(args: &[String]) -> i32 {
    // Recognize `--json` (the family spelling — java/ts rewire ride the shared grammar, which takes it);
    // tolerate candor-ts's output-mode flags; REJECT any other `-`-flag loud (exit 2) — the gains/diff
    // rule, which this bespoke parser lacked (the sibling-route habit: the P8 sink-surface matrix found
    // it hours after the 5cd0d61 sweep). Measured on this binary 2026-08-12: `rewire A B --report --json`
    // ran to exit 0 — BOTH tokens vanished (only a literal `1` in the third slot meant JSON, so even the
    // operator's plain `--json` was silently prose). §3.3.1: a typo'd or not-applicable flag stays an
    // exit-2 error, never a silent swallow. A bare `-` stays positional, matching gains.
    let mut want_json = false;
    let mut pos: Vec<&String> = Vec::new();
    for a in args {
        match a.as_str() {
            "--json" => want_json = true,
            "--text" | "--human" => {}
            other if other.starts_with('-') && other.len() > 1 => {
                let hint = if other == "--policy" { " — `rewire` is a descriptive query with no policy-relative verdict (its SPEC §3.1 JSON shape carries no policy-derived field); apply a policy to this report with `candor-query gate --report <locator> --policy <file>`, or use whatif/fix/fix-gate/unverified for a policy-relative pre-edit check." } else { "" };
                eprintln!("candor-query rewire: unknown flag `{other}`{hint}\n  known flags: --json");
                return 2;
            }
            _ => pos.push(a),
        }
    }
    if pos.len() < 2 {
        eprintln!("usage: candor-query rewire <cur_prefix> <base_prefix> [--json]");
        return 2;
    }
    let (cur_pre, base_pre) = (pos[0], pos[1]);
    // The DEPRECATED old form spelled JSON as a `1` sentinel in the third positional slot.
    if pos.get(2).map(|s| s.as_str()) == Some("1") {
        want_json = true;
    }
    let cur = load_callgraph(cur_pre);
    let base = load_callgraph(base_pre);
    if base.is_empty() {
        eprintln!("candor: no baseline call graph at `{base_pre}` (need its `.callgraph.json` sidecar).");
        return 2;
    }
    // The CURRENT side must be guarded too: a missing/typo'd current prefix loaded an empty graph and
    // reported EVERY baseline edge as "dropped" (a wall of false de-wiring, exit 1) — a CI alarm on a
    // path typo. Fail loud, matching the baseline-side and cmd_diff's no-files check. (/code-review.)
    if cur.is_empty() {
        eprintln!("candor: no current call graph at `{cur_pre}` (need its `.callgraph.json` sidecar).");
        return 2;
    }

    let dropped = dropped_edges(&cur, &base);

    if want_json {
        let out = serde_json::json!({
            "dropped": dropped.iter().map(|(c, g)| serde_json::json!({"caller": c, "no_longer_calls": g}))
                .collect::<Vec<_>>(),
            "ok": dropped.is_empty(),
        });
        println!("{}", serde_json::to_string_pretty(&out).unwrap());
        return if dropped.is_empty() { 0 } else { 1 };
    }
    if dropped.is_empty() {
        println!("  no call edges dropped vs the baseline — nothing de-wired.");
        return 0;
    }
    println!(
        "  {} function(s) DROPPED a call they made in the baseline — a 'fix' may have disconnected \
         functionality (the effect gate can pass while the feature is broken; verify it still works):",
        dropped.len()
    );
    for (caller, gone) in &dropped {
        println!("      {caller}  ⊘  no longer calls: {}", gone.join(", "));
    }
    1
}

/// `gate-verdict <parts-file> <out|->` — assemble the candor-spec §3.3 gate verdict
/// `{ spec, ok, violations }` from a file of NDJSON `GateViolation` records (one JSON object per
/// line — what the deep engine appends to `<CANDOR_GATE_JSON>.parts` per enforcement violation).
/// The wrapper runs this ONCE after the whole `cargo dylint` pass, so the final verdict covers every
/// workspace crate regardless of per-crate write ordering. An ABSENT parts file is the clean run —
/// the spec's `{ ok: true, violations: [] }`. A corrupt record fails (exit 2): a dropped violation
/// would make the verdict under-report vs the gate's exit code, the §3.3 forbidden disagreement. An
/// unwritable output also exits 2 (never silent).
pub(crate) fn cmd_gate_verdict(args: &[String]) -> i32 {
    // ⟨0.15 staged⟩ optional `--report <locator>`: the report whose envelope `coverage` ledger this
    // verdict re-discloses as the ADVISORY `coverage` note (spec §3.3 verb conditionality — a gate
    // verdict over partially-covered code carries the caveat). VERDICT-PRESERVING: ok/violations/exit
    // are computed exactly as before; without the flag, or with a fully-covered report, the output is
    // byte-identical to the pre-⟨0.15⟩ verdict.
    let mut report_loc: Option<String> = None;
    // ⟨0.28⟩ optional `--policy <file>`: the policy the LINT route gated with, re-read through the
    // SHARED parser so the assembled verdict can carry SPEC §6.2's `ignored` disclosure — the lines
    // the parse dropped. The lint's own parse warned per line on stderr during the dylint pass; this
    // is that fact's machine half, and re-deriving it from the same text through the same parser
    // cannot disagree with it. VERDICT-PRESERVING: ok/violations/exit are computed exactly as before,
    // and a clean policy (or no flag) leaves the document byte-identical.
    let mut policy_loc: Option<String> = None;
    let mut pos: Vec<&str> = Vec::new();
    let mut it = args.iter();
    while let Some(a) = it.next() {
        if a == "--policy" {
            match it.next() {
                Some(l) if l == "-" || !l.starts_with('-') => policy_loc = Some(l.clone()),
                Some(l) => {
                    eprintln!("candor-query: --policy was given no value — the next token `{l}` is a flag, not a path (a file really named that is spelled ./{l})");
                    return 2;
                }
                None => {
                    eprintln!("candor-query: --policy requires a file argument");
                    return 2;
                }
            }
        } else if a == "--report" {
            match it.next() {
                Some(l) if l == "-" || !l.starts_with('-') => report_loc = Some(resolve_locator(l)),
                // SPEC §3.2 ⟨0.28⟩: a flag-shaped next token is "given no value" — a usage error, never
                // a locator. Consuming it here was worse than a wrong diagnostic: `resolve_locator`
                // failed SILENTLY (`load_coverage` returns None), so `--report` given no value dropped
                // the coverage advisory and emitted a GREEN verdict at exit 0.
                Some(l) => {
                    eprintln!("candor-query: --report was given no value — the next token `{l}` is a flag, not a locator (a path really named that is spelled ./{l})");
                    return 2;
                }
                None => {
                    eprintln!("candor-query: --report requires a locator argument");
                    return 2;
                }
            }
        } else {
            pos.push(a.as_str());
        }
    }
    let (Some(parts), Some(out)) = (pos.first().copied(), pos.get(1).copied()) else {
        eprintln!("usage: candor-query gate-verdict <parts-file> <out-file|-> [--report <locator>] [--policy <file>]");
        return 2;
    };
    let mut violations: Vec<candor_report::GateViolation> = Vec::new();
    match std::fs::read_to_string(parts) {
        Ok(text) => {
            for line in text.lines().filter(|l| !l.trim().is_empty()) {
                match serde_json::from_str(line) {
                    Ok(v) => violations.push(v),
                    Err(e) => {
                        eprintln!("candor-query: corrupt gate record in {parts} ({e}) — no faithful verdict exists");
                        return 2;
                    }
                }
            }
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {} // clean run: no violations recorded
        Err(e) => {
            eprintln!("candor-query: cannot read {parts} ({e})");
            return 2;
        }
    }
    // ⟨0.15 staged⟩ the advisory note, from the named report's envelope ledger — absent when the flag
    // wasn't given, the report carries no `coverage` field, or the ledger is empty. Package names
    // alphabetical (the same order the scan-time gate advisory uses).
    let coverage = report_loc.as_deref().and_then(load_coverage).filter(|c| !c.uncovered.is_empty()).map(|c| {
        let mut packages: Vec<String> = c.uncovered.iter().map(|e| e.name.clone()).collect();
        packages.sort();
        candor_report::GateCoverage { uncovered: packages.len(), packages }
    });
    // ⟨0.28⟩ the dropped-line disclosure (SPEC §6.2), from the same parse the gate routes use. A
    // CONFIGURED-but-unreadable policy fails loud (§6.2's unreadable-policy posture) — this verb is
    // assembling that gate's verdict, and assembling it as if no policy existed would publish a
    // verdict for a gate whose policy nobody can read.
    let ignored: Vec<candor_report::IgnoredLine> = match policy_loc.as_deref() {
        None => Vec::new(),
        Some(pl) => {
            let Ok(text) = std::fs::read_to_string(pl) else {
                eprintln!("candor-query: gate-verdict --policy {pl} could not be read — failing (exit 2)");
                return 2;
            };
            let aliases = candor_classify::policy::discover_config_text(std::path::Path::new(pl))
                .map(|t| candor_classify::policy::parse_unknown_aliases(&t))
                .unwrap_or_default();
            candor_classify::policy::parse_policy_silent(&text, &aliases)
                .errors
                .iter()
                .filter(|e| !e.fatal)
                .map(|e| candor_report::IgnoredLine {
                    line: e.line,
                    text: e.text.clone(),
                    reason: e.message.clone(),
                })
                .collect()
        }
    };
    let json = match candor_report::gate_verdict_json_with_coverage_v28(&mut violations, coverage.as_ref(), &ignored) {
        Ok(j) => j,
        Err(e) => {
            eprintln!("candor-query: could not serialize the gate verdict ({e})");
            return 2;
        }
    };
    if out == "-" {
        println!("{json}");
        return 0;
    }
    if let Err(e) = candor_report::write_atomic(Path::new(out), format!("{json}\n").as_bytes()) {
        eprintln!("candor-query: could not write the gate verdict to {out} ({e})");
        return 2;
    }
    0
}

#[cfg(test)]
mod tests {
    use super::*;

    /// ⟨0.15 staged⟩ `gate-verdict … --report <loc>`: the advisory coverage note rides the
    /// assembled verdict when the named report's envelope ledger is non-empty, and is
    /// VERDICT-PRESERVING — spec/ok/violations (and the exit code) are identical with and without
    /// the flag; without it, or over a fully-covered report, the verdict is byte-identical to the
    /// pre-⟨0.15⟩ output (the pinned §3.3 fields conformance compares are untouched).
    #[test]
    fn gate_verdict_report_flag_attaches_the_advisory_coverage_note() {
        let dir = std::env::temp_dir().join(format!("candor-gvcov-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let _ = std::fs::create_dir_all(&dir);
        let s = |p: &std::path::Path| p.to_string_lossy().into_owned();
        let parts = dir.join("parts.ndjson");
        std::fs::write(&parts, "{\"rule\":\"AS-EFF-006\",\"fn\":\"f\",\"effects\":[\"Net\"],\"detail\":\"d\"}\n")
            .unwrap();
        let covered = dir.join("rep-cov");
        let _ = std::fs::create_dir_all(&covered);
        std::fs::write(
            covered.join("r.demo.scan.json"),
            r#"{"candor":{"version":"v","toolchain":"t","spec": "0.23"},
                "coverage":{"uncovered":[{"name":"somedep","calls":3},{"name":"anotherdep","calls":1}]},
                "functions":[]}"#,
        )
        .unwrap();
        let full = dir.join("rep-full");
        let _ = std::fs::create_dir_all(&full);
        std::fs::write(
            full.join("r.demo.scan.json"),
            r#"{"candor":{"version":"v","toolchain":"t","spec": "0.23"},"functions":[]}"#,
        )
        .unwrap();
        let (plain, with_cov, fully) = (dir.join("v0.json"), dir.join("v1.json"), dir.join("v2.json"));
        let args = |out: &std::path::Path, rep: Option<&std::path::Path>| -> Vec<String> {
            let mut a = vec![s(&parts), s(out)];
            if let Some(r) = rep {
                a.push("--report".into());
                a.push(s(&r.join("r")));
            }
            a
        };
        assert_eq!(cmd_gate_verdict(&args(&plain, None)), 0);
        assert_eq!(cmd_gate_verdict(&args(&with_cov, Some(&covered))), 0, "same exit with the note");
        assert_eq!(cmd_gate_verdict(&args(&fully, Some(&full))), 0);
        let read = |p: &std::path::Path| std::fs::read_to_string(p).unwrap();
        let (v0, v1) = (read(&plain), read(&with_cov));
        let (j0, j1): (serde_json::Value, serde_json::Value) =
            (serde_json::from_str(&v0).unwrap(), serde_json::from_str(&v1).unwrap());
        for k in ["spec", "ok", "violations"] {
            assert_eq!(j0[k], j1[k], "pinned verdict field `{k}` must be unchanged by the note");
        }
        assert_eq!(j0["ok"], false, "the violation still fails the verdict");
        assert!(j0.get("coverage").is_none(), "no flag → no note (pre-⟨0.15⟩ shape): {v0}");
        assert_eq!(j1["coverage"]["uncovered"], 2);
        assert_eq!(j1["coverage"]["packages"], serde_json::json!(["anotherdep", "somedep"]));
        // A fully-covered report attaches nothing: byte-identical to the no-flag verdict.
        assert_eq!(read(&fully), v0, "fully covered → byte-identical verdict");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
