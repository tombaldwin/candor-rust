//! Syntax-level helpers over `syn`: path/type reading, cfg evaluation, literal and
//! format-macro dissection. No scanner state — pure functions over the AST.

use crate::*;

pub(crate) fn path_to_string(p: &syn::Path) -> String {
    p.segments.iter().map(|s| s.ident.to_string()).collect::<Vec<_>>().join("::")
}

/// The crate roots whose traits are the LANGUAGE's, not a project dependency's — the explicit carve-out
/// for the imported-trait CHA (R4, collector.rs) and the crate-qualified dispatch key it emits.
///
/// A local `impl` of a trait from one of these says nothing about who the receiver of a `dyn`/bound call
/// actually is: essentially every crate in the ecosystem also implements them, so CHA-ing `Iterator` over
/// a local `impl Iterator for RowIter` charges every `.next()` in the crate with RowIter's effects
/// (execution-verified) — a fabrication, and its wide arm floods `Unknown` besides. A DEPENDENCY's trait
/// is the opposite case: the impls in front of us are the ones the dependency was given.
///
/// The empty root is included because it is the unqualified spelling — a PRELUDE trait needs no `use`, so
/// `expand` leaves it bare, and a bare leaf carries no provenance evidence at all. This is the rust
/// analogue of candor-swift's `RAW_VALUE_BASE_TYPES` (`eae2de2`): an imported-supertype CHA is only safe
/// with an explicit carve-out naming the types whose "conformance" means nothing.
pub(crate) fn is_std_trait_root(root: &str) -> bool {
    matches!(root, "std" | "core" | "alloc" | "")
}

/// Is `root` the crate root of a genuine PROJECT DEPENDENCY — the provenance carve-out on the
/// imported-trait CHA (R4, collector.rs)? STRICTER than `!is_std_trait_root`: it also rejects the
/// crate-LOCAL roots.
///
/// `self`/`crate`/`super` reach this predicate at all because a `use` binding is stored with the text it
/// was written with, so `pub use self::error::Error;` puts `Error -> self::error::Error` in the file's
/// map and `expand` hands the `self::` prefix straight through. Measured, not hypothetical: value-bag's
/// `internal/error.rs` re-exports `Error` exactly that way, and treating `self::error::Error` as a
/// dependency trait CHA'd `Error`/`OwnedError`/`Unsupported` onto its `&dyn Error` receivers and put
/// **17 fresh `Unknown`s** on value-bag (`ValueBag::to_str`, every `try_from`, `internal_visit`). The
/// trait there is std's `error::Error` wearing a local re-export — precisely what the std carve-out
/// exists to exclude, sneaking past it under a different spelling.
pub(crate) fn is_dependency_crate_root(root: &str) -> bool {
    !is_std_trait_root(root) && !matches!(root, "self" | "crate" | "super")
}

/// Every trait leaf a type spells in a `dyn` (TYPE-ERASED) position, at any depth — `&dyn T`,
/// `Box<dyn T>`, `Vec<Box<dyn T>>`, `Option<&dyn T>`, `(dyn T, u8)`. The SECOND carve-out on the
/// imported-trait CHA (R4, collector.rs), and the one provenance alone does not give.
///
/// A `dyn` receiver is ERASED: the author chose runtime dispatch, and the crate's own impls of the trait
/// are the candidate witnesses. A GENERIC BOUND (`fn to_string<T: Serialize>(v: &T)`) or an `impl Trait`
/// param is MONOMORPHIZED BY THE CALLER, so the crate's own impls say nothing about what actually runs —
/// they are a sample of one crate out of the whole ecosystem. That asymmetry is not academic: with
/// provenance as the only gate, `serde::Serialize`/`serde::Serializer` (a project dependency, so it
/// passes) CHA'd serde_json's own five `impl Serializer` types onto every generic serialization entry
/// point and put **32 fresh `Unknown`s** on serde_json — `to_string`, `to_vec`, `to_writer` — inherited
/// through edges to witnesses a caller's own `Serializer` would never run. serde_json spells
/// `dyn Serializer` nowhere, so requiring erasure takes that to zero and leaves R4's `&dyn` shape intact.
pub(crate) fn collect_dyn_trait_leaves(ty: &syn::Type, out: &mut std::collections::HashSet<String>) {
    match ty {
        syn::Type::TraitObject(t) => out.extend(bound_leaves(&t.bounds)),
        syn::Type::Reference(r) => collect_dyn_trait_leaves(&r.elem, out),
        syn::Type::Paren(p) => collect_dyn_trait_leaves(&p.elem, out),
        syn::Type::Group(g) => collect_dyn_trait_leaves(&g.elem, out),
        syn::Type::Slice(s) => collect_dyn_trait_leaves(&s.elem, out),
        syn::Type::Array(a) => collect_dyn_trait_leaves(&a.elem, out),
        syn::Type::Ptr(p) => collect_dyn_trait_leaves(&p.elem, out),
        syn::Type::Tuple(t) => t.elems.iter().for_each(|e| collect_dyn_trait_leaves(e, out)),
        // Any generic container, at any nesting: `Vec<Box<dyn T>>`, `Arc<Mutex<Box<dyn T>>>`,
        // `HashMap<String, Box<dyn T>>`. `impl Trait` is deliberately NOT a case here.
        syn::Type::Path(p) => {
            for seg in &p.path.segments {
                if let syn::PathArguments::AngleBracketed(args) = &seg.arguments {
                    for a in &args.args {
                        if let syn::GenericArgument::Type(t) = a {
                            collect_dyn_trait_leaves(t, out);
                        }
                    }
                }
            }
        }
        _ => {}
    }
}

/// The `dyn`-spelled trait leaves of a signature's PARAMETERS — the erased receivers in scope for the
/// body being walked. See `collect_dyn_trait_leaves`.
pub(crate) fn dyn_sig_trait_leaves(sig: &syn::Signature) -> std::collections::HashSet<String> {
    let mut out = std::collections::HashSet::new();
    for arg in &sig.inputs {
        if let syn::FnArg::Typed(pt) = arg {
            collect_dyn_trait_leaves(&pt.ty, &mut out);
        }
    }
    out
}

/// Trait leaf -> the multi-segment path the bound was WRITTEN with, for a signature that spells its
/// bounds in full: `&dyn deplib::Handler`, `impl deplib::Handler`, `T: deplib::Handler`. R6.
///
/// `bound_leaves` keeps only `segments.last()`, because every downstream index (`trait_impls`,
/// `local_traits`, `trait_fields`) is keyed by leaf. That is fine for the IMPORTED spelling — the file's
/// `use deplib::Handler` lets `expand` put the crate back on — but a FULLY-QUALIFIED receiver has no
/// `use` to recover it from, so the crate identity was simply LOST and the consumer never formed the
/// crate-qualified key. That is the whole of R6: the same receiver reads pure written one way and
/// resolves written the other.
///
/// `crate`/`self`/`super`-rooted spellings are deliberately NOT recorded. They are crate-LOCAL, so if
/// the trait were ours it would already be in `local_traits` and never reach this path; recording them
/// would hand `expand` a path whose root it STRIPS, turning `crate::deplib::Handler` into a
/// dependency-looking `deplib::Handler` — the value-bag fabrication class arriving by another door.
/// PER-PARAMETER qualified bounds: param name -> (trait leaf -> the crate-qualified path THAT parameter
/// was declared with). `sig_trait_quals` is keyed by LEAF alone and therefore cannot represent
/// `fn handle(a: &dyn alpha::Handler, b: &dyn beta::Handler)`; tombstoning the collision there is safe
/// against fabrication but LOSES `b`'s genuine reach — a silent under-report, which is worse. The
/// declaration already carries the answer per parameter; only the leaf-keyed map throws it away.
pub(crate) fn sig_trait_quals_by_param(sig: &syn::Signature) -> HashMap<String, HashMap<String, String>> {
    let mut out: HashMap<String, HashMap<String, String>> = HashMap::new();
    // A generic bound belongs to ONE TYPE PARAM, and must be kept that way. Collecting every bound into a
    // single leaf-keyed map re-created the very collision this function exists to avoid:
    // `fn f<A: alpha::Handler, B: beta::Handler>(a: A, b: B)` tombstoned the shared `Handler` leaf and BOTH
    // receivers lost their dep key — a silent under-report, and the same defect as the last-wins map in a
    // different spelling. (The comment here previously said "collision inside a single parameter is
    // impossible", which is true of a parameter's own declared type and irrelevant to a SHARED map.)
    let mut by_tp: HashMap<String, HashMap<String, String>> = HashMap::new();
    for p in &sig.generics.params {
        if let syn::GenericParam::Type(tp) = p {
            quals_from_bounds(&tp.bounds, by_tp.entry(tp.ident.to_string()).or_default());
        }
    }
    if let Some(w) = &sig.generics.where_clause {
        for pred in &w.predicates {
            if let syn::WherePredicate::Type(pt) = pred {
                // `where A: alpha::Handler` — the bounded type is a plain ident for the shapes we model.
                let Some(name) = plain_type_ident(&pt.bounded_ty) else { continue };
                quals_from_bounds(&pt.bounds, by_tp.entry(name).or_default());
            }
        }
    }
    for arg in &sig.inputs {
        let syn::FnArg::Typed(pt) = arg else { continue };
        let syn::Pat::Ident(id) = &*pt.pat else { continue };
        let mut per = HashMap::new();
        collect_trait_quals(&pt.ty, &mut per);
        // Attach ONLY the bounds of the type param this argument is actually declared with, peeling
        // references — `a: A` and `a: &A` both resolve to A's own bounds and to no other param's.
        if let Some(tp) = plain_type_ident(&pt.ty) {
            if let Some(g) = by_tp.get(&tp) {
                for (k, v) in g {
                    per.entry(k.clone()).or_insert_with(|| v.clone());
                }
            }
        }
        if !per.is_empty() {
            out.insert(id.ident.to_string(), per);
        }
    }
    out
}

/// The bare identifier of a type, peeling references/parens/groups: `A`, `&A`, `&mut A` -> `A`.
/// Returns None for anything compound, which is exactly when a generic bound must not be attached.
fn plain_type_ident(ty: &syn::Type) -> Option<String> {
    match ty {
        syn::Type::Reference(r) => plain_type_ident(&r.elem),
        syn::Type::Paren(p) => plain_type_ident(&p.elem),
        syn::Type::Group(g) => plain_type_ident(&g.elem),
        syn::Type::Path(p) if p.qself.is_none() => p.path.get_ident().map(|i| i.to_string()),
        _ => None,
    }
}

pub(crate) fn sig_trait_quals(sig: &syn::Signature) -> HashMap<String, String> {
    let mut out = HashMap::new();
    for arg in &sig.inputs {
        if let syn::FnArg::Typed(pt) = arg {
            collect_trait_quals(&pt.ty, &mut out);
        }
    }
    // `fn f<T: deplib::Handler>(t: T)` — the bound lives on the generics, not on the param type.
    for p in &sig.generics.params {
        if let syn::GenericParam::Type(tp) = p {
            quals_from_bounds(&tp.bounds, &mut out);
        }
    }
    if let Some(w) = &sig.generics.where_clause {
        for pred in &w.predicates {
            if let syn::WherePredicate::Type(pt) = pred {
                quals_from_bounds(&pt.bounds, &mut out);
            }
        }
    }
    out
}

fn quals_from_bounds(
    bounds: &syn::punctuated::Punctuated<syn::TypeParamBound, syn::Token![+]>,
    out: &mut HashMap<String, String>,
) {
    for b in bounds {
        let syn::TypeParamBound::Trait(t) = b else { continue };
        if t.path.segments.len() < 2 {
            continue; // a bare leaf carries no crate identity — `expand` + the file's `use` owns it
        }
        let head = t.path.segments[0].ident.to_string();
        if matches!(head.as_str(), "crate" | "self" | "super") {
            continue; // crate-LOCAL spelling — see the doc comment above
        }
        let Some(leaf) = t.path.segments.last().map(|s| s.ident.to_string()) else { continue };
        let full = path_to_string(&t.path);
        // NEVER GUESS WHICH CRATE. This map is keyed by LEAF, and one signature may bind the same leaf to
        // two different crates — `fn handle(a: &dyn alpha::Handler, b: &dyn beta::Handler)`. Last-wins
        // made `a.go()` form `beta::Handler::go` and inherit BETA's reported effects onto a function that
        // only touches alpha: a fabrication, and the mirror of the sin this rung exists to close. A
        // colliding leaf is TOMBSTONED (empty value) and consumers treat it as absent, falling back to
        // the file's `use` map — the same "two candidates are dropped, never picked from" rule the
        // cross-package join already applies.
        match out.get(&leaf) {
            Some(prev) if *prev != full => { out.insert(leaf, String::new()); }
            Some(_) => {}
            None => { out.insert(leaf, full); }
        }
    }
}

/// Walk a type for qualified trait bounds — mirrors `collect_dyn_trait_leaves`, but INCLUDES
/// `impl Trait`: a qualified bound is worth recording wherever it is spelled, and whether the receiver
/// may DISPATCH is the erasure carve-out's separate decision.
/// Public wrapper: a trait-typed LOCAL binding records its own qualified bounds, so it shadows the
/// parameter of the same name instead of inheriting that parameter's crate.
pub(crate) fn collect_trait_quals_pub(ty: &syn::Type, out: &mut HashMap<String, String>) {
    collect_trait_quals(ty, out)
}

fn collect_trait_quals(ty: &syn::Type, out: &mut HashMap<String, String>) {
    match ty {
        syn::Type::TraitObject(t) => quals_from_bounds(&t.bounds, out),
        syn::Type::ImplTrait(t) => quals_from_bounds(&t.bounds, out),
        syn::Type::Reference(r) => collect_trait_quals(&r.elem, out),
        syn::Type::Paren(p) => collect_trait_quals(&p.elem, out),
        syn::Type::Group(g) => collect_trait_quals(&g.elem, out),
        syn::Type::Slice(s) => collect_trait_quals(&s.elem, out),
        syn::Type::Array(a) => collect_trait_quals(&a.elem, out),
        syn::Type::Ptr(p) => collect_trait_quals(&p.elem, out),
        syn::Type::Tuple(t) => t.elems.iter().for_each(|e| collect_trait_quals(e, out)),
        syn::Type::Path(p) => {
            for seg in &p.path.segments {
                if let syn::PathArguments::AngleBracketed(args) = &seg.arguments {
                    for a in &args.args {
                        if let syn::GenericArgument::Type(t) = a {
                            collect_trait_quals(t, out);
                        }
                    }
                }
            }
        }
        _ => {}
    }
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
/// generic param bound by `Fn*`, a `Box`/`Rc`/`Arc<dyn Fn*>`, or a `Box`/`Rc`/`Arc`/`Symbol<T>` wrapping
/// any of the above (peeled and re-checked recursively, `Symbol` being `libloading`'s opaque
/// runtime-resolved-FFI-symbol handle). A value of such a type called as `cb()` invokes a body the
/// syntactic scan cannot see, so the enclosing fn can't be certified pure — it MUST read `Unknown`, never
/// silently pure (SPEC §4). The non-bare forms are exactly where `trait_leaves` finds an
/// `Fn`/`FnMut`/`FnOnce` leaf; `Type::BareFn` carries no trait so it's matched explicitly.
///
/// SOUNDNESS R161 — `callable_aliases` is the crate-wide set of `type NAME = <callable>` LEAF names.
/// Without it a NOMINAL ALIAS was a hole in every position at once: `pub type AutoExtension =
/// fn(Connection) -> Result<()>` made `fn init(.., ax: AutoExtension)` read a bare `[]` with no
/// `unknownWhy` — an affirmative purity claim over an opaque caller-supplied body — on published
/// rusqlite 0.40.2, and the same alias in a `let` annotation or a closure param was equally silent.
/// A leaf-NAME match, like every other index in this file: this scanner is syntactic and cannot ask
/// rustc whether some other crate's same-named type is the one in scope. A collision can only ever turn
/// a call into an honest `Unknown` (it names no effect), which is the direction that cannot fabricate.
pub(crate) fn is_callable_type(
    ty: &syn::Type,
    generic_bounds: &HashMap<String, Vec<String>>,
    callable_aliases: &std::collections::HashSet<String>,
) -> bool {
    match ty {
        syn::Type::BareFn(_) => true,
        syn::Type::Reference(r) => is_callable_type(&r.elem, generic_bounds, callable_aliases),
        syn::Type::Paren(p) => is_callable_type(&p.elem, generic_bounds, callable_aliases),
        syn::Type::Group(g) => is_callable_type(&g.elem, generic_bounds, callable_aliases),
        syn::Type::Path(p) => {
            if trait_leaves(ty, generic_bounds).iter().any(|l| matches!(l.as_str(), "Fn" | "FnMut" | "FnOnce")) {
                return true;
            }
            // R161, the ALIAS arm. Checked before the wrapper peel so `Alias` and `Option<Alias>` and
            // `Box<Alias>` all answer the same way (the peel below recurses back into here).
            if p.path.segments.last().is_some_and(|s| callable_aliases.contains(&s.ident.to_string())) {
                // §E1 HIT COUNTER — an unchanged row is not evidence the new code ran. Same switch, same
                // shape as R160's `SELFALIAS` line; gated on the cheap set hit so the env lookup is off
                // the hot path.
                if std::env::var("CANDOR_ALIAS_DEBUG").is_ok() {
                    eprintln!("R161ALIAS {}", path_to_string(&p.path));
                }
                return true;
            }
            // R161, the `Option`/`Result` arm. A PARAMETER position never peeled these — only
            // `record_return` did, for a fn's own RETURN type — so `f: Option<fn(&str)>` was not callable
            // here and the `if let Some(g) = f { g(p) }` / `match` / `.map(|g| g(p))` binders never hedged
            // `g` into `fn_typed_vars`: the fn vanished from `functions[]` entirely. `Option<Box<dyn Fn>>`
            // was never affected, which is why this was invisible — `trait_leaves` peels `Box`, and the
            // BARE fn pointer is the one payload that carries no trait to peel to.
            let inner = unwrap_result_option(ty);
            if !std::ptr::eq(inner, ty) && is_callable_type(inner, generic_bounds, callable_aliases) {
                if std::env::var("CANDOR_ALIAS_DEBUG").is_ok() {
                    eprintln!("R161OPT {}", path_to_string(&p.path));
                }
                return true;
            }
            // An OPAQUE RUNTIME-RESOLVED-SYMBOL wrapper. `libloading::Symbol<T>` (and its
            // `os::unix`/`os::windows` twins, which share the leaf name) is a `Deref<Target = T>` handle
            // onto a dynamically-loaded symbol; invoking it runs T's body, which is exactly as opaque to
            // this syntactic scan as a bare `fn()` — a pointer resolved at runtime and then called through
            // this wrapper read silent-pure (SOUNDNESS.md) because `is_callable_type` matched a `fn()`/
            // `dyn Fn*` ANNOTATION directly but never a NAMED type wrapping one. `Box`/`Rc`/`Arc` are
            // peeled the same way, closing the identical (previously unnoticed) hole for a boxed/shared
            // bare fn pointer (`Box<fn()>`), which nothing here recognised either.
            //
            // This is a NAME match on the leaf segment, not a type-resolved one: rust-scan is syntactic
            // and has no way to ask rustc whether some OTHER crate's unrelated `Symbol<T>` is the one in
            // scope. That can only ever turn a call `sym()` into an honest `Unknown` (never fabricate a
            // specific effect) — and the call syntax `sym()` only compiles at all if T really is callable,
            // so a same-named non-callable `Symbol<T>` from an unrelated crate is not even reachable here.
            // rust-deep (rustc-typed) does not need this special case: it asks rustc what the type is.
            let Some(seg) = p.path.segments.last() else { return false };
            let wrapper = matches!(seg.ident.to_string().as_str(), "Box" | "Rc" | "Arc" | "Symbol");
            if !wrapper {
                return false;
            }
            let syn::PathArguments::AngleBracketed(args) = &seg.arguments else { return false };
            args.args
                .iter()
                .any(|a| matches!(a, syn::GenericArgument::Type(inner) if is_callable_type(inner, generic_bounds, callable_aliases)))
        }
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
pub(crate) fn seed_fn_typed_vars(
    sig: &syn::Signature,
    callable_aliases: &std::collections::HashSet<String>,
) -> std::collections::HashSet<String> {
    let gb = generic_bounds_of(sig);
    let mut s = std::collections::HashSet::new();
    for arg in &sig.inputs {
        if let syn::FnArg::Typed(pt) = arg {
            if let syn::Pat::Ident(id) = &*pt.pat {
                if is_callable_type(&pt.ty, &gb, callable_aliases) {
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

/// The DISPATCH leaves of a COLLECTION's element type — the trait-object counterpart of `elem_type`.
/// `Vec<Box<dyn Doer>>` / `[&dyn Doer]` / `Arc<[Box<dyn Doer>]>` → the element's `trait_leaves`
/// (`["Doer"]`), so a `for it in items { it.go() }` over a collection of trait objects dispatches via
/// bounded CHA instead of dropping to pure (`elem_type` returns None for a `dyn`/`impl` element — it has
/// no nominal path, so the loop var was untyped). Empty for a concrete-element collection (`elem_type`
/// owns those) and for a non-collection type.
///
/// SOUNDNESS R177 — `callable_aliases` is threaded in for the same reason `is_callable_type` takes it: a
/// `type Cb = Box<dyn Fn()>` payload is a `Type::Path` like any other, so BOTH questions this function
/// asks of an element (is it a trait object? is it a further container?) answered no, and so did the
/// R161 bare-fn-pointer arm — `Option<Cb>` surfaced no element leaves at all while its one-alias-away
/// twin `Option<Box<dyn Fn()>>` surfaced `["Fn"]`. `is_callable_type` already knew the answer; this
/// function was a second implementation of the same question that had not been told (brief §F1-3).
pub(crate) fn elem_trait_leaves(
    ty: &syn::Type,
    generic_bounds: &HashMap<String, Vec<String>>,
    callable_aliases: &std::collections::HashSet<String>,
) -> Vec<String> {
    match ty {
        syn::Type::Reference(r) => elem_trait_leaves(&r.elem, generic_bounds, callable_aliases),
        syn::Type::Paren(p) => elem_trait_leaves(&p.elem, generic_bounds, callable_aliases),
        syn::Type::Group(g) => elem_trait_leaves(&g.elem, generic_bounds, callable_aliases),
        syn::Type::Slice(s) => trait_leaves(&s.elem, generic_bounds),
        syn::Type::Array(a) => trait_leaves(&a.elem, generic_bounds),
        syn::Type::Path(p) => {
            let Some(seg) = p.path.segments.last() else { return Vec::new() };
            let name = seg.ident.to_string();
            let syn::PathArguments::AngleBracketed(args) = &seg.arguments else { return Vec::new() };
            let type_args = || args.args.iter().filter_map(|a| match a {
                syn::GenericArgument::Type(t) => Some(t),
                _ => None,
            });
            let Some(first_ty) = type_args().next() else { return Vec::new() };
            // Peel one dispatch layer: a `dyn`/`Box<dyn>`/bound leaf (`trait_leaves`) OR a further container
            // (`elem_trait_leaves`), so ARBITRARY nesting composes — `Vec<Option<Box<dyn>>>`,
            // `Option<Vec<Box<dyn>>>`, `HashMap<K, Option<Box<dyn>>>` all surface the element's trait (R46).
            let dispatch = |t: &syn::Type| {
                let d = trait_leaves(t, generic_bounds);
                if !d.is_empty() {
                    return d;
                }
                let e = elem_trait_leaves(t, generic_bounds, callable_aliases);
                if !e.is_empty() {
                    return e;
                }
                // SOUNDNESS R161 — a BARE FN POINTER payload. Both questions above are TRAIT questions,
                // and `fn(&str)` carries no trait to answer with, so `Option<fn(&str)>` surfaced NO
                // element leaves while `Option<Box<dyn Fn(&str)>>` (one `Box` away) surfaced `["Fn"]`.
                // The consequence was not a precision loss but silence: `if let Some(g) = f { g(p) }`,
                // `match f { Some(g) => g(p), .. }` and `f.map(|g| g(p))` all bind `g` through
                // `resolve_elem_trait_leaves`, so with no leaves `g` was never hedged into
                // `fn_typed_vars`, `g(p)` resolved as a phantom free fn, and a function whose ONLY call
                // was that callback disappeared from `functions[]` entirely — an affirmative purity
                // claim (SPEC §2 rule 3) over an opaque caller-supplied body. Executed ground truth:
                // the fixture's callback really writes a file in that frame.
                //
                // The synthetic `"Fn"` leaf is the SAME one `static_holds_callable` and
                // `ret_dispatch_leaves` already produce, and it matches no local trait, so it can name
                // no concrete effect — it can only turn silence into `Unknown`.
                if is_bare_fn(t) {
                    if std::env::var("CANDOR_ALIAS_DEBUG").is_ok() {
                        eprintln!("R161ELEMFN {name}");
                    }
                    return vec!["Fn".to_string()];
                }
                // SOUNDNESS R177 — …and the NOMINAL-ALIAS payload, which is the same silence one
                // spelling over. `pub type Cb = Box<dyn Fn()>; cb: Option<Cb>` with
                // `if let Some(c) = &self.cb { c() }` was ABSENT from `functions[]` on published 0.34.0
                // AND on the 0.35.0 candidate — an affirmative purity claim over a caller-installed
                // callback, executed ground truth. R161 closed the parameter position for this alias and
                // listed the container position as "not established"; it is established now.
                //
                // `is_callable_type` is the ONE authority for "is this value invokable" (it also peels
                // `Box`/`Rc`/`Arc`/`Symbol` and `Option`/`Result`), so this arm asks IT rather than
                // adding a third spelling of the question. The answer it contributes is the synthetic
                // `"Fn"` leaf every other callable site already produces: it matches no local trait, so
                // it can name no concrete effect — only turn a silent drop into `Unknown`.
                if is_callable_type(t, generic_bounds, callable_aliases) {
                    if std::env::var("CANDOR_ALIAS_DEBUG").is_ok() {
                        eprintln!("R177ELEMALIAS {name}");
                    }
                    return vec!["Fn".to_string()];
                }
                Vec::new()
            };
            match name.as_str() {
                "Vec" | "VecDeque" | "HashSet" | "BTreeSet" | "ContiguousArray" | "BinaryHeap"
                | "LinkedList" => dispatch(first_ty),
                // Option<Box<dyn T>> / Result<Box<dyn T>, E> — the payload (Ok/Some) is a trait object; its
                // leaves let `o.map(|d| d.go())` / `for d in o` / `o.iter().for_each(..)` dispatch. (if-let /
                // `.unwrap()` are separate binding sites handled at their pattern.)
                "Option" | "Result" => dispatch(first_ty),
                // A MAP's VALUE (2nd type arg) — a `.values()`/`for v in m.values()` iteration of
                // trait-object values (`HashMap<String, Box<dyn Handler>>`, the keyed-registry shape).
                "HashMap" | "BTreeMap" | "IndexMap" | "DashMap" | "FxHashMap" | "AHashMap" =>
                    type_args().nth(1).map(dispatch).unwrap_or_default(),
                // Smart-pointer / interior-mutability wrappers around a COLLECTION: peel one layer and
                // recurse so a `Arc<Mutex<Vec<Box<dyn>>>>` / `Rc<RefCell<Vec<Box<dyn>>>>` surfaces the element.
                //
                // R101 — the DEFERRED-INIT cells belong in this list and were missing: `OnceLock<T>` /
                // `OnceCell<T>` / `LazyLock<T>` / `LazyCell<T>` / `once_cell::Lazy<T>` are interior-mutability
                // wrappers exactly like `Mutex`/`RefCell`, and their contents are reached the same way (an
                // accessor yielding `Option<&T>`/`&T`). Without them `static CB: OnceLock<Box<dyn Fn()>>`
                // surfaced NO element leaves, so `if let Some(f) = CB.get() { f() }` never hedged `f` into
                // `fn_typed_vars` and the call resolved as a phantom free-fn and vanished (SOUNDNESS R101,
                // driver `pf_oncelock_cb` — a kernel-witnessed silent under-report). Only `elem_trait_leaves`
                // gains them, NOT `trait_leaves`: a `OnceLock<Box<dyn Doer>>` is not itself a `Doer` (it does
                // not `Deref`), so peeling it in the direct-dispatch resolver would fabricate a receiver type.
                "Box" | "Arc" | "Rc" | "Mutex" | "RwLock" | "RefCell" | "Cell"
                | "OnceLock" | "OnceCell" | "LazyLock" | "LazyCell" | "Lazy" => dispatch(first_ty),
                _ => Vec::new(),
            }
        }
        _ => Vec::new(),
    }
}

/// SOUNDNESS R161 — a BARE FN POINTER under the reference/paren/group wrappers `trait_leaves` peels.
/// Split out rather than inlined because it answers the one shape a TRAIT question cannot: `fn(&str)`
/// implements `Fn` but names no trait bound anywhere in its syntax, so every leaf-based test returns
/// empty for it and cannot tell it apart from an ordinary concrete payload.
fn is_bare_fn(ty: &syn::Type) -> bool {
    match ty {
        syn::Type::BareFn(_) => true,
        syn::Type::Reference(r) => is_bare_fn(&r.elem),
        syn::Type::Paren(p) => is_bare_fn(&p.elem),
        syn::Type::Group(g) => is_bare_fn(&g.elem),
        _ => false,
    }
}

/// Whether a set of dispatch leaves names an INVOKABLE callback rather than a user trait — the ONE
/// definition of that question, shared by `CallCollector::leaves_are_callable` (every binder site) and
/// by the Pass-A `callable_statics` index. Rust exposes no stable method on `Fn`/`FnMut`/`FnOnce`, so
/// such a binding is only ever reached with CALL syntax, which the `trait_vars` dispatch machinery
/// cannot see — every site that types these leaves must also hedge into `fn_typed_vars` (R71).
pub(crate) fn leaves_are_callable(leaves: &[String]) -> bool {
    leaves.iter().any(|l| matches!(l.as_str(), "Fn" | "FnMut" | "FnOnce"))
}

/// R101 — whether a `static`/`const` ITEM's declared type holds an INVOKABLE callback inside a
/// container/cell, so that unwrapping it (`if let Some(f) = CB.get()`, `let Some(f) = CB.get() else`,
/// `match CB.get()`, `while let`) binds a name whose `f()` calls a body this scan cannot see.
///
/// DELIBERATELY the ELEMENT question (`elem_trait_leaves`), not `is_callable_type`. The only consumer is
/// `resolve_elem_trait_leaves`, the unwrap/element resolver, so the index answers exactly what that
/// resolver asks. It therefore does NOT capture a DIRECTLY callable static (`static W: fn(&str) = writer;`
/// — `Type::BareFn` yields no element leaves): that shape is not unwrapped, it is CALLED, and R99(3)
/// already resolves `W(..)` to the concrete `writer` through the alias index. Widening this to
/// `is_callable_type` would put such a static in both, and the honest concrete resolution is the better
/// answer — so the narrower question is the deliberate one, not an oversight.
///
/// SOUNDNESS DIRECTION, AND THE CONDITION IT RESTS ON — worded as the assumption it is, because the
/// unconditional reading is FALSE. The index only ever produces the synthetic `"Fn"` leaf, the same one
/// `ret_dispatch_leaves` decodes `RET_FN_TYPED` into, and that leaf matches no local trait — so it cannot
/// contribute a CONCRETE effect. It CAN WITHDRAW one: the consuming arm in `resolve_elem_trait_leaves`
/// runs BEFORE the local `elem_trait_of`/`trait_vars` lookup, so a name that is a callable static
/// crate-wide AND a dispatch-typed local HERE would have its real leaves replaced by `["Fn"]` and lose
/// its dispatch. The `locally_bound` gate on that arm is the only thing preventing that, and it is
/// load-bearing rather than defensive: delete it and the cross-module shadow control in
/// `callback_installed_through_a_static_cell_reads_unknown_not_silent_pure` goes `["Env","Fs"]` ->
/// `["Env"]`, losing `D::go` outright. Measured, not argued. Read the property as ADDITIVE GIVEN THAT
/// GATE.
pub(crate) fn static_holds_callable(
    ty: &syn::Type,
    callable_aliases: &std::collections::HashSet<String>,
) -> bool {
    leaves_are_callable(&elem_trait_leaves(ty, &HashMap::new(), callable_aliases))
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

/// The per-position DISPATCH-trait leaves of a TUPLE type whose elements include a trait object /
/// bound param (`(Box<dyn Doer>, u32)` -> `[["Doer"], []]`). `Some` only when at least one position is a
/// dispatch element (else `tuple_types`' concrete route owns it), so a `let (d, _) = pair` binds `d` into
/// `trait_vars` for bounded-CHA dispatch (`type_path` yields nothing for a `dyn` element — R46 tuple).
pub(crate) fn tuple_trait_leaves(
    ty: &syn::Type,
    generic_bounds: &HashMap<String, Vec<String>>,
) -> Option<Vec<Vec<String>>> {
    match ty {
        syn::Type::Reference(r) => tuple_trait_leaves(&r.elem, generic_bounds),
        syn::Type::Paren(p) => tuple_trait_leaves(&p.elem, generic_bounds),
        syn::Type::Group(g) => tuple_trait_leaves(&g.elem, generic_bounds),
        syn::Type::Tuple(t) if t.elems.len() >= 2 => {
            let v: Vec<Vec<String>> = t.elems.iter().map(|e| trait_leaves(e, generic_bounds)).collect();
            v.iter().any(|l| !l.is_empty()).then_some(v)
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
                // A TRANSPARENT owned smart-pointer constructor (`Box::new(x)`/`Rc::new(x)`/`Arc::new(x)`)
                // yields a value that AUTO-DEREFS to its POINTEE for method dispatch — type it as the
                // pointee (the ctor arg's type) so `let w = Arc::new(Worker); w.run()` resolves
                // `Worker::run` rather than the impl-less "Arc" (a §4 under-report — `type_path` already
                // peels a `Arc<Worker>` FIELD/param, but `Arc::new` dropped the arg here). NOT Mutex/
                // RefCell/RwLock/Cell — their methods (`.lock()`/`.borrow()`) live on the wrapper, so the
                // wrapper layer must survive (`Arc::new(Mutex::new(x))` → "Mutex", not the inner type).
                if last == "new" && matches!(ty_leaf, "Box" | "Rc" | "Arc") {
                    if let Some(inner) = c.args.first().and_then(|a| ctor_type(a, uses, returns)) {
                        return Some(inner);
                    }
                }
                if is_ctor(last) && type_like {
                    return Some(expand(ty, uses));
                }
            }
            // a local factory function call — its recorded (unambiguous) return type. The fn-typed
            // sentinel is NOT a nominal type (it types no var / receiver) — `expr_is_fn_typed` owns it.
            // Neither sentinel is a NOMINAL type: `RET_FN_TYPED` types no var (a callback), and the
            // `RET_DYN_PREFIX` dispatch-object return is resolved by TRAIT (via `resolve_recv_traits`'s
            // Call arm), never as a concrete `Type::method`. Filter both out of concrete var-typing.
            recorded_return_type(leaf, returns)
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

/// The recorded return type of a fn LEAF, as a NOMINAL type path — the one authority for "what concrete
/// type does calling this local factory produce". Filters out both of `record_return`'s sentinels: the
/// fn-typed one (a callback types no var — `expr_is_fn_typed` owns it) and the three `<dyn>` dispatch
/// shapes (resolved by TRAIT, never as a concrete `Type::method`).
///
/// Extracted so `ctor_type` (the `let`-binding type inference) and `ctor_leaf_from_call_returns` (the
/// R165 drop-glue route) cannot answer it differently. They HAD to, before: the drop marker's binder-keyed
/// predecessor consulted this index and the position-independent rewrite that replaced it did not, so a
/// free-function constructor stopped being a construction at all.
pub(crate) fn recorded_return_type(leaf: &str, returns: &ReturnIndex) -> Option<String> {
    returns
        .get(leaf)
        .filter(|t| *t != RET_FN_TYPED && ret_dyn_leaves(t).is_none()
            && ret_elem_dyn_leaves(t).is_none() && ret_tuple_dyn_leaves(t).is_none())
        .cloned()
}

/// SOUNDNESS R165 — the type LEAF a call RELEASES into this scope when the CALLEE PATH does not name it.
///
///     pub fn from_handle(p: &str) -> H { H { p: p.into() } }   // constructs and returns — no charge
///     pub fn holds(p: &str) -> usize { let h = from_handle(p); h.p.len() }   // H dies HERE
///
/// `ctor_leaf_from_call_path` recognises a tuple-struct/variant literal and a `Type::assoc()` call — both
/// spellings in which the callee path IS or CONTAINS the type. A bare `from_handle(p)` is neither: its
/// `rsplit_once("::")` returns `None` and the whole route declines, so `holds` was ABSENT while the
/// destructor really ran in its frame (executed: the file really is removed). The intermediate cannot
/// supply the answer either — `from_handle`'s OWN report is correctly empty, and it leaves no residual
/// edge for a caller to inherit.
///
/// The answer comes from the crate's own `ReturnIndex`, a DECLARED fact, not from a name heuristic. That
/// is what keeps this clear of R160's deliberate refusal to fall back to a bare LEAF: R160 refused to
/// let `Self::NAME` MATCH a same-named free fn, a resolution guess; this reads what the callee's
/// signature says it returns. `ReturnIndex` already drops any leaf recorded with two different return
/// types, and `note_construction`'s `drop_relevant` gate means only a type with a local `impl Drop`
/// survives — so a cross-crate leaf collision has to hit a local `Drop` type of the same name to matter,
/// and when it does the direction is an over-charge, never silence.
///
/// STATED LIMIT: leaf-keyed like the index itself, so `serde_json::from_str` and a local `from_str` are
/// one name to this route. Deliberate — the alternative (single-segment paths only) draws the boundary
/// around the one spelling the row was filed for, and a `use m::from_handle` import already expands to
/// a multi-segment path before it gets here.
pub(crate) fn ctor_leaf_from_call_returns(full: &str, returns: &ReturnIndex) -> Option<String> {
    let last = full.rsplit("::").next().unwrap_or(full);
    if last == "drop" {
        return None;
    }
    local_type_leaf(&recorded_return_type(last, returns)?)
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

/// ⟨typeSurface.returns⟩ The nominal type a caller's BINDING holds for `let x = f()` — the type of the
/// VALUE, not the useful type hiding inside it. Expanded through `uses`; `Self` resolves to the impl
/// type. `None` for anything that is not a plain generic-free nominal path.
///
/// THE REFUSAL IS THE POINT, and it is defect 2 of the reverted attempt. `record_return` (the LOCAL type
/// index) applies `unwrap_result_option`, so `fn connect() -> Result<Conn, E>` records `Conn` — right for
/// local inference, where the consuming site is a `connect()?`. Published across the boundary it is a lie
/// about the binding: `let c = dep::connect();` holds a `Result`, whose `map`/`unwrap`/`is_ok` are the
/// Result's, and keying those against `Conn` charged `Conn::map`'s effects to a caller that never runs
/// them. The design note allows recording the wrapper OR refusing to key through it; this refuses.
///
/// Refusing costs nothing today, and the reason is worth writing down rather than assuming: the
/// consumer's trigger (`dep_bound_vars`) only fires on a DIRECT `let x = dep::f();` — a
/// `let x = dep::f()?;` is a `syn::Expr::Try` and records no provenance at all — so a wrapped return has
/// no consumer to serve. Extending to `?` needs a consumer trigger AND a wrapper encoding, and adding
/// either one alone is how the fabrication comes back.
/// The returned path is MODULE-QUALIFIED in the producing crate's own namespace — the same namespace the
/// report's entry hashes use — because a BARE type name is module-RELATIVE and treating it as "matches
/// any module" is defect 1 by another door. Caught on the second fixture: `mod mock { fn client() ->
/// Client }` published `deplib#sync::Client`, so a consumer's PURE `mock_client()` would have been
/// charged `sync::Client::send`'s `Fs`. `expand` alone cannot fix it — it leaves a bare name bare.
pub(crate) fn bound_return_type(
    ty: &syn::Type,
    uses: &HashMap<String, String>,
    self_ty: Option<&str>,
    modpath: &str,
) -> Option<String> {
    // Deliberately NOT peeling references / Box / Arc the way `type_path` does. That peel is right when
    // resolving a method against a receiver we can SEE; here the type crosses a scan boundary, where the
    // consumer's own peeling decides. Only a bare owned nominal path travels.
    let syn::Type::Path(p) = ty else { return None };
    if p.qself.is_some() {
        return None; // `<T as Trait>::Assoc` — an associated type, not a nameable nominal
    }
    // THE TYPE ARGUMENTS ARE IGNORED; THE OUTER PATH IS WHAT THE BINDING HOLDS.
    //
    // This used to refuse ANY path carrying a generic argument, because one "means a WRAPPER
    // (`Result<_>`/`Option<_>`/`Vec<_>`/`Box<_>`) or a generic instantiation (`Wrapper<T>` — the design
    // note's open question, deliberately left unanswered)". Those are two cases and only the first needed
    // refusing:
    //
    //   `Result<Conn, E>`   the binding holds a RESULT. Keying it to `Conn` is the lie the reverted
    //                       attempt published — `.map`/`.unwrap`/`.is_ok` are the Result's, and charging
    //                       `Conn::send`'s Fs to a caller that never ran it is a fabrication.
    //   `DateTime<Utc>`     the binding holds a DATETIME. `DateTime`'s methods ARE the binding's methods.
    //                       Nothing here was ever unsound; it fell through the same door.
    //
    // Keying on the OUTER path is right for BOTH, and it is the exact opposite of the reverted defect:
    // that one UNWRAPPED (`Result<Conn,E>` -> `Conn`); this never looks inside the angle brackets at all.
    // `Result<Conn,E>` -> `Result` is TRUE, and harmlessly unresolvable because a crate's own report
    // carries no methods under `Result`. `path_to_string` maps `s.ident` only, so the rest of this
    // function has always been argument-blind — this guard was the whole of it.
    //
    // ADDITIVE AT THIS FUNCTION, NOT NECESSARILY AT THE PUBLISHED SURFACE — the first version of this
    // comment said "STRICTLY ADDITIVE … it can only turn `None` into `Some`", and a self-review measured
    // that false. Here it IS additive: every path previously accepted had no arguments to ignore. But
    // `build_type_surface` applies the never-guess rule over the results, so binding MORE returns creates
    // collisions that did not exist, and a collision DROPS the key. Measured against this commit's parent
    // on a fn declared twice under mutually exclusive `#[cfg]`s, one arm returning `A` and the other
    // `W<A>`:
    //
    //     before   returns: {"ar#mk": "ar#A"}      published = 1
    //     after    returns: (absent)               published = 0
    //
    // AND THE NEW BEHAVIOUR IS THE CORRECT ONE, which is why the code stands and only the claim changed.
    // The two arms return genuinely different types, so `let x = mk();` holds an `A` or a `W<A>` depending
    // on target; publishing `ar#A` unconditionally was true on ONE target and asserted on both. Before the
    // fix the generic arm simply did not bind, so the collision was invisible and one arm's answer was
    // published as if it were the only one. Dropping it is never-guess working on evidence it could not
    // previously see. Pinned by `type_surface_drops_a_cfg_pair_that_returns_DIFFERENT_types`.
    //
    // MEASURED, and the queue's diagnosis was wrong. On the real `chrono`, `offset::utc::Utc::now` — whose
    // entry already carries `Clock` — published NO return type, because it returns `DateTime<Utc>`. The
    // work queue filed the cause as a SPURIOUS COLLISION: chrono declares `now()` twice under mutually
    // exclusive `#[cfg]`s, so "the return index sees two same-named defs and the never-guess rule drops
    // the entry even though both name the same type". It does not: a synthetic with a `#[cfg]`-duplicated
    // NON-generic return publishes fine, and chrono's entry never reaches the collision rule at all
    // (`bound_returns=0` for it — there is nothing to collide). Isolated on three one-line variants:
    // `Plain` binds, `DateTime<Utc>` does not, `DateTime<u8>` does not. The generic was the whole cause.
    if is_non_nominal_type(ty) {
        return None; // a bare primitive names no type a dep report carries methods under
    }
    let written = path_to_string(&p.path);
    // `Self` is the impl's own type, declared in THIS module.
    let written = if written == "Self" { self_ty?.to_string() } else { written };
    let mut segs: Vec<&str> = written.split("::").collect();
    let (path, relative) = match segs.first().copied()? {
        // `expand` STRIPS a `super::` root without walking up, so it would hand back a path rooted in
        // the WRONG module. Refuse: an under-emission is the safe direction, a wrong type is not.
        "super" => return None,
        "crate" => {
            segs.remove(0);
            (segs.join("::"), false)
        }
        "self" => {
            segs.remove(0);
            (segs.join("::"), true)
        }
        head => match uses.get(head) {
            // A `use` binding names an ABSOLUTE path — possibly `crate::`-rooted, possibly another
            // crate's, in which case nothing in this report will match it and it simply drops.
            Some(bound) => {
                let rest = &segs[1..];
                let joined = if rest.is_empty() { bound.clone() } else { format!("{bound}::{}", rest.join("::")) };
                let stripped = joined
                    .strip_prefix("crate::")
                    .or_else(|| joined.strip_prefix("self::"))
                    .unwrap_or(&joined)
                    .to_string();
                (stripped, false)
            }
            // No `use` binding: the name is MODULE-RELATIVE. `Client` inside `mod mock` is
            // `mock::Client`, never some other module's `Client`.
            None => (written.clone(), true),
        },
    };
    if path.is_empty() {
        return None;
    }
    Some(if relative && !modpath.is_empty() { format!("{modpath}::{path}") } else { path })
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
            // R105 — `alias_join`, not `format!`: `full` may carry several `#[cfg]`-duplicated arms.
            let joined = alias_join(full, &segs[1..]);
            // SOUNDNESS R160's §E1 HIT COUNTER — an unchanged row is not evidence the new code ran, so the
            // A/B has to be able to prove the corpus REACHES the `Self` binding. Gated on the cheap `Self`
            // compare first, so the env lookup happens only on paths this change can possibly affect.
            if segs[0] == SELF_KEY && std::env::var("CANDOR_ALIAS_DEBUG").is_ok() {
                eprintln!("SELFALIAS {path} -> {joined}");
            }
            // R99 — a crate-LOCAL rebind of the MODULE (`use crate::facade;` then `facade::Command::new`)
            // rewrites to `crate::facade::Command::new`, which is the alias map's own key shape. Re-apply
            // the qualified lookup ONCE to the rewritten path: without it the SIBLING-module spelling of a
            // facade re-export stayed silent while the ancestor-module spelling resolved. Only the
            // `crate::`-rooted rewrite is re-applied — a rebind onto an EXTERNAL module
            // (`use somecrate::facade;`) must keep that crate's identity and is left alone.
            // R105 — skipped when `joined` carries several `#[cfg]`-duplicated arms, because
            // `strip_prefix("crate::")` is a question about ONE path and a joined form has no single
            // answer to it. STATED AS THE LIMIT IT IS RATHER THAN AS A GUARANTEE, because the obvious
            // justification ("a multi-arm value is by construction external") is FALSE: only the `pub use`
            // route checks its head against `crate`/`self`/`super`; the `type` and `const` routes expand
            // through the file's own `use` map and can yield a crate-local target. So a duplicated alias
            // with a crate-local arm does not get the sibling-module re-resolution a single-arm one would.
            // That is an under-resolution — the direction this file prefers — and nothing in the suite or
            // the 256-crate corpus reaches it.
            if !joined.contains(crate::decls::ALIAS_ALT_SEP) {
                if let Some(stripped) = joined.strip_prefix("crate::") {
                    let s2: Vec<&str> = stripped.split("::").collect();
                    if let Some(q) = qualified_alias(&s2, true, uses) {
                        return q;
                    }
                }
            }
            return joined;
        }
        // R99 — a MODULE-QUALIFIED alias (`facade::Command`, seeded by `seed_mod_aliases`) written from a
        // module where the qualifier IS in scope. AFTER the single-segment `use` lookup, never before: a
        // file that binds the head itself (`use somecrate::facade;`) means ITS `facade`, and answering
        // from the alias map there would attribute the call to the wrong origin. The head is unbound here,
        // so the only thing it can name is a module of this crate.
        if let Some(full) = qualified_alias(&segs, false, uses) {
            return full;
        }
        // A BARE qualifier with no `use` binding is NOT glob-rewritten here: it could be a genuine external
        // crate call (`dotenvy::var`) whose crate identity the classifier still needs — rewriting it under a
        // prelude glob would HIJACK that (sqlx's `dotenvy::var` → lost `Env`). A glob-imported bare name is
        // instead resolved at COLLECT time (its `use crate::name` re-bind, `collect_use`/`rebound`); the
        // only glob path handled HERE is a `crate::`-ROOTED call (below), which is definitively crate-local
        // and so safe to attribute to a re-export glob (iso_C: `crate::net::connect_tcp`).
        return segs.join("::");
    }
    // CRATE-ROOTED resolution via the crate-ROOT re-exports (seeded under `crate::<name>` / `crate::` +
    // GLOB_KEY, see `collect_root_reexports`). A `crate::net::foo`:
    //  1. If the root DIRECTLY re-exports `net` (`pub use x::net`, seeded `crate::net -> x::net`), use it.
    //  2. Else, if the root has EXACTLY ONE external re-export glob (`pub use x::prelude::*`), attribute
    //     `net` to it (`x::prelude::net::foo`) — the name was re-exported into the root by the glob.
    // Both DISCLOSE the origin crate in the κ ledger and let `--deps` chaining recover the effect, matching
    // a DIRECT `use`. ATTRIBUTION only: the tail2 (`net::foo`) is unchanged, so a genuinely-LOCAL `net`
    // module still resolves to its local def downstream (local wins) and stays pure — no fabrication. A
    // `crate::`-rooted path can never be a genuine external-crate call, so this can't hijack one (unlike a
    // bare qualifier). Two-plus globs are ambiguous → honest under-report.
    // R99 — the MULTI-segment form of exactly that lookup, tried FIRST because it is the more specific
    // key: `crate::facade::Command` names the re-exported item, `crate::facade` only names its module. A
    // `crate::`-rooted path can never be an external-crate call, so this cannot hijack one either.
    if let Some(full) = qualified_alias(&segs, true, uses) {
        return full;
    }
    if let Some(full) = uses.get(&format!("crate::{}", segs[0])) {
        return alias_join(full, &segs[1..]); // R105 — may carry several `#[cfg]`-duplicated arms
    }
    // The crate's UNIQUE re-export glob: the seeded root glob (`crate::` + GLOB_KEY, the cross-file case) or,
    // failing that, a glob in THIS file's own `use` map (`GLOB_KEY` — the single-file/collect-time case,
    // iso_A/iso_C). Attribution only; a genuinely-local `net` module still resolves by tail2 downstream.
    if let Some(glob) = root_glob(uses).or_else(|| unique_glob(uses)) {
        return format!("{glob}::{}", segs.join("::"));
    }
    segs.join("::")
}

/// R99 — resolve a MULTI-SEGMENT prefix of `segs` against the module-qualified alias entries
/// `seed_mod_aliases` put in the `use` map (`facade::Command -> std::process::Command`). LONGEST prefix
/// first, so a nested `outer::inner::Command` is preferred over any shorter key that happens to exist.
/// Two segments minimum: a one-segment lookup is `expand`'s existing `use`-map route and must keep its
/// own precedence rules.
fn qualified_alias(segs: &[&str], rooted: bool, uses: &HashMap<String, String>) -> Option<String> {
    for n in (2..=segs.len()).rev() {
        let joined = segs[..n].join("::");
        let key = if rooted { format!("crate::{joined}") } else { joined };
        if let Some(full) = uses.get(&key) {
            return Some(alias_join(full, &segs[n..]));
        }
    }
    module_glob_alias(segs, rooted, uses)
}

/// R99 (SHAPE 1) — resolve `glb::write` through a SUBMODULE's external GLOB re-export
/// (`mod glb { pub use std::fs::*; }`, recorded by `decls::collect_module_glob` under `glb::*glob`).
///
/// Tried only AFTER every exact alias key has missed, because a NAMED re-export of the same leaf is the
/// more specific answer and is also rustc's precedence (an explicit import shadows a glob). Longest module
/// prefix first, for the same reason `qualified_alias` searches that way.
///
/// The entry is `<target>` + `\u{2}`-separated names the module DECLARES ITSELF. A leaf among them is
/// shadowed and MUST NOT be rewritten — see `decls::GLOB_SHADOW_SEP`, where the fabrication this prevents
/// is measured. A `#[cfg]`-duplicated glob carries several arms (R105) and has no single answer, so it is
/// refused rather than picked; unlike a named alias it cannot be distributed over the arms, because the
/// shadow list is a property of one arm's module and joining them would apply one arm's shadows to the
/// other's target.
fn module_glob_alias(segs: &[&str], rooted: bool, uses: &HashMap<String, String>) -> Option<String> {
    for n in (1..segs.len()).rev() {
        let joined = segs[..n].join("::");
        let key = if rooted {
            format!("crate::{joined}::{}", crate::decls::MOD_GLOB_KEY)
        } else {
            format!("{joined}::{}", crate::decls::MOD_GLOB_KEY)
        };
        let Some(entry) = uses.get(&key) else { continue };
        if entry.contains(crate::decls::ALIAS_ALT_SEP) {
            continue;
        }
        let mut parts = entry.split(crate::decls::GLOB_SHADOW_SEP);
        let Some(target) = parts.next().filter(|t| !t.is_empty()) else { continue };
        if parts.any(|s| s == segs[n]) {
            continue; // the module declares this name — the glob is shadowed, and rewriting would fabricate
        }
        return Some(format!("{target}::{}", segs[n..].join("::")));
    }
    None
}

/// R105 — append a caller's trailing segments to an alias target that may carry SEVERAL `#[cfg]`-duplicated
/// alternatives (`decls::record_alias`). The suffix DISTRIBUTES over the arms, so `sys::put` with arms
/// `{std::env::set_var, std::fs::write}` becomes both full callee paths, still joined by `ALIAS_ALT_SEP` —
/// scan.rs's call loop then classifies each and either charges their agreed effect or discloses `Unknown`.
/// A single-arm target — every alias that is not duplicated, which is all but 31 sites in a 256-crate
/// corpus — takes the identical `format!` this replaced, so nothing changes for it.
pub(crate) fn alias_join(full: &str, rest: &[&str]) -> String {
    if rest.is_empty() {
        return full.to_string();
    }
    let tail = rest.join("::");
    if !full.contains(crate::decls::ALIAS_ALT_SEP) {
        return format!("{full}::{tail}");
    }
    full.split(crate::decls::ALIAS_ALT_SEP)
        .map(|a| format!("{a}::{tail}"))
        .collect::<Vec<_>>()
        .join(&crate::decls::ALIAS_ALT_SEP.to_string())
}

/// R99 — seed the crate's MODULE-QUALIFIED external aliases into ONE file's `use` map, under the keys a
/// call written in THAT file can actually spell. `modpath` is the file's own module path.
///
/// Three keys per entry, and the scoping rule behind each is the one `seed_root_reexports` established:
/// never bind a BARE name crate-wide, because a submodule that declares its own `Client` would then have
/// its local type hijacked by a root `pub type Client = reqwest::Client` — the misattribution direction.
///
///  * `crate::<qualified>` — always. `crate::facade::Command` / `crate::Cmd` name the item from anywhere,
///    and a `crate::`-rooted path always names THIS crate, so it cannot hijack an external call — the
///    same property `expand`'s existing crate-rooted branch already rests on, not a new claim.
///  * `<qualified>` relative to THIS file's module — the spelling an ANCESTOR module writes
///    (`facade::Command` from the crate root; `inner::Command` from inside `outer`). Two-plus segments,
///    so it is only reachable through `qualified_alias`.
///  * the BARE name, and only when this file IS the declaring module — where the name is genuinely in
///    scope. (Usually redundant with the file's own `use`/`type` item; not redundant when the declaration
///    is an inline submodule of the same file, or the item appears after its first use.)
pub(crate) fn seed_mod_aliases(
    aliases: &HashMap<String, String>,
    modpath: &str,
    uses: &mut HashMap<String, String>,
) {
    for (qualified, target) in aliases {
        uses.insert(format!("crate::{qualified}"), target.clone());
        let (module, name) = match qualified.rsplit_once("::") {
            Some((m, n)) => (m, n),
            None => ("", qualified.as_str()),
        };
        if module == modpath {
            uses.insert(name.to_string(), target.clone());
        } else if modpath.is_empty() {
            uses.insert(qualified.clone(), target.clone());
        } else if let Some(rel) = module.strip_prefix(&format!("{modpath}::")) {
            uses.insert(format!("{rel}::{name}"), target.clone());
        }
    }
}

/// The single crate-ROOT re-export glob (seeded under `crate::` + `GLOB_KEY`), if unambiguous — see
/// `collect_root_reexports` and `expand`'s crate-rooted branch.
fn root_glob(uses: &HashMap<String, String>) -> Option<&str> {
    let list = uses.get(&format!("crate::{GLOB_KEY}"))?;
    let mut it = list.split('\u{1}');
    let first = it.next()?;
    if it.next().is_some() {
        return None;
    }
    Some(first)
}

/// ⟨0.29⟩ The string literal at ARGUMENT POSITION `idx`, or None when that argument is anything else.
///
/// It REPLACES `first_str_lit`, which scanned the whole list and returned the first literal it found —
/// the right answer for nothing, and the wrong answer for a security gate: `fs::write(user_path,
/// "/tmp/lit")` yielded the CONTENTS as the path surface, so an allowlist certified a write to a runtime
/// destination. A call's meaningful literal is at a KNOWN position — the path, the host, the command, the
/// query are all argument 0 — so read that position and let the absence of a literal there mean what it
/// should. The old helper is DELETED rather than left unused: a `first literal anywhere` sitting in scope
/// is how this class comes back at the next call site somebody adds.
pub(crate) fn positional_str_lit(
    args: &syn::punctuated::Punctuated<syn::Expr, syn::token::Comma>,
    idx: usize,
) -> Option<String> {
    match args.iter().nth(idx) {
        Some(syn::Expr::Lit(syn::ExprLit { lit: syn::Lit::Str(s), .. })) => {
            let v = s.value();
            if v.trim().is_empty() { None } else { Some(v) }
        }
        _ => None,
    }
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

/// LITERAL-HEAD HOST EXTRACTION (SPEC §1 static-host): the host from a `format!(FMT, args…)` whose FORMAT
/// STRING literal already SPELLS OUT a complete authority BEFORE its first interpolation hole — the most
/// common real-world URL shape `format!("https://api.openai.com/v1/{}", path)`, where the host is fully
/// present in the literal and only the PATH is interpolated. Returns the host (`api.openai.com`, `:port`
/// stripped) ONLY when the static prefix — the text before the first `{}`/`{name}` hole — contains a
/// COMPLETE authority: a `<scheme>://<authority>/…` with a `/` AFTER the `://` and WITHIN the prefix. That
/// trailing `/` is the proof the authority is TERMINATED in the literal (no hole can have leaked into the
/// host). Returns `None` — leaving the call bare Net with the host masked, exactly as today — when:
///   • there is no `://` in the prefix, or no `/` after it (`format!("https://{}/v1/y", h)`,
///     `format!("https://api.{}.com/y", x)`, `format!("https://api.openai{}/v1", x)`,
///     `format!("https://api.openai.com:{}/v1", port)` — the authority is NOT terminated before a hole);
///   • the format string has no leading static text before the first hole (that is the const-anchored
///     `{}`-at-head case, resolved separately by `format_const_prefix_arg` — this helper defers to it).
/// NO FABRICATION: the returned host is a substring of the LITERAL format string, never a resolved value.
/// The host still runs through the caller's `is_model_host` refinement, so a non-model literal (a CDN)
/// captures the host but stays bare Net.
pub(crate) fn format_literal_head_host(m: &syn::Macro) -> Option<String> {
    if !is_format_macro(m.path.segments.last()?.ident.to_string().as_str()) {
        return None;
    }
    let parsed: syn::punctuated::Punctuated<syn::Expr, syn::Token![,]> =
        m.parse_body_with(syn::punctuated::Punctuated::parse_terminated).ok()?;
    let syn::Expr::Lit(syn::ExprLit { lit: syn::Lit::Str(fmt), .. }) = parsed.iter().next()? else {
        return None;
    };
    literal_head_host(&fmt.value())
}

/// The host in a format-string's STATIC PREFIX — the text before its first interpolation hole — when that
/// prefix already contains a COMPLETE `<scheme>://<authority>/…` authority. Shared by the `format!` head
/// extraction; factored out so it is unit-testable in isolation. `{{`/`}}` are ESCAPED braces (literal
/// text, not holes); the first UNESCAPED `{` ends the static prefix.
pub(crate) fn literal_head_host(fmt: &str) -> Option<String> {
    // The static prefix = text up to the first UNESCAPED `{`. `{{` is a literal brace, so consume it and
    // keep going; a lone `{` opens a hole and terminates the prefix.
    let mut prefix = String::new();
    let mut chars = fmt.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '{' if chars.peek() == Some(&'{') => {
                chars.next(); // an escaped `{{` → one literal `{`
                prefix.push('{');
            }
            '}' if chars.peek() == Some(&'}') => {
                chars.next(); // an escaped `}}` → one literal `}`
                prefix.push('}');
            }
            '{' => break, // a real hole opens here — the static prefix ends
            _ => prefix.push(c),
        }
    }
    // The authority is complete ONLY when there is a `/` AFTER the `://` WITHIN the prefix — that `/`
    // proves the authority is terminated in the literal (no hole leaked into the host). Absent it, a hole
    // could sit inside the authority (`https://api.{}.com/`, `https://{}/`, `https://host:{}/`) → bail.
    let after_scheme = prefix.split_once("://")?.1;
    let authority = after_scheme.split_once('/')?.0;
    // Strip `:port` and any `user@` — the routable host, matching `host_part`. Reject an empty authority.
    let host = authority.rsplit_once('@').map(|(_, h)| h).unwrap_or(authority);
    let host = host.split_once(':').map(|(h, _)| h).unwrap_or(host);
    (!host.trim().is_empty()).then(|| host.to_string())
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

/// The (bound name, variant leaf) of a single-field tuple-variant pattern `Variant(x)` /
/// `Enum::Variant(x)` — generalises `some_ok_binding` beyond `Some`/`Ok` to ANY tuple-variant leaf (R77),
/// so a caller can look the leaf up in EITHER `EnumVariantIndex` (a plain payload type) or
/// `EnumVariantTraitIndex` (a `dyn`/`impl`/bounded-generic payload's dispatch leaves) without this parser
/// needing to know which. `None` when the pattern isn't a single-field tuple-struct with a single-ident
/// binding — a multi-field or destructuring payload has no single receiver to type, an honest
/// under-report. Peels a reference/paren wrapper first (`Some(Variant(x))`-style nesting is rare).
pub(crate) fn tuple_variant_binding(pat: &syn::Pat) -> Option<(String, String)> {
    match pat {
        syn::Pat::Reference(r) => tuple_variant_binding(&r.pat),
        syn::Pat::Paren(p) => tuple_variant_binding(&p.pat),
        syn::Pat::TupleStruct(ts) if ts.elems.len() == 1 => {
            let name = single_pat_ident(ts.elems.first()?)?;
            let leaf = ts.path.segments.last()?.ident.to_string();
            Some((name, leaf))
        }
        _ => None,
    }
}

/// The (bound name, `"VariantLeaf::field"` composite key) pairs of a STRUCT-VARIANT pattern
/// (`Msg::CbField { f }`, `Msg::CbField { f: renamed }`, `Msg::CbField { f, .. }`, `Msg::Both { f, g }`)
/// — the struct-variant counterpart of `tuple_variant_binding`, generalised to MULTIPLE simultaneous
/// bindings since a struct-variant pattern can name several fields at once (R77 residual: no
/// struct-variant binder existed at all before this, for any payload type).
///
/// One pair per field the pattern actually binds to a single ident; a `..` rest is simply not iterated
/// (Rust's own partial-destructure semantics — the omitted fields bind nothing, an honest under-report
/// unchanged from before). A field bound to a non-single-ident sub-pattern (`Msg::CbField { f: (a, b) }`)
/// contributes nothing for that field — same discipline as `tuple_variant_binding`'s `None` result for a
/// multi-field/destructuring payload. `ref`/`@` bindings resolve through `single_pat_ident`, which reads
/// only the `Pat::Ident`'s own bound name, ignoring `by_ref`/`subpat`.
///
/// The composite key is deliberate reuse, not a new index: Rust identifiers never contain `::`, so
/// `"Leaf::field"` can never collide with a bare tuple-variant leaf already stored in
/// `EnumVariantIndex`/`EnumVariantTraitIndex` — see `enum_struct_variant_bindings` in `collector.rs` and
/// the matching Pass-A write site in `decls.rs`'s enum branch. Peels reference/paren wrappers first, like
/// `tuple_variant_binding`.
pub(crate) fn struct_variant_field_bindings(pat: &syn::Pat) -> Vec<(String, String)> {
    match pat {
        syn::Pat::Reference(r) => struct_variant_field_bindings(&r.pat),
        syn::Pat::Paren(p) => struct_variant_field_bindings(&p.pat),
        syn::Pat::Struct(ps) => {
            let Some(leaf) = ps.path.segments.last().map(|s| s.ident.to_string()) else {
                return Vec::new();
            };
            ps.fields
                .iter()
                .filter_map(|fp| {
                    let field = match &fp.member {
                        syn::Member::Named(id) => id.to_string(),
                        syn::Member::Unnamed(idx) => idx.index.to_string(),
                    };
                    single_pat_ident(&fp.pat).map(|name| (name, format!("{leaf}::{field}")))
                })
                .collect()
        }
        _ => Vec::new(),
    }
}

/// The single-ident binding of a `Some(x)` / `Ok(x)` pattern (the payload of an `if let`/`let-else`
/// unwrap of an `Option`/`Result`) — so `if let Some(d) = o { d.go() }` over an `Option<Box<dyn T>>`
/// types `d` for dispatch. `None` for any other pattern (a `None`/`Err` arm, a multi-field or
/// non-single-ident payload — an honest under-report). Peels reference/paren wrappers.
pub(crate) fn some_ok_binding(pat: &syn::Pat) -> Option<String> {
    match pat {
        syn::Pat::Reference(r) => some_ok_binding(&r.pat),
        syn::Pat::Paren(p) => some_ok_binding(&p.pat),
        syn::Pat::TupleStruct(ts) if ts.elems.len() == 1 => {
            let variant = ts.path.segments.last()?.ident.to_string();
            if variant == "Some" || variant == "Ok" {
                single_pat_ident(ts.elems.first()?)
            } else {
                None
            }
        }
        _ => None,
    }
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

/// `test` is FALSE and every other predicate is UNKNOWN — the leaf rule that decides whether a cfg
/// predicate can hold in the non-test build the default scan describes. Fed to [`cfg_fold`].
///
/// Non-`test` leaves stay UNKNOWN on purpose, including `miri`, `doc` and `doctest`: they are not
/// decidable here, and the failure direction of guessing them false is a SILENT UNDER-REPORT, which is
/// the sin this rule exists to prevent (R122). `any(test, miri)` is therefore SCANNED — over-reporting
/// a genuinely test-only item is the affordable half of the trade. Feature predicates are likewise
/// unknown to this rule; `is_cfg_inactive` resolves those separately, against the same attrs.
fn cfg_leaf_test_off(m: &syn::meta::ParseNestedMeta) -> Option<bool> {
    // CONSUME a `= …` tail before returning. `parse_nested_meta` raises its error AFTER this callback
    // returns, and an unconsumed tail aborts the whole sibling iteration — so a leaf that ignores the
    // value silently truncates the predicate at the first name-value child, and `all(feature = "x",
    // test)` was read as NOT test-only purely because `test` was typed second. The `feature` leaf in
    // `cfg_eval` consumes it for the same reason; this one does not care what the value IS.
    //
    // Stepped over as raw token trees rather than `parse::<syn::Lit>()`: syn's negative-literal path
    // BUILDS a new literal token, and this AST may have been parsed on another thread — the fixture
    // `a_cfg_attribute_reparse_survives_the_ast_crossing_a_thread_boundary` (`feature = -1`) panics on
    // that, and it panicked here first. Moving the cursor touches no source map.
    if m.input.peek(syn::Token![=]) {
        let _ = m.input.parse::<syn::Token![=]>();
        while !m.input.is_empty() && !m.input.peek(syn::Token![,]) {
            if m.input.parse::<proc_macro2::TokenTree>().is_err() {
                break;
            }
        }
        return None;
    }
    if m.path.is_ident("test") { Some(false) } else { None }
}

/// The 3-valued (Kleene) fold over a `#[cfg(...)]` predicate tree: `not`/`all`/`any` to any depth, with
/// `leaf` deciding everything else. `Some(true)`/`Some(false)` are DEFINITE; `None` is "unresolvable",
/// and every caller must read `None` as *keep the item*.
///
/// ONE fold, two leaf rules ([`cfg_leaf_test_off`] and the feature rule in [`cfg_eval`]), because the
/// two used to be written out separately and DRIFTED: the `test` copy treated `any` and `all` alike,
/// so `#[cfg(any(test, feature = "x"))]` — production code whenever `x` is on — was classified test-only
/// and erased from the report (SOUNDNESS R122, a published cardinal sin). `any` and `all` differ, and
/// the only way they cannot drift apart again is for there to be one of them.
///
/// (a `parse_nested_meta` on a child may error on a non-meta tail; the error is swallowed exactly as
/// before, so a partially-parsed group folds over the children it did see.)
fn cfg_fold(m: &syn::meta::ParseNestedMeta,
            leaf: &dyn Fn(&syn::meta::ParseNestedMeta) -> Option<bool>) -> Option<bool> {
    if m.path.is_ident("not") {
        let mut inner: Option<bool> = None;
        let _ = m.parse_nested_meta(|n| { inner = cfg_fold(&n, leaf); Ok(()) });
        return inner.map(|b| !b);
    }
    if m.path.is_ident("all") {
        // false if ANY child false; true only if ALL true; else None.
        let (mut any_false, mut all_true, mut saw) = (false, true, false);
        let _ = m.parse_nested_meta(|n| { saw = true; match cfg_fold(&n, leaf) { Some(false) => any_false = true, Some(true) => {}, None => all_true = false }; Ok(()) });
        if any_false { return Some(false); }
        if saw && all_true { return Some(true); }
        return None;
    }
    if m.path.is_ident("any") {
        // true if ANY child true; false only if ALL false; else None.
        let (mut any_true, mut all_false, mut saw) = (false, true, false);
        let _ = m.parse_nested_meta(|n| { saw = true; match cfg_fold(&n, leaf) { Some(true) => any_true = true, Some(false) => {}, None => all_false = false }; Ok(()) });
        if any_true { return Some(true); }
        if saw && all_false { return Some(false); }
        return None;
    }
    leaf(m)
}

/// TEST-ONLY: `Some(false)` from folding the predicate with `test = false` — i.e. this `#[cfg(...)]`
/// CANNOT be satisfied in a non-test build, whatever the features and target are.
pub(crate) fn cfg_meta_is_test_only(m: &syn::meta::ParseNestedMeta) -> bool {
    cfg_fold(m, &cfg_leaf_test_off) == Some(false)
}

/// True if an item carries a `#[cfg(...)]` under which the item cannot exist in a NON-TEST build — a
/// test-only item the default scan skips, since its effects describe the crate's TESTS, not the crate.
///
/// `#[cfg(test)]` and `#[cfg(all(test, unix))]` are test-only. `#[cfg(not(test))]`,
/// `#[cfg(all(unix, not(test)))]` and — R122 — `#[cfg(any(test, feature = "x"))]` are NOT: the last one
/// compiles into an ordinary build whenever `x` is on, and `std`/`alloc`/`derive` usually are.
pub(crate) fn is_cfg_test(attrs: &[syn::Attribute]) -> bool {
    attrs.iter().any(|a| {
        a.path().is_ident("cfg") && {
            let mut found = false;
            let _ = a.parse_nested_meta(|m| {
                if cfg_meta_is_test_only(&m) {
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
    // Same `not`/`all`/`any` Kleene fold as the test-only rule (see `cfg_fold`); only the LEAF differs.
    cfg_fold(m, &|n| {
        if n.path.is_ident("feature") {
            // `feature = "X"` → active⇒Some(true), declared-but-inactive⇒Some(false), undeclared⇒None.
            let v = n.value().ok().and_then(|v| v.parse::<syn::LitStr>().ok());
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
        None // target_os/unix/windows/test/… — unknown to a default-feature scan; keep the item.
    })
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

/// SOUNDNESS R160 — the key under which `scan_items` binds `Self` to the ENCLOSING impl's type (or, in a
/// trait's default body, to the trait) for the length of that block. It is the literal token `Self`, which
/// is a Rust KEYWORD and therefore can never be introduced by a `use`, a `type` alias or a declaration, so
/// it cannot collide with a real imported name and needs no sentinel escape the way `GLOB_KEY` does. It
/// lives in the ordinary `use` map on purpose: `expand` is the single place a written path becomes a
/// resolved one, so every position — assoc-fn call, assoc const, enum variant, struct literal, a
/// `Self::f` passed as a value — is answered by that one authority rather than by a second arm per
/// position.
pub(crate) const SELF_KEY: &str = "Self";

/// Reserved key under which `collect_use` records GLOB re-export PATHs (`use x::y::*`) in the `use` map.
/// `*` can never be a Rust identifier, so it never collides with a real imported name. The value is a
/// `\u{1}`-separated list of the glob PATHs whose ROOT is an external crate (not `crate`/`self`/`super`) —
/// the ones that let `expand` attribute an otherwise-unresolved qualifier to its ORIGIN crate. See
/// `expand`'s glob-fallback: a call `net::foo` where `net` was brought in by `use mycore::prelude::*`
/// resolves to `mycore::prelude::net::foo` (tail2 `net::foo`, crate `mycore`), so the origin crate is
/// disclosed in the ledger and `--deps` chaining recovers the effect — parity with a DIRECT `use`.
pub(crate) const GLOB_KEY: &str = "*";

pub(crate) fn collect_use(tree: &syn::UseTree, prefix: String, out: &mut HashMap<String, String>) {
    let join = |p: &str, s: &str| if p.is_empty() { s.to_string() } else { format!("{p}::{s}") };
    // A crate-LOCAL re-bind (`use crate::net`, `use super::net`) names a target in THIS crate. Store what
    // that target ALREADY resolves to in the (inherited) `use` map rather than the literal `crate::net`,
    // which names no local def and reads silent-pure (iso_B): a submodule's `use crate::net` must inherit
    // the crate-root `net` binding — a direct `use mycore::net` (→ `mycore::net`) or a glob re-export
    // (→ `mycore::…::net`). Resolving the FULL rebind path through `expand` (which follows the glob
    // fallback) recovers the origin crate; a target that resolves to nothing local is stored as-is.
    let rebound = |full: &str, out: &HashMap<String, String>| -> String {
        // ONLY a `crate::`-rooted re-bind (`use crate::net`) is re-resolved: `crate::X` names the CRATE
        // ROOT, where a re-export can bring an external name into scope. `self::`/`super::` are RELATIVE to
        // the current module (whose path `collect_use` doesn't know) — a `use super::core::foo` must keep
        // its literal so downstream tail2 resolution (`core::foo`) links it to the local def; re-resolving
        // it here would DROP the module context and break that edge (clap's `super::core::display_width`).
        // A non-`crate` path is likewise authoritative and stored as-is.
        if full.split("::").next() != Some("crate") {
            return full.to_string();
        }
        // `crate::net` — does the crate ROOT re-export `net`? Consult the seeded root re-exports via
        // `expand` (a crate-root DIRECT re-export `pub use x::net`, iso_B/iso_D/reqwest; or the crate's
        // UNIQUE re-export glob `pub use x::…::*`, iso_A/iso_C/sqlx). `expand` strips the `crate::` root, so
        // an UNRESOLVED `crate::net` comes back as the bare local path `net` (unchanged meaning); a FIRED
        // re-export comes back rooted at the external crate (`mycore::…::net`, `http::header`). Take the
        // resolved value ONLY when a re-export actually fired — else keep the literal `crate::net` so a
        // genuine crate-local `net` module still resolves by tail2 (no meaning change, no fabrication).
        // This closes the cardinal-sin hole: a cross-crate effect reached via a root re-export was read
        // silent-pure because `crate::net` named no local def and disclosed no origin crate.
        let local = full.strip_prefix("crate::").unwrap_or(full);
        let resolved = expand(full, out);
        if resolved == local { full.to_string() } else { resolved }
    };
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
                    let v = rebound(&prefix, out);
                    out.insert(last.to_string(), v);
                }
            } else {
                let v = rebound(&join(&prefix, &id), out);
                out.insert(id.clone(), v);
            }
        }
        syn::UseTree::Rename(r) => {
            let v = rebound(&join(&prefix, &r.ident.to_string()), out);
            out.insert(r.rename.to_string(), v);
        }
        syn::UseTree::Group(g) => {
            for t in &g.items {
                collect_use(t, prefix.clone(), out);
            }
        }
        // A GLOB re-export `use PATH::*` brings PATH's public items into scope under their own names. We
        // can't enumerate those names syntactically (PATH's source may be an unscanned external crate), but
        // a later call `name::foo` — where `name` resolves to no local module and no direct `use` — is
        // then attributable to PATH's crate (`PATH::name::foo`). Record the glob PATH so `expand` can apply
        // it as a fallback. ONLY external-rooted globs (`mycore::driver_prelude::*`) are recorded: a
        // crate-LOCAL glob (`crate::prelude::*`) attributes to no external crate, and its names resolve
        // locally through tail2 anyway. Without this, a cross-crate effectful call reached via a driver-
        // prelude glob (`use sqlx_core::driver_prelude::*; net::connect(..)`) read SILENT-PURE and was
        // disclosed NOWHERE — the cardinal sin (sqlx `PgStream::connect`).
        syn::UseTree::Glob(_) => {
            let rooted_local = matches!(prefix.split("::").next(), Some("crate" | "self" | "super"));
            if !prefix.is_empty() && !rooted_local {
                let e = out.entry(GLOB_KEY.to_string()).or_default();
                // `\u{1}`-separated list (a char that can't appear in a path) — a module may have several.
                if e.is_empty() {
                    *e = prefix;
                } else {
                    e.push('\u{1}');
                    e.push_str(&prefix);
                }
            }
        }
    }
}

/// The CRATE-ROOT re-exports that a `use crate::name` in ANY file (even another one) resolves through —
/// a crate is scanned FILE-BY-FILE with a fresh `use` map per file, so a submodule's `use crate::net`
/// otherwise can't see that the crate ROOT re-exported `net` (from a `pub use x::prelude::*` glob or a
/// `pub use x::net`). Collected once from the root file (`lib.rs`/`main.rs`, module path ""), keyed by
/// the introduced name, and seeded into every file's `use` map under `crate::<name>` so resolution of a
/// crate-rooted path finds them WITHOUT letting a bare `net::foo` (which Rust would NOT resolve to a root
/// re-export) pick them up. The single glob is stored under `crate::` + `GLOB_KEY`. Real-world seam:
/// sqlx-postgres re-exports the whole `sqlx_core::driver_prelude::*` at its root; every driver file then
/// does `use crate::net; net::connect_tcp(..)` — the TCP dial that read SILENT-PURE before this.
pub(crate) fn collect_root_reexports(items: &[syn::Item], include_tests: bool) -> HashMap<String, String> {
    let mut m = HashMap::new();
    collect_item_uses(items, include_tests, &mut m);
    m
}

/// SOUNDNESS R123 — THE ONE RULE for "does this `use` item bind a name in the build we are describing".
///
/// `collect_use` inserts into a `HashMap`, so the LAST spelling of a name wins, and the idiomatic
/// mocking pair is two MUTUALLY EXCLUSIVE `cfg`s:
///
///     #[cfg(not(test))] use std::process::Command as Runner;
///     #[cfg(test)]      use crate::mockproc::Runner;      // <- typed second, so the MOCK won
///
/// With the test arm collected, a PRODUCTION scan resolved `Runner::new(p).status()` through a mock that
/// is pure by construction: `run` vanished from `functions[]` entirely, and which answer you got was
/// decided by SOURCE ORDER. Measured on the published binary, over two crates identical in every byte
/// but the order of those two lines, BOTH compiled and RUN in a normal (non-test) build — each printed
/// `ran=true`, i.e. each really spawned `/usr/bin/true`.
///
/// FIVE SITES ANSWERED THIS QUESTION AND ONLY TWO APPLIED THE FILTER (`collect_module_glob`,
/// `collect_reexports`). `scan_items`, `collect_decls` and `collect_root_reexports` did not, and the
/// last had no `include_tests` PARAMETER to apply — the one site of the five that could not even express
/// the question. They all call this now, so there is one authority rather than five hand-rolled loops
/// free to drift apart again.
pub(crate) fn use_item_applies(u: &syn::ItemUse, include_tests: bool) -> bool {
    include_tests || !is_cfg_test(&u.attrs)
}

/// Collect every module-level `use` that [`use_item_applies`] admits into `out`.
pub(crate) fn collect_item_uses(
    items: &[syn::Item],
    include_tests: bool,
    out: &mut HashMap<String, String>,
) {
    for it in items {
        if let syn::Item::Use(u) = it {
            if use_item_applies(u, include_tests) {
                collect_use(&u.tree, String::new(), out);
            }
        }
    }
}

/// SOUNDNESS R128 — the MODULES whose item list this scan could not read in full, because an
/// item-position MACRO INVOCATION sits in it and `syn` leaves a macro body opaque.
///
/// A macro at item position can declare anything: a `pub fn`, a `pub(crate) use` re-export, a whole
/// `impl`. `collect_decls` deliberately skips those (see its `Item::Macro` arm — only a `macro_rules!`
/// DEFINITION, which carries an `ident`, is recorded), so the module's `by_tail2`/re-export entries are
/// INCOMPLETE and candor cannot tell "this module has no such name" from "the macro declared it".
/// Recording which modules are in that state is what lets the call resolver DISCLOSE the difference
/// instead of reading the absence as purity — see the R128 hedge in `scan.rs`.
///
/// The three shapes measured, each compiled and RUN spawning a real process, each of which left the
/// CALLER absent from `functions[]` before this existed:
///   * `mod m { macro_rules! r { () => { pub(crate) use crate::real::f; } } r!(); }` — tokio's own
///     `cfg_rt! { pub(crate) use crate::runtime::spawn_blocking; }` in `src/blocking.rs`.
///   * `mod m { defit!(); }` where the macro declares the `pub fn` itself — the worst of the three: the
///     TARGET has no report row either, so blanket `deny Exec` also exits 0.
///   * `mod m { include!("gen.rs"); }` — the `include!`/`OUT_DIR` build-script convention. An
///     `Item::Macro` like any other, so it needs no separate rule.
///
/// A `macro_rules!` DEFINITION is not an invocation and declares nothing by itself, so it does not mark
/// the module — only `ident: None` items, which is exactly `collect_decls`'s own skip condition. Reading
/// the same syn shape from the same predicate is deliberate: the index must mark precisely the modules
/// whose items were skipped, so a future arm that starts EXPANDING one of these shapes narrows both.
pub(crate) fn collect_macro_modules(
    items: &[syn::Item],
    modpath: &str,
    include_tests: bool,
    out: &mut std::collections::HashSet<String>,
) {
    for it in items {
        match it {
            syn::Item::Macro(m) if m.ident.is_none() && (include_tests || !is_cfg_test(&m.attrs)) => {
                out.insert(modpath.to_string());
            }
            syn::Item::Mod(m) if include_tests || !is_cfg_test(&m.attrs) => {
                if let Some((_, inner)) = &m.content {
                    let sub = if modpath.is_empty() {
                        m.ident.to_string()
                    } else {
                        format!("{modpath}::{}", m.ident)
                    };
                    collect_macro_modules(inner, &sub, include_tests, out);
                }
            }
            _ => {}
        }
    }
}

/// Build a per-file `use` map seeded with the crate-ROOT re-exports under `crate::<name>` keys (the root
/// glob under `crate::` + `GLOB_KEY`). A `use crate::net` / `crate::net::foo` in the file then resolves
/// through the root re-export via `expand`, while a bare `net::foo` — which never keys on `crate::…` —
/// keeps its own crate identity. Called once per file at Pass B; the returned map is where `scan_items`
/// then accumulates the file's own `use` statements.
pub(crate) fn seed_root_reexports(root: &HashMap<String, String>) -> HashMap<String, String> {
    let mut m = HashMap::with_capacity(root.len());
    for (k, v) in root {
        m.insert(format!("crate::{k}"), v.clone());
    }
    m
}

/// The external-rooted GLOB re-export PATHs recorded in a `use` map, if EXACTLY ONE — the unambiguous
/// origin `expand` can attribute an otherwise-unresolved qualifier to. Zero (no glob) or two-plus
/// (ambiguous — never guess which prelude a name came from; the honest under-report, matching the
/// keying-collision discipline elsewhere) both yield `None`.
fn unique_glob(uses: &HashMap<String, String>) -> Option<&str> {
    let list = uses.get(GLOB_KEY)?;
    let mut it = list.split('\u{1}');
    let first = it.next()?;
    if it.next().is_some() {
        return None; // 2+ globs — ambiguous origin
    }
    Some(first)
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

// ─────────────────────────────────────────────────────────────────────────────────────────────────
// DROP-GLUE: the ONE place that says what a construction expression BUILDS, and the ONE place that
// says whether that value ESCAPES the scope that built it.
//
// These two questions used to be answered in three places that had drifted apart — an assoc-fn CALL
// route in scan.rs (construction-keyed, sound), a `T::<construct>` marker in collector.rs emitted only
// under `Pat::Ident` (BINDER-keyed, so 16 of 17 positions were silent and the tuple-struct/newtype
// spelling had no route at all), and an escape gate that existed on the field route and not the direct
// one. Two paths computing one fact are free to disagree, and that is exactly how the vein opened; the
// rule is now stated once, here, and every caller asks it rather than re-deriving it.
// ─────────────────────────────────────────────────────────────────────────────────────────────────

/// UpperCamel = TYPE-shaped, distinguishing `Guard` from a `snake_case` local and a `SCREAMING_SNAKE`
/// const. A single CHARACTER counts (`struct S;`), counted in `chars()` not bytes so a non-ASCII single
/// -codepoint ident (`struct É;`) is not read as multi-segment.
pub(crate) fn is_type_ident(s: &str) -> bool {
    let mut ch = s.chars();
    ch.next().is_some_and(|c| c.is_uppercase())
        && (s.chars().count() == 1 || s.chars().any(|c| c.is_lowercase()))
}

/// The type LEAF a CALLEE path constructs: a tuple-struct / tuple-variant literal (`Guard(f)`,
/// `E::V(f)`) or an associated-fn call on a nominal type (`Guard::new()`, `a::b::Guard::open(p)`).
/// `None` for a free fn (`compute()`), a module fn (`serde_json::from_str`) and `Type::drop` itself.
///
/// The tuple-struct arm is what the assoc-fn route could NEVER see: `Guard(f)` is a single-segment
/// `Expr::Call`, so it fails a `contains("::")` test, and `use m::Guard; Guard(f)` expands to
/// `m::Guard`, whose `tail2` head is the MODULE. It fell between both routes in every position,
/// including the bound local — and the newtype guard is the commonest effectful-`Drop` shape in Rust.
pub(crate) fn ctor_leaf_from_call_path(full: &str, uses: &HashMap<String, String>) -> Option<String> {
    let full = expand(full, uses);
    // `Guard(f)` / `E::V(f)` — the callee path IS the value's type path.
    if let Some(t) = type_from_value_path(&full, uses) {
        return local_type_leaf(&t);
    }
    // `Type::assoc(..)`. NOT restricted to `is_ctor` names: a guard type's `open_at`/`acquire`/`spawn`
    // returns `Self` just as `new` does, and narrowing to the constructor-name list here would convert
    // the shipped, measured behaviour of the assoc-fn route into a fresh crop of silent under-reports.
    let (head, last) = full.rsplit_once("::")?;
    if last == "drop" {
        return None;
    }
    let ty = head.rsplit("::").next().unwrap_or(head);
    if !is_type_ident(ty) {
        return None;
    }
    local_type_leaf(head)
}

/// The LEAF of a type path, unless the path is rooted in `std`/`core`/`alloc` — in which case it names
/// no local type and a leaf COLLISION with one would fabricate. `drop_types` is leaf-keyed (it has to
/// be: `type_path` produces leaves), so the collision is invisible one layer down and has to be refused
/// here, where the full path is still in hand.
///
/// MEASURED, not hypothetical. tokio declares `impl Drop for Acquire<'_>` (a tracing-instrumented
/// future) AND imports `std::sync::atomic::Ordering::Acquire`. `self.permits.load(Acquire)` writes the
/// enum variant as a bare value path, which is a construction spelling — so every `is_closed`,
/// `is_idle`, `available_permits` in `batch_semaphore`/`mpsc` picked up the FUTURE's `Log` + `Unknown`
/// off an atomic ordering constant. Expanding the path before classifying is what makes
/// `type_from_value_path`'s existing `Enum::Variant` rule fire and answer `Ordering` instead.
fn local_type_leaf(ty: &str) -> Option<String> {
    if matches!(ty.split("::").next(), Some("std") | Some("core") | Some("alloc")) {
        return None;
    }
    Some(ty.rsplit("::").next().unwrap_or(ty).to_string())
}

/// The type LEAF a bare VALUE path denotes: a unit struct (`Guard`) or a unit enum variant
/// (`State::Open`, whose value's type is the ENUM). `None` for a local, a const, a module path.
pub(crate) fn ctor_leaf_from_value_path(
    full: &str,
    uses: &HashMap<String, String>,
    fields: &FieldIndex,
) -> Option<String> {
    let full = expand(full, uses);
    let leaf = type_from_value_path(&full, uses).as_deref().and_then(local_type_leaf)?;
    // A bare value path constructs only a FIELDLESS type — a unit struct (`UnitGuard`) or an enum
    // variant (`State::Open`). A struct WITH fields cannot be written as a bare path at all, so a path
    // resolving to one is a name collision, not a construction.
    //
    // MEASURED on tokio, and it is exactly the collision the leaf keying invites. `batch_semaphore.rs`
    // has `use std::sync::atomic::Ordering::*;` — a GLOB, so `Acquire` in `self.permits.load(Acquire)`
    // does not expand — beside `pub(crate) struct Acquire<'a> { .. }` with a tracing-instrumented
    // `impl Drop`. Every `is_closed`/`is_idle`/`available_permits` in `batch_semaphore` and `mpsc` read
    // the atomic-ordering CONSTANT as a construction of the FUTURE and inherited its `Log` + `Unknown`.
    // The real `Acquire` has fields; the constant does not name a type at all.
    let variant = {
        let segs: Vec<&str> = full.split("::").collect();
        segs.len() >= 2 && is_type_ident(segs[segs.len() - 2]) && is_type_ident(segs[segs.len() - 1])
    };
    if !variant && fields.get(&leaf).is_some_and(|m| !m.is_empty()) {
        return None;
    }
    Some(leaf)
}

/// The type LEAF a construction EXPRESSION builds — the expression-shaped form of the two path
/// functions above, used by the escape pre-pass (which walks raw syntax rather than the collector's
/// already-expanded call paths).
pub(crate) fn ctor_leaf_of_expr(
    expr: &syn::Expr,
    uses: &HashMap<String, String>,
    fields: &FieldIndex,
    // SOUNDNESS R165 — the crate-wide fn-leaf -> return-type index, so a FREE-FUNCTION constructor is
    // the same construction to this authority as `Type::assoc()`. Threaded here rather than added at
    // the marker's call site alone: this fn is read by BOTH the marker and the ESCAPE GATE
    // (`mark_escape`), and widening only the marker made `fn forwards(p) -> H { from_handle(p) }`
    // fabricate a Drop the caller runs — measured on the fixture's own over-charge control, which is
    // why that control exists. Two paths computing one fact are free to disagree; this is the one path.
    returns: &ReturnIndex,
) -> Option<String> {
    match expr {
        // A struct LITERAL names its type directly, so the fieldless test above does not apply to it
        // (`Guard { .. }` is a construction precisely because it has fields).
        syn::Expr::Struct(s) => {
            let full = expand(&path_to_string(&s.path), uses);
            type_from_value_path(&full, uses).as_deref().and_then(local_type_leaf)
        }
        syn::Expr::Path(p) if p.qself.is_none() => {
            ctor_leaf_from_value_path(&path_to_string(&p.path), uses, fields)
        }
        syn::Expr::Call(c) => {
            let syn::Expr::Path(p) = &*c.func else { return None };
            let written = path_to_string(&p.path);
            ctor_leaf_from_call_path(&written, uses)
                .or_else(|| ctor_leaf_from_call_returns(&expand(&written, uses), returns))
        }
        _ => None,
    }
}

// ─────────────────────────────────────────────────────────────────────────────────────────────────
// CROSS-CRATE DROP-GLUE — SOUNDNESS R68(1). The three functions above answer "what type leaf does this
// construction build" for the IN-CRATE authority (`CallCollector::note_construction`, gated on
// `drop_relevant`, which can only ever be built from `impl Drop` blocks this scan actually parsed — so a
// DEPENDENCY's drop-relevant type is invisible to it by construction, no matter how the site is spelled).
// The cross-crate join in `scan.rs` needs the SAME construction-keyed authority one boundary further out:
// `cr::<drop>::Type`, not `Type::<construct>`. Before this, only a bare 2-segment VALUE PATH
// (`deplib::UnitGuard`) reached a correctly-keyed marker, and only by ACCIDENT — it shared code with the
// lazy-static forcing route, whose `dep_lazy_keys` derivation uses the WRITTEN PATH'S REST verbatim as the
// key. That happens to equal the type leaf for a 2-segment path, and is wrong for anything longer:
// `deplib::Guard::new(1)` derived the key `"Guard::new"` (and `"new"`), neither of which is the `"Guard"`
// the join's `{ty}::drop` lookup needs — silent. A STRUCT LITERAL never reached that code at all: syn
// walks a literal's type as a bare `syn::Path`, never as an `ExprPath`, so `visit_expr_path` never sees it.
//
// Each function below returns `(crate, type leaf)` instead of stripping the crate the way
// `local_type_leaf` does — that stripped segment IS the piece the cross-crate marker needs. The crate
// segment is NOT validated against this project's real dependency graph here (collector.rs has no
// visibility into it); a local module that merely LOOKS crate-qualified (`mymod::Guard::new()`) produces a
// marker too, exactly like the lazy-static marker beside it — self-limiting, because `scan.rs`'s join only
// ever consumes the marker when the head resolves to a real, CHAINED `CANDOR_DEPS` crate
// (`deps_idx.crates.contains(cr_real)`); anything else costs nothing.

/// A crate-qualified path's leading segment, or `None` if it names no crate at all (single-segment) or
/// is explicitly LOCAL (`crate`/`self`/`super`, already the in-crate route's territory) or is one of the
/// three roots no `CANDOR_DEPS` report is ever emitted for.
fn cross_crate_head(expanded: &str) -> Option<&str> {
    let (head, _) = expanded.split_once("::")?;
    if matches!(head, "crate" | "self" | "super" | "std" | "core" | "alloc") {
        return None;
    }
    Some(head)
}

/// The cross-crate sibling of `ctor_leaf_from_call_path`: `deplib::Guard::new(1)` (assoc-fn) and
/// `deplib::TupleGuard(1)` (tuple-struct call) both resolve here, keeping the crate the in-crate leaf
/// function discards.
pub(crate) fn cross_ctor_leaf_from_call_path(
    full: &str,
    uses: &HashMap<String, String>,
) -> Option<(String, String)> {
    let leaf = ctor_leaf_from_call_path(full, uses)?;
    let expanded = expand(full, uses);
    let cr = cross_crate_head(&expanded)?;
    Some((cr.to_string(), leaf))
}

/// The cross-crate sibling of `ctor_leaf_from_value_path`: a bare VALUE PATH rooted in another crate
/// (`deplib::UnitGuard`, `deplib::State::Open`).
pub(crate) fn cross_ctor_leaf_from_value_path(
    full: &str,
    uses: &HashMap<String, String>,
    fields: &FieldIndex,
) -> Option<(String, String)> {
    let leaf = ctor_leaf_from_value_path(full, uses, fields)?;
    let expanded = expand(full, uses);
    let cr = cross_crate_head(&expanded)?;
    Some((cr.to_string(), leaf))
}

/// The cross-crate sibling of the `ctor_leaf_of_expr` STRUCT-LITERAL arm: `deplib::Guard { n: 1 }` /
/// `deplib::E::V { .. }`. No `fields`-based fieldless test — a struct literal is a construction
/// precisely because it names fields, the same reasoning `ctor_leaf_of_expr`'s own comment gives.
pub(crate) fn cross_ctor_leaf_from_struct_path(
    full: &str,
    uses: &HashMap<String, String>,
) -> Option<(String, String)> {
    let ty_path = type_from_value_path(full, uses)?;
    let cr = cross_crate_head(&ty_path)?;
    let leaf = ty_path.rsplit("::").next().unwrap_or(&ty_path).to_string();
    Some((cr.to_string(), leaf))
}

/// LEXICAL ESCAPE GATE — the type leaves whose construction in this body does NOT die in this scope,
/// so charging their `Drop` here would FABRICATE an effect that runs in someone else's frame.
///
/// This is the load-bearing half. candor-spec SOUNDNESS R49: the analogous field-route fix went
/// regression-green and was reverted on the A/B for fabricating 14 false `Unknown`s on flate2 —
/// `Compress::new`/`Decompress::new` CONSTRUCT AND RETURN the owner, whose destructor runs in the
/// CALLER. Widening the construction route without this would multiply that over every constructor of
/// every guard type in a real crate.
///
/// A construction escapes when it is (transitively, through value positions) part of:
///   · a `return` expression, or the body's TAIL expression (the implicit return);
///   · an assignment into a FIELD / INDEX / DEREF lvalue (a stored property, a global, `self.g = …`);
///   · an assignment into, or a `let` binding of, a NAME that itself escapes (fixpoint);
///   · an argument of a method call whose RECEIVER is an escaping name (`let mut v = …; v.push(g); v`
///     — the builder shape, which is otherwise the commonest way a guard leaves by the back door).
///
/// STATED LIMIT, not a claim of completeness: a value handed to a FREE callee that RETAINS it is
/// charged, because syntax cannot see the callee's retention. That is the same over-approximation the
/// bound-local path has always made (`let g = Guard::new(); REGISTRY.lock().push(g)`), and extending
/// it rather than special-casing it is what keeps the answers equal across positions.
///
/// Keyed by type LEAF, not by expression identity: a body that constructs the SAME type both escaping
/// and locally collapses to "escapes", losing the local charge. That is strictly more precise than the
/// `returns_escapable` gate it replaces (which skipped EVERY type as soon as the fn returned an
/// aggregate) and it fails toward not-charging, which is the direction that cannot fabricate.
///
/// MUST, NOT MAY. A NAME-BASED gate that suppresses whenever the constructed name reaches *some*
/// return/tail is unsound: `let g = Guard{..}; if f { Some(g) } else { None } }` escapes only on the
/// `f` branch and drops `g` locally — and genuinely runs its `Drop` — on the other, so a single "does
/// ANY exit use the name" test silently suppressed the charge for every conditional shape (`if`/`match`,
/// an early-return guard, a `for`/`?` continuation). This function's contract is the CONSERVATIVE
/// sufficient condition the family's denylist-over-allowlist rule asks for (a narrowing must carve out
/// PROVEN-safe cases, never merely POSSIBLE ones): a leaf is suppressed only when the construction
/// escapes on *every* terminal exit reachable from it — never charging is unsound, never suppressing is
/// merely expensive, so the two implementing passes below both fail toward CHARGING, which is the
/// direction that cannot fabricate:
///   · every independent terminal exit of the function (the tail, each `return`'s operand — including
///     one nested inside a loop/if/match — and `?`'s own implicit early-return, which carries nothing)
///     is analysed SEPARATELY, seeded from nothing but that one exit, and only what every exit agrees on
///     survives the intersection (`escape_from_root`, below);
///   · within a single exit, an `if`/`else` or `match` is exactly one of its arms at runtime, so a name
///     or leaf must appear in *every* arm to count, not just one (`mark_escape`'s `If`/`Match` case).
/// An exit with NO reachable value at all (a bare `return;`, or `?`'s failure residual) is a real,
/// present counterexample — represented as an EMPTY set, not skipped — because a name that does not
/// reach it is proven to sometimes stay behind.
///
/// A known gap, not attempted here per this family's "no full path-sensitive analysis in a syntactic
/// scanner" rule: a `let` whose ESCAPE is unconditional but which follows an EARLIER, wholly unrelated
/// early return (`if unrelated { return Err(..); } let g = Guard::new(); Ok(g)`) is intersected against
/// that unrelated exit too and reads as conditional, over-charging a value that in fact always escapes.
/// Measured cost of that gap: see the corpus A/B in the commit this fixes.
pub(crate) fn escaping_ctor_leaves(
    block: &syn::Block,
    uses: &HashMap<String, String>,
    fields: &FieldIndex,
    returns: &ReturnIndex,
) -> Escapes {
    // Every binding/assignment/method-call site in the body, gathered once; each root below re-reads
    // this table (running its own copy of the fixpoint), never re-walks the tree.
    let mut sites = EscapeSites::default();
    sites.walk_block(block, true);
    // A body with NO terminal exit (a `()`-returning fn with no trailing value — `store` below) still
    // has to run the fixpoint: the field/index/deref `assigns` route is UNCONDITIONAL (not gated on any
    // root, by design — see `escape_from_root`), so `*slot = Some(G::new());` must still be seen. A
    // single call seeded from `None` runs exactly that unconditional half and nothing root-dependent,
    // which is the correct answer when there is no root to be dependent ON.
    if sites.roots.is_empty() {
        return escape_from_root(None, &sites, uses, fields, returns);
    }
    let mut roots = sites.roots.iter();
    let first = roots.next().expect("checked non-empty above");
    let mut acc = escape_from_root(*first, &sites, uses, fields, returns);
    for r in roots {
        if acc.names.is_empty() && acc.leaves.is_empty() {
            break; // the intersection can only shrink further; every remaining root would too.
        }
        let next = escape_from_root(*r, &sites, uses, fields, returns);
        acc.names.retain(|n| next.names.contains(n));
        acc.leaves.retain(|l| next.leaves.contains(l));
    }
    acc
}

/// The escape fixpoint (root use, then the `lets`/`assigns`/`method_args` transitive closure — same
/// rules as before this fix), seeded from exactly ONE of the function's terminal exits. `root` is
/// `None` for an exit proven to carry nothing (see `escaping_ctor_leaves`'s doc comment); such a call
/// simply returns the empty sets, which is what makes it veto any name/leaf during the caller's
/// intersection.
fn escape_from_root(
    root: Option<&syn::Expr>,
    sites: &EscapeSites<'_>,
    uses: &HashMap<String, String>,
    fields: &FieldIndex,
    returns: &ReturnIndex,
) -> Escapes {
    let mut names: std::collections::HashSet<String> = std::collections::HashSet::new();
    let mut leaves: std::collections::HashSet<String> = std::collections::HashSet::new();
    if let Some(e) = root {
        mark_escape(e, uses, fields, returns, &mut names, &mut leaves);
    }
    // Unconditional escape ROUTES (a closure body, a `mem::forget`/`ManuallyDrop::new` operand) — not
    // gated on `root` at all, and identical on every `escape_from_root` call, which is what lets them
    // survive `escaping_ctor_leaves`'s intersection across roots undiminished.
    for e in &sites.escapes {
        mark_escape(e, uses, fields, returns, &mut names, &mut leaves);
    }
    for _ in 0..8 {
        let before = (names.len(), leaves.len());
        for (name, init) in &sites.lets {
            if names.contains(name) {
                mark_escape(init, uses, fields, returns, &mut names, &mut leaves);
            }
        }
        for (lhs, rhs) in &sites.assigns {
            match lhs {
                // `x = Guard::new()` — only an escape if `x` itself escapes (in THIS root).
                Some(n) => {
                    if names.contains(n) {
                        mark_escape(rhs, uses, fields, returns, &mut names, &mut leaves);
                    }
                }
                // `self.g = …` / `xs[i] = …` / `*p = …` — stored somewhere this scope does not own,
                // unconditionally (not gated on this root, same as before this fix: a store is a store
                // regardless of which exit the function eventually takes).
                None => mark_escape(rhs, uses, fields, returns, &mut names, &mut leaves),
            }
        }
        for (recv, args) in &sites.method_args {
            if names.contains(recv) {
                for a in args {
                    mark_escape(a, uses, fields, returns, &mut names, &mut leaves);
                }
            }
        }
        if (names.len(), leaves.len()) == before {
            break;
        }
    }
    Escapes { leaves, names }
}

/// What `escaping_ctor_leaves` learned: the constructed type LEAVES that leave this scope, and the
/// local/param NAMES that do. The names half is what the PARAMETER-OWNED rule needs — an owned param
/// is not constructed here, so it has no construction expression to key a leaf on.
pub(crate) struct Escapes {
    pub(crate) leaves: std::collections::HashSet<String>,
    pub(crate) names: std::collections::HashSet<String>,
}

/// Does this type mention a reference or raw pointer ANYWHERE — `&T`, `Pin<&mut T>`, `*const T`,
/// `Option<&T>`? A parameter of such a type does not own what it names, so its `Drop` does not run in
/// the callee. Checked structurally rather than at the top level, which is the whole point: an
/// arbitrary-self-type `Pin<&mut Self>` hides the `&` one layer down.
fn type_borrows(ty: &syn::Type) -> bool {
    struct V(bool);
    impl<'a> syn::visit::Visit<'a> for V {
        fn visit_type_reference(&mut self, _: &'a syn::TypeReference) { self.0 = true; }
        fn visit_type_ptr(&mut self, _: &'a syn::TypePtr) { self.0 = true; }
    }
    let mut v = V(false);
    syn::visit::Visit::visit_type(&mut v, ty);
    v.0
}

/// PARAMETER-OWNED DROP — a mechanism construction-keying cannot reach BY DEFINITION. `fn take(g:
/// Guard) {}` runs `Guard::drop` inside `take` (proven by executing the destructor against call/return
/// markers), and the scan never saw the value built, so it read `take` PURE in every spelling.
///
/// Returns the type LEAVES released here: a BY-VALUE parameter (or a by-value `self`) whose type is
/// drop-relevant and whose NAME does not escape the body. A `&T`/`&mut T`/`*const T` parameter is
/// BORROWED and must never be charged — that is the fabrication this rule is one keystroke away from,
/// and it is the same distinction the `!c.method` guard makes on the construction side.
///
/// The escape half is deliberately generous: `fn finish(self) -> Vec<u8> { self.inner.finish() }`
/// mentions `self` in the tail, so nothing is charged even though `self` really does die there. That
/// over-skips a consuming method that derives its result from `self` — the direction that cannot
/// fabricate, and the one where a wrong answer is a miss rather than a false claim about someone
/// else's frame.
pub(crate) fn owned_drop_params(
    sig: &syn::Signature,
    self_ty: Option<&str>,
    uses: &HashMap<String, String>,
    escaping_names: &std::collections::HashSet<String>,
) -> Vec<String> {
    let mut out = Vec::new();
    let leaf_of = |t: String| t.rsplit("::").next().unwrap_or(&t).to_string();
    for input in &sig.inputs {
        match input {
            // `self` / `mut self` — by VALUE. `&self`/`&mut self` carry a `reference`, and consume
            // nothing.
            // `self` / `mut self` / `self: Box<Self>` — by VALUE. NOT `&self`, and NOT an ARBITRARY
            // self type that borrows: `self: Pin<&mut Self>` parses as a `Receiver` whose `reference`
            // is `None` (the `&` is inside the `Pin`, not on the binder), so the obvious
            // `r.reference.is_none()` reads every `poll_read`/`poll_flush`/`poll_shutdown` in the
            // ecosystem as consuming its receiver. Measured on tokio: seven drop types charged to
            // `net::unix::pipe::Sender::poll_flush`, whose entire body is `Poll::Ready(Ok(()))`.
            // The test is therefore "the declared self type contains no reference anywhere", which
            // keeps `Box<Self>`/`Pin<Box<Self>>` (genuinely consuming) and rejects the borrowing ones.
            syn::FnArg::Receiver(r)
                if r.reference.is_none() && !type_borrows(&r.ty) =>
            {
                if escaping_names.contains("self") {
                    continue;
                }
                if let Some(t) = self_ty {
                    out.push(leaf_of(t.to_string()));
                }
            }
            syn::FnArg::Typed(pt) => {
                if type_borrows(&pt.ty) {
                    continue;
                }
                let Some(name) = single_pat_ident(&pt.pat) else { continue };
                if escaping_names.contains(&name) {
                    continue;
                }
                if let Some(t) = type_path(&pt.ty, uses) {
                    out.push(leaf_of(t));
                }
            }
            _ => {}
        }
    }
    out
}

#[derive(Default)]
struct EscapeSites<'a> {
    /// `return e` operands and the body's tail expression — each one an independent terminal exit of
    /// the function. `None` marks an exit that provably carries nothing out of this scope (a bare
    /// `return;`, or the implicit early-return a `?` can take): a real, present counterexample, not an
    /// absence of information, so it must veto a name/leaf exactly like an exit that visibly drops it.
    roots: Vec<Option<&'a syn::Expr>>,
    /// Escape ROUTES that are not an exit of THIS function at all, so they must never be intersected
    /// against `roots`: a closure's own return value (which flows out through the closure's eventual
    /// invocation, not through this function returning) and the operand of `mem::forget`/
    /// `ManuallyDrop::new` (which flows to suppression, not to a caller). Each is unconditional — exactly
    /// like a field/deref `assigns` entry — so it is applied once per `escape_from_root` call regardless
    /// of which root that call was seeded from, which is what lets it survive the intersection.
    escapes: Vec<&'a syn::Expr>,
    /// `let NAME = init` — single-ident binders only (a destructuring binder cannot name the value).
    lets: Vec<(String, &'a syn::Expr)>,
    /// `lhs = rhs`; `Some(name)` for a plain local lvalue, `None` for a field/index/deref lvalue.
    assigns: Vec<(Option<String>, &'a syn::Expr)>,
    /// `recv.m(args…)` where the receiver is a plain local name.
    method_args: Vec<(String, Vec<&'a syn::Expr>)>,
}

impl<'a> EscapeSites<'a> {
    /// `tail` says whether this block's trailing expression is in RETURN position for the unit.
    fn walk_block(&mut self, b: &'a syn::Block, tail: bool) {
        for (i, st) in b.stmts.iter().enumerate() {
            let last = i + 1 == b.stmts.len();
            match st {
                syn::Stmt::Local(l) => {
                    if let Some(init) = &l.init {
                        if let Some(n) = single_pat_ident(&l.pat) {
                            self.lets.push((n, &init.expr));
                        }
                        self.walk_expr(&init.expr);
                        if let Some((_, d)) = &init.diverge {
                            self.walk_expr(d);
                        }
                    }
                }
                syn::Stmt::Expr(e, semi) => {
                    if tail && last && semi.is_none() {
                        self.roots.push(Some(e));
                    }
                    self.walk_expr(e);
                }
                syn::Stmt::Item(_) | syn::Stmt::Macro(_) => {}
            }
        }
    }
    fn walk_expr(&mut self, e: &'a syn::Expr) {
        match e {
            syn::Expr::Return(r) => match &r.expr {
                Some(v) => self.roots.push(Some(v)),
                // Bare `return;` — this exit is `()`-typed and provably carries nothing out.
                None => self.roots.push(None),
            },
            // `expr?`'s implicit early-return is a genuine, separate exit of the FUNCTION (not just
            // this block), and it carries only the error/`None` residual — never a name bound earlier in
            // this scope. Left unmodelled, a construction that only escapes on the SUCCESS continuation
            // (`let g = Guard::new(); fallible()?; Some(g)`) read as escaping unconditionally, because
            // the only root the old flat-union walk saw was the success-path tail. Recorded regardless of
            // this Try's own position (tail or not): every `?` is a possible early exit the moment it is
            // evaluated.
            syn::Expr::Try(_) => self.roots.push(None),
            syn::Expr::Assign(a) => match &*a.left {
                // `x = Guard::new()` — an escape only if `x` itself escapes. A MULTI-segment path is a
                // static/const (`GLOBAL = …`), which is storage this scope does not own.
                syn::Expr::Path(p) if p.qself.is_none() => {
                    self.assigns
                        .push((p.path.get_ident().map(|i| i.to_string()), &a.right));
                }
                // `self.g = …` / `xs[i] = …` / `*p = …` — stored somewhere this scope does not own.
                syn::Expr::Field(_) | syn::Expr::Index(_) | syn::Expr::Unary(_) => {
                    self.assigns.push((None, &a.right));
                }
                // `_ = Guard::new();` — the DISCARD spelling. It is not an escape at all: the value is
                // dropped at the end of the statement, in THIS scope. Recording it as one made the
                // wildcard-assign position silent in every construction spelling and REGRESSED the
                // assoc-fn spelling, which the shipped code charged. (Caught by the position matrix,
                // not by reading the code.)
                _ => {}
            },
            syn::Expr::MethodCall(m) => {
                if let syn::Expr::Path(p) = &*m.receiver {
                    if let Some(id) = p.path.get_ident() {
                        self.method_args
                            .push((id.to_string(), m.args.iter().collect()));
                    }
                }
            }
            // SUPPRESSED DESTRUCTORS. `mem::forget(g)` and `ManuallyDrop::new(g)` are the two std
            // spellings whose whole purpose is that the value's `Drop` never runs. Charging them is a
            // FABRICATION, not a conservative over-approximation, and it is one the shipped assoc-fn
            // route already made. Routed through the escape machinery rather than a special case at the
            // construction site, so the BOUND form (`let g = Guard::new(); mem::forget(g);`) and the
            // inline form (`ManuallyDrop::new(Guard(f))`) get the same answer. Matched on the LEAF
            // (`forget` / `ManuallyDrop::new`) so `std::mem::forget`, `mem::forget` and a
            // `use std::mem::forget;` bare call all land.
            syn::Expr::Call(c) => {
                if let syn::Expr::Path(p) = &*c.func {
                    let full = path_to_string(&p.path);
                    let suppresses = full == "forget"
                        || full.ends_with("mem::forget")
                        || full == "ManuallyDrop::new"
                        || full.ends_with("::ManuallyDrop::new");
                    if suppresses {
                        // An unconditional ESCAPE ROUTE, not an exit of this function — see `escapes`'s
                        // doc comment. Must NOT go through `roots`: it would then be intersected against
                        // an unrelated return/tail and could be vetoed by a path that never reaches this
                        // call at all.
                        for a in &c.args {
                            self.escapes.push(a);
                        }
                    }
                }
            }
            // A CLOSURE's own return value leaves the closure, and from there this scope cannot see
            // where it goes. Measured on sharded-slab: `Slab::get` builds its `Entry` inside
            // `shard.with_slot(key, |slot| … Some(Entry { .. }))` and returns it through TWO frames,
            // so without this the guard's `Drop` was charged to a `&self` accessor that releases
            // nothing. Closure bodies are walked lexically by the collector, so the two halves have
            // to agree about them. An unconditional ESCAPE ROUTE (see `escapes`'s doc comment), not an
            // exit of THIS function — the closure's return flows out through its own eventual
            // invocation, so it must not be intersected against this function's unrelated exits.
            syn::Expr::Closure(c) => self.escapes.push(&c.body),
            _ => {}
        }
        for_each_child_expr(e, &mut |c| self.walk_expr(c));
        // A nested block is walked ONLY to find further escape SITES (`return`, an assignment, a method
        // call on an escaping name). Its trailing expression is NOT a root: `let x = if c { Guard::new()
        // } else { … };` has an if-block whose tail is a value, but the value lands in `x`, not in the
        // caller. Treating every nested tail as a return made the ternary position silent in all five
        // construction spellings. Tail position propagates through `mark_escape`'s value-child walk
        // instead, which only ever descends from a genuine root.
        for_each_child_block(e, &mut |b| self.walk_block(b, false));
    }
}

/// Mark one escaping expression: record any construction leaf it builds, any local NAME it hands
/// out, and recurse through the value positions that carry a value outward.
fn mark_escape(
    e: &syn::Expr,
    uses: &HashMap<String, String>,
    fields: &FieldIndex,
    returns: &ReturnIndex,
    names: &mut std::collections::HashSet<String>,
    leaves: &mut std::collections::HashSet<String>,
) {
    if let Some(l) = ctor_leaf_of_expr(e, uses, fields, returns) {
        leaves.insert(l);
    }
    if let syn::Expr::Path(p) = e {
        if p.qself.is_none() {
            if let Some(id) = p.path.get_ident() {
                names.insert(id.to_string());
            }
        }
    }
    // A FIELD/INDEX of a name also hands that name's storage outward (`fn into_inner(self) -> T
    // { self.0 }` must not charge `self`'s Drop — the value moves to the caller).
    if let syn::Expr::Field(f) = e {
        if let syn::Expr::Path(p) = &*f.base {
            if let Some(id) = p.path.get_ident() {
                names.insert(id.to_string());
            }
        }
    }
    // `vec![Guard::new()]` / `Some(g)` written through a macro, in tail position. syn does not parse a
    // macro body, so without this the idiomatic collection literal reads as NOT escaping and every
    // `fn make() -> Vec<Guard> { vec![Guard::new()] }` fabricates the guard's Drop onto the factory.
    if let syn::Expr::Macro(m) = e {
        let parser = syn::punctuated::Punctuated::<syn::Expr, syn::Token![,]>::parse_terminated;
        if let Ok(exprs) = syn::parse::Parser::parse2(parser, m.mac.tokens.clone()) {
            for sub in &exprs {
                mark_escape(sub, uses, fields, returns, names, leaves);
            }
        }
        return;
    }
    // BRANCH POINT — an `if`/`else` or `match` produces exactly ONE of its arms at runtime, so a NAME
    // only escapes THROUGH this expression if it is present in EVERY arm; one arm that doesn't use it
    // is a live counterexample (the value drops locally on that arm), same as an independent exit that
    // doesn't use it (`escaping_ctor_leaves`'s doc comment) — so NAMES are intersected across arms.
    //
    // LEAVES found DIRECTLY within one arm (a construction expression textually inside that arm's own
    // subtree, with no name indirection) are different: measured on lapin's `Channel::new` —
    // `let channel_closer = if id == 0 { None } else { Some(Arc::new(ChannelCloser::new(..))) };` — the
    // `else` arm both BUILDS and ESCAPES `ChannelCloser` in the same breath, and `then` never builds it
    // at all. Intersecting made the `then` arm's vacuous silence veto a fact the `then` arm has no
    // opinion on, reintroducing a silent under-report. A leaf discovered this way is a complete,
    // self-contained fact about the one arm that has it (whenever THAT arm runs, the construction and
    // its escape happen together, regardless of any sibling arm that never runs it at all) — so leaves
    // are UNIONED across arms, never intersected. This cannot reopen the bug this file exists to fix:
    // that bug was about a NAME bound BEFORE the branch, present on every path reaching it, whose FATE
    // the branch decides — untouched here, since such a leaf is never found by a direct in-arm
    // recursion (it only reaches `leaves` later, through `escape_from_root`'s `lets` fixpoint, keyed on
    // the intersected `names`). Every OTHER value-carrying position handled by `for_each_value_child`
    // below (`Struct`/`Tuple`/`Ok(x)`/…) is unconditional once its parent executes, so union-across-
    // children stays correct there too — `If`/`Match` are the only node kinds needing a NAME/leaf split.
    match e {
        syn::Expr::If(iff) => {
            let mut then_names = std::collections::HashSet::new();
            let mut then_leaves = std::collections::HashSet::new();
            if let Some(t) = tail_expr_of(&iff.then_branch) {
                mark_escape(t, uses, fields, returns, &mut then_names, &mut then_leaves);
            }
            // No `else` means the implicit value is `()`, which can carry no NAME — an empty set, which
            // (correctly) vetoes any name the `then` arm alone found. A `()`-typed branch cannot carry a
            // real leaf either (its tail would have to be unit-typed), so this never veto-by-omission a
            // leaf in practice; leaves are unioned regardless, per this fn's doc comment above.
            let (else_names, else_leaves) = match &iff.else_branch {
                Some((_, eb)) => {
                    let mut n = std::collections::HashSet::new();
                    let mut l = std::collections::HashSet::new();
                    mark_escape(eb, uses, fields, returns, &mut n, &mut l);
                    (n, l)
                }
                None => (std::collections::HashSet::new(), std::collections::HashSet::new()),
            };
            names.extend(then_names.intersection(&else_names).cloned());
            leaves.extend(then_leaves.union(&else_leaves).cloned());
            return;
        }
        syn::Expr::Match(m) => {
            let mut arms = m.arms.iter().map(|a| {
                let mut n = std::collections::HashSet::new();
                let mut l = std::collections::HashSet::new();
                mark_escape(&a.body, uses, fields, returns, &mut n, &mut l);
                (n, l)
            });
            if let Some((first_n, first_l)) = arms.next() {
                let (names_int, leaves_union) = arms.fold((first_n, first_l), |(an, al), (n, l)| {
                    (
                        an.intersection(&n).cloned().collect(),
                        al.union(&l).cloned().collect(),
                    )
                });
                names.extend(names_int);
                leaves.extend(leaves_union);
            }
            return;
        }
        _ => {}
    }
    for_each_value_child(e, &mut |c| mark_escape(c, uses, fields, returns, names, leaves));
}

/// Value-position children of an expression — the positions through which a constructed value can
/// travel OUT of the expression it was written in (`Ok(g)`, `Owner { g }`, `(g, 1)`, `[g]`, `&g`,
/// `Box::new(g)`, `vec![g]`). Deliberately NOT the whole subtree: a `while` condition or a `for` iterator
/// carries nothing outward.
///
/// `If`/`Match` are DELIBERATELY ABSENT: unlike every case below, they are a FORK (exactly one arm
/// executes), so "found in a child" is not the right test for them — `mark_escape` intercepts both
/// before reaching this function and requires the name/leaf to be present in EVERY arm, never just one.
/// Two routes computing that one fact was exactly how the conditional-escape regression this replaces
/// opened (this fn used to answer "does any arm carry it", `mark_escape` had no opinion, and their
/// disagreement was silent); keeping only one route is the fix, not an incidental cleanup.
fn for_each_value_child<'a>(e: &'a syn::Expr, f: &mut dyn FnMut(&'a syn::Expr)) {
    match e {
        syn::Expr::Paren(p) => f(&p.expr),
        syn::Expr::Group(g) => f(&g.expr),
        syn::Expr::Try(t) => f(&t.expr),
        syn::Expr::Await(a) => f(&a.base),
        syn::Expr::Reference(r) => f(&r.expr),
        syn::Expr::Unary(u) => f(&u.expr),
        syn::Expr::Cast(c) => f(&c.expr),
        syn::Expr::Unsafe(u) => tail_expr_of(&u.block).into_iter().for_each(&mut *f),
        syn::Expr::Block(b) => tail_expr_of(&b.block).into_iter().for_each(&mut *f),
        syn::Expr::Call(c) => c.args.iter().for_each(&mut *f),
        // ARGUMENTS ONLY, never the RECEIVER. `fn hash_xof(..) -> Result<()> { let mut h =
        // Hasher::new(t)?; h.update(data)?; h.finish_xof(buf) }` (openssl) has a tail method call whose
        // receiver is a local that DIES here — reading it as an escape suppressed the guard in every
        // such shape, which is the commonest way a local guard is used at all. And a receiver that
        // really IS consumed (`g.into_inner()`) hands the value to a callee that this same rule charges
        // for its by-value parameter, so the caller still inherits the effect transitively. Both
        // readings therefore land on "charge", by different routes; escaping the receiver only ever
        // lost rows.
        syn::Expr::MethodCall(m) => m.args.iter().for_each(f),
        syn::Expr::Struct(s) => s.fields.iter().for_each(|fv| f(&fv.expr)),
        syn::Expr::Tuple(t) => t.elems.iter().for_each(&mut *f),
        syn::Expr::Array(a) => a.elems.iter().for_each(&mut *f),
        syn::Expr::Repeat(r) => f(&r.expr),
        _ => {}
    }
}

fn tail_expr_of(b: &syn::Block) -> Option<&syn::Expr> {
    match b.stmts.last() {
        Some(syn::Stmt::Expr(e, None)) => Some(e),
        _ => None,
    }
}

/// Sub-expressions to keep SCANNING for escape SITES (`return`/assignment/method-call), as opposed to
/// the narrower value-carrying positions above. This one is the whole expression subtree.
fn for_each_child_expr<'a>(e: &'a syn::Expr, f: &mut dyn FnMut(&'a syn::Expr)) {
    use syn::Expr::*;
    match e {
        Array(x) => x.elems.iter().for_each(f),
        Assign(x) => {
            f(&x.left);
            f(&x.right);
        }
        Await(x) => f(&x.base),
        Binary(x) => {
            f(&x.left);
            f(&x.right);
        }
        Break(x) => x.expr.iter().for_each(|e| f(e)),
        Call(x) => {
            f(&x.func);
            x.args.iter().for_each(&mut *f);
        }
        Cast(x) => f(&x.expr),
        Field(x) => f(&x.base),
        ForLoop(x) => f(&x.expr),
        Group(x) => f(&x.expr),
        If(x) => {
            f(&x.cond);
            if let Some((_, e)) = &x.else_branch {
                f(e);
            }
        }
        Index(x) => {
            f(&x.expr);
            f(&x.index);
        }
        Let(x) => f(&x.expr),
        Match(x) => {
            f(&x.expr);
            x.arms.iter().for_each(|a| f(&a.body));
        }
        MethodCall(x) => {
            f(&x.receiver);
            x.args.iter().for_each(&mut *f);
        }
        Paren(x) => f(&x.expr),
        Range(x) => {
            x.start.iter().for_each(|e| f(e));
            x.end.iter().for_each(|e| f(e));
        }
        Closure(x) => f(&x.body),
        Reference(x) => f(&x.expr),
        Repeat(x) => {
            f(&x.expr);
            f(&x.len);
        }
        Return(x) => x.expr.iter().for_each(|e| f(e)),
        Struct(x) => x.fields.iter().for_each(|fv| f(&fv.expr)),
        Try(x) => f(&x.expr),
        Tuple(x) => x.elems.iter().for_each(f),
        Unary(x) => f(&x.expr),
        While(x) => f(&x.cond),
        Yield(x) => x.expr.iter().for_each(|e| f(e)),
        _ => {}
    }
}

/// Blocks reachable from an expression, walked to find further escape SITES. A closure body is
/// included: `let f = || REGISTRY.push(Guard::new());` is walked lexically by the collector too, so
/// the two must agree about what it does.
fn for_each_child_block<'a>(e: &'a syn::Expr, f: &mut dyn FnMut(&'a syn::Block)) {
    use syn::Expr::*;
    match e {
        Block(x) => f(&x.block),
        Unsafe(x) => f(&x.block),
        If(x) => f(&x.then_branch),
        ForLoop(x) => f(&x.body),
        Loop(x) => f(&x.body),
        While(x) => f(&x.body),
        TryBlock(x) => f(&x.block),
        Async(x) => f(&x.block),
        // The closure's own tail was pushed as an escape ROOT above; its body is walked for further
        // sites through `for_each_child_expr`.
        Closure(_) => {}
        _ => {}
    }
}

/// ⟨peek-scope-attribution⟩ Every qual reachable by walking `rev` (callee -> callers, the inverse of the
/// normal `calls` graph) BACKWARD from `start` — i.e. `start` itself plus every ANCESTOR that could call
/// into it, directly or transitively. Cycle-safe (a `seen` set, not a depth bound): a caller cycle in real
/// code — mutual recursion, an event loop re-entering its own dispatcher — must not loop forever, and
/// candor's own call graph already tolerates cycles elsewhere (propagation runs to a fixpoint over the
/// SAME graph this inverts).
///
/// Used ONLY to widen which function NAMES a policy's scope string is tested against for a peeked
/// (excluded) finding — never to attribute the finding itself, and never to alter any inferred effect. A
/// normal (non-excluded) effect already gets this for free: propagation carries a callee's effect up
/// through every intermediate caller before the gate ever tests a scope string against a fn's OWN
/// (already-propagated) inferred set. This is the same treatment for the one edge the primary scan cannot
/// see at all — an excluded file's trait implementation — without re-running propagation or unioning the
/// excluded file into this scan's own universe.
pub(crate) fn reaching_ancestors<'a>(
    start: impl IntoIterator<Item = &'a str>,
    rev: &HashMap<&'a str, Vec<&'a str>>,
) -> std::collections::BTreeSet<String> {
    let mut seen: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
    let mut stack: Vec<&str> = Vec::new();
    for s in start {
        if seen.insert(s.to_string()) {
            stack.push(s);
        }
    }
    while let Some(cur) = stack.pop() {
        if let Some(callers) = rev.get(cur) {
            for c in callers {
                if seen.insert((*c).to_string()) {
                    stack.push(c);
                }
            }
        }
    }
    seen
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

/// A PROVIDED `io::Write`/`fmt::Write` method: one whose std body is driven by the required `write`
/// (io) / `write_str` (fmt) method. Calling one on a concrete local `impl Write` reaches that required
/// method's (possibly effectful) body — but the driving happens INSIDE std, invisible to the scan, so
/// the call read silent-pure. Charged to `Type::write`/`Type::write_str` like the iterator-`next` /
/// Display-`fmt` coercions (`charge_write_provided`). The EAGER subset that actually performs the write:
/// `write`/`write_str` themselves are ABSENT — they resolve as ordinary method calls to the local def.
pub(crate) fn is_write_provided(leaf: &str) -> bool {
    matches!(
        leaf,
        "write_all" | "write_fmt" | "write_all_vectored" | "write_char"
    )
}

/// A PROVIDED `io::Read` method driven by the required `read` (`read_to_end`/`read_to_string`/
/// `read_exact`). Its std body loops on `self.read`, so on a concrete local `impl Read` whose `read` is
/// effectful the call read silent-pure. Charged to `Type::read`. The LAZY adaptors (`bytes`/`chars`/
/// `take`/`by_ref`/`chain`) are ABSENT — they return a wrapper and do not drive `read` at the call site
/// (charging them would over-approximate a never-driven chain, mirroring the iterator-adaptor exclusion).
pub(crate) fn is_read_provided(leaf: &str) -> bool {
    matches!(leaf, "read_to_end" | "read_to_string" | "read_exact")
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
    /// a named or inline-captured hole (`{name}`, `{x:?}`) — references a `name = expr` named arg or,
    /// failing that, the same-named binding in scope (the inline capture). Carries the NAME so the
    /// formatting coercion can resolve it; it consumes no POSITIONAL slot either way.
    Named(String),
}

/// The VALUE of a `name = expr` format named-arg, when `e` is exactly that assignment for `name`
/// (`format!("{v}", v = x)` → `x`). `None` for a positional arg or a different name. Used so a NAMED
/// format hole charges the same stringification coercion a positional one does.
pub(crate) fn named_arg_value<'e>(e: &'e syn::Expr, name: &str) -> Option<&'e syn::Expr> {
    let syn::Expr::Assign(a) = e else { return None };
    let syn::Expr::Path(p) = &*a.left else { return None };
    (p.path.get_ident()? == name).then_some(&*a.right)
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
                    FmtArg::Named(name_part.to_string())
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
