//! The scan driver: argument walk (`scan_main`), per-crate scan + report emission
//! (`scan_one`), workspace fan-out (`scan_target`), and `--deps` chaining (`run_with_deps`).

use crate::*;

pub(crate) fn scan_main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let mut dir = ".".to_string();
    let mut prefix = String::new();
    let mut want_json = false;
    let mut include_tests = false;
    let mut policy_path: Option<String> = None;
    let mut gate_json_path: Option<String> = None;
    let mut deps_mode = false;
    let mut incremental = false;
    let mut it = args.iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "--out" => prefix = it.next().cloned().unwrap_or_default(),
            "--json" => want_json = true,
            "--include-tests" => include_tests = true,
            "--incremental" => incremental = true,
            "--policy" => {
                // A valueless trailing `--policy` (no path follows) must ERROR, not silently fall
                // back to no-gate — matching the strict posture of a set-but-unreadable policy.
                // Silently dropping the gate would let a violation ship under an intended-gated run.
                match it.next().cloned() {
                    Some(p) => policy_path = Some(p),
                    None => {
                        eprintln!("candor-scan: --policy requires a path argument");
                        std::process::exit(2);
                    }
                }
            }
            "--gate-json" => {
                // The structured gate verdict target (candor-spec §3.3). Valueless OR flag-shaped fails
                // closed (exit 2): without the dash-check, `--gate-json --policy pol` swallowed `--policy`
                // as the verdict path AND the displaced `pol` token replaced the scan dir (last-positional
                // -wins) — a gateless exit-0 run over the wrong target (max-review find, shipped in 0.8.x).
                // `-` (stream the verdict to stdout) stays valid.
                match it.next().cloned() {
                    Some(p) if p == "-" || !p.starts_with('-') => gate_json_path = Some(p),
                    _ => {
                        eprintln!("candor-scan: --gate-json requires a path argument");
                        std::process::exit(2);
                    }
                }
            }
            "--deps" => deps_mode = true,
            "-V" | "--version" => {
                // Two lines, fully OFFLINE: the installed build + the spec contract it speaks, then
                // the upgrade incantation. <spec> reuses candor_report::SPEC_VERSION — the same source
                // that stamps the report envelope's `spec` field, so the two can never drift.
                println!("candor-scan {} (spec {})", env!("CARGO_PKG_VERSION"), candor_report::SPEC_VERSION);
                println!("upgrade: cargo install candor-scan --force");
                return;
            }
            // The agent contract for THE INSTALLED VERSION, embedded at build time — doc and
            // binary cannot drift (the §2.1 version-trust rule applied to documentation). Agents
            // are told to run this instead of trusting a vendored/remote copy.
            "--agents" => {
                println!("<!-- candor-scan {} · the agent contract for this installed version -->", env!("CARGO_PKG_VERSION"));
                print!("{}", include_str!("../AGENTS.md"));
                return;
            }
            "-h" | "--help" => {
                println!("candor-scan {} — stable-Rust effect scanner (no nightly)", env!("CARGO_PKG_VERSION"));
                println!();
                println!("USAGE:  candor-scan [<dir>] [--out <prefix>] [--json] [--include-tests] [--policy <file>] [--gate-json <file>]");
                println!();
                println!("  <dir>             crate root to scan (default: .). A [workspace] root scans");
                println!("                    every member: one report per member under the one prefix.");
                println!("                    A nested dir with its own Cargo.toml is a different package");
                println!("                    and is never folded into the parent's report.");
                println!("  --out <prefix>    report path prefix (default: <dir>/.candor/report);");
                println!("                    writes <prefix>.<crate>.scan.json + a call-graph sidecar");
                println!("  --json            print the report to stdout instead of writing files");
                println!("  --include-tests   also scan tests/ benches/ examples/ and #[cfg(test)] modules");
                println!("                    (off by default → the report describes the crate, not its harness)");
                println!("  --incremental     reuse a per-file parse/decl cache under <dir>/.candor/cache so an");
                println!("                    edit-then-rescan skips re-parsing unchanged files (~7x on a one-file");
                println!("                    edit). Produces a BYTE-IDENTICAL report to a full scan; a candor-scan");
                println!("                    upgrade or a decl-changing edit invalidates the cache automatically.");
                println!("  --deps            scan the Cargo.lock dependency tree first (registry sources from");
                println!("                    ~/.cargo/registry/src) into <dir>/.candor/deps/, then scan <dir>");
                println!("                    CHAINED over those reports — effects cross every crate boundary");
                println!("                    without the classifier needing to know the crates.");
                println!("  --policy <file>   enforce a CANDOR_POLICY file (deny/pure/allow/forbid, spec §6.2)");
                println!("  --gate-json <f>   write the structured gate verdict {{ spec, ok, violations }} as JSON (spec §3.3)");
                println!("                    over this scan; exit 1 on violation. ADVISORY FLOOR: the syntactic");
                println!("                    backend under-reports, so a miss can pass — the nightly engine is");
                println!("                    the sound gate. (CANDOR_POLICY env is honoured when flag absent.)");
                println!();
                println!("  CANDOR_BASELINE=<p>  AS-EFF-005 regression guard: compare this scan against a saved");
                println!("                    report (a path or --out prefix; also the `baseline` config key). A fn");
                println!("                    GAINING an effect vs the baseline exits 1; absent baseline → note, guard");
                println!("                    inactive; unparseable or produced by a DIFFERENT build → exit 2 (§2.1 —");
                println!("                    never a stale compare). Record one: candor-scan <dir> --out <prefix>.");
                println!("  CANDOR_DEPS=<p:…> chain sibling reports (files or directories of *.json): an");
                println!("                    unclassified call into a crate a report covers inherits that");
                println!("                    function's effects + literal surfaces (spec §2). Scan the dep");
                println!("                    once, chain it everywhere; the coverage ledger names what to scan next.");
                println!("  -V, --version     print the installed build + spec contract (offline) and the upgrade line");
                println!();
                println!("Syntactic, so it under-reports vs the full candor nightly lint (Unknown only for");
                println!("callbacks/FFI/untrusted chained reports; other misses are silent). It never");
                println!("fabricates an effect. See https://github.com/tombaldwin/candor");
                return;
            }
            other => {
                // An unknown flag must FAIL, not become a path: an agent following a newer doc
                // against an older binary ran `candor-scan --agents` and scanned a directory
                // literally named `--agents`; a typo'd `--polcy` would silently drop the gate.
                if other.starts_with('-') {
                    eprintln!("candor-scan: unknown flag '{other}' (see --help)");
                    std::process::exit(2);
                }
                dir = a.clone();
            }
        }
    }
    // `.candor/config` (candor-spec §config): the checked-in floor under the env vars. Discovery is
    // anchored to the SCAN TARGET (walk up from `dir` to the repo root's .candor/config), never the CWD;
    // $CANDOR_CONFIG overrides discovery. FAIL-CLOSED when configured-but-unusable (exit 2 — the §6.2
    // unreadable-policy posture); only genuine absence is empty.
    let cfg = load_candor_config(&dir);
    // The policy source is resolved HERE, once (flag wins, CANDOR_POLICY env next, the config file as the
    // floor) — never inside scan_one, so --deps dependency scans can't inherit the root gate via the env.
    let policy = policy_path
        .or_else(|| std::env::var("CANDOR_POLICY").ok())
        .or_else(|| cfg.get("policy").cloned());
    // The AS-EFF-005 baseline (spec §7 item 5), resolved once like the policy: CANDOR_BASELINE env
    // over the config `baseline` key (already home-anchored by load_candor_config). Dependency scans
    // under --deps run guard-free — a dep's internals are not this repo's ratchet.
    let baseline = std::env::var("CANDOR_BASELINE").ok().or_else(|| cfg.get("baseline").cloned());
    // The --gate-json target rides a global (like INCREMENTAL below) so it threads no ScanOpts. Members
    // RECORD violations (record_gate_violations); the verdict is written ONCE here after the whole scan —
    // per-member writes let a clean last member overwrite an earlier violator's verdict (ok:true vs exit 1).
    // Dependency scans under --deps run gate-free (policy=None in scan_one), so they record nothing.
    let _ = GATE_JSON_PATH.set(gate_json_path);
    if deps_mode {
        let code = run_with_deps(&dir, prefix, want_json, include_tests, policy, baseline);
        write_gate_json(code);
        std::process::exit(code);
    }
    // Incremental is OPT-IN and SAFE: a full scan (no flag) never reads the cache, and `--incremental`
    // with no/invalid cache transparently does a full scan + populates it (the gates downgrade any
    // stale entry to a re-derivation). The flag rides in a thread-local so it doesn't thread through
    // every signature between `main` and `scan_one` (scan_target/run_with_deps are unchanged).
    INCREMENTAL.with(|c| c.set(incremental));
    // Cross-crate report chaining (spec §2): CANDOR_DEPS names sibling reports (a `:`-separated
    // list of files and/or directories of *.json); an unclassified qualified call into a crate one
    // of them covers inherits that function's recorded effects + literal surfaces. The stable
    // scanner's half of the dep-scan story: scan the dep once, chain it everywhere.
    let deps_spec = std::env::var("CANDOR_DEPS").ok().or_else(|| cfg.get("deps").cloned());
    let deps_idx = load_dep_reports(deps_spec.as_deref());
    // scan_target handles both a single crate and a `[workspace]` root (one report per member under
    // one prefix — candor-query's multi-crate merge consumes them together; the policy gates each).
    let code = scan_target(&dir, prefix, want_json, include_tests, policy, baseline, &deps_idx);
    write_gate_json(code);
    std::process::exit(code);
}

/// Options for one crate scan. `policy` and `baseline` are RESOLVED by the caller (flag/env/config) —
/// scan_one itself never reads the env, so dependency scans under --deps can genuinely run
/// gate-free (review: the env fallback inside scan_one ran the root policy 328 times against
/// dependency internals). `quiet` suppresses the per-scan receipts (dep scans; the --deps summary
/// line speaks for them).
pub(crate) struct ScanOpts<'a> {
    pub(crate) prefix: String,
    pub(crate) want_json: bool,
    pub(crate) include_tests: bool,
    pub(crate) policy: Option<String>,
    /// The AS-EFF-005 baseline value (`CANDOR_BASELINE` env / config `baseline` key): a saved report's
    /// path or `--out` prefix. See `check_baseline` for the full guard contract.
    pub(crate) baseline: Option<String>,
    pub(crate) quiet: bool,
    pub(crate) deps_idx: &'a DepIndex,
}

/// One crate scan, end to end (parse -> passes -> report -> receipt -> policy gate). Returns the
/// process exit code. Factored out of `main` so `--deps` can scan a dependency tree IN-PROCESS —
/// candor-scan's own self-gate (`deny Exec`) rightly forbids the spawn-yourself shortcut.
pub(crate) fn scan_one(dir: &str, opts: ScanOpts) -> (i32, Option<String>) {
    let ScanOpts { prefix, want_json, include_tests, policy: policy_path, baseline: baseline_value, quiet, deps_idx } = opts;
    let root = Path::new(dir);
    let crate_name = read_crate_name(root).unwrap_or_else(|| "crate".to_string());
    // Install this crate's cfg-feature picture (active = default closure, declared = all). A
    // `#[cfg(feature="X")]` compiled OUT under the default build is then skipped, so its effects don't
    // count as the crate's behaviour (winnow's debug-trace `std::env::var` fabricated Env). Set before the
    // parallel Pass B reads it; scan_one runs sequentially per workspace member, so members don't race.
    // Read the manifest HERE (the scan I/O layer); `lang::parse_features` stays a pure text pass (an
    // unreadable/absent Cargo.toml → empty string → no features, the same result as before the hoist).
    let cargo_toml = std::fs::read_to_string(root.join("Cargo.toml")).unwrap_or_default();
    set_cfg_features(parse_features(&cargo_toml));

    // Parse every in-scope .rs file ONCE (syn parses are reused across both passes below). The walk +
    // path-shape filters run SEQUENTIALLY (cheap directory traversal, and the filter set is the report's
    // scope contract); the per-file READ + `syn::parse_file` — profiled at ~77% parse + ~19% I/O of
    // wall-clock, and embarrassingly parallel since each file parses independently — is fanned out across
    // cores with rayon below. ORDER IS PRESERVED: paths are collected in walk order, `par_iter().collect()`
    // writes each result back at its own index (completion order is irrelevant), and the post-filter of
    // read/parse failures keeps the survivors' relative order — so `parsed` is byte-identical to the old
    // sequential push, and the report's fn order (which derives from it) does not move.
    let mut paths: Vec<(std::path::PathBuf, String)> = Vec::new();
    for entry in walkdir::WalkDir::new(root)
        .into_iter()
        // A nested directory carrying its own Cargo.toml is a DIFFERENT package (Cargo's own
        // semantics) — folding its files into this crate collides same-named fns across packages
        // and cross-wires the merged call graph (the repo-root self-scan merged 194 eval-fixture
        // `main`s into one unit). It gets its own scan: workspace member, --deps, or directly.
        .filter_entry(|e| {
            if e.depth() == 0 || !e.file_type().is_dir() {
                return true;
            }
            // Prune build/tooling dirs by NAME first — cheap, and it skips DESCENT into huge `target/`
            // and `.git/` trees (the dominant cost on a warm checkout) before the per-dir Cargo.toml
            // stat. A name starting with `.` is a hidden tooling dir (`.git`/`.github`/`.cargo`/…).
            let name = e.file_name().to_str().unwrap_or("");
            if name == "target" || (name.starts_with('.') && name != "." && name != "..") {
                return false;
            }
            // A nested dir carrying its own Cargo.toml is a DIFFERENT package (Cargo's own semantics):
            // folding its files into this crate collides same-named fns across packages and cross-wires
            // the merged call graph. It gets its own scan (workspace member, --deps, or directly).
            !e.path().join("Cargo.toml").is_file()
        })
        .filter_map(Result::ok)
    {
        let p = entry.path();
        if !p.is_file() || p.extension().and_then(|e| e.to_str()) != Some("rs") {
            continue;
        }
        // All path-shape filters run on the path RELATIVE to the scan root — an absolute prefix can itself
        // contain `target`/`.cargo`/… (a vendored crate lives under `~/.cargo/registry/...`), which must
        // not trip them.
        let rel = p.strip_prefix(root).unwrap_or(p);
        // target/ build artifacts; hidden dirs (`.git`, `.github`, `.cargo`, …) holding tooling/CI scripts,
        // not library code (smol_str's `.github/ci.rs` otherwise reported a phantom `Exec`).
        if rel.components().any(|c| {
            c.as_os_str()
                .to_str()
                .is_some_and(|s| s == "target" || (s.starts_with('.') && s != "." && s != ".."))
        }) {
            continue;
        }
        // The Cargo BUILD SCRIPT is `<crate-root>/build.rs` — it runs at COMPILE time (ring's build.rs
        // execs nasm), never the crate's runtime behaviour, so skip it. But ONLY at the root: a nested
        // `src/build.rs` is an ordinary source module that merely shares the name (git2's `src/build.rs`
        // is `RepoBuilder` — the whole clone/fetch NETWORK surface), and dropping it silently under-reports
        // (an A/B found `git2::Repository::clone` reporting no `Net` because its module had vanished).
        if is_build_script(rel) {
            continue;
        }
        // Cargo's non-library compilation targets (tests/, benches/, examples/) — and the common nonstandard
        // singular `test/` tree (e.g. nix) — describe what the crate's HARNESS does (spawn a server, read
        // fixtures, seed RNG), not what the crate itself does. Scanning them conflates the two (redis's bench
        // harness alone showed Exec/Net/Fs/Env/Rand on 200+ fns). Skip by default; `--include-tests` keeps them.
        if !include_tests
            && rel.components().any(|c| {
                matches!(
                    c.as_os_str().to_str(),
                    Some("tests") | Some("test") | Some("benches") | Some("examples")
                )
            })
        {
            continue;
        }
        // A `#[cfg(test)] mod tests;` FILE module is invisible here — its test-ness is declared at the
        // `mod` site, not in the file — so a `tests.rs` / `*_tests.rs` / `*_test.rs` file's effects (a
        // seeded RNG, a temp file) would be mis-read as the crate's. By convention these stems are test
        // modules; skip them by default. (base64's `engine/tests.rs` otherwise reported a phantom `Rand`.)
        if !include_tests {
            if let Some(stem) = p.file_stem().and_then(|s| s.to_str()) {
                if is_test_file_stem(stem) {
                    continue;
                }
            }
        }
        paths.push((p.to_path_buf(), rel.to_string_lossy().into_owned()));
    }

    // ── PARSE + Pass A + Pass B, with an OPTIONAL per-file cache (`--incremental`) ──────────────────
    // The non-incremental path is the original: parallel parse every file, run Pass A then Pass B over
    // all. The incremental path reuses an unchanged file's cached Pass A decls (skipping its parse) and,
    // when the merged decl index is unchanged, its cached Pass B FnInfos too — producing a byte-identical
    // assembled FnInfo set (the merges below replay the original walk-order accumulation exactly). See
    // the cache section above for the soundness argument.
    use rayon::prelude::*;
    let incremental = INCREMENTAL.with(|c| c.get());
    let schema = cache_schema(include_tests);
    let cache_dir = Path::new(dir).join(".candor").join("cache");
    let cache_path = cache_dir.join("scan-cache.json");

    // Load the SINGLE consolidated cache file (`rel -> FileCache`) in one read+deserialize — far cheaper
    // than 1 open per source file. A cache whose schema doesn't match this binary is discarded wholesale.
    let mut prior: HashMap<String, FileCache> = if incremental {
        std::fs::read(&cache_path)
            .ok()
            .and_then(|b| serde_json::from_slice::<ScanCache>(&b).ok())
            .filter(|c| c.schema == schema)
            .map(|c| c.files)
            .unwrap_or_default()
    } else {
        HashMap::new()
    };

    // CONTENT HASHES (cheap parallel reads, no parse). The cached entry for a file is reusable iff its
    // stored content_hash matches the bytes on disk now.
    let hashes: Vec<(String, String)> = paths
        .par_iter()
        .map(|(p, rel)| (rel.clone(), std::fs::read(p).map(|b| fnv1a(&b)).unwrap_or_default()))
        .collect();
    let per_file: Vec<(String, String, Option<FileCache>)> = hashes
        .into_iter()
        .map(|(rel, content_hash)| {
            let cached = prior
                .remove(&rel)
                .filter(|fc| fc.content_hash == content_hash);
            (rel, content_hash, cached)
        })
        .collect();

    // ROUND 1 PARSE (parallel): every file whose Pass A decls are NOT validly cached. A read/parse
    // failure yields `None` (the original `else { continue }`), so its slot carries no parsed file and
    // contributes nothing — identical to before.
    // Each entry is `Option<(SendFile, locs)>`: the `locs` are this file's `file:line:col`s in walk order,
    // resolved HERE on the parse worker because proc-macro2's span line/col only resolves against the
    // parsing thread's source map (see `fn_locs`/`SendFile`). They ride alongside the moved file so Pass B
    // (single-threaded) can zip them onto each FnInfo without re-resolving a now-dead span.
    let round1: Vec<Option<ParsedFile>> = per_file
        .par_iter()
        .map(|(rel, _, cached)| {
            if cached.is_some() {
                return None; // decls reusable from cache — defer the parse (it may not be needed at all)
            }
            let p = &paths.iter().find(|(_, r)| r == rel)?.0;
            let text = std::fs::read_to_string(p).ok()?;
            let file = syn::parse_file(&text).ok()?;
            let mut locs = Vec::new();
            fn_locs(&file.items, rel, include_tests, &mut locs);
            // SAFETY: see `SendFile` — freshly parsed, uniquely owned, moved once, then single-threaded.
            Some((SendFile(file), locs))
        })
        .collect();

    // DISCLOSE files that failed to read/parse (no cache AND round-1 None): their effects are NOT in
    // the report. A silent skip violates "never silently pure" — the query side already discloses an
    // unparseable REPORT; mirror it for unparseable SOURCE (adversarial review).
    let unparsed: Vec<&str> = per_file
        .iter()
        .zip(&round1)
        .filter(|(pf, parsed)| pf.2.is_none() && parsed.is_none())
        .map(|(pf, _)| pf.0.as_str())
        .collect();
    if !unparsed.is_empty() {
        let shown = unparsed.iter().take(8).copied().collect::<Vec<_>>().join(", ");
        let more = if unparsed.len() > 8 { format!(" + {} more", unparsed.len() - 8) } else { String::new() };
        eprintln!(
            "candor-scan: {} source file(s) failed to read/parse — effects in them are NOT in this report (re-check the source): {shown}{more}",
            unparsed.len()
        );
    }
    // Remember whether ANY in-scope source failed to parse: the policy gate below must FAIL non-zero
    // when a policy is configured AND analysis was incomplete — a gateless-green over unanalyzed code
    // is a missed-effect = false-pure hole. (`unparsed` borrows `per_file`, consumed below; keep a flag.)
    let had_parse_failure = !unparsed.is_empty();

    // Per-file Pass A decls (cache or fresh) + a place to hold a parsed file for Pass B. A file dropped
    // by a read/parse failure (no cache AND round-1 parse failed) is excluded entirely, preserving the
    // original survivor set + walk order.
    let mut decls_per_file: Vec<(String, String, FileDecls)> = Vec::new(); // (rel, content_hash, decls)
    let mut parsed_files: HashMap<String, syn::File> = HashMap::new();     // rel -> parsed (round 1)
    let mut parsed_locs: HashMap<String, Vec<String>> = HashMap::new();    // rel -> per-fn loc (walk order)
    let mut cached_fninfos: HashMap<String, (String, Vec<FnInfo>)> = HashMap::new(); // rel -> (decl_index_hash, fninfos)
    // Files whose on-disk entry was already valid for BOTH content + the decl index it recorded — no
    // re-write needed unless the merged index moves (checked after the digest). Lets a no-op / body-only
    // re-scan skip rewriting the whole cache dir (the dominant cost when nothing changed).
    let mut disk_decl_hash: HashMap<String, String> = HashMap::new();
    for ((rel, ch, cached), r1) in per_file.into_iter().zip(round1) {
        match cached {
            Some(fc) => {
                // Decls reusable; the FnInfos are CONDITIONALLY reusable (checked after the digest).
                disk_decl_hash.insert(rel.clone(), fc.decl_index_hash.clone());
                decls_per_file.push((rel.clone(), ch, fc.decls));
                cached_fninfos.insert(rel, (fc.decl_index_hash, fc.fninfos));
            }
            None => {
                // A freshly-parsed file (or a parse failure → skip the file entirely, as before).
                let Some((sf, locs)) = r1 else { continue };
                let fd = file_decls(&sf.0.items, include_tests);
                decls_per_file.push((rel.clone(), ch, fd));
                parsed_locs.insert(rel.clone(), locs);
                parsed_files.insert(rel, sf.0);
            }
        }
    }

    // Pass A MERGE — replay the original accumulation in WALK ORDER over the per-file decls, so the
    // crate-wide index is byte-identical to the old sequential `collect_decls` loop.
    let mut merged = MergedDecls::default();
    for (_, _, fd) in &decls_per_file {
        merge_decls(&mut merged, fd);
    }
    let decl_index_hash = decl_index_digest(&merged);
    // Keep only unambiguous fn-leaf -> return-type / enum-variant-payload mappings (the `None`s drop).
    let returns: ReturnIndex =
        merged.rets.iter().filter_map(|(k, v)| v.clone().map(|t| (k.clone(), t))).collect();
    let enum_variants: EnumVariantIndex =
        merged.enum_tmp.iter().filter_map(|(k, v)| v.clone().map(|t| (k.clone(), t))).collect();
    let fields = &merged.fields;
    let field_elem = &merged.field_elem;
    let trait_impls = &merged.trait_impls;
    let trait_decls = &merged.trait_decls;
    let trait_fields = &merged.trait_fields;
    let traits = TraitIndexes { impls: trait_impls, decls: trait_decls, fields: trait_fields };
    let elems = ElemIndexes { field_elem, enum_variants: &enum_variants };
    let lazy_statics = &merged.lazy_statics;

    // ROUND 2 PARSE (parallel): files whose decls were cached but whose FnInfos are STALE (the merged
    // decl index moved) — exactly the files a decl-changing edit invalidates. On a body-only edit this
    // set is empty; on a decl edit it is "everything else", re-parsed in parallel (degrade-to-full).
    let need_passb: Vec<&str> = decls_per_file
        .iter()
        .map(|(rel, _, _)| rel.as_str())
        .filter(|rel| {
            !parsed_files.contains_key(*rel)
                && cached_fninfos.get(*rel).map(|(h, _)| h != &decl_index_hash).unwrap_or(true)
        })
        .collect();
    let round2: Vec<(String, Option<ParsedFile>)> = need_passb
        .par_iter()
        .map(|rel| {
            let parsed = paths
                .iter()
                .find(|(_, r)| r == rel)
                .and_then(|(p, _)| std::fs::read_to_string(p).ok())
                .and_then(|t| syn::parse_file(&t).ok())
                .map(|file| {
                    // Resolve loc on THIS parse worker (span line/col is thread-local) — same as round 1.
                    let mut locs = Vec::new();
                    fn_locs(&file.items, rel, include_tests, &mut locs);
                    (SendFile(file), locs)
                });
            (rel.to_string(), parsed)
        })
        .collect();
    for (rel, sf) in round2 {
        if let Some((sf, locs)) = sf {
            parsed_locs.insert(rel.clone(), locs);
            parsed_files.insert(rel, sf.0);
        }
    }

    // Pass B — assemble each file's FnInfos in WALK ORDER: reuse the cached set when the decl index is
    // unchanged, else re-derive from the (now parsed) file. Either way the concatenated `fns` is exactly
    // what the old single Pass B loop produced.
    let mut fns: Vec<FnInfo> = Vec::new();
    let mut fresh_fninfos: HashMap<String, Vec<FnInfo>> = HashMap::new();
    for (rel, _, _) in &decls_per_file {
        let reuse = cached_fninfos
            .get(rel)
            .filter(|(h, _)| *h == decl_index_hash)
            .map(|(_, v)| v.clone());
        if let Some(v) = reuse {
            fns.extend(v.iter().cloned());
            continue;
        }
        // Re-derive: the file is parsed (round 1 or round 2); if both parses failed it's simply absent.
        let Some(file) = parsed_files.get(rel) else { continue };
        let modpath = module_path(Path::new(rel));
        // Locs were resolved on the parse worker (spans are dead on this thread); reuse them positionally.
        let locs = parsed_locs.get(rel).map(Vec::as_slice).unwrap_or(&[]);
        let mut loc_idx = 0usize;
        let mut uses = HashMap::new();
        let mut file_fns: Vec<FnInfo> = Vec::new();
        scan_items(&file.items, &modpath, locs, &mut loc_idx, include_tests, fields, &returns, traits, elems, lazy_statics, &mut uses, &mut file_fns);
        fns.extend(file_fns.iter().cloned());
        fresh_fninfos.insert(rel.clone(), file_fns);
    }

    // WRITE BACK the cache (incremental only) as ONE consolidated file. Each entry persists {content_hash,
    // decls, decl_index_hash, fninfos}; the FnInfos written are the CURRENT ones (reused or freshly
    // derived) tagged with the CURRENT decl_index_hash, so the next scan's gate is exact. The map is
    // rebuilt from the current path set, so deleted/renamed files drop out automatically (no pruning pass).
    // The write is SKIPPED entirely when nothing changed — every file's decls came from cache AND already
    // recorded this decl_index_hash AND no file was added/removed — so a no-edit re-scan does zero writes.
    // Best-effort: a cache write failure never affects the report (it only costs a re-derivation later).
    if incremental {
        let unchanged = fresh_fninfos.is_empty()
            && prior.is_empty() // every prior entry was consumed by a current file → none deleted
            && decls_per_file.iter().all(|(rel, _, _)| disk_decl_hash.get(rel) == Some(&decl_index_hash));
        if !unchanged {
            let mut files: HashMap<String, FileCache> = HashMap::with_capacity(decls_per_file.len());
            for (rel, ch, fd) in &decls_per_file {
                let fninfos = fresh_fninfos
                    .get(rel)
                    .cloned()
                    .or_else(|| cached_fninfos.get(rel).map(|(_, v)| v.clone()))
                    .unwrap_or_default();
                files.insert(
                    rel.clone(),
                    FileCache {
                        content_hash: ch.clone(),
                        decls: fd.clone(),
                        decl_index_hash: decl_index_hash.clone(),
                        fninfos,
                    },
                );
            }
            let cache = ScanCache { schema: schema.clone(), files };
            let _ = std::fs::create_dir_all(&cache_dir);
            if let Ok(bytes) = serde_json::to_vec(&cache) {
                let _ = candor_report::write_atomic(&cache_path, &bytes);
            }
        }
    }

    // The κ-coverage ledger: Cargo.toml's [dependencies] are the crate's TRUE external universe, so a
    // dep the calls actually reach whose classification never fires — and that isn't in a calibrated
    // tier — is a named blind spot (invisible, not Unknown: the curated-κ caveat). Counted here,
    // disclosed in the receipt, so the caveat is per-scan evidence instead of a doc footnote.
    let (deps, dep_renames) = cargo_deps(dir);
    let mut dep_seen: HashMap<String, usize> = HashMap::new(); // dep crate root -> call-site count
    let mut dep_classified: std::collections::HashSet<String> = std::collections::HashSet::new();
    // fn -> the dep crates it DIRECTLY calls into where the classifier floored the call. Post-filtered to
    // the genuinely-blind crates (κ never classified them) + propagated transitively → the per-fn
    // `invisible` honesty disclosure (the κ ledger, but attributed per function).
    let mut blind_direct: HashMap<String, BTreeSet<String>> = HashMap::new();
    // Blind crates inherited from a dep fn's `invisible` (sweep [8]): genuinely blind (the dep confirmed
    // it), but a TRANSITIVE crate the consumer never saw directly, so it is absent from `dep_seen` and
    // would be dropped by the `global_blind` filter. Collected here and unioned into global_blind below.
    let mut dep_invisible: BTreeSet<String> = BTreeSet::new();

    // Two name indexes for resolving a call to a local definition. `by_leaf` keys on the bare last
    // segment (`new`); `by_tail2` keys on the last TWO segments (`RequestBuilder::new`). The leaf index
    // alone catastrophically over-connects on real crates: every call to *some* `new()` would link to
    // ALL `*::new` defs (in reqwest, 181 of them), smearing one type's effect across the whole graph.
    // So a `Type::method`/`mod::fn` call matches the qualified tail (keeping `RequestBuilder::new` distinct
    // from `Body::new`) and a bare free call matches the leaf — BOTH only when the match is UNAMBIGUOUS
    // (exactly one def), under-reporting rather than fabricating. See `resolve_target` + the module doc.
    let mut by_leaf: HashMap<String, Vec<String>> = HashMap::new();
    let mut by_tail2: HashMap<String, Vec<String>> = HashMap::new();
    // Type names with a LOCAL definition — the penultimate `Type` segment of a `Type::method` qual. A
    // receiver-typed method call resolves to a local method ONLY if its type is in here, so an external
    // `reqwest::Client::send` can't mis-link to a same-named local `Client::send` (an inverse fabrication).
    let mut local_types: std::collections::HashSet<String> = std::collections::HashSet::new();
    for f in &fns {
        // SYNTHETIC lazy-init units (`<lazy>::NAME`) are resolved ONLY via the qualified `<lazy>::`
        // tail2 route a forcing site emits — they must NOT enter `by_leaf`, or a bare call to a real fn
        // sharing the static's NAME would see an ambiguous leaf and stop resolving (a spurious
        // under-report on unrelated code). Their tail2 (`<lazy>::NAME`) is unique and the forcing edge
        // always qualifies, so keeping them out of `by_leaf` loses nothing.
        let is_lazy_unit = f.qual.starts_with(LAZY_UNIT_PREFIX);
        if !is_lazy_unit {
            by_leaf.entry(f.leaf.clone()).or_default().push(f.qual.clone());
        }
        if let Some(t2) = tail2(&f.qual) {
            if let Some(ty) = t2.split("::").next() {
                if ty.chars().next().is_some_and(|c| c.is_uppercase()) {
                    local_types.insert(ty.to_string());
                }
            }
            by_tail2.entry(t2).or_default().push(f.qual.clone());
        }
    }

    // Inverse of trait_impls (impl-TYPE leaf → the trait leaves it impls), for the trait-DEFAULT-method
    // caller fallback below: a call `t.m()` on a concrete type T that does NOT declare `m` but impls a
    // trait with a DEFAULT `m` should edge to that trait's `Trait::m` (the inherited default body — now
    // scanned, via the Item::Trait arm). Without this the caller silently under-reported (`run()` calling
    // `l.flush()` on a FileLogger that inherits `Logger::flush`'s Fs/Net — adversarial review).
    let mut type_to_traits: HashMap<String, Vec<String>> = HashMap::new();
    for (tr, types) in &merged.trait_impls {
        let tr_leaf = tr.rsplit("::").next().unwrap_or(tr).to_string();
        for ty in types {
            let ty_leaf = ty.rsplit("::").next().unwrap_or(ty).to_string();
            type_to_traits.entry(ty_leaf).or_default().push(tr_leaf.clone());
        }
    }
    // A type whose ONLY impl is an (empty / non-overriding) trait impl has NO fn unit of its own, so it
    // never entered `local_types` (built from fn quals above) — which made its typed calls un-`resolvable`
    // and GATED OUT the trait-default fallback below: `impl Logger for FileLogger {}` + `l.flush()`
    // inheriting `Logger::flush`'s effect read silent-pure (R30). Every type with a local trait impl IS
    // local — register it so the fallback (already present) can fire.
    for ty in type_to_traits.keys() {
        local_types.insert(ty.clone());
    }

    // Method leaves that name a LOCAL method definition (a `Type::method` qual whose `Type` is local).
    // A bare-leaf method CALL (`x.fastrand()`, recorded path==leaf, no `::`) whose leaf matches one of
    // these resolves to the project's OWN method, so the calibrated-crate classification of that leaf
    // (`fastrand` → Rand, `now` → Clock) must be SUPPRESSED — the local definition is authoritative. This
    // covers the case `resolve_target` deliberately leaves unresolved: a method on a receiver whose type
    // the scanner can't infer (`Mutex::lock()`'s guard, `self.state.lock()` → `MutexGuard<FastRand>`),
    // where no typed `FastRand::fastrand` sibling forms yet the leaf still names a local method. Suppress
    // on PRESENCE of a same-named local method, not on a recorded edge — under-reporting on the rare
    // ambiguous leaf beats fabricating an effect candor never observed (the precision failure). (Real tokio
    // sweep: `RngSeedGenerator::next_seed` calls `rng.fastrand()` through a lock guard → bare leaf
    // `fastrand` → Rand, propagated to ~14 fns incl `Runtime::new`.)
    let mut direct: HashMap<String, BTreeSet<&'static str>> = HashMap::new();
    let mut hosts: HashMap<String, BTreeSet<String>> = HashMap::new();
    let mut cmds: HashMap<String, BTreeSet<String>> = HashMap::new();
    let mut paths: HashMap<String, BTreeSet<String>> = HashMap::new();
    let mut tables: HashMap<String, BTreeSet<String>> = HashMap::new();
    // Effects whose literal SURFACE is INCOMPLETE for a fn: it has a Net reach whose host is invisible to
    // the gate (a Net call with no string-literal arg — a runtime host, or a builder terminal whose host was
    // on a pure builder candor doesn't capture). The AS-EFF-008 gate treats an incomplete surface as
    // uncertifiable EVEN with other visible hosts, so a benign literal can't MASK the invisible endpoint
    // (the same gate evasion fixed in candor-java 0.5.29). Generalized from Net to Exec/Fs/Db (a masked
    // path/table alongside a benign sibling literal defeated `opaque` and silently passed `allow Fs`/
    // `allow Db`) — the establishing-allowlist predicate per effect (is_net_establishing /
    // is_cmd_naming_method / is_fs_path_arg / is_db_query_arg), matching candor-java's surfaceIncomplete.
    let mut incomplete: HashMap<String, BTreeSet<&'static str>> = HashMap::new();
    let mut calls: HashMap<String, BTreeSet<String>> = HashMap::new();
    let mut loc: HashMap<String, String> = HashMap::new();
    // Per-fn DIRECT Unknown-origin reasons (the receipt's `unknownWhy`, spec §2). Coarse, like the lint's
    // per-trait tag: a callback we can't see through, an FFI/extern boundary, or a genuinely-unresolvable
    // bare call. Tracked so the disclosure names WHY, not just that an Unknown exists.
    let mut unknown_why: HashMap<String, BTreeSet<&'static str>> = HashMap::new();
    for f in &fns {
        loc.entry(f.qual.clone()).or_insert_with(|| f.loc.clone());
        // The body invoked a callable the scan can't see through (closure / fn-pointer value): it could
        // perform any effect, so record an honest `Unknown` (propagated like any effect, surfaced in the
        // receipt's unresolved count) instead of silently certifying the function pure.
        if f.unresolved {
            direct.entry(f.qual.clone()).or_default().insert("Unknown");
            unknown_why.entry(f.qual.clone()).or_default().insert("callback:unresolved call");
        }
        // DROP-GLUE (#3): local types this fn CONSTRUCTS that have a local `impl Drop`. The `Drop::drop`
        // body runs at scope exit — an implicit edge the call graph misses, so a guard that flushes/closes
        // on drop read silent-pure. Collected per-call below (a `T::*` associated-fn call where `T` has a
        // local Drop), then edged to `T::drop` after the call loop. Over-approximates toward the SOUND
        // direction (a constructed value is assumed to drop in this scope); gated to LOCAL drop types only,
        // so an external type's invisible Drop is never fabricated.
        let mut drops_here: BTreeSet<String> = BTreeSet::new();
        for c in &f.calls {
            let cr = c.path.split("::").next().unwrap_or("");
            let classified = candor_classify::classify(cr, &c.path)
                .or_else(|| scan_builder_entry_effect(cr, &c.path));
            // DROP-GLUE detection: a `T::assoc()` ASSOCIATED-FN call (a CONSTRUCTOR like `Guard::new`)
            // where `T` is a LOCAL drop type means a `T` value is CREATED in this scope and dropped at exit.
            // Record `T` so we edge to `T::drop` after the loop. CRUCIALLY gated to `!c.method`: a METHOD
            // call (`reg.poll()`, recorded typed as `Registration::poll`) operates on a BORROW and does NOT
            // own/drop the value here — including those over-connected every borrow-site to the drop body
            // (tokio: 170 fns). A constructor is `Type::fn(..)` syntax (an associated fn, `method=false`).
            // Excludes `T::drop` itself.
            if !merged.drop_types.is_empty() && !c.method && c.path.contains("::") && c.leaf != "drop" {
                if let Some(ty) = tail2(&c.path).and_then(|t2| t2.split("::").next().map(str::to_string)) {
                    if merged.drop_types.contains(&ty) {
                        drops_here.insert(ty);
                    }
                }
            }
            // κ ledger: a qualified call into a declared dependency. (A bare leaf has no `::`, so it
            // can't name a crate; a local module sharing a dep's name is the rare accepted ambiguity.)
            if c.path.contains("::") && deps.contains(cr) {
                *dep_seen.entry(cr.to_string()).or_insert(0) += 1;
                if classified.is_some() {
                    dep_classified.insert(cr.to_string());
                } else {
                    // a FLOORED dep call: candidate per-fn blind spot (filtered to genuinely-blind below).
                    blind_direct.entry(f.qual.clone()).or_default().insert(cr.to_string());
                }
            }
            // (The CANDOR_DEPS cross-crate JOIN moved BELOW — it must run AFTER `resolved_local`/
            // `suppress_bare_leaf` are known and be gated on them, else a local fn/method/module named like
            // a covered dep crate inherits that dep's effects onto a provably-pure LOCAL path — the same
            // fabrication the classifier's `resolved_local` guard prevents, which this join
            // never had. Found by the cross-jar sweep.)
            // Resolve the call to a local definition via the precise, uniqueness-filtered `resolve_target`.
            // A receiver-typed `Type::method` call (`x.go()` inferred to `S::go`) resolves to the local
            // method ONLY when `Type` is locally defined — this recovers the common `x.method()` edge that
            // a bare leaf can't safely provide, while an external `reqwest::Client::send` is left to the
            // classifier (its type isn't local, so it can't mis-link to a same-named local `Client::send`).
            // A non-typed call uses the leaf/qualified-tail routes; std/core/alloc are the classifier's.
            let resolvable = if c.is_macro {
                // A macro is never a call to a local FUNCTION. Its (possibly crate-local) qualified path
                // must NOT resolve to a same-named local fn, or that fn's effect is fabricated onto the
                // caller (a phantom-edge fabrication — the precision failure). Its effect still flows via `classified` / κ above.
                false
            } else if c.typed {
                tail2(&c.path)
                    .and_then(|t2| t2.split("::").next().map(str::to_string))
                    .is_some_and(|ty| local_types.contains(&ty))
            } else {
                !matches!(cr, "std" | "core" | "alloc")
            };
            // A `Type::assoc()` whose `Type` is a NON-NOMINAL alias (`type Inner = [u8; N]`) names a type
            // with no local impl — its assoc fn is std/core's, NOT a same-named local STRUCT's. Skip the
            // local link so the array alias's `Inner::default()` doesn't inherit `struct Inner`'s
            // effectful `Default` (the sled IVec fabrication).
            let aliased = tail2(&c.path)
                .and_then(|t2| t2.split("::").next().map(str::to_string))
                .is_some_and(|ty| merged.prim_aliases.contains(&ty));
            // Did this call resolve to a LOCAL definition (free fn, method, or a unique trait-default)?
            // If so the local def is AUTHORITATIVE and its effects flow through the `calls` edge — the
            // crate/FFI classifier MUST NOT also fire, or a pure local fn whose NAME collides with an FFI
            // tier (`sqlite3_step`/`git_clone`/`curl_*`/`SSL_*`) or a whole-crate rule (`getrandom`/
            // `fastrand`) inherits that crate's effect: FABRICATION on a provably-pure path, transitively
            // poisoning every caller (a fabrication — the precision failure the syntactic floor must never commit). The
            // bare-leaf-METHOD suppression below was the special case of this; this covers the general
            // case (free fns and qualified `Type::method` calls the bare-leaf guard missed).
            let mut resolved_local = false;
            if resolvable && !aliased {
                let targets = resolve_target(&c.path, &c.leaf, c.method, &by_tail2, &by_leaf);
                if let Some(targets) = targets {
                    resolved_local = true;
                    for t in targets {
                        if t != &f.qual {
                            calls.entry(f.qual.clone()).or_default().insert(t.clone());
                        }
                    }
                } else if c.method && c.typed {
                    // No `T::leaf` resolved (T doesn't declare `leaf`). If T impls EXACTLY ONE trait whose
                    // DEFAULT `leaf` body exists (a `Trait::leaf` FnInfo), the call inherits it — edge there.
                    // COLLISION-SAFE: zero or >1 distinct candidate → skip (the honest under-report; never
                    // guess between traits — the keying-collision discipline that keeps this from FABRICATING
                    // a wrong trait's effect onto the caller).
                    if let Some(t_type) = tail2(&c.path).and_then(|t2| t2.split("::").next().map(str::to_string)) {
                        if let Some(trs) = type_to_traits.get(&t_type) {
                            let mut hits: Vec<&String> = Vec::new();
                            for tr_leaf in trs {
                                if let Some(ts) = by_tail2.get(&format!("{tr_leaf}::{}", c.leaf)) {
                                    for t in ts {
                                        if !hits.contains(&t) {
                                            hits.push(t);
                                        }
                                    }
                                }
                            }
                            if hits.len() == 1 && hits[0] != &f.qual {
                                resolved_local = true;
                                calls.entry(f.qual.clone()).or_default().insert(hits[0].clone());
                            }
                        }
                        // AUTO-DEREF fallback (last, after inherent + trait-default — Rust's resolution
                        // order): a custom `impl Deref for t_type { type Target = U }` makes `recv.leaf()`
                        // dispatch to `U::leaf`. Chase the Deref chain (bounded) and edge to the first
                        // `U::leaf` that resolves — the user-Deref analog of the Box/Arc/Rc peel (a newtype
                        // `impl Deref` dropped `wrapper.method()` to silent-pure — corpus find). `.clone()`
                        // is guarded at the typed-call emit, so no pointee-clone fabrication recurs.
                        if !resolved_local {
                            let mut cur = t_type.clone();
                            let mut hops = 0;
                            while let Some(target) = merged.deref_target.get(&cur).cloned() {
                                if hops >= 8 { break; }
                                hops += 1;
                                if let Some(ts) = resolve_target(&format!("{target}::{}", c.leaf), &c.leaf, false, &by_tail2, &by_leaf) {
                                    resolved_local = true;
                                    for t in ts {
                                        if t != &f.qual {
                                            calls.entry(f.qual.clone()).or_default().insert(t.clone());
                                        }
                                    }
                                    break;
                                }
                                cur = target;
                            }
                        }
                    }
                }
            }
            // A BARE-LEAF method call (`self.fastrand()` → path == leaf, no `::`) carries no crate
            // qualifier, so its `classify` consults the bare leaf against the calibrated crate/verb rules
            // (`fastrand` → Rand, `now` → Clock, …). When that leaf names a LOCAL method definition
            // (`local_method_leaves`), the call resolves to the project's OWN method — the local definition
            // is AUTHORITATIVE — so a local method merely NAMED like a calibrated crate (tokio's pure
            // `FastRand::fastrand` xorshift) must NOT inherit the crate's effect. Suppress the bare-leaf
            // classification; the effect (if any) flows from the resolved target through propagation. The
            // external-crate classification of a bare leaf still applies when NO local method shares the name
            // (a genuine `fastrand::u32` dependency call). Qualified calls keep their type-precise rule.
            // A BARE leaf (no `::`) naming ANY local definition (`by_leaf` keys every local fn/method by
            // leaf) is the project's OWN — the local def is authoritative. Suppress the bare-leaf
            // classifier (and the dep-join below). This covers the bare-leaf METHOD case AND the bare-leaf
            // FREE-FN case the old `c.method && local_method_leaves` guard missed: a pure local free fn
            // whose leaf is AMBIGUOUS (≥2 local defs, e.g. a free `git_clone` + a trait method `git_clone`)
            // defeats `resolve_target`'s uniqueness filter (→ `resolved_local=false`), so the FFI/crate
            // classifier fired unsuppressed and fabricated the effect (the precision failure). A bare leaf with no
            // local def (a genuine prelude/extern call) still classifies; a `use`-imported call is
            // qualified (`::`) and keeps its type-precise rule.
            let suppress_bare_leaf = !c.path.contains("::") && by_leaf.contains_key(&c.leaf);
            // CANDOR_DEPS cross-crate JOIN (spec §2), GATED: an UNCLASSIFIED qualified call into a crate a
            // sibling report covers inherits that fn's recorded effects + literal surfaces — UNLESS the call
            // resolved to a local target or names a local bare leaf (then the local is authoritative; the
            // join would fabricate). Joined unambiguous-tail2-first, then unambiguous leaf, like resolve_target.
            // A renamed dep joins under its real package name.
            let cr_real: &str = dep_renames.get(cr).map(String::as_str).unwrap_or(cr);
            let mut dep_join_hit = false;
            if classified.is_none() && !resolved_local && !suppress_bare_leaf
                && c.path.contains("::") && deps_idx.crates.contains(cr_real)
            {
                let rel = c.path.strip_prefix(&format!("{cr}::")).unwrap_or(&c.path);
                let hit = if rel.contains("::") {
                    tail2(rel).and_then(|t2| deps_idx.by_key.get(&format!("{cr_real}#{t2}")))
                } else {
                    deps_idx.by_key.get(&format!("{cr_real}#{rel}"))
                };
                if let Some(de) = hit {
                    dep_join_hit = true;
                    for e in &de.effects {
                        direct.entry(f.qual.clone()).or_default().insert(e);
                    }
                    hosts.entry(f.qual.clone()).or_default().extend(de.hosts.iter().cloned());
                    cmds.entry(f.qual.clone()).or_default().extend(de.cmds.iter().cloned());
                    paths.entry(f.qual.clone()).or_default().extend(de.paths.iter().cloned());
                    tables.entry(f.qual.clone()).or_default().extend(de.tables.iter().cloned());
                    // sweep [8]: inherit the dep fn's blind-crate disclosure so a consumer's pure verdict
                    // stays qualified across the chain boundary (else the dep's floored reach reads as plain
                    // pure here). Propagated with the local blind spots below.
                    blind_direct.entry(f.qual.clone()).or_default().extend(de.invisible.iter().cloned());
                    dep_invisible.extend(de.invisible.iter().cloned());
                    // sweep [30]: inherit the dep fn's masking-incompleteness so a benign literal here can't
                    // certify the dep's invisible runtime endpoint (propagated with the local incomplete map).
                    if !de.incomplete.is_empty() {
                        incomplete.entry(f.qual.clone()).or_default().extend(de.incomplete.iter().copied());
                    }
                    dep_classified.insert(cr.to_string());
                }
            }
            if let Some(eff) = classified.filter(|_| !suppress_bare_leaf && !resolved_local) {
                direct.entry(f.qual.clone()).or_default().insert(eff);
                // A host-ESTABLISHING Net / program-NAMING Exec call with NO captured literal → the endpoint
                // is invisible to the gate (a runtime value). Mark the surface incomplete so a benign captured
                // literal can't certify it (the masking evasion). Establishing-allowlist via the SHARED
                // predicate (is_net_establishing / is_cmd_naming_method) — same as the deep engine — so a
                // USE-verb (`stream.write()`) whose host was fixed at `connect` never false-positives.
                if c.str_arg.is_none() {
                    if eff == "Net" && candor_classify::is_net_establishing(&c.leaf) {
                        incomplete.entry(f.qual.clone()).or_default().insert("Net");
                    } else if eff == "Exec" && candor_classify::is_cmd_naming_method(&c.leaf) {
                        incomplete.entry(f.qual.clone()).or_default().insert("Exec");
                    } else if eff == "Fs" && !c.method && candor_classify::is_fs_path_arg(&c.leaf) {
                        // A path-NAMING Fs call (`fs::write(p,…)`/`File::open(p)` — a free fn / constructor,
                        // `method=false`) with NO captured path literal → the path is a runtime value,
                        // invisible to the gate. Mark Fs incomplete so a benign sibling literal can't certify
                        // the masked path (`allow Fs` fails closed). The `!c.method` gate excludes the
                        // path-stat METHODS (`p.metadata()`/`p.exists()`) whose path is the RECEIVER, not an
                        // arg — same establishing-allowlist discipline as Net/Exec (matches candor-java).
                        incomplete.entry(f.qual.clone()).or_default().insert("Fs");
                    } else if eff == "Db" && candor_classify::is_db_query_arg(&c.leaf) {
                        // A SQL-QUERY-bearing Db call (`con.execute(sql,…)`/`query`/`prepare`) with NO captured
                        // query literal → the table is a runtime value, invisible to the gate. Mark Db
                        // incomplete so a benign sibling literal can't certify the masked table. The allowlist
                        // excludes build-then-execute terminals (`fetch_all`/`load`/`all`) and lifecycle ops
                        // (`connect`/`open`/`begin`) whose query is built structurally (no maskable string).
                        incomplete.entry(f.qual.clone()).or_default().insert("Db");
                    }
                }
                if let Some(s) = &c.str_arg {
                    match eff {
                        "Net" => { hosts.entry(f.qual.clone()).or_default().insert(host_part(s)); }
                        "Exec" => {
                            // Capture the program head + refine the cliff (spec §4 ⟨0.5⟩) ONLY at a
                            // program-NAMING call (`new`/`cmd`), an ALLOWLIST — not "any method except a
                            // known modifier". A whole-crate-Exec crate (portable_pty/duct) classifies
                            // EVERY method as Exec, so a denylist leaked non-naming methods (a getter
                            // `get_env("psql")` reads back a KEY, not a program) → fabricated Db + polluted
                            // the `cmds` surface (a false `allow Exec` match). Method = the path's last segment.
                            if candor_classify::is_cmd_naming_method(c.path.rsplit("::").next().unwrap_or("")) {
                                cmds.entry(f.qual.clone()).or_default().insert(s.clone());
                                direct.entry(f.qual.clone()).or_default()
                                    .extend(candor_classify::classify_command_head(s).iter().copied());
                            }
                        }
                        "Fs" => { paths.entry(f.qual.clone()).or_default().insert(s.clone()); }
                        // Table-position identifiers in a SQL string literal — the Db literal
                        // surface (feeds `allow Db …`); a dynamically-built query yields nothing.
                        "Db" => { tables.entry(f.qual.clone()).or_default().extend(candor_classify::tables_in_sql(s)); }
                        _ => {}
                    }
                }
            }
            // §4 HONESTY — FFI BOUNDARY: a call that fell through EVERY resolution route above (not
            // classified, not a local def, not a dep-report join) AND names a fn declared in an `extern`
            // block is the canonical unknowable boundary — its body is in another language, so the effect
            // (Fs/Net/Exec/…) is unknowable. DISCLOSE Unknown — the same honest signal an unresolved
            // callback gets — instead of silent-pure. A safe wrapper `unsafe { system(cmd) }` otherwise
            // read pure (the `extern` block was never collected, so the call was a bare leaf resolving to
            // nothing → pure). NEVER fires when a LOCAL def of the same name exists (`suppress_bare_leaf`
            // / `resolved_local` win — the local is authoritative, no fabrication).
            //
            // (The general "any unresolvable bare call → Unknown" disclosure was PROTOTYPED and REJECTED:
            // it floods on a real corpus — closure-param invocations (`func(x)`), macro-DEFINED local
            // helpers absent from `by_leaf`, and cfg-gated platform fns all read as bare-unresolved, so it
            // charged ~80 pure tokio fns Unknown for ~0 genuine signal beyond this FFI case. See the task
            // report's residual note. The extern case below is the precise, non-flooding subset.)
            let already_handled = classified.is_some() || resolved_local || suppress_bare_leaf || dep_join_hit;
            if !c.is_macro && !already_handled && merged.extern_fns.contains(&c.leaf) {
                direct.entry(f.qual.clone()).or_default().insert("Unknown");
                unknown_why.entry(f.qual.clone()).or_default().insert("native:extern fn"); // FFI is a native boundary — canonical `native:` (SPEC §4 ⟨0.7⟩)
            }
            // §4 HONESTY — AMBIGUOUS LOCAL: a BARE leaf naming TWO-OR-MORE local defs (`tail2`/leaf
            // collision: a free `tail2` + a `Type::tail2` method, or two `Type::method`s) defeats
            // `resolve_target`'s uniqueness filter (resolved_local=false) AND is suppressed from the
            // classifier/dep-join (`suppress_bare_leaf` — the local is authoritative). Today that leaves
            // NO edge and NO disclosure: the callee's effects vanish (silent-pure over a real local call).
            // DISCLOSE Unknown instead — we can't pick WHICH local def runs, so its effects are unknown,
            // not absent. PRECISELY scoped (≥2 local defs of this bare leaf) so it can't flood like the
            // rejected "any unresolvable bare call → Unknown": a closure-param call / macro-helper isn't in
            // `by_leaf`, and a UNIQUE leaf resolves through `resolve_target` (never reaches here).
            // EXCLUDE method calls (`x.run()`): an unqualified method call already resolves to NOTHING by
            // design (the `method` flag — linking it to a same-named def would guess/fabricate), and a
            // same-named method is the COMMON case (`run`/`get`/`handle` across many types), so firing here
            // floods every such call with Unknown. This disclosure is for genuinely-bare FREE calls (the M1
            // case): `run()` with ≥2 free `run` defs, where the silent drop really is a lost local edge.
            if !c.is_macro && !c.method && classified.is_none() && !resolved_local && suppress_bare_leaf
                && !c.path.contains("::")
                && by_leaf.get(&c.leaf).is_some_and(|v| v.len() >= 2)
            {
                direct.entry(f.qual.clone()).or_default().insert("Unknown");
                unknown_why.entry(f.qual.clone()).or_default().insert("ambiguous:same-name local defs");
            }
        }
        // DROP-GLUE EDGE (#3): for each LOCAL drop type this fn constructed, add the implicit scope-exit
        // edge to its `T::drop` body — but ONLY when that body is a UNIQUE local def (in `by_tail2` with
        // exactly one target), the same uniqueness discipline `resolve_target` uses. The drop body's
        // effects then propagate to `f` like any other callee (a flushing/closing guard stops reading
        // silent-pure). Self-edges are skipped (a `Drop::drop` that constructs its own type).
        for ty in &drops_here {
            if let Some(targets) = by_tail2.get(&format!("{ty}::drop")) {
                if targets.len() == 1 && targets[0] != f.qual {
                    calls.entry(f.qual.clone()).or_default().insert(targets[0].clone());
                }
            }
        }
    }

    let all: Vec<String> = fns.iter().map(|f| f.qual.clone()).collect();
    let inferred = propagate(&direct, &calls, &all);
    let hostsacc = propagate_str(&hosts, &calls, &all);
    let cmdsacc = propagate_str(&cmds, &calls, &all);
    let pathsacc = propagate_str(&paths, &calls, &all);
    let tablesacc = propagate_str(&tables, &calls, &all);
    let incompleteacc = propagate(&incomplete, &calls, &all); // transitive masking-incompleteness
    let blind_acc = propagate_str(&blind_direct, &calls, &all); // transitive per-fn blind reach
    // The genuinely-blind dep crates (the per-scan κ "unlisted" set): seen, never classified, not
    // dep-report-covered, not calibrated. A fn's `invisible` = its transitive blind reach ∩ this set.
    let global_blind: std::collections::HashSet<String> = dep_seen
        .keys()
        .filter(|cr| {
            !dep_classified.contains(*cr)
                && !deps_idx.crates.contains(dep_renames.get(cr.as_str()).map(String::as_str).unwrap_or(cr.as_str()))
                && !candor_classify::CALIBRATED_CRATES.contains(&cr.as_str())
                && !candor_classify::PATH_CALIBRATED_CRATES.contains(&cr.as_str())
                && !candor_classify::CALIBRATED_PREFIXES.iter().any(|p| cr.starts_with(p))
        })
        .cloned()
        .collect();
    // A dep-inherited invisible crate is genuinely blind (the dep's own scan confirmed it) but transitive,
    // so it never appears in `dep_seen` — keep it so the consumer's `invisible` survives the filter ([8]).
    let global_blind: std::collections::HashSet<String> =
        global_blind.into_iter().chain(dep_invisible).collect();

    let mut entries: Vec<ReportEntry> = Vec::new();
    let mut cg: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for q in &all {
        // SPEC §2.2: the sidecar records EVERY analyzed function — including a LEAF with no local
        // callees, as an empty list. Omitting leaves made an uncalled FFI-only fn (nix `unistd::pipe`)
        // invisible to `whatif`/`callers` ("no function matching") even though it's in the report;
        // an always-present key also lets a consumer distinguish "no callers" from "no such function".
        cg.insert(q.clone(), calls.get(q).map(|cs| cs.iter().cloned().collect()).unwrap_or_default());
        let inf = inferred.get(q).cloned().unwrap_or_default();
        // Keep a pure fn if it has a BLIND reach — so the honesty disclosure survives on exactly the
        // `inferred: []` fns that need it (else `invisible` would be dropped with the pure entry).
        let has_blind = blind_acc.get(q).is_some_and(|s| s.iter().any(|c| global_blind.contains(c)));
        if inf.is_empty() && !has_blind {
            continue;
        }
        entries.push(ReportEntry {
            func: q.clone(),
            loc: loc.get(q).cloned().unwrap_or_default(),
            inferred: inf.iter().map(|s| s.to_string()).collect(),
            direct: direct.get(q).map(|d| d.iter().map(|s| s.to_string()).collect()).unwrap_or_default(),
            declared: Vec::new(),
            undeclared: Vec::new(),
            overdeclared: Vec::new(),
            // Honest blind-spot signal: this function (transitively) reached a callable the scan couldn't
            // see through. Mirrors the lint's `unresolved = has Unknown`, so the receipt's unresolved
            // count is truthful for the stable backend too — not a hardcoded 0.
            unresolved: inf.contains("Unknown"),
            // The cross-crate join key (spec §2): `crate#qual`, derivable by any consumer from its
            // own syntactic view of the call — what CANDOR_DEPS chaining matches against.
            hash: format!("{crate_name}#{q}"),
            fs: Vec::new(),
            hosts: hostsacc.get(q).map(|s| s.iter().cloned().collect()).unwrap_or_default(),
            cmds: cmdsacc.get(q).map(|s| s.iter().cloned().collect()).unwrap_or_default(),
            paths: pathsacc.get(q).map(|s| s.iter().cloned().collect()).unwrap_or_default(),
            tables: tablesacc.get(q).map(|s| s.iter().cloned().collect()).unwrap_or_default(),
            calls: calls.get(q).map(|cs| cs.iter().cloned().collect()).unwrap_or_default(),
            // DIRECTLY-introduced Unknown origins (candor-spec §2 `unknownWhy`): an unresolved callback /
            // fn-pointer call, an FFI/extern boundary, or a genuinely-unresolvable bare call. Coarser than
            // the lint's per-trait tag — by design — but now names WHICH boundary, not just "callback".
            unknown_why: unknown_why
                .get(q)
                .map(|s| s.iter().map(|r| r.to_string()).collect())
                .unwrap_or_default(),
            // candor-spec §2 `entryPoint`: syntactically we can only spot `main` (the program root). The
            // lint also flags `#[no_mangle]`; the scanner can't see attributes, so it under-marks — the
            // sound direction for an optional reachability hint.
            entry_point: q.rsplit("::").next() == Some("main"),
            // Per-fn honesty: the genuinely-blind crates this fn transitively reaches. `inferred` is a
            // LOWER BOUND when this is non-empty.
            invisible: blind_acc
                .get(q)
                .map(|s| s.iter().filter(|c| global_blind.contains(*c)).cloned().collect())
                .unwrap_or_default(),
            // Masking-incomplete effects — carried so a CANDOR_DEPS consumer inherits the incompleteness
            // across the crate boundary (sweep [30]); the gate already fails closed locally on it.
            incomplete: incompleteacc.get(q).map(|s| s.iter().map(|e| e.to_string()).collect()).unwrap_or_default(),
        });
    }
    entries.sort_by(|a, b| a.func.cmp(&b.func));

    let meta = candor_report::ReportMeta {
        version: format!("scan-{}", env!("CARGO_PKG_VERSION")),
        toolchain: "stable".into(),
        spec: candor_report::SPEC_VERSION.into(),
    };
    let body = candor_report::to_packaged_report_json(&meta, &crate_name, &entries).unwrap_or_default();
    // With want_json the body is RETURNED to the caller (which prints one document for a single
    // crate, or wraps N members in a JSON array) rather than printed here — printing per-call gave
    // concatenated, unparseable JSON for a workspace scan.
    let json_body = if want_json {
        Some(body.clone())
    } else {
        let prefix = if prefix.is_empty() { format!("{dir}/.candor/report") } else { prefix };
        if let Some(parent) = Path::new(&prefix).parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let file = format!("{prefix}.{crate_name}.scan.json");
        // Atomic write (temp + rename): a concurrent `candor-query` / `cargo candor watch` reader must
        // never see a half-written report (see candor_report::write_atomic).
        let _ = candor_report::write_atomic(Path::new(&file), body.as_bytes());
        let cgfile = format!("{prefix}.{crate_name}.scan.callgraph.json");
        let _ = candor_report::write_atomic(Path::new(&cgfile), serde_json::to_string(&cg).unwrap_or_default().as_bytes());
        if !quiet {
            eprintln!(
                "candor-scan: wrote {} effectful functions to {file} (stable syntactic backend — see --help)",
                entries.len()
            );
            // Effect breakdown — make the result visible at a glance, not just a count + a file path.
            let mut counts: std::collections::HashMap<&str, usize> = std::collections::HashMap::new();
            for e in &entries {
                for x in &e.inferred {
                    *counts.entry(x.as_str()).or_insert(0) += 1;
                }
            }
            let breakdown = ["Net", "Fs", "Db", "Exec", "Ipc", "Env", "Clipboard", "Clock", "Log", "Rand"]
                .iter()
                .filter_map(|k| counts.get(k).map(|n| format!("{k} {n}")))
                .collect::<Vec<_>>()
                .join(" · ");
            let unknown = counts.get("Unknown").copied().unwrap_or(0);
            if !breakdown.is_empty() || unknown > 0 {
                let u = if unknown > 0 {
                    format!("{}Unknown {unknown} (disclosed)", if breakdown.is_empty() { "" } else { "   ·   " })
                } else {
                    String::new()
                };
                eprintln!("  {breakdown}{u}");
            }
        }
        None
    };

    // The κ-coverage disclosure: dependencies the code demonstrably CALLS that the classifier knows
    // nothing about. Their effects are INVISIBLE — not Unknown — so the report's silence about them
    // is not purity evidence. This turns the curated-κ caveat from a doc footnote into per-scan,
    // named evidence (the argon2 lesson: the blind spot landed on exactly the call a security review
    // cared about).
    let mut unlisted: Vec<(&String, usize)> = dep_seen
        .iter()
        .filter(|(cr, _)| {
            !dep_classified.contains(*cr)
                // A crate with a loaded sibling report is COVERED even when no join fired: the
                // report omits pure functions, so join-less calls are its honest purity claim —
                // the opposite of invisible. (Without this, --deps named serde_json a blind spot.)
                // A RENAMED dep is covered under its real package name.
                && !deps_idx.crates.contains(dep_renames.get(cr.as_str()).map(String::as_str).unwrap_or(cr.as_str()))
                && !candor_classify::CALIBRATED_CRATES.contains(&cr.as_str())
                && !candor_classify::PATH_CALIBRATED_CRATES.contains(&cr.as_str())
                && !candor_classify::CALIBRATED_PREFIXES.iter().any(|p| cr.starts_with(p))
        })
        .map(|(cr, n)| (cr, *n))
        .collect();
    if !unlisted.is_empty() && !quiet {
        unlisted.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(b.0)));
        let shown: Vec<String> =
            unlisted.iter().take(8).map(|(cr, n)| format!("{cr} ({n} call{})", if *n == 1 { "" } else { "s" })).collect();
        let more = if unlisted.len() > 8 { format!(" + {} more", unlisted.len() - 8) } else { String::new() };
        eprintln!(
            "candor-scan: candor's classifier doesn't cover {} dependenc{} this code calls into — their effects are INVISIBLE to the scan (absent from the report, NOT a claim they're pure): {}{}",
            unlisted.len(),
            if unlisted.len() == 1 { "y" } else { "ies" },
            shown.join(", "),
            more
        );
    }

    // The cold-repo hook: after the effect summary + κ ledger, surface the SINGLE most surprising
    // transitive reach + a ready-to-run `candor path` command — so the two-minute demo opener is
    // deterministic, not lucky. Pure call-graph + name analysis (no LLM); honest fallback when
    // nothing clears the bar. See surface.rs / SURFACE-BEST-FIND-DESIGN.md. STDERR only (stdout may
    // carry the JSON report), and AFTER the histogram + κ ledger in output order.
    if !quiet {
        crate::surface::emit(&inferred, &direct, &calls, &loc);
    }

    // Human gate output (the violation lines AND the ✓/count summaries) goes to STDERR whenever
    // stdout carries a JSON document — the report (`--json`) or the streamed verdict
    // (`--gate-json -`) — so stdout stays a single pure JSON document (pipeable to `jq` /
    // candor-sarif). Without this, the AS-EFF lines interleave the stream and corrupt it.
    let stdout_is_json = want_json || matches!(GATE_JSON_PATH.get(), Some(Some(p)) if p == "-");

    // The AS-EFF-005 baseline regression guard (spec §7 item 5) — candor-java's checkBaseline is the
    // model; see check_baseline for the full contract. Runs BEFORE the policy gate so both record
    // toward the one --gate-json verdict; the exit code is the max of the two (2 short-circuits).
    let mut guard_code = 0;
    if let Some(bv) = &baseline_value {
        // A configured guard over INCOMPLETE analysis (a source file failed to parse) must not
        // evaluate: the unparsed file's effects are absent, so a clean compare over it is a
        // false-pure (the same posture as the policy gate below).
        if had_parse_failure {
            eprintln!("candor-scan: baseline guard NOT evaluated — source failed to parse (see above); the guard cannot compare unanalyzed code");
            return (2, json_body);
        }
        match check_baseline(bv, dir, &crate_name, &all, &inferred) {
            BaselineOutcome::Inactive => {} // absent file: noted, exit unchanged
            BaselineOutcome::Invalid => return (2, json_body), // diagnostic already printed
            BaselineOutcome::Checked(v) => {
                for gv in &v {
                    let line = format!("[{}] {}", gv.rule, gv.detail);
                    if stdout_is_json {
                        eprintln!("{line}");
                    } else {
                        println!("{line}");
                    }
                }
                record_gate_violations(&v); // toward the final --gate-json verdict
                if v.is_empty() {
                    eprintln!("candor-scan: baseline guard ✓ — no function gained an effect (advisory floor: the syntactic backend under-reports)");
                } else {
                    eprintln!("candor-scan: {} baseline regression(s) — an existing function gained an effect (AS-EFF-005)", v.len());
                    guard_code = 1;
                }
            }
        }
    }

    // The stable policy gate (spec §6.2 / AS-EFF-006/008/009) — the ADVISORY FLOOR. The syntactic
    // backend under-reports (a missed effect can pass), so this is a floor, never the sound gate
    // (that's the nightly engine / the JVM engine). It still catches every boundary crossing the
    // scan CAN see, deterministically, with zero extra install.
    if let Some(pp) = policy_path {
        let Ok(text) = std::fs::read_to_string(&pp) else {
            // A set-but-unreadable policy must be LOUD — silently passing would let a violation ship.
            eprintln!("candor-scan: policy {pp:?} could not be read; gate NOT enforced");
            return (2, json_body);
        };
        let v = policy_violations(&text, &all, &inferred, &calls, &hostsacc, &cmdsacc, &pathsacc, &tablesacc, &incompleteacc);
        for gv in &v {
            let line = format!("[{}] {}", gv.rule, gv.detail);
            if stdout_is_json {
                eprintln!("{line}");
            } else {
                println!("{line}");
            }
        }
        // A configured gate over INCOMPLETE analysis (a source file failed to parse) must NOT report
        // green: the unparsed file's effects are absent, so a `policy ✓` over it is a false-pure. Fail
        // exit 2 (mirroring the unreadable-policy posture) — never exit 0/1 with a clean-looking ✓. No
        // --gate-json verdict here: the analysis is incomplete, so there is no faithful verdict to emit.
        if had_parse_failure {
            eprintln!("candor-scan: policy NOT enforced — source failed to parse (see above); gate cannot be green over unanalyzed code");
            return (2, json_body);
        }
        record_gate_violations(&v); // toward the final --gate-json verdict (written once, by scan_main)
        // Provable-purity disclosure (advisory — NEVER changes the verdict/exit): pure/deny layers that PASS
        // but are Unknown. Surfaces the gap automatically so an author learns their "pure" layer isn't
        // PROVABLY pure (eval/fixloop/DISPATCH-NOTE.md); the `candor-query unverified` query has the detail.
        let holes = crate::gate::unverified_holes(&text, &all, &inferred);
        if !holes.is_empty() {
            let mut upgrades: BTreeSet<String> = BTreeSet::new();
            eprintln!(
                "candor-scan: note — {} function(s) PASS the policy but are Unknown (purity NOT verified — the Unknown could hide a forbidden effect):",
                holes.len()
            );
            for (fq, up) in &holes {
                eprintln!("    `{fq}`  → add  `{up}`");
                upgrades.insert(up.clone());
            }
            eprintln!(
                "  (advisory; add the upgrade(s) to REQUIRE provable purity, or run `candor-query unverified` for detail — the gate verdict is unchanged)"
            );
        }
        if v.is_empty() {
            eprintln!("candor-scan: policy ✓ (advisory floor — the syntactic backend under-reports; the nightly engine is the sound gate)");
        } else {
            eprintln!("candor-scan: {} policy violation(s) (advisory floor — a clean run is necessary, not sufficient)", v.len());
            return (1, json_body);
        }
    }
    // No-gate runs record nothing; scan_main's final write_gate_json emits { ok: true, [] } for them.
    // A clean policy still exits 1 when the baseline guard fired above (the codes join by max).
    (guard_code, json_body)
}

/// Scan a TARGET — a single crate, or a `[workspace]` root fanned out into one report per member
/// under the shared prefix. The one place both the plain and `--deps` paths funnel through, so a
/// workspace is never scanned as one merged package (colliding same-named fns) nor pruned to an
/// empty report by the nested-package filter. With `want_json`, prints ONE JSON document for a
/// single crate and a JSON ARRAY for a workspace — never concatenated documents. Returns the exit code.
#[allow(clippy::too_many_arguments)]
pub(crate) fn scan_target(
    dir: &str,
    prefix: String,
    want_json: bool,
    include_tests: bool,
    policy: Option<String>,
    baseline: Option<String>,
    deps_idx: &DepIndex,
) -> i32 {
    let members = workspace_members(Path::new(dir));
    if members.is_empty() {
        if has_workspace_table(Path::new(dir)) {
            // A [workspace] with zero RESOLVED members: scanning the root as one crate would let the
            // nested-package filter prune every member into an empty report that passes any gate
            // vacuously (§6.2's forbidden state). Warn loudly; the single-crate scan below still
            // covers the root package's own sources, if any.
            eprintln!("candor-scan: `{dir}` declares [workspace] but no members resolved — \
                       check `members`/globs; scan member crates directly to gate them");
        }
        let (code, json) = scan_one(dir, ScanOpts {
            prefix, want_json, include_tests, policy, baseline, quiet: false, deps_idx,
        });
        if let Some(b) = json {
            println!("{b}");
        }
        return code;
    }
    let prefix = if prefix.is_empty() { format!("{dir}/.candor/report") } else { prefix };
    let mut dirs: Vec<String> = Vec::new();
    if read_crate_name(Path::new(dir)).is_some() {
        dirs.push(dir.to_string()); // the workspace manifest also declares a root package
    }
    dirs.extend(members);
    let mut rc = 0;
    let mut bodies: Vec<String> = Vec::new();
    for d in &dirs {
        let (code, json) = scan_one(d, ScanOpts {
            prefix: prefix.clone(), want_json, include_tests, policy: policy.clone(),
            baseline: baseline.clone(), quiet: false, deps_idx,
        });
        rc = rc.max(code);
        if let Some(b) = json {
            bodies.push(b);
        }
    }
    if want_json {
        println!("[{}]", bodies.join(","));
    } else {
        eprintln!("candor-scan: workspace — {} package report(s) under one prefix", dirs.len());
    }
    rc
}

/// `--deps`: read Cargo.lock, scan every REGISTRY dependency's unbuilt source from
/// `~/.cargo/registry/src/<index>/` into `<dir>/.candor/deps/`, then scan the root crate chained
/// over those reports (plus anything CANDOR_DEPS already names). Path/git/workspace deps have no
/// registry checkout and are skipped with a note — chain them by scanning them yourself.
pub(crate) fn run_with_deps(dir: &str, prefix: String, want_json: bool, include_tests: bool, policy: Option<String>, baseline: Option<String>) -> i32 {
    let lock = match std::fs::read_to_string(format!("{dir}/Cargo.lock")) {
        Ok(t) => t,
        Err(_) => {
            eprintln!("candor-scan: --deps needs {dir}/Cargo.lock (run `cargo generate-lockfile` first)");
            return 2;
        }
    };
    // [[package]] blocks: name + version + source. Only registry deps have a checkout to scan;
    // the root crate itself has no `source` line and is naturally skipped.
    let mut pkgs: Vec<(String, String)> = Vec::new();
    let (mut name, mut version, mut registry) = (String::new(), String::new(), false);
    let flush = |name: &mut String, version: &mut String, registry: &mut bool, pkgs: &mut Vec<(String, String)>| {
        if *registry && !name.is_empty() && !version.is_empty() {
            pkgs.push((name.clone(), version.clone()));
        }
        name.clear();
        version.clear();
        *registry = false;
    };
    for line in lock.lines() {
        let l = line.trim();
        if l == "[[package]]" {
            flush(&mut name, &mut version, &mut registry, &mut pkgs);
        } else if let Some(v) = l.strip_prefix("name = ") {
            name = v.trim_matches('"').to_string();
        } else if let Some(v) = l.strip_prefix("version = ") {
            version = v.trim_matches('"').to_string();
        } else if l.starts_with("source = ") && l.contains("registry+") {
            registry = true;
        }
    }
    flush(&mut name, &mut version, &mut registry, &mut pkgs);

    let registry_roots: Vec<std::path::PathBuf> = dirs_cargo_registry_src();
    let deps_dir = format!("{dir}/.candor/deps");
    let _ = std::fs::create_dir_all(&deps_dir);
    let (mut scanned, mut cached, mut missing) = (0usize, 0usize, Vec::new());
    let no_deps = DepIndex::default();
    for (n, v) in &pkgs {
        let Some(src) = registry_roots.iter().map(|r| r.join(format!("{n}-{v}"))).find(|p| p.is_dir()) else {
            missing.push(format!("{n}-{v}"));
            continue;
        };
        // One subdirectory PER name@version: two locked versions of one crate must not overwrite
        // each other's report (review: last-write-wins silently fed the root the wrong version's
        // effects); with both present, conflicting keys drop as ambiguous — never-guess intact.
        let sub = format!("{deps_dir}/{n}@{v}");
        let already = std::fs::read_dir(&sub).ok().is_some_and(|rd| {
            rd.flatten().any(|e| {
                let f = e.file_name();
                let f = f.to_string_lossy();
                f.ends_with(".scan.json") && !f.contains("callgraph")
            })
        });
        if already {
            cached += 1; // registry checkouts are immutable per name@version — the report stands
            continue;
        }
        let _ = std::fs::create_dir_all(&sub);
        // Dep scans are quiet, unchained, report-only, and GATE-FREE (the resolved root policy and
        // baseline are deliberately not passed): their job is the report files. A registry dep is a
        // single published package, so scan_one (not scan_target) is right; the json body is unused.
        let _ = scan_one(&src.to_string_lossy(), ScanOpts {
            prefix: format!("{sub}/report"),
            want_json: false,
            include_tests: false,
            policy: None,
            baseline: None,
            quiet: true,
            deps_idx: &no_deps,
        });
        scanned += 1;
    }
    eprintln!(
        "candor-scan: --deps scanned {scanned} of {} registry dependencies into {deps_dir}{}{} \
(floor-engine reports: a dep's silent misses pass through — the κ caveat applies to the chain too)",
        pkgs.len(),
        if cached > 0 { format!(" ({cached} already scanned — cached)") } else { String::new() },
        if missing.is_empty() {
            String::new()
        } else {
            format!(" ({} without a local checkout: {}{})", missing.len(),
                missing.iter().take(5).cloned().collect::<Vec<_>>().join(", "),
                if missing.len() > 5 { ", …" } else { "" })
        }
    );
    // Chain the fresh dep reports (plus anything CANDOR_DEPS already names) under the root scan.
    // load_dep_reports dedups canonical paths, so deps_dir appearing in CANDOR_DEPS too is safe.
    let spec = match std::env::var("CANDOR_DEPS") {
        Ok(extra) if !extra.is_empty() => format!("{deps_dir}:{extra}"),
        _ => deps_dir.clone(),
    };
    let idx = load_dep_reports(Some(&spec));
    // The final root scan goes through scan_target so `--deps <workspace>` fans out over members
    // too — the nested-package filter would otherwise prune them all into an empty, gate-passing report.
    scan_target(dir, prefix, want_json, include_tests, policy, baseline, &idx)
}
