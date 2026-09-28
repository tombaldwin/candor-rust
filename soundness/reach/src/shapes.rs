//! PASS B — the SHAPE matchers, one per open drop-glue row.
//!
//! Every matcher reports a site TWICE over: LOOSE (the shape appears) and TIGHT (the shape appears
//! AND the value at its own value position is of a type in this crate's `drop_relevant` set). Only
//! TIGHT can order work — that is the whole of SOUNDNESS R766. LOOSE is kept beside it because the
//! ratio is the interesting number: it says how much of the old ordering was shape and how much was
//! payoff.

use crate::index::{path_leaf, type_all_leaves, CrateIndex};
use std::collections::{HashMap, HashSet};
use syn::spanned::Spanned;

#[derive(Debug, Clone)]
pub struct Hit {
    pub row: &'static str,
    pub krate: String,
    pub file: String,
    pub line: usize,
    pub func: String,
    pub leaf: String,
    pub tight: bool,
}

// ---------------------------------------------------------------------------------------------
// LEAF EXTRACTION — "what does this expression subtree CONSTRUCT?"
//
// Mirrors the engine's `ctor_leaf` keying: a struct literal names its own type, `T::assoc()` names
// `T`, a bare `f()` names `f`'s declared return leaves, and `x.m()` names `m`'s return leaves ONLY
// while `m` resolves uniquely crate-wide. A path expression resolves through the body's `let`
// bindings. Anything else contributes nothing — which is a SILENCE, not a zero (see REACH.md).
// ---------------------------------------------------------------------------------------------

pub struct Ctx<'a> {
    pub idx: &'a CrateIndex,
    /// `let` name -> leaves, and parameter name -> leaves.
    pub locals: HashMap<String, Vec<String>>,
}

impl<'a> Ctx<'a> {
    fn push_leaves_of_path_call(&self, p: &syn::Path, out: &mut HashSet<String>) {
        let segs: Vec<String> = p.segments.iter().map(|s| s.ident.to_string()).collect();
        if segs.len() >= 2 {
            // `H::new`, `ffi::H::with_capacity` -> `H`
            let owner = &segs[segs.len() - 2];
            if owner.chars().next().map(|c| c.is_uppercase()).unwrap_or(false) {
                out.insert(owner.clone());
            }
            let key = format!("{}::{}", owner, segs[segs.len() - 1]);
            if let Some(v) = self.idx.fn_ret.get(&key) {
                out.extend(v.iter().cloned());
            }
        } else if let Some(one) = segs.first() {
            if let Some(v) = self.idx.fn_ret.get(one) {
                out.extend(v.iter().cloned());
            }
        }
    }

    /// Every leaf CONSTRUCTED anywhere in this subtree.
    pub fn ctor_leaves(&self, e: &syn::Expr) -> HashSet<String> {
        let mut out = HashSet::new();
        self.walk(e, &mut out);
        out
    }

    fn walk(&self, e: &syn::Expr, out: &mut HashSet<String>) {
        match e {
            syn::Expr::Struct(s) => {
                if let Some(l) = path_leaf(&s.path) {
                    out.insert(l);
                }
            }
            syn::Expr::Call(c) => {
                if let syn::Expr::Path(p) = &*c.func {
                    self.push_leaves_of_path_call(&p.path, out);
                }
            }
            syn::Expr::MethodCall(m) => {
                if let Some(Some(v)) = self.idx.method_ret.get(&m.method.to_string()) {
                    out.extend(v.iter().cloned());
                }
            }
            syn::Expr::Path(p) => {
                if let Some(id) = p.path.get_ident() {
                    if let Some(v) = self.locals.get(&id.to_string()) {
                        out.extend(v.iter().cloned());
                    }
                }
            }
            syn::Expr::Macro(m) => {
                // `vec![H::new()]`, `hashmap!{..}` — parse the tokens as an expression list when we
                // can, and say nothing when we cannot. R209(a) is the row about what that costs.
                if let Ok(exprs) = m
                    .mac
                    .parse_body_with(syn::punctuated::Punctuated::<syn::Expr, syn::Token![,]>::parse_terminated)
                {
                    for x in &exprs {
                        self.walk_deep(x, out);
                    }
                }
            }
            _ => {}
        }
        // children
        for c in children(e) {
            self.walk_deep_shallow(c, out);
        }
    }

    fn walk_deep_shallow(&self, e: &syn::Expr, out: &mut HashSet<String>) {
        self.walk(e, out);
    }
    fn walk_deep(&self, e: &syn::Expr, out: &mut HashSet<String>) {
        self.walk(e, out);
    }

    /// Leaves a subtree EXPLICITLY constructs — struct literals, `T::assoc()` and bare-fn returns,
    /// plus `let`-bound names whose initialiser constructed one. Method RETURN types are excluded
    /// on purpose.
    ///
    /// R201 is why this exists. `self.subscribe(inbox).await?` scored a tight hit on leaf
    /// `Subscriber` only because `subscribe` RETURNS a `Subscriber` — which is the await's OUTPUT,
    /// handed to the caller, and therefore exactly not a value dropped in this frame. The row is
    /// about a value the future OWNS. `skip_outer` drops the base node's own return type for the
    /// same reason while keeping constructions in its arguments and fields.
    pub fn ctor_leaves_explicit(
        &self,
        e: &syn::Expr,
        via: &HashMap<String, Vec<String>>,
        skip_outer: bool,
    ) -> HashSet<String> {
        let mut out = HashSet::new();
        let mut stack: Vec<(&syn::Expr, bool)> = vec![(e, skip_outer)];
        while let Some((x, skip)) = stack.pop() {
            if !skip {
                match x {
                    syn::Expr::Struct(st) => {
                        if let Some(l) = path_leaf(&st.path) {
                            out.insert(l);
                        }
                    }
                    syn::Expr::Call(c) => {
                        if let syn::Expr::Path(p) = &*c.func {
                            self.push_leaves_of_path_call(&p.path, &mut out);
                        }
                    }
                    syn::Expr::Path(p) => {
                        if let Some(id) = p.path.get_ident() {
                            if let Some(v) = via.get(&id.to_string()) {
                                out.extend(v.iter().cloned());
                            }
                        }
                    }
                    _ => {}
                }
            }
            // A method RECEIVER that is a PLACE (`runner.runnable(..)`, `subscriber.next()`) is
            // borrowed, not owned by the future — `async-executor`'s `run` and `async-nats`'s
            // `send_request` were tight hits on exactly that. A receiver that is an RVALUE
            // (`Ticker::new(self).runnable()`) really is moved in and dropped in this frame, so it
            // is descended. Same reasoning for a field/index BASE.
            match x {
                syn::Expr::MethodCall(m) => {
                    if !is_place(&m.receiver) {
                        stack.push((&m.receiver, false));
                    }
                    for a in &m.args {
                        stack.push((a, false));
                    }
                }
                // A CLOSURE body does not construct anything here — it constructs when the closure
                // runs, wherever the callee runs it. `async_std::File::open`'s
                // `spawn_blocking(move || std::fs::File::open(..)).await?` was a tight R201 hit on
                // exactly this, and the `File` it named was constructed on a thread pool.
                syn::Expr::Closure(_) => {}
                syn::Expr::Field(_) | syn::Expr::Index(_) => {}
                _ => {
                    for c in children(x) {
                        stack.push((c, false));
                    }
                }
            }
        }
        out
    }

    pub fn constructs_relevant(&self, e: &syn::Expr) -> Option<String> {
        let mut v: Vec<String> = self
            .ctor_leaves(e)
            .into_iter()
            .filter(|l| self.idx.drop_relevant.contains(l))
            .collect();
        v.sort();
        v.into_iter().next()
    }

    pub fn constructs_any(&self, e: &syn::Expr) -> bool {
        !self.ctor_leaves(e).is_empty()
    }

    /// Resolve the leaves a PLACE expression's value has (R297). References are NOT peeled away
    /// here: `*slot = v` with `slot: &mut Guard` really does drop a `Guard` in this frame.
    pub fn place_leaves(&self, e: &syn::Expr) -> Vec<String> {
        match e {
            syn::Expr::Path(p) => p
                .path
                .get_ident()
                .and_then(|i| self.locals.get(&i.to_string()))
                .cloned()
                .unwrap_or_default(),
            syn::Expr::Unary(u) if matches!(u.op, syn::UnOp::Deref(_)) => self.place_leaves(&u.expr),
            syn::Expr::Paren(p) => self.place_leaves(&p.expr),
            syn::Expr::Group(g) => self.place_leaves(&g.expr),
            syn::Expr::Index(i) => self.place_leaves(&i.expr),
            syn::Expr::Field(f) => {
                let fname = match &f.member {
                    syn::Member::Named(n) => n.to_string(),
                    syn::Member::Unnamed(i) => i.index.to_string(),
                };
                let owners = self.place_leaves(&f.base);
                let mut out = Vec::new();
                for o in owners {
                    if let Some(fs) = self.idx.fields.get(&o) {
                        for fo in fs {
                            if fo.name.as_deref() == Some(fname.as_str()) {
                                out.extend(fo.all_leaves.iter().cloned());
                            }
                        }
                    }
                }
                if out.is_empty() {
                    // Owner unknown. Fall back to the field name ONLY when it is unique crate-wide —
                    // a guess with two candidates is not a measurement.
                    let mut cands: Vec<&crate::index::FieldOwn> = Vec::new();
                    for fs in self.idx.fields.values() {
                        for fo in fs {
                            if fo.name.as_deref() == Some(fname.as_str()) {
                                cands.push(fo);
                            }
                        }
                    }
                    if cands.len() == 1 {
                        out.extend(cands[0].all_leaves.iter().cloned());
                    }
                }
                out
            }
            _ => Vec::new(),
        }
    }
}

/// A PLACE expression — something that names storage rather than producing a fresh value.
pub fn is_place(e: &syn::Expr) -> bool {
    match e {
        syn::Expr::Path(_) | syn::Expr::Field(_) | syn::Expr::Index(_) => true,
        syn::Expr::Unary(u) => matches!(u.op, syn::UnOp::Deref(_)),
        syn::Expr::Paren(p) => is_place(&p.expr),
        syn::Expr::Group(g) => is_place(&g.expr),
        _ => false,
    }
}

/// The direct expression children of an expression. Hand-written rather than `syn::visit`, because
/// the matchers need to STOP at closure and nested-fn boundaries in some places and not in others.
pub fn children(e: &syn::Expr) -> Vec<&syn::Expr> {
    let mut v: Vec<&syn::Expr> = Vec::new();
    macro_rules! p {
        ($($x:expr),*) => {{ $( v.push($x); )* }};
    }
    match e {
        syn::Expr::Array(a) => v.extend(a.elems.iter()),
        syn::Expr::Assign(a) => p!(&a.left, &a.right),
        syn::Expr::Async(a) => v.extend(block_exprs(&a.block)),
        syn::Expr::Await(a) => p!(&a.base),
        syn::Expr::Binary(b) => p!(&b.left, &b.right),
        syn::Expr::Block(b) => v.extend(block_exprs(&b.block)),
        syn::Expr::Break(b) => {
            if let Some(x) = &b.expr {
                p!(x)
            }
        }
        syn::Expr::Call(c) => {
            p!(&c.func);
            v.extend(c.args.iter())
        }
        syn::Expr::Cast(c) => p!(&c.expr),
        syn::Expr::Closure(c) => p!(&c.body),
        syn::Expr::Const(c) => v.extend(block_exprs(&c.block)),
        syn::Expr::Field(f) => p!(&f.base),
        syn::Expr::ForLoop(f) => {
            p!(&f.expr);
            v.extend(block_exprs(&f.body))
        }
        syn::Expr::Group(g) => p!(&g.expr),
        syn::Expr::If(i) => {
            p!(&i.cond);
            v.extend(block_exprs(&i.then_branch));
            if let Some((_, e2)) = &i.else_branch {
                p!(e2)
            }
        }
        syn::Expr::Index(i) => p!(&i.expr, &i.index),
        syn::Expr::Let(l) => p!(&l.expr),
        syn::Expr::Loop(l) => v.extend(block_exprs(&l.body)),
        syn::Expr::Match(m) => {
            p!(&m.expr);
            for a in &m.arms {
                v.push(&a.body);
                if let Some((_, g)) = &a.guard {
                    v.push(g)
                }
            }
        }
        syn::Expr::MethodCall(m) => {
            p!(&m.receiver);
            v.extend(m.args.iter())
        }
        syn::Expr::Paren(p) => p!(&p.expr),
        syn::Expr::Range(r) => {
            if let Some(x) = &r.start {
                p!(x)
            }
            if let Some(x) = &r.end {
                p!(x)
            }
        }
        syn::Expr::Reference(r) => p!(&r.expr),
        syn::Expr::Repeat(r) => p!(&r.expr, &r.len),
        syn::Expr::Return(r) => {
            if let Some(x) = &r.expr {
                p!(x)
            }
        }
        syn::Expr::Struct(s) => {
            for f in &s.fields {
                v.push(&f.expr)
            }
            if let Some(r) = &s.rest {
                v.push(r)
            }
        }
        syn::Expr::Try(t) => p!(&t.expr),
        syn::Expr::TryBlock(t) => v.extend(block_exprs(&t.block)),
        syn::Expr::Tuple(t) => v.extend(t.elems.iter()),
        syn::Expr::Unary(u) => p!(&u.expr),
        syn::Expr::Unsafe(u) => v.extend(block_exprs(&u.block)),
        syn::Expr::While(w) => {
            p!(&w.cond);
            v.extend(block_exprs(&w.body))
        }
        syn::Expr::Yield(y) => {
            if let Some(x) = &y.expr {
                p!(x)
            }
        }
        _ => {}
    }
    v
}

pub fn block_exprs(b: &syn::Block) -> Vec<&syn::Expr> {
    let mut v = Vec::new();
    for s in &b.stmts {
        match s {
            syn::Stmt::Local(l) => {
                if let Some(init) = &l.init {
                    v.push(&*init.expr);
                    if let Some((_, d)) = &init.diverge {
                        v.push(&**d)
                    }
                }
            }
            syn::Stmt::Expr(e, _) => v.push(e),
            _ => {}
        }
    }
    v
}

/// Every expression in a body, in pre-order, INCLUDING closure and nested-fn bodies.
pub fn all_exprs<'a>(b: &'a syn::Block, out: &mut Vec<&'a syn::Expr>) {
    fn rec<'a>(e: &'a syn::Expr, out: &mut Vec<&'a syn::Expr>) {
        out.push(e);
        for c in children(e) {
            rec(c, out)
        }
    }
    for e in block_exprs(b) {
        rec(e, out);
    }
    // nested `fn` items declared in the body (R300's subject)
    for s in &b.stmts {
        if let syn::Stmt::Item(syn::Item::Fn(f)) = s {
            all_exprs(&f.block, out);
        }
    }
}

// ---------------------------------------------------------------------------------------------
// PER-BODY FACTS
// ---------------------------------------------------------------------------------------------

pub struct BodyFacts {
    /// Spans of expressions in DISCARDED position (`e;` as a statement, or `let _ = e;`).
    pub discarded: HashSet<(usize, usize)>,
    /// Leaves constructed in an operand of `return`, or in the body's tail expression.
    pub escaping: HashSet<String>,
    /// Does the body carry a `?` written outside macro tokens?
    pub plain_q: bool,
    /// `let <name> = |..| ..` bindings, by name, with the closure bodies.
    pub closure_lets: HashMap<String, Vec<(usize, Vec<String>)>>,
}

pub fn body_facts(ctx: &Ctx, b: &syn::Block) -> BodyFacts {
    let mut f = BodyFacts {
        discarded: HashSet::new(),
        escaping: HashSet::new(),
        plain_q: false,
        closure_lets: HashMap::new(),
    };
    collect_discarded(b, &mut f.discarded);
    // escaping: `return X` anywhere, plus the body's own tail expression
    let mut all = Vec::new();
    all_exprs(b, &mut all);
    for e in &all {
        if let syn::Expr::Return(r) = e {
            if let Some(x) = &r.expr {
                f.escaping.extend(ctx.ctor_leaves(x));
            }
        }
        if matches!(e, syn::Expr::Try(_)) {
            f.plain_q = true;
        }
        if let syn::Expr::Closure(c) = e {
            // recorded by name at the `let` below; here only to keep the walk in one place
            let _ = c;
        }
    }
    if let Some(syn::Stmt::Expr(tail, None)) = b.stmts.last() {
        f.escaping.extend(ctx.ctor_leaves(tail));
    }
    collect_closure_lets(ctx, b, &mut f.closure_lets);
    f
}

fn collect_discarded(b: &syn::Block, out: &mut HashSet<(usize, usize)>) {
    for s in &b.stmts {
        match s {
            syn::Stmt::Expr(e, Some(_)) => {
                let sp = e.span().start();
                out.insert((sp.line, sp.column));
            }
            syn::Stmt::Local(l) => {
                if matches!(l.pat, syn::Pat::Wild(_)) {
                    if let Some(init) = &l.init {
                        let sp = init.expr.span().start();
                        out.insert((sp.line, sp.column));
                    }
                }
            }
            _ => {}
        }
    }
    // nested blocks
    let mut all = Vec::new();
    for e in block_exprs(b) {
        fn rec<'a>(e: &'a syn::Expr, out: &mut Vec<&'a syn::Block>) {
            match e {
                syn::Expr::Block(x) => out.push(&x.block),
                syn::Expr::Async(x) => out.push(&x.block),
                syn::Expr::Unsafe(x) => out.push(&x.block),
                syn::Expr::TryBlock(x) => out.push(&x.block),
                syn::Expr::Loop(x) => out.push(&x.body),
                syn::Expr::While(x) => out.push(&x.body),
                syn::Expr::ForLoop(x) => out.push(&x.body),
                syn::Expr::If(x) => out.push(&x.then_branch),
                syn::Expr::Closure(c) => {
                    if let syn::Expr::Block(x) = &*c.body {
                        out.push(&x.block)
                    }
                }
                _ => {}
            }
            for c in children(e) {
                rec(c, out)
            }
        }
        rec(e, &mut all);
    }
    for nb in all {
        collect_discarded(nb, out);
    }
}

fn collect_closure_lets(ctx: &Ctx, b: &syn::Block, out: &mut HashMap<String, Vec<(usize, Vec<String>)>>) {
    for s in &b.stmts {
        if let syn::Stmt::Local(l) = s {
            if let (syn::Pat::Ident(pi), Some(init)) = (&l.pat, &l.init) {
                if let syn::Expr::Closure(c) = &*init.expr {
                    let mut lv: Vec<String> = ctx.ctor_leaves(&c.body).into_iter().collect();
                    lv.sort();
                    out.entry(pi.ident.to_string())
                        .or_default()
                        .push((init.expr.span().start().line, lv));
                }
            }
        }
    }
    let mut nested = Vec::new();
    for e in block_exprs(b) {
        fn rec<'a>(e: &'a syn::Expr, out: &mut Vec<&'a syn::Block>) {
            match e {
                syn::Expr::Block(x) => out.push(&x.block),
                syn::Expr::Async(x) => out.push(&x.block),
                syn::Expr::Unsafe(x) => out.push(&x.block),
                syn::Expr::Loop(x) => out.push(&x.body),
                syn::Expr::While(x) => out.push(&x.body),
                syn::Expr::ForLoop(x) => out.push(&x.body),
                syn::Expr::If(x) => out.push(&x.then_branch),
                _ => {}
            }
            for c in children(e) {
                rec(c, out)
            }
        }
        rec(e, &mut nested);
    }
    for nb in nested {
        collect_closure_lets(ctx, nb, out);
    }
}

/// `let <name> = <init>` bindings whose INITIALISER constructs something — i.e. the names through
/// which a construction is BINDING-MEDIATED rather than operand-local.
///
/// R189's fix turns on exactly this distinction: a site written inline in an exit operand is
/// evaluated on that exit alone and is still unioned (which is what keeps `anyhow`'s `render`
/// uncharged), while a site reached through a binding is intersected. A reach probe that ignores it
/// counts `return Vec::new();` as R189 — and `allocator-api2`'s `repeat` and `anyhow`'s `render`
/// were the two hits that caught this probe doing so.
pub fn let_ctor_names(idx: &CrateIndex, block: &syn::Block) -> HashMap<String, Vec<String>> {
    fn lets<'a>(b: &'a syn::Block, out: &mut Vec<&'a syn::Local>) {
        for s in &b.stmts {
            if let syn::Stmt::Local(l) = s {
                out.push(l)
            }
        }
        let mut nb = Vec::new();
        for e in block_exprs(b) {
            fn rec<'a>(e: &'a syn::Expr, out: &mut Vec<&'a syn::Block>) {
                match e {
                    syn::Expr::Block(x) => out.push(&x.block),
                    syn::Expr::Async(x) => out.push(&x.block),
                    syn::Expr::Unsafe(x) => out.push(&x.block),
                    syn::Expr::Loop(x) => out.push(&x.body),
                    syn::Expr::While(x) => out.push(&x.body),
                    syn::Expr::ForLoop(x) => out.push(&x.body),
                    syn::Expr::If(x) => out.push(&x.then_branch),
                    _ => {}
                }
                for c in children(e) {
                    rec(c, out)
                }
            }
            rec(e, &mut nb);
        }
        for x in nb {
            lets(x, out)
        }
    }
    let mut ls = Vec::new();
    lets(block, &mut ls);
    let ctx0 = Ctx { idx, locals: HashMap::new() };
    let empty: HashMap<String, Vec<String>> = HashMap::new();
    let mut m: HashMap<String, Vec<String>> = HashMap::new();
    for l in &ls {
        let pi = match &l.pat {
            syn::Pat::Ident(pi) => pi,
            syn::Pat::Type(pt) => match &*pt.pat {
                syn::Pat::Ident(pi) => pi,
                _ => continue,
            },
            _ => continue,
        };
        if let Some(init) = &l.init {
            // EXPLICIT leaves only. A `let x = foo.bar()` claims nothing: `bar`'s return type is
            // the value handed BACK, which says nothing about what the callee owned and dropped.
            let mut v: Vec<String> =
                ctx0.ctor_leaves_explicit(&init.expr, &empty, false).into_iter().collect();
            v.sort();
            if !v.is_empty() {
                m.entry(pi.ident.to_string()).or_default().extend(v);
            }
        }
    }
    m
}

/// Locals: parameter names and `let` bindings, name -> leaves.
pub fn body_locals(idx: &CrateIndex, sig: &syn::Signature, b: &syn::Block) -> HashMap<String, Vec<String>> {
    let mut m: HashMap<String, Vec<String>> = HashMap::new();
    for a in &sig.inputs {
        if let syn::FnArg::Typed(t) = a {
            if let syn::Pat::Ident(pi) = &*t.pat {
                let mut leaves = Vec::new();
                type_all_leaves(&t.ty, &mut leaves);
                m.insert(pi.ident.to_string(), leaves);
            }
        }
    }
    // Two sweeps: declared types first (exact), then initialiser leaves for the rest.
    fn lets<'a>(b: &'a syn::Block, out: &mut Vec<&'a syn::Local>) {
        for s in &b.stmts {
            if let syn::Stmt::Local(l) = s {
                out.push(l)
            }
        }
        let mut nb = Vec::new();
        for e in block_exprs(b) {
            fn rec<'a>(e: &'a syn::Expr, out: &mut Vec<&'a syn::Block>) {
                match e {
                    syn::Expr::Block(x) => out.push(&x.block),
                    syn::Expr::Async(x) => out.push(&x.block),
                    syn::Expr::Unsafe(x) => out.push(&x.block),
                    syn::Expr::Loop(x) => out.push(&x.body),
                    syn::Expr::While(x) => out.push(&x.body),
                    syn::Expr::ForLoop(x) => out.push(&x.body),
                    syn::Expr::If(x) => out.push(&x.then_branch),
                    syn::Expr::Closure(c) => {
                        if let syn::Expr::Block(x) = &*c.body {
                            out.push(&x.block)
                        }
                    }
                    _ => {}
                }
                for c in children(e) {
                    rec(c, out)
                }
            }
            rec(e, &mut nb);
        }
        for x in nb {
            lets(x, out)
        }
    }
    let mut ls = Vec::new();
    lets(b, &mut ls);
    for l in &ls {
        if let syn::Pat::Type(pt) = &l.pat {
            if let syn::Pat::Ident(pi) = &*pt.pat {
                let mut leaves = Vec::new();
                type_all_leaves(&pt.ty, &mut leaves);
                m.insert(pi.ident.to_string(), leaves);
            }
        }
    }
    let ctx0 = Ctx { idx, locals: m.clone() };
    for l in &ls {
        if let (syn::Pat::Ident(pi), Some(init)) = (&l.pat, &l.init) {
            let name = pi.ident.to_string();
            if m.contains_key(&name) {
                continue;
            }
            let mut v: Vec<String> = ctx0.ctor_leaves(&init.expr).into_iter().collect();
            v.sort();
            if !v.is_empty() {
                m.insert(name, v);
            }
        }
    }
    m
}
