//! SOURCE-HYGIENE CENSUS — ported from candor-java's `SourceHygieneTest` (BACKLOG item 3).
//!
//! Every assertion here reads this engine's OWN SOURCE and counts. That is worth doing because the
//! defects it catches are invisible to every behavioural test: a rule stated in ONE place and then
//! silently copied, or a single rule that no longer gets ASKED. Both keep every suite green.
//!
//! The java original is the model and carries the shape: assert the copy count of a literal, assert the
//! single rule is actually consulted N times, and carry a VACUITY FLOOR so the census cannot go green
//! by failing to find the source it is asserting about.
//!
//! **WHERE THIS PORT DEPARTS FROM THE ORIGINAL, deliberately.** java asserts "the reserved segments are
//! listed exactly ONCE". Asserting that here would be WRONG, and acting on it would have introduced a
//! file-deletion bug: rust's sweep in `candor-scan/src/gate.rs` is a DELETION list, where a miss is safe
//! and an over-reach destroys a file that is not ours. It is a deliberate SUBSET of
//! `candor_report::SIDECAR_KINDS`, and the two names it drops are load-bearing. So this file pins the
//! DIFFERENCE instead of forbidding it — which is the stronger assertion, because it fails in both
//! directions: widening the sweep (a future "cleanup" unifying the lists) and narrowing the canonical
//! set both go red.
//!
//! **WHAT THIS FILE OWNS AND WHAT THE TYPE SYSTEM ALREADY OWNS**, measured by falsifying each:
//! `SIDECAR_KINDS` is `[&str; 7]`, so DELETING a segment is a compile error (`E0308`) and needs no test.
//! RENAMING one is invisible to the type — the length still checks — and that is this census's half:
//! `"refuzed"` for `"refused"` compiles cleanly and is caught here. Stating the split because a census
//! that claims a guarantee the compiler is actually providing is a census nobody will maintain honestly.

use std::path::PathBuf;

fn crate_src(rel: &str) -> String {
    let p = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("..").join(rel);
    std::fs::read_to_string(&p).unwrap_or_else(|e| panic!("cannot read {}: {e}", p.display()))
}

/// Pull the names out of a `[...]`-style rust string-literal list following `anchor`.
fn names_after(src: &str, anchor: &str) -> Vec<String> {
    let i = src.find(anchor).unwrap_or_else(|| panic!("anchor {anchor:?} not found"));
    let rest = &src[i + anchor.len()..];
    let open = rest.find('[').expect("no `[` after the anchor");
    let close = rest[open..].find(']').expect("no `]` closing the list") + open;
    // `open + 1`: the slice must EXCLUDE the bracket, or the first element arrives as `["callgraph"`,
    // fails the quote-strip and is silently dropped. The vacuity floor below caught exactly that.
    rest[open + 1..close]
        .split(',')
        .filter_map(|t| {
            let t = t.trim();
            t.strip_prefix('"').and_then(|t| t.strip_suffix('"')).map(str::to_string)
        })
        .collect()
}

/// THE RESERVED SIDECAR SET HAS ONE OWNER, AND THE ONE PLACE THAT NARROWS IT SAYS SO BY NAME.
///
/// SPEC §2.2 fixes the family-wide set and records that it exists *"because the engines were already
/// drifting on it"* — three excluded by name with disagreeing lists, one discriminated by segment count.
/// This engine was the segment-count one.
///
/// The sweep in `candor-scan/src/gate.rs` used to be a hardcoded FIVE-name copy of the seven, and a copy
/// SHORTER than its source is unreadable: nothing distinguished "deliberately not swept" from
/// "forgotten", while the comment above it claimed the names came from §2.2's family-wide list.
#[test]
fn the_sidecar_sweep_is_a_named_subset_of_the_one_reserved_set() {
    let report_src = crate_src("candor-report/src/lib.rs");
    let gate_src = crate_src("candor-scan/src/gate.rs");

    let canonical = names_after(&report_src, "pub const SIDECAR_KINDS: [&str; 7] =");
    assert!(canonical.len() >= 7,
        "VACUITY FLOOR: located no reserved-segment set — this census is asserting about source it can \
         no longer find, and would go green through the very defect it exists to catch. Got {canonical:?}");
    for expected in ["callgraph", "hierarchy", "calibrated", "layerreach", "locs", "gate", "refused"] {
        assert!(canonical.iter().any(|c| c == expected),
            "SPEC §2.2 reserves `{expected}`; SIDECAR_KINDS no longer lists it: {canonical:?}");
    }

    // The sweep must DERIVE from the const, not restate it.
    assert!(gate_src.contains("for seg in candor_report::SIDECAR_KINDS"),
        "candor-scan's sidecar sweep must iterate `candor_report::SIDECAR_KINDS`, not a second list. \
         A copy drifts, and this one already had: it carried five of the seven names while its own \
         comment claimed to be §2.2's family-wide set.");

    // …and the two names it skips must be skipped BY NAME, each with its reason on file.
    for (seg, why) in [
        ("gate", "`<stem>.gate.json` is a VERDICT SINK by designation, not a sidecar of this report — \
                  sweeping it destroys a file that is not ours (candor-query's gate.rs excludes it for \
                  the same reason)"),
        ("refused", "the ⟨0.32⟩ refusal MARKER has its own lifecycle: a completing run removes it, and \
                     the rung guarantees a LOST marker fails OPEN while a STALE one fails CLOSED. \
                     Sweeping it here inverts exactly that"),
    ] {
        assert!(gate_src.contains(&format!("seg == \"{seg}\"")),
            "the sidecar sweep must skip `{seg}` BY NAME, so the exclusion is readable as deliberate \
             rather than achieved by omission. Reason it must stay excluded: {why}");
    }
}

/// THE ONE RULE MUST ACTUALLY BE ASKED. A const nothing consults is a const that has stopped being the
/// owner, and the copies that replaced it are invisible until two of them disagree.
#[test]
fn the_reserved_set_is_consulted_by_more_than_one_locator() {
    let report_src = crate_src("candor-report/src/lib.rs");
    let query_src = crate_src("candor-query/src/gate.rs");
    let gate_src = crate_src("candor-scan/src/gate.rs");

    let uses = report_src.matches("SIDECAR_KINDS").count()
        + query_src.matches("SIDECAR_KINDS").count()
        + gate_src.matches("SIDECAR_KINDS").count();
    assert!(uses >= 5,
        "the single reserved-segment rule must actually be CONSULTED across the engine — fewer than \
         five references means a locator has stopped asking it and is discriminating some other way, \
         which is the drift SPEC §2.2 was written to stop. Found {uses}");
}

/// SOUNDNESS R490 — THE §4 REASON VOCABULARY MUST NOT BE HELD TWICE IN THIS REPO.
///
/// `candor_classify::policy::ReasonClass::classify` has carried the note "THIS IS THE ONLY PLACE THIS
/// ENGINE HOLDS SPEC §4's KIND VOCABULARY" since ⟨0.24⟩. It was true of the READER and false of the
/// WRITERS: the nightly dylint lint at the repo root is a SECOND PRODUCER — it publishes into
/// `.candor/baseline.candor.Cdylib.json`, so its spellings reach consumers — and it had drifted in THREE
/// kinds outside §4's closed five (`generic-iter:`, `iter-combinator:`, `deref:`) and in two `dispatch:`
/// details that carried a RUST PATH where §4's detail is normatively `<owner-type>.<member>`.
///
/// **The row named two of those. A sweep of every reason the lint writes found four.** That is this
/// project's audit-boundary rule applied to the thing that was handed over, and it is why this census
/// reads the WHOLE file rather than the lines that were reported.
///
/// WHAT IT ASSERTS AND WHY THAT SHAPE: the lint must spell NO §4 reason itself. Every reason goes
/// through `Kind`, so `Kind` is the only place a kind token exists and the two halves cannot drift.
/// Asserting the absence of the raw literals is what makes a re-drift visible at compile-and-test time
/// rather than at the next corpus round — and it fails on the retired three by name, so they cannot come
/// back under their old spellings either.
#[test]
fn the_dylint_lint_spells_no_section_4_reason_of_its_own() {
    // The lint lives at the REPO root, not under `crates/` — `crate_src` resolves against `crates/`.
    let lint = crate_src("../src/lib.rs");
    // Strip line comments: this file's own prose NAMES the retired spellings, and a census that cannot
    // tell a comment from code would fail on the documentation of its own finding.
    let code: String = lint
        .lines()
        .map(|l| match l.find("//") {
            Some(i) => &l[..i],
            None => l,
        })
        .collect::<Vec<_>>()
        .join("\n");

    // §4 ⟨0.7⟩'s five, plus the three off-vocabulary kinds this row retired. A raw literal of ANY of
    // them in the lint's code means a reason was spelled locally instead of asked for.
    for kind in ["reflect", "native", "dispatch", "callback", "ambiguous",
                 "generic-iter", "iter-combinator", "deref"] {
        let needle = format!("\"{kind}:");
        assert!(!code.contains(&needle),
            "the dylint lint spells a §4 reason itself (`{needle}…`). Every reason it writes must come \
             from `candor_classify::policy::Kind`, or this repo holds SPEC §4's vocabulary in two places \
             again — which is SOUNDNESS R490, and the halves had already drifted in three kinds.");
        // …and the same literal built through `format!`, which is how two of the four got in.
        let fmt = format!("format!(\"{kind}:");
        assert!(!code.contains(&fmt),
            "the dylint lint FORMATS a §4 reason itself (`{fmt}…`) — same defect, one syntax over. Use \
             `Kind::{{reason,dispatch_on}}`.");
    }

    // VACUITY FLOOR. The assertions above are absences, and an absence passes over a file that moved,
    // emptied, or stopped producing reasons at all. So the lint must be REACHING the authority, and the
    // count must cover every reason site: eight at the time of writing (four `callback:`, one `native:`,
    // two `dispatch:`, one for the retired `deref:`), plus the two `dispatch_on` call sites.
    let asked = code.matches("policy::Kind::").count();
    assert!(asked >= 8,
        "the lint reaches `policy::Kind` only {asked} times — fewer than its reason sites, so the \
         absences above are passing because the reasons went somewhere this census cannot see, not \
         because they went through the authority.");
    assert!(code.contains("Kind::dispatch_on("),
        "no site forms §4's ONE normative detail (`dispatch:<owner>.<member>`). The lint used to emit \
         `dispatch:std::io::Write` — a rust path, dot-free — which §3.1's dispatch frontier keys on a \
         dot to read, so that reason was invisible to `possibleViaUnknownDispatch` entirely.");

    // The authority itself must hold the five ONCE, and `classify` must READ that table rather than
    // carry its own copy of the five strings — the exact JVM failure `classify`'s comment records.
    let pol = crate_src("candor-classify/src/policy.rs");
    assert!(pol.contains("Kind::ALL.into_iter().find(|k| w.starts_with(k.token()))"),
        "`ReasonClass::classify` no longer derives the canonical kinds from `Kind::ALL`. A second copy \
         of the five tokens is how one engine came to classify `ambiguous` two different ways.");
    assert_eq!(pol.matches("pub const ALL: [Kind; 5]").count(), 1,
        "SPEC §4 ⟨0.7⟩'s kind set is CLOSED at five and must be declared exactly once.");
}

/// SOUNDNESS R571 — THE MONOMORPHIZATION DENYLIST IS NAME-KEYED, SO EVERY WRITER OF `trait_vars` IS A
/// SITE THAT CAN GIVE A NAME AN ERASED MEANING AND MUST NOT LEAVE A STALE "caller-monomorphized" CLAIM
/// STANDING. A stale entry SUBTRACTS a real dispatch from the imported-trait CHA — the under-report
/// direction, and the one thing that fix was not allowed to buy.
///
/// The claim is that the writers live in exactly THREE regions, each of which clears the denylist:
/// `scoped_binding` (the one binder, for every block-scoped shadow), `visit_expr_closure` (a closure
/// parameter, saved and restored), and `visit_local` (which clears for every name its statement binds,
/// before any of its own binders runs). That was true when R571 landed and it is not enforced by
/// anything in the language — so this census pins the COUNT, which is what makes a NEW writer show up
/// as a decision rather than as a silent hole. The behavioural half is the three SHADOW controls in
/// `a_dyn_binding_elsewhere_in_the_body_does_not_cha_a_monomorphized_receiver_r571`; those catch a
/// missing clear in the three regions that exist, and this catches a FOURTH region being added.
///
/// The right response to a red here is to clear `mono_recv_traits` at the new site (or to confirm it is
/// inside one of the three) and then update the count — not to delete the assertion.
#[test]
fn every_trait_vars_writer_is_in_a_region_that_clears_the_r571_denylist() {
    let src = crate_src("candor-scan/src/collector.rs");
    // VACUITY FLOOR — the census must be able to find what it is asserting about. A renamed table or a
    // moved file would otherwise make this pass by counting nothing, which is precisely the shape
    // `check_soundness_tables.py` failed in (AGENT-CORPUS-BRIEF §1b).
    assert!(src.contains("mono_recv_traits"),
            "R571 CENSUS VACUOUS: `mono_recv_traits` is not in collector.rs at all, so every count \
             below is over a table that no longer exists under that name.");
    let writers = src.matches("self.trait_vars.insert(").count();
    assert_eq!(writers, 10,
               "a `trait_vars` writer was ADDED or REMOVED ({writers} found, 10 expected). Each one \
                binds a NAME to a dispatch meaning, and if that meaning is ERASED while a stale R571 \
                `mono_recv_traits` entry survives for the same name, the CHA is subtracted and a real \
                dispatch is lost with nothing disclosed. Confirm the new site is inside \
                `scoped_binding`, `visit_expr_closure` or `visit_local` — the three regions that clear \
                the denylist — or clear it there, then update this count.");
    // …and the three clears themselves, so removing one is red here as well as in the behavioural test.
    for (needle, region) in [
        ("let p_mono = self.mono_recv_traits.remove(name);", "scoped_binding"),
        ("let prev = self.mono_recv_traits.remove(&name);", "visit_expr_closure"),
        ("self.mono_recv_traits.remove(n);", "visit_local"),
    ] {
        assert!(src.contains(needle),
                "the R571 denylist is no longer cleared in `{region}`: a shadow there inherits the \
                 outer binding's caller-monomorphized claim and loses its own dispatch.");
    }
}
