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

    assert_eq!(verdict["spec"], "0.24", "verdict declares the spec version");
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
            "candor": {{"version": "scan-{}", "toolchain": "stable", "spec": "0.24"}},
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
    let out = Command::new(bin()).arg(d.to_string_lossy().as_ref()).output().expect("run candor-scan");
    let _ = std::fs::remove_dir_all(&d);
    assert_eq!(out.status.code(), Some(0), "inert keys don't fail the scan (and an absent baseline is a note, not a failure)");
    let stderr = String::from_utf8(out.stderr).unwrap();
    assert!(stderr.contains("config key 'taint' is recognized by the candor family but not implemented by candor-scan"),
        "the inert `taint` key must be disclosed loudly: {stderr}");
    assert!(stderr.contains("config key 'strict'"), "every inert recognized key is disclosed: {stderr}");
    assert!(!stderr.contains("config key 'baseline'"),
        "`baseline` is implemented now — it must not be disclosed as inert: {stderr}");
    assert!(stderr.contains("regression guard is not active"),
        "an absent configured baseline gets the adopt note: {stderr}");
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

    // AND the OTHER exit-2 cause is untouched: a gate CONFIG that never loaded still writes NO document
    // (a fabricated verdict would be a guess — SPEC §3.3's cause (a)). Run on a COMPLETE crate, so the
    // absence is attributable to the config and not to the manifest.
    let good = make_crate("incompleteviol-cfg", "pub fn go() {}\n");
    let v3path = good.join("verdict3.json");
    let out3 = Command::new(bin())
        .args([
            good.to_string_lossy().as_ref(),
            "--policy", good.join("nope.policy").to_string_lossy().as_ref(),
            "--gate-json", v3path.to_string_lossy().as_ref(),
        ])
        .output()
        .expect("run candor-scan");
    assert_eq!(out3.status.code(), Some(2), "an unreadable policy is still exit 2");
    assert!(!v3path.exists(), "a broken gate CONFIG must still write no verdict document");

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
        assert!(
            !v.exists(),
            "{name}: …and take the UNREADABLE-POLICY posture — no verdict document, byte-identically to \
             `candor-query gate --report` on the same policy"
        );
    }
    // A config-defined alias is vocabulary, not an error.
    std::fs::create_dir_all(d.join(".candor")).unwrap();
    std::fs::write(d.join(".candor/config"), "unknown-alias corp = indirect\n").unwrap();
    let (rc, err) = run("aliased", "deny Unknown[corp]\n", None);
    assert_eq!(rc, 1, "a defined alias resolves and the rule fires — the refusal must not eat ⟨0.19⟩:\n{err}");

    let _ = std::fs::remove_dir_all(&d);
}
