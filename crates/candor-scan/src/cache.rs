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
    format!("scan-{}/rev6/tests={}", env!("CARGO_PKG_VERSION"), include_tests)
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
    pub(crate) trait_impls: TraitImplIndex,
    /// `trait leaf -> (decl count in this file, declared method names)` — `LocalTrait` flattened for serde.
    pub(crate) trait_decls: HashMap<String, (usize, Vec<String>)>,
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
    /// CONST/STATIC string NAMES → their LITERAL value in this file (`const API_BASE: &str = "…"`) — a
    /// call building its host from one (`post(format!("{}/x", API_BASE))`) resolves the host here (SPEC §1
    /// static-host propagation). Literal-valued only.
    #[serde(default)]
    pub(crate) const_strings: HashMap<String, String>,
    /// The crate-ROOT re-exports, populated ONLY for the root file (`lib.rs`/`main.rs`, module path "").
    /// `name -> resolved path` plus the `GLOB_KEY` sentinel for a `pub use x::prelude::*`. Seeded into every
    /// file's `use` map under `crate::<name>` so a submodule's `use crate::net` / `crate::net::foo` resolves
    /// through the crate-root re-export it can't otherwise see (files are scanned with per-file `use` maps).
    /// See `collect_root_reexports`. Empty for a non-root file.
    #[serde(default)]
    pub(crate) root_reexports: HashMap<String, String>,
}

/// Collect ONE file's Pass A decls in isolation (the per-file input to `merge_decls`). `modpath` is the
/// file's module path — the ROOT file (`""`) additionally contributes its crate-root re-exports, a
/// crate-wide fact seeded into every file's `use` map (see `root_reexports`).
pub(crate) fn file_decls(items: &[syn::Item], include_tests: bool, modpath: &str) -> FileDecls {
    let mut uses = HashMap::new();
    let mut fields = HashMap::new();
    let mut field_elem = HashMap::new();
    let mut field_elem_trait = HashMap::new();
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
    let mut const_strings = HashMap::new();
    collect_decls(items, include_tests, &mut uses, &mut fields, &mut field_elem, &mut field_elem_trait, &mut rets,
                  &mut enum_tmp, &mut trait_impls, &mut trait_decls, &mut trait_fields, &mut prim_aliases,
                  &mut extern_fns, &mut drop_types, &mut deref_target, &mut lazy_statics, &mut const_strings);
    FileDecls {
        fields,
        field_elem,
        field_elem_trait,
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
        const_strings,
        // Crate-root re-exports are a ROOT-file fact only; a submodule's `use crate::X` seeds against them.
        root_reexports: if modpath.is_empty() { collect_root_reexports(items) } else { HashMap::new() },
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
    pub(crate) trait_impls: TraitImplIndex,
    pub(crate) trait_decls: HashMap<String, LocalTrait>,
    pub(crate) trait_fields: TraitFieldIndex,
    pub(crate) prim_aliases: std::collections::HashSet<String>,
    pub(crate) extern_fns: std::collections::HashSet<String>,
    pub(crate) drop_types: std::collections::HashSet<String>,
    pub(crate) deref_target: HashMap<String, String>,
    pub(crate) lazy_statics: std::collections::HashSet<String>,
    pub(crate) const_strings: HashMap<String, String>,
    /// The crate-ROOT re-exports (`name -> path`, plus the `GLOB_KEY` sentinel), contributed by the root
    /// file. Seeded — under `crate::<name>` keys — into every file's `use` map at Pass B so a `use crate::X`
    /// / `crate::X::foo` in ANY file resolves through the crate-root re-export (`root_reexports`).
    pub(crate) root_reexports: HashMap<String, String>,
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
    for (k, v) in &fd.const_strings {
        acc.const_strings.insert(k.clone(), v.clone()); // leaf → literal; last-writer-wins on a rare collision
    }
    for (k, v) in &fd.root_reexports {
        // Only the ROOT file populates this, so there is at most one contributor — a plain insert.
        acc.root_reexports.insert(k.clone(), v.clone());
    }
}

/// A CANONICAL, order-stable digest of the merged decl index — the gate that decides whether a cached
/// file's FnInfos (Pass B output) are still valid. Every map is rendered with SORTED keys (and sorted
/// inner keys / value lists) so the digest depends only on the index's CONTENT, never on `HashMap`
/// iteration order or which files happened to contribute. If this digest is unchanged, every fn's
/// `Call`s resolve identically, so a cached FnInfo set is sound to reuse; if it moves, all files re-run
/// Pass B. `trait_impls`'s Vec order is load-bearing (CHA), so it is hashed in order, NOT sorted.
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
#[derive(serde::Serialize, serde::Deserialize)]
pub(crate) struct FileCache {
    pub(crate) content_hash: String,
    pub(crate) decls: FileDecls,
    pub(crate) decl_index_hash: String,
    pub(crate) fninfos: Vec<FnInfo>,
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
