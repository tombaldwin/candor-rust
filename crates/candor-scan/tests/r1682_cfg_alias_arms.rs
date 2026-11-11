//! SOUNDNESS R1682 / R1684 — A MODULE ALIAS CHOSEN BY `#[cfg]` IS AN ARM SET IN EVERY SCOPE THAT CAN NAME IT.
//!
//! fslock 0.2.1: `#[cfg(unix)] use crate::unix as sys; #[cfg(windows)] use crate::windows as sys;` in lib.rs, then
//! `use crate::sys;` in fmt.rs. The crate root's binding travelled to other modules as ONE value — the arm written LAST —
//! so `fmt::Adapter::drop` edged only `windows::fsync`, and the `unix::fsync` rustc runs on this Mac (`Fs`) was silent.
//! The same defect in its siblings, measured over a 306-entry generated family (lane scratch `rustagent-r1682/gen`),
//! every entry EXECUTED on macOS and observed writing its file:
//!   * the root binding read by `use crate::sys`, `crate::sys::f()`, `use super::sys`, `use crate::sys::f`, a Drop body,
//!     a grandchild, a file child — edge to the windows-last arm only;
//!   * `cfg_if! { if #[cfg(unix)] { use crate::unix as sys; } else { .. } }` — `sys` unbound, every call ABSENT;
//!   * a NON-ROOT module's `#[cfg]` arm set (`pub mod plat { #[cfg(unix)] pub use crate::unix as sys; .. }`, private
//!     too, inline or file) read from any other module — ABSENT;
//!   * `use tu as sys` (a one-segment target);
//!   * R1684 — a module alias reached through a crate-LOCAL glob (`use crate::*; sys::f()`), armed or not — ABSENT.
//!
//! Each case below asserts `Fs` on the row AND the gate `deny Fs <fn>` exiting 1; a pure-arms control stays pure, and the
//! kill switches restore the old reading (proving the fixture reaches the code). Fixture fns carry unique names because
//! `deny <E> <fn>` matches by PREFIX.

use std::path::{Path, PathBuf};
use std::process::Command;

fn bin() -> &'static str {
    env!("CARGO_BIN_EXE_candor-scan")
}

fn fixture(tag: &str, files: &[(&str, String)]) -> PathBuf {
    let d = std::env::temp_dir().join(format!("candor-r1682-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(d.join("src")).unwrap();
    std::fs::write(d.join("Cargo.toml"), "[package]\nname = \"fx\"\nversion = \"0.1.0\"\nedition = \"2021\"\n[dependencies]\n").unwrap();
    for (p, s) in files {
        let f = d.join("src").join(p);
        std::fs::create_dir_all(f.parent().unwrap()).unwrap();
        std::fs::write(f, s).unwrap();
    }
    d
}

/// The report (`--json`) and the exit code of `policy` when one is given; `off` names kill switches to set.
fn scan(dir: &Path, policy: Option<&str>, off: &[&str]) -> (i32, serde_json::Value) {
    let mut cmd = Command::new(bin());
    cmd.arg(dir.to_string_lossy().as_ref()).arg("--json");
    if let Some(p) = policy {
        let pf = dir.join("policy.txt");
        std::fs::write(&pf, p).unwrap();
        cmd.arg("--policy").arg(&pf);
    }
    for v in ["CANDOR_DEPS", "CANDOR_CONFIG", "CANDOR_POLICY", "CANDOR_R1682_OFF", "CANDOR_R1684_OFF"] {
        cmd.env_remove(v);
    }
    for v in off {
        cmd.env(v, "1");
    }
    let out = cmd.output().expect("run candor-scan");
    let code = out.status.code().unwrap_or(-1);
    assert!((0..=2).contains(&code), "scan failed ({code}): {}", String::from_utf8_lossy(&out.stderr));
    (code, serde_json::from_slice(&out.stdout).expect("report json"))
}

fn inferred(v: &serde_json::Value, f: &str) -> Vec<String> {
    let row = v["functions"].as_array().unwrap().iter().find(|r| r["fn"].as_str() == Some(f));
    let mut out: Vec<String> = row
        .and_then(|r| r["inferred"].as_array())
        .map(|a| a.iter().filter_map(|e| e.as_str().map(str::to_string)).collect())
        .unwrap_or_default();
    out.sort();
    out
}

const TU: &str = "pub fn fx() { std::fs::write(std::env::temp_dir().join(\"r1682\"), b\"x\").unwrap(); }\npub fn quiet() {}\n";
const TW: &str = "pub fn fx() {}\npub fn quiet() {}\n";

/// `pair(n, unix_first)` — the two `#[cfg]` arms binding `n` to `tu` / `tw`, in the given order.
fn pair(vis: &str, n: &str, unix_first: bool) -> String {
    let u = format!("#[cfg(unix)] {vis}use crate::tu as {n};\n");
    let w = format!("#[cfg(windows)] {vis}use crate::tw as {n};\n");
    if unix_first { u + &w } else { w + &u }
}

fn family(unix_first: bool) -> Vec<(&'static str, String)> {
    let mut lib = String::from("#![allow(unused_imports)]\n#[cfg(unix)] pub mod tu;\n#[cfg(windows)] pub mod tw;\n");
    lib += &pair("", "sys", unix_first);
    lib += "#[cfg(unix)] use tu as rel;\n#[cfg(windows)] use tw as rel;\n";
    lib += if unix_first {
        "cfg_if::cfg_if! { if #[cfg(unix)] { use crate::tu as ci; } else { use crate::tw as ci; } }\n"
    } else {
        "cfg_if::cfg_if! { if #[cfg(windows)] { use crate::tw as ci; } else { use crate::tu as ci; } }\n"
    };
    lib += &format!("pub mod plat {{\n{}}}\n", pair("pub ", "sys", unix_first));
    lib += &format!("pub mod hide {{\n{}    pub mod kid {{ use super::sys; pub fn r1682_hide_kid() {{ sys::fx() }} }}\n}}\n", pair("", "sys", unix_first));
    lib += "pub mod fmt;\npub mod pfile;\n";
    lib += "pub mod inl { use super::sys; pub fn r1682_inl_super() { sys::fx() } pub fn r1682_inl_crate() { crate::sys::fx() } }\n";
    lib += "pub mod g { pub mod h { use crate::sys; pub fn r1682_grandchild() { sys::fx() } } }\n";
    lib += "pub fn r1682_root_rel() { rel::fx() }\npub fn r1682_root_ci() { ci::fx() }\n";
    lib += "pub fn r1682_plat_out() { plat::sys::fx() }\n";
    lib += "pub fn r1682_pure_ctl() { sys::quiet() }\n";
    let fmt = "use crate::sys;\npub struct Adapter;\nimpl Drop for Adapter { fn drop(&mut self) { sys::fx() } }\n\
               pub fn r1682_fmt_use() { sys::fx() }\npub fn r1682_fmt_path() { crate::sys::fx() }\n\
               pub fn r1682_fmt_plat() { crate::plat::sys::fx() }\npub fn r1682_fmt_ci() { crate::ci::fx() }\n\
               pub mod gl { use crate::*; pub fn r1682_fmt_glob() { sys::fx() } }\n\
               pub fn r1682_fmt_pure() { sys::quiet() }\n"
        .to_string();
    let pfile = format!(
        "#![allow(unused_imports)]\n{}pub mod k {{ use super::sys; pub fn r1682_pfile_kid() {{ sys::fx() }} }}\n",
        if unix_first {
            "cfg_if::cfg_if! { if #[cfg(unix)] { pub use crate::tu as sys; } else { pub use crate::tw as sys; } }\n"
        } else {
            "cfg_if::cfg_if! { if #[cfg(windows)] { pub use crate::tw as sys; } else { pub use crate::tu as sys; } }\n"
        }
    );
    vec![("lib.rs", lib), ("tu.rs", TU.to_string()), ("tw.rs", TW.to_string()), ("fmt.rs", fmt), ("pfile.rs", pfile)]
}

const CHARGED: &[&str] = &[
    "fmt::Adapter::drop",
    "fmt::r1682_fmt_use",
    "fmt::r1682_fmt_path",
    "fmt::r1682_fmt_plat",
    "fmt::r1682_fmt_ci",
    "fmt::gl::r1682_fmt_glob",
    "inl::r1682_inl_super",
    "inl::r1682_inl_crate",
    "g::h::r1682_grandchild",
    "hide::kid::r1682_hide_kid",
    "pfile::k::r1682_pfile_kid",
    "r1682_root_rel",
    "r1682_root_ci",
    "r1682_plat_out",
];

#[test]
fn r1682_every_spelling_of_a_cfg_chosen_module_alias_reaches_the_arm_this_build_runs() {
    for unix_first in [true, false] {
        let d = fixture(if unix_first { "ufirst" } else { "wfirst" }, &family(unix_first));
        let (_, v) = scan(&d, None, &[]);
        for f in CHARGED {
            assert!(inferred(&v, f).contains(&"Fs".to_string()), "unix_first={unix_first}: `{f}` must carry Fs, got {:?}", inferred(&v, f));
            let (code, _) = scan(&d, Some(&format!("deny Fs {f}\n")), &[]);
            assert_eq!(code, 1, "unix_first={unix_first}: `deny Fs {f}` must fire");
        }
        // CONTROL: a call through the same arm set to a function PURE in both arms stays pure — the union adds no effect.
        for f in ["r1682_pure_ctl", "fmt::r1682_fmt_pure"] {
            assert!(inferred(&v, f).is_empty(), "unix_first={unix_first}: `{f}` is pure in every arm, got {:?}", inferred(&v, f));
        }
    }
}

#[test]
fn r1682_the_kill_switches_restore_the_last_written_arm_reading() {
    // Windows written last: with both switches off the old reading returns and the fixture's charges vanish — the cases
    // above reach the new code rather than passing for some other reason.
    let d = fixture("off", &family(true));
    let (_, v) = scan(&d, None, &["CANDOR_R1682_OFF", "CANDOR_R1684_OFF"]);
    let silent: Vec<&&str> = CHARGED.iter().filter(|f| !inferred(&v, f).contains(&"Fs".to_string())).collect();
    for f in ["fmt::Adapter::drop", "fmt::r1682_fmt_use", "r1682_root_ci", "r1682_plat_out", "fmt::gl::r1682_fmt_glob"] {
        assert!(silent.contains(&&f), "with the switches set `{f}` reads the old (silent) answer; silent set: {silent:?}");
    }
}
