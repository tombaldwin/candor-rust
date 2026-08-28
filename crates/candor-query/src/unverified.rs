//! `candor unverified` — the provable-purity disclosure (a policy-guidance companion to `fix`, from
//! eval/fixloop/DISPATCH-NOTE.md). A `deny <E>` or `pure` rule PASSES a function that carries none of its
//! forbidden effects — but if that function is `Unknown` (candor could not resolve one of its calls), the
//! pass is UNVERIFIED: the Unknown could hide the very effect the rule forbids (the classic case is a
//! fn/closure-injected "port" — the layer reads as Unknown, so `deny Net domain`/`pure domain` clear it even
//! though the domain may reach Net at runtime). This names every such function in a governed layer and the
//! `deny <E> Unknown <scope>` upgrade that makes the intent provable. Advisory: exit 0, or `--strict` → exit
//! 1 so CI can REQUIRE provable purity. The gate's verdict is untouched — this only discloses the gap.
//!
//! ⟨0.24⟩ It ALSO names every function `gate --report` could not JUDGE (SPEC §3.2, candor-spec
//! `4fd140c`: *an advisory verb may be LESS certain than the gate, never more*) — with the MISSING
//! EVIDENCE as the reason, the gate's `unevaluated` shape beside it, and `--strict` → exit 2 there,
//! matching the gate.
//!
//! ⟨0.24⟩ …and over an INCOMPLETE report it omits `ok` entirely (SPEC §3.2, candor-spec `ec1a441`) —
//! see [`crate::completeness`], where the measurement and the reasoning live. **This verb is the
//! sharpest case in the family**: it exists to say *"your green gate is not provably green"*, and a
//! function in an unanalyzed file is absent from `functions`, so it cannot be enumerated as an
//! unverified pass at all — that absence is exactly what the verb would have to report.

use crate::grammar::{parse, report_or_discover, Shape};
use candor_classify::policy::{rule_and_upgrade, unverified_hole_rule, PolicyRule};
use candor_report::ReportEntry;

// ⟨0.24⟩ **THE REASON-CLASS FIXPOINT USED TO LIVE HERE, AND THAT WAS THE DEFECT'S HOME ADDRESS** (SPEC
// §3.2, candor-spec `4fd140c`). `reason_class_acc` was this verb's own copy of the gate's accumulator —
// the same two faults reasoned through twice, in two files — and *"an advisory verb may be LESS certain
// than the gate, never more"* is a COMPARISON between the two, which two copies can only ever satisfy by
// coincidence. Both this verb and `fix-gate` now read [`crate::gate::report_signature`], the accumulator
// `gate --report` itself is judged from, and take the ANSWERABILITY set from the same object.

pub(crate) fn cmd_unverified(args: &[String]) -> i32 {
    let g = parse(args, Shape { verb_args: 0, sentinel: true, has_policy: true, verb: "unverified" });
    let Some(prefix) = report_or_discover(&g) else {
        eprintln!("candor: no report found (no --report and no .candor/ discovered) — scan the crate first.");
        return 2;
    };
    let prefix = &prefix;
    let want_json = g.want_json;
    let strict = g.strict;
    let policy_path = g.policy.clone().or_else(|| std::env::var("CANDOR_POLICY").ok());
    let Some(pp) = policy_path else {
        eprintln!("candor unverified: a policy is required (the check is relative to your pure/deny layers).");
        return 2;
    };
    // ⟨0.24⟩ Through the SHARED loader, and here the old bare `parse_policy` LOST A DISCLOSURE rather
    // than adding one: a hole is a function that PASSES its rule while being `Unknown`, so widening
    // `deny Unknown[<alias>]` to a bare `deny Unknown` reclassified real holes as violations-that-aren't
    // and this verb answered "every function in a pure/deny layer is PROVABLY clean ✓". §6.2: the gate
    // and the disclosure MUST apply the same rule.
    let parsed = match crate::policy::load_policy_as_the_gate_does("unverified", &pp) {
        Ok(p) => p,
        Err(code) => return code,
    };
    let rules = &parsed.rules;
    // ⟨0.28⟩ THROUGH THE LOUD LOADER, NOT A BARE EMPTINESS CHECK. `entries.is_empty()` conflated two
    // causes that SPEC rules in OPPOSITE directions: *no report file at all* (§3.2's "no report is a
    // loud failure" — exit 2, and `load_entries_loud` also keeps a net-corrupt report loud) and *a
    // well-formed report that JUDGED NOTHING* (`functions: []`, `analyzed.count: 0` — SPEC §2 ⟨0.24⟩:
    // "A DISCLOSURE, NOT AN EXIT CODE"). This verb exited 2 over the second, claiming it got LESS far
    // than `gate --report` on identical bytes — the mirror of the over-claim `unverified_exit` exists
    // to prevent, and the outlier posture on the rung commit `e1a341f` defined: the count-0 cause
    // reaches both disclosure channels (via `report_completeness` below) and STOPS at the exit code.
    let entries = match crate::load::load_entries_loud(prefix) {
        Ok(e) => e,
        Err(code) => return code,
    };

    // A hole: a function that is Unknown, sits in a deny/pure scope, and PASSES that rule (carries none of its
    // forbidden real effects). The predicate is `unverified_hole_rule` — the SAME one candor-scan's gate note
    // uses (candor_classify::policy), so the disclosure can never drift between the two paths.
    struct Hole<'a> {
        func: &'a ReportEntry,
        rule: &'a PolicyRule,
    }
    // `--class <c,…>` (SPEC §3.1 ⟨0.20⟩, semantics pinned normative at §6.2 ⟨0.24⟩): keep only holes
    // whose Unknown is of a matching reason class — resolved TRANSITIVELY, over the same reach the
    // `deny E Unknown[class]` gate resolves, and failing CLOSED on a hole nothing classified.
    // §6.2 ⟨0.24⟩: an unrecognised token is a USAGE ERROR (exit 2), never a silently narrowed filter —
    // see `parse_class_filter` for why this half of the rule is not the policy side's drop-with-warning.
    let class_filter = match g.class.as_deref().map(crate::containment::parse_class_filter).transpose() {
        Ok(v) => v,
        Err(msg) => {
            eprintln!("{msg}");
            return 2;
        }
    };
    let want: Option<std::collections::BTreeSet<&str>> = class_filter
        .as_ref()
        .map(|set| set.iter().map(|c| c.token()).collect());
    // Computed once. ⟨0.24⟩ UNCONDITIONALLY, where it used to be built only for `--class`: the hole
    // predicate itself needs it now, because a rule with a narrowing `Unknown[…]` filter PASSES or FIRES
    // on this very set, and the disclosure names the holes the gate did not clear. Making the fixpoint
    // conditional on the POLICY's shape as well would be a third place that has to agree about which
    // rules narrow — the arithmetic that decides is one traversal of a report already in memory.
    //
    // ⟨0.24⟩ AND IT IS THE GATE'S OWN SIGNATURE, not a second accumulator beside it (SPEC §3.2) — see the
    // note where `reason_class_acc` used to live.
    let sig = crate::gate::report_signature(&entries);
    let reason_acc = &sig.reason_classes;
    let class_matches = |e: &ReportEntry| -> bool {
        match &want {
            Some(w) => candor_classify::policy::reason_class_matches(reason_acc.get(&crate::gate::entry_key(e)), w),
            None => true, // no --class ⇒ no filter
        }
    };
    let no_classes: Vec<String> = Vec::new();
    let holes: Vec<Hole> = entries
        .iter()
        .filter_map(|e| {
            // ⟨0.20⟩ `netClass` is read VERBATIM off the wire, exactly as `gate --report` reads it — the
            // gate does not recompute it from the hosts on this route and neither may the disclosure.
            let nets = if e.net_class.is_empty() { &no_classes } else { &e.net_class };
            unverified_hole_rule(&e.func, &e.inferred, reason_acc.get(&e.func), nets, rules)
                .filter(|_| class_matches(e))
                .map(|rule| Hole { func: e, rule })
        })
        .collect();

    // ⟨0.24⟩ **THE FUNCTIONS THE GATE COULD NOT JUDGE AT ALL** — SPEC §3.2, candor-spec `4fd140c`:
    // *"where the gate would refuse for want of evidence, `unverified` MUST NAME the function."*
    //
    // THE DEFECT, measured four-way by conformance R11 and here on this engine before the fix: over a
    // report carrying `hosts` and no `netClass`, under `deny Net[unknown-host] app`, `gate --report`
    // exits 2 — §3.1 answerability, it CANNOT judge `app.noClass` — and this verb printed
    // `{"ok": false, "unverified": [app.nativeHole]}`, exit 0. It named a hole, so every "the verb said
    // SOMETHING" check passed; the function the gate withheld on was cleared in silence. **The verb whose
    // entire job is "your green gate is not provably green" was more confident than the gate over
    // identical bytes.**
    //
    // A function the gate COULD NOT JUDGE is an unverified hole in the strongest sense this verb has, so
    // it is named — and the reason recorded is **the MISSING EVIDENCE**, `why` verbatim from the gate's
    // own refusal. Recording what a derivation would have concluded instead (this engine could floor
    // `app.noClass` at `unknown-host` from its `hosts` in one line) is the move the ruling forbids: a
    // derivation is not a hedge, it is a second opinion, and it would restate the defect as a disclosure.
    //
    // **NOT SUBJECT TO `--class`.** That filter selects holes by REASON CLASS, and the whole content of
    // an entry here is that the class evidence is the thing missing — narrowing it away would be the
    // absence-keyed relaxation this rung exists to close, arriving through a flag.
    let mut unanswered = crate::gate::unanswerable_pairs(&parsed, &sig);
    // ⟨0.29⟩ …AND THE TWO WHOLE-POLICY KINDS, for the same reason and by the same shared function as
    // `fix-gate`. `unanswerable_pairs` walks `deny` rules only, so a `forbid`-only policy left this set
    // empty and the verb printed *"every function in a pure/deny layer is PROVABLY clean (no Unknown
    // holes) ✓"* at exit 0 — measured — over a policy nothing had evaluated. The claim is relative to a
    // gate that never ran, which is exactly what this verb's ⟨0.24⟩ disclosure exists to prevent one
    // level down. No `func`: the kind is unanswerable over the whole report, not at one function.
    unanswered.extend(crate::gate::whole_policy_refusals(&parsed, &pp).into_iter().map(|u| {
        crate::gate::Unanswerable { rule: u.rule, func: String::new(), why: u.why }
    }));

    // ⟨0.24⟩ **AND WHAT THE PRODUCING SCAN COULD NOT SEE AT ALL** — SPEC §3.2, candor-spec `ec1a441`.
    // The two disclosures are independent and both are needed: `unanswered` is a function candor DID
    // analyze and the gate could not JUDGE; this is source the scan never read, so there is no function
    // to name. MEASURED on the release build over a report declaring one `unanalyzed` unit, NO holes and
    // `deny Net app` that nothing violates: `{"ok": true, "unverified": []}`, exit 0 under `--strict`,
    // and the stdout line *"every function in a pure/deny layer is PROVABLY clean (no Unknown holes) ✓"*
    // — over a report that declares source candor could not read.
    //
    // ⟨0.32⟩ …AND THE CLASSES NOTHING OPENED, armed against THIS run's policy — see
    // [`crate::completeness::arm_unread`]. Computed ONCE, here, and every channel below reads this one
    // value, so the exit code and the document cannot disagree about a run. MEASURED on the release
    // build at `ab505c0`, over a no-policy report of a tree with an unreadable `build.rs`, under
    // `deny Exec`: `gate --report` exited 2 while this verb printed `{"ok": true, "unverified": []}` at
    // exit 0 — the verb whose whole job is *"your green gate is not provably green"* certifying a
    // universe it is on record as not having seen. (Over a fixture with an `Unknown` in it the verb
    // exits 1 on the HOLES and reads as a refusal; that is a different finding, not this rule.)
    // ⟨0.33⟩ …and the cross-policy cause (SPEC §2 ⟨0.33⟩), armed on the SAME parsed policy: `gate
    // --report` refuses a report whose peek was bounded by a different deny set, so this verb must not
    // certify over one either.
    let comp = crate::completeness::arm_unasked_rules(
        crate::completeness::arm_unread(crate::completeness::report_completeness(prefix), &parsed),
        &parsed,
    );
    comp.warn_unreadable("unverified");

    // ⟨0.28⟩ SPEC §2: a CONFIGURED policy that parsed to zero rules asked nothing — there is no
    // pure/deny layer for a hole to pass, so an empty `unverified` list would be the prose `✓` in
    // wire form over a gate that never asked a question. The caveat document replaces the result;
    // the EXIT is unchanged (the same expression the result path computes, over empty finding sets).
    if crate::policy::policy_asked_nothing(&parsed) {
        crate::policy::emit_zero_rule_caveat("unverified", &pp, want_json, &comp);
        return unverified_exit(strict, false, false, comp.incomplete());
    }

    if want_json {
        let mut items: Vec<_> = holes
            .iter()
            .map(|h| {
                let (rule, upgrade) = rule_and_upgrade(h.rule);
                serde_json::json!({
                    "fn": h.func.func,
                    "rule": rule,
                    "unknownWhy": h.func.unknown_why,
                    "upgrade": upgrade,
                })
            })
            .collect();
        // `rule` is the join field a consumer already uses against `unevaluated` (SPEC §3.1), and `why`
        // is the SAME string the pair carries — one function built both, so the two cannot drift. There
        // is no `upgrade`: no policy edit makes a missing field appear, and printing one would advise a
        // remedy for the wrong problem.
        items.extend(unanswered.iter().map(|u| {
            serde_json::json!({ "fn": u.func, "rule": u.rule, "why": u.why })
        }));
        let out = serde_json::json!({
            "ok": items.is_empty(),
            "unverified": items,
            // ⟨0.24⟩ THE GATE'S OWN SHAPE, `[{rule, why}]` (SPEC §3.1 `fc4b5f6`), one entry per RULE —
            // deliberately NOT a second spelling. Omitted entirely when everything was answerable, so an
            // ordinary document stays byte-identical to a pre-ruling one.
            "unevaluated": unevaluated_json(&unanswered),
        });
        let mut out = out;
        if unanswered.is_empty() {
            out.as_object_mut().unwrap().remove("unevaluated");
        }
        // ⟨0.24⟩ `ok` is REMOVED, not set to `false` (SPEC §3.2 `ec1a441`): `false` here would assert
        // "an unverified hole exists, here it is" beside an empty array — a finding the analysis never
        // made. `unverified` and `unevaluated` still ship: a partial answer that says it is partial
        // beats a refusal. On a COMPLETE report nothing below fires and the document is byte-identical.
        //
        // ⟨0.24⟩ **AND THE WITHHELD-RULE TRIGGER TAKES THE SAME ANSWER** (SPEC §3.2 `142740a`). This
        // engine emitted `ok: false` there, which `4fd140c` argued for deliberately and which was wrong
        // by that same clause's own reasoning: where a rule was WITHHELD, no hole was FOUND — the
        // question was declined — so `false` asserts the finding that did not happen. The two triggers
        // were ruled a day apart and looked like two cases; they are one shape and one answer.
        // MEASURED here before the change: `deny Net[unknown-host] app` over a `hosts`-only entry gave
        // `{"ok": false, …}` while `gate --report` refused outright. `fix-gate` was already right.
        // ⟨0.28⟩ `must_hedge`, not `incomplete`: a judged-nothing report licenses `ok` no more than an
        // unanalyzed one does. The EXIT below still reads `incomplete()`, because ⟨0.24⟩ fixed count-0's
        // exit at the gate's — see [`crate::completeness::ReportCompleteness::incomplete`].
        if comp.must_hedge() || !unanswered.is_empty() {
            out.as_object_mut().unwrap().remove("ok");
            comp.write_json(&mut out);
        }
        println!("{}", serde_json::to_string_pretty(&out).unwrap());
        return unverified_exit(strict, !holes.is_empty(), !unanswered.is_empty(), comp.incomplete());
    }

    // ⟨0.24⟩ THE HUMAN CHANNEL, AND IT IS THE ONE A TEST CANNOT SEE. A mutant that kept the whole JSON
    // fix and deleted this call survived the entire suite (SPEC §3.2 `ec1a441`) — the prose `✓` IS the
    // prose `ok: true`. Printed FIRST, so it qualifies the lists below as much as the verdict.
    comp.print_note(
        "the functions named below are only those candor could see",
        // ⟨0.28⟩ `gate_line()`, not a fixed "exits 2" claim: the two causes get OPPOSITE answers from
        // the gate, and over a judged-nothing-only report the old sentence sent the reader to a CI job
        // that passes. Byte-identical on the `unanalyzed` arm — `gate_line()` IS the old sentence there.
        &format!(
            "A function in one of those is ABSENT from the report, so it cannot be named here at all. \
             {} Re-scan for a complete answer.",
            comp.gate_line()
        ),
    );

    if holes.is_empty() && unanswered.is_empty() {
        if comp.must_hedge() {
            // NO `✓`, and not "PROVABLY" anything. The withheld tick is the same withdrawal `ok` is:
            // a claim of provable purity over a set candor is on record as not having seen.
            println!(
                "candor unverified: nothing candor COULD SEE is an unverified hole — but see the \
                 INCOMPLETE note above; this is NOT the provably-clean all-clear."
            );
            // ⟨0.28⟩ `comp.incomplete()`, NOT a literal `true`: `must_hedge()` is the trigger for the
            // WITHDRAWAL above, but the exit follows the gate, and a judged-nothing-only report is the
            // arm ⟨0.24⟩ ruled "a disclosure, not an exit code". The literal made this verb's two
            // channels disagree about one run — prose `--strict` exited 2 where `--json --strict`
            // exited 0 over identical bytes (measured).
            return unverified_exit(strict, false, false, comp.incomplete());
        }
        println!("candor unverified: every function in a pure/deny layer is PROVABLY clean (no Unknown holes) ✓");
        return 0;
    }
    if !holes.is_empty() {
        println!(
            "candor unverified — {} function(s) PASS their policy but aren't PROVABLY clean:\n",
            holes.len()
        );
    }
    let mut upgrades: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
    for h in &holes {
        let (rule, upgrade) = rule_and_upgrade(h.rule);
        upgrades.insert(upgrade.clone());
        println!("  `{}`  (in `{rule}`)", h.func.func);
        let why = if h.func.unknown_why.is_empty() {
            "an unresolvable call".to_string()
        } else {
            h.func.unknown_why.join(", ")
        };
        println!("     is Unknown ({why}) — candor can't confirm it's free of the forbidden effect(s);");
        println!("     the Unknown could hide the very effect the rule forbids (e.g. a fn/closure-injected port).");
        println!("     → make it provable:  add  `{upgrade}`");
        println!();
    }
    if !unanswered.is_empty() {
        // ⟨0.29⟩ COUNT RULES AND FUNCTIONS SEPARATELY. Every entry used to name a function, so the header
        // said "N function(s)" and each line printed `` `func` (in `rule`) ``. The whole-policy kinds
        // (`forbid`, `allow`) are unanswerable over the REPORT, not at a function, so they carry an empty
        // `func` — and printing them through the old shape produced a bare ```` `` ```` and a count of
        // functions that included something that is not one. A refusal rendered as an empty name is worse
        // than no line: it reads as a bug in the tool, and the reader stops believing the block.
        let (whole, per_fn): (Vec<_>, Vec<_>) =
            unanswered.iter().partition(|u| u.func.is_empty());
        if !per_fn.is_empty() {
            println!(
                "candor unverified — {} function(s) the GATE COULD NOT JUDGE over this report \
                 (`candor-query gate --report` refuses on them, SPEC §3.1):\n",
                per_fn.len()
            );
            for u in &per_fn {
                println!("  `{}`  (in `{}`)", u.func, u.rule);
                println!("     {}", u.why);
                println!();
            }
        }
        if !whole.is_empty() {
            println!(
                "candor unverified — {} POLICY RULE(S) the GATE COULD NOT EVALUATE over this report at \
                 all (SPEC §3.1 answerability — not a property of any one function):\n",
                whole.len()
            );
            for u in &whole {
                println!("  `{}`", u.rule);
                println!("     {}", u.why);
                println!();
            }
        }
    }
    if !holes.is_empty() {
        // "on these" ONLY when the unanswered block is also on screen, where an unqualified "the gate
        // still PASSES" would be false. With nothing unanswered the sentence is the pre-ruling one, to
        // the byte — measured across 224 OLD/NEW runs over four corpora and eight policies, where this
        // line was the ONLY difference until it was made conditional.
        //
        // ⟨0.24⟩ …and over an INCOMPLETE report it is not narrowed but WITHDRAWN, because it is false:
        // `gate --report` over these bytes exits 2, so "the gate still PASSES" is a claim about the
        // gate that the gate contradicts. Found by reading this verb's every printed sentence for the
        // claim it makes, which is what `ec1a441`'s every-channel clause asks for — the `✓` was not the
        // only one.
        if comp.incomplete() {
            println!(
                "  The gate does NOT pass over this report — it declares unanalyzed unit(s) (above) and \
                 `gate --report` exits 2. Once the scan is complete, to REQUIRE provable purity add:"
            );
        } else {
            let scope = if unanswered.is_empty() { "" } else { " on these" };
            println!("  The gate still PASSES{scope} — this is advisory. To REQUIRE provable purity, add:");
        }
        for u in &upgrades {
            println!("      {u}");
        }
    }
    unverified_exit(strict, !holes.is_empty(), !unanswered.is_empty(), comp.incomplete())
}

/// ⟨0.24⟩ The `unevaluated` disclosure — the gate's `[{rule, why}]`, ONE ENTRY PER RULE.
///
/// Per-rule rather than per-function because that is the shape `gate --report` emits and SPEC §3.1
/// `fc4b5f6` fixes; the per-FUNCTION detail is in `unverified` itself, where the ruling puts it, and the
/// two join on `rule`.
fn unevaluated_json(unanswered: &[crate::gate::Unanswerable]) -> Vec<serde_json::Value> {
    let mut seen: std::collections::BTreeSet<&str> = std::collections::BTreeSet::new();
    unanswered
        .iter()
        .filter(|u| seen.insert(u.rule.as_str()))
        .map(|u| serde_json::json!({ "rule": u.rule, "why": u.why }))
        .collect()
}

/// ⟨0.24⟩ `--strict`'s exit code, with the REFUSAL DOMINATING (SPEC §3.2, candor-spec `4fd140c`:
/// *"`--strict` exits 2, matching the gate"*).
///
/// **THE PRECEDENCE IS THE OPPOSITE OF THE GATE'S, AND FOR THE GATE'S OWN REASON.** There, a firing rule
/// dominates a refusal because `Reject` is upward-closed: exit 1 is CERTAIN and no missing evidence can
/// un-reject it. Here neither outcome is certain — both are advisory — and the question the exit code
/// answers is *did this verb evaluate the policy you gave it?*. Where the gate answered "no" with a 2,
/// this verb answering 1 would claim it got further than the gate did on identical bytes, which is the
/// bound the ruling sets. So 2 wins, and the holes are still all named in the document either way.
///
/// Without `--strict` the verb is advisory and exits 0, unchanged: the ruling is about the DISCLOSURE,
/// and minting a non-zero exit for the default agent-loop invocation would fail builds this verb has
/// never failed.
///
/// ⟨0.24⟩ **AN INCOMPLETE REPORT JOINS THE 2**, SPEC §3.2 `ec1a441` — *"`--strict` (the CI form) exits
/// 2"* — and it is the same argument one rung along: `gate --report` exits 2 over these bytes, so
/// answering 0 (or 1) claims this verb got further than the gate on identical input. It sits beside the
/// refusal rather than under it because both are the SAME answer, *this verb did not evaluate the
/// policy you gave it over the code you gave it*.
fn unverified_exit(strict: bool, any_holes: bool, any_unanswered: bool, incomplete: bool) -> i32 {
    match (strict, any_unanswered || incomplete, any_holes) {
        (true, true, _) => 2,
        (true, false, true) => 1,
        _ => 0,
    }
}
