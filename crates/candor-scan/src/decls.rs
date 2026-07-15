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
                out.push(fninfo(&n, &qual(&n), &loc, &f.sig, &f.block, None, uses, fields, returns, traits, elems, lazy_statics, const_strings));
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
                        out.push(fninfo(&n, &q, &loc, &m.sig, &m.block, tyname.as_deref(), uses, fields, returns, traits, elems, lazy_statics, const_strings));
                    }
                }
            }
            syn::Item::Mod(m) => {
                if !include_tests && is_cfg_test(&m.attrs) {
                    continue; // a #[cfg(test)] module — its effects are the tests', not the crate's
                }
                if let Some((_, inner)) = &m.content {
                    let sub = qual(&m.ident.to_string());
                    let mut subuses = uses.clone();
                    scan_items(inner, &sub, locs, loc_idx, include_tests, fields, returns, traits, elems, lazy_statics, const_strings, &mut subuses, out);
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
                        out.push(fninfo(&n, &qual(&format!("{tname}::{n}")), &loc, &m.sig, block,
                            Some(&tname), uses, fields, returns, traits, elems, lazy_statics, const_strings));
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
                let q = format!("{LAZY_UNIT_PREFIX}::{name}");
                out.push(fninfo(&name, &q, &loc, &sig, &block, None, uses, fields, returns, traits, elems, lazy_statics, const_strings));
            }
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
pub(crate) fn seed_elem_of(
    sig: &syn::Signature,
    vars: &mut HashMap<String, String>,
    uses: &HashMap<String, String>,
) -> (HashMap<String, String>, TupleElemIndex) {
    let mut elem_of = HashMap::new();
    let mut tuple_of: TupleElemIndex = HashMap::new();
    for arg in &sig.inputs {
        let syn::FnArg::Typed(pt) = arg else { continue };
        match &*pt.pat {
            syn::Pat::Ident(id) => {
                if let Some(e) = elem_type(&pt.ty, uses) {
                    elem_of.insert(id.ident.to_string(), e);
                }
                // `fn f(pair: (Sender, usize))` — record positions for a later `let (s, _) = pair`.
                if let Some(t) = tuple_types(&pt.ty, uses) {
                    tuple_of.insert(id.ident.to_string(), t);
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
    (elem_of, tuple_of)
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
    let merged: HashMap<String, String>;
    let uses: &HashMap<String, String> = if local_uses.is_empty() {
        uses
    } else {
        let mut m = uses.clone();
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
    let (elem_of, tuple_of) = seed_elem_of(sig, &mut vars, uses);
    let mut c = CallCollector {
        uses,
        vars,
        trait_vars,
        fields,
        trait_fields: traits.fields,
        trait_impls: traits.impls,
        local_traits: traits.decls,
        returns,
        field_elem: elems.field_elem,
        enum_variants: elems.enum_variants,
        elem_of,
        tuple_of,
        calls: Vec::new(),
        closure_vars: std::collections::HashSet::new(),
        fn_typed_vars,
        fn_alias: std::collections::HashMap::new(),
        lazy_statics,
        forced_lazies: std::collections::HashSet::new(),
        unresolved: false,
        err_ret_leaf: result_err_leaf(&sig.output, uses),
        const_strings,
        str_locals: std::collections::HashMap::new(),
    };
    for stmt in &block.stmts {
        c.visit_stmt(stmt);
    }
    FnInfo {
        qual: qual.to_string(),
        leaf: leaf.to_string(),
        loc: loc.to_string(),
        calls: c.calls,
        unresolved: c.unresolved,
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
    rets: &mut HashMap<String, Option<String>>,
    enum_tmp: &mut HashMap<String, Option<String>>,
    trait_impls: &mut TraitImplIndex,
    local_traits: &mut HashMap<String, LocalTrait>,
    trait_fields: &mut TraitFieldIndex,
    prim_aliases: &mut std::collections::HashSet<String>,
    extern_fns: &mut std::collections::HashSet<String>,
    drop_types: &mut std::collections::HashSet<String>,
    deref_target: &mut HashMap<String, String>,
    lazy_statics: &mut std::collections::HashSet<String>,
    const_strings: &mut HashMap<String, String>,
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
            syn::Item::Enum(en) => {
                for v in &en.variants {
                    if has_cfg(&v.attrs) {
                        continue;
                    }
                    let syn::Fields::Unnamed(unnamed) = &v.fields else { continue };
                    if unnamed.unnamed.len() != 1 {
                        continue;
                    }
                    let Some(tp) = type_path(&unnamed.unnamed[0].ty, uses) else { continue };
                    let leaf = v.ident.to_string();
                    match enum_tmp.get(&leaf) {
                        None => {
                            enum_tmp.insert(leaf, Some(tp));
                        }
                        Some(Some(prev)) if *prev != tp => {
                            enum_tmp.insert(leaf, None); // conflicting payloads — ambiguous, drop
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
                        e.methods.insert(m.sig.ident.to_string());
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
                    let mut subuses = uses.clone();
                    collect_decls(inner, include_tests, &mut subuses, fields, field_elem, rets, enum_tmp, trait_impls, local_traits, trait_fields, prim_aliases, extern_fns, drop_types, deref_target, lazy_statics, const_strings);
                }
            }
            _ => {}
        }
    }
}
