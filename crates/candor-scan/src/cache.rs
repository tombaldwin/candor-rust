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
    // rev73: SOUNDNESS R1025 (receiver half) — Pass B's `returns` gains an alias's (TYPE, method) facts under
    // its target's leaf, so cached `calls` change. Mandatory.
    // rev72: SOUNDNESS R1004 (cross-file) — Pass A expands an invocation of a macro defined in ANOTHER file;
    // a rev71 entry lacks those units. Mandatory. (The table's digest also joins every file's key.)
    // rev71: SOUNDNESS R1025 — Pass A records GENERIC type aliases for path resolution (`aliases`, `uses`); a
    // rev70 entry lacks them and replays the silence warm. Mandatory. (rev70's R1023 half landed in a
    // later commit than its bump; rev71 also invalidates any rev70 entry written between the two.)
    // rev70: SOUNDNESS R1024 — Pass B seeds `self` in an impl for a std container/`Option`/`Result` with
    // its element (calls change); SOUNDNESS R1023 — element entries may be LAYERED (`Mutex<Option<G>>`).
    // A rev69 entry replays both silences warm. Mandatory.
    // rev69: SOUNDNESS R962/R963/R982 — Pass B's cached `calls` change (a held Result's payload, a struct-
    // pattern binding, a provably-string `let`, a renamed inactive local `use`); a rev68 entry replays the
    // silences warm. Mandatory.
    // rev68: SOUNDNESS R1004 — a file's item-position invocations of its own `macro_rules!` are expanded and
    // the functions they declare spliced in before Pass A; a rev67 entry lacks those units. Mandatory.
    // rev67: `field_elem`/`elem_of`/`static_types`/the element returns now record a std wrapper's
    // CONTAINER argument's element, MARKED (SOUNDNESS R893, `Mutex<Vec<G>>`). A rev66 entry has none — the
    // silence this change closes, served warm.
    // rev66: SOUNDNESS R986 — Pass A records path-redirected modules in `mod_aliases`; a rev65 entry
    // lacks them and replays the silence warm. Mandatory.
    // rev65: SOUNDNESS R960/R982/R985-R989 — Pass A splices `pin_project!` structs and `link!` foreign
    // declarations, records path-redirected module aliases and inactive-`use` fallbacks; Pass B types
    // transparent wrappers and pin projections. A rev64 entry replays the silences warm. Mandatory.
    // rev64: SOUNDNESS R984 — Pass B pushes a trait-path call's edge to the local impl for its argument's
    // type; a rev63 entry replays the dropped edge warm. Mandatory.
    // rev63: SOUNDNESS R979/R980 — Pass A files `impl_retgen_key`/`field_args_key` and a same-module alias's
    // target into `fields`; Pass B types a generic accessor's return and a pinned receiver — a rev62 entry
    // replays both silences warm. Mandatory.
    // rev62: SOUNDNESS R977 — a file or item compiled out of the default build is now walked inside a
    // `lang::CfgOffScope`, so its nested feature-gated `use`s and statements are KEPT; a rev61 entry replays
    // them dropped, and the stale direction is SILENCE. Mandatory.
    // rev61: ⟨0.40⟩ SOUNDNESS R817/R949 — a `bind` whose argument 0 is a provably string-typed runtime value
    // now sets the cached `Call`'s `path_lits_partial` (the Net bind "resolves a name" fact); a rev60 entry
    // replays it unset, and the bind certifies — the stale direction is SILENCE. Mandatory. R946 (the
    // refutable binders type a constructed payload) and R950 (`to_socket_addrs` on an untyped name
    // receiver), stacked on this change, also change Pass B's cached `calls` and ride this token.
    // rev60: SOUNDNESS R894 — Pass B emits a synthetic unit (`FnInfo::extern_decl`) for each `pub`
    // foreign-function declaration; a rev59 entry replays the file without it, and the report without the
    // row its consumers now join on. Mandatory.
    // rev59: SOUNDNESS R898 — Pass A now re-seeds an INLINE module's own path and declarations, so `fields`,
    // `rets` and the rest RECORD the anchored type for a `use self::…`/`use super::…` there instead of the raw
    // relative string; a rev58 entry replays the unanchored string and the silence it caused. Mandatory.
    // R810, R879 and R899 change what Pass B's walk RECORDS in the cached `FnInfo` (`calls`, `str_arg`) too.
    // rev58: the merge of VEIN A (rev56 on its branch) and VEIN E (rev56/rev57 on its branch) — both
    // lanes bumped from rev55 independently, so neither branch's token names the merged content. Mandatory.
    // rev56: VEIN A (R830, R181, R369, R193(a), R862, R863, R633's residual). Pass A's `expand` now keeps
    // a written `crate::` head and anchors a relative path whose head the module declares, and a
    // `self::`/`super::` `use` value is stored crate-rooted — so `fields`, `rets`, `mod_aliases`,
    // `foreign_impls` and the rest hold DIFFERENT strings; a struct's generic parameter now shadows a
    // same-named import in its field types; and Pass B's `calls` changed (anchored paths, glob origins,
    // the unfollowed-alias reading, the cfg-arm parameter union), with the dependency list folded into
    // the Pass B reuse key. A rev55 entry replays, warm, exactly the silences this closes. Mandatory.
    // rev57: VEIN E (SOUNDNESS R732, R754). `extern_fns` now also records an item-level `link!`, and a
    // bare call to a foreign import a BODY declares is recorded under `EXTERN_SENTINEL` — both change
    // what the cached decls and `calls` RECORD, and a rev56 entry replays the silent wrapper. Mandatory.
    // rev56: VEIN E (SOUNDNESS R145). `FileDecls` gained `opaque_include_modules`, `local_globs` and
    // `include_targets`, and a parse now SPLICES a readable `include!` target, which changes what
    // `fninfos` and every decl field RECORD for an including file. A rev55 entry deserializes the new
    // fields EMPTY and replays, warm, the silence this closes. Mandatory.
    // rev55: VEIN B (R193(b), R197, R733, R568, R542, R341, R861, R877, R878). Pass A's `rets` gained FOUR
    // key spaces (`impl_self_ret_key`, `ret_generic_key`, `elem_ret_key`, `impl_elem_ret_key`) and
    // `static_types` an element key per static; `elem_type_b` now records the std wrappers' argument,
    // which changes what `field_elem` RECORDS; and Pass B's `calls` changed (the strict `let` typer, the
    // wrapper accessors, the turbofish, the handle-argument edges). A rev54 entry deserializes every one
    // of those as absent and replays, warm, exactly the silences this closes. Mandatory.
    // rev54: ⟨0.40⟩ (SOUNDNESS R843). `FnInfo` gained `ret_proto` and `FileDecls` gained `ts`, the
    // per-file declared-type surface. A rev53 entry deserializes both as EMPTY, which on a warm run would
    // publish no `returnsProtocol` and a `types` manifest built from nothing — and a manifest that is
    // short is the one silent direction the rung has (a `supers` list read as complete). Mandatory.
    // rev48: a change in WHICH FUNCTIONS `fninfos` CONTAINS (SOUNDNESS R167). `scan_items` and `fn_locs`
    // now skip a fn carrying a `#[test]`-family attribute in the default scan, so a rev47 entry — written
    // by a binary that emitted a `FnInfo` for every bare `#[test] fn` at module scope — replays those
    // harness rows warm, with their `['Env','Fs','Log']` intact. The stale direction here is the
    // OVER-report, not R631's silence, so this bump is not about a cardinal sin; it is mandatory anyway
    // because `--incremental` promises a BYTE-IDENTICAL report and a warm rev47 read would differ from a
    // cold one on exactly the rows this change removes. Same argument as rev46.
    // rev47: an ANALYSIS change that feeds `fninfos`, not a new field (SOUNDNESS R722). `is_type_ident`
    // now admits an ALL-CAPS type name, so a body that constructs `IO`/`UTF8`/`HSTRING` gains a
    // `<Type>::<construct>` marker and a `let x = MARKER` binding gains its receiver typing — both of
    // which are stored IN the cached `FnInfo` (`CallCollector::calls`, and the `Type::method` call the
    // typing produces). A rev46 entry was written by a binary that emitted NEITHER and deserializes
    // without complaint, so a warm rev46 read republishes exactly the silence this row closes — R631's
    // direction, not R718's, which is why the bump is mandatory and not merely a byte-identity chore.
    // rev46: `FileDecls` gained `field_borrows` (SOUNDNESS R718). A rev45 entry deserializes it EMPTY
    // — `#[serde(default)]` — and an absent key reads as OWNED, so a warm scan over a stale cache
    // republishes the pre-fix FABRICATED drop-glue edge rather than losing a charge. That is the
    // over-report direction, not R631's silence, but the entry still has to be invalidated: the
    // `--incremental` contract is a BYTE-IDENTICAL report, and a warm rev45 read would differ from a
    // cold one on exactly the rows this row fixes.
    // rev45: an ANALYSIS change that feeds `fninfos`, not a field (SOUNDNESS R709). The escape model's
    // UNCONDITIONAL routes — a `mem::forget`/`ManuallyDrop::new` operand, an inline closure body, a
    // field/index/deref store — are now judged against the `?`s that precede them instead of being
    // re-united whole after R173's positional filter, so a body whose route lies AFTER an early exit
    // gains a `<Type>::<construct>` marker and with it a `<Type>::drop` call edge. That marker is a
    // `Call` in `CallCollector::calls`, which is stored IN the cached `FnInfo` — a rev44 entry was
    // written by a binary that emitted none of them and deserializes without complaint, so a warm scan
    // republishes R680's purity claim (`deny Fs <qual>::` exit 0 over a body that really drops one `H`
    // on its Err path). THE STALE DIRECTION IS SILENCE, the cardinal one, which is [[R631]]'s rule and
    // why this bump is part of the fix rather than housekeeping.
    // rev44: an ANALYSIS change that feeds `fninfos`, not a field (SOUNDNESS R693). Two widenings in
    // `visit_expr_path`: a trait-member fn-reference now mints its `{owner}#{qual}::{method}` key when
    // this crate's own `foreign_impls` witnesses the member (not only when the SIGNATURE bounds the
    // trait leaf), and the trait path is read as EVERY segment but the last rather than `segs[-2]`, so
    // `for_each(dep::Sink::emit)` forms the same key `for_each(Sink::emit)` already did. Both land in
    // `CallCollector::foreign_dispatch_sites`, which is stored IN the cached `FnInfo` — a rev43 entry was
    // written by a binary that minted neither, so the consumer hedge that reads `foreign_dispatch` cannot
    // fire and the warm scan republishes R690's silent purity claim. The stale direction is SILENCE, i.e.
    // the cardinal one, which is why this bump is part of the fix and not housekeeping.
    // rev43: `FileDecls::impl_members` gained its FOURTH key shape (SOUNDNESS R652 —
    // `<tr>\u{1f}<ty>\u{1f}!`, "the `impl <tr> for <ty>` block wrote a CRATE-LOCAL trait path"). The
    // FIELD is unchanged, its CONTENT is wider — the rev31/rev37 shape. A rev42 entry was written by a
    // binary that recorded no `!` keys at all, so every implementor in it reads as UNCONFIRMED and the
    // interface-union publishes NOTHING for a trait whose implementors are real. The stale direction is
    // therefore OVER-DISCLOSURE (a chained consumer charges `Unknown` on a dispatch that is answered),
    // not silence — but a warm scan that publishes a different document from a cold one on the same tree
    // is its own defect, exactly as rev41 says, and the rev is what stops it.
    // rev42: an ANALYSIS change that feeds `fninfos`, not a field (SOUNDNESS R569, whose fix landed in
    // `cf8782d` WITHOUT this bump — filed as R652). An unannotated `let` bound to a REFERENCE now types
    // its binding from `resolve_recv_type` (collector.rs, the `Expr::Reference` arm), so `let child =
    // &mut self.0; child.kill()` records a `Type::method` call it previously dropped. That write lands in
    // `CallCollector::vars`, which decides Pass B's `calls` list, and `calls` is stored IN the cached
    // `FnInfo` — so a rev41 entry was written by a binary that resolved none of those receivers, and it
    // deserializes without complaint. THE STALE DIRECTION IS SILENCE, NOT FABRICATION: a warm re-scan
    // replays exactly the pre-fix answer — cc's `KillOnDrop::drop` spawning no process, sqlx-core's
    // `Read::poll` reading no socket, postgres' `Client::is_valid` invisible to `deny Db` — i.e. a fix
    // that is correct but NOT REACHED, byte-identical from the outside to a fix that does not work.
    // Same shape as rev24/rev19 (an analysis change, not a field) and rev10, and neither `content_hash`
    // nor `decl_index_hash` can save it: the fix changes no file's bytes and no file's DECLS, only how an
    // unchanged body's receivers resolve, so a stale entry agrees with itself on both digests.
    // rev41: `FileDecls` gained `impl_members` (SOUNDNESS R598 — WHICH members an `impl Trait for Ty`
    // block declares, the fact `trait_impls` throws away). A rev40 entry has none, so it deserializes
    // EMPTY = "no evidence about this file's impl blocks", and the interface-union goes back to charging
    // an inherent `impl Ty { fn m }`'s effects to `Trait::m`. Unlike every rev below it the stale
    // direction here is a FABRICATION rather than a silence — but a warm scan that publishes a different
    // document from a cold one on the same tree is its own defect, and the rev is what stops it.
    // rev40: `FileDecls` gained `written_trait_quals` (SOUNDNESS R577 — the crate-qualified spelling a
    // FIELD / RETURN / CLOSURE-PARAM declaration wrote, which the leaf-keyed indexes throw away). A rev39
    // entry has none, so it deserializes EMPTY — i.e. "this file qualifies no trait", which is exactly
    // the silent-purity the field exists to close, served from a warm cache and invisible.
    // rev39: `FileDecls` gained `dyn_trait_fields` (SOUNDNESS R562 — whether a dispatch-typed FIELD was
    // spelled `dyn`, which is what the imported-trait CHA erasure carve-out must ask). A rev38 entry has
    // no such field, so `#[serde(default)]` reads an EMPTY map: "no field in this file is erased", for a
    // file whose `Box<dyn dep::Handler>` field is — and `self.inner.roll()` then reads `inferred: []`
    // exactly as it did pre-fix. Same trap as rev38/rev35/rev34/rev25.
    // rev38: `FileDecls` gained `static_types` (SOUNDNESS R557 — the declared type of every module-level
    // `static`/`const`, so one used as a METHOD RECEIVER resolves). A rev37 entry has no such field, so
    // `#[serde(default)]` reads an EMPTY map: "this file declares no typed static", for a file that does
    // — and `C1.fetch()` then falls back to the unit-struct fallback and vanishes from `functions[]`
    // exactly as it did pre-fix. Same trap as rev35/rev34/rev33/rev25: a fix served from a stale cache is
    // indistinguishable, from the outside, from a fix that does not work.
    // rev37: `extern_fns` now also carries the `extern "C" { fn … }` names declared INSIDE A BLOCK
    // (SOUNDNESS R529b). The FIELD is unchanged, its CONTENT is wider — which is precisely the rev31
    // shape ("a change to what an EXISTING field RECORDS"), and a rev36 entry replays the narrower set,
    // i.e. the silent-pure FFI call the row closes.
    // rev36: `FileDecls` gained `nested_impl_members` + `nested_impl_foreign` (SOUNDNESS R529 — the trait
    // impls written inside a BLOCK, which no other Pass A walk reaches). A rev35 entry has neither, so
    // `#[serde(default)]` reads both EMPTY — and an empty hedge set is byte-for-byte the pre-fix report:
    // the dispatch resolves to whatever visible implementor there is and certifies purity. Same trap as
    // rev35/rev34/rev33/rev25, and the reason this rev exists rather than riding the version alone.
    // rev32: the SAME class as rev31, three indexes wider (SOUNDNESS R478/R479/R482). `field_elem_trait`
    // now carries the `\u{1f}gf\u{1f}…` pending key space too (the ELEMENT half of the impl-bound join);
    // `trait_fields` gains a general `Fields::Unnamed` arm, so a tuple position records dispatch leaves a
    // rev31 binary never wrote; and `field_elem` no longer records an entry that is the struct's own
    // generic PARAMETER name. All four are per-file Pass A outputs and all four are what a warm entry
    // replays, so a rev31 entry re-serves precisely the silences these rows close. Reproduced, not
    // argued — see the rev31 note for the shape of that reproduction.
    // rev31: a change to what an EXISTING field RECORDS (SOUNDNESS R476) — `trait_fields` now carries
    // two RESERVED key spaces (`\u{1f}ib\u{1f}…`, `\u{1f}gf\u{1f}…`) whose crate-wide join supplies the
    // dispatch leaves for a generic FIELD bounded on an `impl` block, and the join REMOVES the
    // shadowing `fields` entry for exactly those fields. Both are per-file Pass A outputs, so a rev30
    // entry was written by a binary that recorded neither: a warm re-scan reads "this struct has no
    // dispatch-typed field" for one that has a bounded generic, and replays precisely the silent
    // under-report the row closes. Unlike rev30 this one has an EXECUTED reproduction rather than an
    // argument: on a 4-file fixture (struct in `ty.rs`, `impl<B: Backend>` in `imp.rs`), a PRE binary's
    // `--incremental` run writes the cache, and a binary carrying THIS FIX BUT NOT THIS BUMP then reads
    // it warm and reports 3 rows — `XTerm::dims`, `BoxTerm::dims`, `Pair::second_dims` and
    // `Pair::first_shout` ABSENT, exactly the pre-fix answer — while its own cold scan reports 7. With
    // the bump the entry is discarded and the warm run reproduces the cold one byte for byte.
    // rev30: the ADMITTED FILE SET changed (SOUNDNESS R459 — a `#[cfg(test)]` file module is excluded
    // whatever its filename). This is upstream of everything the cache stores: `decl_index_hash` is
    // computed FROM the decls of the admitted set, so a warm rev29 cache plus a rev30 walk is two
    // binaries disagreeing about which files exist. I could not construct a concrete stale READ — an
    // excluded file's entry is simply never looked up, and a newly admitted one has no entry — so this
    // bump is not backed by a reproduction, and it is taken anyway: this file's own rule is that a
    // change in what FEEDS the cached representation needs a bump exactly as much as a field addition,
    // and "I could not build the bad case" is the sentence this register keeps finding on the wrong
    // side of a silent under-report.
    // rev29: `elem_type` gained the MAP arm (SOUNDNESS R454 — a map's concrete VALUE is its element).
    // It runs in Pass A and its answer is STORED, in `FileDecls::field_elem`, so a rev28 entry holds a
    // `field_elem` with no map rows — the silent purity claim this rev exists to remove, served warm.
    // `decl_index_hash` cannot save it (computed FROM the cached decls, so a stale entry agrees with
    // itself) and neither does the embedded package version, which did not move for this change.
    // MEASURED rather than reasoned: pre-fix binary writes the cache, post-fix binary reads it, and
    // `Reg::map_index` over a `HashMap<String, G>` stays ABSENT — while the same tree scanned COLD
    // charges `['Exec']`. The FIFTH rev for this identical reason; see rev28/rev27/rev26/rev25.
    // rev28: `FileDecls` gained `macro_hidden_types` + `macro_hidden_fns` (SOUNDNESS R452 — the types
    // declared inside a module whose items an unexpanded macro hid, and the `fn` names that text
    // mentions). A rev27 entry has none and deserializes EMPTY, which
    // is exactly the silent purity claim the field closes, replayed from a warm cache; and
    // `decl_index_hash` cannot save it, because that digest is computed FROM the cached decls, so a
    // stale entry agrees with itself. The fourth rev for that identical reason — see rev27/rev26/rev25.
    // rev27: `FileDecls::rets` gained the IMPL-QUALIFIED return keys (SOUNDNESS R451, `impl_ret_key`).
    // A rev26 entry holds a `rets` map recorded by a binary that never wrote one, and `decl_index_hash`
    // cannot save it: that digest is computed FROM the cached decls, so a stale entry agrees with itself
    // and the merged index simply comes out without the keys. The reader then finds nothing for every
    // `base::leaf`, the chain walk stays as it was, and the fabrication this rev exists to remove is
    // replayed from a warm cache — a fix that is correct but NOT REACHED, byte-identical from the
    // outside to a fix that does not work. The third rev for that exact reason; see rev26 and rev25.
    // rev26: `Call` gained `entropy_arg` — an argument names the OS entropy source (SOUNDNESS R334). A
    // rev25 entry has no such field and deserializes to `false`, i.e. "no argument hands over the OS
    // RNG", which is precisely the silent purity claim this rev exists to remove, replayed from a warm
    // cache. This is the SECOND field added to `Call` in one session and the second rev for the same
    // reason; the reason is worth stating once more because it is the cheap half of a lesson whose
    // expensive half was learned three separate times this week: a fix that is correct but not REACHED
    // is byte-identical, from the outside, to a fix that does not work.
    // rev35: `FileDecls` gained `trait_quals` and `FnInfo` gained `foreign_dispatch` (SOUNDNESS R503 +
    // R504 — the WIRE spelling of a dispatched abstraction, and the middle-package dispatch site). A
    // rev34 entry has neither, so `#[serde(default)]` reads both EMPTY: every local interface-union
    // entry falls back to the leaf key §4 ⟨0.39⟩ forbids, and a package dispatching over its OWN
    // dependency's trait publishes no `dispatchesOn` at all — which is byte-for-byte the pre-fix report
    // in both cases. Same trap as rev34/rev33/rev25: a fix served from a stale cache is
    // indistinguishable, from the outside, from a fix that does not work.
    // rev34: `FileDecls` gained `foreign_impls` (SPEC §4 ⟨0.39⟩ obligation 2 — the abstractions a file
    // implements that it does NOT own). A rev33 entry has no such field, so `#[serde(default)]` reads an
    // EMPTY map and the crate publishes NO foreign interface-union entry — which is byte-for-byte the
    // pre-rung report, i.e. the warm cache serves exactly the silent purity claim ⟨0.39⟩ exists to close
    // and does so INVISIBLY (a missing entry and a crate with no foreign impl are the same bytes). Same
    // trap as rev25/rev33: a fix that is correct but served from a stale cache is indistinguishable, from
    // the outside, from a fix that does not work.
    // rev33: FnInfo gained `unresolved_why` (SOUNDNESS R485 — the SPEC §4 reason behind the `unresolved`
    // bool, recorded per write site). A rev32 entry has no such field, so `#[serde(default)]` reads an
    // EMPTY vec, and `scan.rs`'s fail-closed fallback then republishes the pre-fix `callback:unresolved
    // call` for a dispatch or ambiguity hole — i.e. the warm cache serves exactly the wrong reason class
    // this rev exists to correct, and it does so INVISIBLY: the effect set is `['Unknown']` on both sides,
    // so nothing but the reason string distinguishes a stale entry from a fixed one. Same shape as rev10,
    // and the same trap as rev25 — a fix that is correct but served from a stale cache is byte-identical,
    // from the outside, to a fix that does not work.
    // rev25: `Call` gained `argc`, the arity written at the call site (SOUNDNESS R330). A rev24 entry
    // has no such field and deserializes to 0 — the "NOT RECORDED" sentinel — so every cached call in
    // the password-hash family would keep the fabricated `Rand` this rev exists to remove, served from a
    // warm cache and looking exactly like a fix that does not work. The direction is the safe one, which
    // is why it needs the bump rather than being caught: a stale entry here degrades SILENTLY into the
    // old answer, and the previous evening produced three separate cases of correct code on a path the
    // failing case never takes. Discard rev24 wholesale rather than trust the default.
    // rev24: an ANALYSIS change that feeds `fninfos`, not a field (SOUNDNESS R271). The expression
    // WRAPPED around a callable is now peeled by one shared authority, so `v.retain(match 0 { _ =>
    // local_eff })` and `let g = unsafe { fptr }; g(..)` reach the callee instead of dropping it, and a
    // fan-out `let g = if c { a } else { b }` records BOTH targets. A rev23 entry was written by a
    // binary that did none of that, and it deserializes without complaint — so a warm re-scan replays,
    // invisibly, the silent under-report this closes: `getrandom` `fill_inner` calling a `transmute`d
    // syscall pointer, `trybuild` `run::Test::check` spawning `cargo` and writing files, both read as
    // performing nothing. Same shape as rev19 and rev20 — a change in the ANALYSIS that produces the
    // cached `FnInfo`s needs a bump exactly as much as a change in the cached FIELDS.
    // rev23: a change to what an EXISTING field RECORDS (R238) — `trait_fields` now also carries the
    // synthetic `"Fn"` leaf for a struct/tuple field whose declared type is an INVOKABLE callback but
    // carries no trait in its syntax (`cb: fn(&i32) -> bool`, a callable type ALIAS, `Option<fn(..)>`).
    // A rev22 entry was written by a binary that recorded NONE of those, so serde reads it without
    // complaint and yields "this type has no callable field" for a type that does — and the consequence
    // is exactly the silent under-report the entry closes, served warm and invisible: `v.retain(h.cb)`
    // hands a caller-supplied body to an invoking adapter and the enclosing fn is ABSENT from
    // `functions[]`. Same shape as rev20, rev16 and rev13 — a field ADDITION is not the only thing that
    // needs a bump, a change in what an existing field records does too.
    // rev22: FileDecls gained `macro_twins` (R208 — the `macro_rules!` names one file defines twice with
    // different templates). A rev21 entry has none, so it deserializes EMPTY: "this file twins no macro",
    // for a file that does — and the consequence is the order-dependent silence the field exists to
    // disclose, served warm and invisible. Same shape as rev17.
    // rev21: `rets` gained the `<amb>` CANDIDATE entries (R182/R196 — which types the ambiguity rule
    // withdrew from a colliding fn leaf, so the drop route can disclose instead of certifying the
    // caller pure). A rev20 entry has none, so a warm re-scan would find the withdrawal but not the
    // candidates and emit NO disclosure — i.e. exactly the silence the entries exist to close, served
    // warm and invisible. Same shape as rev17.
    // rev20: a change to what an EXISTING field RECORDS (R188) — `rets` no longer files a unit
    // return under the fn's own LEAF but under `<unit><leaf>`, so a rev19 entry deserializes into a
    // map whose typed leaf was withdrawn by its unit twin, and the warm cache replays `let c =
    // net::connect(a); c.send(b)` with no receiver type at all. Same shape as rev16: a field
    // addition is not the only thing that needs a bump.
    // rev19: an ANALYSIS change that feeds `fninfos`, not a field (R187) — a `?` inside a loop now
    // vetoes what that loop body builds, so a rev18 entry replays, warm, a body that reads as pure
    // while its guard demonstrably drops. serde would read that entry without complaint; the token is
    // the only thing that stops it. Same shape as rev16.
    // rev53: `FileDecls` gained `trait_assoc` (R776 — a trait's ASSOCIATED fns) and `nonnominal_impls`
    // (R828 — a non-nominal implementor's unit), `nested_impl_members`/`nested_impl_foreign` now also
    // record macro-generated and derived implementors (R828), `trait_impls`/`impl_members` key a renamed
    // trait import under the trait's real leaf (R828), and Pass B's `calls` records new dispatch edges
    // for trait-member PATH calls and default-body `self.m()` (R570, R576(a), R629, R743). A rev52 entry
    // deserializes the new fields EMPTY and replays the old `calls`, so a warm cache would serve exactly
    // the silences those rows close. Same shape as rev52.
    // rev52: SOUNDNESS R856/R857 changed what Pass B's `calls` RECORDS — a method on a dependency VALUE
    // (`dep::SHARED.ping()`, `n.parent.visit()` with `n: &dep::Node`, `let s = &dep::SHARED; s.ping()`)
    // now emits a `<untyped>` marker, and a qualified unit-struct literal (`m::Unit.go()`) now types.
    // `calls` is stored IN the cached `FnInfo`, so a rev51 entry replays the marker-less list warm and
    // the caller reads ABSENT again — the exact silence the change closes. The bump is mandatory.
    // rev51: SOUNDNESS R718's OWNED-WITH-A-BORROW half changed what `field_borrows` RECORDS — one bool
    // from the whole declared type became a `FieldBorrow` per index, read off each index's own walk. A
    // rev50 entry holds the bool, which does not deserialize as the new shape; and even if it did, it
    // carries the whole-type answer that read `HashMap<&str, Guard>` / `TempFile<&Path>` as BORROWED,
    // so a warm read would replay the withdrawn drop — the SILENCE direction. The bump is mandatory.
    // rev50: SOUNDNESS R751 changed what an EXISTING field RECORDS. Pass A resolves TYPE paths through
    // `expand`, which now turns a `self::`/`super::`-rooted path into a crate-root-ABSOLUTE one instead of
    // collapsing it — so `fields`, `rets`, `field_elem` and the rest hold DIFFERENT strings for every
    // relative path in the file. A rev49 entry was written by a binary that collapsed them, so a warm
    // cache serves the pre-fix answer: the module context absent, the path matching no definition, and
    // the caller ABSENT from `functions[]` over a call that really performs the effect. Same shape as
    // rev16 — a change in what a field records needs a bump exactly as a field addition does.
    // rev49: FileDecls gained `root_decls` (R186 — the crate ROOT's own declared item names). A rev48
    // entry has none, so it deserializes EMPTY: "the crate root declares nothing" — and an empty set is
    // exactly what makes `expand`'s glob branch attribute `crate::stream::Stream` to a prelude again. A
    // warm cache would therefore serve the pre-fix SILENCE (caller absent from `functions[]`, `deny Fs`
    // exit 0 over a real read) for every unchanged file, which is the same shape as rev17, rev15, rev14,
    // rev12 and rev9.
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
    format!("scan-{}/rev73/tests={}", env!("CARGO_PKG_VERSION"), include_tests)
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
    /// SOUNDNESS R718 — `fields`' OWNERSHIP twin, keyed IDENTICALLY (struct leaf -> field name, or the
    /// tuple POSITION's string form) and carrying, per declaration, whether the field's written type
    /// BORROWS (`&T`, `&mut T`, `Option<&mut T>`, `*const T`, `&[T]`). `type_path` peels references on
    /// the way into `fields`, so `g: &'a mut G` arrives there as the bare owned leaf `G` and the R49
    /// transitive drop-owner closure fabricated `G::drop` at every construction of the borrowing
    /// struct. The fact cannot be recovered downstream — only the declaration has the `&` — so it is
    /// recorded HERE, beside the entry it qualifies, and read by nothing else.
    ///
    /// A NEGATIVE index would have been cheaper and is wrong: a leaf collision (R213) where one twin
    /// OWNS the field and the other BORROWS it must keep the charge, and a union of "borrowed" keys
    /// would drop it. Both polarities are recorded so `merge_decls` can let OWNED win.
    #[serde(default)]
    pub(crate) field_borrows: HashMap<String, HashMap<String, crate::model::FieldBorrow>>,
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
    /// R776 — `LocalTrait::assoc`, flattened beside `trait_decls` (trait leaf -> associated fn names).
    /// A separate map rather than a fourth tuple slot so the existing triple keeps its shape.
    #[serde(default)]
    pub(crate) trait_assoc: HashMap<String, Vec<String>>,
    pub(crate) trait_fields: TraitFieldIndex,
    /// SOUNDNESS R562 — the `dyn`-ONLY twin of `trait_fields`, keyed identically (struct leaf ->
    /// field name -> trait leaves). `trait_fields` collapses `dyn T`, `impl T` and `T: Bound`; the
    /// imported-trait CHA erasure carve-out is the one consumer that must tell them apart, and it must
    /// ask PER RECEIVER — a crate-wide union would let one struct's `dyn Serializer` field license CHA
    /// on every unrelated `T: Serializer` receiver in the crate (R4's measured flood).
    #[serde(default)]
    pub(crate) dyn_trait_fields: TraitFieldIndex,
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
    /// SOUNDNESS R557 — `static`/`const` NAME → its DECLARED type path, for this file. `None` is a
    /// REFUSED name: either two declarations of the leaf disagreed, or the type is one `type_path`
    /// declines to name (a trait object, a generic cell). See `decls::collect_static_types`.
    #[serde(default)]
    pub(crate) static_types: HashMap<String, Option<String>>,
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
    /// SOUNDNESS R208 — the `macro_rules!` NAMES this file defines MORE THAN ONCE with DIFFERENT arm
    /// tokens. `local_macros` is keyed by BARE NAME and merges last-writer-wins, so which module's
    /// template R48 expands depends on FILE ORDER: measured, `r5` and `r5b` (the same two files, swapped)
    /// disagree on whether the caller is charged. The order-dependence is NOT fixed here — an index that
    /// records duplicates and refuses is a separate mechanism, and refusing would WITHDRAW the charge the
    /// winning template supplies, which this family does not do without its own A/B. What changes is that
    /// an invocation of a twinned name now DISCLOSES instead of certifying whichever template won the race.
    #[serde(default)]
    pub(crate) macro_twins: Vec<String>,
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
    /// SOUNDNESS R186 — the item names the crate ROOT declares ITSELF, populated ONLY for the root file
    /// (module path ""). Seeded into every file's `use` map under `crate::` + `ROOT_DECL_KEY` so
    /// `expand` can refuse to attribute a `crate::<name>::…` path to a re-export glob when `<name>` is a
    /// declaration of this crate — see the R186 comment at that branch. A cache entry written before this
    /// field deserializes EMPTY, i.e. "the root declares nothing", which restores the pre-fix silence for
    /// every warm file: the under-report direction, and the reason for the rev bump in `cache_schema`.
    #[serde(default)]
    pub(crate) root_decls: Vec<String>,
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
    /// SOUNDNESS R145 — MODULE QUALS in this file holding an `include!` this scan could NOT read (an
    /// `env!("OUT_DIR")` path, a missing or unparseable target). A readable target is SPLICED at parse
    /// time and is not here. See `lang::splice_includes`.
    #[serde(default)]
    pub(crate) opaque_include_modules: Vec<String>,
    /// SOUNDNESS R145 — this file's crate-local GLOB imports, `"<importer>\u{1}<imported module>"`. See
    /// `lang::collect_local_globs`; closes `opaque_include_modules` under re-export by glob.
    #[serde(default)]
    pub(crate) local_globs: Vec<String>,
    /// SOUNDNESS R145 — every file an `include!` in this file named, as consulted by its last parse. Part
    /// of the cache key (`lang::include_closure_hash`), never of the decl index.
    #[serde(default)]
    pub(crate) include_targets: Vec<String>,
    /// SOUNDNESS R452 — the TYPE names this file declares inside one of `macro_modules`. A typed method
    /// call on such a type resolves to nothing through no fault of the program, so it HEDGES rather than
    /// reading the absence of an `impl` as purity. See `collect_macro_hidden_types` for why this is keyed
    /// on the TYPE where R128 is keyed on the call's module path. A cache entry written before this field
    /// deserializes EMPTY — precisely the silent purity claim the field closes, hence the rev bump.
    #[serde(default)]
    pub(crate) macro_hidden_types: Vec<String>,
    /// SOUNDNESS R452 — the `fn` NAMES this file's unexpanded macro text mentions, from an invocation's
    /// arguments or a `macro_rules!` body. The second half of the hedge's evidence: without it the
    /// module fact alone hedges four times as many callers, nearly all of them std combinators on a
    /// mis-typed receiver. See `collect_macro_hidden_decls`. Absent from a pre-rev28 entry.
    #[serde(default)]
    pub(crate) macro_hidden_fns: Vec<String>,
    /// ⟨0.39⟩ SPEC §4 obligation 2 — `"<owning crate>#<trait qual>::<method>" -> the LOCAL impl method
    /// quals implementing it`, for abstractions this file implements that it does NOT own. See
    /// `lang::collect_foreign_trait_impls`. A cache entry written before this field deserializes EMPTY,
    /// which republishes precisely the silence the rung closes — hence the rev bump in `cache_schema`.
    #[serde(default)]
    pub(crate) foreign_impls: HashMap<String, Vec<String>>,
    /// SOUNDNESS R503 — `trait leaf -> the MODULE-QUALIFIED path(s) this file declares it at`
    /// (`Backend -> {backend::Backend}`). See `lang::collect_trait_decl_quals`. Absent from a pre-rev35
    /// entry, which is why that rev exists: an empty map makes every local interface-union entry and
    /// every `dispatchesOn` value fall back to the LEAF spelling — the one §4 ⟨0.39⟩ names as the
    /// second spelling it forbids — served invisibly from a warm cache.
    #[serde(default)]
    pub(crate) trait_quals: HashMap<String, Vec<String>>,
    /// SOUNDNESS R577 — every CRATE-QUALIFIED trait bound this file WRITES (`&dyn dep::Q`,
    /// `Box<dyn dep::Q>`, `impl dep::Q`), leaf -> path, `""` = two spellings seen, refuse. The
    /// declaration-site half of the qualification that `trait_fields`/`rets`/the closure binder throw
    /// away; see `lang::collect_written_trait_quals`.
    #[serde(default)]
    pub(crate) written_trait_quals: HashMap<String, String>,
    /// SOUNDNESS R529 — the `"{trait leaf}::{method}"` pairs this file implements INSIDE A BLOCK, which
    /// no other Pass A index can see. See `lang::collect_block_nested_trait_impls`. A pre-rev36 entry
    /// deserializes EMPTY, which re-serves exactly the silent purity claim the row closes — hence the
    /// rev bump.
    #[serde(default)]
    pub(crate) nested_impl_members: Vec<String>,
    /// SOUNDNESS R529 — the same fact for an abstraction this file does NOT own, in
    /// `collect_foreign_trait_impls`'s own `"{owner}#{trait qual}::{method}"` key spelling.
    #[serde(default)]
    pub(crate) nested_impl_foreign: Vec<String>,
    /// SOUNDNESS R828 — a LOCAL trait implemented for a NON-NOMINAL self type (`impl T5 for (u8, u8)`):
    /// `"{trait leaf}::{method}\u{1f}{unit qual}"`. `collect_decls` files no CHA edge for it (no type
    /// name), but `scan_items` DOES mint the method as a unit — under the module path alone — so the
    /// implementor can be reached, which a hedge would only disclose. See `lang::collect_opaque_trait_impls`.
    #[serde(default)]
    pub(crate) nonnominal_impls: Vec<String>,
    /// SOUNDNESS R598 — the EVIDENCE about which members this file's `impl Trait for Ty` blocks declare
    /// (`lang::collect_local_impl_members`; three key shapes, `model::impl_seen_key`). A pre-rev41 entry
    /// deserializes EMPTY, which reads as "no evidence" and restores the pre-fix over-approximation for
    /// every file served warm: a fabricated effect, not a silence — the safe direction, and a rev bump
    /// all the same, because a warm scan that disagrees with a cold one is its own defect.
    #[serde(default)]
    pub(crate) impl_members: Vec<String>,
    /// ⟨0.40⟩ this file's contribution to `typeSurface.holds`/`types`/`adds` (and the `use` paths the
    /// consumer's in-scope check reads) — see `typesurf::FileSurface`. Cached so a warm run publishes what
    /// a cold one does.
    #[serde(default)]
    pub(crate) ts: crate::typesurf::FileSurface,
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
    // SOUNDNESS R751 — Pass A resolves TYPE paths through `expand` too, so it needs the same fact.
    crate::lang::seed_modpath(modpath, &mut uses);
    // VEIN A — and what a relative path's head can name in this module, so Pass A's TYPE paths are
    // anchored by the same rule Pass B's call paths are (`lang::module_declares`).
    crate::lang::seed_moddecls(items, include_tests, &mut uses);
    let mut fields = HashMap::new();
    let mut field_borrows = HashMap::new();
    let mut field_elem = HashMap::new();
    let mut field_elem_trait = HashMap::new();
    let mut rets = HashMap::new();
    let mut enum_tmp = HashMap::new();
    let mut enum_variant_traits = HashMap::new();
    let mut trait_impls = HashMap::new();
    let mut trait_decls: HashMap<String, LocalTrait> = HashMap::new();
    let mut trait_fields = HashMap::new();
    let mut dyn_trait_fields = HashMap::new();
    let mut prim_aliases = std::collections::HashSet::new();
    let mut extern_fns = std::collections::HashSet::new();
    let mut drop_types = std::collections::HashSet::new();
    let mut deref_target = HashMap::new();
    let mut lazy_statics = std::collections::HashSet::new();
    let mut const_strings = HashMap::new();
    let mut local_macros = HashMap::new();
    let mut macro_twins: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
    let mut blanket_methods = HashMap::new();
    let mut callable_statics = std::collections::HashSet::new();
    let mut callable_aliases = std::collections::HashSet::new();
    // R177 — this file's `type NAME = <callable>` aliases BEFORE the walk that consumes them. See
    // `seed_callable_aliases` for why it is a separate pass and why its boundary is one file.
    crate::decls::seed_callable_aliases(items, include_tests, &mut callable_aliases);
    collect_decls(items, include_tests, &mut uses, &mut fields, &mut field_elem, &mut field_elem_trait, &mut rets,
                  &mut enum_tmp, &mut enum_variant_traits, &mut trait_impls, &mut trait_decls, &mut trait_fields, &mut dyn_trait_fields, &mut prim_aliases,
                  &mut extern_fns, &mut drop_types, &mut deref_target, &mut lazy_statics, &mut const_strings, &mut local_macros, &mut macro_twins, &mut blanket_methods, &mut callable_statics, &mut callable_aliases, &mut field_borrows);
    // ONE walk produces both re-export channels — the intra-crate edges (`reexports`) and the
    // external/alias map (`mod_aliases`, R99), which is collected at the very branch that used to DROP
    // an external `pub use`. `uses` is this file's top-level `use` map, as `collect_decls` left it, so a
    // `type Cmd = Command;` written after `use std::process::Command;` expands to the std path.
    let mut reexports = Vec::new();
    let mut mod_aliases = HashMap::new();
    collect_reexports(items, modpath, &dir, include_tests, &uses, &mut reexports, &mut mod_aliases);
    // R128 + R452 — ONE walk, two facts: which of this file's modules had items hidden behind an
    // unexpanded macro, and which TYPES those modules declare. Computed here rather than in the struct
    // literal so the second cannot be derived from a different module set than the first.
    let mut macro_mods = std::collections::HashSet::new();
    crate::lang::collect_macro_modules(items, modpath, include_tests, &mut macro_mods);
    // SOUNDNESS R145 — which of those are an `include!` the parse could not read, and the crate-local
    // globs that carry such a module's names elsewhere.
    let mut opaque_inc = std::collections::BTreeSet::new();
    crate::lang::collect_opaque_include_modules(items, modpath, include_tests, &mut opaque_inc);
    let mut local_globs = std::collections::BTreeSet::new();
    crate::lang::collect_local_globs(items, modpath, include_tests, &mut local_globs);
    let mut macro_hidden_ty = std::collections::HashSet::new();
    let mut macro_hidden_fn = std::collections::HashSet::new();
    crate::lang::collect_macro_hidden_decls(
        items, modpath, include_tests, &macro_mods, &mut macro_hidden_ty, &mut macro_hidden_fn);
    // ⟨0.39⟩ obligation 2 — the FOREIGN abstractions this file implements, keyed under the OWNING crate.
    // Walked here (not inside `collect_decls`) for the same reason the re-export walk is: it needs the
    // file's assembled `use` map, which is what `collect_decls` has just finished producing.
    let mut foreign_impls = HashMap::new();
    crate::lang::collect_foreign_trait_impls(items, include_tests, &uses, &mut foreign_impls);
    // SOUNDNESS R557 — the declared TYPE of every module-level `static`/`const`. Walked here, beside the
    // two walks above and for their reason: it resolves the annotation through `type_path`, which needs
    // the file's ASSEMBLED `use` map (`static C: Client` where `use dep::Client;` is written below it).
    // `collect_decls` visits the const/static arm mid-walk, before that map is complete.
    let mut static_types = HashMap::new();
    crate::decls::collect_static_types(items, include_tests, &uses, &mut static_types);
    // R503 — the MODULE-QUALIFIED path of every trait this file DECLARES. Walked here beside the
    // foreign-impl walk because it answers the same question from the other side: that one records what
    // an `impl` calls somebody else's abstraction, this one records what the OWNER calls its own. It
    // needs `modpath`, which `collect_decls` does not carry, so it is a separate walk.
    let mut trait_quals: HashMap<String, std::collections::BTreeSet<String>> = HashMap::new();
    crate::lang::collect_trait_decl_quals(items, modpath, include_tests, &mut trait_quals);
    // SOUNDNESS R577 — and the other side of the SAME naming question: not the traits this file
    // DECLARES, but the crate-qualified spellings it WRITES for somebody else's. A separate walk for the
    // reason the three above are: it needs no `use` map at all (only an explicit multi-segment path is
    // recorded), and `collect_decls` has no site that could carry the fact to a crate-wide index.
    let mut written_trait_quals: HashMap<String, String> = HashMap::new();
    crate::lang::collect_written_trait_quals(items, include_tests, &mut written_trait_quals);
    // R529 — the trait impls written inside a BLOCK. Walked beside the two above because it answers the
    // question they cannot: both of those recurse through `Item::Mod` and nothing else, so an
    // `impl Trait for Type` in a fn body reaches neither, while Pass B walks into that body and charges
    // its effects to the enclosing fn. Needs the file's assembled `use` map, like the foreign walk.
    let mut nested_local: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
    let mut nested_foreign: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
    // R529b — the same walk also answers "which `extern "C"` names are declared inside a block". They
    // go straight into `extern_fns`, the crate-wide leaf set the item-level `Item::ForeignMod` arm
    // already feeds: there is ONE question here ("is this leaf an FFI declaration in this crate") and it
    // must have one answer, not a second index a future reader could consult instead of this one.
    let mut nested_externs: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
    crate::lang::collect_block_nested_trait_impls(
        items, include_tests, &uses, &mut nested_local, &mut nested_foreign, &mut nested_externs);
    // SOUNDNESS R828 — and the item-level implementors the CHA index cannot name (a non-nominal self
    // type, a `macro_rules!`-generated impl, a `#[derive]`), into the SAME two sets: one question
    // ("does this crate implement that member somewhere the index cannot see?"), one answer.
    let mut nonnominal: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
    crate::lang::collect_opaque_trait_impls(
        items, include_tests, modpath, &uses, &mut nested_local, &mut nested_foreign, &mut nonnominal);
    extern_fns.extend(nested_externs);
    // R598 — which members each `impl Trait for Ty` block actually declares. Walked beside the two
    // above for the same reason and over the same `items`: `collect_decls` records the CHA edge and
    // discards the block's contents, so `{ty}::{method}` cannot be told apart from an inherent
    // `impl Ty { fn method }`.
    let mut impl_members: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
    crate::lang::collect_local_impl_members(items, include_tests, &uses, &mut impl_members);
    FileDecls {
        fields,
        field_borrows,
        field_elem,
        field_elem_trait,
        rets,
        enum_tmp,
        enum_variant_traits,
        trait_impls,
        trait_assoc: trait_decls
            .iter()
            .filter(|(_, v)| !v.assoc.is_empty())
            .map(|(k, v)| {
                let mut a: Vec<String> = v.assoc.iter().cloned().collect();
                a.sort(); // content-hashed: a HashSet's order must not reach the entry
                (k.clone(), a)
            })
            .collect(),
        trait_decls: trait_decls
            .into_iter()
            .map(|(k, v)| (k, (v.count, v.methods.into_iter().collect(), v.supertraits)))
            .collect(),
        trait_fields,
        dyn_trait_fields,
        prim_aliases: prim_aliases.into_iter().collect(),
        extern_fns: extern_fns.into_iter().collect(),
        drop_types: drop_types.into_iter().collect(),
        deref_target,
        lazy_statics: lazy_statics.into_iter().collect(),
        callable_statics: callable_statics.into_iter().collect(),
        static_types,
        callable_aliases: callable_aliases.into_iter().collect(),
        const_strings,
        local_macros,
        // R208 — drop the `cfg\u{1f}NAME` bookkeeping entries `collect_decls` used to remember which
        // names it had seen `#[cfg]`-gated; only the twinned NAMES cross this boundary.
        macro_twins: macro_twins.into_iter().filter(|s| !s.contains('\u{1f}')).collect(),
        blanket_methods,
        // Crate-root re-exports are a ROOT-file fact only; a submodule's `use crate::X` seeds against them.
        root_reexports: if modpath.is_empty() { collect_root_reexports(items, include_tests) } else { HashMap::new() },
        // R186 — the root's OWN declarations, a root-file fact for the same reason and from the same
        // `items`. A `BTreeSet` on the wire as a sorted `Vec`, so the content hash is deterministic.
        root_decls: if modpath.is_empty() {
            crate::lang::collect_root_decls(items, include_tests).into_iter().collect()
        } else {
            Vec::new()
        },
        // …and the SUBMODULE-level re-exports, which every file can contribute (the crate root included:
        // a `pub use self::platform::*` at the root is BOTH a root re-export and a module-alias edge, and
        // the two answer different questions — see `Reexport`).
        reexports,
        mod_aliases,
        // R128 — which of this file's modules had items hidden behind an unexpanded macro invocation.
        // Keyed on the module QUAL (the file's own `modpath` for its top level), which is the same key
        // space a call's resolved `crate::…` path reduces to at the resolver.
        opaque_include_modules: opaque_inc.into_iter().collect(),
        local_globs: local_globs.into_iter().collect(),
        include_targets: Vec::new(),
        macro_modules: {
            let mut v: Vec<String> = macro_mods.iter().cloned().collect();
            v.sort(); // deterministic on the wire — the cache entry is content-hashed
            v
        },
        // R452 — the types declared INSIDE those modules. Derived from the SAME `items` and the SAME
        // module set, computed once above, so the two facts cannot drift apart: a module that stops
        // being macro-hidden withdraws its types in the same pass.
        macro_hidden_types: {
            let mut v: Vec<String> = macro_hidden_ty.iter().cloned().collect();
            v.sort();
            v
        },
        macro_hidden_fns: {
            let mut v: Vec<String> = macro_hidden_fn.iter().cloned().collect();
            v.sort();
            v
        },
        foreign_impls: {
            // Deterministic on the wire — the cache entry is content-hashed.
            let mut m = foreign_impls;
            for v in m.values_mut() {
                v.sort();
                v.dedup();
            }
            m
        },
        // R503 — a `BTreeSet` in, a sorted `Vec` out: same determinism requirement, same reason.
        trait_quals: trait_quals.into_iter().map(|(k, v)| (k, v.into_iter().collect())).collect(),
        written_trait_quals,
        // R529 — `BTreeSet` in, sorted `Vec` out; the cache entry is content-hashed.
        nested_impl_members: nested_local.into_iter().collect(),
        nested_impl_foreign: nested_foreign.into_iter().collect(),
        nonnominal_impls: nonnominal.into_iter().collect(),
        // R598 — same determinism requirement, same reason.
        impl_members: impl_members.into_iter().collect(),
        ts: crate::typesurf::collect_file(items, modpath, include_tests),
    }
}

/// The assembled crate-wide decl index (Pass A output), ready for Pass B — exactly the seven structures
/// `scan_one` built inline before, now produced from per-file `FileDecls` so unchanged files contribute
/// from cache. `rets`/`enum_tmp` keep the `Option` ambiguity marker until the caller filters them.
#[derive(Default)]
pub(crate) struct MergedDecls {
    pub(crate) fields: FieldIndex,
    /// SOUNDNESS R718 — see `FileDecls::field_borrows`. Merged so OWNED WINS.
    pub(crate) field_borrows: HashMap<String, HashMap<String, crate::model::FieldBorrow>>,
    pub(crate) field_elem: FieldElemIndex,
    pub(crate) field_elem_trait: FieldElemTraitIndex,
    pub(crate) rets: HashMap<String, Option<String>>,
    pub(crate) enum_tmp: HashMap<String, Option<String>>,
    pub(crate) enum_variant_traits: HashMap<String, Option<Vec<String>>>,
    pub(crate) trait_impls: TraitImplIndex,
    pub(crate) trait_decls: HashMap<String, LocalTrait>,
    pub(crate) trait_fields: TraitFieldIndex,
    /// SOUNDNESS R562 — see `FileDecls::dyn_trait_fields`.
    pub(crate) dyn_trait_fields: TraitFieldIndex,
    /// SOUNDNESS R897 — the generic-typed fields the impl-bound join found NO impl-level bound for
    /// (struct leaf -> field -> `<position>\u{1f}<param>`), kept so a METHOD's own `where` clause can
    /// bound them inside that method (`decls::resolve_impl_bound_fields`).
    pub(crate) unbound_gen_fields: TraitFieldIndex,
    pub(crate) prim_aliases: std::collections::HashSet<String>,
    pub(crate) extern_fns: std::collections::HashSet<String>,
    pub(crate) drop_types: std::collections::HashSet<String>,
    pub(crate) deref_target: HashMap<String, String>,
    pub(crate) lazy_statics: std::collections::HashSet<String>,
    pub(crate) callable_statics: std::collections::HashSet<String>,
    /// SOUNDNESS R557 — crate-wide `static`/`const` NAME → declared type path, `None` = REFUSED. The
    /// refusal is STICKY across the merge for the same reason it is within a file: a leaf two modules
    /// spell differently must resolve to neither, because a wrong receiver type is a positive claim
    /// about somebody else's body (the R4 fabrication, reached through a module boundary).
    pub(crate) static_types: HashMap<String, Option<String>>,
    pub(crate) callable_aliases: std::collections::HashSet<String>,
    pub(crate) const_strings: HashMap<String, String>,
    pub(crate) local_macros: HashMap<String, String>,
    /// SOUNDNESS R208 — see `FileDecls::macro_twins`. Crate-wide: a name twinned WITHIN one file, plus
    /// one two files spell differently (the `r5`/`r5b` shape, and the commoner one: `#[cfg]` arms in
    /// separate platform modules).
    pub(crate) macro_twins: std::collections::HashSet<String>,
    pub(crate) blanket_methods: HashMap<String, String>,
    /// The crate-ROOT re-exports (`name -> path`, plus the `GLOB_KEY` sentinel), contributed by the root
    /// file. Seeded — under `crate::<name>` keys — into every file's `use` map at Pass B so a `use crate::X`
    /// / `crate::X::foo` in ANY file resolves through the crate-root re-export (`root_reexports`).
    pub(crate) root_reexports: HashMap<String, String>,
    /// SOUNDNESS R186 — the crate ROOT's own declared item names (see `FileDecls::root_decls`). UNIONED
    /// across contributors rather than inserted, because a crate with BOTH `lib.rs` and `main.rs` has two
    /// files at module path "" and each declares its own items — `root_reexports`' plain insert lets one
    /// of the two win, which for THIS field would silently un-protect the other's modules.
    pub(crate) root_decls: std::collections::BTreeSet<String>,
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
    /// SOUNDNESS R145 — every file's `opaque_include_modules`, unioned.
    pub(crate) opaque_include_modules: std::collections::HashSet<String>,
    /// SOUNDNESS R145 — every file's `local_globs`, unioned.
    pub(crate) local_globs: std::collections::HashSet<String>,
    /// R452 — every file's MACRO-HIDDEN TYPE names, unioned. See `FileDecls::macro_hidden_types`.
    pub(crate) macro_hidden_types: std::collections::HashSet<String>,
    /// R452 — every file's MACRO-MENTIONED `fn` names, unioned. See `FileDecls::macro_hidden_fns`.
    pub(crate) macro_hidden_fns: std::collections::HashSet<String>,
    /// ⟨0.39⟩ every file's FOREIGN-abstraction impls, unioned. See `FileDecls::foreign_impls`. Two files
    /// implementing the same foreign member for DIFFERENT types both contribute — the union over them is
    /// what the entry publishes, which is §4's bounded-CHA over-approximation, not a choice between them.
    pub(crate) foreign_impls: HashMap<String, Vec<String>>,
    /// R503 — every file's TRAIT DECLARATION QUALS, unioned. See `FileDecls::trait_quals`. A leaf with
    /// TWO quals is ambiguous and every consumer refuses it, exactly as `LocalTrait::count > 1` already
    /// refuses the same leaf one field over.
    pub(crate) trait_quals: HashMap<String, std::collections::BTreeSet<String>>,
    /// SOUNDNESS R577 — every file's WRITTEN trait quals, merged under the same tombstone rule a single
    /// signature's are (`lang::merge_trait_qual`). See `FileDecls::written_trait_quals`.
    pub(crate) written_trait_quals: HashMap<String, String>,
    /// R529 — every file's BLOCK-NESTED trait-impl members, unioned. See
    /// `FileDecls::nested_impl_members`. Read as a HEDGE (a dispatch on one of these members cannot be
    /// certified from the visible implementors alone), never as an implementor set — the body-local
    /// method has no unit to edge to.
    pub(crate) nested_impl_members: std::collections::HashSet<String>,
    /// R529 — the same, keyed under the OWNING crate for an abstraction this crate does not own.
    pub(crate) nested_impl_foreign: std::collections::HashSet<String>,
    /// R828 — every file's `FileDecls::nonnominal_impls`, unioned.
    pub(crate) nonnominal_impls: std::collections::HashSet<String>,
    /// SOUNDNESS R598 — every file's IMPL-MEMBER EVIDENCE, unioned. See `FileDecls::impl_members` and
    /// `model::impl_seen_key`. Union is the right merge in both directions: two files can write two
    /// `impl Tr for Ty` blocks for two different `Ty`s under one leaf, and a file that cannot read its
    /// own block contributes the `*` key that stops every other file's evidence from narrowing it.
    pub(crate) impl_members: std::collections::HashSet<String>,
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
    // VEIN A — the declaring file's module path too, so a `crate::`-anchored path this re-expansion
    // rewrites keeps its `crate::` head exactly as Pass A wrote it, instead of being stripped back to
    // the relative spelling (which then compares unequal to its anchored twin in `merge_decls`).
    crate::lang::seed_modpath(modpath, &mut uses);
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
    let re_plain = |p: &str| {
        // SOUNDNESS R978 — a field/return typed through a `type A = Arc<T>;` alias re-expands to the
        // POINTEE, as `type_path` peels the written `Arc<T>`. Asked first: the plain alias entry would
        // otherwise rewrite it to the bare wrapper, which has no impl in the crate.
        if let Some(e) = crate::lang::deref_alias_target(p, &uses) {
            if crate::lang::is_deref_wrapper_leaf(&crate::lang::expand(p, &uses)) {
                return Some(e);
            }
        }
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
    let re = |p: &str| -> Option<String> {
        // R893 / R1023 — a LAYERED element keeps its layers through the re-expansion of its leaf.
        if crate::lang::is_wrapped(p) {
            return re_plain(crate::lang::strip_wrapped(p)).map(|e| crate::lang::rewrap(p, &e));
        }
        re_plain(p)
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
    // SOUNDNESS R718 — OWNED WINS, which is the opposite merge from `fields` one loop up and
    // deliberately so. `fields` keeps the LAST contributor's type because a wrong receiver type is a
    // wrong classification either way; this index decides whether a DROP is charged, and the two
    // errors are not symmetric. Under the R213 leaf collision (`a::Inner { v: Vec<Closer> }` beside
    // `b::Inner { n: u32 }`) letting the borrowing declaration win would WITHDRAW the owning twin's
    // charge — a silent under-report — while letting the owning one win keeps the pre-existing,
    // disclosed over-charge. `&=` is that rule: one owning declaration anywhere clears the key.
    for (s, fmap) in &fd.field_borrows {
        let e = acc.field_borrows.entry(s.clone()).or_default();
        for (k, v) in fmap {
            e.entry(k.clone()).or_default().merge(*v);
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
    // `record_amb` — SOUNDNESS R182. The CROSS-FILE half of `record_return`'s conflict recording: two
    // files each declaring one `mk` see no collision of their own, and the withdrawal happens HERE. The
    // candidate has to be filed at both places or a two-file collision (the commonest shape there is)
    // would leave the drop route unable to tell a withdrawal from an absence. Only `rets` records —
    // `enum_tmp` has its own R90 disclosure and no drop route.
    let merge_amb = |dst: &mut HashMap<String, Option<String>>,
                     src: &HashMap<String, Option<String>>,
                     record_amb: bool| {
        for (leaf, val) in src {
            // SOUNDNESS R451 — an impl-METHOD-EXISTS key is a claim that `Type::method` names ONE unit,
            // which is the condition `by_tail2` resolution actually imposes. A second contributor is
            // therefore a WITHDRAWAL, never a re-affirmation: mio-0.8.11 declares `Selector::deregister`
            // in epoll.rs, kqueue.rs AND poll.rs, so the call resolves to nothing and is dropped
            // silently — the very outcome the gate reading this key exists to avoid.
            if crate::model::is_impl_fn_key(leaf) {
                let dup = dst.contains_key(leaf);
                dst.insert(leaf.clone(), if dup { None } else { val.clone() });
                continue;
            }
            // A sentinel entry (`<unit>x`, `<amb>x\x1fT`) is a fact about a key space no candidate
            // recording applies to; it merges like any other value but must never itself be recorded as
            // a candidate, or the key would nest.
            // SOUNDNESS R451 — `is_impl_ret_key` joins the `<amb>` refusal for the same reason: an
            // impl-qualified entry is a fact about a key space no candidate recording applies to, and
            // filing a conflict on one under `amb_ret_key` would NEST one sentinel inside the other and
            // leave `ambiguous_return_leaves` an entry no fn leaf can ever match.
            let plain = record_amb
                && crate::model::split_amb_ret_key(leaf).is_none()
                && !crate::model::is_impl_ret_key(leaf);
            match val {
                None => {
                    // The contributor is already ambiguous, so its OWN candidates ride in as ordinary
                    // `<amb>` entries — but whatever `dst` had recorded under this leaf is about to be
                    // withdrawn here and would otherwise vanish unrecorded.
                    if plain {
                        if let Some(Some(prev)) = dst.get(leaf) {
                            let prev = prev.clone();
                            crate::decls::note_amb_ret(dst, leaf, &prev);
                        }
                    }
                    dst.insert(leaf.clone(), None); // contributor already ambiguous → ambiguous
                }
                Some(tp) => match dst.get(leaf) {
                    None => {
                        dst.insert(leaf.clone(), Some(tp.clone()));
                    }
                    // VEIN A — a CRATE-ANCHORED spelling and another local spelling of one LEAF are not a conflict for `rets`.
                    // Anchoring split what relative spellings used to share: rustix's two backends'
                    // `ret_owned_fd` return `crate::backend::fd::OwnedFd` and
                    // `crate::backend::libc::fd::OwnedFd`, which are the same re-exported `OwnedFd`
                    // (cfg arms of one polyfill); withdrawing the leaf cost R165's construct marker and the
                    // `OwnedFd::drop` edge in four rustix versions. Every reader of `rets` keys on the
                    // type's LEAF (`local_type_leaf`, `local_types`), so agreeing on it is agreeing.
                    Some(Some(prev))
                        if prev != tp
                            && record_amb
                            && (prev.starts_with("crate::") || tp.starts_with("crate::"))
                            // both MODULE-QUALIFIED local paths — a bare leaf beside an anchored one
                            // (`CompactString` / `crate::CompactString`) conflicted before anchoring
                            // existed and still does, so this restores only what anchoring split
                            && prev.trim_start_matches("crate::").contains("::")
                            && tp.trim_start_matches("crate::").contains("::")
                            && !matches!(prev.split("::").next(), Some("std" | "core" | "alloc"))
                            && !matches!(tp.split("::").next(), Some("std" | "core" | "alloc"))
                            && !prev.contains(crate::decls::ALIAS_ALT_SEP)
                            && !tp.contains(crate::decls::ALIAS_ALT_SEP)
                            && prev.rsplit("::").next() == tp.rsplit("::").next() => {}
                    Some(Some(prev)) if prev != tp => {
                        if plain {
                            let prev = prev.clone();
                            crate::decls::note_amb_ret(dst, leaf, &prev);
                            crate::decls::note_amb_ret(dst, leaf, tp);
                        }
                        dst.insert(leaf.clone(), None); // conflicting types — drop
                    }
                    Some(Some(_)) => {} // same type — keep
                    Some(None) => {
                        // Already ambiguous — still record, or a THIRD contributing file's candidate
                        // would be invisible to the disclosure.
                        if plain {
                            crate::decls::note_amb_ret(dst, leaf, tp);
                        }
                    }
                },
            }
        }
    };
    merge_amb(&mut acc.rets, &fd.rets, true);
    merge_amb(&mut acc.enum_tmp, &fd.enum_tmp, false);
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
    for (tr, assoc) in &fd.trait_assoc {
        acc.trait_decls.entry(tr.clone()).or_default().assoc.extend(assoc.iter().cloned()); // set union (R776)
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
    for (s, fmap) in &fd.dyn_trait_fields {
        let e = acc.dyn_trait_fields.entry(s.clone()).or_default();
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
    for n in &fd.opaque_include_modules {
        acc.opaque_include_modules.insert(n.clone()); // set union — order-independent (R145)
    }
    for n in &fd.local_globs {
        acc.local_globs.insert(n.clone()); // set union — order-independent (R145)
    }
    for n in &fd.macro_hidden_types {
        acc.macro_hidden_types.insert(n.clone()); // set union — order-independent (R452)
    }
    for n in &fd.macro_hidden_fns {
        acc.macro_hidden_fns.insert(n.clone()); // set union — order-independent (R452)
    }
    for n in &fd.nonnominal_impls {
        acc.nonnominal_impls.insert(n.clone()); // set union — order-independent (R828)
    }
    for n in &fd.nested_impl_members {
        acc.nested_impl_members.insert(n.clone()); // set union — order-independent (R529)
    }
    for n in &fd.nested_impl_foreign {
        acc.nested_impl_foreign.insert(n.clone()); // set union — order-independent (R529)
    }
    for n in &fd.impl_members {
        acc.impl_members.insert(n.clone()); // set union — order-independent (R598)
    }
    for (k, v) in &fd.foreign_impls {
        // ⟨0.39⟩ UNION, never last-writer-wins: two files may implement the same foreign member for
        // different types and the entry publishes the union over both (§4 bounded CHA). Picking one would
        // withdraw the other's effect from every consumer — the direction this rung exists to close.
        let e = acc.foreign_impls.entry(k.clone()).or_default();
        for q in v {
            if !e.contains(q) {
                e.push(q.clone());
            }
        }
    }
    for (k, v) in &fd.trait_quals {
        // R503 — set union, so a leaf declared in two modules carries BOTH quals and is refused rather
        // than resolved to whichever file was merged last.
        let e = acc.trait_quals.entry(k.clone()).or_default();
        for q in v {
            e.insert(q.clone());
        }
    }
    for (k, v) in &fd.written_trait_quals {
        // SOUNDNESS R577 — ONE rule for the merge and for a single signature's own collisions: two files
        // spelling one leaf with two crates tombstones it, exactly as two parameters of one fn do. A
        // tombstone in EITHER input is sticky, because it already means "this crate binds the leaf two
        // ways" and a second file agreeing with one of them does not settle it.
        crate::lang::merge_trait_qual(&mut acc.written_trait_quals, k.clone(), v.clone());
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
    // SOUNDNESS R557 — the cross-FILE half of `collect_static_types`' within-file rule, and the same
    // rule: agreement keeps the type, disagreement refuses the NAME, and a refusal is sticky. Written as
    // a match rather than `insert` so the merge is ORDER-INDEPENDENT — whichever file lands first, two
    // differing declarations end at `None`.
    for (k, v) in &fd.static_types {
        match acc.static_types.get(k) {
            None => {
                acc.static_types.insert(k.clone(), v.clone());
            }
            Some(prev) if prev == v => {}
            Some(_) => {
                acc.static_types.insert(k.clone(), None);
            }
        }
    }
    for n in &fd.callable_aliases {
        acc.callable_aliases.insert(n.clone()); // set union — order-independent
    }
    for (k, v) in &fd.const_strings {
        acc.const_strings.insert(k.clone(), v.clone()); // leaf → literal; last-writer-wins on a rare collision
    }
    for (k, v) in &fd.local_macros {
        // SOUNDNESS R208 — the collision is no longer "rare" by assertion: 324 of 1,504 corpus crates
        // define a `macro_rules!` name more than once (`#[cfg]` twins — anyhow, aho-corasick,
        // async-compression …). Last-writer-wins STAYS, so nothing this file already charges is
        // withdrawn; the name is recorded so an invocation of it can disclose.
        if acc.local_macros.get(k).is_some_and(|prev| prev != v) {
            acc.macro_twins.insert(k.clone());
        }
        acc.local_macros.insert(k.clone(), v.clone()); // macro NAME → arm tokens; last-writer-wins on a rare collision
    }
    acc.macro_twins.extend(fd.macro_twins.iter().cloned());
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
    // R186 — UNION, not insert: `lib.rs` and `main.rs` are both at module path "" and both contribute.
    acc.root_decls.extend(fd.root_decls.iter().cloned());
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
    // SOUNDNESS R718 — `field_borrows` is `fields`' ownership twin and steers whether a construction is
    // charged drop glue, so it belongs in the digest for the same reason `fields` does: a Pass-B FnInfo
    // is only reusable if the index it was derived from has not moved. Rendered through the same
    // `nested` shape with the bool stringified, so the key ORDER cannot differ between the two.
    nested(&mut s, "field_borrows",
           &m.field_borrows.iter()
               .map(|(k, v)| (k.clone(),
                              v.iter().map(|(f, b)| (f.clone(), format!("{b:?}"))).collect()))
               .collect());
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
        let mut asc: Vec<&String> = lt.assoc.iter().collect();
        asc.sort();
        for aname in asc {
            s.push('&'); // R776 — associated fns; a distinct sigil so `fn a` and `fn a(&self)` differ
            s.push_str(aname);
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
    // SOUNDNESS R562 — the dyn-only twin changes which receivers CHA, so it must invalidate too.
    s.push_str("dyn_trait_fields");
    nested_tf(&mut s, &m.dyn_trait_fields);
    s.push_str("unbound_gen_fields");
    nested_tf(&mut s, &m.unbound_gen_fields);
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
    // opaque_include_modules + local_globs — SOUNDNESS R145. Read only at the call resolver in `scan.rs`,
    // which runs every scan, so no cached FnInfo depends on them today; folded in anyway because a
    // resolver-time fact that moves without moving the digest is the shape this function exists to stop
    // the day one of them is read in Pass B.
    for (name, set) in [("opaque_include_modules", &m.opaque_include_modules), ("local_globs", &m.local_globs)] {
        s.push_str(name);
        let mut v: Vec<&String> = set.iter().collect();
        v.sort();
        for a in v {
            s.push('|');
            s.push_str(a);
        }
        s.push('\n');
    }
    // macro_hidden_types — R452, sorted set of type names declared in one of those modules. Read at the
    // CALL RESOLVER for the same reason `macro_modules` is, and with the same cross-file consequence: a
    // file that gains or loses an item-position macro changes which TYPES hedge, everywhere.
    s.push_str("macro_hidden_types");
    let mut mhk: Vec<&String> = m.macro_hidden_types.iter().collect();
    mhk.sort();
    for a in mhk {
        s.push('|');
        s.push_str(a);
    }
    s.push('\n');
    // macro_hidden_fns — R452, the other half of that hedge's evidence.
    s.push_str("macro_hidden_fns");
    let mut mhf: Vec<&String> = m.macro_hidden_fns.iter().collect();
    mhf.sort();
    for a in mhf {
        s.push('|');
        s.push_str(a);
    }
    s.push('\n');
    // nested_impl_members / nested_impl_foreign — R529. Both decide whether a dispatch is certified or
    // hedged, so a file that gains or loses a body-local impl changes the verdict for its whole crate
    // AND for every chained consumer of the owning one.
    // impl_members — R598. Decides whether `{ty}::{method}` is read as this trait's implementation at
    // all, so a file that gains or loses an impl member changes what the interface-union publishes for
    // the whole crate and for every chained consumer of it.
    for (label, set) in [
        ("nested_impl_members", &m.nested_impl_members),
        ("nested_impl_foreign", &m.nested_impl_foreign),
        ("nonnominal_impls", &m.nonnominal_impls),
        ("impl_members", &m.impl_members),
    ] {
        s.push_str(label);
        let mut v: Vec<&String> = set.iter().collect();
        v.sort();
        for a in v {
            s.push('|');
            s.push_str(a);
        }
        s.push('\n');
    }
    // foreign_impls — ⟨0.39⟩, the abstractions this crate implements but does not own. Read at BOTH ends
    // of the rung (the published foreign union entry, and the consumer-side edge to its own implementors),
    // so a file that gains or loses such an impl changes what every consumer of the owning crate sees.
    s.push_str("foreign_impls");
    let mut fik: Vec<&String> = m.foreign_impls.keys().collect();
    fik.sort();
    for k in fik {
        s.push('|');
        s.push_str(k);
        let mut v: Vec<&String> = m.foreign_impls[k].iter().collect();
        v.sort();
        for q in v {
            s.push(',');
            s.push_str(q);
        }
    }
    s.push('\n');
    // trait_quals — R503. Decides the WIRE spelling of every local interface-union entry key and of
    // `dispatchesOn`, so a file that moves a trait between modules changes what consumers can join.
    s.push_str("written_trait_quals");
    let mut wtq: Vec<&String> = m.written_trait_quals.keys().collect();
    wtq.sort();
    for k in wtq {
        s.push('|');
        s.push_str(k);
        s.push(',');
        s.push_str(&m.written_trait_quals[k]);
    }
    s.push('\n');
    s.push_str("trait_quals");
    let mut tqk: Vec<&String> = m.trait_quals.keys().collect();
    tqk.sort();
    for k in tqk {
        s.push('|');
        s.push_str(k);
        for q in &m.trait_quals[k] {
            s.push(',');
            s.push_str(q);
        }
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
    // SOUNDNESS R557 — static_types: sorted NAME=type (or NAME=<refused>) pairs. A change here changes
    // which RECEIVERS resolve, so it must invalidate cached FnInfos exactly like `callable_statics`. The
    // refusal is written explicitly rather than omitted: "this leaf is ambiguous" and "this file declares
    // no such static" are different inputs and must not hash the same.
    s.push_str("static_types");
    let mut stk: Vec<&String> = m.static_types.keys().collect();
    stk.sort();
    for k in stk {
        s.push('|');
        s.push_str(k);
        s.push('=');
        s.push_str(m.static_types[k].as_deref().unwrap_or("<refused>"));
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
    s.push_str("macro_twins");
    let mut mtw: Vec<&String> = m.macro_twins.iter().collect();
    mtw.sort();
    for k in mtw {
        s.push('\u{1f}');
        s.push_str(k);
    }
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
    // R186 — root_decls, sorted. Seeded into every file's `use` map and it DECIDES whether a
    // `crate::<name>::…` path is glob-attributed, so adding or removing a root `mod`/type re-resolves
    // types in OTHER files: same invalidation argument as `root_reexports` one block up.
    s.push_str("root_decls");
    for k in &m.root_decls {
        s.push('|');
        s.push_str(k);
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
