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
fn candor_config_relative_path_resolves_against_the_config_dir_not_the_cwd() {
    // Family decision: a RELATIVE path value in .candor/config anchors to the CONFIG FILE'S directory
    // (the config is checked in and travels with the code), never the process CWD. Run the scan from
    // an unrelated CWD: if resolution were CWD-based the policy would be unreadable (exit 2 /
    // gate-off); anchored correctly, the deny-Exec gate FIRES (exit 1).
    let d = make_crate("cfgrel", "pub fn go() { std::process::Command::new(\"sh\").status().unwrap(); }");
    std::fs::create_dir_all(d.join(".candor")).unwrap();
    std::fs::write(d.join(".candor/deny-exec.policy"), "deny Exec\n").unwrap();
    std::fs::write(d.join(".candor/config"), "policy deny-exec.policy\n").unwrap();
    let out = Command::new(bin())
        .arg(d.to_string_lossy().as_ref())
        .current_dir(std::env::temp_dir())
        .output().expect("run candor-scan");
    let _ = std::fs::remove_dir_all(&d);
    assert_eq!(out.status.code(), Some(1),
        "a config-relative policy path must resolve against the config's own directory (gate fires)");
    let stderr = String::from_utf8(out.stderr).unwrap();
    assert!(stderr.contains("AS-EFF-006") || String::from_utf8_lossy(&out.stdout).contains("AS-EFF-006"),
        "the deny-Exec violation must be reported: {stderr}");
}

#[test]
fn candor_config_recognized_but_unimplemented_key_warns_loudly() {
    // A checked-in `baseline` (or strict/no-ambient/closed-world/taint) key is spec-recognized but not
    // wired to any candor-scan mode — a DECLARED-GATE-SILENTLY-OFF unless disclosed. It must warn.
    let d = make_crate("cfginert", "pub fn pure() -> u32 { 1 }");
    std::fs::create_dir_all(d.join(".candor")).unwrap();
    std::fs::write(d.join(".candor/config"), "baseline .candor/baseline\ntaint true\n").unwrap();
    let out = Command::new(bin()).arg(d.to_string_lossy().as_ref()).output().expect("run candor-scan");
    let _ = std::fs::remove_dir_all(&d);
    assert_eq!(out.status.code(), Some(0), "inert keys don't fail the scan");
    let stderr = String::from_utf8(out.stderr).unwrap();
    assert!(stderr.contains("config key 'baseline' is recognized by the candor family but not implemented by candor-scan"),
        "the inert `baseline` key must be disclosed loudly: {stderr}");
    assert!(stderr.contains("config key 'taint'"), "every inert recognized key is disclosed: {stderr}");
}
