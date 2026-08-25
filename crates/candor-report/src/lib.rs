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
/// `<prefix>.<crate>.<type>.json`; the sidecars are NOT reports — either because they carry only ONE
/// segment after the prefix (`<prefix>.calibrated.json`, `<prefix>.encountered-*.json`) or because
/// their trailing segment is a reserved [`SIDECAR_KINDS`] name (`<prefix>.<pkg>.hierarchy.json`).
pub struct ReportFile {
    pub path: PathBuf,
    /// The `<crate>` segment of the filename.
    pub krate: String,
    /// The `<type>` segment (e.g. `lib`, `Executable`).
    pub kind: String,
}

/// The reserved trailing name-segments that mark a SIDECAR, never a crate `<type>` — so a sidecar
/// named `<base>.<pkg>.<kind>.json` can never be mistaken for the report `<base>.<crate>.<type>.json`.
///
/// **This is a DENYLIST, deliberately, and the direction matters.** `report_files` still ACCEPTS any
/// `<base>.<a>.<b>.json`; this list only carves out the names that are provably not crate types. The
/// allowlist inversion — accepting only a known set of `<type>`s (`lib`/`Rlib`/`Executable`/`scan`/…) —
/// would make any report whose type segment we failed to anticipate (a new rustc crate type, another
/// engine's `Swift`/`jar`/`esm` kind) SILENTLY INVISIBLE to every query: a false all-clear, the §4
/// cardinal sin. A denylist can only ever be *incomplete*, and incompleteness here is LOUD, not silent
/// — a sidecar suffix missing from this list falls back into the candidate set, fails to parse as a
/// report, and prints the "failed to parse — its functions are OMITTED" disclosure on stderr for every
/// query. Noise, not a swallowed report. Add the suffix here when that happens.
///
/// Safety of each entry: these are `<type>` positions. `<type>` is a crate/compilation kind (`lib`,
/// `Rlib`, `Executable`, `Cdylib`, `scan`, and other engines' `Swift`/`jar`), and no engine in the
/// family names a kind after one of these artifacts. A CRATE legitimately named `hierarchy` is
/// untouched — it lands in the `<crate>` position (`<base>.hierarchy.lib.json`), which this does not
/// look at (pinned by `report_files_discriminates_and_parses`).
///
/// Sourced from what the family actually writes: `callgraph`/`hierarchy` (SPEC §2.2 — each engine
/// pairs them to its own report stem, so their segment count is NOT fixed), `calibrated`/`layerreach`/
/// `encountered-*` (this engine), `locs`/`gate` (candor-ts). candor-ts (`query-core.mjs` `isReport`),
/// candor-java (`Query.java`) and candor-swift (`FixCLI.swift`) all exclude the same suffixes by name;
/// this engine discriminated by segment count alone, which covered its OWN 3-segment sidecars but not
/// a 2-segment one from another producer.
pub const SIDECAR_KINDS: [&str; 7] =
    // ⟨0.32⟩ `refused` — the refusal marker (SPEC §2.2's family-wide reserved set). This engine already
    // excluded it INCIDENTALLY, because the discrimination rule wants `<crate>.<type>` (two segments)
    // and `<prefix>.refused.json` has one. Listed explicitly anyway: being excluded by SHAPE rather than
    // by NAME is precisely the drift §2.2 records — "three of the four excluded these by name and one
    // discriminated by segment count, but the by-name lists disagreed" — and candor-ts, whose predicate
    // IS by name, counted the marker as a report the moment ⟨0.32⟩ landed there. PART 56 caught it.
    ["callgraph", "hierarchy", "calibrated", "layerreach", "locs", "gate", "refused"];

/// Discover the per-crate report files for a prefix (`.candor/report` →
/// `.candor/report.<crate>.<type>.json`), sorted by path for deterministic output. A directoryless
/// prefix reads the current directory. ONE discrimination rule — `<crate>.<type>`, exactly two
/// segments, and a `<type>` that is not a reserved [`SIDECAR_KINDS`] name — shared by the lint's
/// cross-crate loader and the CLI's queries, so the two can never disagree about which files are
/// reports.
/// ⟨0.32⟩ A refusal marker left beside a report set — SPEC §3.3.1 ⟨0.32⟩.
#[derive(Debug, Clone)]
pub struct RefusalMarker {
    pub prefix: String,
    pub target: String,
    pub reason: String,
}

/// ⟨0.32⟩ IS THE MOST RECENT ATTEMPT OVER THESE REPORTS A REFUSAL?
///
/// A consumer cannot work this out for itself. The hazard is an EVENT — a refusal that happened AFTER
/// these bytes were written — witnessed only by the run that refused. No function of the report and the
/// tree recovers it: `analyzed.digest` is over the sorted analyzed-qual set, so a changed body under an
/// unchanged name is byte-identical. So the refusing run writes it down, and this reads it.
///
/// Resolved for all three §3.3.1 locator forms. The DIRECT-FILE case is why the marker carries its own
/// `prefix`: that locator accepts any `.json` name whatever its dot-segments, so the prefix cannot be
/// recovered from the filename — the marker is found by scanning the file's directory and asking which
/// recorded prefix covers it. Without that, two prefixes sharing a directory would make one refusal
/// refuse the other's reports, which is a false red rather than a missed one but still wrong.
pub fn refusal_marker_for(locator: &str) -> Option<RefusalMarker> {
    fn parse(path: &Path) -> Option<RefusalMarker> {
        let v: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(path).ok()?).ok()?;
        if v.get("refused")?.as_bool() != Some(true) {
            return None;
        }
        Some(RefusalMarker {
            prefix: v.get("prefix")?.as_str()?.to_string(),
            target: v.get("target").and_then(|t| t.as_str()).unwrap_or("").to_string(),
            reason: v.get("reason").and_then(|r| r.as_str()).unwrap_or("").to_string(),
        })
    }
    let p = Path::new(locator);
    if locator.ends_with(".json") && p.is_file() {
        let dir = p.parent()?;
        let me = std::fs::canonicalize(p).unwrap_or_else(|_| p.to_path_buf());
        for entry in std::fs::read_dir(dir).ok()?.flatten() {
            let f = entry.path();
            if !f.to_string_lossy().ends_with(".refused.json") {
                continue;
            }
            if let Some(m) = parse(&f) {
                let covered = std::fs::canonicalize(Path::new(&m.prefix))
                    .map(|c| me.starts_with(&c))
                    .unwrap_or(false)
                    || me.to_string_lossy().starts_with(m.prefix.trim_start_matches("./"));
                if covered {
                    return Some(m);
                }
            }
        }
        return None;
    }
    parse(Path::new(&format!("{locator}.refused.json")))
}

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
        // …and `kind` must not be a reserved SIDECAR name. The segment-count rule alone excludes THIS
        // engine's sidecars (all 3-segment: `<base>.<crate>.<type>.callgraph.json`) but not a 2-segment
        // one from another producer — SPEC §2.2 lets each engine pair its sidecar to its own report
        // stem, so `<base>.<pkg>.hierarchy.json` is a legitimate name that lands exactly on the
        // `<crate>.<type>` shape. Excluded HERE, at the glob, rather than diagnosed after a failed
        // parse: a sidecar in the candidate set produced a FALSE disclosure ("failed to parse — its
        // functions are OMITTED … re-run the scan") on a scan that was fine, and worse, it fed the
        // `load_entries_loud` corruption guard — an effect-free crate (a well-formed `functions: []`
        // report) beside a sidecar was refused at exit 2. Suppressing the message instead would leave
        // both of those live and is one refactor from returning.
        if SIDECAR_KINDS.contains(&kind) {
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
    // ⟨0.26⟩ OPTION, NOT A DEFAULTED VEC — the §2 rule has a deserialization half and this is it. The trio
    // is the §5 capability-reconciliation output: PRESENT means that pass ran, ABSENT means it did not,
    // and `[]` from an engine that computed nothing is forbidden (it claims "no function performs an
    // undeclared effect"). `#[serde(default)]` over a `Vec` destroyed exactly that distinction on the way
    // in — an absent key deserialized to `vec![]`, indistinguishable from an explicit empty answer, so a
    // producer's careful omission became the same claim with extra steps. This engine runs no §5 pass, so
    // it always writes `None`; the Option matters when READING another engine's report.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub declared: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub undeclared: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub overdeclared: Option<Vec<String>>,
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
pub const SPEC_VERSION: &str = "0.32";

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
    /// `#[serde(default)]` on READ only, for the same reason as [`Analyzed::digest`]: `name` is the
    /// load-bearing datum — the verdict's κ note carries the NAMES, and nothing on the gate route reads
    /// `calls` at all — so refusing an entry that names an uncovered package for want of its call count
    /// would DROP a hedge in order to be strict about a decoration. Measured on a hand-built
    /// `{name, why}` ledger: with `calls` required the whole report was refused; with it defaulted the
    /// package name still reaches the verdict, which is the disclosure the field exists for. A ledger
    /// whose entries are not objects at all (`uncovered: [3]`, `coverage: "none"`) is still Corrupt.
    #[serde(default)]
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

/// ⟨0.29⟩ ONE CLASS OF FILE THE SCAN DELIBERATELY DID NOT OPEN — the report's missing DENOMINATOR.
///
/// `analyzed.count` is a numerator: how many functions were judged. The SCOPE that produced it — which
/// files the engine chose not to look at, and why — appeared nowhere, so a consumer could not tell
/// whether the answer was to the question they asked. Every exclusion here is DELIBERATE and was already
/// documented in a code comment; that is precisely why nobody measured what it costs. `deny Exec` over a
/// crate whose `build.rs` runs `curl | sh` was GREEN, on a file that runs on every `cargo build`.
///
/// A CLASS with a COUNT, never a file list: the excluded set includes `target/`, which is unbounded, and
/// a gate that routinely prints thousands of paths is one people learn to scroll past.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct ExcludedClass {
    /// A stable machine token — `build-script`, `non-library-target`, `test-module`, `build-output`.
    pub class: String,
    pub count: usize,
    /// ⟨0.29⟩ Does THE PEEK read this class? The load-bearing half of the pair with [`OutOfScopeFinding`]:
    /// an empty `outOfScope` says "I read the excluded files and none held an effect this policy denies",
    /// and it may make that claim only about the classes it actually read.
    ///
    /// TRUE FOR EVERY CLASS THIS ENGINE EXCLUDES, because the peek is one walk with the selection
    /// INVERTED — the two file sets are exact complements by construction. It is not a constant across
    /// the family, which is why it is a field rather than an assumption: candor-java cannot read a
    /// `.java` that was never compiled (it reads bytecode), and candor-swift does not read `.build/`.
    /// Without it their `[]` would certify files nobody opened — the ⟨0.26⟩ partial-manifest failure, a
    /// partial answer being worse than an absent one.
    #[serde(default)]
    pub peeked: bool,
    /// ⟨0.32⟩ TRUE when the files of this class are COPIES of code this same scan already judged — a
    /// jar or archive under a build tree is a derived copy of what was just analysed, so the class
    /// hides nothing and does not make the verdict INCOMPLETE.
    ///
    /// ONLY THE PRODUCER MAY SET IT. A consumer must not infer it from the class token: those tokens
    /// are engine-chosen (the clause above), and the same concept is spelled `build-output` here and
    /// `build-output-archive` by candor-java — so a consumer carrying its own list of "derived" names
    /// gates another engine's report differently from the engine that wrote it. The distinction is not
    /// cosmetic either: `build-script` is `build.rs`, code that RUNS at build time and can perform any
    /// effect, and it must fail closed; `build-output` must not.
    #[serde(rename = "judgedElsewhere", default, skip_serializing_if = "std::ops::Not::not")]
    pub judged_elsewhere: bool,
    /// WHY, in the engine's own words. A consumer reads this to decide whether the exclusion matches the
    /// question they are asking; conformance asserts on this VALUE, not on the key's presence.
    pub reason: String,
}

/// ⟨0.29⟩ AN EFFECT FOUND IN A FILE THE GATE DID NOT JUDGE.
///
/// Emitted only when a policy is configured, and only for effects that policy DENIES — so a project with
/// no policy sees nothing, and one with `deny Net` is not told about `Exec` in its test tree. That bound
/// is what keeps this from becoming the noise it would otherwise be.
///
/// NEVER A `violation`, and that distinction is why ⟨0.30⟩'s exit code is 2 rather than 1: the gate did
/// not JUDGE these units, so reporting them as violations would be false in the other direction.
///
/// ⟨0.30⟩ THEY ARE NO LONGER NON-BINDING. ⟨0.29⟩ shipped this as pure disclosure — "the exit code MUST be
/// what it would have been without it" — on the assumption that the peek surfaces UNCERTAINTY. Measured
/// on published 0.29.1 it resolves a CONCRETE denied effect and names the function (axios: 37 functions
/// `performs Net`, exit 0, `policy ✓`), so a non-empty block now makes the verdict `ok:false`,
/// `incomplete:true` at exit 2. Same classifier, different file set — that part is unchanged.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct OutOfScopeFinding {
    #[serde(rename = "fn")]
    pub func: String,
    pub path: String,
    pub effects: Vec<String>,
    pub class: String,
    pub reason: String,
}

/// ⟨0.31⟩ The ambient `net-partner` declaration that MOVED a `netClass` — the config file that declared
/// it, and the declared hosts that actually PARTICIPATED in this scan.
///
/// `hosts` is what participated, not what was declared: a config listing twenty partners of which one
/// matched discloses the one, because a list of everything written down buries the line that moved the
/// verdict. Recorded by the PRODUCER because `gate --report` cannot compute it — `net-partner` anchors at
/// the TARGET and that route has no target, so re-classifying through the consumer's own config would make
/// the verdict depend on the reader's working directory (the re-derivation ⟨0.24⟩ forbids). Both routes
/// copy this one record, which is what makes §3.1's byte-equality hold instead of break.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct NetPartners {
    pub config: String,
    pub hosts: Vec<String>,
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
    /// `#[serde(default)]` on READ only — this engine always writes it. `count` is the load-bearing
    /// datum (it is what the verdict carries and what §2's judged-nothing rule is keyed on); `digest` is
    /// an opaque fingerprint nothing gates on. Refusing `{"count": 5}` for a missing digest would mint a
    /// refusal SPEC §2 does not ask for, over a manifest whose claim is perfectly readable — and the
    /// ⟨0.24⟩ present-but-unparseable rule is about claims that CANNOT be read, not about tidiness.
    /// A non-integer `count` (`true`, `"5"`, absent) still makes the whole key `KeyRead::Corrupt`.
    #[serde(default)]
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

/// ⟨0.27⟩ SPEC §2.1 `resolves` — the OPTIONAL refinement surfaces THIS ENGINE computes.
///
/// A producer MUST NOT list a surface it does not compute: that turns "unimplemented" into a false
/// "undetermined", which is the exact inversion the field exists to prevent. So this constant is the one
/// place to change when an optional surface is implemented, and adding a name here without the
/// implementation is a defect, not an aspiration.
/// ⟨0.29⟩ `incomplete` joins the list. It is an optional per-function refinement surface whose absence is
/// overloaded exactly the way `fs`'s was — "this producer does not compute undetermined locators" vs
/// "computed them and found none" — which is the ambiguity this field exists to remove. It earns the
/// declaration by the same rule that governs the list: this engine computes it, so it says so.
pub const RESOLVES: &[&str] = &["fs", "incomplete"];

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
    /// ⟨0.27⟩ SPEC §2.1 `resolves` — the OPTIONAL per-function refinement surfaces this producer actually
    /// computes. Without it the absence of such a field is overloaded between "does not compute this" and
    /// "computed and could not determine it", and a consumer cannot read the omission at all.
    ///
    /// MUST NOT list a surface the engine does not compute: that converts "unimplemented" into a false
    /// "undetermined", the exact inversion the field exists to prevent. Omitted when empty so a report
    /// with nothing to declare stays byte-identical to a pre-rung one.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub resolves: Vec<String>,
    /// ⟨0.29⟩ THE SCOPE — what this scan chose not to open, by class. See [`ExcludedClass`].
    /// NOT `skip_serializing_if`: an EMPTY list is a positive statement ("I looked and excluded nothing"),
    /// and ⟨0.27⟩ requires it to be emitted. Absence of the key must mean "this producer cannot answer",
    /// which is the ⟨0.26⟩ reading — a partial manifest answers worse than an absent one.
    #[serde(default)]
    pub excluded: Vec<ExcludedClass>,
    /// ⟨0.29⟩ what the PEEK found in those files. Omitted when no policy was configured (nothing was
    /// asked); empty when a policy was configured and the excluded files were clean under it.
    #[serde(rename = "outOfScope", default, skip_serializing_if = "Option::is_none")]
    pub out_of_scope: Option<Vec<OutOfScopeFinding>>,
    /// ⟨0.33⟩ …and the QUESTION the peek was put — see [`ScannedUnder`]. Immediately after `out_of_scope`
    /// (the answer it qualifies) and before `net_partners`, matching the reference engine's field order —
    /// a porting engine has one position to match rather than guessing where a new key belongs.
    #[serde(rename = "scannedUnder", default, skip_serializing_if = "Option::is_none")]
    pub scanned_under: Option<ScannedUnder>,
    /// ⟨0.31⟩ Omitted when nothing participated, so a project declaring no partners — or declaring some
    /// that never matched — is byte-identical to a pre-rung report. A declaration that changed nothing is
    /// not provenance.
    #[serde(rename = "netPartners", default, skip_serializing_if = "Option::is_none")]
    pub net_partners: Option<NetPartners>,
    pub functions: Vec<ReportEntry>,
}

/// ⟨0.33⟩ THE QUESTION THE PEEK WAS PUT (SPEC §2 ⟨0.33⟩) — the deny rules this scan HELD, in the
/// canonical EXPANDED form ([`candor_classify::policy::canonical_deny_set`], via the type-erased `String`s
/// here since this crate does not depend on `candor-classify`).
///
/// `ExcludedClass::peeked` is true only RELATIVE to a deny set: the ⟨0.29⟩ bound filters the peek to
/// effects the policy DENIES, so a class read under `deny Net` says nothing about `Exec` in those same
/// files. Without this key a consumer gating with a DIFFERENT deny set gets a definite answer to a
/// question nobody asked, and it fails OPEN on the `gate --report` route — past every ⟨0.32⟩ control,
/// because the class really WAS read.
///
/// **THE EMISSION RULE IS `out_of_scope`'s, deliberately the same one**: `None` (key omitted) when no
/// policy was configured, or over a policy this engine REFUSED — recording the rules a refused parse
/// produced would publish a question that was never put. `Some(ScannedUnder{deny: vec![]})` is a
/// different claim (*a policy stood and it denied nothing*), so the two states must not collapse into
/// one another.
///
/// **THE RULES ARE THE EXPANDED FORM THE MATCHER USED — post-alias — never the raw policy line.**
/// Recording effect NAMES would reintroduce the flattening defect ⟨0.30⟩ closed one layer out (`pure` has
/// no name, so a flattened set makes the strictest policy compare equal to the empty one); recording raw
/// text would let two configs spelling one rule differently compare unequal (§3.1's alias-expansion
/// byte-inequality). One element per RULE — a rule denying several effects is ONE element — deduplicated
/// and code-point sorted, so two runs of one policy (however its lines were ordered) produce one document.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct ScannedUnder {
    pub deny: Vec<String>,
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
/// ⟨0.28⟩ RESOLVE THE SINK TO ITS FINAL ARTIFACT BEFORE WRITING. `rename(2)` REPLACES a symlink rather
/// than following it, so an `artifacts/verdict.json` symlinked into a shared directory — an ordinary CI
/// layout — kept a previous run's `{"ok": true}` while this run's document landed on the link. A stale
/// green with a single `--gate-json` and no operator mistake. SPEC §3.3.1 states identity about
/// ARTIFACTS, and this family had implemented that in the comparison and nowhere in the write.
///
/// Follows a chain of links, and works for a DANGLING one (the target need not exist yet) — `canonicalize`
/// cannot be used for that reason. Bounded, so a symlink cycle cannot spin here.
pub fn resolve_sink_artifact(path: &Path) -> std::path::PathBuf {
    let mut cur = path.to_path_buf();
    for _ in 0..32 {
        match std::fs::symlink_metadata(&cur) {
            Ok(m) if m.file_type().is_symlink() => match std::fs::read_link(&cur) {
                Ok(t) => {
                    cur = if t.is_absolute() {
                        t
                    } else {
                        cur.parent().map(|d| d.join(&t)).unwrap_or(t)
                    };
                }
                Err(_) => return cur,
            },
            _ => return cur,
        }
    }
    cur
}

pub fn write_atomic(path: &Path, contents: &[u8]) -> std::io::Result<()> {
    // The TEMP FILE is a sibling of the RESOLVED artifact, so the rename stays within one filesystem
    // (rename(2) is only atomic there) and lands on the file the operator actually reads.
    let target = resolve_sink_artifact(path);
    // ⟨0.28⟩ A MULTIPLY-LINKED TARGET IS WRITTEN IN PLACE. `rename(2)` gives the destination a NEW inode,
    // so it silently breaks a hard link: an operator with two names for one verdict file gets the new
    // document at one name and a previous run's at the other — the stale green again, through the layout
    // rather than through a flag. In place costs the atomicity window, and that is the right trade here:
    // the reader of the OTHER name is not racing this write, they are reading a file this write was
    // supposed to update. Single-link targets — every ordinary case — keep temp+rename.
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        if std::fs::metadata(&target).map(|m| m.nlink() > 1).unwrap_or(false) {
            return std::fs::write(&target, contents);
        }
    }
    let tmp = target.with_extension(format!("tmp.{}", std::process::id()));
    std::fs::write(&tmp, contents)?;
    std::fs::rename(&tmp, &target)
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
    to_packaged_report_json_full(candor, package, functions, coverage, &[], None, &[], None, None, None)
}

/// ⟨proposed — Gap 2⟩ Like [`to_packaged_report_json_with_coverage`], additionally carrying the
/// `unanalyzed` list (the target source the scan couldn't see). An empty slice omits the field, so a
/// complete scan's report is byte-identical to a pre-rung one (the wire-compatibility contract).
#[allow(clippy::too_many_arguments)]   // the report envelope has that many optional blocks; a struct
                                       // parameter would only move the arity, and the typed sibling
                                       // already carries the same allow for the same reason.
pub fn to_packaged_report_json_full(
    candor: &ReportMeta,
    package: &str,
    functions: &[ReportEntry],
    coverage: Option<&Coverage>,
    unanalyzed: &[UnanalyzedUnit],
    analyzed: Option<&Analyzed>,
    excluded: &[ExcludedClass],
    out_of_scope: Option<&[OutOfScopeFinding]>,
    scanned_under: Option<&ScannedUnder>,
    net_partners: Option<&NetPartners>,
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
        /// ⟨0.27⟩ SPEC §2.1 — emitted by BOTH writers. A capability declaration that appeared on only one
        /// of an engine's report paths would make the same engine's omissions readable in one report and
        /// unreadable in another, which is worse than not declaring at all.
        #[serde(skip_serializing_if = "<[_]>::is_empty")]
        resolves: &'a [&'a str],
        /// ⟨0.29⟩ THE SCOPE. NOT `skip_serializing_if`: an empty list is a positive statement — "I
        /// looked and excluded nothing" — and ⟨0.27⟩ requires it emitted. Absence of the key must mean
        /// "this producer cannot answer" (⟨0.26⟩: a partial manifest answers worse than an absent one).
        /// Emitted by BOTH writers for the reason the `resolves` comment above already gives: a
        /// declaration on only one report path makes the same engine's omissions readable in one report
        /// and unreadable in another, which is worse than not declaring at all.
        excluded: &'a [ExcludedClass],
        /// ⟨0.29⟩ what the PEEK found. `None` (omitted) when no policy was configured — nothing was
        /// asked, so an empty list would be a claim. `Some([])` when a policy WAS configured and the
        /// excluded files were clean under it, which is a real answer and must be emitted.
        #[serde(rename = "outOfScope", skip_serializing_if = "Option::is_none")]
        out_of_scope: Option<&'a [OutOfScopeFinding]>,
        /// ⟨0.33⟩ …and the QUESTION the peek was put, immediately after the answer it qualifies — see
        /// [`ScannedUnder`]. Same `None`/`Some` emission rule as `out_of_scope` (SPEC §2 ⟨0.33⟩).
        #[serde(rename = "scannedUnder", skip_serializing_if = "Option::is_none")]
        scanned_under: Option<&'a ScannedUnder>,
        /// ⟨0.31⟩ after `outOfScope`/`scannedUnder`, before `functions` — one position, both engines'
        /// writers, so the key order a consumer sees does not depend on which engine produced the report.
        #[serde(rename = "netPartners", skip_serializing_if = "Option::is_none")]
        net_partners: Option<&'a NetPartners>,
        functions: &'a [ReportEntry],
    }
    serde_json::to_string_pretty(&Out {
        candor, package, coverage, analyzed, unanalyzed, resolves: RESOLVES, excluded, out_of_scope,
        scanned_under, net_partners, functions,
    })
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
    excluded: &[ExcludedClass],
    out_of_scope: Option<&[OutOfScopeFinding]>,
    scanned_under: Option<&ScannedUnder>,
    net_partners: Option<&NetPartners>,
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
        /// ⟨0.27⟩ SPEC §2.1 — the optional refinement surfaces this producer computes. Listed because
        /// candor-scan resolves `fs` read/write kinds; without the declaration a consumer cannot tell
        /// an omitted `fs` ("reached, kind undetermined") from an engine that never computes kinds.
        #[serde(skip_serializing_if = "<[_]>::is_empty")]
        resolves: &'a [&'a str],
        /// ⟨0.29⟩ THE SCOPE. NOT `skip_serializing_if`: an empty list is a positive statement — "I
        /// looked and excluded nothing" — and ⟨0.27⟩ requires it emitted. Absence of the key must mean
        /// "this producer cannot answer" (⟨0.26⟩: a partial manifest answers worse than an absent one).
        /// Emitted by BOTH writers for the reason the `resolves` comment above already gives: a
        /// declaration on only one report path makes the same engine's omissions readable in one report
        /// and unreadable in another, which is worse than not declaring at all.
        excluded: &'a [ExcludedClass],
        /// ⟨0.29⟩ what the PEEK found. `None` (omitted) when no policy was configured — nothing was
        /// asked, so an empty list would be a claim. `Some([])` when a policy WAS configured and the
        /// excluded files were clean under it, which is a real answer and must be emitted.
        #[serde(rename = "outOfScope", skip_serializing_if = "Option::is_none")]
        out_of_scope: Option<&'a [OutOfScopeFinding]>,
        /// ⟨0.33⟩ …and the QUESTION the peek was put — same position and emission rule as the untyped
        /// writer's (SPEC §2 ⟨0.33⟩).
        #[serde(rename = "scannedUnder", skip_serializing_if = "Option::is_none")]
        scanned_under: Option<&'a ScannedUnder>,
        /// ⟨0.31⟩ same position as the untyped writer puts it — after `outOfScope`/`scannedUnder`, before
        /// `functions` — so key order does not depend on which writer produced the report.
        #[serde(rename = "netPartners", skip_serializing_if = "Option::is_none")]
        net_partners: Option<&'a NetPartners>,
        functions: &'a [ReportEntry],
    }
    let ts = type_surface.filter(|t| !t.returns.is_empty());
    serde_json::to_string_pretty(&Out {
        candor, package, coverage, analyzed, unanalyzed, excluded, out_of_scope, scanned_under,
        net_partners, type_surface: ts, resolves: RESOLVES, functions,
    })
}

/// ⟨proposed: typeSurface⟩ Parse a report's `typeSurface`. Absent = nothing travelled, never an error.
pub fn report_type_surface(text: &str) -> Option<TypeSurface> {
    let val: serde_json::Value = serde_json::from_str(text).ok()?;
    serde_json::from_value(val.get("typeSurface")?.clone()).ok()
}

/// ⟨0.24⟩ THE THREE ANSWERS A §2 KEY CAN GIVE A VERDICT READER, kept apart because two of them were
/// being collapsed into one and the collapse was always in the fail-open direction.
///
/// SPEC §2: *"A KEY THAT IS PRESENT BUT UNPARSEABLE IS CORRUPT INPUT, AND MUST NEVER BE COERCED TO ITS
/// EMPTY VALUE. … That default is always the permissive value — `0`, `[]`, absent — so the coercion
/// converts corrupt input into a claim, and on every one of these keys the claim is the safe-looking
/// one. … `unwrap_or_default`, `?? []`, `optional(...).orElse(…)` and their siblings are the exact
/// idiom to grep for, and finding one on a §2 key is a defect until proven otherwise."*
///
/// [`Absent`](KeyRead::Absent) may take the key's documented default. [`Corrupt`](KeyRead::Corrupt)
/// may not: it is a refusal, exit 2, naming the key.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum KeyRead<T> {
    /// The key is not on the wire. Its documented default applies — for `unanalyzed` that is "nothing
    /// unanalyzed travelled", which is what every complete report this engine writes looks like
    /// (`skip_serializing_if = "Vec::is_empty"`).
    Absent,
    /// The key is present and read as written.
    Present(T),
    /// The key is PRESENT and could not be read as its documented type. Never a default.
    Corrupt,
}

impl<T> KeyRead<T> {
    /// The value when present, the caller's default when ABSENT — and `None` when CORRUPT, which is the
    /// case a bare `unwrap_or_default` erased. Callers that must refuse match on the variant instead.
    pub fn or_default_if_absent(self, absent: T) -> Option<T> {
        match self {
            KeyRead::Absent => Some(absent),
            KeyRead::Present(v) => Some(v),
            KeyRead::Corrupt => None,
        }
    }
}

/// Read one §2 envelope key strictly: ABSENT and PRESENT-BUT-UNPARSEABLE are different answers.
/// Unparseable report TEXT is `Corrupt` too — every caller on this path already refuses such a report
/// before asking, so that arm is a posture rather than a behaviour.
fn read_key<T: serde::de::DeserializeOwned>(text: &str, key: &str) -> KeyRead<T> {
    let Ok(val) = serde_json::from_str::<serde_json::Value>(text) else { return KeyRead::Corrupt };
    match val.get(key) {
        None => KeyRead::Absent,
        Some(v) => match serde_json::from_value::<T>(v.clone()) {
            Ok(t) => KeyRead::Present(t),
            Err(_) => KeyRead::Corrupt,
        },
    }
}

/// ⟨0.21⟩ Read a report's `unanalyzed` manifest (SPEC §2, Gap 2). ABSENT is a complete scan or a
/// pre-rung producer and takes the documented empty default; PRESENT-BUT-UNPARSEABLE is `Corrupt`.
///
/// **THIS IS THE SHARPEST OF THE §2 KEYS AND IT WAS THE ONE THAT WAS WRONG.** `unanalyzed`
/// NON-EMPTINESS *is* the fail-closed trigger, so coercing an unreadable one to `[]` does not merely
/// lose a disclosure — it converts exit 2 into `policy ✓`. Measured 2026-07-28 on
/// `unanalyzed: [{"unit":…,"why":…}]` (right shape, wrong field names — exactly what a hand-built or
/// foreign-produced report yields): this function's `from_value(u).ok().unwrap_or_default()` returned
/// `[]` and candor-rust exited 0 GREEN where ts, java and swift all refused. `unanalyzed: ["a.rs"]`, a
/// bare string list, went green in all four.
pub fn report_unanalyzed(text: &str) -> KeyRead<Vec<UnanalyzedUnit>> {
    read_key(text, "unanalyzed")
}

/// ⟨0.30⟩ Read a report's `outOfScope` peek findings — the functions a policy-configured scan looked at
/// AFTER excluding them and found performing an effect that policy DENIES. Non-empty makes the verdict
/// INCOMPLETE (exit 2), so this rides the STRICT path for `unanalyzed`'s reason: coerced to its empty
/// default, corrupt input becomes the claim *"I looked and nothing was there"*, which is the
/// safe-LOOKING value and the wrong one.
///
/// ABSENT is NOT empty and must stay `Absent`: ⟨0.26⟩ makes an absent key *"this producer cannot
/// answer"*, and a report produced with no policy was never asked the question — a pre-⟨0.30⟩ report
/// must not become exit 2 on contact.
pub fn report_out_of_scope(text: &str) -> KeyRead<Vec<OutOfScopeFinding>> {
    read_key(text, "outOfScope")
}

/// ⟨0.33⟩ Read a report's `scannedUnder.deny` — the canonical expanded deny rules THE PRODUCER'S PEEK
/// WAS BOUNDED BY (SPEC §2 ⟨0.33⟩). `ExcludedClass::peeked` is true only RELATIVE to this set: the
/// ⟨0.29⟩ bound filters the peek to effects the policy DENIES, so a class read under `deny Net` says
/// nothing about `Exec` in those same files, and a consumer's own deny set is compared against this one
/// to decide whether it was ever asked.
///
/// **ABSENT IS THE EMPTY SET FOR THE SUBSET TEST, and that is a deliberate fail-closed default — never a
/// licence.** SPEC §2 ⟨0.33⟩: *"an absent `scannedUnder` is the EMPTY SET for this test, so a pre-⟨0.33⟩
/// report carrying `peeked: true` fails closed."* Returned as `KeyRead::Absent` rather than
/// `Present(vec![])` so the caller applies its own documented default exactly as every other §2 key on
/// this route does (⟨0.24⟩'s three-answer rule).
///
/// **PRESENT-BUT-UNPARSEABLE — a non-object, or a `deny` that is not a list of strings — is `Corrupt`,
/// and the fail-open direction here is the MIRROR of `peeked`'s.** The safe-LOOKING coercion would be
/// "the producer held these rules" (an empty or partial list, read as the whole truth), which
/// MANUFACTURES coverage the producer never claimed — so a garbled `scannedUnder` must impeach the whole
/// document rather than shrink to a value a subset test could pass against. Twelve shapes are driven by
/// conformance PART 69's corruption arm; none may certify.
///
/// An object present with NO `deny` key reads as the EMPTY LIST (which then loses the subset test and
/// refuses) rather than `Corrupt`: the wrapper object standing at all is `outOfScope`'s "a policy stood"
/// claim, and it is only the rule set inside it that is missing — the same value a subset test already
/// treats as covering nothing.
pub fn report_scanned_under(text: &str) -> KeyRead<Vec<String>> {
    let Ok(val) = serde_json::from_str::<serde_json::Value>(text) else { return KeyRead::Corrupt };
    match val.get("scannedUnder") {
        None => KeyRead::Absent,
        Some(v) => {
            let Some(obj) = v.as_object() else { return KeyRead::Corrupt };
            match obj.get("deny") {
                None => KeyRead::Present(Vec::new()),
                Some(d) => match d.as_array() {
                    Some(arr) => {
                        let mut out = Vec::with_capacity(arr.len());
                        for item in arr {
                            match item.as_str() {
                                Some(s) => out.push(s.to_string()),
                                None => return KeyRead::Corrupt,
                            }
                        }
                        KeyRead::Present(out)
                    }
                    None => KeyRead::Corrupt,
                },
            }
        }
    }
}

/// ⟨0.32⟩ Parse a report's `excluded` envelope field — the SCOPE the producing scan recorded. The
/// `gate --report` route needs it for the unread-classes rule: a class the producer marked
/// `peeked: false`, and did not carve out as `judgedElsewhere`, is code that scan never read, and a
/// verdict over it is INCOMPLETE. READ, never recomputed — this route has no target to re-derive an
/// exclusion set from, the same reason `outOfScope` and `netPartners` ride the report.
pub fn report_excluded(text: &str) -> KeyRead<Vec<ExcludedClass>> {
    read_key(text, "excluded")
}

/// ⟨0.31⟩ Parse a report's `netPartners` envelope field — the ambient `net-partner` provenance the
/// PRODUCER recorded. Read, never recomputed: this route has no target to anchor `net-partner` at, and
/// re-classifying the report's hosts through the consumer's own config is the re-derivation ⟨0.24⟩
/// forbids — it would make the verdict depend on the reader's working directory.
pub fn report_net_partners(text: &str) -> KeyRead<NetPartners> {
    read_key(text, "netPartners")
}

/// ⟨0.15 staged⟩ Parse a report's `coverage` envelope field (spec §2). `None` when the field is
/// absent (a fully-covered scan, or any pre-⟨0.15⟩ report), the text isn't a JSON object, or the
/// field doesn't deserialize — absence of the ledger is never an error, just "no disclosure
/// travelled" (the pre-⟨0.15⟩ posture).
///
/// **THE LENIENT READER, kept for the ENRICHMENT callers** (`load.rs`, the query surfaces), where a
/// coverage note that cannot be read costs precision and nothing else. The VERDICT route reads
/// [`report_coverage_strict`] instead: there the κ ledger rides the gate document, so silently dropping
/// an unreadable one deletes a disclosure from a verdict a machine acts on — the same shape as
/// `unanalyzed`, one rung less sharp.
pub fn report_coverage(text: &str) -> Option<Coverage> {
    let val: serde_json::Value = serde_json::from_str(text).ok()?;
    serde_json::from_value(val.get("coverage")?.clone()).ok()
}

/// ⟨0.24⟩ [`report_coverage`] with ABSENT and PRESENT-BUT-UNPARSEABLE told apart, for the verdict route.
pub fn report_coverage_strict(text: &str) -> KeyRead<Coverage> {
    read_key(text, "coverage")
}

/// ⟨0.21⟩ Read a report's `analyzed` completeness manifest. ABSENT is a pre-⟨0.21⟩ producer and takes
/// the documented `count: 0` contribution; PRESENT-BUT-UNPARSEABLE (`"analyzed": "lots"`,
/// `{"count": true}`) is `Corrupt` — SPEC §2's stated shape-table row. Read by `candor-query gate
/// --report` (SPEC §3.1 ⟨0.24⟩), where the manifest that rode the report becomes the verdict's
/// `analyzed.count` — the same number the scan's own `--gate-json` carries, which is half of why the
/// two documents are byte-equal, and a number a reader must never invent.
pub fn report_analyzed(text: &str) -> KeyRead<Analyzed> {
    read_key(text, "analyzed")
}

/// ⟨0.24⟩ Does this report say it **JUDGED NOTHING** — is its ⟨0.21⟩ `analyzed.count` zero?
///
/// **THE DEFECT THIS ANSWERS.** A report carrying `functions: []` and `analyzed.count: 0` bought a
/// consumer MORE confidence than not having the report at all: the caller drops out of `functions`,
/// which under ⟨0.21⟩ is a POSITIVE PURITY CLAIM, while the same scan with nothing chained discloses
/// `invisible` + `coverage.uncovered`. Not a wrong answer — a *confident* one where the honest answer
/// was a hedge, and it is the DISCLOSURE channel, not the verdict, that reading this restores.
///
/// **KEYED ON THE INTEGER, NEVER ON THE EMPTINESS OF `functions`, and that is the whole design.**
/// `functions: []` is equally the shape of a LEGITIMATE all-pure dependency, whose empty report SPEC §2
/// chaining rule 3 requires a consumer to BELIEVE. `analyzed.count` is the only thing on the wire that
/// separates a `pub use`-only facade (count 0) from an all-pure two-function crate (count 2). Measured
/// over 1997 deduplicated JVM dependency jars: 79 (4.0%) emit count 0, of which only 6 granted any
/// coverage — but 104 (5.2%) are the legitimate all-pure kind. **A predicate keyed on emptiness would
/// have withdrawn 104 real claims to catch 6**: the plausible-but-wrong fix is more destructive than
/// the defect it "fixes", which is why SPEC §2's second table row is a CONTROL and not a footnote.
///
/// `has_entries` is whether the report lists any function (either wire shape — the `functions` array or
/// the v0.1 bare array). It is consulted for EXACTLY ONE row, the manifest-less one.
///
/// The rows, in the order they are decided:
/// - `analyzed` ABSENT and the report lists entries → a pre-⟨0.21⟩ producer that judged something and
///   said so the only way it could → NOT judged-nothing;
/// - `analyzed` ABSENT and no entries → SPEC §2's third row: nothing on the wire distinguishes "judged
///   nothing" from "judged and found nothing", and the unchained reading is the only honest one.
///   **This retires a pre-⟨0.21⟩ affordance on purpose** — such a report DID buy coverage before;
/// - `analyzed.count` numeric and ≤ 0 → judged nothing;
/// - `analyzed.count` numeric and > 0 → judged n, whatever `functions` says. The believed all-pure claim;
/// - `analyzed` present but UNREADABLE (a null, a string, an object with no numeric `count`) → a
///   judgment claim that cannot be READ is not a claim → fails CLOSED, the same posture candor-scan's
///   `declares_itself_incomplete` takes for a malformed `unanalyzed`.
///
/// Lives HERE, in the one crate both routes depend on, because ⟨0.24⟩ binds two of them — the chained
/// join (candor-scan `load_dep_reports`) and `candor-query gate --report` (SPEC §3.1) — and a predicate
/// written twice is a predicate that can drift between them.
pub fn claims_to_have_judged_nothing(val: &serde_json::Value, has_entries: bool) -> bool {
    match val.get("analyzed") {
        None => !has_entries,
        Some(a) => match a.get("count").and_then(serde_json::Value::as_f64) {
            Some(n) => n <= 0.0,
            None => true,
        },
    }
}

/// [`claims_to_have_judged_nothing`] over raw report TEXT, deriving `has_entries` from whichever wire
/// shape the report uses. Unparsable text fails CLOSED (it is not a judgment claim either); every caller
/// on this path refuses such a report before asking, so that value is a posture, not a behaviour.
pub fn report_judged_nothing(text: &str) -> bool {
    let Ok(val) = serde_json::from_str::<serde_json::Value>(text) else { return true };
    let has_entries = val
        .get("functions")
        .and_then(|x| x.as_array())
        .or_else(|| val.as_array())
        .is_some_and(|a| !a.is_empty());
    claims_to_have_judged_nothing(&val, has_entries)
}

/// ⟨0.28⟩ SPEC §2 — **THE THIRD ROW IS NOT THE FIRST ROW.** Does this report carry NO ⟨0.21⟩ `analyzed`
/// manifest at all?
///
/// **A SECOND, DISCLOSURE-ONLY PREDICATE, AND IT IS NOT AN INVERSION OF THE ONE ABOVE.** §2's three-row
/// table distinguishes `analyzed.count: 0` (row 1 — *nothing was judged*, a claim the report MAKES) from
/// `analyzed` ABSENT (row 3 — a pre-⟨0.21⟩ producer, which makes no claim at all). Both HEDGE, and
/// [`report_judged_nothing`] keeps saying so for both: it is what the chained join
/// (candor-scan `load_dep_reports`) and `gate --report` read to decide COVERAGE, and row 3's own
/// instruction is *no manifest, no claim* — an absent manifest must keep granting NO coverage. Making
/// that predicate answer `false` here to fix a LABEL would turn every pre-⟨0.21⟩ report into a covered
/// one: a silent under-report introduced by a disclosure fix.
///
/// So this asks a different question — *is there a manifest?* — and only the DISCLOSURE path
/// (`crate::report_judged_nothing`'s caller in candor-query's `completeness`) consults it, to route a
/// hedge that is already happening to the right key. `judgedNothing` is PINNED to *"reports declaring
/// `analyzed.count: 0`"*, which a row-3 report is not, and the two want different repairs: row 1 wants a
/// scan that reaches a conclusion, row 3 wants a producer that emits a manifest at all.
///
/// A legacy BARE ARRAY report has no envelope and therefore no manifest either — row 3 as well. It is
/// only ever HEDGED when it also lists nothing (the caller ANDs this with
/// [`report_judged_nothing`]); a manifest-less report that LISTS entries judged something and said so the
/// only way it could, and keeps the standing §2's manifest-absent row gives it.
///
/// Unparsable text answers `false`: a file whose bytes cannot be read did not "carry no manifest", it
/// carried nothing readable, and the `unreadable` arm is the actionable disclosure for it. (The opposite
/// posture from [`report_judged_nothing`], which fails CLOSED because it decides coverage.)
pub fn report_has_no_manifest(text: &str) -> bool {
    let Ok(val) = serde_json::from_str::<serde_json::Value>(text) else { return false };
    if val.is_array() {
        return true; // legacy bare array: no envelope, so no manifest either
    }
    val.is_object() && val.get("analyzed").is_none()
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
    /// ⟨0.32⟩ **THE UNIT THIS ROW IS ABOUT** — §2.2's join key, `package#fn`.
    ///
    /// SPEC §2 ⟨0.32⟩: *"a verdict row MUST carry enough identity for a consumer to tell two units
    /// apart… and the sort key MUST include that identity."* MEASURED here on a two-member workspace
    /// where both members violate `deny Exec`: two BYTE-IDENTICAL rows, `{rule, fn, effects, detail}`
    /// with nothing to attribute either to a package. A reader cannot tell two broken members from one
    /// listed twice, and a consumer that fingerprints on name alone — candor's own SARIF action did —
    /// hides one finding behind the other.
    ///
    /// **`hash` AND NOT `package` OR `loc`**, because §2.2 already binds a consumer to join a verdict
    /// row back to its report entry BY HASH. A row that omits it forces exactly the name join that
    /// clause forbids, and names are not unique even within one report: an inherent method and a trait
    /// implementation of the same name emit two entries sharing `fn`.
    ///
    /// **BESIDE `fn`, NEVER INSTEAD OF IT.** The NAME is what a policy scope matches (`deny Exec app::`)
    /// and what a human reads; replacing it with the qualified form would silently stop every scoped
    /// rule matching — a false green introduced by fixing a false green.
    ///
    /// Omitted when empty, which is a row whose producer had no unit identity to give: a report with no
    /// `hash` key (a hand-authored one, which §3.1 says this verb serves) and any pre-⟨0.32⟩ record read
    /// back off the NDJSON lint route. Absent is *"this producer cannot answer"*, never a fabricated id.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub hash: String,
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
    gate_verdict_json_with_coverage_v28(violations, coverage, &[])
}

/// ⟨0.28⟩ [`gate_verdict_json_with_coverage`] plus the §6.2 `ignored` disclosure, for the assembled
/// (lint-route) verdict `candor-query gate-verdict` writes — that route's verdict has no `analyzed`
/// envelope, so it cannot ride [`gate_verdict_json_v28`]. Empty ⇒ the key is omitted and the document
/// is byte-identical to the pre-⟨0.28⟩ form.
pub fn gate_verdict_json_with_coverage_v28(
    violations: &mut [GateViolation],
    coverage: Option<&GateCoverage>,
    ignored: &[IgnoredLine],
) -> serde_json::Result<String> {
    // ⟨0.32⟩ …AND `hash` IS PART OF THE KEY (SPEC §2). `(rule, detail)` TIES on two units that share a
    // name — a two-member workspace where both violate `deny Exec` produces twin rows differing only in
    // identity — and §3.3.1 makes the document's ORDER part of the byte-equality between `scan --policy`
    // and `gate --report`. The two routes accumulate in different orders (the scan gates member by
    // member and concatenates; the report route gates one merged unit set), so a tie left unbroken lets
    // them emit the same findings as unequal documents. Identity in the row without identity in the key
    // is half a fix.
    violations.sort_by(|a, b| {
        (a.rule.as_str(), a.detail.as_str(), a.hash.as_str())
            .cmp(&(b.rule.as_str(), b.detail.as_str(), b.hash.as_str()))
    });
    #[derive(Serialize)]
    struct Verdict<'a> {
        spec: &'static str,
        ok: bool,
        violations: &'a [GateViolation],
        /// ⟨0.28⟩ SPEC §6.2 — the policy lines the parse dropped; omitted when nothing was.
        #[serde(skip_serializing_if = "<[_]>::is_empty")]
        ignored: &'a [IgnoredLine],
        #[serde(skip_serializing_if = "Option::is_none")]
        coverage: Option<&'a GateCoverage>,
    }
    serde_json::to_string_pretty(&Verdict {
        spec: SPEC_VERSION,
        ok: violations.is_empty(),
        violations,
        ignored,
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
    gate_verdict_json_v24(violations, coverage, analyzed_count, unanalyzed, None)
}

/// ⟨0.24⟩ THE AMBIENT-VOCABULARY DISCLOSURE (SPEC §3.1): the `.candor/config` whose `unknown-alias`
/// definitions a policy rule actually RESOLVED THROUGH, and the alias names it used.
///
/// A verdict is supposed to be a function of the report and the policy. An `unknown-alias` beside the
/// policy moves it 0→1, and discovery WALKS PARENT DIRECTORIES, so a file anywhere above participates —
/// the fourth channel §3.1's MUST NOT never named. The ruling is not to forbid the input (an alias IS
/// policy vocabulary, and vocabulary belongs to the policy) but to make it **unable to act unnamed**.
///
/// OMITTED unless an alias was actually used, so a verdict from a policy that mentions none is
/// byte-identical to a pre-⟨0.24⟩ one — and a config defining ten unused aliases stays out, because
/// naming a file that changed nothing trains the reader to ignore the field.
///
/// **THE WIRE KEY IS `policyVocabulary`, AND THAT IS A SPEC MUST** (§3.1 ⟨0.24⟩, `b4e9155`). I required
/// the disclosure and specified no shape, and three engines invented three names within the hour —
/// `vocabulary` here, `policyVocabulary` in java, `configSources: [path]` in swift. The clause picks
/// `policyVocabulary` because the verdict already carries other vocabularies (effects, reason classes)
/// and the bare word does not say WHOSE; and it pins the OBJECT form because a disclosure naming the
/// source file but not the alias content leaves the reader knowing they were affected and not how. This
/// engine was the outlier on the NAME. It was **not** already right on the shape, as this comment claimed
/// for three days — see `aliases` below: the ENVELOPE was an object and the `aliases` VALUE was an array,
/// and `7f5b5ba` ruled against the array on the very sentence quoted above.
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct GateVocabulary {
    /// The canonical path of the `.candor/config` the aliases came from. Canonical because the scan and
    /// `gate --report` routes reach the same file from different working directories, and §3.1's
    /// byte-equality MUST is about the DOCUMENT.
    pub config: String,
    /// ⟨0.24⟩ **EACH ALIAS NAME → THE REASON-CLASS TOKENS IT EXPANDS TO — AN OBJECT, AND THAT IS A SPEC
    /// MUST** (§3.1, candor-spec `7f5b5ba`): `{"corp": ["native", "reflect"]}`. Keys sorted and each
    /// value sorted, so the document is deterministic across runs and across the scan / `gate --report`
    /// routes that §3.1 requires to be byte-equal.
    ///
    /// This engine shipped the bare-name array `["corp"]`, as did java and swift; candor-ts kept the
    /// object and won on the clause's OWN sentence rather than on a headcount. `configSources: [path]`
    /// is rejected above because *a disclosure that names the source but not the content leaves the
    /// reader knowing they were affected and not how* — and `["corp"]` fails that same test one level
    /// down. **`corp = reflect` and `corp = reflect,native` gate DIFFERENTLY under one unchanged policy
    /// line**, so a reader given only the NAME cannot tell which gate ran, which is exactly what this
    /// disclosure exists to prevent. The object is a strict SUPERSET — its keys are the old array — so
    /// no consumer of the array form loses anything, and the `config` path is untouched beside it.
    pub aliases: std::collections::BTreeMap<String, std::collections::BTreeSet<String>>,
}

/// ⟨0.24⟩ ONE POLICY RULE THE VERDICT DID NOT ANSWER (SPEC §3.1 `fc4b5f6`).
///
/// **THE DEFECT.** The exit-1 clause *"the refusal message MUST still disclose which rules could not be
/// evaluated"* named no field, no shape and no channel, and this engine put the disclosure on **stderr
/// only**. Measured 2026-07-28 on `deny Fs` + `allow Fs /var/data`, exit 1: java and ts emit `unevaluated`
/// in the `--gate-json` document, rust emitted nothing there. **A machine consumer of rust's exit-1
/// verdict could not see that any rule went unanswered at all** — a finding that never reaches the
/// consumer, arriving through the very disclosure this rung added to stop that. stderr is not the machine
/// channel; that is the same distinction that made the incomplete-analysis defect a defect.
///
/// **ONE ENTRY PER RULE, `rule` VERBATIM.** java's aggregate (`"forbid (× 2)"`) answers *how many* when
/// the operator's question is *which*, so it satisfies a naive reading of "disclose which rules" while
/// answering the other one. Two `forbid` lines are two entries here, each carrying its own raw line; where
/// the REASON is a property of the rule KIND rather than of the individual rule, the same `why` simply
/// repeats — which costs bytes and loses nothing, and is candor-ts's shape (the pinned reference).
///
/// Omitted from both documents when empty, so a policy the verb answers in full stays byte-identical.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct Unevaluated {
    /// The RAW policy line, verbatim — the operator's own text, so they can find it in their file.
    pub rule: String,
    /// Why this run could not decide it.
    pub why: String,
}

/// ⟨0.28⟩ ONE POLICY LINE THE PARSE DROPPED (SPEC §6.2's `ignored` disclosure).
///
/// **THE DEFECT.** The line-level leniency is correct — an unrecognized or malformed line is
/// ignored-with-a-warning, never silently reinterpreted — but every engine's warning went to stderr
/// while the verdict document stayed silent, so a machine consumer reading `{ok: true, violations: []}`
/// could not see that the gate it was reading was SMALLER than the gate that was written. The refusal
/// fires only at ZERO survivors, so a 9-of-10-dropped policy was a 90%-gateless green with nothing on
/// the machine channel at every fraction below 100%.
///
/// DISTINCT from [`Unevaluated`], and the distinction is load-bearing: `unevaluated` carries rules that
/// PARSED and could not be answered; this carries text that never became a rule at all. A consumer that
/// sees neither is entitled to believe the policy on disk is the policy that ran.
///
/// Omitted from the verdict when nothing was dropped, so a clean policy's verdict stays byte-identical.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct IgnoredLine {
    /// The 1-based source line in the policy file.
    pub line: usize,
    /// The source line, VERBATIM (before comment-stripping and trimming).
    pub text: String,
    /// Why the parse dropped it — the same sentence the stderr warning carries.
    pub reason: String,
}

/// [`gate_verdict_json_full`] plus the ⟨0.24⟩ `policyVocabulary` disclosure. Appended LAST so that every
/// verdict without ambient vocabulary — which is nearly all of them — stays byte-identical.
pub fn gate_verdict_json_v24(
    violations: &mut [GateViolation],
    coverage: Option<&GateCoverage>,
    analyzed_count: usize,
    unanalyzed: &[UnanalyzedUnit],
    vocabulary: Option<&GateVocabulary>,
) -> serde_json::Result<String> {
    gate_verdict_json_v27(violations, coverage, analyzed_count, unanalyzed, vocabulary, &[], &[])
}

/// ⟨0.28⟩ [`gate_verdict_json_v27`] plus the §6.2 `ignored` disclosure: the policy lines the parse
/// DROPPED, `[{line, text, reason}]`, omitted when nothing was dropped (a clean policy's verdict is
/// byte-identical to the v27 form). It rides VERDICT documents only, like `zeroMatch` — the refusal
/// document has its own whole-policy `unevaluated` entry, and a refused run has no verdict for a
/// dropped line to have shrunk. Disclosure only: `ok` and the exit code do not consult it.
#[allow(clippy::too_many_arguments)]
pub fn gate_verdict_json_v28(
    violations: &mut [GateViolation],
    coverage: Option<&GateCoverage>,
    analyzed_count: usize,
    unanalyzed: &[UnanalyzedUnit],
    vocabulary: Option<&GateVocabulary>,
    unevaluated: &[Unevaluated],
    zero_match: &[String],
    ignored: &[IgnoredLine],
) -> serde_json::Result<String> {
    gate_verdict_json_impl(
        violations, coverage, analyzed_count, unanalyzed, vocabulary, unevaluated, zero_match, ignored,
        &[], &[], &[], &[],
    )
}

/// ⟨0.30⟩ [`gate_verdict_json_v28`] plus the `outOfScope` findings — the peeked functions performing an
/// effect the policy DENIES. Non-empty makes the verdict INCOMPLETE (`ok:false`, `incomplete:true`,
/// exit 2), reversing ⟨0.29⟩'s "an out-of-scope finding MUST NOT move the verdict" on the measurement
/// that the peek resolves a CONCRETE denied effect rather than uncertainty. Omitted when empty, so a
/// clean verdict stays byte-identical to the v28 form.
#[allow(clippy::too_many_arguments)]
pub fn gate_verdict_json_v30(
    violations: &mut [GateViolation],
    coverage: Option<&GateCoverage>,
    analyzed_count: usize,
    unanalyzed: &[UnanalyzedUnit],
    vocabulary: Option<&GateVocabulary>,
    unevaluated: &[Unevaluated],
    zero_match: &[String],
    ignored: &[IgnoredLine],
    out_of_scope: &[OutOfScopeFinding],
) -> serde_json::Result<String> {
    gate_verdict_json_impl(
        violations, coverage, analyzed_count, unanalyzed, vocabulary, unevaluated, zero_match, ignored,
        out_of_scope, &[], &[], &[],
    )
}

/// ⟨0.31⟩ [`gate_verdict_json_v30`] plus the `netPartners` disclosure: the ambient `net-partner`
/// declarations that MOVED a classification, copied from the report the producer wrote.
///
/// A LIST because a `--report` prefix can match several reports (a workspace writes one per member) and
/// each anchors its own config; a single report carries a single record. Omitted when empty, so every
/// verdict without ambient partner vocabulary — nearly all of them — stays byte-identical to the v30 form.
#[allow(clippy::too_many_arguments)]
pub fn gate_verdict_json_v31(
    violations: &mut [GateViolation],
    coverage: Option<&GateCoverage>,
    analyzed_count: usize,
    unanalyzed: &[UnanalyzedUnit],
    vocabulary: Option<&GateVocabulary>,
    unevaluated: &[Unevaluated],
    zero_match: &[String],
    ignored: &[IgnoredLine],
    out_of_scope: &[OutOfScopeFinding],
    net_partners: &[NetPartners],
    // ⟨0.32⟩ exclusion classes the scan did not READ — the caller supplies it from GATE_UNPEEKED.
    unpeeked: &[String],
) -> serde_json::Result<String> {
    // ⟨0.33⟩ `&[]`: this function is kept for source-compat with any caller that has not been ported to
    // [`gate_verdict_json_v33`] below, and on every such caller the cross-policy cause is inapplicable —
    // it is the SCAN route's own verdict writer (candor-scan's producer and consumer are one run, so
    // `P ⊆ P` holds by construction, SPEC §2 ⟨0.33⟩) or a pre-⟨0.33⟩ call site.
    gate_verdict_json_impl(
        violations, coverage, analyzed_count, unanalyzed, vocabulary, unevaluated, zero_match, ignored,
        out_of_scope, net_partners, unpeeked, &[],
    )
}

/// ⟨0.33⟩ [`gate_verdict_json_v31`] plus the SPEC §2 ⟨0.33⟩ CROSS-POLICY cause: the rules THIS gate's
/// policy holds that some peeked class's producer was never asked about — `Query::unasked_rules`'s
/// result, canonical and code-point sorted.
///
/// Non-empty makes the verdict INCOMPLETE (`ok:false`, `incomplete:true`, exit 2) exactly as `unpeeked`
/// does, and for the identical reason: the document and the exit code must be two readings of ONE value,
/// never a condition stated at the exit arm alone (that split is what shipped `"ok": false, "incomplete":
/// true"` AT EXIT 0 on the ⟨0.32⟩ rung one step back).
///
/// **NO NEW WIRE KEY** — matching the reference engine (candor-java's `Query.GateFacts.unaskedRules` also
/// feeds only `incomplete`/`ok`, with no new serialized field): the rung's contract is the EXIT/`ok`
/// behaviour, and a document with nothing unasked stays byte-identical to the v31 form.
///
/// Empty (`&[]`) on the SCAN route BY CONSTRUCTION: `scan --policy` is its own producer and consumer, so
/// the recorded set IS this policy and the subset test cannot fail (§3.1 route equality holds with no new
/// anchor, unlike the reverted `net-partner` attempt).
#[allow(clippy::too_many_arguments)]
pub fn gate_verdict_json_v33(
    violations: &mut [GateViolation],
    coverage: Option<&GateCoverage>,
    analyzed_count: usize,
    unanalyzed: &[UnanalyzedUnit],
    vocabulary: Option<&GateVocabulary>,
    unevaluated: &[Unevaluated],
    zero_match: &[String],
    ignored: &[IgnoredLine],
    out_of_scope: &[OutOfScopeFinding],
    net_partners: &[NetPartners],
    unpeeked: &[String],
    unasked_rules: &[String],
) -> serde_json::Result<String> {
    gate_verdict_json_impl(
        violations, coverage, analyzed_count, unanalyzed, vocabulary, unevaluated, zero_match, ignored,
        out_of_scope, net_partners, unpeeked, unasked_rules,
    )
}

/// ⟨0.27⟩ [`gate_verdict_json_v24`] plus the two disclosure lists a composed verdict can carry beside its
/// violations: `unevaluated` (what could not be decided) and `zeroMatch` (what was decided over nothing).
///
/// **THE `refused`/`reason` KEYS THIS FUNCTION USED TO TAKE ARE GONE, AND THAT IS A SPEC RULING, not a
/// cleanup** (§3.1 ⟨0.27⟩, the composed-document clause). This engine put `refused: true` beside
/// `violations` on the run that holds both a certain violation and a policy refusal — java did too, and
/// the four engines wrote FOUR spellings of that one document. But `refused: true` is the refusal
/// document's discriminator, and its pinned meaning is *"the gate is making no claim about violations"* —
/// which is precisely the claim a violations-bearing document IS making. A consumer keying on `refused`
/// (which the refusal-document clause invites) would file a certain violation under "no claim about
/// violations". So the two shapes are disjoint on `refused`: a document that carries `violations` is a
/// VERDICT, and the refusal that stood beside the dominating violation travels as `unevaluated` — one
/// entry per rule of the refused policy, so no rule silently reads as evaluated-and-passed. The earlier
/// comment here argued the opposite ("dropping the second half would be the mirror defect") and was right
/// about the harm and wrong about the channel: the disclosure belongs in `unevaluated`, not in the other
/// document's discriminator key.
///
/// `zeroMatch` (§4 ⟨0.27⟩): the raw text of every rule whose SCOPE bound no function — code-point sorted,
/// deduplicated, omitted when empty. It was stderr-only in all five engines, so a machine consumer could
/// not see that a rule bound nothing — the silently-green-typo blindness, one channel over. It rides
/// VERDICT documents only, never the refusal document (a refused run evaluated nothing, so it is not
/// entitled to "this rule was evaluated and bound nothing").
#[allow(clippy::too_many_arguments)]
pub fn gate_verdict_json_v27(
    violations: &mut [GateViolation],
    coverage: Option<&GateCoverage>,
    analyzed_count: usize,
    unanalyzed: &[UnanalyzedUnit],
    vocabulary: Option<&GateVocabulary>,
    unevaluated: &[Unevaluated],
    zero_match: &[String],
) -> serde_json::Result<String> {
    gate_verdict_json_impl(violations, coverage, analyzed_count, unanalyzed, vocabulary, unevaluated, zero_match, &[], &[], &[], &[], &[])
}

/// The ONE verdict writer behind [`gate_verdict_json_v27`]/[`gate_verdict_json_v28`] — a single field
/// list, so the two rungs cannot drift on shape.
#[allow(clippy::too_many_arguments)]
fn gate_verdict_json_impl(
    violations: &mut [GateViolation],
    coverage: Option<&GateCoverage>,
    analyzed_count: usize,
    unanalyzed: &[UnanalyzedUnit],
    vocabulary: Option<&GateVocabulary>,
    unevaluated: &[Unevaluated],
    zero_match: &[String],
    ignored: &[IgnoredLine],
    out_of_scope: &[OutOfScopeFinding],
    net_partners: &[NetPartners],
    // ⟨0.32⟩ exclusion classes the scan did not READ, pre-filtered by the producer.
    unpeeked: &[String],
    // ⟨0.33⟩ the rules THIS gate's policy holds that some peeked class's producer was never asked about
    // (SPEC §2 ⟨0.33⟩) — canonical, deduplicated and code-point sorted. NOT its own wire key (see
    // `gate_verdict_json_v33`); it only widens `incomplete`/`ok`, exactly as the reference engine does.
    unasked_rules: &[String],
) -> serde_json::Result<String> {
    // ⟨0.32⟩ …AND `hash` IS PART OF THE KEY (SPEC §2). `(rule, detail)` TIES on two units that share a
    // name — a two-member workspace where both violate `deny Exec` produces twin rows differing only in
    // identity — and §3.3.1 makes the document's ORDER part of the byte-equality between `scan --policy`
    // and `gate --report`. The two routes accumulate in different orders (the scan gates member by
    // member and concatenates; the report route gates one merged unit set), so a tie left unbroken lets
    // them emit the same findings as unequal documents. Identity in the row without identity in the key
    // is half a fix.
    violations.sort_by(|a, b| {
        (a.rule.as_str(), a.detail.as_str(), a.hash.as_str())
            .cmp(&(b.rule.as_str(), b.detail.as_str(), b.hash.as_str()))
    });
    // ⟨0.31⟩ …AND `outOfScope` FOR THE SAME REASON, which nothing was doing. §3.1 is BYTE equality, so
    // the ORDER of this list is part of the contract, and the two routes arrive at it differently: the
    // scan route accumulates across workspace members in the order it scans them, while `gate --report`
    // reads one report per package in the order the locator expands. Same findings, different sequence,
    // unequal documents.
    //
    // MEASURED on ripgrep under `deny Fs` (corpus round, 2026-08-20): both routes exit 1 and both carry
    // the same 16 findings, but `examples::walk::main` sits at the front on one route and the back on the
    // other. Byte-identical entries, unequal documents — the diff is pure sequence. Sorting HERE is what
    // makes them agree regardless of how each got there, which is the same reason `violations` is sorted
    // on the line above rather than at each call site.
    let mut out_of_scope_sorted = out_of_scope.to_vec();
    out_of_scope_sorted.sort_by(|a, b| {
        (a.path.as_str(), a.func.as_str(), a.class.as_str(), &a.effects)
            .cmp(&(b.path.as_str(), b.func.as_str(), b.class.as_str(), &b.effects))
    });
    let out_of_scope = &out_of_scope_sorted[..];
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
        /// ⟨0.24⟩ SPEC §3.1 `fc4b5f6` — the rules the verdict above does NOT answer. Beside the
        /// violations, never instead of them: a firing rule is certain regardless of how these would
        /// have resolved (Lemma 2), and exit 1 reports the violation it is sure of without concealing
        /// the part it could not read.
        #[serde(skip_serializing_if = "<[_]>::is_empty")]
        unevaluated: &'a [Unevaluated],
        /// ⟨0.27⟩ SPEC §4/§3.1 — the rules whose scope bound NO function, verbatim. Disclosure only:
        /// `ok` and the exit code are computed without consulting it.
        #[serde(rename = "zeroMatch", skip_serializing_if = "<[_]>::is_empty")]
        zero_match: &'a [String],
        /// ⟨0.28⟩ SPEC §6.2 — the policy lines the parse DROPPED (see [`IgnoredLine`]). Disclosure
        /// only, omitted when nothing was dropped.
        #[serde(skip_serializing_if = "<[_]>::is_empty")]
        ignored: &'a [IgnoredLine],
        #[serde(skip_serializing_if = "Option::is_none")]
        coverage: Option<&'a GateCoverage>,
        #[serde(skip_serializing_if = "std::ops::Not::not")]
        incomplete: bool,
        #[serde(skip_serializing_if = "<[_]>::is_empty")]
        unanalyzed: &'a [UnanalyzedUnit],
        /// ⟨0.30⟩ SPEC §2 — the peeked functions performing an effect this policy DENIES. They are the
        /// SECOND cause of `incomplete` (the first is `unanalyzed`), and they are never `violations`:
        /// the gate did not judge them, so exit 2 says "I could not see enough", not "you violated".
        #[serde(rename = "outOfScope", skip_serializing_if = "<[_]>::is_empty")]
        out_of_scope: &'a [OutOfScopeFinding],
        // §3.1 ⟨0.24⟩ pins the WIRE key as `policyVocabulary`; the local binding keeps the short name.
        #[serde(rename = "policyVocabulary", skip_serializing_if = "Option::is_none")]
        vocabulary: Option<&'a GateVocabulary>,
        /// ⟨0.31⟩ the ambient `net-partner` declarations that moved a classification — copied from the
        /// report, never recomputed: `gate --report` has no target to anchor `net-partner` at, and
        /// re-classifying through the consumer's own config is the re-derivation ⟨0.24⟩ forbids.
        #[serde(rename = "netPartners", skip_serializing_if = "<[_]>::is_empty")]
        net_partners: &'a [NetPartners],
    }
    // ⟨0.30⟩ EITHER cause suppresses `ok`. `unanalyzed` is "I opened this file and could not read it";
    // `out_of_scope` is "I never opened it, and when I peeked afterwards it performed the denied effect".
    // Both mean the gate could not see enough of this tree to certify it.
    // ⟨0.32⟩ THE THIRD CAUSE — a class this scan did not READ. ⟨0.30⟩ keys on what the peek FOUND, and
    // a peek that could not open a file finds nothing, which is byte-identical to finding it clean.
    // `unpeeked` arrives already filtered by the producer's `judged_elsewhere` carve-out and by whether
    // the peek RAN, both applied at the recording site in candor-scan.
    // ⟨0.33⟩ THE FOURTH CAUSE — a class the producer's peek READ, but under a DIFFERENT deny set than
    // this gate holds (SPEC §2 ⟨0.33⟩). `unanalyzed`/`out_of_scope`/`unpeeked` each name a MORE concrete
    // gap; this is the residual case where the class really was read and found clean, but clean under a
    // question nobody here asked. Empty on the scan route by construction (§3.1 route equality, `P ⊆ P`).
    let incomplete =
        !unanalyzed.is_empty() || !out_of_scope.is_empty() || !unpeeked.is_empty() || !unasked_rules.is_empty();
    serde_json::to_string_pretty(&Verdict {
        spec: SPEC_VERSION,
        ok: violations.is_empty() && !incomplete,
        analyzed: Count { count: analyzed_count },
        violations,
        unevaluated,
        zero_match,
        ignored,
        coverage,
        incomplete,
        unanalyzed,
        out_of_scope,
        net_partners,
        vocabulary,
    })
}

/// ⟨0.24⟩ THE REFUSAL DOCUMENT (SPEC §3.1) — what a gate writes to `--gate-json` when it could not
/// evaluate the policy AS WRITTEN.
///
/// **THE HAZARD THIS CLOSES.** A refusal used to write NOTHING at the requested path, so a CI wrapper
/// that reads that path unconditionally re-read **the PREVIOUS run's document as current** — yesterday's
/// green file, still on disk, becomes today's all-clear. Deleting the path is not the fix either: a
/// consumer that treats a missing file as "nothing to report" fails open by a different route. The only
/// safe answer is a document whose NAIVE read is the fail-closed one.
///
/// **THE SHAPE IS A MUST, AND THE ABSENT KEY IS THE LOAD-BEARING PART.** `ok: false` so a consumer
/// keying only on `ok` lands on FAIL; `refused: true` + `reason` so one that looks further learns why;
/// and **no `violations` key at all** — the gate is making no claim about violations here, and `[]` is
/// precisely the claim it cannot make. An empty array would be read by every consumer in existence as
/// "we looked and found none", which is the fabrication this whole format refuses.
///
/// Deliberately MINIMAL — no `analyzed`, no `coverage`, no manifest. Those fields all describe a
/// judgment that was made; this document exists to say one was not. The stderr channel still carries the
/// full disclosure (which rules could not be evaluated, and any completeness note alongside).
pub fn gate_refusal_json(reason: &str) -> serde_json::Result<String> {
    gate_refusal_json_v24(reason, &[])
}

/// ⟨0.24⟩ [`gate_refusal_json`] carrying the `unevaluated` disclosure (SPEC §3.1 `fc4b5f6`).
///
/// A SOLE refusal is the case where the disclosure matters most: nothing fired, so `reason` is all the
/// consumer has, and a prose reason is not a list of rules. Empty ⇒ the key is omitted and the document is
/// byte-identical to the minimal refusal — which keeps the shape's load-bearing property intact, since
/// what makes a refusal document safe is the ABSENT `violations` key, not the absence of every other one.
pub fn gate_refusal_json_v24(reason: &str, unevaluated: &[Unevaluated]) -> serde_json::Result<String> {
    #[derive(Serialize)]
    struct Refusal<'a> {
        spec: &'static str,
        ok: bool,
        refused: bool,
        reason: &'a str,
        #[serde(skip_serializing_if = "<[_]>::is_empty")]
        unevaluated: &'a [Unevaluated],
    }
    serde_json::to_string_pretty(&Refusal {
        spec: SPEC_VERSION,
        ok: false,
        refused: true,
        reason,
        unevaluated,
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
    fn the_verdict_does_not_depend_on_the_order_the_findings_arrived_in() {
        // §3.1 is BYTE equality between `scan --policy` and `gate --report`, so the ORDER of
        // `outOfScope` is part of the contract — and the two routes cannot be relied on to build the
        // list the same way. The scan route accumulates across workspace members as it scans them; the
        // gate route reads one report per package in whatever order the locator expands. Same findings,
        // different sequence, unequal documents, and a §3.1 violation that no fixture in this repo could
        // show because candor's own crates do not have that shape.
        //
        // FOUND BY THE CORPUS ROUND on ripgrep under `deny Fs`: both routes exit 1 and both carry the
        // same 16 findings, with `examples::walk::main` at the front on one route and the back on the
        // other. `bin/corpus.sh` is not run by any CI workflow, so this test is the part that is.
        let mk = |func: &str, path: &str| OutOfScopeFinding {
            func: func.into(), path: path.into(), effects: vec!["Fs".into()],
            class: "non-library-target".into(), reason: "outside this scan's scope".into(),
        };
        let a = [mk("examples::walk::main", "examples/walk.rs"),
                 mk("tests::misc::x", "tests/misc.rs"),
                 mk("tests::util::y", "tests/util.rs")];
        let mut b = a.to_vec();
        b.reverse();

        let render = |oos: &[OutOfScopeFinding]| {
            gate_verdict_json_impl(&mut [], None, 1, &[], None, &[], &[], &[], oos, &[], &[], &[]).unwrap()
        };
        assert_eq!(render(&a), render(&b),
                   "the same findings in a different order produced different verdict documents. That is \
                    a §3.1 byte-equality break between the scan and gate routes, and it is invisible to \
                    every other check here: both documents are correct, complete and equally readable — \
                    they simply are not equal.");
        // …and not by collapsing the list to nothing, which would satisfy the assertion above while
        // deleting the disclosure.
        assert!(render(&a).contains("examples::walk::main") && render(&a).contains("tests::misc::x"),
                "every finding must still be present: {}", render(&a));
    }

    #[test]
    fn unanalyzed_round_trips_and_omits_when_empty() {
        // Gap 2: the `unanalyzed` field carries the target source the scan couldn't see into the wire,
        // and is OMITTED when empty (byte-compat with a pre-rung report).
        let meta = ReportMeta { version: "v".into(), toolchain: "t".into(), spec: SPEC_VERSION.into() };
        // present: a parse failure travels
        let units = vec![UnanalyzedUnit { path: "src/broken.rs".into(), reason: "source failed to read/parse".into() }];
        let json = to_packaged_report_json_full(&meta, "p", &[], None, &units, None, &[], None, None, None).unwrap();
        assert!(json.contains("\"unanalyzed\""), "unanalyzed must serialize when non-empty");
        let KeyRead::Present(parsed) = report_unanalyzed(&json) else { panic!("must read back") };
        assert_eq!(parsed.len(), 1);
        assert_eq!(parsed[0].path, "src/broken.rs");
        // empty: omitted entirely (a complete scan is byte-identical to a pre-rung report)
        let clean = to_packaged_report_json_full(&meta, "p", &[], None, &[], None, &[], None, None, None).unwrap();
        assert!(!clean.contains("unanalyzed"), "an empty unanalyzed must be omitted");
        assert_eq!(report_unanalyzed(&clean), KeyRead::Absent, "…and reads back as ABSENT, not as corrupt");
    }

    /// ⟨0.24⟩ SPEC §2's SHAPE TABLE for the §2 keys a VERDICT reads: **ABSENT and PRESENT-BUT-UNPARSEABLE
    /// are different answers, and only the first may take a default.** Every row here returned the
    /// PERMISSIVE default before the fix, because `report_unanalyzed` ended in
    /// `.ok().unwrap_or_default()`.
    ///
    /// `unanalyzed` is the sharp one: its NON-EMPTINESS is the fail-closed trigger, so `[]` is not a lost
    /// hedge, it is an inverted verdict (`candor-query gate --report` exited 0 `policy ✓` where ts, java
    /// and swift exited 2 — measured 2026-07-28). The `analyzed` rows are SPEC §2's own stated table,
    /// including the boolean that candor-swift really did read as `1`.
    #[test]
    fn a_present_but_unparseable_section2_key_is_corrupt_and_an_absent_one_is_not() {
        let unan = |body: &str| {
            report_unanalyzed(&format!(r#"{{"package":"p","functions":[],{body}}}"#))
        };
        // ABSENT — the documented default applies. This is what every complete report this engine writes
        // looks like, so an over-strict reader would refuse the whole corpus.
        assert_eq!(unan(r#""x":1"#), KeyRead::Absent);
        // PRESENT and well-formed, both arms.
        assert_eq!(unan(r#""unanalyzed":[]"#), KeyRead::Present(vec![]));
        assert!(matches!(unan(r#""unanalyzed":[{"path":"a.rs","reason":"why"}]"#), KeyRead::Present(v) if v.len() == 1));
        // PRESENT and UNPARSEABLE — every one of these coerced to `[]` before.
        for body in [
            r#""unanalyzed":[{"unit":"a.rs","why":"parse error"}]"#, // right shape, wrong field names
            r#""unanalyzed":["src/broken.rs"]"#,                     // a bare string list
            r#""unanalyzed":"src/broken.rs""#,                       // a bare string
            r#""unanalyzed":{"path":"a.rs","reason":"w"}"#,          // one object, not a list
            r#""unanalyzed":null"#,
            r#""unanalyzed":3"#,
        ] {
            assert_eq!(unan(body), KeyRead::Corrupt, "must not coerce to the empty list: {body}");
        }

        let an = |body: &str| report_analyzed(&format!(r#"{{"package":"p","functions":[]{body}}}"#));
        assert_eq!(an(""), KeyRead::Absent, "a pre-⟨0.21⟩ producer omits the manifest");
        assert_eq!(an(r#","analyzed":{"count":5,"digest":"ab"}"#),
                   KeyRead::Present(Analyzed { count: 5, digest: "ab".into() }));
        // A digest-less manifest is READABLE — `count` is the load-bearing datum, and refusing here would
        // mint a refusal §2 does not ask for over a claim that is perfectly legible.
        assert_eq!(an(r#","analyzed":{"count":5}"#),
                   KeyRead::Present(Analyzed { count: 5, digest: String::new() }));
        for body in [
            r#","analyzed":{"count":true}"#,  // SPEC §2's live swift row: a boolean is NOT an integer
            r#","analyzed":{"count":"5"}"#,
            r#","analyzed":{"count":-1}"#,
            r#","analyzed":{"digest":"ab"}"#, // no count at all — nothing to read
            r#","analyzed":"lots""#,
            r#","analyzed":null"#,
        ] {
            assert_eq!(an(body), KeyRead::Corrupt, "must not coerce to `count: 0`: {body}");
        }

        // `coverage` — the same rule one rung less sharp: it never moves `ok`, but it rides the verdict,
        // so a silently-dropped ledger DELETES a hedge a machine reads.
        let cv = |body: &str| report_coverage_strict(&format!(r#"{{"package":"p","functions":[]{body}}}"#));
        assert_eq!(cv(""), KeyRead::Absent, "a fully-covered scan omits the key");
        assert!(matches!(cv(r#","coverage":{"uncovered":[{"name":"dep","calls":7}]}"#),
                         KeyRead::Present(c) if c.uncovered[0].calls == 7));
        // A ledger ENTRY missing its decorative `calls` still NAMES an uncovered package, and the name is
        // the whole point — refusing it would drop the hedge in order to be strict about a decoration.
        assert!(matches!(cv(r#","coverage":{"uncovered":[{"name":"dep","why":"not-scanned"}],"covered":[]}"#),
                         KeyRead::Present(c) if c.uncovered[0].name == "dep" && c.uncovered[0].calls == 0),
                "a named-but-uncounted uncovered package must still reach the verdict");
        for body in [
            r#","coverage":"none""#,
            r#","coverage":{"uncovered":[3]}"#,     // entries that are not objects
            r#","coverage":{"uncovered":"dep"}"#,
            r#","coverage":{"uncovered":[{"calls":7}]}"#, // no NAME — nothing to disclose
        ] {
            assert_eq!(cv(body), KeyRead::Corrupt, "must not coerce to an empty ledger: {body}");
        }
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

    /// ⟨0.26⟩ The §2 rule's DESERIALIZATION half: "absent" must survive into the consumer's own model.
    ///
    /// The trio is the §5 reconciliation output — present means that pass ran, absent means it did not,
    /// and `[]` from an engine that computed nothing is a claim ("no function performs an undeclared
    /// effect"). `#[serde(default)]` over a `Vec` destroyed that on the way IN: an absent key became
    /// `vec![]`, indistinguishable from an explicit empty answer, so a producer's careful omission turned
    /// back into the same claim with extra steps. Both directions are asserted because the rule has two
    /// halves and only one of them is about writing.
    #[test]
    fn the_reconciliation_trio_distinguishes_absent_from_empty() {
        // ABSENT in → None, NOT Some(vec![]). This is the assertion the old `Vec` type could not make.
        let absent: ReportEntry = serde_json::from_str(r#"{"fn":"f","inferred":["Fs"]}"#).unwrap();
        assert_eq!(absent.undeclared, None, "an ABSENT key means the §5 pass did not run");
        assert_eq!(absent.declared, None);
        assert_eq!(absent.overdeclared, None);

        // An EXPLICIT empty array is a different input and must stay distinguishable — it is what an
        // engine that DID run the pass and found nothing would write.
        let empty: ReportEntry =
            serde_json::from_str(r#"{"fn":"f","inferred":["Fs"],"undeclared":[]}"#).unwrap();
        assert_eq!(empty.undeclared, Some(vec![]), "an explicit [] is an ANSWER, not an absence");
        assert_ne!(empty.undeclared, absent.undeclared,
                   "absent and empty must not collapse — that collapse IS the defect");

        // And out: None is omitted (this engine's scanner runs no §5 pass), Some is written.
        let none_out = serde_json::to_string(&ReportEntry {
            func: "f".into(), ..Default::default() }).unwrap();
        assert!(!none_out.contains("undeclared"), "None must be OMITTED, never written as []: {none_out}");
        let some_out = serde_json::to_string(&ReportEntry {
            func: "f".into(), undeclared: Some(vec!["Fs".into()]), ..Default::default() }).unwrap();
        assert!(some_out.contains("\"undeclared\":[\"Fs\"]"), "Some must be written: {some_out}");
    }

    #[test]
    fn round_trips() {
        let r = Report {
            candor: ReportMeta { version: "v".into(), toolchain: "t".into(), spec: SPEC_VERSION.into() },
            coverage: None,
            unanalyzed: vec![],
            type_surface: None,
            resolves: vec![],
            excluded: vec![],
            out_of_scope: None,
            scanned_under: None,
            net_partners: None,
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
        assert!(s.contains("\"spec\":\"0.32\""), "envelope must carry the spec version: {s}");
        // A LITERAL on purpose, unlike the two cli.rs assertions above: this canary exists to
        // notice that the constant changed, so deriving it from the constant makes it vacuous.
        assert_eq!(SPEC_VERSION, "0.32");
    }

    /// THE MODEL-LEVEL HALF of the §4 ⟨0.24⟩ forward-compatibility control: the report type must carry an
    /// `unknownWhy` kind it has never heard of, VERBATIM, in both directions. `unknown_why` is a
    /// `Vec<String>` on purpose — nothing here parses a kind, so a fabricated one cannot be normalised,
    /// truncated or dropped on the way through. The classifier half (`banana:whatever` → the conservative
    /// catch-all) is `off_vocabulary_kinds_round_trip_and_classify_through_the_catch_all` in
    /// candor-classify; the end-to-end half is in candor-query's CLI tests.
    #[test]
    fn an_off_vocabulary_unknown_why_kind_round_trips_verbatim() {
        // Two shapes a validating writer would be tempted to touch: a kind outside §4's five, and a
        // `dispatch:` detail carrying colons and spaces rather than the normative `owner.member`.
        let json = r#"[{"fn":"f","inferred":["Unknown"],
                        "unknownWhy":["banana:whatever","ambiguous:same-name local defs"]}]"#;
        let back = report_entries(json).unwrap();
        assert_eq!(back[0].unknown_why,
                   vec!["banana:whatever".to_string(), "ambiguous:same-name local defs".to_string()],
                   "an unrecognised kind must survive READ unchanged and unreordered");
        let s = serde_json::to_string(&back[0]).unwrap();
        assert!(s.contains("\"unknownWhy\":[\"banana:whatever\",\"ambiguous:same-name local defs\"]"),
                "…and survive WRITE unchanged: {s}");
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
        let old = to_packaged_report_json_full(&meta, "p", &e, None, &[], None, &[], None, None, None).unwrap();
        let empty = to_packaged_report_json_typed(&meta, "p", &e, None, &[], None,
                                                  Some(&TypeSurface::default()), &[], None, None, None).unwrap();
        assert_eq!(old, empty, "an empty type surface must not change one byte of the report");
        assert!(report_type_surface(&old).is_none(), "absence must parse as nothing, never an error");
        let mut returns = std::collections::BTreeMap::new();
        returns.insert("dep#sync::build".to_string(), "dep#sync::Client".to_string());
        let full = to_packaged_report_json_typed(&meta, "p", &e, None, &[], None,
                                                 Some(&TypeSurface { returns }), &[], None, None, None).unwrap();
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
    /// `<base>.<crate>.<type>.json` (exactly two segments after the base) whose `<type>` is not a
    /// reserved [`SIDECAR_KINDS`] name; sidecars (one segment, or a reserved trailing segment) and
    /// any 3+-segment name are NOT reports. Real reports always have a dot-free crate name and type,
    /// so two segments is exact. Sorted by path, with krate/kind parsed from the filename.
    ///
    /// The `<pkg>.hierarchy` / `<pkg>.callgraph` rows are the 2-segment sidecar shape SPEC §2.2
    /// licenses (each engine pairs a sidecar to its OWN report stem, so the count is not fixed). They
    /// landed exactly on `<crate>.<type>`, entered the candidate set, and every query printed a FALSE
    /// "report … failed to parse — its functions are OMITTED … re-run the scan" over a perfectly good
    /// scan. `r.hierarchy.lib.json` is the control that keeps the exclusion a DENYLIST over `<type>`
    /// and not a ban on the WORD: a crate legitimately called `hierarchy` must still be found.
    #[test]
    fn report_files_discriminates_and_parses() {
        let dir = std::env::temp_dir().join("candor-report-files-test");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        for f in [
            "r.mycrate.lib.json",          // report ✓ → (mycrate, lib)
            "r.mycrate.Executable.json",   // report ✓ → (mycrate, Executable)
            "r.hierarchy.lib.json",        // report ✓ → a CRATE named `hierarchy` is not a sidecar
            "r.calibrated.json",           // sidecar ✗ (one segment)
            "r.encountered-mycrate.json",  // sidecar ✗ (one segment)
            "r.a.b.c.json",                // ✗ 3 segments (kind "b.c" has a dot)
            "other.x.y.json",              // ✗ different base
        ] {
            std::fs::write(dir.join(f), "[]").unwrap();
        }
        // …and every reserved sidecar kind in the `<type>` position (`<base>.<pkg>.<kind>.json`).
        for k in SIDECAR_KINDS {
            std::fs::write(dir.join(format!("r.app.{k}.json")), "{}").unwrap();
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
                ("r.hierarchy.lib.json".into(), "hierarchy".into(), "lib".into()),
                ("r.mycrate.Executable.json".into(), "mycrate".into(), "Executable".into()),
                ("r.mycrate.lib.json".into(), "mycrate".into(), "lib".into()),
            ]
        );
        let _ = std::fs::remove_dir_all(&dir);
    }
}
