//! SOUNDNESS R1910 / R1853 (lane rust-macropath-l2) — R-ROOT: a macro PATH whose module part is the crate ROOT binds the
//! crate's one root-namespace macro of that name, and nothing else is bound by path.
//!
//! THE QUESTION. Which definition does `crate::m!` (or `$crate::m!` transcribed from a local macro, `self::m!` at the root,
//! `super^k::m!` reaching the root) name? In every program that compiles, the crate ROOT module's macro namespace holds at
//! most one `m`, and it is one of:
//! - the crate's `#[macro_export] macro_rules! m` — written anywhere in the crate, a fn body included, or emitted by a local
//!   macro: a second export of the name is E0428 and an emitted one beside a source one is E0428 too;
//! - a root `use` of a LOCAL `macro_rules!` re-exported in its own module (`macro_rules! m {..} pub(crate) use m;` in `other`,
//!   `pub(crate) use other::m;` at the root) — E0255 keeps it exclusive with an export of the name.
//!
//! An explicit root item beats any root glob (`use dep::*;`, `pub use other::*;`), and the `#[macro_use]` prelude is not the
//! root MODULE, so neither can rebind it. That is why this binds only those two forms and REFUSES everything else: a path
//! whose module is not the root, a name defined twice, a definition under `#[cfg]`/`#[cfg_attr]` (on the item, on any
//! enclosing module, fn, impl or `mod` declaration, or the file's own `#![cfg]`), a file the module tree does not place, a
//! file two crate roots compile, and a crate whose edition the manifest does not prove to be 2018 or later (for the `use`
//! form only: a 2015 `use m;` is root-relative). A refused path stays exactly as main reads it — unexpanded item text at item
//! position (R128 / L1's hedge), main's route at body position.
//!
//! `#[macro_export(local_inner_macros)]`: inside such a definition's transcriber a bare `n!` IS `$crate::n!`, so its body is
//! stored with that rewrite applied.
//!
//! The facts are computed once per scan from SUMMARIES of every walked file (`summarize`, pure: the caller reads the files),
//! stored in the crate macro table under `F1_PREFIX` (so they join its digest and every file's cache key), and read at item
//! position by `mbe::Ctx::local_name` and at body position by the collector's `visit_macro`. `CANDOR_R_ROOT_OFF=1` computes
//! none, which restores main's reading byte for byte.
use std::collections::{BTreeMap, HashMap, HashSet};

/// The key prefix of an R-ROOT entry in `mbe::CrateMacros` and in the collector's macro table: `F1_PREFIX + root file +
/// '\u{1f}' + name`. No macro name or file path can begin with `\u{6}`.
pub(crate) const F1_PREFIX: &str = "\u{6}f1:";

/// The `use`-map key under which a file's R-ROOT scope is seeded (`seed_scope`): `root file '\u{1f}' 1|0`, the flag saying
/// whether the engine's module path for the file (`lang::module_path`) is the one the module tree places it at, which is what
/// licenses a `self::`/`super::` head (it is measured against `MODPATH_KEY`).
pub(crate) const F1_FILE_KEY: &str = "*rroot";

pub(crate) fn off() -> bool {
    static OFF: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *OFF.get_or_init(|| candor_report::env::switch("CANDOR_R_ROOT_OFF"))
}

pub(crate) fn key(root: &str, name: &str) -> String {
    format!("{F1_PREFIX}{root}\u{1f}{name}")
}

/// Any `#[cfg]` or `#[cfg_attr]` — a definition or module that may not exist in every configuration.
fn gated(attrs: &[syn::Attribute]) -> bool {
    attrs.iter().any(|a| a.path().is_ident("cfg") || a.path().is_ident("cfg_attr"))
}

/// `#[macro_export]` (possibly with arguments) or a `cfg_attr` that may add it. `(exported, local_inner_macros, conditional)`.
fn export_attr(attrs: &[syn::Attribute]) -> (bool, bool, bool) {
    let mut out = (false, false, false);
    for a in attrs {
        if a.path().is_ident("macro_export") {
            out.0 = true;
            if let syn::Meta::List(l) = &a.meta {
                if l.tokens.to_string().contains("local_inner_macros") {
                    out.1 = true;
                }
            }
        } else if a.path().is_ident("cfg_attr") && a.meta.to_token_stream_string().contains("macro_export") {
            out.0 = true;
            out.2 = true;
            if a.meta.to_token_stream_string().contains("local_inner_macros") {
                out.1 = true;
            }
        }
    }
    out
}

trait MetaText {
    fn to_token_stream_string(&self) -> String;
}
impl MetaText for syn::Meta {
    fn to_token_stream_string(&self) -> String {
        quote::ToTokens::to_token_stream(self).to_string()
    }
}

/// One exported definition.
#[derive(Clone, Debug)]
pub(crate) struct Export {
    pub name: String,
    pub body: String,
    pub gated: bool,
    pub lim: bool,
}

/// One module's scope within a file (the file itself, or an inline `mod`).
#[derive(Clone, Debug, Default)]
pub(crate) struct ModScope {
    /// the inline path below the file's own module
    pub inline: Vec<String>,
    /// a `#[cfg]`/`#[cfg_attr]` on this inline module or an enclosing one
    pub gated: bool,
    /// out-of-line `mod x;` declarations: (name, gated, has `#[path]`)
    pub decls: Vec<(String, bool, bool)>,
    /// `macro_rules! n {..}` followed (in this item list) by `use n;` / `use self::n;` — both ungated, the module holding no
    /// glob and no other `use` binding `n`: (n, body)
    pub self_reexports: Vec<(String, String)>,
    /// every `use` leaf of this item list that is not a glob: (bound name, module segments, leaf, gated, absolute)
    pub uses: Vec<(String, Vec<String>, String, bool, bool)>,
}

#[derive(Clone, Debug, Default)]
pub(crate) struct FileSummary {
    pub inner_cfg: bool,
    pub mods: Vec<ModScope>,
    pub exports: Vec<Export>,
    /// every `macro_rules!` name anywhere in the file (fn bodies and emitted definitions included), with multiplicity
    pub rules_names: Vec<String>,
}

/// Summarise one file (its source items; `spliced` is the copy `mbe::splice_local_macros` produced when some definition here
/// writes `macro_rules`, so EMITTED exports are seen too).
pub(crate) fn summarize(items: &[syn::Item], file_attrs: &[syn::Attribute], spliced: Option<&[syn::Item]>) -> FileSummary {
    let mut s = FileSummary { inner_cfg: gated(file_attrs), ..Default::default() };
    // Definitions: walk every item at every depth, the gating of every ancestor carried.
    struct V<'s> {
        depth: usize,
        s: &'s mut FileSummary,
    }
    impl V<'_> {
        fn enter(&mut self, attrs: &[syn::Attribute], extra: bool) -> bool {
            let g = gated(attrs) || extra;
            if g {
                self.depth += 1;
            }
            g
        }
    }
    impl<'ast> syn::visit::Visit<'ast> for V<'_> {
        fn visit_item(&mut self, i: &'ast syn::Item) {
            // Design revision 1 (Fable's L2 merge review D1) — an export reached through anything but a module's item list
            // (a fn body, a const / static / associated-const initializer, a closure, an array length, a discriminant, an impl
            // or trait member) is gated unless the ENCLOSING item's whole token stream is free of `cfg`: the attribute positions
            // inside an expression are many (a `#[cfg]` on a `let` in a `const _: () = {..}` read as ungated and fabricated,
            // fconst / fimplc / fstatic), and over-gating only refuses. Modules carry their own gating (`mods`), and a
            // `macro_rules!` item is the definition itself.
            let body_cfg = match i {
                syn::Item::Mod(_) | syn::Item::Macro(_) => false,
                other => quote::ToTokens::to_token_stream(other).to_string().contains("cfg"),
            };
            let g = self.enter(crate::lang::item_attrs(i), body_cfg);
            if let syn::Item::Macro(m) = i {
                if m.mac.path.is_ident("macro_rules") {
                    if let Some(id) = &m.ident {
                        let name = id.to_string();
                        self.s.rules_names.push(name.clone());
                        let (exp, lim, cond) = export_attr(&m.attrs);
                        if exp {
                            self.s.exports.push(Export { name, body: m.mac.tokens.to_string(), gated: self.depth > 0 || cond, lim });
                        }
                    }
                }
            }
            syn::visit::visit_item(self, i);
            if g {
                self.depth -= 1;
            }
        }
        fn visit_impl_item(&mut self, i: &'ast syn::ImplItem) {
            let (attrs, body_cfg): (&[syn::Attribute], bool) = match i {
                syn::ImplItem::Fn(f) => (&f.attrs, quote::ToTokens::to_token_stream(&f.block).to_string().contains("cfg")),
                syn::ImplItem::Const(c) => (&c.attrs, false),
                syn::ImplItem::Type(t) => (&t.attrs, false),
                syn::ImplItem::Macro(m) => (&m.attrs, false),
                _ => (&[], false),
            };
            let g = self.enter(attrs, body_cfg);
            syn::visit::visit_impl_item(self, i);
            if g {
                self.depth -= 1;
            }
        }
        fn visit_trait_item(&mut self, i: &'ast syn::TraitItem) {
            let (attrs, body_cfg): (&[syn::Attribute], bool) = match i {
                syn::TraitItem::Fn(f) => {
                    (&f.attrs, f.default.as_ref().is_some_and(|b| quote::ToTokens::to_token_stream(b).to_string().contains("cfg")))
                }
                syn::TraitItem::Const(c) => (&c.attrs, false),
                syn::TraitItem::Type(t) => (&t.attrs, false),
                syn::TraitItem::Macro(m) => (&m.attrs, false),
                _ => (&[], false),
            };
            let g = self.enter(attrs, body_cfg);
            syn::visit::visit_trait_item(self, i);
            if g {
                self.depth -= 1;
            }
        }
    }
    {
        let mut v = V { depth: 0, s: &mut s };
        for it in items {
            syn::visit::Visit::visit_item(&mut v, it);
        }
    }
    // Emitted definitions: only those the splice ADDED (the source ones were counted above).
    if let Some(sp) = spliced {
        let mut e = FileSummary::default();
        {
            let mut v = V { depth: 0, s: &mut e };
            for it in sp {
                syn::visit::Visit::visit_item(&mut v, it);
            }
        }
        let mut src: HashMap<String, usize> = HashMap::new();
        for n in &s.rules_names {
            *src.entry(n.clone()).or_default() += 1;
        }
        for n in e.rules_names {
            match src.get_mut(&n) {
                Some(c) if *c > 0 => *c -= 1,
                _ => s.rules_names.push(n),
            }
        }
        let mut srcx: HashMap<String, usize> = HashMap::new();
        for x in &s.exports {
            *srcx.entry(x.name.clone()).or_default() += 1;
        }
        for x in e.exports {
            match srcx.get_mut(&x.name) {
                Some(c) if *c > 0 => *c -= 1,
                _ => s.exports.push(x),
            }
        }
    }
    mods(items, Vec::new(), false, &mut s.mods);
    s
}

/// `summarize` over a parsed file, with the spliced copy when some definition in it writes `macro_rules` (emitted exports).
pub(crate) fn summarize_file(file: &syn::File) -> FileSummary {
    let spliced = if crate::mbe::defines_a_definer(&file.items) {
        let mut c = file.items.clone();
        crate::mbe::splice_local_macros(&mut c, &crate::mbe::CrateMacros::new());
        Some(c)
    } else {
        None
    };
    summarize(&file.items, &file.attrs, spliced.as_deref())
}

/// What one `Cargo.toml` says about the edition (`manifest_edition`).
pub(crate) enum Edition {
    /// nothing here (no `[package]`, or — when inheriting — no `[workspace.package]` edition): look further up
    Skip,
    /// `edition.workspace = true`: the nearest `[workspace.package]` above decides
    Inherit,
    /// decided: 2018 or later?
    Is(bool),
}

/// Read a manifest's edition: `[package]`'s, or `[workspace.package]`'s when `inheriting`. A `[package]` with no `edition` is 2015.
pub(crate) fn manifest_edition(text: &str, inheriting: bool) -> Edition {
    fn table_edition(text: &str, table: &str) -> (bool, Option<String>) {
        let (mut inside, mut seen) = (false, false);
        for line in text.lines() {
            let l = line.trim();
            if l.starts_with('[') {
                inside = l == table;
                seen |= inside;
                continue;
            }
            if inside && l.split(['=', '.']).next().unwrap_or("").trim() == "edition" {
                if l.contains("workspace") {
                    return (true, Some("workspace".to_string()));
                }
                return (true, l.split('"').nth(1).map(str::to_string));
            }
        }
        (seen, None)
    }
    let modern = |e: &str| matches!(e, "2018" | "2021" | "2024");
    if inheriting {
        return match table_edition(text, "[workspace.package]") {
            (_, Some(e)) if e != "workspace" => Edition::Is(modern(&e)),
            _ => Edition::Skip,
        };
    }
    match table_edition(text, "[package]") {
        (false, _) => Edition::Skip,
        (true, Some(e)) if e == "workspace" => Edition::Inherit,
        (true, Some(e)) => Edition::Is(modern(&e)),
        (true, None) => Edition::Is(false),
    }
}

/// The module scopes of one item list, depth-first through inline modules.
fn mods(items: &[syn::Item], inline: Vec<String>, gated_above: bool, out: &mut Vec<ModScope>) {
    let mut m = ModScope { inline: inline.clone(), gated: gated_above, ..Default::default() };
    let mut has_glob = false;
    let mut defs: Vec<(usize, String, String, bool)> = Vec::new(); // (index, name, body, gated)
    let mut leaves: Vec<(usize, String, Vec<String>, String, bool, bool)> = Vec::new();
    for (ix, it) in items.iter().enumerate() {
        match it {
            syn::Item::Mod(md) if md.content.is_none() => {
                let path_attr = md.attrs.iter().any(|a| a.path().is_ident("path"));
                m.decls.push((md.ident.to_string(), gated(&md.attrs), path_attr));
            }
            syn::Item::Macro(mm) if mm.mac.path.is_ident("macro_rules") => {
                if let Some(id) = &mm.ident {
                    defs.push((ix, id.to_string(), mm.mac.tokens.to_string(), gated(&mm.attrs)));
                }
            }
            syn::Item::Use(u) => {
                let g = gated(&u.attrs);
                let mut flat = Vec::new();
                flatten(&u.tree, Vec::new(), &mut flat, &mut has_glob);
                for (bound, mut segs, leaf) in flat {
                    let absolute = u.leading_colon.is_some();
                    if segs.first().map(String::as_str) == Some("crate") || segs.first().map(String::as_str) == Some("self") {
                        segs.remove(0);
                    }
                    leaves.push((ix, bound, segs, leaf, g, absolute));
                }
            }
            _ => {}
        }
    }
    let mut bound_count: HashMap<&str, usize> = HashMap::new();
    for l in &leaves {
        *bound_count.entry(l.1.as_str()).or_default() += 1;
    }
    if !has_glob {
        for (ix, bound, segs, leaf, g, abs) in &leaves {
            if *g || *abs || !segs.is_empty() || bound != leaf || bound_count.get(bound.as_str()) != Some(&1) {
                continue;
            }
            // the definition must PRECEDE the `use` (textual scope), be ungated, and be the only one of the name here
            let here: Vec<&(usize, String, String, bool)> = defs.iter().filter(|d| d.1 == *leaf).collect();
            if here.len() == 1 && here[0].0 < *ix && !here[0].3 {
                m.self_reexports.push((leaf.clone(), here[0].2.clone()));
            }
        }
    }
    m.uses = leaves.into_iter().map(|(_, b, s, l, g, a)| (b, s, l, g, a)).collect();
    if has_glob {
        // a glob may supply a name the module also binds: nothing here is a self re-export
        m.self_reexports.clear();
    }
    out.push(m);
    for it in items {
        if let syn::Item::Mod(md) = it {
            if let Some((_, inner)) = &md.content {
                let mut p = inline.clone();
                p.push(md.ident.to_string());
                mods(inner, p, gated_above || gated(&md.attrs), out);
            }
        }
    }
}

/// `use` tree → (bound name, path segments before the leaf, leaf). Globs set `has_glob`.
fn flatten(t: &syn::UseTree, pre: Vec<String>, out: &mut Vec<(String, Vec<String>, String)>, has_glob: &mut bool) {
    match t {
        syn::UseTree::Path(p) => {
            let mut v = pre;
            v.push(p.ident.to_string());
            flatten(&p.tree, v, out, has_glob);
        }
        syn::UseTree::Name(n) => {
            let l = n.ident.to_string();
            if l == "self" {
                // `use a::b::{self}` binds `b`, a module: never a macro
                return;
            }
            out.push((l.clone(), pre, l));
        }
        syn::UseTree::Rename(r) => {
            let l = r.ident.to_string();
            if l == "self" {
                return;
            }
            out.push((r.rename.to_string(), pre, l));
        }
        syn::UseTree::Glob(_) => *has_glob = true,
        syn::UseTree::Group(g) => {
            for x in &g.items {
                flatten(x, pre.clone(), out, has_glob);
            }
        }
    }
}

/// Is `rel` a crate ROOT file: `…/src/lib.rs`, `…/src/main.rs`, `…/src/bin/x.rs`, `…/src/bin/x/main.rs`, a direct
/// `tests/`/`benches/`/`examples/` target (`x.rs` or `x/main.rs`), or `build.rs`?
pub(crate) fn is_root_file(rel: &str) -> bool {
    let c: Vec<&str> = rel.split('/').collect();
    let n = c.len();
    let last = c[n - 1];
    if last == "build.rs" {
        return true;
    }
    if n >= 2 && c[n - 2] == "src" && (last == "lib.rs" || last == "main.rs") {
        return true;
    }
    if n >= 3 && c[n - 2] == "bin" && c[n - 3] == "src" {
        return true;
    }
    if n >= 4 && last == "main.rs" && c[n - 3] == "bin" && c[n - 4] == "src" {
        return true;
    }
    let tgt = |s: &str| matches!(s, "tests" | "benches" | "examples");
    if n >= 2 && tgt(c[n - 2]) && last.ends_with(".rs") && (n == 2 || c[n - 3] != "src") {
        return true;
    }
    if n >= 3 && last == "main.rs" && tgt(c[n - 3]) {
        return true;
    }
    false
}

fn parent_dir(rel: &str) -> String {
    match rel.rfind('/') {
        Some(i) => rel[..i].to_string(),
        None => String::new(),
    }
}

fn join(dir: &str, rest: &str) -> String {
    if dir.is_empty() {
        rest.to_string()
    } else {
        format!("{dir}/{rest}")
    }
}

/// Where a file's OUT-OF-LINE children live: a root or `mod.rs` file's own directory; `a.rs`'s `a/`.
fn child_dir(rel: &str, is_root: bool) -> String {
    let dir = parent_dir(rel);
    let stem = rel.rsplit('/').next().unwrap_or(rel).trim_end_matches(".rs");
    if is_root || stem == "mod" {
        dir
    } else {
        join(&dir, stem)
    }
}

/// A file as the module tree places it under one crate root.
#[derive(Clone, Debug)]
pub(crate) struct Placement {
    pub root: String,
    pub modpath: Vec<String>,
    pub gated: bool,
}

/// The R-ROOT facts of one scan.
#[derive(Default, Debug)]
pub(crate) struct RootFacts {
    /// root file → name → body (only a UNIQUE, ungated binding has an entry)
    pub entries: BTreeMap<String, BTreeMap<String, String>>,
    /// file → (root, module path, gated), for files exactly ONE crate root places
    pub files: HashMap<String, (String, Vec<String>, bool)>,
}

impl RootFacts {
    /// Every entry as `(key, body)`, for the crate macro table.
    pub(crate) fn table(&self) -> Vec<(String, String)> {
        let mut v = Vec::new();
        for (root, m) in &self.entries {
            for (n, b) in m {
                v.push((key(root, n), b.clone()));
            }
        }
        v
    }

    /// The scope a file's expansions bind under: `(root, Some(module path))` when the engine's module path for the file is
    /// the one the tree places it at (so a `self::`/`super::` head can be counted), `(root, None)` otherwise.
    pub(crate) fn scope(&self, rel: &str) -> Option<Scope> {
        if self.entries.is_empty() {
            return None;
        }
        let (root, mp, gated) = self.files.get(rel)?;
        if !self.entries.contains_key(root) {
            return None;
        }
        let eng = crate::lang::module_path(std::path::Path::new(rel));
        let mine = mp.join("::");
        Some(Scope { root: root.clone(), modpath: (eng == mine).then(|| mp.clone()), gated: *gated })
    }
}

/// A file's R-ROOT scope: its crate root, and its module path when that is the engine's too.
#[derive(Clone, Debug)]
pub(crate) struct Scope {
    pub root: String,
    pub modpath: Option<Vec<String>>,
    /// the file sits under a `#[cfg]`-gated module declaration (or its own `#![cfg]`): item position refuses there
    pub gated: bool,
}

impl Scope {
    /// Is a macro path's module part (`segs`, the leaf excluded) the crate root, from a module `inline` levels below the file?
    /// `crate`, or `[self] super^k` with `k` the module's depth. `$crate` arrives here already transcribed to `crate`.
    pub(crate) fn is_root(&self, segs: &[String], inline: usize) -> bool {
        match segs {
            [c] if c == "crate" => true,
            _ => {
                let Some(mp) = &self.modpath else { return false };
                let rest = match segs.first().map(String::as_str) {
                    Some("self") => &segs[1..],
                    _ => segs,
                };
                if segs.is_empty() || rest.iter().any(|s| s != "super") {
                    return false;
                }
                rest.len() == mp.len() + inline
            }
        }
    }

    /// The `use`-map value `seed_scope` plants.
    fn encode(&self) -> String {
        format!("{}\u{1f}{}", self.root, if self.modpath.is_some() { "1" } else { "0" })
    }
}

/// Seed a file's R-ROOT scope into its `use` map (Pass B), for the collector's body route.
pub(crate) fn seed_scope(scope: Option<&Scope>, uses: &mut HashMap<String, String>) {
    if let Some(s) = scope {
        uses.insert(F1_FILE_KEY.to_string(), s.encode());
    }
}

thread_local! {
    /// How many `mod` items declared INSIDE a fn body enclose the collector's walk (`BodyModScope`). `MODPATH_KEY` does not
    /// follow them, so a `self::`/`super::` head is not counted there (only `crate::` binds).
    static BODY_MOD: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

/// RAII: one body-local `mod` entered by the collector (restored on unwind too).
pub(crate) struct BodyModScope;
impl BodyModScope {
    pub(crate) fn enter() -> Self {
        BODY_MOD.with(|c| c.set(c.get() + 1));
        BodyModScope
    }
}
impl Drop for BodyModScope {
    fn drop(&mut self) {
        BODY_MOD.with(|c| c.set(c.get().saturating_sub(1)));
    }
}

/// BODY position: the R-ROOT entry key a macro path names, when its module part is the crate root of the file whose `use`
/// map this is (`seed_scope`; `MODPATH_KEY` supplies the current module, inline modules included). `None`: not a root
/// path, or no scope — the caller keeps main's route.
pub(crate) fn body_key(path: &syn::Path, uses: &HashMap<String, String>) -> Option<String> {
    if off() || path.segments.len() < 2 || path.leading_colon.is_some() {
        return None;
    }
    let v = uses.get(F1_FILE_KEY)?;
    let (root, flag) = v.split_once('\u{1f}')?;
    let segs: Vec<String> = path.segments.iter().map(|s| s.ident.to_string()).collect();
    let (leaf, head) = segs.split_last()?;
    let root_head = match head {
        [c] if c == "crate" => true,
        _ if flag == "1" && BODY_MOD.with(|c| c.get()) == 0 => {
            let mp = uses.get(crate::lang::MODPATH_KEY)?;
            let depth = if mp.is_empty() { 0 } else { mp.split("::").count() };
            let rest = if head.first().map(String::as_str) == Some("self") { &head[1..] } else { head };
            rest.iter().all(|s| s == "super") && rest.len() == depth
        }
        _ => false,
    };
    root_head.then(|| key(root, leaf))
}

/// `#[macro_export(local_inner_macros)]`: every bare `n!` in a transcriber becomes `$crate::n!` (`macro_rules!` excepted).
fn rewrite_lim(def: &str) -> Option<String> {
    use proc_macro2::{Delimiter, Group, Ident, Punct, Spacing, TokenStream, TokenTree};
    fn rw(ts: TokenStream) -> TokenStream {
        let v: Vec<TokenTree> = ts.into_iter().collect();
        let mut out: Vec<TokenTree> = Vec::with_capacity(v.len());
        // Design revision 1 (D2, flimnest) — a NESTED `macro_rules! name { .. }` the transcriber writes is copied untouched:
        // rustc applies `local_inner_macros` to the outermost expansion only, so the nested body's bare invocations resolve at
        // the nested macro's own call site.
        let mut skip_to = 0usize;
        for i in 0..v.len() {
            if i < skip_to {
                out.push(v[i].clone());
                continue;
            }
            if matches!(&v[i], TokenTree::Ident(id) if id == "macro_rules")
                && matches!(v.get(i + 1), Some(TokenTree::Punct(p)) if p.as_char() == '!')
                && matches!(v.get(i + 2), Some(TokenTree::Ident(_)))
                && matches!(v.get(i + 3), Some(TokenTree::Group(_)))
            {
                out.push(v[i].clone());
                skip_to = i + 4;
                continue;
            }
            if let TokenTree::Ident(id) = &v[i] {
                let bang = matches!(v.get(i + 1), Some(TokenTree::Punct(p)) if p.as_char() == '!');
                let after_path = i > 0 && matches!(&v[i - 1], TokenTree::Punct(p) if p.as_char() == ':' || p.as_char() == '$');
                if bang && !after_path && id != "macro_rules" {
                    let sp = id.span();
                    out.push(TokenTree::Punct(Punct::new('$', Spacing::Alone)));
                    out.push(TokenTree::Ident(Ident::new("crate", sp)));
                    out.push(TokenTree::Punct(Punct::new(':', Spacing::Joint)));
                    out.push(TokenTree::Punct(Punct::new(':', Spacing::Alone)));
                }
            }
            match &v[i] {
                TokenTree::Group(g) => {
                    let mut ng = Group::new(g.delimiter(), rw(g.stream()));
                    ng.set_span(g.span());
                    out.push(TokenTree::Group(ng));
                }
                t => out.push(t.clone()),
            }
        }
        out.into_iter().collect()
    }
    // arms: `(matcher) => {transcriber}` separated by `;` — rewrite the transcriber groups only
    let ts: TokenStream = def.parse().ok()?;
    let v: Vec<TokenTree> = ts.into_iter().collect();
    let mut out: Vec<TokenTree> = Vec::with_capacity(v.len());
    let mut i = 0;
    while i < v.len() {
        let is_arrow_body = i >= 2
            && matches!(&v[i - 1], TokenTree::Punct(p) if p.as_char() == '>')
            && matches!(&v[i - 2], TokenTree::Punct(p) if p.as_char() == '=');
        match &v[i] {
            TokenTree::Group(g) if is_arrow_body && g.delimiter() != Delimiter::None => {
                let mut ng = Group::new(g.delimiter(), rw(g.stream()));
                ng.set_span(g.span());
                out.push(TokenTree::Group(ng));
            }
            t => out.push(t.clone()),
        }
        i += 1;
    }
    Some(out.into_iter().collect::<TokenStream>().to_string())
}

/// Build the facts from every walked file's summary. `edition2018(root)` says whether that root's package is edition 2018 or
/// later (the `use` form needs it).
pub(crate) fn build(summaries: &HashMap<String, FileSummary>, edition2018: &dyn Fn(&str) -> bool) -> RootFacts {
    let mut facts = RootFacts::default();
    if off() || !summaries.values().any(|s| !s.rules_names.is_empty()) {
        return facts;
    }
    // The module tree, top-down from every root file: file → every (root, module path, gated) placing it.
    let mut placed: HashMap<String, Vec<Placement>> = HashMap::new();
    // (root, module path) → (file, index into its `mods`)
    let mut module_at: HashMap<(String, Vec<String>), (String, usize)> = HashMap::new();
    let mut roots: Vec<&String> = summaries.keys().filter(|r| is_root_file(r)).collect();
    roots.sort();
    for root in roots {
        let mut stack: Vec<(String, Vec<String>, bool)> = vec![(root.clone(), Vec::new(), false)];
        let mut seen: HashSet<String> = HashSet::new();
        while let Some((file, mp, g)) = stack.pop() {
            if !seen.insert(file.clone()) || mp.len() > 32 {
                continue;
            }
            let Some(s) = summaries.get(&file) else { continue };
            let g = g || s.inner_cfg;
            placed.entry(file.clone()).or_default().push(Placement { root: root.clone(), modpath: mp.clone(), gated: g });
            let cdir = child_dir(&file, &file == root);
            for (ix, m) in s.mods.iter().enumerate() {
                let mut mmp = mp.clone();
                mmp.extend(m.inline.iter().cloned());
                module_at.entry((root.clone(), mmp.clone())).or_insert((file.clone(), ix));
                let dir = m.inline.iter().fold(cdir.clone(), |d, x| join(&d, x));
                for (name, dg, path_attr) in &m.decls {
                    if *path_attr {
                        continue; // a `#[path]` module is not placed: its definitions bind nothing here
                    }
                    let a = join(&dir, &format!("{name}.rs"));
                    let b = join(&dir, &format!("{name}/mod.rs"));
                    let child = match (summaries.contains_key(&a), summaries.contains_key(&b)) {
                        (true, false) => a,
                        (false, true) => b,
                        _ => continue,
                    };
                    let mut cmp = mmp.clone();
                    cmp.push(name.clone());
                    stack.push((child, cmp, g || m.gated || *dg));
                }
            }
        }
    }
    // Scan-wide multiplicity of every `macro_rules!` name (the `use` form needs the target to be the only one).
    let mut rules: HashMap<&str, usize> = HashMap::new();
    for s in summaries.values() {
        for n in &s.rules_names {
            *rules.entry(n.as_str()).or_default() += 1;
        }
    }
    // Per root: exports, then the root `use` form.
    let mut exports: BTreeMap<String, BTreeMap<String, Vec<(String, bool)>>> = BTreeMap::new(); // root → name → (body, gated)
    for (file, ps) in &placed {
        let s = &summaries[file];
        for p in ps {
            let e = exports.entry(p.root.clone()).or_default();
            for x in &s.exports {
                let body = if x.lim { rewrite_lim(&x.body).unwrap_or_default() } else { x.body.clone() };
                let g = x.gated || p.gated || body.is_empty();
                e.entry(x.name.clone()).or_default().push((body, g));
            }
        }
    }
    let mut root_names: BTreeMap<String, BTreeMap<String, Option<String>>> = BTreeMap::new();
    for (root, m) in &exports {
        let r = root_names.entry(root.clone()).or_default();
        for (n, v) in m {
            r.insert(n.clone(), (v.len() == 1 && !v[0].1).then(|| v[0].0.clone()));
        }
    }
    for (root, ps) in placed.iter().filter(|(f, _)| is_root_file(f)) {
        if !ps.iter().any(|p| p.root == *root && p.modpath.is_empty()) || !edition2018(root) {
            continue;
        }
        let s = &summaries[root];
        let Some(top) = s.mods.iter().find(|m| m.inline.is_empty()) else { continue };
        let mut count: HashMap<&str, usize> = HashMap::new();
        for u in &top.uses {
            *count.entry(u.0.as_str()).or_default() += 1;
        }
        let r = root_names.entry(root.clone()).or_default();
        for (bound, segs, leaf, g, abs) in &top.uses {
            if *abs || segs.first().map(String::as_str) == Some("super") {
                continue;
            }
            // the target module, under this root, and its self re-export of `leaf`
            let Some((tf, tix)) = module_at.get(&(root.clone(), segs.clone())) else { continue };
            let Some(body) = summaries[tf].mods[*tix].self_reexports.iter().find(|(n, _)| n == leaf).map(|(_, b)| b.clone()) else {
                continue;
            };
            let target_gated = placed.get(tf).is_some_and(|v| v.iter().any(|p| p.root == *root && p.gated)) || summaries[tf].mods[*tix].gated;
            let unique = rules.get(leaf.as_str()) == Some(&1);
            let ok = !*g && !target_gated && unique && count.get(bound.as_str()) == Some(&1);
            match r.get(bound) {
                // an export of the same name beside it: E0255 in rustc; refuse
                Some(_) => {
                    r.insert(bound.clone(), None);
                }
                None => {
                    r.insert(bound.clone(), ok.then_some(body));
                }
            }
        }
    }
    for (root, m) in root_names {
        let kept: BTreeMap<String, String> = m.into_iter().filter_map(|(n, b)| b.map(|b| (n, b))).collect();
        if !kept.is_empty() {
            facts.entries.insert(root, kept);
        }
    }
    for (file, ps) in placed {
        let roots: HashSet<&String> = ps.iter().map(|p| &p.root).collect();
        if roots.len() == 1 && ps.len() == 1 {
            let p = &ps[0];
            facts.files.insert(file, (p.root.clone(), p.modpath.clone(), p.gated));
        }
    }
    facts
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sum(src: &str) -> FileSummary {
        let f: syn::File = syn::parse_str(src).unwrap();
        summarize(&f.items, &f.attrs, None)
    }

    fn facts(files: &[(&str, &str)], ed2018: bool) -> RootFacts {
        let m: HashMap<String, FileSummary> = files.iter().map(|(r, s)| (r.to_string(), sum(s))).collect();
        build(&m, &|_| ed2018)
    }

    #[test]
    fn a_unique_ungated_export_binds_and_every_gate_refuses() {
        let w = "#[macro_export] macro_rules! mk { () => { w() } }";
        let f = facts(&[("src/lib.rs", "mod other; mod inner;"), ("src/other.rs", w), ("src/inner.rs", "")], true);
        assert!(f.entries["src/lib.rs"].contains_key("mk"));
        assert_eq!(f.files["src/inner.rs"], ("src/lib.rs".to_string(), vec!["inner".to_string()], false));
        // gated on the item, on the mod declaration, on an inline ancestor, on the file, and twice-defined
        for (lib, other) in [
            ("mod other;", "#[cfg(unix)] #[macro_export] macro_rules! mk { () => {} }"),
            ("#[cfg(unix)] mod other;", w),
            ("mod other;", &*format!("mod x {{ }} #[cfg(test)] mod y {{ {w} }}")),
            ("mod other;", &*format!("#![cfg(unix)] {w}")),
            ("mod other;", &*format!("{w} mod z {{ {w} }}")),
            ("mod other;", &*format!("pub fn setup() {{ #[cfg(unix)] let _ = 1; {w} }}")),
            ("mod other;", "#[cfg_attr(unix, macro_export)] macro_rules! mk { () => {} }"),
            ("mod other;", &*format!("pub const _: () = {{ #[cfg(windows)] let _w: () = {{ {w} }}; }};")),
            ("mod other;", &*format!("pub static S: () = {{ #[cfg(windows)] let _w: () = {{ {w} }}; }};")),
            ("mod other;", &*format!("pub struct S; impl S {{ pub const C: () = {{ #[cfg(windows)] let _w: () = {{ {w} }}; }}; }}")),
        ] {
            let f = facts(&[("src/lib.rs", lib), ("src/other.rs", other)], true);
            assert!(!f.entries.get("src/lib.rs").is_some_and(|m| m.contains_key("mk")), "{lib} / {other}");
        }
        // an ungated const block binds (fconstw), as an ungated fn body does (fb)
        let f = facts(&[("src/lib.rs", "mod other;"), ("src/other.rs", &format!("pub const _: () = {{ {w} }};"))], true);
        assert!(f.entries["src/lib.rs"].contains_key("mk"));
        let f = facts(&[("src/lib.rs", "mod other;"), ("src/other.rs", &format!("pub fn setup() {{ {w} }}"))], true);
        assert!(f.entries["src/lib.rs"].contains_key("mk"));
    }

    #[test]
    fn the_use_form_needs_a_self_reexport_a_unique_name_and_edition_2018() {
        let other = "macro_rules! mk { () => { w() } } pub(crate) use mk;";
        let lib = "pub(crate) use other::{mk, mk as mkr}; mod other;";
        let f = facts(&[("src/main.rs", lib), ("src/other.rs", other)], true);
        // two root bindings of one definition: `mk` and `mkr` each bound once
        assert!(f.entries["src/main.rs"].contains_key("mk") && f.entries["src/main.rs"].contains_key("mkr"));
        assert!(facts(&[("src/main.rs", lib), ("src/other.rs", other)], false).entries.is_empty(), "2015: refused");
        // the module re-exports a DEPENDENCY's mk, a textual mk elsewhere: nothing binds
        let f = facts(
            &[("src/main.rs", "pub(crate) use other::mk; mod other; mod third;"), ("src/other.rs", "pub use dep::mk;"), ("src/third.rs", "macro_rules! mk { () => {} }")],
            true,
        );
        assert!(f.entries.is_empty());
        // a second macro_rules! mk anywhere: refused; a glob in the target module: refused; use before def: refused
        for o in [
            "macro_rules! mk { () => {} } pub(crate) use mk; mod q { macro_rules! mk { () => {} } }",
            "use dep::*; macro_rules! mk { () => {} } pub(crate) use mk;",
            "pub(crate) use mk; macro_rules! mk { () => {} }",
            "macro_rules! mk { () => {} } #[cfg(unix)] pub(crate) use mk;",
        ] {
            assert!(facts(&[("src/main.rs", "pub(crate) use other::mk; mod other;"), ("src/other.rs", o)], true).entries.is_empty(), "{o}");
        }
        // an export of the same name beside the root use: refused (E0255)
        let f = facts(&[("src/main.rs", "pub(crate) use other::mk; mod other; mod e;"), ("src/other.rs", other), ("src/e.rs", "#[macro_export] macro_rules! mk { () => {} }")], true);
        assert!(!f.entries.get("src/main.rs").is_some_and(|m| m.contains_key("mk")));
    }

    #[test]
    fn two_crate_roots_keep_their_own_exports_and_a_shared_file_has_no_scope() {
        let f = facts(
            &[("src/lib.rs", "mod other; mod both;"), ("src/main.rs", "mod inner; mod both;"), ("src/other.rs", "#[macro_export] macro_rules! mk { () => {} }"),
              ("src/inner.rs", ""), ("src/both.rs", "")],
            true,
        );
        assert!(f.entries["src/lib.rs"].contains_key("mk"));
        assert!(!f.entries.contains_key("src/main.rs"));
        assert!(f.scope("src/inner.rs").is_none(), "main.rs's root has no entry");
        assert!(!f.files.contains_key("src/both.rs"), "two roots compile it");
        assert!(f.scope("src/other.rs").is_some());
    }

    #[test]
    fn manifest_editions() {
        assert!(matches!(manifest_edition("[package]\nname = \"a\"\nedition = \"2021\"\n", false), Edition::Is(true)));
        assert!(matches!(manifest_edition("[package]\nname = \"a\"\n", false), Edition::Is(false)));
        assert!(matches!(manifest_edition("[package]\nedition.workspace = true\n", false), Edition::Inherit));
        assert!(matches!(manifest_edition("[workspace]\nmembers = []\n", false), Edition::Skip));
        assert!(matches!(manifest_edition("[workspace.package]\nedition = \"2018\"\n", true), Edition::Is(true)));
        assert!(matches!(manifest_edition("[package]\nedition = \"2015\"\n[workspace.package]\n", true), Edition::Skip));
    }

    #[test]
    fn root_heads() {
        let s = Scope { root: "src/lib.rs".into(), modpath: Some(vec!["inner".into()]), gated: false };
        let v = |x: &[&str]| x.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        assert!(s.is_root(&v(&["crate"]), 0));
        assert!(s.is_root(&v(&["super"]), 0));
        assert!(!s.is_root(&v(&["super"]), 1), "an inline module one deeper");
        assert!(s.is_root(&v(&["super", "super"]), 1));
        assert!(!s.is_root(&v(&["self"]), 0));
        assert!(!s.is_root(&v(&["crate", "a"]), 0));
        let r = Scope { root: "src/lib.rs".into(), modpath: Some(vec![]), gated: false };
        assert!(r.is_root(&v(&["self"]), 0));
        let u = Scope { root: "src/lib.rs".into(), modpath: None, gated: false };
        assert!(!u.is_root(&v(&["self"]), 0) && u.is_root(&v(&["crate"]), 0));
    }

    #[test]
    fn local_inner_macros_rewrites_bare_invocations_in_transcribers_only() {
        let f = facts(&[("src/lib.rs", "#[macro_export(local_inner_macros)] macro_rules! wrap { (a!) => { mk!(); x::y!(); $m!(); macro_rules! q { () => { nested!() } } } }")], true);
        let b = &f.entries["src/lib.rs"]["wrap"];
        assert!(b.contains("$ crate :: mk !"), "{b}");
        assert!(b.contains("(a !)"), "the matcher is untouched: {b}");
        assert!(b.contains("x :: y !") && !b.contains("$ crate :: y"), "{b}");
        assert!(!b.contains("$ crate :: macro_rules"), "{b}");
        assert!(b.contains("nested !") && !b.contains("$ crate :: nested"), "a nested definition's body is not rewritten: {b}");
    }
}
