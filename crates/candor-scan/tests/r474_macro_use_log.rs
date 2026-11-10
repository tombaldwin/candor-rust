//! SOUNDNESS R474 — a BARE log macro bound by the crate root's `#[macro_use] extern crate log` was Log-silent.
//!
//! tokio-reactor 0.1.12 `registration::Inner::poll_ready` logs through a bare `debug!` (registration.rs:91) and read
//! `[]` with only `invisible` on every arm (published 0.41.3 covered it with a wrong-reason `Unknown`), while the
//! `use log::debug;` and `log::debug!` spellings of the same call charge Log. Executed fixture: rustagent-r474/fx/r474
//! (a counting `log::Log` installed by `exec`; every crate below with a Log expectation logged exactly one record per
//! call, every pure one logged none).
//!
//! The pure twins are the shapes the rule must not charge: a crate with no log import that defines a no-op `debug!`,
//! a no-op `debug!` declared in the BODY, an ungated crate-level one in textual scope (rustc's shadowing), a name a
//! `#[macro_use(info)]` list does not import, and an import the feature picture proves compiled out. The
//! cfg-exclusive fallback (rand 0.6.5's shape) is NOT a pure twin: with the feature on it logs.

use std::path::{Path, PathBuf};
use std::process::Command;

fn bin() -> &'static str {
    env!("CARGO_BIN_EXE_candor-scan")
}

fn fixture(tag: &str, manifest_tail: &str, src: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("candor-r474-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(d.join("src")).unwrap();
    std::fs::write(
        d.join("Cargo.toml"),
        format!("[package]\nname = \"{tag}\"\nversion = \"0.1.0\"\nedition = \"2018\"\n{manifest_tail}"),
    )
    .unwrap();
    std::fs::write(d.join("src/lib.rs"), src).unwrap();
    d
}

fn scan(dir: &Path, off: bool) -> serde_json::Value {
    let mut cmd = Command::new(bin());
    cmd.arg(dir.to_string_lossy().as_ref()).arg("--json");
    cmd.env_remove("CANDOR_DEPS").env_remove("CANDOR_CONFIG").env_remove("CANDOR_POLICY").env_remove("CANDOR_R474_OFF");
    if off {
        cmd.env("CANDOR_R474_OFF", "1");
    }
    let o = cmd.output().expect("run candor-scan");
    let code = o.status.code().unwrap_or(-1);
    assert!(code == 0 || code == 1, "scan failed ({code}): {}", String::from_utf8_lossy(&o.stderr));
    serde_json::from_slice(&o.stdout).expect("report json")
}

fn inferred(v: &serde_json::Value, f: &str) -> Vec<String> {
    let mut out: Vec<String> = v["functions"]
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r["fn"].as_str() == Some(f))
        .and_then(|r| r["inferred"].as_array())
        .map(|a| a.iter().filter_map(|e| e.as_str().map(str::to_string)).collect())
        .unwrap_or_default();
    out.sort();
    out
}

const LOG_DEP: &str = "[dependencies]\nlog = \"0.4\"\n";
const LOG_OPT: &str = "[dependencies]\nlog = { version = \"0.4\", optional = true }\n";

#[test]
fn r474_bare_log_macro_under_macro_use_is_charged_log() {
    let d = fixture(
        "mu",
        LOG_DEP,
        "#[macro_use]\nextern crate log;\n\
         pub fn mu_debug_zq(x: u8) { debug!(\"x {}\", x); }\n\
         pub fn mu_info_zq(x: u8) { info!(\"x {}\", x); }\n\
         pub fn mu_logm_zq(x: u8) { log!(log::Level::Warn, \"x {}\", x); }\n\
         pub fn mu_path_zq(x: u8) { log::debug!(\"x {}\", x); }\n\
         pub fn mu_bodyshadow_zq(x: u8) { macro_rules! debug { ($($t:tt)*) => () } debug!(\"x {}\", x); }\n\
         pub mod inner { pub fn mu_inner_zq(x: u8) { error!(\"x {}\", x); } }\n\
         pub fn mu_calm_zq(x: u8) -> u8 { x + 1 }\n",
    );
    let v = scan(&d, false);
    for f in ["mu_debug_zq", "mu_info_zq", "mu_logm_zq", "mu_path_zq", "inner::mu_inner_zq"] {
        assert_eq!(inferred(&v, f), vec!["Log"], "{f} logs (executed):\n{v:#}");
    }
    assert!(inferred(&v, "mu_bodyshadow_zq").is_empty(), "a body-local no-op `debug!` shadows the import:\n{v:#}");
    assert!(inferred(&v, "mu_calm_zq").is_empty(), "control:\n{v:#}");

    // The kill switch restores the silence on exactly the bare spellings; the `log::` path is untouched.
    let off = scan(&d, true);
    for f in ["mu_debug_zq", "mu_info_zq", "mu_logm_zq", "inner::mu_inner_zq"] {
        assert!(inferred(&off, f).is_empty(), "{f} under CANDOR_R474_OFF:\n{off:#}");
    }
    assert_eq!(inferred(&off, "mu_path_zq"), vec!["Log"]);
}

#[test]
fn r474_cfg_exclusive_fallback_keeps_the_import() {
    // rand 0.6.5 / getrandom 0.1 / handlebars / quickcheck: a cfg-gated import beside the opposite-gated no-op.
    let rand_shape = "#[cfg(feature = \"log\")] #[macro_use] extern crate log;\n\
         #[cfg(not(feature = \"log\"))] macro_rules! debug { ($($x:tt)*) => () }\n\
         pub fn g_debug_zq(x: u8) { debug!(\"x {}\", x); }\n";
    // `log` an undeclared (optional-dependency) feature: UNKNOWN, so the import is kept.
    let v = scan(&fixture("randshape", LOG_OPT, rand_shape), false);
    assert_eq!(inferred(&v, "g_debug_zq"), vec!["Log"], "{v:#}");
    let gated = |on: &str| format!("[features]\ndefault = [{on}]\nlg = [\"log\"]\n{LOG_OPT}");
    let lg_shape = rand_shape.replace("feature = \"log\"", "feature = \"lg\"");
    // A declared default feature: the import is in the default build.
    let v = scan(&fixture("declaredon", &gated("\"lg\""), &lg_shape), false);
    assert_eq!(inferred(&v, "g_debug_zq"), vec!["Log"], "{v:#}");
    // A declared NON-default feature: the import is compiled out of the default build (executed: 0 records),
    // the same verdict `lang::is_cfg_inactive` gives every other item.
    let v = scan(&fixture("declaredoff", &gated(""), &lg_shape), false);
    assert!(inferred(&v, "g_debug_zq").is_empty(), "{v:#}");

    // getrandom 0.1 writes the import inside a root `cfg_if!` arm.
    let v = scan(
        &fixture(
            "cfgif",
            "[dependencies]\nlog = \"0.4\"\ncfg-if = \"1\"\n",
            "#[macro_use]\nextern crate cfg_if;\n\
             cfg_if! {\n    if #[cfg(unix)] {\n        #[macro_use]\n        extern crate log;\n    } else {\n        \
             macro_rules! warn { ($($x:tt)*) => () }\n    }\n}\n\
             pub fn ci_warn_zq(x: u8) { warn!(\"x {}\", x); }\n",
        ),
        false,
    );
    assert_eq!(inferred(&v, "ci_warn_zq"), vec!["Log"], "{v:#}");
}

#[test]
fn r474_pure_twins_stay_pure() {
    // No log import at all, a crate-local no-op `debug!`: the twin of the charged spelling.
    let v = scan(&fixture("twin", LOG_DEP, "macro_rules! debug { ($($x:tt)*) => () }\npub fn tw_debug_zq(x: u8) { debug!(\"x {}\", x); }\n"), false);
    assert!(inferred(&v, "tw_debug_zq").is_empty(), "{v:#}");
    // `#[macro_use(info)]` imports `info!` only; the local `debug!` is the one that runs.
    let v = scan(
        &fixture(
            "musel",
            LOG_DEP,
            "#[macro_use(info)]\nextern crate log;\nmacro_rules! debug { ($($x:tt)*) => () }\n\
             pub fn sel_info_zq(x: u8) { info!(\"x {}\", x); }\npub fn sel_debug_zq(x: u8) { debug!(\"x {}\", x); }\n",
        ),
        false,
    );
    assert_eq!(inferred(&v, "sel_info_zq"), vec!["Log"], "{v:#}");
    assert!(inferred(&v, "sel_debug_zq").is_empty(), "{v:#}");
    // An UNGATED crate-level definition after an ungated import shadows it (rustc's textual rule; executed: 0 records).
    let v = scan(
        &fixture(
            "both",
            LOG_DEP,
            "#[macro_use]\nextern crate log;\nmacro_rules! debug { ($($x:tt)*) => () }\npub fn both_debug_zq(x: u8) { debug!(\"x {}\", x); }\n",
        ),
        false,
    );
    assert!(inferred(&v, "both_debug_zq").is_empty(), "{v:#}");
}
