//! The scan driver: argument walk (`scan_main`), per-crate scan + report emission
//! (`scan_one`), workspace fan-out (`scan_target`), and `--deps` chaining (`run_with_deps`).

use crate::*;

/// Total CALLS into κ-uncovered dependencies at or above which the scan is assumed to be MISSING AN
/// INPUT — pointed at the crate's own code with nothing standing in for what it depends on — and so
/// earns the scan-completeness nudge under the κ ledger (see the emission site in `scan_one`).
///
/// VOLUME, not dependency COUNT, and that choice is load-bearing: count is the wrong metric in BOTH
/// directions. candor-java's own `build/classes` makes 519 uncovered calls into just 4 packages — the
/// textbook "you pointed it at the classes, not the deployed artifact" scan — which any count-based
/// threshold misses entirely, while a small crate touching 5 tiny util deps would be nudged for
/// nothing. A scan whose dependencies ARE supplied (`--deps` / CANDOR_DEPS chaining) sits at or near
/// zero. Held at candor-java's 50 so both engines nudge on the same evidence.
///
/// ADVISORY ONLY: one stderr line. It never touches the report, the gate verdict, or the exit code.
const UNCOVERED_CALLS_NUDGE_MIN: usize = 50;

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
                // The family house-style help: identity line, model paragraph, USAGE, OPTIONS,
                // ENVIRONMENT/CONFIG, EXAMPLES, footer. tests/cli.rs pins `USAGE` + exit 0.
                println!("candor-scan — the Rust effect scanner that reads a crate without building it.");
                println!();
                println!("Stable toolchain, purely syntactic: parses the source with syn — no nightly, no");
                println!("rustc-dev, no compile. Emits the same report every family engine speaks, through");
                println!("the shared classifier, and it deliberately under-reports rather than fabricates:");
                println!("Unknown marks only boundaries it can see (callbacks, FFI, untrusted chained reports).");
                println!();
                println!("USAGE");
                println!("  candor-scan [<dir>] [options]        scan a crate (default: .). A [workspace] root");
                println!("                                       scans every member — one report per member under");
                println!("                                       the one prefix; a nested dir with its own");
                println!("                                       Cargo.toml is a different package and is never");
                println!("                                       folded into the parent's report");
                println!("  candor-scan --agents                 print the agent contract embedded in this build");
                println!("  candor-scan --version | --help");
                println!();
                println!("OPTIONS");
                println!("  --out <prefix>       report path prefix (default: <dir>/.candor/report);");
                println!("                       writes <prefix>.<crate>.scan.json + a call-graph sidecar");
                println!("  --json               print the report to stdout instead of writing files");
                println!("  --include-tests      also scan tests/ benches/ examples/ and #[cfg(test)] modules");
                println!("                       (off by default — the report describes the crate, not its harness)");
                println!("  --incremental        reuse a per-file parse/decl cache under <dir>/.candor/cache so an");
                println!("                       edit-then-rescan skips re-parsing unchanged files (~7x on a one-file");
                println!("                       edit). Produces a BYTE-IDENTICAL report to a full scan; a candor-scan");
                println!("                       upgrade or a decl-changing edit invalidates the cache automatically");
                println!("  --deps               scan the Cargo.lock dependency tree first (registry sources from");
                println!("                       ~/.cargo/registry/src) into <dir>/.candor/deps/, then scan <dir>");
                println!("                       CHAINED over those reports — effects cross every crate boundary");
                println!("                       without the classifier needing to know the crates");
                println!("  --policy <file>      enforce a CANDOR_POLICY file (deny/pure/allow/forbid rules);");
                println!("                       exit 1 on a violation");
                println!("  --gate-json <f|->    write the structured gate verdict {{ spec, ok, violations }} as JSON");
                println!("                       over this scan; `-` streams it to stdout; exit 1 on violation.");
                println!("                       ADVISORY FLOOR: the syntactic backend under-reports, so a miss can");
                println!("                       pass — the nightly engine is the sound gate");
                println!("  -V, --version        print the installed build and its contract version (offline), and");
                println!("                       the upgrade line");
                println!("  -h, --help           print this help");
                println!();
                println!("ENVIRONMENT / CONFIG");
                println!("  CANDOR_POLICY=<f>    the policy file when --policy is absent; the checked-in floor");
                println!("                       under both is .candor/config (keys this engine reads: policy,");
                println!("                       baseline, deps — discovered from the scan target; a $CANDOR_CONFIG");
                println!("                       path overrides discovery)");
                println!("  CANDOR_BASELINE=<p>  the effect-regression guard (rule AS-EFF-005): compare this scan");
                println!("                       against a saved report (a path or --out prefix; also the `baseline`");
                println!("                       config key). A fn GAINING an effect vs the baseline exits 1; absent");
                println!("                       baseline → note, guard inactive; unparseable or produced by a");
                println!("                       DIFFERENT build → exit 2 — never a stale compare.");
                println!("                       Record one: candor-scan <dir> --out <prefix>");
                println!("  CANDOR_DEPS=<p:…>    chain sibling reports (files or directories of *.json): an");
                println!("                       unclassified call into a crate a report covers inherits that");
                println!("                       function's effects + literal surfaces. Scan the dep once, chain it");
                println!("                       everywhere; the coverage ledger names what to scan next.");
                println!("                       TWO KINDS OF REPORT GRANT NO COVERAGE, so a key they do not");
                println!("                       answer discloses instead of reading pure: one produced by a");
                println!("                       DIFFERENT engine build (§2.1 — its entries are also downgraded");
                println!("                       to Unknown) and one that declares itself INCOMPLETE (a non-empty");
                println!("                       ⟨0.21⟩ `unanalyzed` — its entries are KEPT unchanged, since they");
                println!("                       came from source it did read; only its silence hedges)");
                println!();
                println!("EXAMPLES");
                println!("  candor-scan .");
                println!("  candor-scan . --policy candor.policy --gate-json verdict.json");
                println!("  candor-scan . --deps");
                println!("  candor-scan crates/mycrate --json");
                println!();
                println!("Docs: candor.poly.io   ·   Verify an install: candor doctor");
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
    // ⟨unknown-ratchet⟩ OPT-IN on the AS-EFF-005 guard (config `unknown-ratchet` / CANDOR_UNKNOWN_RATCHET,
    // default OFF): when ON, a NEWLY-introduced Unknown vs the baseline FAILS instead of staying advisory —
    // making `deny Unknown` adoptable on legacy code. Resolved once here (env presence wins, else the config
    // truthy value) and read by every check_baseline via the UNKNOWN_RATCHET global — see check_baseline.
    let _ = crate::gate::UNKNOWN_RATCHET.set(crate::config::flag(&cfg, "unknown-ratchet", "CANDOR_UNKNOWN_RATCHET"));
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

/// ⟨typeSurface.returns⟩ THE PRODUCER. `{crate}#{fn qual}` -> `{crate}#{type qual}`, both FULLY
/// QUALIFIED, for the factory functions whose returned type has at least one non-pure member here.
///
/// Three things this has to get right, each one a confirmed defect of the reverted attempt:
///
/// 1. **The type id is FULLY QUALIFIED, and the report's own hashes are the authority for it.** The
///    attempt published `{crate}#{leaf}`, so `sync::Client` and `mock::Client` were one string and a
///    PURE `mock_client()` factory let `sync::Client::send`'s Net be charged to a caller that cannot
///    reach it. Here the id is the entry hash's PREFIX (`cr#conn::Pool::acquire` -> `cr#conn::Pool`),
///    which is exactly the shape the consumer appends `::<method>` to and asks `by_key` for.
/// 2. **The MODULE is not the type.** An earlier bug took the segment right after `#`, which on any
///    modular crate is the module — invisible on a flat fixture, and it made the rung near-inert. The
///    owning type is the segment BEFORE the method; a lowercase one is a MODULE holding a free fn
///    (`cr#util::helper`), never a type, and is skipped.
/// 3. **A leaf resolving to more than one non-pure type is REFUSED, not picked.** That is defect 1's
///    exact shape and the only place a wrong answer here could fabricate.
///
/// BOUNDED to types with a non-pure member: if the returned type has no effectful and no
/// `Unknown`-carrying member in this report, typing the receiver changes no answer — the lookup it
/// enables succeeds and yields pure, which is what the consumer's silence already yields.
fn build_type_surface(
    crate_name: &str,
    fns: &[FnInfo],
    entries: &[ReportEntry],
) -> candor_report::TypeSurface {
    // The FULL quals of types carrying a non-pure member, straight off the report's own hashes.
    let mut nonpure: BTreeSet<&str> = BTreeSet::new();
    for e in entries {
        if e.inferred.is_empty() {
            continue;
        }
        let Some((_, rest)) = e.hash.split_once('#') else { continue };
        let Some((ty_qual, _method)) = rest.rsplit_once("::") else { continue };
        let leaf = ty_qual.rsplit("::").next().unwrap_or(ty_qual);
        // A TYPE, not a module. `cr#util::helper` is a free fn in module `util`; taking `util` as the
        // owning type is the keying mismatch that made the whole rung inert once before, one layer up.
        if !leaf.chars().next().is_some_and(char::is_uppercase) {
            continue;
        }
        nonpure.insert(ty_qual);
    }
    let mut map: std::collections::BTreeMap<String, String> = std::collections::BTreeMap::new();
    let mut near_misses = 0usize;
    let mut collided: std::collections::HashSet<String> = std::collections::HashSet::new();
    for f in fns {
        let Some(tp) = &f.ret_bound_type else { continue };
        // EXACT match on the full qual, and that is the whole disambiguation. Both sides are now
        // module-qualified in this crate's own namespace — `ret_bound_type` resolves a bare name
        // against its DECLARING module — so `mock::Client` and `sync::Client` are different strings
        // and neither can stand in for the other. A suffix/leaf match here is defect 1.
        if !nonpure.contains(tp.as_str()) {
            if nonpure.iter().any(|q| q.rsplit("::").next() == tp.rsplit("::").next()) {
                near_misses += 1; // same type LEAF, different module — the case that MUST not resolve
            }
            continue;
        }
        let key = format!("{crate_name}#{}", f.qual);
        let val = format!("{crate_name}#{tp}");
        // Two FnInfos under one qual (a `#[cfg]`-duplicated impl) publishing DIFFERENT types: drop the
        // key rather than let walk order decide, the same never-guess rule `by_key` applies.
        if collided.contains(&key) {
            continue;
        }
        match map.get(&key) {
            Some(prev) if *prev != val => {
                map.remove(&key);
                collided.insert(key);
            }
            Some(_) => {}
            None => {
                map.insert(key, val);
            }
        }
    }
    // The COUNTS are the diagnostic, not the output — a bound that admits nothing on a real modular
    // crate is a keying bug, and it is invisible in a diff (standing bar item 8).
    if std::env::var("CANDOR_TYPESURFACE_DEBUG").is_ok() {
        let bound: usize = fns.iter().filter(|f| f.ret_bound_type.is_some()).count();
        eprintln!(
            "TYPESURFACE {crate_name}: fns={} bound_returns={bound} nonpure_type_quals={} \
             published={} leaf_near_misses={near_misses}",
            fns.len(), nonpure.len(), map.len()
        );
    }
    candor_report::TypeSurface { returns: map }
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
    // `mut` because a file can also fail LATER, in the pass-B walk: a parser abort is contained per file
    // (see the `catch_unwind` below) and must reach this flag, or a configured gate would evaluate over a
    // file whose effects are simply absent and call it clean — the false-pure this flag exists to prevent.
    let mut had_parse_failure = !unparsed.is_empty();
    // ⟨Gap 2⟩ Also carry the unparsed set into the REPORT (owned, so it survives `per_file`'s consumption):
    // the stderr warning above is invisible to a machine reading `--json`, so a bare report looked complete.
    // The gated path still exits 2 with no verdict (SPEC §3.3.1); this is the bare-report disclosure.
    let mut unanalyzed_units: Vec<candor_report::UnanalyzedUnit> = unparsed
        .iter()
        .map(|p| candor_report::UnanalyzedUnit { path: p.to_string(), reason: "source failed to read/parse".into() })
        .collect();

    // Per-file Pass A decls (cache or fresh) + a place to hold a parsed file for Pass B. A file dropped
    // by a read/parse failure (no cache AND round-1 parse failed) is excluded entirely, preserving the
    // original survivor set + walk order.
    let mut decls_per_file: Vec<(String, String, FileDecls)> = Vec::new(); // (rel, content_hash, decls)
    let mut parsed_files: HashMap<String, syn::File> = HashMap::new();     // rel -> parsed (round 1)
    let mut parsed_locs: HashMap<String, Vec<String>> = HashMap::new();    // rel -> per-fn loc (walk order)
    // rel -> (decl_index_hash, fninfos) for entries whose FnInfos are REUSABLE. An entry carrying
    // `FileCache::aborted` never lands here — see the `aborted` arm below.
    let mut cached_fninfos: HashMap<String, (String, Vec<FnInfo>)> = HashMap::new();
    // Files whose on-disk entry was already valid for BOTH content + the decl index it recorded — no
    // re-write needed unless the merged index moves (checked after the digest). Lets a no-op / body-only
    // re-scan skip rewriting the whole cache dir (the dominant cost when nothing changed).
    let mut disk_decl_hash: HashMap<String, String> = HashMap::new();
    for ((rel, ch, cached), r1) in per_file.into_iter().zip(round1) {
        match cached {
            Some(fc) => {
                // The DECLS are reusable either way: they were derived from a successful parse of these
                // exact bytes, before the walk that may have aborted. The FnInfos are conditionally
                // reusable (the decl-index check, after the digest) — unless the entry is POISONED.
                decls_per_file.push((rel.clone(), ch, fc.decls));
                if fc.aborted.is_some() {
                    // A CACHED ABORT IS A MARKER THAT THESE FNINFOS WERE NEVER DERIVED, NOT AN ANSWER TO
                    // REPLAY. `39bbc8b` persisted the abort so a warm run could not read `fninfos: []`
                    // as "analysed, no functions" (the false all-clear); it then replayed the disclosure
                    // off a content-hash + decl-index match, on the argument that those two are exactly
                    // the conditions under which reusing the FnInfos is sound. That argument does not
                    // hold, because the abort is not a function of either. `4f7b704` established the
                    // mechanism: proc-macro2's fallback `Span` indexes a THREAD-LOCAL source map, and
                    // whether a moved span lands past the walking thread's map depends on how much each
                    // rayon worker happened to parse — i.e. on the rest of the crate and on the work
                    // split, not on this file's bytes. A one-off therefore LATCHED, and `--incremental`
                    // is exactly the mode nobody re-runs from cold: a permanent spurious `unanalyzed`
                    // and a gate that can never go green (the mirror of the sin, but still a cached
                    // wrong answer).
                    //
                    // So: keep neither the FnInfos nor the `disk_decl_hash` shortcut. Every downstream
                    // gate reads `cached_fninfos`, so dropping the entry HERE is the single decision —
                    // the round-2 re-parse picks the file up (its `cached_fninfos` miss reads as stale),
                    // pass B walks it, and it either aborts again and discloses by the same cold path,
                    // byte for byte, or it produces the answer it always owed and the marker clears.
                    continue;
                }
                disk_decl_hash.insert(rel.clone(), fc.decl_index_hash.clone());
                cached_fninfos.insert(rel, (fc.decl_index_hash, fc.fninfos));
            }
            None => {
                // A freshly-parsed file (or a parse failure → skip the file entirely, as before).
                let Some((sf, locs)) = r1 else { continue };
                let fd = file_decls(&sf.0.items, include_tests, &module_path(Path::new(&rel)));
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
    let field_elem_trait = &merged.field_elem_trait;
    let trait_impls = &merged.trait_impls;
    let trait_decls = &merged.trait_decls;
    let trait_fields = &merged.trait_fields;
    let traits = TraitIndexes { impls: trait_impls, decls: trait_decls, fields: trait_fields };
    let elems = ElemIndexes { field_elem, field_elem_trait, enum_variants: &enum_variants };
    let lazy_statics = &merged.lazy_statics;
    let const_strings = &merged.const_strings;
    let local_macros = &merged.local_macros;

    // ROUND 2 PARSE (parallel): files whose decls were cached but whose FnInfos are STALE (the merged
    // decl index moved) — exactly the files a decl-changing edit invalidates. On a body-only edit this
    // set is empty; on a decl edit it is "everything else", re-parsed in parallel (degrade-to-full).
    // A file whose cached entry carried an ABORT has no `cached_fninfos` row at all (dropped above), so
    // `unwrap_or(true)` puts it here: the re-attempt is a plain stale-FnInfos re-parse, no special case.
    let need_passb: Vec<&str> = decls_per_file
        .iter()
        .map(|(rel, _, _)| rel.as_str())
        .filter(|rel| {
            !parsed_files.contains_key(*rel)
                && cached_fninfos.get(*rel).map(|(h, ..)| h != &decl_index_hash).unwrap_or(true)
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
    // rel -> the abort disclosure this run recorded for it, carried into the cache write-back.
    let mut fresh_aborted: HashMap<String, String> = HashMap::new();
    // Files that must NOT be cached this run: their FnInfos are UNKNOWN (not empty), so persisting
    // anything for them would publish a number nobody derived. Dropping the entry makes the next run
    // re-attempt from scratch, which is the fail-closed direction.
    let mut no_cache: std::collections::HashSet<String> = std::collections::HashSet::new();
    for (rel, _, _) in &decls_per_file {
        // Reuse only a set that was DERIVED. An entry whose walk aborted never reaches `cached_fninfos`
        // (see the `aborted` arm above), so there is nothing here that could replay a disclosure: a file
        // that aborted last run is re-walked, and earns its disclosure again or loses it.
        let reuse = cached_fninfos.get(rel).filter(|(h, _)| *h == decl_index_hash).map(|(_, v)| v.clone());
        if let Some(v) = reuse {
            fns.extend(v.iter().cloned());
            continue;
        }
        // Re-derive: the file needs its parse. Reaching the `else` means its DECLS came from cache (so
        // it was never in round 1) but the round-2 re-read/re-parse failed — the file changed or became
        // unreadable mid-scan. (Round 2 covers two entrances: the decl index moved, or the entry carried
        // an abort and its FnInfos were refused. The reason below names neither, because from here they
        // are the same event and only one of them is true.) It contributes no FnInfos, which under §2
        // rule 3 is a purity claim over every function in it, so disclose it exactly like a round-1
        // failure and refuse to cache anything for it.
        let Some(file) = parsed_files.get(rel) else {
            eprintln!("candor-scan: {rel} failed to re-read/parse — effects in it are NOT in this report");
            had_parse_failure = true;
            unanalyzed_units.push(candor_report::UnanalyzedUnit {
                path: rel.clone(),
                reason: "source failed to re-read/parse during re-derivation".into(),
            });
            no_cache.insert(rel.clone());
            continue;
        };
        let modpath = module_path(Path::new(rel));
        // Locs were resolved on the parse worker (spans are dead on this thread); reuse them positionally.
        let locs = parsed_locs.get(rel).map(Vec::as_slice).unwrap_or(&[]);
        let mut loc_idx = 0usize;
        // Seed the crate-ROOT re-exports under `crate::<name>` (and `crate::` + GLOB_KEY for the root glob)
        // so a `use crate::net` / `crate::net::foo` in THIS file resolves through the root re-export it can't
        // otherwise see (each file starts with a fresh `use` map). Crate-rooted only: a bare `net::foo` never
        // looks up a `crate::…` key, so a genuine external-crate call is never hijacked (see `expand`).
        let mut uses = seed_root_reexports(&merged.root_reexports);
        let mut file_fns: Vec<FnInfo> = Vec::new();
        // A PANIC IN ONE FILE MUST NOT TAKE THE RUN DOWN, and must not vanish either. `syn`/`proc-macro2`
        // can abort on input candor does not control — `getrandom` 0.3.4/0.4.2 hit proc-macro2's
        // `unreachable!("Invalid span with no related FileInfo!")` deterministically — and an unwind here
        // killed the WHOLE `--deps` tree, not the one crate. A chained consumer then proceeded with fewer
        // dependency reports than it asked for, which is exactly the blind spot the κ ledger exists to
        // name. So: contain it at the FILE, and DISCLOSE that file as unanalyzed — the ⟨0.21⟩ channel that
        // already carries "this file failed to parse", and that already makes a configured gate refuse to
        // go green over it. The panic message still reaches stderr through the default hook.
        let walked = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            // FAULT INJECTION, so the containment has a deterministic test. The real trigger needs a whole
            // crate's parse state and does not reduce to a fixture, and a containment nobody can fire is a
            // containment nobody has checked — the same reason the syscall oracle is calibrated with a
            // seeded violation rather than trusted because it reported none.
            if std::env::var("CANDOR_PANIC_ON_FILE").is_ok_and(|v| v == *rel) {
                panic!("CANDOR_PANIC_ON_FILE: injected fault while walking {rel}");
            }
            let mut out: Vec<FnInfo> = Vec::new();
            let mut idx = loc_idx;
            let mut u = uses.clone();
            scan_items(&file.items, &modpath, locs, &mut idx, include_tests, fields, &returns, traits, elems, lazy_statics, const_strings, local_macros, &mut u, &mut out);
            (out, idx, u)
        }));
        match walked {
            Ok((out, idx, u)) => {
                file_fns = out;
                loc_idx = idx;
                uses = u;
            }
            Err(_) => {
                const ABORTED: &str = "the parser aborted while walking this file";
                eprintln!("candor-scan: {rel} could not be analyzed (the parser aborted); disclosed as unanalyzed");
                had_parse_failure = true;   // a configured gate must not certify a scan with a hole in it
                unanalyzed_units.push(candor_report::UnanalyzedUnit {
                    path: rel.clone(),
                    reason: ABORTED.into(),
                });
                // …and the disclosure rides into the CACHE. Without this the entry written below is
                // `{ real content hash, current decl index, fninfos: [] }` — indistinguishable from a
                // file that genuinely has no functions — so the next `--incremental` run over identical
                // bytes reuses it, `continue`s before the walk, and discloses nothing. The panic was
                // fail-closed; the cache turned it into a reproducible false all-clear.
                fresh_aborted.insert(rel.clone(), ABORTED.to_string());
            }
        }
        let _ = loc_idx;
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
                if no_cache.contains(rel) {
                    continue; // nothing was derived for it — see the `no_cache` note above
                }
                let fninfos = fresh_fninfos
                    .get(rel)
                    .cloned()
                    .or_else(|| cached_fninfos.get(rel).map(|(_, v)| v.clone()))
                    .unwrap_or_default();
                // The marker travels with the FnInfos it belongs to, and it can only come from THIS run:
                // a file that aborted last run was re-walked above, so either it aborted again (and is in
                // `fresh_aborted`) or it produced a real set and the marker is gone. That is what keeps
                // an abort from surviving more than the one run that observed it — the cache can record
                // "these FnInfos were never derived", but it can never answer with a stale abort.
                let aborted = fresh_aborted.get(rel).cloned();
                files.insert(
                    rel.clone(),
                    FileCache {
                        content_hash: ch.clone(),
                        decls: fd.clone(),
                        decl_index_hash: decl_index_hash.clone(),
                        fninfos,
                        aborted,
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
    // dep crate root -> count of FLOORED call sites into it. Floored only: the tally has to mean the
    // same thing as the crate name beside it — calls whose effects this scan could not see.
    let mut dep_seen: HashMap<String, usize> = HashMap::new();
    // fn -> the dep crates it DIRECTLY calls into where the classifier floored the call. Post-filtered to
    // the genuinely-blind crates (not calibrated, no sibling report) + propagated transitively → the
    // per-fn `invisible` honesty disclosure (the κ ledger, but attributed per function).
    let mut blind_direct: HashMap<String, BTreeSet<String>> = HashMap::new();
    // Blind crates inherited from a dep fn's `invisible` (sweep [8]): genuinely blind (the dep confirmed
    // it), but a TRANSITIVE crate the consumer never saw directly, so it is absent from `dep_seen` and
    // would be dropped by the `global_blind` filter. Collected here and unioned into global_blind below.
    let mut dep_invisible: BTreeSet<String> = BTreeSet::new();
    // Callers whose `Unknown` came across the chain join with no reason the dependency recorded. See
    // `DepSink::unknown_via_dep` and the §4 invariant in the report writer.
    let mut unknown_via_dep: BTreeSet<String> = BTreeSet::new();

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
    // `String`, not `&'static str`: a chained dependency's own `unknownWhy` is carried VERBATIM across the
    // join (`dispatch:<owner>.<member>` has a normative detail that re-deriving would destroy), so the set
    // can no longer be only compile-time literals.
    let mut unknown_why: HashMap<String, BTreeSet<String>> = HashMap::new();
    // TRANSITIVE DROP-OWNER closure (#3, FIELD edition — R49): constructing a struct `T` also runs the drop
    // glue of any LOCAL drop-type `T` OWNS through a field — directly (`_g: Guard`), via a collection element
    // (`_v: Vec<Guard>`, carried by `field_elem`), or transitively (`_s: Session` where `Session` owns a
    // Guard). The per-call drop detection charges only a local OF the drop-type itself; a guard held as a
    // FIELD dropped silent-pure. `owned_drops[T]` = the local drop-types edged when a `T` is constructed.
    // Gated to LOCAL drop-types reached through LOCAL field types, so an external field's invisible Drop is
    // never fabricated. Computed to a fixpoint (monotone; a struct owning itself via `Box` terminates).
    let owned_drops: HashMap<String, BTreeSet<String>> = if merged.drop_types.is_empty() {
        HashMap::new()
    } else {
        // A field type's LEAF (`type_path` may qualify it — `inner: ffi::Deflate` — but `owned_drops`,
        // `drop_types`, and the struct keys are all LEAF-keyed, so compare by leaf or the transitive
        // owner chain breaks across modules).
        let candidates = |t: &str| -> Vec<String> {
            let leaf = |ty: &String| ty.rsplit("::").next().unwrap_or(ty).to_string();
            let mut v: Vec<String> = Vec::new();
            if let Some(m) = fields.get(t) {
                v.extend(m.values().map(&leaf));
            }
            if let Some(m) = field_elem.get(t) {
                v.extend(m.values().map(&leaf));
            }
            v
        };
        let all_types: BTreeSet<String> = fields.keys().chain(field_elem.keys()).cloned().collect();
        let mut owned: HashMap<String, BTreeSet<String>> = HashMap::new();
        for t in &all_types {
            let s: BTreeSet<String> = candidates(t)
                .into_iter()
                .filter(|c| merged.drop_types.contains(c))
                .collect();
            if !s.is_empty() {
                owned.insert(t.clone(), s);
            }
        }
        loop {
            let mut changed = false;
            for t in &all_types {
                let mut add: BTreeSet<String> = BTreeSet::new();
                for c in candidates(t) {
                    if &c != t {
                        if let Some(inner) = owned.get(&c) {
                            add.extend(inner.iter().cloned());
                        }
                    }
                }
                if !add.is_empty() {
                    let e = owned.entry(t.clone()).or_default();
                    for d in add {
                        if e.insert(d) {
                            changed = true;
                        }
                    }
                }
            }
            if !changed {
                break;
            }
        }
        owned
    };
    for f in &fns {
        loc.entry(f.qual.clone()).or_insert_with(|| f.loc.clone());
        // The body invoked a callable the scan can't see through (closure / fn-pointer value): it could
        // perform any effect, so record an honest `Unknown` (propagated like any effect, surfaced in the
        // receipt's unresolved count) instead of silently certifying the function pure.
        if f.unresolved {
            direct.entry(f.qual.clone()).or_default().insert("Unknown");
            unknown_why.entry(f.qual.clone()).or_default().insert("callback:unresolved call".to_string());
        }
        // DROP-GLUE (#3): local types this fn CONSTRUCTS that have a local `impl Drop`. The `Drop::drop`
        // body runs at scope exit — an implicit edge the call graph misses, so a guard that flushes/closes
        // on drop read silent-pure. Collected per-call below (a `T::*` associated-fn call where `T` has a
        // local Drop), then edged to `T::drop` after the call loop. Over-approximates toward the SOUND
        // direction (a constructed value is assumed to drop in this scope); gated to LOCAL drop types only,
        // so an external type's invisible Drop is never fabricated.
        let mut drops_here: BTreeSet<String> = BTreeSet::new();
        // ESCAPE GATE (R49): a drop-type built here whose value can ESCAPE via the return must NOT be charged
        // — its `Drop` runs in the CALLER's scope (a constructor `Compress::new -> Compress` that builds an
        // owned `Stream` and returns the `Compress` otherwise FABRICATES the Stream's FFI-Drop; the flate2
        // miss that A/B-reverted the naive field fix). CONSERVATIVE test: the fn RETURNS a local aggregate (a
        // struct in `fields`) or a drop-type — so a value built here may be moved into what's returned. A
        // membership check (not ownership traversal), so it stays SOUND under the leaf-name COLLISIONS that
        // flate2's parallel read/write/bufread modules create (three `GzEncoder`s etc.) — precise ownership
        // can't be trusted there. It over-skips (a fn returning an aggregate won't get drop-glue even for a
        // genuinely-local guard), the SOUND direction — never a fabrication. A `-> ()`/`Result`/primitive fn
        // (the local-use case: `let _g = Guard{..}; …`) is NOT an escape, so it still charges.
        let returns_escapable = f
            .ret_idents
            .iter()
            .any(|r| fields.contains_key(r) || merged.drop_types.contains(r));
        for c in &f.calls {
            let cr = c.path.split("::").next().unwrap_or("");
            // A DEP-LAZY MARKER (`<crate>::<lazy>::NAME`, emitted for a qualified mention of what may be a
            // dependency's lazy static) is consumed by the cross-crate join BELOW and by nothing else: it
            // is a speculative name, so letting it reach local resolution or the classifier would invent
            // edges for every qualified path in the file. Join it directly and move on; a crate the deps
            // index does not cover, or a dep with no such unit, resolves to nothing and costs nothing.
            // Cross-crate DROP GLUE: `cr::<drop>::Type` — binding a dependency's value runs its `Drop` at
            // scope exit. Joined here and nowhere else, for the same reason as the lazy marker below.
            if let Some(ty) = c.path.strip_prefix(&format!("{cr}::{DROP_MARKER}::")) {
                let cr_real: &str = dep_renames.get(cr).map(String::as_str).unwrap_or(cr);
                if deps_idx.crates.contains(cr_real) {
                    if let Some(de) = deps_idx.by_key.get(&format!("{cr_real}#{ty}::drop")) {
                        apply_dep_fn(de, &f.qual, DepSink {
                            direct: &mut direct, hosts: &mut hosts, cmds: &mut cmds, paths: &mut paths,
                            tables: &mut tables, incomplete: &mut incomplete, unknown_why: &mut unknown_why,
                            blind_direct: &mut blind_direct, dep_invisible: &mut dep_invisible,
                            unknown_via_dep: &mut unknown_via_dep,
                        });
                    }
                }
                continue;
            }
            // COULD-NOT-FORM-A-KEY: `cr::<untyped>::method`. The receiver came from `cr` but we never
            // learned its type, so no lookup happened — and the dep report's silence is only an answer to
            // a question that was asked. Disclose `Unknown` with the existing `dispatch:` reason class
            // (SPEC §2: "a project abstraction with no visible impl"), which is what this is from the
            // consumer's side. Gated on the crate being a DECLARED dependency, so the marker is inert for
            // a local module that merely looks crate-qualified.
            //
            // Deliberately NOT `invisible`: the κ ledger correctly excludes a crate that HAS a chained
            // sibling report, which is exactly this case — so `invisible` would be filtered away and the
            // disclosure lost. That filtering is right for keyed-and-missed and wrong here; the two need
            // different spellings. See DEP-RECEIVER-TYPING-DESIGN.md.
            if let Some(rest) = c.path.strip_prefix(&format!("{cr}::{UNTYPED_RECV_MARKER}::")) {
                let cr_real: &str = dep_renames.get(cr).map(String::as_str).unwrap_or(cr);
                // THIRD conjunct, and it is what makes this precise rather than noisy: the dep must be
                // CHAINED. For an UNCHAINED dep the κ ledger already discloses `invisible: [cr]`, so the
                // reader is warned and a second disclosure buys nothing but false uncertainty — measured,
                // that arm fired on `let finds = dep::best_finds(); finds.first()`, where the value came
                // from a dep but `.first()` is a std Vec method. Only when the dep IS chained does the
                // ledger fall silent (covered, correctly, per §2 rule 3) — and that silence is the
                // confident purity claim this exists to prevent.
                if deps_idx.crates.contains(cr_real) {
                    // ⟨typeSurface.returns⟩ DETERMINATION BEFORE DISCLOSURE (half 2). `rest` is
                    // `<callee path>::<method>`; the method leaf is one segment and the callee path is
                    // not, so it splits from the RIGHT. If the dependency PUBLISHED what that factory
                    // returns, the receiver's type is recoverable and the real key can be formed after
                    // all — a FULL qual on both ends, answered by the full-qual key the dep index now
                    // carries.
                    let split = rest.rsplit_once("::");
                    let hit = split.and_then(|(callee, method)| {
                        let ty = deps_idx.returns.get(&format!("{cr_real}#{callee}"))?;
                        deps_idx.by_key.get(&format!("{ty}::{method}"))
                    });
                    // Instrument the PRECONDITION, not just the output: a diff cannot show that a
                    // mechanism never fired or fired on the wrong thing (standing bar item 8).
                    if std::env::var("CANDOR_TYPESURFACE_DEBUG").is_ok() {
                        let (callee, method) = split.unwrap_or((rest, ""));
                        let ty = deps_idx.returns.get(&format!("{cr_real}#{callee}"));
                        eprintln!(
                            "TYPESURFACE-{} {} :: {cr_real}#{callee} -> {} :: .{method}()",
                            if hit.is_some() { "HIT " } else { "MISS" },
                            f.qual,
                            ty.map(String::as_str).unwrap_or("<no returns entry>")
                        );
                    }
                    if let Some(de) = hit {
                        apply_dep_fn(de, &f.qual, DepSink {
                            direct: &mut direct, hosts: &mut hosts, cmds: &mut cmds, paths: &mut paths,
                            tables: &mut tables, incomplete: &mut incomplete, unknown_why: &mut unknown_why,
                            blind_direct: &mut blind_direct, dep_invisible: &mut dep_invisible,
                            unknown_via_dep: &mut unknown_via_dep,
                        });
                        continue;
                    }
                    // A MISS — on `returns` or on `by_key` — FALLS THROUGH TO THE DISCLOSURE, never to
                    // silence. This is defect 3 of the reverted attempt, which read a `by_key` miss after
                    // a `returns` hit as a keyed-and-missed and stayed quiet. `by_key` deliberately DROPS
                    // ambiguous keys ("never guess"), so a miss cannot distinguish "no such method" from
                    // "I withdrew an entry" — and that is not hypothetical: measured over three crates'
                    // dep trees, adding the full-qual key alone withdrew 1865 keys on pgman as
                    // full-qual-vs-full-qual collisions. A refusal to answer is not a purity claim.
                    direct.entry(f.qual.clone()).or_default().insert("Unknown");
                    // A COARSE token, like the `callback:` one above — the reason set is `&'static str`,
                    // and interpolating the crate/method here would mean leaking a string per call site.
                    unknown_why
                        .entry(f.qual.clone())
                        .or_default()
                        .insert("dispatch:untyped cross-package receiver".to_string());
                }
                continue;
            }
            if let Some(rest) = c.path.strip_prefix(&format!("{cr}::{LAZY_UNIT_PREFIX}::")) {
                let cr_real: &str = dep_renames.get(cr).map(String::as_str).unwrap_or(cr);
                if deps_idx.crates.contains(cr_real) {
                    if let Some(de) = deps_idx.by_key.get(&format!("{cr_real}#{LAZY_UNIT_PREFIX}::{rest}")) {
                        apply_dep_fn(de, &f.qual, DepSink {
                            direct: &mut direct, hosts: &mut hosts, cmds: &mut cmds, paths: &mut paths,
                            tables: &mut tables, incomplete: &mut incomplete, unknown_why: &mut unknown_why,
                            blind_direct: &mut blind_direct, dep_invisible: &mut dep_invisible,
                            unknown_via_dep: &mut unknown_via_dep,
                        });
                    }
                }
                continue;
            }
            // SPEC §1 ⟨0.13⟩ `Llm` model-SDK surface (candor_classify::MODEL_SDK_CRATES): a qualified call
            // into a curated model-provider client → `Llm` + `Net` (Net is never dropped — a model call IS
            // network I/O). No method gating (single-purpose clients), the analog of java's isModelSdkOwner.
            let model_sdk = c.path.contains("::") && candor_classify::is_model_sdk_crate(cr);
            let classified = candor_classify::classify(cr, &c.path)
                .or_else(|| scan_builder_entry_effect(cr, &c.path))
                .or(if model_sdk { Some("Llm") } else { None });
            // DROP-GLUE detection: a `T::assoc()` ASSOCIATED-FN call (a CONSTRUCTOR like `Guard::new`)
            // where `T` is a LOCAL drop type means a `T` value is CREATED in this scope and dropped at exit.
            // Record `T` so we edge to `T::drop` after the loop. CRUCIALLY gated to `!c.method`: a METHOD
            // call (`reg.poll()`, recorded typed as `Registration::poll`) operates on a BORROW and does NOT
            // own/drop the value here — including those over-connected every borrow-site to the drop body
            // (tokio: 170 fns). A constructor is `Type::fn(..)` syntax (an associated fn, `method=false`).
            // Excludes `T::drop` itself.
            if !merged.drop_types.is_empty() && !c.method && c.path.contains("::") && c.leaf != "drop" {
                if let Some(ty) = tail2(&c.path).and_then(|t2| t2.split("::").next().map(str::to_string)) {
                    // Direct: a local of the drop-type itself (UNCHANGED — the shipped behavior).
                    if merged.drop_types.contains(&ty) {
                        drops_here.insert(ty.clone());
                    }
                    // FIELD edition (R49): constructing `ty` also runs the drop glue of any local drop-type
                    // it transitively OWNS through a field — ADDITIVE, and charged only when the value can't
                    // escape via the return (the conservative gate above), so it never fabricates.
                    if !returns_escapable {
                        if let Some(owned) = owned_drops.get(&ty) {
                            drops_here.extend(owned.iter().cloned());
                        }
                    }
                }
            }
            // κ ledger: a qualified call into a declared dependency. (A bare leaf has no `::`, so it
            // can't name a crate; a local module sharing a dep's name is the rare accepted ambiguity.)
            if c.path.contains("::") && deps.contains(cr) {
                // A FLOORED dep call is a candidate per-fn blind spot (filtered to genuinely-blind
                // below). A CLASSIFIED one is not — its effect is on the record — so it is counted in
                // NEITHER the ledger nor the call tally. Coverage is a REVIEW claim, not a resolution
                // outcome: κ matching `zip::ZipArchive::new` vouches for THAT call, never for the crate,
                // so a single classified call must not clear the blind marker for every other call shape
                // into it. The vouching mechanisms are the CALIBRATED_* lists and a chained sibling
                // report — both of which someone reviewed.
                if classified.is_none() {
                    *dep_seen.entry(cr.to_string()).or_insert(0) += 1;
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
                        // TRAIT-REQUIREMENT dispatch from a trait DEFAULT body: inside `Store::save_all`,
                        // `self` types as the TRAIT `Store` (decls.rs types Self as the trait), so
                        // `self.persist()` is `Store::persist` — a REQUIREMENT with no default body, hence no
                        // `Store::persist` unit for `resolve_target` to find, and `type_to_traits` keys on IMPL
                        // types not the trait. CHA `persist` over Store's IMPLS and edge to each impl's method:
                        // the bounded-CHA analog of the swift protocol-extension→conformer-witness dispatch (R32's
                        // rust sibling — the effectful `impl Store for Db { fn persist }` was reachable ONLY
                        // through the default and read silent-pure). Gated to a LOCAL TRAIT that declares `leaf`
                        // (a struct-named receiver never hijacks) and bounded ≤12 impls (a wider open-world
                        // fan-out is an honest miss, never a guess). Only fires when nothing resolved locally.
                        if !resolved_local
                            && trait_decls.get(&t_type).is_some_and(|lt| lt.methods.contains(&c.leaf))
                        {
                            if let Some(impls) = trait_impls.get(&t_type) {
                                let mut hits: Vec<String> = Vec::new();
                                for imp in impls {
                                    let imp_leaf = imp.rsplit("::").next().unwrap_or(imp);
                                    if let Some(ts) = by_tail2.get(&format!("{imp_leaf}::{}", c.leaf)) {
                                        for t in ts {
                                            if !hits.contains(t) {
                                                hits.push(t.clone());
                                            }
                                        }
                                    }
                                }
                                if !hits.is_empty() && hits.len() <= 12 {
                                    resolved_local = true;
                                    for t in &hits {
                                        if t != &f.qual {
                                            calls.entry(f.qual.clone()).or_default().insert(t.clone());
                                        }
                                    }
                                }
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
            // BLANKET-impl fallback (R45): a `x.leaf()` that resolved to NO concrete / trait-default / deref
            // target may be a blanket-impl method (`impl<T> Ext for T { fn ext }`), whose body qual is
            // `<param>::leaf` (`T::ext`) — a keyed lookup on the receiver's type can't find it. If `leaf` is a
            // UNIQUE blanket method (not the "" ambiguity sentinel), edge to the blanket body. SOUND: the
            // blanket provides `leaf` for EVERY type (a bounded blanket only for conforming types — and a
            // call that COMPILES meets the bound), and it fires only when nothing concrete resolved. Gated to
            // a TYPED receiver (a known LOCAL type): the type's OWN `leaf` (a local FnInfo) would resolve
            // first, so the blanket never overrides an inherent method — no fabrication. A bare/untyped
            // receiver is left an honest under-report (candor can't confirm the blanket applies vs a shadow).
            // CRUCIAL — also suppress the blanket when the receiver TYPE declares `leaf` AMBIGUOUSLY: if
            // `by_tail2["{recv}::{leaf}"]` exists but has >1 target, `resolve_target` returned None (its
            // uniqueness filter) and `resolved_local` is false, yet the type DOES have an inherent `leaf`
            // that shadows the blanket — edging to the blanket would FABRICATE its effect onto a call that
            // runs the (unresolvably-ambiguous) inherent method (review [2]). Fire only when the receiver
            // type has NO local `leaf` at all.
            if !resolved_local && c.method && c.typed && !c.is_macro {
                let recv_has_inherent = tail2(&c.path)
                    .and_then(|t2| t2.split("::").next().map(str::to_string))
                    .is_some_and(|ty| by_tail2.contains_key(&format!("{ty}::{}", c.leaf)));
                if !recv_has_inherent {
                    if let Some(bty) = merged.blanket_methods.get(&c.leaf) {
                        if !bty.is_empty() {
                            if let Some(ts) = by_tail2.get(&format!("{bty}::{}", c.leaf)) {
                                if ts.len() == 1 && ts[0] != f.qual {
                                    resolved_local = true;
                                    calls.entry(f.qual.clone()).or_default().insert(ts[0].clone());
                                }
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
                let key = if rel.contains("::") {
                    tail2(rel).map(|t2| format!("{cr_real}#{t2}"))
                } else {
                    Some(format!("{cr_real}#{rel}"))
                };
                let hit = key.as_ref().and_then(|k| deps_idx.by_key.get(k));
                if let Some(de) = hit {
                    dep_join_hit = true;
                    apply_dep_fn(de, &f.qual, DepSink {
                        direct: &mut direct, hosts: &mut hosts, cmds: &mut cmds, paths: &mut paths,
                        tables: &mut tables, incomplete: &mut incomplete, unknown_why: &mut unknown_why,
                        blind_direct: &mut blind_direct, dep_invisible: &mut dep_invisible,
                            unknown_via_dep: &mut unknown_via_dep,
                    });
                    // (No coverage marking here. A crate whose sibling report we joined is already
                    // covered by the `deps_idx.crates` arm of the ledger filter below — that arm is the
                    // reviewed claim, and it holds whether or not any single join happened to fire.)
                }
            }
            if let Some(eff) = classified.filter(|_| !suppress_bare_leaf && !resolved_local) {
                direct.entry(f.qual.clone()).or_default().insert(eff);
                // §1 ⟨0.13⟩ model-SDK dispatch is `Llm` + `Net`, added UNCONDITIONALLY — a model-SDK
                // crate call IS a model dispatch AND is network I/O, regardless of what `classified`
                // carried. Insert BOTH: a model-SDK crate whose call ALSO resolves via `classify` to
                // `Net` (e.g. `aws_sdk_bedrockruntime::…::send`) short-circuits the `.or(Some("Llm"))`
                // fallback, so `eff == "Net"` and the `Llm` would be DROPPED (a gate evasion — the
                // model surface silently vanishes behind the plain Net) unless we add it here. Matches
                // the deep engine (src/lib.rs) and candor-java, which both add Llm+Net unconditionally.
                if model_sdk && !suppress_bare_leaf && !resolved_local {
                    let d = direct.entry(f.qual.clone()).or_default();
                    d.insert("Llm");
                    d.insert("Net");
                }
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
                        // `Net` and `Llm` share the host surface: `Llm` ⟨0.13⟩ rides the Net host literal
                        // (`allow Llm <host>` certifies against the SAME captured host), so a model-SDK
                        // call reaching `Llm` via the `.or(Some("Llm"))` fallback (classify returned None)
                        // with an endpoint literal MUST capture that host — else `allow Llm <host>` has no
                        // literal to certify and fails closed. Identical capture logic to `Net`.
                        "Net" | "Llm" => {
                            let h = host_part(s);
                            // A DOTLESS host is NOT a certifiable Net allowlist literal — java/ts/swift
                            // `hostLiteral` REJECT dotless hosts (`localhost:8080` is a bare label, not a
                            // routable name), so rust must NOT capture one into the host surface either
                            // (a cross-engine divergence — rust over-captured `localhost:8080` where the
                            // siblings dropped it). The ONE exception is the port-based Ollama refinement:
                            // a dotless `:11434` still adds `Llm` (the model signal is the PORT, not the
                            // host name) but is likewise never captured. So:
                            //   • dotless `:11434`  → add `Llm`, no host capture (Ollama, matches java);
                            //   • dotless otherwise → nothing captured (Net effect stays; no literal — a
                            //     dotless host has no gate-certifiable surface, matching the siblings);
                            //   • dotted host       → model-host refinement + capture (rides `allow Net`
                            //     / `allow Llm <host>`), incl. `127.0.0.1:11434` which IS dotted.
                            if h.contains('.') {
                                // §1 ⟨0.13⟩ host-literal refinement: a known model host adds `Llm` (Net kept).
                                if candor_classify::is_model_host(&h) {
                                    direct.entry(f.qual.clone()).or_default().insert("Llm");
                                }
                                hosts.entry(f.qual.clone()).or_default().insert(h);
                            } else if h.rsplit_once(':').is_some_and(|(_, p)| p == "11434") {
                                // §1 ⟨0.13⟩ Ollama dotless-host (`localhost:11434`): refine to `Llm` WITHOUT
                                // capturing the bare host (`allow Llm localhost` has no literal to certify —
                                // fails closed; `deny Llm` still catches it). Matches java's dotless branch.
                                direct.entry(f.qual.clone()).or_default().insert("Llm");
                            }
                            // else: a plain dotless host (`localhost:8080`) — Net effect already recorded;
                            // no host captured (matches sibling `hostLiteral`; `allow Net` can't certify it).
                        }
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
                unknown_why.entry(f.qual.clone()).or_default().insert("native:extern fn".to_string()); // FFI is a native boundary — canonical `native:` (SPEC §4 ⟨0.7⟩)
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
            //
            // ── WHY THE KIND IS `ambiguous:` AND STAYS THAT WAY (measured, 2026-07-27) ────────────────
            // ⟨0.24⟩ `ambiguous:` IS NOW THE FIFTH KIND IN SPEC §4's closed vocabulary. It was outside it
            // when this engine started emitting it, and the argument below is the one that got it in —
            // kept, because it is the record of what the kind buys. NONE OF THE OTHER FOUR CAN EXPRESS
            // THIS STATE:
            //   • NOT `dispatch:` — that kind is reserved for unresolved member dispatch WITH a resolvable
            //     owner type, and its detail is NORMATIVE `<owner>.<member>`. A bare free call has no owner,
            //     so the detail cannot be formed, and PART 10 rejects a dot-free `dispatch:` outright. It is
            //     also not dispatch in the first place: exactly ONE function runs and Rust resolves it
            //     statically — what failed is this ANALYSER's name resolution, not the program's.
            //   • NOT `callback:` — an unresolved HIGHER-ORDER invocation over a function VALUE. This is a
            //     named call. And `callback:` is not the residual bucket: §6.2 reaches the residual by the
            //     ABSENCE of a reason, and `f2309a5` is the record of what reaching for it costs.
            //   • NOT `native:` / `reflect:` — no foreign boundary, no metadata-driven invocation.
            // SPEC §6.2's reason-class table had ALWAYS named `ambiguous*` and ruled its class `dispatch`,
            // so the spec blessed the prefix in one section and omitted it from the closed set in another
            // — an asymmetry that survives because a CONSUMER never complains about a token it can
            // classify, only a PRODUCER is non-conforming. ⟨0.24⟩ closed it in §4's favour of the
            // producer. THIS ENGINE HOLDS THE VOCABULARY ONCE: as the raw `kind:detail` string it emits,
            // read back only through `ReasonClass::classify`'s prefix table. There is no second, typed
            // kind enum here to drift out of step with that table — which is the failure the ⟨0.24⟩
            // "AN ENGINE HOLDS THIS VOCABULARY TWICE" paragraph records against the JVM engine, where a
            // correct string classifier concealed a typed `Kind` enum that lacked the kind entirely. If a
            // typed kind ever lands in this engine, `ambiguous` goes in it at the same commit.
            //
            // AND THE RENAME IS NOT FREE — the counterfactual was BUILT and RUN, not argued. With
            // `ambiguous*` reclassified to `indirect` (one line in `ReasonClass::classify`, both binaries
            // kept by content hash), `deny E Unknown[dispatch]` goes from firing on **58 of 200 crates.io
            // crates to 0 of 200**, and from exit 1 to exit 0 on pgman, ebman and candor-rust alike. That
            // is not the narrowing candor-ts measured for its malformed `dispatch:` strings (`5ba301c`,
            // where every reclassified reason named NOTHING); it deletes the rule in this engine, because
            // every OTHER `dispatch:` rust emits — 20 in a 1062-report census, all
            // `dispatch:untyped cross-package receiver` — requires a chained dependency to exist at all.
            // Pinned by `the_ambiguous_reason_kind_and_its_class_are_pinned`; conformance PART 10 now
            // scans a purpose-built fixture so the kind is VISIBLE there instead of silently absent.
            // That measurement is also what §4 ⟨0.24⟩ cites for admitting the kind rather than deleting
            // it, so the number and the vocabulary now stand or fall together.
            if !c.is_macro && !c.method && classified.is_none() && !resolved_local && suppress_bare_leaf
                && !c.path.contains("::")
                && by_leaf.get(&c.leaf).is_some_and(|v| v.len() >= 2)
            {
                direct.entry(f.qual.clone()).or_default().insert("Unknown");
                unknown_why.entry(f.qual.clone()).or_default().insert("ambiguous:same-name local defs".to_string());
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
    // Reason-scoped Unknown (REASON-SCOPED-UNKNOWN-DESIGN.md): the Unknown reason CLASS must travel the
    // call graph the same way the Unknown EFFECT does, so `deny E Unknown[reflect]` at a caller inheriting
    // Unknown from a reflect-caused callee still fires. Classify each fn's DIRECT unknown_why tokens to
    // class tokens, then propagate transitively (mirrors the java gate's reasonClassAcc). Report unchanged.
    let mut reason_class_direct: HashMap<String, BTreeSet<String>> = unknown_why
        .iter()
        .map(|(f, whys)| {
            let classes = whys
                .iter()
                .map(|w| candor_classify::policy::ReasonClass::classify(w).token().to_string())
                .collect();
            (f.clone(), classes)
        })
        .collect();
    // …AND THE REASONLESS CHAINED `Unknown` CONTRIBUTES ITS CLASS HERE, rather than being inferred from
    // the ABSENCE of one. `f2309a5` correctly stopped inventing a `callback:` reason for a dep that
    // declared `Unknown` and recorded no reason — §6.2 already answers that case ("a function whose
    // `Unknown` carries no recorded reason is treated as `unresolved`") and §4's closed kind vocabulary
    // has no member that projects there. But it left `unknown_via_dep` feeding only the writer's
    // `debug_assert`, so the class reached the gate ONLY through the fallback below, which is per
    // FUNCTION and fires when the whole class set is absent or empty. **Any other reason on the same
    // function swallowed it**: `both() { dep::murky(); dep::mute(); }` — one dep Unknown carrying
    // `dispatch:` and one carrying nothing — classified `dispatch` alone, and
    // `deny E Unknown[unresolved]` went from exit 1 to exit 0 as the SECOND call was added. So the
    // by-absence fallback worked everywhere except where two reasons meet on one function, which is
    // where a gate needs it. Adding a reason must never REMOVE a class.
    //
    // NO TOKEN IS INVENTED: this writes a §6.2 CLASS (`unresolved`, a member of that section's own
    // closed set), never a §4 kind, and it is confined to this gate-side map — `unknown_why`, and with
    // it the report, is untouched, so the ⟨0.7⟩ field still carries only reasons somebody observed.
    // The RESIDUAL that leaves is the format's: a report cannot say "Unknown, no reason" alongside a
    // reason it does have, so a SECOND-hop consumer chaining this report re-derives `dispatch` alone.
    // That needs a §4/§6.2 rung (see the work queue) and is not patchable with a string here.
    //
    // ⟨0.24⟩ THE RESIDUAL IS NOW MEASURED, AND IT IS A LIVE GATE-vs-DISCLOSURE SPLIT — the exact thing
    // §6.2 forbids ("THE GATE AND THE DISCLOSURE MUST APPLY THE SAME RULE"). pgman over a 270-report dep
    // tree, every report marked §2.1 stale: 36 functions reach this contribution and **12 of them also
    // carry a reason of their own**, which grows to **18 of the 77** functions whose gate-side class set
    // includes `unresolved`. Recomputing that set from the REPORT — which is what `unverified --class`
    // and `blindspots --class` do — recovers only 59: the 18 come back classed `dispatch` alone, so the
    // gate bites them and the disclosure standing beside it does not name them. With TRUSTED reports the
    // split is 0, on both targets measured; it is reachable only through reports the build cannot verify.
    //
    // WHAT WOULD CLOSE IT is exactly what §4 ⟨0.24⟩ registered for the purpose — `dep-stale:<pkg>` (and
    // `dep:<hash>`), permanent kinds now, attached per dependency ENTRY the way candor-swift attaches
    // them, so the reason travels IN the report and the two recomputations cannot disagree. It is not
    // taken here for one reason and it is not a spec one: the SHIPPED conformance PART 10 still holds a
    // four-kind `CANON` plus two named migration kinds, and `dep-stale` is in neither, so emitting it
    // scores a hard DIVERGE today. The rung landed in SPEC and not yet in the harness that checks it.
    for q in &unknown_via_dep {
        reason_class_direct
            .entry(q.clone())
            .or_default()
            .insert(candor_classify::policy::ReasonClass::Unresolved.token().to_string());
    }
    let reason_class_acc = propagate_str(&reason_class_direct, &calls, &all);
    // The genuinely-blind dep crates (the per-scan κ "unlisted" set): seen, never classified, not
    // dep-report-covered, not calibrated. A fn's `invisible` = its transitive blind reach ∩ this set.
    // ⟨0.15 staged⟩ Computed ONCE, with the call counts, as the κ-coverage LEDGER: the same list (same
    // names, same counts, same order) feeds the envelope's `coverage` field (spec §2), the stderr
    // disclosure below, and the --gate-json advisory note — three surfaces that can never disagree
    // because they share this one computation. Sorted by call count (desc) then name, the stderr
    // line's long-standing order. (A crate with a loaded sibling report is COVERED even when no join
    // fired: the report omits pure functions, so join-less calls are its honest purity claim — the
    // opposite of invisible. A RENAMED dep is covered under its real package name.)
    // This filter deliberately does NOT consult a "was ever classified" set. That was an UNVOUCHED
    // proxy standing in front of the reviewed ones: one classified call cleared the blind marker for
    // every other call shape into the same crate, so adding an unrelated call elsewhere in the program
    // silently converted a disclosed blind spot into a purity claim. `blind_direct` already holds the
    // per-call-site datum and it already propagates, so the per-fn truth was present all along.
    // AN UNTRUSTED REPORT MUST NOT GRANT COVERAGE. §2.1 downgrades a stale report's EFFECTS to
    // `Unknown` — and the exemption below would, on the authority of that same refused report, turn
    // every function it does not mention into a confident purity claim (§2 rule 3 makes an absent entry
    // one) with the `invisible` disclosure dropped. The join still fires (it is keyed on `crates`, so
    // the entries that ARE there still charge `Unknown`); only the claim that the report's SILENCE is
    // informative is withdrawn. candor-ts `651c9f9` is the same defect one repo over.
    //
    // ⟨0.21⟩ NEITHER DOES A REPORT THAT DECLARES ITSELF INCOMPLETE — the same door with a different key,
    // read one step earlier: staleness asks whether to believe what a report SAYS, completeness asks
    // whether its SILENCE means anything, and a report naming source it could not analyze has answered
    // that. Its ENTRIES are untouched (they came from source it did read), so the change is strictly
    // additive: an answered key still answers, an unanswered one falls back to the hedge. See
    // `DepIndex::incomplete_pkgs`. THIS IS THE ONLY PLACE COVERAGE IS CONSUMED, so one conjunct here is
    // the whole gate. candor-java's sibling needed two, because coverage was anchored twice there and
    // gating one was a no-op wearing a fix's clothes; the anchor count is a per-engine fact and this one
    // is 1. NOT "read nowhere else" — `dbab8be` said that and it is wrong: `load_dep_reports` reads both
    // sets again for its two stderr disclosures. CONSUMED nowhere else is the claim, and it is no longer
    // just a claim: `coverage_has_exactly_one_anchor_and_exactly_one_consumer` enumerates the writes and
    // the consumers out of the source and fails on a second of either (four mutants, four named rows).
    //
    // ⟨0.24⟩ AND NEITHER DOES A REPORT THAT JUDGED NOTHING — `analyzed.count: 0`, the THIRD answer to
    // "may this report's silence speak?" and the last one the wire can currently express. Same treatment
    // as incompleteness (chained, not covered, entries untouched), and the same single conjunct, because
    // this ledger is still the only place coverage is consumed. See `DepIndex::judged_nothing_pkgs`.
    let mut coverage_ledger: Vec<(String, usize)> = dep_seen
        .iter()
        .filter(|(cr, _)| {
            let real = dep_renames.get(cr.as_str()).map(String::as_str).unwrap_or(cr.as_str());
            // COVERED = a report exists for this crate AND the staleness gate did not refuse it AND the
            // report did not declare itself incomplete AND it judged at least one unit. The second,
            // third and fourth conjuncts are the three fixes; spelled as a named local so clippy's
            // boolean simplification can't rewrite the claims into one unreadable disjunction.
            let covered = deps_idx.crates.contains(real)
                && !deps_idx.untrusted.contains(real)
                && !deps_idx.incomplete_pkgs.contains(real)
                && !deps_idx.judged_nothing_pkgs.contains(real);
            !covered
                && !candor_classify::CALIBRATED_CRATES.contains(&cr.as_str())
                && !candor_classify::PATH_CALIBRATED_CRATES.contains(&cr.as_str())
                && !candor_classify::CALIBRATED_PREFIXES.iter().any(|p| cr.starts_with(p))
        })
        .map(|(cr, n)| (cr.clone(), *n))
        .collect();
    coverage_ledger.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
    let global_blind: std::collections::HashSet<String> =
        coverage_ledger.iter().map(|(cr, _)| cr.clone()).collect();
    // A dep-inherited invisible crate is genuinely blind (the dep's own scan confirmed it) but transitive,
    // so it never appears in `dep_seen` — keep it so the consumer's `invisible` survives the filter ([8]).
    let global_blind: std::collections::HashSet<String> =
        global_blind.into_iter().chain(dep_invisible).collect();

    // ⟨0.20⟩ Net destination-class (NET-DESTINATION-CLASS-DESIGN.md): the config `net-partner` hosts, read
    // here (before the entries) so the report's per-fn `netClass` carries known-partner — the SAME set the
    // gate resolves from `.candor/config`. Empty when no config declares partners (telemetry-only asserts).
    let net_partners = candor_classify::policy::discover_config_text(std::path::Path::new(dir))
        .map(|t| candor_classify::policy::parse_net_partners(&t))
        .unwrap_or_default();

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
        // THE MARKER MUST TRAVEL WITH THE THING IT DESCRIBES. SPEC §4: `unknownWhy` is REQUIRED when a fn
        // introduces `Unknown` DIRECTLY (a source) and absent when the `Unknown` is purely inherited — so
        // the invariant is exactly "`direct` carries Unknown ⇒ `unknownWhy` is non-empty", checked at the
        // one place every entry is built rather than argued site by site. It is a `debug_assert`: it holds
        // in every test and over 21 980 entries of real corpus, and a marker gap must never take down a
        // user's scan in release — the release-mode consequence of a gap is a tolerated Unknown, not a
        // crash. Reason: the chained-dep join used to write `Unknown` into `direct` with no reason at all,
        // and nothing in the writer objected.
        //
        // `unknown_via_dep` IS THE ONE EXEMPTION, and it is §4's own definition rather than a hole punched
        // in the rule. §4 defines a source as a unit "whose own body has the unresolvable call"; a
        // consumer of a chained dependency has no such call — its body calls a known function whose
        // REPORT says Unknown, so its Unknown is INHERITED, across a report boundary instead of a
        // call-graph edge. The join writes into `direct` only because the callee is not a unit in this
        // report, which is an implementation fact and not a claim about whose body holds the hole. The
        // invariant as written forced the boundary case to name one of the four §4 kinds, none of which
        // projects to `unresolved` (§6.2) — so honouring it produced a FABRICATED class, which is how
        // `callback:chained dependency declared Unknown without a reason` came to exist. Every in-scan
        // path is still held to the rule, which is the class of gap this assertion was written to catch.
        debug_assert!(
            !direct.get(q).is_some_and(|d| d.contains("Unknown"))
                || unknown_why.contains_key(q)
                || unknown_via_dep.contains(q),
            "`{q}` introduces Unknown DIRECTLY but carries no `unknownWhy` (SPEC §4). Its reason class is \
             lost, so a `deny E Unknown[class]` gate silently tolerates it — whichever new path put the \
             Unknown into `direct` must charge its reason beside it."
        );
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
            unknown_why: unknown_why.get(q).map(|s| s.iter().cloned().collect()).unwrap_or_default(),
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
            // ⟨0.20⟩ Net destination-class: the classes present in this fn's transitive Net surface. Exact
            // host-literal match for the visible hosts; fail-closed unknown-host when the Net surface is masked
            // (`incomplete` has Net) OR carries no visible host (a runtime endpoint). Empty when no Net.
            net_class: if inf.contains("Net") {
                crate::gate::net_classes_of(q, &hostsacc, &incompleteacc, &net_partners)
            } else {
                Vec::new()
            },
            interface_union: false,
        });
    }
    // ⟨workspace-chain, gated⟩ TRAIT-CHA union entries — the candor-ts/swift `interfaceUnion` analog. A
    // cross-crate consumer calling a trait method on a `&dyn Trait` whose Trait is imported from HERE keys
    // the chain lookup on `crate#Trait::method` (tail2), which has no body → no entry → the call reads pure.
    // Emit a synthetic entry = the UNION over local impls of that method's effects (inferred + invisible),
    // reusing `trait_impls`/`trait_decls` (the CHA universe in-crate dispatch already uses). Sound
    // over-approximation; a `Trait::method` a consumer never resolves is harmless. GATED so a default scan
    // stays byte-identical (four-way conformance unaffected until the rung is pinned).
    if std::env::var_os("CANDOR_WORKSPACE_CHAIN").is_some() {
        let existing: std::collections::HashSet<String> = entries.iter().map(|e| e.hash.clone()).collect();
        for (trait_leaf, lt) in merged.trait_decls.iter() {
            // AMBIGUOUS same-leaf traits (`mod a { trait T } mod b { trait T }`): `trait_decls`/`trait_impls`
            // are keyed by LEAF, so `lt` merges both traits' methods and `impls` merges both traits' impls —
            // a union entry over them could carry an UNRELATED trait's impl effect (a cross-crate fabrication).
            // The in-crate dispatch bails to Unknown here (collector.rs `lt.count > 1`); the emission must too:
            // skip the ambiguous leaf (an honest under-report, never a guess between traits).
            if lt.count > 1 {
                continue;
            }
            let impls = match merged.trait_impls.get(trait_leaf) {
                Some(v) => v,
                None => continue,
            };
            for method in &lt.methods {
                let mut inf_u: std::collections::BTreeSet<&'static str> = std::collections::BTreeSet::new();
                let mut blind_u: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
                for ty in impls {
                    let ty_leaf = ty.rsplit("::").next().unwrap_or(ty);
                    for cand in [format!("{ty}::{method}"), format!("{ty_leaf}::{method}")] {
                        if let Some(s) = inferred.get(&cand) {
                            for e in s {
                                inf_u.insert(e);
                            }
                        }
                        if let Some(s) = blind_acc.get(&cand) {
                            for c in s.iter().filter(|c| global_blind.contains(*c)) {
                                blind_u.insert(c.clone());
                            }
                        }
                    }
                }
                if inf_u.is_empty() && blind_u.is_empty() {
                    continue; // pure across all impls — silence = purity
                }
                let hash = format!("{crate_name}#{trait_leaf}::{method}");
                if existing.contains(&hash) {
                    continue; // a real entry already claims this hash
                }
                entries.push(ReportEntry {
                    func: format!("{trait_leaf}::{method}"),
                    inferred: inf_u.iter().map(|s| s.to_string()).collect(),
                    unresolved: inf_u.contains("Unknown"),
                    hash,
                    invisible: blind_u.into_iter().collect(),
                    interface_union: true,
                    ..Default::default()
                });
            }
        }
    }
    entries.sort_by(|a, b| a.func.cmp(&b.func));

    let meta = candor_report::ReportMeta {
        version: format!("scan-{}", env!("CARGO_PKG_VERSION")),
        toolchain: "stable".into(),
        spec: candor_report::SPEC_VERSION.into(),
    };
    // ⟨0.15 staged⟩ the `coverage` envelope field (spec §2): the κ ledger as data, so "what the scan
    // couldn't see" travels WITH the report instead of evaporating on stderr. Omitted when empty —
    // a fully-covered scan's report is byte-identical to a ⟨0.14⟩ one (wire-compatible rung).
    let coverage = (!coverage_ledger.is_empty()).then(|| candor_report::Coverage {
        uncovered: coverage_ledger
            .iter()
            .map(|(cr, n)| candor_report::CoverageEntry { name: cr.clone(), calls: *n })
            .collect(),
    });
    // ⟨0.21⟩ COMPLETENESS MANIFEST (Gap 1): the analyzed universe = every fn candor formed a judgment for =
    // `all` (the §2.2 callgraph node set, pure leaves included — NOT the effectful-only `entries`). count lets
    // a bare-envelope consumer compute the pure count (count − |functions|); digest fingerprints the set for
    // same-engine re-scan agreement. Recorded toward the --gate-json verdict too (analyzed:{count}).
    let analyzed = {
        let mut sorted = all.clone();
        sorted.sort();
        candor_report::Analyzed { count: sorted.len(), digest: candor_report::fnv1a_hex(&sorted) }
    };
    crate::gate::record_gate_analyzed(analyzed.count, &unanalyzed_units);
    // ⟨typeSurface.returns⟩ THE PRODUCER (DEP-RECEIVER-TYPING-DESIGN.md half 2). A consumer cannot type
    // `let c = deplib::build()` because `build` is PURE and therefore absent from this report entirely —
    // publishing its return type is the only way that key can ever be formed.
    let type_surface = build_type_surface(&crate_name, &fns, &entries);
    let body = candor_report::to_packaged_report_json_typed(
        &meta, &crate_name, &entries, coverage.as_ref(), &unanalyzed_units, Some(&analyzed),
        Some(&type_surface))
        .unwrap_or_default();
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
            let breakdown = ["Net", "Llm", "Fs", "Db", "Exec", "Ipc", "Env", "Clipboard", "Clock", "Log", "Rand"]
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
    // cared about). ⟨0.15 staged⟩ the list itself is `coverage_ledger`, computed once above and shared
    // with the report's `coverage` envelope field — this line and the JSON can never disagree.
    if !quiet {
        // ⟨0.15 staged⟩ ...and the same ledger rides the --gate-json verdict as an ADVISORY note
        // (spec §3.3 verb conditionality; verdict-preserving). Target/member scans only — quiet
        // dependency scans under --deps are not the gated surface, and their gaps are exactly what
        // chaining just closed. A no-op unless --gate-json was given.
        record_gate_coverage(&coverage_ledger);
    }
    if !coverage_ledger.is_empty() && !quiet {
        let shown: Vec<String> = coverage_ledger
            .iter()
            .take(8)
            .map(|(cr, n)| format!("{cr} ({n} call{})", if *n == 1 { "" } else { "s" }))
            .collect();
        let more =
            if coverage_ledger.len() > 8 { format!(" + {} more", coverage_ledger.len() - 8) } else { String::new() };
        eprintln!(
            "candor-scan: candor's classifier doesn't cover {} dependenc{} this code calls into — their effects are INVISIBLE to the scan (absent from the report, NOT a claim they're pure): {}{}",
            coverage_ledger.len(),
            if coverage_ledger.len() == 1 { "y" } else { "ies" },
            shown.join(", "),
            more
        );
        // SCAN-COMPLETENESS NUDGE. A high VOLUME of calls into uncovered dependencies is the signature
        // of a scan that is missing an INPUT rather than one whose classifier is imprecise: the code
        // leans hard on crates nothing in this run can see into, so what those crates do is simply
        // absent. Measured on a real 18.7k-fn webapp (the JVM engine, same threshold): scanned app-only
        // it could PROVE Net on 465 functions; re-scanned as the deployed artifact — the app AND its
        // 222 dependency jars — the same gate proved Net on 5,865. The reaches did not become more
        // precise, they became VISIBLE. Here the equivalent move is `--deps` (or chaining sibling
        // reports through CANDOR_DEPS), which is why the remedy names it.
        //
        // The nudge deliberately promises VISIBILITY ONLY — never dispatch resolution. On that same
        // webapp 23 of 26 unresolved dispatches were over the app's OWN broad hierarchies, which no
        // amount of extra dependency code can fix; promising otherwise would be a claim we can't keep.
        //
        // Keyed on the ledger's own per-dep call counts, so this line and the ledger above can never
        // disagree about the evidence. Rides inside the ledger's `!quiet` block: a dependency scan
        // under --deps is not the surface being reported on, and the user is already doing the thing.
        let uncovered_calls: usize = coverage_ledger.iter().map(|(_, n)| n).sum();
        if uncovered_calls >= UNCOVERED_CALLS_NUDGE_MIN {
            eprintln!(
                "candor-scan: hint — {} call{} go into {} dependenc{} that {} not scanned, so their effects are invisible here. If you scanned only your own crate, scan what it depends on too (`--deps` walks the Cargo.lock tree once and chains the reports; CANDOR_DEPS chains ones you already have): those reaches then resolve to DETERMINED effects instead of being absent.",
                uncovered_calls,
                if uncovered_calls == 1 { "" } else { "s" },
                coverage_ledger.len(),
                if coverage_ledger.len() == 1 { "y" } else { "ies" },
                if coverage_ledger.len() == 1 { "is" } else { "are" }
            );
        }
    }

    // The cold-repo hook: after the effect summary + κ ledger, surface the SINGLE most surprising
    // transitive reach + a ready-to-run `candor path` command — so the two-minute demo opener is
    // deterministic, not lucky. Pure call-graph + name analysis (no LLM); honest fallback when
    // nothing clears the bar. See surface.rs / SURFACE-BEST-FIND-DESIGN.md. STDERR only (stdout may
    // carry the JSON report), and AFTER the histogram + κ ledger in output order.
    // The surface opener is for EXPLORATION scans; under an active policy gate the headline is the VERDICT,
    // and a reassuring "nothing hidden" / "most surprising reach" line ABOVE the violation lines reads as a
    // contradiction in a CI log (#18). Suppress it when gating — the gate summary + fix-gate pointer carry.
    if !quiet && policy_path.is_none() {
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
        match check_baseline(bv, dir, &crate_name, &all, &inferred, crate::gate::unknown_ratchet()) {
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
        // ⟨0.19⟩ reason-class aliases (SPEC §6.2): a multi-value `unknown-alias` config key the single-value
        // cfg map can't hold — read straight from the discovered config so `Unknown[<alias>]` resolves.
        let unknown_aliases = candor_classify::policy::discover_config_text(std::path::Path::new(dir))
            .map(|t| candor_classify::policy::parse_unknown_aliases(&t))
            .unwrap_or_default();
        let v = policy_violations(&text, &all, &inferred, &calls, &hostsacc, &cmdsacc, &pathsacc, &tablesacc, &incompleteacc, &reason_class_acc, &unknown_aliases, &net_partners);
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
            // A CLEAN gate has no violation lines for the exploration opener to contradict, so emit it HERE
            // (after the ✓) — the pre-gate suppression at the top only exists to avoid a "nothing hidden"
            // line ABOVE a FAILING gate's violations (#18); a passing gated scan should not lose it (#8).
            if !quiet {
                crate::surface::emit(&inferred, &direct, &calls, &loc);
            }
        } else {
            eprintln!("candor-scan: {} policy violation(s) (advisory floor — a clean run is necessary, not sufficient)", v.len());
            // Append-only remedy pointer (gate-FAILURE path only): the summary line above is
            // conformance-pinned, so this extra line must never alter it, the violation lines,
            // or the exit code — and a zero-violation run stays byte-identical.
            eprintln!("→ candor-query fix-gate names the remedy for each (or `candor fix <fn> <Effect>` for one)");
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
    // Emit the interfaceUnion entries in the DEP scans. Those entries are what lets a consumer resolve a
    // call on a value typed by the dependency's trait — and the emission is gated on
    // CANDOR_WORKSPACE_CHAIN, which `--deps` never set, so the mechanism was shipped-but-off in exactly the
    // flow the help text tells people to use: a `&dyn DepTrait` dispatch read silent-pure
    // (SOUNDNESS-VEIN-crossing-the-scan-boundary.md, cause 3). candor-swift already sets it on its child
    // scans, which is the sole reason its equivalent case is recovered and rust's was not.
    // Scoped to this loop and restored afterwards: the child scans run sequentially on this thread, and the
    // ROOT scan's own emission must stay off so a default scan remains byte-identical.
    let prior_chain = std::env::var_os("CANDOR_WORKSPACE_CHAIN");
    if prior_chain.is_none() {
        std::env::set_var("CANDOR_WORKSPACE_CHAIN", "1");
    }
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
    if prior_chain.is_none() {
        std::env::remove_var("CANDOR_WORKSPACE_CHAIN");
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
