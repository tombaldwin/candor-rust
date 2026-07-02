//! candor-scan — a STABLE-Rust effect scanner. Produces the same candor report JSON the nightly
//! `rustc_private` lint does, but with a purely syntactic backend: walk the crate's `.rs` files, parse
//! with `syn`, build a name-resolved-enough call graph, classify path-qualified calls with the shared
//! `candor-classify`, and propagate transitively. No nightly, no `rustc-dev`, no dylint — `cargo install`
//! and run anywhere.
//!
//! HONEST PRECISION TRADE vs the lint. This is syntactic, so it sees what's written, not what's
//! resolved. It CATCHES: path-qualified effect calls (`std::fs::read`, `reqwest::Client::execute`,
//! `Command::new`), including `use`-aliased prefixes; and intra-crate calls (matched by name) for
//! transitive propagation. It MISSES (silently — it does NOT emit `Unknown`): effects reached only
//! through a method call whose receiver type isn't path-qualified, trait-object dispatch, closures /
//! fn-pointers, macros, and cross-crate propagation by stable identity. So on resolution-heavy code it
//! under-reports relative to the lint. Use the lint when you need the soundness contract; use this when
//! you need zero-friction, stable, installable triage. Shares the lint's classifier — one source of truth.
//!
//! CALL RESOLUTION. The local call graph is name-resolved, not type-resolved. A qualified `Type::method`
//! call (or an associated-fn call `RequestBuilder::new()`) is matched on its 2-segment tail, but ONLY when
//! that tail is UNAMBIGUOUS, which keeps same-named methods on different types distinct. A bare
//! free-function call falls back to a unique leaf. A `.method()` call whose receiver type is inferred to a
//! LOCAL type resolves through that type's `Type::method` tail (so `x.go()` reaches a local `S::go`); an
//! external or un-inferrable receiver leaves the bare `.method()` with no definite target, so it
//! under-reports rather than guess (this is what stops `range.start()` — on the external `FloatRange` —
//! linking to a unique local `Clipboard::start`). We deliberately do NOT link a many-way-ambiguous name:
//! on a real crate that would link every `.new()` to all 100+ `*::new` defs and smear one type's effect
//! across the whole graph. Under-reporting an ambiguous edge is the honest failure
//! mode; fabricating one is never ok. The shared resolver is `resolve_target`.
//!
//! Usage:  candor-scan [<crate-dir>] [--out <prefix>] [--json] [--include-tests]
//!   default dir = ".", default prefix = "<dir>/.candor/report"; writes <prefix>.<crate>.scan.json (+ a
//!   callgraph sidecar so `cargo candor callers <fn>` works on the stable report too). `--json` prints
//!   the report to stdout instead. By DEFAULT only the crate's library/binary source is scanned —
//!   `tests/`, `benches/`, `examples/`, `test/`, the root `build.rs` (the Cargo build script — but NOT a
//!   `src/build.rs` source module), and `#[cfg(test)]` modules are skipped, so
//!   the report describes what the CRATE does, not what its harness does (`--include-tests` keeps them).
//!   See eval/calibration for accuracy on 35 real crates.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::path::Path;

use candor_report::ReportEntry;
use syn::visit::Visit;

/// A call observed in a function body: the (use-expanded) path string and the leaf name.
//
// The serde attributes are PURELY a cache-wire-format optimization (short field names + omit the common
// defaults): they shrink the consolidated cache, which is read+written every incremental scan. They do
// NOT change any in-memory behaviour — the deserialized value is identical, and `serde(default)` restores
// the omitted fields. The equivalence fuzzer guards that this representation round-trips exactly.
#[derive(Clone, serde::Serialize, serde::Deserialize)]
struct Call {
    #[serde(rename = "p")]
    path: String,            // "std::fs::read", "compute_price", "pricing::priced"
    #[serde(rename = "l")]
    leaf: String,            // last segment
    #[serde(rename = "s", default, skip_serializing_if = "Option::is_none")]
    str_arg: Option<String>, // first string-literal argument (host/cmd/path detail)
    /// Synthesized from receiver-type inference (`reqwest::Client::send` from `client.send()`). Used for
    /// external-crate classification ONLY — excluded from local call-graph edges, since its `Type::method`
    /// tail could spuriously link to a same-named LOCAL method the call doesn't actually target.
    #[serde(rename = "t", default, skip_serializing_if = "std::ops::Not::not")]
    typed: bool,
    /// A METHOD call (`x.foo()`) vs a free-function/path call (`foo()`, `m::foo()`). When the receiver type
    /// can't be inferred, an unqualified method call has NO sound bare-leaf target — linking it to a
    /// same-named def would guess (`.bool()`→free `random::bool::bool`, `range.start()`→`Clipboard::start`),
    /// fabricating that def's effect. So such calls resolve to nothing; only the receiver-typed/qualified
    /// form (the `typed` call) links a method edge. Found on nushell (Rand/Clipboard on the random cmds).
    #[serde(rename = "m", default, skip_serializing_if = "std::ops::Not::not")]
    method: bool,
    /// A MACRO invocation (`log::info!`, `duct::cmd!`, `crate::helpers::trace!`). Recorded so its path can
    /// be classified/builder-mapped/disclosed like an external call — but a macro is NEVER a call to a local
    /// FUNCTION, so it must be EXCLUDED from local call-graph edge resolution: a crate-local macro path
    /// (`crate::helpers::trace` after `expand`) keeps its `::` and would otherwise mis-link to a same-named
    /// local fn, fabricating that fn's effect onto a pure caller (the same hazard the `typed` flag guards).
    #[serde(rename = "mac", default, skip_serializing_if = "std::ops::Not::not")]
    is_macro: bool,
}

/// One function the scan found: its module-qualified name, where, and the calls in its body.
// The serde attributes are a cache-wire-format optimization only (see `Call`); in-memory behaviour is
// unchanged and the equivalence fuzzer guards the round-trip.
#[derive(Clone, serde::Serialize, serde::Deserialize)]
struct FnInfo {
    #[serde(rename = "q")]
    qual: String,
    #[serde(rename = "l")]
    leaf: String,
    #[serde(rename = "f")]
    loc: String,
    #[serde(rename = "c", default, skip_serializing_if = "Vec::is_empty")]
    calls: Vec<Call>,
    /// The body invoked a callable the syntactic scan can't see through — a closure / fn-pointer value
    /// (`(cb)()`, `arr[i]()`, a local bound to a closure). The target could perform ANY effect, so the
    /// function can't honestly be certified pure: it's marked `Unknown` (matching the nightly lint's
    /// soundness fallback) rather than silently reported clean.
    #[serde(rename = "u", default, skip_serializing_if = "std::ops::Not::not")]
    unresolved: bool,
}

/// `struct-name-leaf -> { field -> expanded-type-path }`, e.g. `App -> { http: reqwest::Client }`.
/// Built crate-wide in a pre-pass so a method call on `self.http` can be resolved to its type and
/// classified by the existing per-crate method rules (`reqwest::Client::execute` -> Net).
type FieldIndex = HashMap<String, HashMap<String, String>>;

/// `struct-name-leaf -> { field -> ELEMENT-type-path }` for COLLECTION-typed fields, e.g.
/// `Pool -> { senders: Sender }` (the element T of `Vec<Sender>`). Lets a loop/index/closure over a
/// collection FIELD (`for c in &self.senders`, `self.senders[0].send()`) type its element so the
/// element's method calls classify, instead of silently dropping to pure (a §4 under-report).
type FieldElemIndex = HashMap<String, HashMap<String, String>>;

/// `var-name -> per-position element types of a tuple binding` (`None` = position not type-resolved),
/// e.g. a `let (s, _) = pair` over a `(Sender, usize)` param records `pair -> [Some(Sender), None]`.
type TupleElemIndex = HashMap<String, Vec<Option<String>>>;

/// `enum-variant-leaf -> the single payload type` for SINGLE-payload tuple variants, e.g.
/// `Active -> Sender` from `enum Conn { Active(Sender) }`. Lets a match-arm binding
/// (`Conn::Active(s) => s.send()`) type `s` from the variant's payload. Only UNAMBIGUOUS variant
/// names are kept (a leaf two enums share with different payloads is dropped — never guess), mirroring
/// the return-index ambiguity rule.
type EnumVariantIndex = HashMap<String, String>;

/// `fn-leaf -> expanded return-type-path`, e.g. `create_pool -> sqlx::Pool` (Result/Option unwrapped).
/// Lets type inference flow through a LOCAL factory function: `let p = create_pool()?; p.fetch_one(q)`.
/// Only UNAMBIGUOUS leaves are kept — a name with two different return types across the crate is dropped
/// (no guess), like the unique-leaf call-graph rule.
type ReturnIndex = HashMap<String, String>;

/// Sentinel return-"type" for a fn whose return is a CALLABLE (`-> fn()`/`-> impl Fn`/`-> Box<dyn Fn>`).
/// Stored in the return index under the fn's leaf; `expr_is_fn_typed` reads it so `let g = make_cb()`
/// propagates fn-typed-ness, while `ctor_type` filters it out of var-typing (it's not a nominal type).
/// The angle brackets cannot collide with a real Rust type path.
const RET_FN_TYPED: &str = "<fn>";

/// `trait leaf -> the local types that `impl Trait for Type` it` — the syntactic CHA universe for
/// dispatch-typed receivers (the JVM engine's bounded-CHA move, done on syntax). Keyed by leaf like
/// the other name indexes; includes impls of EXTERNAL traits for local types (the JVM resolves
/// interface impls the same way regardless of where the interface is declared).
type TraitImplIndex = HashMap<String, Vec<String>>;
/// `struct leaf -> field name -> trait bound leaves` for dispatch-typed FIELDS (`store: Box<dyn
/// Store>`) — the DI pattern `self.store.save()`, which `FieldIndex` can't carry (no concrete type).
type TraitFieldIndex = HashMap<String, HashMap<String, Vec<String>>>;

/// A locally-declared trait: how many declarations share the leaf (ambiguity check) and which
/// method names the declaration itself carries — CHA resolves ONLY calls to a declared method of
/// an unambiguous local trait (review found the wider rule fabricating: `impl Iterator for
/// RowIter` + `fn f(it: impl Iterator)` charged pure `f` with RowIter's Db).
#[derive(Default)]
struct LocalTrait {
    count: usize,
    methods: std::collections::HashSet<String>,
}

/// The trait indexes Pass A builds (impl universe, local declarations, dispatch-typed fields),
/// bundled so Pass B threads one handle instead of three more arguments.
#[derive(Clone, Copy)]
struct TraitIndexes<'a> {
    impls: &'a TraitImplIndex,
    decls: &'a HashMap<String, LocalTrait>,
    fields: &'a TraitFieldIndex,
}

/// The collection/enum indexes Pass A builds (collection-field element types, single-payload enum
/// variant types), bundled so Pass B threads one handle — the way `TraitIndexes` bundles the trait ones.
#[derive(Clone, Copy)]
struct ElemIndexes<'a> {
    field_elem: &'a FieldElemIndex,
    enum_variants: &'a EnumVariantIndex,
}

struct CallCollector<'a> {
    uses: &'a HashMap<String, String>,
    /// local variable / param / `self` -> expanded type path, grown as `let`s are visited in order.
    vars: HashMap<String, String>,
    /// local variable / param -> trait bound leaves, for dispatch-typed receivers (`t: &dyn Store`,
    /// `s: impl Store`, `x: X` under `X: Store`). Disjoint from `vars` (no concrete type to put there).
    trait_vars: HashMap<String, Vec<String>>,
    fields: &'a FieldIndex,
    trait_fields: &'a TraitFieldIndex,
    /// trait leaf -> local impl types (None entries never exist; absent = no local impl).
    trait_impls: &'a TraitImplIndex,
    /// leaf -> the local trait declaration(s) sharing it: ambiguity count + declared method names.
    local_traits: &'a HashMap<String, LocalTrait>,
    returns: &'a ReturnIndex,
    /// `Type-leaf -> field -> element-type` for COLLECTION fields (`self.senders[0]`, `for c in
    /// &self.senders`). The field counterpart of `elem_of`, the way `fields` is to `vars`.
    field_elem: &'a FieldElemIndex,
    /// `enum-variant-leaf -> single payload type` for match-arm binding (`Conn::Active(s) => s.send()`).
    enum_variants: &'a EnumVariantIndex,
    /// local var / param -> ELEMENT type of a COLLECTION it holds (a `Vec<T>`/`&[T]`/… binding), grown
    /// as collection-typed `let`s/params are seen. Lets `for c in xs`, `xs[0]`, `xs.iter().for_each`
    /// resolve the element's type. Scoped bindings (loop var, closure param) live in `vars`, not here.
    elem_of: HashMap<String, String>,
    /// local var / param -> the per-position types of a TUPLE it holds (`pair: (Sender, usize)` ->
    /// `[Some("Sender"), Some("usize")]`). Lets a later `let (s, _) = pair;` type each binding from the
    /// matching position. A `None` at a position = that element's type is unknown (binds nothing).
    tuple_of: HashMap<String, Vec<Option<String>>>,
    calls: Vec<Call>,
    /// locals bound to a closure (`let f = |..| ..`), so a later `f()` is recognised as a closure
    /// invocation the scan can't see through — not a call to a free fn named `f`.
    closure_vars: std::collections::HashSet<String>,
    /// params/locals of a fn-pointer / `impl`/`dyn Fn` / generic-`Fn`-bound type. Invoking one (`cb()`)
    /// calls an opaque body → honest `Unknown`, not a silently-dropped phantom call to a free fn `cb`.
    fn_typed_vars: std::collections::HashSet<String>,
    /// locals aliased to a free-FUNCTION path (`let g = eff;` where `eff` is a visible fn): a later `g()`
    /// resolves to the aliased path, so its effect (and whole transitive chain) is not silently dropped
    /// (sweep [6]). Keyed by the local name → the expanded callee path.
    fn_alias: std::collections::HashMap<String, String>,
    /// Crate-wide LAZY/deferred static names (`once_cell`/`std` `Lazy`/`LazyLock`/`LazyCell`,
    /// `lazy_static!`, `thread_local!`). A body that NAMES one of these FORCES its deferred init on
    /// first use — so naming the static edges to its synthetic init unit (`<lazy>::NAME`), carrying the
    /// init's effect to this fn. Over-approximating "names ⇒ forces" is a SAFE over-approximation (the
    /// init does run on first use), never a fabrication. Keyed per static NAME (not module-scoped), so a
    /// pure-init lazy contributes nothing. Set once per forcing site (de-duped via `forced_lazies`).
    lazy_statics: &'a std::collections::HashSet<String>,
    /// Lazy statics already FORCED (edged) in this body — emit at most one forcing edge per static, so a
    /// hot static read in a loop doesn't bloat the call list.
    forced_lazies: std::collections::HashSet<String>,
    /// set once the body invokes a callable we can't resolve (see `FnInfo::unresolved`).
    unresolved: bool,
    /// The ERROR type leaf of the enclosing fn's `Result<_, E>` return, if any — the `?` operator's
    /// `From::from` TARGET. A `may_fail()?` where `may_fail` returns `Result<_, E1>` and this fn returns
    /// `Result<_, E2>` desugars to `E2::from(e1)` via a local `impl From<E1> for E2`; we edge to
    /// `E2::from` when `E2` locally `impl From` (see `charge_from`). `None` for a non-fallible fn or an
    /// unresolvable/`Box<dyn Error>`/external error type → no `?` edge (the no-flood default).
    err_ret_leaf: Option<String>,
}

/// A freshly-parsed `syn::File` made movable across one thread boundary. `syn::File` is `!Send` solely
/// because `proc_macro2::TokenStream` holds an `Rc<Vec<TokenTree>>`. We enable proc-macro2's
/// `span-locations` feature (to fill each fn's `loc` with `file:line:col`): in fallback mode a `Span` then
/// carries inline `u32` byte offsets and `start()`/`end()` resolve them against a THREAD-LOCAL source map
/// populated when the file was parsed. Those byte offsets are plain `Copy` data and the source map is
/// per-thread state we never move — so `span-locations` adds nothing `!Send`; the `Rc` refcount remains the
/// ONLY thing that makes the type unsendable, and this `unsafe impl Send` stays sound. (The corollary the
/// loc derivation depends on: a span's line/col is ONLY resolvable on the thread that parsed the file —
/// after the move to the collector the source map is gone — so loc is computed in the parse closures.)
///
/// SAFETY CONTRACT: a `SendFile` is constructed from a `syn::parse_file` result that is UNIQUELY OWNED
/// (never cloned) and is MOVED EXACTLY ONCE — from the rayon worker that parsed it to the collector — and
/// thereafter accessed only single-threaded (the sequential Pass A / Pass B). No `Rc` clone is ever shared
/// between threads, so no refcount is ever touched concurrently; a one-time move of a uniquely-owned value
/// across a thread boundary races with nothing. This is exactly the case `unsafe impl Send` is sound for.
struct SendFile(syn::File);
// SAFETY: see the type doc — uniquely owned, moved once, then single-threaded. Sound for a parse result
// that is never `Rc`-aliased across threads. (Would be UNSOUND if a clone of the inner `TokenStream`
// were retained on the producing thread; we never clone before the move.)
unsafe impl Send for SendFile {}

/// A parse worker's output for one file: the (Send-wrapped) parsed file plus its per-fn `file:line:col`s
/// resolved on the worker (walk order; see `fn_locs`). Bundled so loc rides alongside the moved file.
type ParsedFile = (SendFile, Vec<String>);

fn path_to_string(p: &syn::Path) -> String {
    p.segments.iter().map(|s| s.ident.to_string()).collect::<Vec<_>>().join("::")
}

/// The synthetic-unit qual PREFIX for a LAZY/deferred static's init body. A deferred static
/// (`static X: Lazy<_> = Lazy::new(|| ..effect..)`, `thread_local!`, `lazy_static!`) attaches an init
/// CLOSURE that runs at FIRST USE, not at definition — so the closure body is reachable from no fn yet
/// performs the effect. We synthesize a unit per such static (`<lazy>::NAME`, the closure body walked
/// as a normal FnInfo) so the existing classifier/propagation charge it, then edge every FORCING site
/// (a fn that names the static) to it. A `<` in a path never collides with a real crate path (and a
/// resolved-local edge suppresses the classifier anyway), so the unit is never mis-classified. The
/// per-static keying is what prevents flooding: a PURE-init lazy's unit carries no effect, so its
/// forcing sites stay pure — only a genuinely-effectful init lights its accessors up.
const LAZY_UNIT_PREFIX: &str = "<lazy>";

/// Recognize the LAZY/deferred CONTAINER constructors whose argument is an init thunk run on first use.
/// A `Container::new(|| body)` defers the `body` to first use. Matched on the TYPE leaf of a `Type::new`
/// associated call so a
/// `use once_cell::sync::Lazy;` rename still hits. `OnceCell`/`OnceLock` are DELIBERATELY ABSENT: their
/// `get_or_init(|| ..)` already passes the closure at a normal reachable CALL SITE (the forcing site IS
/// the call), so the existing closure-arg walking already charges it — adding them would double-count.
fn is_lazy_container_new(path: &str) -> bool {
    // `Lazy::new`, `LazyLock::new`, `LazyCell::new` (once_cell sync/unsync + std). The penultimate
    // segment is the container TYPE; the last must be `new`.
    let Some(t2) = tail2(path) else { return false };
    let mut it = t2.split("::");
    let ty = it.next().unwrap_or("");
    let m = it.next().unwrap_or("");
    m == "new" && matches!(ty, "Lazy" | "LazyLock" | "LazyCell")
}

/// Extract the deferred INIT BODY (a block of statements) from a lazy-container `new` call's first
/// argument — a closure (`Lazy::new(|| { .. })`, `Lazy::new(|| expr)`) or a bare block
/// (`LazyLock::new(|| ..)` is the norm; a non-closure arg is not deferred). Returns the closure body as
/// statements to walk. A non-closure first arg (a function path `Lazy::new(load)`) is NOT inlined here —
/// it would be an ordinary reachable call if it appeared at a call site; the deferred-static seam is
/// specifically the inline CLOSURE/BLOCK, which is reachable from nowhere.
fn lazy_init_body(call: &syn::ExprCall) -> Option<Vec<syn::Stmt>> {
    if !matches!(&*call.func, syn::Expr::Path(p) if is_lazy_container_new(&path_to_string(&p.path))) {
        return None;
    }
    let first = call.args.first()?;
    closure_or_block_stmts(first)
}

/// The statements of an init thunk expression: a closure's body (block or single expr), or a bare block.
fn closure_or_block_stmts(e: &syn::Expr) -> Option<Vec<syn::Stmt>> {
    match e {
        syn::Expr::Closure(cl) => match &*cl.body {
            syn::Expr::Block(b) => Some(b.block.stmts.clone()),
            other => Some(vec![syn::Stmt::Expr(other.clone(), None)]),
        },
        syn::Expr::Block(b) => Some(b.block.stmts.clone()),
        syn::Expr::Paren(p) => closure_or_block_stmts(&p.expr),
        _ => None,
    }
}

/// If `it` is a LAZY/deferred static whose init has a walkable thunk body, return `(static_name, body)`.
/// Covers the four idioms:
///   - `static X: Lazy<_> = Lazy::new(|| ..)` / `LazyLock` / `LazyCell` — an `Item::Static`/`Item::Const`
///     whose init expr is a lazy-container `new` call (handles the `?`-free common form);
///   - `lazy_static! { static ref X: T = effectful(); }` — an `Item::Macro` (`lazy_static`), body parsed;
///   - `thread_local! { static T: Ty = effectful(); }` — an `Item::Macro` (`thread_local`), body parsed.
///
/// A PURE init is still returned (its synthetic unit will simply carry no effect) — purity is decided by
/// the classifier downstream, NOT here; returning it unconditionally is what keeps the keying per-static.
fn lazy_static_unit(it: &syn::Item) -> Option<(String, Vec<syn::Stmt>)> {
    match it {
        syn::Item::Static(s) => {
            let syn::Expr::Call(call) = &*s.expr else { return None };
            let body = lazy_init_body(call)?;
            Some((s.ident.to_string(), body))
        }
        // A `const X: Lazy<_> = Lazy::new(|| ..)` is unusual but legal and behaves identically.
        syn::Item::Const(c) => {
            let syn::Expr::Call(call) = &*c.expr else { return None };
            let body = lazy_init_body(call)?;
            Some((c.ident.to_string(), body))
        }
        syn::Item::Macro(m) => {
            let mname = path_to_string(&m.mac.path);
            let mname = mname.rsplit("::").next().unwrap_or(&mname);
            match mname {
                "lazy_static" => lazy_static_macro_body(&m.mac.tokens),
                "thread_local" => thread_local_macro_body(&m.mac.tokens),
                _ => None,
            }
        }
        _ => None,
    }
}

/// Parse a `lazy_static! { static ref NAME: T = EXPR; }` body: the init EXPR runs lazily on first deref,
/// so its effects are deferred exactly like `Lazy::new`. Returns `(NAME, [EXPR;])`. Single-static bodies
/// are the dominant form; a multi-static block parses the FIRST (a rare multi-static `lazy_static!` is
/// under-approximated to its first entry — honest, never fabricated). Parsing failure → skip (only adds
/// visibility, never breaks).
fn lazy_static_macro_body(tokens: &proc_macro2::TokenStream) -> Option<(String, Vec<syn::Stmt>)> {
    syn::parse2::<LazyStaticDecl>(tokens.clone())
        .ok()
        .map(|d| (d.name, vec![syn::Stmt::Expr(d.init, None)]))
}

/// Parse a `thread_local! { static NAME: Ty = EXPR; }` body the same way — the per-thread init EXPR runs
/// on first `.with(..)`, a deferred thunk. Returns `(NAME, [EXPR;])`.
fn thread_local_macro_body(tokens: &proc_macro2::TokenStream) -> Option<(String, Vec<syn::Stmt>)> {
    syn::parse2::<ThreadLocalDecl>(tokens.clone())
        .ok()
        .map(|d| (d.name, vec![syn::Stmt::Expr(d.init, None)]))
}

/// `[pub] static [ref] NAME: T = INIT;` — the single-static shape inside `lazy_static!`. We tolerate a
/// leading visibility and the `ref` keyword, take the NAME, skip the `: T`, and parse the `= INIT` expr.
struct LazyStaticDecl {
    name: String,
    init: syn::Expr,
}
impl syn::parse::Parse for LazyStaticDecl {
    fn parse(input: syn::parse::ParseStream) -> syn::Result<Self> {
        let _attrs = input.call(syn::Attribute::parse_outer)?;
        let _vis: syn::Visibility = input.parse()?;
        let _static: syn::Token![static] = input.parse()?;
        // `lazy_static!` requires `ref`; tolerate its absence so the parser is liberal.
        if input.peek(syn::Token![ref]) {
            let _ref: syn::Token![ref] = input.parse()?;
        }
        let name: syn::Ident = input.parse()?;
        let _colon: syn::Token![:] = input.parse()?;
        let _ty: syn::Type = input.parse()?;
        let _eq: syn::Token![=] = input.parse()?;
        let init: syn::Expr = input.parse()?;
        // `syn::parse2` requires the WHOLE stream consumed — drain the trailing `;` and any further
        // statics in a multi-static block. We keep only the FIRST static (the dominant single-static
        // form); a rare multi-static `lazy_static!` under-approximates to its first entry (honest).
        let _ = input.parse::<syn::Token![;]>();
        input.parse::<proc_macro2::TokenStream>()?;
        Ok(LazyStaticDecl { name: name.to_string(), init })
    }
}

/// `[pub] static NAME: Ty = INIT;` — the single-static shape inside `thread_local!` (no `ref`).
struct ThreadLocalDecl {
    name: String,
    init: syn::Expr,
}
impl syn::parse::Parse for ThreadLocalDecl {
    fn parse(input: syn::parse::ParseStream) -> syn::Result<Self> {
        let _attrs = input.call(syn::Attribute::parse_outer)?;
        let _vis: syn::Visibility = input.parse()?;
        let _static: syn::Token![static] = input.parse()?;
        let name: syn::Ident = input.parse()?;
        let _colon: syn::Token![:] = input.parse()?;
        let _ty: syn::Type = input.parse()?;
        let _eq: syn::Token![=] = input.parse()?;
        let init: syn::Expr = input.parse()?;
        // Drain the trailing `;` + any further per-thread statics (keep the first — honest under-approx).
        let _ = input.parse::<syn::Token![;]>();
        input.parse::<proc_macro2::TokenStream>()?;
        Ok(ThreadLocalDecl { name: name.to_string(), init })
    }
}

/// candor-SCAN ONLY: builder-ENTRY points whose effect the typed classifier deliberately defers to a
/// terminal VERB. `duct::cmd!(...).run()` is the canonical case — `cmd!`/`cmd` only BUILD an Expression;
/// the spawn is at `.run()`/`.read()`/`.start()`. The DEEP engine types the receiver and catches the verb,
/// so candor-classify keeps the entry pure for PRECISION (lib.rs duct rule + its `cmd → None` test). But
/// the SYNTACTIC scanner can't type a builder chain — least of all through the `cmd!` MACRO whose result
/// is opaque — so the verb's effect is dropped and the program reads silent-pure (a real under-report
/// found by the real-world dynamic oracle; the same macro-blindness family as the log/tracing macros).
/// Classify the ENTRY as the crate's whole effect: a safe OVER-approximation (candor's never-under-report
/// bias), scoped to candor-scan so the deep engine stays precise. Both engines still agree on the
/// function's effect when the builder is actually run (the overwhelmingly common case).
fn scan_builder_entry_effect(_cr: &str, path: &str) -> Option<&'static str> {
    // A DATA TABLE the real-world oracle DRIVES: builder-chain ENTRY paths whose effect candor-classify
    // keys on a TERMINAL VERB the syntactic scanner can't reach (it can't type the chain). Add a row when
    // the oracle proves a verb-keyed crate under-reports here. Entries are exact ENTRY paths — NOT the
    // terminal verbs (those stay candor-classify's job for the typed deep engine, which stays precise).
    const ENTRIES: &[(&str, &str)] = &[
        // duct — `cmd!`/`sh!`/`cmd`/`sh` build; `.run()/.read()/.start()` execute (found 2026-06-17).
        ("duct::cmd", "Exec"),
        ("duct::sh", "Exec"),
        // ureq — `get/post/...` build a Request; `.call()` performs the Net (found 2026-06-17, net_ureq).
        ("ureq::get", "Net"),
        ("ureq::post", "Net"),
        ("ureq::put", "Net"),
        ("ureq::delete", "Net"),
        ("ureq::head", "Net"),
        ("ureq::patch", "Net"),
        ("ureq::request", "Net"),
        // sqlx — `query*()` build; `.execute()/.fetch_*()` round-trip (found 2026-06-17, recall corpus).
        ("sqlx::query", "Db"),
        ("sqlx::query_as", "Db"),
        ("sqlx::query_scalar", "Db"),
        ("sqlx::query_with", "Db"),
        ("sqlx::query_as_with", "Db"),
        // diesel — `sql_query()` builds raw SQL; `.execute()/.load()` round-trips (found 2026-06-17).
        ("diesel::sql_query", "Db"),
    ];
    ENTRIES.iter().find(|(p, _)| *p == path).map(|(_, eff)| *eff)
}

/// A loaded sibling-report function: the effects + literal surfaces a consumer's call inherits.
#[derive(Clone, Default)]
struct DepFn {
    effects: Vec<&'static str>,
    hosts: Vec<String>,
    cmds: Vec<String>,
    paths: Vec<String>,
    tables: Vec<String>,
    /// Blind crates the dep fn (transitively) reaches — its report's `invisible`. Carried across the join
    /// so a consumer inherits the disclosure (sweep [8]): else a dep that floored an unmodeled crate read
    /// as plain pure at the chain boundary, dropping the per-fn honesty caveat.
    invisible: Vec<String>,
    /// Effects whose surface the dep fn left masking-incomplete — carried so a benign literal in the
    /// consumer can't mask the dep's invisible forbidden endpoint across the join (sweep [30]).
    incomplete: Vec<&'static str>,
}

/// The CANDOR_DEPS index: `crate#tail2` and `crate#leaf` keys (UNAMBIGUOUS only — a key two dep
/// functions share is dropped, the same under-report-don't-guess rule as `resolve_target`), plus
/// the covered crate set. A report whose producing version differs from this binary's is
/// DOWNGRADED to `Unknown` rather than silently trusted (spec §2.1).
#[derive(Default)]
struct DepIndex {
    by_key: HashMap<String, DepFn>,
    crates: std::collections::HashSet<String>,
}

fn load_dep_reports(spec: Option<&str>) -> DepIndex {
    let mut idx = DepIndex::default();
    let Some(spec) = spec else { return idx };
    // Canonical-path dedup: the same report loaded twice would self-collide on every key and be
    // dropped as 'ambiguous', silently killing the chain (review: --deps + CANDOR_DEPS=.candor/deps
    // — the natural combination — did exactly that). Directories walk RECURSIVELY: --deps writes
    // one subdirectory per name@version.
    let mut files: Vec<std::path::PathBuf> = Vec::new();
    let mut seen_files: std::collections::HashSet<std::path::PathBuf> = std::collections::HashSet::new();
    let mut push_file = |f: std::path::PathBuf, files: &mut Vec<std::path::PathBuf>| {
        let canon = std::fs::canonicalize(&f).unwrap_or(f);
        if seen_files.insert(canon.clone()) {
            files.push(canon);
        }
    };
    for tok in spec.split(':').filter(|t| !t.is_empty()) {
        let p = Path::new(tok);
        if p.is_dir() {
            for e in walkdir::WalkDir::new(p).into_iter().filter_map(Result::ok) {
                let f = e.path();
                let name = f.file_name().and_then(|n| n.to_str()).unwrap_or("");
                if f.is_file() && name.ends_with(".json") && !name.contains("callgraph") {
                    push_file(f.to_path_buf(), &mut files);
                }
            }
        } else if p.is_file() {
            push_file(p.to_path_buf(), &mut files);
        } else {
            eprintln!("candor-scan: CANDOR_DEPS entry not found, skipped: {tok}");
        }
    }
    let my_version = format!("scan-{}", env!("CARGO_PKG_VERSION"));
    let mut ambiguous: std::collections::HashSet<String> = std::collections::HashSet::new();
    for f in &files {
        let Ok(text) = std::fs::read_to_string(f) else {
            eprintln!("candor-scan: CANDOR_DEPS report unreadable, skipped: {}", f.display());
            continue;
        };
        let Ok(v) = serde_json::from_str::<serde_json::Value>(&text) else {
            eprintln!("candor-scan: CANDOR_DEPS report unparsable, skipped: {}", f.display());
            continue;
        };
        // v0.2+ envelope or the v0.1 bare array; the producing version comes from the envelope.
        let version = v.pointer("/candor/version").and_then(|x| x.as_str()).unwrap_or("");
        let stale = version != my_version;
        let Some(fns) = v.get("functions").and_then(|x| x.as_array()).or_else(|| v.as_array()) else { continue };
        // The crate a report covers: the entries' `hash` prefix (`crate#qual`), else the filename
        // (`report.<crate>.scan.json`).
        let file_crate = f
            .file_name()
            .and_then(|n| n.to_str())
            .and_then(|n| n.strip_suffix(".scan.json"))
            .and_then(|n| n.rsplit('.').next())
            .map(str::to_string);
        // Register the crate at FILE level: an all-pure crate's report has zero entries, and that
        // emptiness is its honest claim — the crate is covered, not invisible.
        if let Some(c) = &file_crate {
            idx.crates.insert(c.clone());
        }
        for e in fns {
            let Some(qual) = e.get("fn").and_then(|x| x.as_str()) else { continue };
            let krate = e
                .get("hash")
                .and_then(|x| x.as_str())
                .and_then(|h| h.split_once('#'))
                .map(|(c, _)| c.to_string())
                .or_else(|| file_crate.clone());
            let Some(krate) = krate else { continue };
            idx.crates.insert(krate.clone());
            let mut de = DepFn::default();
            if stale {
                de.effects.push("Unknown"); // §2.1: a different producer version is not trusted
            } else {
                for s in e.get("inferred").and_then(|x| x.as_array()).into_iter().flatten() {
                    if let Some(s) = s.as_str() {
                        // unknown vocabulary (a future spec's effect) is honestly Unknown
                        de.effects.push(candor_classify::cap_from_name(s).unwrap_or("Unknown"));
                    }
                }
                let strs = |k: &str| -> Vec<String> {
                    e.get(k)
                        .and_then(|x| x.as_array())
                        .into_iter()
                        .flatten()
                        .filter_map(|s| s.as_str().map(str::to_string))
                        .collect()
                };
                de.hosts = strs("hosts");
                de.cmds = strs("cmds");
                de.paths = strs("paths");
                de.tables = strs("tables");
                de.invisible = strs("invisible"); // sweep [8]: carry the blind-crate disclosure across the join
                // sweep [30]: carry masking-incompleteness (mapped to the static effect alphabet).
                for s in e.get("incomplete").and_then(|x| x.as_array()).into_iter().flatten() {
                    if let Some(eff) = s.as_str().and_then(candor_classify::cap_from_name) {
                        de.incomplete.push(eff);
                    }
                }
            }
            let mut keys = vec![format!("{krate}#{}", qual.rsplit("::").next().unwrap_or(qual))];
            if let Some(t2) = tail2(qual) {
                keys.push(format!("{krate}#{t2}"));
            }
            for k in keys {
                if ambiguous.contains(&k) {
                    continue;
                }
                // Not the `entry` API: a collision REMOVES the key (and remembers it as ambiguous),
                // so the present-vs-absent branches move `k` into different maps — clippy's map_entry
                // rewrite (insert-or-modify in place) can't express the remove-on-collision.
                #[allow(clippy::map_entry)]
                if idx.by_key.contains_key(&k) {
                    idx.by_key.remove(&k); // two dep fns share the key — drop it, never guess
                    ambiguous.insert(k);
                } else {
                    idx.by_key.insert(k, de.clone());
                }
            }
        }
    }
    idx
}

// ── shared Cargo.toml line primitives (line-based on purpose — no toml dependency) ────────────────
// The ONE place table-header and scalar parsing live, so a manifest-syntax quirk (`[ spaced ]`
// headers, a trailing `# comment`) is handled once across the three readers below rather than
// drifting between them.

/// A `[section]` header line → its inner name, surrounding spaces tolerated (`[ workspace ]` →
/// "workspace"); None for any non-header line.
fn toml_section(line: &str) -> Option<&str> {
    let l = line.trim();
    Some(l.strip_prefix('[')?.strip_suffix(']')?.trim())
}

/// A scalar `key = "value"` / `key = value` on this line — `key` matched as the WHOLE key (then `=`),
/// the value quote-trimmed and an out-of-quotes trailing `# comment` stripped. None if not this key.
fn toml_scalar<'a>(line: &'a str, key: &str) -> Option<&'a str> {
    let rest = line.trim().strip_prefix(key)?.trim_start().strip_prefix('=')?.trim();
    Some(if let Some(q) = rest.strip_prefix('"') {
        q.split('"').next().unwrap_or(q)
    } else {
        rest.split('#').next().unwrap_or(rest).trim()
    })
}

/// Dependency names declared by EVERY Cargo.toml under the scan root (a workspace's members each
/// declare their own — review: reading only the root manifest left member-declared deps invisible
/// to the κ ledger on the most common project layout), normalized to crate-root form (`-` -> `_`).
/// dev-/build-dependencies are the harness's and the build script's universe, not the crate's
/// runtime one — excluded, like tests/ and build.rs.
fn cargo_deps(dir: &str) -> (std::collections::HashSet<String>, HashMap<String, String>) {
    let mut out = std::collections::HashSet::new();
    let mut renames = HashMap::new();
    // Honour the SAME nested-package rule as the source walk (filter_entry above): a subdir with its
    // own Cargo.toml is a different package whose deps are ITS universe, not this crate's — scan_target
    // scans it separately. Without this, a nested fixture/path-dep's deps polluted the parent's κ
    // ledger (the source walk skips the nested sources, so the two had drifted out of agreement).
    for entry in walkdir::WalkDir::new(dir)
        .into_iter()
        .filter_entry(|e| {
            if e.depth() == 0 || !e.file_type().is_dir() {
                return true;
            }
            let name = e.file_name().to_str().unwrap_or("");
            if name == "target" || (name.starts_with('.') && name != "." && name != "..") {
                return false;
            }
            !e.path().join("Cargo.toml").is_file()
        })
        .filter_map(Result::ok)
    {
        let p = entry.path();
        if p.file_name().and_then(|n| n.to_str()) != Some("Cargo.toml") {
            continue;
        }
        if let Ok(text) = std::fs::read_to_string(p) {
            cargo_toml_deps(&text, &mut out, &mut renames);
        }
    }
    (out, renames)
}

/// One manifest's dependency names, all four header forms: `[dependencies]` /
/// `[workspace.dependencies]` / `[target.….dependencies]` sections, and the table-header
/// declarations `[dependencies.name]` / `[target.….dependencies.name]` (review: the old
/// `ends_with("dependencies]")` gate made the header-form branch unreachable — a table-header
/// dep was invisible to the ledger, execution-verified).
fn cargo_toml_deps(
    text: &str,
    out: &mut std::collections::HashSet<String>,
    renames: &mut HashMap<String, String>,
) {
    // A dependency RENAME (`tui-common = { package = "tb-tui-common" }`) means the manifest KEY is
    // what the code imports while the registry/report knows the REAL package — without the map,
    // --deps scanned the real crate and the join/ledger missed it under the key (found live on
    // ebman: tui_common stayed "invisible" with its report sitting right there).
    // Match `package` only as a KEY (`{ … package = "real" }`), not as a substring of a dependency
    // KEY (`my-package = "1.2"` previously parsed its own version as a rename target) or a value:
    // `package` must sit at a token boundary and be followed by `=`.
    let pkg_re = |l: &str| -> Option<String> {
        let bytes = l.as_bytes();
        let mut search = 0;
        while let Some(rel) = l[search..].find("package") {
            let i = search + rel;
            let boundary = i == 0 || matches!(bytes[i - 1], b'{' | b',' | b' ' | b'\t');
            if boundary {
                if let Some(rest) = l[i + "package".len()..].trim_start().strip_prefix('=') {
                    if let Some(rest) = rest.trim_start().strip_prefix('"') {
                        return rest.split('"').next().map(|s| s.replace('-', "_"));
                    }
                }
            }
            search = i + "package".len();
        }
        None
    };
    let mut in_deps = false;
    let mut header_key: Option<String> = None; // the `[dependencies.name]` we're inside, if any
    for line in text.lines() {
        let l = line.trim();
        if let Some(inner) = toml_section(line) {
            let harness = inner.contains("dev-dependencies") || inner.contains("build-dependencies");
            in_deps = !harness && (inner == "dependencies" || inner.ends_with(".dependencies"));
            header_key = None;
            if !harness && !in_deps {
                let name = inner
                    .rfind(".dependencies.")
                    .map(|i| &inner[i + ".dependencies.".len()..])
                    .or_else(|| inner.strip_prefix("dependencies."));
                if let Some(name) = name {
                    if !name.is_empty() && !name.contains('.') {
                        let key = name.trim_matches('"').replace('-', "_");
                        out.insert(key.clone());
                        header_key = Some(key);
                    }
                }
            }
            continue;
        }
        if l.is_empty() || l.starts_with('#') {
            continue;
        }
        // inside a `[dependencies.name]` table: a `package = "real"` line is the rename
        if let Some(key) = &header_key {
            if l.starts_with("package") {
                if let Some(real) = pkg_re(l) {
                    renames.insert(key.clone(), real);
                }
            }
            continue;
        }
        if !in_deps {
            continue;
        }
        if let Some(name) = l.split('=').next() {
            let name = name.trim().trim_matches('"');
            if !name.is_empty() {
                let key = name.replace('-', "_");
                // A rename only appears in an INLINE TABLE value (`key = { … package = "real" }`),
                // never as a bare `package = "0.1"` (which is a dependency NAMED package) — so search
                // only inside the braces.
                if let Some(brace) = l.find('{') {
                    if let Some(real) = pkg_re(&l[brace..]) {
                        if real != key {
                            renames.insert(key.clone(), real);
                        }
                    }
                }
                out.insert(key);
            }
        }
    }
}

/// The trait leaves of a type-param-bound list (`T: Store + Send` -> ["Store", "Send"]). Marker
/// bounds need no filtering here: a leaf only ever matters if it later matches a local trait or a
/// local impl, and nobody locally declares `trait Send`.
fn bound_leaves(bounds: &syn::punctuated::Punctuated<syn::TypeParamBound, syn::Token![+]>) -> Vec<String> {
    bounds
        .iter()
        .filter_map(|b| match b {
            syn::TypeParamBound::Trait(t) => t.path.segments.last().map(|s| s.ident.to_string()),
            _ => None,
        })
        .collect()
}

/// The trait bound leaves of a DISPATCH-typed `syn::Type`: `&dyn T`, `impl T`, `Box<dyn T>` (and the
/// other single-arg smart pointers), or a bare generic param `X` declared `X: T`. Returns empty for
/// a concrete type — `type_path` owns those.
fn trait_leaves(ty: &syn::Type, generic_bounds: &HashMap<String, Vec<String>>) -> Vec<String> {
    match ty {
        syn::Type::Reference(r) => trait_leaves(&r.elem, generic_bounds),
        syn::Type::Paren(p) => trait_leaves(&p.elem, generic_bounds),
        syn::Type::Group(g) => trait_leaves(&g.elem, generic_bounds),
        syn::Type::TraitObject(t) => bound_leaves(&t.bounds),
        syn::Type::ImplTrait(t) => bound_leaves(&t.bounds),
        syn::Type::Path(p) => {
            if let Some(id) = p.path.get_ident() {
                return generic_bounds.get(&id.to_string()).cloned().unwrap_or_default();
            }
            // Box<dyn T> / Rc / Arc / RefCell / Mutex / RwLock — peel the wrapper, recurse on the arg.
            let Some(seg) = p.path.segments.last() else { return Vec::new() };
            let wrapper = matches!(seg.ident.to_string().as_str(), "Box" | "Rc" | "Arc" | "RefCell" | "Mutex" | "RwLock" | "Cell");
            if !wrapper {
                return Vec::new();
            }
            let syn::PathArguments::AngleBracketed(args) = &seg.arguments else { return Vec::new() };
            args.args
                .iter()
                .find_map(|a| match a {
                    syn::GenericArgument::Type(inner) => Some(trait_leaves(inner, generic_bounds)),
                    _ => None,
                })
                .unwrap_or_default()
        }
        _ => Vec::new(),
    }
}

/// Whether a type is an INVOKABLE callback — a bare fn pointer (`fn()`), an `impl`/`dyn Fn[Mut/Once]`, a
/// generic param bound by `Fn*`, or a `Box`/`Rc`/`Arc<dyn Fn*>`. A value of such a type called as `cb()`
/// invokes a body the syntactic scan cannot see, so the enclosing fn can't be certified pure — it MUST
/// read `Unknown`, never silently pure (SPEC §4). The non-bare forms are exactly where `trait_leaves`
/// finds an `Fn`/`FnMut`/`FnOnce` leaf; `Type::BareFn` carries no trait so it's matched explicitly.
fn is_callable_type(ty: &syn::Type, generic_bounds: &HashMap<String, Vec<String>>) -> bool {
    match ty {
        syn::Type::BareFn(_) => true,
        syn::Type::Reference(r) => is_callable_type(&r.elem, generic_bounds),
        syn::Type::Paren(p) => is_callable_type(&p.elem, generic_bounds),
        syn::Type::Group(g) => is_callable_type(&g.elem, generic_bounds),
        _ => trait_leaves(ty, generic_bounds)
            .iter()
            .any(|l| matches!(l.as_str(), "Fn" | "FnMut" | "FnOnce")),
    }
}

/// The tail (value) expression of a block, if it ends in one (`{ … ; expr }` with no trailing `;`).
fn block_tail_expr(b: &syn::Block) -> Option<&syn::Expr> {
    match b.stmts.last() {
        Some(syn::Stmt::Expr(e, None)) => Some(e),
        _ => None,
    }
}

/// The params of a signature that are invokable callbacks (`is_callable_type`) — so `cb()` on one reads
/// the honest `Unknown` instead of being silently dropped as a phantom call to a free fn `cb`.
fn seed_fn_typed_vars(sig: &syn::Signature) -> std::collections::HashSet<String> {
    let gb = generic_bounds_of(sig);
    let mut s = std::collections::HashSet::new();
    for arg in &sig.inputs {
        if let syn::FnArg::Typed(pt) = arg {
            if let syn::Pat::Ident(id) = &*pt.pat {
                if is_callable_type(&pt.ty, &gb) {
                    s.insert(id.ident.to_string());
                }
            }
        }
    }
    s
}

/// `X -> [trait leaves]` for a signature's generic params, from both inline bounds (`fn f<X: Store>`)
/// and where-clauses (`where X: Store`).
fn generic_bounds_of(sig: &syn::Signature) -> HashMap<String, Vec<String>> {
    let mut m: HashMap<String, Vec<String>> = HashMap::new();
    for gp in &sig.generics.params {
        if let syn::GenericParam::Type(tp) = gp {
            let leaves = bound_leaves(&tp.bounds);
            if !leaves.is_empty() {
                m.entry(tp.ident.to_string()).or_default().extend(leaves);
            }
        }
    }
    if let Some(w) = &sig.generics.where_clause {
        for pred in &w.predicates {
            if let syn::WherePredicate::Type(pt) = pred {
                if let syn::Type::Path(p) = &pt.bounded_ty {
                    if let Some(id) = p.path.get_ident() {
                        let leaves = bound_leaves(&pt.bounds);
                        if !leaves.is_empty() {
                            m.entry(id.to_string()).or_default().extend(leaves);
                        }
                    }
                }
            }
        }
    }
    m
}

/// The (use-expanded) type path of a `syn::Type`, ignoring references and generic args:
/// `&reqwest::Client` -> `reqwest::Client`, `Pool<Postgres>` -> `sqlx::Pool` (via `uses`). `None` for
/// non-nameable types (impl Trait, tuples, …) where there's nothing to classify a method against.
fn type_path(ty: &syn::Type, uses: &HashMap<String, String>) -> Option<String> {
    match ty {
        syn::Type::Reference(r) => type_path(&r.elem, uses),
        syn::Type::Paren(p) => type_path(&p.elem, uses),
        syn::Type::Group(g) => type_path(&g.elem, uses),
        syn::Type::Path(p) => {
            // A transparent OWNED smart-pointer wrapper (`Box<T>`/`Arc<T>`/`Rc<T>`) auto-derefs:
            // `wrapper.method()` dispatches to `T`'s method. Peel to `T` so the method resolves against
            // the POINTEE — without this, a `.method()` on an `Arc<Inner>` field/local/param resolved to
            // "Arc" (no impl in crate) and the call was SILENTLY DROPPED, not even Unknown (a §4
            // under-report). Arc/Rc/Box receivers are ubiquitous in real Rust (found by corpus-testing
            // duct + crates: it dropped duct's whole public-API Exec). Mirrors elem_type's wrapper-peel.
            // Only these three (owned, Deref-to-T); Mutex/RefCell need an explicit .lock()/.borrow().
            if let Some(seg) = p.path.segments.last() {
                if matches!(seg.ident.to_string().as_str(), "Box" | "Arc" | "Rc") {
                    if let syn::PathArguments::AngleBracketed(args) = &seg.arguments {
                        if let Some(inner) = args.args.iter().find_map(|a| match a {
                            syn::GenericArgument::Type(t) => Some(t),
                            _ => None,
                        }) {
                            return type_path(inner, uses);
                        }
                    }
                }
            }
            Some(expand(&path_to_string(&p.path), uses))
        }
        _ => None,
    }
}

/// The ELEMENT type path of a COLLECTION `syn::Type`: `Vec<T>` / `&[T]` / `[T; N]` / `HashSet<T>` /
/// `BTreeSet<T>` / `VecDeque<T>` / `Box<[T]>` (and `Arc`/`Rc`-wrapped slices) -> the expanded type path
/// of `T` (via `uses`, like `type_path`). `None` for a non-collection type. Used to type a loop /
/// subscript / iterator-closure binding over a collection so the element's method calls classify —
/// without it, a very common Rust shape (`for c in xs { c.send() }`, `xs[0].send()`) dropped its
/// receiver to pure (a §4 under-report). Peels references/parens/groups around the collection.
fn elem_type(ty: &syn::Type, uses: &HashMap<String, String>) -> Option<String> {
    match ty {
        syn::Type::Reference(r) => elem_type(&r.elem, uses),
        syn::Type::Paren(p) => elem_type(&p.elem, uses),
        syn::Type::Group(g) => elem_type(&g.elem, uses),
        // `[T]` (slice) and `[T; N]` (array) — the element is the type directly.
        syn::Type::Slice(s) => type_path(&s.elem, uses),
        syn::Type::Array(a) => type_path(&a.elem, uses),
        syn::Type::Path(p) => {
            let seg = p.path.segments.last()?;
            let name = seg.ident.to_string();
            let syn::PathArguments::AngleBracketed(args) = &seg.arguments else { return None };
            let first_ty = args.args.iter().find_map(|a| match a {
                syn::GenericArgument::Type(t) => Some(t),
                _ => None,
            })?;
            match name.as_str() {
                // The single-type-arg sequence collections: their first generic arg IS the element.
                "Vec" | "VecDeque" | "HashSet" | "BTreeSet" | "ContiguousArray" | "BinaryHeap"
                | "LinkedList" => type_path(first_ty, uses),
                // Smart-pointer wrappers around a collection/slice (`Box<[T]>`, `Arc<Vec<T>>`,
                // `Rc<[T]>`) — peel one layer and recurse so the inner collection's element surfaces.
                "Box" | "Arc" | "Rc" => elem_type(first_ty, uses),
                _ => None,
            }
        }
        _ => None,
    }
}

/// The per-position type paths of a TUPLE `syn::Type` (`(Sender, usize)` -> `[Some("Sender"),
/// Some("usize")]`), peeling references/parens/groups. `None` for a non-tuple type — its elements
/// are tracked so a later `let (s, _) = pair` (where `pair: (Sender, usize)`) types each binding.
fn tuple_types(ty: &syn::Type, uses: &HashMap<String, String>) -> Option<Vec<Option<String>>> {
    match ty {
        syn::Type::Reference(r) => tuple_types(&r.elem, uses),
        syn::Type::Paren(p) => tuple_types(&p.elem, uses),
        syn::Type::Group(g) => tuple_types(&g.elem, uses),
        syn::Type::Tuple(t) if t.elems.len() >= 2 => {
            Some(t.elems.iter().map(|e| type_path(e, uses)).collect())
        }
        _ => None,
    }
}

/// Constructor-style associated function names: `let x = Foo::new(..)` (or `::connect().await?`) means
/// `x: Foo`. Conservative set of names that return `Self` (or `Result<Self>`), so the inferred type is
/// reliable. A non-constructor assoc call (`Foo::parse`) is NOT treated as producing a `Foo`.
fn is_ctor(name: &str) -> bool {
    matches!(
        name,
        "new" | "default" | "builder" | "with_capacity" | "connect" | "open" | "init" | "from"
            | "from_path" | "from_str" | "with_config" | "create"
    )
}

/// The type a call expression produces (peeling `&`/`(..)`/`?`/`.await`), by two routes:
///
/// 1. a constructor `Path::ctor(..)` -> the `Path` type (`reqwest::Client::new()` -> `reqwest::Client`);
/// 2. a LOCAL free function whose return type the pre-pass recorded (`create_pool()` -> `sqlx::Pool`).
///
/// Returns the expanded type path. `returns` is the crate-wide fn-leaf -> return-type index.
fn ctor_type(expr: &syn::Expr, uses: &HashMap<String, String>, returns: &ReturnIndex) -> Option<String> {
    match expr {
        syn::Expr::Reference(r) => ctor_type(&r.expr, uses, returns),
        syn::Expr::Paren(p) => ctor_type(&p.expr, uses, returns),
        syn::Expr::Try(t) => ctor_type(&t.expr, uses, returns),
        syn::Expr::Await(a) => ctor_type(&a.base, uses, returns),
        syn::Expr::Call(c) => {
            let syn::Expr::Path(p) = &*c.func else { return None };
            let full = path_to_string(&p.path);
            let leaf = full.rsplit("::").next().unwrap_or(&full);
            if let Some((ty, last)) = full.rsplit_once("::") {
                // `Type::ctor(..)` yields `Type` — but ONLY when the receiver is a TYPE, not a module.
                // Require the receiver's last segment to be type-like (UpperCamel), so `Client::new` →
                // Client but `serde_json::from_str` (module path) does NOT infer the module as a type.
                let ty_leaf = ty.rsplit("::").next().unwrap_or(ty);
                let type_like = ty_leaf.chars().next().is_some_and(|c| c.is_uppercase());
                if is_ctor(last) && type_like {
                    return Some(expand(ty, uses));
                }
            }
            // a local factory function call — its recorded (unambiguous) return type. The fn-typed
            // sentinel is NOT a nominal type (it types no var / receiver) — `expr_is_fn_typed` owns it.
            returns.get(leaf).filter(|t| *t != RET_FN_TYPED).cloned()
        }
        // `let s = S {..};` — a struct literal names its type directly.
        syn::Expr::Struct(s) => type_from_value_path(&path_to_string(&s.path), uses),
        // `let s = S;` — a UNIT-struct literal (or `let c = Color::Red;`, a unit enum variant, whose
        // value is typed as the ENUM). Gated by CamelCase so `let a = b;` (a variable copy) and
        // `let m = MAX_SIZE;` (a SCREAMING_SNAKE const) never mis-infer a type.
        syn::Expr::Path(p) => type_from_value_path(&path_to_string(&p.path), uses),
        _ => None,
    }
}

/// The type a VALUE path denotes, for `let` inference: `S` → `S`; `m::S` → `m::S`; `Color::Red`
/// (UpperCamel::UpperCamel = a unit enum variant) → `Color`. Only CamelCase leaves count as types —
/// a snake_case variable or SCREAMING_SNAKE const yields None (no inference; honest under-report).
fn type_from_value_path(full: &str, uses: &HashMap<String, String>) -> Option<String> {
    let camel = |s: &str| {
        // CamelCase = UpperCamel start, and either a single CHARACTER (`S`) or containing a lowercase
        // (distinguishes a type from a SCREAMING_SNAKE const). `chars().count()`, not `s.len()`: a
        // single-codepoint non-ASCII type ident (`struct É;`) is multi-BYTE and must still count as one
        // character. (/code-review.)
        let mut ch = s.chars();
        ch.next().is_some_and(|c| c.is_uppercase())
            && (s.chars().count() == 1 || s.chars().any(|c| c.is_lowercase()))
    };
    let segs: Vec<&str> = full.split("::").collect();
    let last = segs.last()?;
    if !camel(last) {
        return None;
    }
    // `Enum::Variant` — two trailing CamelCase segments: the VALUE's type is the enum (the penultimate).
    if segs.len() >= 2 && camel(segs[segs.len() - 2]) {
        return Some(expand(&segs[..segs.len() - 1].join("::"), uses));
    }
    Some(expand(full, uses))
}

/// Peel `Result<T, _>` / `Option<T>` / `io::Result<T>` to the inner `T` — a fallible constructor's
/// useful type is what it yields after `?`. Returns the inner type, or the type unchanged.
fn unwrap_result_option(ty: &syn::Type) -> &syn::Type {
    let syn::Type::Path(p) = ty else { return ty };
    let Some(seg) = p.path.segments.last() else { return ty };
    if matches!(seg.ident.to_string().as_str(), "Result" | "Option" | "IoResult") {
        if let syn::PathArguments::AngleBracketed(args) = &seg.arguments {
            if let Some(syn::GenericArgument::Type(inner)) = args.args.first() {
                return inner;
            }
        }
    }
    ty
}

/// The ERROR type LEAF of a fn's `Result<T, E>` / `io::Result<T>` return — the `?` operator's
/// `From::from` conversion TARGET. Returns the leaf of `E` when the output is a two-arg `Result<_, E>`
/// whose `E` is a concrete nominal path (a local error enum/struct). `None` for: a non-`Result` output,
/// a one-arg alias (`io::Result<T>`/`anyhow::Result<T>` carry no visible `E`), or a non-nominal/`Box<dyn
/// Error>` error (no single local type to convert to) — each the no-flood default for `?` (the edge is
/// only ever synthesized when `E` is also a LOCAL `impl From`, gated downstream in `charge_from`).
fn result_err_leaf(output: &syn::ReturnType, uses: &HashMap<String, String>) -> Option<String> {
    let syn::ReturnType::Type(_, ty) = output else { return None };
    let syn::Type::Path(p) = &**ty else { return None };
    let seg = p.path.segments.last()?;
    if seg.ident != "Result" {
        return None; // only the std two-arg Result exposes the error type positionally
    }
    let syn::PathArguments::AngleBracketed(args) = &seg.arguments else { return None };
    // The error is the SECOND generic arg. A one-arg `Result<T>` (an aliased Result) has no `E` here.
    let mut tys = args.args.iter().filter_map(|a| match a {
        syn::GenericArgument::Type(t) => Some(t),
        _ => None,
    });
    let _ok = tys.next()?;
    let err = tys.next()?;
    // The error type leaf — only a concrete nominal path (a local error type). `expand` then strips
    // module qualifiers; we keep just the leaf to match `trait_impls`/`by_tail2` keying.
    let expanded = type_path(err, uses)?;
    Some(expanded.rsplit("::").next().unwrap_or(&expanded).to_string())
}

/// Expand a call path against this file's `use` map: if the first segment is the last segment of some
/// `use a::b::Name`, replace it with the full `a::b::Name`. Turns `fs::read` → `std::fs::read`,
/// `Command::new` → `std::process::Command::new`. `crate`/`self`/`super` prefixes are stripped (local).
fn expand(path: &str, uses: &HashMap<String, String>) -> String {
    let mut segs: Vec<&str> = path.split("::").collect();
    // A path rooted at `crate`/`self`/`super` is EXPLICITLY crate-local — it is NOT subject to the file's
    // `use` aliases, so after stripping the prefix we return it as-is. (Re-applying `uses` here would let
    // a `use other::config;` import hijack a local `crate::config::load` call.)
    let rooted_local = matches!(segs.first().copied(), Some("crate" | "self" | "super"));
    while matches!(segs.first().copied(), Some("crate" | "self" | "super")) {
        segs.remove(0);
    }
    if segs.is_empty() {
        return path.to_string();
    }
    if !rooted_local {
        if let Some(full) = uses.get(segs[0]) {
            let rest = &segs[1..];
            return if rest.is_empty() { full.clone() } else { format!("{full}::{}", rest.join("::")) };
        }
    }
    segs.join("::")
}

fn first_str_lit(args: &syn::punctuated::Punctuated<syn::Expr, syn::token::Comma>) -> Option<String> {
    for a in args {
        if let syn::Expr::Lit(syn::ExprLit { lit: syn::Lit::Str(s), .. }) = a {
            let v = s.value();
            if !v.trim().is_empty() {
                return Some(v);
            }
        }
    }
    None
}

/// The bound identifier of a simple binding pattern: `c` / `mut c` / `&c` / `(c)` -> "c". `None` for a
/// destructuring/wildcard pattern (no single name to bind an element type to). Used for loop vars and
/// closure params.
fn single_pat_ident(pat: &syn::Pat) -> Option<String> {
    match pat {
        syn::Pat::Ident(id) => Some(id.ident.to_string()),
        syn::Pat::Reference(r) => single_pat_ident(&r.pat),
        syn::Pat::Paren(p) => single_pat_ident(&p.pat),
        // `|c: T|` — a type-annotated closure param; the inner pattern carries the name.
        syn::Pat::Type(t) => single_pat_ident(&t.pat),
        _ => None,
    }
}

/// A single-payload enum-variant match pattern `Variant(x)` / `Enum::Variant(x)` -> the bound name and
/// its payload type (looked up by the variant leaf in `enum_variants`; `None` type when the variant is
/// unknown/ambiguous — the caller still SCOPES it, clearing any stale binding so nothing leaks in).
/// `None` overall when the pattern isn't a single-field tuple-struct with a single-ident binding — a
/// multi-field or destructuring payload has no single receiver to type, an honest under-report.
fn arm_payload_binding(pat: &syn::Pat, enum_variants: &EnumVariantIndex) -> Option<(String, Option<String>)> {
    let ts = match pat {
        syn::Pat::TupleStruct(ts) => ts,
        // `Some(Variant(x))`-style nesting is rare; peel a reference/paren wrapper only.
        syn::Pat::Reference(r) => return arm_payload_binding(&r.pat, enum_variants),
        syn::Pat::Paren(p) => return arm_payload_binding(&p.pat, enum_variants),
        _ => return None,
    };
    if ts.elems.len() != 1 {
        return None; // multi-field variant — no single receiver to type
    }
    let name = single_pat_ident(ts.elems.first()?)?;
    let variant_leaf = ts.path.segments.last()?.ident.to_string();
    let ty = enum_variants.get(&variant_leaf).cloned();
    Some((name, ty))
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
                self.vars.get(&name).cloned()
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
    fn resolve_recv_traits(&self, expr: &syn::Expr) -> Vec<String> {
        // Hot-path guard: with no dispatch-typed vars or fields in scope (the overwhelmingly
        // common case), every lookup below is a guaranteed miss — skip the recursive walk.
        if self.trait_vars.is_empty() && self.trait_fields.is_empty() {
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
                        self.calls.push(Call { path, leaf, str_arg: first_str_lit(&node.args), typed: false, method, is_macro: false });
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
        let str_arg = first_str_lit(&node.args);
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
                if !lt.methods.contains(&leaf) {
                    continue; // supertrait/blanket call — not this trait's dispatch
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
        self.visit_expr(&node.receiver);
        if let Some(name) = closure_param {
            for a in &node.args {
                if let syn::Expr::Closure(cl) = a {
                    if cl.inputs.len() == 1 && single_pat_ident(cl.inputs.first().unwrap()).as_deref() == Some(name.as_str()) {
                        self.scoped_var(&name, elem_ty.clone(), |s| s.visit_expr(&cl.body));
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
            self.scoped_var(&name, elem, |s| s.visit_block(&node.body));
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

/// True if the item carries any `#[cfg(...)]` attribute (conditionally compiled).
fn has_cfg(attrs: &[syn::Attribute]) -> bool {
    attrs.iter().any(|a| a.path().is_ident("cfg"))
}

/// True if a file stem names a conventional `#[cfg(test)] mod` FILE module (`tests.rs`, `foo_tests.rs`,
/// `foo_test.rs`) — whose test-ness is declared at the `mod` site, invisible when walking the file.
fn is_test_file_stem(stem: &str) -> bool {
    stem == "tests" || stem == "test" || stem.ends_with("_tests") || stem.ends_with("_test")
}

/// True if a crate-root-RELATIVE path is the Cargo BUILD SCRIPT — i.e. exactly `build.rs` at the root.
/// It runs at COMPILE time, never the crate's runtime behaviour, so it's skipped. A nested `src/build.rs`
/// is NOT the build script — it's an ordinary source module that merely shares the name (git2's
/// `src/build.rs` is `RepoBuilder`, the clone/fetch NETWORK surface) and must be scanned.
fn is_build_script(rel: &std::path::Path) -> bool {
    rel == std::path::Path::new("build.rs")
}

/// True if `test` is POSITIVELY required by this cfg predicate node (recursing through `any`/`all` to
/// any depth, but NOT through `not` — a `test` under `not()` means "compile when NOT testing", i.e.
/// production code that must NOT be skipped).
fn cfg_meta_requires_test(m: &syn::meta::ParseNestedMeta) -> bool {
    if m.path.is_ident("test") {
        return true;
    }
    if m.path.is_ident("any") || m.path.is_ident("all") {
        let mut inner_test = false;
        // (the parse may error on a non-meta tail like a bare `not(unix)` group; `test` is recorded
        // before that, and the error is swallowed — we only care whether a positive `test` was seen.)
        let _ = m.parse_nested_meta(|inner| {
            if cfg_meta_requires_test(&inner) {
                inner_test = true;
            }
            Ok(())
        });
        return inner_test;
    }
    false // `not(...)`, `feature = "..."`, target predicates, etc.
}

/// True if an item carries a `#[cfg(...)]` that POSITIVELY requires `test` — a test-only module the
/// default scan skips, since its effects describe the crate's TESTS, not the crate. `#[cfg(not(test))]`
/// (production code) and `#[cfg(all(unix, not(test)))]` are correctly NOT treated as test.
fn is_cfg_test(attrs: &[syn::Attribute]) -> bool {
    attrs.iter().any(|a| {
        a.path().is_ident("cfg") && {
            let mut found = false;
            let _ = a.parse_nested_meta(|m| {
                if cfg_meta_requires_test(&m) {
                    found = true;
                }
                Ok(())
            });
            found
        }
    })
}

/// The crate's CFG-FEATURE picture, from Cargo.toml `[features]`: `active` = the transitive closure of
/// `default` (features enabling features); `declared` = every feature name that appears. A
/// `#[cfg(feature = "X")]` is KNOWN-FALSE when X is declared but not active (compiled out under a default
/// build), KNOWN-TRUE when active, and UNKNOWN when X isn't declared (a dependent could enable it). Read
/// once per scan from a OnceLock; empty ⇒ no feature info ⇒ nothing is skipped (no behaviour change).
type FeatureSets = (std::collections::HashSet<String>, std::collections::HashSet<String>);
static CFG_FEATURES: std::sync::OnceLock<std::sync::RwLock<FeatureSets>> = std::sync::OnceLock::new();

fn cfg_cell() -> &'static std::sync::RwLock<FeatureSets> {
    CFG_FEATURES.get_or_init(|| std::sync::RwLock::new((Default::default(), Default::default())))
}

/// Install the active/declared feature sets for the crate about to be scanned (called once per `scan_one`,
/// which runs sequentially per workspace member, before its parallel Pass B reads them).
fn set_cfg_features(f: FeatureSets) {
    *cfg_cell().write().unwrap() = f;
}

/// A snapshot of the active feature set, sorted — folded into the decl-index digest so the Pass-B cache
/// invalidates if the crate's enabled features change.
fn active_features_sorted() -> Vec<String> {
    let mut v: Vec<String> = cfg_cell().read().unwrap().0.iter().cloned().collect();
    v.sort();
    v
}

/// Pull every double-quoted token out of `s` into `out` (a manifest array's string entries).
fn push_quoted(s: &str, out: &mut Vec<String>) {
    let mut rest = s;
    while let Some(i) = rest.find('"') {
        rest = &rest[i + 1..];
        if let Some(j) = rest.find('"') {
            out.push(rest[..j].to_string());
            rest = &rest[j + 1..];
        } else {
            break;
        }
    }
}

/// Parse Cargo.toml `[features]` → (active, declared). `active` = closure of `default` over LOCAL feature
/// names (entries that are themselves feature keys); `dep:`/`?`/`crate/feat` entries enable dependencies,
/// not local features, so they don't expand the active SET (but they ARE recorded as declared if they name
/// a key). Line-based (no toml dep), tolerating multi-line arrays via bracket-depth tracking.
fn parse_features(root: &std::path::Path) -> (std::collections::HashSet<String>, std::collections::HashSet<String>) {
    use std::collections::{HashMap, HashSet};
    let txt = match std::fs::read_to_string(root.join("Cargo.toml")) {
        Ok(t) => t,
        Err(_) => return (HashSet::new(), HashSet::new()),
    };
    let mut feats: HashMap<String, Vec<String>> = HashMap::new();
    let mut in_features = false;
    let mut cur: Option<(String, Vec<String>)> = None; // (key, accumulating entries) for an open `[ … ]`
    for line in txt.lines() {
        if let Some((k, vals)) = cur.as_mut() {
            push_quoted(line, vals);
            if line.contains(']') {
                feats.insert(std::mem::take(k), std::mem::take(vals));
                cur = None;
            }
            continue;
        }
        let t = line.trim();
        if let Some(sec) = toml_section(line) {
            in_features = sec == "features";
            continue;
        }
        if !in_features || t.is_empty() || t.starts_with('#') {
            continue;
        }
        if let Some(eq) = t.find('=') {
            let key = t[..eq].trim().trim_matches('"').to_string();
            let rhs = t[eq + 1..].trim();
            if let Some(arr) = rhs.strip_prefix('[') {
                let mut vals = Vec::new();
                push_quoted(arr, &mut vals);
                if rhs.contains(']') {
                    feats.insert(key, vals); // single-line array
                } else {
                    cur = Some((key, vals)); // multi-line — keep accumulating
                }
            }
        }
    }
    let declared: HashSet<String> = feats.keys().cloned().collect();
    // active = transitive closure of `default` over entries that are themselves local feature keys.
    let mut active: HashSet<String> = HashSet::new();
    let mut stack: Vec<String> = feats.get("default").cloned().unwrap_or_default();
    while let Some(f) = stack.pop() {
        // a `dep:x` / `x/y` / `x?/y` entry enables a dependency, not a local feature — ignore for the SET.
        if f.contains(':') || f.contains('/') {
            continue;
        }
        if active.insert(f.clone()) {
            if let Some(next) = feats.get(&f) {
                stack.extend(next.iter().cloned());
            }
        }
    }
    (active, declared)
}

/// 3-valued cfg evaluation under the active feature set: `Some(true)` definitely compiled, `Some(false)`
/// definitely compiled OUT, `None` unknown (a predicate we can't resolve — target_os, an undeclared
/// feature, `test` left to is_cfg_test). Only `Some(false)` lets a caller SKIP. Conservative throughout:
/// anything unrecognised is `None` (kept). `feature = "X"`: active⇒true, declared-but-inactive⇒false,
/// undeclared⇒None. `not/all/any` fold with Kleene logic.
fn cfg_eval(m: &syn::meta::ParseNestedMeta, active: &std::collections::HashSet<String>,
            declared: &std::collections::HashSet<String>) -> Option<bool> {
    if m.path.is_ident("feature") {
        // `feature = "X"` → active⇒Some(true), declared-but-inactive⇒Some(false), undeclared⇒None.
        let v = m.value().ok().and_then(|v| v.parse::<syn::LitStr>().ok());
        return v.and_then(|lit| {
            let name = lit.value();
            if active.contains(&name) {
                Some(true)
            } else if declared.contains(&name) {
                Some(false)
            } else {
                None
            }
        });
    }
    if m.path.is_ident("not") {
        let mut inner: Option<bool> = None;
        let _ = m.parse_nested_meta(|n| { inner = cfg_eval(&n, active, declared); Ok(()) });
        return inner.map(|b| !b);
    }
    if m.path.is_ident("all") {
        // false if ANY child false; true only if ALL true; else None.
        let (mut any_false, mut all_true, mut saw) = (false, true, false);
        let _ = m.parse_nested_meta(|n| { saw = true; match cfg_eval(&n, active, declared) { Some(false) => any_false = true, Some(true) => {}, None => all_true = false }; Ok(()) });
        if any_false { return Some(false); }
        if saw && all_true { return Some(true); }
        return None;
    }
    if m.path.is_ident("any") {
        // true if ANY child true; false only if ALL false; else None.
        let (mut any_true, mut all_false, mut saw) = (false, true, false);
        let _ = m.parse_nested_meta(|n| { saw = true; match cfg_eval(&n, active, declared) { Some(true) => any_true = true, Some(false) => {}, None => all_false = false }; Ok(()) });
        if any_true { return Some(true); }
        if saw && all_false { return Some(false); }
        return None;
    }
    None // target_os/unix/windows/test/… — unknown to a default-feature scan; keep the item.
}

/// True if an item/stmt's `#[cfg(...)]` is KNOWN-FALSE under the active feature set (compiled out, so its
/// effects are not the crate's default behaviour). Multiple cfg attrs are AND-ed (any false ⇒ skip).
fn is_cfg_inactive(attrs: &[syn::Attribute]) -> bool {
    if !attrs.iter().any(|a| a.path().is_ident("cfg")) {
        return false; // fast path: no cfg attrs (the overwhelming majority of items/stmts)
    }
    let guard = cfg_cell().read().unwrap();
    let (active, declared) = &*guard;
    if declared.is_empty() {
        return false; // no [features] info — never skip
    }
    attrs.iter().any(|a| {
        a.path().is_ident("cfg") && {
            let mut verdict: Option<bool> = None;
            let _ = a.parse_nested_meta(|m| { verdict = cfg_eval(&m, active, declared); Ok(()) });
            verdict == Some(false)
        }
    })
}

/// The outer attributes of an expression that can appear in STATEMENT position carrying a `#[cfg(...)]`
/// (e.g. `#[cfg(feature="debug")] { … }`). Variants that can't front a cfg in a body return `&[]`.
fn expr_attrs(e: &syn::Expr) -> &[syn::Attribute] {
    match e {
        syn::Expr::Block(x) => &x.attrs,
        syn::Expr::If(x) => &x.attrs,
        syn::Expr::Match(x) => &x.attrs,
        syn::Expr::Unsafe(x) => &x.attrs,
        syn::Expr::ForLoop(x) => &x.attrs,
        syn::Expr::While(x) => &x.attrs,
        syn::Expr::Loop(x) => &x.attrs,
        syn::Expr::Call(x) => &x.attrs,
        syn::Expr::MethodCall(x) => &x.attrs,
        syn::Expr::Macro(x) => &x.attrs,
        syn::Expr::Async(x) => &x.attrs,
        syn::Expr::Const(x) => &x.attrs,
        _ => &[],
    }
}

/// True if a statement is compiled out under the active feature set — a `#[cfg(feature="X")]`-gated block
/// or stmt whose effects are NOT the crate's default behaviour, so the call-collector must not walk into it
/// (winnow's `trace_result` reaches `std::env::var("COLUMNS")` only through a `#[cfg(feature="debug")]` block).
fn stmt_cfg_inactive(stmt: &syn::Stmt) -> bool {
    match stmt {
        syn::Stmt::Local(l) => is_cfg_inactive(&l.attrs),
        syn::Stmt::Macro(m) => is_cfg_inactive(&m.attrs),
        syn::Stmt::Expr(e, _) => is_cfg_inactive(expr_attrs(e)),
        syn::Stmt::Item(_) => false, // a local item carries its own effects, not the enclosing fn's
    }
}

#[allow(clippy::too_many_arguments)]
fn scan_items(
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
                out.push(fninfo(&n, &qual(&n), &loc, &f.sig, &f.block, None, uses, fields, returns, traits, elems, lazy_statics));
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
                        out.push(fninfo(&n, &q, &loc, &m.sig, &m.block, tyname.as_deref(), uses, fields, returns, traits, elems, lazy_statics));
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
                    scan_items(inner, &sub, locs, loc_idx, include_tests, fields, returns, traits, elems, lazy_statics, &mut subuses, out);
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
                            Some(&tname), uses, fields, returns, traits, elems, lazy_statics));
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
                out.push(fninfo(&name, &q, &loc, &sig, &block, None, uses, fields, returns, traits, elems, lazy_statics));
            }
        }
    }
}

/// Whether a lazy-static synthetic unit will be EMITTED for `it` — a `static`/`const`/macro lazy with a
/// walkable thunk that is NOT `#[cfg(test)]`-gated (unless tests are included). The single source of
/// truth shared by `scan_items` (emits the unit) and `fn_locs` (emits its loc), so the two walks stay in
/// LOCKSTEP (the `debug_assert` count guard). Returns false for any non-lazy item.
fn lazy_unit_emitted(it: &syn::Item, include_tests: bool) -> bool {
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
fn next_loc(locs: &[String], loc_idx: &mut usize) -> String {
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
fn fn_locs(items: &[syn::Item], file: &str, include_tests: bool, out: &mut Vec<String>) {
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
fn seed_vars(sig: &syn::Signature, self_ty: Option<&str>, uses: &HashMap<String, String>) -> HashMap<String, String> {
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
fn seed_elem_of(
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
fn seed_trait_vars(sig: &syn::Signature) -> HashMap<String, Vec<String>> {
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
fn fninfo(
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
) -> FnInfo {
    // Function-LOCAL `use` statements (`fn f() { use rustix::time::clock_settime; … }`) are body
    // STATEMENTS, not module items, so the module-level use map misses them — every call they import then
    // fails to resolve to its crate and is under-reported (found on coreutils `date`: its rustix clock
    // read is imported by a fn-local `use`). Merge them in. (Top-level body stmts — the overwhelmingly
    // common placement; a `use` buried in a nested block is rare and left to the module fallback.)
    let mut local_uses = HashMap::new();
    for stmt in &block.stmts {
        if let syn::Stmt::Item(syn::Item::Use(u)) = stmt {
            collect_use(&u.tree, String::new(), &mut local_uses);
        }
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
fn record_return(
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
fn collect_decls(
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
) {
    for it in items {
        if let syn::Item::Use(u) = it {
            collect_use(&u.tree, String::new(), uses);
        }
    }
    let no_generics = HashMap::new();
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
        match it {
            syn::Item::Struct(s) => {
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
                                let leaves = trait_leaves(&f.ty, &no_generics);
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
                    collect_decls(inner, include_tests, &mut subuses, fields, field_elem, rets, enum_tmp, trait_impls, local_traits, trait_fields, prim_aliases, extern_fns, drop_types, deref_target, lazy_statics);
                }
            }
            _ => {}
        }
    }
}

fn impl_type_name(ty: &syn::Type) -> Option<String> {
    if let syn::Type::Path(p) = ty {
        return p.path.segments.last().map(|s| s.ident.to_string());
    }
    None
}

/// A NON-NOMINAL type: one with no user-definable inherent/trait impl that a local `Alias::method()` call
/// could resolve to — an array/slice/tuple/pointer/reference/fn type, or a bare built-in primitive path
/// (`u8`/`usize`/`bool`/…). A `type Alias = <non-nominal>` therefore can't legitimately link a
/// `Alias::assoc()` call to a same-named local STRUCT's associated fn (see the `prim_aliases` use).
fn is_non_nominal_type(ty: &syn::Type) -> bool {
    match ty {
        syn::Type::Array(_) | syn::Type::Slice(_) | syn::Type::Tuple(_)
        | syn::Type::Ptr(_) | syn::Type::Reference(_) | syn::Type::BareFn(_) => true,
        syn::Type::Path(p) if p.qself.is_none() && p.path.segments.len() == 1 => {
            const PRIMS: &[&str] = &[
                "u8", "u16", "u32", "u64", "u128", "usize", "i8", "i16", "i32", "i64", "i128",
                "isize", "f32", "f64", "bool", "char", "str",
            ];
            let seg = &p.path.segments[0];
            matches!(seg.arguments, syn::PathArguments::None)
                && PRIMS.contains(&seg.ident.to_string().as_str())
        }
        _ => false,
    }
}

fn collect_use(tree: &syn::UseTree, prefix: String, out: &mut HashMap<String, String>) {
    let join = |p: &str, s: &str| if p.is_empty() { s.to_string() } else { format!("{p}::{s}") };
    match tree {
        syn::UseTree::Path(p) => collect_use(&p.tree, join(&prefix, &p.ident.to_string()), out),
        syn::UseTree::Name(n) => {
            let id = n.ident.to_string();
            if id == "self" {
                // `use a::b::{self, ..}` imports the MODULE `b` itself under name `b` → map `b -> a::b`
                // so a later `b::func()` resolves. Without this, `self` was mapped uselessly as
                // `b::self` and the module alias was lost. (Found on coreutils `ls`: `use std::fs::{self,
                // Metadata}` then `fs::read_dir` was unresolved → a file lister reporting ZERO Fs.)
                if let Some(last) = prefix.rsplit("::").next() {
                    out.insert(last.to_string(), prefix.clone());
                }
            } else {
                out.insert(id.clone(), join(&prefix, &id));
            }
        }
        syn::UseTree::Rename(r) => {
            out.insert(r.rename.to_string(), join(&prefix, &r.ident.to_string()));
        }
        syn::UseTree::Group(g) => {
            for t in &g.items {
                collect_use(t, prefix.clone(), out);
            }
        }
        syn::UseTree::Glob(_) => {}
    }
}

/// Module path implied by a file's location under `src/` (root files → ""; `foo.rs`/`foo/mod.rs` →
/// "foo"; `foo/bar.rs` → "foo::bar"). Best-effort mirror of file-based module resolution.
fn module_path(rel: &Path) -> String {
    let mut comps: Vec<String> =
        rel.components().filter_map(|c| c.as_os_str().to_str().map(String::from)).collect();
    // Anchor at the LAST `src/` component, not just a leading one. A workspace member's code lives at
    // `crates/<name>/src/…`, so the module path is what FOLLOWS that `src` — otherwise the filesystem path
    // from the scan root mangles `crates/cli/src/decompress.rs` into `crates::cli::src::decompress`, which
    // ALSO breaks intra-crate call resolution (call sites use the real module path, not the dir path).
    // Found scanning ripgrep's workspace root — every name came out `crates::…::src::…` and `main` was lost.
    if let Some(i) = comps.iter().rposition(|c| c == "src") {
        comps.drain(..=i);
    }
    if let Some(last) = comps.last() {
        let stem = last.trim_end_matches(".rs").to_string();
        if stem == "lib" || stem == "main" || stem == "mod" {
            comps.pop();
        } else {
            // A dotted file stem encodes a NESTED module path — the tonic / prost gRPC convention names
            // a file `envoy.service.accesslog.v3.rs` for the module `envoy::service::accesslog::v3`. Split
            // on `.` so the qualified name is `::`-separated, not one ugly dotted segment.
            let parts: Vec<String> = stem.split('.').map(String::from).collect();
            comps.pop();
            comps.extend(parts);
        }
    }
    comps.join("::")
}

// ── INCREMENTAL SCAN CACHE ────────────────────────────────────────────────────────────────────────
//
// The biggest clock-time lever for the agent edit-loop (edit one file → re-query) is to STOP re-parsing
// the whole crate every scan: `syn::parse_file` is ~77% of wall-clock, and an unchanged file's parse is
// pure waste. This cache skips the parse (and the per-file Pass A / Pass B derivation) for files whose
// content hasn't changed — opt-in via `--incremental`.
//
// CORRECTNESS IS THE WHOLE JOB. An incremental scan MUST produce a report BYTE-FOR-BYTE IDENTICAL to a
// full scan-from-scratch for ANY sequence of edits. The invalidation model:
//
//   * PARSE + Pass A (`collect_decls`: a file's struct fields, enum variants, trait impls, return
//     types) depend ONLY on that file's bytes → cacheable by CONTENT HASH alone.
//   * Pass B (`CallCollector` → each fn's `Call`s) consults the WHOLE-CRATE merged decl index (a struct
//     field added in file Y changes a method-call resolution in unchanged file X). So a file's FnInfos
//     are valid to reuse only when BOTH its content_hash matches AND the merged decl index is unchanged
//     — gated on a canonical DECL_INDEX_HASH stored beside the cached FnInfos.
//
// A body-only edit leaves the decl index unchanged → every other file reuses its FnInfos (and its parse).
// A decl-changing edit bumps the decl index hash → every file re-runs Pass B (still cheap; the parse of
// unchanged files is STILL reused). Either way the assembled FnInfo set is identical to a from-scratch
// run, so the downstream classify/resolve/propagate (deliberately re-run in full every scan — it is the
// cheap, non-parse remainder) produces a byte-identical report. The classify stage is NOT cached: it
// reads no file, only the in-memory FnInfo set + the merged indexes, so re-deriving it is correct by
// construction and far simpler to keep sound than a third cache layer.
//
// VERSIONING: every cache file carries CACHE_SCHEMA (scanner version + format rev + include-tests). A
// mismatch invalidates the entry, so a candor-scan upgrade or a classifier-rules change can never serve
// stale results. A deleted file's entry is simply never consulted (we key by the CURRENT path set) and
// is pruned. A new file has no entry → it parses + derives + caches transparently.

thread_local! {
    /// Whether `--incremental` was passed (set once in `main`). Thread-local rather than a parameter
    /// so the cache opt-in reaches `scan_one` without rewiring `scan_target`/`run_with_deps`; the
    /// process is single-threaded by the time `scan_one` runs (rayon is used only inside the parse).
    static INCREMENTAL: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// The cache-format identity. Bump the trailing rev whenever the cached representation OR any analysis
/// that feeds it changes; the embedded scanner version + include-tests flag make a binary upgrade or a
/// scope change invalidate every entry automatically. A mismatch on read = full re-derivation.
fn cache_schema(include_tests: bool) -> String {
    format!("scan-{}/rev6/tests={}", env!("CARGO_PKG_VERSION"), include_tests)
}

/// A stable 64-bit FNV-1a content hash, hex — no extra dependency, deterministic across runs and hosts
/// (unlike `DefaultHasher`, which is randomized). Used for both file content and the canonical merged
/// decl-index digest, so the cache key never depends on process-random state.
fn fnv1a(bytes: &[u8]) -> String {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for &b in bytes {
        h ^= b as u64;
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    }
    format!("{h:016x}")
}

/// One source file's Pass A contribution, in ISOLATION (collected against fresh per-file maps), so it
/// can be cached by content hash and re-merged into the crate-wide index without re-parsing. Every map
/// here is exactly what `collect_decls` would have written for this one file's items. The merge
/// (`merge_decls`) replays the original accumulation semantics in WALK ORDER, so the assembled crate
/// index is byte-identical to the sequential pass — this equivalence is the cache's correctness linchpin.
#[derive(Default, Clone, serde::Serialize, serde::Deserialize)]
struct FileDecls {
    fields: FieldIndex,
    field_elem: FieldElemIndex,
    /// `leaf -> Some(ty)` or `None` (this file alone already saw conflicting return types for the leaf).
    rets: HashMap<String, Option<String>>,
    enum_tmp: HashMap<String, Option<String>>,
    trait_impls: TraitImplIndex,
    /// `trait leaf -> (decl count in this file, declared method names)` — `LocalTrait` flattened for serde.
    trait_decls: HashMap<String, (usize, Vec<String>)>,
    trait_fields: TraitFieldIndex,
    /// names aliased to a non-nominal type (`type Inner = [u8; N]`) — resolution skips local `Inner::assoc`.
    prim_aliases: Vec<String>,
    /// fn names declared in an `extern` block — a call to one is an FFI boundary → DISCLOSE Unknown.
    extern_fns: Vec<String>,
    /// local type leaves with a local `impl Drop` — a fn binding such a value inherits the drop body.
    drop_types: Vec<String>,
    /// local type leaf -> Deref Target leaf (`impl Deref for T { type Target = U }`) — `t.method()`
    /// auto-derefs to `U::method` when T declares no `method`.
    #[serde(default)]
    deref_target: HashMap<String, String>,
    /// LAZY/deferred static NAMES in this file (`Lazy`/`LazyLock`/`LazyCell`, `lazy_static!`,
    /// `thread_local!`) — a fn naming one of these FORCES its deferred init unit (`<lazy>::NAME`).
    #[serde(default)]
    lazy_statics: Vec<String>,
}

/// Collect ONE file's Pass A decls in isolation (the per-file input to `merge_decls`).
fn file_decls(items: &[syn::Item], include_tests: bool) -> FileDecls {
    let mut uses = HashMap::new();
    let mut fields = HashMap::new();
    let mut field_elem = HashMap::new();
    let mut rets = HashMap::new();
    let mut enum_tmp = HashMap::new();
    let mut trait_impls = HashMap::new();
    let mut trait_decls: HashMap<String, LocalTrait> = HashMap::new();
    let mut trait_fields = HashMap::new();
    let mut prim_aliases = std::collections::HashSet::new();
    let mut extern_fns = std::collections::HashSet::new();
    let mut drop_types = std::collections::HashSet::new();
    let mut deref_target = HashMap::new();
    let mut lazy_statics = std::collections::HashSet::new();
    collect_decls(items, include_tests, &mut uses, &mut fields, &mut field_elem, &mut rets,
                  &mut enum_tmp, &mut trait_impls, &mut trait_decls, &mut trait_fields, &mut prim_aliases,
                  &mut extern_fns, &mut drop_types, &mut deref_target, &mut lazy_statics);
    FileDecls {
        fields,
        field_elem,
        rets,
        enum_tmp,
        trait_impls,
        trait_decls: trait_decls
            .into_iter()
            .map(|(k, v)| (k, (v.count, v.methods.into_iter().collect())))
            .collect(),
        trait_fields,
        prim_aliases: prim_aliases.into_iter().collect(),
        extern_fns: extern_fns.into_iter().collect(),
        drop_types: drop_types.into_iter().collect(),
        deref_target,
        lazy_statics: lazy_statics.into_iter().collect(),
    }
}

/// The assembled crate-wide decl index (Pass A output), ready for Pass B — exactly the seven structures
/// `scan_one` built inline before, now produced from per-file `FileDecls` so unchanged files contribute
/// from cache. `rets`/`enum_tmp` keep the `Option` ambiguity marker until the caller filters them.
#[derive(Default)]
struct MergedDecls {
    fields: FieldIndex,
    field_elem: FieldElemIndex,
    rets: HashMap<String, Option<String>>,
    enum_tmp: HashMap<String, Option<String>>,
    trait_impls: TraitImplIndex,
    trait_decls: HashMap<String, LocalTrait>,
    trait_fields: TraitFieldIndex,
    prim_aliases: std::collections::HashSet<String>,
    extern_fns: std::collections::HashSet<String>,
    drop_types: std::collections::HashSet<String>,
    deref_target: HashMap<String, String>,
    lazy_statics: std::collections::HashSet<String>,
}

/// Merge one file's `FileDecls` into the crate accumulator, replaying EXACTLY the accumulation semantics
/// `collect_decls` used when it wrote a shared map directly — so calling this over the per-file decls in
/// WALK ORDER yields a result byte-identical to the old sequential `collect_decls` loop:
///   * `fields`/`field_elem`/`trait_fields`: nested `insert` (last writer in walk order wins) — same as
///     the original `entry().or_default().insert(..)`.
///   * `rets`/`enum_tmp`: the `record_return` ambiguity rule — a leaf seen with two DIFFERENT types (or
///     already `None` in any contributor) collapses to `None`. Order-independent in result.
///   * `trait_impls`: append in walk order (the Vec's order is preserved exactly as the original push).
///   * `trait_decls`: sum counts, union method names (commutative).
fn merge_decls(acc: &mut MergedDecls, fd: &FileDecls) {
    for (s, fmap) in &fd.fields {
        let e = acc.fields.entry(s.clone()).or_default();
        for (k, v) in fmap {
            e.insert(k.clone(), v.clone());
        }
    }
    for (s, fmap) in &fd.field_elem {
        let e = acc.field_elem.entry(s.clone()).or_default();
        for (k, v) in fmap {
            e.insert(k.clone(), v.clone());
        }
    }
    let merge_amb = |dst: &mut HashMap<String, Option<String>>, src: &HashMap<String, Option<String>>| {
        for (leaf, val) in src {
            match val {
                None => {
                    dst.insert(leaf.clone(), None); // contributor already ambiguous → ambiguous
                }
                Some(tp) => match dst.get(leaf) {
                    None => {
                        dst.insert(leaf.clone(), Some(tp.clone()));
                    }
                    Some(Some(prev)) if prev != tp => {
                        dst.insert(leaf.clone(), None); // conflicting types — drop
                    }
                    Some(Some(_)) => {} // same type — keep
                    Some(None) => {}    // already ambiguous — stays
                },
            }
        }
    };
    merge_amb(&mut acc.rets, &fd.rets);
    merge_amb(&mut acc.enum_tmp, &fd.enum_tmp);
    for (tr, tys) in &fd.trait_impls {
        acc.trait_impls.entry(tr.clone()).or_default().extend(tys.iter().cloned());
    }
    for (tr, (count, methods)) in &fd.trait_decls {
        let e = acc.trait_decls.entry(tr.clone()).or_default();
        e.count += count;
        for m in methods {
            e.methods.insert(m.clone());
        }
    }
    for (s, fmap) in &fd.trait_fields {
        let e = acc.trait_fields.entry(s.clone()).or_default();
        for (k, v) in fmap {
            e.insert(k.clone(), v.clone());
        }
    }
    for a in &fd.prim_aliases {
        acc.prim_aliases.insert(a.clone()); // set union — order-independent
    }
    for n in &fd.extern_fns {
        acc.extern_fns.insert(n.clone()); // set union — order-independent
    }
    for n in &fd.drop_types {
        acc.drop_types.insert(n.clone()); // set union — order-independent
    }
    for (k, v) in &fd.deref_target {
        acc.deref_target.insert(k.clone(), v.clone()); // last-writer-wins (one Deref impl per type)
    }
    for n in &fd.lazy_statics {
        acc.lazy_statics.insert(n.clone()); // set union — order-independent
    }
}

/// A CANONICAL, order-stable digest of the merged decl index — the gate that decides whether a cached
/// file's FnInfos (Pass B output) are still valid. Every map is rendered with SORTED keys (and sorted
/// inner keys / value lists) so the digest depends only on the index's CONTENT, never on `HashMap`
/// iteration order or which files happened to contribute. If this digest is unchanged, every fn's
/// `Call`s resolve identically, so a cached FnInfo set is sound to reuse; if it moves, all files re-run
/// Pass B. `trait_impls`'s Vec order is load-bearing (CHA), so it is hashed in order, NOT sorted.
fn decl_index_digest(m: &MergedDecls) -> String {
    let mut s = String::new();
    let nested = |s: &mut String, tag: &str, map: &HashMap<String, HashMap<String, String>>| {
        s.push_str(tag);
        let mut keys: Vec<&String> = map.keys().collect();
        keys.sort();
        for k in keys {
            s.push('|');
            s.push_str(k);
            let inner = &map[k];
            let mut ik: Vec<&String> = inner.keys().collect();
            ik.sort();
            for f in ik {
                s.push(';');
                s.push_str(f);
                s.push('=');
                s.push_str(&inner[f]);
            }
        }
        s.push('\n');
    };
    nested(&mut s, "fields", &m.fields);
    nested(&mut s, "field_elem", &m.field_elem);
    let amb = |s: &mut String, tag: &str, map: &HashMap<String, Option<String>>| {
        s.push_str(tag);
        let mut keys: Vec<&String> = map.keys().collect();
        keys.sort();
        for k in keys {
            s.push('|');
            s.push_str(k);
            s.push('=');
            s.push_str(map[k].as_deref().unwrap_or("\u{0}AMBIG"));
        }
        s.push('\n');
    };
    amb(&mut s, "rets", &m.rets);
    amb(&mut s, "enum", &m.enum_tmp);
    // trait_impls — Vec order is significant (CHA), hash in stored order, keys sorted.
    s.push_str("trait_impls");
    let mut tik: Vec<&String> = m.trait_impls.keys().collect();
    tik.sort();
    for k in tik {
        s.push('|');
        s.push_str(k);
        for ty in &m.trait_impls[k] {
            s.push(';');
            s.push_str(ty);
        }
    }
    s.push('\n');
    // trait_decls — count + sorted method names.
    s.push_str("trait_decls");
    let mut tdk: Vec<&String> = m.trait_decls.keys().collect();
    tdk.sort();
    for k in tdk {
        let lt = &m.trait_decls[k];
        s.push('|');
        s.push_str(k);
        s.push(':');
        s.push_str(&lt.count.to_string());
        let mut ms: Vec<&String> = lt.methods.iter().collect();
        ms.sort();
        for mname in ms {
            s.push(';');
            s.push_str(mname);
        }
    }
    s.push('\n');
    nested_tf(&mut s, &m.trait_fields);
    // prim_aliases — sorted set of non-nominal alias names (resolution skips local `Alias::assoc`).
    s.push_str("prim_aliases");
    let mut pak: Vec<&String> = m.prim_aliases.iter().collect();
    pak.sort();
    for a in pak {
        s.push('|');
        s.push_str(a);
    }
    s.push('\n');
    // extern_fns — sorted set of FFI-declared fn names (a call to one DISCLOSES Unknown).
    s.push_str("extern_fns");
    let mut efk: Vec<&String> = m.extern_fns.iter().collect();
    efk.sort();
    for a in efk {
        s.push('|');
        s.push_str(a);
    }
    s.push('\n');
    // drop_types — sorted set of local types with a local `impl Drop` (binding one adds the drop edge).
    s.push_str("drop_types");
    let mut dtk: Vec<&String> = m.drop_types.iter().collect();
    dtk.sort();
    for a in dtk {
        s.push('|');
        s.push_str(a);
    }
    s.push('\n');
    // lazy_statics — sorted set of LAZY/deferred static names (naming one adds a forcing edge to its
    // synthetic init unit). A change here re-resolves forcing sites, so it must invalidate cached FnInfos.
    s.push_str("lazy_statics");
    let mut lsk: Vec<&String> = m.lazy_statics.iter().collect();
    lsk.sort();
    for a in lsk {
        s.push('|');
        s.push_str(a);
    }
    s.push('\n');
    // active cfg-features — items behind an inactive feature are skipped in Pass B, so a change to the
    // crate's enabled features must invalidate the cached FnInfos (this digest gates that cache).
    s.push_str("features");
    for f in active_features_sorted() {
        s.push('|');
        s.push_str(&f);
    }
    s.push('\n');
    fnv1a(s.as_bytes())
}

/// `trait_fields` digest (`HashMap<String, HashMap<String, Vec<String>>>`) — sorted struct keys, sorted
/// field keys, bound-leaf lists in stored order (the bound order is what `resolve_recv_traits` returns).
fn nested_tf(s: &mut String, map: &TraitFieldIndex) {
    s.push_str("trait_fields");
    let mut keys: Vec<&String> = map.keys().collect();
    keys.sort();
    for k in keys {
        s.push('|');
        s.push_str(k);
        let inner = &map[k];
        let mut ik: Vec<&String> = inner.keys().collect();
        ik.sort();
        for f in ik {
            s.push(';');
            s.push_str(f);
            s.push('=');
            s.push_str(&inner[f].join(","));
        }
    }
    s.push('\n');
}

/// The cache entry for ONE source file: its content hash, its isolated Pass A decls, and (gated on the
/// decl-index digest captured when they were computed) its Pass B FnInfos. `fninfos` is reusable only
/// when BOTH `content_hash` matches the file on disk AND `decl_index_hash` matches the current merged
/// index; `decls` is reusable on `content_hash` alone.
#[derive(serde::Serialize, serde::Deserialize)]
struct FileCache {
    content_hash: String,
    decls: FileDecls,
    decl_index_hash: String,
    fninfos: Vec<FnInfo>,
}

/// The whole on-disk cache: one file (`<crate>/.candor/cache/scan-cache.json`) holding the schema id and
/// every source file's entry keyed by crate-relative path. A SINGLE consolidated file means one read +
/// one atomic write per scan instead of one syscall per source file (the per-file-file design spent ~19ms
/// just opening + parsing tokio's 337 entries). A schema mismatch discards the whole cache.
#[derive(serde::Serialize, serde::Deserialize)]
struct ScanCache {
    schema: String,
    files: HashMap<String, FileCache>,
}

fn main() {
    // Deeply-nested expressions / method chains recurse in syn's parser AND the single-threaded visitor
    // (Pass B) without depth limits; on the default ~8 MB stack a ~1000-deep file ABORTED the process
    // (SIGABRT) instead of degrading (adversarial review). Run the whole scan on a generous stack — and
    // give rayon's parse workers the same — so a pathological/generated file is handled, not a crash.
    // (A truly adversarial million-deep file still aborts; that's a DoS edge, not real code.)
    const BIG_STACK: usize = 256 * 1024 * 1024;
    rayon::ThreadPoolBuilder::new().stack_size(BIG_STACK).build_global().ok();
    let worker = std::thread::Builder::new()
        .stack_size(BIG_STACK)
        .spawn(scan_main)
        .expect("spawn scan worker thread");
    // scan_main drives its own exit codes via process::exit; a normal return → 0, a panic → 101.
    if worker.join().is_err() {
        std::process::exit(101);
    }
}

fn scan_main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let mut dir = ".".to_string();
    let mut prefix = String::new();
    let mut want_json = false;
    let mut include_tests = false;
    let mut policy_path: Option<String> = None;
    let mut gate_json_path: Option<String> = None;
    let mut deps_mode = false;
    let mut incremental = false;
    let mut it = args.iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "--out" => prefix = it.next().cloned().unwrap_or_default(),
            "--json" => want_json = true,
            "--include-tests" => include_tests = true,
            "--incremental" => incremental = true,
            "--policy" => {
                // A valueless trailing `--policy` (no path follows) must ERROR, not silently fall
                // back to no-gate — matching the strict posture of a set-but-unreadable policy.
                // Silently dropping the gate would let a violation ship under an intended-gated run.
                match it.next().cloned() {
                    Some(p) => policy_path = Some(p),
                    None => {
                        eprintln!("candor-scan: --policy requires a path argument");
                        std::process::exit(2);
                    }
                }
            }
            "--gate-json" => {
                // The structured gate verdict target (candor-spec §3.3). Valueless fails closed, like
                // --policy — a set-but-value-less gate flag must never silently drop its output.
                match it.next().cloned() {
                    Some(p) => gate_json_path = Some(p),
                    None => {
                        eprintln!("candor-scan: --gate-json requires a path argument");
                        std::process::exit(2);
                    }
                }
            }
            "--deps" => deps_mode = true,
            "-V" | "--version" => {
                // Two lines, fully OFFLINE: the installed build + the spec contract it speaks, then
                // the upgrade incantation. <spec> reuses candor_report::SPEC_VERSION — the same source
                // that stamps the report envelope's `spec` field, so the two can never drift.
                println!("candor-scan {} (spec {})", env!("CARGO_PKG_VERSION"), candor_report::SPEC_VERSION);
                println!("upgrade: cargo install candor-scan --force");
                return;
            }
            // The agent contract for THE INSTALLED VERSION, embedded at build time — doc and
            // binary cannot drift (the §2.1 version-trust rule applied to documentation). Agents
            // are told to run this instead of trusting a vendored/remote copy.
            "--agents" => {
                println!("<!-- candor-scan {} · the agent contract for this installed version -->", env!("CARGO_PKG_VERSION"));
                print!("{}", include_str!("../AGENTS.md"));
                return;
            }
            "-h" | "--help" => {
                println!("candor-scan {} — stable-Rust effect scanner (no nightly)", env!("CARGO_PKG_VERSION"));
                println!();
                println!("USAGE:  candor-scan [<dir>] [--out <prefix>] [--json] [--include-tests] [--policy <file>] [--gate-json <file>]");
                println!();
                println!("  <dir>             crate root to scan (default: .). A [workspace] root scans");
                println!("                    every member: one report per member under the one prefix.");
                println!("                    A nested dir with its own Cargo.toml is a different package");
                println!("                    and is never folded into the parent's report.");
                println!("  --out <prefix>    report path prefix (default: <dir>/.candor/report);");
                println!("                    writes <prefix>.<crate>.scan.json + a call-graph sidecar");
                println!("  --json            print the report to stdout instead of writing files");
                println!("  --include-tests   also scan tests/ benches/ examples/ and #[cfg(test)] modules");
                println!("                    (off by default → the report describes the crate, not its harness)");
                println!("  --incremental     reuse a per-file parse/decl cache under <dir>/.candor/cache so an");
                println!("                    edit-then-rescan skips re-parsing unchanged files (~7x on a one-file");
                println!("                    edit). Produces a BYTE-IDENTICAL report to a full scan; a candor-scan");
                println!("                    upgrade or a decl-changing edit invalidates the cache automatically.");
                println!("  --deps            scan the Cargo.lock dependency tree first (registry sources from");
                println!("                    ~/.cargo/registry/src) into <dir>/.candor/deps/, then scan <dir>");
                println!("                    CHAINED over those reports — effects cross every crate boundary");
                println!("                    without κ needing to know the crates.");
                println!("  --policy <file>   enforce a CANDOR_POLICY file (deny/pure/allow/forbid, spec §6.2)");
                println!("  --gate-json <f>   write the structured gate verdict {{ spec, ok, violations }} as JSON (spec §3.3)");
                println!("                    over this scan; exit 1 on violation. ADVISORY FLOOR: the syntactic");
                println!("                    backend under-reports, so a miss can pass — the nightly engine is");
                println!("                    the sound gate. (CANDOR_POLICY env is honoured when flag absent.)");
                println!();
                println!("  CANDOR_DEPS=<p:…> chain sibling reports (files or directories of *.json): an");
                println!("                    unclassified call into a crate a report covers inherits that");
                println!("                    function's effects + literal surfaces (spec §2). Scan the dep");
                println!("                    once, chain it everywhere; the κ ledger names what to scan next.");
                println!("  -V, --version     print the installed build + spec contract (offline) and the upgrade line");
                println!();
                println!("Syntactic, so it under-reports vs the full candor nightly lint (no Unknown). It never");
                println!("fabricates an effect. See https://github.com/tombaldwin/candor");
                return;
            }
            other => {
                // An unknown flag must FAIL, not become a path: an agent following a newer doc
                // against an older binary ran `candor-scan --agents` and scanned a directory
                // literally named `--agents`; a typo'd `--polcy` would silently drop the gate.
                if other.starts_with('-') {
                    eprintln!("candor-scan: unknown flag '{other}' (see --help)");
                    std::process::exit(2);
                }
                dir = a.clone();
            }
        }
    }
    // The policy source is resolved HERE, once (flag wins, CANDOR_POLICY env as fallback) — never
    // inside scan_one, so --deps dependency scans can't inherit the root gate via the env.
    let policy = policy_path.or_else(|| std::env::var("CANDOR_POLICY").ok());
    // The --gate-json target rides a global (like INCREMENTAL below) so it threads no ScanOpts. Members
    // RECORD violations (record_gate_violations); the verdict is written ONCE here after the whole scan —
    // per-member writes let a clean last member overwrite an earlier violator's verdict (ok:true vs exit 1).
    // Dependency scans under --deps run gate-free (policy=None in scan_one), so they record nothing.
    let _ = GATE_JSON_PATH.set(gate_json_path);
    if deps_mode {
        let code = run_with_deps(&dir, prefix, want_json, include_tests, policy);
        write_gate_json(code);
        std::process::exit(code);
    }
    // Incremental is OPT-IN and SAFE: a full scan (no flag) never reads the cache, and `--incremental`
    // with no/invalid cache transparently does a full scan + populates it (the gates downgrade any
    // stale entry to a re-derivation). The flag rides in a thread-local so it doesn't thread through
    // every signature between `main` and `scan_one` (scan_target/run_with_deps are unchanged).
    INCREMENTAL.with(|c| c.set(incremental));
    // Cross-crate report chaining (spec §2): CANDOR_DEPS names sibling reports (a `:`-separated
    // list of files and/or directories of *.json); an unclassified qualified call into a crate one
    // of them covers inherits that function's recorded effects + literal surfaces. The stable
    // scanner's half of the dep-scan story: scan the dep once, chain it everywhere.
    let deps_idx = load_dep_reports(std::env::var("CANDOR_DEPS").ok().as_deref());
    // scan_target handles both a single crate and a `[workspace]` root (one report per member under
    // one prefix — candor-query's multi-crate merge consumes them together; the policy gates each).
    let code = scan_target(&dir, prefix, want_json, include_tests, policy, &deps_idx);
    write_gate_json(code);
    std::process::exit(code);
}

/// Options for one crate scan. `policy` is RESOLVED by the caller (flag or CANDOR_POLICY env) —
/// scan_one itself never reads the env, so dependency scans under --deps can genuinely run
/// gate-free (review: the env fallback inside scan_one ran the root policy 328 times against
/// dependency internals). `quiet` suppresses the per-scan receipts (dep scans; the --deps summary
/// line speaks for them).
struct ScanOpts<'a> {
    prefix: String,
    want_json: bool,
    include_tests: bool,
    policy: Option<String>,
    quiet: bool,
    deps_idx: &'a DepIndex,
}

/// One crate scan, end to end (parse -> passes -> report -> receipt -> policy gate). Returns the
/// process exit code. Factored out of `main` so `--deps` can scan a dependency tree IN-PROCESS —
/// candor-scan's own self-gate (`deny Exec`) rightly forbids the spawn-yourself shortcut.
fn scan_one(dir: &str, opts: ScanOpts) -> (i32, Option<String>) {
    let ScanOpts { prefix, want_json, include_tests, policy: policy_path, quiet, deps_idx } = opts;
    let root = Path::new(dir);
    let crate_name = read_crate_name(root).unwrap_or_else(|| "crate".to_string());
    // Install this crate's cfg-feature picture (active = default closure, declared = all). A
    // `#[cfg(feature="X")]` compiled OUT under the default build is then skipped, so its effects don't
    // count as the crate's behaviour (winnow's debug-trace `std::env::var` fabricated Env). Set before the
    // parallel Pass B reads it; scan_one runs sequentially per workspace member, so members don't race.
    set_cfg_features(parse_features(root));

    // Parse every in-scope .rs file ONCE (syn parses are reused across both passes below). The walk +
    // path-shape filters run SEQUENTIALLY (cheap directory traversal, and the filter set is the report's
    // scope contract); the per-file READ + `syn::parse_file` — profiled at ~77% parse + ~19% I/O of
    // wall-clock, and embarrassingly parallel since each file parses independently — is fanned out across
    // cores with rayon below. ORDER IS PRESERVED: paths are collected in walk order, `par_iter().collect()`
    // writes each result back at its own index (completion order is irrelevant), and the post-filter of
    // read/parse failures keeps the survivors' relative order — so `parsed` is byte-identical to the old
    // sequential push, and the report's fn order (which derives from it) does not move.
    let mut paths: Vec<(std::path::PathBuf, String)> = Vec::new();
    for entry in walkdir::WalkDir::new(root)
        .into_iter()
        // A nested directory carrying its own Cargo.toml is a DIFFERENT package (Cargo's own
        // semantics) — folding its files into this crate collides same-named fns across packages
        // and cross-wires the merged call graph (the repo-root self-scan merged 194 eval-fixture
        // `main`s into one unit). It gets its own scan: workspace member, --deps, or directly.
        .filter_entry(|e| {
            if e.depth() == 0 || !e.file_type().is_dir() {
                return true;
            }
            // Prune build/tooling dirs by NAME first — cheap, and it skips DESCENT into huge `target/`
            // and `.git/` trees (the dominant cost on a warm checkout) before the per-dir Cargo.toml
            // stat. A name starting with `.` is a hidden tooling dir (`.git`/`.github`/`.cargo`/…).
            let name = e.file_name().to_str().unwrap_or("");
            if name == "target" || (name.starts_with('.') && name != "." && name != "..") {
                return false;
            }
            // A nested dir carrying its own Cargo.toml is a DIFFERENT package (Cargo's own semantics):
            // folding its files into this crate collides same-named fns across packages and cross-wires
            // the merged call graph. It gets its own scan (workspace member, --deps, or directly).
            !e.path().join("Cargo.toml").is_file()
        })
        .filter_map(Result::ok)
    {
        let p = entry.path();
        if !p.is_file() || p.extension().and_then(|e| e.to_str()) != Some("rs") {
            continue;
        }
        // All path-shape filters run on the path RELATIVE to the scan root — an absolute prefix can itself
        // contain `target`/`.cargo`/… (a vendored crate lives under `~/.cargo/registry/...`), which must
        // not trip them.
        let rel = p.strip_prefix(root).unwrap_or(p);
        // target/ build artifacts; hidden dirs (`.git`, `.github`, `.cargo`, …) holding tooling/CI scripts,
        // not library code (smol_str's `.github/ci.rs` otherwise reported a phantom `Exec`).
        if rel.components().any(|c| {
            c.as_os_str()
                .to_str()
                .is_some_and(|s| s == "target" || (s.starts_with('.') && s != "." && s != ".."))
        }) {
            continue;
        }
        // The Cargo BUILD SCRIPT is `<crate-root>/build.rs` — it runs at COMPILE time (ring's build.rs
        // execs nasm), never the crate's runtime behaviour, so skip it. But ONLY at the root: a nested
        // `src/build.rs` is an ordinary source module that merely shares the name (git2's `src/build.rs`
        // is `RepoBuilder` — the whole clone/fetch NETWORK surface), and dropping it silently under-reports
        // (an A/B found `git2::Repository::clone` reporting no `Net` because its module had vanished).
        if is_build_script(rel) {
            continue;
        }
        // Cargo's non-library compilation targets (tests/, benches/, examples/) — and the common nonstandard
        // singular `test/` tree (e.g. nix) — describe what the crate's HARNESS does (spawn a server, read
        // fixtures, seed RNG), not what the crate itself does. Scanning them conflates the two (redis's bench
        // harness alone showed Exec/Net/Fs/Env/Rand on 200+ fns). Skip by default; `--include-tests` keeps them.
        if !include_tests
            && rel.components().any(|c| {
                matches!(
                    c.as_os_str().to_str(),
                    Some("tests") | Some("test") | Some("benches") | Some("examples")
                )
            })
        {
            continue;
        }
        // A `#[cfg(test)] mod tests;` FILE module is invisible here — its test-ness is declared at the
        // `mod` site, not in the file — so a `tests.rs` / `*_tests.rs` / `*_test.rs` file's effects (a
        // seeded RNG, a temp file) would be mis-read as the crate's. By convention these stems are test
        // modules; skip them by default. (base64's `engine/tests.rs` otherwise reported a phantom `Rand`.)
        if !include_tests {
            if let Some(stem) = p.file_stem().and_then(|s| s.to_str()) {
                if is_test_file_stem(stem) {
                    continue;
                }
            }
        }
        paths.push((p.to_path_buf(), rel.to_string_lossy().into_owned()));
    }

    // ── PARSE + Pass A + Pass B, with an OPTIONAL per-file cache (`--incremental`) ──────────────────
    // The non-incremental path is the original: parallel parse every file, run Pass A then Pass B over
    // all. The incremental path reuses an unchanged file's cached Pass A decls (skipping its parse) and,
    // when the merged decl index is unchanged, its cached Pass B FnInfos too — producing a byte-identical
    // assembled FnInfo set (the merges below replay the original walk-order accumulation exactly). See
    // the cache section above for the soundness argument.
    use rayon::prelude::*;
    let incremental = INCREMENTAL.with(|c| c.get());
    let schema = cache_schema(include_tests);
    let cache_dir = Path::new(dir).join(".candor").join("cache");
    let cache_path = cache_dir.join("scan-cache.json");

    // Load the SINGLE consolidated cache file (`rel -> FileCache`) in one read+deserialize — far cheaper
    // than 1 open per source file. A cache whose schema doesn't match this binary is discarded wholesale.
    let mut prior: HashMap<String, FileCache> = if incremental {
        std::fs::read(&cache_path)
            .ok()
            .and_then(|b| serde_json::from_slice::<ScanCache>(&b).ok())
            .filter(|c| c.schema == schema)
            .map(|c| c.files)
            .unwrap_or_default()
    } else {
        HashMap::new()
    };

    // CONTENT HASHES (cheap parallel reads, no parse). The cached entry for a file is reusable iff its
    // stored content_hash matches the bytes on disk now.
    let hashes: Vec<(String, String)> = paths
        .par_iter()
        .map(|(p, rel)| (rel.clone(), std::fs::read(p).map(|b| fnv1a(&b)).unwrap_or_default()))
        .collect();
    let per_file: Vec<(String, String, Option<FileCache>)> = hashes
        .into_iter()
        .map(|(rel, content_hash)| {
            let cached = prior
                .remove(&rel)
                .filter(|fc| fc.content_hash == content_hash);
            (rel, content_hash, cached)
        })
        .collect();

    // ROUND 1 PARSE (parallel): every file whose Pass A decls are NOT validly cached. A read/parse
    // failure yields `None` (the original `else { continue }`), so its slot carries no parsed file and
    // contributes nothing — identical to before.
    // Each entry is `Option<(SendFile, locs)>`: the `locs` are this file's `file:line:col`s in walk order,
    // resolved HERE on the parse worker because proc-macro2's span line/col only resolves against the
    // parsing thread's source map (see `fn_locs`/`SendFile`). They ride alongside the moved file so Pass B
    // (single-threaded) can zip them onto each FnInfo without re-resolving a now-dead span.
    let round1: Vec<Option<ParsedFile>> = per_file
        .par_iter()
        .map(|(rel, _, cached)| {
            if cached.is_some() {
                return None; // decls reusable from cache — defer the parse (it may not be needed at all)
            }
            let p = &paths.iter().find(|(_, r)| r == rel)?.0;
            let text = std::fs::read_to_string(p).ok()?;
            let file = syn::parse_file(&text).ok()?;
            let mut locs = Vec::new();
            fn_locs(&file.items, rel, include_tests, &mut locs);
            // SAFETY: see `SendFile` — freshly parsed, uniquely owned, moved once, then single-threaded.
            Some((SendFile(file), locs))
        })
        .collect();

    // DISCLOSE files that failed to read/parse (no cache AND round-1 None): their effects are NOT in
    // the report. A silent skip violates "never silently pure" — the query side already discloses an
    // unparseable REPORT; mirror it for unparseable SOURCE (adversarial review).
    let unparsed: Vec<&str> = per_file
        .iter()
        .zip(&round1)
        .filter(|(pf, parsed)| pf.2.is_none() && parsed.is_none())
        .map(|(pf, _)| pf.0.as_str())
        .collect();
    if !unparsed.is_empty() {
        let shown = unparsed.iter().take(8).copied().collect::<Vec<_>>().join(", ");
        let more = if unparsed.len() > 8 { format!(" + {} more", unparsed.len() - 8) } else { String::new() };
        eprintln!(
            "candor-scan: {} source file(s) failed to read/parse — effects in them are NOT in this report (re-check the source): {shown}{more}",
            unparsed.len()
        );
    }
    // Remember whether ANY in-scope source failed to parse: the policy gate below must FAIL non-zero
    // when a policy is configured AND analysis was incomplete — a gateless-green over unanalyzed code
    // is a missed-effect = false-pure hole. (`unparsed` borrows `per_file`, consumed below; keep a flag.)
    let had_parse_failure = !unparsed.is_empty();

    // Per-file Pass A decls (cache or fresh) + a place to hold a parsed file for Pass B. A file dropped
    // by a read/parse failure (no cache AND round-1 parse failed) is excluded entirely, preserving the
    // original survivor set + walk order.
    let mut decls_per_file: Vec<(String, String, FileDecls)> = Vec::new(); // (rel, content_hash, decls)
    let mut parsed_files: HashMap<String, syn::File> = HashMap::new();     // rel -> parsed (round 1)
    let mut parsed_locs: HashMap<String, Vec<String>> = HashMap::new();    // rel -> per-fn loc (walk order)
    let mut cached_fninfos: HashMap<String, (String, Vec<FnInfo>)> = HashMap::new(); // rel -> (decl_index_hash, fninfos)
    // Files whose on-disk entry was already valid for BOTH content + the decl index it recorded — no
    // re-write needed unless the merged index moves (checked after the digest). Lets a no-op / body-only
    // re-scan skip rewriting the whole cache dir (the dominant cost when nothing changed).
    let mut disk_decl_hash: HashMap<String, String> = HashMap::new();
    for ((rel, ch, cached), r1) in per_file.into_iter().zip(round1) {
        match cached {
            Some(fc) => {
                // Decls reusable; the FnInfos are CONDITIONALLY reusable (checked after the digest).
                disk_decl_hash.insert(rel.clone(), fc.decl_index_hash.clone());
                decls_per_file.push((rel.clone(), ch, fc.decls));
                cached_fninfos.insert(rel, (fc.decl_index_hash, fc.fninfos));
            }
            None => {
                // A freshly-parsed file (or a parse failure → skip the file entirely, as before).
                let Some((sf, locs)) = r1 else { continue };
                let fd = file_decls(&sf.0.items, include_tests);
                decls_per_file.push((rel.clone(), ch, fd));
                parsed_locs.insert(rel.clone(), locs);
                parsed_files.insert(rel, sf.0);
            }
        }
    }

    // Pass A MERGE — replay the original accumulation in WALK ORDER over the per-file decls, so the
    // crate-wide index is byte-identical to the old sequential `collect_decls` loop.
    let mut merged = MergedDecls::default();
    for (_, _, fd) in &decls_per_file {
        merge_decls(&mut merged, fd);
    }
    let decl_index_hash = decl_index_digest(&merged);
    // Keep only unambiguous fn-leaf -> return-type / enum-variant-payload mappings (the `None`s drop).
    let returns: ReturnIndex =
        merged.rets.iter().filter_map(|(k, v)| v.clone().map(|t| (k.clone(), t))).collect();
    let enum_variants: EnumVariantIndex =
        merged.enum_tmp.iter().filter_map(|(k, v)| v.clone().map(|t| (k.clone(), t))).collect();
    let fields = &merged.fields;
    let field_elem = &merged.field_elem;
    let trait_impls = &merged.trait_impls;
    let trait_decls = &merged.trait_decls;
    let trait_fields = &merged.trait_fields;
    let traits = TraitIndexes { impls: trait_impls, decls: trait_decls, fields: trait_fields };
    let elems = ElemIndexes { field_elem, enum_variants: &enum_variants };
    let lazy_statics = &merged.lazy_statics;

    // ROUND 2 PARSE (parallel): files whose decls were cached but whose FnInfos are STALE (the merged
    // decl index moved) — exactly the files a decl-changing edit invalidates. On a body-only edit this
    // set is empty; on a decl edit it is "everything else", re-parsed in parallel (degrade-to-full).
    let need_passb: Vec<&str> = decls_per_file
        .iter()
        .map(|(rel, _, _)| rel.as_str())
        .filter(|rel| {
            !parsed_files.contains_key(*rel)
                && cached_fninfos.get(*rel).map(|(h, _)| h != &decl_index_hash).unwrap_or(true)
        })
        .collect();
    let round2: Vec<(String, Option<ParsedFile>)> = need_passb
        .par_iter()
        .map(|rel| {
            let parsed = paths
                .iter()
                .find(|(_, r)| r == rel)
                .and_then(|(p, _)| std::fs::read_to_string(p).ok())
                .and_then(|t| syn::parse_file(&t).ok())
                .map(|file| {
                    // Resolve loc on THIS parse worker (span line/col is thread-local) — same as round 1.
                    let mut locs = Vec::new();
                    fn_locs(&file.items, rel, include_tests, &mut locs);
                    (SendFile(file), locs)
                });
            (rel.to_string(), parsed)
        })
        .collect();
    for (rel, sf) in round2 {
        if let Some((sf, locs)) = sf {
            parsed_locs.insert(rel.clone(), locs);
            parsed_files.insert(rel, sf.0);
        }
    }

    // Pass B — assemble each file's FnInfos in WALK ORDER: reuse the cached set when the decl index is
    // unchanged, else re-derive from the (now parsed) file. Either way the concatenated `fns` is exactly
    // what the old single Pass B loop produced.
    let mut fns: Vec<FnInfo> = Vec::new();
    let mut fresh_fninfos: HashMap<String, Vec<FnInfo>> = HashMap::new();
    for (rel, _, _) in &decls_per_file {
        let reuse = cached_fninfos
            .get(rel)
            .filter(|(h, _)| *h == decl_index_hash)
            .map(|(_, v)| v.clone());
        if let Some(v) = reuse {
            fns.extend(v.iter().cloned());
            continue;
        }
        // Re-derive: the file is parsed (round 1 or round 2); if both parses failed it's simply absent.
        let Some(file) = parsed_files.get(rel) else { continue };
        let modpath = module_path(Path::new(rel));
        // Locs were resolved on the parse worker (spans are dead on this thread); reuse them positionally.
        let locs = parsed_locs.get(rel).map(Vec::as_slice).unwrap_or(&[]);
        let mut loc_idx = 0usize;
        let mut uses = HashMap::new();
        let mut file_fns: Vec<FnInfo> = Vec::new();
        scan_items(&file.items, &modpath, locs, &mut loc_idx, include_tests, fields, &returns, traits, elems, lazy_statics, &mut uses, &mut file_fns);
        fns.extend(file_fns.iter().cloned());
        fresh_fninfos.insert(rel.clone(), file_fns);
    }

    // WRITE BACK the cache (incremental only) as ONE consolidated file. Each entry persists {content_hash,
    // decls, decl_index_hash, fninfos}; the FnInfos written are the CURRENT ones (reused or freshly
    // derived) tagged with the CURRENT decl_index_hash, so the next scan's gate is exact. The map is
    // rebuilt from the current path set, so deleted/renamed files drop out automatically (no pruning pass).
    // The write is SKIPPED entirely when nothing changed — every file's decls came from cache AND already
    // recorded this decl_index_hash AND no file was added/removed — so a no-edit re-scan does zero writes.
    // Best-effort: a cache write failure never affects the report (it only costs a re-derivation later).
    if incremental {
        let unchanged = fresh_fninfos.is_empty()
            && prior.is_empty() // every prior entry was consumed by a current file → none deleted
            && decls_per_file.iter().all(|(rel, _, _)| disk_decl_hash.get(rel) == Some(&decl_index_hash));
        if !unchanged {
            let mut files: HashMap<String, FileCache> = HashMap::with_capacity(decls_per_file.len());
            for (rel, ch, fd) in &decls_per_file {
                let fninfos = fresh_fninfos
                    .get(rel)
                    .cloned()
                    .or_else(|| cached_fninfos.get(rel).map(|(_, v)| v.clone()))
                    .unwrap_or_default();
                files.insert(
                    rel.clone(),
                    FileCache {
                        content_hash: ch.clone(),
                        decls: fd.clone(),
                        decl_index_hash: decl_index_hash.clone(),
                        fninfos,
                    },
                );
            }
            let cache = ScanCache { schema: schema.clone(), files };
            let _ = std::fs::create_dir_all(&cache_dir);
            if let Ok(bytes) = serde_json::to_vec(&cache) {
                let _ = candor_report::write_atomic(&cache_path, &bytes);
            }
        }
    }

    // The κ-coverage ledger: Cargo.toml's [dependencies] are the crate's TRUE external universe, so a
    // dep the calls actually reach whose classification never fires — and that isn't in a calibrated
    // tier — is a named blind spot (invisible, not Unknown: the curated-κ caveat). Counted here,
    // disclosed in the receipt, so the caveat is per-scan evidence instead of a doc footnote.
    let (deps, dep_renames) = cargo_deps(dir);
    let mut dep_seen: HashMap<String, usize> = HashMap::new(); // dep crate root -> call-site count
    let mut dep_classified: std::collections::HashSet<String> = std::collections::HashSet::new();
    // fn -> the dep crates it DIRECTLY calls into where the classifier floored the call. Post-filtered to
    // the genuinely-blind crates (κ never classified them) + propagated transitively → the per-fn
    // `invisible` honesty disclosure (the κ ledger, but attributed per function).
    let mut blind_direct: HashMap<String, BTreeSet<String>> = HashMap::new();
    // Blind crates inherited from a dep fn's `invisible` (sweep [8]): genuinely blind (the dep confirmed
    // it), but a TRANSITIVE crate the consumer never saw directly, so it is absent from `dep_seen` and
    // would be dropped by the `global_blind` filter. Collected here and unioned into global_blind below.
    let mut dep_invisible: BTreeSet<String> = BTreeSet::new();

    // Two name indexes for resolving a call to a local definition. `by_leaf` keys on the bare last
    // segment (`new`); `by_tail2` keys on the last TWO segments (`RequestBuilder::new`). The leaf index
    // alone catastrophically over-connects on real crates: every call to *some* `new()` would link to
    // ALL `*::new` defs (in reqwest, 181 of them), smearing one type's effect across the whole graph.
    // So a `Type::method`/`mod::fn` call matches the qualified tail (keeping `RequestBuilder::new` distinct
    // from `Body::new`) and a bare free call matches the leaf — BOTH only when the match is UNAMBIGUOUS
    // (exactly one def), under-reporting rather than fabricating. See `resolve_target` + the module doc.
    let mut by_leaf: HashMap<String, Vec<String>> = HashMap::new();
    let mut by_tail2: HashMap<String, Vec<String>> = HashMap::new();
    // Type names with a LOCAL definition — the penultimate `Type` segment of a `Type::method` qual. A
    // receiver-typed method call resolves to a local method ONLY if its type is in here, so an external
    // `reqwest::Client::send` can't mis-link to a same-named local `Client::send` (an inverse fabrication).
    let mut local_types: std::collections::HashSet<String> = std::collections::HashSet::new();
    for f in &fns {
        // SYNTHETIC lazy-init units (`<lazy>::NAME`) are resolved ONLY via the qualified `<lazy>::`
        // tail2 route a forcing site emits — they must NOT enter `by_leaf`, or a bare call to a real fn
        // sharing the static's NAME would see an ambiguous leaf and stop resolving (a spurious
        // under-report on unrelated code). Their tail2 (`<lazy>::NAME`) is unique and the forcing edge
        // always qualifies, so keeping them out of `by_leaf` loses nothing.
        let is_lazy_unit = f.qual.starts_with(LAZY_UNIT_PREFIX);
        if !is_lazy_unit {
            by_leaf.entry(f.leaf.clone()).or_default().push(f.qual.clone());
        }
        if let Some(t2) = tail2(&f.qual) {
            if let Some(ty) = t2.split("::").next() {
                if ty.chars().next().is_some_and(|c| c.is_uppercase()) {
                    local_types.insert(ty.to_string());
                }
            }
            by_tail2.entry(t2).or_default().push(f.qual.clone());
        }
    }

    // Inverse of trait_impls (impl-TYPE leaf → the trait leaves it impls), for the trait-DEFAULT-method
    // caller fallback below: a call `t.m()` on a concrete type T that does NOT declare `m` but impls a
    // trait with a DEFAULT `m` should edge to that trait's `Trait::m` (the inherited default body — now
    // scanned, via the Item::Trait arm). Without this the caller silently under-reported (`run()` calling
    // `l.flush()` on a FileLogger that inherits `Logger::flush`'s Fs/Net — adversarial review).
    let mut type_to_traits: HashMap<String, Vec<String>> = HashMap::new();
    for (tr, types) in &merged.trait_impls {
        let tr_leaf = tr.rsplit("::").next().unwrap_or(tr).to_string();
        for ty in types {
            let ty_leaf = ty.rsplit("::").next().unwrap_or(ty).to_string();
            type_to_traits.entry(ty_leaf).or_default().push(tr_leaf.clone());
        }
    }

    // Method leaves that name a LOCAL method definition (a `Type::method` qual whose `Type` is local).
    // A bare-leaf method CALL (`x.fastrand()`, recorded path==leaf, no `::`) whose leaf matches one of
    // these resolves to the project's OWN method, so the calibrated-crate classification of that leaf
    // (`fastrand` → Rand, `now` → Clock) must be SUPPRESSED — the local definition is authoritative. This
    // covers the case `resolve_target` deliberately leaves unresolved: a method on a receiver whose type
    // the scanner can't infer (`Mutex::lock()`'s guard, `self.state.lock()` → `MutexGuard<FastRand>`),
    // where no typed `FastRand::fastrand` sibling forms yet the leaf still names a local method. Suppress
    // on PRESENCE of a same-named local method, not on a recorded edge — under-reporting on the rare
    // ambiguous leaf beats fabricating an effect candor never observed (the cardinal sin). (Real tokio
    // sweep: `RngSeedGenerator::next_seed` calls `rng.fastrand()` through a lock guard → bare leaf
    // `fastrand` → Rand, propagated to ~14 fns incl `Runtime::new`.)
    let mut direct: HashMap<String, BTreeSet<&'static str>> = HashMap::new();
    let mut hosts: HashMap<String, BTreeSet<String>> = HashMap::new();
    let mut cmds: HashMap<String, BTreeSet<String>> = HashMap::new();
    let mut paths: HashMap<String, BTreeSet<String>> = HashMap::new();
    let mut tables: HashMap<String, BTreeSet<String>> = HashMap::new();
    // Effects whose literal SURFACE is INCOMPLETE for a fn: it has a Net reach whose host is invisible to
    // the gate (a Net call with no string-literal arg — a runtime host, or a builder terminal whose host was
    // on a pure builder candor doesn't capture). The AS-EFF-008 gate treats an incomplete surface as
    // uncertifiable EVEN with other visible hosts, so a benign literal can't MASK the invisible endpoint
    // (the same gate evasion fixed in candor-java 0.5.29). Generalized from Net to Exec/Fs/Db (a masked
    // path/table alongside a benign sibling literal defeated `opaque` and silently passed `allow Fs`/
    // `allow Db`) — the establishing-allowlist predicate per effect (is_net_establishing /
    // is_cmd_naming_method / is_fs_path_arg / is_db_query_arg), matching candor-java's surfaceIncomplete.
    let mut incomplete: HashMap<String, BTreeSet<&'static str>> = HashMap::new();
    let mut calls: HashMap<String, BTreeSet<String>> = HashMap::new();
    let mut loc: HashMap<String, String> = HashMap::new();
    // Per-fn DIRECT Unknown-origin reasons (the receipt's `unknownWhy`, spec §2). Coarse, like the lint's
    // per-trait tag: a callback we can't see through, an FFI/extern boundary, or a genuinely-unresolvable
    // bare call. Tracked so the disclosure names WHY, not just that an Unknown exists.
    let mut unknown_why: HashMap<String, BTreeSet<&'static str>> = HashMap::new();
    for f in &fns {
        loc.entry(f.qual.clone()).or_insert_with(|| f.loc.clone());
        // The body invoked a callable the scan can't see through (closure / fn-pointer value): it could
        // perform any effect, so record an honest `Unknown` (propagated like any effect, surfaced in the
        // receipt's unresolved count) instead of silently certifying the function pure.
        if f.unresolved {
            direct.entry(f.qual.clone()).or_default().insert("Unknown");
            unknown_why.entry(f.qual.clone()).or_default().insert("callback:unresolved call");
        }
        // DROP-GLUE (#3): local types this fn CONSTRUCTS that have a local `impl Drop`. The `Drop::drop`
        // body runs at scope exit — an implicit edge the call graph misses, so a guard that flushes/closes
        // on drop read silent-pure. Collected per-call below (a `T::*` associated-fn call where `T` has a
        // local Drop), then edged to `T::drop` after the call loop. Over-approximates toward the SOUND
        // direction (a constructed value is assumed to drop in this scope); gated to LOCAL drop types only,
        // so an external type's invisible Drop is never fabricated.
        let mut drops_here: BTreeSet<String> = BTreeSet::new();
        for c in &f.calls {
            let cr = c.path.split("::").next().unwrap_or("");
            let classified = candor_classify::classify(cr, &c.path)
                .or_else(|| scan_builder_entry_effect(cr, &c.path));
            // DROP-GLUE detection: a `T::assoc()` ASSOCIATED-FN call (a CONSTRUCTOR like `Guard::new`)
            // where `T` is a LOCAL drop type means a `T` value is CREATED in this scope and dropped at exit.
            // Record `T` so we edge to `T::drop` after the loop. CRUCIALLY gated to `!c.method`: a METHOD
            // call (`reg.poll()`, recorded typed as `Registration::poll`) operates on a BORROW and does NOT
            // own/drop the value here — including those over-connected every borrow-site to the drop body
            // (tokio: 170 fns). A constructor is `Type::fn(..)` syntax (an associated fn, `method=false`).
            // Excludes `T::drop` itself.
            if !merged.drop_types.is_empty() && !c.method && c.path.contains("::") && c.leaf != "drop" {
                if let Some(ty) = tail2(&c.path).and_then(|t2| t2.split("::").next().map(str::to_string)) {
                    if merged.drop_types.contains(&ty) {
                        drops_here.insert(ty);
                    }
                }
            }
            // κ ledger: a qualified call into a declared dependency. (A bare leaf has no `::`, so it
            // can't name a crate; a local module sharing a dep's name is the rare accepted ambiguity.)
            if c.path.contains("::") && deps.contains(cr) {
                *dep_seen.entry(cr.to_string()).or_insert(0) += 1;
                if classified.is_some() {
                    dep_classified.insert(cr.to_string());
                } else {
                    // a FLOORED dep call: candidate per-fn blind spot (filtered to genuinely-blind below).
                    blind_direct.entry(f.qual.clone()).or_default().insert(cr.to_string());
                }
            }
            // (The CANDOR_DEPS cross-crate JOIN moved BELOW — it must run AFTER `resolved_local`/
            // `suppress_bare_leaf` are known and be gated on them, else a local fn/method/module named like
            // a covered dep crate inherits that dep's effects onto a provably-pure LOCAL path — the same
            // cardinal-sin fabrication the classifier's `resolved_local` guard prevents, which this join
            // never had. Found by the cross-jar sweep.)
            // Resolve the call to a local definition via the precise, uniqueness-filtered `resolve_target`.
            // A receiver-typed `Type::method` call (`x.go()` inferred to `S::go`) resolves to the local
            // method ONLY when `Type` is locally defined — this recovers the common `x.method()` edge that
            // a bare leaf can't safely provide, while an external `reqwest::Client::send` is left to the
            // classifier (its type isn't local, so it can't mis-link to a same-named local `Client::send`).
            // A non-typed call uses the leaf/qualified-tail routes; std/core/alloc are the classifier's.
            let resolvable = if c.is_macro {
                // A macro is never a call to a local FUNCTION. Its (possibly crate-local) qualified path
                // must NOT resolve to a same-named local fn, or that fn's effect is fabricated onto the
                // caller (the phantom-edge cardinal sin). Its effect still flows via `classified` / κ above.
                false
            } else if c.typed {
                tail2(&c.path)
                    .and_then(|t2| t2.split("::").next().map(str::to_string))
                    .is_some_and(|ty| local_types.contains(&ty))
            } else {
                !matches!(cr, "std" | "core" | "alloc")
            };
            // A `Type::assoc()` whose `Type` is a NON-NOMINAL alias (`type Inner = [u8; N]`) names a type
            // with no local impl — its assoc fn is std/core's, NOT a same-named local STRUCT's. Skip the
            // local link so the array alias's `Inner::default()` doesn't inherit `struct Inner`'s
            // effectful `Default` (the sled IVec fabrication).
            let aliased = tail2(&c.path)
                .and_then(|t2| t2.split("::").next().map(str::to_string))
                .is_some_and(|ty| merged.prim_aliases.contains(&ty));
            // Did this call resolve to a LOCAL definition (free fn, method, or a unique trait-default)?
            // If so the local def is AUTHORITATIVE and its effects flow through the `calls` edge — the
            // crate/FFI classifier MUST NOT also fire, or a pure local fn whose NAME collides with an FFI
            // tier (`sqlite3_step`/`git_clone`/`curl_*`/`SSL_*`) or a whole-crate rule (`getrandom`/
            // `fastrand`) inherits that crate's effect: FABRICATION on a provably-pure path, transitively
            // poisoning every caller (the cardinal sin the syntactic floor must never commit). The
            // bare-leaf-METHOD suppression below was the special case of this; this covers the general
            // case (free fns and qualified `Type::method` calls the bare-leaf guard missed).
            let mut resolved_local = false;
            if resolvable && !aliased {
                let targets = resolve_target(&c.path, &c.leaf, c.method, &by_tail2, &by_leaf);
                if let Some(targets) = targets {
                    resolved_local = true;
                    for t in targets {
                        if t != &f.qual {
                            calls.entry(f.qual.clone()).or_default().insert(t.clone());
                        }
                    }
                } else if c.method && c.typed {
                    // No `T::leaf` resolved (T doesn't declare `leaf`). If T impls EXACTLY ONE trait whose
                    // DEFAULT `leaf` body exists (a `Trait::leaf` FnInfo), the call inherits it — edge there.
                    // COLLISION-SAFE: zero or >1 distinct candidate → skip (the honest under-report; never
                    // guess between traits — the keying-collision discipline that keeps this from FABRICATING
                    // a wrong trait's effect onto the caller).
                    if let Some(t_type) = tail2(&c.path).and_then(|t2| t2.split("::").next().map(str::to_string)) {
                        if let Some(trs) = type_to_traits.get(&t_type) {
                            let mut hits: Vec<&String> = Vec::new();
                            for tr_leaf in trs {
                                if let Some(ts) = by_tail2.get(&format!("{tr_leaf}::{}", c.leaf)) {
                                    for t in ts {
                                        if !hits.contains(&t) {
                                            hits.push(t);
                                        }
                                    }
                                }
                            }
                            if hits.len() == 1 && hits[0] != &f.qual {
                                resolved_local = true;
                                calls.entry(f.qual.clone()).or_default().insert(hits[0].clone());
                            }
                        }
                        // AUTO-DEREF fallback (last, after inherent + trait-default — Rust's resolution
                        // order): a custom `impl Deref for t_type { type Target = U }` makes `recv.leaf()`
                        // dispatch to `U::leaf`. Chase the Deref chain (bounded) and edge to the first
                        // `U::leaf` that resolves — the user-Deref analog of the Box/Arc/Rc peel (a newtype
                        // `impl Deref` dropped `wrapper.method()` to silent-pure — corpus find). `.clone()`
                        // is guarded at the typed-call emit, so no pointee-clone fabrication recurs.
                        if !resolved_local {
                            let mut cur = t_type.clone();
                            let mut hops = 0;
                            while let Some(target) = merged.deref_target.get(&cur).cloned() {
                                if hops >= 8 { break; }
                                hops += 1;
                                if let Some(ts) = resolve_target(&format!("{target}::{}", c.leaf), &c.leaf, false, &by_tail2, &by_leaf) {
                                    resolved_local = true;
                                    for t in ts {
                                        if t != &f.qual {
                                            calls.entry(f.qual.clone()).or_default().insert(t.clone());
                                        }
                                    }
                                    break;
                                }
                                cur = target;
                            }
                        }
                    }
                }
            }
            // A BARE-LEAF method call (`self.fastrand()` → path == leaf, no `::`) carries no crate
            // qualifier, so its `classify` consults the bare leaf against the calibrated crate/verb rules
            // (`fastrand` → Rand, `now` → Clock, …). When that leaf names a LOCAL method definition
            // (`local_method_leaves`), the call resolves to the project's OWN method — the local definition
            // is AUTHORITATIVE — so a local method merely NAMED like a calibrated crate (tokio's pure
            // `FastRand::fastrand` xorshift) must NOT inherit the crate's effect. Suppress the bare-leaf
            // classification; the effect (if any) flows from the resolved target through propagation. The
            // external-crate classification of a bare leaf still applies when NO local method shares the name
            // (a genuine `fastrand::u32` dependency call). Qualified calls keep their type-precise rule.
            // A BARE leaf (no `::`) naming ANY local definition (`by_leaf` keys every local fn/method by
            // leaf) is the project's OWN — the local def is authoritative. Suppress the bare-leaf
            // classifier (and the dep-join below). This covers the bare-leaf METHOD case AND the bare-leaf
            // FREE-FN case the old `c.method && local_method_leaves` guard missed: a pure local free fn
            // whose leaf is AMBIGUOUS (≥2 local defs, e.g. a free `git_clone` + a trait method `git_clone`)
            // defeats `resolve_target`'s uniqueness filter (→ `resolved_local=false`), so the FFI/crate
            // classifier fired unsuppressed and fabricated the effect (cardinal sin). A bare leaf with no
            // local def (a genuine prelude/extern call) still classifies; a `use`-imported call is
            // qualified (`::`) and keeps its type-precise rule.
            let suppress_bare_leaf = !c.path.contains("::") && by_leaf.contains_key(&c.leaf);
            // CANDOR_DEPS cross-crate JOIN (spec §2), GATED: an UNCLASSIFIED qualified call into a crate a
            // sibling report covers inherits that fn's recorded effects + literal surfaces — UNLESS the call
            // resolved to a local target or names a local bare leaf (then the local is authoritative; the
            // join would fabricate). Joined unambiguous-tail2-first, then unambiguous leaf, like resolve_target.
            // A renamed dep joins under its real package name.
            let cr_real: &str = dep_renames.get(cr).map(String::as_str).unwrap_or(cr);
            let mut dep_join_hit = false;
            if classified.is_none() && !resolved_local && !suppress_bare_leaf
                && c.path.contains("::") && deps_idx.crates.contains(cr_real)
            {
                let rel = c.path.strip_prefix(&format!("{cr}::")).unwrap_or(&c.path);
                let hit = if rel.contains("::") {
                    tail2(rel).and_then(|t2| deps_idx.by_key.get(&format!("{cr_real}#{t2}")))
                } else {
                    deps_idx.by_key.get(&format!("{cr_real}#{rel}"))
                };
                if let Some(de) = hit {
                    dep_join_hit = true;
                    for e in &de.effects {
                        direct.entry(f.qual.clone()).or_default().insert(e);
                    }
                    hosts.entry(f.qual.clone()).or_default().extend(de.hosts.iter().cloned());
                    cmds.entry(f.qual.clone()).or_default().extend(de.cmds.iter().cloned());
                    paths.entry(f.qual.clone()).or_default().extend(de.paths.iter().cloned());
                    tables.entry(f.qual.clone()).or_default().extend(de.tables.iter().cloned());
                    // sweep [8]: inherit the dep fn's blind-crate disclosure so a consumer's pure verdict
                    // stays qualified across the chain boundary (else the dep's floored reach reads as plain
                    // pure here). Propagated with the local blind spots below.
                    blind_direct.entry(f.qual.clone()).or_default().extend(de.invisible.iter().cloned());
                    dep_invisible.extend(de.invisible.iter().cloned());
                    // sweep [30]: inherit the dep fn's masking-incompleteness so a benign literal here can't
                    // certify the dep's invisible runtime endpoint (propagated with the local incomplete map).
                    if !de.incomplete.is_empty() {
                        incomplete.entry(f.qual.clone()).or_default().extend(de.incomplete.iter().copied());
                    }
                    dep_classified.insert(cr.to_string());
                }
            }
            if let Some(eff) = classified.filter(|_| !suppress_bare_leaf && !resolved_local) {
                direct.entry(f.qual.clone()).or_default().insert(eff);
                // A host-ESTABLISHING Net / program-NAMING Exec call with NO captured literal → the endpoint
                // is invisible to the gate (a runtime value). Mark the surface incomplete so a benign captured
                // literal can't certify it (the masking evasion). Establishing-allowlist via the SHARED
                // predicate (is_net_establishing / is_cmd_naming_method) — same as the deep engine — so a
                // USE-verb (`stream.write()`) whose host was fixed at `connect` never false-positives.
                if c.str_arg.is_none() {
                    if eff == "Net" && candor_classify::is_net_establishing(&c.leaf) {
                        incomplete.entry(f.qual.clone()).or_default().insert("Net");
                    } else if eff == "Exec" && candor_classify::is_cmd_naming_method(&c.leaf) {
                        incomplete.entry(f.qual.clone()).or_default().insert("Exec");
                    } else if eff == "Fs" && !c.method && candor_classify::is_fs_path_arg(&c.leaf) {
                        // A path-NAMING Fs call (`fs::write(p,…)`/`File::open(p)` — a free fn / constructor,
                        // `method=false`) with NO captured path literal → the path is a runtime value,
                        // invisible to the gate. Mark Fs incomplete so a benign sibling literal can't certify
                        // the masked path (`allow Fs` fails closed). The `!c.method` gate excludes the
                        // path-stat METHODS (`p.metadata()`/`p.exists()`) whose path is the RECEIVER, not an
                        // arg — same establishing-allowlist discipline as Net/Exec (matches candor-java).
                        incomplete.entry(f.qual.clone()).or_default().insert("Fs");
                    } else if eff == "Db" && candor_classify::is_db_query_arg(&c.leaf) {
                        // A SQL-QUERY-bearing Db call (`con.execute(sql,…)`/`query`/`prepare`) with NO captured
                        // query literal → the table is a runtime value, invisible to the gate. Mark Db
                        // incomplete so a benign sibling literal can't certify the masked table. The allowlist
                        // excludes build-then-execute terminals (`fetch_all`/`load`/`all`) and lifecycle ops
                        // (`connect`/`open`/`begin`) whose query is built structurally (no maskable string).
                        incomplete.entry(f.qual.clone()).or_default().insert("Db");
                    }
                }
                if let Some(s) = &c.str_arg {
                    match eff {
                        "Net" => { hosts.entry(f.qual.clone()).or_default().insert(host_part(s)); }
                        "Exec" => {
                            // Capture the program head + refine the cliff (spec §4 ⟨0.5⟩) ONLY at a
                            // program-NAMING call (`new`/`cmd`), an ALLOWLIST — not "any method except a
                            // known modifier". A whole-crate-Exec crate (portable_pty/duct) classifies
                            // EVERY method as Exec, so a denylist leaked non-naming methods (a getter
                            // `get_env("psql")` reads back a KEY, not a program) → fabricated Db + polluted
                            // the `cmds` surface (a false `allow Exec` match). Method = the path's last segment.
                            if candor_classify::is_cmd_naming_method(c.path.rsplit("::").next().unwrap_or("")) {
                                cmds.entry(f.qual.clone()).or_default().insert(s.clone());
                                direct.entry(f.qual.clone()).or_default()
                                    .extend(candor_classify::classify_command_head(s).iter().copied());
                            }
                        }
                        "Fs" => { paths.entry(f.qual.clone()).or_default().insert(s.clone()); }
                        // Table-position identifiers in a SQL string literal — the Db literal
                        // surface (feeds `allow Db …`); a dynamically-built query yields nothing.
                        "Db" => { tables.entry(f.qual.clone()).or_default().extend(candor_classify::tables_in_sql(s)); }
                        _ => {}
                    }
                }
            }
            // §4 HONESTY — FFI BOUNDARY: a call that fell through EVERY resolution route above (not
            // classified, not a local def, not a dep-report join) AND names a fn declared in an `extern`
            // block is the canonical unknowable boundary — its body is in another language, so the effect
            // (Fs/Net/Exec/…) is unknowable. DISCLOSE Unknown — the same honest signal an unresolved
            // callback gets — instead of silent-pure. A safe wrapper `unsafe { system(cmd) }` otherwise
            // read pure (the `extern` block was never collected, so the call was a bare leaf resolving to
            // nothing → pure). NEVER fires when a LOCAL def of the same name exists (`suppress_bare_leaf`
            // / `resolved_local` win — the local is authoritative, no fabrication).
            //
            // (The general "any unresolvable bare call → Unknown" disclosure was PROTOTYPED and REJECTED:
            // it floods on a real corpus — closure-param invocations (`func(x)`), macro-DEFINED local
            // helpers absent from `by_leaf`, and cfg-gated platform fns all read as bare-unresolved, so it
            // charged ~80 pure tokio fns Unknown for ~0 genuine signal beyond this FFI case. See the task
            // report's residual note. The extern case below is the precise, non-flooding subset.)
            let already_handled = classified.is_some() || resolved_local || suppress_bare_leaf || dep_join_hit;
            if !c.is_macro && !already_handled && merged.extern_fns.contains(&c.leaf) {
                direct.entry(f.qual.clone()).or_default().insert("Unknown");
                unknown_why.entry(f.qual.clone()).or_default().insert("native:extern fn"); // FFI is a native boundary — canonical `native:` (SPEC §4 ⟨0.7⟩)
            }
            // §4 HONESTY — AMBIGUOUS LOCAL: a BARE leaf naming TWO-OR-MORE local defs (`tail2`/leaf
            // collision: a free `tail2` + a `Type::tail2` method, or two `Type::method`s) defeats
            // `resolve_target`'s uniqueness filter (resolved_local=false) AND is suppressed from the
            // classifier/dep-join (`suppress_bare_leaf` — the local is authoritative). Today that leaves
            // NO edge and NO disclosure: the callee's effects vanish (silent-pure over a real local call).
            // DISCLOSE Unknown instead — we can't pick WHICH local def runs, so its effects are unknown,
            // not absent. PRECISELY scoped (≥2 local defs of this bare leaf) so it can't flood like the
            // rejected "any unresolvable bare call → Unknown": a closure-param call / macro-helper isn't in
            // `by_leaf`, and a UNIQUE leaf resolves through `resolve_target` (never reaches here).
            // EXCLUDE method calls (`x.run()`): an unqualified method call already resolves to NOTHING by
            // design (the `method` flag — linking it to a same-named def would guess/fabricate), and a
            // same-named method is the COMMON case (`run`/`get`/`handle` across many types), so firing here
            // floods every such call with Unknown. This disclosure is for genuinely-bare FREE calls (the M1
            // case): `run()` with ≥2 free `run` defs, where the silent drop really is a lost local edge.
            if !c.is_macro && !c.method && classified.is_none() && !resolved_local && suppress_bare_leaf
                && !c.path.contains("::")
                && by_leaf.get(&c.leaf).is_some_and(|v| v.len() >= 2)
            {
                direct.entry(f.qual.clone()).or_default().insert("Unknown");
                unknown_why.entry(f.qual.clone()).or_default().insert("ambiguous:same-name local defs");
            }
        }
        // DROP-GLUE EDGE (#3): for each LOCAL drop type this fn constructed, add the implicit scope-exit
        // edge to its `T::drop` body — but ONLY when that body is a UNIQUE local def (in `by_tail2` with
        // exactly one target), the same uniqueness discipline `resolve_target` uses. The drop body's
        // effects then propagate to `f` like any other callee (a flushing/closing guard stops reading
        // silent-pure). Self-edges are skipped (a `Drop::drop` that constructs its own type).
        for ty in &drops_here {
            if let Some(targets) = by_tail2.get(&format!("{ty}::drop")) {
                if targets.len() == 1 && targets[0] != f.qual {
                    calls.entry(f.qual.clone()).or_default().insert(targets[0].clone());
                }
            }
        }
    }

    let all: Vec<String> = fns.iter().map(|f| f.qual.clone()).collect();
    let inferred = propagate(&direct, &calls, &all);
    let hostsacc = propagate_str(&hosts, &calls, &all);
    let cmdsacc = propagate_str(&cmds, &calls, &all);
    let pathsacc = propagate_str(&paths, &calls, &all);
    let tablesacc = propagate_str(&tables, &calls, &all);
    let incompleteacc = propagate(&incomplete, &calls, &all); // transitive masking-incompleteness
    let blind_acc = propagate_str(&blind_direct, &calls, &all); // transitive per-fn blind reach
    // The genuinely-blind dep crates (the per-scan κ "unlisted" set): seen, never classified, not
    // dep-report-covered, not calibrated. A fn's `invisible` = its transitive blind reach ∩ this set.
    let global_blind: std::collections::HashSet<String> = dep_seen
        .keys()
        .filter(|cr| {
            !dep_classified.contains(*cr)
                && !deps_idx.crates.contains(dep_renames.get(cr.as_str()).map(String::as_str).unwrap_or(cr.as_str()))
                && !candor_classify::CALIBRATED_CRATES.contains(&cr.as_str())
                && !candor_classify::PATH_CALIBRATED_CRATES.contains(&cr.as_str())
                && !candor_classify::CALIBRATED_PREFIXES.iter().any(|p| cr.starts_with(p))
        })
        .cloned()
        .collect();
    // A dep-inherited invisible crate is genuinely blind (the dep's own scan confirmed it) but transitive,
    // so it never appears in `dep_seen` — keep it so the consumer's `invisible` survives the filter ([8]).
    let global_blind: std::collections::HashSet<String> =
        global_blind.into_iter().chain(dep_invisible).collect();

    let mut entries: Vec<ReportEntry> = Vec::new();
    let mut cg: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for q in &all {
        // SPEC §2.2: the sidecar records EVERY analyzed function — including a LEAF with no local
        // callees, as an empty list. Omitting leaves made an uncalled FFI-only fn (nix `unistd::pipe`)
        // invisible to `whatif`/`callers` ("no function matching") even though it's in the report;
        // an always-present key also lets a consumer distinguish "no callers" from "no such function".
        cg.insert(q.clone(), calls.get(q).map(|cs| cs.iter().cloned().collect()).unwrap_or_default());
        let inf = inferred.get(q).cloned().unwrap_or_default();
        // Keep a pure fn if it has a BLIND reach — so the honesty disclosure survives on exactly the
        // `inferred: []` fns that need it (else `invisible` would be dropped with the pure entry).
        let has_blind = blind_acc.get(q).is_some_and(|s| s.iter().any(|c| global_blind.contains(c)));
        if inf.is_empty() && !has_blind {
            continue;
        }
        entries.push(ReportEntry {
            func: q.clone(),
            loc: loc.get(q).cloned().unwrap_or_default(),
            inferred: inf.iter().map(|s| s.to_string()).collect(),
            direct: direct.get(q).map(|d| d.iter().map(|s| s.to_string()).collect()).unwrap_or_default(),
            declared: Vec::new(),
            undeclared: Vec::new(),
            overdeclared: Vec::new(),
            // Honest blind-spot signal: this function (transitively) reached a callable the scan couldn't
            // see through. Mirrors the lint's `unresolved = has Unknown`, so the receipt's unresolved
            // count is truthful for the stable backend too — not a hardcoded 0.
            unresolved: inf.contains("Unknown"),
            // The cross-crate join key (spec §2): `crate#qual`, derivable by any consumer from its
            // own syntactic view of the call — what CANDOR_DEPS chaining matches against.
            hash: format!("{crate_name}#{q}"),
            fs: Vec::new(),
            hosts: hostsacc.get(q).map(|s| s.iter().cloned().collect()).unwrap_or_default(),
            cmds: cmdsacc.get(q).map(|s| s.iter().cloned().collect()).unwrap_or_default(),
            paths: pathsacc.get(q).map(|s| s.iter().cloned().collect()).unwrap_or_default(),
            tables: tablesacc.get(q).map(|s| s.iter().cloned().collect()).unwrap_or_default(),
            calls: calls.get(q).map(|cs| cs.iter().cloned().collect()).unwrap_or_default(),
            // DIRECTLY-introduced Unknown origins (candor-spec §2 `unknownWhy`): an unresolved callback /
            // fn-pointer call, an FFI/extern boundary, or a genuinely-unresolvable bare call. Coarser than
            // the lint's per-trait tag — by design — but now names WHICH boundary, not just "callback".
            unknown_why: unknown_why
                .get(q)
                .map(|s| s.iter().map(|r| r.to_string()).collect())
                .unwrap_or_default(),
            // candor-spec §2 `entryPoint`: syntactically we can only spot `main` (the program root). The
            // lint also flags `#[no_mangle]`; the scanner can't see attributes, so it under-marks — the
            // sound direction for an optional reachability hint.
            entry_point: q.rsplit("::").next() == Some("main"),
            // Per-fn honesty: the genuinely-blind crates this fn transitively reaches. `inferred` is a
            // LOWER BOUND when this is non-empty.
            invisible: blind_acc
                .get(q)
                .map(|s| s.iter().filter(|c| global_blind.contains(*c)).cloned().collect())
                .unwrap_or_default(),
            // Masking-incomplete effects — carried so a CANDOR_DEPS consumer inherits the incompleteness
            // across the crate boundary (sweep [30]); the gate already fails closed locally on it.
            incomplete: incompleteacc.get(q).map(|s| s.iter().map(|e| e.to_string()).collect()).unwrap_or_default(),
        });
    }
    entries.sort_by(|a, b| a.func.cmp(&b.func));

    let meta = candor_report::ReportMeta {
        version: format!("scan-{}", env!("CARGO_PKG_VERSION")),
        toolchain: "stable".into(),
        spec: candor_report::SPEC_VERSION.into(),
    };
    let body = candor_report::to_packaged_report_json(&meta, &crate_name, &entries).unwrap_or_default();
    // With want_json the body is RETURNED to the caller (which prints one document for a single
    // crate, or wraps N members in a JSON array) rather than printed here — printing per-call gave
    // concatenated, unparseable JSON for a workspace scan.
    let json_body = if want_json {
        Some(body.clone())
    } else {
        let prefix = if prefix.is_empty() { format!("{dir}/.candor/report") } else { prefix };
        if let Some(parent) = Path::new(&prefix).parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let file = format!("{prefix}.{crate_name}.scan.json");
        // Atomic write (temp + rename): a concurrent `candor-query` / `cargo candor watch` reader must
        // never see a half-written report (see candor_report::write_atomic).
        let _ = candor_report::write_atomic(Path::new(&file), body.as_bytes());
        let cgfile = format!("{prefix}.{crate_name}.scan.callgraph.json");
        let _ = candor_report::write_atomic(Path::new(&cgfile), serde_json::to_string(&cg).unwrap_or_default().as_bytes());
        if !quiet {
            eprintln!(
                "candor-scan: wrote {} effectful functions to {file} (stable syntactic backend — see --help)",
                entries.len()
            );
            // Effect breakdown — make the result visible at a glance, not just a count + a file path.
            let mut counts: std::collections::HashMap<&str, usize> = std::collections::HashMap::new();
            for e in &entries {
                for x in &e.inferred {
                    *counts.entry(x.as_str()).or_insert(0) += 1;
                }
            }
            let breakdown = ["Net", "Fs", "Db", "Exec", "Ipc", "Env", "Clipboard", "Clock", "Log", "Rand"]
                .iter()
                .filter_map(|k| counts.get(k).map(|n| format!("{k} {n}")))
                .collect::<Vec<_>>()
                .join(" · ");
            let unknown = counts.get("Unknown").copied().unwrap_or(0);
            if !breakdown.is_empty() || unknown > 0 {
                let u = if unknown > 0 {
                    format!("{}Unknown {unknown} (disclosed)", if breakdown.is_empty() { "" } else { "   ·   " })
                } else {
                    String::new()
                };
                eprintln!("  {breakdown}{u}");
            }
        }
        None
    };

    // The κ-coverage disclosure: dependencies the code demonstrably CALLS that the classifier knows
    // nothing about. Their effects are INVISIBLE — not Unknown — so the report's silence about them
    // is not purity evidence. This turns the curated-κ caveat from a doc footnote into per-scan,
    // named evidence (the argon2 lesson: the blind spot landed on exactly the call a security review
    // cared about).
    let mut unlisted: Vec<(&String, usize)> = dep_seen
        .iter()
        .filter(|(cr, _)| {
            !dep_classified.contains(*cr)
                // A crate with a loaded sibling report is COVERED even when no join fired: the
                // report omits pure functions, so join-less calls are its honest purity claim —
                // the opposite of invisible. (Without this, --deps named serde_json a blind spot.)
                // A RENAMED dep is covered under its real package name.
                && !deps_idx.crates.contains(dep_renames.get(cr.as_str()).map(String::as_str).unwrap_or(cr.as_str()))
                && !candor_classify::CALIBRATED_CRATES.contains(&cr.as_str())
                && !candor_classify::PATH_CALIBRATED_CRATES.contains(&cr.as_str())
                && !candor_classify::CALIBRATED_PREFIXES.iter().any(|p| cr.starts_with(p))
        })
        .map(|(cr, n)| (cr, *n))
        .collect();
    if !unlisted.is_empty() && !quiet {
        unlisted.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(b.0)));
        let shown: Vec<String> =
            unlisted.iter().take(8).map(|(cr, n)| format!("{cr} ({n} call{})", if *n == 1 { "" } else { "s" })).collect();
        let more = if unlisted.len() > 8 { format!(" + {} more", unlisted.len() - 8) } else { String::new() };
        eprintln!(
            "candor-scan: κ doesn't know {} dependenc{} this code calls into — effects through {} are INVISIBLE (not Unknown): {}{}",
            unlisted.len(),
            if unlisted.len() == 1 { "y" } else { "ies" },
            if unlisted.len() == 1 { "it" } else { "them" },
            shown.join(", "),
            more
        );
    }

    // The stable policy gate (spec §6.2 / AS-EFF-006/008/009) — the ADVISORY FLOOR. The syntactic
    // backend under-reports (a missed effect can pass), so this is a floor, never the sound gate
    // (that's the nightly engine / the JVM engine). It still catches every boundary crossing the
    // scan CAN see, deterministically, with zero extra install.
    if let Some(pp) = policy_path {
        let Ok(text) = std::fs::read_to_string(&pp) else {
            // A set-but-unreadable policy must be LOUD — silently passing would let a violation ship.
            eprintln!("candor-scan: policy {pp:?} could not be read; gate NOT enforced");
            return (2, json_body);
        };
        let v = policy_violations(&text, &all, &inferred, &calls, &hostsacc, &cmdsacc, &pathsacc, &tablesacc, &incompleteacc);
        // Human gate output (the violation lines AND the ✓/count summary) goes to STDERR whenever
        // `want_json`, so stdout stays a single pure JSON document (pipeable to `jq`). Without this,
        // a gated `--json` run interleaves violation text into the JSON stream and corrupts it.
        for gv in &v {
            let line = format!("[{}] {}", gv.rule, gv.detail);
            if want_json {
                eprintln!("{line}");
            } else {
                println!("{line}");
            }
        }
        // A configured gate over INCOMPLETE analysis (a source file failed to parse) must NOT report
        // green: the unparsed file's effects are absent, so a `policy ✓` over it is a false-pure. Fail
        // exit 2 (mirroring the unreadable-policy posture) — never exit 0/1 with a clean-looking ✓. No
        // --gate-json verdict here: the analysis is incomplete, so there is no faithful verdict to emit.
        if had_parse_failure {
            eprintln!("candor-scan: policy NOT enforced — source failed to parse (see above); gate cannot be green over unanalyzed code");
            return (2, json_body);
        }
        record_gate_violations(&v); // toward the final --gate-json verdict (written once, by scan_main)
        if v.is_empty() {
            eprintln!("candor-scan: policy ✓ (advisory floor — the syntactic backend under-reports; the nightly engine is the sound gate)");
        } else {
            eprintln!("candor-scan: {} policy violation(s) (advisory floor — a clean run is necessary, not sufficient)", v.len());
            return (1, json_body);
        }
    }
    // No-gate runs record nothing; scan_main's final write_gate_json emits { ok: true, [] } for them.
    (0, json_body)
}

/// Scan a TARGET — a single crate, or a `[workspace]` root fanned out into one report per member
/// under the shared prefix. The one place both the plain and `--deps` paths funnel through, so a
/// workspace is never scanned as one merged package (colliding same-named fns) nor pruned to an
/// empty report by the nested-package filter. With `want_json`, prints ONE JSON document for a
/// single crate and a JSON ARRAY for a workspace — never concatenated documents. Returns the exit code.
fn scan_target(
    dir: &str,
    prefix: String,
    want_json: bool,
    include_tests: bool,
    policy: Option<String>,
    deps_idx: &DepIndex,
) -> i32 {
    let members = workspace_members(Path::new(dir));
    if members.is_empty() {
        if has_workspace_table(Path::new(dir)) {
            // A [workspace] with zero RESOLVED members: scanning the root as one crate would let the
            // nested-package filter prune every member into an empty report that passes any gate
            // vacuously (§6.2's forbidden state). Warn loudly; the single-crate scan below still
            // covers the root package's own sources, if any.
            eprintln!("candor-scan: `{dir}` declares [workspace] but no members resolved — \
                       check `members`/globs; scan member crates directly to gate them");
        }
        let (code, json) = scan_one(dir, ScanOpts {
            prefix, want_json, include_tests, policy, quiet: false, deps_idx,
        });
        if let Some(b) = json {
            println!("{b}");
        }
        return code;
    }
    let prefix = if prefix.is_empty() { format!("{dir}/.candor/report") } else { prefix };
    let mut dirs: Vec<String> = Vec::new();
    if read_crate_name(Path::new(dir)).is_some() {
        dirs.push(dir.to_string()); // the workspace manifest also declares a root package
    }
    dirs.extend(members);
    let mut rc = 0;
    let mut bodies: Vec<String> = Vec::new();
    for d in &dirs {
        let (code, json) = scan_one(d, ScanOpts {
            prefix: prefix.clone(), want_json, include_tests, policy: policy.clone(), quiet: false, deps_idx,
        });
        rc = rc.max(code);
        if let Some(b) = json {
            bodies.push(b);
        }
    }
    if want_json {
        println!("[{}]", bodies.join(","));
    } else {
        eprintln!("candor-scan: workspace — {} package report(s) under one prefix", dirs.len());
    }
    rc
}

/// `--deps`: read Cargo.lock, scan every REGISTRY dependency's unbuilt source from
/// `~/.cargo/registry/src/<index>/` into `<dir>/.candor/deps/`, then scan the root crate chained
/// over those reports (plus anything CANDOR_DEPS already names). Path/git/workspace deps have no
/// registry checkout and are skipped with a note — chain them by scanning them yourself.
fn run_with_deps(dir: &str, prefix: String, want_json: bool, include_tests: bool, policy: Option<String>) -> i32 {
    let lock = match std::fs::read_to_string(format!("{dir}/Cargo.lock")) {
        Ok(t) => t,
        Err(_) => {
            eprintln!("candor-scan: --deps needs {dir}/Cargo.lock (run `cargo generate-lockfile` first)");
            return 2;
        }
    };
    // [[package]] blocks: name + version + source. Only registry deps have a checkout to scan;
    // the root crate itself has no `source` line and is naturally skipped.
    let mut pkgs: Vec<(String, String)> = Vec::new();
    let (mut name, mut version, mut registry) = (String::new(), String::new(), false);
    let flush = |name: &mut String, version: &mut String, registry: &mut bool, pkgs: &mut Vec<(String, String)>| {
        if *registry && !name.is_empty() && !version.is_empty() {
            pkgs.push((name.clone(), version.clone()));
        }
        name.clear();
        version.clear();
        *registry = false;
    };
    for line in lock.lines() {
        let l = line.trim();
        if l == "[[package]]" {
            flush(&mut name, &mut version, &mut registry, &mut pkgs);
        } else if let Some(v) = l.strip_prefix("name = ") {
            name = v.trim_matches('"').to_string();
        } else if let Some(v) = l.strip_prefix("version = ") {
            version = v.trim_matches('"').to_string();
        } else if l.starts_with("source = ") && l.contains("registry+") {
            registry = true;
        }
    }
    flush(&mut name, &mut version, &mut registry, &mut pkgs);

    let registry_roots: Vec<std::path::PathBuf> = dirs_cargo_registry_src();
    let deps_dir = format!("{dir}/.candor/deps");
    let _ = std::fs::create_dir_all(&deps_dir);
    let (mut scanned, mut cached, mut missing) = (0usize, 0usize, Vec::new());
    let no_deps = DepIndex::default();
    for (n, v) in &pkgs {
        let Some(src) = registry_roots.iter().map(|r| r.join(format!("{n}-{v}"))).find(|p| p.is_dir()) else {
            missing.push(format!("{n}-{v}"));
            continue;
        };
        // One subdirectory PER name@version: two locked versions of one crate must not overwrite
        // each other's report (review: last-write-wins silently fed the root the wrong version's
        // effects); with both present, conflicting keys drop as ambiguous — never-guess intact.
        let sub = format!("{deps_dir}/{n}@{v}");
        let already = std::fs::read_dir(&sub).ok().is_some_and(|rd| {
            rd.flatten().any(|e| {
                let f = e.file_name();
                let f = f.to_string_lossy();
                f.ends_with(".scan.json") && !f.contains("callgraph")
            })
        });
        if already {
            cached += 1; // registry checkouts are immutable per name@version — the report stands
            continue;
        }
        let _ = std::fs::create_dir_all(&sub);
        // Dep scans are quiet, unchained, report-only, and POLICY-FREE (the resolved root policy
        // is deliberately not passed): their job is the report files. A registry dep is a single
        // published package, so scan_one (not scan_target) is right; the json body is unused.
        let _ = scan_one(&src.to_string_lossy(), ScanOpts {
            prefix: format!("{sub}/report"),
            want_json: false,
            include_tests: false,
            policy: None,
            quiet: true,
            deps_idx: &no_deps,
        });
        scanned += 1;
    }
    eprintln!(
        "candor-scan: --deps scanned {scanned} of {} registry dependencies into {deps_dir}{}{} \
(floor-engine reports: a dep's silent misses pass through — the κ caveat applies to the chain too)",
        pkgs.len(),
        if cached > 0 { format!(" ({cached} already scanned — cached)") } else { String::new() },
        if missing.is_empty() {
            String::new()
        } else {
            format!(" ({} without a local checkout: {}{})", missing.len(),
                missing.iter().take(5).cloned().collect::<Vec<_>>().join(", "),
                if missing.len() > 5 { ", …" } else { "" })
        }
    );
    // Chain the fresh dep reports (plus anything CANDOR_DEPS already names) under the root scan.
    // load_dep_reports dedups canonical paths, so deps_dir appearing in CANDOR_DEPS too is safe.
    let spec = match std::env::var("CANDOR_DEPS") {
        Ok(extra) if !extra.is_empty() => format!("{deps_dir}:{extra}"),
        _ => deps_dir.clone(),
    };
    let idx = load_dep_reports(Some(&spec));
    // The final root scan goes through scan_target so `--deps <workspace>` fans out over members
    // too — the nested-package filter would otherwise prune them all into an empty, gate-passing report.
    scan_target(dir, prefix, want_json, include_tests, policy, &idx)
}

/// The cargo registry source roots (`~/.cargo/registry/src/<index-hash>/`), where unbuilt
/// dependency sources live. CARGO_HOME is honoured.
fn dirs_cargo_registry_src() -> Vec<std::path::PathBuf> {
    let home = std::env::var("CARGO_HOME")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|_| Path::new(&std::env::var("HOME").unwrap_or_default()).join(".cargo"));
    std::fs::read_dir(home.join("registry").join("src"))
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.is_dir())
        .collect()
}

/// One structured gate violation (candor-spec §3.3 ⟨0.8⟩): `effects` is the specific effect set the
/// violation concerns — the denied set (006), the allowed effect (008), or [] (009 layer-flow, no single
/// effect); `detail` is the message BODY (no `[AS-EFF-00x]` prefix — the rule carries the code). The
/// console gate prints `[{rule}] {detail}`; --gate-json serializes these records verbatim.
#[derive(serde::Serialize, Clone)]
struct GateViolation {
    rule: String,
    #[serde(rename = "fn")]
    func: String,
    effects: Vec<String>,
    detail: String,
}

/// Evaluate a CANDOR_POLICY (parsed by the SHARED §6.2 parser in candor-classify, so this gate can
/// never disagree with the nightly/JVM gates on grammar) over a finished scan. Returns one line per
/// violation: deny/pure (AS-EFF-006) against the transitive `inferred` sets, literal allowlists
/// (AS-EFF-008) against the transitive hosts/cmds/paths/tables surfaces, layering `forbid A -> B`
/// (AS-EFF-009) by reachability over the local call graph.
#[allow(clippy::too_many_arguments)]
fn policy_violations(
    policy_text: &str,
    all: &[String],
    inferred: &HashMap<String, BTreeSet<&'static str>>,
    calls: &HashMap<String, BTreeSet<String>>,
    hostsacc: &HashMap<String, BTreeSet<String>>,
    cmdsacc: &HashMap<String, BTreeSet<String>>,
    pathsacc: &HashMap<String, BTreeSet<String>>,
    tablesacc: &HashMap<String, BTreeSet<String>>,
    incompleteacc: &HashMap<String, BTreeSet<&'static str>>,
) -> Vec<GateViolation> {
    use candor_classify::policy::{literal_allowed, parse_policy, scope_matches};
    let p = parse_policy(policy_text);
    let empty: BTreeSet<&'static str> = BTreeSet::new();
    let mut out = Vec::new();
    for q in all {
        let inf = inferred.get(q).unwrap_or(&empty);
        // AS-EFF-006 — deny/pure: forbidden effects in the transitive set.
        for r in &p.rules {
            if let Some(s) = &r.scope {
                if !scope_matches(q, s) {
                    continue;
                }
            }
            let hits: Vec<&str> = if r.effects.is_empty() {
                inf.iter().copied().collect() // `pure` — ANY effect (Unknown included: not certifiably pure)
            } else {
                inf.iter().copied().filter(|e| r.effects.contains(e)).collect()
            };
            if !hits.is_empty() {
                out.push(GateViolation {
                    rule: "AS-EFF-006".into(),
                    func: q.clone(),
                    effects: hits.iter().map(|s| s.to_string()).collect(),
                    detail: format!("`{q}` performs {{ {} }}, forbidden by policy: `{}`", hits.join(", "), r.raw),
                });
            }
        }
        // AS-EFF-008 — literal allowlists over the transitive literal surfaces.
        for r in &p.allow_rules {
            if let Some(s) = &r.scope {
                if !scope_matches(q, s) {
                    continue;
                }
            }
            if !inf.contains(r.effect) {
                continue;
            }
            let lits = match r.effect {
                "Net" => hostsacc.get(q),
                "Exec" => cmdsacc.get(q),
                "Db" => tablesacc.get(q),
                _ => pathsacc.get(q),
            };
            // An INCOMPLETE surface (a structurally-invisible reach) can't be certified even with visible
            // hosts — else a benign literal masks the invisible forbidden endpoint (the masking evasion).
            let surface_incomplete = incompleteacc.get(q).is_some_and(|s| s.contains(r.effect));
            match lits {
                Some(ls) if !ls.is_empty() && !surface_incomplete => {
                    let bad: Vec<&str> =
                        ls.iter().filter(|l| !literal_allowed(r.effect, l, &r.literals)).map(String::as_str).collect();
                    if !bad.is_empty() {
                        out.push(GateViolation {
                            rule: "AS-EFF-008".into(),
                            func: q.clone(),
                            effects: vec![r.effect.to_string()],
                            detail: format!("`{q}` reaches {{ {} }} outside the allowlist: `{}`", bad.join(", "), r.raw),
                        });
                    }
                }
                _ => out.push(GateViolation {
                    rule: "AS-EFF-008".into(),
                    func: q.clone(),
                    effects: vec![r.effect.to_string()],
                    detail: format!("`{q}` performs {} with no visible literal — the surface cannot be certified: `{}`", r.effect, r.raw),
                }),
            }
        }
        // AS-EFF-009 — layering: no fn in scope A may transitively reach scope B.
        for r in &p.layer_rules {
            if !scope_matches(q, &r.from) {
                continue;
            }
            let mut seen: BTreeSet<&str> = BTreeSet::new();
            let mut stack: Vec<&str> = calls.get(q).map(|cs| cs.iter().map(String::as_str).collect()).unwrap_or_default();
            let mut hit: Option<&str> = None;
            while let Some(n) = stack.pop() {
                if !seen.insert(n) {
                    continue;
                }
                if scope_matches(n, &r.to) {
                    hit = Some(n);
                    break;
                }
                if let Some(cs) = calls.get(n) {
                    stack.extend(cs.iter().map(String::as_str));
                }
            }
            if let Some(h) = hit {
                out.push(GateViolation {
                    rule: "AS-EFF-009".into(),
                    func: q.clone(),
                    effects: Vec::new(), // a layer-flow has no single effect
                    detail: format!("`{q}` reaches into a forbidden layer (via `{h}`): `{}`", r.raw),
                });
            }
        }
    }
    // Sort by the rendered console line so ordering is identical to the old Vec<String> sort.
    out.sort_by(|a, b| format!("[{}] {}", a.rule, a.detail).cmp(&format!("[{}] {}", b.rule, b.detail)));
    out
}

/// `--gate-json <file>` target, set once in `scan_main` (a no-op when unset — the direct-`scan_one` test
/// paths never write). Mirrors the `CFG_FEATURES` OnceLock idiom; a plain path so it threads no ScanOpts.
static GATE_JSON_PATH: std::sync::OnceLock<Option<String>> = std::sync::OnceLock::new();

/// Violations ACCUMULATED across `scan_one` calls. A `[workspace]` root runs the gate once per member;
/// writing the verdict per member let the LAST member overwrite the first's violations — `gate.json` said
/// `ok: true` while the process exited 1 (a clean final member masked an earlier violator), violating the
/// §3.3 "verdict MUST agree with the exit code" rule. So members only RECORD here; `scan_main` writes ONCE.
static GATE_VIOLATIONS: std::sync::OnceLock<std::sync::Mutex<Vec<GateViolation>>> = std::sync::OnceLock::new();

/// Record one scan's gate violations toward the final `--gate-json` verdict. A no-op unless the flag was
/// given (the direct-`scan_one` test/selftest paths never record).
fn record_gate_violations(violations: &[GateViolation]) {
    if !matches!(GATE_JSON_PATH.get(), Some(Some(_))) {
        return;
    }
    let acc = GATE_VIOLATIONS.get_or_init(|| std::sync::Mutex::new(Vec::new()));
    acc.lock().unwrap().extend(violations.iter().cloned());
}

/// Write the structured gate verdict `{ spec, ok, violations }` (candor-spec §3.3 ⟨0.8⟩) — the machine
/// analog of the AS-EFF console lines, accumulated from the SAME `policy_violations` that set the exit
/// code, so it can never disagree with the gate. Called ONCE, by `scan_main`, after the whole scan (every
/// workspace member) completes. `-` streams to stdout. On exit 2 (an incomplete scan/gate — unreadable
/// policy, a parse failure) NO verdict is written: there is no faithful verdict to emit. A no-op unless
/// `--gate-json` was given.
fn write_gate_json(exit_code: i32) {
    let Some(Some(path)) = GATE_JSON_PATH.get() else { return };
    if exit_code == 2 {
        eprintln!("candor-scan: --gate-json not written — the scan/gate did not complete (exit 2)");
        return;
    }
    let acc = GATE_VIOLATIONS.get_or_init(|| std::sync::Mutex::new(Vec::new()));
    let violations = acc.lock().unwrap();
    #[derive(serde::Serialize)]
    struct Verdict<'a> {
        spec: &'static str,
        ok: bool,
        violations: &'a [GateViolation],
    }
    let verdict = Verdict { spec: candor_report::SPEC_VERSION, ok: violations.is_empty(), violations: &violations };
    match serde_json::to_string_pretty(&verdict) {
        Ok(json) if path == "-" => println!("{json}"),
        Ok(json) => {
            if let Err(e) = candor_report::write_atomic(std::path::Path::new(path), format!("{json}\n").as_bytes()) {
                eprintln!("candor-scan: could not write --gate-json {path}: {e}");
            }
        }
        Err(e) => eprintln!("candor-scan: could not serialize gate verdict: {e}"),
    }
}

/// The last two `::`-segments of a path (`a::b::Type::new` → `Type::new`), the key used to resolve a
/// `Type::method` call to its definition without colliding every same-named method. `None` for a path
/// with fewer than two segments (a bare leaf — only an unqualified FREE call resolves by leaf; a bare
/// method call with an unresolved receiver under-reports, see `resolve_target`).
fn tail2(path: &str) -> Option<String> {
    let segs: Vec<&str> = path.split("::").collect();
    let n = segs.len();
    if n < 2 {
        return None;
    }
    Some(format!("{}::{}", segs[n - 2], segs[n - 1]))
}

/// A CONSUMING iterator combinator: one that drives `Iterator::next` to completion (or short-circuits
/// after forcing some elements). Calling one on a custom-iterator value runs its `next` — so if the
/// receiver is a concrete local `impl Iterator`, the consumer reaches that `next`'s effect (handled by
/// `charge_iter_next`). This is the EAGER/forcing subset only: lazy ADAPTERS (`map`/`filter`/`take`/…)
/// return a new lazy iterator and do NOT force, so they are deliberately ABSENT — charging them would
/// over-approximate a never-driven chain. `collect` is included (it forces); a never-consumed `collect`
/// result is vanishingly rare and forcing is the safe direction. `next`/`next_back` are also absent —
/// an explicit `.next()` already resolves as an ordinary method call on the receiver type.
fn is_iter_consumer(leaf: &str) -> bool {
    matches!(
        leaf,
        "collect"
            | "count"
            | "sum"
            | "product"
            | "for_each"
            | "try_for_each"
            | "last"
            | "nth"
            | "fold"
            | "try_fold"
            | "reduce"
            | "min"
            | "max"
            | "min_by"
            | "max_by"
            | "min_by_key"
            | "max_by_key"
            | "all"
            | "any"
            | "find"
            | "find_map"
            | "position"
            | "rposition"
            | "partition"
            | "unzip"
            | "collect_into"
    )
}

/// A FORMATTING macro: one whose `{}`/`{:?}` args are run through `Display::fmt`/`Debug::fmt` (#2). The
/// std family `format!`/`format_args!`/`print!`/`println!`/`eprint!`/`eprintln!`/`write!`/`writeln!` plus
/// the very common `panic!`/`assert!` family and `.to_string()` (handled at the method site, not here).
/// Only these implicitly format — a non-format macro never reaches a `Display`/`Debug` impl this way.
fn is_format_macro(leaf: &str) -> bool {
    matches!(
        leaf,
        "format"
            | "format_args"
            | "print"
            | "println"
            | "eprint"
            | "eprintln"
            | "write"
            | "writeln"
            | "panic"
            | "unreachable"
            | "todo"
            | "unimplemented"
            | "assert"
            | "assert_eq"
            | "assert_ne"
            | "debug_assert"
            | "debug_assert_eq"
            | "debug_assert_ne"
    )
}

/// The (trait leaf, method) a binary operator overloads to, for the operator-overload coercion (#4). The
/// dispatch receiver is the LEFT operand (Rust resolves `a OP b` as `<A>::method(a, b)`). Comparison ops
/// route through `PartialOrd::partial_cmp`/`PartialEq::eq` (the impl method, not the per-op `lt`/`gt`
/// which forward to it). Lazy boolean `&&`/`||`, assignment, and the compound-assign ops (`+=` is
/// `AddAssign`, a distinct family left as an honest residual) return None — no overload edge.
fn binop_trait(op: &syn::BinOp) -> Option<(&'static str, &'static str)> {
    use syn::BinOp;
    Some(match op {
        BinOp::Add(_) => ("Add", "add"),
        BinOp::Sub(_) => ("Sub", "sub"),
        BinOp::Mul(_) => ("Mul", "mul"),
        BinOp::Div(_) => ("Div", "div"),
        BinOp::Rem(_) => ("Rem", "rem"),
        BinOp::BitAnd(_) => ("BitAnd", "bitand"),
        BinOp::BitOr(_) => ("BitOr", "bitor"),
        BinOp::BitXor(_) => ("BitXor", "bitxor"),
        BinOp::Shl(_) => ("Shl", "shl"),
        BinOp::Shr(_) => ("Shr", "shr"),
        BinOp::Eq(_) | BinOp::Ne(_) => ("PartialEq", "eq"),
        BinOp::Lt(_) | BinOp::Le(_) | BinOp::Gt(_) | BinOp::Ge(_) => ("PartialOrd", "partial_cmp"),
        _ => return None,
    })
}

/// True if `expr` is a `<recv>.into()` method call (no args), peeling effect-transparent wrappers — the
/// `.into()` coercion (#5) whose `From::from` target is the binding's annotated type.
fn expr_is_into_call(expr: &syn::Expr) -> bool {
    match expr {
        syn::Expr::MethodCall(m) => m.method == "into" && m.args.is_empty(),
        syn::Expr::Reference(r) => expr_is_into_call(&r.expr),
        syn::Expr::Paren(p) => expr_is_into_call(&p.expr),
        syn::Expr::Group(g) => expr_is_into_call(&g.expr),
        _ => false,
    }
}

/// Which argument a format `{…}` hole references.
enum FmtArg {
    /// a bare `{}` / `{:?}` — the next positional value arg in order
    Implicit,
    /// an explicit positional index `{0}` / `{1:?}`
    Index(usize),
    /// a named or inline-captured hole (`{name}`, `{x:?}`) — references a binding, not a value arg
    Named,
}

/// One parsed `{…}` hole of a format string: which arg it draws, and whether it requests `Debug` (`{:?}`/
/// `{:#?}`) rather than `Display`.
struct FmtHole {
    arg: FmtArg,
    debug: bool,
}

/// Parse the `{…}` holes of a format string (`std::fmt` mini-grammar, the subset that matters for picking
/// the formatter trait). Handles `{{`/`}}` escapes, implicit (`{}`) vs indexed (`{0}`) vs named (`{x}`)
/// argument refs, and detects `Debug` via a `?`/`#?` type in the format spec after `:`. We do NOT resolve
/// width/precision `$`-args (a `{:.*}` / `{:1$}` extra positional) — at worst that misaligns one implicit
/// index, a benign miss (an edge to the wrong-but-also-local arg, or none), never a fabrication on a
/// non-local type. Best-effort and forgiving: a malformed hole is skipped.
fn parse_format_holes(fmt: &str) -> Vec<FmtHole> {
    let mut holes = Vec::new();
    let bytes: Vec<char> = fmt.chars().collect();
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            '{' => {
                if bytes.get(i + 1) == Some(&'{') {
                    i += 2; // escaped `{{`
                    continue;
                }
                // Read until the matching `}`.
                let start = i + 1;
                let mut j = start;
                while j < bytes.len() && bytes[j] != '}' {
                    j += 1;
                }
                if j >= bytes.len() {
                    break; // unterminated — malformed, stop
                }
                let inner: String = bytes[start..j].iter().collect();
                // Split off the format SPEC after the first `:`.
                let (name_part, spec) = match inner.split_once(':') {
                    Some((n, s)) => (n.trim(), s),
                    None => (inner.trim(), ""),
                };
                // Debug = the spec's type char is `?` (optionally after `#` for pretty `{:#?}`), i.e. the
                // spec (after stripping fill/align/flags/width/precision) ENDS in `?`. A simple, robust
                // test: the spec contains `?` as its trailing type.
                let debug = spec.trim_end().ends_with('?');
                let arg = if name_part.is_empty() {
                    FmtArg::Implicit
                } else if let Ok(idx) = name_part.parse::<usize>() {
                    FmtArg::Index(idx)
                } else {
                    FmtArg::Named
                };
                holes.push(FmtHole { arg, debug });
                i = j + 1;
            }
            '}' => {
                i += if bytes.get(i + 1) == Some(&'}') { 2 } else { 1 }; // escaped `}}` or stray
            }
            _ => i += 1,
        }
    }
    holes
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
fn resolve_target<'a>(
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

fn host_part(h: &str) -> String {
    let a = h.split_once("://").map(|(_, r)| r).unwrap_or(h);
    let a = a.split('/').next().unwrap_or(a);
    a.rsplit_once('@').map(|(_, h)| h).unwrap_or(a).to_string()
}

fn read_crate_name(root: &Path) -> Option<String> {
    let txt = std::fs::read_to_string(root.join("Cargo.toml")).ok()?;
    let mut in_package = false;
    for line in txt.lines() {
        if let Some(section) = toml_section(line) {
            in_package = section == "package"; // only [package]'s `name` is the crate name
            continue;
        }
        // `name` inside `[package]` only (a `name =` in `[[bin]]`/`[dependencies]` is not the crate name).
        if in_package {
            if let Some(v) = toml_scalar(line, "name") {
                return Some(v.replace('-', "_"));
            }
        }
    }
    None
}

/// The string entries of `key = [ ... ]` inside `[table]` — line-based (the manifest subset that
/// matters), multi-line arrays included. No TOML dependency, same trade as the parsers above.
fn toml_string_array(txt: &str, table: &str, key: &str) -> Vec<String> {
    let (mut in_table, mut collecting) = (false, false);
    let mut out = Vec::new();
    for line in txt.lines() {
        let l = line.trim();
        if !collecting {
            if let Some(section) = toml_section(line) {
                in_table = section == table;
                continue;
            }
        }
        if !in_table {
            continue;
        }
        let rest = if let Some(r) = l.strip_prefix(key) {
            let r = r.trim_start();
            let Some(r) = r.strip_prefix('=') else { continue };
            collecting = true;
            r
        } else if collecting {
            l
        } else {
            continue;
        };
        let mut parts = rest.split('"');
        parts.next();
        while let Some(s) = parts.next() {
            out.push(s.to_string());
            if parts.next().is_none() {
                break;
            }
        }
        if rest.contains(']') {
            collecting = false;
        }
    }
    out
}

/// True if the manifest declares a `[workspace]` table at all (distinct from "has members"): a
/// workspace root with zero RESOLVED members must warn, not silently fall through to a single-crate
/// scan whose nested-package filter then prunes every member into an empty report.
fn has_workspace_table(root: &Path) -> bool {
    std::fs::read_to_string(root.join("Cargo.toml"))
        .map(|t| t.lines().any(|l| l.trim() == "[workspace]"))
        .unwrap_or(false)
}

/// Member directories of the root manifest's `[workspace]`, joined to `root`, honouring `exclude`,
/// expanding globs (a bare `*` = root's immediate children, `prefix/*` = a dir's children), and
/// DEDUPLICATED (a member listed explicitly AND matched by a glob otherwise scans/prints twice).
/// Empty when there is no `members` key. A `*`-pattern this simple matcher can't expand is WARNED,
/// never silently dropped (a dropped member yields a vacuous gate, the §6.2 forbidden state).
fn workspace_members(root: &Path) -> Vec<String> {
    let Ok(txt) = std::fs::read_to_string(root.join("Cargo.toml")) else { return Vec::new() };
    let members = toml_string_array(&txt, "workspace", "members");
    if members.is_empty() {
        return Vec::new();
    }
    let exclude = toml_string_array(&txt, "workspace", "exclude");
    // Expand a `<base>/*` (base "" for a bare `*`) to its child dirs carrying a Cargo.toml.
    let expand = |base: &str| -> Vec<String> {
        let dir = if base.is_empty() { root.to_path_buf() } else { root.join(base) };
        let mut found: Vec<String> = std::fs::read_dir(dir)
            .into_iter()
            .flatten()
            .filter_map(Result::ok)
            .filter(|e| e.path().join("Cargo.toml").is_file())
            .map(|e| {
                let n = e.file_name().to_string_lossy().into_owned();
                if base.is_empty() { n } else { format!("{base}/{n}") }
            })
            .collect();
        found.sort();
        found
    };
    let mut rels: Vec<String> = Vec::new();
    for m in members {
        if m == "*" {
            rels.extend(expand(""));
        } else if let Some(base) = m.strip_suffix("/*") {
            rels.extend(expand(base));
        } else if m.contains('*') {
            eprintln!("candor-scan: workspace member glob `{m}` is not a trailing `*` — not expanded; \
                       scan its crates directly or list them explicitly");
        } else if root.join(&m).join("Cargo.toml").is_file() {
            rels.push(m);
        }
    }
    rels.retain(|m| !exclude.iter().any(|e| m == e || m.starts_with(&format!("{e}/"))));
    rels.sort();
    rels.dedup();
    rels.into_iter().map(|m| root.join(m).to_string_lossy().into_owned()).collect()
}

fn propagate(
    direct: &HashMap<String, BTreeSet<&'static str>>,
    calls: &HashMap<String, BTreeSet<String>>,
    all: &[String],
) -> HashMap<String, BTreeSet<&'static str>> {
    let mut acc = direct.clone();
    for f in all {
        acc.entry(f.clone()).or_default();
    }
    let mut changed = true;
    while changed {
        changed = false;
        for f in all {
            let add: BTreeSet<&'static str> = calls
                .get(f)
                .map(|cs| cs.iter().filter_map(|c| acc.get(c)).flatten().copied().collect())
                .unwrap_or_default();
            let e = acc.entry(f.clone()).or_default();
            let before = e.len();
            e.extend(add);
            if e.len() != before {
                changed = true;
            }
        }
    }
    acc
}

fn propagate_str(
    direct: &HashMap<String, BTreeSet<String>>,
    calls: &HashMap<String, BTreeSet<String>>,
    all: &[String],
) -> HashMap<String, BTreeSet<String>> {
    let mut acc = direct.clone();
    let mut changed = true;
    while changed {
        changed = false;
        for f in all {
            let add: BTreeSet<String> = calls
                .get(f)
                .map(|cs| cs.iter().filter_map(|c| acc.get(c)).flatten().cloned().collect())
                .unwrap_or_default();
            if add.is_empty() {
                continue;
            }
            let e = acc.entry(f.clone()).or_default();
            let before = e.len();
            e.extend(add);
            if e.len() != before {
                changed = true;
            }
        }
    }
    acc
}

#[cfg(test)]
mod tests {
    use super::*;

    fn uses(pairs: &[(&str, &str)]) -> HashMap<String, String> {
        pairs.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect()
    }

    /// A shared, empty lazy-static name set for direct `CallCollector` constructions in unit tests that
    /// don't exercise the lazy-forcing path (the lazy-forcing tests use the full `scan_one`/`scan_src`).
    fn empty_lazy() -> &'static std::collections::HashSet<String> {
        static EMPTY: std::sync::OnceLock<std::collections::HashSet<String>> = std::sync::OnceLock::new();
        EMPTY.get_or_init(std::collections::HashSet::new)
    }

    #[test]
    fn expand_uses_the_use_map_and_strips_local_prefixes() {
        let u = uses(&[("fs", "std::fs"), ("Command", "std::process::Command")]);
        assert_eq!(expand("fs::read_to_string", &u), "std::fs::read_to_string");
        assert_eq!(expand("Command::new", &u), "std::process::Command::new");
        // crate/self/super are local and stripped, leaving the rest unresolved (matched by leaf later)
        assert_eq!(expand("crate::pricing::priced", &u), "pricing::priced");
        assert_eq!(expand("self::helper", &u), "helper");
        // an unknown first segment passes through unchanged
        assert_eq!(expand("foo::bar", &u), "foo::bar");
    }

    #[test]
    fn collect_use_expands_groups_and_renames() {
        let mut out = HashMap::new();
        // `use std::process::{Command, Stdio as Pipe};`
        let tree: syn::UseTree = syn::parse_str("std::process::{Command, Stdio as Pipe}").unwrap();
        collect_use(&tree, String::new(), &mut out);
        assert_eq!(out.get("Command").map(String::as_str), Some("std::process::Command"));
        assert_eq!(out.get("Pipe").map(String::as_str), Some("std::process::Stdio"));
        // `use std::fs::{self, Metadata}` imports the MODULE `fs` itself → map `fs -> std::fs`.
        let mut o2 = HashMap::new();
        collect_use(&syn::parse_str("std::fs::{self, Metadata}").unwrap(), String::new(), &mut o2);
        assert_eq!(o2.get("fs").map(String::as_str), Some("std::fs"));
        assert_eq!(o2.get("Metadata").map(String::as_str), Some("std::fs::Metadata"));
        assert_eq!(o2.get("self"), None); // not the useless `fs::self`
    }

    #[test]
    fn module_path_mirrors_file_based_resolution() {
        assert_eq!(module_path(Path::new("src/lib.rs")), "");
        assert_eq!(module_path(Path::new("src/main.rs")), "");
        assert_eq!(module_path(Path::new("src/pricing.rs")), "pricing");
        assert_eq!(module_path(Path::new("src/billing/mod.rs")), "billing");
        assert_eq!(module_path(Path::new("src/billing/tax.rs")), "billing::tax");
        // a dotted file stem (tonic/prost gRPC codegen) is a nested module path, not one segment.
        assert_eq!(
            module_path(Path::new("src/generated/envoy.service.auth.v3.rs")),
            "generated::envoy::service::auth::v3"
        );
        // a WORKSPACE member's path anchors at its OWN `src/`, not the scan root — otherwise the dir
        // path (`crates/cli/src/decompress.rs`) mangles into `crates::cli::src::decompress`.
        assert_eq!(module_path(Path::new("crates/cli/src/decompress.rs")), "decompress");
        assert_eq!(module_path(Path::new("crates/ignore/src/walk.rs")), "walk");
        assert_eq!(module_path(Path::new("crates/core/src/main.rs")), "");
    }

    #[test]
    fn host_part_strips_scheme_path_and_userinfo() {
        assert_eq!(host_part("https://api.stripe.com/v1/charges"), "api.stripe.com");
        assert_eq!(host_part("user:pass@db.internal:5432"), "db.internal:5432");
        assert_eq!(host_part("example.com"), "example.com");
    }

    #[test]
    fn propagate_is_transitive_across_the_call_graph() {
        // leaf has Fs directly; mid calls leaf; top calls mid — both must inherit Fs.
        let mut direct: HashMap<String, BTreeSet<&'static str>> = HashMap::new();
        direct.insert("leaf".into(), ["Fs"].into_iter().collect());
        let mut calls: HashMap<String, BTreeSet<String>> = HashMap::new();
        calls.insert("mid".into(), ["leaf".to_string()].into_iter().collect());
        calls.insert("top".into(), ["mid".to_string()].into_iter().collect());
        let all = vec!["leaf".to_string(), "mid".to_string(), "top".to_string(), "pure".to_string()];
        let acc = propagate(&direct, &calls, &all);
        assert!(acc["leaf"].contains("Fs"));
        assert!(acc["mid"].contains("Fs"));
        assert!(acc["top"].contains("Fs"));
        assert!(acc["pure"].is_empty());
    }

    #[test]
    fn tail2_keys_on_the_qualified_method() {
        assert_eq!(tail2("a::b::RequestBuilder::new").as_deref(), Some("RequestBuilder::new"));
        assert_eq!(tail2("pricing::compute_price").as_deref(), Some("pricing::compute_price"));
        assert_eq!(tail2("send"), None); // a bare method leaf — no type qualifier to disambiguate
    }

    #[test]
    fn qualified_tail_disambiguates_same_named_methods() {
        // Two distinct `new`s; a `RequestBuilder::new` call must resolve to ONLY the RequestBuilder one,
        // never to every `*::new` (the leaf-collision over-connection that smeared one effect crate-wide).
        let fns = ["http::RequestBuilder::new", "body::Body::new"];
        let mut by_leaf: HashMap<String, Vec<String>> = HashMap::new();
        let mut by_tail2: HashMap<String, Vec<String>> = HashMap::new();
        for q in fns {
            by_leaf.entry("new".into()).or_default().push(q.into());
            by_tail2.entry(tail2(q).unwrap()).or_default().push(q.into());
        }
        // a `RequestBuilder::new(...)` call — routed through PRODUCTION `resolve_target` (qualified tail).
        assert_eq!(resolve_target("api::RequestBuilder::new", "new", false, &by_tail2, &by_leaf),
                   Some(&vec!["http::RequestBuilder::new".to_string()]));
        // a bare `.new()`-by-leaf with two candidates resolves to NEITHER (ambiguous → under-report)
        assert_eq!(resolve_target("new", "new", true, &by_tail2, &by_leaf), None);
    }

    #[test]
    fn macro_bodies_are_walked_for_hidden_calls() {
        // git2 hides every libgit2 FFI call in `try_call!(...)`; format macros hide call args. Both
        // must be collected, while a non-expression macro body (matches!) is skipped without panicking.
        let uses = HashMap::new();
        let fields = FieldIndex::new();
        let block: syn::Block = syn::parse_str(
            "{ try_call!(raw::git_remote_fetch(x)); println!(\"{}\", helper()); let _ = matches!(y, Some(_)); }",
        )
        .unwrap();
        let returns = ReturnIndex::new();
        let (ti, td, tf) = (TraitImplIndex::new(), HashMap::new(), TraitFieldIndex::new());
        let (fe, ev) = (FieldElemIndex::new(), EnumVariantIndex::new());
        let mut c = CallCollector {
            uses: &uses,
            vars: HashMap::new(),
            trait_vars: HashMap::new(),
            fields: &fields,
            trait_fields: &tf,
            trait_impls: &ti,
            local_traits: &td,
            returns: &returns,
            field_elem: &fe,
            enum_variants: &ev,
            elem_of: HashMap::new(), tuple_of: HashMap::new(),
            calls: Vec::new(),
            closure_vars: std::collections::HashSet::new(),
            fn_typed_vars: std::collections::HashSet::new(),
            fn_alias: std::collections::HashMap::new(),
            lazy_statics: empty_lazy(),
            forced_lazies: std::collections::HashSet::new(),
            unresolved: false,
            err_ret_leaf: None,
        };
        for stmt in &block.stmts {
            c.visit_stmt(stmt);
        }
        let leaves: Vec<&str> = c.calls.iter().map(|c| c.leaf.as_str()).collect();
        assert!(leaves.contains(&"git_remote_fetch"), "call inside try_call! macro was missed: {leaves:?}");
        assert!(leaves.contains(&"helper"), "call inside println! macro was missed: {leaves:?}");
    }

    #[test]
    fn receiver_type_inference_resolves_method_dispatch() {
        // A method call on a param/field/typed-let of a known type resolves to `Type::method`, so the
        // existing per-crate rules fire. Build a collector with `client: reqwest::Client` in scope.
        let uses = HashMap::new();
        let mut fields = FieldIndex::new();
        // struct App { http: reqwest::Client }
        fields.entry("App".into()).or_default().insert("http".into(), "reqwest::Client".into());
        let mut vars = HashMap::new();
        vars.insert("client".to_string(), "reqwest::Client".to_string());
        vars.insert("self".to_string(), "App".to_string());
        let returns = ReturnIndex::new();
        let (ti, td, tf) = (TraitImplIndex::new(), HashMap::new(), TraitFieldIndex::new());
        let (fe, ev) = (FieldElemIndex::new(), EnumVariantIndex::new());
        let block: syn::Block =
            syn::parse_str("{ client.get(url).send(); self.http.execute(req); }").unwrap();
        let mut c = CallCollector {
            uses: &uses,
            vars,
            trait_vars: HashMap::new(),
            fields: &fields,
            trait_fields: &tf,
            trait_impls: &ti,
            local_traits: &td,
            returns: &returns,
            field_elem: &fe,
            enum_variants: &ev,
            elem_of: HashMap::new(), tuple_of: HashMap::new(),
            calls: Vec::new(),
            closure_vars: std::collections::HashSet::new(),
            fn_typed_vars: std::collections::HashSet::new(),
            fn_alias: std::collections::HashMap::new(),
            lazy_statics: empty_lazy(),
            forced_lazies: std::collections::HashSet::new(),
            unresolved: false,
            err_ret_leaf: None,
        };
        for stmt in &block.stmts {
            c.visit_stmt(stmt);
        }
        let typed: Vec<&str> = c.calls.iter().map(|c| c.path.as_str()).collect();
        // chain `client.get(url).send()` → base type reqwest::Client, terminal verb send
        assert!(typed.contains(&"reqwest::Client::send"), "chain not typed to base: {typed:?}");
        // field access `self.http.execute(req)` resolves via the struct field index
        assert!(typed.contains(&"reqwest::Client::execute"), "field recv not typed: {typed:?}");
        // and both classify as Net through the shared classifier
        assert_eq!(candor_classify::classify("reqwest", "reqwest::Client::send"), Some("Net"));
        assert_eq!(candor_classify::classify("reqwest", "reqwest::Client::execute"), Some("Net"));
    }

    #[test]
    fn cargo_toml_deps_handles_all_header_forms() {
        let mut out = std::collections::HashSet::new();
        let mut renames = HashMap::new();
        cargo_toml_deps(
            "[package]\nname = \"x\"\n\n[dependencies]\nserde_json = \"1\"\nleft-pad = \"1\"\n\n[dependencies.table-header]\nversion = \"1\"\n\n[target.'cfg(unix)'.dependencies.nix]\nversion = \"0.29\"\n\n[target.'cfg(windows)'.dependencies]\nwinapi = \"0.3\"\n\n[workspace.dependencies]\nshared-dep = \"2\"\n\n[dev-dependencies]\ncriterion = \"0.5\"\n\n[build-dependencies]\ncc = \"1\"\n\n[dev-dependencies.proptest]\nversion = \"1\"\n",
            &mut out,
            &mut renames,
        );
        for d in ["serde_json", "left_pad", "table_header", "nix", "winapi", "shared_dep"] {
            assert!(out.contains(d), "missing {d}: {out:?}");
        }
        for d in ["criterion", "cc", "proptest", "x"] {
            assert!(!out.contains(d), "harness/package dep leaked: {d}");
        }
        // dependency RENAMES, both forms (found live: ebman's `tui-common = { package = "tb-tui-common" }`)
        let mut out2 = std::collections::HashSet::new();
        let mut ren2 = HashMap::new();
        cargo_toml_deps(
            "[dependencies]\ntui-common = { version = \"0.1\", package = \"tb-tui-common\" }\n\n[dependencies.short-name]\nversion = \"1\"\npackage = \"the-real-crate\"\n",
            &mut out2,
            &mut ren2,
        );
        assert!(out2.contains("tui_common") && out2.contains("short_name"), "{out2:?}");
        assert_eq!(ren2.get("tui_common").map(String::as_str), Some("tb_tui_common"));
        assert_eq!(ren2.get("short_name").map(String::as_str), Some("the_real_crate"));
        // A dep whose KEY contains "package" must NOT parse its own version as a rename, and a dep
        // literally NAMED `package` is a dependency, not a rename (the substring-match regression).
        let mut out3 = std::collections::HashSet::new();
        let mut ren3 = HashMap::new();
        cargo_toml_deps(
            "[dependencies]\nmy-package = \"1.2\"\nfoo-package = { version = \"2\" }\npackage = \"0.1\"\nreal = { version = \"1\", package = \"renamed-crate\" }\n",
            &mut out3,
            &mut ren3,
        );
        assert!(out3.contains("my_package") && out3.contains("foo_package") && out3.contains("package"));
        assert!(!ren3.contains_key("my_package"), "key-substring 'package' fabricated a rename: {ren3:?}");
        assert!(!ren3.contains_key("foo_package"), "{ren3:?}");
        assert!(!ren3.contains_key("package"), "a dep named `package` is not a rename: {ren3:?}");
        assert_eq!(ren3.get("real").map(String::as_str), Some("renamed_crate"), "real rename lost: {ren3:?}");
    }

    #[test]
    fn dep_report_chaining_joins_unambiguously_and_distrusts_stale_versions() {
        let d = std::env::temp_dir().join(format!("candor-deps-test-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&d);
        let me = format!("scan-{}", env!("CARGO_PKG_VERSION"));
        // same-version report: effects + surfaces join; two fns sharing a leaf drop that key
        std::fs::write(d.join("report.billing.scan.json"), format!(r#"{{
            "candor": {{"version": "{me}", "toolchain": "stable", "spec": "0.3"}},
            "functions": [
              {{"fn": "ledger::Ledger::post", "inferred": ["Db"], "tables": ["ledger.entries"], "hash": "billing#ledger::Ledger::post"}},
              {{"fn": "a::dup", "inferred": ["Net"], "hash": "billing#a::dup"}},
              {{"fn": "b::dup", "inferred": ["Fs"], "hash": "billing#b::dup"}}
            ]}}"#)).unwrap();
        // a STALE producer version: §2.1 — downgraded to Unknown, never silently trusted
        std::fs::write(d.join("report.old_dep.scan.json"), r#"{
            "candor": {"version": "scan-0.0.1", "toolchain": "stable", "spec": "0.3"},
            "functions": [{"fn": "io::go", "inferred": ["Exec"], "hash": "old_dep#io::go"}]}"#).unwrap();
        let idx = load_dep_reports(Some(d.to_str().unwrap()));
        assert!(idx.crates.contains("billing") && idx.crates.contains("old_dep"));
        let post = idx.by_key.get("billing#Ledger::post").expect("tail2 key");
        assert_eq!(post.effects, vec!["Db"]);
        assert_eq!(post.tables, vec!["ledger.entries"]);
        assert!(idx.by_key.contains_key("billing#post"), "unambiguous leaf key");
        assert!(!idx.by_key.contains_key("billing#dup"), "shared leaf must be dropped, never guessed");
        assert!(idx.by_key.contains_key("billing#a::dup"), "tail2 still disambiguates the dups");
        let old = idx.by_key.get("old_dep#go").expect("stale entry present");
        assert_eq!(old.effects, vec!["Unknown"], "stale version must downgrade to Unknown");
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn dep_join_does_not_fabricate_onto_a_local_shadow() {
        // The CANDOR_DEPS cross-crate join must NOT override a LOCAL definition: a project module/fn named
        // like a covered dep crate, resolving to the project's OWN pure code, must not inherit the dep
        // report's effects (a cardinal-sin fabrication the join lacked the `resolved_local` guard for). A
        // GENUINE external call into the covered crate must still inherit.
        let dep = std::env::temp_dir().join(format!("candor-depjoin-rep-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dep);
        let me = format!("scan-{}", env!("CARGO_PKG_VERSION"));
        std::fs::write(dep.join("report.depb.scan.json"), format!(r#"{{
            "candor": {{"version": "{me}", "toolchain": "stable", "spec": "0.7"}},
            "functions": [{{"fn": "effectful_fn", "inferred": ["Net"], "hash": "depb#effectful_fn"}}]}}"#)).unwrap();
        let idx = load_dep_reports(Some(dep.to_str().unwrap()));
        assert!(idx.crates.contains("depb"));
        let run = |name: &str, src: &str| -> serde_json::Value {
            let d = std::env::temp_dir().join(format!("candor-depjoin-{name}-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&d);
            std::fs::create_dir_all(d.join("src")).unwrap();
            std::fs::write(d.join("Cargo.toml"), format!("[package]\nname = \"{name}\"\n")).unwrap();
            std::fs::write(d.join("src/lib.rs"), src).unwrap();
            let prefix = d.join("out/r").to_string_lossy().into_owned();
            let (rc, body) = scan_one(&d.to_string_lossy(), ScanOpts {
                prefix, want_json: true, include_tests: false, policy: None, quiet: true, deps_idx: &idx,
            });
            assert_eq!(rc, 0);
            let v = serde_json::from_str(&body.unwrap()).unwrap();
            let _ = std::fs::remove_dir_all(&d);
            v
        };
        let eff = |v: &serde_json::Value, needle: &str| -> Vec<String> {
            v["functions"].as_array().into_iter().flatten()
                .filter(|f| f["fn"].as_str().is_some_and(|q| q.contains(needle)))
                .flat_map(|f| f["inferred"].as_array().into_iter().flatten()
                    .filter_map(|e| e.as_str().map(String::from)).collect::<Vec<_>>()).collect()
        };
        // LOCAL module named like the dep crate → its caller must stay PURE (no Net from the join)
        let shadow = run("projsh", "mod depb { pub fn effectful_fn() -> i32 { 42 } }\npub fn uses_local() { let _ = depb::effectful_fn(); }");
        assert!(eff(&shadow, "uses_local").is_empty(),
                "dep-join FABRICATED the dep's effect onto a local shadow:\n{shadow}");
        // GENUINE external call into the covered crate → must STILL inherit Net
        let genuine = run("projgen", "pub fn calls_dep() { depb::effectful_fn(); }");
        assert!(eff(&genuine, "calls_dep").contains(&"Net".to_string()),
                "a genuine dep call must still inherit the dep report's Net:\n{genuine}");
        let _ = std::fs::remove_dir_all(&dep);
    }

    #[test]
    fn smart_pointer_receiver_resolves_pointee_method_but_not_clone() {
        // A method call on an `Arc<T>`/`Rc<T>`/`Box<T>` receiver auto-derefs to T's method, so it must
        // resolve the POINTEE's effect — not silently drop (the corpus-found §4 under-report: duct's
        // whole public API read pure because `self.0: Arc<ExpressionInner>`). BUT `.clone()` must NOT
        // resolve to `T::clone`: `arc.clone()` calls the pure `Arc::clone` (refcount), never the
        // pointee's clone, so resolving it would FABRICATE an effectful in-crate clone.
        let run = |name: &str, src: &str| -> serde_json::Value {
            let d = std::env::temp_dir().join(format!("candor-smartptr-{name}-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&d);
            std::fs::create_dir_all(d.join("src")).unwrap();
            std::fs::write(d.join("Cargo.toml"), format!("[package]\nname = \"{name}\"\n")).unwrap();
            std::fs::write(d.join("src/lib.rs"), src).unwrap();
            let prefix = d.join("out/r").to_string_lossy().into_owned();
            let idx = load_dep_reports(None);
            let (rc, body) = scan_one(&d.to_string_lossy(), ScanOpts {
                prefix, want_json: true, include_tests: false, policy: None, quiet: true, deps_idx: &idx,
            });
            assert_eq!(rc, 0);
            let v: serde_json::Value = serde_json::from_str(&body.unwrap()).unwrap();
            let _ = std::fs::remove_dir_all(&d);
            v
        };
        let eff = |v: &serde_json::Value, needle: &str| -> Vec<String> {
            v["functions"].as_array().into_iter().flatten()
                .filter(|f| f["fn"].as_str().is_some_and(|q| q.contains(needle)))
                .flat_map(|f| f["inferred"].as_array().into_iter().flatten()
                    .filter_map(|e| e.as_str().map(String::from)).collect::<Vec<_>>()).collect()
        };
        let src = r#"
use std::sync::Arc; use std::rc::Rc;
struct Inner;
impl Inner {
    fn doit(&self) { std::process::Command::new("ls").status().unwrap(); }
    fn clone(&self) -> Inner { std::fs::read("/x").unwrap(); Inner }
}
struct A(Arc<Inner>);
impl A { pub fn run(&self) { self.0.doit(); } pub fn dup(&self) -> Arc<Inner> { self.0.clone() } }
struct B(Box<Inner>); impl B { pub fn run(&self) { self.0.doit(); } }
struct R(Rc<Inner>); impl R { pub fn run(&self) { self.0.doit(); } }
"#;
        let v = run("smartptr", src);
        // auto-deref: the pointee's Exec is reached through Arc/Box/Rc receivers (was silently pure)
        assert!(eff(&v, "A::run").contains(&"Exec".to_string()), "Arc deref lost Exec:\n{v}");
        assert!(eff(&v, "B::run").contains(&"Exec".to_string()), "Box deref lost Exec:\n{v}");
        assert!(eff(&v, "R::run").contains(&"Exec".to_string()), "Rc deref lost Exec:\n{v}");
        // anti-fabrication: `arc.clone()` is the pure `Arc::clone`, never the effectful pointee clone
        assert!(eff(&v, "A::dup").is_empty(), "arc.clone() FABRICATED the pointee's clone effect:\n{v}");
    }

    #[test]
    fn custom_deref_resolves_pointee_method() {
        // A custom `impl Deref for W { type Target = Inner }` makes `w.method()` auto-deref to Inner's
        // method — it must reach the pointee's effect, not silently drop (the user-Deref analog of the
        // Box/Arc/Rc peel; a newtype `impl Deref` dropped `wrapper.method()` to silent-pure — corpus find).
        let run = |name: &str, src: &str| -> serde_json::Value {
            let d = std::env::temp_dir().join(format!("candor-deref-{name}-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&d);
            std::fs::create_dir_all(d.join("src")).unwrap();
            std::fs::write(d.join("Cargo.toml"), format!("[package]\nname = \"{name}\"\n")).unwrap();
            std::fs::write(d.join("src/lib.rs"), src).unwrap();
            let prefix = d.join("out/r").to_string_lossy().into_owned();
            let idx = load_dep_reports(None);
            let (rc, body) = scan_one(&d.to_string_lossy(), ScanOpts {
                prefix, want_json: true, include_tests: false, policy: None, quiet: true, deps_idx: &idx,
            });
            assert_eq!(rc, 0);
            let v: serde_json::Value = serde_json::from_str(&body.unwrap()).unwrap();
            let _ = std::fs::remove_dir_all(&d);
            v
        };
        let eff = |v: &serde_json::Value, needle: &str| -> Vec<String> {
            v["functions"].as_array().into_iter().flatten()
                .filter(|f| f["fn"].as_str().is_some_and(|q| q.contains(needle)))
                .flat_map(|f| f["inferred"].as_array().into_iter().flatten()
                    .filter_map(|e| e.as_str().map(String::from)).collect::<Vec<_>>()).collect()
        };
        let src = r#"
use std::ops::Deref;
struct Inner;
impl Inner {
    fn doit(&self) { std::process::Command::new("ls").status().unwrap(); }
    fn clone(&self) -> Inner { std::fs::read("/x").unwrap(); Inner }
}
struct W { inner: Inner }
impl Deref for W { type Target = Inner; fn deref(&self) -> &Inner { &self.inner } }
impl W { pub fn act(&self) { self.doit(); } pub fn dup(&self) { let _ = self.clone(); } }
"#;
        let v = run("deref", src);
        // auto-deref through the custom Deref reaches the pointee's Exec (was silently pure)
        assert!(eff(&v, "W::act").contains(&"Exec".to_string()), "custom Deref lost the pointee Exec:\n{v}");
        // the global `.clone()` guard still holds — `w.clone()` is not attributed the pointee's effectful clone
        assert!(eff(&v, "W::dup").is_empty(), "custom-Deref clone FABRICATED the pointee's clone effect:\n{v}");
    }

    #[test]
    fn implicit_iterator_force_charges_local_iter_next_but_not_generic() {
        // A custom `impl Iterator for LocalType` whose `next()` is effectful must charge EVERY
        // implicit forcing site — a `for` loop and consuming combinators (`.collect()`/`.count()`/
        // `.fold()`/…), not just an explicit `.next()`. Controls: a PURE custom iterator stays pure,
        // and a GENERIC/opaque iterator param must NOT inherit any concrete impl's effect (the
        // review-killed RowIter fabrication must stay closed).
        let run = |name: &str, src: &str| -> serde_json::Value {
            let d = std::env::temp_dir().join(format!("candor-iternext-{name}-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&d);
            std::fs::create_dir_all(d.join("src")).unwrap();
            std::fs::write(d.join("Cargo.toml"), format!("[package]\nname = \"{name}\"\n")).unwrap();
            std::fs::write(d.join("src/lib.rs"), src).unwrap();
            let prefix = d.join("out/r").to_string_lossy().into_owned();
            let idx = load_dep_reports(None);
            let (rc, body) = scan_one(&d.to_string_lossy(), ScanOpts {
                prefix, want_json: true, include_tests: false, policy: None, quiet: true, deps_idx: &idx,
            });
            assert_eq!(rc, 0);
            let v = serde_json::from_str(&body.unwrap()).unwrap();
            let _ = std::fs::remove_dir_all(&d);
            v
        };
        let eff = |v: &serde_json::Value, name: &str| -> Vec<String> {
            v["functions"].as_array().into_iter().flatten()
                .filter(|f| f["fn"].as_str() == Some(name))
                .flat_map(|f| f["inferred"].as_array().into_iter().flatten()
                    .filter_map(|e| e.as_str().map(String::from)).collect::<Vec<_>>()).collect()
        };
        let src = r#"
            struct LogTail { n: usize }
            impl Iterator for LogTail {
                type Item = String;
                fn next(&mut self) -> Option<String> {
                    std::fs::write("/t", b"x").unwrap();
                    if self.n == 0 { None } else { self.n -= 1; Some(String::new()) }
                }
            }
            fn tail(_p: &str) -> LogTail { LogTail { n: 1 } }
            pub fn count_lines(p: &str) -> usize { tail(p).count() }
            pub fn all_lines(p: &str) -> Vec<String> { tail(p).collect() }
            pub fn process(p: &str) { for _l in tail(p) {} }
            pub fn folded(p: &str) -> usize { tail(p).fold(0, |a, _| a + 1) }
            pub fn explicit(p: &str) { let mut t = tail(p); let _ = t.next(); }
            fn build() -> LogTail { LogTail { n: 1 } }
            pub fn built_consumer() -> usize { build().count() }

            struct PureIter { n: usize }
            impl Iterator for PureIter {
                type Item = u8;
                fn next(&mut self) -> Option<u8> { if self.n == 0 { None } else { self.n -= 1; Some(1) } }
            }
            fn pure_src() -> PureIter { PureIter { n: 1 } }
            pub fn pure_collect() -> Vec<u8> { pure_src().collect() }
            pub fn pure_for() { for _ in pure_src() {} }

            struct RowIter { n: usize }
            impl Iterator for RowIter {
                type Item = u8;
                fn next(&mut self) -> Option<u8> {
                    std::fs::write("/db", b"q").unwrap();
                    if self.n == 0 { None } else { self.n -= 1; Some(0) }
                }
            }
            pub fn generic_param(it: impl Iterator<Item = u8>) { for _ in it {} }
            pub fn generic_bound<I: Iterator>(it: I) { let _ = it.count(); }
            pub fn dyn_param(it: &mut dyn Iterator<Item = u8>) { let _ = it.count(); }

            fn opaque() -> impl Iterator<Item = u8> { OpaqueSrc { n: 1 } }
            struct OpaqueSrc { n: usize }
            impl Iterator for OpaqueSrc {
                type Item = u8;
                fn next(&mut self) -> Option<u8> {
                    std::fs::write("/o", b"z").unwrap();
                    if self.n == 0 { None } else { self.n -= 1; Some(0) }
                }
            }
            pub fn opaque_consumer() -> usize { opaque().count() }
        "#;
        let v = run("iternext", src);
        // Effectful custom iterator: implicit force at every consumer carries Fs.
        for f in ["count_lines", "all_lines", "process", "folded", "explicit", "built_consumer"] {
            assert!(eff(&v, f).contains(&"Fs".to_string()),
                    "implicit iterator force under-reported: {f} should be Fs but is {:?}\n{v}", eff(&v, f));
        }
        // Control 1: a PURE custom iterator stays pure (no fabrication from forcing).
        for f in ["pure_collect", "pure_for"] {
            assert!(eff(&v, f).is_empty(), "pure custom iterator fabricated an effect at {f}: {:?}", eff(&v, f));
        }
        // Control 2 (RowIter guard): a generic/opaque iterator param must NOT inherit a concrete
        // impl's effect, even though `impl Iterator for RowIter` does Fs.
        for f in ["generic_param", "generic_bound", "dyn_param"] {
            assert!(eff(&v, f).is_empty(),
                    "RowIter guard breached: generic iterator consumer {f} was charged {:?}", eff(&v, f));
        }
        // `-> impl Iterator` opaque return: can't resolve the concrete type → acceptable miss (pure).
        assert!(eff(&v, "opaque_consumer").is_empty(),
                "opaque-return consumer should stay pure (no concrete type): {:?}", eff(&v, "opaque_consumer"));
    }

    #[test]
    fn implicit_coercion_edges_charge_local_effectful_impls_but_never_std() {
        // The implicit-conversion / coercion edges (cardinal sin = a fn read PURE when an effect is
        // reachable through an IMPLICIT trait-method invocation): `format!`/`.to_string()`→Display::fmt,
        // `{:?}`→Debug::fmt, `?`→From::from, operators→Add/PartialEq/Index/Neg, `*w`→Deref::deref,
        // `.into()`→From::from, and struct-literal/unit Drop-glue. Each must light up the triggering fn
        // when a LOCAL EFFECTFUL impl exists; controls (a PURE impl, a STD/primitive operand) stay pure.
        let run = |name: &str, src: &str| -> serde_json::Value {
            let d = std::env::temp_dir().join(format!("candor-coerce-{name}-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&d);
            std::fs::create_dir_all(d.join("src")).unwrap();
            std::fs::write(d.join("Cargo.toml"), format!("[package]\nname = \"{name}\"\n")).unwrap();
            std::fs::write(d.join("src/lib.rs"), src).unwrap();
            let prefix = d.join("out/r").to_string_lossy().into_owned();
            let idx = load_dep_reports(None);
            let (rc, body) = scan_one(&d.to_string_lossy(), ScanOpts {
                prefix, want_json: true, include_tests: false, policy: None, quiet: true, deps_idx: &idx,
            });
            assert_eq!(rc, 0);
            let v = serde_json::from_str(&body.unwrap()).unwrap();
            let _ = std::fs::remove_dir_all(&d);
            v
        };
        let eff = |v: &serde_json::Value, name: &str| -> Vec<String> {
            v["functions"].as_array().into_iter().flatten()
                .filter(|f| f["fn"].as_str() == Some(name))
                .flat_map(|f| f["inferred"].as_array().into_iter().flatten()
                    .filter_map(|e| e.as_str().map(String::from)).collect::<Vec<_>>()).collect()
        };
        let src = r#"
            use std::fmt;
            // #2 effectful Display via format! / println! / to_string
            struct EffDisp;
            impl fmt::Display for EffDisp {
                fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
                    std::fs::write("/t", b"x").unwrap(); write!(f, "e")
                }
            }
            pub fn disp_format() -> String { let d = EffDisp; format!("{}", d) }
            pub fn disp_println() { let d = EffDisp; println!("hi {}", d); }
            pub fn disp_tostring() -> String { let d = EffDisp; d.to_string() }
            // #2 effectful Debug via {:?}
            struct EffDbg;
            impl fmt::Debug for EffDbg {
                fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
                    std::fs::write("/t", b"x").unwrap(); write!(f, "d")
                }
            }
            pub fn dbg_format() -> String { let d = EffDbg; format!("{:?}", d) }
            // control: PURE Display, and STD types (String/i32) → pure
            struct PureDisp;
            impl fmt::Display for PureDisp {
                fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result { write!(f, "p") }
            }
            pub fn disp_pure() -> String { let d = PureDisp; format!("{}", d) }
            pub fn fmt_std() -> String { let s = String::new(); let n = 3i32; format!("{} {}", s, n) }

            // #1 effectful From via ?
            struct MyErr;
            impl From<std::io::Error> for MyErr {
                fn from(_e: std::io::Error) -> MyErr { std::fs::write("/t", b"x").unwrap(); MyErr }
            }
            fn may_fail() -> Result<(), std::io::Error> { Ok(()) }
            pub fn q_eff() -> Result<(), MyErr> { may_fail()?; Ok(()) }
            // control: PURE From via ?
            struct PureErr;
            impl From<std::io::Error> for PureErr { fn from(_e: std::io::Error) -> PureErr { PureErr } }
            fn may_fail2() -> Result<(), std::io::Error> { Ok(()) }
            pub fn q_pure() -> Result<(), PureErr> { may_fail2()?; Ok(()) }

            // #4 effectful operators
            struct Acc;
            impl std::ops::Add for Acc {
                type Output = Acc;
                fn add(self, _o: Acc) -> Acc { std::fs::write("/t", b"x").unwrap(); Acc }
            }
            pub fn op_add() -> Acc { let a = Acc; let b = Acc; a + b }
            struct Cmp;
            impl PartialEq for Cmp {
                fn eq(&self, _o: &Cmp) -> bool { std::fs::write("/t", b"x").unwrap(); true }
            }
            pub fn op_eq() -> bool { let a = Cmp; let b = Cmp; a == b }
            struct Ix;
            impl std::ops::Index<usize> for Ix {
                type Output = u8;
                fn index(&self, _i: usize) -> &u8 { std::fs::write("/t", b"x").unwrap(); &0 }
            }
            pub fn op_index() -> u8 { let a = Ix; a[0] }
            struct Ng;
            impl std::ops::Neg for Ng {
                type Output = Ng;
                fn neg(self) -> Ng { std::fs::write("/t", b"x").unwrap(); Ng }
            }
            pub fn op_neg() -> Ng { let a = Ng; -a }
            // control: PURE operator, and STD arithmetic → pure
            struct PureAdd;
            impl std::ops::Add for PureAdd { type Output = PureAdd; fn add(self, _o: PureAdd) -> PureAdd { PureAdd } }
            pub fn op_pure() -> PureAdd { let a = PureAdd; let b = PureAdd; a + b }
            pub fn op_std() -> i32 { let a = 1i32; let b = 2i32; a + b }

            // #3 effectful Deref via *w
            struct Wrap;
            impl std::ops::Deref for Wrap {
                type Target = u8;
                fn deref(&self) -> &u8 { std::fs::write("/t", b"x").unwrap(); &0 }
            }
            pub fn deref_eff() -> u8 { let w = Wrap; *w }

            // #5 effectful From via .into()
            struct Tgt;
            impl From<u8> for Tgt { fn from(_v: u8) -> Tgt { std::fs::write("/t", b"x").unwrap(); Tgt } }
            pub fn into_eff() { let x: Tgt = 5u8.into(); let _ = x; }

            // #6 struct-literal & unit Drop-glue
            struct Guard { n: u8 }
            impl Drop for Guard { fn drop(&mut self) { std::fs::write("/t", b"x").unwrap(); } }
            pub fn drop_struct() { let _g = Guard { n: 1 }; }
            struct UnitGuard;
            impl Drop for UnitGuard { fn drop(&mut self) { std::fs::write("/t", b"x").unwrap(); } }
            pub fn drop_unit() { let _g = UnitGuard; }
            // control: PURE-Drop struct literal → pure
            struct PureGuard { n: u8 }
            impl Drop for PureGuard { fn drop(&mut self) {} }
            pub fn drop_pure() { let _g = PureGuard { n: 1 }; }
        "#;
        let v = run("coerce", src);
        // Every effectful coercion must carry Fs at the triggering fn.
        for f in [
            "disp_format", "disp_println", "disp_tostring", "dbg_format", "q_eff",
            "op_add", "op_eq", "op_index", "op_neg", "deref_eff", "into_eff",
            "drop_struct", "drop_unit",
        ] {
            assert!(eff(&v, f).contains(&"Fs".to_string()),
                    "coercion under-reported: {f} should be Fs but is {:?}", eff(&v, f));
        }
        // Controls: a PURE impl or a STD/primitive operand must stay pure — no fabrication, no flood.
        for f in ["disp_pure", "fmt_std", "q_pure", "op_pure", "op_std", "drop_pure"] {
            assert!(eff(&v, f).is_empty(),
                    "coercion control fabricated an effect at {f}: {:?}", eff(&v, f));
        }
    }

    #[test]
    fn dispatch_typed_receivers_resolve_via_local_impls_or_read_unknown() {
        // The trait-object hole, closed: `t.save()` on a `&dyn Store` either edges to the LOCAL
        // implementors (syntactic CHA, the JVM engine's bounded move) or reads honest Unknown —
        // never silent-pure. External traits stay out (no Unknown flood on `impl Iterator`).
        let uses = HashMap::new();
        let fields = FieldIndex::new();
        let returns = ReturnIndex::new();
        let mut ti = TraitImplIndex::new();
        ti.insert("Store".into(), vec!["PgStore".into(), "MemStore".into()]);
        let mut td: HashMap<String, LocalTrait> = HashMap::new();
        td.insert("Store".into(), LocalTrait { count: 1, methods: ["save".to_string()].into_iter().collect() });
        td.insert("Sink".into(), LocalTrait { count: 1, methods: ["flush".to_string()].into_iter().collect() }); // no impl in sight
        let mut tf = TraitFieldIndex::new();
        // struct App { store: Box<dyn Store> }
        tf.entry("App".into()).or_default().insert("store".into(), vec!["Store".into()]);
        let (fe, ev) = (FieldElemIndex::new(), EnumVariantIndex::new());
        let run = |src: &str, sig: &str| {
            let sig: syn::Signature = syn::parse_str(sig).unwrap();
            let blk: syn::Block = syn::parse_str(src).unwrap();
            let trait_vars = seed_trait_vars(&sig);
            let mut vars = seed_vars(&sig, Some("App"), &uses);
            for k in trait_vars.keys() {
                vars.remove(k); // same precedence as fninfo: dispatch-typing wins
            }
            vars.insert("self".to_string(), "App".to_string());
            let mut c = CallCollector {
                uses: &uses,
                vars,
                trait_vars,
                fields: &fields,
                trait_fields: &tf,
                trait_impls: &ti,
                local_traits: &td,
                returns: &returns,
                field_elem: &fe,
                enum_variants: &ev,
                elem_of: HashMap::new(), tuple_of: HashMap::new(),
                calls: Vec::new(),
                closure_vars: std::collections::HashSet::new(),
                fn_typed_vars: std::collections::HashSet::new(),
            fn_alias: std::collections::HashMap::new(),
            lazy_statics: empty_lazy(),
            forced_lazies: std::collections::HashSet::new(),
                unresolved: false,
                err_ret_leaf: None,
            };
            for stmt in &blk.stmts {
                c.visit_stmt(stmt);
            }
            (c.calls.iter().map(|c| c.path.clone()).collect::<Vec<_>>(), c.unresolved)
        };
        // &dyn param → typed edges to BOTH local impls, not unresolved
        let (paths, unres) = run("{ t.save(x); }", "fn f(t: &dyn Store)");
        assert!(paths.contains(&"PgStore::save".to_string()), "dyn param not CHA-resolved: {paths:?}");
        assert!(paths.contains(&"MemStore::save".to_string()), "dyn param missed an impl: {paths:?}");
        assert!(!unres, "narrow local dispatch must not read Unknown");
        // impl-Trait param and generic bound take the same route
        let (paths, _) = run("{ s.save(x); }", "fn f(s: impl Store)");
        assert!(paths.contains(&"PgStore::save".to_string()), "impl-Trait param not resolved: {paths:?}");
        let (paths, _) = run("{ x.save(y); }", "fn f<X: Store>(x: X)");
        assert!(paths.contains(&"PgStore::save".to_string()), "generic bound not resolved: {paths:?}");
        // the DI field: self.store is Box<dyn Store> via the trait-field index
        let (paths, _) = run("{ self.store.save(x); }", "fn f(&self)");
        assert!(paths.contains(&"PgStore::save".to_string()), "trait-typed field not resolved: {paths:?}");
        // a LOCAL trait with no visible impl: something implements it somewhere — honest Unknown
        let (_, unres) = run("{ k.flush(); }", "fn f(k: &dyn Sink)");
        assert!(unres, "local trait without impls must read Unknown, not silent-pure");
        // an EXTERNAL trait (not locally declared, no local impls): documented miss, NO flood
        let (paths, unres) = run("{ it.next(); }", "fn f(it: impl Iterator)");
        assert!(!unres && !paths.iter().any(|p| p.contains("::next")), "external trait must stay out: {paths:?}");
        // FABRICATION GUARD (review, execution-verified): an EXTERNAL trait with a LOCAL impl
        // must STILL stay out — `impl Iterator for RowIter` + `f(it: impl Iterator)` must not
        // charge f with RowIter's effects.
        {
            let mut ti2 = TraitImplIndex::new();
            ti2.insert("Iterator".into(), vec!["RowIter".into()]);
            let sig: syn::Signature = syn::parse_str("fn f(it: impl Iterator)").unwrap();
            let blk: syn::Block = syn::parse_str("{ it.next(); }").unwrap();
            let mut c = CallCollector {
                uses: &uses, vars: HashMap::new(), trait_vars: seed_trait_vars(&sig),
                fields: &fields, trait_fields: &tf, trait_impls: &ti2, local_traits: &td,
                returns: &returns, field_elem: &fe, enum_variants: &ev, elem_of: HashMap::new(), tuple_of: HashMap::new(),
                calls: Vec::new(),
                closure_vars: std::collections::HashSet::new(), fn_typed_vars: std::collections::HashSet::new(), fn_alias: std::collections::HashMap::new(), lazy_statics: empty_lazy(), forced_lazies: std::collections::HashSet::new(), unresolved: false, err_ret_leaf: None,
            };
            for stmt in &blk.stmts { c.visit_stmt(stmt); }
            assert!(!c.calls.iter().any(|x| x.path == "RowIter::next"),
                    "external-trait local impl must not resolve (fabrication)");
            assert!(!c.unresolved, "external trait must not flood Unknown either");
        }
        // a method the LOCAL trait does NOT declare (supertrait/blanket call) — out, no flood
        let (paths, unres) = run("{ t.clone(); }", "fn f(t: &dyn Store)");
        assert!(!unres && !paths.iter().any(|p| p.ends_with("::clone")),
                "undeclared method must neither edge nor flood: {paths:?}");
        // the cross-engine CHA bound: 12 impls resolve, 13 read honest Unknown
        {
            let wide = |n: usize, src: &str, sig: &str| {
                let mut ti2 = TraitImplIndex::new();
                ti2.insert("Store".into(), (0..n).map(|i| format!("S{i}")).collect());
                let sig: syn::Signature = syn::parse_str(sig).unwrap();
                let blk: syn::Block = syn::parse_str(src).unwrap();
                let mut c = CallCollector {
                    uses: &uses, vars: HashMap::new(), trait_vars: seed_trait_vars(&sig),
                    fields: &fields, trait_fields: &tf, trait_impls: &ti2, local_traits: &td,
                    returns: &returns, field_elem: &fe, enum_variants: &ev, elem_of: HashMap::new(), tuple_of: HashMap::new(),
                    calls: Vec::new(),
                    closure_vars: std::collections::HashSet::new(), fn_typed_vars: std::collections::HashSet::new(), fn_alias: std::collections::HashMap::new(), lazy_statics: empty_lazy(), forced_lazies: std::collections::HashSet::new(), unresolved: false, err_ret_leaf: None,
                };
                for stmt in &blk.stmts { c.visit_stmt(stmt); }
                (c.calls.iter().filter(|x| x.typed).count(), c.unresolved)
            };
            let (edges, unres) = wide(12, "{ t.save(x); }", "fn f(t: &dyn Store)");
            assert!(edges == 12 && !unres, "12 impls must resolve (the shared bound)");
            let (edges, unres) = wide(13, "{ t.save(x); }", "fn f(t: &dyn Store)");
            assert!(edges == 0 && unres, "13 impls must read Unknown, not resolve");
        }
    }

    #[test]
    fn return_type_inference_flows_through_local_factories() {
        // `let p = create_pool()?; p.fetch_one(q)` — create_pool's recorded return type lets p resolve.
        let uses = HashMap::new();
        let fields = FieldIndex::new();
        let mut returns = ReturnIndex::new();
        returns.insert("create_pool".to_string(), "sqlx::PgPool".to_string());
        let (ti, td, tf) = (TraitImplIndex::new(), HashMap::new(), TraitFieldIndex::new());
        let (fe, ev) = (FieldElemIndex::new(), EnumVariantIndex::new());
        let block: syn::Block =
            syn::parse_str("{ let p = create_pool()?; p.fetch_one(q); }").unwrap();
        let mut c = CallCollector {
            uses: &uses,
            vars: HashMap::new(),
            trait_vars: HashMap::new(),
            fields: &fields,
            trait_fields: &tf,
            trait_impls: &ti,
            local_traits: &td,
            returns: &returns,
            field_elem: &fe,
            enum_variants: &ev,
            elem_of: HashMap::new(), tuple_of: HashMap::new(),
            calls: Vec::new(),
            closure_vars: std::collections::HashSet::new(),
            fn_typed_vars: std::collections::HashSet::new(),
            fn_alias: std::collections::HashMap::new(),
            lazy_statics: empty_lazy(),
            forced_lazies: std::collections::HashSet::new(),
            unresolved: false,
            err_ret_leaf: None,
        };
        for stmt in &block.stmts {
            c.visit_stmt(stmt);
        }
        let typed: Vec<&str> = c.calls.iter().map(|c| c.path.as_str()).collect();
        assert!(typed.contains(&"sqlx::PgPool::fetch_one"), "return-typed recv not resolved: {typed:?}");

        // A computed-callable invocation (a closure / fn-pointer the scan can't see through) marks the
        // function `unresolved` (→ honest `Unknown`), while a LOCAL closure whose body IS visible does
        // not — its effects were already walked lexically.
        let mk = |src: &str| {
            let blk: syn::Block = syn::parse_str(src).unwrap();
            let mut cc = CallCollector {
                uses: &uses,
                vars: HashMap::new(),
                trait_vars: HashMap::new(),
                fields: &fields,
                trait_fields: &tf,
                trait_impls: &ti,
                local_traits: &td,
                returns: &returns,
                field_elem: &fe,
                enum_variants: &ev,
                elem_of: HashMap::new(), tuple_of: HashMap::new(),
                calls: Vec::new(),
                closure_vars: std::collections::HashSet::new(),
                fn_typed_vars: std::collections::HashSet::new(),
            fn_alias: std::collections::HashMap::new(),
            lazy_statics: empty_lazy(),
            forced_lazies: std::collections::HashSet::new(),
                unresolved: false,
                err_ret_leaf: None,
            };
            for stmt in &blk.stmts {
                cc.visit_stmt(stmt);
            }
            cc.unresolved
        };
        assert!(mk("{ (handlers[k])(); }"), "indexed callable must be unresolved");
        assert!(mk("{ (self.cb)(); }"), "fn-pointer field call must be unresolved");
        assert!(!mk("{ let g = |x: i32| x + 1; let _ = g(3); }"), "local closure body is visible — not unresolved");
        assert!(!mk("{ helper(); other::thing(); }"), "ordinary path calls are not unresolved");

        // unwrap_result_option peels Result/Option to the success type
        let r: syn::Type = syn::parse_str("std::io::Result<reqwest::Client>").unwrap();
        assert_eq!(type_path(unwrap_result_option(&r), &uses).as_deref(), Some("reqwest::Client"));
        let o: syn::Type = syn::parse_str("Option<PgPool>").unwrap();
        assert_eq!(type_path(unwrap_result_option(&o), &uses).as_deref(), Some("PgPool"));
    }

    #[test]
    fn test_file_stems_are_recognised() {
        assert!(is_test_file_stem("tests")); // src/foo/tests.rs
        assert!(is_test_file_stem("test"));
        assert!(is_test_file_stem("decoder_tests")); // base64's read/decoder_tests.rs
        assert!(is_test_file_stem("engine_test"));
        // legitimate non-test modules must NOT be excluded
        assert!(!is_test_file_stem("latest")); // not `_test`-suffixed (no underscore boundary)
        assert!(!is_test_file_stem("request"));
        assert!(!is_test_file_stem("contest"));
        assert!(!is_test_file_stem("lib"));
    }

    #[test]
    fn stable_policy_gate_evaluates_all_three_rule_kinds() {
        let all = vec!["api::handle".to_string(), "db::run".to_string(), "ui::draw".to_string()];
        let mut inferred: HashMap<String, BTreeSet<&'static str>> = HashMap::new();
        inferred.insert("api::handle".into(), ["Net"].into_iter().collect());
        inferred.insert("db::run".into(), ["Db"].into_iter().collect());
        let mut calls: HashMap<String, BTreeSet<String>> = HashMap::new();
        calls.insert("ui::draw".into(), ["db::run".to_string()].into_iter().collect());
        let mut hosts: HashMap<String, BTreeSet<String>> = HashMap::new();
        hosts.insert("api::handle".into(), ["evil.example.com".to_string()].into_iter().collect());
        let empty = HashMap::new();
        let empty_inc: HashMap<String, BTreeSet<&'static str>> = HashMap::new();
        // deny fires on the transitive set; allow flags the out-of-list host; forbid sees ui -> db.
        let mut tables: HashMap<String, BTreeSet<String>> = HashMap::new();
        tables.insert("db::run".into(), ["audit.log".to_string()].into_iter().collect());
        // deny fires on the transitive set; allow flags the out-of-list host; forbid sees ui -> db.
        let v = policy_violations(
            "deny Net api\nallow Net in api good.example.com\nforbid ui -> db\n",
            &all, &inferred, &calls, &hosts, &empty, &empty, &tables, &empty_inc,
        );
        assert_eq!(v.len(), 3, "{}", v.iter().map(|x| x.detail.clone()).collect::<Vec<_>>().join(" | "));
        // 006 names the denied effect in `effects` (the denied SET, not just the message text).
        assert!(v.iter().any(|g| g.rule == "AS-EFF-006" && g.func == "api::handle" && g.effects == ["Net"]));
        assert!(v.iter().any(|g| g.rule == "AS-EFF-008" && g.detail.contains("evil.example.com") && g.effects == ["Net"]));
        // 009 is a layer-flow — no single effect, so `effects` is empty.
        assert!(v.iter().any(|g| g.rule == "AS-EFF-009" && g.func == "ui::draw" && g.effects.is_empty()));
        // clean policy -> no violations; `pure` flags ANY effect incl. the Db fn.
        assert!(policy_violations("deny Exec\n", &all, &inferred, &calls, &hosts, &empty, &empty, &tables, &empty_inc).is_empty());
        assert_eq!(policy_violations("pure db\n", &all, &inferred, &calls, &hosts, &empty, &empty, &tables, &empty_inc).len(), 1);
        // the Db table allowlist: db::run reaches audit.log — outside `ledger.*` -> violation;
        // covered by `audit.*` -> clean. ui::draw INHERITS Db but the literal propagation is the
        // caller's tablesacc, supplied here only for db::run, so only db::run flags.
        let bad = policy_violations("allow Db in db ledger.*\n", &all, &inferred, &calls, &hosts, &empty, &empty, &tables, &empty_inc);
        assert_eq!(bad.len(), 1, "{}", bad.iter().map(|x| x.detail.clone()).collect::<Vec<_>>().join(" | "));
        assert!(bad[0].detail.contains("audit.log"));
        assert!(policy_violations("allow Db in db audit.*\n", &all, &inferred, &calls, &hosts, &empty, &empty, &tables, &empty_inc).is_empty());
    }

    #[test]
    fn masking_incomplete_net_surface_not_certified() {
        // The masking evasion: a fn with a captured BENIGN host AND a structurally-INVISIBLE Net reach
        // (an incomplete surface) must NOT be certified by the benign host. A clean fn (host captured,
        // surface complete) certifies. Mirrors candor-java 0.5.29.
        let all = vec!["a::mask".to_string(), "a::clean".to_string()];
        let mut inferred: HashMap<String, BTreeSet<&'static str>> = HashMap::new();
        inferred.insert("a::mask".into(), ["Net"].into_iter().collect());
        inferred.insert("a::clean".into(), ["Net"].into_iter().collect());
        let calls: HashMap<String, BTreeSet<String>> = HashMap::new();
        let mut hosts: HashMap<String, BTreeSet<String>> = HashMap::new();
        hosts.insert("a::mask".into(), ["api.stripe.com".to_string()].into_iter().collect());
        hosts.insert("a::clean".into(), ["api.stripe.com".to_string()].into_iter().collect());
        let empty: HashMap<String, BTreeSet<String>> = HashMap::new();
        let tables: HashMap<String, BTreeSet<String>> = HashMap::new();
        let mut inc: HashMap<String, BTreeSet<&'static str>> = HashMap::new();
        inc.insert("a::mask".into(), ["Net"].into_iter().collect()); // mask also has an invisible reach
        let v = policy_violations(
            "allow Net api.stripe.com\n",
            &all, &inferred, &calls, &hosts, &empty, &empty, &tables, &inc,
        );
        assert!(v.iter().any(|g| g.func == "a::mask" && g.detail.contains("cannot be certified")), "{:?}", v.iter().map(|x| x.detail.clone()).collect::<Vec<_>>());
        assert!(!v.iter().any(|g| g.func == "a::clean"), "clean must certify: {:?}", v.iter().map(|x| x.detail.clone()).collect::<Vec<_>>());
    }

    #[test]
    fn masking_fs_path_and_db_table_gate_fails_closed() {
        // End-to-end (scan_one + a CANDOR_POLICY file): a MASKED Fs path / Db table reached ALONGSIDE a
        // benign ALLOWED literal must FAIL the allowlist gate (exit 1) — the benign sibling must not mask
        // the runtime-built endpoint. A single compliant literal still PASSES (no false positive). A
        // fully-masked program (no benign sibling) still fails. The gate evasion this closes:
        // `inferred=[Fs] paths=[/var/app/x]` with no `incomplete` certified `allow Fs /var/app` while a
        // sibling `fs::write(format!("/etc/{}","passwd"), …)` hit /etc/passwd at runtime.
        let run = |name: &str, src: &str, policy: &str| -> i32 {
            let d = std::env::temp_dir().join(format!("candor-mask-{name}-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&d);
            std::fs::create_dir_all(d.join("src")).unwrap();
            std::fs::write(d.join("Cargo.toml"), format!("[package]\nname = \"{name}\"\n")).unwrap();
            std::fs::write(d.join("src/lib.rs"), src).unwrap();
            let pp = d.join("candor.policy");
            std::fs::write(&pp, policy).unwrap();
            let prefix = d.join("out/r").to_string_lossy().into_owned();
            let idx = load_dep_reports(None);
            let (rc, _) = scan_one(&d.to_string_lossy(), ScanOpts {
                prefix, want_json: true, include_tests: false,
                policy: Some(pp.to_string_lossy().into_owned()), quiet: true, deps_idx: &idx,
            });
            let _ = std::fs::remove_dir_all(&d);
            rc
        };

        // Fs: a benign allowed write + a MASKED (runtime-path) write → gate FAILS (Fs incomplete).
        let fs_mix = r#"
            use std::fs;
            pub fn go() {
                let _ = fs::write("/var/app/x", b"x");
                let p = format!("/etc/{}", "passwd");
                let _ = fs::write(p, b"x");
            }
        "#;
        assert_eq!(run("fsmix", fs_mix, "allow Fs /var/app\n"), 1, "masked Fs path must fail the gate");

        // Fs: a single compliant literal (no masking) → PASSES.
        let fs_ok = r#"
            use std::fs;
            pub fn go() { let _ = fs::write("/var/app/x", b"x"); }
        "#;
        assert_eq!(run("fsok", fs_ok, "allow Fs /var/app\n"), 0, "compliant Fs path must pass (no false positive)");

        // Fs: fully masked (no benign sibling) → still fails (unchanged behaviour).
        let fs_masked = r#"
            use std::fs;
            pub fn go() { let p = format!("/etc/{}", "passwd"); let _ = fs::write(p, b"x"); }
        "#;
        assert_eq!(run("fsmask", fs_masked, "allow Fs /var/app\n"), 1, "fully-masked Fs path must fail");

        // Db: a benign allowed query + a MASKED (runtime-query) execute → gate FAILS (Db incomplete).
        let db_mix = r#"
            pub fn go(con: &rusqlite::Connection) {
                let _ = con.execute("SELECT id FROM customers", []);
                let q = format!("DELETE FROM {}", "secrets");
                let _ = con.execute(&q, []);
            }
        "#;
        assert_eq!(run("dbmix", db_mix, "allow Db customers\n"), 1, "masked Db table must fail the gate");

        // Db: a single compliant query (no masking) → PASSES.
        let db_ok = r#"
            pub fn go(con: &rusqlite::Connection) {
                let _ = con.execute("SELECT id FROM customers", []);
            }
        "#;
        assert_eq!(run("dbok", db_ok, "allow Db customers\n"), 0, "compliant Db table must pass (no false positive)");
    }

    #[test]
    fn gate_over_unparseable_source_fails_closed() {
        // SOUNDNESS: a policy gate over a crate where a source file failed to PARSE must NOT report
        // green — the unparsed file's effects are absent from the report, so a `policy ✓` over it is a
        // false-pure. Exit 2 (mirroring the unreadable-policy posture), never 0. A clean-parsing crate
        // under the same policy still passes 0 (the failure signal is specific, not a blanket fail).
        let run = |name: &str, src: &str, with_policy: bool| -> i32 {
            let d = std::env::temp_dir().join(format!("candor-parsefail-{name}-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&d);
            std::fs::create_dir_all(d.join("src")).unwrap();
            std::fs::write(d.join("Cargo.toml"), format!("[package]\nname = \"{name}\"\n")).unwrap();
            std::fs::write(d.join("src/lib.rs"), src).unwrap();
            let pp = d.join("candor.policy");
            std::fs::write(&pp, "deny Exec\n").unwrap();
            let prefix = d.join("out/r").to_string_lossy().into_owned();
            let idx = load_dep_reports(None);
            let (rc, _) = scan_one(&d.to_string_lossy(), ScanOpts {
                prefix, want_json: true, include_tests: false,
                policy: if with_policy { Some(pp.to_string_lossy().into_owned()) } else { None },
                quiet: true, deps_idx: &idx,
            });
            let _ = std::fs::remove_dir_all(&d);
            rc
        };
        // A file that does NOT parse (a stray token) under a configured gate → exit 2, never green.
        let broken = "pub fn ok() {}\nthis is not valid rust @@@\n";
        assert_eq!(run("broken", broken, true), 2,
                   "a configured gate over an unparseable source file must FAIL exit 2, never green");
        // No policy configured → no gate verdict to corrupt; the parse-failure is disclosed (stderr) only.
        assert_eq!(run("brokennopol", broken, false), 0,
                   "without a policy there is no gate; a parse failure is disclosed, not an exit code");
        // A clean-parsing crate under the same gate still passes (the failure signal is specific).
        assert_eq!(run("clean", "pub fn ok() {}\n", true), 0,
                   "a clean-parsing crate must still pass the gate (no blanket fail)");
    }

    #[test]
    fn call_returning_a_callable_in_an_unannotated_local_reads_unknown() {
        // §4 HONESTY: `let g = make_cb(); g()` where `make_cb` returns a CALLABLE must read the opaque
        // callback (Unknown), not silent-pure / a phantom free-fn `g`. Covers all three callable return
        // shapes (`fn()`, `impl Fn`, `Box<dyn Fn>`). A NON-callable factory's binding stays pure.
        let run = |name: &str, src: &str| -> serde_json::Value {
            let d = std::env::temp_dir().join(format!("candor-retcb-{name}-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&d);
            std::fs::create_dir_all(d.join("src")).unwrap();
            std::fs::write(d.join("Cargo.toml"), format!("[package]\nname = \"{name}\"\n")).unwrap();
            std::fs::write(d.join("src/lib.rs"), src).unwrap();
            let prefix = d.join("out/r").to_string_lossy().into_owned();
            let idx = load_dep_reports(None);
            let (rc, body) = scan_one(&d.to_string_lossy(), ScanOpts {
                prefix, want_json: true, include_tests: false, policy: None, quiet: true, deps_idx: &idx,
            });
            assert_eq!(rc, 0);
            let v: serde_json::Value = serde_json::from_str(&body.unwrap()).unwrap();
            let _ = std::fs::remove_dir_all(&d);
            v
        };
        let eff = |v: &serde_json::Value, needle: &str| -> Vec<String> {
            v["functions"].as_array().into_iter().flatten()
                .filter(|f| f["fn"].as_str().is_some_and(|q| q.contains(needle)))
                .flat_map(|f| f["inferred"].as_array().into_iter().flatten()
                    .filter_map(|e| e.as_str().map(String::from)).collect::<Vec<_>>()).collect()
        };
        // `Box<dyn Fn>` return: the binding is fn-typed → `g()` is Unknown.
        let boxed = r#"
            pub fn make_cb() -> Box<dyn Fn()> { Box::new(|| {}) }
            pub fn uses() { let g = make_cb(); g(); }
        "#;
        assert!(eff(&run("retbox", boxed), "uses").contains(&"Unknown".to_string()),
                "a call returning Box<dyn Fn> bound to a local must read Unknown at the call site");
        // bare `fn()` return.
        let bare = r#"
            fn h() {}
            pub fn make_cb() -> fn() { h }
            pub fn uses() { let g = make_cb(); g(); }
        "#;
        assert!(eff(&run("retbare", bare), "uses").contains(&"Unknown".to_string()),
                "a call returning fn() bound to a local must read Unknown at the call site");
        // `impl Fn` return.
        let impl_fn = r#"
            pub fn make_cb() -> impl Fn() { || {} }
            pub fn uses() { let g = make_cb(); g(); }
        "#;
        assert!(eff(&run("retimpl", impl_fn), "uses").contains(&"Unknown".to_string()),
                "a call returning impl Fn bound to a local must read Unknown at the call site");
        // CONTROL: a non-callable factory's binding is NOT fn-typed → no fabricated Unknown.
        let plain = r#"
            pub fn make_v() -> i32 { 42 }
            pub fn uses() { let v = make_v(); let _ = v; }
        "#;
        assert!(!eff(&run("retplain", plain), "uses").contains(&"Unknown".to_string()),
                "a non-callable factory binding must stay pure (no fabricated Unknown)");
    }

    #[test]
    fn ambiguous_same_name_local_bare_leaf_reads_unknown() {
        // §4 HONESTY: a BARE leaf naming TWO-OR-MORE local defs (a free fn + a same-named method, here
        // `process`) defeats `resolve_target`'s uniqueness filter AND is suppressed from the classifier —
        // today its callee's effects vanish (silent-pure). Disclose Unknown instead. A UNIQUE same-name
        // leaf must STILL resolve (no spurious Unknown) — the precise-scoping control.
        let run = |name: &str, src: &str| -> serde_json::Value {
            let d = std::env::temp_dir().join(format!("candor-amb-{name}-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&d);
            std::fs::create_dir_all(d.join("src")).unwrap();
            std::fs::write(d.join("Cargo.toml"), format!("[package]\nname = \"{name}\"\n")).unwrap();
            std::fs::write(d.join("src/lib.rs"), src).unwrap();
            let prefix = d.join("out/r").to_string_lossy().into_owned();
            let idx = load_dep_reports(None);
            let (rc, body) = scan_one(&d.to_string_lossy(), ScanOpts {
                prefix, want_json: true, include_tests: false, policy: None, quiet: true, deps_idx: &idx,
            });
            assert_eq!(rc, 0);
            let v: serde_json::Value = serde_json::from_str(&body.unwrap()).unwrap();
            let _ = std::fs::remove_dir_all(&d);
            v
        };
        let eff = |v: &serde_json::Value, needle: &str| -> Vec<String> {
            v["functions"].as_array().into_iter().flatten()
                .filter(|f| f["fn"].as_str().is_some_and(|q| q.contains(needle)))
                .flat_map(|f| f["inferred"].as_array().into_iter().flatten()
                    .filter_map(|e| e.as_str().map(String::from)).collect::<Vec<_>>()).collect()
        };
        // Two local defs of the leaf `process` (a free fn + a method); a bare `process()` call can't be
        // disambiguated → Unknown, not silent-pure.
        let ambiguous = r#"
            pub fn process() {}
            struct W;
            impl W { fn process(&self) {} }
            pub fn caller() { process(); }
        "#;
        assert!(eff(&run("ambig", ambiguous), "caller").contains(&"Unknown".to_string()),
                "a bare leaf with two same-name local defs must read Unknown, not silent-pure");
        // CONTROL: a UNIQUE local `solo` resolves cleanly — no spurious Unknown from this branch.
        let unique = r#"
            pub fn solo() {}
            pub fn caller() { solo(); }
        "#;
        assert!(!eff(&run("uniq", unique), "caller").contains(&"Unknown".to_string()),
                "a unique same-name leaf must resolve, never read a spurious Unknown");
    }

    #[test]
    fn only_root_build_rs_is_the_build_script() {
        use std::path::Path;
        // the Cargo build script — crate-root `build.rs` — IS skipped
        assert!(is_build_script(Path::new("build.rs")));
        // a nested `build.rs` is an ordinary source module and must NOT be skipped (the regression:
        // git2's `src/build.rs` is `RepoBuilder`, the whole clone/fetch network surface)
        assert!(!is_build_script(Path::new("src/build.rs")));
        assert!(!is_build_script(Path::new("src/foo/build.rs")));
        assert!(!is_build_script(Path::new("build/mod.rs"))); // a `build` module dir, not the script
    }

    #[test]
    fn cfg_test_modules_are_recognised() {
        let yes1: syn::ItemMod = syn::parse_str("#[cfg(test)] mod tests {}").unwrap();
        let yes2: syn::ItemMod =
            syn::parse_str("#[cfg(any(test, feature = \"x\"))] mod tests {}").unwrap();
        let no1: syn::ItemMod = syn::parse_str("#[cfg(feature = \"std\")] mod imp {}").unwrap();
        let no2: syn::ItemMod = syn::parse_str("mod real {}").unwrap();
        // deeper nesting positively requiring test → still skipped
        let yes3: syn::ItemMod =
            syn::parse_str("#[cfg(any(all(test, unix), windows))] mod t {}").unwrap();
        // `not(test)` is PRODUCTION code — must NOT be treated as a test module (the regression fix)
        let prod1: syn::ItemMod = syn::parse_str("#[cfg(not(test))] mod prod {}").unwrap();
        let prod2: syn::ItemMod = syn::parse_str("#[cfg(all(unix, not(test)))] mod prod {}").unwrap();
        assert!(is_cfg_test(&yes1.attrs));
        assert!(is_cfg_test(&yes2.attrs));
        assert!(is_cfg_test(&yes3.attrs));
        assert!(!is_cfg_test(&no1.attrs));
        assert!(!is_cfg_test(&no2.attrs));
        assert!(!is_cfg_test(&prod1.attrs), "cfg(not(test)) is production, not a test module");
        assert!(!is_cfg_test(&prod2.attrs), "cfg(all(unix, not(test))) is production");
    }

    #[test]
    fn expand_does_not_alias_a_crate_rooted_path() {
        // `crate::config::load` is explicitly crate-local; a `use other::config;` import must NOT hijack it.
        let u = uses(&[("config", "other::config")]);
        assert_eq!(expand("crate::config::load", &u), "config::load");
        assert_eq!(expand("self::config::load", &u), "config::load");
        // a NON-rooted bare `config::load` still expands via the use alias
        assert_eq!(expand("config::load", &u), "other::config::load");
    }

    #[test]
    fn ctor_type_rejects_a_module_path_receiver() {
        // `serde_json::from_str(s)` must NOT infer the MODULE `serde_json` as a type (lower-case receiver);
        // `reqwest::Client::new()` must still infer `reqwest::Client` (UpperCamel type receiver).
        let u = HashMap::new();
        let r = ReturnIndex::new();
        let modcall: syn::Expr = syn::parse_str("serde_json::from_str(s)").unwrap();
        assert_eq!(ctor_type(&modcall, &u, &r), None);
        let typecall: syn::Expr = syn::parse_str("reqwest::Client::new()").unwrap();
        assert_eq!(ctor_type(&typecall, &u, &r).as_deref(), Some("reqwest::Client"));
    }

    #[test]
    fn struct_literal_bindings_infer_their_type() {
        // `let s = S;` / `let s = S{..};` must type `s` so `s.go()` resolves (was the last named
        // receiver-inference gap: both read pure while `let s: S = S;` worked).
        let u = HashMap::new();
        let r = ReturnIndex::new();
        let t = |src: &str| ctor_type(&syn::parse_str::<syn::Expr>(src).unwrap(), &u, &r);
        assert_eq!(t("S").as_deref(), Some("S")); // unit-struct literal
        assert_eq!(t("S { a: 1 }").as_deref(), Some("S")); // struct literal
        assert_eq!(t("m::S { a: 1 }").as_deref(), Some("m::S")); // module-qualified
        assert_eq!(t("Color::Red").as_deref(), Some("Color")); // unit ENUM variant → the enum
        assert_eq!(t("Color::Red { x: 1 }").as_deref(), Some("Color")); // struct enum variant → the enum
        // negative gates: a variable copy and a SCREAMING_SNAKE const must NOT infer a type.
        assert_eq!(t("other_var"), None);
        assert_eq!(t("MAX_SIZE"), None);
        assert_eq!(t("config::MAX_SIZE"), None);
    }

    #[test]
    fn self_returning_ctor_types_the_local_and_the_edge_survives() {
        // The PROVE-IT-on-ureq miss: `fn new_with_defaults() -> Self` indexed the literal "Self", so
        // `let agent = Agent::new_with_defaults(); agent.run(..)` formed `Self::run` — no local def,
        // edge silently dropped, and the caller read pure though run() does I/O.
        let src = r#"
            pub struct Agent;
            impl Agent {
                pub fn new_with_defaults() -> Self { Agent }
                pub fn run(&self) { let _ = std::fs::read("/tmp/x"); }
            }
            pub fn top() { let agent = Agent::new_with_defaults(); agent.run() }
        "#;
        let file: syn::File = syn::parse_str(src).unwrap();
        let mut uses = HashMap::new();
        let mut fields: FieldIndex = HashMap::new();
        let mut rets: HashMap<String, Option<String>> = HashMap::new();
        let (mut ti, mut td, mut tf) = (TraitImplIndex::new(), HashMap::new(), TraitFieldIndex::new());
        let (mut fe, mut ev) = (FieldElemIndex::new(), HashMap::new());
        collect_decls(&file.items, false, &mut uses, &mut fields, &mut fe, &mut rets, &mut ev, &mut ti, &mut td, &mut tf, &mut std::collections::HashSet::new(), &mut std::collections::HashSet::new(), &mut std::collections::HashSet::new(), &mut std::collections::HashMap::new(), &mut std::collections::HashSet::new());
        assert_eq!(rets.get("new_with_defaults"), Some(&Some("Agent".to_string())),
                   "Self must resolve to the impl type, not the literal");
    }

    #[test]
    fn local_method_named_like_a_crate_does_not_inherit_the_crate_effect() {
        // A bare-leaf method call (`self.fastrand()`) is recorded path==leaf with no crate qualifier, so
        // the classifier would consult the LEAF against the calibrated crate rules (`fastrand` → Rand,
        // `time` → Clock). When that call RESOLVES TO A LOCAL DEFINITION, the local resolution is
        // authoritative and the external bare-leaf classification must be SUPPRESSED — tokio's pure
        // `FastRand::fastrand` xorshift must not fabricate Rand (it propagated to ~14 fns incl
        // `Runtime::new` in a real sweep). A genuine external `fastrand::u32()` call — qualified, NOT
        // local — must STILL classify Rand (no real effect dropped).
        let d = std::env::temp_dir().join(format!("candor-scan-localcrate-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(d.join("src")).unwrap();
        std::fs::write(d.join("Cargo.toml"), "[package]\nname = \"localcrate\"\n").unwrap();
        std::fs::write(
            d.join("src/lib.rs"),
            r#"
            pub struct FastRand { one: u32 }
            impl FastRand {
                // a PURE local method merely NAMED like the `fastrand` crate (tokio's xorshift)
                pub fn fastrand(&mut self) -> u32 { self.one ^= self.one << 1; self.one }
                pub fn fastrand_n(&mut self, n: u32) -> u32 { self.fastrand() % n }
                // a local method named like the `time`/`now` clock verb — also pure
                pub fn time(&self) -> u32 { self.one }
            }
            pub fn uses_local(r: &mut FastRand) { let _ = r.fastrand_n(5); let _ = r.time(); }
            // a REAL external dependency call — qualified, does NOT resolve locally → STILL Rand
            pub fn uses_external() { let _ = fastrand::u32(0..10); }
            "#,
        )
        .unwrap();
        let idx = load_dep_reports(None);
        let prefix = d.join("out/r").to_string_lossy().into_owned();
        let (rc, body) = scan_one(&d.to_string_lossy(), ScanOpts {
            prefix, want_json: true, include_tests: false, policy: None, quiet: true, deps_idx: &idx,
        });
        assert_eq!(rc, 0);
        let body = body.expect("want_json returns the report body");
        let v: serde_json::Value = serde_json::from_str(&body).unwrap();
        // The report only emits EFFECTFUL functions (pure ones are omitted), so the effect key is
        // `inferred`; a function ABSENT from the report carries no effect (the desired outcome here).
        let effects_of = |needle: &str| -> Vec<String> {
            v["functions"].as_array().into_iter().flatten()
                .filter(|f| f["fn"].as_str().is_some_and(|q| q.contains(needle)))
                .flat_map(|f| f["inferred"].as_array().into_iter().flatten()
                    .filter_map(|e| e.as_str().map(String::from)).collect::<Vec<_>>())
                .collect()
        };
        // local `FastRand::fastrand`/`fastrand_n`/`time` and their callers carry NO crate effect.
        for q in ["FastRand::fastrand", "FastRand::fastrand_n", "FastRand::time", "uses_local"] {
            let eff = effects_of(q);
            assert!(!eff.contains(&"Rand".to_string()) && !eff.contains(&"Clock".to_string()),
                    "local method named like a crate fabricated an effect on {q}: {eff:?}\n{body}");
        }
        // the genuine external `fastrand::u32` call — unresolved locally — STILL classifies Rand.
        assert!(effects_of("uses_external").contains(&"Rand".to_string()),
                "a real external fastrand::u32 call must still report Rand:\n{body}");
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn local_fn_or_method_named_like_an_ffi_tier_does_not_fabricate() {
        // The leaf-PREFIX FFI tiers (`sqlite3_`/`git_`/`curl_`/`SSL_`) and whole-crate Rand
        // (`getrandom`/`fastrand`) classify by leaf name independent of the binding crate. A PURE local
        // FREE FN or qualified `Type::method` whose name collides was classified anyway — FABRICATION on a
        // provably-pure path that transitively poisons every caller (the cardinal sin). The general
        // local-resolution suppression must cover the free-fn and qualified-method cases the bare-leaf
        // guard missed. A genuine FFI binding (an `extern "C"` decl, no Rust body) must STILL classify.
        let d = std::env::temp_dir().join(format!("candor-scan-ffiname-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(d.join("src")).unwrap();
        std::fs::write(d.join("Cargo.toml"), "[package]\nname = \"ffiname\"\n").unwrap();
        std::fs::write(
            d.join("src/lib.rs"),
            r#"
            // PURE local free fns named like FFI tiers / whole-crate rules
            pub fn sqlite3_step() -> i32 { 0 }
            pub fn git_clone() {}
            pub fn getrandom() -> u32 { 4 }
            pub fn uses_sqlite() -> i32 { sqlite3_step() }
            pub fn uses_git() { git_clone() }
            pub fn uses_rand() -> u32 { getrandom() }
            // a PURE local qualified Type::method named like the git_ tier
            pub struct Repo;
            impl Repo { pub fn git_remote_fetch(&self) {} }
            pub fn uses_method(r: &Repo) { r.git_remote_fetch() }
            // AMBIGUOUS bare leaf: a free fn AND a method share an FFI-named leaf, defeating
            // resolve_target's uniqueness filter — the bare-leaf classifier must STILL be suppressed.
            pub fn curl_easy_perform() {}
            pub struct Conn;
            impl Conn { pub fn curl_easy_perform(&self) {} }
            pub fn uses_ambig() { curl_easy_perform() }
            // a GENUINE FFI binding (extern decl, no Rust body) — must STILL classify Db
            extern "C" { fn sqlite3_exec(p: *mut i8) -> i32; }
            pub fn real_ffi() { unsafe { sqlite3_exec(std::ptr::null_mut()); } }
            "#,
        )
        .unwrap();
        let idx = load_dep_reports(None);
        let prefix = d.join("out/r").to_string_lossy().into_owned();
        let (rc, body) = scan_one(&d.to_string_lossy(), ScanOpts {
            prefix, want_json: true, include_tests: false, policy: None, quiet: true, deps_idx: &idx,
        });
        assert_eq!(rc, 0);
        let body = body.expect("want_json returns the report body");
        let v: serde_json::Value = serde_json::from_str(&body).unwrap();
        let effects_of = |needle: &str| -> Vec<String> {
            v["functions"].as_array().into_iter().flatten()
                .filter(|f| f["fn"].as_str().is_some_and(|q| q.contains(needle)))
                .flat_map(|f| f["inferred"].as_array().into_iter().flatten()
                    .filter_map(|e| e.as_str().map(String::from)).collect::<Vec<_>>())
                .collect()
        };
        // every pure local fn/method (and its caller) carries NO fabricated FFI effect
        for q in ["sqlite3_step", "git_clone", "getrandom", "uses_sqlite", "uses_git", "uses_rand",
                  "git_remote_fetch", "uses_method", "curl_easy_perform", "uses_ambig"] {
            assert!(effects_of(q).is_empty(),
                    "local fn/method named like an FFI tier FABRICATED an effect on {q}: {:?}\n{body}",
                    effects_of(q));
        }
        // the genuine extern-C FFI binding still classifies Db (no real effect dropped)
        assert!(effects_of("real_ffi").contains(&"Db".to_string()),
                "a real extern-C sqlite3_exec FFI call must still report Db:\n{body}");
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn ffi_safe_wrapper_of_an_unclassified_extern_fn_discloses_unknown() {
        // §4 honesty: a SAFE WRAPPER calling an `extern "C"` fn whose NAME the classifier doesn't know
        // (`system`, `my_native_writer`) has an unknowable body — the effect could be anything — so it must
        // DISCLOSE Unknown, never read silent-pure. Before this fix the `extern` block was never collected,
        // so the call was a bare leaf resolving to nothing → pure (the cardinal sin). CONTROLS: (a) a fn
        // with NO extern call stays pure (no fabrication); (b) a wrapper of a CLASSIFIED extern leaf
        // (`sqlite3_exec` → Db) keeps the precise effect, NOT a coarse Unknown.
        let d = std::env::temp_dir().join(format!("candor-scan-ffiwrap-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(d.join("src")).unwrap();
        std::fs::write(d.join("Cargo.toml"), "[package]\nname = \"ffiwrap\"\n").unwrap();
        std::fs::write(
            d.join("src/lib.rs"),
            r#"
            extern "C" {
                fn system(cmd: *const i8) -> i32;
                fn my_native_writer(p: *const u8, n: usize) -> i32;
                fn sqlite3_exec(p: *mut i8) -> i32;
            }
            // safe wrappers over UNCLASSIFIED extern fns → must DISCLOSE Unknown
            pub fn run_shell(cmd: *const i8) -> i32 { unsafe { system(cmd) } }
            pub fn native_write(p: *const u8, n: usize) -> i32 { unsafe { my_native_writer(p, n) } }
            // a transitive caller inherits the Unknown
            pub fn does_native_io() -> i32 { native_write(std::ptr::null(), 0) }
            // CONTROL (a): a genuinely-pure fn with NO extern call stays pure
            pub fn pure_math(a: i32, b: i32) -> i32 { a + b }
            // CONTROL (b): a wrapper of a CLASSIFIED extern leaf keeps the PRECISE effect (Db), not Unknown
            pub fn run_query() -> i32 { unsafe { sqlite3_exec(std::ptr::null_mut()) } }
            "#,
        )
        .unwrap();
        let idx = load_dep_reports(None);
        let prefix = d.join("out/r").to_string_lossy().into_owned();
        let (rc, body) = scan_one(&d.to_string_lossy(), ScanOpts {
            prefix, want_json: true, include_tests: false, policy: None, quiet: true, deps_idx: &idx,
        });
        assert_eq!(rc, 0);
        let body = body.expect("want_json returns the report body");
        let v: serde_json::Value = serde_json::from_str(&body).unwrap();
        let effects_of = |needle: &str| -> Vec<String> {
            v["functions"].as_array().into_iter().flatten()
                .filter(|f| f["fn"].as_str().is_some_and(|q| q == needle))
                .flat_map(|f| f["inferred"].as_array().into_iter().flatten()
                    .filter_map(|e| e.as_str().map(String::from)).collect::<Vec<_>>())
                .collect()
        };
        // THE FIX: unclassified-extern wrappers disclose Unknown (not silent-pure)
        assert!(effects_of("run_shell").contains(&"Unknown".to_string()),
                "FFI wrapper of `system` must disclose Unknown:\n{body}");
        assert!(effects_of("native_write").contains(&"Unknown".to_string()),
                "FFI wrapper of `my_native_writer` must disclose Unknown:\n{body}");
        // the unknown propagates transitively
        assert!(effects_of("does_native_io").contains(&"Unknown".to_string()),
                "a caller of an FFI wrapper must inherit Unknown:\n{body}");
        // the disclosure names the FFI boundary
        let why = v["functions"].as_array().into_iter().flatten()
            .find(|f| f["fn"].as_str() == Some("run_shell"))
            .and_then(|f| f.get("unknownWhy").or_else(|| f.get("unknown_why")))
            .and_then(|w| w.as_array()).map(|a| a.iter().filter_map(|x| x.as_str()).collect::<Vec<_>>())
            .unwrap_or_default();
        assert!(why.iter().any(|r| r.starts_with("native:")), "unknownWhy must name the native/FFI boundary (canonical native:): {why:?}\n{body}");
        // CONTROL (a): pure_math has NO effect — never fabricated (it's absent from the effectful report)
        assert!(effects_of("pure_math").is_empty(),
                "a pure fn with no extern call must stay pure (no fabricated Unknown):\n{body}");
        // CONTROL (b): a CLASSIFIED extern leaf keeps its precise Db, NOT a coarse Unknown
        let rq = effects_of("run_query");
        assert!(rq.contains(&"Db".to_string()), "classified extern (sqlite3_exec) must stay Db:\n{body}");
        assert!(!rq.contains(&"Unknown".to_string()),
                "a CLASSIFIED extern leaf must NOT be downgraded to Unknown:\n{body}");
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn drop_glue_of_a_local_effectful_drop_propagates_to_the_binder() {
        // §4 honesty (#3): a fn that constructs a value of a LOCAL type whose `impl Drop` does I/O must
        // inherit that Drop body's effect — the scope-exit `drop` is an implicit edge the call graph misses,
        // so a flushing/closing guard otherwise read silent-pure. CONTROLS: (a) a local type with a PURE
        // Drop adds no effect; (b) an EXTERNAL type's invisible Drop is never fabricated (we only model
        // LOCAL Drop impls).
        let d = std::env::temp_dir().join(format!("candor-scan-dropglue-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(d.join("src")).unwrap();
        std::fs::write(d.join("Cargo.toml"), "[package]\nname = \"dropglue\"\n").unwrap();
        std::fs::write(
            d.join("src/lib.rs"),
            r#"
            // a LOCAL type whose Drop writes a file (effectful scope-exit)
            pub struct FlushGuard { path: String }
            impl FlushGuard { pub fn new(p: &str) -> Self { FlushGuard { path: p.to_string() } } }
            impl Drop for FlushGuard {
                fn drop(&mut self) { std::fs::write(&self.path, b"flush").unwrap(); }
            }
            // binds a FlushGuard → the implicit drop edge must give this fn Fs
            pub fn does_work_with_guard() {
                let _g = FlushGuard::new("/tmp/x");
                let _ = 1 + 1;
            }
            // CONTROL (a): a LOCAL type with a PURE Drop — binding it adds no effect
            pub struct PureGuard;
            impl PureGuard { pub fn new() -> Self { PureGuard } }
            impl Drop for PureGuard { fn drop(&mut self) { /* nothing */ } }
            pub fn does_work_pure_guard() { let _g = PureGuard::new(); }
            "#,
        )
        .unwrap();
        let idx = load_dep_reports(None);
        let prefix = d.join("out/r").to_string_lossy().into_owned();
        let (rc, body) = scan_one(&d.to_string_lossy(), ScanOpts {
            prefix, want_json: true, include_tests: false, policy: None, quiet: true, deps_idx: &idx,
        });
        assert_eq!(rc, 0);
        let body = body.expect("want_json returns the report body");
        let v: serde_json::Value = serde_json::from_str(&body).unwrap();
        let effects_of = |needle: &str| -> Vec<String> {
            v["functions"].as_array().into_iter().flatten()
                .filter(|f| f["fn"].as_str().is_some_and(|q| q == needle))
                .flat_map(|f| f["inferred"].as_array().into_iter().flatten()
                    .filter_map(|e| e.as_str().map(String::from)).collect::<Vec<_>>())
                .collect()
        };
        // THE FIX: the binder inherits the effectful Drop's Fs via the implicit scope-exit edge
        assert!(effects_of("does_work_with_guard").contains(&"Fs".to_string()),
                "a fn binding a local guard with an effectful Drop must inherit Fs (drop glue):\n{body}");
        // CONTROL (a): a local guard with a PURE Drop fabricates nothing
        assert!(effects_of("does_work_pure_guard").is_empty(),
                "a local guard with a PURE Drop must add no effect:\n{body}");
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn drop_glue_never_fabricates_for_an_external_type() {
        // CONTROL (b) for #3, isolated: a fn that binds an EXTERNAL type (whose Drop we cannot see) must
        // NOT get a fabricated drop effect — we model ONLY local `impl Drop`. A `std::fs::File` is dropped
        // at scope exit but its Drop (close) is invisible/benign; charging the binder an effect would be a
        // fabrication. (The `std::fs::File::create` open itself is Fs via the classifier — that's correct —
        // but a fn that merely RECEIVES a File by value and lets it drop must stay pure.)
        let d = std::env::temp_dir().join(format!("candor-scan-dropext-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(d.join("src")).unwrap();
        std::fs::write(d.join("Cargo.toml"), "[package]\nname = \"dropext\"\n").unwrap();
        std::fs::write(
            d.join("src/lib.rs"),
            r#"
            // receives an external File by value; it drops here, but its Drop is not a LOCAL impl → pure
            pub fn consumes_a_file(f: std::fs::File) { let _f = f; }
            "#,
        )
        .unwrap();
        let idx = load_dep_reports(None);
        let prefix = d.join("out/r").to_string_lossy().into_owned();
        let (rc, body) = scan_one(&d.to_string_lossy(), ScanOpts {
            prefix, want_json: true, include_tests: false, policy: None, quiet: true, deps_idx: &idx,
        });
        assert_eq!(rc, 0);
        let body = body.expect("want_json returns the report body");
        let v: serde_json::Value = serde_json::from_str(&body).unwrap();
        let effs: Vec<String> = v["functions"].as_array().into_iter().flatten()
            .filter(|f| f["fn"].as_str() == Some("consumes_a_file"))
            .flat_map(|f| f["inferred"].as_array().into_iter().flatten()
                .filter_map(|e| e.as_str().map(String::from)).collect::<Vec<_>>())
            .collect();
        assert!(effs.is_empty(),
                "binding an EXTERNAL type must not fabricate a drop effect (only local Drop is modeled):\n{body}");
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn lazy_static_deferred_init_is_charged_to_the_forcing_site() {
        // THE UNDER-REPORT: a LAZY/deferred static whose init does I/O has its effect reachable from NO
        // fn (the init thunk runs on first use). Before the fix the effect vanished and every forcing site
        // read silent-pure. The fix synthesizes a `<lazy>::NAME` unit (the thunk body) and edges each
        // forcing site to it. This test asserts all four idioms light up, a PURE init fabricates nothing,
        // and the keying is per-STATIC (not module-scoped) so a pure lazy's accessor stays pure even when
        // an effectful lazy sits in the same module.
        let d = std::env::temp_dir().join(format!("candor-scan-lazystatic-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(d.join("src")).unwrap();
        // `once_cell` / `lazy_static` are declared so the κ ledger treats them as known deps; the scan
        // never builds them — the idiom is recognised syntactically.
        std::fs::write(
            d.join("Cargo.toml"),
            "[package]\nname = \"lazystatic\"\n[dependencies]\nonce_cell = \"1\"\nlazy_static = \"1\"\n",
        )
        .unwrap();
        std::fs::write(
            d.join("src/lib.rs"),
            r#"
            use once_cell::sync::Lazy;
            use std::sync::LazyLock;
            use std::fs;
            use lazy_static::lazy_static;

            // 1. once_cell Lazy, effectful init
            pub static CFG: Lazy<String> = Lazy::new(|| fs::read_to_string("/etc/a").unwrap_or_default());
            // 2. std LazyLock, effectful init
            pub static CFG2: LazyLock<String> = LazyLock::new(|| fs::read_to_string("/etc/b").unwrap_or_default());
            // 3. lazy_static!, effectful init
            lazy_static! { pub static ref CFG3: String = fs::read_to_string("/etc/c").unwrap_or_default(); }
            // 4. thread_local!, effectful init
            thread_local! { pub static CFG4: String = fs::read_to_string("/etc/d").unwrap_or_default(); }

            // NO-FABRICATION CONTROLS: pure inits contribute nothing
            pub static PURE_NUM: Lazy<usize> = Lazy::new(|| 1 + 1);

            // forcing sites — each names exactly ONE static
            pub fn force1() -> bool { CFG.contains("x") }
            pub fn force2() -> bool { CFG2.contains("x") }
            pub fn force3() -> bool { CFG3.contains("x") }
            pub fn force4() -> bool { CFG4.with(|c| c.contains("x")) }
            // MULTI-STATIC SCOPING: this fn names only the PURE lazy — must stay pure even though
            // effectful lazies live in the same module (static-scoped, not module-scoped).
            pub fn force_pure() -> usize { *PURE_NUM + 5 }
            "#,
        )
        .unwrap();
        let idx = load_dep_reports(None);
        let prefix = d.join("out/r").to_string_lossy().into_owned();
        let (rc, body) = scan_one(&d.to_string_lossy(), ScanOpts {
            prefix, want_json: true, include_tests: false, policy: None, quiet: true, deps_idx: &idx,
        });
        assert_eq!(rc, 0);
        let body = body.expect("want_json returns the report body");
        let v: serde_json::Value = serde_json::from_str(&body).unwrap();
        let effects_of = |needle: &str| -> Vec<String> {
            v["functions"].as_array().into_iter().flatten()
                .filter(|f| f["fn"].as_str() == Some(needle))
                .flat_map(|f| f["inferred"].as_array().into_iter().flatten()
                    .filter_map(|e| e.as_str().map(String::from)).collect::<Vec<_>>())
                .collect()
        };
        // THE FIX: all four idioms' forcing sites carry Fs (the effect no longer vanishes).
        for (f, idiom) in [("force1", "once_cell Lazy"), ("force2", "std LazyLock"),
                           ("force3", "lazy_static!"), ("force4", "thread_local!")] {
            assert!(effects_of(f).contains(&"Fs".to_string()),
                    "{idiom}: forcing site `{f}` must carry Fs (deferred init under-report):\n{body}");
        }
        // NO-FABRICATION: a pure-init lazy's forcing site stays pure (absent from the effectful report).
        // Also proves MULTI-STATIC scoping — `force_pure` names only PURE_NUM, so the sibling effectful
        // lazies in the same module must NOT bleed into it.
        assert!(effects_of("force_pure").is_empty(),
                "a pure-init lazy's forcing site must stay pure (no fabrication / no module-bleed):\n{body}");
        // The pure lazy's synthetic unit, if present at all, must carry no effect (it's dropped from the
        // effectful report — assert it never appears WITH an effect).
        let pure_unit_effectful = v["functions"].as_array().into_iter().flatten().any(|f| {
            f["fn"].as_str() == Some("<lazy>::PURE_NUM")
                && f["inferred"].as_array().is_some_and(|a| !a.is_empty())
        });
        assert!(!pure_unit_effectful, "the pure lazy's synthetic unit must carry no effect:\n{body}");
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn tuple_struct_fields_index_by_position() {
        // The other PROVE-IT miss: `self.0.0.run()` (ureq's ConfigBuilder newtype chain) — tuple
        // fields weren't in the FieldIndex, so the receiver never typed and the edge dropped.
        let src = r#"
            pub struct Inner;
            pub struct Outer(Inner);
            pub struct Stack(Outer);
        "#;
        let file: syn::File = syn::parse_str(src).unwrap();
        let mut uses = HashMap::new();
        let mut fields: FieldIndex = HashMap::new();
        let mut rets: HashMap<String, Option<String>> = HashMap::new();
        let (mut ti, mut td, mut tf) = (TraitImplIndex::new(), HashMap::new(), TraitFieldIndex::new());
        let (mut fe, mut ev) = (FieldElemIndex::new(), HashMap::new());
        collect_decls(&file.items, false, &mut uses, &mut fields, &mut fe, &mut rets, &mut ev, &mut ti, &mut td, &mut tf, &mut std::collections::HashSet::new(), &mut std::collections::HashSet::new(), &mut std::collections::HashSet::new(), &mut std::collections::HashMap::new(), &mut std::collections::HashSet::new());
        assert_eq!(fields["Outer"]["0"], "Inner");
        assert_eq!(fields["Stack"]["0"], "Outer");
    }

    /// Run the FULL pipeline (Pass A indexes + Pass B collection, with the same wiring as `scan_one`)
    /// over a source string and return `fn-qual -> the typed `Type::method` call paths it produced`.
    /// A receiver typed by one of the new idioms shows up as `Sender::send` here; a dropped receiver
    /// would leave only the bare leaf `send` (no `Type::` qualifier) — the silent-under-report shape.
    fn typed_calls_of(src: &str) -> HashMap<String, Vec<String>> {
        let file: syn::File = syn::parse_str(src).unwrap();
        let mut uses = HashMap::new();
        let mut fields = FieldIndex::new();
        let mut field_elem = FieldElemIndex::new();
        let mut rets: HashMap<String, Option<String>> = HashMap::new();
        let mut enum_tmp: HashMap<String, Option<String>> = HashMap::new();
        let mut ti = TraitImplIndex::new();
        let mut td: HashMap<String, LocalTrait> = HashMap::new();
        let mut tf = TraitFieldIndex::new();
        collect_decls(&file.items, false, &mut uses, &mut fields, &mut field_elem, &mut rets,
                      &mut enum_tmp, &mut ti, &mut td, &mut tf, &mut std::collections::HashSet::new(),
                      &mut std::collections::HashSet::new(), &mut std::collections::HashSet::new(), &mut std::collections::HashMap::new(), &mut std::collections::HashSet::new());
        let returns: ReturnIndex = rets.into_iter().filter_map(|(k, v)| v.map(|t| (k, t))).collect();
        let enum_variants: EnumVariantIndex =
            enum_tmp.into_iter().filter_map(|(k, v)| v.map(|t| (k, t))).collect();
        let traits = TraitIndexes { impls: &ti, decls: &td, fields: &tf };
        let elems = ElemIndexes { field_elem: &field_elem, enum_variants: &enum_variants };
        let mut fns: Vec<FnInfo> = Vec::new();
        let mut us2 = HashMap::new();
        let mut locs = Vec::new();
        fn_locs(&file.items, "lib.rs", false, &mut locs);
        let mut loc_idx = 0usize;
        scan_items(&file.items, "", &locs, &mut loc_idx, false, &fields, &returns, traits, elems, &std::collections::HashSet::new(), &mut us2, &mut fns);
        fns.into_iter()
            .map(|f| (f.qual, f.calls.into_iter().filter(|c| c.typed).map(|c| c.path).collect()))
            .collect()
    }

    /// fn-name -> `unresolved` flag, through the same full pipeline — for the opacity/callback tests.
    fn unresolved_of(src: &str) -> HashMap<String, bool> {
        let file: syn::File = syn::parse_str(src).unwrap();
        let mut uses = HashMap::new();
        let mut fields = FieldIndex::new();
        let mut field_elem = FieldElemIndex::new();
        let mut rets: HashMap<String, Option<String>> = HashMap::new();
        let mut enum_tmp: HashMap<String, Option<String>> = HashMap::new();
        let mut ti = TraitImplIndex::new();
        let mut td: HashMap<String, LocalTrait> = HashMap::new();
        let mut tf = TraitFieldIndex::new();
        collect_decls(&file.items, false, &mut uses, &mut fields, &mut field_elem, &mut rets,
                      &mut enum_tmp, &mut ti, &mut td, &mut tf, &mut std::collections::HashSet::new(),
                      &mut std::collections::HashSet::new(), &mut std::collections::HashSet::new(), &mut std::collections::HashMap::new(), &mut std::collections::HashSet::new());
        let returns: ReturnIndex = rets.into_iter().filter_map(|(k, v)| v.map(|t| (k, t))).collect();
        let enum_variants: EnumVariantIndex =
            enum_tmp.into_iter().filter_map(|(k, v)| v.map(|t| (k, t))).collect();
        let traits = TraitIndexes { impls: &ti, decls: &td, fields: &tf };
        let elems = ElemIndexes { field_elem: &field_elem, enum_variants: &enum_variants };
        let mut fns: Vec<FnInfo> = Vec::new();
        let mut us2 = HashMap::new();
        let mut locs = Vec::new();
        fn_locs(&file.items, "lib.rs", false, &mut locs);
        let mut loc_idx = 0usize;
        scan_items(&file.items, "", &locs, &mut loc_idx, false, &fields, &returns, traits, elems, &std::collections::HashSet::new(), &mut us2, &mut fns);
        fns.into_iter().map(|f| (f.qual, f.unresolved)).collect()
    }

    /// fn-qual -> its `loc` (`file:line:col`), through the same full pipeline — for the loc-fidelity test.
    fn locs_of(src: &str) -> HashMap<String, String> {
        let file: syn::File = syn::parse_str(src).unwrap();
        let mut uses = HashMap::new();
        let mut fields = FieldIndex::new();
        let mut field_elem = FieldElemIndex::new();
        let mut rets: HashMap<String, Option<String>> = HashMap::new();
        let mut enum_tmp: HashMap<String, Option<String>> = HashMap::new();
        let mut ti = TraitImplIndex::new();
        let mut td: HashMap<String, LocalTrait> = HashMap::new();
        let mut tf = TraitFieldIndex::new();
        collect_decls(&file.items, false, &mut uses, &mut fields, &mut field_elem, &mut rets,
                      &mut enum_tmp, &mut ti, &mut td, &mut tf, &mut std::collections::HashSet::new(),
                      &mut std::collections::HashSet::new(), &mut std::collections::HashSet::new(), &mut std::collections::HashMap::new(), &mut std::collections::HashSet::new());
        let returns: ReturnIndex = rets.into_iter().filter_map(|(k, v)| v.map(|t| (k, t))).collect();
        let enum_variants: EnumVariantIndex =
            enum_tmp.into_iter().filter_map(|(k, v)| v.map(|t| (k, t))).collect();
        let traits = TraitIndexes { impls: &ti, decls: &td, fields: &tf };
        let elems = ElemIndexes { field_elem: &field_elem, enum_variants: &enum_variants };
        let mut fns: Vec<FnInfo> = Vec::new();
        let mut us2 = HashMap::new();
        let mut locs = Vec::new();
        fn_locs(&file.items, "lib.rs", false, &mut locs);
        let mut loc_idx = 0usize;
        scan_items(&file.items, "", &locs, &mut loc_idx, false, &fields, &returns, traits, elems, &std::collections::HashSet::new(), &mut us2, &mut fns);
        fns.into_iter().map(|f| (f.qual, f.loc)).collect()
    }

    /// The `loc` field MUST be `file:line:col` (spec report schema), with the line being the fn's ACTUAL
    /// source line — not the file-only `src/lib.rs` an adversarial cross-engine fidelity review found.
    /// Line is 1-based; column is 1-based (proc-macro2's 0-based column + 1), pointing at the item's first
    /// token, so a top-level `fn` at column 0 reads col 1 (matching the deep engine's `build.rs:10:1`). The
    /// walk covers free fns, impl methods, nested-module fns, and trait default methods — the same set
    /// `scan_items` emits — so every loc lines up with its FnInfo.
    #[test]
    fn loc_carries_actual_line_and_col() {
        // Lines (1-based): blank=1, alpha=2, beta=4(`pub` indented?), … keep it explicit below.
        let src = "\
fn alpha() {}
    fn beta() {}
struct T;
impl T {
    fn gamma(&self) {}
}
mod inner {
    fn delta() {}
}
trait G {
    fn hello(&self) {}
}
";
        let m = locs_of(src);
        // alpha: line 1, first token `fn` at column 0 -> 1-based col 1.
        assert_eq!(m["alpha"], "lib.rs:1:1");
        // beta: line 2, indented 4 -> col 5 (1-based).
        assert_eq!(m["beta"], "lib.rs:2:5");
        // gamma: line 5 (inside impl), indented 4 -> col 5.
        assert_eq!(m["T::gamma"], "lib.rs:5:5");
        // delta: line 8 (inside `mod inner`), indented 4 -> col 5; qualified by the module path.
        assert_eq!(m["inner::delta"], "lib.rs:8:5");
        // hello: a trait DEFAULT method, line 11, indented 4 -> col 5.
        assert_eq!(m["G::hello"], "lib.rs:11:5");
    }

    /// Invoking a fn-typed binding (`cb: fn()`/`impl Fn`/`dyn Fn`/generic `F: Fn`/`Box<dyn Fn>`) calls a
    /// body the syntactic scan can't see, so the fn is `unresolved` (honest Unknown) — NOT silently pure.
    /// Found by the cross-engine generative differential: candor-scan dropped these while java/ts/swift
    /// propagated/Unknowned them. A normal free-fn call must NOT be flagged unresolved (no over-report).
    #[test]
    fn fn_typed_callback_invocation_is_unresolved() {
        for hof in [
            "fn h(cb: fn()) { cb(); }",
            "fn h(cb: impl Fn()) { cb(); }",
            "fn h<F: Fn()>(cb: F) { cb(); }",
            "fn h(cb: &dyn Fn()) { cb(); }",
            "fn h(cb: Box<dyn Fn()>) { cb(); }",
        ] {
            let m = unresolved_of(hof);
            assert!(m["h"], "fn-typed callback invocation silently dropped (not unresolved): {hof}");
        }
        // A fn-typed binding REBOUND to a local must still be Unknown when invoked (the max review
        // found the param-only seeding missed `let g = cb; g()`). Covers a plain rebind, an `if`-yield,
        // and an annotated `let g: fn()`.
        for hof in [
            "fn h(cb: impl Fn()) { let g = cb; g(); }",
            "fn h(cb: fn()) { let g = if true { cb } else { return }; g(); }",
            "fn s() {} fn h() { let g: fn() = s; g(); }",
        ] {
            let m = unresolved_of(hof);
            assert!(m["h"], "fn-typed callback rebound to a local silently dropped: {hof}");
        }
        // NO over-report: a normal free-fn call, AND a normal value rebind, stay resolved.
        let m = unresolved_of("fn helper() {} fn caller() { helper(); }");
        assert!(!m["caller"], "a normal free-fn call must not be flagged unresolved");
        let m = unresolved_of("struct T; impl T { fn m(&self) {} } fn f() { let x = T; let y = x; y.m(); }");
        assert!(!m["f"], "a normal value rebind must not be flagged unresolved");
    }

    /// A `#[cfg(test)]` free fn / impl block / impl method at module scope is test-only and must NOT
    /// appear in the default (non-`--include-tests`) report — the guard was on `mod` only.
    #[test]
    fn cfg_test_items_excluded_from_default_report() {
        let m = typed_calls_of(
            "pub fn prod() {}\n\
             #[cfg(test)] fn freefn() {}\n\
             struct S;\n\
             #[cfg(test)] impl S { fn blk(&self) {} }\n\
             struct P; impl P { #[cfg(test)] fn meth(&self) {} fn keep(&self) {} }",
        );
        assert!(m.contains_key("prod"), "a production fn must be in the report");
        assert!(m.keys().any(|k| k.ends_with("P::keep")), "a production method must be in the report");
        for leaked in ["freefn", "S::blk", "P::meth"] {
            assert!(!m.keys().any(|k| k.ends_with(leaked)), "a #[cfg(test)] item leaked: {leaked}");
        }
    }

    /// Each of the six PROVEN-dropped idioms (the silent-under-report bug): a method call whose
    /// receiver is reached via for-loop / iterator-adapter closure / subscript / nested field+subscript
    /// / enum-payload match / tuple destructure. The effectful `Sender::send` must be TYPED (so it
    /// classifies Net), mirroring the candor-swift sweep that fixed the same six.
    #[test]
    fn dropped_receiver_idioms_now_resolve() {
        let prelude = "struct Sender; impl Sender { fn send(&self) {} }\n\
                       struct Pool { senders: Vec<Sender> }\n\
                       enum Conn { Active(Sender), Idle }\n";
        let cases: &[(&str, &str)] = &[
            // 1. for-loop over a Vec<Sender> param
            ("fn f(xs: Vec<Sender>) { for c in xs { c.send(); } }", "f"),
            // 1b. for-loop over a &[Sender] param
            ("fn f(xs: &[Sender]) { for c in xs { c.send(); } }", "f"),
            // 2. iterator-adapter closure (for_each / map)
            ("fn f(xs: Vec<Sender>) { xs.iter().for_each(|c| c.send()); }", "f"),
            ("fn f(xs: Vec<Sender>) { let _ = xs.iter().map(|c| c.send()).count(); }", "f"),
            // 3. subscript
            ("fn f(xs: Vec<Sender>) { xs[0].send(); }", "f"),
            // 6. tuple destructure from a tuple-typed param
            ("fn f(p: (Sender, usize)) { let (s, _) = p; s.send(); }", "f"),
        ];
        for (body, fnname) in cases {
            let src = format!("{prelude}{body}");
            let m = typed_calls_of(&src);
            let calls = m.get(*fnname).cloned().unwrap_or_default();
            assert!(
                calls.iter().any(|c| c == "Sender::send"),
                "idiom dropped the effectful receiver (silent under-report): {body}\n  typed calls: {calls:?}"
            );
        }
        // nested field + subscript (`self.senders[0].send()`) and for-loop over a collection FIELD.
        let m = typed_calls_of(&format!(
            "{prelude}impl Pool {{ fn first(&self) {{ self.senders[0].send(); }} \
             fn each(&self) {{ for c in &self.senders {{ c.send(); }} }} }}"
        ));
        assert!(m["Pool::first"].iter().any(|c| c == "Sender::send"), "nested field+subscript dropped: {:?}", m["Pool::first"]);
        assert!(m["Pool::each"].iter().any(|c| c == "Sender::send"), "for-loop over field dropped: {:?}", m["Pool::each"]);
        // 5. enum-payload match binding (`Conn::Active(s) => s.send()`).
        let m = typed_calls_of(&format!(
            "{prelude}fn g(c: Conn) {{ match c {{ Conn::Active(s) => s.send(), Conn::Idle => {{}} }} }}"
        ));
        assert!(m["g"].iter().any(|c| c == "Sender::send"), "enum-payload match binding dropped: {:?}", m["g"]);
    }

    /// NO FABRICATION (the cardinal sin): the same six idioms over a PURE element type, or over an
    /// effect-irrelevant element, must NOT type a `Type::send` edge to anything effectful — the element
    /// is pure, so the receiver typing must stay honest. We assert the effectful `Sender::send` never
    /// appears; a pure `Pure::send` edge is fine (it classifies to nothing).
    #[test]
    fn idioms_never_fabricate_on_pure_elements() {
        let prelude = "struct Pure; impl Pure { fn send(&self) {} }\n\
                       struct Bag { items: Vec<Pure> }\n";
        let bodies: &[&str] = &[
            "fn f(xs: Vec<Pure>) { for c in xs { c.send(); } }",
            "fn f(xs: Vec<Pure>) { xs.iter().for_each(|c| c.send()); }",
            "fn f(xs: Vec<Pure>) { xs[0].send(); }",
            "fn f(xs: Vec<i32>) { for c in xs { let _ = c + 1; } }",
            "fn f(p: (Pure, usize)) { let (s, _) = p; s.send(); }",
        ];
        for body in bodies {
            let m = typed_calls_of(&format!("{prelude}{body}"));
            let calls = m.get("f").cloned().unwrap_or_default();
            // the ONLY typed edge a pure element may form is `Pure::send` — never an effectful type's.
            assert!(
                calls.iter().all(|c| c != "Sender::send" && !c.contains("TcpStream")),
                "fabricated an effectful edge on a pure element: {body}\n  typed: {calls:?}"
            );
        }
    }

    /// The candor-swift `vars`-leak lesson: a scoped binding (loop var, closure param, match payload)
    /// must NOT leak past its block into a later same-named, uninferable var. Here the first loop binds
    /// `c: Sender`; the second loop's `c` (a Pure) and a trailing free `c.send()` on an indeterminate
    /// `c` must NOT inherit `Sender` and fabricate its edge.
    #[test]
    fn scoped_bindings_do_not_leak() {
        let prelude = "struct Sender; impl Sender { fn send(&self) {} }\n\
                       struct Pure; impl Pure { fn send(&self) {} }\n\
                       fn mk() -> Pure { Pure }\n";
        // second loop over Pure: `c` must be re-typed Pure, not the prior Sender.
        let m = typed_calls_of(&format!(
            "{prelude}fn f(xs: Vec<Sender>, ys: Vec<Pure>) {{ for c in xs {{ c.send(); }} for c in ys {{ c.send(); }} }}"
        ));
        let calls = &m["f"];
        // exactly ONE Sender::send (the genuine first loop); the second loop's `c.send()` is Pure::send.
        assert_eq!(
            calls.iter().filter(|c| *c == "Sender::send").count(),
            1,
            "loop binding leaked into the next same-named loop (fabrication): {calls:?}"
        );
        // a closure param binding must not leak to a later free var of the same name.
        let m = typed_calls_of(&format!(
            "{prelude}fn f(xs: Vec<Sender>) {{ xs.iter().for_each(|c| c.send()); let c = mk(); c.send(); }}"
        ));
        let calls = &m["f"];
        assert!(
            !calls.iter().any(|c| *c == "Sender::send" && calls.iter().filter(|x| *x == "Sender::send").count() > 1),
            "closure param leaked"
        );
        // the trailing `let c = mk(); c.send()` must be Pure::send, never Sender::send.
        assert_eq!(calls.iter().filter(|c| *c == "Sender::send").count(), 1,
                   "closure param binding leaked into a later same-named var: {calls:?}");
        assert!(calls.iter().any(|c| c == "Pure::send"), "later c should type Pure::send: {calls:?}");
    }

    /// `elem_type` extracts T from every supported collection shape and resolves it through `uses`;
    /// returns None for non-collections (so a non-collection receiver is never mis-typed as an element).
    #[test]
    fn elem_type_covers_the_collection_shapes() {
        let u = uses(&[("Sender", "net::Sender")]);
        let p = |s: &str| -> Option<String> {
            let t: syn::Type = syn::parse_str(s).unwrap();
            elem_type(&t, &u)
        };
        assert_eq!(p("Vec<Sender>").as_deref(), Some("net::Sender"));
        assert_eq!(p("&[Sender]").as_deref(), Some("net::Sender"));
        assert_eq!(p("[Sender; 4]").as_deref(), Some("net::Sender"));
        assert_eq!(p("HashSet<Sender>").as_deref(), Some("net::Sender"));
        assert_eq!(p("BTreeSet<Sender>").as_deref(), Some("net::Sender"));
        assert_eq!(p("VecDeque<Sender>").as_deref(), Some("net::Sender"));
        assert_eq!(p("Box<[Sender]>").as_deref(), Some("net::Sender"));
        assert_eq!(p("Arc<Vec<Sender>>").as_deref(), Some("net::Sender"));
        // a non-collection is NOT an element source
        assert_eq!(p("Sender"), None);
        assert_eq!(p("Option<Sender>"), None);
        // a map's value carries the element only via `.values()` (not the bare type here)
        assert_eq!(p("HashMap<String, Sender>"), None);
    }

    /// The Pass-A enum-variant index keeps only UNAMBIGUOUS single-payload variant leaves; a leaf two
    /// enums share with different payloads is dropped (never guess), like the return-index rule.
    #[test]
    fn enum_variant_index_drops_ambiguous_leaves() {
        let src = "enum A { One(i32), Pair(i32, i32), Unit }\n\
                   enum B { Two(String) }\n\
                   enum C { Two(Vec<u8>) }\n"; // `Two` conflicts across B and C → ambiguous, dropped
        let file: syn::File = syn::parse_str(src).unwrap();
        let mut uses = HashMap::new();
        let mut fields = FieldIndex::new();
        let mut field_elem = FieldElemIndex::new();
        let mut rets: HashMap<String, Option<String>> = HashMap::new();
        let mut enum_tmp: HashMap<String, Option<String>> = HashMap::new();
        let (mut ti, mut td, mut tf) = (TraitImplIndex::new(), HashMap::new(), TraitFieldIndex::new());
        collect_decls(&file.items, false, &mut uses, &mut fields, &mut field_elem, &mut rets,
                      &mut enum_tmp, &mut ti, &mut td, &mut tf, &mut std::collections::HashSet::new(),
                      &mut std::collections::HashSet::new(), &mut std::collections::HashSet::new(), &mut std::collections::HashMap::new(), &mut std::collections::HashSet::new());
        let ev: EnumVariantIndex = enum_tmp.into_iter().filter_map(|(k, v)| v.map(|t| (k, t))).collect();
        assert_eq!(ev.get("One").map(String::as_str), Some("i32")); // single-payload: kept
        assert_eq!(ev.get("Pair"), None);                           // multi-field: not indexed
        assert_eq!(ev.get("Unit"), None);                           // unit variant: not indexed
        assert_eq!(ev.get("Two"), None);                            // conflicting payloads: dropped
    }

    #[test]
    fn embedded_agents_contract_matches_the_repo_doc() {
        // --agents prints the contract EMBEDDED at build time; this gate keeps the packaged copy
        // (crates/candor-scan/AGENTS.md, the only file a crates.io tarball can carry) in lockstep
        // with the repo-root AGENTS.md. If this fails: cp AGENTS.md crates/candor-scan/AGENTS.md
        let embedded = include_str!("../AGENTS.md");
        assert!(embedded.contains("candor-scan"), "the contract must describe this tool");
        // The repo-root doc exists ONLY in a workspace checkout. In a published-crate / `cargo
        // vendor` layout `../../AGENTS.md` is absent (panic) or an UNRELATED file (false diff), so
        // `cargo test` on the shipped crate would fail spuriously. Only assert the drift gate when
        // the root doc is actually present AND is candor's own (contains the marker) — otherwise
        // skip: the include_str above already proves the packaged copy compiles in.
        match std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/../../AGENTS.md")) {
            Ok(root) if root.contains("instructions for an AI coding agent") => {
                assert_eq!(embedded, root, "crate AGENTS.md drifted from the repo root — re-copy it");
            }
            _ => { /* not a workspace checkout (registry/vendor layout) — drift gate N/A */ }
        }
    }

    #[test]
    fn toml_primitives_tolerate_spacing_and_comments() {
        // The shared toml_section/toml_scalar fix a latent inconsistency: a `[ spaced ]` header and a
        // trailing `# comment` are now handled uniformly across all three manifest readers.
        assert_eq!(toml_section("[ workspace ]"), Some("workspace"));
        assert_eq!(toml_section("[package]"), Some("package"));
        assert_eq!(toml_section("name = \"x\""), None);
        assert_eq!(toml_scalar("name = \"my-crate\"  # the name", "name"), Some("my-crate"));
        assert_eq!(toml_scalar("name=bare # c", "name"), Some("bare"));
        assert_eq!(toml_scalar("namespace = \"x\"", "name"), None); // key is whole, not a prefix
        // read_crate_name through a spaced header + comment.
        let d = std::env::temp_dir().join(format!("candor-scan-tomlhdr-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        std::fs::write(d.join("Cargo.toml"), "[ package ]\nname = \"spaced-crate\"  # trailing\n").unwrap();
        assert_eq!(read_crate_name(&d).as_deref(), Some("spaced_crate"));
        // toml_string_array through a spaced [ workspace ] header.
        assert_eq!(
            toml_string_array("[ workspace ]\nmembers = [\"a\", \"b\"]\n", "workspace", "members"),
            vec!["a", "b"]
        );
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn cargo_deps_excludes_nested_package_manifests() {
        // The κ dep universe is the crate's OWN deps — a nested package (fixture, path-dep) is scanned
        // separately, so its deps must not pollute the parent's ledger (matching the source walk).
        let d = std::env::temp_dir().join(format!("candor-scan-nesteddeps-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(d.join("fixture")).unwrap();
        std::fs::write(d.join("Cargo.toml"), "[package]\nname = \"outer\"\n[dependencies]\nserde = \"1\"\n").unwrap();
        std::fs::write(d.join("fixture/Cargo.toml"), "[package]\nname = \"inner\"\n[dependencies]\nreqwest = \"0.12\"\n").unwrap();
        let (deps, _) = cargo_deps(&d.to_string_lossy());
        assert!(deps.contains("serde"), "the crate's own dep is present: {deps:?}");
        assert!(!deps.contains("reqwest"), "a nested package's dep leaked into the parent: {deps:?}");
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn toml_string_array_reads_inline_and_multiline_members() {
        let txt = "[package]\nname = \"x\"\n\n[workspace]\nmembers = [\"crates/a\", \"crates/b\"]\nexclude = [\n  \"eval\",\n  \"sample\",\n]\n";
        assert_eq!(toml_string_array(txt, "workspace", "members"), vec!["crates/a", "crates/b"]);
        assert_eq!(toml_string_array(txt, "workspace", "exclude"), vec!["eval", "sample"]);
        assert!(toml_string_array(txt, "workspace", "default-members").is_empty());
        assert!(toml_string_array("[dependencies]\nserde = \"1\"\n", "workspace", "members").is_empty());
    }

    #[test]
    fn workspace_members_expand_globs_and_honour_exclude() {
        let d = std::env::temp_dir().join(format!("candor-scan-ws-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        for m in ["crates/a", "crates/skipme", "tools/one", "crates/no-manifest"] {
            std::fs::create_dir_all(d.join(m)).unwrap();
            if m != "crates/no-manifest" {
                std::fs::write(d.join(m).join("Cargo.toml"), "[package]\nname = \"m\"\n").unwrap();
            }
        }
        std::fs::write(
            d.join("Cargo.toml"),
            "[workspace]\nmembers = [\"crates/*\", \"tools/one\", \"gone/away\"]\nexclude = [\"crates/skipme\"]\n",
        )
        .unwrap();
        let got: Vec<String> = workspace_members(&d)
            .into_iter()
            .map(|p| p.strip_prefix(&format!("{}/", d.to_string_lossy())).unwrap().to_string())
            .collect();
        assert_eq!(got, vec!["crates/a", "tools/one"], "glob expands, exclude + missing-manifest drop");
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn workspace_members_bare_star_and_dedup() {
        let d = std::env::temp_dir().join(format!("candor-scan-ws2-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        for m in ["a", "b"] {
            std::fs::create_dir_all(d.join(m)).unwrap();
            std::fs::write(d.join(m).join("Cargo.toml"), "[package]\nname = \"m\"\n").unwrap();
        }
        // bare `*` = immediate children; AND `a` listed explicitly too — must dedup, not double-scan.
        std::fs::write(d.join("Cargo.toml"), "[workspace]\nmembers = [\"*\", \"a\"]\n").unwrap();
        let got: Vec<String> = workspace_members(&d)
            .into_iter()
            .map(|p| p.strip_prefix(&format!("{}/", d.to_string_lossy())).unwrap().to_string())
            .collect();
        assert_eq!(got, vec!["a", "b"], "bare * expands to children, deduped against the explicit `a`");
        assert!(has_workspace_table(&d));
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn workspace_root_scans_members_even_under_deps_filter() {
        // The --deps × workspace regression: the nested-package filter prunes member dirs, so a
        // workspace root scanned as one crate yields an empty report. scan_target must fan out.
        let d = std::env::temp_dir().join(format!("candor-scan-wsfan-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        for (m, body) in [("a", "pub fn ea() { let _ = std::fs::read(\"/x\"); }"),
                          ("b", "pub fn eb() { let _ = std::process::Command::new(\"x\"); }")] {
            std::fs::create_dir_all(d.join(m).join("src")).unwrap();
            std::fs::write(d.join(m).join("Cargo.toml"), format!("[package]\nname = \"{m}\"\n")).unwrap();
            std::fs::write(d.join(m).join("src/lib.rs"), body).unwrap();
        }
        std::fs::write(d.join("Cargo.toml"), "[workspace]\nmembers = [\"a\", \"b\"]\n").unwrap();
        let idx = load_dep_reports(None);
        let prefix = d.join("out/r").to_string_lossy().into_owned();
        let rc = scan_target(&d.to_string_lossy(), prefix.clone(), false, false, None, &idx);
        assert_eq!(rc, 0);
        let ra = std::fs::read_to_string(format!("{prefix}.a.scan.json")).unwrap();
        let rb = std::fs::read_to_string(format!("{prefix}.b.scan.json")).unwrap();
        assert!(ra.contains("ea") && ra.contains("Fs"), "member a not scanned: {ra}");
        assert!(rb.contains("eb") && rb.contains("Exec"), "member b not scanned: {rb}");
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn nested_packages_are_not_modules_of_the_parent() {
        // The repo-root self-scan merged 194 eval-fixture `main`s into ONE unit (un-namespaced
        // collision -> cross-wired inheritance). A subtree with its own Cargo.toml is a different
        // package: the parent's walk must not descend into it.
        let d = std::env::temp_dir().join(format!("candor-scan-nested-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(d.join("src")).unwrap();
        std::fs::create_dir_all(d.join("fixture/src")).unwrap();
        std::fs::write(d.join("Cargo.toml"), "[package]\nname = \"outer\"\n").unwrap();
        std::fs::write(d.join("src/lib.rs"), "pub fn outer_eff() { let _ = std::fs::read(\"/x\"); }\n").unwrap();
        std::fs::write(d.join("fixture/Cargo.toml"), "[package]\nname = \"inner\"\n").unwrap();
        std::fs::write(
            d.join("fixture/src/lib.rs"),
            "pub fn inner_eff() { let _ = std::process::Command::new(\"x\"); }\n",
        )
        .unwrap();
        let idx = load_dep_reports(None);
        let prefix = d.join("out/r").to_string_lossy().into_owned();
        let (rc, _) = scan_one(&d.to_string_lossy(), ScanOpts {
            prefix: prefix.clone(), want_json: false, include_tests: false,
            policy: None, quiet: true, deps_idx: &idx,
        });
        assert_eq!(rc, 0);
        let rep = std::fs::read_to_string(format!("{prefix}.outer.scan.json")).unwrap();
        assert!(rep.contains("outer_eff"), "the parent's own fn must report: {rep}");
        assert!(!rep.contains("inner_eff") && !rep.contains("Exec"),
                "nested package's fn leaked into the parent report: {rep}");
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn report_is_written_atomically_no_tmp_leftovers() {
        // A concurrent `candor-query` / `cargo candor watch` reader must never observe a half-written
        // report (write_atomic: temp + rename). We assert the rename discipline by its observable
        // effect: the scan leaves NO `.tmp.*` file behind, and both written files are WHOLE valid JSON.
        let d = std::env::temp_dir().join(format!("candor-scan-atomic-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(d.join("src")).unwrap();
        std::fs::write(d.join("Cargo.toml"), "[package]\nname = \"atomiccrate\"\n").unwrap();
        std::fs::write(d.join("src/lib.rs"), "pub fn eff() { let _ = std::fs::read(\"/x\"); }\n").unwrap();
        let idx = load_dep_reports(None);
        let outdir = d.join("out");
        let prefix = outdir.join("r").to_string_lossy().into_owned();
        let (rc, _) = scan_one(&d.to_string_lossy(), ScanOpts {
            prefix, want_json: false, include_tests: false, policy: None, quiet: true, deps_idx: &idx,
        });
        assert_eq!(rc, 0);
        // no temp turds: every output file ends in `.json`, never `.tmp.<pid>`.
        let leftovers: Vec<String> = std::fs::read_dir(&outdir).unwrap()
            .filter_map(|e| e.ok().map(|e| e.file_name().to_string_lossy().into_owned()))
            .filter(|n| n.contains(".tmp")).collect();
        assert!(leftovers.is_empty(), "atomic write left temp files behind: {leftovers:?}");
        // both files parse as a whole — the post-rename invariant a concurrent reader relies on.
        for name in ["r.atomiccrate.scan.json", "r.atomiccrate.scan.callgraph.json"] {
            let body = std::fs::read_to_string(outdir.join(name)).unwrap();
            serde_json::from_str::<serde_json::Value>(&body)
                .unwrap_or_else(|e| panic!("{name} is not whole JSON ({e}): {body}"));
        }
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn classifier_resolves_a_std_fs_call() {
        // guards the shared-classifier contract the scanner relies on: an expanded std::fs path is Fs.
        assert_eq!(candor_classify::classify("std", "std::fs::read_to_string"), Some("Fs"));
        assert_eq!(candor_classify::classify("std", "std::process::Command::new"), Some("Exec"));
    }

    #[test]
    fn builder_chain_entries_over_approximate_for_the_syntactic_scanner() {
        // The real-world oracle caught candor-scan silent-pure'ing builder chains whose effect candor-
        // classify keys on a terminal VERB it can't type-resolve: `duct::cmd!(...).run()` (macro entry) and
        // `ureq::get(url)...call()` (fn entry). Over-approximate the ENTRY here (scan-only); the shared
        // classifier keeps the entry pure so the DEEP engine (which types the verb) stays precise.
        assert_eq!(scan_builder_entry_effect("duct", "duct::cmd"), Some("Exec"));
        assert_eq!(scan_builder_entry_effect("duct", "duct::sh"), Some("Exec"));
        assert_eq!(scan_builder_entry_effect("ureq", "ureq::get"), Some("Net"));
        assert_eq!(scan_builder_entry_effect("ureq", "ureq::post"), Some("Net"));
        assert_eq!(scan_builder_entry_effect("sqlx", "sqlx::query"), Some("Db"));
        assert_eq!(scan_builder_entry_effect("diesel", "diesel::sql_query"), Some("Db"));
        // terminal verbs stay classify()'s job; an unrelated path is None:
        assert_eq!(scan_builder_entry_effect("duct", "duct::Expression::run"), None);
        assert_eq!(scan_builder_entry_effect("ureq", "ureq::Request::call"), None);
        assert_eq!(scan_builder_entry_effect("std", "std::process::Command::new"), None);
        // invariant the deep engine relies on stays intact (entries pure in the SHARED classifier):
        assert_eq!(candor_classify::classify("duct", "duct::cmd"), None);
        assert_eq!(candor_classify::classify("ureq", "ureq::get"), None);
    }

    #[test]
    fn macro_invocation_never_mints_a_local_edge() {
        // REGRESSION (review F1): a crate-LOCAL qualified macro (`crate::helpers::trace!`) expands to
        // `helpers::trace`, KEEPING its `::` — so before the `is_macro` guard it mis-linked to a same-named
        // LOCAL fn and FABRICATED that fn's effect onto a pure caller (the phantom-edge cardinal sin). The
        // guard must be SURGICAL: a genuine (non-macro) call to the same fn STILL inherits the effect, and
        // a genuine external classified emit-macro (`log::info!`) STILL attributes its effect.
        let idx = load_dep_reports(None);
        let run = |name: &str, src: &str| -> serde_json::Value {
            let d = std::env::temp_dir().join(format!("candor-macroedge-{name}-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&d);
            std::fs::create_dir_all(d.join("src")).unwrap();
            std::fs::write(d.join("Cargo.toml"), format!("[package]\nname = \"{name}\"\n")).unwrap();
            std::fs::write(d.join("src/lib.rs"), src).unwrap();
            let prefix = d.join("out/r").to_string_lossy().into_owned();
            let (rc, body) = scan_one(&d.to_string_lossy(), ScanOpts {
                prefix, want_json: true, include_tests: false, policy: None, quiet: true, deps_idx: &idx,
            });
            assert_eq!(rc, 0);
            let v = serde_json::from_str(&body.unwrap()).unwrap();
            let _ = std::fs::remove_dir_all(&d);
            v
        };
        let eff = |v: &serde_json::Value, needle: &str| -> Vec<String> {
            v["functions"].as_array().into_iter().flatten()
                .filter(|f| f["fn"].as_str().is_some_and(|q| q.contains(needle)))
                .flat_map(|f| f["inferred"].as_array().into_iter().flatten()
                    .filter_map(|e| e.as_str().map(String::from)).collect::<Vec<_>>()).collect()
        };
        let local = "mod helpers { pub fn trace() { let _ = std::fs::read(\"/x\"); } }\n";
        // The bug: a pure fn whose ONLY body is the LOCAL macro must NOT inherit `helpers::trace`'s Fs.
        let bug = run("macedge", &format!("{local}pub fn pure_caller() {{ crate::helpers::trace!(); }}"));
        assert!(eff(&bug, "pure_caller").is_empty(),
                "macro invocation FABRICATED a same-named local fn's effect onto a pure caller:\n{bug}");
        // Surgical: a GENUINE (non-macro) call to the same local fn STILL edges and inherits Fs.
        let real = run("macedge2", &format!("{local}pub fn real_caller() {{ crate::helpers::trace(); }}"));
        assert!(eff(&real, "real_caller").contains(&"Fs".to_string()),
                "the is_macro guard wrongly suppressed a genuine local fn edge:\n{real}");
        // A genuine EXTERNAL classified emit-macro still attributes its effect (the intended new behavior).
        let ext = run("macedge3", "pub fn logs() { log::info!(\"hi\"); }");
        assert!(eff(&ext, "logs").contains(&"Log".to_string()),
                "an external classified emit-macro must still attribute its effect:\n{ext}");
    }

    #[test]
    fn write_macro_charges_the_local_writer_side() {
        // R14 cross-engine sweep (scan): `write!(w, ...)` to a custom `fmt::Write` writer dropped the
        // writer's effectful `write_str` — silent-pure (the deep engine had this too, fixed as HOLE 2c).
        // The writer is the arg BEFORE the format string; charge its `write_str`/`write`. A std writer
        // (`String`) must light nothing (no fabrication), and a leading `assert_eq!` operand must never be
        // mistaken for a writer (the charge is gated to the write/writeln family).
        let idx = load_dep_reports(None);
        let run = |name: &str, src: &str| -> serde_json::Value {
            let d = std::env::temp_dir().join(format!("candor-wr-{name}-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&d);
            std::fs::create_dir_all(d.join("src")).unwrap();
            std::fs::write(d.join("Cargo.toml"), format!("[package]\nname = \"{name}\"\n")).unwrap();
            std::fs::write(d.join("src/lib.rs"), src).unwrap();
            let prefix = d.join("out/r").to_string_lossy().into_owned();
            let (rc, body) = scan_one(&d.to_string_lossy(), ScanOpts {
                prefix, want_json: true, include_tests: false, policy: None, quiet: true, deps_idx: &idx,
            });
            assert_eq!(rc, 0);
            let v = serde_json::from_str(&body.unwrap()).unwrap();
            let _ = std::fs::remove_dir_all(&d);
            v
        };
        let eff = |v: &serde_json::Value, needle: &str| -> Vec<String> {
            v["functions"].as_array().into_iter().flatten()
                .filter(|f| f["fn"].as_str().is_some_and(|q| q.contains(needle)))
                .flat_map(|f| f["inferred"].as_array().into_iter().flatten()
                    .filter_map(|e| e.as_str().map(String::from)).collect::<Vec<_>>()).collect()
        };
        let w = "use std::fmt::Write as _;\nstruct Loud;\nimpl std::fmt::Write for Loud { fn write_str(&mut self, _s: &str) -> std::fmt::Result { let _ = std::fs::read(\"/x\"); Ok(()) } }\n";
        // effectful local fmt::Write writer -> Fs (the bug: this was silent-pure)
        let hit = run("wrfmt", &format!("{w}pub fn via_write(w: &mut Loud) {{ let _ = write!(w, \"hi {{}}\", 1); }}"));
        assert!(eff(&hit, "via_write").contains(&"Fs".to_string()),
                "write! to a local effectful fmt::Write writer must charge the writer side:\n{hit}");
        // std String writer -> pure (no fabrication)
        let pure = run("wrstr", "use std::fmt::Write as _;\npub fn via_str(s: &mut String) { let _ = write!(s, \"hi {}\", 1); }");
        assert!(eff(&pure, "via_str").is_empty(),
                "write! to a std String writer must stay pure:\n{pure}");
        // assert_eq! leads with operands, not a writer — must NOT be charged (gated to write/writeln)
        let asrt = run("wrassert", &format!("{w}#[derive(Debug, PartialEq)]\nstruct Tag;\npub fn via_assert(a: Tag, b: Tag) {{ assert_eq!(a, b, \"ctx {{}}\", 1); }}"));
        assert!(eff(&asrt, "via_assert").is_empty(),
                "an assert_eq! operand was wrongly charged as a writer:\n{asrt}");
    }

    #[test]
    fn qself_call_never_mints_a_bare_leaf_local_edge() {
        // REGRESSION (qself hole): `path_to_string` drops the qself receiver type, so an inherent-form
        // fully-qualified assoc call `<Vec<u8>>::new()` collapses to the BARE leaf `new`, which the by_leaf
        // route mis-linked to a unique local `new`/`dump`, FABRICATING its effect onto a pure path. The fix
        // RESTORES the receiver type (`Vec::new`), staying precise in BOTH directions: `<Vec<u8>>::new()`
        // finds no local `Vec` (no fabrication) AND `<Daemon>::new()` still resolves to the local effectful
        // `Daemon::new` (no under-report — the blunt `method:true` suppress would have lost this).
        let idx = load_dep_reports(None);
        let run = |name: &str, src: &str| -> serde_json::Value {
            let d = std::env::temp_dir().join(format!("candor-qself-{name}-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&d);
            std::fs::create_dir_all(d.join("src")).unwrap();
            std::fs::write(d.join("Cargo.toml"), format!("[package]\nname = \"{name}\"\n")).unwrap();
            std::fs::write(d.join("src/lib.rs"), src).unwrap();
            let prefix = d.join("out/r").to_string_lossy().into_owned();
            let (rc, body) = scan_one(&d.to_string_lossy(), ScanOpts {
                prefix, want_json: true, include_tests: false, policy: None, quiet: true, deps_idx: &idx,
            });
            assert_eq!(rc, 0);
            let v: serde_json::Value = serde_json::from_str(&body.unwrap()).unwrap();
            let _ = std::fs::remove_dir_all(&d);
            v
        };
        let eff = |v: &serde_json::Value, needle: &str| -> Vec<String> {
            v["functions"].as_array().into_iter().flatten()
                .filter(|f| f["fn"].as_str().is_some_and(|q| q.contains(needle)))
                .flat_map(|f| f["inferred"].as_array().into_iter().flatten()
                    .filter_map(|e| e.as_str().map(String::from)).collect::<Vec<_>>()).collect()
        };
        // A unique local `new` doing Exec; the pure builder uses idiomatic `<Vec<u8>>::new()`.
        let bug = run("qselfnew",
            "use std::process::Command;\npub struct Daemon;\nimpl Daemon { pub fn new() -> Self { Command::new(\"/bin/sh\").status().unwrap(); Daemon } }\npub fn pure_build() -> Vec<u8> { let mut v = <Vec<u8>>::new(); v.push(1); v }");
        assert!(eff(&bug, "pure_build").is_empty(),
                "a qself assoc call FABRICATED a same-named local fn's effect onto a pure caller:\n{bug}");
        // Surgical: a genuine bare free-fn call STILL edges and inherits the effect.
        let real = run("qselfreal",
            "use std::fs;\npub fn dump() { fs::write(\"/x\", b\"d\").unwrap(); }\npub fn real_caller() { dump(); }");
        assert!(eff(&real, "real_caller").contains(&"Fs".to_string()),
                "the qself guard wrongly suppressed a genuine bare free-fn edge:\n{real}");
        // A qualified-tail qself into a LOCAL trait default still links (the accepted trait-default band).
        let band = run("qselfband",
            "use std::fs;\npub trait Marker { fn go() { fs::write(\"/x\", b\"d\").unwrap(); } }\npub fn via_trait_ufcs() { <SomeType as Marker>::go(); }");
        assert!(eff(&band, "via_trait_ufcs").contains(&"Fs".to_string()),
                "a qualified-tail qself into a local trait default must still link:\n{band}");
        // PRECISION (what the blunt method:true suppress would have lost): an INHERENT qself on a LOCAL type
        // whose assoc fn is effectful must STILL propagate — the restored `Daemon::new` tail resolves it.
        let local = run("qselflocal",
            "use std::process::Command;\npub struct Daemon;\nimpl Daemon { pub fn new() -> Self { Command::new(\"/bin/sh\").status().unwrap(); Daemon } }\npub fn boot() { let _d = <Daemon>::new(); }");
        assert!(eff(&local, "boot").contains(&"Exec".to_string()),
                "an inherent qself on a LOCAL effectful assoc fn must NOT be under-reported:\n{local}");
    }

    #[test]
    fn resolve_target_is_precise_and_never_fabricates() {
        // Exercises the PRODUCTION `resolve_target` (not a copy) so a regression in `run`'s resolution is
        // caught here. Defs: a unique `bool` free fn, a unique `start` method, a unique `Worker::run`
        // method, and TWO same-named `Job::run` methods in different modules (an ambiguous 2-segment tail).
        let mut by_leaf: HashMap<String, Vec<String>> = HashMap::new();
        let mut by_tail2: HashMap<String, Vec<String>> = HashMap::new();
        for q in ["random::bool::bool", "clip::ClipboardThread::start", "util::helper",
                  "app::Worker::run", "a::Job::run", "b::Job::run"] {
            by_leaf.entry(q.rsplit("::").next().unwrap().into()).or_default().push(q.into());
            by_tail2.entry(tail2(q).unwrap()).or_default().push(q.into());
        }
        // (a) qualified `Value::bool(..)` — external `Value`, tail absent locally → NONE (never the
        // unique-leaf `random::bool::bool`; the original nushell Rand-on-146-fns fabrication).
        assert_eq!(resolve_target("Value::bool", "bool", false, &by_tail2, &by_leaf), None);
        // (b) unresolved-receiver method `range.start()` → NONE (never the unique `ClipboardThread::start`).
        assert_eq!(resolve_target("start", "start", true, &by_tail2, &by_leaf), None);
        // (c) unqualified free call `helper()` with a unique def → resolves.
        assert_eq!(resolve_target("helper", "helper", false, &by_tail2, &by_leaf),
                   Some(&vec!["util::helper".to_string()]));
        // (d) associated-fn call `Worker::run()` (qualified, unique tail) → resolves to the one local def.
        assert_eq!(resolve_target("Worker::run", "run", false, &by_tail2, &by_leaf),
                   Some(&vec!["app::Worker::run".to_string()]));
        // (e) AMBIGUOUS tail `Job::run` (two types, two modules) → NONE: linking both would fabricate one
        // type's effect onto the other's caller (the bug the `len()==1` filter on the tail2 branch fixes).
        assert_eq!(resolve_target("Job::run", "run", false, &by_tail2, &by_leaf), None);
    }

    // A REFLECTIVE guard on the `--incremental` cache key. `decl_index_digest` decides whether a cached
    // file's Pass-B FnInfos may be reused; a `MergedDecls` field that steers resolution but is MISSING
    // from the digest = a stale cache silently returning unsound effect sets. Rust has no runtime field
    // reflection, so this stands in for it two ways: (1) the exhaustive destructure below stops COMPILING
    // the moment a field is added to `MergedDecls` until you bind it here — forcing the question "is it in
    // the digest?"; (2) every field is then mutated in isolation and the digest MUST move, proving each is
    // actually folded in. Add a field → add its `_` binding AND a mutator case, or the build/test fails.
    #[test]
    fn every_merged_decls_field_is_folded_into_the_digest() {
        // (1) Compile-time exhaustiveness: no `..`, so a new field breaks this line until it's listed.
        let MergedDecls {
            fields: _,
            field_elem: _,
            rets: _,
            enum_tmp: _,
            trait_impls: _,
            trait_decls: _,
            trait_fields: _,
            prim_aliases: _,
            extern_fns: _,
            drop_types: _,
            deref_target: _,
            lazy_statics: _,
        } = MergedDecls::default();

        let empty = decl_index_digest(&MergedDecls::default());

        // (2) One mutator per field — each touches exactly that field and nothing else.
        type Mutator = fn(&mut MergedDecls);
        let mutators: Vec<(&str, Mutator)> = vec![
            ("fields", |m| { m.fields.entry("S".into()).or_default().insert("f".into(), "T".into()); }),
            ("field_elem", |m| { m.field_elem.entry("S".into()).or_default().insert("f".into(), "E".into()); }),
            ("rets", |m| { m.rets.insert("f".into(), Some("T".into())); }),
            ("enum_tmp", |m| { m.enum_tmp.insert("v".into(), Some("E".into())); }),
            ("trait_impls", |m| { m.trait_impls.entry("Tr".into()).or_default().push("Ty".into()); }),
            ("trait_decls", |m| { m.trait_decls.entry("Tr".into()).or_default().count += 1; }),
            ("trait_fields", |m| { m.trait_fields.entry("S".into()).or_default().insert("f".into(), vec!["b".into()]); }),
            ("prim_aliases", |m| { m.prim_aliases.insert("A".into()); }),
            ("extern_fns", |m| { m.extern_fns.insert("system".into()); }),
            ("drop_types", |m| { m.drop_types.insert("Guard".into()); }),
            ("lazy_statics", |m| { m.lazy_statics.insert("CONFIG".into()); }),
        ];
        for (name, mutate) in mutators {
            let mut m = MergedDecls::default();
            mutate(&mut m);
            assert_ne!(
                decl_index_digest(&m), empty,
                "MergedDecls.{name} changes the index but NOT the digest — the --incremental cache would \
                 reuse stale FnInfos. Fold `{name}` into decl_index_digest().",
            );
        }
    }
}
