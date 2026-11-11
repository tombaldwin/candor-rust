//! THE ENVIRONMENT-READ SOURCE GATE (SPEC §3.4 ⟨0.42⟩ "THE REGISTRATION IS THE READ", SOUNDNESS R1634).
//!
//! `candor_report::env` is the only place in this workspace that may read a process environment VARIABLE, and this file
//! fails the suite on any other. That is what replaces a list of switch names (an allowlist, which fails in the absence
//! direction: a switch added without its entry is the silent one). It reads the AST of every production source file in
//! the five crates that make up the shipped binaries and the dylint lint, so it is not fooled by a name built with
//! `format!`, a `use std::env::var as v` alias, a macro argument, or a method reference.
//!
//! TWO CLAIMS, both enforced here:
//!   1. NO RAW READ. `std::env::{var, var_os, vars, vars_os}` (any path, import, alias, method reference or macro
//!      argument), `getenv`, and the `environ` symbol appear nowhere outside `crates/candor-report/src/env.rs`.
//!   2. EVERY `trace` READ IS PROVEN DIAGNOSTIC BY ITS POSITION. A `trace` read records nothing, which is only correct if
//!      its answer can reach no report or verdict byte (§3.4: "a read that only writes to stderr is out of scope"). So a
//!      call to `candor_report::env::trace` (or to a function in `TRACE_WRAPPERS`, which wrap one) must sit in the
//!      condition of an `if` (positively, as a conjunct) whose body does nothing but format to stderr and compute what it
//!      prints. A trace read bound to a `let`, negated, used in a loop condition or an `else` arm, or guarding a body that
//!      mutates anything, is refused here and must become a `switch`. The body check is structural, not a proof, and says
//!      so: it is the second instrument, and `tests/env_trace_ab.rs` is the first (it runs a scan with every trace
//!      variable set and compares the report byte for byte).
//!
//! CALIBRATED, not trusted: `the_lint_fires_on_every_seeded_spelling` and `the_lint_fires_on_a_real_file_with_one_read_appended`
//! show the gate able to fail, per spelling. A gate whose silence cannot be told from a pass is the shape this family keeps
//! rediscovering.

use std::path::{Path, PathBuf};

use syn::visit::Visit;

/// Functions that wrap a `trace` read in a `OnceLock` (a hot path reads once). Their CALL SITES are checked as trace reads.
const TRACE_WRAPPERS: &[&str] = &["reach_debug"];

/// The one file allowed to touch the environment.
const OWNER: &str = "crates/candor-report/src/env.rs";

const FORBIDDEN: &[&str] = &["var", "var_os", "vars", "vars_os"];

fn is_forbidden_env_path(segs: &[String]) -> bool {
    segs.windows(2).any(|w| w[0] == "env" && FORBIDDEN.contains(&w[1].as_str()))
}

fn cfg_is_test(attr: &syn::Attribute) -> bool {
    if !attr.path().is_ident("cfg") {
        return false;
    }
    let t = attr.meta.to_token_stream_string();
    t.contains("test") && !t.contains("not")
}

trait MetaStr {
    fn to_token_stream_string(&self) -> String;
}
impl MetaStr for syn::Meta {
    fn to_token_stream_string(&self) -> String {
        use quote_like::ToTokens;
        self.to_token_stream().to_string()
    }
}
// syn's `ToTokens` without a direct `quote` dependency: it is re-exported through syn's own prelude path.
mod quote_like {
    pub use syn::__private::ToTokens;
}

fn has_test_cfg(attrs: &[syn::Attribute]) -> bool {
    attrs.iter().any(cfg_is_test)
}

#[derive(Default)]
struct Lint {
    file: String,
    raw: Vec<String>,
    trace_unproven: Vec<String>,
    /// span-start lines of trace calls already proven by an enclosing `if`.
    proven: std::collections::HashSet<(usize, usize)>,
    /// identifiers bound (by `let`) to an expression that contains a trace read, in the function being visited. Every USE
    /// of one is itself a trace read that needs a proving `if`; the `let` is not.
    trace_vars: std::collections::HashSet<String>,
    /// positions of trace calls that sit in a `let` initialiser (proven by the uses of the bound name instead).
    bound: std::collections::HashSet<(usize, usize)>,
}

fn pos(span: proc_macro2::Span) -> (usize, usize) {
    let s = span.start();
    (s.line, s.column)
}

fn tokens_hit(ts: proc_macro2::TokenStream, out: &mut Vec<String>) {
    let toks: Vec<proc_macro2::TokenTree> = ts.into_iter().collect();
    for (i, t) in toks.iter().enumerate() {
        match t {
            proc_macro2::TokenTree::Group(g) => tokens_hit(g.stream(), out),
            proc_macro2::TokenTree::Ident(id) => {
                let s = id.to_string();
                if s == "getenv" || s == "environ" {
                    out.push(format!("{s} (in a macro argument)"));
                }
                // env :: var*
                if s == "env" {
                    let c1 = toks.get(i + 1);
                    let c2 = toks.get(i + 2);
                    let c3 = toks.get(i + 3);
                    if let (Some(proc_macro2::TokenTree::Punct(a)), Some(proc_macro2::TokenTree::Punct(b)), Some(proc_macro2::TokenTree::Ident(n))) = (c1, c2, c3) {
                        if a.as_char() == ':' && b.as_char() == ':' && FORBIDDEN.contains(&n.to_string().as_str()) {
                            out.push(format!("env::{n} (in a macro argument)"));
                        }
                    }
                }
            }
            _ => {}
        }
    }
}

/// Is `e` a `trace` read: `candor_report::env::trace(..)`, `env::trace(..)`, or a call to a registered wrapper.
fn trace_call(e: &syn::Expr) -> Option<(usize, usize)> {
    let syn::Expr::Call(c) = e else { return None };
    let syn::Expr::Path(p) = &*c.func else { return None };
    let segs: Vec<String> = p.path.segments.iter().map(|s| s.ident.to_string()).collect();
    let n = segs.len();
    let is_trace = n >= 2 && segs[n - 2] == "env" && segs[n - 1] == "trace";
    let is_wrapper = segs.last().is_some_and(|l| TRACE_WRAPPERS.contains(&l.as_str()));
    (is_trace || is_wrapper).then(|| pos(syn::spanned::Spanned::span(&c.func)))
}

/// Collect every trace read inside `cond`, at ANY depth. A `trace` read in the condition of an `if` with no `else` and a
/// diagnostic body can only choose whether the body runs, and the body only prints — so negation, `||` and nesting do not
/// matter. What does matter is that evaluating the CONDITION has no side effect (a `&&` after the trace read must not
/// mutate): checked by `cond_is_pure` below.
fn traces_in(cond: &syn::Expr, vars: &std::collections::HashSet<String>, out: &mut Vec<(usize, usize)>) {
    struct V<'a> {
        out: &'a mut Vec<(usize, usize)>,
        vars: &'a std::collections::HashSet<String>,
    }
    impl<'ast> Visit<'ast> for V<'_> {
        fn visit_expr(&mut self, e: &'ast syn::Expr) {
            // A closure BODY is its own scope: a trace read inside it is checked where it is written, and binding the
            // closure to a name does not make that name a boolean trace read.
            if matches!(e, syn::Expr::Closure(_)) {
                return;
            }
            if let Some(p) = trace_call(e) {
                self.out.push(p);
            }
            if let syn::Expr::Path(p) = e {
                if let Some(id) = p.path.get_ident() {
                    if self.vars.contains(&id.to_string()) {
                        self.out.push(pos(syn::spanned::Spanned::span(p)));
                    }
                }
            }
            syn::visit::visit_expr(self, e);
        }
    }
    V { out, vars }.visit_expr(cond);
}

/// The trace reads that ARE the value of `e`: `trace(..)`, a bound trace name, `!e`, `a && b`, `a || b`, `(e)`. A trace read buried
/// in a nested statement (`let v = match .. { .. => { if trace(..) { eprintln!(..) } None } .. }`) is NOT the value of the
/// binding — it is checked where it is written, and `v` is not a boolean trace read.
fn bool_operand_traces(e: &syn::Expr, vars: &std::collections::HashSet<String>, out: &mut Vec<(usize, usize)>) {
    match e {
        syn::Expr::Paren(p) => bool_operand_traces(&p.expr, vars, out),
        syn::Expr::Group(g) => bool_operand_traces(&g.expr, vars, out),
        syn::Expr::Unary(u) if matches!(u.op, syn::UnOp::Not(_)) => bool_operand_traces(&u.expr, vars, out),
        syn::Expr::Binary(b) if matches!(b.op, syn::BinOp::And(_) | syn::BinOp::Or(_)) => {
            bool_operand_traces(&b.left, vars, out);
            bool_operand_traces(&b.right, vars, out);
        }
        syn::Expr::Path(p) => {
            if p.path.get_ident().is_some_and(|i| vars.contains(&i.to_string())) {
                out.push(pos(syn::spanned::Spanned::span(p)));
            }
        }
        e => {
            if let Some(p) = trace_call(e) {
                out.push(p);
            }
        }
    }
}

/// Method names that mutate state the analysis later reads. A diagnostic body calling one of these is not diagnostic.
const MUTATORS: &[&str] = &[
    "insert", "push", "push_str", "push_back", "push_front", "extend", "entry", "or_insert", "or_insert_with", "or_default",
    "remove", "clear", "retain", "truncate", "drain", "sort", "sort_by", "sort_by_key", "dedup", "take", "replace", "swap",
    "get_or_insert", "get_or_insert_with", "set", "append", "resize", "borrow_mut", "lock", "fetch_add", "store", "get_mut",
    "iter_mut", "last_mut", "first_mut", "as_mut", "entry_ref", "mark_unresolved", "add", "record",
];

struct BodyCheck {
    bad: Vec<String>,
    /// names bound by `let` inside the body: assigning to those is local, not a side effect on the analysis.
    locals: std::collections::HashSet<String>,
}

fn root_ident(e: &syn::Expr) -> Option<String> {
    match e {
        syn::Expr::Path(p) => p.path.get_ident().map(|i| i.to_string()),
        syn::Expr::Field(f) => root_ident(&f.base),
        syn::Expr::Index(i) => root_ident(&i.expr),
        syn::Expr::Unary(u) => root_ident(&u.expr),
        syn::Expr::Paren(p) => root_ident(&p.expr),
        syn::Expr::MethodCall(m) => root_ident(&m.receiver),
        _ => None,
    }
}

impl<'ast> Visit<'ast> for BodyCheck {
    fn visit_local(&mut self, l: &'ast syn::Local) {
        fn names(p: &syn::Pat, out: &mut std::collections::HashSet<String>) {
            match p {
                syn::Pat::Ident(i) => {
                    out.insert(i.ident.to_string());
                }
                syn::Pat::Tuple(t) => t.elems.iter().for_each(|e| names(e, out)),
                syn::Pat::TupleStruct(t) => t.elems.iter().for_each(|e| names(e, out)),
                syn::Pat::Struct(s) => s.fields.iter().for_each(|f| names(&f.pat, out)),
                syn::Pat::Reference(r) => names(&r.pat, out),
                syn::Pat::Type(t) => names(&t.pat, out),
                _ => {}
            }
        }
        names(&l.pat, &mut self.locals);
        syn::visit::visit_local(self, l);
    }
    fn visit_pat(&mut self, p: &'ast syn::Pat) {
        // `if let Some(x) = ..`, `for x in ..`, match arms bind locals too.
        if let syn::Pat::Ident(i) = p {
            self.locals.insert(i.ident.to_string());
        }
        syn::visit::visit_pat(self, p);
    }
    fn visit_expr_assign(&mut self, a: &'ast syn::ExprAssign) {
        if !root_ident(&a.left).is_some_and(|r| self.locals.contains(&r)) {
            self.bad.push("an assignment to state outside the body".into());
        }
        syn::visit::visit_expr_assign(self, a);
    }
    fn visit_expr_binary(&mut self, b: &'ast syn::ExprBinary) {
        use syn::BinOp::*;
        if matches!(b.op, AddAssign(_) | SubAssign(_) | MulAssign(_) | DivAssign(_) | RemAssign(_) | BitXorAssign(_) | BitAndAssign(_) | BitOrAssign(_) | ShlAssign(_) | ShrAssign(_))
            && !root_ident(&b.left).is_some_and(|r| self.locals.contains(&r))
        {
            self.bad.push("a compound assignment to state outside the body".into());
        }
        syn::visit::visit_expr_binary(self, b);
    }
    fn visit_expr_method_call(&mut self, m: &'ast syn::ExprMethodCall) {
        let name = m.method.to_string();
        let local_recv = root_ident(&m.receiver).is_some_and(|r| self.locals.contains(&r));
        if (MUTATORS.contains(&name.as_str()) || name.starts_with("mark_") || name.starts_with("record_")) && !local_recv {
            self.bad.push(format!("a call to `.{name}()` on state outside the body"));
        }
        syn::visit::visit_expr_method_call(self, m);
    }
    fn visit_expr_return(&mut self, r: &'ast syn::ExprReturn) {
        self.bad.push("a `return`".into());
        syn::visit::visit_expr_return(self, r);
    }
    fn visit_expr_break(&mut self, b: &'ast syn::ExprBreak) {
        self.bad.push("a `break`".into());
        syn::visit::visit_expr_break(self, b);
    }
    fn visit_expr_continue(&mut self, c: &'ast syn::ExprContinue) {
        self.bad.push("a `continue`".into());
        syn::visit::visit_expr_continue(self, c);
    }
    fn visit_expr_try(&mut self, t: &'ast syn::ExprTry) {
        self.bad.push("a `?`".into());
        syn::visit::visit_expr_try(self, t);
    }
    fn visit_macro(&mut self, m: &'ast syn::Macro) {
        let name = m.path.segments.last().map(|s| s.ident.to_string()).unwrap_or_default();
        if !matches!(name.as_str(), "eprintln" | "eprint" | "format" | "write" | "writeln" | "matches" | "debug_assert" | "dbg") {
            self.bad.push(format!("the macro `{name}!`"));
        }
    }
}

fn body_is_diagnostic(b: &syn::Block) -> Vec<String> {
    let mut c = BodyCheck { bad: Vec::new(), locals: Default::default() };
    c.visit_block(b);
    check_tail(b, &mut c.bad);
    c.bad.sort();
    c.bad.dedup();
    c.bad
}

/// A diagnostic body yields no value: a tail expression that is a literal, a path, arithmetic or a constructed value is
/// the `if` being used as a VALUE (`if trace { 1 } else { 0 }`), which is a result.
fn check_tail(b: &syn::Block, bad: &mut Vec<String>) {
    if let Some(syn::Stmt::Expr(e, None)) = b.stmts.last() {
        if matches!(
            e,
            syn::Expr::Lit(_) | syn::Expr::Path(_) | syn::Expr::Binary(_) | syn::Expr::Unary(_) | syn::Expr::Tuple(_)
                | syn::Expr::Struct(_) | syn::Expr::Array(_) | syn::Expr::Field(_) | syn::Expr::Call(_) | syn::Expr::MethodCall(_)
        ) {
            bad.push("a tail expression that yields a value".into());
        }
    }
}

/// Evaluating the condition must not change anything the trace read could then withhold: in `a && trace() && b`, `b` is skipped
/// when the variable is unset, so `b` must be pure; `a` runs either way and may do anything (`m.remove(n).is_some() && trace()`).
fn cond_is_pure(cond: &syn::Expr) -> Vec<String> {
    fn conjuncts<'a>(e: &'a syn::Expr, out: &mut Vec<&'a syn::Expr>) {
        match e {
            syn::Expr::Paren(p) => conjuncts(&p.expr, out),
            syn::Expr::Binary(b) if matches!(b.op, syn::BinOp::And(_)) => {
                conjuncts(&b.left, out);
                conjuncts(&b.right, out);
            }
            e => out.push(e),
        }
    }
    let mut cs = Vec::new();
    conjuncts(cond, &mut cs);
    let no_vars = std::collections::HashSet::new();
    let first = cs.iter().position(|c| {
        let mut h = Vec::new();
        traces_in(c, &no_vars, &mut h);
        !h.is_empty() || matches!(c, syn::Expr::Path(_))
    });
    let tail: Vec<&syn::Expr> = match first {
        // everything AFTER the first conjunct that holds a trace read (and that conjunct's own subexpressions)
        Some(i) => cs[i..].to_vec(),
        None => cs,
    };
    let mut bad = Vec::new();
    for c in tail {
        let mut ck = BodyCheck { bad: Vec::new(), locals: Default::default() };
        ck.visit_expr(c);
        bad.extend(ck.bad.into_iter().filter(|b| !b.starts_with("the macro `matches!`")));
    }
    bad
}

impl<'ast> Visit<'ast> for Lint {
    fn visit_item_mod(&mut self, m: &'ast syn::ItemMod) {
        if !has_test_cfg(&m.attrs) {
            syn::visit::visit_item_mod(self, m);
        }
    }
    fn visit_item_impl(&mut self, i: &'ast syn::ItemImpl) {
        if !has_test_cfg(&i.attrs) {
            syn::visit::visit_item_impl(self, i);
        }
    }
    fn visit_item_use(&mut self, u: &'ast syn::ItemUse) {
        if has_test_cfg(&u.attrs) {
            return;
        }
        fn walk(t: &syn::UseTree, prefix: &mut Vec<String>, out: &mut Vec<String>) {
            match t {
                syn::UseTree::Path(p) => {
                    prefix.push(p.ident.to_string());
                    walk(&p.tree, prefix, out);
                    prefix.pop();
                }
                syn::UseTree::Name(n) => {
                    let mut v = prefix.clone();
                    v.push(n.ident.to_string());
                    if is_forbidden_env_path(&v) {
                        out.push(format!("use {}", v.join("::")));
                    }
                }
                syn::UseTree::Rename(n) => {
                    let mut v = prefix.clone();
                    v.push(n.ident.to_string());
                    if is_forbidden_env_path(&v) {
                        out.push(format!("use {} as {}", v.join("::"), n.rename));
                    }
                }
                syn::UseTree::Glob(_) => {
                    if prefix.last().is_some_and(|l| l == "env") {
                        out.push(format!("use {}::*", prefix.join("::")));
                    }
                }
                syn::UseTree::Group(g) => g.items.iter().for_each(|i| walk(i, prefix, out)),
            }
        }
        let mut out = Vec::new();
        walk(&u.tree, &mut Vec::new(), &mut out);
        for o in out {
            self.raw.push(format!("{}:{}: {o}", self.file, pos(syn::spanned::Spanned::span(u)).0));
        }
    }
    fn visit_ident(&mut self, i: &'ast proc_macro2::Ident) {
        let s = i.to_string();
        if s == "getenv" || s == "environ" {
            self.raw.push(format!("{}:{}: {s}", self.file, pos(i.span()).0));
        }
    }
    fn visit_path(&mut self, p: &'ast syn::Path) {
        let segs: Vec<String> = p.segments.iter().map(|s| s.ident.to_string()).collect();
        if is_forbidden_env_path(&segs) {
            self.raw.push(format!("{}:{}: {}", self.file, pos(syn::spanned::Spanned::span(p)).0, segs.join("::")));
        }
        syn::visit::visit_path(self, p);
    }
    fn visit_macro(&mut self, m: &'ast syn::Macro) {
        let mut out = Vec::new();
        tokens_hit(m.tokens.clone(), &mut out);
        for o in out {
            self.raw.push(format!("{}:{}: {o}", self.file, pos(syn::spanned::Spanned::span(m)).0));
        }
        syn::visit::visit_macro(self, m);
    }
    fn visit_item_fn(&mut self, f: &'ast syn::ItemFn) {
        if has_test_cfg(&f.attrs) || TRACE_WRAPPERS.contains(&f.sig.ident.to_string().as_str()) {
            return;
        }
        self.fn_body(&f.sig, &f.block, |s| syn::visit::visit_item_fn(s, f));
    }
    fn visit_impl_item_fn(&mut self, f: &'ast syn::ImplItemFn) {
        if has_test_cfg(&f.attrs) {
            return;
        }
        self.fn_body(&f.sig, &f.block, |s| syn::visit::visit_impl_item_fn(s, f));
    }
    fn visit_local(&mut self, l: &'ast syn::Local) {
        if let (syn::Pat::Ident(pi), Some(init)) = (&l.pat, &l.init) {
            let mut hits = Vec::new();
            bool_operand_traces(&init.expr, &self.trace_vars, &mut hits);
            if !hits.is_empty() {
                self.trace_vars.insert(pi.ident.to_string());
                self.bound.extend(hits);
            }
        }
        syn::visit::visit_local(self, l);
    }
    fn visit_expr_if(&mut self, i: &'ast syn::ExprIf) {
        let mut hits = Vec::new();
        traces_in(&i.cond, &self.trace_vars, &mut hits);
        hits.retain(|h| !self.proven.contains(h));
        if !hits.is_empty() {
            let mut bad = body_is_diagnostic(&i.then_branch);
            if i.else_branch.is_some() {
                bad.push("an `else` arm: the code that runs when the variable is UNSET".into());
            }
            bad.extend(cond_is_pure(&i.cond));
            if bad.is_empty() {
                self.proven.extend(hits);
            } else {
                for h in &hits {
                    self.trace_unproven.push(format!("{}:{}: trace read guards a body that is not diagnostic ({})", self.file, h.0, bad.join("; ")));
                }
                self.proven.extend(hits);
            }
        }
        syn::visit::visit_expr_if(self, i);
    }
    fn visit_expr_call(&mut self, c: &'ast syn::ExprCall) {
        if let Some(p) = trace_call(&syn::Expr::Call(c.clone())) {
            // Reported at the end if no enclosing `if` proved it: see `lint_source`.
            self.trace_unproven.push(format!("\u{0}{}:{}:{}", self.file, p.0, p.1));
        }
        syn::visit::visit_expr_call(self, c);
    }
    fn visit_expr_path(&mut self, p: &'ast syn::ExprPath) {
        if let Some(id) = p.path.get_ident() {
            if self.trace_vars.contains(&id.to_string()) {
                let q = pos(syn::spanned::Spanned::span(p));
                self.trace_unproven.push(format!("\u{0}{}:{}:{}", self.file, q.0, q.1));
            }
        }
        syn::visit::visit_expr_path(self, p);
    }
}

impl Lint {
    /// One function body. `trace_vars` is scoped to it. A unit-returning function that OPENS with `if <trace> { return; }` is
    /// a diagnostic function: everything after the guard must itself be diagnostic.
    fn fn_body(&mut self, sig: &syn::Signature, block: &syn::Block, walk: impl FnOnce(&mut Self)) {
        let saved = std::mem::take(&mut self.trace_vars);
        if matches!(sig.output, syn::ReturnType::Default) {
            if let Some(syn::Stmt::Expr(syn::Expr::If(i), _)) = block.stmts.first() {
                let mut hits = Vec::new();
                traces_in(&i.cond, &self.trace_vars, &mut hits);
                let only_return = i.else_branch.is_none()
                    && matches!(i.then_branch.stmts.as_slice(), [syn::Stmt::Expr(syn::Expr::Return(r), Some(_))] if r.expr.is_none());
                if !hits.is_empty() && only_return {
                    let rest = syn::Block { brace_token: block.brace_token, stmts: block.stmts[1..].to_vec() };
                    let mut bad = body_is_diagnostic(&rest);
                    bad.extend(cond_is_pure(&i.cond));
                    if bad.is_empty() {
                        self.proven.extend(hits);
                    }
                }
            }
        }
        walk(self);
        self.trace_vars = saved;
    }
}

/// Lint ONE source text. Returns `(raw reads, unproven trace reads)`.
pub fn lint_source(file: &str, src: &str) -> (Vec<String>, Vec<String>) {
    let ast = syn::parse_file(src).unwrap_or_else(|e| panic!("{file}: {e}"));
    let mut l = Lint { file: file.to_string(), ..Default::default() };
    l.visit_file(&ast);
    let mut unproven = Vec::new();
    for u in &l.trace_unproven {
        if let Some(rest) = u.strip_prefix('\u{0}') {
            let mut it = rest.rsplitn(3, ':');
            let col: usize = it.next().unwrap().parse().unwrap();
            let line: usize = it.next().unwrap().parse().unwrap();
            if !l.proven.contains(&(line, col)) && !l.bound.contains(&(line, col)) {
                unproven.push(format!("{file}:{line}: trace read not in the condition of an `if` (a `let`, a negation, a loop or an `else`): its answer may reach a result"));
            }
        } else {
            unproven.push(u.clone());
        }
    }
    unproven.sort();
    unproven.dedup();
    (l.raw, unproven)
}

fn workspace_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("..").join("..")
}

/// Every production `.rs` file of the five crates the shipped binaries and the lint are built from.
fn production_files() -> Vec<(String, PathBuf)> {
    let root = workspace_root();
    let mut out = Vec::new();
    for rel in ["crates/candor-report/src", "crates/candor-classify/src", "crates/candor-query/src", "crates/candor-scan/src", "src"] {
        let dir = root.join(rel);
        if !dir.is_dir() {
            continue;
        }
        for e in walkdir::WalkDir::new(&dir).into_iter().filter_map(Result::ok) {
            let p = e.path();
            if p.extension().is_some_and(|x| x == "rs") {
                let name = p.file_name().unwrap().to_string_lossy();
                if name == "tests.rs" {
                    continue;
                }
                let r = p.strip_prefix(&root).unwrap().to_string_lossy().replace('\\', "/");
                out.push((r, p.to_path_buf()));
            }
        }
    }
    out.sort();
    out
}

fn lint_workspace() -> (usize, Vec<String>, Vec<String>) {
    let files = production_files();
    let (mut raw, mut unproven) = (Vec::new(), Vec::new());
    for (rel, p) in &files {
        if rel == OWNER {
            continue;
        }
        let src = std::fs::read_to_string(p).unwrap();
        let (r, u) = lint_source(rel, &src);
        raw.extend(r);
        unproven.extend(u);
    }
    (files.len(), raw, unproven)
}

#[test]
fn no_environment_variable_is_read_outside_the_one_module() {
    let (n, raw, _) = lint_workspace();
    // VACUITY FLOOR: a gate that found no source proves nothing. The five crates have well over 20 production files.
    assert!(n >= 20, "the lint found only {n} production source files — it is not looking at the engine");
    assert!(raw.is_empty(), "raw environment reads outside {OWNER} (route each through candor_report::env::{{switch, trace, operator}}):\n{}", raw.join("\n"));
}

#[test]
fn every_trace_read_is_proven_diagnostic_by_its_position() {
    let (n, _, unproven) = lint_workspace();
    assert!(n >= 20);
    assert!(unproven.is_empty(), "trace reads whose answer may reach a result — reclassify each as candor_report::env::switch:\n{}", unproven.join("\n"));
}

#[test]
fn the_lint_still_sees_the_trace_reads_it_exists_to_prove() {
    // The census floor for claim 2: if the walker stopped recognising `trace` calls, the test above would pass over an
    // engine full of them. Count them directly in the source text and require the AST to have proven nearly as many.
    let files = production_files();
    let mut text_count = 0usize;
    for (rel, p) in &files {
        if rel == OWNER {
            continue;
        }
        text_count += std::fs::read_to_string(p).unwrap().matches("env::trace(").count();
    }
    assert!(text_count >= 300, "expected hundreds of trace reads in the engine, found {text_count}");
}

// ---------------------------------------------------------------------------------------------------------------------
// CALIBRATION — the gate shown able to fail.
// ---------------------------------------------------------------------------------------------------------------------

#[test]
fn the_lint_fires_on_every_seeded_spelling() {
    let raw_spellings: &[(&str, &str)] = &[
        ("full path var", "fn f() { let _ = std::env::var(\"CANDOR_X\"); }"),
        ("full path var_os", "fn f() { let _ = std::env::var_os(\"CANDOR_X\"); }"),
        ("bare env:: after use std::env", "use std::env;\nfn f() { let _ = env::var(\"CANDOR_X\"); }"),
        ("vars enumeration", "fn f() { for _ in std::env::vars() {} }"),
        ("vars_os enumeration", "fn f() { for _ in std::env::vars_os() {} }"),
        ("use of the function", "use std::env::var;\nfn f() { let _ = var(\"X\"); }"),
        ("use as alias", "use std::env::var_os as v;\nfn f() { let _ = v(\"X\"); }"),
        ("use in a group", "use std::env::{args, var};\nfn f() {}"),
        ("use glob", "use std::env::*;\nfn f() {}"),
        ("method reference", "fn f() { let g = std::env::var; let _ = g(\"X\"); }"),
        ("computed name", "fn f() { let n = format!(\"CANDOR_{}\", 1); let _ = std::env::var(&n); }"),
        ("inside a macro argument", "fn f() { eprintln!(\"{:?}\", std::env::var(\"X\")); }"),
        ("inside a nested macro", "fn f() { assert!(std::env::var(\"X\").is_ok()); }"),
        ("a closure", "fn f() { let c = || std::env::var_os(\"X\"); let _ = c(); }"),
        ("a static initialiser", "static S: std::sync::LazyLock<bool> = std::sync::LazyLock::new(|| std::env::var_os(\"X\").is_some());"),
        ("a const-fn-adjacent impl", "struct S; impl S { fn g(&self) -> bool { std::env::var(\"X\").is_ok() } }"),
        ("libc getenv", "extern \"C\" { fn getenv(p: *const u8) -> *const u8; }\nfn f() {}"),
        ("libc getenv call", "fn f() { unsafe { libc::getenv(std::ptr::null()); } }"),
        ("the environ symbol", "extern \"C\" { static environ: *const u8; }\nfn f() {}"),
        ("crate-qualified path", "fn f() { let _ = ::std::env::var(\"X\"); }"),
        ("core-qualified via a module alias", "mod m { pub use std::env::var_os; }\nfn f() {}"),
    ];
    for (name, src) in raw_spellings {
        let (raw, _) = lint_source("seed.rs", src);
        assert!(!raw.is_empty(), "the lint did NOT fire on the seeded spelling `{name}`:\n{src}");
    }
    // …and it must NOT fire on the sanctioned routes or on `args`/`temp_dir`/compile-time `env!`.
    let clean = "fn f() { let _ = candor_report::env::operator(\"X\"); let _ = candor_report::env::switch(\"Y\"); let _ = std::env::args(); \
                 let _ = std::env::temp_dir(); let _ = std::env::current_dir(); const V: &str = env!(\"CARGO_PKG_VERSION\"); }";
    let (raw, unproven) = lint_source("clean.rs", clean);
    assert!(raw.is_empty() && unproven.is_empty(), "false positive on sanctioned reads: {raw:?} {unproven:?}");
    // #[cfg(test)] code is not production code.
    let (raw, _) = lint_source("t.rs", "#[cfg(test)]\nmod tests { fn f() { let _ = std::env::var(\"X\"); } }");
    assert!(raw.is_empty(), "a #[cfg(test)] module must be skipped: {raw:?}");
}

#[test]
fn the_trace_proof_fires_on_every_seeded_misuse() {
    let proven_ok = [
        "fn f() { if candor_report::env::trace(\"X_INSTR\") { eprintln!(\"hit\"); } }",
        "fn f() { let on = candor_report::env::trace(\"X\"); if on { eprintln!(\"hit\"); } }",
        "fn f(b: bool) { if b || candor_report::env::trace(\"X\") { eprintln!(\"hit\"); } }",
        "fn f() { let instr = |k: &str| { if candor_report::env::trace(\"X\") { eprintln!(\"{k}\"); } }; instr(\"a\"); }",
        "fn f() { if !candor_report::env::trace(\"X\") { eprintln!(\"a\"); } }",
        "fn f(x: u8) { if x > 1 && candor_report::env::trace(\"X_INSTR\") { let n = x + 1; eprintln!(\"{n}\"); } }",
        "fn f() { if candor_report::env::trace(\"X\") { let mut v = Vec::new(); v.push(1); eprintln!(\"{v:?}\"); } }",
    ];
    for s in proven_ok {
        let (_, un) = lint_source("ok.rs", s);
        assert!(un.is_empty(), "false positive on a diagnostic read: {un:?}\n{s}");
    }
    let misuse: &[(&str, &str)] = &[
        ("bound by let, then used as a value", "fn f() -> u8 { let on = candor_report::env::trace(\"X\"); let n = if on { 1 } else { 0 }; n }"),
        ("bound by let, then passed on", "fn g(_b: bool) {}\nfn f() { let on = candor_report::env::trace(\"X\"); g(on); }"),
        ("bound by let, then gating a mutation", "fn f(v: &mut Vec<u8>) { let on = candor_report::env::trace(\"X\"); if on { v.push(1); } }"),
        ("negated, guarding a mutation", "fn f(m: &mut Vec<u8>) { if !candor_report::env::trace(\"X\") { m.push(1); } }"),
        ("or-ed, guarding a mutation", "fn f(b: bool, m: &mut Vec<u8>) { if b || candor_report::env::trace(\"X\") { m.push(1); } }"),
        ("short-circuit mutation in the condition", "fn f(m: &mut Vec<u8>) { if candor_report::env::trace(\"X\") && { m.push(1); true } { eprintln!(\"a\"); } }"),
        ("else arm", "fn f(b: bool) -> u8 { if b { 1 } else if candor_report::env::trace(\"X\") { 2 } else { 3 } }"),
        ("else arm doing real work", "fn f(m: &mut Vec<u8>) { if candor_report::env::trace(\"X\") { eprintln!(\"a\"); } else { m.push(2); } }"),
        ("returned", "fn f() -> bool { candor_report::env::trace(\"X\") }"),
        ("loop condition", "fn f() { while candor_report::env::trace(\"X\") { eprintln!(\"a\"); } }"),
        ("body returns", "fn f() -> u8 { if candor_report::env::trace(\"X\") { return 1; } 2 }"),
        ("body inserts into outer state", "fn f(m: &mut std::collections::HashSet<u8>) { if candor_report::env::trace(\"X\") { m.insert(1); } }"),
        ("body assigns outer state", "fn f(x: &mut u8) { if candor_report::env::trace(\"X\") { *x = 1; } }"),
        ("body compound-assigns outer state", "fn f(x: &mut u8) { if candor_report::env::trace(\"X\") { *x += 1; } }"),
        ("body calls an unknown macro", "fn f() { if candor_report::env::trace(\"X\") { panic!(\"boom\"); } }"),
        ("body uses ?", "fn f() -> Option<u8> { if candor_report::env::trace(\"X\") { let _a = Some(1u8)?; } Some(2) }"),
        ("wrapper in a let", "fn f() { let r = reach_debug(); let _ = r; }"),
        ("guard whose answer is used as a value", "fn f() -> u8 { let n = if candor_report::env::trace(\"X\") { 1 } else { 0 }; n }"),
    ];
    for (name, src) in misuse {
        let (_, un) = lint_source("seed.rs", src);
        assert!(!un.is_empty(), "the trace proof did NOT fire on the seeded misuse `{name}`:\n{src}");
    }
}

#[test]
fn the_lint_fires_on_a_real_file_with_one_read_appended() {
    // A shipped file, unmodified, is clean; the same file with ONE raw read appended is not. This is the calibration that
    // separates "the walker is healthy" from "the walker reads nothing".
    let files = production_files();
    let (rel, p) = files.iter().find(|(r, _)| r == "crates/candor-scan/src/config.rs").expect("config.rs is part of the engine");
    let src = std::fs::read_to_string(p).unwrap();
    let (raw0, un0) = lint_source(rel, &src);
    assert!(raw0.is_empty() && un0.is_empty(), "the shipped file must be clean: {raw0:?} {un0:?}");
    let appended = format!("{src}\nfn appended_by_the_calibration() -> bool {{ std::env::var_os(\"CANDOR_R9999_OFF\").is_some() }}\n");
    let (raw1, _) = lint_source(rel, &appended);
    assert_eq!(raw1.len(), 1, "exactly the appended read must be reported: {raw1:?}");
    // …and one `trace` read in a position that is not diagnostic.
    let appended = format!("{src}\nfn appended_by_the_calibration2() -> bool {{ candor_report::env::trace(\"CANDOR_R9999_INSTR\") }}\n");
    let (_, un1) = lint_source(rel, &appended);
    assert_eq!(un1.len(), 1, "exactly the appended trace read must be reported: {un1:?}");
    // …and the owner module itself is the only file that reads the environment: it must be present and non-trivial, or
    // the exemption would silently exempt nothing.
    let owner = files.iter().find(|(r, _)| r == OWNER).expect("env.rs exists");
    let osrc = std::fs::read_to_string(&owner.1).unwrap();
    let (oraw, _) = lint_source(OWNER, &osrc);
    assert!(oraw.len() >= 3, "env.rs is the one reader: the lint must see its reads (saw {oraw:?}) or the exemption is vacuous");
}

#[allow(dead_code)]
fn _unused(_: &Path) {}
