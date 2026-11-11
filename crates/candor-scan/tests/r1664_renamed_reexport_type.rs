//! SOUNDNESS R1664 — A CALL ON A TYPE SPELLED BY A RENAME RE-EXPORT MUST NOT READ ABSENT.
//!
//! `pub(crate) use segment::Tbl as RenamedTbl` is not indexed (`reexport_aliases` folds fns only), so `c.poke()` with
//! `c: &RenamedTbl` formed `RenamedTbl::poke`, named no unit, and was dropped with no edge and no `Unknown`. EXECUTED (lane
//! scratch `rustagent-floors/fx/f64`, the binary writes the marker): all three spellings of the owner — a `type` alias, a
//! crate path, an imported name — read ABSENT on published 0.41.3 and on 4df3788. Fixture fns carry `_zq`.

use std::path::{Path, PathBuf};
use std::process::Command;

fn bin() -> &'static str {
    env!("CARGO_BIN_EXE_candor-scan")
}

fn fixture(tag: &str, src: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("candor-r1664-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(d.join("src")).unwrap();
    std::fs::write(d.join("Cargo.toml"), "[package]\nname = \"fx\"\nversion = \"0.1.0\"\nedition = \"2021\"\n[dependencies]\n").unwrap();
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
    cmd.env_remove("CANDOR_DEPS").env_remove("CANDOR_CONFIG").env_remove("CANDOR_POLICY").env_remove("CANDOR_R1664_OFF");
    if off {
        cmd.env("CANDOR_R1664_OFF", "1");
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

// `user::plain_zq` calls the SAME type through its declared name: it resolves and charges, no disclosure (the control).
const SRC: &str = "pub mod cht {\n\
pub(crate) mod segment { pub struct Tbl(pub u8); impl Tbl { pub fn poke(&self) { std::fs::write(std::env::temp_dir().join(\"r1664\"), b\"x\").unwrap(); } } }\n\
pub(crate) use segment::Tbl as RenamedTbl;\n\
}\n\
pub mod user {\n\
use crate::cht::RenamedTbl;\n\
type Store = crate::cht::RenamedTbl;\n\
pub fn alias_zq(c: &Store) { c.poke(); }\n\
pub fn path_zq(c: &crate::cht::RenamedTbl) { c.poke(); }\n\
pub fn imported_zq(c: &RenamedTbl) { c.poke(); }\n\
pub fn plain_zq(c: &crate::cht::segment::Tbl) { c.poke(); }\n\
}\n";

#[test]
fn r1664_a_renamed_reexport_owner_that_names_no_unit_discloses() {
    let d = fixture("ren", SRC);
    let (_, on) = scan(&d, None, false);
    for f in ["user::alias_zq", "user::path_zq", "user::imported_zq"] {
        assert_eq!(inferred(&on, f), ["Unknown"], "{f}: {on:#}");
        assert!(why(&on, f).iter().any(|w| w.starts_with("unresolved:renamed re-export type `RenamedTbl`")), "{f}: {on:#}");
    }
    // The control: the declared spelling resolves, charges the write, and carries no such disclosure.
    assert!(inferred(&on, "user::plain_zq").contains(&"Fs".to_string()), "{on:#}");
    assert!(!why(&on, "user::plain_zq").iter().any(|w| w.contains("renamed re-export")), "{on:#}");
    let (rc, _) = scan(&d, Some("deny Unknown user::alias_zq\n"), false);
    assert_eq!(rc, 1, "the disclosure reaches the gate");
    let (_, off) = scan(&d, None, true);
    for f in ["user::alias_zq", "user::path_zq", "user::imported_zq"] {
        assert!(row(&off, f).is_none(), "CANDOR_R1664_OFF restores the ABSENT row for {f}: {off:#}");
    }
    let (refused, _) = scan(&d, Some("deny Unknown user::alias_zq\n"), true);
    assert_eq!(refused, 2);
    let _ = std::fs::remove_dir_all(&d);
}
