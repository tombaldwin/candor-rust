//! SOUNDNESS R1627 — IS THE INHERENT METHOD THE ONE THAT RUNS? A proof, over the CONSUMER, that no trait can supply a competitor.
//!
//! R1578 types a dependency value through the lock-pinned source, and a METHOD step (`b.build()`) only where Rust's method probe
//! provably reaches the dependency's INHERENT method before any trait method (R1200): an inherent method wins only against a trait
//! method found at the SAME probe step, so a by-value `fn build(self)` of any trait in scope at the consumer (a blanket
//! `impl<T> Ext for T` in a third crate, never in this source) beats an inherent `fn build(&self)` on an owned receiver. That is
//! why `ExponentialBackoffBuilder::new().build()` (owned receiver, `&self` method) refused to type, and the chain read as its root.
//!
//! The probe only matters when a trait method of that NAME is in scope. So the refusal can be lifted by a proof about the
//! consumer, not about the dependency: NO TRAIT IN SCOPE AT ANY CALL SITE OF THIS CRATE DECLARES A METHOD NAMED `m`. A trait is in
//! scope by the std prelude, a `use`, a glob, a declaration in the crate, a macro, or a bound the path names. Each is closed here:
//!
//!   * a name no std trait has (`is_std_method_name`) is in no prelude and no `use std::..`;
//!   * every `use` of the crate (any file, any depth) names a module, a free fn, or a type that is not a `trait` (read from the
//!     lock-pinned source, `strict_not_trait`) — or a trait whose declaration, supertraits included, lacks `m` (`trait_declares`);
//!     a glob from outside the crate, a leaf this reading cannot place, a crate with no lock-pinned source: refuse;
//!   * no trait declared in the crate declares `m`, and no local `macro_rules!` body can write a trait, a `use`, or a `m`;
//!   * no item-position or statement-position macro other than an inert std macro or such a local one (it may expand to a `use`);
//!   * no attribute this reading does not know, and no non-std `derive` (an attribute macro may emit any item);
//!   * no trait bound spelled by a path that leaves the crate (a where-clause bound brings its trait's methods into scope for its
//!     type without any `use`).
//!
//! Every doubt is `Err(reason)` and the caller keeps R1200's form rule. `CANDOR_R1627_OFF` disables it, for falsification.
use super::*;

/// A `use` leaf, flattened: the path segments, and what the leaf is.
#[derive(Debug)]
enum UseLeaf {
    Name(Vec<String>),
    Glob(Vec<String>),
}

#[derive(Default)]
struct Facts {
    uses: Vec<UseLeaf>,
    /// every name an item of the crate declares (a module, a type, a trait, a fn, a macro)
    local: HashSet<String>,
    why: Option<String>,
    m: String,
    /// local `macro_rules!` names whose bodies are known not to write a trait, a `use`, a nested macro call or `m`
    safe_macros: HashSet<String>,
}

impl Facts {
    fn doubt(&mut self, why: String) {
        if self.why.is_none() {
            self.why = Some(why);
        }
    }

    fn flatten(&mut self, t: &syn::UseTree, prefix: &mut Vec<String>) {
        match t {
            syn::UseTree::Path(p) => {
                prefix.push(p.ident.to_string());
                self.flatten(&p.tree, prefix);
                prefix.pop();
            }
            syn::UseTree::Name(n) => {
                // `use a::b::{self}` binds the module `a::b`; a module brings no trait
                if n.ident != "self" {
                    let mut v = prefix.clone();
                    v.push(n.ident.to_string());
                    self.uses.push(UseLeaf::Name(v));
                }
            }
            syn::UseTree::Rename(r) => {
                if r.ident != "self" {
                    let mut v = prefix.clone();
                    v.push(r.ident.to_string());
                    self.uses.push(UseLeaf::Name(v));
                }
            }
            syn::UseTree::Glob(_) => self.uses.push(UseLeaf::Glob(prefix.clone())),
            syn::UseTree::Group(g) => g.items.iter().for_each(|i| self.flatten(i, prefix)),
        }
    }

    fn attr(&mut self, a: &syn::Attribute) {
        let segs: Vec<String> = a.path().segments.iter().map(|s| s.ident.to_string()).collect();
        let head = segs.first().cloned().unwrap_or_default();
        if segs.len() > 1 && !matches!(head.as_str(), "rustfmt" | "clippy" | "diagnostic") || !INERT.contains(&head.as_str()) {
            self.doubt(format!("attr:{}", segs.join("::")));
        } else if head == "derive" {
            let toks = a.meta.require_list().map(|l| l.tokens.to_string()).unwrap_or_default();
            if let Some(w) = toks
                .split(|c: char| !(c.is_alphanumeric() || c == '_'))
                .filter(|w| !w.is_empty())
                .find(|w| !STD_DERIVES.contains(w) && !matches!(*w, "core" | "std" | "fmt" | "cmp" | "hash" | "clone" | "default" | "marker"))
            {
                self.doubt(format!("derive:{w}"));
            }
        } else if matches!(head.as_str(), "cfg_attr" | "path") {
            self.doubt(format!("attr:{head}"));
        }
    }

    /// May the tokens of a `macro_rules!` body write an item (a trait, a `use`), name the method, or call another macro?
    fn body_unsafe(ts: &proc_macro2::TokenStream, m: &str) -> bool {
        use proc_macro2::TokenTree as T;
        let toks: Vec<T> = ts.clone().into_iter().collect();
        for (i, t) in toks.iter().enumerate() {
            match t {
                T::Ident(id) if id == "trait" || id == "use" || id == "macro_rules" || *id == m => return true,
                T::Punct(p) if p.as_char() == '!' && i > 0 && matches!(toks[i - 1], T::Ident(_)) => return true,
                // a `$name` fragment can stand for any identifier, including a `trait` name or the method
                T::Punct(p) if p.as_char() == '$' && matches!(toks.get(i + 1), Some(T::Ident(_))) => {
                    if matches!(toks.get(i + 2), Some(T::Punct(c)) if c.as_char() == ':')
                        && matches!(toks.get(i + 3), Some(T::Ident(k)) if k == "ident" || k == "item" || k == "tt" || k == "path" || k == "meta" || k == "vis")
                    {
                        return true;
                    }
                }
                T::Group(g) if Self::body_unsafe(&g.stream(), m) => return true,
                _ => {}
            }
        }
        false
    }

    fn body_writes_scope(ts: &proc_macro2::TokenStream, m: &str) -> bool {
        use proc_macro2::TokenTree as T;
        ts.clone().into_iter().any(|t| match t {
            T::Ident(id) => id == "trait" || id == "use" || id == m,
            T::Group(g) => Self::body_writes_scope(&g.stream(), m),
            _ => false,
        })
    }

    fn macro_ok(&self, mac: &syn::Macro) -> bool {
        let segs: Vec<String> = mac.path.segments.iter().map(|s| s.ident.to_string()).collect();
        let last = segs.last().map(String::as_str).unwrap_or("");
        match segs.len() {
            1 => self.safe_macros.contains(last) || (INERT_STD_MACROS.contains(&last) && !self.local.contains(last)),
            _ => matches!(segs[0].as_str(), "std" | "core" | "alloc") && INERT_STD_MACROS.contains(&last),
        }
    }
}

impl<'ast> Visit<'ast> for Facts {
    fn visit_item(&mut self, i: &'ast syn::Item) {
        let (name, attrs): (Option<String>, &[syn::Attribute]) = match i {
            syn::Item::Fn(x) => (Some(x.sig.ident.to_string()), &x.attrs),
            syn::Item::Struct(x) => (Some(x.ident.to_string()), &x.attrs),
            syn::Item::Enum(x) => (Some(x.ident.to_string()), &x.attrs),
            syn::Item::Union(x) => (Some(x.ident.to_string()), &x.attrs),
            syn::Item::Trait(x) => (Some(x.ident.to_string()), &x.attrs),
            syn::Item::TraitAlias(x) => (Some(x.ident.to_string()), &x.attrs),
            syn::Item::Type(x) => (Some(x.ident.to_string()), &x.attrs),
            syn::Item::Mod(x) => (Some(x.ident.to_string()), &x.attrs),
            syn::Item::Const(x) => (Some(x.ident.to_string()), &x.attrs),
            syn::Item::Static(x) => (Some(x.ident.to_string()), &x.attrs),
            syn::Item::Impl(x) => (None, &x.attrs),
            syn::Item::Use(x) => (None, &x.attrs),
            syn::Item::ExternCrate(x) => (Some(x.rename.as_ref().map(|r| r.1.to_string()).unwrap_or_else(|| x.ident.to_string())), &x.attrs),
            syn::Item::Macro(x) => (x.ident.as_ref().map(|i| i.to_string()), &x.attrs),
            _ => (None, &[]),
        };
        // an `extern crate dep as d;` makes `d::Ext` a path this reading would take for a local module's
        if let syn::Item::ExternCrate(x) = i {
            if !matches!(x.ident.to_string().as_str(), "std" | "core" | "alloc" | "self") {
                self.doubt(format!("externcrate:{}", x.ident));
            }
        }
        if let Some(n) = name {
            self.local.insert(n);
        }
        for a in attrs {
            self.attr(a);
        }
        syn::visit::visit_item(self, i);
    }
    fn visit_item_use(&mut self, u: &'ast syn::ItemUse) {
        let mut p = Vec::new();
        self.flatten(&u.tree, &mut p);
    }
    fn visit_item_trait(&mut self, t: &'ast syn::ItemTrait) {
        for it in &t.items {
            match it {
                syn::TraitItem::Fn(f) if f.sig.ident == self.m.as_str() => self.doubt("localtrait".into()),
                syn::TraitItem::Verbatim(_) | syn::TraitItem::Macro(_) => self.doubt("traitmacro".into()),
                _ => {}
            }
        }
        syn::visit::visit_item_trait(self, t);
    }
    fn visit_item_macro(&mut self, im: &'ast syn::ItemMacro) {
        if im.mac.path.is_ident("macro_rules") && im.ident.is_some() {
            // a body that writes a `use` or a trait, or names the method, may put a trait in scope around the tokens it is handed
            // (`($e:expr) => {{ use dep::Ext; $e }}`) at ANY invocation, expression position included
            if Self::body_writes_scope(&im.mac.tokens, &self.m) {
                self.doubt(format!("macrobody:{}", im.ident.as_ref().map(|i| i.to_string()).unwrap_or_default()));
            }
            if !Self::body_unsafe(&im.mac.tokens, &self.m) {
                if let Some(n) = &im.ident {
                    self.safe_macros.insert(n.to_string());
                }
            }
        } else if !self.macro_ok(&im.mac) {
            self.doubt(format!("itemmacro:{}", im.mac.path.segments.last().map(|s| s.ident.to_string()).unwrap_or_default()));
        }
    }
    fn visit_stmt(&mut self, s: &'ast syn::Stmt) {
        if let syn::Stmt::Macro(sm) = s {
            if !self.macro_ok(&sm.mac) {
                self.doubt(format!("stmtmacro:{}", sm.mac.path.segments.last().map(|s| s.ident.to_string()).unwrap_or_default()));
            }
        }
        syn::visit::visit_stmt(self, s);
    }
    fn visit_trait_bound(&mut self, b: &'ast syn::TraitBound) {
        let segs: Vec<String> = b.path.segments.iter().map(|s| s.ident.to_string()).collect();
        if segs.len() > 1 && !matches!(segs[0].as_str(), "crate" | "self" | "super" | "std" | "core" | "alloc") {
            self.doubt(format!("boundpath:{}", segs.join("::")));
        }
        syn::visit::visit_trait_bound(self, b);
    }
    fn visit_attribute(&mut self, a: &'ast syn::Attribute) {
        self.attr(a);
    }
}

impl DepSigs {
    /// `Ok(())` — PROVEN: no trait in scope at any call site of the crate at `dir` declares a method named `m` (module doc).
    /// `deps` / `renames` are the consumer's declared dependency names and their `package = ` renames. `Err(reason)`: unproven.
    pub(crate) fn probe_clear(&mut self, dir: &str, m: &str, deps: &HashSet<String>, renames: &HashMap<String, String>) -> Result<(), String> {
        if candor_report::env::switch("CANDOR_R1627_OFF") {
            return Err("off".into());
        }
        let key = (dir.to_string(), m.to_string());
        if let Some(v) = self.probes.get(&key) {
            return v.clone();
        }
        let v = self.probe_uncached(dir, m, deps, renames);
        if candor_report::env::trace("CANDOR_R1627_INSTR") {
            eprintln!("R1627PROBE\t{dir}\t{m}\t{v:?}"); // §E1 REACH COUNTER
        }
        self.probes.insert(key, v.clone());
        v
    }

    fn probe_uncached(&mut self, dir: &str, m: &str, deps: &HashSet<String>, renames: &HashMap<String, String>) -> Result<(), String> {
        if crate::std_methods::is_std_method_name(m) {
            return Err("stdname".into());
        }
        let root = Path::new(dir);
        let mut facts = Facts { m: m.to_string(), ..Default::default() };
        let walk = walkdir::WalkDir::new(root).into_iter().filter_entry(|e| {
            let n = e.file_name().to_string_lossy();
            if e.depth() == 0 {
                return true;
            }
            if e.file_type().is_dir() {
                // a nested package is another crate; `target` and dot-directories hold no source of this one
                return !(n.starts_with('.') || n == "target") && !e.path().join("Cargo.toml").is_file();
            }
            true
        });
        let mut files = 0usize;
        for e in walk.flatten() {
            if e.file_type().is_file() && e.path().extension().is_some_and(|x| x == "rs") {
                let text = std::fs::read_to_string(e.path()).map_err(|_| "unreadable".to_string())?;
                let f = syn::parse_file(&text).map_err(|_| "unparsed".to_string())?;
                files += 1;
                for a in &f.attrs {
                    facts.attr(a);
                }
                facts.visit_file(&f);
            }
        }
        if files == 0 {
            return Err("nofiles".into());
        }
        if let Some(w) = facts.why.take() {
            return Err(w);
        }
        for r in ["std", "core", "alloc"] {
            if facts.local.contains(r) {
                return Err(format!("rebound:{r}"));
            }
        }
        let uses = std::mem::take(&mut facts.uses);
        for u in uses {
            let (segs, glob) = match &u {
                UseLeaf::Name(s) => (s, false),
                UseLeaf::Glob(s) => (s, true),
            };
            let Some(first) = segs.first() else { continue };
            match first.as_str() {
                "crate" | "self" | "super" | "std" | "core" | "alloc" => continue,
                _ => {}
            }
            if deps.contains(first) || renames.contains_key(first) {
                if facts.local.contains(first) {
                    return Err(format!("shadow:{first}"));
                }
                if glob {
                    return Err(format!("extglob:{}", segs.join("::")));
                }
                let real = renames.get(first).cloned().unwrap_or_else(|| first.clone());
                let rest = &segs[1..];
                if rest.is_empty() {
                    continue; // `use dep;` / `use dep as d;` — the crate itself, a module
                }
                let src = self.lock_source(dir, first, &real).ok_or_else(|| format!("nolock:{first}"))?;
                // a trait whose declaration (supertraits included) lacks the method brings none; else the leaf must not be a trait
                let leaf = rest.last().cloned().unwrap_or_default();
                if self.trait_declares(dir, first, &real, &leaf, m) == Some(false) {
                    continue;
                }
                self.strict_not_trait(&src.dir, rest).map_err(|e| format!("use:{}:{e}", segs.join("::")))?;
            } else if facts.local.contains(first) {
                continue; // a module of this crate; what it re-exports is another `use` of the crate, read in its own right
            } else {
                return Err(format!("root:{first}"));
            }
        }
        Ok(())
    }
}
