//! The `--incremental` scan cache. CORRECTNESS IS THE WHOLE JOB: an incremental scan
//! must be byte-identical to a from-scratch scan (see the invalidation model below).

use crate::*;

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
    pub(crate) static INCREMENTAL: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// The cache-format identity. Bump the trailing rev whenever the cached representation OR any analysis
/// that feeds it changes; the embedded scanner version + include-tests flag make a binary upgrade or a
/// scope change invalidate every entry automatically. A mismatch on read = full re-derivation.
pub(crate) fn cache_schema(include_tests: bool) -> String {
    // rev18: `Reexport` gained `cfg_gated` (R176 — whether a `pub use` is one arm of a `#[cfg]` split).
    // A pre-rev18 entry has no such field, so `#[serde(default)]` reads FALSE — "this re-export is
    // unconditional" — which is precisely the input that makes explicit-beats-glob silence the other
    // platform's arm. Same shape as every bump below.
    //
    // AND THE STRING SAID `rev16` WHILE THE LADDER BELOW DOCUMENTED `rev17`. The R161 bump was written
    // in the comment and never applied to the format string, so a rev16 entry and a "rev17" entry share
    // one key. `CARGO_PKG_VERSION` is also in the key, so entries only ever collided BETWEEN TWO BUILDS
    // OF THE SAME VERSION — which is exactly what an A/B of a fix against its own pre-image is. Fixed by
    // moving straight to rev18 rather than reusing rev17, so no key is ever served two meanings.
    // rev17: FileDecls gained `callable_aliases` (R161 — the `type NAME = <callable>` alias leaves). A
    // rev16 entry has none, so it deserializes EMPTY: "this file declares no callable type alias", for a
    // file that does — and the consequence is exactly the silent under-report the field closes, served
    // warm and invisible. Same shape as rev15, rev14, rev13, rev12 and rev9.
    // rev16: R123 changed what an EXISTING field RECORDS — `uses` and `root_reexports` no longer collect
    // a `#[cfg(test)]`-gated `use`. A rev15 entry was written by a binary that DID collect them, so a warm
    // cache serves the mock-resolved answer for the very file the fix is about: `run` ABSENT over a call
    // that really spawns a process. Same shape as rev13 — a field addition is not the only thing that
    // needs a bump, a change in what a field records does too.
    // rev15: FileDecls gained `macro_modules` (R128 — the module quals whose item list carries an
    // unexpanded item-position macro invocation). A rev14 entry has none, so it deserializes EMPTY:
    // "every module in this file was read in full", for files where a `cfg_rt!`/`include!`/`foo!()`
    // hid a `pub fn` or a `pub(crate) use`. That is exactly the silent under-report the field closes,
    // served warm and invisible — the same shape as rev14, rev13, rev12 and rev9.
    // rev14: FileDecls gained `callable_statics` (R101 — the `static`/`const` items holding an invokable
    // callback). A rev13 entry has none, so it deserializes EMPTY: "this file declares no externally
    // installable callback slot", for a file that does — and the consequence is precisely the silent
    // under-report the field closes, served warm and invisible. Same shape as rev12 and rev9, and the
    // reason a `#[serde(default)]` field addition is never cache-neutral in this scanner.
    // rev13: `mod_aliases` gained a NEW KIND OF ENTRY — a submodule's external GLOB re-export, keyed
    // `<module>::*glob` (R99 shape 1). The FIELD is unchanged, so serde reads a rev12 entry without
    // complaint and simply yields a map with no glob in it: "this module re-exports nothing by glob",
    // for a module that does. That is the same warm-cache-served silent under-report rev12 was bumped
    // for, and a field addition is not the only thing that causes it — a change in what an EXISTING
    // field records does too. (The Pass A re-expansion added in the same change needs no bump: the
    // cached `FileDecls` are the PRE-expansion ones by construction, and the expansion is redone from
    // the merged index on every run.)
    // rev12: FileDecls gained `mod_aliases` (R99 — the module-qualified external alias map). A rev11
    // entry has none, so it deserializes EMPTY: "this file gives no std/dependency item a second
    // spelling", which is exactly the silent under-report the field closes, served warm and invisible.
    // rev10: FnInfo gained `dispatch` (⟨peek-scope-attribution⟩ — the peek out-of-scope scope-matching
    // fix). A rev9 entry deserializes it as an empty Vec, i.e. "this fn dispatches on no local trait" —
    // which for a warm-cached fn that genuinely does dispatch is exactly the silent under-report this
    // field exists to close: a peeked finding reachable ONLY through that fn's dispatch would stay
    // invisible to scope-matching on an incremental re-scan even though a from-scratch scan catches it.
    // rev9: FileDecls gained `reexports` (the SUBMODULE-level `pub use` edges). A rev8 entry has none, so
    // it deserializes as an EMPTY vec — i.e. "this file re-exports nothing", which is precisely the
    // silent under-report the field exists to close, served from a warm cache and invisible.
    // rev8: FileCache gained `aborted` (the contained-parser-abort disclosure). A rev7 entry has no
    // such field, so it deserializes as None — i.e. "this file was analysed and has no functions",
    // which for an entry written by the aborting run is exactly the false all-clear rev8 exists to
    // stop. Discard those wholesale rather than trust the default.
    // rev7: FnInfo gained `ret_bound_type` (⟨typeSurface.returns⟩). A rev6 entry deserializes it as
    // None, which would silently publish an EMPTY type surface off a warm cache.
    format!("scan-{}/rev18/tests={}", env!("CARGO_PKG_VERSION"), include_tests)
}

/// A stable 64-bit FNV-1a content hash, hex — no extra dependency, deterministic across runs and hosts
/// (unlike `DefaultHasher`, which is randomized). Used for both file content and the canonical merged
/// decl-index digest, so the cache key never depends on process-random state.
pub(crate) fn fnv1a(bytes: &[u8]) -> String {
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
pub(crate) struct FileDecls {
    pub(crate) fields: FieldIndex,
    pub(crate) field_elem: FieldElemIndex,
    /// `Type -> { field -> element dispatch leaves }` for a COLLECTION-OF-TRAIT-OBJECTS field (R37 field form).
    #[serde(default)]
    pub(crate) field_elem_trait: FieldElemTraitIndex,
    /// `leaf -> Some(ty)` or `None` (this file alone already saw conflicting return types for the leaf).
    pub(crate) rets: HashMap<String, Option<String>>,
    pub(crate) enum_tmp: HashMap<String, Option<String>>,
    /// `variant leaf -> Some(dispatch trait leaves)` or `None` (ambiguous) for a single-field DISPATCH-typed
    /// tuple-variant payload (R77) — the `enum_tmp` counterpart for a `dyn`/`impl`/bounded-generic payload,
    /// which `type_path` can't name so `enum_tmp` never records it at all.
    #[serde(default)]
    pub(crate) enum_variant_traits: HashMap<String, Option<Vec<String>>>,
    pub(crate) trait_impls: TraitImplIndex,
    /// `trait leaf -> (decl count in this file, declared method names)` — `LocalTrait` flattened for serde.
    pub(crate) trait_decls: HashMap<String, (usize, Vec<String>, Vec<String>)>,
    pub(crate) trait_fields: TraitFieldIndex,
    /// names aliased to a non-nominal type (`type Inner = [u8; N]`) — resolution skips local `Inner::assoc`.
    pub(crate) prim_aliases: Vec<String>,
    /// fn names declared in an `extern` block — a call to one is an FFI boundary → DISCLOSE Unknown.
    pub(crate) extern_fns: Vec<String>,
    /// local type leaves with a local `impl Drop` — a fn binding such a value inherits the drop body.
    pub(crate) drop_types: Vec<String>,
    /// local type leaf -> Deref Target leaf (`impl Deref for T { type Target = U }`) — `t.method()`
    /// auto-derefs to `U::method` when T declares no `method`.
    #[serde(default)]
    pub(crate) deref_target: HashMap<String, String>,
    /// LAZY/deferred static NAMES in this file (`Lazy`/`LazyLock`/`LazyCell`, `lazy_static!`,
    /// `thread_local!`) — a fn naming one of these FORCES its deferred init unit (`<lazy>::NAME`).
    #[serde(default)]
    pub(crate) lazy_statics: Vec<String>,
    /// R101 — `static`/`const` NAMES in this file whose declared type holds an INVOKABLE callback inside a
    /// container/cell (`static CB: OnceLock<Box<dyn Fn()>>`). Unwrapping one binds a name whose `f()` calls
    /// a body this scan cannot see, so the binder must hedge `Unknown` instead of dropping it silently.
    #[serde(default)]
    pub(crate) callable_statics: Vec<String>,
    /// R161 — type-alias NAMES in this file whose RHS is an INVOKABLE callback (`type AutoExtension =
    /// fn(Connection) -> Result<()>`, `type Cb = Box<dyn Fn()>`). A parameter/annotation of such a name
    /// is a callback boundary, and nothing resolved the alias, so it read silently pure.
    #[serde(default)]
    pub(crate) callable_aliases: Vec<String>,
    /// CONST/STATIC string NAMES → their LITERAL value in this file (`const API_BASE: &str = "…"`) — a
    /// call building its host from one (`post(format!("{}/x", API_BASE))`) resolves the host here (SPEC §1
    /// static-host propagation). Literal-valued only.
    #[serde(default)]
    pub(crate) const_strings: HashMap<String, String>,
    /// LOCAL `macro_rules!` NAME → the arm TOKENS (as a string). A bare `NAME!(..)` invocation inline-expands
    /// the template so an effectful macro body isn't silent-pure (R48). String-valued (re-parsed at use).
    #[serde(default)]
    pub(crate) local_macros: HashMap<String, String>,
    /// BLANKET-impl method leaf -> the blanket self-param name (`ext` -> `T` for `impl<T> Ext for T`); "" if
    /// ambiguous. Lets an unresolved `x.ext()` edge to the blanket body `T::ext` (R45).
    #[serde(default)]
    pub(crate) blanket_methods: HashMap<String, String>,
    /// The crate-ROOT re-exports, populated ONLY for the root file (`lib.rs`/`main.rs`, module path "").
    /// `name -> resolved path` plus the `GLOB_KEY` sentinel for a `pub use x::prelude::*`. Seeded into every
    /// file's `use` map under `crate::<name>` so a submodule's `use crate::net` / `crate::net::foo` resolves
    /// through the crate-root re-export it can't otherwise see (files are scanned with per-file `use` maps).
    /// See `collect_root_reexports`. Empty for a non-root file.
    #[serde(default)]
    pub(crate) root_reexports: HashMap<String, String>,
    /// This file's SUBMODULE-level `pub use` RE-EXPORT edges (see `Reexport` / `collect_reexports`) —
    /// every module in the file, not just its top level. The crate-wide union feeds the alias index a
    /// qualified call falls back to when its 2-segment tail names no definition.
    #[serde(default)]
    pub(crate) reexports: Vec<Reexport>,
    /// R99 — MODULE-QUALIFIED name → EXTERNAL/aliased path, for the three shapes that give a std or
    /// dependency item a second crate-local spelling: a SUBMODULE `pub use` of an external item, a
    /// NOMINAL `type` alias, and a callable-typed `const`/`static` bound to a fn item. Keyed by the path a
    /// caller elsewhere writes (`facade::Command`; bare `Cmd` at the crate root) and seeded into every
    /// file's `use` map by `seed_mod_aliases`, so `expand` — the one resolution authority — answers for
    /// them. See `collect_reexports`. A rev11 entry has none and deserializes EMPTY, which is exactly the
    /// silent under-report this field closes — hence the rev bump in `cache_schema`.
    #[serde(default)]
    pub(crate) mod_aliases: HashMap<String, String>,
    /// R128 — MODULE QUALS in this file whose item list carries an unexpanded item-position MACRO
    /// INVOCATION (`foo!();`, `cfg_rt! { .. }`, `include!("gen.rs")`). Those items are skipped by
    /// `collect_decls`, so the module's decl/re-export entries are INCOMPLETE and a call naming a name
    /// in one must HEDGE rather than read the absence as purity. See `collect_macro_modules`. A cache
    /// entry written before this field deserializes EMPTY — which is precisely the silent under-report
    /// the field closes, hence the rev bump in `cache_schema`.
    #[serde(default)]
    pub(crate) macro_modules: Vec<String>,
}

/// Collect ONE file's Pass A decls in isolation (the per-file input to `merge_decls`). `modpath` is the
/// file's module path — the ROOT file (`""`) additionally contributes its crate-root re-exports, a
/// crate-wide fact seeded into every file's `use` map (see `root_reexports`).
pub(crate) fn file_decls(items: &[syn::Item], include_tests: bool, rel: &Path) -> FileDecls {
    let modpath = module_path(rel);
    let modpath = modpath.as_str();
    // `#[path = "…"]` on a `mod` resolves against the DIRECTORY of the file that declares it.
    let dir = rel.parent().unwrap_or(Path::new("")).to_path_buf();
    let mut uses = HashMap::new();
    let mut fields = HashMap::new();
    let mut field_elem = HashMap::new();
    let mut field_elem_trait = HashMap::new();
    let mut rets = HashMap::new();
    let mut enum_tmp = HashMap::new();
    let mut enum_variant_traits = HashMap::new();
    let mut trait_impls = HashMap::new();
    let mut trait_decls: HashMap<String, LocalTrait> = HashMap::new();
    let mut trait_fields = HashMap::new();
    let mut prim_aliases = std::collections::HashSet::new();
    let mut extern_fns = std::collections::HashSet::new();
    let mut drop_types = std::collections::HashSet::new();
    let mut deref_target = HashMap::new();
    let mut lazy_statics = std::collections::HashSet::new();
    let mut const_strings = HashMap::new();
    let mut local_macros = HashMap::new();
    let mut blanket_methods = HashMap::new();
    let mut callable_statics = std::collections::HashSet::new();
    let mut callable_aliases = std::collections::HashSet::new();
    // R177 — this file's `type NAME = <callable>` aliases BEFORE the walk that consumes them. See
    // `seed_callable_aliases` for why it is a separate pass and why its boundary is one file.
    crate::decls::seed_callable_aliases(items, include_tests, &mut callable_aliases);
    collect_decls(items, include_tests, &mut uses, &mut fields, &mut field_elem, &mut field_elem_trait, &mut rets,
                  &mut enum_tmp, &mut enum_variant_traits, &mut trait_impls, &mut trait_decls, &mut trait_fields, &mut prim_aliases,
                  &mut extern_fns, &mut drop_types, &mut deref_target, &mut lazy_statics, &mut const_strings, &mut local_macros, &mut blanket_methods, &mut callable_statics, &mut callable_aliases);
    // ONE walk produces both re-export channels — the intra-crate edges (`reexports`) and the
    // external/alias map (`mod_aliases`, R99), which is collected at the very branch that used to DROP
    // an external `pub use`. `uses` is this file's top-level `use` map, as `collect_decls` left it, so a
    // `type Cmd = Command;` written after `use std::process::Command;` expands to the std path.
    let mut reexports = Vec::new();
    let mut mod_aliases = HashMap::new();
    collect_reexports(items, modpath, &dir, include_tests, &uses, &mut reexports, &mut mod_aliases);
    FileDecls {
        fields,
        field_elem,
        field_elem_trait,
        rets,
        enum_tmp,
        enum_variant_traits,
        trait_impls,
        trait_decls: trait_decls
            .into_iter()
            .map(|(k, v)| (k, (v.count, v.methods.into_iter().collect(), v.supertraits)))
            .collect(),
        trait_fields,
        prim_aliases: prim_aliases.into_iter().collect(),
        extern_fns: extern_fns.into_iter().collect(),
        drop_types: drop_types.into_iter().collect(),
        deref_target,
        lazy_statics: lazy_statics.into_iter().collect(),
        callable_statics: callable_statics.into_iter().collect(),
        callable_aliases: callable_aliases.into_iter().collect(),
        const_strings,
        local_macros,
        blanket_methods,
        // Crate-root re-exports are a ROOT-file fact only; a submodule's `use crate::X` seeds against them.
        root_reexports: if modpath.is_empty() { collect_root_reexports(items, include_tests) } else { HashMap::new() },
        // …and the SUBMODULE-level re-exports, which every file can contribute (the crate root included:
        // a `pub use self::platform::*` at the root is BOTH a root re-export and a module-alias edge, and
        // the two answer different questions — see `Reexport`).
        reexports,
        mod_aliases,
        // R128 — which of this file's modules had items hidden behind an unexpanded macro invocation.
        // Keyed on the module QUAL (the file's own `modpath` for its top level), which is the same key
        // space a call's resolved `crate::…` path reduces to at the resolver.
        macro_modules: {
            let mut s = std::collections::HashSet::new();
            crate::lang::collect_macro_modules(items, modpath, include_tests, &mut s);
            let mut v: Vec<String> = s.into_iter().collect();
            v.sort(); // deterministic on the wire — the cache entry is content-hashed
            v
        },
    }
}

/// The assembled crate-wide decl index (Pass A output), ready for Pass B — exactly the seven structures
/// `scan_one` built inline before, now produced from per-file `FileDecls` so unchanged files contribute
/// from cache. `rets`/`enum_tmp` keep the `Option` ambiguity marker until the caller filters them.
#[derive(Default)]
pub(crate) struct MergedDecls {
    pub(crate) fields: FieldIndex,
    pub(crate) field_elem: FieldElemIndex,
    pub(crate) field_elem_trait: FieldElemTraitIndex,
    pub(crate) rets: HashMap<String, Option<String>>,
    pub(crate) enum_tmp: HashMap<String, Option<String>>,
    pub(crate) enum_variant_traits: HashMap<String, Option<Vec<String>>>,
    pub(crate) trait_impls: TraitImplIndex,
    pub(crate) trait_decls: HashMap<String, LocalTrait>,
    pub(crate) trait_fields: TraitFieldIndex,
    pub(crate) prim_aliases: std::collections::HashSet<String>,
    pub(crate) extern_fns: std::collections::HashSet<String>,
    pub(crate) drop_types: std::collections::HashSet<String>,
    pub(crate) deref_target: HashMap<String, String>,
    pub(crate) lazy_statics: std::collections::HashSet<String>,
    pub(crate) callable_statics: std::collections::HashSet<String>,
    pub(crate) callable_aliases: std::collections::HashSet<String>,
    pub(crate) const_strings: HashMap<String, String>,
    pub(crate) local_macros: HashMap<String, String>,
    pub(crate) blanket_methods: HashMap<String, String>,
    /// The crate-ROOT re-exports (`name -> path`, plus the `GLOB_KEY` sentinel), contributed by the root
    /// file. Seeded — under `crate::<name>` keys — into every file's `use` map at Pass B so a `use crate::X`
    /// / `crate::X::foo` in ANY file resolves through the crate-root re-export (`root_reexports`).
    pub(crate) root_reexports: HashMap<String, String>,
    /// Every file's SUBMODULE-level `pub use` re-export edges, concatenated in file walk order. Turned
    /// into the tail2-keyed alias index by `reexport_aliases`.
    pub(crate) reexports: Vec<Reexport>,
    /// Every file's MODULE-QUALIFIED external aliases, unioned (R99 — see `FileDecls::mod_aliases`). A key
    /// carries its module path, so two files collide only where they declare the SAME name in the SAME
    /// module, which no crate that compiles does.
    pub(crate) mod_aliases: HashMap<String, String>,
    /// R128 — every file's MACRO-HIDDEN module quals, unioned. A module in this set had item-position
    /// macro invocations its decl walk could not read, so "this module declares no such name" is a
    /// statement candor is NOT entitled to make about it. See `collect_macro_modules`.
    pub(crate) macro_modules: std::collections::HashSet<String>,
}

/// R99 (SHAPE 2) — re-expand ONE file's recorded TYPE PATHS against the crate-wide module-alias map.
///
/// THE DEFECT THIS CLOSES. Pass A runs per file, in isolation, so `type_path`/`record_return` expand a
/// written type against THAT FILE'S OWN `use` map and nothing else. `mod_aliases` is a crate-wide fact —
/// it does not exist until every file has been walked — so a type spelled through a module alias was
/// recorded UNRESOLVED and stayed that way for the whole run:
///
///     src/facade.rs   pub use std::process::Command;
///     src/main.rs     struct Holder { c: facade::Command }
///                     impl Holder { fn run_aliased(&mut self) { let _ = self.c.status(); } }
///
/// `fields["Holder"]["c"]` held the literal string `facade::Command`, `self.c` typed to a name with no
/// impl anywhere, and `run_aliased` was ABSENT from `functions[]` — while the paired control
/// (`c: std::process::Command`) reported `Exec`, and so did the same alias written as a LOCAL
/// (`let c: facade::Command`), because a local's type is decoded in Pass B where the seed is present.
/// Measured, EXECUTED, by the syscall oracle driver `pf_alias_field`.
///
/// WHY HERE AND NOT AT THE DECODE SITE. The obvious cheaper repair — re-expand the recorded string where
/// `CallCollector` reads it — resolves a path written in the DECLARING file against the CALLING file's
/// `use` map, so a bare local type `Widget` in `a.rs` would be rewritten by an unrelated
/// `use somecrate::Widget;` in `b.rs`. The declaring file's module path is the only context in which the
/// recorded path means anything, and this is the last place that still knows it.
///
/// It is the SAME authority, not a second one: `seed_mod_aliases` + `expand`, seeded with the DECLARING
/// file's `modpath`, exactly as Pass B seeds it for the file it is walking. Only the four index families
/// whose values are type PATHS are rewritten — `fields`, `field_elem`, `rets`, `enum_tmp`. The trait
/// indexes, `deref_target` and `prim_aliases` hold bare LEAVES rather than paths, so an alias map keyed
/// by qualified name cannot say anything about them.
///
/// Returns `None` when nothing moved, which is the case for every file in a crate with no module alias —
/// the caller then keeps the original and does not re-merge at all.
pub(crate) fn alias_expand_decls(
    fd: &FileDecls,
    modpath: &str,
    aliases: &HashMap<String, String>,
) -> Option<FileDecls> {
    let mut uses: HashMap<String, String> = HashMap::new();
    crate::lang::seed_mod_aliases(aliases, modpath, &mut uses);
    if uses.is_empty() {
        return None;
    }
    // A rewritten path, or None when `expand` leaves it alone. An ALIAS-ONLY `uses` map is deliberate:
    // a general `use` map here would re-apply an unrelated file's imports to this file's paths.
    //
    // THE SECOND ATTEMPT IS NOT BELT-AND-BRACES. Pass A already ran `expand` on the written type, and
    // `expand` STRIPS a `crate::`/`self::`/`super::` prefix — so a field written `crate::facade::Widget`
    // in a SIBLING module is recorded as `facade::Widget`, and the alias for it is seeded under
    // `crate::facade::Widget` (a sibling's relative spelling is not, and must not be, bound). Without the
    // retry the whole sibling-module spelling stays unresolved, which the over-charge control's second arm
    // caught: it asserts the qualified spelling still RESOLVES, precisely so the control cannot pass by
    // the mechanism being inert (brief §E3).
    //
    // THE BOUND, STATED RATHER THAN CLAIMED AWAY: the retry asserts the recorded path is crate-local, and
    // after Pass A's strip nothing distinguishes `crate::facade::Widget` from a bare `facade::Widget`
    // naming an EXTERN CRATE `facade`. Hijacking one would need a crate that has a root module `facade`
    // declaring `Widget`, an extern crate `facade` also exporting `Widget`, and a submodule writing the
    // bare spelling — and `qualified_alias`'s rooted lookup answers only for keys this crate declared, so
    // the wrong answer would be this crate's own `facade::Widget`. Nothing in the 256-crate corpus reaches
    // it, and it is an ATTRIBUTION between two same-named items rather than an invented effect.
    //
    // A MULTI-ARM RESULT IS REFUSED, AND THE A/B IS WHY. R105 keeps every `#[cfg]` arm of a duplicated
    // alias in one `\u{1}`-joined value and adjudicates at the CALL SITE, where the leaf is in hand. A
    // decl INDEX has no such site: a joined string lands in `fields` and every consumer — `tail2`,
    // `local_types`, the receiver-type chain — reads it as one path. Measured on the 256-crate corpus:
    // async-lock's `state: AtomicUsize` (through `mod sync { #[cfg(not(loom))] pub use core::sync::atomic;
    // #[cfg(loom)] pub use loom::sync::atomic; }`) and bytes' `Bytes::data_mut` went
    // `inferred: ["Unknown"], unresolved: true` -> `inferred: [], invisible: ["loom"]` on 5 rows — the
    // effect is genuinely absent either way (`with_mut` is a pure local trait method on the arm that
    // actually compiles), but `deny Unknown` flips 1 -> 0, which is a DISCLOSURE LOSS whatever the
    // underlying truth is. Refusing leaves those rows exactly as they were.
    let re = |p: &str| {
        let try_one = |q: &str| {
            let e = crate::lang::expand(q, &uses);
            (e != q && !e.contains(crate::decls::ALIAS_ALT_SEP)).then_some(e)
        };
        if let Some(e) = try_one(p) {
            return Some(e);
        }
        if p.contains("::") {
            if let Some(e) = try_one(&format!("crate::{p}")).filter(|e| e != p) {
                return Some(e);
            }
        }
        None
    };
    let moves_nested = |src: &HashMap<String, HashMap<String, String>>| {
        src.values().any(|m| m.values().any(|v| re(v).is_some()))
    };
    let moves_amb = |src: &HashMap<String, Option<String>>| {
        src.values().any(|v| v.as_deref().is_some_and(|t| re(t).is_some()))
    };
    if !moves_nested(&fd.fields)
        && !moves_nested(&fd.field_elem)
        && !moves_amb(&fd.rets)
        && !moves_amb(&fd.enum_tmp)
    {
        return None;
    }
    let mut out = fd.clone();
    for m in out.fields.values_mut().chain(out.field_elem.values_mut()) {
        for v in m.values_mut() {
            if let Some(e) = re(v) {
                *v = e;
            }
        }
    }
    for v in out.rets.values_mut().chain(out.enum_tmp.values_mut()) {
        if let Some(t) = v.as_deref().and_then(re) {
            *v = Some(t);
        }
    }
    Some(out)
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
pub(crate) fn merge_decls(acc: &mut MergedDecls, fd: &FileDecls) {
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
    for (s, fmap) in &fd.field_elem_trait {
        let e = acc.field_elem_trait.entry(s.clone()).or_default();
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
    // R77: the Vec-valued twin of `merge_amb` — same ambiguity rule (a leaf seen with two DIFFERENT leaf
    // sets, or already `None` in any contributor, collapses to `None`), for `enum_variant_traits`.
    let merge_amb_vec = |dst: &mut HashMap<String, Option<Vec<String>>>, src: &HashMap<String, Option<Vec<String>>>| {
        for (leaf, val) in src {
            match val {
                None => {
                    dst.insert(leaf.clone(), None);
                }
                Some(leaves) => match dst.get(leaf) {
                    None => {
                        dst.insert(leaf.clone(), Some(leaves.clone()));
                    }
                    Some(Some(prev)) if prev != leaves => {
                        dst.insert(leaf.clone(), None); // conflicting leaf sets — drop
                    }
                    Some(Some(_)) => {} // same leaves — keep
                    Some(None) => {}    // already ambiguous — stays
                },
            }
        }
    };
    merge_amb_vec(&mut acc.enum_variant_traits, &fd.enum_variant_traits);
    for (tr, tys) in &fd.trait_impls {
        acc.trait_impls.entry(tr.clone()).or_default().extend(tys.iter().cloned());
    }
    for (tr, (count, methods, supers)) in &fd.trait_decls {
        let e = acc.trait_decls.entry(tr.clone()).or_default();
        e.count += count;
        for m in methods {
            e.methods.insert(m.clone());
        }
        for s in supers {
            if !e.supertraits.contains(s) {
                e.supertraits.push(s.clone());
            }
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
    for n in &fd.macro_modules {
        acc.macro_modules.insert(n.clone()); // set union — order-independent (R128)
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
    for n in &fd.callable_statics {
        acc.callable_statics.insert(n.clone()); // set union — order-independent
    }
    for n in &fd.callable_aliases {
        acc.callable_aliases.insert(n.clone()); // set union — order-independent
    }
    for (k, v) in &fd.const_strings {
        acc.const_strings.insert(k.clone(), v.clone()); // leaf → literal; last-writer-wins on a rare collision
    }
    for (k, v) in &fd.local_macros {
        acc.local_macros.insert(k.clone(), v.clone()); // macro NAME → arm tokens; last-writer-wins on a rare collision
    }
    for (k, v) in &fd.blanket_methods {
        // blanket method leaf → self-param; a cross-file collision on DIFFERENT params is ambiguous ("").
        match acc.blanket_methods.get(k) {
            Some(prev) if prev != v || v.is_empty() => { acc.blanket_methods.insert(k.clone(), String::new()); }
            Some(_) => {}
            None => { acc.blanket_methods.insert(k.clone(), v.clone()); }
        }
    }
    for (k, v) in &fd.root_reexports {
        // Only the ROOT file populates this, so there is at most one contributor — a plain insert.
        acc.root_reexports.insert(k.clone(), v.clone());
    }
    // Re-export EDGES are facts about distinct modules — they never collide, so they concatenate. The
    // caller walks files in a fixed order and `reexport_aliases` folds them into sorted maps, so the
    // resulting index does not depend on this order.
    acc.reexports.extend(fd.reexports.iter().cloned());
    for (k, v) in &fd.mod_aliases {
        // R105 — this used to be a plain insert, carrying the assertion that "a cross-file collision means
        // two declarations of one name in one module — not a program that compiles". MEASURED FALSE, by
        // the same construct the intra-file half of R105 is about: `#[cfg(unix)] #[path = "unix.rs"] mod
        // imp;` beside `#[cfg(windows)] #[path = "windows.rs"] mod imp;` puts TWO files at ONE modpath,
        // this scanner walks both branches by design, and a `pub type Handle = …` in each collides here.
        // The winner was then whichever file the walk reached last. One rule for both halves: record every
        // arm, let the call site adjudicate. `record_alias` sorts, so the merged value does not depend on
        // walk order — which the decl-index digest below requires.
        record_alias(&mut acc.mod_aliases, k.clone(), v.clone());
    }
}

/// A CANONICAL, order-stable digest of the merged decl index — the gate that decides whether a cached
/// file's FnInfos (Pass B output) are still valid. Every map is rendered with SORTED keys (and sorted
/// inner keys / value lists) so the digest depends only on the index's CONTENT, never on `HashMap`
/// iteration order or which files happened to contribute. If this digest is unchanged, every fn's
/// `Call`s resolve identically, so a cached FnInfo set is sound to reuse; if it moves, all files re-run
/// Pass B. `trait_impls`'s Vec order is load-bearing (CHA), so it is hashed in order, NOT sorted.
///
/// EVERY FIELD OF `MergedDecls` MUST BE HASHED HERE, and that is pinned by
/// `every_merged_decl_field_moves_the_decl_index_digest` rather than by this sentence. `deref_target`
/// was missing and it did NOT show up as a wrong answer, because the auto-deref chase reads
/// `merged.deref_target` LIVE in `scan_one` rather than baking it into an FnInfo — so the omission was
/// harmless by an accident of WHERE the lookup happens, not by anything preventing it. Every other
/// receiver-typing rung of the last month landed in `CallCollector`; moving this one there too would
/// have turned a stale `impl Deref for W { type Target = … }` into a replayed purity claim with no
/// change to this file to hint at it. A digest that summarises the index must summarise all of it.
pub(crate) fn decl_index_digest(m: &MergedDecls) -> String {
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
    // field_elem_trait — nested map whose leaf is a Vec<String> (the element dispatch leaves).
    s.push_str("field_elem_trait");
    let mut fetk: Vec<&String> = m.field_elem_trait.keys().collect();
    fetk.sort();
    for k in fetk {
        s.push('|');
        s.push_str(k);
        let inner = &m.field_elem_trait[k];
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
    // enum_variant_traits — the Vec-valued twin of `enum_tmp` (R77): a DISPATCH-typed single-field
    // tuple-variant payload's trait leaves, joined so the digest moves if the leaf SET changes.
    s.push_str("enum_variant_traits");
    let mut evtk: Vec<&String> = m.enum_variant_traits.keys().collect();
    evtk.sort();
    for k in evtk {
        s.push('|');
        s.push_str(k);
        s.push('=');
        match &m.enum_variant_traits[k] {
            Some(leaves) => s.push_str(&leaves.join(",")),
            None => s.push_str("\u{0}AMBIG"),
        }
    }
    s.push('\n');
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
        let mut sup: Vec<&String> = lt.supertraits.iter().collect();
        sup.sort();
        for sname in sup {
            s.push('^');
            s.push_str(sname);
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
    // macro_modules — R128, sorted set of module quals whose items were hidden behind an unexpanded
    // macro invocation. It is read at the CALL RESOLVER (a crate-local call into one hedges Unknown
    // instead of vanishing), so a file gaining or losing an item-position macro must re-run Pass B for
    // every OTHER file too — exactly the property this digest exists to enforce.
    s.push_str("macro_modules");
    let mut mmk: Vec<&String> = m.macro_modules.iter().collect();
    mmk.sort();
    for a in mmk {
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
    // deref_target — sorted TYPE=Target pairs. A custom `impl Deref for W { type Target = U }` makes
    // `w.leaf()` dispatch to `U::leaf`, so a change here re-resolves every auto-deref call site.
    s.push_str("deref_target");
    let mut dttk: Vec<&String> = m.deref_target.keys().collect();
    dttk.sort();
    for k in dttk {
        s.push('|');
        s.push_str(k);
        s.push('=');
        s.push_str(&m.deref_target[k]);
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
    // callable_statics — sorted set of static/const names holding a callable. A change here changes which
    // unwrap binders hedge `Unknown`, so it must invalidate cached FnInfos exactly like `lazy_statics`.
    s.push_str("callable_statics");
    let mut cak: Vec<&String> = m.callable_statics.iter().collect();
    cak.sort();
    for a in cak {
        s.push('|');
        s.push_str(a);
    }
    s.push('\n');
    // callable_aliases — sorted set of `type NAME = <callable>` alias names (R161). A change here changes
    // which PARAMETERS/annotations are callback boundaries, so it must invalidate cached FnInfos exactly
    // like `callable_statics`.
    s.push_str("callable_aliases");
    let mut cak2: Vec<&String> = m.callable_aliases.iter().collect();
    cak2.sort();
    for a in cak2 {
        s.push('|');
        s.push_str(a);
    }
    s.push('\n');
    // const_strings — sorted NAME=literal pairs (a call resolving its host from one changes its captured
    // literal → its Net/Llm refinement), so a change here must invalidate cached FnInfos.
    s.push_str("const_strings");
    let mut csk: Vec<&String> = m.const_strings.keys().collect();
    csk.sort();
    for k in csk {
        s.push('|');
        s.push_str(k);
        s.push('=');
        s.push_str(&m.const_strings[k]);
    }
    s.push('\n');
    // local_macros — sorted NAME=arm-tokens pairs. A bare `NAME!(..)` inline-expands the template, so a
    // change to a macro body changes the effects of its invokers → must invalidate their cached FnInfos.
    s.push_str("local_macros");
    let mut lmk: Vec<&String> = m.local_macros.keys().collect();
    lmk.sort();
    for k in lmk {
        s.push('|');
        s.push_str(k);
        s.push('=');
        s.push_str(&m.local_macros[k]);
    }
    s.push('\n');
    // blanket_methods — a change to which method leaves resolve to a blanket body changes callers' effects.
    s.push_str("blanket_methods");
    let mut bmk: Vec<&String> = m.blanket_methods.keys().collect();
    bmk.sort();
    for k in bmk {
        s.push('|');
        s.push_str(k);
        s.push('=');
        s.push_str(&m.blanket_methods[k]);
    }
    s.push('\n');
    // root_reexports — sorted NAME=path pairs. Seeded into every file's `use` map, so a change re-resolves
    // `use crate::X` / `crate::X::foo` in OTHER files → must invalidate their cached FnInfos.
    s.push_str("root_reexports");
    let mut rrk: Vec<&String> = m.root_reexports.keys().collect();
    rrk.sort();
    for k in rrk {
        s.push('|');
        s.push_str(k);
        s.push('=');
        s.push_str(&m.root_reexports[k]);
    }
    s.push('\n');
    // reexports — the SUBMODULE-level `pub use` edges, SORTED (they are facts about distinct modules; the
    // walk order they arrive in carries no meaning). They steer which definition a qualified call in
    // ANOTHER file resolves to, which is the same reason `root_reexports` is here.
    //
    // Folded in even though they are consumed AFTER Pass B (`reexport_aliases` runs over the assembled
    // `fns`, a stage re-derived on every scan) — because "this field cannot reach a cached FnInfo" is
    // exactly the reasoning that left `deref_target` out of this digest for months. Over-invalidating
    // costs a re-derivation nobody notices; under-invalidating publishes a stale purity claim.
    s.push_str("reexports");
    let mut rx: Vec<String> = m
        .reexports
        .iter()
        .map(|r| format!("{}<-{}::{} as {}{}", r.module, r.from.join(","), r.name, r.alias,
                         if r.cfg_gated { " #[cfg]" } else { "" }))
        .collect();
    rx.sort();
    for r in rx {
        s.push('|');
        s.push_str(&r);
    }
    s.push('\n');
    // mod_aliases — sorted QUALIFIED-NAME=path pairs (R99). Seeded into EVERY file's `use` map, so a
    // change here re-resolves `facade::Command::new` / `Cmd::new` in OTHER files: same invalidation
    // obligation as `root_reexports`, for the same reason.
    s.push_str("mod_aliases");
    let mut mak: Vec<&String> = m.mod_aliases.keys().collect();
    mak.sort();
    for k in mak {
        s.push('|');
        s.push_str(k);
        s.push('=');
        s.push_str(&m.mod_aliases[k]);
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
pub(crate) fn nested_tf(s: &mut String, map: &TraitFieldIndex) {
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
///
/// `aborted` is the DISCLOSURE riding with the entry, and it exists because an empty `fninfos` is
/// ambiguous: a file with no functions and a file whose Pass B walk ABORTED both cache as `[]`, and the
/// second one is a hole. Without this field the warm run replayed the empty vector, said nothing, and a
/// configured gate went green over a file whose effects were never derived — a cached, reproducible
/// false all-clear (the cold run exits 2 on the same bytes). Replaying it is sound on exactly the
/// assumption the FnInfo reuse already makes: same content + same decl index ⇒ same walk, so same
/// outcome. `Some(reason)` reproduces the ⟨0.21⟩ `unanalyzed` entry verbatim.
#[derive(serde::Serialize, serde::Deserialize)]
pub(crate) struct FileCache {
    pub(crate) content_hash: String,
    pub(crate) decls: FileDecls,
    pub(crate) decl_index_hash: String,
    pub(crate) fninfos: Vec<FnInfo>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) aborted: Option<String>,
}

/// The whole on-disk cache: one file (`<crate>/.candor/cache/scan-cache.json`) holding the schema id and
/// every source file's entry keyed by crate-relative path. A SINGLE consolidated file means one read +
/// one atomic write per scan instead of one syscall per source file (the per-file-file design spent ~19ms
/// just opening + parsing tokio's 337 entries). A schema mismatch discards the whole cache.
#[derive(serde::Serialize, serde::Deserialize)]
pub(crate) struct ScanCache {
    pub(crate) schema: String,
    pub(crate) files: HashMap<String, FileCache>,
}
