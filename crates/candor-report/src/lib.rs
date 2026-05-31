//! Shared candor report types and parsing — **no `rustc_private`**, so both the lint (which writes
//! reports) and the CLI / tooling (which read them) depend on one definition instead of re-deriving
//! the JSON shape in every script. This is the type-safe, DRY core the bash+Python tooling lacked.

use serde::{Deserialize, Serialize};

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
