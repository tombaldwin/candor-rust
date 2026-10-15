//! SOUNDNESS R1165 — which std traits a CHAINED dependency's callee can run on the value a consumer hands it, read
//! from the dependency's own SOURCE SIGNATURE where the consumer can see it (a `path` dependency, or the registry
//! checkout Cargo.lock names).
//!
//! The consumer-side disclosure in `scan.rs` (`witness_sites`) fires whenever a local value with an effectful std-trait
//! impl is handed to a chained dependency, because the dependency's report cannot say whether its body runs that impl.
//! This module only ever NARROWS that disclosure, so every answer it is unsure of is `None` ("any trait may run"),
//! and the narrowing is a DENYLIST of proven-inert shapes, never an allowlist:
//!
//!   * every hand-written fn of the callee's leaf anywhere in the dependency's source is a candidate (re-export paths
//!     are not resolved; the union over same-leaf fns stands in for the one that runs), and the answer is the UNION
//!     of what each candidate allows;
//!   * a candidate's parameter at the argument's position allows the std traits its generic bounds (inline, `where`,
//!     `impl Trait`, `dyn Trait`, transitively through bound arguments) imply — a bound this module does not know
//!     (a dependency's own trait, `serde::Serialize`, anything with a supertrait it cannot see) is `None`;
//!   * a parameter of a CONCRETE type (no generic, no `impl`/`dyn`) allows nothing: the consumer's type cannot be
//!     passed there;
//!   * a parameter whose binding and generic names the body never mentions (identifier or format-string capture)
//!     allows nothing;
//!   * with no fn candidate at all, a tuple struct / variant of that name is a constructor and allows nothing; and
//!     anything else — a macro-made fn, a `pub use` of the leaf (or a glob) from another crate, an item macro whose
//!     tokens name the leaf, a file that does not parse, a source that cannot be found — is `None`.
use crate::*;

/// The std-witness traits (`model::STD_WITNESS_TRAITS` leaves) a body bounded by `bound` can reach on the value.
/// `Some(empty)` for a MARKER bound that runs nothing; `None` for a bound this table does not know.
fn implied(bound: &str) -> Option<&'static [&'static str]> {
    Some(match bound {
        "Sized" | "Send" | "Sync" | "Unpin" | "Any" | "Fn" | "FnMut" | "FnOnce" | "UnwindSafe" | "RefUnwindSafe" => &[],
        "PartialEq" | "Eq" => &["PartialEq"],
        "PartialOrd" => &["PartialOrd", "PartialEq"],
        "Ord" => &["Ord", "PartialOrd", "PartialEq"],
        "Hash" => &["Hash"],
        "Hasher" => &["Hasher"],
        "Display" | "ToString" => &["Display"],
        "Debug" => &["Debug"],
        "LowerHex" => &["LowerHex"],
        "UpperHex" => &["UpperHex"],
        "Octal" => &["Octal"],
        "Binary" => &["Binary"],
        "LowerExp" => &["LowerExp"],
        "UpperExp" => &["UpperExp"],
        "Pointer" => &["Pointer"],
        "Clone" | "Copy" => &["Clone"],
        "Default" => &["Default"],
        "Iterator" => &["Iterator"],
        "DoubleEndedIterator" => &["Iterator", "DoubleEndedIterator"],
        "ExactSizeIterator" => &["Iterator", "ExactSizeIterator"],
        "IntoIterator" => &["IntoIterator", "Iterator", "DoubleEndedIterator", "ExactSizeIterator"],
        "Extend" => &["Extend"],
        "FromIterator" => &["FromIterator"],
        "Read" => &["Read"],
        "Write" => &["Write"],
        "BufRead" => &["BufRead", "Read"],
        "Seek" => &["Seek"],
        "AsRef" => &["AsRef"],
        "AsMut" => &["AsMut"],
        "Borrow" => &["Borrow"],
        "BorrowMut" => &["BorrowMut", "Borrow"],
        "Deref" => &["Deref"],
        "DerefMut" => &["DerefMut", "Deref"],
        "Into" => &["Into"],
        "TryInto" => &["TryInto"],
        "FromStr" => &["FromStr"],
        "Future" => &["Future"],
        "IntoFuture" => &["IntoFuture", "Future"], // R1181
        "ToSocketAddrs" => &["ToSocketAddrs"],
        "AsFd" => &["AsFd"],
        "AsRawFd" => &["AsRawFd"],
        "IntoRawFd" => &["IntoRawFd"],
        "AsHandle" => &["AsHandle"],
        "AsRawHandle" => &["AsRawHandle"],
        "IntoRawHandle" => &["IntoRawHandle"],
        "AsSocket" => &["AsSocket"],
        "AsRawSocket" => &["AsRawSocket"],
        "IntoRawSocket" => &["IntoRawSocket"],
        "Wake" => &["Wake"],
        "Error" => &["Error", "Display", "Debug"],
        "Index" => &["Index"],
        "IndexMut" => &["IndexMut", "Index"],
        "Add" => &["Add"],
        "Sub" => &["Sub"],
        "Mul" => &["Mul"],
        "Div" => &["Div"],
        "Rem" => &["Rem"],
        "Neg" => &["Neg"],
        "Not" => &["Not"],
        "BitAnd" => &["BitAnd"],
        "BitOr" => &["BitOr"],
        "BitXor" => &["BitXor"],
        "Shl" => &["Shl"],
        "Shr" => &["Shr"],
        "AddAssign" => &["AddAssign"],
        "SubAssign" => &["SubAssign"],
        "MulAssign" => &["MulAssign"],
        "DivAssign" => &["DivAssign"],
        "RemAssign" => &["RemAssign"],
        "BitAndAssign" => &["BitAndAssign"], // R1181
        "BitOrAssign" => &["BitOrAssign"],
        "BitXorAssign" => &["BitXorAssign"],
        "ShlAssign" => &["ShlAssign"],
        "ShrAssign" => &["ShrAssign"],
        _ => return None,
    })
}

/// Every identifier token in a token stream, and every identifier-shaped word inside its literals (a format string's
/// inline capture, `"{t}"`). Used only to ask "is this name mentioned at all", so over-collection is the safe side.
fn token_words(ts: &proc_macro2::TokenStream, out: &mut HashSet<String>) {
    for tt in ts.clone() {
        match tt {
            proc_macro2::TokenTree::Ident(i) => {
                out.insert(i.to_string());
            }
            proc_macro2::TokenTree::Group(g) => token_words(&g.stream(), out),
            proc_macro2::TokenTree::Literal(l) => lit_words(&l.to_string(), out),
            proc_macro2::TokenTree::Punct(_) => {}
        }
    }
}

fn lit_words(s: &str, out: &mut HashSet<String>) {
    for w in s.split(|c: char| !(c.is_alphanumeric() || c == '_')) {
        if !w.is_empty() {
            out.insert(w.to_string());
        }
    }
}

/// The names a body mentions — identifiers anywhere (macro arguments included) and words inside string literals.
#[derive(Default)]
struct Words(HashSet<String>);

impl<'ast> Visit<'ast> for Words {
    fn visit_ident(&mut self, i: &'ast proc_macro2::Ident) {
        self.0.insert(i.to_string());
    }
    fn visit_lit_str(&mut self, l: &'ast syn::LitStr) {
        lit_words(&l.value(), &mut self.0);
    }
    fn visit_macro(&mut self, m: &'ast syn::Macro) {
        syn::visit::visit_macro(self, m);
        token_words(&m.tokens, &mut self.0);
    }
}

/// The identifier-ish names a type mentions (path segments, generic arguments, `impl`/`dyn` bound paths).
#[derive(Default)]
struct TyNames(HashSet<String>);

impl<'ast> Visit<'ast> for TyNames {
    fn visit_ident(&mut self, i: &'ast proc_macro2::Ident) {
        self.0.insert(i.to_string());
    }
}

/// The trait leaves of a bound list; `?Sized` and lifetimes are dropped (they run nothing).
fn bound_leaves<'a>(bounds: impl Iterator<Item = &'a syn::TypeParamBound>) -> Vec<String> {
    bounds
        .filter_map(|b| match b {
            syn::TypeParamBound::Trait(t) if matches!(t.modifier, syn::TraitBoundModifier::None) => {
                t.path.segments.last().map(|s| s.ident.to_string())
            }
            _ => None,
        })
        .collect()
}

/// One fn-like item of the dependency: its generics in scope (impl/trait + own), its inputs, and its body if any.
struct Cand<'a> {
    scope: Vec<&'a syn::Generics>,
    inputs: Vec<&'a syn::FnArg>,
    body: Option<&'a syn::Block>,
}

#[derive(Default)]
struct Collect<'a> {
    leaf: String,
    stack: Vec<&'a syn::Generics>,
    fns: Vec<Cand<'a>>,
    ctor: bool,
    /// A `pub use` that may bring the leaf in from ANOTHER crate (the leaf by name, or a glob), or an item macro whose
    /// tokens name it: the definition that runs may not be in this source, so nothing can be narrowed.
    opaque: bool,
}

impl<'a> Collect<'a> {
    fn sig(&mut self, sig: &'a syn::Signature, body: Option<&'a syn::Block>) {
        if sig.ident == self.leaf.as_str() {
            let mut scope = self.stack.clone();
            scope.push(&sig.generics);
            self.fns.push(Cand { scope, inputs: sig.inputs.iter().collect(), body });
        }
    }
}

impl<'a> Visit<'a> for Collect<'a> {
    fn visit_item_fn(&mut self, f: &'a syn::ItemFn) {
        self.sig(&f.sig, Some(&f.block));
        syn::visit::visit_item_fn(self, f);
    }
    fn visit_item_impl(&mut self, i: &'a syn::ItemImpl) {
        self.stack.push(&i.generics);
        for it in &i.items {
            if let syn::ImplItem::Fn(f) = it {
                self.sig(&f.sig, Some(&f.block));
            }
        }
        syn::visit::visit_item_impl(self, i);
        self.stack.pop();
    }
    fn visit_impl_item_fn(&mut self, f: &'a syn::ImplItemFn) {
        // the signature was taken by `visit_item_impl` with the impl's generics in scope; walk the body for nested items
        syn::visit::visit_block(self, &f.block);
    }
    fn visit_item_trait(&mut self, t: &'a syn::ItemTrait) {
        self.stack.push(&t.generics);
        for it in &t.items {
            if let syn::TraitItem::Fn(f) = it {
                self.sig(&f.sig, f.default.as_ref());
            }
        }
        for it in &t.items {
            if let syn::TraitItem::Fn(f) = it {
                if let Some(b) = &f.default {
                    syn::visit::visit_block(self, b);
                }
            }
        }
        self.stack.pop();
    }
    fn visit_item_struct(&mut self, s: &'a syn::ItemStruct) {
        if s.ident == self.leaf.as_str() && matches!(s.fields, syn::Fields::Unnamed(_)) {
            self.ctor = true;
        }
    }
    fn visit_variant(&mut self, v: &'a syn::Variant) {
        if v.ident == self.leaf.as_str() {
            self.ctor = true;
        }
    }
    fn visit_item_use(&mut self, u: &'a syn::ItemUse) {
        if !matches!(u.vis, syn::Visibility::Inherited) && use_may_import(&u.tree, &self.leaf, None) {
            self.opaque = true;
        }
    }
    fn visit_item_macro(&mut self, m: &'a syn::ItemMacro) {
        let mut w = HashSet::new();
        token_words(&m.mac.tokens, &mut w);
        if w.contains(&self.leaf) || m.ident.is_some() && w.contains("fn") {
            // a `macro_rules!` that defines fns, or an invocation naming the leaf, may make the fn that runs
            self.opaque = true;
        }
    }
}

/// Does this `use` tree bring `leaf` (or a glob) in from a path rooted outside the crate?
fn use_may_import(t: &syn::UseTree, leaf: &str, root: Option<&str>) -> bool {
    let local = |r: Option<&str>| matches!(r, Some("crate" | "self" | "super"));
    match t {
        syn::UseTree::Path(p) => {
            let r = root.unwrap_or(&p.ident.to_string()).to_string();
            use_may_import(&p.tree, leaf, Some(&r))
        }
        syn::UseTree::Name(n) => n.ident == leaf && !local(root),
        syn::UseTree::Rename(r) => r.rename == leaf && !local(root),
        syn::UseTree::Glob(_) => !local(root),
        syn::UseTree::Group(g) => g.items.iter().any(|i| use_may_import(i, leaf, root)),
    }
}

/// What one candidate allows at input `pos`: `Some(set)` (possibly empty) or `None` (any). `Err(())` when the
/// candidate has no input at `pos` (it is not the callee of a call with that many arguments).
fn cand_allows(c: &Cand<'_>, pos: usize) -> Result<Option<BTreeSet<String>>, ()> {
    let arg = c.inputs.get(pos).ok_or(())?;
    let syn::FnArg::Typed(pt) = arg else { return Ok(None) }; // the receiver: the consumer's type implements this trait
    let generic_names: HashSet<String> = c
        .scope
        .iter()
        .flat_map(|g| g.params.iter())
        .filter_map(|p| match p {
            syn::GenericParam::Type(t) => Some(t.ident.to_string()),
            _ => None,
        })
        .collect();
    let mut tn = TyNames::default();
    tn.visit_type(&pt.ty);
    if tn.0.contains("Self") {
        return Ok(None);
    }
    // The bounds reachable from the parameter's type, closed over generics their own arguments name.
    let mut leaves: Vec<String> = Vec::new();
    let mut seen: HashSet<String> = HashSet::new();
    let mut work: Vec<String> = tn.0.iter().filter(|n| generic_names.contains(*n)).cloned().collect();
    // `impl Trait` / `dyn Trait` in the parameter's own type.
    struct Inline<'b>(&'b mut Vec<String>, &'b mut Vec<String>, &'b HashSet<String>);
    impl<'ast> Visit<'ast> for Inline<'_> {
        fn visit_type_impl_trait(&mut self, t: &'ast syn::TypeImplTrait) {
            self.0.extend(bound_leaves(t.bounds.iter()));
            let mut n = TyNames::default();
            for b in &t.bounds {
                n.visit_type_param_bound(b);
            }
            self.1.extend(n.0.into_iter().filter(|x| self.2.contains(x)));
        }
        fn visit_type_trait_object(&mut self, t: &'ast syn::TypeTraitObject) {
            self.0.extend(bound_leaves(t.bounds.iter()));
            let mut n = TyNames::default();
            for b in &t.bounds {
                n.visit_type_param_bound(b);
            }
            self.1.extend(n.0.into_iter().filter(|x| self.2.contains(x)));
        }
    }
    let had_inline = {
        let before = leaves.len();
        Inline(&mut leaves, &mut work, &generic_names).visit_type(&pt.ty);
        leaves.len() != before
    };
    if work.is_empty() && !had_inline {
        return Ok(Some(BTreeSet::new())); // a concrete type: the consumer's value cannot be passed here
    }
    let mentioned_generics: Vec<String> = work.clone();
    while let Some(g) = work.pop() {
        if !seen.insert(g.clone()) {
            continue;
        }
        for gens in &c.scope {
            for p in &gens.params {
                if let syn::GenericParam::Type(t) = p {
                    if t.ident == g.as_str() {
                        leaves.extend(bound_leaves(t.bounds.iter()));
                        let mut n = TyNames::default();
                        for b in &t.bounds {
                            n.visit_type_param_bound(b);
                        }
                        work.extend(n.0.into_iter().filter(|x| generic_names.contains(x) && !seen.contains(x)));
                    }
                }
            }
            if let Some(w) = &gens.where_clause {
                for pred in &w.predicates {
                    if let syn::WherePredicate::Type(pt2) = pred {
                        let mut bn = TyNames::default();
                        bn.visit_type(&pt2.bounded_ty);
                        if bn.0.contains(&g) {
                            leaves.extend(bound_leaves(pt2.bounds.iter()));
                            let mut n = TyNames::default();
                            for b in &pt2.bounds {
                                n.visit_type_param_bound(b);
                            }
                            n.0.extend(bn.0);
                            work.extend(n.0.into_iter().filter(|x| generic_names.contains(x) && !seen.contains(x)));
                        }
                    }
                }
            }
        }
    }
    let mut out = BTreeSet::new();
    for l in &leaves {
        match implied(l) {
            Some(ts) => out.extend(ts.iter().map(|t| t.to_string())),
            None => return Ok(None),
        }
    }
    // A parameter neither whose binding nor whose generic names the body mentions cannot have a member run on it.
    if !out.is_empty() {
        if let Some(b) = c.body {
            let mut w = Words::default();
            w.visit_block(b);
            let mut binds = TyNames::default();
            binds.visit_pat(&pt.pat);
            let used = binds.0.iter().any(|n| w.0.contains(n)) || mentioned_generics.iter().any(|g| w.0.contains(g));
            if !used {
                return Ok(Some(BTreeSet::new()));
            }
        }
    }
    Ok(Some(out))
}

/// Per-scan cache of parsed dependency sources, keyed by the source directory.
#[derive(Default)]
pub(crate) struct DepSigs {
    files: HashMap<std::path::PathBuf, Option<Vec<syn::File>>>,
    dirs: HashMap<String, Vec<std::path::PathBuf>>,
}

impl DepSigs {
    /// The source directories of the dependency the consumer at `dir` spells `alias` (real package `real`): a manifest
    /// `path` (own manifest, then the workspace root's `[workspace.dependencies]`), else every registry checkout the
    /// lockfile names. Empty when none is found.
    fn source_dirs(&mut self, dir: &str, alias: &str, real: &str) -> Vec<std::path::PathBuf> {
        if let Some(v) = self.dirs.get(alias) {
            return v.clone();
        }
        let norm = |s: &str| s.replace('-', "_");
        let mut out: Vec<std::path::PathBuf> = Vec::new();
        let root = crate::deps::find_workspace_root(Path::new(dir));
        let mut manifests: Vec<std::path::PathBuf> = vec![Path::new(dir).to_path_buf()];
        if let Some(r) = &root {
            manifests.push(r.clone());
        }
        let mut reqs: Vec<String> = Vec::new();
        'm: for m in &manifests {
            let Ok(text) = std::fs::read_to_string(m.join("Cargo.toml")) else { continue };
            let Ok(doc) = text.parse::<toml::Table>() else { continue };
            for table in crate::deps::manifest_dependency_tables(&doc) {
                for (k, v) in table {
                    if norm(k) != alias {
                        continue;
                    }
                    if let Some(p) = v.as_table().and_then(|t| t.get("path")).and_then(|p| p.as_str()) {
                        out.push(m.join(p));
                        break 'm;
                    }
                    if let Some(r) = v.as_str().or_else(|| v.as_table().and_then(|t| t.get("version")).and_then(|r| r.as_str())) {
                        reqs.push(r.to_string());
                    }
                }
            }
        }
        if out.is_empty() {
            let regs = crate::deps::dirs_cargo_registry_src();
            let mut lock_dirs = vec![Path::new(dir).to_path_buf()];
            if let Some(r) = &root {
                lock_dirs.push(r.clone());
            }
            for ld in lock_dirs {
                let Ok(lock) = std::fs::read_to_string(ld.join("Cargo.lock")) else { continue };
                let mut name = String::new();
                for line in lock.lines() {
                    let l = line.trim();
                    if l.starts_with('[') {
                        name.clear();
                    } else if let Some(v) = l.strip_prefix("name = ") {
                        name = v.trim_matches('"').to_string();
                    } else if let Some(v) = l.strip_prefix("version = ") {
                        if norm(&name) == real {
                            let v = v.trim_matches('"');
                            if let Some(p) = regs.iter().map(|r| r.join(format!("{name}-{v}"))).find(|p| p.is_dir()) {
                                if !out.contains(&p) {
                                    out.push(p);
                                }
                            }
                        }
                    }
                }
                if !out.is_empty() {
                    break;
                }
            }
        }
        // No lockfile entry (a library tree, a registry checkout): every registry checkout of the package that is
        // semver-compatible with a manifest requirement — a UNION, so the narrowing must hold for each of them.
        if out.is_empty() && !reqs.is_empty() {
            let compat = |v: &[u64]| -> Vec<u64> {
                match v {
                    [0, 0, p, ..] => vec![0, 0, *p],
                    [0, m, ..] => vec![0, *m],
                    [x, ..] => vec![*x],
                    _ => vec![],
                }
            };
            let nums = |s: &str| -> Vec<u64> {
                let s = s.trim_start_matches(|c: char| "^=~> ".contains(c));
                let core = s.split(['-', '+', ',', ' ']).next().unwrap_or("");
                let mut v: Vec<u64> = core.split('.').map_while(|x| x.parse().ok()).collect();
                v.resize(3, 0);
                v
            };
            for r in &reqs {
                if r.contains('*') || r.contains('<') || r.contains(',') {
                    continue; // a range this reading cannot bound: no narrowing source from it
                }
                let want = compat(&nums(r));
                for reg in crate::deps::dirs_cargo_registry_src() {
                    for e in std::fs::read_dir(&reg).into_iter().flatten().flatten() {
                        let n = e.file_name().to_string_lossy().to_string();
                        let Some((name, ver)) = n.rsplit_once('-').filter(|(_, v)| v.chars().next().is_some_and(|c| c.is_ascii_digit())) else { continue };
                        if norm(name) == real && !ver.contains('-') && compat(&nums(ver)) == want && !out.contains(&e.path()) {
                            out.push(e.path());
                        }
                    }
                }
            }
        }
        self.dirs.insert(alias.to_string(), out.clone());
        out
    }

    fn parsed(&mut self, src: &Path) -> Option<&Vec<syn::File>> {
        if !self.files.contains_key(src) {
            let mut files = Vec::new();
            let mut ok = true;
            let walk = walkdir::WalkDir::new(src).into_iter().filter_entry(|e| {
                let n = e.file_name().to_string_lossy();
                e.depth() == 0 || !(n.starts_with('.') || n == "target" || n == "tests" || n == "benches" || n == "examples")
            });
            for e in walk.flatten() {
                if e.file_type().is_file() && e.path().extension().is_some_and(|x| x == "rs") {
                    match std::fs::read_to_string(e.path()).ok().and_then(|t| syn::parse_file(&t).ok()) {
                        Some(f) => files.push(f),
                        None => {
                            ok = false;
                            break;
                        }
                    }
                }
            }
            self.files.insert(src.to_path_buf(), (ok && !files.is_empty()).then_some(files));
        }
        self.files.get(src).and_then(|o| o.as_ref())
    }

    /// The std-witness traits the dependency's callee (`callee`, the path after the crate) can run on the value at
    /// argument position `pos` (the receiver is 0 for a method). `None` = cannot narrow.
    pub(crate) fn allowed(&mut self, dir: &str, alias: &str, real: &str, callee: &str, pos: usize) -> Option<BTreeSet<String>> {
        let leaf = callee.rsplit("::").next().unwrap_or(callee).to_string();
        let srcs = self.source_dirs(dir, alias, real);
        if srcs.is_empty() {
            return None;
        }
        let mut out = BTreeSet::new();
        for s in srcs {
            let files = self.parsed(&s)?;
            let mut c = Collect { leaf: leaf.clone(), ..Default::default() };
            for f in files {
                c.visit_file(f);
            }
            if c.opaque {
                return None;
            }
            let mut fitted = 0usize;
            for cand in &c.fns {
                match cand_allows(cand, pos) {
                    Ok(Some(set)) => {
                        fitted += 1;
                        out.extend(set);
                    }
                    Ok(None) => return None,
                    Err(()) => {}
                }
            }
            if fitted == 0 && !(c.fns.is_empty() && c.ctor) {
                return None;
            }
        }
        Some(out)
    }
}
