//! One matcher per open drop-glue row, each written from the row's own RE-VERIFIED cells
//! (SOUNDNESS.md, the "RE-VERIFIED 2026-09-27" halves), not from its title.
//!
//! Each emits LOOSE hits (shape present) and TIGHT hits (shape present AND the value at the shape's
//! own value position is drop-relevant in this crate). R766: only TIGHT can order work.

use crate::index::{type_all_leaves, type_owned_leaves, CrateIndex};
use crate::shapes::*;
use std::collections::HashSet;
use syn::spanned::Spanned;

pub const ROWS: &[&str] = &[
    "R189", "R198", "R200c", "R200d", "R201", "R209a", "R297", "R297opt", "R300", "R323",
];

pub struct Scope<'a> {
    pub krate: &'a str,
    pub file: &'a str,
    pub func: &'a str,
}

fn emit(out: &mut Vec<Hit>, row: &'static str, s: &Scope, line: usize, leaf: &str, tight: bool) {
    out.push(Hit {
        row,
        krate: s.krate.to_string(),
        file: s.file.to_string(),
        line,
        func: s.func.to_string(),
        leaf: leaf.to_string(),
        tight,
    });
}

fn is_wrapper(e: &syn::Expr) -> bool {
    match e {
        syn::Expr::Tuple(_) | syn::Expr::Array(_) | syn::Expr::Repeat(_) => true,
        syn::Expr::Call(c) => match &*c.func {
            syn::Expr::Path(p) => p
                .path
                .segments
                .last()
                .map(|s| matches!(s.ident.to_string().as_str(), "Some" | "Ok" | "Err"))
                .unwrap_or(false),
            _ => false,
        },
        syn::Expr::Macro(m) => m.mac.path.is_ident("vec"),
        syn::Expr::Paren(p) => is_wrapper(&p.expr),
        syn::Expr::Group(g) => is_wrapper(&g.expr),
        _ => false,
    }
}

/// A `?` token in a macro's token stream that is really the TRY OPERATOR.
///
/// String literals are a single `Literal` token, so a `?` inside `"…?"` cannot reach here. The
/// harder case is the one that made `h2` five tight R209(a) hits: `tracing::trace_span!("…",
/// ?stream.id)` uses `?` as a FIELD-CAPTURE SIGIL, and `T: ?Sized` uses it as a bound relaxation.
/// Both PRECEDE their operand. The try operator POSTFIXES an expression, so it can only follow a
/// token that can end one — `)`, `]`, `}`, an identifier, or a literal.
fn tokens_have_q(ts: proc_macro2::TokenStream) -> bool {
    let mut prev_ends_expr = false;
    for t in ts {
        match &t {
            proc_macro2::TokenTree::Punct(p) if p.as_char() == '?' => {
                if prev_ends_expr {
                    return true;
                }
            }
            proc_macro2::TokenTree::Group(g) => {
                if tokens_have_q(g.stream()) {
                    return true;
                }
            }
            _ => {}
        }
        prev_ends_expr = match &t {
            proc_macro2::TokenTree::Ident(i) => {
                // a keyword that cannot end an expression is not an operand
                !matches!(i.to_string().as_str(), "dyn" | "impl" | "for" | "where" | "as" | "in")
            }
            proc_macro2::TokenTree::Literal(_) => true,
            proc_macro2::TokenTree::Group(g) => !matches!(g.delimiter(), proc_macro2::Delimiter::None),
            proc_macro2::TokenTree::Punct(p) => p.as_char() == '?',
        };
    }
    false
}

/// Every `mac` in a body, expression- and statement-position alike.
fn body_macros(b: &syn::Block, out: &mut Vec<(usize, proc_macro2::TokenStream)>) {
    for s in &b.stmts {
        if let syn::Stmt::Macro(m) = s {
            out.push((m.mac.span().start().line, m.mac.tokens.clone()));
        }
    }
    let mut all = Vec::new();
    all_exprs(b, &mut all);
    for e in all {
        if let syn::Expr::Macro(m) = e {
            out.push((m.mac.span().start().line, m.mac.tokens.clone()));
        }
    }
}

/// The main entry: match every row against one function body.
#[allow(clippy::too_many_arguments)]
pub fn match_fn(idx: &CrateIndex, s: &Scope, sig: &syn::Signature, block: &syn::Block, out: &mut Vec<Hit>) {
    let locals = body_locals(idx, sig, block);
    let ctx = Ctx { idx, locals };
    let facts = body_facts(&ctx, block);
    let mut all: Vec<&syn::Expr> = Vec::new();
    all_exprs(block, &mut all);
    let via = let_ctor_names(idx, block);
    // Names that reach an EXIT — a `return` operand or the body's tail. R201's refined trigger needs
    // the await's OUTPUT to escape, and "bound to a local that is never returned" is not that.
    let mut exit_names: HashSet<String> = HashSet::new();
    {
        let mut exit_roots: Vec<&syn::Expr> = Vec::new();
        for e in &all {
            if let syn::Expr::Return(r) = e {
                if let Some(x) = &r.expr {
                    exit_roots.push(x)
                }
            }
        }
        if let Some(syn::Stmt::Expr(tail, None)) = block.stmts.last() {
            exit_roots.push(tail);
        }
        let mut st: Vec<&syn::Expr> = exit_roots;
        while let Some(x) = st.pop() {
            if let syn::Expr::Path(p) = x {
                if let Some(id) = p.path.get_ident() {
                    exit_names.insert(id.to_string());
                }
            }
            st.extend(children(x));
        }
    }
    // Spans of `.await`s whose output reaches an exit: inside a `?`, a `return`/tail operand, or
    // bound by a `let` whose name appears in one.
    let mut escaping_awaits: HashSet<(usize, usize)> = HashSet::new();
    {
        fn mark(e: &syn::Expr, on: bool, out: &mut HashSet<(usize, usize)>) {
            let on = on || matches!(e, syn::Expr::Try(_) | syn::Expr::Return(_));
            if on {
                if let syn::Expr::Await(_) = e {
                    let sp = e.span().start();
                    out.insert((sp.line, sp.column));
                }
            }
            for c in children(e) {
                mark(c, on, out);
            }
        }
        for e in block_exprs(block) {
            mark(e, false, &mut escaping_awaits);
        }
        if let Some(syn::Stmt::Expr(tail, None)) = block.stmts.last() {
            mark(tail, true, &mut escaping_awaits);
        }
        // `let v = <..>.await;` where `v` reaches an exit
        fn lets_awaits(b: &syn::Block, names: &HashSet<String>, out: &mut HashSet<(usize, usize)>) {
            for st in &b.stmts {
                if let syn::Stmt::Local(l) = st {
                    if let (syn::Pat::Ident(pi), Some(init)) = (&l.pat, &l.init) {
                        if names.contains(&pi.ident.to_string()) {
                            let mut stk = vec![&*init.expr];
                            while let Some(x) = stk.pop() {
                                if matches!(x, syn::Expr::Await(_)) {
                                    let sp = x.span().start();
                                    out.insert((sp.line, sp.column));
                                }
                                stk.extend(children(x));
                            }
                        }
                    }
                }
            }
        }
        lets_awaits(block, &exit_names, &mut escaping_awaits);
    }

    // ---- R200d: DECL side. A by-value parameter that is a CONTAINER over a drop-relevant leaf.
    // The row's own fine-cases are excluded: a bare `H`, and `Box<H>`/`Arc`/`Rc` (the callee is
    // charged and the caller inherits). A reference parameter is not by-value.
    for a in &sig.inputs {
        if let syn::FnArg::Typed(t) = a {
            let fo = type_owned_leaves(&t.ty);
            if fo.borrowed {
                continue;
            }
            let (top, bare) = top_of_type(&t.ty);
            if matches!(top.as_deref(), Some("Box") | Some("Arc") | Some("Rc")) {
                continue;
            }
            if bare {
                continue; // a bare-typed parameter is the row's own CONTROL, charged correctly
            }
            let inner: Vec<&String> =
                fo.leaves.iter().filter(|l| Some(l.as_str()) != top.as_deref()).collect();
            let loose = inner.iter().any(|l| idx.fields.contains_key(l.as_str()));
            let tight = inner.iter().find(|l| idx.drop_relevant.contains(l.as_str()));
            if let Some(l) = tight {
                emit(out, "R200d", s, t.span().start().line, l, true);
            } else if loose {
                emit(out, "R200d", s, t.span().start().line, "-", false);
            }
        }
    }

    for e in &all {
        match e {
            // ---- R201: `.await` on a base that is NOT an inline `async` block, carrying a
            // construction the future OWNS, whose output is USED (the row's refined trigger).
            syn::Expr::Await(a) => {
                if matches!(&*a.base, syn::Expr::Async(_)) {
                    continue;
                }
                let sp = e.span().start();
                if facts.discarded.contains(&(sp.line, sp.column)) {
                    continue; // statement-position `.await;` is CHARGED today
                }
                if !escaping_awaits.contains(&(sp.line, sp.column)) {
                    continue; // the row's refined trigger: the OUTPUT must escape
                }
                let leaves = ctx.ctor_leaves_explicit(&a.base, &via, true);
                let mut t: Vec<&String> =
                    leaves.iter().filter(|l| idx.drop_relevant.contains(l.as_str())).collect();
                t.sort();
                if let Some(l) = t.first() {
                    emit(out, "R201", s, sp.line, l, true);
                } else if !leaves.is_empty() {
                    emit(out, "R201", s, sp.line, "-", false);
                }
            }

            // ---- R200c: CALL SITE. A by-value WRAPPER argument containing a construction.
            syn::Expr::Call(_) | syn::Expr::MethodCall(_) => {
                let (args, meth): (Vec<&syn::Expr>, Option<String>) = match e {
                    syn::Expr::Call(c) => {
                        // A tuple-struct / enum-variant CONSTRUCTOR is not a callee that can drop
                        // its argument — it wraps it and hands it on. `Poll::Ready(Ok(Buffer{..}))`
                        // scored a tight R200c hit until this was here.
                        if let syn::Expr::Path(p) = &*c.func {
                            let upper = p
                                .path
                                .segments
                                .last()
                                .map(|x| x.ident.to_string().chars().next().map(|ch| ch.is_uppercase()).unwrap_or(false))
                                .unwrap_or(false);
                            if upper {
                                continue;
                            }
                        }
                        (c.args.iter().collect(), None)
                    }
                    syn::Expr::MethodCall(m) => (m.args.iter().collect(), Some(m.method.to_string())),
                    _ => unreachable!(),
                };
                // the std-callee cells: `.ok_or(H::new())`, `.unwrap_or(H::new())` — the drop is
                // inside std, so a BARE construction in the argument is the shape there.
                let std_sink = matches!(
                    meth.as_deref(),
                    Some("ok_or") | Some("unwrap_or") | Some("get_or_insert") | Some("and") | Some("or")
                );
                for arg in args {
                    if matches!(arg, syn::Expr::Reference(_)) {
                        continue;
                    }
                    if !(is_wrapper(arg) || std_sink) {
                        continue;
                    }
                    if let Some(l) = ctx.constructs_relevant(arg) {
                        emit(out, "R200c", s, arg.span().start().line, &l, true);
                    } else if ctx.constructs_any(arg) {
                        emit(out, "R200c", s, arg.span().start().line, "-", false);
                    }
                }
                // ---- R198: the ACCUMULATOR forms.
                if let Some(m) = &meth {
                    let is_fold = matches!(m.as_str(), "try_fold" | "try_for_each");
                    let is_res_collect = m == "collect" && {
                        if let syn::Expr::MethodCall(mc) = e {
                            turbofish_mentions_result(mc)
                        } else {
                            false
                        }
                    };
                    if is_fold || is_res_collect {
                        let (recv, cargs): (&syn::Expr, Vec<&syn::Expr>) = match e {
                            syn::Expr::MethodCall(mc) => (&mc.receiver, mc.args.iter().collect()),
                            _ => continue,
                        };
                        let mut cls: Vec<&syn::ExprClosure> = Vec::new();
                        for a in cargs {
                            if let syn::Expr::Closure(c) = a {
                                cls.push(c)
                            }
                        }
                        if is_res_collect {
                            // the `.map(|..| ..).collect::<Result<_,_>>()` twin
                            let mut r = Vec::new();
                            fn recclos<'a>(e: &'a syn::Expr, out: &mut Vec<&'a syn::ExprClosure>) {
                                if let syn::Expr::Closure(c) = e {
                                    out.push(c)
                                }
                                for c in children(e) {
                                    recclos(c, out)
                                }
                            }
                            recclos(recv, &mut r);
                            cls.extend(r);
                        }
                        for c in cls {
                            if is_fold && !tail_is_bare_ok(&c.body) {
                                continue; // the row's own control: an explicit `Err` arm is CHARGED
                            }
                            if let Some(l) = ctx.constructs_relevant(&c.body) {
                                emit(out, "R198", s, e.span().start().line, &l, true);
                            } else if ctx.constructs_any(&c.body) {
                                emit(out, "R198", s, e.span().start().line, "-", false);
                            }
                        }
                    }
                }
            }

            // ---- R297: assignment THROUGH A PLACE. The dying value has no construction site in
            // this body at all, so the leaf must come from the PLACE's declared type.
            syn::Expr::Assign(a) => {
                let kind_ok = matches!(
                    &*a.left,
                    syn::Expr::Unary(_) | syn::Expr::Field(_) | syn::Expr::Index(_) | syn::Expr::Path(_)
                );
                if !kind_ok {
                    continue;
                }
                let leaves = ctx.place_leaves(&a.left);
                let mut tight: Vec<&String> =
                    leaves.iter().filter(|l| idx.drop_relevant.contains(l.as_str())).collect();
                tight.sort();
                // SPLIT, because it inverts how the number reads. Where the PLACE is an `Option`,
                // the previous value may be `None` and the assignment may drop nothing — and the
                // census's own top hits are that shape (`this.acquire_slow = Some(..)` immediately
                // after matching `None`). The row's remedy accepts that over-charge on purpose
                // ("over-charging it is the acceptable direction"), so these sites DO move a row —
                // but as blast radius, not as payoff. Counted separately so nobody reads one as the
                // other.
                let row: &'static str =
                    if leaves.iter().any(|l| l == "Option") { "R297opt" } else { "R297" };
                if let Some(l) = tight.first() {
                    emit(out, row, s, e.span().start().line, l, true);
                } else {
                    emit(out, row, s, e.span().start().line, "-", false);
                }
            }
            _ => {}
        }
    }

    // ---- R209(a): a `?` living inside MACRO TOKENS, where it is the body's SOLE exit (the row's
    // own narrowing — a second `?` written outside the macro rescues it).
    let mut macs = Vec::new();
    body_macros(block, &mut macs);
    let mut body_relevant: HashSet<String> = HashSet::new();
    for e in &all {
        for l in ctx.ctor_leaves(e) {
            if idx.drop_relevant.contains(&l) {
                body_relevant.insert(l);
            }
        }
    }
    let body_constructs_any = all.iter().any(|e| ctx.constructs_any(e));
    for (line, ts) in macs {
        if !tokens_have_q(ts) {
            continue;
        }
        if !facts.plain_q && !body_relevant.is_empty() {
            let mut v: Vec<&String> = body_relevant.iter().collect();
            v.sort();
            emit(out, "R209a", s, line, v[0], true);
        } else if body_constructs_any {
            emit(out, "R209a", s, line, "-", false);
        }
    }

    // ---- R323: two or more `let <name> = |..| ..` bindings of ONE name in one body.
    for (name, binds) in &facts.closure_lets {
        if binds.len() < 2 {
            continue;
        }
        let mut tight: Vec<&String> = binds
            .iter()
            .flat_map(|(_, lv)| lv.iter())
            .filter(|l| idx.drop_relevant.contains(l.as_str()))
            .collect();
        tight.sort();
        tight.dedup();
        let line = binds[0].0;
        if let Some(l) = tight.first() {
            emit(out, "R323", s, line, l, true);
        } else if binds.iter().any(|(_, lv)| !lv.is_empty()) {
            emit(out, "R323", s, line, name, false);
        }
    }

    // ---- R300: a `fn` ITEM declared in a BODY, called here, whose leaf is ALSO constructed on an
    // escaping exit of the enclosing body (the 2026-09-27 re-verification: the escape is a second
    // NECESSARY ingredient — pair (1) does not reproduce without it).
    let mut inner_fns: Vec<&syn::ItemFn> = Vec::new();
    collect_body_fns(block, &mut inner_fns);
    for f in inner_fns {
        let mut inner_exprs = Vec::new();
        all_exprs(&f.block, &mut inner_exprs);
        let mut leaves: HashSet<String> = HashSet::new();
        for e in &inner_exprs {
            leaves.extend(ctx.ctor_leaves(e));
        }
        if leaves.is_empty() {
            continue;
        }
        let name = f.sig.ident.to_string();
        let called = all.iter().any(|e| match e {
            syn::Expr::Call(c) => matches!(&*c.func, syn::Expr::Path(p) if p.path.is_ident(&name)),
            _ => false,
        });
        if !called {
            continue;
        }
        let mut tight: Vec<&String> = leaves
            .iter()
            .filter(|l| idx.drop_relevant.contains(l.as_str()) && facts.escaping.contains(l.as_str()))
            .collect();
        tight.sort();
        if let Some(l) = tight.first() {
            emit(out, "R300", s, f.sig.ident.span().start().line, l, true);
        } else {
            emit(out, "R300", s, f.sig.ident.span().start().line, "-", false);
        }
    }

    // ---- R189 (CLOSED — carried as the probe's REAL-WORLD CALIBRATION, see REACH.md). Two or more
    // EXITS, at least one an explicit `return`, each escaping the SAME drop-relevant leaf THROUGH A
    // DIFFERENT BINDING.
    //
    // The binding requirement is load-bearing and it is what this matcher got wrong first. R189's
    // fix splits sites into operand-local (still unioned) and binding-mediated (intersected), so
    // `return Vec::new();` + a tail `Vec` — `allocator-api2`'s `repeat` — is NOT this row, and
    // neither is `anyhow`'s `render`, which the row itself names as the case the union keeps
    // uncharged. Both were tight hits until this condition was added.
    //
    // R766 measured the true payoff of R189's fix on this census as ZERO rows. A big number here is
    // evidence the probe is wrong, not that the row is big; that is the point of keeping it.
    let exit_via = |e: &syn::Expr| -> std::collections::HashMap<String, HashSet<String>> {
        let mut m: std::collections::HashMap<String, HashSet<String>> = Default::default();
        let mut st = vec![e];
        while let Some(x) = st.pop() {
            if let syn::Expr::Path(p) = x {
                if let Some(id) = p.path.get_ident() {
                    let n = id.to_string();
                    if let Some(ls) = via.get(&n) {
                        for l in ls {
                            m.entry(l.clone()).or_default().insert(n.clone());
                        }
                    }
                }
            }
            st.extend(children(x));
        }
        m
    };
    let mut exits: Vec<std::collections::HashMap<String, HashSet<String>>> = Vec::new();
    let mut has_return = false;
    for e in &all {
        if let syn::Expr::Return(r) = e {
            has_return = true;
            exits.push(r.expr.as_ref().map(|x| exit_via(x)).unwrap_or_default());
        }
    }
    if let Some(syn::Stmt::Expr(tail, None)) = block.stmts.last() {
        exits.push(exit_via(tail));
    }
    if has_return && exits.len() >= 2 {
        let mut best_tight: Option<String> = None;
        let mut any = false;
        let mut per: std::collections::HashMap<&str, (usize, HashSet<&str>)> = Default::default();
        for ex in &exits {
            for (l, names) in ex {
                let e = per.entry(l.as_str()).or_insert((0, HashSet::new()));
                e.0 += 1;
                for n in names {
                    e.1.insert(n.as_str());
                }
            }
        }
        let mut keys: Vec<&&str> = per.keys().collect();
        keys.sort();
        for l in keys {
            let (n_exits, names) = &per[*l];
            if *n_exits >= 2 && names.len() >= 2 {
                any = true;
                if idx.drop_relevant.contains(*l) && best_tight.is_none() {
                    best_tight = Some((*l).to_string());
                }
            }
        }
        let line = block.span().start().line;
        if let Some(l) = best_tight {
            emit(out, "R189", s, line, &l, true);
        } else if any {
            emit(out, "R189", s, line, "-", false);
        }
    }
}

fn collect_body_fns<'a>(b: &'a syn::Block, out: &mut Vec<&'a syn::ItemFn>) {
    for s in &b.stmts {
        if let syn::Stmt::Item(syn::Item::Fn(f)) = s {
            out.push(f)
        }
    }
}

fn top_of_type(ty: &syn::Type) -> (Option<String>, bool) {
    match ty {
        syn::Type::Path(tp) => {
            let seg = match tp.path.segments.last() {
                Some(s) => s,
                None => return (None, false),
            };
            let bare = matches!(seg.arguments, syn::PathArguments::None);
            (Some(seg.ident.to_string()), bare)
        }
        syn::Type::Tuple(_) => (Some("(tuple)".into()), false),
        syn::Type::Array(_) | syn::Type::Slice(_) => (Some("(array)".into()), false),
        syn::Type::Paren(p) => top_of_type(&p.elem),
        syn::Type::Group(g) => top_of_type(&g.elem),
        syn::Type::ImplTrait(_) => (Some("(impl)".into()), false),
        _ => (None, false),
    }
}

fn turbofish_mentions_result(m: &syn::ExprMethodCall) -> bool {
    match &m.turbofish {
        Some(tf) => {
            let mut leaves = Vec::new();
            for a in &tf.args {
                if let syn::GenericArgument::Type(t) = a {
                    type_all_leaves(t, &mut leaves)
                }
            }
            leaves.iter().any(|l| l == "Result")
        }
        None => false,
    }
}

/// The closure tail is a bare `Ok(..)` — R198's discriminator, established by its own one-variable
/// control (`Ok(acc)` SILENT vs `if .. {Ok(acc)} else {Err(())}` CHARGED).
fn tail_is_bare_ok(body: &syn::Expr) -> bool {
    fn tail(e: &syn::Expr) -> &syn::Expr {
        match e {
            syn::Expr::Block(b) => match b.block.stmts.last() {
                Some(syn::Stmt::Expr(x, None)) => tail(x),
                _ => e,
            },
            syn::Expr::Paren(p) => tail(&p.expr),
            syn::Expr::Group(g) => tail(&g.expr),
            _ => e,
        }
    }
    matches!(tail(body), syn::Expr::Call(c)
        if matches!(&*c.func, syn::Expr::Path(p)
            if p.path.segments.last().map(|s| s.ident == "Ok").unwrap_or(false)))
}
