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
        self.facts.extend(o.facts);
    }
    pub(crate) fn fact(&self) -> String {
        self.facts.iter().copied().collect::<Vec<_>>().join("+")
    }
    pub(crate) fn is_empty(&self) -> bool {
        self.typed.is_empty() && self.traits.is_empty() && self.hedges.is_empty()
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
            Val::Proj { .. } | Val::Opaque => None,
        }
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

    /// THE PROBE, for the receiver of `.leaf()`: `None` leaves every existing route exactly as it was.
    pub(crate) fn bp_answer(&self, recv: &syn::Expr, leaf: &str, recv_ty: Option<&str>) -> Option<Ans> {
        if bp_off() || self.bp.env.is_empty() {
            return None;
        }
        if let Some(node) = self.bp_node(recv, recv_ty) {
            let a = match &node {
                Val::Generic(g) => self.bp_member_generic(g, leaf, 0)?,
                _ => return None,
            };
            return (!a.is_empty()).then_some(a);
        }
        None
    }
}

impl<'a> super::CallCollector<'a> {
    /// Record a probe answer at its call site. `typed_route`: the first typed receiver goes down the ordinary typed-call
    /// route (the caller set `recv_ty` to it); every other typed receiver is pushed here as its own typed call.
    pub(crate) fn bp_apply(&mut self, a: &Ans, node: &syn::ExprMethodCall, leaf: &str, str_arg: Option<String>, typed_route: bool) {
        let skip = usize::from(typed_route);
        for t in a.typed.iter().skip(skip) {
            self.calls.push(crate::model::Call {
                sp: None, assoc_via: None, cside: None, prov: None, exact_ty: None, recv_form: 2, dot: true,
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

