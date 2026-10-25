//! Call-target round 2 — GENERATE the std bound table (`std_bounds.tsv`) from the toolchain's rust-src.
//!
//!     bash crates/candor-scan/gen/std_bounds.sh [--check]     # regenerate src/std_bounds.tsv / verify it reproduces
//!
//! (a `#[test] #[ignore]` in this crate, `std_bounds_gen::regenerate`, because it reuses the engine's own
//! macro-by-example expander, `mbe`, for the impls std declares through `macro_rules!`; the crate is a binary.)
//!
//! What the table records is what a std DECLARATION says user code may be run for, and nothing an engine author
//! believes about std's bodies:
//!
//! * `T` — every stable `pub trait`: its supertraits, and its members (required / provided, and whether a provided
//!   member's return value carries `Self` — a lazy adapter such as `Iterator::map` — `d`).
//! * `I` — every stable impl of such a trait whose implementing type is a std shape (`Vec`, `[T]`, `Option`, a tuple, a
//!   reference, `*` for a blanket impl over a type parameter): which TYPE ARGUMENT of the implementing type must
//!   implement which traits (`impl<T: Debug, A> Debug for Vec<T, A>` → `e0=Debug`). The rule an engine reads off it:
//!   a bound licenses the bound trait's members, and its supertraits' members, on that argument.
//! * `F` — every stable public std fn and method with a trait bound on a generic parameter, and WHERE that parameter
//!   reaches the call: the receiver's type argument `eN`, the receiver itself `s` (a trait's own `Self`), the
//!   receiver's `Self::Item` `i`, a non-receiver argument `aN`, or only the return type `r`; `:d` when the slot is
//!   moved into the returned value (`BufWriter::new(w)`, `iter.zip(other)`) rather than used at the call.
//!
//! Macro-generated impls (tuples, the slice-equality family) are read from the `macro_rules!` bodies with every
//! repetition collapsed to one parameter; the impls rustc builds itself (`Clone`/`Copy` for tuples) are read from the
//! bullet list `core/src/primitive_docs.rs` publishes for them. Nightly-only syntax in rust-src (`const impl`,
//! `[const] Trait`) is rewritten to its stable spelling before parsing. A file syn cannot parse is reported on stderr
//! and in the table's `# unparsed` trailer, so a miss is a listed miss.
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::path::{Path, PathBuf};

#[derive(Default, Clone)]
struct TraitDecl {
    path: String,
    supers: Vec<String>, // written bound paths, resolved after collection
    req: BTreeSet<String>,
    prov: BTreeSet<String>,
    prov_deferred: BTreeSet<String>,
    uses: HashMap<String, String>,
    module: String,
}

struct ImplDecl {
    tr: String, // written trait path
    shape: String,
    slots: Vec<(String, Vec<String>)>,
    uses: HashMap<String, String>,
    module: String,
}

struct FnDecl {
    owner: String,
    name: String,
    nargs: usize,
    slots: Vec<(String, Vec<String>, bool)>,
    uses: HashMap<String, String>,
    module: String,
}

#[derive(Default)]
struct Out {
    traits: Vec<TraitDecl>,
    impls: Vec<ImplDecl>,
    fns: Vec<FnDecl>,
    unparsed: Vec<String>,
    unexpanded: BTreeSet<String>,
}

fn is_unstable(attrs: &[syn::Attribute]) -> bool {
    attrs.iter().any(|a| a.path().is_ident("unstable"))
}

fn is_cfg_test(attrs: &[syn::Attribute]) -> bool {
    attrs.iter().any(|a| {
        let s = tokens(&a.meta);
        a.path().is_ident("cfg") && s.contains("test") && !s.contains("not (test)") && !s.contains("not(test)")
    })
}

fn tokens<T: syn::__private::ToTokens>(t: &T) -> String {
    let mut ts = proc_macro2::TokenStream::new();
    t.to_tokens(&mut ts);
    ts.to_string()
}

fn path_string(p: &syn::Path) -> String {
    p.segments.iter().map(|s| s.ident.to_string()).collect::<Vec<_>>().join("::")
}

/// The std SHAPE an implementing / inherent type is: its last path segment (`Vec`, `Option`, `str`), `[]` a slice,
/// `[;]` an array, `()` a tuple, `&` a reference, `*` a bare type parameter (a blanket impl). Its TYPE ARGUMENTS, in
/// order, as the types written.
fn shape_of<'a>(ty: &'a syn::Type, params: &BTreeSet<String>) -> Option<(String, Vec<&'a syn::Type>)> {
    match ty {
        syn::Type::Path(p) if p.qself.is_none() => {
            let seg = p.path.segments.last()?;
            let id = seg.ident.to_string();
            if p.path.segments.len() == 1 && params.contains(&id) {
                return Some(("*".into(), vec![ty]));
            }
            let mut args = Vec::new();
            if let syn::PathArguments::AngleBracketed(ab) = &seg.arguments {
                for a in &ab.args {
                    if let syn::GenericArgument::Type(t) = a {
                        args.push(t);
                    }
                }
            }
            Some((id, args))
        }
        syn::Type::Slice(s) => Some(("[]".into(), vec![&*s.elem])),
        syn::Type::Array(a) => Some(("[;]".into(), vec![&*a.elem])),
        syn::Type::Tuple(t) if !t.elems.is_empty() => Some(("()".into(), t.elems.iter().collect())),
        syn::Type::Reference(r) => Some(("&".into(), vec![&*r.elem])),
        syn::Type::Ptr(_) => None,
        syn::Type::Paren(p) => shape_of(&p.elem, params),
        _ => None,
    }
}

fn as_param(ty: &syn::Type, params: &BTreeSet<String>) -> Option<String> {
    match ty {
        syn::Type::Path(p) if p.qself.is_none() && p.path.segments.len() == 1 => {
            let id = p.path.segments[0].ident.to_string();
            params.contains(&id).then_some(id)
        }
        syn::Type::Reference(r) => as_param(&r.elem, params),
        syn::Type::Paren(p) => as_param(&p.elem, params),
        _ => None,
    }
}

fn mentions(ty: &syn::Type, name: &str) -> bool {
    let s = tokens(ty);
    s.split(|c: char| !(c.is_alphanumeric() || c == '_')).any(|w| w == name)
}

/// A provided member's (or a slot's) value is DEFERRED when the fn returns a value that holds it: the return type
/// names the parameter (or `Self`) inside a NON-transparent wrapper. A plain `T`, `Option<T>`, `Result<T, _>`, `Vec<T>`,
/// a tuple of those, `bool`/numbers — a value computed now — is not deferral; `Map<Self, F>`, `&mut Self`,
/// `BufWriter<W>`, `Zip<Self, U::IntoIter>` are.
thread_local! {
    /// The associated types the trait being read declares WITH a trait bound (`type IntoIter: Iterator<..>`): a member
    /// returning `Self::IntoIter` hands back a value that is the receiver turned into another implementor — deferral —
    /// where `Self::Item` / `Self::Output` (no bound) is a value computed now.
    static BOUNDED_ASSOC: std::cell::RefCell<BTreeSet<String>> = const { std::cell::RefCell::new(BTreeSet::new()) };
}

fn deferred_in(out: &syn::ReturnType, name: &str) -> bool {
    let syn::ReturnType::Type(_, ty) = out else { return false };
    fn walk(ty: &syn::Type, name: &str) -> bool {
        match ty {
            syn::Type::Reference(r) => {
                // `&mut Self` hands the receiver itself back (`by_ref`): deferred. A reference to anything else is a
                // value computed now (`first(&self) -> Option<&T>`), read like the referent.
                if name == "Self" && tokens(&r.elem) == "Self" {
                    return true;
                }
                walk(&r.elem, name)
            }
            syn::Type::Tuple(t) => t.elems.iter().any(|e| walk(e, name)),
            syn::Type::Path(p) if p.qself.is_none() => {
                let seg = match p.path.segments.last() {
                    Some(s) => s,
                    None => return false,
                };
                let id = seg.ident.to_string();
                let first = p.path.segments.first().map(|s| s.ident.to_string()).unwrap_or_default();
                if p.path.segments.len() == 1 && id == name {
                    return false; // returns the value itself
                }
                if first == name {
                    // a projection: `Self::Item` is a value of another type; `Self::IntoIter` (a BOUNDED associated type)
                    // is the receiver converted, handed back
                    return name == "Self" && p.path.segments.len() == 2 && BOUNDED_ASSOC.with(|b| b.borrow().contains(&id));
                }
                let transparent = matches!(id.as_str(), "Option" | "Result" | "Vec" | "Box" | "Ordering" | "ControlFlow");
                let mut any = false;
                if let syn::PathArguments::AngleBracketed(ab) = &seg.arguments {
                    for a in &ab.args {
                        if let syn::GenericArgument::Type(t) = a {
                            if transparent {
                                any |= walk(t, name);
                            } else if mentions(t, name) {
                                any = true;
                            }
                        }
                    }
                }
                any
            }
            _ => false,
        }
    }
    walk(ty, name)
}

/// Bounds written on each generic parameter: `<T: A + B>`, `where T: C`, and `impl Trait` arguments (named `#implN`).
fn collect_bounds(gens: &[&syn::Generics], out: &mut BTreeMap<String, Vec<String>>) {
    fn bound_paths(bs: &syn::punctuated::Punctuated<syn::TypeParamBound, syn::Token![+]>) -> Vec<String> {
        let mut v = Vec::new();
        for b in bs {
            if let syn::TypeParamBound::Trait(tb) = b {
                if matches!(tb.modifier, syn::TraitBoundModifier::Maybe(_)) {
                    continue;
                }
                v.push(path_string(&tb.path));
            }
        }
        v
    }
    for g in gens {
        for p in &g.params {
            if let syn::GenericParam::Type(t) = p {
                out.entry(t.ident.to_string()).or_default().extend(bound_paths(&t.bounds));
            }
        }
        if let Some(w) = &g.where_clause {
            for pred in &w.predicates {
                if let syn::WherePredicate::Type(pt) = pred {
                    let key = match &pt.bounded_ty {
                        syn::Type::Path(p) if p.qself.is_none() => {
                            let segs: Vec<String> = p.path.segments.iter().map(|s| s.ident.to_string()).collect();
                            match segs.as_slice() {
                                [one] => one.clone(),
                                [s, item] if s == "Self" && item == "Item" => "Self::Item".into(),
                                _ => continue,
                            }
                        }
                        syn::Type::Path(p) if p.qself.is_some() => {
                            // `<Self as Iterator>::Item`
                            let last = p.path.segments.last().map(|s| s.ident.to_string()).unwrap_or_default();
                            let q = tokens(&p.qself.as_ref().unwrap().ty);
                            if q == "Self" && last == "Item" {
                                "Self::Item".into()
                            } else {
                                continue;
                            }
                        }
                        _ => continue,
                    };
                    out.entry(key).or_default().extend(bound_paths(&pt.bounds));
                }
            }
        }
    }
}

/// The generic parameters a `Self: Iterator<Item = P>` / `Item = &P` predicate names as the receiver's item.
fn item_params(gens: &[&syn::Generics]) -> Vec<String> {
    let mut out = Vec::new();
    for g in gens {
        let Some(w) = &g.where_clause else { continue };
        for pred in &w.predicates {
            let syn::WherePredicate::Type(pt) = pred else { continue };
            if tokens(&pt.bounded_ty) != "Self" {
                continue;
            }
            for b in &pt.bounds {
                let syn::TypeParamBound::Trait(tb) = b else { continue };
                let Some(seg) = tb.path.segments.last() else { continue };
                let syn::PathArguments::AngleBracketed(ab) = &seg.arguments else { continue };
                for a in &ab.args {
                    if let syn::GenericArgument::AssocType(at) = a {
                        if at.ident == "Item" {
                            let ty = match &at.ty {
                                syn::Type::Reference(r) => &*r.elem,
                                t => t,
                            };
                            if let syn::Type::Path(tp) = ty {
                                if let Some(id) = tp.path.get_ident() {
                                    out.push(id.to_string());
                                }
                            }
                        }
                    }
                }
            }
        }
    }
    out
}

struct Ctx<'o> {
    out: &'o mut Out,
    module: Vec<String>,
    uses: HashMap<String, String>,
    macros: &'o HashMap<String, Option<String>>,
    file_macros: HashMap<String, Option<String>>,
    depth: usize,
}

fn collect_uses(items: &[syn::Item], module: &[String], uses: &mut HashMap<String, String>) {
    fn walk(t: &syn::UseTree, prefix: &mut Vec<String>, module: &[String], uses: &mut HashMap<String, String>) {
        match t {
            syn::UseTree::Path(p) => {
                prefix.push(p.ident.to_string());
                walk(&p.tree, prefix, module, uses);
                prefix.pop();
            }
            syn::UseTree::Name(n) => {
                let mut full = prefix.clone();
                let id = n.ident.to_string();
                if id == "self" {
                    if let Some(last) = full.last().cloned() {
                        uses.insert(last, absolutize(&full, module));
                    }
                    return;
                }
                full.push(id.clone());
                uses.insert(id, absolutize(&full, module));
            }
            syn::UseTree::Rename(r) => {
                let mut full = prefix.clone();
                full.push(r.ident.to_string());
                uses.insert(r.rename.to_string(), absolutize(&full, module));
            }
            syn::UseTree::Group(g) => {
                for t in &g.items {
                    walk(t, prefix, module, uses);
                }
            }
            syn::UseTree::Glob(_) => {}
        }
    }
    for it in items {
        if let syn::Item::Use(u) = it {
            walk(&u.tree, &mut Vec::new(), module, uses);
        }
    }
}

fn absolutize(segs: &[String], module: &[String]) -> String {
    let krate = module.first().cloned().unwrap_or_default();
    let mut out: Vec<String> = Vec::new();
    let mut it = segs.iter().peekable();
    match it.peek().map(|s| s.as_str()) {
        Some("crate") => {
            out.push(krate);
            it.next();
        }
        Some("self") => {
            out.extend(module.iter().cloned());
            it.next();
        }
        Some("super") => {
            out.extend(module.iter().cloned());
            while it.peek().map(|s| s.as_str()) == Some("super") {
                out.pop();
                it.next();
            }
        }
        _ => {}
    }
    out.extend(it.cloned());
    out.join("::")
}

impl Ctx<'_> {
    fn modpath(&self) -> String {
        self.module.join("::")
    }

    fn items(&mut self, items: &[syn::Item]) {
        let saved = self.uses.clone();
        collect_uses(items, &self.module, &mut self.uses);
        for it in items {
            self.item(it);
        }
        self.uses = saved;
    }

    fn item(&mut self, it: &syn::Item) {
        match it {
            syn::Item::Mod(m) => {
                if is_cfg_test(&m.attrs) {
                    return;
                }
                if let Some((_, items)) = &m.content {
                    self.module.push(m.ident.to_string());
                    self.items(items);
                    self.module.pop();
                }
            }
            syn::Item::Trait(t) => {
                if !matches!(t.vis, syn::Visibility::Public(_)) || is_unstable(&t.attrs) {
                    return;
                }
                BOUNDED_ASSOC.with(|b| {
                    let mut b = b.borrow_mut();
                    b.clear();
                    for ti in &t.items {
                        if let syn::TraitItem::Type(ty) = ti {
                            if ty.bounds.iter().any(|x| matches!(x, syn::TypeParamBound::Trait(tb)
                                if !matches!(tb.modifier, syn::TraitBoundModifier::Maybe(_)) && !tb.path.is_ident("Sized")))
                            {
                                b.insert(ty.ident.to_string());
                            }
                        }
                    }
                });
                let mut d = TraitDecl {
                    path: format!("{}::{}", self.modpath(), t.ident),
                    uses: self.uses.clone(),
                    module: self.modpath(),
                    ..Default::default()
                };
                for b in &t.supertraits {
                    if let syn::TypeParamBound::Trait(tb) = b {
                        if !matches!(tb.modifier, syn::TraitBoundModifier::Maybe(_)) {
                            d.supers.push(path_string(&tb.path));
                        }
                    }
                }
                for ti in &t.items {
                    if let syn::TraitItem::Fn(f) = ti {
                        if is_unstable(&f.attrs) {
                            continue;
                        }
                        let n = f.sig.ident.to_string();
                        if f.default.is_some() {
                            d.prov.insert(n.clone());
                            let by_value = matches!(f.sig.inputs.first(), Some(syn::FnArg::Receiver(r)) if r.reference.is_none());
                            if deferred_in(&f.sig.output, "Self")
                                && (by_value || matches!(&f.sig.output, syn::ReturnType::Type(_, t) if tokens(t) == "& mut Self"))
                            {
                                d.prov_deferred.insert(n);
                            }
                        } else {
                            d.req.insert(n);
                        }
                    }
                }
                // Each member is also an `F` row on `trait:<path>` so its slots (`Self::Item: PartialOrd`, an
                // `IntoIterator` argument) are read like an inherent method's.
                let tparams: BTreeSet<String> = t
                    .generics
                    .params
                    .iter()
                    .filter_map(|g| match g {
                        syn::GenericParam::Type(tp) => Some(tp.ident.to_string()),
                        _ => None,
                    })
                    .collect();
                for ti in &t.items {
                    if let syn::TraitItem::Fn(f) = ti {
                        if is_unstable(&f.attrs) {
                            continue;
                        }
                        self.fn_decl(&format!("trait:{}", d.path), &f.sig, &[&t.generics], &tparams, None, true);
                    }
                }
                // `#[rustc_must_implement_one_of(read_buf, read)]`: an implementor defines one of them — on stable, the
                // stable one — so it is REQUIRED for every implementor a user can write.
                for a in &t.attrs {
                    if a.path().is_ident("rustc_must_implement_one_of") {
                        let names: Vec<String> = tokens(&a.meta)
                            .split(|c: char| !(c.is_alphanumeric() || c == '_'))
                            .filter(|w| !w.is_empty() && *w != "rustc_must_implement_one_of")
                            .map(String::from)
                            .collect();
                        for n in names {
                            if d.prov.remove(&n) {
                                d.prov_deferred.remove(&n);
                                d.req.insert(n);
                            }
                        }
                    }
                }
                self.out.traits.push(d);
                BOUNDED_ASSOC.with(|b| b.borrow_mut().clear());
            }
            syn::Item::Impl(im) => {
                if is_unstable(&im.attrs) || is_cfg_test(&im.attrs) {
                    return;
                }
                let params: BTreeSet<String> = im
                    .generics
                    .params
                    .iter()
                    .filter_map(|g| match g {
                        syn::GenericParam::Type(tp) => Some(tp.ident.to_string()),
                        _ => None,
                    })
                    .collect();
                let Some((shape, args)) = shape_of(&im.self_ty, &params) else { return };
                let mut bounds = BTreeMap::new();
                collect_bounds(&[&im.generics], &mut bounds);
                if let Some((neg, tp, _)) = &im.trait_ {
                    if neg.is_some() {
                        return;
                    }
                    let mut slots = Vec::new();
                    for (i, a) in args.iter().enumerate() {
                        if let Some(p) = as_param(a, &params) {
                            let bs = bounds.get(&p).cloned().unwrap_or_default();
                            if !bs.is_empty() {
                                slots.push((if shape == "*" { "s".to_string() } else { format!("e{i}") }, bs));
                            }
                        }
                    }
                    self.out.impls.push(ImplDecl {
                        tr: path_string(tp),
                        shape: shape.clone(),
                        slots,
                        uses: self.uses.clone(),
                        module: self.modpath(),
                    });
                    return;
                }
                // An inherent impl: each `pub` method is an `F` row on the shape.
                for ii in &im.items {
                    if let syn::ImplItem::Fn(f) = ii {
                        if !matches!(f.vis, syn::Visibility::Public(_)) || is_unstable(&f.attrs) {
                            continue;
                        }
                        let recv_args: Vec<Option<String>> = args.iter().map(|a| as_param(a, &params)).collect();
                        self.fn_decl(&shape, &f.sig, &[&im.generics], &params, Some(&recv_args), false);
                    }
                }
            }
            // `#[derive(Clone, Debug, PartialEq, ..)] pub enum Option<T>`: a derive implements the trait with every type
            // parameter bounded by it — that IS the derive's declaration, read here like a written impl.
            syn::Item::Struct(_) | syn::Item::Enum(_) => {
                let (attrs, vis, ident, generics) = match it {
                    syn::Item::Struct(x) => (&x.attrs, &x.vis, &x.ident, &x.generics),
                    syn::Item::Enum(x) => (&x.attrs, &x.vis, &x.ident, &x.generics),
                    _ => unreachable!(),
                };
                if !matches!(vis, syn::Visibility::Public(_)) || is_unstable(attrs) || is_cfg_test(attrs) {
                    return;
                }
                let params: Vec<String> = generics
                    .params
                    .iter()
                    .filter_map(|g| match g {
                        syn::GenericParam::Type(tp) => Some(tp.ident.to_string()),
                        _ => None,
                    })
                    .collect();
                for a in attrs {
                    if !a.path().is_ident("derive") {
                        continue;
                    }
                    let Ok(list) = a.parse_args_with(syn::punctuated::Punctuated::<syn::Path, syn::Token![,]>::parse_terminated)
                    else {
                        continue;
                    };
                    for tr in list {
                        let trs = path_string(&tr);
                        let slots: Vec<(String, Vec<String>)> =
                            params.iter().enumerate().map(|(i, _)| (format!("e{i}"), vec![trs.clone()])).collect();
                        self.out.impls.push(ImplDecl {
                            tr: trs.clone(),
                            shape: ident.to_string(),
                            slots,
                            uses: self.uses.clone(),
                            module: self.modpath(),
                        });
                    }
                }
            }
            syn::Item::Fn(f) => {
                if !matches!(f.vis, syn::Visibility::Public(_)) || is_unstable(&f.attrs) || is_cfg_test(&f.attrs) {
                    return;
                }
                let owner = format!("fn:{}", self.modpath());
                self.fn_decl(&owner, &f.sig, &[], &BTreeSet::new(), None, false);
            }
            syn::Item::Macro(m) => {
                // An item-position invocation of a `macro_rules!` this crate defines: expanded by macro-by-example
                // (`mbe::expand_invocation`, the engine's own expander) and its items read like any other. A
                // definition that is not unique, or an invocation the expander refuses, is a listed miss.
                if m.mac.path.is_ident("macro_rules") {
                    return;
                }
                let Some(name) = m.mac.path.segments.last().map(|s| s.ident.to_string()) else { return };
                let Some(def) = self.file_macros.get(&name).cloned().or_else(|| self.macros.get(&name).cloned()) else { return };
                let Some(def) = def else {
                    self.out.unexpanded.insert(format!("{name} (defined twice)"));
                    return;
                };
                if self.depth > 40 {
                    self.out.unexpanded.insert(format!("{name} (depth)"));
                    return;
                }
                let expanded = crate::mbe::expand_invocation(&def, &m.mac.tokens)
                    .or_else(|| crate::mbe::expand_invocation(&collapse_varless_reps(&def), &m.mac.tokens));
                match expanded {
                    Some(ts) => match syn::parse_file(&rewrite_nightly(&ts.to_string())) {
                        Ok(f) => {
                            self.depth += 1;
                            for it in &f.items {
                                self.item(it);
                            }
                            self.depth -= 1;
                        }
                        Err(_) => {
                            self.out.unexpanded.insert(format!("{name} (expansion does not parse as items)"));
                        }
                    },
                    None => {
                        self.out.unexpanded.insert(format!("{name} (expander refused)"));
                    }
                }
            }
            _ => {}
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn fn_decl(
        &mut self,
        owner: &str,
        sig: &syn::Signature,
        outer: &[&syn::Generics],
        outer_params: &BTreeSet<String>,
        recv_args: Option<&[Option<String>]>,
        is_trait: bool,
    ) {
        let mut gens: Vec<&syn::Generics> = outer.to_vec();
        gens.push(&sig.generics);
        let mut bounds: BTreeMap<String, Vec<String>> = BTreeMap::new();
        collect_bounds(&gens, &mut bounds);
        let mut params: BTreeSet<String> = outer_params.clone();
        for g in &sig.generics.params {
            if let syn::GenericParam::Type(t) = g {
                params.insert(t.ident.to_string());
            }
        }
        let has_self = matches!(sig.inputs.first(), Some(syn::FnArg::Receiver(_)));
        let mut slots: Vec<(String, Vec<String>, bool)> = Vec::new();
        let mut placed: BTreeSet<String> = BTreeSet::new();
        // the receiver's own type arguments
        if let Some(ra) = recv_args {
            for (i, p) in ra.iter().enumerate() {
                if let Some(p) = p {
                    if let Some(bs) = bounds.get(p).filter(|b| !b.is_empty()) {
                        slots.push((format!("e{i}"), bs.clone(), deferred_in(&sig.output, p)));
                        placed.insert(p.clone());
                    }
                }
            }
        }
        // the trait's own Self, and Self::Item
        if is_trait && has_self {
            let mut bs = vec![owner.trim_start_matches("trait:").to_string()];
            bs.extend(bounds.get("Self").cloned().unwrap_or_default().into_iter().filter(|b| b != "Sized"));
            // DEFERRED only when the receiver can be MOVED into the result: a by-value `self` (`map`, `into_iter`), or
            // `&mut self` handed straight back (`by_ref`). A `&self` member (`to_owned`, `clone`) computes a new value now.
            let by_value = matches!(sig.inputs.first(), Some(syn::FnArg::Receiver(r)) if r.reference.is_none());
            let d = deferred_in(&sig.output, "Self")
                && (by_value || matches!(&sig.output, syn::ReturnType::Type(_, t) if tokens(t) == "& mut Self"));
            slots.push(("s".into(), bs, d));
        }
        if let Some(bs) = bounds.get("Self::Item").filter(|b| !b.is_empty()) {
            slots.push(("i".into(), bs.clone(), false));
        }
        // a parameter that IS the receiver's item (`fn cloned<'a, T>(self) where T: Clone, Self: Iterator<Item = &'a T>`)
        for p in item_params(&gens) {
            if let Some(bs) = bounds.get(&p).filter(|b| !b.is_empty()) {
                slots.push(("i".into(), bs.clone(), false));
                placed.insert(p);
            }
        }
        // the non-receiver arguments
        let mut k = 0usize;
        let mut nargs = 0usize;
        for inp in &sig.inputs {
            let syn::FnArg::Typed(pt) = inp else { continue };
            nargs += 1;
            let ty = &*pt.ty;
            let peeled = match ty {
                syn::Type::Reference(r) => &*r.elem,
                t => t,
            };
            if let syn::Type::ImplTrait(it) = peeled {
                let mut bs = Vec::new();
                for b in &it.bounds {
                    if let syn::TypeParamBound::Trait(tb) = b {
                        bs.push(path_string(&tb.path));
                    }
                }
                if !bs.is_empty() {
                    slots.push((format!("a{k}"), bs, false));
                }
            } else if let Some(p) = as_param(ty, &params) {
                if let Some(bs) = bounds.get(&p).filter(|b| !b.is_empty()) {
                    slots.push((format!("a{k}"), bs.clone(), deferred_in(&sig.output, &p)));
                    placed.insert(p.clone());
                }
            }
            k += 1;
        }
        // a bounded parameter that reaches the call only as the return type (`parse::<F>`, `collect::<B>`)
        if let syn::ReturnType::Type(_, rt) = &sig.output {
            // `-> F`, `-> Option<F>`, `-> Result<F, F::Err>` (`str::parse`)
            let direct = as_param(rt, &params).or_else(|| match &**rt {
                syn::Type::Path(p) if p.qself.is_none() => {
                    let seg = p.path.segments.last()?;
                    if !matches!(seg.ident.to_string().as_str(), "Option" | "Result") {
                        return None;
                    }
                    let syn::PathArguments::AngleBracketed(ab) = &seg.arguments else { return None };
                    match ab.args.first()? {
                        syn::GenericArgument::Type(t) => as_param(t, &params),
                        _ => None,
                    }
                }
                _ => None,
            });
            if let Some(p) = direct {
                if !placed.contains(&p) && !outer_params.contains(&p) {
                    if let Some(bs) = bounds.get(&p).filter(|b| !b.is_empty()) {
                        slots.push(("r".into(), bs.clone(), false));
                    }
                }
            }
        }
        if slots.is_empty() {
            return;
        }
        self.out.fns.push(FnDecl {
            owner: owner.to_string(),
            name: sig.ident.to_string(),
            nargs,
            slots,
            uses: self.uses.clone(),
            module: self.modpath(),
        });
    }
}

/// A repetition `$( .. ) sep? op` with no metavariable inside (what `$(${ignore($T)} T,)+` leaves once the
/// `ignore` is blanked) is not macro-by-example; it becomes its contents, once.
fn collapse_varless_reps(d: &str) -> String {
    let b: Vec<char> = d.chars().collect();
    let mut out = String::new();
    let mut i = 0;
    while i < b.len() {
        if b[i] == '$' {
            let mut j = i + 1;
            while j < b.len() && b[j] == ' ' {
                j += 1;
            }
            if j < b.len() && b[j] == '(' {
                let mut depth = 0i32;
                let mut k = j;
                while k < b.len() {
                    match b[k] {
                        '(' | '[' | '{' => depth += 1,
                        ')' | ']' | '}' => {
                            depth -= 1;
                            if depth == 0 {
                                break;
                            }
                        }
                        _ => {}
                    }
                    k += 1;
                }
                let inner: String = b[j + 1..k.min(b.len())].iter().collect();
                if !inner.contains('$') {
                    out.push_str(&collapse_varless_reps(&inner));
                    // skip an optional separator and the repetition operator
                    let mut m = k + 1;
                    let rest: String = b[m.min(b.len())..].iter().collect();
                    let t = rest.trim_start();
                    m += rest.len() - t.len();
                    for pre in ["&& +", "&&+", "|| +", "||+", ", +", ",+", ", *", ",*", "+", "*", "?"] {
                        if t.starts_with(pre) {
                            m += pre.len();
                            break;
                        }
                    }
                    i = m;
                    continue;
                }
            }
        }
        out.push(b[i]);
        i += 1;
    }
    out
}

/// Every `fn` BODY in a token stream replaced by `{}` — the table reads signatures only, and bodies are where std's
/// macros use the forms the expander does not read (`${index()}`, `&&`-separated repetitions without a variable).
fn blank_fn_bodies(ts: proc_macro2::TokenStream) -> proc_macro2::TokenStream {
    use proc_macro2::{Delimiter, Group, TokenTree};
    let mut out: Vec<TokenTree> = Vec::new();
    let mut in_sig = false;
    for tt in ts {
        match tt {
            TokenTree::Ident(ref i) if i == "fn" => {
                in_sig = true;
                out.push(tt);
            }
            TokenTree::Group(g) if in_sig && g.delimiter() == Delimiter::Brace => {
                in_sig = false;
                let mut ng = Group::new(Delimiter::Brace, proc_macro2::TokenStream::new());
                ng.set_span(g.span());
                out.push(TokenTree::Group(ng));
            }
            TokenTree::Punct(ref p) if in_sig && p.as_char() == ';' => {
                in_sig = false;
                out.push(tt);
            }
            TokenTree::Group(g) if !in_sig => {
                let mut ng = Group::new(g.delimiter(), blank_fn_bodies(g.stream()));
                ng.set_span(g.span());
                out.push(TokenTree::Group(ng));
            }
            other => out.push(other),
        }
    }
    out.into_iter().collect()
}

/// `$ { ignore ($T) }` → nothing, `$ { index () }` / `$ { count (..) }` / `$ { len (..) }` → `0`, in a definition's token text.
fn strip_meta_exprs(d: &str) -> String {
    let b: Vec<char> = d.chars().collect();
    let mut out = String::new();
    let mut i = 0;
    while i < b.len() {
        if b[i] == '$' {
            let mut j = i + 1;
            while j < b.len() && b[j] == ' ' {
                j += 1;
            }
            if j < b.len() && b[j] == '{' {
                let mut depth = 0i32;
                let mut k = j;
                while k < b.len() {
                    match b[k] {
                        '{' => depth += 1,
                        '}' => {
                            depth -= 1;
                            if depth == 0 {
                                break;
                            }
                        }
                        _ => {}
                    }
                    k += 1;
                }
                let inner: String = b[j + 1..k.min(b.len())].iter().collect();
                if !inner.trim_start().starts_with("ignore") {
                    out.push('0');
                }
                i = k + 1;
                continue;
            }
        }
        out.push(b[i]);
        i += 1;
    }
    out
}

fn module_of(lib: &Path, f: &Path) -> Vec<String> {
    let rel = f.strip_prefix(lib).unwrap();
    let parts: Vec<String> = rel.iter().map(|c| c.to_string_lossy().into_owned()).collect();
    // <crate>/src/a/b.rs
    let krate = parts[0].clone();
    let mut m = vec![krate];
    for p in &parts[2..] {
        let p = p.trim_end_matches(".rs");
        if p == "mod" || p == "lib" {
            continue;
        }
        m.push(p.to_string());
    }
    m
}

fn rewrite_nightly(s: &str) -> String {
    s.replace("impl(self) ", "")
        .replace("impl(crate) ", "")
        .replace("impl(super) ", "")
        .replace("final fn ", "fn ")
        .replace("*const impl Thin", "*const ()")
        .replace("const impl", "impl")
        .replace("impl const ", "impl ")
        .replace("const unsafe impl", "unsafe impl")
        .replace("[const] ", "")
        .replace("~const ", "")
        .replace("const trait ", "trait ")
        .replace("const unsafe trait ", "unsafe trait ")
}

/// The table's text for the rust-src `library/` directory `lib`, stamped with `rustc` (the `rustc -V` line of the
/// toolchain that rust-src belongs to). Deterministic: every row set is sorted.
pub(crate) fn generate(lib: &Path, rustc: &str) -> String {
    let lib = lib.to_path_buf();
    let mut out = Out::default();
    let mut sealed: BTreeSet<String> = BTreeSet::new();
    let mut parsed: Vec<(Vec<String>, syn::File, PathBuf)> = Vec::new();
    let mut files: Vec<PathBuf> = Vec::new();
    for krate in ["core", "alloc", "std"] {
        let root = lib.join(krate).join("src");
        walk(&root, &mut files);
    }
    files.sort();
    for f in &files {
        let src = std::fs::read_to_string(f).unwrap_or_default();
        let module = module_of(&lib, f);
        // `pub impl(self) trait X` / `impl(crate)`: a SEALED trait (impl restriction) — no user type can implement
        // it, so a bound on it licenses nothing user-side. Read before the restriction is rewritten away.
        for (i, _) in src.match_indices("impl(") {
            let rest = &src[i..];
            if !(rest.starts_with("impl(self)") || rest.starts_with("impl(crate)") || rest.starts_with("impl(super)")) {
                continue;
            }
            let after = &rest[rest.find(')').unwrap() + 1..];
            let toks: Vec<&str> = after.split_whitespace().take(4).collect();
            if let Some(k) = toks.iter().position(|t| *t == "trait") {
                if let Some(name) = toks.get(k + 1) {
                    let name: String = name.chars().take_while(|c| c.is_alphanumeric() || *c == '_').collect();
                    sealed.insert(format!("{}::{name}", module.join("::")));
                }
            }
        }
        let src = rewrite_nightly(&src);
        match syn::parse_file(&src) {
            Ok(file) => parsed.push((module, file, f.parent().unwrap().to_path_buf())),
            Err(e) => {
                let rel = f.strip_prefix(&lib).unwrap().display().to_string();
                out.unparsed.push(format!("{rel}:{} {e}", e.span().start().line));
            }
        }
    }
    // Every `macro_rules!` definition by name, across core/alloc/std: a name defined twice differently is refused.
    // A definition in the invoking FILE wins (two files may each define a private helper of one name); otherwise the
    // crate-wide one. `${ignore(..)}` / `${index()}` metavariable expressions (the tuple impls' bodies) are not
    // macro-by-example the expander reads; they only ever occur in fn BODIES, which the table never reads, so they
    // are blanked before expansion.
    let meta_expr = |d: String| {
        let blanked = d.parse::<proc_macro2::TokenStream>().map(|t| blank_fn_bodies(t).to_string()).unwrap_or(d);
        strip_meta_exprs(&blanked)
    };
    let per_file: Vec<Vec<(String, Option<String>)>> = parsed
        .iter()
        .map(|(_, f, _)| crate::mbe::file_macro_defs(&f.items).into_iter().map(|(k, v)| (k, v.map(meta_expr))).collect())
        .collect();
    let macros = crate::mbe::merge_crate_macros(per_file.clone());
    // …and, between the two, the definitions of the files in the SAME directory (`#[macro_use] mod macros;` beside
    // the module that invokes them — vec_deque's `__impl_slice_eq1`), when they agree.
    let mut by_dir: HashMap<PathBuf, HashMap<String, Option<String>>> = HashMap::new();
    for ((_, _, dir), defs) in parsed.iter().zip(per_file.iter()) {
        let e = by_dir.entry(dir.clone()).or_default();
        for (k, v) in defs {
            match e.get(k) {
                None => {
                    e.insert(k.clone(), v.clone());
                }
                Some(p) if p == v => {}
                Some(_) => {
                    e.insert(k.clone(), None);
                }
            }
        }
    }
    for ((module, file, dir), defs) in parsed.iter().zip(per_file) {
        let mut local: HashMap<String, Option<String>> = by_dir.get(dir).cloned().unwrap_or_default();
        local.extend(defs);
        let mut c = Ctx {
            out: &mut out,
            module: module.clone(),
            uses: HashMap::new(),
            macros: &macros,
            file_macros: local,
            depth: 0,
        };
        c.items(&file.items);
    }
    // primitive_docs.rs — the impls rustc generates for tuples, which have no source.
    let pd = std::fs::read_to_string(lib.join("core/src/primitive_docs.rs")).unwrap_or_default();
    let builtin_tuple = builtin_tuple_traits(&pd);

    // ---- resolve written trait paths to declared pub trait paths
    let mut by_leaf: HashMap<String, Vec<String>> = HashMap::new();
    for t in &out.traits {
        let leaf = t.path.rsplit("::").next().unwrap().to_string();
        let v = by_leaf.entry(leaf).or_default();
        if !v.contains(&t.path) {
            v.push(t.path.clone());
        }
    }
    let resolve = |written: &str, uses: &HashMap<String, String>, module: &str| -> Option<String> {
        let segs: Vec<&str> = written.split("::").collect();
        let leaf = *segs.last()?;
        let cands = by_leaf.get(leaf)?;
        if cands.len() == 1 {
            return Some(cands[0].clone());
        }
        // through the file's `use` of the first segment
        let full = match uses.get(segs[0]) {
            Some(u) => {
                let mut f = u.clone();
                for s in &segs[1..] {
                    f.push_str("::");
                    f.push_str(s);
                }
                f
            }
            None if segs.len() == 1 => format!("{module}::{leaf}"),
            None => written.to_string(),
        };
        let norm = |p: &str| p.replacen("std::", "", 1).replacen("core::", "", 1).replacen("alloc::", "", 1);
        let nf = norm(&full);
        let hit: Vec<&String> = cands.iter().filter(|c| norm(c) == nf || nf.ends_with(&norm(c)) || norm(c).ends_with(&nf)).collect();
        if hit.len() == 1 {
            return Some(hit[0].clone());
        }
        // the qualifier's last module segment (`fmt::Write`, `io::Write`)
        if segs.len() >= 2 {
            let q = segs[segs.len() - 2];
            let hit: Vec<&String> = cands.iter().filter(|c| c.split("::").any(|s| s == q)).collect();
            if hit.len() == 1 {
                return Some(hit[0].clone());
            }
        }
        // same top-level module (`std::io::*` writes `Write` for `io::Write`)
        let m2: Vec<&str> = module.split("::").take(2).collect();
        let hit: Vec<&String> = cands
            .iter()
            .filter(|c| {
                let c2: Vec<&str> = c.split("::").collect();
                c2.len() > 1 && m2.len() > 1 && c2[1] == m2[1]
            })
            .collect();
        if hit.len() == 1 {
            return Some(hit[0].clone());
        }
        None
    };
    let mut lines: BTreeSet<String> = BTreeSet::new();
    let mut unresolved: BTreeSet<String> = BTreeSet::new();
    // canonical trait identity: a trait declared once under core and re-exported is ONE row; if a leaf+path pair is
    // declared twice (cfg twins), the rows merge.
    let mut tmerged: BTreeMap<String, TraitDecl> = BTreeMap::new();
    for t in &out.traits {
        let e = tmerged.entry(t.path.clone()).or_insert_with(|| TraitDecl { path: t.path.clone(), ..Default::default() });
        for s in &t.supers {
            if let Some(r) = resolve(s, &t.uses, &t.module) {
                if r != t.path && !e.supers.contains(&r) {
                    e.supers.push(r);
                }
            } else if !matches!(s.as_str(), "Sized") {
                unresolved.insert(s.clone());
            }
        }
        e.req.extend(t.req.iter().cloned());
        e.prov.extend(t.prov.iter().cloned());
        e.prov_deferred.extend(t.prov_deferred.iter().cloned());
    }
    for t in tmerged.values() {
        let mut sup = t.supers.clone();
        sup.sort();
        let prov_run: Vec<&String> = t.prov.iter().filter(|m| !t.prov_deferred.contains(*m)).collect();
        lines.insert(format!(
            "T\t{}\t{}\t{}\t{}\t{}\t{}",
            t.path,
            if sealed.contains(&t.path) { "sealed" } else { "open" },
            sup.join(","),
            t.req.iter().cloned().collect::<Vec<_>>().join(","),
            prov_run.iter().map(|s| s.as_str()).collect::<Vec<_>>().join(","),
            t.prov_deferred.iter().cloned().collect::<Vec<_>>().join(",")
        ));
    }
    let fmt_slots = |slots: &[(String, Vec<String>)], uses: &HashMap<String, String>, module: &str, unresolved: &mut BTreeSet<String>| -> Vec<String> {
        let mut v = Vec::new();
        for (s, bs) in slots {
            let mut rs: Vec<String> = bs
                .iter()
                .filter_map(|b| {
                    let r = resolve(b, uses, module);
                    if r.is_none() && !matches!(b.as_str(), "Sized" | "Send" | "Sync" | "Unpin") {
                        unresolved.insert(b.clone());
                    }
                    r
                })
                .collect();
            rs.sort();
            rs.dedup();
            if !rs.is_empty() {
                v.push(format!("{s}={}", rs.join("+")));
            }
        }
        v
    };
    for im in &out.impls {
        let Some(tr) = resolve(&im.tr, &im.uses, &im.module) else { continue };
        let mut slots = fmt_slots(&im.slots, &im.uses, &im.module, &mut unresolved);
        // A tuple's impls are one per arity (1..=12) with every element bounded alike: one `e*` row.
        if im.shape == "()" && !slots.is_empty() {
            let bs: BTreeSet<&str> = slots.iter().filter_map(|x| x.split_once('=').map(|(_, b)| b)).collect();
            if bs.len() == 1 && slots.len() == im.slots.len() {
                slots = vec![format!("e*={}", bs.iter().next().unwrap())];
            }
        }
        lines.insert(format!("I\t{tr}\t{}\t{}", im.shape, slots.join(";")));
    }
    for t in &builtin_tuple {
        if let Some(tr) = resolve(t, &HashMap::new(), "core") {
            lines.insert(format!("I\t{tr}\t()\te*={tr}"));
        }
    }
    for f in &out.fns {
        let mut v = Vec::new();
        for (s, bs, d) in &f.slots {
            let one = fmt_slots(&[(s.clone(), bs.clone())], &f.uses, &f.module, &mut unresolved);
            for x in one {
                v.push(if *d { format!("{x}:d") } else { x });
            }
        }
        if v.is_empty() {
            continue;
        }
        let owner = if let Some(rest) = f.owner.strip_prefix("trait:") {
            match resolve(rest, &f.uses, &f.module) {
                Some(r) => format!("trait:{r}"),
                None => f.owner.clone(),
            }
        } else {
            f.owner.clone()
        };
        lines.insert(format!("F\t{owner}\t{}\t{}\t{}", f.name, f.nargs, v.join(";")));
    }
    let mut o = String::new();
    o.push_str(&format!("# GENERATED by crates/candor-scan/src/std_bounds_gen.rs from the rust-src of {rustc} — do not edit by hand.\n"));
    o.push_str(&format!("# Regenerate / verify: bash crates/candor-scan/gen/std_bounds.sh [--check]. Rows: T trait (open|sealed, supers, required, provided, provided-deferred);\n"));
    o.push_str(&format!("# I impl (trait, implementing shape, type-argument slot=bounds); F fn (owner, name, non-receiver args, slot=bounds[:d]).\n"));
    for l in &lines {
        o.push_str(l);
        o.push('\n');
    }
    for u in &out.unparsed {
        o.push_str(&format!("# unparsed\t{u}\n"));
    }
    for u in &out.unexpanded {
        o.push_str(&format!("# unexpanded\t{u}\n"));
    }
    eprintln!("traits {} impls {} fns {} lines {} unparsed {} unresolved-bounds {}", tmerged.len(), out.impls.len(), out.fns.len(), lines.len(), out.unparsed.len(), unresolved.len());
    if std::env::var_os("GEN_STD_BOUNDS_VERBOSE").is_some() {
        for u in &unresolved {
            eprintln!("unresolved\t{u}");
        }
    }
    o
}

/// The traits `primitive_docs.rs` lists as "implemented for tuples of any length … automatically generated by the
/// compiler" — the bullet list under that sentence.
fn builtin_tuple_traits(pd: &str) -> Vec<String> {
    let mut out = Vec::new();
    let Some(at) = pd.find("are implemented for tuples of any length") else { return out };
    for line in pd[at..].lines().skip(1) {
        let l = line.trim_start_matches("///").trim();
        if l.is_empty() || l.starts_with("These traits") || l.starts_with("implementations") || l.starts_with("missing") {
            continue;
        }
        if let Some(rest) = l.strip_prefix("* [`") {
            if let Some(name) = rest.split('`').next() {
                out.push(name.to_string());
            }
            continue;
        }
        break;
    }
    out
}

fn walk(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(rd) = std::fs::read_dir(dir) else { return };
    for e in rd.flatten() {
        let p = e.path();
        let name = p.file_name().unwrap().to_string_lossy().into_owned();
        if p.is_dir() {
            if matches!(name.as_str(), "tests" | "benches" | "sys" | "os" | "backtrace") {
                continue;
            }
            walk(&p, out);
        } else if name.ends_with(".rs") && name != "tests.rs" && name != "benches.rs" {
            out.push(p);
        }
    }
}

/// `STD_BOUNDS_SRC=<rust-src library dir> STD_BOUNDS_RUSTC="<rustc -V>" STD_BOUNDS_OUT=<file>` — writes the table;
/// with `STD_BOUNDS_CHECK=1` compares instead and fails on any difference. Driven by `gen/std_bounds.sh`.
#[test]
#[ignore]
fn regenerate() {
    let lib = std::env::var("STD_BOUNDS_SRC").expect("STD_BOUNDS_SRC");
    let rustc = std::env::var("STD_BOUNDS_RUSTC").expect("STD_BOUNDS_RUSTC");
    let out = std::env::var("STD_BOUNDS_OUT").expect("STD_BOUNDS_OUT");
    let text = generate(Path::new(&lib), &rustc);
    if std::env::var_os("STD_BOUNDS_CHECK").is_some() {
        let have = std::fs::read_to_string(&out).unwrap_or_default();
        assert!(have == text, "{out} does not reproduce from {lib} ({rustc}); regenerate with gen/std_bounds.sh");
    } else {
        std::fs::write(&out, text).expect("write table");
    }
}

#[test]
#[ignore]
fn dbg_tuple_impls() {
    for (d, inp) in [
        ("(@impl $($T:ident)+) => { impl<$($T: PartialEq),+> PartialEq for ($($T,)+) {} }", "@impl E D"),
        ("(@impl $($T:ident)+) => { struct X; }", "@impl E D"),
        ("(@ impl $($T:ident)+) => { struct X; }", "@impl E D"),
        ("(@x $($T:ident)+) => { struct X; }", "@x E D"),
        ("(x $($T:ident)+) => { struct X; }", "x E D"),
        ("(x $($T:ident)+) => { impl<$($T: PartialEq),+> PartialEq for ($($T,)+) {} }", "x E D"),
    ] {
        let r = crate::mbe::expand_invocation(d, &inp.parse().unwrap());
        eprintln!("IN {d} :: {inp} => {:?}", r.map(|t| t.to_string()));
    }
}
