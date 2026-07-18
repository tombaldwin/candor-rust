//! Pass B: `CallCollector` walks one function body and records its calls, resolving
//! receivers through the Pass-A indexes. `resolve_target` is the shared call resolver.

use crate::*;

pub(crate) struct CallCollector<'a> {
    pub(crate) uses: &'a HashMap<String, String>,
    /// local variable / param / `self` -> expanded type path, grown as `let`s are visited in order.
    pub(crate) vars: HashMap<String, String>,
    /// local variable / param -> trait bound leaves, for dispatch-typed receivers (`t: &dyn Store`,
    /// `s: impl Store`, `x: X` under `X: Store`). Disjoint from `vars` (no concrete type to put there).
    pub(crate) trait_vars: HashMap<String, Vec<String>>,
    pub(crate) fields: &'a FieldIndex,
    pub(crate) trait_fields: &'a TraitFieldIndex,
    /// trait leaf -> local impl types (None entries never exist; absent = no local impl).
    pub(crate) trait_impls: &'a TraitImplIndex,
    /// leaf -> the local trait declaration(s) sharing it: ambiguity count + declared method names.
    pub(crate) local_traits: &'a HashMap<String, LocalTrait>,
    pub(crate) returns: &'a ReturnIndex,
    /// Whether ANY recorded factory return is a `<dyn>` dispatch-object sentinel — the hot-path guard in
    /// `resolve_recv_traits` keeps the factory-call arm live only when this holds (crate-wide, computed
    /// once), so a `get().run()` on a `-> Box<dyn Trait>` factory resolves rather than dropping pure.
    pub(crate) has_dyn_return: bool,
    /// `Type-leaf -> field -> element-type` for COLLECTION fields (`self.senders[0]`, `for c in
    /// &self.senders`). The field counterpart of `elem_of`, the way `fields` is to `vars`.
    pub(crate) field_elem: &'a FieldElemIndex,
    /// `enum-variant-leaf -> single payload type` for match-arm binding (`Conn::Active(s) => s.send()`).
    pub(crate) enum_variants: &'a EnumVariantIndex,
    /// local var / param -> ELEMENT type of a COLLECTION it holds (a `Vec<T>`/`&[T]`/… binding), grown
    /// as collection-typed `let`s/params are seen. Lets `for c in xs`, `xs[0]`, `xs.iter().for_each`
    /// resolve the element's type. Scoped bindings (loop var, closure param) live in `vars`, not here.
    pub(crate) elem_of: HashMap<String, String>,
    /// `Type -> { field -> element dispatch leaves }` for a COLLECTION-OF-TRAIT-OBJECTS FIELD
    /// (`self.handlers: Vec<Box<dyn Handler>>`) — the field counterpart of `elem_trait_of`, the way
    /// `field_elem` is to `elem_of`. Lets `self.handlers.iter().for_each(|h| h.handle())` dispatch.
    pub(crate) field_elem_trait: &'a FieldElemTraitIndex,
    /// local var / param -> the DISPATCH-trait leaves of a COLLECTION of trait objects it holds
    /// (`items: Vec<Box<dyn Doer>>` -> `["Doer"]`). The trait-object counterpart of `elem_of`: a
    /// `for it in items { it.go() }` types the loop var into `trait_vars` (bounded-CHA dispatch) instead
    /// of dropping to pure (`elem_of` can't hold a `dyn` element — it has no nominal type path).
    pub(crate) elem_trait_of: HashMap<String, Vec<String>>,
    /// local var / param -> the per-position types of a TUPLE it holds (`pair: (Sender, usize)` ->
    /// `[Some("Sender"), Some("usize")]`). Lets a later `let (s, _) = pair;` type each binding from the
    /// matching position. A `None` at a position = that element's type is unknown (binds nothing).
    pub(crate) tuple_of: HashMap<String, Vec<Option<String>>>,
    pub(crate) calls: Vec<Call>,
    /// locals bound to a closure (`let f = |..| ..`), so a later `f()` is recognised as a closure
    /// invocation the scan can't see through — not a call to a free fn named `f`.
    pub(crate) closure_vars: std::collections::HashSet<String>,
    /// params/locals of a fn-pointer / `impl`/`dyn Fn` / generic-`Fn`-bound type. Invoking one (`cb()`)
    /// calls an opaque body → honest `Unknown`, not a silently-dropped phantom call to a free fn `cb`.
    pub(crate) fn_typed_vars: std::collections::HashSet<String>,
    /// locals aliased to a free-FUNCTION path (`let g = eff;` where `eff` is a visible fn): a later `g()`
    /// resolves to the aliased path, so its effect (and whole transitive chain) is not silently dropped
    /// (sweep [6]). Keyed by the local name → the expanded callee path.
    pub(crate) fn_alias: std::collections::HashMap<String, String>,
    /// Crate-wide LAZY/deferred static names (`once_cell`/`std` `Lazy`/`LazyLock`/`LazyCell`,
    /// `lazy_static!`, `thread_local!`). A body that NAMES one of these FORCES its deferred init on
    /// first use — so naming the static edges to its synthetic init unit (`<lazy>::NAME`), carrying the
    /// init's effect to this fn. Over-approximating "names ⇒ forces" is a SAFE over-approximation (the
    /// init does run on first use), never a fabrication. Keyed per static NAME (not module-scoped), so a
    /// pure-init lazy contributes nothing. Set once per forcing site (de-duped via `forced_lazies`).
    pub(crate) lazy_statics: &'a std::collections::HashSet<String>,
    /// Lazy statics already FORCED (edged) in this body — emit at most one forcing edge per static, so a
    /// hot static read in a loop doesn't bloat the call list.
    pub(crate) forced_lazies: std::collections::HashSet<String>,
    /// set once the body invokes a callable we can't resolve (see `FnInfo::unresolved`).
    pub(crate) unresolved: bool,
    /// The ERROR type leaf of the enclosing fn's `Result<_, E>` return, if any — the `?` operator's
    /// `From::from` TARGET. A `may_fail()?` where `may_fail` returns `Result<_, E1>` and this fn returns
    /// `Result<_, E2>` desugars to `E2::from(e1)` via a local `impl From<E1> for E2`; we edge to
    /// `E2::from` when `E2` locally `impl From` (see `charge_from`). `None` for a non-fallible fn or an
    /// unresolvable/`Box<dyn Error>`/external error type → no `?` edge (the no-flood default).
    pub(crate) err_ret_leaf: Option<String>,
    /// Crate-wide CONST/STATIC string index (`API_BASE -> "https://api.openai.com/v1"`), literal-valued
    /// only (Pass A). Resolves a host built from a const — `post(API_BASE)` / `post(format!("{}/x",
    /// API_BASE))` — so the SPEC §1 static-host refinement (Llm / Db jdbc / Net-allowlist) fires just as
    /// it does on an inline literal (parity with candor-java's inlined `static final String`).
    pub(crate) const_strings: &'a std::collections::HashMap<String, String>,
    /// LOCAL string bindings we can resolve to a literal host — `let url = format!("{}/x", API_BASE)` /
    /// `let url = "https://…"` / `let url = API_BASE` — so a later `post(url)` recovers the host (ONE level
    /// of local `let` following; a rebind to a non-resolvable value clears the entry). Grown in source
    /// order, like `vars`. Literal/const-anchored only — a runtime-built binding contributes nothing.
    pub(crate) str_locals: std::collections::HashMap<String, String>,
}

impl<'a> CallCollector<'a> {
    /// Best-effort type of a method-call receiver, so `recv.method()` can be classified as
    /// `Type::method`. Resolves a bare variable/param/`self` (via `vars`), a `base.field` access (via
    /// the struct `FieldIndex`), and peels `&`/`(..)`/`?`/`.await`. For a method CHAIN
    /// (`client.get(url).send()`) it returns the BASE receiver's type — the chain stays within one
    /// crate's builder family, and the classifier verb-gates per crate, so attributing the terminal
    /// verb to the base type is correct in practice (`reqwest::Client` + `::send` -> Net).
    fn resolve_recv_type(&self, expr: &syn::Expr) -> Option<String> {
        match expr {
            syn::Expr::Reference(r) => self.resolve_recv_type(&r.expr),
            syn::Expr::Paren(p) => self.resolve_recv_type(&p.expr),
            syn::Expr::Group(g) => self.resolve_recv_type(&g.expr),
            syn::Expr::Try(t) => self.resolve_recv_type(&t.expr),
            syn::Expr::Await(a) => self.resolve_recv_type(&a.base),
            syn::Expr::MethodCall(m) => {
                // A method whose return is a DISPATCH trait object (`self.handler() -> &dyn Doer` /
                // `-> Box<dyn Doer>`) has NO concrete type — return None so the dispatch path
                // (`resolve_recv_traits`, which decodes the `<dyn>` sentinel by method leaf) resolves
                // `self.handler().go()`, instead of walking THROUGH the chain to the base receiver's type
                // (`Reg`) and shadowing the dispatch silent-pure. Safe: None only DECLINES concrete typing
                // (never fabricates), gated on an unambiguous `<dyn>` return recorded upstream.
                if self.returns.get(&m.method.to_string()).and_then(|t| ret_dyn_leaves(t)).is_some() {
                    return None;
                }
                // A method that returns a DIFFERENT (std) type — an iterator / slice / string / view
                // producer — breaks the builder-chain assumption that the chain stays one crate's type.
                // After `.iter()`/`.get_argv()`/`.as_slice()` the value is a std iterator/slice, so
                // attributing the OUTER leaf to the BASE crate's type fabricates: `mmap.iter().map()` →
                // `Mmap::map` → Fs, `cmd.get_argv().len()` → `CommandBuilder::len` → Exec (adversarial
                // review). These names are UNIVERSALLY non-`Self` (no builder uses them as a fluent step,
                // unlike `get`/`post`/`arg`/`bind`), so a hard type-change here → the chain's type is
                // unknown → honest miss (the safe direction), never the base's coarse/whole-crate rule.
                if matches!(
                    m.method.to_string().as_str(),
                    "iter" | "into_iter" | "iter_mut" | "drain" | "as_slice" | "as_mut_slice"
                        | "as_bytes" | "as_str" | "to_vec" | "keys" | "values" | "values_mut"
                        | "chars" | "bytes" | "get_argv" | "into_inner" | "lines"
                ) {
                    return None;
                }
                // Otherwise walk through the chain to the base receiver's type. We deliberately do NOT
                // consult the return-type index by method NAME here: a method name doesn't identify the
                // method, so a single crate-wide `fn conn() -> redis::Connection` would otherwise hijack
                // every `x.conn().get()` on an unrelated `x`, fabricating a Db effect. The return index
                // is used only for free-function factory calls (the `Expr::Call` arm via `ctor_type`).
                self.resolve_recv_type(&m.receiver)
            }
            syn::Expr::Path(p) => {
                let name = p.path.get_ident()?.to_string();
                // A local binding/param/`self` wins. Failing that, a bare UPPER-INITIAL path used AS A
                // VALUE is a UNIT-STRUCT (or unit enum variant) LITERAL receiver — `T0.run()` where
                // `struct T0;`: its type IS the path itself. Without this a direct `T0.run()` typed to
                // nothing (not in `vars`, not a dispatch var) and dropped SILENT-PURE, while the
                // `let x = T0; x.run()` form resolved (via `vars` seeded from `ctor_type`). We accept any
                // Upper-initial ident WITHOUT an underscore — this admits `T0`/`DB` that the
                // lowercase-requiring `type_from_value_path` (which must also gate `let`-typing) rejects,
                // while still excluding a SCREAMING_SNAKE const (`MAX_SIZE`). Fabrication-safe: the
                // downstream `local_types` gate in scan.rs confines the resulting `Type::method` link to
                // genuinely-LOCAL types, so a non-local Upper-initial value never mis-links.
                let upper_no_underscore = name.chars().next().is_some_and(|c| c.is_uppercase())
                    && !name.contains('_');
                self.vars.get(&name).cloned().or_else(|| {
                    upper_no_underscore.then(|| expand(&name, self.uses))
                })
            }
            syn::Expr::Field(f) => {
                let base = self.resolve_recv_type(&f.base)?;
                // Named field (`self.http`) or TUPLE field (`self.0`, incl. chained `self.0.0` via the
                // recursion — newtype wrappers; found by the PROVE-IT dogfood on ureq's
                // `ConfigBuilder(Scoped(..))`). Both index by the member's string form.
                let key = match &f.member {
                    syn::Member::Named(field) => field.to_string(),
                    syn::Member::Unnamed(idx) => idx.index.to_string(),
                };
                let base_leaf = base.rsplit("::").next().unwrap_or(&base);
                self.fields.get(base_leaf)?.get(&key).cloned()
            }
            syn::Expr::Call(_) => ctor_type(expr, self.uses, self.returns),
            // `S {..}.method()` / `for _ in (S {..})` — an inline struct literal names its type directly
            // (the same type a `let x = S{..}` binding already resolves via `ctor_type`). Without this a
            // value CONSTRUCTED INLINE and immediately consumed typed to nothing, so the iterator-forcing
            // edge (`charge_iter_next`) and method resolution dropped it silent-pure (a local effectful
            // `Iterator` built inline: `for _ in (RowIter::new())` read pure). `type_from_value_path`
            // (inside `ctor_type`) gates to a real type name, and scan.rs's `local_types` gate confines any
            // resulting `Type::method` link to LOCAL types, so this never fabricates onto a non-local value.
            // (`Expr::Paren`/`Expr::Group` transparent wrappers are unwrapped by the arms above, so a
            // parenthesised `for _ in (S {..})` reaches this Struct arm through them.)
            syn::Expr::Struct(_) => ctor_type(expr, self.uses, self.returns),
            // `xs[i].method()` / `self.senders[0].method()` — the receiver is the indexed BASE's
            // element type. Composes through the recursion: a nested `grid[i][j]` resolves the inner
            // index to its element collection, then this index to ITS element.
            syn::Expr::Index(idx) => self.resolve_elem_type(&idx.expr),
            _ => None,
        }
    }

    /// The ELEMENT type of an expression that evaluates to a COLLECTION — a collection var/param (via
    /// `elem_of`), a collection FIELD (`self.senders`, via `field_elem`), an iterator adapter that
    /// preserves the element (`.iter()`/`.into_iter()`/`.iter_mut()`/`.clone()`), or another subscript
    /// (`grid[i]` -> a row, whose own element types `grid[i][j]`). Peels the effect-transparent
    /// wrappers. `None` when the element type can't be determined — honest under-report, never a guess.
    fn resolve_elem_type(&self, expr: &syn::Expr) -> Option<String> {
        match expr {
            syn::Expr::Reference(r) => self.resolve_elem_type(&r.expr),
            syn::Expr::Paren(p) => self.resolve_elem_type(&p.expr),
            syn::Expr::Group(g) => self.resolve_elem_type(&g.expr),
            syn::Expr::Try(t) => self.resolve_elem_type(&t.expr),
            syn::Expr::Await(a) => self.resolve_elem_type(&a.base),
            syn::Expr::Path(p) => {
                let name = p.path.get_ident()?.to_string();
                self.elem_of.get(&name).cloned()
            }
            syn::Expr::Field(f) => {
                let base = self.resolve_recv_type(&f.base)?;
                let key = match &f.member {
                    syn::Member::Named(field) => field.to_string(),
                    syn::Member::Unnamed(idx) => idx.index.to_string(),
                };
                let base_leaf = base.rsplit("::").next().unwrap_or(&base);
                self.field_elem.get(base_leaf)?.get(&key).cloned()
            }
            // An element-PRESERVING iterator adapter (`xs.iter()`, `xs.into_iter()`, `&xs.iter_mut()`,
            // `xs.clone()`) yields the same element type as its receiver — so `xs.iter().for_each(..)`
            // and `for c in xs.iter()` both type the element. A transforming adapter (`.map`) changes
            // the element, so it is deliberately NOT listed (its element is indeterminate → None).
            syn::Expr::MethodCall(m) => {
                let adapter = matches!(
                    m.method.to_string().as_str(),
                    "iter" | "into_iter" | "iter_mut" | "clone" | "drain" | "as_slice" | "as_mut_slice"
                        | "to_vec" | "values" | "values_mut"
                );
                if adapter {
                    self.resolve_elem_type(&m.receiver)
                } else {
                    None
                }
            }
            // `grid[i]` is itself a collection (a row): its element type is the indexed base's element.
            syn::Expr::Index(idx) => self.resolve_elem_type(&idx.expr),
            _ => None,
        }
    }

    /// The DISPATCH-trait leaves of an expression evaluating to a COLLECTION OF TRAIT OBJECTS — the
    /// `resolve_elem_type` counterpart backed by `elem_trait_of`. Lets `for it in items { it.go() }` over
    /// an `items: Vec<Box<dyn Doer>>` type the loop var into `trait_vars` (bounded-CHA dispatch) instead
    /// of dropping silent-pure. Peels references + the element-preserving iterator adapters, exactly like
    /// `resolve_elem_type`. Empty when the collection's element isn't a trait object (no guess).
    fn resolve_elem_trait_leaves(&self, expr: &syn::Expr) -> Vec<String> {
        match expr {
            syn::Expr::Reference(r) => self.resolve_elem_trait_leaves(&r.expr),
            syn::Expr::Paren(p) => self.resolve_elem_trait_leaves(&p.expr),
            syn::Expr::Group(g) => self.resolve_elem_trait_leaves(&g.expr),
            syn::Expr::Path(p) => p
                .path
                .get_ident()
                .and_then(|id| self.elem_trait_of.get(&id.to_string()))
                .cloned()
                .unwrap_or_default(),
            // `self.handlers` / `reg.handlers` — a COLLECTION-OF-TRAIT-OBJECTS FIELD: resolve the receiver's
            // type and look up its element dispatch leaves (mirrors `resolve_elem_type`'s field arm).
            syn::Expr::Field(f) => {
                let Some(base) = self.resolve_recv_type(&f.base) else { return Vec::new() };
                let key = match &f.member {
                    syn::Member::Named(field) => field.to_string(),
                    syn::Member::Unnamed(idx) => idx.index.to_string(),
                };
                let base_leaf = base.rsplit("::").next().unwrap_or(&base);
                self.field_elem_trait
                    .get(base_leaf)
                    .and_then(|m| m.get(&key))
                    .cloned()
                    .unwrap_or_default()
            }
            syn::Expr::MethodCall(m) => {
                let adapter = matches!(
                    m.method.to_string().as_str(),
                    // element-preserving collection adapters + map value views…
                    "iter" | "into_iter" | "iter_mut" | "drain" | "as_slice" | "as_mut_slice" | "values" | "values_mut"
                    // …and the interior-mutability / smart-pointer GUARD chain that peels back to the
                    // wrapped collection: `reg.lock().unwrap().iter()` / `cell.borrow().iter()` /
                    // `rw.read().unwrap().iter()` over an `Arc<Mutex<Vec<Box<dyn>>>>` etc.
                    | "lock" | "unwrap" | "expect" | "borrow" | "borrow_mut" | "read" | "write" | "as_ref" | "as_mut"
                );
                if adapter {
                    self.resolve_elem_trait_leaves(&m.receiver)
                } else {
                    // A METHOD factory returning a COLLECTION of trait objects (`for d in r.all()`,
                    // `all() -> Vec<Box<dyn>>` → `<elemdyn>`) OR an Option/Result of one (`if let Some(d) =
                    // self.opt()`, `opt() -> Option<Box<dyn>>` → recorded scalar `<dyn>` via
                    // unwrap_result_option). Decode either — this arm is only reached in a collection/option
                    // context, which a plain scalar-`dyn` return can't be, so the `<dyn>` fallback is safe.
                    self.returns.get(&m.method.to_string())
                        .and_then(|t| ret_elem_dyn_leaves(t).or_else(|| ret_dyn_leaves(t)))
                        .unwrap_or_default()
                }
            }
            // A FREE/STATIC factory returning a collection (or Option/Result) of trait objects.
            syn::Expr::Call(c) => {
                let syn::Expr::Path(p) = &*c.func else { return Vec::new() };
                let leaf = p.path.segments.last().map(|s| s.ident.to_string()).unwrap_or_default();
                self.returns.get(&leaf)
                    .and_then(|t| ret_elem_dyn_leaves(t).or_else(|| ret_dyn_leaves(t)))
                    .unwrap_or_default()
            }
            _ => Vec::new(),
        }
    }

    /// IMPLICIT ITERATOR FORCING. A `for x in <expr>` or a consuming combinator (`.collect()`,
    /// `.count()`, `for_each`, …) on `<expr>` drives the iterator's `next()` to completion — so if
    /// `<expr>`'s receiver is a CONCRETE LOCAL TYPE with a local `impl Iterator`, that type's
    /// effectful `next` is reachable but NEVER written at the forcing site (only an explicit
    /// `x.next()` / `while let Some(_) = x.next()` was caught). This emits a synthetic edge to
    /// `Type::next` so the init's effect propagates to the forcing fn (a §4 under-report fix).
    ///
    /// SCOPING — the RowIter no-fabrication guard. We charge ONLY when `resolve_recv_type` yields a
    /// CONCRETE local type (a `vars`/field/`ctor_type`-return binding) whose leaf locally
    /// `impl Iterator`. A bare `impl Iterator` / generic `T: Iterator` / `&mut dyn Iterator` param
    /// lands in `trait_vars` (removed from `vars` in `fninfo`), so `resolve_recv_type` returns None
    /// → no charge: a generic iterator consumer stays Unknown/pure, never charged with some local
    /// impl's effect (the review-killed `fn f(it: impl Iterator)` + `impl Iterator for RowIter`
    /// fabrication). A `-> impl Iterator` opaque return isn't recorded as a concrete return type, so
    /// `build().count()` over an opaque builder also yields None (acceptable miss, not a guess). The
    /// edge resolves to the local `Type::next` def via `resolve_target`'s unambiguous-tail2 route.
    fn iter_next_target(&self, expr: &syn::Expr) -> Option<String> {
        let ty = self.resolve_recv_type(expr)?;
        let ty_leaf = ty.rsplit("::").next().unwrap_or(&ty);
        // The receiver's concrete type must locally `impl Iterator`. `trait_impls` values are type
        // LEAVES (see `impl_type_name`), and the receiver type may be module-qualified, so compare
        // by leaf. (A non-local / external iterator type is absent from `trait_impls` → None.)
        let impls = self.trait_impls.get("Iterator")?;
        if impls.iter().any(|t| t == ty_leaf) {
            Some(ty_leaf.to_string())
        } else {
            None
        }
    }

    /// Push the synthetic `Type::next` forcing edge for an implicitly-forced concrete-local iterator
    /// (see `iter_next_target`). De-dup is unnecessary: the call list tolerates duplicate edges
    /// (they collapse to one resolved target), and forcing sites are not hot-looped like lazy reads.
    fn charge_iter_next(&mut self, expr: &syn::Expr) {
        if let Some(ty_leaf) = self.iter_next_target(expr) {
            let path = format!("{ty_leaf}::next");
            self.calls.push(Call {
                path,
                leaf: "next".to_string(),
                str_arg: None,
                typed: false,
                method: false,
                is_macro: false,
            });
        }
    }

    /// Push a synthetic `Type::method` edge — the shared primitive for every implicit trait-method
    /// coercion (`Display::fmt`, `From::from`, `Deref::deref`, the operator family). The path mirrors what
    /// the impl-method walker records as a FnInfo qual (`impl Display for T { fn fmt }` → qual `T::fmt`,
    /// via `impl_type_name`), so it resolves through `resolve_target`'s unambiguous-tail2 route to the
    /// LOCAL impl body, carrying its (possibly effectful) effects to this fn. `method=false`/`typed=false`
    /// like the iterator/lazy edges. The CALLER owns the resolve-or-skip gate (the type must be a concrete
    /// local `impl <trait>`), so this never fabricates.
    fn push_coercion_edge(&mut self, ty_leaf: &str, method: &str) {
        self.calls.push(Call {
            path: format!("{ty_leaf}::{method}"),
            leaf: method.to_string(),
            str_arg: None,
            typed: false,
            method: false,
            is_macro: false,
        });
    }

    /// IMPLICIT TRAIT-METHOD COERCION on an OPERAND. Synthesize an edge to `<Type>::<method>` ONLY when
    /// `operand` resolves to a CONCRETE LOCAL type (via `resolve_recv_type`) that locally `impl <trait>`
    /// (present in `trait_impls[trait_leaf]`, keyed by type leaf per `impl_type_name`). This covers the
    /// constructs whose hidden call is dispatched on the operand's OWN type: `{}`/`{:?}` format args
    /// (`Display::fmt`/`Debug::fmt`), `*w`/auto-deref (`Deref::deref`), and the operator overloads
    /// (`a + b`→`Add::add`, `a == b`→`PartialEq::eq`, …). (`From::from`, dispatched on the TARGET type,
    /// has its own `charge_from`.)
    ///
    /// GOVERNING DISCIPLINE (critical — these constructs are PERVASIVE): a PURE local impl contributes
    /// nothing (its `Type::method` FnInfo carries no effect — the edge resolves to a pure body); an
    /// UNRESOLVABLE operand (a std/external value like `String`/`i32`, a generic/`impl Trait` param in
    /// `trait_vars`, an opaque return) yields None → NO edge, NOT a disclosed Unknown — a blanket Unknown
    /// here would FLOOD every `format!`/`+`/`concat` in real code. We never fabricate; only a real LOCAL
    /// effectful impl lights up. An impl whose method body isn't a UNIQUE local def is an honest miss
    /// (`resolve_target`'s uniqueness filter), e.g. a type impl'ing both Display and Debug (two `T::fmt`).
    fn charge_coercion(&mut self, operand: &syn::Expr, trait_leaf: &str, method: &str) {
        let Some(ty) = self.resolve_recv_type(operand) else { return };
        let ty_leaf = ty.rsplit("::").next().unwrap_or(&ty);
        if let Some(impls) = self.trait_impls.get(trait_leaf) {
            if impls.iter().any(|t| t == ty_leaf) {
                self.push_coercion_edge(ty_leaf, method);
            }
        }
    }

    /// IMPLICIT `From::from` (the `?` operator's error conversion and `.into()`), dispatched on the TARGET
    /// type. `?` on `expr: Result<_, E1>` inside a fn returning `Result<_, E2>` desugars (when `E1 != E2`)
    /// to `E2::from(e1)` via a local `impl From<E1> for E2`; `.into()` to `Target::from(src)`. The body
    /// that may be effectful lives on the TARGET `E2`/`Target`, NOT the operand's source type — so we
    /// resolve from the supplied `target_leaf` (the enclosing fn's error type for `?`; a context type for
    /// `.into()`) and edge to `Target::from` ONLY when `Target` locally `impl From`. A None/unknown target
    /// → NO edge (no flood — the overwhelming case is std `From` like `String: From<&str>`, never local).
    /// CONST-STRING PROPAGATION (SPEC §1 static-host): recover a host literal from a call's args when the
    /// URL is built from a `const` rather than written inline. Used ONLY as a FALLBACK when `first_str_lit`
    /// found no inline literal — the inline path is unchanged. Resolves exactly three sound shapes for the
    /// FIRST relevant value expr and NOTHING else (never a guess):
    ///   • a bare const/local path         `post(API_BASE)`            → the const's / local's literal;
    ///   • a `let`-bound resolvable string  `post(url)`  (url set earlier from a resolvable shape);
    ///   • a leading-`{}` format!           `post(format!("{}/chat", API_BASE))` → the prefix arg's literal.
    /// Returns `None` — leaving the call bare Net with the host masked, exactly as today — for a runtime
    /// value, an unknown identifier, a non-const format arg, or a format string with a literal prefix
    /// before the first hole. NO FABRICATION: only a genuinely literal-valued const/local ever resolves.
    fn resolve_host_arg(
        &self,
        args: &syn::punctuated::Punctuated<syn::Expr, syn::token::Comma>,
    ) -> Option<String> {
        let first = args.iter().next()?;
        self.resolve_str_expr(first)
    }

    /// Resolve a single expr to a literal string via const/local propagation (the shared resolver for a
    /// call arg AND a `let` initializer). Bare path → const or local; leading-`{}` `format!` → its prefix
    /// arg resolved the same way. One level deep — a nested/runtime shape yields `None` (no fabrication).
    fn resolve_str_expr(&self, expr: &syn::Expr) -> Option<String> {
        match expr {
            // A bare path: a local resolvable string binding first (`let url = …`), then a crate const.
            syn::Expr::Path(_) => {
                let leaf = path_leaf_ident(expr)?;
                self.str_locals
                    .get(&leaf)
                    .or_else(|| self.const_strings.get(&leaf))
                    .cloned()
            }
            syn::Expr::Macro(m) => {
                // (a) A `format!("{}…", CONST, …)` whose FIRST hole is a bare `{}` — the const is the URL
                // PREFIX (host). Resolve the const/local literal (one level; no nested-format recursion).
                if let Some(prefix_arg) = format_const_prefix_arg(&m.mac) {
                    if let Some(leaf) = path_leaf_ident(&prefix_arg) {
                        if let Some(v) = self.str_locals.get(&leaf).or_else(|| self.const_strings.get(&leaf)) {
                            return Some(v.clone());
                        }
                    }
                    return None;
                }
                // (b) LITERAL-HEAD: a `format!("https://api.openai.com/v1/{}", path)` whose format-string
                // literal already spells out a COMPLETE authority before its first hole — the host is the
                // literal, only the PATH is interpolated. `format_literal_head_host` returns it ONLY when
                // the authority is terminated by a `/` in the literal (else a hole could be inside the host
                // → bare Net). The returned bare host runs through the caller's host refinement unchanged.
                format_literal_head_host(&m.mac)
            }
            _ => None,
        }
    }

    fn charge_from(&mut self, target_leaf: &str) {
        if let Some(impls) = self.trait_impls.get("From") {
            if impls.iter().any(|t| t == target_leaf) {
                self.push_coercion_edge(target_leaf, "from");
            }
        }
    }

    /// Charge `Display::fmt`/`Debug::fmt` coercion edges for a formatting macro's arguments (#2). `exprs`
    /// is the macro's comma-separated token parse: a leading format-string LITERAL (when present) followed
    /// by the value args. We parse the literal's `{…}` holes to learn which positional arg each uses and
    /// whether it requests Debug (`{:?}`/`{:#?}`) or Display (everything else, incl. bare `{}`); a NAMED
    /// or inline-captured hole (`{x}`) and `write!`/`writeln!`'s leading writer arg are handled by the
    /// positional accounting below. For each value arg whose type is a concrete local impl of the
    /// requested formatter trait, edge to `Type::fmt` (resolve-or-skip — a std/external arg lights nothing).
    fn charge_format_args(
        &mut self,
        leaf: &str,
        exprs: &syn::punctuated::Punctuated<syn::Expr, syn::Token![,]>,
    ) {
        let exprs: Vec<&syn::Expr> = exprs.iter().collect();
        // Locate the format-string literal and the index where positional value args begin. `write!`/
        // `writeln!`/`fwrite` take a WRITER as the first arg, then the format string; `format!`/`print!`/
        // … lead with the string. We find the first string-literal expr and treat everything AFTER it as
        // the positional value args (named args `name = expr` are `Expr::Assign` — skipped as positional).
        let Some(fmt_pos) = exprs.iter().position(|e| {
            matches!(e, syn::Expr::Lit(syn::ExprLit { lit: syn::Lit::Str(_), .. }))
        }) else {
            return; // no literal format string (a runtime `&str` fmt) — can't map holes, skip (no flood)
        };
        let syn::Expr::Lit(syn::ExprLit { lit: syn::Lit::Str(s), .. }) = exprs[fmt_pos] else {
            return;
        };
        let fmt = s.value();
        // The positional value args, in order (a `name = expr` named arg has no positional hole; skip it
        // so positional indices stay aligned). These are the args AFTER the format string.
        let pos_args: Vec<&syn::Expr> = exprs[fmt_pos + 1..]
            .iter()
            .copied()
            .filter(|e| !matches!(e, syn::Expr::Assign(_)))
            .collect();
        // Each parsed hole → (positional index, wants_debug). A hole with an inline capture/name
        // (`{x}`, `{x:?}`) is NOT positional — it captures a same-named binding, not a value arg, so it
        // consumes no positional slot (we can't resolve the captured ident's type here → skip it).
        let mut next_positional = 0usize;
        for hole in parse_format_holes(&fmt) {
            let idx = match hole.arg {
                FmtArg::Implicit => {
                    let i = next_positional;
                    next_positional += 1;
                    i
                }
                FmtArg::Index(i) => i,
                // a named/inline-captured hole consumes no positional value arg
                FmtArg::Named => continue,
            };
            let Some(arg) = pos_args.get(idx) else { continue };
            let trait_method = if hole.debug { ("Debug", "fmt") } else { ("Display", "fmt") };
            self.charge_coercion(arg, trait_method.0, trait_method.1);
        }
        // WRITER side of `write!`/`writeln!`: the arg BEFORE the format string is the writer, whose
        // effectful `fmt::Write::write_str` / `io::Write::write` (driven by the default `write_fmt`) was
        // dropped — the writer side, distinct from the arg-`Display` side above (a cross-engine blind spot:
        // the deep engine had it too, HOLE 2c). Charge it (resolve-or-skip — a std writer like `String`/
        // `Vec`/`Stdout` resolves to no local impl → nothing). Gated to the write family so a leading
        // `assert!`/`assert_eq!` operand is never mistaken for a writer. Both method names are tried because
        // the `Write` trait leaf is shared by fmt (`write_str`) and io (`write`); only the one the local
        // type actually defines resolves to a body, so a mismatch is a harmless no-op edge.
        if matches!(leaf, "write" | "writeln") && fmt_pos >= 1 {
            let writer = exprs[fmt_pos - 1];
            self.charge_coercion(writer, "Write", "write_str");
            self.charge_coercion(writer, "Write", "write");
        }
    }

    /// The trait bounds of a DISPATCH-typed receiver — a `&dyn T`/`impl T`/generic param (via
    /// `trait_vars`) or a trait-typed field (`self.store` where `store: Box<dyn Store>`, via
    /// `trait_fields`). Empty when the receiver has a concrete type (`resolve_recv_type` owns it)
    /// or can't be resolved at all.
    /// Does local trait `tr` declare `leaf`, or INHERIT it from a (transitive, local) supertrait?
    /// (`trait Sub: Super` — a `Super` method is callable on a `Sub` receiver.) Bounded depth guards a
    /// cyclic/deep hierarchy; an external supertrait resolves to nothing (documented miss).
    fn trait_declares_method(&self, tr: &str, leaf: &str, depth: usize) -> bool {
        if depth > 16 {
            return false;
        }
        let Some(lt) = self.local_traits.get(tr) else { return false };
        lt.methods.contains(leaf)
            || lt.supertraits.iter().any(|s| self.trait_declares_method(s, leaf, depth + 1))
    }
    fn resolve_recv_traits(&self, expr: &syn::Expr) -> Vec<String> {
        // Hot-path guard: with no dispatch-typed vars or fields in scope AND no dispatch-object-returning
        // factory recorded (the overwhelmingly common case), every lookup below is a guaranteed miss —
        // skip the recursive walk. The Call arm depends on `returns`, not on the vars/fields, so it is
        // kept live whenever any factory return is a `<dyn>` sentinel.
        if self.trait_vars.is_empty() && self.trait_fields.is_empty() && !self.has_dyn_return {
            return Vec::new();
        }
        match expr {
            syn::Expr::Reference(r) => self.resolve_recv_traits(&r.expr),
            syn::Expr::Paren(p) => self.resolve_recv_traits(&p.expr),
            syn::Expr::Group(g) => self.resolve_recv_traits(&g.expr),
            syn::Expr::Try(t) => self.resolve_recv_traits(&t.expr),
            syn::Expr::Await(a) => self.resolve_recv_traits(&a.base),
            syn::Expr::Path(p) => p
                .path
                .get_ident()
                .and_then(|id| self.trait_vars.get(&id.to_string()).cloned())
                .unwrap_or_default(),
            // A FACTORY call returning a DISPATCH trait object (`get().run()` where `get() -> Box<dyn
            // Task>`): the recorded return is the `<dyn>` sentinel, whose decoded bound leaves feed the
            // SAME bounded-CHA the direct-`Box<dyn Task>` control uses. Only a LOCAL fn's UNAMBIGUOUS
            // return is in `returns` (ambiguous leaves are dropped upstream), so this never guesses.
            syn::Expr::Call(c) => {
                let syn::Expr::Path(p) = &*c.func else { return Vec::new() };
                let full = path_to_string(&p.path);
                let leaf = full.rsplit("::").next().unwrap_or(&full);
                self.returns.get(leaf).and_then(|t| ret_dyn_leaves(t)).unwrap_or_default()
            }
            // A METHOD factory returning a dispatch trait object (`self.handler().go()` where
            // `handler(&self) -> &dyn Doer`): decode the recorded `<dyn>` sentinel by the method leaf,
            // exactly like the free/static-fn Call arm above (an ambiguous leaf was dropped upstream).
            syn::Expr::MethodCall(m) => self
                .returns
                .get(&m.method.to_string())
                .and_then(|t| ret_dyn_leaves(t))
                .unwrap_or_default(),
            syn::Expr::Field(f) => {
                let Some(base) = self.resolve_recv_type(&f.base) else { return Vec::new() };
                let key = match &f.member {
                    syn::Member::Named(field) => field.to_string(),
                    syn::Member::Unnamed(idx) => idx.index.to_string(),
                };
                let base_leaf = base.rsplit("::").next().unwrap_or(&base);
                self.trait_fields.get(base_leaf).and_then(|m| m.get(&key).cloned()).unwrap_or_default()
            }
            _ => Vec::new(),
        }
    }
}

impl<'a> CallCollector<'a> {
    /// Whether an expression evaluates to a fn-typed (callback) value — a fn-typed binding, through
    /// `&`/paren/group wrappers, or an `if` whose then-branch tail yields one. Lets `let g = cb`
    /// propagate fn-typed-ness so a later `g()` reads the honest `Unknown` instead of a phantom free-fn
    /// call. Over-approximating toward fn-typed only ever marks `g()` Unknown (the safe direction) — it
    /// never fabricates a specific effect.
    fn expr_is_fn_typed(&self, expr: &syn::Expr) -> bool {
        match expr {
            syn::Expr::Path(p) => p.path.get_ident().is_some_and(|i| self.fn_typed_vars.contains(&i.to_string())),
            syn::Expr::Paren(p) => self.expr_is_fn_typed(&p.expr),
            syn::Expr::Group(g) => self.expr_is_fn_typed(&g.expr),
            syn::Expr::Reference(r) => self.expr_is_fn_typed(&r.expr),
            syn::Expr::Try(t) => self.expr_is_fn_typed(&t.expr),
            syn::Expr::Await(a) => self.expr_is_fn_typed(&a.base),
            syn::Expr::If(e) => block_tail_expr(&e.then_branch).is_some_and(|t| self.expr_is_fn_typed(t)),
            // `let g = make_callback();` — a call to a LOCAL factory the pre-pass recorded as returning a
            // callable (the fn-typed sentinel). Without this, `g()` resolves as a phantom free-fn `g` and
            // is silently dropped (or fabricates a same-named local fn). Over-approximating to fn-typed
            // only marks `g()` Unknown — the safe direction for a missed-effect-is-a-hole tool.
            syn::Expr::Call(c) => match &*c.func {
                syn::Expr::Path(p) => p
                    .path
                    .get_ident()
                    .and_then(|id| self.returns.get(&id.to_string()))
                    .is_some_and(|t| t == RET_FN_TYPED),
                _ => false,
            },
            _ => false,
        }
    }

    /// Bind `name -> ty` in `vars` for the duration of `body`, then RESTORE the prior binding (or
    /// remove it). ⚠️ `vars` is function-wide, NOT block-scoped — an unscoped binding leaks into a
    /// later same-named, uninferable var and FABRICATES its effect (the candor-swift `vars`-leak bug).
    /// Every binder that types a pattern (loop var, closure param, match payload, tuple element) MUST
    /// route through here so the binding is torn down after its block. A `None` type still scopes: it
    /// REMOVES any stale binding for the body and restores it after, so a prior effectful binding can't
    /// leak in either.
    fn scoped_var<R>(&mut self, name: &str, ty: Option<String>, body: impl FnOnce(&mut Self) -> R) -> R {
        let prior = self.vars.remove(name);
        if let Some(t) = ty {
            self.vars.insert(name.to_string(), t);
        }
        let r = body(self);
        match prior {
            Some(p) => {
                self.vars.insert(name.to_string(), p);
            }
            None => {
                self.vars.remove(name);
            }
        }
        r
    }

    /// Bind each single-ident element of a tuple PATTERN to the matching element of a tuple TYPE
    /// (`let (s, _): (Sender, usize)` → `s: Sender`). A `_`/wildcard element is skipped. Each binding
    /// CLEARS any prior `vars`/`elem_of` for the name first, so a stale effectful binding can't survive
    /// a rebind — these are top-level `let` bindings (function-wide), so they're not torn down.
    fn bind_tuple<'p>(
        &mut self,
        pats: &syn::punctuated::Punctuated<syn::Pat, syn::Token![,]>,
        tys: impl Iterator<Item = &'p syn::Type>,
    ) {
        for (pat_el, ty_el) in pats.iter().zip(tys) {
            if let Some(name) = single_pat_ident(pat_el) {
                self.vars.remove(&name);
                self.elem_of.remove(&name);
                if let Some(ty) = type_path(ty_el, self.uses) {
                    self.vars.insert(name.clone(), ty);
                }
                if let Some(e) = elem_type(ty_el, self.uses) {
                    self.elem_of.insert(name, e);
                }
            }
        }
    }
}

impl<'a, 'ast> Visit<'ast> for CallCollector<'a> {
    fn visit_stmt(&mut self, node: &'ast syn::Stmt) {
        // A `#[cfg(feature="X")]`-gated statement/block that is compiled OUT under the active feature set
        // contributes no effects to this fn — don't walk it (else its calls fabricate effects the default
        // build never performs; winnow's debug-trace block reaching `std::env::var`).
        if stmt_cfg_inactive(node) {
            return;
        }
        syn::visit::visit_stmt(self, node);
    }
    fn visit_expr_call(&mut self, node: &'ast syn::ExprCall) {
        // Peel `(..)`/`{..}` wrappers around the callee so `(f)()` is treated like `f()`.
        let mut func = &*node.func;
        loop {
            match func {
                syn::Expr::Paren(p) => func = &p.expr,
                syn::Expr::Group(g) => func = &g.expr,
                _ => break,
            }
        }
        match func {
            syn::Expr::Path(p) => {
                let ident = p.path.get_ident().map(|id| id.to_string());
                // Invoking a fn-typed binding (`cb: fn()`/`impl Fn`/`dyn Fn`/generic `F: Fn`) calls a body
                // the syntactic scan can't see — honest `Unknown`, never silently pure (SPEC §4). The
                // computed/field form (`(self.f)()`, `arr[i]()`) already hits the `_ => unresolved` arm
                // below; the bare-Path param/local form was silently dropped as a phantom call to a free
                // fn `cb`. (Found by the cross-engine generative differential: java/ts/swift propagated or
                // marked Unknown, candor-scan read pure.)
                if ident.as_ref().is_some_and(|n| self.fn_typed_vars.contains(n)) {
                    self.unresolved = true;
                } else {
                    // A local bound to a closure — `let f = |..| ..` — has its body walked LEXICALLY by
                    // this same visitor, so `f()` adds nothing and is NOT a blind spot. (Skip recording it
                    // as a phantom call to a free fn `f`, too.) Any other path is a normal call. The
                    // `!is_empty()` short-circuit avoids allocating the ident String on the common path.
                    let is_closure_call = !self.closure_vars.is_empty()
                        && ident.as_ref().is_some_and(|n| self.closure_vars.contains(n));
                    if !is_closure_call {
                        // resolve a fn-alias local (`let g = eff; g()`) to its aliased path (sweep [6]);
                        // otherwise the bare path as written.
                        let mut path = ident
                            .as_ref()
                            .and_then(|n| self.fn_alias.get(n).cloned())
                            .unwrap_or_else(|| expand(&path_to_string(&p.path), self.uses));
                        // A QSELF call (`<Type>::assoc()` / `<Type as Trait>::m()`) is an ASSOCIATED-fn call
                        // on the qself receiver TYPE, not a free fn — but `path_to_string(&p.path)` DROPS the
                        // qself type (`p.qself.ty`), so an INHERENT-form `<Vec<u8>>::new()` collapses to the
                        // BARE leaf `new`, which `resolve_target`'s by_leaf route then mis-linked to ANY
                        // unique local fn/method named `new`/`dump`/… — FABRICATING that def's effect onto a
                        // provably-pure path (`<Vec<u8>>::new()` charged Exec via a sibling `Daemon::new`).
                        // FIX: RESTORE the receiver type (`Vec::new`) so resolution stays PRECISE in BOTH
                        // directions — `<Daemon>::new()` still resolves to a local effectful `Daemon::new`
                        // (no under-report), while `<Vec<u8>>::new()` finds no local `Vec` (no fabrication).
                        // The trait-qualified form `<T as Trait>::m` already keeps `Trait::m` (has `::`), so
                        // only the bare-collapsed inherent form is touched. If the receiver isn't a nominal
                        // path (tuple/slice/…) the type can't be recovered → suppress the bare-leaf route
                        // (`method:true` → resolve_target None): an honest under-report, never a fabrication.
                        let mut method = false;
                        if p.qself.is_some() && !path.contains("::") {
                            match p.qself.as_ref().and_then(|q| type_path(&q.ty, self.uses)) {
                                Some(ty) => path = format!("{ty}::{path}"),
                                None => method = true,
                            }
                        }
                        let leaf = path.rsplit("::").next().unwrap_or(&path).to_string();
                        // Inline literal, else const-string propagation (`reqwest::get(API_BASE)` /
                        // `Client::post(format!("{}/x", API_BASE))`) — SPEC §1 static-host, same refinement.
                        let str_arg = first_str_lit(&node.args).or_else(|| self.resolve_host_arg(&node.args));
                        self.calls.push(Call { path, leaf, str_arg, typed: false, method, is_macro: false });
                    }
                }
            }
            // The callee is a COMPUTED value, not a path or a visible local closure: `(self.handler)()`,
            // `arr[i]()`, `make_cb()()`. The scan can't identify the target or see its body — it could
            // perform any effect — so the enclosing function can't be certified pure: honest `Unknown`.
            _ => self.unresolved = true,
        }
        syn::visit::visit_expr_call(self, node);
    }
    fn visit_expr_method_call(&mut self, node: &'ast syn::ExprMethodCall) {
        let leaf = node.method.to_string();
        // Inline literal first (unchanged); fall back to const-string propagation so `post(API_BASE)` /
        // `post(format!("{}/x", API_BASE))` / `post(url)` recover a statically-known host (SPEC §1). The
        // resolved literal flows through the SAME Net/Llm/Db host refinement in scan.rs as an inline one.
        let str_arg = first_str_lit(&node.args).or_else(|| self.resolve_host_arg(&node.args));
        // IMPLICIT ITERATOR FORCING via a consuming combinator: `it.count()`, `it.collect()`,
        // `it.for_each(..)`, `it.fold(..)`, … each drive `Iterator::next` to completion. When `it`
        // is a CONCRETE LOCAL type with a local `impl Iterator` (incl. a builder `build().count()`
        // whose receiver type is the recorded return type), charge its effectful `next` — else the
        // forcing site reads silent-pure (only an explicit `.next()` was caught). A generic/opaque
        // iterator receiver yields no concrete type (`iter_next_target` → None), so the RowIter
        // fabrication stays closed. (`.next()` itself isn't here — it resolves directly as a method.)
        if is_iter_consumer(&leaf) {
            self.charge_iter_next(&node.receiver);
        }
        // IMPLICIT `.to_string()` → `Display::fmt` (#2): `ToString` is blanket-impl'd for every `T: Display`
        // by routing through `Display::fmt`, so `v.to_string()` on a concrete local `impl Display` reaches
        // that (possibly effectful) formatter — silent-pure otherwise. Charge it like a `{}` format arg.
        // (A type with its OWN inherent `to_string` is rare; the blanket impl is the universal case, and a
        // local impl Display is the resolve-or-skip gate.) `format_args!`-family macros are handled in
        // `visit_macro`.
        if leaf == "to_string" && node.args.is_empty() {
            self.charge_coercion(&node.receiver, "Display", "fmt");
        }
        // IMPLICIT io::Write / io::Read PROVIDED-METHOD forcing (#w): `w.write_all(..)`/`w.write_fmt(..)`
        // and `r.read_to_end(..)`/`r.read_to_string(..)`/`r.read_exact(..)` are driven by the required
        // `write`/`read` INSIDE std, so a call on a CONCRETE LOCAL `impl Write`/`impl Read` whose
        // `write`/`read` is effectful read silent-pure (the provided→required callback is invisible —
        // even on a concrete receiver, distinct from the `write!` MACRO writer side already charged in
        // `visit_macro`). Charge the required method like the iterator-`next` / `to_string`-`fmt`
        // coercions — resolve-or-skip on the concrete local type: a std `File`/`Vec`/`Stdout` receiver is
        // absent from `trait_impls` (LOCAL impls only) → no edge; a generic/`dyn` receiver yields no
        // concrete type → the documented external-dispatch miss, unchanged. Both `Write` required-method
        // leaves are tried (io `write`, fmt `write_str`); only the one the local type defines resolves.
        if is_write_provided(&leaf) {
            self.charge_coercion(&node.receiver, "Write", "write");
            self.charge_coercion(&node.receiver, "Write", "write_str");
        }
        if is_read_provided(&leaf) {
            self.charge_coercion(&node.receiver, "Read", "read");
        }
        // Leaf-only call: feeds the intra-crate call graph and bare-leaf classification.
        self.calls.push(Call { path: leaf.clone(), leaf: leaf.clone(), str_arg: str_arg.clone(), typed: false, method: true, is_macro: false });
        // Typed call: if the receiver's type resolves, form `Type::method` so the existing per-crate
        // method rules (reqwest/sqlx/redis/…) — unreachable from a bare method name — can fire. This is
        // the method-dispatch frontier: light, local type inference, no compiler.
        //
        // EXTERNAL types only. The external-crate rules are verb-precise (`ends_with("::execute")`), so
        // they're safe to apply to an inferred method call. The std rules are coarse PREFIX matches
        // (`std::fs::`, `std::process::Command`) written for free-function/constructor calls — applied to
        // arbitrary method calls they mis-fire on pure ones (`File::as_raw_fd`, `Command::arg`). So skip
        // std/core/alloc receivers: their free-function effects are already caught path-qualified, and an
        // honest miss on a std method beats a wrong effect on a pure one.
        if let Some(ty) = self.resolve_recv_type(&node.receiver) {
            let cr = ty.split("::").next().unwrap_or("");
            // EXCEPTION to the std exclusion: `std::path::Path`/`PathBuf` receivers route through —
            // the classifier has a VERB-PRECISE stat-family rule for them (metadata/read_dir/exists/…
            // → Fs; the pure join/file_name surface returns None), so the coarse-prefix mis-fire risk
            // doesn't apply. Without this an entire directory walker reads as pure (gix-dir: zero Fs).
            let std_path_recv = ty == "std::path::Path" || ty == "std::path::PathBuf";
            // `.clone()` resolves to NO typed `Type::clone`: it is conventionally pure, and through the
            // smart-pointer deref-peel (type_path) an `Arc<T>`/`Rc<T>` receiver types as `T`, so
            // `arc.clone()` would form `T::clone` and FABRICATE — but `arc.clone()` calls the pointer's
            // own `Arc::clone` (a pure refcount bump), NEVER `T::clone`. An effectful `T::clone` is a rare
            // anti-pattern, so skipping the typed clone resolution is the safe choice (no fabrication).
            if (!matches!(cr, "std" | "core" | "alloc") || std_path_recv) && leaf != "clone" {
                let path = format!("{ty}::{leaf}");
                self.calls.push(Call { path, leaf: leaf.clone(), str_arg, typed: true, method: true, is_macro: false });
            }
        } else {
            // DISPATCH-typed receiver (`&dyn T` / `impl T` / `X: T` / a `Box<dyn T>` field): no
            // concrete type to classify against — previously a SILENT miss (the documented
            // trait-object hole). The JVM engine's bounded-CHA lesson, done on syntax, gated
            // THREE ways (each gate review-earned):
            //  - the trait must be LOCALLY DECLARED and unambiguous — resolving through local
            //    impls of an EXTERNAL trait fabricated effects onto pure generic fns
            //    (`impl Iterator for RowIter` + `fn f(it: impl Iterator)` charged f with
            //    RowIter's Db — execution-verified); external dispatch stays a documented miss;
            //  - the trait's own declaration must carry the called METHOD — a same-named method
            //    on a non-dispatching bound (`T: Store + Default` hitting a Default impl's
            //    `save`) is the same fabrication, and a supertrait call (`.clone()` on a bound
            //    param) must not flood Unknown;
            //  - the dispatch must be narrow (≤12 impls, the cross-engine bound) → edges to
            //    every local implementor; otherwise (or with no impl visible) honest `Unknown`.
            for tr in self.resolve_recv_traits(&node.receiver) {
                let Some(lt) = self.local_traits.get(&tr) else { continue }; // external: documented miss
                // The leaf must be a method the trait declares OR INHERITS from a (local) SUPERTRAIT — a
                // `Super` method is callable on a `Sub`-bound/`dyn Sub` receiver, and the sub's impls (which
                // provide the super method) resolve it via the `trait_impls[tr]` CHA below. Without the
                // supertrait walk `t.base()` (base ∈ Super, `t: T: Sub`) read silent-pure.
                if !self.trait_declares_method(&tr, &leaf, 0) {
                    continue; // blanket/unrelated call — not this trait's dispatch
                }
                if lt.count > 1 {
                    self.unresolved = true; // ambiguous local leaf — never guess between traits
                    continue;
                }
                match self.trait_impls.get(&tr) {
                    Some(impls) if impls.len() <= 12 => {
                        for ty in impls {
                            self.calls.push(Call {
                                path: format!("{ty}::{leaf}"),
                                leaf: leaf.clone(),
                                str_arg: str_arg.clone(),
                                typed: true,
                                method: true,
                                is_macro: false,
                            });
                        }
                    }
                    _ => self.unresolved = true, // >12, or no impl visible: honest indeterminacy
                }
            }
        }
        // ITERATOR-ADAPTER CLOSURE: `xs.iter().for_each(|c| c.send())`, `.map(|c| ..)`, `.filter`, …
        // pass each ELEMENT as the closure's first param. Type that param from the receiver's element
        // type so the closure body — walked lexically below — resolves the element's method calls
        // (else dropped to pure: a §4 under-report on a very common shape). SCOPED via `scoped_var`,
        // so the binding can't leak into a later same-named uninferable var and fabricate (the
        // candor-swift `vars`-leak lesson). When the element type is indeterminate, `scoped_var`
        // still REMOVES any stale binding for the closure body — never leaks an effectful type in.
        let elem_adapter = matches!(
            leaf.as_str(),
            "for_each" | "map" | "filter" | "filter_map" | "flat_map" | "find" | "find_map" | "any"
                | "all" | "position" | "inspect" | "take_while" | "skip_while" | "map_while"
                | "partition" | "fold" | "try_for_each" | "retain" | "sort_by" | "sort_by_key"
                | "min_by_key" | "max_by_key" | "count"
        );
        // The single-ident closure param of the FIRST closure arg (`|c| ..` or `|c, ..| ..`). We
        // only type the FIRST element param — `fold`'s accumulator is its first param so it is NOT a
        // single-param closure and is skipped (would mis-type the accumulator); the common adapters
        // take a single element param. Default-visit the rest; visit the typed closure under scope.
        let elem_ty = if elem_adapter { self.resolve_elem_type(&node.receiver) } else { None };
        let closure_param = if elem_adapter {
            node.args.iter().find_map(|a| match a {
                syn::Expr::Closure(cl) if cl.inputs.len() == 1 => single_pat_ident(cl.inputs.first()?),
                _ => None,
            })
        } else {
            None
        };
        // A NAMED fn / method passed BY VALUE to an INVOKING adapter (`xs.iter().for_each(Conn::send)`,
        // `opt.map(eff_fn)`) is invoked by the adapter, so its effect is reachable — edge to it (sweep [28];
        // the Rust/TS engines' fn-as-value posture). Gated on `elem_adapter` (an invoking HOF) so a STORE
        // sink never fabricates; a bare LOCAL (a value/closure, not a free-fn path) is skipped.
        if elem_adapter {
            for a in &node.args {
                if let syn::Expr::Path(p) = a {
                    let is_local = p.path.get_ident().is_some_and(|i| {
                        let n = i.to_string();
                        self.vars.contains_key(&n) || self.closure_vars.contains(&n) || self.fn_typed_vars.contains(&n)
                    });
                    if p.qself.is_none() && !is_local {
                        let name = path_to_string(&p.path);
                        let path = self.fn_alias.get(&name).cloned().unwrap_or_else(|| expand(&name, self.uses));
                        let leaf2 = path.rsplit("::").next().unwrap_or(&path).to_string();
                        self.calls.push(Call { path, leaf: leaf2, str_arg: None, typed: false, method: false, is_macro: false });
                    }
                }
            }
        }
        // Visit the receiver and args. The receiver and non-closure args carry no element binding; the
        // closure arg (if any) is visited under the scoped element binding so its body resolves `c`.
        // A COLLECTION-OF-TRAIT-OBJECTS receiver (`items.iter().for_each(|it| it.go())` over a
        // `Vec<Box<dyn Doer>>`): the closure param is a trait object → type it into `trait_vars` for
        // bounded-CHA dispatch, the closure-param twin of the for-loop's `elem_trait_of` route. Only when
        // there's no concrete element type (a concrete-element adapter keeps the `vars` route).
        let elem_leaves = if elem_adapter {
            self.resolve_elem_trait_leaves(&node.receiver)
        } else {
            Vec::new()
        };
        self.visit_expr(&node.receiver);
        if let Some(name) = closure_param {
            for a in &node.args {
                if let syn::Expr::Closure(cl) = a {
                    if cl.inputs.len() == 1 && single_pat_ident(cl.inputs.first().unwrap()).as_deref() == Some(name.as_str()) {
                        if !elem_leaves.is_empty() {
                            let prior = self.trait_vars.remove(&name);
                            let prior_var = self.vars.remove(&name);
                            self.trait_vars.insert(name.clone(), elem_leaves.clone());
                            self.visit_expr(&cl.body);
                            self.trait_vars.remove(&name);
                            if let Some(p) = prior { self.trait_vars.insert(name.clone(), p); }
                            if let Some(p) = prior_var { self.vars.insert(name.clone(), p); }
                        } else {
                            self.scoped_var(&name, elem_ty.clone(), |s| s.visit_expr(&cl.body));
                        }
                        continue;
                    }
                }
                self.visit_expr(a);
            }
        } else {
            for a in &node.args {
                self.visit_expr(a);
            }
        }
    }
    fn visit_expr_if(&mut self, node: &'ast syn::ExprIf) {
        // `if let Some(d) = <opt> { d.go() }` / `if let Ok(d) = <res>` where the scrutinee is an
        // Option/Result OF A TRAIT OBJECT (`Option<Box<dyn Doer>>`, a `Some`-of-dyn field) — type the
        // unwrapped binding `d` into `trait_vars` (bounded-CHA dispatch) for the THEN branch, else `d.go()`
        // read silent-pure. Scoped to the then-branch (trait_vars is fn-wide) so it can't leak; the
        // scrutinee + else are visited normally. A concrete/non-dyn payload yields no leaves → default walk.
        if let syn::Expr::Let(el) = &*node.cond {
            if let Some(binding) = some_ok_binding(&el.pat) {
                let leaves = self.resolve_elem_trait_leaves(&el.expr);
                if !leaves.is_empty() {
                    self.visit_expr(&el.expr);
                    let prior = self.trait_vars.remove(&binding);
                    let prior_var = self.vars.remove(&binding);
                    self.trait_vars.insert(binding.clone(), leaves);
                    self.visit_block(&node.then_branch);
                    self.trait_vars.remove(&binding);
                    if let Some(p) = prior {
                        self.trait_vars.insert(binding.clone(), p);
                    }
                    if let Some(p) = prior_var {
                        self.vars.insert(binding.clone(), p);
                    }
                    if let Some((_, else_b)) = &node.else_branch {
                        self.visit_expr(else_b);
                    }
                    return;
                }
            }
        }
        syn::visit::visit_expr_if(self, node);
    }
    fn visit_expr_match(&mut self, node: &'ast syn::ExprMatch) {
        // `match <opt> { Some(d) => d.go(), None => {} }` — when the scrutinee is an Option/Result OF A
        // TRAIT OBJECT, type each `Some(d)`/`Ok(d)` arm's payload into `trait_vars` for that arm's body
        // (bounded-CHA dispatch), else silent-pure. Non-Some/Ok arms keep the normal `visit_arm` route (a
        // LOCAL enum payload); an empty-leaves scrutinee falls through to the default walk.
        let leaves = self.resolve_elem_trait_leaves(&node.expr);
        if !leaves.is_empty() {
            self.visit_expr(&node.expr);
            for arm in &node.arms {
                if let Some(binding) = some_ok_binding(&arm.pat) {
                    let prior = self.trait_vars.remove(&binding);
                    let prior_var = self.vars.remove(&binding);
                    self.trait_vars.insert(binding.clone(), leaves.clone());
                    if let Some((_, guard)) = &arm.guard {
                        self.visit_expr(guard);
                    }
                    self.visit_expr(&arm.body);
                    self.trait_vars.remove(&binding);
                    if let Some(p) = prior {
                        self.trait_vars.insert(binding.clone(), p);
                    }
                    if let Some(p) = prior_var {
                        self.vars.insert(binding.clone(), p);
                    }
                } else {
                    self.visit_arm(arm);
                }
            }
            return;
        }
        syn::visit::visit_expr_match(self, node);
    }
    fn visit_expr_for_loop(&mut self, node: &'ast syn::ExprForLoop) {
        // `for c in xs { c.send() }` / `for c in xs.iter()` / `for c in &self.senders` — type the
        // loop variable from the iterated collection's element type so the body's `c.method()` calls
        // resolve (else dropped to pure: a §4 under-report on the most common iteration shape). SCOPED
        // to the BODY ONLY via `scoped_var`: `vars` is function-wide, so an unscoped binding would leak
        // into a later same-named uninferable var and FABRICATE its effect (the candor-swift bug). When
        // the element type is indeterminate, the binding is still cleared for the body, never leaked in.
        // The iterated expr is visited FIRST (outside the binding — it's evaluated before the body).
        self.visit_expr(&node.expr);
        // A `for _ in <concrete-local-iter>` IMPLICITLY drives the iterator's `next()` to completion —
        // charge the local `Type::next` so its effect isn't silently dropped (see `iter_next_target`).
        self.charge_iter_next(&node.expr);
        if let Some(name) = single_pat_ident(&node.pat) {
            let elem = self.resolve_elem_type(&node.expr);
            // A `for it in items` over a COLLECTION OF TRAIT OBJECTS (`items: Vec<Box<dyn Doer>>`) types
            // the loop var into `trait_vars` for dispatch (`it.go()` → bounded CHA over Doer's impls),
            // which `elem_of`/`vars` can't express (a `dyn` element has no nominal type). Only when there's
            // no concrete element type (a concrete-element collection takes the `vars` route above).
            // Prefer the trait-object route whenever the element is a dispatch type — `elem_trait_of` is
            // populated ONLY for a `dyn`/`impl`/generic-bound element, and for a `Vec<T: Doer>` element
            // `elem_type` returns the bogus generic-param name "T" (not None), so gating on `elem.is_none()`
            // would wrongly take the (dead) concrete route. A concrete-element collection has empty leaves.
            let leaves = self.resolve_elem_trait_leaves(&node.expr);
            if !leaves.is_empty() {
                let prior = self.trait_vars.remove(&name);
                let prior_var = self.vars.remove(&name); // dispatch-typed shadows any stale concrete binding
                self.trait_vars.insert(name.clone(), leaves);
                self.visit_block(&node.body);
                self.trait_vars.remove(&name);
                if let Some(p) = prior {
                    self.trait_vars.insert(name.clone(), p);
                }
                if let Some(p) = prior_var {
                    self.vars.insert(name.clone(), p);
                }
            } else {
                self.scoped_var(&name, elem, |s| s.visit_block(&node.body));
            }
        } else {
            // A destructuring loop pattern (`for (k, v) in ..`, `for [a, b] in ..`) — no single name
            // to type; just walk the body. (Tuple-pair value typing is left to the under-report.)
            self.visit_block(&node.body);
        }
    }
    fn visit_arm(&mut self, node: &'ast syn::Arm) {
        // ENUM-PAYLOAD MATCH BINDING: `match c { Conn::Active(s) => s.send() }` — the arm pattern
        // `Conn::Active(s)` is a `Pat::TupleStruct` whose single field binds `s` to the variant's
        // payload type. Type `s` from the Pass-A enum-variant index so `s.method()` resolves (else
        // dropped to pure: a §4 under-report). SCOPED to the arm body + guard via `scoped_var`, so the
        // binding can't leak into a later arm or a later same-named var (the `vars`-leak fabrication).
        let binding = arm_payload_binding(&node.pat, self.enum_variants);
        if let Some((name, ty)) = binding {
            self.scoped_var(&name, ty, |s| {
                if let Some((_, guard)) = &node.guard {
                    s.visit_expr(guard);
                }
                s.visit_expr(&node.body);
            });
        } else {
            syn::visit::visit_arm(self, node);
        }
    }
    fn visit_expr_path(&mut self, node: &'ast syn::ExprPath) {
        // FORCING a lazy/deferred static: any mention of the static's NAME (deref `*X`, `X.method()`,
        // `Lazy::force(&X)`, `X.with(..)`, or a bare path `X`) runs its deferred init on first use. Edge
        // to the static's synthetic init unit (`<lazy>::NAME`) so the init's effect propagates here. We
        // key on the LAST path segment so a module-qualified mention (`config::CONFIG`) also forces, and
        // skip a name shadowed by a LOCAL binding (a same-named param/let/closure is not the static).
        if node.qself.is_none() {
            if let Some(last) = node.path.segments.last() {
                let name = last.ident.to_string();
                let locally_bound = self.vars.contains_key(&name)
                    || self.closure_vars.contains(&name)
                    || self.fn_typed_vars.contains(&name)
                    || self.fn_alias.contains_key(&name)
                    || self.elem_of.contains_key(&name)
                    || self.trait_vars.contains_key(&name);
                if !locally_bound
                    && self.lazy_statics.contains(&name)
                    && self.forced_lazies.insert(name.clone())
                {
                    let qual = format!("{LAZY_UNIT_PREFIX}::{name}");
                    // path has `::` (the `<lazy>::` prefix) so it resolves via the tail2 route in
                    // `resolve_target`, edging to the unique synthetic unit. Not a macro/typed/method.
                    self.calls.push(Call { path: qual, leaf: name, str_arg: None, typed: false, method: false, is_macro: false });
                }
            }
        }
        syn::visit::visit_expr_path(self, node);
    }
    fn visit_expr_closure(&mut self, node: &'ast syn::ExprClosure) {
        // Type each ANNOTATED closure param (`|c: &Conn| c.send()`) into `vars` so the body's `c.method()`
        // resolves — the annotation was discarded, so an effectful method on it read silent-pure (sweep
        // [29]). Save+restore scopes the bindings to the closure body (closures are walked lexically here).
        let mut saved: Vec<(String, Option<String>)> = Vec::new();
        for input in &node.inputs {
            if let syn::Pat::Type(pt) = input {
                if let (Some(name), Some(ty)) = (single_pat_ident(&pt.pat), type_path(&pt.ty, self.uses)) {
                    let prev = self.vars.insert(name.clone(), ty);
                    saved.push((name, prev));
                }
            }
        }
        syn::visit::visit_expr_closure(self, node);
        for (name, prev) in saved {
            match prev {
                Some(v) => { self.vars.insert(name, v); }
                None => { self.vars.remove(&name); }
            }
        }
    }
    fn visit_local(&mut self, node: &'ast syn::Local) {
        // `let Some(d) = <opt> else { .. };` (let-else) — the unwrapped payload of an Option/Result OF A
        // TRAIT OBJECT is valid for the REST of the fn (let-else binds fn-wide), so type `d` into
        // `trait_vars` (fn-wide, like any top-level let) → `d.go()` dispatches. Only a `Some`/`Ok` pattern
        // reaches `some_ok_binding` (a refutable `let` without `else` won't compile); a concrete payload
        // yields no leaves. The init/else are walked by the trailing `syn::visit::visit_local` below.
        if let Some(binding) = some_ok_binding(&node.pat) {
            if let Some(init) = &node.init {
                let leaves = self.resolve_elem_trait_leaves(&init.expr);
                if !leaves.is_empty() {
                    self.vars.remove(&binding);
                    self.trait_vars.insert(binding, leaves);
                }
            }
        }
        // Record `let x: T = ..` (annotated) and `let x = T::new(..)` (constructor) so later method
        // calls on `x` resolve. Visited in source order, before any use of `x` (Rust requires it).
        if let syn::Pat::Type(pt) = &node.pat {
            if let syn::Pat::Ident(id) = &*pt.pat {
                // Dispatch-typing first (`let s: Box<dyn Store>` reads as concrete `Box` otherwise).
                // A fn-typed let (`let g: fn() = ..`, `: impl Fn() = ..`, `: Box<dyn Fn> = ..`): invoking
                // `g()` calls an opaque body, so track it for the call-site `fn_typed_vars` check (else it
                // resolves as a phantom free-fn `g` and is silently dropped — the max review's local-rebind
                // find). Annotation wins over a stale binding from any source.
                if is_callable_type(&pt.ty, &HashMap::new()) {
                    self.fn_typed_vars.insert(id.ident.to_string());
                    self.vars.remove(&id.ident.to_string());
                } else {
                    self.fn_typed_vars.remove(&id.ident.to_string()); // a non-callable annotation clears it
                }
                let leaves = trait_leaves(&pt.ty, &HashMap::new());
                if !leaves.is_empty() {
                    self.vars.remove(&id.ident.to_string()); // a stale concrete binding must not shadow the rebind
                    self.trait_vars.insert(id.ident.to_string(), leaves);
                } else if let Some(ty) = type_path(&pt.ty, self.uses) {
                    self.vars.insert(id.ident.to_string(), ty);
                }
                // A COLLECTION-typed let (`let xs: Vec<Sender> = ..`) — record its element type so a
                // later `for c in xs`, `xs[0]`, `xs.iter().for_each(..)` resolves the element.
                if let Some(e) = elem_type(&pt.ty, self.uses) {
                    self.elem_of.insert(id.ident.to_string(), e);
                }
                // A TUPLE-typed let (`let pair: (Sender, usize) = ..`) — record its per-position types
                // so a later `let (s, _) = pair;` types `s`.
                self.tuple_of.remove(&id.ident.to_string());
                if let Some(t) = tuple_types(&pt.ty, self.uses) {
                    self.tuple_of.insert(id.ident.to_string(), t);
                }
                // IMPLICIT `.into()` → `From::from` (#5), TARGET resolved from the annotation. `let d:
                // Dst = src.into();` desugars to `Dst::from(src)` via a local `impl From<Src> for Dst`;
                // the conversion body lives on `Dst` (the annotated type), so charge `Dst::from` when `Dst`
                // is a LOCAL `impl From`. We only do this where the target is KNOWN (the annotation) — a
                // bare `f(x.into())` arg whose target is inferred from the callee's param type can't be
                // resolved syntactically → skipped (no flood; an `.into()` to a std type lights nothing).
                if let Some(init) = &node.init {
                    if expr_is_into_call(&init.expr) {
                        if let Some(tgt) = type_path(&pt.ty, self.uses) {
                            let tgt_leaf = tgt.rsplit("::").next().unwrap_or(&tgt).to_string();
                            self.charge_from(&tgt_leaf);
                        }
                    }
                }
            } else if let syn::Pat::Tuple(tup) = &*pt.pat {
                // ANNOTATED TUPLE DESTRUCTURE: `let (s, _): (Sender, usize) = pair;` — bind each
                // single-ident element to its annotated tuple element type so `s.send()` resolves.
                if let syn::Type::Tuple(tty) = &*pt.ty {
                    self.bind_tuple(&tup.elems, tty.elems.iter());
                }
            }
        } else if let syn::Pat::Tuple(tup) = &node.pat {
            // UNANNOTATED TUPLE DESTRUCTURE: `let (s, _) = pair;` / `let (s, _) = (svc, 0);` — type each
            // binding from (a) a tuple-typed source VAR's recorded per-position types (`pair`'s
            // `tuple_of`), or (b) a tuple LITERAL initializer's per-element exprs. A non-tuple, untyped
            // source carries no per-element type — honest under-report. Each binding clears stale state.
            let init = node.init.as_ref().map(|i| &*i.expr);
            let src_tuple = match init {
                Some(syn::Expr::Path(p)) => p
                    .path
                    .get_ident()
                    .and_then(|id| self.tuple_of.get(&id.to_string()))
                    .cloned(),
                _ => None,
            };
            for (i, pat_el) in tup.elems.iter().enumerate() {
                let Some(name) = single_pat_ident(pat_el) else { continue };
                self.vars.remove(&name);
                self.elem_of.remove(&name);
                self.tuple_of.remove(&name);
                let ty = src_tuple
                    .as_ref()
                    .and_then(|t| t.get(i).cloned().flatten())
                    .or_else(|| match init {
                        Some(syn::Expr::Tuple(it)) => {
                            it.elems.iter().nth(i).and_then(|e| ctor_type(e, self.uses, self.returns))
                        }
                        _ => None,
                    });
                if let Some(ty) = ty {
                    self.vars.insert(name, ty);
                }
            }
        } else if let syn::Pat::Ident(id) = &node.pat {
            // CONST-STRING PROPAGATION, local level: `let url = format!("{}/x", API_BASE)` / `let url =
            // "https://…"` / `let url = API_BASE` — record `url`'s resolved literal so a later `post(url)`
            // recovers the host (SPEC §1). One level only; a rebind to a non-resolvable value CLEARS the
            // entry (a literal `String`-typed binding first, an inline `&str`, or a const-anchored format).
            let name = id.ident.to_string();
            self.str_locals.remove(&name); // clear stale binding on any rebind
            if let Some(init) = &node.init {
                let resolved = const_str_value(&init.expr).or_else(|| self.resolve_str_expr(&init.expr));
                if let Some(v) = resolved {
                    self.str_locals.insert(name, v);
                }
            }
            if let Some(init) = &node.init {
                if matches!(&*init.expr, syn::Expr::Closure(_)) {
                    // `let f = |..| ..` — remember `f` so a later `f()` is seen as a closure call.
                    self.closure_vars.insert(id.ident.to_string());
                } else {
                    // Rebinding the name to a NON-closure (a fn-pointer, a value) — drop any stale
                    // closure marking so a later `f()` isn't wrongly treated as a visible closure.
                    self.closure_vars.remove(&id.ident.to_string());
                    // Propagate fn-typed-ness through a rebind from a fn-typed binding (`let g = cb` where
                    // `cb: fn()`/`impl Fn`): invoking `g()` is the same opaque-callback call as `cb()` →
                    // Unknown, not a phantom free-fn `g` (the max review found the param-only seeding
                    // missed this). A rebind to a non-fn clears the stale fn-typed marking.
                    self.fn_alias.remove(&id.ident.to_string()); // drop any stale alias on rebind
                    if self.expr_is_fn_typed(&init.expr) {
                        self.fn_typed_vars.insert(id.ident.to_string());
                        self.vars.remove(&id.ident.to_string());
                    } else {
                        self.fn_typed_vars.remove(&id.ident.to_string());
                        if let Some(ty) = ctor_type(&init.expr, self.uses, self.returns) {
                            self.vars.insert(id.ident.to_string(), ty.clone());
                            // DROP-GLUE via STRUCT-LITERAL / UNIT construction (#6). The existing drop-glue
                            // catches only `let g = T::new()` constructor CALLS — a `let g = Guard { .. };`
                            // or bare `let g = UnitGuard;` builds a `T` value just the same (its `impl Drop`
                            // runs at scope exit) but emits NO call, so an effectful-Drop guard built this
                            // way read silent-pure. Emit a synthetic CONSTRUCTION marker (`T::<construct>`,
                            // method=false, an angle-bracket leaf that can't collide with a real fn or a
                            // crate classifier) so the call-loop's drop detection — gated on `drop_types`
                            // (LOCAL `impl Drop` only) — picks `T` up and edges to `T::drop`. A non-drop
                            // type's marker is inert (filtered out there); a plain typed binding (a CALL
                            // init, already handled) is excluded — only literal construction emits this.
                            if matches!(&*init.expr, syn::Expr::Struct(_) | syn::Expr::Path(_)) {
                                let ty_leaf = ty.rsplit("::").next().unwrap_or(&ty).to_string();
                                self.calls.push(Call {
                                    path: format!("{ty_leaf}::<construct>"),
                                    leaf: "<construct>".to_string(),
                                    str_arg: None,
                                    typed: false,
                                    method: false,
                                    is_macro: false,
                                });
                            }
                        }
                        // `let g = eff;` where the init is a bare PATH (not a call) — `g` aliases a free fn,
                        // so a later `g()` resolves to it (sweep [6]). `g()` only compiles if the path is
                        // callable, so aliasing any bare path is sound (an unused alias is never resolved).
                        if let syn::Expr::Path(p) = &*init.expr {
                            let single_local = p.path.get_ident().is_some_and(|i| {
                                let n = i.to_string();
                                self.vars.contains_key(&n) || self.closure_vars.contains(&n)
                                    || self.fn_typed_vars.contains(&n)
                            });
                            if p.qself.is_none() && !single_local {
                                self.fn_alias.insert(id.ident.to_string(), expand(&path_to_string(&p.path), self.uses));
                            }
                        }
                    }
                    // Carry an element type through an element-preserving rebind (`let xs =
                    // self.senders.clone()`, `let xs = pool.conns()` factory) so `xs[0]`/`for c in xs`
                    // still resolve. Drop any STALE element binding first — a rebind to a non-collection
                    // must not leave the old element type to mis-type a later subscript/loop.
                    self.elem_of.remove(&id.ident.to_string());
                    if let Some(e) = self.resolve_elem_type(&init.expr) {
                        self.elem_of.insert(id.ident.to_string(), e);
                    }
                }
            }
        }
        syn::visit::visit_local(self, node);
    }
    fn visit_macro(&mut self, node: &'ast syn::Macro) {
        // The macro PATH itself can carry/hide an effect that a syntactic (pre-expansion) scan can't see.
        // Record a CRATE-QUALIFIED macro path (has `::`) so attribution handles it like any external call:
        //   - classified → its effect (the log/tracing EMIT macros: `log::info!`, `tracing::warn!`),
        //   - a builder ENTRY → its effect (`duct::cmd!`), or
        //   - an unmodeled DECLARED dep → DISCLOSED blind (`slog::info!` → invisible, not silent-pure: the
        //     macro-disclosure gap — a macro reach now gets the same honest Unknown a normal call does).
        // BARE macros (`println!`/`vec!`/`format!`/`matches!`) have no `::` → skipped (no spurious edge).
        // CRUCIAL: a crate-LOCAL qualified macro (`crate::helpers::trace!` → `expand` → `helpers::trace`)
        // keeps its `::`, so it is NOT necessarily external — flag it `is_macro` so local edge resolution
        // SKIPS it (a macro is never a call to a local FUNCTION; without this it would mis-link to a
        // same-named local fn and fabricate that fn's effect onto a pure caller). Classification, the
        // builder table, and κ blind-disclosure still apply (they key on the path/crate, not the edge).
        let mpath = expand(&path_to_string(&node.path), self.uses);
        let mleaf = mpath.rsplit("::").next().unwrap_or(&mpath).to_string();
        // `cfg_if::cfg_if! { if #[cfg(..)] { .. } else if #[cfg(..)] { .. } else { .. } }` (and the bare
        // `cfg_if!` after `use cfg_if::cfg_if`) is a MACRO that syn leaves opaque, so every effectful call
        // inside an arm reads pure and the reach is disclosed only as an `invisible: ["cfg_if"]` blind —
        // MISLEADING when the arm holds the user's own COVERED std call (sqlx-core's platform code). Expand
        // it: parse the arm grammar and walk EVERY arm's block through the normal effect walk (the same
        // sound all-cfg-branches over-approximation candor-scan already applies to `#[cfg(unix)]`/etc. —
        // it never fabricates, the calls are real, just cfg-gated). On an unexpected token shape the parse
        // fails and we fall through to the opaque-macro path below (never panic). Because a successful
        // expansion COVERS the reach, it is NOT recorded as an invisible macro call.
        if mleaf == "cfg_if" {
            if let Ok(arms) = syn::parse2::<CfgIfArms>(node.tokens.clone()) {
                for block in &arms.0 {
                    self.visit_block(block);
                }
                return;
            }
        }
        if mpath.contains("::") {
            self.calls.push(Call { path: mpath, leaf: mleaf.clone(), str_arg: None, typed: false, method: false, is_macro: true });
        }
        // syn does not parse a macro's body, so every call hidden inside one is invisible by default —
        // a real miss on crates that route effectful calls through a macro (git2 wraps EVERY libgit2 FFI
        // call in `try_call!(raw::git_...())`; `println!("{}", f())` hides `f`). Best-effort: parse the
        // token stream as comma-separated expressions and walk any that parse. If the body isn't
        // expression syntax (`quote!{}`, `matches!(x, Pat)`, macro_rules arms), parsing fails and we skip
        // — so this only ever ADDS visibility, never breaks. Owned exprs, so visit a local copy.
        let parser = syn::punctuated::Punctuated::<syn::Expr, syn::Token![,]>::parse_terminated;
        if let Ok(exprs) = syn::parse::Parser::parse2(parser, node.tokens.clone()) {
            // IMPLICIT FORMATTING (#2): a formatting macro (`format!`/`println!`/`write!`/…) runs each
            // `{}`/`{:?}` argument through `Display::fmt`/`Debug::fmt`. A LOCAL type with an effectful
            // `impl Display`/`impl Debug` (a custom formatter that touches the fs/net/etc.) is therefore
            // reached at every format site but reads silent-pure. When this is such a macro, charge the
            // coercion for each formatted argument whose type is a concrete local impl. The format STRING
            // (the first literal) tells us which args are `{:?}` (Debug) vs `{}`/`{:…}` (Display); an arg
            // we can't map to a holder defaults to Display (the bare `{}` case). A std/external arg type
            // (`String`, `i32`) resolves to no local impl → no edge (no flood — the common case).
            if is_format_macro(&mleaf) {
                self.charge_format_args(&mleaf, &exprs);
            }
            for e in &exprs {
                self.visit_expr(e);
            }
        }
    }
    fn visit_expr_try(&mut self, node: &'ast syn::ExprTry) {
        // THE `?` OPERATOR (#1): `may_fail()?` in a fn returning `Result<_, E2>`, where the operand is
        // `Result<_, E1>` with `E1 != E2`, desugars to `E2::from(e1)` via a local `impl From<E1> for E2`.
        // The conversion body lives on the enclosing fn's ERROR type `E2` (`err_ret_leaf`); edge to
        // `E2::from` when `E2` is a LOCAL `impl From`. A `?` whose enclosing error type is unknown /
        // std / `Box<dyn Error>` (the overwhelming case) has no `err_ret_leaf` → no edge (no flood).
        if let Some(err_leaf) = self.err_ret_leaf.clone() {
            self.charge_from(&err_leaf);
        }
        syn::visit::visit_expr_try(self, node);
    }
    fn visit_expr_binary(&mut self, node: &'ast syn::ExprBinary) {
        // OPERATOR OVERLOADS (#4): a binary operator on a value of a concrete LOCAL type with the matching
        // `impl Add`/`Sub`/…/`PartialEq`/`PartialOrd` runs that impl's method (`a + b`→`Add::add`,
        // `a == b`→`PartialEq::eq`, `a < b`→`PartialOrd::partial_cmp`). An effectful operator impl
        // (rare, but a real reach) reads silent-pure otherwise. Charge on the LEFT operand's type (the
        // dispatch receiver for `Add`/`PartialEq`/…). A std/primitive operand resolves to no local impl
        // → no edge (no flood on arithmetic over `i32`/`usize`/etc.).
        if let Some((tr, method)) = binop_trait(&node.op) {
            self.charge_coercion(&node.left, tr, method);
        }
        syn::visit::visit_expr_binary(self, node);
    }
    fn visit_expr_unary(&mut self, node: &'ast syn::ExprUnary) {
        // UNARY OPERATOR OVERLOADS (#4): `-a`→`Neg::neg`, `!a`→`Not::not`, and the DEREF coercion
        // `*w`→`Deref::deref` (#3) on a concrete local impl. (`&a`/`Borrow` is not an operator overload.)
        match &node.op {
            syn::UnOp::Neg(_) => self.charge_coercion(&node.expr, "Neg", "neg"),
            syn::UnOp::Not(_) => self.charge_coercion(&node.expr, "Not", "not"),
            // `*w` on a local `impl Deref for W` runs `W::deref` (the explicit-deref case of #3).
            syn::UnOp::Deref(_) => self.charge_coercion(&node.expr, "Deref", "deref"),
            _ => {}
        }
        syn::visit::visit_expr_unary(self, node);
    }
    fn visit_expr_index(&mut self, node: &'ast syn::ExprIndex) {
        // INDEX OVERLOAD (#4): `a[i]` on a concrete local `impl Index for A` runs `A::index`. (A std
        // collection / slice / array operand resolves to no local impl → no edge.)
        self.charge_coercion(&node.expr, "Index", "index");
        syn::visit::visit_expr_index(self, node);
    }
}

/// Resolve a call to the local definition(s) it links to in the intra-crate graph, or `None` to
/// under-report. A QUALIFIED path (`a::Job::run`, `mod::helper`, or an associated-fn call `Type::new()`)
/// matches on its precise 2-segment tail, but ONLY when that tail is UNAMBIGUOUS — two same-named types in
/// different modules share a tail (`a::Job::run` / `b::Job::run`), so linking a many-way tail would
/// fabricate one type's effect onto the other's caller (the same flood the bare-leaf index causes, one
/// level up). An UNQUALIFIED free-function call falls back to a unique bare leaf. An UNQUALIFIED method
/// call with an unresolved receiver names no definite target, so it under-reports rather than guess —
/// this is what stops `range.start()` linking to a unique local `Clipboard::start`. NB a receiver-typed
/// `Type::method` call DOES arrive here (via the qualified-tail branch) — but only after the caller has
/// confirmed `Type` is locally defined, so an external `reqwest::Client::send` is filtered out upstream.
pub(crate) fn resolve_target<'a>(
    path: &str,
    leaf: &str,
    method: bool,
    by_tail2: &'a HashMap<String, Vec<String>>,
    by_leaf: &'a HashMap<String, Vec<String>>,
) -> Option<&'a Vec<String>> {
    if path.contains("::") {
        tail2(path).and_then(|t2| by_tail2.get(&t2)).filter(|v| v.len() == 1)
    } else if method {
        None
    } else {
        by_leaf.get(leaf).filter(|v| v.len() == 1)
    }
}

/// The parsed BLOCKS of a `cfg_if::cfg_if! { .. }` body — one per arm. `cfg_if`'s grammar is a chain of
/// `if #[cfg(COND)] { BLOCK }` clauses (each cfg is an outer attribute on the arm), optionally chained by
/// `else if #[cfg(COND)] { BLOCK }`, and optionally terminated by a bare `else { BLOCK }`. We keep EVERY
/// arm's block (sound over-approximation — see the call site) and DISCARD the `#[cfg(..)]` conditions
/// themselves: they only decide which arm compiles, not what any arm does, and candor-scan already scans
/// all `#[cfg(unix)]`/`#[cfg(windows)]`/… branches regardless. Parsing is strict — any token that isn't
/// this exact `if #[cfg(..)] { } [else if #[cfg(..)] { }]* [else { }]?` shape makes the parse fail, so the
/// caller falls back to treating the macro as opaque rather than mis-reading a novel `cfg_if` extension.
struct CfgIfArms(Vec<syn::Block>);

impl syn::parse::Parse for CfgIfArms {
    fn parse(input: syn::parse::ParseStream) -> syn::Result<Self> {
        let mut blocks = Vec::new();
        loop {
            // `if #[cfg(..)] { .. }` — require the `if`, the (discarded) outer `#[cfg(..)]` attribute(s),
            // then the arm block.
            input.parse::<syn::Token![if]>()?;
            let _attrs = input.call(syn::Attribute::parse_outer)?;
            blocks.push(input.parse::<syn::Block>()?);
            if input.is_empty() {
                break;
            }
            // A trailing chain: `else { .. }` (final arm) or `else if #[cfg(..)] { .. }` (loop again).
            input.parse::<syn::Token![else]>()?;
            if input.peek(syn::Token![if]) {
                continue; // `else if` — the loop head parses the next `if #[cfg(..)] { .. }`
            }
            blocks.push(input.parse::<syn::Block>()?); // bare `else { .. }` — the last arm
            break;
        }
        if !input.is_empty() {
            // Trailing tokens after a well-formed chain ⇒ not the grammar we understand; bail to opaque.
            return Err(input.error("unexpected trailing tokens in cfg_if! body"));
        }
        Ok(CfgIfArms(blocks))
    }
}
