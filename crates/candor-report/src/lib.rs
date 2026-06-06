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
        if kind.contains('.') {
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
    /// Stable cross-crate identity (hex `DefPathHash`); empty in older reports.
    #[serde(default)]
    pub hash: String,
    /// Filesystem access detail when the `Fs` effect's verbs revealed it: `["read"]`, `["write"]`, or
    /// both. A non-breaking refinement (the `Fs` effect itself is unchanged); omitted when unknown or
    /// when the function performs no `Fs`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub fs: Vec<String>,
    /// Effectful local functions this one calls — the effect-relevant call graph ("who calls X?").
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub calls: Vec<String>,
}

/// The envelope header (v0.2): which engine produced the report.
#[derive(Serialize, Deserialize, Default, Clone, Debug)]
pub struct ReportMeta {
    pub version: String,
    #[serde(default)]
    pub toolchain: String,
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
    #[derive(Deserialize)]
    struct Envelope {
        functions: Vec<ReportEntry>,
    }
    if let Ok(env) = serde_json::from_str::<Envelope>(text) {
        return Some(env.functions);
    }
    serde_json::from_str::<Vec<ReportEntry>>(text).ok()
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
            candor: ReportMeta { version: "v".into(), toolchain: "t".into() },
            functions: vec![ReportEntry { func: "f".into(), inferred: vec!["Db".into()], ..Default::default() }],
        };
        let s = serde_json::to_string(&r).unwrap();
        let back = report_entries(&s).unwrap();
        assert_eq!(back[0].func, "f");
        // empty `calls` is omitted on write.
        assert!(!s.contains("\"calls\""));
    }
}
