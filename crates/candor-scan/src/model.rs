//! The scanner's data model: a parsed call, a function's collected info, and the
//! whole-crate name indexes Pass A builds for Pass B's resolution.

use crate::*;

/// A call observed in a function body: the (use-expanded) path string and the leaf name.
//
// The serde attributes are PURELY a cache-wire-format optimization (short field names + omit the common
// defaults): they shrink the consolidated cache, which is read+written every incremental scan. They do
// NOT change any in-memory behaviour — the deserialized value is identical, and `serde(default)` restores
// the omitted fields. The equivalence fuzzer guards that this representation round-trips exactly.
#[derive(Clone, serde::Serialize, serde::Deserialize)]
pub(crate) struct Call {
    #[serde(rename = "p")]
    pub(crate) path: String,            // "std::fs::read", "compute_price", "pricing::priced"
    #[serde(rename = "l")]
    pub(crate) leaf: String,            // last segment
    #[serde(rename = "s", default, skip_serializing_if = "Option::is_none")]
    pub(crate) str_arg: Option<String>, // first string-literal argument (host/cmd/path detail)
    /// Synthesized from receiver-type inference (`reqwest::Client::send` from `client.send()`). Used for
    /// external-crate classification ONLY — excluded from local call-graph edges, since its `Type::method`
    /// tail could spuriously link to a same-named LOCAL method the call doesn't actually target.
    #[serde(rename = "t", default, skip_serializing_if = "std::ops::Not::not")]
    pub(crate) typed: bool,
    /// A METHOD call (`x.foo()`) vs a free-function/path call (`foo()`, `m::foo()`). When the receiver type
    /// can't be inferred, an unqualified method call has NO sound bare-leaf target — linking it to a
    /// same-named def would guess (`.bool()`→free `random::bool::bool`, `range.start()`→`Clipboard::start`),
    /// fabricating that def's effect. So such calls resolve to nothing; only the receiver-typed/qualified
    /// form (the `typed` call) links a method edge. Found on nushell (Rand/Clipboard on the random cmds).
    #[serde(rename = "m", default, skip_serializing_if = "std::ops::Not::not")]
    pub(crate) method: bool,
    /// A MACRO invocation (`log::info!`, `duct::cmd!`, `crate::helpers::trace!`). Recorded so its path can
    /// be classified/builder-mapped/disclosed like an external call — but a macro is NEVER a call to a local
    /// FUNCTION, so it must be EXCLUDED from local call-graph edge resolution: a crate-local macro path
    /// (`crate::helpers::trace` after `expand`) keeps its `::` and would otherwise mis-link to a same-named
    /// local fn, fabricating that fn's effect onto a pure caller (the same hazard the `typed` flag guards).
    #[serde(rename = "mac", default, skip_serializing_if = "std::ops::Not::not")]
    pub(crate) is_macro: bool,
}

/// One function the scan found: its module-qualified name, where, and the calls in its body.
// The serde attributes are a cache-wire-format optimization only (see `Call`); in-memory behaviour is
// unchanged and the equivalence fuzzer guards the round-trip.
#[derive(Clone, serde::Serialize, serde::Deserialize)]
pub(crate) struct FnInfo {
    #[serde(rename = "q")]
    pub(crate) qual: String,
    #[serde(rename = "l")]
    pub(crate) leaf: String,
    #[serde(rename = "f")]
    pub(crate) loc: String,
    #[serde(rename = "c", default, skip_serializing_if = "Vec::is_empty")]
    pub(crate) calls: Vec<Call>,
    /// The body invoked a callable the syntactic scan can't see through — a closure / fn-pointer value
    /// (`(cb)()`, `arr[i]()`, a local bound to a closure). The target could perform ANY effect, so the
    /// function can't honestly be certified pure: it's marked `Unknown` (matching the nightly lint's
    /// soundness fallback) rather than silently reported clean.
    #[serde(rename = "u", default, skip_serializing_if = "std::ops::Not::not")]
    pub(crate) unresolved: bool,
    /// The nominal type IDENTS of this fn's RETURN type (`-> Result<Compress, E>` → `["Result","Compress",
    /// "E"]`; `-> ()` → empty). Used only by the drop-glue ESCAPE GATE: a drop-type OWNED by the return type
    /// leaves via the returned value, so its `Drop` doesn't run in THIS scope — don't charge it (else a
    /// constructor like `Compress::new` that builds an owned `Stream` and returns the `Compress` fabricates
    /// the Stream's Drop, R49). Not serialized to the report (analysis-only).
    #[serde(rename = "r", default, skip_serializing_if = "Vec::is_empty")]
    pub(crate) ret_idents: Vec<String>,
}

/// `struct-name-leaf -> { field -> expanded-type-path }`, e.g. `App -> { http: reqwest::Client }`.
/// Built crate-wide in a pre-pass so a method call on `self.http` can be resolved to its type and
/// classified by the existing per-crate method rules (`reqwest::Client::execute` -> Net).
pub(crate) type FieldIndex = HashMap<String, HashMap<String, String>>;

/// `struct-name-leaf -> { field -> ELEMENT-type-path }` for COLLECTION-typed fields, e.g.
/// `Pool -> { senders: Sender }` (the element T of `Vec<Sender>`). Lets a loop/index/closure over a
/// collection FIELD (`for c in &self.senders`, `self.senders[0].send()`) type its element so the
/// element's method calls classify, instead of silently dropping to pure (a §4 under-report).
pub(crate) type FieldElemIndex = HashMap<String, HashMap<String, String>>;

/// `Type -> { field -> element DISPATCH leaves }` — the trait-object counterpart of `FieldElemIndex`:
/// a COLLECTION field of trait objects (`handlers: Vec<Box<dyn Handler>>`, or a generic `Vec<T: Handler>`
/// on a `struct Registry<T: Handler>`). Lets a loop/`for_each` over such a FIELD (`self.handlers.iter()
/// .for_each(|h| h.handle())`) dispatch the element's method via bounded CHA, which `FieldElemIndex`
/// can't express (a `dyn` element has no nominal type). Registries/observers-as-fields are the common shape.
pub(crate) type FieldElemTraitIndex = HashMap<String, HashMap<String, Vec<String>>>;

/// `var-name -> per-position element types of a tuple binding` (`None` = position not type-resolved),
/// e.g. a `let (s, _) = pair` over a `(Sender, usize)` param records `pair -> [Some(Sender), None]`.
pub(crate) type TupleElemIndex = HashMap<String, Vec<Option<String>>>;

/// `enum-variant-leaf -> the single payload type` for SINGLE-payload tuple variants, e.g.
/// `Active -> Sender` from `enum Conn { Active(Sender) }`. Lets a match-arm binding
/// (`Conn::Active(s) => s.send()`) type `s` from the variant's payload. Only UNAMBIGUOUS variant
/// names are kept (a leaf two enums share with different payloads is dropped — never guess), mirroring
/// the return-index ambiguity rule.
pub(crate) type EnumVariantIndex = HashMap<String, String>;

/// `fn-leaf -> expanded return-type-path`, e.g. `create_pool -> sqlx::Pool` (Result/Option unwrapped).
/// Lets type inference flow through a LOCAL factory function: `let p = create_pool()?; p.fetch_one(q)`.
/// Only UNAMBIGUOUS leaves are kept — a name with two different return types across the crate is dropped
/// (no guess), like the unique-leaf call-graph rule.
pub(crate) type ReturnIndex = HashMap<String, String>;

/// Sentinel return-"type" for a fn whose return is a CALLABLE (`-> fn()`/`-> impl Fn`/`-> Box<dyn Fn>`).
/// Stored in the return index under the fn's leaf; `expr_is_fn_typed` reads it so `let g = make_cb()`
/// propagates fn-typed-ness, while `ctor_type` filters it out of var-typing (it's not a nominal type).
/// The angle brackets cannot collide with a real Rust type path.
pub(crate) const RET_FN_TYPED: &str = "<fn>";

/// Sentinel prefix for a fn whose return is a DISPATCH trait object (`-> Box<dyn Trait>` / `-> impl
/// Trait` / `-> &dyn Trait`). The trait bound leaves are joined after it (`"<dyn>Task"` /
/// `"<dyn>Read+Seek"`), so `get().run()` on such a factory resolves the receiver's TRAIT bounds and
/// runs the SAME bounded-CHA the direct trait-object control cases use — resolving to every local
/// implementor, or disclosing `Unknown` (>12 / none visible). Without this a `-> Box<dyn Trait>`
/// return had NO recordable nominal type (`type_path` drops the trait object) so the factory-call
/// receiver typed to nothing and the method dropped SILENT-PURE. The angle brackets can't collide
/// with a real Rust type path. Encoded/decoded via `ret_dyn_encode` / `ret_dyn_leaves`.
pub(crate) const RET_DYN_PREFIX: &str = "<dyn>";

/// Encode a dispatch-trait-object return's bound leaves into the `RET_DYN_PREFIX` sentinel string.
pub(crate) fn ret_dyn_encode(leaves: &[String]) -> String {
    format!("{RET_DYN_PREFIX}{}", leaves.join("+"))
}

/// Decode a `RET_DYN_PREFIX` sentinel back to its trait bound leaves, or `None` if not one.
pub(crate) fn ret_dyn_leaves(s: &str) -> Option<Vec<String>> {
    s.strip_prefix(RET_DYN_PREFIX)
        .map(|rest| rest.split('+').filter(|p| !p.is_empty()).map(str::to_string).collect())
}

/// Sentinel prefix for a fn returning a COLLECTION of trait objects (`-> Vec<Box<dyn Task>>` /
/// `-> Option<Box<dyn Task>>`): the ELEMENT's trait bound leaves ride after it. Distinct from
/// `RET_DYN_PREFIX` (which means the value ITSELF is a dyn, decoded by `resolve_recv_traits`) — this is
/// decoded by `resolve_elem_trait_leaves` so a `for d in factory() { d.run() }` dispatches the element.
pub(crate) const RET_ELEM_DYN_PREFIX: &str = "<elemdyn>";

/// Encode a collection-of-trait-objects return's ELEMENT bound leaves into the sentinel string.
pub(crate) fn ret_elem_dyn_encode(leaves: &[String]) -> String {
    format!("{RET_ELEM_DYN_PREFIX}{}", leaves.join("+"))
}

/// Decode a `RET_ELEM_DYN_PREFIX` sentinel back to its element bound leaves, or `None` if not one.
pub(crate) fn ret_elem_dyn_leaves(s: &str) -> Option<Vec<String>> {
    s.strip_prefix(RET_ELEM_DYN_PREFIX)
        .map(|rest| rest.split('+').filter(|p| !p.is_empty()).map(str::to_string).collect())
}

/// A factory returning a TUPLE with trait-object position(s) (`fn make() -> (Box<dyn Doer>, u32)`) —
/// `let (d, _) = make()` must dispatch `d.go()`. Encodes per-position bound leaves (positions `;`-joined,
/// a position's leaves `+`-joined; a concrete position is empty) so the destructure binds each dyn
/// position into `trait_vars`. Like `<dyn>`/`<elemdyn>`, filtered out of concrete var-typing (R46 tuple).
pub(crate) const RET_TUPLE_DYN_PREFIX: &str = "<tupledyn>";

pub(crate) fn ret_tuple_dyn_encode(positions: &[Vec<String>]) -> String {
    let joined = positions.iter().map(|p| p.join("+")).collect::<Vec<_>>().join(";");
    format!("{RET_TUPLE_DYN_PREFIX}{joined}")
}

/// Decode a `RET_TUPLE_DYN_PREFIX` sentinel back to its per-position bound leaves, or `None` if not one.
pub(crate) fn ret_tuple_dyn_leaves(s: &str) -> Option<Vec<Vec<String>>> {
    s.strip_prefix(RET_TUPLE_DYN_PREFIX).map(|rest| {
        rest.split(';')
            .map(|p| p.split('+').filter(|x| !x.is_empty()).map(str::to_string).collect())
            .collect()
    })
}

/// `trait leaf -> the local types that `impl Trait for Type` it` — the syntactic CHA universe for
/// dispatch-typed receivers (the JVM engine's bounded-CHA move, done on syntax). Keyed by leaf like
/// the other name indexes; includes impls of EXTERNAL traits for local types (the JVM resolves
/// interface impls the same way regardless of where the interface is declared).
pub(crate) type TraitImplIndex = HashMap<String, Vec<String>>;

/// `struct leaf -> field name -> trait bound leaves` for dispatch-typed FIELDS (`store: Box<dyn
/// Store>`) — the DI pattern `self.store.save()`, which `FieldIndex` can't carry (no concrete type).
pub(crate) type TraitFieldIndex = HashMap<String, HashMap<String, Vec<String>>>;

/// A locally-declared trait: how many declarations share the leaf (ambiguity check) and which
/// method names the declaration itself carries — CHA resolves ONLY calls to a declared method of
/// an unambiguous local trait (review found the wider rule fabricating: `impl Iterator for
/// RowIter` + `fn f(it: impl Iterator)` charged pure `f` with RowIter's Db).
#[derive(Default)]
pub(crate) struct LocalTrait {
    pub(crate) count: usize,
    pub(crate) methods: std::collections::HashSet<String>,
    /// The trait's SUPERTRAIT leaves (`trait Sub: Super + Other` → `["Super", "Other"]`). A method of a
    /// supertrait is callable on a `Sub`-bound/`dyn Sub` receiver, so dispatch checks a leaf against this
    /// trait's methods AND (transitively) its supertraits' — else `t.base()` (a Super method via a `T: Sub`
    /// bound) read silent-pure. External supertraits are recorded but resolve to nothing (documented miss).
    pub(crate) supertraits: Vec<String>,
}

/// The trait indexes Pass A builds (impl universe, local declarations, dispatch-typed fields),
/// bundled so Pass B threads one handle instead of three more arguments.
#[derive(Clone, Copy)]
pub(crate) struct TraitIndexes<'a> {
    pub(crate) impls: &'a TraitImplIndex,
    pub(crate) decls: &'a HashMap<String, LocalTrait>,
    pub(crate) fields: &'a TraitFieldIndex,
}

/// The collection/enum indexes Pass A builds (collection-field element types, single-payload enum
/// variant types), bundled so Pass B threads one handle — the way `TraitIndexes` bundles the trait ones.
#[derive(Clone, Copy)]
pub(crate) struct ElemIndexes<'a> {
    pub(crate) field_elem: &'a FieldElemIndex,
    pub(crate) field_elem_trait: &'a FieldElemTraitIndex,
    pub(crate) enum_variants: &'a EnumVariantIndex,
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
pub(crate) struct SendFile(pub(crate) syn::File);

// SAFETY: see the type doc — uniquely owned, moved once, then single-threaded. Sound for a parse result
// that is never `Rc`-aliased across threads. (Would be UNSOUND if a clone of the inner `TokenStream`
// were retained on the producing thread; we never clone before the move.)
unsafe impl Send for SendFile {}

/// A parse worker's output for one file: the (Send-wrapped) parsed file plus its per-fn `file:line:col`s
/// resolved on the worker (walk order; see `fn_locs`). Bundled so loc rides alongside the moved file.
pub(crate) type ParsedFile = (SendFile, Vec<String>);
