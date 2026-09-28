//! PASS A — the crate's DROP-RELEVANT set, built to the same definition the engine uses.
//!
//! This is the half of the conjunction SOUNDNESS R766 says the old probe never counted. The engine's
//! `note_construction` gate (`crates/candor-scan/src/collector.rs:694`) fires only for a leaf in
//! `drop_relevant`, and that set is assembled at `crates/candor-scan/src/scan.rs:1596` as exactly:
//!
//!     drop_relevant = merged.drop_types  ∪  owned_drops.keys()
//!
//! `drop_types` (`decls.rs:3339`) is every LOCAL type leaf carrying a local `impl Drop for T`.
//! `owned_drops` (`scan.rs:1505-1589`) is the monotone fixpoint of "T owns a drop-type through a
//! non-borrowed field, directly or through a container element or transitively" — with R718's rule
//! that a BORROWED field (`&G`, `&mut G`, `Vec<&G>`) is not an owned one.
//!
//! If a shape's leaf is outside this set, NO fix to the drop/escape model can make a row follow from
//! it: the construction is never marked in the first place. That is why the tight counts here are the
//! only ones that can order work.

use std::collections::{BTreeSet, HashMap, HashSet};

/// One declared field, reduced to the leaves it OWNS.
#[derive(Debug, Clone)]
pub struct FieldOwn {
    pub name: Option<String>,
    pub leaves: Vec<String>,
    /// Every leaf mentioned, references included. `owned_drops` must NOT use this (R718); the R297
    /// PLACE matcher must, because `*slot = v` with `slot: &mut Guard` drops a `Guard` in this frame.
    pub all_leaves: Vec<String>,
    /// R718: the field is reached through a reference, so its drop runs in whoever owns the referent.
    pub borrowed: bool,
}

#[derive(Debug, Default)]
pub struct CrateIndex {
    /// `impl Drop for T` — leaf-keyed, local types only.
    pub drop_types: HashSet<String>,
    /// type leaf -> its fields' owned leaves.
    pub fields: HashMap<String, Vec<FieldOwn>>,
    /// free-fn name -> the leaves its return type mentions.
    pub fn_ret: HashMap<String, Vec<String>>,
    /// method name -> return leaves, kept ONLY where the name resolves uniquely crate-wide.
    /// A colliding method name is dropped rather than guessed (blind spot, stated in REACH.md).
    pub method_ret: HashMap<String, Option<Vec<String>>>,
    /// The answer: `drop_types ∪ owned_drops.keys()`.
    pub drop_relevant: HashSet<String>,
    /// Files parsed / files that failed to parse — an unparsed file is silence, so it is REPORTED.
    pub files_ok: usize,
    pub files_failed: usize,
}

/// The leaf of a path type: `ffi::Deflate` -> `Deflate`. Matches the engine's leaf keying.
pub fn path_leaf(p: &syn::Path) -> Option<String> {
    p.segments.last().map(|s| s.ident.to_string())
}

/// Collect the leaves a TYPE owns, and whether it is reached through a reference.
///
/// Peels containers by recursing into generic arguments, tuples, arrays and slices, which is what
/// makes `Vec<Guard>` and `Option<Guard>` own `Guard` the way `fields` + `field_elem` do in the
/// engine. A `&`/`&mut` anywhere on the way sets `borrowed`, which is R718's rule.
pub fn type_owned_leaves(ty: &syn::Type) -> FieldOwn {
    fn go(ty: &syn::Type, under_ref: bool, out: &mut Vec<String>, saw_ref: &mut bool) {
        match ty {
            syn::Type::Reference(r) => {
                *saw_ref = true;
                go(&r.elem, true, out, saw_ref);
            }
            syn::Type::Ptr(p) => go(&p.elem, true, out, saw_ref),
            syn::Type::Paren(p) => go(&p.elem, under_ref, out, saw_ref),
            syn::Type::Group(g) => go(&g.elem, under_ref, out, saw_ref),
            syn::Type::Tuple(t) => {
                for e in &t.elems {
                    go(e, under_ref, out, saw_ref)
                }
            }
            syn::Type::Array(a) => go(&a.elem, under_ref, out, saw_ref),
            syn::Type::Slice(s) => go(&s.elem, under_ref, out, saw_ref),
            syn::Type::Path(tp) => {
                if let Some(seg) = tp.path.segments.last() {
                    if !under_ref {
                        out.push(seg.ident.to_string());
                    }
                    if let syn::PathArguments::AngleBracketed(ab) = &seg.arguments {
                        for a in &ab.args {
                            if let syn::GenericArgument::Type(t) = a {
                                go(t, under_ref, out, saw_ref)
                            }
                        }
                    }
                }
            }
            _ => {}
        }
    }
    let mut out = Vec::new();
    let mut saw_ref = false;
    go(ty, false, &mut out, &mut saw_ref);
    let mut all = Vec::new();
    type_all_leaves(ty, &mut all);
    FieldOwn { name: None, leaves: out, all_leaves: all, borrowed: saw_ref }
}

/// Every leaf a type mentions, references NOT excluded. See `FieldOwn::all_leaves`.
pub fn type_all_leaves(ty: &syn::Type, out: &mut Vec<String>) {
    fn rec(ty: &syn::Type, out: &mut Vec<String>) {
        match ty {
            syn::Type::Reference(r) => rec(&r.elem, out),
            syn::Type::Ptr(p) => rec(&p.elem, out),
            syn::Type::Paren(p) => rec(&p.elem, out),
            syn::Type::Group(g) => rec(&g.elem, out),
            syn::Type::Tuple(t) => t.elems.iter().for_each(|e| rec(e, out)),
            syn::Type::Array(a) => rec(&a.elem, out),
            syn::Type::Slice(s) => rec(&s.elem, out),
            syn::Type::Path(tp) => {
                if let Some(seg) = tp.path.segments.last() {
                    out.push(seg.ident.to_string());
                    if let syn::PathArguments::AngleBracketed(ab) = &seg.arguments {
                        for a in &ab.args {
                            if let syn::GenericArgument::Type(t) = a {
                                rec(t, out)
                            }
                        }
                    }
                }
            }
            _ => {}
        }
    }
    rec(ty, out);
}

fn self_ty_leaf(ty: &syn::Type) -> Option<String> {
    match ty {
        syn::Type::Path(tp) => path_leaf(&tp.path),
        syn::Type::Paren(p) => self_ty_leaf(&p.elem),
        syn::Type::Group(g) => self_ty_leaf(&g.elem),
        _ => None,
    }
}

pub fn has_cfg_test(attrs: &[syn::Attribute]) -> bool {
    attrs.iter().any(|a| {
        a.path().is_ident("cfg") && {
            let s = quote::quote!(#a).to_string();
            s.contains("test")
        }
    })
}

fn ret_leaves(out: &syn::ReturnType) -> Vec<String> {
    match out {
        syn::ReturnType::Default => Vec::new(),
        syn::ReturnType::Type(_, t) => type_owned_leaves(t).leaves,
    }
}

impl CrateIndex {
    pub fn collect_items(&mut self, items: &[syn::Item]) {
        for it in items {
            match it {
                syn::Item::Mod(m) => {
                    if has_cfg_test(&m.attrs) {
                        continue;
                    }
                    if let Some((_, inner)) = &m.content {
                        self.collect_items(inner);
                    }
                }
                syn::Item::Struct(s) => {
                    let e = self.fields.entry(s.ident.to_string()).or_default();
                    for f in &s.fields {
                        let mut fo = type_owned_leaves(&f.ty);
                        fo.name = f.ident.as_ref().map(|i| i.to_string());
                        e.push(fo);
                    }
                }
                syn::Item::Enum(en) => {
                    let e = self.fields.entry(en.ident.to_string()).or_default();
                    for v in &en.variants {
                        for f in &v.fields {
                            let mut fo = type_owned_leaves(&f.ty);
                            fo.name = f.ident.as_ref().map(|i| i.to_string());
                            e.push(fo);
                        }
                    }
                }
                syn::Item::Union(u) => {
                    let e = self.fields.entry(u.ident.to_string()).or_default();
                    for f in &u.fields.named {
                        let mut fo = type_owned_leaves(&f.ty);
                        fo.name = f.ident.as_ref().map(|i| i.to_string());
                        e.push(fo);
                    }
                }
                syn::Item::Fn(f) => {
                    if has_cfg_test(&f.attrs) {
                        continue;
                    }
                    self.fn_ret.insert(f.sig.ident.to_string(), ret_leaves(&f.sig.output));
                }
                syn::Item::Impl(im) => {
                    // `impl Drop for T` — the whole of `drop_types`.
                    if let (Some((_, tr, _)), Some(ty)) = (&im.trait_, self_ty_leaf(&im.self_ty)) {
                        if tr.segments.last().map(|s| s.ident == "Drop").unwrap_or(false) {
                            self.drop_types.insert(ty);
                        }
                    }
                    let owner = self_ty_leaf(&im.self_ty);
                    for item in &im.items {
                        if let syn::ImplItem::Fn(m) = item {
                            let leaves = ret_leaves(&m.sig.output);
                            // Associated `T::ctor()` — recorded under `T::name` so a call can resolve
                            // it exactly rather than by a colliding bare method name.
                            if let Some(o) = &owner {
                                self.fn_ret.insert(format!("{o}::{}", m.sig.ident), leaves.clone());
                            }
                            // Bare method name — kept only while it resolves uniquely.
                            match self.method_ret.get(&m.sig.ident.to_string()) {
                                None => {
                                    self.method_ret.insert(m.sig.ident.to_string(), Some(leaves));
                                }
                                Some(Some(prev)) if *prev == leaves => {}
                                Some(_) => {
                                    self.method_ret.insert(m.sig.ident.to_string(), None);
                                }
                            }
                        }
                    }
                }
                _ => {}
            }
        }
    }

    /// `owned_drops` — the transitive field-ownership fixpoint, then the union that is `drop_relevant`.
    pub fn finish(&mut self) {
        self.drop_relevant = self.drop_types.clone();
        if self.drop_types.is_empty() {
            return;
        }
        let mut owned: HashMap<String, BTreeSet<String>> = HashMap::new();
        let candidates = |t: &str, fields: &HashMap<String, Vec<FieldOwn>>| -> Vec<String> {
            let mut v = Vec::new();
            if let Some(fs) = fields.get(t) {
                for f in fs {
                    if f.borrowed {
                        continue; // R718
                    }
                    v.extend(f.leaves.iter().cloned());
                }
            }
            v
        };
        let all: Vec<String> = self.fields.keys().cloned().collect();
        for t in &all {
            let s: BTreeSet<String> = candidates(t, &self.fields)
                .into_iter()
                .filter(|c| self.drop_types.contains(c))
                .collect();
            if !s.is_empty() {
                owned.insert(t.clone(), s);
            }
        }
        loop {
            let mut changed = false;
            for t in &all {
                let mut add: BTreeSet<String> = BTreeSet::new();
                for c in candidates(t, &self.fields) {
                    if let Some(inner) = owned.get(&c) {
                        add.extend(inner.iter().cloned());
                    }
                }
                if !add.is_empty() {
                    let e = owned.entry(t.clone()).or_default();
                    for d in add {
                        if e.insert(d) {
                            changed = true;
                        }
                    }
                }
            }
            if !changed {
                break;
            }
        }
        for k in owned.keys() {
            self.drop_relevant.insert(k.clone());
        }
    }
}
