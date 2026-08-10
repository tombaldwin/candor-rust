//! Property-based tests for the §6.2 policy parser — the surface the family's fuzzers do not reach.
//!
//! WHY HERE, AND WHY NOT MORE FUZZING. Every engine already has a generative soundness harness
//! (candor-rust `soundness/`, candor-ts `fuzz.mjs`, candor-agents `fuzz.py`), and all three generate
//! CODE and check EFFECT PROPAGATION: a function that transitively reaches an effect must never read
//! pure. None of them generates a POLICY. That is the gap, and it is not an idle one — the policy
//! parser is where this project's fail-open defects have actually lived:
//!
//!   · `deny Unknown[corp]` (sole unrecognised token) widened to a bare `deny Unknown` while printing
//!     "ignoring policy rule" — a FALSE disclosure.
//!   · `deny Unknown[dispatch,nativ]` (a typo BESIDE valid tokens) narrowed to `[dispatch]` and stopped
//!     gating native-caused holes while the operator read a gate that looked armed. ⟨0.24⟩ made both
//!     unhonourable-as-written, which is what `ParsedPolicy::errors` records.
//!
//! Both are the same shape: the operator wrote a rule, nothing enforces it, and nothing says so. These
//! properties state that shape directly, over generated input rather than over the fixed battery in
//! `conformance/policydsl/`. A fixed battery tests the lines someone thought of; this tests the space.
//!
//! Property-based rather than a fuzzer for one concrete reason: SHRINKING. A failure here reports the
//! minimal line that breaks the rule, which is the difference between "some 40-line generated policy
//! misbehaves" and "`deny Unknown[a,b` misbehaves".

use crate::policy::{parse_policy_quiet, ParsedPolicy};
use proptest::prelude::*;

/// Is this line one a policy author would expect to DO something? Blank and comment lines are neither
/// honoured nor an error, and correctly so.
fn is_meaningful(line: &str) -> bool {
    let t = line.split('#').next().unwrap_or("").trim();
    !t.is_empty()
}

fn honoured_count(p: &ParsedPolicy) -> usize {
    p.rules.len() + p.allow_rules.len() + p.layer_rules.len()
}

/// A vocabulary deliberately mixing VALID tokens with near-misses, because the defects above were all
/// near-misses: a typo that lands beside correct tokens is the common case, not a lone garbage token.
fn token() -> impl Strategy<Value = String> {
    prop_oneof![
        // valid effect names and rule kinds
        Just("Net".to_string()), Just("Fs".to_string()), Just("Exec".to_string()),
        Just("Env".to_string()), Just("Clock".to_string()), Just("Unknown".to_string()),
        Just("deny".to_string()), Just("allow".to_string()), Just("pure".to_string()),
        Just("forbid".to_string()), Just("in".to_string()), Just("->".to_string()),
        // near-misses of each
        Just("net".to_string()), Just("NET".to_string()), Just("Nett".to_string()),
        Just("Deny".to_string()), Just("denyy".to_string()), Just("fobid".to_string()),
        // Unknown filters, valid and typo'd, alone and beside a valid token
        Just("Unknown[dispatch]".to_string()), Just("Unknown[dispatch,native]".to_string()),
        Just("Unknown[dispatch,nativ]".to_string()), Just("Unknown[corp]".to_string()),
        Just("Unknown[]".to_string()), Just("Unknown[".to_string()),
        // scopes and literals
        Just("app".to_string()), Just("billing".to_string()), Just("api.stripe.com".to_string()),
        Just("a.b.c".to_string()), Just("*".to_string()),
        // and free-form, so the space is not only the tokens someone listed
        "[a-zA-Z0-9_.:*\\[\\],-]{1,12}".prop_map(|s| s),
    ]
}

fn line() -> impl Strategy<Value = String> {
    prop::collection::vec(token(), 1..5).prop_map(|ts| ts.join(" "))
}

proptest! {
    /// P1 — ACCOUNTING. Every meaningful line is either HONOURED (it became a rule the gate will
    /// enforce) or DISCLOSED (it is in `errors`, which every gate route reads). A line that is neither
    /// is silently dropped, and a silently dropped rule is a gate the operator believes is armed.
    ///
    /// This is the policy-surface statement of the cardinal sin: not a wrong answer, an absent one.
    #[test]
    fn every_meaningful_line_is_honoured_or_disclosed(l in line()) {
        prop_assume!(is_meaningful(&l));
        let p = parse_policy_quiet(&l);
        prop_assert!(
            honoured_count(&p) > 0 || !p.errors.is_empty(),
            "line was neither honoured nor disclosed — silently dropped: {l:?}"
        );
    }

    /// P2 — LINE INDEPENDENCE. Parsing lines together answers exactly as parsing them apart. A parser
    /// where one line changes how the NEXT one reads is how a single malformed rule silently disarms
    /// the rules below it — and this parser has a live hazard of exactly that shape, recorded in its own
    /// comments: `str::lines()` does not split a bare `\r`, so a classic-Mac policy collapses to ONE
    /// line and every rule after the first is swallowed. That is a whole gate lost to a line ending.
    #[test]
    fn lines_do_not_interfere(ls in prop::collection::vec(line(), 1..6),
                              sep in prop_oneof![Just("\n"), Just("\r\n"), Just("\r")]) {
        let ls: Vec<String> = ls.into_iter().filter(|l| is_meaningful(l)).collect();
        prop_assume!(!ls.is_empty());
        // ALL THREE LINE ENDINGS, because the hazard above is specifically about the one `str::lines()`
        // does not split. A generator that only ever joins with `\n` describes the risk without testing
        // it — which is the difference between a comment and a guard.
        let together = parse_policy_quiet(&ls.join(sep));
        let apart: (usize, usize) = ls.iter().map(|l| {
            let p = parse_policy_quiet(l);
            (honoured_count(&p), p.errors.len())
        }).fold((0, 0), |a, b| (a.0 + b.0, a.1 + b.1));
        prop_assert_eq!(
            (honoured_count(&together), together.errors.len()), apart,
            "parsing {} lines together differs from parsing them apart: {:?}", ls.len(), ls
        );
    }

    /// P3 — A TYPO IN AN `Unknown[…]` FILTER IS FATAL, WHEREVER IT SITS. This is the ⟨0.24⟩ defect
    /// stated as a property rather than as two fixtures.
    ///
    /// The measured failure had two halves and only one of them looked alarming. A SOLE unrecognised
    /// token emptied the filter and WIDENED the rule to a bare `deny Unknown` — surprising, but loud.
    /// A typo BESIDE valid tokens NARROWED the rule to the tokens that parsed, so it silently stopped
    /// gating the classes the operator had asked about while the gate still looked armed. That half is
    /// the fail-open, and it is the common case: a typo lands next to correct tokens far more often
    /// than alone. So the property quantifies over BOTH — any position, any mix — and asserts the one
    /// thing that makes them safe: the parser must mark the policy unhonourable, never re-scope it.
    #[test]
    fn a_typo_in_an_unknown_filter_is_always_fatal(
        valid in prop::collection::vec(
            prop_oneof![Just("reflect"), Just("dispatch"), Just("indirect"),
                        Just("native"), Just("unresolved"), Just("setup")], 0..3),
        typo in prop_oneof![Just("nativ"), Just("corp"), Just("dispatchh"), Just("Reflect"), Just("zz")],
        at in 0usize..4,
    ) {
        let mut toks: Vec<&str> = valid.clone();
        let at = at.min(toks.len());
        toks.insert(at, typo);
        let l = format!("deny Unknown[{}] app", toks.join(","));
        let p = parse_policy_quiet(&l);
        prop_assert!(
            p.errors.iter().any(|e| e.fatal),
            "an unrecognised reason-class token must make the policy unhonourable — it was instead \
             absorbed, leaving a rule that gates less than it says: {l:?} -> {} rule(s), {} error(s)",
            honoured_count(&p), p.errors.len()
        );
    }
}
