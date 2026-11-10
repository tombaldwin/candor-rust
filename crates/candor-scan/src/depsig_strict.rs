//! SOUNDNESS R1619 — TYPING A DEPENDENCY VALUE BY A PER-ITEM PROOF, where `judged` refuses the whole crate.
//!
//! `DepSigs::ret_of` (R1578) asks `judged` what a callee returns. `judged` was written for NARROWING (`allowed`): there the
//! candidates are every same-leaf definition anywhere in the crate and the answer is their UNION, so one definition out of sight
//! (a foreign item macro anywhere — `pin_project!`, `include!`, `bitflags!` —, a macro naming the leaf, an external `pub use`
//! glob anywhere) makes every answer unreadable. Over the 4,951 readable non-proc-macro registry checkouts on this machine that
//! refuses 1,956 crates for ANY free fn and 2,074 for any method of a declared type, which is why `reqwest`, `backoff`,
//! `deadpool-postgres` and `tokio-postgres` values stay typed as their chain root.
//!
//! Typing asks a narrower question — what does THIS item return — and Rust's own duplicate-definition rules answer the
//! "out of sight" part per item. A name can be bound only once per namespace in a module (E0428/E0255), and an inherent method
//! name only once over overlapping impls of a type (E0592), whatever macro or file produced the second definition. So a VISIBLE
//! definition that is present in every build (no `#[cfg]`, not re-emitted by `cfg_if!`/a pass-through, no attribute macro) and,
//! for a method, sits in an impl over the type's own generic parameters, is the only definition there is. Hidden code elsewhere
//! cannot change what it returns. That is the proof this module checks, together with two resolutions that the leaf-based
//! reading did not need:
//!
//!   * the RECEIVER: the type path is resolved through the module tree, following only explicit `mod` declarations and
//!     non-glob `use`s, to the one module that declares it, and the method's impl must sit in that module (an impl in a module
//!     that does not declare the leaf could be an impl of a hidden same-leaf type, and refuses);
//!   * the RETURN: the written return path is resolved the same way from the module the candidate sits in. A name an explicit
//!     `use` reaches in a module that binds it out of sight (`deadpool_postgres::Pool`, made by `managed_reexports!`) is typed
//!     as that PATH, which is exactly the type a consumer who annotates `pool: &deadpool_postgres::Pool` names.
//!
//! A binding that IS conditional (a `#[cfg]`, a re-emitted item) is accepted only in a module with no opaque item (foreign
//! macro, attribute macro, custom derive) and no glob, where every alternative would be visible; a conditional METHOD only when
//! the crate has no opaque module at all. Every doubt is `None`, and `ret_uncached` then keeps its refusal. This runs only after
//! `judged` refused, so what `judged` answers is unchanged (`CANDOR_R1619_OFF` disables it, for falsification).
use super::*;

/// The names an item binds in `module` (and, for an inline module, its items under the child module): fns, types, `use`
/// bindings, modules, and an impl's self leaf and method names.
pub(super) fn item_names(it: &syn::Item, module: Option<&str>, out: &mut HashSet<(Option<String>, String)>) {
    let m = module.map(str::to_string);
    let mut put = |n: String| {
        out.insert((m.clone(), n));
    };
    match it {
        syn::Item::Fn(f) => put(f.sig.ident.to_string()),
        syn::Item::Struct(i) => put(i.ident.to_string()),
        syn::Item::Enum(i) => put(i.ident.to_string()),
        syn::Item::Union(i) => put(i.ident.to_string()),
        syn::Item::Trait(i) => put(i.ident.to_string()),
        syn::Item::Type(i) => put(i.ident.to_string()),
        syn::Item::Const(i) => put(i.ident.to_string()),
        syn::Item::Static(i) => put(i.ident.to_string()),
        syn::Item::ExternCrate(i) => put(i.rename.as_ref().map(|r| r.1.to_string()).unwrap_or_else(|| i.ident.to_string())),
        syn::Item::Use(u) => {
            let mut v = Vec::new();
            flat_use(&u.tree, &mut Vec::new(), &mut v);
            for (n, _, _) in v {
                if let Some(n) = n {
                    put(n);
                }
            }
        }
        syn::Item::Impl(i) => {
            if let Some(l) = ty_leaf(&i.self_ty) {
                put(l);
            }
            for x in &i.items {
                if let syn::ImplItem::Fn(f) = x {
                    put(f.sig.ident.to_string());
                }
            }
        }
        syn::Item::Mod(md) => {
            put(md.ident.to_string());
            if let Some((_, items)) = &md.content {
                let child = module.map(|b| if b.is_empty() { md.ident.to_string() } else { format!("{b}::{}", md.ident) });
                for x in items {
                    item_names(x, child.as_deref(), out);
                }
            }
        }
        _ => {}
    }
}

/// A `use` tree flattened: `(bound name or None for a glob, full path, is_glob)`.
fn flat_use(t: &syn::UseTree, pre: &mut Vec<String>, out: &mut Vec<(Option<String>, Vec<String>, bool)>) {
    match t {
        syn::UseTree::Path(p) => {
            pre.push(p.ident.to_string());
            flat_use(&p.tree, pre, out);
            pre.pop();
        }
        syn::UseTree::Name(n) => {
            let mut v = pre.clone();
            v.push(n.ident.to_string());
            // `use a::b::{self}` binds `b`
            if n.ident == "self" {
                v.pop();
                if let Some(l) = v.last().cloned() {
                    out.push((Some(l), v, false));
                }
                return;
            }
            out.push((Some(n.ident.to_string()), v, false));
        }
        syn::UseTree::Rename(r) => {
            let mut v = pre.clone();
            v.push(r.ident.to_string());
            if r.ident == "self" {
                v.pop();
            }
            if r.rename != "_" {
                out.push((Some(r.rename.to_string()), v, false));
            }
        }
        syn::UseTree::Glob(_) => out.push((None, pre.clone(), true)),
        syn::UseTree::Group(g) => g.items.iter().for_each(|i| flat_use(i, pre, out)),
    }
}

fn is_pub(v: &syn::Visibility) -> bool {
    matches!(v, syn::Visibility::Public(_))
}

/// `#[cfg(test)]` exactly: an item never compiled into a dependency.
fn test_only(attrs: &[syn::Attribute]) -> bool {
    attrs.iter().any(|a| a.path().is_ident("cfg") && a.meta.require_list().is_ok_and(|l| l.tokens.to_string() == "test"))
}

/// What an item's attributes say about it.
#[derive(Default, Clone, Copy)]
struct Attrs {
    /// `#[cfg]`, or a `cfg_attr` that may add one: the item may be absent from a build
    cond: bool,
    /// an attribute macro (directly or through `cfg_attr`) or an attribute this reading does not know: the item as written
    /// may not be the item compiled
    rewrites: bool,
    /// a custom derive: the item stands, but its module gains items out of sight
    emits: bool,
    /// every attribute macro on it is one that rewrites the item IN PLACE and adds no other (`async_trait`,
    /// `tracing::instrument`, `tokio::test`): the item is still unreliable as written, but nothing new appears beside it
    in_place: bool,
}

/// The attribute chunks after a `cfg_attr`'s predicate, each as text (`derive (serde :: Deserialize)`).
fn cfg_attr_chunks(a: &syn::Attribute) -> Vec<String> {
    let Ok(list) = a.meta.require_list() else { return Vec::new() };
    let mut chunks: Vec<String> = vec![String::new()];
    for t in list.tokens.clone() {
        match &t {
            proc_macro2::TokenTree::Punct(p) if p.as_char() == ',' => chunks.push(String::new()),
            _ => {
                let c = chunks.last_mut().expect("nonempty");
                c.push_str(&t.to_string());
                c.push(' ');
            }
        }
    }
    chunks.into_iter().skip(1).map(|c| c.trim().to_string()).filter(|c| !c.is_empty()).collect()
}

fn attrs_of(attrs: &[syn::Attribute]) -> Attrs {
    const INERT: &[&str] = &["doc", "inline", "cold", "must_use", "deprecated", "allow", "warn", "deny", "forbid", "expect",
        "track_caller", "repr", "non_exhaustive", "macro_export", "macro_use", "automatically_derived", "rustfmt", "clippy",
        "diagnostic", "no_mangle", "export_name", "unstable", "stable", "rustc_const_stable", "feature"];
    // std's derives, and two whose output is trait impls only (serde's in a `const _: () = { .. }` block, thiserror's
    // `impl Error`/`impl Display`): none binds a name or adds an inherent method
    const STD_DERIVES: &[&str] = &["Debug", "Clone", "Copy", "PartialEq", "Eq", "PartialOrd", "Ord", "Hash", "Default",
        "Serialize", "Deserialize", "serde", "Error", "thiserror"];
    let custom_derive = |toks: &str| {
        toks.split(|c: char| !(c.is_alphanumeric() || c == '_')).filter(|w| !w.is_empty())
            .any(|w| !STD_DERIVES.contains(&w) && !matches!(w, "core" | "std" | "fmt" | "cmp" | "hash" | "clone" | "default" | "marker"))
    };
    let mut out = Attrs { in_place: true, ..Attrs::default() };
    const IN_PLACE: &[&str] = &["async_trait", "instrument", "test", "main", "maybe_async", "must_use"];
    // an attribute this reading does not know, on an item that has a custom derive, is most likely that derive's HELPER
    // (`#[serde(..)]`, `#[error(..)]`): it emits, it does not rewrite
    let derived = attrs.iter().any(|a| {
        (a.path().is_ident("derive") && a.meta.require_list().is_ok_and(|l| custom_derive(&l.tokens.to_string())))
            || (a.path().is_ident("cfg_attr") && cfg_attr_chunks(a).iter().any(|c| c.starts_with("derive") && custom_derive(&c["derive".len()..])))
    });
    let unknown = |out: &mut Attrs| if derived { out.emits = true } else { out.rewrites = true; out.in_place = false };
    for a in attrs {
        let segs: Vec<String> = a.path().segments.iter().map(|s| s.ident.to_string()).collect();
        let head = segs.first().map(String::as_str).unwrap_or("");
        let last = segs.last().map(String::as_str).unwrap_or("");
        if (segs.len() > 1 && !matches!(head, "rustfmt" | "clippy" | "diagnostic")) || (segs.len() == 1 && IN_PLACE.contains(&head) && head != "must_use") {
            out.rewrites = true;
            out.in_place &= IN_PLACE.contains(&last);
            continue;
        }
        if segs.len() == 1 && derived && !INERT.contains(&head) && !matches!(head, "cfg" | "cfg_attr" | "derive") {
            unknown(&mut out);
            continue;
        }
        match head {
            "cfg" => out.cond = true,
            "cfg_attr" => {
                let Ok(list) = a.meta.require_list() else {
                    out.rewrites = true;
                    continue;
                };
                let toks: Vec<proc_macro2::TokenTree> = list.tokens.clone().into_iter().collect();
                let mut chunks: Vec<Vec<&proc_macro2::TokenTree>> = vec![Vec::new()];
                for t in &toks {
                    match t {
                        proc_macro2::TokenTree::Punct(p) if p.as_char() == ',' => chunks.push(Vec::new()),
                        _ => chunks.last_mut().expect("nonempty").push(t),
                    }
                }
                for c in chunks.iter().skip(1).filter(|c| !c.is_empty()) {
                    match c.first() {
                        Some(proc_macro2::TokenTree::Ident(i)) if i == "cfg" || i == "cfg_attr" => out.cond = true,
                        Some(proc_macro2::TokenTree::Ident(i)) if i == "derive" => {
                            let s: String = c.iter().skip(1).map(|t| t.to_string()).collect::<Vec<_>>().join(" ");
                            if custom_derive(&s) {
                                out.emits = true;
                            }
                        }
                        Some(proc_macro2::TokenTree::Ident(i)) if INERT.contains(&i.to_string().as_str()) && c.len() <= 2 => {}
                        Some(proc_macro2::TokenTree::Ident(_)) if c.len() <= 2 => unknown(&mut out),
                        _ => {
                            out.rewrites = true;
                            out.in_place = false;
                        }
                    }
                }
            }
            "derive" => {
                let toks = a.meta.require_list().map(|l| l.tokens.to_string()).unwrap_or_default();
                if custom_derive(&toks) {
                    out.emits = true;
                }
            }
            h if INERT.contains(&h) => {}
            _ => unknown(&mut out),
        }
    }
    out
}

#[derive(Debug)]
enum Bind {
    /// a struct / enum / union / trait, with its generic parameter count and whether it is conditional
    Nominal { params: usize, cond: bool, public: bool },
    Alias { target: Option<Vec<String>>, cond: bool, public: bool },
    Use { path: Vec<String>, cond: bool, public: bool },
    Mod { cond: bool, public: bool },
    /// anything else in the type namespace this reading does not follow (an `extern crate`, an opaque item's)
    Other,
}

impl Bind {
    fn cond(&self) -> bool {
        match self {
            Bind::Nominal { cond, .. } | Bind::Alias { cond, .. } | Bind::Use { cond, .. } | Bind::Mod { cond, .. } => *cond,
            Bind::Other => true,
        }
    }
}

/// A crate-level free fn, as the per-item proof reads it.
struct FreeFn {
    public: bool,
    cond: bool,
    is_async: bool,
    generics: HashSet<String>,
    output: syn::ReturnType,
}

/// The module tree of one dependency source, read once.
#[derive(Default)]
pub(super) struct ModTable {
    /// every module a `mod` item declares (`""` the crate root), with the files that hold it
    mods: HashSet<String>,
    /// module -> files mapped to it by layout (two is a doubt)
    files_of: HashMap<String, usize>,
    /// (module, name) -> the type/module-namespace bindings
    binds: HashMap<(String, String), Vec<Bind>>,
    /// (module, name) -> free fns
    fns: HashMap<(String, String), Vec<FreeFn>>,
    /// modules a `use` glob fills
    globs: HashSet<String>,
    /// modules holding an item this reading cannot see into: a foreign item macro, an attribute macro, a custom derive
    opaque: HashSet<String>,
    /// a file outside the conventional layout (its items' module is unknown)
    unplaced: bool,
    /// SOUNDNESS R1619 — for a CONDITIONAL method (`#[cfg]` on it or its impl), whose alternative under the other cfg may sit
    /// in any module: every identifier an opaque item's tokens carry (a foreign macro can bind what it is handed), the items
    /// a custom derive is on (it is handed them), and whether anything can write items naming nothing it was handed (an
    /// attribute macro, `include!`, a macro in an impl body, a local macro with a computed name)
    opaque_words: HashSet<String>,
    derived: HashSet<String>,
    blind: bool,
}

impl ModTable {
    pub(super) fn build(idx: &SrcIndex) -> ModTable {
        let mut t = ModTable::default();
        let mut declared: HashSet<String> = HashSet::new();
        declared.insert(String::new());
        for (f, m) in idx.files.iter().zip(idx.file_mods.iter()) {
            let Some(m) = m else {
                t.unplaced = true;
                continue;
            };
            *t.files_of.entry(m.clone()).or_default() += 1;
            t.items(idx, m, &f.items, &mut declared);
        }
        t.mods = declared;
        // what can add items INSIDE an impl or trait body, or rewrite one of its members
        struct B(bool);
        impl<'ast> Visit<'ast> for B {
            fn visit_impl_item(&mut self, i: &'ast syn::ImplItem) {
                match i {
                    syn::ImplItem::Macro(_) | syn::ImplItem::Verbatim(_) => self.0 = true,
                    syn::ImplItem::Fn(f) if { let a = attrs_of(&f.attrs); a.rewrites && !a.in_place } => self.0 = true,
                    _ => {}
                }
                syn::visit::visit_impl_item(self, i);
            }
            fn visit_item_impl(&mut self, i: &'ast syn::ItemImpl) {
                let a = attrs_of(&i.attrs);
                if a.rewrites && !a.in_place {
                    self.0 = true;
                }
                syn::visit::visit_item_impl(self, i);
            }
        }
        let mut b = B(false);
        for f in &idx.files {
            b.visit_file(f);
        }
        t.blind |= b.0;
        t
    }

    /// SOUNDNESS R1619 — may a CONDITIONAL inherent method `want` of type `leaf` have an alternative definition out of sight?
    fn cond_method_hidden(&self, leaf: &str, want: &str) -> bool {
        self.blind || self.opaque_words.contains(leaf) || self.opaque_words.contains(want) || self.derived.contains(leaf)
    }

    fn items(&mut self, idx: &SrcIndex, m: &str, items: &[syn::Item], declared: &mut HashSet<String>) {
        let join = |n: &str| if m.is_empty() { n.to_string() } else { format!("{m}::{n}") };
        for it in items {
            let attrs: &[syn::Attribute] = match it {
                syn::Item::Fn(i) => &i.attrs,
                syn::Item::Struct(i) => &i.attrs,
                syn::Item::Enum(i) => &i.attrs,
                syn::Item::Union(i) => &i.attrs,
                syn::Item::Trait(i) => &i.attrs,
                syn::Item::Type(i) => &i.attrs,
                syn::Item::Use(i) => &i.attrs,
                syn::Item::Mod(i) => &i.attrs,
                syn::Item::Impl(i) => &i.attrs,
                syn::Item::Macro(i) => &i.attrs,
                syn::Item::ExternCrate(i) => &i.attrs,
                syn::Item::Const(i) => &i.attrs,
                syn::Item::Static(i) => &i.attrs,
                _ => &[],
            };
            if test_only(attrs) {
                continue;
            }
            let at = attrs_of(attrs);
            if at.rewrites || at.emits {
                self.opaque.insert(m.to_string());
            }
            if at.rewrites && !at.in_place {
                self.blind = true;
            }
            if at.emits {
                self.derived.insert(item_key(it));
            }
            let cond = at.cond || at.rewrites || idx.expanded.contains(&(Some(m.to_string()), item_key(it)));
            let public = match it {
                syn::Item::Struct(i) => is_pub(&i.vis),
                syn::Item::Enum(i) => is_pub(&i.vis),
                syn::Item::Union(i) => is_pub(&i.vis),
                syn::Item::Trait(i) => is_pub(&i.vis),
                syn::Item::Type(i) => is_pub(&i.vis),
                syn::Item::Use(i) => is_pub(&i.vis),
                syn::Item::Mod(i) => is_pub(&i.vis),
                _ => false,
            };
            let bind = |s: &mut ModTable, n: String, b: Bind| s.binds.entry((m.to_string(), n)).or_default().push(b);
            match it {
                syn::Item::Struct(i) => bind(self, i.ident.to_string(), Bind::Nominal { params: i.generics.params.len(), cond, public }),
                syn::Item::Enum(i) => bind(self, i.ident.to_string(), Bind::Nominal { params: i.generics.params.len(), cond, public }),
                syn::Item::Union(i) => bind(self, i.ident.to_string(), Bind::Nominal { params: i.generics.params.len(), cond, public }),
                syn::Item::Trait(i) => bind(self, i.ident.to_string(), Bind::Nominal { params: i.generics.params.len(), cond, public }),
                syn::Item::Type(i) => {
                    let target = match &*i.ty {
                        syn::Type::Path(p) if p.qself.is_none() && p.path.leading_colon.is_none() => {
                            Some(p.path.segments.iter().map(|s| s.ident.to_string()).collect())
                        }
                        _ => None,
                    };
                    bind(self, i.ident.to_string(), Bind::Alias { target, cond, public })
                }
                syn::Item::ExternCrate(i) => {
                    let n = i.rename.as_ref().map(|r| r.1.to_string()).unwrap_or_else(|| i.ident.to_string());
                    bind(self, n, Bind::Other)
                }
                syn::Item::Use(u) => {
                    let mut v = Vec::new();
                    flat_use(&u.tree, &mut Vec::new(), &mut v);
                    for (n, path, glob) in v {
                        if glob {
                            self.globs.insert(m.to_string());
                        } else if let Some(n) = n {
                            let cond = cond || idx.expanded.contains(&(Some(m.to_string()), n.clone()));
                            bind(self, n, Bind::Use { path, cond, public });
                        }
                    }
                }
                syn::Item::Mod(md) => {
                    let child = join(&md.ident.to_string());
                    bind(self, md.ident.to_string(), Bind::Mod { cond, public });
                    if md.attrs.iter().any(|a| a.path().is_ident("path")) {
                        self.unplaced = true;
                    }
                    declared.insert(child.clone());
                    if let Some((_, inner)) = &md.content {
                        *self.files_of.entry(child.clone()).or_default() += 1;
                        self.items(idx, &child, inner, declared);
                    }
                }
                syn::Item::Fn(f) => {
                    let generics = generic_params(&[&f.sig.generics]);
                    self.fns.entry((m.to_string(), f.sig.ident.to_string())).or_default().push(FreeFn {
                        public: is_pub(&f.vis),
                        cond,
                        is_async: f.sig.asyncness.is_some(),
                        generics,
                        output: f.sig.output.clone(),
                    });
                }
                syn::Item::Macro(mi) if mi.ident.is_none() => {
                    let segs: Vec<String> = mi.mac.path.segments.iter().map(|s| s.ident.to_string()).collect();
                    let name = segs.last().cloned().unwrap_or_default();
                    let root = (segs.len() > 1).then(|| segs[0].clone());
                    let local_writes = root.is_none() && idx.macros.get(&name).is_some_and(|d| d.metavar || d.computed || !d.fixed.is_empty());
                    if !macro_is_known(root.as_deref(), &name, &idx.macros, &idx.tainted) || local_writes {
                        // a foreign macro, or a local macro that writes items: what it binds is not read here
                        self.opaque.insert(m.to_string());
                        let mut w = HashSet::new();
                        token_words(&mi.mac.tokens, &mut w);
                        if name == "include" || w.is_empty() || idx.macros.get(&name).is_some_and(|d| d.computed) {
                            self.blind = true;
                        }
                        self.opaque_words.extend(w);
                    }
                }
                _ => {}
            }
        }
    }

    /// A module can be read only if one file (or inline body) holds it and its own declaration is sound.
    fn readable(&self, m: &str) -> bool {
        self.mods.contains(m) && self.files_of.get(m).copied().unwrap_or(0) <= 1
    }

    /// The one binding of `n` in module `m`, if it may be relied on: unconditional, or conditional in a module where every
    /// alternative would be visible (no opaque item, no glob).
    fn one(&self, m: &str, n: &str) -> Option<&Bind> {
        let v = self.binds.get(&(m.to_string(), n.to_string()))?;
        let [b] = v.as_slice() else { return None };
        if b.cond() && (self.opaque.contains(m) || self.globs.contains(m)) {
            return None;
        }
        Some(b)
    }

    /// The module a path's module segments name, from module `from`: `crate`/`self`/`super`, then declared child modules only.
    fn mod_path(&self, from: &str, segs: &[String]) -> Option<String> {
        let mut cur = from.to_string();
        for (i, s) in segs.iter().enumerate() {
            match s.as_str() {
                "crate" if i == 0 => cur.clear(),
                "self" if i == 0 => {}
                "super" if segs[..i].iter().all(|x| x == "super" || x == "self") => {
                    if cur.is_empty() {
                        return None;
                    }
                    cur = cur.rsplit_once("::").map(|(a, _)| a.to_string()).unwrap_or_default();
                }
                name => {
                    if !matches!(self.one(&cur, name), Some(Bind::Mod { .. })) {
                        return None;
                    }
                    cur = if cur.is_empty() { name.to_string() } else { format!("{cur}::{name}") };
                    if !self.readable(&cur) {
                        return None;
                    }
                }
            }
        }
        Some(cur)
    }

    /// What name `n` denotes in module `m` (type namespace). `via_use`: reached through an explicit `use`, so a module with no
    /// visible binding of it still binds it somewhere out of sight, and the PATH is the answer.
    fn name(&self, idx: &SrcIndex, m: &str, n: &str, via_use: bool, depth: usize) -> Option<Res> {
        if depth > 8 || !self.readable(m) {
            return None;
        }
        let qual = if m.is_empty() { n.to_string() } else { format!("{m}::{n}") };
        if !self.binds.contains_key(&(m.to_string(), n.to_string())) {
            // a name the module binds out of sight: only an explicit `use` (which compiled) proves it is bound here at all
            return (via_use && !self.fns.contains_key(&(m.to_string(), n.to_string()))).then_some(Res::Path(qual));
        }
        match self.one(m, n)? {
            Bind::Nominal { params, .. } => Some(Res::Decl(qual, *params)),
            Bind::Alias { target, .. } => Some(Res::Alias(qual, m.to_string(), target.clone())),
            Bind::Use { path, .. } => {
                let root = path.first()?;
                if matches!(root.as_str(), "std" | "core" | "alloc") {
                    return Some(Res::Std(path.last()?.clone()));
                }
                if !matches!(root.as_str(), "crate" | "self" | "super") && idx.external_root(root) {
                    return None;
                }
                let (last, mods) = path.split_last()?;
                let tm = self.mod_path(m, mods)?;
                self.name(idx, &tm, last, true, depth + 1)
            }
            Bind::Mod { .. } | Bind::Other => None,
        }
    }

    /// A written type path in module `m`.
    fn ty(&self, idx: &SrcIndex, m: &str, segs: &[String]) -> Option<Res> {
        let (last, mods) = segs.split_last()?;
        if mods.first().is_some_and(|r| matches!(r.as_str(), "std" | "core" | "alloc")) {
            return Some(Res::Std(last.clone()));
        }
        if mods.is_empty() {
            if self.binds.contains_key(&(m.to_string(), last.clone())) {
                return self.name(idx, m, last, false, 0);
            }
            // a std prelude type, where nothing in the module could have bound the name out of sight
            let prelude = matches!(last.as_str(), "String" | "Vec" | "Box" | "Option" | "Result");
            return (prelude && self.readable(m) && !self.opaque.contains(m) && !self.globs.contains(m)).then(|| Res::Std(last.clone()));
        }
        let tm = self.mod_path(m, mods)?;
        self.name(idx, &tm, last, false, 0)
    }

    /// SOUNDNESS R1619 — the shortest PUBLIC path a consumer can spell the nominal `q` (crate-relative declaring qual) by: a
    /// `pub` binding, in a module reached from the root through `pub mod`s only, that resolves to `q`. `None` when there is
    /// none (or `q` is itself that path).
    fn public_path(&self, idx: &SrcIndex, q: &str) -> Option<String> {
        let mut public: HashSet<String> = HashSet::new();
        let mut work = vec![String::new()];
        while let Some(m) = work.pop() {
            if public.len() > 512 || !public.insert(m.clone()) {
                continue;
            }
            for ((bm, n), v) in &self.binds {
                if *bm == m && matches!(v.as_slice(), [Bind::Mod { public: true, .. }]) {
                    work.push(if m.is_empty() { n.clone() } else { format!("{m}::{n}") });
                }
            }
        }
        let mut best: Option<String> = None;
        for ((bm, n), v) in &self.binds {
            if !public.contains(bm) {
                continue;
            }
            let pubb = matches!(v.as_slice(), [Bind::Nominal { public: true, .. }] | [Bind::Use { public: true, .. }] | [Bind::Alias { public: true, .. }]);
            if !pubb {
                continue;
            }
            let hit = match self.name(idx, bm, n, false, 0) {
                Some(Res::Decl(d, _)) => d == q,
                Some(Res::Alias(_, am, Some(t))) => matches!(self.nominal(idx, &am, &t, 0), Some((dm, l, _)) if (if dm.is_empty() { l.clone() } else { format!("{dm}::{l}") }) == q) && matches!(v.as_slice(), [Bind::Use { .. }]),
                _ => false,
            };
            if hit {
                let cand = if bm.is_empty() { n.clone() } else { format!("{bm}::{n}") };
                let better = match &best {
                    None => true,
                    Some(b) => (cand.matches("::").count(), &cand) < (b.matches("::").count(), b),
                };
                if better {
                    best = Some(cand);
                }
            }
        }
        best.filter(|b| b != q)
    }

    /// The declaring module and generic count of the NOMINAL type a path in module `m` names, through aliases.
    fn nominal(&self, idx: &SrcIndex, m: &str, segs: &[String], depth: usize) -> Option<(String, String, usize)> {
        if depth > 4 {
            return None;
        }
        match self.ty(idx, m, segs)? {
            Res::Decl(q, n) => {
                let (dm, leaf) = q.rsplit_once("::").map(|(a, b)| (a.to_string(), b.to_string())).unwrap_or((String::new(), q.clone()));
                Some((dm, leaf, n))
            }
            Res::Alias(_, am, Some(target)) => self.nominal(idx, &am, &target, depth + 1),
            _ => None,
        }
    }
}

/// What a resolved type path is.
#[derive(Debug)]
enum Res {
    /// a nominal declaration: its crate-relative qual, and generic parameter count
    Decl(String, usize),
    /// a type alias: its qual, its module and its target path
    Alias(String, String, Option<Vec<String>>),
    /// a name bound out of sight, reached through explicit `use`s: its path
    Path(String),
    Std(String),
}

/// The key `expand_items` recorded an item's name under.
fn item_key(it: &syn::Item) -> String {
    match it {
        syn::Item::Fn(f) => f.sig.ident.to_string(),
        syn::Item::Struct(i) => i.ident.to_string(),
        syn::Item::Enum(i) => i.ident.to_string(),
        syn::Item::Union(i) => i.ident.to_string(),
        syn::Item::Trait(i) => i.ident.to_string(),
        syn::Item::Type(i) => i.ident.to_string(),
        syn::Item::Mod(i) => i.ident.to_string(),
        syn::Item::Impl(i) => ty_leaf(&i.self_ty).unwrap_or_default(),
        _ => String::new(),
    }
}

/// One inherent method (or associated fn) named `want` on a self type of leaf `leaf`, as the per-item proof reads it.
struct MCand {
    module: Option<String>,
    nested: bool,
    /// the impl's self type: its leaf is `leaf` and its arguments are distinct generic parameters of the impl
    complete: bool,
    args: usize,
    cond: bool,
    unknown_attr: bool,
    public: bool,
    recv: Option<u8>,
    is_async: bool,
    generics: HashSet<String>,
    output: syn::ReturnType,
}

fn method_cands(idx: &SrcIndex, leaf: &str, want: &str) -> Vec<MCand> {
    struct V<'a> {
        leaf: &'a str,
        want: &'a str,
        idx: &'a SrcIndex,
        base: Option<String>,
        stack: Vec<String>,
        body: usize,
        out: Vec<MCand>,
    }
    impl V<'_> {
        fn module(&self) -> Option<String> {
            self.base.as_ref().map(|b| {
                let mut v: Vec<&str> = if b.is_empty() { vec![] } else { vec![b.as_str()] };
                v.extend(self.stack.iter().map(String::as_str));
                v.join("::")
            })
        }
    }
    impl<'ast> Visit<'ast> for V<'_> {
        fn visit_item_mod(&mut self, m: &'ast syn::ItemMod) {
            if test_only(&m.attrs) {
                return;
            }
            if m.content.is_some() {
                self.stack.push(m.ident.to_string());
                syn::visit::visit_item_mod(self, m);
                self.stack.pop();
            }
        }
        fn visit_block(&mut self, b: &'ast syn::Block) {
            self.body += 1;
            syn::visit::visit_block(self, b);
            self.body -= 1;
        }
        fn visit_item_impl(&mut self, i: &'ast syn::ItemImpl) {
            if i.trait_.is_none() && !test_only(&i.attrs) && ty_leaf(&i.self_ty).as_deref() == Some(self.leaf) {
                let params: HashSet<String> = i.generics.params.iter().map(|p| match p {
                    syn::GenericParam::Type(t) => t.ident.to_string(),
                    syn::GenericParam::Const(c) => c.ident.to_string(),
                    syn::GenericParam::Lifetime(l) => l.lifetime.to_string(),
                }).collect();
                let (complete, args) = match &*i.self_ty {
                    syn::Type::Path(p) if p.qself.is_none() && p.path.segments.len() == 1 => match &p.path.segments[0].arguments {
                        syn::PathArguments::None => (true, 0),
                        syn::PathArguments::AngleBracketed(ab) => {
                            let mut seen: HashSet<String> = HashSet::new();
                            let ok = ab.args.iter().all(|a| {
                                let n = match a {
                                    syn::GenericArgument::Lifetime(l) if l.ident == "_" => return true,
                                    syn::GenericArgument::Lifetime(l) => l.to_string(),
                                    syn::GenericArgument::Type(syn::Type::Path(tp)) if tp.qself.is_none() => match tp.path.get_ident() {
                                        Some(id) => id.to_string(),
                                        None => return false,
                                    },
                                    _ => return false,
                                };
                                params.contains(&n) && seen.insert(n)
                            });
                            (ok, ab.args.len())
                        }
                        syn::PathArguments::Parenthesized(_) => (false, 0),
                    },
                    _ => (false, 0),
                };
                let impl_cls = attrs_of(&i.attrs);
                let module = self.module();
                let impl_cond = module.as_ref().is_some_and(|m| self.idx.expanded.contains(&(Some(m.clone()), self.leaf.to_string())));
                for it in &i.items {
                    let syn::ImplItem::Fn(f) = it else { continue };
                    if f.sig.ident != self.want || test_only(&f.attrs) {
                        continue;
                    }
                    let fcls = attrs_of(&f.attrs);
                    let recv = f.sig.receiver().map(|r| match (&r.reference, r.mutability.is_some(), &*r.ty) {
                        (Some(_), true, _) => 2,
                        (Some(_), false, _) => 1,
                        (None, _, syn::Type::Path(p)) if p.path.is_ident("Self") => 0,
                        (None, _, syn::Type::Reference(t)) => if t.mutability.is_some() { 2 } else { 1 },
                        _ => 9,
                    });
                    self.out.push(MCand {
                        module: module.clone(),
                        nested: self.body > 0,
                        complete,
                        args,
                        cond: impl_cls.cond || fcls.cond || impl_cond
                            || module.as_ref().is_some_and(|m| self.idx.expanded.contains(&(Some(m.clone()), self.want.to_string()))),
                        unknown_attr: impl_cls.rewrites || fcls.rewrites,
                        public: is_pub(&f.vis),
                        recv,
                        is_async: f.sig.asyncness.is_some(),
                        generics: generic_params(&[&i.generics, &f.sig.generics]),
                        output: f.sig.output.clone(),
                    });
                }
            }
            syn::visit::visit_item_impl(self, i);
        }
    }
    let mut v = V { leaf, want, idx, base: None, stack: Vec::new(), body: 0, out: Vec::new() };
    for (i, f) in idx.files.iter().enumerate() {
        v.base = idx.file_mods.get(i).cloned().flatten();
        v.stack.clear();
        v.visit_file(f);
    }
    v.out
}

/// The written return of a candidate: `(async, wraps, path)`, with `Self` replaced by the impl's type; `None` for a shape this
/// reading does not type (no return, a reference, `impl`/`dyn`, a generic, `Self::Assoc`).
fn ret_path(output: &syn::ReturnType, generics: &HashSet<String>, self_ty: Option<&str>) -> Option<(String, Vec<String>)> {
    let syn::ReturnType::Type(_, ty) = output else { return None };
    let mut t: &syn::Type = ty;
    let mut wraps = String::new();
    loop {
        match t {
            syn::Type::Paren(p) => t = &p.elem,
            syn::Type::Group(g) => t = &g.elem,
            syn::Type::Path(p) if p.qself.is_none() && p.path.leading_colon.is_none() => {
                let last = p.path.segments.last()?;
                let leaf = last.ident.to_string();
                if (leaf == "Result" || leaf == "Option") && wraps.len() < 4 {
                    if let syn::PathArguments::AngleBracketed(ab) = &last.arguments {
                        if let Some(syn::GenericArgument::Type(inner)) = ab.args.first() {
                            wraps.push(if leaf == "Result" { 'R' } else { 'O' });
                            t = inner;
                            continue;
                        }
                    }
                    return None;
                }
                let mut v: Vec<String> = p.path.segments.iter().map(|s| s.ident.to_string()).collect();
                if v.iter().any(|s| s == "Self") {
                    if v.len() != 1 {
                        return None;
                    }
                    v = vec![format!("\u{1}{}", self_ty?)];
                }
                if v.len() == 1 && generics.contains(&v[0]) {
                    return None;
                }
                return Some((wraps, v));
            }
            _ => return None,
        }
    }
}

impl DepSigs {
    /// SOUNDNESS R1619 — the public path (`ModTable::public_path`) of the dependency type `q` (a crate-relative declaring qual
    /// R1578 placed), in the lock-pinned source of `alias`.
    pub(crate) fn public_qual(&mut self, dir: &str, alias: &str, real: &str, q: &str) -> Option<String> {
        let src = self.lock_source(dir, alias, real)?;
        let idx = self.index(&src.dir)?;
        if !idx.unparsed.is_empty() || idx.path_attr {
            return None;
        }
        let mt = idx.strict.get_or_init(|| ModTable::build(idx));
        if mt.unplaced {
            return None;
        }
        mt.public_path(idx, q)
    }

    /// SOUNDNESS R1619 — `ret_uncached`'s per-item proof (module doc). `path` is the callee after the crate: `Type::m` (with
    /// `need` the receiver form `0..=2`, or `3` for an associated fn) or a free fn's module path and leaf (`need` `None`).
    /// `Err(reason)` for the reach counter.
    pub(super) fn strict_ret(&mut self, src: &Path, path: &str, need: Option<u8>) -> Result<DepRet, String> {
        let idx = self.index(src).ok_or("unreadable")?;
        if !idx.unparsed.is_empty() {
            return Err("unparsed".into());
        }
        if idx.path_attr {
            return Err("pathattr".into());
        }
        let mt = idx.strict.get_or_init(|| ModTable::build(idx));
        if mt.unplaced {
            return Err("unplaced".into());
        }
        let segs: Vec<String> = path.split("::").map(str::to_string).collect();
        let (leaf, pre) = segs.split_last().ok_or("empty")?;
        let (is_async, wraps, written, home, self_ty) = match need {
            None => {
                let m = mt.mod_path("", pre).ok_or("fnmod")?;
                let v = mt.fns.get(&(m.clone(), leaf.clone())).ok_or("nofn")?;
                let [f] = v.as_slice() else { return Err(format!("fns:{}", v.len())) };
                if mt.binds.contains_key(&(m.clone(), leaf.clone())) && !mt.binds.get(&(m.clone(), leaf.clone())).is_some_and(|b| b.iter().all(|x| matches!(x, Bind::Nominal { .. } | Bind::Alias { .. }))) {
                    return Err("fnbind".into()); // a `use` of the same name beside it: not a proof this reading can make
                }
                if !f.public {
                    return Err("private".into());
                }
                if f.cond && (mt.opaque.contains(&m) || mt.globs.contains(&m)) {
                    return Err("cond".into());
                }
                let (w, p) = ret_path(&f.output, &f.generics, None).ok_or("ret?")?;
                (f.is_async, w, p, m, None)
            }
            Some(k) => {
                let (dm, tleaf, params) = mt.nominal(idx, "", pre, 0).ok_or("recv?")?;
                let cands = method_cands(idx, &tleaf, leaf);
                let mut mine: Vec<&MCand> = Vec::new();
                for c in &cands {
                    let Some(cm) = &c.module else { return Err("candmod?".into()) };
                    if !mt.mods.contains(cm) {
                        continue; // a file no `mod` declares is not compiled (and could only duplicate a unique candidate)
                    }
                    if c.nested {
                        return Err("nested".into());
                    }
                    if *cm == dm {
                        mine.push(c);
                    } else if !matches!(mt.one(cm, &tleaf), Some(Bind::Nominal { .. })) {
                        // an impl in a module that does not itself declare the leaf may be an impl of OUR type
                        return Err("foreignimpl".into());
                    }
                }
                let [c] = mine.as_slice() else { return Err(format!("cands:{}", mine.len())) };
                if !c.complete || c.args != params {
                    return Err("instance".into());
                }
                if c.unknown_attr {
                    return Err("attr".into());
                }
                if c.cond && mt.cond_method_hidden(&tleaf, leaf) {
                    return Err("cond".into());
                }
                if !c.public {
                    return Err("private".into());
                }
                let want = if k == 3 { None } else { Some(k) };
                if c.recv != want {
                    return Err("form".into());
                }
                let (w, p) = ret_path(&c.output, &c.generics, Some(&tleaf)).ok_or("ret?")?;
                (c.is_async, w, p, dm.clone(), Some((dm, tleaf)))
            }
        };
        let ty = if written.len() == 1 && written[0].starts_with('\u{1}') {
            let (dm, tleaf) = self_ty.ok_or("self?")?;
            RetTy::Dep(if dm.is_empty() { tleaf } else { format!("{dm}::{tleaf}") })
        } else {
            match mt.ty(idx, &home, &written).ok_or("place?")? {
                Res::Decl(q, _) | Res::Path(q) => RetTy::Dep(q),
                Res::Std(l) => RetTy::Std(l),
                // an alias: the nominal (or std) type it names when that resolves; a PATH when its target is a path that leaves
                // the crate (`type Pool = deadpool::managed::Pool<..>`); anything else is not typed
                Res::Alias(q, am, target) => {
                    let target = target.ok_or("alias?")?;
                    match mt.ty(idx, &am, &target) {
                        Some(Res::Decl(d, _)) => RetTy::Dep(d),
                        Some(Res::Std(l)) => RetTy::Std(l),
                        _ => RetTy::Dep(q),
                    }
                }
            }
        };
        Ok(DepRet { is_async, wraps: wraps.chars().collect(), ty })
    }
}
