//! ⟨0.40⟩ DECLARED TYPES AND THE DEPENDENCY'S OWN HIERARCHY — `typeSurface.holds`, `returnsProtocol`,
//! `types` and `adds`, FOR RESOLUTION ONLY (candor-spec SPEC §2 ⟨0.40⟩; SOUNDNESS R843; conformance
//! PART 95, `gen_type_surface.py`).
//!
//! THE PRODUCER half (`collect_file` per file, cached in `FileDecls`; `build` per crate) publishes four
//! keys beside ⟨0.23⟩'s `returns`. THE CONSUMER half (`SurfaceIdx`, loaded in `deps.rs` and asked from
//! `scan.rs`'s call loop) uses them to ADD a resolution and never to remove one:
//!
//!   * a hit ADDS the declared target's join; whatever the consumer already charged or disclosed for the
//!     site is KEPT beside it. In particular rust's untyped-hop `Unknown` (`dispatch:untyped cross-package
//!     receiver`) is NEVER withdrawn: SPEC §2 ⟨0.40⟩ PERMITS that withdrawal where every walk path hits,
//!     and this engine does not take the permission — see the CHANGELOG for why;
//!   * a walk reads an absent `<T>::<member>` as "may be inherited" and continues through `supers` and
//!     through every `adds` a chained report records for `T`; a node with no key, or a KIND-ONLY key, is
//!     a MISS and is never read as "complete, no supertypes";
//!   * an unknown kind is open (joined with every row, never the own-only rows of an exact receiver);
//!   * two trusted copies UNION `holds`/`returnsProtocol`/`adds`, and read a `types` key they do not
//!     agree on as ABSENT; a stale, judged-nothing or malformed copy contributes a MISS.
//!
//! Nothing in this file touches an effect directly: the producer emits wire keys, and the consumer
//! returns keys for `scan.rs` to join through the one `apply_dep_fn` every other chained join uses.

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};

pub(crate) const KIND_VALUE: &str = "value";
pub(crate) const KIND_PROTOCOL: &str = "protocol";
/// The five kinds SPEC §2 ⟨0.40⟩ defines. Anything else is an UNKNOWN kind — never an exact one.
const KINDS: [&str; 5] = ["protocol", "final", "class", "open", "value"];

/// A written type reference, resolved as far as ONE FILE can, finished at crate level (`resolve`) where
/// the crate's own type set and the dependency list are known:
///   `L:<qual>`              rooted in THIS crate (`crate::`, `self::`, `super::`, a `use crate::…`);
///   `A:<path>`              bound by a `use` to a non-crate path (`dep::Other`, `std::fmt::Display`);
///   `R:<modpath>\u{1}<written>`  written with no `use` binding — module-relative or an extern crate;
///   `*`                     a blanket impl's self type (a bare type parameter).
const BLANKET: &str = "*";
const REL_SEP: char = '\u{1}';

/// Rust's compiler-defined auto traits and the `?Sized` relaxation: the NAMED markers "exactly ONE
/// protocol" ignores (SPEC §2 ⟨0.40⟩). Lifetime bounds are skipped by construction.
const AUTO_TRAITS: [&str; 6] = ["Send", "Sync", "Unpin", "UnwindSafe", "RefUnwindSafe", "Sized"];

/// The std prelude's nominal names a bare spelling can reach with no `use`. A supertype resolving here is
/// a PLATFORM supertype, which SPEC §2 ⟨0.40⟩ lets `supers` omit (the builtin frontier).
const PRELUDE: [&str; 33] = [
    "Clone", "Copy", "Send", "Sync", "Sized", "Unpin", "Default", "Drop", "Eq", "PartialEq", "Ord",
    "PartialOrd", "Fn", "FnMut", "FnOnce", "From", "Into", "TryFrom", "TryInto", "AsRef", "AsMut",
    "Iterator", "IntoIterator", "DoubleEndedIterator", "ExactSizeIterator", "Extend", "ToOwned",
    "ToString", "FromIterator", "Option", "Result", "String", "Vec",
];
const PLATFORM_ROOTS: [&str; 5] = ["std", "core", "alloc", "proc_macro", "test"];

/// `#[derive]`s whose expansion the producer KNOWS: each implements only the std trait it names, which
/// `supers` omits anyway. Any other derive is a proc macro whose output is invisible, and SPEC §2 ⟨0.40⟩
/// makes every type of its crate unclosable ("a proc macro may emit an `impl` for any type").
const STD_DERIVES: [&str; 9] = ["Debug", "Clone", "Copy", "PartialEq", "Eq", "PartialOrd", "Ord", "Hash", "Default"];

/// Attributes the COMPILER (or a tool namespace) defines. Any other attribute on an item may be an
/// attribute proc macro, which REPLACES its item: the item's type key is absent, and the crate is
/// unclosable (the macro may emit an `impl` anywhere). THE LIST IS THE SAFE DIRECTION: an attribute it
/// does not know costs a `supers` list (a miss downstream), never publishes a short one (a silence).
const BUILTIN_ATTRS: [&str; 52] = [
    "cfg", "cfg_attr", "derive", "allow", "warn", "deny", "forbid", "expect", "deprecated", "must_use",
    "doc", "inline", "cold", "track_caller", "non_exhaustive", "repr", "path", "macro_use", "macro_export",
    "automatically_derived", "test", "ignore", "should_panic", "bench", "no_mangle", "export_name",
    "link_section", "link", "link_name", "link_ordinal", "used", "target_feature", "global_allocator",
    "panic_handler", "proc_macro", "proc_macro_derive", "proc_macro_attribute", "no_std",
    "no_implicit_prelude", "recursion_limit", "type_length_limit", "crate_type", "crate_name", "feature",
    "debugger_visualizer", "collapse_debuginfo", "coverage", "instruction_set", "naked", "unsafe",
    "windows_subsystem", "no_main",
];
const TOOL_ATTR_ROOTS: [&str; 4] = ["rustfmt", "clippy", "diagnostic", "rust_analyzer"];
/// Item-position macro invocations whose expansion cannot add a supertype to a type this crate declares.
const INERT_ITEM_MACROS: [&str; 4] = ["thread_local", "compile_error", "global_asm", "lazy_static"];

/// What ONE FILE contributes to the ⟨0.40⟩ surface. Cached inside `FileDecls`, so a warm `--incremental`
/// run publishes exactly what a cold one does.
#[derive(Clone, Debug, Default, PartialEq, serde::Serialize, serde::Deserialize)]
pub(crate) struct FileSurface {
    /// (qual, kind, replaced) — `replaced`: an attribute macro sits on it, so its kind is unknowable.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub(crate) types: Vec<(String, String, bool)>,
    /// (self TyRef, supertype TyRef): an `impl Tr for S`, a supertrait (`trait Sub: Super`, `where Self:
    /// Super`, recorded as `(L:<Sub>, Super)`), and a blanket impl (`("*", Tr)`).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub(crate) impls: Vec<(String, String)>,
    /// (owner, member, value TyRef, the value is ONE protocol). `owner` is `M:<modpath>` for a module
    /// item, else the owning type's TyRef.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub(crate) holds: Vec<(String, String, String, bool)>,
    /// Why a type of this crate cannot be closed, if anything in this file says so. Non-empty anywhere
    /// in the crate = every type of the crate is KIND-ONLY.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub(crate) unclosable: Vec<String>,
    /// CONSUMER SIDE: every `use` path written anywhere in this file (globs as `a::b::*`), so the call loop
    /// can ask whether a trait of a chained package is IN SCOPE where a member lookup missed.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub(crate) uses: Vec<String>,
}

fn path_segs(p: &syn::Path) -> Vec<String> {
    p.segments.iter().map(|s| s.ident.to_string()).collect()
}

fn attr_is_builtin(a: &syn::Attribute) -> bool {
    let segs = path_segs(a.path());
    match segs.as_slice() {
        [one] => BUILTIN_ATTRS.contains(&one.as_str()) || one.starts_with("rustc_"),
        [root, ..] => TOOL_ATTR_ROOTS.contains(&root.as_str()),
        [] => true,
    }
}

/// `cfg_attr(pred, a, b)`: the attributes it may apply, as (path, is a `derive`, the derive names).
/// Unparseable → a path no builtin list contains, so an unreadable `cfg_attr` counts as a possible macro.
fn cfg_attr_inner(a: &syn::Attribute) -> Vec<(String, Vec<String>)> {
    let parsed = a.parse_args_with(
        syn::punctuated::Punctuated::<syn::Meta, syn::Token![,]>::parse_terminated,
    );
    let Ok(metas) = parsed else { return vec![("?".to_string(), Vec::new())] };
    metas
        .iter()
        .skip(1) // the predicate
        .map(|m| {
            let path = path_segs(m.path()).join("::");
            let derives = match m {
                syn::Meta::List(l) if l.path.is_ident("derive") => l
                    .parse_args_with(syn::punctuated::Punctuated::<syn::Path, syn::Token![,]>::parse_terminated)
                    .map(|ps| ps.iter().map(|p| p.segments.last().map(|s| s.ident.to_string()).unwrap_or_default()).collect())
                    .unwrap_or_else(|_| vec!["?".to_string()]),
                _ => Vec::new(),
            };
            (path, derives)
        })
        .collect()
}

fn derive_names(a: &syn::Attribute) -> Vec<String> {
    let mut out = Vec::new();
    let _ = a.parse_nested_meta(|m| {
        out.push(m.path.segments.last().map(|s| s.ident.to_string()).unwrap_or_default());
        Ok(())
    });
    out
}

/// (an attribute that may be a macro sits here, a non-std derive sits here).
fn scan_attrs(attrs: &[syn::Attribute]) -> (bool, bool) {
    let (mut mac, mut derive) = (false, false);
    for a in attrs {
        if a.path().is_ident("derive") {
            if derive_names(a).iter().any(|d| !STD_DERIVES.contains(&d.as_str())) {
                derive = true;
            }
        } else if a.path().is_ident("cfg_attr") {
            for (inner, derives) in cfg_attr_inner(a) {
                if inner == "derive" {
                    if derives.iter().any(|d| !STD_DERIVES.contains(&d.as_str())) {
                        derive = true;
                    }
                    continue;
                }
                let segs: Vec<&str> = inner.split("::").collect();
                let builtin = match segs.as_slice() {
                    [one] => BUILTIN_ATTRS.contains(one) || one.starts_with("rustc_"),
                    [root, ..] => TOOL_ATTR_ROOTS.contains(root),
                    [] => true,
                };
                if !builtin {
                    mac = true;
                }
            }
        } else if !attr_is_builtin(a) {
            mac = true;
        }
    }
    (mac, derive)
}

fn skip_item(attrs: &[syn::Attribute], include_tests: bool) -> bool {
    !include_tests && crate::lang::is_cfg_test(attrs)
}

/// A written path, resolved as far as this file can (see the TyRef encodings above). `None` for a path
/// this producer cannot stand behind (an associated-type projection, a `#[cfg]`-collided import).
fn path_ref(
    path: &syn::Path,
    uses: &HashMap<String, String>,
    modpath: &str,
    self_ref: Option<&str>,
) -> Option<String> {
    let segs = path_segs(path);
    let first = segs.first()?.as_str();
    if path.leading_colon.is_some() {
        return Some(format!("A:{}", segs.join("::")));
    }
    let local = |rest: &[String], base: &str| -> String {
        let r = rest.join("::");
        if base.is_empty() { format!("L:{r}") } else if r.is_empty() { format!("L:{base}") } else { format!("L:{base}::{r}") }
    };
    match first {
        "Self" => (segs.len() == 1).then(|| self_ref.map(str::to_string)).flatten(),
        "crate" => Some(local(&segs[1..], "")),
        "self" => Some(local(&segs[1..], modpath)),
        "super" => {
            let mut m: Vec<&str> = modpath.split("::").filter(|s| !s.is_empty()).collect();
            let mut i = 0;
            while segs.get(i).map(String::as_str) == Some("super") {
                m.pop()?;
                i += 1;
            }
            Some(local(&segs[i..], &m.join("::")))
        }
        head => match uses.get(head) {
            Some(v) if v.contains(crate::decls::ALIAS_ALT_SEP) => None,
            Some(v) => {
                let joined = if segs.len() > 1 { format!("{v}::{}", segs[1..].join("::")) } else { v.clone() };
                if let Some(r) = joined.strip_prefix("crate::") {
                    Some(format!("L:{r}"))
                } else if let Some(r) = joined.strip_prefix("self::") {
                    Some(if modpath.is_empty() { format!("L:{r}") } else { format!("L:{modpath}::{r}") })
                } else if joined.starts_with("super::") {
                    None
                } else {
                    Some(format!("A:{joined}"))
                }
            }
            None => Some(format!("R:{modpath}{REL_SEP}{}", segs.join("::"))),
        },
    }
}

/// The ONE protocol among a bound list, auto traits / `?Sized` / lifetimes ignored. Two or more → `None`
/// (a composition is not published).
fn one_protocol<'a>(
    bounds: impl Iterator<Item = &'a syn::TypeParamBound>,
    uses: &HashMap<String, String>,
    modpath: &str,
) -> Option<String> {
    let mut found: Vec<&syn::Path> = Vec::new();
    for b in bounds {
        if let syn::TypeParamBound::Trait(tb) = b {
            if matches!(tb.modifier, syn::TraitBoundModifier::Maybe(_)) {
                continue;
            }
            let leaf = tb.path.segments.last().map(|s| s.ident.to_string()).unwrap_or_default();
            if AUTO_TRAITS.contains(&leaf.as_str()) {
                continue;
            }
            found.push(&tb.path);
        }
    }
    match found.as_slice() {
        [p] => path_ref(p, uses, modpath, None),
        _ => None,
    }
}

/// A declared type for `holds` / `returnsProtocol`: `(TyRef, is_one_protocol)`. A reference is the value's
/// own spelling (`&T` → `T`, `&dyn Tr` → `Tr`); a path's generic arguments are ignored (the binding holds
/// the OUTER type, `bound_return_type`'s rule); anything else is not published.
fn decl_ty(
    ty: &syn::Type,
    uses: &HashMap<String, String>,
    modpath: &str,
    self_ref: Option<&str>,
    generics: &HashSet<String>,
) -> Option<(String, bool)> {
    match ty {
        syn::Type::Reference(r) => decl_ty(&r.elem, uses, modpath, self_ref, generics),
        syn::Type::Paren(p) => decl_ty(&p.elem, uses, modpath, self_ref, generics),
        syn::Type::Group(g) => decl_ty(&g.elem, uses, modpath, self_ref, generics),
        syn::Type::TraitObject(t) => one_protocol(t.bounds.iter(), uses, modpath).map(|r| (r, true)),
        syn::Type::Path(p) if p.qself.is_none() => {
            let head = p.path.segments.first()?.ident.to_string();
            if p.path.segments.len() == 1 && generics.contains(&head) {
                return None;
            }
            path_ref(&p.path, uses, modpath, self_ref).map(|r| (r, false))
        }
        _ => None,
    }
}

fn generic_names(g: &syn::Generics) -> HashSet<String> {
    g.type_params().map(|t| t.ident.to_string()).collect()
}

/// ⟨0.40⟩ `returnsProtocol`'s per-fn half: the ONE trait a signature's result is (`-> impl Tr + Send`,
/// `-> &dyn Tr`), as a TyRef. `Box<dyn Tr>` is a wrapper and is not published.
pub(crate) fn ret_proto_ref(
    output: &syn::ReturnType,
    uses: &HashMap<String, String>,
    modpath: &str,
) -> Option<String> {
    let syn::ReturnType::Type(_, ty) = output else { return None };
    let mut t: &syn::Type = ty;
    let mut behind_ref = false;
    loop {
        match t {
            syn::Type::Reference(r) => {
                behind_ref = true;
                t = &r.elem;
            }
            syn::Type::Paren(p) => t = &p.elem,
            syn::Type::Group(g) => t = &g.elem,
            _ => break,
        }
    }
    match t {
        syn::Type::ImplTrait(i) => one_protocol(i.bounds.iter(), uses, modpath),
        // `-> &dyn Tr`: a bare `dyn Tr` result is unsized, so only the referenced form is real.
        syn::Type::TraitObject(o) if behind_ref => one_protocol(o.bounds.iter(), uses, modpath),
        _ => None,
    }
}

/// Does any block-nested item IMPLEMENT a trait? An `impl` inside a fn body is global in Rust; a
/// producer that does not attribute it cannot close the type it is for.
#[derive(Default)]
struct NestedImpl(bool);
impl<'ast> syn::visit::Visit<'ast> for NestedImpl {
    fn visit_item_impl(&mut self, i: &'ast syn::ItemImpl) {
        if i.trait_.is_some() {
            self.0 = true;
        }
    }
    fn visit_item_macro(&mut self, m: &'ast syn::ItemMacro) {
        // A block-position macro INVOCATION may expand to an impl too.
        if m.ident.is_none() {
            self.0 = true;
        }
    }
}

struct UseCollector<'a>(&'a mut BTreeSet<String>, bool);
impl UseCollector<'_> {
    fn tree(&mut self, t: &syn::UseTree, prefix: &str) {
        let join = |s: &str| if prefix.is_empty() { s.to_string() } else { format!("{prefix}::{s}") };
        match t {
            syn::UseTree::Path(p) => self.tree(&p.tree, &join(&p.ident.to_string())),
            syn::UseTree::Name(n) => {
                self.0.insert(join(&n.ident.to_string()));
            }
            syn::UseTree::Rename(r) => {
                self.0.insert(join(&r.ident.to_string()));
            }
            syn::UseTree::Glob(_) => {
                self.0.insert(join("*"));
            }
            syn::UseTree::Group(g) => {
                for i in &g.items {
                    self.tree(i, prefix);
                }
            }
        }
    }
}
impl<'ast> syn::visit::Visit<'ast> for UseCollector<'_> {
    fn visit_item_use(&mut self, u: &'ast syn::ItemUse) {
        if self.1 || !crate::lang::is_cfg_test(&u.attrs) {
            self.tree(&u.tree, "");
        }
    }
    fn visit_item_mod(&mut self, m: &'ast syn::ItemMod) {
        // A `#[cfg(test)] mod tests { use super::*; }` is not in the build a production scan describes.
        if self.1 || !crate::lang::is_cfg_test(&m.attrs) {
            syn::visit::visit_item_mod(self, m);
        }
    }
    fn visit_item_fn(&mut self, f: &'ast syn::ItemFn) {
        if self.1 || !crate::lang::is_cfg_test(&f.attrs) {
            syn::visit::visit_item_fn(self, f);
        }
    }
}

/// THE PRODUCER, per file. A separate walk over declarations only: nothing here can move an effect.
pub(crate) fn collect_file(items: &[syn::Item], modpath: &str, include_tests: bool) -> FileSurface {
    let mut out = FileSurface::default();
    let mut uses = HashMap::new();
    crate::lang::seed_modpath(modpath, &mut uses);
    walk_items(items, modpath, include_tests, uses, &mut out);
    let mut u = BTreeSet::new();
    let mut uc = UseCollector(&mut u, include_tests);
    for it in items {
        syn::visit::Visit::visit_item(&mut uc, it);
    }
    out.uses = u.into_iter().collect();
    out.unclosable.sort();
    out.unclosable.dedup();
    out
}

fn qual_of(modpath: &str, name: &str) -> String {
    if modpath.is_empty() { name.to_string() } else { format!("{modpath}::{name}") }
}

#[allow(clippy::too_many_lines)]
fn walk_items(
    items: &[syn::Item],
    modpath: &str,
    include_tests: bool,
    mut uses: HashMap<String, String>,
    out: &mut FileSurface,
) {
    let mut alts = HashMap::new();
    crate::lang::collect_item_uses(items, include_tests, &mut uses, &mut alts);
    for (k, a) in &alts {
        if a.len() > 1 {
            uses.insert(k.clone(), a.join(&crate::decls::ALIAS_ALT_SEP.to_string()));
        }
    }
    let nested = |out: &mut FileSurface, f: &dyn Fn(&mut NestedImpl)| {
        let mut v = NestedImpl::default();
        f(&mut v);
        if v.0 {
            out.unclosable.push("an impl or macro inside a block".to_string());
        }
    };
    for it in items {
        let attrs: &[syn::Attribute] = match it {
            syn::Item::Struct(s) => &s.attrs,
            syn::Item::Enum(e) => &e.attrs,
            syn::Item::Union(u) => &u.attrs,
            syn::Item::Trait(t) => &t.attrs,
            syn::Item::Impl(i) => &i.attrs,
            syn::Item::Fn(f) => &f.attrs,
            syn::Item::Mod(m) => &m.attrs,
            syn::Item::Static(s) => &s.attrs,
            syn::Item::Const(c) => &c.attrs,
            syn::Item::Macro(m) => &m.attrs,
            syn::Item::Type(t) => &t.attrs,
            _ => &[],
        };
        if skip_item(attrs, include_tests) {
            continue;
        }
        let (mac, derive) = scan_attrs(attrs);
        if mac {
            out.unclosable.push("an attribute macro".to_string());
        }
        if derive {
            out.unclosable.push("a proc-macro derive".to_string());
        }
        match it {
            syn::Item::Struct(s) => {
                let q = qual_of(modpath, &s.ident.to_string());
                out.types.push((q.clone(), KIND_VALUE.to_string(), mac));
                let g = generic_names(&s.generics);
                let self_ref = format!("L:{q}");
                for (i, f) in s.fields.iter().enumerate() {
                    if !matches!(f.vis, syn::Visibility::Public(_)) {
                        continue;
                    }
                    let member = f.ident.as_ref().map_or_else(|| i.to_string(), |id| id.to_string());
                    if let Some((v, proto)) = decl_ty(&f.ty, &uses, modpath, Some(&self_ref), &g) {
                        out.holds.push((self_ref.clone(), member, v, proto));
                    }
                }
            }
            syn::Item::Enum(e) => out.types.push((qual_of(modpath, &e.ident.to_string()), KIND_VALUE.to_string(), mac)),
            syn::Item::Union(u) => out.types.push((qual_of(modpath, &u.ident.to_string()), KIND_VALUE.to_string(), mac)),
            syn::Item::Trait(t) => {
                let q = qual_of(modpath, &t.ident.to_string());
                out.types.push((q.clone(), KIND_PROTOCOL.to_string(), mac));
                let me = format!("L:{q}");
                let sup = |b: &syn::TypeParamBound, out: &mut FileSurface| {
                    if let syn::TypeParamBound::Trait(tb) = b {
                        if matches!(tb.modifier, syn::TraitBoundModifier::Maybe(_)) {
                            return;
                        }
                        match path_ref(&tb.path, &uses, modpath, Some(&me)) {
                            Some(r) => out.impls.push((me.clone(), r)),
                            None => out.impls.push((me.clone(), "?".to_string())),
                        }
                    }
                };
                for b in &t.supertraits {
                    sup(b, out);
                }
                // `where Self: Super` is a supertrait the bound list does not show.
                if let Some(w) = &t.generics.where_clause {
                    for p in &w.predicates {
                        if let syn::WherePredicate::Type(pt) = p {
                            if matches!(&pt.bounded_ty, syn::Type::Path(tp) if tp.path.is_ident("Self")) {
                                for b in &pt.bounds {
                                    sup(b, out);
                                }
                            }
                        }
                    }
                }
                for ti in &t.items {
                    let (ia, body): (&[syn::Attribute], Option<&syn::Block>) = match ti {
                        syn::TraitItem::Fn(f) => (&f.attrs, f.default.as_ref()),
                        syn::TraitItem::Const(c) => (&c.attrs, None),
                        syn::TraitItem::Macro(m) => {
                            out.unclosable.push("a macro in a trait".to_string());
                            (&m.attrs, None)
                        }
                        _ => (&[], None),
                    };
                    if scan_attrs(ia).0 {
                        out.unclosable.push("an attribute macro".to_string());
                    }
                    if let Some(b) = body {
                        nested(out, &|v| syn::visit::Visit::visit_block(v, b));
                    }
                }
            }
            syn::Item::Impl(i) => {
                let g = generic_names(&i.generics);
                // The self type. A bare type parameter is a BLANKET impl (`impl<T: B> Tr for T`); a
                // reference is the referent (`impl Tr for &S` serves `s.m()` by autoref).
                let mut st: &syn::Type = &i.self_ty;
                while let syn::Type::Reference(r) = st {
                    st = &r.elem;
                }
                let self_ref = match st {
                    syn::Type::Path(p) if p.qself.is_none() => {
                        let segs = path_segs(&p.path);
                        if segs.len() == 1 && g.contains(&segs[0]) {
                            Some(BLANKET.to_string())
                        } else {
                            path_ref(&p.path, &uses, modpath, None)
                        }
                    }
                    _ => None, // `impl Tr for dyn X`, a tuple, a slice: no key can name it
                };
                if let Some((_, tp, _)) = &i.trait_ {
                    if let Some(sr) = &self_ref {
                        let tr = path_ref(tp, &uses, modpath, None).unwrap_or_else(|| "?".to_string());
                        out.impls.push((sr.clone(), tr));
                    }
                }
                for ii in &i.items {
                    match ii {
                        syn::ImplItem::Fn(f) => {
                            if scan_attrs(&f.attrs).0 {
                                out.unclosable.push("an attribute macro".to_string());
                            }
                            nested(out, &|v| syn::visit::Visit::visit_block(v, &f.block));
                        }
                        syn::ImplItem::Const(c) => {
                            if i.trait_.is_none() && matches!(c.vis, syn::Visibility::Public(_)) {
                                if let Some(sr) = self_ref.as_ref().filter(|s| s.as_str() != BLANKET) {
                                    if let Some((v, proto)) = decl_ty(&c.ty, &uses, modpath, Some(sr), &g) {
                                        out.holds.push((sr.clone(), c.ident.to_string(), v, proto));
                                    }
                                }
                            }
                        }
                        syn::ImplItem::Macro(m) if scan_attrs(&m.attrs).0 => {
                            out.unclosable.push("an attribute macro".to_string());
                        }
                        _ => {}
                    }
                }
            }
            syn::Item::Static(s) => {
                if matches!(s.vis, syn::Visibility::Public(_)) {
                    if let Some((v, proto)) = decl_ty(&s.ty, &uses, modpath, None, &HashSet::new()) {
                        out.holds.push((format!("M:{modpath}"), s.ident.to_string(), v, proto));
                    }
                }
                nested(out, &|v| syn::visit::Visit::visit_expr(v, &s.expr));
            }
            syn::Item::Const(c) => {
                if matches!(c.vis, syn::Visibility::Public(_)) && c.ident != "_" {
                    if let Some((v, proto)) = decl_ty(&c.ty, &uses, modpath, None, &HashSet::new()) {
                        out.holds.push((format!("M:{modpath}"), c.ident.to_string(), v, proto));
                    }
                }
                nested(out, &|v| syn::visit::Visit::visit_expr(v, &c.expr));
            }
            syn::Item::Fn(f) => nested(out, &|v| syn::visit::Visit::visit_block(v, &f.block)),
            syn::Item::Mod(m) => {
                if let Some((_, inner)) = &m.content {
                    let sub = crate::decls::submodule_uses(&uses, inner, include_tests);
                    walk_items(inner, &qual_of(modpath, &m.ident.to_string()), include_tests, sub, out);
                }
            }
            syn::Item::Macro(m) if m.ident.is_none() => {
                let leaf = m.mac.path.segments.last().map(|s| s.ident.to_string()).unwrap_or_default();
                if !INERT_ITEM_MACROS.contains(&leaf.as_str()) {
                    out.unclosable.push("an item-position macro invocation".to_string());
                }
            }
            _ => {}
        }
    }
}

// ═══════════════════════════════════════════════════════════════════════════════════════════════════
// THE PRODUCER, per crate.
// ═══════════════════════════════════════════════════════════════════════════════════════════════════

/// How a TyRef resolved, crate-wide.
#[derive(Debug, Clone, PartialEq)]
enum Res {
    Local(String),
    /// A wire key in the OWNING package's namespace (`base#Tok`).
    Foreign(String),
    Platform,
    Unresolved,
}

pub(crate) struct BuildCtx<'a> {
    pub(crate) crate_name: &'a str,
    /// Manifest dependency names as a path spells them (`[dependencies]` keys).
    pub(crate) deps: &'a HashSet<String>,
    /// alias -> real package name.
    pub(crate) renames: &'a HashMap<String, String>,
    /// The chained reports' merged `types`, to spell a foreign type in its owner's namespace.
    pub(crate) chained: &'a SurfaceIdx,
    /// The scan has `unanalyzed` source: no `supers` list can be called complete.
    pub(crate) incomplete: bool,
}

struct Locals {
    /// qual -> kind; `None` = declared with two different kinds (cfg arms) — no key.
    kinds: HashMap<String, Option<String>>,
    replaced: HashSet<String>,
    by_leaf: HashMap<String, Vec<String>>,
}

impl BuildCtx<'_> {
    fn foreign(&self, alias: &str, rest: &str) -> Res {
        let real = self.renames.get(alias).map_or(alias, String::as_str).replace('-', "_");
        if rest.is_empty() {
            return Res::Unresolved;
        }
        let written = format!("{real}#{rest}");
        // Spell it in the OWNER's namespace when a chained `types` says how (⟨0.39⟩ obligation 2): the
        // exact key, else the one key of that package with the same leaf.
        let key = self.chained.type_key(&real, rest).unwrap_or(written);
        Res::Foreign(key)
    }

    fn resolve(&self, r: &str, locals: &Locals) -> Res {
        let local = |q: &str| -> Option<Res> { locals.kinds.contains_key(q).then(|| Res::Local(q.to_string())) };
        let local_leaf = |q: &str| -> Option<Res> {
            let leaf = q.rsplit("::").next().unwrap_or(q);
            match locals.by_leaf.get(leaf).map(Vec::as_slice) {
                Some([one]) => Some(Res::Local(one.clone())),
                _ => None,
            }
        };
        if let Some(q) = r.strip_prefix("L:") {
            return local(q).or_else(|| local_leaf(q)).unwrap_or(Res::Unresolved);
        }
        let (modpath, written, via_use) = if let Some(a) = r.strip_prefix("A:") {
            ("", a, true)
        } else if let Some(rel) = r.strip_prefix("R:") {
            let (m, w) = rel.split_once(REL_SEP).unwrap_or(("", rel));
            (m, w, false)
        } else {
            return Res::Unresolved;
        };
        let head = written.split("::").next().unwrap_or(written);
        let rest = written.split_once("::").map_or("", |(_, r)| r);
        if !via_use {
            if let Some(l) = local(&qual_of(modpath, written)) {
                return l;
            }
        }
        if head == self.crate_name {
            return local(rest).or_else(|| local_leaf(rest)).unwrap_or(Res::Unresolved);
        }
        if self.deps.contains(head) || self.renames.contains_key(head) {
            return self.foreign(head, rest);
        }
        if PLATFORM_ROOTS.contains(&head) || (rest.is_empty() && PRELUDE.contains(&head)) {
            return Res::Platform;
        }
        if via_use {
            // `use inner::X` (2018 module-relative) or a `use` the walk could not root: ask the crate.
            if let Some(l) = local(written) {
                return l;
            }
        }
        Res::Unresolved
    }

    fn key(&self, res: &Res) -> Option<String> {
        match res {
            Res::Local(q) => Some(format!("{}#{q}", self.crate_name)),
            Res::Foreign(k) => Some(k.clone()),
            _ => None,
        }
    }
}

/// THE PRODUCER, per crate: the four ⟨0.40⟩ keys, written into `ts` beside ⟨0.23⟩'s `returns`.
pub(crate) fn build(
    ctx: &BuildCtx<'_>,
    files: &[&FileSurface],
    fns: &[crate::model::FnInfo],
    ts: &mut candor_report::TypeSurface,
) {
    let mut locals = Locals { kinds: HashMap::new(), replaced: HashSet::new(), by_leaf: HashMap::new() };
    for f in files {
        for (q, kind, replaced) in &f.types {
            let e = locals.kinds.entry(q.clone()).or_insert_with(|| Some(kind.clone()));
            if e.as_deref() != Some(kind.as_str()) {
                *e = None;
            }
            if *replaced {
                locals.replaced.insert(q.clone());
            }
        }
    }
    for q in locals.kinds.keys() {
        let leaf = q.rsplit("::").next().unwrap_or(q).to_string();
        locals.by_leaf.entry(leaf).or_default().push(q.clone());
    }
    let crate_unclosable = ctx.incomplete || files.iter().any(|f| !f.unclosable.is_empty());
    let is_proto = |q: &str| locals.kinds.get(q).and_then(Option::as_deref) == Some(KIND_PROTOCOL);

    // supers + adds, from every impl and supertrait clause.
    let mut supers: HashMap<String, BTreeSet<String>> = HashMap::new();
    let mut open: HashSet<String> = HashSet::new(); // types an unresolvable edge makes unclosable
    let mut blanket: BTreeSet<String> = BTreeSet::new();
    let mut blanket_unresolved = false;
    let mut adds: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    for f in files {
        for (sr, tr) in &f.impls {
            let t = if tr == "?" { Res::Unresolved } else { ctx.resolve(tr, &locals) };
            if sr == BLANKET {
                match &t {
                    Res::Platform => {}
                    Res::Unresolved => blanket_unresolved = true,
                    other => blanket.extend(ctx.key(other)),
                }
                continue;
            }
            match ctx.resolve(sr, &locals) {
                Res::Local(q) => match &t {
                    Res::Platform => {}
                    Res::Unresolved => {
                        open.insert(q);
                    }
                    other => {
                        supers.entry(q).or_default().extend(ctx.key(other));
                    }
                },
                Res::Foreign(fk) => {
                    if let Res::Local(tq) = &t {
                        if is_proto(tq) {
                            adds.entry(fk).or_default().extend(ctx.key(&t));
                        }
                    }
                }
                Res::Platform => {}
                Res::Unresolved => {
                    // An impl for a type this producer cannot attribute: if its LEAF names a local type,
                    // that type may be the one — it cannot be closed.
                    let leaf = sr.rsplit(['\u{1}', ':']).next().unwrap_or(sr);
                    for q in locals.by_leaf.get(leaf).into_iter().flatten() {
                        open.insert(q.clone());
                    }
                }
            }
        }
    }
    for (q, kind) in &locals.kinds {
        let Some(kind) = kind else { continue };
        if locals.replaced.contains(q) {
            continue; // an attribute macro may replace the item: no knowable kind, no key
        }
        let closable = !crate_unclosable && !open.contains(q) && !(blanket_unresolved && kind == KIND_VALUE);
        let sup = closable.then(|| {
            let mut s = supers.get(q).cloned().unwrap_or_default();
            if kind == KIND_VALUE {
                s.extend(blanket.iter().cloned()); // a blanket impl may hold for it: over-include
            }
            s.into_iter().collect::<Vec<_>>()
        });
        ts.types.insert(format!("{}#{q}", ctx.crate_name), candor_report::TypeEntry { kind: kind.clone(), supers: sup });
    }
    for (k, v) in adds {
        ts.adds.insert(k, v.into_iter().collect());
    }

    // holds — never-guess on a key two declarations publish differently (a `#[cfg]` pair).
    let mut holds: BTreeMap<String, Option<String>> = BTreeMap::new();
    for f in files {
        for (owner, member, val, _proto) in &f.holds {
            let okey = if let Some(m) = owner.strip_prefix("M:") {
                Some(format!("{}#{}", ctx.crate_name, qual_of(m, member)))
            } else {
                match ctx.resolve(owner, &locals) {
                    Res::Local(q) => Some(format!("{}#{q}::{member}", ctx.crate_name)),
                    _ => None,
                }
            };
            let Some(okey) = okey else { continue };
            let Some(vkey) = ctx.key(&ctx.resolve(val, &locals)) else { continue };
            match holds.get(&okey) {
                Some(Some(prev)) if *prev != vkey => {
                    holds.insert(okey, None);
                }
                Some(_) => {}
                None => {
                    holds.insert(okey, Some(vkey));
                }
            }
        }
    }
    ts.holds = holds.into_iter().filter_map(|(k, v)| v.map(|v| (k, v))).collect();

    // returnsProtocol — keyed exactly as `returns` is, on the FnInfo qual. Two bodies under one qual (a
    // `#[cfg]` pair) publish only when they agree; anything else is dropped, never picked.
    let mut rp: BTreeMap<String, Vec<Option<String>>> = BTreeMap::new();
    for f in fns {
        let val = f.ret_proto.as_deref().and_then(|r| ctx.key(&ctx.resolve(r, &locals)));
        rp.entry(format!("{}#{}", ctx.crate_name, f.qual)).or_default().push(val);
    }
    ts.returns_protocol = rp
        .into_iter()
        .filter_map(|(k, vs)| {
            let first = vs.first()?.clone()?;
            vs.iter().all(|v| v.as_deref() == Some(first.as_str())).then_some((k, first))
        })
        .collect();
    // `returns` must NEVER name a trait — a shipped ⟨0.23⟩ consumer joins its value exactly.
    let own = format!("{}#", ctx.crate_name);
    ts.returns.retain(|_, v| v.strip_prefix(&own).is_none_or(|q| !is_proto(q)));
}

// ═══════════════════════════════════════════════════════════════════════════════════════════════════
// THE CONSUMER INDEX
// ═══════════════════════════════════════════════════════════════════════════════════════════════════

/// One merged `types` key. `kind: None` is an UNKNOWN kind; `supers: None` is KIND-ONLY (or a key the
/// copies disagreed on) — a MISS for a walk, never an empty list.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct TypeInfo {
    pub(crate) kind: Option<&'static str>,
    pub(crate) supers: Option<Vec<String>>,
}

/// Per-package `types` as each TRUSTED copy carried it, before the merge. `None` for a copy = it
/// publishes no readable `types` (an older producer, `types` not in `resolves`, not an object); an inner
/// `None` = that key is malformed in that copy.
type CopyTypes = Option<HashMap<String, Option<(Option<&'static str>, Option<Vec<String>>)>>>;

#[derive(Default)]
pub(crate) struct SurfaceIdx {
    /// `<pkg>#<owner>::<member>` -> declared targets, UNIONED across trusted copies.
    pub(crate) holds: HashMap<String, BTreeSet<String>>,
    pub(crate) rproto: HashMap<String, BTreeSet<String>>,
    /// The merged manifest (see `TypeInfo`).
    pub(crate) types: HashMap<String, TypeInfo>,
    /// `(pkg, leaf)` -> the keys of that package's types with that leaf, for a spelling that is not the
    /// declaring path (a re-export).
    types_by_leaf: HashMap<(String, String), Vec<String>>,
    /// `<owner pkg>#<foreign type>` -> (the ADDING package, the trait). Never complete.
    pub(crate) adds: HashMap<String, BTreeSet<(String, String)>>,
    /// Packages one of whose copies is stale, judged nothing, incomplete or malformed: a walk through
    /// them is a MISS whatever a trusted copy says beside it.
    pub(crate) distrusted: HashSet<String>,
    copies: HashMap<String, Vec<CopyTypes>>,
    holds_tail: HashMap<String, BTreeSet<String>>,
    holds_tail_n: HashMap<String, usize>,
}

fn strings(v: &serde_json::Value) -> Option<Vec<String>> {
    v.as_array()?.iter().map(|x| x.as_str().map(str::to_string)).collect()
}

fn kind_of(s: &str) -> Option<&'static str> {
    KINDS.iter().find(|k| **k == s).copied()
}

/// The `(pkg, last two segments)` tail of a key, for a spelling that reached the same member through a
/// re-export.
fn tail_key(k: &str) -> Option<String> {
    let (pkg, rest) = k.split_once('#')?;
    let segs: Vec<&str> = rest.split("::").collect();
    let t = if segs.len() >= 2 { segs[segs.len() - 2..].join("::") } else { rest.to_string() };
    Some(format!("{pkg}#{t}"))
}

impl SurfaceIdx {
    /// Read one report's ⟨0.40⟩ keys. `trusted` is false for a stale or judged-nothing copy: its surface is
    /// no more trusted than its entries, so it contributes nothing but a MISS for its package.
    pub(crate) fn load(&mut self, v: &serde_json::Value, pkg: Option<&str>, trusted: bool, incomplete: bool) {
        let Some(pkg) = pkg.map(|p| p.replace('-', "_")) else { return };
        if !trusted {
            self.distrusted.insert(pkg);
            return;
        }
        let resolves: HashSet<&str> = v
            .get("resolves")
            .and_then(|x| x.as_array())
            .into_iter()
            .flatten()
            .filter_map(|x| x.as_str())
            .collect();
        let ts = v.get("typeSurface");
        let field = |name: &str| -> Option<&serde_json::Value> {
            if !resolves.contains(name) {
                return None;
            }
            ts.and_then(|t| t.get(name))
        };
        let mut malformed = false;
        // holds / returnsProtocol: string -> string. A wrong JSON type is ABSENT for the whole key.
        for (name, dest) in [("holds", &mut self.holds), ("returnsProtocol", &mut self.rproto)] {
            match field(name) {
                None => {}
                Some(serde_json::Value::Object(m)) => {
                    for (k, val) in m {
                        match val.as_str() {
                            Some(t) => {
                                dest.entry(k.clone()).or_default().insert(t.to_string());
                            }
                            None => malformed = true,
                        }
                    }
                }
                Some(_) => malformed = true,
            }
        }
        match field("adds") {
            None => {}
            Some(serde_json::Value::Object(m)) => {
                for (k, val) in m {
                    match strings(val) {
                        Some(l) => {
                            for s in l {
                                self.adds.entry(k.clone()).or_default().insert((pkg.clone(), s));
                            }
                        }
                        None => malformed = true,
                    }
                }
            }
            Some(_) => malformed = true,
        }
        let copy: CopyTypes = match field("types") {
            // A package that does not LIST `types` (or lists it with no key, meaning "none declared") —
            // the listed-but-absent case is an empty manifest, which is complete.
            None if resolves.contains("types") && ts.is_none_or(|t| t.get("types").is_none()) => Some(HashMap::new()),
            None => None,
            Some(serde_json::Value::Object(m)) => Some(
                m.iter()
                    .map(|(k, e)| {
                        let parsed = e.as_object().and_then(|o| {
                            let kind = o.get("kind")?.as_str()?;
                            let supers = match o.get("supers") {
                                None => None,
                                Some(s) => Some(strings(s)?),
                            };
                            Some((kind_of(kind), supers.map(|mut s| { s.sort(); s.dedup(); s })))
                        });
                        if parsed.is_none() {
                            malformed = true;
                        }
                        (k.clone(), parsed)
                    })
                    .collect(),
            ),
            Some(_) => {
                malformed = true;
                None
            }
        };
        if malformed || incomplete {
            self.distrusted.insert(pkg.clone());
        }
        self.copies.entry(pkg).or_default().push(copy);
    }

    /// Merge the per-copy `types` tables (SPEC §2 ⟨0.40⟩ "two copies union; a distrusted copy is a
    /// miss"): a key every copy of its package carries IDENTICALLY is kept; anything else — present in one
    /// copy and absent in another, FULL in one and KIND-ONLY in the other, a different kind, malformed
    /// anywhere — is read as ABSENT. Taking `supers` from whichever copy closes the type would rebuild the
    /// short closure the manifest rule exists to forbid.
    pub(crate) fn finish(&mut self) {
        for (pkg, copies) in std::mem::take(&mut self.copies) {
            if copies.iter().any(Option::is_none) {
                continue; // a copy with no readable manifest: every key of the package is absent
            }
            let tables: Vec<&HashMap<_, _>> = copies.iter().flatten().collect();
            let Some(first) = tables.first() else { continue };
            for (k, e) in first.iter() {
                let Some(e) = e else { continue };
                if !tables.iter().all(|t| t.get(k) == Some(&Some(e.clone()))) {
                    continue;
                }
                let supers = if self.distrusted.contains(&pkg) { None } else { e.1.clone() };
                self.types.insert(k.clone(), TypeInfo { kind: e.0, supers });
                if let Some((p, q)) = k.split_once('#') {
                    let leaf = q.rsplit("::").next().unwrap_or(q).to_string();
                    self.types_by_leaf.entry((p.to_string(), leaf)).or_default().push(k.clone());
                }
            }
        }
        for (k, v) in &self.holds {
            if let Some(t) = tail_key(k) {
                *self.holds_tail_n.entry(t.clone()).or_default() += 1;
                self.holds_tail.entry(t).or_default().extend(v.iter().cloned());
            }
        }
    }

    /// The canonical `types` key for `<pkg>#<path>`: the exact key, else the ONE key of that package with
    /// the same leaf (a re-exported spelling). Ambiguous or absent → `None`.
    pub(crate) fn type_key(&self, pkg: &str, path: &str) -> Option<String> {
        let exact = format!("{pkg}#{path}");
        if self.types.contains_key(&exact) {
            return Some(exact);
        }
        let leaf = path.rsplit("::").next().unwrap_or(path);
        match self.types_by_leaf.get(&(pkg.to_string(), leaf.to_string())).map(Vec::as_slice) {
            Some([one]) => Some(one.clone()),
            _ => None,
        }
    }

    /// `holds` for `<key>`: the exact key, else its tail where only ONE declaration publishes it.
    pub(crate) fn holds_of(&self, key: &str) -> Option<&BTreeSet<String>> {
        if let Some(v) = self.holds.get(key) {
            return Some(v);
        }
        let t = tail_key(key)?;
        // A tail several declarations share is ambiguous — never a pick.
        (self.holds_tail_n.get(&t) == Some(&1)).then(|| self.holds_tail.get(&t)).flatten()
    }

    pub(crate) fn kind(&self, key: &str) -> Option<&'static str> {
        self.types.get(key).and_then(|t| t.kind)
    }

    pub(crate) fn pkg_distrusted(&self, key: &str) -> bool {
        key.split_once('#').is_some_and(|(p, _)| self.distrusted.contains(p))
    }
}

/// What a walk found for one `(start, member)`.
#[derive(Debug, Default)]
pub(crate) struct Walk {
    /// Entry keys to join, and whether only the OWN (non-`interfaceUnion`) rows apply.
    pub(crate) hits: Vec<(String, bool)>,
    /// A node had no key, a KIND-ONLY key, or sits in a distrusted package.
    pub(crate) structural: bool,
}

/// THE WALK (SPEC §2 ⟨0.40⟩ "the walk reads absence as 'may be inherited'"). `present(key, own_only)`
/// asks the chained entry index. `exact` — the start is a KNOWN `value` (or `final`): past the start,
/// only a body the ancestor itself carries runs, never a sibling implementor's union row.
pub(crate) fn walk(
    idx: &SurfaceIdx,
    start: &str,
    member: &str,
    exact: bool,
    present: &dyn Fn(&str, bool) -> Option<String>,
) -> Walk {
    let mut w = Walk::default();
    let mut stack: Vec<String> = vec![start.to_string()];
    let mut seen: HashSet<String> = HashSet::new();
    while let Some(t) = stack.pop() {
        if !seen.insert(t.clone()) || seen.len() > 64 {
            continue;
        }
        if idx.pkg_distrusted(&t) {
            w.structural = true;
        }
        let own = exact && t != start;
        if let Some(k) = present(&format!("{t}::{member}"), own) {
            w.hits.push((k, own));
            continue;
        }
        // A trait another package ADDS to `t`: that package's own `impl` body for `t` (keyed under the
        // implementing package, `dep#Tok::m`) runs before the trait's default.
        let leaf = t.rsplit(['#', ':']).next().unwrap_or(&t).to_string();
        for (adder, sup) in idx.adds.get(&t).into_iter().flatten() {
            if let Some(k) = present(&format!("{adder}#{leaf}::{member}"), true) {
                w.hits.push((k, true));
            } else {
                stack.push(sup.clone());
            }
        }
        match idx.types.get(&t) {
            Some(TypeInfo { supers: Some(s), .. }) => stack.extend(s.iter().cloned()),
            _ => w.structural = true,
        }
    }
    w
}

// ═══════════════════════════════════════════════════════════════════════════════════════════════════
// THE CONSUMER'S QUESTIONS, asked from `scan.rs`. Each returns rows to join; the caller applies them
// through `apply_dep_fn`, the one sink every chained join uses.
// ═══════════════════════════════════════════════════════════════════════════════════════════════════

/// The rows under entry key `k`: every row, or — `own` — only the non-`interfaceUnion` ones.
pub(crate) fn row<'a>(idx: &'a crate::deps::DepIndex, k: &str, own: bool) -> Option<&'a crate::deps::DepFn> {
    if !own {
        return idx.by_key.get(k);
    }
    match idx.by_key_own.get(k) {
        Some(d) => Some(d),
        None if idx.union_keys.contains(k) => None, // only a union row: no body of its own
        None => idx.by_key.get(k),
    }
}

/// Is `<owner>::<member>` answered — the exact key, else its 2-segment tail (a re-exported spelling, the
/// same fallback every chained join asks)? Returns the key that answered.
pub(crate) fn present(idx: &crate::deps::DepIndex, k: &str, own: bool) -> Option<String> {
    if row(idx, k, own).is_some() {
        return Some(k.to_string());
    }
    let (p, q) = k.split_once('#')?;
    let t2 = format!("{p}#{}", crate::lang::tail2(q)?);
    row(idx, &t2, own).is_some().then_some(t2)
}

/// What a dependency VALUE receiver's provenance (`collector::dep_value_provenance`'s `<value>`/`<type>`/
/// `<field>` callee, or a factory callee) is DECLARED to be: the targets, and whether they are protocols by
/// construction (a `returnsProtocol` hit). `None` — no trusted surface answers some hop: a MISS.
pub(crate) fn receiver_targets(idx: &crate::deps::DepIndex, cr: &str, callee: &str) -> Option<(Vec<String>, bool)> {
    use crate::lazy::{FIELD_HOP_SEG, TYPE_PROV_SEG, VALUE_PROV_SEG};
    let ts = &idx.surface;
    let sep = format!("::{FIELD_HOP_SEG}::");
    let mut parts = callee.split(sep.as_str());
    let base = parts.next()?;
    let hops: Vec<&str> = parts.collect();
    let mut forced = false;
    let mut cur: BTreeSet<String> = if let Some(vp) = base.strip_prefix(&format!("{VALUE_PROV_SEG}::")) {
        match ts.holds_of(&format!("{cr}#{vp}")) {
            Some(t) => t.clone(),
            // A value path naming a TYPE the manifest knows, with no `holds` entry: a unit struct is a
            // value of its own type.
            None => std::iter::once(ts.type_key(cr, vp)?).collect(),
        }
    } else if let Some(tp) = base.strip_prefix(&format!("{TYPE_PROV_SEG}::")) {
        // Typed from the CONSUMER's own source: its key whether or not the manifest has one.
        std::iter::once(ts.type_key(cr, tp).unwrap_or_else(|| format!("{cr}#{tp}"))).collect()
    } else if !base.contains('<') {
        let k = format!("{cr}#{base}");
        if let Some(t) = idx.returns.get(&k) {
            std::iter::once(t.clone()).collect()
        } else {
            let t = ts.rproto.get(&k)?;
            forced = true;
            t.clone()
        }
    } else {
        return None;
    };
    for h in hops {
        let mut next = BTreeSet::new();
        for t in &cur {
            next.extend(ts.holds_of(&format!("{t}::{h}"))?.iter().cloned());
        }
        cur = next;
        forced = false;
    }
    (!cur.is_empty()).then(|| (cur.into_iter().collect(), forced))
}

/// Join `<start>::<member>` through the walk, plus the ⟨0.39⟩ `dispatchesOn` union of every row it
/// reaches (the join applies every surface the ordinary chained join applies). Returns the rows to
/// apply and the walk itself (the caller decides what a miss discloses).
pub(crate) fn join_through<'a>(
    idx: &'a crate::deps::DepIndex,
    start: &str,
    member: &str,
    forced_protocol: bool,
) -> (Vec<&'a crate::deps::DepFn>, Walk) {
    let kind = if forced_protocol { Some(KIND_PROTOCOL) } else { idx.surface.kind(start) };
    let exact = matches!(kind, Some("value" | "final"));
    let w = walk(&idx.surface, start, member, exact, &|k, own| present(idx, k, own));
    let mut rows: Vec<&crate::deps::DepFn> = Vec::new();
    for (k, own) in &w.hits {
        if let Some(r) = row(idx, k, *own) {
            rows.push(r);
        }
    }
    let mut pending: Vec<String> = rows.iter().flat_map(|r| r.dispatches_on.iter().cloned()).collect();
    let mut seen: HashSet<String> = pending.iter().cloned().collect();
    while let Some(m) = pending.pop() {
        let Some((o, q)) = m.split_once('#') else { continue };
        let t2 = crate::lang::tail2(q).map(|t| format!("{o}#{t}"));
        if let Some(u) = idx.by_key.get(&m).or_else(|| t2.as_ref().and_then(|k| idx.by_key.get(k))) {
            rows.push(u);
            for m2 in &u.dispatches_on {
                if seen.insert(m2.clone()) {
                    pending.push(m2.clone());
                }
            }
        }
    }
    (rows, w)
}

/// Is a trait that publishes `member` IN SCOPE in this file? A method a concrete receiver INHERITS comes
/// from a trait, and Rust resolves a trait method only through a trait in scope — so where a walk found
/// nothing, an in-scope trait carrying the member is the one place an `impl` the walk could not see (an
/// `adds` withheld, a kind-only node, an unkeyed type) can still be supplying the body. `adds` is never
/// complete (PART 95 `o10_adds_partial`).
///
/// Matched by the trait's NAME, not its path, so a re-export (`use facade::Tr` naming `dep::Tr`) and a
/// local re-export (`use crate::prelude::*`) are both seen: a `use` whose leaf is the owner's leaf, or any
/// glob. Only an owner the manifest KNOWS to be a trait counts.
pub(crate) fn in_scope_publishes(
    idx: &crate::deps::DepIndex,
    file_uses: &[String],
    renames: &HashMap<String, String>,
    member: &str,
) -> Option<String> {
    let owners = idx.members.get(member)?;
    // A glob of a CHAINED package (`use dep::prelude::*`) may bring any of its traits into scope. A
    // std glob brings none of a package's, and a LOCAL glob (`use crate::params::*`, `use super::*`) is
    // this crate's own items: measured, reading it as "every trait anywhere" hedged 584 rows of one crate
    // (`self.headers.insert(..)` beside `use crate::params::*`). A dependency trait a local module
    // re-exports and a glob then imports is the residual, stated.
    let glob = file_uses.iter().any(|u| {
        let head = u.split("::").next().unwrap_or("");
        let real = renames.get(head).map_or(head, String::as_str).replace('-', "_");
        u.ends_with('*') && idx.crates.contains(&real)
    });
    let leaves: HashSet<&str> = file_uses.iter().filter_map(|u| u.rsplit("::").next()).collect();
    owners.iter().find_map(|(pkg, o)| {
        let leaf = o.rsplit("::").next().unwrap_or(o);
        let fits = glob || leaves.contains(leaf);
        let key = format!("{pkg}#{o}");
        // A TRAIT, known as one: an owner with no `types` key may be a std or foreign type an `impl`
        // was written for (`serde_with`'s `BTreeMap::insert`), and reading that as a trait in scope
        // hedged every `map.insert` in a file importing `BTreeMap` — measured, 610 rows of one crate.
        (fits && idx.surface.kind(&key) == Some(KIND_PROTOCOL)).then_some(key)
    })
}

#[cfg(test)]
mod unit {
    use super::*;

    fn idx(types: &[(&str, &str, Option<&[&str]>)]) -> SurfaceIdx {
        let mut i = SurfaceIdx::default();
        for (k, kind, sup) in types {
            i.types.insert(
                k.to_string(),
                TypeInfo { kind: kind_of(kind), supers: sup.map(|s| s.iter().map(|x| x.to_string()).collect()) },
            );
        }
        i
    }

    #[test]
    fn a_kind_only_node_is_a_miss_never_an_empty_list() {
        // T2: PA2 + PM2, PM2 kind-only. A walk must stop at PM2 as STRUCTURAL, not settle for PA2.
        let i = idx(&[
            ("d#T2", "value", Some(&["d#PA2", "d#PM2"])),
            ("d#PA2", "protocol", Some(&[])),
            ("d#PM2", "protocol", None),
        ]);
        let have = |k: &str, _own: bool| (k == "d#PA2::m2").then(|| k.to_string());
        let w = walk(&i, "d#T2", "m2", true, &have);
        assert_eq!(w.hits.len(), 1);
        assert!(w.structural, "a KIND-ONLY key read as `[]` settles the walk short (R860's shape)");
    }

    #[test]
    fn an_unkeyed_start_is_structural_and_a_complete_walk_is_not() {
        let i = idx(&[("d#Mid4", "value", Some(&["d#Grand4"])), ("d#Grand4", "protocol", Some(&[]))]);
        let have = |k: &str, _own: bool| (k == "d#Grand4::tok4").then(|| k.to_string());
        let w = walk(&i, "d#Mid4", "tok4", true, &have);
        assert_eq!(w.hits, vec![("d#Grand4::tok4".to_string(), true)]);
        assert!(!w.structural);
        let w = walk(&i, "d#Nope", "tok4", true, &have);
        assert!(w.hits.is_empty() && w.structural);
    }

    #[test]
    fn two_copies_that_disagree_on_a_types_key_read_it_absent() {
        let mut i = SurfaceIdx::default();
        let full = serde_json::json!({"package": "d", "resolves": ["types"],
            "typeSurface": {"types": {"d#PM": {"kind": "protocol", "supers": ["d#PA"]}}}});
        let short = serde_json::json!({"package": "d", "resolves": ["types"],
            "typeSurface": {"types": {"d#PM": {"kind": "protocol"}}}});
        i.load(&full, Some("d"), true, false);
        i.load(&short, Some("d"), true, false);
        i.finish();
        assert!(!i.types.contains_key("d#PM"), "FULL in one copy and KIND-ONLY in the other is ABSENT");
        let mut j = SurfaceIdx::default();
        j.load(&full, Some("d"), true, false);
        j.load(&full, Some("d"), true, false);
        j.finish();
        assert_eq!(j.types["d#PM"].supers.as_deref(), Some(&["d#PA".to_string()][..]));
    }

    #[test]
    fn a_malformed_surface_is_absent_and_distrusts_its_package() {
        let mut i = SurfaceIdx::default();
        let bad = serde_json::json!({"package": "d", "resolves": ["holds", "types", "adds"],
            "typeSurface": {"holds": 42, "types": [], "adds": "x"}});
        i.load(&bad, Some("d"), true, false);
        i.finish();
        assert!(i.holds.is_empty() && i.types.is_empty() && i.adds.is_empty());
        assert!(i.distrusted.contains("d"));
    }

    #[test]
    fn a_key_not_listed_in_resolves_is_not_read() {
        let mut i = SurfaceIdx::default();
        let old = serde_json::json!({"package": "d", "resolves": ["fs"],
            "typeSurface": {"holds": {"d#S": "d#Other"}, "types": {"d#Other": {"kind": "value", "supers": []}}}});
        i.load(&old, Some("d"), true, false);
        i.finish();
        assert!(i.holds.is_empty() && i.types.is_empty());
    }
}
