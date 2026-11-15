//! SOUNDNESS R1657 (bound projection) — the STRUCTURED bounds of a body's generics, and which bindings hold a value of one.
//!
//! `lang::bound_leaves` keeps only a bound's trait LEAF, so everything this family needs was thrown away where the bound was
//! read: a trait's written ARGUMENTS (`AsRef<Path>`), its associated-type BINDINGS (`Deref<Target = Conn>`), and the bounds a
//! `where` clause puts on a PROJECTION (`C::Target: Tr`). And a parameter typed by a generic is in `trait_vars` as those
//! leaves, so the value lost even its own generic NAME. `BoundEnv` keeps the bounds as written; `BVal` names the generic (or
//! projection) a binding holds. Both are FACTS: nothing here decides a call. The consumers are in `collector.rs`
//! (`bp_*`), and they ask these tables beside the existing routes rather than instead of them.
//!
//! WHY A SIDE TABLE AND NOT `vars` (Fable review §1.3). "A generic is not in `vars`" is load-bearing (R564, the RowIter guard):
//! dozens of readers route to bounded dispatch BECAUSE the receiver types to nothing. So the generic a binding holds lives in
//! `generic_vars`, a name-keyed RESOLUTION table scoped exactly as `trait_vars` is (`scoped_binding` clears and restores it,
//! `BoundNameState` carries it through the self-shadow window, `visit_local` clears it for every name a `let` binds, and a
//! nested item starts with none).
use std::collections::{BTreeSet, HashMap};

/// One written trait bound: `AsRef<Path>`, `Deref<Target = Conn>`, `dep::Tr<A = X>`.
#[derive(Clone, Debug)]
pub(crate) struct TraitRef {
    /// the last path segment (`Deref`)
    pub(crate) leaf: String,
    /// the written path, `::`-joined, before `use` expansion (`std::ops::Deref`, `dep::Tr`, `Deref`)
    pub(crate) path: String,
    /// the written TYPE arguments, in order (`AsRef<Path>` → `[Path]`)
    pub(crate) args: Vec<syn::Type>,
    /// the written associated-type bindings (`Deref<Target = Conn>` → `[("Target", Conn)]`)
    pub(crate) bindings: Vec<(String, syn::Type)>,
}

impl TraitRef {
    fn of(tb: &syn::TraitBound) -> Option<TraitRef> {
        if matches!(tb.modifier, syn::TraitBoundModifier::Maybe(_)) {
            return None; // `?Sized` is a relaxation, not a bound
        }
        let seg = tb.path.segments.last()?;
        let mut args = Vec::new();
        let mut bindings = Vec::new();
        if let syn::PathArguments::AngleBracketed(ab) = &seg.arguments {
            for a in &ab.args {
                match a {
                    syn::GenericArgument::Type(t) => args.push(t.clone()),
                    syn::GenericArgument::AssocType(at) => bindings.push((at.ident.to_string(), at.ty.clone())),
                    _ => {}
                }
            }
        }
        let path = tb.path.segments.iter().map(|s| s.ident.to_string()).collect::<Vec<_>>().join("::");
        Some(TraitRef { leaf: seg.ident.to_string(), path, args, bindings })
    }
}

fn refs_of(bounds: &syn::punctuated::Punctuated<syn::TypeParamBound, syn::Token![+]>) -> Vec<TraitRef> {
    bounds
        .iter()
        .filter_map(|b| match b {
            syn::TypeParamBound::Trait(tb) => TraitRef::of(tb),
            _ => None,
        })
        .collect()
}

/// What a binding holds, when it holds a value of a generic of this scope.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum BVal {
    /// a type parameter in scope (`C`), or `impl@<param>` for an argument-position `impl Trait`
    Generic(String),
    /// an associated-type projection of one (`C::A`, `<C as Has>::A`): the parameter, the written trait (if any), the assoc
    Proj { g: String, tr: Option<String>, assoc: String },
}

/// A body's generics (fn + enclosing impl, and argument-position `impl Trait`), with their bounds AS WRITTEN.
#[derive(Clone, Debug, Default)]
pub(crate) struct BoundEnv {
    /// every type parameter in scope, plus `impl@<param>` for each argument-position `impl Trait`
    pub(crate) params: BTreeSet<String>,
    /// parameter -> its bounds, inline and `where`, impl's and fn's
    pub(crate) of_param: HashMap<String, Vec<TraitRef>>,
    /// (parameter, associated type) -> the bounds a `where` clause puts on that projection (`where C::Target: Tr`,
    /// `where <C as Has>::A: Tr`)
    pub(crate) of_projection: HashMap<(String, String), Vec<TraitRef>>,
}

impl BoundEnv {
    /// The bounds of `g` (empty for a name that is not a parameter here).
    pub(crate) fn bounds(&self, g: &str) -> &[TraitRef] {
        self.of_param.get(g).map(Vec::as_slice).unwrap_or(&[])
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.params.is_empty()
    }

    /// What a WRITTEN type names, when it names a generic of this scope or a projection of one: `C`, `C::A`,
    /// `<C as Has>::A` (behind any `&`/`&mut`/parens). `None` for anything else.
    pub(crate) fn generic_of(&self, ty: &syn::Type) -> Option<BVal> {
        match ty {
            syn::Type::Reference(r) => self.generic_of(&r.elem),
            syn::Type::Paren(p) => self.generic_of(&p.elem),
            syn::Type::Group(g) => self.generic_of(&g.elem),
            syn::Type::Path(tp) => match &tp.qself {
                None => {
                    let segs: Vec<&syn::PathSegment> = tp.path.segments.iter().collect();
                    if tp.path.leading_colon.is_some() {
                        return None;
                    }
                    let head = segs.first()?.ident.to_string();
                    if !self.params.contains(&head) || !matches!(segs[0].arguments, syn::PathArguments::None) {
                        return None;
                    }
                    match segs.len() {
                        1 => Some(BVal::Generic(head)),
                        2 if matches!(segs[1].arguments, syn::PathArguments::None) => {
                            Some(BVal::Proj { g: head, tr: None, assoc: segs[1].ident.to_string() })
                        }
                        _ => None,
                    }
                }
                Some(q) => {
                    let syn::Type::Path(inner) = &*q.ty else { return None };
                    let g = inner.path.get_ident().filter(|_| inner.qself.is_none())?.to_string();
                    if !self.params.contains(&g) {
                        return None;
                    }
                    let segs: Vec<&syn::PathSegment> = tp.path.segments.iter().collect();
                    let assoc = segs.last()?.ident.to_string();
                    let tr = (q.position >= 1).then(|| segs.get(q.position - 1).map(|s| s.ident.to_string())).flatten();
                    (segs.len() == q.position + 1).then_some(BVal::Proj { g, tr, assoc })
                }
            },
            _ => None,
        }
    }

    /// Build the env of one body from its WRITTEN signature and the enclosing impl's generics, and the parameters that hold
    /// a value of one of those generics (`c: &mut C`, `a: <C as Has>::A`, `p: impl AsRef<Path>`).
    pub(crate) fn build(sig: &syn::Signature, impl_generics: Option<&syn::Generics>) -> (BoundEnv, HashMap<String, BVal>) {
        let mut env = BoundEnv::default();
        let gens: Vec<&syn::Generics> = impl_generics.into_iter().chain(std::iter::once(&sig.generics)).collect();
        for g in &gens {
            for tp in g.type_params() {
                env.params.insert(tp.ident.to_string());
            }
        }
        for g in &gens {
            for tp in g.type_params() {
                let r = refs_of(&tp.bounds);
                if !r.is_empty() {
                    env.of_param.entry(tp.ident.to_string()).or_default().extend(r);
                }
            }
            let Some(w) = &g.where_clause else { continue };
            for p in &w.predicates {
                let syn::WherePredicate::Type(pt) = p else { continue };
                let r = refs_of(&pt.bounds);
                if r.is_empty() {
                    continue;
                }
                match env.generic_of(&pt.bounded_ty) {
                    // only the bare written type (`C: ..`, `C::A: ..`), never a reference to one
                    Some(BVal::Generic(n)) if matches!(pt.bounded_ty, syn::Type::Path(_)) => {
                        env.of_param.entry(n).or_default().extend(r);
                    }
                    Some(BVal::Proj { g, assoc, .. }) if matches!(pt.bounded_ty, syn::Type::Path(_)) => {
                        env.of_projection.entry((g, assoc)).or_default().extend(r);
                    }
                    _ => {}
                }
            }
        }
        let mut held: HashMap<String, BVal> = HashMap::new();
        for a in &sig.inputs {
            let syn::FnArg::Typed(pt) = a else { continue };
            let syn::Pat::Ident(pi) = &*pt.pat else { continue };
            if pi.subpat.is_some() {
                continue;
            }
            let name = pi.ident.to_string();
            let mut t: &syn::Type = &pt.ty;
            loop {
                match t {
                    syn::Type::Reference(r) => t = &r.elem,
                    syn::Type::Paren(p) => t = &p.elem,
                    syn::Type::Group(g) => t = &g.elem,
                    _ => break,
                }
            }
            if let syn::Type::ImplTrait(it) = t {
                let r = refs_of(&it.bounds);
                if !r.is_empty() {
                    let g = format!("impl@{name}");
                    env.params.insert(g.clone());
                    env.of_param.insert(g.clone(), r);
                    held.insert(name, BVal::Generic(g));
                }
                continue;
            }
            if let Some(v) = env.generic_of(t) {
                held.insert(name, v);
            }
        }
        (env, held)
    }
}

/// The per-body state: the env, and the name-keyed `generic_vars` (see the module doc for its scoping).
#[derive(Clone, Debug, Default)]
pub(crate) struct BpState {
    pub(crate) env: BoundEnv,
    pub(crate) generic_vars: HashMap<String, BVal>,
}

impl BpState {
    pub(crate) fn new(sig: &syn::Signature, impl_generics: Option<&syn::Generics>) -> BpState {
        let (env, generic_vars) = BoundEnv::build(sig, impl_generics);
        let st = BpState { env, generic_vars };
        if candor_report::env::trace("CANDOR_R1657BP_ENV") && !st.env.is_empty() {
            eprintln!("R1657BPENV\t{}\t{}", sig.ident, st.describe()); // §E1 the fact source, per body
        }
        st
    }

    /// One line per body: every parameter with its bounds as written (`C: Deref<Target=Conn>+DerefMut`), every projection
    /// bound, and every binding that holds a generic. A REACH instrument for the fact source (`CANDOR_R1657BP_ENV`).
    pub(crate) fn describe(&self) -> String {
        use quote::ToTokens;
        let tr = |r: &TraitRef| {
            let mut a: Vec<String> = r.args.iter().map(|t| t.to_token_stream().to_string()).collect();
            a.extend(r.bindings.iter().map(|(n, t)| format!("{n}={}", t.to_token_stream())));
            if a.is_empty() { format!("{}[{}]", r.leaf, r.path) } else { format!("{}[{}]<{}>", r.leaf, r.path, a.join(",")) }
        };
        let mut out: Vec<String> = Vec::new();
        for p in &self.env.params {
            out.push(format!("{p}: {}", self.env.bounds(p).iter().map(tr).collect::<Vec<_>>().join("+")));
        }
        let mut pk: Vec<&(String, String)> = self.env.of_projection.keys().collect();
        pk.sort();
        for k in pk {
            out.push(format!("{}::{}: {}", k.0, k.1, self.env.of_projection[k].iter().map(tr).collect::<Vec<_>>().join("+")));
        }
        let mut gv: Vec<(&String, &BVal)> = self.generic_vars.iter().collect();
        gv.sort_by(|a, b| a.0.cmp(b.0));
        for (n, v) in gv {
            out.push(format!("{n}@{v:?}"));
        }
        out.join("; ")
    }
}
