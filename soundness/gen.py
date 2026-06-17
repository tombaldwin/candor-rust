#!/usr/bin/env python3
"""Construction-based soundness fuzzer for candor (Bet 1, phase 1).

Generates a compilable Rust crate that threads a KNOWN effect (Fs/Net/Exec/Env) from a `sink` function
up through a random chain of functions, where each call edge uses a randomly-chosen call FORM — the
exact forms that have produced silent under-reports (direct call, inline closure, generic `impl Fn`,
a named fn handed to a combinator, a boxed `dyn Fn` value, `dyn`-method dispatch, and the RECEIVING
side: a function that takes a `Box<dyn Fn>` / `impl Fn` parameter and invokes it).

Every emitted function transitively reaches the effect, so candor must report each one with the effect
in its `inferred` set OR with `Unknown` (a sound over-approximation). A function reported PURE — or
omitted from the report entirely (candor omits effect-free fns) — is a SILENT UNDER-REPORT: the bug
class this harness exists to catch. The generator writes `truth.json` listing the functions that must
be effect-or-Unknown; `run.sh` checks the report against it.

Usage:  gen.py <seed> <out-dir>     # writes <out-dir>/{Cargo.toml, src/main.rs, truth.json}
"""
import json
import os
import random
import sys

# effect -> (the std leaf call that performs it, a distinctive runtime MARKER greppable in a syscall
# trace). The marker lets the dynamic oracle (phase 2) attribute the observed syscall to THIS effect
# and filter out the runtime's own startup syscalls (every binary opens libc, etc.). A `None` marker
# means the effect isn't syscall-observable (`Env` reads process memory — no syscall) so the oracle
# skips it. The leaf calls are chosen to emit their syscall even on failure (missing file, refused
# connection): `openat`, `connect(127.0.0.1)`, `execve(echo candor_fuzz_marker)`.
def effects_for(seed):
    return {
        "Fs":   ('let _ = std::fs::read_to_string("/tmp/candor_fuzz_%d");' % seed, "/tmp/candor_fuzz_%d" % seed),
        "Net":  ('let _ = std::net::TcpStream::connect("127.0.0.1:9");', "127.0.0.1"),
        "Exec": ('let _ = std::process::Command::new("echo").arg("candor_fuzz_marker").status();', "candor_fuzz_marker"),
        "Env":  ('let _ = std::env::var("CANDOR_FUZZ");', None),
    }

# Shared helpers, emitted once when any edge needs them. Keyed so we only emit each at most once.
HELPERS = {
    "apply":      "fn apply<F: Fn()>(f: F) { f() }",
    "run_trait":  "trait Run { fn run(&self); }\nstruct W<F>(F);\nimpl<F: Fn()> Run for W<F> { fn run(&self) { (self.0)(); } }",
    "recv_boxed": "fn recv_boxed(cb: Box<dyn Fn()>) { cb(); }",          # receiving side: boxed dyn Fn param
    "recv_impl":  "fn recv_impl<G: Fn()>(cb: G) { cb(); }",              # receiving side: generic Fn param
    "mcall":      "macro_rules! mcall { ($f:expr) => { $f() }; }",        # the call lives in a macro expansion
    # arbitrary self type: dispatch on `Arc<dyn Trait>` via `self: Arc<Self>` (the is_dyn_receiver case).
    # Fully-qualified `std::sync::Arc` (no `use`) so it never clashes with another helper/crate import.
    "arc_run":    "trait ARun { fn arun(self: std::sync::Arc<Self>); }\nstruct AW<F>(F);\nimpl<F: Fn()> ARun for AW<F> { fn arun(self: std::sync::Arc<Self>) { (self.0)(); } }",
}

# Each "edge form" returns (body_calling_callee, helpers_needed, extra_expected_fns, extra_items).
# `extra_expected_fns` are helper fns that ALSO transitively reach the effect via this edge and so must
# themselves be effect-or-Unknown (the receiving-side checks — where the real bugs live). `extra_items`
# are full module-level items emitted verbatim (used by the operator-overload forms, which need a
# PER-EDGE struct + effectful trait impl naming the specific callee — they can't be shared HELPERS).
def edge_forms(callee, i=0):
    return {
        "direct":     (f"{callee}();", [], [], []),
        "iife":       (f"(|| {callee}())();", [], [], []),
        "stored":     (f"{{ let c = || {callee}(); c(); }}", [], [], []),
        # named fn handed to a generic combinator (passing side) + apply itself invokes its param.
        "generic":    (f"apply({callee});", ["apply"], ["apply"], []),
        # named fn boxed as a dyn Fn value, then called.
        "boxed_val":  (f"{{ let b: Box<dyn Fn()> = Box::new({callee}); b(); }}", [], [], []),
        # named fn in a generic struct dispatched via &dyn Trait (dyn dispatch + a field closure call).
        "dyn_method": (f"{{ let w = W({callee}); let r: &dyn Run = &w; r.run(); }}", ["run_trait"], [], []),
        # RECEIVING side: the effect reaches a fn ONLY through a Box<dyn Fn> param it invokes.
        "recv_boxed": (f"recv_boxed(Box::new(|| {callee}()));", ["recv_boxed"], ["recv_boxed"], []),
        # RECEIVING side: the effect reaches a fn ONLY through a generic Fn param it invokes.
        "recv_impl":  (f"recv_impl(|| {callee}());", ["recv_impl"], ["recv_impl"], []),
        # the call lives inside a MACRO expansion (from_expansion span) — candor must still see it.
        "macro_call": (f"mcall!({callee});", ["mcall"], [], []),
        # ARBITRARY SELF TYPE: dispatch through `Arc<dyn ARun>` whose method takes `self: Arc<Self>`
        # (peel_refs doesn't see the `dyn` behind the `Arc` — the is_dyn_receiver path).
        "arc_dyn":    (f"{{ let a: std::sync::Arc<dyn ARun> = std::sync::Arc::new(AW({callee})); a.arun(); }}", ["arc_run"], [], []),
        # UFCS DYNAMIC DISPATCH: `Trait::method(obj)` on a `&dyn Trait` is a `Call` (not a MethodCall), so
        # `is_dyn_receiver` never runs and `dynamic` starts false. Resolving the trait method on a `dyn`
        # Self yields a VIRTUAL instance whose `def_id()` is the bodyless trait method — devirtualize must
        # report it as still-virtual (→ CHA the local impls) instead of edging to that bodyless method,
        # or the caller looks pure. Teeth: soundness/gen.py `ufcs_dyn` form.
        "ufcs_dyn":   (
            f"{{ let o: &dyn Jd{i:02d} = &Sd{i:02d}; Jd{i:02d}::run(o); }}",
            [], [],
            [
                f"trait Jd{i:02d} {{ fn run(&self); }}",
                f"struct Sd{i:02d};",
                f"impl Jd{i:02d} for Sd{i:02d} {{ fn run(&self) {{ {callee}(); }} }}",
            ],
        ),
        # OPERATOR OVERLOAD: the effect is reached through an overloaded `+` whose `Add::add` impl calls
        # the next fn. In HIR this is `ExprKind::Binary`, NOT a Call/MethodCall — so resolve_callee must
        # query type_dependent_def_id on the operator node or the edge is invisible (silent-pure hole).
        "op_add":     (f"{{ let _ = Op{i:02d} + Op{i:02d}; }}", [], [], [
            f"struct Op{i:02d};\nimpl std::ops::Add for Op{i:02d} {{ type Output = (); fn add(self, _: Self) {{ {callee}(); }} }}"]),
        # COMPARISON `==`: `a == b` is `ExprKind::Binary(Eq)` but — unlike `+` — records NO
        # type_dependent_def_id, so the normal operator path can't see the local `PartialEq::eq`. candor
        # needs a dedicated comparison resolver (resolve_cmp_op) keyed on the operand type, or an effectful
        # `eq` reached through `==` is silent-pure. By VALUE (`Eq(0) == Eq(0)`) so it can't fall through to
        # the std blanket `&T: PartialEq` impl (which DOES carry a tddi → std-pure → fix never engages).
        "eq_op":      (f"{{ let _ = Eq{i:02d}(0) == Eq{i:02d}(0); }}", [], [], [
            f"struct Eq{i:02d}(i32);\nimpl PartialEq for Eq{i:02d} {{ fn eq(&self, _o: &Self) -> bool {{ {callee}(); true }} }}"]),
        # COMPARISON `<`: `a < b` records a tddi — but it points at the non-local DEFAULT `PartialOrd::lt`,
        # which HIDES the local `partial_cmp` the operator actually dispatches to. The normal path resolves
        # the default (std, pure) and misses the effectful local `partial_cmp` → silent-pure. resolve_cmp_op
        # must resolve `partial_cmp` directly. The `eq` impl here is pure, isolating the PartialOrd path.
        "cmp_op":     (f"{{ let _ = Cmp{i:02d}(0) < Cmp{i:02d}(0); }}", [], [], [
            f"struct Cmp{i:02d}(i32);\nimpl PartialEq for Cmp{i:02d} {{ fn eq(&self, _o: &Self) -> bool {{ true }} }}\nimpl PartialOrd for Cmp{i:02d} {{ fn partial_cmp(&self, _o: &Self) -> Option<std::cmp::Ordering> {{ {callee}(); Some(std::cmp::Ordering::Equal) }} }}"]),
        # RETURN-TYPE-DIRECTED std drivers: a std method (`collect`/`into`/`parse`) selects a LOCAL trait
        # impl by the call's RESULT type and runs it inside its non-local body — invisible like the `?`-From
        # edge, so an effectful FromIterator/From/FromStr impl reached this way was silent-pure. The
        # receiver-directed iter-combinator bridge only peels the RECEIVER, so it's blind to these.
        "into_from":  (f"{{ let _: Wf{i:02d} = 5i32.into(); }}", [], [], [
            f"struct Wf{i:02d}(i32);\nimpl From<i32> for Wf{i:02d} {{ fn from(_v: i32) -> Self {{ {callee}(); Wf{i:02d}(0) }} }}"]),
        "collect_fromiter": (f"{{ let _: Cl{i:02d} = (0..2).collect(); }}", [], [], [
            f"struct Cl{i:02d};\nimpl FromIterator<i32> for Cl{i:02d} {{ fn from_iter<I: IntoIterator<Item=i32>>(_i: I) -> Self {{ {callee}(); Cl{i:02d} }} }}"]),
        "parse_fromstr": (f"{{ let _ = \"x\".parse::<Pf{i:02d}>(); }}", [], [], [
            f"struct Pf{i:02d};\nimpl std::str::FromStr for Pf{i:02d} {{ type Err = (); fn from_str(_s: &str) -> Result<Self, ()> {{ {callee}(); Ok(Pf{i:02d}) }} }}"]),
        # EXPLICIT mem::drop: `drop(v)` relocates v's destructor into the non-local mem::drop body, so an
        # effectful local Drop impl reached via explicit early-release was silent-pure (scope-end glue is
        # modeled, but the move into mem::drop is not). candor must resolve <T as Drop>::drop for the arg.
        "mem_drop": (f"{{ let v = Dp{i:02d}(()); drop(v); }}", [], [], [
            f"struct Dp{i:02d}(());\nimpl Drop for Dp{i:02d} {{ fn drop(&mut self) {{ {callee}(); }} }}"]),
        # OVERLOADED INDEX: `v[0]` is `ExprKind::Index` → `Index::index`; same root cause as op_add.
        "index":      (f"{{ let v = Ix{i:02d}(()); let _ = v[0]; }}", [], [], [
            f"struct Ix{i:02d}(());\nimpl std::ops::Index<usize> for Ix{i:02d} {{ type Output = (); fn index(&self, _: usize) -> &() {{ {callee}(); &self.0 }} }}"]),
        # OVERLOADED DEREF: `*v` is `ExprKind::Unary(Deref)` → `Deref::deref`; same root cause.
        "deref":      (f"{{ let v = Dr{i:02d}(()); let _ = *v; }}", [], [], [
            f"struct Dr{i:02d}(());\nimpl std::ops::Deref for Dr{i:02d} {{ type Target = (); fn deref(&self) -> &() {{ {callee}(); &self.0 }} }}"]),
        # IMPLICIT AUTO-DEREF: a method call on the wrapper (`w.tgt()`) auto-derefs `Aw -> AwI`, inserting
        # an IMPLICIT `<Aw as Deref>::deref` as an `Adjust::Deref(Overloaded(..))` EXPRESSION ADJUSTMENT —
        # NOT a `Call`/`MethodCall`/`Unary(Deref)` HIR node. The effectful local `deref` impl reached only
        # this way was reported neither with its effect nor `Unknown`: silently pure (the smart-pointer
        # hole). candor must walk `expr_adjustments` and edge to the overloaded deref. Teeth for the
        # implicit-Deref fix.
        "autoderef":  (f"{{ let w = Aw{i:02d}(AwI{i:02d}); w.tgt(); }}", [], [], [
            f"struct AwI{i:02d}; impl AwI{i:02d} {{ fn tgt(&self) {{}} }}",
            f"struct Aw{i:02d}(AwI{i:02d});\nimpl std::ops::Deref for Aw{i:02d} {{ type Target = AwI{i:02d}; fn deref(&self) -> &AwI{i:02d} {{ {callee}(); &self.0 }} }}"]),
        # DEREF-COERCION AT A CALL SITE: passing `&Cw` where `&CwI` is expected coerces via an IMPLICIT
        # `<Cw as Deref>::deref` adjustment at the ARG expression — again no HIR call node. Same hole as
        # `autoderef` but reached through coercion rather than method auto-deref.
        "deref_coerce": (f"{{ let w = Cw{i:02d}(CwI{i:02d}); cwtake{i:02d}(&w); }}", [], [], [
            f"struct CwI{i:02d}; fn cwtake{i:02d}(_: &CwI{i:02d}) {{}}",
            f"struct Cw{i:02d}(CwI{i:02d});\nimpl std::ops::Deref for Cw{i:02d} {{ type Target = CwI{i:02d}; fn deref(&self) -> &CwI{i:02d} {{ {callee}(); &self.0 }} }}"]),
        # `?` ERROR CONVERSION: the effect is reached through a custom `From<Ea> for Eb` impl invoked by
        # the `?` desugar's error path. candor sees the std `FromResidual::from_residual` call but not the
        # LOCAL `From::from` it dispatches to — so the edge must be recovered from the residual/Self types
        # (from_residual_local_edge). `help` always Errs, so the conversion (and the effect) also runs at
        # runtime, keeping the oracle honest.
        "try_from":   (
            f"{{ let _: Result<(), Eb{i:02d}> = (|| -> Result<(), Eb{i:02d}> {{ help{i:02d}()?; Ok(()) }})(); }}",
            [], [],
            [
                f"struct Ea{i:02d}; struct Eb{i:02d};",
                f"impl From<Ea{i:02d}> for Eb{i:02d} {{ fn from(_: Ea{i:02d}) -> Eb{i:02d} {{ {callee}(); Eb{i:02d} }} }}",
                f"fn help{i:02d}() -> Result<(), Ea{i:02d}> {{ Err(Ea{i:02d}) }}",
            ],
        ),
        # `.await` over a CUSTOM Future whose `poll` performs the I/O. `Fut.await` desugars to a
        # (compiler-generated) `Future::poll(..)` Call — a non-local trait method dispatched statically to
        # the LOCAL impl, which candor must devirtualize through the Call or the caller looks pure. The
        # awaited future is created-but-not-driven (no executor), so this NEVER RUNS at runtime — it's a
        # CONSTRUCTION-only form (excluded from DEFAULT_FORMS so it can't make the strace oracle vacuous).
        "await_poll": (
            f"{{ let _ = aw{i:02d}(); }}",
            [], [],
            [
                f"struct Fut{i:02d}(());",
                f"impl std::future::Future for Fut{i:02d} {{ type Output = (); "
                f"fn poll(self: std::pin::Pin<&mut Self>, _cx: &mut std::task::Context<'_>) "
                f"-> std::task::Poll<()> {{ {callee}(); std::task::Poll::Ready(()) }} }}",
                f"async fn aw{i:02d}() {{ Fut{i:02d}(()).await; }}",
            ],
        ),
        # THREAD SPAWN: the effect is inside a closure handed to `std::thread::spawn` — the runtime
        # invokes it on another thread. SEMANTICS §2 attributes a closure's call sites to the nearest
        # enclosing fn, so the SPAWNING fn must inherit (the JVM twin of this — an anonymous Runnable
        # handed to Thread — was a real under-report, candor-java bug fixed 2026-06-10). `.join()` so
        # the effect runs before exit (keeps the strace oracle honest).
        "spawn":      (f"{{ let h = std::thread::spawn(|| {callee}()); let _ = h.join(); }}", [], [], []),
        # RAII DROP: the effect lives in `Drop::drop`, run at SCOPE END — there is NO call expression
        # in HIR (drop elaboration happens in MIR), so a HIR-walking lint sees nothing unless it
        # special-cases locals whose type has a local effectful Drop impl.
        "drop":       (f"{{ let _g = Dp{i:02d}; }}", [], [], [
            f"struct Dp{i:02d};\nimpl Drop for Dp{i:02d} {{ fn drop(&mut self) {{ {callee}(); }} }}"]),
        # FOR-LOOP over a custom Iterator: the effect is in `next()`, reached via the loop desugar
        # (`IntoIterator::into_iter` + `Iterator::next` calls candor must resolve to the local impl).
        "iterator":   (f"for _ in It{i:02d}(true) {{}}", [], [], [
            f"struct It{i:02d}(bool);\nimpl Iterator for It{i:02d} {{ type Item = (); "
            f"fn next(&mut self) -> Option<()> {{ if self.0 {{ self.0 = false; {callee}(); Some(()) }} else {{ None }} }} }}"]),
        # OPAQUE RETURN (`impl Trait`), concrete hidden type: the effect is in a custom Iterator's
        # `next()`, returned as `impl Iterator` and consumed via `.next()` ON THE OPAQUE. The analysis
        # typing env can't resolve a trait method on an opaque Self (`Ok(None)`), so this fell into the
        # unresolvable-generic-stays-pure calibration: no edge, no `Unknown` — bug #33, found dogfooding
        # the `which` crate (`which_all(..).and_then(|mut i| i.next())`). devirtualize must RETRY with
        # opaques revealed (post-analysis env) to pin the concrete local impl. (`mk` only CONSTRUCTS the
        # iterator — it's correctly pure, so it is NOT in the expected set.)
        "opaque_iter": (f"{{ let mut it = mk{i:02d}(); let _ = it.next(); }}", [], [], [
            f"struct Oi{i:02d}(bool);\nimpl Iterator for Oi{i:02d} {{ type Item = (); "
            f"fn next(&mut self) -> Option<()> {{ if self.0 {{ self.0 = false; {callee}(); Some(()) }} else {{ None }} }} }}",
            f"fn mk{i:02d}() -> impl Iterator<Item = ()> {{ Oi{i:02d}(true) }}"]),
        # OPAQUE RETURN hiding a `Box<dyn …>`: same call shape, but the hidden type is a trait object —
        # is_dyn_receiver must REVEAL the local opaque and walk through the Box to see the `dyn`, or the
        # call is neither edged nor `Unknown` (the other half of bug #33).
        "opaque_dyn": (f"{{ let mut it = mkd{i:02d}(); let _ = it.next(); }}", [], [], [
            f"struct Od{i:02d}(bool);\nimpl Iterator for Od{i:02d} {{ type Item = (); "
            f"fn next(&mut self) -> Option<()> {{ if self.0 {{ self.0 = false; {callee}(); Some(()) }} else {{ None }} }} }}",
            f"fn mkd{i:02d}() -> impl Iterator<Item = ()> {{ Box::new(Od{i:02d}(true)) as Box<dyn Iterator<Item = ()>> }}"]),
        # OVERLOADED `==`: `ExprKind::Binary(Eq)` → `PartialEq::eq` — same operator-node family as
        # op_add but a different binop, locked separately.
        "eq":         (f"{{ let _ = Qe{i:02d} == Qe{i:02d}; }}", [], [], [
            f"struct Qe{i:02d};\nimpl PartialEq for Qe{i:02d} {{ fn eq(&self, _: &Self) -> bool {{ {callee}(); true }} }}"]),
        # COMPOUND ASSIGN `+=`: `ExprKind::AssignOp` → `AddAssign::add_assign` — a DIFFERENT HIR node
        # than Binary (op_add), so it needs its own operator-node handling or the edge is invisible.
        "add_assign": (f"{{ let mut a{i:02d} = As{i:02d}; a{i:02d} += As{i:02d}; }}", [], [], [
            f"struct As{i:02d};\nimpl std::ops::AddAssign for As{i:02d} {{ fn add_assign(&mut self, _: Self) {{ {callee}(); }} }}"]),
        # STD ITERATOR COMBINATOR / consumer driving a LOCAL `Iterator::next`: the effect is in a custom
        # iterator's `next()`, consumed NOT by a bare `for`/`while let` (those desugar `next()` into the
        # consumer's own HIR and are already sound) but through a std COMBINATOR — `.for_each(..)`,
        # `.map(..).collect()`, `.sum()`, and a `for x in it.map(..)` over an ADAPTED iterator. candor
        # resolves the outer call to the std method (pure for itself) and can't follow std's hidden
        # `next()` callback to the local impl — so it must recover the edge to that local `next()`
        # (HOLE 1) or the consumer looks silently pure. The `i`-derived rotation exercises all four
        # consumer shapes across a chain. (The struct's `next()` performs the effect.)
        "iter_combinator": (
            [
                f"Ic{i:02d}(2).for_each(|_| {{}});",                          # combinator/consumer: for_each
                f"{{ let _: Vec<u64> = Ic{i:02d}(2).map(|x| x + 1).collect(); }}",  # map + collect
                f"{{ let _: u64 = Ic{i:02d}(2).sum(); }}",                    # sum (Sum consumer)
                f"for _ in Ic{i:02d}(2).map(|x| x + 1) {{}}",                # for over an ADAPTED iter
            ][i % 4],
            [], [],
            [
                f"struct Ic{i:02d}(u8);\nimpl Iterator for Ic{i:02d} {{ type Item = u64; "
                f"fn next(&mut self) -> Option<u64> {{ if self.0 == 0 {{ None }} else {{ self.0 -= 1; {callee}(); Some(1) }} }} }}"
            ],
        ),
        # GENERIC-PARAM RECEIVER driving a LOCAL `Iterator::next`: a generic consumer
        # `fn gicons<I: Iterator>(it: I) { it.for_each(..) }` is called at a CONCRETE site with a custom
        # iterator (`Gi{i}`) whose `next()` performs the effect. INSIDE the consumer `I` is an unresolved
        # `Param`, so candor reports it pure for itself (the silent-pure generic-iterator hole) AND loses
        # the effect at the call site too. The fix recovers the call-site substs (`I=Gi{i}`) and resolves
        # the consumer's internal `<I as Iterator>::next` to the LOCAL `Gi{i}::next` — so the CALLING fn
        # (`f{i}`) gets the PRECISE effect, while the generic consumer carries a report-only honest
        # `Unknown` (`generic-iter:<method>`). The `i`-derived rotation exercises for_each / map+collect /
        # sum / a `for` over a generic-param `Map` adapter. Both the caller AND the consumer must be
        # effect-or-Unknown, never pure. (`Gi{i}::next` performs the effect.) Teeth for the generic fix.
        "generic_iter": (
            f"gicons{i:02d}(Gi{i:02d}(2));",
            [], [f"gicons{i:02d}"],
            [
                f"struct Gi{i:02d}(u8);\nimpl Iterator for Gi{i:02d} {{ type Item = u64; "
                f"fn next(&mut self) -> Option<u64> {{ if self.0 == 0 {{ None }} else {{ self.0 -= 1; {callee}(); Some(1) }} }} }}",
                [
                    f"fn gicons{i:02d}<I: Iterator>(it: I) {{ it.for_each(|_| {{}}); }}",
                    f"fn gicons{i:02d}<I: Iterator<Item = u64>>(it: I) {{ let _: Vec<u64> = it.collect(); }}",
                    f"fn gicons{i:02d}<I: Iterator<Item = u64>>(it: I) {{ let _: u64 = it.sum(); }}",
                    f"fn gicons{i:02d}<I: Iterator<Item = u64>>(it: I) {{ for _ in it.map(|x| x + 1) {{}} }}",
                ][i % 4],
            ],
        ),
        # LOCAL `Display::fmt` reached via a `core::fmt` formatting macro: the effect is in an
        # `impl Display for T` whose `fmt()` does I/O; `format!("{}", t)` / `println!("{}", t)` reach it
        # through core::fmt's machinery — candor sees the std `Argument::new_display` / `write_fmt`, never
        # the local `fmt` call, so the caller looks silently pure (HOLE 2). The `i % 2` rotation covers
        # both `format!` and `println!`. (`fmt` performs the effect, then writes a byte so it type-checks.)
        "display_fmt": (
            [
                f'{{ let _ = format!("{{}}", Df{i:02d}); }}',
                f'println!("{{}}", Df{i:02d});',
            ][i % 2],
            [], [],
            [
                f"struct Df{i:02d};\nimpl std::fmt::Display for Df{i:02d} {{ "
                f"fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {{ {callee}(); write!(f, \"x\") }} }}"
            ],
        ),
        # `x.to_string()` drives `<X as Display>::fmt` INSIDE the std blanket `ToString` impl — candor sees
        # only the non-local `ToString::to_string`, never the local `fmt` (sweep [25]). The std blanket is
        # in `is_pure_std_trait`, so without the driver recovery the caller looked silently pure.
        "to_string_fmt": (
            f'{{ let _ = Ts{i:02d}.to_string(); }}',
            [], [],
            [
                f"struct Ts{i:02d};\nimpl std::fmt::Display for Ts{i:02d} {{ "
                f"fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {{ {callee}(); write!(f, \"x\") }} }}"
            ],
        ),
        # `v.contains(x)` drives `<E as PartialEq>::eq` inside the std `<[E]>::contains` (a non-local generic
        # driver bounded by `T: PartialEq`) — candor sees only `contains`, never the local `eq` (sweep [26]).
        "vec_contains_eq": (
            f'{{ let v = vec![Eq{i:02d}(0)]; let _ = v.contains(&Eq{i:02d}(1)); }}',
            [], [],
            [
                f"struct Eq{i:02d}(u32);\nimpl PartialEq for Eq{i:02d} {{ "
                f"fn eq(&self, o: &Self) -> bool {{ {callee}(); self.0 == o.0 }} }}"
            ],
        ),
        # `v.clone()` drives `<E as Clone>::clone` element-wise inside the std `Vec::clone` (sweep [26]).
        "vec_clone": (
            f'{{ let v = vec![Cl{i:02d}(0)]; let _ = v.clone(); }}',
            [], [],
            [
                f"struct Cl{i:02d}(u32);\nimpl Clone for Cl{i:02d} {{ "
                f"fn clone(&self) -> Self {{ {callee}(); Cl{i:02d}(self.0) }} }}"
            ],
        ),
    }


# Forms that DON'T execute at runtime (the awaited future is never driven) — fine for the construction
# checker (candor's static report), but they'd make the dynamic strace oracle vacuous, so they're kept
# OUT of the default form set and only reachable via an explicit CANDOR_FUZZ_FORMS lane.
CONSTRUCTION_ONLY = {"await_poll"}


def main():
    seed = int(sys.argv[1])
    out = sys.argv[2]
    rng = random.Random(seed)

    EFFECTS = effects_for(seed)
    # The oracle restricts to syscall-observable effects via CANDOR_FUZZ_EFFECTS (e.g. "Fs Net Exec").
    allowed = os.environ.get("CANDOR_FUZZ_EFFECTS", "").split() or list(EFFECTS)
    effect = rng.choice([e for e in EFFECTS if e in allowed])
    leaf, marker = EFFECTS[effect]
    n = rng.randint(3, 9)  # chain length

    fns = [f"f{i:02d}" for i in range(n)]
    bodies = {}
    needed_helpers = set()
    extra_items = []  # per-edge module-level items (operator-overload structs/impls)
    expected = set(fns) | {"sink", "main"}
    # Optionally restrict the call FORMS (e.g. CANDOR_FUZZ_FORMS="op_add index deref" to fuzz only the
    # operator-overload edges — a focused lane for the desugared-call soundness holes).
    only_forms = os.environ.get("CANDOR_FUZZ_FORMS", "").split()

    # sink performs the effect directly. Sometimes DEFINE sink via a macro — a macro-generated
    # function that performs I/O must still be reported (the #5 macro-fn-visibility fix); if candor
    # ever re-omits macro-gen fns, the checker flags `sink(pure/omitted)`.
    bodies["sink"] = leaf
    # CANDOR_FUZZ_INSTRUMENT=1 brackets each chain fn's body with eprintln entry/exit markers — visible
    # to strace (a write(2,…)) but NOT to candor (stdio is not a classified effect), so the per-function
    # dynamic oracle can reconstruct the call stack at each effect syscall. (Disables macro_sink, which
    # would otherwise wrap sink in a macro that's awkward to instrument.)
    instrument = os.environ.get("CANDOR_FUZZ_INSTRUMENT") == "1"
    macro_sink = (rng.random() < 0.3) and not instrument

    def emit(name, body):
        # eprintln routes through the free fn `std::io::_eprint` (NOT a trait method), so candor sees a
        # pure non-local call — the markers don't pollute its analysis (a UFCS `io::Write::write_all`
        # would, via #6's effectful-dispatch rule). A literal eprintln is one atomic `write(2,…)` syscall,
        # which is exactly what strace and oracle_pf_check.py's regex expect.
        if instrument:
            return 'fn %s() { eprintln!("CFE %s"); %s eprintln!("CFX %s"); }' % (name, name, body, name)
        return "fn %s() { %s }" % (name, body)

    forms_log = {}
    for i in range(n):
        callee = fns[i + 1] if i + 1 < n else "sink"
        forms = edge_forms(callee, i)
        choices = (
            [f for f in forms if f in only_forms]
            if only_forms
            else [f for f in forms if f not in CONSTRUCTION_ONLY]
        )
        form_name = rng.choice(choices)
        body, helpers, extra, items = forms[form_name]
        bodies[fns[i]] = body
        needed_helpers.update(helpers)
        expected.update(extra)
        extra_items.extend(items)
        forms_log[fns[i]] = form_name

    # Assemble the source.
    lines = ["// GENERATED by soundness/gen.py — do not edit. seed=%d effect=%s" % (seed, effect), ""]
    for h in HELPERS:
        if h in needed_helpers:
            lines.append(HELPERS[h])
    for item in extra_items:  # per-edge operator-overload structs + effectful trait impls
        lines.append(item)
    lines.append("")
    if macro_sink:
        lines.append("macro_rules! mksink { () => { fn sink() { %s } }; }" % bodies["sink"])
        lines.append("mksink!();")
    else:
        lines.append(emit("sink", bodies["sink"]))
    for name in fns:
        lines.append(emit(name, bodies[name]))
    lines.append("")
    lines.append(emit("main", "%s();" % fns[0]))
    src = "\n".join(lines) + "\n"

    os.makedirs(os.path.join(out, "src"), exist_ok=True)
    with open(os.path.join(out, "Cargo.toml"), "w") as f:
        f.write('[package]\nname = "candor_fuzz"\nversion = "0.1.0"\nedition = "2021"\n\n[dependencies]\n')
    with open(os.path.join(out, "src", "main.rs"), "w") as f:
        f.write(src)
    with open(os.path.join(out, "truth.json"), "w") as f:
        json.dump(
            {"seed": seed, "effect": effect, "marker": marker, "expect": sorted(expected), "forms": forms_log},
            f,
            indent=2,
        )


if __name__ == "__main__":
    main()
