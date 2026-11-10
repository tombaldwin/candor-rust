//! SOUNDNESS R1612 — a call's SECOND effects (`candor_classify::ALSO_EFFECTS`) are charged at every site that charges a
//! `classify` answer, and one with a locator surface is marked `incomplete`.
//!
//! Every assertion here FAILED on b0a2dc4 (one-effect `classify` plus R338's hook asked by the main path only):
//!   - the main path charged `Env` alone for `portable_pty::CommandBuilder::new` (EXECUTED: also `access(2)` of the passwd
//!     shell when `SHELL` is unset), so `deny Fs` passed;
//!   - the cfg-duplicated `pub use`-alias path (R105) never asked the trust-store hook, so a call spelled through an
//!     opposing-cfg `pub use … as Cb;` pair lost any second effect the main path would have charged;
//!   - the trust-store `Fs` R338 added was charged with NO `incomplete` mark, so a benign sibling literal certified it:
//!     `allow Fs in f /etc/hosts` exited 0 over a function that also calls `tungstenite::client_tls`.

use std::path::{Path, PathBuf};
use std::process::Command;

fn bin() -> &'static str {
    env!("CARGO_BIN_EXE_candor-scan")
}

fn fixture(tag: &str, src: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("candor-r1612-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(d.join("src")).unwrap();
    std::fs::write(
        d.join("Cargo.toml"),
        "[package]\nname = \"fx\"\nversion = \"0.1.0\"\nedition = \"2021\"\n[dependencies]\nbackoff = \"0.4\"\n\
         grep-cli = \"0.1\"\ntime = \"0.3\"\nportable-pty = \"0.9\"\ntungstenite = \"0.30\"\n",
    )
    .unwrap();
    std::fs::write(d.join("src/lib.rs"), src).unwrap();
    d
}

fn scan(dir: &Path, policy: Option<&str>) -> (i32, serde_json::Value) {
    let mut cmd = Command::new(bin());
    cmd.arg(dir.to_string_lossy().as_ref()).arg("--json");
    let pf = dir.join("policy.txt");
    if let Some(p) = policy {
        std::fs::write(&pf, p).unwrap();
        cmd.arg("--policy").arg(&pf);
    }
    let out = cmd
        .env_remove("CANDOR_DEPS")
        .env_remove("CANDOR_CONFIG")
        .env_remove("CANDOR_POLICY")
        .output()
        .expect("run candor-scan");
    let code = out.status.code().unwrap_or(-1);
    assert!(code == 0 || code == 1, "scan failed ({code}): {}", String::from_utf8_lossy(&out.stderr));
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).expect("report json");
    (code, v)
}

fn field(v: &serde_json::Value, f: &str, key: &str) -> Vec<String> {
    let row = v["functions"].as_array().unwrap().iter().find(|r| r["fn"].as_str() == Some(f));
    let mut out: Vec<String> = row
        .and_then(|r| r[key].as_array())
        .map(|a| a.iter().filter_map(|e| e.as_str().map(str::to_string)).collect())
        .unwrap_or_default();
    out.sort();
    out
}

#[test]
fn second_effects_are_charged_beside_the_primary_with_pure_controls() {
    let src = "\
pub fn f_pp_new(p: &str) -> portable_pty::CommandBuilder { portable_pty::CommandBuilder::new(p) }\n\
pub fn f_bo_next(b: &mut backoff::ExponentialBackoff) -> Option<std::time::Duration> { use backoff::backoff::Backoff; b.next_backoff() }\n\
pub fn f_gc_associate(b: &mut grep_cli::DecompressionMatcherBuilder) { b.associate(\"*.zz\", \"gzip\", [\"-d\"]); }\n\
pub fn f_tm_local() -> bool { time::UtcOffset::current_local_offset().is_ok() }\n\
pub fn g_caller(b: &mut backoff::ExponentialBackoff) -> bool { f_bo_next(b).is_some() }\n\
pub fn c_pp_arg(c: &mut portable_pty::CommandBuilder) { c.arg(\"x\") }\n\
pub fn c_gc_new() -> grep_cli::DecompressionMatcherBuilder { grep_cli::DecompressionMatcherBuilder::new() }\n\
pub fn c_tm_hms() -> bool { time::UtcOffset::from_hms(1, 0, 0).is_ok() }\n\
mod local { pub struct CommandBuilder; impl CommandBuilder { pub fn new(_p: &str) -> Self { CommandBuilder } } }\n\
pub fn x_local_new() -> local::CommandBuilder { local::CommandBuilder::new(\"sh\") }\n";
    let d = fixture("main", src);
    let (_, v) = scan(&d, None);
    let s = |x: &[&str]| x.iter().map(|e| e.to_string()).collect::<Vec<_>>();
    assert_eq!(field(&v, "f_pp_new", "inferred"), s(&["Env", "Fs"]), "{v:#}");
    assert_eq!(field(&v, "f_bo_next", "inferred"), s(&["Clock", "Rand"]));
    assert_eq!(field(&v, "f_gc_associate", "inferred"), s(&["Env", "Fs"]));
    assert_eq!(field(&v, "f_tm_local", "inferred"), s(&["Clock", "Fs"]));
    assert_eq!(field(&v, "g_caller", "inferred"), s(&["Clock", "Rand"]), "propagates to a caller");
    // the second Fs has a locator nobody can see — marked, not left looking complete
    for f in ["f_pp_new", "f_gc_associate", "f_tm_local"] {
        assert_eq!(field(&v, f, "incomplete"), s(&["Fs"]), "{f}");
    }
    for f in ["c_pp_arg", "c_gc_new", "c_tm_hms", "x_local_new"] {
        assert!(field(&v, f, "inferred").is_empty(), "{f} is a pure neighbour or a look-alike:\n{v:#}");
    }
    let _ = std::fs::remove_dir_all(&d);
}

#[test]
fn a_benign_sibling_literal_does_not_certify_a_second_effect() {
    let src = "\
pub fn m_ws(r: &str, s: std::net::TcpStream) -> usize { let n = std::fs::read(\"/etc/hosts\").map(|v| v.len()).unwrap_or(0); let _ = tungstenite::client_tls(r, s); n }\n\
pub fn m_gc(b: &mut grep_cli::DecompressionMatcherBuilder) -> usize { let n = std::fs::read(\"/etc/hosts\").map(|v| v.len()).unwrap_or(0); b.associate(\"*.zz\", \"gzip\", [\"-d\"]); n }\n\
pub fn ok_only(_x: u8) -> usize { std::fs::read(\"/etc/hosts\").map(|v| v.len()).unwrap_or(0) }\n";
    let d = fixture("mask", src);
    for f in ["m_ws", "m_gc"] {
        let (code, _) = scan(&d, Some(&format!("allow Fs in {f} /etc/hosts\n")));
        assert_eq!(code, 1, "{f}: a second Fs whose path the call does not name must not be certified by /etc/hosts");
    }
    // The second Fs declares `read` (its executed window wrote nothing), so the sibling's `read` claim survives.
    let (_, v) = scan(&d, None);
    assert_eq!(field(&v, "m_gc", "fs"), vec!["read".to_string()], "{v:#}");
    // The control: the same literal, alone, IS certified — the mark is what changed, not the allowlist.
    let (code, _) = scan(&d, Some("allow Fs in ok_only /etc/hosts\n"));
    assert_eq!(code, 0, "a function whose only Fs is the visible literal stays certifiable");
    let _ = std::fs::remove_dir_all(&d);
}

#[test]
fn the_cfg_duplicated_alias_route_charges_second_effects_too() {
    // A `pub use` pair under opposing cfgs is the R105 alias route (`ALIAS_ALT_SEP`), which charges each arm's answer
    // itself and `continue`s past the main path — so it must ask `also_effects` on its own (EXECUTED: on b0a2dc4 this
    // read `[Env, Exec]`, no `Fs`; `CANDOR_ALIAS_DEBUG` prints `ALIASCOLLIDE … DISAGREE` for it).
    let src = "\
mod sh {\n\
#[cfg(unix)] pub use portable_pty::CommandBuilder as Cb;\n\
#[cfg(not(unix))] pub use std::process::Command as Cb;\n\
}\n\
pub fn a_alias(p: &str) { let _ = sh::Cb::new(p); }\n";
    let d = fixture("alias", src);
    let (_, v) = scan(&d, None);
    let inf = field(&v, "a_alias", "inferred");
    assert!(inf.contains(&"Env".to_string()) && inf.contains(&"Fs".to_string()), "alias route: {inf:?}\n{v:#}");
    let _ = std::fs::remove_dir_all(&d);
}
