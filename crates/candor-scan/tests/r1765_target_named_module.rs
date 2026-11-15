//! SOUNDNESS R1765 — A SOURCE MODULE IN A DIRECTORY NAMED `target` IS ANALYSED; CARGO'S BUILD DIRECTORY STILL IS NOT.
//!
//! Every walk pruned a directory NAMED `target` at any depth. cc 1.4.7 declares `mod target;` and keeps it in
//! `src/target/` — rustc compiles it, the scanner dropped it before recording any exclusion, so its functions (one reads
//! the environment) were absent from `functions[]` AND from `excluded`. EXECUTED (lane scratch `rustagent-r1765/fx`, the
//! shape of the first fixture below): `R1765_PROBE=observed-from-env ./target/debug/run` prints the variable, and
//! `deny Env` went 0 (published 0.42.0, and main 432eb075) -> 1.
//!
//! The build directory is `<root>/target`, a directory Cargo tagged with `CACHEDIR.TAG` (any name, any depth), or one
//! `CARGO_TARGET_DIR` / `CARGO_BUILD_TARGET_DIR` / `build.target-dir` names. Its generated code must stay out: a
//! `generated_net` written there would otherwise FABRICATE `Net` on code the crate never compiles.

use std::path::{Path, PathBuf};
use std::process::Command;

fn bin() -> &'static str {
    env!("CARGO_BIN_EXE_candor-scan")
}

fn tmpdir(tag: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("candor-r1765-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn write(p: &Path, s: &str) {
    std::fs::create_dir_all(p.parent().unwrap()).unwrap();
    std::fs::write(p, s).unwrap();
}

/// The executed fixture: the crate's one public fn reaches an environment read that lives in `src/target/info.rs`.
fn crate_with_target_module(d: &Path) {
    write(&d.join("Cargo.toml"), "[package]\nname = \"fx\"\nversion = \"0.1.0\"\nedition = \"2021\"\n");
    write(&d.join("src/lib.rs"), "mod target;\npub fn describe_zq() -> String { target::info::from_env_zq() }\n");
    write(&d.join("src/target.rs"), "pub mod info;\n");
    write(
        &d.join("src/target/info.rs"),
        "pub fn from_env_zq() -> String { std::env::var(\"R1765_PROBE\").unwrap_or_else(|_| \"unset\".to_string()) }\n",
    );
}

const GENERATED_NET: &str = "pub fn generated_net_zq() { let _ = std::net::TcpStream::connect(\"127.0.0.1:9\"); }\n";

fn scan(dir: &Path, policy: Option<&str>, env: &[(&str, &Path)]) -> (i32, serde_json::Value, String) {
    let mut cmd = Command::new(bin());
    cmd.arg(dir.to_string_lossy().as_ref()).arg("--json");
    if let Some(p) = policy {
        let pf = dir.join("policy.txt");
        std::fs::write(&pf, p).unwrap();
        cmd.arg("--policy").arg(&pf);
    }
    cmd.env_remove("CANDOR_DEPS").env_remove("CANDOR_CONFIG").env_remove("CANDOR_POLICY").env_remove("CARGO_TARGET_DIR")
        .env_remove("CARGO_BUILD_TARGET_DIR");
    for (k, v) in env {
        cmd.env(k, v);
    }
    let out = cmd.output().expect("run candor-scan");
    let code = out.status.code().unwrap_or(-1);
    assert!(code == 0 || code == 1, "scan failed ({code}): {}", String::from_utf8_lossy(&out.stderr));
    (code, serde_json::from_slice(&out.stdout).expect("report json"), String::from_utf8_lossy(&out.stderr).into_owned())
}

fn inferred(v: &serde_json::Value, f: &str) -> Option<Vec<String>> {
    v["functions"].as_array().unwrap().iter().find(|r| r["fn"].as_str() == Some(f)).map(|r| {
        r["inferred"].as_array().map(|a| a.iter().filter_map(|e| e.as_str().map(str::to_string)).collect()).unwrap_or_default()
    })
}

fn fns_ending(v: &serde_json::Value, leaf: &str) -> Vec<String> {
    v["functions"].as_array().unwrap().iter().filter_map(|r| r["fn"].as_str()).filter(|f| f.ends_with(leaf)).map(str::to_string).collect()
}

#[test]
fn r1765_a_module_directory_named_target_is_analysed_and_reaches_the_gate() {
    let d = tmpdir("module");
    crate_with_target_module(&d);
    let (_, v, _) = scan(&d, None, &[]);
    assert_eq!(inferred(&v, "target::info::from_env_zq").as_deref(), Some(&["Env".to_string()][..]), "{v:#}");
    assert_eq!(inferred(&v, "describe_zq").as_deref(), Some(&["Env".to_string()][..]), "the caller inherits it: {v:#}");
    let excluded = v["excluded"].as_array().cloned().unwrap_or_default();
    assert!(excluded.iter().all(|e| e["class"] != "build-output"), "src/target is source, not build output: {excluded:?}");
    let (rc, _, _) = scan(&d, Some("deny Env\n"), &[]);
    assert_eq!(rc, 1, "deny Env must fire on the environment read under src/target/");
    let (rc, _, _) = scan(&d, Some("deny Env describe_zq\n"), &[]);
    assert_eq!(rc, 1, "and on its caller, scoped");
    let _ = std::fs::remove_dir_all(&d);
}

#[test]
fn r1765_deeper_target_directories_are_walked_too() {
    let d = tmpdir("deep");
    write(&d.join("Cargo.toml"), "[package]\nname = \"fx\"\nversion = \"0.1.0\"\nedition = \"2021\"\n");
    write(&d.join("src/lib.rs"), "pub mod a;\n");
    write(&d.join("src/a.rs"), "pub mod target;\n");
    write(&d.join("src/a/target.rs"), "pub mod target;\n");
    write(&d.join("src/a/target/target.rs"), "pub fn deep_env_zq() -> Option<String> { std::env::var(\"X\").ok() }\n");
    let (_, v, _) = scan(&d, None, &[]);
    assert_eq!(inferred(&v, "a::target::target::deep_env_zq").as_deref(), Some(&["Env".to_string()][..]), "{v:#}");
    let _ = std::fs::remove_dir_all(&d);
}

#[test]
fn r1765_the_cargo_build_directory_stays_out_of_the_scan() {
    let d = tmpdir("builddir");
    crate_with_target_module(&d);
    // <root>/target — by name, with or without a tag (Cargo's default build directory; `cargo package` never ships one).
    write(&d.join("target/debug/build/fx-1/out/gen.rs"), GENERATED_NET);
    // A tagged build directory deeper down, named `target` (`CARGO_TARGET_DIR=nested/target`) and named anything else.
    write(&d.join("nested/target/CACHEDIR.TAG"), "Signature: 8a477f597d28d172789f06886806bc55\n");
    write(&d.join("nested/target/debug/build/fx-1/out/gen.rs"), GENERATED_NET);
    write(&d.join("build-out/CACHEDIR.TAG"), "Signature: 8a477f597d28d172789f06886806bc55\n");
    write(&d.join("build-out/debug/build/fx-1/out/gen.rs"), GENERATED_NET);
    // An untagged directory `build.target-dir` names.
    write(&d.join(".cargo/config.toml"), "[build]\ntarget-dir = \"cfgout\"\n");
    write(&d.join("cfgout/debug/build/fx-1/out/gen.rs"), GENERATED_NET);
    // An untagged directory only CARGO_TARGET_DIR names — the control arm below scans without it.
    write(&d.join("envout/debug/build/fx-1/out/gen.rs"), GENERATED_NET);
    let (rc, v, _) = scan(&d, Some("deny Net\n"), &[("CARGO_TARGET_DIR", &d.join("envout"))]);
    assert!(fns_ending(&v, "generated_net_zq").is_empty(), "build output was analysed: {:?}", fns_ending(&v, "generated_net_zq"));
    assert_eq!(rc, 0, "no Net is compiled into this crate: {v:#}");
    assert!(inferred(&v, "target::info::from_env_zq").is_some(), "the source module is still analysed: {v:#}");
    // CALIBRATION — the fixture's generated file IS read when nothing names its directory as build output, so the arm above
    // measured the pruning and not an unreadable fixture.
    let (rc, v, _) = scan(&d, Some("deny Net\n"), &[]);
    assert_eq!(fns_ending(&v, "generated_net_zq"), ["envout::debug::build::fx-1::out::gen::generated_net_zq"], "{v:#}");
    assert_eq!(rc, 1);
    let _ = std::fs::remove_dir_all(&d);
}

/// The SECOND copy of the name rule, in the scan's path-shape filter: un-pruning the walk alone would have recorded the module
/// as `build-output` (`judgedElsewhere`, never peeked) — disclosed in name, still unanalysed. `--include-tests` changes nothing.
#[test]
fn r1765_the_path_shape_filter_does_not_exclude_the_module_either() {
    let d = tmpdir("shape");
    crate_with_target_module(&d);
    for extra in [false, true] {
        let mut cmd = Command::new(bin());
        cmd.arg(d.to_string_lossy().as_ref()).arg("--json").env_remove("CANDOR_DEPS").env_remove("CANDOR_CONFIG");
        if extra {
            cmd.arg("--include-tests");
        }
        let out = cmd.output().unwrap();
        let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
        assert!(inferred(&v, "target::info::from_env_zq").is_some(), "include_tests={extra}: {v:#}");
        assert!(v["excluded"].as_array().map(|a| a.is_empty()).unwrap_or(true), "include_tests={extra}: {:?}", v["excluded"]);
    }
    let _ = std::fs::remove_dir_all(&d);
}

/// A hermetic `backoff` (the classified crate's name) for the R1627 method probe, which must read EVERY file of the consumer:
/// a blanket by-value `build` declared in the consumer's `src/target/` beats the inherent `build(&self)`, so the chain must stay
/// untyped, exactly as the same trait one directory over leaves it.
#[test]
fn r1765_the_method_probe_reads_a_trait_declared_under_src_target() {
    let root = tmpdir("probe");
    let dep = root.join("backoff");
    write(&dep.join("Cargo.toml"), "[package]\nname = \"backoff\"\nversion = \"0.4.0\"\nedition = \"2021\"\n");
    write(
        &dep.join("src/lib.rs"),
        "pub struct ExponentialBackoffBuilder;\npub struct ExponentialBackoff;\n\
         impl ExponentialBackoffBuilder { pub fn new() -> Self { ExponentialBackoffBuilder } pub fn build(&self) -> ExponentialBackoff { ExponentialBackoff } }\n\
         impl ExponentialBackoff { pub fn next_backoff(&mut self) -> Option<u32> { None } }\n",
    );
    let call = "use backoff::ExponentialBackoffBuilder;\n\
                pub fn h_chain_zq() -> Option<u32> { ExponentialBackoffBuilder::new().build().next_backoff() }\n";
    let mk = |name: &str, dir: &str| -> PathBuf {
        let d = root.join(name);
        write(
            &d.join("Cargo.toml"),
            &format!("[package]\nname = \"{name}\"\nversion = \"0.1.0\"\nedition = \"2021\"\n[dependencies]\nbackoff = {{ path = \"../backoff\" }}\n"),
        );
        write(&d.join("Cargo.lock"), &format!("version = 4\n\n[[package]]\nname = \"{name}\"\nversion = \"0.1.0\"\n\n[[package]]\nname = \"backoff\"\nversion = \"0.4.0\"\n"));
        write(&d.join("src/lib.rs"), &format!("mod {dir};\n{call}"));
        write(&d.join(format!("src/{dir}.rs")), "pub mod mine;\n");
        write(&d.join(format!("src/{dir}/mine.rs")), "pub trait Mine { fn build(self) -> u8; }\nimpl<T> Mine for T { fn build(self) -> u8 { 0 } }\n");
        d
    };
    let rand = |v: &serde_json::Value| inferred(v, "h_chain_zq").is_some_and(|r| r.iter().any(|e| e == "Rand"));
    let (_, under_target, _) = scan(&mk("ct", "target"), None, &[]);
    let (_, under_other, _) = scan(&mk("co", "tgt"), None, &[]);
    assert_eq!(rand(&under_target), rand(&under_other), "the directory name decided the probe: {under_target:#} vs {under_other:#}");
    assert!(!rand(&under_target), "a trait that could supply `build` must keep the chain untyped: {under_target:#}");
    let _ = std::fs::remove_dir_all(&root);
}

/// The dependency SOURCE index (R1165's narrowing, `depsig.rs`) takes the UNION of every same-leaf candidate, so it is sound
/// only if every definition is in sight. Here the bounded `check<T: PartialEq>` that runs lives in the dependency's
/// `src/target/mod.rs`; with it out of sight the unbounded twin alone withholds the `dispatch:PartialEq.eq` disclosure and the
/// chained caller reads pure while the program runs `Noisy::eq`. EXECUTED in the lane (`rustagent-r1765/depsig-exec`): the
/// consumer writes its marker. The same crates with the module directory named `tgt` are the control.
#[test]
fn r1765_the_dependency_source_index_reads_a_candidate_under_src_target() {
    let chained = |dir: &str| -> serde_json::Value {
        let root = tmpdir(&format!("depsig-{dir}"));
        let dep = root.join("cdep");
        write(&dep.join("Cargo.toml"), "[package]\nname = \"cdep_r1765\"\nversion = \"0.1.0\"\nedition = \"2021\"\n");
        write(&dep.join("src/lib.rs"), &format!("mod {dir};\npub mod a {{ pub fn check<T>(_t: T) -> bool {{ true }} }}\npub use {dir}::check;\n"));
        write(&dep.join(format!("src/{dir}/mod.rs")), "pub fn check<T: PartialEq>(t: T) -> bool { t == t }\n");
        let reports = root.join("reports");
        let out = Command::new(bin()).arg(&dep).arg("--out").arg(reports.join("r")).env_remove("CANDOR_DEPS")
            .env_remove("CANDOR_CONFIG").env_remove("CANDOR_POLICY").output().unwrap();
        assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
        let cons = root.join("ccons");
        write(&cons.join("Cargo.toml"), "[package]\nname = \"ccons\"\nversion = \"0.1.0\"\nedition = \"2021\"\n[dependencies]\ncdep_r1765 = { path = \"../cdep\" }\n");
        write(
            &cons.join("src/lib.rs"),
            "pub struct Noisy(pub i32);\n\
             impl PartialEq for Noisy { fn eq(&self, o: &Noisy) -> bool { let _ = std::fs::write(\"/tmp/r1765_eq\", \"x\"); self.0 == o.0 } }\n\
             pub fn call_check_zq() -> bool { cdep_r1765::check(Noisy(1)) }\n",
        );
        let out = Command::new(bin()).arg(&cons).arg("--json").env("CANDOR_DEPS", &reports).env_remove("CANDOR_CONFIG")
            .env_remove("CANDOR_POLICY").output().unwrap();
        let v = serde_json::from_slice(&out.stdout).expect("report json");
        let _ = std::fs::remove_dir_all(&root);
        v
    };
    let under_target = chained("target");
    let under_other = chained("tgt");
    let control = inferred(&under_other, "call_check_zq").unwrap_or_default();
    assert!(!control.is_empty(), "control: the chained caller of the bounded `check` is disclosed: {under_other:#}");
    assert_eq!(inferred(&under_target, "call_check_zq").unwrap_or_default(), control, "the directory name decided it: {under_target:#}");
}
