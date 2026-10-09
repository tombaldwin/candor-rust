//! Syntax-level helpers over `syn`: path/type reading, cfg evaluation, literal and
//! format-macro dissection. No scanner state — pure functions over the AST.

use crate::*;

pub(crate) fn path_to_string(p: &syn::Path) -> String {
    p.segments.iter().map(|s| s.ident.to_string()).collect::<Vec<_>>().join("::")
}

/// VEIN A — `path_to_string` KEEPING a leading `::`. `::smol::net::resolve` names the EXTERN crate
/// `smol` even where the writing module declares its own `mod smol` (redis-1.7.0's `aio` does exactly
/// that, behind a feature), and with the colon dropped the anchoring rule read it as the local module —
/// measured, 37 redis rows lost a real `Net`. `expand` strips the colon itself and answers such a path
/// with no vein-A anchoring and no glob reading, which is precisely today's answer for it. Used only at
/// the sites whose string goes straight into `expand`.
pub(crate) fn path_to_string_lc(p: &syn::Path) -> String {
    let s = path_to_string(p);
    if p.leading_colon.is_some() { format!("::{s}") } else { s }
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
/// SOUNDNESS R562 — `collect_dyn_trait_leaves` as a VALUE, for the two positions whose erasure fact is
/// keyed per-DECLARATION rather than per-body (a struct field, a fn return). Deterministic order, so a
/// cached index and a fresh one hash the same.
pub(crate) fn dyn_trait_leaves_of(ty: &syn::Type) -> Vec<String> {
    let mut set = std::collections::HashSet::new();
    collect_dyn_trait_leaves(ty, &mut set);
    let mut v: Vec<String> = set.into_iter().collect();
    v.sort();
    v
}

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

/// SOUNDNESS R571 — the COMPLEMENT of `dyn_trait_leaves_of` within `trait_leaves`: the trait leaves
/// this ONE declaration spells in a position the CALLER monomorphizes (`impl T`, `T` under `T: Bound`),
/// as opposed to a `dyn` position the callee erases.
///
/// Derived from the two existing authorities rather than by a third walk over `syn::Type`, because that
/// is precisely the drift §F1 Q3 names: `trait_leaves` and `collect_dyn_trait_leaves` already disagree
/// deliberately about wrappers (`Vec<Box<dyn T>>` is a `dyn` leaf and NOT a `trait_leaves` answer), and
/// a hand-written third arm set would have to re-decide every one of those cases. Set difference cannot.
///
/// Read as a DENYLIST and nothing else: a leaf here SUBTRACTS a receiver from the imported-trait CHA
/// (`collector.rs`, R4's erasure carve-out). A declaration shape this misses therefore keeps whatever
/// the CHA already did — the over-charge direction — and never turns one into a silent under-report.
pub(crate) fn mono_trait_leaves(
    ty: &syn::Type,
    generic_bounds: &HashMap<String, Vec<String>>,
) -> Vec<String> {
    let erased = dyn_trait_leaves_of(ty);
    trait_leaves(ty, generic_bounds).into_iter().filter(|l| !erased.contains(l)).collect()
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

/// SOUNDNESS R549 — THE NAMES THIS SCOPE BINDS AS TRAITS, bare leaves included.
///
/// `quals_from_bounds` deliberately SKIPS a bare-leaf bound (`T: Buffy`) because it carries no crate
/// identity and `expand` + the file's `use` map owns the qualification. That is right for building a
/// QUALIFIER map and wrong for answering a different question: *is this name a trait at all?* Without an
/// answer, `Buffy::chunk` in value position is indistinguishable from `SomeStruct::new`, and R549's
/// fn-ref dispatch cannot be recorded without also minting keys for inherent associated functions.
///
/// So this collects the LAST segment of every trait bound, qualified or not, and is consulted ONLY as a
/// predicate. It is additive: no existing resolver reads it, so it cannot change any effect already
/// inferred. Caller merges the enclosing `impl<T: Trait>` block's generics with the method's own —
/// `sig_trait_quals` sees only the signature, which is why `impl<T: Buf> BufList<T>` was invisible.
pub(crate) fn bound_trait_leaves(generics: &syn::Generics, out: &mut std::collections::HashSet<String>) {
    fn eat(
        bounds: &syn::punctuated::Punctuated<syn::TypeParamBound, syn::Token![+]>,
        out: &mut std::collections::HashSet<String>,
    ) {
        for b in bounds {
            if let syn::TypeParamBound::Trait(t) = b {
                if let Some(last) = t.path.segments.last() {
                    out.insert(last.ident.to_string());
                }
            }
        }
    }
    for p in &generics.params {
        if let syn::GenericParam::Type(tp) = p {
            eat(&tp.bounds, out);
        }
    }
    if let Some(w) = &generics.where_clause {
        for pred in &w.predicates {
            if let syn::WherePredicate::Type(pt) = pred {
                eat(&pt.bounds, out);
            }
        }
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
        merge_trait_qual(out, leaf, full);
    }
}

/// The TOMBSTONE RULE for a leaf-keyed qualification map, in ONE place. Two different qualified paths
/// for one leaf is not a choice to be made — it is a refusal, recorded as an empty value that every
/// consumer treats as absent and falls back from. Extracted because SOUNDNESS R586 adds a second
/// producer (the crate-wide written-qual index below) that must merge under exactly the same rule as
/// `quals_from_bounds`; two copies of a "never guess which crate" rule is the shape that produces a
/// guess.
pub(crate) fn merge_trait_qual(out: &mut HashMap<String, String>, leaf: String, full: String) {
    match out.get(&leaf) {
        Some(prev) if *prev != full => { out.insert(leaf, String::new()); }
        Some(_) => {}
        None => { out.insert(leaf, full); }
    }
}

/// SOUNDNESS R577 — EVERY CRATE-QUALIFIED TRAIT BOUND THIS FILE WRITES, wherever it writes it.
///
/// `sig_trait_quals` and `visit_local` record the QUALIFICATION a declaration was written with;
/// `trait_fields`, `rets` and the closure-parameter binder record only the BARE LEAF, because every
/// index downstream of them is leaf-keyed. The consumer then expands that leaf through the CONSUMING
/// FILE's `use` map — so the same field resolved in a scope that imported the trait and vanished in one
/// that did not. One question, four sites, three answering with less information than the fourth.
///
/// This is the shared answer for the three that cannot carry it themselves: a struct field and a fn
/// return are declared somewhere else entirely, so the fact has to survive to a crate-wide index.
///
/// NO ARM SET, DELIBERATELY. It is a `syn::visit` over every `syn::Type` in the file, so it cannot drift
/// from the declaration sites the way a hand-written field/return walker would (§F1 Q3 is exactly how
/// this defect was reached). The DERIVATION is `collect_trait_quals` — the same one `sig_trait_quals`
/// and `visit_local` use — so what counts as a qualification is decided in one place.
///
/// ONLY AN EXPLICITLY WRITTEN MULTI-SEGMENT PATH IS RECORDED (`quals_from_bounds` drops a bare leaf and
/// every `crate`/`self`/`super` spelling). A bare leaf is left to the consuming file's `use` map exactly
/// as before: this index answers only where that map has no answer at all. Conflicting spellings for one
/// leaf TOMBSTONE, so a crate binding `Handler` to two dependencies resolves it to neither.
pub(crate) fn collect_written_trait_quals(
    items: &[syn::Item],
    include_tests: bool,
    out: &mut HashMap<String, String>,
) {
    struct V<'a> {
        out: &'a mut HashMap<String, String>,
        include_tests: bool,
    }
    impl<'ast> syn::visit::Visit<'ast> for V<'_> {
        fn visit_item_mod(&mut self, m: &'ast syn::ItemMod) {
            if self.include_tests || !is_cfg_test(&m.attrs) {
                syn::visit::visit_item_mod(self, m);
            }
        }
        fn visit_type(&mut self, t: &'ast syn::Type) {
            collect_trait_quals(t, self.out);
            syn::visit::visit_type(self, t);
        }
    }
    let mut v = V { out, include_tests };
    for it in items {
        syn::visit::Visit::visit_item(&mut v, it);
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

/// SOUNDNESS R582 — THE ELEMENT EXPRESSIONS OF A COLLECTION LITERAL: `[a, b]`, `[a; n]`, `vec![a, b]`,
/// `vec![a; n]`. `None` for anything else.
///
/// The two element resolvers (`resolve_elem_type` and `resolve_elem_trait_leaves`) between them carry
/// thirteen arms and NEITHER had one for a collection written out in the source, so
/// `let v = vec![s]; for x in v { x.emit() }` lost the element entirely — no row, no `Unknown`, no
/// `invisible`, over a body that reads a file — while the ANNOTATED spelling of the same statement
/// charged. The defect is not in the `let` binder (it already asks both resolvers); it is that the
/// collection had no answer to give.
///
/// ONE helper for both resolvers, deliberately: they already disagree about arms (§F1 Q3, and R575 is
/// this codebase's open instance of exactly that), and the question "which expressions are the elements
/// of this literal" has one answer.
pub(crate) fn collection_literal_elems(expr: &syn::Expr) -> Option<Vec<syn::Expr>> {
    match expr {
        syn::Expr::Array(a) => Some(a.elems.iter().cloned().collect()),
        // `[x; n]` — every element IS `x`, so one expression answers for the whole collection.
        syn::Expr::Repeat(r) => Some(vec![(*r.expr).clone()]),
        syn::Expr::Macro(m) => {
            if m.mac.path.segments.last()?.ident != "vec" {
                return None;
            }
            // `respan_call_site` is REQUIRED, not hygiene — see `macro_reading`: these tokens were
            // parsed on another thread and syn's span JOIN aborts the parser without it.
            let tokens = crate::model::respan_call_site(m.mac.tokens.clone());
            let comma = syn::punctuated::Punctuated::<syn::Expr, syn::Token![,]>::parse_terminated;
            if let Ok(exprs) = syn::parse::Parser::parse2(comma, tokens.clone()) {
                return Some(exprs.into_iter().collect());
            }
            // `vec![x; n]`, which the comma parser cannot read. Same rule as `Expr::Repeat`.
            let semi = syn::punctuated::Punctuated::<syn::Expr, syn::Token![;]>::parse_terminated;
            let exprs = syn::parse::Parser::parse2(semi, tokens).ok()?;
            exprs.into_iter().next().map(|e| vec![e])
        }
        _ => None,
    }
}

/// The trait leaves of a type-param-bound list (`T: Store + Send` -> ["Store", "Send"]). Marker
/// bounds need no filtering here: a leaf only ever matters if it later matches a local trait or a
/// local impl, and nobody locally declares `trait Send`.
///
/// SOUNDNESS R483 — EXCEPT A `?Trait` RELAXATION, WHICH IS NOT A BOUND AND WAS BEING RECORDED AS ONE.
/// `T: ?Sized` REMOVES the implicit `Sized` bound; it guarantees nothing about `T` and no method is
/// reachable through it. The sentence above ("a leaf only matters if it matches a local trait") was
/// true when written and stopped being true the moment a leaf could DISPLACE something: a field's
/// dispatch leaves suppress its concrete `fields` entry, so `struct Mutex<T: ?Sized>(PhantomData<..>,
/// parking_lot::Mutex<T>)` reported `["Sized"]` — `trait_leaves` peels `Mutex` and finds the param —
/// and the entry naming the REAL `parking_lot::Mutex` went with it. Measured over 1,608 crates: of the
/// 19 rows the first A/B REMOVED, THIRTEEN were this — tokio's
/// `loom::std::parking_lot::Mutex::{lock,try_lock,get_mut}` (3), async-lock 2.8.0/3.4.2's
/// `MutexGuard::drop`/`MutexGuardArc::drop`/`Lock::poll`/`poll_with_strategy` family (8), and
/// regex-lite/fancy-regex's `ReplacerRef::replace_append` (2) — each losing a disclosed `invisible` or
/// `Unknown` and going ABSENT, the cardinal-sin direction. (The remaining 6 were diesel's and have a
/// different, benign cause; the CHANGELOG entry traces them.) The MODIFIER is the discriminator, not
/// the name: `T: Sized` is a real (if useless) bound, `T: ?Sized` is its removal.
pub(crate) fn bound_leaves(bounds: &syn::punctuated::Punctuated<syn::TypeParamBound, syn::Token![+]>) -> Vec<String> {
    bounds
        .iter()
        .filter_map(|b| match b {
            syn::TypeParamBound::Trait(t)
                if !matches!(t.modifier, syn::TraitBoundModifier::Maybe(_)) =>
            {
                t.path.segments.last().map(|s| s.ident.to_string())
            }
            _ => None,
        })
        .collect()
}

/// SOUNDNESS R535 — is this cast target an UNSIZING cast, i.e. does it name a trait object or an
/// opaque `impl Trait` (bare, behind a reference, or inside one wrapper)?
///
/// MEASURED, and the measurement is why this exists rather than calling `trait_leaves` directly on the
/// cast type. `trait_leaves` answers for a BARE IDENT out of `generic_bounds`, and a where-clause can
/// bind a CONCRETE type: moxcms-0.8.1 declares `where u32: AsPrimitive<T>`, so
/// `((src * max).round() as u32).min(max as u32)` resolved `min` to `num_traits#AsPrimitive::min` — a
/// trait that has no `min`. The effect set stayed `[]`, but the row gained a `dispatchesOn` edge to a
/// method that does not exist and an `invisible: ["num_traits"]` beside it. A numeric cast does not
/// change what a receiver dispatches to; only an unsizing cast does, and that is the whole of the case
/// R535 was written for (`(subscriber as &dyn Subscriber).is::<..>()`, which is real and is now
/// disclosed). Narrowing a widening of my OWN is not the denylist rule — there is no sound
/// over-approximation here to preserve, there is a fabricated dispatch target to not introduce.
pub(crate) fn is_unsizing_cast_target(ty: &syn::Type) -> bool {
    match ty {
        syn::Type::TraitObject(_) | syn::Type::ImplTrait(_) => true,
        syn::Type::Reference(r) => is_unsizing_cast_target(&r.elem),
        syn::Type::Paren(p) => is_unsizing_cast_target(&p.elem),
        syn::Type::Group(g) => is_unsizing_cast_target(&g.elem),
        // `Box::new(x) as Box<dyn T>` / `Arc<dyn T>` — one wrapper level, the same wrapper question
        // `trait_leaves` answers below; this only decides WHETHER to ask it.
        syn::Type::Path(p) => p
            .path
            .segments
            .last()
            .and_then(|seg| match &seg.arguments {
                syn::PathArguments::AngleBracketed(a) => Some(a),
                _ => None,
            })
            .is_some_and(|a| {
                a.args.iter().any(|g| matches!(g, syn::GenericArgument::Type(t) if is_unsizing_cast_target(t)))
            }),
        _ => false,
    }
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
            //
            // SOUNDNESS R380 — `Option` BELONGS HERE AND WAS MISSING, which is why an
            // `Option<Box<dyn Doer>>` field had no entry in `trait_fields` at all and every chain
            // through it resolved to nothing. The binder spelling `if let Some(h) = &self.opt { h.go() }`
            // charges correctly, but it does so through the Option-PAYLOAD binder route, not this index —
            // so it looked like a control proving the field was recorded when it proves something else.
            // A row's stated mechanism is a hypothesis too.
            //
            // Peeling `Option` is sound in the strict sense: the field can YIELD a `dyn Doer` and nothing
            // else, and `self.opt.go()` does not compile, so the only way to reach the method is through
            // an unwrap — which is exactly the chain the caller writes. `Result` is deliberately NOT added
            // with it: its `Err` arm is a different type, and R347 already priced the shape where a peel
            // hands a closure an error value it types as the payload (`async-process`'s
            // `unwrap_or_else(|x| x.into_inner())`, which fabricated `Exec`). One wrapper, measured.
            let Some(seg) = p.path.segments.last() else { return Vec::new() };
            // SOUNDNESS R401 — `Pin` and `ManuallyDrop` join the list, and the criterion is the one
            // `elem_trait_leaves` states below rather than a longer list of names: a wrapper belongs in
            // THIS resolver only when the receiver genuinely IS the inner type, because peeling one that
            // is not fabricates a receiver. Both qualify by Deref and nothing weaker —
            // `impl<P: Deref> Deref for Pin<P> { type Target = P::Target; }` and
            // `impl<T> Deref for ManuallyDrop<T> { type Target = T; }` — so `p.go()` on a
            // `Pin<Box<dyn Doer>>` COMPILES and dispatches to the trait object. `Pin<Box<dyn _>>` is the
            // dominant async spelling, and it read silent-pure.
            //
            // MEASURED ABSENT on HEAD with `Box`/`Arc`/`Vec[i]` charging in the same scan as controls.
            //
            // `Weak` and a `HashMap` VALUE are the other two R401 names and are deliberately NOT here:
            // neither Derefs, so `w.go()` does not compile and the method is reached through a named
            // accessor (`upgrade()`, `get()`). They belong to the ELEMENT route, which is where a
            // payload reached through an accessor is already modelled — putting them here would be the
            // `OnceLock` mistake that comment warns about, one row later.
            let wrapper = matches!(seg.ident.to_string().as_str(), "Box" | "Rc" | "Arc" | "RefCell" | "Mutex" | "RwLock" | "Cell" | "Option" | "Pin" | "ManuallyDrop");
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

/// SOUNDNESS R271 — the ONE authority for "which sub-expressions can this expression evaluate TO,
/// as a value". Peels the expression wrappers that pass a value through UNCHANGED and returns the
/// operand expressions at the leaves: a `match`/`if` yields one per arm/branch (both of them —
/// a value read from only the THEN branch is a dataflow merge judged by adjacency, §F1 question 1),
/// a block or `unsafe {}` yields its tail, `*`/`as`/`&`/`?`/`.await`/parens yield their operand,
/// and an INDEXED ARRAY LITERAL (`[cb][0]`) yields its elements.
///
/// Two consumers ask this question and they used to answer it separately, which is how it drifted:
/// `expr_is_fn_typed` peeled `&`/paren/group/`?`/`.await` and an `if`'s THEN branch, while the
/// named-fn-by-value edge for an invoking adapter peeled NOTHING and matched a bare `syn::Expr::Path`.
/// So `v.retain(match 0 { _ => local_eff })`, over a fn the scanner can read and KNOWS writes a file,
/// was ABSENT from `functions[]` — not a lost hedge, a lost KNOWN effect. Measured over a generated
/// 270-cell matrix: 8 of 10 wrappers x 7 access paths x {invoking adapter, bound-then-called} silent,
/// on `v0.35.0` and on HEAD alike.
///
/// DIRECTION IT FAILS IN. Toward SILENCE, unchanged, for every wrapper NOT listed: a `loop`/`while`
/// whose value arrives via `break`, a subscript of a real collection (`xs[i]` — the sound answer needs
/// element typing, and peeling to the BASE path would let an unrelated same-named leaf be resolved as
/// a callee, which is the leaf-collision fabrication class), and any wrapper reached through a call
/// this fn does not model. It never invents an operand: every expression returned is a real
/// sub-expression of the input, so a consumer that resolves nothing for it is exactly as silent as
/// before. Each sub-expression is visited at most once, so the output is linear in the AST.
pub(crate) fn callable_operands<'a>(expr: &'a syn::Expr, out: &mut Vec<&'a syn::Expr>) {
    match expr {
        syn::Expr::Paren(e) => callable_operands(&e.expr, out),
        syn::Expr::Group(e) => callable_operands(&e.expr, out),
        syn::Expr::Reference(e) => callable_operands(&e.expr, out),
        syn::Expr::Try(e) => callable_operands(&e.expr, out),
        syn::Expr::Await(e) => callable_operands(&e.base, out),
        syn::Expr::Cast(e) => callable_operands(&e.expr, out),
        // The CAST'S OWN TARGET TYPE is deliberately NOT consulted. `is_callable_type(&e.ty)` would
        // make `local_pure as Cb` opaque, degrading a provably pure, locally visible callee to
        // `Unknown` — the over-charge control this change must not worsen. `transmute::<_, F>` reads
        // its turbofish because there the OPERAND is a raw pointer and no other information exists.
        syn::Expr::Unary(u) if matches!(u.op, syn::UnOp::Deref(_)) => callable_operands(&u.expr, out),
        syn::Expr::Block(b) => match block_tail_expr(&b.block) {
            Some(t) => callable_operands(t, out),
            None => out.push(expr),
        },
        syn::Expr::Unsafe(u) => match block_tail_expr(&u.block) {
            Some(t) => callable_operands(t, out),
            None => out.push(expr),
        },
        syn::Expr::If(e) => {
            if let Some(t) = block_tail_expr(&e.then_branch) {
                callable_operands(t, out);
            }
            if let Some((_, els)) = &e.else_branch {
                callable_operands(els, out);
            }
        }
        syn::Expr::Match(m) => {
            for arm in &m.arms {
                callable_operands(&arm.body, out);
            }
        }
        // `[cb][0]` — an ARRAY LITERAL subscripted. Only the literal: see the direction note above.
        syn::Expr::Index(i) => match &*i.expr {
            syn::Expr::Array(a) => {
                for e in &a.elems {
                    callable_operands(e, out);
                }
            }
            _ => out.push(expr),
        },
        _ => out.push(expr),
    }
}

/// [`callable_operands`] as an owned vector — the form both consumers use.
pub(crate) fn callable_operand_list(expr: &syn::Expr) -> Vec<&syn::Expr> {
    let mut v = Vec::new();
    callable_operands(expr, &mut v);
    v
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
/// SOUNDNESS R899 — the first written segment of a type's path under references (`&Backend`,
/// `&mut m::T` -> `Backend`, `m`): the name a `#[cfg]`-duplicated `use` binds.
pub(crate) fn type_written_head(ty: &syn::Type) -> Option<String> {
    let mut t = ty;
    while let syn::Type::Reference(r) = t {
        t = &r.elem;
    }
    match t {
        syn::Type::Path(tp) if tp.qself.is_none() => tp.path.segments.first().map(|s| s.ident.to_string()),
        _ => None,
    }
}

pub(crate) fn type_path(ty: &syn::Type, uses: &HashMap<String, String>) -> Option<String> {
    type_path_b(ty, uses).map(|(t, _)| t)
}

/// SOUNDNESS R718, THE OWNED-WITH-A-BORROW HALF — `type_path` AND whether the leaf it answers was reached THROUGH a reference.
/// `type_path` is the ONE authority for which layers are peeled on the way to a field's type; the
/// ownership question has to be asked along that SAME walk, because it is a question about the leaf
/// this function returns and not about the declared type as a whole. `type_borrows(&f.ty)` asked the
/// whole type — "is there a `&` ANYWHERE?" — and so `t: TempFile<&'a Path>` read as borrowed although
/// the leaf, `TempFile`, is OWNED and its `Drop` runs in the constructing frame (EXECUTED: 1 drop, the
/// file really removed). A `&` inside a GENERIC ARGUMENT of the leaf is not on this walk and does not
/// make the leaf borrowed. Returning the flag from the walk itself rather than re-implementing the peel
/// list beside it is §G: two copies of one peel drift, and here the drift is a silence.
pub(crate) fn type_path_b(ty: &syn::Type, uses: &HashMap<String, String>) -> Option<(String, bool)> {
    match ty {
        syn::Type::Reference(r) => type_path_b(&r.elem, uses).map(|(t, _)| (t, true)),
        syn::Type::Paren(p) => type_path_b(&p.elem, uses),
        syn::Type::Group(g) => type_path_b(&g.elem, uses),
        syn::Type::Path(p) => {
            // A transparent OWNED smart-pointer wrapper (`Box<T>`/`Arc<T>`/`Rc<T>`) auto-derefs:
            // `wrapper.method()` dispatches to `T`'s method. Peel to `T` so the method resolves against
            // the POINTEE — without this, a `.method()` on an `Arc<Inner>` field/local/param resolved to
            // "Arc" (no impl in crate) and the call was SILENTLY DROPPED, not even Unknown (a §4
            // under-report). Arc/Rc/Box receivers are ubiquitous in real Rust (found by corpus-testing
            // duct + crates: it dropped duct's whole public-API Exec). Mirrors elem_type's wrapper-peel.
            // Only these three (owned, Deref-to-T); Mutex/RefCell need an explicit .lock()/.borrow().
            if let Some(seg) = p.path.segments.last() {
                // SOUNDNESS R980 — and `Pin<P>`, which derefs to `P::Target`: a `p: Pin<&mut Req>`
                // parameter's `p.poll(cx)` is `Req::poll` (fixture `rustagent-rel/p1` `e_pin_param`).
                // SOUNDNESS R1036 — and a lock/borrow GUARD, which derefs to what it guards: `fn get() ->
                // MutexGuard<'static, Runtime>` then `get().write(..)` is `Runtime::write` (snapbox's
                // `Data::write_to`, a dropped edge in every arm). Only by its RESOLVED std path: a crate's own
                // `Ref<T>` is its own type.
                let guard = matches!(
                    seg.ident.to_string().as_str(),
                    "MutexGuard" | "RwLockReadGuard" | "RwLockWriteGuard" | "Ref" | "RefMut" | "MappedMutexGuard"
                        | "MappedRwLockReadGuard" | "MappedRwLockWriteGuard" | "ReentrantMutexGuard"
                ) && {
                    let full = expand(&path_to_string_lc(&p.path), uses);
                    // std's own only: a DEPENDENCY's guard (`parking_lot::MutexGuard`) derefs through the
                    // dependency's code, and peeling it hid that boundary (tokio's loom shim lost
                    // `invisible: [parking_lot]` on its `Deref` impls in the A/B).
                    matches!(full.split("::").next(), Some("std" | "core"))
                };
                if guard || matches!(seg.ident.to_string().as_str(), "Box" | "Arc" | "Rc" | "Pin") {
                    if let syn::PathArguments::AngleBracketed(args) = &seg.arguments {
                        if let Some(inner) = args.args.iter().find_map(|a| match a {
                            syn::GenericArgument::Type(t) => Some(t),
                            _ => None,
                        }) {
                            return type_path_b(inner, uses);
                        }
                    }
                }
            }
            let written = path_to_string_lc(&p.path);
            let t = expand(&written, uses);
            // SOUNDNESS R978 — the same peel, through a `type A = Arc<T>;` alias. See `DEREF_ALIAS_SUF`.
            if is_deref_wrapper_leaf(&t)
                && p.path.segments.last().is_some_and(|s| matches!(s.arguments, syn::PathArguments::None))
            {
                if let Some(pointee) = deref_alias_target(&written, uses) {
                    if reach_debug() {
                        eprintln!("R978PEEL {written} -> {pointee}"); // §E1 reach, `CANDOR_R977_DEBUG=1`
                    }
                    return Some((pointee, false));
                }
            }
            Some((t, false))
        }
        _ => None,
    }
}

/// SOUNDNESS R978 — the suffix of the TYPE-ONLY key under which a `type A = Box<T>|Arc<T>|Rc<T>;` alias
/// records its POINTEE `T`, beside the ordinary `A` -> wrapper entry. `\u{4}` cannot appear in a Rust path,
/// so the key collides with no name; it rides the module-alias map (`seed_mod_aliases`,
/// `alias_expand_decls`) under exactly the spellings the alias itself is seeded under, and only
/// `type_path` and `alias_expand_decls` ever read it — never a call path, so `A::new(..)` stays `Arc::new`.
pub(crate) const DEREF_ALIAS_SUF: &str = "\u{4}deref";

/// The single type argument of a written `Box<T>` / `Arc<T>` / `Rc<T>` — the three wrappers `type_path_b`
/// peels (owned, `Deref<Target = T>`). `None` for anything else.
pub(crate) fn deref_wrapper_arg(p: &syn::Path) -> Option<&syn::Type> {
    let seg = p.segments.last()?;
    if !matches!(seg.ident.to_string().as_str(), "Box" | "Arc" | "Rc") {
        return None;
    }
    let syn::PathArguments::AngleBracketed(args) = &seg.arguments else { return None };
    args.args.iter().find_map(|a| match a {
        syn::GenericArgument::Type(t) => Some(t),
        _ => None,
    })
}

/// Does an EXPANDED type path name one of `deref_wrapper_arg`'s wrappers (by leaf, the same test the peel
/// applies to a written one)?
pub(crate) fn is_deref_wrapper_leaf(t: &str) -> bool {
    matches!(t.rsplit("::").next(), Some("Box" | "Arc" | "Rc"))
}

/// R978 — the pointee recorded for the wrapper alias spelled `written`, under the keys `seed_mod_aliases`
/// binds: the spelling itself (bare in the declaring module, relative from an ancestor), its `crate::`
/// form, and — for a name brought in by `use` — the import's target in both forms. A multi-arm (`#[cfg]`
/// twinned) value is refused, as `alias_expand_decls` refuses one: a decl type has no call site to
/// adjudicate at.
pub(crate) fn deref_alias_target(written: &str, uses: &HashMap<String, String>) -> Option<String> {
    let mut bases: Vec<String> = vec![written.to_string()];
    if let Some(v) = uses.get(written) {
        bases.push(v.clone());
    }
    let mut keys: Vec<String> = Vec::new();
    for b in &bases {
        keys.push(format!("{b}{DEREF_ALIAS_SUF}"));
        if !b.starts_with("crate::") {
            keys.push(format!("crate::{b}{DEREF_ALIAS_SUF}"));
        }
    }
    keys.iter()
        .find_map(|k| uses.get(k))
        .filter(|v| !v.contains(crate::decls::ALIAS_ALT_SEP))
        .cloned()
}

/// SOUNDNESS R454 — the SEQUENCE containers whose FIRST type argument is the element, and the MAP
/// containers whose SECOND is. ONE authority each, because `elem_type` (concrete elements) and
/// `elem_trait_leaves` (trait-object elements) each held their own copy and **the map list existed in
/// only one of them**: `HashMap<String, Box<dyn Doer>>` + `.values()` charged while the byte-identical
/// statement over `HashMap<String, G>` read silent-pure, and so did `m[k].run()` and
/// `m.get(k).unwrap().run()`. That is R347's §G shape one level up — R347 unified the ADAPTER list the
/// two resolvers peel with, and left the CONTAINER-SHAPE list they dispatch on as two copies.
///
/// WHAT IS **NOT** UNIFIED, AND DELIBERATELY. The two functions' remaining arms differ on purpose and
/// collapsing them would be the file-deletion mistake: `elem_type` answers for `IoResult` and
/// `elem_trait_leaves` does not; `elem_trait_leaves` peels the INTERIOR-MUTABILITY cells
/// (`Mutex`/`RwLock`/`RefCell`/`OnceLock`/`Weak`) and `elem_type` must not, which is R347's backed-out
/// half and is recorded at the arm itself. Only the two lists that should be IDENTICAL are shared.
pub(crate) fn is_sequence_container(name: &str) -> bool {
    matches!(
        name,
        "Vec" | "VecDeque" | "HashSet" | "BTreeSet" | "ContiguousArray" | "BinaryHeap" | "LinkedList"
    )
}

/// See `is_sequence_container`. The element is the SECOND type argument — a map's VALUE.
pub(crate) fn is_map_container(name: &str) -> bool {
    matches!(name, "HashMap" | "BTreeMap" | "IndexMap" | "DashMap" | "FxHashMap" | "AHashMap")
}

/// The ELEMENT type path of a COLLECTION `syn::Type`: `Vec<T>` / `&[T]` / `[T; N]` / `HashSet<T>` /
/// `BTreeSet<T>` / `VecDeque<T>` / `Box<[T]>` (and `Arc`/`Rc`-wrapped slices) -> the expanded type path
/// of `T` (via `uses`, like `type_path`). `None` for a non-collection type. Used to type a loop /
/// subscript / iterator-closure binding over a collection so the element's method calls classify —
/// without it, a very common Rust shape (`for c in xs { c.send() }`, `xs[0].send()`) dropped its
/// receiver to pure (a §4 under-report). Peels references/parens/groups around the collection.
pub(crate) fn elem_type(ty: &syn::Type, uses: &HashMap<String, String>) -> Option<String> {
    elem_type_b(ty, uses).map(|(t, _)| t)
}

/// SOUNDNESS R718, THE OWNED-WITH-A-BORROW HALF — `elem_type` AND whether the ELEMENT it answers is reached through a reference,
/// either around the collection (`&mut Vec<G>`, `&[G]`) or around the element (`Vec<&G>`). The
/// question `type_path_b` asks for the field's own leaf, asked along THIS function's walk: a map's KEY
/// is not on the walk, so `HashMap<&'static str, Guard>` owns its `Guard` values (EXECUTED: 1 drop)
/// although the type contains a `&`.
pub(crate) fn elem_type_b(ty: &syn::Type, uses: &HashMap<String, String>) -> Option<(String, bool)> {
    let plain = elem_type_b_plain(ty, uses);
    // SOUNDNESS R1023 — the one-level answer stands wherever it names a NOMINAL element (`Vec<G>` -> `G`,
    // `Mutex<G>` -> `G`): that is every entry the collector's one-level consumers were written for, and
    // it is unchanged. Where it names a std container/`Option` instead (`Option<Vec<G>>` -> `Vec`, one
    // level off) or nothing (`Mutex<Option<G>>`), a type two or more std layers above a nominal leaf is
    // recorded LAYERED (see `WRAPPED_CONTAINER_MARK`), so the binders can peel it a layer at a time.
    if plain.as_ref().is_some_and(|(t, _)| is_wrapped(t) || is_layer_leaf(t)) {
        return plain;
    }
    if let Some((ls, g, b)) = layer_walk(ty, uses) {
        if ls.len() >= 2 {
            if std::env::var_os("CANDOR_R1023_INSTR").is_some() {
                eprintln!("R1023LAYER\t{}", ls.iter().map(|l| l.0).collect::<String>());
            }
            return Some((encode_layers(&g, &ls), b));
        }
    }
    plain
}

fn elem_type_b_plain(ty: &syn::Type, uses: &HashMap<String, String>) -> Option<(String, bool)> {
    match ty {
        syn::Type::Reference(r) => elem_type_b(&r.elem, uses).map(|(t, _)| (t, true)),
        syn::Type::Paren(p) => elem_type_b(&p.elem, uses),
        syn::Type::Group(g) => elem_type_b(&g.elem, uses),
        // `[T]` (slice) and `[T; N]` (array) — the element is the type directly.
        syn::Type::Slice(s) => type_path_b(&s.elem, uses),
        syn::Type::Array(a) => type_path_b(&a.elem, uses),
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
                n if is_sequence_container(n) => type_path_b(first_ty, uses),
                // SOUNDNESS R454 — a MAP's VALUE (2nd type arg), the arm `elem_trait_leaves` has had
                // since R46 and this one never got. The consequence was an asymmetry nobody chose:
                // `HashMap<String, Box<dyn Doer>>` answered and `HashMap<String, G>` did not, so which
                // silence you got depended on whether the value happened to be a trait object — the
                // same shape R347 recorded for the adapter list, one level up.
                //
                // R347's NOTE SAID A MAP ARM "HITS THE FABRICATION ROUTE". IT DOES NOT — the route it
                // named cannot fire for a map, and that was measured before this shipped.
                // `iter`/`into_iter`/`drain` are on `is_element_preserving_adapter` and on a MAP they
                // yield `(&K, &V)` rather than `&V`, so `m.iter().for_each(|x| x.run())` would type `x`
                // as the VALUE. Two things close it: that spelling does not COMPILE (a tuple has no
                // `run`), and the spelling that DOES occur — `m.iter().for_each(|(k, v)| ..)`, and
                // `for (k, v) in &m` — goes through `resolve_elem_tuple`, which has NO map arm and so
                // contributes no binding at all rather than a wrong one. Those two tuple spellings stay
                // a STATED under-report (`a_maps_tuple_spellings_are_still_an_under_report` pins them),
                // because recovering them means answering with a PAIR and that is a different change.
                //
                // MEASURED over 1,561 registry crates: **ADDED 18 · REMOVED 0 · CHANGED 51** (4 on
                // `inferred`), reach **3,811** element resolutions across **307** crates, and 0 changed
                // rows lose an effect. The reach is large and the movement is small because a map's
                // VALUE is usually plain data; where it is not, the silence was total — all six
                // spellings (`m[k]`, `m.get(k).unwrap()`, `for v in m.values()`, `m.values()` HOF, and
                // both param/local forms) went ABSENT while the `Vec` controls beside them charged.
                //
                // THE ONE OVER-CHARGE, TRACED RATHER THAN EXPLAINED AWAY: 14 rows in lapin (3 versions)
                // gain a drop-glue edge and 2 of them gain `Log`, because `channels::Inner` owns a
                // `Channel` through a map and `owned_drops` is LEAF-KEYED, so the unrelated
                // `frames::Inner` inherits it. **That is R213, it is pre-existing, and the arms of the
                // fixture prove it: a `b::Inner` beside an `a::Inner { v: Vec<Closer> }` is charged
                // `Exec` on BOTH sides of this change** — the map arm only hands an already-broken
                // index a new and CORRECT fact. Over-charge, disclosed in `inferred`, one crate.
                n if is_map_container(n) => {
                    let v = args.args.iter().filter_map(|a| match a {
                        syn::GenericArgument::Type(t) => Some(t),
                        _ => None,
                    }).nth(1).and_then(|v| type_path_b(v, uses));
                    if v.is_some() && std::env::var_os("CANDOR_R454_INSTR").is_some() {
                        eprintln!("R454HIT\t{}\t{}", n, v.as_ref().map(|(t, _)| t.as_str()).unwrap_or(""));
                    }
                    v
                }
                // SOUNDNESS R185 — `Option<T>` yields `T` here too. It is not a collection in the
                // sense of the doc above, but every CONSUMER of this function asks the same question —
                // "if I bind a name out of this type, what is the name's type?" — and `Option` answers
                // it: `if let Some(h) = &self.o` binds `h: &T`, and `for h in &self.o` is legal Rust
                // that binds the same thing. Measured before the change: a field `Vec<Guard>` and a
                // bare field `Guard` both charge `Fs`, while `Option<Guard>` reads ABSENT — the same
                // value, the same call, silent on the one wrapper nobody added.
                // …and `Result<T, E>`, whose FIRST arg is the payload for exactly the same reason.
                // `if let Ok(h) = &self.r` binds `h: &T`, and `for h in &self.r` is legal Rust too.
                // The error type is deliberately not reachable here: nothing binds a name out of it
                // through any of this function's callers.
                "Option" | "Result" | "IoResult" | "Bound" => type_path_b(first_ty, uses),
                // VEIN B (R878, R568) — a std interior-mutability / lazy-init WRAPPER holds exactly one
                // value of its type argument, the same shape as `Option`. Recorded here so the wrapper
                // accessors (`is_wrapper_accessor`) can answer `self.db.borrow_mut()` with `T`.
                // NOT when the argument is itself a container, `Option`/`Result` or another wrapper
                // (`Mutex<Option<X>>`, `Mutex<Vec<G>>`): that value names nothing the chain goes on to call,
                // and recording it fed the element-preserving adapters (`lock`, `as_ref`) an `Option` where
                // they had nothing before — measured on the chained corpus, mongodb's
                // `inner.lock().await.as_ref()?.cache` lost its ⟨0.40⟩ `dispatch:` disclosure. R347's
                // guard-chain half (`Mutex<Vec<Guard>>`) stays open, as it was.
                n if is_value_wrapper(n) => type_path_b(first_ty, uses).filter(|(t, _)| is_held_nominal(t)),
                // SOUNDNESS R893 / R1023 — a wrapper of a container or of an `Option` (`Mutex<Vec<G>>`,
                // `Mutex<Option<G>>`) is TWO std layers deep and is recorded LAYERED by `elem_type_b`'s
                // fallback below, never here: the plain form means "the held value is G" to
                // `wrapper_accessor_type`, the payload binders and the HOF closure route.
                // Smart-pointer wrappers around a collection/slice (`Box<[T]>`, `Arc<Vec<T>>`,
                // `Rc<[T]>`) — peel one layer and recurse so the inner collection's element surfaces.
                //
                // SOUNDNESS R347 — the INTERIOR-MUTABILITY wrappers (`Mutex`, `RwLock`, `RefCell`,
                // `OnceLock`…) BELONG here and are deliberately ABSENT. Adding them closes a real
                // silence — `self.m.lock().unwrap().iter().for_each(|h| h.run())` over an
                // `Arc<Mutex<Vec<Guard>>>` reads ABSENT while the identical statement over
                // `Arc<Mutex<Vec<Box<dyn Doer>>>>` charges `Fs`, because the DISPATCH index peels them
                // and this one does not. It was written, measured working, and BACKED OUT.
                //
                // WHAT IT COST, measured on the corpus rather than reasoned about: peeling `Mutex` here
                // makes the HOF closure-typing route hand a container element to a closure whose
                // parameter is NOT one. `async-process-2.5.0`'s
                // `!self.zombies.lock().unwrap_or_else(|x| x.into_inner()).is_empty()` types `x` — a
                // `PoisonError` — as the map's `ChildGuard` element, so `x.into_inner()` resolves to
                // `ChildGuard::into_inner`, and `Reaper::has_zombies` is charged **Exec** for asking
                // whether a map is empty. A fabrication on a function that spawns nothing.
                //
                // So the type peel needs the closure route to know that `unwrap_or_else`'s parameter is
                // the ERROR and not the element, which is a separate fix in a separate place. The
                // silence is recorded rather than traded for an over-report.
                "Box" | "Arc" | "Rc" => elem_type_b(first_ty, uses),
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
                // `ret_dispatch_leaves` already produce. It can only turn silence into `Unknown` GIVEN
                // that no crate in scope defines its own `trait Fn` — see `leaves_are_callable`'s R272
                // note, which states that condition once and records what happens when it fails.
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
                // `"Fn"` leaf every other callable site already produces: it turns a silent drop into
                // `Unknown` GIVEN no crate in scope defines its own `trait Fn` (R272 — the condition is
                // stated at `leaves_are_callable`, not re-asserted here).
                if is_callable_type(t, generic_bounds, callable_aliases) {
                    if std::env::var("CANDOR_ALIAS_DEBUG").is_ok() {
                        eprintln!("R177ELEMALIAS {name}");
                    }
                    return vec!["Fn".to_string()];
                }
                Vec::new()
            };
            match name.as_str() {
                n if is_sequence_container(n) => dispatch(first_ty),
                // Option<Box<dyn T>> / Result<Box<dyn T>, E> — the payload (Ok/Some) is a trait object; its
                // leaves let `o.map(|d| d.go())` / `for d in o` / `o.iter().for_each(..)` dispatch. (if-let /
                // `.unwrap()` are separate binding sites handled at their pattern.)
                "Option" | "Result" => dispatch(first_ty),
                // SOUNDNESS R1034 residue — `std::ops::Bound<&T>`, whose `Included`/`Excluded` payload is a `T`
                // (diesel's `ranges::to_sql` dispatches `value.to_sql(..)` on it, `T: ToSql`).
                "Bound" => dispatch(first_ty),
                // A MAP's VALUE (2nd type arg) — a `.values()`/`for v in m.values()` iteration of
                // trait-object values (`HashMap<String, Box<dyn Handler>>`, the keyed-registry shape).
                // R454 — the list is shared with `elem_type`, which did not have this arm at all.
                n if is_map_container(n) => type_args().nth(1).map(dispatch).unwrap_or_default(),
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
                // SOUNDNESS R401 — `Weak<T>` belongs on THIS list and on no other, by the criterion
                // the `OnceLock` note above already states. `Weak` does NOT `Deref`, so `w.go()` does
                // not compile and peeling it in `trait_leaves` would fabricate a receiver; its payload
                // is reached through the named accessor `upgrade()`, which is the ELEMENT route — the
                // same shape as `OnceLock::get`. Measured at HEAD with `p_box`/`p_arc`/`p_vec`/`p_opt`
                // charging `['Exec']` as the calibration and the guard opened by a `-> Box<dyn Doer>`
                // factory: `if let Some(s) = w.upgrade() { s.go() }` over a `Weak<dyn Doer>` was ABSENT.
                "Box" | "Arc" | "Rc" | "Weak" | "Mutex" | "RwLock" | "RefCell" | "Cell"
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

/// SOUNDNESS R272 — THE CONDITION THE SYNTHETIC `"Fn"` HEDGE RESTS ON, STATED ONCE AND AS AN
/// ASSUMPTION, BECAUSE THE UNCONDITIONAL READING IS FALSE.
///
/// Several sites write a synthetic `"Fn"` leaf for a value whose declared type is invokable but
/// carries no trait in its syntax (`cb: fn()`, a callable type ALIAS, `Option<fn(..)>`, a
/// `RET_FN_TYPED` return, a callable `static`). Each used to assert, in its own words, that this
/// "matches no local trait, so no CHA fan-out, no `Type::method` edge and no concrete effect can come
/// out of it — the only reachable outcome is `Unknown`".
///
/// **THAT HOLDS ONLY WHILE NO CRATE IN SCOPE DEFINES ITS OWN `trait Fn` / `FnMut` / `FnOnce`.** Where
/// one does, the hedge hands the bounded CHA what it reads as that trait's name and the caller is
/// charged an effect from an implementor its receiver can never be. EXECUTED: `call_it(h) { h.cb.go() }`
/// over `cb: fn()` provably creates no file and is reported `['Fs']`, with caller-scoped `deny Fs
/// call_it` exit 1. `Fn` is not a name real crates give a trait — 0 of 1,509 registry crates define
/// one — but that is an assumption about the ecosystem, not a property of this code, and it is written
/// here as one.
///
/// **WHY THE HEDGE IS NOT SIMPLY RE-SPELLED.** Bracketing it (`<callable>`, the family's own
/// `<dyn>`/`<elemdyn>`/`<fn>`/`<lazy>` convention) makes the claim true by construction, was built,
/// and was REJECTED on measurement: it removes the over-charge and introduces a SILENT UNDER-REPORT
/// one character away, where the `impl Fn for fn()` is itself effectful — the caller then resolves to
/// nothing and is certified pure over a file it really writes. An over-charge is disclosed as an
/// effect the caller may not perform; the silence is a purity claim over one it does, and this family
/// ranks those. Both fixtures are pinned in `r272_a_crate_that_defines_its_own_trait_fn`; read them
/// before narrowing this. A real fix has to keep the effect — a method call on a hedge-typed receiver
/// disclosing `Unknown` rather than resolving to nothing — and its over-charge surface is not
/// measurable on a corpus where neither shape occurs (that A/B is byte-identical, which says only
/// that, §E1).
///
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
/// `ret_dispatch_leaves` decodes `RET_FN_TYPED` into, which cannot contribute a CONCRETE effect GIVEN no
/// crate in scope defines its own `trait Fn` (R272, stated at `leaves_are_callable`). It CAN WITHDRAW one: the consuming arm in `resolve_elem_trait_leaves`
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
///
/// SOUNDNESS R807 — `bind` WAS ABSENT, AND IT IS THE ONLY CONSTRUCTOR A UDP SOCKET HAS. So
/// `let s = UdpSocket::bind("0.0.0.0:0")?; s.send_to(buf, dst)` left `s` untyped, the `send_to` never
/// became `UdpSocket::send_to`, and the caller-chosen destination reached no rule and no masking arm:
/// `allow Net in <fn> <benign literal>` exited 0 on the unit AND its caller, EXECUTED sending a real
/// datagram to the caller's address. The same function taking the socket as a PARAMETER was typed from
/// the signature and failed closed — the local bind was the single variable. (The row as filed blamed
/// `is_net_binding` withholding a collapsed record; measured, no such record exists — the `bind` record
/// is withheld correctly and the `send_to` was never classified at all.) `connect` sat here already,
/// so the asymmetry was one name. std, tokio, mio, async-std and smol all spell a UDP socket's and a
/// listener's constructor `bind`, returning `Self` (or `Result<Self>`, or a future of it).
pub(crate) fn is_ctor(name: &str) -> bool {
    matches!(
        name,
        "new" | "default" | "builder" | "with_capacity" | "connect" | "open" | "init" | "from"
            | "from_path" | "from_str" | "with_config" | "create" | "bind"
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
                    // R807 REACH PROBE — the one name R807 adds, so the A/B can count what it typed.
                    if last == "bind" && std::env::var_os("CANDOR_MASK_DEBUG").is_some() {
                        eprintln!("R807CTOR {}::bind", expand(ty, uses));
                    }
                    return Some(expand(ty, uses));
                }
                // SOUNDNESS R807 — THE PRECONDITION PROBE, diagnostic only. A `Type::f(..)` factory that
                // `classify` CHARGES but that is not a known constructor leaves the value it produces
                // untyped, so every later method on it (`s.send_to(buf, dst)` after
                // `let s = UdpSocket::bind(..).unwrap()`) reaches no rule and no masking arm — and so
                // prints none of the mask probes. `R807UNTYPED` (scan.rs) sees only the `?` and inline
                // spellings, because only those leave a `<untyped>` marker; `.unwrap()`/`.expect()`
                // leave NOTHING, which is the commonest spelling. This line fires wherever typing is
                // attempted and fails on such a factory, whatever the binder, so it is the census the
                // fix's reach is read against.
                if type_like && std::env::var_os("CANDOR_MASK_DEBUG").is_some() {
                    let full_ty = expand(ty, uses);
                    let cr = full_ty.split("::").next().unwrap_or("");
                    let p = format!("{full_ty}::{last}");
                    if let Some(eff) = candor_classify::classify(cr, &p) {
                        eprintln!("R807CTORMISS {eff} {p}");
                    }
                }
            }
            // a local factory function call — its recorded (unambiguous) return type. The fn-typed
            // sentinel is NOT a nominal type (it types no var / receiver) — `expr_is_fn_typed` owns it.
            // Neither sentinel is a NOMINAL type: `RET_FN_TYPED` types no var (a callback), and the
            // `RET_DYN_PREFIX` dispatch-object return is resolved by TRAIT (via `resolve_recv_traits`'s
            // Call arm), never as a concrete `Type::method`. Filter both out of concrete var-typing.
            //
            // VEIN B (R197) — A TURBOFISH NAMES A GENERIC RETURN. `net::mk::<net::Conn>()` over
            // `fn mk<T: Default>() -> T`: the plain recorded return below is the PARAMETER NAME (`T`), a
            // type no value has, so the binding typed to nothing and `c.send()` read ABSENT while the
            // annotated twin charged. `ret_generic_key` holds the parameter's position only when every
            // same-leaf contributor agrees, so a non-generic `mk` elsewhere withdraws it.
            if let Some(t) = turbofish_return(p, leaf, uses, returns) {
                return Some(t);
            }
            recorded_return_type(leaf, returns).or_else(|| qual_recorded_return(&full, uses, returns))
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

/// VEIN B (R197, R733) — the TYPE ARGUMENT a call-site turbofish supplies for the generic parameter the
/// callee's return names. `None` unless `ret_generic_key(leaf)` holds a position (every same-leaf
/// contributor agreed) and the last path segment carries a type argument there.
pub(crate) fn turbofish_return_ty<'e>(p: &'e syn::ExprPath, leaf: &str, returns: &ReturnIndex) -> Option<&'e syn::Type> {
    let pos: usize = returns.get(&crate::model::ret_generic_key(leaf))?.parse().ok()?;
    let syn::PathArguments::AngleBracketed(args) = &p.path.segments.last()?.arguments else { return None };
    args.args
        .iter()
        .filter_map(|a| match a {
            syn::GenericArgument::Type(t) => Some(t),
            _ => None,
        })
        .nth(pos)
}

/// `turbofish_return_ty` as a nominal type path (`type_path`, so `&`/`Box`/`Arc`/`Rc` peel as every
/// other binder peels them). `_` and a non-path type answer nothing.
pub(crate) fn turbofish_return(
    p: &syn::ExprPath,
    leaf: &str,
    uses: &HashMap<String, String>,
    returns: &ReturnIndex,
) -> Option<String> {
    let t = turbofish_return_ty(p, leaf, returns)?;
    let tp = type_path(t, uses)?;
    if tp == "_" {
        return None;
    }
    if std::env::var_os("CANDOR_VEINB_INSTR").is_some() {
        eprintln!("VEINB_TURBOFISH\t{leaf}\t{tp}");
    }
    Some(tp)
}

/// The recorded return type of a fn LEAF, as a NOMINAL type path — the one authority for "what concrete
/// type does calling this local factory produce". Filters out both of `record_return`'s sentinels: the
/// fn-typed one (a callback types no var — `expr_is_fn_typed` owns it) and the three `<dyn>` dispatch
/// shapes (resolved by TRAIT, never as a concrete `Type::method`).
///
/// R174(b): `RET_UNIT` is filtered here too — a leaf that is BOTH a factory and a `()`-returning fn
/// names no type at the call site, which is the same "no claim" answer the ambiguity rule gives.
///
/// Extracted so `ctor_type` (the `let`-binding type inference) and `ctor_leaf_from_call_returns` (the
/// R165 drop-glue route) cannot answer it differently. They HAD to, before: the drop marker's binder-keyed
/// predecessor consulted this index and the position-independent rewrite that replaced it did not, so a
/// free-function constructor stopped being a construction at all.
/// SOUNDNESS R893 — the declared return of the FREE fn a written call path names, by its crate-anchored
/// qual (`model::qual_ret_key`). Consulted only after the leaf-keyed answer declines (two same-named fns
/// withdrew it), and only for a path `expand` anchors in this crate: a dependency's `dep::mk()` has no
/// entry here by construction, and a bare `mk()` stays the leaf route's question.
pub(crate) fn qual_recorded_return(written: &str, uses: &HashMap<String, String>, returns: &ReturnIndex) -> Option<String> {
    let full = expand(written, uses);
    if !full.starts_with("crate::") {
        return None;
    }
    let t = returns.get(&crate::model::qual_ret_key(&full))?;
    if std::env::var_os("CANDOR_ALIAS_DEBUG").is_some() {
        eprintln!("R893QUALRET {full}");
    }
    Some(t.clone())
}

pub(crate) fn recorded_return_type(leaf: &str, returns: &ReturnIndex) -> Option<String> {
    returns
        .get(leaf)
        .filter(|t| *t != RET_FN_TYPED && *t != RET_UNIT && ret_dyn_leaves(t).is_none()
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
/// signature says it returns. `ReturnIndex` drops any leaf recorded with two CONFLICTING returns, and
/// `note_construction`'s `drop_relevant` gate means only a type with a local `impl Drop` survives — so a
/// cross-crate leaf collision has to hit a local `Drop` type of the same name to matter, and when it
/// does the direction is an over-charge, never silence.
///
/// SOUNDNESS R174(b) — "any leaf recorded with two different return types" is what that sentence said
/// before, and it was FALSE for the commonest conflict of all: `record_return` used to return early on
/// a UNIT return, so a `fn init()` twin left a `fn init() -> Repository` unambiguous. Measured on git2
/// (free `crate::init()` beside `Repository::init`): 76 functions per version gained a phantom
/// `Repository::drop` edge, invisible in `inferred` only because `git_repository_free` is unclassified,
/// while `path`/`callers`/`gains` were already wrong. A unit return is now recorded as a conflicting
/// shape like every other.
///
/// STATED LIMIT: leaf-keyed like the index itself, so `serde_json::from_str` and a local `from_str` are
/// one name to this route. Deliberate — the alternative (single-segment paths only) draws the boundary
/// around the one spelling the row was filed for, and a `use m::from_handle` import already expands to
/// a multi-segment path before it gets here.
///
/// SOUNDNESS R174(a) — IT HONOURS THE PATH ROUTE'S STD REFUSAL, and the paragraph above is why it has
/// to be repeated here rather than inherited. `ctor_leaf_from_call_path` declines a `std`/`core`/
/// `alloc`-rooted callee ON PURPOSE (`local_type_leaf`'s comment has the measured tokio `Acquire`
/// story); this route then keyed the SAME call on its bare leaf and undid the refusal. Measured: a
/// crate with a local `fn open(..) -> Conn` (Drop = Net) charged `Conn::drop` to every `File::open(p)?`
/// caller — `read_it` `['Fs']` -> `['Fs','Net']`, `count` (`HashMap::new()`) absent -> `['Net']`,
/// `is_dir` (`Path::new(p).is_dir()`) `['Fs']` -> `['Fs','Net']`; 3,896 std-rooted hits across 480
/// registry crates. `std::mem::size_of`, `Vec::with_capacity` and `ManuallyDrop::new` are all one
/// leaf collision away from a local factory of the same name, and none of them constructs anything a
/// local `impl Drop` describes.
pub(crate) fn ctor_leaf_from_call_returns(full: &str, returns: &ReturnIndex) -> Option<String> {
    if matches!(full.split("::").next(), Some("std") | Some("core") | Some("alloc")) {
        // §E1 HIT COUNTER — printed only when the refusal actually withdraws an answer this route
        // would otherwise have given, not on every std-rooted call.
        if std::env::var("CANDOR_ALIAS_DEBUG").is_ok() {
            let last = full.rsplit("::").next().unwrap_or(full);
            if last != "drop" && recorded_return_type(last, returns).is_some() {
                eprintln!("R174STD {full}");
            }
        }
        return None;
    }
    let last = full.rsplit("::").next().unwrap_or(full);
    if last == "drop" {
        return None;
    }
    // R174(b)/R188 — a UNIT-returning twin of this leaf makes the call an ambiguity FOR THIS ROUTE.
    // `c::init()` in statement position is exactly where git2's phantom `Repository::drop` came from,
    // and this route cannot tell which of the two same-named callees a leaf-keyed lookup answered for.
    // Read from its own key so the refusal reaches nothing else: R174(b) expressed it by ambiguating
    // the leaf in `rets` itself, which also withdrew `let`-typing and cost a real `Net` (R188).
    if returns.contains_key(&unit_twin_key(last)) {
        // §E1 HIT COUNTER — printed only when the refusal withdraws an answer this route would
        // otherwise have given, never on every unit fn.
        if std::env::var("CANDOR_ALIAS_DEBUG").is_ok() && recorded_return_type(last, returns).is_some() {
            eprintln!("R174UNIT {last}");
        }
        return None;
    }
    local_type_leaf(&recorded_return_type(last, returns)?)
}

/// The type a VALUE path denotes, for `let` inference: `S` → `S`; `m::S` → `m::S`; `Color::Red`
/// (UpperCamel::UpperCamel = a unit enum variant) → `Color`. Only CamelCase leaves count as types —
/// a snake_case variable or SCREAMING_SNAKE const yields None (no inference; honest under-report).
pub(crate) fn type_from_value_path(full: &str, uses: &HashMap<String, String>) -> Option<String> {
    let segs: Vec<&str> = full.split("::").collect();
    let last = segs.last()?;
    // SOUNDNESS R722 — `is_type_ident`, NOT a second copy of it. This fn used to carry a local `camel`
    // closure that was the same two clauses written again, so widening one and not the other would have
    // reintroduced exactly the §G drift this row is about: `is_type_ident` gates the CALL-path route
    // while this gates `let` inference and hence receiver typing.
    if !is_type_ident(last) {
        return None;
    }
    // `u32::MAX` / `f64::EPSILON` / `f32::consts::PI` — an associated const of a PRIMITIVE. Reachable
    // only because R722's union admits an all-caps leaf, and unbounded in a way the rest of that
    // exposure is not: `u32::MAX` appears in ordinary code everywhere, so leaf-keying it as a local
    // `MAX` would fabricate wherever any crate happens to declare an all-caps `MAX` type. `mod u32 {}`
    // is legal Rust and would make this refuse a real local type — see `is_primitive_root`, where that
    // corner is measured rather than waved away.
    if segs.len() >= 2 && is_primitive_root(segs[0]) {
        if std::env::var_os("CANDOR_ALIAS_DEBUG").is_some() {
            eprintln!("R722PRIM {full}");
        }
        return None;
    }
    // `Enum::Variant` — two trailing type-shaped segments: the VALUE's type is the enum (the
    // penultimate).
    if segs.len() >= 2 && is_type_ident(segs[segs.len() - 2]) {
        // ...UNLESS THE TRAILING SEGMENT IS ALL-CAPS, in which case it is an ASSOCIATED CONST and its
        // type is the CONST's declared type, NOT the enclosing one. FOUND BY THE A/B, not reasoned
        // about: `windows-core`'s `TearOff::WeakQueryInterface` compares `*iid == crate::IUnknown::IID`,
        // and `IUnknown` has an `impl Drop` — so the variant rule typed a `GUID` constant as an
        // `IUnknown` and charged the function `IUnknown::drop`. `IWeakReference::IID` and
        // `IAgileObject::IID` are the same shape one line over. This is [[R168]]'s `Ordering::Acquire`
        // measurement one level in: there the colliding const was module-level, here it is ASSOCIATED,
        // so `caps_leaf_shadowed_by_const` — which reads `collect_static_types`, a module-level index —
        // cannot see it.
        //
        // THE COST, stated: an ALL-CAPS ENUM VARIANT (`Color::RED`) is refused too, because nothing in
        // the path distinguishes it from `Color::MAX`. Variants are UpperCamel by convention and by
        // clippy, an associated const is SCREAMING by both, and the two errors are not symmetric — a
        // refused variant is an under-report of a charge that never shipped, while a mistyped assoc
        // const FABRICATES onto ordinary code.
        if caps_only_ident(last) {
            if std::env::var_os("CANDOR_ALIAS_DEBUG").is_some() {
                eprintln!("R722ASSOCCONST {full}");
            }
            return None;
        }
        return Some(expand(&segs[..segs.len() - 1].join("::"), uses));
    }
    // §E1 REACH COUNTER, on the CHANGED branch only — an all-caps leaf this fn now answers for and
    // previously refused. An unchanged row is not evidence the new code ran.
    if caps_only_ident(last) && std::env::var_os("CANDOR_ALIAS_DEBUG").is_some() {
        eprintln!("R722CAPS {full}");
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
    let leaf = expanded.rsplit("::").next().unwrap_or(&expanded).to_string();
    // VEIN A — and the UNFOLLOWED reading beside it, `\u{1}`-joined, when the error type is spelled through
    // a local type ALIAS: `impl From<X> for ConsumerInfoError` on `type ConsumerInfoError = Error<Kind>`
    // keys under the alias's name (async-nats), and once `use super::context::ConsumerInfoError` is
    // anchored the followed leaf is `Error`. `visit_expr_try` charges each leaf a `From` impl exists for.
    if let syn::Type::Path(tp) = err {
        if tp.qself.is_none() {
            let raw = expand_noalias(&path_to_string_lc(&tp.path), uses);
            let raw_leaf = raw.rsplit("::").next().unwrap_or(&raw).to_string();
            if raw_leaf != leaf && !raw_leaf.is_empty() {
                return Some(format!("{leaf}\u{1}{raw_leaf}"));
            }
        }
    }
    Some(leaf)
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
    expand_with(path, uses, true, true)
}

/// VEIN A — `expand` without following a MODULE-QUALIFIED alias (`qualified_alias`): the path as the
/// source names it, made absolute. Asked only by the call site, for the one shape where the two
/// readings name different units: an `impl` written ON a local type alias (`impl TzifOwned { fn
/// parse32 }` in jiff) keys its methods under the ALIAS's name, so following `TzifOwned` to its target
/// `Tzif` finds no `Tzif::parse32` — measured, jiff-0.2.28's `TzifOwned::parse` lost a real `Log`
/// once `use super::TzifOwned` was anchored and became followable. The caller pushes this reading
/// BESIDE the followed one; whichever names no unit resolves to nothing.
pub(crate) fn expand_noalias(path: &str, uses: &HashMap<String, String>) -> String {
    expand_with(path, uses, true, false)
}

/// VEIN A — `expand` WITHOUT the two vein-A anchors (a written `crate::` head kept, a module-declared
/// relative head made absolute), for a MACRO path. A macro is not a value or a type: `crate::debug!` and
/// `crate::tracing::implementation::event!` name macros, which the engine resolves through its own
/// leaf-keyed template table (`local_macros`) or the classifier, and both were built against the
/// stripped spelling. Measured: anchoring macro paths took `Log` off 1,277 x11rb rows whose `crate::debug!`
/// expands to `$crate::tracing::implementation::event!` → `tracing::event!`. Macro resolution is not
/// this change's subject, so macro paths keep exactly today's answer.
pub(crate) fn expand_noanchor(path: &str, uses: &HashMap<String, String>) -> String {
    expand_with(path, uses, false, true)
}

fn expand_with(path: &str, uses: &HashMap<String, String>, anchor: bool, follow: bool) -> String {
    // A leading `::` (only `path_to_string_lc` produces one) is an EXTERN-crate path: no module anchor
    // applies to it. Everything else is answered exactly as before the colon was kept.
    if let Some(rest) = path.strip_prefix("::") {
        return expand_with(rest, uses, false, follow);
    }
    let mut segs: Vec<&str> = path.split("::").collect();
    // A path rooted at `crate`/`self`/`super` is EXPLICITLY crate-local — it is NOT subject to the file's
    // `use` aliases, so after stripping the prefix we return it as-is. (Re-applying `uses` here would let
    // a `use other::config;` import hijack a local `crate::config::load` call.)
    let rooted_local = matches!(segs.first().copied(), Some("crate" | "self" | "super"));
    // SOUNDNESS R751 — recover the module a `self::`/`super::` path is relative to BEFORE the collapse
    // below discards it. `abs` is the crate-root-absolute form; the only OUTPUT it changes is this
    // branch's literal fallthrough, which is returned `crate::`-rooted so downstream reads the
    // absoluteness assertion rather than a proxy for it. See `absolutise`.
    let abs = if !anchor && segs.first() == Some(&"crate") { None } else { absolutise(&segs, uses) };
    if let Some(a) = &abs {
        segs = a.split("::").collect();
        if std::env::var("CANDOR_R186_DEBUG").is_ok() {
            eprintln!("R751ABS {path} -> crate::{a}"); // §E1 REACH COUNTER
        }
    }
    while matches!(segs.first().copied(), Some("crate" | "self" | "super")) {
        segs.remove(0);
    }
    if segs.is_empty() {
        return path.to_string();
    }
    if !rooted_local {
        // SOUNDNESS R982 — inside an item the default build does not have (a `CfgOffScope`), a name bound
        // only by a FEATURE-INACTIVE `use` of this module is bound in every build that compiles the item
        // (redis `create_rustls_config`'s `load_native_certs()` under `#[cfg(feature = "tls-rustls")]`).
        let off;
        let hit = match uses.get(segs[0]) {
            Some(v) => Some(v),
            // Only an EXTERNAL target: a crate-local one (`crate::sys::local_offset_at`, rustix's
            // `crate::backend::conv::ret`) lands in cfg-selected local modules this scanner resolves
            // poorly, and MEASURED it withdrew the `ambiguous:` hedge that stood over real FFI / clock
            // reads (time `UtcOffset::local_offset_at`, rustix `try_close`) for nothing in its place.
            // SOUNDNESS R982 (residual) — …EXCEPT a RENAMED one (`use crate::imp::deep::eff2 as renamed;`). The
            // exclusion above is about a crate-local name that ALSO resolves some other way — by its leaf, or
            // to an `ambiguous:` hedge when two modules define it (`call_bare` below) — so dropping the
            // binding left an answer standing. A rename has no leaf of its own: nothing else can name the
            // target, and `renamed()` read ABSENT under `#[cfg(feature = "x")]` (executed with the feature: it
            // writes a file). The measured losses (time `local_offset_at`, rustix `try_close`) are both
            // non-renamed and stay excluded.
            None if cfg_off_active() => {
                off = uses.get(&format!("{CFG_OFF_USE_PREFIX}{}", segs[0])).filter(|v| {
                    let local = v.starts_with("crate::") || v.starts_with("self::") || v.starts_with("super::");
                    let renamed = v.rsplit("::").next() != Some(segs[0]) && !v.contains(crate::decls::ALIAS_ALT_SEP);
                    if local && renamed && std::env::var_os("CANDOR_R982_INSTR").is_some() {
                        eprintln!("R982RENAMED\t{}", v);
                    }
                    !local || renamed
                });
                off
            }
            None => None,
        };
        if let Some(full) = hit {
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
                    if let Some(q) = qualified_alias(&s2, true, uses).filter(|_| follow) {
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
        if let Some(full) = qualified_alias(&segs, false, uses).filter(|_| follow) {
            return full;
        }
        // VEIN A — A RELATIVE PATH WHOSE HEAD THE CURRENT MODULE DECLARES NAMES THAT DECLARATION, AND
        // NOTHING ELSE. `a::Tx::grab` written in `mod x { pub mod a {..} }` is `x::a::Tx::grab` in
        // rustc: an item declared in the module shadows the extern prelude and every glob, and an
        // explicit `use` of the same name in the same module is E0255 — so once the `use` lookups above
        // have missed, a declared head is decisive. Returned crate-rooted, which is the absoluteness
        // assertion `arm_exact_target` ranks on (R748(i)); without it the path reached `tail2` as
        // `Tx::grab`, tied with the root's own `a::Tx::grab`, and R830's `x::f_rel` went ABSENT.
        //
        // MULTI-SEGMENT ONLY. A one-segment name keeps `by_leaf` (the R751 root rule's reason, and the
        // value namespace a bare call lives in is not the type namespace `MODDECL_KEY` lists). The head
        // of a multi-segment path is always a type-namespace name, which is what is seeded.
        if anchor && segs.len() >= 2 && module_declares(uses, segs[0]) == Some(true) {
            if let Some(modpath) = uses.get(MODPATH_KEY) {
                let abs = if modpath.is_empty() {
                    segs.join("::")
                } else {
                    format!("{modpath}::{}", segs.join("::"))
                };
                if std::env::var("CANDOR_R186_DEBUG").is_ok() {
                    eprintln!("VEINAREL {path} -> crate::{abs}"); // §E1 REACH COUNTER
                }
                // The anchored form is the crate-rooted key a module-qualified alias is seeded under
                // (`seed_mod_aliases`), so ask it exactly as an explicitly written `crate::` path is.
                let abs_segs: Vec<&str> = abs.split("::").collect();
                if let Some(q) = qualified_alias(&abs_segs, true, uses).filter(|_| follow) {
                    return q;
                }
                return format!("crate::{abs}");
            }
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
    if let Some(full) = qualified_alias(&segs, true, uses).filter(|_| follow) {
        return full;
    }
    if let Some(full) = uses.get(&format!("crate::{}", segs[0])) {
        return alias_join(full, &segs[1..]); // R105 — may carry several `#[cfg]`-duplicated arms
    }
    // The crate's UNIQUE re-export glob: the seeded root glob (`crate::` + GLOB_KEY, the cross-file case) or,
    // failing that, a glob in THIS file's own `use` map (`GLOB_KEY` — the single-file/collect-time case,
    // iso_A/iso_C). Attribution only; a genuinely-local `net` module still resolves by tail2 downstream.
    //
    // SOUNDNESS R186 — **"ATTRIBUTION ONLY … LOCAL WINS … NO FABRICATION" IS TRUE FOR A CALL PATH AND
    // FALSE FOR A TYPE PATH, AND THAT ASYMMETRY IS THE WHOLE DEFECT.** A call keeps its 2-segment tail, so
    // a genuinely-local `net::foo` is rescued downstream by the tail2 index. A TYPE has no such rescue:
    // the receiver-typing route stores exactly this string, so one `use std::io::prelude::*` anywhere in
    // the file — or, through the seeded `crate::*`, anywhere in `lib.rs` — renamed `crate::stream::Stream`
    // to `std::io::prelude::stream::Stream`, matched no local definition, dropped the CALL EDGE from
    // `p.wibble()` entirely, and left the caller absent from `functions[]` with `deny Fs <mod>` at exit 0
    // while the body provably read a file. Measured on two byte-identical modules differing only in that
    // `use` line, both EXECUTED.
    //
    // TWO CLAIMS, AND EACH IS DRIVEN BY A TEST THAT FAILS WITHOUT IT — stated that way because the
    // sentence this replaced ("ATTRIBUTION only … local wins … no fabrication") read as considered and
    // was unfalsifiable in place, which is exactly what stopped it being measured for months.
    //
    //  (1) For a path whose head the crate ROOT declares, `crate::head` names that declaration in rustc
    //      and can name nothing else — an explicit item shadows a glob. Driven by `globshadow` in
    //      `r186_glob_attribution_survives_where_the_crate_does_not_declare_the_name`: the crate declares
    //      its own PURE `read` beside `pub use std::fs::*`, the fixture COMPILES AND RUNS and prints 0
    //      bytes, so the pre-fix `Fs` there was a fabrication. If (1) were false it would print 65.
    //  (2) This does not narrow the rescue the branch exists for. Driven by `globrescue` in the same test:
    //      the crate declares no `read`, `crate::read` really IS the root glob's name, the fixture reads
    //      65 real bytes and the row must keep `["Fs"]`. **A review proposed deleting this branch outright
    //      for rooted paths on the argument that "rustc never resolves a `crate::`-prefixed path through a
    //      glob at all". That argument is FALSE and `globrescue` is the refutation: a `pub use x::*` at the
    //      CRATE ROOT puts those names in the root module, so `crate::<name>` does resolve to them — which
    //      is the sqlx `PgStream::connect` cardinal-sin shape. A binary built with that rule reads
    //      `pg::slurp` ABSENT over a provably real filesystem read.**
    //
    // A THIRD CLAIM IS NOT MADE, BECAUSE IT IS FALSE: this guard is NOT asked of the right scope.
    // `expand` strips `crate`/`self`/`super` in ONE loop above and then runs crate-ROOT resolution on all
    // three, so a `self::stream::X` inside `mod m` — which in rustc means `m::stream::X`, the root's item
    // not being in scope in `m` at all — is asked whether the CRATE ROOT declares `stream`. That
    // conflation is PRE-EXISTING and this guard inherits it; it is not safe, and the only thing that makes
    // it affordable is the DIRECTION it fails in, which is measured rather than argued:
    //
    //   * a BARE relative path never reaches here — the `!rooted_local` branch above returns
    //     `segs.join("::")` first — so the case a reviewer worried about (a relative path resolved to the
    //     ROOT's item where rustc takes the glob's) cannot occur;
    //   * and this branch has only TWO possible outputs, the glob prefix or the literal, so a
    //     wrong-evidence refusal can only UNDER-RESOLVE. It can never emit a third, different prefix and
    //     so can never mis-attribute.
    //
    // `r186_a_wrong_scope_refusal_under_resolves_and_never_fabricates` pins exactly that, on a fixture
    // that COMPILES AND RUNS (`rel=10 slf=10 rooted=63` — m's pure method twice, the root's real file
    // read once). **The cost of the wrong scope is real and measured, not hypothetical:**
    // `filetime`'s `unix::macos::set_times` writes `super::utimes::set_times(..)`, whose head `utimes` is
    // not a ROOT declaration, so the guard does not fire, the file's glob still eats the path and a real
    // `Fs` stays lost. Closing that needs the enclosing module's declarations rather than the root's — a
    // different change with its own audit surface, not a tightening of this one.
    //
    // WHAT IT DOES NOT COVER, stated as the limit it is: `root_decls` is read off the root file's item
    // list, so a `mod`/type hidden behind an item-position macro (`cfg_rt! { pub mod net; }`, `include!`)
    // is not in it and the glob attribution still fires there. That is the PRE-EXISTING behaviour, not a
    // new hole, and the alternative — refusing whenever the root is macro-hidden — would withdraw the
    // rescue that closed the sqlx `PgStream::connect` cardinal sin on evidence we do not have. Under-fire
    // here, never over-fire.
    // VEIN A — a path `absolutise` made crate-rooted FROM `self::`/`super::` names a module chain that IS
    // this file's own module path (from the file layout) — a real local module, never something a glob
    // re-exported into the root. Measured: aes-0.8.4's `use super::intrinsics::vaeseq_u8;` in a file
    // with `use core::arch::aarch64::*;` became `core::arch::aarch64::armv8::intrinsics::vaeseq_u8` once
    // the `use` value was anchored, because the root's `mod armv8;` sits inside `cfg_if!` and so is not
    // in `root_decls` — the R186 limit, newly reachable — and the local call edge was lost.
    if abs.is_some() && matches!(path.split("::").next(), Some("self" | "super")) {
        return r751_literal(&segs, true);
    }
    // VEIN A — a file's OWN glob puts names at the crate ROOT only when the file IS the root. In any
    // other file, `crate::connection::worker::ConnectionWorker` written beside `pub(crate) use
    // sqlx_core::connection::*;` (sqlx-sqlite) was attributed to that glob — `sqlx_core::connection::
    // connection::…` — which no rustc reading allows; it went unnoticed while every consumer read the type
    // by its leaf, and R862's dependency-type check (rightly) refused a type rooted at a dependency.
    // Where the map knows its module and that module is not the root, only the seeded ROOT glob applies.
    let file_glob = match uses.get(MODPATH_KEY) {
        Some(m) if !m.is_empty() => None,
        _ => unique_glob(uses),
    };
    match root_glob(uses).or(file_glob) {
        // R186 — THE REACH COUNTER (§E1), same shape and same reason as R160's `SELFALIAS` above: an
        // unchanged corpus row is not evidence this code ran, and "0 changed with 0 reaches" is a
        // different claim from "0 changed with 800 reaches". Gated on the cheap glob lookup having already
        // succeeded, so the env read happens only where the refusal is the thing being counted.
        Some(glob) if !root_declares(uses, segs[0]) => format!("{glob}::{}", segs.join("::")),
        Some(_) => {
            if std::env::var("CANDOR_R186_DEBUG").is_ok() {
                eprintln!("R186REFUSE {path}");
            }
            r751_literal(&segs, abs.is_some())
        }
        None => r751_literal(&segs, abs.is_some()),
    }
}

/// SOUNDNESS R751 — the rooted branch's LITERAL fallthrough, carrying the absoluteness assertion when
/// `absolutise` supplied it. One helper rather than two call sites, so the two cannot drift.
fn r751_literal(segs: &[&str], absolutised: bool) -> String {
    let joined = segs.join("::");
    if absolutised {
        format!("crate::{joined}")
    } else {
        joined
    }
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

/// SOUNDNESS R751 — the sentinel key under which THIS FILE'S OWN MODULE PATH is seeded. Same
/// escape-free argument as `GLOB_KEY`: `*` cannot appear in a Rust path segment, so it can collide with
/// no import, and `expand` matches whole `::`-separated segments so it can never be matched as one.
pub(crate) const MODPATH_KEY: &str = "*modpath";

/// SOUNDNESS R751 — seed this file's module path, so `expand` can recover the module a `self::`/`super::`
/// path is relative TO instead of discarding it.
pub(crate) fn seed_modpath(modpath: &str, uses: &mut HashMap<String, String>) {
    uses.insert(MODPATH_KEY.to_string(), modpath.to_string());
}

/// VEIN A — the sentinel key under which the TYPE-NAMESPACE names the CURRENT module declares are seeded
/// (`mod`, `struct`, `enum`, `union`, `type`, `trait`), `\u{1}`-separated. Same escape-free argument as
/// `MODPATH_KEY`. It is the second half of "which module is this path written in": `MODPATH_KEY` says
/// WHERE the module is, this says WHAT a relative path's head can name there.
///
/// It travels with `MODPATH_KEY` and is only ever read beside it. Both are REMOVED by
/// `decls::submodule_uses`, because an inline module's map is cloned from its parent's and a parent's
/// module path or declaration list in a child's map is wrong by one level — the R400 shape. A site that
/// knows the child's path re-seeds both; a site that does not leaves them absent, and every reader then
/// refuses exactly as `absolutise` refused an inline module before.
pub(crate) const MODDECL_KEY: &str = "*moddecls";

/// VEIN A — seed the TYPE-namespace names this module declares (see `MODDECL_KEY`). An item-position
/// macro invocation declares nothing candor can read, so a name it would declare is absent here and a
/// path headed by it keeps today's unanchored handling — an under-resolution, never a mis-anchoring.
pub(crate) fn seed_moddecls(items: &[syn::Item], include_tests: bool, uses: &mut HashMap<String, String>) {
    let names: Vec<String> = items
        .iter()
        .filter(|it| {
            matches!(
                it,
                syn::Item::Mod(_)
                    | syn::Item::Struct(_)
                    | syn::Item::Enum(_)
                    | syn::Item::Union(_)
                    | syn::Item::Type(_)
                    | syn::Item::Trait(_)
                    | syn::Item::TraitAlias(_)
            )
        })
        .filter_map(|it| crate::decls::declared_item_name(it, include_tests))
        .collect();
    uses.insert(MODDECL_KEY.to_string(), names.join("\u{1}"));
    // Every namespace, for the glob question: a module's own `fn read` shadows a glob's `read` exactly
    // as its own `struct File` shadows a glob's `File`.
    let all: Vec<String> =
        items.iter().filter_map(|it| crate::decls::declared_item_name(it, include_tests)).collect();
    uses.insert(MODDECL_ALL_KEY.to_string(), all.join("\u{1}"));
    // `extern crate X;` written IN this module binds `X` here to the EXTERN crate (log-0.4.20's
    // `kv/value.rs`: `extern crate value_bag; use self::value_bag::ValueBag;`) — a `self::X::…` path names
    // that crate, not a child module, and must not be anchored (`absolutise`).
    let externs: Vec<String> = items
        .iter()
        .filter(|it| matches!(it, syn::Item::ExternCrate(_)))
        .filter_map(|it| crate::decls::declared_item_name(it, include_tests))
        .collect();
    uses.insert(MODEXTERN_KEY.to_string(), externs.join("\u{1}"));
    // An item-position macro invocation can declare any name, so the module's declaration list is not
    // complete and a glob cannot be said to be the only possible origin of an undeclared name.
    let hidden = items.iter().any(|it| matches!(it, syn::Item::Macro(m) if m.mac.path.segments.last()
        .is_some_and(|s| s.ident != "macro_rules") && m.ident.is_none()));
    if hidden {
        uses.insert(MODMACRO_KEY.to_string(), String::new());
    } else {
        uses.remove(MODMACRO_KEY);
    }
}

/// VEIN A — the names this module binds with `extern crate` (see `seed_moddecls`); same lifecycle.
pub(crate) const MODEXTERN_KEY: &str = "*modextern";
/// VEIN A — every-namespace declaration list (see `seed_moddecls`); same lifecycle as `MODDECL_KEY`.
pub(crate) const MODDECL_ALL_KEY: &str = "*moddeclsall";
/// VEIN A — present when the module holds an item-position macro invocation (see `seed_moddecls`).
pub(crate) const MODMACRO_KEY: &str = "*modmacro";
/// VEIN A — EVERY glob import in scope, as written (`std::fs`, `libc`, `super`, `crate::x`), `\u{1}`-joined.
/// `GLOB_KEY` records only the external ones and is inherited by inline children for R186's crate-rooted
/// attribution; this one is the SCOPE's own, so `submodule_uses` strips it and a body's own globs are
/// appended to its module's (`decls::fninfo`).
pub(crate) const ALLGLOB_KEY: &str = "*globs";
/// VEIN A — the manifest's dependency crate names (post `-`→`_`), `\u{1}`-joined, seeded for Pass B.
pub(crate) const DEPS_KEY: &str = "*deps";

/// VEIN A — seed the manifest dependency names into a file's map (see `DEPS_KEY`).
pub(crate) fn seed_deps(deps: &std::collections::HashSet<String>, uses: &mut HashMap<String, String>) {
    let mut v: Vec<&str> = deps.iter().map(String::as_str).collect();
    v.sort_unstable();
    uses.insert(DEPS_KEY.to_string(), v.join("\u{1}"));
}

/// The std prelude's names and the primitive types: a name a glob does NOT supply resolves here, so a
/// head in this list is never attributed to a glob (an under-resolution if the glob really does shadow
/// it, which is the status quo).
const PRELUDE_AND_PRIMS: [&str; 52] = [
    "Option", "Some", "None", "Result", "Ok", "Err", "Box", "String", "Vec", "ToString", "ToOwned",
    "Clone", "Copy", "Default", "Drop", "Eq", "PartialEq", "Ord", "PartialOrd", "Iterator",
    "IntoIterator", "Extend", "FromIterator", "DoubleEndedIterator", "ExactSizeIterator", "AsRef",
    "AsMut", "From", "Into", "TryFrom", "TryInto", "Fn", "FnMut", "FnOnce", "Send", "Sync", "Sized",
    "Unpin", "drop", "bool", "char", "str", "u8", "u16", "u32", "u64", "u128", "usize", "i8", "i16",
    "i32", "i64",
];
const PRIMS_MORE: [&str; 5] = ["i128", "isize", "f32", "f64", "Self"];

/// The public names of the two std modules the classifier charges by PREFIX (`std::fs::` → `Fs`,
/// `std::env::` → `Env`). Attributing a name to one of those globs is a CHARGE, so it is taken only for a
/// name the module really exports; a name missing here keeps today's unresolved answer (an
/// under-resolution, never a fabrication). Every other std/core/alloc glob is classified by exact or
/// type-anchored rules, where a wrong attribution names nothing and charges nothing.
const STD_FS_NAMES: [&str; 29] = [
    "File", "OpenOptions", "Metadata", "DirEntry", "ReadDir", "Permissions", "FileType", "DirBuilder",
    "FileTimes", "canonicalize", "copy", "create_dir", "create_dir_all", "exists", "hard_link",
    "metadata", "read", "read_dir", "read_link", "read_to_string", "remove_dir", "remove_dir_all",
    "remove_file", "rename", "set_permissions", "soft_link", "symlink_metadata", "write", "TryLockError",
];
const STD_ENV_NAMES: [&str; 22] = [
    "args", "args_os", "current_dir", "current_exe", "home_dir", "join_paths", "remove_var",
    "set_current_dir", "set_var", "split_paths", "temp_dir", "var", "var_os", "vars", "vars_os",
    "consts", "Args", "ArgsOs", "JoinPathsError", "SplitPaths", "VarError", "Vars",
];

/// VEIN A — R193(a), R863, R830's `libc::*` cell: WHERE A NAME NO DECLARATION OR NAMED IMPORT SUPPLIES
/// CAN COME FROM.
///
/// `use std::fs::*; File::open(p)` and `use ratescore::*; carrier()` reached resolution as the bare
/// `File::open` / `carrier`, which names no rule, no local definition and no dependency, so the caller went
/// ABSENT — while the named-import spelling of the same call charged. rustc's rule decides it: a name is
/// looked up in the module's own items and explicit imports, then its GLOB imports, then the preludes. So
/// when (1) the map knows its module's declarations and they are complete (no item-position macro),
/// (2) no `use` binds the head, and (3) the head is not a dependency crate, a std root, a prelude name, a
/// primitive or a generic parameter — the name can only come from a glob in scope, and each external glob
/// is a CANDIDATE origin: `G::<path>`, byte-identical to what `use G::Head;` would have produced.
///
/// Returns the candidates and whether they are EXCLUSIVE. Exclusive means exactly one glob is in scope
/// and it is not crate-local: the candidate IS the name's origin, and the caller may rewrite the path
/// to it. Otherwise (two external globs; a crate-local glob such as `use super::*` beside an external
/// one) rustc still resolves the name to exactly ONE of them — a name two globs both supply is E0659 —
/// but which is not decidable here, so the caller keeps the written path for local resolution and ADDS
/// each candidate beside it. A candidate the glob does not really export joins nothing and classifies
/// as nothing, except under the two std modules the classifier charges by PREFIX, which take a name only
/// from their real export list.
///
/// `is_generic` is the caller's own knowledge of the generic parameters in scope (a `T::new()` head).
pub(crate) fn glob_candidates(
    segs: &[&str],
    uses: &HashMap<String, String>,
    is_generic: &dyn Fn(&str) -> bool,
) -> Option<(Vec<String>, bool)> {
    let head = *segs.first()?;
    let all = uses.get(MODDECL_ALL_KEY)?;
    // SOUNDNESS R989 — an item-position macro may declare `head` locally, so the module's declaration list
    // is not complete and a glob cannot be the EXCLUSIVE origin. It is still a CANDIDATE origin: refusing
    // outright left security-framework's `SecKeychainCreate(..)` (from `use security_framework_sys::
    // keychain::*;`, beside `declare_TCFType!`) with no edge and no disclosure, while the qualified
    // spelling disclosed. Non-exclusive candidates are ADDED beside the written path, never instead of it.
    let macro_hidden = uses.contains_key(MODMACRO_KEY);
    if all.split('\u{1}').any(|n| n == head) || uses.contains_key(head) {
        return None;
    }
    if matches!(head, "std" | "core" | "alloc" | "proc_macro" | "crate" | "self" | "super")
        || PRELUDE_AND_PRIMS.contains(&head)
        || PRIMS_MORE.contains(&head)
        || is_generic(head)
        || uses.get(DEPS_KEY).is_some_and(|d| d.split('\u{1}').any(|n| n == head))
    {
        return None;
    }
    let globs = uses.get(ALLGLOB_KEY)?;
    let mut local = 0usize;
    let mut external: Vec<String> = Vec::new();
    for g in globs.split('\u{1}').filter(|g| !g.is_empty()) {
        let groot = g.split("::").next().unwrap_or(g);
        if matches!(groot, "crate" | "self" | "super")
            || module_declares(uses, groot) == Some(true)
            || uses.get(groot).is_some_and(|v| {
                v.starts_with("crate::") || v.starts_with("self::") || v.starts_with("super::")
            })
        {
            local += 1; // a crate-local glob: its names are local, and `by_leaf`/`by_tail2` own them
            continue;
        }
        // A glob whose head is bound by a `use` (`use x::prelude; use prelude::*;`) is spelled through it.
        let g = match uses.get(groot) {
            Some(v) if !v.is_empty() => alias_join(v, &g.split("::").skip(1).collect::<Vec<_>>()),
            _ => g.to_string(),
        };
        if g.contains(crate::decls::ALIAS_ALT_SEP) {
            continue;
        }
        let charged = match g.as_str() {
            "std::fs" => Some(&STD_FS_NAMES[..]),
            "std::env" => Some(&STD_ENV_NAMES[..]),
            _ => None,
        };
        if charged.is_some_and(|names| !names.contains(&head)) {
            local += 1; // not this glob's name: it cannot be the origin, and it is not exclusive either
            continue;
        }
        if !external.contains(&g) {
            external.push(g);
        }
    }
    if external.is_empty() {
        return None;
    }
    let exclusive = external.len() == 1 && local == 0 && !macro_hidden;
    let cands: Vec<String> = external.iter().map(|g| format!("{g}::{}", segs.join("::"))).collect();
    if std::env::var("CANDOR_R186_DEBUG").is_ok() {
        eprintln!("VEINAGLOB {} -> {} exclusive={exclusive}", segs.join("::"), cands.join(" | ")); // §E1
    }
    Some((cands, exclusive))
}


/// VEIN A — does the module this map describes declare `name` in the TYPE namespace? `None` when the
/// map carries no declaration list (a pass that does not know which module it is in), which every
/// caller reads as "cannot say" and answers exactly as before this existed.
pub(crate) fn module_declares(uses: &HashMap<String, String>, name: &str) -> Option<bool> {
    uses.get(MODDECL_KEY).map(|list| list.split('\u{1}').any(|n| n == name))
}

/// SOUNDNESS R751 — **THE CONFLATION ITSELF, NOT A THIRD GUARD OVER ITS SYMPTOMS.**
///
/// `expand` strips `crate`/`self`/`super` in ONE loop and then resolves all three as if crate-rooted, so
/// the module a RELATIVE path is relative to is thrown away. Three separate rules were added over that
/// loss before anyone said what the loss was; this recovers the fact instead. `self::X` in module M is
/// `M::X`; each `super::` pops one segment off M.
///
/// **THE SENTENCE THAT DECIDES THIS WHOLE AREA, and neither R186's row nor the brief that commissioned
/// the alternative had it: A GUARD CAN SUPPRESS BUT IT CANNOT SUPPLY.** The remedy proposed for this was a
/// per-MODULE version of `root_decls` — ask whether the path's own parent declares the head, rather than
/// whether the crate root does. Measured on x11rb, that is WORSE than doing nothing: `protocol::present`
/// really does declare `pixmap`, so the guard fires on `self::pixmap(..)`, correctly suppresses the glob —
/// and the glob was the only thing SUPPLYING the module qualifier that stripping `self::` had discarded.
/// The path collapses to the one-segment `pixmap`, whose LEAF is claimed by both
/// `protocol::present::pixmap` and `protocol::xproto::PixmapWrapper::pixmap`, so it is refused and a real
/// `Log` is lost. A guard over this branch can only ever withhold an attribution; nothing it can do
/// produces a module name. That is why the fix is here and not there.
///
/// **IT EMITS A `crate::`-ROOTED PATH ON PURPOSE, AND THAT IS THE WHOLE PLUMBING.** A surviving `crate::`
/// head already means "absolute from the crate root" everywhere downstream, so returning one turns what
/// was a PROXY into the assertion itself — `arm_exact_target`'s exact-qual preference was gated on
/// `starts_with("crate::")` precisely as a stand-in for absoluteness, and now that gate is reading the
/// real thing. Three readers consult that head and all three want exactly root-absoluteness, so none has
/// to be taught a second spelling and none can drift from the others:
///   * `arm_exact_target` — the exact-qual preference (and `bare`);
///   * `macro_hidden_owner` — R128's "which module owns this path", whose own note recorded the
///     relative spellings as a STATED under-report; they are now answerable, so that hedge reaches them;
///   * `scan.rs`'s R270 module-relative `by_leaf` check, which strips the head to get the target.
///
/// **DIRECTION — AND IT IS A WEAKER BOUND THAN THE GUARD-ONLY CHANGES, WHICH MUST NEVER BE QUOTED FOR IT.**
/// R186's guard could only ever UNDER-resolve, because its branch emits the glob prefix or the literal and
/// nothing else (`r186_the_rooted_glob_branch_emits_only_the_glob_prefix_or_the_literal` pins that). This
/// REWRITES the path, so that argument does not carry. The bound that does:
///
/// > `module_path()` does not have to agree with rustc. It has to agree with the ENGINE'S OWN QUAL SPACE,
/// > and it does BY CONSTRUCTION, because the quals this scan mints and the `modpath` seeded here both
/// > come from that one function.
///
/// So a `#[path = "elsewhere.rs"]` module whose real path is `real::inner` is called `elsewhere` by both,
/// and `self::sibling` resolves to `elsewhere::sibling` — the qual the scan actually emitted. Pinned by
/// `r751_a_path_attribute_module_stays_consistent_with_the_engines_own_quals`.
///
/// **REFUSED INSIDE AN INLINE MODULE, and that refusal is load-bearing rather than tidy.** `modpath` is
/// the FILE's, while an inline `mod inner { … super::X … }` sits one level deeper, so using it there is
/// wrong by exactly one level — the R400 shape this file already records a REVERT for. `submodule_uses`
/// plants `SUPER_SCOPE_MARKER` for precisely this question and `rebound` already consults it; so does
/// this, and `r751_an_inline_module_is_refused_rather_than_answered_one_level_off` drives it.
fn absolutise(segs: &[&str], uses: &HashMap<String, String>) -> Option<String> {
    // VEIN A — THE INLINE-MODULE REFUSAL IS GONE BECAUSE ITS PREMISE IS. It refused because `modpath`
    // was the FILE's inside an inline `mod`, one level stale. `decls::submodule_uses` now REMOVES
    // `MODPATH_KEY` from every child map, and only a site that knows the child's own path
    // (`scan_items`, `collect_reexports`, `typesurf::walk_items`) puts it back — so a `modpath` present
    // here is the CURRENT module's, inline or file, and an absent one refuses below exactly as before.
    let modpath = uses.get(MODPATH_KEY)?;
    let mut ups = 0usize;
    let mut i = 0usize;
    while let Some(h) = segs.get(i) {
        match *h {
            "self" => {}
            "super" => ups += 1,
            // VEIN A — an explicitly written `crate::` head IS absolute, and stripping it (what `expand`
            // did) made `crate::b::Rx::grab` byte-identical to a module-RELATIVE `b::Rx::grab`, so
            // `arm_exact_target`'s exact-qual preference — which is licensed by a surviving `crate::`
            // head and nothing else — could not run on the one spelling that most plainly names its
            // target. R830's `z::g_crate` was exactly that: the root's `b::Rx::grab` (the exact qual)
            // tied with `z::b::Rx::grab` (a suffix) and the caller went ABSENT. Returned as the path
            // after the head; the root-one-segment rule below still applies to it.
            "crate" if i == 0 => {
                let rest = &segs[1..];
                if rest.is_empty() || rest.len() == 1 || root_extern_crate(uses, rest[0]) {
                    return None;
                }
                return Some(rest.join("::"));
            }
            "crate" => return None,
            // The first non-prefix segment ENDS the prefix run — `break`, never `return`. Writing
            // `return None` here made the whole function inert (`self::pixmap` bails on `pixmap`), and it
            // was caught by the §E1 reach counter reading 0 where the prototype read 62 on the same crate,
            // not by any test: every fixture simply kept its pre-fix answer, which is what an inert change
            // and a safe one look like from the outside.
            _ => break,
        }
        i += 1;
    }
    if i == 0 {
        return None; // not a relative rooted path
    }
    let mut base: Vec<&str> =
        if modpath.is_empty() { Vec::new() } else { modpath.split("::").collect() };
    for _ in 0..ups {
        // `?` rather than an `if`: walking above the crate root refuses rather than guessing. Written as
        // the explicit `if` first, which `cargo +stable clippy` rejects (`question_mark`) while the pinned
        // nightly accepts — the third time this session the two toolchains have disagreed.
        base.pop()?;
    }
    let rest = &segs[i..];
    if rest.is_empty() {
        return None;
    }
    // `self::X` where this module says `extern crate X;` names the extern crate (see `MODEXTERN_KEY`).
    if ups == 0
        && uses.get(MODEXTERN_KEY).is_some_and(|l| l.split('\u{1}').any(|n| n == rest[0]))
    {
        return None;
    }
    // A target that lands at the CRATE ROOT with a ONE-segment name gets no prefix, because the prefix
    // would break the very key it has to be looked up by: `tail2("crate::pick")` is `crate::pick`, which
    // no definition qual can ever equal, so the path would resolve to nothing at all. A root free fn has
    // a one-segment qual and is reached through `by_leaf` — `scan.rs`'s R270 note records exactly that
    // shape. Emitting the bare leaf here leaves it on that route, which is what it was on before, so this
    // case is a NO-OP against HEAD rather than an improvement. Found by an inline-module test that kept
    // passing with its own subject disabled.
    if base.is_empty() && rest.len() == 1 {
        return None;
    }
    base.extend_from_slice(rest);
    Some(base.join("::"))
}

/// SOUNDNESS R186 — the sentinel key under which the CRATE ROOT's own declared item names are seeded
/// into every file's `use` map, as a `\u{1}`-separated list (`seed_root_decls`). Same escape-free
/// reasoning as `GLOB_KEY`: `*` cannot appear in a Rust path segment, so the key can collide with no
/// real import, and the seeded form is `crate::`-prefixed so no bare lookup can reach it either.
pub(crate) const ROOT_DECL_KEY: &str = "*rootdecls";

/// SOUNDNESS R186 — does the crate ROOT declare an item called `name`? Consulted before `expand`
/// attributes a `crate::`-rooted path to a re-export glob; see the call site for why a positive answer
/// makes the attribution provably wrong rather than merely doubtful.
pub(crate) fn root_declares(uses: &HashMap<String, String>, name: &str) -> bool {
    uses.get(&format!("crate::{ROOT_DECL_KEY}"))
        .is_some_and(|list| list.split('\u{1}').any(|n| n == name))
}

/// SOUNDNESS R186 — seed the crate ROOT's declared item names into ONE file's `use` map under
/// `crate::` + `ROOT_DECL_KEY`. Crate-rooted key only, for the reason `seed_root_reexports` states: a
/// bare lookup must never reach a crate-wide fact.
pub(crate) fn seed_root_decls(names: &std::collections::BTreeSet<String>, uses: &mut HashMap<String, String>) {
    if names.is_empty() {
        return;
    }
    let joined = names.iter().cloned().collect::<Vec<_>>().join("\u{1}");
    uses.insert(format!("crate::{ROOT_DECL_KEY}"), joined);
}

/// SOUNDNESS R186 — the item names the crate ROOT declares ITSELF (`mod net;`, `struct Stream`, `fn f`),
/// read from the root file's TOP-LEVEL item list only, because `crate::<head>` resolves at the root and
/// nowhere else. Uses `decls::declared_item_name`, the one authority for "does this item bind a name",
/// so a `#[cfg(test)]` item is included exactly when the scan includes tests.
pub(crate) fn collect_root_decls(
    items: &[syn::Item],
    include_tests: bool,
) -> std::collections::BTreeSet<String> {
    let mut out: std::collections::BTreeSet<String> = items
        .iter()
        .filter_map(|it| crate::decls::declared_item_name(it, include_tests))
        .collect();
    // VEIN A — and, MARKED, the names the root binds with `extern crate` (`extern crate alloc;`): a
    // written `crate::alloc::…` names that EXTERN crate, not a module of this one, so `absolutise` must
    // not anchor it (see `ROOT_EXTERN_MARK`). The mark keeps these out of `root_declares`' plain-name test.
    for it in items {
        if let syn::Item::ExternCrate(e) = it {
            if let Some(name) = crate::decls::declared_item_name(it, include_tests) {
                let _ = e;
                out.insert(format!("{ROOT_EXTERN_MARK}{name}"));
            }
        }
    }
    // VEIN A — and a root `use` that binds a name to ANOTHER CRATE (`pub use bson3 as bson;`, mongodb,
    // `#[cfg]`-duplicated with `bson2`): `crate::bson::RawDocumentBuf` is that crate's type exactly as an
    // `extern crate` binding's would be. Measured: anchored as a local path it named no body and no
    // dependency, so the typed `dispatch:RawDocumentBuf.append` hedge — and, chained, the join — vanished
    // with nothing in its place (35 mongodb rows C3). Only a single-ident target whose head is not a
    // root-declared name and not `crate`/`self`/`super` qualifies; a path through a local module does not.
    let declared: std::collections::BTreeSet<String> =
        out.iter().filter(|n| !n.starts_with(ROOT_EXTERN_MARK)).cloned().collect();
    for it in items {
        let syn::Item::Use(u) = it else { continue };
        if !include_tests && is_cfg_test(&u.attrs) {
            continue;
        }
        let (target, bound) = match &u.tree {
            syn::UseTree::Rename(r) => (r.ident.to_string(), r.rename.to_string()),
            syn::UseTree::Name(n) => (n.ident.to_string(), n.ident.to_string()),
            _ => continue,
        };
        if matches!(target.as_str(), "crate" | "self" | "super") || declared.contains(&target) {
            continue;
        }
        out.insert(format!("{ROOT_EXTERN_MARK}{bound}"));
    }
    out
}

/// VEIN A — the prefix `collect_root_decls` marks an `extern crate` binding with. `\u{2}` cannot appear in
/// an identifier, so a marked entry never equals a plain declared name.
pub(crate) const ROOT_EXTERN_MARK: &str = "\u{2}";

/// VEIN A — does the crate root bind `name` with `extern crate`? Measured: hashbrown's `extern crate
/// alloc;` makes `crate::alloc::alloc::Layout` the STD `Layout`; anchored as a crate path it read as a
/// local type named `Layout` — harmless there, a fabricated `Drop` wherever a local `Layout` exists.
pub(crate) fn root_extern_crate(uses: &HashMap<String, String>, name: &str) -> bool {
    uses.get(&format!("crate::{ROOT_DECL_KEY}"))
        .is_some_and(|list| list.split('\u{1}').any(|n| n.strip_prefix(ROOT_EXTERN_MARK) == Some(name)))
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

/// SOUNDNESS R334 — does any ARGUMENT of this call name the OS entropy source as a VALUE?
///
/// The counterpart to `positional_str_lit` above, and deliberately its opposite in one respect: that
/// helper reads a KNOWN position because a literal anywhere in the list was the wrong answer for a
/// security gate. Here the whole list is read, because the entropy source has no fixed position —
/// `StdRng::try_from_rng(&mut SysRng)` puts it at 0 and `ReseedingRng::new(1024, OsRng)` at 1 — and
/// unlike a path literal, its presence ANYWHERE in the arguments is the fact worth recording. The
/// hazard `positional_str_lit` exists to stop does not apply: no argument here can be mistaken for a
/// different argument's meaning, since the only thing being asked is "is the OS RNG in this list".
///
/// Recursive on purpose: the source arrives wrapped. `&mut SysRng` is a `Reference`, `&mut *r` adds a
/// `Unary`, a fully-qualified `rand::rngs::OsRng` is a longer `Path`, and `SysRng::default()` is a
/// `Call` whose own callee path names it. A match on the bare `Expr::Path` arm alone would see the
/// first of those and miss the rest — which is the shape of defect this register keeps recording as
/// "correct code on a path the failing case never takes".
pub(crate) fn args_name_entropy_source(
    args: &syn::punctuated::Punctuated<syn::Expr, syn::token::Comma>,
) -> bool {
    struct Seek(bool);
    impl<'ast> syn::visit::Visit<'ast> for Seek {
        fn visit_path(&mut self, p: &'ast syn::Path) {
            if p.segments.iter().any(|seg| {
                candor_classify::is_entropy_source_ident(&seg.ident.to_string())
            }) {
                self.0 = true;
            }
            syn::visit::visit_path(self, p);
        }
    }
    let mut seek = Seek(false);
    for a in args {
        syn::visit::Visit::visit_expr(&mut seek, a);
        if seek.0 {
            // §E1 HIT COUNTER (`CANDOR_ALIAS_DEBUG`) for `bin/corpus-ab.py --mark`.
            if std::env::var("CANDOR_ALIAS_DEBUG").is_ok() {
                eprintln!("R334ENTROPYARG");
            }
            return true;
        }
    }
    false
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

/// SOUNDNESS R535 — THE VALUE POSITIONS OF A CONTROL-FLOW MERGE, as ONE authority for every resolver
/// that asks what an expression evaluates to.
///
/// A receiver spelled `(if c { a } else { b })`, `(match c { .. })`, `{ a }` or `unsafe { a }` matched
/// no arm in EITHER receiver resolver, so it resolved to nothing and its caller read ABSENT from
/// `functions[]` — a §4 purity claim over a body that spawns a process, with `deny Net` and `pure` both
/// exiting 0 on the minimal case. That is §F1 question 1 at the receiver position: the answer was read
/// from syntactic ADJACENCY (a `match` over expression shapes) where the question is a control-flow
/// MERGE. It lives here, not inside one resolver, because four resolvers ask this same question about
/// the same expression and this family's bugs recur wherever one question has two implementations
/// (R347, R380, R446, §G).
///
/// WHAT THE CALLER DOES WITH THE LIST IS THE CALLER'S RULE, and the two differ on purpose: in
/// well-typed Rust every branch of an `if`/`match` has ONE type, so a CONCRETE resolver may take any
/// branch that answers and must DECLINE when two answer differently (its resolver is then wrong about
/// at least one of them); a DISPATCH resolver unions the branches' leaves, the direction bounded CHA
/// already over-approximates in.
///
/// TWO REFUSALS, both about NAME SHADOWING, because the resolvers read `vars`/`trait_vars` by BARE NAME
/// and a branch can rebind one:
///   · a block containing a `let` contributes nothing — `({ let a = P; a }).go()` must not resolve the
///     OUTER `a`. (R101's arm in `resolve_elem_trait_leaves` refused any block with statements at all
///     for this reason; this is the same refusal keyed on the statement that can actually shadow.)
///   · a `match` arm whose pattern BINDS a name contributes nothing, and an `if let`'s then-branch
///     likewise — its `else` still counts, which is what makes `(if let Some(x) = o { x } else { b })`
///     resolve from `b` rather than guess.
/// `None` for everything else, including a LABELLED block and a `loop { break v }` — their value can
/// arrive at a `break`, which this does not walk. Measured ABSENT and left so, deliberately.
pub(crate) fn merge_value_exprs(expr: &syn::Expr) -> Option<Vec<&syn::Expr>> {
    match expr {
        syn::Expr::If(i) => {
            let mut out: Vec<&syn::Expr> = Vec::new();
            if !matches!(&*i.cond, syn::Expr::Let(_)) {
                if let Some(e) = block_tail_value(&i.then_branch) {
                    out.push(e);
                }
            }
            // The `else` is itself an expression — a block, or the next `if` of a chain. Handed back
            // whole, so the caller's own recursion walks it through THIS function again.
            if let Some((_, e)) = &i.else_branch {
                out.push(e);
            }
            (!out.is_empty()).then_some(out)
        }
        syn::Expr::Match(m) => {
            let out: Vec<&syn::Expr> = m
                .arms
                .iter()
                .filter(|a| !pat_binds_ident(&a.pat))
                .map(|a| &*a.body)
                .collect();
            (!out.is_empty()).then_some(out)
        }
        // A LABELLED block is excluded: `'l: { break 'l v; }` carries its value to the `break`.
        syn::Expr::Block(b) if b.label.is_none() => block_tail_value(&b.block).map(|e| vec![e]),
        syn::Expr::Unsafe(u) => block_tail_value(&u.block).map(|e| vec![e]),
        _ => None,
    }
}

/// The expression a BLOCK evaluates to — its trailing expression with no semicolon — but only when no
/// statement in it is a `let`. See `merge_value_exprs` for why the `let` is the disqualifier: the
/// resolvers key on bare names, so a block-local binding would be read through as the outer one.
fn block_tail_value(b: &syn::Block) -> Option<&syn::Expr> {
    if b.stmts.iter().any(|s| matches!(s, syn::Stmt::Local(_))) {
        return None;
    }
    match b.stmts.last() {
        Some(syn::Stmt::Expr(e, None)) => Some(e),
        _ => None,
    }
}

/// Does this pattern bind ANY identifier? The shadow test `merge_value_exprs` uses on a `match` arm.
/// Deliberately coarse — a pattern that binds a name it does not use still disqualifies the arm — and
/// coarse in the direction that declines rather than resolves.
///
/// EXCEPT FOR ONE SHAPE, AND IT IS THE COMMON ONE. **syn parses a bare `None` as `Pat::Ident`**, not as
/// `Pat::Path` — measured, not assumed — so a first draft that answered `true` for every `Pat::Ident`
/// disqualified the `None` arm of `(match o { Some(x) => x, None => b }).go()` and left that receiver
/// resolving to nothing, which is the single most ordinary spelling of an optional receiver in Rust.
/// An UPPER-INITIAL ident with no subpattern and no `ref`/`mut` is a unit variant or a const, never a
/// binding (a binding spelled that way is `non_snake_case` and warns), so it cannot shadow anything.
/// This is the same Upper-initial convention `resolve_recv_type`'s unit-struct arm already uses. A
/// SCREAMING_SNAKE const is still treated as a binding here, which only declines.
fn pat_binds_ident(pat: &syn::Pat) -> bool {
    use syn::Pat;
    match pat {
        Pat::Ident(id) => {
            let unit_variant_or_const = id.subpat.is_none()
                && id.by_ref.is_none()
                && id.mutability.is_none()
                && id.ident.to_string().chars().next().is_some_and(|c| c.is_uppercase());
            !unit_variant_or_const
        }
        Pat::Reference(r) => pat_binds_ident(&r.pat),
        Pat::Paren(p) => pat_binds_ident(&p.pat),
        Pat::Type(t) => pat_binds_ident(&t.pat),
        Pat::Or(o) => o.cases.iter().any(pat_binds_ident),
        Pat::Tuple(t) => t.elems.iter().any(pat_binds_ident),
        Pat::TupleStruct(t) => t.elems.iter().any(pat_binds_ident),
        Pat::Slice(s) => s.elems.iter().any(pat_binds_ident),
        Pat::Struct(s) => !s.fields.is_empty(),
        _ => false,
    }
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
/// VEIN A — the ENUM segment a single-field tuple-variant pattern names (`Self` in `Self::Cmd(x)`, `E4`
/// in `E4::Cmd(x)`, the enum in `m::E4::Cmd(x)`); `None` for a bare `Cmd(x)`. Same peeling as
/// `tuple_variant_binding`.
pub(crate) fn tuple_variant_enum(pat: &syn::Pat) -> Option<String> {
    match pat {
        syn::Pat::Reference(r) => tuple_variant_enum(&r.pat),
        syn::Pat::Paren(p) => tuple_variant_enum(&p.pat),
        syn::Pat::TupleStruct(ts) if ts.elems.len() == 1 => {
            let n = ts.path.segments.len();
            (n >= 2).then(|| ts.path.segments[n - 2].ident.to_string())
        }
        _ => None,
    }
}

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

/// SOUNDNESS R946 — whether `pat` is std's `Some(x)`/`Ok(x)`: bare, or qualified through `Option`/`Result`
/// (`Option::Some`, `std::result::Result::Ok`). A LOCAL enum's variant that happens to be named `Some`/`Ok`
/// (`ArchivedRcWeak::Some(r)`) is not, and its payload type is the variant's, not the scrutinee's.
pub(crate) fn std_some_ok_pat(pat: &syn::Pat) -> bool {
    match pat {
        syn::Pat::Reference(r) => std_some_ok_pat(&r.pat),
        syn::Pat::Paren(p) => std_some_ok_pat(&p.pat),
        // An or-pattern of payload variants (`Bound::Included(v) | Bound::Excluded(v)`).
        syn::Pat::Or(o) => !o.cases.is_empty() && o.cases.iter().all(std_some_ok_pat),
        syn::Pat::TupleStruct(ts) => {
            let segs: Vec<String> = ts.path.segments.iter().map(|s| s.ident.to_string()).collect();
            match segs.as_slice() {
                [v] => v == "Some" || v == "Ok",
                [.., o, v] => {
                    let head_ok = segs[..segs.len() - 2]
                        .iter()
                        .all(|s| matches!(s.as_str(), "std" | "core" | "option" | "result" | "ops"));
                    head_ok
                        && ((o == "Option" && v == "Some")
                            || (o == "Result" && v == "Ok")
                            // R1034 residue — `std::ops::Bound`'s two payload variants, spelled through
                            // the enum (bare `Included` names nothing std exports).
                            || (o == "Bound" && (v == "Included" || v == "Excluded")))
                }
                _ => false,
            }
        }
        _ => false,
    }
}

/// The single-ident binding of a `Some(x)` / `Ok(x)` pattern (the payload of an `if let`/`let-else`
/// unwrap of an `Option`/`Result`) — so `if let Some(d) = o { d.go() }` over an `Option<Box<dyn T>>`
/// types `d` for dispatch. `None` for any other pattern (a `None`/`Err` arm, a multi-field or
/// non-single-ident payload — an honest under-report). Peels reference/paren wrappers.
pub(crate) fn some_ok_binding(pat: &syn::Pat) -> Option<String> {
    // SOUNDNESS R946 (second fixture) — std's `Some`/`Ok` ONLY. This matched the LAST SEGMENT, so a local
    // enum's `W::Some(r)` was claimed as an Option payload and never reached the R77 enum-variant route:
    // `if let W::Some(r) = self { r.go() }` read the caller ABSENT (measured, before this change too), and
    // with R946's construction fallback `match self { W::Some(r) => r.go() }` typed `r` as `W` itself.
    if !std_some_ok_pat(pat) {
        return None;
    }
    match pat {
        syn::Pat::Reference(r) => some_ok_binding(&r.pat),
        syn::Pat::Paren(p) => some_ok_binding(&p.pat),
        syn::Pat::TupleStruct(ts) if ts.elems.len() == 1 => {
            let variant = ts.path.segments.last()?.ident.to_string();
            if matches!(variant.as_str(), "Some" | "Ok" | "Included" | "Excluded") {
                single_pat_ident(ts.elems.first()?)
            } else {
                None
            }
        }
        // Every case binds the SAME single name, or none is taken.
        syn::Pat::Or(o) => {
            let mut names = o.cases.iter().map(some_ok_binding);
            let first = names.next()??;
            names.all(|n| n.as_deref() == Some(first.as_str())).then_some(first)
        }
        _ => None,
    }
}

/// True if the item carries any `#[cfg(...)]` attribute (conditionally compiled).
pub(crate) fn has_cfg(attrs: &[syn::Attribute]) -> bool {
    attrs.iter().any(|a| a.path().is_ident("cfg"))
}

/// True if a file stem names a CONVENTIONAL test-module file (`tests.rs`, `foo_tests.rs`, `foo_test.rs`).
///
/// R457 — this is a CANDIDATE filter, not a verdict. It used to be the whole rule, and `regex-cli`'s
/// `cmd/compile_test.rs` (the real source of that binary) was dropped for matching it. The verdict is
/// `decls::test_stem_file_is_test_module`, which goes and reads the declaring `mod`. Measured over 1,608
/// registry crates: 365 files match this stem and 17 of them are production source.
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

/// SOUNDNESS R167 — a fn carrying a `#[test]`-FAMILY attribute is test-only, and this is not a policy
/// choice: **the function does not exist in a non-test build.** Measured with rustc, not argued — a
/// `#[test] pub fn bare_test_fn()` called from `main` in the same crate fails to compile with
/// `E0425: cannot find function 'bare_test_fn' in this scope`, and `#[bench]` fails the same way. The
/// builtin `test` macro strips the item outside `--test`, so nothing a consumer builds can reach its
/// effects. `is_cfg_test` answers only for `#[cfg(test)]`, so a `#[test] fn` written directly at module
/// scope — sqlx-postgres does this in `options/parse.rs` — was scanned as PRODUCTION and its
/// `['Env','Fs','Log']` FABRICATED into the crate's report (46 of R160's 2,621 added rows were this).
///
/// The match is deliberately TIGHT — the LAST path segment being exactly `test` or `bench` — because the
/// two error directions do not cost the same: excluding a PRODUCTION fn is a silent under-report, the
/// cardinal sin, while failing to exclude a harness fn is only the fabrication this closes. So `#[test]`,
/// `#[bench]`, `#[tokio::test]`, `#[async_std::test]` and `#[actix_rt::test]` are matched — all of them
/// expand to the builtin `test` and are stripped identically — and `#[test_case(..)]`, `#[rstest]`,
/// `#[quickcheck]` and every other third-party harness spelling is NOT: those keep their current
/// over-charge rather than risk the other direction. `#[should_panic]` needs no arm of its own; it only
/// ever accompanies `#[test]`.
///
/// Applied at the fn-emitting sites of `scan_items` AND the matching sites of `fn_locs`, which consume
/// positions in LOCKSTEP — one predicate, both walks, for the same reason `is_cfg_test` is one function
/// rather than two conditions (§G: the two walks must not answer one question two ways).
/// SOUNDNESS R167 — is this source file a NON-LIBRARY TARGET (`tests/`, `benches/`, `examples/`)?
///
/// This is the exact SCOPE of `is_test_attr_fn`'s justification, and it is load-bearing rather than
/// tidy. `is_test_attr_fn` is sound because rustc STRIPS a `#[test]` item outside `--test` — true of a
/// LIBRARY file, and FALSE of these three, which are only ever compiled as a test/bench/example binary,
/// harness included. Applying the skip to them anyway produced a gate flip in the cardinal direction,
/// measured on a two-file fixture and caught by this repo's own
/// `peek_scope_attribution_reaches_the_dispatching_caller_and_never_double_reports`: over a crate whose
/// `tests/it.rs` holds `#[test] fn direct_net() { TcpStream::connect("evil.example.com:80") }`,
/// `deny Net` went from exit **2** with the function named in `outOfScope` to exit **0** with
/// `outOfScope: []` — a clean pass over an integration test that really opens a socket. The ⟨0.29⟩ peek
/// exists precisely to look INTO excluded targets, so a rule premised on the ordinary build must not
/// reach it.
///
/// Keyed on the FIRST segment, taken from the file's path relative to the scan root — one authority, fed
/// from `rel` at both the `fn_locs` and `scan_items` call sites, because those two walks are consumed in
/// lockstep and a predicate they could answer differently would desynchronise every later `loc`.
///
/// The known FALSE NEGATIVE, named rather than folded in: a LIBRARY module that happens to be called
/// `tests` (`src/tests.rs`, or `src/tests/`) is read as non-library here, so a `#[test] fn` inside it
/// keeps its (fabricated) charge. That is the over-report direction. R457 is the standing warning about
/// deciding test-ness from a filename; this use of the filename can only ever FAIL TO EXCLUDE, never
/// exclude something real, which is the direction that warning asks for.
pub(crate) fn is_nonlib_target_file(rel: &str) -> bool {
    matches!(
        rel.split(['/', '\\']).next(),
        Some("tests" | "benches" | "examples")
    )
}

pub(crate) fn is_test_attr_fn(attrs: &[syn::Attribute]) -> bool {
    attrs.iter().any(|a| {
        matches!(
            a.path().segments.last().map(|s| s.ident.to_string()).as_deref(),
            Some("test" | "bench")
        )
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

/// SOUNDNESS R1056 — the `{key}::{method}` tails of the crate's impls on a SLICE / ARRAY / TUPLE self type
/// (`[u8]::enc`, `[u8;_]::ar`, `(u8,u8)::pr`), installed once per `scan_one` before Pass B, which reads it
/// to type a receiver the collector otherwise leaves untyped (see `CallCollector::nonpath_recv_calls`).
/// Crate-wide on purpose, the same way `CFG_FEATURES` is: it is read deep inside the collector, and the
/// decl-index digest already folds `nonpath_receivers` in, so a warm Pass B cannot replay a stale answer.
pub(crate) static NONPATH_RECV_TAILS: std::sync::OnceLock<std::sync::RwLock<std::collections::HashSet<String>>> =
    std::sync::OnceLock::new();

pub(crate) fn nonpath_recv_cell() -> &'static std::sync::RwLock<std::collections::HashSet<String>> {
    NONPATH_RECV_TAILS.get_or_init(|| std::sync::RwLock::new(Default::default()))
}

/// SOUNDNESS R1056 (residual) — the std wrapper a typed receiver AUTODEREFS through to a non-path impl's
/// referent: `Vec<T>` → `[T]`, `String` → `str`. `None` for anything else, and for a bare `Vec` / `String`
/// the scope rebinds (`use x::Vec`), which is not std's.
pub(crate) fn std_deref_owner(ty: &str, uses: &HashMap<String, String>) -> Option<&'static str> {
    match ty {
        "Vec" if !uses.contains_key("Vec") => Some("Vec"),
        "String" if !uses.contains_key("String") => Some("String"),
        "std::vec::Vec" | "alloc::vec::Vec" => Some("Vec"),
        "std::string::String" | "alloc::string::String" => Some("String"),
        _ => None,
    }
}

/// SOUNDNESS R1056 (residual) — the `NONPATH_RECV_TAILS` entry recording that this crate's own `impl Tr for
/// Vec<..>` / `for String` writes member `m` (`*`: a block whose members cannot be read). Such a member
/// wins method lookup at the wrapper's own step, before the deref to `[T]` / `str` is tried. `\u{1}` cannot
/// begin a `{key}::{method}` tail, so the two kinds of entry never collide.
pub(crate) fn deref_owner_marker(owner: &str, m: &str) -> String {
    format!("\u{1}{owner}::{m}")
}

/// SOUNDNESS R1056 (residual) — what a `Vec<T>` receiver finds at its OWN autoderef steps (`Vec<T>`,
/// `&Vec<T>`, `&mut Vec<T>`) before it reaches `[T]`: `Vec`'s inherent methods and the methods of the std
/// traits `Vec` implements. A DENYLIST over a resolution (see `CallCollector::nonpath_recv_calls`).
pub(crate) const VEC_SURFACE: &[&str] = &[
    "new", "with_capacity", "capacity", "reserve", "reserve_exact", "try_reserve", "try_reserve_exact",
    "shrink_to_fit", "shrink_to", "into_boxed_slice", "truncate", "as_slice", "as_mut_slice", "as_ptr",
    "as_mut_ptr", "set_len", "swap_remove", "insert", "remove", "retain", "retain_mut", "dedup_by_key",
    "dedup_by", "dedup", "push", "pop", "pop_if", "append", "drain", "clear", "len", "is_empty", "split_off",
    "resize_with", "resize", "leak", "spare_capacity_mut", "extend_from_slice", "extend_from_within",
    "splice", "extract_if", "into_raw_parts", "allocator", "push_within_capacity", "into_flattened",
    "into_parts", "as_non_null",
    // std traits implemented for `Vec<T>`
    "clone", "clone_from", "extend", "extend_one", "extend_reserve", "into_iter", "deref", "deref_mut",
    "as_ref", "as_mut", "borrow", "borrow_mut", "hash", "eq", "ne", "cmp", "partial_cmp", "lt", "le", "gt",
    "ge", "max", "min", "clamp", "fmt", "index", "index_mut", "drop", "into", "try_into", "to_owned",
    "clone_into", "write", "write_all", "write_vectored", "write_fmt", "flush", "by_ref", "type_id",
    "is_write_vectored", "write_all_vectored", "from_iter", "default",
];

/// SOUNDNESS R1056 (residual) — what a `String` receiver finds before `str`'s trait impls: `String`'s inherent
/// methods, the std traits it implements, and `str`'s own INHERENT surface (which wins at the deref step,
/// as it does for a direct `&str` receiver). A DENYLIST over a resolution.
pub(crate) const STR_SURFACE: &[&str] = &[
    // String inherent
    "new", "with_capacity", "from_utf8", "from_utf8_lossy", "from_utf16", "into_bytes", "as_str",
    "as_mut_str", "push_str", "extend_from_within", "capacity", "reserve", "reserve_exact", "try_reserve",
    "try_reserve_exact", "shrink_to_fit", "shrink_to", "push", "as_bytes", "truncate", "pop", "remove",
    "retain", "insert", "insert_str", "as_mut_vec", "len", "is_empty", "split_off", "clear", "drain",
    "replace_range", "into_boxed_str", "leak", "into_chars",
    // std traits implemented for `String`
    "clone", "clone_from", "extend", "extend_one", "extend_reserve", "deref", "deref_mut", "as_ref",
    "as_mut", "borrow", "borrow_mut", "hash", "eq", "ne", "cmp", "partial_cmp", "lt", "le", "gt", "ge", "max",
    "min", "clamp", "fmt", "index", "index_mut", "drop", "into", "try_into", "to_owned", "clone_into",
    "to_string", "write_str", "write_char", "write_fmt", "add", "add_assign", "type_id", "from_iter",
    "default", "from_str", "into_iter",
    // str inherent
    "is_char_boundary", "as_bytes_mut", "as_ptr", "as_mut_ptr", "get", "get_mut", "get_unchecked",
    "get_unchecked_mut", "slice_unchecked", "split_at", "split_at_mut", "split_at_checked", "chars",
    "char_indices", "bytes", "split_whitespace", "split_ascii_whitespace", "lines", "lines_any",
    "encode_utf16", "contains", "starts_with", "ends_with", "find", "rfind", "split", "split_inclusive",
    "rsplit", "split_terminator", "rsplit_terminator", "splitn", "rsplitn", "split_once", "rsplit_once",
    "matches", "rmatches", "match_indices", "rmatch_indices", "trim", "trim_start", "trim_end", "trim_left",
    "trim_right", "trim_matches", "trim_start_matches", "strip_prefix", "strip_suffix", "trim_end_matches",
    "trim_left_matches", "trim_right_matches", "parse", "is_ascii", "eq_ignore_ascii_case",
    "make_ascii_uppercase", "make_ascii_lowercase", "trim_ascii", "trim_ascii_start", "trim_ascii_end",
    "escape_debug", "escape_default", "escape_unicode", "replace", "replacen", "to_lowercase",
    "to_uppercase", "into_string", "repeat", "to_ascii_uppercase", "to_ascii_lowercase", "floor_char_boundary",
    "ceil_char_boundary", "char_count",
];

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

thread_local! {
    /// SOUNDNESS R977 — how many enclosing items/modules of the code being walked are themselves
    /// compiled OUT of the default build. Non-zero ⇒ `is_cfg_inactive` answers `false`. See `CfgOffScope`.
    static CFG_OFF_DEPTH: std::cell::Cell<u32> = const { std::cell::Cell::new(0) };
}

/// SOUNDNESS R977 — THE DEFAULT-BUILD FILTER HAS NO MEANING INSIDE CODE THE DEFAULT BUILD DOES NOT HAVE.
///
/// `is_cfg_inactive` drops a `use` (`use_item_applies`) or a statement (`stmt_cfg_inactive`) whose
/// `#[cfg(feature = "x")]` is declared-but-inactive, on the argument that its effects "are not the crate's
/// default behaviour". ITEMS are never dropped that way — a `#[cfg(feature = "aio")] mod aio;` or a
/// `#[cfg(feature = "outer")] fn` is scanned and reported. So inside such an item the engine described a
/// build that cannot exist: the item ON, and every nested feature arm OFF. redis 0.27.6:
///
///     #[cfg(feature = "aio")] pub mod aio;                       // lib.rs — not default, still scanned
///     #[cfg(feature = "tokio-comp")] use ::tokio::net::lookup_host;      // aio/connection.rs
///     async fn get_socket_addrs(..) {
///         #[cfg(feature = "tokio-comp")] let socket_addrs = lookup_host((host, port)).await?;
///         #[cfg(all(not(feature = "tokio-comp"), feature = "async-std-comp"))]
///         let socket_addrs = (host, port).to_socket_addrs().await?;
///
/// Both arms were dropped, so `get_socket_addrs` — which resolves a host name in EVERY build that compiles
/// it — read PURE (ABSENT, `deny Net` 0), and with it `connect_simple` and `ClusterClientBuilder::open`.
/// v0.39.3 hid the silence behind an unrelated `ambiguous:same-name` `Unknown` that ⟨0.40⟩ vein A resolved.
///
/// Inside an item whose own cfg is known-false, every feature is a free variable of the build being
/// described, so a nested feature cfg is UNDECIDABLE, exactly as `unix`/`windows` are — and the engine's
/// rule for an undecidable cfg is to keep every arm (the union; R287/R369). Entering such an item opens a
/// scope in which `is_cfg_inactive` answers `false`. Outside one, nothing changes: the default-build filter
/// keeps its R140 / winnow meaning for code that really is in the default build.
///
/// A thread-local depth rather than a parameter: `is_cfg_inactive` is reached from five call sites in
/// three passes, and threading a flag through every one is the shape that left R373's sixth and seventh
/// site unasked. Every walk that can enter an item is single-threaded per file (Pass A in the merge loop,
/// Pass B in its sequential loop), and the guard is RAII, so an unwind out of `catch_unwind` restores it.
pub(crate) struct CfgOffScope(bool);

/// SOUNDNESS R982 — the key prefix under which a module's FEATURE-INACTIVE `use` bindings are kept beside
/// the active ones (`collect_item_uses`), consulted by `expand` only inside a `CfgOffScope`. `\u{5}` cannot
/// appear in a Rust path.
pub(crate) const CFG_OFF_USE_PREFIX: &str = "\u{5}";

/// Is a `CfgOffScope` open on this thread (and the R982 fallback not suppressed)?
pub(crate) fn cfg_off_active() -> bool {
    CFG_OFF_DEPTH.with(|c| c.get() > 0) && !NO_OFF_FALLBACK.with(|c| c.get())
}

thread_local! {
    static NO_OFF_FALLBACK: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// `expand_noanchor` without R982's inactive-`use` fallback — for a MACRO name, whose other definitions
/// are `macro_rules!` items (not `use`s) the fallback cannot see: taking the inactive `use` arm alone
/// replaced sea-orm's `debug_print!` twin set (a `log::debug` arm and a local no-op arm) with one arm and
/// withdrew the `ambiguous:same-name macro_rules!` disclosure (measured).
pub(crate) fn expand_noanchor_macro(path: &str, uses: &HashMap<String, String>) -> String {
    NO_OFF_FALLBACK.with(|c| c.set(true));
    let r = expand_noanchor(path, uses);
    NO_OFF_FALLBACK.with(|c| c.set(false));
    r
}

/// `CANDOR_R977_DEBUG` set? Read once — the R977/R978 reach markers sit on hot paths.
pub(crate) fn reach_debug() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| std::env::var_os("CANDOR_R977_DEBUG").is_some())
}

impl CfgOffScope {
    /// Open a scope iff `attrs` carry a cfg that is KNOWN-FALSE in the default build. Inside an already
    /// open scope `is_cfg_inactive` answers `false`, so nesting opens nothing — which is right: the outer
    /// scope already suspended the filter.
    pub(crate) fn enter_if(attrs: &[syn::Attribute]) -> Self {
        let on = is_cfg_inactive(attrs);
        if on && reach_debug() {
            eprintln!("R977ITEM"); // §E1 reach, `CANDOR_R977_DEBUG=1` — an item-level scope opened
        }
        Self::enter(on)
    }
    /// Open a scope iff `on` — the FILE-level entry, where the evidence is the declaring `mod`'s cfg (or
    /// an ancestor's), found by `decls::file_module_cfg_off`.
    pub(crate) fn enter(on: bool) -> Self {
        if on {
            CFG_OFF_DEPTH.with(|c| c.set(c.get() + 1));
        }
        CfgOffScope(on)
    }
}

impl Drop for CfgOffScope {
    fn drop(&mut self) {
        if self.0 {
            CFG_OFF_DEPTH.with(|c| c.set(c.get().saturating_sub(1)));
        }
    }
}

/// True if an item/stmt's `#[cfg(...)]` is KNOWN-FALSE under the active feature set (compiled out, so its
/// effects are not the crate's default behaviour). Multiple cfg attrs are AND-ed (any false ⇒ skip).
/// Always `false` inside a `CfgOffScope` (SOUNDNESS R977).
pub(crate) fn is_cfg_inactive(attrs: &[syn::Attribute]) -> bool {
    if !attrs.iter().any(|a| a.path().is_ident("cfg")) {
        return false; // fast path: no cfg attrs (the overwhelming majority of items/stmts)
    }
    if CFG_OFF_DEPTH.with(|c| c.get()) > 0 {
        return false; // R977 — inside an item the default build does not have: every arm is kept
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

/// SOUNDNESS R1034 — the key an impl's methods are filed under, NARROWED to the defect: a non-path self type
/// keeps the module-level qual it always had (`{modpath}::{m}`) UNLESS that qual is a free fn this module
/// declares — the merge R1034 is. Keying every non-path impl (the first cut) renamed thousands of units and
/// moved CHA fan-outs past their cap (jni lost `Log` on 34 rows to a `dispatch:` hedge, bstr/bumpalo traded
/// hedges for resolutions); keying only the colliding ones changes exactly the units that were wrong.
pub(crate) fn impl_key_avoiding_free_fns(im: &syn::ItemImpl, items: &[syn::Item]) -> Option<String> {
    if let Some(t) = impl_type_name(&im.self_ty) {
        return Some(t);
    }
    let methods: Vec<String> = im
        .items
        .iter()
        .filter_map(|ii| match ii {
            syn::ImplItem::Fn(m) => Some(m.sig.ident.to_string()),
            _ => None,
        })
        .collect();
    let collides = items.iter().any(|it| matches!(it, syn::Item::Fn(f) if methods.contains(&f.sig.ident.to_string())));
    if collides {
        if std::env::var_os("CANDOR_R1034_INSTR").is_some() {
            eprintln!("R1034KEY\t{}", methods.join(","));
        }
        impl_unit_type_name(&im.self_ty, &im.generics)
    } else {
        None
    }
}

/// SOUNDNESS R1034 — the type KEY an impl block's methods are filed under, for the unit qual and the CHA
/// edge. `impl_type_name` answers only a PATH self type, so `impl Encode for &'a [u8]` / `&'a str` had no
/// key and its `encode` was filed as the FREE fn `encode::encode` — the same qual as wasm-bindgen-backend's
/// `pub fn encode(program)`, which reads the environment and the filesystem. The two bodies merged into one
/// unit (three rows at the free fn's location) and every `Encode::encode` dispatch charged Env/Fs, while a
/// real `impl Encode for &str` that writes a file was reached by no call at all.
///
/// A non-path self type now gets a key no identifier can spell and no free fn can share: a reference to a
/// primitive/`str`/slice keys as its referent (`&str` → `str`, so `s.encode()` on a `&str` — typed `str` —
/// reaches it), a reference to a nominal type as `&Name` (never `Name`, which would collide with `impl Tr
/// for Name`, the `IntoIterator for &Coll` + `for Coll` pair), a slice/array as `[T]`, a tuple as `(A,B)`.
/// A self type built from the impl's OWN parameters (`&W`, `(C, M)`) is keyed the same way (`&W`, `(C,M)`), as a
/// blanket `impl<T> Tr for T` already is (`T::m`): jni's `impl<C, M> Desc for (C, M)` performs effects of its
/// own (`Log`), and it reached the CHA only by colliding with the `&str` impl's free-fn qual — keying the
/// `&str` impl alone dropped it (measured: 34 jni rows lost `Log`).
pub(crate) fn impl_unit_type_name(ty: &syn::Type, generics: &syn::Generics) -> Option<String> {
    if let Some(t) = impl_type_name(ty) {
        return Some(t);
    }
    let own = |id: &syn::Ident| generics.type_params().any(|tp| tp.ident == *id);
    fn prim(n: &str) -> bool {
        matches!(n, "u8" | "u16" | "u32" | "u64" | "u128" | "usize" | "i8" | "i16" | "i32" | "i64" | "i128"
            | "isize" | "f32" | "f64" | "bool" | "char" | "str")
    }
    fn elem(t: &syn::Type, own: &dyn Fn(&syn::Ident) -> bool) -> Option<String> {
        match t {
            syn::Type::Paren(p) => elem(&p.elem, own),
            syn::Type::Group(g) => elem(&g.elem, own),
            syn::Type::Path(p) if p.qself.is_none() => {
                let seg = p.path.segments.last()?;
                let _ = own(&seg.ident);
                Some(seg.ident.to_string())
            }
            syn::Type::Slice(s) => elem(&s.elem, own).map(|e| format!("[{e}]")),
            syn::Type::Array(a) => elem(&a.elem, own).map(|e| format!("[{e};_]")),
            syn::Type::Tuple(t) if !t.elems.is_empty() => {
                let parts: Option<Vec<String>> = t.elems.iter().map(|e| elem(e, own)).collect();
                parts.map(|v| format!("({})", v.join(",")))
            }
            syn::Type::Reference(r) => elem(&r.elem, own).map(|e| format!("&{e}")),
            _ => None,
        }
    }
    match ty {
        syn::Type::Reference(r) => {
            let inner = elem(&r.elem, &own)?;
            let bare = inner.trim_start_matches('&');
            if inner.starts_with('[') || prim(bare) && !inner.starts_with('&') {
                Some(inner)
            } else {
                Some(format!("&{inner}"))
            }
        }
        _ => elem(ty, &own),
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

/// SOUNDNESS R140, the UNDECIDABLE half — DELIBERATELY STILL ORDER-DEPENDENT HERE, and the reason is a
/// SPEC clause, not a difficulty.
///
/// Two same-name module-level `use` items under mutually-exclusive NON-TEST cfgs resolve by SOURCE
/// ORDER. Reproduced with the order of two lines as the only variable:
/// `#[cfg(unix)] use std::process::Command as Runner;` followed by
/// `#[cfg(windows)] use crate::mockproc::Runner;` reads ABSENT — a purity claim over a real
/// `Command::new(..).status()` — and swapped reads `['Exec']`. Prevalence over 1490 crates: 221 crates
/// (14.8%), 451 sites.
///
/// **The DECIDABLE half is fixed, in `use_item_applies`:** a `#[cfg(feature = "x")]` on a declared-but-
/// inactive feature binds nothing in this build, so the arm is dropped and the remaining one answers
/// precisely. Measured on that pair: ABSENT -> `['Exec']`, and `deny Exec`, `deny Exec Unknown` and
/// `pure run_it` all move 0 -> 1 on the losing order with no policy losing its catch.
///
/// **The UNDECIDABLE half — `unix`/`windows`/`target_os`, which `cfg_eval` returns `None` for because a
/// source scan does not assume a target — is NOT fixed here, and the obvious fix was BUILT AND BACKED
/// OUT.** Routing these three inserts through `decls::record_alias` (R105's join, which keeps both arms
/// and defers to scan.rs) does make the answer order-independent: measured, both orders come back
/// `['Unknown']` instead of one reading ABSENT. But scan.rs adjudicates a joined arm set to
/// `Unknown` + `ambiguous:cfg-duplicated alias`, and **SPEC §4's 2026-09-05 clarification says that is
/// the wrong answer twice over**: the effects "are the union of the arms", picking is fabrication,
/// dropping is a ⟨0.21⟩ purity claim, and `ambiguous:` is reserved for two separately-written
/// definitions rather than a conditional-compilation arm set. That is R287, which is OPEN and owed a
/// ruling plus a four-way rung because the union is a behaviour change candor-rust does not own.
///
/// So the join would have made a known-NONCONFORMANT behaviour more prevalent, and R287's own remedy
/// names that move: "acting on half of it — the kind without the union — is precisely the
/// boundary-drawn-around-the-trigger error". The measured cost of backing it out is stated rather than
/// implied: on the platform pair, `deny Exec` and `pure run_it` still exit 0 over a real `Command`
/// under one arm order. **This is a live cardinal sin held open on purpose, waiting on R287's ruling,
/// not an oversight.**
/// SOUNDNESS R347 — DOES THIS METHOD YIELD THE SAME ELEMENT AS ITS RECEIVER? ONE authority, for both
/// element resolvers.
///
/// **THE DEFECT THIS CLOSES IS THE DUPLICATION ITSELF.** `resolve_elem_type` (concrete element types)
/// and `resolve_elem_trait_leaves` (trait-object elements) each carried their own copy of this list,
/// and the second one's doc said it peels *"exactly like `resolve_elem_type`"* — a sentence that was
/// already false before either was touched. They diverged in BOTH directions and each divergence was a
/// live silence, measured with the other resolver as its own control:
///
/// * The CONCRETE list had no GUARD CHAIN. `self.m.lock().unwrap().iter().for_each(|h| h.run())` over
///   an `Arc<Mutex<Vec<Guard>>>` — one of the most common shapes in real Rust — read ABSENT, while the
///   identical statement over `Arc<Mutex<Vec<Box<dyn Doer>>>>` charged `Fs`. Same for `RefCell::borrow`
///   and `RwLock::read`.
/// * The DISPATCH list had none of the iterator adapters R346 added. `self.vd.iter().rev()
///   .for_each(|d| d.go())` read ABSENT while the same chain without `.rev()` charged.
///
/// So a name being on one list and not the other decided whether an effect was seen, and which way it
/// fell depended on whether the element happened to be a trait object. Neither list was wrong about
/// its own entries; the defect was that there were two.
///
/// **The union is sound because every entry answers the same question for both resolvers** — a
/// `Mutex` guard yields the wrapped collection whatever its element is, and `.rev()` preserves the
/// element whatever it is. The EXCLUSIONS carry over unchanged and are the half worth guarding:
/// `map`/`flat_map`/`flatten`/`zip`/`enumerate` change the element, and `windows`/`chunks` yield a
/// SLICE of it (R346's pinned residual).
pub(crate) fn is_element_preserving_adapter(method: &str) -> bool {
    matches!(
        method,
        // collection adapters and map value views
        "iter" | "into_iter" | "iter_mut" | "clone" | "drain" | "as_slice" | "as_mut_slice"
            | "to_vec" | "values" | "values_mut"
        // R345 — the `as_*` family: `Option::as_ref` gives `Option<&T>`, `Vec::as_ref` gives `&[T]`
            | "as_ref" | "as_mut" | "as_deref" | "as_deref_mut"
        // R346 — the element-preserving ITERATOR adapters
            | "rev" | "take" | "skip" | "step_by" | "peekable" | "by_ref" | "fuse"
            | "chain" | "filter" | "take_while" | "skip_while" | "inspect"
            | "cloned" | "copied"
        // the interior-mutability / smart-pointer GUARD chain, which peels back to the wrapped
        // collection: `reg.lock().unwrap().iter()`, `cell.borrow().iter()`, `rw.read().unwrap().iter()`
            | "lock" | "expect" | "borrow" | "borrow_mut" | "read" | "write"
        // R101 — the deferred-init CELL accessors, which yield the cell's contents exactly as
        // `lock`/`borrow`/`read` yield a `Mutex`/`RefCell`'s …
            | "get_or_init" | "get_mut"
        // … and the Option-returning ELEMENT accessors, whose payload IS the receiver's element, so
        // `if let Some(h) = v.last()` and `for h in v.last()` both want what this returns (R346).
            | "first" | "last" | "get"
        // R401 — `Weak::upgrade` is one of those: it returns `Option<Arc<T>>`, whose payload is the
        // `Weak`'s own element. It is the accessor that makes `Weak` reachable at all, since `Weak`
        // does not `Deref` (see `elem_trait_leaves`).
            | "upgrade"
        // SOUNDNESS R536 — THE REST OF THE `first`/`last`/`get`/`upgrade` FAMILY. Those four arrived
        // one row at a time (R346, R401) and the twenty siblings that answer the SAME question were
        // never added, so `if let Some(g) = v.pop()` read silent-pure while `if let Some(g) =
        // v.first()` beside it charged. Every name here satisfies the list's contract — the ELEMENT
        // of what it returns is the element of its receiver — because each returns `Option<Elem>`
        // and `elem_type`/`elem_trait_leaves` both peel `Option`.
        //   * the iterator terminals that yield ONE item
            | "next" | "next_back" | "nth" | "nth_back" | "find" | "rfind"
            | "min" | "max" | "min_by" | "max_by" | "min_by_key" | "max_by_key" | "reduce"
            | "peek" | "peek_mut" | "next_if" | "next_if_eq"
        //   * the sequence / deque / heap END accessors
            | "pop" | "pop_if" | "pop_front" | "pop_back" | "pop_first" | "pop_last"
            | "front" | "back" | "front_mut" | "back_mut" | "first_mut" | "last_mut"
        //   * the REMOVAL accessors. A map's `remove` is `Option<V>` and satisfies the contract
        //     exactly; a `Vec`'s is a bare `T`, which satisfies it only when `T` is itself a
        //     container. That residual is PINNED (`a_vec_remove_is_one_level_off`) rather than left
        //     to be rediscovered: it is the same one-level-off shape as `unwrap` two comments down,
        //     and it needs `Vec<X>` where `X: IntoIterator` AND the `for y in v.remove(i)` spelling.
            | "remove" | "swap_remove"
        //   * the `Option`/`Result` payload accessors. `Result::ok` is `Option<T>` and
        //     `Option::replace` returns the OLD `Option<T>`; both keep the payload they were given.
            | "ok" | "replace"
        //   * a map's VALUE view, the by-value twin of `values`/`values_mut` already above.
            | "into_values"
        // DELIBERATELY ABSENT, each because it breaks the contract rather than because nobody
        // thought of it — the half R346 says is worth guarding:
        //   * `keys`/`into_keys` yield the KEY, and this list's map arm is about the VALUE.
        //   * `position`/`rposition`/`count`/`len` yield a `usize`, not the element.
        //   * `find_map`/`filter_map`/`flat_map` CHANGE the element (R346's exclusion).
        //   * `split_first`/`split_last`/`first_key_value`/`last_key_value` yield a TUPLE or an
        //     (element, rest) pair — one index deep, exactly like `windows`/`chunks` (R346).
        //     `pop_first`/`pop_last` are IN rather than out, and the split is R454's argument rather
        //     than a preference: on a `BTreeSet` they are `Option<T>` and satisfy the contract, and on
        //     a `BTreeMap` the `Option<(K, V)>` spelling that would mistype cannot reach a consumer —
        //     `if let Some(g) = m.pop_first() { g.run() }` does not COMPILE (a tuple has no method),
        //     and `for (k, v) in m.pop_first()` goes through `resolve_elem_tuple`, which has no map
        //     arm and contributes no binding at all rather than a wrong one.
        //   * `into_inner`/`try_lock`/`try_borrow` are the guard chain's siblings and were swept
        //     with this family, but `into_inner` already carries a CONFLICTING role in
        //     `collector::is_recv_type_changing`, so moving it is a different change with a
        //     different risk and is left stated rather than smuggled in here.
        // `unwrap` is here for the guard chain (`lock().unwrap()`) and is the one entry that is a
        // judgement rather than a fact.
        //
        // SOUNDNESS R536 — THE SENTENCE THAT STOOD HERE WAS WRONG IN BOTH HALVES, AND MEASURED SO.
        // It read: *"it also unwraps an `Option<T>`/`Result<T, _>` whose `T` is NOT a collection,
        // where both resolvers then find no element and return nothing. Harmless in that direction."*
        //
        //   * They do NOT return nothing. `elem_type` has had an `Option`/`Result` arm since R185, so
        //     peeling `unwrap` answers with the PAYLOAD — and the payload of `o.unwrap()` is the value
        //     itself, not its element. The answer is one level off, not absent.
        //   * It is NOT harmless. Built as a two-arm fixture, both arms compiling, one variable:
        //     `struct Holder` whose `go()` spawns and whose `IntoIterator::Item` is an `Item2` whose
        //     `go()` is PURE. `fn ctrl(h: Holder) { for y in h { y.go() } }` reads ABSENT (correct);
        //     `fn f(o: Option<Holder>) { for y in o.unwrap() { y.go() } }` is charged **`['Exec']`**
        //     over a loop whose real item spawns nothing. Held constant: the same `Holder`, the same
        //     body, the same crate — the only difference is the `Option` wrapper and the `.unwrap()`.
        //
        // The entry STAYS, because removing it costs the guard chain R347 measured (`self.m.lock()
        // .unwrap().iter()` over an `Arc<Mutex<Vec<Box<dyn Doer>>>>`), and a fabrication is disclosed
        // where a silence is not. But it is a KNOWN over-charge with a reproduction, not a safe entry:
        // the shape needs `Option<X>`/`Result<X, _>` where `X: IntoIterator` and `X` and its `Item`
        // share a method name. `expect` and the `unwrap_or*` family have the identical shape. Closing
        // it means splitting "the result CONTAINS the same element" from "the result IS the element",
        // which is the distinction `is_element_yielding_accessor` states — a separate change, owed its
        // own row and its own A/B, and named here rather than left as a claim of safety.
            | "unwrap"
    )
}

/// SOUNDNESS R349 — THE HOFs WHOSE ELEMENT PARAMETER IS **NOT** PARAMETER 0.
///
/// The adapter list in `collector.rs` types parameter 0 of a SINGLE-parameter closure from the
/// receiver's element, and its comment said *"`fold`'s accumulator is its first param so it is NOT a
/// single-param closure and is skipped (would mis-type the accumulator)"*. **That sentence is correct
/// and it is the defect**: it settles parameter 0 and reads as a ruling on `fold`, so parameter 1 —
/// which IS the element — was never typed by anything, and `v.iter().fold(0, |a, g| a + g.run())` over
/// a `Vec<Guard>` whose `run()` writes a file left the CALLER absent from `functions[]`: a §4 purity
/// claim over a file write, on one of the most common HOFs in the language. The swift engine had the
/// same hole in the same place for the same reason (`reduce`), which is what the row is about.
///
/// Two categories, because arity alone does not say WHICH parameter is the element:
///
/// * **LAST parameter** — the FOLD family. `fold`/`rfold`/`try_fold`/`try_rfold` take `(Acc, Item)`
///   and `scan` takes `(&mut State, Item)`: parameter 0 is the accumulator/state and must NOT be typed
///   from the element — that is exactly the mis-typing the old comment was right to refuse, and the
///   `trap_acc` control in the fixture is what holds this half honest.
/// * **EVERY parameter** — the COMPARATOR family, `(&Item, &Item)`; `reduce` is `(Item, Item)`.
///
/// Both lists are `Iterator`/slice inherent methods, so the element is the receiver's element by
/// definition — the same warrant every entry on the single-parameter list already has. A user type
/// with its own `fold` carries the same (pre-existing) risk as one with its own `map`, and the binding
/// fires at all only once the receiver already resolved to a known collection.
/// SOUNDNESS R446 — THE ACCESSORS WHOSE RESULT IS THE RECEIVER'S ELEMENT, asked at a RECEIVER
/// position. `is_element_preserving_adapter` beside this one answers "does this adapter keep the
/// element the same", which is the question the element RESOLVERS ask as they peel a chain; this one
/// answers "is this expression the element of the thing it was called on", which is what the two
/// RECEIVER resolvers need in order to type `c.get(k).unwrap().run()` at all.
///
/// MEASURED AT HEAD, over a `Vec<G>` / `HashMap<String, Box<dyn Doer>>` whose method spawns a process,
/// with `v[0].run()` and `if let Some(g) = v.get(0) { g.run() }` both charging as the calibration:
/// `v.get(0).unwrap().run()`, `v.first().unwrap().run()`, `m.get(k).unwrap().go()` and
/// `w.upgrade().unwrap().go()` were ALL ABSENT — on the concrete route and the dispatch route alike.
/// **That is a wider class than R401's "a `HashMap` VALUE" framing, and the framing is what hid it:
/// `v.get(0).unwrap()` over a `Vec` is absent too, so the axis is not the container's shape, it is the
/// BINDING SITE.** The `if let` / `for` / index sites resolve an element and the receiver position did
/// not, so the same container answered or stayed silent according to how the caller spelled the reach.
///
/// SOUNDNESS R536 — AND IT WAS FIVE NAMES OF A FAMILY OF TWENTY-FIVE. The paragraph above is about
/// the BINDING SITE and it is right; what it did not say is that the five names it shipped with were
/// simply the ones R346 and R401 had already put on `is_element_preserving_adapter` for a different
/// reason. So `v.get(0).unwrap().emit()` charged and `v.pop().unwrap().emit()` — the same container,
/// the same call, one accessor over — was ABSENT, on the concrete route and the dispatch route alike.
/// Measured over `Vec<Box<dyn Sink>>` / `VecDeque` / `BinaryHeap` / `HashMap` / `BTreeSet` /
/// `Option` / `Result`, **38 of 52 arms silent** against `first()`/`last()`/`get(0)`/`v[0]`/
/// `Option::take()` as the charging calibration. The BINDING SITE was swept as its own axis and is
/// not the variable: `if let` / `while let` / `match` / `let`-else / `?` / `for g in` / `.map(|g|..)`
/// and a FIELD receiver all read the same, before and after.
///
/// THE TWO LISTS ARE NOT THE SAME QUESTION and the difference decides membership:
/// `is_element_preserving_adapter` asks *does the result CONTAIN the same element* (so `take(2)`,
/// `rev()`, `filter(..)` belong and are deliberately absent from here — `v.take(2)` is still a
/// container, and typing it as the element would fabricate one level in). THIS list asks *is the
/// result the element itself*, which is why the `Option`-returning accessors are on both: at a
/// receiver position they are always spelled through an `.unwrap()`/`.expect()`, and those two walk
/// to their own receiver rather than needing an entry here.
/// VEIN B — the std single-value wrappers whose type argument `elem_type_b` records as their element.
/// SOUNDNESS R1023 — A LAYERED ELEMENT ENTRY. An element index entry (`elem_of`, `field_elem`, the
/// static and return element keys) is either PLAIN — `G`, meaning "one std layer (a sequence, a map's
/// values, an `Option`/`Result` payload, a wrapper's held value) and then `G`", the semantics every
/// one-level consumer in the collector was written for — or LAYERED: `G` + this mark + the LIST of std
/// layers between the value and `G`, outermost first (`m: Mutex<Option<G>>` records `G\u{1c}MO`).
///
/// WHY A LIST AND NOT THE ONE-BIT MARK IT REPLACES. R893 recorded `Mutex<Vec<G>>` as `G` + a bare mark
/// meaning "a container of G held by a wrapper", and then had to hand-encode, consumer by consumer, which
/// layer each adapter and binder was looking at: `get`/`get_mut`/`ok` were "ambiguous", `for v in
/// m.lock()` needed a thirteen-name exception, the payload binders and the HOF route refused outright.
/// None of those ambiguities is in Rust — they are what a one-bit encoding of a two-layer type cannot
/// say. And the bit could not express a second level at all, which is R1023: `Mutex<Option<G>>`,
/// `Option<Vec<G>>`, `&Mutex<Option<G>>` recorded nothing and their callers read ABSENT (executed). The
/// list states which layer is on top, so each adapter is ONE transition on the top layer
/// (`layer_step`) and each binder ONE pop of a layer of its own kind (`layer_bind`). A method or binder
/// the table does not name REFUSES — the answer is `None`, never a guess — so the failure direction of an
/// incomplete table is the silence this replaces, not a fabrication.
///
/// The codes: `M` a lock cell (`Mutex`/`RwLock`/`ReentrantMutex`), `R` a `RefCell`, `L` a `OnceLock`/
/// `OnceCell`, `Q` what a lock accessor returned (std's `LockResult`, or another crate's guard — see
/// `layer_step`), `O` `Option`, `E` `Result`, `C` a sequence (or slice/array), `K` a map (its VALUES), `I`
/// an iterator. Transparent layers (`&`, `Box`/`Arc`/`Rc`/`Pin`, a guard's `Deref`, `LazyLock`) are not
/// recorded. A list of ONE layer is always written PLAIN — that is exactly the one-level semantics — and a
/// list of ZERO is the value `G` itself, which is not an element entry at all (`Layered::Val`).
///
/// A SUFFIX so every leaf taken by `rsplit("::")` keeps it (a twin leaf, an alias re-expansion), behind a
/// control character no identifier can contain.
pub(crate) const WRAPPED_CONTAINER_MARK: char = '\u{1c}';
/// After the codes, the WRITTEN path of each layer whose type is not std's own (`ahash::HashMap`, a
/// crate's `type Map = …`), one per code and empty for std's — so a binder that peels to that layer types
/// the binding with the path the source named, as the one-level route did (redis's `Vec<HashMap<..>>`
/// over an `ahash` map: typing `row` as std's `HashMap` lost `row.into_iter()`'s `invisible: [ahash]`).
/// `::` is re-spelled so a leaf taken by `rsplit("::")` still ends at `G`'s suffix.
const LAYER_PATH_SEP: char = '\u{1b}';
const LAYER_PATH_COLONS: char = '\u{1a}';

/// One std layer: its code and the written path of its type when that is not std's own (else empty).
pub(crate) type Layer = (char, String);

/// `G` with the layer list `codes` (outermost first, std's own types), normalised: one layer is plain `G`.
#[cfg(test)]
pub(crate) fn with_layers(g: &str, codes: &str) -> String {
    encode_layers(g, &codes.chars().map(|c| (c, String::new())).collect::<Vec<_>>())
}
fn encode_layers(g: &str, ls: &[Layer]) -> String {
    if ls.len() <= 1 {
        return g.to_string();
    }
    let codes: String = ls.iter().map(|(c, _)| *c).collect();
    if ls.iter().all(|(_, p)| p.is_empty()) {
        return format!("{g}{WRAPPED_CONTAINER_MARK}{codes}");
    }
    let paths: Vec<String> = ls.iter().map(|(_, p)| p.replace("::", &LAYER_PATH_COLONS.to_string())).collect();
    format!("{g}{WRAPPED_CONTAINER_MARK}{codes}{LAYER_PATH_SEP}{}", paths.join(&LAYER_PATH_SEP.to_string()))
}
fn decode_layers(t: &str) -> (&str, Vec<Layer>) {
    let Some((g, rest)) = t.split_once(WRAPPED_CONTAINER_MARK) else { return (t, Vec::new()) };
    let mut parts = rest.split(LAYER_PATH_SEP);
    let codes = parts.next().unwrap_or("");
    let paths: Vec<String> = parts.map(|p| p.replace(LAYER_PATH_COLONS, "::")).collect();
    let ls = codes.chars().enumerate().map(|(i, c)| (c, paths.get(i).cloned().unwrap_or_default())).collect();
    (g, ls)
}
pub(crate) fn is_wrapped(t: &str) -> bool {
    t.contains(WRAPPED_CONTAINER_MARK)
}
/// The `G` of an entry, plain or layered.
pub(crate) fn strip_wrapped(t: &str) -> &str {
    t.split(WRAPPED_CONTAINER_MARK).next().unwrap_or(t)
}
/// The layer CODES of a LAYERED entry (`""` for a plain one).
pub(crate) fn wrapped_layers(t: &str) -> &str {
    t.split_once(WRAPPED_CONTAINER_MARK).map(|(_, c)| c.split(LAYER_PATH_SEP).next().unwrap_or("")).unwrap_or("")
}
/// The same layers over a different `G` (an alias re-expansion of the leaf).
pub(crate) fn rewrap(t: &str, g: &str) -> String {
    match t.split_once(WRAPPED_CONTAINER_MARK) {
        Some((_, rest)) => format!("{g}{WRAPPED_CONTAINER_MARK}{rest}"),
        None => g.to_string(),
    }
}
/// `e` (plain = one layer `inner`, or layered) under one more std layer `code` on top.
pub(crate) fn prefix_layer(code: char, inner: char, e: &str) -> String {
    let (g, mut ls) = decode_layers(e);
    if ls.is_empty() {
        ls.push((inner, String::new()));
    }
    ls.insert(0, (code, String::new()));
    encode_layers(g, &ls)
}

/// What a step or a binder over a LAYERED entry leaves.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Layered {
    /// No layer left: the value IS `G`.
    Val(String),
    /// Layers left: the element entry (plain when one is left) and the type of the new OUTER layer, which
    /// is what a binder installs as the binding's own type — the same string the one-level route gave it
    /// before this existed (`for row in grid` bound `row: Vec`).
    Elem { entry: String, outer: Option<String> },
}

/// The std type a binding whose outermost layer is `code` has, for `vars`. `None` for `Q`/`I`, which
/// name no single type.
pub(crate) fn layer_outer_type(code: char) -> Option<&'static str> {
    Some(match code {
        'C' => "Vec",
        'K' => "std::collections::HashMap",
        'O' => "Option",
        'E' => "Result",
        'M' => "std::sync::Mutex",
        'R' => "std::cell::RefCell",
        'L' => "std::sync::OnceLock",
        _ => return None,
    })
}

fn layered_from(g: &str, ls: &[Layer]) -> Layered {
    match ls.first() {
        None => Layered::Val(g.to_string()),
        Some((c, p)) => Layered::Elem {
            entry: encode_layers(g, ls),
            outer: if p.is_empty() { layer_outer_type(*c).map(str::to_string) } else { Some(p.clone()) },
        },
    }
}

/// The layer code a std type's LAST segment names, for the layer walk. `None` for anything else.
fn layer_code_of(leaf: &str) -> Option<char> {
    if is_sequence_container(leaf) {
        return Some('C');
    }
    if is_map_container(leaf) {
        return Some('K');
    }
    Some(match leaf {
        "Option" => 'O',
        "Result" | "IoResult" => 'E',
        "Mutex" | "RwLock" | "ReentrantMutex" => 'M',
        "RefCell" => 'R',
        "OnceLock" | "OnceCell" => 'L',
        _ => return None,
    })
}

/// VEIN B — what a std wrapper's held value must be for `elem_type_b` to record it plain: not a std
/// container/wrapper/`Option`, and not a std type other than an effect handle.
pub(crate) fn is_held_nominal(t: &str) -> bool {
    let leaf = t.rsplit("::").next().unwrap_or(t);
    !(is_sequence_container(leaf)
        || is_map_container(leaf)
        || is_value_wrapper(leaf)
        || matches!(leaf, "Option" | "Result" | "IoResult" | "String" | "Box" | "Arc" | "Rc" | "Cow" | "Pin")
        || (matches!(t.split("::").next(), Some("std" | "core" | "alloc"))
            && !candor_classify::is_std_effect_handle(t)))
}

/// The NOMINAL leaf a layer walk may end on: a type the crate can give methods to (or a std effect
/// handle), never a std container/wrapper, a `String`, or a primitive.
pub(crate) fn is_layer_leaf(t: &str) -> bool {
    let leaf = t.rsplit("::").next().unwrap_or(t);
    is_held_nominal(t)
        && !(leaf == "Self"
        || matches!(
            leaf,
            "u8" | "u16" | "u32" | "u64" | "u128" | "usize" | "i8" | "i16" | "i32" | "i64" | "i128" | "isize"
                | "f32" | "f64" | "bool" | "char" | "str"
        ))
}

/// SOUNDNESS R1023 — the std layers of `ty` down to a nominal leaf: `(codes, leaf, borrowed)`. `None` when
/// the walk meets anything but a std layer, a transparent pointer, or a nominal leaf. `borrowed` is
/// R718's question asked along this walk (a map's KEY is not on it).
pub(crate) fn layer_walk(ty: &syn::Type, uses: &HashMap<String, String>) -> Option<(Vec<Layer>, String, bool)> {
    let push = |code: char, path: String, (mut ls, g, b): (Vec<Layer>, String, bool)| {
        ls.insert(0, (code, path));
        (ls, g, b)
    };
    match ty {
        syn::Type::Reference(r) => layer_walk(&r.elem, uses).map(|(c, g, _)| (c, g, true)),
        syn::Type::Paren(p) => layer_walk(&p.elem, uses),
        syn::Type::Group(g) => layer_walk(&g.elem, uses),
        syn::Type::Slice(s) => layer_walk(&s.elem, uses).map(|w| push('C', String::new(), w)),
        syn::Type::Array(a) => layer_walk(&a.elem, uses).map(|w| push('C', String::new(), w)),
        syn::Type::Path(p) => {
            let seg = p.path.segments.last()?;
            let name = seg.ident.to_string();
            let tys: Vec<&syn::Type> = match &seg.arguments {
                syn::PathArguments::AngleBracketed(a) => a
                    .args
                    .iter()
                    .filter_map(|x| match x {
                        syn::GenericArgument::Type(t) => Some(t),
                        _ => None,
                    })
                    .collect(),
                _ => Vec::new(),
            };
            if !layer_path_is_std(&p.path, uses) {
                let (t, b) = type_path_b(ty, uses)?;
                return is_layer_leaf(&t).then_some((Vec::new(), t, b));
            }
            if matches!(name.as_str(), "Box" | "Arc" | "Rc" | "Pin" | "LazyLock" | "LazyCell") {
                return layer_walk(tys.first()?, uses);
            }
            if let Some(code) = layer_code_of(&name) {
                let inner = if code == 'K' { tys.get(1)? } else { tys.first()? };
                // The written path, kept only when it is not std's own type (see `LAYER_PATH_SEP`).
                let full = expand(&path_to_string_lc(&p.path), uses);
                let root = full.split("::").next().unwrap_or("");
                let path = if matches!(root, "std" | "core" | "alloc") || !full.contains("::") {
                    String::new()
                } else {
                    full
                };
                return layer_walk(inner, uses).map(|w| push(code, path, w));
            }
            let (t, b) = type_path_b(ty, uses)?;
            is_layer_leaf(&t).then_some((Vec::new(), t, b))
        }
        _ => None,
    }
}

/// SOUNDNESS R1023 — whether a written type path may be read as the std layer its LEAF names. Refused only
/// where the evidence says the name is the crate's OWN type: the written path is one segment and this
/// module declares it (R482's `Vec<Mutex<T>>` regression control has a local `struct Mutex<T>` whose
/// `read()` is effectful — reading it as a lock layer typed `self.hs[0]` as `std::sync::Mutex` and dropped
/// the edge). Every other spelling is read by its leaf, the rule the one-level route has always applied
/// (`Mutex<G>` records `G` by the segment name): a `use crate::sync::Mutex` over a crate-root `use
/// std::sync;` (rayon-core) is std's, and refusing it lost R893's element — measured, `inject_broadcast`'s
/// `invisible: [crossbeam_deque]` disappeared.
fn layer_path_is_std(path: &syn::Path, uses: &HashMap<String, String>) -> bool {
    if path.leading_colon.is_some() || path.segments.len() != 1 {
        return true;
    }
    module_declares(uses, &path.segments[0].ident.to_string()) != Some(true)
}

/// SOUNDNESS R1023 — ONE adapter `method` applied to a LAYERED entry `t`: the transition on its top layer.
/// `None` for a method this table does not name on that layer (refused, never guessed).
///
/// `Q` is the one layer whose identity is not known from the type: std's `lock()` returns a `LockResult`,
/// `parking_lot`'s and `tokio`'s a guard. `unwrap`/`expect`/`unwrap_or_else`/`ok`/`?` exist only on the
/// `LockResult` reading (a guard over a non-`Copy` payload cannot be unwrapped by value) and are answered
/// that way; any other method is the guard's `Deref` target's, so `Q` is popped and the method applied to
/// what it guards.
pub(crate) fn layer_step(t: &str, method: &str) -> Option<Layered> {
    let (g, ls) = decode_layers(t);
    layer_step_ls(g, &ls, method)
}
fn layer_step_ls(g: &str, ls: &[Layer], method: &str) -> Option<Layered> {
    let top = ls.first()?.0;
    let rest = &ls[1..];
    let out = |r: &[Layer]| Some(layered_from(g, r));
    let same = || Some(layered_from(g, ls));
    let swap = |c: char| {
        let mut v = vec![(c, String::new())];
        v.extend_from_slice(rest);
        Some(layered_from(g, &v))
    };
    let rest_top = rest.first().map(|l| l.0);
    match top {
        'M' => match method {
            "lock" | "read" | "write" | "try_lock" | "try_read" | "try_write" | "blocking_lock" | "blocking_read"
            | "blocking_write" | "get_mut" | "into_inner" => swap('Q'),
            _ => None,
        },
        'R' => match method {
            "borrow" | "borrow_mut" | "get_mut" | "into_inner" | "take" => out(rest),
            "try_borrow" | "try_borrow_mut" => swap('E'),
            _ => None,
        },
        'L' => match method {
            "get" | "get_mut" | "into_inner" | "take" => swap('O'),
            "get_or_init" | "wait" | "force" => out(rest),
            "get_or_try_init" => swap('E'),
            _ => None,
        },
        'Q' => match method {
            "unwrap" | "expect" | "unwrap_or_else" | "unwrap_unchecked" => out(rest),
            "ok" => swap('O'),
            "is_ok" | "is_err" | "err" | "map_err" | "map" | "and_then" | "or_else" => None,
            // the guard's `Deref`: the method is its target's
            _ if !rest.is_empty() => layer_step_ls(g, rest, method),
            _ => None,
        },
        'O' => match method {
            "unwrap" | "expect" | "unwrap_unchecked" | "unwrap_or_default" | "unwrap_or" | "unwrap_or_else"
            | "insert" | "get_or_insert" | "get_or_insert_with" => out(rest),
            "as_ref" | "as_mut" | "as_deref" | "as_deref_mut" | "clone" | "cloned" | "copied" | "take" | "replace"
            | "filter" | "or" | "or_else" | "xor" | "inspect" | "take_if" => same(),
            "iter" | "iter_mut" | "into_iter" => swap('I'),
            "ok_or" | "ok_or_else" => swap('E'),
            "flatten" if rest_top == Some('O') => out(rest),
            _ => None,
        },
        'E' => match method {
            "unwrap" | "expect" | "unwrap_unchecked" | "unwrap_or_default" | "unwrap_or" | "unwrap_or_else" => out(rest),
            "as_ref" | "as_mut" | "as_deref" | "as_deref_mut" | "clone" | "cloned" | "copied" | "inspect" | "map_err"
            | "or_else" => same(),
            "ok" => swap('O'),
            "iter" | "iter_mut" | "into_iter" => swap('I'),
            _ => None,
        },
        'C' => match method {
            "iter" | "iter_mut" | "into_iter" | "drain" => swap('I'),
            "clone" | "as_slice" | "as_mut_slice" | "to_vec" | "as_ref" | "as_mut" => same(),
            "first" | "last" | "get" | "get_mut" | "first_mut" | "last_mut" | "pop" | "pop_front" | "pop_back"
            | "front" | "back" | "front_mut" | "back_mut" | "pop_first" | "pop_last" | "pop_if" => swap('O'),
            _ => None,
        },
        'K' => match method {
            "values" | "values_mut" | "into_values" => swap('I'),
            "get" | "get_mut" | "remove" => swap('O'),
            "clone" => same(),
            _ => None,
        },
        'I' => match method {
            "next" | "next_back" | "nth" | "nth_back" | "last" | "find" | "rfind" | "min" | "max" | "min_by"
            | "max_by" | "min_by_key" | "max_by_key" | "peek" | "peek_mut" | "reduce" | "next_if" | "next_if_eq" => {
                swap('O')
            }
            "rev" | "take" | "skip" | "step_by" | "peekable" | "by_ref" | "fuse" | "filter" | "take_while"
            | "skip_while" | "inspect" | "cloned" | "copied" | "iter" | "into_iter" => same(),
            "flatten" if matches!(rest_top, Some('O' | 'E' | 'C' | 'I')) => {
                let mut v = vec![('I', String::new())];
                v.extend_from_slice(&rest[1..]);
                Some(layered_from(g, &v))
            }
            _ => None,
        },
        _ => None,
    }
}

/// `?`/`*` over a LAYERED entry. `?` pops an `Option`/`Result`/lock result; `*` pops only `Q` (a guard's
/// `Deref` — every other layer is either transparent to `*` or not dereferenceable).
pub(crate) fn layer_try(t: &str) -> Option<Layered> {
    let (g, ls) = decode_layers(t);
    matches!(ls.first()?.0, 'O' | 'E' | 'Q').then(|| layered_from(g, &ls[1..]))
}
pub(crate) fn layer_deref(t: &str) -> Option<Layered> {
    let (g, ls) = decode_layers(t);
    match ls.first()?.0 {
        'Q' => Some(layered_from(g, &ls[1..])),
        _ => Some(layered_from(g, &ls)),
    }
}

/// The binder positions a LAYERED entry can be peeled at, each popping a layer of its own kind only.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum LayerBinder {
    /// `for x in e` — a sequence, an iterator, an `Option`/`Result`/lock result (iterating yields the
    /// payload). NOT a map: its item is a `(K, V)` tuple.
    For,
    /// `e[i]` — a sequence or a map's value.
    Index,
    /// `Some(x)`
    Some,
    /// `Ok(x)`
    Ok,
    /// The element parameter of a closure passed to an adapter `method` called ON the entry.
    Hof,
}

/// SOUNDNESS R1023 — the binding a `binder` peels out of a LAYERED entry `t`. `None` when the top layer
/// is not one that binder peels — refused.
pub(crate) fn layer_bind(t: &str, binder: LayerBinder, method: &str) -> Option<Layered> {
    let (g, ls) = decode_layers(t);
    layer_bind_ls(g, &ls, binder, method)
}
fn layer_bind_ls(g: &str, ls: &[Layer], binder: LayerBinder, method: &str) -> Option<Layered> {
    let top = ls.first()?.0;
    let ok = match binder {
        LayerBinder::For => matches!(top, 'C' | 'I' | 'O' | 'E' | 'Q'),
        LayerBinder::Index => matches!(top, 'C' | 'K'),
        LayerBinder::Some => top == 'O',
        LayerBinder::Ok => matches!(top, 'E' | 'Q'),
        LayerBinder::Hof => match top {
            'I' | 'O' | 'E' => true,
            // `LockResult::map`/`and_then`/`is_ok_and` receive the guard; any other adapter is the
            // guarded value's (a `parking_lot` guard's `Deref`), so pop `Q` and ask again.
            'Q' => {
                if matches!(method, "map" | "and_then" | "is_ok_and" | "inspect" | "map_or" | "map_or_else") {
                    true
                } else {
                    return layer_bind_ls(g, &ls[1..], binder, method);
                }
            }
            // `Vec::retain(|x| ..)`, `sort_by_key`, … take the element.
            'C' => matches!(
                method,
                "retain" | "retain_mut" | "sort_by_key" | "sort_by_cached_key" | "sort_unstable_by_key"
                    | "dedup_by_key" | "binary_search_by" | "binary_search_by_key" | "partition_point"
                    | "extract_if" | "is_sorted_by_key"
            ),
            _ => false,
        },
    };
    ok.then(|| layered_from(g, &ls[1..]))
}

pub(crate) fn is_value_wrapper(name: &str) -> bool {
    matches!(
        name,
        "RefCell" | "Cell" | "Mutex" | "RwLock" | "OnceLock" | "OnceCell" | "LazyLock" | "LazyCell"
            | "ReentrantMutex"
    )
}

/// VEIN B — `(wrapper kind, method)` pairs whose result IS the wrapper's type argument (possibly behind a
/// guard, `&`, `LockResult` or `Option`, all of which the typer already treats as transparent). Keyed on
/// the KIND, never on the method alone: `Vec::as_ref` yields a slice and `Mutex::get_mut` a `&mut T`, so
/// the same leaf means different things on different receivers. Every entry is a std inherent method.
/// True when ANY wrapper kind in `is_wrapper_accessor` declares `method` — the cheap pre-test that keeps
/// the receiver re-typing off every other chain step.
/// VEIN B — the `Option`/`Result` PLUMBING a guard or cell accessor's result is unwrapped through
/// (`m.lock().unwrap()`, `c.try_borrow_mut().expect(..)`). The typer answers the accessor with the
/// wrapped `T`, so a typed call formed for the plumbing would read `T::unwrap` — and for a std effect
/// handle the whole-type rule charges that. The method is never `T`'s own.
pub(crate) fn is_result_plumbing(method: &str) -> bool {
    matches!(
        method,
        "unwrap" | "expect" | "unwrap_or" | "unwrap_or_else" | "unwrap_or_default" | "unwrap_unchecked"
            | "ok" | "err" | "map_err" | "is_ok" | "is_err" | "is_some" | "is_none" | "as_ref" | "as_mut"
            | "as_deref" | "as_deref_mut" | "cloned" | "copied"
    )
}

/// VEIN B — whether any step of a method-call RECEIVER chain is a wrapper accessor, i.e. whether the
/// vein-B wrapper arm can have changed the walk's answer for a call on it.
pub(crate) fn is_any_wrapper_accessor_chain(expr: &syn::Expr) -> bool {
    let mut e = expr;
    loop {
        match e {
            syn::Expr::Reference(r) => e = &r.expr,
            syn::Expr::Paren(p) => e = &p.expr,
            syn::Expr::Group(g) => e = &g.expr,
            syn::Expr::Try(t) => e = &t.expr,
            syn::Expr::Await(a) => e = &a.base,
            syn::Expr::Field(f) => e = &f.base,
            syn::Expr::MethodCall(m) => {
                if is_any_wrapper_accessor(&m.method.to_string()) {
                    return true;
                }
                e = &m.receiver;
            }
            _ => return false,
        }
    }
}

pub(crate) fn is_any_wrapper_accessor(method: &str) -> bool {
    ["RefCell", "Mutex", "RwLock", "OnceLock", "LazyLock", "Cell", "Option", "Result"]
        .iter()
        .any(|k| is_wrapper_accessor(k, method))
}

pub(crate) fn is_wrapper_accessor(kind: &str, method: &str) -> bool {
    match kind {
        "RefCell" => matches!(method, "borrow" | "borrow_mut" | "try_borrow" | "try_borrow_mut" | "get_mut"),
        "Mutex" | "ReentrantMutex" => matches!(method, "lock" | "try_lock" | "get_mut" | "blocking_lock"),
        "RwLock" => matches!(method, "read" | "write" | "try_read" | "try_write" | "get_mut"
            | "blocking_read" | "blocking_write"),
        "OnceLock" | "OnceCell" => matches!(method, "get" | "get_mut" | "get_or_init" | "get_or_try_init" | "wait"),
        "LazyLock" | "LazyCell" => matches!(method, "force"),
        "Cell" => matches!(method, "get" | "take" | "replace" | "get_mut"),
        // NOT `as_ref`/`as_mut`/`as_deref`: those yield `Option<&T>`, still an Option, and typing them as
        // `T` formed `Child::unwrap` on `self.inner.as_mut().unwrap()` — charged `Exec` by the whole-type
        // handle rule over a getter (async-process `ChildGuard::get_mut`, measured on the corpus). They
        // are element-PRESERVING adapters, which is how the `unwrap` after them already reaches `T`.
        "Option" => matches!(method, "unwrap" | "expect"
            | "unwrap_unchecked" | "unwrap_or_default" | "insert" | "get_or_insert" | "get_or_insert_with"),
        "Result" | "IoResult" => matches!(method, "unwrap" | "expect" | "unwrap_unchecked"
            | "unwrap_or_default"),
        _ => false,
    }
}

pub(crate) fn is_element_yielding_accessor(method: &str) -> bool {
    matches!(
        method,
        "get" | "get_mut" | "first" | "last" | "upgrade"
        // R536 — the rest of the family. Same order and same warrant as the additions to
        // `is_element_preserving_adapter`; see there for the exclusions and why each is out.
            | "first_mut" | "last_mut" | "front" | "back" | "front_mut" | "back_mut"
            | "pop" | "pop_if" | "pop_front" | "pop_back" | "pop_first" | "pop_last"
            | "remove" | "swap_remove"
            | "next" | "next_back" | "nth" | "nth_back" | "find" | "rfind"
            | "min" | "max" | "min_by" | "max_by" | "min_by_key" | "max_by_key" | "reduce"
            | "peek" | "peek_mut" | "next_if" | "next_if_eq"
            | "ok" | "replace"
    )
}

/// SOUNDNESS R536 §E1 REACH PROBE — true for the names R536 ADDED to either list, and for nothing
/// else. Every consumer of the two lists is instrumented through this one predicate so "the branch
/// never ran" and "the branch ran and moved nothing" are distinguishable claims in the A/B, which is
/// the distinction R79/R85/R87/R92 each got wrong by measuring only the diff. Compiled in
/// unconditionally and gated at each call site on `CANDOR_R536_INSTR`.
pub(crate) fn is_r536_added_name(method: &str) -> bool {
    matches!(
        method,
        "next" | "next_back" | "nth" | "nth_back" | "find" | "rfind"
            | "min" | "max" | "min_by" | "max_by" | "min_by_key" | "max_by_key" | "reduce"
            | "peek" | "peek_mut" | "next_if" | "next_if_eq"
            | "pop" | "pop_if" | "pop_front" | "pop_back" | "pop_first" | "pop_last"
            | "front" | "back" | "front_mut" | "back_mut" | "first_mut" | "last_mut"
            | "remove" | "swap_remove"
            | "ok" | "replace"
            | "into_values"
    )
}

pub(crate) fn is_elem_last_param_adapter(method: &str) -> bool {
    matches!(method, "fold" | "rfold" | "try_fold" | "try_rfold" | "scan")
}

/// SOUNDNESS R349 — see `is_elem_last_param_adapter`. EVERY closure parameter is the element.
/// `eq_by`/`cmp_by`/`partial_cmp_by` are deliberately ABSENT: their second parameter is the OTHER
/// iterator's item, not this receiver's, so typing it from this element would FABRICATE.
pub(crate) fn is_elem_pair_adapter(method: &str) -> bool {
    matches!(
        method,
        "sort_by"
            | "sort_unstable_by"
            | "max_by"
            | "min_by"
            | "dedup_by"
            | "reduce"
            | "is_sorted_by"
            | "chunk_by"
            | "chunk_by_mut"
            | "select_nth_unstable_by"
    )
}

/// SOUNDNESS R349 — the (name, element type) bindings a TUPLE pattern takes from a per-slot element
/// answer (`Collector::resolve_elem_tuple`). Used by BOTH tuple consumers — the `for (_, g) in
/// xs.iter().enumerate()` binder and the `for_each(|(_, g)| ..)` closure parameter — because they are
/// the same question and this family's bugs recur wherever one question has two implementations.
///
/// STRICTLY ADDITIVE: a slot with no resolved type contributes no binding and is left ALONE rather than
/// cleared, and a pattern whose arity disagrees with the answer contributes NOTHING — an arity mismatch
/// means the shape is not what the resolver thinks it is, and binding positionally through it would
/// mistype every slot after the disagreement.
///
/// SOUNDNESS R538 — the slot carries a `Bound`, not a type STRING. R349 wrote it as a `String` and that
/// is the whole of R538: a `Vec<Box<dyn Sink>>` element has no concrete type to put in one, so the
/// dispatch half of the question could not be carried through this function even after the resolver
/// answered it. The pattern side is unchanged.
pub(crate) fn tuple_pat_elem_binds(
    pat: &syn::Pat,
    slots: Option<&[Option<crate::collector::Bound>]>,
) -> Vec<(String, crate::collector::Bound)> {
    let slots = match slots {
        Some(s) => s,
        None => return Vec::new(),
    };
    let inner = match pat {
        syn::Pat::Reference(r) => &*r.pat,
        syn::Pat::Paren(p) => &*p.pat,
        syn::Pat::Type(t) => &*t.pat,
        p => p,
    };
    let syn::Pat::Tuple(tup) = inner else { return Vec::new() };
    if tup.elems.len() != slots.len() {
        return Vec::new();
    }
    tup.elems
        .iter()
        .zip(slots)
        .filter_map(|(el, slot)| Some((single_pat_ident(el)?, slot.clone()?)))
        .collect()
}

/// SOUNDNESS R350 — OF THE ADAPTERS ABOVE, THE ONES THAT MUST **NOT** FALL BACK TO THE `returns`
/// INDEX when the receiver route answers nothing. This is a DENYLIST, and the direction is the whole
/// point: a name added to `is_element_preserving_adapter` in future falls back BY DEFAULT, which is
/// the safe direction. **The allowlist spelling is what caused R350 in the first place** — R347
/// unified the two element-resolver lists and the dispatch arm's fallback stayed keyed on a
/// three-name allowlist (`get`/`get_mut`/`get_or_init`), so the twenty names that moved into that arm
/// silently lost the `returns`-index route they had always had. Every one became a purity CLAIM over a
/// caller-supplied trait object. See `candor-denylist-over-allowlist`: when you narrow a sound
/// over-approximation, narrow with a denylist and say which direction it fails in.
///
/// **THESE SEVENTEEN, AND WHY EACH.** They are exactly the pre-R347 DISPATCH list minus the R101 cell
/// accessors — i.e. the set whose fallback was MEASURED bad, not the set that looked risky. Extending
/// the fallback to them cost 4 real rows their `invisible` disclosure across 78 registry crates
/// (sea-query-derive `iden::find_attr`, wit-bindgen-rust `declare_import`, x509-parser 0.17/0.18
/// `find_attribute`) — each an `xs.iter().find(..)` whose closure param stopped resolving through the
/// field route once a local `fn iter` answered for `.iter()`. `returns` is keyed by bare method LEAF
/// crate-wide, so one local `fn iter` answers for EVERY `.iter()` in the crate; these are the names a
/// crate is most likely to define itself.
///
/// Everything else in the union — the R346 iterator adapters, the R345 `as_deref` family, `clone`,
/// `to_vec`, `first`, `last`, and the R101 cell accessors — falls back, which is byte-for-byte the
/// route it took before R347 for the empty-receiver path — though note the arm now tries
/// `resolve_elem_trait_leaves(receiver)` FIRST and short-circuits, which pre-R347 it did not for these
/// names; that ordering is the safe direction (a resolved receiver beats a leaf-keyed guess) but the
/// first draft of this sentence said "byte-for-byte" without the qualifier.
///
/// `the_receiver_only_denylist_is_a_subset_of_the_adapter_union` in the test module asserts BOTH
/// directions: that each of these seventeen IS in the union, and that a name deliberately OUTSIDE the
/// union (`map`, `flat_map`, `zip`, `windows`, …) is not on this denylist. **The first draft of this
/// comment named `partition_is_total`, a symbol that does not exist, and claimed a guarantee the test
/// did not make** — it walked a hardcoded literal, so adding an out-of-union name here passed 430/430.
/// Both were found by review, and both are the class this very function was written to close: a
/// sentence that makes its own diff look correct.
pub(crate) fn is_receiver_only_adapter(method: &str) -> bool {
    matches!(
        method,
        "iter" | "into_iter" | "iter_mut" | "drain" | "as_slice" | "as_mut_slice"
            | "values" | "values_mut"
            | "lock" | "unwrap" | "expect" | "borrow" | "borrow_mut" | "read" | "write"
            | "as_ref" | "as_mut"
    )
}

/// SOUNDNESS R308 — PARSE A FILE, AND IF AND ONLY IF THAT FAILS, RETRY ONCE WITH RUST-2015 BARE
/// CLOSURE-TRAIT OBJECTS NORMALISED.
///
/// **THE DEFECT.** `syn` parses `&dyn Fn(..)` and rejects the 2015 spelling `&Fn(..)`, and a parse
/// failure is per-FILE, so ONE elided `dyn` drops every function in the file. Measured on
/// `serial-core-0.4.0`: 802 lines, 35 bodied fns, the entire crate reports zero rows over
/// `fn reconfigure(&mut self, setup: &Fn(&mut SerialPortSettings) -> ::Result<()>)`. Two-line repro:
/// `&Fn(&mut u8)` beside a `std::fs::read` gives analyzed=0 / rows=0; `&dyn Fn(&mut u8)` gives
/// analyzed=1 / rows=1. The arrow is not required — `&Fn(&mut u8)` alone does it.
///
/// **WHY THIS CANNOT REGRESS A FILE THAT PARSES TODAY, by construction rather than by care.** The
/// rewrite runs ONLY on text `syn` has already rejected. A file that parses is returned from the first
/// attempt and never sees the transform, so the blast radius is exactly the set of files that
/// currently contribute nothing. The worst case for a file in that set is that it still fails, which
/// is where it already was.
///
/// **AND THE PARSER IS THE VALIDATOR.** If the rewrite produced nonsense, `syn` rejects the retry and
/// we fall back — the normalisation cannot smuggle a mis-parse through, only fail to help. It is also
/// semantics-preserving where it fires: in the 2015 edition `Fn(..)` in type position IS `dyn Fn(..)`,
/// which is why rustc's own migration is this same insertion.
///
/// **SCOPE, and why it is not "insert `dyn` before every bare trait".** A general 2015 trait object
/// (`&Error`, `Box<Error>`) cannot be told from a type by syntax, so normalising it would need name
/// resolution. `Fn`/`FnMut`/`FnOnce` followed by `(` are unambiguous — they are traits, never types —
/// which is the whole family the row names and the only one attempted. The match additionally requires
/// a preceding `&`, `<` or `:` so a `Fn(` inside a string literal is not touched.
pub(crate) fn parse_file_2015_tolerant(text: &str) -> Option<(syn::File, bool)> {
    let first = match syn::parse_file(text) {
        Ok(f) => return Some((f, false)),
        Err(e) => e,
    };
    let mut out = String::with_capacity(text.len() + 32);
    let b = text.as_bytes();
    let mut i = 0usize;
    let mut rewrote = false;
    while i < b.len() {
        let rest = &text[i..];
        let hit = ["Fn(", "FnMut(", "FnOnce("]
            .iter()
            .find(|k| rest.starts_with(**k))
            .filter(|_| {
                // TYPE POSITION, approximated by the token that introduces it. Also refuse when the
                // previous non-space char could make this an identifier tail (`MyFn(`) or when `dyn`
                // or `impl` is already there.
                let before = text[..i].trim_end();
                // `=` is the TYPE-ALIAS position — `type Action = Fn(&siginfo_t) + Send + Sync;`,
                // which is signal-hook-registry 1.4.8 lib.rs:140 and was the second real instance,
                // found in seconds by the `PARSEFAIL` diagnostic below after the first one cost an
                // hour of bisecting. It is as unambiguous as the others: `Fn` is a trait, and a trait
                // is the only thing that can follow `=` in a type alias.
                // `:` IS DELIBERATELY NOT HERE, and the reason is measured. It is ambiguous between a
                // parameter type (`setup: &Fn(..)`, a trait object) and a GENERIC BOUND
                // (`F: Fn() + Sync + Send`, where `dyn` is a syntax error) — signal-hook-registry
                // 1.4.8 lib.rs:576 has both in one file. Dropping it costs nothing: every real trait
                // object is already reached by `&`, `<` or `=`, because a bare `x: Fn()` parameter is
                // unsized and does not compile in any edition.
                //
                // The wrong version of this shipped for about a minute and was caught by `syn`
                // rejecting the retry rather than by review — which is the safety property working
                // exactly as designed, and the reason a rewrite that only ever sees already-failing
                // source can afford to be approximate.
                (before.ends_with('&') || before.ends_with('<') || before.ends_with('='))
                    && !before.ends_with("==")
            });
        match hit {
            Some(k) => {
                out.push_str("dyn ");
                out.push_str(k);
                i += k.len();
                rewrote = true;
            }
            None => {
                let c = text[i..].chars().next().unwrap();
                out.push(c);
                i += c.len_utf8();
            }
        }
    }
    if !rewrote {
        report_parse_error(&first, text);
        return None;
    }
    match syn::parse_file(&out) {
        Ok(f) => Some((f, true)),
        Err(e) => {
            report_parse_error(&e, text);
            None
        }
    }
}

/// SOUNDNESS R308 — say WHY a file could not be parsed, when asked.
///
/// The `unanalyzed` entry a failed parse produces reads `"source failed to read/parse"` and carries
/// nothing else, so identifying the construct means bisecting the file by hand — which is how R308 was
/// found and it cost an hour. `syn`'s error already knows the line, the column and the token it choked
/// on; it was simply being dropped with `.ok()`. Behind `CANDOR_ALIAS_DEBUG` (the same switch the §E1
/// hit counters use) rather than in the report, because a parse error is a diagnostic for whoever is
/// extending the scanner, not a fact about the scanned crate's effects.
fn report_parse_error(e: &syn::Error, text: &str) {
    if std::env::var("CANDOR_ALIAS_DEBUG").is_err() {
        return;
    }
    let start = e.span().start();
    let line = text.lines().nth(start.line.saturating_sub(1)).unwrap_or("");
    eprintln!(
        "PARSEFAIL {}:{} — {} | {}",
        start.line,
        start.column,
        e,
        line.trim().chars().take(100).collect::<String>()
    );
}

/// SOUNDNESS R140/R287 — RECORD A SECOND TARGET FOR A NAME BOUND TWICE, in a COMPANION map rather
/// than by making `out` multi-valued.
///
/// Two `use` items can bind one name under mutually-exclusive `#[cfg]`s. `out` is single-valued, so the
/// second silently overwrites the first and the answer is decided by SOURCE ORDER: with the unix arm
/// written first, `run_it` reads pure over a real `Command::new("true").status()`; swapping only those
/// two lines charges `Exec`. That is R140, live at 221 crates / 451 sites.
///
/// **WHY A COMPANION MAP AND NOT A JOINED VALUE IN `out`.** `record_alias` already joins alternatives
/// with `ALIAS_ALT_SEP`, and routing this insert through it was the obvious fix — it is also wrong: the
/// collector reads these maps at 34 sites and only 17 places in the whole crate are join-aware, so a
/// joined value would flow into path construction as a literal and produce malformed paths, which land
/// as fabrication or as silence depending where. Measured before writing this: the obvious fix did not
/// even fire, because `collect_use` has TWO callers and the alias substitution that builds a call's path
/// runs through the OTHER one (R213's double-collection shape, in a second place).
///
/// So `out` keeps exactly the value it has today — every existing reader is untouched — and the
/// collision is recorded beside it. This mirrors `fn_alias`, which already holds a LIST of targets for
/// a `let`-bound function alias and whose call site already pushes every target beyond the first as its
/// own edge so the row carries the UNION. A `use` alias simply never reached that machinery.
///
/// **SOUNDNESS R375 — THE PARAGRAPH ABOVE IS HALF WRONG AND IS KEPT SO THE CORRECTION IS LEGIBLE.** It
/// argues a joined value "would flow into path construction as a literal and produce malformed paths".
/// The companion map did exactly that: `prev` is read straight out of `out`, and `out` ALREADY holds
/// `ALIAS_ALT_SEP`-joined values written by `record_alias` and seeded by `seed_mod_aliases` —
/// `arc-swap` binds `Arc` to `alloc::sync::Arc<SEP>std::sync::Arc`, `axum` binds `FromRef` to a
/// trait/derive-macro pair. The call site then built `format!("{t}::{rest}")` over one, so the suffix
/// reached only the LAST arm and `alloc::sync::Arc` lost its `::new`. Fixed at the point the value
/// ENTERS rather than at each place it is read: the arms are SPLIT here, so `alts` can only ever hold
/// flat single paths and every consumer's concatenation is safe by construction. 7 live sites / 6
/// crates (`jiff` ×2, `redis` ×3, `similar`), all previously inert.
///
/// **SOUNDNESS R370 — THE ARM-SET TEST IS "DID THIS ITEM LIST BIND THE NAME TWICE", which this helper
/// did not have.** It fired on "this name already had a different value in `out`" and nothing more, so
/// ordinary legal SHADOWING was recorded as an alternative and the union charged a binding the code
/// cannot reach: an inline `mod inner { use std::vec::Vec as Cmd; }` under a file-level `use
/// std::process::Command as Cmd` read `["Exec"]` — Rust does not propagate a parent's imports into an
/// inline `mod` at all — and so did a body that DECLARES the name, while rustc itself says
/// `warning: unused import`. Gate teeth measured: `deny Exec` 0→1, bare `pure` 0→1, scoped
/// `deny Exec <fn>` 0→1. 676 of 1,168 collision records over 1,545 registry crates had no `#[cfg]`.
///
/// **A CFG TEST WAS THE FIRST FIX AND IT WAS TOO BLUNT — the corpus priced it and the price is the
/// reason this rule is the one it is.** Requiring a `#[cfg]` also silences a same-file pair that is
/// genuinely two live bindings in two NAMESPACES: `tracing-subscriber`'s `fmt_layer.rs` binds `format`
/// to `crate::fmt::format` (a MODULE) on line 3 and to `alloc::format` (the MACRO) on line 7, source
/// order picks the macro, and `Layer::default` lost three real call edges to `fmt::format::*`. Same
/// shape cost `lettre`, `quinn-proto` and `aws-smithy-types` their blind-spot disclosures. Two `use`
/// items binding one name in ONE list is E0252 unless they are cfg-gated or in different namespaces —
/// and in BOTH of those cases keeping every target is right, so the list is the test and the `#[cfg]`
/// is not. Inheritance is what must be excluded, and `seen` excludes exactly it.
///
/// The narrowing is a DENYLIST on the recording side and it fails in the pre-R140 direction: an
/// inherited rebind goes back to being decided by source order, which is no worse than before this
/// helper existed and strictly better than a fabricated edge with gate teeth.
///
/// R140 — note that `name` now names a SECOND, different target. `out` is left alone; the alternative
/// is appended to `alts`, deduped, with the value `out` already held recorded first so the set is the
/// whole arm set rather than only the losers.
fn note_use_collision(
    out: &HashMap<String, String>,
    alts: &mut HashMap<String, Vec<String>>,
    name: &str,
    target: &str,
) {
    let prev = match out.get(name) {
        Some(p) if p != target => p.clone(),
        _ => return,
    };
    let e = alts.entry(name.to_string()).or_default();
    // R375 — SPLIT, never push a joined value. `prev` can already carry `ALIAS_ALT_SEP` arms.
    let push = |t: &str, e: &mut Vec<String>| {
        for arm in t.split(crate::decls::ALIAS_ALT_SEP) {
            if !arm.is_empty() && !e.iter().any(|x| x == arm) {
                e.push(arm.to_string());
            }
        }
    };
    if e.is_empty() {
        push(&prev, e);
    }
    push(target, e);
}

/// Expand one `use` TREE into `out`, recording `#[cfg]` arm-set collisions in `alts`.
///
/// `seen` is the set of names ALREADY BOUND BY THIS ITEM LIST — the arm-set test. See
/// `note_use_collision`'s R370 paragraph. It is per-SCOPE, so it is threaded through the `Path`/`Group`
/// recursion and accumulated across the sibling `use` items of one list.
pub(crate) fn collect_use(
    tree: &syn::UseTree,
    prefix: String,
    out: &mut HashMap<String, String>,
    alts: &mut HashMap<String, Vec<String>>,
    seen: &mut std::collections::HashSet<String>,
) {
    let join = |p: &str, s: &str| if p.is_empty() { s.to_string() } else { format!("{p}::{s}") };
    // A crate-LOCAL re-bind (`use crate::net`, `use super::net`) names a target in THIS crate. Store what
    // that target ALREADY resolves to in the (inherited) `use` map rather than the literal `crate::net`,
    // which names no local def and reads silent-pure (iso_B): a submodule's `use crate::net` must inherit
    // the crate-root `net` binding — a direct `use mycore::net` (→ `mycore::net`) or a glob re-export
    // (→ `mycore::…::net`). Resolving the FULL rebind path through `expand` (which follows the glob
    // fallback) recovers the origin crate; a target that resolves to nothing local is stored as-is.
    let rebound = |full: &str, out: &HashMap<String, String>| -> String {
        // SOUNDNESS R378 — `super::X` IS RE-RESOLVED ONLY INSIDE AN INLINE MODULE, AND ONLY WHEN `X`
        // IS A NAME THE ENCLOSING MODULE ACTUALLY IMPORTS. Both halves are load-bearing, and each was
        // measured by the corpus rows the other one broke. Inside an INLINE module, `out` starts as the parent's `use` map
        // (`submodule_uses`), so `use super::proc_alias` names a binding that is right there — and
        // storing the literal `super::proc_alias` names no local def, so the origin was LOST and every
        // call through it read silent-pure. Measured: `mod inner { use super::proc_alias; fn via_super()
        // { proc_alias::Command::new("true").status(); } }` reported ABSENT while the identical call in
        // the parent charged `Exec`. Found because R370's cfg gate stopped R140's union from ACCIDENTALLY
        // masking it — the union had been pushing the parent's binding as a second edge, which papered
        // over this for the effect but not for the row.
        //
        // The narrowing is the whole safety argument, and the comment below is why it is needed: a
        // `super::` path is RELATIVE to a module whose path `collect_use` does not know, so re-resolving
        // one blindly drops the module context and breaks tail2's link to a LOCAL def (clap's
        // `super::core::display_width`). Requiring the first segment to be a key in `out` distinguishes
        // the two exactly: `core` is a local MODULE and not a `use` key, so it keeps its literal; a
        // parent's imported alias is a key, and resolving it is the only way to keep its origin.
        //
        // AND THE SCOPE TEST IS THE OTHER HALF, measured after the first version of this shipped without
        // it. `out` is the ENCLOSING module's map only for an INLINE `mod` — `submodule_uses` clones it
        // and plants `SUPER_SCOPE_MARKER`. For a FILE module `out` is that FILE's own map while `super::`
        // still means its parent, which is a different scope entirely. `hyper`'s `proto/h1/conn.rs` binds
        // `io` to `std::io` on line 2 and writes `use super::io::Buffered` on line 19, where `super::io`
        // is hyper's own `proto::h1::io` MODULE: without the marker test that resolved to
        // `std::io::Buffered`, dropped the local edge, and removed 19 rows from `hyper` — two of them
        // carrying a real `Log`. A full-registry A/B caught it, in the REMOVED column.
        // SOUNDNESS R400 — REVERTED 2026-09-14, AND THE REVERT IS THE FINDING.
        //
        // This was briefly `strip exactly one level, refuse beyond`, to kill a measured FABRICATION
        // (grandparent `Vec`, parent `Command`, `super::super::Runner` → a body that constructs a Vec
        // reported `['Exec']`). The justification I wrote was: "the grandparent's binding is not
        // recoverable here, because `submodule_uses` builds each inline module's map FROM its parent's,
        // so the grandparent's binding has already been SHADOWED".
        //
        // **THAT PREMISE IS FALSE, and a pre-release review falsified it at the root.** `decls.rs:244`
        // is `let mut subuses = uses.clone();` — it CLONES the parent's map and removes only the names
        // the inline module itself DECLARES. So `out` is a FLATTENED CHAIN and `out[X]` is the NEAREST
        // ENCLOSING binding of `X`. The grandparent's binding is shadowed only when the parent rebinds
        // that exact name — which is the UNCOMMON case for a program that compiles and spells
        // `super::super::X`. I generalised from my own fabrication fixture, in which the parent DID
        // rebind the name; that is the whole error.
        //
        // Measured, one variable, parent does NOT rebind:
        //     mod b { mod c { use super::super::Command; fn go() { Command::new("true").status(); } } }
        //   pre-refusal: `b::c::go` → ['Exec'], `deny Exec <fn>` exit 1
        //   with refusal: ABSENT, exit 0 — over a real spawn, with a direct-spawn control charging in
        //   the same scan.
        //
        // So the refusal closed the rarer FABRICATION and opened a commoner SILENT UNDER-REPORT, which
        // is the direction this family ranks worst. Reverted to the behaviour that was shipped for
        // months, and R400 is OPEN AGAIN with both halves measured rather than one.
        //
        // THE CORRECT FIX, for whoever takes it: refuse only when the PARENT'S OWN use-list binds the
        // head name — i.e. only when the grandparent really is shadowed. That fact exists at
        // `submodule_uses`'s call site (`decls.rs:153`) and is not threaded through; `out` alone cannot
        // tell an inherited entry from an own one. It is a real change to the recursion and wants its
        // own measured cycle, not a hurried one before a cut.
        let supers = {
            let mut r = full;
            while let Some(t) = r.strip_prefix("super::") { r = t; }
            r
        };
        if !std::ptr::eq(supers, full) && out.contains_key(crate::decls::SUPER_SCOPE_MARKER) {
            let head = supers.split("::").next().unwrap_or(supers);
            if out.contains_key(head) {
                let resolved = expand(supers, out);
                if resolved != supers {
                    return resolved;
                }
            }
            return anchored_value(full, out).unwrap_or_else(|| full.to_string());
        }
        // VEIN A — a `self::`/`super::` VALUE is relative to the module this `use` is written in, and
        // `out` is that module's own map whenever it carries `MODPATH_KEY` (`submodule_uses` strips the
        // key from an inherited map, so a present one is never a parent's). Stored crate-ROOTED so it
        // cannot be re-read against a different module later: an inline child inherits this map by
        // clone, and a `self::a::Tx` read in the child would name the CHILD's `a`, one level off — the
        // R400 shape. An absolute value means the same thing from everywhere. Without it,
        // `fn f_use() { use self::a::Tx; Tx::grab(p) }` stored `self::a::Tx`, which `arm_exact_target`
        // could only strip to the relative `a::Tx::grab`, tied, and R830's `x::f_use` went ABSENT.
        if matches!(full.split("::").next(), Some("self" | "super")) {
            if let Some(a) = anchored_value(full, out) {
                return a;
            }
            // …and `self::X::…` where this module says `extern crate X;` is that EXTERN crate's path,
            // stored as the crate's own spelling (lazy_static's `core_lazy`: `extern crate spin; use
            // self::spin::Once;`). Kept `self::`-headed, every later expansion read it as a local path,
            // the typed `self.0.call_once(..)` named nothing, and the chained join into `spin` went with it.
            if let Some(rest) = full.strip_prefix("self::") {
                let head = rest.split("::").next().unwrap_or(rest);
                if rest.contains("::")
                    && out.get(MODEXTERN_KEY).is_some_and(|l| l.split('\u{1}').any(|n| n == head))
                {
                    return rest.to_string();
                }
            }
        }
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
        syn::UseTree::Path(p) => collect_use(&p.tree, join(&prefix, &p.ident.to_string()), out, alts, seen),
        syn::UseTree::Name(n) => {
            let id = n.ident.to_string();
            if id == "self" {
                // `use a::b::{self, ..}` imports the MODULE `b` itself under name `b` → map `b -> a::b`
                // so a later `b::func()` resolves. Without this, `self` was mapped uselessly as
                // `b::self` and the module alias was lost. (Found on coreutils `ls`: `use std::fs::{self,
                // Metadata}` then `fs::read_dir` was unresolved → a file lister reporting ZERO Fs.)
                if let Some(last) = prefix.rsplit("::").next() {
                    let v = rebound(&prefix, out);
                    if !seen.insert(last.to_string()) {
                        note_use_collision(out, alts, last, &v);
                    }
                    out.insert(last.to_string(), v);
                }
            } else {
                let v = rebound(&join(&prefix, &id), out);
                if !seen.insert(id.clone()) {
                    note_use_collision(out, alts, &id.clone(), &v);
                }
                out.insert(id.clone(), v);
            }
        }
        syn::UseTree::Rename(r) => {
            let v = rebound(&join(&prefix, &r.ident.to_string()), out);
            if !seen.insert(r.rename.to_string()) {
                note_use_collision(out, alts, &r.rename.to_string(), &v);
            }
            out.insert(r.rename.to_string(), v);
        }
        syn::UseTree::Group(g) => {
            for t in &g.items {
                collect_use(t, prefix.clone(), out, alts, seen);
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
            // VEIN A — every glob, local or not, for `glob_origin`'s "exactly one, and not local" test.
            if !prefix.is_empty() {
                let e = out.entry(ALLGLOB_KEY.to_string()).or_default();
                if !e.is_empty() {
                    e.push('\u{1}');
                }
                e.push_str(&prefix);
            }
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

/// VEIN A — a `self::`/`super::` `use` value made crate-root-absolute against the module whose map
/// `out` is, through `expand` so a re-export or alias on the way is followed exactly as for a written
/// path. `None` when `out` does not know its module (no `MODPATH_KEY`) or the path walks above the
/// root — the caller then keeps the literal, which is today's answer.
fn anchored_value(full: &str, out: &HashMap<String, String>) -> Option<String> {
    let segs: Vec<&str> = full.split("::").collect();
    absolutise(&segs, out)?;
    // NOT alias-followed: the value names what the `use` names. Following a local type alias here
    // would bake the TARGET into the binding and leave no way back to an `impl` written on the alias
    // itself (see `expand_noalias`); a later `expand` of a path through this binding still follows it.
    Some(expand_noalias(full, out))
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
    // R140 — a re-export collision is a DIFFERENT question (glob fan-out, R190's territory), so this
    // caller discards the companion map rather than pretending to answer it. Scoping the change to the
    // `use`-alias case is deliberate: it is the one with a measured defect and a measured fixture.
    let mut _alts = HashMap::new();
    collect_item_uses(items, include_tests, &mut m, &mut _alts);
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
    // SOUNDNESS R140 — this function's own doc calls itself "THE ONE RULE for does this `use` item bind
    // a name in the build we are describing", and it answered only the `test` half. `is_cfg_inactive`
    // is the authority for the OTHER half and already exists: a `#[cfg(feature = "x")]` on a feature
    // that is DECLARED and INACTIVE is `Some(false)`, so the item binds nothing in this build and
    // keeping it made a second arm compete with the real one.
    //
    // This is the DECIDABLE half only. `cfg_eval` deliberately returns `None` for `unix`, `windows` and
    // `target_os` — a source scan does not assume a target — so a platform-gated pair is genuinely
    // undecidable here and is handled the other way, by `record_alias` keeping BOTH arms so the answer
    // stops depending on source order. Dropping a decidable arm and hedging an undecidable one are the
    // two halves of one fix, and neither is sufficient alone: without this, an inactive feature arm
    // would turn a precise answer into `Unknown`; without the join, a platform arm would still be
    // decided by whichever line was written last.
    if is_cfg_inactive(&u.attrs) {
        return false;
    }
    include_tests || !is_cfg_test(&u.attrs)
}

/// Collect every module-level `use` that [`use_item_applies`] admits into `out`.
pub(crate) fn collect_item_uses(
    items: &[syn::Item],
    include_tests: bool,
    out: &mut HashMap<String, String>,
    alts: &mut HashMap<String, Vec<String>>,
) {
    // R370 — the arm-set test is "THIS ITEM LIST bound the name twice", accumulated across the sibling
    // `use` items of one scope. A name already in `out` because it was INHERITED (an inline module's
    // map is its parent's, via `submodule_uses`) is being SHADOWED, not alternated with.
    let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
    let mut off: HashMap<String, String> = HashMap::new();
    for it in items {
        if let syn::Item::Use(u) = it {
            if use_item_applies(u, include_tests) {
                collect_use(&u.tree, String::new(), out, alts, &mut seen);
            } else if (include_tests || !is_cfg_test(&u.attrs)) && is_cfg_inactive(&u.attrs) {
                // R982 — kept aside, under `CFG_OFF_USE_PREFIX`, for `expand` inside a `CfgOffScope`.
                let mut a2 = HashMap::new();
                let mut s2 = std::collections::HashSet::new();
                collect_use(&u.tree, String::new(), &mut off, &mut a2, &mut s2);
            }
        }
    }
    for (k, v) in off {
        if !k.starts_with(['*', '\u{1}']) {
            out.entry(format!("{CFG_OFF_USE_PREFIX}{k}")).or_insert(v);
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
/// SOUNDNESS R503 — the MODULE-QUALIFIED path of every trait this file DECLARES (`backend` +
/// `Backend` → `backend::Backend`), keyed by leaf.
///
/// WHY IT EXISTS. `trait_decls`/`trait_impls` are keyed by trait LEAF throughout this engine, which is
/// fine for CHA (the ambiguity count guards the collision) and is NOT fine for a WIRE key. SPEC §4
/// ⟨0.39⟩ pins both obligation 2's key and `dispatchesOn`'s value to the ⟨0.23⟩ rule — *fully qualified
/// in the OWNING package's namespace, the namespace that package's entry hashes use* — and names the
/// leaf-abbreviated spelling (`ratatui_core#Backend::size`) as the second spelling the clause forbids.
/// The FOREIGN half of that key comes free from the implementing file's `use` map
/// (`collect_foreign_trait_impls`); the LOCAL half has no such source, because a crate does not `use`
/// its own trait by its full path. This walk is that source.
///
/// A leaf declared twice (two modules, one name) records BOTH quals. Every consumer already refuses an
/// ambiguous leaf (`LocalTrait::count > 1`), and a set with two members is the same refusal one field
/// over — never a guess between two traits.
pub(crate) fn collect_trait_decl_quals(
    items: &[syn::Item],
    modpath: &str,
    include_tests: bool,
    out: &mut HashMap<String, std::collections::BTreeSet<String>>,
) {
    for it in items {
        match it {
            // NO `#[cfg(test)]` FILTER ON THE TRAIT ITSELF, and that is not an oversight: `collect_decls`
            // records EVERY `Item::Trait` into `trait_decls` and filters only the enclosing MODULE. A
            // filter here that the index does not share would leave a leaf present in `trait_decls` with
            // no qual beside it, and `union_member_key` refuses such a leaf — so a `#[cfg(test)]` trait
            // would silently DELETE its crate's interface-union entry. Two walks answering one question
            // must admit the same items; this comment is the reason they do.
            syn::Item::Trait(t) => {
                let leaf = t.ident.to_string();
                let qual = if modpath.is_empty() { leaf.clone() } else { format!("{modpath}::{leaf}") };
                out.entry(leaf).or_default().insert(qual);
            }
            syn::Item::Mod(m) if include_tests || !is_cfg_test(&m.attrs) => {
                if let Some((_, inner)) = &m.content {
                    let sub = if modpath.is_empty() {
                        m.ident.to_string()
                    } else {
                        format!("{modpath}::{}", m.ident)
                    };
                    collect_trait_decl_quals(inner, &sub, include_tests, out);
                }
            }
            _ => {}
        }
    }
}

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

/// SOUNDNESS R452 — TWO facts about what an unexpanded macro hid in this file: the TYPE names declared
/// inside a module `collect_macro_modules` just marked (a module whose item list candor could NOT read
/// in full), and the `fn` NAMES that appear inside any unexpanded macro text at all.
///
/// WHY A SECOND INDEX AND NOT `local_types`. `local_types` is built in Pass B from FUNCTION quals, so a
/// type whose every `impl` sits inside an unexpanded item-position macro is absent from it — and the
/// call `s.deregister(3)` on such a type is then not even *attempted*: `resolvable` is false, the whole
/// resolution block is skipped, and the call leaves NO edge, NO `Unknown` and NO reason. That is R452's
/// measured fixture and mio-0.8.11's `cfg_os_poll!` shape.
///
/// WHY IT IS KEYED ON THE TYPE AND NOT ON THE CALL PATH, which is what R128 does. A receiver-typed
/// method call arrives at the resolver as `Type::method` — the collector formed it from the receiver's
/// TYPE, so it carries no module and `macro_hidden_owner` (which needs one, and needs a `crate::` head)
/// can never answer for it. That is the half of R452's premise that does not hold: the engine has the
/// disclosure VOCABULARY for this state but not the EVIDENCE, because the evidence R128 uses is a
/// module qualifier this call shape does not have. Keying on the declared type restores it.
///
/// WHY BOTH, AND WHY THE SECOND IS NOT OPTIONAL — MEASURED. The module fact alone condemns every type
/// in the module: over 250 registry crates it hedges 581 caller functions, and the sample is dominated
/// by `BigDecimal::unwrap`, `SmallIndex::expect`, `Unstructured::collect`, `Vec::add` — std combinators
/// on a receiver this engine typed wrongly, where nothing was hidden at all. Requiring the METHOD NAME
/// to appear as `fn <name>` inside unexpanded macro text narrows that to 95 callers and keeps the real
/// catches: aho-corasick's `StateID::as_usize`/`as_u32` and `PatternID::as_usize` (declared by
/// `index_type_impls!`) and bitflags' `Flag::bits`. Both conditions NARROW a sound over-approximation on
/// a named fact, which is the denylist direction; neither is an allowlist of shapes permitted to hedge.
///
/// THE BOUNDARY, STATED. Only a type whose DECLARATION candor could read is here — a `struct` declared
/// *inside* the macro body (mio's own `IoSourceState`) is invisible to this walk exactly as its impls
/// are, and stays an under-report. And the module set is this FILE's, so a type declared in a readable
/// module whose `impl` lives in a macro-hidden module elsewhere is not covered either. Both are misses,
/// both are stated; neither is a guess.
pub(crate) fn collect_macro_hidden_decls(
    items: &[syn::Item],
    modpath: &str,
    include_tests: bool,
    macro_modules: &std::collections::HashSet<String>,
    types: &mut std::collections::HashSet<String>,
    fns: &mut std::collections::HashSet<String>,
) {
    let hidden = macro_modules.contains(modpath);
    for it in items {
        if hidden {
            match it {
                syn::Item::Struct(s) if include_tests || !is_cfg_test(&s.attrs) => {
                    types.insert(s.ident.to_string());
                }
                syn::Item::Enum(e) if include_tests || !is_cfg_test(&e.attrs) => {
                    types.insert(e.ident.to_string());
                }
                syn::Item::Union(u) if include_tests || !is_cfg_test(&u.attrs) => {
                    types.insert(u.ident.to_string());
                }
                _ => {}
            }
        }
        // The `fn` NAMES the unexpanded text mentions — from an item-position INVOCATION's arguments
        // (`os_only! { impl Sel { pub fn deregister(..) } }`, mio's `cfg_os_poll!` shape, where the items
        // are at the call site) AND from a `macro_rules!` BODY (`index_type_impls!` declares
        // `fn as_usize` inside itself and the invocation names only the type). Both are collected, in
        // any module: a macro defined in one module and invoked in another is ordinary, and this index
        // answers "did candor SEE this fn name inside something it could not expand", which is a fact
        // about the text rather than about where it sits.
        if let syn::Item::Macro(m) = it {
            if include_tests || !is_cfg_test(&m.attrs) {
                let toks = m.mac.tokens.to_string();
                let mut prev_fn = false;
                for t in toks.split(|c: char| !(c.is_alphanumeric() || c == '_')) {
                    if prev_fn && !t.is_empty() {
                        fns.insert(t.to_string());
                    }
                    if !t.is_empty() {
                        prev_fn = t == "fn";
                    }
                }
            }
        }
        if let syn::Item::Mod(m) = it {
            if include_tests || !is_cfg_test(&m.attrs) {
                if let Some((_, inner)) = &m.content {
                    let sub = if modpath.is_empty() {
                        m.ident.to_string()
                    } else {
                        format!("{modpath}::{}", m.ident)
                    };
                    collect_macro_hidden_decls(inner, &sub, include_tests, macro_modules, types, fns);
                }
            }
        }
    }
}

/// ⟨0.39⟩ SPEC §4 obligation 2 — the FOREIGN abstractions this file implements, keyed under the package
/// that OWNS the abstraction rather than under this one.
///
/// `"<owning crate>#<trait qual>::<method>"  ->  the LOCAL impl method quals (`Crossterm::size`) that
/// implement it`. The key spelling is NOT a new convention: it is the ⟨0.23⟩ `typeSurface` rule — fully
/// qualified in the OWNING package's namespace, the same namespace that package's own entry hashes use —
/// so the consumer's ORDINARY chained lookup (`{crate}#{tail2}`) resolves it with no special case, and
/// `ratatui_core#Backend::size` is that rule's rust spelling rather than a second one.
///
/// WHY THIS LEG CANNOT BE DROPPED. In the measured instance (SOUNDNESS R475, live on `ratatui`) the
/// effectful implementor lives in a THIRD package — neither the dispatching dependency nor the consumer —
/// so a producer that publishes a union only over its OWN traits misses it entirely: `ratatui-core` sees
/// one pure `TestBackend`, `ratatui-crossterm` sees a trait it does not own, and the consumer chained onto
/// both is told nothing.
///
/// PROVENANCE, not shape, is the gate: the trait path must expand (through this scope's `use` map) to a
/// path rooted at something that is NOT std/`crate`/`self`/`super`. A local module spelled like a crate
/// (`impl mymod::Tr for X`) survives that test and is filtered at emission against the manifest's real
/// dependency set, where the answer is known — a bogus key here is inert wire noise, but it is cheaper to
/// refuse it than to explain it.
/// SOUNDNESS R529 / SPEC §4 ⟨0.39⟩ obligation 2 — THE ONE PLACE A FOREIGN ABSTRACTION'S WIRE KEY IS
/// FORMED. `(owning crate, trait qual)` for an `impl <path> for T`, or `None` when the path does not
/// root at a genuine dependency crate (std/`crate`/`self`/`super` are ours, and a single-segment path
/// names no crate at all).
///
/// TWO WALKS ASK THIS: `collect_foreign_trait_impls` (item level) and `collect_block_nested_trait_impls`
/// (block depth >= 1). They ask ONE function rather than each running `expand` + `split_once` +
/// `is_dependency_crate_root` themselves, because a key spelled two ways inside one engine is the
/// ⟨0.34⟩ drift the clause exists to forbid — and the two spellings would have to MATCH for the
/// R529 hedge to find the entry obligation 2 published. Two paths computing one fact are free to
/// disagree; this is the one path.
pub(crate) fn foreign_trait_owner_qual(
    tr: &syn::Path,
    uses: &HashMap<String, String>,
) -> Option<(String, String)> {
    // The trait as WRITTEN, then expanded through this scope's `use` map — both spellings
    // (`use iface::Backend; impl Backend for X` and `impl iface::Backend for X`) must form the same
    // key, which is R6's lesson on the receiver side.
    let written: String =
        tr.segments.iter().map(|s| s.ident.to_string()).collect::<Vec<_>>().join("::");
    let full = expand(&written, uses);
    let (root, qual) = full.split_once("::")?;
    is_dependency_crate_root(root).then(|| (root.to_string(), qual.to_string()))
}

pub(crate) fn collect_foreign_trait_impls(
    items: &[syn::Item],
    include_tests: bool,
    uses: &HashMap<String, String>,
    out: &mut HashMap<String, Vec<String>>,
) {
    for it in items {
        match it {
            syn::Item::Impl(im) if include_tests || !is_cfg_test(&im.attrs) => {
                let Some((None, tr, _)) = &im.trait_ else { continue };  // `impl !Tr for X` is not an impl
                let Some(ty) = impl_type_name(&im.self_ty) else { continue };
                // ONE AUTHORITY for the key — see `foreign_trait_owner_qual`.
                let Some((root, qual)) = foreign_trait_owner_qual(tr, uses) else { continue };
                for ii in &im.items {
                    if let syn::ImplItem::Fn(m) = ii {
                        let method = m.sig.ident.to_string();
                        out.entry(format!("{root}#{qual}::{method}"))
                            .or_default()
                            .push(format!("{ty}::{method}"));
                    }
                }
            }
            syn::Item::Mod(m) if include_tests || !is_cfg_test(&m.attrs) => {
                if let Some((_, inner)) = &m.content {
                    let _cfg_off = crate::lang::CfgOffScope::enter_if(&m.attrs); // R977 — see `lang::CfgOffScope`
                    // The inner module's OWN imports on top of this scope's — the same widening
                    // `collect_decls` does for a nested module, so an `impl` written beside its own
                    // `use` resolves the same way it would at file level.
                    let mut sub = uses.clone();
                    let mut alts = HashMap::new();
                    collect_item_uses(inner, include_tests, &mut sub, &mut alts);
                    collect_foreign_trait_impls(inner, include_tests, &sub, out);
                }
            }
            _ => {}
        }
    }
}

/// SOUNDNESS R598 — WHICH MEMBERS AN `impl Trait for Ty` BLOCK ACTUALLY DECLARES.
///
/// `decls.rs` records the CHA edge (`trait_impls[trait leaf] += ty leaf`) and throws the block's
/// contents away, so every consumer asking "what does `Ty` do for `Trait::m`" has to spell the unit
/// `{ty}::{method}` — the SAME qual an inherent `impl Ty { fn m }` produces. The interface-union read
/// that as the implementation and charged an inherent method's effects to the trait member: on
/// hickory-proto, `serialize::binary::BinEncodable::to_bytes` publishes a `Log` that comes from the
/// PRIVATE inherent `RData::to_bytes` (`rr/record_data.rs:839`, a `warn!`), while `impl BinEncodable
/// for RData` (`:1133`) declares `emit` alone — so no dispatch through that member can reach it.
///
/// THE FOREIGN TWIN OF THIS FACT ALREADY EXISTS AND IS ALREADY MEMBER-PRECISE: `collect_foreign_trait_impls`
/// above iterates `im.items` and keys `{owner}#{qual}::{method} -> {ty}::{method}` per DECLARED member,
/// so obligation 2's leg cannot take this defect. The asymmetry was the bug — the same shape as R513,
/// where the local half of this rung was missing the tail resolution the foreign half already had. This
/// walk is the local half, written against the same `items`, the same `Item::Mod` recursion and the same
/// `is_cfg_test` guard so the two cannot answer one question two ways (§G).
///
/// IT IS DELIBERATELY NOT A MEMBER SET BUT AN EVIDENCE SET — see `model::impl_seen_key` for the three
/// key shapes and why the `*` (opaque) one exists. Narrowing a sound over-approximation on an index that
/// can be incomplete is the denylist/allowlist hazard, and the direction it fails in is silence; so the
/// consumer narrows only where this walk positively saw the block AND could read every item in it.
///
/// The one asymmetry with `collect_decls`, stated rather than left to be discovered: that walk does NOT
/// skip a `#[cfg(test)]` impl, and this one does. It can only make this index SMALLER than the CHA
/// universe, i.e. withhold a narrowing — never license one.
///
/// ⟨0.40⟩ SOUNDNESS R652 — THE SAME WALK ALSO RECORDS **WHICH TRAIT PATH** EACH BLOCK WROTE, as the
/// fourth key shape (`model::impl_local_trait_key`). One walk, two facts, for the R128/R529 reason:
/// both are read by the SAME interface-union loop about the SAME `impl` block, and two walks free to
/// disagree about which blocks exist is how `trait_impls`' leaf collision got published as a purity
/// claim in the first place. It needs the file's assembled `use` map, which is why this function now
/// takes one and widens it per module exactly as `collect_foreign_trait_impls` does — `use std::io::Write;
/// impl Write for W` and `impl std::io::Write for W` must record one string, not two (R6).
pub(crate) fn collect_local_impl_members(
    items: &[syn::Item],
    include_tests: bool,
    uses: &HashMap<String, String>,
    out: &mut std::collections::BTreeSet<String>,
) {
    for it in items {
        match it {
            syn::Item::Impl(im) if include_tests || !is_cfg_test(&im.attrs) => {
                let Some((None, tr, _)) = &im.trait_ else { continue }; // `impl !Tr for X` is not an impl
                let Some(ty) = impl_type_name(&im.self_ty) else { continue };
                let Some(seg) = tr.segments.last() else { continue };
                // The TRAIT LEAF, which is what `trait_impls`/`trait_decls` are keyed by throughout this
                // engine and therefore the only spelling a consumer of those indexes can ask with.
                // SOUNDNESS R828 — the RESOLVED leaf, the same one `collect_decls` files the CHA edge
                // under, so a renamed import's members are found under the trait they implement.
                let tr_leaf = impl_trait_leaf(tr, uses).unwrap_or_else(|| seg.ident.to_string());
                out.insert(crate::model::impl_seen_key(&tr_leaf, &ty));
                // ⟨0.40⟩ SOUNDNESS R652 — AND WHICH TRAIT PATH THIS BLOCK ACTUALLY WROTE.
                //
                // `decls.rs` files the CHA edge under the trait LEAF with no locality test at all
                // (`trait_impls.entry(leaf.ident.to_string()).or_default().push(ty)`), so a crate that
                // declares its own `trait Write` and ALSO writes `impl std::io::Write for W` hands the
                // LOCAL trait an implementor vector naming a type that does not implement it. Recording
                // the expanded path is what lets `scan.rs` tell the two apart.
                //
                // EVIDENCE, NOT A VERDICT — see `model::impl_local_trait_key` for why the classification
                // is NOT done here. `expand` drops a `crate::` prefix, so a crate-local
                // `use crate::de::Deserializer` arrives as `de::Deserializer`, indistinguishable BY ROOT
                // from a dependency called `de`; only the trait's own declaration qual settles it, and
                // that index is crate-wide. Expansion still happens here because it needs THIS scope's
                // `use` map: `use std::io::Write; impl Write for W` and `impl std::io::Write for W` must
                // record the same string (R6).
                let written: String =
                    tr.segments.iter().map(|sg| sg.ident.to_string()).collect::<Vec<_>>().join("::");
                out.insert(crate::model::impl_local_trait_key(
                    &tr_leaf, &ty, &expand(&written, uses)));
                for ii in &im.items {
                    match ii {
                        syn::ImplItem::Fn(m) => {
                            out.insert(crate::model::impl_member_key(&tr_leaf, &ty, &m.sig.ident.to_string()));
                        }
                        // An associated const or type cannot introduce a METHOD, so neither blinds the
                        // member list.
                        syn::ImplItem::Const(_) | syn::ImplItem::Type(_) => {}
                        // A macro item (`impl Tr for T { forward_all!(); }`), a `Verbatim` this syn
                        // version could not parse, or a variant added to `syn` after this match was
                        // written: any of them may expand to members, so the block stops being evidence
                        // of ABSENCE. Charging on is the pre-existing (over-approximating) answer.
                        _ => {
                            out.insert(crate::model::impl_opaque_key(&tr_leaf, &ty));
                        }
                    }
                }
            }
            syn::Item::Mod(m) if include_tests || !is_cfg_test(&m.attrs) => {
                if let Some((_, inner)) = &m.content {
                    let _cfg_off = crate::lang::CfgOffScope::enter_if(&m.attrs); // R977 — see `lang::CfgOffScope`
                    // The inner module's OWN imports on top of this scope's — the same widening
                    // `collect_foreign_trait_impls` does, and for the same reason: an `impl` written
                    // beside its own `use` must classify the way it would at file level (R652).
                    let mut sub = uses.clone();
                    let mut alts = HashMap::new();
                    collect_item_uses(inner, include_tests, &mut sub, &mut alts);
                    collect_local_impl_members(inner, include_tests, &sub, out);
                }
            }
            _ => {}
        }
    }
}

/// SOUNDNESS R529 — THE TRAIT IMPLS THAT LIVE INSIDE A BLOCK, which every Pass A decl walk is blind to.
///
/// `collect_decls`, `collect_foreign_trait_impls` and `collect_trait_decl_quals` all walk `items` and
/// recurse through `Item::Mod` and NOTHING ELSE. An `impl Trait for Type` written inside a function body
/// — `fn register() -> Box<dyn Backend> { struct L; impl Backend for L { fn size(&self) { …net… } } … }`
/// — is a `Stmt::Item`, so it reaches none of those indexes. Pass B is the opposite: `rebind_self`
/// (R175) exists precisely because the COLLECTOR does walk into bodies, so the impl's effects are charged
/// to the ENCLOSING function by syntactic containment and the implementor's own `Type::method` is never
/// minted as a unit. The two halves disagree, and the disagreement is silent in the certifying direction:
/// the CHA universe contains every implementor except the one it cannot name, so a dispatch over a trait
/// whose only OTHER visible implementor is pure resolves to that pure body and the row claims purity.
/// Measured: a `dyn Backend` dispatch with one module-level pure impl and one body-local `Net` impl
/// reports `inferred: []` with no `Unknown` and no `invisible`, and `deny Net` exits 0 — and it does so
/// with a chained dep report too, which is the ⟨0.39⟩ rung's own toggle reached one spelling over.
///
/// THIS INDEX IS A HEDGE, NOT AN IMPLEMENTOR SET. The body-local method has no unit, so adding it to
/// `trait_impls` would add an edge to nothing (R452's "typed call that resolved to NO UNIT", which that
/// row deliberately does NOT hedge in general) and would also move the ≤12 bound and the ambiguity count.
/// What is recorded is the narrowest fact that licenses a disclosure: the `(trait, member)` pairs this
/// crate implements in a position Pass A cannot read. Same shape as R452's `macro_hidden_types` /
/// `macro_hidden_fns` gate — a named fact about THIS crate, never a blanket hedge on every dispatch.
///
/// THREE OUTPUTS, because the consumers differ: `local` is `"{trait leaf}::{method}"` (the leaf is what
/// `trait_impls`/`local_traits` are keyed by), `foreign` is `collect_foreign_trait_impls`'s own
/// `"{owner}#{trait qual}::{method}"` key — so the ⟨0.39⟩ union emission and the consumer-side join both
/// ask in the spelling they already use — and `externs` is R529b's `extern "C" { fn … }` names, which
/// join the crate-wide `extern_fns` leaf set rather than forming an index of their own.
/// (This sentence said TWO until R529b added the third output in the same walk. A doc comment that goes
/// stale reads as CONSIDERED, which is what stops it being checked — see `feedback-documented-limitation`.)
///
/// THE BOUNDARY, STATED. Per-MEMBER, so a trait method the body-local impl does not override (a default
/// body, which IS visible) is untouched. The set is keyed by trait LEAF for the local half, which is the
/// granularity every trait index in this engine already uses — so two distinct local traits sharing a
/// leaf hedge each other's dispatches. That is over-disclosure, in the same direction and for the same
/// reason `LocalTrait::count > 1` refuses to choose between them. An impl the walk cannot READ at all —
/// one inside an unexpanded macro — is NOT here and stays R128/R452's subject, not this one.
pub(crate) fn collect_block_nested_trait_impls(
    items: &[syn::Item],
    include_tests: bool,
    uses: &HashMap<String, String>,
    local: &mut std::collections::BTreeSet<String>,
    foreign: &mut std::collections::BTreeSet<String>,
    externs: &mut std::collections::BTreeSet<String>,
) {
    for it in items {
        if let syn::Item::Mod(m) = it {
            if !include_tests && is_cfg_test(&m.attrs) {
                continue;
            }
            if let Some((_, inner)) = &m.content {
                let _cfg_off = crate::lang::CfgOffScope::enter_if(&m.attrs); // R977 — see `lang::CfgOffScope`
                // The inner module's OWN imports on top of this scope's — the same widening
                // `collect_foreign_trait_impls` does, so one `impl` keys identically from both walks.
                let mut sub = uses.clone();
                let mut alts = HashMap::new();
                collect_item_uses(inner, include_tests, &mut sub, &mut alts);
                collect_block_nested_trait_impls(inner, include_tests, &sub, local, foreign, externs);
            }
            continue;
        }
        let mut v = NestedImplWalk { include_tests, uses, local, foreign, externs, depth: 0 };
        syn::visit::Visit::visit_item(&mut v, it);
    }
}

/// The block-depth walk behind `collect_block_nested_trait_impls`. An `ItemImpl` seen at depth 0 is one
/// Pass A already indexed; one seen at depth ≥ 1 is inside SOME block — a fn body, an impl method, a
/// trait default, a `const`/`static` initializer, a bare `let x = { … }` — and is exactly what is missing.
struct NestedImplWalk<'a> {
    include_tests: bool,
    uses: &'a HashMap<String, String>,
    local: &'a mut std::collections::BTreeSet<String>,
    foreign: &'a mut std::collections::BTreeSet<String>,
    /// SOUNDNESS R529b — the THIRD fact this one walk answers: the `extern "C" { fn … }` names declared
    /// inside a block. `collect_decls`'s `Item::ForeignMod` arm is item-level like every other, so
    /// `fn wrap() { extern "C" { fn ffi(); } unsafe { ffi(); } }` records NO name and the call falls
    /// through to silent-pure — while the module-level spelling of the SAME program discloses
    /// `Unknown` + `native:extern fn`. One walk, three facts, computed from the same items so they
    /// cannot drift apart (the R128+R452 pattern).
    externs: &'a mut std::collections::BTreeSet<String>,
    depth: usize,
}

impl NestedImplWalk<'_> {
    fn record(&mut self, im: &syn::ItemImpl) {
        let Some((None, tr, _)) = &im.trait_ else { return }; // `impl !Tr for X` is not an impl
        let Some(leaf) = tr.segments.last().map(|s| s.ident.to_string()) else { return };
        // ONE AUTHORITY for the key, shared with `collect_foreign_trait_impls` — and not merely for
        // tidiness: the hedge below only finds the entry obligation 2 published if the two walks agree
        // on the spelling, so a second copy here is a silent-purity bug waiting on one of them moving.
        if let Some((root, qual)) = foreign_trait_owner_qual(tr, self.uses) {
            for ii in &im.items {
                if let syn::ImplItem::Fn(m) = ii {
                    self.foreign.insert(format!("{root}#{qual}::{}", m.sig.ident));
                }
            }
            return; // a foreign abstraction is not also a local trait leaf
        }
        for ii in &im.items {
            if let syn::ImplItem::Fn(m) = ii {
                self.local.insert(format!("{leaf}::{}", m.sig.ident));
            }
        }
    }
}

impl<'ast> syn::visit::Visit<'ast> for NestedImplWalk<'_> {
    fn visit_block(&mut self, b: &'ast syn::Block) {
        self.depth += 1;
        syn::visit::visit_block(self, b);
        self.depth -= 1;
    }
    fn visit_item_mod(&mut self, m: &'ast syn::ItemMod) {
        // A module at depth 0 is the CALLER's job (it widens the `use` map first). One inside a BODY is
        // walked here — and it gets the same widening, rather than the enclosing scope's map. That is
        // not a nicety: a body-local `mod m { use other::Trait; impl Trait for X { … } }` under an outer
        // `use somedep::Trait` would otherwise expand through the OUTER binding and record a key naming
        // the wrong owner. The failure would be over-disclosure rather than silence, which is the safe
        // direction — but it would be a WRONG name, and this file's own R6/R503 lessons are that two
        // spellings of one abstraction is the expensive kind of wrong.
        if self.depth == 0 || (!self.include_tests && is_cfg_test(&m.attrs)) {
            return;
        }
        let Some((_, inner)) = &m.content else { return };
        let _cfg_off = crate::lang::CfgOffScope::enter_if(&m.attrs); // R977 — see `lang::CfgOffScope`
        let mut sub = self.uses.clone();
        let mut alts = HashMap::new();
        collect_item_uses(inner, self.include_tests, &mut sub, &mut alts);
        let mut v = NestedImplWalk {
            include_tests: self.include_tests,
            uses: &sub,
            local: &mut *self.local,
            foreign: &mut *self.foreign,
            externs: &mut *self.externs,
            depth: self.depth,
        };
        for it in inner {
            syn::visit::Visit::visit_item(&mut v, it);
        }
    }
    fn visit_item_impl(&mut self, im: &'ast syn::ItemImpl) {
        if self.depth > 0 && (self.include_tests || !is_cfg_test(&im.attrs)) {
            self.record(im);
        }
        syn::visit::visit_item_impl(self, im);
    }
    /// R529b — a block-nested `extern "C" { fn … }`. The recorded NAME joins the crate-wide
    /// `extern_fns` leaf set the ordinary arm feeds, so the safe wrapper around it discloses `Unknown`
    /// plus `native:extern fn` exactly as the module-level spelling already does.
    ///
    /// Direction: over-disclosure on a leaf collision (a local `fn ffi` elsewhere sharing the name),
    /// which is the same residual the item-level arm has carried since it was written.
    fn visit_item_foreign_mod(&mut self, fm: &'ast syn::ItemForeignMod) {
        if self.depth > 0 && (self.include_tests || !is_cfg_test(&fm.attrs)) {
            for fi in &fm.items {
                if let syn::ForeignItem::Fn(f) = fi {
                    // §E1 REACH PROBE — a byte-identical A/B is not evidence the branch ran.
                    if std::env::var_os("CANDOR_R529_INSTR").is_some() {
                        eprintln!("R529HIT\tEXTERN\t{}", f.sig.ident); // §E1 REACH PROBE
                    }
                    self.externs.insert(f.sig.ident.to_string());
                }
            }
        }
        syn::visit::visit_item_foreign_mod(self, fm);
    }
}

struct BlockStructWalker {
    include_tests: bool,
    depth: usize,
    out: Vec<syn::Item>,
}
impl<'ast> syn::visit::Visit<'ast> for BlockStructWalker {
    fn visit_block(&mut self, b: &'ast syn::Block) {
        self.depth += 1;
        syn::visit::visit_block(self, b);
        self.depth -= 1;
    }
    fn visit_item_mod(&mut self, m: &'ast syn::ItemMod) {
        if self.depth > 0 || (!self.include_tests && is_cfg_test(&m.attrs)) {
            return;
        }
        syn::visit::visit_item_mod(self, m);
    }
    fn visit_item_fn(&mut self, f: &'ast syn::ItemFn) {
        if !self.include_tests && (is_cfg_test(&f.attrs) || is_test_attr_fn(&f.attrs)) {
            return;
        }
        syn::visit::visit_item_fn(self, f);
    }
    fn visit_item_impl(&mut self, im: &'ast syn::ItemImpl) {
        if !self.include_tests && is_cfg_test(&im.attrs) {
            return;
        }
        syn::visit::visit_item_impl(self, im);
    }
    fn visit_item_struct(&mut self, st: &'ast syn::ItemStruct) {
        if self.depth > 0 && (self.include_tests || !is_cfg_test(&st.attrs)) {
            self.out.push(syn::Item::Struct(st.clone()));
        }
        syn::visit::visit_item_struct(self, st);
    }
}

/// SOUNDNESS R529c — every `struct` declared INSIDE A BLOCK (a fn body, a method, a `const` initializer),
/// cloned out as items so `collect_decls`' own struct arm can type its fields — one authority for how a
/// field's type is read, not a second copy. Pass A's decl walk recurses through `Item::Mod` and nothing
/// else, so `fn f() { struct H { c: Inner } let h = H { c: Inner }; h.c.go(); }` had no `fields` entry for
/// `H`, `h.c` typed to nothing, and the caller read PURE over `Inner::go`'s write. A module (`mod`) inside a
/// block is not entered: its items see that module's own imports, which this walk does not assemble.
pub(crate) fn collect_block_local_structs(items: &[syn::Item], include_tests: bool) -> Vec<syn::Item> {
    let mut w = BlockStructWalker { include_tests, depth: 0, out: Vec::new() };
    for it in items {
        syn::visit::Visit::visit_item(&mut w, it);
    }
    w.out
}

/// SOUNDNESS R529c — the block-local structs of ONE fn body, by the same walker as
/// `collect_block_local_structs` (the body's block is depth 1, so every struct in it and in any nested
/// block, nested fn or impl body counts — the collector walks all of those into this one unit).
pub(crate) fn collect_body_local_structs(block: &syn::Block, include_tests: bool) -> Vec<syn::Item> {
    let mut w = BlockStructWalker { include_tests, depth: 0, out: Vec::new() };
    syn::visit::Visit::visit_block(&mut w, block);
    w.out
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

/// TYPE-shaped, distinguishing `Guard` from a `snake_case` local and a `SCREAMING_SNAKE` const. THE
/// ONE COPY — SOUNDNESS R722. A single CHARACTER counts (`struct S;`), counted in `chars()` not bytes
/// so a non-ASCII single-codepoint ident (`struct É;`) is not read as multi-segment.
///
/// R722: the rule used to be *upper-initial AND (one char OR contains a lowercase)*, and the lowercase
/// requirement means an ALL-CAPS TYPE NAME IS NOT A TYPE AT ALL. `IO`, `BSTR`, `HSTRING`, `UTF8`, `O3`
/// all failed it, so `let _g = IO { n: 1 }` with `impl Drop for IO { fs::remove_file }` was ABSENT from
/// `functions[]` while the byte-identical `Io` control charged `['Fs']` — a silent under-report decided
/// by nothing but the case of a name. `windows-core`'s `BSTR`/`HSTRING`/`VARIANT`/`PROPVARIANT` are
/// real instances whose `Drop` is `SysFreeString`/`WindowsDeleteString`.
///
/// AND THE RULE EXISTED IN THREE COPIES, which is why it is stated here and only here (§G):
///   · this fn — gated `ctor_leaf_from_call_path` and `ctor_leaf_from_value_path`'s variant test
///   · a `camel` closure inside `type_from_value_path` — the same two clauses written again, gating
///     `let` inference and therefore RECEIVER TYPING
///   · `collector.rs`'s bare-upper receiver fallback — upper-initial AND NO UNDERSCORE, a THIRD rule,
///     which is why `DIRECT.touch()` already resolved while `let x = DIRECT; x.touch()` did not
///
/// The unified rule is the UNION of all three, so no caller narrows: upper-initial, and either it
/// contains a lowercase (`Guard`, `Foo_Bar`) or it contains no underscore (`S`, `IO`, `MAX`). A
/// `SCREAMING_SNAKE` const is still refused, because it has an underscore and no lowercase.
///
/// `MAX`/`NONE`/`DEFAULT` are now type-shaped, and they are also plausible const names. That exposure
/// is [[R213]]'s leaf collision and is bounded the same way — every consumer is gated on a LOCAL
/// declaration (`drop_types` for the glue, `local_types` for a method link), so an all-caps const only
/// ever mis-answers where a local type of that exact leaf also exists. The one shape that was NOT so
/// bounded — `u32::MAX`, an associated const of a PRIMITIVE, which is everywhere in real Rust — is
/// refused by `type_from_value_path` rather than by narrowing this predicate.
pub(crate) fn is_type_ident(s: &str) -> bool {
    let mut ch = s.chars();
    ch.next().is_some_and(|c| c.is_uppercase())
        && (s.chars().any(|c| c.is_lowercase()) || !s.contains('_'))
}

/// True for an ident the R722 union admits and the OLD lowercase-requiring rule REJECTED — the
/// CHANGED BRANCH, for the §E1 reach counter. Not a classification anything depends on.
pub(crate) fn caps_only_ident(s: &str) -> bool {
    is_type_ident(s) && s.chars().count() > 1 && !s.chars().any(|c| c.is_lowercase())
}

/// SOUNDNESS R856 — a type-shaped leaf with NO lowercase letter (`SHARED`, `DB`, `C`, `T0`): the
/// spelling a `static`/`const` and a unit struct share, so a VALUE path ending in one cannot be read as
/// naming its own type. `caps_only_ident` plus the one-character case it excludes (it is an R722 reach
/// counter and must stay as it is); a dependency's `pub static C` is as much a static as its `SHARED`.
pub(crate) fn caps_value_leaf(s: &str) -> bool {
    is_type_ident(s) && !s.chars().any(|c| c.is_lowercase())
}

/// SOUNDNESS R722, the OVER-CHARGE REFUSAL the row named. A leaf the union admits ONLY because it is
/// ALL-CAPS is spelled identically to a declared `const`/`static` of that name — `MAX`, `NONE`,
/// `DEFAULT`, `EMPTY` are all plausible both ways. `collect_static_types` is the authority on those
/// names, and `resolve_recv_type_for`'s `via_static` arm ALREADY consults it for the receiver route;
/// the construction and `let`-inference routes did not, which is §G one more time.
///
/// MEASURED on the row's own control, and it fabricated before this: `mod a { pub const MAX: u32 = 9; }`
/// beside `mod b { pub struct MAX; impl Drop for MAX { fs::remove_file } impl MAX { fn count_ones ..} }`
/// — every index here is LEAF-keyed and crate-wide, so `a`'s `let m = MAX; m.count_ones()` charged
/// `['Fs','Net']` with edges to `b::MAX::drop` AND `b::MAX::count_ones` for a function that adds two
/// integers. That is [[R168]]'s `Ordering::Acquire` shape with the case inverted.
///
/// The index is leaf-keyed and crate-wide, so this REFUSES the ambiguity rather than picking a winner
/// — and it is NOT [[R718]]'s `&=` situation. There, ownership had to win because withdrawing was a
/// silence. That holds only because of WHERE this is called: the BARE VALUE PATH arm of
/// `visit_expr_path`, the one construction spelling a const shares.
///
/// THE FIRST VERSION WAS CALLED SOMEWHERE ELSE AND DID INTRODUCE A SILENCE. It filtered the
/// `drop_relevant` SET, which gates all three spellings — and `ctor_leaf_from_call_returns`, the R165
/// rescue, ALREADY reached an all-caps leaf pre-R722 wherever the constructor's own fn leaf was
/// unambiguous crate-wide. Run against the PRE binary, `let _g = MAX::new()` beside `mod a`'s
/// `const MAX` charged `['Fs']`; the wide filter withdrew it. `assert-audit.sh` flagged the comment that
/// claimed otherwise, and running the PRE binary is what settled it. `MAX { .. }`, `MAX(1)` and
/// `MAX::new()` cannot be written for a const, so they keep their charge and only the bare path is
/// refused.
///
/// RESIDUAL: `use a::*;` does not expand, so a GLOB-imported const of a colliding all-caps name is not
/// seen here at all — the same residual [[R168]] measured on tokio's `Ordering::*`. And an ASSOCIATED
/// const (`IUnknown::IID`) is invisible to this index by construction — `collect_static_types` records
/// MODULE-level consts — which is why `type_from_value_path`'s variant branch refuses that shape
/// separately.
pub(crate) fn caps_leaf_shadowed_by_const(
    leaf: &str,
    statics: &HashMap<String, Option<String>>,
) -> bool {
    // `contains_key`, not a `Some` test: a `None` entry means two declarations of that const name
    // disagreed on their type, which still makes the name a const.
    caps_only_ident(leaf) && statics.contains_key(leaf)
}

/// Rust's PRIMITIVE type names. A multi-segment VALUE path rooted in one of these is, in ordinary code,
/// an associated const or fn of a primitive (`u32::MAX`, `f64::EPSILON`, `char::MAX`, `f32::consts::PI`),
/// whose LEAF must not be allowed to collide with a local type. Same refusal `local_type_leaf` already
/// makes for a `std`/`core`/`alloc` root, and for the same measured reason; stated here because R722's
/// union admits `MAX`, which made `u32::MAX` reachable for the first time. Closed set, defined by the
/// language.
///
/// NOT "a primitive root can never name a local type" — that is what this comment said first, and it is
/// false: `mod u32 { pub struct BITS; }` COMPILES (primitive names are not keywords), and `u32::BITS`
/// then denotes that struct. VERIFIED by building it rather than asserted. The corner is accepted, not
/// overlooked: such a module shadows a primitive's inherent consts for every reader of the code, and
/// declining to type it costs a charge no shipped build ever made — whereas reading `u32::MAX` as a
/// local `MAX` fabricates in code that is everywhere.
fn is_primitive_root(s: &str) -> bool {
    matches!(s, "u8" | "u16" | "u32" | "u64" | "u128" | "usize"
                | "i8" | "i16" | "i32" | "i64" | "i128" | "isize"
                | "f16" | "f32" | "f64" | "f128" | "bool" | "char" | "str")
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
    // §E1 REACH COUNTER — R722's union answering for an all-caps type this arm used to refuse
    // (`BSTR::new()`). It was rescued by `ctor_leaf_from_call_returns` ONLY while the assoc-fn LEAF was
    // unambiguous crate-wide; a second `new` anywhere in the crate made that rescue fail and the row
    // land on `ambiguous:same-name fns with different return types`.
    if caps_only_ident(ty) && std::env::var_os("CANDOR_ALIAS_DEBUG").is_some() {
        eprintln!("R722ASSOC {full}");
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
pub(crate) fn local_type_leaf(ty: &str) -> Option<String> {
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
            let full = expand(&path_to_string_lc(&s.path), uses);
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
/// SOUNDNESS R172 — THE ANSWER IS PER CONSTRUCTION SITE, NOT PER TYPE LEAF, and the leaf set this
/// returns is only the sites' summary.
///
///     pub fn swap_free(p: &str, q: &str) -> H { let _g = H::new(p); from_handle(q) }
///
/// Two DIFFERENT `H` values: one dies here (executed — `H::drop` really runs inside that frame), one
/// is returned. Keyed by leaf, the returned one's escape suppressed the local one's drop and the
/// function vanished from `functions[]` — a silent under-report, and a REGRESSION against published
/// 0.34.0, because R160 (`Self::mk(q)`) and R165 (`from_handle(q)`) taught `ctor_leaf_of_expr` to
/// answer `H` for two tail spellings that used to answer nothing. R168 fixed exactly this shape for
/// the PARAMETER half by keying on `escapes.names`; this is the construction half of the same defect,
/// and the pre-existing `swap_type` (`H::mk(q)`) victim goes with it — same mechanism, so a fix scoped
/// to the two spellings the row was filed for would be an audit boundary drawn around its trigger.
///
/// So a leaf is suppressed only when EVERY construction of it in this body escapes. One non-escaping
/// site is a live counterexample and the whole leaf is charged, which is the over-approximating
/// direction: a site-set that is too small can only ever charge more.
///
/// SITES ARE IDENTIFIED BY THE ADDRESS of the borrowed `syn::Expr`, which is why the body walk and the
/// escape walk have to see the SAME tree. The one place they cannot is a MACRO body — `mark_escape`
/// parses `vec![Guard::new()]`'s tokens into owned temporaries whose addresses die with the call (and
/// could be reused by a later allocation).
///
/// SOUNDNESS R229 — SO THE TWO WALKS SHARE A READING AND A NUMBERING INSTEAD. `macro_reading` is the
/// one reading (invocation tokens as an expression list, as statements, and the resolved
/// `macro_rules!` arms) and `macro_reading_ordinals` the one numbering, so a construction inside a
/// macro is a SITE with an identity both walks can compute; `ord_nested` composes a nested reading's
/// ordinals into the enclosing one's space. What this replaced was `macro_ctor_leaves`, a leaf-keyed
/// set that made this gate CERTIFY the escape for any leaf a macro anywhere in the body constructed —
/// an unevidenced licence, and the direction this file is not allowed to be wrong in.
pub(crate) fn escaping_ctor_leaves<'a>(
    block: &'a syn::Block,
    uses: &'a HashMap<String, String>,
    fields: &'a FieldIndex,
    returns: &'a ReturnIndex,
    // SOUNDNESS R203 — the crate-wide `macro_rules!` NAME -> arm-tokens index the collector already
    // builds for R48. Read ONLY inside a `?`'s operand, and only to decide what a macro CONSTRUCTS; the
    // expansion never becomes a construction SITE (see `note_local_macro_template_leaves`).
    local_macros: &'a HashMap<String, String>,
) -> Escapes {
    // Every binding/assignment/method-call site in the body, gathered once; each root below re-reads
    // this table (running its own copy of the fixpoint), never re-walks the tree.
    let mut sites = EscapeSites::new(uses, fields, returns, local_macros);
    sites.walk_block(block, true);
    // SOUNDNESS R198/R209(b) — decide the LET-BOUND closures now that the whole body is walked.
    //
    // A closure's body is only built when the closure RUNS, and only dies here if the value it
    // produces also dies here. So the suppression is withdrawn in exactly one provable case: the name
    // is used, and EVERY use of it is the callee of a call whose value is discarded (`f();`,
    // `let _ = f();`). Then the body ran in this frame and nothing carried its result out.
    //
    // Everything else keeps the old unconditional escape, and each exclusion is a measured cell:
    //   · never invoked (`let f = || H::new(); ` and no call) — the body NEVER RUNS, so charging it
    //     fabricates. Executed ground truth: 0 drops.
    //   · the result leaves (`return f()`) — it is the caller's to drop. Executed: 0 drops here.
    //   · any other use of the name (stored, passed on, invoked with its value bound and kept) — not
    //     provably local, so it stays suppressed. This is a DENYLIST of the exemption: an unrecognised
    //     spelling keeps today's behaviour and stays silent rather than fabricating.
    for (name, body, seq) in std::mem::take(&mut sites.pending_closure_escapes) {
        let uses_n = sites.path_uses.get(&name).copied().unwrap_or(0);
        let disc_n = sites.discarded_calls.get(&name).copied().unwrap_or(0);
        // R304 — the counters are keyed on a BARE IDENT with no scope, so they only describe one
        // entity when the body binds that name exactly once and never spells it inside macro tokens
        // the walk cannot read. Either condition failing means the counts may be about something
        // else entirely, and the only safe answer is the old unconditional escape.
        let one_entity = sites.name_bindings.get(&name).copied().unwrap_or(0) == 1
            && !sites.macro_idents.contains(&name);
        if !(one_entity && uses_n > 0 && uses_n == disc_n) {
            sites.escapes.push((seq, body));
        } else if std::env::var("CANDOR_ALIAS_DEBUG").is_ok() {
            // INSTRUMENTED, because an unchanged corpus row is not evidence the new code ran — the
            // same switch decls.rs's alias counter and scan.rs's R105 counter use. This fires exactly
            // when the suppression is WITHDRAWN, i.e. on the charging path this change adds.
            eprintln!("R209BCLOSURE {} ({} use(s), all discarded calls)", name, uses_n);
        }
    }
    // A body with NO terminal exit (a `()`-returning fn with no trailing value — `store` below) still
    // has to run the fixpoint: the field/index/deref `assigns` route is UNCONDITIONAL (not gated on any
    // root, by design — see `escape_from_root`), so `*slot = Some(G::new());` must still be seen. A
    // single call seeded from `None` runs exactly that unconditional half and nothing root-dependent,
    // which is the correct answer when there is no root to be dependent ON.
    // The UNCONDITIONAL half (a closure's return, a `mem::forget`/`ManuallyDrop::new` operand, a
    // field/index/deref store) is in every root's set, so intersecting the roots preserves it. IT USED
    // TO BE RE-UNITED WHOLE AFTER R173's POSITIONAL `?` FILTER, on this comment: *a forgotten value's
    // destructor does not run whichever exit the function takes*. SOUNDNESS R680 / R709 — THAT SENTENCE
    // IS FALSE WHEN THE ROUTE FOLLOWS THE EXIT, and it is the sentence that made the claim stop being
    // measured (the standing rule about a comment asserting safety). The re-union below is now
    // POSITIONAL, per route; read it there.
    let every_route: &dyn Fn(usize) -> bool = &|_| true;
    // §E1 HIT COUNTER state for R189 — the UNION of every root's binding-mediated sites, i.e. exactly
    // what `acc.sites` held before this change. Read once, at the site gate, to print the leaves whose
    // answer the intersection actually moved. It must never be read by a decision.
    let mut via_union: std::collections::HashSet<SiteId> = std::collections::HashSet::new();
    let mut acc = if sites.roots.is_empty() {
        escape_from_root(None, &sites, every_route)
    } else {
        let mut roots = sites.roots.iter();
        let (q0, first) = roots.next().expect("checked non-empty above");
        let mut acc = escape_from_root(*first, &sites, every_route);
        // SOUNDNESS R189 — every root's binding-mediated sites, WITH THAT ROOT'S POSITION, kept until
        // the whole root set is known. They cannot be folded pairwise like `names`/`leaves`, because a
        // root only gets a VOTE on a site it could have reached — see the intersection below.
        let mut via_by_root: Vec<(usize, std::collections::HashSet<SiteId>)> =
            vec![(*q0, std::mem::take(&mut acc.via))];
        for (q, r) in roots {
            if acc.names.is_empty() && acc.leaves.is_empty() {
                break; // the intersection can only shrink further; every remaining root would too.
            }
            let next = escape_from_root(*r, &sites, every_route);
            acc.names.retain(|n| next.names.contains(n));
            acc.leaves.retain(|l| next.leaves.contains(l));
            via_by_root.push((*q, next.via));
            // SITES ARE UNIONED ACROSS EXITS WHILE LEAVES ARE INTERSECTED, and the two together are the
            // rule. A site is a single construction expression, so an exit that cannot reach it has no
            // opinion about it — intersecting made `fn render(..) -> Error { .. return Error::msg(s);
            // .. Error::msg(msg) }` (anyhow) charge `Error::drop`, because each exit escapes a
            // DIFFERENT one of the two sites and neither is in both. The conditional-escape regression
            // this file exists to prevent is still caught, by the leaf INTERSECTION one line up: a name
            // bound before a branch whose fate the branch decides drops its leaf from `acc.leaves` at
            // the exit that does not carry it. The site gate below then only ever REMOVES leaves from
            // whatever set it is handed — it can never add one. (It was true at R172 that the result
            // could not exceed the shipped leaf-keyed set; R173, one commit later, deliberately widens
            // the set the gate is handed, so state the narrowing property, which is the one this code
            // actually has.)
            //
            // SOUNDNESS R189 — AND THAT REASONING IS SOUND FOR EXACTLY THE SITES IT DESCRIBES AND FOR
            // NO OTHERS. `return Error::msg(s)` and the tail `Error::msg(msg)` are each lexically
            // inside ONE root's operand, which is why "an exit that cannot reach it has no opinion
            // about it" is true of them — and it is FALSE of `let a = H::new(p);`, a value that is
            // live at every exit and whose site the old code unioned all the same. So the union is
            // kept, narrowed to the operand-local half (`sites`); the binding-mediated half (`via`) is
            // intersected one line up. The comment above claimed the leaf INTERSECTION caught the
            // conditional case; measured, it does not when both exits carry the SAME leaf under
            // DIFFERENT names — `either` (R189) keeps `H` through the intersection and then loses it
            // to this union, executed 1 drop, reported ABSENT, `deny Fs` exit 0.
            acc.sites.extend(next.sites);
        }
        // SOUNDNESS R189 — THE BINDING-MEDIATED INTERSECTION, AND IT IS POSITIONAL FOR R173's REASON.
        // A value reached through a `let`/assignment/receiver is live at every exit that comes AFTER
        // its construction, so each of those exits has to certify its escape; an exit that comes BEFORE
        // it has no opinion, because the value does not exist yet and that exit cannot drop it.
        //
        // MEASURED, and the corpus A/B is what found it: `windows-strings`' `BSTR::from_wide` is
        // `if value.is_empty() { return Self::new(); } let result = Self(SysAllocStringLen(..)); ..
        // result` — a blanket intersection let the EARLY return, taken before `result` exists, veto
        // `result`'s escape, and charged `BSTR::drop` to a body that drops nothing on either path. That
        // was the ONE row the whole 1,624-crate census moved, and it was a fabrication.
        //
        // `all()` over an empty vote set answers TRUE (suppressed), which is the pre-change direction:
        // a site no exit could have reached is not evidence of a local drop.
        let all_via: std::collections::HashSet<SiteId> =
            via_by_root.iter().flat_map(|(_, v)| v.iter().copied()).collect();
        via_union.extend(all_via.iter().copied());
        acc.via = all_via
            .into_iter()
            .filter(|s| {
                let p = sites.site_seq.get(s).copied().unwrap_or(0);
                via_by_root.iter().filter(|(q, _)| *q > p).all(|(_, v)| v.contains(s))
            })
            .collect();
        acc
    };
    // SOUNDNESS R173 — THE `?` VETO IS POSITIONAL. A `?` is a genuine early exit that carries nothing
    // out, so anything live when it is evaluated may die there and must be charged — that is why it
    // vetoes at all, and `order_after` (`let w = Self::for_region(n); gen(n)?; Ok(w)`) is the fixture
    // that proves it still does. But it was applied as a BLANKET empty root, so it also vetoed values
    // the function has not built yet: `gen(n)?; Ok(Self::for_region(n))` charged `Wr::drop` to a body
    // where nothing is dropped at all. Pre-existing, and R160 made it fire on the dominant spelling —
    // 302 rows gained a `::drop` edge across the registry corpus, 106 of them NEW positive claims.
    //
    // So each `?` removes only what its own position can reach: a leaf whose FIRST construction, or a
    // name whose binding, precedes it. Anything with no recorded position (a parameter; a leaf the
    // site pass never placed) counts as live from the start — the charging direction, and the shipped
    // answer.
    for t in &sites.try_exits {
        if acc.names.is_empty() && acc.leaves.is_empty() {
            break;
        }
        let p = &t.seq;
        // §E1 HIT COUNTER — R173's branch is "this `?` no longer vetoes something built after it".
        if std::env::var("CANDOR_ALIAS_DEBUG").is_ok() {
            for l in &acc.leaves {
                if sites.first_ctor_seq.get(l).is_some_and(|s| s > p) {
                    eprintln!("R173ORDER {l}");
                }
            }
        }
        // SOUNDNESS R194 — POSITION IS NOT ENOUGH FOR THE `?`'S OWN OPERAND. `Expr::Try` is numbered at
        // its PRE-order position, i.e. before its operand is walked, so every construction inside the
        // operand gets a HIGHER number than the `?` and reads as "not built yet" — while evaluating
        // that operand is precisely what built it. `{ out.push(H::new()); gen(n) }?` therefore kept `H`
        // in the escaping set and lost the drop that really runs when `gen` fails (executed: one
        // `H::drop` in that frame), a REGRESSION against published 0.34.0, which vetoed blanket.
        // `t.interior` carries the leaves the operand builds OFF ITS VALUE SPINE; they are vetoed
        // regardless of position. See `value_spine_addrs` for what the spine is and why the leaves ON
        // it must NOT be vetoed — that exemption is the whole of R173's gain here (`Ok(Repr::new(s)?)`,
        // `Async::new(stream)?`), and a lump post-order without it re-fabricates published's blanket
        // charges over the corpus.
        if std::env::var("CANDOR_ALIAS_DEBUG").is_ok() {
            for l in &acc.leaves {
                // §E1 HIT COUNTER — R194's branch is "position would have KEPT this leaf and the
                // operand-interior rule removes it". Printed only when it really fires.
                if sites.first_ctor_seq.get(l).is_some_and(|s| s > p) && t.interior.contains(l) {
                    // R199's branch is the MACRO half of the same term, and it is disjoint from what
                    // R194 could ever have printed: before R199 a macro-borne leaf was not in any
                    // `interior`, so this line firing IS the new code changing the answer.
                    // ORDER MATTERS AND IS THE POINT. R203's walk deliberately re-covers the tokens
                    // R199 already walked (it goes deeper — into the blocks `for_each_child_expr` stops
                    // at), so a leaf can be inserted by both. Asking R199's question FIRST keeps
                    // `R199MACRO` a count of ITS branch across builds, and leaves `R203OPAQUE` counting
                    // exactly the leaves NO earlier mechanism could reach — the number §E1 asks for.
                    if sites.macro_interior_leaves.contains(l) {
                        eprintln!("R199MACRO {l}");
                    } else if sites.opaque_interior_leaves.contains(l) {
                        eprintln!("R203OPAQUE {l}");
                    } else {
                        eprintln!("R194OPERAND {l}");
                    }
                }
            }
        }
        acc.leaves
            .retain(|l| sites.first_ctor_seq.get(l).is_some_and(|s| s > p) && !t.interior.contains(l));
        acc.names.retain(|n| sites.first_bind_seq.get(n).is_some_and(|s| s > p));
    }
    // SOUNDNESS R709 — THE UNCONDITIONAL ROUTES ARE RE-UNITED *POSITIONALLY*. The comment on `uncond`
    // above used to justify re-uniting them whole, on the ground that "a forgotten value's destructor
    // does not run whichever exit the function takes". That is true of the FORGETTING and false of the
    // FUNCTION: if the `mem::forget`/`ManuallyDrop::new` call, the field/index/deref store, or the
    // inline closure lies AFTER a `?`, then on that `?`'s error path the route is never reached and a
    // value that was already live really does die in this frame. Executed ground truth, both spellings
    // the row was filed for plus the `ManuallyDrop` sibling: 1 in-frame `H::drop` on the Err path, 0 on
    // the Ok path, against 0/0 for every control whose route precedes the `?`.
    //
    // THE FIX IS POSITIONAL, NOT THE BLUNT ONE. Dropping the exemption outright is priced in
    // `collector.rs`'s `charge_at_construction` at 551 FABRICATED hard-effect charges over 1,561
    // crates, 0 of 28 sampled a real in-frame drop — so a route still certifies its escape, it just
    // only certifies it against the `?`s it has already passed. Each route is judged by ITS OWN
    // position: routes are grouped into BUCKETS by which `?`s precede them (which is why this is one
    // extra fixpoint run per bucket and not one per route — a body with no `?` has exactly one bucket
    // and gets the shipped answer unchanged), each bucket's marks are computed alone, and only the
    // `?`s that precede that bucket filter it, by exactly R173/R187/R194's rule.
    //
    // A `?` INSIDE A CLOSURE IS NOT AN EXIT OF THIS FUNCTION, so it cannot kill a value live here and
    // is excluded from the route comparison. R173's conditional half keeps its (pre-existing,
    // over-charging) treatment of those; this half would be ADDING the over-charge, which is the one
    // direction `charge_at_construction` measured as ruinous.
    let mut route_seqs: Vec<usize> = sites
        .escapes
        .iter()
        .map(|(q, _)| *q)
        .chain(sites.assigns.iter().filter(|(l, _, _)| l.is_none()).map(|(_, _, q)| *q))
        .collect();
    route_seqs.sort_unstable();
    route_seqs.dedup();
    let mut buckets: HashMap<Vec<usize>, Vec<usize>> = HashMap::new();
    for q in route_seqs {
        let key: Vec<usize> = sites
            .try_exits
            .iter()
            .enumerate()
            .filter(|(i, t)| {
                !t.in_closure
                    && t.src_seq < q
                    && !sites.route_inside.get(&q).is_some_and(|open| open.contains(i))
            })
            .map(|(i, _)| i)
            .collect();
        buckets.entry(key).or_default().push(q);
    }
    for (preceding, qs) in buckets {
        let gate: &dyn Fn(usize) -> bool = &|s| qs.binary_search(&s).is_ok();
        let mut m = escape_from_root(None, &sites, gate);
        for i in preceding {
            let t = &sites.try_exits[i];
            // The LEAF test reads `t.seq` (R187's rewritten position), because the question there is
            // R173's — "was this leaf already built?" — and R187 rewrote `seq` precisely for it. The
            // ROUTE test above reads `src_seq`. Two questions, two positions; see `TryExit::src_seq`.
            let p = &t.seq;
            // §E1 HIT COUNTER — R709's branch is "an unconditional route lies after this `?`, so the
            // exemption is withdrawn for something that was live there". Fires only when the answer
            // really moves, so a zero count in an A/B means the corpus never reached the change.
            if std::env::var("CANDOR_ALIAS_DEBUG").is_ok() {
                for l in &m.leaves {
                    if !(sites.first_ctor_seq.get(l).is_some_and(|s| s > p) && !t.interior.contains(l)) {
                        eprintln!("R709ROUTE {l}");
                    }
                }
                for n in &m.names {
                    if !sites.first_bind_seq.get(n).is_some_and(|s| s > p) {
                        eprintln!("R709ROUTENAME {n}");
                    }
                }
            }
            m.leaves
                .retain(|l| sites.first_ctor_seq.get(l).is_some_and(|s| s > p) && !t.interior.contains(l));
            m.names.retain(|n| sites.first_bind_seq.get(n).is_some_and(|s| s > p));
        }
        acc.names.extend(m.names);
        acc.leaves.extend(m.leaves);
        // R189 — a route-only (`root: None`) walk has no exit to be relative to: `mem::forget(a)` or
        // `*slot = a` happens on whatever path reaches it, which is what the bucket machinery above
        // already decides positionally. So BOTH halves of its marks join the unioned side here, which
        // is the shipped behaviour for the unconditional routes unchanged.
        acc.sites.extend(m.sites);
        acc.sites.extend(m.via);
    }
    // R172, the site gate. `acc.leaves` is the shipped leaf-keyed answer; a leaf survives it only if
    // every construction of that leaf recorded by the body walk is one of the escaping sites. A leaf
    // with NO recorded site (only reachable through a macro, or through a walk position the site pass
    // does not reach) keeps the old answer — the site set is an added refusal, never a new licence.
    let mut escaped_sites: std::collections::HashSet<SiteId> = std::mem::take(&mut acc.sites);
    escaped_sites.extend(std::mem::take(&mut acc.via));
    // R189 — what the pre-change code would have handed the gate. Only the hit counter reads it.
    via_union.extend(escaped_sites.iter().copied());
    acc.leaves.retain(|l| {
        // SOUNDNESS R229 — THE LEAF-KEYED MACRO LICENCE IS GONE. `macro_ctor_leaves` said "this leaf
        // was built inside a macro somewhere, and no site table can hold that construction, so keep
        // the shipped leaf-keyed answer" — i.e. CERTIFY the escape without evidence, which is the one
        // direction this file is not allowed to be wrong in. It existed because a nested macro's parse
        // produces owned temporaries whose addresses die with the call; `ord_nested` now composes a
        // nested reading's ordinals into the enclosing reading's space, so the site walk and
        // `mark_escape` agree about those constructions and the licence has nothing left to cover.
        match sites.ctor_sites.get(l) {
            Some(v) => {
                let all = v.iter().all(|s| escaped_sites.contains(s));
                // §E1 HIT COUNTER, R229's CHARGING half — and it counts the DECISION, not the walk.
                // `R229MACROSITE` counts every construction the shared macro reading finds, which is
                // six figures on this corpus and says nothing about whether the answer moved. This
                // fires only when the gate would have SUPPRESSED the leaf on the sites the pre-change
                // walk could see, and does not because of a site only the shared reading records.
                if !all && std::env::var("CANDOR_ALIAS_DEBUG").is_ok()
                    && v.iter().filter(|s| s.1 < ORD_DEEP0).all(|s| escaped_sites.contains(s))
                {
                    eprintln!("R229DECIDE {l}");
                }
                // §E1 HIT COUNTER — R189's branch is "every site of this leaf was escaping under the
                // blanket site UNION, and is not under the binding-mediated INTERSECTION". That is
                // precisely the set of leaves this change charges and the old code suppressed, so a
                // zero count in an A/B means the corpus never reached it.
                if !all && v.iter().all(|s| via_union.contains(s))
                    && std::env::var("CANDOR_ALIAS_DEBUG").is_ok()
                {
                    eprintln!("R189EXIT {l} ({} sites, {} escaping)", v.len(),
                              v.iter().filter(|s| escaped_sites.contains(s)).count());
                }
                // §E1 HIT COUNTER — the branch R172 adds is exactly "the leaf gate WOULD have
                // suppressed a leaf that has a NON-escaping construction site too". Printed only when
                // it really fires, so a zero count in an A/B means the corpus never reached the change.
                if !all && std::env::var("CANDOR_ALIAS_DEBUG").is_ok() {
                    eprintln!("R172SITE {l} ({} sites, {} escaping)", v.len(),
                              v.iter().filter(|s| escaped_sites.contains(s)).count());
                }
                all
            }
            None => true,
        }
    });
    Escapes { leaves: acc.leaves, names: acc.names }
}

/// The escape fixpoint (root use, then the `lets`/`assigns`/`method_args` transitive closure — same
/// rules as before this fix), seeded from exactly ONE of the function's terminal exits. `root` is
/// `None` for an exit proven to carry nothing (see `escaping_ctor_leaves`'s doc comment); such a call
/// simply returns the empty sets, which is what makes it veto any name/leaf during the caller's
/// intersection.
fn escape_from_root(
    root: Option<&syn::Expr>,
    sites: &EscapeSites<'_>,
    // SOUNDNESS R709 — which UNCONDITIONAL routes this call may follow, by the route's own pre-order
    // position. `route_at` is the identity `|_| true` for every root-seeded call (exactly the shipped
    // behaviour: a root's set contains the whole unconditional half, which is what makes the
    // intersection across roots preserve it). `escaping_ctor_leaves` uses it to compute ONE bucket of
    // routes at a time, so each bucket's contribution can be filtered by the `?`s that precede it.
    route_at: &dyn Fn(usize) -> bool,
) -> Marks {
    let (uses, fields, returns) = (sites.uses, sites.fields, sites.returns);
    // The SAME macro indexes the site walk used. `body_macros` is complete by now — `walk_block` has
    // finished — so both walks resolve every invocation to the same template.
    let lens = MacroLens { local: sites.local_macros, body: &sites.body_macros };
    let guard = &mut Vec::new();
    let mut m = Marks::default();
    if let Some(e) = root {
        mark_escape(e, &lens, guard, uses, fields, returns, &mut m);
    }
    // Unconditional escape ROUTES (a closure body, a `mem::forget`/`ManuallyDrop::new` operand) — not
    // gated on `root` at all, and identical on every `escape_from_root` call, which is what lets them
    // survive `escaping_ctor_leaves`'s intersection across roots undiminished.
    for (q, e) in &sites.escapes {
        if route_at(*q) {
            mark_escape(e, &lens, guard, uses, fields, returns, &mut m);
        }
    }
    // SOUNDNESS R189 — EVERY MARK BELOW THIS LINE IS BINDING-MEDIATED, so its sites are `via`, not
    // `sites`. `mark_escape` only ever INSERTS, so a scratch `Marks` per call is equivalent to marking
    // into `m` directly, and `absorb_via` then files the sites under the kind that decides how they
    // combine across exits. The one exception is the `lhs: None` store arm, which is unconditional and
    // not exit-dependent — it keeps marking directly, exactly as the route walk above does.
    for _ in 0..8 {
        let before = (m.names.len(), m.leaves.len(), m.sites.len(), m.via.len());
        for (name, init) in &sites.lets {
            if m.names.contains(name) {
                let mut sub = Marks::default();
                mark_escape(init, &lens, guard, uses, fields, returns, &mut sub);
                m.absorb_via(sub);
            }
        }
        for (lhs, rhs, q) in &sites.assigns {
            match lhs {
                // `x = Guard::new()` — only an escape if `x` itself escapes (in THIS root).
                Some(n) => {
                    if m.names.contains(n) {
                        let mut sub = Marks::default();
                        mark_escape(rhs, &lens, guard, uses, fields, returns, &mut sub);
                        m.absorb_via(sub);
                    }
                }
                // `self.g = …` / `xs[i] = …` / `*p = …` — stored somewhere this scope does not own,
                // unconditionally (not gated on this root, same as before this fix: a store is a store
                // regardless of which exit the function eventually takes). R709 — it IS gated on the
                // store's own position, because a store that has not happened yet stores nothing.
                None => {
                    if route_at(*q) {
                        mark_escape(rhs, &lens, guard, uses, fields, returns, &mut m);
                    }
                }
            }
        }
        for (recv, args) in &sites.method_args {
            if m.names.contains(recv) {
                for a in args {
                    let mut sub = Marks::default();
                    mark_escape(a, &lens, guard, uses, fields, returns, &mut sub);
                    m.absorb_via(sub);
                }
            }
        }
        if (m.names.len(), m.leaves.len(), m.sites.len(), m.via.len()) == before {
            break;
        }
    }
    m
}

/// One escape walk's findings. `leaves` is the shipped leaf-keyed answer, kept because it is what the
/// macro fallback still needs; `sites` is the R172 refinement — the ADDRESS of each construction
/// expression that escapes, so two constructions of one type in one body stop being one fact.
///
/// SOUNDNESS R189 — AND THE SITES COME IN TWO KINDS, BECAUSE THEY ANSWER TO DIFFERENT EXITS. A site
/// LEXICALLY INSIDE this root's operand (`return Ok(H::new(p))`, a tail operand) is evaluated on THIS
/// exit and on no other, so no other exit has an opinion about it and unioning it across roots is
/// sound — that is `sites`. A site reached through a `let`/assignment/receiver BINDING is a value that
/// exists at EVERY exit, so one exit carrying it out says nothing about the exits that do not; its
/// escape has to be certified by all of them, which is an INTERSECTION — that is `via`. Collapsing the
/// two into one unioned set is R189: `let a=H::new(p); let b=H::new(q); if n==0 { return Ok(a); } Ok(b)`
/// handed the site gate both sites as escaping, so the leaf the intersection had correctly KEPT was
/// suppressed anyway and one executed `H::drop` vanished.
#[derive(Default, Clone)]
struct Marks {
    names: std::collections::HashSet<String>,
    leaves: std::collections::HashSet<String>,
    sites: std::collections::HashSet<SiteId>,
    /// Sites reached through a BINDING rather than through this root's own operand — see the type
    /// comment. Intersected across roots by `escaping_ctor_leaves`; unioned for the route-only
    /// (`root: None`) calls, whose marks are not exit-dependent in the first place.
    via: std::collections::HashSet<SiteId>,
}

impl Marks {
    /// Merge a binding-mediated walk's findings: its names and leaves are this walk's, and every site
    /// it found — including sites IT reached through a further binding — is a `via` site here.
    fn absorb_via(&mut self, sub: Marks) {
        self.names.extend(sub.names);
        self.leaves.extend(sub.leaves);
        self.via.extend(sub.sites);
        self.via.extend(sub.via);
    }
}

/// R172 — a construction site's identity, comparable between the body walk and the escape walk.
/// `(address of the `syn::Expr`, 0)` for a construction written in the tree itself; `(address of the
/// enclosing `syn::ExprMacro`, 1-based pre-order ordinal)` for one inside a macro's token stream,
/// where the two walks each parse their OWN copy and so cannot share addresses. The macro's node IS
/// an `Expr`, but an `Expr::Macro` is never itself a construction, so the two forms cannot collide.
type SiteId = (usize, usize);

/// The canonical numbering both walks use for a macro's parsed token expressions: pre-order over
/// EVERY child, stopping at a NESTED macro (whose own parse would need its own base). Deterministic
/// from the token stream alone, which is what makes two separate parses agree.
fn macro_site_ordinals<'e>(exprs: impl IntoIterator<Item = &'e syn::Expr>) -> HashMap<usize, usize> {
    fn go(e: &syn::Expr, n: &mut usize, out: &mut HashMap<usize, usize>) {
        if matches!(e, syn::Expr::Macro(_)) {
            return;
        }
        *n += 1;
        out.insert(e as *const syn::Expr as usize, *n);
        for_each_child_expr(e, &mut |c| go(c, n, out));
    }
    let mut out = HashMap::new();
    let mut n = 0usize;
    for e in exprs {
        go(e, &mut n, &mut out);
    }
    out
}

/// The two `macro_rules!` indexes an expansion may resolve against: the crate-wide one R48 builds
/// (`local_macros`) and the body-local overlay `walk_block` fills as it passes a `Stmt::Item`
/// definition. Carried as one value so the SITE walk and the ESCAPE walk are handed the identical
/// pair — §F1 #3, two implementations of one question drift, and this family has been the proof of
/// it four times (R199, R203, R204, R210 are each one walk seeing what the other could not).
#[derive(Clone, Copy)]
struct MacroLens<'a> {
    local: &'a HashMap<String, String>,
    body: &'a HashMap<String, String>,
}

/// ONE READING OF ONE MACRO INVOCATION, produced by `macro_reading` and consumed by BOTH walks.
///
/// A macro can be read three ways and the readings are not alternatives — a template invocation has
/// tokens AND a definition, and both can construct. Each reading is a REGION, and a construction in
/// region *r* is site-keyed `(address of the enclosing `syn::ExprMacro`, ordinal(r, k))`; the two
/// walks parse their own copies of the same tokens, so the ordinal is the only thing that can carry
/// identity across them (`macro_site_ordinals`' original reason, generalised to every region).
struct MacroReading {
    /// Region 0 — the invocation tokens as a comma-punctuated expression list. `vec![H::new()]`.
    token_exprs: Vec<syn::Expr>,
    /// Region 1 — the same tokens read as a STATEMENT sequence, when region 0 could not parse them.
    /// `stmts!(let x = H::new("a"); x)`.
    token_stmts: Option<syn::Block>,
    /// Region 2+ — the arms of a resolved crate-local or body-local `macro_rules!`, flattened by
    /// `macro_template_blocks_flat`. The construction of `mk_h!("a")` lives HERE and in no other
    /// region: the invocation's tokens are `"a"`.
    template_arms: Vec<syn::Block>,
}

/// Region bases for `macro_reading_ordinals`. Region 0's numbering is deliberately left at 1..n,
/// byte-identical to what `macro_site_ordinals` produced before the other regions existed, so every
/// site identity the two walks already agreed on is unchanged and this can only ADD keys.
pub(crate) const ORD_DEEP0: usize = 1 << 20;
pub(crate) const ORD_STMTS: usize = 1 << 21;
pub(crate) const ORD_TMPL: usize = 1 << 22;
pub(crate) const ORD_ARM_STRIDE: usize = 1 << 14;

/// The shallow pre-order `macro_site_ordinals` numbers — every child, stopping at a nested macro and
/// at a callee path. Factored out so the nested site walk visits exactly the nodes that numbering
/// gave an ordinal to.
fn macro_shallow_exprs<'e>(e: &'e syn::Expr, f: &mut dyn FnMut(&'e syn::Expr)) {
    if matches!(e, syn::Expr::Macro(_)) {
        return;
    }
    f(e);
    match e {
        syn::Expr::Call(c) => c.args.iter().for_each(|a| macro_shallow_exprs(a, f)),
        _ => for_each_child_expr(e, &mut |c| macro_shallow_exprs(c, f)),
    }
}

/// A macro invoked INSIDE a reading. `Expr` is one `mark_escape` can reach through a value position;
/// `Stmt` is one whose value is DISCARDED, which no escape walk ever reaches — so its constructions
/// are always non-escaping sites, i.e. they charge.
enum NestedMacro<'a> {
    Expr(&'a syn::ExprMacro),
    Stmt(&'a syn::StmtMacro),
}

/// SOUNDNESS R229 — THE ORDINAL FOR A SITE REACHED THROUGH A NESTED READING. A positional scheme
/// would need an unbounded stride (a nested reading's own ordinals already run into the millions once
/// template arms are numbered), so compose by FNV-1a over `(j, inner)` instead and set the top bit,
/// which no direct ordinal ever carries. Deterministic from the two indices alone, so the site walk
/// and the escape walk — which parse their own copies — agree.
pub(crate) fn ord_nested(j: usize, inner: usize) -> usize {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in (j as u64).to_le_bytes().iter().chain((inner as u64).to_le_bytes().iter()) {
        h ^= *b as u64;
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    }
    // The top bit of a `usize`, NOT of a `u64`: `1u64 << 63` truncates to ZERO when cast to a 32-bit
    // `usize`, and the separation this relies on would silently disappear on that target while every
    // test passed on the 64-bit machine anyone would check it on. `ORD_NESTED_FLOOR` is the pinned
    // claim; `nested_ordinals_never_alias_a_direct_one` is the test.
    (h as usize) | ORD_NESTED_FLOOR
}

/// The bit `ord_nested` sets, and the floor no DIRECT ordinal may reach. Direct ordinals are region 0's
/// 1..n, `ORD_DEEP0 + k`, `ORD_STMTS + k` and `ORD_TMPL + arm * ORD_ARM_STRIDE + k` — so the claim is
/// that a template's arm count cannot push the last of those over the floor.
pub(crate) const ORD_NESTED_FLOOR: usize = 1usize << (usize::BITS - 1);

/// Every macro invocation inside a reading, in ONE canonical order — region 0's own pre-order, then
/// the block interiors that pre-order stops at, then region 1, then each template arm. Index `j`
/// therefore means the same invocation to both walks, which is the whole of what `ord_nested` needs.
fn nested_macro_nodes(r: &MacroReading) -> Vec<NestedMacro<'_>> {
    let mut out: Vec<NestedMacro<'_>> = Vec::new();
    fn shallow<'e>(e: &'e syn::Expr, out: &mut Vec<NestedMacro<'e>>) {
        if let syn::Expr::Macro(m) = e {
            out.push(NestedMacro::Expr(m));
            return;
        }
        for_each_child_expr(e, &mut |c| shallow(c, out));
    }
    fn deep_block<'e>(b: &'e syn::Block, out: &mut Vec<NestedMacro<'e>>) {
        for st in &b.stmts {
            match st {
                syn::Stmt::Local(l) => {
                    if let Some(init) = &l.init {
                        deep_expr(&init.expr, out);
                        if let Some((_, d)) = &init.diverge {
                            deep_expr(d, out);
                        }
                    }
                }
                syn::Stmt::Expr(e, _) => deep_expr(e, out),
                syn::Stmt::Macro(m) => out.push(NestedMacro::Stmt(m)),
                syn::Stmt::Item(_) => {}
            }
        }
    }
    fn deep_expr<'e>(e: &'e syn::Expr, out: &mut Vec<NestedMacro<'e>>) {
        if let syn::Expr::Macro(m) = e {
            out.push(NestedMacro::Expr(m));
            return;
        }
        for_each_child_expr(e, &mut |c| deep_expr(c, out));
        for_each_child_block(e, &mut |bl| deep_block(bl, out));
    }
    for e in &r.token_exprs {
        shallow(e, &mut out);
    }
    // The interiors region 0's shallow pre-order stops at, walked in the same order `deep0_exprs`
    // numbers them so the two enumerations cannot disagree about which invocation is which.
    fn deep0<'e>(e: &'e syn::Expr, out: &mut Vec<NestedMacro<'e>>) {
        if matches!(e, syn::Expr::Macro(_)) {
            return;
        }
        for_each_child_expr(e, &mut |c| deep0(c, out));
        for_each_child_block(e, &mut |bl| deep_block(bl, out));
    }
    for e in &r.token_exprs {
        deep0(e, &mut out);
    }
    if let Some(b) = &r.token_stmts {
        deep_block(b, &mut out);
    }
    for arm in &r.template_arms {
        deep_block(arm, &mut out);
    }
    out
}

/// Pre-order over a block's statements and expressions, descending into nested blocks — what
/// `for_each_child_expr` deliberately does not do. Stops at a nested macro, exactly as
/// `macro_site_ordinals` does, because a nested macro's own parse needs its own base.
fn deep_exprs<'e>(b: &'e syn::Block, f: &mut dyn FnMut(&'e syn::Expr)) {
    fn ex<'e>(e: &'e syn::Expr, f: &mut dyn FnMut(&'e syn::Expr)) {
        if matches!(e, syn::Expr::Macro(_)) {
            return;
        }
        f(e);
        match e {
            // The callee PATH of a call is the constructor's own NAME, not a second construction —
            // `skip_sites`' reason, applied here by not numbering it. `mark_escape` never descends a
            // callee either (`for_each_value_child` gives a `Call` its ARGS), so a site recorded there
            // could never be marked escaping and would charge a value handed straight to the caller.
            syn::Expr::Call(c) => c.args.iter().for_each(|a| ex(a, f)),
            _ => for_each_child_expr(e, &mut |c| ex(c, f)),
        }
        for_each_child_block(e, &mut |bl| deep_exprs(bl, f));
    }
    for st in &b.stmts {
        match st {
            syn::Stmt::Local(l) => {
                if let Some(init) = &l.init {
                    ex(&init.expr, f);
                    if let Some((_, d)) = &init.diverge {
                        ex(d, f);
                    }
                }
            }
            syn::Stmt::Expr(e, _) => ex(e, f),
            _ => {}
        }
    }
}

/// The nodes region 0 reaches only by descending a BLOCK — `idm!({ out.push(H::new("a")); })`, whose
/// single token expression is an `Expr::Block` that `for_each_child_expr` stops at. Numbered in their
/// own range so region 0's shallow ordinals never move.
fn deep0_exprs<'e>(exprs: &'e [syn::Expr], f: &mut dyn FnMut(&'e syn::Expr)) {
    fn ex<'e>(e: &'e syn::Expr, f: &mut dyn FnMut(&'e syn::Expr)) {
        if matches!(e, syn::Expr::Macro(_)) {
            return;
        }
        match e {
            syn::Expr::Call(c) => c.args.iter().for_each(|a| ex(a, f)),
            _ => for_each_child_expr(e, &mut |c| ex(c, f)),
        }
        for_each_child_block(e, &mut |bl| deep_exprs(bl, f));
    }
    for e in exprs {
        ex(e, f);
    }
}

/// The canonical numbering over a whole `MacroReading`. Deterministic from the reading's structure
/// alone, which is what lets two independent parses of one token stream agree on site identity.
fn macro_reading_ordinals(r: &MacroReading) -> HashMap<usize, usize> {
    let mut out = macro_site_ordinals(r.token_exprs.iter());
    let mut n = 0usize;
    deep0_exprs(&r.token_exprs, &mut |e| {
        n += 1;
        out.entry(e as *const syn::Expr as usize).or_insert(ORD_DEEP0 + n);
    });
    if let Some(b) = &r.token_stmts {
        let mut n = 0usize;
        deep_exprs(b, &mut |e| {
            n += 1;
            out.entry(e as *const syn::Expr as usize).or_insert(ORD_STMTS + n);
        });
    }
    for (j, arm) in r.template_arms.iter().enumerate() {
        let mut n = 0usize;
        let base = ORD_TMPL + j * ORD_ARM_STRIDE;
        deep_exprs(arm, &mut |e| {
            n += 1;
            out.entry(e as *const syn::Expr as usize).or_insert(base + n);
        });
    }
    out
}

/// SOUNDNESS R205/R206 — which `macro_rules!` NAME an invocation path resolves to, if any. Extracted
/// so the site walk and the escape walk cannot resolve differently; see `note_local_macro_template`'s
/// comment for why the LEAF of a path is used and which direction that fails in.
fn resolve_local_macro(m: &syn::ExprMacro, lens: &MacroLens<'_>) -> Option<String> {
    let full = path_to_string(&m.mac.path);
    if full.contains("::") {
        let leaf = full.rsplit("::").next().unwrap_or_default().to_string();
        if lens.local.contains_key(&leaf) || lens.body.contains_key(&leaf) {
            return Some(leaf);
        }
        return None;
    }
    if lens.local.contains_key(&full) || lens.body.contains_key(&full) {
        return Some(full);
    }
    None
}

/// THE ONE READING both walks use. `guard` is the recursion guard R48 uses, threaded so a template
/// that invokes itself terminates identically on both sides.
fn macro_reading(
    m: &syn::ExprMacro,
    lens: &MacroLens<'_>,
    expanding: &dyn Fn(&str) -> bool,
) -> MacroReading {
    // `respan_call_site` is REQUIRED, not hygiene: these tokens were parsed on a rayon worker and this
    // walk runs on a different thread, so syn's one span-JOIN (`parse_negative_lit`, reached by any
    // `-1` in the tokens) aborts the parser. `every_moved_token_reparse_site_survives_the_thread_
    // boundary` is the test; `model::respan_call_site` carries the full explanation.
    let tokens = crate::model::respan_call_site(m.mac.tokens.clone());
    let parser = syn::punctuated::Punctuated::<syn::Expr, syn::Token![,]>::parse_terminated;
    let (token_exprs, token_stmts) = match syn::parse::Parser::parse2(parser, tokens.clone()) {
        Ok(exprs) => (exprs.into_iter().collect::<Vec<_>>(), None),
        Err(_) => (
            Vec::new(),
            syn::parse::Parser::parse2(syn::Block::parse_within, tokens)
                .ok()
                .map(|stmts| syn::Block { brace_token: Default::default(), stmts }),
        ),
    };
    let mut template_arms = Vec::new();
    if let Some(name) = resolve_local_macro(m, lens) {
        if !expanding(&name) {
            // BOTH bodies, never one instead of the other — `note_local_macro_template`'s comment
            // states why (the overlay is per-body, not per-block, so consulting it first can silence).
            let mut bodies: Vec<&String> = Vec::new();
            if let Some(b) = lens.body.get(&name) {
                bodies.push(b);
            }
            if let Some(b) = lens.local.get(&name) {
                if !bodies.contains(&b) {
                    bodies.push(b);
                }
            }
            for b in bodies {
                template_arms.extend(crate::collector::macro_template_blocks_flat(b).1);
            }
        }
    }
    MacroReading { token_exprs, token_stmts, template_arms }
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
///
/// SOUNDNESS R718 — ONE AUTHORITY, TWO OWNERSHIP QUESTIONS. This is the test R168 added for the
/// PARAMETER half; the FIELD half (`owned_drops`, the R49 transitive drop-owner closure) asked the
/// same question and never consulted it, because `type_path` peels `&`/`&mut` on the way into
/// `fields` and a borrowed field arrives as a bare owned leaf. `pub(crate)` rather than a second copy
/// for the reason §G names: two paths computing one fact drift, and the FIELD side is where a wrong
/// answer FABRICATES somebody else's drop glue rather than merely missing one.
pub(crate) fn type_borrows(ty: &syn::Type) -> bool {
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
                // §6d S2 — the NAME half of the same escape gate. Under charge-at-construction a
                // by-value receiver is charged whatever its name does, which is the whole point:
                // `escapes.names` is the site-free twin of `escaping_ctors` and it is keyed on a bare
                // identifier with no scope (R323/R305), so two bindings of one name are one key.
                if escaping_names.contains("self") && !crate::collector::charge_at_construction() {
                    continue;
                }
                if escaping_names.contains("self") && std::env::var("CANDOR_ALIAS_DEBUG").is_ok() {
                    eprintln!("S2SELF {}", self_ty.unwrap_or("?"));
                }
                if let Some(t) = self_ty {
                    out.push(leaf_of(t.to_string()));
                }
            }
            syn::FnArg::Typed(pt) => {
                // SOUNDNESS R718, THE OWNED-WITH-A-BORROW HALF, on the PARAMETER side: the question is
                // whether the leaf charged below — `type_path`'s — is reached through a reference, so it
                // is asked along THAT walk (`type_path_b`) and not of the whole type. `type_borrows`
                // read `t: TempFile<&Path>` as borrowed and the by-value `TempFile`, whose `Drop` really
                // runs here (EXECUTED: 1 drop, the file removed), was ABSENT from `functions[]` on every
                // build since R168. The RECEIVER arm above keeps `type_borrows` deliberately: there the
                // charged leaf is `self_ty`, not the declared type, and `self: Pin<&mut Self>` is exactly
                // the `&`-inside-a-generic case where the whole-type test is the right one.
                let Some((t, borrowed)) = type_path_b(&pt.ty, uses) else { continue };
                if borrowed {
                    continue;
                }
                if std::env::var("CANDOR_ALIAS_DEBUG").is_ok() && type_borrows(&pt.ty) {
                    eprintln!("R718OWNP {t}"); // §E1 REACH COUNTER — the changed branch only
                }
                let Some(name) = single_pat_ident(&pt.pat) else { continue };
                if escaping_names.contains(&name) && !crate::collector::charge_at_construction() {
                    continue;
                }
                if escaping_names.contains(&name) && std::env::var("CANDOR_ALIAS_DEBUG").is_ok() {
                    eprintln!("S2PARAM {name}");
                }
                out.push(leaf_of(t));
            }
            _ => {}
        }
    }
    out
}

/// SOUNDNESS R187 — one `?` early exit: its position in the body walk, and whether it sits inside a
/// CLOSURE (in which case it exits the closure, not this function, and the loop rewrite leaves it alone).
/// SOUNDNESS R194 — plus the leaves this `?`'s OWN OPERAND constructs off its value spine, which a
/// position alone cannot express: they are built BEFORE the `?` runs and numbered AFTER it.
struct TryExit {
    seq: usize,
    /// SOUNDNESS R709 — the pre-order position this `?` was WALKED at, never rewritten. `seq` is moved
    /// to the enclosing loop's last position by R187 so that it vetoes everything that loop body builds;
    /// that rewrite is right for the question R187 asks ("is this leaf live when the `?` runs again?")
    /// and INVERTS for the question R709 asks ("had this escape ROUTE already run when the `?` fired?"),
    /// because moving the `?` later makes a route inside the same loop look like it came first. Within
    /// one iteration the lexical order is the evaluation order, so the route comparison reads this.
    src_seq: usize,
    in_closure: bool,
    /// R194 — type leaves constructed inside this `?`'s operand at a position that is NOT on the
    /// operand's value spine. Evaluating the operand ran them, so they are live when this `?` takes
    /// its error exit, whatever their pre-order number says.
    interior: std::collections::HashSet<String>,
}

struct EscapeSites<'a> {
    uses: &'a HashMap<String, String>,
    fields: &'a FieldIndex,
    returns: &'a ReturnIndex,
    /// SOUNDNESS R172 — every construction site in the body, by type leaf, each identified by the
    /// ADDRESS of its `syn::Expr`. Collected from the SAME walk that finds the escape sites, so the
    /// two halves cannot disagree about what counts as a construction.
    ctor_sites: HashMap<String, Vec<SiteId>>,
    /// SOUNDNESS R189 — each site's own PRE-ORDER POSITION, which is what makes the binding-mediated
    /// site intersection positional. Exactly `first_ctor_seq`'s question asked per SITE instead of per
    /// LEAF: a construction has not happened yet at an exit that precedes it, so that exit cannot drop
    /// it and must not veto its escape. A site missing from this table counts as position 0 — live from
    /// the start, the same default R173 takes for a leaf it cannot place, and the CHARGING direction.
    site_seq: HashMap<SiteId, usize>,
    /// R172 — CALLEE positions, which are NOT construction sites. `MemBio(bio)` is a construction; the
    /// `MemBio` inside it is the callee PATH of that same call, and reading it as a second site of the
    /// same leaf is fatal, because `mark_escape` descends only VALUE children (`for_each_value_child`
    /// gives a `Call` its ARGS, never its `func`) and so can never mark it escaping. The leaf then has
    /// a site that never escapes and is charged for a value it hands straight back to its caller —
    /// measured as three fabrications on the first corpus A/B (openssl `MemBio::from_ptr`,
    /// tokio-postgres `Socket::new_tcp`/`SqlState::from_code`), all of them ABSENT on published 0.34.0.
    skip_sites: std::collections::HashSet<usize>,
    /// SOUNDNESS R198/R209(b) — addresses of closure expressions that are the initialiser of a
    /// `let NAME = |…| …`. The `Expr::Closure` arm below pushes a closure's body onto `escapes`
    /// unconditionally, which is right for an INLINE closure (an argument, a tail) whose value this
    /// scope hands away and cannot follow — but wrong for one bound to a name here, where whether the
    /// body's constructions outlive the frame is decided by what happens to that NAME. Recorded before
    /// the initialiser is walked so the arm can tell the two apart, and only for a SINGLE-IDENT binder:
    /// a destructuring binder never enters `lets`, so the fixpoint below could never reach it and the
    /// unconditional push stays its only correct treatment.
    let_bound_closures: HashMap<usize, String>,
    /// `(name, closure body)` for each `let NAME = |…| …` whose body was NOT pushed onto `escapes`
    /// during the walk. The decision needs counts that only exist once the whole body has been walked,
    /// so it is deferred to `escape_sites` and applied there.
    pending_closure_escapes: Vec<(String, &'a syn::Expr, usize)>,
    /// Every single-ident `Expr::Path` occurrence, counted.
    path_uses: HashMap<String, usize>,
    /// How many times each name is BOUND in this body — every `let NAME = …` and every body-local
    /// `fn NAME`. SOUNDNESS R304: `path_uses` is keyed on a bare identifier with no scope, so a
    /// DIFFERENT entity's discarded call satisfied the test for a closure that never runs
    /// (`let cb: fn() -> u32 = tick; cb(); let cb = || H::new();` — executed 0 in-frame drops,
    /// charged). A name bound more than once is not one name, so it is disqualified outright.
    name_bindings: HashMap<String, usize>,
    /// Identifiers appearing anywhere in a macro's token stream. `path_uses` cannot see them, and an
    /// UNSEEN use makes `uses == disc` more likely, which withdraws suppression that should stand.
    macro_idents: std::collections::HashSet<String>,
    /// Occurrences that are the callee of a call whose VALUE IS DISCARDED (`f();`, `let _ = f();`).
    discarded_calls: HashMap<String, usize>,
    /// SOUNDNESS R173 — a pre-order counter over the body, standing in for evaluation order. Only the
    /// ORDER of these numbers is ever read, never their values.
    seq: usize,
    /// R173 — the position of every `?`. Each one is a real early exit, but it can only drop what is
    /// ALREADY LIVE when it is evaluated, so it is a veto with a position rather than a blanket one.
    /// R187 — a `?` inside a LOOP is rewritten to the loop's own last position when the loop closes,
    /// so the pre-order number stops standing in for evaluation order where the two disagree.
    try_exits: Vec<TryExit>,
    /// SOUNDNESS R194 — the `?`s whose OPERAND the walk is currently inside: the index of each one in
    /// `try_exits`, paired with the ADDRESSES of its operand's value spine. A construction reached
    /// while this is non-empty and not on the spine is recorded in that `?`'s `interior`.
    open_tries: Vec<(usize, std::collections::HashSet<usize>)>,
    /// SOUNDNESS R199 — the leaves put into some `?`'s `interior` by the MACRO path rather than by
    /// `note_ctor_site`. Read only by the §E1 counter, so that R194's number stays a count of ITS branch
    /// and this one gets its own. That the two are disjoint is MEASURED, not argued: over the same
    /// 1,504-crate registry corpus `R194OPERAND` fires 2,824 times in 427 crates both before and after
    /// this change, and `R199MACRO` adds 10 hits in 9 crates on top — a leaf counted here would have
    /// been subtracted there.
    macro_interior_leaves: std::collections::HashSet<String>,
    /// SOUNDNESS R203 — the crate-local `macro_rules!` index (R48's `local_macros`), consulted ONLY
    /// while some `?`'s operand is open. A macro whose TEMPLATE constructs is invisible to the token
    /// re-parse above — the tokens are the INVOCATION's, and `mk_h!("a")` carries no construction at
    /// all — so without this the leaf gets no `interior` entry from the operand and, if the same leaf
    /// is also built somewhere AFTER the `?`, it takes that site's `first_ctor_seq` and survives.
    local_macros: &'a HashMap<String, String>,
    /// SOUNDNESS R206 — `macro_rules!` defined INSIDE the body being walked. `decls.rs` indexes
    /// ITEM-level definitions only, so a body-local one is in no index at all; it is a `Stmt::Item`,
    /// which `walk_block` discards. NAME -> arm tokens, filled as the walk passes the definition (a
    /// `macro_rules!` is not usable before its definition, so the walk order is the scope order) and
    /// read ALONGSIDE `local_macros`, never instead of it — see `note_local_macro_template`.
    /// One `EscapeSites` per function body (`decls.rs` calls `escaping_ctor_leaves` per body), so this
    /// cannot leak between functions. It can leak between BLOCKS of one body, which over-charges.
    body_macros: HashMap<String, String>,
    /// R203 — local macros being expanded on this path, the same recursion guard R48 uses.
    macro_expanding: std::collections::HashSet<String>,
    /// R203 — the leaves put into some `?`'s `interior` by a path that could not address-key the
    /// construction at all (a `macro_rules!` template, a statement-only token stream, a NESTED macro).
    /// Read only by the §E1 counter, so `R203OPAQUE` counts ITS branch and not R194's or R199's.
    opaque_interior_leaves: std::collections::HashSet<String>,
    /// R187 — how many CLOSURE bodies enclose the expression being walked. A `?` inside a closure is an
    /// exit of the CLOSURE, not of this function, so the loop rewrite must not move it: leaving it where
    /// R173 put it keeps that (pre-existing, over-charging) treatment exactly as shipped.
    closure_depth: usize,
    /// R173 — the FIRST position at which each type leaf is constructed, and at which each `let` name
    /// is bound. First, not last: a leaf built both before and after a `?` is live at it.
    first_ctor_seq: HashMap<String, usize>,
    first_bind_seq: HashMap<String, usize>,
    /// `return e` operands and the body's tail expression — each one an independent terminal exit of
    /// the function. `None` marks an exit that provably carries nothing out of this scope (a bare
    /// `return;`, or the implicit early-return a `?` can take): a real, present counterexample, not an
    /// absence of information, so it must veto a name/leaf exactly like an exit that visibly drops it.
    /// SOUNDNESS R189 — each exit WITH ITS PRE-ORDER POSITION. The position is read for one question
    /// only: whether a binding-mediated construction site had already been evaluated when this exit
    /// was taken. An exit that precedes the construction has no opinion about it (the value does not
    /// exist yet), which is R173's rule for a `?` applied to an ordinary exit.
    roots: Vec<(usize, Option<&'a syn::Expr>)>,
    /// Escape ROUTES that are not an exit of THIS function at all, so they must never be intersected
    /// against `roots`: a closure's own return value (which flows out through the closure's eventual
    /// invocation, not through this function returning) and the operand of `mem::forget`/
    /// `ManuallyDrop::new` (which flows to suppression, not to a caller). Each is unconditional — exactly
    /// like a field/deref `assigns` entry — so it is applied once per `escape_from_root` call regardless
    /// of which root that call was seeded from, which is what lets it survive the intersection.
    /// R709 — each route carries its own pre-order POSITION. Surviving the root intersection is not the
    /// same as surviving the `?` filter: a route that lies after a `?` has not run when that `?` takes
    /// its error exit, so it certifies nothing about a value that was already live there.
    escapes: Vec<(usize, &'a syn::Expr)>,
    /// `let NAME = init` — single-ident binders only (a destructuring binder cannot name the value).
    lets: Vec<(String, &'a syn::Expr)>,
    /// `lhs = rhs`; `Some(name)` for a plain local lvalue, `None` for a field/index/deref lvalue.
    /// R709 — each one carries the pre-order position of the assignment, read only for the `None`
    /// (unconditional) half, which is an escape ROUTE and so has a position of its own.
    assigns: Vec<(Option<String>, &'a syn::Expr, usize)>,
    /// `recv.m(args…)` where the receiver is a plain local name.
    method_args: Vec<(String, Vec<&'a syn::Expr>)>,
    /// SOUNDNESS R709 — for each unconditional-route POSITION, the `try_exits` indices whose OPERAND
    /// encloses it. This is R194's trap seen from the other side: `Expr::Try` is numbered at its
    /// PRE-order position, i.e. before its operand is walked, so a route INSIDE that operand gets a
    /// higher number and reads as "after the `?`" — while evaluating the operand is precisely what ran
    /// it. `let h = run_cb_h(|| H::try_new(n, "a"))?;` (async-std's
    /// `spawn_blocking(|| File::create(&p)).await?`, executed: 0 in-frame drops) is the fixture: the
    /// closure IS the `?`'s operand, so the `?` cannot strip its route, and reading the raw pre-order
    /// numbers charged it. A route in the operand has run — or, for a closure the callee never invoked,
    /// built nothing at all — by the time that `?` decides, either way nothing live dies there.
    route_inside: HashMap<usize, std::collections::HashSet<usize>>,
}

impl<'a> EscapeSites<'a> {
    fn new(
        uses: &'a HashMap<String, String>,
        fields: &'a FieldIndex,
        returns: &'a ReturnIndex,
        local_macros: &'a HashMap<String, String>,
    ) -> Self {
        EscapeSites {
            uses,
            fields,
            returns,
            local_macros,
            body_macros: HashMap::new(),
            macro_expanding: std::collections::HashSet::new(),
            opaque_interior_leaves: std::collections::HashSet::new(),
            ctor_sites: HashMap::new(),
            site_seq: HashMap::new(),
            skip_sites: std::collections::HashSet::new(),
            let_bound_closures: HashMap::new(),
            pending_closure_escapes: Vec::new(),
            path_uses: HashMap::new(),
            name_bindings: HashMap::new(),
            macro_idents: std::collections::HashSet::new(),
            discarded_calls: HashMap::new(),
            seq: 0,
            try_exits: Vec::new(),
            open_tries: Vec::new(),
            macro_interior_leaves: std::collections::HashSet::new(),
            closure_depth: 0,
            first_ctor_seq: HashMap::new(),
            first_bind_seq: HashMap::new(),
            roots: Vec::new(),
            escapes: Vec::new(),
            lets: Vec::new(),
            assigns: Vec::new(),
            method_args: Vec::new(),
            route_inside: HashMap::new(),
        }
    }

    /// R709 — record that any unconditional route at the CURRENT position sits inside the operand of
    /// every `?` the walk still has open. Called at each route push site.
    fn note_route_position(&mut self) {
        let open: std::collections::HashSet<usize> = self.open_tries.iter().map(|(i, _)| *i).collect();
        self.route_inside.entry(self.seq).or_default().extend(open);
    }

    /// R172 — record this expression if it is a construction. Called on EVERY expression the walk
    /// reaches, which is a superset of what `mark_escape` can reach from a root (that one descends
    /// only value positions; this one descends every child), so a site the escape walk finds is
    /// always in this table.
    fn note_ctor_site(&mut self, e: &'a syn::Expr) {
        if self.skip_sites.contains(&(e as *const syn::Expr as usize)) {
            return; // a callee path — see `skip_sites`
        }
        if let Some(l) = ctor_leaf_of_expr(e, self.uses, self.fields, self.returns) {
            let seq = self.seq;
            // R194 — every `?` whose operand encloses this construction OFF ITS VALUE SPINE has run it
            // by the time it takes its error exit, so it must veto this leaf whatever the pre-order
            // numbers say. Disjoint field borrows: the spine sets are read, the exits are written.
            let addr = e as *const syn::Expr as usize;
            let (open, exits) = (&self.open_tries, &mut self.try_exits);
            for (idx, spine) in open {
                if !spine.contains(&addr) {
                    exits[*idx].interior.insert(l.clone());
                }
            }
            self.first_ctor_seq.entry(l.clone()).or_insert(seq); // R173
            self.ctor_sites.entry(l).or_default().push((addr, 0));
            self.site_seq.insert((addr, 0), seq); // R189

        }
    }

    /// THE SITE HALF OF THE SHARED MACRO READING — the regions `note_macro_site`'s shallow token walk
    /// cannot address-key: the interiors of BLOCKS inside the tokens, tokens that only read as
    /// STATEMENTS, and a resolved `macro_rules!` TEMPLATE, whose construction is in the DEFINITION and
    /// appears nowhere in the invocation at all.
    ///
    /// WHY IT HAS TO EXIST. R172's site gate suppresses a leaf only when EVERY construction of it in
    /// the body is one of the escaping sites, and a construction it cannot see is not a site — so
    /// `out.push(mk_h!("a")); let h = H::try_new(m, "b")?; let _ = out; Ok(h)` had one site, the
    /// escaping one, and the gate certified the function pure while an `H::drop` really ran in that
    /// frame (executed: 1 in-frame drop; the direct twin `out.push(H::new("a"))` is charged). R199,
    /// R203, R204 and R210 each closed one spelling of this INSIDE a `?` operand and left it open
    /// outside one, because every walk they added is gated on `open_tries` and writes only
    /// `TryExit::interior`. This is not gated on anything.
    ///
    /// SAY WHICH DIRECTION IT FAILS IN. It writes `ctor_sites` and NOTHING else. Not `first_ctor_seq`
    /// — a leaf with no recorded position counts as live from the start, so CREATING one can suppress
    /// a charge. Adding sites can only make
    /// the gate's "every site escapes" test harder to satisfy: a site `mark_escape` also finds through
    /// `macro_reading_ordinals` is neutral, and one it does not find CHARGES.
    fn note_reading_sites(&mut self, reading: &MacroReading, base: usize) {
        let ords = macro_reading_ordinals(reading);
        let (uses, fields, returns) = (self.uses, self.fields, self.returns);
        let mut found: Vec<(String, usize)> = Vec::new();
        {
            let mut rec = |e: &syn::Expr| {
                if let Some(l) = ctor_leaf_of_expr(e, uses, fields, returns) {
                    if let Some(o) = ords.get(&(e as *const syn::Expr as usize)) {
                        found.push((l, *o));
                    }
                }
            };
            deep0_exprs(&reading.token_exprs, &mut rec);
            if let Some(b) = &reading.token_stmts {
                deep_exprs(b, &mut rec);
            }
            for arm in &reading.template_arms {
                deep_exprs(arm, &mut rec);
            }
        }
        if !found.is_empty() && std::env::var("CANDOR_ALIAS_DEBUG").is_ok() {
            for (l, _) in &found {
                eprintln!("R229MACROSITE {l}"); // §E1 HIT COUNTER
            }
        }
        for (l, o) in found {
            self.ctor_sites.entry(l).or_default().push((base, o));
            // R189 — a construction inside a macro's tokens is evaluated where the INVOCATION is, so
            // the invocation's own position is the site's position. `self.seq` is that position: this
            // runs from the walk of the `Expr::Macro` node itself.
            self.site_seq.insert((base, o), self.seq);
        }
        // NESTED READINGS. A macro invoked inside a macro is where R203 stopped, and its
        // constructions were the last shape neither walk could address-key. `ord_nested(j, inner)`
        // composes the child's own ordinal into this reading's space, and `mark_escape` composes it
        // the same way, so a nested construction that ESCAPES is still matched and stays uncharged.
        // ONE LEVEL of nesting: R203's own "nested macro" shape, and the template that reaches
        // its construction through a second template. A macro three deep keeps the pre-change answer,
        // which is stated in `soundness/known_open.tsv` rather than assumed away.
        {
            for (j, node) in nested_macro_nodes(reading).into_iter().enumerate() {
                let inner_m = match node {
                    NestedMacro::Expr(m) => syn::Expr::Macro(m.clone()),
                    // A statement macro's value is discarded, so no escape walk reaches it; its
                    // constructions are recorded as sites that can only charge.
                    NestedMacro::Stmt(m) => syn::Expr::Macro(syn::ExprMacro {
                        attrs: Vec::new(),
                        mac: m.mac.clone(),
                    }),
                };
                let syn::Expr::Macro(em) = &inner_m else { unreachable!() };
                let inner = {
                    let lens = MacroLens { local: self.local_macros, body: &self.body_macros };
                    macro_reading(em, &lens, &|n| self.macro_expanding.contains(n))
                };
                let iords = macro_reading_ordinals(&inner);
                let mut sub: Vec<(String, usize)> = Vec::new();
                {
                    let (uses, fields, returns) = (self.uses, self.fields, self.returns);
                    let mut rec = |e: &syn::Expr| {
                        if let Some(l) = ctor_leaf_of_expr(e, uses, fields, returns) {
                            if let Some(o) = iords.get(&(e as *const syn::Expr as usize)) {
                                sub.push((l, ord_nested(j, *o)));
                            }
                        }
                    };
                    // Region 0's own expressions too: unlike the OUTER reading, whose shallow
                    // region 0 is already site-recorded by `note_macro_site`, nothing has walked
                    // this one.
                    for e in &inner.token_exprs {
                        macro_shallow_exprs(e, &mut rec);
                    }
                    deep0_exprs(&inner.token_exprs, &mut rec);
                    if let Some(b) = &inner.token_stmts {
                        deep_exprs(b, &mut rec);
                    }
                    for arm in &inner.template_arms {
                        deep_exprs(arm, &mut rec);
                    }
                }
                for (l, o) in sub {
                    if std::env::var("CANDOR_ALIAS_DEBUG").is_ok() {
                        eprintln!("R229NESTED {l}"); // §E1 HIT COUNTER
                    }
                    self.ctor_sites.entry(l).or_default().push((base, o));
                    self.site_seq.insert((base, o), self.seq); // R189, as above
                }
            }
        }
    }

    /// One macro invocation, read ONCE for both halves of this walk.
    fn note_macro_reading(&mut self, m: &syn::ExprMacro) {
        let reading = {
            let lens = MacroLens { local: self.local_macros, body: &self.body_macros };
            macro_reading(m, &lens, &|n| self.macro_expanding.contains(n))
        };
        self.note_reading_sites(&reading, m as *const syn::ExprMacro as usize);
    }

    /// R172 — the leaves a MACRO body constructs, which `note_ctor_site` cannot address-key. Mirrors
    /// `mark_escape`'s macro handling (parse the tokens as a comma-punctuated expression list) and then
    /// walks the whole parsed subtree, deliberately wider than `mark_escape`'s value-position descent:
    /// over-recording here only widens the set that keeps the SHIPPED answer.
    fn note_macro_ctor_leaves(&mut self, m: &syn::ExprMacro, host: &syn::Expr) {
        // The SITE half — not gated on an open `?`, unlike everything below it.
        self.note_macro_reading(m);
        // SOUNDNESS R199 — SPINE MEMBERSHIP IS DECIDED IN TWO STEPS, because a macro's contents are
        // parsed out of TOKENS into a FRESH tree whose addresses exist in no spine set and never can:
        // `value_spine_addrs` walks the real body only.
        //   1. Is the MACRO NODE itself on this `?`'s value spine? `host` is the `syn::Expr::Macro` in
        //      the real body, which is what a spine can contain (`Ok(vec![H::new()])?` puts it there as
        //      a by-value argument). NB `base` is the `ExprMacro` PAYLOAD — a different address, used
        //      for site ordinals, never a spine key.
        //   2. If so, the macro's VALUE is on the spine, so the value positions INSIDE it are too — but
        //      only those. `idm!({ out.push(H::new()); gen(n) })?` is a macro node on the spine whose
        //      statement still runs before the error exit, so re-running the same spine rule over the
        //      parsed tree is what keeps that case charged. Judging the macro whole would exempt it.
        // A macro NOT on the spine gets no inner spine at all: everything it builds is interior.
        let on_spine: Vec<bool> = {
            let addr = host as *const syn::Expr as usize;
            self.open_tries.iter().map(|(_, spine)| spine.contains(&addr)).collect()
        };
        // SOUNDNESS R203 — TWO THINGS THE TOKEN RE-PARSE BELOW CANNOT SEE, and both of them lose a real
        // drop when the same LEAF is also built somewhere AFTER the `?`: that other site supplies a
        // `first_ctor_seq` greater than the `?`, so R173's positional filter keeps the leaf, and nothing
        // here ever put it in `interior`.
        //   (a) a crate-local `macro_rules!` whose TEMPLATE constructs. `mk_h!("a")`'s tokens are `"a"`
        //       — there is no construction in them at all; the construction is in the DEFINITION, which
        //       only the collector's R48 index has. So ask that authority rather than guessing.
        //   (b) tokens that are not an expression list — `stmts!(let x = H::new("a"); x)`. The `let`
        //       makes `parse_terminated` fail and this method used to return, silently.
        // Both are gated on an OPEN `?`: with none, there is no `interior` to write and no answer can
        // change, so the extra parsing never runs over the vast majority of macro invocations.
        if !self.open_tries.is_empty() {
            self.note_local_macro_template(m, &on_spine);
        }
        let parser = syn::punctuated::Punctuated::<syn::Expr, syn::Token![,]>::parse_terminated;
        let Ok(exprs) = syn::parse::Parser::parse2(parser, m.mac.tokens.clone()) else {
            if !self.open_tries.is_empty() {
                self.note_opaque_stmt_tokens(&m.mac.tokens, &on_spine);
            }
            return;
        };
        let base = m as *const syn::ExprMacro as usize;
        let mut inner_spine = std::collections::HashSet::new();
        if on_spine.iter().any(|b| *b) {
            for e in &exprs {
                value_spine_addrs(e, &mut inner_spine);
            }
        }
        let mut n = 0usize;
        for e in &exprs {
            self.note_macro_site(e, base, &on_spine, &inner_spine, &mut n);
        }
        // SOUNDNESS R203 — `note_macro_site` walks with `for_each_child_expr`, which STOPS AT A BLOCK
        // BOUNDARY, so `idm!({ out.push(H::new("a")); gen(n) })?` recorded nothing for the `H` inside the
        // braces. On its own that is harmless (a leaf with no recorded position is charged); paired with
        // a later VISIBLE `H::try_new(..)` it is the same silence as the template case. This second pass
        // is interior-only over the same spine sets, so it adds no site and no ordinal — the ordinal
        // numbering above must stay byte-identical to `macro_site_ordinals`, and this cannot touch it.
        if !self.open_tries.is_empty() {
            for e in &exprs {
                self.note_opaque_expr(e, &on_spine, &inner_spine);
            }
        }
    }
    /// The site-recording half of `macro_site_ordinals` — the SAME pre-order, the same stop at a
    /// nested macro, so ordinal `k` here is ordinal `k` in `mark_escape`'s independent parse.
    /// `on_spine[i]` is whether the enclosing macro node sits on the value spine of `open_tries[i]`;
    /// `inner_spine` is the value spine INSIDE the parsed tokens (empty unless some `on_spine[i]`).
    fn note_macro_site(
        &mut self,
        e: &syn::Expr,
        base: usize,
        on_spine: &[bool],
        inner_spine: &std::collections::HashSet<usize>,
        n: &mut usize,
    ) {
        if let syn::Expr::Macro(_) = e {
            // SOUNDNESS R229 — a nested macro's constructions are recorded as SITES by
            // `note_reading_sites`, under `ord_nested`, which `mark_escape` composes identically.
            // Before that they were recorded as a leaf-keyed LICENCE (`macro_ctor_leaves`) that made
            // the site gate certify the escape outright.
            // SOUNDNESS R203 — the interior walk below. It records no `first_ctor_seq` and no site, so
            // `{ out.extend(vec![vec![H::new("a")].pop().unwrap()]); gen(n) }?` lost the `H::drop` that
            // really runs on the error exit as soon as a later `H::try_new(..)` supplied a
            // `first_ctor_seq` past the `?`. The exemption test for the nested NODE is R199's two-step,
            // one level in.
            if !self.open_tries.is_empty() {
                let node_exempt = inner_spine.contains(&(e as *const syn::Expr as usize));
                let nested: Vec<bool> = on_spine.iter().map(|b| *b && node_exempt).collect();
                self.note_opaque_macro(e, &nested);
            }
            return;
        }
        // NUMBERED regardless (the ordinal must stay identical to `macro_site_ordinals`, which numbers
        // every node); only the SITE recording is skipped for a callee path.
        *n += 1;
        let ord = *n;
        if let syn::Expr::Call(c) = e {
            self.skip_sites.insert(&*c.func as *const syn::Expr as usize);
        }
        if self.skip_sites.contains(&(e as *const syn::Expr as usize)) {
            for_each_child_expr(e, &mut |c| self.note_macro_site(c, base, on_spine, inner_spine, n));
            return;
        }
        if let Some(l) = ctor_leaf_of_expr(e, self.uses, self.fields, self.returns) {
            let seq = self.seq; // R173 — the macro's own position; its contents evaluate there
            // SOUNDNESS R199 — the same insertion `note_ctor_site` makes, which this path never did.
            // A macro-borne construction inside a `?`'s operand is numbered at the MACRO's position,
            // which is inside the operand and therefore GREATER than the pre-order `?`, so R173's
            // positional filter reads it as "not built yet" and keeps it in the escaping set; and
            // because nothing here ever consulted `open_tries`, R194's operand-interior term — the
            // whole point of which is that position cannot express this — never saw it either.
            // `{ out.extend(vec![H::new()]); gen(n) }?` therefore lost the `H::drop` that really runs
            // when `gen` fails (executed: one drop in that frame), a REGRESSION against published
            // 0.34.0, which vetoed blanket. Denylist direction: OFF the spine unless the macro node
            // itself is on it, and `interior` can only ever veto MORE.
            let exempt = inner_spine.contains(&(e as *const syn::Expr as usize));
            let (open, exits) = (&self.open_tries, &mut self.try_exits);
            for ((idx, _), spined) in open.iter().zip(on_spine) {
                if !(*spined && exempt) {
                    exits[*idx].interior.insert(l.clone());
                    self.macro_interior_leaves.insert(l.clone());
                }
            }
            self.first_ctor_seq.entry(l.clone()).or_insert(seq);
            self.ctor_sites.entry(l).or_default().push((base, ord));
            self.site_seq.insert((base, ord), seq); // R189
        }
        for_each_child_expr(e, &mut |c| self.note_macro_site(c, base, on_spine, inner_spine, n));
    }

    // ─── SOUNDNESS R203 ──────────────────────────────────────────────────────────────────────────
    // A construction `lang.rs` cannot ADDRESS-KEY still has to reach the open `?`s' `interior`.
    //
    // R173 made the `?` veto positional and R194/R199 added `interior` for what the operand builds off
    // its value spine. Both read a table keyed by the constructions the walk can SEE. Three shapes are
    // invisible to it — a `macro_rules!` TEMPLATE (the construction is in the definition, not in the
    // invocation's tokens), a token stream that is not an expression list, and a NESTED macro (where the
    // ordinal walk deliberately stops) — and each of them, on its own, is harmless: a leaf with NO
    // recorded position counts as live from the start and is charged. What makes them a CARDINAL SIN is
    // the pair: build the leaf invisibly inside the operand AND visibly again after the `?`, and the
    // visible site's `first_ctor_seq` carries the leaf past the filter while nothing put it in
    // `interior`. `{ out.push(mk_h!("a")); gen(n) }?; let h = H::try_new(m, "b")?;` executed one
    // in-frame `H::drop` and published 0.34.0 charged it; `e9cdd23` stopped.
    //
    // SAY WHICH DIRECTION THIS FAILS IN. Everything below writes ONLY `TryExit::interior`, and
    // `interior` appears in exactly one place — a `retain` that can only REMOVE a leaf from the escaping
    // set, i.e. charge a drop that was not charged. It never writes `first_ctor_seq` or `ctor_sites`,
    // either of which can LICENSE an escape. So the failure mode of an over-reaching
    // walk here is an over-charge, never a new silence; the over-charge is what the corpus A/B and the
    // six ABSENT controls measure. The exemptions are enumerated (the value spine, as everywhere else in
    // R194) and every shape not enumerated stays charged — denylist, not allowlist.

    /// R203 — the `interior` insertion, for a construction with no address the site tables can hold.
    fn note_opaque_interior_leaf(&mut self, l: &str, on_spine: &[bool], exempt: bool) {
        let (open, exits, marks) =
            (&self.open_tries, &mut self.try_exits, &mut self.opaque_interior_leaves);
        for ((idx, _), spined) in open.iter().zip(on_spine) {
            if !(*spined && exempt) {
                exits[*idx].interior.insert(l.to_string());
                marks.insert(l.to_string());
            }
        }
    }

    /// R203 — a macro's tokens (or a `macro_rules!` template body) walked for CONSTRUCTIONS only.
    /// `inner` is the value spine inside this parse, empty unless the macro node itself is on some open
    /// `?`'s spine. Deliberately deeper than `for_each_child_expr`, which stops at a block boundary: a
    /// macro that constructs almost always does it inside `{ .. }`, and a wider veto only charges more.
    fn note_opaque_expr(
        &mut self,
        e: &syn::Expr,
        on_spine: &[bool],
        inner: &std::collections::HashSet<usize>,
    ) {
        if let syn::Expr::Macro(_) = e {
            let node_exempt = inner.contains(&(e as *const syn::Expr as usize));
            let nested: Vec<bool> = on_spine.iter().map(|b| *b && node_exempt).collect();
            self.note_opaque_macro(e, &nested);
            return;
        }
        if let Some(l) = ctor_leaf_of_expr(e, self.uses, self.fields, self.returns) {
            let exempt = inner.contains(&(e as *const syn::Expr as usize));
            self.note_opaque_interior_leaf(&l, on_spine, exempt);
        }
        match e {
            // The callee PATH of a call is the constructor's own NAME, not a second construction —
            // `skip_sites`' reason, applied by not descending into it (these addresses belong to a token
            // tree that dies with this call, so they cannot go in the persistent set).
            syn::Expr::Call(c) => c.args.iter().for_each(|a| self.note_opaque_expr(a, on_spine, inner)),
            syn::Expr::Block(b) => self.note_opaque_block(&b.block, on_spine, inner),
            syn::Expr::Unsafe(u) => self.note_opaque_block(&u.block, on_spine, inner),
            syn::Expr::Async(a) => self.note_opaque_block(&a.block, on_spine, inner),
            syn::Expr::TryBlock(t) => self.note_opaque_block(&t.block, on_spine, inner),
            syn::Expr::Loop(l) => self.note_opaque_block(&l.body, on_spine, inner),
            syn::Expr::ForLoop(f) => {
                self.note_opaque_expr(&f.expr, on_spine, inner);
                self.note_opaque_block(&f.body, on_spine, inner);
            }
            syn::Expr::While(w) => {
                self.note_opaque_expr(&w.cond, on_spine, inner);
                self.note_opaque_block(&w.body, on_spine, inner);
            }
            syn::Expr::If(i) => {
                self.note_opaque_expr(&i.cond, on_spine, inner);
                self.note_opaque_block(&i.then_branch, on_spine, inner);
                if let Some((_, els)) = &i.else_branch {
                    self.note_opaque_expr(els, on_spine, inner);
                }
            }
            syn::Expr::Closure(c) => self.note_opaque_expr(&c.body, on_spine, inner),
            syn::Expr::Const(c) => self.note_opaque_block(&c.block, on_spine, inner),
            _ => for_each_child_expr(e, &mut |c| self.note_opaque_expr(c, on_spine, inner)),
        }
    }

    /// R203 — the statement half of the walk above.
    fn note_opaque_block(
        &mut self,
        b: &syn::Block,
        on_spine: &[bool],
        inner: &std::collections::HashSet<usize>,
    ) {
        for st in &b.stmts {
            match st {
                syn::Stmt::Local(l) => {
                    if let Some(init) = &l.init {
                        self.note_opaque_expr(&init.expr, on_spine, inner);
                        if let Some((_, d)) = &init.diverge {
                            self.note_opaque_expr(d, on_spine, inner);
                        }
                    }
                }
                syn::Stmt::Expr(e, _) => self.note_opaque_expr(e, on_spine, inner),
                // SOUNDNESS R204 — a macro in STATEMENT position, walked THE SAME WAY `walk_block` walks
                // one. R203's own summary said statement-position macros go to the interior walk; this
                // arm did not, it went straight to `note_opaque_stmt_tokens`, so the TEMPLATE lookup and
                // the expression-list parse were both skipped one level in — inside a template body,
                // inside block tokens, inside statement tokens. Its value is discarded, so nothing in it
                // is exempt (the all-`false` spine), which is exactly what the old call passed too:
                // `note_opaque_stmt_tokens` hands `note_opaque_block` an EMPTY `inner`, so every leaf it
                // found was already unexempt. The new call is therefore a superset of the old one — same
                // statement-token walk on the `Err` arm, plus the two paths that were missing.
                syn::Stmt::Macro(m) => {
                    if std::env::var("CANDOR_ALIAS_DEBUG").is_ok() {
                        eprintln!("R204STMTMACRO {}", path_to_string(&m.mac.path));
                    }
                    let none = vec![false; on_spine.len()];
                    let host =
                        syn::Expr::Macro(syn::ExprMacro { attrs: Vec::new(), mac: m.mac.clone() });
                    self.note_opaque_macro(&host, &none);
                }
                // SOUNDNESS R206, SECOND INSTANCE — THE AUDIT BOUNDARY, DRAWN PAST ITS OWN TRIGGER.
                // R206 taught `walk_block` that a body-local `macro_rules!` is a `Stmt::Item` nobody
                // records. This is the SAME walk one level in — over a template body, block tokens or
                // statement tokens the veto had to re-parse — and it discarded `Stmt::Item` for the
                // same reason, so `idm!({ macro_rules! lm { .. } out.push(lm!("a")); gen(n) })?`
                // stayed silent while the body-level spelling beside it was closed. That is exactly
                // the R203 -> R204 relationship, one row over. Executed: 1 in-frame drop on the error
                // exit; published 0.34.0 charges it, `c22a31d` and R206 alone do not.
                //
                // Same map, same direction as `walk_block`'s arm: an item-position macro INVOCATION
                // carries no `ident` and is skipped, and a definition seen here can outlive the token
                // block it was written in, which over-charges. This walk writes `TryExit::interior`
                // only, so over-reach cannot silence.
                syn::Stmt::Item(syn::Item::Macro(im))
                    if im.ident.is_some() && im.mac.path.is_ident("macro_rules") =>
                {
                    if let Some(id) = &im.ident {
                        if std::env::var("CANDOR_ALIAS_DEBUG").is_ok() {
                            eprintln!("R206BODYMACRO2 {id}");
                        }
                        self.body_macros.insert(id.to_string(), im.mac.tokens.to_string());
                    }
                }
                syn::Stmt::Item(_) => {}
            }
        }
    }

    /// R203 — one macro invocation, walked for constructions: its INVOCATION tokens (as an expression
    /// list, else as statements) and, for a crate-local `macro_rules!`, its DEFINITION template.
    fn note_opaque_macro(&mut self, e: &syn::Expr, on_spine: &[bool]) {
        let syn::Expr::Macro(m) = e else { return };
        if self.open_tries.is_empty() {
            return;
        }
        self.note_local_macro_template(m, on_spine);
        let parser = syn::punctuated::Punctuated::<syn::Expr, syn::Token![,]>::parse_terminated;
        match syn::parse::Parser::parse2(parser, m.mac.tokens.clone()) {
            Ok(exprs) => {
                let mut inner = std::collections::HashSet::new();
                if on_spine.iter().any(|b| *b) {
                    for x in &exprs {
                        value_spine_addrs(x, &mut inner);
                    }
                }
                for x in &exprs {
                    self.note_opaque_expr(x, on_spine, &inner);
                }
            }
            Err(_) => self.note_opaque_stmt_tokens(&m.mac.tokens, on_spine),
        }
    }

    /// R203(b) — tokens that are not an expression list at all. `syn` will still read them as a
    /// STATEMENT sequence (`Block::parse_within`), which is what `stmts!(let x = H::new("a"); x)` is;
    /// ask it rather than treating the macro as empty. NOTHING inside is exempt: a token stream we could
    /// not read as an expression tells us nothing about which position carries the macro's VALUE, and
    /// guessing one would be an exemption — the direction that loses drops.
    fn note_opaque_stmt_tokens(&mut self, tokens: &proc_macro2::TokenStream, on_spine: &[bool]) {
        let Ok(stmts) = syn::parse::Parser::parse2(syn::Block::parse_within, tokens.clone()) else {
            return; // genuinely unreadable (`quote!{ #x }`, a `macro_rules!` arm list) — R203's residual
        };
        let b = syn::Block { brace_token: Default::default(), stmts };
        self.note_opaque_block(&b, on_spine, &std::collections::HashSet::new());
    }

    /// R203(a) — a crate-local `macro_rules!` whose TEMPLATE constructs. ASK THE AUTHORITY: this reads
    /// the SAME index the collector's R48 inline expansion reads (`local_macros`, via
    /// `macro_template_blocks`), so the two cannot drift about what a template CONTAINS.
    ///
    /// IT DIVERGES FROM R48 ON MULTI-ARM, AND ONLY HERE — DELIBERATELY, BECAUSE THE TWO PATHS ASK
    /// OPPOSITE QUESTIONS. R48 expands a template to RESOLVE CALL EDGES, i.e. to ADD an effect, so a
    /// non-matching arm fabricates one and it stops at multi-arm; that rule is untouched, and the
    /// resolution side still has exactly one authority. This path only ever adds to `TryExit::interior`,
    /// which is a REFUSAL to certify that a leaf escaped — over a macro whose arms disagree about
    /// whether they construct, "some arm builds it" is the over-approximating answer, and it is the
    /// answer published 0.34.0 gave, because it vetoed blanket. So every parseable arm is walked.
    ///
    /// SAY WHAT IT COSTS, MEASURED ON BOTH SIDES. It closes `r_multiarm_hole` — `{ out.push(two!(a
    /// "a")); gen(n) }?` with the leaf built again after the `?`, executed 1 in-frame drop, charged by
    /// published 0.34.0 and silent from R173 to `5cefa62`. It over-charges `c_multiarm_pure`, whose
    /// invocation matches an arm that constructs nothing (executed 0 in-frame drops) — an over-charge
    /// the published build already had, and a fabrication against `5cefa62`. Both cells live in the
    /// fixture. The 1,504-crate registry A/B is byte-identical either way, so the corpus cannot choose;
    /// the ruling is that a SILENCE against published outranks an over-charge AT published parity.
    fn note_local_macro_template(&mut self, m: &syn::ExprMacro, on_spine: &[bool]) {
        let full = path_to_string(&m.mac.path);
        // SOUNDNESS R205 — A PATH IS STILL A NAME. `$crate::mk_h!(..)` is the canonical hygienic spelling
        // for one exported macro calling a helper in the same crate, and `strip_dollars` renders it
        // `crate::mk_h` — so this lookup, keyed on the whole path, refused exactly the spelling the
        // ecosystem uses and the veto went blind to what those templates build. The index (R48's
        // `local_macros`) is keyed by BARE NAME, so resolve the path's LEAF against it.
        //
        // SAY WHICH DIRECTION IT FAILS IN. The leaf is not proof the macro is local: `dep::mk_h!` from
        // another crate whose leaf happens to collide with a local `macro_rules! mk_h` resolves here too,
        // and walks the wrong template. That is deliberate and it is the safe direction — this function
        // only ever adds to `TryExit::interior`, i.e. REFUSES to certify that a leaf escaped, and the
        // branch it replaces added NOTHING at all, so every outcome of a wrong resolution is an
        // over-charge. A leaf whose name is in no local index is external and stays R203's residual.
        // (Which module's template a bare name resolves to is R208's open question, not this one's: the
        // index is last-writer-wins across the crate, here and for R48's call-edge resolution alike.)
        let name = if full.contains("::") {
            let leaf = full.rsplit("::").next().unwrap_or_default().to_string();
            if !(self.local_macros.contains_key(&leaf) || self.body_macros.contains_key(&leaf)) {
                return;
            }
            leaf
        } else {
            full
        };
        if self.macro_expanding.contains(&name) {
            return;
        }
        // §E1 counter, printed HERE and not at the resolution above so that it counts templates this
        // branch actually WALKS. `tokio-util`'s own `macro_rules! trace` invokes the `tracing` crate's
        // `tracing::trace!` in its template; the leaf rule resolves that back to the local `trace` — the
        // wrong template, harmlessly — and R48's recursion guard stops it one line up. Counting the
        // resolution rather than the walk reported that as three hits that do nothing.
        if name != path_to_string(&m.mac.path) && std::env::var("CANDOR_ALIAS_DEBUG").is_ok() {
            eprintln!("R205PATHTMPL {}", path_to_string(&m.mac.path));
        }
        // SOUNDNESS R206 — A BODY-LOCAL DEFINITION IS A DEFINITION. `macro_rules!` written inside a fn
        // body is a `Stmt::Item`: `decls.rs` indexes item-level definitions only and `walk_block` threw
        // `Stmt::Item` away, so its template was in no index and the veto could not see what it builds.
        //
        // BOTH BODIES ARE WALKED, NEVER ONE INSTEAD OF THE OTHER, and that is the whole of the care this
        // needs. Rust's own rule is that a body-local `macro_rules!` SHADOWS a crate-level one of the
        // same name for the rest of that body, so "consult the overlay first" reads like the correct
        // model — but it is the one shape that could turn this into a new SILENCE: the overlay map is
        // filled per BODY, not per block, so an entry from an earlier block would suppress the
        // crate-level template that the later block really uses, and a template that constructs would be
        // replaced by one that does not. Walking both is a strict superset of what shipped: every leaf
        // `c22a31d` put in `interior` is still put there, and a shadowing pair adds the other template's
        // leaves as an over-charge. This term can only refuse to certify an escape, so a superset is the
        // direction it is allowed to be wrong in.
        let mut bodies: Vec<String> = Vec::new();
        if let Some(b) = self.body_macros.get(&name) {
            if std::env::var("CANDOR_ALIAS_DEBUG").is_ok() {
                eprintln!("R206BODYMACRO {name}");
            }
            bodies.push(b.clone());
        }
        if let Some(b) = self.local_macros.get(&name) {
            if !bodies.contains(b) {
                bodies.push(b.clone());
            }
        }
        if bodies.is_empty() {
            return;
        }
        // `macro_template_blocks_flat` returns (total arms, the arms that PARSED). An arm that does not
        // parse contributes nothing here, exactly as it contributes nothing to R48 — an unreadable
        // template is a residual either way, never a fabrication.
        //
        // SOUNDNESS R207 — THE FLATTENED TRANSFORM, VETO SIDE ONLY. `strip_dollars` leaves a repetition
        // as `( X ) *`, which parses as no statement, so the whole arm was dropped and
        // `{{ $( $o.push($crate::H::new($p)); )* }}` — the ordinary way to write a template that pushes
        // — was as invisible as an unreadable one. `macro_template_blocks_flat` reads `$( X ) sep? *`
        // as `X`. That is not what any single expansion produces (a repetition can run zero times), and
        // it is the direction this term is allowed to be wrong in: everything here lands in
        // `TryExit::interior`, a REFUSAL to certify an escape. R48's resolution keeps the unflattened
        // `macro_template_blocks`, where the same reading would ADD an effect instead of withholding
        // one. The arm-splitting walk is shared, so the two cannot disagree about arm COUNT.
        let blocks: Vec<syn::Block> =
            bodies.iter().flat_map(|b| crate::collector::macro_template_blocks_flat(b).1).collect();
        self.macro_expanding.insert(name.clone());
        for b in &blocks {
            // Each arm's VALUE is its tail expression, so when the invocation sits on a `?`'s spine that
            // arm's tail is on the spine too and ONLY the tail is exempt — R199's step 2, over the
            // definition instead of the invocation. Every statement of every arm stays interior, which
            // is what `tmpl_spine_stmt_hole` pins.
            let mut inner = std::collections::HashSet::new();
            if on_spine.iter().any(|x| *x) {
                if let Some(t) = tail_expr_of(b) {
                    value_spine_addrs(t, &mut inner);
                }
            }
            self.note_opaque_block(b, on_spine, &inner);
        }
        self.macro_expanding.remove(&name);
    }

    /// `tail` says whether this block's trailing expression is in RETURN position for the unit.
    fn walk_block(&mut self, b: &'a syn::Block, tail: bool) {
        for (i, st) in b.stmts.iter().enumerate() {
            let last = i + 1 == b.stmts.len();
            match st {
                syn::Stmt::Local(l) => {
                    // SOUNDNESS R325 — EVERY name this statement binds, whatever the pattern shape and
                    // whether or not there is an initialiser. Counting only `single_pat_ident` inside
                    // the `if let Some(init)` saw 2 of at least 8 binder forms, and an UNDER-count makes
                    // `one_entity` wrongly TRUE — the FABRICATION direction. `let (a, b) = ..`,
                    // `let Some(x) = .. else`, and a bare `let x;` all bind names the guard must see.
                    self.note_pat_bindings(&l.pat);
                    if let Some(init) = &l.init {
                        // R198/R209(b) — see `let_bound_closures`. Must be recorded BEFORE the walk,
                        // because the walk is what reaches the `Expr::Closure` arm that reads it.
                        if matches!(&*init.expr, syn::Expr::Closure(_)) {
                            if let Some(n) = single_pat_ident(&l.pat) {
                                self.let_bound_closures
                                    .insert(&*init.expr as *const syn::Expr as usize, n);
                            }
                        }
                        // `let _ = f();` discards the value exactly as `f();` does — but ONLY in THIS
                        // frame. See the `closure_depth` note on the statement-call counter below.
                        if self.closure_depth == 0 && matches!(l.pat, syn::Pat::Wild(_)) {
                            if let syn::Expr::Call(c) = &*init.expr {
                                if let syn::Expr::Path(p) = &*c.func {
                                    if let Some(id) = p.path.get_ident() {
                                        *self
                                            .discarded_calls
                                            .entry(id.to_string())
                                            .or_insert(0) += 1;
                                    }
                                }
                            }
                        }
                        self.walk_expr(&init.expr);
                        if let Some((_, d)) = &init.diverge {
                            self.walk_expr(d);
                        }
                        // R173 — the binder's position is AFTER its initialiser: in
                        // `let h = try_handle(p)?;` the `?` can take its early exit before `h` exists,
                        // so `h` is not live at that exit. Recorded after the walk for exactly that.
                        if let Some(n) = single_pat_ident(&l.pat) {
                            let seq = self.seq;
                            self.first_bind_seq.entry(n.clone()).or_insert(seq);
                            self.lets.push((n, &init.expr));
                        }
                    }
                }
                syn::Stmt::Expr(e, semi) => {
                    if tail && last && semi.is_none() {
                        // R189 — the TAIL is evaluated after every statement before it, so
                        // `self.seq` here is greater than every binding's position, which is what makes
                        // the tail root veto every one of them (the shipped behaviour).
                        self.roots.push((self.seq, Some(e)));
                    }
                    // `f();` — the value is discarded, so anything the callee built dies here.
                    //
                    // SOUNDNESS R303: `self.closure_depth == 0` is load-bearing, and leaving it out was a
                    // FABRICATION. A closure captured by ANOTHER closure that invokes it and discards the
                    // result — `let f = || H::new(); Box::new(move || { f(); })` — satisfied
                    // `uses == disc` at depth 1, so the suppression was withdrawn and the `Drop` charged
                    // to a frame that never runs it (executed: 0 in-frame drops). `mark_escape` cannot
                    // rescue it either: `for_each_value_child`'s `Block` arm descends only the TAIL, so an
                    // `f` in a non-tail statement never reaches `m.names` and the `lets` fixpoint never
                    // re-marks the closure. A discarded call inside a nested closure says nothing about
                    // whether THIS frame consumes the value, so it must not count as one.
                    if self.closure_depth == 0 && semi.is_some() {
                        if let syn::Expr::Call(c) = e {
                            if let syn::Expr::Path(p) = &*c.func {
                                if let Some(id) = p.path.get_ident() {
                                    *self.discarded_calls.entry(id.to_string()).or_insert(0) += 1;
                                }
                            }
                        }
                    }
                    self.walk_expr(e);
                }
                // SOUNDNESS R203 — a macro in STATEMENT position is not an `Expr::Macro`, so the whole
                // macro machinery above (`note_macro_ctor_leaves` and everything R199 added to it) never
                // sees it: `{ push_h!(out, "a"); gen(n) }?` was silent for the same reason a
                // `macro_rules!` template is, one spelling over. Its value is DISCARDED, so nothing in
                // it is on any spine and everything it builds is interior. Interior only — never a site,
                // never a `first_ctor_seq`, so this can only charge more.
                // R304 — a body-local `fn NAME` binds that name too, and shadowing a let-bound
                // closure with one (or vice versa) is the same collision.
                syn::Stmt::Item(syn::Item::Fn(itf)) => {
                    *self
                        .name_bindings
                        .entry(itf.sig.ident.to_string())
                        .or_insert(0) += 1;
                }
                syn::Stmt::Macro(m) => {
                    self.note_macro_idents(m.mac.tokens.clone());
                    // The SITE half, not gated on an open `?`. A statement macro's VALUE is discarded,
                    // so `mark_escape` never reaches it from any root and every site recorded here is
                    // a non-escaping one — which is the point: `push_h!(out, "a")` builds an `H` that
                    // dies with `out`, and without a site the R172 gate certified the body pure
                    // whenever some OTHER `H` escaped. Keyed on the `StmtMacro`'s own address, which no
                    // `ExprMacro` shares.
                    let host = syn::Expr::Macro(syn::ExprMacro { attrs: Vec::new(), mac: m.mac.clone() });
                    let syn::Expr::Macro(em) = &host else { unreachable!() };
                    let reading = {
                        let lens = MacroLens { local: self.local_macros, body: &self.body_macros };
                        macro_reading(em, &lens, &|n| self.macro_expanding.contains(n))
                    };
                    self.note_reading_sites(&reading, m as *const syn::StmtMacro as usize);
                    if !self.open_tries.is_empty() {
                        let on_spine = vec![false; self.open_tries.len()];
                        self.note_opaque_macro(&host, &on_spine);
                    }
                }
                // SOUNDNESS R206 — the only `Stmt::Item` this walk reads: a body-local `macro_rules!`
                // DEFINITION, recorded for `note_local_macro_template`. An item-position macro
                // INVOCATION (`foo!();`) carries no `ident` and is skipped, the same test `decls.rs`
                // uses for the crate-level index. Nothing else about `Stmt::Item` changes.
                syn::Stmt::Item(syn::Item::Macro(im))
                    if im.ident.is_some() && im.mac.path.is_ident("macro_rules") =>
                {
                    if let Some(id) = &im.ident {
                        self.body_macros.insert(id.to_string(), im.mac.tokens.to_string());
                    }
                }
                syn::Stmt::Item(_) => {}
            }
        }
    }
    /// Every identifier in a macro's tokens, flattened through nested groups. R304: the escape walk
    /// does not read macro tokens as expressions, so a use spelled there is invisible to `path_uses`.
    fn note_macro_idents(&mut self, ts: proc_macro2::TokenStream) {
        for t in ts {
            match t {
                proc_macro2::TokenTree::Ident(i) => {
                    self.macro_idents.insert(i.to_string());
                }
                proc_macro2::TokenTree::Group(g) => self.note_macro_idents(g.stream()),
                _ => {}
            }
        }
    }

    /// Every identifier a PATTERN binds, recursively. SOUNDNESS R325: the guard's key is a bare name,
    /// so it must know every binder form or its count is wrong in the FABRICATION direction — tuple and
    /// struct destructuring, `ref`/`mut`, slices, `|` alternatives, and the `@` sub-binding.
    fn note_pat_bindings(&mut self, p: &syn::Pat) {
        match p {
            syn::Pat::Ident(i) => {
                *self.name_bindings.entry(i.ident.to_string()).or_insert(0) += 1;
                if let Some((_, sub)) = &i.subpat {
                    self.note_pat_bindings(sub);
                }
            }
            syn::Pat::Tuple(t) => t.elems.iter().for_each(|q| self.note_pat_bindings(q)),
            syn::Pat::TupleStruct(t) => t.elems.iter().for_each(|q| self.note_pat_bindings(q)),
            syn::Pat::Struct(t) => t.fields.iter().for_each(|f| self.note_pat_bindings(&f.pat)),
            syn::Pat::Slice(t) => t.elems.iter().for_each(|q| self.note_pat_bindings(q)),
            syn::Pat::Or(t) => t.cases.iter().for_each(|q| self.note_pat_bindings(q)),
            syn::Pat::Reference(r) => self.note_pat_bindings(&r.pat),
            syn::Pat::Type(t) => self.note_pat_bindings(&t.pat),
            syn::Pat::Paren(t) => self.note_pat_bindings(&t.pat),
            _ => {}
        }
    }

    fn walk_expr(&mut self, e: &'a syn::Expr) {
        // R198/R209(b) — every single-ident path occurrence, counted, so the deferred decision below
        // can ask whether a let-bound closure's name is used for anything BUT a value-discarded call.
        if let syn::Expr::Path(p) = e {
            if p.qself.is_none() {
                if let Some(id) = p.path.get_ident() {
                    *self.path_uses.entry(id.to_string()).or_insert(0) += 1;
                }
            }
        }
        self.seq += 1; // R173 — pre-order position, read only for its ORDER
        self.note_ctor_site(e); // R172
        if let syn::Expr::Macro(m) = e {
            self.note_macro_idents(m.mac.tokens.clone());
            self.note_macro_ctor_leaves(m, e); // R172 — address-keying cannot reach inside a macro
        }
        match e {
            syn::Expr::Return(r) => match &r.expr {
                // R189 — the `return`'s OWN pre-order position, which is before its operand is
                // walked: the exit is taken there, so a `let` bound later is not live at it.
                Some(v) => self.roots.push((self.seq, Some(v))),
                // Bare `return;` — this exit is `()`-typed and provably carries nothing out.
                None => self.roots.push((self.seq, None)),
            },
            // `expr?`'s implicit early-return is a genuine, separate exit of the FUNCTION (not just
            // this block), and it carries only the error/`None` residual — never a name bound earlier in
            // this scope. Left unmodelled, a construction that only escapes on the SUCCESS continuation
            // (`let g = Guard::new(); fallible()?; Some(g)`) read as escaping unconditionally, because
            // the only root the old flat-union walk saw was the success-path tail. Recorded regardless of
            // this Try's own position (tail or not): every `?` is a possible early exit the moment it is
            // evaluated.
            // R173 — recorded WITH ITS POSITION rather than as a blanket empty root. Pre-order puts a
            // `?` BEFORE its own operand, which is what makes `let c = make()?;` read as "the value
            // does not exist yet at this exit" while `let g = G::new(); f()?;` reads as "it does".
            syn::Expr::Try(t) => {
                let e = TryExit {
                    seq: self.seq,
                    src_seq: self.seq,
                    in_closure: self.closure_depth > 0,
                    interior: std::collections::HashSet::new(),
                };
                self.try_exits.push(e);
                // R194 — open this `?`'s operand scope for the child walk below.
                let mut spine = std::collections::HashSet::new();
                value_spine_addrs(&t.expr, &mut spine);
                self.open_tries.push((self.try_exits.len() - 1, spine));
            }
            syn::Expr::Assign(a) => match &*a.left {
                // `x = Guard::new()` — an escape only if `x` itself escapes. A MULTI-segment path is a
                // static/const (`GLOBAL = …`), which is storage this scope does not own.
                syn::Expr::Path(p) if p.qself.is_none() => {
                    self.assigns
                        .push((p.path.get_ident().map(|i| i.to_string()), &a.right, self.seq));
                }
                // `self.g = …` / `xs[i] = …` / `*p = …` — stored somewhere this scope does not own.
                syn::Expr::Field(_) | syn::Expr::Index(_) | syn::Expr::Unary(_) => {
                    self.assigns.push((None, &a.right, self.seq));
                    self.note_route_position(); // R709
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
                // R172 — the callee is not a construction site of its own, whatever it names.
                self.skip_sites.insert(&*c.func as *const syn::Expr as usize);
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
                            self.escapes.push((self.seq, a));
                        }
                        self.note_route_position(); // R709
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
            syn::Expr::Closure(c) => {
                // A closure bound to a name is NOT an unconditional escape: `let f = || H::new("a");
                // let _ = f();` invokes it here and the value dies here, so charging is correct and the
                // old unconditional push certified the body pure. Whether it really escapes is decided
                // by the NAME, and `escape_from_root`'s `lets` fixpoint already answers that — it calls
                // `mark_escape` on this very closure expression when the name escapes, and
                // `for_each_value_child` now descends a closure's body so that reaches the
                // construction. An INLINE closure keeps the unconditional push: its value is handed to
                // something this scope cannot follow (sharded-slab's `shard.with_slot(key, |slot| …
                // Some(Entry { .. }))`, the case this arm was written for), so it must stay suppressed.
                match self
                    .let_bound_closures
                    .get(&(e as *const syn::Expr as usize))
                    .cloned()
                {
                    Some(name) => self.pending_closure_escapes.push((name, &c.body, self.seq)),
                    None => self.escapes.push((self.seq, &c.body)),
                }
                self.note_route_position(); // R709 — a let-bound closure can become a route later
            }
            _ => {}
        }
        // SOUNDNESS R187 — A `?` INSIDE A LOOP IS LIVE FOR EVERYTHING THAT LOOP BODY BUILDS. R173 gave
        // each `?` a PRE-ORDER number and read it as evaluation order; in a loop the two disagree, because
        // the body runs again. `for it in items { let v = it?; out.push(H::new(v)); } Ok(out)` numbers the
        // `?` BEFORE the `H::new` it precedes in the source, so the positional filter kept `H` in the
        // escaping set and the drop that really runs on iteration 2 (executed: one `H::drop` in that frame)
        // was never charged — silent, and a REGRESSION against published 0.34.0, which vetoed blanket.
        // So when the loop closes, every `?` recorded inside it (condition INCLUDED — `while step(&mut n)?`
        // is re-evaluated after the body has built its value) moves to the loop's LAST position. That is
        // the smallest change that models re-entry: it can only ever move a `?` LATER, i.e. veto MORE, the
        // over-charging direction, and a construction AFTER the loop still outranks it, which is what keeps
        // R173's straight-line gains (`order`, `order_type`, `order_between`) and its loop-shaped sibling
        // (`for x in v { g(x)?; } let h = H::new(); Ok(h)`) intact.
        // A `?` in a CLOSURE inside the loop is NOT this function's exit and is left where it was — see
        // `closure_depth`. Nested loops rewrite twice, outermost last, which is correct and monotone.
        let loop_mark = matches!(e, syn::Expr::ForLoop(_) | syn::Expr::While(_) | syn::Expr::Loop(_))
            .then(|| self.try_exits.len());
        // SOUNDNESS R324 — an ASYNC BLOCK defers execution exactly as a closure does, and
        // `for_each_child_block` descends `Async(x) => f(&x.block)`, so its statements were walked at
        // depth 0. R303 guarded the closure spelling of its own shape and left the async one open:
        // `let f = || H::new(); async move { f(); }` counted a discarded call as "in this frame",
        // withdrew the suppression, and charged a drop that happens in the POLLER's frame — executed,
        // 0 in-frame drops. `closure_depth` names DEFERRAL, not the `Closure` node.
        let in_closure = matches!(e, syn::Expr::Closure(_) | syn::Expr::Async(_));
        if in_closure {
            self.closure_depth += 1;
        }
        for_each_child_expr(e, &mut |c| self.walk_expr(c));
        // A nested block is walked ONLY to find further escape SITES (`return`, an assignment, a method
        // call on an escaping name). Its trailing expression is NOT a root: `let x = if c { Guard::new()
        // } else { … };` has an if-block whose tail is a value, but the value lands in `x`, not in the
        // caller. Treating every nested tail as a return made the ternary position silent in all five
        // construction spellings. Tail position propagates through `mark_escape`'s value-child walk
        // instead, which only ever descends from a genuine root.
        for_each_child_block(e, &mut |b| self.walk_block(b, false));
        if in_closure {
            self.closure_depth -= 1;
        }
        if matches!(e, syn::Expr::Try(_)) {
            self.open_tries.pop(); // R194 — the operand scope closes with the `?`
        }
        if let Some(mark) = loop_mark {
            let end = self.seq; // every position inside this loop is <= this one
            for t in &mut self.try_exits[mark..] {
                if !t.in_closure && t.seq < end {
                    // §E1 HIT COUNTER — R187's branch is "a `?` inside a loop moved to the loop's end".
                    if std::env::var("CANDOR_ALIAS_DEBUG").is_ok() {
                        eprintln!("R187LOOPQ {} -> {end}", t.seq);
                    }
                    t.seq = end;
                }
            }
        }
    }
}

/// Mark one escaping expression: record any construction leaf it builds, any local NAME it hands
/// out, and recurse through the value positions that carry a value outward.
fn mark_escape(
    e: &syn::Expr,
    lens: &MacroLens<'_>,
    guard: &mut Vec<String>,
    uses: &HashMap<String, String>,
    fields: &FieldIndex,
    returns: &ReturnIndex,
    m: &mut Marks,
) {
    if let Some(l) = ctor_leaf_of_expr(e, uses, fields, returns) {
        m.leaves.insert(l);
        // SOUNDNESS R172 — the ESCAPING construction site, address-keyed. `escaping_ctor_leaves`
        // suppresses a leaf only when every site of it in the body is in this set.
        m.sites.insert((e as *const syn::Expr as usize, 0));
    }
    if let syn::Expr::Path(p) = e {
        if p.qself.is_none() {
            if let Some(id) = p.path.get_ident() {
                m.names.insert(id.to_string());
            }
        }
    }
    // A FIELD/INDEX of a name also hands that name's storage outward (`fn into_inner(self) -> T
    // { self.0 }` must not charge `self`'s Drop — the value moves to the caller).
    if let syn::Expr::Field(f) = e {
        if let syn::Expr::Path(p) = &*f.base {
            if let Some(id) = p.path.get_ident() {
                m.names.insert(id.to_string());
            }
        }
    }
    // `vec![Guard::new()]` / `Some(g)` written through a macro, in tail position. syn does not parse a
    // macro body, so without this the idiomatic collection literal reads as NOT escaping and every
    // `fn make() -> Vec<Guard> { vec![Guard::new()] }` fabricates the guard's Drop onto the factory.
    if let syn::Expr::Macro(mac) = e {
        // ONE READING, SHARED WITH THE SITE WALK. Before this, the two walks read a macro differently:
        // the site walk consulted the `macro_rules!` TEMPLATE index and re-read unparsable tokens as
        // statements (R203/R206/R207), and this one did neither — it parsed the invocation tokens as an
        // expression list and stopped. A construction the site walk can SEE and this walk cannot is a
        // site that can never be marked escaping, i.e. a FABRICATION; one this walk can see and the
        // site walk cannot is a leaf certified pure, i.e. a SILENT UNDER-REPORT. Both were live at
        // 7dfe710 and both are measured in the commit that adds this.
        let reading = macro_reading(mac, lens, &|n| guard.iter().any(|x| x == n));
        let ords = macro_reading_ordinals(&reading);
        let base = mac as *const syn::ExprMacro as usize;
        let mut sub = Marks::default();
        // R172 — the parsed forms are OWNED TEMPORARIES, so an address recorded under them dies with
        // this call and a later allocation could reuse one. Marked into a scratch `Marks`, then each
        // site RENUMBERED onto `macro_reading_ordinals`, the numbering the body walk also uses —
        // `vec![from_handle(q)]` beside a local `H::new(p)` has to distinguish the two, or the
        // published charge on that body stays lost (measured: `mixed_macro`).
        for one in &reading.token_exprs {
            mark_escape(one, lens, guard, uses, fields, returns, &mut sub);
        }
        // Region 0's NAMES are the caller's own tokens (`vec![g]` really does carry the local `g`
        // out) and are extended exactly as before this change.
        m.names.extend(std::mem::take(&mut sub.names));
        let region0_leaves = sub.leaves.clone();
        // Region 1 and 2 carry the macro's VALUE in exactly one position each — a block's TAIL. A
        // statement inside them runs and discards its value, so its constructions do NOT escape
        // through the invocation, which is the whole reason a template that pushes into a collection
        // (`{ let x = H::new($p); $o.push(x); }`) must stay charged while `mk_h!` (`{ H::new($p) }`)
        // must not. Marking only the tail is also the CHARGING direction: an escape mark suppresses.
        //
        // THEIR NAMES ARE DISCARDED, deliberately. `strip_dollars` renders `$o` as `o`, a name that
        // means nothing in the caller's body; extending `m.names` with it would let an unrelated local
        // of that name read as escaping — a silence, from a walk whose whole job here is to stop one.
        if let Some(b) = &reading.token_stmts {
            if let Some(t) = tail_expr_of(b) {
                mark_escape(t, lens, guard, uses, fields, returns, &mut sub);
            }
        }
        if !reading.template_arms.is_empty() {
            if let Some(name) = resolve_local_macro(mac, lens) {
                guard.push(name);
                for arm in &reading.template_arms {
                    if let Some(t) = tail_expr_of(arm) {
                        mark_escape(t, lens, guard, uses, fields, returns, &mut sub);
                    }
                }
                guard.pop();
            }
        }
        // §E1 HIT COUNTER, R229's SUPPRESSING half: a leaf that escapes through the macro's VALUE and
        // that only the template / statement-token reading can see. Before the shared reading these
        // never reached `Marks::leaves`, so the leaf was never in the escaping set and the collector's
        // own R48 expansion charged a construction that is handed to the caller.
        if std::env::var("CANDOR_ALIAS_DEBUG").is_ok() {
            for l in sub.leaves.difference(&region0_leaves) {
                eprintln!("R229ESCAPE {l}");
            }
        }
        m.leaves.extend(sub.leaves);
        // A site reached through a NESTED macro carries that macro's own temporary base, which dies
        // with this call. Compose it into THIS reading's space with `ord_nested`, exactly as the site
        // walk does — before this, such an entry was DROPPED, so a construction inside a nested macro
        // could never be marked escaping and the site walk had to leave it unrecorded to avoid
        // charging every one of them.
        let nested_base: HashMap<usize, usize> = nested_macro_nodes(&reading)
            .iter()
            .enumerate()
            .filter_map(|(j, n)| match n {
                NestedMacro::Expr(m) => Some((*m as *const syn::ExprMacro as usize, j)),
                NestedMacro::Stmt(_) => None,
            })
            .collect();
        // R189 — this renumbering reads `sub.sites` ONLY, and that is complete because `mark_escape`
        // never files a `via` site: the two kinds are separated by `escape_from_root`'s binding
        // fixpoint (`Marks::absorb_via`), which is not on this path. Asserted rather than commented,
        // because a `via` site silently dropped here would be a site that can never be marked
        // escaping — a FABRICATION, the direction this file is not allowed to be wrong in.
        debug_assert!(sub.via.is_empty(), "mark_escape must not produce `via` sites");
        for (addr, ord) in &sub.sites {
            // A site written DIRECTLY in this parse carries `ord == 0` and is renumbered onto this
            // reading's ordinals.
            if *ord == 0 {
                if let Some(o) = ords.get(addr) {
                    m.sites.insert((base, *o));
                }
            } else if let Some(j) = nested_base.get(addr) {
                m.sites.insert((base, ord_nested(*j, *ord)));
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
            let mut then_m = Marks::default();
            if let Some(t) = tail_expr_of(&iff.then_branch) {
                mark_escape(t, lens, guard, uses, fields, returns, &mut then_m);
            }
            // No `else` means the implicit value is `()`, which can carry no NAME — an empty set, which
            // (correctly) vetoes any name the `then` arm alone found. A `()`-typed branch cannot carry a
            // real leaf either (its tail would have to be unit-typed), so this never veto-by-omission a
            // leaf in practice; leaves are unioned regardless, per this fn's doc comment above.
            let else_m = match &iff.else_branch {
                Some((_, eb)) => {
                    let mut n = Marks::default();
                    mark_escape(eb, lens, guard, uses, fields, returns, &mut n);
                    n
                }
                None => Marks::default(),
            };
            m.names.extend(then_m.names.intersection(&else_m.names).cloned());
            // Leaves AND their sites ride the same union rule (see the comment above): a construction
            // found directly inside one arm is a self-contained fact about that arm.
            m.leaves.extend(then_m.leaves.union(&else_m.leaves).cloned());
            m.sites.extend(then_m.sites.union(&else_m.sites).cloned());
            // R189 — `mark_escape` itself never files a `via` site (only `escape_from_root`'s
            // binding fixpoint does, through `absorb_via`), so these two sets are always empty here.
            // Carried anyway rather than dropped: a dropped set is a silent licence if that ever
            // changes, and the union is the same rule the `sites` line above uses.
            m.via.extend(then_m.via.union(&else_m.via).cloned());
            return;
        }
        syn::Expr::Match(mt) => {
            let mut arms = mt.arms.iter().map(|a| {
                let mut n = Marks::default();
                mark_escape(&a.body, lens, guard, uses, fields, returns, &mut n);
                n
            });
            if let Some(first) = arms.next() {
                let folded = arms.fold(first, |a, n| Marks {
                    names: a.names.intersection(&n.names).cloned().collect(),
                    leaves: a.leaves.union(&n.leaves).cloned().collect(),
                    sites: a.sites.union(&n.sites).cloned().collect(),
                    // R189 — always empty here; see the `If` arm's note on why it is carried.
                    via: a.via.union(&n.via).cloned().collect(),
                });
                m.names.extend(folded.names);
                m.leaves.extend(folded.leaves);
                m.sites.extend(folded.sites);
                m.via.extend(folded.via);
            }
            return;
        }
        _ => {}
    }
    for_each_value_child(e, &mut |c| mark_escape(c, lens, guard, uses, fields, returns, m));
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
        // SOUNDNESS R198/R209(b) — a closure's body is a value position OF THE CLOSURE: when the
        // closure value itself escapes this frame, everything its body constructs is built in some
        // later frame and is not this scope's to drop. Reached only from `mark_escape`, which is this
        // function's sole caller, and in practice only through `escape_from_root`'s `lets` fixpoint —
        // i.e. exactly when the name holding the closure was found to escape. Without this the narrowed
        // `Expr::Closure` arm in the collector would under-suppress a closure that IS handed away.
        syn::Expr::Closure(c) => f(&c.body),
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

/// SOUNDNESS R194 — THE VALUE SPINE of a `?`'s operand: every position from which the operand's own
/// RESULT can come. A construction here does not exist when that `?` takes its ERROR exit — the operand
/// returned `Err`/`None`, so it never produced the value — and charging its `Drop` fabricates an effect.
/// A construction ANYWHERE ELSE in the operand has already run by then and is live in this frame, which
/// is what `TryExit::interior` records.
///
/// SAY WHICH DIRECTION IT FAILS IN: this set is the EXEMPTION, so it is enumerated explicitly and every
/// shape not listed stays charged. Three exclusions are the load-bearing ones, each pinned by an
/// executed fixture whose `Drop` really runs in the caller's frame:
///   · `Reference` — `foo(&H::try_new(n)?)?` borrows a TEMPORARY that lives to the end of the
///     statement, so it dies on this `?`'s error exit (`attack_ref_arg`, 1 drop). Its by-value twin
///     `foo(H::try_new(n)?)?` is exempt through the `Call` args below, because a by-value argument is
///     MOVED into the callee and dies there (`spine_call_arg_byval`, 0 drops here, 1 in the callee —
///     which the caller inherits through the call edge, not through a local charge). Those two differ
///     by one `&` and by nothing else, which is why the descent has to stop at it.
///   · a `MethodCall`'s RECEIVER — `H::try_new(n)?.check()?` leaves the receiver a live temporary when
///     the second `?` fires (`attack_recv_chain`, 1 drop). `for_each_value_child` already refuses a
///     receiver for the neighbouring reason, so the two halves agree.
///   · a `Closure`/`Async` BODY and a block's non-tail STATEMENTS — `run_cb(|| { out.push(H::new());
///     .. })?` and `{ out.push(H::new()); gen(n) }?` store into a binding this frame owns
///     (`call_closure_operand_q`, `block_operand_q`, `attack_async_inline`, 1 drop each). A closure's
///     TAIL is a different matter and needs no exemption at all: it is already an unconditional escape
///     route, re-united after the positional filter, so no `?` can strip it (async-std's
///     `spawn_blocking(|| File::create(&p)).await?`).
/// The listed shapes are exactly the ones the audited corpus removals rely on: `Ok(Repr::new(s)?)`
/// (compact_str), `Ok(Self { inner: RsaPrivateKey::new(..)? })` (rsa, rusqlite), `Ok((Async::new(
/// stream)?, addr))` (async-io), `let raw = RawBatchCursor::generic_new(..)?` (mongodb, rdkafka).
fn value_spine_addrs(e: &syn::Expr, out: &mut std::collections::HashSet<usize>) {
    if !out.insert(e as *const syn::Expr as usize) {
        return; // already seen — a shared address cannot recur, but this keeps the walk total
    }
    match e {
        syn::Expr::Paren(p) => value_spine_addrs(&p.expr, out),
        syn::Expr::Group(g) => value_spine_addrs(&g.expr, out),
        // `(x?)?` / `X::new(..).await?` — the inner value is the outer operand's value.
        syn::Expr::Try(t) => value_spine_addrs(&t.expr, out),
        syn::Expr::Await(a) => value_spine_addrs(&a.base, out),
        // A block's value is its TAIL; its statements are not on the spine (see above).
        syn::Expr::Block(b) => tail_expr_of(&b.block).into_iter().for_each(|t| value_spine_addrs(t, out)),
        syn::Expr::Unsafe(u) => tail_expr_of(&u.block).into_iter().for_each(|t| value_spine_addrs(t, out)),
        // A FORK: exactly one arm produced the operand's value, and whichever it was, an `Err` from it
        // constructed nothing. Every arm is therefore exempt, unlike `for_each_value_child`, which
        // refuses forks because it is asking the opposite question ("does this escape on EVERY path").
        syn::Expr::If(i) => {
            tail_expr_of(&i.then_branch).into_iter().for_each(|t| value_spine_addrs(t, out));
            if let Some((_, e)) = &i.else_branch {
                value_spine_addrs(e, out);
            }
        }
        syn::Expr::Match(m) => m.arms.iter().for_each(|a| value_spine_addrs(&a.body, out)),
        // A by-value ARGUMENT is moved into the callee and dies in ITS frame. The callee PATH is not a
        // construction site at all (`skip_sites`), so it is not listed.
        syn::Expr::Call(c) => c.args.iter().for_each(|a| value_spine_addrs(a, out)),
        syn::Expr::MethodCall(m) => m.args.iter().for_each(|a| value_spine_addrs(a, out)),
        // Pure value composition: the parts move into the whole, which is the operand's value.
        syn::Expr::Tuple(t) => t.elems.iter().for_each(|x| value_spine_addrs(x, out)),
        syn::Expr::Array(a) => a.elems.iter().for_each(|x| value_spine_addrs(x, out)),
        syn::Expr::Struct(st) => st.fields.iter().for_each(|f| value_spine_addrs(&f.expr, out)),
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
/// result is vanishingly rare and forcing is the safe direction.
///
/// `next`/`next_back` are also absent, and SOUNDNESS R536 TESTED THAT SENTENCE RATHER THAN TRUSTING
/// IT. On its own subject it HOLDS: with a local `struct It` carrying `impl Iterator for It` whose
/// `next` spawns, `it.next()`, `it.next_back()` and `it.take(2).next()` all charge `['Exec']` beside
/// `it.count()` as the control, so an explicit `.next()` does resolve as an ordinary method call on
/// the receiver type and needs no forcing edge.
///
/// **What it does not say, and was read as saying, is anything about ELEMENT TYPING.** `next` is this
/// function's subject only as a driver of a custom `Iterator::next`; the separate question of what
/// `v.into_iter().next()` BINDS was answered by nobody, and that spelling read silent-pure over a
/// `Vec<Box<dyn Sink>>` until R536 put `next`/`next_back`/`nth` on the two element lists in
/// `is_element_preserving_adapter` / `is_element_yielding_accessor`. A true sentence beside a list of
/// names reads as a ruling on those names; this one is scoped now so the next reader does not have to
/// re-measure it to find out which question it settles.
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

/// One parsed `{…}` hole of a format string: which arg it draws, and WHICH `std::fmt` trait it requests.
///
/// SOUNDNESS R388 — this carried a `debug: bool`, i.e. TWO of the NINE std format traits. A type
/// implementing only `LowerHex` and formatted `{:x}` was checked against `Display`, found not to
/// implement it, and — per the resolve-or-skip discipline, which never hedges to `Unknown` here — was
/// DROPPED SILENTLY. Measured: a `HexOnly` whose `fmt` writes a file reads ABSENT through `{:x}`, while
/// the byte-identical `Display` control through `{}` charges `['Fs']`.
pub(crate) struct FmtHole {
    pub(crate) arg: FmtArg,
    /// The `std::fmt` trait this hole's type char names — `Display` when the spec names none.
    pub(crate) trait_leaf: &'static str,
}

/// The `std::fmt` trait a format spec's TYPE names. The whole family, written out rather than the two
/// spellings that were implemented (R346).
///
/// `?` is tested FIRST and wins: `{:x?}` and `{:X?}` are `Debug` with hex-formatted integers, NOT
/// `LowerHex`/`UpperHex` — reading the trailing type char alone would send them to the wrong trait, which
/// is the same class of error as checking only two traits, one layer down.
pub(crate) fn fmt_trait_of_spec(spec: &str) -> &'static str {
    let t = spec.trim_end();
    if t.ends_with('?') {
        return "Debug";
    }
    if std::env::var("CANDOR_FMT_DEBUG").is_ok() {
        if let Some(c) = t.chars().last() {
            if matches!(c, 'x' | 'X' | 'o' | 'b' | 'e' | 'E' | 'p') {
                eprintln!("R388HIT {t}");
            }
        }
    }
    match t.chars().last() {
        Some('x') => "LowerHex",
        Some('X') => "UpperHex",
        Some('o') => "Octal",
        Some('b') => "Binary",
        Some('e') => "LowerExp",
        Some('E') => "UpperExp",
        Some('p') => "Pointer",
        _ => "Display",
    }
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
                // R388 — the spec's TYPE names one of nine traits, not one of two. See
                // `fmt_trait_of_spec`: `?` wins over a preceding `x`/`X` because `{:x?}` is Debug.
                let trait_leaf = fmt_trait_of_spec(spec);
                let arg = if name_part.is_empty() {
                    FmtArg::Implicit
                } else if let Ok(idx) = name_part.parse::<usize>() {
                    FmtArg::Index(idx)
                } else {
                    FmtArg::Named(name_part.to_string())
                };
                holes.push(FmtHole { arg, trait_leaf });
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

/// SOUNDNESS R828 — the trait LEAF an `impl <path> for X` block implements, with a RENAMED import
/// resolved. Every implementor index in this engine is keyed by trait leaf, and they all took the leaf
/// AS WRITTEN — so `use super::T10 as Renamed; impl Renamed for M10` filed `M10` under `Renamed`, a
/// trait that does not exist, and `T10`'s CHA and its published `interfaceUnion` row never saw the one
/// effectful implementor (`deny Fs` exit 0 while the program writes the file).
///
/// A one-segment path is expanded through the scope's `use` map and the LAST segment of what it names
/// is the trait's real leaf. Anything that does not expand to a plain identifier — no binding, a
/// sentinel, an alternation — keeps the written leaf, which is exactly the answer before this change.
/// A multi-segment path already ends in the trait's own name (a renamed MODULE does not rename the
/// trait), so it is untouched.
pub(crate) fn impl_trait_leaf(tr: &syn::Path, uses: &HashMap<String, String>) -> Option<String> {
    let written = tr.segments.last()?.ident.to_string();
    if tr.segments.len() != 1 || tr.leading_colon.is_some() {
        return Some(written);
    }
    let full = expand(&written, uses);
    let last = full.rsplit("::").next().unwrap_or(&full);
    let is_ident = !last.is_empty()
        && last.chars().next().is_some_and(|c| c.is_alphabetic() || c == '_')
        && last.chars().all(|c| c.is_alphanumeric() || c == '_');
    if is_ident && last != written && std::env::var_os("CANDOR_VEINC_INSTR").is_some() {
        eprintln!("VEINC_RENAMED\t{written}\t{last}"); // §E1 REACH PROBE
    }
    Some(if is_ident { last.to_string() } else { written })
}

/// SOUNDNESS R828 — IMPLEMENTORS THE ITEM-LEVEL INDEX CANNOT NAME. A non-nominal implementor of a LOCAL
/// trait is RESOLVED (its method is a unit; recorded in `nonnominal`); every other one is recorded as the
/// R529 hedge records a block-nested one: into the SAME `nested_local` / `nested_foreign` sets, so every
/// consumer R529 already wired (the local dispatch hedge, the chained join, both union publications)
/// reads them without a second spelling of the rule.
///
/// Three positions, each one a real implementor that `trait_impls` never holds, so a `&dyn T` whose
/// other implementors are pure resolved to them and the row — and since R609 the PUBLISHED
/// `interfaceUnion` row — claimed purity over a body that writes a file:
///   * a NON-NOMINAL self type (`impl T5 for (u8, u8)`, `for &X`, `for [u8]`, `for fn()`):
///     `impl_type_name` returns `None`, so `collect_decls` records no CHA edge at all;
///   * an impl generated by a LOCAL `macro_rules!` (`macro_rules! mk { ($t:ident) => { impl T1 for $t
///     {..} } }`): the engine does not expand item macros, so the impl is never an item. The trait is
///     read off the macro's own tokens (a `$crate::` prefix is the crate itself); a trait spelled by a
///     metavariable (`impl $tr for $t`) cannot be named and is NOT recorded — the stated residual;
///   * `#[derive(Tr)]` naming a trait: the derive's expansion is the implementor. Recorded as a
///     whole-trait wildcard (`Tr::*`) because the derive's members are not in the source.
///
/// A member written as a metavariable (`fn $name`) is also recorded as the wildcard. The wildcard is
/// LOCAL-only: a foreign key needs the member, and a foreign trait with such an implementor stays the
/// residual it was.
///
/// DIRECTION: the hedged ones feed `Unknown` + `dispatch:<T>.<m>`, never an edge — the implementing body
/// has no unit to edge to — so a wrong entry costs an over-disclosure and never a purity claim. The
/// RESOLVED (non-nominal) ones edge to `{modpath}::{method}`; Pass B hedges instead when no unit carries
/// that qual, and a same-named free fn in the same module is unioned in (an over-charge, stated).
pub(crate) fn collect_opaque_trait_impls(
    items: &[syn::Item],
    include_tests: bool,
    modpath: &str,
    uses: &HashMap<String, String>,
    local: &mut std::collections::BTreeSet<String>,
    foreign: &mut std::collections::BTreeSet<String>,
    nonnominal: &mut std::collections::BTreeSet<String>,
) {
    fn record(
        tr: &syn::Path,
        members: &[String],
        uses: &HashMap<String, String>,
        local: &mut std::collections::BTreeSet<String>,
        foreign: &mut std::collections::BTreeSet<String>,
        why: &str,
    ) {
        if let Some((root, qual)) = foreign_trait_owner_qual(tr, uses) {
            for m in members.iter().filter(|m| m.as_str() != "*") {
                if std::env::var_os("CANDOR_VEINC_INSTR").is_some() {
                    eprintln!("VEINC_OPAQUE\t{why}\tforeign\t{root}#{qual}::{m}"); // §E1 REACH PROBE
                }
                foreign.insert(format!("{root}#{qual}::{m}"));
            }
            return;
        }
        let Some(leaf) = impl_trait_leaf(tr, uses) else { return };
        for m in members {
            if std::env::var_os("CANDOR_VEINC_INSTR").is_some() {
                eprintln!("VEINC_OPAQUE\t{why}\tlocal\t{leaf}::{m}"); // §E1 REACH PROBE
            }
            local.insert(format!("{leaf}::{m}"));
        }
    }
    for it in items {
        match it {
            syn::Item::Mod(m) if include_tests || !is_cfg_test(&m.attrs) => {
                if let Some((_, inner)) = &m.content {
                    let _cfg_off = crate::lang::CfgOffScope::enter_if(&m.attrs); // R977 — see `lang::CfgOffScope`
                    let mut sub = uses.clone();
                    let mut alts = HashMap::new();
                    collect_item_uses(inner, include_tests, &mut sub, &mut alts);
                    let sub_mod = crate::decls::qualify(modpath, &m.ident.to_string());
                    collect_opaque_trait_impls(inner, include_tests, &sub_mod, &sub, local, foreign, nonnominal);
                }
            }
            syn::Item::Impl(im) if include_tests || !is_cfg_test(&im.attrs) => {
                let Some((None, tr, _)) = &im.trait_ else { continue };
                if impl_type_name(&im.self_ty).is_some() {
                    continue; // nominal: `trait_impls` holds it
                }
                // A STRUCTURAL FORWARDER — a self type built only from the impl's OWN type parameters
                // (`impl<W: Write> Write for &mut W`, `impl<A: T, B: T> T for (A, B)`) — names no
                // implementor of its own: its body reaches the trait through those parameters, whose
                // implementors the CHA already holds. Not hedged. THE RESIDUAL: a forwarder whose body
                // ALSO performs an effect of its own is not disclosed by this rule.
                if self_type_is_params_only(&im.self_ty, &im.generics) {
                    if std::env::var_os("CANDOR_VEINC_INSTR").is_some() {
                        eprintln!("VEINC_FORWARDER");
                    }
                    continue;
                }
                let members: Vec<String> = im.items.iter().filter_map(|ii| match ii {
                    syn::ImplItem::Fn(f) if include_tests || !is_cfg_test(&f.attrs) => Some(f.sig.ident.to_string()),
                    _ => None,
                }).collect();
                // A LOCAL trait: RESOLVE, don't hedge. `scan_items` mints each method as a unit under the
                // module path (`{modpath}::{method}` — there is no type name to put in it), so the
                // dispatch can edge to that unit; `nonnominal` records which unit answers which member.
                // A FOREIGN trait keeps the hedge: its key needs an owner-qualified implementor the
                // obligation-2 index has no spelling for.
                if foreign_trait_owner_qual(tr, uses).is_none() {
                    if let Some(leaf) = impl_trait_leaf(tr, uses) {
                        // SOUNDNESS R1034 — the unit is minted under the impl's own key now (`&str` → `str::m`),
                        // never as the free fn `{modpath}::{m}` it used to merge with.
                        let key = impl_key_avoiding_free_fns(im, items);
                        for m in &members {
                            let q = match &key {
                                Some(k) => crate::decls::qualify(modpath, &format!("{k}::{m}")),
                                None => crate::decls::qualify(modpath, m),
                            };
                            if std::env::var_os("CANDOR_VEINC_INSTR").is_some() {
                                eprintln!("VEINC_NONNOMINAL\t{leaf}::{m}\t{q}"); // §E1 REACH PROBE
                            }
                            nonnominal.insert(format!("{leaf}::{m}\u{1f}{q}"));
                        }
                    }
                    continue;
                }
                record(tr, &members, uses, local, foreign, "nonnominal");
            }
            syn::Item::Macro(mm) if include_tests || !is_cfg_test(&mm.attrs) => {
                if !mm.mac.path.is_ident("macro_rules") {
                    continue;
                }
                let mut found: Vec<(syn::Path, Vec<String>)> = Vec::new();
                macro_token_impls(mm.mac.tokens.clone(), &mut found);
                for (tr, members) in found {
                    record(&tr, &members, uses, local, foreign, "macro");
                }
            }
            syn::Item::Struct(_) | syn::Item::Enum(_) | syn::Item::Union(_) => {
                let attrs = match it {
                    syn::Item::Struct(s) => &s.attrs,
                    syn::Item::Enum(e) => &e.attrs,
                    syn::Item::Union(u) => &u.attrs,
                    _ => continue,
                };
                if !include_tests && is_cfg_test(attrs) {
                    continue;
                }
                for a in attrs.iter().filter(|a| a.path().is_ident("derive")) {
                    let Ok(paths) = a.parse_args_with(
                        syn::punctuated::Punctuated::<syn::Path, syn::Token![,]>::parse_terminated)
                    else { continue };
                    for p in paths {
                        let leaf = p.segments.last().map(|s| s.ident.to_string()).unwrap_or_default();
                        // The toolchain's own derives implement toolchain traits: never a local leaf
                        // a consumer of this index asks about, so they are skipped rather than stored.
                        if matches!(leaf.as_str(), "Debug" | "Clone" | "Copy" | "PartialEq" | "Eq"
                            | "Hash" | "PartialOrd" | "Ord" | "Default")
                        {
                            continue;
                        }
                        record(&p, &["*".to_string()], uses, local, foreign, "derive");
                    }
                }
            }
            _ => {}
        }
    }
}

/// SOUNDNESS R1034 (residual) — THE RECEIVER A NON-PATH IMPL'S METHODS ANSWER TO. A non-path self type
/// that collides with no free fn keeps the module-level qual it always had (`{modpath}::{m}`; see
/// `impl_key_avoiding_free_fns` for why it is not re-keyed), and a qual with no type segment is a key no
/// receiver-typed call can spell: `s.encode()` on `s: &str` is typed `str::encode`, `f.tk()` on `f: &Foo`
/// (or `f: Foo`, by autoref) is typed `Foo::tk`, and neither matched anything, so the caller read PURE over
/// an impl body that wrote a file. This records, per such method, `"{referent}::{m}\u{1f}{unit qual}\u{1f}{0|1}"`
/// (the last field: 1 when several non-path impls of the module merge into that qual) —
/// the REFERENT is the self type with its references peeled (`&'a str` → `str`, `&mut Foo` → `Foo`), which is
/// the spelling the receiver typing produces. Pass B admits it as a by_tail2 FALLBACK only where no
/// definition already claims that tail, so it can supply an edge and never displace or split an existing one.
///
/// Not recorded: a referent that is not a plain path (a slice, array, tuple — the receiver typing does not
/// type those, so no typed call could use the entry; they stay the residual), and a referent that is one of
/// the impl's OWN type parameters (`impl<T: Tr> Tr for &T` forwards to `T`'s implementor; it names no
/// receiver of its own).
pub(crate) fn collect_nonpath_receivers(
    items: &[syn::Item],
    include_tests: bool,
    modpath: &str,
    out: &mut std::collections::BTreeSet<String>,
) {
    for it in items {
        match it {
            syn::Item::Mod(m) if include_tests || !is_cfg_test(&m.attrs) => {
                if let Some((_, inner)) = &m.content {
                    let _cfg_off = crate::lang::CfgOffScope::enter_if(&m.attrs); // R977
                    let sub_mod = crate::decls::qualify(modpath, &m.ident.to_string());
                    collect_nonpath_receivers(inner, include_tests, &sub_mod, out);
                }
            }
            syn::Item::Impl(im) if include_tests || !is_cfg_test(&im.attrs) => {
                if let Some(k) = impl_key_avoiding_free_fns(im, items) {
                    // SOUNDNESS R1056 (residual) — a COLLIDING non-path impl is keyed under its own type
                    // (`str::m`, `[u8]::m`), a tail the typed call reaches with no alias. It is recorded with an
                    // EMPTY unit qual (which Pass B's alias builder skips) only so `NONPATH_RECV_TAILS` knows the
                    // tail exists: without it a `Vec`/`String`/slice receiver never formed the call at all.
                    if impl_type_name(&im.self_ty).is_none()
                        && (k == "str" || k.starts_with('[') || k.starts_with('('))
                        && !im.generics.type_params().any(|p| {
                            let id = p.ident.to_string();
                            k.split(|c: char| !(c.is_alphanumeric() || c == '_')).any(|w| w == id)
                        })
                    {
                        for ii in &im.items {
                            if let syn::ImplItem::Fn(f) = ii {
                                if include_tests || !is_cfg_test(&f.attrs) {
                                    out.insert(format!("{k}::{}\u{1f}\u{1f}0", f.sig.ident));
                                }
                            }
                        }
                    }
                    continue; // a path self type, or a colliding one already keyed under its own type
                }
                // How many non-path impl blocks of THIS module file a method of this name under the one
                // module-level qual: more than one means the unit is a MERGE of distinct bodies (`impl Enc
                // for &str` beside `impl Enc for &[u8]`), and an edge to it would charge one impl's effect
                // to a receiver that runs the other. Such an entry is marked SHARED and Pass B discloses
                // instead of edging.
                let shared_count = |m: &str| {
                    items.iter().filter(|o| match o {
                        syn::Item::Impl(oi) if (include_tests || !is_cfg_test(&oi.attrs))
                            && impl_key_avoiding_free_fns(oi, items).is_none() =>
                        {
                            oi.items.iter().any(|x| matches!(x, syn::ImplItem::Fn(f) if f.sig.ident == m))
                        }
                        _ => false,
                    }).count()
                };
                let mut t: &syn::Type = &im.self_ty;
                loop {
                    t = match t {
                        syn::Type::Reference(r) => &r.elem,
                        syn::Type::Paren(p) => &p.elem,
                        syn::Type::Group(g) => &g.elem,
                        _ => break,
                    };
                }
                // SOUNDNESS R1056 — a SLICE / ARRAY / TUPLE referent is keyed the way `impl_unit_type_name`
                // spells it (`[u8]`, `[u8;_]`, `(u8,u8)`), which is also what the collector builds from a
                // receiver's element / per-position types. A key naming one of the impl's OWN parameters
                // (`impl<T: Tr> Tr for [T]`) is a forwarder over T's implementors, not a receiver, and is skipped.
                if matches!(t, syn::Type::Slice(_) | syn::Type::Array(_) | syn::Type::Tuple(_)) {
                    let Some(key) = impl_unit_type_name(t, &im.generics) else { continue };
                    let own = |k: &str| im.generics.type_params().any(|p| {
                        let id = p.ident.to_string();
                        k.split(|c: char| !(c.is_alphanumeric() || c == '_')).any(|w| w == id)
                    });
                    // SOUNDNESS R1056 (residual) — `impl<T> Tr for [T]` (or `[T; N]`) is not a forwarder: its
                    // body is the method EVERY slice (array) receiver runs, whatever its element, and skipping
                    // it left `s.gen_w()` on `&[String]` / `Vec<String>` ABSENT over a write (EXECUTED,
                    // scratchpad `rustagent-v044/fxvec2`). Filed under the wildcard element `[_]` / `[_;_]`,
                    // which the collector asks only where no exact key answers. Where the body forwards to
                    // `T`'s own implementor (`for x in self { x.m() }`), that call is the unit's bounded
                    // dispatch, the same over-approximation a generic caller already gets. A key that only
                    // CONTAINS a parameter (`[Vec<T>]`, `(T, u8)`) is still skipped: no receiver forms it.
                    let elem_is_own_param = |e: &syn::Type| matches!(e, syn::Type::Path(tp)
                        if tp.qself.is_none() && tp.path.get_ident()
                            .is_some_and(|id| im.generics.type_params().any(|p| &p.ident == id)));
                    let key = match t {
                        syn::Type::Slice(sl) if own(&key) && elem_is_own_param(&sl.elem) => "[_]".to_string(),
                        syn::Type::Array(ar) if own(&key) && elem_is_own_param(&ar.elem) => "[_;_]".to_string(),
                        _ if own(&key) => continue,
                        _ => key,
                    };
                    for ii in &im.items {
                        if let syn::ImplItem::Fn(f) = ii {
                            if !include_tests && is_cfg_test(&f.attrs) {
                                continue;
                            }
                            let m = f.sig.ident.to_string();
                            let shared = if shared_count(&m) > 1 { "1" } else { "0" };
                            out.insert(format!("{key}::{m}\u{1f}{}\u{1f}{shared}", crate::decls::qualify(modpath, &m)));
                        }
                    }
                    continue;
                }
                let syn::Type::Path(tp) = t else { continue };
                if tp.qself.is_some() {
                    continue;
                }
                let Some(seg) = tp.path.segments.last() else { continue };
                let referent = seg.ident.to_string();
                if im.generics.type_params().any(|p| p.ident == seg.ident) {
                    continue; // `impl<T> Tr for &T` — a forwarder, not a receiver
                }
                for ii in &im.items {
                    if let syn::ImplItem::Fn(f) = ii {
                        if !include_tests && is_cfg_test(&f.attrs) {
                            continue;
                        }
                        let m = f.sig.ident.to_string();
                        let shared = if shared_count(&m) > 1 { "1" } else { "0" };
                        out.insert(format!("{referent}::{m}\u{1f}{}\u{1f}{shared}", crate::decls::qualify(modpath, &m)));
                    }
                }
            }
            _ => {}
        }
    }
}

/// R828 — does a non-nominal self type mention ONLY the impl's own type parameters (at least one)?
fn self_type_is_params_only(ty: &syn::Type, generics: &syn::Generics) -> bool {
    let params: std::collections::HashSet<String> = generics.params.iter().filter_map(|g| match g {
        syn::GenericParam::Type(t) => Some(t.ident.to_string()),
        _ => None,
    }).collect();
    fn walk(t: &syn::Type, ps: &std::collections::HashSet<String>, seen: &mut bool) -> bool {
        match t {
            syn::Type::Reference(r) => walk(&r.elem, ps, seen),
            syn::Type::Ptr(p) => walk(&p.elem, ps, seen),
            syn::Type::Slice(s) => walk(&s.elem, ps, seen),
            syn::Type::Array(a) => walk(&a.elem, ps, seen),
            syn::Type::Paren(p) => walk(&p.elem, ps, seen),
            syn::Type::Group(g) => walk(&g.elem, ps, seen),
            syn::Type::Tuple(tu) => !tu.elems.is_empty() && tu.elems.iter().all(|e| walk(e, ps, seen)),
            syn::Type::Path(p) if p.qself.is_none() => match p.path.get_ident() {
                Some(id) if ps.contains(&id.to_string()) => { *seen = true; true }
                _ => false,
            },
            _ => false,
        }
    }
    let mut seen = false;
    !params.is_empty() && walk(ty, &params, &mut seen) && seen
}

/// R828 — every `impl <Trait> for <..> { .. }` written inside a `macro_rules!` body, as
/// `(trait path, member names)`. Token-level, because a macro body is not parseable as items: the arm
/// patterns and `$` metavariables are not Rust. A `$crate::` prefix is dropped (it IS this crate); a
/// trait path containing any other `$` is skipped, since nothing here can say which trait it names.
fn macro_token_impls(ts: proc_macro2::TokenStream, out: &mut Vec<(syn::Path, Vec<String>)>) {
    use proc_macro2::{Delimiter, TokenTree};
    let toks: Vec<TokenTree> = ts.into_iter().collect();
    let mut i = 0;
    while i < toks.len() {
        if let TokenTree::Group(g) = &toks[i] {
            macro_token_impls(g.stream(), out);
        }
        let is_impl = matches!(&toks[i], TokenTree::Ident(id) if id == "impl");
        if !is_impl {
            i += 1;
            continue;
        }
        let mut j = i + 1;
        // skip `<...>` generics directly after `impl`
        if matches!(toks.get(j), Some(TokenTree::Punct(p)) if p.as_char() == '<') {
            let mut depth = 0i32;
            while j < toks.len() {
                if let TokenTree::Punct(p) = &toks[j] {
                    match p.as_char() {
                        '<' => depth += 1,
                        '>' => {
                            depth -= 1;
                            if depth == 0 {
                                j += 1;
                                break;
                            }
                        }
                        _ => {}
                    }
                }
                j += 1;
            }
        }
        let start = j;
        while j < toks.len() && !matches!(&toks[j], TokenTree::Ident(id) if id == "for") {
            if matches!(&toks[j], TokenTree::Group(g) if g.delimiter() == Delimiter::Brace) {
                break; // an inherent `impl X { .. }` — no `for`
            }
            j += 1;
        }
        if j >= toks.len() || !matches!(&toks[j], TokenTree::Ident(id) if id == "for") {
            i += 1;
            continue;
        }
        let mut tr_toks: Vec<TokenTree> = toks[start..j].to_vec();
        // `$crate::Tr` names this crate's own trait.
        if matches!(tr_toks.first(), Some(TokenTree::Punct(p)) if p.as_char() == '$')
            && matches!(tr_toks.get(1), Some(TokenTree::Ident(id)) if id == "crate")
        {
            tr_toks.drain(0..2);
            while matches!(tr_toks.first(), Some(TokenTree::Punct(p)) if p.as_char() == ':') {
                tr_toks.remove(0);
            }
        }
        let has_meta = tr_toks.iter().any(|t| matches!(t, TokenTree::Punct(p) if p.as_char() == '$'));
        // find the body brace after the self type
        let mut k = j + 1;
        while k < toks.len() && !matches!(&toks[k], TokenTree::Group(g) if g.delimiter() == Delimiter::Brace) {
            k += 1;
        }
        if has_meta || k >= toks.len() {
            i = j + 1;
            continue;
        }
        let path_ts: proc_macro2::TokenStream = tr_toks.into_iter().collect();
        let Ok(path) = syn::parse2::<syn::Path>(path_ts) else {
            i = j + 1;
            continue;
        };
        let mut members = Vec::new();
        if let TokenTree::Group(body) = &toks[k] {
            let bt: Vec<TokenTree> = body.stream().into_iter().collect();
            for w in 0..bt.len() {
                if matches!(&bt[w], TokenTree::Ident(id) if id == "fn") {
                    match bt.get(w + 1) {
                        Some(TokenTree::Ident(n)) => members.push(n.to_string()),
                        Some(TokenTree::Punct(p)) if p.as_char() == '$' => members.push("*".to_string()),
                        _ => {}
                    }
                }
            }
        }
        if !members.is_empty() {
            out.push((path, members));
        }
        i = k + 1;
    }
}

/// SOUNDNESS R732 / R754 — the FUNCTION NAME a `link!` invocation declares as a foreign import, or
/// `None` when the invocation is not that shape.
///
/// `windows_targets::link!("kernel32.dll" "system" fn CreateFileW(..) -> HANDLE)` (and its successor
/// `windows_link::link!`) expands to an `extern "system" { pub fn CreateFileW(..); }` block: a foreign
/// declaration in every sense `collect_decls`' `Item::ForeignMod` arm records, written in a spelling no
/// arm read. Recognised on the macro's LEAF plus the shape of its arguments — a LIBRARY-NAME string
/// literal first, then a `fn NAME` — rather than on the leaf alone, because `link` is an ordinary word
/// and a crate's own `link!` that declares a bodied fn is not a foreign boundary. The shape test is what
/// keeps an unrelated macro out; the failure it leaves is a third-party macro with this exact argument
/// shape that does NOT declare an import, which over-discloses (`Unknown`) and never silences.
pub(crate) fn link_macro_fn_name(mac: &syn::Macro) -> Option<String> {
    if mac.path.segments.last()?.ident != "link" {
        return None;
    }
    let toks: Vec<proc_macro2::TokenTree> = mac.tokens.clone().into_iter().collect();
    match toks.first()? {
        proc_macro2::TokenTree::Literal(l) if l.to_string().starts_with('"') => {}
        _ => return None,
    }
    toks.windows(2).find_map(|w| match (&w[0], &w[1]) {
        (proc_macro2::TokenTree::Ident(k), proc_macro2::TokenTree::Ident(n)) if k == "fn" => Some(n.to_string()),
        _ => None,
    })
}

// ── SOUNDNESS R145 — `include!` TEXT IS READ WHERE IT CAN BE, AND DISCLOSED WHERE IT CANNOT ─────────────
//
// THE DEFECT. `collect_decls` skips an item-position macro invocation, so `include!("gen.rs")` contributed
// no unit and no edge: a call into an included function was an unresolved bare call and the caller read
// PURE. Measured, EXECUTED (the file is written): an out-of-tree `include!` of a real `fs::write`, with
// `deny Fs caller` exiting 0 against an inline-write control exiting 1 — on a unit the engine CERTIFIES
// (`analyzed` counts it, `functions[]` omits it, no `unanalyzed`, no `outOfScope`).
//
// TWO ANSWERS, chosen by whether the producer can read the text:
//   * RESOLUTION — the target is a file this scan can read and does not already walk (an out-of-tree path,
//     a non-`.rs` extension like ICU's `*.rs.data`, a `concat!(env!("CARGO_MANIFEST_DIR"), ..)` path).
//     Its items are SPLICED into the invoking module in place of the macro, which is what rustc does: they
//     get the module's path, its `use` map and the invocation's own `#[cfg]`s, so every existing pass
//     reads them with no special case. A target the walk ALREADY reads (an in-tree `.rs` file) is left
//     alone — it is analysed as its own file today, and splicing it as well would emit every unit twice.
//   * DISCLOSURE — the path is not one this scan can know (`env!("OUT_DIR")`, the build-script convention,
//     or any env var other than `CARGO_MANIFEST_DIR`), or the file is missing or does not parse. The
//     invocation stays, tagged `OPAQUE_INCLUDE_ATTR`, and `scan.rs` hedges the calls whose callee that
//     module's namespace could be supplying (see `opaque_include_scope`).

/// The outer attribute a spliced item carries: the DISPLAY path of the file it came from, so `fn_locs`
/// can report the included file rather than the includer (the span's line is the included file's).
pub(crate) const INCLUDED_ATTR: &str = "candor_included";
/// The outer attribute an `include!` this scan could NOT read is tagged with. See `opaque_include_modules`.
pub(crate) const OPAQUE_INCLUDE_ATTR: &str = "candor_opaque_include";

fn marker_attr(text: &str) -> Option<syn::Attribute> {
    use syn::parse::Parser;
    syn::Attribute::parse_outer.parse_str(text).ok()?.into_iter().next()
}

pub(crate) fn has_marker(attrs: &[syn::Attribute], name: &str) -> bool {
    attrs.iter().any(|a| a.path().is_ident(name))
}

/// The display path an `INCLUDED_ATTR` carries, if any.
pub(crate) fn included_label(attrs: &[syn::Attribute]) -> Option<String> {
    attrs.iter().find(|a| a.path().is_ident(INCLUDED_ATTR)).and_then(|a| match &a.meta {
        syn::Meta::NameValue(nv) => match &nv.value {
            syn::Expr::Lit(syn::ExprLit { lit: syn::Lit::Str(s), .. }) => Some(s.value()),
            _ => None,
        },
        _ => None,
    })
}

/// An item's outer attributes, read-only — `item_attrs_mut`'s twin, for SOUNDNESS R977's `CfgOffScope`.
pub(crate) fn item_attrs(it: &syn::Item) -> &[syn::Attribute] {
    match it {
        syn::Item::Const(x) => &x.attrs,
        syn::Item::Enum(x) => &x.attrs,
        syn::Item::ExternCrate(x) => &x.attrs,
        syn::Item::Fn(x) => &x.attrs,
        syn::Item::ForeignMod(x) => &x.attrs,
        syn::Item::Impl(x) => &x.attrs,
        syn::Item::Macro(x) => &x.attrs,
        syn::Item::Mod(x) => &x.attrs,
        syn::Item::Static(x) => &x.attrs,
        syn::Item::Struct(x) => &x.attrs,
        syn::Item::Trait(x) => &x.attrs,
        syn::Item::TraitAlias(x) => &x.attrs,
        syn::Item::Type(x) => &x.attrs,
        syn::Item::Union(x) => &x.attrs,
        syn::Item::Use(x) => &x.attrs,
        _ => &[],
    }
}

pub(crate) fn item_attrs_mut(it: &mut syn::Item) -> Option<&mut Vec<syn::Attribute>> {
    Some(match it {
        syn::Item::Const(x) => &mut x.attrs,
        syn::Item::Enum(x) => &mut x.attrs,
        syn::Item::ExternCrate(x) => &mut x.attrs,
        syn::Item::Fn(x) => &mut x.attrs,
        syn::Item::ForeignMod(x) => &mut x.attrs,
        syn::Item::Impl(x) => &mut x.attrs,
        syn::Item::Macro(x) => &mut x.attrs,
        syn::Item::Mod(x) => &mut x.attrs,
        syn::Item::Static(x) => &mut x.attrs,
        syn::Item::Struct(x) => &mut x.attrs,
        syn::Item::Trait(x) => &mut x.attrs,
        syn::Item::TraitAlias(x) => &mut x.attrs,
        syn::Item::Type(x) => &mut x.attrs,
        syn::Item::Union(x) => &mut x.attrs,
        syn::Item::Use(x) => &mut x.attrs,
        _ => return None,
    })
}

/// Is this macro `include!` (bare, or `std::`/`core::`-qualified)? `include_str!`/`include_bytes!` are
/// different macros (a VALUE, never code) and are not matched.
pub(crate) fn is_include_macro(mac: &syn::Macro) -> bool {
    let segs: Vec<String> = mac.path.segments.iter().map(|s| s.ident.to_string()).collect();
    match segs.as_slice() {
        [one] => one == "include",
        [root, leaf] => leaf == "include" && (root == "std" || root == "core"),
        _ => false,
    }
}

/// The file an `include!` names, when the scan can know it without building: a string literal (relative
/// to the INVOKING file's directory, rustc's rule), or `concat!(env!("CARGO_MANIFEST_DIR"), "lit", ..)`.
/// `None` for anything else — chiefly `env!("OUT_DIR")`, which exists only inside a build.
pub(crate) fn include_target(mac: &syn::Macro, base_dir: &Path, manifest_dir: &Path) -> Option<std::path::PathBuf> {
    fn leaf_is(m: &syn::Macro, name: &str) -> bool {
        m.path.segments.last().is_some_and(|s| s.ident == name)
    }
    let e: syn::Expr = mac.parse_body().ok()?;
    match e {
        syn::Expr::Lit(syn::ExprLit { lit: syn::Lit::Str(s), .. }) => Some(base_dir.join(s.value())),
        syn::Expr::Macro(m) if leaf_is(&m.mac, "concat") => {
            use syn::punctuated::Punctuated;
            let parts = m.mac.parse_body_with(Punctuated::<syn::Expr, syn::Token![,]>::parse_terminated).ok()?;
            let mut out = String::new();
            for (i, p) in parts.iter().enumerate() {
                match p {
                    syn::Expr::Lit(syn::ExprLit { lit: syn::Lit::Str(s), .. }) => out.push_str(&s.value()),
                    syn::Expr::Macro(em) if i == 0 && leaf_is(&em.mac, "env") => {
                        let v: syn::LitStr = em.mac.parse_body().ok()?;
                        if v.value() != "CARGO_MANIFEST_DIR" {
                            return None;
                        }
                        out.push_str(manifest_dir.to_str()?);
                    }
                    _ => return None,
                }
            }
            let p = std::path::PathBuf::from(out);
            Some(if p.is_absolute() { p } else { base_dir.join(p) })
        }
        _ => None,
    }
}

/// What the splice needs to know about the scan, and what it reports back.
pub(crate) struct IncludeEnv<'a> {
    /// The scan root — `INCLUDED_ATTR` labels are relative to it when the target lies inside.
    pub(crate) root: &'a Path,
    pub(crate) manifest_dir: &'a Path,
    /// Canonical paths of every file the walk admitted. A target in here is left unspliced.
    pub(crate) walked: &'a std::collections::HashSet<std::path::PathBuf>,
    pub(crate) include_tests: bool,
}

/// Splice every readable item-position `include!` in `items` (and in their inline modules), tag every
/// unreadable one, and append to `read` the path of every target consulted — so the file's cache entry is
/// keyed on those bytes too (`include_closure_hash`). `base_dir` is the directory of the file the items
/// came from. Recursion into an included file's own `include!`s is bounded (depth 8, no cycles).
pub(crate) fn splice_includes(
    items: &mut Vec<syn::Item>,
    base_dir: &Path,
    env: &IncludeEnv<'_>,
    stack: &mut Vec<std::path::PathBuf>,
    read: &mut Vec<String>,
) {
    let old = std::mem::take(items);
    for mut it in old {
        match &mut it {
            syn::Item::Mod(m) if m.content.is_some() => {
                if let Some((_, inner)) = &mut m.content {
                    splice_includes(inner, base_dir, env, stack, read);
                }
                items.push(it);
            }
            syn::Item::Macro(m)
                if m.ident.is_none()
                    && is_include_macro(&m.mac)
                    && (env.include_tests || !is_cfg_test(&m.attrs)) =>
            {
                let target = include_target(&m.mac, base_dir, env.manifest_dir);
                let canon = target.as_ref().and_then(|t| t.canonicalize().ok());
                if let Some(t) = &target {
                    read.push(canon.as_ref().unwrap_or(t).to_string_lossy().into_owned());
                }
                if canon.as_ref().is_some_and(|c| env.walked.contains(c)) {
                    items.push(it); // read as its own file already — the status quo, see above
                    continue;
                }
                let parsed = canon
                    .as_ref()
                    .filter(|c| stack.len() < 8 && !stack.contains(c))
                    .and_then(|c| std::fs::read_to_string(c).ok().map(|t| (c.clone(), t)))
                    .and_then(|(c, t)| parse_file_2015_tolerant(&t).map(|(f, _)| (c, f)));
                match parsed {
                    Some((c, f)) => {
                        if std::env::var_os("CANDOR_VEINE_INSTR").is_some() {
                            eprintln!("VEINE_SPLICE\t{}", c.display()); // §E1 REACH PROBE
                        }
                        let label = c
                            .strip_prefix(env.root.canonicalize().unwrap_or_else(|_| env.root.to_path_buf()))
                            .map(|r| r.to_string_lossy().into_owned())
                            .unwrap_or_else(|_| c.to_string_lossy().into_owned());
                        let mut inner = f.items;
                        stack.push(c.clone());
                        let dir = c.parent().map(Path::to_path_buf).unwrap_or_default();
                        splice_includes(&mut inner, &dir, env, stack, read);
                        stack.pop();
                        let tag = marker_attr(&format!("#[{INCLUDED_ATTR} = {label:?}]"));
                        let outer = m.attrs.clone();
                        for mut x in inner {
                            if let Some(a) = item_attrs_mut(&mut x) {
                                // The invocation's own attributes (its `#[cfg]`s) govern every item it
                                // supplies — `#[cfg(feature = "x")] include!(..)` gates them all.
                                let mut merged = outer.clone();
                                merged.extend(tag.clone());
                                merged.append(a);
                                *a = merged;
                            }
                            items.push(x);
                        }
                    }
                    None => {
                        if std::env::var_os("CANDOR_VEINE_INSTR").is_some() {
                            eprintln!("VEINE_OPAQUE\t{}", m.mac.tokens); // §E1 REACH PROBE
                        }
                        m.attrs.extend(marker_attr(&format!("#[{OPAQUE_INCLUDE_ATTR}]")));
                        items.push(it);
                    }
                }
            }
            _ => items.push(it),
        }
    }
}

/// The cache key of a file whose parse READ other files: its own content hash, extended with the current
/// bytes (or absence) and walk status of every target its last parse consulted. With no targets this is
/// the plain hash, so a file with no `include!` keys exactly as before. Any change to an included file —
/// or a missing one appearing — moves the key and forces a re-parse, which is what keeps a warm
/// `--incremental` run from replaying a splice of bytes that are no longer there.
pub(crate) fn include_closure_hash(
    own: &str,
    targets: &[String],
    walked: &std::collections::HashSet<std::path::PathBuf>,
) -> String {
    if targets.is_empty() {
        return own.to_string();
    }
    let mut s = own.to_string();
    for t in targets {
        let p = Path::new(t);
        let h = std::fs::read(p).map(|b| crate::cache::fnv1a(&b)).unwrap_or_else(|_| "-".into());
        let w = p.canonicalize().ok().is_some_and(|c| walked.contains(&c));
        s.push_str(&format!("\u{1}{t}\u{2}{h}\u{2}{w}"));
    }
    crate::cache::fnv1a(s.as_bytes())
}

/// SOUNDNESS R145 — the modules (by qual) holding an `include!` this scan could NOT read.
pub(crate) fn collect_opaque_include_modules(
    items: &[syn::Item],
    modpath: &str,
    include_tests: bool,
    out: &mut std::collections::BTreeSet<String>,
) {
    for it in items {
        match it {
            syn::Item::Macro(m) if m.ident.is_none() && has_marker(&m.attrs, OPAQUE_INCLUDE_ATTR) => {
                out.insert(modpath.to_string());
            }
            syn::Item::Mod(m) if include_tests || !is_cfg_test(&m.attrs) => {
                if let Some((_, inner)) = &m.content {
                    let sub = if modpath.is_empty() { m.ident.to_string() } else { format!("{modpath}::{}", m.ident) };
                    collect_opaque_include_modules(inner, &sub, include_tests, out);
                }
            }
            _ => {}
        }
    }
}

/// SOUNDNESS R145 — every CRATE-LOCAL glob import, as `"<importing module>\u{1}<imported module>"`. A
/// glob of a module whose namespace an unreadable `include!` supplies supplies the same unknown names to
/// the importer (libsqlite3-sys: `mod bindings { include!(OUT_DIR..) } pub use bindings::*;`). A relative
/// `use a::*` is recorded under both readings (`<here>::a` and the crate-root `a`), since which one is
/// meant needs name resolution this pass does not do; the extra reading can only widen the hedge.
pub(crate) fn collect_local_globs(
    items: &[syn::Item],
    modpath: &str,
    include_tests: bool,
    out: &mut std::collections::BTreeSet<String>,
) {
    fn walk(t: &syn::UseTree, prefix: &mut Vec<String>, modpath: &str, out: &mut std::collections::BTreeSet<String>) {
        match t {
            syn::UseTree::Path(p) => {
                prefix.push(p.ident.to_string());
                walk(&p.tree, prefix, modpath, out);
                prefix.pop();
            }
            syn::UseTree::Group(g) => {
                for x in &g.items {
                    walk(x, prefix, modpath, out);
                }
            }
            syn::UseTree::Glob(_) => {
                let join = |base: &[&str], rest: &[String]| {
                    base.iter().map(|s| s.to_string()).chain(rest.iter().cloned()).collect::<Vec<_>>().join("::")
                };
                let here: Vec<&str> = if modpath.is_empty() { vec![] } else { modpath.split("::").collect() };
                let mut targets = Vec::new();
                match prefix.first().map(String::as_str) {
                    Some("crate") => targets.push(join(&[], &prefix[1..])),
                    Some("self") => targets.push(join(&here, &prefix[1..])),
                    Some("super") => {
                        let mut base = here.clone();
                        let mut i = 0;
                        while prefix.get(i).map(String::as_str) == Some("super") {
                            base.pop();
                            i += 1;
                        }
                        targets.push(join(&base, &prefix[i..]));
                    }
                    Some(_) => {
                        targets.push(join(&here, prefix));
                        targets.push(join(&[], prefix));
                    }
                    None => {}
                }
                for tg in targets {
                    out.insert(format!("{modpath}\u{1}{tg}"));
                }
            }
            _ => {}
        }
    }
    for it in items {
        match it {
            syn::Item::Use(u) if include_tests || !is_cfg_test(&u.attrs) => {
                walk(&u.tree, &mut Vec::new(), modpath, out);
            }
            syn::Item::Mod(m) if include_tests || !is_cfg_test(&m.attrs) => {
                if let Some((_, inner)) = &m.content {
                    let sub = if modpath.is_empty() { m.ident.to_string() } else { format!("{modpath}::{}", m.ident) };
                    collect_local_globs(inner, &sub, include_tests, out);
                }
            }
            _ => {}
        }
    }
}

/// SOUNDNESS R145 — the closure of `opaque` under crate-local glob imports: every module whose namespace
/// may hold a name an unreadable `include!` supplied.
pub(crate) fn opaque_include_scope(
    opaque: &std::collections::HashSet<String>,
    globs: &std::collections::HashSet<String>,
) -> std::collections::HashSet<String> {
    let mut scope = opaque.clone();
    if scope.is_empty() {
        return scope;
    }
    let edges: Vec<(&str, &str)> = globs.iter().filter_map(|g| g.split_once('\u{1}')).collect();
    loop {
        let before = scope.len();
        for (from, to) in &edges {
            if scope.contains(*to) && !scope.contains(*from) {
                scope.insert(from.to_string());
            }
        }
        if scope.len() == before {
            return scope;
        }
    }
}

/// SOUNDNESS R981 — the members of the async abstractions (`futures_core::Stream`/`TryStream`,
/// `Future`, `futures_io`/`tokio::io` `AsyncRead`/`AsyncWrite`/`AsyncBufRead`) that RUN the abstraction:
/// the required `poll_*` members, and the extension-trait methods defined by calling them. A dispatch
/// through one of these on a bound the crate itself implements may land in the crate's own impl.
pub(crate) fn is_driving_async_member(leaf: &str) -> bool {
    matches!(
        leaf,
        "poll" | "poll_next" | "poll_read" | "poll_write" | "poll_flush" | "poll_close" | "poll_shutdown"
            | "poll_fill_buf" | "poll_write_vectored" | "poll_read_buf" | "try_poll" | "try_poll_next"
            | "poll_unpin" | "poll_next_unpin" | "try_poll_unpin" | "try_poll_next_unpin" | "now_or_never"
            | "next" | "try_next" | "collect" | "try_collect" | "for_each" | "try_for_each"
            | "for_each_concurrent" | "try_for_each_concurrent" | "fold" | "try_fold" | "count" | "forward"
            | "select_next_some" | "concat" | "into_future"
            | "read" | "read_exact" | "read_to_end" | "read_to_string" | "read_buf" | "read_line"
            | "read_until" | "fill_buf" | "write" | "write_all" | "write_vectored" | "write_buf"
            | "write_all_buf" | "flush"
    )
    // NOT `close`/`shutdown`: on a lock/channel the builder-chain walk carries the bound through
    // `self.inner.lock().close()` and named `dispatch:RawMutex.close` (futures-intrusive, measured) — a
    // false reason. `poll_close`/`poll_shutdown` above are the members those extension methods drive.
}

/// SOUNDNESS R988 — `pin_project_lite::pin_project! { struct S<D> { #[pin] inner: D, .. } }` declares its
/// struct INSIDE a macro, so the struct's fields were never indexed and every `this.inner` (through the
/// generated `project()`, typed as the struct by `type_of_method`) named nothing: tower's
/// `PendingRequestsDiscover::poll_next` and `Constant::poll_next` read pure over a `D: Discover` poll.
/// The body is ordinary item syntax (`#[pin]` and `#[project = ..]` are just attributes), so the STRUCT /
/// ENUM items it contains are spliced beside the invocation — declarations only: no fn is added, so the
/// `fn_locs` / `scan_items` lockstep is unaffected. The invocation stays, so nothing it may also generate
/// is claimed.
pub(crate) fn splice_pin_project(items: &mut Vec<syn::Item>) {
    let mut add: Vec<syn::Item> = Vec::new();
    for it in items.iter_mut() {
        // SOUNDNESS R960 — clang-sys declares all of libclang through its own `link!( pub fn clang_x(..);
        // .. )` macro, which expands to foreign imports; the declarations were invisible, so the crate's
        // report covered `clang_sys` while publishing no unit for `clang_createIndex`, and a chained
        // consumer (bindgen `BindgenContext::new`) read every libclang call PURE. A `link!` body that PARSES
        // as an `extern` block's items IS one — spliced as `extern "C" { .. }`, so R894 publishes each
        // `pub fn` as a `native:extern fn` unit. The windows `link!("dll" "abi" fn X(..))` shape does not
        // parse that way and keeps R754's route.
        if let syn::Item::Macro(m) = it {
            if m.ident.is_none() && m.mac.path.segments.last().is_some_and(|s| s.ident == "link") {
                use proc_macro2::{Delimiter, Group, Ident, Literal, Span, TokenStream, TokenTree};
                let mut ts = TokenStream::new();
                ts.extend([
                    TokenTree::Ident(Ident::new("extern", Span::call_site())),
                    TokenTree::Literal(Literal::string("C")),
                    TokenTree::Group(Group::new(Delimiter::Brace, m.mac.tokens.clone())),
                ]);
                if let Ok(fm) = syn::parse2::<syn::ItemForeignMod>(ts) {
                    if fm.items.iter().any(|f| matches!(f, syn::ForeignItem::Fn(_))) {
                        add.push(syn::Item::ForeignMod(fm));
                    }
                }
            }
        }
        match it {
            syn::Item::Macro(m) if m.ident.is_none() && m.mac.path.segments.last().is_some_and(|s| s.ident == "pin_project") => {
                if let Ok(f) = syn::parse2::<syn::File>(m.mac.tokens.clone()) {
                    for inner in f.items {
                        if matches!(inner, syn::Item::Struct(_) | syn::Item::Enum(_)) {
                            add.push(inner);
                        }
                    }
                }
            }
            syn::Item::Mod(md) => {
                if let Some((_, inner)) = &mut md.content {
                    splice_pin_project(inner);
                }
            }
            _ => {}
        }
    }
    items.extend(add);
}
