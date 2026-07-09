//! Everything Cargo/dependency-shaped: Cargo.toml reading (line-based on purpose),
//! dependency-report chaining (`--deps`), and the registry-source locator.

use crate::*;

/// candor-SCAN ONLY: builder-ENTRY points whose effect the typed classifier deliberately defers to a
/// terminal VERB. `duct::cmd!(...).run()` is the canonical case — `cmd!`/`cmd` only BUILD an Expression;
/// the spawn is at `.run()`/`.read()`/`.start()`. The DEEP engine types the receiver and catches the verb,
/// so candor-classify keeps the entry pure for PRECISION (lib.rs duct rule + its `cmd → None` test). But
/// the SYNTACTIC scanner can't type a builder chain — least of all through the `cmd!` MACRO whose result
/// is opaque — so the verb's effect is dropped and the program reads silent-pure (a real under-report
/// found by the real-world dynamic oracle; the same macro-blindness family as the log/tracing macros).
/// Classify the ENTRY as the crate's whole effect: a safe OVER-approximation (candor's never-under-report
/// bias), scoped to candor-scan so the deep engine stays precise. Both engines still agree on the
/// function's effect when the builder is actually run (the overwhelmingly common case).
pub(crate) fn scan_builder_entry_effect(_cr: &str, path: &str) -> Option<&'static str> {
    // A DATA TABLE the real-world oracle DRIVES: builder-chain ENTRY paths whose effect candor-classify
    // keys on a TERMINAL VERB the syntactic scanner can't reach (it can't type the chain). Add a row when
    // the oracle proves a verb-keyed crate under-reports here. Entries are exact ENTRY paths — NOT the
    // terminal verbs (those stay candor-classify's job for the typed deep engine, which stays precise).
    const ENTRIES: &[(&str, &str)] = &[
        // duct — `cmd!`/`sh!`/`cmd`/`sh` build; `.run()/.read()/.start()` execute (found 2026-06-17).
        ("duct::cmd", "Exec"),
        ("duct::sh", "Exec"),
        // ureq — `get/post/...` build a Request; `.call()` performs the Net (found 2026-06-17, net_ureq).
        ("ureq::get", "Net"),
        ("ureq::post", "Net"),
        ("ureq::put", "Net"),
        ("ureq::delete", "Net"),
        ("ureq::head", "Net"),
        ("ureq::patch", "Net"),
        ("ureq::request", "Net"),
        // sqlx — `query*()` build; `.execute()/.fetch_*()` round-trip (found 2026-06-17, recall corpus).
        ("sqlx::query", "Db"),
        ("sqlx::query_as", "Db"),
        ("sqlx::query_scalar", "Db"),
        ("sqlx::query_with", "Db"),
        ("sqlx::query_as_with", "Db"),
        // diesel — `sql_query()` builds raw SQL; `.execute()/.load()` round-trips (found 2026-06-17).
        ("diesel::sql_query", "Db"),
    ];
    ENTRIES.iter().find(|(p, _)| *p == path).map(|(_, eff)| *eff)
}

/// A loaded sibling-report function: the effects + literal surfaces a consumer's call inherits.
#[derive(Clone, Default)]
pub(crate) struct DepFn {
    pub(crate) effects: Vec<&'static str>,
    pub(crate) hosts: Vec<String>,
    pub(crate) cmds: Vec<String>,
    pub(crate) paths: Vec<String>,
    pub(crate) tables: Vec<String>,
    /// Blind crates the dep fn (transitively) reaches — its report's `invisible`. Carried across the join
    /// so a consumer inherits the disclosure (sweep [8]): else a dep that floored an unmodeled crate read
    /// as plain pure at the chain boundary, dropping the per-fn honesty caveat.
    pub(crate) invisible: Vec<String>,
    /// Effects whose surface the dep fn left masking-incomplete — carried so a benign literal in the
    /// consumer can't mask the dep's invisible forbidden endpoint across the join (sweep [30]).
    pub(crate) incomplete: Vec<&'static str>,
}

/// The CANDOR_DEPS index: `crate#tail2` and `crate#leaf` keys (UNAMBIGUOUS only — a key two dep
/// functions share is dropped, the same under-report-don't-guess rule as `resolve_target`), plus
/// the covered crate set. A report whose producing version differs from this binary's is
/// DOWNGRADED to `Unknown` rather than silently trusted (spec §2.1).
#[derive(Default)]
pub(crate) struct DepIndex {
    pub(crate) by_key: HashMap<String, DepFn>,
    pub(crate) crates: std::collections::HashSet<String>,
}

pub(crate) fn load_dep_reports(spec: Option<&str>) -> DepIndex {
    let mut idx = DepIndex::default();
    let Some(spec) = spec else { return idx };
    // Canonical-path dedup: the same report loaded twice would self-collide on every key and be
    // dropped as 'ambiguous', silently killing the chain (review: --deps + CANDOR_DEPS=.candor/deps
    // — the natural combination — did exactly that). Directories walk RECURSIVELY: --deps writes
    // one subdirectory per name@version.
    let mut files: Vec<std::path::PathBuf> = Vec::new();
    let mut seen_files: std::collections::HashSet<std::path::PathBuf> = std::collections::HashSet::new();
    let mut push_file = |f: std::path::PathBuf, files: &mut Vec<std::path::PathBuf>| {
        let canon = std::fs::canonicalize(&f).unwrap_or(f);
        if seen_files.insert(canon.clone()) {
            files.push(canon);
        }
    };
    for tok in spec.split(':').filter(|t| !t.is_empty()) {
        let p = Path::new(tok);
        if p.is_dir() {
            for e in walkdir::WalkDir::new(p).into_iter().filter_map(Result::ok) {
                let f = e.path();
                let name = f.file_name().and_then(|n| n.to_str()).unwrap_or("");
                if f.is_file() && name.ends_with(".json") && !name.contains("callgraph") {
                    push_file(f.to_path_buf(), &mut files);
                }
            }
        } else if p.is_file() {
            push_file(p.to_path_buf(), &mut files);
        } else {
            eprintln!("candor-scan: CANDOR_DEPS entry not found, skipped: {tok}");
        }
    }
    let my_version = format!("scan-{}", env!("CARGO_PKG_VERSION"));
    let mut ambiguous: std::collections::HashSet<String> = std::collections::HashSet::new();
    for f in &files {
        let Ok(text) = std::fs::read_to_string(f) else {
            eprintln!("candor-scan: CANDOR_DEPS report unreadable, skipped: {}", f.display());
            continue;
        };
        let Ok(v) = serde_json::from_str::<serde_json::Value>(&text) else {
            eprintln!("candor-scan: CANDOR_DEPS report unparsable, skipped: {}", f.display());
            continue;
        };
        // v0.2+ envelope or the v0.1 bare array; the producing version comes from the envelope.
        let version = v.pointer("/candor/version").and_then(|x| x.as_str()).unwrap_or("");
        let stale = version != my_version;
        let Some(fns) = v.get("functions").and_then(|x| x.as_array()).or_else(|| v.as_array()) else { continue };
        // The crate(s) a report COVERS, for the §7.14 ledger exemption (§2 chaining rule 3): the
        // AUTHORITATIVE claim is the envelope's `package` (or the JVM-shape `packages`) field — an
        // EMPTY report ({functions: []}) is an all-pure purity claim for that package, covered and
        // never a κ blind spot. Keyed on the envelope so the exemption doesn't depend on the file
        // NAME or on any join firing (found live: an empty chained report named outside the
        // `….<crate>.scan.json` shape still drew a "κ doesn't know" line here while candor-java and
        // candor-ts correctly stayed quiet). A hyphenated package name also registers in Rust ident
        // form (`dep-c` → `dep_c`), the form call paths carry.
        for pkg in v
            .get("package")
            .and_then(|x| x.as_str())
            .into_iter()
            .chain(v.get("packages").and_then(|x| x.as_array()).into_iter().flatten().filter_map(|x| x.as_str()))
        {
            idx.crates.insert(pkg.to_string());
            idx.crates.insert(pkg.replace('-', "_"));
        }
        // Filename fallback (`report.<crate>.scan.json`) for pre-`package` reports, and the default
        // crate attribution for entries carrying no `hash` prefix.
        let file_crate = f
            .file_name()
            .and_then(|n| n.to_str())
            .and_then(|n| n.strip_suffix(".scan.json"))
            .and_then(|n| n.rsplit('.').next())
            .map(str::to_string);
        // Register the crate at FILE level too: an all-pure crate's report has zero entries, and that
        // emptiness is its honest claim — the crate is covered, not invisible.
        if let Some(c) = &file_crate {
            idx.crates.insert(c.clone());
        }
        for e in fns {
            let Some(qual) = e.get("fn").and_then(|x| x.as_str()) else { continue };
            let krate = e
                .get("hash")
                .and_then(|x| x.as_str())
                .and_then(|h| h.split_once('#'))
                .map(|(c, _)| c.to_string())
                .or_else(|| file_crate.clone());
            let Some(krate) = krate else { continue };
            idx.crates.insert(krate.clone());
            let mut de = DepFn::default();
            if stale {
                de.effects.push("Unknown"); // §2.1: a different producer version is not trusted
            } else {
                for s in e.get("inferred").and_then(|x| x.as_array()).into_iter().flatten() {
                    if let Some(s) = s.as_str() {
                        // unknown vocabulary (a future spec's effect) is honestly Unknown
                        de.effects.push(candor_classify::cap_from_name(s).unwrap_or("Unknown"));
                    }
                }
                let strs = |k: &str| -> Vec<String> {
                    e.get(k)
                        .and_then(|x| x.as_array())
                        .into_iter()
                        .flatten()
                        .filter_map(|s| s.as_str().map(str::to_string))
                        .collect()
                };
                de.hosts = strs("hosts");
                de.cmds = strs("cmds");
                de.paths = strs("paths");
                de.tables = strs("tables");
                de.invisible = strs("invisible"); // sweep [8]: carry the blind-crate disclosure across the join
                // sweep [30]: carry masking-incompleteness (mapped to the static effect alphabet).
                for s in e.get("incomplete").and_then(|x| x.as_array()).into_iter().flatten() {
                    if let Some(eff) = s.as_str().and_then(candor_classify::cap_from_name) {
                        de.incomplete.push(eff);
                    }
                }
            }
            let mut keys = vec![format!("{krate}#{}", qual.rsplit("::").next().unwrap_or(qual))];
            if let Some(t2) = tail2(qual) {
                keys.push(format!("{krate}#{t2}"));
            }
            for k in keys {
                if ambiguous.contains(&k) {
                    continue;
                }
                // Not the `entry` API: a collision REMOVES the key (and remembers it as ambiguous),
                // so the present-vs-absent branches move `k` into different maps — clippy's map_entry
                // rewrite (insert-or-modify in place) can't express the remove-on-collision.
                #[allow(clippy::map_entry)]
                if idx.by_key.contains_key(&k) {
                    idx.by_key.remove(&k); // two dep fns share the key — drop it, never guess
                    ambiguous.insert(k);
                } else {
                    idx.by_key.insert(k, de.clone());
                }
            }
        }
    }
    idx
}

// ── shared Cargo.toml line primitives (line-based on purpose — no toml dependency) ────────────────
// The ONE place table-header and scalar parsing live, so a manifest-syntax quirk (`[ spaced ]`
// headers, a trailing `# comment`) is handled once across the three readers below rather than
// drifting between them.

/// A `[section]` header line → its inner name, surrounding spaces tolerated (`[ workspace ]` →
/// "workspace"); None for any non-header line.
pub(crate) fn toml_section(line: &str) -> Option<&str> {
    let l = line.trim();
    Some(l.strip_prefix('[')?.strip_suffix(']')?.trim())
}

/// A scalar `key = "value"` / `key = value` on this line — `key` matched as the WHOLE key (then `=`),
/// the value quote-trimmed and an out-of-quotes trailing `# comment` stripped. None if not this key.
pub(crate) fn toml_scalar<'a>(line: &'a str, key: &str) -> Option<&'a str> {
    let rest = line.trim().strip_prefix(key)?.trim_start().strip_prefix('=')?.trim();
    Some(if let Some(q) = rest.strip_prefix('"') {
        q.split('"').next().unwrap_or(q)
    } else {
        rest.split('#').next().unwrap_or(rest).trim()
    })
}

/// Dependency names declared by EVERY Cargo.toml under the scan root (a workspace's members each
/// declare their own — review: reading only the root manifest left member-declared deps invisible
/// to the κ ledger on the most common project layout), normalized to crate-root form (`-` -> `_`).
/// dev-/build-dependencies are the harness's and the build script's universe, not the crate's
/// runtime one — excluded, like tests/ and build.rs.
pub(crate) fn cargo_deps(dir: &str) -> (std::collections::HashSet<String>, HashMap<String, String>) {
    let mut out = std::collections::HashSet::new();
    let mut renames = HashMap::new();
    // Honour the SAME nested-package rule as the source walk (filter_entry above): a subdir with its
    // own Cargo.toml is a different package whose deps are ITS universe, not this crate's — scan_target
    // scans it separately. Without this, a nested fixture/path-dep's deps polluted the parent's κ
    // ledger (the source walk skips the nested sources, so the two had drifted out of agreement).
    for entry in walkdir::WalkDir::new(dir)
        .into_iter()
        .filter_entry(|e| {
            if e.depth() == 0 || !e.file_type().is_dir() {
                return true;
            }
            let name = e.file_name().to_str().unwrap_or("");
            if name == "target" || (name.starts_with('.') && name != "." && name != "..") {
                return false;
            }
            !e.path().join("Cargo.toml").is_file()
        })
        .filter_map(Result::ok)
    {
        let p = entry.path();
        if p.file_name().and_then(|n| n.to_str()) != Some("Cargo.toml") {
            continue;
        }
        if let Ok(text) = std::fs::read_to_string(p) {
            cargo_toml_deps(&text, &mut out, &mut renames);
        }
    }
    (out, renames)
}

/// One manifest's dependency names, all four header forms: `[dependencies]` /
/// `[workspace.dependencies]` / `[target.….dependencies]` sections, and the table-header
/// declarations `[dependencies.name]` / `[target.….dependencies.name]` (review: the old
/// `ends_with("dependencies]")` gate made the header-form branch unreachable — a table-header
/// dep was invisible to the ledger, execution-verified).
pub(crate) fn cargo_toml_deps(
    text: &str,
    out: &mut std::collections::HashSet<String>,
    renames: &mut HashMap<String, String>,
) {
    // A dependency RENAME (`tui-common = { package = "tb-tui-common" }`) means the manifest KEY is
    // what the code imports while the registry/report knows the REAL package — without the map,
    // --deps scanned the real crate and the join/ledger missed it under the key (found live on
    // ebman: tui_common stayed "invisible" with its report sitting right there).
    // Match `package` only as a KEY (`{ … package = "real" }`), not as a substring of a dependency
    // KEY (`my-package = "1.2"` previously parsed its own version as a rename target) or a value:
    // `package` must sit at a token boundary and be followed by `=`.
    let pkg_re = |l: &str| -> Option<String> {
        let bytes = l.as_bytes();
        let mut search = 0;
        while let Some(rel) = l[search..].find("package") {
            let i = search + rel;
            let boundary = i == 0 || matches!(bytes[i - 1], b'{' | b',' | b' ' | b'\t');
            if boundary {
                if let Some(rest) = l[i + "package".len()..].trim_start().strip_prefix('=') {
                    if let Some(rest) = rest.trim_start().strip_prefix('"') {
                        return rest.split('"').next().map(|s| s.replace('-', "_"));
                    }
                }
            }
            search = i + "package".len();
        }
        None
    };
    let mut in_deps = false;
    let mut header_key: Option<String> = None; // the `[dependencies.name]` we're inside, if any
    for line in text.lines() {
        let l = line.trim();
        if let Some(inner) = toml_section(line) {
            let harness = inner.contains("dev-dependencies") || inner.contains("build-dependencies");
            in_deps = !harness && (inner == "dependencies" || inner.ends_with(".dependencies"));
            header_key = None;
            if !harness && !in_deps {
                let name = inner
                    .rfind(".dependencies.")
                    .map(|i| &inner[i + ".dependencies.".len()..])
                    .or_else(|| inner.strip_prefix("dependencies."));
                if let Some(name) = name {
                    if !name.is_empty() && !name.contains('.') {
                        let key = name.trim_matches('"').replace('-', "_");
                        out.insert(key.clone());
                        header_key = Some(key);
                    }
                }
            }
            continue;
        }
        if l.is_empty() || l.starts_with('#') {
            continue;
        }
        // inside a `[dependencies.name]` table: a `package = "real"` line is the rename
        if let Some(key) = &header_key {
            if l.starts_with("package") {
                if let Some(real) = pkg_re(l) {
                    renames.insert(key.clone(), real);
                }
            }
            continue;
        }
        if !in_deps {
            continue;
        }
        if let Some(name) = l.split('=').next() {
            let name = name.trim().trim_matches('"');
            if !name.is_empty() {
                let key = name.replace('-', "_");
                // A rename only appears in an INLINE TABLE value (`key = { … package = "real" }`),
                // never as a bare `package = "0.1"` (which is a dependency NAMED package) — so search
                // only inside the braces.
                if let Some(brace) = l.find('{') {
                    if let Some(real) = pkg_re(&l[brace..]) {
                        if real != key {
                            renames.insert(key.clone(), real);
                        }
                    }
                }
                out.insert(key);
            }
        }
    }
}

/// The cargo registry source roots (`~/.cargo/registry/src/<index-hash>/`), where unbuilt
/// dependency sources live. CARGO_HOME is honoured.
pub(crate) fn dirs_cargo_registry_src() -> Vec<std::path::PathBuf> {
    let home = std::env::var("CARGO_HOME")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|_| Path::new(&std::env::var("HOME").unwrap_or_default()).join(".cargo"));
    std::fs::read_dir(home.join("registry").join("src"))
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.is_dir())
        .collect()
}

pub(crate) fn read_crate_name(root: &Path) -> Option<String> {
    let txt = std::fs::read_to_string(root.join("Cargo.toml")).ok()?;
    let mut in_package = false;
    for line in txt.lines() {
        if let Some(section) = toml_section(line) {
            in_package = section == "package"; // only [package]'s `name` is the crate name
            continue;
        }
        // `name` inside `[package]` only (a `name =` in `[[bin]]`/`[dependencies]` is not the crate name).
        if in_package {
            if let Some(v) = toml_scalar(line, "name") {
                return Some(v.replace('-', "_"));
            }
        }
    }
    None
}

/// The string entries of `key = [ ... ]` inside `[table]` — line-based (the manifest subset that
/// matters), multi-line arrays included. No TOML dependency, same trade as the parsers above.
pub(crate) fn toml_string_array(txt: &str, table: &str, key: &str) -> Vec<String> {
    let (mut in_table, mut collecting) = (false, false);
    let mut out = Vec::new();
    for line in txt.lines() {
        let l = line.trim();
        if !collecting {
            if let Some(section) = toml_section(line) {
                in_table = section == table;
                continue;
            }
        }
        if !in_table {
            continue;
        }
        let rest = if let Some(r) = l.strip_prefix(key) {
            let r = r.trim_start();
            let Some(r) = r.strip_prefix('=') else { continue };
            collecting = true;
            r
        } else if collecting {
            l
        } else {
            continue;
        };
        let mut parts = rest.split('"');
        parts.next();
        while let Some(s) = parts.next() {
            out.push(s.to_string());
            if parts.next().is_none() {
                break;
            }
        }
        if rest.contains(']') {
            collecting = false;
        }
    }
    out
}

/// True if the manifest declares a `[workspace]` table at all (distinct from "has members"): a
/// workspace root with zero RESOLVED members must warn, not silently fall through to a single-crate
/// scan whose nested-package filter then prunes every member into an empty report.
pub(crate) fn has_workspace_table(root: &Path) -> bool {
    std::fs::read_to_string(root.join("Cargo.toml"))
        .map(|t| t.lines().any(|l| l.trim() == "[workspace]"))
        .unwrap_or(false)
}

/// Member directories of the root manifest's `[workspace]`, joined to `root`, honouring `exclude`,
/// expanding globs (a bare `*` = root's immediate children, `prefix/*` = a dir's children), and
/// DEDUPLICATED (a member listed explicitly AND matched by a glob otherwise scans/prints twice).
/// Empty when there is no `members` key. A `*`-pattern this simple matcher can't expand is WARNED,
/// never silently dropped (a dropped member yields a vacuous gate, the §6.2 forbidden state).
pub(crate) fn workspace_members(root: &Path) -> Vec<String> {
    let Ok(txt) = std::fs::read_to_string(root.join("Cargo.toml")) else { return Vec::new() };
    let members = toml_string_array(&txt, "workspace", "members");
    if members.is_empty() {
        return Vec::new();
    }
    let exclude = toml_string_array(&txt, "workspace", "exclude");
    // Expand a `<base>/*` (base "" for a bare `*`) to its child dirs carrying a Cargo.toml.
    let expand = |base: &str| -> Vec<String> {
        let dir = if base.is_empty() { root.to_path_buf() } else { root.join(base) };
        let mut found: Vec<String> = std::fs::read_dir(dir)
            .into_iter()
            .flatten()
            .filter_map(Result::ok)
            .filter(|e| e.path().join("Cargo.toml").is_file())
            .map(|e| {
                let n = e.file_name().to_string_lossy().into_owned();
                if base.is_empty() { n } else { format!("{base}/{n}") }
            })
            .collect();
        found.sort();
        found
    };
    let mut rels: Vec<String> = Vec::new();
    for m in members {
        if m == "*" {
            rels.extend(expand(""));
        } else if let Some(base) = m.strip_suffix("/*") {
            rels.extend(expand(base));
        } else if m.contains('*') {
            eprintln!("candor-scan: workspace member glob `{m}` is not a trailing `*` — not expanded; \
                       scan its crates directly or list them explicitly");
        } else if root.join(&m).join("Cargo.toml").is_file() {
            rels.push(m);
        }
    }
    rels.retain(|m| !exclude.iter().any(|e| m == e || m.starts_with(&format!("{e}/"))));
    rels.sort();
    rels.dedup();
    rels.into_iter().map(|m| root.join(m).to_string_lossy().into_owned()).collect()
}
