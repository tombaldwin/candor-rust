//! Pass B: `CallCollector` walks one function body and records its calls, resolving
//! receivers through the Pass-A indexes. `resolve_target` is the shared call resolver.

use crate::*;

/// What a binder knows about the name it introduces — the argument to `scoped_binding`.
/// `Unknown` is a real case, not a fallback: a loop variable over an untypable iterator still BINDS the
/// name, and it is precisely that case where a stale side-table entry is not masked by a fresh one.
pub(crate) enum Bound {
    /// A concrete type path (`vars`).
    Concrete(String),
    /// Dispatch-trait leaves for a `dyn`/generic element (`trait_vars`).
    Traits(Vec<String>),
    /// The name is bound but its type is undetermined — clear, install nothing.
    Unknown,
}

pub(crate) struct CallCollector<'a> {
    /// The module path of the function being walked. A bare `CFG` reference names the ENCLOSING module's
    /// static, so the forcing edge must be built with this path — see `lazy_qual`.
    pub(crate) modpath: String,
    pub(crate) uses: &'a HashMap<String, String>,
    /// local variable / param / `self` -> expanded type path, grown as `let`s are visited in order.
    pub(crate) vars: HashMap<String, String>,
    /// local variable / param -> trait bound leaves, for dispatch-typed receivers (`t: &dyn Store`,
    /// `s: impl Store`, `x: X` under `X: Store`). Disjoint from `vars` (no concrete type to put there).
    pub(crate) trait_vars: HashMap<String, Vec<String>>,
    /// The subset of this signature's trait leaves spelled in a `dyn` (TYPE-ERASED) position — `&dyn T`,
    /// `Box<dyn T>`, `Vec<Box<dyn T>>` — as opposed to a generic bound or `impl Trait`, which the CALLER
    /// monomorphizes. The IMPORTED-trait CHA (R4) fires only on these; see `lang::dyn_sig_trait_leaves`
    /// for why, and for the serde_json measurement that forced the distinction.
    pub(crate) dyn_sig_traits: std::collections::HashSet<String>,
    /// This signature's generic parameter -> its trait bounds (`<T: Doer>` → `T -> ["Doer"]`), i.e.
    /// `lang::generic_bounds_of(sig)`. Pass A already threads exactly this into every PARAMETER, FIELD
    /// and RETURN position; the collector needs its own copy because a LOCAL `let` can name a generic
    /// too (`let d: T = pick();`) and Pass B is the only place that type annotation is read. Scoped
    /// across a nested `fn`/`impl` beside `dyn_sig_traits`, for the same reason — a nested item's `T`
    /// is its own, not the enclosing signature's.
    pub(crate) generic_bounds: HashMap<String, Vec<String>>,
    /// Trait leaf -> the multi-segment path this signature WROTE the bound with (`&dyn deplib::Handler`).
    /// `bound_leaves` keeps only the leaf, so a FULLY-QUALIFIED receiver otherwise loses its crate
    /// identity entirely and never forms the crate-qualified key — R6. See `lang::sig_trait_quals`.
    /// PER-PARAMETER qualified bounds — the precise form of `trait_quals`. Consulted FIRST when the
    /// receiver is a plain parameter, so `fn handle(a: &dyn alpha::Handler, b: &dyn beta::Handler)`
    /// resolves each receiver to its OWN crate instead of collapsing both onto one leaf.
    pub(crate) trait_quals_by_param: HashMap<String, HashMap<String, String>>,
    pub(crate) trait_quals: HashMap<String, String>,
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
    /// local var / param -> per-position DISPATCH-trait leaves of a TUPLE with a trait-object element
    /// (`pair: (Box<dyn Doer>, u32)` -> `[["Doer"], []]`). The trait-object counterpart of `tuple_of`, so a
    /// `let (d, _) = pair; d.go()` binds `d` into `trait_vars` (bounded CHA) — `tuple_of` can't hold a `dyn`
    /// element (it has no nominal path). Grown as tuple-of-dyn params / var rebinds are seen (R46 tuple).
    pub(crate) tuple_trait_of: HashMap<String, Vec<Vec<String>>>,
    pub(crate) calls: Vec<Call>,
    /// locals bound to a closure (`let f = |..| ..`), so a later `f()` is recognised as a closure
    /// invocation the scan can't see through — not a call to a free fn named `f`.
    pub(crate) closure_vars: std::collections::HashSet<String>,
    /// params/locals of a fn-pointer / `impl`/`dyn Fn` / generic-`Fn`-bound type. Invoking one (`cb()`)
    /// calls an opaque body → honest `Unknown`, not a silently-dropped phantom call to a free fn `cb`.
    pub(crate) fn_typed_vars: std::collections::HashSet<String>,
    /// locals bound from a call into a CROSS-CRATE path whose return type we could not determine —
    /// `let c = deplib::build();`. The value's PROVENANCE is known (the crate root) even though its TYPE
    /// is not, and that conjunction is exactly the "could-not-form-a-key" case: a later `c.fetch()` asks
    /// no question of the chained report, so its silence licenses nothing. See
    /// candor-spec/DEP-RECEIVER-TYPING-DESIGN.md — this is half 1, the disclosure half, which needs no
    /// format change. Deliberately NOT "untyped receiver" (pervasive, and hedging on it is the 8-25%
    /// false-uncertainty flood measured in COVERAGE-GRANULARITY-FINDING.md) — only the conjunction.
    pub(crate) dep_bound_vars: HashMap<String, String>,
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
    /// LOCAL `macro_rules!` NAME → its arm TOKENS (as a string). A bare `NAME!(..)` invocation inline-expands
    /// the template so an effectful macro body (`macro_rules! do_io { () => { fs::write(..) } }`) isn't
    /// silent-pure (R48). Metavars are `$`-stripped and the template parse-or-skipped as a block — only ever
    /// ADDS visibility (an unparseable/`$(..)*`-repetition template is skipped), never fabricates.
    pub(crate) local_macros: &'a std::collections::HashMap<String, String>,
    /// Local macros currently being inline-expanded on this path — a recursion guard so a macro whose
    /// template invokes itself (or a mutually-recursive macro) can't loop forever.
    pub(crate) macro_expanding: std::collections::HashSet<String>,
    /// LOCAL string bindings we can resolve to a literal host — `let url = format!("{}/x", API_BASE)` /
    /// `let url = "https://…"` / `let url = API_BASE` — so a later `post(url)` recovers the host (ONE level
    /// of local `let` following; a rebind to a non-resolvable value clears the entry). Grown in source
    /// order, like `vars`. Literal/const-anchored only — a runtime-built binding contributes nothing.
    pub(crate) str_locals: std::collections::HashMap<String, String>,
    /// FUNCTION-BODY `use` bindings (`fn f() { use deplib::CFG; .. }`) — the same `name -> full path` map
    /// as `uses`, which is collected from FILE-level items only. A body-level `use` is the OTHER SPELLING
    /// of a file-level one and reaches the identical code, so consulting only `uses` made a forcing site
    /// silent purely because of where the import was written. Fn-wide rather than block-scoped: the
    /// over-approximation direction (a `use` in one block seen by a later block) can only ADD a candidate
    /// key, and every key here is speculative and inert unless the dependency report actually carries it.
    pub(crate) local_uses: std::collections::HashMap<String, String>,
    /// EVERY ident in a binding position anywhere in this body or signature (`let`, param, closure param,
    /// `for` pattern, match arm) — computed once up front by `lang::bound_idents`. The typed side-tables
    /// (`vars`, `elem_of`, …) only hold a binding whose type was RECOVERABLE, so they cannot answer "is
    /// this name shadowed by a local?" for a `let` that typed to nothing. The forcing/provenance sites use
    /// this instead, so their shadow test does not depend on inference having succeeded.
    /// Flow-INSENSITIVE and body-wide, matching `vars`' existing discipline: a false shadow costs a
    /// forcing edge (a miss), where a missed shadow charges a static's effect to a local (a fabrication).
    pub(crate) bound_names: std::collections::HashSet<String>,
}

impl<'a> CallCollector<'a> {
    /// What a bare NAME was imported as, consulting the body-level `use` map first and the file-level one
    /// second (a body-level `use` shadows a file-level import of the same name). Returns `None` for a name
    /// no `use` brought in — which is the honest answer for a name declared in this very module.
    fn use_target(&self, name: &str) -> Option<&String> {
        self.local_uses.get(name).or_else(|| self.uses.get(name))
    }

    /// Normalise a WRITTEN module prefix (the segments before a name) into the module path a local unit's
    /// qual is built from. `crate::` is dropped (a unit qual is crate-relative), `self::` means this
    /// function's own module, and each `super::` pops one segment off it. An empty result means "this
    /// module", which is the caller's fallback anyway.
    fn normalise_modpath(&self, segs: &[&str]) -> String {
        let mut out: Vec<String> = Vec::new();
        let mut rest = segs;
        loop {
            match rest.first().copied() {
                Some("crate") => rest = &rest[1..],
                Some("self") => {
                    out = self.modpath.split("::").filter(|s| !s.is_empty()).map(str::to_string).collect();
                    rest = &rest[1..];
                }
                Some("super") => {
                    if out.is_empty() {
                        out = self.modpath.split("::").filter(|s| !s.is_empty()).map(str::to_string).collect();
                    }
                    out.pop();
                    rest = &rest[1..];
                }
                _ => break,
            }
        }
        out.extend(rest.iter().map(|s| s.to_string()));
        out.join("::")
    }

    /// The module path a lazy-static forcing site NAMES, derived from the SPELLING rather than from the
    /// reader's own module. `*m::INNER` names `m`; `use m::INNER; *INNER` names `m` too. Returns `None`
    /// when the spelling says nothing (a bare name no `use` imported) — then the reader's own module is
    /// the right answer and the caller falls back to it.
    ///
    /// This is the reader half of `lazy_qual`. `5447eba` moved the module path INSIDE the `<lazy>::`
    /// prefix so two same-named statics stop merging — that made the WRITER module-qualified while the
    /// reader still built `<lazy>::<my own module>::NAME`, so every cross-module read of a module-scoped
    /// lazy static missed its unit's tail2 and read silent-pure.
    fn named_lazy_modpath(&self, segs: &[&str]) -> Option<String> {
        if segs.len() >= 2 {
            return Some(self.normalise_modpath(&segs[..segs.len() - 1]));
        }
        // A bare name: a `use` (file- or body-level) is what says which module it came from.
        let full = self.use_target(segs[0])?;
        let fsegs: Vec<&str> = full.split("::").collect();
        (fsegs.len() >= 2).then(|| self.normalise_modpath(&fsegs[..fsegs.len() - 1]))
    }

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
                // The PURE READ-BACKS of an invocation object belong in the same list, and this is what
                // MEASURED wrong before them: SPEC §1 ⟨0.32⟩ requires them carved out of `Exec`, and
                // `classify` does carve them out — but the carve-out only survives as far as the next `.`.
                // `c.get_program().to_str()` walked THROUGH `get_program` to the base `Command` and formed
                // `Command::to_str`, which the whole-type Exec rule then charged; likewise `get_args().len()`
                // /`.collect()`, `get_current_dir().unwrap()` and a `for .. in c.get_envs()`. Each returns a
                // DIFFERENT type (`&OsStr`, `CommandArgs`, `CommandEnvs`, `Option<&Path>`) — the exact
                // hard-type-change shape this guard is for, and `get_argv` (portable_pty's spelling of the
                // same read-back) was already here for the same reason.
                if matches!(
                    m.method.to_string().as_str(),
                    "iter" | "into_iter" | "iter_mut" | "drain" | "as_slice" | "as_mut_slice"
                        | "as_bytes" | "as_str" | "to_vec" | "keys" | "values" | "values_mut"
                        | "chars" | "bytes" | "get_argv" | "into_inner" | "lines"
                        | "get_program" | "get_args" | "get_envs" | "get_current_dir"
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
            // Explicit DEREFERENCE receiver `(*b).method()` — transparent: candor already collapses a
            // smart-pointer/reference binding to its POINTEE (`let b = Box::new(W)` types `b` as `W`, a
            // `&W` param types as `W`), so `*b` has the same resolved type as `b`. Recurse into the operand.
            syn::Expr::Unary(u) if matches!(u.op, syn::UnOp::Deref(_)) => self.resolve_recv_type(&u.expr),
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
                .and_then(|id| {
                    let n = id.to_string();
                    // A collection-of-dyn var (`elem_trait_of`), OR — for a NESTED dispatch container
                    // (`Vec<Option<Box<dyn>>>` / `Option<Vec<Box<dyn>>>`) whose OUTER layer already bound this
                    // var into `trait_vars` (the leaves collapsed by `elem_trait_leaves`) — the same leaves,
                    // so the INNER unwrap (`for d in v` / `if let Some(d) = x`) still dispatches (R46). Sound:
                    // it over-approximates only on non-compiling paths (a truly-single dispatch var is never
                    // iterated or re-unwrapped), and the bounded-CHA / local-trait gates never fabricate.
                    self.elem_trait_of.get(&n).or_else(|| self.trait_vars.get(&n))
                })
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
                path_lits_partial: false, path_lit2: None,
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
            path_lits_partial: false, path_lit2: None,
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
    /// UNRESOLVABLE operand (a std/external value like `String`/`i32`, an opaque return) yields None →
    /// NO edge, NOT a disclosed Unknown — a blanket Unknown here would FLOOD every `format!`/`+`/`concat`
    /// in real code. We never fabricate; only a real LOCAL effectful impl lights up. An impl whose method
    /// body isn't a UNIQUE local def is an honest miss (`resolve_target`'s uniqueness filter), e.g. a
    /// type impl'ing both Display and Debug (two `T::fmt`).
    ///
    /// A DISPATCH-TYPED operand (a generic/`impl Trait`/`dyn` param in `trait_vars`) has no concrete type
    /// here — for the STRINGIFY family only it falls through to `charge_stringify_dispatch` (bounded CHA
    /// over the bound); every other coercion keeps the old resolve-or-skip behaviour unchanged.
    fn charge_coercion(&mut self, operand: &syn::Expr, trait_leaf: &str, method: &str) {
        let Some(ty) = self.resolve_recv_type(operand) else {
            self.charge_stringify_dispatch(operand, trait_leaf, method);
            return;
        };
        self.charge_coercion_ty(&ty, trait_leaf, method);
    }

    /// The concrete half of `charge_coercion`, split out so a NAMED/inline-captured format hole
    /// (`format!("{val}")`) — which names a binding rather than an operand expression — can reuse it.
    fn charge_coercion_ty(&mut self, ty: &str, trait_leaf: &str, method: &str) {
        let ty_leaf = ty.rsplit("::").next().unwrap_or(ty);
        if let Some(impls) = self.trait_impls.get(trait_leaf) {
            if impls.iter().any(|t| t == ty_leaf) {
                self.push_coercion_edge(ty_leaf, method);
                return;
            }
        }
        // NO LOCAL IMPL — the type may belong to a DEPENDENCY, whose `Display for Entry` is in its own
        // chained report under `deplib#Entry::fmt`. `trait_impls` is local-only, so the site emitted nothing
        // at all and an effectful dependency formatter was absorbed silently at every format hole. This is
        // the implicit-stringification vein (SOUNDNESS-VEIN-implicit-stringify.md) on the far side of the
        // scan boundary — closed inside the scan in all four engines, still open across it
        // (SOUNDNESS-VEIN-crossing-the-scan-boundary.md).
        //
        // Emit the call the cross-crate join already understands: a crate-qualified `cr::Type::method`,
        // whose tail2 is exactly the dep report's key. If no chained report covers the crate it resolves to
        // nothing, as today.
        let segs: Vec<&str> = ty.split("::").collect();
        if segs.len() >= 2 && !matches!(segs[0], "crate" | "self" | "super" | "") {
            self.calls.push(Call {
                path: format!("{}::{}::{}", segs[0], ty_leaf, method),
                leaf: method.to_string(), str_arg: None, path_lits_partial: false, path_lit2: None,
                typed: false, method: false, is_macro: false,
            });
        }
    }

    /// IMPLICIT STRINGIFICATION through a DISPATCH-TYPED value — the four-way silent under-report
    /// recorded in `candor-spec/SOUNDNESS-VEIN-implicit-stringify.md` (found on HikariCP by the RQ1
    /// runtime oracle, reproduced in all four engines). `format!("{}", e)` / `println!` / `write!` /
    /// `panic!` / `e.to_string()` where `e: T` under `T: Display` runs `<T as Display>::fmt`. candor
    /// analyses that impl correctly — it just never edged to it from the formatting site, so an
    /// effectful local formatter (a lazily-resolved hostname, a metrics counter, a clock read) was
    /// absorbed SILENTLY at every format site in the crate.
    ///
    /// Resolution is the engine's EXISTING bounded CHA (the `visit_expr_method_call` dispatch route),
    /// applied to the BOUND instead of a concrete receiver — the same shape candor-java resolves for
    /// `LOGGER.warn("{}", bagEntry)` with `T extends IConcurrentBagEntry`: CHA the contract method over
    /// the argument's declared type, edge to the LOCAL overrides, contribute nothing when there are none.
    ///
    /// THREE GATES, so this widens the stringify vein and nothing else:
    ///  - ONLY the formatter family takes this route (`Display`/`Debug`, plus `ToString` as Display's
    ///    blanket alias). The operator / `Deref` / `Index` / `Write`-writer coercions that share
    ///    `charge_coercion` are unchanged — a `T: Add` param still resolves to nothing.
    ///  - The bound must actually LICENSE stringification: the formatter trait itself (`T: Display`,
    ///    `impl Display`, `&dyn Display`) or a LOCAL trait that inherits it as a supertrait
    ///    (`trait Entry: Display` + `T: Entry` — the narrow, precise case). An unrelated bound
    ///    (`T: Store`) resolves to nothing.
    ///  - A bound with NO local implementor contributes NOTHING — no edge and no `Unknown`. A crate that
    ///    formats only std types lights nothing, exactly as the concrete route does for a `String`/`i32`.
    ///
    /// DENYLIST-over-allowlist: the local implementor set is taken WHOLE — we never enumerate which
    /// formatters are "effectful". A pure `impl Display` is edged too and propagates nothing, so what we
    /// forget over-discloses (safe) rather than silently missing (the cardinal sin).
    fn charge_stringify_dispatch(&mut self, operand: &syn::Expr, trait_leaf: &str, method: &str) {
        if !matches!(trait_leaf, "Display" | "Debug") {
            return;
        }
        for bound in self.resolve_recv_traits(operand) {
            self.charge_stringify_bound(&bound, trait_leaf, method);
        }
    }

    /// One bound leaf of a stringified dispatch-typed value → its bounded-CHA edges. See
    /// `charge_stringify_dispatch` for the gates; this is the per-bound resolution.
    fn charge_stringify_bound(&mut self, bound: &str, trait_leaf: &str, method: &str) {
        // Which trait's LOCAL implementors this bound licenses CHA over, and which method on each
        // implementor actually runs. `T: ToString` admits both routes: the blanket `impl<T: Display>
        // ToString` (→ the type's `fmt`) and a hand-written `impl ToString` (→ its own `to_string`).
        let mut sources: Vec<(&str, &str)> = Vec::new();
        if bound == trait_leaf {
            sources.push((bound, method));
        } else if trait_leaf == "Display" && bound == "ToString" {
            sources.push(("Display", "fmt"));
            sources.push(("ToString", "to_string"));
        } else if self.trait_inherits(bound, trait_leaf, 0)
            || (trait_leaf == "Display" && self.trait_inherits(bound, "ToString", 0))
        {
            // A LOCAL trait that INHERITS the formatter — CHA over ITS implementors, which is strictly
            // narrower (and more precise) than the formatter trait's whole local universe.
            sources.push((bound, method));
        }
        // Copy the shared index handle out of `self` so the `&mut self` edge pushes below don't
        // conflict with holding a borrow into it.
        let trait_impls = self.trait_impls;
        for (cha, target_method) in sources {
            match trait_impls.get(cha) {
                // Narrow dispatch → edge to every local implementor's formatter. The 12-impl bound is
                // the same cross-engine one `visit_expr_method_call` uses.
                Some(impls) if impls.len() <= 12 => {
                    for ty in impls {
                        self.push_coercion_edge(ty, target_method);
                    }
                }
                // Too wide to enumerate: honest indeterminacy, exactly as the local-trait CHA route
                // reports it. (NO local impl at all is the `None` arm — nothing, not Unknown: that is
                // the no-flood default for the overwhelmingly common std-only-formatting crate.)
                Some(_) => self.unresolved = true,
                None => {}
            }
        }
    }

    /// Does local trait `tr` have `target` among its (transitive, local) SUPERTRAITS? Lets a
    /// `trait Entry: Display` bound resolve stringification over `Entry`'s implementors. Depth-bounded
    /// like `trait_declares_method`; an external supertrait chain resolves to nothing.
    fn trait_inherits(&self, tr: &str, target: &str, depth: usize) -> bool {
        if depth > 16 {
            return false;
        }
        let Some(lt) = self.local_traits.get(tr) else { return false };
        lt.supertraits.iter().any(|s| s == target || self.trait_inherits(s, target, depth + 1))
    }

    /// A NAMED (`format!("{v}", v = x)`) or INLINE-CAPTURED (`format!("{val}")`) format hole stringifies
    /// a BINDING rather than a positional value arg. Resolve the binding the same two ways the operand
    /// route does — concrete type via `vars`, dispatch bound via `trait_vars` — and charge the same
    /// coercion. (Rust only permits a bare identifier as an inline capture, so `vars`/`trait_vars` is
    /// the complete lookup.) Previously such a hole was skipped outright, which made the now-dominant
    /// `format!("{val}")` spelling a silent miss even for a CONCRETE local `impl Display`.
    fn charge_capture(&mut self, name: &str, trait_leaf: &str, method: &str) {
        if let Some(ty) = self.vars.get(name).cloned() {
            self.charge_coercion_ty(&ty, trait_leaf, method);
            return;
        }
        if let Some(bounds) = self.trait_vars.get(name).cloned() {
            for b in bounds {
                self.charge_stringify_bound(&b, trait_leaf, method);
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
    /// URL is built from a `const` rather than written inline. Used ONLY as a FALLBACK when `positional_str_lit`
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
            let (tr, m) = if hole.debug { ("Debug", "fmt") } else { ("Display", "fmt") };
            let idx = match hole.arg {
                FmtArg::Implicit => {
                    let i = next_positional;
                    next_positional += 1;
                    i
                }
                FmtArg::Index(i) => i,
                // A NAMED / INLINE-CAPTURED hole consumes no positional value arg — but it still
                // stringifies a value: the `name = expr` named arg if the macro supplies one, else the
                // same-named binding it captures (`format!("{val}")`, the dominant modern spelling).
                FmtArg::Named(name) => {
                    match exprs[fmt_pos + 1..].iter().find_map(|e| named_arg_value(e, &name)) {
                        Some(v) => self.charge_coercion(v, tr, m),
                        None => self.charge_capture(&name, tr, m),
                    }
                    continue;
                }
            };
            let Some(arg) = pos_args.get(idx) else { continue };
            self.charge_coercion(arg, tr, m);
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

    /// Bind `name -> ty` in `vars` for the duration of `body`. Thin wrapper over `scoped_binding`.
    fn scoped_var<R>(&mut self, name: &str, ty: Option<String>, body: impl FnOnce(&mut Self) -> R) -> R {
        self.scoped_binding(name, ty.map(Bound::Concrete).unwrap_or(Bound::Unknown), body)
    }

    /// THE ONE BINDER. Introduce `name` for the duration of `body`, then restore everything the outer
    /// scope knew about that name. ⚠️ Every side table here is keyed by BARE NAME and is
    /// function-wide, NOT block-scoped — an entry that survives a shadow answers for the shadow.
    ///
    /// This used to scope three maps (`vars`, `dep_bound_vars`, `trait_quals_by_param`) and the four
    /// dispatch binders hand-rolled a fourth (`trait_vars`) inline. **It looked correct and it was not,
    /// for the reason that makes this class hard to see: the common case is saved by PRECEDENCE rather
    /// than by scoping.** A shadow that resolves to a concrete type writes `vars`, and `vars` is
    /// consulted before `trait_vars`, so the stale dispatch binding is masked. A shadow that resolves to
    /// NOTHING writes nothing — and then `trait_vars` still answers:
    ///
    ///     fn f(s: &dyn Store) { for s in 0..3 { s.go(); } }
    ///
    /// charged `f` with the `Fs` of `impl Store for DbStore`, on a loop variable that is a `u8`. A
    /// FABRICATION on a genuinely pure function — the mirror of the cardinal sin — and the same shape as
    /// candor-swift's `71de627`/`83cd607`/`42093b6`, where a name-keyed flag was not restored at some
    /// scope form. Reproduced in three binder forms (for-loop, adapter closure, indeterminate iterator).
    ///
    /// So the enumeration stops here rather than being maintained at five call sites, and the split
    /// between what is cleared and what is kept is by ROLE, not by a list of names:
    ///
    /// * **RESOLUTION tables are cleared.** Everything below answers "what does this name refer to",
    ///   so a stale entry resolves the shadow to the outer binding's target and FABRICATES its effect.
    ///   Clearing them cannot lose a real reach: inside the body the name IS the shadow, so no use of it
    ///   ever meant the outer binding.
    /// * **HEDGING sets are kept** — `closure_vars` and `fn_typed_vars`. Those two only ever suppress a
    ///   phantom call or raise an honest `Unknown` at an invocation `name()`. Clearing them would let a
    ///   shadowed `name()` be resolved as a call to a free fn of that name, which is the fabrication
    ///   mirror of the bug being fixed — trading a cardinal sin for the other one (standing bar item 1).
    ///   They cost at most a spurious `Unknown`, which is the honest direction.
    ///
    /// `every_name_keyed_table_is_scoped_by_the_one_binder` pins the split so a new side table cannot be
    /// added without deciding which half it is in.
    pub(crate) fn scoped_binding<R>(&mut self, name: &str, bound: Bound, body: impl FnOnce(&mut Self) -> R) -> R {
        // Save + clear every RESOLUTION table for this name.
        let p_vars = self.vars.remove(name);
        let p_traits = self.trait_vars.remove(name);
        // DEPENDENCY PROVENANCE. Keyed by bare name like `vars`, so a shadow inherited the OUTER
        // binding's provenance and its member calls were attributed to a dependency they have nothing to
        // do with:  `let client = deplib::build();` then `for client in local_items() { client.go() }`.
        let p_prov = self.dep_bound_vars.remove(name);
        // The PER-PARAM crate qualification: a shadow must not permanently overwrite the parameter's
        // crate, or the rest of the function resolves its receiver through the shadow's dependency.
        let p_qual = self.trait_quals_by_param.remove(name);
        // Collection/tuple element types — a shadow inheriting these resolves `name[0]` / `for x in name`
        // / `let (a, _) = name` to the OUTER collection's element.
        let p_elem = self.elem_of.remove(name);
        let p_elem_tr = self.elem_trait_of.remove(name);
        let p_tuple = self.tuple_of.remove(name);
        let p_tuple_tr = self.tuple_trait_of.remove(name);
        // A free-fn alias: `name()` inside the body would call the OUTER alias's target and inherit its
        // whole transitive effect chain.
        let p_alias = self.fn_alias.remove(name);
        // A resolved host literal: a shadow inheriting it attributes the outer binding's endpoint to
        // this one's call, which lands in the gate's `hosts` surface.
        let p_str = self.str_locals.remove(name);

        match bound {
            Bound::Concrete(t) => {
                self.vars.insert(name.to_string(), t);
            }
            Bound::Traits(leaves) => {
                self.trait_vars.insert(name.to_string(), leaves);
            }
            Bound::Unknown => {}
        }
        let r = body(self);

        let restore = |m: &mut HashMap<String, String>, p: Option<String>| match p {
            Some(v) => { m.insert(name.to_string(), v); }
            None => { m.remove(name); }
        };
        restore(&mut self.vars, p_vars);
        restore(&mut self.dep_bound_vars, p_prov);
        restore(&mut self.fn_alias, p_alias);
        restore(&mut self.str_locals, p_str);
        restore(&mut self.elem_of, p_elem);
        match p_traits {
            Some(v) => { self.trait_vars.insert(name.to_string(), v); }
            None => { self.trait_vars.remove(name); }
        }
        match p_qual {
            Some(v) => { self.trait_quals_by_param.insert(name.to_string(), v); }
            None => { self.trait_quals_by_param.remove(name); }
        }
        match p_elem_tr {
            Some(v) => { self.elem_trait_of.insert(name.to_string(), v); }
            None => { self.elem_trait_of.remove(name); }
        }
        match p_tuple {
            Some(v) => { self.tuple_of.insert(name.to_string(), v); }
            None => { self.tuple_of.remove(name); }
        }
        match p_tuple_tr {
            Some(v) => { self.tuple_trait_of.insert(name.to_string(), v); }
            None => { self.tuple_trait_of.remove(name); }
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
    /// A `fn`/`impl` NESTED INSIDE a body has its OWN signature, but its calls are still attributed to
    /// the enclosing unit (`scan_items` only walks top-level items, so nobody else records them). Its
    /// params therefore SHADOW the outer ones under the same names, and `dyn_sig_traits` — read off the
    /// OUTER signature — must not follow the walk in.
    ///
    /// Measured, and the reason this exists: value-bag's `internal_visit(v: &dyn Serialize, …)` declares a
    /// nested `impl Serializer` whose `serialize_some<T: Serialize>(self, v: &T)` calls `v.serialize(self)`.
    /// That inner `v` is a caller-monomorphized GENERIC, but it inherited the outer `v`'s `dyn`-ness, so
    /// the imported-trait CHA (R4) fired on it and put 17 fresh `Unknown`s on value-bag through
    /// `ValueBag::serialize`. Clearing the set for the nested walk is exactly the erasure carve-out doing
    /// what it says. Only this rung reads the set, so nothing else changes.
    /// `trait_quals` is scoped with it, for the same reason and one step further along: it maps a trait
    /// LEAF to the crate-qualified path THIS signature wrote. A nested item's same-named receiver would
    /// otherwise inherit the OUTER signature's crate and form a dep key naming the wrong dependency —
    /// the leaf-collision fabrication, arriving by nesting instead of by two params.
    /// **A SHADOW ALONE IS THE MIRROR SIN**, and that is what the bound map was: `std::mem::take` left it
    /// EMPTY for the nested walk and never installed the NESTED signature's own bounds, so
    /// `fn outer() { fn inner<T: Doer>(d: T) { let x: T = d; x.go() } }` resolved `T` to nothing and
    /// `outer` was ABSENT from the report — a purity claim over a call that performs the effect. The
    /// identical `let x: Box<dyn Doer>` inside the same nested `fn` resolves, which is what makes the
    /// miss attributable to the emptied map and not to the position being dead. candor-swift's sibling
    /// (`83cd607`) re-applies the nested signature's own opacity for exactly this reason; this is its
    /// rust half, through the same `generic_bounds_of` the top-level construction uses rather than a
    /// second copy of the rule.
    ///
    /// REPLACE, not merge — and the honest version of that claim is narrower than it first looked. The
    /// two are indistinguishable on anything rustc ACCEPTS, because a nested item may not name the
    /// enclosing fn's generics at all (E0401), so the only keys they disagree about are names no legal
    /// nested body can mention; the merge mutant duly passes every other test in this suite. REPLACE is
    /// chosen because candor-scan analyses crates WITHOUT building them and therefore walks bodies rustc
    /// never accepted — `#[cfg]`-gated, macro-shaped, mid-edit — where an inherited bound DOES reach a
    /// same-named local type and fabricates an effect on it. Pinned by
    /// `an_outer_bound_does_not_reach_a_nested_item_that_never_declared_it`.
    fn visit_item_fn(&mut self, node: &'ast syn::ItemFn) {
        let outer = std::mem::take(&mut self.dyn_sig_traits);
        let outer_g = std::mem::replace(
            &mut self.generic_bounds,
            crate::lang::generic_bounds_of(&node.sig),
        );
        let outer_q = std::mem::take(&mut self.trait_quals);
        // …and the PER-PARAM map, which is now the FIRST source consulted. Scoping the two leaf-keyed maps
        // but not this one left a nested item reading the outer signature's crate for a same-named
        // receiver — the collision arriving by nesting, which is what scoping the others was meant to stop.
        let outer_p = std::mem::take(&mut self.trait_quals_by_param);
        syn::visit::visit_item_fn(self, node);
        self.dyn_sig_traits = outer;
        self.generic_bounds = outer_g;
        self.trait_quals = outer_q;
        self.trait_quals_by_param = outer_p;
    }
    /// The three ERASURE/PROVENANCE maps stay CLEARED for a nested item rather than re-installed from
    /// its own signature, and this is measured, not an oversight: a nested item's PARAMETERS are never
    /// typed at all — `fn outer() { fn inner(d: &dyn Doer) { d.go() } }` is silent in exactly the same
    /// way its `<T: Doer>` twin is, so the position is dead for every spelling and those maps would have
    /// nothing to bind to. Re-installing them would widen R4's and R6's fabrication surface for a gain
    /// no fixture can show. The nested-parameter position is a POSITION-level gap of its own (pinned as
    /// a residual test); when it is closed, this is the second half to close with it.
    fn visit_item_impl(&mut self, node: &'ast syn::ItemImpl) {
        let outer = std::mem::take(&mut self.dyn_sig_traits);
        // The impl BLOCK's own generics (`impl<T: Doer> Wrap<T>`) are the outer scope for every method
        // in it; `visit_impl_item_fn` layers each method's own on top.
        let outer_g = std::mem::replace(
            &mut self.generic_bounds,
            crate::lang::generic_bounds_of_generics(&node.generics),
        );
        let outer_q = std::mem::take(&mut self.trait_quals);
        let outer_p = std::mem::take(&mut self.trait_quals_by_param);
        syn::visit::visit_item_impl(self, node);
        self.dyn_sig_traits = outer;
        self.generic_bounds = outer_g;
        self.trait_quals = outer_q;
        self.trait_quals_by_param = outer_p;
    }
    /// A METHOD of a nested `impl` carries its own generics (`fn m<T: Doer>(&self, d: T)`), which the
    /// impl block's `Generics` do not contain — so without this the impl form of the shadow above stays
    /// silent even with `visit_item_impl` fixed. EXTEND rather than replace: unlike a nested `fn`, a
    /// method genuinely may name its impl block's generics, and rust forbids a method from shadowing
    /// one, so the two sets are disjoint and the union is exactly what is in scope.
    ///
    /// Only ever reached through a nested `impl` — this collector walks one unit's BODY, and a
    /// top-level impl method gets its own collector with its own signature (see `collect_calls`).
    fn visit_impl_item_fn(&mut self, node: &'ast syn::ImplItemFn) {
        let mut inner = self.generic_bounds.clone();
        inner.extend(crate::lang::generic_bounds_of(&node.sig));
        let outer_g = std::mem::replace(&mut self.generic_bounds, inner);
        syn::visit::visit_impl_item_fn(self, node);
        self.generic_bounds = outer_g;
    }
    /// A BLOCK scopes the per-param crate-qualification map. `visit_local` writes a trait-typed local's
    /// own qualification under the binding NAME so it correctly shadows a parameter — but a shadow inside
    /// a nested block was never undone, so the parameter's crate stayed overwritten for the rest of the
    /// function and its later calls resolved through the wrong dependency (measured: `a.go()` after a
    /// block-scoped `let a: &dyn alpha::Handler` lost beta's Net entirely).
    ///
    /// Only this map is scoped here. `vars` deliberately keeps its existing flow-insensitive behaviour —
    /// changing that is a much larger question and not one this fix should smuggle in.
    fn visit_block(&mut self, node: &'ast syn::Block) {
        let saved = self.trait_quals_by_param.clone();
        syn::visit::visit_block(self, node);
        self.trait_quals_by_param = saved;
    }
    /// A `use` written INSIDE a body (`fn f() { use deplib::CFG; .. }`). Pass A's `uses` map is built from
    /// FILE-level items, so this spelling contributed nothing and every name it imported looked
    /// crate-local — the forcing/provenance sites below then had no way to learn the name's origin. Record
    /// it in `local_uses`, which `use_target` consults FIRST.
    fn visit_item_use(&mut self, node: &'ast syn::ItemUse) {
        crate::lang::collect_use(&node.tree, String::new(), &mut self.local_uses);
        syn::visit::visit_item_use(self, node);
    }
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
                        let str_arg = positional_str_lit(&node.args, 0)
                            // ⟨0.29⟩ …or argument 1 for the two Net verbs whose locator lives there
                            // (`request(Method, url)`, `send_to(buf, addr)`) — see `is_net_host_arg1`.
                            // Gated on the VERB, never a blanket "try the next argument": a blanket
                            // fallback is the literal-anywhere hazard this rung removed.
                            .or_else(|| if candor_classify::is_net_host_arg1(&leaf) {
                                positional_str_lit(&node.args, 1)
                            } else { None })
                            .or_else(|| self.resolve_host_arg(&node.args));
                        // (R53 UFCS-dispatch edge REVERTED after code review: pushing a typed `T::method` edge
                        // from a UFCS `Trait::method(&t)` / `<T as Trait>::method` could resolve to T's
                        // *inherent* `method` when the call actually runs the trait method — candor keys both
                        // an inherent `impl T { fn m }` and a trait `impl Trait for T { fn m }` as `T::m`, so
                        // for a T that uses the trait's DEFAULT and *also* has an inherent `m`, the edge
                        // FABRICATED the inherent's effect. The default case is already handled by the bare
                        // `Trait::method` edge below resolving to the default body; the override case is left
                        // an honest under-report. The `&self`-only filter on `LocalTrait.methods` is kept — it
                        // is independently sound and sharpens the R36 trait-default CHA.)
                        // ⟨0.29⟩ A TWO-PATH Fs OPERATION MUST HAVE A LITERAL IN *BOTH* POSITIONS.
                        // `fs::copy("/safe", user_path)` has one at position 0 and still writes somewhere
                        // nobody can see, so reading position 0 alone would leave the same hole one
                        // argument along. Recorded here, where the argument list is in hand.
                        let two_path = candor_classify::is_fs_path_arg(&leaf)
                            && candor_classify::fs_path_arity(&leaf) == 2;
                        let path_lit2 = if two_path { positional_str_lit(&node.args, 1) } else { None };
                        let path_lits_partial = two_path && path_lit2.is_none();
                        self.calls.push(Call { path, leaf, str_arg, typed: false, method,
                                               is_macro: false, path_lits_partial, path_lit2 });
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
        let str_arg = positional_str_lit(&node.args, 0)
                            // ⟨0.29⟩ …or argument 1 for the two Net verbs whose locator lives there
                            // (`request(Method, url)`, `send_to(buf, addr)`) — see `is_net_host_arg1`.
                            // Gated on the VERB, never a blanket "try the next argument": a blanket
                            // fallback is the literal-anywhere hazard this rung removed.
                            .or_else(|| if candor_classify::is_net_host_arg1(&leaf) {
                                positional_str_lit(&node.args, 1)
                            } else { None })
                            .or_else(|| self.resolve_host_arg(&node.args));
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
        self.calls.push(Call { path: leaf.clone(), leaf: leaf.clone(), str_arg: str_arg.clone(), typed: false, method: true, is_macro: false, path_lits_partial: false, path_lit2: None });
        // COULD-NOT-FORM-A-KEY (DEP-RECEIVER-TYPING-DESIGN.md half 1). The receiver is a local bound from
        // a cross-crate call whose return type we never learned — `let c = deplib::build(); c.fetch()`.
        // No key is formed, so no question is asked of the chained report, so its silence licenses
        // nothing; dropping here is a CONFIDENT purity claim about a call we never looked up. Emit a
        // marker so the call loop can disclose `Unknown` instead. Requires the CONJUNCTION — untyped AND
        // dep-provenance — and is skipped the moment either the concrete type or a trait bound resolves,
        // because then a real key exists and this would only add noise beside it.
        //
        // BOTH SPELLINGS, and the second one was the hole. `let c = deplib::build(); c.fetch()` routes
        // through `dep_bound_vars`, which only a `let` ever writes — so `deplib::build().fetch()`, the
        // very same call with the binding elided, matched no branch and read SILENT-PURE. That is not a
        // precision gap in an un-attempted position: it is the shipped guard failing to cover its own
        // ruling that a key which could not be formed must never read pure (conformance PART 21, whose
        // rust fixture binds the result).
        let dep_recv_callee: Option<String> = match &*node.receiver {
            // The BOUND form: a local whose provenance a `let` recorded.
            syn::Expr::Path(rp) => rp
                .path
                .get_ident()
                .map(|i| i.to_string())
                .and_then(|n| self.dep_bound_vars.get(&n).cloned()),
            // The UNBOUND form: the factory call IS the receiver. Same provenance test as `visit_local`'s
            // (a multi-segment callee path, crate-root checked at CONSUMPTION against the manifest's
            // declared deps) so the two spellings cannot drift apart again.
            _ => match peel_recv(&node.receiver) {
                syn::Expr::Call(c) => match &*c.func {
                    syn::Expr::Path(p) => {
                        let full = expand(&path_to_string(&p.path), self.uses);
                        (full.contains("::") && !full.starts_with("::")).then_some(full)
                    }
                    _ => None,
                },
                _ => None,
            },
        };
        if let Some(callee_path) = dep_recv_callee {
            if self.resolve_recv_type(&node.receiver).is_none()
                && self.resolve_recv_traits(&node.receiver).is_empty()
            {
                // Angle-bracket marker segment: cannot collide with a real path (the same device
                // as `<drop>`/`<construct>`), so it can never reach local resolution or the
                // classifier. Bounded at consumption in scan.rs: join, disclose, `continue`.
                //
                // `<crate>::<untyped>::<the rest of the callee path>::<method>`. The crate root
                // alone drives half 1's disclosure; the WHOLE callee path is what half 2 looks up
                // in the dep's published `typeSurface.returns`, and it has to be the whole path
                // because that map is keyed by the dependency's MODULE-QUALIFIED fn qual. The
                // consumer splits it back off with `rsplit_once` — a method leaf is one segment,
                // a callee path is not, so splitting from the FRONT (as the reverted attempt did)
                // truncates every non-root factory to its first module segment.
                let (root, rest) = callee_path.split_once("::").unwrap_or((callee_path.as_str(), ""));
                self.calls.push(Call {
                    path: format!("{root}::<untyped>::{rest}::{leaf}"),
                    leaf: leaf.clone(),
                    str_arg: None,
                    path_lits_partial: false, path_lit2: None,
                    typed: false,
                    method: false,
                    is_macro: false,
                });
            }
        }
        // Typed call: if the receiver's type resolves, form `Type::method` so the existing per-crate
        // method rules (reqwest/sqlx/redis/…) — unreachable from a bare method name — can fire. This is
        // the method-dispatch frontier: light, local type inference, no compiler.
        //
        // EXTERNAL types only. The external-crate rules are verb-precise (`ends_with("::execute")`), so
        // they're safe to apply to an inferred method call. MOST std rules are coarse PREFIX matches
        // (`std::fs::`) written for free-function/constructor calls — applied to arbitrary method calls
        // they mis-fire on the pure DATA types under the same prefix (`Metadata::len`, `DirEntry::path`).
        // So std/core/alloc receivers are skipped by default, with two NAMED exceptions below where the
        // classifier's rule is precise enough to answer a method call.
        //
        // The default was ONCE the whole story, and it hid a cardinal sin: with no exception for the
        // handle types, `fn run(cmd: &mut Command) { cmd.spawn(); }` formed no path at all, reached no
        // rule, and certified PURE under `deny Exec` — a silent false all-clear on a real subprocess
        // spawn (`cc`'s `command_helpers::spawn` in the wild). Receiver typing was never the problem:
        // the same function over a `tokio::process::Command` was caught, because tokio is not std. "An
        // honest miss beats a wrong effect" was the right instinct for `std::fs::Metadata` and the wrong
        // one for `std::fs::File`, and it was applied to both by a test on the crate ROOT.
        if let Some(ty) = self.resolve_recv_type(&node.receiver) {
            let cr = ty.split("::").next().unwrap_or("");
            // EXCEPTION 1 to the std exclusion: `std::path::Path`/`PathBuf` receivers route through —
            // the classifier has a VERB-PRECISE stat-family rule for them (metadata/read_dir/exists/…
            // → Fs; the pure join/file_name surface returns None), so the coarse-prefix mis-fire risk
            // doesn't apply. Without this an entire directory walker reads as pure (gix-dir: zero Fs).
            let std_path_recv = ty == "std::path::Path" || ty == "std::path::PathBuf";
            // EXCEPTION 2: the std I/O HANDLE types (`Command`/`Child`, the TCP/UDP/Unix sockets,
            // `File`) — an open descriptor whose every method is either the effect or a pure read-back
            // the classifier already carves out. The membership rule, and what is deliberately left
            // out (`OpenOptions`/`DirBuilder`/`ReadDir`, which have pure setters), are documented on
            // `STD_EFFECT_HANDLES`. Keyed on the RESOLVED, use-expanded TYPE PATH, never a leaf: a
            // crate's own `struct Command` expands to a bare `Command` (or `crate::…`) and cannot
            // collide with `std::process::Command`, so the local-shadowing case the blanket exclusion
            // also happened to cover stays covered — that is what the shadow controls pin.
            //
            // HALF ONE OF TWO. Emitting the path is not enough: `resolvable` in scan.rs keys local
            // resolution on `tail2`, which DROPS the `std::process` qualifier this exception depends
            // on, so a crate defining its own `Command` captured the std call there instead and
            // suppressed the classifier. Both halves are needed for either to close the hole; see the
            // `c.typed` branch of `resolvable`.
            let std_handle_recv = candor_classify::is_std_effect_handle(&ty);
            // `.clone()` resolves to NO typed `Type::clone`: it is conventionally pure, and through the
            // smart-pointer deref-peel (type_path) an `Arc<T>`/`Rc<T>` receiver types as `T`, so
            // `arc.clone()` would form `T::clone` and FABRICATE — but `arc.clone()` calls the pointer's
            // own `Arc::clone` (a pure refcount bump), NEVER `T::clone`. An effectful `T::clone` is a rare
            // anti-pattern, so skipping the typed clone resolution is the safe choice (no fabrication).
            if (!matches!(cr, "std" | "core" | "alloc") || std_path_recv || std_handle_recv)
                && leaf != "clone"
            {
                let path = format!("{ty}::{leaf}");
                self.calls.push(Call { path, leaf: leaf.clone(), str_arg, typed: true, method: true, is_macro: false, path_lits_partial: false, path_lit2: None });
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
                let Some(lt) = self.local_traits.get(&tr) else {
                    // EXTERNAL trait dispatch (`x.publish()`, `x: &dyn dep::OutboundChannel`). Formerly a
                    // documented miss (dropped → pure). If the trait resolves via `use` to a DEPENDENCY-
                    // qualified path (not std/core/alloc), emit a crate-qualified Call so a CANDOR_DEPS chain
                    // resolves it against the trait-CHA `interfaceUnion` entry the dep exposes (WORKSPACE-
                    // CHAINING-DESIGN.md). Precise path (the actual trait method) → no fabrication; unresolved
                    // to pure/invisible exactly as before when the dep isn't chained.
                    // R6 — a FULLY-QUALIFIED receiver (`fn f(h: &dyn deplib::Handler)`) has no `use` to
                    // expand through, and `bound_leaves` already threw the qualifier away, so `expand`
                    // handed back the bare leaf, the `contains("::")` test failed and the site emitted
                    // NOTHING: no dep key, no CHA, silent-pure — while the IMPORTED spelling of the very
                    // same receiver resolves. `trait_quals` keeps the path the signature actually wrote,
                    // so both spellings form the same key. Still run it through `expand`: the head segment
                    // can itself be a `use` alias (`use foo::bar as deplib`). `crate`/`self`/`super`
                    // spellings are never recorded (see `sig_trait_quals`) — those are ours.
                    // An EMPTY value is the tombstone for a leaf this signature bound to two different
                    // crates (see `quals_from_bounds`): treat it as absent and fall back to the file's
                    // `use` map, which the language guarantees cannot bind one leaf to two crates.
                    // The RECEIVER's own declared bound wins. Only when the receiver is not a plain
                    // parameter do we fall back to the leaf-keyed map, whose empty value is the tombstone
                    // for a leaf this signature bound to two different crates.
                    let recv_param = match &*node.receiver {
                        syn::Expr::Path(p) => p.path.get_ident().map(|i| i.to_string()),
                        syn::Expr::Reference(r) => match &*r.expr {
                            syn::Expr::Path(p) => p.path.get_ident().map(|i| i.to_string()),
                            _ => None,
                        },
                        _ => None,
                    };
                    let per_param = recv_param
                        .as_ref()
                        .and_then(|n| self.trait_quals_by_param.get(n))
                        .and_then(|m| m.get(&tr))
                        .map(|q| q.as_str())
                        .filter(|q| !q.is_empty());   // a tombstone is "unknown", never a path
                    let written = per_param.or_else(|| {
                        self.trait_quals.get(&tr).map(|q| q.as_str()).filter(|q| !q.is_empty())
                    }).unwrap_or(tr.as_str());
                    let full = crate::lang::expand(written, self.uses);
                    let root = full.split("::").next().unwrap_or("");
                    if full.contains("::") && !crate::lang::is_std_trait_root(root) {
                        self.calls.push(Call {
                            path: format!("{full}::{leaf}"),
                            leaf: leaf.clone(),
                            str_arg: str_arg.clone(),
                            path_lits_partial: false, path_lit2: None,
                            typed: true,
                            method: true,
                            is_macro: false,
                        });
                        // R4 — the trait crossed the SCAN BOUNDARY but the impl that runs did NOT.
                        // `use deplib::Handler; fn run(h: &dyn Handler) { h.go() }` with
                        // `impl Handler for MyH` declared HERE recorded no dispatch at all: `local_traits`
                        // is built only from local `ItemTrait` nodes, so CHA never fired and `run` read
                        // silent-pure — while the witness that actually runs is a project unit candor
                        // analysed correctly in the same report, and the SINGLE-CRATE control gets it right
                        // (candor-spec SOUNDNESS-VEIN-crossing-the-scan-boundary.md; candor-swift's
                        // imported-protocol CHA `eae2de2` is the sibling, and hit the identical trap).
                        //
                        // TWO CARVE-OUTS, and they are the whole reason this is safe. Both are measured,
                        // not assumed; each is pinned by a test.
                        //
                        // (1) PROVENANCE (`is_dependency_crate_root`): only a trait resolving to a genuine
                        //     PROJECT-DEPENDENCY crate root takes this route. std/core/alloc are out, and
                        //     a PRELUDE trait (`Iterator` and friends) needs no `use` at all, so `expand`
                        //     leaves it unqualified and the `contains("::")` gate keeps it out — which is
                        //     what the "external-trait local impl must not resolve (fabrication)" guard in
                        //     `dispatch_typed_receivers_resolve_via_local_impls_or_read_unknown` still
                        //     pins, unchanged. A blanket version got this wrong: CHA-ing `Iterator` over a
                        //     local `impl Iterator for RowIter` charges every `.next()` in the crate with
                        //     RowIter's effects (execution-verified), and its >12-impl arm alone put 30
                        //     fresh `Unknown`s on serde_json. `self`/`crate`/`super` are rejected HERE and
                        //     not on the emission above, deliberately: a `pub use self::error::Error`
                        //     re-export makes std's `Error` look dependency-qualified, and that alone cost
                        //     17 fresh Unknowns on value-bag — see `is_dependency_crate_root`.
                        //
                        // (2) ERASURE (`dyn_sig_traits`): the receiver must be spelled `dyn`. Provenance
                        //     alone is NOT enough, and the queue's resolution-1 as written would have
                        //     shipped a flood: `serde::Serialize`/`serde::Serializer` ARE dependency
                        //     traits, so they pass (1) — and CHA-ing serde_json's own five `impl
                        //     Serializer` types onto its GENERIC entry points put 32 fresh `Unknown`s on
                        //     `to_string`/`to_vec`/`to_writer`, inherited through edges to witnesses a
                        //     caller's own `Serializer` never runs. A `dyn` receiver is erased and the
                        //     local impls are its candidate witnesses; a `T: Trait` bound / `impl Trait`
                        //     param is monomorphized BY THE CALLER, so they are not. Requiring erasure
                        //     takes serde_json to ZERO fresh Unknowns. The bound/`impl Trait` forms of an
                        //     IMPORTED trait therefore stay a documented residual, not a guess.
                        //
                        // ADDITIVE and PRECISE-OR-NOTHING, the swift template: edges only, and ONLY when
                        // the local impl set is narrow (≤12, the cross-engine bound). `self.unresolved` is
                        // deliberately NOT set on the wide/absent arms — the local impl set is a LOWER
                        // bound on the true one (a third crate may implement the trait too), so a wide one
                        // stays the documented miss it already was rather than flooding Unknown over every
                        // externally-typed receiver. The call ALSO keeps its crate-qualified shape above,
                        // so a chained dep report still contributes its own impls via `interfaceUnion`.
                        if let Some(impls) = self.trait_impls.get(&tr).filter(|_| {
                            crate::lang::is_dependency_crate_root(root)
                                && self.dyn_sig_traits.contains(&tr)
                        }) {
                            if impls.len() <= 12 {
                                for ty in impls {
                                    self.calls.push(Call {
                                        path: format!("{ty}::{leaf}"),
                                        leaf: leaf.clone(),
                                        str_arg: str_arg.clone(),
                                        path_lits_partial: false, path_lit2: None,
                                        typed: true,
                                        method: true,
                                        is_macro: false,
                                    });
                                }
                            }
                        }
                    }
                    continue;
                };
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
                                path_lits_partial: false, path_lit2: None,
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
                // Option/Result synchronous callback-invokers: the combinator calls the callback on the
                // unwrapped value in-line (single element param, like the iterator adapters). Adding them
                // lets an OPAQUE callable passed directly (`o.and_then(cb)`, `o.map_or(d, cb)`) disclose
                // Unknown via the opaque-arg guard above, while an inline closure keeps its analyzed body.
                | "and_then" | "map_or" | "map_or_else" | "unwrap_or_else" | "get_or_insert_with"
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
                // An OPAQUE callable passed BY VALUE to a synchronous callback-invoker (`xs.iter()
                // .for_each(cb)`, `opt.map(cb)`) — where `cb` is a generic/`impl`/`dyn` `Fn` param or an
                // otherwise-unresolvable fn-typed local — is invoked by the adapter on a body the scan
                // can't see: honest `Unknown`, exactly as the DIRECT-call form `cb()` (line ~721) and the
                // CLOSURE-WRAPPED form `for_each(|x| cb(x))` already are. The direct-pass form was silently
                // dropped as pure — a §4 under-report (the Rust arm of the four-way sync-callback parity
                // fix; candor-java c755acd). `expr_is_fn_typed` peels `&`/paren/group so `for_each(&cb)`
                // and `let g = cb; for_each(g)` are covered too. Checked BEFORE the named-fn edge below so an
                // opaque local never falls through to a phantom free-fn resolution.
                if self.expr_is_fn_typed(a) {
                    self.unresolved = true;
                    continue;
                }
                if let syn::Expr::Path(p) = a {
                    let is_local = p.path.get_ident().is_some_and(|i| {
                        let n = i.to_string();
                        self.vars.contains_key(&n) || self.closure_vars.contains(&n) || self.fn_typed_vars.contains(&n)
                    });
                    if p.qself.is_none() && !is_local {
                        let name = path_to_string(&p.path);
                        let path = self.fn_alias.get(&name).cloned().unwrap_or_else(|| expand(&name, self.uses));
                        let leaf2 = path.rsplit("::").next().unwrap_or(&path).to_string();
                        self.calls.push(Call { path, leaf: leaf2, str_arg: None, typed: false, method: false, is_macro: false, path_lits_partial: false, path_lit2: None });
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
                            self.scoped_binding(&name, Bound::Traits(elem_leaves.clone()), |s| s.visit_expr(&cl.body));
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
                    self.scoped_binding(&binding, Bound::Traits(leaves), |s| s.visit_block(&node.then_branch));
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
                    self.scoped_binding(&binding, Bound::Traits(leaves.clone()), |s| {
                        if let Some((_, guard)) = &arm.guard {
                            s.visit_expr(guard);
                        }
                        s.visit_expr(&arm.body);
                    });
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
                self.scoped_binding(&name, Bound::Traits(leaves), |s| s.visit_block(&node.body));
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
                    || self.trait_vars.contains_key(&name)
                    // …and every OTHER binding form. The five side-tables above only hold a binding whose
                    // TYPE was recoverable, so `let C = "aa";` — a `let` whose initializer types to nothing
                    // — left `C` looking like the imported static and the forcing edge fired on a local
                    // string. That was harmless while only a QUALIFIED path could force (a shadow is
                    // spelled bare); adding the bare `use` spelling made it live, and the control caught it.
                    // `bound_names` is every ident in a binding POSITION anywhere in this body, so the
                    // shadow test no longer depends on type inference having succeeded.
                    || self.bound_names.contains(&name);
                // A DEPENDENCY's lazy static, mentioned qualified (`deplib::CFG`). Forcing it runs the
                // dep's initializer, which a chained report records as `<lazy>::…NAME` — but only LOCAL
                // statics are in `lazy_statics`, so nothing was emitted and the consumer read pure even
                // with the dep chained (candor-spec SOUNDNESS-VEIN-initializer-edge.md — rust's side of
                // the boundary java and ts needed). The marker path is consumed ONLY by the cross-crate
                // join in scan.rs, which skips it everywhere else, so it can never become a local edge:
                // an earlier prototype without that guard added a spurious callgraph node because it fired
                // on every qualified path expression.
                //
                // BOTH SPELLINGS. `deplib::CFG` names the crate in the expression; `use deplib::CFG; CFG`
                // names it in the import and leaves a ONE-segment path behind. Only the first was handled,
                // so the idiomatic import made the identical read silent-pure — and PART 19's rust fixture
                // happens to use the qualified spelling, which is why it never saw it. The `use` route also
                // recovers the dependency's own MODULE path (`use deplib::cfg::CFG` → `<lazy>::cfg::CFG`),
                // which is the key its report actually carries when the static is not crate-root.
                //
                // The dependency's OWN module path is part of the key. Its report writes
                // `<lazy>::cfg::MODC` for a static declared in its `cfg` module (`lazy_qual`), so the
                // crate-root spelling alone answers only for a crate-root static — `deplib::cfg::MODC`
                // asked `<lazy>::MODC` and got silence. Ask both: the full path after the crate root, and
                // the leaf alone (which is what a dep that RE-EXPORTS the static at its root needs). Each
                // is speculative and inert unless the chained report carries it, so asking both costs
                // nothing but answers both shapes.
                let dep_lazy_keys: Vec<(String, String)> = {
                    let written: Option<(String, String)> = if node.path.segments.len() >= 2 {
                        let segs: Vec<String> =
                            node.path.segments.iter().map(|s| s.ident.to_string()).collect();
                        Some((segs[0].clone(), segs[1..].join("::")))
                    } else {
                        // `use deplib::CFG; CFG` — the crate is named in the IMPORT, not the expression.
                        self.use_target(&name)
                            .filter(|f| f.contains("::"))
                            .cloned()
                            .and_then(|full| full.split_once("::").map(|(c, r)| (c.to_string(), r.to_string())))
                    };
                    match written {
                        Some((cr, rest)) if !rest.is_empty() => {
                            let mut v = vec![(cr.clone(), rest.clone())];
                            if rest != name {
                                v.push((cr, name.clone()));
                            }
                            v
                        }
                        _ => Vec::new(),
                    }
                };
                for (cr, key) in dep_lazy_keys {
                    if !locally_bound && !self.lazy_statics.contains(&name)
                        && self.forced_lazies.insert(format!("{cr}\u{0}{key}"))
                    {
                        self.calls.push(Call {
                            path: format!("{cr}::{LAZY_UNIT_PREFIX}::{key}"),
                            leaf: name.clone(), str_arg: None,
                            typed: false, method: false, is_macro: false, path_lits_partial: false, path_lit2: None,
                        });
                        // DROP GLUE across the boundary. Naming a dependency's type as a value (`let _g =
                        // deplib::Guard;`) binds it here, so its `Drop::drop` runs at scope exit — an
                        // implicit edge the syntactic call graph never sees. `drop_types` is built from
                        // LOCAL `impl Drop` blocks only, so the dependency case emitted nothing and the
                        // scope read pure while the same code in one crate reads `Fs`
                        // (SOUNDNESS-VEIN-crossing-the-scan-boundary.md). Emit the shape the cross-crate
                        // join already understands — `cr::Type::drop`, whose tail2 is exactly the dep
                        // report's key. Speculative but self-limiting: a dep report only carries
                        // `Type::drop` when that drop is EFFECTFUL (pure units are omitted), so a type with
                        // no Drop, or no chained report, resolves to nothing.
                        self.calls.push(Call {
                            path: format!("{cr}::{DROP_MARKER}::{key}"),
                            leaf: "drop".to_string(), str_arg: None,
                            typed: false, method: false, is_macro: false, path_lits_partial: false, path_lit2: None,
                        });
                    }
                }
                if !locally_bound && self.lazy_statics.contains(&name) {
                    // path has `::` so it resolves via the tail2 route in `resolve_target`. The module
                    // path sits INSIDE the prefix precisely so tail2 (`<mod>::<NAME>`) discriminates: with
                    // the old bare `<lazy>::NAME`, two modules each declaring `CFG` produced two units with
                    // the same tail2, `resolve_target`'s uniqueness filter rejected both, and every forcing
                    // site went SILENT-PURE (candor-spec SOUNDNESS-VEIN-global-unit-identity.md).
                    //
                    // …and the READER has to be qualified the same way, which is the half `5447eba` left
                    // open: `<lazy>::<MY module>::NAME` is the right key only when the static is declared
                    // in the reader's own module. `fn outside() { let _ = *m::INNER; }` built
                    // `<lazy>::INNER`, whose tail2 matches no unit, so a module-scoped lazy static read
                    // from OUTSIDE its module was silent-pure while `m::inside` was charged correctly.
                    // Take the module the SPELLING names (`m::INNER`, or `use m::INNER; INNER`) and keep
                    // the reader's-own-module key beside it: they are two candidate keys, each filtered by
                    // `resolve_target`'s uniqueness rule, so this can only ADD the edge that was missing.
                    let owned: Vec<String> =
                        node.path.segments.iter().map(|s| s.ident.to_string()).collect();
                    let segs: Vec<&str> = owned.iter().map(String::as_str).collect();
                    let mut modpaths = vec![self.modpath.clone()];
                    if let Some(named) = self.named_lazy_modpath(&segs) {
                        if named != self.modpath {
                            modpaths.push(named);
                        }
                    }
                    for mp in modpaths {
                        let qual = lazy_qual(&mp, &name);
                        // Not a macro/typed/method.
                        if self.forced_lazies.insert(qual.clone()) {
                            self.calls.push(Call { path: qual, leaf: name.clone(), str_arg: None, typed: false, method: false, is_macro: false, path_lits_partial: false, path_lit2: None });
                        }
                    }
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
                // Any REBIND of the name drops stale dependency provenance, exactly as it drops a stale
                // concrete type. Without this an annotated rebind kept the old binding's provenance.
                self.dep_bound_vars.remove(&id.ident.to_string());
                // A trait-typed LOCAL carries its own crate qualification — `let a: &dyn beta::Handler = b;`.
                // Only SIGNATURES were ever recorded, so such a local had no crate identity of its own and,
                // when it shadowed a param of the same name, inherited the PARAM's crate: the call keyed to
                // the wrong dependency and its real reach was lost. (The pre-tombstone last-wins map
                // happened to supply the right answer here by accident, which is why this surfaced only
                // once the guessing stopped.) Recorded under the binding name, so the local correctly
                // SHADOWS the parameter entry.
                {
                    let mut per = HashMap::new();
                    crate::lang::collect_trait_quals_pub(&pt.ty, &mut per);
                    if per.is_empty() {
                        self.trait_quals_by_param.remove(&id.ident.to_string());
                    } else {
                        self.trait_quals_by_param.insert(id.ident.to_string(), per);
                    }
                }
                // `self.generic_bounds` for the same reason as the `trait_leaves` call below: an
                // annotation can name a generic (`let g: F = f;` under `<F: Fn()>`). The PARAMETER
                // position already discloses `Unknown` for that callable; the annotation read pure.
                if is_callable_type(&pt.ty, &self.generic_bounds) {
                    self.fn_typed_vars.insert(id.ident.to_string());
                    self.vars.remove(&id.ident.to_string());
                } else {
                    self.fn_typed_vars.remove(&id.ident.to_string()); // a non-callable annotation clears it
                }
                // `self.generic_bounds`, not an empty map: an annotation can NAME a generic parameter
                // (`let d: T = pick();` under `fn f<T: Doer>`), and with no bounds to consult that read
                // silent-pure while the identical PARAMETER position (`fn f<T: Doer>(d: T)`) resolved.
                // Measured with a `dyn` control — `let d: Box<dyn Doer> = x` already resolved here — so
                // the position was live and only the bound question was never asked.
                let leaves = trait_leaves(&pt.ty, &self.generic_bounds);
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
                // …and the DISPATCH counterpart, which this site never asked for at all: a
                // `let v: Vec<Box<dyn Doer>> = ..` / `let m: HashMap<K, Box<dyn Doer>> = ..` /
                // `let v: Vec<T> = ..` under `<T: Doer>` left the later `for d in v { d.go() }` untyped
                // and silent-pure, though the same shapes in PARAMETER (`seed_elem_of`) and FIELD
                // (`field_elem_trait`) position have resolved since R37/R40. Same helper, same
                // arguments — this is the annotation catching up with the other two positions, not a
                // new resolution path. Cleared first, so an annotation that is NOT a dispatch container
                // cannot leave a previous binding's leaves standing (the stale-rebind defect of `71c2495`).
                self.elem_trait_of.remove(&id.ident.to_string());
                let elem_leaves = elem_trait_leaves(&pt.ty, &self.generic_bounds);
                if !elem_leaves.is_empty() {
                    self.elem_trait_of.insert(id.ident.to_string(), elem_leaves);
                }
                // A TUPLE-typed let (`let pair: (Sender, usize) = ..`) — record its per-position types
                // so a later `let (s, _) = pair;` types `s`.
                self.tuple_of.remove(&id.ident.to_string());
                if let Some(t) = tuple_types(&pt.ty, self.uses) {
                    self.tuple_of.insert(id.ident.to_string(), t);
                }
                // …and the tuple's per-position DISPATCH leaves, the `tuple_trait_of` twin of the above.
                self.tuple_trait_of.remove(&id.ident.to_string());
                if let Some(t) = tuple_trait_leaves(&pt.ty, &self.generic_bounds) {
                    self.tuple_trait_of.insert(id.ident.to_string(), t);
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
            // A TUPLE-OF-DYN source's per-position dispatch leaves — from a source VAR's `tuple_trait_of`
            // (`let (d, _) = pair` where `pair: (Box<dyn Doer>, u32)`), or a FACTORY CALL's `<tupledyn>`
            // return sentinel (`let (d, _) = make()` where `make() -> (Box<dyn Doer>, u32)`) — so `d.go()`
            // dispatches (R46 tuple).
            let src_trait_tuple = match init {
                Some(syn::Expr::Path(p)) => p
                    .path
                    .get_ident()
                    .and_then(|id| self.tuple_trait_of.get(&id.to_string()))
                    .cloned(),
                Some(syn::Expr::Call(c)) => {
                    if let syn::Expr::Path(p) = &*c.func {
                        let full = path_to_string(&p.path);
                        let leaf = full.rsplit("::").next().unwrap_or(&full);
                        self.returns.get(leaf).and_then(|t| ret_tuple_dyn_leaves(t))
                    } else {
                        None
                    }
                }
                _ => None,
            };
            for (i, pat_el) in tup.elems.iter().enumerate() {
                let Some(name) = single_pat_ident(pat_el) else { continue };
                self.vars.remove(&name);
                self.dep_bound_vars.remove(&name);   // a destructured rebind drops stale provenance too
                self.elem_of.remove(&name);
                self.tuple_of.remove(&name);
                self.trait_vars.remove(&name);
                let ty = src_tuple
                    .as_ref()
                    .and_then(|t| t.get(i).cloned().flatten())
                    .or_else(|| match init {
                        Some(syn::Expr::Tuple(it)) => {
                            it.elems.iter().nth(i).and_then(|e| ctor_type(e, self.uses, self.returns))
                        }
                        _ => None,
                    });
                // Bind a DISPATCH position into `trait_vars`: from the source var's `tuple_trait_of`, or an
                // inline tuple literal's cast element (`(x as Box<dyn Doer>, 1)` → position 0's leaves).
                let leaves = src_trait_tuple
                    .as_ref()
                    .and_then(|t| t.get(i).cloned())
                    .filter(|l| !l.is_empty())
                    .or_else(|| match init {
                        Some(syn::Expr::Tuple(it)) => it.elems.iter().nth(i).and_then(|e| {
                            let inner = match e {
                                syn::Expr::Cast(c) => trait_leaves(&c.ty, &std::collections::HashMap::new()),
                                _ => Vec::new(),
                            };
                            (!inner.is_empty()).then_some(inner)
                        }),
                        _ => None,
                    });
                // A position with dispatch leaves takes the `trait_vars` route and must NOT also land in
                // `vars` — the two collide on exactly one shape, a bare generic parameter, where
                // `tuple_types` yields the SPELLING (`"T"`) while `tuple_trait_leaves` yields the BOUND.
                // `vars` wins at the call site, so `let p: (T, u32) = t; let (d, _) = p; d.go()` resolved
                // `d` to a type named `T` — which is nothing — and read silent-pure, while the same
                // destructure of a `(Box<dyn Doer>, u32)` (where `tuple_types` yields `None`) resolved.
                // The spelling is never a real type here, so this loses nothing and closes the shadow.
                if let Some(ty) = ty.filter(|_| leaves.is_none()) {
                    self.vars.insert(name.clone(), ty);
                }
                if let Some(l) = leaves {
                    self.trait_vars.insert(name, l);
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
                        // PROVENANCE without a type: `let c = deplib::build();` where `build`'s return
                        // type does not travel in the report (a pure factory is omitted from it
                        // entirely). Record the crate root so a later `c.fetch()` can DISCLOSE rather
                        // than silently drop — half 1 of DEP-RECEIVER-TYPING-DESIGN.md. Cleared on any
                        // rebind, and immediately overwritten below if the init DOES type.
                        self.dep_bound_vars.remove(&id.ident.to_string());
                        if let syn::Expr::Call(c) = peel_recv(&init.expr) {
                            if let syn::Expr::Path(p) = &*c.func {
                                let full = expand(&path_to_string(&p.path), self.uses);
                                // A multi-segment path whose head is a plausible crate root. The head is
                                // checked against the manifest's declared deps at CONSUMPTION in scan.rs,
                                // so a local module sharing the shape emits an inert marker.
                                //
                                // The WHOLE expanded callee path is stored, not just the head. The head
                                // is what half 1's disclosure needs; the full path is the key half 2
                                // looks up in the dependency's published `typeSurface.returns`, which is
                                // keyed by the dependency's MODULE-QUALIFIED fn qual.
                                if full.contains("::") && !full.starts_with("::") {
                                    self.dep_bound_vars.insert(id.ident.to_string(), full.clone());
                                }
                            }
                        }
                        if let Some(ty) = ctor_type(&init.expr, self.uses, self.returns) {
                            self.vars.insert(id.ident.to_string(), ty.clone());
                            // It typed after all — the provenance marker is redundant and must not fire.
                            self.dep_bound_vars.remove(&id.ident.to_string());
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
                                    path_lits_partial: false, path_lit2: None,
                                    typed: false,
                                    method: false,
                                    is_macro: false,
                                });
                            }
                        } else if let syn::Expr::MethodCall(m) = &*init.expr {
                            // A `.clone()` REBIND is type-preserving — `Clone::clone(&self) -> Self`, so the
                            // binding has the receiver's type: `let b = a.clone(); b.run()` must resolve
                            // `b.run()` through `a`'s type (ctor_type misses this — it doesn't consult `vars`
                            // for the clone receiver, so `b` typed to nothing and dropped SILENT-PURE, R52).
                            // Carrying the type NEVER fabricates (clone cannot change the type, and the clone
                            // CALL itself stays uncharged — the anti-fabrication clone guard is untouched);
                            // scan.rs's `local_types` gate still confines any resulting `Type::method` edge.
                            if m.method == "clone" {
                                if let Some(t) = self.resolve_recv_type(&m.receiver) {
                                    self.vars.insert(id.ident.to_string(), t);
                                }
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
            // `respan_call_site`: these tokens were parsed on a rayon worker while this walk runs on the
            // collector thread, so their spans index a source map this thread does not have. See that
            // function for what syn does with them if they are handed over as-is.
            if let Ok(arms) = syn::parse2::<CfgIfArms>(respan_call_site(node.tokens.clone())) {
                for block in &arms.0 {
                    self.visit_block(block);
                }
                return;
            }
        }
        // LOCAL `macro_rules!` EXPANSION (R48): a bare `NAME!(..)` whose TEMPLATE does I/O or calls a local
        // fn read silent-pure — syn leaves the macro body opaque, and the arg-walk below only sees the
        // INVOCATION args, never the definition template. Inline-expand the recorded template so its own
        // calls are charged to this fn (the macro really does expand here). Metavars are `$`-stripped and
        // the arm parse-or-skipped as a block, so this only ever ADDS visibility — a `$(..)*` repetition or
        // otherwise-unparseable template is skipped, never fabricated. Recursion-guarded per macro name.
        // CRUCIAL — expand ONLY a SINGLE-ARM macro: a multi-arm `macro_rules!` invocation matches EXACTLY ONE
        // arm, but a syntactic (unexpanded) scan can't tell which, so walking every arm would charge a
        // NON-matching arm's effect onto a call that only expands a different arm — a FABRICATION (found in
        // code review: `emit!(log x)` on `macro_rules! emit { (log $m)=>{..}; (save $m)=>{fs::write(..)} }`
        // wrongly read Fs). A multi-arm macro is left an honest under-report; anti-fabrication wins over
        // recall. (Single-arm covers the dominant effectful-macro shape — logging wrappers like `trace!`.)
        if !mpath.contains("::")
            && !self.macro_expanding.contains(&mleaf)
            && self.local_macros.contains_key(&mleaf)
        {
            if let Some(body) = self.local_macros.get(&mleaf).cloned() {
                let (arm_count, blocks) = macro_template_blocks(&body);
                // Only a genuinely SINGLE-arm macro (one arm total, and it parsed) — multi-arm is skipped to
                // avoid charging a non-matching arm's effect (the review-caught fabrication).
                if arm_count == 1 && blocks.len() == 1 {
                    self.macro_expanding.insert(mleaf.clone());
                    let before = self.calls.len();
                    self.visit_block(&blocks[0]);
                    // Mark every call the template contributed as macro-origin (`is_macro`): a `$`-stripped
                    // metavar in CALLEE/RECEIVER position (`$f()` → `f()`, `$x.m()` → `x.m()`) becomes a bare
                    // ident that must NOT resolve to a same-named local fn/method — that is a FABRICATION
                    // (review [7]: `run!(cb)` on `macro_rules! run { ($x:expr) => { $x() } }` wrongly edged to
                    // a local `fn f`). `is_macro` suppresses LOCAL resolution (scan.rs gates `resolvable` on
                    // it) while KEEPING classification of `::`-qualified std/crate calls (`fs::write`→Fs) and
                    // the nested `::`-macro effects (`tracing::trace!`→Log) — the genuine recoveries survive,
                    // and a template that only calls a bare local fn is now an honest under-report.
                    for c in self.calls[before..].iter_mut() {
                        c.is_macro = true;
                    }
                    self.macro_expanding.remove(&mleaf);
                }
            }
        }
        if mpath.contains("::") {
            self.calls.push(Call { path: mpath, leaf: mleaf.clone(), str_arg: None, typed: false, method: false, is_macro: true,
                            path_lits_partial: false, path_lit2: None,
                        });
        }
        // syn does not parse a macro's body, so every call hidden inside one is invisible by default —
        // a real miss on crates that route effectful calls through a macro (git2 wraps EVERY libgit2 FFI
        // call in `try_call!(raw::git_...())`; `println!("{}", f())` hides `f`). Best-effort: parse the
        // token stream as comma-separated expressions and walk any that parse. If the body isn't
        // expression syntax (`quote!{}`, `matches!(x, Pat)`, macro_rules arms), parsing fails and we skip
        // — so this only ever ADDS visibility, never breaks. Owned exprs, so visit a local copy.
        // `respan_call_site` is REQUIRED, not hygiene: these tokens were parsed on a rayon worker and
        // this walk runs on the collector thread, so their spans index a source map this thread does not
        // have — a `-1` anywhere in the body reaches syn's `parse_negative_lit`, which JOINS spans, and
        // the join aborts the parser (getrandom's `debug_assert!`). See `respan_call_site`.
        let parser = syn::punctuated::Punctuated::<syn::Expr, syn::Token![,]>::parse_terminated;
        if let Ok(exprs) = syn::parse::Parser::parse2(parser, respan_call_site(node.tokens.clone())) {
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

/// Peel the wrappers a method-call RECEIVER can carry without changing which value it is — `&x`, `(x)`,
/// `{x}`, `x?`, `x.await`. Mirrors the peeling `resolve_recv_type` does, so the untyped-dep-receiver
/// disclosure sees `deplib::build()` through `(&deplib::build())`, `deplib::build()?` and
/// `deplib::build().await` exactly as it sees the bare spelling.
pub(crate) fn peel_recv(expr: &syn::Expr) -> &syn::Expr {
    let mut e = expr;
    loop {
        e = match e {
            syn::Expr::Reference(r) => &r.expr,
            syn::Expr::Paren(p) => &p.expr,
            syn::Expr::Group(g) => &g.expr,
            syn::Expr::Try(t) => &t.expr,
            syn::Expr::Await(a) => &a.base,
            other => return other,
        };
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

/// Above how many definitions one re-exported NAME may stand for before the alias is dropped. A name
/// with several targets is normal and correct — a `#[cfg]`-redirected platform module contributes one
/// definition per target — but only up to a point: past it the edge is not a platform split, it is
/// something this index has mis-read, and an honest miss beats a fan-out nobody can explain. The bound
/// matches the trait-CHA fallback's, for the same reason.
const REEXPORT_FANOUT_MAX: usize = 12;

/// How many re-export HOPS a name may travel. Re-exports chain (`a` re-exports `a::b`, which re-exports
/// `a::b::c`), so the alias index is a fixpoint; the bound is what stops a cyclic `pub use` pair spinning.
/// Separate from the fan-out bound above: they limit different things and neither should follow the other.
const REEXPORT_CHAIN_MAX: usize = 12;

/// Sentinel "edge id" for a name a module DECLARES itself, as opposed to one a `pub use` brought in.
const DECLARED_HERE: usize = usize::MAX;

/// Fold the crate's `pub use` RE-EXPORT edges into the alias index `reexport_target` consults:
/// `tail2 of <module>::<name>` -> the definition qual(s) that name stands for.
///
/// THE PROBLEM. The intra-crate call graph keys a qualified call on its last TWO segments. A definition
/// re-exported out of its own module is nameable by two different tails — `platform::doit` where it is
/// written, `imp::doit` where callers write it — and only the first was ever indexed, so the caller
/// resolved to nothing and read silent-pure (tempfile's eight `NamedTempFile` entry points, whose only
/// `Fs` sits behind `pub use self::platform::*`).
///
/// THE RULES, each of which is a direction this index refuses to guess in:
///   * A name the module DECLARES ITSELF is never aliased — the primary index already owns it, and in
///     Rust a declaration shadows a glob import anyway.
///   * A name reaching a module through TWO OR MORE independent `pub use` edges is dropped. That is the
///     never-guess rule the leaf and tail2 indexes already apply, one level up.
///   * ONE edge standing for several definitions is KEPT and charged as a union — that is the cfg
///     platform split (`#[cfg_attr(unix, path = "unix.rs")] mod platform;`), where the scanner already
///     analyses every branch and `cfg_if` already unions every arm.
///   * A tail2 key two DIFFERENT modules would answer differently is dropped, because the key cannot
///     tell them apart. (`file::imp::create` and `dir::imp::create` are both `imp::create` — and so is
///     the CALL, which is why the primary index cannot tell them apart either.)
///
/// Fixpoint, because re-exports chain: `a` re-exports `a::b`, which re-exports `a::b::c`, and `a::doit()`
/// has to travel both edges. Bounded, so a cyclic `pub use` pair cannot spin.
pub(crate) fn reexport_aliases(edges: &[Reexport], fns: &[FnInfo]) -> HashMap<String, Vec<String>> {
    use std::collections::BTreeMap;
    // module key -> name -> (which edges put it there, which definitions it stands for). The module key
    // is the fn qual minus its last segment, so a FREE fn keys on its module (`imp::platform`) and a
    // METHOD keys one level deeper (`imp::platform::Type`) — which is what makes a glob from a module
    // pick up that module's free functions and nothing else.
    /// Which `pub use` edges put a name in a module, and which definitions it stands for. Named because
    /// clippy's `type_complexity` refuses the inline spelling and CI denies warnings — the alias is the
    /// documentation the nested generics were not.
    type ExportClaims = (BTreeSet<usize>, BTreeSet<String>);
    let mut exported: BTreeMap<String, BTreeMap<String, ExportClaims>> = BTreeMap::new();
    for f in fns {
        // A synthetic lazy-init unit is reachable only through the forcing edge its own qual spells out.
        if f.qual.starts_with(LAZY_UNIT_PREFIX) {
            continue;
        }
        let Some((m, n)) = f.qual.rsplit_once("::") else { continue };
        let e = exported.entry(m.to_string()).or_default().entry(n.to_string()).or_default();
        e.0.insert(DECLARED_HERE);
        e.1.insert(f.qual.clone());
    }
    for _ in 0..REEXPORT_CHAIN_MAX {
        let mut changed = false;
        for (i, ed) in edges.iter().enumerate() {
            let mut adds: Vec<(String, String)> = Vec::new();
            for src in &ed.from {
                let Some(names) = exported.get(src) else { continue };
                if ed.name == "*" {
                    for (n, (_, quals)) in names {
                        adds.extend(quals.iter().map(|q| (n.clone(), q.clone())));
                    }
                } else if let Some((_, quals)) = names.get(&ed.name) {
                    adds.extend(quals.iter().map(|q| (ed.alias.clone(), q.clone())));
                }
            }
            for (n, q) in adds {
                let e = exported.entry(ed.module.clone()).or_default().entry(n).or_default();
                changed |= e.0.insert(i);
                changed |= e.1.insert(q);
            }
        }
        if !changed {
            break;
        }
    }
    // WHO COULD CLAIM EACH tail2 KEY, before any other rule has run. A call writes `imp::create`, and
    // `dir::imp` and `file::imp` are both spelled `imp` in a 2-segment tail — the key names neither, so
    // it must answer for neither.
    //
    // MEASURED: the first cut of this index counted claimants only among the aliases that SURVIVED the
    // per-module rules, and tempfile's `dir::imp` had already been dropped by the two-edge rule (its
    // re-export is a `#[cfg]` pair). One claimant was left standing, the key looked unambiguous, and
    // `dir::create` — which only makes a directory — inherited `file::imp`'s temp-name `Env` + `Rand`.
    // A key is ambiguous because of who COULD claim it, not who is left after the other filters.
    let mut claimants: BTreeMap<String, BTreeSet<&String>> = BTreeMap::new();
    for (module, names) in &exported {
        // The crate ROOT has no second segment to key on — and a root re-export already resolves, both
        // by bare leaf and through `collect_root_reexports`.
        if module.is_empty() {
            continue;
        }
        let mlast = module.rsplit("::").next().unwrap_or(module);
        for (name, (from_edges, _)) in names {
            // A name the module only DECLARES is not a claim on the alias index: the primary tail2 index
            // holds it, and `reexport_target` steps aside whenever that index has the key at all.
            if from_edges.iter().all(|e| *e == DECLARED_HERE) {
                continue;
            }
            claimants.entry(format!("{mlast}::{name}")).or_default().insert(module);
        }
    }
    let mut keyed: HashMap<String, Vec<String>> = HashMap::new();
    for (module, names) in &exported {
        if module.is_empty() {
            continue;
        }
        let mlast = module.rsplit("::").next().unwrap_or(module);
        for (name, (from_edges, quals)) in names {
            if from_edges.contains(&DECLARED_HERE) || from_edges.len() != 1 {
                continue;
            }
            if quals.is_empty() || quals.len() > REEXPORT_FANOUT_MAX {
                continue;
            }
            let key = format!("{mlast}::{name}");
            if claimants.get(&key).is_none_or(|c| c.len() != 1) {
                continue;
            }
            keyed.insert(key, quals.iter().cloned().collect());
        }
    }
    keyed
}

/// The definition(s) a qualified FREE call names through a SUBMODULE-level `pub use` re-export, for a
/// call `resolve_target` could not place. Consulted ONLY when the call's 2-segment tail names NO
/// definition at all, so an alias can never override a definition nor break an ambiguity tie — it is a
/// weaker fact than a definition and is used only where there was nothing.
///
/// METHOD calls are excluded outright. A re-export alias keys on `<module>::<fn>`, which shares its key
/// space with `<Type>::<method>`; letting one answer a method call would mark that call locally-resolved
/// and so SUPPRESS the classifier for a receiver the classifier owns — trading a miss for a wrong
/// answer. A re-export is about free functions named through a module: exactly the non-method form.
pub(crate) fn reexport_target<'a>(
    path: &str,
    method: bool,
    by_tail2: &HashMap<String, Vec<String>>,
    by_reexport: &'a HashMap<String, Vec<String>>,
) -> Option<&'a Vec<String>> {
    if method || !path.contains("::") {
        return None;
    }
    let t2 = tail2(path)?;
    if by_tail2.contains_key(&t2) {
        return None;
    }
    by_reexport.get(&t2)
}

/// The parsed BLOCKS of a `cfg_if::cfg_if! { .. }` body — one per arm. `cfg_if`'s grammar is a chain of
/// `if #[cfg(COND)] { BLOCK }` clauses (each cfg is an outer attribute on the arm), optionally chained by
/// `else if #[cfg(COND)] { BLOCK }`, and optionally terminated by a bare `else { BLOCK }`. We keep EVERY
/// arm's block (sound over-approximation — see the call site) and DISCARD the `#[cfg(..)]` conditions
/// themselves: they only decide which arm compiles, not what any arm does, and candor-scan already scans
/// all `#[cfg(unix)]`/`#[cfg(windows)]`/… branches regardless. Parsing is strict — any token that isn't
/// this exact `if #[cfg(..)] { } [else if #[cfg(..)] { }]* [else { }]?` shape makes the parse fail, so the
/// caller falls back to treating the macro as opaque rather than mis-reading a novel `cfg_if` extension.
/// Parse a `macro_rules!` body (`(matcher) => { template }; …`) into the parseable arm TEMPLATE blocks.
/// Metavars are `$`-stripped (a `$msg` metavar → the ident `msg` — an untyped local that resolves to no
/// effect, never a fabrication) and each template is parse-or-skipped as a block: an arm using repetition
/// (`$(..)*`) or non-block syntax simply yields nothing. Only ADDS visibility to an effectful template.
/// Returns `(total_arm_count, parseable_arm_templates)`. The caller expands ONLY when `total_arm_count == 1`
/// (a single-arm macro is unambiguous — every invocation expands it): a multi-arm macro's invocation matches
/// exactly ONE arm and a syntactic scan can't tell which, so walking all arms would fabricate a non-matching
/// arm's effect. `total_arm_count` counts EVERY `(..) => {..}` arm — including one whose template fails to
/// parse (so a 2-arm macro with one `$(..)*`-repetition arm is correctly seen as multi-arm, not single).
fn macro_template_blocks(body: &str) -> (usize, Vec<syn::Block>) {
    use proc_macro2::{Delimiter, Group, TokenStream, TokenTree};
    let ts: TokenStream = match body.parse() {
        Ok(t) => t,
        Err(_) => return (0, Vec::new()),
    };
    let mut arm_count = 0usize;
    let mut out = Vec::new();
    let mut it = ts.into_iter().peekable();
    while let Some(tok) = it.next() {
        // arm matcher — a delimited group `(..)`/`[..]`/`{..}`
        if !matches!(tok, TokenTree::Group(_)) {
            continue;
        }
        // the fat arrow `=>` (two joined puncts)
        match it.next() {
            Some(TokenTree::Punct(p)) if p.as_char() == '=' => {}
            _ => continue,
        }
        match it.next() {
            Some(TokenTree::Punct(p)) if p.as_char() == '>' => {}
            _ => continue,
        }
        // the template group
        let tmpl = match it.next() {
            Some(TokenTree::Group(g)) => g.stream(),
            _ => continue,
        };
        arm_count += 1; // a well-formed arm, whether or not its template parses below
        let braced = TokenStream::from(TokenTree::Group(Group::new(Delimiter::Brace, strip_dollars(tmpl))));
        if let Ok(block) = syn::parse2::<syn::Block>(braced) {
            out.push(block);
        }
        // optional `;` separator between arms
        if let Some(TokenTree::Punct(p)) = it.peek() {
            if p.as_char() == ';' {
                it.next();
            }
        }
    }
    (arm_count, out)
}

/// Drop `$` punct tokens (recursing into groups) so a `macro_rules!` template's metavars (`$msg`, `$crate`)
/// become plain idents/paths and the template parses as ordinary Rust. `$(..)*` repetition survives as
/// `(..)*` which won't parse as a statement — the caller's parse-or-skip drops that arm.
fn strip_dollars(ts: proc_macro2::TokenStream) -> proc_macro2::TokenStream {
    use proc_macro2::{Group, TokenTree};
    ts.into_iter()
        .filter_map(|tt| match tt {
            TokenTree::Punct(p) if p.as_char() == '$' => None,
            TokenTree::Group(g) => {
                Some(TokenTree::Group(Group::new(g.delimiter(), strip_dollars(g.stream()))))
            }
            other => Some(other),
        })
        .collect()
}

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
