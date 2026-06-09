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
    #[serde(default)]
    pub loc: String,
    #[serde(default)]
    pub inferred: Vec<String>,
    #[serde(default)]
    pub direct: Vec<String>,
    #[serde(default)]
    pub declared: Vec<String>,
    #[serde(default)]
    pub undeclared: Vec<String>,
    #[serde(default)]
    pub overdeclared: Vec<String>,
    #[serde(default)]
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
}

/// The candor-spec contract version this build implements (the report SCHEMA + AS-EFF codes), distinct
/// from the engine build id (`ReportMeta::version`) and from the crate release version. Bumped only when
/// the spec contract changes; emitted as the envelope's `spec` so a consumer can see which contract a
/// report conforms to. Both backends and the JVM port declare the SAME value — see candor-spec §2.1.
pub const SPEC_VERSION: &str = "0.3";

/// The envelope header: which engine produced the report (`version` = build id, `toolchain`), and which
/// candor-spec contract it implements (`spec`).
#[derive(Serialize, Deserialize, Default, Clone, Debug)]
pub struct ReportMeta {
    pub version: String,
    #[serde(default)]
    pub toolchain: String,
    /// candor-spec contract version (e.g. `"0.3"`). `#[serde(default)]` so a legacy report without it
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
    // Accept the v0.2 envelope `{candor, functions:[...]}` or a legacy bare array. Deserialize each
    // entry INDEPENDENTLY (via raw `Value`s), skipping any that fail — so one malformed entry (a
    // partial write, a hand-edit, an entry missing `fn`) loses only ITSELF, not the whole crate's
    // report. The old all-or-nothing `Vec<ReportEntry>` deser dropped every function in the file on a
    // single bad entry — a silent under-report of the entire crate.
    let val: serde_json::Value = serde_json::from_str(text).ok()?;
    let arr = val
        .get("functions")
        .and_then(|f| f.as_array())
        .or_else(|| val.as_array())?;
    Some(
        arr.iter()
            .filter_map(|e| serde_json::from_value::<ReportEntry>(e.clone()).ok())
            .collect(),
    )
}

/// Serialize a v0.2 report from a header + entries, borrowing both so the caller keeps ownership
/// (the lint logs the entry count after writing). Pretty-printed.
pub fn to_report_json(candor: &ReportMeta, functions: &[ReportEntry]) -> serde_json::Result<String> {
    #[derive(Serialize)]
    struct Out<'a> {
        candor: &'a ReportMeta,
        functions: &'a [ReportEntry],
    }
    serde_json::to_string_pretty(&Out { candor, functions })
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
    fn round_trips() {
        let r = Report {
            candor: ReportMeta { version: "v".into(), toolchain: "t".into(), spec: "0.3".into() },
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
        assert!(s.contains("\"spec\":\"0.3\""), "envelope must carry the spec version: {s}");
        assert_eq!(SPEC_VERSION, "0.3");
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
