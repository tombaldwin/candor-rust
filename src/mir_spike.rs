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
use rustc_middle::ty::{Ty, TyCtxt, TyKind, TypeVisitableExt, TypingEnv};

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
///
/// `PhantomData` is not a container; it is the LANGUAGE'S OWN declaration of this property. `PhantomData
/// <T>` in a struct's fields is precisely how a raw-pointer container tells drop-check "dropping me can
/// drop a `T`" — so following its argument is asking the authority rather than growing this name list
/// one victim at a time. Measured: it is what recovers `std::vec::IntoIter<Guard>` (a `for` loop left
/// early still drops the remaining elements), and it generalises to every hand-written arena / smart
/// pointer that marks ownership the documented way. It cannot over-charge on the variance-only spellings
/// — `PhantomData<&'a T>` / `PhantomData<fn() -> T>` present a reference / fn-pointer argument, and
/// neither is followed by the walker.
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
            | "PhantomData"
    )
}

/// Collect the LOCAL `Drop::drop` impls reachable when a value of `ty` is dropped: the type's own
/// destructor, plus (transitively) those of its fields / elements that run via drop glue. References
/// and raw pointers don't drop their pointee, so they're not followed. `seen` guards recursive types.
///
/// `seen` is keyed on the whole TYPE, not on the ADT's `DefId`. Keying it on the `DefId` made the walk
/// order-dependent and silently lossy: the FIRST instantiation of a generic ADT claimed the key, so
/// every LATER one — a different type argument, a different drop set — returned immediately. Because a
/// struct's fields are walked in DECLARATION ORDER, `struct S { a: Cell<u8>, b: Cell<Guard> }` lost
/// `Guard::drop` entirely while the same struct with its two fields SWAPPED was caught. That is not a
/// corner case: it is why `Mutex<Guard>`, `RwLock<Guard>` and `RefCell<Guard>` all read silent-pure —
/// each one reaches its payload through `UnsafeCell<T>`, and each one walks a *different* `UnsafeCell`
/// (inside a poison flag / borrow counter / platform lock) first.
pub(crate) fn local_drop_impls<'tcx>(
    tcx: TyCtxt<'tcx>,
    ty: Ty<'tcx>,
    out: &mut std::collections::HashSet<LocalDefId>,
    seen: &mut std::collections::HashSet<Ty<'tcx>>,
) {
    walk_drop_tys(tcx, ty, out, seen, 0);
}

/// How deep the drop walk may descend. Termination already comes from the type-keyed `seen` memo for
/// every type rustc can lay out; this is the backstop for a type whose ownership chain reaches ITSELF at
/// a DIFFERENT instantiation, where every level is a distinct `Ty` and `seen` therefore never repeats.
/// A bound, not an ancestor set: an ancestor set keyed on the ADT's `DefId` cuts `Vec<Vec<Guard>>`,
/// `Box<Box<Guard>>`, `Option<Option<Guard>>` and every user generic nested in itself — MEASURED, three
/// of those four went silent-pure under one — because a nested same-ADT type is its own "ancestor" while
/// being a perfectly ordinary, finite type. The bound is far past anything a real type reaches (the
/// deepest std container chain here is a handful of levels), so it never truncates real code.
const DROP_WALK_MAX_DEPTH: u32 = 128;

/// `local_drop_impls`'s body — see `DROP_WALK_MAX_DEPTH` for why the recursion bound is a depth and not
/// an ancestor set.
fn walk_drop_tys<'tcx>(
    tcx: TyCtxt<'tcx>,
    ty: Ty<'tcx>,
    out: &mut std::collections::HashSet<LocalDefId>,
    seen: &mut std::collections::HashSet<Ty<'tcx>>,
    depth: u32,
) {
    if depth > DROP_WALK_MAX_DEPTH {
        return;
    }
    if !seen.insert(ty) {
        return;
    }
    match ty.kind() {
        TyKind::Adt(adt, args) => {
            if let Some(dtor) = adt.destructor(tcx)
                && let Some(l) = dtor.did.as_local()
            {
                out.insert(l);
            }
            for field in adt.all_fields() {
                walk_drop_tys(tcx, field.ty(tcx, args).skip_normalization(), out, seen, depth + 1);
            }
            // A std OWNING container (Box/Vec/Rc/Arc/HashMap/…) holds its element behind a heap pointer,
            // so field-recursion stops at the raw pointer — yet dropping the container DOES drop the
            // element (Rc/Arc: when the last ref goes — conservatively assume it can). Follow the type
            // arguments for these curated types so a `Vec<Guard>` / `Box<Guard>` whose `Guard` has an
            // effectful Drop is still caught. `seen` bounds recursion on cyclic types.
            if is_std_owning_container(tcx, adt.did()) {
                for arg in args.iter() {
                    if let Some(t) = arg.as_type() {
                        walk_drop_tys(tcx, t, out, seen, depth + 1);
                    }
                }
            }
        }
        TyKind::Tuple(tys) => {
            for t in tys.iter() {
                walk_drop_tys(tcx, t, out, seen, depth + 1);
            }
        }
        TyKind::Array(t, _) | TyKind::Slice(t) => walk_drop_tys(tcx, *t, out, seen, depth + 1),
        // A CLOSURE/COROUTINE that captured an effectful-Drop value BY MOVE carries it as an upvar —
        // dropping the closure/coroutine (scope exit, `drop(c)`, or as the concrete type behind a
        // `Box<dyn Fn*>` the Dynamic arm below already resolves to here) drops every upvar in turn.
        // Closures/coroutines are NOT `TyKind::Adt` (rustc gives them their own `TyKind` variants), so
        // without this arm they fell into the `_ => {}` catch-all and an effectful guard captured by a
        // closure that is dropped WITHOUT ever being called was silently lost — the fn was reported
        // pure despite the guard's `Drop::drop` running. `CoroutineClosure` covers an `async || {}`
        // closure before it's called (which itself produces a `Coroutine` on call, covered by the
        // `Coroutine` arm). No `seen`-dedup key exists for these (no `DefId` naming a single ADT to
        // guard recursion on), but upvars are a fixed, finite field list, so plain recursion terminates.
        TyKind::Closure(_, args) => {
            for t in args.as_closure().upvar_tys() {
                walk_drop_tys(tcx, t, out, seen, depth + 1);
            }
        }
        TyKind::Coroutine(_, args) => {
            for t in args.as_coroutine().upvar_tys() {
                walk_drop_tys(tcx, t, out, seen, depth + 1);
            }
        }
        TyKind::CoroutineClosure(_, args) => {
            for t in args.as_coroutine_closure().upvar_tys() {
                walk_drop_tys(tcx, t, out, seen, depth + 1);
            }
        }
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
                    let self_ty = tcx.type_of(impl_did).instantiate_identity().skip_normalization();
                    walk_drop_tys(tcx, self_ty, out, seen, depth + 1);
                }
            }
        }
        _ => {}
    }
}

/// SOUNDNESS R1117 — a drop that runs INSIDE a generic body the caller instantiates.
///
/// `drop_edges` reads each LOCAL body's own `Drop` terminators, so it sees exactly the drops written in
/// this crate's MIR. But a caller that hands a concrete drop-relevant type to a GENERIC body — a std one
/// (`Vec::<H>::clear`, `Option::<u32>::ok_or::<H>`, `Result::<u32, H>::ok`, the `try_process` behind
/// `collect::<Result<Vec<H>, _>>`, the `fold` closure behind `count`) or a local one (`fn eat<T>(_: T)`
/// called with `H`) — runs that type's destructor in the CALLEE's body, where the drop is written over a
/// type PARAMETER. Read standalone, `T` has no destructor; read at the call site, it is `H`. Measured at
/// v0.40.9 on R1117's nine executed cells plus twelve more of the same shape (drop observed at run
/// time, row absent, `deny Fs <fn>` exit 0); `tests/integration.sh` 9c-vi pins a set of them.
///
/// This is not monomorphization of the whole crate. It is the one question `drop_edges` already asks —
/// "which local `Drop` impls run here?" — asked of a callee's body INSTANTIATED with the call site's own
/// generic args, which the MIR carries on every `Call` terminator. std's generic bodies ship their MIR
/// in metadata precisely because every downstream crate must instantiate them, so the body is there to
/// read; nothing about it is guessed or tabled.
///
/// Three bounds keep it the same question rather than a new analysis:
///   * DESCENT IS GATED ON THE ARGS: a callee is entered only if its generic args mention a type that
///     reaches a LOCAL `Drop` impl (`args_reach_local_drop`). A std body can only name a local destructor
///     through its type parameters, so a callee whose args reach none cannot contribute and is not read.
///     That prunes every call that does not carry the crate's own drop types.
///   * NORMAL PATH ONLY inside callees: cleanup (unwind) blocks are skipped. Panic-path drops inside a
///     callee were never charged before this; they stay so (a precision choice, not a new silence), and
///     reading them would charge every `Vec::push(h)` for the `h` an allocation panic would drop.
///   * Only drops whose type MENTIONS A PARAMETER of the callee are read. A drop over a concrete type in a
///     LOCAL callee is already that callee's own edge, inherited through the call edge.
///
/// A drop of an ADT whose own destructor is GENERIC over a drop-relevant arg — std's `Drain<'_, H>`,
/// whose `Drop::drop` drops the un-yielded elements, or a local `impl<T> Drop for RawBox<T>` — is
/// followed into that destructor's instantiated body the same way, at the root and inside callees:
/// reading what the destructor does, so `MutexGuard<H>` (which only unlocks) charges nothing. And a
/// drop that elaboration guarded by a switch on the value's discriminant is read per VARIANT
/// (`guarded_variant`), which is what keeps `Result::<H, E>::ok` uncharged.
pub(crate) struct InstDropWalk<'tcx> {
    tcx: TyCtxt<'tcx>,
    reach: std::collections::HashMap<Ty<'tcx>, bool>,
    body: std::collections::HashMap<
        (TypingEnv<'tcx>, rustc_middle::ty::Instance<'tcx>),
        std::rc::Rc<InstFacts<'tcx>>,
    >,
    closure: std::collections::HashMap<
        (TypingEnv<'tcx>, rustc_middle::ty::Instance<'tcx>),
        std::rc::Rc<std::collections::HashSet<LocalDefId>>,
    >,
    /// Roots whose walk hit `INST_WALK_BUDGET` (reported under `CANDOR_DROP_DEBUG`).
    pub(crate) truncated: usize,
}

/// One instance's own facts: the local drop impls its normal path runs, and the callees it hands
/// drop-relevant types to.
type InstFacts<'tcx> = (Vec<LocalDefId>, Vec<rustc_middle::ty::Instance<'tcx>>);

/// Instances one root may visit. A backstop, not a tuning knob: the args gate keeps real walks to tens of
/// instances; a root that exceeds it keeps what it found (never less than before this walk existed).
const INST_WALK_BUDGET: usize = 4096;

impl<'tcx> InstDropWalk<'tcx> {
    pub(crate) fn new(tcx: TyCtxt<'tcx>) -> Self {
        InstDropWalk {
            tcx,
            reach: Default::default(),
            body: Default::default(),
            closure: Default::default(),
            truncated: 0,
        }
    }

    /// Does dropping, or handing over, a value of `ty` reach a LOCAL `Drop` impl anywhere in the type —
    /// including behind references (a `&mut Vec<H>` receiver still lets `clear` drop an `H`)?
    fn ty_reaches(&mut self, ty: Ty<'tcx>) -> bool {
        if let Some(&b) = self.reach.get(&ty) {
            return b;
        }
        let mut b = false;
        for t in ty.walk().filter_map(|g| g.as_type()) {
            let mut out = std::collections::HashSet::new();
            let mut seen = std::collections::HashSet::new();
            local_drop_impls(self.tcx, t, &mut out, &mut seen);
            if !out.is_empty() {
                b = true;
                break;
            }
        }
        self.reach.insert(ty, b);
        b
    }

    fn args_reach_local_drop(&mut self, args: rustc_middle::ty::GenericArgsRef<'tcx>) -> bool {
        args.types().any(|t| self.ty_reaches(t))
    }

    /// The non-local destructors that run when a value of `ty` is dropped and whose type args reach a
    /// local `Drop` — each as the `Drop::drop` instance to read. Fields are followed so a local wrapper
    /// around a `Drain<'_, H>` is covered. A LOCAL destructor is `local_drop_impls`'s job, not this one's.
    fn nonlocal_dtors(
        &mut self,
        ty: Ty<'tcx>,
        out: &mut Vec<rustc_middle::ty::Instance<'tcx>>,
        seen: &mut std::collections::HashSet<Ty<'tcx>>,
        depth: u32,
    ) {
        if depth > DROP_WALK_MAX_DEPTH || !seen.insert(ty) {
            return;
        }
        match ty.kind() {
            TyKind::Adt(adt, args) => {
                if let Some(dtor) = adt.destructor(self.tcx)
                    && self.args_reach_local_drop(args)
                {
                    out.push(rustc_middle::ty::Instance::new_raw(dtor.did, args));
                }
                for field in adt.all_fields() {
                    let fty = field.ty(self.tcx, args).skip_normalization();
                    self.nonlocal_dtors(fty, out, seen, depth + 1);
                }
            }
            TyKind::Tuple(tys) => {
                for t in tys.iter() {
                    self.nonlocal_dtors(t, out, seen, depth + 1);
                }
            }
            TyKind::Array(t, _) | TyKind::Slice(t) => self.nonlocal_dtors(*t, out, seen, depth + 1),
            TyKind::Closure(_, args) => {
                for t in args.as_closure().upvar_tys() {
                    self.nonlocal_dtors(t, out, seen, depth + 1);
                }
            }
            _ => {}
        }
    }

    /// The MIR of an instance we are willing to read: a fn-like ITEM with MIR available (std generics
    /// and `#[inline]` items ship it; a non-generic, non-inline foreign fn does not and cannot name a
    /// local type anyway). Shims, intrinsics and virtual calls are not read.
    fn readable_body(&self, inst: rustc_middle::ty::Instance<'tcx>) -> Option<&'tcx rustc_middle::mir::Body<'tcx>> {
        let rustc_middle::ty::InstanceKind::Item(did) = inst.def else { return None };
        if !self.tcx.def_kind(did).is_fn_like() || !self.tcx.is_mir_available(did) {
            return None;
        }
        Some(self.tcx.instance_mir(inst.def))
    }

    /// Resolve a `FnDef` callee to the instance that runs. `try_resolve` asserts a Fn/AssocFn def; any
    /// failure (too generic, an error) is "not read" — the status quo before this walk.
    fn resolve(
        &self,
        env: TypingEnv<'tcx>,
        did: rustc_hir::def_id::DefId,
        args: rustc_middle::ty::GenericArgsRef<'tcx>,
    ) -> Option<rustc_middle::ty::Instance<'tcx>> {
        if !matches!(self.tcx.def_kind(did), DefKind::Fn | DefKind::AssocFn) {
            return None;
        }
        rustc_middle::ty::Instance::try_resolve(self.tcx, env, did, args).ok().flatten()
    }

    /// One instance's own contribution: the local drop impls its NORMAL-path body runs over a
    /// parameter-dependent type, and the callee instances (args gated) it hands those types to.
    fn body_facts(
        &mut self,
        env: TypingEnv<'tcx>,
        inst: rustc_middle::ty::Instance<'tcx>,
    ) -> std::rc::Rc<InstFacts<'tcx>> {
        if let Some(f) = self.body.get(&(env, inst)) {
            return f.clone();
        }
        let tcx = self.tcx;
        let mut drops = std::collections::HashSet::new();
        let mut next = Vec::new();
        if let Some(body) = self.readable_body(inst) {
            let inst_ty = |t: Ty<'tcx>| {
                inst.try_instantiate_mir_and_normalize_erasing_regions(
                    tcx,
                    env,
                    rustc_middle::ty::EarlyBinder::bind(t),
                )
                .ok()
            };
            for (bb_idx, bb) in body.basic_blocks.iter_enumerated() {
                if bb.is_cleanup {
                    continue;
                }
                let Some(term) = &bb.terminator else { continue };
                match &term.kind {
                    TerminatorKind::Drop { place, .. } => {
                        let raw = place.ty(&body.local_decls, tcx).ty;
                        if !raw.has_param() {
                            continue;
                        }
                        let Some(ty) = inst_ty(raw) else { continue };
                        // The value's drop-relevant parts: the whole type, or — when drop elaboration
                        // guarded this drop by a switch on the value's own discriminant — only the
                        // variant that switch selected. See `guarded_variant`.
                        let parts = match guarded_variant(tcx, body, bb_idx, place, ty) {
                            Some(fields) => fields,
                            None => vec![ty],
                        };
                        for part in parts {
                            let mut seen = std::collections::HashSet::new();
                            local_drop_impls(tcx, part, &mut drops, &mut seen);
                            let mut seen = std::collections::HashSet::new();
                            self.nonlocal_dtors(part, &mut next, &mut seen, 0);
                        }
                    }
                    TerminatorKind::Call { func, .. } => {
                        let raw = func.ty(&body.local_decls, tcx);
                        if !raw.has_param() {
                            continue;
                        }
                        let Some(fty) = inst_ty(raw) else { continue };
                        let TyKind::FnDef(d, a) = *fty.kind() else { continue };
                        if is_drop_glue_call(tcx, d) {
                            if let Some(t) = a.types().next() {
                                let mut seen = std::collections::HashSet::new();
                                local_drop_impls(tcx, t, &mut drops, &mut seen);
                                let mut seen = std::collections::HashSet::new();
                                self.nonlocal_dtors(t, &mut next, &mut seen, 0);
                            }
                            continue;
                        }
                        if !self.args_reach_local_drop(a) {
                            continue;
                        }
                        if let Some(callee) = self.resolve(env, d, a) {
                            next.push(callee);
                        }
                    }
                    _ => {}
                }
            }
        }
        let f = std::rc::Rc::new((drops.into_iter().collect(), next));
        self.body.insert((env, inst), f.clone());
        f
    }

    /// Every local drop impl that runs on the normal path of `root`, transitively through the args-gated
    /// callees. Exact for a completed walk, so it is memoised and reused when another root reaches it.
    pub(crate) fn reach(
        &mut self,
        env: TypingEnv<'tcx>,
        root: rustc_middle::ty::Instance<'tcx>,
    ) -> std::rc::Rc<std::collections::HashSet<LocalDefId>> {
        if let Some(c) = self.closure.get(&(env, root)) {
            return c.clone();
        }
        let mut out = std::collections::HashSet::new();
        let mut visited = std::collections::HashSet::new();
        let mut stack = vec![root];
        let mut truncated = false;
        while let Some(i) = stack.pop() {
            if !visited.insert(i) {
                continue;
            }
            if visited.len() > INST_WALK_BUDGET {
                truncated = true;
                break;
            }
            if let Some(c) = self.closure.get(&(env, i)) {
                out.extend(c.iter().copied());
                continue;
            }
            let f = self.body_facts(env, i);
            out.extend(f.0.iter().copied());
            stack.extend(f.1.iter().copied());
        }
        let c = std::rc::Rc::new(out);
        if truncated {
            self.truncated += 1;
        } else {
            self.closure.insert((env, root), c.clone());
        }
        c
    }

    /// The roots one LOCAL body contributes: every (args-gated) callee it calls, and every non-local
    /// destructor its own `Drop` terminators run. The environment is the body's own when its types still
    /// mention its parameters, else the fully monomorphic one.
    pub(crate) fn roots(
        &mut self,
        did: LocalDefId,
        body: &rustc_middle::mir::Body<'tcx>,
    ) -> Vec<(TypingEnv<'tcx>, rustc_middle::ty::Instance<'tcx>)> {
        let tcx = self.tcx;
        let mut out = Vec::new();
        for bb in body.basic_blocks.iter() {
            let Some(term) = &bb.terminator else { continue };
            match &term.kind {
                TerminatorKind::Call { func, .. } if !bb.is_cleanup => {
                    let TyKind::FnDef(d, a) = *func.ty(&body.local_decls, tcx).kind() else { continue };
                    if is_drop_glue_call(tcx, d) {
                        if let Some(t) = a.types().next() {
                            let env = env_for(tcx, did, t.has_param());
                            let mut v = Vec::new();
                            let mut seen = std::collections::HashSet::new();
                            self.nonlocal_dtors(t, &mut v, &mut seen, 0);
                            out.extend(v.into_iter().map(|i| (env, i)));
                        }
                        continue;
                    }
                    if !self.args_reach_local_drop(a) {
                        continue;
                    }
                    let env = env_for(tcx, did, a.has_param());
                    if let Some(i) = self.resolve(env, d, a) {
                        out.push((env, i));
                    }
                }
                TerminatorKind::Drop { place, .. } => {
                    let t = place.ty(&body.local_decls, tcx).ty;
                    let env = env_for(tcx, did, t.has_param());
                    let mut v = Vec::new();
                    let mut seen = std::collections::HashSet::new();
                    self.nonlocal_dtors(t, &mut v, &mut seen, 0);
                    out.extend(v.into_iter().map(|i| (env, i)));
                }
                _ => {}
            }
        }
        out
    }
}

/// Drop elaboration in a GENERIC body cannot drop "just the `Err` payload" by place when a sibling arm
/// moved the `Ok` payload out, so it emits the OPEN-DROP shape — `_k = discriminant(_1); switchInt(_k)
/// -> [0: skip, 1: drop(_1)]` — and drops the WHOLE value only on the arm where it is still whole.
/// Read naively, `drop(_1)` over `Result<H, u32>` charges `H::drop` in `Result::ok`, where the only
/// value it can ever drop is the `Err(u32)`: a fabrication the concrete-MIR path never makes, because
/// rustc elaborates a concrete body's drop by place. Measured on the corpus (lapin's
/// `try_recv().ok()`), and on the fixture `m_resokok`.
///
/// So when the drop's block has exactly ONE predecessor, that predecessor's terminator switches on
/// `discriminant(<the dropped local>)` computed IN THAT SAME BLOCK, and this block is the target of
/// exactly one discriminant value (never `otherwise`), the drop runs on that one variant: return its
/// fields' instantiated types. Anything else — a projection, an enum with its OWN destructor (which
/// runs on the whole value), a shared target, a reassignment between — returns `None` and the whole
/// type is charged, as before. It narrows only charges `InstDropWalk` itself adds; no pre-existing
/// edge is read through it.
fn guarded_variant<'tcx>(
    tcx: TyCtxt<'tcx>,
    body: &rustc_middle::mir::Body<'tcx>,
    bb: rustc_middle::mir::BasicBlock,
    place: &rustc_middle::mir::Place<'tcx>,
    ty: Ty<'tcx>,
) -> Option<Vec<Ty<'tcx>>> {
    use rustc_middle::mir::{Rvalue, StatementKind};
    if !place.projection.is_empty() {
        return None;
    }
    let TyKind::Adt(adt, args) = ty.kind() else { return None };
    if !adt.is_enum() || adt.destructor(tcx).is_some() {
        return None;
    }
    let preds = &body.basic_blocks.predecessors()[bb];
    let [pred] = preds.as_slice() else { return None };
    let pdata = &body.basic_blocks[*pred];
    let TerminatorKind::SwitchInt { discr, targets } = &pdata.terminator.as_ref()?.kind else {
        return None;
    };
    let switched = discr.place()?;
    if !switched.projection.is_empty() {
        return None;
    }
    // `_k = discriminant(<place>)` must be the LAST write to `_k` in the predecessor, and nothing
    // after it may write the dropped local.
    let mut reads_place = false;
    for st in &pdata.statements {
        if let StatementKind::Assign(b) = &st.kind {
            let (lhs, rv) = &**b;
            if lhs.local == switched.local {
                reads_place = lhs.projection.is_empty()
                    && matches!(rv, Rvalue::Discriminant(p) if p.local == place.local && p.projection.is_empty());
            } else if lhs.local == place.local {
                reads_place = false;
            }
        } else if let StatementKind::SetDiscriminant { place: p, .. } = &st.kind
            && p.local == place.local
        {
            reads_place = false;
        }
    }
    if !reads_place || targets.otherwise() == bb {
        return None;
    }
    let mut hit = targets.iter().filter(|(_, t)| *t == bb).map(|(v, _)| v);
    let value = hit.next()?;
    if hit.next().is_some() {
        return None;
    }
    let (vidx, _) = adt.discriminants(tcx).find(|(_, d)| d.val == value)?;
    Some(adt.variant(vidx).fields.iter().map(|f| f.ty(tcx, args).skip_normalization()).collect())
}

/// `core::ptr::drop_in_place::<T>` or the compiler's `core::ptr::drop_glue::<T>` lang item that std's
/// OPTIMIZED MIR calls in its place (the inliner replaces `drop_in_place` with `drop_glue` in every std
/// body read by `InstDropWalk` — `Vec::clear` is `drop_glue::<[T]>(ptr)`). Either runs `T`'s drop glue.
fn is_drop_glue_call(tcx: TyCtxt<'_>, d: rustc_hir::def_id::DefId) -> bool {
    (tcx.item_name(d).as_str() == "drop_in_place" && tcx.crate_name(d.krate).as_str() == "core")
        || tcx.lang_items().drop_glue_fn() == Some(d)
}

fn env_for<'tcx>(tcx: TyCtxt<'tcx>, did: LocalDefId, generic: bool) -> TypingEnv<'tcx> {
    if generic { TypingEnv::post_analysis(tcx, did.to_def_id()) } else { TypingEnv::fully_monomorphized() }
}

/// Production use of MIR (narrow, by design): the `(caller, drop-impl)` edges implied by the `Drop`
/// terminators in every local function's MIR. HIR has no node for an implicit, scope-exit drop, so the
/// HIR engine was structurally blind to an effectful `Drop` (a guard that does I/O on the way out — a
/// §4 trust-contract hole). Folded into the call graph, the drop impl's effects propagate to the caller.
/// Returns edges; the caller inserts them into `calls`.
pub(crate) fn drop_edges(tcx: TyCtxt<'_>) -> Vec<(LocalDefId, LocalDefId)> {
    let mut edges = Vec::new();
    let mut walk = InstDropWalk::new(tcx);
    for did in tcx.hir_body_owners() {
        // `is_fn_like()` is RUSTC'S OWN predicate for the bodies `optimized_mir` is defined on —
        // `Fn | AssocFn | Closure | SyntheticCoroutineBody` — and it is load-bearing, not tidiness.
        // `hir_body_owners()` also yields `Const`/`Static`/`AnonConst` bodies, `is_mir_available` says
        // TRUE for them, and `optimized_mir` then ICEs with *"do not use `optimized_mir` for
        // constants"* (those bodies are CTFE-only; `mir_for_ctfe` is their query). The previous
        // `DefKind::Fn | DefKind::AssocFn` filter avoided that by accident while ALSO excluding every
        // closure and coroutine body, which was SOUNDNESS R756 — so widening it to "every body owner"
        // traded a silent under-report for an ICE. Asking the compiler which bodies the query accepts
        // is the fix that is right for both reasons at once.
        if !tcx.def_kind(did.to_def_id()).is_fn_like() || !tcx.is_mir_available(did.to_def_id()) {
            continue;
        }
        // WHICH UNIT OWES THIS DROP. `hir_body_owners()` yields closure, `async` block and coroutine
        // bodies alongside `Fn`/`AssocFn`, and each of those has its OWN MIR — so a `let` inside a
        // closure body or an `async fn` produces its scope-exit `Drop` terminator in the CLOSURE's MIR,
        // not the enclosing function's. This used to filter to `DefKind::Fn | DefKind::AssocFn` and so
        // never looked at those bodies at all: SOUNDNESS R756, a silent under-report across the whole
        // closure/coroutine class, in the engine every soundness script drives as the ORACLE. Measured
        // on a one-line pair with observed drop counts: a closure body dropping an effectful guard read
        // 0 violations under `deny Fs <enclosing fn>` while the identical `drop(gg)` read 1.
        //
        // ADMITTING THE BODY IS ONLY HALF THE REPAIR, and the missing half is the caller. An edge keyed
        // on the CLOSURE's own `LocalDefId` charges a unit candor never reports as such, and nothing
        // edges the enclosing fn to its own closure — the HIR path charges a closure body's CALLS
        // straight to the enclosing item instead of edging to it — so the effect stops at the closure.
        // MEASURED on the same fixture, three engines, one policy each (`tests/integration.sh` 9c-v):
        //   HEAD            `deny Fs` lists only `Guard::drop`.
        //   edge on closure `deny Fs` lists `in_closure_body::{closure#0}` and `in_async_fn::{closure#0}`
        //                   — and NOT `main`, which calls `in_closure_body` and really does reach the
        //                   filesystem. The enclosing fn's own `inferred` stays empty and nothing
        //                   propagates past it, so every transitive caller reads pure.
        //   edge on owner   `deny Fs` lists `in_closure_body`, `in_async_fn`, `Guard::drop` AND `main`.
        // Note what the half-fix does NOT do, because the row predicted otherwise and the prediction is
        // worth correcting: a rule scoped by NAME PREFIX (`deny Fs in_closure_body`) still fires, on
        // `in_closure_body::{closure#0}`. The gate flip is therefore not the discriminator — the CALLER
        // is. So attribute the edge exactly where every other effect found in a closure body goes:
        // `enclosing_reportable_owner`, the SAME rule `enclosing_named_fn` applies on the HIR side.
        //
        // The attribution is an over-approximation in the same direction and by the same rule as the
        // rest of the engine: a closure that is never called, or one that escapes and is dropped
        // elsewhere, still charges the item whose body constructed it — which is precisely what already
        // happens to a `fs::read()` CALL written inside an uncalled closure. The alternative is silence,
        // and a caller that invokes an escaped closure it cannot see keeps its own honest `Unknown`.
        let Some(owner) = crate::enclosing_reportable_owner(tcx, did) else { continue };
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
                    // The sync `drop_in_place` lang item was removed upstream (async-drop landed);
                    // identify `core::ptr::drop_in_place` by name + defining crate instead.
                    TyKind::FnDef(d, args) if is_drop_glue_call(tcx, *d) =>
                    {
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
        // SOUNDNESS R1117 — the drops this body's CALLEES run over the concrete types it hands them, and
        // the drops a non-local destructor runs over this body's dropped values. See `InstDropWalk`.
        let mut inst_impls = std::collections::HashSet::new();
        for (env, root) in walk.roots(did, body) {
            for l in walk.reach(env, root).iter() {
                if !impls.contains(l) && inst_impls.insert(*l) && std::env::var_os("CANDOR_DROP_DEBUG").is_some() {
                    eprintln!(
                        "CANDOR_DROP_INST_EDGE {} <- {} (drop impl {})",
                        tcx.def_path_str(owner.to_def_id()),
                        tcx.def_path_str_with_args(root.def_id(), root.args),
                        tcx.def_path_str(l.to_def_id()),
                    );
                }
            }
        }
        impls.extend(inst_impls);
        for impl_did in impls {
            // Self-edge guard, against the ATTRIBUTION TARGET rather than the body owner: a `Drop::drop`
            // that lets a value of its own type fall out of scope (directly or inside a closure in its
            // body) would otherwise edge itself.
            if impl_did != owner {
                // REACH, on demand. `CANDOR_DROP_DEBUG=1` names every edge whose drop was found in a
                // body that is NOT the charged unit's own — a closure, an `async` block, a coroutine.
                // It exists because "CHANGED 0" over a corpus is not evidence a change is inert until
                // REACH is measured: the register has a row (R756's own vein) where 17,944 analysed
                // units read "inert" and a probe showed the code had never run. This is the probe, kept
                // rather than rebuilt, so the next A/B over this path can quote a reach count.
                if owner != did && std::env::var_os("CANDOR_DROP_DEBUG").is_some() {
                    eprintln!(
                        "CANDOR_DROP_BODY_EDGE {} <- body {} (drop impl {})",
                        tcx.def_path_str(owner.to_def_id()),
                        tcx.def_path_str(did.to_def_id()),
                        tcx.def_path_str(impl_did.to_def_id()),
                    );
                }
                edges.push((owner, impl_did));
            }
        }
    }
    if walk.truncated > 0 && std::env::var_os("CANDOR_DROP_DEBUG").is_some() {
        eprintln!("CANDOR_DROP_INST_TRUNCATED {}", walk.truncated);
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
