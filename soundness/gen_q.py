#!/usr/bin/env python3
"""`?`-POSITION generator — the property gate for the R173 `?`-veto family.

WHAT THIS CHECKS. Each seed emits pairs of functions that are two SPELLINGS OF ONE PROGRAM,
generated from a single description so they cannot drift apart. Two modes:

  hoist    REF: `let t = <OPERAND>; t?;`        VAR: `<OPERAND>?;`
           `EXPR?` and `let t = EXPR; t?` evaluate identically — same constructions, same drops,
           same order. So the two must be charged the SAME. Any difference is a defect, and the
           side charging less is under-reporting a drop the ground-truth run actually performed.

  looprot  REF: `<loop> { <CTOR> ...?; }`       VAR: `<loop> { ...?; <CTOR> }`
           A `?` inside a loop body is live for everything that body constructs: on the second and
           later iterations the construction has already happened when the `?` is reached. So
           inferred(VAR) ⊇ inferred(REF) — moving a `?` earlier in a loop body cannot LOSE drops.
           (This is R187's own fix rationale read as a property.)

WHY NOT "insert a `?` that can never error". That was the first formulation tried, and it is
VACUOUS on exactly the shapes the bugs live in — measured, 0 of 6 rediscovered over 40 seeds. The
`?`-free twin of the canonical shape is `collect_loop_noq` in the R187 fixture, and it is a control
whose expected answer is ABSENT: with the constructed value escaping by return, NOTHING drops in
frame until a `?` creates an early exit. Comparing against it compares against an empty set, which
no under-report can violate. The two spellings above are non-vacuous because both sides really do
drop, which `examples/gt.rs` measures.

CALIBRATION. Against the six pre-fix binaries of the ⟨0.35⟩ regressions — see soundness/README.md.

The shape space is COMPOSED, not enumerated: {construction form} x {operand or loop form} x
{ending} — so it reaches combinations no fixture author wrote down.

Usage:  gen_q.py <seed> <out-dir>
"""
import json
import os
import random
import sys

# The effect `H::drop` performs, varied by seed. Every leaf is READ-ONLY: the generated program is
# EXECUTED by examples/gt.rs, so a destructive leaf would delete real files in the crate directory.
EFFECT_LEAVES = {
    "Fs":  'let _ = std::fs::metadata(&self.p);',
    "Net": 'let _ = std::net::UdpSocket::bind("127.0.0.1:0");',
    "Env": 'let _ = std::env::var(&self.p);',
}

# --- construction forms: a STATEMENT LIST that constructs an `H` and stores it in `out`. ----------
# `{K}` is the pair's unique suffix (a `macro_rules!` in a function body must not collide).
CTORS = {
    "direct":        'out.push(H::new("a"));',
    "let_bind":      'let g{K} = H::new("a"); out.push(g{K});',
    "ctor_fn":       'out.push(make_h("a"));',
    # crate-level single-arm templates — the R199 shape (a macro inside the `?` operand)
    "mac_expr":      'out.push(mk_h!("a"));',
    "mac_stmt":      'push_h!(out, "a");',
    # a template reaching its construction through ANOTHER template (R203 (c), nested)
    "mac_nested":    'out.push(via!("a"));',
    "mac_tmpl_stmt": 'wrap_stmt!(out, "a");',
    # a `macro_rules!` defined in the FUNCTION BODY (R206)
    "mac_local":     'macro_rules! lm{K} {{ ($p:expr) => {{ H::new($p) }} }} out.push(lm{K}!("a"));',
    # a `macro_rules!` defined INSIDE re-parsed macro tokens and used there (R210)
    "mac_blocktok":  'idm!({{ macro_rules! lb{K} {{ ($p:expr) => {{ H::new($p) }} }} out.push(lb{K}!("a")); }});',
    # a REPETITION template — its arm does not parse as a comma-punctuated expression list (R203 (b))
    "mac_repeat":    'push_all!(out, "a");',
    # tokens readable neither as an expression list nor as statements (R203 (b), maplit shape)
    "mac_unparsed":  'let mm{K} = kv!("a" => H::new("a")); out.extend(mm{K}.into_values());',
    # a std collection macro in the operand — R199's literal fixture
    "mac_vec":       'out.extend(vec![H::new("a")]);',
}

# --- `?` operands (hoist mode): an expression of type `Result<_, ()>` that runs `{S}` first. ------
# `parens` = the VAR spelling needs parentheses before `?` in statement position.
OPERANDS = {
    "block":      ('{{ {S} gen(n) }}', False),
    "nested":     ('{{ {{ {S} }} gen(n) }}', False),
    "closure":    ('run_cb(|| {{ {S} gen(n).map(|_| ()) }})', False),
    "matcharm":   ('match m {{ _ => {{ {S} gen(n) }} }}', True),
    "ifelse":     ('if m > 0 {{ {S} gen(n) }} else {{ Ok(0u32) }}', True),
    "tryforeach": ('[0u32].iter().try_for_each(|_x{K}| {{ {S} gen(n).map(|_| ()) }})', False),
    "macroid":    ('idm!({{ {S} gen(n) }})', False),
}

# --- loop heads (looprot mode). Every one exits through the `?` after two clean iterations. -------
LOOPS = {
    "forloop":   'for _i{K} in 0..9u32 {{ {B} }}',
    "whileloop": 'while c{K} < 9 {{ {B} }}',
    "loopbrk":   'loop {{ {B} }}',
}
LOOP_Q = 'let _v{K} = tick(&mut c{K})?;'

# --- how the function disposes of `out` and what it returns. The escape model is part of the
# question (R172's site gate suppresses a leaf all of whose sites escape), so it is generated too.
ENDINGS = {
    # `out` escapes by return: nothing drops in frame on the normal path, only on the `?` exit
    "ret_out":  ("Vec<H>", "Ok(out)"),
    # `out` is dropped in frame while a DIFFERENT value of the same leaf escapes (R203's shape)
    "drop_h":   ("H", 'let h{K} = H::try_new(m, "b")?; let _ = out; Ok(h{K})'),
    # `out` is dropped in frame and nothing of the leaf escapes
    "drop_len": ("usize", 'let r{K} = out.len(); let _ = out; Ok(r{K})'),
}


def prelude(leaf):
    return [
        "use std::sync::atomic::{AtomicUsize, Ordering};",
        "pub static DROPS: AtomicUsize = AtomicUsize::new(0);",
        "pub struct H { pub p: String }",
        "impl Drop for H { fn drop(&mut self) { DROPS.fetch_add(1, Ordering::SeqCst); %s } }" % leaf,
        "impl H {",
        '    pub fn new(p: &str) -> H { H { p: p.to_string() } }',
        "    pub fn try_new(n: u32, p: &str) -> Result<H, ()> { if n == 0 { Err(()) } else { Ok(H::new(p)) } }",
        "}",
        "pub fn make_h(p: &str) -> H { H::new(p) }",
        "pub fn gen(n: u32) -> Result<u32, ()> { if n == 0 { Err(()) } else { Ok(n) } }",
        "/// The loop `?`: two clean iterations, then the error that exits the loop and the function.",
        "pub fn tick(c: &mut u32) -> Result<u32, ()> { if *c == 0 { Err(()) } else { *c -= 1; Ok(*c) } }",
        "pub fn run_cb<F: FnOnce() -> Result<(), ()>>(f: F) -> Result<(), ()> { f() }",
        "",
        "#[macro_export] macro_rules! idm { ($e:expr) => { $e } }",
        "#[macro_export] macro_rules! mk_h { ($p:expr) => { $crate::H::new($p) } }",
        "#[macro_export] macro_rules! via { ($p:expr) => { $crate::mk_h!($p) } }",
        "#[macro_export] macro_rules! push_h { ($o:expr, $p:expr) => {{ let x = $crate::H::new($p); $o.push(x); }} }",
        "#[macro_export] macro_rules! wrap_stmt { ($o:expr, $p:expr) => {{ push_h!($o, $p); }} }",
        "#[macro_export] macro_rules! push_all { ($o:expr, $($p:expr),*) => {{ $( $o.push($crate::H::new($p)); )* }} }",
        "#[macro_export] macro_rules! kv { ($($k:expr => $v:expr),*) => {{ let mut m = ::std::collections::HashMap::new(); $( m.insert($k, $v); )* m }} }",
        "",
    ]


def render(name, shape, k, variant):
    """Render one spelling. `variant` False = REF (hoisted / `?` last), True = VAR."""
    ctor = CTORS[shape["ctor"]].format(K=k)
    ret, end = ENDINGS[shape["ending"]]
    end = end.format(K=k)
    pre = ""
    if shape["mode"] == "hoist":
        tmpl, parens = OPERANDS[shape["operand"]]
        op = tmpl.format(S=ctor, K=k)
        body = ("(%s)?;" % op if parens else "%s?;" % op) if variant \
            else "let t%s = %s; t%s?;" % (k, op, k)
    else:
        q = LOOP_Q.format(K=k)
        inner = "%s %s" % (q, ctor) if variant else "%s %s" % (ctor, q)
        pre = "let mut c%s = 2u32;" % k
        body = LOOPS[shape["loop"]].format(B=inner, K=k)
    return (
        "#[allow(unused, clippy::all)]\n"
        "pub fn %s(n: u32, m: u32) -> Result<%s, ()> {\n"
        "    let mut out: Vec<H> = Vec::new();\n"
        "    %s\n    %s\n    %s\n}\n" % (name, ret, pre, body, end)
    )


def pick_shape(rng):
    mode = rng.choice(["hoist", "hoist", "looprot"])
    s = {"mode": mode, "ctor": rng.choice(list(CTORS)), "ending": rng.choice(list(ENDINGS))}
    if mode == "hoist":
        s["operand"] = rng.choice(list(OPERANDS))
    else:
        s["loop"] = rng.choice(list(LOOPS))
    return s


def emit(out, seed, effect, leaf, pairs, kind, crate="qgen", extra_items=(), gen_name="gen_q.py",
         render_fn=None, equal_fn=None):
    """Write the crate. `render_fn(name, shape, k, variant) -> str` defaults to this module's.

    Shared with gen_macro.py so the two generators cannot drift apart in how they build the crate,
    the ground-truth runner or truth.json (§F1 #3: two implementations of one question do drift).
    """
    render_fn = render_fn or render
    # Which pairs are an EXACT equivalence (both directions checked) rather than a one-sided bound.
    equal_fn = equal_fn or (lambda sh: sh.get("mode") == "hoist")
    lines = ["// GENERATED by soundness/%s — do not edit. seed=%d effect=%s" % (gen_name, seed, effect)]
    lines += prelude(leaf)
    lines += list(extra_items)
    for i, (a, b, shape) in enumerate(pairs):
        k = "%03d" % i
        lines.append("// pair %s: %s" % (k, json.dumps(shape, sort_keys=True)))
        lines.append(render_fn(a, shape, k, False))
        lines.append(render_fn(b, shape, k, True))

    # The ground-truth run. `mem::forget` on the RETURN VALUE is load-bearing: without it the
    # caller's own drop of the returned `Vec<H>`/`H` is counted, and the number would no longer be
    # the IN-FRAME drop count the report is about.
    gt = ["// GENERATED by soundness/%s — the ground-truth run." % gen_name,
          "// Prints, per pair: `<base d_err> <base d_ok> <twin d_err> <twin d_ok> <base-name>`,",
          "// i.e. the IN-FRAME drops each spelling performs with the `?` firing and on the normal path.",
          "// A pair whose spellings never drop cannot ground-truth anything (§E3).",
          "use std::sync::atomic::Ordering;",
          "#[allow(unused, forgetting_copy_types, forgetting_references)]",
          "fn main() {"]
    for a, b, _ in pairs:
        for nm in (a, b):
            gt.append('    let z = %s::DROPS.load(Ordering::SeqCst);' % crate)
            gt.append('    let r = %s::%s(0, 1); let d_err = %s::DROPS.load(Ordering::SeqCst) - z;' % (crate, nm, crate))
            gt.append('    std::mem::forget(r);')
            gt.append('    let z2 = %s::DROPS.load(Ordering::SeqCst);' % crate)
            gt.append('    let r2 = %s::%s(1, 1); let d_ok = %s::DROPS.load(Ordering::SeqCst) - z2;' % (crate, nm, crate))
            gt.append('    std::mem::forget(r2);')
            gt.append('    print!("{} {} ", d_err, d_ok);')
        gt.append('    println!("{}", "%s");' % a)
    gt.append("}")
    # The ground-truth main is SPLIT into chunks. A single `main` with a few thousand statements
    # made rustc die with SIGBUS while emitting DWARF (measured on the exhaustive `--all` crate),
    # and `debug = 0` in the manifest below is the other half of that fix. A generated program that
    # cannot compile is no evidence at all, so both are load-bearing, not tidiness.
    body, chunks, cur = [], [], []
    hdr = gt.index("fn main() {")   # located, not counted: a comment line added above must
    for line in gt[hdr + 1:-1]:     # not silently shift the slice and emit unbalanced braces
        cur.append(line)
        if line.startswith('    println!'):
            if len(cur) > 120:
                chunks.append(cur); cur = []
    if cur:
        chunks.append(cur)
    out_lines = gt[:hdr - 1]   # everything up to (not including) the `#[allow]` + `fn main`
    for i, ch in enumerate(chunks):
        out_lines.append("#[allow(unused, forgetting_copy_types, forgetting_references)]")
        out_lines.append("fn part%d() {" % i)
        out_lines += ch
        out_lines.append("}")
    out_lines.append("fn main() { %s }" % " ".join("part%d();" % i for i in range(len(chunks))))
    gt = out_lines

    os.makedirs(os.path.join(out, "src"), exist_ok=True)
    os.makedirs(os.path.join(out, "examples"), exist_ok=True)
    with open(os.path.join(out, "Cargo.toml"), "w") as f:
        f.write('[package]\nname = "%s"\nversion = "0.1.0"\nedition = "2021"\n\n[dependencies]\n'
                '\n# `debug = 0`: see the SIGBUS note by the chunked ground-truth main above.\n'
                '[profile.dev]\ndebug = 0\nincremental = false\n' % crate)
    with open(os.path.join(out, "src", "lib.rs"), "w") as f:
        f.write("\n".join(lines) + "\n")
    with open(os.path.join(out, "examples", "gt.rs"), "w") as f:
        f.write("\n".join(gt) + "\n")
    with open(os.path.join(out, "truth.json"), "w") as f:
        json.dump({"seed": seed, "effect": effect, "kind": kind,
                   "pairs": [{"base": a, "twin": b, "shape": s,
                              "equal": equal_fn(s)} for a, b, s in pairs]}, f, indent=2)


def all_shapes():
    """Every point of the shape space, in a fixed order. Used by `--all` to build the known-open
    baseline: a baseline SAMPLED from random seeds would mark a shape "new" the first time a later
    seed happened to reach it, so the baseline is enumerated exhaustively instead."""
    out = []
    for ctor in CTORS:
        for ending in ENDINGS:
            for op in OPERANDS:
                out.append({"mode": "hoist", "ctor": ctor, "ending": ending, "operand": op})
            for lp in LOOPS:
                out.append({"mode": "looprot", "ctor": ctor, "ending": ending, "loop": lp})
    return out


def main():
    if sys.argv[1] == "--all":
        shapes = all_shapes()
        pairs = [("r%03d" % i, "v%03d" % i, sh) for i, sh in enumerate(shapes)]
        emit(sys.argv[2], 0, "Fs", EFFECT_LEAVES["Fs"], pairs, "q")
        return
    seed = int(sys.argv[1])
    out = sys.argv[2]
    rng = random.Random(seed)
    effect = rng.choice(sorted(EFFECT_LEAVES))
    n = rng.randint(6, 10)
    pairs = [("r%03d" % i, "v%03d" % i, pick_shape(rng)) for i in range(n)]
    emit(out, seed, effect, EFFECT_LEAVES[effect], pairs, "q")


if __name__ == "__main__":
    main()
