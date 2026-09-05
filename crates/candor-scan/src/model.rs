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
    /// ⟨0.29⟩ the string literal at ARGUMENT POSITION 0 — the host, command, path or query. It was "the
    /// first literal found anywhere in the argument list", which read `fs::write(user_path, "/tmp/lit")`
    /// as writing to `/tmp/lit` when that string is the DATA, so `allow Fs /tmp/lit` certified a write to
    /// a runtime-controlled destination at exit 0.
    pub(crate) str_arg: Option<String>,
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
    /// ⟨0.29⟩ A two-path `Fs` call (`copy`/`rename`/`hard_link`/`symlink*`) whose SECOND path argument is
    /// not a literal. Position 0 alone is not the surface for these: `fs::copy("/safe", user_path)` reads
    /// fully determined while writing somewhere nobody can see, so the marker travels with the call and
    /// the scan treats the surface as incomplete.
    #[serde(rename = "pp", default, skip_serializing_if = "std::ops::Not::not")]
    pub(crate) path_lits_partial: bool,
    /// ⟨0.29⟩ The literal at the SECOND path position of a two-path Fs op (`copy`/`rename`/`symlink*`).
    /// `str_arg` carries position 0; both are destinations the gate must see. Publishing only the first
    /// while calling the surface COMPLETE certified the second: `copy("/tmp/lit", "/tmp/dst")` under
    /// `allow Fs /tmp/lit` exited 0 while writing `/tmp/dst`. candor-java and candor-swift publish both.
    #[serde(rename = "p2", default, skip_serializing_if = "Option::is_none")]
    pub(crate) path_lit2: Option<String>,
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
    /// ⟨typeSurface.returns⟩ The type a caller's BINDING holds for `let x = f()` — see
    /// `bound_return_type`. Distinct from `ret_idents` (the raw ident list, wrapper included) and from
    /// the crate-wide `ReturnIndex` (keyed by fn LEAF, and it UNWRAPS `Result`/`Option`, so it cannot
    /// answer "what does the binding hold"). It lives on FnInfo because the producer needs the
    /// MODULE-QUALIFIED fn qual beside it, which a leaf-keyed index does not have.
    #[serde(rename = "b", default, skip_serializing_if = "Option::is_none")]
    pub(crate) ret_bound_type: Option<String>,
    /// SOUNDNESS R182/R196 — the REFUSAL reasons this body hit: a place where name resolution found
    /// MORE THAN ONE answer, refused to guess (correctly), and then had nothing to say. Each entry is a
    /// SPEC §4 `kind:detail` reason string, which `scan.rs` turns into a DIRECT `Unknown` beside the
    /// reason. Distinct from `unresolved` (one bool, one reason — "a callable I could not see through"):
    /// these are refusals with a NAMED cause, and one body can hit several.
    ///
    /// THE RULE THEY INSTANCE: **a refusal must disclose, not certify.** Before this the refusal simply
    /// returned `None` and the enclosing function was reported PURE — the cardinal sin's signature,
    /// because an omitted pure function and an omitted effectful one are the same bytes. Every write
    /// here can only ADD `Unknown`; none of them withdraws an answer.
    #[serde(rename = "rf", default, skip_serializing_if = "Vec::is_empty")]
    pub(crate) refusals: Vec<String>,
    /// ⟨peek-scope-attribution⟩ `(trait_leaf, method_leaf)` pairs this fn dispatches on through a local
    /// bounded-CHA-eligible receiver — see `CallCollector::dispatch_sites` for the full rationale. NOT
    /// part of `candor_report::ReportEntry` (the public wire schema): an ordinary scan's published report
    /// is untouched by this field's existence. It exists so `scan.rs`'s out-of-scope block can test a
    /// policy's scope against every in-scope function that could REACH a peeked (excluded) declaration
    /// through dynamic dispatch, not only the excluded declaration's own name. Cached like any other
    /// Pass-B result (`cache_schema` rev bump on this field's addition).
    #[serde(rename = "ds", default, skip_serializing_if = "Vec::is_empty")]
    pub(crate) dispatch: Vec<(String, String)>,
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

/// `enum-variant-leaf -> the single payload's DISPATCH trait leaves` (R77), the trait-object counterpart
/// of `EnumVariantIndex` — `Cb -> ["Fn"]` from `enum Msg { Cb(Box<dyn Fn()>) }`. `type_path` returns
/// `None` for a `dyn`/`impl`/bounded-generic payload (no nominal path), so such a variant was ABSENT from
/// `EnumVariantIndex` entirely and a match-arm/if-let/while-let/let-else binding of it (`Msg::Cb(f) =>
/// f()`) typed `f` into NEITHER `vars` nor `trait_vars`/`fn_typed_vars` — a silent drop of the payload,
/// closures included (SOUNDNESS.md R77). Same ambiguity rule as `EnumVariantIndex`: a leaf two enums
/// share with different leaf sets is dropped (never guess).
pub(crate) type EnumVariantTraitIndex = HashMap<String, Vec<String>>;

/// R77 CROSS-INDEX AMBIGUITY GUARD. `EnumVariantIndex` and `EnumVariantTraitIndex` are both keyed by bare
/// variant LEAF, crate-wide, with no enum qualifier — the exact imprecision `EnumVariantIndex`'s own
/// same-index ambiguity rule already accepts (two enums, same leaf, different CONCRETE payloads ->
/// dropped, never guess). That rule never had to consider a leaf that is CONCRETE in one enum and
/// DISPATCH-typed in another, because a dispatch payload was invisible to any index before R77.
///
/// Measured on reqwest 0.13.4's real source in the 256-crate A/B: `enum Matcher_ { Custom(Custom) }` (a
/// concrete struct payload) and the unrelated `enum PolicyKind { Custom(Box<dyn Fn(..)->..>) }` (a
/// callable payload) share the leaf `Custom`. Without this guard, `Matcher_::Custom(ref c) =>
/// c.call(dst)` took the WRONG (dispatch) route from the unrelated enum, typed `c` as a bare `Fn` with no
/// local impl, and the call SILENTLY DROPPED — `intercept` read pure instead of inheriting the correctly
/// `Unknown` `Custom::call`, an under-report R77 itself introduced on a function nowhere near a closure.
///
/// A leaf present in BOTH indexes can't be told apart by leaf alone: drop it from both (never guess),
/// same as the existing same-index rule. This does not recover the accidental cross-enum precision the
/// pre-R77 code had here (dispatch payloads being invisible was never a designed guarantee).
///
/// R90 — SOUNDNESS, comment correction: this used to claim the drop "converts a WRONGLY-ROUTED result
/// into an HONEST unresolved-receiver one". **The first half was true and MEASURED; the second was never
/// measured and was false.** Dropping the leaf from BOTH indexes made `enum_variant_binding` /
/// `enum_struct_variant_bindings` return `(name, [], None)` — no dispatch leaves, no plain type — and
/// every consumer (match-arm/if-let/while-let/let-else) treats that exactly like a payload it could not
/// resolve at all: it binds nothing and visits the arm body with no type info, so a call on the payload
/// (`Custom::call` in the fixture above) drops SILENTLY — no `Unknown`, no disclosure, on BOTH arms whose
/// leaf collided, not just the one this doc used to describe. The collision-detection half was attacked
/// and HELD (measured on the reqwest 0.13.4 shape above, re-verified this round); only the "honest"
/// half was asserted, not measured, and cost the SOUNDNESS entry that quoted it.
///
/// The RETURNED set records exactly which leaves were dropped, so a caller can now do what the comment
/// always claimed: thread it to `CallCollector` (`ambiguous_enum_leaves`) and disclose `Unknown` at the
/// binder — the same `self.unresolved = true` an ambiguous LOCAL trait name or an unbounded dispatch fan-
/// out already uses (collector.rs's `dispatch_calls_for_trait_method`), not a new mechanism. Called from
/// every place that finalises these two indexes (the real scan path AND the test helpers that replicate
/// it), so a unit test actually exercises the same guard the CLI does.
#[must_use]
pub(crate) fn drop_cross_ambiguous_enum_leaves(
    enum_variants: &mut EnumVariantIndex,
    enum_variant_traits: &mut EnumVariantTraitIndex,
) -> HashSet<String> {
    let colliding: HashSet<String> =
        enum_variants.keys().filter(|k| enum_variant_traits.contains_key(*k)).cloned().collect();
    for leaf in &colliding {
        enum_variants.remove(leaf);
        enum_variant_traits.remove(leaf);
    }
    colliding
}

/// `fn-leaf -> expanded return-type-path`, e.g. `create_pool -> sqlx::Pool` (Result/Option unwrapped).
/// Lets type inference flow through a LOCAL factory function: `let p = create_pool()?; p.fetch_one(q)`.
/// Only UNAMBIGUOUS leaves are kept — a name with two different return types across the crate is dropped
/// (no guess), like the unique-leaf call-graph rule.
pub(crate) type ReturnIndex = HashMap<String, String>;

/// One `pub use` RE-EXPORT edge: the fact that a name defined in ANOTHER module is ALSO nameable through
/// `module`. Collected per file/inline-module by `collect_reexports`, merged crate-wide, and turned into
/// the alias index `reexport_aliases` that `reexport_target` consults.
///
/// WHY IT HAS TO EXIST SEPARATELY FROM THE `use` MAP. A file's `use` map answers "what does a name written
/// IN THIS FILE mean"; a re-export is the opposite direction — what a name written in a DIFFERENT file
/// means when it is qualified by this module. `collect_root_reexports` already covers that for the crate
/// ROOT; a re-export declared in a SUBMODULE (`mod imp { pub use self::platform::*; }`) had no such
/// channel at all, so a call `imp::doit()` — whose 2-segment tail (`imp::doit`) matches no definition's
/// tail (`platform::doit`) — resolved to nothing and every caller read silent-pure.
///
/// Private (`use …`, no `pub`) statements are NOT collected: they bind a name for the module's own body
/// and export nothing, so treating one as a re-export would answer for a path that names no item.
#[derive(Clone, serde::Serialize, serde::Deserialize)]
pub(crate) struct Reexport {
    /// The module path the name becomes visible IN (`file::imp`), in the scanner's file-derived spelling.
    pub(crate) module: String,
    /// The module path(s) the name comes FROM (`file::imp::unix`), already resolved through any
    /// `#[path]` / `#[cfg_attr(.., path = "..")]` redirect on the `mod` declaration. SEVERAL when the
    /// redirect is cfg-conditional (`unix.rs` / `windows.rs` / `other.rs`): the scanner analyses every
    /// `#[cfg]` branch, so the re-export names every branch's definition and the caller charges their
    /// union — the same over-approximation `cfg_if` arms already get.
    pub(crate) from: Vec<String>,
    /// The name in the SOURCE module, or `*` for a glob (`pub use self::platform::*`).
    pub(crate) name: String,
    /// The name it is visible AS in `module` — differs from `name` only for `pub use … as …`. `*` for a glob.
    pub(crate) alias: String,
    /// SOUNDNESS R176 — whether the `pub use` item carries a `#[cfg(..)]`. Explicit-import-beats-glob is
    /// Rust's rule WITHIN ONE CONFIGURATION; a `#[cfg(unix)] pub use unix::*` and a `#[cfg(windows)]
    /// pub use windows::size` never coexist in any build, so letting the second SHADOW the first drops
    /// the unix arm's definition behind the windows arm's positive claim. The scanner analyses every
    /// `#[cfg]` branch — that is the whole reason `from` is a `Vec` — so the arms must UNION here too.
    /// Defaults to `false` so a cache entry written before this field reads as "not gated", which is the
    /// shadowing (narrower) answer; the cache rev is bumped for exactly that reason.
    #[serde(default)]
    pub(crate) cfg_gated: bool,
}

/// Sentinel return-"type" for a fn whose return is a CALLABLE (`-> fn()`/`-> impl Fn`/`-> Box<dyn Fn>`).
/// Stored in the return index under the fn's leaf; `expr_is_fn_typed` reads it so `let g = make_cb()`
/// propagates fn-typed-ness, while `ctor_type` filters it out of var-typing (it's not a nominal type).
/// The angle brackets cannot collide with a real Rust type path.
pub(crate) const RET_FN_TYPED: &str = "<fn>";

/// SOUNDNESS R174(b) — sentinel return-"type" for a fn that returns NOTHING. It exists only to make a
/// unit-returning twin a CONFLICT: `record_return` used to bail on `ReturnType::Default` before
/// recording anything, so `fn init()` beside `fn init() -> Repository` left the leaf `init`
/// unambiguously typed `Repository` and every `crate::init()` call read as constructing one (git2: 76
/// functions per version given a phantom `Repository::drop` edge). Never a nominal type —
/// `recorded_return_type` filters it out exactly as it filters `RET_FN_TYPED`, so a leaf that reaches
/// it makes no claim at all rather than the wrong one.
///
/// SOUNDNESS R188 — IT IS ALSO THE KEY PREFIX, because the conflict belongs to ONE consumer. R174(b)
/// wrote the sentinel under the fn's own leaf, so the twin collapsed that leaf to "ambiguous" for
/// EVERY reader of the index — including `let`-typing, which is not a resolution guess at all: with
/// `net::connect(addr) -> Conn` beside an unrelated unit `ui::Ui::connect(&mut self)`, `let c =
/// net::connect(addr); c.send(b)` lost `c`'s type and went `['Net']` → ABSENT, silently, against a
/// module-qualified (unambiguous) call. `()` has no INHERENT methods, so a body that resolves
/// `c.send(b)` through this index could not have called the unit twin — which is why the conflict is
/// now recorded under `<unit><leaf>` and read by the DROP route alone (`ctor_leaf_from_call_returns`),
/// where the phantom `Repository::drop` edge actually came from. The angle brackets keep the key out of
/// the identifier space the index is otherwise keyed on.
///
/// STATED LIMIT, not a guarantee: a blanket TRAIT method IS callable on `()` (`let c = ui::mk();
/// c.clone()`), and there this index still answers with the typed twin's type. That is published
/// 0.34.0's answer and it is the over-charging direction — an edge too many, never a silence — so it
/// is left where it was rather than narrowed on the strength of this paragraph.
pub(crate) const RET_UNIT: &str = "<unit>";

/// R188 — the `rets` key under which `record_return` files "some fn with this leaf returns `()`".
/// One place, so the writer (`record_return`) and the reader (`ctor_leaf_from_call_returns`) cannot
/// spell it differently — the corpus brief's "a key two paths can spell differently" shape (§F1.7).
pub(crate) fn unit_twin_key(leaf: &str) -> String {
    format!("{RET_UNIT}{leaf}")
}

/// SOUNDNESS R182 — sentinel prefix for a CANDIDATE return type of a fn leaf the ambiguity rule
/// WITHDREW. `rets` drops a leaf recorded with two conflicting return types (right: never guess), and
/// the drop route then read that absence as "this call constructs nothing" and certified the caller
/// pure. To disclose instead, the site has to tell "this leaf named no type" apart from "this leaf's
/// type was thrown away because it collided" — and, so the disclosure does not flood, WHICH types
/// collided, because the refusal costs nothing unless one of them was drop-relevant.
///
/// Recorded in the same key-space trick `unit_twin_key` uses: every reader of `ReturnIndex` keys on a
/// plain identifier (or on `unit_twin_key`), so none of them can address one of these. THE ONE READER
/// THAT DOES NOT KEY is `decls::fninfo`'s `has_dyn_return`, which scans the index's VALUES — and the
/// value here is the constant `RET_AMB`, distinct from all three dyn prefixes; pinned by
/// `the_amb_sentinel_is_not_readable_as_any_other_return_shape`, because an assertion written by the
/// commit that needs it to be true is the one nobody checks. The candidate TYPE
/// LEAF rides in the KEY and the value is the constant prefix, which is what makes the entry
/// conflict-free under `merge_amb`'s "two values for one key ⇒ ambiguous" rule — one key can only ever
/// carry one value. Leaves, not paths, for the reason `alias_expand_decls` states about `prim_aliases`
/// and `drop_types`: a bare leaf is not something a module-alias rewrite can say anything about, so the
/// entry cannot go stale when that rewrite runs.
///
/// Written ONLY at a detected conflict (in `record_return` for an intra-file collision, in `merge_decls`
/// for a cross-file one), so an unambiguous crate adds no entries at all.
pub(crate) const RET_AMB: &str = "<amb>";

/// The `rets` key under which a WITHDRAWN candidate return type is filed. One place, so the two writers
/// (`record_return`, `merge_decls`) and the one reader (`ambiguous_return_leaves`) cannot spell it
/// differently — §F1.7's shape, the same argument as `unit_twin_key`.
pub(crate) fn amb_ret_key(fn_leaf: &str, type_leaf: &str) -> String {
    format!("{RET_AMB}{fn_leaf}\u{1f}{type_leaf}")
}

/// The inverse of `amb_ret_key`: `(fn leaf, candidate type leaf)` for a `rets` key that is one, `None`
/// for every ordinary entry.
pub(crate) fn split_amb_ret_key(key: &str) -> Option<(&str, &str)> {
    key.strip_prefix(RET_AMB)?.split_once('\u{1f}')
}

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
    pub(crate) enum_variant_traits: &'a EnumVariantTraitIndex,
    /// R90 — the leaves `drop_cross_ambiguous_enum_leaves` removed from BOTH indexes above because two
    /// unrelated enums share a variant name with different (concrete vs. dispatch) payloads. Threaded
    /// through so the binder sites that consult `enum_variants`/`enum_variant_traits` can tell "this
    /// payload is genuinely untyped" apart from "this payload's type was thrown away because it collided"
    /// — only the second case discloses `Unknown`; see collector.rs's `enum_variant_binding`.
    pub(crate) ambiguous_enum_leaves: &'a HashSet<String>,
    /// R101 — the names of `static`/`const` items whose declared type holds an INVOKABLE callback inside
    /// a container/cell (`static CB: OnceLock<Box<dyn Fn()>>`, `static H: Mutex<Option<Box<dyn Fn()>>>`).
    /// The module-level counterpart of `field_elem_trait`: a static has no binding site to type it at, so
    /// before this index `resolve_elem_trait_leaves` returned NOTHING for a static receiver and every
    /// unwrap binder over one silently dropped the call as pure (SOUNDNESS R101, kernel-witnessed).
    /// Carries only the synthetic `"Fn"` leaf, so it cannot CONTRIBUTE a concrete effect. It can WITHDRAW
    /// one if its consuming arm is reached for a name that is a dispatch-typed LOCAL here — see
    /// `lang::static_holds_callable`, which states that condition and the guard-deletion measurement.
    pub(crate) callable_statics: &'a HashSet<String>,
    /// SOUNDNESS R161 — the LEAF names of crate-wide `type NAME = <callable>` aliases (`pub type
    /// AutoExtension = fn(Connection) -> Result<()>`, `type Cb = Box<dyn Fn()>`). A nominal alias is a
    /// `Type::Path` like any other, so `is_callable_type` answered FALSE for it in every position at
    /// once — parameter, `let` annotation, closure param — and a fn whose only call was through such a
    /// parameter vanished from `functions[]`. Carries names only; like `callable_statics` it can name no
    /// concrete effect, only turn a silent drop into `Unknown`.
    pub(crate) callable_aliases: &'a HashSet<String>,
    /// SOUNDNESS R182 — fn LEAF -> the DROP-RELEVANT candidate return types the ambiguity rule withdrew
    /// from `ReturnIndex`. Built in `scan.rs` from the `amb_ret_key` entries, intersected with
    /// `drop_relevant`, so the map is EMPTY for a crate whose collisions could never have been charged
    /// anything. That intersection is what keeps the disclosure off the flood: a leaf like `new`/`parse`
    /// is ambiguous in almost every crate, and hedging on ambiguity ALONE would charge `Unknown` at
    /// every free call of such a name. Entries only exist where the refusal really did withdraw a drop.
    pub(crate) ambiguous_return_leaves: &'a HashMap<String, Vec<String>>,
    /// SOUNDNESS R208 — the `macro_rules!` names this crate defines more than once with DIFFERENT arm
    /// tokens; see `cache::FileDecls::macro_twins`. Carries names only: it can name no concrete effect,
    /// only turn an order-dependent silence into `Unknown`.
    pub(crate) macro_twins: &'a HashSet<String>,
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
///
/// THE OTHER HALF OF THE CONTRACT, and it is not about `Send` at all: the spans inside a moved AST name
/// files the WALKING thread's source map does not have. It is not enough that candor resolves `loc` on
/// the parse worker — **syn's own parser reads spans too**, so handing a moved token stream back to
/// `syn::parse2` reads them on the wrong thread. See `respan_call_site`, which every such re-parse must
/// go through.
pub(crate) struct SendFile(pub(crate) syn::File);

// SAFETY: see the type doc — uniquely owned, moved once, then single-threaded. Sound for a parse result
// that is never `Rc`-aliased across threads. (Would be UNSOUND if a clone of the inner `TokenStream`
// were retained on the producing thread; we never clone before the move.)
unsafe impl Send for SendFile {}

/// A parse worker's output for one file: the (Send-wrapped) parsed file plus its per-fn `file:line:col`s
/// resolved on the worker (walk order; see `fn_locs`). Bundled so loc rides alongside the moved file.
pub(crate) type ParsedFile = (SendFile, Vec<String>);

/// Re-stamp every token of a MOVED token stream with `Span::call_site()`, so it can be handed back to
/// `syn` on a thread that did not parse it.
///
/// WHY THIS IS NEEDED. proc-macro2's fallback `Span` is a pair of byte offsets into a THREAD-LOCAL
/// source map (`SendFile` explains why we enable `span-locations`). candor parses files on rayon
/// workers and walks them on the collector thread, so every span in a moved AST is an index into a map
/// that thread does not have. candor never reads such a span itself — `loc` is resolved in the parse
/// closure — but candor DOES hand macro bodies straight back to syn (`visit_macro`,
/// `lazy_static_macro_body`, `thread_local_macro_body`), and **syn's parser reads spans**:
/// `syn::lit::parsing::parse_negative_lit` JOINs the `-` punct's span with the literal's, and
/// `Span::join` looks the receiver up in the map.
///
/// Both outcomes of that lookup are wrong, and the quiet one is worse:
///   - past the end of the walking thread's map → `unreachable!("Invalid span with no related
///     FileInfo!")`, i.e. the parser aborts. `getrandom` 0.3.4 / 0.4.2 spell exactly this,
///     `debug_assert!({ match ret { 0 => true, -1 => …, _ => false } })`, and took the whole scan down.
///   - INSIDE that map's range → it silently resolves against an unrelated file. No panic, no signal.
///
/// Which one you get depends on how much the two threads happened to have parsed, which is why the
/// crash looked data-dependent and would not reproduce on the file in isolation.
///
/// `Span::call_site()` is `(0, 0)`, the dummy file proc-macro2 seeds EVERY thread's map with, so it
/// resolves everywhere and joins to itself. Nothing is lost by dropping to it: the spans of a macro's
/// INTERIOR tokens are never read by candor — only syn's own error paths use them, and every one of
/// these parses is `.ok()`-discarded on failure.
pub(crate) fn respan_call_site(ts: proc_macro2::TokenStream) -> proc_macro2::TokenStream {
    use proc_macro2::{Group, Span, TokenTree};
    ts.into_iter()
        .map(|tt| match tt {
            // A group carries its own delimiter span AND its contents' — recurse, and let `Group::new`
            // mint the fresh call-site delimiter span.
            TokenTree::Group(g) => TokenTree::Group(Group::new(g.delimiter(), respan_call_site(g.stream()))),
            TokenTree::Ident(mut t) => {
                t.set_span(Span::call_site());
                TokenTree::Ident(t)
            }
            TokenTree::Punct(mut t) => {
                t.set_span(Span::call_site());
                TokenTree::Punct(t)
            }
            TokenTree::Literal(mut t) => {
                t.set_span(Span::call_site());
                TokenTree::Literal(t)
            }
        })
        .collect()
}
