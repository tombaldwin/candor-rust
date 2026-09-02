//! Pass A over a parsed file: enumerate the functions (`scan_items`/`fninfo`) and
//! collect the crate-wide declaration indexes (`collect_decls`).

use crate::*;

/// Collect every `use` binding that appears ANYWHERE in a function body — the top-level body statements
/// AND any nested block (an inner `{ }`, an `if`/`else`/`match` arm, a loop body). A plain top-of-body
/// loop over `block.stmts` misses the nested ones, so a call through a block-local `use` binding resolved
/// to nothing and read silent-pure (fd's `else { use std::process::{Command, Stdio}; … }`). syn's `Visit`
/// walks the whole tree; every `ItemUse` it reaches (statement-position `use` only — a `use` can appear
/// nowhere else in a body) is expanded into `out`. Over-approximating scope to the whole fn is deliberate
/// and matches the module fallback: it only ever ATTRIBUTES a name to its declared origin (never invents an
/// effect for a name that has none).
struct LocalUseCollector<'a> {
    out: &'a mut HashMap<String, String>,
}

impl<'ast, 'a> Visit<'ast> for LocalUseCollector<'a> {
    fn visit_item_use(&mut self, u: &'ast syn::ItemUse) {
        collect_use(&u.tree, String::new(), self.out);
        // A `use` tree contains no further `use` items — no need to recurse into it.
    }
    // A nested `fn`/`impl`/`mod` item inside a body is a SEPARATE scope: its `use`s belong to it, not the
    // enclosing fn, and it is scanned as its own unit. Don't let the default walk descend into one and
    // leak its imports up here. (`Visit` has no dedicated hook for "item that isn't a use", so stop the
    // three item kinds that carry their own bodies; every other item kind has no fn body to mis-attribute.)
    fn visit_item_fn(&mut self, _: &'ast syn::ItemFn) {}
    fn visit_item_impl(&mut self, _: &'ast syn::ItemImpl) {}
    fn visit_item_mod(&mut self, _: &'ast syn::ItemMod) {}
    // A CLOSURE body is still the same fn's code path (its calls are visited by CallCollector under this
    // fn), so a `use` inside a closure block should stay in scope — the default walk descends into it.
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn scan_items(
    items: &[syn::Item],
    modpath: &str,
    // Pre-resolved `file:line:col` for each emitted fn IN WALK ORDER (see `fn_locs`): line/col can only be
    // resolved on the parse thread, so Pass B threads them in. Consumed positionally via `loc_idx`, which
    // advances exactly once per emitted FnInfo, in lockstep with `fn_locs`.
    locs: &[String],
    loc_idx: &mut usize,
    include_tests: bool,
    fields: &FieldIndex,
    returns: &ReturnIndex,
    traits: TraitIndexes,
    elems: ElemIndexes,
    lazy_statics: &std::collections::HashSet<String>,
    const_strings: &HashMap<String, String>,
    local_macros: &HashMap<String, String>,
    // DROP-GLUE: the local type leaves worth emitting a `<construct>` marker for (see
    // `CallCollector::drop_relevant`). Threaded from scan.rs, where the merged decl index is complete.
    drop_relevant: &std::collections::HashSet<String>,
    uses: &mut HashMap<String, String>,
    out: &mut Vec<FnInfo>,
) {
    for it in items {
        if let syn::Item::Use(u) = it {
            collect_use(&u.tree, String::new(), uses);
        }
    }
    let qual = |name: &str| if modpath.is_empty() { name.to_string() } else { format!("{modpath}::{name}") };
    for it in items {
        match it {
            syn::Item::Fn(f) => {
                // A `#[cfg(test)]` FREE fn (or impl, below) at module scope is test-only — its effects are
                // the tests', not the crate's, same as a `#[cfg(test)] mod`. The guard was on `mod` only,
                // so a bare `#[cfg(test)] fn helper()` leaked into the default report.
                if !include_tests && is_cfg_test(&f.attrs) {
                    continue;
                }
                let n = f.sig.ident.to_string();
                let loc = next_loc(locs, loc_idx);
                out.push(fninfo(&n, &qual(&n), modpath, &loc, &f.sig, &f.block, None, uses, fields, returns, traits, elems, lazy_statics, const_strings, local_macros, drop_relevant));
            }
            syn::Item::Impl(im) => {
                if !include_tests && is_cfg_test(&im.attrs) {
                    continue; // a `#[cfg(test)] impl` block — test-only
                }
                let tyname = impl_type_name(&im.self_ty);
                for ii in &im.items {
                    if let syn::ImplItem::Fn(m) = ii {
                        if !include_tests && is_cfg_test(&m.attrs) {
                            continue; // a `#[cfg(test)]` method within an otherwise-production impl
                        }
                        let n = m.sig.ident.to_string();
                        let q = match &tyname {
                            Some(t) => qual(&format!("{t}::{n}")),
                            None => qual(&n),
                        };
                        let loc = next_loc(locs, loc_idx);
                        out.push(fninfo(&n, &q, modpath, &loc, &m.sig, &m.block, tyname.as_deref(), uses, fields, returns, traits, elems, lazy_statics, const_strings, local_macros, drop_relevant));
                    }
                }
            }
            syn::Item::Mod(m) => {
                if !include_tests && is_cfg_test(&m.attrs) {
                    continue; // a #[cfg(test)] module — its effects are the tests', not the crate's
                }
                if let Some((_, inner)) = &m.content {
                    let sub = qual(&m.ident.to_string());
                    // The file's imports do NOT reach into an inline module's own declarations — see
                    // `submodule_uses`, and the `mod mine { struct Command }` fabrication it closes.
                    let mut subuses = submodule_uses(uses, inner, include_tests);
                    scan_items(inner, &sub, locs, loc_idx, include_tests, fields, returns, traits, elems, lazy_statics, const_strings, local_macros, drop_relevant, &mut subuses, out);
                }
            }
            // A trait's PROVIDED (default) methods have bodies that can perform effects directly
            // (`fn flush(&self) { fs::write(..); }`). Without this arm those bodies were never visited,
            // so the effect — and every caller reaching it — was silently dropped (the "never silently
            // pure" contract broken on a common idiom; adversarial review). A signature-only
            // `fn f(&self);` has no `default` block and stays the impl's job (the Item::Impl arm).
            syn::Item::Trait(tr) => {
                if !include_tests && is_cfg_test(&tr.attrs) {
                    continue;
                }
                let tname = tr.ident.to_string();
                for ti in &tr.items {
                    if let syn::TraitItem::Fn(m) = ti {
                        let Some(block) = &m.default else { continue }; // no body ⇒ abstract, skip
                        if !include_tests && is_cfg_test(&m.attrs) {
                            continue;
                        }
                        let n = m.sig.ident.to_string();
                        let loc = next_loc(locs, loc_idx);
                        // `self` is `Self` (the implementor) — type it as the trait so calls on `self`
                        // resolve through the trait's CHA, exactly like an impl method's `self`.
                        out.push(fninfo(&n, &qual(&format!("{tname}::{n}")), modpath, &loc, &m.sig, block,
                            Some(&tname), uses, fields, returns, traits, elems, lazy_statics, const_strings, local_macros, drop_relevant));
                    }
                }
            }
            _ => {}
        }
        // SYNTHETIC LAZY-INIT UNIT: a `static X: Lazy<_> = Lazy::new(|| ..)` (or LazyLock/LazyCell,
        // `lazy_static!`, `thread_local!`) attaches a deferred init thunk reachable from NO fn — yet it
        // runs on first use and may perform effects (the silent-under-report seam). Emit the thunk body
        // as its own unit (`<lazy>::NAME`) so the classifier/propagation charge it; forcing sites
        // (`visit_expr_path`) edge to it. Always emitted (even for a PURE init) — purity is decided
        // downstream, keeping the keying per-static so a pure lazy floods nothing. Synthetic units are
        // EXCLUDED from `by_leaf` later so they never pollute bare-leaf resolution. Mirrored in
        // `fn_locs` (same walk position + same `#[cfg(test)]` skip via `lazy_unit_emitted`).
        if lazy_unit_emitted(it, include_tests) {
            if let Some((name, body)) = lazy_static_unit(it) {
                let block = syn::Block { brace_token: Default::default(), stmts: body };
                let sig: syn::Signature = syn::parse_quote!(fn __candor_lazy_init());
                let loc = next_loc(locs, loc_idx);
                let q = lazy_qual(modpath, &name);
                out.push(fninfo(&name, &q, modpath, &loc, &sig, &block, None, uses, fields, returns, traits, elems, lazy_statics, const_strings, local_macros, drop_relevant));
            }
        }
    }
}

/// The `use` map an INLINE `mod m { .. }` inherits from the file around it: the enclosing map MINUS every
/// name the module DECLARES for itself.
///
/// Rust gives an inline module its own namespace — a file-level `use` does not reach into it at all, which
/// is why a submodule that wants an import writes its own (and `scan_items`/`collect_decls` pick that up
/// from the inner items, re-binding the name over this map). Inheriting the enclosing map wholesale is a
/// deliberate over-approximation that costs nothing while the names are DISJOINT, and FABRICATES the moment
/// they are not: with `use std::process::Command;` at the top of the file, `mod mine { pub struct Command;
/// pub fn run(c: &Command) { c.spawn(); } }` typed its own receiver as `std::process::Command`, and the std
/// I/O-handle receiver route then charged the crate's own do-nothing `spawn` with `Exec` — MEASURED, a
/// false positive on a provably-pure local path, and SPEC §1 ⟨0.32⟩'s PART 66 over-charge control names
/// exactly it ("a project-local type that merely shares the name gains nothing").
///
/// Removal, not rewriting: the name becomes MODULE-RELATIVE again, which is what it is. Downstream that
/// means `Command::spawn` resolves against `local_types` (the crate's own definition, the right answer) or,
/// if the module declares the name but candor cannot link the call, an honest miss. Removing an inherited
/// binding can never invent an effect — the direction a fabrication would need.
///
/// Namespace-blind on purpose (a `fn read` removes an imported `read` whatever namespace it lives in): in
/// real Rust NO enclosing import reaches an inline module, so removing a shadowed name is always a step
/// toward the truth, never past it. `#[cfg(test)]`-gated declarations are skipped in a production scan,
/// matching the discipline of every other index here.
pub(crate) fn submodule_uses(
    uses: &HashMap<String, String>,
    inner: &[syn::Item],
    include_tests: bool,
) -> HashMap<String, String> {
    let mut subuses = uses.clone();
    for it in inner {
        if let Some(name) = declared_item_name(it, include_tests) {
            subuses.remove(&name);
        }
    }
    subuses
}

/// The NAME an item declares in its enclosing scope — `None` for an item that declares none (an
/// item-position macro INVOCATION, a `use`, an `impl`) and for one a production scan skips
/// (`#[cfg(test)]`).
///
/// ONE ANSWER, TWO CONSUMERS, and the second one is why this was extracted. `submodule_uses` shadows an
/// inherited import with an INLINE MODULE's own declarations; `body_shadowed_uses` does the same for a
/// FUNCTION BODY's. Only the module half existed, which is R106: a body-local `struct Cmd` did not shadow
/// a file-level `Cmd` binding, so `Cmd::new("true").spawn()` inside the body was charged `Exec` with
/// `cmds: ["true"]` for a body that only ever wrote a file. The item-kind list lives here once so the two
/// halves cannot answer the same question differently.
pub(crate) fn declared_item_name(it: &syn::Item, include_tests: bool) -> Option<String> {
    let (attrs, name): (&[syn::Attribute], String) = match it {
        syn::Item::Fn(f) => (&f.attrs, f.sig.ident.to_string()),
        syn::Item::Struct(s) => (&s.attrs, s.ident.to_string()),
        syn::Item::Enum(e) => (&e.attrs, e.ident.to_string()),
        syn::Item::Union(u) => (&u.attrs, u.ident.to_string()),
        syn::Item::Type(t) => (&t.attrs, t.ident.to_string()),
        syn::Item::Trait(t) => (&t.attrs, t.ident.to_string()),
        syn::Item::TraitAlias(t) => (&t.attrs, t.ident.to_string()),
        syn::Item::Mod(m) => (&m.attrs, m.ident.to_string()),
        syn::Item::Const(c) => (&c.attrs, c.ident.to_string()),
        syn::Item::Static(s) => (&s.attrs, s.ident.to_string()),
        syn::Item::ExternCrate(e) => (&e.attrs, e.ident.to_string()),
        // a `macro_rules! NAME` DEFINITION carries an ident; an item-position INVOCATION does not.
        syn::Item::Macro(m) => (&m.attrs, m.ident.as_ref()?.to_string()),
        _ => return None,
    };
    (include_tests || !is_cfg_test(attrs)).then_some(name)
}

/// R106 — the `use` map a FUNCTION BODY sees: the file's map MINUS every name the body itself declares
/// as an item.
///
/// THE DEFECT. A body-local item is a declaration in the body's own scope and it SHADOWS anything the
/// file imported under that name — rustc's rule, and the rule `submodule_uses` has always applied to an
/// inline module. Nothing applied it to a function body, so:
///
///     pub type Cmd = std::process::Command;               // or: use std::process::Command;
///     pub fn f() { struct Cmd; impl Cmd { fn new(_: &str) -> Self { fs::write(..); Cmd } … }
///                  Cmd::new("true").spawn(); }
///
/// resolved the body's OWN `Cmd::new` through the file-level binding: EXECUTED ground truth is one file
/// write and no process, the report said `["Exec","Fs"]` with `cmds: ["true"]`, and `deny Exec` exited 1.
/// A fabricated effect AND a fabricated command surface on a provably-pure-of-Exec path.
///
/// NOT ONLY THE R99 SEED. The `pub type` spelling reaches this through the alias seed b00956b added, but
/// the plain `use std::process::Command;` spelling is the same hole and PREDATES it — measured identically
/// at HEAD. So the fix is at the BODY, where the shadow belongs, not at the seed: fixing the seed alone
/// would have left the commoner spelling silent, which is §9's audit-boundary trap exactly.
///
/// REBOUND TO A SENTINEL, **NOT REMOVED**, AND THAT DISTINCTION IS MEASURED. The first cut of this fix
/// simply removed the name, on `submodule_uses`' own stated argument that "removing an inherited binding
/// can never invent an effect". **For a function body that argument is FALSE**, and the fixture is small:
///
///     pub mod other { pub struct W; impl W { pub fn make() -> Self { fs::write(..); W } } }
///     use std::process::Command as W;
///     pub fn body_pure() { struct W; impl W { fn make() -> Self { W } } let _ = W::make(); }
///
/// `body_pure` calls a body-local, provably pure `W::make`. With the import in scope it fabricated `Exec`
/// (that is R106). With the import merely REMOVED it fabricated `Fs`, because `W::make` fell back to
/// module-relative resolution and the tail2 index linked it to `other::W::make`. One fabrication traded
/// for another. So the name is rebound to `ITEM_SENTINEL + name` — a single segment that can appear in no
/// Rust path, so `expand` yields a tail2 (`<body-item>W::make`) that matches no definition, the classifier
/// returns `None` for its head, and the κ ledger cannot mistake it for a dependency. The BODY's own item
/// is not lost by this: the collector walks a body's nested `fn`/`impl` into the SAME unit, so an
/// effectful body-local `make` still charges its caller.
///
/// What this DOES cost is precision, in the under-report direction: a body that declares `struct write`
/// and elsewhere calls a genuinely-imported `write` loses the import. Bounded deliberately — the walk
/// stops at a nested `fn`/`impl`/`mod`, exactly where `LocalUseCollector`'s does, because those are
/// separate scopes whose declarations do not shadow anything out here.
fn body_declared_items(block: &syn::Block) -> std::collections::HashSet<String> {
    struct C {
        out: std::collections::HashSet<String>,
    }
    impl<'ast> Visit<'ast> for C {
        fn visit_item(&mut self, it: &'ast syn::Item) {
            // `include_tests: false` — a `#[cfg(test)]` item inside a body is not compiled into the
            // build this report describes, so it shadows nothing in it. Under `--include-tests` that
            // makes this UNDER-shadow, i.e. exactly the pre-R106 behaviour for that one case, which is
            // the safe way to be wrong here (stated, not assumed away).
            if let Some(n) = crate::decls::declared_item_name(it, false) {
                self.out.insert(n);
            }
            syn::visit::visit_item(self, it);
        }
        fn visit_item_fn(&mut self, f: &'ast syn::ItemFn) {
            self.out.insert(f.sig.ident.to_string()); // the nested fn's own NAME shadows out here…
        }
        fn visit_item_impl(&mut self, _: &'ast syn::ItemImpl) {}
        fn visit_item_mod(&mut self, m: &'ast syn::ItemMod) {
            self.out.insert(m.ident.to_string());
        }
    }
    let mut c = C { out: std::collections::HashSet::new() };
    c.visit_block(block);
    c.out
}

// ── SUBMODULE-LEVEL RE-EXPORTS ────────────────────────────────────────────────────────────────────
//
// `collect_root_reexports` answers "what does the CRATE ROOT re-export", because a submodule's
// `use crate::net` cannot see the root's `pub use x::net` from its own file. The mirror-image question —
// "what does THIS module re-export, so that a call written ELSEWHERE and qualified by this module names
// it" — had no answer at all, and the intra-crate call graph keys on the last TWO segments, so the two
// spellings of one function never met:
//
//     src/lib.rs            mod imp;  pub fn go() { imp::doit(); }         call tail2:  imp::doit
//     src/imp/mod.rs        mod platform;  pub use self::platform::*;
//     src/imp/platform.rs   pub fn doit() { Command::new("sh").spawn(); }  def  tail2:  platform::doit
//
// `go` reported NOTHING. The named spelling (`pub use self::platform::doit;`) missed identically, so the
// failure is the submodule re-export and not the glob. `collect_reexports` records the edge; scan.rs
// turns the crate's edges into an ALIAS index and consults it only where a qualified tail names no
// definition at all (see `reexport_aliases` / `reexport_target`).

/// The `#[path = "…"]` values on a `mod` declaration, including the ones carried by a
/// `#[cfg_attr(COND, path = "…")]`. SEVERAL are normal and are all kept: the platform-module idiom
/// (`#[cfg_attr(unix, path = "unix.rs")] #[cfg_attr(windows, path = "windows.rs")] mod platform;`, which
/// is tempfile's `src/file/imp/mod.rs` verbatim) names a different file per target, and the scanner
/// analyses EVERY `#[cfg]` branch — so the module has several bodies here, exactly as `cfg_if` arms do.
fn mod_path_attrs(attrs: &[syn::Attribute]) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    fn take(m: &syn::Meta, out: &mut Vec<String>) {
        let syn::Meta::NameValue(nv) = m else { return };
        if !nv.path.is_ident("path") {
            return;
        }
        if let syn::Expr::Lit(syn::ExprLit { lit: syn::Lit::Str(s), .. }) = &nv.value {
            let v = s.value();
            if !v.trim().is_empty() && !out.contains(&v) {
                out.push(v);
            }
        }
    }
    for a in attrs {
        if a.path().is_ident("path") {
            take(&a.meta, &mut out);
        } else if a.path().is_ident("cfg_attr") {
            // `#[cfg_attr(COND, attr, …)]` — the first element is the condition, the rest are the
            // attributes it would apply. Only the `path = "…"` ones matter here; the CONDITION is
            // deliberately discarded, matching how every other `#[cfg]` branch is treated.
            if let Ok(items) = a.parse_args_with(
                syn::punctuated::Punctuated::<syn::Meta, syn::Token![,]>::parse_terminated,
            ) {
                for m in items.iter().skip(1) {
                    take(m, &mut out);
                }
            }
        }
    }
    out
}

/// Lexically normalise a source-relative path (`src/imp/../shared/x.rs` -> `src/shared/x.rs`) so
/// `module_path` sees the same spelling the file walk produced. Purely textual — no filesystem access,
/// so it cannot depend on what happens to exist.
fn normalize_rel(p: &Path) -> std::path::PathBuf {
    let mut out = std::path::PathBuf::new();
    for c in p.components() {
        match c {
            std::path::Component::CurDir => {}
            std::path::Component::ParentDir => {
                out.pop();
            }
            other => out.push(other.as_os_str()),
        }
    }
    out
}

/// `mod NAME;` / `mod NAME { … }` declarations at ONE module level -> the module path(s) their body is
/// analysed under. Without a `#[path]` redirect that is simply `<modpath>::<NAME>`, which is what
/// `module_path` yields for both `NAME.rs` and `NAME/mod.rs`. With one, the target is resolved as a FILE
/// path relative to `dir` and then run through `module_path` — the scanner names a file by where it SITS,
/// so resolving to the file and asking `module_path` is exactly right even where Rust's own module path
/// would differ (a `#[path]` on a `mod` in a non-`mod.rs` file).
fn mod_targets(
    items: &[syn::Item],
    modpath: &str,
    dir: &Path,
    include_tests: bool,
) -> HashMap<String, Vec<String>> {
    let mut m = HashMap::new();
    for it in items {
        let syn::Item::Mod(md) = it else { continue };
        if !include_tests && is_cfg_test(&md.attrs) {
            continue;
        }
        let name = md.ident.to_string();
        let mut targets: Vec<String> = Vec::new();
        for p in mod_path_attrs(&md.attrs) {
            let mp = module_path(&normalize_rel(&dir.join(p)));
            if !targets.contains(&mp) {
                targets.push(mp);
            }
        }
        if targets.is_empty() {
            targets.push(if modpath.is_empty() { name.clone() } else { format!("{modpath}::{name}") });
        }
        m.insert(name, targets);
    }
    m
}

/// Flatten a `use` tree into `(source module segments, source name, visible-as name)` triples. A glob
/// yields `("*", "*")` with the whole prefix as the module.
///
/// `use a::b::{self, …}` (re-exporting the MODULE `b` under its own name) is DELIBERATELY skipped: this
/// index maps NAMES to function definitions, and a module alias would need the whole path rewritten
/// rather than one name aliased. Skipping it is the under-report direction.
fn use_leaves(tree: &syn::UseTree, prefix: Vec<String>, out: &mut Vec<(Vec<String>, String, String)>) {
    match tree {
        syn::UseTree::Path(p) => {
            let mut pf = prefix;
            pf.push(p.ident.to_string());
            use_leaves(&p.tree, pf, out);
        }
        syn::UseTree::Name(n) => {
            let id = n.ident.to_string();
            if id != "self" {
                out.push((prefix, id.clone(), id));
            }
        }
        syn::UseTree::Rename(r) => {
            let id = r.ident.to_string();
            if id != "self" {
                out.push((prefix, id, r.rename.to_string()));
            }
        }
        syn::UseTree::Group(g) => {
            for t in &g.items {
                use_leaves(t, prefix.clone(), out);
            }
        }
        syn::UseTree::Glob(_) => out.push((prefix, "*".to_string(), "*".to_string())),
    }
}

/// The module path(s) a `use` prefix names, or EMPTY when it names nothing local (an external crate).
///
/// Only `self::` / `super::` / `crate::` and the Rust-2018 UNIFORM-PATH form (a bare head segment that
/// this very module DECLARES as a `mod` — tempfile's `pub use unix::*;`) resolve. A bare head segment
/// that names no local `mod` is an EXTERNAL crate and yields nothing: assuming it were local is how an
/// alias index would start answering for paths that name no item in this crate.
fn resolve_use_base(segs: &[String], modpath: &str, mods: &HashMap<String, Vec<String>>) -> Vec<String> {
    let mut i = 0usize;
    let mut bases: Vec<String> = match segs.first().map(String::as_str) {
        Some("self") => {
            i = 1;
            vec![modpath.to_string()]
        }
        Some("crate") => {
            i = 1;
            vec![String::new()]
        }
        Some("super") => {
            let mut m = modpath.to_string();
            while segs.get(i).map(String::as_str) == Some("super") {
                match m.rsplit_once("::") {
                    Some((p, _)) => m = p.to_string(),
                    None if !m.is_empty() => m = String::new(),
                    // more `super`s than the module has ancestors — impossible in code that compiles,
                    // and answering anyway would name the wrong module.
                    None => return Vec::new(),
                }
                i += 1;
            }
            vec![m]
        }
        Some(head) if mods.contains_key(head) => {
            i = 1;
            mods[head].clone()
        }
        _ => return Vec::new(),
    };
    // Having landed on THIS module, the next segment may be one of its own `mod` declarations — and that
    // is where a `#[path]` redirect has to be honoured (`self::platform` -> `imp::unix`/`imp::windows`).
    // Only at this module's level: no other module's `mod` declarations are in `mods`.
    if bases.len() == 1 && bases[0] == modpath {
        if let Some(t) = segs.get(i).and_then(|n| mods.get(n)) {
            bases = t.clone();
            i += 1;
        }
    }
    let rest = segs[i..].join("::");
    bases
        .into_iter()
        .map(|b| match (b.is_empty(), rest.is_empty()) {
            (_, true) => b,
            (true, false) => rest.clone(),
            (false, false) => format!("{b}::{rest}"),
        })
        .collect()
}

/// A name declared at `modpath`, spelled as the path a caller ELSEWHERE writes for it (`facade::Command`;
/// just `Command` at the crate root). The key shape `seed_mod_aliases` and `expand`'s multi-segment lookup
/// both agree on.
pub(crate) fn qualify(modpath: &str, name: &str) -> String {
    if modpath.is_empty() { name.to_string() } else { format!("{modpath}::{name}") }
}

/// R105 — the separator joining the SEVERAL targets one alias name can have when its declaration is
/// duplicated across `#[cfg]` arms. `\u{1}` cannot appear in a Rust path, and it is the SAME convention
/// `collect_root_reexports` already uses for the multi-glob list under `GLOB_KEY` — one spelling for
/// "this key has more than one answer", not a second one.
pub(crate) const ALIAS_ALT_SEP: char = '\u{1}';

/// R99 (SHAPE 1) — the alias key under which a SUBMODULE's single external GLOB re-export is recorded
/// (`mod glb { pub use std::fs::*; }` -> key `glb::*glob`). Deliberately NOT `lang::GLOB_KEY`: that key
/// means "a glob in THIS file's own `use` map" and is read by `unique_glob`/`root_glob`, and
/// `seed_mod_aliases` would otherwise drop a submodule's glob into a file's map under the very name those
/// two read. A leading `*` means no Rust path can spell it, so no call site can collide with it either.
pub(crate) const MOD_GLOB_KEY: &str = "*glob";

/// R99 (SHAPE 1) — separates a module glob's TARGET from the sorted list of names the module DECLARES
/// ITSELF. A glob import is shadowed by an explicit item of the same name (rustc's rule), so
/// `mod glb { pub use std::fs::*; pub fn write(..) {..} }` means `glb::write` is the LOCAL fn — rewriting
/// it to `std::fs::write` would FABRICATE an `Fs` on a path that performs none. The shadow list rides in
/// the same entry rather than in a second index, so there is one thing to seed, one thing to hash, and no
/// way for the two halves to drift apart.
pub(crate) const GLOB_SHADOW_SEP: char = '\u{2}';

/// R106 — the prefix a FUNCTION-BODY-LOCAL item's name is rebound to in that body's `use` map. A single
/// segment containing characters no Rust path can carry, so the resulting `tail2` matches no definition
/// in the crate, the classifier's head lookup finds nothing, and the κ ledger cannot read it as a
/// dependency. See `body_shadowed_uses` for why REBINDING and not removing.
pub(crate) const ITEM_SENTINEL: &str = "<body-item>";

/// R105 — record ONE crate-local alias for an external item, resolving a DUPLICATE declaration instead
/// of letting source order decide it.
///
/// THE DEFECT THIS CLOSES. All three alias-collection branches below used to end in a bare
/// `aliases.insert(..)`, so two `#[cfg]` arms declaring the same qualified name — the ordinary
/// platform/feature shim, `#[cfg(unix)] pub use std::fs::write as put;` beside `#[cfg(not(unix))] pub use
/// std::env::set_var as put;` — resolved to WHICHEVER ARM WAS WRITTEN LAST. Measured on two crates
/// identical but for arm order: one reported `go: ["Fs"]` and failed `deny Fs` (exit 1), the other
/// reported `go: ["Env"]` and PASSED it (exit 0) — a fabricated effect class and, behind it, the real
/// `Fs` unreported anywhere in the document. candor analyses every `cfg` branch everywhere else (see
/// `Reexport::from`, which is a `Vec` for exactly this reason); this map was the one place that collapsed
/// them, and it collapsed them by position.
///
/// WHY THE ARMS ARE KEPT RATHER THAN ADJUDICATED HERE. The alias target is a PREFIX — a caller appends
/// its own leaf (`Cmd::new`, `put(..)`) — so whether two arms answer the same question is a property of
/// the CALL, not of the target alone, and it is `candor_classify` that decides it. Deciding it here would
/// mean re-implementing the classifier against a leaf nobody has yet written (§G). So both arms are kept,
/// SORTED (the result no longer depends on walk order or `HashMap` iteration order, which is what the
/// decl-index digest requires), and the choice is made in scan.rs's call loop where the leaf is in hand:
/// identical classifications → charge that one effect; different → `Unknown`, the same honest signal
/// `drop_cross_ambiguous_enum_leaves`'s R90 collision already earns. Never a pick by position.
pub(crate) fn record_alias(aliases: &mut HashMap<String, String>, key: String, target: String) {
    match aliases.get_mut(&key) {
        None => {
            aliases.insert(key, target);
        }
        Some(prev) => {
            // The SAME target declared twice (the common `cfg_if` shape where both arms re-export the
            // identical item) is not an ambiguity at all — it is one answer, recorded twice.
            if prev.split(ALIAS_ALT_SEP).any(|t| t == target) {
                return;
            }
            let mut alts: Vec<&str> =
                prev.split(ALIAS_ALT_SEP).chain(std::iter::once(target.as_str())).collect();
            alts.sort_unstable();
            alts.dedup();
            *prev = alts.join(&ALIAS_ALT_SEP.to_string());
        }
    }
}

/// Collect this file's `pub use` RE-EXPORT edges, recursing into inline `mod` blocks. `modpath` is the
/// module path the items sit at and `dir` is the DIRECTORY of the source file, relative to the scan root
/// — `#[path]` targets resolve against it (Rust resolves a `#[path]` on an out-of-line `mod` relative to
/// the containing file's directory, and inside an inline `mod` block with the inline names as
/// directories, which is what the recursion below does).
///
/// R99 — the SAME walk also collects `aliases`, the MODULE-QUALIFIED name → EXTERNAL path map, from the
/// three item shapes that give a std/dependency item a second, crate-local spelling:
///
///   1. the `from.is_empty()` branch below — a `pub use` whose head names no local `mod`, i.e. a
///      re-export of a std/external item (`mod facade { pub use std::process::Command; }`). `Reexport`
///      cannot carry it: its `from` is an INTRA-crate module path feeding the tail2 call-graph index,
///      and this names nothing in the crate. It was DROPPED, and `facade::Command::new("x").status()`
///      was absent from `functions[]` under every policy form including a blanket `deny`.
///   2. a NOMINAL type alias (`pub type Cmd = std::process::Command;`). `Item::Type` was recorded only
///      when the target is NON-nominal (`prim_aliases`, a resolution SKIP); the nominal case — which is
///      what `pub type Client = reqwest::Client;` is — recorded nothing at all.
///   3. a `const`/`static` of CALLABLE type bound to a bare path (`const W: fn(&str) = writer;`).
///
/// All three are recorded exactly as the equivalent `use … as NAME` would have been, and consumed through
/// the SAME `expand`/`uses` authority — no second resolution path (§G). The direction is ADD-only: a name
/// that resolved to nothing now resolves to its declared origin.
/// R99 (SHAPE 1) — record ONE module's external GLOB re-export, with the names that shadow it.
///
/// THE DEFECT THIS CLOSES. `collect_reexports`'s external branch skipped `name == "*"`, stating the skip
/// as a residual: enumerating an external module's exports needs its source. But the glob does not have to
/// be enumerated to be USED — a caller writes the leaf, so the leaf is in hand at resolution time and only
/// the PREFIX is missing. Measured, EXECUTED, by the syscall oracle driver `pf_alias_glob`:
///
///     src/glb.rs    pub use std::fs::*;
///     src/main.rs   mod glb;  fn put() { let _ = glb::write("/tmp/…", b"x"); }
///
/// `functions: []` — the whole report EMPTY, `excluded: []`, no `Unknown`, no `unresolved`, nothing —
/// while strace watched the write and three functions sat on the stack. The paired control (the same
/// module re-exporting `write` BY NAME) reported `Fs` on all three.
///
/// THE SHADOW LIST IS THE WHOLE DIFFICULTY, and it is why this is not a one-line `name != "*"` deletion.
/// A glob import is shadowed by an explicit item of the same name, so a module that globs `std::fs` AND
/// declares its own `write` means the LOCAL one — and unlike every other alias route, rewriting is
/// FABRICATION here rather than a missed resolution, because the rewritten path leaves the crate
/// (`std::fs::write`) and `scan.rs` never tries a local link for a std-rooted path. So the module's own
/// declared names ride in the entry and `lang::qualified_alias` refuses any leaf among them.
///
/// TWO MORE REFUSALS, both in the under-report direction and both pinned by DELETING them (§C):
///  * a PRIVATE glob exports nothing, so it cannot answer a qualified `glb::write` from outside.
///  * TWO OR MORE exported globs in one module — external or crate-local — is ambiguous, and the local
///    kind is already answered by `Reexport`/`reexport_target` through the tail2 index. Never guess which
///    glob a name arrived through; that is `unique_glob`'s own rule, one level out.
///
/// AND A THIRD THAT IS NOT BEHAVIOUR-BEARING, SAID AS SUCH RATHER THAN LISTED AS A GUARD. The
/// `modpath.is_empty()` return skips the crate ROOT, and deleting it turns no test red and moves no
/// corpus row: a root entry's key is a bare `*glob`, and `module_glob_alias` only ever looks up
/// `<module-prefix>::*glob`, so the entry would be unreachable dead weight in the merged index and in its
/// digest. It stays because `collect_root_reexports` already answers the root question under `GLOB_KEY`,
/// and two indexes answering one question is how this family produced its worst defects (§G).
fn collect_module_glob(
    items: &[syn::Item],
    modpath: &str,
    mods: &HashMap<String, Vec<String>>,
    include_tests: bool,
    aliases: &mut HashMap<String, String>,
) {
    if modpath.is_empty() {
        return;
    }
    let mut exported_globs = 0usize;
    let mut external: Option<String> = None;
    let mut shadow: Vec<String> = Vec::new();
    for it in items {
        let syn::Item::Use(u) = it else {
            if let Some(n) = declared_item_name(it, include_tests) {
                shadow.push(n);
            }
            continue;
        };
        if !include_tests && is_cfg_test(&u.attrs) {
            continue;
        }
        let exported = !matches!(&u.vis, syn::Visibility::Inherited)
            && !matches!(&u.vis, syn::Visibility::Restricted(r) if r.path.is_ident("self"));
        let mut leaves = Vec::new();
        use_leaves(&u.tree, Vec::new(), &mut leaves);
        for (segs, name, alias) in leaves {
            if name != "*" {
                // A NAMED import binds the name in this module whether or not it is re-exported, and a
                // binding shadows the glob either way.
                shadow.push(alias);
                continue;
            }
            if !exported {
                continue;
            }
            exported_globs += 1;
            let head = segs.first().map(String::as_str).unwrap_or("");
            if !segs.is_empty()
                && !matches!(head, "crate" | "self" | "super")
                && resolve_use_base(&segs, modpath, mods).is_empty()
            {
                external = Some(segs.join("::"));
            }
        }
    }
    if exported_globs != 1 {
        return;
    }
    let Some(mut target) = external else { return };
    shadow.sort();
    shadow.dedup();
    for s in &shadow {
        target.push(GLOB_SHADOW_SEP);
        target.push_str(s);
    }
    record_alias(aliases, qualify(modpath, MOD_GLOB_KEY), target);
}

pub(crate) fn collect_reexports(
    items: &[syn::Item],
    modpath: &str,
    dir: &Path,
    include_tests: bool,
    uses: &HashMap<String, String>,
    out: &mut Vec<Reexport>,
    aliases: &mut HashMap<String, String>,
) {
    let mods = mod_targets(items, modpath, dir, include_tests);
    let no_bounds: HashMap<String, Vec<String>> = HashMap::new();
    collect_module_glob(items, modpath, &mods, include_tests, aliases);
    for it in items {
        match it {
            syn::Item::Use(u) => {
                // A PRIVATE `use` exports nothing — it binds a name for this module's own body. `pub(self)`
                // is spelled differently and means private too.
                match &u.vis {
                    syn::Visibility::Inherited => continue,
                    syn::Visibility::Restricted(r) if r.path.is_ident("self") => continue,
                    _ => {}
                }
                if !include_tests && is_cfg_test(&u.attrs) {
                    continue;
                }
                let mut leaves = Vec::new();
                use_leaves(&u.tree, Vec::new(), &mut leaves);
                for (segs, name, alias) in leaves {
                    let from = resolve_use_base(&segs, modpath, &mods);
                    if from.is_empty() {
                        // R99 (1): the head names no local `mod` and is not `crate`/`self`/`super`, so this
                        // re-exports an EXTERNAL/std item. A GLOB (`pub use std::process::*`) names no
                        // single item and is skipped — enumerating an external module's exports needs its
                        // source, which a syntactic scan does not have (stated residual, not a guess).
                        if name != "*" && !segs.is_empty() {
                            let head = segs[0].as_str();
                            if !matches!(head, "crate" | "self" | "super") {
                                let target = format!("{}::{name}", segs.join("::"));
                                record_alias(aliases, qualify(modpath, &alias), target);
                            }
                        }
                        continue;
                    }
                    out.push(Reexport { module: modpath.to_string(), from, name, alias });
                }
            }
            // R99 (2): a NOMINAL type alias is exactly a `use <target> as <ident>` for path resolution,
            // and is recorded as one. THE EXPOSURE IS THEREFORE THE SAME AS A `use`'S, not smaller —
            // stated as the assumption it is rather than as a guarantee. A same-module `use Bar;` and
            // `type Bar = …` cannot coexist (one type namespace, E0255), so no COMPILING input reaches
            // that collision and no fixture could witness it; but a VALUE-namespace name may share the
            // spelling (`type Foo = Bar;` beside `fn Foo()`), and there this entry answers for `Foo()`
            // exactly as `use x::Foo;` already does today. Converging on the `use` route means inheriting
            // its residual, deliberately, rather than opening a second one.
            //
            // Generic aliases (`type R<T> = Result<T, E>`) are skipped: their target carries parameters
            // this map has no way to substitute.
            syn::Item::Type(t) if t.generics.params.is_empty() && !is_non_nominal_type(&t.ty) => {
                if !include_tests && is_cfg_test(&t.attrs) {
                    continue;
                }
                if let syn::Type::Path(p) = &*t.ty {
                    if p.qself.is_none() {
                        let written = path_to_string(&p.path);
                        if written != "Self" {
                            record_alias(aliases, qualify(modpath, &t.ident.to_string()), expand(&written, uses));
                        }
                    }
                }
            }
            // R99 (3): `const W: fn(&str) -> R = writer;` / the `static` twin. Gated on BOTH a callable
            // declared type and a bare-path initializer, which is what keeps it narrow: the initializer of
            // a callable-typed const can only name a function item. Same residual as (2) — a TYPE of the
            // same spelling could coexist and would now expand through this entry — and the same reason
            // for accepting it: a `use std::fs::write as W;` is already recorded identically.
            syn::Item::Const(_) | syn::Item::Static(_) => {
                let (attrs, ident, ty, expr) = match it {
                    syn::Item::Const(c) => (&c.attrs, &c.ident, &c.ty, &c.expr),
                    syn::Item::Static(s) => (&s.attrs, &s.ident, &s.ty, &s.expr),
                    _ => unreachable!(),
                };
                if !include_tests && is_cfg_test(attrs) {
                    continue;
                }
                if is_callable_type(ty, &no_bounds) {
                    if let syn::Expr::Path(p) = &**expr {
                        if p.qself.is_none() {
                            let written = path_to_string(&p.path);
                            record_alias(aliases, qualify(modpath, &ident.to_string()), expand(&written, uses));
                        }
                    }
                }
            }
            syn::Item::Mod(m) => {
                if !include_tests && is_cfg_test(&m.attrs) {
                    continue;
                }
                if let Some((_, inner)) = &m.content {
                    let name = m.ident.to_string();
                    let sub =
                        if modpath.is_empty() { name.clone() } else { format!("{modpath}::{name}") };
                    // The inline module's OWN `use` map — the same shadowing rule `collect_decls` and
                    // `scan_items` apply, so a submodule that declares its own `Command` is not typed
                    // through the enclosing file's import.
                    let subuses = submodule_uses(uses, inner, include_tests);
                    collect_reexports(inner, &sub, &dir.join(&name), include_tests, &subuses, out, aliases);
                }
            }
            _ => {}
        }
    }
}

/// Whether a lazy-static synthetic unit will be EMITTED for `it` — a `static`/`const`/macro lazy with a
/// walkable thunk that is NOT `#[cfg(test)]`-gated (unless tests are included). The single source of
/// truth shared by `scan_items` (emits the unit) and `fn_locs` (emits its loc), so the two walks stay in
/// LOCKSTEP (the `debug_assert` count guard). Returns false for any non-lazy item.
pub(crate) fn lazy_unit_emitted(it: &syn::Item, include_tests: bool) -> bool {
    let attrs: &[syn::Attribute] = match it {
        syn::Item::Static(s) => &s.attrs,
        syn::Item::Const(c) => &c.attrs,
        syn::Item::Macro(m) => &m.attrs,
        _ => return false,
    };
    if !include_tests && is_cfg_test(attrs) {
        return false;
    }
    lazy_static_unit(it).is_some()
}

/// Pop the next pre-resolved loc for an emitted fn, advancing the cursor. `locs` is produced by `fn_locs`
/// in the SAME walk order, so the indices line up exactly. A `debug_assert!` trips if they ever drift
/// (more fns emitted than locs precomputed); in release it falls back to the bare file path with the line
/// stripped — wrong-but-not-crashing — so the scan still produces a report rather than panicking.
pub(crate) fn next_loc(locs: &[String], loc_idx: &mut usize) -> String {
    debug_assert!(*loc_idx < locs.len(), "fn_locs/scan_items walk-order drift: more fns than locs");
    let l = locs.get(*loc_idx).cloned().unwrap_or_default();
    *loc_idx += 1;
    l
}

/// Resolve `file:line:col` for every fn `scan_items` will emit, in IDENTICAL walk order — the loc
/// counterpart to `scan_items`. It MUST be called on the thread that PARSED `items`: proc-macro2's
/// `span-locations` resolves a span's line/col against a THREAD-LOCAL source map populated at parse time,
/// so a span moved to another thread (our single-threaded Pass B) resolves to nothing. Pass B can't derive
/// loc; the parse closures call this and Pass B zips the result onto each FnInfo by position.
///
/// The fn-emitting structure here mirrors `scan_items` arm-for-arm (same `#[cfg(test)]` skips, same nested
/// `mod` recursion, same impl/trait-default coverage), so the i-th loc lines up with the i-th FnInfo. A
/// `debug_assert_eq!` in Pass B guards that the two counts agree (any future drift trips the 42 tests).
/// Line is proc-macro2's 1-based line; column is its 0-based column + 1 (1-based, matching the deep
/// engine's `build.rs:10:1` baselines). The span used is the whole item/method (its first token), not just
/// the ident, so a `pub fn foo` at column 0 reports col 1, not the column of `foo`.
pub(crate) fn fn_locs(items: &[syn::Item], file: &str, include_tests: bool, out: &mut Vec<String>) {
    use syn::spanned::Spanned;
    let loc = |sp: proc_macro2::Span| {
        let s = sp.start();
        format!("{file}:{}:{}", s.line, s.column + 1)
    };
    for it in items {
        match it {
            syn::Item::Fn(f) => {
                if !include_tests && is_cfg_test(&f.attrs) {
                    continue;
                }
                out.push(loc(f.span()));
            }
            syn::Item::Impl(im) => {
                if !include_tests && is_cfg_test(&im.attrs) {
                    continue;
                }
                for ii in &im.items {
                    if let syn::ImplItem::Fn(m) = ii {
                        if !include_tests && is_cfg_test(&m.attrs) {
                            continue;
                        }
                        out.push(loc(m.span()));
                    }
                }
            }
            syn::Item::Mod(m) => {
                if !include_tests && is_cfg_test(&m.attrs) {
                    continue;
                }
                if let Some((_, inner)) = &m.content {
                    fn_locs(inner, file, include_tests, out);
                }
            }
            syn::Item::Trait(tr) => {
                if !include_tests && is_cfg_test(&tr.attrs) {
                    continue;
                }
                for ti in &tr.items {
                    if let syn::TraitItem::Fn(m) = ti {
                        if m.default.is_none() {
                            continue;
                        }
                        if !include_tests && is_cfg_test(&m.attrs) {
                            continue;
                        }
                        out.push(loc(m.span()));
                    }
                }
            }
            _ => {}
        }
        // Mirror the synthetic LAZY-INIT UNIT loc in lockstep with `scan_items` (same `lazy_unit_emitted`
        // predicate, same walk position). The unit's loc is the static item's own span.
        if lazy_unit_emitted(it, include_tests) {
            out.push(loc(it.span()));
        }
    }
}

/// EVERY ident this signature and body BIND, whether or not their type could be recovered — parameters,
/// `let`s, closure params, `for` patterns, match-arm bindings. `syn::Pat::Ident` is the one node all of
/// those funnel through, so a single walk over the signature's patterns plus the body collects them all.
///
/// Its ONLY consumer is the shadow test on the lazy-static / dep-provenance forcing sites, which must
/// answer "does this name refer to a LOCAL binding rather than the static?". The typed side-tables
/// answer that only for bindings whose type resolved — `let C = "aa";` is invisible to all of them —
/// so a `use`d static shadowed by an untypable `let` was charged to the local. Over-collecting here
/// (a match arm's `Pat::Ident` that is really a unit-struct pattern, a nested item's params) costs a
/// forcing edge; UNDER-collecting fabricates an effect on a provably-unrelated local, so the walk is
/// deliberately liberal.
pub(crate) fn bound_idents(sig: &syn::Signature, block: &syn::Block) -> std::collections::HashSet<String> {
    struct V(std::collections::HashSet<String>);
    impl<'ast> syn::visit::Visit<'ast> for V {
        fn visit_pat_ident(&mut self, node: &'ast syn::PatIdent) {
            self.0.insert(node.ident.to_string());
            syn::visit::visit_pat_ident(self, node);
        }
    }
    use syn::visit::Visit;
    let mut v = V(std::collections::HashSet::new());
    for arg in &sig.inputs {
        if let syn::FnArg::Typed(pt) = arg {
            v.visit_pat(&pt.pat);
        }
    }
    v.visit_block(block);
    v.0
}

/// R100 — the idents ONE pattern binds. Same `Pat::Ident` funnel as `bound_idents` above, narrowed to a
/// single pattern: `visit_local` needs the names THIS statement binds so it can walk the statement's own
/// RHS under the state that was live before the binding took effect.
pub(crate) fn pat_bound_idents(pat: &syn::Pat) -> Vec<String> {
    struct V(Vec<String>);
    impl<'ast> syn::visit::Visit<'ast> for V {
        fn visit_pat_ident(&mut self, node: &'ast syn::PatIdent) {
            let n = node.ident.to_string();
            if !self.0.contains(&n) {
                self.0.push(n);
            }
            syn::visit::visit_pat_ident(self, node);
        }
    }
    use syn::visit::Visit;
    let mut v = V(Vec::new());
    v.visit_pat(pat);
    v.0
}

/// Seed a function's variable→type map from its parameters (`fn h(c: &reqwest::Client)`) and, for an
/// impl method, `self` → the impl type. These are the most reliable type facts available syntactically.
pub(crate) fn seed_vars(sig: &syn::Signature, self_ty: Option<&str>, uses: &HashMap<String, String>) -> HashMap<String, String> {
    let mut vars = HashMap::new();
    if let Some(t) = self_ty {
        vars.insert("self".to_string(), t.to_string());
    }
    for arg in &sig.inputs {
        if let syn::FnArg::Typed(pt) = arg {
            if let syn::Pat::Ident(id) = &*pt.pat {
                if let Some(ty) = type_path(&pt.ty, uses) {
                    vars.insert(id.ident.to_string(), ty);
                }
            }
        }
    }
    vars
}

/// The collection counterpart of `seed_vars`: a param whose type is a COLLECTION (`xs: &[Sender]`,
/// `xs: Vec<Sender>`) seeds `name -> element type` so a `for c in xs` / `xs[0]` / `xs.iter().for_each`
/// inside the body resolves the element. ALSO binds the single-ident elements of a TUPLE param
/// (`fn f((s, _): (Sender, usize))` → `s: Sender` into `vars`) — a destructuring param pattern that
/// `seed_vars` (Ident-only) misses. Also records the per-position types of a TUPLE-typed param into
/// `tuple_of` (`fn f(pair: (Sender, usize))` → a later `let (s, _) = pair`). Returns
/// `(elem_of, tuple_of)`; tuple-DESTRUCTURED param bindings are merged into `vars`.
#[allow(clippy::type_complexity)] // an internal multi-index return — the four maps are each meaningful and named in the doc above; a tuple-of-aliases would obscure more than it clarifies
pub(crate) fn seed_elem_of(
    sig: &syn::Signature,
    vars: &mut HashMap<String, String>,
    uses: &HashMap<String, String>,
) -> (HashMap<String, String>, TupleElemIndex, HashMap<String, Vec<String>>, HashMap<String, Vec<Vec<String>>>) {
    let mut elem_of = HashMap::new();
    let mut tuple_of: TupleElemIndex = HashMap::new();
    let mut tuple_trait_of: HashMap<String, Vec<Vec<String>>> = HashMap::new();
    // Element DISPATCH leaves for a param that is a COLLECTION of trait objects — `for it in items {
    // it.go() }` dispatches via bounded CHA. Covers a CONCRETE-dyn element (`items: Vec<Box<dyn Doer>>` →
    // ["Doer"]) AND a GENERIC element bound by a trait (`fn f<T: Doer>(items: Vec<T>)` → ["Doer"], via the
    // fn's generic bounds — `trait_leaves` resolves the bare `T` through them).
    let mut elem_trait_of: HashMap<String, Vec<String>> = HashMap::new();
    let gbounds = generic_bounds_of(sig);
    for arg in &sig.inputs {
        let syn::FnArg::Typed(pt) = arg else { continue };
        match &*pt.pat {
            syn::Pat::Ident(id) => {
                if let Some(e) = elem_type(&pt.ty, uses) {
                    elem_of.insert(id.ident.to_string(), e);
                }
                let leaves = elem_trait_leaves(&pt.ty, &gbounds);
                if !leaves.is_empty() {
                    elem_trait_of.insert(id.ident.to_string(), leaves);
                }
                // `fn f(pair: (Sender, usize))` — record positions for a later `let (s, _) = pair`.
                if let Some(t) = tuple_types(&pt.ty, uses) {
                    tuple_of.insert(id.ident.to_string(), t);
                }
                // `fn f(pair: (Box<dyn Doer>, u32))` — a TRAIT-OBJECT tuple position, so a later
                // `let (d, _) = pair; d.go()` dispatches (R46 tuple; `tuple_types`/`type_path` can't hold a
                // `dyn` element).
                if let Some(t) = tuple_trait_leaves(&pt.ty, &gbounds) {
                    tuple_trait_of.insert(id.ident.to_string(), t);
                }
            }
            // `fn f((s, n): (Sender, usize))` — a tuple-destructured param.
            syn::Pat::Tuple(tup) => {
                if let syn::Type::Tuple(tty) = &*pt.ty {
                    for (pat_el, ty_el) in tup.elems.iter().zip(tty.elems.iter()) {
                        if let Some(name) = single_pat_ident(pat_el) {
                            if let Some(ty) = type_path(ty_el, uses) {
                                vars.insert(name.clone(), ty);
                            }
                            if let Some(e) = elem_type(ty_el, uses) {
                                elem_of.insert(name, e);
                            }
                        }
                    }
                }
            }
            _ => {}
        }
    }
    (elem_of, tuple_of, elem_trait_of, tuple_trait_of)
}

/// The dispatch-typed counterpart of `seed_vars`: params whose type is a trait bound rather than a
/// concrete path (`t: &dyn Store`, `s: impl Store`, `x: X` under `X: Store`) -> their bound leaves.
pub(crate) fn seed_trait_vars(sig: &syn::Signature) -> HashMap<String, Vec<String>> {
    let gb = generic_bounds_of(sig);
    let mut m = HashMap::new();
    for arg in &sig.inputs {
        if let syn::FnArg::Typed(pt) = arg {
            if let syn::Pat::Ident(id) = &*pt.pat {
                let leaves = trait_leaves(&pt.ty, &gb);
                if !leaves.is_empty() {
                    m.insert(id.ident.to_string(), leaves);
                }
            }
        }
    }
    m
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn fninfo(
    leaf: &str,
    qual: &str,
    // The enclosing MODULE path (not the type path): a bare static reference in this body names this
    // module's static, so the forcing edge is built from it.
    modpath: &str,
    loc: &str,
    sig: &syn::Signature,
    block: &syn::Block,
    self_ty: Option<&str>,
    uses: &HashMap<String, String>,
    fields: &FieldIndex,
    returns: &ReturnIndex,
    traits: TraitIndexes,
    elems: ElemIndexes,
    lazy_statics: &std::collections::HashSet<String>,
    const_strings: &HashMap<String, String>,
    local_macros: &HashMap<String, String>,
    drop_relevant: &std::collections::HashSet<String>,
) -> FnInfo {
    // Function-LOCAL `use` statements (`fn f() { use rustix::time::clock_settime; … }`) are body
    // STATEMENTS, not module items, so the module-level use map misses them — every call they import then
    // fails to resolve to its crate and is under-reported (found on coreutils `date`: its rustix clock
    // read is imported by a fn-local `use`). Merge them in. This walks the WHOLE body tree, so a `use`
    // buried in a NESTED block — an inner `{ }`, an `if`/`else`/`match` arm, a loop body — is captured too
    // (found on fd `src/main.rs`: `else { use std::process::{Command, Stdio}; Command::new(..).status() }`
    // read silent-pure because the nested `use` was never collected). Scope is treated conservatively — a
    // `use` anywhere in the fn makes the binding available for the whole fn body (over-approximation, the
    // same posture the syntactic backend takes for the module fallback); the point is the ORIGIN is no
    // longer LOST, so a std/covered call classifies as it would at module level and an external crate is
    // disclosed in the ledger exactly as a module-level `use` would disclose it. Never fabrication: it only
    // resolves a name to its already-declared origin — a genuinely-local pure call stays pure.
    let mut local_uses = HashMap::new();
    {
        let mut c = LocalUseCollector { out: &mut local_uses };
        c.visit_block(block);
    }
    // R106 — …and the mirror-image adjustment, which was missing entirely: a name the body DECLARES as
    // an item shadows whatever the file imported under it (`body_shadowed_uses`' doc has the measured
    // fabrication). Applied in the same place and the same way as the additive half, so a body's `use`
    // and a body's `struct` are answered by one map rather than by one map and an omission.
    let shadowed = body_declared_items(block);
    let merged: HashMap<String, String>;
    let uses: &HashMap<String, String> = if local_uses.is_empty() && shadowed.is_empty() {
        uses
    } else {
        let mut m = uses.clone();
        for n in &shadowed {
            // Instrument the PRECONDITION, not just the output (the standing bar): a REMOVAL that hits no
            // key changes nothing, so only a removal that actually took a binding away counts as this
            // branch having fired. `CANDOR_ALIAS_DEBUG` is the same switch scan.rs's R105 counter uses.
            if m.insert(n.clone(), format!("{ITEM_SENTINEL}{n}")).is_some()
                && std::env::var("CANDOR_ALIAS_DEBUG").is_ok()
            {
                eprintln!("BODYSHADOW {qual} :: {n}");
            }
        }
        // A body-level `use` is itself a body declaration, so it wins over the removal above — that is
        // what `use` means, and it is the ADD direction, which cannot lose an effect.
        m.extend(local_uses);
        merged = m;
        &merged
    };
    // Dispatch-typing WINS where both could apply: `x: X` under `X: Store` also looks like a
    // concrete type `X` to `type_path` (and `Box<dyn Store>` looks like `Box`), which would shadow
    // the CHA route with a meaningless receiver type.
    let trait_vars = seed_trait_vars(sig);
    let fn_typed_vars = seed_fn_typed_vars(sig);
    let mut vars = seed_vars(sig, self_ty, uses);
    for k in trait_vars.keys() {
        vars.remove(k);
    }
    // A fn-typed param (`cb: Box<dyn Fn()>` reads as `Box` via type_path; `impl Fn` lands in trait_vars)
    // must not be treated as a concrete/dispatch receiver — `cb()` is an opaque-callback invocation, not
    // a method call on it. Drop it from both so the call-site `fn_typed_vars` check owns it.
    for k in &fn_typed_vars {
        vars.remove(k);
    }
    // Seed element types for COLLECTION params (`fn f(xs: &[Sender])` → `xs`'s element is `Sender`)
    // and bind single-ident elements of a TUPLE param (`fn f((s, _): (Sender, usize))` → `s`).
    let (elem_of, tuple_of, elem_trait_of, tuple_trait_of) = seed_elem_of(sig, &mut vars, uses);
    let escapes = crate::lang::escaping_ctor_leaves(block, uses, fields);
    let mut c = CallCollector {
        modpath: modpath.to_string(),
        uses,
        vars,
        trait_vars,
        // The `dyn`-spelled (type-ERASED) subset of the same bounds — the imported-trait CHA (R4) fires
        // only on these, never on a caller-monomorphized generic bound / `impl Trait`.
        dyn_sig_traits: crate::lang::dyn_sig_trait_leaves(sig),
        // The FULL bound map (not just its erased subset), for the one position Pass A cannot reach:
        // a LOCAL `let`'s type annotation.
        generic_bounds: crate::lang::generic_bounds_of(sig),
        // The crate-qualified spelling of any bound written in full (`&dyn deplib::Handler`) — R6.
        trait_quals: crate::lang::sig_trait_quals(sig),
        trait_quals_by_param: crate::lang::sig_trait_quals_by_param(sig),
        fields,
        trait_fields: traits.fields,
        trait_impls: traits.impls,
        local_traits: traits.decls,
        returns,
        // Crate-wide: does any factory return a `<dyn>` dispatch object? Cheap `any` — keeps the
        // `resolve_recv_traits` hot-path guard closed on the overwhelming majority of crates.
        has_dyn_return: returns.values().any(|t| ret_dyn_leaves(t).is_some()),
        field_elem: elems.field_elem,
        field_elem_trait: elems.field_elem_trait,
        enum_variants: elems.enum_variants,
        enum_variant_traits: elems.enum_variant_traits,
        ambiguous_enum_leaves: elems.ambiguous_enum_leaves,
        elem_of,
        elem_trait_of,
        tuple_of,
        tuple_trait_of,
        calls: Vec::new(),
        closure_vars: std::collections::HashSet::new(),
        fn_typed_vars,
        // Empty at entry: filled as `let`s are visited (DEP-RECEIVER-TYPING-DESIGN.md half 1).
        dep_bound_vars: HashMap::new(),
        fn_alias: std::collections::HashMap::new(),
        lazy_statics,
        forced_lazies: std::collections::HashSet::new(),
        unresolved: false,
        err_ret_leaf: result_err_leaf(&sig.output, uses),
        const_strings,
        str_locals: std::collections::HashMap::new(),
        local_macros,
        macro_expanding: std::collections::HashSet::new(),
        // Empty at entry: filled as body-level `use` items are visited (`visit_item_use`).
        local_uses: std::collections::HashMap::new(),
        bound_names: bound_idents(sig, block),
        dispatch_sites: std::collections::BTreeSet::new(),
        drop_relevant,
        // Computed ONCE per body, before the walk, because the answer is a property of the whole
        // function (a value constructed on line 2 may escape through a `return` on line 40) and the
        // visitor sees each expression without its ancestors. (See `escapes`, hoisted above.)
        escaping_ctors: escapes.leaves,
        marked_ctors: std::collections::HashSet::new(),
        marked_cross_ctors: std::collections::HashSet::new(),
        in_pattern: false,
    };
    // PARAMETER-OWNED DROP, marked before the walk: a by-value parameter of a drop-relevant type dies
    // in THIS scope, and no construction expression in this body says so. Same marker, same consumer —
    // the marker's meaning is "a value of this type is RELEASED here", of which construction is the
    // dominant but not the only cause.
    for leaf in crate::lang::owned_drop_params(sig, self_ty, uses, &escapes.names) {
        c.note_construction(Some(leaf));
    }
    for stmt in &block.stmts {
        c.visit_stmt(stmt);
    }
    let ret_idents = match &sig.output {
        syn::ReturnType::Type(_, ty) => {
            let mut v = Vec::new();
            collect_type_idents(ty, &mut v);
            // A constructor commonly returns `Self` (`fn make(..) -> Self`, `fn new() -> Self`) — resolve it
            // to the impl's own type so the drop-glue ESCAPE GATE sees that a `Self`-returning `Stream::make`
            // returns the very Stream it built (else the FFI-Drop is fabricated onto the constructor, R49).
            if let Some(t) = self_ty {
                for id in &mut v {
                    if id == "Self" {
                        *id = t.rsplit("::").next().unwrap_or(t).to_string();
                    }
                }
            }
            v
        }
        syn::ReturnType::Default => Vec::new(),
    };
    // ⟨typeSurface.returns⟩ What a caller's `let x = f()` binding would HOLD. Not `ret_idents` (which
    // keeps the wrapper's idents beside the payload's) and not `rets`/`ReturnIndex` (leaf-keyed, and it
    // UNWRAPS `Result`/`Option` — exactly the lie the reverted attempt published across the boundary).
    let ret_bound_type = match &sig.output {
        syn::ReturnType::Type(_, ty) => bound_return_type(ty, uses, self_ty, modpath),
        syn::ReturnType::Default => None,
    };
    FnInfo {
        qual: qual.to_string(),
        leaf: leaf.to_string(),
        loc: loc.to_string(),
        calls: c.calls,
        unresolved: c.unresolved,
        ret_idents,
        ret_bound_type,
        dispatch: c.dispatch_sites.into_iter().collect(),
    }
}

/// Collect the nominal-type IDENTS of a `syn::Type` (`Result<Compress, E>` → `[Result, Compress, E]`),
/// peeling references / tuples / slices / arrays / generic args. Used by the drop-glue escape gate to see
/// which drop-types a fn's RETURN type owns (and thus which escape the scope).
pub(crate) fn collect_type_idents(ty: &syn::Type, out: &mut Vec<String>) {
    match ty {
        syn::Type::Path(p) => {
            for seg in &p.path.segments {
                out.push(seg.ident.to_string());
                if let syn::PathArguments::AngleBracketed(a) = &seg.arguments {
                    for arg in &a.args {
                        if let syn::GenericArgument::Type(t) = arg {
                            collect_type_idents(t, out);
                        }
                    }
                }
            }
        }
        syn::Type::Reference(r) => collect_type_idents(&r.elem, out),
        syn::Type::Paren(p) => collect_type_idents(&p.elem, out),
        syn::Type::Group(g) => collect_type_idents(&g.elem, out),
        syn::Type::Tuple(t) => t.elems.iter().for_each(|e| collect_type_idents(e, out)),
        syn::Type::Slice(s) => collect_type_idents(&s.elem, out),
        syn::Type::Array(a) => collect_type_idents(&a.elem, out),
        _ => {}
    }
}

/// Record `fn-leaf -> return type` into `rets`, tracking ambiguity: a leaf seen with two different
/// return types is set to `None` (dropped later), so only UNAMBIGUOUS names survive. Result/Option are
/// unwrapped to the success type.
pub(crate) fn record_return(
    sig: &syn::Signature,
    uses: &HashMap<String, String>,
    rets: &mut HashMap<String, Option<String>>,
    self_ty: Option<&str>,
) {
    let syn::ReturnType::Type(_, ty) = &sig.output else { return };
    // A factory that returns a CALLABLE (`fn make_cb() -> Box<dyn Fn()>` / `-> fn()` / `-> impl Fn()`):
    // record the FN-TYPED sentinel under its leaf so `let g = make_cb(); g()` reads the honest opaque
    // call (Unknown), not a phantom free-fn `g`. A bare `fn()`/`impl Fn` has no path so `type_path` would
    // drop it entirely (silent-pure); `Box<dyn Fn>` would record as "Box" and mis-resolve `g.method()`.
    // The sentinel rides the SAME ambiguity rule as a type (a leaf seen callable AND non-callable, or two
    // shapes, collapses to None → no claim) and is filtered out of var-typing in `ctor_type`. Over-
    // approximating toward fn-typed only ever marks the binding Unknown — the safe direction.
    if is_callable_type(unwrap_result_option(ty), &generic_bounds_of(sig)) {
        let leaf = sig.ident.to_string();
        match rets.get(&leaf) {
            None => { rets.insert(leaf, Some(RET_FN_TYPED.to_string())); }
            Some(Some(prev)) if prev != RET_FN_TYPED => { rets.insert(leaf, None); }
            _ => {}
        }
        return;
    }
    // A DISPATCH trait-object return (`-> Box<dyn Task>` / `-> impl Task` / `-> &dyn Task`): `type_path`
    // drops it (no nominal type), so `get().run()` on the factory typed to nothing and dropped SILENT-
    // PURE. Record the trait bound leaves under a `<dyn>` sentinel so the call-site runs the same
    // bounded-CHA the direct trait-object receiver does — edging to every local implementor, or Unknown.
    // Rides the SAME ambiguity rule as a nominal type (a leaf recorded with two different shapes → None).
    // `trait_leaves` peels the Box/Rc/Arc/&/impl/dyn wrapper; empty = not a dispatch object (fall through).
    let dyn_leaves = trait_leaves(unwrap_result_option(ty), &generic_bounds_of(sig));
    if !dyn_leaves.is_empty() {
        let sentinel = ret_dyn_encode(&dyn_leaves);
        let leaf = sig.ident.to_string();
        match rets.get(&leaf) {
            None => {
                rets.insert(leaf, Some(sentinel));
            }
            Some(Some(prev)) if *prev != sentinel => {
                rets.insert(leaf, None);
            }
            _ => {}
        }
        return;
    }
    // A COLLECTION-OF-TRAIT-OBJECTS return (`fn all() -> Vec<Box<dyn Task>>`): `type_path` records it as
    // the useless "Vec", so `for d in all() { d.run() }` dropped the element dispatch. Record the ELEMENT
    // bound leaves under the distinct `<elemdyn>` sentinel (decoded by `resolve_elem_trait_leaves`); the
    // scalar-`<dyn>` check above already claimed a direct `-> Box<dyn>` return, so this only sees genuine
    // collections. Rides the same ambiguity rule (two shapes for a leaf → None).
    let elem_dyn = elem_trait_leaves(unwrap_result_option(ty), &generic_bounds_of(sig));
    if !elem_dyn.is_empty() {
        let sentinel = ret_elem_dyn_encode(&elem_dyn);
        let leaf = sig.ident.to_string();
        match rets.get(&leaf) {
            None => {
                rets.insert(leaf, Some(sentinel));
            }
            Some(Some(prev)) if *prev != sentinel => {
                rets.insert(leaf, None);
            }
            _ => {}
        }
        return;
    }
    // A TUPLE-WITH-TRAIT-OBJECT return (`fn make() -> (Box<dyn Doer>, u32)`): `type_path` drops it (a tuple
    // has no nominal path), so `let (d, _) = make(); d.go()` dropped SILENT-PURE. Record the per-position
    // bound leaves under a `<tupledyn>` sentinel so the destructure binds each dyn position into
    // `trait_vars` (R46 tuple). Rides the same ambiguity rule (two shapes for a leaf → None).
    if let Some(positions) = tuple_trait_leaves(unwrap_result_option(ty), &generic_bounds_of(sig)) {
        let sentinel = ret_tuple_dyn_encode(&positions);
        let leaf = sig.ident.to_string();
        match rets.get(&leaf) {
            None => {
                rets.insert(leaf, Some(sentinel));
            }
            Some(Some(prev)) if *prev != sentinel => {
                rets.insert(leaf, None);
            }
            _ => {}
        }
        return;
    }
    let Some(mut tp) = type_path(unwrap_result_option(ty), uses) else { return };
    // An impl method returning `Self` (`fn new_with_defaults() -> Self`) must index its IMPL type,
    // not the literal "Self": vars typed "Self" form `Self::method` calls that resolve to no local
    // def — so an ordinary `let agent = Agent::new_with_defaults(); agent.run(..)` silently dropped
    // its edge (found by the PROVE-IT dogfood on ureq: 3 public API entry points missing from a
    // blast radius). Worse, two same-named ctors on DIFFERENT types both recording "Self" defeated
    // the ambiguity check. `Result<Self>`/`Option<Self>` arrive here already unwrapped.
    if tp == "Self" {
        match self_ty {
            Some(s) => tp = s.to_string(),
            None => return, // `Self` outside an impl — nothing safe to record
        }
    }
    let leaf = sig.ident.to_string();
    match rets.get(&leaf) {
        None => {
            rets.insert(leaf, Some(tp));
        }
        Some(Some(prev)) if *prev != tp => {
            rets.insert(leaf, None); // conflicting return types — ambiguous, drop
        }
        _ => {}
    }
}

/// Pre-pass: index struct field types (`App -> { http: reqwest::Client }`) AND function return types
/// (`create_pool -> sqlx::Pool`), expanded via each module's `use` map. Recurses into modules like
/// `scan_items`. Field index keyed by struct leaf; return map keyed by fn leaf (ambiguous names dropped
/// by the caller). A name collision is rare and at worst yields a wrong (still verb-gated) classify.
#[allow(clippy::too_many_arguments)]
pub(crate) fn collect_decls(
    items: &[syn::Item],
    include_tests: bool,
    uses: &mut HashMap<String, String>,
    fields: &mut FieldIndex,
    field_elem: &mut FieldElemIndex,
    field_elem_trait: &mut FieldElemTraitIndex,
    rets: &mut HashMap<String, Option<String>>,
    enum_tmp: &mut HashMap<String, Option<String>>,
    enum_variant_traits: &mut HashMap<String, Option<Vec<String>>>,
    trait_impls: &mut TraitImplIndex,
    local_traits: &mut HashMap<String, LocalTrait>,
    trait_fields: &mut TraitFieldIndex,
    prim_aliases: &mut std::collections::HashSet<String>,
    extern_fns: &mut std::collections::HashSet<String>,
    drop_types: &mut std::collections::HashSet<String>,
    deref_target: &mut HashMap<String, String>,
    lazy_statics: &mut std::collections::HashSet<String>,
    const_strings: &mut HashMap<String, String>,
    local_macros: &mut HashMap<String, String>,
    blanket_methods: &mut HashMap<String, String>,
) {
    for it in items {
        if let syn::Item::Use(u) = it {
            collect_use(&u.tree, String::new(), uses);
        }
    }
    for it in items {
        // LAZY/deferred static NAME collection (crate-wide) — a forcing site (any fn naming the static)
        // edges to its synthetic init unit, and the forcing site lives anywhere, so the name set must be
        // crate-wide (Pass A), exactly like the trait/return/extern indexes. The synthetic UNIT itself is
        // emitted in `scan_items`; this only records WHICH names are lazy statics so `CallCollector` can
        // recognise a force. `#[cfg(test)]`-gated statics are excluded unless tests are included.
        if lazy_unit_emitted(it, include_tests) {
            if let Some((name, _)) = lazy_static_unit(it) {
                lazy_statics.insert(name);
            }
        }
        // CONST-STRING index (crate-wide, Pass A) — `const NAME: &str = "lit"` / `static NAME: … = "lit"`
        // whose initializer is a PLAIN string literal (or a trivial `concat!` of literals). Keyed by NAME
        // (leaf); a later `client.post(format!("{}/x", NAME))` / `post(NAME)` resolves the host from this
        // map, feeding the SAME §1 host refinement as an inline literal (SPEC §1: a statically-known host,
        // even via a const, classifies Llm — parity with candor-java, where javac inlines a `static final
        // String`). ONLY literal-valued consts land here — a runtime-valued const contributes nothing, so
        // its call stays bare Net with the host masked (no fabrication). `#[cfg(test)]`-gated consts are
        // excluded from a production scan, matching the lazy-static / field discipline above.
        match it {
            syn::Item::Const(c) if include_tests || !is_cfg_test(&c.attrs) => {
                if let Some(v) = const_str_value(&c.expr) {
                    const_strings.insert(c.ident.to_string(), v);
                }
            }
            syn::Item::Static(s) if include_tests || !is_cfg_test(&s.attrs) => {
                if let Some(v) = const_str_value(&s.expr) {
                    const_strings.insert(s.ident.to_string(), v);
                }
            }
            // LOCAL `macro_rules! NAME { (..) => { TEMPLATE }; .. }` — record NAME → the arm TOKENS (as a
            // string, cacheable) so a bare `NAME!(..)` can INLINE-EXPAND the template and see any I/O / local
            // call hidden in it. Without this a metavar-free effectful macro (`macro_rules! do_io { () => {
            // fs::write(..) } }`) read silent-pure (R48). Only a `macro_rules!` DEFINITION (which carries an
            // `ident`); an item-position macro INVOCATION (`foo!();`) has `ident: None` and is skipped.
            syn::Item::Macro(m)
                if (include_tests || !is_cfg_test(&m.attrs))
                    && m.ident.is_some()
                    && m.mac.path.is_ident("macro_rules") =>
            {
                if let Some(name) = &m.ident {
                    local_macros.insert(name.to_string(), m.mac.tokens.to_string());
                }
            }
            _ => {}
        }
        match it {
            syn::Item::Struct(s) => {
                // the struct's OWN generic bounds (`struct Pipe<T: Saver>` / `where T: Saver`) — so a field
                // typed as a bounded param resolves to its trait bound and dispatches (R31).
                let struct_bounds = generic_bounds_of_generics(&s.generics);
                match &s.fields {
                    syn::Fields::Named(named) => {
                        let entry = fields.entry(s.ident.to_string()).or_default();
                        for f in &named.named {
                            // Skip `#[cfg(...)]`-gated fields: they aren't unconditionally present, so
                            // inferring effects through them mis-fires. (tokio's `resource_span:
                            // tracing::Span`, gated on the off-by-default `tracing` feature, otherwise made
                            // every `self.resource_span.in_scope(..)` read as Log — 452 phantom functions.)
                            if has_cfg(&f.attrs) {
                                continue;
                            }
                            if let Some(name) = &f.ident {
                                // Dispatch-typing first: `store: Box<dyn Store>` reads as concrete
                                // `Box` to `type_path`, which would shadow the CHA route.
                                let leaves = trait_leaves(&f.ty, &struct_bounds);
                                if !leaves.is_empty() {
                                    trait_fields
                                        .entry(s.ident.to_string())
                                        .or_default()
                                        .insert(name.to_string(), leaves);
                                } else if let Some(ty) = type_path(&f.ty, uses) {
                                    entry.insert(name.to_string(), ty);
                                }
                                // A COLLECTION field (`senders: Vec<Sender>`) records its element type so
                                // `self.senders[0].send()` / `for c in &self.senders` resolve the element.
                                if let Some(e) = elem_type(&f.ty, uses) {
                                    field_elem
                                        .entry(s.ident.to_string())
                                        .or_default()
                                        .insert(name.to_string(), e);
                                }
                                // A COLLECTION-OF-TRAIT-OBJECTS field (`handlers: Vec<Box<dyn Handler>>`, or
                                // `Vec<T>` on `struct Registry<T: Handler>`) records its element DISPATCH
                                // leaves so `self.handlers.iter().for_each(|h| h.handle())` dispatches (R37
                                // field form). Uses the struct's own generic bounds for a bounded element.
                                let leaves = elem_trait_leaves(&f.ty, &struct_bounds);
                                if !leaves.is_empty() {
                                    field_elem_trait
                                        .entry(s.ident.to_string())
                                        .or_default()
                                        .insert(name.to_string(), leaves);
                                }
                            }
                        }
                    }
                    // TUPLE structs index by position (`"0"`, `"1"`), so a newtype-wrapped receiver
                    // (`self.0.run()`, chained `self.0.0`) resolves like a named field. Same
                    // `#[cfg]` rule.
                    syn::Fields::Unnamed(unnamed) => {
                        let entry = fields.entry(s.ident.to_string()).or_default();
                        for (i, f) in unnamed.unnamed.iter().enumerate() {
                            if has_cfg(&f.attrs) {
                                continue;
                            }
                            if let Some(ty) = type_path(&f.ty, uses) {
                                entry.insert(i.to_string(), ty);
                            }
                            if let Some(e) = elem_type(&f.ty, uses) {
                                field_elem
                                    .entry(s.ident.to_string())
                                    .or_default()
                                    .insert(i.to_string(), e);
                            }
                            let leaves = elem_trait_leaves(&f.ty, &struct_bounds);
                            if !leaves.is_empty() {
                                field_elem_trait
                                    .entry(s.ident.to_string())
                                    .or_default()
                                    .insert(i.to_string(), leaves);
                            }
                        }
                    }
                    syn::Fields::Unit => {}
                }
            }
            syn::Item::Fn(f) => record_return(&f.sig, uses, rets, None),
            // Enum SINGLE-PAYLOAD tuple variants (`enum Conn { Active(Sender) }`) — index `variant
            // leaf -> payload type` so a match arm `Conn::Active(s) => s.send()` types `s`. Only the
            // single-field tuple form is recorded; a leaf two enums share with conflicting payloads is
            // marked ambiguous (None) and dropped by the caller — never guess (the return-index rule).
            // STRUCT variants (`enum Msg { CbField { f: T } }`) are indexed too, per FIELD, under a
            // composite `"VariantLeaf::field"` key in these SAME two maps — see the `Fields::Named` arm
            // below (R77 residual).
            syn::Item::Enum(en) => {
                // R77: the enum's OWN generic bounds (`enum Msg<T: Doer> { Cb(T) }`), mirroring the
                // struct field route just above — a bounded-generic single-field payload dispatches too.
                let enum_bounds = generic_bounds_of_generics(&en.generics);
                for v in &en.variants {
                    if has_cfg(&v.attrs) {
                        continue;
                    }
                    match &v.fields {
                        syn::Fields::Unnamed(unnamed) if unnamed.unnamed.len() == 1 => {
                            let payload_ty = &unnamed.unnamed[0].ty;
                            let leaf = v.ident.to_string();
                            // R77: DISPATCH-TYPING FIRST — same precedence as the struct-field route just
                            // above ("Dispatch-typing first: `store: Box<dyn Store>` reads as concrete
                            // `Box` to `type_path`"). A BOUNDED GENERIC payload (`enum Msg<F: FnOnce() ->
                            // V> { Function(F) }`) is worse than that struct case: `type_path` doesn't
                            // just mis-name it, it SUCCEEDS — a bare generic ident IS a `Type::Path`, so
                            // `type_path` returns the literal, USELESS string `"F"` as if it were a real
                            // nominal type. Recording that into `enum_tmp` alongside the correct
                            // `enum_variant_traits` entry made this ONE variant collide with ITSELF
                            // crate-wide once R77's cross-index ambiguity guard shipped (moka's
                            // `ValueOrFunction::Function(F: FnOnce() -> V)`, measured in the 256-crate
                            // A/B): both maps had "Function", so the guard — designed for two DIFFERENT
                            // enums sharing a leaf — dropped a single variant's own correct dispatch claim
                            // as if it were a foreign collision. `else if` (not two independent `if`s)
                            // makes them mutually exclusive PER OCCURRENCE, exactly like the struct-field
                            // route, so a bounded-generic payload only ever contributes to
                            // `enum_variant_traits`, never also to `enum_tmp`.
                            let leaves = trait_leaves(payload_ty, &enum_bounds);
                            if !leaves.is_empty() {
                                match enum_variant_traits.get(&leaf) {
                                    None => {
                                        enum_variant_traits.insert(leaf, Some(leaves));
                                    }
                                    Some(Some(prev)) if *prev != leaves => {
                                        enum_variant_traits.insert(leaf, None); // conflicting leaf sets — ambiguous, drop
                                    }
                                    _ => {}
                                }
                            } else if let Some(tp) = type_path(payload_ty, uses) {
                                match enum_tmp.get(&leaf) {
                                    None => {
                                        enum_tmp.insert(leaf.clone(), Some(tp));
                                    }
                                    Some(Some(prev)) if *prev != tp => {
                                        enum_tmp.insert(leaf.clone(), None); // conflicting payloads — ambiguous, drop
                                    }
                                    _ => {}
                                }
                            }
                        }
                        // R77 STRUCT-VARIANT FIELDS (the remaining half of the vein — SOUNDNESS.md R77):
                        // `enum Msg { CbField { f: Box<dyn Fn()> } }`. No binder mechanism existed for
                        // ANY struct-variant field before this, callable or concrete. Rather than a THIRD
                        // index, this writes into the SAME `enum_tmp`/`enum_variant_traits` maps the
                        // tuple-variant route above uses, under the composite key `"VariantLeaf::field"`
                        // — a Rust identifier can never contain `::`, so this key space is provably
                        // disjoint from every bare tuple-variant leaf already stored there (no new
                        // collision surface, and the EXISTING `drop_cross_ambiguous_enum_leaves` guard —
                        // and the EXISTING digest/merge/cache plumbing — cover it for free; see
                        // `lang::struct_variant_field_bindings` and `collector::enum_struct_variant_bindings`
                        // for the read side). Per-field, same dispatch-typing-first / `else if`
                        // mutual-exclusion precedence as every other route in this function; a `#[cfg]`
                        // field is skipped, matching the struct-field discipline above.
                        syn::Fields::Named(named) => {
                            let leaf = v.ident.to_string();
                            for f in &named.named {
                                if has_cfg(&f.attrs) {
                                    continue;
                                }
                                let Some(field_name) = &f.ident else { continue };
                                let key = format!("{leaf}::{field_name}");
                                let leaves = trait_leaves(&f.ty, &enum_bounds);
                                if !leaves.is_empty() {
                                    match enum_variant_traits.get(&key) {
                                        None => {
                                            enum_variant_traits.insert(key, Some(leaves));
                                        }
                                        Some(Some(prev)) if *prev != leaves => {
                                            enum_variant_traits.insert(key, None); // conflicting leaf sets — ambiguous, drop
                                        }
                                        _ => {}
                                    }
                                } else if let Some(tp) = type_path(&f.ty, uses) {
                                    match enum_tmp.get(&key) {
                                        None => {
                                            enum_tmp.insert(key.clone(), Some(tp));
                                        }
                                        Some(Some(prev)) if *prev != tp => {
                                            enum_tmp.insert(key.clone(), None); // conflicting payloads — ambiguous, drop
                                        }
                                        _ => {}
                                    }
                                }
                            }
                        }
                        _ => {}
                    }
                }
            }
            syn::Item::Type(it) => {
                // A type alias to a NON-NOMINAL type (`type Inner = [u8; N]`, a slice/tuple/ptr/ref/fn,
                // or a bare primitive) names a type that has NO local impl block — so a call
                // `Inner::assoc()` resolves through the std/core inherent impl, NOT a same-named local
                // STRUCT's associated fn. Recording the alias lets resolution SKIP the local link, which
                // would otherwise fabricate the struct's effect: sled's `type Inner = [u8; CUTOFF]`
                // collided with `struct Inner`'s effectful `Default` (gen_temp_path → Clock+Env), so the
                // pure `IVec::inline`/`subslice` (calling the array's `Inner::default()`) inherited both.
                if is_non_nominal_type(&it.ty) {
                    prim_aliases.insert(it.ident.to_string());
                }
            }
            syn::Item::Trait(t) => {
                let e = local_traits.entry(t.ident.to_string()).or_default();
                e.count += 1;
                for ti in &t.items {
                    if let syn::TraitItem::Fn(m) = ti {
                        // `methods` holds only `&self`/`self` DISPATCH methods (its two uses — CHA on
                        // `t.method()` and the R36 trait-default fallback — are both receiver calls). An
                        // ASSOCIATED fn (`fn new()`, no receiver) is excluded, so the UFCS resolver (R53)
                        // never mis-reads `Trait::assoc(&x)` as a receiver call on `x`.
                        if matches!(m.sig.inputs.first(), Some(syn::FnArg::Receiver(_))) {
                            e.methods.insert(m.sig.ident.to_string());
                        }
                    }
                }
                // Supertrait bounds (`trait Sub: Super`) — a Super method is callable on a Sub receiver.
                for s in bound_leaves(&t.supertraits) {
                    if !e.supertraits.contains(&s) {
                        e.supertraits.push(s);
                    }
                }
            }
            // A foreign (`extern "C" { fn system(..); }`) function declaration: the body lives in
            // another language and is the canonical unknowable boundary. Record every declared name so a
            // safe-wrapper call to it (`unsafe { system(cmd) }`) DISCLOSES Unknown instead of reading
            // silent-pure — a bare leaf with no local def and no classification would otherwise fall
            // through to pure (the FFI safe-wrapper under-report). We can't know the effect (Fs vs Net vs
            // Exec), so Unknown is the honest signal, exactly as for an unresolved callback.
            syn::Item::ForeignMod(fm) => {
                for fi in &fm.items {
                    if let syn::ForeignItem::Fn(f) = fi {
                        extern_fns.insert(f.sig.ident.to_string());
                    }
                }
            }
            syn::Item::Impl(im) => {
                let self_ty = impl_type_name(&im.self_ty);
                // BLANKET impl (`impl<T> Trait for T` / `impl<T: Bound> Trait for T`): the self type IS one of
                // the impl's own generic type params, so the impl provides `Trait`'s methods for EVERY type
                // (bounded → every type meeting the bound). A `x.method()` that resolves to no CONCRETE
                // `X::method` is then this blanket body — but its qual is `<param>::method` (`T::method`), so
                // a keyed lookup on the receiver's type missed it silent-pure (R45). Record `method-leaf ->
                // the blanket self-param name` so scan.rs can edge an unresolved call to the blanket body via
                // `by_tail2["<param>::method"]`. Ambiguous (two blankets share a leaf) → "" sentinel, dropped.
                if let (Some((None, _, _)), Some(sty)) = (&im.trait_, &self_ty) {
                    let is_blanket = matches!(&*im.self_ty, syn::Type::Path(p)
                        if p.qself.is_none()
                            && p.path.get_ident().is_some_and(|id| im.generics.params.iter().any(|g|
                                matches!(g, syn::GenericParam::Type(t) if &t.ident == id))));
                    if is_blanket {
                        for ii in &im.items {
                            if let syn::ImplItem::Fn(m) = ii {
                                let leaf = m.sig.ident.to_string();
                                match blanket_methods.get(&leaf) {
                                    Some(prev) if prev != sty => { blanket_methods.insert(leaf, String::new()); }
                                    Some(_) => {}
                                    None => { blanket_methods.insert(leaf, sty.clone()); }
                                }
                            }
                        }
                    }
                }
                // `impl Trait for Type` — a CHA edge from the trait leaf to the implementing type.
                if let (Some((_, tr, _)), Some(ty)) = (&im.trait_, &self_ty) {
                    if let Some(leaf) = tr.segments.last() {
                        trait_impls.entry(leaf.ident.to_string()).or_default().push(ty.clone());
                        // A LOCAL `impl Drop for Type` — its `drop` body runs at scope exit, an implicit
                        // edge the syntactic call graph doesn't otherwise model. Record the type leaf so a
                        // fn that BINDS a value of this type inherits the (already-scanned) `Type::drop`
                        // body's effects (drop-glue under-report). LOCAL types only — never fabricate a drop
                        // edge for an external type whose Drop we can't see.
                        if leaf.ident == "Drop" {
                            drop_types.insert(ty.clone());
                        }
                        // A LOCAL `impl Deref for T { type Target = U }` — `t.method()` AUTO-DEREFS to U's
                        // method (Rust auto-deref). Record T-leaf -> U-leaf so a method that resolves on no
                        // `T::method` retries on `U::method` (the user-Deref analog of the Box/Arc/Rc peel; a
                        // newtype `impl Deref` dropped `wrapper.method()` to silent-pure — corpus find).
                        // `.clone()` stays guarded elsewhere, so the pointer-clone fabrication can't recur.
                        if leaf.ident == "Deref" {
                            for it in &im.items {
                                if let syn::ImplItem::Type(at) = it {
                                    if at.ident == "Target" {
                                        if let Some(tp) = type_path(&at.ty, uses) {
                                            let tl = tp.rsplit("::").next().unwrap_or(&tp).to_string();
                                            let kl = ty.rsplit("::").next().unwrap_or(ty).to_string();
                                            deref_target.insert(kl, tl);
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
                for ii in &im.items {
                    if let syn::ImplItem::Fn(m) = ii {
                        record_return(&m.sig, uses, rets, self_ty.as_deref());
                    }
                    // `impl X { const BASE: &str = "https://api.openai.com/v1"; }` — an associated const
                    // string. Indexed by its LEAF (`BASE`) exactly like a module const, so a body's
                    // `format!("{}/chat", Self::BASE)` / `format!("{}/chat", X::BASE)` (both resolve by
                    // leaf) picks it up. Literal-valued only, same soundness gate.
                    if let syn::ImplItem::Const(cst) = ii {
                        if let Some(v) = const_str_value(&cst.expr) {
                            const_strings.insert(cst.ident.to_string(), v);
                        }
                    }
                }
            }
            syn::Item::Mod(m) => {
                // Skip `#[cfg(test)]` modules here too (Pass B / scan_items already does): otherwise a
                // test module's struct fields and fn return types leak into the crate-wide index and get
                // used to type PRODUCTION code (e.g. `struct App { http: MockClient }` in `mod tests`
                // colliding with the real App).
                if !include_tests && is_cfg_test(&m.attrs) {
                    continue;
                }
                if let Some((_, inner)) = &m.content {
                    // Same shadowing rule as `scan_items` — the DECL indexes (field types, return types)
                    // are built through this map too, so leaving it un-shadowed would type a submodule's
                    // own `Command` FIELD as std's even after Pass B stopped doing it for parameters.
                    let mut subuses = submodule_uses(uses, inner, include_tests);
                    collect_decls(inner, include_tests, &mut subuses, fields, field_elem, field_elem_trait, rets, enum_tmp, enum_variant_traits, trait_impls, local_traits, trait_fields, prim_aliases, extern_fns, drop_types, deref_target, lazy_statics, const_strings, local_macros, blanket_methods);
                }
            }
            _ => {}
        }
    }
}
