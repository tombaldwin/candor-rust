//! SOUNDNESS R1710 — a call written through a RENAMED `extern crate` (`extern crate libc as ffi; ffi::unlink(p)`, rusqlite's
//! `libsqlite3_sys as ffi`, boring's `boring_sys as ffi`, backtrace's `std as mystd`) formed no edge and no disclosure: the
//! binding was read as a local DECLARATION, so `ffi::unlink` was anchored to `crate::ffi::unlink`, which names no unit and no
//! dependency. ABSENT on published 0.41.3 and on e412a781 under every spelling below, `deny Fs` exit 0.
//!
//! Every `s_*` module here was BUILT AND EXECUTED in the lane (scratchpad `rustagent-r1710/fam`, one crate per spelling,
//! `libc::unlink` / `std::fs::remove_file` deleting a real file, a pure twin deleting nothing); this file folds them into two
//! crates. `CANDOR_R1710_OFF=1` restores the old reading and every `s_*` row must then be empty again — the proof that the
//! fixture reaches the change. The `ctl_*` modules are the precedence controls, each EXECUTED as deleting nothing: a module
//! that declares its own `ffi`, imports another `ffi`, or GLOB-imports a module that declares one, means that `ffi` and not
//! the extern prelude's (rustc looks a glob up before the extern prelude); charging `libc::unlink` there is a fabrication.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Command;

const Z: &str = "pub fn zq(p: *const std::os::raw::c_char) { unsafe { HEAD::unlink(p); } }";

fn z(head: &str) -> String {
    Z.replace("HEAD", head)
}

fn files() -> Vec<(String, String)> {
    let mut v: Vec<(String, String)> = Vec::new();
    let mut root = String::from(
        "#![allow(unused_imports, dead_code, unused_unsafe)]\nextern crate libc as ffi;\nextern crate std as mystd;\n",
    );
    let mut add = |name: &str, body: String, v: &mut Vec<(String, String)>| {
        root.push_str(&format!("pub mod {name};\n"));
        v.push((format!("src/{name}.rs"), body));
    };
    // the extern prelude, every route a module reaches it by
    add("s_bare", z("ffi"), &mut v);
    add("s_use_crate", format!("use crate::ffi;\n{}", z("ffi")), &mut v);
    add("s_use_super", format!("use super::ffi;\n{}", z("ffi")), &mut v);
    add("s_crate_path", z("crate::ffi"), &mut v);
    add("s_super_path", z("super::ffi"), &mut v);
    add("s_use_fn", "use ffi::unlink;\npub fn zq(p: *const std::os::raw::c_char) { unsafe { unlink(p); } }".into(), &mut v);
    add("s_glob", "use ffi::*;\npub fn zq(p: *const std::os::raw::c_char) { unsafe { unlink(p); } }".into(), &mut v);
    add("s_glob_crate", "use crate::ffi::*;\npub fn zq(p: *const std::os::raw::c_char) { unsafe { unlink(p); } }".into(), &mut v);
    // re-bound in one module, reached from another (the re-export table's terminal arm)
    add("n_rebind", "pub(crate) use crate::ffi;\npub(crate) use crate::ffi as sys;".into(), &mut v);
    add("s_named_rebind", format!("use crate::n_rebind::ffi;\n{}", z("ffi")), &mut v);
    add("s_renamed_rebind", format!("use crate::n_rebind::sys;\n{}", z("sys")), &mut v);
    add("s_qual_rebind", z("crate::n_rebind::sys"), &mut v);
    // the stage-2b loss shape: a glob of a module that privately re-binds `ffi`, the call relying on the prelude
    add("s_glob_priv", format!("mod n {{ use crate::ffi; pub fn z() {{}} }}\nuse self::n::*;\n{}", z("ffi")), &mut v);
    add("s_nested", format!("pub mod b {{ {} }}", z("ffi")), &mut v);
    // a MODULE-level renamed extern: its own path, `self::`, and children (inline, file, path), and another module's path
    add("s_sub", format!("extern crate libc as ffi;\n{}", z("ffi")), &mut v);
    add("s_sub_self", format!("extern crate libc as ffi;\n{}", z("self::ffi")), &mut v);
    add("s_sub_child", format!("pub extern crate libc as ffi;\npub mod c {{ use super::ffi; {} }}\npub mod d;", z("ffi")), &mut v);
    v.push(("src/s_sub_child/d.rs".into(), z("super::ffi")));
    add("s_sub_other", z("crate::s_sub_child::ffi"), &mut v);
    // `extern crate std as mystd`, at the root and in a module
    add("s_std", "pub fn zq(p: &str) { mystd::fs::remove_file(p).unwrap(); }".into(), &mut v);
    add("s_std_sub", "extern crate std as ms;\npub fn zq(p: &str) { ms::fs::remove_file(p).unwrap(); }".into(), &mut v);
    // a body-local renamed extern
    add("s_body", "pub fn zq(p: *const std::os::raw::c_char) { extern crate libc as lc; unsafe { lc::unlink(p); } }".into(), &mut v);
    // controls: a nearer `ffi` wins over the extern prelude
    let fake = "pub mod ffi { pub unsafe fn unlink(_p: *const std::os::raw::c_char) -> i32 { 0 } }";
    add("ctl_own_mod", format!("{fake}\n{}", z("ffi")), &mut v);
    add("other", fake.to_string(), &mut v);
    add("ctl_use_other", format!("use crate::other::ffi;\n{}", z("ffi")), &mut v);
    add("ctl_glob_other", format!("use crate::other::*;\n{}", z("ffi")), &mut v);
    add("ctl_inline_glob", format!("pub mod i {{ use crate::other::*; {} }}", z("ffi")), &mut v);
    v.insert(0, ("src/lib.rs".into(), root));
    v
}

const DEFECT: &[&str] = &[
    "s_bare::zq", "s_use_crate::zq", "s_use_super::zq", "s_crate_path::zq", "s_super_path::zq", "s_use_fn::zq", "s_glob::zq",
    "s_glob_crate::zq", "s_named_rebind::zq", "s_renamed_rebind::zq", "s_qual_rebind::zq", "s_glob_priv::zq", "s_nested::b::zq",
    "s_sub::zq", "s_sub_self::zq", "s_sub_child::c::zq", "s_sub_child::d::zq", "s_sub_other::zq", "s_std::zq", "s_std_sub::zq",
    "s_body::zq",
];
const CONTROL: &[&str] = &["ctl_own_mod::zq", "ctl_use_other::zq", "ctl_glob_other::zq", "ctl_inline_glob::i::zq"];

fn fixture(tag: &str, files: &[(String, String)]) -> PathBuf {
    let d = std::env::temp_dir().join(format!("candor-r1710-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    std::fs::write(
        d.join("Cargo.toml"),
        "[package]\nname = \"r1710fx\"\nversion = \"0.1.0\"\nedition = \"2021\"\n[dependencies]\nlibc = \"0.2\"\n",
    )
    .unwrap();
    for (p, t) in files {
        let f = d.join(p);
        std::fs::create_dir_all(f.parent().unwrap()).unwrap();
        std::fs::write(f, t).unwrap();
    }
    d
}

type Rows = HashMap<String, (Vec<String>, Vec<String>)>;

fn scan(dir: &Path, off: bool) -> (Rows, serde_json::Value) {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_candor-scan"));
    cmd.arg(dir.to_string_lossy().as_ref()).arg("--json");
    for k in ["CANDOR_DEPS", "CANDOR_CONFIG", "CANDOR_POLICY", "CANDOR_R1710_OFF"] {
        cmd.env_remove(k);
    }
    if off {
        cmd.env("CANDOR_R1710_OFF", "1");
    }
    let out = cmd.output().expect("run candor-scan");
    assert!(out.status.code().is_some_and(|c| c == 0 || c == 1), "scan failed: {}", String::from_utf8_lossy(&out.stderr));
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).expect("report json");
    let strs = |x: &serde_json::Value| -> Vec<String> {
        x.as_array().map(|a| a.iter().filter_map(|e| e.as_str().map(str::to_string)).collect()).unwrap_or_default()
    };
    let rows = v["functions"]
        .as_array()
        .unwrap()
        .iter()
        .map(|f| (f["fn"].as_str().unwrap().to_string(), (strs(&f["inferred"]), strs(&f["invisible"]))))
        .collect();
    (rows, v)
}

fn inferred(rows: &Rows, f: &str) -> Vec<String> {
    rows.get(f).map(|r| r.0.clone()).unwrap_or_default()
}

#[test]
fn r1710_a_call_through_a_renamed_extern_crate_reaches_the_dependency() {
    let d = fixture("family", &files());
    let (on, v) = scan(&d, false);
    for f in DEFECT {
        assert_eq!(inferred(&on, f), vec!["Fs".to_string()], "{f}: the renamed extern crate is the dependency\n{v:#}");
    }
    for f in CONTROL {
        assert!(inferred(&on, f).is_empty(), "{f}: a nearer `ffi` shadows the extern prelude — no charge\n{v:#}");
    }
    // reach: with the switch off no defect case is charged `Fs` (each is ABSENT, except `s_glob_crate`, whose bare `unlink`
    // meets this crate's own pure `unlink` controls and reads `ambiguous:` Unknown — still no `Fs`)
    let (off, _) = scan(&d, true);
    for f in DEFECT {
        assert!(!inferred(&off, f).contains(&"Fs".to_string()), "{f}: CANDOR_R1710_OFF must restore the old reading (fixture reaches the change)");
    }
}

/// `extern crate self as me;` names THIS crate: `me::eff::go` is the local `eff::go`, never a dependency called `me` or
/// `crate` (no `invisible`, no uncovered dependency), and the pure twin stays pure.
#[test]
fn r1710_a_self_alias_names_this_crate_and_fabricates_no_dependency() {
    let src = "extern crate self as me;\n\
        pub mod eff { pub fn go(p: &str) { std::fs::remove_file(p).unwrap(); } pub fn noop(_p: &str) {} }\n\
        pub mod m;\n";
    let m = "pub fn zq(p: &str) { me::eff::go(p); }\npub fn zp(p: &str) { me::eff::noop(p); }\n\
        pub mod u { use crate::me; pub fn zq(p: &str) { me::eff::go(p); } }\n";
    let d = fixture("self", &[("src/lib.rs".into(), src.into()), ("src/m.rs".into(), m.into())]);
    let (on, v) = scan(&d, false);
    assert_eq!(inferred(&on, "m::zq"), vec!["Fs".to_string()], "{v:#}");
    assert_eq!(inferred(&on, "m::u::zq"), vec!["Fs".to_string()], "{v:#}");
    assert!(inferred(&on, "m::zp").is_empty(), "{v:#}");
    for (f, (_, inv)) in &on {
        assert!(inv.iter().all(|d| !matches!(d.as_str(), "me" | "crate" | "self")), "{f}: a self alias is no dependency: {inv:?}");
    }
    let uncovered = v["coverage"]["uncovered"].as_array().cloned().unwrap_or_default();
    assert!(
        uncovered.iter().all(|u| !matches!(u["name"].as_str(), Some("me" | "crate" | "self"))),
        "a self alias is no dependency: {uncovered:?}"
    );
}
