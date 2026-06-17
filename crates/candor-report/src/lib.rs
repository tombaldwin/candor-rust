//! Shared candor report types and parsing — **no `rustc_private`**, so both the lint (which writes
//! reports) and the CLI / tooling (which read them) depend on one definition instead of re-deriving
//! the JSON shape in every script. This is the type-safe, DRY core the bash+Python tooling lacked.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// The ten effects candor classifies (excluding the synthetic `Unknown`). Defined once here so the
/// lint and the CLI share one vocabulary instead of each keeping its own copy (which had already
/// drifted in ordering). Order is irrelevant to consumers — both tally by name and sort the output.
pub const EFFECTS: [&str; 10] =
    ["Net", "Db", "Fs", "Exec", "Ipc", "Env", "Clock", "Rand", "Clipboard", "Log"];

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
}

/// The candor-spec contract version this build implements (the report SCHEMA + AS-EFF codes), distinct
/// from the engine build id (`ReportMeta::version`) and from the crate release version. Bumped only when
/// the spec contract changes; emitted as the envelope's `spec` so a consumer can see which contract a
/// report conforms to. Both backends and the JVM port declare the SAME value — see candor-spec §2.1.
pub const SPEC_VERSION: &str = "0.5";

/// The envelope header: which engine produced the report (`version` = build id, `toolchain`), and which
/// candor-spec contract it implements (`spec`).
#[derive(Serialize, Deserialize, Default, Clone, Debug)]
pub struct ReportMeta {
    pub version: String,
    #[serde(default)]
    pub toolchain: String,
    /// candor-spec contract version (e.g. `"0.5"`). `#[serde(default)]` so a legacy report without it
    /// still parses (absent ⇒ pre-spec-field, treat as ≤ 0.2).
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub spec: String,
}

/// The v0.2 self-describing report: a provenance header plus the function entries.
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Report {
    pub candor: ReportMeta,
    pub functions: Vec<ReportEntry>,
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
    #[derive(Serialize)]
    struct Out<'a> {
        candor: &'a ReportMeta,
        #[serde(skip_serializing_if = "str::is_empty")]
        package: &'a str,
        functions: &'a [ReportEntry],
    }
    serde_json::to_string_pretty(&Out { candor, package, functions })
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
        assert!(s.contains("\"spec\":\"0.5\""), "envelope must carry the spec version: {s}");
        assert_eq!(SPEC_VERSION, "0.5");
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
