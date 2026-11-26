//! SOUNDNESS R1004 — ITEM-POSITION invocations of a `macro_rules!` defined in the SAME FILE are EXPANDED
//! before Pass A, so the functions, impls and modules they declare are ordinary units.
//!
//! THE DEFECT. `collect_decls` skips an item-position macro invocation (`syn` leaves its body opaque), so
//! a function a local macro DECLARES had no unit, and a call to it by a bare or `super::` path resolved to
//! nothing and read PURE. R128 discloses the `crate::<module>::<name>` spelling with a `macro:` hedge and
//! states the bare and relative spellings as an under-report (its boundaries 2 and 3). Measured: wasm-
//! bindgen's `externs! { extern "C" { fn __wbindgen_describe(v: u32) -> (); } }`, which off-wasm expands
//! to `unsafe extern "C" fn __wbindgen_describe(..) { panic!(..) }`, leaves `describe::inform`
//! (`super::__wbindgen_describe(a)`) with no edge at all; with an effectful stub the caller is silent
//! (executed fixture, scratchpad `rustagent-v041/fx1004`).
//!
//! A RESOLUTION, NOT A HEDGE: the generated functions are the program's own, so expanding the macro the
//! way rustc does is the authority (§G) and every downstream route — `#[cfg]` filtering, `by_tail2`, the
//! call resolver — sees them as written. A hedge would have charged `Unknown` to 21 wasm-bindgen rows
//! whose real host-target bodies are `panic!` stubs.
//!
//! FUNCTIONS ONLY, BESIDE THE INVOCATION. Only the free `fn`s and `extern` blocks an expansion yields are
//! spliced, after the invocation, which stays: generated `impl`s would resolve dispatch onto their methods
//! and that is a different change with its own losses (see `Ctx::splice`).
//!
//! WHAT IS EXPANDED, AND THE REFUSALS THAT KEEP IT FROM GUESSING. Macro-by-example as rustc defines it:
//! the FIRST arm whose matcher matches the whole invocation, fragments parsed by `syn` (`ident` `tt`
//! `expr` `ty` `path` `pat` `pat_param` `item` `block` `meta` `vis` `literal` `lifetime`), repetitions
//! `$( .. ) sep? * + ?` with nested depth, `$crate` → `crate`. Anything outside that — a definition this
//! file spells twice (`#[cfg]` twins), a `stmt` fragment, a matcher or transcriber this module cannot
//! read, an expansion that does not parse as items, a recursion deeper than `MAX_DEPTH` — leaves the
//! invocation EXACTLY as it was, so R128's evidence and hedge still apply to it.
//!
//! A macro defined in ANOTHER file of the crate (`#[macro_use] mod mac;`) is expanded too, when the crate
//! defines that name once — identically in every file that defines it (`CrateMacros`) — and the invoking
//! file does not define it. That was the residual of R1004: a bare call to the generated `fn` from inside
//! the invoking module had no unit and read ABSENT, while its `super::`/`crate::` spellings disclosed
//! `macro:`. The per-file cache keys on one file's bytes, so the table's digest is folded into every file's
//! key (`scan.rs`) — a definition read from elsewhere cannot go stale under a warm entry.
//!
//! I1 STAGE 3 — WHERE A DEFINITION COMES FROM AND WHERE AN INVOCATION SITS ARE READ AS DATA, NOT POSITIONS. A definition enters
//! textual scope where it is met, whether written in source or EMITTED by an expansion (R1784: `Ctx::emitted`, `scope`; the
//! crate table reads a file's emitted definitions from its own splice, `file_macro_defs`; the collector keeps one a
//! statement-position expansion emits, `collector::walk_expansion`). An invocation at item, impl-item and trait-item position
//! is spliced as before; one in a BODY (fn, method, trait default, const, static, closure, nested block) has its emitted
//! `impl`s hoisted beside the host item (R1813, `Ctx::hoist_body_impls`), since an `impl` is not scoped to its block.

use proc_macro2::{Delimiter, Group, Ident, Spacing, TokenStream, TokenTree};
use std::collections::HashMap;

const MAX_DEPTH: usize = 8;
const MAX_EXPANSIONS: usize = 4096;
const MAX_OUTCOMES: usize = 2048;

#[derive(Clone, Copy, Debug, PartialEq)]
enum Frag {
    Ident,
    Tt,
    Expr,
    Ty,
    Path,
    Pat,
    PatParam,
    Item,
    Block,
    Meta,
    Vis,
    Literal,
    Lifetime,
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum RepOp {
    Star,
    Plus,
    Opt,
}

#[derive(Clone, Debug)]
enum M {
    Tok(TokenTree),
    Group(Delimiter, Vec<M>),
    Var(String, Frag),
    Rep(Vec<M>, Option<Vec<TokenTree>>, RepOp),
}

#[derive(Clone, Debug)]
enum Binding {
    One(TokenStream),
    Seq(Vec<Binding>),
}

type Binds = HashMap<String, Binding>;

struct Arm {
    matcher: Vec<M>,
    body: TokenStream,
}

fn frag_of(s: &str) -> Option<Frag> {
    Some(match s {
        "ident" => Frag::Ident,
        "tt" => Frag::Tt,
        "expr" | "expr_2021" => Frag::Expr,
        "ty" => Frag::Ty,
        "path" => Frag::Path,
        "pat" => Frag::Pat,
        "pat_param" => Frag::PatParam,
        "item" => Frag::Item,
        "block" => Frag::Block,
        "meta" => Frag::Meta,
        "vis" => Frag::Vis,
        "literal" => Frag::Literal,
        "lifetime" => Frag::Lifetime,
        // `stmt` excludes its trailing `;` in rustc and `syn::Stmt` consumes it — refused rather than
        // matched one token off.
        _ => return None,
    })
}

fn is_punct(t: &TokenTree, c: char) -> bool {
    matches!(t, TokenTree::Punct(p) if p.as_char() == c)
}

fn rep_op(t: &TokenTree) -> Option<RepOp> {
    match t {
        TokenTree::Punct(p) if p.as_char() == '*' => Some(RepOp::Star),
        TokenTree::Punct(p) if p.as_char() == '+' => Some(RepOp::Plus),
        TokenTree::Punct(p) if p.as_char() == '?' => Some(RepOp::Opt),
        _ => None,
    }
}

/// The matcher of one arm. `None` for anything this module does not read.
fn parse_matcher(ts: TokenStream) -> Option<Vec<M>> {
    let toks: Vec<TokenTree> = ts.into_iter().collect();
    let mut out = Vec::new();
    let mut i = 0;
    while i < toks.len() {
        let t = &toks[i];
        if is_punct(t, '$') {
            match toks.get(i + 1)? {
                TokenTree::Ident(name) => {
                    // `$name:frag`
                    if !is_punct(toks.get(i + 2)?, ':') {
                        return None;
                    }
                    let TokenTree::Ident(f) = toks.get(i + 3)? else { return None };
                    out.push(M::Var(name.to_string(), frag_of(&f.to_string())?));
                    i += 4;
                }
                TokenTree::Group(g) if g.delimiter() == Delimiter::Parenthesis => {
                    let inner = parse_matcher(g.stream())?;
                    let (sep, op, used) = rep_tail(&toks[i + 2..])?;
                    out.push(M::Rep(inner, sep, op));
                    i += 2 + used;
                }
                _ => return None,
            }
            continue;
        }
        match t {
            TokenTree::Group(g) => out.push(M::Group(g.delimiter(), parse_matcher(g.stream())?)),
            _ => out.push(M::Tok(t.clone())),
        }
        i += 1;
    }
    Some(out)
}

/// After `$( .. )`: an optional separator, then the operator. Returns (sep, op, tokens used).
///
/// A separator is ONE rustc token, which `proc_macro2` splits into joint puncts when it is a multi-character
/// operator: SOUNDNESS R1748 (a) — serde_core's `declare_error_trait!` repeats `$($supertrait:ident)::+`, whose
/// separator is the path separator `::`, and a one-tree separator refused the whole macro (so the traits it declares
/// were never seen). The two-character operators rustc lexes as one token are read as one separator.
/// `CANDOR_R1748SEP_OFF=1` restores the one-tree reading.
fn rep_tail(rest: &[TokenTree]) -> Option<(Option<Vec<TokenTree>>, RepOp, usize)> {
    let first = rest.first()?;
    if let Some(op) = rep_op(first) {
        // `$(..)?` takes no separator; `$(..)*`/`+` directly.
        return Some((None, op, 1));
    }
    if matches!(first, TokenTree::Group(_)) {
        return None;
    }
    if let (TokenTree::Punct(a), Some(TokenTree::Punct(b))) = (first, rest.get(1)) {
        let pair: String = [a.as_char(), b.as_char()].iter().collect();
        if a.spacing() == Spacing::Joint && !candor_report::env::switch("CANDOR_R1748SEP_OFF") && matches!(pair.as_str(), "::" | "=>" | "->" | ".." | "==" | "!=" | "<=" | ">=" | "&&" | "||") {
            if let Some(op) = rest.get(2).and_then(rep_op) {
                if op == RepOp::Opt {
                    return None;
                }
                SEP2_USED.with(|c| c.set(true));
                return Some((Some(vec![first.clone(), rest[1].clone()]), op, 3));
            }
        }
    }
    let op = rep_op(rest.get(1)?)?;
    if op == RepOp::Opt {
        return None; // `?` admits no separator
    }
    Some((Some(vec![first.clone()]), op, 2))
}

fn parse_arms(body: &TokenStream) -> Option<Vec<Arm>> {
    let toks: Vec<TokenTree> = body.clone().into_iter().collect();
    let mut arms = Vec::new();
    let mut i = 0;
    while i < toks.len() {
        let TokenTree::Group(m) = &toks[i] else { return None };
        if !(is_punct(toks.get(i + 1)?, '=') && is_punct(toks.get(i + 2)?, '>')) {
            return None;
        }
        let TokenTree::Group(b) = toks.get(i + 3)? else { return None };
        arms.push(Arm { matcher: parse_matcher(m.stream())?, body: b.stream() });
        i += 4;
        if toks.get(i).is_some_and(|t| is_punct(t, ';')) {
            i += 1;
        }
    }
    (!arms.is_empty()).then_some(arms)
}

fn tok_eq(a: &TokenTree, b: &TokenTree) -> bool {
    match (a, b) {
        (TokenTree::Ident(x), TokenTree::Ident(y)) => x == y,
        (TokenTree::Punct(x), TokenTree::Punct(y)) => x.as_char() == y.as_char(),
        (TokenTree::Literal(x), TokenTree::Literal(y)) => x.to_string() == y.to_string(),
        _ => false,
    }
}

/// How many leading token trees of `toks` form one `frag`. `None` when they do not.
fn parse_frag(frag: Frag, toks: &[TokenTree]) -> Option<usize> {
    use syn::parse::{ParseStream, Parser};
    let first = toks.first();
    match frag {
        Frag::Tt => return first.map(|_| 1),
        Frag::Ident => {
            return match first? {
                TokenTree::Ident(i) if i != "_" => Some(1),
                _ => None,
            }
        }
        Frag::Lifetime => {
            return match (first?, toks.get(1)) {
                (TokenTree::Punct(p), Some(TokenTree::Ident(_))) if p.as_char() == '\'' && p.spacing() == Spacing::Joint => Some(2),
                _ => None,
            }
        }
        Frag::Literal => {
            return match first? {
                TokenTree::Literal(_) => Some(1),
                TokenTree::Ident(i) if i == "true" || i == "false" => Some(1),
                TokenTree::Punct(p) if p.as_char() == '-' => matches!(toks.get(1), Some(TokenTree::Literal(_))).then_some(2),
                _ => None,
            }
        }
        _ => {}
    }
    let ts: TokenStream = toks.iter().cloned().collect();
    let total = toks.len();
    let parser = move |input: ParseStream| -> syn::Result<usize> {
        match frag {
            Frag::Expr => {
                input.parse::<syn::Expr>()?;
            }
            Frag::Ty => {
                input.parse::<syn::Type>()?;
            }
            Frag::Path => {
                input.parse::<syn::Path>()?;
            }
            Frag::Pat => {
                syn::Pat::parse_multi_with_leading_vert(input)?;
            }
            Frag::PatParam => {
                syn::Pat::parse_single(input)?;
            }
            Frag::Item => {
                input.parse::<syn::Item>()?;
            }
            Frag::Block => {
                input.parse::<syn::Block>()?;
            }
            Frag::Meta => {
                input.parse::<syn::Meta>()?;
            }
            Frag::Vis => {
                input.parse::<syn::Visibility>()?;
            }
            _ => unreachable!(),
        }
        let rest: TokenStream = input.parse()?;
        Ok(rest.into_iter().count())
    };
    let left = parser.parse2(ts).ok()?;
    let used = total.checked_sub(left)?;
    // A fragment that consumed NOTHING is only `vis` (which may be empty); every other one is a miss.
    (used > 0 || frag == Frag::Vis).then_some(used)
}

/// Every way `ms` can match a prefix of `inp[i..]`, as (end, bindings). Bounded by `MAX_OUTCOMES`;
/// `None` when the bound is hit, which refuses the whole expansion rather than picking one.
fn match_seq(ms: &[M], inp: &[TokenTree], i: usize, b: Binds) -> Option<Vec<(usize, Binds)>> {
    let Some((m, rest)) = ms.split_first() else { return Some(vec![(i, b)]) };
    let mut outs: Vec<(usize, Binds)> = Vec::new();
    let push = |v: Vec<(usize, Binds)>, outs: &mut Vec<(usize, Binds)>| -> Option<()> {
        outs.extend(v);
        (outs.len() <= MAX_OUTCOMES).then_some(())
    };
    match m {
        M::Tok(t) => {
            if inp.get(i).is_some_and(|x| tok_eq(t, x)) {
                push(match_seq(rest, inp, i + 1, b)?, &mut outs)?;
            }
        }
        M::Group(d, inner) => {
            if let Some(TokenTree::Group(g)) = inp.get(i) {
                if g.delimiter() == *d {
                    let sub: Vec<TokenTree> = g.stream().into_iter().collect();
                    for (end, bb) in match_seq(inner, &sub, 0, b)? {
                        if end == sub.len() {
                            push(match_seq(rest, inp, i + 1, bb)?, &mut outs)?;
                        }
                    }
                }
            }
        }
        M::Var(name, frag) => {
            if let Some(n) = parse_frag(*frag, &inp[i..]) {
                let mut cap: TokenStream = inp[i..i + n].iter().cloned().collect();
                // A parsed nonterminal is ONE opaque unit in the expansion (rustc's invisible group), so
                // `$e * 2` over `$e = 1 + 1` keeps its precedence.
                if !matches!(frag, Frag::Tt | Frag::Ident | Frag::Lifetime | Frag::Literal | Frag::Vis) {
                    cap = TokenStream::from(TokenTree::Group(Group::new(Delimiter::None, cap)));
                }
                let mut bb = b;
                if bb.insert(name.clone(), Binding::One(cap)).is_some() {
                    return None; // a name bound twice in one matcher — not valid macro_rules
                }
                push(match_seq(rest, inp, i + n, bb)?, &mut outs)?;
            }
        }
        M::Rep(body, sep, op) => {
            let names = var_names(body);
            // iterations: a list of (end, per-iteration binds collected so far)
            let mut frontier: Vec<(usize, Vec<Binds>)> = vec![(i, Vec::new())];
            let max_iter = if *op == RepOp::Opt { 1 } else { usize::MAX };
            let mut done: Vec<(usize, Vec<Binds>)> = Vec::new();
            while let Some((pos, iters)) = frontier.pop() {
                done.push((pos, iters.clone()));
                if done.len() > MAX_OUTCOMES {
                    return None;
                }
                if iters.len() >= max_iter {
                    continue;
                }
                let mut start = pos;
                if !iters.is_empty() {
                    if let Some(s) = sep {
                        if !s.iter().enumerate().all(|(k, st)| inp.get(pos + k).is_some_and(|x| tok_eq(st, x))) {
                            continue;
                        }
                        start = pos + s.len();
                    }
                }
                for (end, ib) in match_seq(body, inp, start, Binds::new())? {
                    if end == pos {
                        continue; // an empty iteration would never terminate
                    }
                    let mut it2 = iters.clone();
                    it2.push(ib);
                    frontier.push((end, it2));
                }
            }
            for (pos, iters) in done {
                if *op == RepOp::Plus && iters.is_empty() {
                    continue;
                }
                let mut bb = b.clone();
                for n in &names {
                    let seq: Vec<Binding> = iters.iter().filter_map(|ib| ib.get(n).cloned()).collect();
                    if seq.len() != iters.len() {
                        return None;
                    }
                    if bb.insert(n.clone(), Binding::Seq(seq)).is_some() {
                        return None;
                    }
                }
                push(match_seq(rest, inp, pos, bb)?, &mut outs)?;
            }
        }
    }
    Some(outs)
}

fn var_names(ms: &[M]) -> Vec<String> {
    let mut out = Vec::new();
    for m in ms {
        match m {
            M::Var(n, _) => out.push(n.clone()),
            M::Group(_, inner) | M::Rep(inner, _, _) => out.extend(var_names(inner)),
            M::Tok(_) => {}
        }
    }
    out
}

/// The `$name`s a transcriber fragment mentions (not `$crate`).
fn used_names(ts: &TokenStream, out: &mut Vec<String>) {
    let toks: Vec<TokenTree> = ts.clone().into_iter().collect();
    for (k, t) in toks.iter().enumerate() {
        match t {
            TokenTree::Group(g) => used_names(&g.stream(), out),
            TokenTree::Punct(p) if p.as_char() == '$' => {
                if let Some(TokenTree::Ident(n)) = toks.get(k + 1) {
                    if n != "crate" {
                        out.push(n.to_string());
                    }
                }
            }
            _ => {}
        }
    }
}

fn lookup<'a>(b: &'a Binds, name: &str, idx: &[usize]) -> Option<&'a Binding> {
    let mut cur = b.get(name)?;
    for &k in idx {
        match cur {
            Binding::Seq(v) => cur = v.get(k)?,
            Binding::One(_) => return Some(cur), // a shallower binding repeats unchanged
        }
    }
    Some(cur)
}

fn transcribe(ts: &TokenStream, b: &Binds, idx: &[usize]) -> Option<TokenStream> {
    let toks: Vec<TokenTree> = ts.clone().into_iter().collect();
    let mut out = TokenStream::new();
    let mut i = 0;
    while i < toks.len() {
        let t = &toks[i];
        if is_punct(t, '$') {
            match toks.get(i + 1)? {
                TokenTree::Ident(n) if n == "crate" => {
                    out.extend([TokenTree::Ident(Ident::new("crate", n.span()))]);
                    i += 2;
                }
                // I1 stage 3 (R1784) — a `$name` the arm does not bind is copied through as written, as rustc does: it is the
                // metavariable of a `macro_rules!` this transcriber DEFINES (`macro_rules! inner { ($t:ident) => { .. $t .. } }`),
                // read when that definition is invoked. Refusing it made every macro-defining macro with a metavariable in
                // its inner definition unexpandable. `CANDOR_I1S3_MDEF_OFF=1` restores the refusal.
                TokenTree::Ident(n) if !b.contains_key(&n.to_string()) && !mdef_off() => {
                    out.extend([t.clone(), TokenTree::Ident(n.clone())]);
                    i += 2;
                }
                TokenTree::Ident(n) => {
                    match lookup(b, &n.to_string(), idx)? {
                        Binding::One(ts) => out.extend(ts.clone()),
                        Binding::Seq(_) => return None, // used at a shallower depth than it was bound
                    }
                    i += 2;
                }
                TokenTree::Group(g) if g.delimiter() == Delimiter::Parenthesis => {
                    let (sep, _op, used) = rep_tail(&toks[i + 2..])?;
                    let mut names = Vec::new();
                    used_names(&g.stream(), &mut names);
                    let mut count: Option<usize> = None;
                    for n in &names {
                        if let Some(Binding::Seq(v)) = lookup(b, n, idx) {
                            match count {
                                None => count = Some(v.len()),
                                Some(c) if c != v.len() => return None,
                                _ => {}
                            }
                        }
                    }
                    let count = count?; // a repetition that repeats no bound sequence
                    for k in 0..count {
                        if k > 0 {
                            if let Some(s) = &sep {
                                out.extend(s.iter().cloned());
                            }
                        }
                        let mut idx2 = idx.to_vec();
                        idx2.push(k);
                        out.extend(transcribe(&g.stream(), b, &idx2)?);
                    }
                    i += 2 + used;
                }
                _ => return None,
            }
            continue;
        }
        match t {
            TokenTree::Group(g) => {
                let mut ng = Group::new(g.delimiter(), transcribe(&g.stream(), b, idx)?);
                ng.set_span(g.span());
                out.extend([TokenTree::Group(ng)]);
            }
            _ => out.extend([t.clone()]),
        }
        i += 1;
    }
    Some(out)
}

/// Expand one invocation: the first arm that matches the WHOLE input, transcribed.
fn expand(arms: &[Arm], input: &TokenStream) -> Option<TokenStream> {
    let inp: Vec<TokenTree> = input.clone().into_iter().collect();
    for arm in arms {
        let outs = match_seq(&arm.matcher, &inp, 0, Binds::new())?;
        let full: Vec<&(usize, Binds)> = outs.iter().filter(|(e, _)| *e == inp.len()).collect();
        match full.len() {
            0 => continue,
            1 => return transcribe(&arm.body, &full[0].1, &[]),
            _ => return None, // ambiguous — rustc rejects it; never pick one
        }
    }
    None
}

/// SOUNDNESS R1144 — one EXPRESSION/STATEMENT-position invocation of a `macro_rules!` whose arms are `def` (the
/// definition's tokens as `local_macros` / `body_macros` record them): macro-by-example exactly as for an item-position
/// invocation (`expand`: the first arm that matches the whole input, `None` on anything this module does not read).
pub(crate) fn expand_invocation(def: &str, input: &TokenStream) -> Option<TokenStream> {
    let t: TokenStream = def.parse().ok()?;
    let arms = parse_arms(&t)?;
    expand(&arms, input)
}

/// The file's `macro_rules!` definitions by name — a name defined more than once (a `#[cfg]` twin, a
/// textual redefinition) is dropped: which one an invocation sees depends on order and configuration.
fn collect_defs(
    items: &[syn::Item],
    imported: &std::collections::HashSet<String>,
    defs: &mut HashMap<String, Option<TokenStream>>,
) {
    for it in items {
        match it {
            syn::Item::Macro(m) if m.mac.path.is_ident("macro_rules") => {
                if let Some(name) = &m.ident {
                    // I1 stage 3 — an emitted definition (only in `file_macro_defs`' spliced copy) is keyed apart
                    // (`model::EMITTED_MACRO_PREFIX`), read only for a name no source definition has.
                    let emitted = m.attrs.iter().any(|a| a.path().is_ident(crate::model::MBE_EMITTED_DEF_ATTR));
                    let name =
                        if emitted { format!("{}{name}", crate::model::EMITTED_MACRO_PREFIX) } else { name.to_string() };
                    let e = defs.entry(name.clone()).or_insert(Some(m.mac.tokens.clone()));
                    if e.as_ref().is_some_and(|prev| prev.to_string() != m.mac.tokens.to_string()) {
                        *e = None;
                    }
                    // SOUNDNESS R1541 / I1 step 1 — a `#[cfg]`-gated definition of a name this FILE ALSO IMPORTS through a
                    // `use` is not the binding rustc picks where the cfg is off (deranged-0.5.8's `#[cfg(docsrs)] macro_rules!
                    // int` beside `pub use deranged_macros::int`): refused exactly as a cfg twin is. A gated definition with no
                    // competing import is kept — its invocations are gated with it (futures-util's `#[cfg(feature = "io")]
                    // macro_rules! delegate_async_read`), and refusing those deleted 184 rows on the 181-entry A/B.
                    // `CANDOR_I1_BIND_OFF=1` restores the old binding.
                    if crate::lang::has_cfg(&m.attrs)
                        && (imported.contains(&name) || imported.contains("*"))
                        && !candor_report::env::switch("CANDOR_I1_BIND_OFF")
                    {
                        if candor_report::env::trace("CANDOR_I1_INSTR") {
                            eprintln!("I1REFUSE\tcfg-def\t{name}"); // §E1 REACH COUNTER
                        }
                        *e = None;
                    }
                }
            }
            syn::Item::Mod(md) => {
                if let Some((_, inner)) = &md.content {
                    collect_defs(inner, imported, defs);
                }
            }
            _ => {}
        }
    }
}

/// SOUNDNESS R1004 (the cross-file residual) — the crate's `macro_rules!` definitions by name, as token
/// TEXT (a `TokenStream` is not `Send`, and the round-1 parse is parallel): `Some` when every file that
/// defines the name defines it identically, `None` when two differ (cfg twins across files, a textual
/// redefinition — which one an invocation sees depends on order and configuration, so it is refused).
pub(crate) type CrateMacros = HashMap<String, Option<String>>;

/// This file's definitions, for `merge_crate_macros`.
pub(crate) fn file_macro_defs(items: &[syn::Item]) -> Vec<(String, Option<String>)> {
    // I1 stage 3 (R1784) — a definition another definition's EXPANSION emits (`m_defx!();` in `defs.rs`, invoked from a
    // `#[macro_use] mod defs;` parent) is the crate's too: read from this file's own splice, which emits it. Only where some
    // `macro_rules!` body here writes `macro_rules` itself (the cheap test), and with no other file's definitions (one level).
    let spliced;
    let items = if !mdef_off() && defines_a_definer(items) {
        let mut c = items.to_vec();
        splice_local_macros(&mut c, &CrateMacros::new());
        spliced = c;
        &spliced[..]
    } else {
        items
    };
    let mut imported = std::collections::HashSet::new();
    if !candor_report::env::switch("CANDOR_I1_BIND_OFF") {
        imported_names(items, &mut imported);
    }
    let mut defs = HashMap::new();
    collect_defs(items, &imported, &mut defs);
    let mut v: Vec<(String, Option<String>)> = defs.into_iter().map(|(k, v)| (k, v.map(|t| t.to_string()))).collect();
    // I1 stage 2 — EVERY body of every definition, cfg twins included, under `ALL_BODIES_KEY` + name: never expanded from,
    // read only to say what hidden text MAY define (`Ctx::read_hidden`), where a twice-defined wrapper's bodies are both real.
    fn walk(items: &[syn::Item], v: &mut Vec<(String, Option<String>)>) {
        for it in items {
            match it {
                syn::Item::Macro(m) if m.mac.path.is_ident("macro_rules") => {
                    if let Some(n) = &m.ident {
                        v.push((format!("{ALL_BODIES_KEY}{n}"), Some(m.mac.tokens.to_string())));
                    }
                }
                syn::Item::Mod(md) => {
                    if let Some((_, inner)) = &md.content {
                        walk(inner, v);
                    }
                }
                _ => {}
            }
        }
    }
    walk(items, &mut v);
    v
}

/// I1 stage 3 — the crate-table key of a definition an expansion emitted (`model::EMITTED_MACRO_PREFIX`).
fn emitted_key(name: &str) -> String {
    format!("{}{name}", crate::model::EMITTED_MACRO_PREFIX)
}

/// I1 stage 3 — does some `macro_rules!` body in these items (at any module depth) itself write `macro_rules`?
pub(crate) fn defines_a_definer(items: &[syn::Item]) -> bool {
    items.iter().any(|it| match it {
        syn::Item::Macro(m) if m.mac.path.is_ident("macro_rules") => m.mac.tokens.to_string().contains("macro_rules"),
        syn::Item::Mod(md) => md.content.as_ref().is_some_and(|(_, inner)| defines_a_definer(inner)),
        _ => false,
    })
}

/// The macro bodies `Ctx::read_hidden` may expand with: this file's (`Ctx::bodies`) and the crate's (`CrateMacros`, all bodies under
/// `ALL_BODIES_KEY`). A plain lookup rather than a closure, so the reading makes no indirect call.
#[derive(Default)]
struct BodyTable<'a> {
    /// I1 stage 2d — the bodies in textual scope (`Ctx::scope_bodies`), consulted first; absent from the visible splice's table.
    scoped: Option<&'a HashMap<String, Vec<TokenStream>>>,
    bodies: Option<&'a HashMap<String, Vec<TokenStream>>>,
    xdefs: Option<&'a CrateMacros>,
}

impl BodyTable<'_> {
    /// `None`: not a local macro; `Some(None)`: local, but no body readable; `Some(Some(bodies))`.
    fn get(&self, n: &str) -> Option<Option<Vec<TokenStream>>> {
        if let Some(bs) = self.scoped.and_then(|b| b.get(n)).filter(|v| !v.is_empty()) {
            if candor_report::env::trace("CANDOR_I1_INSTR") && self.bodies.and_then(|b| b.get(n)).is_some_and(|all| all.len() > bs.len()) {
                eprintln!("I1S2DSCOPEBODY\t{n}"); // §E1 REACH COUNTER: a same-named body out of textual scope left unread
            }
            return Some(Some(bs.clone()));
        }
        if let Some(bs) = self.bodies.and_then(|b| b.get(n)) {
            return Some(Some(bs.clone()));
        }
        let x = self.xdefs?;
        // Another file's definitions — ALL of them, cfg twins included, so a twice-defined wrapper is read, not opaque.
        match x.get(&format!("{ALL_BODIES_KEY}{n}")) {
            Some(Some(all)) => Some(all.split('\u{0}').map(|b| b.parse::<TokenStream>().ok()).collect()),
            _ => match x.get(n) {
                Some(Some(t)) => Some(t.parse::<TokenStream>().ok().map(|ts| vec![ts])),
                Some(None) => Some(None),
                None => None,
            },
        }
    }
}

/// The key prefix of the all-bodies entries in `CrateMacros` (no macro name can begin with a control character).
pub(crate) const ALL_BODIES_KEY: &str = "\u{1}all:";

pub(crate) fn merge_crate_macros(per_file: impl IntoIterator<Item = Vec<(String, Option<String>)>>) -> CrateMacros {
    let mut out: CrateMacros = HashMap::new();
    for defs in per_file {
        for (name, body) in defs {
            if name.starts_with(ALL_BODIES_KEY) {
                // a union of bodies, separated by NUL (a token stream never holds one)
                let e = out.entry(name).or_insert(Some(String::new()));
                if let (Some(acc), Some(b)) = (e.as_mut(), body) {
                    if !acc.split('\u{0}').any(|x| x == b) {
                        if !acc.is_empty() {
                            acc.push('\u{0}');
                        }
                        acc.push_str(&b);
                    }
                }
                continue;
            }
            match out.get(&name) {
                None => {
                    out.insert(name, body);
                }
                Some(prev) if *prev == body => {}
                Some(_) => {
                    out.insert(name, None);
                }
            }
        }
    }
    out
}

/// A stable digest of the table, folded into every file's cache key when it is non-empty: a file's
/// expansion now depends on bytes in ANOTHER file, which the per-file content hash alone cannot see.
pub(crate) fn crate_macros_digest(m: &CrateMacros) -> String {
    let mut keys: Vec<&String> = m.keys().collect();
    keys.sort();
    let mut s = String::new();
    for k in keys {
        s.push_str(k);
        s.push('\u{1f}');
        s.push_str(m[k].as_deref().unwrap_or("\u{0}"));
        s.push('\u{1e}');
    }
    let mut h: u64 = 0xcbf29ce484222325;
    for b in s.as_bytes() {
        h ^= *b as u64;
        h = h.wrapping_mul(0x100000001b3);
    }
    format!("{h:016x}")
}

/// Every token of `ts` re-spanned to `span` (an expansion from a definition parsed out of another file's
/// TEXT carries spans into that text; re-spanning to the invocation makes its units' locations the
/// invocation's, and keeps every span on this thread's source map).
fn respan_to(ts: TokenStream, span: proc_macro2::Span) -> TokenStream {
    ts.into_iter()
        .map(|tt| match tt {
            TokenTree::Group(g) => {
                let mut ng = Group::new(g.delimiter(), respan_to(g.stream(), span));
                ng.set_span(span);
                TokenTree::Group(ng)
            }
            TokenTree::Ident(mut t) => {
                t.set_span(span);
                TokenTree::Ident(t)
            }
            TokenTree::Punct(mut t) => {
                t.set_span(span);
                TokenTree::Punct(t)
            }
            TokenTree::Literal(mut t) => {
                t.set_span(span);
                TokenTree::Literal(t)
            }
        })
        .collect()
}

struct Ctx<'x> {
    arms: HashMap<String, Option<std::rc::Rc<Vec<Arm>>>>,
    defs: HashMap<String, Option<TokenStream>>,
    /// R1004 — the crate-wide table, consulted only for a name THIS file does not define.
    xdefs: &'x CrateMacros,
    budget: usize,
    /// SOUNDNESS R1156 (item position) — the `macro_rules!` definitions in TEXTUAL scope at the item being spliced:
    /// `Some(tokens)` for an ungated one, `None` once a `#[cfg]`-gated definition of the name is in scope (no
    /// textual order picks between cfg twins). Consulted only for a name this file defines more than once.
    scope: HashMap<String, Option<String>>,
    /// I1 step 1 — names a non-crate-rooted `use` in this file binds (see `local_name`).
    imported: std::collections::HashSet<String>,
    /// I1 stage 2 — the invocations kept beside a PARTIAL splice, at any depth (a partial `cfg_trace!` nested in `cfg_rt!`
    /// is kept in the OUTER output): their spliced items stand in the module, so their text is not hidden-twin evidence
    /// (`mark_hidden_twins`); what they did not splice is kept as its own item or marker.
    partial: std::collections::HashSet<String>,
    /// I1 stage 2 — EVERY `macro_rules!` body this file writes, by name, cfg twins included (`read_hidden`).
    bodies: HashMap<String, Vec<TokenStream>>,
    /// I1 stage 2d — the `macro_rules!` bodies in TEXTUAL scope at the module being spliced, cfg twins included, saved and
    /// restored around a child module exactly as `scope` is. `read_hidden` reads a name's bodies from here when it has any,
    /// so a sibling module's same-named macro is not bound to an invocation that cannot reach it (Fable re-check 2c-2 RC4.3,
    /// tokio's `impl_traits!` in `mod sys` read as `mod windows`'s hidden text).
    scope_bodies: HashMap<String, Vec<TokenStream>>,
    /// I1 stage 3 (R1784) — the names of `macro_rules!` definitions an EXPANSION emitted (a macro-defining macro). Such a
    /// name binds only where its definition is in TEXTUAL scope (`scope`), never through the file-wide once-defined table.
    emitted: std::collections::HashSet<String>,
    /// SOUNDNESS R1899 — the `TOOLCHAIN_BANG_MACROS` names whose BARE spelling may not bind the toolchain's macro in this
    /// file: a non-crate-rooted `use` imports the name (or a glob / `#[macro_use] extern crate` may), or a local or crate-wide
    /// `macro_rules!` defines it. `is_hidden_text` and `read_hidden`'s toolchain arm read the path alone; for these names a
    /// bare invocation is another macro's text, read as any other.
    ///
    /// R-BARE-TC (rust-macropath L3) — read PER MODULE: `splice` swaps in the set of the list it is splicing (`tc_names_for`) and
    /// puts the enclosing list's back, and the module-arm readers (`holds_arm_text`, `read_arm_hidden`), which reach nested
    /// modules, read `tc_file`, the file-wide union. With no facts for the file (`tc` is `None`) it is `tc_file` throughout, as
    /// R1899 left it.
    tc_shadow: std::cell::RefCell<std::collections::HashSet<String>>,
    /// R-BARE-TC — the file-wide reading: R1899's, but for a glob (see `tc_names_for`).
    tc_file: std::collections::HashSet<String>,
    /// R-BARE-TC — this file's facts (`baretc`): `None` = R1899's reading.
    tc: Option<crate::baretc::TcFile>,
    /// R-BARE-TC — the file holds a non-toolchain `#[macro_use] extern crate` (every toolchain name may be rebound)
    tc_mu: bool,
    /// R-BARE-TC — the toolchain names this crate or file defines itself (`macro_rules!`, emitted or not)
    tc_locals: std::collections::HashSet<String>,
    /// SOUNDNESS R1862 / R1824 (cfg-twin lane A) — the cross-file claims this file's splice consumes (`ArmClaims`).
    claims: DecodedClaims,
    /// the inline module path of the list being spliced, within this file (`a::b`), for `claims` lookups.
    modpath: Vec<String>,
    /// SOUNDNESS R1910 / R1853 (R-ROOT) — this file's crate root and module path (`rroot::Scope`), so a macro PATH whose module
    /// part is the root binds the crate's one root-namespace definition (`rroot::F1_PREFIX` entries of `xdefs`).
    f1: Option<crate::rroot::Scope>,
    /// R-ROOT — how many `#[cfg]`/`#[cfg_attr]`-gated containers (inline module, impl, trait) enclose the list being spliced,
    /// plus one while a gated invocation itself is expanded. A root path is not bound inside one (`root_path_name`).
    f1_gated: usize,
}

/// SOUNDNESS R1899 (cfg-twin lane A) — every name a `use` of this file binds through a path rooted at neither the crate
/// (`crate`/`self`/`super`) nor the toolchain (`std`/`core`/`alloc`, and a leading `::` before one): `use std::thread_local;`
/// (proc-macro2's fallback.rs) binds the toolchain's macro, `use dep::thread_local;` does not.
///
/// `deep` false reads this item list alone: an import does not reach an inline module's list, and an inline module's does not
/// reach this one (R-BARE-TC, arm 2 per MODULE).
fn non_toolchain_imports_in(items: &[syn::Item], deep: bool) -> std::collections::HashSet<String> {
    fn walk(t: &syn::UseTree, root: Option<&str>, out: &mut std::collections::HashSet<String>) {
        let skip = |r: Option<&str>| matches!(r, Some("crate" | "self" | "super" | "std" | "core" | "alloc"));
        match t {
            syn::UseTree::Path(p) => {
                let first = p.ident.to_string();
                walk(&p.tree, Some(root.unwrap_or(first.as_str())), out);
            }
            syn::UseTree::Name(n) if root.is_some() && !skip(root) => {
                out.insert(n.ident.to_string());
            }
            syn::UseTree::Rename(r) if root.is_some() && !skip(root) => {
                out.insert(r.rename.to_string());
            }
            syn::UseTree::Group(g) => g.items.iter().for_each(|x| walk(x, root, out)),
            _ => {}
        }
    }
    fn items_walk(items: &[syn::Item], deep: bool, out: &mut std::collections::HashSet<String>) {
        for it in items {
            match it {
                syn::Item::Use(u) => walk(&u.tree, None, out),
                syn::Item::Mod(m) if deep => {
                    if let Some((_, inner)) = &m.content {
                        items_walk(inner, deep, out);
                    }
                }
                _ => {}
            }
        }
    }
    let mut out = std::collections::HashSet::new();
    items_walk(items, deep, &mut out);
    out
}

/// R-BARE-TC — does this file hold a `#[macro_use] extern crate` of something other than the toolchain? (`imported_names` records
/// it as the wildcard `*`, together with every non-toolchain glob; this is that half alone.)
fn has_macro_use_extern(items: &[syn::Item]) -> bool {
    items.iter().any(|it| match it {
        syn::Item::ExternCrate(e) => {
            e.attrs.iter().any(|a| a.path().is_ident("macro_use")) && !matches!(e.ident.to_string().as_str(), "std" | "core" | "alloc")
        }
        syn::Item::Mod(m) => m.content.as_ref().is_some_and(|(_, inner)| has_macro_use_extern(inner)),
        _ => false,
    })
}

/// I1 step 1 — every name a `use` item of this file binds through a path that is NOT rooted at `crate`/`self`/`super`
/// (a dependency, or a 2018 uniform path that may be one). Renames bind their new name. Globs bind nothing nameable.
fn imported_names(items: &[syn::Item], out: &mut std::collections::HashSet<String>) {
    fn walk(t: &syn::UseTree, root: Option<&str>, out: &mut std::collections::HashSet<String>) {
        let local = |r: Option<&str>| matches!(r, Some("crate") | Some("self") | Some("super"));
        match t {
            syn::UseTree::Path(p) => {
                let first = p.ident.to_string();
                walk(&p.tree, Some(root.unwrap_or(first.as_str())), out);
            }
            syn::UseTree::Name(n) if !local(root) && root.is_some() => {
                out.insert(n.ident.to_string());
            }
            syn::UseTree::Rename(r) if !local(root) && root.is_some() => {
                out.insert(r.rename.to_string());
            }
            syn::UseTree::Group(g) => g.items.iter().for_each(|x| walk(x, root, out)),
            _ => {}
        }
    }
    // A non-crate-rooted GLOB `use` or a `#[macro_use] extern crate` can bring ANY macro name into scope; it is recorded as
    // the wildcard `*`, which only `collect_defs`' cfg-gated rule reads (Fable merge review §6k: a `#[cfg(any())]` stub
    // beside `use dep::*;` or `#[macro_use] extern crate dep;` was still bound, pre-existing). An UNGATED local definition
    // still wins over a glob or `macro_use` name, as rustc's textual scope does.
    fn glob_root(t: &syn::UseTree, root: Option<&str>) -> bool {
        match t {
            syn::UseTree::Path(p) => {
                let first = p.ident.to_string();
                glob_root(&p.tree, Some(root.unwrap_or(first.as_str())))
            }
            // A glob of the toolchain's own crates (`use core::simd::*`) is not a dependency's macro namespace: counting it
            // refused rand's `#[cfg(feature = "simd_support")] macro_rules! simd_impl` and lost two of I1's `Rand` charges.
            syn::UseTree::Glob(_) => {
                !matches!(root, Some("crate") | Some("self") | Some("super") | Some("std") | Some("core") | Some("alloc"))
                    && root.is_some()
            }
            syn::UseTree::Group(g) => g.items.iter().any(|x| glob_root(x, root)),
            _ => false,
        }
    }
    for it in items {
        match it {
            syn::Item::Use(u) if glob_root(&u.tree, if u.leading_colon.is_some() { Some("::") } else { None }) => {
                out.insert("*".to_string());
                walk(&u.tree, if u.leading_colon.is_some() { Some("::") } else { None }, out)
            }
            syn::Item::ExternCrate(e)
                if e.attrs.iter().any(|a| a.path().is_ident("macro_use"))
                    && !matches!(e.ident.to_string().as_str(), "std" | "core" | "alloc") =>
            {
                out.insert("*".to_string());
            }
            syn::Item::Use(u) => walk(&u.tree, if u.leading_colon.is_some() { Some("::") } else { None }, out),
            syn::Item::Mod(m) => {
                if let Some((_, inner)) = &m.content {
                    imported_names(inner, out);
                }
            }
            _ => {}
        }
    }
}

impl Ctx<'_> {
    /// I1 stage 2 — ONE READER for hidden text: an unexpanded invocation is EXPANDED with every body its macro has (cfg twins
    /// included, crate-wide through `ALL_BODIES_KEY`) and the expansion is parsed by `syn` and read with `item_keys`, the same
    /// function that reads the visible arm. Nested invocations are read the same way. `member` is the impl's self type for an
    /// impl-item-position invocation (its expansion is parsed as impl items). Where expansion cannot finish — the depth or step
    /// limit, no arm matching, another crate's macro — the text falls to a STATED over-approximation, `loose`: every identifier
    /// of the invocation and of its bodies. Where it is unreadable (a local macro with no readable body, `concat_idents!`), opaque.
    fn read_hidden(&self, mac: &syn::Macro, member: Option<&str>, out: &[syn::Item], h: &mut Hidden, depth: usize) {
        let name = mac.path.segments.last().map(|s| s.ident.to_string()).unwrap_or_default();
        if name == "concat_idents" || name == "include" {
            h.opaque = true;
            h.note("opaque", &name);
            return;
        }
        // Textual scope binds an UNQUALIFIED invocation only. A path invocation (`crate::m!`, `a::m!` through `pub(crate) use m;`,
        // `$crate::m!` arriving as `crate::m`) is resolved by path in rustc, so it falls through to the file-wide and crate-wide
        // bodies as before (Fable merge review 2d D-1: `qb_e`/`qc_e`/`qd_e` read the textual same-name body and dropped the one
        // the path named).
        let scoped = (mac.path.segments.len() == 1 && mac.path.leading_colon.is_none()
            && !candor_report::env::switch("CANDOR_I1S2D_SCOPEBODY_OFF"))
        .then_some(&self.scope_bodies);
        let table = BodyTable { scoped, bodies: Some(&self.bodies), xdefs: Some(self.xdefs) };
        // FALLBACK ARM "toolchain". SUPERSET ARGUMENT: of `TOOLCHAIN_BANG_MACROS`, the only ones that emit ITEMS are
        // `thread_local!` (a `static` per declaration it is passed) and `include!` (opaque above); `global_asm!`/`asm!` emit no
        // named item, and the rest are expression or diagnostic macros. So the names an item-position invocation of one can
        // bind are the names its input declares after an item keyword, which is what `declared_names` reads. A local definition
        // shadowing the name is read below.
        let root_tc = mac.path.segments.len() == 1
            || matches!(mac.path.segments.first().map(|s| s.ident.to_string()).as_deref(), Some("std" | "core" | "alloc"));
        let bare_shadowed = mac.path.segments.len() == 1 && mac.path.leading_colon.is_none() && self.tc_shadow.borrow().contains(&name);
        if root_tc && !bare_shadowed && TOOLCHAIN_BANG_MACROS.contains(&name.as_str()) && table.get(&name).is_none() {
            declared_names(&mac.tokens, &mut h.loose);
            return;
        }
        let bodies = match table.get(&name) {
            None => {
                // Another crate's macro: its expansion is not here. Where what it is passed parses as items (pin_project's
                // struct, `loom::thread_local! { static X .. }`), the names those items declare, plus every identifier of
                // their attributes (`#[project = ProjName]` names a type the macro defines); otherwise every identifier passed.
                //
                // The items passed are ALSO read exactly as a local expansion's items are (`read_hidden_item`), so their
                // QUALIFIED keys land where the cover looks them up: `Tr` and `Tr::m` for a trait, `S as Tr` (+ `::m`, with
                // the signature) for a trait impl, `S::m` for a member, and a nested invocation read in turn. Filed under
                // `loose` alone they were invisible to every qualified lookup (`hidden_trait`, the `S as Tr` key,
                // `covers_method`'s `S::m`): a same-module `fxpdep::passthru! { #[cfg(unix)] trait Tr { fn go { write } } }`
                // beside a produced gated `impl Tr for S { fn go {} }` read silent (Fable merge review 2a, `rp_e`/`rp2_e`).
                // This only ADDS keys: the `loose` reading below stays, so the arm reads a superset of what it read before.
                match syn::parse2::<syn::File>(mac.tokens.clone()) {
                    Ok(f) if !f.items.is_empty() => {
                        for it in &f.items {
                            self.read_hidden_item(it, out, h, depth + 1);
                            let k = item_keys(it, out);
                            h.loose.extend(k.names);
                            h.opaque |= k.opaque;
                            for a in crate::lang::item_attrs(it) {
                                all_idents(&quote::ToTokens::to_token_stream(a), &mut h.loose);
                            }
                        }
                        h.note("foreign-items", &name);
                    }
                    _ => {
                        all_idents(&mac.tokens, &mut h.loose);
                        h.note("foreign", &name);
                    }
                }
                // FALLBACK ARMS "foreign-items" / "foreign". SUPERSET ARGUMENT: none of what is read above is one. Another
                // crate's macro (a `macro_rules!` this scan has no body for, or a proc macro) can emit any item under any name,
                // including a name written nowhere in what it is passed (Fable RC3-1 was a nested invocation in its input; a
                // dependency's `mk!(S)` minting `impl S { fn go }` names `go` only in ITS body). The only superset is "anything":
                // the reading stays (it feeds the qualified keys) and the text is OPAQUE. `CANDOR_I1S2B_FOREIGN_LOOSE=1` restores
                // the narrower reading, for measurement.
                //
                // No exception for `cfg_if!`. The cfg-if crate's macro only re-arranges its input, but which macro the name
                // binds is not something this pass can prove from the source: three rounds of an identity check each closed the
                // probe in hand and stayed open one spelling over (a Cargo `package =` rename, `extern crate x as cfg_if` with the
                // path form, a `cfg_attr`-gated `#[macro_use]`, edition-2015 root-relative paths reaching a private `mod
                // cfg_if`, renames re-exported or globbed from another file, a dependency's own `macro_rules! cfg_if`). Its
                // whole corpus reach was 12 sites in 4 crates and opacity is row-identical there (STAGE2B-REPORT UPDATE A3).
                if !candor_report::env::switch("CANDOR_I1S2B_FOREIGN_LOOSE") {
                    h.opaque = true;
                }
                return;
            }
            Some(None) => {
                h.opaque = true;
                h.note("opaque", &name);
                return;
            }
            Some(Some(bs)) => bs,
        };
        if depth >= HIDDEN_DEPTH || h.steps >= HIDDEN_STEP_BUDGET {
            // FALLBACK ARM "limit". SUPERSET ARGUMENT: see `loose_closure` (every identifier rustc's expansion could emit).
            self.loose_closure(&name, &mac.tokens, h);
            h.note("limit", &name);
            return;
        }
        let mut any = false;
        let mut unparsed = false;
        for b in &bodies {
            h.steps += 1;
            let Some(ts) = parse_arms(b).and_then(|arms| expand(&arms, &mac.tokens)) else { continue };
            match member {
                None => {
                    if let Ok(f) = syn::parse2::<syn::File>(ts) {
                        any = true;
                        for it in &f.items {
                            self.read_hidden_item(it, out, h, depth + 1);
                        }
                    } else {
                        unparsed = true; // an arm expanded to text `syn` cannot read as items (Fable RC3.1 nit)
                    }
                }
                Some(owner) => {
                    use syn::parse::Parser;
                    let parsed = (|input: syn::parse::ParseStream| {
                        let mut v = Vec::new();
                        while !input.is_empty() {
                            v.push(input.parse::<syn::ImplItem>()?);
                        }
                        Ok(v)
                    })
                    .parse2(ts);
                    if let Ok(iis) = parsed {
                        any = true;
                        for ii in &iis {
                            self.read_hidden_member(owner, ii, out, h, depth + 1);
                        }
                    } else {
                        unparsed = true;
                    }
                }
            }
        }
        if !any || unparsed {
            // FALLBACK ARM "noexpand" (no arm matched, or an arm's expansion did not parse). SUPERSET ARGUMENT: see
            // `loose_closure`. Before, it read the invocation's identifiers and the names its bodies DECLARE, which missed a
            // name written as a nested invocation's argument or a `use` leaf (Fable RC3-3 `noexpnest2_e`, RC3-4 `noexpuse_e`).
            self.loose_closure(&name, &mac.tokens, h);
            h.note("noexpand", &name);
        }
    }

    /// I1 stage 2b — the `loose` reading of text whose expansion this pass could not finish (the "noexpand" and "limit" arms).
    /// SUPERSET ARGUMENT: a `macro_rules!` expansion emits only tokens of its invocation and of the transcriber that matched,
    /// and a nested invocation's tokens come from ITS invocation and transcriber in turn. So every identifier rustc's expansion
    /// can contain is an identifier of this invocation, of any body of this macro, or of any body of a local macro named
    /// (`name!`) anywhere in that set, transitively; that union is read here. Where the set names a macro with no body this
    /// scan holds (another crate's, or an unreadable local one) or a token-pasting macro, the expansion can contain any name,
    /// so the text is opaque. Toolchain macros other than `include!`/`concat_idents!` emit no new identifiers.
    ///
    /// Two lookups make this a superset (Fable 2b review): a name reached here reads the UNION of every same-named body the
    /// crate holds, not the textual-scope one, since a path invocation (`crate::m::inner!{}`) can name another module's
    /// definition (`ps_e`); and a `$m!` whose name is a metavariable is opaque, since the macro is not knowable from the body
    /// (`mv_e`). `a != b` is not an invocation (`!` joint with `=`). Past the step bound the reading is opaque, never short.
    fn loose_closure(&self, name: &str, tokens: &TokenStream, h: &mut Hidden) {
        /// Pushes every `name!` the tokens invoke; `$m!` pushes `$` (a metavariable-named macro).
        fn invoked(ts: &TokenStream, out: &mut Vec<String>) {
            let toks: Vec<TokenTree> = ts.clone().into_iter().collect();
            for (i, tt) in toks.iter().enumerate() {
                match tt {
                    TokenTree::Group(g) => invoked(&g.stream(), out),
                    TokenTree::Ident(id) => {
                        let bang = matches!(toks.get(i + 1),
                            Some(TokenTree::Punct(p)) if p.as_char() == '!' && p.spacing() == proc_macro2::Spacing::Alone);
                        if bang {
                            let n = id.to_string();
                            let meta = i > 0 && matches!(toks.get(i - 1), Some(TokenTree::Punct(p)) if p.as_char() == '$');
                            if meta {
                                out.push("$".to_string());
                            } else if n != "macro_rules" {
                                out.push(n);
                            }
                        }
                    }
                    _ => {}
                }
            }
        }
        all_idents(tokens, &mut h.loose);
        let mut todo: Vec<String> = vec![name.to_string()];
        invoked(tokens, &mut todo);
        let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
        while let Some(n) = todo.pop() {
            if !seen.insert(n.clone()) {
                continue;
            }
            if seen.len() > 4096 {
                h.opaque = true; // the bound fails CLOSED: past it the reading is "anything", never a short list
                break;
            }
            if n == "$" || n == "concat_idents" || n == "paste" || n == "include" {
                h.opaque = true;
                continue;
            }
            match self.closure_bodies(&n) {
                Some(Some(bs)) => {
                    for b in &bs {
                        all_idents(b, &mut h.loose);
                        invoked(b, &mut todo);
                    }
                }
                Some(None) => h.opaque = true,
                // a toolchain macro emits no new identifier
                None if TOOLCHAIN_BANG_MACROS.contains(&n.as_str()) => {}
                None => {
                    if !candor_report::env::switch("CANDOR_I1S2B_FOREIGN_LOOSE") {
                        h.opaque = true; // another crate's macro inside the closure: it can emit any name
                    }
                }
            }
        }
    }

    /// I1 stage 2b — every body of `n` the crate holds: this file's (all modules, cfg twins included) and the crate-wide set,
    /// unioned. `None`: no local macro of the name; `Some(None)`: local, but no body readable.
    fn closure_bodies(&self, n: &str) -> Option<Option<Vec<TokenStream>>> {
        let mut v: Vec<TokenStream> = self.bodies.get(n).cloned().unwrap_or_default();
        let mut known = !v.is_empty();
        let mut unreadable = false;
        match self.xdefs.get(&format!("{ALL_BODIES_KEY}{n}")) {
            Some(Some(all)) => {
                known = true;
                for b in all.split('\u{0}') {
                    match b.parse::<TokenStream>() {
                        Ok(ts) => v.push(ts),
                        Err(_) => unreadable = true,
                    }
                }
            }
            _ => match self.xdefs.get(n) {
                Some(Some(t)) => {
                    known = true;
                    match t.parse::<TokenStream>() {
                        Ok(ts) => v.push(ts),
                        Err(_) => unreadable = true,
                    }
                }
                Some(None) => {
                    known = true;
                    unreadable |= v.is_empty();
                }
                None => {}
            },
        }
        if !known {
            return None;
        }
        if unreadable {
            return Some(None);
        }
        Some(Some(v))
    }

    fn read_hidden_item(&self, it: &syn::Item, out: &[syn::Item], h: &mut Hidden, depth: usize) {
        match it {
            syn::Item::Macro(m) if m.ident.is_none() => self.read_hidden(&m.mac, None, out, h, depth),
            syn::Item::Impl(im) => {
                let owner = ty_key(&im.self_ty);
                // A hidden TRAIT impl is keyed `Owner as Trait`, and each member it writes `Owner as Trait::m` with its
                // signature kept: the visible arm of that impl may lack the member (it inherits the trait's default), and the
                // cover then has to stand a marked member where the hidden arm's override would be (`mark_hidden_twins`).
                if let Some(t) = trait_key(im) {
                    let ik = format!("{owner} as {t}");
                    // I1 stage 2c — a BLANKET impl (`impl<T: Mk> Tr for T`) is an arm for EVERY type it may cover, so it is
                    // also keyed under the wildcard owner `*` (`* as Tr`, `* as Tr::m`), which every candidate of `Tr` asks
                    // for (`twin_cand_keys`). Keyed under its parameter's name alone it matched nothing (Fable merge
                    // review 2c `tg_e`/`tg2_e`, introduced on main by stage 1).
                    let xmod = !candor_report::env::switch("CANDOR_I1S2C_XMOD_OFF");
                    let blanket = xmod && im.generics.type_params().any(|p| p.ident == owner);
                    // I1 stage 2d — a blanket over a non-path type (`&T`, a tuple, a slice…) twins only impls of that SHAPE.
                    let shaped = (xmod && !blanket && structural_blanket(im)).then(|| self_shape(&owner)).flatten();
                    if let Some(sh) = &shaped {
                        h.xkeys.insert(format!("{sh} as {t}"));
                        for ii in &im.items {
                            if let syn::ImplItem::Fn(f) = ii {
                                h.xkeys.insert(format!("{sh} as {t}::{}", f.sig.ident));
                            }
                        }
                    }
                    for ii in &im.items {
                        if let syn::ImplItem::Fn(f) = ii {
                            let mk = format!("{ik}::{}", f.sig.ident);
                            h.sigs.entry(mk.clone()).or_insert_with(|| f.sig.clone());
                            if blanket {
                                h.xkeys.insert(format!("* as {t}::{}", f.sig.ident));
                            }
                            h.xkeys.insert(mk.clone());
                            h.keys.insert(mk);
                        }
                    }
                    if blanket {
                        h.xkeys.insert(format!("* as {t}"));
                    }
                    h.xkeys.insert(ik.clone());
                    h.keys.insert(ik);
                }
                for ii in &im.items {
                    self.read_hidden_member(&owner, ii, out, h, depth);
                }
            }
            // I1 stage 2c — a module INSIDE hidden text (`hid! { pub mod inner { pub trait Tr { .. } } }`): its path items are
            // keyed under its name (`inner::x`), its impl-scoped keys unprefixed (they are not path-scoped). Before, the
            // module's NAME was all that was read, so a trait or impl inside it was invisible to every lookup (Fable merge
            // review 2a `rc_e`).
            syn::Item::Mod(md) if md.content.is_some() && !candor_report::env::switch("CANDOR_I1S2C_XMOD_OFF") => {
                h.keys.insert(md.ident.to_string());
                let inner = &md.content.as_ref().map(|c| &c.1).into_iter().flatten().cloned().collect::<Vec<_>>();
                let mut sub = Hidden { steps: h.steps, ..Hidden::default() };
                for x in inner {
                    self.read_hidden_item(x, inner, &mut sub, depth + 1);
                }
                h.steps = sub.steps;
                let pre = md.ident.to_string();
                let Hidden { opaque, keys, loose, xkeys, sigs, .. } = sub;
                h.opaque |= opaque;
                h.keys.extend(keys.into_iter().map(|k| format!("{pre}::{k}")));
                h.loose.extend(loose);
                h.xkeys.extend(xkeys);
                for (k, v) in sigs {
                    h.sigs.entry(k).or_insert(v);
                }
            }
            _ => {
                let k = item_keys(it, out);
                if matches!(it, syn::Item::Trait(_)) || mbe_trait_body_of(it).is_some() {
                    h.xkeys.extend(k.names.iter().cloned()); // `Tr`, `Tr::m`: a trait's items reach its impls crate-wide
                }
                // I1 stage 2d — a `type` alias the hidden text writes names its type another way (`=Alias=Orig`, as a rename).
                if let syn::Item::Type(t) = it {
                    if !candor_report::env::switch("CANDOR_I1S2D_ALIAS_OFF") {
                        h.xkeys.insert(format!("={}={}", t.ident, ty_key(&t.ty)));
                    }
                }
                if let syn::Item::Use(u) = it {
                    // I1 stage 2c — a `use … as` the hidden text writes is a rename the crate-wide check must follow too
                    // (Fable merge review 2c `tr2_e`: the rename was hidden on both arms, so the visible-only reading
                    // never saw it). Exported as `=Alias=Orig`; `twin_facts` files it with the visible renames.
                    if !candor_report::env::switch("CANDOR_I1S2C_XMOD_OFF") {
                        let mut rn = Vec::new();
                        use_renames(&u.tree, &mut rn);
                        h.xkeys.extend(rn.into_iter().map(|r| format!("={r}")));
                    }
                    if candor_report::env::trace("CANDOR_I1_INSTR") {
                        let t = quote::ToTokens::to_token_stream(&u.tree).to_string();
                        if t.contains('{') {
                            eprintln!("I1S2HIDUSEGROUP\t{t}"); // §E1 REACH COUNTER: a hidden grouped `use`
                        }
                        if t.contains('*') && (t.starts_with("std") || t.starts_with("core") || t.starts_with("alloc")) {
                            eprintln!("I1S2HIDSTDGLOB\t{t}"); // §E1 REACH COUNTER: a hidden toolchain glob
                        }
                    }
                }
                h.opaque |= k.opaque;
                h.keys.extend(k.names);
            }
        }
    }

    fn read_hidden_member(&self, owner: &str, ii: &syn::ImplItem, out: &[syn::Item], h: &mut Hidden, depth: usize) {
        match ii {
            syn::ImplItem::Fn(f) => {
                h.xkeys.insert(format!("{owner}::{}", f.sig.ident));
                h.keys.insert(format!("{owner}::{}", f.sig.ident));
            }
            syn::ImplItem::Macro(m) => self.read_hidden(&m.mac, Some(owner), out, h, depth),
            _ => {}
        }
    }

    /// SOUNDNESS R1857 — what one module arm's hidden text may define, at every depth of it: each item-position invocation
    /// left unexpanded, and each impl-item and trait-item invocation (keyed `Owner::m`), read by `read_hidden` as the
    /// per-module cover reads them. Keys are unprefixed (see `cover_arm_keyed`).
    /// Does this arm hold text `read_arm_hidden` reads: `holds_hidden`'s, or a body-position path invocation of a local macro
    /// that writes an `impl` (see `read_arm_hidden`).
    fn holds_arm_text(&self, items: &[syn::Item]) -> bool {
        self.with_file_tc(|| self.holds_arm_text_inner(items))
    }

    /// R-BARE-TC — run `f` reading the FILE-WIDE toolchain-name set: the module-arm readers reach nested modules, each of which
    /// would need its own set, and the union is the sound one. A no-op without facts (the set is the file's throughout).
    fn with_file_tc<R>(&self, f: impl FnOnce() -> R) -> R {
        if self.tc.is_none() {
            return f();
        }
        let saved = self.tc_shadow.replace(self.tc_file.clone());
        let r = f();
        self.tc_shadow.replace(saved);
        r
    }

    fn holds_arm_text_inner(&self, items: &[syn::Item]) -> bool {
        if holds_hidden(items, &self.partial, &self.tc_shadow.borrow()) {
            return true;
        }
        if !self.bodies.values().any(|bs| bs.iter().any(writes_impl)) && !self.xdefs.values().flatten().any(|t| t.contains("impl")) {
            return false;
        }
        let mut h = Hidden::default();
        self.read_arm_hidden(items, &mut h);
        !h.is_empty()
    }

    fn read_arm_hidden(&self, items: &[syn::Item], h: &mut Hidden) {
        self.with_file_tc(|| self.read_arm_hidden_inner(items, h));
    }

    fn read_arm_hidden_inner(&self, items: &[syn::Item], h: &mut Hidden) {
        // cfg-twin lane A — a BODY-position invocation by PATH (`crate::m! {}` inside `fn host`) of a local macro whose body
        // writes an `impl`: the body hoist (R1813) binds bare names only, so the impl it emits stays unexpanded text of this arm
        // (`p6`), and the twin impl in a sibling arm read alone. Read with the one reader, as item text.
        struct BodyPaths<'a>(Vec<&'a syn::Macro>);
        impl<'ast> syn::visit::Visit<'ast> for BodyPaths<'ast> {
            fn visit_macro(&mut self, m: &'ast syn::Macro) {
                if m.path.segments.len() > 1 {
                    self.0.push(m);
                }
            }
            fn visit_item(&mut self, _: &'ast syn::Item) {}
        }
        let body_paths = |b: &syn::Block, h: &mut Hidden| {
            let mut v = BodyPaths(Vec::new());
            syn::visit::Visit::visit_block(&mut v, b);
            let table = BodyTable { scoped: None, bodies: Some(&self.bodies), xdefs: Some(self.xdefs) };
            for mac in v.0 {
                let name = mac.path.segments.last().map(|s| s.ident.to_string()).unwrap_or_default();
                if table.get(&name).is_some_and(|bs| bs.is_some_and(|bs| bs.iter().any(writes_impl))) {
                    self.read_hidden(mac, None, items, h, 0);
                }
            }
        };
        for it in items {
            match it {
                syn::Item::Fn(f) => body_paths(&f.block, h),
                syn::Item::Macro(m) if is_hidden_text(m, &self.partial, &self.tc_shadow.borrow()) => self.read_hidden(&m.mac, None, items, h, 0),
                syn::Item::Impl(im) => {
                    let owner = ty_key(&im.self_ty);
                    for ii in &im.items {
                        match ii {
                            syn::ImplItem::Macro(m) => self.read_hidden(&m.mac, Some(&owner), items, h, 0),
                            syn::ImplItem::Fn(f) => body_paths(&f.block, h),
                            _ => {}
                        }
                    }
                }
                syn::Item::Trait(t) => {
                    let owner = t.ident.to_string();
                    for ti in &t.items {
                        if let syn::TraitItem::Macro(m) = ti {
                            self.read_hidden(&m.mac, Some(&owner), items, h, 0);
                        }
                    }
                }
                syn::Item::Mod(m) => {
                    if let Some((_, inner)) = &m.content {
                        self.read_arm_hidden_inner(inner, h);
                    }
                }
                _ => {}
            }
        }
    }

    /// SOUNDNESS R1857 — A CFG-TWIN MODULE WHOSE OTHER ARM HOLDS HIDDEN TEXT.
    ///
    /// THE EXPOSURE. Stage 2 splices the modules an expansion emits (`i1_refused`). Two `#[cfg]` arms of one module (`#[cfg(unix)]
    /// mod sub { .. }` / `#[cfg(not(unix))] mod sub { .. }`) both become visible, and a path into either resolves to the union of
    /// the units the two arms write. Where the live arm's item is still unexpanded text — at ANY depth of that arm: an item
    /// invocation, a trait-item invocation in a trait (`ftrcfgmod`), an impl-item invocation, inside a nested module — no unit
    /// stands for it, and the call resolves to the OTHER arm's visible item alone (a pure trait default, a pure fn). On 0.42.0 the
    /// whole expansion stayed opaque and R128 disclosed `macro:`. The per-module cover (`mark_hidden_twins`) cannot see this: the
    /// hidden text sits in a SIBLING module, not in the arm's own list, and pass B's crate-wide check reads impl members only.
    ///
    /// THE UNIT. A module arm, read whole: the other arms' hidden text at every depth, by the ONE reader (`read_arm_hidden` →
    /// `read_hidden`), and every position of this arm checked against it (`cover_arm_keyed`): fns, `use`s, aliases, named
    /// macro definitions, nested modules, impl members (by `Owner::m`, owner type or trait) and trait defaults (written or as a
    /// generated-trait marker). An opaque reading covers the arm whole. The marked arm must be PRODUCED (inside an expansion, or
    /// spliced into this list): a pair of hand-written arms is not this pass's change. The arm holding the hidden text may be
    /// either (`p9hand`: a hand-written live arm beside a produced one; 0.42.0 refused the produced arm). Arms are grouped by
    /// module name within one item list, which is where rustc requires a module's cfg twins to be. An OUT-OF-LINE arm
    /// (`#[cfg(unix)] mod sub;`, Fable merge review C1 `k_outline`) takes part with an opaque reading: its body is another
    /// file, so every produced inline twin is covered whole. An out-of-line arm is never itself marked here (its units are
    /// that file's; `k_path`, `k_outline2` and the hand-written pair `k_hand` read silent on 0.42.0 as well). `CANDOR_R1857_WHOLE=1`
    /// covers every produced arm whole wherever another arm holds hidden text (measurement only). `CANDOR_R1857_OFF=1` drops it.
    fn mark_module_twins(&self, out: &mut [syn::Item], spliced: &[bool], depth: usize) {
        let mut groups: std::collections::BTreeMap<String, Vec<usize>> = std::collections::BTreeMap::new();
        for (i, it) in out.iter().enumerate() {
            // An out-of-line arm (`#[cfg(unix)] mod sub;`) is in the group too (Fable merge review C1, `k_outline`): its body is
            // another file, which this per-file pass cannot read, so its reading is OPAQUE below.
            if let syn::Item::Mod(m) = it {
                if is_gated(&m.attrs) {
                    groups.entry(m.ident.to_string()).or_default().push(i);
                }
            }
        }
        let whole = candor_report::env::switch("CANDOR_R1857_WHOLE");
        let here = self.modpath.join("::");
        for (name, idxs) in groups.iter() {
            // cfg-twin lane A — what the arms of this group in OTHER files claim (`ArmClaims::inline`): the real reading of
            // every out-of-line target's hidden text. A group of one `mod` item can still be a group (`cfg_attr(path)`: one
            // declaration, two files), which only that claim can show.
            let xclaim = self.claims.inline.get(&claim_key(&here, name)).filter(|h| !h.is_empty());
            if idxs.len() < 2 && xclaim.is_none() {
                continue;
            }
            let readings: Vec<Option<Hidden>> = idxs
                .iter()
                .map(|&i| {
                    let syn::Item::Mod(m) = &out[i] else { return None };
                    let Some((_, inner)) = m.content.as_ref() else {
                        // SUPERSET ARGUMENT: the arm's items are a file's, read nowhere in this pass, so they may define any
                        // name the produced twin shows; the twin is covered whole (`cover_arm`).
                        if candor_report::env::trace("CANDOR_I1_INSTR") {
                            eprintln!("R1857OUTLINE\t{}", m.ident); // §E1 REACH COUNTER
                        }
                        return Some(Hidden { opaque: true, ..Hidden::default() });
                    };
                    if !self.holds_arm_text(inner) {
                        return None;
                    }
                    let mut h = Hidden::default();
                    self.read_arm_hidden(inner, &mut h);
                    h.opaque |= whole;
                    Some(h)
                })
                .collect();
            // the stand-ins: an out-of-line arm's opaque reading above (R1857's floor for a PRODUCED inline twin)
            let standin: Vec<bool> =
                idxs.iter().map(|&i| matches!(&out[i], syn::Item::Mod(m) if m.content.is_none())).collect();
            // cfg-twin lane A — export each INLINE arm's reading for the out-of-line arms in other files (`ARM_READ_MARK`,
            // read by `arm_records`). Read here, before any arm of the group is marked.
            if !r1857_off() {
                for (ai, &a) in idxs.iter().enumerate() {
                    if let (false, Some(r), syn::Item::Mod(m)) = (standin[ai], readings[ai].as_ref(), &mut out[a]) {
                        if let Some((_, inner)) = &mut m.content {
                            if !inner.iter().any(|it| is_read_marker(it, ARM_READ_MARK)) {
                                inner.push(read_marker(ARM_READ_MARK, r));
                            }
                        }
                    }
                }
            }
            for (bi, &b) in idxs.iter().enumerate() {
                // SOUNDNESS R1828 / R1862 `k_hand` (cfg-twin lane A) — a HAND-WRITTEN arm is an arm: rustc chooses between
                // two written `#[cfg]` modules exactly as between two produced ones. It is covered from what its siblings'
                // text REALLY claims: an inline sibling's reading, and an out-of-line sibling's FILE reading (`xclaim`),
                // never the opaque stand-in, which stays where v0.43.0 applies it, on a produced arm (Fable S5′). It is
                // marked without removing a binding (`cover_any(.., hand)`), so it only adds `Unknown`.
                let hand = depth == 0 && !spliced.get(b).copied().unwrap_or(false);
                let mut h = Hidden::default();
                for (ai, r) in readings.iter().enumerate() {
                    if ai == bi || (hand && standin[ai]) {
                        continue;
                    }
                    if let Some(r) = r {
                        h.absorb(r.clone());
                    }
                }
                let own_target = matches!(&out[b], syn::Item::Mod(m) if m.content.is_none());
                if !own_target {
                    if let Some(x) = xclaim {
                        h.absorb(x.clone());
                    }
                }
                if h.is_empty() {
                    continue;
                }
                if let syn::Item::Mod(m) = &mut out[b] {
                    if candor_report::env::trace("CANDOR_I1_INSTR") {
                        let tag = if hand { "R1862HAND" } else { "R1857MODTWIN" };
                        eprintln!("{tag}\t{}\t{}", m.ident, if h.opaque { "opaque" } else { "keyed" }); // §E1 REACH COUNTER
                    }
                    if let Some((_, inner)) = &mut m.content {
                        cover_arm_keyed(inner, &h, hand);
                    }
                }
            }
        }
    }

    /// R1156 — the definition in textual scope for a name this FILE defines more than once.
    fn scoped_def(&self, name: &str) -> Option<String> {
        // I1 stage 3 (R1784) — an emitted definition is read from textual scope whatever the file-wide table holds: it shadows a
        // source definition of the name from its invocation on (the table was set to `None` there), and it is the only
        // definition of a name the table never held.
        let emitted = self.emitted.contains(name) && !matches!(self.defs.get(name), Some(Some(_)));
        if !emitted && !matches!(self.defs.get(name), Some(None)) {
            return None;
        }
        self.scope.get(name).cloned().flatten()
    }

    fn arms_of(&mut self, name: &str) -> Option<std::rc::Rc<Vec<Arm>>> {
        if let Some(t) = self.scoped_def(name) {
            let key = format!("{name}\u{1}{t}");
            if let Some(a) = self.arms.get(&key) {
                return a.clone();
            }
            if candor_report::env::trace("CANDOR_V053_INSTR") {
                eprintln!("R1156ITEM\t{name}"); // §E1 REACH COUNTER
            }
            let a = t.parse::<TokenStream>().ok().and_then(|t| parse_arms(&t)).map(std::rc::Rc::new);
            self.arms.insert(key, a.clone());
            return a;
        }
        if let Some(a) = self.arms.get(name) {
            return a.clone();
        }
        let local = self.defs.get(name).cloned().flatten();
        let def = match local {
            Some(t) => Some(t),
            // R1004 — a definition from ANOTHER file, re-read from its text on THIS thread.
            None if !self.defs.contains_key(name) => match self.xdefs.get(name) {
                Some(d) => d.clone().and_then(|s| s.parse::<TokenStream>().ok()),
                // I1 stage 3 — a name only an expansion (in another file) defines
                None => self.xdefs.get(&emitted_key(name)).cloned().flatten().and_then(|s| s.parse::<TokenStream>().ok()),
            },
            None => None,
        };
        let a = def.and_then(|t| parse_arms(&t)).map(std::rc::Rc::new);
        self.arms.insert(name.to_string(), a.clone());
        a
    }

    /// The invocation's macro, when it is a bare name this file defines once — or, R1004, a name this
    /// file does not define at all and the CRATE defines once (identically everywhere it is defined).
    fn local_name(&self, mac: &syn::Macro) -> Option<String> {
        if mac.path.get_ident().is_none() {
            return self.root_path_name(mac);
        }
        let id = mac.path.get_ident()?.to_string();
        if id == "macro_rules" {
            return None;
        }
        // SOUNDNESS R1541 / I1 step 1 — a name this file IMPORTS through a `use` that is not crate-rooted may be another
        // crate's macro (`use bitflags::bitflags;` beside a local wrapper also named `bitflags`, redox_syscall); binding it
        // to the local definition by name would mint the WRONG items. Refused: the invocation stays, and R128 discloses it.
        let bound = match self.defs.get(&id) {
            Some(d) => d.is_some() || self.scoped_def(&id).is_some(),
            None if self.emitted.contains(&id) && self.scope.contains_key(&id) => self.scoped_def(&id).is_some(),
            None => match self.xdefs.get(&id) {
                Some(d) => d.is_some(),
                None => !mdef_off() && self.xdefs.get(&emitted_key(&id)).is_some_and(|d| d.is_some()),
            },
        };
        if bound && self.imported.contains(&id) {
            if candor_report::env::trace("CANDOR_I1_INSTR") {
                eprintln!("I1REFUSE\timported\t{id}"); // §E1 REACH COUNTER
            }
            return None;
        }
        bound.then_some(id)
    }

    /// SOUNDNESS R1910 / R1853 (R-ROOT) — an item / impl-item / trait-item invocation by PATH binds only when the path's module
    /// part is the crate root (`crate::`, `$crate::` as transcribed, `self::`/`super^k::` reaching it) and the crate has ONE
    /// ungated root-namespace macro of the name (`rroot`): the key it is stored under in `xdefs`, which `arms_of` reads like any
    /// other crate-table name. Every other path is refused, exactly as before: the invocation stays, and R128 / L1 disclose it.
    fn root_path_name(&self, mac: &syn::Macro) -> Option<String> {
        let sc = self.f1.as_ref()?;
        if mac.path.leading_colon.is_some() || crate::rroot::off() {
            return None;
        }
        // NOT INSIDE A CFG ARM. Expanded there, the invocation stops being hidden text, and what stands in its place is a
        // PRODUCED cfg arm, whose union with the sibling arm the cfg-twin cover does not compute for every item kind — the
        // bare spellings `wuse!`/`wov!`/`wt!` read silent there on main (lane residue-C's unnumbered findings 1-3, the
        // cfg-twin design queue), and binding the path would carry that silence to it (soundness/cfgtwin d*v5, d*v7, d12*,
        // e1/e4/e9/e13/e15 measured 1->0). Refused, the path keeps main's hidden-text hold. Lift this when that cover is.
        if self.f1_gated > 0 || sc.gated {
            if candor_report::env::trace("CANDOR_I1_INSTR") {
                eprintln!("RROOTCFGREFUSE\t{}", crate::lang::path_to_string(&mac.path)); // §E1 REACH COUNTER
            }
            return None;
        }
        let segs: Vec<String> = mac.path.segments.iter().map(|s| s.ident.to_string()).collect();
        let (leaf, head) = segs.split_last()?;
        if !sc.is_root(head, self.modpath.len()) {
            return None;
        }
        let k = crate::rroot::key(&sc.root, leaf);
        let hit = matches!(self.xdefs.get(&k), Some(Some(_)));
        if hit && candor_report::env::trace("CANDOR_I1_INSTR") {
            eprintln!("RROOTITEM\t{}", segs.join("::")); // §E1 REACH COUNTER, on the CHANGED branch only
        }
        hit.then_some(k)
    }

    /// SOUNDNESS R1898 (cfg-twin lane B) — the invocation bound THIS file's definition of the name, but that definition is not
    /// yet in textual scope (`scope`) and another file defines the name differently (the crate table holds `None`, or an
    /// emitted definition of other tokens): rustc binds the earlier definition a `#[macro_use] mod` brought, not this file's
    /// later one, and which one that is is exactly what this table cannot say. The expansion is KEPT (a charge it makes is
    /// real for one of the two readings); only the disclosure is added (`mark_ambiguous`). Body position is R208's.
    fn pre_def_ambiguous(&self, mac: &syn::Macro) -> bool {
        if candor_report::env::switch("CANDOR_I1S3_SHADOW_OFF") {
            return false;
        }
        let Some(id) = mac.path.get_ident().map(|i| i.to_string()) else { return false };
        let Some(Some(own)) = self.defs.get(&id) else { return false };
        if self.scope.contains_key(&id) || self.emitted.contains(&id) {
            return false;
        }
        let own = own.to_string();
        let other_plain = matches!(self.xdefs.get(&id), Some(None));
        let other_emitted = match self.xdefs.get(&emitted_key(&id)) {
            Some(Some(t)) => *t != own,
            Some(None) => true,
            None => false,
        };
        let hit = other_plain || other_emitted;
        if hit && candor_report::env::trace("CANDOR_I1_INSTR") {
            eprintln!("R1898AMBIG\t{id}"); // §E1 REACH COUNTER, on the ADDED branch only
        }
        hit
    }

    /// The invocation's expansion tokens, and whether its definition came from ANOTHER file.
    fn expand_tokens(&mut self, mac: &syn::Macro, depth: usize) -> Option<(TokenStream, bool)> {
        if depth >= MAX_DEPTH || self.budget == 0 {
            return None;
        }
        let name = self.local_name(mac)?;
        let cross_file = !self.defs.contains_key(&name) && !(self.emitted.contains(&name) && self.scope.contains_key(&name));
        let arms = self.arms_of(&name)?;
        SEP2_USED.with(|c| c.set(false));
        let ts = expand(&arms, &mac.tokens)?;
        // R-ROOT — an expansion that writes a `cfg` of its own is a produced cfg arm too (see `root_path_name`): refused.
        if name.starts_with(crate::rroot::F1_PREFIX) && ts.to_string().contains("cfg") {
            if candor_report::env::trace("CANDOR_I1_INSTR") {
                eprintln!("RROOTCFGREFUSE\texpansion"); // §E1 REACH COUNTER
            }
            return None;
        }
        // SOUNDNESS R1748 (a) — §E1 REACH COUNTER for the two-character separator: a macro with such a separator in a
        // MATCHER was refused whole before (`rep_tail`), and one in the TRANSCRIBER (rkyv's `$(a == b)&&*`) failed at
        // transcription, so every expansion of either is new.
        if crate::lang::reach_debug() && (SEP2_USED.with(|c| c.get()) || arms.iter().any(|a| uses_sep2(&a.matcher))) {
            eprintln!("R1748SEP\t{name}");
        }
        if cross_file && candor_report::env::trace("CANDOR_R1004_INSTR") {
            eprintln!("R1004XFILE\t{name}"); // §E1 REACH COUNTER, on the CHANGED branch only
        }
        // SOUNDNESS R1541 / I1 stage 1 (Fable §7.7) — a SAME-FILE expansion is re-spanned to its invocation too, as a
        // cross-file one always was. With definition spans, N expansions of one macro published N units at ONE `loc`
        // inside the `macro_rules!` body (hickory-resolver's eleven `*_lookup` at `resolver.rs:51:9`), which is the input
        // the census mapper could not disambiguate (F1) and R1537's SAMELOC class. rustc's own expansion sites carry the
        // invocation as their outer span. `CANDOR_I1_RESPAN_OFF=1` keeps definition spans for a same-file macro.
        let ts = if cross_file || !candor_report::env::switch("CANDOR_I1_RESPAN_OFF") {
            respan_to(ts, mac.path.segments.first().map(|s| s.ident.span()).unwrap_or_else(proc_macro2::Span::call_site))
        } else {
            ts
        };
        Some((ts, cross_file))
    }

    fn expand_items(&mut self, mac: &syn::Macro, depth: usize) -> Option<Vec<syn::Item>> {
        let ambig = self.pre_def_ambiguous(mac);
        let (ts, cross_file) = self.expand_tokens(mac, depth)?;
        let name = mac.path.get_ident().map(|i| i.to_string()).unwrap_or_default();
        let mut file = syn::parse2::<syn::File>(ts).ok()?;
        if !file.attrs.is_empty() {
            return None; // an inner attribute at expansion top level is not something items carry
        }
        // A CONFIGURATION macro from another file (tokio's `cfg_rt! { .. }` / `cfg_not_rt! { .. }`, which
        // stamp `#[cfg(..)]` onto every item they wrap) is REFUSED. Its arms are split across invocations
        // of DIFFERENT macros, and this splice keeps only the `fn`s of each: `cfg_not_taskdump! { fn
        // trace_leaf() {} }` became a unit while its twin `cfg_taskdump! { use …::trace_leaf; }` (a `use`)
        // did not, so `crate::trace::trace_leaf()` resolved to the stub alone — one cfg arm where the engine
        // answers with the UNION (SPEC §4). Measured on the first cut's corpus A/B: tokio lost `Log` on 86
        // rows that way. Refused, the invocation reads exactly as before (R128's `macro:` evidence).
        //
        // I1 stage 2b FLIP — LIFTED BY DEFAULT; `CANDOR_I1S2_XCFG_OFF` (result-moving) restores the refusal. Stage 2a refused it
        // by default for the reason below; stage 2b made every fallback arm a superset, part B2 added the untyped-receiver
        // ARM-SET floor (Fable `fxmio2`), and the lift-ON arm was measured against 0.42.0 with every lost element slotted.
        // HISTORY — I1 stage 2a refused it by default. Stage 2 lifted it, on the argument that with modules and `use`s spliced the
        // twin arm (`cfg_taskdump! { use …::trace_leaf; }`) stands beside the stub and the union is whole. That holds only
        // where the twin arm's text EXPANDS; where it does not (another crate's macro, a nested invocation, an arm the matcher
        // cannot take) the lift turns the call site's `macro:` disclosure into a lone stub, and five review rounds of
        // hidden-twin holes were all restored by refusing here (Fable stage-2 RC3, `fxp`). The lift and its cover
        // (`mark_hidden_twins`) stayed behind an opt-in until every fallback arm was shown a superset (stage 2b);
        // palette's `impl_eq!` (`#[cfg(feature = "approx")]` on what it stamps) was the measured cost of refusing.
        let xcfg = cross_file
            && file.items.iter().any(|it| {
                crate::lang::item_attrs(it).iter().any(|a| a.path().is_ident("cfg") || a.path().is_ident("cfg_attr"))
            });
        if xcfg && xcfg_off() {
            if candor_report::env::trace("CANDOR_R1004_INSTR") {
                eprintln!("R1004XCFG\t{name}");
            }
            return None;
        }
        if xcfg && candor_report::env::trace("CANDOR_I1_INSTR") {
            eprintln!("I1S2XCFG\t{name}"); // §E1 REACH COUNTER, on the CHANGED branch only
        }
        self.budget -= 1;
        self.splice(&mut file.items, depth + 1);
        // R-ROOT — and one whose NESTED expansions wrote one (a bare local macro inside it): refused the same way.
        if mac.path.get_ident().is_none() && mentions_cfg(&file.items) {
            return None;
        }
        if ambig {
            mark_ambiguous_items(&mut file.items);
        }
        Some(file.items)
    }

    /// SOUNDNESS R1245 — an IMPL-ITEM-position invocation (`impl Report for Reports { reports_impl!(fn
    /// summarize(&self)); }`) expanded to the impl items it declares, under the same once-defined rule
    /// as an item-position one. `None` — the invocation stays exactly as written — when the macro is not one
    /// this crate defines once, no arm matches, the expansion is not a sequence of impl items, or (another
    /// file's macro) any generated item carries a `#[cfg]` (see `expand_items`).
    fn expand_impl_items(&mut self, mac: &syn::Macro, depth: usize) -> Option<Vec<syn::ImplItem>> {
        use syn::parse::Parser;
        let ambig = self.pre_def_ambiguous(mac);
        let (ts, cross_file) = self.expand_tokens(mac, depth)?;
        let mut gen = (|input: syn::parse::ParseStream| {
            let mut v = Vec::new();
            while !input.is_empty() {
                v.push(input.parse::<syn::ImplItem>()?);
            }
            Ok(v)
        })
        .parse2(ts)
        .ok()?;
        let xcfg = cross_file
            && gen.iter().any(|ii| impl_item_attrs(ii).iter().any(|a| a.path().is_ident("cfg") || a.path().is_ident("cfg_attr")));
        if xcfg && xcfg_off() {
            return None; // as for item position: lifted by default, `CANDOR_I1S2_XCFG_OFF` refuses it
        }
        if xcfg && candor_report::env::trace("CANDOR_I1_INSTR") {
            eprintln!("I1S2XCFG\timpl-item"); // §E1 REACH COUNTER, on the CHANGED branch only
        }
        self.budget -= 1;
        self.splice_impl_items(&mut gen, depth + 1);
        if mac.path.get_ident().is_none() && mentions_cfg(&gen) {
            return None; // R-ROOT, as at item position
        }
        if ambig {
            for g in gen.iter_mut() {
                if let syn::ImplItem::Fn(f) = g {
                    push_ambig(&mut f.attrs);
                }
            }
        }
        Some(gen)
    }

    /// SOUNDNESS R1245 — replace every impl-item-position invocation of a macro this crate defines once (in a
    /// trait impl or an inherent one) by the impl items it expands to. THE DEFECT: `syn` leaves `reports_impl!(fn summarize(&self));`
    /// opaque, so the impl had no `Reports::summarize` unit and a call on a `Reports` receiver bound to the
    /// trait's PURE default body — `criterion`'s `BenchmarkGroup::drop` read `[Net]` while its report fan-out
    /// writes files. The hand-written twin (`fn summarize(&self) { .. }` in the impl itself) was charged.
    ///
    /// A RESOLUTION, NOT A HEDGE — the same argument as R1004: the generated method is the program's own
    /// member of an impl that is ALREADY hand-written, so this adds no implementor to any dispatch universe
    /// (the concern that keeps a macro-generated TRAIT impl unexpanded); it only puts the member where rustc
    /// puts it. A refused expansion leaves the impl exactly as before — opaque, which the call site now discloses
    /// (`macro:impl members hidden by an unexpanded macro`, `scan.rs`).
    ///
    /// EVERY generated impl item is spliced (associated `const`s and `type`s too): unlike a generated `impl`, none of
    /// them can move a dispatch universe, and the block is then exactly what rustc compiles.
    fn splice_impl_items(&mut self, items: &mut Vec<syn::ImplItem>, depth: usize) {
        if !items.iter().any(|ii| matches!(ii, syn::ImplItem::Macro(_))) {
            return;
        }
        let mut out: Vec<syn::ImplItem> = Vec::with_capacity(items.len());
        for it in std::mem::take(items) {
            if let syn::ImplItem::Macro(m) = &it {
                let g = usize::from(crate::lang::has_cfg(&m.attrs) || m.attrs.iter().any(|a| a.path().is_ident("cfg_attr")));
                self.f1_gated += g;
                let gen = self.expand_impl_items(&m.mac, depth);
                self.f1_gated -= g;
                if let Some(mut gen) = gen {
                    for g in &mut gen {
                        // The invocation's own outer attributes (`#[cfg(..)]`) govern every item it expands to.
                        if let Some(a) = impl_item_attrs_mut(g) {
                            let mut na = m.attrs.clone();
                            na.append(a);
                            *a = na;
                        }
                    }
                    if candor_report::env::trace("CANDOR_R1245_INSTR") {
                        eprintln!(
                            "R1245EXPAND\t{}\t{}",
                            m.mac.path.get_ident().map(|i| i.to_string()).unwrap_or_default(),
                            gen.iter().filter(|g| matches!(g, syn::ImplItem::Fn(_))).count()
                        ); // §E1 REACH COUNTER, on the CHANGED branch only
                    }
                    // The invocation is REPLACED, not kept beside its expansion (unlike item position, where R128
                    // reads the invocation as module evidence): an impl-item macro left in the block marks the
                    // block OPAQUE (`lang.rs`, `impl_opaque_key`) — "its members are not all readable" — and once
                    // the expansion stands in its place that is no longer true. A nested invocation the expansion
                    // itself could not expand is still a `Macro` item in `gen`, so the block stays opaque for it.
                    out.extend(gen);
                    continue;
                }
            }
            out.push(it);
        }
        *items = out;
    }

    /// I1 stage 3 (Fable stage-3 review D1) — a TRAIT-ITEM-position invocation (`pub trait AsyncWriteExt { write_impl! { fn
    /// write_u8(..); } }`, tokio) expanded to the trait items it declares, under the impl-item rule (`expand_impl_items`). Left
    /// unexpanded, the provided methods it writes had no unit and a call to one read ABSENT; main hid that behind R452's
    /// name-keyed hedge only while R1784 kept the DEFINER partial. A RESOLUTION: the generated members are the trait's own.
    fn expand_trait_items(&mut self, mac: &syn::Macro, depth: usize) -> Option<Vec<syn::TraitItem>> {
        use syn::parse::Parser;
        let ambig = self.pre_def_ambiguous(mac);
        let (ts, cross_file) = self.expand_tokens(mac, depth)?;
        let mut gen = (|input: syn::parse::ParseStream| {
            let mut v = Vec::new();
            while !input.is_empty() {
                v.push(input.parse::<syn::TraitItem>()?);
            }
            Ok(v)
        })
        .parse2(ts)
        .ok()?;
        let xcfg = cross_file
            && gen.iter().any(|ti| trait_item_attrs(ti).iter().any(|a| a.path().is_ident("cfg") || a.path().is_ident("cfg_attr")));
        if xcfg && xcfg_off() {
            return None;
        }
        self.budget -= 1;
        self.splice_trait_items(&mut gen, depth + 1);
        if mac.path.get_ident().is_none() && mentions_cfg(&gen) {
            return None; // R-ROOT, as at item position
        }
        if ambig {
            for g in gen.iter_mut() {
                if let syn::TraitItem::Fn(f) = g {
                    if f.default.is_some() {
                        push_ambig(&mut f.attrs);
                    }
                }
            }
        }
        Some(gen)
    }

    fn splice_trait_items(&mut self, items: &mut Vec<syn::TraitItem>, depth: usize) {
        if trait_item_off() || !items.iter().any(|ti| matches!(ti, syn::TraitItem::Macro(_))) {
            return;
        }
        let mut out: Vec<syn::TraitItem> = Vec::with_capacity(items.len());
        for it in std::mem::take(items) {
            if let syn::TraitItem::Macro(m) = &it {
                let g = usize::from(crate::lang::has_cfg(&m.attrs) || m.attrs.iter().any(|a| a.path().is_ident("cfg_attr")));
                self.f1_gated += g;
                let gen = self.expand_trait_items(&m.mac, depth);
                self.f1_gated -= g;
                if let Some(mut gen) = gen {
                    for g in &mut gen {
                        if let Some(a) = trait_item_attrs_mut(g) {
                            let mut na = m.attrs.clone();
                            na.append(a);
                            *a = na;
                        }
                    }
                    if candor_report::env::trace("CANDOR_I1_INSTR") {
                        eprintln!("I1S3TRAITITEM\t{}", m.mac.path.get_ident().map(|i| i.to_string()).unwrap_or_default()); // §E1 REACH COUNTER
                    }
                    out.extend(gen);
                    continue;
                }
            }
            out.push(it);
        }
        *items = out;
    }

    /// R-BARE-TC (rust-macropath L3) — the `TOOLCHAIN_BANG_MACROS` names whose BARE spelling may not bind the toolchain's macro in
    /// THIS module's item list (`items`, the module at inline path `modkey` of this file; `None` reads the whole file). A name is
    /// rebound by: a definition of it in this crate (`tc_locals`); a non-toolchain `use` of it in this list; a `#[macro_use]
    /// extern crate dep;` of this file (`tc_mu`); or a glob of this module that may supply a name the crate's prelude does not
    /// make immune to it (`baretc::TcFile::glob_rebinds`: no glob rebinds a std-prelude name where the crate has std's prelude,
    /// and none a core-prelude name anywhere). Without facts for the file, R1899's reading (any glob of the file, any name).
    fn tc_names_for(&self, items: &[syn::Item], modkey: Option<&str>) -> std::collections::HashSet<String> {
        let imports = non_toolchain_imports_in(items, modkey.is_none());
        TOOLCHAIN_BANG_MACROS
            .iter()
            .filter(|n| {
                self.tc_locals.contains(**n)
                    || imports.contains(**n)
                    || self.tc_mu
                    || match (&self.tc, modkey) {
                        (Some(t), Some(k)) => t.glob_rebinds(n, k),
                        (Some(t), None) => t.glob_rebinds_anywhere(n),
                        (None, _) => self.imported.contains("*"),
                    }
            })
            .map(|n| n.to_string())
            .collect()
    }

    fn splice(&mut self, items: &mut Vec<syn::Item>, depth: usize) {
        // R-BARE-TC — this list's toolchain-name set, for the whole of its splice; the enclosing list's is put back at the end
        let tc_saved = self.tc.is_some().then(|| {
            let k = self.modpath.join("::");
            self.tc_shadow.replace(self.tc_names_for(items, Some(&k)))
        });
        let mut out: Vec<syn::Item> = Vec::with_capacity(items.len());
        // I1 stage 2 — which items of `out` the splice PRODUCED (`mark_hidden_twins`), index for index.
        let mut spliced: Vec<bool> = Vec::with_capacity(items.len());
        for mut it in std::mem::take(items) {
            if let syn::Item::Macro(m) = &it {
                if let (Some(name), true) = (&m.ident, m.mac.path.is_ident("macro_rules")) {
                    let gated = crate::lang::has_cfg(&m.attrs);
                    self.scope.insert(name.to_string(), (!gated).then(|| m.mac.tokens.to_string()));
                    self.scope_bodies.entry(name.to_string()).or_default().push(m.mac.tokens.clone());
                    // I1 stage 3 (R1784) — a definition met INSIDE an expansion (depth > 0) was emitted by a macro. It enters
                    // textual scope above like any other; a source definition of the same name no longer answers for the
                    // whole file (`None`: read from scope), since which one an invocation sees now depends on order.
                    if depth > 0 && !mdef_off() {
                        let n = name.to_string();
                        if candor_report::env::trace("CANDOR_I1_INSTR") {
                            eprintln!("I1S3MDEF\t{n}"); // §E1 REACH COUNTER, on the CHANGED branch only
                        }
                        if self.defs.get(&n).is_some_and(|d| d.as_ref().map(|t| t.to_string()) != Some(m.mac.tokens.to_string())) {
                            self.defs.insert(n.clone(), None);
                        }
                        self.emitted.insert(n);
                    }
                }
            }
            match &mut it {
                // I1 stage 3 (R1784) — mark a definition an expansion emitted (see `model::MBE_EMITTED_DEF_ATTR`).
                syn::Item::Macro(m) if m.ident.is_some() && depth > 0 && m.mac.path.is_ident("macro_rules") && !mdef_off() => {
                    if !m.attrs.iter().any(|a| a.path().is_ident(crate::model::MBE_EMITTED_DEF_ATTR)) {
                        let id = syn::Ident::new(crate::model::MBE_EMITTED_DEF_ATTR, proc_macro2::Span::call_site());
                        m.attrs.push(syn::parse_quote!(#[#id]));
                    }
                }
                syn::Item::Macro(m) if m.ident.is_none() && !seed_dropped(&m.mac) => {
                    let g = usize::from(crate::lang::has_cfg(&m.attrs) || m.attrs.iter().any(|a| a.path().is_ident("cfg_attr")));
                    self.f1_gated += g;
                    let gen = self.expand_items(&m.mac, depth);
                    self.f1_gated -= g;
                    if let Some(gen) = gen {
                        // SOUNDNESS R1748 (a) — the TRAITS the expansion declares are not spliced (see below), but their
                        // existence is recorded: one inert `Item::Verbatim` marker per generated trait, naming it and its
                        // `fn` members, which `lang::collect_mbe_traits` reads into the decl index. No other walk reads a
                        // `Verbatim` item. serde_core's `declare_error_trait!(Error: ..)` declares `de::Error` / `ser::Error`
                        // this way, and without the record a call naming either was keyed under the only HAND-WRITTEN
                        // `Error` the crate has (`std_error::Error`, which declares no `custom`).
                        let mut markers: Vec<syn::Item> = gen
                            .iter()
                            .filter_map(|g| match g {
                                syn::Item::Trait(t) => mbe_trait_marker(t),
                                _ => None,
                            })
                            .collect();
                        // SOUNDNESS R1754 — …and the generated trait's PROVIDED bodies, which run for every implementor that
                        // does not override them (serde_core's `de::Error::invalid_type`/`missing_field`/… each call
                        // `Error::custom`). One inert marker per trait with a default body (`mbe_trait_body_marker`), read
                        // ONLY by the unit walk and its loc mirror (`decls.rs`), so the bodies become units under the trait's
                        // qual while every decl index (`trait_decls`, the leaf CHA, `local_traits`) stays exactly as R1748 left
                        // it. The invocation's own attributes (`#[cfg]`) ride on the trait, as on every spliced item.
                        markers.extend(gen.iter().filter_map(|g| match g {
                            syn::Item::Trait(t) => mbe_trait_body_marker(t, &m.attrs),
                            _ => None,
                        }));
                        // SOUNDNESS R1541 / I1 — EXPAND, DO NOT FILTER. Every item the expansion parses as is spliced beside
                        // the invocation, as rustc places it, EXCEPT the refusal list (`i1_refused`): a nested invocation this
                        // pass could not expand, and traits (the R1748/R1754 markers above stand
                        // for them). This replaced an ALLOWLIST that grew one item kind per row (free fns, then inherent impls,
                        // then operator impls — R1004 → R1146 → R1157 → R1180 → R1748 → R1754): a macro-made TRAIT impl had no
                        // unit (R1541: `<AtomicU32 as LoadIt>::load_it` read pure over a write), a macro-made struct left its
                        // impls keyed to no declared type (R1678's class), and a macro-made module or `use` left the names it
                        // binds unread (R1535, R1688). Stage 1 spliced impls and declarations; stage 2 deleted the allowlist
                        // and splices modules, `use`s and `extern crate`s too.
                        let seed_name = m.mac.path.get_ident().map(|i| i.to_string()).unwrap_or_default();
                        let seed_body = !seed_name.is_empty()
                            && candor_report::env::switch_value(
                                "CANDOR_I1_SEED_BODY",
                                candor_report::env::Selects::AnyOf(&[seed_name.as_str()]),
                            );
                        // I1 stage 3 (D1) — a produced trait that still holds an unexpanded trait-item invocation keeps the invocation
                        // PARTIAL: its text stays hidden evidence (R452), the cover main read through, since no unit stands for
                        // the members it would write.
                        // At ANY module depth of the expansion (Fable re-check 1, D1′: a trait inside an EMITTED `mod` read as
                        // full here and its definer was retired while its member invocation stayed hidden; `ftrmod`).
                        let full = gen.iter().all(|g| !i1_refused(g) || matches!(g, syn::Item::Trait(_)))
                            && !holds_unexpanded_trait_member(&gen);
                        if candor_report::env::trace("CANDOR_I1_INSTR") {
                            for (g, k) in gen.iter().filter_map(|g| stage2_kind(g).map(|k| (g, k))) {
                                if !i1_refused(g) {
                                    eprintln!("I1S2SPLICE\t{k}\t{seed_name}"); // §E1 REACH COUNTER, on the CHANGED branch only
                                }
                            }
                        }
                        // A nested invocation the expansion could not itself expand is KEPT in the output (Fable §1.3): it is
                        // still unexpanded program text, so R128 must see it as module evidence and `expansion_facts` must
                        // count its macro as invoked-unexpanded — dropped, it made the crate-wide retirement treat
                        // `__impl_public_bitflags` (bitflags 2.9.4, invoked by path inside `__impl_internal_bitflags`) as
                        // fully expanded and retire the `fn bits` evidence a `.bits()` call still needed:
                        // `parser::from_str_truncate` lost its `Flags::bits` key.
                        let mut fns: Vec<syn::Item> = gen
                            .into_iter()
                            .filter(|g| !i1_refused(g) || matches!(g, syn::Item::Macro(n) if n.ident.is_none()))
                            .collect();
                        if !fns.is_empty() && candor_report::env::trace("CANDOR_R1004_INSTR") {
                            eprintln!(
                                "R1004EXPAND\t{}\t{}",
                                m.mac.path.get_ident().map(|i| i.to_string()).unwrap_or_default(),
                                fns.len()
                            );
                        }
                        // The invocation's own outer attributes (`#[cfg(..)]` on `foo!(..);`) govern every
                        // item it expands to.
                        for g in &mut fns {
                            // A `$t:ty` fragment transcribes as an invisible-delimited GROUP, which no
                            // written impl has; peel it so the impl's units are keyed under the type
                            // (`W::touch`) exactly as the hand-written impl's are.
                            if let syn::Item::Impl(i) = g {
                                // R1157 — a primitive-self operator impl is marked, so Pass A files it apart from a
                                // hand-written one and the operator site can refuse it a LITERAL right operand.
                                if prim_operator_impl(i) {
                                    let id = syn::Ident::new(crate::model::MBE_PRIM_ATTR, proc_macro2::Span::call_site());
                                    i.attrs.push(syn::parse_quote!(#[#id]));
                                    if candor_report::env::trace("CANDOR_V053_INSTR") {
                                        eprintln!("R1157SPLICE"); // §E1 REACH COUNTER
                                    }
                                }
                                // R1180 — a nominal-self operator impl is marked too: where a hand-written impl claims
                                // the same `Type::method` tail, its units must not contest that claimant (`scan.rs`).
                                if !prim_operator_impl(i) && operator_trait_impl(i) {
                                    let id = syn::Ident::new(crate::model::MBE_OP_ATTR, proc_macro2::Span::call_site());
                                    i.attrs.push(syn::parse_quote!(#[#id]));
                                }
                                if candor_report::env::trace("CANDOR_R1004_INSTR") {
                                    eprintln!("R1004IMPL\t{}", i.items.len()); // §E1 REACH PROBE
                                }
                                // R1146 — at every depth, not only the top: `impl Add<&'a $t> for &'a $t` holds the
                                // group under a reference (and in the trait's `Rhs`), which `operator_ref_self` and
                                // the `Rhs` index read through.
                                peel_groups(&mut i.self_ty);
                                if let Some((_, tp, _)) = &mut i.trait_ {
                                    for seg in tp.segments.iter_mut() {
                                        if let syn::PathArguments::AngleBracketed(ab) = &mut seg.arguments {
                                            for a in ab.args.iter_mut() {
                                                if let syn::GenericArgument::Type(t) = a {
                                                    peel_groups(t);
                                                }
                                            }
                                        }
                                    }
                                }
                            }
                            // I1 stage 1 — a `$t:ty` group inside a spliced DECLARATION is peeled too, so a field, an alias or a
                            // const typed by a fragment reads as the hand-written spelling does.
                            peel_decl_groups(g);
                            // I1 ORACLE SEED (an instrument, recorded as a switch): `CANDOR_I1_SEED_BODY=<macro>` empties every
                            // fn body this macro's expansion yields, so the unit exists under the right name with the WRONG body.
                            // `eval/i1/xcheck.py`'s effect-equality leg must report it; a presence-only oracle cannot.
                            if seed_body {
                                empty_bodies(g);
                            }
                            if let Some(a) = crate::lang::item_attrs_mut(g) {
                                let mut na = m.attrs.clone();
                                na.append(a);
                                *a = na;
                            }
                        }
                        // I1 stage 1 — ONE fact, "this invocation was expanded": a FULLY expanded invocation is REPLACED by an
                        // inert marker (`mbe_expanded_of`), so every model that reads an item-position `Item::Macro` as hidden
                        // text (R128's module evidence, R452/R1636's names, `typesurf`'s unclosable mark, the module-macro
                        // `use`-map key) no longer re-discloses what now stands spliced beside it; R828 and R452's
                        // `macro_rules!`-BODY evidence is retired crate-wide from the same markers (`cache::retire_expanded`).
                        // A partial expansion (anything on the refusal list) keeps the invocation, and with it every hedge.
                        if full {
                            // R-ROOT — a path invocation (`crate::mkfn!`) is named by its LEAF, the name `expansion_facts` keys
                            // an unexpanded one under; only R-ROOT expands a path, so a bare name reads exactly as before.
                            let name = m.mac.path.segments.last().map(|s| s.ident.to_string()).unwrap_or_default();
                            if candor_report::env::trace("CANDOR_I1_INSTR") {
                                eprintln!("I1EXPANDED\t{name}"); // §E1 REACH COUNTER
                            }
                            match format!("{MBE_EXPANDED_MARK} {name}").parse::<TokenStream>() {
                                Ok(ts) => out.push(syn::Item::Verbatim(ts)),
                                Err(_) => out.push(it),
                            }
                        } else {
                            if candor_report::env::trace("CANDOR_I1_INSTR") {
                                eprintln!(
                                    "I1PARTIAL\t{}",
                                    m.mac.path.get_ident().map(|i| i.to_string()).unwrap_or_default()
                                ); // §E1 REACH COUNTER
                            }
                            self.partial.insert(invocation_key(&m.mac));
                            out.push(it);
                        }
                        // I1 stage 3 (R1813) — a produced item's bodies were already read for body-position impls inside the
                        // expansion's own splice (`expand_items` → `splice`), at every module depth of it, where `scope` is the
                        // textual scope at the item; the hoisted impls stand among `fns`. (Reading them HERE, at the top level
                        // only, missed a host inside an emitted module: `u_modhoist`, Fable re-check 1's question.)
                        spliced.resize(out.len(), false);
                        spliced.resize(out.len() + fns.len(), true);
                        out.extend(fns);
                        out.append(&mut markers);
                        continue;
                    }
                }
                // R1245 — a hand-written impl whose members a local macro declares.
                syn::Item::Impl(im) => {
                    let g = usize::from(crate::lang::has_cfg(&im.attrs) || im.attrs.iter().any(|a| a.path().is_ident("cfg_attr")));
                    self.f1_gated += g;
                    self.splice_impl_items(&mut im.items, depth);
                    self.f1_gated -= g;
                }
                // I1 stage 3 (D1) — a trait whose members a local macro declares.
                syn::Item::Trait(t) => {
                    let g = usize::from(crate::lang::has_cfg(&t.attrs) || t.attrs.iter().any(|a| a.path().is_ident("cfg_attr")));
                    self.f1_gated += g;
                    self.splice_trait_items(&mut t.items, depth);
                    self.f1_gated -= g;
                }
                syn::Item::Mod(md) => {
                    let g = usize::from(crate::lang::has_cfg(&md.attrs) || md.attrs.iter().any(|a| a.path().is_ident("cfg_attr")));
                    if let Some((_, inner)) = &mut md.content {
                        self.f1_gated += g;
                        // R1156 — a child sees the parent's textual scope; its own definitions leave with it
                        // unless the module is `#[macro_use]`.
                        let saved = self.scope.clone();
                        let saved_bodies = self.scope_bodies.clone();
                        self.modpath.push(md.ident.to_string());
                        self.splice(inner, depth);
                        self.modpath.pop();
                        self.f1_gated -= g;
                        if !md.attrs.iter().any(|a| a.path().is_ident("macro_use")) {
                            self.scope = saved;
                            self.scope_bodies = saved_bodies;
                        }
                    }
                }
                _ => {}
            }
            // I1 stage 3 (R1813) — the impls a BODY-position invocation in this item emits, beside it (`hoist_body_impls`). Read
            // HERE, in the loop, so `scope` is the textual scope at the item (a later redefinition does not answer for it).
            let hoisted = self.hoist_body_impls(&it, depth);
            out.push(it);
            if !hoisted.is_empty() {
                spliced.resize(out.len(), false);
                spliced.resize(out.len() + hoisted.len(), true);
                out.extend(hoisted);
            }
        }
        // R-BARE-TC — read again from the SPLICED list: a `use` an expansion emitted is an import of this module too
        if tc_saved.is_some() {
            let k = self.modpath.join("::");
            let now = self.tc_names_for(&out, Some(&k));
            self.tc_shadow.replace(now);
        }
        if !candor_report::env::switch("CANDOR_I1S2_TWIN_OFF") {
            spliced.resize(out.len(), false);
            let (mut item, mut member) = (Hidden::default(), Hidden::default());
            for it in out.iter() {
                match it {
                    syn::Item::Macro(m) if is_hidden_text(m, &self.partial, &self.tc_shadow.borrow()) => {
                        let mut h = Hidden::default();
                        self.read_hidden(&m.mac, None, &out, &mut h, 0);
                        if candor_report::env::trace("CANDOR_I1_INSTR") {
                            eprintln!("I1S2HIDBY\t{}", invocation_key(&m.mac).split('!').next().unwrap_or("")); // §E1 REACH COUNTER
                        }
                        item.absorb(h);
                    }
                    syn::Item::Impl(im) => {
                        let owner = ty_key(&im.self_ty);
                        for ii in &im.items {
                            if let syn::ImplItem::Macro(m) = ii {
                                let mut h = Hidden::default();
                                self.read_hidden(&m.mac, Some(&owner), &out, &mut h, 0);
                                member.absorb(h);
                            }
                        }
                    }
                    // I1 stage 2c — a TRAIT-ITEM-position invocation left unexpanded (`trait Tr { hid! { fn go { .. } }
                    // #[cfg(not(unix))] fn go(&self) {} }`) is the trait's own hidden member text, keyed `Tr::m` (`rd_e`).
                    syn::Item::Trait(t) if !candor_report::env::switch("CANDOR_I1S2C_XMOD_OFF") => {
                        let owner = t.ident.to_string();
                        for ti in &t.items {
                            if let syn::TraitItem::Macro(m) = ti {
                                let mut h = Hidden::default();
                                self.read_hidden(&m.mac, Some(&owner), &out, &mut h, 0);
                                member.absorb(h);
                            }
                        }
                    }
                    _ => {}
                }
            }
            // I1 stage 2b — does this module still hold unexpanded ITEM text at all (whatever its reading came to)?
            let has_hidden = out.iter().any(|it| matches!(it, syn::Item::Macro(m) if is_hidden_text(m, &self.partial, &self.tc_shadow.borrow())));
            mark_hidden_twins(&mut out, &spliced, &item, &member, has_hidden);
            // SOUNDNESS R1857 — cfg-twin MODULES: one arm's hidden text, at any depth of it, may define what the other arm
            // shows (`mark_module_twins`).
            if !r1857_off() {
                self.mark_module_twins(&mut out, &spliced, depth);
            }
            // I1 stage 2c — export this module's impl-scoped hidden keys (and, for the measured `all` scope, its path keys)
            // for the crate-wide check at pass B (`cache::twin_facts`).
            if !candor_report::env::switch("CANDOR_I1S2C_XMOD_OFF") {
                let mut ks: Vec<String> = item.xkeys.iter().chain(&member.xkeys).cloned().collect();
                ks.extend(item.keys.iter().map(|k| format!("p:{k}")));
                ks.sort();
                ks.dedup();
                if !ks.is_empty() {
                    let mut ts = TokenStream::new();
                    ts.extend([TokenTree::Ident(Ident::new(XKEYS_MARK, proc_macro2::Span::call_site()))]);
                    for k in ks {
                        ts.extend([TokenTree::Literal(proc_macro2::Literal::string(&k))]);
                    }
                    out.push(syn::Item::Verbatim(ts));
                }
            }
        }
        // SOUNDNESS R1853 (rust-macropath L1(a)) — the names this module's unexpanded, LOCAL-READABLE item / impl-item / trait-item
        // text declares, exported for the CALLER-side hedge (`minted_facts`, `scan.rs`). Additive: a marker nothing else reads.
        if let Some(mk) = self.minted_marker(&out) {
            out.push(mk);
        }
        if let Some(saved) = tc_saved {
            self.tc_shadow.replace(saved);
        }
        *items = out;
    }

    /// SOUNDNESS R1853 (rust-macropath L1(a)) — WHAT THE UNEXPANDED LOCAL TEXT OF THIS MODULE DECLARES.
    ///
    /// R128 discloses only the PATH-qualified caller of a module holding unexpanded item text. A bare same-module call, a
    /// `use`-imported one (named, renamed, glob), a method on a type whose INHERENT impl holds an unexpanded impl-item
    /// invocation, and a method on a trait holding an unexpanded trait-item invocation read ABSENT over an executed write. The
    /// names such text can declare are readable when the invocation is of a LOCAL macro (`read_hidden`, the one reader), which
    /// is the whole of this arm: an invocation of another crate's macro, a `$m!`, a token-paster or an unreadable body reads
    /// OPAQUE and is NOT recorded (its names are unknowable; that is the foreign arm, a separate question).
    ///
    /// LOCAL means the path proves it. A single-segment name this file does not import; or a path headed `crate`/`self`/`super`
    /// or by a module declared in this same item list (an in-scope item shadows the extern prelude). `dep::mk!` whose leaf
    /// happens to equal a local macro's name binds the DEPENDENCY, and reading the local body for it would record wrong names:
    /// refused, not guessed.
    ///
    /// ENTRIES (string literals after the marker ident): `F\u{1}<rel path>::<name>` (a free name, relative to this module) and
    /// `M\u{1}<Owner>\u{1}<name>` (a member of an inherent impl's self type, or of a trait). Trait impls are R1245's.
    fn minted_marker(&self, out: &[syn::Item]) -> Option<syn::Item> {
        let table = BodyTable { scoped: None, bodies: Some(&self.bodies), xdefs: Some(self.xdefs) };
        let local = |mac: &syn::Macro| -> bool {
            let leaf = mac.path.segments.last().map(|s| s.ident.to_string()).unwrap_or_default();
            if mac.path.leading_colon.is_some() || table.get(&leaf).is_none() {
                return false;
            }
            if mac.path.segments.len() == 1 {
                return !self.imported.contains(&leaf) && !self.imported.contains("*");
            }
            let head = mac.path.segments.first().map(|s| s.ident.to_string()).unwrap_or_default();
            matches!(head.as_str(), "crate" | "self" | "super") || out.iter().any(|it| matches!(it, syn::Item::Mod(m) if m.ident == head))
        };
        let mut ents: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
        let mut absorb = |h: &Hidden, owner: Option<&str>| {
            if h.opaque {
                return;
            }
            for k in h.keys.iter().chain(h.xkeys.iter()) {
                if k.starts_with('=') || k.starts_with('*') || k.starts_with("p:") {
                    continue;
                }
                // `S as Tr` / `S as Tr::m`: a trait impl the text writes; its member `m` is the type's.
                if let Some((o, rest)) = k.split_once(" as ") {
                    if let Some((_, m)) = rest.rsplit_once("::") {
                        ents.insert(format!("M\u{1}{o}\u{1}{m}"));
                    }
                    continue;
                }
                match k.rsplit_once("::") {
                    // `Owner::m` (an impl member, or a trait's) AND `module::name` (a path item in a module the text declares):
                    // both readings recorded, the over-approximation direction.
                    Some((o, m)) => {
                        let o_leaf = o.rsplit("::").next().unwrap_or(o);
                        ents.insert(format!("M\u{1}{o_leaf}\u{1}{m}"));
                        ents.insert(format!("F\u{1}{k}"));
                    }
                    None => {
                        ents.insert(format!("F\u{1}{k}"));
                    }
                }
            }
            for l in &h.loose {
                match owner {
                    Some(o) => {
                        ents.insert(format!("M\u{1}{o}\u{1}{l}"));
                    }
                    None => {
                        ents.insert(format!("F\u{1}{l}"));
                    }
                }
            }
        };
        // R1853 L1(b) COUNTER FACTS (rust-l1b-r1911) — which modules hold item-position hidden text the reader can read in full
        // (`local`) and which hold text it cannot (`opaque`: a dependency macro, a `$m!`, a token-paster, an unreadable `include!`,
        // or a past-the-bound read). Collected ALWAYS and read ONLY by the `CANDOR_L1B_INSTR` counter's stderr lines (`scan.rs`):
        // no verdict reads them, and they are not part of `decl_index_digest`. Never gated on the environment, so the trace read
        // stays a diagnostic one and the default output cannot depend on it.
        let (mut l1b_local, mut l1b_opaque) = (false, false);
        for it in out {
            match it {
                syn::Item::Macro(m) if is_hidden_text(m, &self.partial, &self.tc_shadow.borrow()) && !local(&m.mac) => {
                    l1b_opaque = true;
                }
                syn::Item::Macro(m) if is_hidden_text(m, &self.partial, &self.tc_shadow.borrow()) && local(&m.mac) => {
                    let mut h = Hidden::default();
                    self.read_hidden(&m.mac, None, out, &mut h, 0);
                    if h.opaque {
                        l1b_opaque = true;
                    } else {
                        l1b_local = true;
                    }
                    absorb(&h, None);
                }
                syn::Item::Impl(im) if im.trait_.is_none() => {
                    let owner = ty_key(&im.self_ty);
                    for ii in &im.items {
                        if let syn::ImplItem::Macro(m) = ii {
                            if local(&m.mac) {
                                let mut h = Hidden::default();
                                self.read_hidden(&m.mac, Some(&owner), out, &mut h, 0);
                                absorb(&h, Some(&owner));
                            }
                        }
                    }
                }
                syn::Item::Trait(t) => {
                    let owner = t.ident.to_string();
                    for ti in &t.items {
                        if let syn::TraitItem::Macro(m) = ti {
                            if local(&m.mac) {
                                let mut h = Hidden::default();
                                self.read_hidden(&m.mac, Some(&owner), out, &mut h, 0);
                                absorb(&h, Some(&owner));
                            }
                        }
                    }
                }
                _ => {}
            }
        }
        if l1b_opaque {
            ents.insert("O\u{1}opaque".to_string());
        }
        if l1b_local {
            ents.insert("O\u{1}local".to_string());
        }
        if ents.is_empty() {
            return None;
        }
        if candor_report::env::trace("CANDOR_R1853L1_INSTR") {
            eprintln!("R1853MINTED\t{ents:?}"); // §E1 REACH COUNTER
        }
        let mut ts = TokenStream::new();
        ts.extend([TokenTree::Ident(Ident::new(MINTED_MARK, proc_macro2::Span::call_site()))]);
        for e in ents {
            ts.extend([TokenTree::Literal(proc_macro2::Literal::string(&e))]);
        }
        Some(syn::Item::Verbatim(ts))
    }
}

/// SOUNDNESS R1853 (rust-macropath L1(a)) — the marker `Ctx::minted_marker` writes.
const MINTED_MARK: &str = "__candor_macro_minted";

/// SOUNDNESS R1853 (rust-macropath L1(a)) — what a file's unexpanded local text declares, crate-wide keyed: `(free, members)`,
/// `free` = `"<module qual>\u{1}<name>"` and `members` = `"<Owner>\u{1}<name>"`. Walked over the SPLICED items, so a marker an
/// expansion wrote into a module it emitted is read at that module's qual. A `#[cfg(test)]` module is skipped as every other
/// collector skips it.
pub(crate) fn minted_facts(items: &[syn::Item], modpath: &str, include_tests: bool) -> (Vec<String>, Vec<String>, Vec<String>) {
    fn walk(items: &[syn::Item], modpath: &str, include_tests: bool, free: &mut Vec<String>, mem: &mut Vec<String>, kinds: &mut Vec<String>) {
        for it in items {
            match it {
                syn::Item::Verbatim(ts) => {
                    let mut toks = ts.clone().into_iter();
                    if matches!(toks.next(), Some(TokenTree::Ident(a)) if a == MINTED_MARK) {
                        for t in toks {
                            let TokenTree::Literal(l) = t else { continue };
                            let Ok(syn::Lit::Str(s)) = syn::parse2::<syn::Lit>(TokenTree::Literal(l).into()) else { continue };
                            let v = s.value();
                            if let Some(rest) = v.strip_prefix("F\u{1}") {
                                let (rel_mod, name) = match rest.rsplit_once("::") {
                                    Some((m, n)) => (m.to_string(), n.to_string()),
                                    None => (String::new(), rest.to_string()),
                                };
                                let q = match (modpath.is_empty(), rel_mod.is_empty()) {
                                    (true, _) => rel_mod,
                                    (false, true) => modpath.to_string(),
                                    (false, false) => format!("{modpath}::{rel_mod}"),
                                };
                                free.push(format!("{q}\u{1}{name}"));
                            } else if let Some(rest) = v.strip_prefix("M\u{1}") {
                                mem.push(rest.to_string());
                            } else if let Some(kind) = v.strip_prefix("O\u{1}") {
                                kinds.push(format!("{modpath}\u{1}{kind}")); // the L1(b) counter's module facts
                            }
                        }
                    }
                }
                syn::Item::Mod(m) if include_tests || !crate::lang::is_cfg_test(&m.attrs) => {
                    if let Some((_, inner)) = &m.content {
                        let sub = if modpath.is_empty() { m.ident.to_string() } else { format!("{modpath}::{}", m.ident) };
                        walk(inner, &sub, include_tests, free, mem, kinds);
                    }
                }
                _ => {}
            }
        }
    }
    let (mut f, mut m, mut k) = (Vec::new(), Vec::new(), Vec::new());
    walk(items, modpath, include_tests, &mut f, &mut m, &mut k);
    for v in [&mut f, &mut m, &mut k] {
        v.sort();
        v.dedup();
    }
    (f, m, k)
}

/// I1 stage 3 (R1813) — one body-position invocation's expansion, read: the impls it emits (at any block depth of the
/// expansion, and through the invocations nested in it), and whether some part could not be read.
#[derive(Default)]
struct BodyRead {
    impls: Vec<syn::ItemImpl>,
    /// an invocation of a CRATE-LOCAL macro whose bodies write `impl` could not be expanded or parsed
    unread: bool,
    /// every name an EXPANSION declares as an item (fn, type, const, static, trait, module, `macro_rules!`), at any block depth,
    /// and `\u{1}use` when one writes a `use` (Fable stage-3 review D4: the refusal read the host's text only)
    declared: std::collections::HashSet<String>,
}

/// I1 stage 3 (R1813) — the visitor over a host item's bodies. `overlay` is the body-local `macro_rules!` scope (R142's, and
/// definitions a body-position expansion emits, R1784): name → arm tokens, `None` for a `#[cfg]`-gated one (no textual order
/// picks between cfg twins). `in_expansion` is false over the host's own text: a HAND-WRITTEN impl in a body is SOUNDNESS
/// R1257's, not hoisted here.
struct BodyWalk<'c, 'x> {
    cx: &'c mut Ctx<'x>,
    overlay: HashMap<String, Option<String>>,
    in_expansion: bool,
    depth: usize,
    out: BodyRead,
}

impl BodyWalk<'_, '_> {
    fn invocation(&mut self, mac: &syn::Macro) {
        let foreign = |mac: &syn::Macro| {
            // §E1 REACH COUNTER — a body-position invocation of a macro this pass does not read (another crate's, by path or
            // by name): SPEC §2's closability rule counts it too, and `typesurf` does not mark it (pre-existing, measured).
            if candor_report::env::trace("CANDOR_I1_INSTR") {
                let root = mac.path.segments.first().map(|s| s.ident.to_string()).unwrap_or_default();
                let leaf = mac.path.segments.last().map(|s| s.ident.to_string()).unwrap_or_default();
                let tc = (mac.path.segments.len() == 1 || matches!(root.as_str(), "std" | "core" | "alloc"))
                    && TOOLCHAIN_BANG_MACROS.contains(&leaf.as_str());
                if !tc {
                    eprintln!("I1S3BODYFOREIGN\t{root}::{leaf}");
                }
            }
        };
        let Some(id) = mac.path.get_ident().map(|i| i.to_string()) else {
            foreign(mac);
            return;
        };
        if id == "macro_rules" || self.depth >= MAX_DEPTH || self.cx.budget == 0 {
            return;
        }
        // which definition: the body-local one in textual scope, else the module's (`local_name`, the item splice's rule)
        let def: Option<Option<std::rc::Rc<Vec<Arm>>>> = match self.overlay.get(&id) {
            Some(Some(t)) => Some(t.parse::<TokenStream>().ok().and_then(|t| parse_arms(&t)).map(std::rc::Rc::new)),
            Some(None) => Some(None),
            None => self.cx.local_name(mac).map(|n| self.cx.arms_of(&n)),
        };
        let Some(def) = def else {
            foreign(mac); // not a crate-local macro: another crate's or the toolchain's (pre-existing)
            return;
        };
        let may_impl = |arms: &Option<std::rc::Rc<Vec<Arm>>>, cx: &Ctx<'_>| match arms {
            Some(a) => a.iter().any(|arm| writes_impl(&arm.body)),
            None => cx.bodies.get(&id).is_none_or(|bs| bs.iter().any(writes_impl)),
        };
        let ts = def.as_ref().and_then(|arms| expand(arms, &mac.tokens));
        let Some(ts) = ts else {
            if may_impl(&def, self.cx) {
                self.out.unread = true;
            }
            return;
        };
        let ts = respan_to(ts, mac.path.segments.first().map(|s| s.ident.span()).unwrap_or_else(proc_macro2::Span::call_site));
        let braced = TokenStream::from(TokenTree::Group(Group::new(Delimiter::Brace, ts)));
        let Ok(block) = syn::parse2::<syn::Block>(braced) else {
            if may_impl(&def, self.cx) {
                self.out.unread = true;
            }
            return;
        };
        self.cx.budget -= 1;
        let (saved_in, saved_depth) = (self.in_expansion, self.depth);
        self.in_expansion = true;
        self.depth += 1;
        // The expansion's top-level statements are the ENCLOSING block's (a statement-position invocation is spliced there),
        // so a definition it emits stays in the overlay after it (R1784, `bm_def!(); bm_eff!();`).
        self.stmts(&block.stmts);
        self.in_expansion = saved_in;
        self.depth = saved_depth;
    }

    fn stmts(&mut self, stmts: &[syn::Stmt]) {
        for st in stmts {
            match st {
                syn::Stmt::Item(syn::Item::Macro(m)) if m.ident.is_some() && m.mac.path.is_ident("macro_rules") => {
                    let n = m.ident.as_ref().map(|i| i.to_string()).unwrap_or_default();
                    if self.in_expansion {
                        self.out.declared.insert(n.clone());
                    }
                    let gated = crate::lang::has_cfg(&m.attrs);
                    self.overlay.insert(n, (!gated).then(|| m.mac.tokens.to_string()));
                }
                _ => syn::visit::Visit::visit_stmt(self, st),
            }
        }
    }
}

impl<'ast> syn::visit::Visit<'ast> for BodyWalk<'_, '_> {
    fn visit_block(&mut self, b: &'ast syn::Block) {
        let saved = self.overlay.clone();
        self.stmts(&b.stmts);
        self.overlay = saved;
    }
    fn visit_macro(&mut self, m: &'ast syn::Macro) {
        self.invocation(m);
    }
    fn visit_item_impl(&mut self, im: &'ast syn::ItemImpl) {
        if self.in_expansion {
            self.out.impls.push(im.clone());
        }
        syn::visit::visit_item_impl(self, im);
    }
    // I1 stage 3 (D4) — what an expansion DECLARES, read by the hoist's refusal beside the host's own declarations.
    fn visit_item(&mut self, it: &'ast syn::Item) {
        if self.in_expansion {
            match it {
                syn::Item::Use(_) => {
                    self.out.declared.insert("\u{1}use".to_string());
                }
                syn::Item::Impl(_) => {}
                syn::Item::Macro(m) => {
                    if let Some(i) = &m.ident {
                        self.out.declared.insert(i.to_string());
                    }
                }
                other => {
                    if let Some(n) = crate::decls::declared_item_name(other, true) {
                        self.out.declared.insert(n);
                    }
                }
            }
        }
        syn::visit::visit_item(self, it);
    }
    // An IMPL-ITEM / TRAIT-ITEM invocation is not a statement: its expansion is members of that impl or trait (read by
    // `splice_impl_items` on the hoisted impl), not items of the block. Read as statements, rand 0.10's
    // `impl Fill for $t { impl_fill!(fill_slice! ..) }` counted `fill_slice` as a block declaration and refused its own impl.
    fn visit_impl_item_macro(&mut self, _: &'ast syn::ImplItemMacro) {}
    fn visit_trait_item_macro(&mut self, _: &'ast syn::TraitItemMacro) {}
    // a module nested in a body is its own scope: not read here (the residual is counted, `I1S3BODYMOD`)
    fn visit_item_mod(&mut self, m: &'ast syn::ItemMod) {
        if candor_report::env::trace("CANDOR_I1_INSTR") {
            eprintln!("I1S3BODYMOD\t{}", m.ident); // §E1 REACH COUNTER
        }
    }
}

/// Does a `macro_rules!` arm body write the `impl` keyword anywhere (the cheap test for "may emit an impl")?
fn writes_impl(ts: &TokenStream) -> bool {
    ts.clone().into_iter().any(|t| match t {
        TokenTree::Ident(i) => i == "impl",
        TokenTree::Group(g) => writes_impl(&g.stream()),
        _ => false,
    })
}

/// Every identifier a body DECLARES as an item (fn, type, const, static, trait, macro) or binds by a `use`, at any block
/// depth: a hoisted impl that names one would resolve it at module scope, which is not where rustc resolves it.
fn body_declared(it: &syn::Item) -> std::collections::HashSet<String> {
    struct D(std::collections::HashSet<String>, bool);
    impl<'ast> syn::visit::Visit<'ast> for D {
        fn visit_stmt(&mut self, st: &'ast syn::Stmt) {
            if let syn::Stmt::Item(i) = st {
                match i {
                    syn::Item::Use(u) => {
                        let mut v = Vec::new();
                        use_renames_all(&u.tree, &mut v);
                        self.0.extend(v);
                        self.1 = true;
                    }
                    other => {
                        if let Some(n) = crate::decls::declared_item_name(other, false) {
                            self.0.insert(n.to_string());
                        }
                    }
                }
            }
            syn::visit::visit_stmt(self, st);
        }
    }
    let mut d = D(Default::default(), false);
    syn::visit::Visit::visit_item(&mut d, it);
    if d.1 {
        d.0.insert("\u{1}use".to_string());
    }
    d.0
}

/// Does the host body declare a type (struct, enum, union, alias) named `leaf`, at any block depth?
fn body_declared_type(host: &syn::Item, leaf: &str) -> bool {
    struct T<'a>(&'a str, bool);
    impl<'ast> syn::visit::Visit<'ast> for T<'_> {
        fn visit_stmt(&mut self, st: &'ast syn::Stmt) {
            if let syn::Stmt::Item(i) = st {
                let n = match i {
                    syn::Item::Struct(x) => Some(&x.ident),
                    syn::Item::Enum(x) => Some(&x.ident),
                    syn::Item::Union(x) => Some(&x.ident),
                    syn::Item::Type(x) => Some(&x.ident),
                    _ => None,
                };
                if n.is_some_and(|n| n == self.0) {
                    self.1 = true;
                }
            }
            syn::visit::visit_stmt(self, st);
        }
    }
    let mut t = T(leaf, false);
    syn::visit::Visit::visit_item(&mut t, host);
    t.1
}

/// Every name a `use` tree binds (leaf or rename), and `*` for a glob.
fn use_renames_all(t: &syn::UseTree, out: &mut Vec<String>) {
    match t {
        syn::UseTree::Path(p) => use_renames_all(&p.tree, out),
        syn::UseTree::Name(n) => out.push(n.ident.to_string()),
        syn::UseTree::Rename(r) => out.push(r.rename.to_string()),
        syn::UseTree::Glob(_) => out.push("*".to_string()),
        syn::UseTree::Group(g) => g.items.iter().for_each(|x| use_renames_all(x, out)),
    }
}

impl Ctx<'_> {
    /// SOUNDNESS R1813 — THE IMPLS A BODY-POSITION INVOCATION EMITS, HOISTED BESIDE THE HOST ITEM.
    ///
    /// THE DEFECT. `fn _f() { mk_e!(WrapE, InnerX); }` where `mk_e!` expands to `impl Deref for WrapE { .. }`: the splice
    /// reads item, impl-item and trait-item positions only, so this impl was in no index — no `WrapE::deref` unit, `WrapE`
    /// published CLOSED with no `deref`, and a method of an inherent impl made this way (`TB.tb_go()`) had no unit at all.
    /// An `impl` is not scoped to the block that holds it (rustc collects it crate-wide), so hoisting it is the expansion
    /// rustc performs, read where Pass A can see it: a RESOLUTION. The invocation itself stays in the body, so the collector's
    /// own R1144 walk (and its containment charge on the host) is unchanged.
    ///
    /// REFUSED, AND DISCLOSED INSTEAD, where module scope is not where the impl's names resolve: its self type, or any name
    /// it mentions, is an item the host body declares or a `use` the host body writes. Then (and where a crate-local
    /// macro whose bodies write `impl` could not be expanded or parsed) a placeholder item-position invocation stands beside
    /// the host — R128's module evidence and `typesurf`'s unclosable mark, the cover main read through for a macro another
    /// macro defined (`m_binh`). A hand-written impl in a body is SOUNDNESS R1257's and is not touched (`in_expansion`).
    /// `CANDOR_I1S3_BODY_OFF=1` turns the hoist off.
    fn hoist_body_impls(&mut self, host: &syn::Item, depth: usize) -> Vec<syn::Item> {
        let has_body = match host {
            syn::Item::Fn(_) | syn::Item::Const(_) | syn::Item::Static(_) | syn::Item::Impl(_) => true,
            syn::Item::Trait(t) => t.items.iter().any(|ti| matches!(ti, syn::TraitItem::Fn(f) if f.default.is_some())),
            _ => false,
        };
        if !has_body || body_off() {
            return Vec::new();
        }
        let read = {
            let mut w = BodyWalk { cx: self, overlay: HashMap::new(), in_expansion: false, depth: 0, out: BodyRead::default() };
            syn::visit::Visit::visit_item(&mut w, host);
            w.out
        };
        if read.impls.is_empty() && !read.unread {
            return Vec::new();
        }
        let host_attrs = crate::lang::item_attrs(host);
        let mut carry: Vec<syn::Attribute> =
            host_attrs.iter().filter(|a| a.path().is_ident("cfg") || a.path().is_ident("cfg_attr")).cloned().collect();
        if host_attrs.iter().any(|a| a.path().is_ident("test")) {
            carry.push(syn::parse_quote!(#[cfg(test)]));
        }
        // THE REFUSAL'S SUPERSET ARGUMENT. Hoisting moves an impl from block scope to module scope, and only a NAME can resolve
        // differently between the two: a name the block declares (an item, a `macro_rules!`) or binds (a `use`, which may be a
        // glob and so bind any name). The block is the host body AND every expansion read in it, since an expansion's items are
        // the block's items (`struct LS2; impl LS2 { .. }` from one invocation: `flocaltwin2`). So: any `use` in either refuses
        // every impl; otherwise an impl naming no declared identifier resolves every name as module scope does. Fable stage-3
        // review D4 measured the hole when the expansion's half was missing (`ls2_call_q` charged over a pure body).
        let mut declared = body_declared(host);
        let local_decl = read.declared.clone();
        declared.extend(read.declared);
        let mut out = Vec::new();
        let mut refused = read.unread;
        for mut im in read.impls {
            let mut names = std::collections::HashSet::new();
            all_idents(&quote::ToTokens::to_token_stream(&im), &mut names);
            if declared.contains("\u{1}use") || names.iter().any(|n| declared.contains(n)) {
                if candor_report::env::trace("CANDOR_I1_INSTR") {
                    eprintln!("I1S3BODYREFUSE\t{}", ty_key(&im.self_ty)); // §E1 REACH COUNTER
                }
                // An INHERENT impl of a type the block itself declares has no caller outside the block (the type is not
                // nameable there), so refusing it hides nothing a caller elsewhere could reach: no disclosure is owed. A trait
                // impl of such a type still is one (dispatch through a returned value), so it discloses.
                let self_leaf = ty_key(&im.self_ty).rsplit("::").next().unwrap_or_default().to_string();
                let block_local = local_decl.contains(&self_leaf) || body_declared_type(host, &self_leaf);
                if !(block_local && im.trait_.is_none() && !declared.contains("\u{1}use")) {
                    refused = true;
                }
                continue;
            }
            // D5 — a hoisted impl whose member is an impl-item invocation (rand 0.10's `impl_fill!`) is read as a hand-written
            // impl's is (`splice_impl_items`, R1245), so the member is a unit and not an opaque block.
            self.splice_impl_items(&mut im.items, depth + 1);
            if candor_report::env::trace("CANDOR_I1_INSTR") {
                eprintln!("I1S3BODYHOIST\t{}", ty_key(&im.self_ty)); // §E1 REACH COUNTER, on the CHANGED branch only
            }
            if prim_operator_impl(&im) {
                let id = syn::Ident::new(crate::model::MBE_PRIM_ATTR, proc_macro2::Span::call_site());
                im.attrs.push(syn::parse_quote!(#[#id]));
            } else if operator_trait_impl(&im) {
                let id = syn::Ident::new(crate::model::MBE_OP_ATTR, proc_macro2::Span::call_site());
                im.attrs.push(syn::parse_quote!(#[#id]));
            }
            peel_groups(&mut im.self_ty);
            if let Some((_, tp, _)) = &mut im.trait_ {
                for seg in tp.segments.iter_mut() {
                    if let syn::PathArguments::AngleBracketed(ab) = &mut seg.arguments {
                        for a in ab.args.iter_mut() {
                            if let syn::GenericArgument::Type(t) = a {
                                peel_groups(t);
                            }
                        }
                    }
                }
            }
            let mut na = carry.clone();
            na.append(&mut im.attrs);
            im.attrs = na;
            out.push(syn::Item::Impl(im));
        }
        if refused {
            if candor_report::env::trace("CANDOR_I1_INSTR") {
                eprintln!("I1S3BODYHIDDEN"); // §E1 REACH COUNTER
            }
            let mut ph: syn::ItemMacro = syn::parse_quote!(__candor_i1_body_hidden! {});
            ph.attrs = carry;
            out.push(syn::Item::Macro(ph));
        }
        out
    }
}

/// I1 stage 3 (D1, D1′) — does a trait anywhere in these items (through inline modules, at any depth) still hold an
/// unexpanded trait-item invocation? Such an expansion is PARTIAL: its text stays R452 evidence for the members it hides.
fn holds_unexpanded_trait_member(items: &[syn::Item]) -> bool {
    items.iter().any(|it| match it {
        syn::Item::Trait(t) => t.items.iter().any(|ti| matches!(ti, syn::TraitItem::Macro(_))),
        syn::Item::Mod(m) => m.content.as_ref().is_some_and(|(_, inner)| holds_unexpanded_trait_member(inner)),
        _ => false,
    })
}

/// I1 stage 3 (D1) — the opt-out of trait-item splicing (`CANDOR_I1S3_TRAITITEM_OFF`); it implies `mdef_off` (see there).
pub(crate) fn trait_item_off() -> bool {
    static OFF: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *OFF.get_or_init(|| candor_report::env::switch("CANDOR_I1S3_TRAITITEM_OFF"))
}

fn trait_item_attrs(ti: &syn::TraitItem) -> &[syn::Attribute] {
    match ti {
        syn::TraitItem::Fn(x) => &x.attrs,
        syn::TraitItem::Const(x) => &x.attrs,
        syn::TraitItem::Type(x) => &x.attrs,
        syn::TraitItem::Macro(x) => &x.attrs,
        _ => &[],
    }
}

fn trait_item_attrs_mut(ti: &mut syn::TraitItem) -> Option<&mut Vec<syn::Attribute>> {
    match ti {
        syn::TraitItem::Fn(x) => Some(&mut x.attrs),
        syn::TraitItem::Const(x) => Some(&mut x.attrs),
        syn::TraitItem::Type(x) => Some(&mut x.attrs),
        syn::TraitItem::Macro(x) => Some(&mut x.attrs),
        _ => None,
    }
}

pub(crate) fn body_off() -> bool {
    static OFF: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *OFF.get_or_init(|| candor_report::env::switch("CANDOR_I1S3_BODY_OFF"))
}

/// I1 stage 2 — the item kinds stage 2 started splicing, named for the reach counter.
fn stage2_kind(g: &syn::Item) -> Option<&'static str> {
    match g {
        syn::Item::Mod(_) => Some("mod"),
        syn::Item::Use(_) => Some("use"),
        syn::Item::ExternCrate(_) => Some("extern-crate"),
        _ => None,
    }
}

fn invocation_key(mac: &syn::Macro) -> String {
    let p: Vec<String> = mac.path.segments.iter().map(|s| s.ident.to_string()).collect();
    format!("{}!{}", p.join("::"), mac.tokens)
}

/// Is this unexpanded item-position invocation text that may hold a cfg arm? Every invocation the splice left as written,
/// except a PARTIAL one (its items stand spliced; what it did not splice is kept as its own item) and the toolchain's own
/// item macros, whose expansions are fixed and bind nothing of the crate's (`thread_local!`, `compile_error!`, …;
/// `include!` is not exempt: an unread include is hidden text).
fn is_hidden_text(m: &syn::ItemMacro, partial: &std::collections::HashSet<String>, tc_shadow: &std::collections::HashSet<String>) -> bool {
    if m.ident.is_some() || m.mac.path.is_ident("macro_rules") || partial.contains(&invocation_key(&m.mac)) {
        return false;
    }
    let leaf = m.mac.path.segments.last().map(|s| s.ident.to_string()).unwrap_or_default();
    // An `include!` R145 left in place is either UNREADABLE (it carries the opaque-include mark: hidden text) or a file the
    // walk reads as its own (syn's `include!("await.rs")`): its text is read, so it hides nothing here.
    if leaf == "include" {
        return m.attrs.iter().any(|a| a.path().is_ident(crate::lang::OPAQUE_INCLUDE_ATTR));
    }
    let root = m.mac.path.segments.first().map(|s| s.ident.to_string()).unwrap_or_default();
    let toolchain_path = m.mac.path.segments.len() == 1 || matches!(root.as_str(), "std" | "core" | "alloc");
    // SOUNDNESS R1899 — the exclusion is for the TOOLCHAIN's binding, not for the name: a bare `thread_local!` this file
    // imports from a dependency (`use dep::thread_local;`), or defines itself, is that macro's text.
    let bare_shadowed = m.mac.path.segments.len() == 1 && m.mac.path.leading_colon.is_none() && tc_shadow.contains(&leaf);
    !(toolchain_path && TOOLCHAIN_BANG_MACROS.contains(&leaf.as_str())) || bare_shadowed
}

pub(crate) fn is_gated(a: &[syn::Attribute]) -> bool {
    a.iter().any(|x| x.path().is_ident("cfg") || x.path().is_ident("cfg_attr"))
}

/// A `use` put back as unexpanded text: the state every pre-stage-2 build read for it, so R128 discloses the module.
fn use_as_hidden_text(u: &syn::ItemUse) -> syn::Item {
    syn::Item::Macro(syn::ItemMacro {
        attrs: Vec::new(),
        ident: None,
        mac: syn::Macro {
            path: syn::parse_quote!(__candor_i1_hidden_twin_use),
            bang_token: Default::default(),
            delimiter: syn::MacroDelimiter::Brace(Default::default()),
            tokens: quote::ToTokens::to_token_stream(u),
        },
        semi_token: None,
    })
}

fn mark_fn(sp: Option<proc_macro2::Span>, attrs: &mut Vec<syn::Attribute>, n: &str) {
    mark_fn_at(sp, attrs, n, "I1S2MARKFN");
}

/// The span of an item's first token: where `decls::fn_locs` reads its location from.
fn first_span(t: &impl quote::ToTokens) -> Option<proc_macro2::Span> {
    t.to_token_stream().into_iter().next().map(|t| t.span())
}

/// `mark_fn_as`, with the marker carrying the item's first-token span (cfg-twin lane A). A marker pushed onto an item with
/// NO attributes becomes its first token, so a `call_site` span moved the unit's location to `:1:1` (measured: getrandom
/// 0.2.17's out-of-line `#[path]` arms, 35 rows in this lane's first A/B).
fn mark_fn_at(sp: Option<proc_macro2::Span>, attrs: &mut Vec<syn::Attribute>, n: &str, counter: &str) {
    if !attrs.iter().any(|a| a.path().is_ident(crate::model::MBE_TWIN_ATTR)) {
        let sp = if attrs.is_empty() { sp.unwrap_or_else(proc_macro2::Span::call_site) } else { proc_macro2::Span::call_site() };
        let id = syn::Ident::new(crate::model::MBE_TWIN_ATTR, sp);
        attrs.push(syn::parse_quote_spanned!(sp=> #[#id]));
        if candor_report::env::trace("CANDOR_I1_INSTR") {
            eprintln!("{counter}\t{n}"); // §E1 REACH COUNTER, on the ADDED branch only
        }
    }
}

/// `counter`: the §E1 reach counter the mark is counted under (fn, method, trait default).
fn mark_fn_as(attrs: &mut Vec<syn::Attribute>, n: &str, counter: &str) {
    if !attrs.iter().any(|a| a.path().is_ident(crate::model::MBE_TWIN_ATTR)) {
        let id = syn::Ident::new(crate::model::MBE_TWIN_ATTR, proc_macro2::Span::call_site());
        attrs.push(syn::parse_quote!(#[#id]));
        if candor_report::env::trace("CANDOR_I1_INSTR") {
            eprintln!("{counter}\t{n}"); // §E1 REACH COUNTER, on the ADDED branch only
        }
    }
}

/// One spliced cfg arm, made to disclose: a fn or method is marked, a `use` (named, renamed, grouped or GLOB) is put back as
/// hidden text, and a module arm is treated the same way at every depth of its body.
fn cover_arm(it: &mut syn::Item) {
    match it {
        // I1 stage 3 (R1784) — a produced `macro_rules!` arm is kept and MARKED: Pass A records its name as a cfg twin
        // (`decls`, R208), so a body-position invocation discloses `ambiguous:` instead of reading this arm alone.
        syn::Item::Macro(m) if m.ident.is_some() => {
            if candor_report::env::trace("CANDOR_I1_INSTR") {
                eprintln!("I1S3MARKMDEF\t{}", m.ident.as_ref().map(|i| i.to_string()).unwrap_or_default()); // §E1 REACH COUNTER
            }
            let id = syn::Ident::new(crate::model::MBE_TWIN_DEF_ATTR, proc_macro2::Span::call_site());
            m.attrs.push(syn::parse_quote!(#[#id]));
        }
        syn::Item::Fn(f) => {
            let n = f.sig.ident.to_string();
            mark_fn(first_span(&*f), &mut f.attrs, &n);
        }
        syn::Item::Impl(im) => {
            for ii in im.items.iter_mut() {
                if let syn::ImplItem::Fn(f) = ii {
                    let n = f.sig.ident.to_string();
                    mark_fn(first_span(&*f), &mut f.attrs, &n);
                }
            }
        }
        syn::Item::Use(u) => {
            if candor_report::env::trace("CANDOR_I1_INSTR") {
                eprintln!("I1S2MARKUSE\t{}", quote::ToTokens::to_token_stream(&u.tree)); // §E1 REACH COUNTER, ADDED branch only
            }
            *it = use_as_hidden_text(u);
        }
        // I1 stage 2b — a `type` alias arm is put back as unexpanded text, as a `use` arm is: a call through it then resolves
        // to nothing and R128 discloses the module (Fable RC3-7 `aliasty_e`: the alias resolved to a pure ungated impl).
        syn::Item::Type(t) => {
            if candor_report::env::trace("CANDOR_I1_INSTR") {
                eprintln!("I1S2MARKTYPE\t{}", t.ident); // §E1 REACH COUNTER
            }
            *it = syn::Item::Macro(syn::ItemMacro {
                attrs: Vec::new(),
                ident: None,
                mac: syn::Macro {
                    path: syn::parse_quote!(__candor_i1_hidden_twin_use),
                    bang_token: Default::default(),
                    delimiter: syn::MacroDelimiter::Brace(Default::default()),
                    tokens: quote::ToTokens::to_token_stream(&*t),
                },
                semi_token: None,
            });
        }
        syn::Item::Mod(m) => {
            if candor_report::env::trace("CANDOR_I1_INSTR") {
                eprintln!("I1S2MARKMOD\t{}", m.ident); // §E1 REACH COUNTER, on the ADDED branch only
            }
            if let Some((_, inner)) = &mut m.content {
                inner.iter_mut().for_each(cover_arm);
            }
        }
        // SOUNDNESS R1857 — a trait in a module arm: every default body is a unit the arm defines (`p6uptr`: the visible arm's
        // pure `ExtG::go` default answered a call whose live arm was the hidden text beside it). Both spellings: a trait the
        // module writes, and the marker an expansion inside it left for a generated trait.
        syn::Item::Trait(t) if !r1857_off() => {
            let tn = t.ident.to_string();
            for ti in t.items.iter_mut() {
                if let syn::TraitItem::Fn(f) = ti {
                    if f.default.is_some() {
                        let n = format!("{tn}::{}", f.sig.ident);
                        mark_fn_at(first_span(&*f), &mut f.attrs, &n, "R1857MARKTRAIT");
                    }
                }
            }
        }
        syn::Item::Verbatim(_) if !r1857_off() && mbe_trait_body_of(it).is_some() => {
            cover_trait_marker(it, &Hidden { opaque: true, ..Hidden::default() });
        }
        _ => {}
    }
}

/// cfg-twin lane A — `cover_arm` for an arm the splice did NOT produce (a hand-written module arm, or a file read as an
/// out-of-line arm). The same marks, with one difference: a `use` or a `type` alias is LEFT AS WRITTEN. `cover_arm` puts a
/// produced one back as hidden text, which is the state every pre-stage-2 build read for it; for a hand-written binding no
/// build read it that way, and replacing it would remove the edge through it to a possibly effectful target — a removal,
/// which this additive cover may not make. So those two positions stay a named residual on a hand-written arm (counted
/// under `CFGTWINRESIDUAL`), while every unit, trait default and `macro_rules!` definition is marked.
fn cover_any(it: &mut syn::Item, hand: bool) {
    if !hand {
        cover_arm(it);
        return;
    }
    match it {
        syn::Item::Use(_) | syn::Item::Type(_) => {
            if candor_report::env::trace("CANDOR_I1_INSTR") {
                eprintln!("CFGTWINRESIDUAL\t{}", if matches!(it, syn::Item::Use(_)) { "use" } else { "type" }); // §E1 REACH COUNTER
            }
        }
        // a hand-written `macro_rules!` arm keeps expanding (`MBE_TWIN_HAND_DEF_ATTR`: R208's disclosure only)
        syn::Item::Macro(m) if m.ident.is_some() => {
            if !m.attrs.iter().any(|a| a.path().is_ident(crate::model::MBE_TWIN_HAND_DEF_ATTR)) {
                let id = syn::Ident::new(crate::model::MBE_TWIN_HAND_DEF_ATTR, proc_macro2::Span::call_site());
                m.attrs.push(syn::parse_quote!(#[#id]));
            }
        }
        syn::Item::Mod(m) => {
            if candor_report::env::trace("CANDOR_I1_INSTR") {
                eprintln!("I1S2MARKMOD\t{}", m.ident); // §E1 REACH COUNTER, on the ADDED branch only
            }
            if let Some((_, inner)) = &mut m.content {
                inner.iter_mut().for_each(|x| cover_any(x, true));
            }
        }
        _ => cover_arm(it),
    }
}

/// SOUNDNESS R1857 — does this module body hold unexpanded text at any depth: an item-position invocation (`is_hidden_text`),
/// or an impl-item or trait-item invocation left in an impl or trait.
fn holds_hidden(items: &[syn::Item], partial: &std::collections::HashSet<String>, tc: &std::collections::HashSet<String>) -> bool {
    items.iter().any(|it| match it {
        syn::Item::Macro(m) => is_hidden_text(m, partial, tc),
        syn::Item::Impl(im) => im.items.iter().any(|ii| matches!(ii, syn::ImplItem::Macro(_))),
        syn::Item::Trait(t) => t.items.iter().any(|ti| matches!(ti, syn::TraitItem::Macro(_))),
        syn::Item::Mod(m) => m.content.as_ref().is_some_and(|(_, inner)| holds_hidden(inner, partial, tc)),
        _ => false,
    })
}

/// SOUNDNESS R1857 — the visible side of a module arm, against what the OTHER arm's hidden text may define (`h`, read by
/// `Ctx::read_arm_hidden` with the same `item_keys`/`read_hidden` the per-module cover uses). Keys are read unprefixed through
/// every depth of the arm, so a name the hidden text defines in a nested module covers that name at every depth here: an
/// over-approximation in the hedge direction. An opaque reading covers the whole arm (`cover_arm`).
fn cover_arm_keyed(items: &mut [syn::Item], h: &Hidden, hand: bool) {
    let type_twins: std::collections::HashSet<String> = items
        .iter()
        .filter_map(|it| match it {
            syn::Item::Struct(x) => Some(x.ident.to_string()),
            syn::Item::Enum(x) => Some(x.ident.to_string()),
            syn::Item::Union(x) => Some(x.ident.to_string()),
            syn::Item::Type(x) => Some(x.ident.to_string()),
            _ => None,
        })
        .collect();
    let covered_names: Vec<bool> = items
        .iter()
        .map(|it| {
            let k = item_keys(it, items);
            k.opaque || k.names.iter().any(|n| h.covers(n))
        })
        .collect();
    for (i, it) in items.iter_mut().enumerate() {
        if h.opaque {
            cover_any(it, hand);
            continue;
        }
        match it {
            syn::Item::Fn(f) => {
                let n = f.sig.ident.to_string();
                if h.covers(&n) {
                    mark_fn_at(first_span(&*f), &mut f.attrs, &n, "R1857MARKFN");
                }
            }
            syn::Item::Use(_) | syn::Item::Type(_) => {
                if covered_names[i] {
                    cover_any(it, hand);
                }
            }
            syn::Item::Macro(m) if m.ident.is_some() => {
                if covered_names[i] {
                    cover_any(it, hand);
                }
            }
            syn::Item::Mod(m) => {
                if h.covers(&m.ident.to_string()) {
                    cover_any(it, hand);
                } else if let Some((_, inner)) = &mut m.content {
                    cover_arm_keyed(inner, h, hand);
                }
            }
            syn::Item::Impl(im) => {
                let owner = ty_key(&im.self_ty);
                let tr = trait_key(im);
                let tt = type_twins.contains(&owner);
                for ii in im.items.iter_mut() {
                    if let syn::ImplItem::Fn(f) = ii {
                        let n = f.sig.ident.to_string();
                        let trait_leg = tr.as_ref().is_some_and(|t| {
                            h.keys.contains(t)
                                || h.keys.contains(&format!("{t}::{n}"))
                                || h.xkeys.contains(&format!("{owner} as {t}"))
                                || h.xkeys.contains(&format!("* as {t}"))
                        });
                        if h.covers_method(&owner, &n, tt) || trait_leg {
                            mark_fn_at(first_span(&*f), &mut f.attrs, &format!("{owner}::{n}"), "R1857MARKMETH");
                        }
                    }
                }
            }
            syn::Item::Trait(t) => {
                let tn = t.ident.to_string();
                for ti in t.items.iter_mut() {
                    if let syn::TraitItem::Fn(f) = ti {
                        let n = f.sig.ident.to_string();
                        if f.default.is_some() && trait_default_twin(h, &tn, &n) {
                            mark_fn_at(first_span(&*f), &mut f.attrs, &format!("{tn}::{n}"), "R1857MARKTRAIT");
                        }
                    }
                }
            }
            syn::Item::Verbatim(_) if mbe_trait_body_of(it).is_some() => cover_trait_marker_by(it, |tn, n| trait_default_twin(h, tn, n)),
            _ => {}
        }
    }
}

/// SOUNDNESS R1857 — the opt-out of the module-twin cover (`Ctx::mark_module_twins`) and of `cover_arm`'s trait arm.
pub(crate) fn r1857_off() -> bool {
    static OFF: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *OFF.get_or_init(|| candor_report::env::switch("CANDOR_R1857_OFF"))
}

/// SOUNDNESS R1541 / I1 stage 2 — A CFG ARM THE SPLICE PRODUCED, BESIDE TEXT IT DID NOT EXPAND.
///
/// THE EXPOSURE. Each widening of the splice puts one more kind of cfg arm in the program: stage 1 macro-made trait impls and
/// declarations, stage 2a modules and `use`s (and, by default since the stage 2b flip, another file's cfg-stamping macro). If the
/// arm's twin stays UNEXPANDED (its wrapper is defined twice, the budget ran out, no arm matches, the name is imported, it is
/// another crate's macro…), the name RESOLVES to the produced arm and reads pure over a program that writes. Before the splice
/// reached that kind, the name resolved to nothing and R128 disclosed `macro:`; the widening removed that cover.
///
/// THE MODEL (stage 2a): keyed on the NAMES the module's hidden text may define, read by ONE reader. Each unexpanded
/// invocation left in a module is expanded with every body its macro has and parsed by `syn` (`Ctx::read_hidden`), and its
/// items are keyed by `item_keys`, the function that keys the visible arm; where expansion cannot finish the text falls to a
/// fallback (`loose`, below). A cfg-gated item on the visible side is marked (`cover_arm`, `mark_fn_as`) when its key is one the
/// hidden text may define: a PRODUCED fn, `use` or module; ANY gated method (its `Owner::m` or its owner type, where the visible
/// arm declares that type as a produced cfg arm); a produced trait's default bodies; and a gated TRAIT impl whose trait, or an
/// impl of it for the same type, is in the hidden text. An ungated item cannot have a compiled twin in its module (a second
/// definition is a rustc error).
///
/// SCOPE (stage 2c). This reader is per MODULE, which is the right unit for a path item (a fn, `use`, module or type: its cfg
/// twin sits at the same path) and the wrong one for an impl or a trait. So each module also EXPORTS its impl-scoped keys
/// (`Hidden::xkeys`, read through nested hidden modules, with blanket impls under `* as Tr` and the renames the hidden text
/// writes), and pass B (`scan.rs`) checks every gated impl member, including one inside a cfg-gated module, against the
/// crate-wide union. WHAT IT DOES NOT SEE, stated: the fallback arms are not shown to be supersets of what `item_keys` would
/// have read (RC3.1; under the opt-in lift RC3-1..4 are open), and crate-wide matching reads `keys` only, not `loose`; an
/// out-of-line module arm (`#[path] mod imp;`) is a module whose body the cover cannot mark (it holds today through R128 on
/// the unresolved call, Fable merge review 2c `tpath_e`). With the cross-file lift off (the default) the cover acts on
/// same-file splices only. `CANDOR_I1S2_TWIN_OFF=1` drops the cover.
///
/// An impl block that still holds an unexpanded impl-item invocation is the same at member level: its cfg-gated methods are
/// marked.
/// What one module's hidden text may define (`Ctx::read_hidden`): `keys` read by `syn` from its expansions (`item_keys`), and
/// from the items another crate's macro is passed; `loose`, the fallback where expansion could not finish — every identifier
/// of the invocation and of every body reachable from it (`loose_closure`); or `opaque` (another crate's macro, a `$m!`, a
/// token-paster, an unreadable body, or the closure's bound). Each fallback arm states its superset argument where it is.
#[derive(Default, Clone)]
struct Hidden {
    opaque: bool,
    keys: std::collections::HashSet<String>,
    loose: std::collections::HashSet<String>,
    steps: usize,
    /// the signature of each hidden trait-impl member, keyed `Owner as Trait::m` (see `read_hidden_item`)
    sigs: std::collections::HashMap<String, syn::Signature>,
    /// I1 stage 2c — the IMPL-SCOPED keys: `Tr`, `Tr::m` (a trait and its items), `S as Tr`, `S as Tr::m` (a trait impl),
    /// `S::m` (any impl member). Unlike a path item (a fn, `use`, module or type, whose cfg twin must sit at the same path, so
    /// in this module), an impl's twin can sit in ANY module of the crate, and a trait's default reaches every impl of it
    /// wherever that impl is written: these keys are read through nested hidden modules unprefixed and exported crate-wide
    /// (`XKEYS_MARK`, `cache::FileDecls::i1_xkeys`), where pass B checks every gated impl member against them.
    xkeys: std::collections::HashSet<String>,
}

/// Expansions one hidden invocation's reading may attempt before it falls to `loose`.
const HIDDEN_STEP_BUDGET: usize = 2048;
/// How deep the reading follows nested invocations. Deeper than the splice's `MAX_DEPTH`: a recursive tower past that limit
/// (syn's `generate_to_tokens!`, one level per enum variant) is often WHY the text stayed hidden, and reading it to the end
/// is what keeps its names precise instead of `loose`.
const HIDDEN_DEPTH: usize = 128;

impl Hidden {
    fn note(&self, how: &str, name: &str) {
        if candor_report::env::trace("CANDOR_I1_INSTR") {
            eprintln!("I1S2HIDREAD\t{how}\t{name}"); // §E1 REACH COUNTER: how hidden text was read (opaque/foreign/limit/noexpand)
        }
    }
    fn covers(&self, name: &str) -> bool {
        self.opaque || self.keys.contains(name) || self.loose.contains(name)
    }
    /// A method `Owner::m`: its key, or the loose method name; or — only where the VISIBLE arm itself declares the type as a
    /// produced cfg arm (`type_twin`) — a hidden type of that name, whose methods are then unknown (`std::fs::*` beside a stub
    /// `struct File`). A hidden type with no visible twin (pin_project's struct) is that type's only declaration, not an arm.
    fn covers_method(&self, owner: &str, m: &str, type_twin: bool) -> bool {
        self.opaque
            || self.keys.contains(&format!("{owner}::{m}"))
            || self.xkeys.contains(&format!("{owner}::{m}"))
            || self.loose.contains(m)
            || (type_twin && (self.keys.contains(owner) || self.loose.contains(owner)))
    }
    fn is_empty(&self) -> bool {
        !self.opaque && self.keys.is_empty() && self.loose.is_empty() && self.xkeys.is_empty()
    }
    /// Fold another reading in (the module-level union of every hidden invocation's reading).
    fn absorb(&mut self, h: Hidden) {
        self.opaque |= h.opaque;
        self.keys.extend(h.keys);
        self.loose.extend(h.loose);
        self.xkeys.extend(h.xkeys);
        for (k, v) in h.sigs {
            self.sigs.entry(k).or_insert(v);
        }
    }
}

/// The names a macro BODY declares as written (the identifier after `fn`/`mod`/`struct`/`enum`/`union`/`type`/`trait`/
/// `const`/`static`, or a `macro_rules!` name) — what a body the expander could not run may define besides the identifiers its
/// invocation passes (`all_idents`): the `loose` over-approximation's second half.
fn declared_names(ts: &TokenStream, out: &mut std::collections::HashSet<String>) {
    let toks: Vec<TokenTree> = ts.clone().into_iter().collect();
    for (i, tt) in toks.iter().enumerate() {
        match tt {
            TokenTree::Group(g) => declared_names(&g.stream(), out),
            TokenTree::Ident(id) => {
                let n = id.to_string();
                let kw = matches!(n.as_str(), "fn" | "mod" | "struct" | "enum" | "union" | "type" | "trait" | "const" | "static");
                let mr = n == "macro_rules" && matches!(toks.get(i + 1), Some(TokenTree::Punct(p)) if p.as_char() == '!');
                let at = if mr { i + 2 } else { i + 1 };
                if kw || mr {
                    if let Some(TokenTree::Ident(nm)) = toks.get(at) {
                        out.insert(nm.to_string().trim_start_matches("r#").to_string());
                    }
                }
            }
            _ => {}
        }
    }
}

fn all_idents(ts: &TokenStream, out: &mut std::collections::HashSet<String>) {
    for tt in ts.clone() {
        match tt {
            TokenTree::Group(g) => all_idents(&g.stream(), out),
            TokenTree::Ident(id) => {
                out.insert(id.to_string().trim_start_matches("r#").to_string());
            }
            _ => {}
        }
    }
}

/// An impl's self type as both readers key it: the last path segment, or the type's text for a non-path type.
pub(crate) fn ty_key(t: &syn::Type) -> String {
    match t {
        syn::Type::Path(p) => p.path.segments.last().map(|s| s.ident.to_string()).unwrap_or_default(),
        syn::Type::Group(g) => ty_key(&g.elem),
        syn::Type::Paren(p) => ty_key(&p.elem),
        other => quote::ToTokens::to_token_stream(other).to_string(),
    }
}

/// I1 stage 2d — an impl whose self type is a NON-PATH type built over one of its own type parameters (`&T`, `&mut T`,
/// `*const T`, `(T, U)`, `[T]`, `[T; N]`): a blanket over every type of that SHAPE, keyed `{shape} as Tr` (`self_shape`), which
/// every candidate whose self type has the shape asks for (Fable re-check 2c-2 RC4-1 `tgref_e`: `ty_key(&T)` read `& T`, which
/// no candidate asked for). A PATH self type over a parameter (`Box<T>`, `Vec<T>`) is keyed by its last segment, which the
/// visible arm's `Box<S>` shares already.
fn structural_blanket(im: &syn::ItemImpl) -> bool {
    if candor_report::env::switch("CANDOR_I1S2D_REFBLANKET_OFF") || matches!(&*im.self_ty, syn::Type::Path(_)) {
        return false;
    }
    let params: std::collections::HashSet<String> = im.generics.type_params().map(|p| p.ident.to_string()).collect();
    let mut ids = std::collections::HashSet::new();
    all_idents(&quote::ToTokens::to_token_stream(&*im.self_ty), &mut ids);
    let hit = ids.iter().any(|i| params.contains(i));
    if hit && candor_report::env::trace("CANDOR_I1_INSTR") {
        eprintln!("I1S2DBLANKET\t{}", ty_key(&im.self_ty)); // §E1 REACH COUNTER
    }
    hit
}

/// I1 stage 2d — the outer constructor of a non-path self type, read from its `ty_key` text: `&`, `&mut`, `*const`, `*mut`,
/// `()` (a tuple), `[;]` (an array), `[]` (a slice). `None` for a path type.
pub(crate) fn self_shape(k: &str) -> Option<String> {
    let t = k.trim_start();
    let sh = if t.starts_with("& mut") || t.starts_with("&mut") {
        "&mut"
    } else if t.starts_with('&') {
        "&"
    } else if t.starts_with("* const") || t.starts_with("*const") {
        "*const"
    } else if t.starts_with("* mut") || t.starts_with("*mut") {
        "*mut"
    } else if t.starts_with('(') {
        "()"
    } else if t.starts_with('[') {
        let mut depth = 0i32;
        let mut arr = false;
        for c in t.chars() {
            match c {
                '[' | '(' | '<' | '{' => depth += 1,
                ']' | ')' | '>' | '}' => depth -= 1,
                ';' if depth == 1 => arr = true,
                _ => {}
            }
        }
        if arr { "[;]" } else { "[]" }
    } else {
        return None;
    };
    Some(sh.to_string())
}

/// The trait an impl implements, as both readers key it: its last path segment.
fn trait_key(im: &syn::ItemImpl) -> Option<String> {
    im.trait_.as_ref().and_then(|(_, p, _)| p.segments.last()).map(|s| s.ident.to_string())
}

/// The names an item binds in its module, read by `syn` — THE reader of both arms.
struct ItemKeys {
    names: Vec<String>,
    /// a glob whose names cannot be enumerated here
    opaque: bool,
}

/// What a glob `use <path>::*` binds: the item names of an inline module of this list it names (`self::m::*`, `m::*`), the
/// curated `std::fs` / `std::env` name tables, or — anything else — names this pass cannot list.
fn glob_names(path: &[String], out: &[syn::Item]) -> Option<Vec<String>> {
    let p: Vec<&str> = path.iter().map(String::as_str).filter(|s| *s != "self").collect();
    match p.as_slice() {
        ["std" | "core", "fs"] => return Some(crate::lang::STD_FS_NAMES.iter().map(|s| s.to_string()).collect()),
        ["std", "env"] => return Some(crate::lang::STD_ENV_NAMES.iter().map(|s| s.to_string()).collect()),
        [m] => {
            for it in out {
                if let syn::Item::Mod(md) = it {
                    if md.ident == m {
                        if let Some((_, inner)) = &md.content {
                            let mut v = Vec::new();
                            for x in inner {
                                let k = item_keys(x, inner);
                                if k.opaque {
                                    return None;
                                }
                                v.extend(k.names);
                            }
                            return Some(v);
                        }
                    }
                }
            }
        }
        _ => {}
    }
    None
}

fn item_keys(it: &syn::Item, out: &[syn::Item]) -> ItemKeys {
    fn use_keys(t: &syn::UseTree, prefix: &mut Vec<String>, out: &[syn::Item], k: &mut ItemKeys) {
        match t {
            syn::UseTree::Path(p) => {
                prefix.push(p.ident.to_string());
                use_keys(&p.tree, prefix, out, k);
                prefix.pop();
            }
            syn::UseTree::Name(n) if n.ident == "self" => {
                if let Some(last) = prefix.last() {
                    k.names.push(last.clone());
                }
            }
            syn::UseTree::Name(n) => k.names.push(n.ident.to_string()),
            syn::UseTree::Rename(r) => k.names.push(r.rename.to_string()),
            syn::UseTree::Glob(_) => match glob_names(prefix, out) {
                Some(v) => k.names.extend(v),
                None => k.opaque = true,
            },
            syn::UseTree::Group(g) => g.items.iter().for_each(|i| use_keys(i, prefix, out, k)),
        }
    }
    let mut k = ItemKeys { names: Vec::new(), opaque: false };
    let decoded;
    let it = match mbe_trait_body_of(it) {
        Some(t) => {
            decoded = syn::Item::Trait(t);
            &decoded
        }
        None => it,
    };
    match it {
        syn::Item::Fn(f) => k.names.push(f.sig.ident.to_string()),
        syn::Item::Use(u) => use_keys(&u.tree, &mut Vec::new(), out, &mut k),
        syn::Item::Mod(m) => k.names.push(m.ident.to_string()),
        syn::Item::Impl(im) => {
            let owner = ty_key(&im.self_ty);
            for ii in &im.items {
                if let syn::ImplItem::Fn(f) = ii {
                    k.names.push(format!("{owner}::{}", f.sig.ident));
                }
            }
        }
        syn::Item::Trait(t) => {
            k.names.push(t.ident.to_string());
            for ti in &t.items {
                if let syn::TraitItem::Fn(f) = ti {
                    k.names.push(format!("{}::{}", t.ident, f.sig.ident));
                }
            }
        }
        syn::Item::Struct(x) => k.names.push(x.ident.to_string()),
        syn::Item::Enum(x) => k.names.push(x.ident.to_string()),
        syn::Item::Union(x) => k.names.push(x.ident.to_string()),
        syn::Item::Type(x) => k.names.push(x.ident.to_string()),
        syn::Item::Const(x) => k.names.push(x.ident.to_string()),
        syn::Item::Static(x) => k.names.push(x.ident.to_string()),
        syn::Item::TraitAlias(x) => k.names.push(x.ident.to_string()),
        syn::Item::Macro(m) => {
            if let Some(i) = &m.ident {
                k.names.push(i.to_string());
            }
        }
        syn::Item::ExternCrate(e) => k.names.push(e.rename.as_ref().map(|r| r.1.to_string()).unwrap_or_else(|| e.ident.to_string())),
        syn::Item::ForeignMod(fm) => {
            for fi in &fm.items {
                match fi {
                    syn::ForeignItem::Fn(f) => k.names.push(f.sig.ident.to_string()),
                    syn::ForeignItem::Static(s) => k.names.push(s.ident.to_string()),
                    syn::ForeignItem::Type(t) => k.names.push(t.ident.to_string()),
                    _ => {}
                }
            }
        }
        _ => {}
    }
    k
}

/// Does every path this `use` binds lead into one of `mods` (modules of this list the cover marks), so the names keep
/// resolving and reach marked units? Then the `use` stays: putting it back would only lose the resolution.
fn use_into(t: &syn::UseTree, mods: &std::collections::HashSet<String>, first: bool) -> bool {
    match t {
        syn::UseTree::Path(p) if first && p.ident == "self" => use_into(&p.tree, mods, true),
        syn::UseTree::Path(p) if first => mods.contains(&p.ident.to_string()),
        syn::UseTree::Path(p) => use_into(&p.tree, mods, false),
        syn::UseTree::Group(g) => !g.items.is_empty() && g.items.iter().all(|i| use_into(i, mods, first)),
        _ => !first,
    }
}

/// Mark a trait-body marker's default methods (the visible arm of a hidden trait twin).
fn cover_trait_marker(it: &mut syn::Item, h: &Hidden) {
    cover_trait_marker_by(it, |tn, n| h.covers(tn) || h.covers_method(tn, n, true));
}

/// SOUNDNESS R1857 — a default body of trait `tn` in one module arm is twinned by the other arm's hidden text when that text
/// may define the trait or the member, or writes an impl of the trait that defines the member (`p14himpl`: the hidden arm's
/// `impl TI for Q { fn go_i }` overrides the default this arm's empty impl inherits). The DEFAULT is marked, not a member
/// stood on this arm's impl: a stood member would take the call from a default that may itself perform an effect
/// (`p15stood`, `Fs` on main), which turns a concrete effect into `Unknown` alone.
fn trait_default_twin(h: &Hidden, tn: &str, n: &str) -> bool {
    let tail = format!(" as {tn}::{n}");
    h.covers(tn) || h.covers_method(tn, n, true) || h.keys.iter().chain(&h.xkeys).any(|k| k.ends_with(&tail))
}

fn cover_trait_marker_by(it: &mut syn::Item, twin: impl Fn(&str, &str) -> bool) {
    let Some(mut t) = mbe_trait_body_of(it) else { return };
    let tn = t.ident.to_string();
    let mut hit = false;
    for ti in t.items.iter_mut() {
        if let syn::TraitItem::Fn(f) = ti {
            if f.default.is_some() && twin(&tn, &f.sig.ident.to_string()) {
                let n = format!("{tn}::{}", f.sig.ident);
                mark_fn_at(first_span(&*f), &mut f.attrs, &n, "I1S2MARKTRAIT");
                hit = true;
            }
        }
    }
    if hit {
        let mut ts = TokenStream::new();
        ts.extend([TokenTree::Ident(Ident::new(MBE_TRAIT_BODY_MARK, proc_macro2::Span::call_site()))]);
        ts.extend(syn::__private::ToTokens::to_token_stream(&t));
        *it = syn::Item::Verbatim(ts);
    }
}

/// THE VISIBLE SIDE, read with the same `item_keys`. `item` is what this module's hidden ITEM text may define, `member` what
/// its hidden IMPL-ITEM text may define (each keyed `Owner::m`). Cover, by kind:
/// - a PRODUCED (spliced) cfg-gated fn, `use` or module whose bound names the hidden text may define: the fn is marked, the
///   module's fns are marked, the `use` is put back as hidden text unless it leads into a module the cover marks. A produced
///   glob whose target is an inline module here is a twin only where those names meet the hidden ones; any other glob is.
/// - ANY cfg-gated method (hand-written impl included — the impl-item splice keeps no provenance) whose `Owner::m` key, or
///   whose owner type, the hidden text may define.
/// - a produced trait whose default bodies the hidden text may define (`cover_trait_marker`).
fn mark_hidden_twins(out: &mut [syn::Item], spliced: &[bool], item: &Hidden, member: &Hidden, has_hidden: bool) {
    // I1 stage 2b — THE TRIVIAL SUPERSET. Superset argument: a cfg twin of a PRODUCED gated item is either another produced
    // item (both arms expanded; this module then reads the union and nothing is hidden), an item written by hand (not this
    // pass's to cover; R1828), or text this pass did not expand. If ANY unexpanded item text remains in the module, the
    // twin may be in it, and no reading of that text can be shown to bind fewer names than rustc's expansion would; so every
    // produced gated item of the module carries the refusal, with no name matching at all. Where no unexpanded text remains,
    // the module has no hidden twin to cover. MEASURED, NOT THE DEFAULT (STAGE2B-REPORT): `CANDOR_I1S2B_TRIVIAL=1` turns it on.
    let trivial = has_hidden && candor_report::env::switch("CANDOR_I1S2B_TRIVIAL");
    // the types a produced, cfg-gated declaration of this list names: a hidden type of that name is its twin
    let type_twins: std::collections::HashSet<String> = out
        .iter()
        .enumerate()
        .filter(|(i, it)| spliced.get(*i).copied().unwrap_or(false) && is_gated(crate::lang::item_attrs(it)))
        .filter_map(|(_, it)| match it {
            syn::Item::Struct(x) => Some(x.ident.to_string()),
            syn::Item::Enum(x) => Some(x.ident.to_string()),
            syn::Item::Union(x) => Some(x.ident.to_string()),
            syn::Item::Type(x) => Some(x.ident.to_string()),
            _ => None,
        })
        .collect();
    let both = |owner: &str, m: &str| {
        let tt = type_twins.contains(owner);
        item.covers_method(owner, m, tt) || member.covers_method(owner, m, tt)
    };
    let visible_twin = |it: &syn::Item, out: &[syn::Item]| -> bool {
        if item.is_empty() {
            return false;
        }
        let k = item_keys(it, out);
        if k.opaque {
            return true; // a glob this pass cannot enumerate, beside hidden text
        }
        k.names.iter().any(|n| item.covers(n))
    };
    // SOUNDNESS R1828 (cfg-twin lane A) — a HAND-WRITTEN gated fn, inline module or `macro_rules!` definition beside hidden
    // text that may define its name is an arm too (`d11`: `#[cfg(unix)] crate::wfn! { pub fn go(); }` beside
    // `#[cfg(not(unix))] pub fn go() {}` in one file read the stub alone, ABSENT over a write). The same `visible_twin` test a
    // produced arm passes; marked without removing a binding (`cover_any(.., true)`), so a hand-written `use` or `type` is
    // left as written (a named residual) and only `Unknown` is added.
    let hand_flags: Vec<bool> = out
        .iter()
        .enumerate()
        .map(|(i, it)| {
            !spliced.get(i).copied().unwrap_or(false)
                && is_gated(crate::lang::item_attrs(it))
                && !crate::lang::is_cfg_test(crate::lang::item_attrs(it))
                && (matches!(it, syn::Item::Fn(_))
                    || matches!(it, syn::Item::Mod(m) if m.content.is_some())
                    || matches!(it, syn::Item::Macro(m) if m.ident.is_some()))
                && visible_twin(it, out)
        })
        .collect();
    let flags: Vec<bool> = out
        .iter()
        .enumerate()
        .map(|(i, it)| {
            spliced.get(i).copied().unwrap_or(false)
                && is_gated(crate::lang::item_attrs(it))
                && ((trivial
                    && matches!(it, syn::Item::Fn(_) | syn::Item::Use(_) | syn::Item::Mod(_) | syn::Item::Impl(_) | syn::Item::Type(_)))
                    || (matches!(it, syn::Item::Fn(_) | syn::Item::Use(_) | syn::Item::Mod(_) | syn::Item::Type(_))
                        && visible_twin(it, out))
                    // I1 stage 3 (R1784) — a produced, gated `macro_rules!` definition beside hidden text that may define the
                    // name: its twin arm may be in that text (Fable `macrodef_e`), so a body-position invocation must not read
                    // this arm alone. Superset argument: the same `visible_twin` test every other produced kind passes.
                    || (matches!(it, syn::Item::Macro(m) if m.ident.is_some()) && (trivial || visible_twin(it, out))))
        })
        .collect();
    let marked_mods: std::collections::HashSet<String> = out
        .iter()
        .zip(&flags)
        .filter_map(|(it, f)| match it {
            syn::Item::Mod(m) if *f && m.content.is_some() => Some(m.ident.to_string()),
            _ => None,
        })
        .collect();
    for (i, it) in out.iter_mut().enumerate() {
        if hand_flags[i] {
            if candor_report::env::trace("CANDOR_I1_INSTR") {
                eprintln!("R1828HAND\t{}", item_keys(it, &[]).names.first().cloned().unwrap_or_default()); // §E1 REACH COUNTER
            }
            cover_any(it, true);
            continue;
        }
        if flags[i] {
            let keep_use = matches!(&*it, syn::Item::Use(u) if use_into(&u.tree, &marked_mods, true));
            if !keep_use {
                if candor_report::env::trace("CANDOR_I1_INSTR") && trivial {
                    eprintln!("I1S2BTRIVIAL\t{}", item_keys(it, &[]).names.first().cloned().unwrap_or_default()); // §E1 REACH COUNTER
                }
                cover_arm(it);
            }
            continue;
        }
        // a trait-body marker exists only for a trait an expansion declared (`mbe_trait_body_marker`): produced by construction
        if mbe_trait_body_of(it).is_some() && (!item.is_empty() || trivial) {
            if trivial {
                // the trivial superset: every default body of a produced trait beside unexpanded text
                let all = Hidden { opaque: true, ..Hidden::default() };
                cover_trait_marker(it, &all);
            } else {
                cover_trait_marker(it, item);
            }
            continue;
        }
        // I1 stage 2c — a gated DEFAULT body beside the trait's own unexpanded trait-item text that may define it (`rd_e`).
        if let syn::Item::Trait(t) = it {
            if (!member.is_empty() || (is_gated(&t.attrs) && !item.is_empty())) && !candor_report::env::switch("CANDOR_I1S2C_XMOD_OFF") {
                let tn = t.ident.to_string();
                // SOUNDNESS R1828 (cfg-twin lane A) — or a default of a trait that is itself one `#[cfg]` arm, beside a twin trait
                // whose member text (or item text) may define it (`d11v2`: `#[cfg(unix)] trait T { hid! { fn go } }` beside
                // `#[cfg(not(unix))] trait T { fn go(&self) {} }` read the pure default alone).
                let trait_arm = is_gated(&t.attrs);
                for ti in t.items.iter_mut() {
                    if let syn::TraitItem::Fn(f) = ti {
                        let n = f.sig.ident.to_string();
                        if f.default.is_some() && is_gated(&f.attrs) && member.covers_method(&tn, &n, false) {
                            mark_fn_at(first_span(&*f), &mut f.attrs, &format!("{tn}::{n}"), "I1S2MARKTRAITITEM");
                        } else if f.default.is_some()
                            && trait_arm
                            && (member.covers_method(&tn, &n, false) || trait_default_twin(item, &tn, &n))
                        {
                            mark_fn_at(first_span(&*f), &mut f.attrs, &format!("{tn}::{n}"), "R1828TRAITARM");
                        }
                    }
                }
            }
            continue;
        }
        if let syn::Item::Impl(im) = it {
            if item.is_empty() && member.is_empty() {
                continue;
            }
            let owner = ty_key(&im.self_ty);
            let block = is_gated(&im.attrs);
            // THE TRAIT-IMPL ARM (UPDATE 4a-2; SOUNDNESS R1541 stage 1 exposed it, Fable RC3-5 named its cross-file form). A
            // cfg-gated `impl Tr for S` is one arm; its twin may be hidden as (a) the trait itself, whose default body the other
            // arm's impl inherits, or (b) another `impl Tr for S`, which overrides what this arm inherits or writes. Measured
            // (`fxq`, executed): `S.go()` read `Unknown macro:` on 0.42.0, and resolved to this arm's pure member, or through
            // it to the pure default, once stage 1 spliced macro-made trait impls. So: a member of this arm is marked where
            // the hidden text may define the trait (`Tr`, `Tr::m`), or an impl of `Tr` for `S`; and a member the hidden impl
            // writes that this arm lacks is STOOD HERE, marked, with the hidden member's signature, so the call lands on a
            // disclosure instead of the trait's default. What the legs read: the trait leg reads `keys` (and `opaque`) only;
            // the impl leg reads the `S as Tr` key, or BOTH `Tr` and `S` among `loose` identifiers. Hidden text that reached
            // only `loose` with neither is caught here only through the member rule's bare-name leg (`covers_method`:
            // `loose ∋ m`), which is what holds `rl_e`/`rl2_e`. This arm reads THIS module's hidden text (nested hidden
            // modules included, through `xkeys`); a twin in another module is answered at pass B from the keys every module
            // exports (stage 2c, `scan.rs`). A hidden blanket impl reaches the impl leg through its `* as Tr` key.
            // `CANDOR_I1S2_TRAITARM_OFF=1` drops this arm.
            let tr = trait_key(im);
            let ik = tr.as_ref().map(|t| format!("{owner} as {t}"));
            let hidden_trait =
                |t: &str| item.keys.contains(t) || item.xkeys.contains(t) || member.keys.contains(t) || item.opaque;
            let shape = self_shape(&owner);
            let hidden_impl = |k: &str, t: &str| {
                item.keys.contains(k)
                    || item.xkeys.contains(k)
                    || item.xkeys.contains(&format!("* as {t}"))
                    || shape.as_ref().is_some_and(|sh| item.xkeys.contains(&format!("{sh} as {t}")))
                    || member.keys.contains(k)
                    || (item.loose.contains(t) && item.loose.contains(&owner))
            };
            let arm_enabled = tr.is_some() && !candor_report::env::switch("CANDOR_I1S2_TRAITARM_OFF");
            let arm_on = block && arm_enabled;
            // I1 stage 2c — the trait leg applies to a member gated on its own as well as to a gated block (`rm_e`: a
            // hand-written `impl Tr for S { #[cfg(not(unix))] fn go(&self) {} }` beside a hidden trait default).
            let trait_twin = arm_enabled
                && tr.as_ref().is_some_and(|t| hidden_trait(t) || hidden_impl(ik.as_deref().unwrap_or(""), t))
                && (block || !candor_report::env::switch("CANDOR_I1S2C_XMOD_OFF"));
            let mut have: std::collections::HashSet<String> = std::collections::HashSet::new();
            for ii in im.items.iter_mut() {
                if let syn::ImplItem::Fn(f) = ii {
                    let n = f.sig.ident.to_string();
                    have.insert(n.clone());
                    // I1 stage 2b — a PRODUCED gated impl block is an arm of its type's configuration even where the type
                    // itself is written by hand: a hidden definition of the type's NAME (a glob `std::fs::*`, a hidden
                    // `struct File`) may bring the other arm's methods (Fable RC3-6 `handty_e`).
                    let produced_block = block && spliced.get(i).copied().unwrap_or(false);
                    if (block || is_gated(&f.attrs))
                        && (both(&owner, &n) || trait_twin || (produced_block && item.covers_method(&owner, &n, true)))
                    {
                        mark_fn_at(first_span(&*f), &mut f.attrs, &format!("{owner}::{n}"), "I1S2MARKMETH");
                    }
                }
            }
            if let (true, Some(ik)) = (arm_on, ik.as_ref()) {
                let pre = format!("{ik}::");
                let mut extra: Vec<(&String, &syn::Signature)> =
                    item.sigs.iter().filter(|(k, _)| k.starts_with(&pre) && !have.contains(&k[pre.len()..])).collect();
                extra.sort_by(|a, b| a.0.cmp(b.0));
                for (k, sig) in extra {
                    // SOUNDNESS R1863 (cfg-twin lane B) — the stood member is the UNION of its two arms: the hidden override (which
                    // this pass cannot read, hence its `Unknown`) and the trait's own default, which is what this arm's impl
                    // inherits when the visible arm is the one compiled. An EMPTY body dropped the second: the call resolved to
                    // the stood member and the effectful default (`same_e`, executed `Fs`) was charged to nothing, so `deny Fs`
                    // read 0. The body now calls the trait's member through its UFCS path, which the resolver answers with the
                    // default's unit (and, for a trait with no local default, the foreign model or nothing, so the member's own
                    // `Unknown` is still the floor). A call edge only ADDS what the default does; no charge or disclosure moves.
                    let body = match &im.trait_ {
                        Some((_, tp, _)) => stood_body(&im.self_ty, tp, sig),
                        None => syn::parse_quote!({}),
                    };
                    let mut f: syn::ImplItemFn = syn::parse_quote!(#sig #body);
                    mark_fn_as(&mut f.attrs, &format!("{owner}::{}", &k[pre.len()..]), "I1S2MARKSTOOD");
                    im.items.push(syn::ImplItem::Fn(f));
                }
            }
        }
    }
}

/// SOUNDNESS R1898 — stamp `MBE_AMBIG_ATTR` on an expansion's fns (see `Ctx::pre_def_ambiguous`).
fn push_ambig(attrs: &mut Vec<syn::Attribute>) {
    if !attrs.iter().any(|a| a.path().is_ident(crate::model::MBE_AMBIG_ATTR)) {
        let id = syn::Ident::new(crate::model::MBE_AMBIG_ATTR, proc_macro2::Span::call_site());
        attrs.push(syn::parse_quote!(#[#id]));
    }
}

fn mark_ambiguous_items(items: &mut [syn::Item]) {
    for it in items {
        match it {
            syn::Item::Fn(f) => push_ambig(&mut f.attrs),
            syn::Item::Impl(im) => {
                for ii in im.items.iter_mut() {
                    if let syn::ImplItem::Fn(f) = ii {
                        push_ambig(&mut f.attrs);
                    }
                }
            }
            syn::Item::Trait(t) => {
                for ti in t.items.iter_mut() {
                    if let syn::TraitItem::Fn(f) = ti {
                        if f.default.is_some() {
                            push_ambig(&mut f.attrs);
                        }
                    }
                }
            }
            syn::Item::Mod(m) => {
                if let Some((_, inner)) = &mut m.content {
                    mark_ambiguous_items(inner);
                }
            }
            _ => {}
        }
    }
}

/// SOUNDNESS R1863 (cfg-twin lane B) — the body of a stood member: `{ <S as Tr>::m(self, (), ..) }`, the call that reaches the
/// trait's own member. Argument values are placeholders (the body is never compiled, only read for its edges).
fn stood_body(self_ty: &syn::Type, tr: &syn::Path, sig: &syn::Signature) -> syn::Block {
    let name = &sig.ident;
    let recv = sig.receiver().is_some();
    let rest = sig.inputs.len().saturating_sub(usize::from(recv));
    let args: Vec<syn::Expr> = (0..rest).map(|_| syn::parse_quote!(())).collect();
    if recv {
        syn::parse_quote!({ <#self_ty as #tr>::#name(self #(, #args)*) })
    } else {
        syn::parse_quote!({ <#self_ty as #tr>::#name(#(#args),*) })
    }
}

/// I1 stage 1 — the explicit REFUSAL list for a spliced expansion (the denylist that replaced the R1004 allowlist).
///
/// I1 stage 2 — modules, `use`s and `extern crate`s are SPLICED: the expansion is the program rustc compiles, so a module a
/// macro makes (`mk!(bool)` → `pub mod bool { pub fn go() .. }`, R1535) is a module, and a `use` it makes binds a name
/// (`sys!(tu)` → `use crate::tu as sys;`, R1688) exactly as the hand-written item does. What stays refused is what this pass
/// cannot stand behind: a nested invocation it could not expand (kept as written, so it remains evidence) and a trait (the
/// R1748/R1754 markers stand for it). Stage 3 splices a nested `macro_rules!` DEFINITION (R1784; `CANDOR_I1S3_MDEF_OFF=1`
/// refuses it again). `CANDOR_I1S2_MODUSE_OFF=1` refuses modules, `use`s and `extern crate`s again (the stage-1 list).
fn i1_refused(g: &syn::Item) -> bool {
    match g {
        // I1 stage 3 (R1784) — a `macro_rules!` DEFINITION the expansion emits is spliced: it enters textual scope (`splice`), so
        // the invocations after it expand, and Pass A's `local_macros` reads it for a body-position invocation (R1144).
        syn::Item::Macro(m) if m.ident.is_some() && m.mac.path.is_ident("macro_rules") => mdef_off(),
        syn::Item::Macro(_) | syn::Item::Trait(_) | syn::Item::TraitAlias(_) => true,
        syn::Item::Mod(_) | syn::Item::Use(_) | syn::Item::ExternCrate(_) => {
            candor_report::env::switch("CANDOR_I1S2_MODUSE_OFF")
        }
        _ => false,
    }
}

/// cfg-twin lane A — the marker carrying one INLINE module arm's reading (pushed into the arm by `mark_module_twins`), and
/// the one carrying a whole FILE's reading as an out-of-line arm (`splice_local_macros_with`). Read by `arm_records`.
const ARM_READ_MARK: &str = "__candor_i1_arm_read";
const FILE_READ_MARK: &str = "__candor_i1_file_read";

/// One reading as marker tokens: `O` (opaque), `k:`/`l:`/`x:` + a key, loose name or impl-scoped key. `sigs` is not carried:
/// only the in-list impl arm's stood member reads it, and a claim is never read there.
fn read_marker(mark: &str, h: &Hidden) -> syn::Item {
    let mut v: Vec<String> = Vec::new();
    if h.opaque {
        v.push("O".to_string());
    }
    v.extend(h.keys.iter().map(|k| format!("k:{k}")));
    v.extend(h.loose.iter().map(|k| format!("l:{k}")));
    v.extend(h.xkeys.iter().map(|k| format!("x:{k}")));
    v.sort();
    let mut ts = TokenStream::new();
    ts.extend([TokenTree::Ident(Ident::new(mark, proc_macro2::Span::call_site()))]);
    for k in v {
        ts.extend([TokenTree::Literal(proc_macro2::Literal::string(&k))]);
    }
    syn::Item::Verbatim(ts)
}

fn marker_strings(it: &syn::Item, mark: &str) -> Option<Vec<String>> {
    let syn::Item::Verbatim(ts) = it else { return None };
    let mut toks = ts.clone().into_iter();
    if !matches!(toks.next(), Some(TokenTree::Ident(a)) if a == mark) {
        return None;
    }
    Some(
        toks.filter_map(|t| match t {
            TokenTree::Literal(l) => match syn::parse2::<syn::Lit>(TokenTree::Literal(l).into()) {
                Ok(syn::Lit::Str(s)) => Some(s.value()),
                _ => None,
            },
            _ => None,
        })
        .collect(),
    )
}

fn is_read_marker(it: &syn::Item, mark: &str) -> bool {
    marker_strings(it, mark).is_some()
}

/// A serialised reading (the `read_marker` strings joined by `\u{1d}`) back into a `Hidden`.
fn reading_of(s: &str) -> Hidden {
    let mut h = Hidden::default();
    for t in s.split('\u{1d}').filter(|t| !t.is_empty()) {
        match t.split_once(':') {
            _ if t == "O" => h.opaque = true,
            Some(("k", k)) => {
                h.keys.insert(k.to_string());
            }
            Some(("l", k)) => {
                h.loose.insert(k.to_string());
            }
            Some(("x", k)) => {
                h.xkeys.insert(k.to_string());
            }
            _ => {}
        }
    }
    h
}

/// The `ArmClaims::inline` key of one group: the inline module path of its list within the file, and the `mod` name.
fn claim_key(list: &str, name: &str) -> String {
    format!("{list}\u{1f}{name}")
}

/// SOUNDNESS R1862 / R1824 / R1828 (cfg-twin lane A) — THE CROSS-FILE CLAIMS ONE FILE'S SPLICE CONSUMES.
///
/// THE EXPOSURE. The module-twin cover (`Ctx::mark_module_twins`) is per item list. Where an arm of a `#[cfg]` module group
/// is OUT OF LINE (`mod x;`, `#[path]`, `cfg_attr(path)`), its items are another file's, which this file's splice never
/// reads, and that file's splice does not know it is an arm: so a pure out-of-line arm beside an inline arm's hidden text
/// (R1862 `k_outline2`), two `#[path]` arms (`k_path`), and a hand-written inline arm beside an out-of-line live arm
/// (`k_handout`) all read silent on v0.43.0. The R1857 stand-in answers only a PRODUCED inline arm, wholesale.
///
/// THE MODEL. Every file exports the real readings of its arms' hidden text (`arm_records`: each inline arm's, and the
/// file's own as an out-of-line arm), and the groups its lists declare, with each out-of-line arm's TARGET module path
/// (`decls::mod_targets`, which already honours `#[path]` and `cfg_attr(path)`). `scan.rs` joins them crate-wide
/// (`arm_claims`) and re-splices each file that has a claim to consume, with `ArmClaims`: `as_arm` (the union of every
/// OTHER arm's reading, for this file's own units) and `inline` (the out-of-line arms' readings, for this file's inline
/// arms). Degeneracy is by target FILE only (two `mod x;` naming one file are one arm, Fable S1): an inline arm is never
/// degenerate with anything. Marking is `cover_arm_keyed`'s, whose depth-insensitive match keeps S3.
///
/// Held SERIALISED (`read_marker`'s strings), so the table is `Sync` for the parallel splice; decoded per file
/// (`DecodedClaims`).
#[derive(Default, Clone)]
pub(crate) struct ArmClaims {
    as_arm: Option<String>,
    inline: std::collections::BTreeMap<String, String>,
}

fn ser_reading(h: &Hidden) -> String {
    let mut v: Vec<String> = h.keys.iter().map(|k| format!("k:{k}")).collect();
    v.extend(h.loose.iter().map(|k| format!("l:{k}")));
    v.extend(h.xkeys.iter().map(|k| format!("x:{k}")));
    if h.opaque {
        v.push("O".into());
    }
    v.sort();
    v.join("\u{1d}")
}

impl ArmClaims {
    pub(crate) fn is_empty(&self) -> bool {
        self.as_arm.as_ref().is_none_or(|h| h.is_empty()) && self.inline.values().all(|h| h.is_empty())
    }
    /// A stable digest of what this file consumes: part of what its decls and units were derived under.
    pub(crate) fn digest(&self) -> String {
        if self.is_empty() {
            return String::new();
        }
        let mut parts: Vec<String> = self.inline.iter().map(|(k, h)| format!("{k}={h}")).collect();
        if let Some(h) = &self.as_arm {
            parts.push(format!("@={h}"));
        }
        crate::cache::fnv1a(parts.join("\u{1e}").as_bytes())
    }
    fn decode(&self) -> DecodedClaims {
        DecodedClaims {
            as_arm: self.as_arm.as_deref().map(reading_of),
            inline: self.inline.iter().map(|(k, v)| (k.clone(), reading_of(v))).collect(),
        }
    }
}

/// `ArmClaims`, decoded for one file's splice.
#[derive(Default)]
struct DecodedClaims {
    as_arm: Option<Hidden>,
    inline: HashMap<String, Hidden>,
}

/// cfg-twin lane A — this file's arm records (`cache::FileDecls::i1_arms`), from the spliced items:
/// - `F␟<file module path>␟<reading>`: the file's own reading as an out-of-line arm (`FILE_READ_MARK`);
/// - `I␟<group>␟<list path in file>␟<name>␟<reading>`: an inline arm of a gated `mod` group (reading may be empty);
/// - `O␟<group>␟<target module path>`: one target of an out-of-line arm.
///
/// `<group>` is `<parent module path>␞<mod name>`.
pub(crate) fn arm_records(items: &[syn::Item], modpath: &str, dir: &std::path::Path, include_tests: bool) -> Vec<String> {
    fn walk(
        items: &[syn::Item],
        modpath: &str,
        inpath: &str,
        dir: &std::path::Path,
        include_tests: bool,
        out: &mut Vec<String>,
    ) {
        let targets = crate::decls::mod_targets_pub(items, modpath, dir, include_tests);
        // SOUNDNESS R1901 (cfg-twin lane A) — an out-of-line `mod` declared inside an item-position `cfg_if!` (`if #[cfg(unix)] {
        // #[path = "u.rs"] mod imp; } else { .. }`) is an arm of the same group as a written one: every branch's declarations
        // are lifted here (for the group table only; nothing else reads them), as `lang::cfg_if_use_items` lifts its `use`s.
        let lifted: Vec<syn::Item> = items
            .iter()
            .filter_map(|it| match it {
                syn::Item::Macro(m) if m.mac.path.segments.last().is_some_and(|s| s.ident == "cfg_if") => Some(m),
                _ => None,
            })
            .filter_map(|m| syn::parse2::<crate::collector::CfgIfArms>(crate::model::respan_call_site(m.mac.tokens.clone())).ok())
            .flat_map(|arms| arms.0.into_iter())
            .flat_map(|b| b.stmts.into_iter())
            .filter_map(|st| match st {
                syn::Stmt::Item(syn::Item::Mod(mut m)) if m.content.is_none() => {
                    m.attrs.push(syn::parse_quote!(#[cfg(candor_cfg_if_arm)]));
                    Some(syn::Item::Mod(m))
                }
                _ => None,
            })
            .collect();
        if !lifted.is_empty() {
            let lt = crate::decls::mod_targets_pub(&lifted, modpath, dir, include_tests);
            for (name, ts) in lt {
                let group = format!("{modpath}\u{1e}{name}");
                for t in ts {
                    out.push(format!("O\u{1f}{group}\u{1f}{t}\u{1f}{inpath}"));
                }
            }
        }
        for it in items {
            let syn::Item::Mod(m) = it else { continue };
            if !include_tests && crate::lang::is_cfg_test(&m.attrs) {
                continue;
            }
            let name = m.ident.to_string();
            let sub = if modpath.is_empty() { name.clone() } else { format!("{modpath}::{name}") };
            let group = format!("{modpath}\u{1e}{name}");
            match &m.content {
                Some((_, inner)) => {
                    if is_gated(&m.attrs) {
                        let r = inner.iter().filter_map(|x| marker_strings(x, ARM_READ_MARK)).flatten().collect::<Vec<_>>();
                        out.push(format!("I\u{1f}{group}\u{1f}{inpath}\u{1f}{name}\u{1f}{}", r.join("\u{1d}")));
                    }
                    let inp = if inpath.is_empty() { name.clone() } else { format!("{inpath}::{name}") };
                    walk(inner, &sub, &inp, &dir.join(&name), include_tests, out);
                }
                None if is_gated(&m.attrs) => {
                    for t in targets.get(&name).into_iter().flatten() {
                        out.push(format!("O\u{1f}{group}\u{1f}{t}\u{1f}{inpath}"));
                    }
                }
                None => {}
            }
        }
    }
    let mut out = Vec::new();
    let file_read: Vec<String> = items.iter().filter_map(|x| marker_strings(x, FILE_READ_MARK)).flatten().collect();
    if !file_read.is_empty() {
        out.push(format!("F\u{1f}{modpath}\u{1f}{}", file_read.join("\u{1d}")));
    }
    walk(items, modpath, "", dir, include_tests, &mut out);
    out.sort();
    out.dedup();
    out
}

/// cfg-twin lane A — the crate-wide join: per file (by its module path and its own records), the claims it consumes.
/// `records`: every file's `(module path, i1_arms)`. A group is an arm set when it has at least two DISTINCT arms (inline
/// arms, each distinct; out-of-line targets, distinct by module path, i.e. by file).
pub(crate) fn arm_claims(records: &[(String, &[String])]) -> HashMap<String, ArmClaims> {
    let mut file_read: HashMap<String, Hidden> = HashMap::new();
    // group -> (inline arms: (file modpath, list path, name, reading)), out-of-line targets
    #[derive(Default)]
    struct G {
        inline: Vec<(String, String, String, Hidden)>,
        targets: std::collections::BTreeSet<String>,
        decl_files: std::collections::BTreeSet<(String, String, String)>,
    }
    let mut groups: std::collections::BTreeMap<String, G> = Default::default();
    for (fmp, recs) in records {
        for r in recs.iter() {
            let p: Vec<&str> = r.split('\u{1f}').collect();
            match p.as_slice() {
                ["F", mp, read] => {
                    file_read.entry(mp.to_string()).or_default().absorb(reading_of(read));
                }
                ["I", g, inpath, name, read] => {
                    groups.entry(g.to_string()).or_default().inline.push((
                        fmp.clone(),
                        inpath.to_string(),
                        name.to_string(),
                        reading_of(read),
                    ));
                }
                ["O", g, t, inpath] => {
                    let e = groups.entry(g.to_string()).or_default();
                    e.targets.insert(t.to_string());
                    let name = g.rsplit('\u{1e}').next().unwrap_or("").to_string();
                    e.decl_files.insert((fmp.clone(), inpath.to_string(), name));
                }
                _ => {}
            }
        }
    }
    let mut acc: HashMap<String, DecodedClaims> = HashMap::new();
    for g in groups.values() {
        if g.targets.is_empty() || g.inline.len() + g.targets.len() < 2 {
            continue; // no out-of-line arm (the in-list cover answers it), or one arm only (Fable S1: same file twice)
        }
        // each out-of-line target, as an arm: every OTHER arm's reading
        for t in &g.targets {
            let mut h = Hidden::default();
            for (_, _, _, r) in &g.inline {
                h.absorb(r.clone());
            }
            for t2 in g.targets.iter().filter(|t2| *t2 != t) {
                if let Some(r) = file_read.get(t2) {
                    h.absorb(r.clone());
                }
            }
            if !h.is_empty() {
                let c = acc.entry(t.clone()).or_default();
                c.as_arm.get_or_insert_with(Hidden::default).absorb(h);
            }
        }
        // the declaring list's inline arms: every out-of-line target's reading
        let mut x = Hidden::default();
        for t in &g.targets {
            if let Some(r) = file_read.get(t) {
                x.absorb(r.clone());
            }
        }
        if !x.is_empty() {
            for (fmp, inpath, name) in &g.decl_files {
                acc.entry(fmp.clone()).or_default().inline.entry(claim_key(inpath, name)).or_default().absorb(x.clone());
            }
        }
    }
    acc.into_iter()
        .map(|(k, d)| {
            let c = ArmClaims {
                as_arm: d.as_arm.as_ref().map(ser_reading),
                inline: d.inline.iter().map(|(k, h)| (k.clone(), ser_reading(h))).collect(),
            };
            (k, c)
        })
        .collect()
}

/// I1 stage 2c — the marker carrying one module's exported hidden keys (`Hidden::xkeys`, `p:`-prefixed path keys, and the
/// renames the hidden text writes as `=Alias=Orig`).
const XKEYS_MARK: &str = "__candor_i1_xkeys";

/// I1 stage 2c — the crate-wide twin facts of one file: every module's exported hidden keys (`XKEYS_MARK`), the `use … as`
/// renames it writes (`Alias=Orig`, last segments), and every cfg-gated trait impl block (`S as Tr|m1,m2`, the members it
/// writes). Read into `cache::FileDecls`, unioned at the merge, consulted at pass B.
pub(crate) fn twin_facts(items: &[syn::Item]) -> (Vec<String>, Vec<String>, Vec<String>) {
    fn walk(items: &[syn::Item], x: &mut Vec<String>, r: &mut Vec<String>, g: &mut Vec<String>, gated_mod: bool) {
        for it in items {
            match it {
                syn::Item::Verbatim(ts) => {
                    let mut toks = ts.clone().into_iter();
                    if matches!(toks.next(), Some(TokenTree::Ident(a)) if a == XKEYS_MARK) {
                        for t in toks {
                            if let TokenTree::Literal(l) = t {
                                if let Ok(syn::Lit::Str(s)) = syn::parse2::<syn::Lit>(TokenTree::Literal(l).into()) {
                                    match s.value().strip_prefix('=') {
                                        Some(rn) => r.push(rn.to_string()),
                                        None => x.push(s.value()),
                                    }
                                }
                            }
                        }
                    }
                }
                syn::Item::Use(u) => use_renames(&u.tree, r),
                // I1 stage 2d — a visible `type` alias: the self type `impl Tr for Alias` names (Fable RC4-2 `trsa_e`).
                syn::Item::Type(t) if !candor_report::env::switch("CANDOR_I1S2D_ALIAS_OFF") => {
                    r.push(format!("{}={}", t.ident, ty_key(&t.ty)));
                }
                // SOUNDNESS R1897 (cfg-twin lane A) — an impl inside a `#[cfg]`-gated inline module is an arm of that cfg as surely
                // as one carrying the attribute itself: the member leg already reads it so (`decls::GatedModScope`), and without
                // the same reading here an EMPTY visible impl in a gated module (`gatevia viamod_e`) put no default into the
                // default leg, so the trait's pure default answered a call whose live arm was hidden text (ABSENT over a write).
                syn::Item::Impl(im) if is_gated(&im.attrs) || gated_mod => {
                    if let Some(t) = trait_key(im) {
                        let ms: Vec<String> = im
                            .items
                            .iter()
                            .filter_map(|ii| match ii {
                                syn::ImplItem::Fn(f) => Some(f.sig.ident.to_string()),
                                _ => None,
                            })
                            .collect();
                        g.push(format!("{} as {t}|{}", ty_key(&im.self_ty), ms.join(",")));
                    }
                }
                syn::Item::Mod(m) => {
                    if let Some((_, inner)) = &m.content {
                        walk(inner, x, r, g, gated_mod || is_gated(&m.attrs));
                    }
                }
                _ => {}
            }
        }
    }
    let (mut x, mut r, mut g) = (Vec::new(), Vec::new(), Vec::new());
    walk(items, &mut x, &mut r, &mut g, false);
    for v in [&mut x, &mut r, &mut g] {
        v.sort();
        v.dedup();
    }
    (x, r, g)
}

/// The `Alias=Orig` pairs (last segments) a `use` tree's renames write.
fn use_renames(t: &syn::UseTree, out: &mut Vec<String>) {
    match t {
        syn::UseTree::Path(p) => use_renames(&p.tree, out),
        syn::UseTree::Rename(r) => out.push(format!("{}={}", r.rename, r.ident)),
        syn::UseTree::Group(g) => g.items.iter().for_each(|i| use_renames(i, out)),
        _ => {}
    }
}

/// I1 stage 2c — the lookup keys of one gated impl member, `Tr|S|m` (`Tr` empty for an inherent impl), with the trait's
/// renames followed: what the crate-wide hidden keys are checked for (pass B).
pub(crate) fn twin_cand_keys(cand: &str, renames: &std::collections::BTreeMap<String, std::collections::BTreeSet<String>>) -> Vec<String> {
    let mut it = cand.splitn(3, '|');
    let (Some(tr), Some(s0), Some(m)) = (it.next(), it.next(), it.next()) else { return Vec::new() };
    fn closure(x: &str, renames: &std::collections::BTreeMap<String, std::collections::BTreeSet<String>>) -> Vec<String> {
        let mut v: Vec<String> = vec![x.to_string()];
        let mut i = 0;
        while i < v.len() && i < 16 {
            if let Some(o) = renames.get(&v[i]) {
                for y in o {
                    if !v.contains(y) {
                        v.push(y.clone());
                    }
                }
            }
            i += 1;
        }
        v
    }
    // I1 stage 2d — the self type's aliases and renames too (`type Alias = S;`, `use S as Alias`), both ways.
    let ss: Vec<String> = if s0.is_empty() { Vec::new() } else { closure(s0, renames) };
    let mut out = Vec::new();
    for s in &ss {
        out.push(format!("{s}::{m}"));
    }
    if tr.is_empty() {
        if s0.is_empty() {
            out.push(format!("p:{m}")); // a gated free fn: only the measured `all` scope asks
        }
        return out;
    }
    let trs = closure(tr, renames);
    for t in &trs {
        out.push(t.clone());
        out.push(format!("{t}::{m}"));
        out.push(format!("* as {t}")); // a hidden blanket impl of the trait
        out.push(format!("* as {t}::{m}"));
        for s in &ss {
            out.push(format!("{s} as {t}"));
            out.push(format!("{s} as {t}::{m}"));
            if let Some(sh) = self_shape(s) {
                out.push(format!("{sh} as {t}")); // a hidden blanket over this shape (`impl<T> Tr for &T`)
                out.push(format!("{sh} as {t}::{m}"));
            }
        }
    }
    out
}

/// R-ROOT — does any of these (spliced) items write `cfg` anywhere? Over-matches (`cfg_x`, `my_cfg`), which only refuses.
fn mentions_cfg<T: quote::ToTokens>(items: &[T]) -> bool {
    items.iter().any(|i| i.to_token_stream().to_string().contains("cfg"))
}

/// I1 stage 1 — the marker that stands where a fully expanded invocation was. Read by `mbe_expanded_of`.
const MBE_EXPANDED_MARK: &str = "__candor_mbe_expanded";

/// I1 stage 1 — the macro name of a marker `splice` left for a fully expanded invocation.
pub(crate) fn mbe_expanded_of(it: &syn::Item) -> Option<String> {
    let syn::Item::Verbatim(ts) = it else { return None };
    let mut ids = ts.clone().into_iter();
    match (ids.next(), ids.next(), ids.next()) {
        (Some(TokenTree::Ident(a)), Some(TokenTree::Ident(n)), None) if a == MBE_EXPANDED_MARK => Some(n.to_string()),
        _ => None,
    }
}

/// I1 stage 1 — the crate-wide retirement input of one file: the macros it holds a fully expanded invocation of, and
/// the LAST segment of every macro path it still holds as an unexpanded invocation anywhere (item, impl-item, statement
/// or expression position). A `macro_rules!` body's hedge evidence is retired only for a name in the first set and not
/// in the second.
pub(crate) fn expansion_facts(items: &[syn::Item]) -> (Vec<String>, Vec<String>) {
    struct V {
        expanded: std::collections::BTreeSet<String>,
        unexpanded: std::collections::BTreeSet<String>,
    }
    impl<'ast> syn::visit::Visit<'ast> for V {
        fn visit_item(&mut self, i: &'ast syn::Item) {
            if let Some(n) = mbe_expanded_of(i) {
                self.expanded.insert(n);
            }
            syn::visit::visit_item(self, i);
        }
        fn visit_macro(&mut self, m: &'ast syn::Macro) {
            if let Some(s) = m.path.segments.last() {
                if s.ident != "macro_rules" {
                    self.unexpanded.insert(s.ident.to_string());
                }
            }
        }
    }
    let mut v = V { expanded: Default::default(), unexpanded: Default::default() };
    for it in items {
        syn::visit::Visit::visit_item(&mut v, it);
    }
    (v.expanded.into_iter().collect(), v.unexpanded.into_iter().collect())
}

/// I1 oracle seed — `CANDOR_I1_SEED_DROP=<macro>` leaves every item-position invocation of that macro unexpanded, so its
/// units are ABSENT; the oracle's presence leg must report them. An instrument, recorded as a switch.
fn seed_dropped(mac: &syn::Macro) -> bool {
    let Some(id) = mac.path.get_ident() else { return false };
    let n = id.to_string();
    candor_report::env::switch_value("CANDOR_I1_SEED_DROP", candor_report::env::Selects::AnyOf(&[n.as_str()]))
}

/// I1 oracle seed — every fn body in a spliced item (a free fn, or an impl's methods) emptied to `{}`.
fn empty_bodies(g: &mut syn::Item) {
    match g {
        syn::Item::Fn(f) => f.block.stmts.clear(),
        syn::Item::Impl(i) => {
            for ii in i.items.iter_mut() {
                if let syn::ImplItem::Fn(f) = ii {
                    f.block.stmts.clear();
                }
            }
        }
        // I1 stage 2 — a spliced MODULE's items too, or the seed could not reach a stage-2 shape.
        syn::Item::Mod(m) => {
            if let Some((_, inner)) = &mut m.content {
                inner.iter_mut().for_each(empty_bodies);
            }
        }
        _ => {}
    }
}

/// SOUNDNESS R1541 / I1 stage 1 (the R1804 exposure) — does this file hold anything that can add an `impl` the scan cannot
/// see: a non-std `#[derive]`, an attribute that is not a builtin (an attribute proc macro), or an item-position invocation
/// of a macro this crate does not define (a function-like proc macro or another crate's `macro_rules!`)? Where it does, a
/// trait's visible implementors are not all of them, and R828's `macro_rules!`-body hedge — measured on wasm-bindgen 0.2.130
/// (`#[wasm_bindgen]` adds `impl RefFromWasmAbi for JsClosure`) and palette 0.7.7 (`#[derive(WithAlpha)]`) — was the only
/// disclosure on a dispatch over such a trait. `cache::retire_expanded` keeps it in that crate. A guard on the retirement,
/// not a fix for R1804: it keeps the old cover, it does not supply the missing implementor.
/// The toolchain's own function-like macros (std / core / alloc, and the prelude's). Their expansions are fixed by the
/// toolchain and add no `impl` of a caller's choosing (Fable merge re-check RC2: the hold is for every OTHER bang macro).
pub(crate) const TOOLCHAIN_BANG_MACROS: &[&str] = &[
    "println", "print", "eprintln", "eprint", "format", "format_args", "write", "writeln", "panic", "assert", "assert_eq",
    "assert_ne", "debug_assert", "debug_assert_eq", "debug_assert_ne", "vec", "unreachable", "unimplemented", "todo",
    "matches", "dbg", "concat", "stringify", "line", "column", "file", "module_path", "env", "option_env", "include_str",
    "include_bytes", "include", "cfg", "compile_error", "thread_local", "try", "ready", "addr_of", "addr_of_mut",
    "offset_of", "assert_matches", "debug_assert_matches", "const_format_args", "format_args_nl", "concat_idents",
    "trace_macros", "log_syntax", "asm", "global_asm", "naked_asm", "pin", "is_x86_feature_detected",
    "is_aarch64_feature_detected", "is_arm_feature_detected", "unreachable_unchecked", "macro_rules",
    // candor's own placeholder for a visible `use` twin put back as unexpanded text (`mark_hidden_twins`)
    "__candor_i1_hidden_twin_use",
];

/// SOUNDNESS R1541 / R1804, Fable's merge re-checks RC2 and RC2-1/RC2-2 — the TEXT THIS SCAN NEVER PARSES, read for what it
/// can add. An `impl` the scan cannot see hides inside any of:
/// - an invocation of a macro whose expansion is not read here — a function-like proc macro, a dependency's `macro_rules!`
///   (`pub fn setup() { other::make_go!(Hid); }`), at ANY position, a fn body's statements and expressions included;
/// - the ARGUMENTS of ANY unexpanded invocation, a toolchain macro's included (`println!("{}", { impl Go for Hid {..} 1 })`):
///   an `impl` token, a non-builtin attribute or derive, or a nested bang invocation;
/// - the BODY of a local `macro_rules!` that is invoked where it is not expanded (a fn body), followed transitively through
///   the local macros it invokes: `macro_rules! wrap { ($t:ident) => { $crate::re::make_go_dep!($t); } }` + `wrap!(Hid)`
///   in a body;
/// - an `include!` the parse could not splice (an `OUT_DIR` target): its whole text.
///
/// A name is not trusted under an import that may rebind it: a `#[macro_use] extern crate dep` (crate-wide) or a glob `use`
/// (this file) puts a dependency's macro ahead of the std prelude (a `vec!` that emits an impl) and beside a `#[cfg]`-gated
/// local stub; only an UNGATED local `macro_rules!` of that name outranks it.
///
/// Returned as ENTRIES the merge judges, because which names are local and which local macros are invoked unexpanded are
/// crate-wide facts:
/// - `!{why}` — this file's unparsed text can add an impl (hold);
/// - `::{path}` — an invocation of a path rooted at another crate, a `$var!` metavariable invocation, or a bare name this
///   file imports by `use` (foreign);
/// - `^{name}` — a `crate::`/`self::`/`super::`/`$crate::` path: foreign unless the crate defines that `macro_rules!`;
/// - `?{name}` — a bare name under this file's glob import: foreign unless an UNGATED local definition exists;
/// - `{name}` — a bare name: foreign unless local or a toolchain macro (and, under a crate-wide `macro_use`, ungated local);
/// - `={name}` — an UNGATED `macro_rules!` this file defines, at any depth;
/// - `*macro_use` — this file holds a non-toolchain `#[macro_use] extern crate`;
/// - `*glob` — this file holds a non-toolchain glob `use` of a non-local root (its names reach other modules by `use super::*`);
/// - `%{module}` — (added by `cache::file_decls`) an out-of-line `mod NAME;`, judged at the merge against the files read;
/// - `@{mac}>{entry}` — an entry read in the BODY of local `macro_rules! mac`, judged only when `mac` is invoked unexpanded
///   (directly or through another such body); a bare `{name}` there also marks that local macro unexpanded.
///
/// `#[cfg(test)]` items are skipped: they are not in the build the report describes.
pub(crate) fn foreign_bang_macros(items: &[syn::Item]) -> Vec<String> {
    let mut imported = std::collections::HashSet::new();
    imported_names(items, &mut imported);
    // A name (a macro, or a module a macro path starts from) this file imports ONLY from the toolchain (`use core::arch::asm;`,
    // `use core::ptr;`) is the toolchain's: measured, cmov's `asm!` under `use core::arch::asm` read as foreign and held 12
    // records over nothing. A name imported from both a toolchain crate and another stays foreign.
    let toolchain_only = toolchain_imports(items);
    for n in &toolchain_only {
        imported.remove(n);
        imported.insert(format!("\u{1}tc:{n}")); // read by `judge_path` for a path's first segment (`ptr::addr_of!`)
    }
    let glob = imported.contains("*");
    let mut out = std::collections::BTreeSet::new();
    if glob {
        out.insert("*glob".to_string()); // judged crate-wide: `use super::*` carries a glob's names down
    }
    for it in items {
        if let syn::Item::ExternCrate(e) = it {
            if e.attrs.iter().any(|a| a.path().is_ident("macro_use"))
                && !matches!(e.ident.to_string().as_str(), "std" | "core" | "alloc")
                && !crate::lang::is_cfg_test(&e.attrs)
            {
                out.insert("*macro_use".to_string());
            }
        }
    }
    struct V<'a> {
        imported: &'a std::collections::HashSet<String>,
        glob: bool,
        out: std::collections::BTreeSet<String>,
    }
    impl<'ast> syn::visit::Visit<'ast> for V<'_> {
        fn visit_item(&mut self, it: &'ast syn::Item) {
            if crate::lang::is_cfg_test(crate::lang::item_attrs(it)) {
                return;
            }
            if let syn::Item::Macro(m) = it {
                if let (Some(id), true) = (&m.ident, m.mac.path.is_ident("macro_rules")) {
                    let name = id.to_string();
                    if !crate::lang::has_cfg(&m.attrs) {
                        self.out.insert(format!("={name}"));
                    }
                    let mut body = std::collections::BTreeSet::new();
                    unparsed_tokens(m.mac.tokens.clone(), true, false, self.imported, &mut body);
                    for e in body {
                        self.out.insert(format!("@{name}>{e}"));
                    }
                    return;
                }
            }
            syn::visit::visit_item(self, it);
        }
        fn visit_macro(&mut self, m: &'ast syn::Macro) {
            let segs: Vec<String> = m.path.segments.iter().map(|s| s.ident.to_string()).collect();
            if segs.last().is_some_and(|l| l == "macro_rules") {
                return;
            }
            if let Some(e) = judge_path(&segs, m.path.leading_colon.is_some(), false, self.glob, self.imported) {
                self.out.insert(e);
            }
            if segs.last().is_some_and(|l| l == "include") {
                // A spliced `include!` is gone by now; one still here is text the parse could not read (an `OUT_DIR` target).
                self.out.insert("!an include! the parse could not splice".to_string());
            }
            unparsed_tokens(m.tokens.clone(), false, self.glob, self.imported, &mut self.out);
        }
    }
    let mut v = V { imported: &imported, glob, out };
    for it in items {
        syn::visit::Visit::visit_item(&mut v, it);
    }
    v.out.into_iter().collect()
}

/// Names this file's `use` items bind from `std`/`core`/`alloc` and from nowhere else.
fn toolchain_imports(items: &[syn::Item]) -> std::collections::HashSet<String> {
    fn walk(t: &syn::UseTree, root: Option<&str>, tc: &mut std::collections::HashSet<String>, other: &mut std::collections::HashSet<String>) {
        let is_tc = matches!(root, Some("std") | Some("core") | Some("alloc"));
        match t {
            syn::UseTree::Path(p) => {
                let first = p.ident.to_string();
                walk(&p.tree, Some(root.unwrap_or(first.as_str())), tc, other);
            }
            syn::UseTree::Name(n) if root.is_some() => {
                if is_tc { tc.insert(n.ident.to_string()) } else { other.insert(n.ident.to_string()) };
            }
            syn::UseTree::Rename(r) if root.is_some() => {
                if is_tc { tc.insert(r.rename.to_string()) } else { other.insert(r.rename.to_string()) };
            }
            syn::UseTree::Group(g) => g.items.iter().for_each(|x| walk(x, root, tc, other)),
            _ => {}
        }
    }
    let (mut tc, mut other) = (std::collections::HashSet::new(), std::collections::HashSet::new());
    for it in items {
        if let syn::Item::Use(u) = it {
            walk(&u.tree, if u.leading_colon.is_some() { Some("::") } else { None }, &mut tc, &mut other);
        }
    }
    tc.retain(|n| !other.contains(n));
    tc
}

/// One invocation path as an entry (see `foreign_bang_macros`). `None` for a toolchain-rooted path.
fn judge_path(
    segs: &[String],
    leading_colon: bool,
    dollar_crate: bool,
    glob: bool,
    imported: &std::collections::HashSet<String>,
) -> Option<String> {
    let leaf = segs.last()?;
    if dollar_crate {
        return Some(format!("^{leaf}"));
    }
    if segs.len() > 1 || leading_colon {
        return match segs[0].as_str() {
            "std" | "core" | "alloc" => None,
            m if !leading_colon && imported.contains(&format!("\u{1}tc:{m}")) => None,
            "crate" | "self" | "super" => Some(format!("^{leaf}")),
            _ => Some(format!("::{}", segs.join("::"))),
        };
    }
    Some(if imported.contains(leaf) {
        format!("::{leaf}")
    } else if glob {
        format!("?{leaf}")
    } else {
        leaf.clone()
    })
}

/// Read a token stream the scan does not parse. `body` is a local `macro_rules!` definition (matchers and transcribers):
/// there an `impl` is R828's to hedge (kept while the macro is unexpanded) and a `#[$m]` forwards the caller's attribute,
/// which the caller's own arguments carry. Elsewhere (an invocation's arguments) an `impl` token is a hidden impl.
fn unparsed_tokens(
    ts: TokenStream,
    body: bool,
    glob: bool,
    imported: &std::collections::HashSet<String>,
    out: &mut std::collections::BTreeSet<String>,
) {
    const KEYWORDS: &[&str] = &[
        "as", "break", "const", "continue", "else", "enum", "extern", "false", "fn", "for", "if", "impl", "in", "let", "loop",
        "match", "mod", "move", "mut", "pub", "ref", "return", "static", "struct", "trait", "true", "type", "unsafe", "use",
        "where", "while", "async", "await", "dyn", "yield", "box",
    ];
    fn has_dollar(ts: &TokenStream) -> bool {
        ts.clone().into_iter().any(|t| match t {
            TokenTree::Punct(p) => p.as_char() == '$',
            TokenTree::Group(g) => has_dollar(&g.stream()),
            _ => false,
        })
    }
    let tt: Vec<TokenTree> = ts.into_iter().collect();
    for i in 0..tt.len() {
        match &tt[i] {
            TokenTree::Ident(id) if id == "impl" && !body => {
                out.insert("!an impl in a macro's arguments".to_string());
            }
            TokenTree::Punct(p) if p.as_char() == '#' => {
                let mut j = i + 1;
                if matches!(tt.get(j), Some(TokenTree::Punct(q)) if q.as_char() == '!') {
                    j += 1;
                }
                if let Some(TokenTree::Group(g)) = tt.get(j) {
                    if g.delimiter() == Delimiter::Bracket && !has_dollar(&g.stream()) {
                        let hides = syn::parse2::<syn::Meta>(g.stream()).map(|m| meta_hides(&m)).unwrap_or(true);
                        if hides {
                            out.insert("!an attribute or derive in unparsed text".to_string());
                        }
                    }
                }
            }
            TokenTree::Punct(p) if p.as_char() == '!' && p.spacing() == Spacing::Alone && i > 0 => {
                let (Some(TokenTree::Ident(last)), Some(TokenTree::Group(_))) = (tt.get(i - 1), tt.get(i + 1)) else { continue };
                if KEYWORDS.contains(&last.to_string().as_str()) {
                    continue;
                }
                // Walk the path back: `a::b::c!`, `::a::c!`, `$crate::c!`, `$m!`.
                let mut segs = vec![last.to_string()];
                let mut k = i - 1;
                let (mut leading, mut dollar_crate, mut metavar) = (false, false, false);
                loop {
                    let colons = k >= 2
                        && matches!(&tt[k - 1], TokenTree::Punct(c) if c.as_char() == ':')
                        && matches!(&tt[k - 2], TokenTree::Punct(c) if c.as_char() == ':' && c.spacing() == Spacing::Joint);
                    if colons {
                        match tt.get(k.wrapping_sub(3)) {
                            Some(TokenTree::Ident(seg)) if k >= 3 => {
                                segs.insert(0, seg.to_string());
                                k -= 3;
                                continue;
                            }
                            _ => {
                                leading = true;
                                break;
                            }
                        }
                    }
                    if k >= 1 && matches!(&tt[k - 1], TokenTree::Punct(d) if d.as_char() == '$') {
                        if segs[0] == "crate" {
                            dollar_crate = true;
                            segs.remove(0);
                        } else {
                            metavar = true;
                        }
                    }
                    break;
                }
                if metavar {
                    out.insert(format!("::${}", segs.join("::")));
                } else if segs.last().is_some_and(|l| l != "macro_rules") {
                    if let Some(e) = judge_path(&segs, leading, dollar_crate, glob, imported) {
                        out.insert(e);
                    }
                    if segs.last().is_some_and(|l| l == "include") {
                        out.insert("!an include! the parse could not splice".to_string());
                    }
                }
            }
            TokenTree::Group(g) => unparsed_tokens(g.stream(), body, glob, imported, out),
            _ => {}
        }
    }
}

/// The merge's reading of every file's `foreign_bang_macros` entries. Returns the first reason the crate can add an impl
/// this scan cannot see (if any), and every local `macro_rules!` that is invoked unexpanded only THROUGH another local
/// macro's body (`wrap2!` in a fn body invoking `mk3!`): their R828 records must not be retired as fully expanded.
pub(crate) fn judge_unparsed(
    entries: &std::collections::BTreeSet<String>,
    local: &std::collections::HashMap<String, String>,
    unexpanded: &std::collections::BTreeSet<String>,
) -> (Option<String>, Vec<String>) {
    // Fable re-check 3, RC3-2: a dependency glob's rebinding is not confined to its file (`use super::*` carries it down),
    // so a glob ANYWHERE in the crate, like a `#[macro_use] extern crate`, puts a dependency's macro in reach of every bare
    // name; and under such an import only an UNGATED local definition outranks it. Executed: `use dmlib2::*;` + a
    // `#[cfg(any())]` empty stub in `lib.rs`, `use super::*;` + `make_go_dep!(Hid)` in `inner.rs` wrote, 4b52cf1c read `[]`.
    let rebind = entries.contains("*macro_use") || entries.contains("*glob");
    let ungated: std::collections::HashSet<&str> = entries.iter().filter_map(|e| e.strip_prefix('=')).collect();
    let is_local = |n: &str| ungated.contains(n) || (!rebind && local.contains_key(n));
    // `None` = not foreign; `Some(Some(n))` = a local macro to follow; `Some(None)` = foreign.
    let judge = |e: &str| -> Result<Option<String>, ()> {
        if e.starts_with('!') || e.starts_with("::") {
            return Err(());
        }
        if let Some(n) = e.strip_prefix('^') {
            return if is_local(n) { Ok(Some(n.to_string())) } else { Err(()) };
        }
        if let Some(n) = e.strip_prefix('?') {
            return if ungated.contains(n) { Ok(Some(n.to_string())) } else { Err(()) };
        }
        if is_local(e) {
            return Ok(Some(e.to_string()));
        }
        if !rebind && TOOLCHAIN_BANG_MACROS.contains(&e) { Ok(None) } else { Err(()) }
    };
    let mut bodies: HashMap<&str, Vec<&str>> = HashMap::new();
    for e in entries {
        if let Some((mac, inner)) = e.strip_prefix('@').and_then(|r| r.split_once('>')) {
            bodies.entry(mac).or_default().push(inner);
        }
    }
    // The file-level entries (invocations and their arguments) are judged as they stand.
    for e in entries {
        if e.starts_with('@') || e.starts_with('=') || e.starts_with('%') || e.starts_with('*') {
            continue;
        }
        if judge(e).is_err() {
            return (Some(e.clone()), vec![]);
        }
    }
    // Local bodies, from every local macro invoked unexpanded, transitively.
    let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
    let mut work: Vec<String> = unexpanded.iter().filter(|n| is_local(n)).cloned().collect();
    let mut via_body = vec![];
    while let Some(mac) = work.pop() {
        if !seen.insert(mac.clone()) {
            continue;
        }
        for inner in bodies.get(mac.as_str()).into_iter().flatten() {
            match judge(inner) {
                Err(()) => return (Some(format!("{mac} -> {inner}")), via_body),
                Ok(Some(n)) => {
                    if !unexpanded.contains(&n) {
                        via_body.push(n.clone());
                    }
                    work.push(n);
                }
                Ok(None) => {}
            }
        }
    }
    (None, via_body)
}

// The attribute reading shared by the item walk and the unparsed-text reading (Fable re-check 2).
const STD_DERIVES: &[&str] = &["Debug", "Clone", "Copy", "PartialEq", "Eq", "PartialOrd", "Ord", "Hash", "Default"];
const BUILTIN: &[&str] = &[
    "cfg", "cfg_attr", "derive", "doc", "allow", "warn", "deny", "forbid", "expect", "inline", "cold", "must_use",
    "deprecated", "repr", "non_exhaustive", "test", "ignore", "should_panic", "bench", "path", "macro_use", "macro_export",
    "no_mangle", "export_name", "link", "link_name", "link_section", "used", "track_caller", "automatically_derived",
    "rustfmt", "clippy", "diagnostic", "unsafe", "target_feature", "proc_macro", "proc_macro_derive", "proc_macro_attribute",
    "naked", "instruction_set", "debugger_visualizer", "collapse_debuginfo", "global_allocator", "panic_handler", "lang",
    "stable", "unstable", "rustc_const_stable", "rustc_const_unstable", "may_dangle", "optimize", "coverage",
    "no_coverage", "doc_cfg", "feature", "rustc_diagnostic_item", "const_trait", "fundamental", "no_std", "no_implicit_prelude",
    "crate_type", "crate_name", "recursion_limit", "type_length_limit", "windows_subsystem", "no_main", "macro_escape",
    "target_arch", "ignore_function", "rustversion",
    // candor's OWN marker attributes, none of which is an attribute macro: the I1 stage-2 twin mark (`mark_hidden_twins`), the
    // R1157/R1180 operator-impl marks and the R1146 synthesised-derive mark. Read as attribute macros they held every R828
    // hedge in any crate with a macro-made operator impl or a synthesised comparison (Fable stage-2 review, finding (c)).
    "candor_i1_hidden_twin", "candor_i1_ambiguous_macro", "candor_mbe_op", "candor_mbe_prim_op", "candor_derive_synth",
    // I1 stage 3 — the emitted-definition and produced-twin marks (`model::MBE_EMITTED_DEF_ATTR`, `MBE_TWIN_DEF_ATTR`). Unlisted,
    // they held every crate with an emitted `macro_rules!` unclosable (Fable stage-3 review D2, `fmarkerhold2`).
    "__candor_i1_emitted_def", "__candor_i1_twin_def", "__candor_i1_twin_def_hand",
];
const STD_ITEM_MACROS: &[&str] = &["macro_rules", "thread_local", "include", "compile_error", "global_asm", "asm"];
fn derive_hides(list: &syn::punctuated::Punctuated<syn::Path, syn::Token![,]>) -> bool {
    list.iter().any(|d| !STD_DERIVES.contains(&d.segments.last().map(|s| s.ident.to_string()).unwrap_or_default().as_str()))
}
fn path_hides(p: &syn::Path) -> bool {
    let first = p.segments.first().map(|s| s.ident.to_string()).unwrap_or_default();
    // `rustversion::since(..)` & co. only gate; every other multi-segment or unknown path is a macro.
    !(p.segments.len() == 1 && BUILTIN.contains(&first.as_str()) || matches!(first.as_str(), "rustfmt" | "clippy" | "rustversion"))
}
/// One `Meta` as an attribute body: `derive(..)`, a nested `cfg_attr(pred, ..)`, or any other path.
fn meta_hides(m: &syn::Meta) -> bool {
    let p = m.path();
    if p.is_ident("derive") {
        return match m {
            syn::Meta::List(l) => l
                .parse_args_with(syn::punctuated::Punctuated::<syn::Path, syn::Token![,]>::parse_terminated)
                .map(|l| derive_hides(&l))
                .unwrap_or(true),
            _ => false,
        };
    }
    if p.is_ident("cfg_attr") {
        return match m {
            syn::Meta::List(l) => l
                .parse_args_with(syn::punctuated::Punctuated::<syn::Meta, syn::Token![,]>::parse_terminated)
                .map(|metas| metas.iter().skip(1).any(meta_hides)) // the first is the predicate
                .unwrap_or(true),
            _ => false,
        };
    }
    path_hides(p)
}
fn attrs_hide(attrs: &[syn::Attribute]) -> bool {
    attrs.iter().any(|a| meta_hides(&a.meta))
}

pub(crate) fn has_hidden_impl_surface(items: &[syn::Item]) -> bool {
    // THE POSITIONS (Fable merge review §4: the first cut read only plain attributes on fn/struct/enum/union/impl/trait and
    // missed 7 of 9 measured shapes). An attribute macro or a derive can add an `impl` from ANY item it decorates, at ANY
    // depth the splice walks — a module, an `extern` block and its foreign items, a `const`/`static`/`type`, an `impl` and
    // its items, a trait and its items, an item inside a fn body or a block — and `#[cfg_attr(pred, ..)]` can carry either,
    // nested. So this is a `syn::visit` over EVERY item and associated item of the file (fields and variants excluded: an
    // attribute there is a derive helper, never a macro), with `cfg_attr` read recursively; plus every item-position macro
    // invocation still unexpanded at any depth, and the brace groups of one (`cfg_if! { if #[cfg(..)] { items } }`) parsed
    // as items and walked the same way.
    struct V {
        hit: bool,
    }
    impl V {
        fn macro_body(&mut self, mac: &syn::Macro) {
            // An unexpanded invocation: its brace groups may be item lists (`cfg_if!`'s branches); walk them too.
            fn groups(ts: proc_macro2::TokenStream, v: &mut V) {
                for t in ts {
                    if let TokenTree::Group(g) = t {
                        if g.delimiter() == Delimiter::Brace {
                            if let Ok(f) = syn::parse2::<syn::File>(g.stream()) {
                                for it in &f.items {
                                    syn::visit::Visit::visit_item(v, it);
                                }
                            }
                        }
                        groups(g.stream(), v);
                    }
                }
            }
            groups(mac.tokens.clone(), self);
        }
    }
    impl<'ast> syn::visit::Visit<'ast> for V {
        fn visit_item(&mut self, it: &'ast syn::Item) {
            if self.hit {
                return;
            }
            let attrs = crate::lang::item_attrs(it);
            if crate::lang::is_cfg_test(attrs) {
                return;
            }
            if attrs_hide(attrs) {
                self.hit = true;
                return;
            }
            if let syn::Item::Macro(m) = it {
                if m.ident.is_none() {
                    let leaf = m.mac.path.segments.last().map(|s| s.ident.to_string()).unwrap_or_default();
                    // Read AFTER the splice: a fully expanded invocation is a marker by now, so any item-position invocation
                    // still here was not expanded (another crate's macro, a function-like proc macro, a refused or partial
                    // local one). `cfg_if!`/`pin_project!`-style item lists are walked instead of counted.
                    if !STD_ITEM_MACROS.contains(&leaf.as_str()) && !matches!(leaf.as_str(), "cfg_if" | "pin_project") {
                        self.hit = true;
                        return;
                    }
                    self.macro_body(&m.mac);
                    return;
                }
            }
            syn::visit::visit_item(self, it);
        }
        fn visit_impl_item(&mut self, ii: &'ast syn::ImplItem) {
            let attrs: &[syn::Attribute] = match ii {
                syn::ImplItem::Fn(f) => &f.attrs,
                syn::ImplItem::Const(c) => &c.attrs,
                syn::ImplItem::Type(t) => &t.attrs,
                syn::ImplItem::Macro(m) => &m.attrs,
                _ => &[],
            };
            if attrs_hide(attrs) {
                self.hit = true;
                return;
            }
            syn::visit::visit_impl_item(self, ii);
        }
        fn visit_trait_item(&mut self, ti: &'ast syn::TraitItem) {
            let attrs: &[syn::Attribute] = match ti {
                syn::TraitItem::Fn(f) => &f.attrs,
                syn::TraitItem::Const(c) => &c.attrs,
                syn::TraitItem::Type(t) => &t.attrs,
                syn::TraitItem::Macro(m) => &m.attrs,
                _ => &[],
            };
            if attrs_hide(attrs) {
                self.hit = true;
                return;
            }
            syn::visit::visit_trait_item(self, ti);
        }
        fn visit_foreign_item(&mut self, fi: &'ast syn::ForeignItem) {
            let attrs: &[syn::Attribute] = match fi {
                syn::ForeignItem::Fn(f) => &f.attrs,
                syn::ForeignItem::Static(s) => &s.attrs,
                syn::ForeignItem::Type(t) => &t.attrs,
                syn::ForeignItem::Macro(m) => &m.attrs,
                _ => &[],
            };
            if attrs_hide(attrs) {
                self.hit = true;
                return;
            }
            syn::visit::visit_foreign_item(self, fi);
        }
    }
    let mut v = V { hit: false };
    for it in items {
        syn::visit::Visit::visit_item(&mut v, it);
        if v.hit {
            return true;
        }
    }
    false
}

/// I1 stage 1 — `peel_groups` over the types a spliced DECLARATION carries (struct / enum / union fields, an alias, a
/// const or static). Impls are peeled where they always were.
fn peel_decl_groups(g: &mut syn::Item) {
    fn fields(f: &mut syn::Fields) {
        for x in f.iter_mut() {
            peel_groups(&mut x.ty);
        }
    }
    match g {
        syn::Item::Struct(s) => fields(&mut s.fields),
        syn::Item::Union(u) => u.fields.named.iter_mut().for_each(|x| peel_groups(&mut x.ty)),
        syn::Item::Enum(e) => e.variants.iter_mut().for_each(|v| fields(&mut v.fields)),
        syn::Item::Type(t) => peel_groups(&mut t.ty),
        syn::Item::Const(c) => peel_groups(&mut c.ty),
        syn::Item::Static(st) => peel_groups(&mut st.ty),
        _ => {}
    }
}

fn impl_item_attrs(ii: &syn::ImplItem) -> &[syn::Attribute] {
    match ii {
        syn::ImplItem::Fn(f) => &f.attrs,
        syn::ImplItem::Const(c) => &c.attrs,
        syn::ImplItem::Type(t) => &t.attrs,
        syn::ImplItem::Macro(m) => &m.attrs,
        _ => &[],
    }
}

fn impl_item_attrs_mut(ii: &mut syn::ImplItem) -> Option<&mut Vec<syn::Attribute>> {
    match ii {
        syn::ImplItem::Fn(f) => Some(&mut f.attrs),
        syn::ImplItem::Const(c) => Some(&mut c.attrs),
        syn::ImplItem::Type(t) => Some(&mut t.attrs),
        syn::ImplItem::Macro(m) => Some(&mut m.attrs),
        _ => None,
    }
}

/// A `$t:ty` fragment transcribes as an invisible-delimited `Type::Group`; replace each, at any depth, by its content.
fn peel_groups(t: &mut syn::Type) {
    while let syn::Type::Group(g) = t {
        let inner = (*g.elem).clone();
        *t = inner;
    }
    match t {
        syn::Type::Reference(r) => peel_groups(&mut r.elem),
        syn::Type::Paren(p) => peel_groups(&mut p.elem),
        syn::Type::Slice(s) => peel_groups(&mut s.elem),
        syn::Type::Array(a) => peel_groups(&mut a.elem),
        syn::Type::Ptr(p) => peel_groups(&mut p.elem),
        syn::Type::Tuple(tu) => tu.elems.iter_mut().for_each(peel_groups),
        syn::Type::Path(p) => {
            for seg in p.path.segments.iter_mut() {
                if let syn::PathArguments::AngleBracketed(ab) = &mut seg.arguments {
                    for a in ab.args.iter_mut() {
                        if let syn::GenericArgument::Type(x) = a {
                            peel_groups(x);
                        }
                    }
                }
            }
        }
        _ => {}
    }
}

/// SOUNDNESS R1177 / R1182 — the operator traits whose MACRO-MADE impls are spliced: all of `OPERATOR_TRAITS`, the
/// compound-assignment family included. R1177's first cut spliced that family and the new unit made a hand-written
/// `Type::sub_assign` key ambiguous, trading 47 edges (bigdecimal, arrow-buffer) for an `ambiguous:` reason, so it was
/// left unspliced and a macro-made `impl BitOrAssign for M` had no unit: `a |= M(1)` read ABSENT over its write, and a
/// chained consumer's join found no published key (R1182, EXECUTED, rustagent-v061/fx/v061). R1180's `mbe_side` removes
/// the trade at its root for every operator family: a macro-made unit never contests a hand-written claimant's tail.
fn spliced_operator_trait(leaf: &str) -> bool {
    crate::lang::OPERATOR_TRAITS.contains(&leaf)
}

/// SOUNDNESS R1146 — a macro-generated impl of a std OPERATOR trait (`lang::OPERATOR_TRAITS`, by the trait path's
/// written last segment: `impl Rem for $t`, `impl<'a> Rem<&'a $t> for &'a $t`). Spliced like an inherent impl: the
/// trait is std's, so no LOCAL dispatch universe (the bounded CHA, SPEC §4) holds it and splicing moves no fan-out —
/// the R1004 refusal above is about local traits. Unspliced, the impl had no unit: `a % b` over a macro-made `Rem`
/// read ABSENT in the defining crate, and its report published neither `M::rem` nor `&M::rem`, so a chained
/// consumer's join found nothing and coverage deleted the `invisible` disclosure (auditagent-v051/fxchain2, EXECUTED).
///
/// Only a NOMINAL self type (through references) that is not one of the impl's own parameters: a macro-made
/// `impl PartialEq<Value> for u64` (serde_json) would put the primitive in the operator index, and every `x != 0` over a
/// `u64` would then edge to it although the orphan rule means only a `Value` right operand can run it — those stay
/// unspliced, exactly as before.
/// SOUNDNESS R1157 — a macro-made std OPERATOR impl whose self type (through references and invisible groups) is a
/// PRIMITIVE (`impl Add<D> for $t` with `$t = u32`). Spliced like `operator_trait_impl`'s nominal case, and marked
/// (`model::MBE_PRIM_ATTR`): the orphan rule means its `Rhs` names a crate-local type, so a LITERAL right operand can
/// never run it, and the operator site refuses exactly that syntactic case — `x != 0` over a `u64` beside serde_json's
/// macro-made `impl PartialEq<Value> for u64` forms no edge, while any other right operand does (no name-typing claim
/// withholds the edge). Unspliced, the impl had no unit and `x + d` read ABSENT over its executed write.
fn prim_operator_impl(i: &syn::ItemImpl) -> bool {
    let op = matches!(&i.trait_, Some((None, p, _))
        if p.segments.last().is_some_and(|s| spliced_operator_trait(&s.ident.to_string())));
    let mut t: &syn::Type = &i.self_ty;
    loop {
        match t {
            syn::Type::Reference(r) => t = &r.elem,
            syn::Type::Group(g) => t = &g.elem,
            syn::Type::Paren(p) => t = &p.elem,
            _ => break,
        }
    }
    op && matches!(t, syn::Type::Path(p) if p.qself.is_none() && p.path.get_ident().is_some_and(|id| {
        let s = id.to_string();
        crate::model::PRIMITIVE_LEAVES.contains(&s.as_str())
    }))
}

fn operator_trait_impl(i: &syn::ItemImpl) -> bool {
    let op = matches!(&i.trait_, Some((None, p, _))
        if p.segments.last().is_some_and(|s| spliced_operator_trait(&s.ident.to_string())));
    let mut t: &syn::Type = &i.self_ty;
    loop {
        match t {
            syn::Type::Reference(r) => t = &r.elem,
            syn::Type::Group(g) => t = &g.elem,
            syn::Type::Paren(p) => t = &p.elem,
            _ => break,
        }
    }
    let nominal = match t {
        syn::Type::Path(p) if p.qself.is_none() => {
            let leaf = p.path.segments.last().map(|s| s.ident.to_string()).unwrap_or_default();
            !crate::lang::is_non_nominal_type(t)
                && !(p.path.segments.len() == 1 && i.generics.type_params().any(|tp| tp.ident == leaf))
        }
        _ => false,
    };
    op && nominal
}

/// SOUNDNESS R1605 — THE DECLARED OUTPUT OF A KNOWN DEPENDENCY DERIVE. A derive's expansion is opaque to this engine, so a
/// type it generates was never declared here: with `#[derive(DeriveEntityModel)] #[sea_orm(table_name = "..")] struct Model`
/// in a module, `Entity::find().all(db)` named a type no item declares, the value `find()` returns stayed untyped, and the
/// `SELECT` it runs reached no rule — the caller ABSENT from `functions[]` with no `invisible` (EXECUTED, rust-fab `fxso`).
///
/// This splices only what the derive DECLARES, read off its generator in `sea-orm-macros` (`derives/entity_model.rs`
/// emits `#vis struct Entity;` with `#[derive(DeriveEntity)]` when `table_name` is set; `derives/entity.rs` emits
/// `impl sea_orm::entity::EntityTrait for #ident` with associated TYPES only — no method is overridden). The splice is
/// that skeleton and nothing else: a unit struct and an EMPTY trait impl. Every signature a call on it needs comes from
/// the trait's own declaration in the lock-pinned dependency source (`scan.rs`, `r1605_seed`), never from this table, and
/// effects still come only from the classifier. If the derive's real output differs (a later version that declares more,
/// or an inherent method shadowing a trait member), the skeleton under-declares: a member it does not know is untyped
/// exactly as before, and the disclosure at the `<untyped>` marker speaks for it.
///
/// Gated on the derive being sea_orm's: written `sea_orm::…::DeriveEntityModel`, or bare in a module whose `use` items bring
/// `sea_orm` names in (a glob of a `sea_orm` path, or the derive by name). Not spliced where the module already declares
/// an item named `Entity` (the derive would not compile), or for a generic `DeriveEntity` type.
pub(crate) fn splice_sea_orm_entities(items: &mut Vec<syn::Item>) {
    if candor_report::env::switch("CANDOR_R1605_OFF") {
        return;
    }
    fn use_brings_sea_orm(t: &syn::UseTree, under: bool) -> bool {
        match t {
            syn::UseTree::Path(p) => use_brings_sea_orm(&p.tree, under || p.ident == "sea_orm"),
            syn::UseTree::Glob(_) => under,
            syn::UseTree::Name(n) => under && (n.ident == "DeriveEntityModel" || n.ident == "DeriveEntity"),
            syn::UseTree::Rename(r) => under && (r.ident == "DeriveEntityModel" || r.ident == "DeriveEntity"),
            syn::UseTree::Group(g) => g.items.iter().any(|i| use_brings_sea_orm(i, under)),
        }
    }
    let scoped = items.iter().any(|it| matches!(it, syn::Item::Use(u) if use_brings_sea_orm(&u.tree, false)));
    let declares = |items: &[syn::Item], name: &str| {
        items.iter().any(|it| match it {
            syn::Item::Struct(s) => s.ident == name,
            syn::Item::Enum(e) => e.ident == name,
            syn::Item::Type(t) => t.ident == name,
            syn::Item::Union(u) => u.ident == name,
            _ => false,
        })
    };
    let has_entity = declares(items, "Entity");
    let has_active_model = declares(items, "ActiveModel");
    let mut active_added = false;
    let mut adds: Vec<syn::Item> = Vec::new();
    let mut entity_added = false;
    for it in items.iter_mut() {
        let st = match it {
            syn::Item::Mod(md) => {
                if let Some((_, inner)) = &mut md.content {
                    splice_sea_orm_entities(inner);
                }
                continue;
            }
            syn::Item::Struct(st) => st,
            _ => continue,
        };
        let mut kinds: Vec<&'static str> = Vec::new();
        for a in st.attrs.iter().filter(|a| a.path().is_ident("derive")) {
            let Ok(paths) = a.parse_args_with(syn::punctuated::Punctuated::<syn::Path, syn::Token![,]>::parse_terminated)
            else { continue };
            for p in paths {
                let Some(leaf) = p.segments.last().map(|s| s.ident.to_string()) else { continue };
                let ours = if p.segments.len() > 1 {
                    p.segments.first().is_some_and(|s| s.ident == "sea_orm")
                } else {
                    scoped
                };
                match leaf.as_str() {
                    "DeriveEntityModel" if ours => kinds.push("model"),
                    "DeriveEntity" if ours => kinds.push("entity"),
                    _ => {}
                }
            }
        }
        let table_name = st.attrs.iter().any(|a| {
            a.path().is_ident("sea_orm") && a.meta.require_list().is_ok_and(|l| l.tokens.to_string().contains("table_name"))
        });
        let mut srcs: Vec<String> = Vec::new();
        if kinds.contains(&"model") && table_name && !entity_added && !has_entity {
            srcs.push("pub struct Entity;".to_string());
            srcs.push("impl sea_orm::entity::EntityTrait for Entity {}".to_string());
            entity_added = true;
        }
        // SOUNDNESS R1607 — `DeriveEntityModel` also expands `DeriveModel` on the struct and `DeriveActiveModel` beside it
        // (sea-orm-macros `lib.rs` `derive_entity_model`): `impl sea_orm::ModelTrait for Model` and `struct ActiveModel` with
        // `impl sea_orm::ActiveModelTrait for ActiveModel` (`derives/model.rs`, `derives/active_model.rs`). The derive's
        // overrides are field accessors with no effect and are NOT spliced (a member stub would publish an interface-union
        // row per member for every entity): a call to one goes to the classifier under the trait's spelling (R1607), which
        // names only the members that run a statement.
        if kinds.contains(&"model") && st.ident == "Model" && st.generics.params.is_empty() && !candor_report::env::switch("CANDOR_R1607_OFF") {
            srcs.push("impl sea_orm::ModelTrait for Model {}".to_string());
            if !has_active_model && !active_added {
                srcs.push("pub struct ActiveModel;".to_string());
                srcs.push("impl sea_orm::ActiveModelTrait for ActiveModel {}".to_string());
                active_added = true;
            }
        }
        if kinds.contains(&"entity") && st.generics.params.is_empty() {
            srcs.push(format!("impl sea_orm::entity::EntityTrait for {} {{}}", st.ident));
        }
        for src in srcs {
            let Ok(ts) = src.parse::<TokenStream>() else { continue };
            let Ok(item) = syn::parse2::<syn::Item>(respan_to(ts, st.ident.span())) else { continue };
            if candor_report::env::trace("CANDOR_R1605_INSTR") {
                eprintln!("R1605SPLICE\t{}\t{src}", st.ident); // §E1 REACH COUNTER
            }
            adds.push(item);
        }
    }
    items.extend(adds);
}

/// SOUNDNESS R1146 — a `#[derive(PartialEq)]` / `#[derive(PartialOrd)]` has no body in the source, so the crate's
/// report published no `Type::eq` / `Type::partial_cmp`: inside the crate the comparison reaches the fields through
/// the derive route (`CallCollector::charge_coercion_ty_at`, R1111), but a CHAINED consumer joins the dependency's
/// report by that key, found nothing, and coverage deleted the `invisible` disclosure — `a == b` over a derived
/// `cdep::Outer` whose field's `eq` writes read ABSENT (auditagent-v051/fxchain2 `outer_eq`/`ko_lt`, EXECUTED).
///
/// Each such type gets an INHERENT impl holding one method of that name whose body is the comparison itself
/// (`::core::cmp::PartialEq::eq(self, other)`), so the unit's edges are exactly the derive route's, it is keyed and
/// published as `Type::eq`, and — being inherent — it joins no dispatch universe and does not enter `trait_impls`
/// (the derive route keeps running at every operator site). Not synthesised where the module already declares an
/// inherent method of that name on the type (two `Type::eq` definitions would make each call ambiguous).
pub(crate) fn splice_derived_comparisons(items: &mut Vec<syn::Item>) {
    fn derives(attrs: &[syn::Attribute]) -> Vec<&'static str> {
        let mut out = Vec::new();
        for a in attrs.iter().filter(|a| a.path().is_ident("derive")) {
            let _ = a.parse_nested_meta(|m| {
                match m.path.segments.last().map(|s| s.ident.to_string()).as_deref() {
                    Some("PartialEq") => out.push("PartialEq"),
                    Some("PartialOrd") => out.push("PartialOrd"),
                    _ => {}
                }
                Ok(())
            });
        }
        out
    }
    let inherent: std::collections::HashSet<(String, String)> = items
        .iter()
        .filter_map(|it| match it {
            syn::Item::Impl(i) if i.trait_.is_none() => crate::lang::impl_type_name(&i.self_ty).map(|t| (t, i)),
            _ => None,
        })
        .flat_map(|(t, i)| {
            i.items.iter().filter_map(move |ii| match ii {
                syn::ImplItem::Fn(f) => Some((t.clone(), f.sig.ident.to_string())),
                _ => None,
            })
        })
        .collect();
    let mut out: Vec<syn::Item> = Vec::with_capacity(items.len());
    for mut it in std::mem::take(items) {
        let (attrs, ident, generics) = match &mut it {
            syn::Item::Mod(md) => {
                if let Some((_, inner)) = &mut md.content {
                    splice_derived_comparisons(inner);
                }
                out.push(it);
                continue;
            }
            syn::Item::Struct(st) => (st.attrs.clone(), st.ident.clone(), st.generics.clone()),
            syn::Item::Enum(en) => (en.attrs.clone(), en.ident.clone(), en.generics.clone()),
            _ => {
                out.push(it);
                continue;
            }
        };
        let ds = derives(&attrs);
        out.push(it);
        for d in ds {
            let (m, ret) = if d == "PartialEq" { ("eq", "bool") } else { ("partial_cmp", "::core::option::Option<::core::cmp::Ordering>") };
            if inherent.contains(&(ident.to_string(), m.to_string())) {
                continue;
            }
            let args: Vec<String> = generics
                .params
                .iter()
                .map(|g| match g {
                    syn::GenericParam::Type(t) => t.ident.to_string(),
                    syn::GenericParam::Lifetime(l) => l.lifetime.to_string(),
                    syn::GenericParam::Const(c) => c.ident.to_string(),
                })
                .collect();
            let self_ty = if args.is_empty() { ident.to_string() } else { format!("{ident}<{}>", args.join(", ")) };
            // The body is the comparison itself, written as the operator: `*self == *other` reaches the fields through
            // the derive route exactly as `a == b` does, and names no path (a `::core::cmp::..` spelling would be
            // read as a call into a crate and disclose it).
            let body = if d == "PartialEq" { "let _ = *self == *other; true" } else { "let _ = *self < *other; None" };
            let src = format!("impl {self_ty} {{ fn {m}(&self, other: &Self) -> {ret} {{ {body} }} }}");
            let Ok(ts) = src.parse::<TokenStream>() else { continue };
            let Ok(mut im) = syn::parse2::<syn::ItemImpl>(respan_to(ts, ident.span())) else { continue };
            im.generics = generics.clone();
            // The type's own `#[cfg]`s govern the impl, as the derive's expansion would.
            im.attrs = attrs.iter().filter(|a| a.path().is_ident("cfg")).cloned().collect();
            // The marker `crate::model::is_derive_synth` reads: Pass A records nothing for this impl, and its unit is
            // kept out of in-crate resolution (published only; see `FnInfo::derive_synth`).
            if let Ok(marker) = syn::parse_str::<syn::ItemImpl>(&format!("#[{}] impl X {{}}", crate::model::DERIVE_SYNTH_ATTR)) {
                im.attrs.extend(marker.attrs);
            }
            if candor_report::env::trace("CANDOR_R1146_INSTR") {
                eprintln!("R1146DERIVE\t{ident}\t{m}"); // §E1 REACH COUNTER
            }
            out.push(syn::Item::Impl(im));
        }
    }
    *items = out;
}

thread_local! {
    /// SOUNDNESS R1748 (a) — set by `rep_tail` when it reads a two-token separator (reach counter only).
    static SEP2_USED: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// SOUNDNESS R1748 (a) — does this matcher hold a repetition with a two-token separator?
fn uses_sep2(ms: &[M]) -> bool {
    ms.iter().any(|m| match m {
        M::Rep(inner, sep, _) => sep.as_ref().is_some_and(|s| s.len() > 1) || uses_sep2(inner),
        M::Group(_, inner) => uses_sep2(inner),
        _ => false,
    })
}

/// SOUNDNESS R1748 (a) — the first token of the inert marker `splice` leaves for a trait a local macro declares.
const MBE_TRAIT_MARK: &str = "__candor_mbe_trait";

/// SOUNDNESS R1748 (a) — `Item::Verbatim(__candor_mbe_trait <Trait> <fn member>*)` for a trait an expansion declares.
/// `None` under `CANDOR_R1748A_OFF=1`, which leaves the index empty and every key as it was.
fn mbe_trait_marker(t: &syn::ItemTrait) -> Option<syn::Item> {
    if candor_report::env::switch("CANDOR_R1748A_OFF") {
        return None;
    }
    let mut s = format!("{MBE_TRAIT_MARK} {}", t.ident);
    if crate::lang::reach_debug() {
        eprintln!("R1748MBETRAIT\t{}", t.ident); // §E1 REACH COUNTER
    }
    for ti in &t.items {
        if let syn::TraitItem::Fn(f) = ti {
            s.push(' ');
            s.push_str(&f.sig.ident.to_string());
        }
    }
    s.parse::<TokenStream>().ok().map(syn::Item::Verbatim)
}

/// SOUNDNESS R1754 — the marker ident of a generated trait's BODY marker (see `mbe_trait_body_marker`).
const MBE_TRAIT_BODY_MARK: &str = "__candor_mbe_trait_body";

/// SOUNDNESS R1754 — an inert `Item::Verbatim` carrying a macro-generated trait WHOLE (with the invocation's outer
/// attributes), emitted only where the trait has at least one PROVIDED body. No decl walk reads a `Verbatim` item; the unit
/// walk (`decls::scan_items`) and its loc mirror (`decls::fn_locs`) decode it with `mbe_trait_body_of` and treat it as the
/// trait it carries, so each default body is a unit (`de::Error::missing_field`) and nothing else about the trait is
/// indexed. The trait's tokens keep their spans, so the units' locs are the macro definition's lines.
/// `CANDOR_R1754D_OFF=1` disables it.
fn mbe_trait_body_marker(t: &syn::ItemTrait, attrs: &[syn::Attribute]) -> Option<syn::Item> {
    if candor_report::env::switch("CANDOR_R1754D_OFF")
        || !t.items.iter().any(|ti| matches!(ti, syn::TraitItem::Fn(f) if f.default.is_some()))
    {
        return None;
    }
    let mut t = t.clone();
    let mut na = attrs.to_vec();
    na.append(&mut t.attrs);
    t.attrs = na;
    if crate::lang::reach_debug() || candor_report::env::trace("CANDOR_R1754_INSTR") {
        eprintln!("R1754MBEBODY\t{}", t.ident); // §E1 REACH COUNTER
    }
    let mut ts = TokenStream::new();
    ts.extend([TokenTree::Ident(Ident::new(MBE_TRAIT_BODY_MARK, proc_macro2::Span::call_site()))]);
    ts.extend(syn::__private::ToTokens::to_token_stream(&t));
    Some(syn::Item::Verbatim(ts))
}

/// SOUNDNESS R1754 — read a marker `mbe_trait_body_marker` wrote back into the trait it carries.
pub(crate) fn mbe_trait_body_of(it: &syn::Item) -> Option<syn::ItemTrait> {
    let syn::Item::Verbatim(ts) = it else { return None };
    let mut it = ts.clone().into_iter();
    match it.next()? {
        TokenTree::Ident(i) if i == MBE_TRAIT_BODY_MARK => {}
        _ => return None,
    }
    syn::parse2::<syn::ItemTrait>(it.collect()).ok()
}

/// SOUNDNESS R1748 (a) — read a marker `mbe_trait_marker` wrote: `(trait ident, fn members)`.
pub(crate) fn mbe_trait_of(it: &syn::Item) -> Option<(String, Vec<String>)> {
    let syn::Item::Verbatim(ts) = it else { return None };
    let mut ids = ts.clone().into_iter().map(|t| match t {
        TokenTree::Ident(i) => Some(i.to_string()),
        _ => None,
    });
    if ids.next()?.as_deref() != Some(MBE_TRAIT_MARK) {
        return None;
    }
    let name = ids.next()??;
    let members: Option<Vec<String>> = ids.collect();
    Some((name, members?))
}

/// Splice, beside every item-position invocation of a `macro_rules!` this file defines exactly once, the
/// free functions and `extern` blocks it expands to. See the module doc for what is refused.
pub(crate) fn splice_local_macros(items: &mut Vec<syn::Item>, xdefs: &CrateMacros) {
    splice_local_macros_with(items, xdefs, &ArmClaims::default());
}

/// `splice_local_macros`, with the cross-file cfg-twin claims this file consumes (`ArmClaims`, cfg-twin lane A).
pub(crate) fn splice_local_macros_with(items: &mut Vec<syn::Item>, xdefs: &CrateMacros, claims: &ArmClaims) {
    splice_local_macros_scoped(items, xdefs, claims, None, None);
}

/// `splice_local_macros_with`, with the file's R-ROOT scope (`rroot::Scope`): a root-headed macro path binds there.
pub(crate) fn splice_local_macros_scoped(
    items: &mut Vec<syn::Item>,
    xdefs: &CrateMacros,
    claims: &ArmClaims,
    f1: Option<crate::rroot::Scope>,
    tc: Option<crate::baretc::TcFile>,
) {
    let mut imported = std::collections::HashSet::new();
    if !candor_report::env::switch("CANDOR_I1_BIND_OFF") {
        imported_names(items, &mut imported);
    }
    let mut defs = HashMap::new();
    collect_defs(items, &imported, &mut defs);
    // R1156 — a name defined more than once (`None`) may still be expanded from its textual scope.
    // I1 stage 3 (R1813) — a file whose only definitions are BODY-local still has body-position invocations to read.
    if defs.is_empty() && xdefs.iter().all(|(k, d)| d.is_none() || k.starts_with(ALL_BODIES_KEY)) && body_off() && claims.is_empty() {
        return;
    }
    let mut bodies: HashMap<String, Vec<TokenStream>> = HashMap::new();
    fn all_bodies(items: &[syn::Item], out: &mut HashMap<String, Vec<TokenStream>>) {
        for it in items {
            match it {
                syn::Item::Macro(m) if m.mac.path.is_ident("macro_rules") => {
                    if let Some(n) = &m.ident {
                        out.entry(n.to_string()).or_default().push(m.mac.tokens.clone());
                    }
                }
                syn::Item::Mod(md) => {
                    if let Some((_, inner)) = &md.content {
                        all_bodies(inner, out);
                    }
                }
                _ => {}
            }
        }
    }
    all_bodies(items, &mut bodies);
    // SOUNDNESS R1899 — which toolchain macro names a BARE invocation in this file may not bind to the toolchain.
    let tc_locals: std::collections::HashSet<String> = TOOLCHAIN_BANG_MACROS
        .iter()
        .filter(|n| bodies.contains_key(**n) || xdefs.get(**n).is_some() || xdefs.get(&emitted_key(n)).is_some())
        .map(|n| n.to_string())
        .collect();
    let tc_mu = tc.is_some() && has_macro_use_extern(items);
    let mut cx = Ctx {
        arms: HashMap::new(),
        defs,
        xdefs,
        budget: MAX_EXPANSIONS,
        scope: HashMap::new(),
        imported,
        partial: Default::default(),
        bodies,
        scope_bodies: HashMap::new(),
        emitted: Default::default(),
        tc_shadow: Default::default(),
        tc_file: Default::default(),
        tc,
        tc_mu,
        tc_locals,
        claims: claims.decode(),
        modpath: Vec::new(),
        f1,
        f1_gated: 0,
    };
    // SOUNDNESS R1899 / R-BARE-TC — the file-wide reading (the module-arm readers' and the splice's starting point)
    cx.tc_file = cx.tc_names_for(items, None);
    cx.tc_shadow.replace(cx.tc_file.clone());
    cx.splice(items, 0);
    // SOUNDNESS R1862 / R1824 (cfg-twin lane A) — THIS FILE AS AN OUT-OF-LINE MODULE ARM. Its own hidden text, read whole
    // (the same `read_arm_hidden` an inline arm is read with), is exported for the sibling arms in other files
    // (`FILE_READ_MARK` → `cache::FileDecls::i1_arms`); read BEFORE this file's own units are marked, so a claim never
    // depends on another claim. Then the sibling arms' claims, joined crate-wide by `arm_claims`, mark this file's units
    // exactly as an inline arm's are (`cover_arm_keyed`, hand-written: no binding is removed).
    if !r1857_off() && cx.holds_arm_text(items) {
        let mut h = Hidden::default();
        cx.read_arm_hidden(items, &mut h);
        if !h.is_empty() {
            items.push(read_marker(FILE_READ_MARK, &h));
        }
    }
    if let Some(h) = cx.claims.as_arm.as_ref().filter(|h| !h.is_empty()) {
        if candor_report::env::trace("CANDOR_I1_INSTR") {
            eprintln!("CFGTWINFILE\t{}", if h.opaque { "opaque" } else { "keyed" }); // §E1 REACH COUNTER, ADDED branch only
        }
        cover_arm_keyed(items, h, true);
    }
}


/// I1 stage 2b FLIP — the opt-OUT of the cross-file `#[cfg]` lift (result-moving: a scan run under it records the switch and its
/// gates refuse, as an opted-in scan's did while the lift was opt-in).
/// I1 stage 3 (R1784) — the opt-out of macro-defined definitions. `CANDOR_I1S3_BODY_OFF` implies it: a definer that expands
/// fully no longer stands as hidden text, and the R452 / R128 cover a body-position invocation of what it defines read through
/// (`m_binh`, `Unknown` on 0.42.0) is then answered only by the body hoist. Measured: MDEF on with the hoist off read ABSENT.
pub(crate) fn mdef_off() -> bool {
    static OFF: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *OFF.get_or_init(|| candor_report::env::switch("CANDOR_I1S3_MDEF_OFF") || body_off() || trait_item_off())
}

pub(crate) fn xcfg_off() -> bool {
    static OFF: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *OFF.get_or_init(|| candor_report::env::switch("CANDOR_I1S2_XCFG_OFF"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn exp(def: &str, call: &str) -> Option<String> {
        let f: syn::File = syn::parse_str(&format!("{def}\n{call}")).unwrap();
        let mut items = f.items;
        splice_local_macros(&mut items, &CrateMacros::new());
        let names: Vec<String> = items
            .iter()
            .filter_map(|i| match i {
                syn::Item::Fn(f) => Some(f.sig.ident.to_string()),
                _ => None,
            })
            .collect();
        Some(if names.is_empty() { "<none>".to_string() } else { names.join(",") })
    }

    /// SOUNDNESS R1748 (a) — a two-character separator (`::`, serde_core's `$($supertrait:ident)::+`) is one rustc token,
    /// and the traits an expansion declares are recorded (not spliced) by an inert marker.
    #[test]
    fn a_path_separator_repetition_expands_and_its_trait_is_recorded() {
        let def = "macro_rules! d { (Error: Sized $(+ $($s:ident)::+)*) => { pub trait Error: Sized $(+ $($s)::+)* { \
                   fn custom(m: u8) -> Self; fn invalid_type(u: u8) -> Self { Error::custom(u) } } fn made() {} }; }";
        assert_eq!(exp(def, "d!(Error: Sized + core::fmt::Display);").as_deref(), Some("made"));
        let f: syn::File = syn::parse_str(&format!("{def}\nd!(Error: Sized + Display);")).unwrap();
        let mut items = f.items;
        splice_local_macros(&mut items, &CrateMacros::new());
        let traits: Vec<(String, Vec<String>)> = items.iter().filter_map(mbe_trait_of).collect();
        assert_eq!(traits, vec![("Error".to_string(), vec!["custom".to_string(), "invalid_type".to_string()])]);
        assert!(!items.iter().any(|i| matches!(i, syn::Item::Trait(_))), "the trait itself is not spliced");
        let p = "macro_rules! p { ($($s:ident)::+ => $f:ident) => { fn $f() {} }; }";
        assert_eq!(exp(p, "p!(a::b::c => f1);").as_deref(), Some("f1"));
        assert_eq!(exp(p, "p!(a => f2);").as_deref(), Some("f2"));
        assert_eq!(exp(p, "p!(a::b:c => f3);").as_deref(), Some("<none>"), "a broken separator matches nothing");
        let c = "macro_rules! c { ($($s:ident),*) => { $(fn $s() {})* }; }";
        assert_eq!(exp(c, "c!(x, y);").as_deref(), Some("x,y"), "a one-token separator still reads");
    }

    #[test]
    fn first_matching_arm_wins_and_repetitions_expand() {
        let def = "macro_rules! m { (one $n:ident) => { fn $n() {} }; ($($n:ident),* $(,)?) => { $(fn $n() {})* }; }";
        assert_eq!(exp(def, "m!(one a);").as_deref(), Some("a"));
        assert_eq!(exp(def, "m!(b, c, d,);").as_deref(), Some("b,c,d"));
        assert_eq!(exp(def, "m!(b c);").as_deref(), Some("<none>"), "no arm matches — nothing added");
    }

    #[test]
    fn a_twice_defined_macro_is_not_expanded() {
        let def = "#[cfg(unix)] macro_rules! m { ($n:ident) => { fn $n() {} } }\n#[cfg(windows)] macro_rules! m { ($n:ident) => { fn other() {} } }";
        assert_eq!(exp(def, "m!(a);").as_deref(), Some("<none>"));
    }

    #[test]
    fn typed_fragments_and_nested_repetition() {
        let def = "macro_rules! ex { ($(#[$a:meta])* extern \"C\" { $(fn $n:ident($($args:tt)*) -> $r:ty;)* }) => { $( unsafe extern \"C\" fn $n($($args)*) -> $r { panic!() } )* } }";
        assert_eq!(exp(def, "ex! { extern \"C\" { fn f(v: u32) -> (); fn g() -> u8; } }").as_deref(), Some("f,g"));
    }

    /// Fable stage-2 review, finding (c): candor's OWN marker attributes are not attribute macros, so a crate whose only
    /// "attribute" is one of them holds no R828 hedge; a real attribute macro beside them still does.
    #[test]
    fn candor_marker_attributes_are_not_a_hidden_impl_surface() {
        for m in [
            "candor_mbe_op",
            "candor_mbe_prim_op",
            "candor_derive_synth",
            "candor_i1_hidden_twin",
            crate::model::MBE_AMBIG_ATTR,
            crate::model::MBE_EMITTED_DEF_ATTR,
            crate::model::MBE_TWIN_DEF_ATTR,
            crate::model::MBE_TWIN_HAND_DEF_ATTR,
        ] {
            let f: syn::File = syn::parse_str(&format!("#[{m}] impl X {{ #[{m}] fn f(&self) {{}} }} pub struct X;")).unwrap();
            assert!(!super::has_hidden_impl_surface(&f.items), "{m} read as an attribute macro");
        }
        let f: syn::File = syn::parse_str("#[candor_mbe_op] impl X {} #[other::attr] pub struct X;").unwrap();
        assert!(super::has_hidden_impl_surface(&f.items), "a real attribute macro still counts");
    }
}
