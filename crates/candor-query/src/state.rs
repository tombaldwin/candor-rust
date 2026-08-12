//! Workspace/report state: `state`, `reports`, `locate`, `engine-version`,
//! `merge-hook`.

use crate::*;

/// `state [<root>]` — print a stable content hash of every `.rs` file under `<root>` (default cwd),
/// excluding `target/` and `.git/`. This is the source-freshness key the wrapper writes to
/// `.candor/state` and later compares, to tell whether a saved report still matches the tree. It
/// replaces a fragile `find … | sort -z | xargs shasum | shasum | cut` pipeline that was copy-pasted
/// into ~10 shell sites — and had already DRIFTED (some copies excluded `.git`, some didn't), so two
/// code paths could hash the same tree differently. One canonical implementation kills that bug class.
/// The hash is FNV-1a over each file's path then bytes (NUL-separated) in sorted order — deterministic
/// and dependency-free; the value need only be stable, not cryptographic (the state file is ephemeral).
pub(crate) fn cmd_state(args: &[String]) -> i32 {
    let root = args.first().map(String::as_str).unwrap_or(".");
    let mut files: Vec<PathBuf> = Vec::new();
    collect_rs(Path::new(root), &mut files);
    files.sort();
    let mut h: u64 = 0xcbf2_9ce4_8422_2325; // FNV-1a 64-bit offset basis
    let mut feed = |bytes: &[u8]| {
        for &b in bytes {
            h ^= b as u64;
            h = h.wrapping_mul(0x0000_0100_0000_01b3);
        }
    };
    for f in &files {
        // Path relative to root, so the same tree hashes the same regardless of where it lives.
        let rel = f.strip_prefix(root).unwrap_or(f);
        feed(rel.to_string_lossy().as_bytes());
        feed(&[0]);
        if let Ok(bytes) = std::fs::read(f) {
            feed(&bytes);
        }
        feed(&[0]);
    }
    println!("{h:016x}");
    0
}

/// Remove every report artifact for <prefix> that does NOT belong to the `keep` backend — so a lint
/// report and a scan report never coexist under one prefix. Removes reports + callgraph/encountered/
/// calibrated sidecars of the other backend; never touches the kept backend's files (so a build failure
/// keeps the same-backend last-good report). Returns the count removed.
pub(crate) fn clear_other_reports(prefix: &str, keep: &str) -> usize {
    let p = Path::new(prefix);
    let dir = p.parent().filter(|d| !d.as_os_str().is_empty()).map(Path::to_path_buf).unwrap_or_else(|| PathBuf::from("."));
    let Some(base) = p.file_name().and_then(|s| s.to_str()) else { return 0 };
    let prefix_dot = format!("{base}.");
    let Ok(rd) = std::fs::read_dir(&dir) else { return 0 };
    let mut removed = 0;
    for ent in rd.flatten() {
        let name = ent.file_name();
        let Some(name) = name.to_str() else { continue };
        if !name.starts_with(&prefix_dot) || !name.ends_with(".json") {
            continue;
        }
        let scan = is_scan_artifact(base, name);
        let remove = match keep {
            "scan" => !scan, // keep the scan report; drop the lint reports + its sidecars
            "lint" => scan,  // keep the lint report(s); drop the scan report + its callgraph
            _ => false,
        };
        if remove && std::fs::remove_file(ent.path()).is_ok() {
            removed += 1;
        }
    }
    removed
}

pub(crate) fn cmd_reports(args: &[String]) -> i32 {
    let exists_only = args.iter().any(|a| a == "--exists");
    let backend = args.iter().any(|a| a == "--backend");
    // SPEC §3.2 ⟨0.28⟩: `--clear-other` takes a value, so "given no value" — nothing follows, or the
    // next token is flag-shaped — is a usage error at exit 2. It used to read the next token whatever
    // its shape (or None), and every wrong reading was a SILENT exit-0 no-op: `--clear-other --exists`
    // "cleared" with keep=`--exists` (removing nothing), and a trailing `--clear-other` fell through to
    // the LIST mode as if the flag had not been typed. `-` is refused too — the value is an enum, and a
    // dash names nothing it could keep.
    let clear_keep = match args.iter().position(|a| a == "--clear-other") {
        Some(i) => match args.get(i + 1) {
            Some(v) if !v.starts_with('-') => Some(v.clone()),
            Some(v) => {
                eprintln!("candor-query: --clear-other was given no value — the next token `{v}` is a flag, not <scan|lint>");
                return 2;
            }
            None => {
                eprintln!("candor-query: --clear-other requires <scan|lint>");
                return 2;
            }
        },
        None => None,
    };
    let Some(prefix) = args.iter().find(|a| !a.starts_with("--")) else {
        eprintln!("usage: candor-query reports <prefix> [--exists | --backend | --clear-other <scan|lint>]");
        return 2;
    };
    if backend {
        println!("{}", report_backend(prefix));
        return 0;
    }
    if let Some(keep) = clear_keep {
        clear_other_reports(prefix, &keep);
        return 0;
    }
    let files = report_files(prefix);
    if exists_only {
        return if files.is_empty() { 1 } else { 0 };
    }
    for rf in &files {
        println!("{}", rf.path.display());
    }
    0
}

/// `candor-query locate <lib|scan> <dir>...` — print the NEWEST-by-mtime matching artifact across the
/// given dirs (a dylib for `lib`, the `candor-scan` binary for `scan`), or nothing. The single owner of
/// the newest-mtime locator logic (was copied into both bash scripts; `ls | head` there silently picked
/// the alphabetically-first — stale — toolchain dylib after a bump). `query` itself stays a bash
/// bootstrap (it can't locate the binary that does the locating).
/// Newest-by-mtime artifact of `kind` (`lib` = a `libcandor@*.{dylib,so}`, `scan` = the `candor-scan`
/// binary) across `dirs`, or None. Newest mtime — NOT alphabetical (`ls | head` picks the stale
/// toolchain dylib after a bump). Pure (unit-tested); `cmd_locate` just prints it.
pub(crate) fn locate_newest(kind: &str, dirs: &[String]) -> Option<PathBuf> {
    let matches_kind = |name: &str| -> bool {
        match kind {
            "scan" => name == "candor-scan",
            "lib" => name.starts_with("libcandor@") && (name.ends_with(".dylib") || name.ends_with(".so")),
            _ => false,
        }
    };
    let mut newest: Option<(std::time::SystemTime, PathBuf)> = None;
    for dir in dirs {
        let Ok(rd) = std::fs::read_dir(dir) else { continue };
        for ent in rd.flatten() {
            let name = ent.file_name();
            let Some(name) = name.to_str() else { continue };
            if !matches_kind(name) {
                continue;
            }
            let Ok(mtime) = ent.metadata().and_then(|m| m.modified()) else { continue };
            if newest.as_ref().is_none_or(|(t, _)| mtime > *t) {
                newest = Some((mtime, ent.path()));
            }
        }
    }
    newest.map(|(_, p)| p)
}

pub(crate) fn cmd_locate(args: &[String]) -> i32 {
    let Some(kind) = args.first() else {
        eprintln!("usage: candor-query locate <lib|scan> <dir>...");
        return 2;
    };
    match locate_newest(kind, &args[1..]) {
        Some(p) => {
            println!("{}", p.display());
            0
        }
        None => 1,
    }
}

/// `candor-query engine-version <lib-path>` — print the `candor-build-version=` tag build.rs embedded in
/// the dylib (the version of the engine that actually produced a report), or nothing. Replaces a
/// `strings -a | grep -oE` incantation duplicated across both bash scripts (and not portable without
/// `strings`). Reads the file bytes directly.
pub(crate) fn cmd_engine_version(args: &[String]) -> i32 {
    let Some(path) = args.first() else {
        eprintln!("usage: candor-query engine-version <lib-path>");
        return 2;
    };
    let Ok(bytes) = std::fs::read(path) else { return 1 };
    let needle = b"candor-build-version=";
    let Some(start) = bytes.windows(needle.len()).position(|w| w == needle) else { return 1 };
    let v: Vec<u8> = bytes[start + needle.len()..]
        .iter()
        .copied()
        .take_while(|b| b.is_ascii_alphanumeric())
        .collect();
    if v.is_empty() {
        return 1;
    }
    println!("{}", String::from_utf8_lossy(&v));
    0
}

/// `merge-hook <settings.json> <hook-command>` — idempotently merge candor's Stop hook into a Claude
/// Code settings file, NON-destructively. Replaces an inline `python3` heredoc (one less interpreter
/// dependency, and typed + tested). If the file exists but isn't strict JSON (comments / trailing
/// commas, which Claude Code tolerates but a strict parser rejects), it is LEFT UNTOUCHED and a manual
/// snippet is printed — never reset-and-overwritten, which would wipe the user's other settings.
pub(crate) fn cmd_merge_hook(args: &[String]) -> i32 {
    let (Some(path), Some(cmd)) = (args.first(), args.get(1)) else {
        eprintln!("usage: candor-query merge-hook <settings.json> <hook-command>");
        return 2;
    };
    let manual = || {
        eprintln!("  WARNING: {path} isn't plain JSON (comments or trailing commas?) — NOT modifying it.");
        eprintln!(
            "  Add this Stop hook by hand: {{\"matcher\":\"*\",\"hooks\":[{{\"type\":\"command\",\"command\":\"{cmd}\"}}]}}"
        );
    };
    let mut data: serde_json::Value = if Path::new(path).exists() {
        match std::fs::read_to_string(path) {
            Ok(s) => match serde_json::from_str(&s) {
                Ok(v) => v,
                Err(_) => {
                    manual();
                    return 0; // unparseable → leave it; do not risk the user's settings
                }
            },
            Err(e) => {
                eprintln!("candor-query: cannot read {path}: {e}");
                return 1;
            }
        }
    } else {
        serde_json::json!({})
    };
    // Navigate/insert hooks.Stop, bailing (untouched) if any node is the wrong JSON type.
    let Some(obj) = data.as_object_mut() else {
        manual();
        return 0;
    };
    let hooks = obj.entry("hooks").or_insert_with(|| serde_json::json!({}));
    let Some(stop) = hooks
        .as_object_mut()
        .map(|h| h.entry("Stop").or_insert_with(|| serde_json::json!([])))
        .and_then(|s| s.as_array_mut())
    else {
        manual();
        return 0;
    };
    let present = stop.iter().any(|g| {
        g.get("hooks").and_then(|h| h.as_array()).is_some_and(|hs| {
            hs.iter().any(|h| h.get("command").and_then(|c| c.as_str()) == Some(cmd.as_str()))
        })
    });
    if present {
        println!("  Stop hook already present in {path}");
        return 0;
    }
    stop.push(serde_json::json!({"matcher": "*", "hooks": [{"type": "command", "command": cmd}]}));
    let body = serde_json::to_string_pretty(&data).unwrap_or_default();
    if let Err(e) = std::fs::write(path, format!("{body}\n")) {
        eprintln!("candor-query: cannot write {path}: {e}");
        return 1;
    }
    println!("  merged Stop hook into {path}");
    0
}

/// Recursively collect `*.rs` files under `dir`, skipping `target` and `.git` directories (and any
/// symlinked dir, to avoid cycles). Order-independent; the caller sorts.
pub(crate) fn collect_rs(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(rd) = std::fs::read_dir(dir) else { return };
    for ent in rd.flatten() {
        let path = ent.path();
        let Ok(ft) = ent.file_type() else { continue };
        if ft.is_dir() {
            let name = ent.file_name();
            if name == "target" || name == ".git" {
                continue;
            }
            collect_rs(&path, out);
        } else if ft.is_file() && path.extension().is_some_and(|e| e == "rs") {
            out.push(path);
        }
    }
}
