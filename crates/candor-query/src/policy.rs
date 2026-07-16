//! Policy-facing commands: `parsepolicy`, `whatif`, `rewire`, `gate-verdict`.

use crate::*;

// ── whatif ──────────────────────────────────────────────────────────────────────────────────────

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
    let p = candor_classify::policy::parse_policy(&text);
    let mut deny: Vec<serde_json::Value> = p
        .rules
        .iter()
        .map(|r| {
            serde_json::json!({
                "effects": r.effects.iter().copied().collect::<Vec<&str>>(),
                "scope": r.scope.as_deref().unwrap_or(""),
            })
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
    println!("{}", serde_json::json!({ "deny": deny, "allow": allow, "forbid": forbid }));
    0
}

/// `whatif <prefix> <fn> <Effect> [policy] [0|1]` — the PRE-EDIT verdict. Computes the blast radius of
/// introducing `Effect` into `fn` (the fn + every transitive caller, all of which would gain it), then —
/// given a policy — reports which of them would VIOLATE a `deny <Effect>` / `pure` boundary. Answers
/// "if I add a network call here, what happens and is it allowed?" BEFORE the edit, instead of edit →
/// run the gate → revert. Read-only over the call-graph sidecar + the policy file.
pub(crate) fn cmd_whatif(args: &[String]) -> i32 {
    let g = parse(args, Shape { verb_args: 2, sentinel: true, has_policy: true });
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
    let rules = match policy_path.as_deref() {
        None => None,
        Some(p) => match std::fs::read_to_string(p) {
            Ok(t) => Some(candor_classify::policy::parse_policy(&t).rules),
            Err(e) => {
                eprintln!("candor: policy `{p}` could not be read ({e}) — verdict NOT computed.");
                return 2;
            }
        },
    };
    let mut violations: Vec<(&str, String)> = Vec::new();
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
                    let r = if rule.effects.is_empty() {
                        format!("pure{}", rule.scope.as_deref().map(|s| format!(" {s}")).unwrap_or_default())
                    } else {
                        format!("deny {effect}{}", rule.scope.as_deref().map(|s| format!(" {s}")).unwrap_or_default())
                    };
                    violations.push((fname, r));
                    break;
                }
            }
        }
    }

    if want_json {
        let out = serde_json::json!({
            "of": targets,
            "effect": effect,
            "affected": affected.iter().collect::<Vec<_>>(),
            "violations": violations.iter().map(|(f, r)| serde_json::json!({"fn": f, "rule": r})).collect::<Vec<_>>(),
            "ok": violations.is_empty(),
        });
        println!("{}", serde_json::to_string_pretty(&out).unwrap());
        return if violations.is_empty() { 0 } else { 1 };
    }

    println!("whatif: adding `{effect}` to `{}`", targets.join(", "));
    println!("  → propagates to {} function(s) (the blast radius):", affected.len());
    for f in &affected {
        println!("      {f}");
    }
    if rules.is_none() {
        println!("  (no policy given — pass a policy file or set CANDOR_POLICY for the gate verdict)");
        return 0;
    }
    if violations.is_empty() {
        println!("  ✓ within policy — this edit introduces no `deny`/`pure` boundary violation.");
        0
    } else {
        println!("  ⚠ WOULD VIOLATE policy ({}) — run BEFORE the edit:", violations.len());
        for (f, r) in &violations {
            println!("      [AS-EFF-006] `{f}`  (rule: `{r}`)");
        }
        1
    }
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
    if args.len() < 2 {
        eprintln!("usage: candor-query rewire <cur_prefix> <base_prefix> [0|1]");
        return 2;
    }
    let (cur_pre, base_pre) = (&args[0], &args[1]);
    let want_json = args.get(2).map(|s| s == "1").unwrap_or(false);
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
    let mut pos: Vec<&str> = Vec::new();
    let mut it = args.iter();
    while let Some(a) = it.next() {
        if a == "--report" {
            match it.next() {
                Some(l) => report_loc = Some(resolve_locator(l)),
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
        eprintln!("usage: candor-query gate-verdict <parts-file> <out-file|-> [--report <locator>]");
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
    let json = match candor_report::gate_verdict_json_with_coverage(&mut violations, coverage.as_ref()) {
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
            r#"{"candor":{"version":"v","toolchain":"t","spec": "0.17"},
                "coverage":{"uncovered":[{"name":"somedep","calls":3},{"name":"anotherdep","calls":1}]},
                "functions":[]}"#,
        )
        .unwrap();
        let full = dir.join("rep-full");
        let _ = std::fs::create_dir_all(&full);
        std::fs::write(
            full.join("r.demo.scan.json"),
            r#"{"candor":{"version":"v","toolchain":"t","spec": "0.17"},"functions":[]}"#,
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
