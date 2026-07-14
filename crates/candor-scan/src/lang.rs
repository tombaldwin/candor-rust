//! Syntax-level helpers over `syn`: path/type reading, cfg evaluation, literal and
//! format-macro dissection. No scanner state — pure functions over the AST.

use crate::*;

pub(crate) fn path_to_string(p: &syn::Path) -> String {
    p.segments.iter().map(|s| s.ident.to_string()).collect::<Vec<_>>().join("::")
}

/// The trait leaves of a type-param-bound list (`T: Store + Send` -> ["Store", "Send"]). Marker
/// bounds need no filtering here: a leaf only ever matters if it later matches a local trait or a
/// local impl, and nobody locally declares `trait Send`.
pub(crate) fn bound_leaves(bounds: &syn::punctuated::Punctuated<syn::TypeParamBound, syn::Token![+]>) -> Vec<String> {
    bounds
        .iter()
        .filter_map(|b| match b {
            syn::TypeParamBound::Trait(t) => t.path.segments.last().map(|s| s.ident.to_string()),
            _ => None,
        })
        .collect()
}

/// The trait bound leaves of a DISPATCH-typed `syn::Type`: `&dyn T`, `impl T`, `Box<dyn T>` (and the
/// other single-arg smart pointers), or a bare generic param `X` declared `X: T`. Returns empty for
/// a concrete type — `type_path` owns those.
pub(crate) fn trait_leaves(ty: &syn::Type, generic_bounds: &HashMap<String, Vec<String>>) -> Vec<String> {
    match ty {
        syn::Type::Reference(r) => trait_leaves(&r.elem, generic_bounds),
        syn::Type::Paren(p) => trait_leaves(&p.elem, generic_bounds),
        syn::Type::Group(g) => trait_leaves(&g.elem, generic_bounds),
        syn::Type::TraitObject(t) => bound_leaves(&t.bounds),
        syn::Type::ImplTrait(t) => bound_leaves(&t.bounds),
        syn::Type::Path(p) => {
            if let Some(id) = p.path.get_ident() {
                return generic_bounds.get(&id.to_string()).cloned().unwrap_or_default();
            }
            // Box<dyn T> / Rc / Arc / RefCell / Mutex / RwLock — peel the wrapper, recurse on the arg.
            let Some(seg) = p.path.segments.last() else { return Vec::new() };
            let wrapper = matches!(seg.ident.to_string().as_str(), "Box" | "Rc" | "Arc" | "RefCell" | "Mutex" | "RwLock" | "Cell");
            if !wrapper {
                return Vec::new();
            }
            let syn::PathArguments::AngleBracketed(args) = &seg.arguments else { return Vec::new() };
            args.args
                .iter()
                .find_map(|a| match a {
                    syn::GenericArgument::Type(inner) => Some(trait_leaves(inner, generic_bounds)),
                    _ => None,
                })
                .unwrap_or_default()
        }
        _ => Vec::new(),
    }
}

/// Whether a type is an INVOKABLE callback — a bare fn pointer (`fn()`), an `impl`/`dyn Fn[Mut/Once]`, a
/// generic param bound by `Fn*`, or a `Box`/`Rc`/`Arc<dyn Fn*>`. A value of such a type called as `cb()`
/// invokes a body the syntactic scan cannot see, so the enclosing fn can't be certified pure — it MUST
/// read `Unknown`, never silently pure (SPEC §4). The non-bare forms are exactly where `trait_leaves`
/// finds an `Fn`/`FnMut`/`FnOnce` leaf; `Type::BareFn` carries no trait so it's matched explicitly.
pub(crate) fn is_callable_type(ty: &syn::Type, generic_bounds: &HashMap<String, Vec<String>>) -> bool {
    match ty {
        syn::Type::BareFn(_) => true,
        syn::Type::Reference(r) => is_callable_type(&r.elem, generic_bounds),
        syn::Type::Paren(p) => is_callable_type(&p.elem, generic_bounds),
        syn::Type::Group(g) => is_callable_type(&g.elem, generic_bounds),
        _ => trait_leaves(ty, generic_bounds)
            .iter()
            .any(|l| matches!(l.as_str(), "Fn" | "FnMut" | "FnOnce")),
    }
}

/// The tail (value) expression of a block, if it ends in one (`{ … ; expr }` with no trailing `;`).
pub(crate) fn block_tail_expr(b: &syn::Block) -> Option<&syn::Expr> {
    match b.stmts.last() {
        Some(syn::Stmt::Expr(e, None)) => Some(e),
        _ => None,
    }
}

/// The params of a signature that are invokable callbacks (`is_callable_type`) — so `cb()` on one reads
/// the honest `Unknown` instead of being silently dropped as a phantom call to a free fn `cb`.
pub(crate) fn seed_fn_typed_vars(sig: &syn::Signature) -> std::collections::HashSet<String> {
    let gb = generic_bounds_of(sig);
    let mut s = std::collections::HashSet::new();
    for arg in &sig.inputs {
        if let syn::FnArg::Typed(pt) = arg {
            if let syn::Pat::Ident(id) = &*pt.pat {
                if is_callable_type(&pt.ty, &gb) {
                    s.insert(id.ident.to_string());
                }
            }
        }
    }
    s
}

/// `X -> [trait leaves]` for a signature's generic params, from both inline bounds (`fn f<X: Store>`)
/// and where-clauses (`where X: Store`).
pub(crate) fn generic_bounds_of(sig: &syn::Signature) -> HashMap<String, Vec<String>> {
    generic_bounds_of_generics(&sig.generics)
}

/// Generic `T -> [trait bounds]` for any `syn::Generics` — a fn signature's OR a TYPE's own generics
/// (`struct Pipe<T: Saver>`), covering both the inline `<T: P>` bound and the `where T: P` clause. Reused so
/// a struct field typed `T` resolves to its bound (else `self.item.save()` on such a field read silent-pure).
pub(crate) fn generic_bounds_of_generics(generics: &syn::Generics) -> HashMap<String, Vec<String>> {
    let mut m: HashMap<String, Vec<String>> = HashMap::new();
    for gp in &generics.params {
        if let syn::GenericParam::Type(tp) = gp {
            let leaves = bound_leaves(&tp.bounds);
            if !leaves.is_empty() {
                m.entry(tp.ident.to_string()).or_default().extend(leaves);
            }
        }
    }
    if let Some(w) = &generics.where_clause {
        for pred in &w.predicates {
            if let syn::WherePredicate::Type(pt) = pred {
                if let syn::Type::Path(p) = &pt.bounded_ty {
                    if let Some(id) = p.path.get_ident() {
                        let leaves = bound_leaves(&pt.bounds);
                        if !leaves.is_empty() {
                            m.entry(id.to_string()).or_default().extend(leaves);
                        }
                    }
                }
            }
        }
    }
    m
}

/// The (use-expanded) type path of a `syn::Type`, ignoring references and generic args:
/// `&reqwest::Client` -> `reqwest::Client`, `Pool<Postgres>` -> `sqlx::Pool` (via `uses`). `None` for
/// non-nameable types (impl Trait, tuples, …) where there's nothing to classify a method against.
pub(crate) fn type_path(ty: &syn::Type, uses: &HashMap<String, String>) -> Option<String> {
    match ty {
        syn::Type::Reference(r) => type_path(&r.elem, uses),
        syn::Type::Paren(p) => type_path(&p.elem, uses),
        syn::Type::Group(g) => type_path(&g.elem, uses),
        syn::Type::Path(p) => {
            // A transparent OWNED smart-pointer wrapper (`Box<T>`/`Arc<T>`/`Rc<T>`) auto-derefs:
            // `wrapper.method()` dispatches to `T`'s method. Peel to `T` so the method resolves against
            // the POINTEE — without this, a `.method()` on an `Arc<Inner>` field/local/param resolved to
            // "Arc" (no impl in crate) and the call was SILENTLY DROPPED, not even Unknown (a §4
            // under-report). Arc/Rc/Box receivers are ubiquitous in real Rust (found by corpus-testing
            // duct + crates: it dropped duct's whole public-API Exec). Mirrors elem_type's wrapper-peel.
            // Only these three (owned, Deref-to-T); Mutex/RefCell need an explicit .lock()/.borrow().
            if let Some(seg) = p.path.segments.last() {
                if matches!(seg.ident.to_string().as_str(), "Box" | "Arc" | "Rc") {
                    if let syn::PathArguments::AngleBracketed(args) = &seg.arguments {
                        if let Some(inner) = args.args.iter().find_map(|a| match a {
                            syn::GenericArgument::Type(t) => Some(t),
                            _ => None,
                        }) {
                            return type_path(inner, uses);
                        }
                    }
                }
            }
            Some(expand(&path_to_string(&p.path), uses))
        }
        _ => None,
    }
}

/// The ELEMENT type path of a COLLECTION `syn::Type`: `Vec<T>` / `&[T]` / `[T; N]` / `HashSet<T>` /
/// `BTreeSet<T>` / `VecDeque<T>` / `Box<[T]>` (and `Arc`/`Rc`-wrapped slices) -> the expanded type path
/// of `T` (via `uses`, like `type_path`). `None` for a non-collection type. Used to type a loop /
/// subscript / iterator-closure binding over a collection so the element's method calls classify —
/// without it, a very common Rust shape (`for c in xs { c.send() }`, `xs[0].send()`) dropped its
/// receiver to pure (a §4 under-report). Peels references/parens/groups around the collection.
pub(crate) fn elem_type(ty: &syn::Type, uses: &HashMap<String, String>) -> Option<String> {
    match ty {
        syn::Type::Reference(r) => elem_type(&r.elem, uses),
        syn::Type::Paren(p) => elem_type(&p.elem, uses),
        syn::Type::Group(g) => elem_type(&g.elem, uses),
        // `[T]` (slice) and `[T; N]` (array) — the element is the type directly.
        syn::Type::Slice(s) => type_path(&s.elem, uses),
        syn::Type::Array(a) => type_path(&a.elem, uses),
        syn::Type::Path(p) => {
            let seg = p.path.segments.last()?;
            let name = seg.ident.to_string();
            let syn::PathArguments::AngleBracketed(args) = &seg.arguments else { return None };
            let first_ty = args.args.iter().find_map(|a| match a {
                syn::GenericArgument::Type(t) => Some(t),
                _ => None,
            })?;
            match name.as_str() {
                // The single-type-arg sequence collections: their first generic arg IS the element.
                "Vec" | "VecDeque" | "HashSet" | "BTreeSet" | "ContiguousArray" | "BinaryHeap"
                | "LinkedList" => type_path(first_ty, uses),
                // Smart-pointer wrappers around a collection/slice (`Box<[T]>`, `Arc<Vec<T>>`,
                // `Rc<[T]>`) — peel one layer and recurse so the inner collection's element surfaces.
                "Box" | "Arc" | "Rc" => elem_type(first_ty, uses),
                _ => None,
            }
        }
        _ => None,
    }
}

/// The per-position type paths of a TUPLE `syn::Type` (`(Sender, usize)` -> `[Some("Sender"),
/// Some("usize")]`), peeling references/parens/groups. `None` for a non-tuple type — its elements
/// are tracked so a later `let (s, _) = pair` (where `pair: (Sender, usize)`) types each binding.
pub(crate) fn tuple_types(ty: &syn::Type, uses: &HashMap<String, String>) -> Option<Vec<Option<String>>> {
    match ty {
        syn::Type::Reference(r) => tuple_types(&r.elem, uses),
        syn::Type::Paren(p) => tuple_types(&p.elem, uses),
        syn::Type::Group(g) => tuple_types(&g.elem, uses),
        syn::Type::Tuple(t) if t.elems.len() >= 2 => {
            Some(t.elems.iter().map(|e| type_path(e, uses)).collect())
        }
        _ => None,
    }
}

/// Constructor-style associated function names: `let x = Foo::new(..)` (or `::connect().await?`) means
/// `x: Foo`. Conservative set of names that return `Self` (or `Result<Self>`), so the inferred type is
/// reliable. A non-constructor assoc call (`Foo::parse`) is NOT treated as producing a `Foo`.
pub(crate) fn is_ctor(name: &str) -> bool {
    matches!(
        name,
        "new" | "default" | "builder" | "with_capacity" | "connect" | "open" | "init" | "from"
            | "from_path" | "from_str" | "with_config" | "create"
    )
}

/// The type a call expression produces (peeling `&`/`(..)`/`?`/`.await`), by two routes:
///
/// 1. a constructor `Path::ctor(..)` -> the `Path` type (`reqwest::Client::new()` -> `reqwest::Client`);
/// 2. a LOCAL free function whose return type the pre-pass recorded (`create_pool()` -> `sqlx::Pool`).
///
/// Returns the expanded type path. `returns` is the crate-wide fn-leaf -> return-type index.
pub(crate) fn ctor_type(expr: &syn::Expr, uses: &HashMap<String, String>, returns: &ReturnIndex) -> Option<String> {
    match expr {
        syn::Expr::Reference(r) => ctor_type(&r.expr, uses, returns),
        syn::Expr::Paren(p) => ctor_type(&p.expr, uses, returns),
        syn::Expr::Try(t) => ctor_type(&t.expr, uses, returns),
        syn::Expr::Await(a) => ctor_type(&a.base, uses, returns),
        // A BUILDER-terminated chain in a `let` binding: `let c = reqwest::Client::builder().build()?;`
        // The value's crate type is the CHAIN ROOT (`reqwest::Client::builder()` → `reqwest::Client`), so
        // a later `c.post(url).send()` resolves to `reqwest::Client::post`/`::send` and the URL is
        // captured (the dominant real-world reqwest idiom split across two statements — the fully-inline
        // form roots directly through `resolve_recv_type`'s MethodCall walk; this is its `let`-bound
        // sibling). Walk to the receiver's ctor type through builder steps. GUARDED with the SAME
        // type-CHANGE blocklist as `resolve_recv_type`: a method that yields a DIFFERENT (std) type
        // (`.iter()`/`.as_str()`/…) breaks the one-crate-type assumption → None (honest miss, never the
        // base crate's coarse rule fabricated onto a std value). The imprecision of a builder-vs-built
        // type name (`ClientBuilder` vs `Client`) is harmless: the reqwest rule matches the METHOD leaf
        // (`::post`/`::send`) regardless of the type segment, so either roots the same classification.
        syn::Expr::MethodCall(m) => {
            if matches!(
                m.method.to_string().as_str(),
                "iter" | "into_iter" | "iter_mut" | "drain" | "as_slice" | "as_mut_slice"
                    | "as_bytes" | "as_str" | "to_vec" | "keys" | "values" | "values_mut"
                    | "chars" | "bytes" | "get_argv" | "into_inner" | "lines"
            ) {
                return None;
            }
            ctor_type(&m.receiver, uses, returns)
        }
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
            // a local factory function call — its recorded (unambiguous) return type. The fn-typed
            // sentinel is NOT a nominal type (it types no var / receiver) — `expr_is_fn_typed` owns it.
            returns.get(leaf).filter(|t| *t != RET_FN_TYPED).cloned()
        }
        // `let s = S {..};` — a struct literal names its type directly.
        syn::Expr::Struct(s) => type_from_value_path(&path_to_string(&s.path), uses),
        // `let s = S;` — a UNIT-struct literal (or `let c = Color::Red;`, a unit enum variant, whose
        // value is typed as the ENUM). Gated by CamelCase so `let a = b;` (a variable copy) and
        // `let m = MAX_SIZE;` (a SCREAMING_SNAKE const) never mis-infer a type.
        syn::Expr::Path(p) => type_from_value_path(&path_to_string(&p.path), uses),
        _ => None,
    }
}

/// The type a VALUE path denotes, for `let` inference: `S` → `S`; `m::S` → `m::S`; `Color::Red`
/// (UpperCamel::UpperCamel = a unit enum variant) → `Color`. Only CamelCase leaves count as types —
/// a snake_case variable or SCREAMING_SNAKE const yields None (no inference; honest under-report).
pub(crate) fn type_from_value_path(full: &str, uses: &HashMap<String, String>) -> Option<String> {
    let camel = |s: &str| {
        // CamelCase = UpperCamel start, and either a single CHARACTER (`S`) or containing a lowercase
        // (distinguishes a type from a SCREAMING_SNAKE const). `chars().count()`, not `s.len()`: a
        // single-codepoint non-ASCII type ident (`struct É;`) is multi-BYTE and must still count as one
        // character. (/code-review.)
        let mut ch = s.chars();
        ch.next().is_some_and(|c| c.is_uppercase())
            && (s.chars().count() == 1 || s.chars().any(|c| c.is_lowercase()))
    };
    let segs: Vec<&str> = full.split("::").collect();
    let last = segs.last()?;
    if !camel(last) {
        return None;
    }
    // `Enum::Variant` — two trailing CamelCase segments: the VALUE's type is the enum (the penultimate).
    if segs.len() >= 2 && camel(segs[segs.len() - 2]) {
        return Some(expand(&segs[..segs.len() - 1].join("::"), uses));
    }
    Some(expand(full, uses))
}

/// Peel `Result<T, _>` / `Option<T>` / `io::Result<T>` to the inner `T` — a fallible constructor's
/// useful type is what it yields after `?`. Returns the inner type, or the type unchanged.
pub(crate) fn unwrap_result_option(ty: &syn::Type) -> &syn::Type {
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

/// The ERROR type LEAF of a fn's `Result<T, E>` / `io::Result<T>` return — the `?` operator's
/// `From::from` conversion TARGET. Returns the leaf of `E` when the output is a two-arg `Result<_, E>`
/// whose `E` is a concrete nominal path (a local error enum/struct). `None` for: a non-`Result` output,
/// a one-arg alias (`io::Result<T>`/`anyhow::Result<T>` carry no visible `E`), or a non-nominal/`Box<dyn
/// Error>` error (no single local type to convert to) — each the no-flood default for `?` (the edge is
/// only ever synthesized when `E` is also a LOCAL `impl From`, gated downstream in `charge_from`).
pub(crate) fn result_err_leaf(output: &syn::ReturnType, uses: &HashMap<String, String>) -> Option<String> {
    let syn::ReturnType::Type(_, ty) = output else { return None };
    let syn::Type::Path(p) = &**ty else { return None };
    let seg = p.path.segments.last()?;
    if seg.ident != "Result" {
        return None; // only the std two-arg Result exposes the error type positionally
    }
    let syn::PathArguments::AngleBracketed(args) = &seg.arguments else { return None };
    // The error is the SECOND generic arg. A one-arg `Result<T>` (an aliased Result) has no `E` here.
    let mut tys = args.args.iter().filter_map(|a| match a {
        syn::GenericArgument::Type(t) => Some(t),
        _ => None,
    });
    let _ok = tys.next()?;
    let err = tys.next()?;
    // The error type leaf — only a concrete nominal path (a local error type). `expand` then strips
    // module qualifiers; we keep just the leaf to match `trait_impls`/`by_tail2` keying.
    let expanded = type_path(err, uses)?;
    Some(expanded.rsplit("::").next().unwrap_or(&expanded).to_string())
}

/// Expand a call path against this file's `use` map: if the first segment is the last segment of some
/// `use a::b::Name`, replace it with the full `a::b::Name`. Turns `fs::read` → `std::fs::read`,
/// `Command::new` → `std::process::Command::new`. `crate`/`self`/`super` prefixes are stripped (local).
pub(crate) fn expand(path: &str, uses: &HashMap<String, String>) -> String {
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

pub(crate) fn first_str_lit(args: &syn::punctuated::Punctuated<syn::Expr, syn::token::Comma>) -> Option<String> {
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

/// The LITERAL string VALUE of a `const NAME: &str = "…";` / `static NAME: … = "…";` initializer — the
/// input to const-string propagation (SPEC §1: a STATICALLY-KNOWN host, even when the call builds the URL
/// with `format!`/a `const`, classifies Llm — candor-java gets this free because javac inlines a `static
/// final String`; the syntactic scanner must inline it itself). Returns `Some(value)` ONLY when the
/// initializer is a PLAIN string literal, or a `concat!(…)` of plain string literals (the trivial compile-
/// time concatenation `concat!("https://", "api.openai.com")`). Any RUNTIME initializer — a fn call, an
/// env read, a field, another identifier, an interpolation — returns `None`: we NEVER resolve a const to a
/// non-literal value (the no-fabrication invariant; an unknown-valued const must leave the call exactly as
/// it is today, bare Net with the host masked).
pub(crate) fn const_str_value(expr: &syn::Expr) -> Option<String> {
    match expr {
        syn::Expr::Lit(syn::ExprLit { lit: syn::Lit::Str(s), .. }) => {
            let v = s.value();
            (!v.trim().is_empty()).then_some(v)
        }
        // `concat!("a", "b", …)` of PLAIN string literals only — a trivial compile-time concat. A
        // non-literal token inside (an ident, a nested macro) aborts the whole thing → None (never a
        // partial/guessed value).
        syn::Expr::Macro(m) if m.mac.path.segments.last().is_some_and(|s| s.ident == "concat") => {
            let parsed: syn::punctuated::Punctuated<syn::Expr, syn::Token![,]> =
                m.mac.parse_body_with(syn::punctuated::Punctuated::parse_terminated).ok()?;
            let mut out = String::new();
            for part in &parsed {
                match part {
                    syn::Expr::Lit(syn::ExprLit { lit: syn::Lit::Str(s), .. }) => out.push_str(&s.value()),
                    _ => return None, // a non-string-literal part — not a trivial literal concat
                }
            }
            (!out.trim().is_empty()).then_some(out)
        }
        _ => None,
    }
}

/// The single leaf identifier of a bare-path expression (`API_BASE`, `crate::foo::API_BASE` → "API_BASE"),
/// for looking a reference up in the crate-wide const-string index. `None` for anything that isn't a plain
/// path (a method call, an index, a literal, …). We key the const index by LEAF only — a module-qualified
/// reference and its declaration share the leaf, and a genuine leaf collision at worst resolves to another
/// const's LITERAL string, which still runs through the same sound host refinement (no fabrication: a
/// non-model literal stays bare Net).
pub(crate) fn path_leaf_ident(expr: &syn::Expr) -> Option<String> {
    if let syn::Expr::Path(p) = expr {
        if p.qself.is_none() {
            return p.path.segments.last().map(|s| s.ident.to_string());
        }
    }
    None
}

/// The `format!` argument shape that anchors a host to a `const`: a `format!(FMT, ARGS…)` whose format
/// string BEGINS with `{}` (the interpolated value is the URL PREFIX) and whose FIRST value arg is the
/// given expr. Returns that first value expr when the shape matches, so the caller can resolve it against
/// the const index. Returns `None` when the format string has ANY literal prefix before the first hole
/// (`format!("https://{}/x", h)` — the host is the LITERAL, not the arg; already captured elsewhere) or
/// the first hole isn't a bare positional `{}` — in either case the first arg is NOT the host prefix and
/// must NOT be resolved (soundness: only a leading-`{}` prefix makes the const the host anchor).
pub(crate) fn format_const_prefix_arg(
    m: &syn::Macro,
) -> Option<syn::Expr> {
    if !is_format_macro(m.path.segments.last()?.ident.to_string().as_str()) {
        return None;
    }
    let parsed: syn::punctuated::Punctuated<syn::Expr, syn::Token![,]> =
        m.parse_body_with(syn::punctuated::Punctuated::parse_terminated).ok()?;
    let mut it = parsed.iter();
    // The format string must be the FIRST token and a plain literal that starts with a bare `{}` hole.
    let syn::Expr::Lit(syn::ExprLit { lit: syn::Lit::Str(fmt), .. }) = it.next()? else {
        return None;
    };
    let fmt = fmt.value();
    // Must LEAD with `{}` (not `{{`, not a literal prefix, not `{0}`/`{named}`): the first interpolated
    // value is the URL prefix. `{{` is an escaped brace, not a hole.
    if !fmt.starts_with("{}") {
        return None;
    }
    // Skip any leading NAMED args (`format!("{}", url = x)` is unusual, but a `name = expr` is not the
    // first POSITIONAL value); the first positional value arg is the `{}` prefix.
    it.find(|e| !matches!(e, syn::Expr::Assign(_))).cloned()
}

/// The bound identifier of a simple binding pattern: `c` / `mut c` / `&c` / `(c)` -> "c". `None` for a
/// destructuring/wildcard pattern (no single name to bind an element type to). Used for loop vars and
/// closure params.
pub(crate) fn single_pat_ident(pat: &syn::Pat) -> Option<String> {
    match pat {
        syn::Pat::Ident(id) => Some(id.ident.to_string()),
        syn::Pat::Reference(r) => single_pat_ident(&r.pat),
        syn::Pat::Paren(p) => single_pat_ident(&p.pat),
        // `|c: T|` — a type-annotated closure param; the inner pattern carries the name.
        syn::Pat::Type(t) => single_pat_ident(&t.pat),
        _ => None,
    }
}

/// A single-payload enum-variant match pattern `Variant(x)` / `Enum::Variant(x)` -> the bound name and
/// its payload type (looked up by the variant leaf in `enum_variants`; `None` type when the variant is
/// unknown/ambiguous — the caller still SCOPES it, clearing any stale binding so nothing leaks in).
/// `None` overall when the pattern isn't a single-field tuple-struct with a single-ident binding — a
/// multi-field or destructuring payload has no single receiver to type, an honest under-report.
pub(crate) fn arm_payload_binding(pat: &syn::Pat, enum_variants: &EnumVariantIndex) -> Option<(String, Option<String>)> {
    let ts = match pat {
        syn::Pat::TupleStruct(ts) => ts,
        // `Some(Variant(x))`-style nesting is rare; peel a reference/paren wrapper only.
        syn::Pat::Reference(r) => return arm_payload_binding(&r.pat, enum_variants),
        syn::Pat::Paren(p) => return arm_payload_binding(&p.pat, enum_variants),
        _ => return None,
    };
    if ts.elems.len() != 1 {
        return None; // multi-field variant — no single receiver to type
    }
    let name = single_pat_ident(ts.elems.first()?)?;
    let variant_leaf = ts.path.segments.last()?.ident.to_string();
    let ty = enum_variants.get(&variant_leaf).cloned();
    Some((name, ty))
}

/// True if the item carries any `#[cfg(...)]` attribute (conditionally compiled).
pub(crate) fn has_cfg(attrs: &[syn::Attribute]) -> bool {
    attrs.iter().any(|a| a.path().is_ident("cfg"))
}

/// True if a file stem names a conventional `#[cfg(test)] mod` FILE module (`tests.rs`, `foo_tests.rs`,
/// `foo_test.rs`) — whose test-ness is declared at the `mod` site, invisible when walking the file.
pub(crate) fn is_test_file_stem(stem: &str) -> bool {
    stem == "tests" || stem == "test" || stem.ends_with("_tests") || stem.ends_with("_test")
}

/// True if a crate-root-RELATIVE path is the Cargo BUILD SCRIPT — i.e. exactly `build.rs` at the root.
/// It runs at COMPILE time, never the crate's runtime behaviour, so it's skipped. A nested `src/build.rs`
/// is NOT the build script — it's an ordinary source module that merely shares the name (git2's
/// `src/build.rs` is `RepoBuilder`, the clone/fetch NETWORK surface) and must be scanned.
pub(crate) fn is_build_script(rel: &std::path::Path) -> bool {
    rel == std::path::Path::new("build.rs")
}

/// True if `test` is POSITIVELY required by this cfg predicate node (recursing through `any`/`all` to
/// any depth, but NOT through `not` — a `test` under `not()` means "compile when NOT testing", i.e.
/// production code that must NOT be skipped).
pub(crate) fn cfg_meta_requires_test(m: &syn::meta::ParseNestedMeta) -> bool {
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
pub(crate) fn is_cfg_test(attrs: &[syn::Attribute]) -> bool {
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

/// The crate's CFG-FEATURE picture, from Cargo.toml `[features]`: `active` = the transitive closure of
/// `default` (features enabling features); `declared` = every feature name that appears. A
/// `#[cfg(feature = "X")]` is KNOWN-FALSE when X is declared but not active (compiled out under a default
/// build), KNOWN-TRUE when active, and UNKNOWN when X isn't declared (a dependent could enable it). Read
/// once per scan from a OnceLock; empty ⇒ no feature info ⇒ nothing is skipped (no behaviour change).
pub(crate) type FeatureSets = (std::collections::HashSet<String>, std::collections::HashSet<String>);

pub(crate) static CFG_FEATURES: std::sync::OnceLock<std::sync::RwLock<FeatureSets>> = std::sync::OnceLock::new();

pub(crate) fn cfg_cell() -> &'static std::sync::RwLock<FeatureSets> {
    CFG_FEATURES.get_or_init(|| std::sync::RwLock::new((Default::default(), Default::default())))
}

/// Install the active/declared feature sets for the crate about to be scanned (called once per `scan_one`,
/// which runs sequentially per workspace member, before its parallel Pass B reads them).
pub(crate) fn set_cfg_features(f: FeatureSets) {
    *cfg_cell().write().unwrap() = f;
}

/// A snapshot of the active feature set, sorted — folded into the decl-index digest so the Pass-B cache
/// invalidates if the crate's enabled features change.
pub(crate) fn active_features_sorted() -> Vec<String> {
    let mut v: Vec<String> = cfg_cell().read().unwrap().0.iter().cloned().collect();
    v.sort();
    v
}

/// Pull every double-quoted token out of `s` into `out` (a manifest array's string entries).
pub(crate) fn push_quoted(s: &str, out: &mut Vec<String>) {
    let mut rest = s;
    while let Some(i) = rest.find('"') {
        rest = &rest[i + 1..];
        if let Some(j) = rest.find('"') {
            out.push(rest[..j].to_string());
            rest = &rest[j + 1..];
        } else {
            break;
        }
    }
}

/// Parse a Cargo.toml's `[features]` → (active, declared). `active` = closure of `default` over LOCAL feature
/// names (entries that are themselves feature keys); `dep:`/`?`/`crate/feat` entries enable dependencies,
/// not local features, so they don't expand the active SET (but they ARE recorded as declared if they name
/// a key). Line-based (no toml dep), tolerating multi-line arrays via bracket-depth tracking.
///
/// PURE — takes the manifest TEXT, not a path: the filesystem read is the caller's (the scan I/O layer),
/// so this syntax-analysis pass stays effect-free (candor's own `deny Fs lang` fix, dogfooded 2026-07-11).
/// An absent manifest is the caller's empty string → no `[features]` section → empty sets, as before.
pub(crate) fn parse_features(cargo_toml: &str) -> (std::collections::HashSet<String>, std::collections::HashSet<String>) {
    use std::collections::{HashMap, HashSet};
    let mut feats: HashMap<String, Vec<String>> = HashMap::new();
    let mut in_features = false;
    let mut cur: Option<(String, Vec<String>)> = None; // (key, accumulating entries) for an open `[ … ]`
    for line in cargo_toml.lines() {
        if let Some((k, vals)) = cur.as_mut() {
            push_quoted(line, vals);
            if line.contains(']') {
                feats.insert(std::mem::take(k), std::mem::take(vals));
                cur = None;
            }
            continue;
        }
        let t = line.trim();
        if let Some(sec) = toml_section(line) {
            in_features = sec == "features";
            continue;
        }
        if !in_features || t.is_empty() || t.starts_with('#') {
            continue;
        }
        if let Some(eq) = t.find('=') {
            let key = t[..eq].trim().trim_matches('"').to_string();
            let rhs = t[eq + 1..].trim();
            if let Some(arr) = rhs.strip_prefix('[') {
                let mut vals = Vec::new();
                push_quoted(arr, &mut vals);
                if rhs.contains(']') {
                    feats.insert(key, vals); // single-line array
                } else {
                    cur = Some((key, vals)); // multi-line — keep accumulating
                }
            }
        }
    }
    let declared: HashSet<String> = feats.keys().cloned().collect();
    // active = transitive closure of `default` over entries that are themselves local feature keys.
    let mut active: HashSet<String> = HashSet::new();
    let mut stack: Vec<String> = feats.get("default").cloned().unwrap_or_default();
    while let Some(f) = stack.pop() {
        // a `dep:x` / `x/y` / `x?/y` entry enables a dependency, not a local feature — ignore for the SET.
        if f.contains(':') || f.contains('/') {
            continue;
        }
        if active.insert(f.clone()) {
            if let Some(next) = feats.get(&f) {
                stack.extend(next.iter().cloned());
            }
        }
    }
    (active, declared)
}

/// 3-valued cfg evaluation under the active feature set: `Some(true)` definitely compiled, `Some(false)`
/// definitely compiled OUT, `None` unknown (a predicate we can't resolve — target_os, an undeclared
/// feature, `test` left to is_cfg_test). Only `Some(false)` lets a caller SKIP. Conservative throughout:
/// anything unrecognised is `None` (kept). `feature = "X"`: active⇒true, declared-but-inactive⇒false,
/// undeclared⇒None. `not/all/any` fold with Kleene logic.
pub(crate) fn cfg_eval(m: &syn::meta::ParseNestedMeta, active: &std::collections::HashSet<String>,
            declared: &std::collections::HashSet<String>) -> Option<bool> {
    if m.path.is_ident("feature") {
        // `feature = "X"` → active⇒Some(true), declared-but-inactive⇒Some(false), undeclared⇒None.
        let v = m.value().ok().and_then(|v| v.parse::<syn::LitStr>().ok());
        return v.and_then(|lit| {
            let name = lit.value();
            if active.contains(&name) {
                Some(true)
            } else if declared.contains(&name) {
                Some(false)
            } else {
                None
            }
        });
    }
    if m.path.is_ident("not") {
        let mut inner: Option<bool> = None;
        let _ = m.parse_nested_meta(|n| { inner = cfg_eval(&n, active, declared); Ok(()) });
        return inner.map(|b| !b);
    }
    if m.path.is_ident("all") {
        // false if ANY child false; true only if ALL true; else None.
        let (mut any_false, mut all_true, mut saw) = (false, true, false);
        let _ = m.parse_nested_meta(|n| { saw = true; match cfg_eval(&n, active, declared) { Some(false) => any_false = true, Some(true) => {}, None => all_true = false }; Ok(()) });
        if any_false { return Some(false); }
        if saw && all_true { return Some(true); }
        return None;
    }
    if m.path.is_ident("any") {
        // true if ANY child true; false only if ALL false; else None.
        let (mut any_true, mut all_false, mut saw) = (false, true, false);
        let _ = m.parse_nested_meta(|n| { saw = true; match cfg_eval(&n, active, declared) { Some(true) => any_true = true, Some(false) => {}, None => all_false = false }; Ok(()) });
        if any_true { return Some(true); }
        if saw && all_false { return Some(false); }
        return None;
    }
    None // target_os/unix/windows/test/… — unknown to a default-feature scan; keep the item.
}

/// True if an item/stmt's `#[cfg(...)]` is KNOWN-FALSE under the active feature set (compiled out, so its
/// effects are not the crate's default behaviour). Multiple cfg attrs are AND-ed (any false ⇒ skip).
pub(crate) fn is_cfg_inactive(attrs: &[syn::Attribute]) -> bool {
    if !attrs.iter().any(|a| a.path().is_ident("cfg")) {
        return false; // fast path: no cfg attrs (the overwhelming majority of items/stmts)
    }
    let guard = cfg_cell().read().unwrap();
    let (active, declared) = &*guard;
    if declared.is_empty() {
        return false; // no [features] info — never skip
    }
    attrs.iter().any(|a| {
        a.path().is_ident("cfg") && {
            let mut verdict: Option<bool> = None;
            let _ = a.parse_nested_meta(|m| { verdict = cfg_eval(&m, active, declared); Ok(()) });
            verdict == Some(false)
        }
    })
}

/// The outer attributes of an expression that can appear in STATEMENT position carrying a `#[cfg(...)]`
/// (e.g. `#[cfg(feature="debug")] { … }`). Variants that can't front a cfg in a body return `&[]`.
pub(crate) fn expr_attrs(e: &syn::Expr) -> &[syn::Attribute] {
    match e {
        syn::Expr::Block(x) => &x.attrs,
        syn::Expr::If(x) => &x.attrs,
        syn::Expr::Match(x) => &x.attrs,
        syn::Expr::Unsafe(x) => &x.attrs,
        syn::Expr::ForLoop(x) => &x.attrs,
        syn::Expr::While(x) => &x.attrs,
        syn::Expr::Loop(x) => &x.attrs,
        syn::Expr::Call(x) => &x.attrs,
        syn::Expr::MethodCall(x) => &x.attrs,
        syn::Expr::Macro(x) => &x.attrs,
        syn::Expr::Async(x) => &x.attrs,
        syn::Expr::Const(x) => &x.attrs,
        _ => &[],
    }
}

/// True if a statement is compiled out under the active feature set — a `#[cfg(feature="X")]`-gated block
/// or stmt whose effects are NOT the crate's default behaviour, so the call-collector must not walk into it
/// (winnow's `trace_result` reaches `std::env::var("COLUMNS")` only through a `#[cfg(feature="debug")]` block).
pub(crate) fn stmt_cfg_inactive(stmt: &syn::Stmt) -> bool {
    match stmt {
        syn::Stmt::Local(l) => is_cfg_inactive(&l.attrs),
        syn::Stmt::Macro(m) => is_cfg_inactive(&m.attrs),
        syn::Stmt::Expr(e, _) => is_cfg_inactive(expr_attrs(e)),
        syn::Stmt::Item(_) => false, // a local item carries its own effects, not the enclosing fn's
    }
}

pub(crate) fn impl_type_name(ty: &syn::Type) -> Option<String> {
    if let syn::Type::Path(p) = ty {
        return p.path.segments.last().map(|s| s.ident.to_string());
    }
    None
}

/// A NON-NOMINAL type: one with no user-definable inherent/trait impl that a local `Alias::method()` call
/// could resolve to — an array/slice/tuple/pointer/reference/fn type, or a bare built-in primitive path
/// (`u8`/`usize`/`bool`/…). A `type Alias = <non-nominal>` therefore can't legitimately link a
/// `Alias::assoc()` call to a same-named local STRUCT's associated fn (see the `prim_aliases` use).
pub(crate) fn is_non_nominal_type(ty: &syn::Type) -> bool {
    match ty {
        syn::Type::Array(_) | syn::Type::Slice(_) | syn::Type::Tuple(_)
        | syn::Type::Ptr(_) | syn::Type::Reference(_) | syn::Type::BareFn(_) => true,
        syn::Type::Path(p) if p.qself.is_none() && p.path.segments.len() == 1 => {
            const PRIMS: &[&str] = &[
                "u8", "u16", "u32", "u64", "u128", "usize", "i8", "i16", "i32", "i64", "i128",
                "isize", "f32", "f64", "bool", "char", "str",
            ];
            let seg = &p.path.segments[0];
            matches!(seg.arguments, syn::PathArguments::None)
                && PRIMS.contains(&seg.ident.to_string().as_str())
        }
        _ => false,
    }
}

pub(crate) fn collect_use(tree: &syn::UseTree, prefix: String, out: &mut HashMap<String, String>) {
    let join = |p: &str, s: &str| if p.is_empty() { s.to_string() } else { format!("{p}::{s}") };
    match tree {
        syn::UseTree::Path(p) => collect_use(&p.tree, join(&prefix, &p.ident.to_string()), out),
        syn::UseTree::Name(n) => {
            let id = n.ident.to_string();
            if id == "self" {
                // `use a::b::{self, ..}` imports the MODULE `b` itself under name `b` → map `b -> a::b`
                // so a later `b::func()` resolves. Without this, `self` was mapped uselessly as
                // `b::self` and the module alias was lost. (Found on coreutils `ls`: `use std::fs::{self,
                // Metadata}` then `fs::read_dir` was unresolved → a file lister reporting ZERO Fs.)
                if let Some(last) = prefix.rsplit("::").next() {
                    out.insert(last.to_string(), prefix.clone());
                }
            } else {
                out.insert(id.clone(), join(&prefix, &id));
            }
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
pub(crate) fn module_path(rel: &Path) -> String {
    let mut comps: Vec<String> =
        rel.components().filter_map(|c| c.as_os_str().to_str().map(String::from)).collect();
    // Anchor at the LAST `src/` component, not just a leading one. A workspace member's code lives at
    // `crates/<name>/src/…`, so the module path is what FOLLOWS that `src` — otherwise the filesystem path
    // from the scan root mangles `crates/cli/src/decompress.rs` into `crates::cli::src::decompress`, which
    // ALSO breaks intra-crate call resolution (call sites use the real module path, not the dir path).
    // Found scanning ripgrep's workspace root — every name came out `crates::…::src::…` and `main` was lost.
    if let Some(i) = comps.iter().rposition(|c| c == "src") {
        comps.drain(..=i);
    }
    if let Some(last) = comps.last() {
        let stem = last.trim_end_matches(".rs").to_string();
        if stem == "lib" || stem == "main" || stem == "mod" {
            comps.pop();
        } else {
            // A dotted file stem encodes a NESTED module path — the tonic / prost gRPC convention names
            // a file `envoy.service.accesslog.v3.rs` for the module `envoy::service::accesslog::v3`. Split
            // on `.` so the qualified name is `::`-separated, not one ugly dotted segment.
            let parts: Vec<String> = stem.split('.').map(String::from).collect();
            comps.pop();
            comps.extend(parts);
        }
    }
    comps.join("::")
}

/// The last two `::`-segments of a path (`a::b::Type::new` → `Type::new`), the key used to resolve a
/// `Type::method` call to its definition without colliding every same-named method. `None` for a path
/// with fewer than two segments (a bare leaf — only an unqualified FREE call resolves by leaf; a bare
/// method call with an unresolved receiver under-reports, see `resolve_target`).
pub(crate) fn tail2(path: &str) -> Option<String> {
    let segs: Vec<&str> = path.split("::").collect();
    let n = segs.len();
    if n < 2 {
        return None;
    }
    Some(format!("{}::{}", segs[n - 2], segs[n - 1]))
}

/// A CONSUMING iterator combinator: one that drives `Iterator::next` to completion (or short-circuits
/// after forcing some elements). Calling one on a custom-iterator value runs its `next` — so if the
/// receiver is a concrete local `impl Iterator`, the consumer reaches that `next`'s effect (handled by
/// `charge_iter_next`). This is the EAGER/forcing subset only: lazy ADAPTERS (`map`/`filter`/`take`/…)
/// return a new lazy iterator and do NOT force, so they are deliberately ABSENT — charging them would
/// over-approximate a never-driven chain. `collect` is included (it forces); a never-consumed `collect`
/// result is vanishingly rare and forcing is the safe direction. `next`/`next_back` are also absent —
/// an explicit `.next()` already resolves as an ordinary method call on the receiver type.
pub(crate) fn is_iter_consumer(leaf: &str) -> bool {
    matches!(
        leaf,
        "collect"
            | "count"
            | "sum"
            | "product"
            | "for_each"
            | "try_for_each"
            | "last"
            | "nth"
            | "fold"
            | "try_fold"
            | "reduce"
            | "min"
            | "max"
            | "min_by"
            | "max_by"
            | "min_by_key"
            | "max_by_key"
            | "all"
            | "any"
            | "find"
            | "find_map"
            | "position"
            | "rposition"
            | "partition"
            | "unzip"
            | "collect_into"
    )
}

/// A FORMATTING macro: one whose `{}`/`{:?}` args are run through `Display::fmt`/`Debug::fmt` (#2). The
/// std family `format!`/`format_args!`/`print!`/`println!`/`eprint!`/`eprintln!`/`write!`/`writeln!` plus
/// the very common `panic!`/`assert!` family and `.to_string()` (handled at the method site, not here).
/// Only these implicitly format — a non-format macro never reaches a `Display`/`Debug` impl this way.
pub(crate) fn is_format_macro(leaf: &str) -> bool {
    matches!(
        leaf,
        "format"
            | "format_args"
            | "print"
            | "println"
            | "eprint"
            | "eprintln"
            | "write"
            | "writeln"
            | "panic"
            | "unreachable"
            | "todo"
            | "unimplemented"
            | "assert"
            | "assert_eq"
            | "assert_ne"
            | "debug_assert"
            | "debug_assert_eq"
            | "debug_assert_ne"
    )
}

/// The (trait leaf, method) a binary operator overloads to, for the operator-overload coercion (#4). The
/// dispatch receiver is the LEFT operand (Rust resolves `a OP b` as `<A>::method(a, b)`). Comparison ops
/// route through `PartialOrd::partial_cmp`/`PartialEq::eq` (the impl method, not the per-op `lt`/`gt`
/// which forward to it). Lazy boolean `&&`/`||`, assignment, and the compound-assign ops (`+=` is
/// `AddAssign`, a distinct family left as an honest residual) return None — no overload edge.
pub(crate) fn binop_trait(op: &syn::BinOp) -> Option<(&'static str, &'static str)> {
    use syn::BinOp;
    Some(match op {
        BinOp::Add(_) => ("Add", "add"),
        BinOp::Sub(_) => ("Sub", "sub"),
        BinOp::Mul(_) => ("Mul", "mul"),
        BinOp::Div(_) => ("Div", "div"),
        BinOp::Rem(_) => ("Rem", "rem"),
        BinOp::BitAnd(_) => ("BitAnd", "bitand"),
        BinOp::BitOr(_) => ("BitOr", "bitor"),
        BinOp::BitXor(_) => ("BitXor", "bitxor"),
        BinOp::Shl(_) => ("Shl", "shl"),
        BinOp::Shr(_) => ("Shr", "shr"),
        BinOp::Eq(_) | BinOp::Ne(_) => ("PartialEq", "eq"),
        BinOp::Lt(_) | BinOp::Le(_) | BinOp::Gt(_) | BinOp::Ge(_) => ("PartialOrd", "partial_cmp"),
        _ => return None,
    })
}

/// True if `expr` is a `<recv>.into()` method call (no args), peeling effect-transparent wrappers — the
/// `.into()` coercion (#5) whose `From::from` target is the binding's annotated type.
pub(crate) fn expr_is_into_call(expr: &syn::Expr) -> bool {
    match expr {
        syn::Expr::MethodCall(m) => m.method == "into" && m.args.is_empty(),
        syn::Expr::Reference(r) => expr_is_into_call(&r.expr),
        syn::Expr::Paren(p) => expr_is_into_call(&p.expr),
        syn::Expr::Group(g) => expr_is_into_call(&g.expr),
        _ => false,
    }
}

/// Which argument a format `{…}` hole references.
pub(crate) enum FmtArg {
    /// a bare `{}` / `{:?}` — the next positional value arg in order
    Implicit,
    /// an explicit positional index `{0}` / `{1:?}`
    Index(usize),
    /// a named or inline-captured hole (`{name}`, `{x:?}`) — references a binding, not a value arg
    Named,
}

/// One parsed `{…}` hole of a format string: which arg it draws, and whether it requests `Debug` (`{:?}`/
/// `{:#?}`) rather than `Display`.
pub(crate) struct FmtHole {
    pub(crate) arg: FmtArg,
    pub(crate) debug: bool,
}

/// Parse the `{…}` holes of a format string (`std::fmt` mini-grammar, the subset that matters for picking
/// the formatter trait). Handles `{{`/`}}` escapes, implicit (`{}`) vs indexed (`{0}`) vs named (`{x}`)
/// argument refs, and detects `Debug` via a `?`/`#?` type in the format spec after `:`. We do NOT resolve
/// width/precision `$`-args (a `{:.*}` / `{:1$}` extra positional) — at worst that misaligns one implicit
/// index, a benign miss (an edge to the wrong-but-also-local arg, or none), never a fabrication on a
/// non-local type. Best-effort and forgiving: a malformed hole is skipped.
pub(crate) fn parse_format_holes(fmt: &str) -> Vec<FmtHole> {
    let mut holes = Vec::new();
    let bytes: Vec<char> = fmt.chars().collect();
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            '{' => {
                if bytes.get(i + 1) == Some(&'{') {
                    i += 2; // escaped `{{`
                    continue;
                }
                // Read until the matching `}`.
                let start = i + 1;
                let mut j = start;
                while j < bytes.len() && bytes[j] != '}' {
                    j += 1;
                }
                if j >= bytes.len() {
                    break; // unterminated — malformed, stop
                }
                let inner: String = bytes[start..j].iter().collect();
                // Split off the format SPEC after the first `:`.
                let (name_part, spec) = match inner.split_once(':') {
                    Some((n, s)) => (n.trim(), s),
                    None => (inner.trim(), ""),
                };
                // Debug = the spec's type char is `?` (optionally after `#` for pretty `{:#?}`), i.e. the
                // spec (after stripping fill/align/flags/width/precision) ENDS in `?`. A simple, robust
                // test: the spec contains `?` as its trailing type.
                let debug = spec.trim_end().ends_with('?');
                let arg = if name_part.is_empty() {
                    FmtArg::Implicit
                } else if let Ok(idx) = name_part.parse::<usize>() {
                    FmtArg::Index(idx)
                } else {
                    FmtArg::Named
                };
                holes.push(FmtHole { arg, debug });
                i = j + 1;
            }
            '}' => {
                i += if bytes.get(i + 1) == Some(&'}') { 2 } else { 1 }; // escaped `}}` or stray
            }
            _ => i += 1,
        }
    }
    holes
}
