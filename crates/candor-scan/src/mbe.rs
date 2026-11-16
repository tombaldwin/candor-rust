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
                    let e = defs.entry(name.to_string()).or_insert(Some(m.mac.tokens.clone()));
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
                        && (imported.contains(&name.to_string()) || imported.contains("*"))
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

/// The macro bodies `Ctx::read_hidden` may expand with: this file's (`Ctx::bodies`) and the crate's (`CrateMacros`, all bodies under
/// `ALL_BODIES_KEY`). A plain lookup rather than a closure, so the reading makes no indirect call.
#[derive(Default)]
struct BodyTable<'a> {
    bodies: Option<&'a HashMap<String, Vec<TokenStream>>>,
    xdefs: Option<&'a CrateMacros>,
}

impl BodyTable<'_> {
    /// `None`: not a local macro; `Some(None)`: local, but no body readable; `Some(Some(bodies))`.
    fn get(&self, n: &str) -> Option<Option<Vec<TokenStream>>> {
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
        let table = BodyTable { bodies: Some(&self.bodies), xdefs: Some(self.xdefs) };
        // The toolchain's own item macros define no name of the crate's but what they DECLARE: `thread_local! { static X .. }`
        // binds `X`; `compile_error!`, `global_asm!` and the rest bind nothing. A local definition shadowing the name is read below.
        let root_tc = mac.path.segments.len() == 1
            || matches!(mac.path.segments.first().map(|s| s.ident.to_string()).as_deref(), Some("std" | "core" | "alloc"));
        if root_tc && TOOLCHAIN_BANG_MACROS.contains(&name.as_str()) && table.get(&name).is_none() {
            declared_names(&mac.tokens, &mut h.loose);
            return;
        }
        let bodies = match table.get(&name) {
            None => {
                // Another crate's macro: its expansion is not here. Where what it is passed parses as items (pin_project's
                // struct, `loom::thread_local! { static X .. }`), the names those items declare, plus every identifier of
                // their attributes (`#[project = ProjName]` names a type the macro defines); otherwise every identifier passed.
                match syn::parse2::<syn::File>(mac.tokens.clone()) {
                    Ok(f) if !f.items.is_empty() => {
                        for it in &f.items {
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
            all_idents(&mac.tokens, &mut h.loose);
            bodies.iter().for_each(|b| declared_names(b, &mut h.loose));
            h.note("limit", &name);
            return;
        }
        let mut any = false;
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
                    }
                }
            }
        }
        if !any {
            all_idents(&mac.tokens, &mut h.loose);
            bodies.iter().for_each(|b| declared_names(b, &mut h.loose));
            h.note("noexpand", &name);
        }
    }

    fn read_hidden_item(&self, it: &syn::Item, out: &[syn::Item], h: &mut Hidden, depth: usize) {
        match it {
            syn::Item::Macro(m) if m.ident.is_none() => self.read_hidden(&m.mac, None, out, h, depth),
            syn::Item::Impl(im) => {
                let owner = ty_key(&im.self_ty);
                for ii in &im.items {
                    self.read_hidden_member(&owner, ii, out, h, depth);
                }
            }
            _ => {
                let k = item_keys(it, out);
                if let syn::Item::Use(u) = it {
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
                h.keys.insert(format!("{owner}::{}", f.sig.ident));
            }
            syn::ImplItem::Macro(m) => self.read_hidden(&m.mac, Some(owner), out, h, depth),
            _ => {}
        }
    }

    /// R1156 — the definition in textual scope for a name this FILE defines more than once.
    fn scoped_def(&self, name: &str) -> Option<String> {
        if !matches!(self.defs.get(name), Some(None)) {
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
            None if !self.defs.contains_key(name) => {
                self.xdefs.get(name).cloned().flatten().and_then(|s| s.parse::<TokenStream>().ok())
            }
            None => None,
        };
        let a = def.and_then(|t| parse_arms(&t)).map(std::rc::Rc::new);
        self.arms.insert(name.to_string(), a.clone());
        a
    }

    /// The invocation's macro, when it is a bare name this file defines once — or, R1004, a name this
    /// file does not define at all and the CRATE defines once (identically everywhere it is defined).
    fn local_name(&self, mac: &syn::Macro) -> Option<String> {
        let id = mac.path.get_ident()?.to_string();
        if id == "macro_rules" {
            return None;
        }
        // SOUNDNESS R1541 / I1 step 1 — a name this file IMPORTS through a `use` that is not crate-rooted may be another
        // crate's macro (`use bitflags::bitflags;` beside a local wrapper also named `bitflags`, redox_syscall); binding it
        // to the local definition by name would mint the WRONG items. Refused: the invocation stays, and R128 discloses it.
        let bound = match self.defs.get(&id) {
            Some(d) => d.is_some() || self.scoped_def(&id).is_some(),
            None => self.xdefs.get(&id).is_some_and(|d| d.is_some()),
        };
        if bound && self.imported.contains(&id) {
            if candor_report::env::trace("CANDOR_I1_INSTR") {
                eprintln!("I1REFUSE\timported\t{id}"); // §E1 REACH COUNTER
            }
            return None;
        }
        bound.then_some(id)
    }

    /// The invocation's expansion tokens, and whether its definition came from ANOTHER file.
    fn expand_tokens(&mut self, mac: &syn::Macro, depth: usize) -> Option<(TokenStream, bool)> {
        if depth >= MAX_DEPTH || self.budget == 0 {
            return None;
        }
        let name = self.local_name(mac)?;
        let cross_file = !self.defs.contains_key(&name);
        let arms = self.arms_of(&name)?;
        SEP2_USED.with(|c| c.set(false));
        let ts = expand(&arms, &mac.tokens)?;
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
        // I1 stage 2 — LIFTED. The refusal existed because the splice dropped `use`s: with modules and `use`s spliced, the
        // twin arm (`cfg_taskdump! { use …::trace_leaf; }`) stands beside the stub and the union is whole, exactly as for a
        // hand-written pair. palette's `impl_eq!` (`#[cfg(feature = "approx")]` on what it stamps) was the measured cost of
        // keeping it. `CANDOR_I1S2_XCFG_OFF=1` refuses again.
        let xcfg = cross_file
            && file.items.iter().any(|it| {
                crate::lang::item_attrs(it).iter().any(|a| a.path().is_ident("cfg") || a.path().is_ident("cfg_attr"))
            });
        if xcfg && candor_report::env::switch("CANDOR_I1S2_XCFG_OFF") {
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
        Some(file.items)
    }

    /// SOUNDNESS R1245 — an IMPL-ITEM-position invocation (`impl Report for Reports { reports_impl!(fn
    /// summarize(&self)); }`) expanded to the impl items it declares, under the same once-defined rule
    /// as an item-position one. `None` — the invocation stays exactly as written — when the macro is not one
    /// this crate defines once, no arm matches, the expansion is not a sequence of impl items, or (another
    /// file's macro) any generated item carries a `#[cfg]` (see `expand_items`).
    fn expand_impl_items(&mut self, mac: &syn::Macro, depth: usize) -> Option<Vec<syn::ImplItem>> {
        use syn::parse::Parser;
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
        if xcfg && candor_report::env::switch("CANDOR_I1S2_XCFG_OFF") {
            return None; // I1 stage 2 — lifted, as for item position
        }
        if xcfg && candor_report::env::trace("CANDOR_I1_INSTR") {
            eprintln!("I1S2XCFG\timpl-item"); // §E1 REACH COUNTER, on the CHANGED branch only
        }
        self.budget -= 1;
        self.splice_impl_items(&mut gen, depth + 1);
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
                if let Some(mut gen) = self.expand_impl_items(&m.mac, depth) {
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

    fn splice(&mut self, items: &mut Vec<syn::Item>, depth: usize) {
        let mut out: Vec<syn::Item> = Vec::with_capacity(items.len());
        // I1 stage 2 — which items of `out` the splice PRODUCED (`mark_hidden_twins`), index for index.
        let mut spliced: Vec<bool> = Vec::with_capacity(items.len());
        for mut it in std::mem::take(items) {
            if let syn::Item::Macro(m) = &it {
                if let (Some(name), true) = (&m.ident, m.mac.path.is_ident("macro_rules")) {
                    let gated = crate::lang::has_cfg(&m.attrs);
                    self.scope.insert(name.to_string(), (!gated).then(|| m.mac.tokens.to_string()));
                }
            }
            match &mut it {
                syn::Item::Macro(m) if m.ident.is_none() && !seed_dropped(&m.mac) => {
                    if let Some(gen) = self.expand_items(&m.mac, depth) {
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
                        // the invocation, as rustc places it, EXCEPT the refusal list (`i1_refused`): a nested invocation or
                        // `macro_rules!` this pass could not expand (R1784), and traits (the R1748/R1754 markers above stand
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
                        let full = gen.iter().all(|g| !i1_refused(g) || matches!(g, syn::Item::Trait(_)));
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
                            let name = m.mac.path.get_ident().map(|i| i.to_string()).unwrap_or_default();
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
                        spliced.resize(out.len(), false);
                        spliced.resize(out.len() + fns.len(), true);
                        out.extend(fns);
                        out.append(&mut markers);
                        continue;
                    }
                }
                // R1245 — a hand-written impl whose members a local macro declares.
                syn::Item::Impl(im) => self.splice_impl_items(&mut im.items, depth),
                syn::Item::Mod(md) => {
                    if let Some((_, inner)) = &mut md.content {
                        // R1156 — a child sees the parent's textual scope; its own definitions leave with it
                        // unless the module is `#[macro_use]`.
                        let saved = self.scope.clone();
                        self.splice(inner, depth);
                        if !md.attrs.iter().any(|a| a.path().is_ident("macro_use")) {
                            self.scope = saved;
                        }
                    }
                }
                _ => {}
            }
            out.push(it);
        }
        if !candor_report::env::switch("CANDOR_I1S2_TWIN_OFF") {
            spliced.resize(out.len(), false);
            let (mut item, mut member) = (Hidden::default(), Hidden::default());
            for it in out.iter() {
                match it {
                    syn::Item::Macro(m) if is_hidden_text(m, &self.partial) => {
                        let mut h = Hidden::default();
                        self.read_hidden(&m.mac, None, &out, &mut h, 0);
                        if candor_report::env::trace("CANDOR_I1_INSTR") {
                            eprintln!("I1S2HIDBY\t{}", invocation_key(&m.mac).split('!').next().unwrap_or("")); // §E1 REACH COUNTER
                        }
                        item.opaque |= h.opaque;
                        item.keys.extend(h.keys);
                        item.loose.extend(h.loose);
                    }
                    syn::Item::Impl(im) => {
                        let owner = ty_key(&im.self_ty);
                        for ii in &im.items {
                            if let syn::ImplItem::Macro(m) = ii {
                                let mut h = Hidden::default();
                                self.read_hidden(&m.mac, Some(&owner), &out, &mut h, 0);
                                member.opaque |= h.opaque;
                                member.keys.extend(h.keys);
                                member.loose.extend(h.loose);
                            }
                        }
                    }
                    _ => {}
                }
            }
            mark_hidden_twins(&mut out, &spliced, &item, &member);
        }
        *items = out;
    }
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
fn is_hidden_text(m: &syn::ItemMacro, partial: &std::collections::HashSet<String>) -> bool {
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
    !(toolchain_path && TOOLCHAIN_BANG_MACROS.contains(&leaf.as_str()))
}

fn is_gated(a: &[syn::Attribute]) -> bool {
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

fn mark_fn(attrs: &mut Vec<syn::Attribute>, n: &str) {
    mark_fn_as(attrs, n, "I1S2MARKFN");
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
        syn::Item::Fn(f) => {
            let n = f.sig.ident.to_string();
            mark_fn(&mut f.attrs, &n);
        }
        syn::Item::Impl(im) => {
            for ii in im.items.iter_mut() {
                if let syn::ImplItem::Fn(f) = ii {
                    let n = f.sig.ident.to_string();
                    mark_fn(&mut f.attrs, &n);
                }
            }
        }
        syn::Item::Use(u) => {
            if candor_report::env::trace("CANDOR_I1_INSTR") {
                eprintln!("I1S2MARKUSE\t{}", quote::ToTokens::to_token_stream(&u.tree)); // §E1 REACH COUNTER, ADDED branch only
            }
            *it = use_as_hidden_text(u);
        }
        syn::Item::Mod(m) => {
            if candor_report::env::trace("CANDOR_I1_INSTR") {
                eprintln!("I1S2MARKMOD\t{}", m.ident); // §E1 REACH COUNTER, on the ADDED branch only
            }
            if let Some((_, inner)) = &mut m.content {
                inner.iter_mut().for_each(cover_arm);
            }
        }
        _ => {}
    }
}

/// SOUNDNESS R1541 / I1 stage 2 — A CFG ARM THE SPLICE PRODUCED, BESIDE TEXT IT DID NOT EXPAND.
///
/// THE EXPOSURE. Stage 2 lifts the cross-file cfg refusal and splices `use`s, so `cfg_not_unix! { .. }`'s stub arm is spliced.
/// If its twin stays UNEXPANDED (its wrapper is defined twice, the budget ran out, no arm matches, the name is imported, it is
/// another crate's macro…), the name RESOLVES to the stub and reads pure over a program that writes. Before stage 2 both
/// invocations were refused, the name resolved to nothing and R128 disclosed `macro:`; the correct lift removed that cover.
///
/// THE MODEL: keyed on PROVENANCE, not on names. Three rounds keyed the cover on the names the hidden text defines (`fn x`,
/// then `use` leaves and `mod x`), and each round a reviewer found a spelling the enumeration missed: a glob arm (which binds
/// every name of its target, so no name can be read off it, in either orientation) and an identifier passed to a nested
/// macro (`cfg_unix! { mk_w!(leaf) }` writes no `fn` token at all). Hidden text cannot be enumerated — that is what makes it
/// hidden. So the cover keys on what the pre-stage-2 build keyed its disclosure on: THIS MODULE STILL HOLDS UNEXPANDED ITEM
/// TEXT. In such a module every cfg-gated item the splice PRODUCED is one arm of a configuration whose other arms may be in
/// that text, and it discloses instead of resolving alone (`cover_arm`). An item the splice did not produce (hand-written) is
/// read as before; an ungated one cannot have a compiled twin (a second definition in one module is a rustc error).
///
/// An impl block that still holds an unexpanded impl-item invocation is the same at member level: its cfg-gated methods are
/// marked. `CANDOR_I1S2_TWIN_OFF=1` drops the cover.
/// What one module's hidden text may define (`Ctx::read_hidden`): `keys` read by `syn` from its expansions (`item_keys`),
/// `loose` from a stated over-approximation (every identifier, where expansion could not finish), or `opaque`.
#[derive(Default)]
struct Hidden {
    opaque: bool,
    keys: std::collections::HashSet<String>,
    loose: std::collections::HashSet<String>,
    steps: usize,
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
            || self.loose.contains(m)
            || (type_twin && (self.keys.contains(owner) || self.loose.contains(owner)))
    }
    fn is_empty(&self) -> bool {
        !self.opaque && self.keys.is_empty() && self.loose.is_empty()
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
fn ty_key(t: &syn::Type) -> String {
    match t {
        syn::Type::Path(p) => p.path.segments.last().map(|s| s.ident.to_string()).unwrap_or_default(),
        syn::Type::Group(g) => ty_key(&g.elem),
        syn::Type::Paren(p) => ty_key(&p.elem),
        other => quote::ToTokens::to_token_stream(other).to_string(),
    }
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
    let Some(mut t) = mbe_trait_body_of(it) else { return };
    let tn = t.ident.to_string();
    let mut hit = false;
    for ti in t.items.iter_mut() {
        if let syn::TraitItem::Fn(f) = ti {
            if f.default.is_some() && (h.covers(&tn) || h.covers_method(&tn, &f.sig.ident.to_string(), true)) {
                let n = format!("{tn}::{}", f.sig.ident);
                mark_fn_as(&mut f.attrs, &n, "I1S2MARKTRAIT");
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
fn mark_hidden_twins(out: &mut [syn::Item], spliced: &[bool], item: &Hidden, member: &Hidden) {
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
    let flags: Vec<bool> = out
        .iter()
        .enumerate()
        .map(|(i, it)| {
            spliced.get(i).copied().unwrap_or(false)
                && is_gated(crate::lang::item_attrs(it))
                && matches!(it, syn::Item::Fn(_) | syn::Item::Use(_) | syn::Item::Mod(_))
                && visible_twin(it, out)
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
        if flags[i] {
            let keep_use = matches!(&*it, syn::Item::Use(u) if use_into(&u.tree, &marked_mods, true));
            if !keep_use {
                cover_arm(it);
            }
            continue;
        }
        // a trait-body marker exists only for a trait an expansion declared (`mbe_trait_body_marker`): produced by construction
        if mbe_trait_body_of(it).is_some() && !item.is_empty() {
            cover_trait_marker(it, item);
            continue;
        }
        if let syn::Item::Impl(im) = it {
            if item.is_empty() && member.is_empty() {
                continue;
            }
            let owner = ty_key(&im.self_ty);
            let block = is_gated(&im.attrs);
            for ii in im.items.iter_mut() {
                if let syn::ImplItem::Fn(f) = ii {
                    let n = f.sig.ident.to_string();
                    if (block || is_gated(&f.attrs)) && both(&owner, &n) {
                        mark_fn_as(&mut f.attrs, &format!("{owner}::{n}"), "I1S2MARKMETH");
                    }
                }
            }
        }
    }
}

/// I1 stage 1 — the explicit REFUSAL list for a spliced expansion (the denylist that replaced the R1004 allowlist).
///
/// I1 stage 2 — modules, `use`s and `extern crate`s are SPLICED: the expansion is the program rustc compiles, so a module a
/// macro makes (`mk!(bool)` → `pub mod bool { pub fn go() .. }`, R1535) is a module, and a `use` it makes binds a name
/// (`sys!(tu)` → `use crate::tu as sys;`, R1688) exactly as the hand-written item does. What stays refused is what this pass
/// cannot stand behind: a nested invocation it could not expand (kept as written, so it remains evidence), a nested
/// `macro_rules!` definition (R1784, stage 3), and a trait (the R1748/R1754 markers stand for it). `CANDOR_I1S2_MODUSE_OFF=1`
/// refuses modules, `use`s and `extern crate`s again (the stage-1 list).
fn i1_refused(g: &syn::Item) -> bool {
    match g {
        syn::Item::Macro(_) | syn::Item::Trait(_) | syn::Item::TraitAlias(_) => true,
        syn::Item::Mod(_) | syn::Item::Use(_) | syn::Item::ExternCrate(_) => {
            candor_report::env::switch("CANDOR_I1S2_MODUSE_OFF")
        }
        _ => false,
    }
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
    "candor_i1_hidden_twin", "candor_mbe_op", "candor_mbe_prim_op", "candor_derive_synth",
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
    let mut imported = std::collections::HashSet::new();
    if !candor_report::env::switch("CANDOR_I1_BIND_OFF") {
        imported_names(items, &mut imported);
    }
    let mut defs = HashMap::new();
    collect_defs(items, &imported, &mut defs);
    // R1156 — a name defined more than once (`None`) may still be expanded from its textual scope.
    if defs.is_empty() && xdefs.iter().all(|(k, d)| d.is_none() || k.starts_with(ALL_BODIES_KEY)) {
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
    let mut cx = Ctx { arms: HashMap::new(), defs, xdefs, budget: MAX_EXPANSIONS, scope: HashMap::new(), imported, partial: Default::default(), bodies };
    cx.splice(items, 0);
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
        for m in ["candor_mbe_op", "candor_mbe_prim_op", "candor_derive_synth", "candor_i1_hidden_twin"] {
            let f: syn::File = syn::parse_str(&format!("#[{m}] impl X {{ #[{m}] fn f(&self) {{}} }} pub struct X;")).unwrap();
            assert!(!super::has_hidden_impl_surface(&f.items), "{m} read as an attribute macro");
        }
        let f: syn::File = syn::parse_str("#[candor_mbe_op] impl X {} #[other::attr] pub struct X;").unwrap();
        assert!(super::has_hidden_impl_surface(&f.items), "a real attribute macro still counts");
    }
}
