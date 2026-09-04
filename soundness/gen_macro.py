#!/usr/bin/env python3
"""MACRO-EQUIVALENCE generator — a single-arm `macro_rules!` and the spelling it expands to must
produce the SAME answer for the calling function.

WHAT THIS CHECKS. Each pair is generated from ONE description and rendered twice: once with the
construction written directly, once with the identical construction reached through a single-arm
`macro_rules!` template. Neither spelling is written by hand, so the two cannot drift apart the way
a fixture and its "control" do. Both are placed in the SAME surrounding context (a `?` operand, a
loop, a plain statement) and given the SAME ending, so the only thing that differs is the macro.

    inferred(macro spelling)  ==  inferred(direct spelling)

The macro spelling losing an effect the direct spelling is charged is a SILENT UNDER-REPORT. It is
also, historically, this engine's single most productive bug shape: R142, R143 (call-edge macro
resolution), R199 (a macro in the `?` operand), R203 (a template, unparsable tokens, a nested
macro), R204 (a statement-position macro inside a template), R206/R207 (a `macro_rules!` in a body
/ a repetition template), R210 (a `macro_rules!` inside re-parsed block tokens). In every one, the
direct twin was charged and the macro spelling was silent — and in every one the direct twin was a
hand-written control that somebody had to think of.

Both spellings are EXECUTED by `examples/gt.rs`; a pair whose spellings never drop is not judged.

CALIBRATION. Against the six pre-fix binaries of the ⟨0.35⟩ regressions — see soundness/README.md.

Usage:  gen_macro.py <seed> <out-dir>
"""
import random
import sys
import os

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from gen_q import EFFECT_LEAVES, ENDINGS, emit  # noqa: E402

# --- The macro forms. Each is (crate-level items, DIRECT statements, MACRO statements). ----------
# `{K}` is replaced by the pair's suffix. Everything lives in one lib.rs, so a plain `macro_rules!`
# above the functions is in scope; `#[macro_export]` + `$crate::` is used where the form is about
# the exported/hygienic spelling specifically.
MACRO_FORMS = {
    # the plain expression template
    "expr": (
        ['macro_rules! mE{K} { ($p:expr) => { H::new($p) } }'],
        'out.push(H::new("a"));',
        'out.push(mE{K}!("a"));',
    ),
    # a statement-position template with a block body
    "stmt": (
        ['macro_rules! mS{K} { ($o:expr, $p:expr) => {{ let x = H::new($p); $o.push(x); }} }'],
        '{ let x{K} = H::new("a"); out.push(x{K}); }',
        'mS{K}!(out, "a");',
    ),
    # a template that reaches its construction through ANOTHER template (R203 (c))
    "nested": (
        ['macro_rules! mEn{K} { ($p:expr) => { H::new($p) } }',
         'macro_rules! mN{K} { ($p:expr) => { mEn{K}!($p) } }'],
        'out.push(H::new("a"));',
        'out.push(mN{K}!("a"));',
    ),
    # a template whose body is a STATEMENT-POSITION call of another template (R204)
    "tmpl_stmt": (
        ['macro_rules! mSt{K} { ($o:expr, $p:expr) => {{ let x = H::new($p); $o.push(x); }} }',
         'macro_rules! mW{K} { ($o:expr, $p:expr) => {{ mSt{K}!($o, $p); }} }'],
        '{ let x{K} = H::new("a"); out.push(x{K}); }',
        'mW{K}!(out, "a");',
    ),
    # a `macro_rules!` defined in the FUNCTION BODY (R206)
    "body_local": (
        [],
        'out.push(H::new("a"));',
        'macro_rules! mL{K} { ($p:expr) => { H::new($p) } } out.push(mL{K}!("a"));',
    ),
    # a `macro_rules!` defined INSIDE re-parsed macro tokens and used there (R210)
    "blocktok_local": (
        [],
        '{ out.push(H::new("a")); }',
        'idm!({ macro_rules! mB{K} { ($p:expr) => { H::new($p) } } out.push(mB{K}!("a")); });',
    ),
    # a REPETITION template — its arm does not parse as a comma-punctuated expression list (R203 (b))
    "repetition": (
        ['macro_rules! mR{K} { ($o:expr, $($p:expr),*) => {{ $( $o.push(H::new($p)); )* }} }'],
        'out.push(H::new("a"));',
        'mR{K}!(out, "a");',
    ),
    # std `vec!` in the operand — R199's literal fixture
    "std_vec": (
        [],
        '{ let mut v{K} = Vec::new(); v{K}.push(H::new("a")); out.extend(v{K}); }',
        'out.extend(vec![H::new("a")]);',
    ),
    # tokens readable neither as an expression list nor as statements (R203 (b), maplit shape)
    "unparsed": (
        ['macro_rules! mKV{K} { ($($k:expr => $v:expr),*) => {{ let mut mm = ::std::collections::HashMap::new(); $( mm.insert($k, $v); )* mm }} }'],
        '{ let mut u{K} = ::std::collections::HashMap::new(); u{K}.insert("a", H::new("a")); out.extend(u{K}.into_values()); }',
        'let u{K} = mKV{K}!("a" => H::new("a")); out.extend(u{K}.into_values());',
    ),
    # a `$crate::`-hygienic exported template invoked through a `crate::` path
    "crate_path": (
        ['#[macro_export] macro_rules! mP{K} { ($p:expr) => { $crate::H::new($p) } }'],
        'out.push(H::new("a"));',
        'out.push(crate::mP{K}!("a"));',
    ),
    # a template whose body is a std STATEMENT macro whose tokens construct (R204's a6b) — the H is
    # a temporary, dropped in frame at the end of the statement rather than stored in `out`.
    "std_stmt_tokens": (
        ['macro_rules! mLg{K} { ($p:expr) => {{ let s = format!("{}", H::new($p).p.len()); let _ = s; }} }'],
        '{ let s{K} = format!("{}", H::new("a").p.len()); let _ = s{K}; }',
        'mLg{K}!("a");',
    ),
    # a match-arm-list template (R203's A12 class)
    "match_arms": (
        ['macro_rules! mM{K} { ($n:expr, $($pat:pat => $e:expr),*) => { match $n { $($pat => $e),* } } }'],
        'out.push(match m { 0 => H::new("a"), _ => H::new("c") });',
        'out.push(mM{K}!(m, 0 => H::new("a"), _ => H::new("c")));',
    ),
}

# --- Where the fragment sits. `{S}` = the fragment's statements, `{K}` = the pair suffix. --------
# The `?` shapes are IN-PLACE (`EXPR?`), which is where the macro-invisibility regressions lived.
CONTEXTS = {
    "straight":    '{S}',
    "q_block":     '{ {S} gen(n) }?;',
    "q_nested":    '{ { {S} } gen(n) }?;',
    "q_macroid":   'idm!({ {S} gen(n) })?;',
    "q_closure":   'run_cb(|| { {S} gen(n).map(|_| ()) })?;',
    "q_match":     '(match m { _ => { {S} gen(n) } })?;',
    "q_if":        '(if m > 0 { {S} gen(n) } else { Ok(0u32) })?;',
    "q_tryforeach": '[0u32].iter().try_for_each(|_x{K}| { {S} gen(n).map(|_| ()) })?;',
    "q_hoisted":   'let t{K} = { {S} gen(n) }; t{K}?;',
    "loop_for":    'let mut c{K} = 2u32; for _i{K} in 0..9u32 { let _v{K} = tick(&mut c{K})?; {S} }',
    "loop_while":  'let mut c{K} = 2u32; while c{K} < 9 { {S} let _v{K} = tick(&mut c{K})?; }',
}


def render(name, shape, k, variant):
    """variant False = the DIRECT spelling, True = the MACRO spelling."""
    _items, direct, macro = MACRO_FORMS[shape["macro"]]
    frag = (macro if variant else direct).replace("{K}", k)
    body = CONTEXTS[shape["context"]].replace("{S}", frag).replace("{K}", k)
    ret, end = ENDINGS[shape["ending"]]
    return (
        "#[allow(unused, clippy::all)]\n"
        "pub fn %s(n: u32, m: u32) -> Result<%s, ()> {\n"
        "    let mut out: Vec<H> = Vec::new();\n"
        "    %s\n    %s\n}\n" % (name, ret, body, end.replace("{K}", k))
    )


def build(out, seed, effect, shapes):
    pairs, items = [], []
    for i, shape in enumerate(shapes):
        k = "%03d" % i
        pairs.append(("d%s" % k, "m%s" % k, shape))
        items += [x.replace("{K}", k) for x in MACRO_FORMS[shape["macro"]][0]]
    # Every pair here is an EXACT equivalence: one program, two spellings, so both directions of a
    # disagreement are defects and both are reported.
    emit(out, seed, effect, EFFECT_LEAVES[effect], pairs, "macro", crate="qgen",
         extra_items=items, gen_name="gen_macro.py", render_fn=render, equal_fn=lambda sh: True)


def all_shapes():
    """Every point of the shape space, in a fixed order — see gen_q.all_shapes()."""
    return [{"macro": mf, "context": cx, "ending": en}
            for mf in MACRO_FORMS for cx in CONTEXTS for en in ENDINGS]


def main():
    if sys.argv[1] == "--all":
        build(sys.argv[2], 0, "Fs", all_shapes())
        return
    seed = int(sys.argv[1])
    out = sys.argv[2]
    rng = random.Random(seed)
    effect = rng.choice(sorted(EFFECT_LEAVES))
    n = rng.randint(6, 10)
    shapes = []
    for i in range(n):
        shapes.append({"macro": rng.choice(list(MACRO_FORMS)),
                       "context": rng.choice(list(CONTEXTS)),
                       "ending": rng.choice(list(ENDINGS))})
    build(out, seed, effect, shapes)


if __name__ == "__main__":
    main()
