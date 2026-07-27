//! candor-query's unit tests — the former in-file `#[cfg(test)] mod tests` of main.rs
//! (`super::*` still resolves to the crate root; original indentation kept verbatim).

    use super::*;

    #[test]
    fn embedded_agents_contract_matches_the_repo_doc() {
        // The drift gate for `--agents`: the packaged copy (crates/candor-query/AGENTS.md — the
        // only file a crates.io tarball can carry) must equal the repo-root AGENTS.md.
        // If this fails: cp AGENTS.md crates/candor-query/AGENTS.md
        let embedded = include_str!("../AGENTS.md");
        // The root doc exists only in a workspace checkout; in a published-crate / `cargo vendor`
        // layout `../../AGENTS.md` is absent or unrelated, so `cargo test` on the shipped crate
        // would fail spuriously. Gate the comparison on the root doc being present AND candor's own.
        match std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/../../AGENTS.md")) {
            Ok(root) if root.contains("instructions for an AI coding agent") => {
                assert_eq!(embedded, root, "crate AGENTS.md drifted from the repo root — re-copy it");
            }
            _ => { /* registry/vendor layout — drift gate N/A; include_str proves the copy compiles */ }
        }
    }

    #[test]
    fn rewire_flags_dropped_edges_not_added_ones() {
        let cg = |pairs: &[(&str, &[&str])]| -> BTreeMap<String, Vec<String>> {
            pairs.iter().map(|(k, v)| (k.to_string(), v.iter().map(|s| s.to_string()).collect())).collect()
        };
        let base = cg(&[("api::handle", &["service::place_order"]), ("cart::total", &["pricing::quote"])]);
        // gamed: api::handle dropped its call into the pricing chain; cart::total unchanged; a NEW edge added.
        let cur = cg(&[("cart::total", &["pricing::quote"]), ("main", &["api::handle"])]);
        let d = dropped_edges(&cur, &base);
        assert_eq!(d.get("api::handle"), Some(&vec!["service::place_order"])); // the de-wiring is flagged
        assert!(!d.contains_key("cart::total")); // unchanged edge → not flagged
        assert!(!d.contains_key("main")); // a purely-ADDED edge is not a drop
        // a correct fix that only ADDS edges yields nothing dropped.
        assert!(dropped_edges(&base, &base).is_empty());
    }

    #[test]
    fn gains_flags_a_supply_chain_capability_gain() {
        // The supply-chain alarm core (spec §5.1): an effect PRESENT in the new surface and ABSENT
        // from the old is a gained capability — a dependency that grew a Net/Exec reach between
        // releases. Only growth alarms: an unchanged or shrunk surface gains nothing.
        let set = |xs: &[&str]| xs.iter().map(|s| s.to_string()).collect::<BTreeSet<String>>();
        assert_eq!(gained_effects(&set(&["Fs", "Net"]), &set(&["Fs"])), vec!["Net"]); // gained Net
        assert!(gained_effects(&set(&["Fs"]), &set(&["Fs"])).is_empty()); // stable → no alarm
        assert!(gained_effects(&set(&["Fs"]), &set(&["Fs", "Net"])).is_empty()); // a LOSS is not a gain
    }

    #[test]
    fn match_tier_ladder() {
        // exact > segment-suffix > substring; suffix requires a `::` boundary (the red-team find:
        // `Pricing::quote` must not widen to `quote_bulk`).
        assert_eq!(match_tier("pricing::Pricing::quote", "pricing::Pricing::quote"), 3);
        assert_eq!(match_tier("pricing::Pricing::quote", "Pricing::quote"), 2);
        assert_eq!(match_tier("pricing::Pricing::quote", "quote"), 2);
        assert_eq!(match_tier("pricing::Pricing::quote_bulk", "quote"), 1); // substring only
        assert_eq!(match_tier("pricing::Pricing::quote", "ricing::quote"), 1); // mid-segment ≠ suffix
        assert_eq!(match_tier("pricing::Pricing::quote", "zzz"), 0);
        // selection: with both present, best tier 2 excludes the substring cousin.
        let names = ["pricing::Pricing::quote", "pricing::Pricing::quote_bulk"];
        let t = best_tier(names.iter().copied(), "Pricing::quote");
        assert_eq!(t, 2);
        assert!(q_match(names[0], "Pricing::quote", t));
        assert!(!q_match(names[1], "Pricing::quote", t));
        // with NO suffix candidate, substring still works (browsing).
        let t2 = best_tier(names.iter().copied(), "ricing");
        assert_eq!(t2, 1);
        assert!(q_match(names[0], "ricing", t2) && q_match(names[1], "ricing", t2));
    }

    #[test]
    fn whatif_scope_and_policy_parse() {
        // whatif parses policy through the SHARED canonical parser (candor_classify::policy, SPEC §6.2),
        // so its pre-edit verdict can't diverge from the real gate. Spot-check that path here.
        use candor_classify::policy::{parse_policy, scope_matches};
        // scope match is segment-aware: last segment prefix, intermediates exact, never mid-word.
        assert!(scope_matches("app::domain::handle", "domain"));
        assert!(scope_matches("crate::domain_logic", "domain")); // segment-prefixed
        assert!(!scope_matches("app::subdomain::handle", "domain")); // mid-word must NOT match
        assert!(scope_matches("api::handle", "api"));

        // `deny Net Db api` -> forbid {Net,Db} in scope `api`; `pure parse` -> forbid ALL in `parse`;
        // `deny Exec` -> forbid Exec crate-wide (no scope). allow/forbid go to separate rule sets.
        let p = parse_policy("deny Net Db api\npure parse\ndeny Exec\nallow Net in billing x\nforbid a -> b\n# c");
        let rules = &p.rules;
        assert_eq!(rules.len(), 3);
        assert_eq!(rules[0].effects.iter().copied().collect::<Vec<_>>(), vec!["Db", "Net"]);
        assert_eq!(rules[0].scope.as_deref(), Some("api"));
        assert!(rules[1].effects.is_empty()); // pure = all effects
        assert_eq!(rules[1].scope.as_deref(), Some("parse"));
        assert_eq!(rules[2].scope, None); // crate-wide deny Exec
        // the allow/forbid lines landed in their own rule sets, not in `rules`.
        assert_eq!(p.allow_rules.len(), 1);
        assert_eq!(p.layer_rules.len(), 1);
    }

    #[test]
    fn containment_layer_derivation() {
        // The common root prefix is stripped; the next MODULE segment is the layer.
        let names: Vec<String> = ["pgman::conn::connect", "pgman::query::run", "pgman::main"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        let refs: Vec<&String> = names.iter().collect();
        let pl = common_prefix_len(&refs);
        assert_eq!(pl, 1, "shared root is `pgman`");
        assert_eq!(layer_of("pgman::conn::connect", pl), "conn");
        assert_eq!(layer_of("pgman::query::Q::run", pl), "query");
        // a free function at the crate root has no module → buckets into (root), not its own layer.
        assert_eq!(layer_of("pgman::main", pl), "(root)");
        // a UFCS trait-impl path normalizes to the impl Type's module, not the literal `<Type as …`.
        assert_eq!(norm_name("<pgman::conn::Conn as std::fmt::Debug>::fmt"), "pgman::conn::Conn");
        assert_eq!(layer_of("<pgman::conn::Conn as std::fmt::Debug>::fmt", pl), "conn");
        // multi-crate report: no shared first segment → the crate IS the layer.
        let multi: Vec<String> =
            ["a::x::f".to_string(), "b::y::g".to_string()].to_vec();
        let mrefs: Vec<&String> = multi.iter().collect();
        assert_eq!(common_prefix_len(&mrefs), 0);
        assert_eq!(layer_of("a::x::f", 0), "a");
    }

    #[test]
    fn is_scan_artifact_discriminates() {
        assert!(is_scan_artifact("report", "report.mycrate.scan.json"));
        assert!(is_scan_artifact("report", "report.mycrate.scan.callgraph.json"));
        // lint artifacts are NOT scan
        assert!(!is_scan_artifact("report", "report.mycrate.Rlib.json"));
        assert!(!is_scan_artifact("report", "report.mycrate.Executable.callgraph.json"));
        assert!(!is_scan_artifact("report", "report.calibrated.json"));
        // a different prefix is not ours
        assert!(!is_scan_artifact("report", "other.mycrate.scan.json"));
    }

    #[test]
    fn report_backend_and_clear_other() {
        let dir = std::env::temp_dir().join("candor-query-backend-test");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let pre = dir.join("report");
        let prefix = pre.to_string_lossy().to_string();
        let w = |name: &str| std::fs::write(dir.join(name), b"{}").unwrap();

        // none
        assert_eq!(report_backend(&prefix), "none");
        // a scan report
        w("report.c.scan.json");
        w("report.c.scan.callgraph.json");
        assert_eq!(report_backend(&prefix), "scan");
        // now a lint run lands too → both present; clear the scan side (keep lint)
        w("report.c.Rlib.json");
        w("report.c.Rlib.callgraph.json");
        w("report.calibrated.json");
        assert_eq!(report_backend(&prefix), "scan"); // scan still present
        let removed = clear_other_reports(&prefix, "lint"); // keep lint, drop scan
        assert_eq!(removed, 2); // report.c.scan.json + report.c.scan.callgraph.json
        assert!(!dir.join("report.c.scan.json").exists());
        assert!(dir.join("report.c.Rlib.json").exists());
        assert_eq!(report_backend(&prefix), "lint");
        // and the reverse: keep scan would drop the lint reports + calibrated sidecar
        w("report.c.scan.json");
        let removed = clear_other_reports(&prefix, "scan");
        assert!(removed >= 3); // Rlib.json + Rlib.callgraph.json + calibrated.json
        assert!(dir.join("report.c.scan.json").exists());
        assert_eq!(report_backend(&prefix), "scan");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn locate_picks_newest_by_mtime() {
        use std::time::{Duration, SystemTime};
        let dir = std::env::temp_dir().join("candor-query-locate-test");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        // two toolchain-suffixed dylibs; the alphabetically-FIRST is the OLDER (the stale-pick bug)
        let old = dir.join("libcandor@nightly-2025-01-01-x.dylib");
        let new = dir.join("libcandor@nightly-2026-01-01-x.dylib");
        std::fs::write(&old, b"x").unwrap();
        std::fs::write(&new, b"x").unwrap();
        // make `new` strictly newer
        let f_old = std::fs::OpenOptions::new().write(true).open(&old).unwrap();
        f_old.set_modified(SystemTime::now() - Duration::from_secs(100)).unwrap();
        let f_new = std::fs::OpenOptions::new().write(true).open(&new).unwrap();
        f_new.set_modified(SystemTime::now()).unwrap();
        let out = locate_newest("lib", &[dir.to_string_lossy().to_string()]);
        assert_eq!(out, Some(new)); // newest mtime, not the alphabetically-first (older) one
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// merge-hook must: add the hook to a fresh/empty file; PRESERVE the user's other settings; be
    /// idempotent; and — the critical safeguard — leave an unparseable file UNTOUCHED rather than
    /// clobber it. (The bug this guards against once wiped a user's permissions/model on re-install.)
    #[test]
    fn merge_hook_is_nondestructive_and_idempotent() {
        let dir = std::env::temp_dir().join("candor-query-merge-test");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let cmd = "X/stop-hook.sh".to_string();
        let arg = |p: &std::path::Path| vec![p.to_string_lossy().to_string(), cmd.clone()];

        // 1) fresh file → hook added, parseable.
        let fresh = dir.join("fresh.json");
        assert_eq!(cmd_merge_hook(&arg(&fresh)), 0);
        let v: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(&fresh).unwrap()).unwrap();
        assert_eq!(v["hooks"]["Stop"][0]["hooks"][0]["command"], cmd.as_str());

        // 2) existing unrelated settings preserved.
        let keep = dir.join("keep.json");
        std::fs::write(&keep, r#"{"model":"opus","permissions":{"allow":["Bash"]}}"#).unwrap();
        assert_eq!(cmd_merge_hook(&arg(&keep)), 0);
        let v: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(&keep).unwrap()).unwrap();
        assert_eq!(v["model"], "opus");
        assert_eq!(v["permissions"]["allow"][0], "Bash");
        assert_eq!(v["hooks"]["Stop"][0]["hooks"][0]["command"], cmd.as_str());

        // 3) idempotent — a second merge doesn't duplicate the hook.
        assert_eq!(cmd_merge_hook(&arg(&keep)), 0);
        let v: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(&keep).unwrap()).unwrap();
        assert_eq!(v["hooks"]["Stop"].as_array().unwrap().len(), 1);

        // 4) unparseable (comments/trailing comma) → LEFT UNTOUCHED.
        let bad = dir.join("bad.json");
        let original = "{ // comment\n  \"model\": \"x\",\n}";
        std::fs::write(&bad, original).unwrap();
        assert_eq!(cmd_merge_hook(&arg(&bad)), 0);
        assert_eq!(std::fs::read_to_string(&bad).unwrap(), original, "must not touch a non-JSON file");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// `collect_rs` + the FNV digest must be deterministic, sensitive to `.rs` content, and blind to
    /// `target/` and `.git/` — the contract the ~10 shell sites used to re-implement (inconsistently).
    #[test]
    fn state_hash_is_deterministic_and_scoped() {
        let dir = std::env::temp_dir().join("candor-query-state-test");
        let _ = std::fs::remove_dir_all(&dir);
        for d in ["src", "sub", "target/x", ".git"] {
            std::fs::create_dir_all(dir.join(d)).unwrap();
        }
        std::fs::write(dir.join("src/a.rs"), "fn a(){}").unwrap();
        std::fs::write(dir.join("sub/b.rs"), "fn b(){}").unwrap();
        std::fs::write(dir.join("target/x/c.rs"), "fn c(){}").unwrap(); // must be ignored
        std::fs::write(dir.join(".git/d.rs"), "fn d(){}").unwrap(); // must be ignored
        std::fs::write(dir.join("src/notrust.txt"), "ignored").unwrap();

        let hash = |root: &Path| -> u64 {
            let mut files = Vec::new();
            collect_rs(root, &mut files);
            files.sort();
            let mut h: u64 = 0xcbf2_9ce4_8422_2325;
            for f in &files {
                for &b in f.strip_prefix(root).unwrap_or(f).to_string_lossy().as_bytes() {
                    h ^= b as u64;
                    h = h.wrapping_mul(0x0000_0100_0000_01b3);
                }
                for &b in std::fs::read(f).unwrap_or_default().iter() {
                    h ^= b as u64;
                    h = h.wrapping_mul(0x0000_0100_0000_01b3);
                }
            }
            h
        };

        // only the two real .rs files are collected (target/, .git/, and .txt excluded).
        let mut files = Vec::new();
        collect_rs(&dir, &mut files);
        assert_eq!(files.len(), 2, "must collect exactly src/a.rs + sub/b.rs");

        let h1 = hash(&dir);
        assert_eq!(h1, hash(&dir), "deterministic");
        // editing an ignored dir must NOT change the hash.
        std::fs::write(dir.join("target/x/c.rs"), "fn c(){ let _=9; }").unwrap();
        std::fs::write(dir.join(".git/d.rs"), "fn d(){ let _=9; }").unwrap();
        assert_eq!(h1, hash(&dir), "target/ and .git/ edits are ignored");
        // editing a real source file MUST change it.
        std::fs::write(dir.join("src/a.rs"), "fn a(){ let _=1; }").unwrap();
        assert_ne!(h1, hash(&dir), "a real .rs edit changes the hash");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The report glob must pick up `<base>.<crate>.<type>.json` (the `.*.*.json` shape) but NOT the
    /// `<base>.calibrated.json` / `<base>.encountered-*.json` sidecars (only two dot-segments) — and
    /// `glob_encountered` must do the reverse. Getting this wrong folds coverage data into entries.
    #[test]
    fn globs_discriminate_reports_from_sidecars() {
        let dir = std::env::temp_dir().join("candor-query-glob-test");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        for f in [
            "report.mycrate.lib.json",       // report ✓
            "report.mycrate.Executable.json", // report ✓
            "report.calibrated.json",        // sidecar ✗ (2 dots)
            "report.encountered-mycrate.json", // sidecar ✗ for reports, ✓ for encountered
            "report.single.json",            // ✗ (2 dots)
            "other.a.b.json",                // different base ✗
        ] {
            std::fs::write(dir.join(f), "[]").unwrap();
        }
        let prefix = dir.join("report");
        let prefix = prefix.to_str().unwrap();

        let reports: Vec<String> =
            glob_reports(prefix).iter().map(|p| p.file_name().unwrap().to_str().unwrap().to_string()).collect();
        assert_eq!(reports, vec!["report.mycrate.Executable.json", "report.mycrate.lib.json"]);

        let enc: Vec<String> =
            glob_encountered(prefix).iter().map(|p| p.file_name().unwrap().to_str().unwrap().to_string()).collect();
        assert_eq!(enc, vec!["report.encountered-mycrate.json"]);

        assert_eq!(prefix_base(prefix), "report");
        let _ = std::fs::remove_dir_all(&dir);
    }

    // ── `unverified --class` (SPEC §6.2 ⟨0.24⟩) ───────────────────────────────────────────────────
    // Written as EXIT-CODE assertions through `cmd_unverified --strict` (1 = holes remain, 0 = none),
    // so they exercise the shipped verb end to end — report load, `--class` parse, the transitive
    // accumulator and the match rule — rather than a copy of the predicate that could drift from it.

    /// Write a one-file report + policy under a fresh temp dir and return `(report_path, policy_path)`.
    /// The dir is REMOVED first: a stale artifact from a previous run reads as a flattering result.
    fn unv_fixture(name: &str, entries: serde_json::Value, policy: &str) -> (String, String) {
        let dir = std::env::temp_dir().join(format!("candor-query-unverified-{name}"));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let rep = dir.join("rep.fixture.scan.json");
        let pol = dir.join("candor.policy");
        std::fs::write(&rep, serde_json::json!({ "candor": "0.24", "functions": entries }).to_string())
            .unwrap();
        std::fs::write(&pol, policy).unwrap();
        (rep.to_str().unwrap().to_string(), pol.to_str().unwrap().to_string())
    }

    fn unv(rep: &str, pol: &str, class: Option<&str>) -> i32 {
        let mut args: Vec<String> = vec![
            "--report".into(), rep.into(), "--policy".into(), pol.into(), "--strict".into(),
        ];
        if let Some(c) = class {
            args.push("--class".into());
            args.push(c.into());
        }
        crate::unverified::cmd_unverified(&args)
    }

    #[test]
    fn unverified_class_resolves_the_reason_transitively() {
        // FAULT 2. `unknownWhy` is DIRECT-only by design (§4), so `domain::price` — whose `Unknown` is
        // purely INHERITED from `infra::dial` — carries no reason of its own. It is the ONLY hole here
        // (the policy is scoped to `domain`), so a filter reading the direct field drops the single
        // thing the verb exists to name, and drops it MORE the more the user narrows.
        let (rep, pol) = unv_fixture(
            "transitive",
            serde_json::json!([
                { "fn": "infra::dial", "inferred": ["Unknown"], "direct": ["Unknown"],
                  "unknownWhy": ["dispatch:Port"] },
                { "fn": "domain::price", "inferred": ["Unknown"], "calls": ["infra::dial"] },
            ]),
            "deny Exec domain\n",
        );
        assert_eq!(unv(&rep, &pol, None), 1, "unfiltered: the inherited hole is a hole");
        // `dynamic` names every genuine class, so it must exclude NOTHING — the cheap diagnostic.
        assert_eq!(unv(&rep, &pol, Some("dynamic")), 1, "--class dynamic must exclude nothing");
        assert_eq!(unv(&rep, &pol, Some("*")), 1, "--class * must exclude nothing");
        // …resolved to the CALLEE's class. This is the assertion the direct-only read fails.
        assert_eq!(unv(&rep, &pol, Some("dispatch")), 1, "inherited Unknown carries the callee's class");
        // CONTROL — and the one a blanket "keep everything" would fail. The filter must still
        // DISCRIMINATE: a class nothing in the reach carries selects nothing.
        assert_eq!(unv(&rep, &pol, Some("native")), 0, "no native reason anywhere in the reach");
        assert_eq!(unv(&rep, &pol, Some("reflect")), 0, "no reflect reason anywhere in the reach");
        // CONTROL — the MIRROR FABRICATION. `domain::price` has no reason set of its own, but its
        // `Unknown` is perfectly well classified at the callee; contributing `unresolved` to it because
        // its own reasons are absent would trade the fail-open for a fabricated class.
        assert_eq!(unv(&rep, &pol, Some("unresolved")), 0, "an inherited, CLASSIFIED hole is not `unresolved`");
    }

    #[test]
    fn unverified_class_fails_closed_on_an_unnamed_direct_unknown() {
        // FAULT 1, and the gate on it. `infra::mute` INTRODUCED its `Unknown` (`direct ∋ Unknown`) and
        // named nothing — §6.2 says that contributes `unresolved`. It must contribute PER ENTRY, into
        // the direct map, not by the absence of a class set at the caller: `domain::both` also calls a
        // `dispatch:`-reasoned callee, and an absence-keyed rule is swallowed by that other reason.
        let (rep, pol) = unv_fixture(
            "failclosed",
            serde_json::json!([
                { "fn": "infra::mute", "inferred": ["Unknown"], "direct": ["Unknown"] },
                { "fn": "infra::murky", "inferred": ["Unknown"], "direct": ["Unknown"],
                  "unknownWhy": ["dispatch:Port"] },
                { "fn": "domain::both", "inferred": ["Unknown"],
                  "calls": ["infra::mute", "infra::murky"] },
            ]),
            "deny Exec domain\n",
        );
        assert_eq!(unv(&rep, &pol, None), 1);
        assert_eq!(unv(&rep, &pol, Some("dynamic")), 1, "--class dynamic must exclude nothing");
        // Both classes reach `domain::both`; naming EITHER keeps it. Adding a reason must never REMOVE
        // a class — the failure mode is `unresolved` here going quiet because `dispatch` is also present.
        assert_eq!(unv(&rep, &pol, Some("unresolved")), 1, "the unnamed direct Unknown contributes `unresolved`");
        assert_eq!(unv(&rep, &pol, Some("dispatch")), 1, "the named callee still contributes `dispatch`");
        assert_eq!(unv(&rep, &pol, Some("native")), 0, "still discriminates");
    }

    #[test]
    fn reason_class_matches_keeps_the_unclassifiable() {
        // The §6.2 fail-closed net, at the rule itself: an entry with NO class set is kept by a filter
        // naming `unresolved` and by `*`/`dynamic` (which contain it) — never dropped by every filter.
        use candor_classify::policy::reason_class_matches;
        fn want<'a>(ts: &[&'a str]) -> BTreeSet<&'a str> {
            ts.iter().copied().collect()
        }
        assert!(reason_class_matches(None, &want(&["unresolved"])));
        assert!(!reason_class_matches(None, &want(&["dispatch"])));
        assert!(reason_class_matches(Some(&BTreeSet::new()), &want(&["unresolved"])));
        let cs: BTreeSet<String> = ["dispatch".to_string()].into_iter().collect();
        assert!(reason_class_matches(Some(&cs), &want(&["dispatch", "native"])));
        assert!(!reason_class_matches(Some(&cs), &want(&["unresolved"])));
    }
