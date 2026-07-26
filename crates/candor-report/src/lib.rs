//! Shared candor report types and parsing — **no `rustc_private`**, so both the lint (which writes
//! reports) and the CLI / tooling (which read them) depend on one definition instead of re-deriving
//! the JSON shape in every script. This is the type-safe, DRY core the bash+Python tooling lacked.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// The ten effects candor classifies (excluding the synthetic `Unknown`). Defined once here so the
/// lint and the CLI share one vocabulary instead of each keeping its own copy (which had already
/// drifted in ordering). Order is irrelevant to consumers — both tally by name and sort the output.
pub const EFFECTS: [&str; 11] =
    ["Net", "Db", "Llm", "Fs", "Exec", "Ipc", "Env", "Clock", "Rand", "Clipboard", "Log"];

/// A discovered per-crate report file. candor's report naming convention is
/// `<prefix>.<crate>.<type>.json`; the sidecars (`<prefix>.calibrated.json`,
/// `<prefix>.encountered-*.json`) have only ONE segment after the prefix and are NOT reports.
pub struct ReportFile {
    pub path: PathBuf,
    /// The `<crate>` segment of the filename.
    pub krate: String,
    /// The `<type>` segment (e.g. `lib`, `Executable`).
    pub kind: String,
}

/// Discover the per-crate report files for a prefix (`.candor/report` →
/// `.candor/report.<crate>.<type>.json`), sorted by path for deterministic output. A directoryless
/// prefix reads the current directory. ONE discrimination rule — `<crate>.<type>`, exactly two
/// segments — shared by the lint's cross-crate loader and the CLI's queries, so the two can never
/// disagree about which files are reports.
pub fn report_files(prefix: &str) -> Vec<ReportFile> {
    let p = Path::new(prefix);
    // A locator that is an existing FILE ending `.json` is a DIRECT single-report reference (SPEC
    // §3.3.1: "a path ending `.json` → that single report file loaded directly", ANY filename, whatever
    // its internal dot-segments — so one engine can query another's report by path). It is NOT globbed
    // as a prefix: return exactly that file. `<crate>.<type>` are parsed best-effort from the name (for
    // `report_backend`/`audit` labelling) and default to the whole stem / `""` when the name isn't the
    // canonical `<base>.<crate>.<type>.json` shape.
    if prefix.ends_with(".json") && p.is_file() {
        let stem = p.file_name().and_then(|s| s.to_str()).and_then(|n| n.strip_suffix(".json")).unwrap_or("");
        let (krate, kind) = match stem.rsplit_once('.') {
            Some((rest, kind)) => (rest.rsplit_once('.').map(|(_, k)| k).unwrap_or(rest).to_string(), kind.to_string()),
            None => (stem.to_string(), String::new()),
        };
        return vec![ReportFile { path: p.to_path_buf(), krate, kind }];
    }
    let dir = p
        .parent()
        .filter(|d| !d.as_os_str().is_empty())
        .map(Path::to_path_buf)
        .unwrap_or_else(|| PathBuf::from("."));
    let Some(base) = p.file_name().and_then(|s| s.to_str()) else { return Vec::new() };
    let prefix_dot = format!("{base}.");
    let mut out = Vec::new();
    let Ok(rd) = std::fs::read_dir(&dir) else { return out };
    for ent in rd.flatten() {
        let name = ent.file_name();
        let Some(name) = name.to_str() else { continue };
        let Some(rest) = name.strip_prefix(&prefix_dot) else { continue };
        let Some(rest) = rest.strip_suffix(".json") else { continue };
        // `<crate>.<type>` — exactly two segments; a one-segment sidecar or any 3+-segment name is not
        // a report.
        let mut segs = rest.splitn(2, '.');
        let (Some(krate), Some(kind)) = (segs.next(), segs.next()) else { continue };
        // both segments must be non-empty (`<base>.<crate>..json` would otherwise parse to an empty
        // `kind`) and `kind` must itself be a single segment (no further dots).
        if krate.is_empty() || kind.is_empty() || kind.contains('.') {
            continue;
        }
        out.push(ReportFile { path: ent.path(), krate: krate.to_string(), kind: kind.to_string() });
    }
    out.sort_by(|a, b| a.path.cmp(&b.path));
    out
}

/// One function entry of a candor report. `#[serde(default)]` on the non-essential fields so a
/// partial or legacy report still deserializes; the lint sets them all when writing.
#[derive(Serialize, Deserialize, Default, Clone, Debug)]
pub struct ReportEntry {
    #[serde(rename = "fn")]
    pub func: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub loc: String,
    // `inferred` (the effect-set payload) is kept ALWAYS present for clarity even when empty — every
    // other field below is omitted when empty/default (`#[serde(default)]` means a reader defaults it,
    // so omission is wire-compatible). The reconciliation trio (declared/undeclared/overdeclared) is
    // populated only by the declaration-reconciliation pass (the deep/JVM engines); it is ALWAYS empty
    // in the stable scanner's reports, where serializing it cost ~15% of the bytes for nothing.
    #[serde(default)]
    pub inferred: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub direct: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub declared: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub undeclared: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub overdeclared: Vec<String>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub unresolved: bool,
    /// True if the RUNTIME invokes this function rather than (only) project code — a reachability ROOT
    /// with no in-project caller (`main`, a `#[test]`, a `#[no_mangle]`/exported fn). candor-spec §2
    /// `entryPoint`. Far richer on a reflection/framework runtime (the JVM port marks Spring/servlet
    /// callbacks); on Rust it's the language's external-invocation surface. Omitted when false.
    #[serde(default, rename = "entryPoint", skip_serializing_if = "std::ops::Not::not")]
    pub entry_point: bool,
    /// Stable cross-crate identity (hex `DefPathHash`); empty in older reports.
    #[serde(default)]
    pub hash: String,
    /// Filesystem access detail when the `Fs` effect's verbs revealed it: `["read"]`, `["write"]`, or
    /// both. A non-breaking refinement (the `Fs` effect itself is unchanged); omitted when unknown or
    /// when the function performs no `Fs`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub fs: Vec<String>,
    /// Literal Net endpoints statically visible from this function (`host[:port]`, scheme/path
    /// stripped) — the decidable subset of "who does it talk to". A non-breaking refinement of `Net`;
    /// omitted when none are visible (a runtime-computed address, or no `Net` at all). Never a
    /// completeness claim — host-by-runtime-value is undecidable, so absence ≠ "no network".
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub hosts: Vec<String>,
    /// Literal subprocess commands statically visible from this function (the program name passed to
    /// `Command::new("…")`). The decidable subset of "what does it run". A non-breaking refinement of
    /// `Exec`; omitted when none are visible (a runtime-computed command, or no `Exec`). Never a
    /// completeness claim — command-by-runtime-value is undecidable, so absence ≠ "runs nothing".
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub cmds: Vec<String>,
    /// Literal filesystem paths statically visible from this function (the path passed to a built-in
    /// `Fs` call). The decidable subset of "what does it touch". A non-breaking refinement of `Fs`;
    /// omitted when none are visible (a runtime-computed path, or no `Fs`). Never a completeness claim.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub paths: Vec<String>,
    /// Literal database tables statically visible from this function (table-position identifiers in
    /// a SQL string literal — `FROM`/`JOIN`/`INTO`/leading `UPDATE`…). The decidable subset of "what
    /// data does it touch". A non-breaking refinement of `Db`; omitted when none are visible (a
    /// dynamically-built query, an ORM call with no SQL literal, or no `Db`). Never a completeness
    /// claim — table-by-runtime-value is undecidable, so absence ≠ "touches nothing".
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tables: Vec<String>,
    /// Effectful local functions this one calls — the effect-relevant call graph ("who calls X?").
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub calls: Vec<String>,
    /// Why this function introduces `Unknown` DIRECTLY (candor-spec §2 `unknownWhy`): an origin tag per
    /// unresolvable site — `dispatch:<trait>` (a `dyn`/effectful-trait call with no visible impl),
    /// `callback:<fn-pointer / closure>` (an unresolvable indirect call). Lets a consumer tell the
    /// improvable kind (a dispatch that would resolve with more inputs) from the irreducible. Omitted
    /// when the function introduces no direct `Unknown`.
    #[serde(default, rename = "unknownWhy", skip_serializing_if = "Vec::is_empty")]
    pub unknown_why: Vec<String>,
    /// The external crates/packages this function (transitively) reaches that the classifier could NOT see
    /// through — κ floored them and never classified them anywhere. Effects through them are NOT in
    /// `inferred`, so this is the per-fn HONESTY caveat: `inferred` is a LOWER BOUND when this is non-empty
    /// (`inferred: []` with a non-empty `invisible` means "pure as far as candor could see, but it could
    /// not see through these"). The per-scan κ line is the same disclosure aggregated. Omitted when none.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub invisible: Vec<String>,
    /// Effects (Net/Exec/Fs/Db) whose literal SURFACE this function leaves INCOMPLETE — a host-/command-
    /// establishing call performed with a runtime (non-literal) locator, so the endpoint is invisible to
    /// the AS-EFF-008 allowlist. Carried in the report ONLY so a CANDOR_DEPS consumer inherits the
    /// incompleteness across the crate boundary (else a benign literal in the consumer could mask the
    /// dep's invisible forbidden endpoint — sweep [30]). Omitted when none; consumed by the gate, not a
    /// primary surface.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub incomplete: Vec<String>,
    /// ⟨0.20⟩ Net DESTINATION classes present in this function's (transitive) `Net` surface —
    /// `known-telemetry` / `known-partner` / `unknown-host` (NET-DESTINATION-CLASS-DESIGN.md). An exact
    /// host-literal match for the visible hosts, plus the fail-closed `unknown-host` when the Net surface is
    /// masked (`incomplete` has `Net`) OR carries no visible host (a runtime endpoint). The class travels the
    /// call graph like the effect. Omitted when the function has no `Net`; never a claim a host is SAFE.
    #[serde(default, rename = "netClass", skip_serializing_if = "Vec::is_empty")]
    pub net_class: Vec<String>,
    /// ⟨workspace-chain⟩ True on a synthetic TRAIT-CHA union entry — `crate#Trait::method` whose effects are
    /// the UNION over local impls, emitted (gated behind CANDOR_WORKSPACE_CHAIN) so a cross-crate consumer's
    /// trait-dispatch call resolves via chaining instead of reading pure. NOT an analyzed unit; omitted when
    /// false. See WORKSPACE-CHAINING-DESIGN.md.
    #[serde(default, rename = "interfaceUnion", skip_serializing_if = "std::ops::Not::not")]
    pub interface_union: bool,
}

/// The candor-spec contract version this build implements (the report SCHEMA + AS-EFF codes), distinct
/// from the engine build id (`ReportMeta::version`) and from the crate release version. Bumped only when
/// the spec contract changes; emitted as the envelope's `spec` so a consumer can see which contract a
/// report conforms to. Both backends and the JVM port declare the SAME value — see candor-spec §2.1.
pub const SPEC_VERSION: &str = "0.23";

/// The envelope header: which engine produced the report (`version` = build id, `toolchain`), and which
/// candor-spec contract it implements (`spec`).
#[derive(Serialize, Deserialize, Default, Clone, Debug)]
pub struct ReportMeta {
    pub version: String,
    #[serde(default)]
    pub toolchain: String,
    /// candor-spec contract version (e.g. `"0.6"`). `#[serde(default)]` so a legacy report without it
    /// still parses (absent ⇒ pre-spec-field, treat as ≤ 0.2).
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub spec: String,
}

/// ⟨0.15 staged⟩ One uncovered package of the κ-coverage ledger: an external package/module this
/// code demonstrably calls that the classifier could not see through, with the call-site count as
/// the engine counts it. Same names/counts as the per-scan stderr disclosure — this is that line
/// as data.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct CoverageEntry {
    pub name: String,
    pub calls: usize,
}

/// ⟨0.15 staged⟩ The `coverage` envelope field (spec §2): the κ-coverage ledger (§7 item 14)
/// travelling WITH the report instead of evaporating on stderr. `uncovered` effects are INVISIBLE
/// to the scan — absent from the report, NOT a claim they're pure. OMITTED entirely when nothing
/// is uncovered (the `extensions`-field precedent), so a fully-covered scan's report is
/// byte-identical to a pre-⟨0.15⟩ one.
#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq, Eq)]
pub struct Coverage {
    pub uncovered: Vec<CoverageEntry>,
}

/// ⟨proposed — COMPLETENESS-MANIFEST-DESIGN.md, Gap 2⟩ One unit of the TARGET's own source that candor
/// could NOT analyze — a file that failed to read/parse, or a scope it skipped. Its effects are absent
/// from the report NOT because they're pure but because the code was never seen. Disclosed on stderr
/// today (and the gate fails exit 2 when a policy is configured), but INVISIBLE to a machine reading the
/// JSON report — so a `--json` consumer saw a report that looked complete. This carries it into the wire.
/// Distinct from `coverage` (an unmodeled *dependency*): `unanalyzed` is the target's own unseen source.
#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq, Eq)]
pub struct UnanalyzedUnit {
    pub path: String,
    pub reason: String,
}

/// ⟨0.22⟩ COMPLETENESS MANIFEST (Gap 1): the analyzed-universe summary. `count` = the functions candor
/// formed an effect judgment for (effectful + pure) = the §2.2 callgraph node set — so a consumer reading
/// the bare envelope computes `count − |functions|` = the pure count and tells analyzed-pure from
/// never-seen without loading the sidecar. `digest` = an opaque within-engine-stable fingerprint of the
/// sorted analyzed-qual set (FNV-1a-64 hex — see [`fnv1a_hex`]); a same-input re-scan agrees, compare
/// same-engine only.
#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq, Eq)]
pub struct Analyzed {
    pub count: usize,
    pub digest: String,
}

/// ⟨0.22⟩ An opaque, within-engine-stable fingerprint of a SORTED qual set — FNV-1a 64-bit over the
/// newline-joined UTF-8 quals, lowercase hex (16 chars). Dependency-free + deterministic: it changes iff
/// the set changes. Byte-for-byte the java reference's `ReportWriter.fnv1aHex` so the SPEC describes ONE
/// algorithm. NOT cryptographic and NOT cross-engine comparable (qualifiers differ `::` vs `.`).
pub fn fnv1a_hex(sorted_quals: &[String]) -> String {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325; // FNV offset basis
    for q in sorted_quals {
        for b in q.as_bytes() {
            h ^= *b as u64;
            h = h.wrapping_mul(0x100_0000_01b3); // FNV prime
        }
        h ^= b'\n' as u64;
        h = h.wrapping_mul(0x100_0000_01b3);
    }
    format!("{h:016x}")
}

/// The v0.2 self-describing report: a provenance header plus the function entries.
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Report {
    pub candor: ReportMeta,
    /// ⟨0.15 staged⟩ the optional coverage ledger; `None` (omitted) when fully covered. `default`
    /// so every pre-⟨0.15⟩ report still deserializes.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub coverage: Option<Coverage>,
    /// ⟨proposed⟩ the target source candor couldn't analyze (Gap 2). Empty (omitted) on a complete
    /// scan — wire-compatible with a pre-rung report. `default` so older reports still deserialize.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub unanalyzed: Vec<UnanalyzedUnit>,
    /// ⟨proposed: typeSurface⟩ The minimum TYPE information a consumer needs to form a key it otherwise
    /// cannot. Omitted when empty, so a report with nothing to say is byte-identical to a pre-rung one
    /// and a consumer that ignores the block behaves exactly as today (tier-1 additive — the rule that
    /// let `interfaceUnion` ride gated).
    #[serde(rename = "typeSurface", default, skip_serializing_if = "Option::is_none")]
    pub type_surface: Option<TypeSurface>,
    pub functions: Vec<ReportEntry>,
}

/// ⟨proposed⟩ candor-spec/DEP-RECEIVER-TYPING-DESIGN.md, half 2.
///
/// A PURE FACTORY IS ABSENT FROM THE REPORT ENTIRELY — reports omit pure functions — so no field added
/// to a function *entry* could carry this; there is no entry to put it on. That is why the type surface
/// is a separate envelope block, and it is the reason the item stalled as long as it did.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct TypeSurface {
    /// `<fn id>` -> `<type id>`, BOTH FULLY QUALIFIED: `deplib#sync::build` -> `deplib#sync::Client`.
    ///
    /// THE QUALIFICATION IS THE MECHANISM, not a naming detail. The reverted attempt published
    /// `{crate}#{leaf}` on both ends, so `sync::Client` and `mock::Client` in one dependency were the
    /// same string and a PURE `mock_client()` factory let `sync::Client::send`'s `Net` be charged to a
    /// caller that cannot reach it. The type id is a PREFIX of that type's real entry hashes, so the
    /// consumer forms `<type id>::<method>` and asks the dep index for exactly that.
    ///
    /// BOUNDED to types with at least one non-pure member in the same report: if the returned type has
    /// no effectful and no `Unknown`-carrying member, typing the receiver changes no answer — the lookup
    /// it enables succeeds and yields pure, which is what silence already yields.
    #[serde(default, skip_serializing_if = "std::collections::BTreeMap::is_empty")]
    pub returns: std::collections::BTreeMap<String, String>,
}

/// Parse a report's function entries, accepting BOTH the v0.2 envelope `{ candor, functions }` and
/// the legacy v0.1 bare array `[...]` (the migration contract — candor-spec §2). An envelope is a
/// JSON object, so a bare array fails that parse and falls through; the two forms are unambiguous.
pub fn report_entries(text: &str) -> Option<Vec<ReportEntry>> {
    report_entries_counted(text).map(|(entries, _dropped)| entries)
}

/// Like [`report_entries`] but also returns how many entries FAILED to deserialize and were dropped.
/// Each entry is deserialized INDEPENDENTLY (via raw `Value`s), skipping any that fail — so one
/// malformed entry (a partial write, a hand-edit, an entry whose `inferred` is a string not an array)
/// loses only ITSELF, not the whole crate's report. But a SILENT per-entry drop is the same kind of
/// under-report a whole-file parse failure is — and the latter IS disclosed by callers. So this variant
/// surfaces the drop count: a caller MUST disclose `dropped > 0` (a vanished function's effects read as
/// pure otherwise — exactly the "never silently pure" failure the gate exists to prevent).
pub fn report_entries_counted(text: &str) -> Option<(Vec<ReportEntry>, usize)> {
    let val: serde_json::Value = serde_json::from_str(text).ok()?;
    let arr = val
        .get("functions")
        .and_then(|f| f.as_array())
        .or_else(|| val.as_array())?;
    let mut dropped = 0usize;
    let entries = arr
        .iter()
        .filter_map(|e| match serde_json::from_value::<ReportEntry>(e.clone()) {
            Ok(entry) => Some(entry),
            Err(_) => {
                dropped += 1;
                None
            }
        })
        .collect();
    Some((entries, dropped))
}

/// Serialize a v0.2 report from a header + entries, borrowing both so the caller keeps ownership
/// (the lint logs the entry count after writing). Pretty-printed.
pub fn to_report_json(candor: &ReportMeta, functions: &[ReportEntry]) -> serde_json::Result<String> {
    to_packaged_report_json(candor, "", functions)
}

/// Write a report file ATOMICALLY: serialize to a sibling temp file, then `rename` it into place.
/// Both backends (the lint and the stable scanner) write reports that a concurrent reader — a `cargo
/// candor` query, or a `watch` loop re-scanning while a query reads — may open at any moment. An
/// in-place `fs::write` leaves a window where the reader observes a half-written file and its JSON
/// parse fails (which `load_entries` then silently skips → the report's effectful functions read as
/// "no effect", a silent under-report against the never-silently-pure promise). `rename(2)` is atomic
/// within a filesystem, so a reader sees either the old report or the new one whole. The temp name
/// carries the PID so two concurrent writers (unlikely, but cheap to make safe) don't collide. Falls
/// back to nothing on error — callers already tolerate a failed write (they only `eprintln!`).
pub fn write_atomic(path: &Path, contents: &[u8]) -> std::io::Result<()> {
    let tmp = path.with_extension(format!("tmp.{}", std::process::id()));
    std::fs::write(&tmp, contents)?;
    std::fs::rename(&tmp, path)
}

/// Like [`to_report_json`], with the envelope's `package` field (spec §2, 0.4-amended SHOULD):
/// name what the report covers, so an all-pure EMPTY report's coverage is readable without
/// parsing entry hash prefixes. An empty `package` omits the field (the pre-amendment shape).
pub fn to_packaged_report_json(
    candor: &ReportMeta,
    package: &str,
    functions: &[ReportEntry],
) -> serde_json::Result<String> {
    to_packaged_report_json_with_coverage(candor, package, functions, None)
}

/// ⟨0.15 staged⟩ Like [`to_packaged_report_json`], with the optional `coverage` envelope field
/// (spec §2): the κ-coverage ledger as data. `None` omits the field entirely — a fully-covered
/// scan's report stays byte-identical to a pre-⟨0.15⟩ one (the wire-compatibility contract).
pub fn to_packaged_report_json_with_coverage(
    candor: &ReportMeta,
    package: &str,
    functions: &[ReportEntry],
    coverage: Option<&Coverage>,
) -> serde_json::Result<String> {
    to_packaged_report_json_full(candor, package, functions, coverage, &[], None)
}

/// ⟨proposed — Gap 2⟩ Like [`to_packaged_report_json_with_coverage`], additionally carrying the
/// `unanalyzed` list (the target source the scan couldn't see). An empty slice omits the field, so a
/// complete scan's report is byte-identical to a pre-rung one (the wire-compatibility contract).
pub fn to_packaged_report_json_full(
    candor: &ReportMeta,
    package: &str,
    functions: &[ReportEntry],
    coverage: Option<&Coverage>,
    unanalyzed: &[UnanalyzedUnit],
    analyzed: Option<&Analyzed>,
) -> serde_json::Result<String> {
    #[derive(Serialize)]
    struct Out<'a> {
        candor: &'a ReportMeta,
        #[serde(skip_serializing_if = "str::is_empty")]
        package: &'a str,
        #[serde(skip_serializing_if = "Option::is_none")]
        coverage: Option<&'a Coverage>,
        // ⟨0.22⟩ the completeness-manifest summary (Gap 1) — always present when the engine can enumerate its
        // analyzed set. Field order: coverage, analyzed, unanalyzed, functions (matches the java reference).
        #[serde(skip_serializing_if = "Option::is_none")]
        analyzed: Option<&'a Analyzed>,
        // Keys sort by serde field order in the struct; `unanalyzed` after `analyzed`, before `functions`.
        #[serde(skip_serializing_if = "<[_]>::is_empty")]
        unanalyzed: &'a [UnanalyzedUnit],
        functions: &'a [ReportEntry],
    }
    serde_json::to_string_pretty(&Out { candor, package, coverage, analyzed, unanalyzed, functions })
}

/// ⟨proposed: typeSurface⟩ As [`to_packaged_report_json_full`], additionally carrying the type surface.
/// An empty/`None` surface omits the field entirely, so a report from a crate with nothing to publish is
/// byte-identical to one produced before the rung existed.
#[allow(clippy::too_many_arguments)]
pub fn to_packaged_report_json_typed(
    candor: &ReportMeta,
    package: &str,
    functions: &[ReportEntry],
    coverage: Option<&Coverage>,
    unanalyzed: &[UnanalyzedUnit],
    analyzed: Option<&Analyzed>,
    type_surface: Option<&TypeSurface>,
) -> serde_json::Result<String> {
    #[derive(Serialize)]
    struct Out<'a> {
        candor: &'a ReportMeta,
        #[serde(skip_serializing_if = "str::is_empty")]
        package: &'a str,
        #[serde(skip_serializing_if = "Option::is_none")]
        coverage: Option<&'a Coverage>,
        #[serde(skip_serializing_if = "Option::is_none")]
        analyzed: Option<&'a Analyzed>,
        #[serde(skip_serializing_if = "<[_]>::is_empty")]
        unanalyzed: &'a [UnanalyzedUnit],
        #[serde(rename = "typeSurface", skip_serializing_if = "Option::is_none")]
        type_surface: Option<&'a TypeSurface>,
        functions: &'a [ReportEntry],
    }
    let ts = type_surface.filter(|t| !t.returns.is_empty());
    serde_json::to_string_pretty(&Out {
        candor, package, coverage, analyzed, unanalyzed, type_surface: ts, functions,
    })
}

/// ⟨proposed: typeSurface⟩ Parse a report's `typeSurface`. Absent = nothing travelled, never an error.
pub fn report_type_surface(text: &str) -> Option<TypeSurface> {
    let val: serde_json::Value = serde_json::from_str(text).ok()?;
    serde_json::from_value(val.get("typeSurface")?.clone()).ok()
}

/// ⟨proposed — Gap 2⟩ Parse a report's `unanalyzed` field. Empty when absent (a complete scan or any
/// pre-rung report) — absence is never an error, just "nothing unanalyzed travelled."
pub fn report_unanalyzed(text: &str) -> Vec<UnanalyzedUnit> {
    serde_json::from_str::<serde_json::Value>(text)
        .ok()
        .and_then(|v| v.get("unanalyzed").cloned())
        .and_then(|u| serde_json::from_value(u).ok())
        .unwrap_or_default()
}

/// ⟨0.15 staged⟩ Parse a report's `coverage` envelope field (spec §2). `None` when the field is
/// absent (a fully-covered scan, or any pre-⟨0.15⟩ report), the text isn't a JSON object, or the
/// field doesn't deserialize — absence of the ledger is never an error, just "no disclosure
/// travelled" (the pre-⟨0.15⟩ posture).
pub fn report_coverage(text: &str) -> Option<Coverage> {
    let val: serde_json::Value = serde_json::from_str(text).ok()?;
    serde_json::from_value(val.get("coverage")?.clone()).ok()
}

/// One structured gate violation (candor-spec §3.3 ⟨0.8⟩), shared by every backend so the verdict
/// shape is defined ONCE: `effects` is the specific effect set the violation concerns — the denied
/// set (006), the allow rule's effect (008), the gained set (005), or `[]` (009 layer-flow, no single
/// effect); `detail` is the message BODY (no `[AS-EFF-00x]` prefix — the rule carries the code). The
/// console gates print `[{rule}] {detail}`; `--gate-json` serializes these records verbatim.
#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct GateViolation {
    pub rule: String,
    #[serde(rename = "fn")]
    pub func: String,
    #[serde(default)]
    pub effects: Vec<String>,
    #[serde(default)]
    pub detail: String,
    /// Reason-scoped Unknown ⟨0.19⟩: on an AS-EFF-006 violation whose `effects` include `Unknown`, ALL the
    /// reason classes present (transitively) on the function — so a consumer sees every reason the strict
    /// gate bit, not just the matched one. Empty/omitted otherwise (SPEC §6.2).
    #[serde(rename = "reasonClass", default, skip_serializing_if = "Vec::is_empty")]
    pub reason_class: Vec<String>,
    /// Net destination-class ⟨0.20⟩: on an AS-EFF-006 violation whose `effects` include `Net`, ALL the
    /// destination classes present (transitively) on the function — so a consumer sees which class the
    /// security gate bit. Empty/omitted otherwise (SPEC §6.2, NET-DESTINATION-CLASS-DESIGN.md).
    #[serde(rename = "netClass", default, skip_serializing_if = "Vec::is_empty")]
    pub net_class: Vec<String>,
}

/// Serialize the §3.3 gate verdict `{ spec, ok, violations }` — the machine analog of the `AS-EFF`
/// console lines. Defined here (with [`GateViolation`]) so the stable scanner, the deep engine, and
/// `candor-query gate-verdict` can never drift on field names or shape. Violations are sorted by
/// `(rule, detail)` — the same order the console gates print — so the verdict is deterministic
/// regardless of which crate/member recorded first. Pretty-printed; callers append the trailing
/// newline when writing to a file.
pub fn gate_verdict_json(violations: &mut [GateViolation]) -> serde_json::Result<String> {
    gate_verdict_json_with_coverage(violations, None)
}

/// ⟨0.15 staged⟩ The gate verdict's ADVISORY coverage note (spec §3.3): how many uncovered
/// packages the scanned/queried report's κ ledger names, and which. Disclosure only — a gate does
/// NOT fail on uncovered deps (nearly every real scan has some); the policy author sees the note
/// and decides. `deny Unknown` remains the opt-in strict posture.
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct GateCoverage {
    pub uncovered: usize,
    pub packages: Vec<String>,
}

/// ⟨0.15 staged⟩ Like [`gate_verdict_json`], with the optional advisory `coverage` note appended
/// after the pinned `{ spec, ok, violations }` fields. VERDICT-PRESERVING by construction:
/// `ok`/`violations` are computed exactly as before (the ⟨0.9⟩ provable-purity auto-disclosure
/// precedent), and `None` omits the field — the verdict is then byte-identical to the pre-⟨0.15⟩
/// one, so a fully-covered gate is unchanged on the wire.
pub fn gate_verdict_json_with_coverage(
    violations: &mut [GateViolation],
    coverage: Option<&GateCoverage>,
) -> serde_json::Result<String> {
    violations.sort_by(|a, b| (a.rule.as_str(), a.detail.as_str()).cmp(&(b.rule.as_str(), b.detail.as_str())));
    #[derive(Serialize)]
    struct Verdict<'a> {
        spec: &'static str,
        ok: bool,
        violations: &'a [GateViolation],
        #[serde(skip_serializing_if = "Option::is_none")]
        coverage: Option<&'a GateCoverage>,
    }
    serde_json::to_string_pretty(&Verdict {
        spec: SPEC_VERSION,
        ok: violations.is_empty(),
        violations,
        coverage,
    })
}

/// ⟨0.22⟩ COMPLETENESS MANIFEST verdict: like [`gate_verdict_json_with_coverage`], plus the `analyzed`
/// count (Gap 1, always present) and — when the scan was INCOMPLETE (`unanalyzed` non-empty) — `incomplete:
/// true` + the `unanalyzed` list (Gap 2). `ok` requires BOTH no violation AND a complete analysis, so a
/// machine/agent reading the verdict can't see `ok:true` over code candor never analyzed. Field order
/// matches the java reference: spec, ok, analyzed, violations, coverage?, incomplete?, unanalyzed?.
pub fn gate_verdict_json_full(
    violations: &mut [GateViolation],
    coverage: Option<&GateCoverage>,
    analyzed_count: usize,
    unanalyzed: &[UnanalyzedUnit],
) -> serde_json::Result<String> {
    violations.sort_by(|a, b| (a.rule.as_str(), a.detail.as_str()).cmp(&(b.rule.as_str(), b.detail.as_str())));
    #[derive(Serialize)]
    struct Count {
        count: usize,
    }
    #[derive(Serialize)]
    struct Verdict<'a> {
        spec: &'static str,
        ok: bool,
        analyzed: Count,
        violations: &'a [GateViolation],
        #[serde(skip_serializing_if = "Option::is_none")]
        coverage: Option<&'a GateCoverage>,
        #[serde(skip_serializing_if = "std::ops::Not::not")]
        incomplete: bool,
        #[serde(skip_serializing_if = "<[_]>::is_empty")]
        unanalyzed: &'a [UnanalyzedUnit],
    }
    let incomplete = !unanalyzed.is_empty();
    serde_json::to_string_pretty(&Verdict {
        spec: SPEC_VERSION,
        ok: violations.is_empty() && !incomplete,
        analyzed: Count { count: analyzed_count },
        violations,
        coverage,
        incomplete,
        unanalyzed,
    })
}

/// The engine version that produced a v0.2 report (its envelope `candor.version`). None for a legacy
/// v0.1 bare array (no header).
/// Does the report carry a v0.2 `{ candor: {...}, functions }` ENVELOPE (vs a legacy v0.1 bare array)?
/// Used to tell "no version because it's pre-v0.2 (trust as documented)" from "has an envelope but the
/// version is missing/garbage (a partial write — do NOT trust)".
pub fn report_has_envelope(text: &str) -> bool {
    serde_json::from_str::<serde_json::Value>(text)
        .map(|v| v.get("candor").is_some())
        .unwrap_or(false)
}

pub fn report_version(text: &str) -> Option<String> {
    #[derive(Deserialize)]
    struct Meta {
        version: Option<String>,
    }
    #[derive(Deserialize)]
    struct Env {
        candor: Option<Meta>,
    }
    serde_json::from_str::<Env>(text).ok().and_then(|e| e.candor).and_then(|m| m.version)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unanalyzed_round_trips_and_omits_when_empty() {
        // Gap 2: the `unanalyzed` field carries the target source the scan couldn't see into the wire,
        // and is OMITTED when empty (byte-compat with a pre-rung report).
        let meta = ReportMeta { version: "v".into(), toolchain: "t".into(), spec: SPEC_VERSION.into() };
        // present: a parse failure travels
        let units = vec![UnanalyzedUnit { path: "src/broken.rs".into(), reason: "source failed to read/parse".into() }];
        let json = to_packaged_report_json_full(&meta, "p", &[], None, &units, None).unwrap();
        assert!(json.contains("\"unanalyzed\""), "unanalyzed must serialize when non-empty");
        let parsed = report_unanalyzed(&json);
        assert_eq!(parsed.len(), 1);
        assert_eq!(parsed[0].path, "src/broken.rs");
        // empty: omitted entirely (a complete scan is byte-identical to a pre-rung report)
        let clean = to_packaged_report_json_full(&meta, "p", &[], None, &[], None).unwrap();
        assert!(!clean.contains("unanalyzed"), "an empty unanalyzed must be omitted");
        assert!(report_unanalyzed(&clean).is_empty());
    }

    #[test]
    fn analyzed_digest_is_deterministic_and_stable() {
        // Gap 1: the FNV-1a digest is 16 lowercase hex, changes iff the sorted qual set changes, and is
        // stable across a re-hash of the same input (a same-engine re-scan agrees).
        let a = fnv1a_hex(&["a::f".to_string(), "a::g".to_string()]);
        let b = fnv1a_hex(&["a::f".to_string(), "a::g".to_string()]);
        assert_eq!(a, b, "same set ⇒ same digest");
        assert_eq!(a.len(), 16);
        assert!(a.chars().all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase()));
        assert_ne!(a, fnv1a_hex(&["a::f".to_string()]), "a different set ⇒ a different digest");
    }

    #[test]
    fn incomplete_verdict_is_fail_closed_and_machine_legible() {
        // Gap 2: an incomplete gate verdict carries ok:false + incomplete:true + the unanalyzed list + the
        // analyzed count, so a machine learns WHY the gate can't certify (never a fabricated ok:true).
        let units = vec![UnanalyzedUnit { path: "src/bad.rs".into(), reason: "source failed to read/parse".into() }];
        let json = gate_verdict_json_full(&mut [], None, 7, &units).unwrap();
        let v: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(v["ok"], false, "an incomplete gate is never ok");
        assert_eq!(v["incomplete"], true);
        assert_eq!(v["analyzed"]["count"], 7);
        assert_eq!(v["unanalyzed"][0]["path"], "src/bad.rs");
        // a COMPLETE verdict carries analyzed:{count} but omits incomplete/unanalyzed (byte-compat).
        let clean = gate_verdict_json_full(&mut [], None, 7, &[]).unwrap();
        let cv: serde_json::Value = serde_json::from_str(&clean).unwrap();
        assert_eq!(cv["ok"], true);
        assert!(cv.get("incomplete").is_none() && cv.get("unanalyzed").is_none());
        assert_eq!(cv["analyzed"]["count"], 7);
    }

    #[test]
    fn parses_envelope_and_bare_array() {
        let env = r#"{"candor":{"version":"v9","toolchain":"t"},"functions":[{"fn":"a","inferred":["Net"]}]}"#;
        let e = report_entries(env).unwrap();
        assert_eq!(e.len(), 1);
        assert_eq!(e[0].func, "a");
        assert_eq!(e[0].inferred, ["Net"]);
        assert_eq!(report_version(env).as_deref(), Some("v9"));

        let bare = r#"[{"fn":"a","inferred":["Fs"],"hash":""}]"#;
        assert_eq!(report_entries(bare).unwrap().len(), 1);
        assert!(report_version(bare).is_none());

        assert!(report_entries("not json").is_none());
        assert!(report_entries(r#"{"candor":{}}"#).is_none()); // object without `functions`
    }

    #[test]
    fn counted_surfaces_dropped_entries_for_disclosure() {
        // A mid-write / hand-edited entry whose `inferred` is a string not an array fails to deserialize.
        // The good entries survive (per-entry independence), but the drop must be COUNTED so the caller
        // can disclose it — a silently-vanished function reads as pure, the under-report the gate forbids.
        let mixed = r#"{"candor":{"version":"v","toolchain":"t"},"functions":[
            {"fn":"good","inferred":["Net"]},
            {"fn":"corrupt","inferred":"Fs"},
            {"fn":"alsogood","inferred":["Db"]}
        ]}"#;
        let (entries, dropped) = report_entries_counted(mixed).unwrap();
        assert_eq!(dropped, 1, "the string-`inferred` entry must be counted as dropped");
        assert_eq!(entries.len(), 2);
        assert_eq!(report_entries(mixed).unwrap().len(), 2); // back-compat: same surviving entries
        // a clean report drops nothing
        let clean = r#"[{"fn":"a","inferred":["Fs"],"hash":""}]"#;
        assert_eq!(report_entries_counted(clean).unwrap().1, 0);
    }

    #[test]
    fn round_trips() {
        let r = Report {
            candor: ReportMeta { version: "v".into(), toolchain: "t".into(), spec: SPEC_VERSION.into() },
            coverage: None,
            unanalyzed: vec![],
            type_surface: None,
            functions: vec![ReportEntry {
                func: "f".into(),
                inferred: vec!["Db".into(), "Unknown".into()],
                unknown_why: vec!["dispatch:foo::Bar".into()],
                entry_point: true,
                ..Default::default()
            }],
        };
        let s = serde_json::to_string(&r).unwrap();
        let back = report_entries(&s).unwrap();
        assert_eq!(back[0].func, "f");
        // empty `calls` is omitted on write.
        assert!(!s.contains("\"calls\""));
        // `unknownWhy` round-trips under its JSON name and survives deserialization.
        assert!(s.contains("\"unknownWhy\":[\"dispatch:foo::Bar\"]"), "unknownWhy must serialize: {s}");
        assert_eq!(back[0].unknown_why, vec!["dispatch:foo::Bar".to_string()]);
        // `entryPoint` round-trips under its JSON name and is omitted when false.
        assert!(s.contains("\"entryPoint\":true"), "entryPoint must serialize when set: {s}");
        assert!(back[0].entry_point);
        // …and is omitted entirely when empty/false (the common case).
        let empty = serde_json::to_string(&ReportEntry { func: "g".into(), ..Default::default() }).unwrap();
        assert!(!empty.contains("unknownWhy"), "empty unknownWhy must be omitted: {empty}");
        assert!(!empty.contains("entryPoint"), "false entryPoint must be omitted: {empty}");
        // the spec contract version (§2.1) is emitted in the envelope header.
        assert!(s.contains("\"spec\":\"0.23\""), "envelope must carry the spec version: {s}");
        assert_eq!(SPEC_VERSION, "0.23");
    }

    #[test]
    fn report_entries_skips_a_bad_entry_not_the_whole_file() {
        // One entry missing the required `fn` must lose ONLY itself — not drop every function in the
        // file (which was a silent whole-crate under-report under the old all-or-nothing deser).
        let bare = r#"[{"fn":"good","inferred":["Net"]},{"inferred":["Db"]},{"fn":"good2"}]"#;
        let got = report_entries(bare).unwrap();
        assert_eq!(got.iter().map(|e| e.func.as_str()).collect::<Vec<_>>(), ["good", "good2"]);
        // same inside a v0.2 envelope.
        let env = r#"{"candor":{"version":"v","toolchain":"t"},"functions":[{"inferred":["Db"]},{"fn":"ok"}]}"#;
        assert_eq!(report_entries(env).unwrap().len(), 1);
        // genuinely-broken JSON still yields None (not a panic).
        assert!(report_entries("{not json").is_none());
    }

    #[test]
    fn type_surface_is_omitted_when_empty_and_round_trips_when_not() {
        // ⟨typeSurface⟩ WIRE COMPATIBILITY is the whole reason this is a separate envelope block: a
        // crate with nothing to publish must produce the SAME BYTES as the pre-rung writer, so a
        // consumer that ignores the field is unaffected (tier-1 additive).
        let meta = ReportMeta { version: "v".into(), toolchain: "t".into(), spec: SPEC_VERSION.into() };
        let e = [ReportEntry { func: "f".into(), inferred: vec!["Net".into()], ..Default::default() }];
        let old = to_packaged_report_json_full(&meta, "p", &e, None, &[], None).unwrap();
        let empty = to_packaged_report_json_typed(&meta, "p", &e, None, &[], None,
                                                  Some(&TypeSurface::default())).unwrap();
        assert_eq!(old, empty, "an empty type surface must not change one byte of the report");
        assert!(report_type_surface(&old).is_none(), "absence must parse as nothing, never an error");
        let mut returns = std::collections::BTreeMap::new();
        returns.insert("dep#sync::build".to_string(), "dep#sync::Client".to_string());
        let full = to_packaged_report_json_typed(&meta, "p", &e, None, &[], None,
                                                 Some(&TypeSurface { returns })).unwrap();
        let back = report_type_surface(&full).expect("typeSurface must round-trip");
        assert_eq!(back.returns.get("dep#sync::build").map(String::as_str), Some("dep#sync::Client"),
                   "both ids stay FULLY QUALIFIED on the wire — the leaf form is the reverted defect");
    }

    #[test]
    fn gate_verdict_shape_ok_flag_and_sort() {
        // Empty → the clean verdict (spec §3.3: "with no gate configured it writes { ok: true, [] }").
        let clean = gate_verdict_json(&mut []).unwrap();
        let v: serde_json::Value = serde_json::from_str(&clean).unwrap();
        assert_eq!(v["spec"], SPEC_VERSION);
        assert_eq!(v["ok"], true);
        assert_eq!(v["violations"], serde_json::json!([]));
        // Violations sort by (rule, detail) and serialize under the pinned field names (`fn`, not func).
        let mut vs = vec![
            GateViolation { rule: "AS-EFF-009".into(), func: "b".into(), effects: vec![], detail: "z".into(), ..Default::default() },
            GateViolation { rule: "AS-EFF-006".into(), func: "a".into(), effects: vec!["Net".into()], detail: "y".into(), ..Default::default() },
            GateViolation { rule: "AS-EFF-006".into(), func: "c".into(), effects: vec!["Db".into()], detail: "x".into(), ..Default::default() },
        ];
        let s = gate_verdict_json(&mut vs).unwrap();
        let v: serde_json::Value = serde_json::from_str(&s).unwrap();
        assert_eq!(v["ok"], false);
        let arr = v["violations"].as_array().unwrap();
        assert_eq!(arr.len(), 3);
        assert_eq!(arr[0]["fn"], "c", "sorted by (rule, detail): x before y");
        assert_eq!(arr[0]["effects"], serde_json::json!(["Db"]));
        assert_eq!(arr[2]["rule"], "AS-EFF-009");
        // round-trips (candor-query gate-verdict re-parses NDJSON records of this shape).
        let back: GateViolation = serde_json::from_str(&serde_json::to_string(&arr[0]).unwrap()).unwrap();
        assert_eq!(back.func, "c");
    }

    /// ⟨0.15 staged⟩ The `coverage` envelope field: emitted when the ledger is non-empty, OMITTED
    /// entirely when `None` — a fully-covered scan's report must stay BYTE-IDENTICAL to the
    /// pre-⟨0.15⟩ serializer's output (the wire-compatibility contract), and a consumer must
    /// tolerate the field (§2 forward compatibility).
    #[test]
    fn coverage_envelope_emitted_omitted_and_tolerated() {
        let meta = ReportMeta { version: "v".into(), toolchain: "t".into(), spec: SPEC_VERSION.into() };
        let fns = vec![ReportEntry { func: "f".into(), inferred: vec!["Net".into()], ..Default::default() }];
        // None → byte-identical to the pre-⟨0.15⟩ shape (the delegating old entry point).
        let plain = to_packaged_report_json(&meta, "pkg", &fns).unwrap();
        let with_none = to_packaged_report_json_with_coverage(&meta, "pkg", &fns, None).unwrap();
        assert_eq!(plain, with_none, "None coverage must not change a byte");
        assert!(!plain.contains("coverage"), "omitted when nothing is uncovered: {plain}");
        // Some → the §2 shape, between `package` and `functions`.
        let cov = Coverage {
            uncovered: vec![
                CoverageEntry { name: "serde_json".into(), calls: 165 },
                CoverageEntry { name: "somedep".into(), calls: 1 },
            ],
        };
        let with = to_packaged_report_json_with_coverage(&meta, "pkg", &fns, Some(&cov)).unwrap();
        assert!(with.contains("\"coverage\""), "{with}");
        assert!(with.contains("\"name\": \"serde_json\""), "{with}");
        assert!(with.contains("\"calls\": 165"), "{with}");
        // The reader round-trips it; absence reads None (a pre-⟨0.15⟩ report).
        assert_eq!(report_coverage(&with), Some(cov));
        assert_eq!(report_coverage(&plain), None);
        assert_eq!(report_coverage("not json"), None);
        // §2 forward compatibility: the entries loader TOLERATES the new envelope field — every
        // report-consuming verb loads through this, so a coverage-carrying report queries fine.
        let entries = report_entries(&with).expect("a coverage-carrying report must still parse");
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].func, "f");
        // And the typed Report round-trips the optional field.
        let r: Report = serde_json::from_str(&with).unwrap();
        assert_eq!(r.coverage.as_ref().map(|c| c.uncovered.len()), Some(2));
        let r: Report = serde_json::from_str(&plain).unwrap();
        assert!(r.coverage.is_none());
    }

    /// ⟨0.15 staged⟩ The gate verdict's advisory coverage note is VERDICT-PRESERVING: `spec`, `ok`
    /// and `violations` are byte-for-byte what the pre-⟨0.15⟩ serializer emits (the pinned §3.3
    /// fields conformance PARTs 12/12b/12d compare); the note only APPENDS, and `None` omits it.
    #[test]
    fn gate_verdict_coverage_note_is_advisory_and_verdict_preserving() {
        let mk = || {
            vec![GateViolation {
                rule: "AS-EFF-006".into(),
                func: "f".into(),
                effects: vec!["Net".into()],
                detail: "d".into(),
                ..Default::default()
            }]
        };
        let cov = GateCoverage { uncovered: 2, packages: vec!["anyhow".into(), "bstr".into()] };
        // None → byte-identical to the pre-⟨0.15⟩ verdict.
        assert_eq!(
            gate_verdict_json_with_coverage(&mut mk(), None).unwrap(),
            gate_verdict_json(&mut mk()).unwrap(),
        );
        // Some → the pinned fields are UNCHANGED; `coverage` is appended.
        let with = gate_verdict_json_with_coverage(&mut mk(), Some(&cov)).unwrap();
        let without = gate_verdict_json(&mut mk()).unwrap();
        let vw: serde_json::Value = serde_json::from_str(&with).unwrap();
        let vo: serde_json::Value = serde_json::from_str(&without).unwrap();
        for k in ["spec", "ok", "violations"] {
            assert_eq!(vw[k], vo[k], "pinned verdict field `{k}` must be unchanged by the note");
        }
        assert_eq!(vw["ok"], false, "a violating gate stays failing with the note");
        assert_eq!(vw["coverage"]["uncovered"], 2);
        assert_eq!(vw["coverage"]["packages"], serde_json::json!(["anyhow", "bstr"]));
        // A clean gate with a non-empty ledger stays ok:true — disclosure, never a gate-failure.
        let clean = gate_verdict_json_with_coverage(&mut [], Some(&cov)).unwrap();
        let vc: serde_json::Value = serde_json::from_str(&clean).unwrap();
        assert_eq!(vc["ok"], true);
        assert_eq!(vc["coverage"]["uncovered"], 2);
    }

    /// The ONE discrimination rule shared by the lint's cross-crate loader and the CLI: a report is
    /// `<base>.<crate>.<type>.json` (exactly two segments after the base); sidecars (one segment) and
    /// any 3+-segment name are NOT reports. Real reports always have a dot-free crate name and type,
    /// so two segments is exact. Sorted by path, with krate/kind parsed from the filename.
    #[test]
    fn report_files_discriminates_and_parses() {
        let dir = std::env::temp_dir().join("candor-report-files-test");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        for f in [
            "r.mycrate.lib.json",          // report ✓ → (mycrate, lib)
            "r.mycrate.Executable.json",   // report ✓ → (mycrate, Executable)
            "r.calibrated.json",           // sidecar ✗ (one segment)
            "r.encountered-mycrate.json",  // sidecar ✗ (one segment)
            "r.a.b.c.json",                // ✗ 3 segments (kind "b.c" has a dot)
            "other.x.y.json",              // ✗ different base
        ] {
            std::fs::write(dir.join(f), "[]").unwrap();
        }
        let prefix = dir.join("r");
        let got: Vec<(String, String, String)> = report_files(prefix.to_str().unwrap())
            .into_iter()
            .map(|r| {
                (r.path.file_name().unwrap().to_str().unwrap().to_string(), r.krate, r.kind)
            })
            .collect();
        assert_eq!(
            got,
            vec![
                ("r.mycrate.Executable.json".into(), "mycrate".into(), "Executable".into()),
                ("r.mycrate.lib.json".into(), "mycrate".into(), "lib".into()),
            ]
        );
        let _ = std::fs::remove_dir_all(&dir);
    }
}
