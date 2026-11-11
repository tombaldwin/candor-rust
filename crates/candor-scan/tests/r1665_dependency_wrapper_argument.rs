//! SOUNDNESS R1665 — A METHOD CALLED ON A DEPENDENCY WRAPPER FIELD WHOSE ARGUMENT IS A CRATE-LOCAL TYPE MUST NOT READ ABSENT.
//!
//! `self.st.push_w()` with `st: CachePadded<stack::Stack>` (`CachePadded` a dependency whose `Deref` reaches its argument): the
//! dependency declares no `push_w`, and the body that runs is THIS crate's `Stack::push_w`, which chaining the dependency cannot
//! reveal. EXECUTED (lane scratch `rustagent-floors/fx/f65d`, the binary writes the marker): `Pool::field_zq` read `[]` with
//! `invisible: [cp]` on published 0.41.3 and on 4df3788, `deny Fs` and `deny Unknown` both 0. The field spelling only; a
//! parameter's written arguments are not recorded (a named residual on the row).

use std::path::{Path, PathBuf};
use std::process::Command;

fn bin() -> &'static str {
    env!("CARGO_BIN_EXE_candor-scan")
}

fn fixture(tag: &str, src: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("candor-r1665-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(d.join("src")).unwrap();
    std::fs::create_dir_all(d.join("cp/src")).unwrap();
    std::fs::write(d.join("Cargo.toml"), "[package]\nname = \"fx\"\nversion = \"0.1.0\"\nedition = \"2021\"\n[dependencies]\ncp = { path = \"cp\" }\n").unwrap();
    std::fs::write(d.join("cp/Cargo.toml"), "[package]\nname = \"cp\"\nversion = \"0.1.0\"\nedition = \"2021\"\n").unwrap();
    std::fs::write(
        d.join("cp/src/lib.rs"),
        "pub struct CachePadded<T> { v: T }\nimpl<T> CachePadded<T> { pub fn new(v: T) -> Self { CachePadded { v } } }\nimpl<T> std::ops::Deref for CachePadded<T> { type Target = T; fn deref(&self) -> &T { &self.v } }\n",
    )
    .unwrap();
    std::fs::write(d.join("src/lib.rs"), src).unwrap();
    d
}

fn scan(dir: &Path, policy: Option<&str>, off: bool) -> (i32, serde_json::Value) {
    let mut cmd = Command::new(bin());
    cmd.arg(dir.to_string_lossy().as_ref()).arg("--json");
    if let Some(p) = policy {
        let pf = dir.join("policy.txt");
        std::fs::write(&pf, p).unwrap();
        cmd.arg("--policy").arg(&pf);
    }
    cmd.env_remove("CANDOR_DEPS").env_remove("CANDOR_CONFIG").env_remove("CANDOR_POLICY").env_remove("CANDOR_R1665_OFF");
    if off {
        cmd.env("CANDOR_R1665_OFF", "1");
    }
    let out = cmd.output().expect("run candor-scan");
    let code = out.status.code().unwrap_or(-1);
    let refused_by_switch = off && policy.is_some() && code == 2;
    assert!(code == 0 || code == 1 || refused_by_switch, "scan failed ({code}): {}", String::from_utf8_lossy(&out.stderr));
    (code, serde_json::from_slice(&out.stdout).expect("report json"))
}

fn row<'a>(v: &'a serde_json::Value, f: &str) -> Option<&'a serde_json::Value> {
    v["functions"].as_array().unwrap().iter().find(|r| r["fn"].as_str() == Some(f))
}

fn inferred(v: &serde_json::Value, f: &str) -> Vec<String> {
    row(v, f).and_then(|r| r["inferred"].as_array()).map(|a| a.iter().filter_map(|e| e.as_str().map(str::to_string)).collect()).unwrap_or_default()
}

fn why(v: &serde_json::Value, f: &str) -> Vec<String> {
    row(v, f).and_then(|r| r["unknownWhy"].as_array()).map(|a| a.iter().filter_map(|e| e.as_str().map(str::to_string)).collect()).unwrap_or_default()
}

// `push_w` writes (and a same-named pure method on `other::Other` keeps the name non-unique); `push_p` is pure on a pure type.
const SRC: &str = "use cp::CachePadded;\n\
pub mod stack { pub struct Stack; impl Stack { pub fn push_w(&self) { std::fs::write(std::env::temp_dir().join(\"r1665\"), b\"x\").unwrap(); } }\n\
pub struct StackP; impl StackP { pub fn push_p(&self) {} } }\n\
pub mod other { pub struct Other; impl Other { pub fn push_w(&self) {} } }\n\
pub struct Pool { st: CachePadded<stack::Stack>, sp: CachePadded<stack::StackP> }\n\
impl Pool {\n\
pub fn field_zq(&self) { self.st.push_w(); }\n\
pub fn pure_twin_zq(&self) { self.sp.push_p(); }\n\
}\n";

#[test]
fn r1665_a_dependency_wrapper_over_a_local_argument_discloses() {
    let d = fixture("wrap", SRC);
    let (_, on) = scan(&d, None, false);
    assert_eq!(inferred(&on, "Pool::field_zq"), ["Unknown"], "{on:#}");
    assert!(why(&on, "Pool::field_zq").iter().any(|w| w.starts_with("dispatch:untyped receiver of `push_w`")), "{on:#}");
    // The pure twin: same shape, the argument's method is pure, so no disclosure.
    assert!(!inferred(&on, "Pool::pure_twin_zq").contains(&"Unknown".to_string()), "{on:#}");
    let (rc, _) = scan(&d, Some("deny Unknown Pool::field_zq\n"), false);
    assert_eq!(rc, 1, "the disclosure reaches the gate");
    let (rc_pure, _) = scan(&d, Some("deny Unknown Pool::pure_twin_zq\n"), false);
    assert_eq!(rc_pure, 0);
    let (_, off) = scan(&d, None, true);
    assert!(!inferred(&off, "Pool::field_zq").contains(&"Unknown".to_string()), "CANDOR_R1665_OFF restores the silence {off:#}");
    let (refused, _) = scan(&d, Some("deny Unknown Pool::field_zq\n"), true);
    assert_eq!(refused, 2);
    let _ = std::fs::remove_dir_all(&d);
}
