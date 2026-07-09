//! Deferred-init bodies: `lazy_static!`/`once_cell`/`LazyLock`/`thread_local!` statics,
//! surfaced as `<lazy>` units so their effects aren't silently dropped.

use crate::*;

/// The synthetic-unit qual PREFIX for a LAZY/deferred static's init body. A deferred static
/// (`static X: Lazy<_> = Lazy::new(|| ..effect..)`, `thread_local!`, `lazy_static!`) attaches an init
/// CLOSURE that runs at FIRST USE, not at definition — so the closure body is reachable from no fn yet
/// performs the effect. We synthesize a unit per such static (`<lazy>::NAME`, the closure body walked
/// as a normal FnInfo) so the existing classifier/propagation charge it, then edge every FORCING site
/// (a fn that names the static) to it. A `<` in a path never collides with a real crate path (and a
/// resolved-local edge suppresses the classifier anyway), so the unit is never mis-classified. The
/// per-static keying is what prevents flooding: a PURE-init lazy's unit carries no effect, so its
/// forcing sites stay pure — only a genuinely-effectful init lights its accessors up.
pub(crate) const LAZY_UNIT_PREFIX: &str = "<lazy>";

/// Recognize the LAZY/deferred CONTAINER constructors whose argument is an init thunk run on first use.
/// A `Container::new(|| body)` defers the `body` to first use. Matched on the TYPE leaf of a `Type::new`
/// associated call so a
/// `use once_cell::sync::Lazy;` rename still hits. `OnceCell`/`OnceLock` are DELIBERATELY ABSENT: their
/// `get_or_init(|| ..)` already passes the closure at a normal reachable CALL SITE (the forcing site IS
/// the call), so the existing closure-arg walking already charges it — adding them would double-count.
pub(crate) fn is_lazy_container_new(path: &str) -> bool {
    // `Lazy::new`, `LazyLock::new`, `LazyCell::new` (once_cell sync/unsync + std). The penultimate
    // segment is the container TYPE; the last must be `new`.
    let Some(t2) = tail2(path) else { return false };
    let mut it = t2.split("::");
    let ty = it.next().unwrap_or("");
    let m = it.next().unwrap_or("");
    m == "new" && matches!(ty, "Lazy" | "LazyLock" | "LazyCell")
}

/// Extract the deferred INIT BODY (a block of statements) from a lazy-container `new` call's first
/// argument — a closure (`Lazy::new(|| { .. })`, `Lazy::new(|| expr)`) or a bare block
/// (`LazyLock::new(|| ..)` is the norm; a non-closure arg is not deferred). Returns the closure body as
/// statements to walk. A non-closure first arg (a function path `Lazy::new(load)`) is NOT inlined here —
/// it would be an ordinary reachable call if it appeared at a call site; the deferred-static seam is
/// specifically the inline CLOSURE/BLOCK, which is reachable from nowhere.
pub(crate) fn lazy_init_body(call: &syn::ExprCall) -> Option<Vec<syn::Stmt>> {
    if !matches!(&*call.func, syn::Expr::Path(p) if is_lazy_container_new(&path_to_string(&p.path))) {
        return None;
    }
    let first = call.args.first()?;
    closure_or_block_stmts(first)
}

/// The statements of an init thunk expression: a closure's body (block or single expr), or a bare block.
pub(crate) fn closure_or_block_stmts(e: &syn::Expr) -> Option<Vec<syn::Stmt>> {
    match e {
        syn::Expr::Closure(cl) => match &*cl.body {
            syn::Expr::Block(b) => Some(b.block.stmts.clone()),
            other => Some(vec![syn::Stmt::Expr(other.clone(), None)]),
        },
        syn::Expr::Block(b) => Some(b.block.stmts.clone()),
        syn::Expr::Paren(p) => closure_or_block_stmts(&p.expr),
        _ => None,
    }
}

/// If `it` is a LAZY/deferred static whose init has a walkable thunk body, return `(static_name, body)`.
/// Covers the four idioms:
///   - `static X: Lazy<_> = Lazy::new(|| ..)` / `LazyLock` / `LazyCell` — an `Item::Static`/`Item::Const`
///     whose init expr is a lazy-container `new` call (handles the `?`-free common form);
///   - `lazy_static! { static ref X: T = effectful(); }` — an `Item::Macro` (`lazy_static`), body parsed;
///   - `thread_local! { static T: Ty = effectful(); }` — an `Item::Macro` (`thread_local`), body parsed.
///
/// A PURE init is still returned (its synthetic unit will simply carry no effect) — purity is decided by
/// the classifier downstream, NOT here; returning it unconditionally is what keeps the keying per-static.
pub(crate) fn lazy_static_unit(it: &syn::Item) -> Option<(String, Vec<syn::Stmt>)> {
    match it {
        syn::Item::Static(s) => {
            let syn::Expr::Call(call) = &*s.expr else { return None };
            let body = lazy_init_body(call)?;
            Some((s.ident.to_string(), body))
        }
        // A `const X: Lazy<_> = Lazy::new(|| ..)` is unusual but legal and behaves identically.
        syn::Item::Const(c) => {
            let syn::Expr::Call(call) = &*c.expr else { return None };
            let body = lazy_init_body(call)?;
            Some((c.ident.to_string(), body))
        }
        syn::Item::Macro(m) => {
            let mname = path_to_string(&m.mac.path);
            let mname = mname.rsplit("::").next().unwrap_or(&mname);
            match mname {
                "lazy_static" => lazy_static_macro_body(&m.mac.tokens),
                "thread_local" => thread_local_macro_body(&m.mac.tokens),
                _ => None,
            }
        }
        _ => None,
    }
}

/// Parse a `lazy_static! { static ref NAME: T = EXPR; }` body: the init EXPR runs lazily on first deref,
/// so its effects are deferred exactly like `Lazy::new`. Returns `(NAME, [EXPR;])`. Single-static bodies
/// are the dominant form; a multi-static block parses the FIRST (a rare multi-static `lazy_static!` is
/// under-approximated to its first entry — honest, never fabricated). Parsing failure → skip (only adds
/// visibility, never breaks).
pub(crate) fn lazy_static_macro_body(tokens: &proc_macro2::TokenStream) -> Option<(String, Vec<syn::Stmt>)> {
    syn::parse2::<LazyStaticDecl>(tokens.clone())
        .ok()
        .map(|d| (d.name, vec![syn::Stmt::Expr(d.init, None)]))
}

/// Parse a `thread_local! { static NAME: Ty = EXPR; }` body the same way — the per-thread init EXPR runs
/// on first `.with(..)`, a deferred thunk. Returns `(NAME, [EXPR;])`.
pub(crate) fn thread_local_macro_body(tokens: &proc_macro2::TokenStream) -> Option<(String, Vec<syn::Stmt>)> {
    syn::parse2::<ThreadLocalDecl>(tokens.clone())
        .ok()
        .map(|d| (d.name, vec![syn::Stmt::Expr(d.init, None)]))
}

/// `[pub] static [ref] NAME: T = INIT;` — the single-static shape inside `lazy_static!`. We tolerate a
/// leading visibility and the `ref` keyword, take the NAME, skip the `: T`, and parse the `= INIT` expr.
pub(crate) struct LazyStaticDecl {
    pub(crate) name: String,
    pub(crate) init: syn::Expr,
}

impl syn::parse::Parse for LazyStaticDecl {
    fn parse(input: syn::parse::ParseStream) -> syn::Result<Self> {
        let _attrs = input.call(syn::Attribute::parse_outer)?;
        let _vis: syn::Visibility = input.parse()?;
        let _static: syn::Token![static] = input.parse()?;
        // `lazy_static!` requires `ref`; tolerate its absence so the parser is liberal.
        if input.peek(syn::Token![ref]) {
            let _ref: syn::Token![ref] = input.parse()?;
        }
        let name: syn::Ident = input.parse()?;
        let _colon: syn::Token![:] = input.parse()?;
        let _ty: syn::Type = input.parse()?;
        let _eq: syn::Token![=] = input.parse()?;
        let init: syn::Expr = input.parse()?;
        // `syn::parse2` requires the WHOLE stream consumed — drain the trailing `;` and any further
        // statics in a multi-static block. We keep only the FIRST static (the dominant single-static
        // form); a rare multi-static `lazy_static!` under-approximates to its first entry (honest).
        let _ = input.parse::<syn::Token![;]>();
        input.parse::<proc_macro2::TokenStream>()?;
        Ok(LazyStaticDecl { name: name.to_string(), init })
    }
}

/// `[pub] static NAME: Ty = INIT;` — the single-static shape inside `thread_local!` (no `ref`).
pub(crate) struct ThreadLocalDecl {
    pub(crate) name: String,
    pub(crate) init: syn::Expr,
}

impl syn::parse::Parse for ThreadLocalDecl {
    fn parse(input: syn::parse::ParseStream) -> syn::Result<Self> {
        let _attrs = input.call(syn::Attribute::parse_outer)?;
        let _vis: syn::Visibility = input.parse()?;
        let _static: syn::Token![static] = input.parse()?;
        let name: syn::Ident = input.parse()?;
        let _colon: syn::Token![:] = input.parse()?;
        let _ty: syn::Type = input.parse()?;
        let _eq: syn::Token![=] = input.parse()?;
        let init: syn::Expr = input.parse()?;
        // Drain the trailing `;` + any further per-thread statics (keep the first — honest under-approx).
        let _ = input.parse::<syn::Token![;]>();
        input.parse::<proc_macro2::TokenStream>()?;
        Ok(ThreadLocalDecl { name: name.to_string(), init })
    }
}
