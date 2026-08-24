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

    // ── ⟨0.28⟩ the descriptive verbs' completeness re-disclosure ──────────────────────────────────
    //
    // The shipped ACCEPTANCE TEST for this rung is candor-spec conformance PART 40, the (artifact state
    // × read verb) matrix, and it covers the six verbs end to end. These two tests exist for the parts
    // PART 40 structurally CANNOT see, both of which are about the mechanism rather than a verb:
    //
    //   · PART 40 reads only STDOUT and classifies by key, so a change to the ORDER of the keys it does
    //     not read is invisible to it — and the first draft of this rung re-sorted two verbs' documents
    //     on every ordinary run by round-tripping them through `serde_json::to_value` (a BTreeMap). It
    //     was caught by diffing output over an intact report, which is not something a suite does;
    //   · PART 40 has no cell for "an ordinary complete report", only the intact-CONTROL cell, which
    //     asserts the verb ANSWERED and nothing about what it answered.

    /// Build a one-report fixture under a fresh temp dir and return its path. `unanalyzed`/`analyzed`
    /// are written verbatim so a test can produce any row of SPEC §2's table.
    fn comp_fixture(name: &str, envelope: serde_json::Value) -> String {
        let dir = std::env::temp_dir().join(format!("candor-query-completeness-{name}"));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let rep = dir.join("rep.fixture.scan.json");
        std::fs::write(&rep, envelope.to_string()).unwrap();
        rep.to_str().unwrap().to_string()
    }

    /// A COMPLETE report must produce no disclosure at all — the property every caller relies on to stay
    /// byte-identical, asserted on the mechanism rather than on one verb's output.
    #[test]
    fn a_complete_report_discloses_nothing_on_either_channel() {
        let rep = comp_fixture(
            "complete",
            serde_json::json!({
                "candor": "0.28",
                "analyzed": { "count": 2 },
                "functions": [{ "fn": "f", "inferred": ["Fs"], "direct": ["Fs"] }],
            }),
        );
        let comp = crate::completeness::report_completeness(&rep);
        assert!(!comp.incomplete(), "a complete report is not incomplete");
        assert!(!comp.must_hedge(), "a complete report has nothing to hedge about");
        assert!(comp.fields().is_none(), "no flattened disclosure on a complete report");
        // `write_json` must leave the document EXACTLY as it found it — no `incomplete`, and no key
        // re-ordering either, which is why this compares serialized bytes and not a `Value`.
        let mut doc = serde_json::json!({ "effect": "Fs", "directly": [], "inherited": [] });
        let before = serde_json::to_string(&doc).unwrap();
        comp.write_json(&mut doc);
        assert_eq!(serde_json::to_string(&doc).unwrap(), before, "write_json is not a no-op");
        let mut note = Vec::new();
        comp.write_note_for_test(&mut note, "x", "y");
        assert!(note.is_empty(), "print_note is not a no-op");
    }

    /// `analyzed.count: 0` MUST raise the disclosure and MUST NOT raise `incomplete()`.
    ///
    /// The whole reason the two predicates are separate: `incomplete()` is what `unverified --strict`
    /// and `fix-gate --strict` compute an exit 2 from, and ⟨0.24⟩ ruled count-0 *"A DISCLOSURE, NOT AN
    /// EXIT CODE"* — `gate --report` exits 0 over these bytes, so a verb exiting 2 would claim it got
    /// less far than the gate on identical input. Fold the count-0 arm into `incomplete()` and this
    /// fails; nothing else in the tree would notice, because no conformance part gates a --strict
    /// advisory verb over a judged-nothing report.
    #[test]
    fn judged_nothing_hedges_the_answer_without_touching_the_exit_code() {
        let rep = comp_fixture(
            "judged-nothing",
            serde_json::json!({ "candor": "0.28", "analyzed": { "count": 0 }, "functions": [] }),
        );
        let comp = crate::completeness::report_completeness(&rep);
        assert!(!comp.incomplete(), "count-0 must NOT reach the exit-code predicate");
        assert!(comp.must_hedge(), "count-0 must reach the disclosure predicate");
        let mut doc = serde_json::json!({ "reaches": [] });
        comp.write_json(&mut doc);
        assert_eq!(doc["incomplete"], serde_json::json!(true));
        assert_eq!(doc["judgedNothing"].as_array().unwrap().len(), 1);
        assert!(doc.get("unanalyzed").is_none(), "there is no unread FILE in the count-0 row");
        // …and the prose must not send the reader to a gate that will pass and make this look like noise.
        assert!(comp.gate_line().contains("exits 0"), "count-0 must not claim the gate refuses");

        // The OTHER cause still reaches both, and still claims the refusal, which is true of it.
        let rep = comp_fixture(
            "unanalyzed",
            serde_json::json!({
                "candor": "0.28",
                "analyzed": { "count": 3 },
                "unanalyzed": [{ "path": "src/a.rs", "reason": "parse error" }],
                "functions": [{ "fn": "f", "inferred": ["Fs"], "direct": ["Fs"] }],
            }),
        );
        let comp = crate::completeness::report_completeness(&rep);
        assert!(comp.incomplete() && comp.must_hedge());
        assert!(comp.gate_line().contains("exits 2"));
        let mut doc = serde_json::json!({ "reaches": [] });
        comp.write_json(&mut doc);
        assert_eq!(doc["unanalyzed"].as_array().unwrap().len(), 1);
        assert!(doc.get("judgedNothing").is_none(), "a report with 3 analyzed units judged something");
    }

    /// ⟨0.28⟩ SPEC §2 — **THE THIRD ROW IS NOT THE FIRST ROW.** A report carrying NO `analyzed` key
    /// hedges under `noManifest`, NEVER under `judgedNothing`.
    ///
    /// MEASURED on this engine before the split, over `{"candor":…,"functions":[]}` with no `analyzed`
    /// key: `where`, `blindspots`, `map`, `reachable`, `unverified`, `fix-gate` and `gains` all filed it
    /// under `judgedNothing`, and the note said it *"say[s] they JUDGED NOTHING (`analyzed.count: 0`)"*.
    /// **The report declares nothing.** The hedge is the right direction — row 3's instruction is *no
    /// manifest, no claim* — but the disclosure is FALSE, and this family rates a false disclosure worse
    /// than a missing one. It is also a hole in ⟨0.28⟩'s own pin, which defines `judgedNothing` as
    /// *reports declaring `analyzed.count: 0`*: a row-3 report is not one, and the two want different
    /// repairs (row 1: a scan that reaches a conclusion; row 3: a producer that emits a manifest).
    ///
    /// THE SPLIT GOES BOTH WAYS OR IT IS A RENAME, so row 1 is asserted here too — and row 2 is the
    /// CONTROL that makes either meaningful: `count: n>0` with `functions: []` is a legitimate all-pure
    /// claim §2 rule 3 requires a consumer to BELIEVE, and a fix that hedges all three has disabled the
    /// feature rather than implemented the rule (measured over 1997 JVM dependency jars: it would
    /// withdraw 104 real claims to catch 6).
    #[test]
    fn a_report_with_no_analyzed_manifest_is_row_three_not_row_one() {
        // ROW 3: no `analyzed` key at all, and nothing listed.
        let rep = comp_fixture(
            "no-manifest",
            serde_json::json!({ "candor": "0.20", "functions": [] }),
        );
        let comp = crate::completeness::report_completeness(&rep);
        assert!(!comp.incomplete(), "row 3 must NOT reach the exit-code predicate — the gate exits 0");
        assert!(comp.must_hedge(), "row 3 must reach the disclosure predicate — no manifest, no claim");
        let mut doc = serde_json::json!({ "reaches": [] });
        comp.write_json(&mut doc);
        assert_eq!(doc["incomplete"], serde_json::json!(true));
        assert_eq!(doc["noManifest"], serde_json::json!([rep]),
                   "SPEC §2 pins `noManifest: [\"<report path>\", …]` verbatim: {doc}");
        assert!(doc.get("judgedNothing").is_none(),
                "a row-3 report DECLARES nothing — saying it declared `analyzed.count: 0` is a FALSE \
                 disclosure, and one key meaning two things loses the distinction §2's table draws: {doc}");
        // …and the human channel stops asserting it too, on both the per-file line and the gate line.
        let mut note = Vec::new();
        comp.write_note_for_test(&mut note, "x", "y");
        let note = String::from_utf8(note).unwrap();
        assert!(note.contains("NO `analyzed` manifest"), "the prose must name the real cause: {note}");
        assert!(!note.contains("analyzed.count: 0") && !note.contains("JUDGED NOTHING"),
                "…and must not send the reader to row 1's repair: {note}");

        // ROW 1: `analyzed.count: 0` stays `judgedNothing` and never becomes `noManifest`.
        let rep1 = comp_fixture(
            "no-manifest-row1",
            serde_json::json!({ "candor": "0.28", "analyzed": { "count": 0 }, "functions": [] }),
        );
        let c1 = crate::completeness::report_completeness(&rep1);
        let mut d1 = serde_json::json!({ "reaches": [] });
        c1.write_json(&mut d1);
        assert_eq!(d1["judgedNothing"], serde_json::json!([rep1]), "{d1}");
        assert!(d1.get("noManifest").is_none(), "the split goes both ways or it is a rename: {d1}");

        // ROW 2, THE CONTROL: `count: 7`, `functions: []` — an all-pure claim, and it MUST NOT hedge.
        let rep2 = comp_fixture(
            "no-manifest-row2",
            serde_json::json!({ "candor": "0.28", "analyzed": { "count": 7 }, "functions": [] }),
        );
        let c2 = crate::completeness::report_completeness(&rep2);
        assert!(!c2.must_hedge(), "row 2 is a legitimate all-pure claim §2 rule 3 requires a consumer \
                                   to BELIEVE — hedging all three rows disables the feature");
        assert!(c2.fields().is_none());

        // AND THE OTHER CONTROL: manifest-less but it LISTS functions. It judged something and said so
        // the only way a pre-⟨0.21⟩ producer could, so it keeps the standing §2's row 3 gives it.
        let rep3 = comp_fixture(
            "no-manifest-with-entries",
            serde_json::json!({
                "candor": "0.20",
                "functions": [{ "fn": "f", "inferred": ["Fs"], "direct": ["Fs"] }],
            }),
        );
        let c3 = crate::completeness::report_completeness(&rep3);
        assert!(!c3.must_hedge(), "a manifest-less report that LISTS entries is not hedging at all");

        // A LOCATOR NAMING ONE OF EACH discloses them under SEPARATE keys — the whole point of the split.
        let dir = std::env::temp_dir().join("candor-query-completeness-no-manifest-both");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("rep.aa.scan.json"),
                       r#"{"candor":"0.28","analyzed":{"count":0},"functions":[]}"#).unwrap();
        std::fs::write(dir.join("rep.bb.scan.json"), r#"{"candor":"0.20","functions":[]}"#).unwrap();
        let both = crate::completeness::report_completeness(dir.join("rep").to_str().unwrap());
        let mut db = serde_json::json!({ "reaches": [] });
        both.write_json(&mut db);
        assert_eq!(db["judgedNothing"].as_array().unwrap().len(), 1, "{db}");
        assert_eq!(db["noManifest"].as_array().unwrap().len(), 1, "{db}");
        assert!(db["judgedNothing"][0].as_str().unwrap().ends_with("rep.aa.scan.json"), "{db}");
        assert!(db["noManifest"][0].as_str().unwrap().ends_with("rep.bb.scan.json"), "{db}");
    }

    /// ⟨0.32⟩ **AN UNREAD EXCLUSION CLASS HEDGES A DESCRIPTIVE ANSWER, AND MUST NOT TOUCH AN EXIT
    /// CODE** — the 2026-08-24 four-way ruling, pinned on the mechanism. See
    /// [`crate::completeness::ReportCompleteness::must_hedge`] for the argument; this asserts the shape
    /// of it in BOTH directions, because the SAFE value of the key under test passes a one-sided
    /// assertion while deleting the feature.
    ///
    /// The divergence it closes: `tour` printed the bare *"nothing hidden"* over a report whose
    /// `excluded` names a class nothing opened — in this engine, candor-ts and candor-swift; candor-java
    /// hedged and named the class, and was right.
    #[test]
    fn an_unread_exclusion_class_hedges_the_answer_without_touching_the_exit_code() {
        let unread = serde_json::json!({
            "candor": "0.32",
            "analyzed": { "count": 1 },
            "excluded": [{ "class": "build-script", "count": 1, "peeked": false, "reason": "r" }],
            "functions": [{ "fn": "f", "inferred": ["Fs"], "direct": ["Fs"] }],
        });
        let rep = comp_fixture("unread-class", unread.clone());
        let comp = crate::completeness::report_completeness(&rep);
        assert!(comp.must_hedge(), "a class the scan never opened must reach the disclosure predicate");
        assert!(
            !comp.incomplete(),
            "…and MUST NOT reach the exit-code predicate: `unverified --strict` computes its 2 from \
             this, and over a report the gate — holding no deny rule — exits 0 on, a verb exiting 2 \
             would claim it got LESS far than the gate. ⟨0.24⟩'s over-claim, mirrored."
        );
        // The MACHINE half raises the flag and mints NO new key: `unread` is the ADVISORY route's wire
        // spelling and this engine is the only one that publishes it, so widening it to the six
        // descriptive verbs would be a fifth key set nothing else speaks (see `fields`).
        let mut doc = serde_json::json!({ "reaches": [] });
        comp.write_json(&mut doc);
        assert_eq!(doc["incomplete"], serde_json::json!(true), "{doc}");
        assert!(doc.get("unread").is_none(), "the descriptive hedge mints no wire key: {doc}");
        // The HUMAN half names the class AND the cause — a hedge whose sentence says nothing is the
        // deleted disclosure arriving inside the disclosure (measured on java and rust, same rung).
        let mut note = Vec::new();
        comp.write_note_for_test(&mut note, "so-what", "tail");
        let note = String::from_utf8(note).unwrap();
        assert!(note.contains("exclusion class(es) the scan did NOT READ"), "{note}");
        assert!(note.contains("build-script"), "the class must be NAMED: {note}");
        // …and the tail must not send the reader to a gate that will pass and make this read as noise,
        // nor claim a gate is running when this verb holds no policy at all.
        assert!(comp.gate_line().contains("under any policy it can evaluate"), "{}", comp.gate_line());

        // OVER-CHARGE CONTROL 1 — the SAME report with the class PEEKED gets the unhedged answer and a
        // byte-identical document. Without it this test passes for an engine that hedges every report,
        // which deletes the sentence rather than qualifying it.
        let mut peeked = unread.clone();
        peeked["excluded"][0]["peeked"] = serde_json::json!(true);
        let c = crate::completeness::report_completeness(&comp_fixture("peeked-class", peeked));
        assert!(!c.must_hedge(), "a class the peek READ is not an unread class");
        assert!(c.fields().is_none());

        // OVER-CHARGE CONTROL 2 — the producer's own ⟨0.32⟩ carve-out. `judgedElsewhere: true` says
        // another report judges those files, and it must cost this answer nothing, exactly as it costs
        // `gate --report` nothing (measured four-way, exit 0).
        let mut elsewhere = unread;
        elsewhere["excluded"][0]["judgedElsewhere"] = serde_json::json!(true);
        let c = crate::completeness::report_completeness(&comp_fixture("elsewhere-class", elsewhere));
        assert!(!c.must_hedge(), "`judgedElsewhere: true` is judged, not unread");
    }

    /// ⟨0.28⟩ **THE TRAP THE ROW-3 SPLIT SETS, PINNED.** [`candor_report::report_judged_nothing`] is not
    /// only a disclosure predicate — it is what candor-scan's chained join
    /// (`DepIndex::judged_nothing_pkgs` → the κ ledger's coverage exemption) and `gate --report` read to
    /// decide COVERAGE, and row 3's own instruction is *no manifest, no claim*: an absent manifest must
    /// keep granting NONE. The tempting fix for the false label — make that predicate answer `false`
    /// for a manifest-less report — would turn every pre-⟨0.21⟩ report into a COVERED one, a silent
    /// under-report introduced by a disclosure fix. So the split adds a SECOND predicate and this asserts
    /// the first is unmoved.
    #[test]
    fn the_row_three_split_does_not_move_the_coverage_predicate() {
        let row3 = r#"{"candor":"0.20","package":"legacy","functions":[]}"#;
        assert!(candor_report::report_judged_nothing(row3),
                "an absent manifest must STILL grant no coverage — row 3 is `no manifest, no claim`");
        assert!(candor_report::report_has_no_manifest(row3), "…and it is row 3, not row 1");
        let row1 = r#"{"candor":"0.28","package":"facade","analyzed":{"count":0},"functions":[]}"#;
        assert!(candor_report::report_judged_nothing(row1));
        assert!(!candor_report::report_has_no_manifest(row1), "row 1 HAS a manifest; it declares 0");
        let row2 = r#"{"candor":"0.28","package":"pure","analyzed":{"count":7},"functions":[]}"#;
        assert!(!candor_report::report_judged_nothing(row2), "the row-2 control: a believed all-pure claim");
        assert!(!candor_report::report_has_no_manifest(row2));
        // A manifest-less report that LISTS entries judged something: not row-3-hedged (the disclosure
        // ANDs the two predicates), and it grants coverage exactly as it did before this rung.
        let row3_full = r#"{"candor":"0.20","package":"legacy","functions":[{"fn":"f","inferred":["Fs"]}]}"#;
        assert!(!candor_report::report_judged_nothing(row3_full));
        assert!(candor_report::report_has_no_manifest(row3_full));
        // Unparsable text: `report_judged_nothing` fails CLOSED because it decides coverage; the row-3
        // predicate answers `false`, because a file whose bytes cannot be read did not "carry no
        // manifest" — the `unreadable` arm is the actionable disclosure for it.
        assert!(candor_report::report_judged_nothing("{not json"));
        assert!(!candor_report::report_has_no_manifest("{not json"));
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
