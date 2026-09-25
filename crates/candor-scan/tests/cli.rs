//! End-to-end CLI tests that drive the COMPILED `candor-scan` binary as a subprocess, so they can
//! assert on the real stdout/stderr split + process exit code — things an in-process `scan_one` call
//! cannot observe. (Cargo sets `CARGO_BIN_EXE_candor-scan` to the built binary for this integration test.)

use std::path::PathBuf;
use std::process::Command;

fn bin() -> &'static str {
    env!("CARGO_BIN_EXE_candor-scan")
}

/// A throwaway crate dir under the temp dir, removed by the caller.
fn make_crate(name: &str, src: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("candor-scan-cli-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(d.join("src")).unwrap();
    std::fs::write(d.join("Cargo.toml"), format!("[package]\nname = \"{name}\"\n")).unwrap();
    std::fs::write(d.join("src/lib.rs"), src).unwrap();
    d
}

#[test]
fn json_plus_policy_keeps_stdout_pure_json_and_routes_violations_to_stderr() {
    // CRITICAL: a gated `--json` run must keep stdout a SINGLE pure JSON document (pipeable to `jq`).
    // The policy gate's human output — the violation lines AND the ✓/count summary — must go to STDERR,
    // never interleave into the JSON stream. Verified on a VIOLATING crate (exit 1).
    let d = make_crate("jsonpol", "pub fn go() { std::process::Command::new(\"sh\").status().unwrap(); }");
    let pp = d.join("candor.policy");
    std::fs::write(&pp, "deny Exec\n").unwrap();

    let out = Command::new(bin())
        .arg(d.to_string_lossy().as_ref())
        .arg("--json")
        .arg("--policy")
        .arg(pp.to_string_lossy().as_ref())
        .output()
        .expect("run candor-scan");

    let _ = std::fs::remove_dir_all(&d);

    // A real violation → exit 1.
    assert_eq!(out.status.code(), Some(1), "a deny-Exec violation must exit 1");

    // stdout parses as JSON — the gate output did NOT pollute it.
    let stdout = String::from_utf8(out.stdout).expect("utf8 stdout");
    let parsed: Result<serde_json::Value, _> = serde_json::from_str(stdout.trim());
    assert!(parsed.is_ok(), "stdout under --json --policy must parse as JSON, got:\n{stdout}");

    // the violation text is on STDERR, not stdout.
    let stderr = String::from_utf8(out.stderr).expect("utf8 stderr");
    assert!(stderr.contains("AS-EFF") || stderr.contains("violation"),
            "the policy violation must be reported on stderr, got stderr:\n{stderr}");
    assert!(!stdout.contains("AS-EFF"),
            "no policy/violation text may appear on the JSON stdout stream:\n{stdout}");
}

#[test]
fn valueless_trailing_policy_flag_errors_exit_2() {
    // LOW: a trailing bare `--policy` with no value must ERROR (exit 2) — matching the strict posture of
    // a set-but-unreadable policy — rather than silently falling back to a no-gate scan.
    let d = make_crate("nopolval", "pub fn go() {}");
    let out = Command::new(bin())
        .arg(d.to_string_lossy().as_ref())
        .arg("--policy") // no value follows
        .output()
        .expect("run candor-scan");
    let _ = std::fs::remove_dir_all(&d);
    assert_eq!(out.status.code(), Some(2), "a valueless --policy must exit 2, not silently skip the gate");
}

#[test]
fn unreadable_policy_exits_2() {
    // The existing strict posture this fix mirrors: a SET but UNREADABLE policy path must exit 2.
    let d = make_crate("unreadpol", "pub fn go() {}");
    let missing = d.join("does-not-exist.policy");
    let out = Command::new(bin())
        .arg(d.to_string_lossy().as_ref())
        .arg("--policy")
        .arg(missing.to_string_lossy().as_ref())
        .output()
        .expect("run candor-scan");
    let _ = std::fs::remove_dir_all(&d);
    assert_eq!(out.status.code(), Some(2), "an unreadable policy must exit 2");
}

#[test]
fn json_plus_policy_over_unparseable_source_exits_2() {
    // CRITICAL (cross-check via the real binary): a configured gate over a crate with an UNPARSEABLE
    // source file must exit 2 (gateless-green closed), and stdout — when it emits any — must still be JSON.
    let d = make_crate("brokenbin", "pub fn ok() {}\nthis is not valid rust @@@\n");
    let pp = d.join("candor.policy");
    std::fs::write(&pp, "deny Exec\n").unwrap();
    let out = Command::new(bin())
        .arg(d.to_string_lossy().as_ref())
        .arg("--json")
        .arg("--policy")
        .arg(pp.to_string_lossy().as_ref())
        .output()
        .expect("run candor-scan");
    let _ = std::fs::remove_dir_all(&d);
    assert_eq!(out.status.code(), Some(2),
               "a gate over an unparseable source must exit 2, never green");
    let stdout = String::from_utf8(out.stdout).expect("utf8 stdout");
    if !stdout.trim().is_empty() {
        assert!(serde_json::from_str::<serde_json::Value>(stdout.trim()).is_ok(),
                "any stdout under --json must remain valid JSON:\n{stdout}");
    }
}

// ── the bare-scan / --json baseline ───────────────────────────────────────────────────────────────

#[test]
fn bare_scan_writes_report_files_and_exits_0() {
    // The default mode: no flags → write the report (+ callgraph sidecar) under <dir>/.candor/, exit 0.
    let d = make_crate("bare", "pub fn go() { let _ = std::fs::read(\"/x\"); }");
    let out = Command::new(bin()).arg(d.to_string_lossy().as_ref()).output().expect("run candor-scan");
    assert_eq!(out.status.code(), Some(0), "a clean bare scan must exit 0");
    // Default prefix is <dir>/.candor/report → report.<crate>.scan.json + the callgraph sidecar.
    assert!(d.join(".candor/report.bare.scan.json").is_file(), "bare scan must write the report file");
    assert!(d.join(".candor/report.bare.scan.callgraph.json").is_file(), "bare scan must write the callgraph sidecar");
    let _ = std::fs::remove_dir_all(&d);
}

#[test]
fn generic_struct_field_resolves_to_its_trait_bound_dispatch() {
    // R31 (soundness 2026-07-10): a stored field typed as the STRUCT's own bounded generic param
    // (`struct Pipe<T: Saver> { item: T }`) reaching `self.item.save()` read silent-pure — field types
    // were resolved with an EMPTY generic-bounds map, so `T` never resolved to `Saver` and never
    // dispatched. Now the struct's own `<T: Bound>` / `where T: Bound` seeds the field's trait leaves.
    let src = "
        use std::fs;
        trait Saver { fn save(&self); }
        struct DiskSaver;
        impl Saver for DiskSaver { fn save(&self) { let _ = fs::write(\"/tmp/s\", \"x\"); } }
        struct Pipe<T: Saver> { item: T }
        impl<T: Saver> Pipe<T> { fn run(&self) { self.item.save(); } }
        pub fn use_pipe(p: &Pipe<DiskSaver>) { p.run(); }
        struct Plain<T> { item: T }
        pub fn use_plain(p: &Plain<DiskSaver>) -> &DiskSaver { &p.item }  // no method call → must stay pure
    ";
    let d = make_crate("genfield", src);
    let out = Command::new(bin()).arg(d.to_string_lossy().as_ref()).arg("--json").output().expect("run");
    let stdout = String::from_utf8(out.stdout).expect("utf8");
    let v: serde_json::Value = serde_json::from_str(stdout.trim()).expect("json");
    let eff = |fnname: &str| -> Vec<String> {
        v["functions"].as_array().unwrap().iter()
            .find(|f| f["fn"].as_str().unwrap_or("").ends_with(fnname))
            .and_then(|f| f["inferred"].as_array())
            .map(|a| a.iter().filter_map(|x| x.as_str().map(String::from)).collect())
            .unwrap_or_default()
    };
    assert!(eff("use_pipe").contains(&"Fs".to_string()),
            "a bounded-generic struct field's method must dispatch (was silent-pure): {:?}", eff("use_pipe"));
    assert!(!eff("use_plain").contains(&"Fs".to_string()),
            "an unconstrained-generic field read (no method call) must not fabricate: {:?}", eff("use_plain"));
    let _ = std::fs::remove_dir_all(&d);
}

#[test]
fn trait_default_method_via_empty_impl_charges_the_default_body() {
    // R30 (soundness 2026-07-11): a trait DEFAULT method reached via an empty `impl Trait for T {}` read
    // silent-pure. The fallback that edges `t.m()` → `Trait::m` existed, but a type whose ONLY impl is an
    // (empty/non-overriding) trait impl had no fn unit of its own, so it was absent from `local_types` →
    // its typed call was un-`resolvable` → the fallback was gated out. Fix: register every trait-impl type
    // as local. An OVERRIDE still wins (only the override's effect); a pure default stays pure.
    let src = "
        use std::fs;
        trait Logger { fn flush(&self) { let _ = fs::write(\"/tmp/l\", \"x\"); } }  // Fs default
        struct FileLogger;
        impl Logger for FileLogger {}
        pub fn use_default(l: &FileLogger) { l.flush(); }        // was silent → Fs
        struct Quiet;
        impl Logger for Quiet { fn flush(&self) {} }             // pure override
        pub fn use_override(q: &Quiet) { q.flush(); }            // must stay pure (override wins, no fab)
    ";
    let d = make_crate("traitdefault", src);
    let out = Command::new(bin()).arg(d.to_string_lossy().as_ref()).arg("--json").output().expect("run");
    let v: serde_json::Value = serde_json::from_str(String::from_utf8(out.stdout).unwrap().trim()).expect("json");
    let eff = |name: &str| -> Vec<String> {
        v["functions"].as_array().unwrap().iter()
            .find(|f| f["fn"].as_str().map(|s| s == name || s.ends_with(&format!("::{name}"))).unwrap_or(false))
            .and_then(|f| f["inferred"].as_array())
            .map(|a| a.iter().filter_map(|x| x.as_str().map(String::from)).collect())
            .unwrap_or_default()
    };
    assert!(eff("use_default").contains(&"Fs".to_string()),
            "a trait default reached via an empty impl must charge (was silent): {:?}", eff("use_default"));
    assert!(!eff("use_override").contains(&"Fs".to_string()),
            "an override of the default must win — no fabrication of the default's effect: {:?}", eff("use_override"));
    let _ = std::fs::remove_dir_all(&d);
}

#[test]
fn concrete_receiver_and_fn_returned_trait_object_method_calls_are_never_silent_pure() {
    // SILENT-PURE cardinal-sin fix (2026-07-18): a trait method called on a CONCRETE receiver via
    // method syntax (`T0.run()`), and a dispatch through a FUNCTION-RETURNED boxed trait object
    // (`get().run()` where `get() -> Box<dyn Task>`), were both reported PURE — not even `Unknown` —
    // though they reach an effectful impl at runtime. The trait-OBJECT-via-CHA control cases already
    // worked; the concrete-receiver and fn-return-typed paths did not.
    //   - Case C root cause: `resolve_recv_type`'s Path arm only consulted `vars`; a bare unit-struct
    //     VALUE literal (`T0`) is a type, not a binding, so it typed to nothing and dropped pure.
    //     Fix: an Upper-initial value path with no underscore types as itself (gated downstream by
    //     `local_types`, so a non-local name never mis-links).
    //   - Case D root cause: a `-> Box<dyn Task>` return has no nominal type (`type_path` drops the
    //     trait object), so the factory-call receiver typed to nothing and `resolve_recv_traits` had
    //     no `Expr::Call` arm. Fix: record the return's trait bounds under a `<dyn>` sentinel and run
    //     the SAME bounded-CHA the direct trait-object receiver does — resolving to every local
    //     implementor, or `Unknown` when none is visible (never silent-pure).
    let src = "
        trait Task { fn run(&self); }
        struct T0;
        impl Task for T0 { fn run(&self) { let _ = std::fs::read(\"x\"); } }   // Fs
        pub fn case_c() { T0.run(); }                                          // concrete receiver → Fs
        fn get() -> Box<dyn Task> { Box::new(T0) }
        pub fn case_d() { get().run(); }                                       // fn-returned dyn → Fs (CHA)
        pub fn ctrl_boxed() { let t: Box<dyn Task> = Box::new(T0); t.run(); }  // control: Fs
        pub fn ctrl_ref() { let t: &dyn Task = &T0; t.run(); }                 // control: Fs
        trait Void { fn run(&self); }                                         // declared, NO local impl
        fn make() -> Box<dyn Void> { unimplemented!() }
        pub fn case_d_unknown() { make().run(); }                             // unresolvable → Unknown
    ";
    let d = make_crate("silentpure", src);
    let out = Command::new(bin()).arg(d.to_string_lossy().as_ref()).arg("--json").output().expect("run");
    let v: serde_json::Value = serde_json::from_str(String::from_utf8(out.stdout).unwrap().trim()).expect("json");
    let eff = |name: &str| -> Vec<String> {
        v["functions"].as_array().unwrap().iter()
            .find(|f| f["fn"].as_str().map(|s| s == name || s.ends_with(&format!("::{name}"))).unwrap_or(false))
            .and_then(|f| f["inferred"].as_array())
            .map(|a| a.iter().filter_map(|x| x.as_str().map(String::from)).collect())
            .unwrap_or_default()
    };
    assert!(eff("case_c").contains(&"Fs".to_string()),
            "Case C: a trait method on a concrete receiver must resolve to its impl (was silent-pure): {:?}", eff("case_c"));
    assert!(eff("case_d").contains(&"Fs".to_string()),
            "Case D: a fn-returned boxed trait object must dispatch via CHA (was silent-pure): {:?}", eff("case_d"));
    assert!(eff("ctrl_boxed").contains(&"Fs".to_string()),
            "control: a direct Box<dyn Task> receiver must still resolve to Fs: {:?}", eff("ctrl_boxed"));
    assert!(eff("ctrl_ref").contains(&"Fs".to_string()),
            "control: a direct &dyn Task receiver must still resolve to Fs: {:?}", eff("ctrl_ref"));
    let unk = eff("case_d_unknown");
    assert!(unk.contains(&"Unknown".to_string()) && !unk.is_empty(),
            "Case D unresolvable: a fn-returned dyn with no visible impl must disclose Unknown, never silent-pure: {:?}", unk);
    let _ = std::fs::remove_dir_all(&d);
}

#[test]
fn json_prints_to_stdout_and_writes_no_files_exit_0() {
    // `--json` prints ONE JSON document to stdout and writes NOTHING to disk (no .candor/ dir).
    let d = make_crate("jsononly", "pub fn go() {}");
    let out = Command::new(bin())
        .arg(d.to_string_lossy().as_ref())
        .arg("--json")
        .output()
        .expect("run candor-scan");
    assert_eq!(out.status.code(), Some(0), "a clean --json scan must exit 0");
    let stdout = String::from_utf8(out.stdout).expect("utf8 stdout");
    assert!(serde_json::from_str::<serde_json::Value>(stdout.trim()).is_ok(),
            "--json stdout must parse as JSON, got:\n{stdout}");
    assert!(!d.join(".candor").exists(), "--json must NOT write any report files to disk");
    let _ = std::fs::remove_dir_all(&d);
}

#[test]
fn json_plus_clean_policy_is_pure_json_exit_0() {
    // `--json --policy <clean>`: stdout stays pure JSON, the gate's ✓ goes to stderr, exit 0.
    let d = make_crate("jsonclean", "pub fn go() {}");
    let pp = d.join("candor.policy");
    std::fs::write(&pp, "deny Exec\n").unwrap();
    let out = Command::new(bin())
        .arg(d.to_string_lossy().as_ref())
        .arg("--json")
        .arg("--policy")
        .arg(pp.to_string_lossy().as_ref())
        .output()
        .expect("run candor-scan");
    let _ = std::fs::remove_dir_all(&d);
    assert_eq!(out.status.code(), Some(0), "a clean --json --policy run must exit 0");
    let stdout = String::from_utf8(out.stdout).expect("utf8 stdout");
    assert!(serde_json::from_str::<serde_json::Value>(stdout.trim()).is_ok(),
            "stdout under --json --policy (clean) must parse as JSON, got:\n{stdout}");
    assert!(!stdout.contains('✓') && !stdout.contains("policy"),
            "the gate's ✓ summary must be on stderr, not stdout:\n{stdout}");
}

// ── the policy gate exit-code contract (non-json) ─────────────────────────────────────────────────

#[test]
fn violating_policy_exits_1_clean_policy_exits_0() {
    // A real violation → exit 1; the same scan against a non-overlapping deny → exit 0. The two halves
    // share a crate body so the only variable is the policy (the gate's verdict, not the scan).
    let d = make_crate("gate", "pub fn go() { let _ = std::fs::read(\"/x\"); }");

    let violating = d.join("violating.policy");
    std::fs::write(&violating, "deny Fs\n").unwrap();
    let out = Command::new(bin())
        .arg(d.to_string_lossy().as_ref())
        .arg("--policy")
        .arg(violating.to_string_lossy().as_ref())
        .output()
        .expect("run candor-scan");
    assert_eq!(out.status.code(), Some(1), "deny Fs over an Fs effect must exit 1");

    let clean = d.join("clean.policy");
    std::fs::write(&clean, "deny Exec\n").unwrap();
    let out = Command::new(bin())
        .arg(d.to_string_lossy().as_ref())
        .arg("--policy")
        .arg(clean.to_string_lossy().as_ref())
        .output()
        .expect("run candor-scan");
    assert_eq!(out.status.code(), Some(0), "deny Exec over an Fs-only crate must exit 0");

    let _ = std::fs::remove_dir_all(&d);
}

// ── version / help ────────────────────────────────────────────────────────────────────────────────

#[test]
fn version_prints_build_and_spec_exit_0() {
    // `--version` and `-V` both print `candor-scan <ver> (spec <X>)` as the first line, exit 0.
    for flag in ["--version", "-V"] {
        let out = Command::new(bin()).arg(flag).output().expect("run candor-scan");
        assert_eq!(out.status.code(), Some(0), "{flag} must exit 0");
        let stdout = String::from_utf8(out.stdout).expect("utf8 stdout");
        let first = stdout.lines().next().unwrap_or("");
        assert!(first.starts_with("candor-scan ") && first.contains("(spec "),
                "{flag} first line must be `candor-scan <ver> (spec <X>)`, got: {first}");
    }
}

#[test]
fn help_prints_usage_exit_0() {
    // `--help` and `-h` both print a USAGE banner, exit 0.
    for flag in ["--help", "-h"] {
        let out = Command::new(bin()).arg(flag).output().expect("run candor-scan");
        assert_eq!(out.status.code(), Some(0), "{flag} must exit 0");
        let stdout = String::from_utf8(out.stdout).expect("utf8 stdout");
        assert!(stdout.contains("USAGE"), "{flag} must print a USAGE line, got:\n{stdout}");
    }
}

// ── unknown flags ─────────────────────────────────────────────────────────────────────────────────

#[test]
fn unknown_flags_exit_2() {
    // A dash-prefixed token that isn't a known flag must FAIL (exit 2), never be swallowed as a path.
    // Covers a long `--bogus` and a single-dash `-x` (the typo'd-flag / newer-doc-old-binary failure).
    for flag in ["--bogus", "-x"] {
        let out = Command::new(bin()).arg(flag).output().expect("run candor-scan");
        assert_eq!(out.status.code(), Some(2), "unknown flag {flag} must exit 2");
        let stderr = String::from_utf8(out.stderr).expect("utf8 stderr");
        assert!(stderr.contains("unknown flag"), "{flag} must report `unknown flag`, got:\n{stderr}");
    }
}

#[test]
fn an_unknown_flags_operand_is_never_taken_as_the_scan_target() {
    // ⟨0.32⟩ The refusal MARKER is written to the prefix the refusing run WOULD have used, so the
    // target `prescan_argv` resolves is load-bearing on a run that never scans anything. It used to
    // take the LAST bare token, which on `candor-scan . --scope src` is the REJECTED flag's operand:
    // the marker went to `src/.candor/`, CREATING that directory in the operator's tree and
    // overwriting whatever marker was already there, while the target they actually gave was thrown
    // away. Both halves are asserted here — where it lands, and what it must not touch.
    let d = make_crate("uftarget", "pub fn go() {}");
    let root = d.to_string_lossy().into_owned();

    // A decoy at the path the operand names, holding bytes no candor run may replace.
    let decoy_dir = d.join("src").join(".candor");
    std::fs::create_dir_all(&decoy_dir).unwrap();
    let decoy = decoy_dir.join("report.refused.json");
    std::fs::write(&decoy, b"PRECIOUS").unwrap();

    let out = Command::new(bin())
        .current_dir(&d)
        .args([root.as_str(), "--scope", "src"])
        .output()
        .expect("run candor-scan");
    assert_eq!(out.status.code(), Some(2), "an unknown flag still refuses");

    assert_eq!(
        std::fs::read(&decoy).unwrap(),
        b"PRECIOUS",
        "the rejected flag's operand must not become a sink: {} was overwritten",
        decoy.display()
    );
    assert!(
        d.join(".candor").join("report.refused.json").exists(),
        "the marker belongs at the target the operator gave, not at the operand"
    );
}

#[test]
fn a_refused_flag_before_the_target_cannot_turn_gate_json_into_a_source_file_delete() {
    // SOUNDNESS R264 — DATA LOSS, and the worst thing in this file. `-V` was in the main loop's
    // valueless set and NOT in `prescan_argv`'s copy, so it set `stopped`, the target was suppressed,
    // `pre_target` fell back to `"."`, and `gate_json_input_collision` was asked "is this sink under the
    // CWD?" instead of "is it under the target?". From an unrelated cwd the answer is no, arming
    // proceeded, and the fail-closed verdict document was written OVER the source file at parse time.
    //
    //     candor-scan --version <dir> --gate-json <dir>/src/lib.rs   exit 2, intact
    //     candor-scan -V        <dir> --gate-json <dir>/src/lib.rs   exit 0, DESTROYED
    //
    // Two fixes, and the test asserts both: one VALUELESS_FLAGS list instead of two hand-written copies
    // (that closes `-V`), and a target-independent refusal for a source-shaped sink when the target is
    // unknown (that closes every OTHER refusable leading token, which the single list cannot reach —
    // an unrecognised flag legitimately suppresses the target).
    //
    // The cwd matters and is why this bit me once: on macOS `/tmp` is a symlink to `/private/tmp`, so
    // running from `/tmp` puts a scratch sink UNDER the cwd and the old guard sees it. This test uses a
    // cwd unrelated to both.
    let d = make_crate("dataloss", "pub fn go() { let _ = std::fs::read(\"x\"); }");
    let src = d.join("src").join("lib.rs");
    let original = std::fs::read(&src).expect("fixture source readable");
    // The cwd must NOT be an ancestor of the sink, or the OLD guard sees it and the test proves
    // nothing. `make_crate` builds under the system temp dir, so `temp_dir()` is exactly the wrong
    // choice here — I wrote that first and the test passed against the unfixed binary. The crate root
    // is on a different branch of the filesystem from the fixture.
    let elsewhere = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).to_path_buf();

    for leading in ["--version", "-V", "--zzz-not-a-flag", "--scope"] {
        std::fs::write(&src, &original).unwrap();
        let out = Command::new(bin())
            .current_dir(&elsewhere)
            .args([leading, d.to_string_lossy().as_ref(), "--gate-json", src.to_string_lossy().as_ref()])
            .output()
            .expect("run candor-scan");
        assert_eq!(
            std::fs::read(&src).unwrap(),
            original,
            "{leading}: --gate-json overwrote a SOURCE FILE of the scan (exit {:?})",
            out.status.code()
        );
    }

    // CONTROL, and it is the reason the fix is not simply "never arm without a target": a legitimate
    // non-source sink must STILL be armed behind a refusable flag, or the ⟨0.27⟩ stale green comes back.
    let sink = d.join("verdict.json");
    std::fs::write(&sink, br#"{"ok":true,"note":"YESTERDAYS GREEN"}"#).unwrap();
    let out = Command::new(bin())
        .current_dir(&elsewhere)
        .args(["--zzz-not-a-flag", d.to_string_lossy().as_ref(),
               "--gate-json", sink.to_string_lossy().as_ref()])
        .output()
        .expect("run candor-scan");
    assert_eq!(out.status.code(), Some(2), "an unknown flag still refuses");
    let got = std::fs::read_to_string(&sink).unwrap();
    assert!(!got.contains("YESTERDAYS GREEN"),
            "a non-source sink must still be armed behind a refused flag:\n{got}");
}

#[test]
fn an_unknown_flag_arms_the_gate_sink_in_either_argv_order() {
    // ⟨0.27⟩ (1) — the verdict sink is armed with the refusal WHATEVER the argv order, so a reader of
    // the sink can never be handed yesterday's `{"ok": true}` by a run that refused. `prescan_argv`
    // therefore collects `--gate-json` PAST a token the parse loop would refuse; that over-collection
    // is deliberate and load-bearing.
    //
    // The first version of the R232 fix stopped the target walk with a `break`, which also stopped that
    // collection: `--zzz --gate-json G` left the stale green at G, and only the ORDER
    // `--gate-json G --zzz` still armed. Conformance PART 34 (b) caught it — "the contract depends on
    // ARGV ORDER" — and nothing in this crate did. It does now.
    for (label, args) in [
        ("bad flag FIRST", vec!["--zzz-not-a-flag", "--gate-json"]),
        ("bad flag LAST", vec!["--gate-json"]),
    ] {
        let d = make_crate("armorder", "pub fn go() {}");
        let sink = d.join("verdict.json");
        std::fs::write(&sink, br#"{"ok":true,"note":"YESTERDAYS GREEN"}"#).unwrap();

        let mut cmd = Command::new(bin());
        cmd.arg(d.to_string_lossy().as_ref());
        for a in &args { cmd.arg(a); }
        cmd.arg(sink.to_string_lossy().as_ref());
        if label == "bad flag LAST" { cmd.arg("--zzz-not-a-flag"); }
        let out = cmd.output().expect("run candor-scan");

        assert_eq!(out.status.code(), Some(2), "{label}: an unknown flag refuses");
        let got = std::fs::read_to_string(&sink).expect("sink readable");
        assert!(
            !got.contains("YESTERDAYS GREEN"),
            "{label}: the refusing run left the previous green at the sink:\n{got}"
        );
    }
}

#[test]
fn two_positionals_mark_the_first_which_is_the_one_the_refusal_names() {
    // The main loop refuses a second positional and its message names the FIRST as the target it
    // holds. The prescan used to take the last "to mirror this loop" — a mirror that went stale when
    // the loop stopped letting the last one win, so the marker landed under a directory the run had
    // already refused to scan.
    let a = make_crate("twoposa", "pub fn go() {}");
    let b = make_crate("twoposb", "pub fn go() {}");
    let out = Command::new(bin())
        .args([a.to_string_lossy().as_ref(), b.to_string_lossy().as_ref()])
        .output()
        .expect("run candor-scan");
    assert_eq!(out.status.code(), Some(2), "two targets is a usage error");
    assert!(
        a.join(".candor").join("report.refused.json").exists(),
        "the marker goes to the FIRST positional, the one the refusal message names"
    );
    assert!(
        !b.join(".candor").join("report.refused.json").exists(),
        "the second positional was refused, so nothing may be written under it"
    );
}

#[test]
fn a_refusal_over_an_absent_target_writes_nothing_while_an_existing_one_still_gets_the_marker() {
    // SOUNDNESS R520 — TWO ARMS, AND THE SECOND IS THE ONE THAT MATTERS MORE.
    //
    // ARM A (the defect): `candor-scan nonexistent-target /also-bogus` exits 2 correctly and used to
    // leave `./nonexistent-target/.candor/report.refused.json` behind — the refusal marker's
    // `create_dir_all` MADE a directory tree in the operator's CWD, named after a mistyped argument.
    // Found as litter in this family's own repo: an agent mistyped a scan invocation and left a
    // `gate/` directory nothing in the umbrella expects. Measured four-way at the time: rust and ts
    // created it, java and swift created nothing, all four exit 2 — only the filesystem effect
    // differed, which is why no clause changed. Both of the everyday refusal spellings are driven
    // here (a second positional, and an unknown flag), because they refuse at different sites.
    //
    // ARM B (the control, and the expensive direction to get wrong): a target that EXISTS and holds a
    // previous run's report must STILL be shadowed by the marker. That is the whole of ⟨0.32⟩ — scan
    // a tree green, refuse for any reason, and without the marker `gate --report` answers `policy ✓`
    // off the previous run's bytes. A fix that silenced this sink would be far worse than the litter,
    // so the arm asserts the CONSUMER resolves the marker, not merely that a file exists.
    //
    // The two arms differ in exactly one thing: whether the target directory is on disk.

    // ── ARM A: nothing on disk, nothing written ──────────────────────────────────────────────────
    let cwd = std::env::temp_dir().join(format!("candor-r520-cwd-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&cwd);
    std::fs::create_dir_all(&cwd).unwrap();
    for argv in [
        vec!["nonexistent-target", "/also-bogus"],
        vec!["nonexistent-target", "--zzz-not-a-flag"],
        vec!["nope/deeper/still", "/also-bogus"],
    ] {
        let out = Command::new(bin())
            .current_dir(&cwd)
            .args(&argv)
            .output()
            .expect("run candor-scan");
        assert_eq!(out.status.code(), Some(2), "{argv:?}: a usage error still refuses at exit 2");
        let left: Vec<String> = std::fs::read_dir(&cwd)
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        assert!(
            left.is_empty(),
            "{argv:?}: a refusal over a target that is not there created {left:?} in the operator's \
             CWD — the marker shadows a previous run's report, and an absent target cannot have one"
        );
    }
    let _ = std::fs::remove_dir_all(&cwd);

    // ── ARM B: the target exists and holds a report — the marker MUST still land ──────────────────
    let d = make_crate("r520present", "pub fn go() {}");
    let prefix = format!("{}/.candor/report", d.to_string_lossy());

    // A real previous run, so there is something for a stale read to find.
    let first = Command::new(bin())
        .arg(d.to_string_lossy().as_ref())
        .output()
        .expect("run candor-scan");
    assert_eq!(first.status.code(), Some(0), "the seeding scan must complete");
    let reports: Vec<PathBuf> = std::fs::read_dir(d.join(".candor"))
        .expect("the completing run wrote no .candor/ — arm B would prove nothing")
        .map(|e| e.unwrap().path())
        .filter(|p| p.file_name().is_some_and(|n| {
            let n = n.to_string_lossy();
            n.starts_with("report.") && n.ends_with(".json") && n != "report.refused.json"
        }))
        .collect();
    assert!(!reports.is_empty(), "no previous report to shadow: arm B is asserting about nothing");
    let before: Vec<Vec<u8>> = reports.iter().map(|p| std::fs::read(p).unwrap()).collect();
    assert!(
        candor_report::refusal_marker_for(&prefix).is_none(),
        "a completing run left a marker behind"
    );

    // …now refuse, by the same everyday spelling arm A drove.
    let out = Command::new(bin())
        .args([d.to_string_lossy().as_ref(), "--zzz-not-a-flag"])
        .output()
        .expect("run candor-scan");
    assert_eq!(out.status.code(), Some(2), "an unknown flag still refuses");
    assert!(
        d.join(".candor").join("report.refused.json").exists(),
        "the fail-closed marker did NOT land beside a real report set — a `gate --report` here now \
         answers off the previous run's bytes, which is the stale green ⟨0.32⟩ exists to close"
    );
    let m = candor_report::refusal_marker_for(&prefix)
        .expect("the CONSUMER does not see the marker: the sink is silenced in substance");
    assert!(
        m.reason.contains("zzz-not-a-flag"),
        "the marker must name the cause that stopped the run: {m:?}"
    );
    // …and it SHADOWS the reports rather than destroying them (⟨0.28⟩'s own review found this rung
    // destroying user files four-way; the marker's charter is that it overwrites nothing).
    for (p, was) in reports.iter().zip(before.iter()) {
        assert_eq!(&std::fs::read(p).unwrap(), was,
                   "the refusal rewrote {} — the marker destroys nothing", p.display());
    }
    let _ = std::fs::remove_dir_all(&d);
}

// ── adversarial inputs: no panic, clean handling ──────────────────────────────────────────────────

#[test]
fn corrupt_random_bytes_source_does_not_panic() {
    // A crate whose source is random bytes (not valid UTF-8/Rust): the scan must HANDLE it (no panic /
    // SIGABRT — exit code is never 101), and a --json run still emits parseable JSON. With a gate it
    // must exit 2 (parse failure → gate cannot be green), never 0.
    let d = std::env::temp_dir().join(format!("candor-scan-cli-randbytes-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(d.join("src")).unwrap();
    std::fs::write(d.join("Cargo.toml"), "[package]\nname = \"randbytes\"\n").unwrap();
    // Deterministic non-UTF8/garbage bytes — no RNG dependency.
    let garbage: Vec<u8> = (0u16..2048).map(|i| (i.wrapping_mul(37) ^ 0xA5) as u8).collect();
    std::fs::write(d.join("src/lib.rs"), &garbage).unwrap();

    let out = Command::new(bin())
        .arg(d.to_string_lossy().as_ref())
        .arg("--json")
        .output()
        .expect("run candor-scan");
    assert_ne!(out.status.code(), Some(101), "a random-bytes source must not panic the scanner");
    let stdout = String::from_utf8_lossy(&out.stdout);
    if !stdout.trim().is_empty() {
        assert!(serde_json::from_str::<serde_json::Value>(stdout.trim()).is_ok(),
                "--json over a garbage source must still emit valid JSON:\n{stdout}");
    }

    let pp = d.join("candor.policy");
    std::fs::write(&pp, "deny Exec\n").unwrap();
    let out = Command::new(bin())
        .arg(d.to_string_lossy().as_ref())
        .arg("--policy")
        .arg(pp.to_string_lossy().as_ref())
        .output()
        .expect("run candor-scan");
    let _ = std::fs::remove_dir_all(&d);
    assert_eq!(out.status.code(), Some(2), "a gate over an unparseable garbage source must exit 2, never green");
}

#[test]
fn empty_dir_scan_is_clean_exit_0() {
    // A directory with no Cargo.toml / no sources: no crash, exit 0, and --json emits valid JSON
    // (an empty `functions` list). The package name falls back to "crate".
    let d = std::env::temp_dir().join(format!("candor-scan-cli-emptydir-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    let out = Command::new(bin())
        .arg(d.to_string_lossy().as_ref())
        .arg("--json")
        .output()
        .expect("run candor-scan");
    let _ = std::fs::remove_dir_all(&d);
    assert_eq!(out.status.code(), Some(0), "an empty dir must scan cleanly (exit 0)");
    let stdout = String::from_utf8(out.stdout).expect("utf8 stdout");
    assert!(serde_json::from_str::<serde_json::Value>(stdout.trim()).is_ok(),
            "--json over an empty dir must emit valid JSON:\n{stdout}");
}

#[test]
fn nonexistent_path_does_not_panic() {
    // A path that does not exist must be handled, not panic (exit code never 101).
    let missing = std::env::temp_dir().join(format!("candor-scan-cli-no-such-{}-xyz", std::process::id()));
    let _ = std::fs::remove_dir_all(&missing);
    let out = Command::new(bin())
        .arg(missing.to_string_lossy().as_ref())
        .arg("--json")
        .output()
        .expect("run candor-scan");
    assert_ne!(out.status.code(), Some(101), "a nonexistent path must not panic the scanner");
}

#[test]
fn gate_json_writes_the_structured_verdict_faithful_to_the_exit_code() {
    // --gate-json (candor-spec §3.3 ⟨0.8⟩): the machine verdict { spec, ok, violations:[{rule,fn,effects,
    // detail}] }, from the SAME gate that sets the exit code. Verified on a violating crate (exit 1).
    let d = make_crate("gatejson", "pub fn go() { std::process::Command::new(\"sh\").status().unwrap(); }");
    let pp = d.join("candor.policy");
    std::fs::write(&pp, "deny Exec\n").unwrap();
    let gp = d.join("gate.json");

    let out = Command::new(bin())
        .arg(d.to_string_lossy().as_ref())
        .arg("--policy").arg(pp.to_string_lossy().as_ref())
        .arg("--gate-json").arg(gp.to_string_lossy().as_ref())
        .output()
        .expect("run candor-scan");
    assert_eq!(out.status.code(), Some(1), "a deny-Exec violation must exit 1");

    let verdict: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&gp).expect("gate.json written")).expect("valid JSON");
    let _ = std::fs::remove_dir_all(&d);

    assert_eq!(verdict["spec"], candor_report::SPEC_VERSION, "verdict declares the spec version");
    assert_eq!(verdict["ok"], false, "ok:false on a failing gate");
    let viols = verdict["violations"].as_array().expect("violations array");
    assert_eq!(viols.len(), 1, "one violation: {verdict}");
    assert_eq!(viols[0]["rule"], "AS-EFF-006");
    assert_eq!(viols[0]["fn"], "go");
    assert_eq!(viols[0]["effects"], serde_json::json!(["Exec"]), "effects = the denied set");
}

#[test]
fn gate_json_valueless_fails_closed() {
    let d = make_crate("gatejsonnoval", "pub fn go() {}");
    let out = Command::new(bin())
        .arg(d.to_string_lossy().as_ref())
        .arg("--gate-json")
        .output()
        .expect("run candor-scan");
    let _ = std::fs::remove_dir_all(&d);
    assert_eq!(out.status.code(), Some(2), "a valueless --gate-json must fail (exit 2)");
}

#[test]
fn a_violation_dominates_incomplete_in_either_member_order_and_without_a_sink() {
    // ⟨0.30⟩ TWO defects with one symptom, neither of which had a row.
    //
    // (1) Member exit codes aggregated with `rc.max(code)`, and 2 > 1 — so one member's "could not
    //     evaluate" displaced another member's CERTAIN violation, against §3.3's "a real violation
    //     (exit 1) still dominates". Which member won depended on the WALK ORDER, so both orders are
    //     asserted here: a row that only tried one would have passed throughout.
    //
    // (2) The precedence check read a violation record that was only populated when `--gate-json` was
    //     requested, so the exit code differed with and without a machine sink. An output channel must
    //     never decide a verdict, so every case is run both ways and the codes compared.
    for (first, second) in [("a_viol", "z_bad"), ("a_bad", "z_viol")] {
        let d = std::env::temp_dir()
            .join(format!("candor-scan-cli-prec-{}-{first}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(d.join(first).join("src")).unwrap();
        std::fs::create_dir_all(d.join(second).join("src")).unwrap();
        std::fs::write(d.join("Cargo.toml"),
            format!("[workspace]\nmembers = [\"{first}\", \"{second}\"]\n")).unwrap();
        // one member holds a CERTAIN violation, the other cannot be fully analysed
        let (viol, bad) = if first.ends_with("viol") { (first, second) } else { (second, first) };
        std::fs::write(d.join(viol).join("Cargo.toml"),
            format!("[package]\nname = \"{viol}\"\n")).unwrap();
        std::fs::write(d.join(viol).join("src/lib.rs"),
            "pub fn fetch() { let _ = std::net::TcpStream::connect(\"x:80\"); }\n").unwrap();
        std::fs::write(d.join(bad).join("Cargo.toml"),
            format!("[package]\nname = \"{bad}\"\n")).unwrap();
        std::fs::write(d.join(bad).join("src/lib.rs"), "pub fn ok() -> u32 { 1 }\n").unwrap();
        std::fs::write(d.join(bad).join("src/broken.rs"), "pub fn x( {{{\n").unwrap();
        let pp = d.join("candor.policy");
        std::fs::write(&pp, "deny Net\n").unwrap();

        let gp = d.join("gate.json");
        let with_sink = Command::new(bin())
            .arg(d.to_string_lossy().as_ref())
            .arg("--policy").arg(pp.to_string_lossy().as_ref())
            .arg("--gate-json").arg(gp.to_string_lossy().as_ref())
            .output().expect("run candor-scan");
        let without_sink = Command::new(bin())
            .arg(d.to_string_lossy().as_ref())
            .arg("--policy").arg(pp.to_string_lossy().as_ref())
            .output().expect("run candor-scan");

        let verdict: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&gp).expect("gate.json")).expect("JSON");
        let _ = std::fs::remove_dir_all(&d);

        assert_eq!(with_sink.status.code(), Some(1),
            "members [{first}, {second}]: a certain violation must dominate an incomplete member \
             whichever order they are walked in");
        assert_eq!(without_sink.status.code(), with_sink.status.code(),
            "members [{first}, {second}]: the exit code changed with --gate-json — a machine sink is an \
             output channel and must not decide a verdict");
        assert_eq!(verdict["ok"], false, "ok must agree with the exit code");
        assert!(!verdict["violations"].as_array().unwrap().is_empty(),
            "the verdict must CARRY the violation it exited 1 for, not just report incompleteness");
    }
}

#[test]
fn gate_json_workspace_accumulates_across_members() {
    // The workspace bug the spec review caught: the gate runs per member, and a per-member verdict write
    // let a clean LAST member overwrite an earlier violator's — gate.json said ok:true while the process
    // exited 1, violating §3.3's "the verdict MUST agree with the exit code". Members must ACCUMULATE.
    let d = std::env::temp_dir().join(format!("candor-scan-cli-gatews-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(d.join("a_viol/src")).unwrap();
    std::fs::create_dir_all(d.join("z_clean/src")).unwrap();
    std::fs::write(d.join("Cargo.toml"), "[workspace]\nmembers = [\"a_viol\", \"z_clean\"]\n").unwrap();
    std::fs::write(d.join("a_viol/Cargo.toml"), "[package]\nname = \"a_viol\"\n").unwrap();
    std::fs::write(d.join("a_viol/src/lib.rs"), "pub fn fetch() { let _ = std::net::TcpStream::connect(\"x:80\"); }\n").unwrap();
    std::fs::write(d.join("z_clean/Cargo.toml"), "[package]\nname = \"z_clean\"\n").unwrap();
    std::fs::write(d.join("z_clean/src/lib.rs"), "pub fn add(a: i32, b: i32) -> i32 { a + b }\n").unwrap();
    let pp = d.join("candor.policy");
    std::fs::write(&pp, "deny Net\n").unwrap();
    let gp = d.join("gate.json");

    let out = Command::new(bin())
        .arg(d.to_string_lossy().as_ref())
        .arg("--policy").arg(pp.to_string_lossy().as_ref())
        .arg("--gate-json").arg(gp.to_string_lossy().as_ref())
        .output()
        .expect("run candor-scan");
    assert_eq!(out.status.code(), Some(1), "the violating member fails the workspace gate");

    let verdict: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&gp).expect("gate.json written")).expect("valid JSON");
    let _ = std::fs::remove_dir_all(&d);

    assert_eq!(verdict["ok"], false,
        "ok must agree with exit 1 — the clean last member must NOT overwrite the violator's verdict");
    let viols = verdict["violations"].as_array().expect("violations array");
    assert_eq!(viols.len(), 1, "the a_viol violation survives to the final verdict: {verdict}");
    assert_eq!(viols[0]["fn"], "fetch");
    assert_eq!(viols[0]["effects"], serde_json::json!(["Net"]));
}

#[test]
fn gate_json_rejects_a_flag_shaped_value_and_dash_stays_pure() {
    // `--gate-json --policy pol` must fail (exit 2) — it used to swallow `--policy` as the verdict path
    // and let the displaced `pol` REPLACE the scan dir: gateless exit-0 over the wrong target.
    let d = make_crate("gatejsondash", "pub fn go() { std::process::Command::new(\"sh\").status().unwrap(); }");
    let pp = d.join("candor.policy");
    std::fs::write(&pp, "deny Exec\n").unwrap();
    let out = Command::new(bin())
        .arg(d.to_string_lossy().as_ref())
        .arg("--gate-json").arg("--policy").arg(pp.to_string_lossy().as_ref())
        .output().expect("run candor-scan");
    assert_eq!(out.status.code(), Some(2), "a flag-shaped --gate-json value fails closed");

    // `--gate-json -` streams the verdict to stdout, which must be PURE JSON (AS-EFF lines → stderr).
    let out = Command::new(bin())
        .arg(d.to_string_lossy().as_ref())
        .arg("--policy").arg(pp.to_string_lossy().as_ref())
        .arg("--gate-json").arg("-")
        .output().expect("run candor-scan");
    let _ = std::fs::remove_dir_all(&d);
    assert_eq!(out.status.code(), Some(1));
    let stdout = String::from_utf8(out.stdout).unwrap();
    let v: serde_json::Value = serde_json::from_str(stdout.trim()).expect("stdout is pure verdict JSON");
    assert_eq!(v["ok"], false);
    let stderr = String::from_utf8(out.stderr).unwrap();
    assert!(stderr.contains("AS-EFF-006"), "the human AS-EFF line goes to stderr: {stderr}");
}

#[test]
fn flag_shaped_policy_value_is_refused_and_the_swallowed_sink_still_gets_the_document() {
    // Conformance §3.1 (b13), SPEC §3.2 ⟨0.28⟩ "given no value" ruling. `--policy --gate-json -`:
    // the loop used to consume `--gate-json` as the policy FILENAME, so the verdict sink the operator
    // named was never a sink — measured on this engine as exit 2 with NOTHING on the stream where the
    // fail-closed refusal document belongs. A flag-shaped token after a value-taking flag is a usage
    // error at exit 2, and the sinks named elsewhere in that argv are STILL SINKS: the run has a
    // broken command line, not a redefined one. BOTH halves are asserted — the exit-code half alone
    // passes against the broken behaviour, which also exited 2.
    let d = make_crate("polflagval", "pub fn go() {}");
    let out = Command::new(bin())
        .arg(d.to_string_lossy().as_ref())
        .arg("--policy").arg("--gate-json").arg("-")
        // The conformance row runs env-scrubbed (`env -u …`); a CANDOR_POLICY in the harness
        // environment must not turn this into a different run.
        .env_remove("CANDOR_POLICY").env_remove("CANDOR_CONFIG").env_remove("CANDOR_BASELINE")
        .output().expect("run candor-scan");
    assert_eq!(out.status.code(), Some(2), "a flag-shaped --policy value is a usage error");
    let stdout = String::from_utf8(out.stdout).unwrap();
    let doc: serde_json::Value = serde_json::from_str(stdout.trim())
        .unwrap_or_else(|_| panic!("the `--gate-json -` stream sink must carry the refusal document \
                                    (it was swallowed as the policy filename), got stdout:\n{stdout}"));
    assert_eq!(doc["ok"], false, "fail-closed to a naive reader: {doc}");
    assert_eq!(doc["refused"], true, "a refusal, not a verdict: {doc}");
    let stderr = String::from_utf8(out.stderr).unwrap();
    assert!(stderr.contains("--policy") && stderr.contains("--gate-json"),
            "stderr names the flag given no value AND the token that is not one: {stderr}");

    // The FILE spelling of the same sink: armed by the pre-pass even though it appears after the
    // broken flag, so the refusal replaces any previous run's green rather than leaving it current.
    let gp = d.join("verdict.json");
    std::fs::write(&gp, "{\"ok\": true}\n").unwrap(); // a previous run's green — must not survive
    let out = Command::new(bin())
        .arg(d.to_string_lossy().as_ref())
        .arg("--policy").arg("--gate-json").arg(gp.to_string_lossy().as_ref())
        .env_remove("CANDOR_POLICY").env_remove("CANDOR_CONFIG").env_remove("CANDOR_BASELINE")
        .output().expect("run candor-scan");
    assert_eq!(out.status.code(), Some(2));
    let doc: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&gp).expect("sink written")).expect("valid JSON");
    let _ = std::fs::remove_dir_all(&d);
    assert_eq!(doc["ok"], false, "the stale green was replaced by the refusal: {doc}");
    assert_eq!(doc["refused"], true, "{doc}");
}

#[test]
fn candor_config_drives_the_gate_env_overrides_and_typo_fails_closed() {
    // .candor/config (candor-spec §config): the checked-in floor under the env vars.
    let d = make_crate("cfggate", "pub fn go() { std::process::Command::new(\"sh\").status().unwrap(); }");
    std::fs::create_dir_all(d.join(".candor")).unwrap();
    let deny_exec = d.join("deny-exec.policy");
    std::fs::write(&deny_exec, "deny Exec\n").unwrap();
    let deny_net = d.join("deny-net.policy");
    std::fs::write(&deny_net, "deny Net\n").unwrap();
    std::fs::write(d.join(".candor/config"),
        format!("policy {}\npolcy typo\n", deny_exec.display())).unwrap();

    // (a) the config drives the gate — no flag, no env — discovered via the target's ancestors.
    let out = Command::new(bin()).arg(d.to_string_lossy().as_ref()).output().expect("run");
    assert_eq!(out.status.code(), Some(1), "the config-supplied deny-Exec gates the scan");
    let stderr = String::from_utf8(out.stderr).unwrap();
    assert!(stderr.contains("unknown config key 'polcy'"), "typo protection warns: {stderr}");

    // (b) the env overrides the config (a passing deny-Net wins over the config's deny-Exec).
    let out = Command::new(bin()).arg(d.to_string_lossy().as_ref())
        .env("CANDOR_POLICY", deny_net.to_string_lossy().as_ref())
        .output().expect("run");
    assert_eq!(out.status.code(), Some(0), "CANDOR_POLICY env overrides the config");

    // (c) a set-but-unusable CANDOR_CONFIG fails closed.
    let out = Command::new(bin()).arg(d.to_string_lossy().as_ref())
        .env("CANDOR_CONFIG", d.join("no-such").to_string_lossy().as_ref())
        .output().expect("run");
    let _ = std::fs::remove_dir_all(&d);
    assert_eq!(out.status.code(), Some(2), "a typo'd CANDOR_CONFIG must fail closed");
}

#[test]
fn kappa_ledger_honors_an_empty_chained_report_as_coverage() {
    // SPEC §2 chaining rule 3 / §7.14: a dependency covered by a CHAINED report is exempt from the
    // κ ledger — INCLUDING an EMPTY report ({functions: []}, package field intact), which is that
    // crate's all-pure purity CLAIM, not a blind spot. Found live: the exemption was keyed on the
    // filename shape + entry hashes, so an empty report still drew a "classifier doesn't cover 1 dependency…" line
    // (candor-java/candor-ts stay correctly quiet on the same shape).
    //
    // ⟨0.24⟩ RE-POINTED, NOT DELETED, and the reason is written down because the edit LOOKS like a
    // weakening. This test's subject is that the ENVELOPE `package` field carries coverage on its own —
    // independent of the filename and of any join firing — and that subject is unchanged. What changed is
    // the fixture it makes the point with: an empty report is a purity claim only when its ⟨0.21⟩
    // manifest says something WAS judged, so the claim arm now carries `analyzed.count: 2` and the two
    // arms that do NOT make a claim (count 0, and the manifest-less pre-⟨0.21⟩ form of SPEC §2's third
    // row) sit beside it as their own rows. SPEC §2 ⟨0.24⟩ names this retirement explicitly: "An engine
    // carrying such a pin should re-point it at a manifest-bearing fixture rather than delete it."
    let d = std::env::temp_dir().join(format!("candor-scan-cli-kappaempty-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(d.join("src")).unwrap();
    std::fs::write(d.join("Cargo.toml"),
        "[package]\nname = \"kappaledger\"\n\n[dependencies]\ndepc = \"1\"\n").unwrap();
    std::fs::write(d.join("src/lib.rs"), "pub fn use_dep() { depc::hit(); }\n").unwrap();
    // The empty depc reports, all named OUTSIDE the `….<crate>.scan.json` shape — the envelope's
    // `package` field alone must carry (or withhold) the coverage claim. The three differ in the
    // ⟨0.21⟩ manifest and in NOTHING else, which is what makes them a control set.
    let write_rep = |name: &str, manifest: &str| -> std::path::PathBuf {
        let p = d.join(name);
        std::fs::write(&p, format!(r#"{{
            "candor": {{"version": "scan-{}", "toolchain": "stable", "spec": "0.27"}},
            "package": "depc", {manifest}
            "functions": []}}"#, env!("CARGO_PKG_VERSION"))).unwrap();
        p
    };
    let claim = write_rep("depc-purity.json", r#""analyzed": {"count": 2, "digest": "0"},"#);
    let judged_nothing = write_rep("depc-facade.json", r#""analyzed": {"count": 0, "digest": "0"},"#);
    let no_manifest = write_rep("depc-legacy.json", "");

    // CONTROL (no chaining): the ledger fires — depc is a genuine blind spot. Its report is the
    // reference every other arm below is compared against, so it is captured, not just asserted on.
    let out = Command::new(bin()).arg(d.to_string_lossy().as_ref()).arg("--json")
        .output().expect("run candor-scan");
    let unchained_stderr = String::from_utf8(out.stderr).unwrap();
    let unchained: serde_json::Value =
        serde_json::from_str(String::from_utf8(out.stdout).unwrap().trim()).expect("pure JSON report");
    assert!(unchained_stderr.contains("classifier doesn't cover") && unchained_stderr.contains("depc"),
        "without chaining, the called-but-unknown dep must be disclosed: {unchained_stderr}");

    let run_chained = |rep: &std::path::Path| -> (i32, String, serde_json::Value) {
        let out = Command::new(bin()).arg(d.to_string_lossy().as_ref()).arg("--json")
            .env("CANDOR_DEPS", rep.to_string_lossy().as_ref())
            .output().expect("run candor-scan");
        let code = out.status.code().unwrap_or(-1);
        let err = String::from_utf8(out.stderr).unwrap();
        let v = serde_json::from_str(String::from_utf8(out.stdout).unwrap().trim()).expect("pure JSON");
        (code, err, v)
    };

    // (a) THE CLAIM — `analyzed.count: 2` with `functions: []` is a dependency that judged two units and
    //     found neither effectful. SPEC §2 rule 3 says BELIEVE it: no ledger line, and the join-less call
    //     reads pure. THIS IS THE CONTROL FOR (b): a "fix" keyed on `functions` being empty rather than on
    //     the integer would hedge here too, and would have disabled chained coverage rather than
    //     implemented ⟨0.24⟩.
    let (code, err, v) = run_chained(&claim);
    assert_eq!(code, 0);
    assert!(!err.contains("classifier doesn't cover"),
        "an empty chained report that JUDGED something is coverage — the ledger must stay quiet: {err}");
    assert!(!err.contains("judged NOTHING"), "…and it must not draw the ⟨0.24⟩ advisory either: {err}");
    assert!(v["functions"].as_array().unwrap().iter()
            .all(|f| f["fn"].as_str() != Some("use_dep")),
        "the call into the all-pure dep reads pure (omitted from the report): {v}");
    assert!(v.get("coverage").is_none(), "a covered dep leaves no κ ledger in the envelope: {v}");

    // (b) ⟨0.24⟩ THE FLOOR — `analyzed.count: 0` is "I judged nothing". The consumer must carry EXACTLY
    //     the disclosure the UNCHAINED arm carries: asserted as EQUALITY with that arm's report rather
    //     than against a literal, because "exactly as if it had not been chained" is what SPEC §2 states
    //     and a literal could drift away from the unchained reading without anything noticing.
    let (code, err, v) = run_chained(&judged_nothing);
    assert_eq!(code, 0, "a count-0 report adds a HEDGE, never a verdict — there is no effect to charge");
    assert_eq!(v["functions"], unchained["functions"],
        "⟨0.24⟩ a count-0 chained report bought MORE confidence than not chaining at all — the caller's \
         `invisible` disclosure is gone:\nchained={v:#}\nunchained={unchained:#}");
    assert_eq!(v["coverage"], unchained["coverage"],
        "…and the envelope's κ ledger with it:\nchained={v:#}\nunchained={unchained:#}");
    assert!(err.contains("judged NOTHING") && err.contains("depc"),
        "the withheld coverage must be EXPLAINED — nothing else on any channel says why a crate with a \
         chained report is being hedged: {err}");

    // (c) SPEC §2's THIRD ROW — no manifest at all (a pre-⟨0.21⟩ producer) and no entries. Nothing on the
    //     wire distinguishes "judged nothing" from "judged and found nothing", so it falls back to the
    //     unchained reading too. A deliberate behaviour change: this exact shape DID buy coverage before,
    //     and it was this test that pinned it.
    let (_, err, v) = run_chained(&no_manifest);
    assert_eq!(v["functions"], unchained["functions"],
        "a manifest-less empty report makes no ⟨0.21⟩ claim, so its silence cannot license one: {v:#}");
    assert!(err.contains("judged NOTHING"), "…and it is disclosed on the same channel: {err}");
    let _ = std::fs::remove_dir_all(&d);
}

/// SOUNDNESS R68(1): the drop-glue vein, one boundary over. `7af62f1`/`0576b8c` gave the IN-CRATE
/// construction authority position-independence across a CALL / STRUCT-LITERAL / bare VALUE PATH, with
/// an every-path escape gate — but the CROSS-CRATE marker at the old `collector.rs:1948` piggybacked on
/// unrelated lazy-static-forcing code and only got the right join key by ACCIDENT, for a 2-segment
/// written path. Measured pre-fix: `deplib::UnitGuard` (bare value path, 2 segments) and
/// `deplib::TupleGuard(1)` (tuple-struct CALL, also 2 segments) both charged; `deplib::Guard::new(1)`
/// (assoc-fn CALL, 3 segments — wrong key `"Guard::new"`) and `deplib::Guard { n: 1 }` (STRUCT LITERAL —
/// never reached the old code at all) were SILENT. Ground-truthed by real execution in a two-crate
/// `CANDOR_DEPS` chain outside this suite (the destructor appends to a log; call/return markers bracket
/// each spelling) before this fix was written; this test pins the same four spellings plus the
/// over-charge controls against a hand-written chained dep report so CI catches a regression cheaply.
#[test]
fn cross_crate_drop_glue_charges_every_construction_spelling() {
    let d = make_crate(
        "r68one",
        r#"
        pub fn spell_unit() { deplib::UnitGuard; }
        pub fn spell_call() { deplib::Guard::new(1); }
        pub fn spell_struct() { deplib::Guard { n: 1 }; }
        pub fn spell_tuple() { deplib::TupleGuard(1); }
        // OVER-CHARGE CONTROL A: constructs-and-RETURNS — the flate2 shape. Must stay PURE; the escape
        // gate (`escaping_ctors`) is crate-agnostic, so it must cover this cross-crate case exactly as
        // it covers the in-crate one, with no extra plumbing.
        pub fn direct_escape() -> deplib::Guard { deplib::Guard::new(2) }
        pub fn direct_escape_struct() -> deplib::Guard { deplib::Guard { n: 3 } }
        // CONTROL B: a value that escapes on ONE path and dies on the OTHER (the R69 shape) must stay
        // CHARGED — only an escape on EVERY terminal exit may suppress.
        pub fn conditional_escape(flag: bool) -> Option<deplib::Guard> {
            let g = deplib::Guard::new(4);
            if flag { Some(g) } else { None }
        }
        // OVER-CHARGE CONTROL C: a cross-crate type with NO Drop impl at all must stay ABSENT in every
        // spelling — the dep report below never mentions `Plain`, so a by_key miss must resolve to
        // nothing, never a fabricated disclosure.
        pub fn plain_call() { deplib::Plain::new(1); }
        pub fn plain_struct() { deplib::Plain { n: 1 }; }
        "#,
    );
    std::fs::write(
        d.join("Cargo.toml"),
        "[package]\nname = \"r68one\"\n\n[dependencies]\ndeplib = \"1\"\n",
    )
    .unwrap();
    let dep_report = d.join("deplib.json");
    std::fs::write(&dep_report, format!(r#"{{
        "candor": {{"version": "scan-{}", "toolchain": "stable", "spec": "0.34"}},
        "package": "deplib",
        "analyzed": {{"count": 4, "digest": "0"}},
        "functions": [
            {{"fn": "Guard::drop", "inferred": ["Fs"], "hash": "deplib#Guard::drop"}},
            {{"fn": "TupleGuard::drop", "inferred": ["Fs"], "hash": "deplib#TupleGuard::drop"}},
            {{"fn": "UnitGuard::drop", "inferred": ["Fs"], "hash": "deplib#UnitGuard::drop"}}
        ]}}"#, env!("CARGO_PKG_VERSION"))).unwrap();

    let out = Command::new(bin())
        .arg(d.to_string_lossy().as_ref())
        .arg("--json")
        .env("CANDOR_DEPS", dep_report.to_string_lossy().as_ref())
        .output()
        .expect("run candor-scan");
    let _ = std::fs::remove_dir_all(&d);

    let v: serde_json::Value =
        serde_json::from_str(String::from_utf8(out.stdout).unwrap().trim()).expect("pure JSON report");
    let charged = |name: &str| -> Option<Vec<String>> {
        v["functions"].as_array().unwrap().iter().find(|f| f["fn"] == name).map(|f| {
            f["inferred"].as_array().unwrap().iter().map(|e| e.as_str().unwrap().to_string()).collect()
        })
    };

    for spelling in ["spell_unit", "spell_call", "spell_struct", "spell_tuple"] {
        assert_eq!(charged(spelling), Some(vec!["Fs".to_string()]),
            "R68(1): {spelling} must charge the dependency's effectful Drop — got {:?}\n{v:#}",
            charged(spelling));
    }
    for pure_fn in ["direct_escape", "direct_escape_struct", "plain_call", "plain_struct"] {
        assert_eq!(charged(pure_fn), None,
            "over-charge control: {pure_fn} must stay PURE (omitted) — got {:?}\n{v:#}",
            charged(pure_fn));
    }
    assert_eq!(charged("conditional_escape"), Some(vec!["Fs".to_string()]),
        "SOME-path (not EVERY-path) escape must still CHARGE — the R69 shape, cross-crate edition: {v:#}");
}

/// SOUNDNESS R529 — A DISPATCH WHOSE IMPLEMENTOR IS WRITTEN INSIDE A BLOCK.
///
/// Every Pass A decl walk recurses through `Item::Mod` and nothing else, so `impl Backend for L`
/// written inside `fn register`'s body is in NO index. Pass B is the opposite (`rebind_self`/R175
/// exists because the collector DOES walk into bodies), so its `Net` is charged to `register` by
/// syntactic containment and `L::size` is never minted as a unit. The CHA universe therefore holds
/// every implementor except the one it cannot name — and with a module-level PURE implementor beside
/// it the dispatch resolves to that pure body and `term_size` claims purity outright.
///
/// PRE-FIX (candor-scan 0.39.1, `5e6843e`): `term_size` reads `inferred: []`, no `Unknown`, no
/// `invisible`. The ZERO-implementor case is not the subject — that already reads `Unknown`; what makes
/// this the ⟨0.39⟩ toggle one spelling over is that ADDING the pure `PureBackend` is what removes the
/// disclosure.
#[test]
fn r529_a_dispatch_with_a_block_nested_implementor_discloses_instead_of_certifying() {
    let d = make_crate(
        "r529local",
        r#"
        pub trait Backend { fn size(&self); }

        // the implementor the engine CAN name, and it is pure — this is what makes the silence silent
        pub struct PureBackend;
        impl Backend for PureBackend { fn size(&self) {} }

        pub fn register() -> Box<dyn Backend> {
            struct NetBackend;
            impl Backend for NetBackend {
                fn size(&self) { let _ = std::net::TcpStream::connect("example.com:80"); }
            }
            Box::new(NetBackend)
        }

        pub fn term_size(b: &dyn Backend) { b.size() }

        // OVER-CHARGE CONTROL — a SECOND trait, dispatched the same way, with no block-nested impl
        // anywhere. It must stay absent (pure), or the hedge is a blanket flood on every dispatch
        // rather than the named-fact narrowing it claims to be.
        pub trait Other { fn ping(&self); }
        pub struct OnlyImpl;
        impl Other for OnlyImpl { fn ping(&self) {} }
        pub fn other_dispatch(o: &dyn Other) { o.ping() }
        "#,
    );
    let out = Command::new(bin())
        .arg(d.to_string_lossy().as_ref())
        .arg("--json")
        .output()
        .expect("run candor-scan");
    let _ = std::fs::remove_dir_all(&d);
    let v: serde_json::Value =
        serde_json::from_str(String::from_utf8(out.stdout).unwrap().trim()).expect("pure JSON report");
    let row = |name: &str| -> Option<serde_json::Value> {
        v["functions"].as_array().unwrap().iter().find(|f| f["fn"] == name).cloned()
    };

    let ts = row("term_size").unwrap_or_else(|| panic!(
        "R529: `term_size` is ABSENT, which §2 rule 3 makes an affirmative PURITY CLAIM over a \
         dispatch whose implementor opens a TcpStream: {v:#}"));
    let inferred: Vec<&str> =
        ts["inferred"].as_array().unwrap().iter().map(|e| e.as_str().unwrap()).collect();
    assert!(inferred.contains(&"Unknown"),
        "R529: `term_size` must DISCLOSE — the engine cannot read the body-local implementor: {ts:#}");
    let why: Vec<&str> =
        ts["unknownWhy"].as_array().map(|a| a.iter().map(|e| e.as_str().unwrap()).collect())
            .unwrap_or_default();
    assert!(why.contains(&"dispatch:Backend.size"),
        "R529: §4's normative dotted `dispatch:<owner>.<member>` — the owner IS resolvable (the \
         trait), only the body is not. got {why:?}: {ts:#}");

    // The PUBLISHED union entry carries it too, so a CHAINED consumer is not told the union is complete.
    let u = v["functions"].as_array().unwrap().iter()
        .find(|f| f["hash"] == "r529local#Backend::size")
        .unwrap_or_else(|| panic!("R529: no interface-union entry for `Backend::size`: {v:#}"));
    assert_eq!(u["interfaceUnion"], serde_json::Value::Bool(true), "{u:#}");
    assert!(u["inferred"].as_array().unwrap().iter().any(|e| e == "Unknown"),
        "R529: the union is taken over the implementors this engine could NAME, so publishing it as \
         complete is the same purity claim one hop out: {u:#}");

    // …AND THE CONTROL STAYS CLEAN. Note the shape: `other_dispatch` is PRESENT, because ⟨0.39⟩
    // obligation 1 publishes a pure function that DISPATCHES. So the control is not "absent" — it is
    // "present, determined, and carrying no disclosure", which is the stronger assertion anyway: an
    // absence-shaped control here would have passed for the wrong reason (§E3).
    let other = row("other_dispatch").unwrap_or_else(|| panic!(
        "R529 control: ⟨0.39⟩ obligation 1 publishes a pure DISPATCHING row — its absence would mean \
         the control is measuring something else: {v:#}"));
    assert_eq!(other["inferred"].as_array().unwrap().len(), 0,
        "R529 over-charge control: a dispatch with NO block-nested implementor must stay DETERMINED \
         — the hedge is gated on a named fact, not on dispatching at all: {other:#}");
    assert!(other["unknownWhy"].is_null(), "{other:#}");
    assert!(other["unresolved"].is_null(), "{other:#}");
}

/// SOUNDNESS R529b — A BLOCK-NESTED `extern "C"` BLOCK LOSES THE FFI DISCLOSURE.
///
/// Same mechanism as R529, a different index: `collect_decls`'s `Item::ForeignMod` arm is item-level
/// like every other, so `fn wrap() { extern "C" { fn ffi(); } unsafe { ffi(); } }` records no name in
/// `extern_fns`, the bare leaf matches no local def and no classifier rule, and the caller falls through
/// to silent-pure. The MODULE-LEVEL spelling of the identical program discloses
/// `Unknown` + `native:extern fn`, so the two arms differ in exactly one thing: where the `extern` block
/// is written. That control is the whole evidence — without it, "absent" and "pure" are the same bytes.
#[test]
fn r529b_a_block_nested_extern_block_discloses_the_ffi_boundary() {
    let d = make_crate(
        "r529extern",
        r#"
        extern "C" { pub fn mod_ffi(x: i32) -> i32; }
        pub fn mod_wrapper() { unsafe { mod_ffi(1); } }

        pub fn body_wrapper() {
            extern "C" { fn body_ffi(x: i32) -> i32; }
            unsafe { body_ffi(1); }
        }

        // CONTROL: an ordinary local fn called the ordinary way must stay pure — the hedge is keyed on
        // a declared FFI name, not on being called from an `unsafe` block.
        fn plain(x: i32) -> i32 { x }
        pub fn plain_wrapper() { unsafe { plain(1); } }
        "#,
    );
    let out = Command::new(bin())
        .arg(d.to_string_lossy().as_ref())
        .arg("--json")
        .output()
        .expect("run candor-scan");
    let _ = std::fs::remove_dir_all(&d);
    let v: serde_json::Value =
        serde_json::from_str(String::from_utf8(out.stdout).unwrap().trim()).expect("pure JSON report");
    let row = |name: &str| v["functions"].as_array().unwrap().iter().find(|f| f["fn"] == name).cloned();

    for arm in ["mod_wrapper", "body_wrapper"] {
        let r = row(arm).unwrap_or_else(|| panic!(
            "R529b: `{arm}` is ABSENT, which is an affirmative purity claim over a call into a body \
             that is not in this language at all: {v:#}"));
        assert!(r["inferred"].as_array().unwrap().iter().any(|e| e == "Unknown"), "{r:#}");
        assert!(r["unknownWhy"].as_array()
                    .map(|a| a.iter().any(|w| w == "native:extern fn")).unwrap_or(false),
            "R529b: §4's canonical `native:` kind — the boundary is a foreign one: {r:#}");
    }
    assert!(row("plain_wrapper").is_none(),
        "R529b control: a plain local call inside `unsafe` must stay pure — the disclosure is keyed on \
         the declared FFI NAME, not on the `unsafe` block: {v:#}");
}

/// SOUNDNESS R529 — A MODULE DECLARED INSIDE A BODY IS WIDENED BY ITS OWN `use` MAP.
///
/// The LEAF is the same on both sides here (`Backend`), and only the `use` map distinguishes them: the
/// crate declares its own `Backend` at the root, and the body-local `mod inner` imports a DEPENDENCY's
/// `Backend` and implements that one. Expanding the body-local impl through the ENCLOSING scope's map
/// would file it under the local leaf and hedge `local_dispatch` — a key naming the wrong owner, which
/// is R6/R503's "two spellings for one abstraction" in the index that decides a disclosure.
///
/// So this is BOTH halves of one control: the foreign key is formed correctly, AND the local dispatch
/// that shares its leaf stays DETERMINED.
#[test]
fn r529_a_body_local_module_is_keyed_through_its_own_use_map() {
    let d = make_crate(
        "r529modscope",
        r#"
        pub trait Backend { fn size(&self); }
        pub struct Pure;
        impl Backend for Pure { fn size(&self) {} }

        pub fn register() {
            mod inner {
                use iface::backend::Backend;
                pub struct NetB;
                impl Backend for NetB {
                    fn size(&self) { let _ = std::net::TcpStream::connect("example.com:80"); }
                }
            }
            let _ = inner::NetB;
        }

        pub fn local_dispatch(b: &dyn Backend) { b.size() }
        "#,
    );
    std::fs::write(
        d.join("Cargo.toml"),
        "[package]\nname = \"r529modscope\"\n\n[dependencies]\niface = \"1\"\n",
    )
    .unwrap();
    let out = Command::new(bin())
        .arg(d.to_string_lossy().as_ref())
        .arg("--json")
        .output()
        .expect("run candor-scan");
    let _ = std::fs::remove_dir_all(&d);
    let v: serde_json::Value =
        serde_json::from_str(String::from_utf8(out.stdout).unwrap().trim()).expect("pure JSON report");

    let u = v["functions"].as_array().unwrap().iter()
        .find(|f| f["hash"] == "iface#backend::Backend::size")
        .unwrap_or_else(|| panic!(
            "R529: the body-local `mod inner`'s OWN `use` names the dependency's abstraction, so the \
             union entry must be published under `iface`: {v:#}"));
    assert!(u["inferred"].as_array().unwrap().iter().any(|e| e == "Unknown"), "{u:#}");

    let ld = v["functions"].as_array().unwrap().iter().find(|f| f["fn"] == "local_dispatch")
        .unwrap_or_else(|| panic!("⟨0.39⟩ obligation 1 publishes a pure dispatching row: {v:#}"));
    assert_eq!(ld["inferred"].as_array().unwrap().len(), 0,
        "R529: the crate's OWN `Backend` shares the leaf and has no block-nested impl — expanding the \
         body-local one through the enclosing scope would hedge this and name the wrong owner: {ld:#}");
    assert!(ld["unresolved"].is_null(), "{ld:#}");
}

/// SOUNDNESS R529, THE CHAINED HALF — SPEC §4 ⟨0.39⟩ obligation 3 with an implementor the consumer
/// supplies from inside a fn body.
///
/// The dependency declares the abstraction, dispatches on it, and is HONESTLY pure (its only visible
/// implementor is). The consumer's implementor is body-local, so `foreign_impls` holds no key for it
/// and the consumer-side join finds no contributor — the exact silence ⟨0.39⟩ closes for a module-level
/// implementor, reached by one the engine cannot NAME rather than one it cannot SEE. Pre-fix the
/// consumer's `app_size` is ABSENT from `functions[]` entirely.
#[test]
fn r529_a_chained_consumers_block_nested_implementor_is_disclosed_not_dropped() {
    let d = make_crate(
        "r529chain",
        r#"
        use iface::backend::Backend;
        pub fn register() -> Box<dyn Backend> {
            struct NetBackend;
            impl Backend for NetBackend {
                fn size(&self) { let _ = std::net::TcpStream::connect("example.com:80"); }
            }
            Box::new(NetBackend)
        }
        // never calls `register` — the containment charge on `register` is NOT what is under test
        pub fn app_size(b: &dyn Backend) { iface::term_size(b) }
        "#,
    );
    std::fs::write(
        d.join("Cargo.toml"),
        "[package]\nname = \"r529chain\"\n\n[dependencies]\niface = \"1\"\n",
    )
    .unwrap();
    // The dependency's own report: `term_size` is PURE and says it DISPATCHES (⟨0.39⟩ obligation 1).
    // Nothing here is wrong — the dependency really cannot see the consumer's implementor.
    let dep_report = d.join("iface.json");
    std::fs::write(&dep_report, format!(r#"{{
        "candor": {{"version": "scan-{}", "toolchain": "stable", "spec": "0.39"}},
        "package": "iface",
        "analyzed": {{"count": 2, "digest": "0"}},
        "functions": [
            {{"fn": "term_size", "inferred": [], "hash": "iface#term_size",
              "dispatchesOn": ["iface#backend::Backend::size"]}}
        ]}}"#, env!("CARGO_PKG_VERSION"))).unwrap();

    let out = Command::new(bin())
        .arg(d.to_string_lossy().as_ref())
        .arg("--json")
        .env("CANDOR_DEPS", dep_report.to_string_lossy().as_ref())
        .output()
        .expect("run candor-scan");
    let _ = std::fs::remove_dir_all(&d);
    let v: serde_json::Value =
        serde_json::from_str(String::from_utf8(out.stdout).unwrap().trim()).expect("pure JSON report");

    let app = v["functions"].as_array().unwrap().iter().find(|f| f["fn"] == "app_size")
        .unwrap_or_else(|| panic!(
            "R529 chained: `app_size` is ABSENT — a purity claim over a chained dispatch whose \
             implementor this very crate supplies: {v:#}"));
    assert!(app["inferred"].as_array().unwrap().iter().any(|e| e == "Unknown"),
        "R529 chained: the consumer must DISCLOSE what it supplied and could not read: {app:#}");
    assert!(app["unknownWhy"].as_array().map(|a| a.iter().any(|w| w == "dispatch:backend::Backend.size"))
            .unwrap_or(false),
        "R529 chained: the reason names the OWNING package's spelling of the member: {app:#}");

    // …and it REPUBLISHES the obligation-2 entry under the owner's key, so a package one hop further
    // out is told as well. `foreign_impls` has no key for a body-local impl, so pre-fix this entry did
    // not exist at all.
    let u = v["functions"].as_array().unwrap().iter()
        .find(|f| f["hash"] == "iface#backend::Backend::size")
        .unwrap_or_else(|| panic!("R529 chained: no foreign interface-union entry published: {v:#}"));
    assert_eq!(u["interfaceUnion"], serde_json::Value::Bool(true), "{u:#}");
    assert!(u["inferred"].as_array().unwrap().iter().any(|e| e == "Unknown"), "{u:#}");
}

/// A crate whose whole body is `calls` qualified calls spread over two DECLARED-but-unvendored
/// dependencies — the κ ledger's raw material, at an exact call VOLUME. Split over two deps so the
/// fixture also proves the trigger is the SUM, not the dependency count (2 either side of the line).
fn make_uncovered_caller(name: &str, calls: usize) -> PathBuf {
    let d = std::env::temp_dir().join(format!("candor-scan-cli-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(d.join("src")).unwrap();
    std::fs::write(d.join("Cargo.toml"),
        format!("[package]\nname = \"{name}\"\n\n[dependencies]\ndepa = \"1\"\ndepb = \"1\"\n")).unwrap();
    let mut body = String::from("pub fn go() {\n");
    for i in 0..calls {
        // Distinct leaf names: the count must come from CALL SITES, not from distinct paths happening
        // to coincide — a fixture that leaned on repetition could pass for the wrong reason.
        body.push_str(&format!("    {}::hit{i}();\n", if i % 2 == 0 { "depa" } else { "depb" }));
    }
    body.push_str("}\n");
    std::fs::write(d.join("src/lib.rs"), body).unwrap();
    d
}

#[test]
fn scan_completeness_nudge_keys_on_call_volume_at_an_exact_threshold() {
    // The scan-completeness nudge (candor-java parity, UNCOVERED_CALLS_NUDGE_MIN = 50): heavy CALL
    // VOLUME into κ-uncovered dependencies means the scan is missing an INPUT, not that the classifier
    // was imprecise — so say so, and name `--deps` as the remedy. ADVISORY ONLY.
    //
    // The boundary is pinned with LITERAL call counts, deliberately not derived from the constant: a
    // fixture built as "threshold" / "threshold - 1" would keep passing if the constant silently
    // drifted, which is exactly the regression this guards. If the constant moves, this test must be
    // edited on purpose.

    // JUST BELOW (49 calls over 2 deps): the ledger still discloses the blind spot, the nudge stays
    // silent. Volume, not COUNT — 2 uncovered deps is not itself evidence of a dependency-less scan.
    let d = make_uncovered_caller("nudgeunder", 49);
    let out = Command::new(bin()).arg(d.to_string_lossy().as_ref()).output().expect("run candor-scan");
    let _ = std::fs::remove_dir_all(&d);
    let stderr = String::from_utf8(out.stderr).unwrap();
    assert!(stderr.contains("classifier doesn't cover") && stderr.contains("depa (25 calls)"),
        "fixture precondition: 49 uncovered calls over 2 deps must reach the ledger: {stderr}");
    assert!(!stderr.contains("hint —"),
        "one call below the threshold the nudge must stay silent: {stderr}");

    // AT the threshold (50): the nudge fires, once, after the ledger line.
    let d = make_uncovered_caller("nudgeat", 50);
    let out = Command::new(bin()).arg(d.to_string_lossy().as_ref()).output().expect("run candor-scan");
    let stderr = String::from_utf8(out.stderr).unwrap();
    assert_eq!(stderr.matches("hint —").count(), 1, "exactly one nudge line: {stderr}");
    assert!(stderr.contains("50 calls go into 2 dependencies that are not scanned"),
        "the nudge reports the SUM of the ledger's call counts: {stderr}");
    assert!(stderr.find("classifier doesn't cover").unwrap() < stderr.find("hint —").unwrap(),
        "the nudge follows the ledger it is keyed on: {stderr}");
    // It promises VISIBILITY, never dispatch resolution — more dependency code cannot resolve a
    // dispatch over the crate's OWN broad trait hierarchy, so the wording must not imply it can.
    // Asserted on the NUDGE LINE alone, so neighbouring receipts can't satisfy or break it.
    let nudge = stderr.lines().find(|l| l.contains("hint —")).unwrap();
    assert!(nudge.contains("invisible here") && nudge.contains("--deps"),
        "the nudge names what is lost (visibility) and the remedy: {nudge}");
    assert!(!nudge.contains("dispatch") && !nudge.contains("Unknown"),
        "the nudge must not promise dispatch resolution: {nudge}");
    // ADVISORY: an ungated scan of pure-looking code still exits 0.
    assert_eq!(out.status.code(), Some(0), "the nudge must not move the exit code");

    // ...and it CANNOT contaminate a JSON stdout stream (`--json` — stdout stays one pure document).
    let out = Command::new(bin()).arg(d.to_string_lossy().as_ref()).arg("--json")
        .output().expect("run candor-scan");
    let _ = std::fs::remove_dir_all(&d);
    let stdout = String::from_utf8(out.stdout).unwrap();
    let stderr = String::from_utf8(out.stderr).unwrap();
    assert!(stderr.contains("hint —"), "the nudge still prints under --json: {stderr}");
    assert!(!stdout.contains("hint —"), "the advisory must never reach the JSON stream:\n{stdout}");
    serde_json::from_str::<serde_json::Value>(stdout.trim()).expect("stdout is one pure JSON document");
    assert_eq!(out.status.code(), Some(0), "the nudge must not move the exit code under --json");
}

#[test]
fn candor_config_relative_path_resolves_against_the_config_home_not_the_cwd() {
    // SPEC §3.4: a RELATIVE path value anchors to the config's HOME directory — the directory
    // CONTAINING `.candor/` (the repo root the config travels with) — never the process CWD (and not
    // the literal dirname of the config, which would break `policy .candor/gate.pol`). Run the scan
    // from an unrelated CWD: if resolution were CWD-based the policy would be unreadable (exit 2);
    // anchored correctly, the deny-Exec gate FIRES (exit 1).
    let d = make_crate("cfgrel", "pub fn go() { std::process::Command::new(\"sh\").status().unwrap(); }");
    std::fs::create_dir_all(d.join(".candor")).unwrap();
    std::fs::write(d.join(".candor/gate.pol"), "deny Exec\n").unwrap();
    std::fs::write(d.join(".candor/config"), "policy .candor/gate.pol\n").unwrap();
    let out = Command::new(bin())
        .arg(d.to_string_lossy().as_ref())
        .current_dir(std::env::temp_dir())
        .output().expect("run candor-scan");
    assert_eq!(out.status.code(), Some(1),
        "a home-relative `.candor/gate.pol` policy value must resolve and fire the gate");
    let stderr = String::from_utf8(out.stderr).unwrap();
    assert!(stderr.contains("AS-EFF-006") || String::from_utf8_lossy(&out.stdout).contains("AS-EFF-006"),
        "the deny-Exec violation must be reported: {stderr}");
    // …and a root-relative value (candor-init's scaffolded `policy arch.policy`) anchors there too.
    std::fs::write(d.join("arch.policy"), "deny Exec\n").unwrap();
    std::fs::write(d.join(".candor/config"), "policy arch.policy\n").unwrap();
    let out = Command::new(bin())
        .arg(d.to_string_lossy().as_ref())
        .current_dir(std::env::temp_dir())
        .output().expect("run candor-scan");
    let _ = std::fs::remove_dir_all(&d);
    assert_eq!(out.status.code(), Some(1),
        "a root-relative `arch.policy` value must resolve against the config home (gate fires)");
}

#[test]
fn candor_config_bare_policy_key_fails_loud() {
    // A configured-but-EMPTY policy (a bare `policy` line) means "enabled with the empty value" —
    // it must FAIL (exit 2, the unreadable-policy posture), never be silently skipped as falsy
    // (the declared-gate-silently-off class).
    let d = make_crate("cfgbarepol", "pub fn go() { std::process::Command::new(\"sh\").status().unwrap(); }");
    std::fs::create_dir_all(d.join(".candor")).unwrap();
    std::fs::write(d.join(".candor/config"), "policy\n").unwrap();
    let out = Command::new(bin()).arg(d.to_string_lossy().as_ref()).output().expect("run candor-scan");
    let _ = std::fs::remove_dir_all(&d);
    assert_eq!(out.status.code(), Some(2), "a bare `policy` config key must fail loud, never skip the gate");
}

#[test]
fn candor_config_recognized_but_unimplemented_key_warns_loudly() {
    // A checked-in strict/no-ambient/closed-world/taint key is spec-recognized but not wired to any
    // candor-scan mode — a DECLARED-GATE-SILENTLY-OFF unless disclosed. It must warn. (`baseline` used
    // to be on this list; it is now IMPLEMENTED — the AS-EFF-005 guard — and must NOT warn as inert.)
    let d = make_crate("cfginert", "pub fn pure() -> u32 { 1 }");
    std::fs::create_dir_all(d.join(".candor")).unwrap();
    std::fs::write(d.join(".candor/config"), "baseline .candor/baseline\ntaint true\nstrict 1\n").unwrap();
    // RECORD THE BASELINE FIRST. This assertion's own message used to read "an absent baseline is a
    // note, not a failure" — true until a baseline DECLARED in `.candor/config` became exit 2 (a
    // checked-in declaration says the repo HAS one, so an absent file was deleted or never committed).
    // The test was right about its intent — inert-key DISCLOSURE — and wrong about its premise, so it
    // now supplies a real baseline instead of relying on an absent one being harmless.
    Command::new(bin())
        .arg(d.to_string_lossy().as_ref())
        .args(["--out", d.join(".candor/baseline").to_string_lossy().as_ref()])
        .output()
        .expect("record the baseline");
    let out = Command::new(bin()).arg(d.to_string_lossy().as_ref()).output().expect("run candor-scan");
    let _ = std::fs::remove_dir_all(&d);
    assert_eq!(out.status.code(), Some(0), "inert keys don't fail the scan");
    let stderr = String::from_utf8(out.stderr).unwrap();
    assert!(stderr.contains("config key 'taint' is recognized by the candor family but not implemented by candor-scan"),
        "the inert `taint` key must be disclosed loudly: {stderr}");
    assert!(stderr.contains("config key 'strict'"), "every inert recognized key is disclosed: {stderr}");
    assert!(!stderr.contains("config key 'baseline'"),
        "`baseline` is implemented now — it must not be disclosed as inert: {stderr}");
    assert!(!stderr.contains("regression guard is not active"),
        "the baseline was recorded above, so the guard is ACTIVE — not the adopt note: {stderr}");
}

/// A baseline DECLARED in `.candor/config` but MISSING is exit 2, not a green pass.
///
/// An adopter review measured this as the second-likeliest first-commit mistake — `.candor/` committed,
/// the baseline not — and found every engine printing a note and exiting 0, so the gate quietly stopped
/// gating. THE SPLIT IS BY SOURCE and the sibling test below pins the other half: `CANDOR_BASELINE` is
/// set UNCONDITIONALLY by the adopt workflow, so an absent path there still means "the ratchet is not
/// adopted yet". Same absence, two meanings; only the source separates them.
#[test]
fn config_declared_baseline_that_is_missing_fails_closed() {
    let d = make_crate("blmissing", "pub fn pure() -> u32 { 1 }");
    std::fs::create_dir_all(d.join(".candor")).unwrap();
    std::fs::write(d.join(".candor/config"), "baseline .candor/nope\n").unwrap();
    let out = Command::new(bin()).arg(d.to_string_lossy().as_ref()).output().expect("run candor-scan");
    let stderr = String::from_utf8(out.stderr).unwrap();
    let _ = std::fs::remove_dir_all(&d);
    assert_eq!(out.status.code(), Some(2),
        "a checked-in declaration whose file is absent must not pass green: {stderr}");
    assert!(stderr.contains("declares") && stderr.contains("not there"),
        "and it must say WHY, naming the declaration: {stderr}");
}

/// The other half: `CANDOR_BASELINE` naming a missing path stays the adopt note (exit 0). The adopt
/// workflow sets it unconditionally, so absence there is "not adopted yet" rather than "deleted".
#[test]
fn env_named_baseline_that_is_missing_stays_a_note() {
    let d = make_crate("blenv", "pub fn pure() -> u32 { 1 }");
    let out = Command::new(bin())
        .arg(d.to_string_lossy().as_ref())
        .env("CANDOR_BASELINE", d.join(".candor/nope.json").to_string_lossy().as_ref())
        .output()
        .expect("run candor-scan");
    let stderr = String::from_utf8(out.stderr).unwrap();
    let _ = std::fs::remove_dir_all(&d);
    assert_eq!(out.status.code(), Some(0), "the env var's absence is not adoption yet: {stderr}");
    assert!(stderr.contains("regression guard is not active"), "…and it says so: {stderr}");
}

// ── the AS-EFF-005 baseline regression guard (spec §7 item 5; candor-java's checkBaseline is the model) ──

/// Run `candor-scan <dir> [args…]` with `CANDOR_BASELINE=<baseline>` (when given) and return
/// (exit code, stdout, stderr).
fn scan_with_baseline(d: &std::path::Path, baseline: Option<&str>, args: &[&str]) -> (Option<i32>, String, String) {
    let mut cmd = Command::new(bin());
    cmd.arg(d.to_string_lossy().as_ref()).args(args);
    // hermetic: the ambient environment must not smuggle in a gate/config of its own
    cmd.env_remove("CANDOR_BASELINE").env_remove("CANDOR_POLICY").env_remove("CANDOR_CONFIG").env_remove("CANDOR_DEPS");
    if let Some(b) = baseline {
        cmd.env("CANDOR_BASELINE", b);
    }
    let out = cmd.output().expect("run candor-scan");
    (out.status.code(), String::from_utf8(out.stdout).unwrap(), String::from_utf8(out.stderr).unwrap())
}

#[test]
fn baseline_guard_flags_a_gained_effect_exit_1_and_rides_the_gate_json() {
    // The happy ratchet: snapshot a crate whose fn performs { Fs }, make the fn ALSO spawn a process,
    // guard against the snapshot → one [AS-EFF-005] naming the gained Exec, exit 1 — and the violation
    // joins the --gate-json verdict via the same accumulator as the policy gate.
    let d = make_crate("blratchet", "pub fn go() { let _ = std::fs::read(\"/x\"); }");
    let pre = d.join("base");
    let (rc, _, _) = scan_with_baseline(&d, None, &["--out", pre.to_string_lossy().as_ref()]);
    assert_eq!(rc, Some(0), "recording the baseline is a plain scan");
    std::fs::write(d.join("src/lib.rs"),
        "pub fn go() { let _ = std::fs::read(\"/x\"); std::process::Command::new(\"sh\").status().unwrap(); }").unwrap();
    let verdict = d.join("verdict.json");
    let (rc, stdout, stderr) = scan_with_baseline(&d, Some(pre.to_string_lossy().as_ref()),
        &["--gate-json", verdict.to_string_lossy().as_ref()]);
    assert_eq!(rc, Some(1), "a gained effect is a violation (exit 1): {stderr}");
    let all = format!("{stdout}{stderr}");
    assert!(all.contains("[AS-EFF-005] `go` gained effect { Exec }"),
        "the violation line names the fn and the gained effect: {all}");
    let v: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(&verdict).unwrap()).unwrap();
    let _ = std::fs::remove_dir_all(&d);
    assert_eq!(v["ok"], serde_json::json!(false));
    assert!(v["violations"].as_array().unwrap().iter().any(|gv|
        gv["rule"] == "AS-EFF-005" && gv["fn"] == "go" && gv["effects"] == serde_json::json!(["Exec"])),
        "AS-EFF-005 joins the structured verdict: {v}");
}

/// A REAL REGRESSION DOMINATES AN INCOMPLETE SCAN — SPEC §3.3.1, verbatim: *"A configured gate over
/// incompletely-analyzed code MUST fail closed (exit ≠ 0); **a real violation (exit 1) still
/// dominates.**"* Both halves, and the second one was missing here.
///
/// The incomplete-analysis refusal used to run BEFORE `check_baseline` was called at all, so a crate
/// carrying a real AS-EFF-005 regression AND one unparseable file exited 2 and wrote
/// `{ok:false, incomplete:true, violations: []}` — the regression **absent from the artifact** a CI
/// consumer reads, not merely mis-coded. A machine-consumer under-report wearing an exit code.
///
/// THE POLICY GATE HAD EXACTLY THIS DEFECT AND WAS FIXED 2026-07-28; this is its sibling site and the
/// fix did not reach it. Two identical sequences, one repaired.
///
/// BOTH DIRECTIONS ARE ASSERTED, because the refusal is still right when there is nothing to report: a
/// CLEAN compare over unanalyzed code is the false-pure the refusal exists to prevent, and a fix that
/// simply dropped the refusal would trade a lost finding for a fabricated all-clear. What licenses
/// evaluating at all is an ASYMMETRY: a parse failure makes the scan see LESS, and AS-EFF-005 fires on
/// effects GAINED, so less evidence can only MASK a regression, never manufacture one.
#[test]
fn a_baseline_regression_beside_an_unparseable_file_still_reaches_the_verdict() {
    let d = make_crate("blincomplete", "pub fn go() { let _ = std::fs::read(\"/x\"); }");
    let pre = d.join("base");
    let (rc, _, _) = scan_with_baseline(&d, None, &["--out", pre.to_string_lossy().as_ref()]);
    assert_eq!(rc, Some(0), "recording the baseline is a plain scan");
    let base = format!("{}.blincomplete.scan.json", pre.to_string_lossy());

    // (a) THE CONTROL: the regression alone must be exit 1 with the finding, or the row below proves
    //     nothing about incompleteness — it would just be measuring a guard that never fires.
    std::fs::write(d.join("src/lib.rs"),
        "pub fn go() { let _ = std::fs::read(\"/x\"); std::process::Command::new(\"sh\").status().unwrap(); }").unwrap();
    let v1 = d.join("v1.json");
    let (rc, _, err) = scan_with_baseline(&d, Some(&base), &["--gate-json", v1.to_string_lossy().as_ref()]);
    assert_eq!(rc, Some(1), "the control must fire: {err}");
    let j1: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(&v1).unwrap()).unwrap();
    assert_eq!(j1["violations"].as_array().unwrap().len(), 1, "control verdict: {j1}");

    // (b) THE ROW: the same regression, with one file that fails to parse beside it.
    std::fs::write(d.join("src/broken.rs"), "pub fn broken( {{{ not rust\n").unwrap();
    let v2 = d.join("v2.json");
    let (rc, _, err) = scan_with_baseline(&d, Some(&base), &["--gate-json", v2.to_string_lossy().as_ref()]);
    assert_eq!(rc, Some(1),
        "a REAL regression must dominate an incomplete scan (§3.3.1) — exit 2 here reported \
         'I could not analyse' over 'your code regressed': {err}");
    let j2: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(&v2).unwrap()).unwrap();
    assert!(j2["violations"].as_array().unwrap().iter().any(|gv| gv["rule"] == "AS-EFF-005"),
        "the regression must be IN THE DOCUMENT, not just on stderr — dropping it is the \
         machine-consumer under-report: {j2}");
    assert_eq!(j2["incomplete"], serde_json::json!(true),
        "…and the incompleteness must ALSO be carried — this is both halves, not a swap: {j2}");
    assert!(!j2["unanalyzed"].as_array().unwrap().is_empty(), "the unparsed file is named: {j2}");

    // (c) THE OTHER DIRECTION: no regression, same unparseable file. The refusal MUST survive — a clean
    //     compare over unanalyzed code is a false-pure, and exit 0 here would be the fabricated all-clear.
    std::fs::write(d.join("src/lib.rs"), "pub fn go() { let _ = std::fs::read(\"/x\"); }").unwrap();
    let (rc, _, err) = scan_with_baseline(&d, Some(&base), &[]);
    let _ = std::fs::remove_dir_all(&d);
    assert_eq!(rc, Some(2),
        "with nothing to report, the guard must still REFUSE over an incomplete scan: {err}");
    assert!(err.contains("baseline guard NOT evaluated"), "and say so: {err}");
}

#[test]
fn baseline_guard_clean_compare_exits_0_and_new_fns_are_exempt() {
    // No gains → exit 0 with the guard-✓ receipt; and a NEW effectful fn (absent from the baseline)
    // is exempt — the guard is for regressions in EXISTING functions, new code is reviewed as new code.
    let d = make_crate("blclean", "pub fn go() { let _ = std::fs::read(\"/x\"); }");
    let pre = d.join("base");
    let (rc, _, _) = scan_with_baseline(&d, None, &["--out", pre.to_string_lossy().as_ref()]);
    assert_eq!(rc, Some(0));
    // (a) unchanged code: clean.
    let (rc, _, stderr) = scan_with_baseline(&d, Some(pre.to_string_lossy().as_ref()), &[]);
    assert_eq!(rc, Some(0), "an unchanged crate passes the ratchet: {stderr}");
    assert!(stderr.contains("baseline guard ✓"), "the clean guard prints its receipt: {stderr}");
    // (b) a brand-new effectful fn: exempt, still exit 0, no AS-EFF-005.
    std::fs::write(d.join("src/lib.rs"),
        "pub fn go() { let _ = std::fs::read(\"/x\"); }\npub fn newbie() { std::process::Command::new(\"sh\").status().unwrap(); }").unwrap();
    let (rc, stdout, stderr) = scan_with_baseline(&d, Some(pre.to_string_lossy().as_ref()), &[]);
    let _ = std::fs::remove_dir_all(&d);
    assert_eq!(rc, Some(0), "a new fn is not a regression: {stderr}");
    assert!(!format!("{stdout}{stderr}").contains("AS-EFF-005"), "no violation for new code: {stdout}{stderr}");
}

#[test]
fn baseline_guard_absent_file_notes_once_and_exit_unchanged() {
    // CANDOR_BASELINE set but no such file: the ratchet is not adopted yet — a stderr note with the
    // record incantation, exit unchanged (candor-java's absent-file posture; NOT a failure).
    let d = make_crate("blabsent", "pub fn go() { let _ = std::fs::read(\"/x\"); }");
    let (rc, _, stderr) = scan_with_baseline(&d, Some(d.join("nosuch").to_string_lossy().as_ref()), &[]);
    let _ = std::fs::remove_dir_all(&d);
    assert_eq!(rc, Some(0), "an absent baseline leaves the exit code unchanged: {stderr}");
    assert!(stderr.contains("regression guard is not active") && stderr.contains("record one:"),
        "the note says the guard is inactive and how to record a baseline: {stderr}");
}

#[test]
fn baseline_guard_version_mismatch_fails_closed_without_evaluating() {
    // §2.1: a baseline is comparable only to its OWN producing build. Doctor the envelope version on a
    // baseline that WOULD flag a gain — the guard must exit 2 WITHOUT evaluating (no AS-EFF-005 wave,
    // no silent skip). A MISSING version (legacy bare array) is the same class.
    let d = make_crate("blver", "pub fn go() { let _ = std::fs::read(\"/x\"); }");
    let pre = d.join("base");
    let (rc, _, _) = scan_with_baseline(&d, None, &["--out", pre.to_string_lossy().as_ref()]);
    assert_eq!(rc, Some(0));
    // introduce a gain, then doctor the baseline's producing version
    std::fs::write(d.join("src/lib.rs"),
        "pub fn go() { let _ = std::fs::read(\"/x\"); std::process::Command::new(\"sh\").status().unwrap(); }").unwrap();
    let file = d.join("base.blver.scan.json");
    let mut v: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(&file).unwrap()).unwrap();
    v["candor"]["version"] = serde_json::json!("scan-0.0.0-doctored");
    std::fs::write(&file, serde_json::to_string(&v).unwrap()).unwrap();
    let (rc, stdout, stderr) = scan_with_baseline(&d, Some(pre.to_string_lossy().as_ref()), &[]);
    assert_eq!(rc, Some(2), "a producing-version mismatch is invalid gate input (exit 2): {stderr}");
    assert!(stderr.contains("scan-0.0.0-doctored") && stderr.contains("cannot evaluate"),
        "the diagnostic names both builds and refuses to evaluate: {stderr}");
    assert!(!format!("{stdout}{stderr}").contains("[AS-EFF-005]"),
        "no AS-EFF-005 violation may be emitted from a stale baseline: {stdout}{stderr}");
    // MISSING version: a bare-array legacy report has no provenance — same exit 2, no evaluation.
    std::fs::write(&file, "[{\"fn\":\"go\",\"inferred\":[]}]").unwrap();
    let (rc, stdout, stderr) = scan_with_baseline(&d, Some(pre.to_string_lossy().as_ref()), &[]);
    let _ = std::fs::remove_dir_all(&d);
    assert_eq!(rc, Some(2), "a provenance-less baseline cannot certify its build: {stderr}");
    assert!(!format!("{stdout}{stderr}").contains("[AS-EFF-005]"), "never evaluated: {stdout}{stderr}");
}

#[test]
fn baseline_guard_unparseable_or_empty_value_fails_closed() {
    // An UNPARSEABLE baseline (corrupt/truncated) exits 2 — the unreadable-policy class (§6.2), never a
    // silent pass. A configured-but-EMPTY value is the same class (matches the bare `policy` posture).
    let d = make_crate("blcorrupt", "pub fn go() { let _ = std::fs::read(\"/x\"); }");
    let garbage = d.join("base.blcorrupt.scan.json");
    std::fs::write(&garbage, "{ this is not json").unwrap();
    let (rc, _, stderr) = scan_with_baseline(&d, Some(d.join("base").to_string_lossy().as_ref()), &[]);
    assert_eq!(rc, Some(2), "a corrupt baseline is invalid gate input: {stderr}");
    assert!(stderr.contains("could not be parsed"), "the diagnostic says why: {stderr}");
    // empty value (e.g. `CANDOR_BASELINE=` or a bare `baseline` config line) — fail closed, loud.
    let (rc, _, stderr) = scan_with_baseline(&d, Some(""), &[]);
    let _ = std::fs::remove_dir_all(&d);
    assert_eq!(rc, Some(2), "a configured-but-empty baseline must not silently skip the guard: {stderr}");
    assert!(stderr.contains("EMPTY value"), "the diagnostic names the empty value: {stderr}");
}

// ── ⟨0.16⟩ callgraph-sidecar existence: a formerly-PURE fn turning effectful is a GAIN ──
// (spec §7 item 5 ⟨0.16⟩; the `gains --json` `origin` existence rule applied to the scan ratchet.)

/// The two-fn probe of §7 item 5: `util::fmt` is PURE (omitted from the report but a callgraph node),
/// `api::fetch` performs Net. The sidecar sits beside `<pre>.<crate>.scan.json` as
/// `<pre>.<crate>.scan.callgraph.json` and lists BOTH names, so existence keyed on it sees the pure leaf.
const PROBE_SRC: &str = "pub mod util { pub fn fmt(s:&str)->String{ s.to_uppercase() } }\n\
     pub mod api { pub fn fetch(h:&str){ let _=std::net::TcpStream::connect((h,80)); } }";

#[test]
fn baseline_guard_sidecar_present_flags_pure_to_effectful_transition_exit_1() {
    // The sharpest supply-chain shape: a fn that was PURE in the baseline (absent from the report, but a
    // node in the callgraph sidecar) now performs an effect. Report-only existence read it as exempt
    // "new"; keyed on the sidecar its baseline set is ∅ and any current effect is an AS-EFF-005 gain.
    let d = make_crate("blcgpure", PROBE_SRC);
    let pre = d.join("base");
    let (rc, _, _) = scan_with_baseline(&d, None, &["--out", pre.to_string_lossy().as_ref()]);
    assert_eq!(rc, Some(0), "recording the baseline (with its callgraph sidecar) is a plain scan");
    assert!(d.join("base.blcgpure.scan.callgraph.json").is_file(), "the baseline records a callgraph sidecar");
    // util::fmt gains Fs; it was pure in the baseline → a gain, not exempt new code.
    std::fs::write(d.join("src/lib.rs"),
        "pub mod util { pub fn fmt(s:&str)->String{ let _=std::fs::read_to_string(\"x\"); s.to_uppercase() } }\n\
         pub mod api { pub fn fetch(h:&str){ let _=std::net::TcpStream::connect((h,80)); } }").unwrap();
    let (rc, stdout, stderr) = scan_with_baseline(&d, Some(pre.to_string_lossy().as_ref()), &[]);
    let _ = std::fs::remove_dir_all(&d);
    let all = format!("{stdout}{stderr}");
    assert_eq!(rc, Some(1), "a formerly-pure fn turning effectful is a violation (exit 1): {all}");
    assert!(all.contains("[AS-EFF-005] `util::fmt` gained effect { Fs }"),
        "the violation names the pure leaf and its gained effect: {all}");
}

#[test]
fn baseline_guard_sidecar_absent_degrades_to_report_only_with_a_note_exit_0() {
    // Delete the sidecar: existence degrades to pre-⟨0.16⟩ report-only, so the formerly-pure fn reads as
    // exempt "new" and ESCAPES (exit 0), with a one-time stderr note that the guard is weaker. This is a
    // degradation, not a failure — a baseline recorded by an older build simply has no sidecar.
    let d = make_crate("blcgabsent", PROBE_SRC);
    let pre = d.join("base");
    let (rc, _, _) = scan_with_baseline(&d, None, &["--out", pre.to_string_lossy().as_ref()]);
    assert_eq!(rc, Some(0));
    std::fs::remove_file(d.join("base.blcgabsent.scan.callgraph.json")).unwrap();
    std::fs::write(d.join("src/lib.rs"),
        "pub mod util { pub fn fmt(s:&str)->String{ let _=std::fs::read_to_string(\"x\"); s.to_uppercase() } }\n\
         pub mod api { pub fn fetch(h:&str){ let _=std::net::TcpStream::connect((h,80)); } }").unwrap();
    let (rc, stdout, stderr) = scan_with_baseline(&d, Some(pre.to_string_lossy().as_ref()), &[]);
    let _ = std::fs::remove_dir_all(&d);
    let all = format!("{stdout}{stderr}");
    assert_eq!(rc, Some(0), "an absent sidecar degrades, it does not fail: {all}");
    assert!(!all.contains("[AS-EFF-005]"), "the pure→effectful fn escapes under report-only existence: {all}");
    assert!(stderr.contains("sidecar") && stderr.contains("degrades to"),
        "the note discloses the weakened guard: {stderr}");
}

#[test]
fn baseline_guard_sidecar_corrupt_fails_closed_exit_2() {
    // A PRESENT-but-corrupt sidecar must fail closed (exit 2), mirroring a corrupt baseline: a broken
    // sidecar must not silently narrow the guard by making its pure leaves read as exempt "new".
    let d = make_crate("blcgcorrupt", PROBE_SRC);
    let pre = d.join("base");
    let (rc, _, _) = scan_with_baseline(&d, None, &["--out", pre.to_string_lossy().as_ref()]);
    assert_eq!(rc, Some(0));
    std::fs::write(d.join("base.blcgcorrupt.scan.callgraph.json"), "{").unwrap();
    std::fs::write(d.join("src/lib.rs"),
        "pub mod util { pub fn fmt(s:&str)->String{ let _=std::fs::read_to_string(\"x\"); s.to_uppercase() } }\n\
         pub mod api { pub fn fetch(h:&str){ let _=std::net::TcpStream::connect((h,80)); } }").unwrap();
    let (rc, stdout, stderr) = scan_with_baseline(&d, Some(pre.to_string_lossy().as_ref()), &[]);
    let _ = std::fs::remove_dir_all(&d);
    let all = format!("{stdout}{stderr}");
    assert_eq!(rc, Some(2), "a corrupt sidecar is invalid gate input (exit 2): {all}");
    assert!(stderr.contains("callgraph") && stderr.contains("could not be parsed"),
        "the diagnostic names the broken sidecar: {stderr}");
    assert!(!all.contains("[AS-EFF-005]"), "the guard is NOT evaluated on a broken sidecar: {all}");
}

#[test]
fn baseline_guard_pure_to_unknown_only_gain_is_advisory_not_a_regression_exit_0() {
    // ⟨0.16⟩ the ratchet fires only on gaining a REAL boundary effect. A formerly-pure fn that gains
    // ONLY Unknown (an unresolved call — the §4 trust marker, not an effect) is DISCLOSED as advisory,
    // exit 0 — on real version bumps an Unknown-only gain is dominated by resolution noise, so failing
    // on it would break CI on innocuous updates (SOUNDNESS-LOG 2026-07-16).
    let d = make_crate("blunk", "pub fn helper()->usize{ 0 }\npub fn fmt(s:&str)->usize{ s.len() }\n");
    let pre = d.join("base");
    let (rc, _, _) = scan_with_baseline(&d, None, &["--out", pre.to_string_lossy().as_ref()]);
    assert_eq!(rc, Some(0));
    // fmt was pure; now it calls a fn pointer → Unknown-ONLY (no real effect).
    std::fs::write(d.join("src/lib.rs"),
        "pub fn helper()->usize{ 0 }\npub fn fmt(s:&str)->usize{ let g: fn()->usize = helper; g() }\n").unwrap();
    let (rc, stdout, stderr) = scan_with_baseline(&d, Some(pre.to_string_lossy().as_ref()), &[]);
    let _ = std::fs::remove_dir_all(&d);
    let all = format!("{stdout}{stderr}");
    assert_eq!(rc, Some(0), "an Unknown-only gain is advisory, not a regression: {all}");
    assert!(!all.contains("[AS-EFF-005]"), "no violation for an Unknown-only gain: {all}");
    assert!(stderr.contains("Unknown") && stderr.contains("advisory"),
        "the advisory note discloses the Unknown-gain: {stderr}");
}

#[test]
fn baseline_unknown_ratchet_flips_a_new_unknown_gain_to_a_failure_end_to_end() {
    // ⟨unknown-ratchet⟩ OPT-IN through the REAL binary + env var (config `unknown-ratchet` /
    // CANDOR_UNKNOWN_RATCHET; candor-java Policy.checkBaseline is the model). Default OFF an Unknown-only
    // gain stays advisory (exit 0); ON it becomes an AS-EFF-005 FAILURE (exit 1) — making `deny Unknown`
    // adoptable on legacy code by freezing today's report and ratcheting the Unknown surface DOWN.
    let d = make_crate("blratchetnew", "pub fn helper()->usize{ 0 }\npub fn fmt(s:&str)->usize{ s.len() }\n");
    let pre = d.join("base");
    let (rc, _, _) = scan_with_baseline(&d, None, &["--out", pre.to_string_lossy().as_ref()]);
    assert_eq!(rc, Some(0));
    // fmt was pure; now it calls a fn pointer → gains ONLY Unknown (a NEW blind spot vs the baseline).
    std::fs::write(d.join("src/lib.rs"),
        "pub fn helper()->usize{ 0 }\npub fn fmt(s:&str)->usize{ let g: fn()->usize = helper; g() }\n").unwrap();
    // ratchet OFF (default): advisory, exit 0 — byte-identical to the ⟨0.16⟩ posture.
    let (rc, stdout, stderr) = scan_with_baseline(&d, Some(pre.to_string_lossy().as_ref()), &[]);
    let off = format!("{stdout}{stderr}");
    assert_eq!(rc, Some(0), "ratchet OFF: an Unknown-only gain is advisory: {off}");
    assert!(!off.contains("[AS-EFF-005]"), "ratchet OFF raises no violation: {off}");
    // ratchet ON via CANDOR_UNKNOWN_RATCHET: the new Unknown FAILS (AS-EFF-005, exit 1).
    let mut cmd = Command::new(bin());
    cmd.arg(d.to_string_lossy().as_ref())
        .env_remove("CANDOR_POLICY").env_remove("CANDOR_CONFIG").env_remove("CANDOR_DEPS")
        .env("CANDOR_BASELINE", pre.to_string_lossy().as_ref())
        .env("CANDOR_UNKNOWN_RATCHET", "1");
    let out = cmd.output().expect("run candor-scan");
    let _ = std::fs::remove_dir_all(&d);
    let on = format!("{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr));
    assert_eq!(out.status.code(), Some(1), "ratchet ON: a newly-introduced Unknown fails: {on}");
    assert!(on.contains("[AS-EFF-005]") && on.contains("unknown-ratchet"),
        "the ratchet violation is the AS-EFF-005 unknown-ratchet finding: {on}");
}

#[test]
fn baseline_guard_config_key_resolves_against_the_config_home_and_env_wins() {
    // The `.candor/config` `baseline` key drives the guard with a RELATIVE value anchored to the
    // config's HOME dir (spec §3.4) — never the process CWD — and the CANDOR_BASELINE env overrides it.
    let d = make_crate("blcfg", "pub fn go() { let _ = std::fs::read(\"/x\"); }");
    std::fs::create_dir_all(d.join(".candor")).unwrap();
    std::fs::write(d.join(".candor/config"), "baseline .candor/base\n").unwrap();
    // record the baseline at the config's (home-anchored) prefix, then introduce a gain
    let (rc, _, _) = scan_with_baseline(&d, None,
        &["--out", d.join(".candor/base").to_string_lossy().as_ref()]);
    assert_eq!(rc, Some(0));
    std::fs::write(d.join("src/lib.rs"),
        "pub fn go() { let _ = std::fs::read(\"/x\"); std::process::Command::new(\"sh\").status().unwrap(); }").unwrap();
    // run from an UNRELATED CWD with no env: only home-anchored resolution finds the baseline → exit 1
    let out = Command::new(bin()).arg(d.to_string_lossy().as_ref())
        .current_dir(std::env::temp_dir()).output().expect("run candor-scan");
    assert_eq!(out.status.code(), Some(1),
        "the config `baseline` key must activate the guard, home-anchored: {}",
        String::from_utf8_lossy(&out.stderr));
    assert!(String::from_utf8_lossy(&out.stdout).contains("[AS-EFF-005]"),
        "the gain is reported: {}", String::from_utf8_lossy(&out.stdout));
    // env wins over config: record a FRESH snapshot of the current code (env pointed at an absent
    // path so the config's stale baseline can't gate the recording run — exit 0 proves the override),
    // then guard against it → exit 0 despite the config still naming the stale prefix.
    let fresh = d.join("fresh");
    let (rc, _, stderr) = scan_with_baseline(&d, Some(d.join("void").to_string_lossy().as_ref()),
        &["--out", fresh.to_string_lossy().as_ref()]);
    assert_eq!(rc, Some(0), "an absent env baseline overrides the config's firing one: {stderr}");
    let (rc, _, stderr) = scan_with_baseline(&d, Some(fresh.to_string_lossy().as_ref()), &[]);
    let _ = std::fs::remove_dir_all(&d);
    assert_eq!(rc, Some(0), "CANDOR_BASELINE env overrides the config key: {stderr}");
}

#[test]
fn cfg_feature_gated_statements_scope_effects_to_the_default_build() {
    // End-to-end through the real binary: Cargo.toml [features] parsing (incl. the transitive
    // `default` closure) → the 3-valued cfg evaluator → the collector's statement skip. A statement
    // gated on a declared-but-inactive feature is compiled OUT under the default build, so its effect
    // must NOT be the crate's (winnow's debug-trace `std::env::var` fabricated Env). An UNKNOWN
    // predicate (target_os, an undeclared feature) keeps the statement — the conservative direction.
    let d = std::env::temp_dir().join(format!("candor-scan-cli-cfgfeat-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(d.join("src")).unwrap();
    std::fs::write(
        d.join("Cargo.toml"),
        "[package]\nname = \"cfgfeat\"\n\n[features]\ndefault = [\"on\"]\non = [\"deep\"]\ndeep = []\noff = []\n",
    )
    .unwrap();
    std::fs::write(
        d.join("src/lib.rs"),
        r#"
pub fn gated_out() {
    #[cfg(feature = "off")]
    { let _ = std::process::Command::new("sh").status(); }
}
pub fn gated_out_let() {
    #[cfg(feature = "off")]
    let _x = std::fs::read("/x");
}
pub fn nested_out() {
    #[cfg(all(feature = "on", feature = "off"))]
    { let _ = std::fs::read("/x"); }
}
pub fn unknown_kept() {
    #[cfg(any(feature = "off", target_os = "linux"))]
    { let _ = std::net::TcpStream::connect("h:1"); }
}
pub fn nested_in() {
    #[cfg(all(feature = "deep", not(feature = "off")))]
    { let _ = std::env::var("HOME"); }
}
"#,
    )
    .unwrap();
    let out = Command::new(bin())
        .arg(d.to_string_lossy().as_ref())
        .arg("--json")
        .env_remove("CANDOR_POLICY")
        .env_remove("CANDOR_CONFIG")
        .env_remove("CANDOR_DEPS")
        .output()
        .expect("run candor-scan");
    let _ = std::fs::remove_dir_all(&d);
    assert_eq!(out.status.code(), Some(0));
    let v: serde_json::Value =
        serde_json::from_str(String::from_utf8(out.stdout).unwrap().trim()).expect("json report");
    let eff = |needle: &str| -> Vec<String> {
        v["functions"].as_array().into_iter().flatten()
            .filter(|f| f["fn"].as_str().is_some_and(|q| q.contains(needle)))
            .flat_map(|f| f["inferred"].as_array().into_iter().flatten()
                .filter_map(|e| e.as_str().map(String::from)).collect::<Vec<_>>())
            .collect()
    };
    // compiled-out statements contribute NOTHING (the report holds only effectful fns, so these are absent)
    assert!(eff("gated_out").is_empty(), "a feature-inactive stmt fabricated its effect:\n{v}");
    assert!(eff("nested_out").is_empty(), "all(on, off) is definite-false — Fs here is fabricated:\n{v}");
    // unresolvable predicates KEEP the statement (kept = the sound, never-under-report direction)
    assert!(eff("unknown_kept").contains(&"Net".to_string()),
            "any(false, unknown) is unknown — the stmt must be kept, Net lost:\n{v}");
    // the transitive default closure (default → on → deep) makes `deep` ACTIVE
    assert!(eff("nested_in").contains(&"Env".to_string()),
            "all(deep-active, not(off)) is definite-true — Env lost (default closure broken?):\n{v}");
}

// ── --deps: the registry-tree scan mode (run_with_deps — was 0-covered everywhere) ─────────────────
//
// Hermetic: a FAKE cargo registry checkout tree under a per-test CARGO_HOME
// (`<CARGO_HOME>/registry/src/<index-hash>/<name>-<version>/` — the shape dirs_cargo_registry_src
// discovers), no network, no real ~/.cargo. Every test scrubs the CANDOR_* env so the runner's own
// config can't leak into the child.

/// Build `<tag>`'s fake CARGO_HOME carrying one registry-src index with the given package checkouts.
fn make_registry(tag: &str, pkgs: &[(&str, &str, &str)]) -> PathBuf {
    let ch = std::env::temp_dir().join(format!("candor-scan-cli-ch-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&ch);
    let idx = ch.join("registry/src/index.crates.io-0000000000000000");
    std::fs::create_dir_all(&idx).unwrap();
    for (n, v, src) in pkgs {
        let d = idx.join(format!("{n}-{v}"));
        std::fs::create_dir_all(d.join("src")).unwrap();
        std::fs::write(d.join("Cargo.toml"), format!("[package]\nname = \"{n}\"\n")).unwrap();
        std::fs::write(d.join("src/lib.rs"), src).unwrap();
    }
    ch
}

/// Write `root/Cargo.lock` naming the packages; `registry: true` marks a crates.io source line
/// (the root package itself carries no `source =` — exactly what cargo writes).
fn write_lockfile(root: &std::path::Path, pkgs: &[(&str, &str, bool)]) {
    let mut s = String::from("version = 3\n");
    for (n, v, reg) in pkgs {
        s.push_str(&format!("\n[[package]]\nname = \"{n}\"\nversion = \"{v}\"\n"));
        if *reg {
            s.push_str("source = \"registry+https://github.com/rust-lang/crates.io-index\"\n");
        }
    }
    std::fs::write(root.join("Cargo.lock"), s).unwrap();
}

/// Spawn the binary in --deps mode against `dir` with the fake CARGO_HOME, extra args appended.
fn run_deps(dir: &std::path::Path, cargo_home: &std::path::Path, args: &[&str]) -> std::process::Output {
    let mut c = Command::new(bin());
    c.arg(dir.to_string_lossy().as_ref()).arg("--deps");
    for a in args {
        c.arg(a);
    }
    c.env("CARGO_HOME", cargo_home)
        .env_remove("CANDOR_DEPS")
        .env_remove("CANDOR_POLICY")
        .env_remove("CANDOR_CONFIG");
    c.output().expect("run candor-scan --deps")
}

#[test]
fn deps_without_cargo_lock_fails_closed_exit_2() {
    // The documented precondition: --deps reads Cargo.lock. Missing lockfile → clean one-line error
    // naming the fix (`cargo generate-lockfile`), exit 2 — never a silent lockless "success".
    let d = make_crate("depsnolock", "pub fn go() {}");
    let ch = make_registry("nolock", &[]);
    let out = run_deps(&d, &ch, &[]);
    let _ = std::fs::remove_dir_all(&d);
    let _ = std::fs::remove_dir_all(&ch);
    assert_eq!(out.status.code(), Some(2), "--deps without a lockfile must exit 2");
    let stderr = String::from_utf8(out.stderr).unwrap();
    assert!(stderr.contains("Cargo.lock") && stderr.contains("generate-lockfile"),
            "the error must name the missing lockfile + the incantation, got:\n{stderr}");
}

#[test]
fn deps_scans_registry_tree_chains_effects_and_caches() {
    // The happy path, end to end: the locked registry dep is discovered in the fake CARGO_HOME,
    // scanned into <dir>/.candor/deps/<name>@<version>/ (the documented location), and the root scan
    // is CHAINED over the fresh report — the dep's effect + literal surface cross the crate boundary.
    // A dep in the lockfile with NO local checkout is disclosed in the summary, not fatal.
    let ch = make_registry("happy", &[(
        "depx", "0.1.0",
        r#"pub fn eff() { let _ = std::fs::read("/etc/depx.conf"); }"#,
    )]);
    let d = make_crate("depsroot", "pub fn uses() { depx::eff(); }");
    std::fs::write(d.join("Cargo.toml"),
        "[package]\nname = \"depsroot\"\n\n[dependencies]\ndepx = \"0.1.0\"\nghost = \"0.9.9\"\n").unwrap();
    write_lockfile(&d, &[("depsroot", "0.1.0", false), ("depx", "0.1.0", true), ("ghost", "0.9.9", true)]);

    let out = run_deps(&d, &ch, &["--json"]);
    assert_eq!(out.status.code(), Some(0), "a clean --deps run must exit 0: {}",
               String::from_utf8_lossy(&out.stderr));
    // dep report lands where documented: <dir>/.candor/deps/<name>@<version>/report.<crate>.scan.json
    assert!(d.join(".candor/deps/depx@0.1.0/report.depx.scan.json").is_file(),
            "the dep report must be written under .candor/deps/<name>@<version>/");
    let stderr = String::from_utf8(out.stderr).unwrap();
    assert!(stderr.contains("scanned 1 of 2 registry dependencies"),
            "the summary counts scanned/locked (root pkg is not a registry dep), got:\n{stderr}");
    assert!(stderr.contains("without a local checkout") && stderr.contains("ghost-0.9.9"),
            "a lockfile dep with no checkout is DISCLOSED, not fatal, got:\n{stderr}");
    // the chained join: the root fn inherits the dep's Fs AND its literal path surface
    let v: serde_json::Value =
        serde_json::from_str(String::from_utf8(out.stdout).unwrap().trim()).expect("json report");
    let uses = v["functions"].as_array().into_iter().flatten()
        .find(|f| f["fn"].as_str() == Some("uses"))
        .unwrap_or_else(|| panic!("`uses` missing from the chained report:\n{v}"));
    assert!(uses["inferred"].as_array().is_some_and(|a| a.iter().any(|e| e == "Fs")),
            "the dep's Fs must cross the crate boundary via the chain:\n{v}");
    assert!(uses["paths"].as_array().is_some_and(|a| a.iter().any(|p| p == "/etc/depx.conf")),
            "the dep's literal path surface must ride the join:\n{v}");

    // SECOND run: registry checkouts are immutable per name@version — the report is reused, not rescanned.
    let out2 = run_deps(&d, &ch, &["--json"]);
    assert_eq!(out2.status.code(), Some(0));
    let stderr2 = String::from_utf8(out2.stderr).unwrap();
    assert!(stderr2.contains("scanned 0 of 2") && stderr2.contains("1 already scanned — cached"),
            "the second run must reuse the cached dep report, got:\n{stderr2}");
    let _ = std::fs::remove_dir_all(&d);
    let _ = std::fs::remove_dir_all(&ch);
}

#[test]
fn deps_dependency_scans_run_gate_free_but_root_gate_sees_the_chain() {
    // Two sides of one contract (scan_one's `policy: None` for dep scans): the ROOT policy must not
    // run against dependency internals (328 spurious gate runs, per the review), yet an effect the
    // root INHERITS through the chain is fully gate-visible.
    let ch = make_registry("gate", &[(
        "depg", "0.2.0",
        r#"pub fn spawn() { let _ = std::process::Command::new("sh").status(); }"#,
    )]);
    // (a) root does NOT call the dep → `deny Exec` is clean for the root → exit 0. If dep scans were
    // gated, the dep's own `spawn` would fail the build here.
    let d = make_crate("gatefree", "pub fn quiet() {}");
    std::fs::write(d.join("Cargo.toml"),
        "[package]\nname = \"gatefree\"\n\n[dependencies]\ndepg = \"0.2.0\"\n").unwrap();
    write_lockfile(&d, &[("gatefree", "0.1.0", false), ("depg", "0.2.0", true)]);
    let pol = d.join("candor.policy");
    std::fs::write(&pol, "deny Exec\n").unwrap();
    let out = run_deps(&d, &ch, &["--policy", pol.to_string_lossy().as_ref()]);
    assert_eq!(out.status.code(), Some(0),
               "dep scans must run GATE-FREE — the root policy fired on dependency internals:\n{}",
               String::from_utf8_lossy(&out.stderr));
    let _ = std::fs::remove_dir_all(&d);

    // (b) root DOES call the dep → it inherits Exec through the chain → the same policy exits 1.
    let d2 = make_crate("gatechain", "pub fn uses() { depg::spawn(); }");
    std::fs::write(d2.join("Cargo.toml"),
        "[package]\nname = \"gatechain\"\n\n[dependencies]\ndepg = \"0.2.0\"\n").unwrap();
    write_lockfile(&d2, &[("gatechain", "0.1.0", false), ("depg", "0.2.0", true)]);
    let pol2 = d2.join("candor.policy");
    std::fs::write(&pol2, "deny Exec\n").unwrap();
    let out2 = run_deps(&d2, &ch, &["--policy", pol2.to_string_lossy().as_ref()]);
    assert_eq!(out2.status.code(), Some(1),
               "a chained-in Exec must fail the root gate (exit 1):\n{}",
               String::from_utf8_lossy(&out2.stderr));
    // (non-json runs print the violation lines on stdout; the summary count goes to stderr)
    let stdout2 = String::from_utf8(out2.stdout).unwrap();
    assert!(stdout2.contains("uses"), "the violation names the ROOT fn, got:\n{stdout2}");
    let _ = std::fs::remove_dir_all(&d2);
    let _ = std::fs::remove_dir_all(&ch);
}

#[test]
fn deps_workspace_root_fans_out_over_members() {
    // `--deps <workspace>`: the final root scan funnels through scan_target, so members are scanned
    // individually — the nested-package filter must NOT prune them into an empty, gate-passing report.
    let ch = make_registry("wsfan", &[(
        "depw", "1.0.0",
        r#"pub fn tick() { let _ = std::env::var("TZ"); }"#,
    )]);
    let d = std::env::temp_dir().join(format!("candor-scan-cli-depsws-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(d.join("m1/src")).unwrap();
    std::fs::write(d.join("Cargo.toml"), "[workspace]\nmembers = [\"m1\"]\n").unwrap();
    std::fs::write(d.join("m1/Cargo.toml"),
        "[package]\nname = \"m1\"\n\n[dependencies]\ndepw = \"1.0.0\"\n").unwrap();
    std::fs::write(d.join("m1/src/lib.rs"), "pub fn go() { depw::tick(); }\n").unwrap();
    write_lockfile(&d, &[("m1", "0.1.0", false), ("depw", "1.0.0", true)]);
    let out = run_deps(&d, &ch, &[]);
    assert_eq!(out.status.code(), Some(0), "{}", String::from_utf8_lossy(&out.stderr));
    // the member report exists under the workspace's default prefix and carries the chained Env
    let rep = std::fs::read_to_string(d.join(".candor/report.m1.scan.json"))
        .expect("member report must be written (the fan-out, not the pruned-empty root scan)");
    assert!(rep.contains("\"go\"") && rep.contains("Env"),
            "the member's chained dep effect is missing: {rep}");
    let _ = std::fs::remove_dir_all(&d);
    let _ = std::fs::remove_dir_all(&ch);
}

#[test]
fn deps_appends_candor_deps_env_reports_to_the_chain() {
    // CANDOR_DEPS is honoured IN ADDITION to the fresh .candor/deps tree (run_with_deps concatenates
    // the spec) — a sibling report for a crate outside the registry still joins.
    let ch = make_registry("extra", &[]); // no registry checkouts at all
    let extra = std::env::temp_dir().join(format!("candor-scan-cli-extradep-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&extra);
    std::fs::create_dir_all(&extra).unwrap();
    let me = env!("CARGO_PKG_VERSION");
    std::fs::write(extra.join("report.extdep.scan.json"), format!(r#"{{
        "candor": {{"version": "scan-{me}", "toolchain": "stable", "spec": "0.23"}},
        "package": "extdep",
        "functions": [{{"fn": "ping", "inferred": ["Net"], "hash": "extdep#ping"}}]}}"#)).unwrap();
    let d = make_crate("extroot", "pub fn calls() { extdep::ping(); }");
    write_lockfile(&d, &[("extroot", "0.1.0", false)]);
    let mut c = Command::new(bin());
    c.arg(d.to_string_lossy().as_ref()).arg("--deps").arg("--json")
        .env("CARGO_HOME", &ch)
        .env("CANDOR_DEPS", extra.to_string_lossy().as_ref())
        .env_remove("CANDOR_POLICY")
        .env_remove("CANDOR_CONFIG");
    let out = c.output().expect("run candor-scan --deps");
    let _ = std::fs::remove_dir_all(&d);
    let _ = std::fs::remove_dir_all(&extra);
    let _ = std::fs::remove_dir_all(&ch);
    assert_eq!(out.status.code(), Some(0), "{}", String::from_utf8_lossy(&out.stderr));
    let v: serde_json::Value =
        serde_json::from_str(String::from_utf8(out.stdout).unwrap().trim()).expect("json report");
    let calls = v["functions"].as_array().into_iter().flatten()
        .find(|f| f["fn"].as_str() == Some("calls"))
        .unwrap_or_else(|| panic!("`calls` missing:\n{v}"));
    assert!(calls["inferred"].as_array().is_some_and(|a| a.iter().any(|e| e == "Net")),
            "a CANDOR_DEPS sibling report must still chain under --deps:\n{v}");
}

/// The dispatch-classification hierarchy the fix `port` advice relies on (eval/fixloop/DISPATCH-NOTE.md):
/// a call through a resolvable TRAIT object charges the impl's effect (sound — the domain CAN reach it); a
/// call through a FUNCTION VALUE is Unknown (candor can't resolve it — the §4 marker, never "clean"); a plain
/// DATA parameter is pure. This is WHY a trait "port" doesn't clear `deny Net domain` but a fn/closure does,
/// and why the simplest hoist (pass data) is the only PROVABLY-pure fix. Guarding it so it can't silently
/// regress (which would change what candor fix should advise).
#[test]
fn dispatch_classification_hierarchy_trait_net_fn_unknown_data_pure() {
    let d = make_crate("dispatchclass", r#"
pub mod tr {
    pub trait R { fn g(&self) -> u64; }
    pub struct NetImpl;
    impl R for NetImpl { fn g(&self) -> u64 { let _ = std::net::TcpStream::connect("h:1"); 1 } }
    pub fn via_trait(r: &dyn R) -> u64 { r.g() }
}
pub mod fnv {
    pub fn via_fn(f: &dyn Fn() -> u64) -> u64 { f() }
}
pub mod dat {
    pub fn via_data(x: u64) -> u64 { x + 1 }
}
"#);
    let out = Command::new(bin())
        .arg(d.to_string_lossy().as_ref()).arg("--json")
        .env_remove("CANDOR_POLICY").env_remove("CANDOR_CONFIG").env_remove("CANDOR_DEPS")
        .output().expect("run candor-scan");
    let _ = std::fs::remove_dir_all(&d);
    assert_eq!(out.status.code(), Some(0), "{}", String::from_utf8_lossy(&out.stderr));
    let v: serde_json::Value =
        serde_json::from_str(String::from_utf8(out.stdout).unwrap().trim()).expect("json report");
    let eff = |needle: &str| -> Vec<String> {
        v["functions"].as_array().into_iter().flatten()
            .filter(|f| f["fn"].as_str().is_some_and(|q| q.contains(needle)))
            .flat_map(|f| f["inferred"].as_array().into_iter().flatten()
                .filter_map(|e| e.as_str().map(String::from)).collect::<Vec<_>>())
            .collect()
    };
    // trait dispatch → resolved to the Net impl → the caller performs Net.
    assert!(eff("via_trait").contains(&"Net".to_string()),
            "a trait call whose impl does Net must charge Net to the caller (resolved dispatch):\n{v}");
    // fn-value → Unknown (candor can't see through a function value). NOT Net, NOT pure.
    assert!(eff("via_fn").contains(&"Unknown".to_string()) && !eff("via_fn").contains(&"Net".to_string()),
            "a call through a function value must be Unknown, not Net and not clean:\n{v}");
    // plain data → pure (absent from the effectful report).
    assert!(eff("via_data").is_empty(), "a plain-data parameter must stay pure (no effect, no Unknown):\n{v}");
}

// ── SPEC §1 ⟨0.13⟩ `Llm` — the Rust mirror of candor-java's LlmEffectTest ────────────────────────────

/// Helper: scan a fixture `--json` and return a fn's inferred effects (by fn-name substring).
fn llm_scan_effects(d: &std::path::Path, needle: &str) -> Vec<String> {
    let out = Command::new(bin()).arg(d.to_string_lossy().as_ref()).arg("--json").output().expect("run");
    let v: serde_json::Value =
        serde_json::from_str(String::from_utf8(out.stdout).unwrap().trim()).expect("json report");
    v["functions"].as_array().into_iter().flatten()
        .filter(|f| f["fn"].as_str().is_some_and(|q| q.ends_with(needle)))
        .flat_map(|f| f["inferred"].as_array().into_iter().flatten()
            .filter_map(|e| e.as_str().map(String::from)).collect::<Vec<_>>())
        .collect()
}

#[test]
fn llm_host_literal_refinement_keeps_net_and_adds_llm_only_for_model_hosts() {
    // (a) host-literal refinement: a statically-known request to a KNOWN model host carries Llm + Net
    // (Net is never dropped — a model call IS network I/O); an UNKNOWN host stays bare Net (never guessed);
    // a local Ollama endpoint (:11434) carries Llm too.
    let src = "\
        use std::net::TcpStream;\n\
        pub fn anthropic() { let _ = TcpStream::connect(\"api.anthropic.com:443\"); }\n\
        pub fn ollama() { let _ = TcpStream::connect(\"localhost:11434\"); }\n\
        pub fn other() { let _ = TcpStream::connect(\"example.com:443\"); }\n";
    let d = make_crate("llmhost", src);
    for m in ["anthropic", "ollama"] {
        let e = llm_scan_effects(&d, m);
        assert!(e.contains(&"Net".to_string()), "{m} must keep Net (a model call IS network I/O), got {e:?}");
        assert!(e.contains(&"Llm".to_string()), "{m} must carry Llm, got {e:?}");
    }
    let other = llm_scan_effects(&d, "other");
    assert!(other.contains(&"Net".to_string()), "an unknown host is Net, got {other:?}");
    assert!(!other.contains(&"Llm".to_string()), "an unknown host must NOT be Llm (never guessed), got {other:?}");
    let _ = std::fs::remove_dir_all(&d);
}

#[test]
fn deny_llm_gates_a_model_host_reach_and_names_llm() {
    let d = make_crate("denyllm",
        "pub fn chat() { let _ = std::net::TcpStream::connect(\"api.openai.com:443\"); }");
    let pol = d.join("p.policy");
    std::fs::write(&pol, "deny Llm chat\n").unwrap();
    let out = Command::new(bin())
        .arg(d.to_string_lossy().as_ref()).arg("--policy").arg(pol.to_string_lossy().as_ref())
        .output().expect("run");
    let all = format!("{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr));
    let _ = std::fs::remove_dir_all(&d);
    assert_eq!(out.status.code(), Some(1), "deny Llm on a model-host reach must fail the gate (exit 1):\n{all}");
    assert!(all.contains("AS-EFF-006") && all.contains("Llm"),
            "the AS-EFF-006 diagnostic must name Llm:\n{all}");
}

#[test]
fn allow_llm_fails_closed_on_a_masked_model_host() {
    // A runtime-computed host (structurally invisible) marks the Net surface incomplete. Because Llm rides
    // the Net host literal, `allow Llm` must fail closed too — a benign visible model host cannot MASK the
    // invisible one (the gate-evasion defense).
    let src = "\
        use std::net::TcpStream;\n\
        pub fn chat(h: &str) { let _ = TcpStream::connect(h); let _ = TcpStream::connect(\"api.openai.com:443\"); }\n";
    let d = make_crate("maskllm", src);
    let pol = d.join("p.policy");
    std::fs::write(&pol, "allow Llm in chat api.openai.com\n").unwrap();
    let out = Command::new(bin())
        .arg(d.to_string_lossy().as_ref()).arg("--policy").arg(pol.to_string_lossy().as_ref())
        .output().expect("run");
    let _ = std::fs::remove_dir_all(&d);
    assert_eq!(out.status.code(), Some(1),
        "an incomplete (masked) host surface must fail-close `allow Llm` — a benign model host cannot certify a hidden reach");
}

#[test]
fn model_sdk_crate_call_classifies_llm_and_net() {
    // (b) model-SDK surface: a call into a curated model-provider crate (async-openai) classifies Llm + Net
    // with NO method gating (single-purpose client) — the analog of java's isModelSdkOwner.
    let src = "\
        pub fn ask() { let c = async_openai::Client::new(); let _ = c.chat().create(); }\n";
    let d = make_crate("sdkllm", src);
    let e = llm_scan_effects(&d, "ask");
    let _ = std::fs::remove_dir_all(&d);
    assert!(e.contains(&"Llm".to_string()), "a call into a curated model-SDK crate must be Llm, got {e:?}");
    assert!(e.contains(&"Net".to_string()), "a model-SDK dispatch is also Net, got {e:?}");
}

/// ONE VERDICT PER (rule, function), even when two UNITS share one qualified name.
///
/// `#[cfg(unix)] fn f` beside `#[cfg(not(unix))] fn f` is the everyday shape, and both are analyzed —
/// so the gate's `all` list carried the name TWICE while `inferred` held one merged signature, and the
/// gate reported it twice: two byte-identical `GateViolation` records, an inflated
/// `N policy violation(s)` count, and a `--gate-json` document a consumer would read as two findings.
///
/// FOUND BY THE ⟨0.24⟩ §3.1 BYTE-EQUALITY OBLIGATION — `candor-query gate --report` over the same
/// report cannot reach the duplicate (a report is keyed by name), so the two routes disagreed on 15 of
/// 90 rows across ebman, pgman and the candor workspace. That is the argument for the verb: no
/// end-to-end test could have told this apart from a classifier defect.
#[test]
fn a_qualified_name_carried_by_two_cfg_gated_units_yields_one_violation_not_two() {
    let d = make_crate(
        "dupqual",
        "#[cfg(unix)]\npub fn twice() { let _ = std::fs::read_to_string(\"/etc/hosts\"); }\n\
         #[cfg(not(unix))]\npub fn twice() { let _ = std::fs::read_to_string(\"/etc/hosts\"); }\n\
         pub fn once() { let _ = std::fs::read_to_string(\"/tmp/x\"); }\n",
    );
    let pp = d.join("candor.policy");
    std::fs::write(&pp, "deny Fs\n").unwrap();
    let verdict = d.join("verdict.json");
    let out = Command::new(bin())
        .args([
            d.to_string_lossy().as_ref(),
            "--out",
            d.join("rep").to_string_lossy().as_ref(),
            "--policy",
            pp.to_string_lossy().as_ref(),
            "--gate-json",
            verdict.to_string_lossy().as_ref(),
        ])
        .output()
        .expect("run candor-scan");
    assert_eq!(out.status.code(), Some(1), "the deny-Fs gate must fire");
    let v: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&verdict).expect("a verdict")).unwrap();
    let fns: Vec<&str> =
        v["violations"].as_array().unwrap().iter().map(|x| x["fn"].as_str().unwrap()).collect();
    // THE CONTROL that keeps this from passing on an empty verdict: the singly-defined fn is there too,
    // so the fixture demonstrably fires, and `twice` appearing ONCE is a de-duplication rather than a drop.
    assert!(fns.contains(&"once"), "the gate must still catch the ordinary fn: {fns:?}");
    assert_eq!(
        fns.iter().filter(|f| **f == "twice").count(),
        1,
        "two cfg-gated units under one qualified name are ONE signature and must yield ONE violation \
         (the report is keyed by name, so `candor-query gate --report` cannot produce the duplicate — a \
         second record here is a scan-vs-gate divergence): {fns:?}"
    );
    // …and the report itself still lists both units, so this is a GATE de-duplication, not a lost entry.
    let rep = std::fs::read_to_string(d.join("rep.dupqual.scan.json")).expect("a report");
    assert_eq!(rep.matches("\"fn\": \"twice\"").count(), 2, "the report is unchanged: {rep}");
    let _ = std::fs::remove_dir_all(&d);
}

/// SPEC §3.3: *"A configured gate over incompletely-analyzed code MUST fail closed (exit ≠ 0); a real
/// violation (exit 1) still dominates."* Both halves, and the second one is the one that regressed.
///
/// MEASURED BEFORE THE FIX (2026-07-28), on a crate with one `deny Net` hit AND one unparseable file:
/// exit 2, and a `--gate-json` document reading `{ok:false, incomplete:true, violations: []}`. The two
/// AS-EFF-006 lines were printed to stderr and then DELETED from the document — the `had_parse_failure`
/// branch returned BEFORE `record_gate_violations`, so the accumulator the verdict is built from was
/// empty. The exit code was the lesser loss: a CI consumer reading gate.json saw a fail-closed verdict
/// with NOTHING in it, so the finding never reached the PR.
///
/// THE ASSERTION IS ON THE VIOLATION COUNT, not on the exit code — the count is what regressed, and an
/// exit-code-only test passed throughout. The `deny Db` row below is the CONTROL for the other half: no
/// violation to dominate, so the same crate must still fail closed at exit 2 with an empty list, which
/// is the shape the four-way completeness differential pins.
#[test]
fn a_violation_survives_an_incomplete_scan_and_dominates_the_exit_code() {
    let d = make_crate("incompleteviol", "pub mod broken;\npub fn fetch() { let _ = std::net::TcpStream::connect(\"api.example.com:80\"); }\n");
    std::fs::write(d.join("src/broken.rs"), "pub fn oops( { this is not rust\n").unwrap();
    let pp = d.join("net.policy");
    std::fs::write(&pp, "deny Net\n").unwrap();
    let verdict = d.join("verdict.json");
    let out = Command::new(bin())
        .args([
            d.to_string_lossy().as_ref(),
            "--out", d.join("rep").to_string_lossy().as_ref(),
            "--policy", pp.to_string_lossy().as_ref(),
            "--gate-json", verdict.to_string_lossy().as_ref(),
        ])
        .output()
        .expect("run candor-scan");
    let err = String::from_utf8_lossy(&out.stderr).into_owned();
    let v: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&verdict).expect("a verdict document")).unwrap();
    let fns: Vec<&str> =
        v["violations"].as_array().unwrap().iter().filter_map(|x| x["fn"].as_str()).collect();
    assert!(
        fns.contains(&"fetch"),
        "the violation must be IN the verdict document, not merely on stderr — an incomplete analysis \
         must not swallow a real finding (SPEC §3.3):\n{v:#}\nstderr:\n{err}"
    );
    assert_eq!(fns.len(), 1, "exactly the one real finding: {fns:?}");
    // …and the incompleteness is disclosed on the SAME document, not instead of it.
    assert_eq!(v["ok"], false, "a verdict with a violation is never ok:\n{v:#}");
    assert_eq!(v["incomplete"], true, "the manifest must still ride the verdict:\n{v:#}");
    assert_eq!(v["unanalyzed"][0]["path"], "src/broken.rs", "{v:#}");
    assert_eq!(out.status.code(), Some(1), "a real violation dominates the incomplete exit 2:\n{err}");

    // THE CONTROL — the same incomplete crate under a policy nothing violates still fails CLOSED, with
    // an empty violation list. Without this row the fix above could be "stopped failing closed".
    let dbp = d.join("db.policy");
    std::fs::write(&dbp, "deny Db\n").unwrap();
    let v2path = d.join("verdict2.json");
    let out2 = Command::new(bin())
        .args([
            d.to_string_lossy().as_ref(),
            "--out", d.join("rep").to_string_lossy().as_ref(),
            "--policy", dbp.to_string_lossy().as_ref(),
            "--gate-json", v2path.to_string_lossy().as_ref(),
        ])
        .output()
        .expect("run candor-scan");
    assert_eq!(out2.status.code(), Some(2), "no violation to dominate → the incomplete refusal stands");
    let v2: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&v2path).expect("a verdict document")).unwrap();
    assert_eq!(v2["ok"], false);
    assert_eq!(v2["incomplete"], true);
    assert_eq!(v2["violations"].as_array().unwrap().len(), 0, "{v2:#}");

    // ⟨0.24⟩ AND THE OTHER exit-2 cause writes a REFUSAL document — candor-spec `1503368` (b) removes
    // the carve-out. This row read `assert!(!v3path.exists(), "a broken gate CONFIG must still write no
    // verdict document")`, on the reasoning that a policy nobody could parse has no faithful verdict to
    // emit. True, and beside the point: the argument that mandates a document is that a CI wrapper of
    // the shape `candor-scan … --gate-json v.json || true; jq .ok v.json` re-reads the PREVIOUS run's
    // document as current, and a stale green does not care why this run declined to overwrite it.
    //
    // A refusal document is not a fabricated verdict — no `violations` key at all — which is why this is
    // consistent with the rule it replaces rather than a reversal of it. Run on a COMPLETE crate, so the
    // shape is attributable to the config and not to the manifest.
    let good = make_crate("incompleteviol-cfg", "pub fn go() {}\n");
    let v3path = good.join("verdict3.json");
    let _ = std::fs::remove_file(&v3path);
    let out3 = Command::new(bin())
        .args([
            good.to_string_lossy().as_ref(),
            "--policy", good.join("nope.policy").to_string_lossy().as_ref(),
            "--gate-json", v3path.to_string_lossy().as_ref(),
        ])
        .output()
        .expect("run candor-scan");
    assert_eq!(out3.status.code(), Some(2), "an unreadable policy is still exit 2");
    let v3: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&v3path).expect("a refusal document")).unwrap();
    assert_eq!(v3["ok"], false, "the naive read must be fail-closed:\n{v3:#}");
    assert_eq!(v3["refused"], true, "{v3:#}");
    assert!(v3.get("violations").is_none(), "a refusal makes NO claim about violations:\n{v3:#}");
    assert!(
        v3["reason"].as_str().unwrap().contains("could not be read"),
        "…and it names the cause, so the operator is not sent back to a scan they do not own:\n{v3:#}"
    );

    let _ = std::fs::remove_dir_all(&d);
    let _ = std::fs::remove_dir_all(&good);
}

/// SPEC §6.2 ⟨0.24⟩ **THE SCAN ROUTE TAKES THE SAME POLICY-ERROR RULE AS `gate --report`** (candor-spec
/// `382a7e0`). An unrecognised reason-class token used to be dropped with a warning, which REWRITES the
/// rule the operator wrote — and the direction that matters narrows it:
///
///   `deny Unknown[dispatch,indirct]` → gated on `[dispatch]` alone → **exit 0** over a crate whose only
///   hole is `indirect`. A gate that looks armed and covers nothing it was written to cover.
///
/// Both routes now refuse identically (exit 2, no verdict document), which is also what keeps §3.1's
/// byte-equality MUST true on a broken policy: neither route writes a document, so there is nothing to
/// disagree about.
#[test]
fn scan_refuses_a_policy_naming_an_unrecognised_reason_class_token() {
    // The one hole is INDIRECT (a call through a `&dyn Fn`), so the narrowing row's green is a real
    // miss, not a vacuous pass.
    let d = make_crate("badclass", "pub fn go(f: &dyn Fn() -> i32) -> i32 { f() }\n");
    let run = |name: &str, rule: &str, gate_json: Option<&std::path::Path>| -> (i32, String) {
        let pp = d.join(format!("{name}.policy"));
        std::fs::write(&pp, rule).unwrap();
        let mut args: Vec<String> =
            vec![d.to_string_lossy().into_owned(), "--policy".into(), pp.to_string_lossy().into_owned()];
        if let Some(g) = gate_json {
            args.push("--gate-json".into());
            args.push(g.to_string_lossy().into_owned());
        }
        let out = Command::new(bin()).args(&args).output().expect("run candor-scan");
        (out.status.code().unwrap_or(-1), String::from_utf8_lossy(&out.stderr).into_owned())
    };
    // CONTROL — spelled correctly the rule FIRES, so every row below is about the TOKEN.
    let (rc, err) = run("good", "deny Unknown[dispatch,indirect]\n", None);
    assert_eq!(rc, 1, "the correctly-spelled rule must fire, or the rows below prove nothing:\n{err}");

    let v = d.join("verdict.json");
    for (name, rule, token) in [
        // THE FAIL-OPEN ROW: exit 0 before the fix.
        ("typo_beside_valid", "deny Unknown[dispatch,indirct]\n", "indirct"),
        // The widening row: loud, but on a rule the engine claimed to be ignoring.
        ("sole_unrecognised", "deny Unknown[corp]\n", "corp"),
    ] {
        let _ = std::fs::remove_file(&v);
        let (rc, err) = run(name, rule, Some(&v));
        assert_eq!(rc, 2, "{name}: a policy that cannot be honoured AS WRITTEN must be refused:\n{err}");
        assert!(err.contains(token), "{name}: the refusal must NAME the token:\n{err}");
        // ⟨0.24⟩ …and take the UNREADABLE-POLICY posture, byte-identically to `candor-query gate
        // --report` on the same policy — which since candor-spec `1503368` (b) means a fail-closed
        // REFUSAL document rather than none at all. This row read `assert!(!v.exists())`. The
        // byte-equality obligation is what it always was; only the shape both routes must produce moved.
        let doc: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&v).expect("a refusal document")).unwrap();
        assert_eq!(doc["ok"], false, "{name}: {doc:#}");
        assert_eq!(doc["refused"], true, "{name}: {doc:#}");
        assert!(doc.get("violations").is_none(), "{name}: a refusal claims nothing about violations:\n{doc:#}");
        assert!(
            doc["reason"].as_str().unwrap().contains(token),
            "{name}: the document must name the token too — stderr is not the channel CI reads:\n{doc:#}"
        );
    }
    // A config-defined alias is vocabulary, not an error.
    std::fs::create_dir_all(d.join(".candor")).unwrap();
    std::fs::write(d.join(".candor/config"), "unknown-alias corp = indirect\n").unwrap();
    let (rc, err) = run("aliased", "deny Unknown[corp]\n", None);
    assert_eq!(rc, 1, "a defined alias resolves and the rule fires — the refusal must not eat ⟨0.19⟩:\n{err}");

    let _ = std::fs::remove_dir_all(&d);
}

/// SPEC §3.1 ⟨0.24⟩ **POLICY VOCABULARY ANCHORS AT THE POLICY FILE, ON BOTH ROUTES** (candor-spec
/// `99eb4e9`) — plus the disclosure that keeps it from acting unnamed.
///
/// §3.1 names three channels through which an effect must never enter a gate its report does not carry.
/// A review found a FOURTH that no engine tested: `.candor/config`'s `unknown-alias`. The scan route
/// anchored discovery at the **scan target** while all four `gate` verbs anchored at the **policy
/// file** — so with the policy filed outside the target, `scan --policy P` and `gate --report R --policy
/// P` expanded the same rule differently and **§3.1's byte-equality MUST was breakable by a file that is
/// neither the report nor the policy** (measured 2026-07-28: scan exit 1 / gate exit 0, two different
/// documents from one report and one policy).
///
/// Vocabulary travels with the policy that uses it. Target-scoped keys (`deps`, `net-partner`, scan
/// settings) still anchor at the target, because they describe the thing being scanned.
///
/// THE SECOND HALF IS THE DISCLOSURE: discovery walks PARENT directories, so an alias file anywhere
/// above participates — ambient, and until ⟨0.24⟩ invisible in the output. A verdict changed by a file
/// the operator cannot see named is the ambient-input failure this format exists to refuse, so the
/// `--gate-json` document names it.
#[test]
fn scan_resolves_policy_vocabulary_beside_the_policy_and_names_the_config_that_moved_the_verdict() {
    // The crate's only hole is INDIRECT (a call through `&dyn Fn`).
    let d = make_crate("anchorvocab", "pub fn go(f: &dyn Fn() -> i32) -> i32 { f() }\n");
    // The policy lives OUTSIDE the scan target, with its vocabulary beside it — the everyday shape for
    // an org-wide policy checked into its own repo.
    let home = std::env::temp_dir().join(format!("candor-polhome-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&home);
    std::fs::create_dir_all(home.join(".candor")).unwrap();
    let pp = home.join("org.policy");
    std::fs::write(&pp, "deny Unknown[corp]\n").unwrap();
    let cfgpath = home.join(".candor/config");

    let run = |verdict: &std::path::Path| -> (i32, String) {
        let _ = std::fs::remove_file(verdict);
        let out = Command::new(bin())
            .args([
                d.to_string_lossy().as_ref(),
                "--out", d.join("rep").to_string_lossy().as_ref(),
                "--policy", pp.to_string_lossy().as_ref(),
                "--gate-json", verdict.to_string_lossy().as_ref(),
            ])
            .output()
            .expect("run candor-scan");
        (out.status.code().unwrap_or(-1), String::from_utf8_lossy(&out.stderr).into_owned())
    };

    // (1) THE ANCHOR. `corp = indirect` beside the POLICY must resolve and the rule must FIRE. Before
    // the fix the scan looked only under the TARGET, found nothing, and the token was unresolvable —
    // which ⟨0.24⟩'s companion rung now reports as a policy error (exit 2), where it previously widened
    // the rule to a bare `deny Unknown` in silence.
    std::fs::write(&cfgpath, "unknown-alias corp = indirect\n").unwrap();
    let v = d.join("verdict.json");
    let (rc, err) = run(&v);
    assert_eq!(rc, 1, "an `unknown-alias` beside the POLICY must resolve on the SCAN route too:\n{err}");

    // (2) THE DISCLOSURE. The config that supplied the vocabulary is NAMED on the verdict, with the
    // alias it supplied — a verdict moved by a file the operator cannot see named is ambient input.
    let j: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(&v).unwrap()).unwrap();
    let named = j["policyVocabulary"]["config"].as_str().map(|s| std::fs::canonicalize(s).ok()).unwrap_or(None);
    assert_eq!(
        named,
        std::fs::canonicalize(&cfgpath).ok(),
        "the verdict must NAME the config whose vocabulary it used:\n{j:#}"
    );
    // ⟨0.24⟩ AN OBJECT — name → the classes it EXPANDED TO (SPEC §3.1 `7f5b5ba`). `["corp"]` names the
    // alias and drops the definition, and the definition is the half that moved the verdict: `corp =
    // indirect` and `corp = indirect,native` gate differently under the SAME policy line, so a reader
    // given only the name cannot tell which gate ran. `candor-query gate --report`'s counterpart row
    // carries the two-config differential in full; this one pins that the SCAN route emits the identical
    // shape, which is §3.1's byte-equality MUST one level down.
    assert_eq!(j["policyVocabulary"]["aliases"], serde_json::json!({"corp": ["indirect"]}), "{j:#}");
    // THE MIRROR: the object is a strict SUPERSET, so the alias NAME is still recoverable (the keys ARE
    // the old array) and the `config` path asserted just above is untouched.
    assert_eq!(
        j["policyVocabulary"]["aliases"].as_object().expect("an OBJECT").keys().collect::<Vec<_>>(),
        vec!["corp"],
        "{j:#}"
    );
    // …UNDER THE SPEC'S NAME. §3.1 ⟨0.24⟩ (`b4e9155`) pins the key as `policyVocabulary`, because the
    // verdict already carries other vocabularies (effects, reason classes) and the bare word does not say
    // WHOSE. This engine emitted `vocabulary` and was the last red cell in conformance PART 27's
    // `key-parity(opt)`. The old name is asserted ABSENT as well: an engine keeping both keys would
    // satisfy the new assertion while leaving the divergence exactly where it was.
    assert!(j.get("vocabulary").is_none(), "the pre-`b4e9155` key must not survive beside it:\n{j:#}");

    // (3) THE DISCRIMINATION CONTROL. Same anchor, different definition: `corp = reflect` does NOT match
    // an indirect hole, so the gate goes GREEN. Without this row (1) is satisfied by an engine that
    // ignores the alias and widens to a bare `deny Unknown`, which also exits 1 — the exact pre-fix
    // behaviour. The alias must be steering the verdict, not merely being present.
    std::fs::write(&cfgpath, "unknown-alias corp = reflect\n").unwrap();
    let v2 = d.join("verdict2.json");
    let (rc2, err2) = run(&v2);
    assert_eq!(rc2, 0, "the alias must NARROW the rule, not just unlock it:\n{err2}");

    // (4) AN UNUSED ALIAS IS NOT DISCLOSED — naming a file that changed nothing trains the reader to
    // ignore the field, and a verdict with no ambient vocabulary must stay byte-identical to pre-⟨0.24⟩.
    std::fs::write(&cfgpath, "unknown-alias corp = indirect\n").unwrap();
    std::fs::write(&pp, "deny Unknown\n").unwrap();
    let v3 = d.join("verdict3.json");
    let (rc3, _) = run(&v3);
    assert_eq!(rc3, 1);
    let j3: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(&v3).unwrap()).unwrap();
    assert!(j3.get("policyVocabulary").is_none(), "an alias the policy never mentions is not disclosed:\n{j3:#}");

    let _ = std::fs::remove_dir_all(&d);
    let _ = std::fs::remove_dir_all(&home);
}

/// ⟨0.24⟩ THE GATE'S OWN NOTE MUST DISCLOSE THE HOLE THE GATE JUST DECLINED TO CLEAR — SPEC §6.2, and
/// the ROUTE half of `a_narrowed_rule_the_gate_tolerates_is_a_hole_and_the_one_it_fires_on_is_not`.
///
/// A plain `--policy` scan auto-emits the provable-purity note (conformance PART 12d) from
/// `unverified_holes`, and that path carried TWO copies of the same defect:
///
///   - the shared predicate computed "PASSES" from `r.effects` alone, blind to the ⟨0.19⟩/⟨0.20⟩
///     narrowing filters, so a rule the gate TOLERATED read as violated and the hole was deleted; and
///   - this route's re-parse dropped the `.candor/config` vocabulary, so `deny Unknown[<alias>]` widened
///     to a bare `deny Unknown` — under which every hole is a violation and the note has nothing to say.
///     `ea0df4f` fixed the query verb; the same defect was standing here in the other copy.
///
/// Both arms in ONE run, because a fix that kills an over-charge is exactly where a silent under-report
/// gets introduced: `corp = reflect` does NOT match this crate's `indirect` hole (the gate tolerates ⇒
/// the note MUST name it), and `corp = indirect` DOES (the gate fires ⇒ it is a violation, and the note
/// MUST stay silent rather than report the gate's own finding back as an unproven pass).
#[test]
fn the_gate_note_discloses_a_hole_a_narrowed_rule_tolerates_and_stays_silent_on_one_it_fires_on() {
    let d = make_crate("gatenotefilter", "pub fn go(f: &dyn Fn() -> i32) -> i32 { f() }\n");
    let home = std::env::temp_dir().join(format!("candor-notehome-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&home);
    std::fs::create_dir_all(home.join(".candor")).unwrap();
    let pp = home.join("org.policy");
    let cfgpath = home.join(".candor/config");

    let run = |policy: &std::path::Path| -> (i32, String) {
        let out = Command::new(bin())
            .args([
                d.to_string_lossy().as_ref(),
                "--out", d.join("rep").to_string_lossy().as_ref(),
                "--policy", policy.to_string_lossy().as_ref(),
            ])
            .output()
            .expect("run candor-scan");
        (out.status.code().unwrap_or(-1), String::from_utf8_lossy(&out.stderr).into_owned())
    };

    // ── ARM 1, the fix: the filter does NOT match, so the gate tolerates and the note must speak. ──
    std::fs::write(&pp, "deny Unknown[reflect]\n").unwrap();
    let (rc, err) = run(&pp);
    assert_eq!(rc, 0, "`[reflect]` does not name this crate's `indirect` hole — the gate tolerates:\n{err}");
    assert!(
        err.contains("`go`  → add  `deny Unknown`"),
        "a rule the gate DECLINED to clear this function under leaves it unproven, and the note is the \
         only place that says so — it printed nothing:\n{err}"
    );

    // …and through an ALIAS, which is this route's own half of `ea0df4f`: the verdict path resolves the
    // vocabulary and the advisory re-parse used not to, so the rule widened and the note went quiet.
    std::fs::write(&pp, "deny Unknown[corp]\n").unwrap();
    std::fs::write(&cfgpath, "unknown-alias corp = reflect\n").unwrap();
    let (rc_a, err_a) = run(&pp);
    assert_eq!(rc_a, 0, "corp = reflect does not match an indirect hole:\n{err_a}");
    assert!(
        err_a.contains("`go`  → add  `deny Unknown`"),
        "the advisory re-parse must carry the SAME `.candor/config` vocabulary the verdict resolved \
         through, or it is reasoning about a rule the operator did not write:\n{err_a}"
    );

    // ── ARM 2, THE MIRROR: spell the same filter to MATCH. The gate FIRES, so this is a violation and
    // the note must NOT report it back as an unproven pass. Without this arm, arm 1 is satisfied by a
    // predicate that calls every Unknown function a hole. ──
    std::fs::write(&cfgpath, "unknown-alias corp = indirect\n").unwrap();
    let (rc_m, err_m) = run(&pp);
    assert_eq!(rc_m, 1, "corp = indirect DOES name this hole — the gate fires:\n{err_m}");
    assert!(
        !err_m.contains("→ add"),
        "a function the gate CHARGED is a violation, not an unverified pass — the note must not \
         disclose the gate's own finding a second time:\n{err_m}"
    );

    let _ = std::fs::remove_dir_all(&d);
    let _ = std::fs::remove_dir_all(&home);
}

/// ⟨0.24⟩ A CERTAIN BASELINE REGRESSION SURVIVES AN UNRELATED REFUSAL — SPEC §3.1 `4c79958`.
///
/// The worst shape this rung has produced: a pure fn gains an `Fs` call against a frozen baseline, and a
/// TYPO IN A POLICY TOKEN — which the regression has nothing to do with — used to delete the finding from
/// the `--gate-json` document. Exit 1 with `violations:["AS-EFF-005"]` became exit 2 with no `violations`
/// key at all, while the `[AS-EFF-005]` line stayed on stderr. The human kept the finding; CI lost it.
///
/// **THE EXIT CODE IS NOT WHERE THE HARM IS**, so the assertions are on the DOCUMENT. Both refusal causes
/// are covered — an unhonourable policy and an unreadable one — because the defect was in a predicate
/// (`exit 2 && nothing unanalyzed`) that neither cause was special to.
#[test]
fn a_certain_baseline_regression_stays_in_the_document_when_an_unrelated_policy_refuses() {
    let d = make_crate("blprec", "pub fn go() -> usize { 1 }");
    let pre = d.join("base");
    let (rc, _, _) = scan_with_baseline(&d, None, &["--out", pre.to_string_lossy().as_ref()]);
    assert_eq!(rc, Some(0), "recording the baseline is a plain scan");
    std::fs::write(d.join("src/lib.rs"), "pub fn go() -> usize { let _ = std::fs::read(\"/x\"); 1 }").unwrap();

    // Read the `--gate-json` document a run leaves behind. Written to a FRESH path each time so a
    // missing document can never be mistaken for a previous run's — the stale-verdict hazard this rung
    // exists to close would otherwise make the test pass by reading the control's answer.
    let verdict_of = |tag: &str, args: &[&str]| -> (Option<i32>, String, serde_json::Value) {
        let vp = d.join(format!("verdict-{tag}.json"));
        let _ = std::fs::remove_file(&vp);
        let mut a: Vec<&str> = vec!["--gate-json"];
        let vps = vp.to_string_lossy().to_string();
        a.push(&vps);
        a.extend_from_slice(args);
        let (rc, stdout, stderr) = scan_with_baseline(&d, Some(pre.to_string_lossy().as_ref()), &a);
        let doc = std::fs::read_to_string(&vp)
            .unwrap_or_else(|e| panic!("{tag}: no --gate-json document at all ({e}) — a consumer reading \
                                        that path gets the PREVIOUS run's answer:\n{stdout}{stderr}"));
        (rc, format!("{stdout}{stderr}"), serde_json::from_str(&doc).unwrap())
    };
    let has_regression = |v: &serde_json::Value| -> bool {
        v["violations"].as_array().is_some_and(|a| {
            a.iter().any(|gv| gv["rule"] == "AS-EFF-005" && gv["fn"] == "go")
        })
    };

    // ── THE CONTROL: no policy at all. Exit 1, the regression in the document. ──
    let (rc, all, ctl) = verdict_of("control", &[]);
    assert_eq!(rc, Some(1), "a gained effect is a violation: {all}");
    assert!(has_regression(&ctl), "control: the regression is in the document: {ctl}");

    // ── ARM 1: a policy carrying a token that cannot be honoured (SPEC §6.2). ──
    let bad = d.join("bad.policy");
    std::fs::write(&bad, "deny Unknown[dispatch,nativ]\n").unwrap();
    let (rc, all, v) = verdict_of("badtoken", &["--policy", bad.to_string_lossy().as_ref()]);
    // ⟨0.27⟩ EXIT 1, NOT 2 — this asserted 2, on the reading that "precedence binds the VERDICT, not the
    // policy gate". SPEC §3.1 states the ordering in EXIT CODES: "The order is violation (1) > refusal
    // (2) > incomplete (2) … Exit 1 is therefore not merely fail-closed here, it is CERTAIN, and it is
    // strictly more informative than exit 2: it names the violation." java, ts and swift all exit 1 on
    // this shape; this engine was alone, and the split was found by a cross-engine differential.
    //
    // The narrow reading was also inconsistent with this engine's own code: the incomplete-analysis arm
    // fifty lines away already lets a real regression dominate, citing the same principle. Refusal and
    // incomplete sit at the SAME rank in the ordering, so a regression cannot dominate one and not the
    // other.
    assert_eq!(rc, Some(1), "a certain regression DOMINATES a refusal beside it (SPEC §3.1): {all}");
    assert!(
        has_regression(&v),
        "THE FINDING: a typo in a policy token must not delete a certain baseline regression from the \
         machine channel — precedence binds the VERDICT, not the policy gate (SPEC §3.1): {v}"
    );
    // …and the refusal is NOT swallowed by the rescue. Without this the mirror is a document reading
    // `{ok:false, violations:[AS-EFF-005]}`, from which an operator concludes the gate ran and passed.
    //
    // ⟨0.27⟩ THE CHANNEL CHANGED, AND THE OLD ONE IS NOW FORBIDDEN (SPEC §3.1's composed-document
    // clause). This test asserted `refused: true` beside `violations` — but `refused` is the refusal
    // document's DISCRIMINATOR, whose pinned meaning ("the gate is making no claim about violations")
    // contradicts a document that carries them; measured, the four engines wrote four spellings of this
    // one document. The disclosure travels as `unevaluated` instead: one entry PER RULE of the refused
    // policy, the raw line verbatim, so no rule silently reads as evaluated-and-passed.
    assert_eq!(v["ok"], serde_json::json!(false), "a refused run is never ok: {v}");
    assert!(v.get("refused").is_none(), "a violations-bearing document is a VERDICT and must not carry \
             the refusal document's discriminator (SPEC §3.1 ⟨0.27⟩): {v}");
    assert!(
        v["unevaluated"].as_array().is_some_and(|a| a
            .iter()
            .any(|u| u["rule"] == "deny Unknown[dispatch,nativ]"
                && u["why"].as_str().is_some_and(|s| s.contains("nativ")))),
        "the refused rule rides `unevaluated`, verbatim, with the token named in its why: {v}"
    );

    // ── ARM 2: an UNREADABLE policy — the other refusal cause, same predicate. ──
    let (rc, all, v) = verdict_of("unreadable", &["--policy", "/nonexistent/candor.policy"]);
    // Same precedence as ARM 1, and for the same reason: the refusal CAUSE does not change the ordering.
    assert_eq!(rc, Some(1), "a certain regression dominates this refusal too (SPEC §3.1): {all}");
    assert!(has_regression(&v), "an unreadable policy must not delete it either: {v}");
    assert!(v.get("refused").is_none(), "no `refused` on a verdict here either (SPEC §3.1 ⟨0.27⟩): {v}");
    assert!(
        v["unevaluated"].as_array().is_some_and(|a| a
            .iter()
            .any(|u| u["rule"].as_str().is_some_and(|s| s.contains("entire policy")))),
        "an unreadable policy has no lines to name, so ONE entry names the whole file — an exit-1 \
         document with violations and no `unevaluated` claims the policy ran and passed: {v}"
    );

    // ── THE MIRROR, MEASURED: with NO violation to carry, a refusal is still the MINIMAL document with
    // NO `violations` key. `[]` is precisely the claim a refusal cannot make, and a fix that rescued the
    // violation by always emitting the full verdict would have fabricated it here. ──
    std::fs::write(d.join("src/lib.rs"), "pub fn go() -> usize { 1 }").unwrap(); // back to the baseline
    let (rc, all, v) = verdict_of("norepr", &["--policy", bad.to_string_lossy().as_ref()]);
    let _ = std::fs::remove_dir_all(&d);
    assert_eq!(rc, Some(2), "still a refusal: {all}");
    assert_eq!(v["refused"], serde_json::json!(true), "still a refusal document: {v}");
    assert!(
        v.get("violations").is_none(),
        "MIRROR: a refusal with nothing established must carry NO `violations` key — an empty array \
         reads as \"we looked and found none\", which is the fabrication this format refuses: {v}"
    );
}

// ── SPEC §3.3.1 ⟨0.28⟩ — the arming rung must never destroy an INPUT, and must never hand back what
// the run did not re-earn. Four data-destroying defects from the adversarial review of the rung, each
// pinned on the BYTES of the file at risk: an exit-code assertion alone cannot see these regress,
// because every one of them already "failed" with a plausible exit 2. ──

/// A helper for this section: the previous bytes of a path, asserted unchanged after a run.
fn bytes_of(p: &std::path::Path) -> Vec<u8> {
    std::fs::read(p).unwrap_or_else(|e| panic!("read {}: {e}", p.display()))
}

#[test]
fn gate_json_naming_the_scan_target_is_refused_before_anything_is_written() {
    // CRITICAL (⟨0.28⟩ (3)): the target's own source tree is an INPUT of the run, and `run_inputs` did
    // not register it. Measured before the fix: `candor-scan src/lib.rs --gate-json src/lib.rs`
    // replaced the operator's SOURCE FILE with the armed verdict document and exited 0 — the sink guard
    // covered every input channel except the one every run has.
    let d = make_crate("gatetarget", "pub fn go() {}\n");
    let lib = d.join("src/lib.rs");
    let before = bytes_of(&lib);

    // The FILE-target spelling — the one that destroyed data (a directory target merely failed the write).
    let out = Command::new(bin())
        .args([lib.to_string_lossy().as_ref(), "--gate-json", lib.to_string_lossy().as_ref()])
        .env_remove("CANDOR_BASELINE").env_remove("CANDOR_POLICY").env_remove("CANDOR_CONFIG").env_remove("CANDOR_DEPS")
        .output().expect("run candor-scan");
    let stderr = String::from_utf8(out.stderr).unwrap();
    assert_eq!(out.status.code(), Some(2),
        "a gate sink naming the scan target must be refused (exit 2), not scanned: {stderr}");
    assert!(stderr.contains("the scan target"),
        "the refusal must name the colliding input channel: {stderr}");
    assert_eq!(bytes_of(&lib), before,
        "the scan target's bytes must be untouched — before the fix this file held the verdict placeholder");

    // The directory-target spelling: refused for the same reason, under the same rule.
    let out = Command::new(bin())
        .args([d.to_string_lossy().as_ref(), "--gate-json", d.to_string_lossy().as_ref()])
        .env_remove("CANDOR_BASELINE").env_remove("CANDOR_POLICY").env_remove("CANDOR_CONFIG").env_remove("CANDOR_DEPS")
        .output().expect("run candor-scan");
    let _ = std::fs::remove_dir_all(&d);
    assert_eq!(out.status.code(), Some(2), "a directory target as the sink is the same collision");
}

#[test]
fn duplicate_gate_json_refusal_never_lands_on_a_chained_dep_report() {
    // CRITICAL: `gate_json_input_collision` (the DUPLICATE-sink route) re-derived the input set by hand
    // and its copy omitted CANDOR_DEPS/CANDOR_BASELINE/the config's keys — so the repeated-`--gate-json`
    // refusal, which is deliberately written to EVERY named sink, destroyed the operator's dep report.
    // Measured: `CANDOR_DEPS=R --gate-json R` refused with R intact (the single-sink route reads
    // `run_inputs`), while `--gate-json R --gate-json V` wrote the refusal document OVER R. The two
    // routes now ask one spelling of one question.
    let d = make_crate("dupdep", "pub fn go() {}\n");
    let dep = d.join("dep");
    let (rc, _, _) = scan_with_baseline(&d, None, &["--out", dep.to_string_lossy().as_ref()]);
    assert_eq!(rc, Some(0), "recording the dep report is a plain scan");
    let dep_report = d.join("dep.dupdep.scan.json");
    let before = bytes_of(&dep_report);
    let other = d.join("other-verdict.json");

    let out = Command::new(bin())
        .args([d.to_string_lossy().as_ref(),
               "--gate-json", dep_report.to_string_lossy().as_ref(),
               "--gate-json", other.to_string_lossy().as_ref()])
        .env_remove("CANDOR_BASELINE").env_remove("CANDOR_POLICY").env_remove("CANDOR_CONFIG")
        .env("CANDOR_DEPS", dep_report.to_string_lossy().as_ref())
        .output().expect("run candor-scan");
    assert_eq!(out.status.code(), Some(2), "a repeated --gate-json is refused");
    assert_eq!(bytes_of(&dep_report), before,
        "the dep report is an INPUT of this run — the duplicate refusal must not be written over it");
    // …while the innocent sink still gets its refusal: its reader must be able to learn it lost.
    let v: serde_json::Value = serde_json::from_slice(&bytes_of(&other)).expect("other sink holds JSON");
    let _ = std::fs::remove_dir_all(&d);
    assert_eq!(v["refused"], serde_json::json!(true), "the non-input sink carries the refusal: {v}");
}

#[test]
fn a_failing_run_never_arms_the_prefix_form_baseline_it_reads() {
    // CRITICAL (the dep-DIRECTORY lesson un-applied to its sibling): `run_inputs` registered the raw
    // CANDOR_BASELINE string, but a prefix value RESOLVES to `<value>.<crate>.scan.json` (+ the
    // callgraph sidecar `check_baseline` reads beside it), and `same_artifact("base",
    // "base.app.scan.json")` is false. Measured: `CANDOR_BASELINE=base candor-scan . --out base
    // --zzz-not-a-flag` exited 2 having replaced the ratchet's baseline — a file this run READS — with
    // the placeholder. Permanently: the argv never stops failing, so no later run rewrites it.
    let d = make_crate("blarm", "pub fn go() { let _ = std::fs::read(\"/x\"); }");
    let pre = d.join("base");
    let (rc, _, _) = scan_with_baseline(&d, None, &["--out", pre.to_string_lossy().as_ref()]);
    assert_eq!(rc, Some(0), "recording the baseline is a plain scan");
    let report = d.join("base.blarm.scan.json");
    let sidecar = d.join("base.blarm.scan.callgraph.json");
    let (report_before, sidecar_before) = (bytes_of(&report), bytes_of(&sidecar));

    let (rc, _, stderr) = scan_with_baseline(&d, Some(pre.to_string_lossy().as_ref()),
        &["--out", pre.to_string_lossy().as_ref(), "--zzz-not-a-flag"]);
    assert_eq!(rc, Some(2), "the unknown flag still refuses: {stderr}");
    assert_eq!(bytes_of(&report), report_before,
        "the baseline report this run READS must survive a failing argv — before the fix it held the placeholder");
    assert_eq!(bytes_of(&sidecar), sidecar_before,
        "…and the callgraph sidecar `check_baseline` reads beside it, which no channel registered at all");
    assert!(stderr.contains("would arm over"),
        "the skip is DISCLOSED, not silent — the operator learns their baseline sat in the arming path: {stderr}");
    let _ = std::fs::remove_dir_all(&d);
}

#[test]
fn the_baseline_update_workflow_still_writes_fresh_reports_over_the_skipped_arming() {
    // The workflow this repo's own error text prescribes ("Commit it, or record one: candor-scan {dir}
    // --out {value}"): CANDOR_BASELINE=X with --out X. Arming must SKIP the baseline files (they are
    // inputs) with the existing diagnostic — `arm_out_prefix` uses `continue` + a warning rather than
    // exiting precisely so this composes — and the completed run must still write its new reports.
    let d = make_crate("blupdate", "pub fn go() { let _ = std::fs::read(\"/x\"); }");
    let pre = d.join("base");
    let (rc, _, _) = scan_with_baseline(&d, None, &["--out", pre.to_string_lossy().as_ref()]);
    assert_eq!(rc, Some(0));

    let (rc, _, stderr) = scan_with_baseline(&d, Some(pre.to_string_lossy().as_ref()),
        &["--out", pre.to_string_lossy().as_ref()]);
    assert_eq!(rc, Some(0), "re-recording over an unchanged crate is a clean run: {stderr}");
    assert!(stderr.contains("would arm over"), "the input skip is disclosed on the update too: {stderr}");
    let v: serde_json::Value = serde_json::from_slice(&bytes_of(&d.join("base.blupdate.scan.json")))
        .expect("the update wrote a fresh report");
    let _ = std::fs::remove_dir_all(&d);
    assert_eq!(v["analyzed"]["count"], serde_json::json!(1),
        "the run's OWN write phase is unaffected by the arming skip — a real report, not a placeholder: {v}");
}

#[test]
fn a_deps_run_that_fails_before_scanning_leaves_the_placeholders_standing() {
    // CRITICAL: `run_with_deps` RETURNS 2 on a missing Cargo.lock, and the disarm hand-back ran
    // whenever control returned — so a run that failed before writing ANYTHING restored the previous
    // run's green reports, the precise state the arming exists to destroy (⟨0.24⟩: "not left holding a
    // previous run's answer"). The hand-back is now licensed by the write phase completing, not by
    // being reached.
    let d = make_crate("depsarm", "pub fn go() {}\n"); // make_crate writes no Cargo.lock
    let pre = d.join("pre");
    let (rc, _, _) = scan_with_baseline(&d, None, &["--out", pre.to_string_lossy().as_ref()]);
    assert_eq!(rc, Some(0), "the previous good run records its report");
    let report = d.join("pre.depsarm.scan.json");
    let stale_green = bytes_of(&report);

    let (rc, _, stderr) = scan_with_baseline(&d, None,
        &["--deps", "--out", pre.to_string_lossy().as_ref()]);
    assert_eq!(rc, Some(2), "--deps without a Cargo.lock refuses: {stderr}");
    let now = bytes_of(&report);
    assert_ne!(now, stale_green,
        "a run that failed before its write phase must NOT hand the previous run's report back");
    assert!(String::from_utf8_lossy(&now).contains("\"reason\": \"armed:"),
        "what stands is the armed placeholder — a non-claim, not a stale claim: {}",
        String::from_utf8_lossy(&now));

    // The control: a COMPLETED run over the same prefix still hands back what it did not own (the
    // orphan rule) — the license keys on the write phase, not on the exit code.
    let orphan = d.join("pre.gone.scan.json");
    std::fs::write(&orphan,
        "{\n  \"candor\": { \"version\": \"scan-x\", \"toolchain\": \"stable\", \"spec\": \"0.27\" },\n  \"functions\": []\n}\n").unwrap();
    let orphan_before = bytes_of(&orphan);
    let (rc, _, _) = scan_with_baseline(&d, None, &["--out", pre.to_string_lossy().as_ref()]);
    assert_eq!(rc, Some(0));
    assert_eq!(bytes_of(&orphan), orphan_before,
        "a completed run still restores the orphan it armed but did not overwrite");
    let _ = std::fs::remove_dir_all(&d);
}

#[test]
fn a_sink_named_after_a_broken_flag_is_still_a_sink() {
    // SPEC §3.2 ⟨0.28⟩, the "given no value" ruling — and the successor to a test that pinned the
    // OPPOSITE. While the loop consumed a flag-shaped token as a value, `--policy --out X` meant
    // *policy = the file named `--out`*, so X really was never accepted and this test asserted
    // nothing could be armed under it. The ruling overturned the premise: a flag-shaped token after
    // a value-taking flag is NOT a value (usage error, exit 2), so `--out X` here is parsed as
    // itself — the run has a broken command line, not a redefined one — and X IS this run's declared
    // prefix. What must stand under it after the refusal is the fail-closed placeholder, never the
    // previous run's green.
    let d = make_crate("prepassout", "pub fn go() {}\n");
    let pre = d.join("X");
    let (rc, _, _) = scan_with_baseline(&d, None, &["--out", pre.to_string_lossy().as_ref()]);
    assert_eq!(rc, Some(0), "the previous good run records its report");
    let report = d.join("X.prepassout.scan.json");
    let stale_green = bytes_of(&report);

    let (rc, _, stderr) = scan_with_baseline(&d, None, &["--policy", "--out", pre.to_string_lossy().as_ref()]);
    assert_eq!(rc, Some(2), "--policy was given no value (the next token is a flag): {stderr}");
    assert!(stderr.contains("--policy"), "the refusal names the broken flag, not the sink: {stderr}");
    let now = bytes_of(&report);
    assert_ne!(now, stale_green,
        "X is still this run's --out prefix — the previous run's green must not stand as current");
    assert!(String::from_utf8_lossy(&now).contains("\"reason\": \"armed:"),
        "what stands is the armed placeholder — a non-claim, not a stale claim: {}",
        String::from_utf8_lossy(&now));

    // The verdict-sink sibling: `--out --gate-json V` — `--gate-json V` stays live past the broken
    // `--out`, so V is a sink and the fail-closed refusal document MUST reach it (it used to be
    // swallowed as --out's value and received nothing — conformance §3.1 (b13)'s file spelling).
    let v = d.join("V.json");
    let (rc, _, stderr) = scan_with_baseline(&d, None, &["--out", "--gate-json", v.to_string_lossy().as_ref()]);
    assert_eq!(rc, Some(2), "a valueless --out refuses: {stderr}");
    let doc: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&v).expect("V holds the refusal")).expect("valid JSON");
    assert_eq!(doc["ok"], false, "fail-closed at the sink the broken command line still named: {doc}");
    assert_eq!(doc["refused"], true, "{doc}");
    let _ = std::fs::remove_dir_all(&d);
}

/// SPEC §3.3.1 ⟨0.28⟩: **a repeated `--out` is the same rule as a repeated `--gate-json`** — refused
/// at exit 2, with the fail-closed report written to EVERY prefix named, under the report sink's own
/// arming rules (each prefix's previous report set rewritten to the ⟨0.21⟩ Row-1 no-claim shape, its
/// §2.2 sidecars deleted with it, and NO hand-back — the run scanned nothing). Measured before the
/// fix: `--out A --out B` took the LAST at exit 0, leaving `A` holding the previous run's whole
/// per-crate report set, readable as current, with nothing saying otherwise.
#[test]
fn repeated_out_is_refused_and_every_named_prefix_gets_the_fail_closed_report() {
    let d = make_crate("repout", "pub fn go() { let _ = std::fs::read(\"x\"); }");
    let tmp = std::env::temp_dir().join(format!("candor-scan-repout-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&tmp);
    std::fs::create_dir_all(&tmp).unwrap();
    let (p1, p2) = (tmp.join("o1"), tmp.join("o2"));
    let (p1s, p2s) = (p1.to_string_lossy().into_owned(), p2.to_string_lossy().into_owned());

    // Seed both prefixes with a PREVIOUS run's real report set (report + callgraph sidecar).
    for p in [&p1s, &p2s] {
        let out = Command::new(bin()).arg(d.to_string_lossy().as_ref()).args(["--out", p])
            .output().expect("seed run");
        assert_eq!(out.status.code(), Some(0), "seeding scan must succeed");
    }
    let rep = |p: &str| format!("{p}.repout.scan.json");
    let side = |p: &str| format!("{p}.repout.scan.callgraph.json");
    assert!(std::path::Path::new(&side(&p1s)).exists(), "seed left a sidecar to observe");

    let out = Command::new(bin())
        .arg(d.to_string_lossy().as_ref())
        .args(["--out", &p1s, "--out", &p2s])
        .output().expect("run candor-scan");
    assert_eq!(out.status.code(), Some(2),
        "a repeated --out is refused — last-wins published a previous run's reports at the losing prefix");
    let stderr = String::from_utf8(out.stderr).unwrap();
    assert!(stderr.contains("--out given more than once"), "the diagnostic names the rule: {stderr}");

    // EVERY prefix named gets the fail-closed report: the previous sets are armed, not left current…
    for p in [&p1s, &p2s] {
        let v: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(rep(p)).unwrap()).unwrap();
        assert_eq!(v["analyzed"]["count"], serde_json::json!(0),
            "{p}: the previous report must be the Row-1 no-claim shape after the refusal: {v}");
        assert_eq!(v["functions"], serde_json::json!([]));
        assert!(!std::path::Path::new(&side(p)).exists(),
            "{p}: an armed report's §2.2 sidecar goes with it — a live sidecar beside a no-claim \
             report is a pair that contradicts itself");
    }

    // …and the run exited before scanning, so NO hand-back: the placeholders STAND (fail-closed).

    // CONTROLS. (a) A single --out still scans: exit 0, a real report.
    let out = Command::new(bin()).arg(d.to_string_lossy().as_ref()).args(["--out", &p1s])
        .output().expect("run candor-scan");
    assert_eq!(out.status.code(), Some(0), "single --out control");
    let v: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(rep(&p1s)).unwrap()).unwrap();
    assert_eq!(v["analyzed"]["count"], serde_json::json!(1), "the control writes a REAL report: {v}");

    // (b) Two spellings of ONE path are ONE sink (the §3.3.1 artifact rule), not refused.
    let out = Command::new(bin())
        .current_dir(&tmp)
        .arg(d.to_string_lossy().as_ref())
        .args(["--out", "o2", "--out", "./o2"])
        .output().expect("run candor-scan");
    assert_eq!(out.status.code(), Some(0),
        "two spellings of one prefix are one sink — refusing a legal command is the mirror defect: {}",
        String::from_utf8_lossy(&out.stderr));

    let _ = std::fs::remove_dir_all(&d);
    let _ = std::fs::remove_dir_all(&tmp);
}

/// SPEC §3.3.1 ⟨0.28⟩: **a sink under the scan target that bears an extension this engine parses is
/// refused**, having written nothing — the residual the exact-artifact rule left. Measured before the
/// fix: `candor-scan . --policy P --gate-json src/lib.rs` replaced the operator's SOURCE FILE with the
/// armed verdict, then reported the file it had just destroyed as a parse failure. EXACT scope, never
/// containment: `<dir>/.candor/verdict.json` is under the target and not source — the recommended
/// layout — and a `.rs` sink OUTSIDE the target is not this rule; both are pinned as controls.
#[test]
fn gate_json_naming_parsed_source_under_the_target_is_refused_and_candor_layout_still_works() {
    let d = make_crate("srcsink", "pub fn go() { let _ = std::fs::read(\"x\"); }");
    let pp = d.join("candor.policy");
    std::fs::write(&pp, "deny Exec\n").unwrap();
    let src = d.join("src/lib.rs");
    let before = std::fs::read(&src).unwrap();

    // (1) The defect route, single sink: refused at exit 2, source byte-identical.
    let out = Command::new(bin())
        .current_dir(&d)
        .args([".", "--policy", "candor.policy", "--gate-json", "src/lib.rs"])
        .output().expect("run candor-scan");
    assert_eq!(out.status.code(), Some(2),
        "a .rs sink under the target must be refused — arming would overwrite source the run is \
         about to parse: {}", String::from_utf8_lossy(&out.stderr));
    assert_eq!(std::fs::read(&src).unwrap(), before,
        "NOTHING is written to the refused sink — before the fix the verdict replaced the source file");

    // (2) The duplicate route shares the predicate: the source path is exempt (nothing written),
    // the innocent sink still gets the duplicate refusal document.
    let v2 = d.join("v2.json");
    let out = Command::new(bin())
        .current_dir(&d)
        .args([".", "--policy", "candor.policy", "--gate-json", "src/lib.rs",
               "--gate-json", v2.to_string_lossy().as_ref()])
        .output().expect("run candor-scan");
    assert_eq!(out.status.code(), Some(2));
    assert_eq!(std::fs::read(&src).unwrap(), before, "source intact on the duplicate route too");
    let v: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(&v2).unwrap()).unwrap();
    assert_eq!(v["refused"], serde_json::json!(true), "the other named sink learns it lost: {v}");

    // (3) CONTROL — the recommended layout: a NON-source sink under the target still gates for real.
    let out = Command::new(bin())
        .current_dir(&d)
        .args([".", "--policy", "candor.policy", "--gate-json", ".candor/verdict.json"])
        .output().expect("run candor-scan");
    assert_eq!(out.status.code(), Some(0),
        "a verdict into .candor/ INSIDE the tree being scanned is ordinary usage — a rule that \
         refuses any sink under the target refuses the default: {}",
        String::from_utf8_lossy(&out.stderr));
    let v: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(d.join(".candor/verdict.json")).unwrap()).unwrap();
    assert_eq!(v["ok"], serde_json::json!(true), "a REAL verdict, not the armed placeholder: {v}");

    // (4) CONTROL — a .rs sink OUTSIDE the target is not this rule (it is nobody's input).
    let outside = std::env::temp_dir().join(format!("candor-scan-outside-{}.rs", std::process::id()));
    let _ = std::fs::remove_file(&outside);
    let out = Command::new(bin())
        .current_dir(&d)
        .args([".", "--policy", "candor.policy", "--gate-json", outside.to_string_lossy().as_ref()])
        .output().expect("run candor-scan");
    assert_eq!(out.status.code(), Some(0), "a .rs path outside the target is a legal (odd) sink");
    let _ = std::fs::remove_file(&outside);
    let _ = std::fs::remove_dir_all(&d);
}

/// SPEC §6.2 ⟨0.28⟩: the scan route's `--gate-json` verdict carries `ignored: [{line, text, reason}]`
/// for every policy line the parse dropped, omitted when nothing was dropped. The per-line stderr
/// warnings are unchanged — this is their machine half, and before the fix the verdict document was
/// silent while stderr warned (a route is not covered by its sibling: candor-query gate is pinned in
/// its own crate's tests).
#[test]
fn scan_gate_json_carries_dropped_policy_lines_as_ignored() {
    let d = make_crate("ignoredscan", "pub fn go() { let _ = std::fs::read(\"x\"); }");
    let pp = d.join("candor.policy");
    std::fs::write(&pp, "deny Exec\nfrobnicate the walrus\n").unwrap();
    let sink = d.join("v.json");

    let out = Command::new(bin())
        .arg(d.to_string_lossy().as_ref())
        .args(["--policy", pp.to_string_lossy().as_ref(), "--gate-json", sink.to_string_lossy().as_ref()])
        .output().expect("run candor-scan");
    assert_eq!(out.status.code(), Some(0),
        "a dropped line changes NEITHER ok NOR the exit — the leniency is unchanged, only disclosed: {}",
        String::from_utf8_lossy(&out.stderr));
    let v: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(&sink).unwrap()).unwrap();
    assert_eq!(v["ok"], serde_json::json!(true));
    let ig = v["ignored"].as_array().unwrap_or_else(|| panic!(
        "the verdict must carry the dropped line — stderr is not the machine channel: {v}"));
    assert_eq!(ig[0]["line"], serde_json::json!(2));
    assert_eq!(ig[0]["text"], serde_json::json!("frobnicate the walrus"));
    assert!(ig[0]["reason"].as_str().unwrap().contains("unknown rule kind"), "{v}");

    // CONTROL: a clean policy's verdict has no `ignored` key (byte-identity pinned out of band).
    std::fs::write(&pp, "deny Exec\n").unwrap();
    let out = Command::new(bin())
        .arg(d.to_string_lossy().as_ref())
        .args(["--policy", pp.to_string_lossy().as_ref(), "--gate-json", sink.to_string_lossy().as_ref()])
        .output().expect("run candor-scan");
    assert_eq!(out.status.code(), Some(0));
    let v: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(&sink).unwrap()).unwrap();
    assert!(v.get("ignored").is_none(), "a clean policy's verdict stays byte-identical: {v}");
    let _ = std::fs::remove_dir_all(&d);
}

/// ⟨0.29⟩ THE NET LOCATOR CAN LIVE AT ARGUMENT 1, AND A BIND ADDRESS IS NOT A DESTINATION.
///
/// Two certify-direction fixes that shipped with NO test and were caught by a release panel for exactly
/// that — this project's own rule is that the OVER-CHARGE CONTROL is the deliverable and the second
/// fixture gets written first, and both commits broke it. Pinned here together because they are the same
/// hazard from opposite sides: one restores a capture that a fix had deleted, the other removes a capture
/// that was never a destination.
///
/// (a) `positional_str_lit(args, 0)` became the universal default when `first_str_lit` was removed. That
/// is right for `Fs`/`Db`/`Exec`, whose locator is argument 0, and WRONG for `reqwest::Client::request`,
/// whose signature is `(Method, url)` — the URL stopped being captured and the call could no longer be
/// certified. The direction was safe, so nothing failed; the surface simply disappeared.
///
/// (b) `UdpSocket::bind("0.0.0.0:0")` put a LOCAL address into `hosts`, the DESTINATION surface `allow
/// Net` gates on, and being a captured literal it made the surface look complete — so `allow Net 0.0.0.0`
/// certified a `send_to` to a runtime endpoint. Withholding the literal is the whole fix: an empty
/// surface fails closed on its own (asserted below), which is why no extra `incomplete` hedge is needed —
/// the first attempt added one and cost every UDP client its certification.
#[test]
fn the_net_locator_position_and_the_bind_address_rule() {
    let d = make_crate(
        "netlocator",
        "use std::net::{UdpSocket, TcpStream};
         pub fn arg1_lit(c: &reqwest::Client) { let _ = c.request(reqwest::Method::GET, \"https://api.example.com/v1\"); }
         pub fn arg1_runtime(c: &reqwest::Client, u: &str) { let _ = c.request(reqwest::Method::GET, u); }
         pub fn arg0_lit(c: &reqwest::Client) { let _ = c.get(\"https://api.example.com/v1\"); }
         pub fn bind_then_dest() { let _ = UdpSocket::bind(\"0.0.0.0:0\"); let _ = TcpStream::connect(\"api.example.com:443\"); }
         pub fn bind_only() { let _ = UdpSocket::bind(\"0.0.0.0:0\"); }
",
    );
    let out = Command::new(bin())
        .args([d.to_string_lossy().as_ref(), "--out", d.join("rep").to_string_lossy().as_ref()])
        .output()
        .expect("run candor-scan");
    assert!(out.status.success(), "scan failed: {}", String::from_utf8_lossy(&out.stderr));
    let rep = std::fs::read_dir(&d).unwrap().filter_map(Result::ok).map(|e| e.path())
        .find(|p| p.to_string_lossy().contains("rep.") && p.to_string_lossy().ends_with(".scan.json")
                  && !p.to_string_lossy().contains("callgraph"))
        .expect("a report");
    let v: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(rep).unwrap()).unwrap();
    let hosts = |name: &str| -> Vec<String> {
        v["functions"].as_array().unwrap().iter()
            .find(|f| f["fn"].as_str() == Some(name))
            .and_then(|f| f["hosts"].as_array().cloned())
            .map(|a| a.iter().filter_map(|x| x.as_str().map(str::to_string)).collect())
            .unwrap_or_default()
    };
    // (a) the locator at argument 1 is captured — the regression this pins.
    assert_eq!(hosts("arg1_lit"), vec!["api.example.com"],
               "`request(Method, url)` puts its URL at argument 1; reading only position 0 silently \
                deleted this surface and the call could no longer be certified");
    // …and the OVER-CHARGE CONTROL: a runtime URL in that same position fabricates nothing.
    assert!(hosts("arg1_runtime").is_empty(),
            "a RUNTIME argument-1 url must capture no host — the position rule must not become \
             'try the next argument until something sticks', which is the literal-anywhere hazard again");
    assert_eq!(hosts("arg0_lit"), vec!["api.example.com"], "the ordinary argument-0 verb still captures");
    // (b) a bind address never enters the destination surface, and does not suppress a real one.
    assert_eq!(hosts("bind_then_dest"), vec!["api.example.com:443"],
               "a LOCAL bind address must not appear in `hosts`, and must not cost the function the \
                certification of its actual, visible destination");
    assert!(hosts("bind_only").is_empty(), "a bind alone names no destination");
}

/// ⟨0.29⟩ THE PEEK IS A NESTED SCAN, AND IT MUST NOT COUNT TOWARD THE VERDICT IT IS FORBIDDEN TO CHANGE.
///
/// The peek re-enters `scan_one` over the EXCLUDED files to answer `outOfScope`. `record_gate_analyzed`
/// accumulates (`+= count`) into a process-global, so the peek's units were landing in the --gate-json
/// verdict — while the peek writes no report, so `gate --report` could never reach the same number.
///
/// MEASURED on `crates/candor-query`: the scan route wrote `analyzed.count 276`, the report it had just
/// produced said 129, and `ci/gate-equivalence.sh` failed 20 of its 54 §3.1 byte-equality rows. The scan
/// route was the wrong one twice: `analyzed.count` is IN the verdict, so inflating it IS the verdict
/// change the peek promises not to make; and the count is the ⟨0.21⟩ completeness manifest, so it told a
/// consumer 276 units were judged when 129 were — the OVER-CLAIM direction.
///
/// This row is in-tree because the CI equivalence script needs BOTH binaries and takes minutes; the
/// property itself needs only this one. It asserts the two numbers the two routes read AGREE, at their
/// source, so the defect cannot come back through some other consumer of the accumulator.
#[test]
fn the_peek_does_not_inflate_the_gate_verdicts_analyzed_count() {
    let d = make_crate("exclcount", "pub fn go() { std::fs::read(\"/etc/hosts\").unwrap(); }");
    // build.rs is EXCLUDED (`build-script`), which is what arms the peek: it runs only when the policy
    // denies something AND the run excluded files. Its function is the bait — before the fix its unit
    // was counted into the verdict, and it must not be counted after it either.
    std::fs::write(d.join("build.rs"), "fn main() { std::fs::read(\"/etc/passwd\").unwrap(); }").unwrap();
    let pol = d.join("candor.policy");
    std::fs::write(&pol, "deny Net\n").unwrap();          // denies SOMETHING (arms the peek), matches nothing
    let gate = d.join("verdict.json");
    let out = Command::new(bin())
        .args([d.to_string_lossy().as_ref(),
               "--out", d.join("rep").to_string_lossy().as_ref(),
               "--policy", pol.to_string_lossy().as_ref(),
               "--gate-json", gate.to_string_lossy().as_ref()])
        .output()
        .expect("run candor-scan");

    let rep_path = std::fs::read_dir(&d).unwrap().filter_map(Result::ok).map(|e| e.path())
        .find(|p| p.to_string_lossy().contains("rep.") && p.to_string_lossy().ends_with(".scan.json")
                  && !p.to_string_lossy().contains("callgraph") && !p.to_string_lossy().contains("peek"))
        .expect("the scan wrote a report");
    let rep: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(rep_path).unwrap()).unwrap();
    let verdict: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&gate).unwrap()).unwrap();

    // THE PEEK MUST HAVE RUN, or this row asserts nothing: without an exclusion to walk there is no
    // nested scan, and the counts would agree for the boring reason.
    assert!(rep.get("excluded").and_then(|e| e.as_array()).map(|a| !a.is_empty()).unwrap_or(false),
            "the fixture excluded nothing, so the peek never ran and this row is vacuous: {rep}");

    let rep_count = rep["analyzed"]["count"].as_u64().expect("the report carries analyzed.count");
    let verdict_count = verdict["analyzed"]["count"].as_u64().expect("the verdict carries analyzed.count");
    assert_eq!(
        verdict_count, rep_count,
        "the --gate-json verdict counts {verdict_count} analyzed units while the report this same run \
         wrote counts {rep_count} — the peek's nested scan is accumulating into the verdict. §3.1 makes \
         `gate --report` reproduce this document byte-for-byte, and it can only ever see the report's \
         number, so the two routes are now two gates. stderr: {}",
        String::from_utf8_lossy(&out.stderr));

    // THE CONTROL. `analyzed.count` must still be a real count — a fix that zeroed it, or that skipped
    // `record_gate_analyzed` entirely, passes the equality above and deletes the manifest.
    assert!(verdict_count > 0,
            "analyzed.count is {verdict_count} — the counts agree because nothing is counted, which is \
             the ⟨0.21⟩ manifest deleted rather than corrected");
}


/// A WORKSPACE ROOT THAT IS ALSO A MEMBER MUST BE SCANNED ONCE.
///
/// `members = ["sub", "."]` is legal and real — bollard v0.16.1 ships it. `workspace_members` dedupes
/// STRINGS, so `.` survives as `<root>/.`: a different string, the same directory as the root pushed
/// beside it. `scan_one` then ran twice over one package, and the two symptoms were:
///   · `record_gate_analyzed` fired twice, so the --gate-json verdict OVER-CLAIMED. On bollard it said
///     `analyzed.count 856` where its own three reports summed to 592, which also breaks SPEC §3.1:
///     `gate --report` can only ever see the reports, so the two routes stopped agreeing.
///   · `--json` emitted the same package TWICE in its array.
/// The report FILES were unharmed — the second write is identical — which is why nothing else noticed.
///
/// Found by the corpus round's §3.1 oracle over THIRD-PARTY trees; the in-repo gate-equivalence
/// fixtures cannot reach it, because candor's own workspace does not list its root as a member.
#[test]
fn a_workspace_root_that_is_also_a_member_is_scanned_once() {
    let d = std::env::temp_dir().join(format!("candor-wsdup-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(d.join("src")).unwrap();
    std::fs::create_dir_all(d.join("sub/src")).unwrap();
    std::fs::write(d.join("Cargo.toml"),
        "[package]\nname = \"rootpkg\"\n\n[workspace]\nmembers = [\"sub\", \".\"]\n").unwrap();
    std::fs::write(d.join("src/lib.rs"),
        "pub fn go() { let _ = std::fs::read(\"/etc/passwd\"); }\n").unwrap();
    std::fs::write(d.join("sub/Cargo.toml"), "[package]\nname = \"subpkg\"\n").unwrap();
    std::fs::write(d.join("sub/src/lib.rs"),
        "pub fn sub_go() { let _ = std::fs::read(\"/etc/hosts\"); }\n").unwrap();

    // (1) --json must carry each package ONCE.
    let out = Command::new(bin()).args([d.to_string_lossy().as_ref(), "--json"]).output().expect("scan");
    let docs: serde_json::Value =
        serde_json::from_slice(&out.stdout).expect("--json emits a JSON array over a workspace");
    let names: Vec<String> = docs.as_array().expect("an array").iter()
        .map(|x| x["package"].as_str().unwrap_or("").to_string()).collect();
    let mut uniq = names.clone(); uniq.sort(); uniq.dedup();
    assert_eq!(names.len(), uniq.len(),
               "`--json` emitted a package twice over a workspace whose root is also a member: {names:?}");

    // (2) …and the VERDICT must count what the REPORTS contain, or §3.1 byte-equality is gone.
    let pol = d.join("candor.policy");
    std::fs::write(&pol, "deny Fs\n").unwrap();
    let gate = d.join("verdict.json");
    Command::new(bin())
        .args([d.to_string_lossy().as_ref(),
               "--out", d.join("rep").to_string_lossy().as_ref(),
               "--policy", pol.to_string_lossy().as_ref(),
               "--gate-json", gate.to_string_lossy().as_ref()])
        .output().expect("scan");
    let mut sum = 0u64;
    for e in std::fs::read_dir(&d).unwrap().filter_map(Result::ok) {
        let p = e.path(); let n = p.to_string_lossy().to_string();
        if n.contains("rep.") && n.ends_with(".scan.json") && !n.contains("callgraph") {
            let v: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(&p).unwrap()).unwrap();
            sum += v["analyzed"]["count"].as_u64().unwrap_or(0);
        }
    }
    let v: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(&gate).unwrap()).unwrap();
    let verdict = v["analyzed"]["count"].as_u64().unwrap();
    assert_eq!(verdict, sum,
               "the verdict counts {verdict} analyzed units while its own reports hold {sum} — the root \
                was scanned twice, so `gate --report` (which sees only the reports) can never agree");
    assert!(sum > 0, "the fixture analyzed nothing — this row would pass for the wrong reason");
    let _ = std::fs::remove_dir_all(&d);
}

/// ⟨0.32⟩ THE VERDICT DOCUMENT AND THE EXIT CODE MUST BE DECIDED BY ONE PREDICATE.
///
/// The unread-class recorder keyed on `out_of_scope.is_some()` while the exit arm keyed on
/// `peek_attempted`, and the two are NOT the same question. `outOfScope` comes back `Some(vec![])`
/// when the policy carries no DENY rule — the peek short-circuits, returning "asked and clear" — so an
/// `allow`-only or `forbid`-only policy over a tree with a build script recorded every exclusion class
/// as unread INTO THE DOCUMENT. MEASURED 2026-08-24: exit 0 beside `"ok": false, "incomplete": true`.
/// The exit was right and the document was the over-charge, visible only to a reader of the JSON —
/// which is the CI consumer, i.e. the only reader that matters here.
///
/// TWO ROWS: the over-charge (a policy with no deny rule asks nothing of the excluded code, so it must
/// cost nothing) and the CONTROL that the disclosure still fires when a deny rule really is unanswered.
/// Without the second, deleting the recorder passes the first.
#[test]
fn a_policy_with_no_deny_rule_does_not_record_unread_classes_into_the_verdict() {
    let d = make_crate("nodeny", "pub fn go() { let _ = 1; }");
    // build.rs is EXCLUDED (`build-script`) — without an exclusion there is nothing to mis-record and
    // both rows below are vacuous. It is readable and effectful; the point is that NOBODY ASKS.
    std::fs::write(d.join("build.rs"),
        "fn main() { let _ = std::process::Command::new(\"rustc\").status(); }").unwrap();

    let run = |pol: &str, tag: &str| -> (Option<i32>, serde_json::Value) {
        let pp = d.join(format!("{tag}.policy"));
        std::fs::write(&pp, pol).unwrap();
        let gate = d.join(format!("{tag}.verdict.json"));
        let _ = std::fs::remove_file(&gate);
        let out = Command::new(bin())
            .args([d.to_string_lossy().as_ref(),
                   "--out", d.join(format!("rep-{tag}")).to_string_lossy().as_ref(),
                   "--policy", pp.to_string_lossy().as_ref(),
                   "--gate-json", gate.to_string_lossy().as_ref()])
            .output().expect("run candor-scan");
        let v: serde_json::Value = serde_json::from_str(
            &std::fs::read_to_string(&gate).unwrap_or_else(|e| panic!(
                "no --gate-json document at {}: {e}; stderr: {}",
                gate.display(), String::from_utf8_lossy(&out.stderr))))
            .expect("the verdict is JSON");
        (out.status.code(), v)
    };

    // THE OVER-CHARGE. `allow` asks nothing of code outside the scan's scope, and neither does
    // `forbid` — both are answered from the scanned surface — so an unpeeked class must not move
    // either half of the verdict.
    for (pol, tag) in [("allow Net api.example.com\n", "allowonly"), ("forbid app -> infra\n", "forbidonly")] {
        let (code, v) = run(pol, tag);
        assert_eq!(code, Some(0), "a policy with no deny rule passes on this tree: {v}");
        assert_eq!(v["ok"], serde_json::json!(true),
            "the DOCUMENT said not-ok at exit 0 — the recorder and the exit arm were keyed on two \
             different predicates, and only a reader of the JSON could see the disagreement: {v}");
        assert!(v.get("incomplete").is_none(),
            "`incomplete` claims the scan could not see enough; nothing here went unread that this \
             policy needed read: {v}");
    }

    // THE CONTROL. Add a DENY rule and the same tree is genuinely unanswered — the peek reads build.rs,
    // finds the denied effect, and the verdict is INCOMPLETE at exit 2. A fix that simply stopped
    // recording would pass the rows above and delete the rung.
    let (code, v) = run("deny Exec\n", "deny");
    assert_eq!(code, Some(2), "the deny rule's answer DOES depend on the excluded build script: {v}");
    assert_eq!(v["incomplete"], serde_json::json!(true), "{v}");
    assert_eq!(v["ok"], serde_json::json!(false), "{v}");
    let _ = std::fs::remove_dir_all(&d);
}

/// ⟨0.32⟩ **TWO UNITS, TWO ROWS A READER CAN TELL APART** — SPEC §2: *"a verdict row MUST carry enough
/// identity for a consumer to tell two units apart… and the sort key MUST include that identity."*
///
/// MEASURED at `ab505c0` on exactly this fixture — a two-member workspace whose members both define
/// `go()` and both spawn `curl` — under `deny Exec`:
///
/// ```text
///   "violations": [
///     { "rule": "AS-EFF-006", "fn": "go", "effects": ["Exec"], "detail": "`go` performs { Exec } …" },
///     { "rule": "AS-EFF-006", "fn": "go", "effects": ["Exec"], "detail": "`go` performs { Exec } …" }
///   ]
/// ```
///
/// Byte-identical. A reader cannot tell two broken members from one listed twice, and this is the
/// SCAN route — where the two rows are produced by two separate `policy_violations` calls and
/// concatenated, so their order was the member walk's and the `(rule, detail)` sort could not break
/// the tie. §3.3.1 makes the document's order part of the byte-equality with `gate --report`.
#[test]
fn a_workspace_verdict_tells_two_same_named_units_apart() {
    let d = std::env::temp_dir().join(format!("candor-scan-cli-wsidentity-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    for m in ["a", "b"] {
        std::fs::create_dir_all(d.join(m).join("src")).unwrap();
        std::fs::write(d.join(m).join("Cargo.toml"), format!("[package]\nname = \"{m}\"\n")).unwrap();
        std::fs::write(
            d.join(m).join("src/lib.rs"),
            "pub fn go() { let _ = std::process::Command::new(\"curl\").status(); }\n",
        ).unwrap();
    }
    std::fs::write(d.join("Cargo.toml"), "[workspace]\nmembers = [\"a\", \"b\"]\n").unwrap();
    let pp = d.join("candor.policy");
    std::fs::write(&pp, "deny Exec\n").unwrap();
    let gp = d.join("gate.json");

    let out = Command::new(bin())
        .arg(d.to_string_lossy().as_ref())
        .arg("--out").arg(d.join("rep").to_string_lossy().as_ref())
        .arg("--policy").arg(pp.to_string_lossy().as_ref())
        .arg("--gate-json").arg(gp.to_string_lossy().as_ref())
        .output()
        .expect("run candor-scan");
    assert_eq!(out.status.code(), Some(1), "both members violate `deny Exec`");
    let doc = std::fs::read_to_string(&gp).expect("gate.json written");
    let v: serde_json::Value = serde_json::from_str(&doc).expect("valid JSON");
    // READ BEFORE DELETING. This block used to sit after the `remove_dir_all` below and read a path
    // that no longer existed, behind an `if !rep.is_empty()` — an assertion that could only ever be
    // skipped. A join key the report does not carry is not a join key, so the row must be REACHED.
    let rep = std::fs::read_to_string(d.join("rep.a.scan.json")).expect("the member's report");
    let _ = std::fs::remove_dir_all(&d);

    let rows = v["violations"].as_array().unwrap();
    assert_eq!(rows.len(), 2, "instrument: this row means nothing without TWO violations: {doc}");
    assert_ne!(rows[0], rows[1],
        "two units, two rows, and NOTHING in either says which member it is about: {doc}");
    // The §2.2 JOIN KEY, and in SORTED order — the sort key must include the identity or the tie is
    // broken by the member walk, which the report route does not share.
    let hashes: Vec<&str> = rows.iter().map(|r| r["hash"].as_str().unwrap_or("")).collect();
    assert_eq!(hashes, vec!["a#go", "b#go"], "each row carries its unit's `hash`, sorted: {doc}");
    // …and it is the SAME string the report entry carries, which is what makes the join work at all.
    assert!(rep.contains("\"hash\": \"a#go\""),
        "the verdict row's identity must be findable in the report it is about: {rep}");
    // The NAME is untouched: it is what a policy scope matches and what a human reads.
    let names: Vec<&str> = rows.iter().map(|r| r["fn"].as_str().unwrap()).collect();
    assert_eq!(names, vec!["go", "go"], "`fn` stays the bare name: {doc}");
}

/// ⟨dd90fae, 2026-08-29⟩ NESTED WORKSPACE: a resolved member that is ITSELF a `[workspace]` root.
///
/// Pre-fix, `scan_target` computed `workspace_members` once and handed each resolved member straight
/// to `scan_one` as a plain crate. `scan_one`'s OWN nested-package filter ("a subdir carrying its own
/// Cargo.toml is a different package") then pruned every one of THAT member's inner members out of the
/// walk — with no `excluded` entry, no `outOfScope` entry, and no stderr. `analyzed.count` for the
/// member came back 0 and `deny Net` printed "policy ✓" at exit 0 over a real, unread Net call one
/// level further down.
///
/// This layout is real, reachable input, not an invented edge case: `cargo metadata` refuses it
/// ("multiple workspace roots found"), but that is an argument about `cargo build`, not about whether a
/// static scanner reading source WITHOUT building it should see the tree.
///
/// PROVEN to discriminate the fix (see the task's reproduction step): reverting only
/// `expand_nested_workspace_member` / its call site in `scan_target` (scan.rs) turns this test red
/// while the full existing suite (254 unit + 74 CLI) stays green — which is exactly the gap that let
/// `dd90fae` ship without a dedicated test in the first place.
#[test]
fn nested_workspace_member_is_fanned_out_and_deny_net_catches_the_inner_call() {
    let d = std::env::temp_dir().join(format!("candor-scan-cli-nestedws-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(d.join("mid/inner/src")).unwrap();
    // outer root: a workspace whose one member ("mid") is not a package, only a nested workspace root.
    std::fs::write(d.join("Cargo.toml"), "[workspace]\nmembers = [\"mid\"]\n").unwrap();
    // "mid": itself a `[workspace]` root, no `[package]` of its own — a real, if unusual, layout.
    std::fs::write(d.join("mid/Cargo.toml"), "[workspace]\nmembers = [\"inner\"]\n").unwrap();
    std::fs::write(d.join("mid/inner/Cargo.toml"), "[package]\nname = \"inner\"\n").unwrap();
    std::fs::write(d.join("mid/inner/src/lib.rs"),
        "pub fn go() { let _ = std::net::TcpStream::connect(\"evil.example.com:80\"); }\n").unwrap();
    let pp = d.join("candor.policy");
    std::fs::write(&pp, "deny Net\n").unwrap();

    let out = Command::new(bin())
        .arg(d.to_string_lossy().as_ref())
        .arg("--policy").arg(pp.to_string_lossy().as_ref())
        .arg("--json")
        .output()
        .expect("run candor-scan");
    let stdout = String::from_utf8(out.stdout).expect("utf8 stdout");
    let _ = std::fs::remove_dir_all(&d);

    assert_eq!(out.status.code(), Some(1),
        "a nested-workspace member's inner member must be scanned, not pruned into an empty report: \
         `deny Net` should catch its real Net call, got exit {:?}\nstdout:\n{stdout}", out.status.code());
    let docs: serde_json::Value = serde_json::from_str(stdout.trim()).expect("--json emits valid JSON");
    let names: Vec<&str> = docs.as_array().expect("an array over the fanned-out members").iter()
        .map(|x| x["package"].as_str().unwrap_or("")).collect();
    assert!(names.contains(&"inner"),
        "the nested workspace's inner member must appear in the fan-out, not vanish: {names:?}");
}

/// ⟨dd90fae, 2026-08-29⟩ MULTI-LEVEL GLOB: `members = ["crates/*/*"]` over an ordinary two-level
/// layout `cargo metadata` resolves to real members.
///
/// Pre-fix, `workspace_members`'s hand-rolled matcher special-cased a bare `*` and a single trailing
/// `/*`; a two-level glob fell through `.strip_suffix("/*")`, which looked for a literal directory
/// named `crates/*`, found none, and returned ZERO members with no glob-specific disclosure.
/// `scan_target` then fell back to a single-crate scan of the root and `deny Net` printed "policy ✓" at
/// exit 0 over three real, unscanned crates three directories down.
///
/// `cargo metadata` is the ground truth this fixture is checked against: this is not an exotic or
/// invalid layout, it is a shape cargo itself expands the same way `glob` does.
///
/// PROVEN to discriminate the fix: reverting only `expand_member_glob` / its call site in
/// `workspace_members` (deps.rs) turns this test red while the full existing suite stays green.
#[test]
fn multi_level_glob_workspace_members_resolve_and_deny_net_catches_the_violation() {
    let d = std::env::temp_dir().join(format!("candor-scan-cli-globws-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    for p in ["crates/a/x", "crates/b/y", "crates/c/z"] {
        std::fs::create_dir_all(d.join(p).join("src")).unwrap();
    }
    std::fs::write(d.join("Cargo.toml"), "[workspace]\nmembers = [\"crates/*/*\"]\n").unwrap();
    std::fs::write(d.join("crates/a/x/Cargo.toml"), "[package]\nname = \"a_x\"\n").unwrap();
    std::fs::write(d.join("crates/a/x/src/lib.rs"), "pub fn noop() -> u32 { 1 }\n").unwrap();
    std::fs::write(d.join("crates/b/y/Cargo.toml"), "[package]\nname = \"b_y\"\n").unwrap();
    std::fs::write(d.join("crates/b/y/src/lib.rs"),
        "pub fn go() { let _ = std::net::TcpStream::connect(\"evil.example.com:80\"); }\n").unwrap();
    std::fs::write(d.join("crates/c/z/Cargo.toml"), "[package]\nname = \"c_z\"\n").unwrap();
    std::fs::write(d.join("crates/c/z/src/lib.rs"), "pub fn noop2() -> u32 { 2 }\n").unwrap();

    // GROUND TRUTH: `cargo metadata` resolves this ordinary layout to exactly the 3 leaf crates.
    let meta = Command::new("cargo")
        .args(["metadata", "--no-deps", "--format-version=1"])
        .current_dir(&d)
        .output()
        .expect("run cargo metadata");
    let meta_json: serde_json::Value =
        serde_json::from_slice(&meta.stdout).expect("cargo metadata emits JSON");
    let mut meta_names: Vec<String> = meta_json["packages"].as_array().expect("packages array")
        .iter().map(|p| p["name"].as_str().unwrap_or("").to_string()).collect();
    meta_names.sort();
    assert_eq!(meta_names, vec!["a_x", "b_y", "c_z"],
        "ground truth: cargo metadata must resolve `crates/*/*` to the 3 real members, or this fixture \
         is not testing an ordinary layout: {meta_names:?}");

    let pp = d.join("candor.policy");
    std::fs::write(&pp, "deny Net\n").unwrap();
    let out = Command::new(bin())
        .arg(d.to_string_lossy().as_ref())
        .arg("--policy").arg(pp.to_string_lossy().as_ref())
        .arg("--json")
        .output()
        .expect("run candor-scan");
    let stdout = String::from_utf8(out.stdout).expect("utf8 stdout");
    let _ = std::fs::remove_dir_all(&d);

    assert_eq!(out.status.code(), Some(1),
        "a multi-level glob `crates/*/*` must resolve to its real members, not fall back to a \
         single-crate scan of the root: `deny Net` should catch b_y's real Net call, got exit {:?}\n\
         stdout:\n{stdout}", out.status.code());
    let docs: serde_json::Value = serde_json::from_str(stdout.trim()).expect("--json emits valid JSON");
    let mut names: Vec<String> = docs.as_array().expect("an array over the resolved members").iter()
        .map(|x| x["package"].as_str().unwrap_or("").to_string()).collect();
    names.sort();
    assert_eq!(names, vec!["a_x", "b_y", "c_z"],
        "all three multi-level-glob members must be fanned out, matching cargo metadata's ground \
         truth, not collapsed into a single root-level scan: {names:?}");
}

/// ⟨R416, 2026-09-12⟩ A PATH WRAPPER IS NOT A TRANSFORMATION: `let p = Path::new("/tmp/benign")`
/// must leave the locator DETERMINED, exactly as the inline literal, a `&str` local and a `const` do.
///
/// Pre-fix this shape published `paths: null` and `incomplete: ["Fs"]`, so AS-EFF-008 REFUSED
/// `allow Fs /tmp/benign` over a write whose destination is a compile-time literal one hop away —
/// "performs Fs with no visible literal", of a call site that has nothing but a visible literal.
/// The three sibling spellings all passed, which is what made it a drift rather than a policy.
///
/// Direction matters: this moves a gate from FAIL to PASS, so it is the FABRICATION direction's
/// mirror and can only ever credit a literal it can prove. `Path::new`/`PathBuf::from` are
/// documented identity over the string; `join`, `with_extension` and `canonicalize` are not and
/// must keep returning None — `path_join_is_not_credited_as_a_determined_locator` below is that
/// half, and the two are one test in two functions.
///
/// PROVEN to discriminate: with only the `Expr::Call` arm of `resolve_str_expr` reverted AND THE
/// BINARY REBUILT (a stale binary faked this pass once already — see the 2026-09-12 note in
/// SOUNDNESS-LOG), this asserts exit 1 with "no visible literal" and goes red.
#[test]
fn a_path_wrapper_keeps_a_determined_locator_determined() {
    let d = std::env::temp_dir().join(format!("candor-scan-cli-r416-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(d.join("src")).unwrap();
    std::fs::write(d.join("Cargo.toml"), "[package]\nname = \"r416\"\n").unwrap();
    std::fs::write(d.join("src/lib.rs"), concat!(
        "pub fn via_path_new() {\n",
        "    let p = std::path::Path::new(\"/tmp/benign\");\n",
        "    let _ = std::fs::write(p, b\"x\");\n",
        "}\n",
        "pub fn via_pathbuf_from() {\n",
        "    let p = std::path::PathBuf::from(\"/tmp/benign\");\n",
        "    let _ = std::fs::write(p, b\"x\");\n",
        "}\n",
    )).unwrap();
    let pp = d.join("candor.policy");
    std::fs::write(&pp, "allow Fs /tmp/benign\n").unwrap();

    let out = Command::new(bin())
        .arg(d.to_string_lossy().as_ref())
        .arg("--policy").arg(pp.to_string_lossy().as_ref())
        .output()
        .expect("run candor-scan");
    let so = String::from_utf8_lossy(&out.stdout).to_string();
    let se = String::from_utf8_lossy(&out.stderr).to_string();
    let _ = std::fs::remove_dir_all(&d);

    assert_eq!(out.status.code(), Some(0),
        "`allow Fs /tmp/benign` must PASS over a write whose path is a literal behind Path::new / \
         PathBuf::from — both spellings name the same determined destination as the inline form.\n\
         stdout:\n{so}\nstderr:\n{se}");
    assert!(!format!("{so}{se}").contains("no visible literal"),
        "the surface must not be reported incomplete when the literal is one documented-identity \
         constructor away.\nstdout:\n{so}\nstderr:\n{se}");
}

/// ⟨R416, 2026-09-12⟩ THE OVER-CHARGE HALF, and the reason the fix above is a TWO-NAME MATCH rather
/// than "anything taking a string returns that string".
///
/// `Path::new(base).join(tail)` does not evaluate to `base`. Crediting it would let
/// `allow Fs /tmp/benign` certify a write to `/tmp/benign/<anything a caller chose>` — the
/// gate-bypass shape of AS-EFF-008, arrived at through a fix aimed at the opposite direction.
/// SOUNDNESS [[feedback-fabrication-fixes-cause-misses]]: write the second fixture FIRST.
#[test]
fn path_join_is_not_credited_as_a_determined_locator() {
    let d = std::env::temp_dir().join(format!("candor-scan-cli-r416j-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(d.join("src")).unwrap();
    std::fs::write(d.join("Cargo.toml"), "[package]\nname = \"r416j\"\n").unwrap();
    std::fs::write(d.join("src/lib.rs"), concat!(
        "pub fn via_join(tail: &str) {\n",
        "    let p = std::path::Path::new(\"/tmp/benign\").join(tail);\n",
        "    let _ = std::fs::write(p, b\"x\");\n",
        "}\n",
    )).unwrap();
    let pp = d.join("candor.policy");
    std::fs::write(&pp, "allow Fs /tmp/benign\n").unwrap();

    let out = Command::new(bin())
        .arg(d.to_string_lossy().as_ref())
        .arg("--policy").arg(pp.to_string_lossy().as_ref())
        .output()
        .expect("run candor-scan");
    let so = String::from_utf8_lossy(&out.stdout).to_string();
    let se = String::from_utf8_lossy(&out.stderr).to_string();
    let _ = std::fs::remove_dir_all(&d);

    assert_ne!(out.status.code(), Some(0),
        "a caller-chosen `.join(tail)` is NOT the literal it was built from: `allow Fs /tmp/benign` \
         must not certify it.\nstdout:\n{so}\nstderr:\n{se}");
}

/// ⟨R417, 2026-09-12⟩ AS-EFF-008 GATE BYPASS via the CAPABILITY-`Dir` API: `d.write(caller, …)` on a
/// `cap_std::fs::Dir` performs `Fs` with an invisible, caller-chosen path and did NOT mark the surface
/// incomplete, so a benign sibling literal in the same function CERTIFIED it and `allow Fs /tmp/benign`
/// exited 0. Found by probing the masking guard's method half rather than its free-fn half.
///
/// Three arms, because two of them are what make the third mean anything:
///   `control`        — the free-fn spelling of the identical hazard. Must stay red. If this ever goes
///                      green the fixture has stopped testing the guard at all.
///   `mixed`          — the defect. Must be red.
///   `noop_exception` — `d.try_clone()`, a `Dir` method that provably takes NO path. Must stay GREEN:
///                      the fix masks by default on a `Dir` receiver, so this is the over-mask control
///                      that prices that decision. Without it the fix could be "mask everything".
///
/// PROVEN to discriminate: with `is_fs_path_arg_method`'s `Dir` arm reverted to the two-name form AND
/// THE CLI REBUILT, `mixed` goes green (exit 0) while the other two are unchanged.
#[test]
fn a_capability_dir_method_path_cannot_be_certified_by_a_sibling_literal() {
    let d = std::env::temp_dir().join(format!("candor-scan-cli-r417-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(d.join("src")).unwrap();
    std::fs::write(d.join("Cargo.toml"), "[package]\nname = \"r417\"\n").unwrap();
    std::fs::write(d.join("src/lib.rs"), concat!(
        "use cap_std::fs::Dir;\n",
        "pub fn mixed(d: &Dir, caller: &str) {\n",
        "    let _ = std::fs::write(\"/tmp/benign\", b\"x\");\n",
        "    let _ = d.write(caller, b\"pwned\");\n",
        "}\n",
        "pub fn control(caller: &str) {\n",
        "    let _ = std::fs::write(\"/tmp/benign\", b\"x\");\n",
        "    let _ = std::fs::write(caller, b\"pwned\");\n",
        "}\n",
        "pub fn noop_exception(d: &Dir) {\n",
        "    let _ = std::fs::write(\"/tmp/benign\", b\"x\");\n",
        "    let _ = d.try_clone();\n",
        "}\n",
    )).unwrap();
    let pp = d.join("candor.policy");
    std::fs::write(&pp, "allow Fs /tmp/benign\n").unwrap();

    let out = Command::new(bin())
        .arg(d.to_string_lossy().as_ref())
        .arg("--policy").arg(pp.to_string_lossy().as_ref())
        .output()
        .expect("run candor-scan");
    let all = format!("{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr));
    let _ = std::fs::remove_dir_all(&d);

    assert!(all.contains("`mixed`"),
        "`cap_std::fs::Dir::write(caller, …)` names its path as an ARGUMENT: with the path invisible \
         the Fs surface must be incomplete, so the sibling `/tmp/benign` literal cannot certify it.\n{all}");
    assert!(all.contains("`control`"),
        "CALIBRATION: the free-fn spelling of the same hazard must still be caught — if it is not, this \
         fixture is no longer exercising the masking guard.\n{all}");
    assert!(!all.contains("`noop_exception`"),
        "OVER-MASK CONTROL: `Dir::try_clone()` takes no path, so masking by default on a `Dir` receiver \
         must not swallow it.\n{all}");
    assert_eq!(out.status.code(), Some(1), "two of the three arms violate, so the gate must exit 1\n{all}");
}

/// ⟨R414 / SPEC ⟨0.37⟩, 2026-09-12⟩ A RECEIVER-FORM PATH STAT NAMES ITS DESTINATION. `p.exists()` reaches
/// the filesystem against the path it is invoked ON, so an indeterminate receiver leaves the `Fs` surface
/// incomplete exactly as `fs::metadata(p)` does — and a DETERMINED receiver is captured and published
/// instead, not masked.
///
/// Pre-fix, `fs::write("/tmp/benign", …); p.exists()` reported `paths:['/tmp/benign'] incomplete:NONE`, so
/// `allow Fs /tmp/benign` exited 0 over a caller-chosen path while the run printed "nothing hidden". The
/// ARGUMENT spelling of the identical reach was already marked, which is what makes it drift rather than
/// policy: the method form was excluded on the assumption that a receiver is an OPEN HANDLE. True of
/// `File`, false of `Path` — a `Path` is a path VALUE.
///
/// ONE TREE PER ARM, deliberately. The gate's exit code answers for the WHOLE scan, so putting these
/// functions in one package lets an already-correct sibling (`arg_masked`) make the gate red with the fix
/// REVERTED — a test that passes with and without the change. candor-swift's agent hit exactly that on
/// this rung the same day and restructured for the same reason.
///
/// Four arms: the defect, the argument-form CALIBRATION control (if it ever goes green this fixture has
/// stopped testing the guard), the handle OVER-MASK control (`File::write_all` must still certify — marking
/// it would fail every program that opens a file by a literal name), and the determined-receiver over-mask
/// control in BOTH spellings, inline and `let`-bound.
///
/// PROVEN to discriminate: with `is_fs_receiver_locator`'s disjunct removed AND THE CLI REBUILT, the
/// masked arm goes green while the three controls are unchanged.
#[test]
fn a_receiver_form_path_stat_names_its_own_destination() {
    let base = std::env::temp_dir().join(format!("candor-scan-cli-r414-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&base);

    // (arm, body, must the gate refuse?)
    let arms: &[(&str, &str, bool)] = &[
        ("recv_masked",
         "use std::path::Path;\npub fn f(p: &Path) {\n  let _ = std::fs::write(\"/tmp/benign\", b\"x\");\n  let _ = p.exists();\n}\n",
         true),
        ("arg_masked",
         "use std::path::Path;\npub fn f(p: &Path) {\n  let _ = std::fs::write(\"/tmp/benign\", b\"x\");\n  let _ = std::fs::metadata(p);\n}\n",
         true),
        ("handle_use",
         "pub fn f(h: &mut std::fs::File) {\n  use std::io::Write;\n  let _ = std::fs::write(\"/tmp/benign\", b\"x\");\n  let _ = h.write_all(b\"y\");\n}\n",
         false),
        ("recv_determined_inline",
         "use std::path::Path;\npub fn f() {\n  let _ = std::fs::write(\"/tmp/benign\", b\"x\");\n  let _ = Path::new(\"/tmp/benign\").exists();\n}\n",
         false),
        ("recv_determined_local",
         "use std::path::Path;\npub fn f() {\n  let _ = std::fs::write(\"/tmp/benign\", b\"x\");\n  let p = Path::new(\"/tmp/benign\");\n  let _ = p.metadata();\n}\n",
         false),
        // The two arms above prove the determined receiver is NOT OVER-MASKED, and nothing more: both
        // exit 0 with the fix reverted too, because an unmarked-and-uncaptured receiver also certifies.
        // This arm is the one that proves it is actually CAPTURED — the stat names a directory the policy
        // does NOT allow, so publishing the locator is what makes the gate refuse. Reverted, it exits 0
        // (nothing published, nothing marked, silently certified against a policy naming somewhere else),
        // which is the SECOND, quieter half of R414 and would otherwise ship untested.
        ("recv_determined_elsewhere",
         "use std::path::Path;\npub fn f() {\n  let _ = std::fs::write(\"/tmp/benign\", b\"x\");\n  let _ = Path::new(\"/tmp/elsewhere\").exists();\n}\n",
         true),
    ];

    for (name, body, must_refuse) in arms {
        let d = base.join(name);
        std::fs::create_dir_all(d.join("src")).unwrap();
        std::fs::write(d.join("Cargo.toml"), format!("[package]\nname = \"{name}\"\n")).unwrap();
        std::fs::write(d.join("src/lib.rs"), body).unwrap();
        let pp = d.join("candor.policy");
        std::fs::write(&pp, "allow Fs /tmp/benign\n").unwrap();

        let out = Command::new(bin())
            .arg(d.to_string_lossy().as_ref())
            .arg("--policy").arg(pp.to_string_lossy().as_ref())
            .output()
            .expect("run candor-scan");
        let all = format!("{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr));
        let refused = out.status.code() != Some(0);

        assert_eq!(refused, *must_refuse,
            "arm `{name}`: expected `allow Fs /tmp/benign` to {} this tree, got exit {:?}.\n\
             recv_masked and arg_masked are the SAME filesystem reach written two ways and must agree; \
             handle_use and the two determined arms are over-mask controls and must certify.\n{all}",
            if *must_refuse { "REFUSE" } else { "certify" }, out.status.code());
    }
    let _ = std::fs::remove_dir_all(&base);
}

// ── BACKLOG §6d S2 — the charge-at-construction PRICING flag ──────────────────────────────────────
//
// The flag is default-OFF and priced NOT-adopted (see `collector::charge_at_construction` for the
// 1,561-crate measurement). These two arms keep it honest in both directions: the escape model must
// still suppress when the flag is unset, and the flag must still charge when it is set — a pricing
// instrument that silently stops working is worse than none, because the next re-price reads its
// inertness as a result.

/// The two shapes, in one crate: a construction that ESCAPES by return (the over-charge control — 0
/// executed in-frame drops, measured) and one that dies in frame through a shadowed closure name
/// (R323 — 2 executed in-frame drops, measured).
const S2_SRC: &str = r#"
use std::fs;
pub struct H { pub p: String }
impl H { pub fn mk(p: &str) -> H { H { p: p.to_string() } } }
impl Drop for H { fn drop(&mut self) { let _ = fs::write(&self.p, b"x"); } }
pub fn oc_factory(p: &str) -> H { H::mk(p) }
pub fn r323_shadowed(p: &str) { let c = || H::mk(p); c(); let c = || H::mk("b"); c(); }
"#;

fn s2_inferred(charge: bool) -> std::collections::BTreeMap<String, Vec<String>> {
    let d = make_crate(if charge { "s2on" } else { "s2off" }, S2_SRC);
    let mut cmd = Command::new(bin());
    cmd.arg(d.to_string_lossy().as_ref()).arg("--json");
    if charge {
        cmd.env("CANDOR_CHARGE_AT_CTOR", "1");
    } else {
        cmd.env_remove("CANDOR_CHARGE_AT_CTOR");
    }
    let out = cmd.output().expect("run candor-scan");
    let _ = std::fs::remove_dir_all(&d);
    let v: serde_json::Value = serde_json::from_str(String::from_utf8(out.stdout).unwrap().trim())
        .expect("--json stdout must parse");
    v["functions"]
        .as_array()
        .unwrap()
        .iter()
        .map(|f| {
            (
                f["fn"].as_str().unwrap().to_string(),
                f["inferred"].as_array().map(|a| a.iter().map(|x| x.as_str().unwrap().to_string()).collect())
                    .unwrap_or_default(),
            )
        })
        .collect()
}

#[test]
fn charge_at_construction_is_off_by_default() {
    // THE INERTNESS ARM. Measured at corpus scale too — flag-off vs the pre-change binary over 300
    // crates, 17,419 rows per arm, ADDED 0 / REMOVED 0 / CHANGED 0 — but that measurement is not in
    // the repo and this is. `oc_factory` returning its construction must stay uncharged, and the
    // R323 shadowed-closure silence must stay silent: the flag changes NOTHING while unset, INCLUDING
    // the open rows, which is what makes it a switch rather than a half-landed fix.
    let off = s2_inferred(false);
    assert!(!off.contains_key("oc_factory"), "escape model ON: a returned construction is not charged, got {off:?}");
    assert!(!off.contains_key("r323_shadowed"), "escape model ON: R323 is still OPEN, got {off:?}");
    assert_eq!(off.get("H::drop").map(Vec::as_slice), Some(&["Fs".to_string()][..]), "the Drop impl itself is always charged");
}

#[test]
fn charge_at_construction_charges_both_the_row_and_the_fabrication() {
    // THE OVER-CHARGE CONTROL IS THE POINT OF THIS ARM, not an afterthought. Turning the flag on
    // closes R323 (`r323_shadowed` — 2 executed in-frame drops) AND fabricates on `oc_factory`
    // (0 executed in-frame drops). A future edit that made the flag close the row WITHOUT the
    // fabrication would be the change worth shipping, and it would fail this assertion loudly rather
    // than passing unnoticed — which is the only reason the fabrication is asserted rather than
    // merely described.
    let on = s2_inferred(true);
    assert_eq!(on.get("r323_shadowed").map(Vec::as_slice), Some(&["Fs".to_string()][..]),
               "the flag must still close R323, got {on:?}");
    assert_eq!(on.get("oc_factory").map(Vec::as_slice), Some(&["Fs".to_string()][..]),
               "…and must still FABRICATE on a returned construction — this is the priced cost, got {on:?}");
}

/// SPEC §4 ⟨0.39⟩ — THE CHAINED-DISPATCH UNION, all three obligations, on the shape SOUNDNESS R475
/// measured live on `ratatui`.
///
/// THE DEFECT IS A TOGGLE AND IT RAN THE WRONG WAY. A library whose public abstraction has ZERO local
/// implementors gave a chained consumer a disclosed `Unknown`; adding ONE PURE implementor to that library
/// SILENTLY CERTIFIED the consumer pure — so **adding a pure implementation to a library removed a
/// disclosure from every consumer of it**. `ratatui-core`'s `Terminal::size` dispatches `Backend::size`
/// over its sole local implementor `TestBackend` (pure), `ratatui-crossterm`'s `CrosstermBackend::size`
/// performs `Ipc`, and an app chained onto both reported that function ABSENT.
///
/// THREE PACKAGES, BECAUSE NO TWO-PACKAGE ARM CAN EXPRESS IT: the effectful implementor lives in a THIRD
/// package, neither the dispatching dependency nor the consumer, which is why §4 says no two of the three
/// obligations are separable and why each is asserted here on its own evidence rather than inferred from
/// the consumer's verdict.
///
/// THE CONSUMER'S SOURCE IS BYTE-IDENTICAL ACROSS THE ARMS — one `app` tree, scanned three ways — so the
/// only thing that differs between the effectful arm and the pure-only control is WHICH dependency reports
/// were chained. A fixture-induced cross is what produced all three corrections to R475's original filing.
#[test]
fn a_foreign_effectful_implementor_reaches_a_chained_consumer_and_a_pure_only_one_does_not() {
    let d = std::env::temp_dir().join(format!("candor-scan-cli-r475-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    let pkg = |name: &str, deps: &str, src: &str| {
        let p = d.join(name);
        std::fs::create_dir_all(p.join("src")).unwrap();
        std::fs::write(p.join("Cargo.toml"),
            format!("[package]\nname = \"{name}\"\n\n[dependencies]\n{deps}")).unwrap();
        std::fs::write(p.join("src/lib.rs"), src).unwrap();
        p
    };
    // The dispatching dependency: ONE PURE local implementor. Nothing here is wrong on its own.
    let iface = pkg("iface", "",
        "pub trait Backend { fn size(&self) -> usize; }\n\
         pub struct TestBackend;\n\
         impl Backend for TestBackend { fn size(&self) -> usize { 7 } }\n\
         pub fn term_size(b: &dyn Backend) -> usize { b.size() }\n");
    // The THIRD package: implements the dependency's FOREIGN abstraction, effectfully.
    let effimpl = pkg("effimpl", "iface = \"1\"\n",
        "pub struct Crossterm;\n\
         impl iface::Backend for Crossterm {\n\
             fn size(&self) -> usize { let _ = std::net::TcpStream::connect(\"h:1\"); 0 }\n\
         }\n");
    // The consumer. `app_size` never spells the dispatch — it calls the dependency's dispatching fn —
    // which is exactly why the producer has to name the member on the row.
    let app = pkg("app", "iface = \"1\"\neffimpl = \"1\"\n",
        "pub fn app_size(b: &dyn iface::Backend) -> usize { iface::term_size(b) }\n\
         pub fn app_run() -> usize { app_size(&effimpl::Crossterm) }\n");

    let scan = |dir: &std::path::Path, deps: &[&std::path::Path]| -> serde_json::Value {
        let mut c = Command::new(bin());
        c.arg(dir.to_string_lossy().as_ref()).arg("--json")
            .env_remove("CANDOR_POLICY").env_remove("CANDOR_CONFIG");
        if deps.is_empty() {
            c.env_remove("CANDOR_DEPS");
        } else {
            c.env("CANDOR_DEPS", deps.iter().map(|p| p.to_string_lossy().into_owned())
                .collect::<Vec<_>>().join(" "));
        }
        let out = c.output().expect("run candor-scan");
        serde_json::from_str(String::from_utf8(out.stdout).unwrap().trim()).expect("pure JSON report")
    };
    let row = |v: &serde_json::Value, name: &str| -> Option<serde_json::Value> {
        v["functions"].as_array().unwrap().iter().find(|e| e["fn"] == name).cloned()
    };
    let write = |dir: &std::path::Path, v: &serde_json::Value, file: &str| -> std::path::PathBuf {
        let p = d.join(file);
        let _ = dir;
        std::fs::write(&p, serde_json::to_string(v).unwrap()).unwrap();
        p
    };

    // ── OBLIGATION 1: the producer names the member ON A ROW THAT IS OTHERWISE PURE ────────────────
    let iface_rep = scan(&iface, &[]);
    let term = row(&iface_rep, "term_size").expect(
        "⟨0.39⟩ obligation 1: a PURE function that DISPATCHES must be EMITTED. §2 rule 3 omits pure \
         functions and §2 chaining rule 3 makes that absence a purity CLAIM — which is the defect: the \
         row vanished precisely because the one implementor the library could see was pure");
    assert_eq!(term["inferred"].as_array().map(Vec::len).unwrap_or(0), 0,
        "…and it is emitted while still PURE — the row is the disclosure, not a new effect: {term}");
    assert_eq!(term["dispatchesOn"], serde_json::json!(["iface#Backend::size"]),
        "…naming the dispatched member in the OWNING package's entry-key spelling (SOUNDNESS R503), so \
         the consumer joins the value AS GIVEN and prefixes nothing. `Backend` sits at this fixture's \
         crate root, so only the `iface#` prefix distinguishes the two spellings here; the module-path \
         fixture below is the one that discriminates the rest: {term}");

    // ── OBLIGATION 2: the FOREIGN implementor publishes under the OWNING package's key ─────────────
    let eff_rep = scan(&effimpl, &[]);
    let union = row(&eff_rep, "Backend::size").expect(
        "⟨0.39⟩ obligation 2: a package implementing a FOREIGN abstraction must publish an \
         `interfaceUnion` entry — without it the only real-world instance measured is missed entirely");
    assert_eq!(union["hash"], "iface#Backend::size",
        "…keyed under the OWNING package (the ⟨0.23⟩ typeSurface spelling), never under the \
         implementing one, or the consumer's ordinary chained lookup cannot reach it: {union}");
    assert_eq!(union["interfaceUnion"], serde_json::json!(true), "{union}");
    assert_eq!(union["inferred"], serde_json::json!(["Net"]), "{union}");

    let iface_p = write(&iface, &iface_rep, "iface.json");
    let eff_p = write(&effimpl, &eff_rep, "effimpl.json");

    // ── OBLIGATION 3: the consumer's join unions per key ───────────────────────────────────────────
    let chained = scan(&app, &[&iface_p, &eff_p]);
    let app_size = row(&chained, "app_size").expect(
        "⟨0.39⟩: the consumer's inherited signature must carry the effects of every implementor visible \
         to it — this row was ABSENT, and `deny Ipc`/`pure` over it BOTH exited 0 (R475)");
    assert!(app_size["inferred"].as_array().unwrap().iter().any(|e| e == "Net"),
        "the foreign implementor's effect must REACH the consumer: {app_size}");

    // ── CONTROL (the fabrication guard): chained onto the PURE-ONLY library, the SAME consumer source
    //    must stay pure. An engine that unions indiscriminately — charging every consumer of a
    //    dispatching library for effects nobody implements — reddens HERE and nowhere else. This is
    //    conformance PART 92's `c3_pure_only`, and it is the arm that makes the one above evidence.
    let pure_only = scan(&app, &[&iface_p]);
    assert!(row(&pure_only, "app_size").is_none(),
        "a consumer over a library whose only implementor ANYWHERE is pure is LEGITIMATELY pure and must \
         stay absent — a hedge on `a dispatch occurred` is the fabrication direction §4 forbids: {pure_only}");

    // ── CONTROL (the disclosure the fix must not delete): a foreign union entry is NOT coverage of the
    //    package it names. `effimpl`'s report carries `iface#Backend::size`; reading that as "iface was
    //    analyzed" would withdraw the κ ledger's `invisible` for every call into `iface` — R475's own
    //    shape, manufactured by its own fix.
    let eff_alone = scan(&app, &[&eff_p]);
    let run = row(&eff_alone, "app_run").expect("app_run reaches an unanalyzed crate, so it is disclosed");
    assert!(run["invisible"].as_array().unwrap().iter().any(|c| c == "iface"),
        "chaining only the FOREIGN implementor must leave `iface` disclosed as invisible — a synthetic \
         entry keyed under a package is not a claim to have analyzed it: {run}");
}

/// ⟨0.39⟩ obligation 3's FIRST half: "its own visible implementors". The test above covers the chained
/// contributor; this one covers the consumer that supplies the effectful implementor ITSELF — and it
/// puts the dependency's abstraction in a MODULE, which is what distinguishes the two key spellings.
///
/// AND IT IS THE R503 PIN. `dispatchesOn` and obligation 2's key are now ONE spelling — fully qualified
/// in the owning package's namespace, the ⟨0.23⟩ rule — because they were two: the value carried the
/// trait LEAF while the foreign union entry carried `iface#backend::Backend::size`, so one member had two
/// names in one report and a consumer could join only one of them without a per-engine rule. Written with
/// the abstraction at the dependency's crate ROOT this arm cannot tell those apart, which is precisely
/// the shape a fixture is flattered by: `ratatui_core::backend::Backend` is in a module, and so is this.
#[test]
fn a_consumers_own_implementor_of_a_dependencys_abstraction_joins_the_union_through_a_module_path() {
    let d = std::env::temp_dir().join(format!("candor-scan-cli-r475own-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    let pkg = |name: &str, deps: &str, src: &str| {
        let p = d.join(name);
        std::fs::create_dir_all(p.join("src")).unwrap();
        std::fs::write(p.join("Cargo.toml"),
            format!("[package]\nname = \"{name}\"\n\n[dependencies]\n{deps}")).unwrap();
        std::fs::write(p.join("src/lib.rs"), src).unwrap();
        p
    };
    // The abstraction lives in `iface::backend`, not at the crate root — the `ratatui_core` shape.
    let iface = pkg("iface", "",
        "pub mod backend {\n\
         \x20   pub trait Backend { fn size(&self) -> usize; }\n\
         \x20   pub struct TestBackend;\n\
         \x20   impl Backend for TestBackend { fn size(&self) -> usize { 7 } }\n\
         }\n\
         pub fn term_size(b: &dyn backend::Backend) -> usize { b.size() }\n");
    // The consumer implements the dependency's FOREIGN abstraction itself, effectfully.
    let app = pkg("app", "iface = \"1\"\n",
        "use iface::backend::Backend;\n\
         pub struct Mine;\n\
         impl Backend for Mine {\n\
             fn size(&self) -> usize { let _ = std::net::TcpStream::connect(\"h:1\"); 0 }\n\
         }\n\
         pub fn app_size(b: &dyn Backend) -> usize { iface::term_size(b) }\n");

    let scan = |dir: &std::path::Path, deps: &[&std::path::Path]| -> serde_json::Value {
        let mut c = Command::new(bin());
        c.arg(dir.to_string_lossy().as_ref()).arg("--json")
            .env_remove("CANDOR_POLICY").env_remove("CANDOR_CONFIG");
        if deps.is_empty() { c.env_remove("CANDOR_DEPS"); }
        else {
            c.env("CANDOR_DEPS", deps.iter().map(|p| p.to_string_lossy().into_owned())
                .collect::<Vec<_>>().join(" "));
        }
        let out = c.output().expect("run candor-scan");
        serde_json::from_str(String::from_utf8(out.stdout).unwrap().trim()).expect("pure JSON report")
    };
    let row = |v: &serde_json::Value, name: &str| -> Option<serde_json::Value> {
        v["functions"].as_array().unwrap().iter().find(|e| e["fn"] == name).cloned()
    };

    let iface_rep = scan(&iface, &[]);
    // R503 — THE DISCRIMINATING ASSERTION. The leaf spelling (`Backend::size`) and the pre-R503
    // crate-prefixed one (`iface#Backend::size`) are both WRONG here and both would have passed the
    // root-level fixture above. The member must be spelled as the owning package's own entry hashes
    // spell it, which is the same key `collect_foreign_trait_impls` writes for the same abstraction.
    assert_eq!(row(&iface_rep, "term_size").expect("the dispatching row is emitted")["dispatchesOn"],
        serde_json::json!(["iface#backend::Backend::size"]),
        "⟨0.39⟩/R503: ONE spelling for one abstraction — the owning package's entry key: {iface_rep}");
    let iface_p = d.join("iface.json");
    std::fs::write(&iface_p, serde_json::to_string(&iface_rep).unwrap()).unwrap();

    // …and the consumer's own entry is keyed by the QUALIFIED path, per obligation 2.
    let app_alone = scan(&app, &[]);
    assert_eq!(row(&app_alone, "backend::Backend::size").expect(
        "⟨0.39⟩ obligation 2 applies to the CONSUMER too — it implements a foreign abstraction")["hash"],
        "iface#backend::Backend::size", "{app_alone}");

    let chained = scan(&app, &[&iface_p]);
    let app_size = row(&chained, "app_size").expect("app_size dispatches into the dependency");
    assert!(app_size["inferred"].as_array().unwrap().iter().any(|e| e == "Net"),
        "⟨0.39⟩ obligation 3 names the consumer's OWN visible implementors FIRST, and after R503 the \
         member arrives in exactly the spelling `foreign_impls` is keyed by, so the two halves meet \
         without reconciliation. Before it they met only for an abstraction at its crate's ROOT — which \
         no real one is: {app_size}");
}

/// SOUNDNESS R504 / ⟨0.39⟩ obligation 1 — THE MIDDLE PACKAGE, on a FOUR-package chain.
///
/// `dispatch_sites` recorded LOCAL-trait dispatch only (`trait_declares_method` over `trait_decls`), so a
/// package that depends on the abstraction's owner and dispatches over it — owning neither the trait nor
/// any implementor of it — named NOTHING, and the ⟨0.39⟩ chain broke one hop short. Found by candor-java's
/// port reading this engine's source, and measured silent here before it was closed: `iface::backend::
/// Backend` · `mid::term_size(&dyn Backend)` · `effimpl::Crossterm` (effectful) · an app chained onto all
/// three. The app's `app_size` was ABSENT, which under ⟨0.21⟩ is a positive claim of purity — `deny Net`
/// over it exited 0.
///
/// WHY THE THREE-PACKAGE ARMS ABOVE CANNOT SEE IT. In those, the dispatching package IS the owner, so the
/// local route answers and the hole never opens. The defect needs a dispatcher that owns nothing, which
/// takes a fourth package — the same lesson every audit-boundary row in SOUNDNESS records: the fixture
/// that closes one defect is the boundary of the next.
///
/// TWO ASSERTIONS, ONE PACKAGE APART, because the fix has two halves: `mid` must NAME the member
/// (effect-free — it names, it does not hedge), and the app must then reach `effimpl`'s union THROUGH it.
///
/// WHERE THE COMPILE PROOF IS. These fixtures are scanned, not built — like every other arm in this file.
/// The COMPILING twin of this exact program is conformance PART 92's `c6_middle_package`, which renders
/// the same four packages and `cargo build`s the consumer (pulling all three dependencies in) before any
/// engine sees them. That matters most for the pure-only control below, because an absence asserted over
/// a program that cannot exist is not weak evidence but none.
/// ⟨0.39⟩ obligation 2, the LOCAL leg, and the spelling that is the NORMAL case: `pub mod backend`.
///
/// SOUNDNESS R513. A crate declaring a trait AND implementing it locally publishes the union entry only
/// when BOTH sit at the crate ROOT. Nest the identical code in a module and the entry vanished, so a
/// chained consumer read `inferred: []` with NO `invisible` — §2 chaining rule 3 makes that a purity
/// CLAIM, and `deny Net` exited 0 over a dependency whose sole implementor opens a `TcpStream`.
///
/// MECHANISM, because the arms below are otherwise indistinguishable from a passing test: `trait_impls`
/// holds the self type AS WRITTEN (`Net1`, never `backend::Net1` — `decls.rs`), while `inferred` is keyed
/// by the unit's full qual, so both of the local leg's candidate keys missed and the member was scored
/// "pure across all impls". The FOREIGN leg of the same rung already resolved its implementor tails
/// through `by_tail2`; the local leg did not, and that asymmetry WAS the bug.
///
/// TWO ARMS, BYTE-IDENTICAL BUT FOR THE MODULE WRAPPER, because a one-arm version of this test passes on
/// the engine that has the defect: the crate-root arm is the control that says the leg worked at all.
/// Asserted on the KEY as well as the effect — `depmod#backend::Backend::size`, fully qualified in the
/// owning package's namespace (R503) — since a row published under the leaf would satisfy "a row exists"
/// while reintroducing the fourth spelling `69dd565` removed.
#[test]
fn a_trait_and_impl_nested_in_a_module_publish_the_same_union_entry_as_at_the_crate_root() {
    let d = std::env::temp_dir().join(format!("candor-scan-cli-r513-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    let pkg = |name: &str, deps: &str, src: &str| {
        let p = d.join(name);
        std::fs::create_dir_all(p.join("src")).unwrap();
        std::fs::write(p.join("Cargo.toml"), format!(
            "[package]\nname = \"{name}\"\nversion = \"0.1.0\"\nedition = \"2021\"\n\n[dependencies]\n{deps}"))
            .unwrap();
        std::fs::write(p.join("src/lib.rs"), src).unwrap();
        p
    };
    const BODY: &str = "pub trait Backend { fn size(&self) -> usize; }\n\
         pub struct Net1;\n\
         impl Backend for Net1 {\n\
             fn size(&self) -> usize { let _ = std::net::TcpStream::connect(\"h:1\"); 0 }\n\
         }\n";
    let deproot = pkg("deproot", "", BODY);
    let depmod = pkg("depmod", "", &format!("pub mod backend {{\n{BODY}}}\n"));
    let approot = pkg("approot", "deproot = \"1\"\n",
        "pub fn go(b: &dyn deproot::Backend) -> usize { b.size() }\n");
    let appmod = pkg("appmod", "depmod = \"1\"\n",
        "pub fn go(b: &dyn depmod::backend::Backend) -> usize { b.size() }\n");

    let scan = |dir: &std::path::Path, deps: &[&std::path::Path]| -> serde_json::Value {
        let mut c = Command::new(bin());
        c.arg(dir.to_string_lossy().as_ref()).arg("--json")
            .env_remove("CANDOR_POLICY").env_remove("CANDOR_CONFIG");
        if deps.is_empty() {
            c.env_remove("CANDOR_DEPS");
        } else {
            c.env("CANDOR_DEPS", deps.iter().map(|p| p.to_string_lossy().into_owned())
                .collect::<Vec<_>>().join(" "));
        }
        let out = c.output().expect("run candor-scan");
        serde_json::from_str(String::from_utf8(out.stdout).unwrap().trim()).expect("pure JSON report")
    };
    let by_hash = |v: &serde_json::Value, h: &str| -> Option<serde_json::Value> {
        v["functions"].as_array().unwrap().iter().find(|e| e["hash"] == h).cloned()
    };
    let write = |v: &serde_json::Value, file: &str| -> std::path::PathBuf {
        let p = d.join(file);
        std::fs::write(&p, serde_json::to_string(v).unwrap()).unwrap();
        p
    };

    for (dep, app, key) in [
        (&deproot, &approot, "deproot#Backend::size"),
        (&depmod, &appmod, "depmod#backend::Backend::size"),
    ] {
        let rep = scan(dep, &[]);
        let union = by_hash(&rep, key).unwrap_or_else(|| panic!(
            "⟨0.39⟩ obligation 2: the local union entry must be published under `{key}` — R513 measured \
             it ABSENT for the module-nested spelling while the byte-identical crate-root one published \
             it, and the consumer below then read the silence as purity: {rep}"));
        assert_eq!(union["interfaceUnion"], serde_json::json!(true), "{union}");
        assert_eq!(union["inferred"], serde_json::json!(["Net"]),
            "…carrying the implementor's REAL effect, not an `Unknown` hedge: {union}");

        let dep_p = write(&rep, &format!("{}.json", dep.file_name().unwrap().to_string_lossy()));
        let chained = scan(app, &[&dep_p]);
        let go = by_hash(&chained, &format!("{}#go", app.file_name().unwrap().to_string_lossy()))
            .unwrap_or_else(|| panic!(
                "the chained consumer's `go` must carry the dispatched effect; ABSENT is the cardinal \
                 sin R513 filed — no row, no `invisible`, and `deny Net` at exit 0: {chained}"));
        assert!(go["inferred"].as_array().unwrap().iter().any(|e| e == "Net"),
            "the dep's implementor effect must reach the consumer through the union entry: {go}");
    }
}

#[test]
fn a_middle_package_dispatching_over_its_dependencys_abstraction_names_the_member_too() {
    let d = std::env::temp_dir().join(format!("candor-scan-cli-r504-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    let pkg = |name: &str, deps: &str, src: &str| {
        let p = d.join(name);
        std::fs::create_dir_all(p.join("src")).unwrap();
        std::fs::write(p.join("Cargo.toml"),
            format!("[package]\nname = \"{name}\"\n\n[dependencies]\n{deps}")).unwrap();
        std::fs::write(p.join("src/lib.rs"), src).unwrap();
        p
    };
    // The OWNER of the abstraction. In a module, not at the root — the `ratatui_core` shape, and the one
    // that distinguishes the wire spellings (R503).
    let iface = pkg("iface", "",
        "pub mod backend {\n\
         \x20   pub trait Backend { fn size(&self) -> usize; }\n\
         \x20   pub struct TestBackend;\n\
         \x20   impl Backend for TestBackend { fn size(&self) -> usize { 7 } }\n\
         }\n");
    // THE MIDDLE PACKAGE: it dispatches over its own dependency's trait and owns nothing.
    let mid = pkg("mid", "iface = \"1\"\n",
        "use iface::backend::Backend;\n\
         pub fn term_size(b: &dyn Backend) -> usize { b.size() }\n");
    // The THIRD package: the effectful implementor of the same foreign abstraction.
    let effimpl = pkg("effimpl", "iface = \"1\"\n",
        "pub struct Crossterm;\n\
         impl iface::backend::Backend for Crossterm {\n\
             fn size(&self) -> usize { let _ = std::net::TcpStream::connect(\"h:1\"); 0 }\n\
         }\n");
    // The consumer, which spells no dispatch of its own.
    let app = pkg("app", "iface = \"1\"\nmid = \"1\"\neffimpl = \"1\"\n",
        "pub fn app_size(b: &dyn iface::backend::Backend) -> usize { mid::term_size(b) }\n\
         pub fn app_run() -> usize { app_size(&effimpl::Crossterm) }\n");

    let scan = |dir: &std::path::Path, deps: &[&std::path::Path]| -> serde_json::Value {
        let mut c = Command::new(bin());
        c.arg(dir.to_string_lossy().as_ref()).arg("--json")
            .env_remove("CANDOR_POLICY").env_remove("CANDOR_CONFIG");
        if deps.is_empty() { c.env_remove("CANDOR_DEPS"); }
        else {
            c.env("CANDOR_DEPS", deps.iter().map(|p| p.to_string_lossy().into_owned())
                .collect::<Vec<_>>().join(" "));
        }
        let out = c.output().expect("run candor-scan");
        serde_json::from_str(String::from_utf8(out.stdout).unwrap().trim()).expect("pure JSON report")
    };
    let row = |v: &serde_json::Value, name: &str| -> Option<serde_json::Value> {
        v["functions"].as_array().unwrap().iter().find(|e| e["fn"] == name).cloned()
    };
    let write = |v: &serde_json::Value, file: &str| -> std::path::PathBuf {
        let p = d.join(file);
        std::fs::write(&p, serde_json::to_string(v).unwrap()).unwrap();
        p
    };

    let iface_p = write(&scan(&iface, &[]), "iface.json");

    // ── HALF 1: the middle package NAMES the member, under the owner's key ─────────────────────────
    let mid_rep = scan(&mid, &[&iface_p]);
    let term = row(&mid_rep, "term_size").expect(
        "R504: the MIDDLE package must name the dependency's member it dispatches on, even though it \
         owns neither the abstraction nor any implementor of it — the key is the dependency's and is \
         already fully qualified, so naming it invents nothing. Absence here is the purity claim");
    assert_eq!(term["dispatchesOn"], serde_json::json!(["iface#backend::Backend::size"]),
        "…keyed under the OWNING crate, which is why the value has to carry the `#` itself: prefixing \
         the DISPATCHING crate would name the wrong owner entirely: {term}");
    assert_eq!(term["inferred"].as_array().map(Vec::len).unwrap_or(0), 0,
        "…and effect-free: it names, it does not hedge. A middle package that started charging `Unknown` \
         on `a dispatch occurred` would bill every consumer of every dispatching library: {term}");

    // ── HALF 2: the app reaches the THIRD package's implementor THROUGH it ─────────────────────────
    let mid_p = write(&mid_rep, "mid.json");
    let eff_p = write(&scan(&effimpl, &[&iface_p]), "effimpl.json");
    let chained = scan(&app, &[&iface_p, &mid_p, &eff_p]);
    let app_size = row(&chained, "app_size").expect(
        "…and the consumer reaches the third package's implementor through it. Absence here is R475's \
         purity claim, four packages out");
    assert!(app_size["inferred"].as_array().unwrap().iter().any(|e| e == "Net"),
        "the foreign implementor's effect must REACH the consumer across the middle package: {app_size}");

    // ── CONTROL (the fabrication guard, one package further out than PART 92's c3_pure_only): with
    //    the effectful implementor NOT in the chain, the same consumer source must gain nothing. A
    //    middle package that names a member is a disclosure; unioning on the name alone is a charge.
    let pure_only = scan(&app, &[&iface_p, &mid_p]);
    assert!(row(&pure_only, "app_size").is_none_or(|e| !e["inferred"].as_array()
                .is_some_and(|a| a.iter().any(|x| x == "Net"))),
        "a consumer over a chain whose only implementor ANYWHERE is pure must gain no effect — a miss \
         adds nothing, which is what keeps this a disclosure rather than a hedge: {pure_only}");
}

/// ⟨0.39⟩ THE REPORT MUST BE BYTE-STABLE, and obligation 2 is what stopped it being so.
///
/// Entries were sorted on `fn` alone, which was a total order for as long as one package's entries all
/// carried one package's hashes. Obligation 2 breaks that: a crate implementing the SAME member of TWO
/// different owners' abstractions (`reqwest` really does emit `Service::call` under both `tower#` and
/// `tower_service#`) produces two rows that tie on `fn`, and a stable sort then preserves whatever order
/// the `foreign_impls` HashMap iteration happened to produce. Measured live on reqwest-0.13.5: two runs
/// of ONE binary over ONE crate, identical row multiset, different bytes.
///
/// Asserted by RE-RUNNING rather than by inspecting the order, because the order is not the property —
/// same input, same bytes is. A single run cannot fail this test however the tie is resolved.
#[test]
fn two_owners_of_one_member_name_do_not_make_the_report_order_depend_on_hash_iteration() {
    let d = std::env::temp_dir().join(format!("candor-scan-cli-r475order-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(d.join("src")).unwrap();
    std::fs::write(d.join("Cargo.toml"),
        "[package]\nname = \"twoowners\"\n\n[dependencies]\nalpha = \"1\"\nbeta = \"1\"\n").unwrap();
    // One method name, `call`, implemented for two DIFFERENT dependencies' abstractions — the reqwest
    // shape. Both entries are emitted as `fn: "Service::call"`, under `alpha#…` and `beta#…`.
    std::fs::write(d.join("src/lib.rs"),
        "pub struct A;\n\
         impl alpha::Service for A { fn call(&self) { let _ = std::net::TcpStream::connect(\"h:1\"); } }\n\
         pub struct B;\n\
         impl beta::Service for B { fn call(&self) { let _ = std::fs::read(\"/tmp/x\"); } }\n").unwrap();

    let once = || -> String {
        let out = Command::new(bin()).arg(d.to_string_lossy().as_ref()).arg("--json")
            .env_remove("CANDOR_POLICY").env_remove("CANDOR_CONFIG").env_remove("CANDOR_DEPS")
            .output().expect("run candor-scan");
        String::from_utf8(out.stdout).unwrap()
    };
    let first = once();
    let v: serde_json::Value = serde_json::from_str(first.trim()).expect("pure JSON report");
    let unions: Vec<&str> = v["functions"].as_array().unwrap().iter()
        .filter(|e| e["interfaceUnion"] == serde_json::json!(true))
        .filter_map(|e| e["hash"].as_str()).collect();
    assert!(unions.contains(&"alpha#Service::call") && unions.contains(&"beta#Service::call"),
        "the fixture must actually produce the tie this test is about — got {unions:?}\n{v}");
    // Ten runs: a HashMap's iteration order is stable within a process but not across them, so the
    // instability only shows up by re-running the binary.
    for i in 0..10 {
        assert_eq!(once(), first,
            "run {i} produced different BYTES for the same input. Two interface-union entries tie on \
             `fn`, so the entry sort must break the tie on `hash` — otherwise the report order follows \
             `foreign_impls`' hash iteration and no consumer can diff two scans of one tree.");
    }
}

/// SOUNDNESS R511 — **THE ⟨0.29⟩ PEEK READ A REPORT ROW THAT IS NOT A UNIT, AND IT COST A VERDICT.**
///
/// ⟨0.39⟩ un-gated ⟨0.23⟩, so the peek's own recursion now returns synthetic `interfaceUnion` rows: the
/// union over an abstraction member's implementors, with no body and no `loc`. The attribution loop
/// scope-matched the row's `fn` and derived the finding's `path` and exclusion `class` from its absent
/// `loc`, so it published an `outOfScope` finding with an EMPTY PATH and the fabricated class
/// `"excluded"` — and under ⟨0.30⟩ a non-empty `outOfScope` is INCOMPLETE at **exit 2**.
///
/// THE FIXTURE IS A CROSS: one tree, two policies differing ONLY in the scope token. `deny Net Backend`
/// names the TRAIT, which the real implementor `examples::ex::Crossterm::size` does not match and the
/// synthetic row does — so before the fix the defect arm exited 2 with one path-less finding while the
/// control exited 0, and the entire difference was a row with no body. candor-swift measured and fixed
/// the same reader first; candor-ts hit the class in its gate instead.
///
/// The FOREIGN half of the rung is what makes an excluded file produce one at all: `examples/` is
/// excluded as `non-library-target`, and a file there implementing a DEPENDENCY's trait emits the
/// obligation-2 union keyed under the owning package.
#[test]
fn r511_a_synthetic_union_row_in_the_peek_does_not_publish_a_pathless_out_of_scope_finding() {
    let d = std::env::temp_dir().join(format!("candor-scan-cli-r511-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(d.join("src")).unwrap();
    std::fs::create_dir_all(d.join("examples")).unwrap();
    std::fs::write(d.join("Cargo.toml"),
        "[package]\nname = \"peekpkg\"\nversion = \"0.1.0\"\n\n[dependencies]\niface = \"1\"\n").unwrap();
    std::fs::write(d.join("src/lib.rs"), "pub fn lib_entry() -> usize { 1 }\n").unwrap();
    // EXCLUDED (`non-library-target`), and it implements the DEPENDENCY's abstraction effectfully — the
    // obligation-2 shape, which is the one that emits a union row from a file the primary scan skips.
    std::fs::write(d.join("examples/ex.rs"),
        "pub struct Crossterm;\n\
         impl iface::Backend for Crossterm {\n\
             fn size(&self) -> usize { let _ = std::net::TcpStream::connect(\"h:1\"); 0 }\n\
         }\n\
         fn main() { let c = Crossterm; let _ = c.size(); }\n").unwrap();

    let scan = |policy: &str| -> (i32, serde_json::Value) {
        let p = d.join("policy");
        std::fs::write(&p, policy).unwrap();
        let out = Command::new(bin())
            .arg(d.to_string_lossy().as_ref()).arg("--json")
            .env("CANDOR_POLICY", &p)
            .env_remove("CANDOR_CONFIG").env_remove("CANDOR_DEPS")
            .output().expect("run candor-scan");
        let v = serde_json::from_str(String::from_utf8(out.stdout).unwrap().trim())
            .expect("pure JSON report");
        (out.status.code().unwrap_or(-1), v)
    };

    // PROVE THE FIXTURE REACHES THE CODE: the scan of the excluded tree must actually mint a union row,
    // or this test passes for the wrong reason (it is the same shape that made the coordinator's first
    // probe come back empty — a crate whose trait sits in a MODULE emits none).
    let inc = Command::new(bin())
        .arg(d.to_string_lossy().as_ref()).arg("--json").arg("--include-tests")
        .env_remove("CANDOR_POLICY").env_remove("CANDOR_CONFIG").env_remove("CANDOR_DEPS")
        .output().expect("run candor-scan");
    let iv: serde_json::Value =
        serde_json::from_str(String::from_utf8(inc.stdout).unwrap().trim()).expect("pure JSON report");
    assert!(
        iv["functions"].as_array().unwrap().iter().any(|e| e["interfaceUnion"] == serde_json::json!(true)),
        "the fixture must PRODUCE a union row over the excluded file set, or this test cannot fail: {iv}"
    );

    // ── THE DEFECT ARM: a rule scoped to the ABSTRACTION's name ────────────────────────────────────
    let (code, v) = scan("deny Net Backend\n");
    let oos = v["outOfScope"].as_array().cloned().unwrap_or_default();
    assert!(
        oos.is_empty(),
        "no REAL unit's name matches the scope `Backend` — the only row that did was the synthetic \
         union, which has no body to have performed anything. It published `path: \"\"` and the class \
         `excluded`, a class this run's own `excluded` array does not contain: {v}"
    );
    assert_eq!(
        code, 0,
        "⟨0.30⟩ makes a non-empty `outOfScope` INCOMPLETE at exit 2, so a bodiless row turned a green \
         gate red. The control below is the same tree under `deny Net Zzz`: {v}"
    );

    // ── CONTROL: the same tree, the same rule shape, a scope nothing matches ───────────────────────
    let (ccode, cv) = scan("deny Net Zzz\n");
    assert_eq!(ccode, 0, "control must be green — the arms differ only in the scope token: {cv}");

    // ── AND THE REAL FINDINGS ARE UNTOUCHED: an unscoped rule must still disclose the implementor ──
    let (ucode, uv) = scan("deny Net\n");
    let uoos = uv["outOfScope"].as_array().cloned().unwrap_or_default();
    assert!(
        uoos.iter().any(|f| f["fn"] == serde_json::json!("examples::ex::Crossterm::size")),
        "the excluded implementor that actually performs Net must STILL be disclosed — the filter \
         removes a duplicate of it, never the finding: {uv}"
    );
    assert!(
        uoos.iter().all(|f| !f["path"].as_str().unwrap_or("").is_empty()),
        "every `outOfScope` finding names the file it was read from; an empty path is the tell: {uv}"
    );
    assert_eq!(ucode, 2, "…and an excluded file holding a denied effect is still INCOMPLETE: {uv}");

    let _ = std::fs::remove_dir_all(&d);
}

/// SOUNDNESS **R525** — A CARDINAL SIN: adding an UNRELATED rule that uses a `.candor/config`
/// `unknown-alias` turned a red verdict GREEN and erased the ⟨0.30⟩ disclosure that made it red.
///
/// MEASURED against the published `candor-scan 0.39.0 (spec 0.39)`, over a crate whose `build.rs` opens a
/// `TcpStream` (so the ⟨0.29⟩/⟨0.30⟩ PEEK has something real to find) with `unknown-alias corp = reflect`
/// in the `.candor/config` beside the policy:
///
/// | policy                            | exit | `outOfScope` | `scannedUnder` | `excluded[].peeked` |
/// |-----------------------------------|------|--------------|----------------|---------------------|
/// | `deny Net`                        | 2    | names it     | present        | true                |
/// | `deny Net` + `deny Unknown[corp]` | **0, "policy ✓"** | **ABSENT** | **ABSENT** | **false**  |
/// | `deny Net` + `deny Unknown[reflect]` | 2 | names it     | present        | true                |
///
/// MECHANISM: the peek parsed the policy through the alias-LESS `parse_policy` while the gate used
/// `parse_policy_with_aliases`. With an empty vocabulary `Unknown[corp]` is an unrecognised reason-class,
/// which is FATAL, so the peek's §3.1 refusal arm returned `None` and the fail-closed verdict never armed
/// — one file read through two vocabularies, which is the divergence ⟨0.24⟩ closed between the scan route
/// and `gate --report` and which had reopened one level down.
///
/// **THE THIRD ROW IS WHY THIS IS A FINDING AND NOT A GUESS.** `deny Unknown[reflect]` is the SAME
/// policy shape — two rules, the second an `Unknown[…]` filter the tree does not satisfy — differing from
/// the second row in exactly one thing: whether the token is a BUILTIN reason-class or a config-defined
/// alias. It stayed red throughout, so "a second rule suppresses the peek" is ruled out by measurement
/// rather than by argument.
///
/// **THE FOURTH ROW IS THE DIRECTION GUARD**, and it is the row that must not be sacrificed to fix the
/// others: `deny Unknown[nosuchtok]`, with no such alias defined, is a policy that CANNOT BE HONOURED AS
/// WRITTEN. It must still refuse — exit 2, `refused: true`, no `outOfScope`/`scannedUnder` in the report.
/// Making the peek tolerant enough to swallow row 2 by accepting everything would trade a silent
/// under-report for a silent over-acceptance, which is the shape this register has recorded four times.
#[test]
fn r525_a_config_alias_in_an_unrelated_rule_does_not_erase_the_out_of_scope_disclosure() {
    let d = make_crate("r525alias", "pub fn add(a: i32, b: i32) -> i32 { a + b }\n");
    // The crate's own surface is pure. EVERY effect in this fixture lives in the EXCLUDED build script,
    // so each row below is a statement about the peek and nothing else — if the peek does not run, there
    // is no other route by which `Net` can reach the verdict.
    std::fs::write(d.join("build.rs"),
        "fn grab() { let _ = std::net::TcpStream::connect(\"example.com:80\"); }\nfn main() { grab(); }\n")
        .unwrap();
    std::fs::create_dir_all(d.join(".candor")).unwrap();
    std::fs::write(d.join(".candor/config"), "unknown-alias corp = reflect\n").unwrap();

    let run = |pol: &str, tag: &str| -> (Option<i32>, serde_json::Value, serde_json::Value) {
        let pp = d.join(format!("{tag}.policy"));
        std::fs::write(&pp, pol).unwrap();
        let gate = d.join(format!("{tag}.verdict.json"));
        let _ = std::fs::remove_file(&gate);
        let out = Command::new(bin())
            .args([d.to_string_lossy().as_ref(),
                   "--out", d.join(format!("rep-{tag}")).to_string_lossy().as_ref(),
                   "--policy", pp.to_string_lossy().as_ref(),
                   "--gate-json", gate.to_string_lossy().as_ref()])
            .output().expect("run candor-scan");
        let v: serde_json::Value = serde_json::from_str(
            &std::fs::read_to_string(&gate).unwrap_or_else(|e| panic!(
                "no --gate-json document at {}: {e}; stderr: {}",
                gate.display(), String::from_utf8_lossy(&out.stderr))))
            .expect("the verdict is JSON");
        let rep: serde_json::Value = serde_json::from_str(
            &std::fs::read_to_string(d.join(format!("rep-{tag}.r525alias.scan.json")))
                .expect("a scan report")).expect("the report is JSON");
        (out.status.code(), v, rep)
    };

    // THE BASELINE, and the calibration: without it every row below is a claim about an instrument that
    // was never shown able to fail. `deny Net` alone must find the build script's `Net` and refuse to
    // certify the tree.
    let (base_code, base_v, base_rep) = run("deny Net\n", "base");
    assert_eq!(base_code, Some(2), "calibration: the peek must find build.rs's Net under `deny Net`: {base_v}");
    assert_eq!(base_v["ok"], serde_json::json!(false), "{base_v}");
    assert_eq!(base_v["incomplete"], serde_json::json!(true), "{base_v}");
    assert!(base_rep["outOfScope"].as_array().is_some_and(|a| a.iter().any(|f| f["fn"] == "build::grab")),
        "the disclosure must NAME the function, or the rows below cannot tell an erased one from an empty one: {base_rep}");
    assert!(base_rep.get("scannedUnder").is_some(), "{base_rep}");
    assert_eq!(base_rep["excluded"][0]["peeked"], serde_json::json!(true), "{base_rep}");

    // THE DEFECT. `deny Unknown[corp]` says nothing whatever about `Net`, and the gate route resolves the
    // alias perfectly. Adding it must not change ONE field of the answer the row above gave.
    let (code, v, rep) = run("deny Net\ndeny Unknown[corp]\n", "alias");
    assert_eq!(code, Some(2),
        "R525: an unrelated `deny Unknown[<config alias>]` turned `deny Net` from exit 2 into exit 0 \
         `policy ✓` — the peek parsed the policy with an EMPTY alias vocabulary, called the alias a fatal \
         reason-class error, and withheld the disclosure the gate had no trouble with: {v}");
    assert_eq!(v["ok"], serde_json::json!(false), "{v}");
    assert_eq!(v["incomplete"], serde_json::json!(true), "{v}");
    assert!(rep["outOfScope"].as_array().is_some_and(|a| a.iter().any(|f| f["fn"] == "build::grab")),
        "R525: the ⟨0.30⟩ `outOfScope` disclosure vanished — ABSENCE is the sin's signature, and here it \
         is byte-identical to a tree with no excluded effect at all: {rep}");
    assert_eq!(rep["excluded"][0]["peeked"], serde_json::json!(true),
        "R525: `peeked: false` here means the peek NEVER OPENED the build script under this policy — and \
         `peeked` is an OUTCOME, so a reader is told the file went unread when the only thing that went \
         wrong was the vocabulary the policy was parsed with: {rep}");
    // …and `scannedUnder` records the rule in its ⟨0.33⟩ CANONICAL EXPANDED form, so the alias resolved
    // rather than merely being tolerated: `Unknown[corp]` must come back out as `Unknown[reflect]`.
    assert_eq!(rep["scannedUnder"]["deny"],
        serde_json::json!(["deny Net", "deny Unknown[reflect]"]),
        "R525: the peek must record the deny set THE MATCHER USED, with the config alias EXPANDED — a \
         peek that recorded `Unknown[corp]` would be one that never resolved it: {rep}");

    // THE CONTROL that makes the row above evidence: same shape, builtin class instead of an alias. It
    // was red before the fix and must stay red, so "a second rule suppresses the peek" is excluded.
    let (ccode, cv, crep) = run("deny Net\ndeny Unknown[reflect]\n", "builtin");
    assert_eq!(ccode, Some(2), "the builtin-class control must be unaffected: {cv}");
    assert_eq!(crep["scannedUnder"], rep["scannedUnder"],
        "the alias arm and the builtin arm must produce the SAME deny set — that is what `corp = reflect` \
         means, and it is the only way to show the alias was resolved and not merely ignored:\n{crep}\n{rep}");
    assert_eq!(crep["outOfScope"], rep["outOfScope"], "…and the same disclosure:\n{crep}\n{rep}");

    // THE DIRECTION GUARD. An UNDEFINED token is still a policy that cannot be honoured as written, and
    // must still be REFUSED. This is the row a fix that widened the peek into "accept everything" would
    // break, and it is the more dangerous failure of the two: a refused policy that runs anyway gates on
    // a rule the operator did not write.
    let (bcode, bv, brep) = run("deny Net\ndeny Unknown[nosuchtok]\n", "bogus");
    assert_eq!(bcode, Some(2), "an unhonourable policy is refused: {bv}");
    assert_eq!(bv["refused"], serde_json::json!(true),
        "R525's fix must not buy the rows above by making the peek accept a policy the gate refuses: {bv}");
    assert!(bv.get("violations").is_none(), "a refusal claims nothing about violations: {bv}");
    assert!(brep.get("outOfScope").is_none(),
        "§3.1: a producer that refused the policy must not publish a look it took under it: {brep}");
    assert!(brep.get("scannedUnder").is_none(), "{brep}");

    let _ = std::fs::remove_dir_all(&d);
}

/// SOUNDNESS R549 (mechanism B) — A TRAIT METHOD NAMED AS A FUNCTION REFERENCE IS A DISPATCH, AND THE
/// KEY FOR IT WAS NEVER PUBLISHED.
///
/// `xs.front().map(Buffy::chunk)` spells the dispatch as a VALUE, never as a method call, so
/// `visit_expr_method_call` never saw it. Measured on http-body-util at HEAD: `bytes#Buf::chunk` appeared
/// NOWHERE in the report, while the same function published `bytes#Buf::map` and
/// `bytes#Buf::unwrap_or_default` — neither of which `Buf` declares. ⟨0.39⟩ obligation 3 tells a consumer
/// to JOIN on the key; there was no key to join, so the dispatch read as purity.
///
/// THE CONTROL IS THE POINT AND IT IS WHY THIS IS A THREE-ROW TEST: `via_method` writes the SAME dispatch
/// on the SAME trait through the SAME dep as an ordinary method call and always resolved. One variable —
/// how the dispatch is spelled — and only one spelling was reported.
///
/// The bound is written on the IMPL BLOCK (`impl<T: Buffy>`), deliberately: that is the shape the real
/// code uses, and `sig_trait_quals` reads a FUNCTION SIGNATURE's generics only, so the trait-name index
/// could not see it. `bound_trait_leaves` is the additive index that answers "is this name a trait" —
/// including a BARE-LEAF bound, which `quals_from_bounds` drops on purpose because it carries no crate
/// identity. Qualification still comes from `expand` + the file's `use` map.
#[test]
fn r549_a_trait_method_named_as_a_function_reference_publishes_the_same_key_as_a_method_call() {
    let d = std::env::temp_dir().join(format!("candor-scan-cli-r549-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    let pkg = |name: &str, deps: &str, src: &str| {
        let p = d.join(name);
        std::fs::create_dir_all(p.join("src")).unwrap();
        std::fs::write(p.join("Cargo.toml"),
            format!("[package]\nname = \"{name}\"\n\n[dependencies]\n{deps}")).unwrap();
        std::fs::write(p.join("src/lib.rs"), src).unwrap();
        p
    };
    // A defaulted method and a real impl so the dep report is not HOLLOW — a trait of nothing but
    // bodiless members analyses to `count: 0`, which every downstream number would then be read off.
    let dep = pkg("depcrate", "",
        "pub trait Buffy {\n\
         \x20   fn chunk(&self) -> &[u8];\n\
         \x20   fn describe(&self) -> usize { self.chunk().len() }\n\
         }\n\
         pub struct Real;\n\
         impl Buffy for Real { fn chunk(&self) -> &[u8] { b\"x\" } }\n");
    let app = pkg("appcrate", "depcrate = \"1\"\n",
        "use depcrate::Buffy;\n\
         use std::collections::VecDeque;\n\
         pub struct BufList<T> { pub bufs: VecDeque<T> }\n\
         impl<T: Buffy> BufList<T> {\n\
         \x20   pub fn via_fnref(&self) -> &[u8] {\n\
         \x20       self.bufs.front().map(Buffy::chunk).unwrap_or_default()\n\
         \x20   }\n\
         \x20   pub fn via_method(&self) -> &[u8] {\n\
         \x20       match self.bufs.front() { Some(b) => b.chunk(), None => &[] }\n\
         \x20   }\n\
         }\n");

    let scan = |dir: &std::path::Path, deps: &[&std::path::Path]| -> serde_json::Value {
        let mut c = Command::new(bin());
        c.arg(dir.to_string_lossy().as_ref()).arg("--json");
        if deps.is_empty() {
            c.env_remove("CANDOR_DEPS");
        } else {
            c.env("CANDOR_DEPS", deps.iter().map(|p| p.to_string_lossy().into_owned())
                .collect::<Vec<_>>().join(" "));
        }
        let out = c.output().expect("run candor-scan");
        serde_json::from_str(String::from_utf8(out.stdout).unwrap().trim()).expect("pure JSON report")
    };
    let row = |v: &serde_json::Value, name: &str| -> Option<serde_json::Value> {
        v["functions"].as_array().unwrap().iter().find(|e| e["fn"] == name).cloned()
    };

    let dep_rep = scan(&dep, &[]);
    assert_ne!(dep_rep["analyzed"]["count"].as_u64().unwrap_or(0), 0,
        "the dep report must not be HOLLOW — `analyzed.count: 0` makes every downstream figure a \
         reading off nothing (SOUNDNESS R242)");
    let dep_path = d.join("dep.json");
    std::fs::write(&dep_path, serde_json::to_string(&dep_rep).unwrap()).unwrap();

    let app_rep = scan(&app, &[&dep_path]);
    let keys = |name: &str| -> Vec<String> {
        row(&app_rep, name)
            .unwrap_or_else(|| panic!("{name} must be emitted"))
            ["dispatchesOn"].as_array().cloned().unwrap_or_default()
            .iter().filter_map(|k| k.as_str().map(str::to_string)).collect()
    };

    // THE CONTROL — the ordinary spelling, which has always worked.
    assert!(keys("BufList::via_method").iter().any(|k| k == "depcrate#Buffy::chunk"),
        "CONTROL: a dispatch written as a method call names the member: {:?}",
        keys("BufList::via_method"));

    // THE ROW THIS TEST EXISTS FOR — the same dispatch, named as a function reference.
    assert!(keys("BufList::via_fnref").iter().any(|k| k == "depcrate#Buffy::chunk"),
        "SOUNDNESS R549: `map(Buffy::chunk)` IS a dispatch on `Buffy::chunk`, and the key must be \
         published so a consumer can join it under ⟨0.39⟩ obligation 3. Before the fix this row \
         carried ONLY `Buffy::map` and `Buffy::unwrap_or_default` — two members `Buffy` does not \
         declare — while the member it really dispatches on appeared nowhere in the report: {:?}",
        keys("BufList::via_fnref"));
}

/// SOUNDNESS R551 (R549 mechanism A) — AN EXTENSION TRAIT'S METHOD, PUBLISHED UNDER THE BASE TRAIT THE
/// RECEIVER HAPPENS TO CARRY.
///
/// `inner.map_future(..)` where `inner: S, S: tower_service::Service<R>` published
/// `tower_service#Service::map_future`. `tower_service::Service` declares exactly `call` and
/// `poll_ready`; `map_future` is a member of `ServiceExt`, a trait TOWER DECLARES LOCALLY. So the key
/// named a member of nothing — ⟨0.39⟩ obligation 3's join can never find it — and the key that IS
/// joinable, `tower#util::ServiceExt::map_future`, was never published at all. Absence is the sin's
/// signature, and that second half is the same loss shape as the fn-ref half fixed in `186e854`.
///
/// THE CONTROL IS THE POINT. `via_base` calls a method the DEP's trait really declares, through the same
/// receiver, the same bound and the same dep — one variable, WHICH METHOD — and must still publish
/// `depcrate#Sink::emit`. Without it this test would pass just as well for a fix that silenced every
/// foreign key, which is the cardinal-sin direction.
///
/// THE FIXTURE COMPILES — verified with a path dependency before any assertion was read (§E3: a control
/// that asserts an absence must compile and run, or no correct engine could pass it differently). That
/// matters most for `via_collision`: `s.poll_emit()` building at all is the proof that a supertrait and
/// its extension MAY declare one name, resolved by receiver type rather than by ambiguity.
///
/// WHAT THIS FIXTURE DOES NOT PIN, said plainly: `depcrate` here never implements `Sink::emit_twice`,
/// which is what licenses the rewrite. The rule's OTHER half — a crate that DOES implement the member
/// for the base trait keeps the base-trait key — is not exercisable from this two-crate shape and is
/// pinned by the corpus measurement instead (futures-lite's `StreamExt::poll_next` beside
/// `Stream::poll_next`, recorded in CHANGELOG.md). Stated rather than implied, because a supertrait
/// relation does NOT forbid the base trait from declaring the same name: the receiver types differ and
/// the call resolves without ambiguity. An earlier draft of this fix asserted that it did.
#[test]
fn r551_an_extension_traits_method_is_keyed_to_the_trait_that_declares_it_not_the_base_trait() {
    let d = std::env::temp_dir().join(format!("candor-scan-cli-r551-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    let pkg = |name: &str, deps: &str, src: &str| {
        let p = d.join(name);
        std::fs::create_dir_all(p.join("src")).unwrap();
        std::fs::write(p.join("Cargo.toml"),
            format!("[package]\nname = \"{name}\"\n\n[dependencies]\n{deps}")).unwrap();
        std::fs::write(p.join("src/lib.rs"), src).unwrap();
        p
    };
    // A defaulted method and a real impl so the dep report is not HOLLOW — a trait of nothing but
    // bodiless members analyses to `count: 0`, and every downstream number would be read off nothing.
    let dep = pkg("depcrate", "",
        "use std::pin::Pin;\n\
         pub trait Sink {\n\
         \x20   fn emit(&self) -> usize;\n\
         \x20   fn poll_emit(self: Pin<&mut Self>) -> usize;\n\
         \x20   fn describe(&self) -> usize { self.emit() }\n\
         }\n\
         pub struct Real;\n\
         impl Sink for Real {\n\
         \x20   fn emit(&self) -> usize { 1 }\n\
         \x20   fn poll_emit(self: Pin<&mut Self>) -> usize { 2 }\n\
         }\n");
    // `SinkExt: Sink` is the shape: an extension trait declared HERE over an abstraction declared THERE,
    // with the blanket impl that makes it callable on any `Sink`.
    let app = pkg("appcrate", "depcrate = \"1\"\n",
        "use depcrate::Sink;\n\
         use std::pin::Pin;\n\
         pub trait SinkExt: Sink {\n\
         \x20   fn emit_twice(&self) -> usize { self.emit() + self.emit() }\n\
         \x20   fn poll_emit(&mut self) -> usize where Self: Unpin + Sized {\n\
         \x20       Sink::poll_emit(Pin::new(self))\n\
         \x20   }\n\
         }\n\
         impl<T: Sink + ?Sized> SinkExt for T {}\n\
         pub struct Mine;\n\
         impl Sink for Mine {\n\
         \x20   fn emit(&self) -> usize { 3 }\n\
         \x20   fn poll_emit(self: Pin<&mut Self>) -> usize { 4 }\n\
         }\n\
         pub fn via_collision<T: Sink + Unpin>(s: &mut T) -> usize { s.poll_emit() }\n\
         pub trait CountExt: Iterator {\n\
         \x20   fn tally(&mut self) -> usize { 0 }\n\
         }\n\
         impl<T: Iterator + ?Sized> CountExt for T {}\n\
         pub fn via_ext<T: Sink>(s: &T) -> usize { s.emit_twice() }\n\
         pub fn via_base<T: Sink>(s: &T) -> usize { s.emit() }\n\
         pub fn via_prelude<I: Iterator<Item = u8>>(mut it: I) -> usize { it.tally() }\n");

    let scan = |dir: &std::path::Path, deps: &[&std::path::Path]| -> serde_json::Value {
        let mut c = Command::new(bin());
        c.arg(dir.to_string_lossy().as_ref()).arg("--json");
        if deps.is_empty() {
            c.env_remove("CANDOR_DEPS");
        } else {
            c.env("CANDOR_DEPS", deps.iter().map(|p| p.to_string_lossy().into_owned())
                .collect::<Vec<_>>().join(" "));
        }
        let out = c.output().expect("run candor-scan");
        serde_json::from_str(String::from_utf8(out.stdout).unwrap().trim()).expect("pure JSON report")
    };

    let dep_rep = scan(&dep, &[]);
    assert_ne!(dep_rep["analyzed"]["count"].as_u64().unwrap_or(0), 0,
        "the dep report must not be HOLLOW — `analyzed.count: 0` makes every downstream figure a \
         reading off nothing (SOUNDNESS R242)");
    let dep_path = d.join("dep.json");
    std::fs::write(&dep_path, serde_json::to_string(&dep_rep).unwrap()).unwrap();

    let app_rep = scan(&app, &[&dep_path]);
    let keys = |name: &str| -> Vec<String> {
        app_rep["functions"].as_array().unwrap().iter()
            .find(|e| e["fn"] == name)
            .unwrap_or_else(|| panic!("{name} must be emitted; report: {app_rep}"))
            ["dispatchesOn"].as_array().cloned().unwrap_or_default()
            .iter().filter_map(|k| k.as_str().map(str::to_string)).collect()
    };

    // THE CONTROL — a member the DEP's own trait really declares still keys to the dep. Same receiver,
    // same bound, same dep; the method is the only variable.
    assert!(keys("via_base").iter().any(|k| k == "depcrate#Sink::emit"),
        "CONTROL: a genuine base-trait member must still be keyed to the base trait: {:?}",
        keys("via_base"));

    // THE ROW THIS TEST EXISTS FOR.
    let ext = keys("via_ext");
    assert!(ext.iter().any(|k| k == "appcrate#SinkExt::emit_twice"),
        "SOUNDNESS R551: `emit_twice` is declared by the LOCAL extension trait `SinkExt`, so the \
         published key must name `SinkExt` — that is the key a consumer can join under ⟨0.39⟩ \
         obligation 3: {ext:?}");
    assert!(!ext.iter().any(|k| k == "depcrate#Sink::emit_twice"),
        "SOUNDNESS R551: `depcrate#Sink::emit_twice` names a member `Sink` does not declare, so no \
         consumer could ever join it — it must not be published: {ext:?}");

    // THE SECOND CONTROL — THE REWRITE IS A REWRITE, NEVER AN ADDITION. `CountExt: Iterator` is the
    // same extension-trait shape over a PRELUDE bound, which publishes no key today because the name
    // expands to a bare leaf with no owner. A draft that omitted that gate minted keys here, and on
    // rayon (`trait Producer: Send + Sized`) it claimed `OptionProducer::into_iter` dispatches on
    // `Producer::into_iter` where the call is `Option::into_iter` — a FABRICATED dispatch a consumer
    // would union implementors onto. Found by auditing the A/B's ADDED column, which is why this
    // control exists at all.
    let prelude: Vec<String> = app_rep["functions"].as_array().unwrap().iter()
        .find(|e| e["fn"] == "via_prelude")
        .map(|r| r["dispatchesOn"].as_array().cloned().unwrap_or_default()
            .iter().filter_map(|k| k.as_str().map(str::to_string)).collect())
        .unwrap_or_default();
    assert!(prelude.is_empty(),
        "SOUNDNESS R551: a prelude-rooted bound publishes no key today, so the rewrite must add \
         none — it fires only where a MALFORMED key would otherwise have been published: {prelude:?}");

    // THE THIRD CONTROL — A NAME COLLISION, WHICH IS NOT A MALFORMED KEY. `SinkExt::poll_emit(&mut
    // self)` sits beside `Sink::poll_emit(self: Pin<&mut Self>)`: legal, unambiguous, and resolved by
    // the RECEIVER TYPE, which this engine does not model. It is the real shape — futures-lite's
    // `StreamExt::poll_next` beside `Stream::poll_next` — and it is why the supertrait relation is
    // EVIDENCE and not a proof. `appcrate` implements `Sink::poll_emit` for `Mine`, so obligation 2's
    // index says the base trait really has the member, and the base-trait key STAYS. That is the
    // conservative choice on purpose: `depcrate#Sink::poll_emit` is joinable and reaches the
    // implementors that carry effects, while the extension trait's key reaches a defaulted body one hop
    // away. Without this guard the fix rewrote 11 real `futures_core#stream::Stream::poll_next` rows.
    let coll = keys("via_collision");
    assert!(coll.iter().any(|k| k == "depcrate#Sink::poll_emit"),
        "SOUNDNESS R551: `appcrate` implements `Sink::poll_emit`, so the base trait demonstrably HAS \
         the member and its key must survive the extension-trait rewrite: {coll:?}");

    let _ = std::fs::remove_dir_all(&d);
}

/// SOUNDNESS R597 — **A REAL ROW AT THE MEMBER KEY USED TO DISCARD THE IMPLEMENTOR UNION, AND THE TWO
/// ANSWER DIFFERENT QUESTIONS.**
///
/// The interface-union entry was emitted *only* where no real row claimed its hash. A trait method with a
/// DEFAULT BODY is itself an analysed unit at exactly that hash, so the union over the IMPLEMENTORS'
/// OVERRIDES was thrown away and `crate#Trait::m` published the default body alone — under the one key
/// ⟨0.39⟩ §4 obligation 3 tells a chained consumer to join a dyn dispatch on. Neither set contains the
/// other: the default body is what a non-overriding implementor runs, the overrides are what everybody
/// else runs.
///
/// Measured over 1,626 cargo-registry crates before the fix: 244 suppressions reached, 233 where the
/// union added nothing, **11 where it knew more** — five of them a published key reading `inferred: []`
/// with no disclosure at all, which §2 makes a purity claim, while this engine had computed `Unknown` for
/// the dispatch targets and discarded it.
///
/// **THE OVER-CHARGE CONTROL IS THE FIRST TWO ASSERTIONS, AND THEY ARE THE POINT.** The repair that
/// suggests itself — merge the union INTO the real row — is the CHARGING direction and it fabricates: the
/// real row carries a `loc` and is a concrete body, and `calls_the_default` statically binds to it. Both
/// must keep `["Fs"]` and neither may gain `Net`. The union goes BESIDE the row instead, which two
/// entries under one key already mean family-wide (candor-spec/ENTRY-COLLISION-DECISION.md, conformance
/// PART 26) — `deps.rs` unions them at the consumer.
#[test]
fn r597_a_default_bodys_row_no_longer_discards_the_implementor_union() {
    let d = make_crate(
        "r597default",
        "pub trait Sink {\n\
         \x20   fn emit(&self) { let _ = std::fs::read(\"/tmp/a\"); }\n\
         }\n\
         pub struct Silent;\n\
         impl Sink for Silent {}\n\
         pub struct Loud;\n\
         impl Sink for Loud { fn emit(&self) { let _ = std::net::TcpStream::connect(\"h:1\"); } }\n\
         pub fn calls_the_default() { let s = Silent; s.emit(); }\n",
    );
    let out = Command::new(bin())
        .arg(d.to_string_lossy().as_ref()).arg("--json")
        .env_remove("CANDOR_POLICY").env_remove("CANDOR_CONFIG").env_remove("CANDOR_DEPS")
        .output().expect("run candor-scan");
    let _ = std::fs::remove_dir_all(&d);
    let v: serde_json::Value =
        serde_json::from_str(String::from_utf8(out.stdout).unwrap().trim()).expect("pure JSON report");
    let fns = v["functions"].as_array().unwrap();
    let eff = |e: &serde_json::Value| -> Vec<String> {
        e["inferred"].as_array().map(|a| a.iter().map(|x| x.as_str().unwrap().to_string()).collect())
            .unwrap_or_default()
    };

    // ── OVER-CHARGE CONTROL 1: the ANALYSED UNIT at that hash keeps its own bytes ──────────────────
    let real: Vec<&serde_json::Value> = fns.iter()
        .filter(|e| e["hash"] == "r597default#Sink::emit" && e["interfaceUnion"] != serde_json::json!(true))
        .collect();
    assert_eq!(real.len(), 1, "exactly one ANALYSED unit at the member hash: {v}");
    assert_eq!(eff(real[0]), vec!["Fs".to_string()],
        "THE DEFAULT BODY IS A CONCRETE BODY AT A CONCRETE `loc`, and it reads a file — it does not open \
         a socket. Merging the implementor union into this row would charge `Net` to `src/lib.rs:2`, \
         which is the fabrication direction this fix exists to avoid taking: {v}");
    assert!(real[0]["loc"].is_string(), "the analysed unit must keep its location: {v}");

    // ── OVER-CHARGE CONTROL 2: a caller that can only ever reach the default body ──────────────────
    let caller = fns.iter().find(|e| e["fn"] == "calls_the_default").unwrap_or_else(|| panic!("{v}"));
    assert_eq!(eff(caller), vec!["Fs".to_string()],
        "`Silent` does not override `emit`, so this call runs the default body and nothing else. A merge \
         would have charged it `Net` from a SIBLING implementor it cannot reach: {v}");

    // ── THE FIX: the union sits BESIDE the real row, carrying what the overrides do ────────────────
    let union: Vec<&serde_json::Value> = fns.iter()
        .filter(|e| e["hash"] == "r597default#Sink::emit" && e["interfaceUnion"] == serde_json::json!(true))
        .collect();
    assert_eq!(union.len(), 1,
        "the union over `Sink`'s implementors must be PUBLISHED, not discarded because the member \
         happens to have a default body. Without it `r597default#Sink::emit` answers a chained \
         consumer's dyn dispatch with the default body alone: {v}");
    assert_eq!(eff(union[0]), vec!["Net".to_string()],
        "`Loud::emit` opens a TcpStream and it is the only override: {v}");
    assert!(union[0]["loc"].is_null(), "a synthetic union row has no body and must carry no `loc`: {v}");
}

/// SOUNDNESS R597, THE SECOND ROUTE INTO THE SAME SUPPRESSION — **AN IMPLEMENTOR TYPE WHOSE LEAF EQUALS
/// THE TRAIT'S.** Found by tracing the one corpus case that dropped a CONCRETE effect rather than an
/// `Unknown`, and the mechanism is NOT the default body: `portable_pty`'s `Child` trait has no default
/// body at all. `impl Child for std::process::Child` is keyed by the self type's LEAF, which is also
/// `Child`, so the implementor's own unit claims `portable_pty#Child::wait` — the trait member key — and
/// the union over all three implementors was dropped. `SerialChild::wait` (src/serial.rs:153) contains
/// `log::error!("Error reading carrier detect: {:#}", err)`, and that `Log` was the effect the published
/// key stopped carrying.
///
/// The over-charge control has MORE teeth here than in the default-body case: the row at that hash is a
/// concrete implementor's method for a FOREIGN type, so merging would charge one implementor's `Log` to
/// another implementor's body.
#[test]
fn r597_an_implementor_leaf_equal_to_the_traits_does_not_discard_the_union() {
    let d = make_crate(
        "r597leaf",
        "pub trait Child { fn wait(&self); }\n\
         impl Child for std::process::Child {\n\
         \x20   fn wait(&self) { let _ = std::process::Command::new(\"true\").status(); }\n\
         }\n\
         pub struct SerialChild;\n\
         impl Child for SerialChild { fn wait(&self) { log::error!(\"carrier detect\"); } }\n",
    );
    std::fs::write(d.join("Cargo.toml"),
        "[package]\nname = \"r597leaf\"\n\n[dependencies]\nlog = \"0.4\"\n").unwrap();
    let out = Command::new(bin())
        .arg(d.to_string_lossy().as_ref()).arg("--json")
        .env_remove("CANDOR_POLICY").env_remove("CANDOR_CONFIG").env_remove("CANDOR_DEPS")
        .output().expect("run candor-scan");
    let _ = std::fs::remove_dir_all(&d);
    let v: serde_json::Value =
        serde_json::from_str(String::from_utf8(out.stdout).unwrap().trim()).expect("pure JSON report");
    let fns = v["functions"].as_array().unwrap();
    let eff = |e: &serde_json::Value| -> Vec<String> {
        e["inferred"].as_array().map(|a| a.iter().map(|x| x.as_str().unwrap().to_string()).collect())
            .unwrap_or_default()
    };

    // ── OVER-CHARGE CONTROL: `std::process::Child::wait` spawns nothing that LOGS ──────────────────
    let real: Vec<&serde_json::Value> = fns.iter()
        .filter(|e| e["hash"] == "r597leaf#Child::wait" && e["interfaceUnion"] != serde_json::json!(true))
        .collect();
    assert_eq!(real.len(), 1, "exactly one ANALYSED unit at that hash: {v}");
    assert_eq!(eff(real[0]), vec!["Exec".to_string()],
        "this row IS `impl Child for std::process::Child`'s `wait`, at its own `loc`. `Log` belongs to a \
         DIFFERENT implementor; charging it here would attribute one type's body to another's: {v}");

    // ── THE FIX ───────────────────────────────────────────────────────────────────────────────────
    let union: Vec<&serde_json::Value> = fns.iter()
        .filter(|e| e["hash"] == "r597leaf#Child::wait" && e["interfaceUnion"] == serde_json::json!(true))
        .collect();
    assert_eq!(union.len(), 1, "the trait member key must still publish the union: {v}");
    assert_eq!(eff(union[0]), vec!["Exec".to_string(), "Log".to_string()],
        "both implementors, and the `Log` is the one the suppression dropped on portable-pty: {v}");
}

/// SOUNDNESS R597 — **THE CONSUMER SIDE, WHICH IS THE ONLY PLACE THE DEFECT WAS EVER VISIBLE.**
///
/// One tree, one consumer binary, one dep tree; the ONLY variable is whether the dependency's report
/// carries the beside-union row. The consumer dispatches on `&dyn Sink`, which is the shape ⟨0.39⟩ §4
/// obligation 3 exists for. Before the fix `deny Net` over `app::run` exits 0 while the dependency's only
/// effectful implementor opens a socket — the ⟨0.39⟩ toggle one spelling over, reached through a member
/// that happens to have a default body.
///
/// The PRE image is constructed by DELETING the union row from a report this binary produced, rather than
/// hand-authoring one: a hand-authored report proves the consumer joins SOMETHING, not that the producer
/// emits it.
#[test]
fn r597_a_chained_consumer_gains_the_implementor_effect_through_the_member_key() {
    let dep = make_crate(
        "r597dep",
        "pub trait Sink {\n\
         \x20   fn emit(&self) { let _ = std::fs::read(\"/tmp/a\"); }\n\
         }\n\
         pub struct Loud;\n\
         impl Sink for Loud { fn emit(&self) { let _ = std::net::TcpStream::connect(\"h:1\"); } }\n",
    );
    let outdir = dep.join("rep");
    std::fs::create_dir_all(&outdir).unwrap();
    let st = Command::new(bin())
        .arg(dep.to_string_lossy().as_ref())
        .arg("--out").arg(outdir.join("r").to_string_lossy().as_ref())
        .env_remove("CANDOR_POLICY").env_remove("CANDOR_CONFIG").env_remove("CANDOR_DEPS")
        .output().expect("run candor-scan");
    assert!(st.status.success(), "producing the dep report must succeed: {}",
        String::from_utf8_lossy(&st.stderr));
    let rep_path = outdir.join("r.r597dep.scan.json");
    let post: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&rep_path).unwrap()).unwrap();

    // The PRE image: the same report with the beside-union row removed — exactly what the suppression
    // produced. Assert it was THERE, or this test's two arms are the same document.
    let mut pre = post.clone();
    let before = pre["functions"].as_array().unwrap().len();
    pre["functions"] = serde_json::Value::Array(
        pre["functions"].as_array().unwrap().iter()
            .filter(|e| !(e["hash"] == "r597dep#Sink::emit"
                          && e["interfaceUnion"] == serde_json::json!(true)))
            .cloned().collect());
    assert_eq!(pre["functions"].as_array().unwrap().len() + 1, before,
        "the producer must have emitted exactly one beside-union row, or the arms do not differ: {post}");
    let pre_path = outdir.join("pre.r597dep.scan.json");
    std::fs::write(&pre_path, serde_json::to_string(&pre).unwrap()).unwrap();

    let app = make_crate("r597app", "pub fn run(s: &dyn r597dep::Sink) { s.emit(); }\n");
    std::fs::write(app.join("Cargo.toml"),
        "[package]\nname = \"r597app\"\n\n[dependencies]\nr597dep = \"0.1\"\n").unwrap();
    let policy = app.join("candor.policy");
    std::fs::write(&policy, "deny Net\n").unwrap();

    let run = |deps: &std::path::Path| -> i32 {
        Command::new(bin())
            .arg(app.to_string_lossy().as_ref()).arg("--json")
            .env("CANDOR_DEPS", deps.to_string_lossy().as_ref())
            .env("CANDOR_POLICY", policy.to_string_lossy().as_ref())
            .env_remove("CANDOR_CONFIG")
            .output().expect("run candor-scan").status.code().unwrap_or(-1)
    };
    let pre_code = run(&pre_path);
    let post_code = run(&rep_path);
    let _ = std::fs::remove_dir_all(&dep);
    let _ = std::fs::remove_dir_all(&app);

    assert_eq!(pre_code, 0,
        "CALIBRATION — without the beside-union row the consumer reads `Sink::emit` as the default body \
         alone and `deny Net` passes. If this ever fails, the arms no longer differ in the one thing \
         under test and the assertion below proves nothing.");
    assert_eq!(post_code, 1,
        "`deny Net` MUST fail: the consumer supplies `&dyn Sink`, the dependency's only override opens a \
         TcpStream, and obligation 3 says the member key carries every implementor the producer can see.");
}

/// SOUNDNESS R597, THE SILENCE CONTROL — **233 OF THE 244 SUPPRESSIONS WERE CORRECT AND MUST STAY
/// SILENT.** The fix emits a beside-union only where the union knows something the real row does not; a
/// blanket "always emit" would put a duplicate, informationless row on every trait member with a body.
/// The narrowing fails in the WITHHOLDING direction if it is ever wrong, which is why it is exact set
/// arithmetic on the published fields rather than a heuristic — and this is the arm that pins it.
#[test]
fn r597_a_union_that_adds_nothing_emits_no_second_row() {
    // The override and the default body do the SAME thing, so the union is a subset of the real row.
    let d = make_crate(
        "r597quiet",
        "pub trait Sink {\n\
         \x20   fn emit(&self) { let _ = std::fs::read(\"/tmp/a\"); }\n\
         }\n\
         pub struct Same;\n\
         impl Sink for Same { fn emit(&self) { let _ = std::fs::read(\"/tmp/b\"); } }\n",
    );
    let out = Command::new(bin())
        .arg(d.to_string_lossy().as_ref()).arg("--json")
        .env_remove("CANDOR_POLICY").env_remove("CANDOR_CONFIG").env_remove("CANDOR_DEPS")
        .output().expect("run candor-scan");
    let _ = std::fs::remove_dir_all(&d);
    let v: serde_json::Value =
        serde_json::from_str(String::from_utf8(out.stdout).unwrap().trim()).expect("pure JSON report");
    let fns = v["functions"].as_array().unwrap();
    // PROVE THE FIXTURE REACHES THE BRANCH: a real row must exist at the member hash, or this test is
    // asserting the absence of something that was never a candidate.
    assert!(fns.iter().any(|e| e["hash"] == "r597quiet#Sink::emit"
                             && e["interfaceUnion"] != serde_json::json!(true)),
        "the default body must be an analysed unit at the member hash, or there is no suppression to \
         control: {v}");
    assert!(!fns.iter().any(|e| e["hash"] == "r597quiet#Sink::emit"
                              && e["interfaceUnion"] == serde_json::json!(true)),
        "`Same::emit` performs `Fs` and so does the default body — the union adds nothing and must \
         publish nothing: {v}");
}

/// SOUNDNESS R597, THE COVERAGE LEG — **EMITTED NOW THAT R598 IS CLOSED.**
///
/// This test replaces `r597_the_coverage_leg_is_not_emitted_while_r598_is_open`, which was the gate on
/// that decision and said in its own failure message that re-enabling the leg deliberately means
/// deleting it. R598 is closed one function up in `scan.rs`, so the trade the leg was refused on no
/// longer exists: the union can no longer carry a fabricated effect from an inherent method beside its
/// `invisible`, and `invisible` itself arms no policy form (⟨0.30⟩'s non-gating ruling).
///
/// The fixture is the old one unchanged: `Loud::emit` performs the SAME effect as the default body and
/// adds only an UNCOVERED package, so nothing but the coverage channel can explain the row.
#[test]
fn r597_the_coverage_leg_is_emitted_now_that_r598_is_closed() {
    let d = make_crate(
        "r597blind",
        "pub trait Sink {\n\
         \x20   fn emit(&self) { let _ = std::fs::read(\"/tmp/a\"); }\n\
         }\n\
         pub struct Loud;\n\
         impl Sink for Loud {\n\
         \x20   fn emit(&self) { let _ = std::fs::read(\"/tmp/b\"); uncov::helper(); }\n\
         }\n",
    );
    std::fs::write(d.join("Cargo.toml"),
        "[package]\nname = \"r597blind\"\n\n[dependencies]\nuncov = \"1\"\n").unwrap();
    let out = Command::new(bin())
        .arg(d.to_string_lossy().as_ref()).arg("--json")
        .env_remove("CANDOR_POLICY").env_remove("CANDOR_CONFIG").env_remove("CANDOR_DEPS")
        .env_remove("CANDOR_R598_PRE")
        .output().expect("run candor-scan");
    let _ = std::fs::remove_dir_all(&d);
    let v: serde_json::Value =
        serde_json::from_str(String::from_utf8(out.stdout).unwrap().trim()).expect("pure JSON report");
    let fns = v["functions"].as_array().unwrap();

    // PROVE THE FIXTURE REACHES THE BRANCH, in BOTH of the two ways it has to.
    // 1. the override must genuinely be the coverage-only case: same effect as the default body, plus an
    //    uncovered package the default body does not touch.
    let over = fns.iter().find(|e| e["fn"] == "Loud::emit").unwrap_or_else(|| panic!("{v}"));
    assert_eq!(over["invisible"], serde_json::json!(["uncov"]),
        "the override must call into an UNCOVERED package, or there is no coverage leg to exercise: {v}");
    assert_eq!(over["inferred"], serde_json::json!(["Fs"]),
        "…and it must add NO effect the default body lacks, or this exercises the effects leg instead \
         and proves nothing about the coverage one: {v}");
    // 2. a real row must claim the member hash, or nothing was ever suppressed.
    assert!(fns.iter().any(|e| e["hash"] == "r597blind#Sink::emit"
                             && e["interfaceUnion"] != serde_json::json!(true)),
        "the default body must be an analysed unit at the member hash: {v}");

    let union: Vec<&serde_json::Value> = fns.iter()
        .filter(|e| e["hash"] == "r597blind#Sink::emit"
                 && e["interfaceUnion"] == serde_json::json!(true))
        .collect();
    assert_eq!(union.len(), 1,
        "THE COVERAGE LEG MUST NOW BE EMITTED. A consumer joining `Sink::emit` is told the implementor \
         set reaches a package this scan could not read; withholding it publishes the default body's \
         narrower coverage as if it were the member's: {v}");
    assert_eq!(union[0]["invisible"], serde_json::json!(["uncov"]),
        "…and the uncovered package is the whole reason the row exists: {v}");
    assert_eq!(union[0]["inferred"], serde_json::json!(["Fs"]),
        "the row carries the implementors' OWN effect and nothing invented — R598 was the reason this \
         leg was held back, because `{{ty}}::{{method}}` could pick up an inherent method's effects: {v}");
}

/// SOUNDNESS R598 — **THE UNDER-REPORT CONTROL, AND IT IS THE FIRST THING IN THIS FILE THAT WAS
/// WRITTEN.** A fabrication fix removes charges, which is where this family introduces silent
/// under-reports (4 defects in 5 such fixes, 2 of them cardinal sins), so the arm that must NOT move is
/// the arm to pin first: an implementor that really does DECLARE the member must still carry its effect
/// into the union, before and after the narrowing.
///
/// One fixture, one scan, two types differing in exactly one thing — whether `impl Enc for X` declares
/// `to_bytes`:
///   · `Loud`  DECLARES it (`Net`)                        → must contribute, in BOTH arms.
///   · `Quiet` does NOT, and has an INHERENT twin (`Fs`)   → must contribute in NEITHER, post-fix.
/// So the same assertion pair distinguishes the fix from a narrowing that went too far: lose `Net` and
/// the fix is a silent under-report; keep `Fs` and it never fired.
#[test]
fn r598_a_declared_override_still_carries_its_effect_into_the_union() {
    let d = make_crate(
        "r598decl",
        "pub trait Enc {\n\
         \x20   fn emit(&self);\n\
         \x20   fn to_bytes(&self) -> u8 { 7 }\n\
         }\n\
         pub struct Loud;\n\
         impl Enc for Loud {\n\
         \x20   fn emit(&self) {}\n\
         \x20   fn to_bytes(&self) -> u8 { let _ = std::net::TcpStream::connect(\"h:1\"); 1 }\n\
         }\n\
         pub struct Quiet;\n\
         impl Quiet {\n\
         \x20   fn to_bytes(&self) -> u8 { let _ = std::fs::read(\"/tmp/a\"); 2 }\n\
         \x20   pub fn local(&self) -> u8 { self.to_bytes() }\n\
         }\n\
         impl Enc for Quiet { fn emit(&self) {} }\n",
    );
    let out = Command::new(bin())
        .arg(d.to_string_lossy().as_ref()).arg("--json")
        .env_remove("CANDOR_POLICY").env_remove("CANDOR_CONFIG").env_remove("CANDOR_DEPS")
        .env_remove("CANDOR_R598_PRE")
        .output().expect("run candor-scan");
    let _ = std::fs::remove_dir_all(&d);
    let v: serde_json::Value =
        serde_json::from_str(String::from_utf8(out.stdout).unwrap().trim()).expect("pure JSON report");
    let fns = v["functions"].as_array().unwrap();

    // PROVE THE FIXTURE REACHES THE BRANCH: both `{ty}::to_bytes` units must EXIST as analysed rows, or
    // the union had nothing to pick up and this test cannot tell a fix from a typo.
    assert!(fns.iter().any(|e| e["hash"] == "r598decl#Loud::to_bytes"),
        "the DECLARED override must be an analysed unit: {v}");
    assert!(fns.iter().any(|e| e["hash"] == "r598decl#Quiet::to_bytes"),
        "the INHERENT twin must be an analysed unit, or there is no fabrication to remove: {v}");

    let union: Vec<&serde_json::Value> = fns.iter()
        .filter(|e| e["hash"] == "r598decl#Enc::to_bytes").collect();
    assert_eq!(union.len(), 1, "exactly one row at the member key: {v}");
    // ── THE UNDER-REPORT CONTROL ──────────────────────────────────────────────────────────────────
    assert!(union[0]["inferred"].as_array().unwrap().iter().any(|e| e == "Net"),
        "`Loud` DECLARES `to_bytes` and opens a socket — a dispatch through `Enc::to_bytes` really can \
         reach it. Losing this is the silent under-report a fabrication fix introduces, and it is worse \
         than the fabrication it removes: {v}");
    // ── THE FIX ───────────────────────────────────────────────────────────────────────────────────
    assert_eq!(union[0]["inferred"], serde_json::json!(["Net"]),
        "`Quiet::to_bytes` is an INHERENT method — `impl Enc for Quiet` does not override `to_bytes`, so \
         that dispatch runs the trait's default body and NOTHING can reach the `Fs`. Charging it here is \
         the hickory-proto fabrication: {v}");
    // …and the inherent method keeps its own row and its own effect. The fix withholds a CANDIDATE from
    // one union, it does not stop analysing a function.
    let inherent = fns.iter().find(|e| e["hash"] == "r598decl#Quiet::to_bytes").unwrap();
    assert_eq!(inherent["inferred"], serde_json::json!(["Fs"]), "{v}");
    let caller = fns.iter().find(|e| e["fn"] == "Quiet::local").unwrap_or_else(|| panic!("{v}"));
    assert_eq!(caller["inferred"], serde_json::json!(["Fs"]),
        "a caller that reaches the inherent method BY NAME is unaffected — this fix is about which \
         function a TRAIT MEMBER dispatches to, not about what the inherent one does: {v}");
}

/// SOUNDNESS R598 — **AN IMPL BLOCK THIS ENGINE CANNOT READ IS NOT EVIDENCE OF ABSENCE.**
///
/// The narrowing is a DENYLIST over `trait_impls`' sound over-approximation, and the hazard
/// ([[candor-denylist-over-allowlist]]) is that the index it narrows on is incomplete. The one
/// incompleteness inside a block that is real is a MACRO ITEM: `impl Enc for Opaque { gen!(); }` can
/// expand to the member, and `syn` reports it as `ImplItem::Macro` with no names in it.
///
/// So `collect_local_impl_members` writes an OPAQUE marker for such a block and `impl_does_not_declare`
/// refuses to narrow on it — the pre-existing over-approximation stands. **This fixture makes that the
/// SOUND answer rather than merely the conservative one:** the macro really does generate
/// `to_bytes`, and it really does read a file, so a dispatch through `Enc::to_bytes` on an `Opaque`
/// really does perform `Fs`. The engine cannot see that body; it can see the inherent twin's `Fs`, and
/// charging it is right for the wrong reason, which is what an over-approximation is for.
///
/// **Delete the opaque marker and this test goes RED with a silent under-report**, which is the only
/// thing that makes it a guard rather than a comment.
#[test]
fn r598_an_impl_block_this_engine_cannot_read_is_not_narrowed() {
    let d = make_crate(
        "r598opaque",
        "#[macro_export]\n\
         macro_rules! gen_to_bytes {\n\
         \x20   () => { fn to_bytes(&self) -> u8 { let _ = std::fs::read(\"/tmp/m\"); 3 } };\n\
         }\n\
         pub trait Enc {\n\
         \x20   fn emit(&self);\n\
         \x20   fn to_bytes(&self) -> u8 { 7 }\n\
         }\n\
         pub struct Opaque;\n\
         impl Opaque {\n\
         \x20   fn to_bytes(&self) -> u8 { let _ = std::fs::read(\"/tmp/a\"); 2 }\n\
         \x20   pub fn local(&self) -> u8 { self.to_bytes() }\n\
         }\n\
         impl Enc for Opaque {\n\
         \x20   fn emit(&self) {}\n\
         \x20   gen_to_bytes!();\n\
         }\n",
    );
    let out = Command::new(bin())
        .arg(d.to_string_lossy().as_ref()).arg("--json")
        .env_remove("CANDOR_POLICY").env_remove("CANDOR_CONFIG").env_remove("CANDOR_DEPS")
        .env_remove("CANDOR_R598_PRE")
        .output().expect("run candor-scan");
    let _ = std::fs::remove_dir_all(&d);
    let v: serde_json::Value =
        serde_json::from_str(String::from_utf8(out.stdout).unwrap().trim()).expect("pure JSON report");
    let fns = v["functions"].as_array().unwrap();
    let union: Vec<&serde_json::Value> = fns.iter()
        .filter(|e| e["hash"] == "r598opaque#Enc::to_bytes").collect();
    assert_eq!(union.len(), 1,
        "`impl Enc for Opaque` holds a MACRO ITEM, so its member list is not evidence that `to_bytes` \
         is unimplemented. Narrowing here would withhold the `Fs` that the macro-generated override \
         really performs — a silent under-report bought with an allowlist over an index known to be \
         incomplete: {v}");
    assert_eq!(union[0]["inferred"], serde_json::json!(["Fs"]), "{v}");
}

/// SOUNDNESS R598 — **THE CONSUMER SIDE, WHERE THE FABRICATION IS GATE-FLIPPABLE.**
///
/// One tree, one consumer, one dep report; the ONLY variable is `CANDOR_R598_PRE`, which restores the
/// pre-fix `{ty}::{method}` lookup in the producer. `deny Log` over a `&dyn Enc` dispatch goes exit
/// **1 → 0**, and the exit 0 is the correct answer: `impl Enc for Quiet` does not override `to_bytes`,
/// so the `warn!` inside the private inherent `Quiet::to_bytes` is unreachable through that member.
///
/// **THE CALIBRATION IS THE SECOND MEMBER**, and without it this test would pass for a producer that
/// had simply stopped publishing union rows: `shout` IS overridden, it IS effectful, and `deny Log`
/// must fire on it in BOTH arms.
#[test]
fn r598_a_chained_consumer_stops_reading_an_inherent_methods_effect() {
    let dep = make_crate(
        "r598dep",
        "pub trait Enc {\n\
         \x20   fn emit(&self);\n\
         \x20   fn to_bytes(&self) -> u8 { 7 }\n\
         \x20   fn shout(&self) {}\n\
         }\n\
         pub struct Quiet;\n\
         impl Quiet {\n\
         \x20   fn to_bytes(&self) -> u8 { log::warn!(\"unreachable through the member\"); 2 }\n\
         \x20   pub fn local(&self) -> u8 { self.to_bytes() }\n\
         }\n\
         impl Enc for Quiet {\n\
         \x20   fn emit(&self) {}\n\
         \x20   fn shout(&self) { log::error!(\"reachable through the member\"); }\n\
         }\n",
    );
    std::fs::write(dep.join("Cargo.toml"),
        "[package]\nname = \"r598dep\"\n\n[dependencies]\nlog = \"0.4\"\n").unwrap();
    let outdir = dep.join("rep");
    std::fs::create_dir_all(&outdir).unwrap();
    let produce = |name: &str, pre: bool| -> std::path::PathBuf {
        let mut c = Command::new(bin());
        c.arg(dep.to_string_lossy().as_ref())
            .arg("--out").arg(outdir.join(name).to_string_lossy().as_ref())
            .env_remove("CANDOR_POLICY").env_remove("CANDOR_CONFIG").env_remove("CANDOR_DEPS")
            .env_remove("CANDOR_R598_PRE");
        if pre {
            c.env("CANDOR_R598_PRE", "1");
        }
        let st = c.output().expect("run candor-scan");
        assert!(st.status.success(), "producing the dep report must succeed: {}",
            String::from_utf8_lossy(&st.stderr));
        outdir.join(format!("{name}.r598dep.scan.json"))
    };
    let pre_rep = produce("pre", true);
    let post_rep = produce("post", false);

    // The two reports MUST differ in the one row under test, or the arms below are one document.
    let member_effects = |p: &std::path::Path, member: &str| -> Vec<String> {
        let v: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(p).unwrap()).unwrap();
        v["functions"].as_array().unwrap().iter()
            .filter(|e| e["hash"] == format!("r598dep#Enc::{member}"))
            .flat_map(|e| e["inferred"].as_array().cloned().unwrap_or_default())
            .map(|x| x.as_str().unwrap().to_string())
            .collect()
    };
    assert_eq!(member_effects(&pre_rep, "to_bytes"), vec!["Log".to_string()],
        "CALIBRATION — the PRE arm must publish the fabricated `Log` at the member key, or there is \
         nothing for the fix to remove and both exits below would agree for the wrong reason");
    assert!(member_effects(&post_rep, "to_bytes").is_empty(),
        "the POST arm must not publish it");
    assert_eq!(member_effects(&post_rep, "shout"), vec!["Log".to_string()],
        "CALIBRATION — the DECLARED override's `Log` must survive in the POST arm, or the producer has \
         simply stopped publishing union rows and the exit codes below say nothing about R598");

    let app = make_crate("r598app", "pub fn run(e: &dyn r598dep::Enc) { let _ = e.to_bytes(); }\n");
    std::fs::write(app.join("Cargo.toml"),
        "[package]\nname = \"r598app\"\n\n[dependencies]\nr598dep = \"0.1\"\n").unwrap();
    let shouter = make_crate("r598shout", "pub fn run(e: &dyn r598dep::Enc) { e.shout(); }\n");
    std::fs::write(shouter.join("Cargo.toml"),
        "[package]\nname = \"r598shout\"\n\n[dependencies]\nr598dep = \"0.1\"\n").unwrap();
    let policy = app.join("candor.policy");
    std::fs::write(&policy, "deny Log\n").unwrap();

    let run = |crate_dir: &std::path::Path, deps: &std::path::Path| -> i32 {
        Command::new(bin())
            .arg(crate_dir.to_string_lossy().as_ref()).arg("--json")
            .env("CANDOR_DEPS", deps.to_string_lossy().as_ref())
            .env("CANDOR_POLICY", policy.to_string_lossy().as_ref())
            .env_remove("CANDOR_CONFIG").env_remove("CANDOR_R598_PRE")
            .output().expect("run candor-scan").status.code().unwrap_or(-1)
    };
    let pre_code = run(&app, &pre_rep);
    let post_code = run(&app, &post_rep);
    let calib_pre = run(&shouter, &pre_rep);
    let calib_post = run(&shouter, &post_rep);
    let _ = std::fs::remove_dir_all(&dep);
    let _ = std::fs::remove_dir_all(&app);
    let _ = std::fs::remove_dir_all(&shouter);

    assert_eq!(pre_code, 1,
        "CALIBRATION — the fabricated `Log` really did flip a gate; `invisible` could not have, which \
         is why this row blocked R597's coverage leg");
    assert_eq!(post_code, 0,
        "`impl Enc for Quiet` does not override `to_bytes`, so the dispatch runs the trait's default \
         body and the `warn!` in the private inherent `Quiet::to_bytes` is unreachable. Exit 0 is the \
         TRUE answer here, not a suppressed one");
    assert_eq!((calib_pre, calib_post), (1, 1),
        "the OVERRIDDEN member's `Log` must still fire in both arms — a producer that published no \
         union rows at all would pass the assertion above and fail this one");
}

/// SOUNDNESS R598 — **THE NARROWING WITHHOLDS A CLAIM AND NEVER A DISCLOSURE, AND BOTH HALVES OF THAT
/// SENTENCE COST A MEASUREMENT.**
///
/// The first form of this fix dropped a non-declaring implementor's candidate outright. Over the
/// 1,626-crate cargo-registry corpus that removed 14 keys, and the FULL audit of all fourteen — not a
/// sample — found NINE where the effect really is reachable through the member, because the trait's
/// DEFAULT BODY delegates to a same-named method on the receiver and the resolver drops that call:
/// `snapbox#data::IntoData::is` (`self.into_data().is(format)` → the inherent `Data::is`) and
/// `diesel#query_dsl::QueryDsl::{limit,offset,having,find}` (`methods::LimitDsl::limit(self, limit)` →
/// `CombinationClause::limit`). Nine keys went from a disclosure to silence: a fabrication (an
/// over-report) traded for the cardinal sin.
///
/// So the narrowing gained two guards, and this fixture is three traits differing in EXACTLY ONE THING
/// each — what the member's default body does — over one implementing type, one scan:
///
///   · `IntoData::is`   default body NAMES `is`      → **not narrowed at all** (delegation hedge).
///   · `Hedged::opaque` default body names nothing    → narrowed, but its `Unknown` RIDES THROUGH.
///   · `Plain::size`    default body names nothing    → narrowed, and `Fs` alone means no row at all.
///
/// The third is the hickory-proto shape and the reason the row exists; the first two are the reason it
/// is not a `continue`.
#[test]
fn r598_the_narrowing_withholds_a_claim_but_never_a_disclosure() {
    let d = make_crate(
        "r598three",
        "pub trait IntoData {\n\
         \x20   fn into_data(self) -> Data;\n\
         \x20   fn is(self) -> Data where Self: Sized { self.into_data().is() }\n\
         }\n\
         pub trait Hedged {\n\
         \x20   fn h(&self) -> u8;\n\
         \x20   fn opaque(&self) -> u8 { 7 }\n\
         }\n\
         pub trait Plain {\n\
         \x20   fn p(&self) -> u8;\n\
         \x20   fn size(&self) -> u8 { 7 }\n\
         }\n\
         pub struct Data { pub cb: fn() -> u8 }\n\
         impl Data {\n\
         \x20   pub fn is(self) -> Data { let _ = std::fs::read(\"/tmp/a\"); (self.cb)(); self }\n\
         \x20   pub fn opaque(&self) -> u8 { let _ = std::fs::read(\"/tmp/b\"); (self.cb)() }\n\
         \x20   pub fn size(&self) -> u8 { let _ = std::fs::read(\"/tmp/c\"); 1 }\n\
         }\n\
         impl IntoData for Data { fn into_data(self) -> Data { self } }\n\
         impl Hedged for Data { fn h(&self) -> u8 { 0 } }\n\
         impl Plain for Data { fn p(&self) -> u8 { 0 } }\n",
    );
    let out = Command::new(bin())
        .arg(d.to_string_lossy().as_ref()).arg("--json")
        .env_remove("CANDOR_POLICY").env_remove("CANDOR_CONFIG").env_remove("CANDOR_DEPS")
        .env_remove("CANDOR_R598_PRE")
        .output().expect("run candor-scan");
    let _ = std::fs::remove_dir_all(&d);
    let v: serde_json::Value =
        serde_json::from_str(String::from_utf8(out.stdout).unwrap().trim()).expect("pure JSON report");
    let fns = v["functions"].as_array().unwrap();
    let row = |h: &str, union: bool| -> Option<&serde_json::Value> {
        fns.iter().find(|e| e["hash"] == h
            && (e["interfaceUnion"] == serde_json::json!(true)) == union)
    };

    // PROVE THE FIXTURE REACHES THE BRANCH. All three inherent methods must be analysed units with the
    // effects the arms below reason about, and NONE of the three `impl … for Data` blocks declares the
    // member — otherwise nothing here is narrowable and every assertion passes vacuously.
    assert_eq!(row("r598three#Data::is", false).unwrap_or_else(|| panic!("{v}"))["inferred"],
        serde_json::json!(["Fs", "Unknown"]), "{v}");
    assert_eq!(row("r598three#Data::opaque", false).unwrap_or_else(|| panic!("{v}"))["inferred"],
        serde_json::json!(["Fs", "Unknown"]), "{v}");
    assert_eq!(row("r598three#Data::size", false).unwrap_or_else(|| panic!("{v}"))["inferred"],
        serde_json::json!(["Fs"]), "{v}");

    // ── 1. THE DELEGATION HEDGE — the snapbox/diesel shape, and the arm that must NOT move ─────────
    assert_eq!(row("r598three#IntoData::is", true).unwrap_or_else(|| panic!("{v}"))["inferred"],
        serde_json::json!(["Fs", "Unknown"]),
        "`IntoData::is`'s default body is `self.into_data().is(…)`, which really does reach the inherent \
         `Data::is` — the engine does not resolve that call, so withholding the union's copy leaves the \
         member key SILENT about a file read. Nine of the fourteen keys the unqualified drop removed \
         were this, on snapbox and diesel: {v}");

    // ── 2. THE CHANNEL RULE — narrowed, and the disclosure still rides ─────────────────────────────
    assert_eq!(row("r598three#Hedged::opaque", true).unwrap_or_else(|| panic!("{v}"))["inferred"],
        serde_json::json!(["Unknown"]),
        "a narrowed candidate's `Unknown` is the engine saying what it does not know, and withholding \
         THAT converts 'we cannot see this' into 'there is nothing here'. The concrete `Fs` is a claim \
         about a function no dispatch through this member reaches, and it is the half that goes: {v}");

    // ── 3. THE FABRICATION ITSELF — hickory-proto's shape, no CONCRETE claim left to publish ───────
    //
    // ⟨0.40⟩/R609 MOVED THE SPELLING OF THIS ARM, NOT ITS TEETH. It used to read `is_none()`: with
    // nothing left in the union the producer dropped the row entirely. R609 publishes a pure-only
    // union instead, because a dropped row and a never-existing one are the same bytes on the wire and
    // R608's conjunct 3 has to tell them apart. What this arm tests is unchanged and the
    // discrimination is EXACTLY as sharp — calibrated on this same fixture with `CANDOR_R598_PRE=1`,
    // which restores the pre-R598 lookup and makes this row read `["Fs"]`.
    let plain = row("r598three#Plain::size", true).unwrap_or_else(|| panic!(
        "R609 publishes the union even when it is pure — the row must EXIST, or this arm is asserting \
         the absence of a leg that never ran: {v}"));
    assert_eq!(plain["inferred"], serde_json::json!([]),
        "`impl Plain for Data` does not declare `size` and the default body is `{{ 7 }}` — the inherent \
         `Data::size`'s `Fs` is unreachable through this member and there is no disclosure riding with \
         it, so the key must publish NO CONCRETE EFFECT. `[]` here is a true statement about the union \
         (the only implementor runs the trait's pure default); a concrete Fs would be the \
         fabrication: {v}");
}

/// SOUNDNESS R598, THE TWO ASSERTIONS IN THE DIFF THAT `assert-audit.sh` FLAGS — **MADE INTO ARMS
/// INSTEAD OF SENTENCES.** §E2: a documented guarantee closes the question AND licenses narrowing a
/// sound over-approximation, so it converts straight into a silent under-report if it is wrong. Both
/// were true when written; neither had a fixture.
///
/// 1. *"An associated const or type cannot introduce a METHOD, so neither blinds the member list."* —
///    if that arm of `collect_local_impl_members` tainted instead, the narrowing would silently stop
///    firing for the commonest impl shape in the ecosystem (`type Future = …`), and the A/B that
///    priced this change would have measured almost nothing.
/// 2. *"R598 CANNOT REACH [the foreign] LEG AT ALL"* — `collect_foreign_trait_impls` keys ONE ENTRY PER
///    DECLARED MEMBER, so a member the impl does not write has no candidate and no key. Asserted as an
///    ABSENCE, which is also what a leg that simply did not run would produce — so the arm that proves
///    the branch was reached (the DECLARED member's key IS published) comes first.
#[test]
fn r598_an_assoc_item_does_not_blind_the_member_list_and_the_foreign_leg_is_immune() {
    // The local half: an impl block carrying an associated CONST and an associated TYPE beside the one
    // method it declares must still be readable evidence, so `size` is still narrowed.
    let d = make_crate(
        "r598assoc",
        "pub trait Enc {\n\
         \x20   type Out;\n\
         \x20   const TAG: u8;\n\
         \x20   fn emit(&self) -> Self::Out;\n\
         \x20   fn size(&self) -> u8 { 7 }\n\
         }\n\
         pub struct RData;\n\
         impl RData {\n\
         \x20   fn size(&self) -> u8 { let _ = std::fs::read(\"/tmp/a\"); 2 }\n\
         \x20   pub fn local(&self) -> u8 { self.size() }\n\
         }\n\
         impl Enc for RData {\n\
         \x20   type Out = ();\n\
         \x20   const TAG: u8 = 3;\n\
         \x20   fn emit(&self) -> Self::Out {}\n\
         }\n",
    );
    let out = Command::new(bin())
        .arg(d.to_string_lossy().as_ref()).arg("--json")
        .env_remove("CANDOR_POLICY").env_remove("CANDOR_CONFIG").env_remove("CANDOR_DEPS")
        .env_remove("CANDOR_R598_PRE")
        .output().expect("run candor-scan");
    let _ = std::fs::remove_dir_all(&d);
    let v: serde_json::Value =
        serde_json::from_str(String::from_utf8(out.stdout).unwrap().trim()).expect("pure JSON report");
    let fns = v["functions"].as_array().unwrap();
    assert!(fns.iter().any(|e| e["hash"] == "r598assoc#RData::size"),
        "the inherent method must be an analysed unit, or nothing is narrowable here: {v}");
    // ⟨0.40⟩/R609: the row is now PUBLISHED with an empty union rather than dropped (see the
    // `r598three#Plain::size` arm for why), so the assertion moved from "no row" to "no concrete
    // effect". Same calibration, same teeth: under `CANDOR_R598_PRE=1` this row reads `["Fs"]`.
    let encsize = fns.iter().find(|e| e["hash"] == "r598assoc#Enc::size")
        .unwrap_or_else(|| panic!("R609 publishes the pure union, so the row must exist: {v}"));
    assert_eq!(encsize["inferred"], serde_json::json!([]),
        "an associated `type`/`const` beside the declared method must not blind the member list — if it \
         did, the narrowing would stop firing for the commonest impl shape in the ecosystem and the \
         inherent `RData::size`'s `Fs` would be published at the trait member key: {v}");

    // The FOREIGN half (⟨0.39⟩ obligation 2): the same source shape, but the trait belongs to a
    // dependency. `collect_foreign_trait_impls` keys one entry per DECLARED member.
    let f = make_crate(
        "r598foreign",
        "pub struct RData;\n\
         impl RData {\n\
         \x20   fn to_bytes(&self) -> u8 { let _ = std::fs::read(\"/tmp/a\"); 2 }\n\
         \x20   pub fn local(&self) -> u8 { self.to_bytes() }\n\
         }\n\
         impl iface::Enc for RData {\n\
         \x20   fn emit(&self) { let _ = std::net::TcpStream::connect(\"h:1\"); }\n\
         }\n",
    );
    std::fs::write(f.join("Cargo.toml"),
        "[package]\nname = \"r598foreign\"\n\n[dependencies]\niface = \"1\"\n").unwrap();
    let out = Command::new(bin())
        .arg(f.to_string_lossy().as_ref()).arg("--json")
        .env_remove("CANDOR_POLICY").env_remove("CANDOR_CONFIG").env_remove("CANDOR_DEPS")
        .env_remove("CANDOR_R598_PRE")
        .output().expect("run candor-scan");
    let _ = std::fs::remove_dir_all(&f);
    let v: serde_json::Value =
        serde_json::from_str(String::from_utf8(out.stdout).unwrap().trim()).expect("pure JSON report");
    let fns = v["functions"].as_array().unwrap();
    // REACH FIRST: the DECLARED member's foreign key must be published, or the absence below is the
    // absence of a leg that never ran.
    let emit = fns.iter().find(|e| e["hash"] == "iface#Enc::emit")
        .unwrap_or_else(|| panic!("obligation 2's foreign union entry must be published: {v}"));
    assert_eq!(emit["inferred"], serde_json::json!(["Net"]), "{v}");
    assert!(!fns.iter().any(|e| e["hash"] == "iface#Enc::to_bytes"),
        "the foreign leg publishes one entry PER DECLARED MEMBER, so an inherent `RData::to_bytes` can \
         never be read as `iface::Enc`'s implementation of a member the impl does not write. If this \
         ever fails, R598 has a second site and the narrowing above must be taught to it: {v}");
}

// ═══ ⟨0.40⟩ SOUNDNESS R608 / R609 — THE CHAINED ABSTRACTION WITH AN EMPTY IMPLEMENTOR UNION ═══════
//
// One helper, because every arm below needs a dep report on disk and a consumer chained onto it, and
// the ONE thing that may vary between arms is the dependency's source.
fn r608_dep_report(name: &str, src: &str) -> (PathBuf, PathBuf) {
    let dep = make_crate(name, src);
    let out = dep.join("rep");
    std::fs::create_dir_all(&out).unwrap();
    let st = Command::new(bin())
        .arg(dep.to_string_lossy().as_ref())
        .arg("--out").arg(out.join("r").to_string_lossy().as_ref())
        .env_remove("CANDOR_POLICY").env_remove("CANDOR_CONFIG").env_remove("CANDOR_DEPS")
        .output().expect("run candor-scan");
    assert!(st.status.success(), "producing the dep report must succeed: {}",
        String::from_utf8_lossy(&st.stderr));
    let rep = out.join(format!("r.{name}.scan.json"));
    (dep, rep)
}

fn r608_consumer(name: &str, dep: &str, src: &str, policy: &str) -> (PathBuf, PathBuf) {
    let app = make_crate(name, src);
    std::fs::write(app.join("Cargo.toml"),
        format!("[package]\nname = \"{name}\"\n\n[dependencies]\n{dep} = \"0.1\"\n")).unwrap();
    let pol = app.join("candor.policy");
    std::fs::write(&pol, policy).unwrap();
    (app, pol)
}

fn r608_run(app: &std::path::Path, pol: &std::path::Path, deps: Option<&std::path::Path>)
    -> (i32, serde_json::Value)
{
    let mut c = Command::new(bin());
    c.arg(app.to_string_lossy().as_ref()).arg("--json")
        .env("CANDOR_POLICY", pol.to_string_lossy().as_ref())
        .env_remove("CANDOR_CONFIG");
    match deps {
        Some(d) => { c.env("CANDOR_DEPS", d.to_string_lossy().as_ref()); }
        None => { c.env_remove("CANDOR_DEPS"); }
    }
    let out = c.output().expect("run candor-scan");
    let v: serde_json::Value =
        serde_json::from_str(String::from_utf8(out.stdout).unwrap().trim()).expect("pure JSON report");
    (out.status.code().unwrap_or(-1), v)
}

/// ⟨0.40⟩ SOUNDNESS R608 — **A DISPATCH NOBODY IMPLEMENTS MUST NOT READ AS PURE**, and the arm that
/// makes it a measurement rather than an assertion is the one BESIDE it: PART 92's `c3_pure_only`.
///
/// Both arms share a byte-identical consumer (`pub fn run(h: &dyn dep::Handler) { h.handle(); }`), the
/// same policy and the same chaining. The ONLY variable is whether the dependency declares an
/// implementor. `zero` must disclose; `pure` must stay pure, because a library whose only implementor
/// anywhere is pure is LEGITIMATELY pure and hedging it is a fabrication.
///
/// **THE `pure` ARM IS WHY BOTH HALVES OF THIS RUNG SHIP TOGETHER.** Measured on a binary carrying the
/// consumer rule ALONE (R609's producer publication reverted): `apppure` exits **1** with
/// `inferred: ['Unknown']` — the producer's `silence = purity` drop made "no implementor" and "every
/// implementor is pure" the same bytes on the wire, so the consumer could not tell them apart and
/// charged both. With R609's publication in place the dependency emits
/// `puredep#Handler::handle  inferred: []  interfaceUnion: true` and this arm is pure again.
#[test]
fn r608_a_chained_abstraction_with_no_implementor_anywhere_reads_unknown() {
    let (zdep, zrep) = r608_dep_report("r608zero", "pub trait Handler { fn handle(&self); }\n");
    let (pdep, prep) = r608_dep_report("r608pure",
        "pub trait Handler { fn handle(&self); }\n\
         pub struct P;\n\
         impl Handler for P { fn handle(&self) { let _ = 1 + 1; } }\n");

    let (zapp, zpol) = r608_consumer("r608zeroapp", "r608zero",
        "pub fn run(h: &dyn r608zero::Handler) { h.handle(); }\n", "deny Net Unknown\n");
    let (papp, ppol) = r608_consumer("r608pureapp", "r608pure",
        "pub fn run(h: &dyn r608pure::Handler) { h.handle(); }\n", "deny Net Unknown\n");

    let (zcode, zv) = r608_run(&zapp, &zpol, Some(&zrep));
    let (pcode, pv) = r608_run(&papp, &ppol, Some(&prep));
    for d in [&zdep, &pdep, &zapp, &papp] { let _ = std::fs::remove_dir_all(d); }

    // ── CONTROL FIRST (PART 92 `c3_pure_only`) ────────────────────────────────────────────────────
    let prun = pv["functions"].as_array().unwrap().iter().find(|e| e["fn"] == "run")
        .unwrap_or_else(|| panic!("the consumer must be judged at all: {pv}"));
    assert_eq!(prun["inferred"], serde_json::json!([]),
        "a library whose ONLY implementor anywhere is pure is legitimately pure — hedging it is a \
         fabrication, and it is the failure mode the consumer rule has on its own: {pv}");
    assert!(prun["unknownWhy"].is_null(), "{pv}");
    assert_eq!(pcode, 0, "`deny Net Unknown` must PASS over an all-pure implementor union: {pv}");

    // ── THE ROW ───────────────────────────────────────────────────────────────────────────────────
    let zrun = zv["functions"].as_array().unwrap().iter().find(|e| e["fn"] == "run")
        .unwrap_or_else(|| panic!("{zv}"));
    assert_eq!(zrun["inferred"], serde_json::json!(["Unknown"]),
        "NOTHING implements `Handler` — not the dependency, not this crate. The engine publishes \
         `dispatchesOn` for exactly this key, so it KNOWS a dispatch happened, and reading `[]` here is \
         the ⟨0.39⟩ purity claim in its barest form: {zv}");
    assert_eq!(zrun["unknownWhy"], serde_json::json!(["dispatch:Handler.handle"]),
        "§4's normative detail for an unresolvable dispatch target, so `deny E Unknown[dispatch]` can \
         class it: {zv}");
    assert_eq!(zcode, 1,
        "`deny Net Unknown` MUST fail. Before ⟨0.40⟩ this exited 0 — that is the cardinal sin this rung \
         closes, and the `pure` arm above is what proves the two are distinguishable: {zv}");
}

/// ⟨0.40⟩ SOUNDNESS R608 — **CONJUNCT 4 IS ASKED OF THE ABSTRACTION, NEVER OF THE MEMBER**, which is
/// the single thing the port of java's rule can get wrong.
///
/// java's `chaTargets(owner, name, desc).isEmpty()` is also spelled per-member; what makes it behave as
/// an abstraction query is that its `name`+`desc` come off an `INVOKEINTERFACE`, verifier-guaranteed to
/// name a member the interface DECLARES. This engine mints its key from SOURCE, where nothing guarantees
/// that, so a member-keyed conjunct 4 cannot fire on a malformed key and charges `Unknown` for a
/// dispatch that does not exist ([[R549]]'s class).
///
/// The fixture is the smallest well-formed case that separates them: the consumer DOES implement the
/// dependency's abstraction, but its `impl` block writes only `a`, so `r608two#Handler::b` is not a key
/// in `foreign_impls` while the prefix `r608two#Handler::` is. The implementor union for `Handler` is
/// `{Mine}` and `Mine::b` runs the trait's own pure default, so the honest answer is PURE.
///
/// **CALIBRATED, not assumed.** Built with conjunct 4 spelled member-keyed (the mis-port), same tree,
/// same dep report, this arm goes RED exactly here:
///   `('r608twoapp#run', ['Unknown'], ['dispatch:Handler.b'])` / `R533HIT run r608two#Handler::b`
/// Measured over six chained crate pairs, Σ 2,826 analysed: member-keyed fires on 45 functions, of
/// which 38 sit on keys R549 proved malformed; trait-prefix-keyed fires on 6.
#[test]
fn r608_conjunct_4_is_asked_of_the_abstraction_not_of_the_member() {
    let (dep, rep) = r608_dep_report("r608two",
        "pub trait Handler { fn a(&self); fn b(&self) { let _ = 1 + 1; } }\n");
    let (app, pol) = r608_consumer("r608twoapp", "r608two",
        "pub struct Mine;\n\
         impl r608two::Handler for Mine { fn a(&self) { let _ = 1 + 1; } }\n\
         pub fn run(h: &dyn r608two::Handler) { h.b(); }\n", "deny Net Unknown\n");
    let (code, v) = r608_run(&app, &pol, Some(&rep));
    let _ = std::fs::remove_dir_all(&dep);
    let _ = std::fs::remove_dir_all(&app);

    // REACH: the consumer's own `impl` must have reached `foreign_impls`, or this asserts the absence
    // of a hedge that conjunct 4 was never consulted about.
    assert!(v["functions"].as_array().unwrap().iter()
            .any(|e| e["hash"] == "r608two#Handler::a" && e["interfaceUnion"] == serde_json::json!(true)),
        "obligation 2 must have published the consumer's foreign implementor under the owning crate's \
         key, which is the index conjunct 4 reads: {v}");

    let run = v["functions"].as_array().unwrap().iter().find(|e| e["fn"] == "run")
        .unwrap_or_else(|| panic!("{v}"));
    assert_eq!(run["inferred"], serde_json::json!([]),
        "this crate DOES implement `Handler`; `b` is simply not a member its `impl` block writes. The \
         abstraction is answered, so there is nothing to hedge — and a member-keyed conjunct 4 hedges \
         here, which is what makes it the mis-port: {v}");
    assert_eq!(code, 0, "{v}");
}

/// ⟨0.40⟩ SOUNDNESS R608, THE TWO NARROWING CONTROLS — an UNCHAINED dependency and a conventionally-pure
/// leaf. Both are `continue`s in the rule, and a `continue` nothing exercises is a comment.
#[test]
fn r608_an_unchained_dep_and_a_conventionally_pure_leaf_are_not_hedged() {
    // (a) UNCHAINED — conjunct 2. The crate is not covered at all, so the honest channel is coverage
    //     (`invisible`), which arms no policy form. Reading a dispatch hedge into it would charge every
    //     consumer of every un-scanned library.
    let (dep, _rep) = r608_dep_report("r608un", "pub trait Handler { fn handle(&self); }\n");
    let (app, pol) = r608_consumer("r608unapp", "r608un",
        "pub fn run(h: &dyn r608un::Handler) { h.handle(); }\n", "deny Net Unknown\n");
    let (code, v) = r608_run(&app, &pol, None);
    let run = v["functions"].as_array().unwrap().iter().find(|e| e["fn"] == "run")
        .unwrap_or_else(|| panic!("{v}"));
    assert_eq!(run["inferred"], serde_json::json!([]), "{v}");
    assert_eq!(run["invisible"], serde_json::json!(["r608un"]),
        "the unchained answer is the COVERAGE disclosure, unchanged by this rung: {v}");
    assert_eq!(code, 0, "{v}");
    let _ = std::fs::remove_dir_all(&dep);
    let _ = std::fs::remove_dir_all(&app);

    // (b) A CONVENTIONALLY-PURE LEAF — SPEC §4's permitted exclusion, here with a zero implementor union
    //     so that EVERY other conjunct passes and the exempt list is the only thing left deciding.
    let (fdep, frep) = r608_dep_report("r608fmt", "pub trait Shower { fn fmt(&self); }\n");
    let (fapp, fpol) = r608_consumer("r608fmtapp", "r608fmt",
        "pub fn run(s: &dyn r608fmt::Shower) { s.fmt(); }\n", "deny Net Unknown\n");
    let (fcode, fv) = r608_run(&fapp, &fpol, Some(&frep));
    let _ = std::fs::remove_dir_all(&fdep);
    let _ = std::fs::remove_dir_all(&fapp);
    let frun = fv["functions"].as_array().unwrap().iter().find(|e| e["fn"] == "run")
        .unwrap_or_else(|| panic!("{fv}"));
    assert_eq!(frun["inferred"], serde_json::json!([]),
        "`fmt` is in the exempt leaf set; `call` deliberately is NOT, because in rust it is a real \
         effectful trait method (`tower_service#Service::call`): {fv}");
    assert_eq!(fcode, 0, "{fv}");
}

/// ⟨0.40⟩ SOUNDNESS R609 — **A PURE-ONLY UNION IS PUBLISHED; A ZERO-IMPLEMENTOR ONE IS NOT.** That
/// distinction is the whole format half: it is what lets a consumer tell "every implementor is pure"
/// from "nothing implements this", which `silence = purity` made the same bytes.
///
/// The zero arm is not a nicety — R608's conjunct 3 reads WIRE ABSENCE, so if this leg published a row
/// for an abstraction with no implementor it would silence the rung it exists to serve.
#[test]
fn r609_a_pure_only_union_is_published_and_a_zero_implementor_one_is_not() {
    let run1 = |name: &str, src: &str| -> serde_json::Value {
        let d = make_crate(name, src);
        let out = Command::new(bin())
            .arg(d.to_string_lossy().as_ref()).arg("--json")
            .env_remove("CANDOR_POLICY").env_remove("CANDOR_CONFIG").env_remove("CANDOR_DEPS")
            .output().expect("run candor-scan");
        let _ = std::fs::remove_dir_all(&d);
        serde_json::from_str(String::from_utf8(out.stdout).unwrap().trim()).expect("pure JSON report")
    };

    // (a) PURE-ONLY — published, with `inferred: []` and no `loc` (it is synthetic, not a body).
    let p = run1("r609pure",
        "pub trait Handler { fn handle(&self); }\n\
         pub struct P;\n\
         impl Handler for P { fn handle(&self) { let _ = 1 + 1; } }\n");
    let u: Vec<&serde_json::Value> = p["functions"].as_array().unwrap().iter()
        .filter(|e| e["hash"] == "r609pure#Handler::handle").collect();
    assert_eq!(u.len(), 1,
        "the union over `Handler`'s implementors must be PUBLISHED even when every one is pure. \
         Dropping it is what made absence ambiguous on the wire: {p}");
    assert_eq!(u[0]["interfaceUnion"], serde_json::json!(true), "{p}");
    assert_eq!(u[0]["inferred"], serde_json::json!([]), "{p}");
    assert!(u[0]["loc"].is_null(), "a synthetic union row has no body and must carry no `loc`: {p}");

    // (b) ZERO IMPLEMENTORS — nothing published, because there is no union to publish.
    let z = run1("r609zero", "pub trait Handler { fn handle(&self); }\n");
    assert!(!z["functions"].as_array().unwrap().iter()
             .any(|e| e["hash"] == "r609zero#Handler::handle"),
        "an abstraction NOBODY implements must publish no union row — R608's conjunct 3 reads exactly \
         this absence, and a row here would silence the rung: {z}");
    // WHICH MECHANISM DELIVERS THAT, stated so this arm is not read as coverage of the
    // `impls.is_empty()` conjunct beside the publication: it is `trait_impls`' own miss
    // (`None => continue`), because `decls.rs` only ever pushes into that map and no source can
    // produce a leaf with an empty implementor vec. The conjunct is labelled DEFENSIVE in `scan.rs`
    // for that reason. This arm pins the PROPERTY, which is what R608 depends on.

    // (c) OVER-CHARGE CONTROL — an effectful implementor's union keeps its own bytes, unchanged.
    let l = run1("r609loud",
        "pub trait Handler { fn handle(&self); }\n\
         pub struct L;\n\
         impl Handler for L { fn handle(&self) { let _ = std::net::TcpStream::connect(\"h:1\"); } }\n");
    let lu = l["functions"].as_array().unwrap().iter()
        .find(|e| e["hash"] == "r609loud#Handler::handle").unwrap_or_else(|| panic!("{l}"));
    assert_eq!(lu["inferred"], serde_json::json!(["Net"]), "{l}");
}

/// ⟨0.40⟩ SOUNDNESS R652 — **A LEAF COLLISION IS NOT AN IMPLEMENTOR.** `trait_impls` is keyed by trait
/// LEAF and records impls of FOREIGN and std traits too (`decls.rs`:
/// `trait_impls.entry(leaf.ident.to_string()).or_default().push(ty)`, no locality test), so a crate that
/// declares its own `trait Write` and ALSO writes `impl std::io::Write for W` gives the LOCAL trait a
/// non-empty implementor vector naming a type that does not implement it.
///
/// `lt.count > 1` does not catch it: `trait_decls` counts only local `Item::Trait` declarations, so the
/// leaf is UNAMBIGUOUS and the ambiguity refusal never fires. Every lookup resolves nothing, the union
/// comes out empty, and since R609 an empty union PUBLISHES — `inferred: []`, `unresolved: false`, a §2
/// purity claim about an abstraction with no implementor at all. Downstream that row is in the
/// consumer's `deps_idx.by_key`, so conjunct 3 reads the key as ANSWERED and R608's `Unknown` is never
/// charged: `deny Net Unknown` over a `&dyn dep::Write` dispatch exited **0**.
///
/// The colliding leaf is the everyday case, not a corner — `Write`, `Read`, `Error`, `Display`,
/// `Iterator`, `Default`, `From`, `Service`, `Handler`. A crate with its own `Service`/`Handler`
/// abstraction that also implements `tower::Service`/`axum::Handler` is the ordinary instance.
///
/// **CALIBRATED: arm (d) runs the same fixture with `CANDOR_R652_PRE=1` (the pre-fix emission) and
/// asserts the sin REPRODUCES.** Without it this test would pass on a binary that refuses every union
/// row, and arm (c) is the other side of that: a REAL local implementor must still publish.
#[test]
fn r652_a_foreign_impl_sharing_a_trait_leaf_is_not_an_implementor_of_the_local_trait() {
    // The colliding-leaf dependency. `impl std::io::Write for W` is the only legal spelling of the
    // collision in one scope — `use std::io::Write;` beside a local `trait Write` is E0255 — and this
    // source `cargo build`s (§E3), as does every arm below it.
    const COLLIDE: &str =
        "pub trait Write { fn emit(&self) -> String; }\n\
         pub struct W;\n\
         impl std::io::Write for W {\n\
             fn write(&mut self, b: &[u8]) -> std::io::Result<usize> { Ok(b.len()) }\n\
             fn flush(&mut self) -> std::io::Result<()> { Ok(()) }\n\
         }\n\
         pub fn touch() { let _ = std::fs::read(\"x\"); }\n";
    // The SAME crate plus a genuine `impl Write for R`. ONE VARIABLE between (a)/(b) and (c).
    const REAL: &str =
        "pub trait Write { fn emit(&self) -> String; }\n\
         pub struct W;\n\
         impl std::io::Write for W {\n\
             fn write(&mut self, b: &[u8]) -> std::io::Result<usize> { Ok(b.len()) }\n\
             fn flush(&mut self) -> std::io::Result<()> { Ok(()) }\n\
         }\n\
         pub struct R;\n\
         impl Write for R { fn emit(&self) -> String { String::new() } }\n\
         pub fn touch() { let _ = std::fs::read(\"x\"); }\n";
    const CONSUMER: &str = "pub fn run(h: &dyn r652dep::Write) -> String { h.emit() }\n";

    let rows = |v: &serde_json::Value, hash: &str| -> usize {
        v["functions"].as_array().unwrap().iter().filter(|e| e["hash"] == hash).count()
    };

    // ── (a) THE PRODUCER MUST PUBLISH NO ROW ───────────────────────────────────────────────────────
    let (cdep, crep) = r608_dep_report("r652dep", COLLIDE);
    let cprod: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&crep).unwrap()).unwrap();
    assert_eq!(rows(&cprod, "r652dep#Write::emit"), 0,
        "NOTHING implements the local `Write`; the `impls` vector holds `W` only because \
         `impl std::io::Write for W` shares its LEAF. Publishing a row here is a §2 purity claim about \
         an abstraction with no implementor: {cprod}");
    // REACH CONTROL — the rest of the crate is still analysed, so arm (a) is an assertion about a
    // report that exists rather than about an empty one (§E1: an unanalysed entry judges nothing).
    assert!(cprod["analyzed"]["count"].as_u64().unwrap_or(0) > 0,
        "the producer must have judged something: {cprod}");
    assert_eq!(rows(&cprod, "r652dep#touch"), 1, "{cprod}");

    // ── (b) AND THE CONSUMER MUST CHARGE `Unknown` ─────────────────────────────────────────────────
    let (capp, cpol) = r608_consumer("r652app", "r652dep", CONSUMER, "deny Net Unknown\n");
    let (ccode, cv) = r608_run(&capp, &cpol, Some(&crep));
    let crun = cv["functions"].as_array().unwrap().iter().find(|e| e["fn"] == "run")
        .unwrap_or_else(|| panic!("{cv}"));
    assert_eq!(crun["inferred"], serde_json::json!(["Unknown"]),
        "this is R608's conjunct 3 reading wire absence — it can only read it if the producer stopped \
         manufacturing the row: {cv}");
    assert_eq!(crun["unknownWhy"], serde_json::json!(["dispatch:Write.emit"]), "{cv}");
    assert_eq!(ccode, 1,
        "`deny Net Unknown` MUST fail. Before R652 this exited 0 with `inferred: []` — the cardinal \
         sin: {cv}");

    // ── (c) THE CONTROL: A REAL LOCAL IMPLEMENTOR STILL PUBLISHES, AND STILL READS PURE ────────────
    // Without this arm the test passes on a binary that refuses every union row, which would delete
    // everything R609 bought.
    let (rdep, rrep) = r608_dep_report("r652real", REAL);
    let rprod: serde_json::Value = serde_json::from_slice(&std::fs::read(&rrep).unwrap()).unwrap();
    let ru = rprod["functions"].as_array().unwrap().iter()
        .find(|e| e["hash"] == "r652real#Write::emit")
        .unwrap_or_else(|| panic!("a CONFIRMED local implementor must still publish its pure-only \
                                   union — that is R609, and R652 must not take it: {rprod}"));
    assert_eq!(ru["interfaceUnion"], serde_json::json!(true), "{rprod}");
    assert_eq!(ru["inferred"], serde_json::json!([]), "{rprod}");
    let (rapp, rpol) = r608_consumer("r652realapp", "r652real",
        "pub fn run(h: &dyn r652real::Write) -> String { h.emit() }\n", "deny Net Unknown\n");
    let (rcode, rv) = r608_run(&rapp, &rpol, Some(&rrep));
    let rrun = rv["functions"].as_array().unwrap().iter().find(|e| e["fn"] == "run")
        .unwrap_or_else(|| panic!("{rv}"));
    assert_eq!(rrun["inferred"], serde_json::json!([]),
        "a library whose only implementor is pure is LEGITIMATELY pure — hedging it is the \
         fabrication R609's `pure` arm exists to forbid: {rv}");
    assert_eq!(rcode, 0, "{rv}");

    // ── (d) CALIBRATION — the SAME fixture on the pre-fix emission, which must show the sin ─────────
    // The guard lives in the PRODUCER, so the pre-arm is a dep report re-made with `CANDOR_R652_PRE`;
    // the consumer run below is byte-identical to arm (b) and reads whichever report it is handed.
    let preout = cdep.join("prerep");
    std::fs::create_dir_all(&preout).unwrap();
    let st = Command::new(bin())
        .arg(cdep.to_string_lossy().as_ref())
        .arg("--out").arg(preout.join("r").to_string_lossy().as_ref())
        .env("CANDOR_R652_PRE", "1")
        .env_remove("CANDOR_POLICY").env_remove("CANDOR_CONFIG").env_remove("CANDOR_DEPS")
        .output().expect("run candor-scan");
    assert!(st.status.success(), "{}", String::from_utf8_lossy(&st.stderr));
    let prerep = preout.join("r.r652dep.scan.json");
    let preprod: serde_json::Value = serde_json::from_slice(&std::fs::read(&prerep).unwrap()).unwrap();
    let pre_rows = rows(&preprod, "r652dep#Write::emit");
    let (precode, prev) = r608_run(&capp, &cpol, Some(&prerep));
    let prerun = prev["functions"].as_array().unwrap().iter().find(|e| e["fn"] == "run")
        .unwrap_or_else(|| panic!("{prev}"));

    for d in [&cdep, &capp, &rdep, &rapp] { let _ = std::fs::remove_dir_all(d); }

    assert_eq!(pre_rows, 1,
        "THE GATE MUST BE ABLE TO FAIL: with the guard disabled the producer must manufacture the row \
         again, or arm (a) is passing for some other reason: {preprod}");
    assert_eq!(prerun["inferred"], serde_json::json!([]),
        "and the consumer must read PURE off it — conjunct 3 sees the key as ANSWERED: {prev}");
    assert_eq!(precode, 0,
        "and `deny Net Unknown` must exit 0 on the pre-fix emission. That is the sin R652 closes, \
         executed rather than described: {prev}");
}
