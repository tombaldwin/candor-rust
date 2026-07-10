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

    assert_eq!(verdict["spec"], "0.8", "verdict declares the spec version");
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
    // filename shape + entry hashes, so an empty report still drew "κ doesn't know 1 dependency…"
    // (candor-java/candor-ts stay correctly quiet on the same shape).
    let d = std::env::temp_dir().join(format!("candor-scan-cli-kappaempty-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(d.join("src")).unwrap();
    std::fs::write(d.join("Cargo.toml"),
        "[package]\nname = \"kappaledger\"\n\n[dependencies]\ndepc = \"1\"\n").unwrap();
    std::fs::write(d.join("src/lib.rs"), "pub fn use_dep() { depc::hit(); }\n").unwrap();
    // The empty depc report, named OUTSIDE the `….<crate>.scan.json` shape — the envelope's
    // `package` field alone must carry the coverage claim.
    let rep = d.join("depc-purity.json");
    std::fs::write(&rep, format!(r#"{{
        "candor": {{"version": "scan-{}", "toolchain": "stable", "spec": "0.8"}},
        "package": "depc",
        "functions": []}}"#, env!("CARGO_PKG_VERSION"))).unwrap();

    // CONTROL (no chaining): the ledger fires — depc is a genuine blind spot.
    let out = Command::new(bin()).arg(d.to_string_lossy().as_ref()).arg("--json")
        .output().expect("run candor-scan");
    let stderr = String::from_utf8(out.stderr).unwrap();
    assert!(stderr.contains("κ doesn't know") && stderr.contains("depc"),
        "without chaining, the called-but-unknown dep must be disclosed: {stderr}");

    // CHAINED empty report: NO ledger line, and the join-less call reads pure (the claim honored).
    let out = Command::new(bin()).arg(d.to_string_lossy().as_ref()).arg("--json")
        .env("CANDOR_DEPS", rep.to_string_lossy().as_ref())
        .output().expect("run candor-scan");
    let _ = std::fs::remove_dir_all(&d);
    assert_eq!(out.status.code(), Some(0));
    let stderr = String::from_utf8(out.stderr).unwrap();
    assert!(!stderr.contains("κ doesn't know"),
        "an empty chained report is coverage — the ledger must stay quiet: {stderr}");
    let v: serde_json::Value = serde_json::from_str(String::from_utf8(out.stdout).unwrap().trim())
        .expect("pure JSON report");
    assert!(v["functions"].as_array().unwrap().iter()
            .all(|f| f["fn"].as_str() != Some("use_dep")),
        "the call into the all-pure dep reads pure (omitted from the report): {v}");
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
        "candor": {{"version": "scan-{me}", "toolchain": "stable", "spec": "0.8"}},
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
