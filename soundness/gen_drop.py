#!/usr/bin/env python3
"""Drop-soundness fuzzer (Bet 4 follow-up).

The HIR engine has no node for an implicit, scope-exit `Drop`, so an effectful `Drop` guard (I/O on
the way out — an RAII transaction/lock/flush) was invisible to it. The fix reads MIR `Drop` terminators
and follows the dropped type's reachable LOCAL `Drop::drop` impls — directly, through value-embedded
fields (struct/tuple/array/enum), AND through std OWNING containers (Box/Vec/Rc/Arc/HashMap/…), whose
element is hidden behind a heap pointer. This generator makes that a tested gate: it threads a KNOWN
effect into a `Guard`'s `Drop`, wraps the guard in a random container FORM, lets it drop, and asserts
the dropping function inherits the effect (or `Unknown`). A function reported PURE is a silent
under-report — the same trust-contract violation the rest of the harness hunts, but via drop glue.

TWO AXES, and the second one exists because the first could not express SOUNDNESS R756. A drop has a
FORM (what wraps the guard) and a SITE (which BODY the drop happens in). Every form this generator knew
emitted a top-level `fn`, so all 40 default seeds landed in a `DefKind::Fn` body — and the defect was a
filter that admitted exactly those, dropping every closure and coroutine body on the floor. The gate was
green over 40 cases none of which could reach the filter (R757: a test inheriting the blind spot of the
report that prompted it). The SITE axis puts the same drop inside a closure body, an uncalled closure, a
nested closure, an `async` block, an `async` block across an `.await`, and a combinator callback.

`closure_uncalled` deliberately asserts the OVER-approximation: the enclosing fn is charged for a drop
inside a closure that is never called. That is the same rule the engine already applies to a CALL inside
an uncalled closure body (`enclosing_named_fn` charges it to the enclosing item), so the assertion pins
the two paths together rather than encoding a new policy.

Usage:  gen_drop.py <seed> <out-dir>     # writes <out-dir>/{Cargo.toml, src/main.rs, truth.json}
        gen_drop.py --handoff <seed> <out-dir> [--all]   # the candor-scan HAND-OFF arm (see below)
Env:    CANDOR_FUZZ_EFFECTS="Fs Net"     # restrict the effect pool
        CANDOR_FUZZ_DROP_FORMS="box vec" # restrict the FORM pool
        CANDOR_FUZZ_DROP_SITES="closure" # restrict the SITE pool
"""
import json
import os
import random
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from gen import effects_for  # noqa: E402

# Each form wraps a `Guard` value in a type that, when dropped, must still run `Guard::drop` — directly,
# through a value field, or through a std owning container's heap-indirected drop glue.
DROP_FORMS = {
    "direct": "Guard",
    "field":  "S { g: Guard }",                 # value field of a local struct
    "tuple":  "(0u8, Guard)",
    "array":  "[Guard]",
    "option": "Some(Guard)",                    # enum variant
    "box":    "Box::new(Guard)",                # heap, single
    "vec":    "vec![Guard]",                    # heap, collection
    "rc":     "std::rc::Rc::new(Guard)",        # refcounted (drops on last ref — over-approximate)
    "arc":    "std::sync::Arc::new(Guard)",
    "hashmap": "{ let mut m = std::collections::HashMap::new(); m.insert(0u8, Guard); m }",
    "nested": "vec![Box::new(Guard)]",          # container-of-container
    # a CLOSURE that captured the guard by move, dropped WITHOUT ever being called. Closures are their
    # own `TyKind` (not `TyKind::Adt`), so the field/element walker needs a dedicated arm for them — a
    # real hole found 2026-08-29 (a captured effectful Drop was silently lost on scope-exit-only drop).
    "closure": "{ let g = Guard; move || { let _ = &g; } }",
}

# Emitted once, when a site needs it. A hand-rolled `block_on` keeps the generated crate dependency-free
# (the fuzzer must build with a bare `cargo build`, no registry access).
SITE_HELPERS = {
    "block_on": (
        "fn block_on<F: std::future::Future>(f: F) -> F::Output {\n"
        "    use std::task::{Context, Poll};\n"
        "    let mut f = std::pin::pin!(f);\n"
        "    let w = std::task::Waker::noop();\n"
        "    let mut cx = Context::from_waker(w);\n"
        "    loop { if let Poll::Ready(v) = f.as_mut().poll(&mut cx) { return v } }\n"
        "}"
    ),
    "ready": "async fn ready() {}",
}

# Each SITE wraps the drop statement in the BODY it happens in. `%s` is the `let _x = <form>;` statement.
# The key fact the SITE axis tests: a closure or coroutine body is its OWN `DefKind` and its OWN MIR body
# — so anything keyed on the enclosing function's `DefKind` never sees these drops at all.
DROP_SITES = {
    # the original (and only) shape: the drop happens in the top-level `fn`'s own body.
    "fn":               ("%s", []),
    # a closure BODY (not a closure VALUE — that is the `closure` FORM, which is a different question).
    "closure":          ("let c = || { %s }; c();", []),
    # the same, never called. The enclosing fn must STILL be charged (sound over-approximation) — this is
    # the fabrication-direction control, and it pins the drop path to the call path's existing rule.
    "closure_uncalled": ("let c = || { %s }; let _ = &c;", []),
    "closure_nested":   ("let c = || { let d = || { %s }; d(); }; c();", []),
    # a coroutine body: `async` desugars to a separate body owner, so the drop is not in the fn's MIR.
    "async_block":      ("block_on(async { %s });", ["block_on"]),
    # the drop is LIVE ACROSS an await point — the shape the row measured as a distinct cell.
    "async_await":      ("block_on(async { %s ready().await; });", ["block_on", "ready"]),
    # the closure is handed to a std combinator, so the body is a callback the engine resolves per site.
    "for_each":         ("(0..1).for_each(|_| { %s });", []),
}


def main():
    seed = int(sys.argv[1])
    out = sys.argv[2]
    rng = random.Random(seed)

    EFFECTS = effects_for(seed)
    allowed = os.environ.get("CANDOR_FUZZ_EFFECTS", "").split() or list(EFFECTS)
    effect = rng.choice([e for e in EFFECTS if e in allowed])
    leaf, marker = EFFECTS[effect]
    n = rng.randint(3, 9)

    form_pool = os.environ.get("CANDOR_FUZZ_DROP_FORMS", "").split() or list(DROP_FORMS)
    site_pool = os.environ.get("CANDOR_FUZZ_DROP_SITES", "").split() or list(DROP_SITES)
    for bad in [f for f in form_pool if f not in DROP_FORMS]:
        sys.exit("gen_drop.py: unknown DROP FORM %r" % bad)
    for bad in [t for t in site_pool if t not in DROP_SITES]:
        sys.exit("gen_drop.py: unknown DROP SITE %r" % bad)

    fns = [f"f{i:02d}" for i in range(n)]
    forms_log = {}
    sites_log = {}
    bodies = {}
    helpers = []
    for name in fns:
        form = rng.choice(form_pool)
        site = rng.choice(site_pool)
        forms_log[name] = form
        sites_log[name] = site
        tmpl, needs = DROP_SITES[site]
        bodies[name] = tmpl % ("let _x = %s;" % DROP_FORMS[form])
        for h in needs:
            if h not in helpers:
                helpers.append(h)

    lines = [
        "// GENERATED by soundness/gen_drop.py — do not edit. seed=%d effect=%s" % (seed, effect),
        "struct Guard;",
        "impl Drop for Guard { fn drop(&mut self) { %s } }" % leaf,
        "struct S { g: Guard }",
        "",
    ]
    for h in helpers:
        lines.append(SITE_HELPERS[h])
    if helpers:
        lines.append("")
    for name in fns:
        lines.append("fn %s() { %s }" % (name, bodies[name]))
    lines.append("")
    lines.append("fn main() { %s }" % " ".join("%s();" % f for f in fns))

    expected = set(fns) | {"main"}
    os.makedirs(os.path.join(out, "src"), exist_ok=True)
    with open(os.path.join(out, "Cargo.toml"), "w") as f:
        f.write('[package]\nname = "candor_drop"\nversion = "0.1.0"\nedition = "2021"\n\n[dependencies]\n')
    with open(os.path.join(out, "src", "main.rs"), "w") as f:
        f.write("\n".join(lines) + "\n")
    with open(os.path.join(out, "truth.json"), "w") as f:
        json.dump(
            {
                "seed": seed,
                "effect": effect,
                "marker": marker,
                "expect": sorted(expected),
                "forms": forms_log,
                "sites": sites_log,
            },
            f, indent=2,
        )


# ─────────────────────────────────────────────────────────────────────────────────────────────────────
# THE HAND-OFF ARM (`gen_drop.py --handoff <seed> <out>`), for candor-scan — the STABLE scanner.
#
# WHY IT EXISTS. Every value the arm above generates is let-bound and dies at its own scope end, so it
# never reaches the shapes where a value is HANDED to something else and dies THERE: a callee that
# drops its by-value parameter, a wrapper parameter, a generic one, a closure parameter, std's `drop`,
# a callee that stores into a container this frame owns. SOUNDNESS R198/R200/R323/R1121 were all in
# that gap, and the gate above (which drives the DEEP engine) was green over every one of them. This
# arm puts the hand-off on three axes:
#
#   POSITION  stmt (`call;`), tail (`call` as the fn's value), q (`let _x = call?;`), qret (`let x =
#             call?; Ok(x)` — the `?` operand whose value is RETURNED, R200's own spelling), clos (the
#             call as the tail of a let-bound closure invoked and discarded)
#   TARGET    local fn, wrapper param (`Option<H>`), generic param, closure param, std `drop`, a
#             callee that STORES into a `Vec` this frame owns
#   BINDING   direct (`H::new()` inline), let, shadowed (the name was bound to something else first),
#             sibling (a sibling block bound the name first)
#
# GROUND TRUTH IS EXECUTED, never predicted: the crate carries a `gt` binary that calls every cell
# against a drop counter. `soundness/run_handoff.sh` builds and runs it, scans with candor-scan, and
# FAILS on a cell that drops (GT > 0) whose row is neither charged nor disclosed, unless the shape is in
# the measured known-open register (`soundness/known_open_handoff.tsv`, kind `h`).
# ─────────────────────────────────────────────────────────────────────────────────────────────────────
HANDOFF_POSITIONS = ["stmt", "tail", "q", "qret", "clos"]
HANDOFF_TARGETS = ["local", "wrap", "generic", "closure", "stddrop", "store"]
HANDOFF_BINDINGS = ["direct", "let", "shadow", "sibling"]


def handoff_cell(name, pos, target, binding, sfx):
    """-> (signature, body) for one cell. Every cell hands exactly one fresh `H` to `target`."""
    pre = []
    # BINDING — how the VALUE (and, for a closure target, the CALLEE) reaches the call.
    if binding == "direct":
        val = "H::new()"
    else:
        if binding == "shadow":
            pre.append("let h = 0u32; let _ = h;")
        elif binding == "sibling":
            pre.append("{ let h = 0u32; let _ = h; }")
        pre.append("let h = H::new();")
        val = "h"
    q = pos in ("q", "qret")
    if target == "local":
        call = (f"tlq_{sfx}({val}, n)" if q else f"tl_{sfx}({val})")
    elif target == "wrap":
        call = (f"twq_{sfx}(Some({val}), n)" if q else f"tw_{sfx}(Some({val}))")
    elif target == "generic":
        call = (f"tgq_{sfx}({val}, n)" if q else f"tg_{sfx}({val})")
    elif target == "closure":
        if binding == "shadow":
            pre.append("let f = |_x: u32| {}; f(1);")
        elif binding == "sibling":
            pre.append("{ let f = |_x: u32| {}; f(1); }")
        if q:
            pre.append("let f = |_h: H, n: u32| -> Result<u32, ()> { gen(n) };")
            call = f"f({val}, n)"
        else:
            pre.append("let f = |_h: H| {};")
            call = f"f({val})"
    elif target == "stddrop":
        call = (f"{{ std::mem::drop({val}); gen(n) }}" if q else f"std::mem::drop({val})")
    elif target == "store":
        pre.insert(0, "let mut v: Vec<H> = Vec::new();")
        call = (f"tsq_{sfx}({val}, &mut v, n)" if q else f"ts_{sfx}({val}, &mut v)")
    else:
        raise ValueError(target)
    pre_s = " ".join(pre)
    if pos == "stmt":
        return f"pub fn {name}()", f"{pre_s} {call};"
    if pos == "tail":
        return f"pub fn {name}()", f"{pre_s} {call}"
    if pos == "q":
        return f"pub fn {name}(n: u32) -> Result<(), ()>", f"{pre_s} let _x = {call}?; Ok(())"
    if pos == "qret":
        return f"pub fn {name}(n: u32) -> Result<u32, ()>", f"{pre_s} let x = {call}?; Ok(x)"
    if pos == "clos":
        return f"pub fn {name}()", f"let k = || {{ {pre_s} {call} }}; k();"
    raise ValueError(pos)


def handoff_main(seed, out, every):
    rng = random.Random(seed)
    sfx = "%x" % rng.randint(0x100, 0xfff)  # names vary per seed, so no answer can key on one spelling
    grid = [(p, t, b) for p in HANDOFF_POSITIONS for t in HANDOFF_TARGETS for b in HANDOFF_BINDINGS]
    cells = grid if every else rng.sample(grid, 12)
    lines = [
        "// GENERATED by soundness/gen_drop.py --handoff — do not edit. seed=%d" % seed,
        "use std::sync::atomic::{AtomicUsize, Ordering};",
        "pub static DROPS: AtomicUsize = AtomicUsize::new(0);",
        "pub struct H { pub tag: u32 }",
        "impl H { pub fn new() -> H { H { tag: 0 } } }",
        "impl Drop for H { fn drop(&mut self) { DROPS.fetch_add(1, Ordering::SeqCst);"
        " let _ = std::fs::write(\"/dev/null\", b\"x\"); } }",
        "pub fn gen(i: u32) -> Result<u32, ()> { if i == 99 { Err(()) } else { Ok(i) } }",
        f"fn tl_{sfx}(_h: H) {{}}",
        f"fn tlq_{sfx}(_h: H, n: u32) -> Result<u32, ()> {{ gen(n) }}",
        f"fn tw_{sfx}(_o: Option<H>) {{}}",
        f"fn twq_{sfx}(_o: Option<H>, n: u32) -> Result<u32, ()> {{ gen(n) }}",
        f"fn tg_{sfx}<T>(_t: T) {{}}",
        f"fn tgq_{sfx}<T>(_t: T, n: u32) -> Result<u32, ()> {{ gen(n) }}",
        f"fn ts_{sfx}(h: H, v: &mut Vec<H>) {{ v.push(h) }}",
        f"fn tsq_{sfx}(h: H, v: &mut Vec<H>, n: u32) -> Result<u32, ()> {{ v.push(h); gen(n) }}",
        "",
    ]
    shapes, calls = {}, []
    for i, (p, t, b) in enumerate(cells):
        name = f"c{i:02d}_{sfx}"
        sig, body = handoff_cell(name, p, t, b, sfx)
        lines.append(f"{sig} {{ {body} }}")
        shapes[name] = f"pos={p},target={t},binding={b}"
        calls.append(f'm!("{name}", {name}({"1" if p in ("q", "qret") else ""}));')
    # CALIBRATION — a shape candor-scan is KNOWN to miss and that is NOT in the register, so the checker
    # must fail on it. A BORROW handed to a non-unit std call in the tail keeps the binding on the escape
    # spine (call arguments are "the callee's"), although `size_of_val` only reads it: the `H` dies here
    # (executed: 1). The previous seed, a field read in the tail (`let h = H::new(); h.tag`), stopped being
    # a silence when SOUNDNESS R209(b)/R201 (rustagent-v048) stopped a drop-free root's field projection
    # from marking a `let`-bound base as escaping — the seed has to track what is still open.
    if os.environ.get("CANDOR_HANDOFF_SEED_SILENCE"):
        name = f"seeded_{sfx}"
        lines.append(f"pub fn {name}() -> usize {{ let h = H::new(); std::mem::size_of_val(&h) }}")
        shapes[name] = "pos=seeded,target=borrow_to_std_tail,binding=let"
        calls.append(f'm!("{name}", {name}());')
    os.makedirs(os.path.join(out, "src", "bin"), exist_ok=True)
    with open(os.path.join(out, "Cargo.toml"), "w") as f:
        f.write('[package]\nname = "candor_handoff"\nversion = "0.1.0"\nedition = "2021"\n\n'
                '[lib]\npath = "src/lib.rs"\n\n[[bin]]\nname = "gt"\npath = "src/bin/gt.rs"\n\n[workspace]\n')
    with open(os.path.join(out, "src", "lib.rs"), "w") as f:
        f.write("\n".join(lines) + "\n")
    with open(os.path.join(out, "src", "bin", "gt.rs"), "w") as f:
        f.write(
            "#![allow(unused_must_use, clippy::let_unit_value)]\n"
            "use candor_handoff::*;\nuse std::sync::atomic::Ordering;\n"
            "macro_rules! m { ($n:expr, $e:expr) => {{ DROPS.store(0, Ordering::SeqCst); let r = $e;"
            " let d = DROPS.load(Ordering::SeqCst); std::mem::forget(r); println!(\"{}\\t{}\", $n, d); }}; }\n"
            "fn main() {\n    " + "\n    ".join(calls) + "\n}\n")
    with open(os.path.join(out, "truth.json"), "w") as f:
        json.dump({"seed": seed, "shapes": shapes}, f, indent=2)


if __name__ == "__main__":
    if len(sys.argv) > 1 and sys.argv[1] == "--handoff":
        handoff_main(int(sys.argv[2]), sys.argv[3], "--all" in sys.argv[4:])
    else:
        main()
