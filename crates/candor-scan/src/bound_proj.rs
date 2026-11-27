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

/// SOUNDNESS R1940 — the bounds an associated-type CONSTRAINT puts on a projection of the bounded parameter: `T: Iterator<Item: Lt>`
/// bounds `T::Item` by `Lt`, exactly as `where T::Item: Lt` does. `(assoc, bounds)` per constraint in the written bound list.
fn constraints_of(bounds: &syn::punctuated::Punctuated<syn::TypeParamBound, syn::Token![+]>) -> Vec<(String, Vec<TraitRef>)> {
    let mut out = Vec::new();
    for b in bounds {
        let syn::TypeParamBound::Trait(tb) = b else { continue };
        let Some(seg) = tb.path.segments.last() else { continue };
        let syn::PathArguments::AngleBracketed(ab) = &seg.arguments else { continue };
        for a in &ab.args {
            if let syn::GenericArgument::Constraint(c) = a {
                let r = refs_of(&c.bounds);
                if !r.is_empty() {
                    out.push((c.ident.to_string(), r));
                }
            }
        }
    }
    out
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
                for (assoc, rs) in constraints_of(&tp.bounds) {
                    env.of_projection.entry((tp.ident.to_string(), assoc)).or_default().extend(rs);
                }
            }
            let Some(w) = &g.where_clause else { continue };
            for p in &w.predicates {
                let syn::WherePredicate::Type(pt) = p else { continue };
                let r = refs_of(&pt.bounds);
                if r.is_empty() {
                    continue;
                }
                if let Some(BVal::Generic(n)) = env.generic_of(&pt.bounded_ty).filter(|_| matches!(pt.bounded_ty, syn::Type::Path(_))) {
                    for (assoc, rs) in constraints_of(&pt.bounds) {
                        env.of_projection.entry((n.clone(), assoc)).or_default().extend(rs);
                    }
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

/// A value the probe reasons about: what a written type (a `Target`, a conversion argument) or a receiver names.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Val {
    /// a nominal type, as the typer names it (`type_path`)
    Nominal(String),
    /// a type parameter in scope (or `impl@<param>`)
    Generic(String),
    /// an associated-type projection of one
    Proj { g: String, tr: Option<String>, assoc: String },
    /// a trait object (`dyn Tr`): its trait leaves
    Dyn(Vec<String>),
    /// a type the typer has no name for (`[u8]`, a tuple, an array): an explicit residual, never "no answer" by accident
    Opaque,
}

impl From<BVal> for Val {
    fn from(b: BVal) -> Val {
        match b {
            BVal::Generic(g) => Val::Generic(g),
            BVal::Proj { g, tr, assoc } => Val::Proj { g, tr, assoc },
        }
    }
}

/// What the probe answers for one member call: the typed receivers (each a typed call), the LOCAL traits to dispatch on
/// (bounded CHA with the trait's key), and the hedges to disclose beside them — a UNION, never a pick (SPEC §4, the
/// arm-union paragraph: `{Fs, Unknown}` is conformant, `{Unknown}` alone is not).
#[derive(Clone, Debug, Default)]
pub(crate) struct Ans {
    pub(crate) typed: Vec<String>,
    pub(crate) traits: Vec<String>,
    pub(crate) hedges: Vec<String>,
    /// the closing facts, for the sidecar's `recv_fact` (`bound-deref`, `bound-conv`, `projection`; several when one answer
    /// went through more than one, `bound-conv+projection`)
    pub(crate) facts: BTreeSet<&'static str>,
    /// a bound of the generic is a DEPENDENCY trait whose member list is unknown: today's dispatch route must still run
    /// for it (its key is a real candidate), so the typed answer is pushed beside that route instead of replacing it
    pub(crate) keep_dispatch_route: bool,
    /// SOUNDNESS R1798 — a typed receiver was reached by the `Deref` STEP: the value is a PLACE of the Target type, probed by
    /// value, then `&`, then `&mut` (receiver form 1), not a `&X` value (form 2, what a conversion's `as_ref() -> &X` is)
    pub(crate) via_deref: bool,
}

impl Ans {
    fn merge(&mut self, o: Ans) {
        for t in o.typed {
            if !self.typed.contains(&t) {
                self.typed.push(t);
            }
        }
        for t in o.traits {
            if !self.traits.contains(&t) {
                self.traits.push(t);
            }
        }
        for h in o.hedges {
            if !self.hedges.contains(&h) {
                self.hedges.push(h);
            }
        }
        self.keep_dispatch_route |= o.keep_dispatch_route;
        self.via_deref |= o.via_deref;
        self.facts.extend(o.facts);
    }
    pub(crate) fn fact(&self) -> String {
        self.facts.iter().copied().collect::<Vec<_>>().join("+")
    }
    pub(crate) fn is_empty(&self) -> bool {
        self.typed.is_empty() && self.traits.is_empty() && self.hedges.is_empty()
    }
    /// The receiver form (`Call::recv_form`) of a call this answer types: 1 for a place reached by auto-deref, whose probe
    /// steps start BY VALUE (a `Copy` Target's `fn m(self)` trait member beats the inherent `&self` one, R1798); 2 for a
    /// conversion's `&X`, where the inherent `&self` method matches at the first step.
    pub(crate) fn recv_form(&self) -> u8 {
        if self.via_deref { 1 } else { 2 }
    }
}

/// Step 1 of Rust's probe at a generic node: which of its bounds (and, for a local trait, its supertraits) declare the member.
#[derive(Default, Debug)]
struct Step1 {
    /// a bound whose member list is COMPLETE declares it: the probe stops here
    hit: bool,
    /// the LOCAL traits that declare it (the dispatch the probe stopped at)
    local_hits: Vec<String>,
    /// a std trait declares it, with its I/O-driving hedge if it is one (R1286's rule)
    std_hits: Vec<String>,
    /// candidates whose member list is NOT known (a dependency bound, a local trait's dependency supertrait): disclosed beside
    /// whatever the later step answers
    incomplete: Vec<String>,
    /// a dependency bound trait was among them (see `Ans::keep_dispatch_route`)
    dep_bound: bool,
    /// local traits with a BLANKET impl that declare the member: applicability is undecided without a solver, so they are
    /// unioned with the later step's answer, with their key
    blanket: Vec<String>,
}

/// What the path-call projection arm found for `T::Item::f()` (see `bp_path_projection`).
#[derive(Default, Debug)]
pub(crate) struct PathProj {
    /// nominal types a written `Item = X` binding names: typed calls
    pub(crate) typed: Vec<String>,
    /// LOCAL bound traits that declare the member: bounded dispatch over their implementors
    pub(crate) traits: Vec<String>,
    /// DEPENDENCY bound traits (member list unknown): keyed when alone, else disclosed
    pub(crate) deps: Vec<TraitRef>,
    /// a STD bound of the projection declares the member too
    pub(crate) std_declares: bool,
    /// a candidate whose member list or type this route cannot see (dependency supertrait, a binding to a non-nominal type)
    pub(crate) unnamed: bool,
}

/// SOUNDNESS R1957–R1959 / R1952 — THE PROJECTION'S FACTS, asked once by both spellings (Fable review (d): merge the FACT half, keep
/// two renderings). `<g as via>::assoc`: the written BINDINGS on the bounds of `g` through which `assoc` is reached, the bounds
/// WRITTEN on the projection (design step 1(a): `where g::assoc: B`, `g: Tr<assoc: B>`), and the bounds the declaring LOCAL trait
/// puts on it (step 1(b): `type assoc: B;`). Facts only: which of them a spelling answers, and how, is its adapter's.
#[derive(Debug, Default)]
pub(crate) struct ProjFacts {
    /// what a written `assoc = X` binding names, as written (generics first), deduplicated
    pub(crate) bindings: Vec<Val>,
    /// step 1(a), in written order
    pub(crate) written: Vec<TraitRef>,
    /// step 1(b), leaves not already in `written`
    pub(crate) declared: Vec<TraitRef>,
}

/// Where a WRITTEN bound's trait comes from, decided by its RESOLVED ROOT through the module's `use` map — never by its leaf, so a
/// dependency trait named `Error`, `Read`, `Sub` or `Pointee` is a dependency's (SOUNDNESS R1952).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Origin {
    /// a crate-local trait (by leaf) whose written path does not root at a dependency
    Local,
    /// rooted at a dependency crate: the expanded path (`fxdep::Error`); `local` when a crate-local trait shares the leaf
    Dep { full: String, local: bool },
    /// a std trait leaf or a marker (`Send`, `Sized`, ..), with no dependency root
    Std,
    /// a bare leaf no `use` roots and no local or std trait declares
    Unrooted,
}

/// A member every type has through a std blanket impl (or through `&C`, which is `Clone`/`Borrow`/`Into`-able whatever `C`
/// is): Rust's probe finds it at the autoref step, BEFORE any `Deref` step, so the Target is never consulted.
fn universal_member(leaf: &str) -> bool {
    matches!(
        leaf,
        "clone" | "clone_from" | "clone_into" | "to_owned" | "borrow" | "borrow_mut" | "into" | "try_into" | "type_id" | "deref"
            | "deref_mut"
    )
}

/// Does the std trait at `path` (or a supertrait) declare `member`, required or provided?
fn std_declares(path: &str, member: &str) -> bool {
    let t = crate::std_bounds::table();
    let mut stack = vec![path.to_string()];
    let mut seen: BTreeSet<String> = BTreeSet::new();
    while let Some(p) = stack.pop() {
        if !seen.insert(p.clone()) {
            continue;
        }
        let Some(row) = t.trait_row(&p) else { continue };
        if row.required.contains(&member) || row.provided.contains(&member) || row.provided_deferred.contains(&member) {
            return true;
        }
        stack.extend(row.supers.iter().map(|s| s.to_string()));
    }
    false
}

/// The kill switch for the whole bound-projection probe.
pub(crate) fn bp_off() -> bool {
    candor_report::env::switch("CANDOR_R1657BP_OFF")
}

const MAX_DEREF: usize = 8; // Rust's own autoderef chain is unbounded but finite; past this the answer is "not known"

impl<'a> super::CallCollector<'a> {
    /// What a WRITTEN type names, with in-scope generics FIRST (Fable review §2.2, s1_shadow): `fn f<Loud: Q, C: Deref<Target
    /// = Loud>>` names the PARAMETER `Loud`, never a struct of that name — stage 1a's generic-shadowing rule, asked here before
    /// any module declaration.
    pub(crate) fn bp_written(&self, ty: &syn::Type) -> Val {
        if let Some(b) = self.bp.env.generic_of(ty) {
            return b.into();
        }
        match ty {
            syn::Type::Reference(r) => self.bp_written(&r.elem),
            syn::Type::Paren(p) => self.bp_written(&p.elem),
            syn::Type::Group(g) => self.bp_written(&g.elem),
            syn::Type::TraitObject(t) => Val::Dyn(crate::lang::bound_leaves(&t.bounds)),
            syn::Type::Path(_) => match crate::lang::type_path(ty, &self.uses) {
                Some(t) if !t.is_empty() => Val::Nominal(t),
                _ => Val::Opaque,
            },
            _ => Val::Opaque,
        }
    }

    /// Step 1 over a list of bounds.
    fn bp_step1(&self, bounds: &[TraitRef], leaf: &str) -> Step1 {
        let mut s = Step1::default();
        for tr in bounds {
            let l = tr.leaf.as_str();
            if self.local_traits.contains_key(l) {
                if self.trait_declares_method(l, leaf, 0) {
                    s.hit = true;
                    if !s.local_hits.iter().any(|x| x == l) {
                        s.local_hits.push(l.to_string());
                    }
                    continue;
                }
                // a local trait's list is complete only over LOCAL and STD supertraits (R1495's rule for `dyn`)
                let mut stack = vec![(l.to_string(), 0usize)];
                let mut seen: BTreeSet<String> = BTreeSet::new();
                while let Some((t, d)) = stack.pop() {
                    if !seen.insert(t.clone()) || d > 16 {
                        continue;
                    }
                    let Some(lt) = self.local_traits.get(&t) else { continue };
                    for sup in &lt.supertraits {
                        if self.local_traits.contains_key(sup) {
                            stack.push((sup.clone(), d + 1));
                        } else if crate::std_bounds::is_std_trait_leaf(sup) {
                            let t = crate::std_bounds::table();
                            if t.traits_by_leaf(sup).iter().any(|p| std_declares(p, leaf)) {
                                s.hit = true;
                                s.std_hits.push(sup.clone());
                            }
                        } else if !matches!(sup.as_str(), "Send" | "Sync" | "Unpin" | "Sized" | "Copy") {
                            let h = format!("dispatch:{sup}.{leaf}");
                            if !s.incomplete.contains(&h) {
                                s.incomplete.push(h);
                            }
                        }
                    }
                }
            } else if crate::std_bounds::is_std_trait_leaf(l) {
                let t = crate::std_bounds::table();
                let paths: Vec<&str> = match t.resolve_trait(&tr.path) {
                    Some(p) => vec![p],
                    None => t.traits_by_leaf(l).to_vec(),
                };
                if paths.iter().any(|p| std_declares(p, leaf)) {
                    s.hit = true;
                    s.std_hits.push(l.to_string());
                }
            } else if !matches!(l, "Send" | "Sync" | "Unpin" | "Sized" | "Copy") {
                s.dep_bound = true;
                let h = format!("dispatch:{l}.{leaf}");
                if !s.incomplete.contains(&h) {
                    s.incomplete.push(h);
                }
            }
        }
        for name in self.local_traits.keys() {
            if bounds.iter().any(|b| &b.leaf == name) {
                continue;
            }
            if self.returns.contains_key(&crate::model::blanket_impl_key(name))
                && self.trait_declares_method(name, leaf, 0)
                && self.bp_blanket_may_cover(name, bounds)
            {
                s.blanket.push(name.clone());
            }
        }
        s.blanket.sort();
        s
    }

    /// Can a BLANKET impl of local trait `name` cover a generic bounded by `bounds`? `false` only where it is DECIDED that it
    /// cannot: every blanket impl of `name` requires a std or local trait that the generic's bound set does not contain,
    /// transitively over supertraits, and that set is COMPLETE (no dependency trait among it or its supertraits, whose own
    /// supertraits are unknown here). Anything else is undecided, and an undecided candidate is a candidate. Leaf-level, so
    /// two different `AsRef<X>` read as one: that errs toward "may cover", the union direction.
    fn bp_blanket_may_cover(&self, name: &str, bounds: &[TraitRef]) -> bool {
        let Some(spec) = self.returns.get(&crate::model::blanket_bounds_key(name)) else { return true };
        let t = crate::std_bounds::table();
        let mut closure: BTreeSet<String> = BTreeSet::new();
        let mut complete = true;
        let mut stack: Vec<String> = bounds.iter().map(|b| b.leaf.clone()).collect();
        while let Some(l) = stack.pop() {
            if !closure.insert(l.clone()) {
                continue;
            }
            if let Some(lt) = self.local_traits.get(&l) {
                stack.extend(lt.supertraits.iter().cloned());
            } else if crate::std_bounds::is_std_trait_leaf(&l) {
                for p in t.traits_by_leaf(&l) {
                    if let Some(row) = t.trait_row(p) {
                        stack.extend(row.supers.iter().map(|s| crate::std_bounds::leaf(s).to_string()));
                    }
                }
            } else if !matches!(l.as_str(), "Send" | "Sync" | "Unpin" | "Sized" | "Copy") {
                complete = false;
            }
        }
        if !complete {
            return true;
        }
        // SOUNDNESS R1797 — …AND WHAT STD'S OWN BLANKET IMPLS PROVIDE from it. `C: Display` makes `C: ToString` hold through
        // `impl<T: Display> ToString for T`, so a local `impl<T: ToString> Ext for T` DOES cover `C`, and Rust finds `Ext::m` at
        // step `&C`, before any deref (EXECUTED Fable `fxc` h1_bridge: the blanket ran; the branch had read the Target's inherent
        // and dropped it). The closure takes every std trait one of whose blanket impls' `Self` requirements it already holds,
        // to a fixpoint (the generator's `I <trait> *` rows). A blanket row with no recorded requirement is taken as covering
        // every type. That is exact for `Any`/`Borrow`/`From`; for `Debug`/`Hash`/`PartialEq`/`Ord` the generator dropped an
        // unstable requirement (`impl<F: FnPtr> Debug for F`), so it over-covers. Over-covering only ever KEEPS a candidate
        // (the union direction, an over-charge), never decides one away.
        let blankets = t.blanket_impls();
        loop {
            let mut grew = false;
            for (p, rows) in blankets {
                let l = crate::std_bounds::leaf(p);
                if closure.contains(l) {
                    continue;
                }
                if rows.iter().any(|r| r.iter().all(|q| closure.contains(crate::std_bounds::leaf(q)))) {
                    closure.insert(l.to_string());
                    if let Some(row) = t.trait_row(p) {
                        closure.extend(row.supers.iter().map(|s| crate::std_bounds::leaf(s).to_string()));
                    }
                    grew = true;
                }
            }
            if !grew {
                break;
            }
        }
        spec.split(';').any(|imp| {
            imp.split('+').filter(|b| !b.is_empty()).all(|b| {
                closure.contains(b)
                    || matches!(b, "Send" | "Sync" | "Unpin" | "Sized")
                    || !(self.local_traits.contains_key(b) || crate::std_bounds::is_std_trait_leaf(b))
            })
        })
    }

    /// The `Deref` step's targets of generic `g`: every written `Target` binding on its `Deref`/`DerefMut` bounds.
    fn bp_deref_targets(&self, g: &str) -> Vec<Val> {
        let mut out: Vec<Val> = Vec::new();
        for tr in self.bp.env.bounds(g) {
            if !matches!(tr.leaf.as_str(), "Deref" | "DerefMut") {
                continue;
            }
            for (n, t) in &tr.bindings {
                if n == "Target" {
                    let v = self.bp_written(t);
                    if !out.contains(&v) {
                        out.push(v);
                    }
                }
            }
        }
        // `C: Deref` with no written `Target` but `where C::Target: Tr` (rand_core's, plotters' shape): the Target is the
        // projection, and its bounds are the answer (step 4).
        if out.is_empty()
            && self.bp.env.bounds(g).iter().any(|t| matches!(t.leaf.as_str(), "Deref" | "DerefMut"))
            && self.bp.env.of_projection.contains_key(&(g.to_string(), "Target".to_string()))
        {
            out.push(Val::Proj { g: g.to_string(), tr: Some("Deref".to_string()), assoc: "Target".to_string() });
        }
        out
    }

    /// The member `leaf` on a value of generic `g`, at autoderef depth `depth`. `None`: the probe has nothing to add, and the
    /// existing routes answer as before.
    fn bp_member_generic(&self, g: &str, leaf: &str, depth: usize) -> Option<Ans> {
        if depth > MAX_DEREF || universal_member(leaf) {
            return None;
        }
        let bounds: Vec<TraitRef> = self.bp.env.bounds(g).to_vec();
        let s1 = self.bp_step1(&bounds, leaf);
        if s1.hit {
            // At the receiver itself, today's bounded dispatch is this answer already: change nothing.
            if depth == 0 {
                return None;
            }
            let mut a = Ans { traits: s1.local_hits.clone(), ..Default::default() };
            for st in &s1.std_hits {
                // a std trait's member over a caller-chosen type: the engine-wide rule (R1286) hedges the I/O-driving ones
                if crate::lang::is_driving_io_member(st, leaf) {
                    a.hedges.push(format!("dispatch:{st}.{leaf}"));
                }
            }
            for h in s1.incomplete {
                if !a.hedges.contains(&h) {
                    a.hedges.push(h);
                }
            }
            return Some(a);
        }
        let targets = self.bp_deref_targets(g);
        if targets.is_empty() {
            return None;
        }
        let mut a = Ans::default();
        let mut answered = false;
        for t in &targets {
            if let Some(x) = self.bp_member_val(t, leaf, depth + 1) {
                a.merge(x);
                answered = true;
            } else {
                a.hedges.push(format!("dispatch:unresolved Deref target of `{leaf}`"));
            }
        }
        if !answered {
            return None;
        }
        // The union with every step-1 candidate whose member list is not complete (R-C) and every local blanket impl whose
        // applicability is undecided (Fable review §2.3, item 3) — never the probe-order pick.
        for h in s1.incomplete {
            if !a.hedges.contains(&h) {
                a.hedges.push(h);
            }
        }
        for b in s1.blanket {
            if !a.traits.contains(&b) {
                a.traits.push(b);
            }
        }
        a.keep_dispatch_route |= s1.dep_bound;
        a.via_deref |= !a.typed.is_empty();
        a.facts.insert("bound-deref");
        Some(a)
    }

    /// The member `leaf` on a value `v` reached at depth `depth`.
    fn bp_member_val(&self, v: &Val, leaf: &str, depth: usize) -> Option<Ans> {
        if depth > MAX_DEREF {
            return None;
        }
        match v {
            Val::Nominal(t) => Some(Ans { typed: vec![t.clone()], ..Default::default() }),
            Val::Generic(x) => self.bp_member_generic(x, leaf, depth),
            Val::Dyn(leaves) => {
                let mut a = Ans::default();
                for l in leaves {
                    if self.local_traits.contains_key(l) && self.trait_declares_method(l, leaf, 0) {
                        a.traits.push(l.clone());
                    } else if !matches!(l.as_str(), "Send" | "Sync" | "Unpin") {
                        a.hedges.push(format!("dispatch:{l}.{leaf}"));
                    }
                }
                (!a.is_empty()).then_some(a)
            }
            Val::Proj { g, tr, assoc } => self.bp_member_proj(g, tr.as_deref(), assoc, leaf, depth),
            Val::Opaque => None,
        }
    }

    /// The projection's facts (see `ProjFacts`). `via`: the trait the projection was written through (`<g as via>::assoc`), which
    /// limits the bounds of `g` whose bindings and declarations count; `Target` reaches `Deref`'s through `DerefMut` too.
    pub(crate) fn bp_proj_facts(&self, g: &str, via: Option<&str>, assoc: &str) -> ProjFacts {
        let bound_tr = |t: &TraitRef| via.is_none_or(|x| x == t.leaf || (assoc == "Target" && matches!(t.leaf.as_str(), "Deref" | "DerefMut")));
        let mut f = ProjFacts::default();
        for t in self.bp.env.bounds(g).iter().filter(|t| bound_tr(t)) {
            for (n, ty) in &t.bindings {
                if n == assoc {
                    let v = self.bp_written(ty);
                    if !f.bindings.contains(&v) {
                        f.bindings.push(v);
                    }
                }
            }
        }
        f.written = self.bp.env.of_projection.get(&(g.to_string(), assoc.to_string())).cloned().unwrap_or_default();
        for t in self.bp.env.bounds(g).iter().filter(|t| bound_tr(t)) {
            let Some(lt) = self.local_traits.get(&t.leaf) else { continue };
            for b in lt.assoc_types.get(assoc).into_iter().flatten() {
                if !f.written.iter().chain(f.declared.iter()).any(|x| &x.leaf == b) {
                    f.declared.push(TraitRef { leaf: b.clone(), path: b.clone(), args: Vec::new(), bindings: Vec::new() });
                }
            }
        }
        f
    }

    /// A written bound's origin, by its RESOLVED ROOT (`Origin`).
    pub(crate) fn bp_origin(&self, t: &TraitRef) -> Origin {
        let full = crate::lang::expand(&t.path, &self.uses);
        let dep = full.contains("::") && crate::lang::is_dependency_crate_root(full.split("::").next().unwrap_or(""));
        let local = self.local_traits.contains_key(&t.leaf);
        if dep {
            Origin::Dep { full, local }
        } else if local {
            Origin::Local
        } else if crate::std_bounds::is_std_trait_leaf(&t.leaf) || matches!(t.leaf.as_str(), "Send" | "Sync" | "Unpin" | "Sized" | "Copy") {
            Origin::Std
        } else {
            Origin::Unrooted
        }
    }

    /// Design step 4, THE PROJECTION ARM (R1795): the member `leaf` on a value of `<g as tr>::assoc`. Three sources, in this
    /// order: a written BINDING on a bound of `g` (`C: Has2<A = Conn>`, `C: Deref<Target = X>`) — the projection IS that
    /// type; else the projection's BOUND SET, from a `where` clause (`where C::A: Tr`, `where C::Target: Tr`) and from the
    /// local trait's own declaration (`type A: DoerD;`), dispatched like a parameter with those bounds. Nothing else: a
    /// projection with neither is not answered here.
    fn bp_member_proj(&self, g: &str, tr: Option<&str>, assoc: &str, leaf: &str, depth: usize) -> Option<Ans> {
        if depth > MAX_DEREF {
            return None;
        }
        let facts = self.bp_proj_facts(g, tr, assoc);
        let bindings = facts.bindings;
        if !bindings.is_empty() {
            let mut a = Ans::default();
            let mut answered = false;
            for v in &bindings {
                if let Some(x) = self.bp_member_val(v, leaf, depth + 1) {
                    a.merge(x);
                    answered = true;
                }
            }
            a.facts.insert("projection");
            return answered.then_some(a);
        }
        let mut bset: Vec<TraitRef> = facts.written;
        bset.extend(facts.declared);
        if bset.is_empty() {
            return None;
        }
        let s1 = self.bp_step1(&bset, leaf);
        if !s1.hit {
            return None;
        }
        let mut a = Ans { traits: s1.local_hits.clone(), ..Default::default() };
        for st in &s1.std_hits {
            if crate::lang::is_driving_io_member(st, leaf) {
                a.hedges.push(format!("dispatch:{st}.{leaf}"));
            }
        }
        for h in s1.incomplete {
            if !a.hedges.contains(&h) {
                a.hedges.push(h);
            }
        }
        a.facts.insert("projection");
        Some(a)
    }

    /// SOUNDNESS R1940 — the PATH-call form of the projection arm: `T::Item::f()` / `T::A::g()`. `route_trait_member_path` answered this
    /// spelling only from a LOCAL bound trait's declaration of the associated type, so a bound written on the projection itself (`where
    /// T::Item: Lt`, `T: Iterator<Item: Lt>`, an impl's clause) or a written `Item = X` binding, under a STD-only bound on `T`, formed no
    /// edge, no key and no disclosure (EXECUTED `fl40_*`). The sources are the receiver spelling's own (`bp_member_proj`, R1795): a written
    /// binding, then the projection's bound set; the difference is the MEMBER TEST, because an associated fn with no receiver (`f()`, the
    /// usual path-call member) is in the trait's `assoc`, not its `methods` (`declaring_traits(.., true)`).
    ///
    /// The facts are `bp_proj_facts` (the receiver arm's own): `via` (R1959) limits the bindings to the bound the projection was
    /// written through. Only step 1(a) is read here; step 1(b), a local trait's own `type A: B`, is the R1037 arm's, whose member
    /// rendering dispatches over the BOUND and keys the declaring trait, where this arm's `declaring_traits` rendering dispatches over
    /// the declaring trait's implementors (R1953): reading 1(b) here would widen that over-charge to the population R1037 answers
    /// exactly. Each written bound's origin is its RESOLVED ROOT (`bp_origin`, R1952): a dependency trait named like a std one
    /// (`dep::Error`, `dep::Read`, `dep::Sub`) was read as std by its leaf and dropped, standalone and chained (EXECUTED `fr40_depsup_e`,
    /// `wg3_{err,read,ptr}_proj_e`).
    pub(crate) fn bp_path_projection(&self, g: &str, via: Option<&str>, assoc: &str, leaf: &str) -> PathProj {
        let mut out = PathProj::default();
        if bp_off() || universal_member(leaf) {
            return out;
        }
        let facts = self.bp_proj_facts(g, via, assoc);
        for v in facts.bindings {
            match v {
                Val::Nominal(x) => {
                    if !out.typed.contains(&x) {
                        out.typed.push(x);
                    }
                }
                _ => out.unnamed = true, // bound to a generic / dyn / opaque type this route does not name
            }
        }
        if !out.typed.is_empty() || out.unnamed {
            return out; // a written binding IS the projection's type; its bound set is not consulted (as `bp_member_proj`)
        }
        for t in &facts.written {
            let l = t.leaf.as_str();
            let origin = self.bp_origin(t);
            if let Origin::Dep { local: false, .. } = origin {
                if crate::lang::reach_debug() && (crate::std_bounds::is_std_trait_leaf(l) || matches!(l, "Send" | "Sync" | "Unpin" | "Sized" | "Copy")) {
                    eprintln!("R1952ROOT\t{g}::{assoc}::{leaf}\t{}", t.path); // §E1 REACH COUNTER (the changed branch only)
                }
                out.deps.push(t.clone());
                continue;
            }
            if self.local_traits.contains_key(l) {
                if self.local_declares_member(l, leaf, 0) {
                    // the DECLARING trait(s): `T::Item: Sub` with `Sub: Lt` and `lf` declared in `Lt` dispatches over `Lt`'s implementors
                    for d in self.declaring_traits(l, leaf, true) {
                        if !out.traits.contains(&d) {
                            out.traits.push(d);
                        }
                    }
                } else if self.local_trait_has_foreign_super(l) {
                    out.unnamed = true; // a dependency supertrait whose member list is unknown may declare it
                }
                // a dependency trait sharing the leaf (`dep::Lt` beside a local `Lt`) is a candidate as well: its key, beside today's
                if matches!(origin, Origin::Dep { .. }) {
                    out.deps.push(t.clone());
                }
            } else if crate::std_bounds::is_std_trait_leaf(l) || matches!(l, "Send" | "Sync" | "Unpin" | "Sized" | "Copy") {
                // a std bound is not this route's to answer (the receiver spelling's R1286 hedge is for driving I/O members of a
                // receiver), but one that DECLARES the member means a dependency bound beside it is not the only candidate
                let table = crate::std_bounds::table();
                let paths: Vec<&str> = match table.resolve_trait(&t.path) {
                    Some(p) => vec![p],
                    None => table.traits_by_leaf(l).to_vec(),
                };
                if paths.iter().any(|p| std_declares(p, leaf)) {
                    out.std_declares = true;
                }
            } else {
                out.deps.push(t.clone());
            }
        }
        out
    }

    /// Does the local trait `l` (or a local supertrait) declare `leaf` as a method OR an associated fn?
    fn local_declares_member(&self, l: &str, leaf: &str, depth: usize) -> bool {
        if depth > 16 {
            return false;
        }
        let Some(lt) = self.local_traits.get(l) else { return false };
        lt.methods.contains(leaf) || lt.assoc.contains(leaf) || lt.supertraits.iter().any(|s| self.local_declares_member(s, leaf, depth + 1))
    }

    /// Does the local trait `l` have a supertrait (transitively) that is neither local nor std — one whose member list is unknown?
    fn local_trait_has_foreign_super(&self, l: &str) -> bool {
        let mut stack = vec![(l.to_string(), 0usize)];
        let mut seen: BTreeSet<String> = BTreeSet::new();
        while let Some((t, d)) = stack.pop() {
            if !seen.insert(t.clone()) || d > 16 {
                continue;
            }
            let Some(lt) = self.local_traits.get(&t) else { continue };
            for sup in &lt.supertraits {
                if self.local_traits.contains_key(sup) {
                    stack.push((sup.clone(), d + 1));
                } else if !crate::std_bounds::is_std_trait_leaf(sup) && !matches!(sup.as_str(), "Send" | "Sync" | "Unpin" | "Sized" | "Copy") {
                    return true;
                }
            }
        }
        false
    }

    /// SOUNDNESS R1723's reading, for a FIELD typed by its owner's own type parameter: the impl-scope generic that position is
    /// instantiated with (`self.session` over `struct Stream<'a, C> { session: &'a mut C }` inside `impl<C: ..> Stream<'_, C>`).
    fn bp_field_param(&self, base_expr: &syn::Expr, key: &str) -> Option<String> {
        if self.bp.env.is_empty() {
            return None;
        }
        let owner = self.resolve_recv_type(base_expr)?;
        if self.is_dependency_type(&owner) {
            return None;
        }
        let (qi, qual) = self.owner_qual(base_expr, &owner)?;
        let v = qi.fields.get(&qual)?.get(key)?;
        let params = qi.params.get(&qual)?;
        let pos = params.iter().position(|p| p == v)?;
        let owner_leaf = owner.rsplit("::").next().unwrap_or(&owner).to_string();
        let fits = |(l, args): &(String, Vec<String>)| {
            let l = self.returns.get(&crate::model::projection_of_key(l)).map(String::as_str).unwrap_or(l);
            l == owner_leaf && args.len() == params.len()
        };
        let self_leaf = self.uses.get(crate::lang::SELF_KEY).map(|s| s.rsplit("::").next().unwrap_or(s).to_string());
        let ev = self.scrut_evidence(base_expr).filter(fits).or_else(|| {
            (self_leaf.as_deref() == Some(owner_leaf.as_str())).then(|| self.scrut_evidence(&syn::parse_quote!(self))).flatten().filter(fits)
        });
        let (_, args) = ev?;
        let a = args.get(pos)?.clone();
        self.bp.env.params.contains(&a).then_some(a)
    }

    /// The generic node a RECEIVER expression is, if it is one: a binding holding a generic (`c`), `self` in an impl over a
    /// parameter, or a field typed by the owner's parameter. `recv_ty` is what the typer answered (a bare parameter NAME
    /// there is the same fact).
    pub(crate) fn bp_node(&self, recv: &syn::Expr, recv_ty: Option<&str>) -> Option<Val> {
        let mut e = recv;
        loop {
            match e {
                syn::Expr::Paren(p) => e = &p.expr,
                syn::Expr::Group(g) => e = &g.expr,
                syn::Expr::Reference(r) => e = &r.expr,
                syn::Expr::Unary(u) if matches!(u.op, syn::UnOp::Deref(_)) => e = &u.expr,
                _ => break,
            }
        }
        match e {
            syn::Expr::Path(p) if p.qself.is_none() => {
                let id = p.path.get_ident()?.to_string();
                if let Some(v) = self.bp.generic_vars.get(&id) {
                    return Some(v.clone().into());
                }
                if id == "self" {
                    if let Some(t) = self.vars.get("self").filter(|t| self.bp.env.params.contains(*t)) {
                        return Some(Val::Generic(t.clone()));
                    }
                }
            }
            syn::Expr::Field(fe) => {
                if let syn::Member::Named(n) = &fe.member {
                    if let Some(g) = self.bp_field_param(&fe.base, &n.to_string()) {
                        return Some(Val::Generic(g));
                    }
                }
            }
            // a call's value is never the generic itself here: that is return substitution's question (`bp_conv_vals`)
            _ => return None,
        }
        // a binding or field the typer itself answered with a bare parameter NAME (which names no type)
        recv_ty.filter(|t| !t.contains("::") && self.bp.env.params.contains(*t)).map(|t| Val::Generic(t.to_string()))
    }

    /// The written `Target` (or other associated-type) binding on ANY bound of `g` (`DerefMut::deref_mut` returns the
    /// `Target` its supertrait `Deref` binds, which may be written on either bound).
    fn bp_binding(&self, g: &str, assoc: &str) -> Vec<Val> {
        let mut out: Vec<Val> = Vec::new();
        for tr in self.bp.env.bounds(g) {
            for (n, t) in &tr.bindings {
                if n == assoc {
                    let v = self.bp_written(t);
                    if !out.contains(&v) {
                        out.push(v);
                    }
                }
            }
        }
        out
    }

    /// RETURN SUBSTITUTION (design step 3): what `x.method()` returns when `x` is a value of generic `g` and `method` is a
    /// member of one of `g`'s bounds whose declared return is the trait's own type parameter or associated type. For a std
    /// trait the slot is read off std's declaration (`std_bounds` `R` rows: `as_ref -> &T` is `p0`, `deref -> &Self::Target`
    /// is `a:Target`); for a local trait, its `-> Self::<Assoc>` members (`model::trait_ret_assoc_key`). The argument or
    /// binding is resolved as WRITTEN, generics first, so `C: AsRef<X>` with `X` a parameter yields `Generic(X)` and never a
    /// type named `X` (Fable review §2.1, s12: the CONV twin of the plotters C3). An unbound associated type is a projection.
    fn bp_ret_vals(&self, g: &str, method: &str) -> Vec<Val> {
        let mut out: Vec<Val> = Vec::new();
        let push = |v: Val, out: &mut Vec<Val>| {
            if !out.contains(&v) {
                out.push(v);
            }
        };
        let t = crate::std_bounds::table();
        for tr in self.bp.env.bounds(g) {
            let l = tr.leaf.as_str();
            if let Some(lt) = self.local_traits.get(l) {
                for a in lt.assoc_types.get(&crate::model::trait_ret_assoc_key(method)).into_iter().flatten() {
                    match tr.bindings.iter().find(|(n, _)| n == a) {
                        Some((_, ty)) => push(self.bp_written(ty), &mut out),
                        None => push(Val::Proj { g: g.to_string(), tr: Some(l.to_string()), assoc: a.clone() }, &mut out),
                    }
                }
                continue;
            }
            if !crate::std_bounds::is_std_trait_leaf(l) {
                continue;
            }
            let paths: Vec<&str> = match t.resolve_trait(&tr.path) {
                Some(p) => vec![p],
                None => t.traits_by_leaf(l).to_vec(),
            };
            // the member may be declared by a supertrait of the bound (`DerefMut` -> `Deref::deref`)
            let mut stack: Vec<String> = paths.iter().map(|p| p.to_string()).collect();
            let mut seen: BTreeSet<String> = BTreeSet::new();
            while let Some(p) = stack.pop() {
                if !seen.insert(p.clone()) {
                    continue;
                }
                if let Some(slot) = t.ret_slot(&p, method) {
                    if let Some(n) = slot.strip_prefix('p').and_then(|n| n.parse::<usize>().ok()) {
                        if let Some(a) = tr.args.get(n) {
                            push(self.bp_written(a), &mut out);
                        }
                    } else if let Some(assoc) = slot.strip_prefix("a:") {
                        let b = self.bp_binding(g, assoc);
                        if b.is_empty() {
                            push(Val::Proj { g: g.to_string(), tr: Some(crate::std_bounds::leaf(&p).to_string()), assoc: assoc.to_string() }, &mut out);
                        }
                        for v in b {
                            push(v, &mut out);
                        }
                    }
                }
                if let Some(row) = t.trait_row(&p) {
                    stack.extend(row.supers.iter().map(|s| s.to_string()));
                }
            }
        }
        out
    }

    /// The values a RECEIVER expression holds when it is a conversion (or other bound member with a substitutable return)
    /// called on a generic node: `p.as_ref()`, `c.borrow()`, `self.deref()`, `c.get()` under `C: Has`. Nested conversions
    /// recurse. Empty for anything else.
    pub(crate) fn bp_conv_vals(&self, recv: &syn::Expr, depth: usize) -> Vec<Val> {
        if depth > MAX_DEREF {
            return Vec::new();
        }
        let mut e = recv;
        while let syn::Expr::Paren(p) = e {
            e = &p.expr;
        }
        let syn::Expr::MethodCall(mc) = e else { return Vec::new() };
        if !mc.args.is_empty() || mc.turbofish.is_some() {
            return Vec::new();
        }
        let m = mc.method.to_string();
        let inner: Vec<Val> = match self.bp_node(&mc.receiver, None) {
            Some(v) => vec![v],
            None => self.bp_conv_vals(&mc.receiver, depth + 1),
        };
        let mut out: Vec<Val> = Vec::new();
        for v in inner {
            if let Val::Generic(g) = v {
                // Rust finds the conversion at the generic's own step when a bound declares it; otherwise it is not this rule
                if !self.bp_step1(self.bp.env.bounds(&g), &m).hit {
                    continue;
                }
                for x in self.bp_ret_vals(&g, &m) {
                    if !out.contains(&x) {
                        out.push(x);
                    }
                }
            }
        }
        out
    }

    /// THE PROBE, for the receiver of `.leaf()`: `None` leaves every existing route exactly as it was.
    pub(crate) fn bp_answer(&self, recv: &syn::Expr, leaf: &str, recv_ty: Option<&str>) -> Option<Ans> {
        if bp_off() || self.bp.env.is_empty() {
            return None;
        }
        if let Some(node) = self.bp_node(recv, recv_ty) {
            let a = match &node {
                Val::Generic(g) => self.bp_member_generic(g, leaf, 0)?,
                // a binding typed `<C as Has>::A` (d4): no route answers it today, so the projection's own answer stands
                Val::Proj { g, tr, assoc } => self.bp_member_proj(g, tr.as_deref(), assoc, leaf, 1)?,
                _ => return None,
            };
            return (!a.is_empty()).then_some(a);
        }
        // design step 3: a conversion's return, substituted (R1793)
        let vals = self.bp_conv_vals(recv, 0);
        if vals.is_empty() {
            return None;
        }
        let mut a = Ans::default();
        let mut answered = false;
        for v in &vals {
            match self.bp_member_val(v, leaf, 1) {
                Some(x) => {
                    a.merge(x);
                    answered = true;
                }
                None => a.hedges.push(format!("dispatch:unresolved conversion value of `{leaf}`")),
            }
        }
        if !answered {
            return None;
        }
        a.facts.insert("bound-conv");
        (!a.is_empty()).then_some(a)
    }
}

impl<'a> super::CallCollector<'a> {
    /// Record a probe answer at its call site. `typed_route`: the first typed receiver goes down the ordinary typed-call
    /// route (the caller set `recv_ty` to it); every other typed receiver is pushed here as its own typed call.
    pub(crate) fn bp_apply(&mut self, a: &Ans, node: &syn::ExprMethodCall, leaf: &str, str_arg: Option<String>, typed_route: bool) {
        let skip = usize::from(typed_route);
        for t in a.typed.iter().skip(skip) {
            self.calls.push(crate::model::Call {
                sp: None, assoc_via: None, cside: None, prov: None, exact_ty: None, recv_form: a.recv_form(), dot: true,
                argc: node.args.len().min(255) as u8, entropy_arg: false,
                path: crate::lang::alias_join(t, &[leaf]), leaf: leaf.to_string(), str_arg: str_arg.clone(),
                typed: true, method: true, is_macro: false, path_lits_partial: false, path_lit2: None,
            });
        }
        for tr in &a.traits {
            if !self.dispatch_calls_for_trait_method(tr, leaf, str_arg.clone()) {
                self.mark_unresolved(format!("dispatch:{tr}.{leaf}"));
            }
        }
        for h in &a.hedges {
            self.mark_unresolved(h.clone());
        }
        crate::sidecar::note("recv_fact", &a.fact());
        if candor_report::env::trace("CANDOR_R1657BP_INSTR") {
            eprintln!("R1657BP\t{}\t.{leaf}\ttyped={:?}\ttraits={:?}\thedges={:?}\tkeep={}", a.fact(), a.typed, a.traits, a.hedges,
                a.keep_dispatch_route); // §E1 REACH COUNTER
        }
    }
}

