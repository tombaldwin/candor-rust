//! Bet 4 spike — an exploratory MIR-based effect extractor (NON-PRODUCTION).
//!
//! Gated behind `CANDOR_MIR=1`; when set, this replaces the normal HIR analysis for the run and prints
//! a findings report to stderr. It exists to answer ONE question with evidence, not to ship: would
//! moving candor's core from HIR to MIR deliver "soundness by construction" — i.e. does MIR collapse
//! the per-syntax-form call handling (direct / closure / `dyn` / `Arc<dyn>` / operator) that has been
//! the recurring source of soundness holes into a single uniform case?
//!
//! The hypothesis: in MIR, EVERY call — however it was written — lowers to one `TerminatorKind::Call`
//! whose `func` operand is either a `FnDef` (resolved callee) or not (`fn` pointer / `dyn` dispatch ⇒
//! the honest `Unknown`). There is no closure node, no method-call node, no `Arc<dyn>` node to special-
//! case and therefore none to forget. This module tests that on the same fixtures the HIR engine needs
//! bespoke handling for, and reports what MIR additionally sees (implicit `Drop` calls) and what it
//! still cannot resolve (the same dynamic dispatch HIR can't), so the go/no-go is grounded in fact.

use rustc_middle::mir::{Operand, TerminatorKind};
use rustc_middle::ty::{Ty, TyCtxt, TyKind, TypingEnv};

use rustc_hir::def::DefKind;
use rustc_hir::def_id::LocalDefId;

/// One function's MIR-observed effect picture.
struct FnFacts {
    name: String,
    /// Resolved (FnDef) callee paths that the built-in classifier maps to an effect: (callee, effect).
    classified: Vec<(String, &'static str)>,
    /// `Call` terminators whose callee is NOT a `FnDef` — a fn pointer or `dyn`/closure dispatch. These
    /// are the honest `Unknown` of a MIR engine: it can't see the target, exactly like HIR can't.
    unresolved_calls: usize,
    /// Every callee path seen (resolved FnDefs), for characterising how each call FORM lowers.
    callee_paths: Vec<String>,
    /// `Drop` terminators on a type with a `Drop` impl — an IMPLICIT call site HIR never sees at all.
    drop_sites: usize,
    /// Total `Call` terminators, all forms (the uniform surface).
    total_calls: usize,
}

/// The callee `DefId` of a `Call`, if the `func` operand is a statically-known `FnDef`. A non-`FnDef`
/// operand (fn pointer, `dyn` method, closure via `Fn::call`) returns `None` — the unresolvable case.
fn call_target<'tcx>(tcx: TyCtxt<'tcx>, func: &Operand<'tcx>, fn_ty: Ty<'tcx>) -> Option<String> {
    let _ = func;
    match fn_ty.kind() {
        TyKind::FnDef(did, _) => Some(tcx.def_path_str(*did)),
        _ => None,
    }
}

fn facts_for(tcx: TyCtxt<'_>, did: LocalDefId) -> FnFacts {
    let body = tcx.optimized_mir(did);
    let mut classified = Vec::new();
    let mut unresolved_calls = 0;
    let mut drop_sites = 0;
    let mut total_calls = 0;
    let mut callee_paths = Vec::new();
    for bb in body.basic_blocks.iter() {
        let Some(term) = &bb.terminator else { continue };
        match &term.kind {
            TerminatorKind::Call { func, .. } => {
                total_calls += 1;
                let fn_ty = func.ty(&body.local_decls, tcx);
                match call_target(tcx, func, fn_ty) {
                    Some(path) => {
                        // Classify by the callee crate + path, reusing the production classifier so the
                        // comparison is apples-to-apples. crate_name of the callee's def id:
                        if let TyKind::FnDef(cdid, _) = fn_ty.kind() {
                            let crate_name = tcx.crate_name(cdid.krate);
                            if let Some(eff) = crate::classify(crate_name.as_str(), &path) {
                                classified.push((path.clone(), eff));
                            }
                        }
                        callee_paths.push(path);
                    }
                    None => {
                        unresolved_calls += 1;
                        callee_paths.push(format!("<{:?}>", fn_ty.kind()));
                    }
                }
            }
            // An implicit drop: scope exit runs `Drop::drop` if the place's type needs it. HIR has NO
            // node for this, so the current engine is structurally blind to an effectful Drop (a guard
            // that flushes/closes/logs on drop). MIR makes it an explicit terminator.
            TerminatorKind::Drop { place, .. } => {
                let ty = place.ty(&body.local_decls, tcx).ty;
                if ty.needs_drop(tcx, TypingEnv::post_analysis(tcx, did.to_def_id())) {
                    drop_sites += 1;
                }
            }
            _ => {}
        }
    }
    FnFacts {
        name: tcx.def_path_str(did.to_def_id()),
        classified,
        unresolved_calls,
        callee_paths,
        drop_sites,
        total_calls,
    }
}

/// A standard-library OWNING container — one that drops its element type(s) via heap-indirected drop
/// glue (so the element is hidden behind a raw pointer in its fields). Curated by name + crate (alloc /
/// std / core), so a user type that merely shares a name isn't matched. `Rc`/`Arc` only drop on the
/// last reference, but candor over-approximates (the drop CAN run), which is the sound direction.
fn is_std_owning_container(tcx: TyCtxt<'_>, did: rustc_hir::def_id::DefId) -> bool {
    if !matches!(tcx.crate_name(did.krate).as_str(), "alloc" | "std" | "core") {
        return false;
    }
    matches!(
        tcx.item_name(did).as_str(),
        "Box"
            | "Vec"
            | "VecDeque"
            | "Rc"
            | "Arc"
            | "BTreeMap"
            | "BTreeSet"
            | "HashMap"
            | "HashSet"
            | "LinkedList"
            | "BinaryHeap"
    )
}

/// Collect the LOCAL `Drop::drop` impls reachable when a value of `ty` is dropped: the type's own
/// destructor, plus (transitively) those of its fields / elements that run via drop glue. References
/// and raw pointers don't drop their pointee, so they're not followed. `seen` guards recursive types.
fn local_drop_impls<'tcx>(
    tcx: TyCtxt<'tcx>,
    ty: Ty<'tcx>,
    out: &mut std::collections::HashSet<LocalDefId>,
    seen: &mut std::collections::HashSet<rustc_hir::def_id::DefId>,
) {
    match ty.kind() {
        TyKind::Adt(adt, args) => {
            if !seen.insert(adt.did()) {
                return;
            }
            if let Some(dtor) = adt.destructor(tcx) {
                if let Some(l) = dtor.did.as_local() {
                    out.insert(l);
                }
            }
            for field in adt.all_fields() {
                local_drop_impls(tcx, field.ty(tcx, args), out, seen);
            }
            // A std OWNING container (Box/Vec/Rc/Arc/HashMap/…) holds its element behind a heap pointer,
            // so field-recursion stops at the raw pointer — yet dropping the container DOES drop the
            // element (Rc/Arc: when the last ref goes — conservatively assume it can). Follow the type
            // arguments for these curated types so a `Vec<Guard>` / `Box<Guard>` whose `Guard` has an
            // effectful Drop is still caught. `seen` bounds recursion on cyclic types.
            if is_std_owning_container(tcx, adt.did()) {
                for arg in args.iter() {
                    if let Some(t) = arg.as_type() {
                        local_drop_impls(tcx, t, out, seen);
                    }
                }
            }
        }
        TyKind::Tuple(tys) => {
            for t in tys.iter() {
                local_drop_impls(tcx, t, out, seen);
            }
        }
        TyKind::Array(t, _) | TyKind::Slice(t) => local_drop_impls(tcx, *t, out, seen),
        // Dropping a TRAIT OBJECT (`Box<dyn Trait>` etc.) runs the CONCRETE type's destructor through
        // the vtable — statically unknown. Sound over-approximation (the §4 trust contract): CHA the
        // impls of the object's principal trait and follow each self type's LOCAL Drop, so a local
        // effectful-Drop type behind a `Box<dyn Trait>` is caught exactly as behind a concrete
        // `Box<T>` (otherwise it was a silent under-report — pure despite an I/O-on-drop guard). This
        // matches candor only ever tracking LOCAL Drops: a std concrete type's Drop isn't followed,
        // dyn or not. No flood — most trait objects (`Box<dyn Error/Any/Fn…>`) have no local impl
        // carrying a Drop, so produce no edge.
        TyKind::Dynamic(preds, ..) => {
            if let Some(principal) = preds.principal_def_id() {
                let impls = tcx.trait_impls_of(principal);
                for impl_did in impls
                    .non_blanket_impls()
                    .values()
                    .flatten()
                    .chain(impls.blanket_impls())
                    .copied()
                {
                    let self_ty = tcx.type_of(impl_did).instantiate_identity();
                    local_drop_impls(tcx, self_ty, out, seen);
                }
            }
        }
        _ => {}
    }
}

/// Production use of MIR (narrow, by design): the `(caller, drop-impl)` edges implied by the `Drop`
/// terminators in every local function's MIR. HIR has no node for an implicit, scope-exit drop, so the
/// HIR engine was structurally blind to an effectful `Drop` (a guard that does I/O on the way out — a
/// §4 trust-contract hole). Folded into the call graph, the drop impl's effects propagate to the caller.
/// Returns edges; the caller inserts them into `calls`.
pub(crate) fn drop_edges(tcx: TyCtxt<'_>) -> Vec<(LocalDefId, LocalDefId)> {
    let mut edges = Vec::new();
    for did in tcx.hir_body_owners() {
        if !matches!(tcx.def_kind(did.to_def_id()), DefKind::Fn | DefKind::AssocFn)
            || !tcx.is_mir_available(did.to_def_id())
        {
            continue;
        }
        let body = tcx.optimized_mir(did);
        let mut impls = std::collections::HashSet::new();
        for bb in body.basic_blocks.iter() {
            let Some(term) = &bb.terminator else { continue };
            // The type whose Drop runs at this terminator: an IMPLICIT scope-exit `Drop`, or an EXPLICIT
            // `core::ptr::drop_in_place::<T>(p)` — the canonical way a hand-written container (smart
            // pointer / arena) drops its element through a raw pointer. The latter is just a call to a
            // non-local std fn (no effect, no edge), so without this its element's effectful Drop is
            // silently lost; recover `T` from the call's type arg and follow its local Drop identically.
            let drop_ty = match &term.kind {
                TerminatorKind::Drop { place, .. } => Some(place.ty(&body.local_decls, tcx).ty),
                TerminatorKind::Call { func, .. } => match func.ty(&body.local_decls, tcx).kind() {
                    TyKind::FnDef(d, args) if Some(*d) == tcx.lang_items().drop_in_place_fn() => {
                        args.types().next()
                    }
                    _ => None,
                },
                _ => None,
            };
            if let Some(ty) = drop_ty {
                let mut seen = std::collections::HashSet::new();
                local_drop_impls(tcx, ty, &mut impls, &mut seen);
            }
        }
        for impl_did in impls {
            if impl_did != did {
                edges.push((did, impl_did));
            }
        }
    }
    edges
}

/// Entry point: run the MIR spike over every local function and print a findings report.
pub fn run(tcx: TyCtxt<'_>) {
    let mut facts: Vec<FnFacts> = Vec::new();
    for did in tcx.hir_body_owners() {
        if !matches!(tcx.def_kind(did.to_def_id()), DefKind::Fn | DefKind::AssocFn) {
            continue;
        }
        // optimized_mir is unavailable for some bodies (const/static, foreign); guard with a probe.
        if !tcx.is_mir_available(did.to_def_id()) {
            continue;
        }
        facts.push(facts_for(tcx, did));
    }
    facts.sort_by(|a, b| a.name.cmp(&b.name));

    let total_calls: usize = facts.iter().map(|f| f.total_calls).sum();
    let unresolved: usize = facts.iter().map(|f| f.unresolved_calls).sum();
    let classified: usize = facts.iter().map(|f| f.classified.len()).sum();
    let drops: usize = facts.iter().map(|f| f.drop_sites).sum();

    eprintln!("=== CANDOR MIR SPIKE (Bet 4, non-production) ===");
    eprintln!(
        "functions: {}  |  Call terminators: {} (one uniform case for ALL call forms)  |  \
         classified-effect calls: {}  |  unresolved (fn-ptr/dyn ⇒ Unknown): {}  |  \
         implicit Drop sites (HIR-invisible): {}",
        facts.len(),
        total_calls,
        classified,
        unresolved,
        drops,
    );
    for f in &facts {
        if f.classified.is_empty() && f.unresolved_calls == 0 && f.drop_sites == 0 {
            continue;
        }
        let effs: Vec<String> =
            f.classified.iter().map(|(c, e)| format!("{e}<-{c}")).collect();
        eprintln!(
            "  {}: effects[{}] unresolved={} drops={}\n      callees: {}",
            f.name,
            effs.join(", "),
            f.unresolved_calls,
            f.drop_sites,
            f.callee_paths.join(" | "),
        );
    }
    eprintln!("=== end spike ===");
}
