//! candor-scan — a STABLE-Rust effect scanner. Produces the same candor report JSON the nightly
//! `rustc_private` lint does, but with a purely syntactic backend: walk the crate's `.rs` files, parse
//! with `syn`, build a name-resolved-enough call graph, classify path-qualified calls with the shared
//! `candor-classify`, and propagate transitively. No nightly, no `rustc-dev`, no dylint — `cargo install`
//! and run anywhere.
//!
//! HONEST PRECISION TRADE vs the lint. This is syntactic, so it sees what's written, not what's
//! resolved. It CATCHES: path-qualified effect calls (`std::fs::read`, `reqwest::Client::execute`,
//! `Command::new`), including `use`-aliased prefixes; and intra-crate calls (matched by name) for
//! transitive propagation. It MISSES (silently — it does NOT emit `Unknown`): effects reached only
//! through a method call whose receiver type isn't path-qualified, trait-object dispatch, closures /
//! fn-pointers, macros, and cross-crate propagation by stable identity. So on resolution-heavy code it
//! under-reports relative to the lint. Use the lint when you need the soundness contract; use this when
//! you need zero-friction, stable, installable triage. Shares the lint's classifier — one source of truth.
//!
//! CALL RESOLUTION. The local call graph is name-resolved, not type-resolved. A `Type::method` call is
//! matched on its qualified tail (`RequestBuilder::new`), which keeps same-named methods on different
//! types distinct; a bare `.method()` call (no type qualifier) is linked only when the name is
//! UNAMBIGUOUS across the crate. We deliberately do NOT link a many-way-ambiguous bare name: on a real
//! crate that would link every `.new()` to all 100+ `*::new` defs and smear one type's effect across the
//! whole graph (a false-positive blow-up). Under-reporting an ambiguous edge is the honest failure mode.
//!
//! Usage:  candor-scan [<crate-dir>] [--out <prefix>] [--json] [--include-tests]
//!   default dir = ".", default prefix = "<dir>/.candor/report"; writes <prefix>.<crate>.scan.json (+ a
//!   callgraph sidecar so `cargo candor callers <fn>` works on the stable report too). `--json` prints
//!   the report to stdout instead. By DEFAULT only the crate's library/binary source is scanned —
//!   `tests/`, `benches/`, `examples/`, `test/`, `build.rs`, and `#[cfg(test)]` modules are skipped, so
//!   the report describes what the CRATE does, not what its harness does (`--include-tests` keeps them).
//!   See eval/calibration for accuracy on 35 real crates.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::path::Path;

use candor_report::ReportEntry;
use syn::visit::Visit;

/// A call observed in a function body: the (use-expanded) path string and the leaf name.
struct Call {
    path: String,            // "std::fs::read", "compute_price", "pricing::priced"
    leaf: String,            // last segment
    str_arg: Option<String>, // first string-literal argument (host/cmd/path detail)
    /// Synthesized from receiver-type inference (`reqwest::Client::send` from `client.send()`). Used for
    /// external-crate classification ONLY — excluded from local call-graph edges, since its `Type::method`
    /// tail could spuriously link to a same-named LOCAL method the call doesn't actually target.
    typed: bool,
}

/// One function the scan found: its module-qualified name, where, and the calls in its body.
struct FnInfo {
    qual: String,
    leaf: String,
    loc: String,
    calls: Vec<Call>,
    /// The body invoked a callable the syntactic scan can't see through — a closure / fn-pointer value
    /// (`(cb)()`, `arr[i]()`, a local bound to a closure). The target could perform ANY effect, so the
    /// function can't honestly be certified pure: it's marked `Unknown` (matching the nightly lint's
    /// soundness fallback) rather than silently reported clean.
    unresolved: bool,
}

/// `struct-name-leaf -> { field -> expanded-type-path }`, e.g. `App -> { http: reqwest::Client }`.
/// Built crate-wide in a pre-pass so a method call on `self.http` can be resolved to its type and
/// classified by the existing per-crate method rules (`reqwest::Client::execute` -> Net).
type FieldIndex = HashMap<String, HashMap<String, String>>;

/// `fn-leaf -> expanded return-type-path`, e.g. `create_pool -> sqlx::Pool` (Result/Option unwrapped).
/// Lets type inference flow through a LOCAL factory function: `let p = create_pool()?; p.fetch_one(q)`.
/// Only UNAMBIGUOUS leaves are kept — a name with two different return types across the crate is dropped
/// (no guess), like the unique-leaf call-graph rule.
type ReturnIndex = HashMap<String, String>;

struct CallCollector<'a> {
    uses: &'a HashMap<String, String>,
    /// local variable / param / `self` -> expanded type path, grown as `let`s are visited in order.
    vars: HashMap<String, String>,
    fields: &'a FieldIndex,
    returns: &'a ReturnIndex,
    calls: Vec<Call>,
    /// locals bound to a closure (`let f = |..| ..`), so a later `f()` is recognised as a closure
    /// invocation the scan can't see through — not a call to a free fn named `f`.
    closure_vars: std::collections::HashSet<String>,
    /// set once the body invokes a callable we can't resolve (see `FnInfo::unresolved`).
    unresolved: bool,
}

fn path_to_string(p: &syn::Path) -> String {
    p.segments.iter().map(|s| s.ident.to_string()).collect::<Vec<_>>().join("::")
}

/// The (use-expanded) type path of a `syn::Type`, ignoring references and generic args:
/// `&reqwest::Client` -> `reqwest::Client`, `Pool<Postgres>` -> `sqlx::Pool` (via `uses`). `None` for
/// non-nameable types (impl Trait, tuples, …) where there's nothing to classify a method against.
fn type_path(ty: &syn::Type, uses: &HashMap<String, String>) -> Option<String> {
    match ty {
        syn::Type::Reference(r) => type_path(&r.elem, uses),
        syn::Type::Paren(p) => type_path(&p.elem, uses),
        syn::Type::Group(g) => type_path(&g.elem, uses),
        syn::Type::Path(p) => Some(expand(&path_to_string(&p.path), uses)),
        _ => None,
    }
}

/// Constructor-style associated function names: `let x = Foo::new(..)` (or `::connect().await?`) means
/// `x: Foo`. Conservative set of names that return `Self` (or `Result<Self>`), so the inferred type is
/// reliable. A non-constructor assoc call (`Foo::parse`) is NOT treated as producing a `Foo`.
fn is_ctor(name: &str) -> bool {
    matches!(
        name,
        "new" | "default" | "builder" | "with_capacity" | "connect" | "open" | "init" | "from"
            | "from_path" | "from_str" | "with_config" | "create"
    )
}

/// The type a call expression produces (peeling `&`/`(..)`/`?`/`.await`), by two routes:
///   1. a constructor `Path::ctor(..)` -> the `Path` type (`reqwest::Client::new()` -> `reqwest::Client`);
///   2. a LOCAL free function whose return type the pre-pass recorded (`create_pool()` -> `sqlx::Pool`).
/// Returns the expanded type path. `returns` is the crate-wide fn-leaf -> return-type index.
fn ctor_type(expr: &syn::Expr, uses: &HashMap<String, String>, returns: &ReturnIndex) -> Option<String> {
    match expr {
        syn::Expr::Reference(r) => ctor_type(&r.expr, uses, returns),
        syn::Expr::Paren(p) => ctor_type(&p.expr, uses, returns),
        syn::Expr::Try(t) => ctor_type(&t.expr, uses, returns),
        syn::Expr::Await(a) => ctor_type(&a.base, uses, returns),
        syn::Expr::Call(c) => {
            let syn::Expr::Path(p) = &*c.func else { return None };
            let full = path_to_string(&p.path);
            let leaf = full.rsplit("::").next().unwrap_or(&full);
            if let Some((ty, last)) = full.rsplit_once("::") {
                // `Type::ctor(..)` yields `Type` — but ONLY when the receiver is a TYPE, not a module.
                // Require the receiver's last segment to be type-like (UpperCamel), so `Client::new` →
                // Client but `serde_json::from_str` (module path) does NOT infer the module as a type.
                let ty_leaf = ty.rsplit("::").next().unwrap_or(ty);
                let type_like = ty_leaf.chars().next().is_some_and(|c| c.is_uppercase());
                if is_ctor(last) && type_like {
                    return Some(expand(ty, uses));
                }
            }
            // a local factory function call — its recorded (unambiguous) return type
            returns.get(leaf).cloned()
        }
        _ => None,
    }
}

/// Peel `Result<T, _>` / `Option<T>` / `io::Result<T>` to the inner `T` — a fallible constructor's
/// useful type is what it yields after `?`. Returns the inner type, or the type unchanged.
fn unwrap_result_option(ty: &syn::Type) -> &syn::Type {
    let syn::Type::Path(p) = ty else { return ty };
    let Some(seg) = p.path.segments.last() else { return ty };
    if matches!(seg.ident.to_string().as_str(), "Result" | "Option" | "IoResult") {
        if let syn::PathArguments::AngleBracketed(args) = &seg.arguments {
            if let Some(syn::GenericArgument::Type(inner)) = args.args.first() {
                return inner;
            }
        }
    }
    ty
}

/// Expand a call path against this file's `use` map: if the first segment is the last segment of some
/// `use a::b::Name`, replace it with the full `a::b::Name`. Turns `fs::read` → `std::fs::read`,
/// `Command::new` → `std::process::Command::new`. `crate`/`self`/`super` prefixes are stripped (local).
fn expand(path: &str, uses: &HashMap<String, String>) -> String {
    let mut segs: Vec<&str> = path.split("::").collect();
    // A path rooted at `crate`/`self`/`super` is EXPLICITLY crate-local — it is NOT subject to the file's
    // `use` aliases, so after stripping the prefix we return it as-is. (Re-applying `uses` here would let
    // a `use other::config;` import hijack a local `crate::config::load` call.)
    let rooted_local = matches!(segs.first().copied(), Some("crate" | "self" | "super"));
    while matches!(segs.first().copied(), Some("crate" | "self" | "super")) {
        segs.remove(0);
    }
    if segs.is_empty() {
        return path.to_string();
    }
    if !rooted_local {
        if let Some(full) = uses.get(segs[0]) {
            let rest = &segs[1..];
            return if rest.is_empty() { full.clone() } else { format!("{full}::{}", rest.join("::")) };
        }
    }
    segs.join("::")
}

fn first_str_lit(args: &syn::punctuated::Punctuated<syn::Expr, syn::token::Comma>) -> Option<String> {
    for a in args {
        if let syn::Expr::Lit(syn::ExprLit { lit: syn::Lit::Str(s), .. }) = a {
            let v = s.value();
            if !v.trim().is_empty() {
                return Some(v);
            }
        }
    }
    None
}

impl<'a> CallCollector<'a> {
    /// Best-effort type of a method-call receiver, so `recv.method()` can be classified as
    /// `Type::method`. Resolves a bare variable/param/`self` (via `vars`), a `base.field` access (via
    /// the struct `FieldIndex`), and peels `&`/`(..)`/`?`/`.await`. For a method CHAIN
    /// (`client.get(url).send()`) it returns the BASE receiver's type — the chain stays within one
    /// crate's builder family, and the classifier verb-gates per crate, so attributing the terminal
    /// verb to the base type is correct in practice (`reqwest::Client` + `::send` -> Net).
    fn resolve_recv_type(&self, expr: &syn::Expr) -> Option<String> {
        match expr {
            syn::Expr::Reference(r) => self.resolve_recv_type(&r.expr),
            syn::Expr::Paren(p) => self.resolve_recv_type(&p.expr),
            syn::Expr::Group(g) => self.resolve_recv_type(&g.expr),
            syn::Expr::Try(t) => self.resolve_recv_type(&t.expr),
            syn::Expr::Await(a) => self.resolve_recv_type(&a.base),
            syn::Expr::MethodCall(m) => {
                // Walk through the chain to the base receiver's type. We deliberately do NOT consult the
                // return-type index by method NAME here: a method name doesn't identify the method, so a
                // single crate-wide `fn conn() -> redis::Connection` would otherwise hijack every
                // `x.conn().get()` on an unrelated `x`, fabricating a Db effect. The return index is used
                // only for free-function factory calls (the `Expr::Call` arm via `ctor_type`), where the
                // name does identify the function.
                self.resolve_recv_type(&m.receiver)
            }
            syn::Expr::Path(p) => {
                let name = p.path.get_ident()?.to_string();
                self.vars.get(&name).cloned()
            }
            syn::Expr::Field(f) => {
                let base = self.resolve_recv_type(&f.base)?;
                let syn::Member::Named(field) = &f.member else { return None };
                let base_leaf = base.rsplit("::").next().unwrap_or(&base);
                self.fields.get(base_leaf)?.get(&field.to_string()).cloned()
            }
            syn::Expr::Call(_) => ctor_type(expr, self.uses, self.returns),
            _ => None,
        }
    }
}

impl<'a, 'ast> Visit<'ast> for CallCollector<'a> {
    fn visit_expr_call(&mut self, node: &'ast syn::ExprCall) {
        // Peel `(..)`/`{..}` wrappers around the callee so `(f)()` is treated like `f()`.
        let mut func = &*node.func;
        loop {
            match func {
                syn::Expr::Paren(p) => func = &p.expr,
                syn::Expr::Group(g) => func = &g.expr,
                _ => break,
            }
        }
        match func {
            syn::Expr::Path(p) => {
                // A local bound to a closure — `let f = |..| ..` — has its body walked LEXICALLY by this
                // same visitor, so `f()` adds nothing and is NOT a blind spot. (Skip recording it as a
                // phantom call to a free fn `f`, too.) Any other path is a normal call.
                if !p.path.get_ident().is_some_and(|id| self.closure_vars.contains(&id.to_string())) {
                    let path = expand(&path_to_string(&p.path), self.uses);
                    let leaf = path.rsplit("::").next().unwrap_or(&path).to_string();
                    self.calls.push(Call { path, leaf, str_arg: first_str_lit(&node.args), typed: false });
                }
            }
            // The callee is a COMPUTED value, not a path or a visible local closure: `(self.handler)()`,
            // `arr[i]()`, `make_cb()()`. The scan can't identify the target or see its body — it could
            // perform any effect — so the enclosing function can't be certified pure: honest `Unknown`.
            _ => self.unresolved = true,
        }
        syn::visit::visit_expr_call(self, node);
    }
    fn visit_expr_method_call(&mut self, node: &'ast syn::ExprMethodCall) {
        let leaf = node.method.to_string();
        let str_arg = first_str_lit(&node.args);
        // Leaf-only call: feeds the intra-crate call graph and bare-leaf classification.
        self.calls.push(Call { path: leaf.clone(), leaf: leaf.clone(), str_arg: str_arg.clone(), typed: false });
        // Typed call: if the receiver's type resolves, form `Type::method` so the existing per-crate
        // method rules (reqwest/sqlx/redis/…) — unreachable from a bare method name — can fire. This is
        // the method-dispatch frontier: light, local type inference, no compiler.
        //
        // EXTERNAL types only. The external-crate rules are verb-precise (`ends_with("::execute")`), so
        // they're safe to apply to an inferred method call. The std rules are coarse PREFIX matches
        // (`std::fs::`, `std::process::Command`) written for free-function/constructor calls — applied to
        // arbitrary method calls they mis-fire on pure ones (`File::as_raw_fd`, `Command::arg`). So skip
        // std/core/alloc receivers: their free-function effects are already caught path-qualified, and an
        // honest miss on a std method beats a wrong effect on a pure one.
        if let Some(ty) = self.resolve_recv_type(&node.receiver) {
            let cr = ty.split("::").next().unwrap_or("");
            if !matches!(cr, "std" | "core" | "alloc") {
                let path = format!("{ty}::{leaf}");
                self.calls.push(Call { path, leaf: leaf.clone(), str_arg, typed: true });
            }
        }
        syn::visit::visit_expr_method_call(self, node);
    }
    fn visit_local(&mut self, node: &'ast syn::Local) {
        // Record `let x: T = ..` (annotated) and `let x = T::new(..)` (constructor) so later method
        // calls on `x` resolve. Visited in source order, before any use of `x` (Rust requires it).
        if let syn::Pat::Type(pt) = &node.pat {
            if let syn::Pat::Ident(id) = &*pt.pat {
                if let Some(ty) = type_path(&pt.ty, self.uses) {
                    self.vars.insert(id.ident.to_string(), ty);
                }
            }
        } else if let syn::Pat::Ident(id) = &node.pat {
            if let Some(init) = &node.init {
                if matches!(&*init.expr, syn::Expr::Closure(_)) {
                    // `let f = |..| ..` — remember `f` so a later `f()` is seen as a closure call.
                    self.closure_vars.insert(id.ident.to_string());
                } else if let Some(ty) = ctor_type(&init.expr, self.uses, self.returns) {
                    self.vars.insert(id.ident.to_string(), ty);
                }
            }
        }
        syn::visit::visit_local(self, node);
    }
    fn visit_macro(&mut self, node: &'ast syn::Macro) {
        // syn does not parse a macro's body, so every call hidden inside one is invisible by default —
        // a real miss on crates that route effectful calls through a macro (git2 wraps EVERY libgit2 FFI
        // call in `try_call!(raw::git_...())`; `println!("{}", f())` hides `f`). Best-effort: parse the
        // token stream as comma-separated expressions and walk any that parse. If the body isn't
        // expression syntax (`quote!{}`, `matches!(x, Pat)`, macro_rules arms), parsing fails and we skip
        // — so this only ever ADDS visibility, never breaks. Owned exprs, so visit a local copy.
        let parser = syn::punctuated::Punctuated::<syn::Expr, syn::Token![,]>::parse_terminated;
        if let Ok(exprs) = syn::parse::Parser::parse2(parser, node.tokens.clone()) {
            for e in &exprs {
                self.visit_expr(e);
            }
        }
    }
}

/// True if the item carries any `#[cfg(...)]` attribute (conditionally compiled).
fn has_cfg(attrs: &[syn::Attribute]) -> bool {
    attrs.iter().any(|a| a.path().is_ident("cfg"))
}

/// True if a file stem names a conventional `#[cfg(test)] mod` FILE module (`tests.rs`, `foo_tests.rs`,
/// `foo_test.rs`) — whose test-ness is declared at the `mod` site, invisible when walking the file.
fn is_test_file_stem(stem: &str) -> bool {
    stem == "tests" || stem == "test" || stem.ends_with("_tests") || stem.ends_with("_test")
}

/// True if `test` is POSITIVELY required by this cfg predicate node (recursing through `any`/`all` to
/// any depth, but NOT through `not` — a `test` under `not()` means "compile when NOT testing", i.e.
/// production code that must NOT be skipped).
fn cfg_meta_requires_test(m: &syn::meta::ParseNestedMeta) -> bool {
    if m.path.is_ident("test") {
        return true;
    }
    if m.path.is_ident("any") || m.path.is_ident("all") {
        let mut inner_test = false;
        // (the parse may error on a non-meta tail like a bare `not(unix)` group; `test` is recorded
        // before that, and the error is swallowed — we only care whether a positive `test` was seen.)
        let _ = m.parse_nested_meta(|inner| {
            if cfg_meta_requires_test(&inner) {
                inner_test = true;
            }
            Ok(())
        });
        return inner_test;
    }
    false // `not(...)`, `feature = "..."`, target predicates, etc.
}

/// True if an item carries a `#[cfg(...)]` that POSITIVELY requires `test` — a test-only module the
/// default scan skips, since its effects describe the crate's TESTS, not the crate. `#[cfg(not(test))]`
/// (production code) and `#[cfg(all(unix, not(test)))]` are correctly NOT treated as test.
fn is_cfg_test(attrs: &[syn::Attribute]) -> bool {
    attrs.iter().any(|a| {
        a.path().is_ident("cfg") && {
            let mut found = false;
            let _ = a.parse_nested_meta(|m| {
                if cfg_meta_requires_test(&m) {
                    found = true;
                }
                Ok(())
            });
            found
        }
    })
}

#[allow(clippy::too_many_arguments)]
fn scan_items(
    items: &[syn::Item],
    modpath: &str,
    file: &str,
    include_tests: bool,
    fields: &FieldIndex,
    returns: &ReturnIndex,
    uses: &mut HashMap<String, String>,
    out: &mut Vec<FnInfo>,
) {
    for it in items {
        if let syn::Item::Use(u) = it {
            collect_use(&u.tree, String::new(), uses);
        }
    }
    let qual = |name: &str| if modpath.is_empty() { name.to_string() } else { format!("{modpath}::{name}") };
    for it in items {
        match it {
            syn::Item::Fn(f) => {
                let n = f.sig.ident.to_string();
                out.push(fninfo(&n, &qual(&n), file, &f.sig, &f.block, None, uses, fields, returns));
            }
            syn::Item::Impl(im) => {
                let tyname = impl_type_name(&im.self_ty);
                for ii in &im.items {
                    if let syn::ImplItem::Fn(m) = ii {
                        let n = m.sig.ident.to_string();
                        let q = match &tyname {
                            Some(t) => qual(&format!("{t}::{n}")),
                            None => qual(&n),
                        };
                        out.push(fninfo(&n, &q, file, &m.sig, &m.block, tyname.as_deref(), uses, fields, returns));
                    }
                }
            }
            syn::Item::Mod(m) => {
                if !include_tests && is_cfg_test(&m.attrs) {
                    continue; // a #[cfg(test)] module — its effects are the tests', not the crate's
                }
                if let Some((_, inner)) = &m.content {
                    let sub = qual(&m.ident.to_string());
                    let mut subuses = uses.clone();
                    scan_items(inner, &sub, file, include_tests, fields, returns, &mut subuses, out);
                }
            }
            _ => {}
        }
    }
}

/// Seed a function's variable→type map from its parameters (`fn h(c: &reqwest::Client)`) and, for an
/// impl method, `self` → the impl type. These are the most reliable type facts available syntactically.
fn seed_vars(sig: &syn::Signature, self_ty: Option<&str>, uses: &HashMap<String, String>) -> HashMap<String, String> {
    let mut vars = HashMap::new();
    if let Some(t) = self_ty {
        vars.insert("self".to_string(), t.to_string());
    }
    for arg in &sig.inputs {
        if let syn::FnArg::Typed(pt) = arg {
            if let syn::Pat::Ident(id) = &*pt.pat {
                if let Some(ty) = type_path(&pt.ty, uses) {
                    vars.insert(id.ident.to_string(), ty);
                }
            }
        }
    }
    vars
}

#[allow(clippy::too_many_arguments)]
fn fninfo(
    leaf: &str,
    qual: &str,
    file: &str,
    sig: &syn::Signature,
    block: &syn::Block,
    self_ty: Option<&str>,
    uses: &HashMap<String, String>,
    fields: &FieldIndex,
    returns: &ReturnIndex,
) -> FnInfo {
    let vars = seed_vars(sig, self_ty, uses);
    let mut c = CallCollector {
        uses,
        vars,
        fields,
        returns,
        calls: Vec::new(),
        closure_vars: std::collections::HashSet::new(),
        unresolved: false,
    };
    for stmt in &block.stmts {
        c.visit_stmt(stmt);
    }
    FnInfo {
        qual: qual.to_string(),
        leaf: leaf.to_string(),
        loc: file.to_string(),
        calls: c.calls,
        unresolved: c.unresolved,
    }
}

/// Record `fn-leaf -> return type` into `rets`, tracking ambiguity: a leaf seen with two different
/// return types is set to `None` (dropped later), so only UNAMBIGUOUS names survive. Result/Option are
/// unwrapped to the success type.
fn record_return(sig: &syn::Signature, uses: &HashMap<String, String>, rets: &mut HashMap<String, Option<String>>) {
    let syn::ReturnType::Type(_, ty) = &sig.output else { return };
    let Some(tp) = type_path(unwrap_result_option(ty), uses) else { return };
    let leaf = sig.ident.to_string();
    match rets.get(&leaf) {
        None => {
            rets.insert(leaf, Some(tp));
        }
        Some(Some(prev)) if *prev != tp => {
            rets.insert(leaf, None); // conflicting return types — ambiguous, drop
        }
        _ => {}
    }
}

/// Pre-pass: index struct field types (`App -> { http: reqwest::Client }`) AND function return types
/// (`create_pool -> sqlx::Pool`), expanded via each module's `use` map. Recurses into modules like
/// `scan_items`. Field index keyed by struct leaf; return map keyed by fn leaf (ambiguous names dropped
/// by the caller). A name collision is rare and at worst yields a wrong (still verb-gated) classify.
fn collect_decls(
    items: &[syn::Item],
    include_tests: bool,
    uses: &mut HashMap<String, String>,
    fields: &mut FieldIndex,
    rets: &mut HashMap<String, Option<String>>,
) {
    for it in items {
        if let syn::Item::Use(u) = it {
            collect_use(&u.tree, String::new(), uses);
        }
    }
    for it in items {
        match it {
            syn::Item::Struct(s) => {
                if let syn::Fields::Named(named) = &s.fields {
                    let entry = fields.entry(s.ident.to_string()).or_default();
                    for f in &named.named {
                        // Skip `#[cfg(...)]`-gated fields: they aren't unconditionally present, so
                        // inferring effects through them mis-fires. (tokio's `resource_span:
                        // tracing::Span`, gated on the off-by-default `tracing` feature, otherwise made
                        // every `self.resource_span.in_scope(..)` read as Log — 452 phantom functions.)
                        if has_cfg(&f.attrs) {
                            continue;
                        }
                        if let Some(name) = &f.ident {
                            if let Some(ty) = type_path(&f.ty, uses) {
                                entry.insert(name.to_string(), ty);
                            }
                        }
                    }
                }
            }
            syn::Item::Fn(f) => record_return(&f.sig, uses, rets),
            syn::Item::Impl(im) => {
                for ii in &im.items {
                    if let syn::ImplItem::Fn(m) = ii {
                        record_return(&m.sig, uses, rets);
                    }
                }
            }
            syn::Item::Mod(m) => {
                // Skip `#[cfg(test)]` modules here too (Pass B / scan_items already does): otherwise a
                // test module's struct fields and fn return types leak into the crate-wide index and get
                // used to type PRODUCTION code (e.g. `struct App { http: MockClient }` in `mod tests`
                // colliding with the real App).
                if !include_tests && is_cfg_test(&m.attrs) {
                    continue;
                }
                if let Some((_, inner)) = &m.content {
                    let mut subuses = uses.clone();
                    collect_decls(inner, include_tests, &mut subuses, fields, rets);
                }
            }
            _ => {}
        }
    }
}

fn impl_type_name(ty: &syn::Type) -> Option<String> {
    if let syn::Type::Path(p) = ty {
        return p.path.segments.last().map(|s| s.ident.to_string());
    }
    None
}

fn collect_use(tree: &syn::UseTree, prefix: String, out: &mut HashMap<String, String>) {
    let join = |p: &str, s: &str| if p.is_empty() { s.to_string() } else { format!("{p}::{s}") };
    match tree {
        syn::UseTree::Path(p) => collect_use(&p.tree, join(&prefix, &p.ident.to_string()), out),
        syn::UseTree::Name(n) => {
            out.insert(n.ident.to_string(), join(&prefix, &n.ident.to_string()));
        }
        syn::UseTree::Rename(r) => {
            out.insert(r.rename.to_string(), join(&prefix, &r.ident.to_string()));
        }
        syn::UseTree::Group(g) => {
            for t in &g.items {
                collect_use(t, prefix.clone(), out);
            }
        }
        syn::UseTree::Glob(_) => {}
    }
}

/// Module path implied by a file's location under `src/` (root files → ""; `foo.rs`/`foo/mod.rs` →
/// "foo"; `foo/bar.rs` → "foo::bar"). Best-effort mirror of file-based module resolution.
fn module_path(rel: &Path) -> String {
    let mut comps: Vec<String> =
        rel.components().filter_map(|c| c.as_os_str().to_str().map(String::from)).collect();
    if comps.first().map(String::as_str) == Some("src") {
        comps.remove(0);
    }
    if let Some(last) = comps.last_mut() {
        let stem = last.trim_end_matches(".rs").to_string();
        if stem == "lib" || stem == "main" || stem == "mod" {
            comps.pop();
        } else {
            *last = stem;
        }
    }
    comps.join("::")
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let mut dir = ".".to_string();
    let mut prefix = String::new();
    let mut want_json = false;
    let mut include_tests = false;
    let mut it = args.iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "--out" => prefix = it.next().cloned().unwrap_or_default(),
            "--json" => want_json = true,
            "--include-tests" => include_tests = true,
            "-V" | "--version" => {
                println!("candor-scan {}", env!("CARGO_PKG_VERSION"));
                return;
            }
            "-h" | "--help" => {
                println!("candor-scan {} — stable-Rust effect scanner (no nightly)", env!("CARGO_PKG_VERSION"));
                println!();
                println!("USAGE:  candor-scan [<dir>] [--out <prefix>] [--json] [--include-tests]");
                println!();
                println!("  <dir>             crate root to scan (default: .)");
                println!("  --out <prefix>    report path prefix (default: <dir>/.candor/report);");
                println!("                    writes <prefix>.<crate>.scan.json + a call-graph sidecar");
                println!("  --json            print the report to stdout instead of writing files");
                println!("  --include-tests   also scan tests/ benches/ examples/ and #[cfg(test)] modules");
                println!("                    (off by default → the report describes the crate, not its harness)");
                println!("  -V, --version     print version");
                println!();
                println!("Syntactic, so it under-reports vs the full candor nightly lint (no Unknown). It never");
                println!("fabricates an effect. See https://github.com/tombaldwin/candor");
                return;
            }
            _ => dir = a.clone(),
        }
    }
    let root = Path::new(&dir);
    let crate_name = read_crate_name(root).unwrap_or_else(|| "crate".to_string());

    // Parse every in-scope .rs file ONCE (syn parses are reused across both passes below).
    let mut parsed: Vec<(String, syn::File)> = Vec::new();
    for entry in walkdir::WalkDir::new(root).into_iter().filter_map(Result::ok) {
        let p = entry.path();
        if !p.is_file() || p.extension().and_then(|e| e.to_str()) != Some("rs") {
            continue;
        }
        // All path-shape filters run on the path RELATIVE to the scan root — an absolute prefix can itself
        // contain `target`/`.cargo`/… (a vendored crate lives under `~/.cargo/registry/...`), which must
        // not trip them.
        let rel = p.strip_prefix(root).unwrap_or(p);
        // target/ build artifacts; hidden dirs (`.git`, `.github`, `.cargo`, …) holding tooling/CI scripts,
        // not library code (smol_str's `.github/ci.rs` otherwise reported a phantom `Exec`).
        if rel.components().any(|c| {
            c.as_os_str()
                .to_str()
                .is_some_and(|s| s == "target" || (s.starts_with('.') && s != "." && s != ".."))
        }) {
            continue;
        }
        // The build script runs at COMPILE time (ring's build.rs execs nasm) — never the crate's runtime
        // behaviour, so skip it always.
        if p.file_name().and_then(|s| s.to_str()) == Some("build.rs") {
            continue;
        }
        // Cargo's non-library compilation targets (tests/, benches/, examples/) — and the common nonstandard
        // singular `test/` tree (e.g. nix) — describe what the crate's HARNESS does (spawn a server, read
        // fixtures, seed RNG), not what the crate itself does. Scanning them conflates the two (redis's bench
        // harness alone showed Exec/Net/Fs/Env/Rand on 200+ fns). Skip by default; `--include-tests` keeps them.
        if !include_tests
            && rel.components().any(|c| {
                matches!(
                    c.as_os_str().to_str(),
                    Some("tests") | Some("test") | Some("benches") | Some("examples")
                )
            })
        {
            continue;
        }
        // A `#[cfg(test)] mod tests;` FILE module is invisible here — its test-ness is declared at the
        // `mod` site, not in the file — so a `tests.rs` / `*_tests.rs` / `*_test.rs` file's effects (a
        // seeded RNG, a temp file) would be mis-read as the crate's. By convention these stems are test
        // modules; skip them by default. (base64's `engine/tests.rs` otherwise reported a phantom `Rand`.)
        if !include_tests {
            if let Some(stem) = p.file_stem().and_then(|s| s.to_str()) {
                if is_test_file_stem(stem) {
                    continue;
                }
            }
        }
        let Ok(text) = std::fs::read_to_string(p) else { continue };
        let Ok(file) = syn::parse_file(&text) else { continue };
        parsed.push((rel.to_string_lossy().into_owned(), file));
    }

    // Pass A — index struct field types and function return types crate-wide, so a method call on
    // `self.field` or on the result of a local factory function can be typed and classified.
    let mut fields: FieldIndex = HashMap::new();
    let mut rets_tmp: HashMap<String, Option<String>> = HashMap::new();
    for (_, file) in &parsed {
        let mut uses = HashMap::new();
        collect_decls(&file.items, include_tests, &mut uses, &mut fields, &mut rets_tmp);
    }
    // Keep only unambiguous fn-leaf -> return-type mappings (a name with conflicting return types was
    // marked `None`); a guessed type would mis-classify.
    let returns: ReturnIndex = rets_tmp.into_iter().filter_map(|(k, v)| v.map(|t| (k, t))).collect();

    // Pass B — collect each function's calls (now with receiver-type inference available).
    let mut fns: Vec<FnInfo> = Vec::new();
    for (rel, file) in &parsed {
        let modpath = module_path(Path::new(rel));
        let mut uses = HashMap::new();
        scan_items(&file.items, &modpath, rel, include_tests, &fields, &returns, &mut uses, &mut fns);
    }

    // Two name indexes for resolving a call to a local definition. `by_leaf` keys on the bare last
    // segment (`new`); `by_tail2` keys on the last TWO segments (`RequestBuilder::new`). The leaf index
    // alone catastrophically over-connects on real crates: every call to *some* `new()` would link to
    // ALL `*::new` defs (in reqwest, 181 of them), smearing one type's effect across the whole graph.
    // So we prefer the qualified-tail match, which keeps `RequestBuilder::new` distinct from `Body::new`,
    // and fall back to the leaf only when it's UNAMBIGUOUS (exactly one def) — under-reporting (the honest
    // failure mode) rather than fabricating edges. See the precision note in the module doc.
    let mut by_leaf: HashMap<String, Vec<String>> = HashMap::new();
    let mut by_tail2: HashMap<String, Vec<String>> = HashMap::new();
    for f in &fns {
        by_leaf.entry(f.leaf.clone()).or_default().push(f.qual.clone());
        if let Some(t2) = tail2(&f.qual) {
            by_tail2.entry(t2).or_default().push(f.qual.clone());
        }
    }

    let mut direct: HashMap<String, BTreeSet<&'static str>> = HashMap::new();
    let mut hosts: HashMap<String, BTreeSet<String>> = HashMap::new();
    let mut cmds: HashMap<String, BTreeSet<String>> = HashMap::new();
    let mut paths: HashMap<String, BTreeSet<String>> = HashMap::new();
    let mut calls: HashMap<String, BTreeSet<String>> = HashMap::new();
    let mut loc: HashMap<String, String> = HashMap::new();
    for f in &fns {
        loc.entry(f.qual.clone()).or_insert_with(|| f.loc.clone());
        // The body invoked a callable the scan can't see through (closure / fn-pointer value): it could
        // perform any effect, so record an honest `Unknown` (propagated like any effect, surfaced in the
        // receipt's unresolved count) instead of silently certifying the function pure.
        if f.unresolved {
            direct.entry(f.qual.clone()).or_default().insert("Unknown");
        }
        for c in &f.calls {
            let cr = c.path.split("::").next().unwrap_or("");
            if let Some(eff) = candor_classify::classify(cr, &c.path) {
                direct.entry(f.qual.clone()).or_default().insert(eff);
                if let Some(s) = &c.str_arg {
                    match eff {
                        "Net" => { hosts.entry(f.qual.clone()).or_default().insert(host_part(s)); }
                        "Exec" => { cmds.entry(f.qual.clone()).or_default().insert(s.clone()); }
                        "Fs" => { paths.entry(f.qual.clone()).or_default().insert(s.clone()); }
                        _ => {}
                    }
                }
            }
            if !c.typed && !matches!(cr, "std" | "core" | "alloc") {
                // Resolve the call to local definitions: a qualified-tail match first (`Type::method`),
                // else a leaf match ONLY when it's unique. Never link to a many-way-ambiguous bare leaf.
                // Typed (receiver-inferred) calls are skipped here — they're for external classification,
                // and their synthetic `Type::method` tail could mis-link to a same-named local method.
                let targets: Option<&Vec<String>> = tail2(&c.path)
                    .and_then(|t2| by_tail2.get(&t2))
                    .or_else(|| by_leaf.get(&c.leaf).filter(|v| v.len() == 1));
                if let Some(targets) = targets {
                    for t in targets {
                        if t != &f.qual {
                            calls.entry(f.qual.clone()).or_default().insert(t.clone());
                        }
                    }
                }
            }
        }
    }

    let all: Vec<String> = fns.iter().map(|f| f.qual.clone()).collect();
    let inferred = propagate(&direct, &calls, &all);
    let hostsacc = propagate_str(&hosts, &calls, &all);
    let cmdsacc = propagate_str(&cmds, &calls, &all);
    let pathsacc = propagate_str(&paths, &calls, &all);

    let mut entries: Vec<ReportEntry> = Vec::new();
    let mut cg: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for q in &all {
        if let Some(cs) = calls.get(q) {
            cg.insert(q.clone(), cs.iter().cloned().collect());
        }
        let inf = inferred.get(q).cloned().unwrap_or_default();
        if inf.is_empty() {
            continue;
        }
        entries.push(ReportEntry {
            func: q.clone(),
            loc: loc.get(q).cloned().unwrap_or_default(),
            inferred: inf.iter().map(|s| s.to_string()).collect(),
            direct: direct.get(q).map(|d| d.iter().map(|s| s.to_string()).collect()).unwrap_or_default(),
            declared: Vec::new(),
            undeclared: Vec::new(),
            overdeclared: Vec::new(),
            // Honest blind-spot signal: this function (transitively) reached a callable the scan couldn't
            // see through. Mirrors the lint's `unresolved = has Unknown`, so the receipt's unresolved
            // count is truthful for the stable backend too — not a hardcoded 0.
            unresolved: inf.contains("Unknown"),
            hash: String::new(),
            fs: Vec::new(),
            hosts: hostsacc.get(q).map(|s| s.iter().cloned().collect()).unwrap_or_default(),
            cmds: cmdsacc.get(q).map(|s| s.iter().cloned().collect()).unwrap_or_default(),
            paths: pathsacc.get(q).map(|s| s.iter().cloned().collect()).unwrap_or_default(),
            calls: calls.get(q).map(|cs| cs.iter().cloned().collect()).unwrap_or_default(),
        });
    }
    entries.sort_by(|a, b| a.func.cmp(&b.func));

    let meta = candor_report::ReportMeta {
        version: format!("scan-{}", env!("CARGO_PKG_VERSION")),
        toolchain: "stable".into(),
    };
    let body = candor_report::to_report_json(&meta, &entries).unwrap_or_default();
    if want_json {
        println!("{body}");
    } else {
        let prefix = if prefix.is_empty() { format!("{dir}/.candor/report") } else { prefix };
        if let Some(parent) = Path::new(&prefix).parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let file = format!("{prefix}.{crate_name}.scan.json");
        let _ = std::fs::write(&file, &body);
        let _ = std::fs::write(
            format!("{prefix}.{crate_name}.scan.callgraph.json"),
            serde_json::to_string(&cg).unwrap_or_default(),
        );
        eprintln!(
            "candor-scan: wrote {} effectful functions to {file} (stable syntactic backend — see --help)",
            entries.len()
        );
    }
}

/// The last two `::`-segments of a path (`a::b::Type::new` → `Type::new`), the key used to resolve a
/// `Type::method` call to its definition without colliding every same-named method. `None` for a path
/// with fewer than two segments (a bare method leaf — resolved by unique-leaf fallback instead).
fn tail2(path: &str) -> Option<String> {
    let segs: Vec<&str> = path.split("::").collect();
    let n = segs.len();
    if n < 2 {
        return None;
    }
    Some(format!("{}::{}", segs[n - 2], segs[n - 1]))
}

fn host_part(h: &str) -> String {
    let a = h.split_once("://").map(|(_, r)| r).unwrap_or(h);
    let a = a.split('/').next().unwrap_or(a);
    a.rsplit_once('@').map(|(_, h)| h).unwrap_or(a).to_string()
}

fn read_crate_name(root: &Path) -> Option<String> {
    let txt = std::fs::read_to_string(root.join("Cargo.toml")).ok()?;
    let mut in_package = false;
    for line in txt.lines() {
        let l = line.trim();
        if l.starts_with('[') {
            in_package = l == "[package]"; // only the [package] table's `name` is the crate name
            continue;
        }
        // Match the key `name` exactly — `name =` / `name=` — not `namespace`/`name-server`, and only
        // inside `[package]` (a `name = ` in `[[bin]]`/`[dependencies]` is not the crate name).
        if in_package {
            if let Some(rest) = l.strip_prefix("name") {
                let rest = rest.trim_start();
                if let Some(v) = rest.strip_prefix('=') {
                    return Some(v.trim().trim_matches('"').replace('-', "_"));
                }
            }
        }
    }
    None
}

fn propagate(
    direct: &HashMap<String, BTreeSet<&'static str>>,
    calls: &HashMap<String, BTreeSet<String>>,
    all: &[String],
) -> HashMap<String, BTreeSet<&'static str>> {
    let mut acc = direct.clone();
    for f in all {
        acc.entry(f.clone()).or_default();
    }
    let mut changed = true;
    while changed {
        changed = false;
        for f in all {
            let add: BTreeSet<&'static str> = calls
                .get(f)
                .map(|cs| cs.iter().filter_map(|c| acc.get(c)).flatten().copied().collect())
                .unwrap_or_default();
            let e = acc.entry(f.clone()).or_default();
            let before = e.len();
            e.extend(add);
            if e.len() != before {
                changed = true;
            }
        }
    }
    acc
}

fn propagate_str(
    direct: &HashMap<String, BTreeSet<String>>,
    calls: &HashMap<String, BTreeSet<String>>,
    all: &[String],
) -> HashMap<String, BTreeSet<String>> {
    let mut acc = direct.clone();
    let mut changed = true;
    while changed {
        changed = false;
        for f in all {
            let add: BTreeSet<String> = calls
                .get(f)
                .map(|cs| cs.iter().filter_map(|c| acc.get(c)).flatten().cloned().collect())
                .unwrap_or_default();
            if add.is_empty() {
                continue;
            }
            let e = acc.entry(f.clone()).or_default();
            let before = e.len();
            e.extend(add);
            if e.len() != before {
                changed = true;
            }
        }
    }
    acc
}

#[cfg(test)]
mod tests {
    use super::*;

    fn uses(pairs: &[(&str, &str)]) -> HashMap<String, String> {
        pairs.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect()
    }

    #[test]
    fn expand_uses_the_use_map_and_strips_local_prefixes() {
        let u = uses(&[("fs", "std::fs"), ("Command", "std::process::Command")]);
        assert_eq!(expand("fs::read_to_string", &u), "std::fs::read_to_string");
        assert_eq!(expand("Command::new", &u), "std::process::Command::new");
        // crate/self/super are local and stripped, leaving the rest unresolved (matched by leaf later)
        assert_eq!(expand("crate::pricing::priced", &u), "pricing::priced");
        assert_eq!(expand("self::helper", &u), "helper");
        // an unknown first segment passes through unchanged
        assert_eq!(expand("foo::bar", &u), "foo::bar");
    }

    #[test]
    fn collect_use_expands_groups_and_renames() {
        let mut out = HashMap::new();
        // `use std::process::{Command, Stdio as Pipe};`
        let tree: syn::UseTree = syn::parse_str("std::process::{Command, Stdio as Pipe}").unwrap();
        collect_use(&tree, String::new(), &mut out);
        assert_eq!(out.get("Command").map(String::as_str), Some("std::process::Command"));
        assert_eq!(out.get("Pipe").map(String::as_str), Some("std::process::Stdio"));
    }

    #[test]
    fn module_path_mirrors_file_based_resolution() {
        assert_eq!(module_path(Path::new("src/lib.rs")), "");
        assert_eq!(module_path(Path::new("src/main.rs")), "");
        assert_eq!(module_path(Path::new("src/pricing.rs")), "pricing");
        assert_eq!(module_path(Path::new("src/billing/mod.rs")), "billing");
        assert_eq!(module_path(Path::new("src/billing/tax.rs")), "billing::tax");
    }

    #[test]
    fn host_part_strips_scheme_path_and_userinfo() {
        assert_eq!(host_part("https://api.stripe.com/v1/charges"), "api.stripe.com");
        assert_eq!(host_part("user:pass@db.internal:5432"), "db.internal:5432");
        assert_eq!(host_part("example.com"), "example.com");
    }

    #[test]
    fn propagate_is_transitive_across_the_call_graph() {
        // leaf has Fs directly; mid calls leaf; top calls mid — both must inherit Fs.
        let mut direct: HashMap<String, BTreeSet<&'static str>> = HashMap::new();
        direct.insert("leaf".into(), ["Fs"].into_iter().collect());
        let mut calls: HashMap<String, BTreeSet<String>> = HashMap::new();
        calls.insert("mid".into(), ["leaf".to_string()].into_iter().collect());
        calls.insert("top".into(), ["mid".to_string()].into_iter().collect());
        let all = vec!["leaf".to_string(), "mid".to_string(), "top".to_string(), "pure".to_string()];
        let acc = propagate(&direct, &calls, &all);
        assert!(acc["leaf"].contains("Fs"));
        assert!(acc["mid"].contains("Fs"));
        assert!(acc["top"].contains("Fs"));
        assert!(acc["pure"].is_empty());
    }

    #[test]
    fn tail2_keys_on_the_qualified_method() {
        assert_eq!(tail2("a::b::RequestBuilder::new").as_deref(), Some("RequestBuilder::new"));
        assert_eq!(tail2("pricing::compute_price").as_deref(), Some("pricing::compute_price"));
        assert_eq!(tail2("send"), None); // a bare method leaf — no type qualifier to disambiguate
    }

    #[test]
    fn qualified_tail_disambiguates_same_named_methods() {
        // Two distinct `new`s; a `RequestBuilder::new` call must resolve to ONLY the RequestBuilder one,
        // never to every `*::new` (the leaf-collision over-connection that smeared one effect crate-wide).
        let fns = ["http::RequestBuilder::new", "body::Body::new"];
        let mut by_leaf: HashMap<String, Vec<String>> = HashMap::new();
        let mut by_tail2: HashMap<String, Vec<String>> = HashMap::new();
        for q in fns {
            by_leaf.entry("new".into()).or_default().push(q.into());
            by_tail2.entry(tail2(q).unwrap()).or_default().push(q.into());
        }
        // a `RequestBuilder::new(...)` call
        let resolved: Option<&Vec<String>> = tail2("api::RequestBuilder::new")
            .and_then(|t2| by_tail2.get(&t2))
            .or_else(|| by_leaf.get("new").filter(|v| v.len() == 1));
        assert_eq!(resolved, Some(&vec!["http::RequestBuilder::new".to_string()]));
        // a bare `.new()`-by-leaf with two candidates resolves to NEITHER (ambiguous → under-report)
        let bare: Option<&Vec<String>> =
            tail2("new").and_then(|t2| by_tail2.get(&t2)).or_else(|| by_leaf.get("new").filter(|v| v.len() == 1));
        assert_eq!(bare, None);
    }

    #[test]
    fn macro_bodies_are_walked_for_hidden_calls() {
        // git2 hides every libgit2 FFI call in `try_call!(...)`; format macros hide call args. Both
        // must be collected, while a non-expression macro body (matches!) is skipped without panicking.
        let uses = HashMap::new();
        let fields = FieldIndex::new();
        let block: syn::Block = syn::parse_str(
            "{ try_call!(raw::git_remote_fetch(x)); println!(\"{}\", helper()); let _ = matches!(y, Some(_)); }",
        )
        .unwrap();
        let returns = ReturnIndex::new();
        let mut c = CallCollector {
            uses: &uses,
            vars: HashMap::new(),
            fields: &fields,
            returns: &returns,
            calls: Vec::new(),
            closure_vars: std::collections::HashSet::new(),
            unresolved: false,
        };
        for stmt in &block.stmts {
            c.visit_stmt(stmt);
        }
        let leaves: Vec<&str> = c.calls.iter().map(|c| c.leaf.as_str()).collect();
        assert!(leaves.contains(&"git_remote_fetch"), "call inside try_call! macro was missed: {leaves:?}");
        assert!(leaves.contains(&"helper"), "call inside println! macro was missed: {leaves:?}");
    }

    #[test]
    fn receiver_type_inference_resolves_method_dispatch() {
        // A method call on a param/field/typed-let of a known type resolves to `Type::method`, so the
        // existing per-crate rules fire. Build a collector with `client: reqwest::Client` in scope.
        let uses = HashMap::new();
        let mut fields = FieldIndex::new();
        // struct App { http: reqwest::Client }
        fields.entry("App".into()).or_default().insert("http".into(), "reqwest::Client".into());
        let mut vars = HashMap::new();
        vars.insert("client".to_string(), "reqwest::Client".to_string());
        vars.insert("self".to_string(), "App".to_string());
        let returns = ReturnIndex::new();
        let block: syn::Block =
            syn::parse_str("{ client.get(url).send(); self.http.execute(req); }").unwrap();
        let mut c = CallCollector {
            uses: &uses,
            vars,
            fields: &fields,
            returns: &returns,
            calls: Vec::new(),
            closure_vars: std::collections::HashSet::new(),
            unresolved: false,
        };
        for stmt in &block.stmts {
            c.visit_stmt(stmt);
        }
        let typed: Vec<&str> = c.calls.iter().map(|c| c.path.as_str()).collect();
        // chain `client.get(url).send()` → base type reqwest::Client, terminal verb send
        assert!(typed.contains(&"reqwest::Client::send"), "chain not typed to base: {typed:?}");
        // field access `self.http.execute(req)` resolves via the struct field index
        assert!(typed.contains(&"reqwest::Client::execute"), "field recv not typed: {typed:?}");
        // and both classify as Net through the shared classifier
        assert_eq!(candor_classify::classify("reqwest", "reqwest::Client::send"), Some("Net"));
        assert_eq!(candor_classify::classify("reqwest", "reqwest::Client::execute"), Some("Net"));
    }

    #[test]
    fn return_type_inference_flows_through_local_factories() {
        // `let p = create_pool()?; p.fetch_one(q)` — create_pool's recorded return type lets p resolve.
        let uses = HashMap::new();
        let fields = FieldIndex::new();
        let mut returns = ReturnIndex::new();
        returns.insert("create_pool".to_string(), "sqlx::PgPool".to_string());
        let block: syn::Block =
            syn::parse_str("{ let p = create_pool()?; p.fetch_one(q); }").unwrap();
        let mut c = CallCollector {
            uses: &uses,
            vars: HashMap::new(),
            fields: &fields,
            returns: &returns,
            calls: Vec::new(),
            closure_vars: std::collections::HashSet::new(),
            unresolved: false,
        };
        for stmt in &block.stmts {
            c.visit_stmt(stmt);
        }
        let typed: Vec<&str> = c.calls.iter().map(|c| c.path.as_str()).collect();
        assert!(typed.contains(&"sqlx::PgPool::fetch_one"), "return-typed recv not resolved: {typed:?}");

        // A computed-callable invocation (a closure / fn-pointer the scan can't see through) marks the
        // function `unresolved` (→ honest `Unknown`), while a LOCAL closure whose body IS visible does
        // not — its effects were already walked lexically.
        let mk = |src: &str| {
            let blk: syn::Block = syn::parse_str(src).unwrap();
            let mut cc = CallCollector {
                uses: &uses,
                vars: HashMap::new(),
                fields: &fields,
                returns: &returns,
                calls: Vec::new(),
                closure_vars: std::collections::HashSet::new(),
                unresolved: false,
            };
            for stmt in &blk.stmts {
                cc.visit_stmt(stmt);
            }
            cc.unresolved
        };
        assert!(mk("{ (handlers[k])(); }"), "indexed callable must be unresolved");
        assert!(mk("{ (self.cb)(); }"), "fn-pointer field call must be unresolved");
        assert!(!mk("{ let g = |x: i32| x + 1; let _ = g(3); }"), "local closure body is visible — not unresolved");
        assert!(!mk("{ helper(); other::thing(); }"), "ordinary path calls are not unresolved");

        // unwrap_result_option peels Result/Option to the success type
        let r: syn::Type = syn::parse_str("std::io::Result<reqwest::Client>").unwrap();
        assert_eq!(type_path(unwrap_result_option(&r), &uses).as_deref(), Some("reqwest::Client"));
        let o: syn::Type = syn::parse_str("Option<PgPool>").unwrap();
        assert_eq!(type_path(unwrap_result_option(&o), &uses).as_deref(), Some("PgPool"));
    }

    #[test]
    fn test_file_stems_are_recognised() {
        assert!(is_test_file_stem("tests")); // src/foo/tests.rs
        assert!(is_test_file_stem("test"));
        assert!(is_test_file_stem("decoder_tests")); // base64's read/decoder_tests.rs
        assert!(is_test_file_stem("engine_test"));
        // legitimate non-test modules must NOT be excluded
        assert!(!is_test_file_stem("latest")); // not `_test`-suffixed (no underscore boundary)
        assert!(!is_test_file_stem("request"));
        assert!(!is_test_file_stem("contest"));
        assert!(!is_test_file_stem("lib"));
    }

    #[test]
    fn cfg_test_modules_are_recognised() {
        let yes1: syn::ItemMod = syn::parse_str("#[cfg(test)] mod tests {}").unwrap();
        let yes2: syn::ItemMod =
            syn::parse_str("#[cfg(any(test, feature = \"x\"))] mod tests {}").unwrap();
        let no1: syn::ItemMod = syn::parse_str("#[cfg(feature = \"std\")] mod imp {}").unwrap();
        let no2: syn::ItemMod = syn::parse_str("mod real {}").unwrap();
        // deeper nesting positively requiring test → still skipped
        let yes3: syn::ItemMod =
            syn::parse_str("#[cfg(any(all(test, unix), windows))] mod t {}").unwrap();
        // `not(test)` is PRODUCTION code — must NOT be treated as a test module (the regression fix)
        let prod1: syn::ItemMod = syn::parse_str("#[cfg(not(test))] mod prod {}").unwrap();
        let prod2: syn::ItemMod = syn::parse_str("#[cfg(all(unix, not(test)))] mod prod {}").unwrap();
        assert!(is_cfg_test(&yes1.attrs));
        assert!(is_cfg_test(&yes2.attrs));
        assert!(is_cfg_test(&yes3.attrs));
        assert!(!is_cfg_test(&no1.attrs));
        assert!(!is_cfg_test(&no2.attrs));
        assert!(!is_cfg_test(&prod1.attrs), "cfg(not(test)) is production, not a test module");
        assert!(!is_cfg_test(&prod2.attrs), "cfg(all(unix, not(test))) is production");
    }

    #[test]
    fn expand_does_not_alias_a_crate_rooted_path() {
        // `crate::config::load` is explicitly crate-local; a `use other::config;` import must NOT hijack it.
        let u = uses(&[("config", "other::config")]);
        assert_eq!(expand("crate::config::load", &u), "config::load");
        assert_eq!(expand("self::config::load", &u), "config::load");
        // a NON-rooted bare `config::load` still expands via the use alias
        assert_eq!(expand("config::load", &u), "other::config::load");
    }

    #[test]
    fn ctor_type_rejects_a_module_path_receiver() {
        // `serde_json::from_str(s)` must NOT infer the MODULE `serde_json` as a type (lower-case receiver);
        // `reqwest::Client::new()` must still infer `reqwest::Client` (UpperCamel type receiver).
        let u = HashMap::new();
        let r = ReturnIndex::new();
        let modcall: syn::Expr = syn::parse_str("serde_json::from_str(s)").unwrap();
        assert_eq!(ctor_type(&modcall, &u, &r), None);
        let typecall: syn::Expr = syn::parse_str("reqwest::Client::new()").unwrap();
        assert_eq!(ctor_type(&typecall, &u, &r).as_deref(), Some("reqwest::Client"));
    }

    #[test]
    fn classifier_resolves_a_std_fs_call() {
        // guards the shared-classifier contract the scanner relies on: an expanded std::fs path is Fs.
        assert_eq!(candor_classify::classify("std", "std::fs::read_to_string"), Some("Fs"));
        assert_eq!(candor_classify::classify("std", "std::process::Command::new"), Some("Exec"));
    }
}
