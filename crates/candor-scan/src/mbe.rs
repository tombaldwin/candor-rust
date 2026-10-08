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
    Rep(Vec<M>, Option<TokenTree>, RepOp),
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

/// After `$( .. )`: an optional single-token separator, then the operator. Returns (sep, op, tokens used).
fn rep_tail(rest: &[TokenTree]) -> Option<(Option<TokenTree>, RepOp, usize)> {
    let first = rest.first()?;
    if let Some(op) = rep_op(first) {
        // `$(..)?` takes no separator; `$(..)*`/`+` directly.
        return Some((None, op, 1));
    }
    if matches!(first, TokenTree::Group(_)) {
        return None;
    }
    let op = rep_op(rest.get(1)?)?;
    if op == RepOp::Opt {
        return None; // `?` admits no separator
    }
    Some((Some(first.clone()), op, 2))
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
                        if !inp.get(pos).is_some_and(|x| tok_eq(s, x)) {
                            continue;
                        }
                        start = pos + 1;
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
                                out.extend([s.clone()]);
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

/// The file's `macro_rules!` definitions by name — a name defined more than once (a `#[cfg]` twin, a
/// textual redefinition) is dropped: which one an invocation sees depends on order and configuration.
fn collect_defs(items: &[syn::Item], defs: &mut HashMap<String, Option<TokenStream>>) {
    for it in items {
        match it {
            syn::Item::Macro(m) if m.mac.path.is_ident("macro_rules") => {
                if let Some(name) = &m.ident {
                    let e = defs.entry(name.to_string()).or_insert(Some(m.mac.tokens.clone()));
                    if e.as_ref().is_some_and(|prev| prev.to_string() != m.mac.tokens.to_string()) {
                        *e = None;
                    }
                }
            }
            syn::Item::Mod(md) => {
                if let Some((_, inner)) = &md.content {
                    collect_defs(inner, defs);
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
    let mut defs = HashMap::new();
    collect_defs(items, &mut defs);
    defs.into_iter().map(|(k, v)| (k, v.map(|t| t.to_string()))).collect()
}

pub(crate) fn merge_crate_macros(per_file: impl IntoIterator<Item = Vec<(String, Option<String>)>>) -> CrateMacros {
    let mut out: CrateMacros = HashMap::new();
    for defs in per_file {
        for (name, body) in defs {
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
}

impl Ctx<'_> {
    fn arms_of(&mut self, name: &str) -> Option<std::rc::Rc<Vec<Arm>>> {
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
        match self.defs.get(&id) {
            Some(d) => d.is_some().then_some(id),
            None => self.xdefs.get(&id).is_some_and(|d| d.is_some()).then_some(id),
        }
    }

    fn expand_items(&mut self, mac: &syn::Macro, depth: usize) -> Option<Vec<syn::Item>> {
        if depth >= MAX_DEPTH || self.budget == 0 {
            return None;
        }
        let name = self.local_name(mac)?;
        let cross_file = !self.defs.contains_key(&name);
        let arms = self.arms_of(&name)?;
        let ts = expand(&arms, &mac.tokens)?;
        let ts = if cross_file {
            if std::env::var_os("CANDOR_R1004_INSTR").is_some() {
                eprintln!("R1004XFILE\t{name}"); // §E1 REACH COUNTER, on the CHANGED branch only
            }
            respan_to(ts, mac.path.segments.first().map(|s| s.ident.span()).unwrap_or_else(proc_macro2::Span::call_site))
        } else {
            ts
        };
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
        if cross_file
            && file.items.iter().any(|it| {
                crate::lang::item_attrs(it).iter().any(|a| a.path().is_ident("cfg") || a.path().is_ident("cfg_attr"))
            })
        {
            if std::env::var_os("CANDOR_R1004_INSTR").is_some() {
                eprintln!("R1004XCFG\t{name}");
            }
            return None;
        }
        self.budget -= 1;
        self.splice(&mut file.items, depth + 1);
        Some(file.items)
    }

    fn splice(&mut self, items: &mut Vec<syn::Item>, depth: usize) {
        let mut out: Vec<syn::Item> = Vec::with_capacity(items.len());
        for mut it in std::mem::take(items) {
            match &mut it {
                syn::Item::Macro(m) if m.ident.is_none() => {
                    if let Some(gen) = self.expand_items(&m.mac, depth) {
                        // FUNCTIONS ONLY — free `fn`s and `extern` blocks, which is what a call BY PATH
                        // reaches and what R1004 is. A macro-generated `impl`/`struct`/`mod` stays
                        // unindexed exactly as before: expanding impls RESOLVES dispatch onto their
                        // methods, and on wasm-bindgen that removed a `dispatch:` hedge from
                        // `ScopedClosure::once` whose real reach (`Closure::wrap_maybe_aborting`, through
                        // `type Closure<T> = ScopedClosure<'static, T>`) has no edge — a pre-existing
                        // dropped edge the hedge was standing in front of. Owed separately.
                        //
                        // The INVOCATION STAYS, so R128's "this module holds hidden items" evidence is
                        // unchanged for whatever the expansion declared besides functions.
                        let mut fns: Vec<syn::Item> = gen
                            .into_iter()
                            .filter(|g| match g {
                                syn::Item::Fn(_) | syn::Item::ForeignMod(_) => true,
                                // SOUNDNESS R1004 (impl residual) — an INHERENT impl: its methods are
                                // reached BY PATH (`W::touch(&w)`) and by a typed receiver, exactly as a
                                // free fn is, and no dispatch universe holds them, so splicing it cannot
                                // move a CHA fan-out. A TRAIT impl stays unexpanded (see above).
                                syn::Item::Impl(i) => i.trait_.is_none(),
                                _ => false,
                            })
                            .collect();
                        if !fns.is_empty() && std::env::var_os("CANDOR_R1004_INSTR").is_some() {
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
                                if std::env::var_os("CANDOR_R1004_INSTR").is_some() {
                                    eprintln!("R1004IMPL\t{}", i.items.len()); // §E1 REACH PROBE
                                }
                                while let syn::Type::Group(gr) = &*i.self_ty {
                                    let inner = (*gr.elem).clone();
                                    *i.self_ty = inner;
                                }
                            }
                            if let Some(a) = crate::lang::item_attrs_mut(g) {
                                let mut na = m.attrs.clone();
                                na.append(a);
                                *a = na;
                            }
                        }
                        out.push(it);
                        out.extend(fns);
                        continue;
                    }
                }
                syn::Item::Mod(md) => {
                    if let Some((_, inner)) = &mut md.content {
                        self.splice(inner, depth);
                    }
                }
                _ => {}
            }
            out.push(it);
        }
        *items = out;
    }
}

/// Splice, beside every item-position invocation of a `macro_rules!` this file defines exactly once, the
/// free functions and `extern` blocks it expands to. See the module doc for what is refused.
pub(crate) fn splice_local_macros(items: &mut Vec<syn::Item>, xdefs: &CrateMacros) {
    let mut defs = HashMap::new();
    collect_defs(items, &mut defs);
    if defs.values().all(|d| d.is_none()) && xdefs.values().all(|d| d.is_none()) {
        return;
    }
    let mut cx = Ctx { arms: HashMap::new(), defs, xdefs, budget: MAX_EXPANSIONS };
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
}
