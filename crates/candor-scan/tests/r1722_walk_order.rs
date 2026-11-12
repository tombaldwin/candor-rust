//! SOUNDNESS R1722 — the scan is a function of the TREE, not of the filesystem's directory-listing order.
//!
//! `walkdir` yields entries in `read_dir` order: APFS, ext4, overlayfs and tmpfs differ (tmpfs lists in reverse creation
//! order). A last-wins pick over same-named twins therefore answered differently per machine — on CI the legacy
//! (`CANDOR_R1657_OFF`) path picked the pure twin where the dev machine picked the effectful one.
//!
//! The fixture is built twice, creating the same files in FORWARD and in REVERSE order. On a filesystem whose listing follows
//! creation order (tmpfs: `TMPDIR=/dev/shm`) the two trees list differently, and without the sorted walk the reports differ;
//! on APFS the two orders can coincide, so this is a regression guard there and a fails-before test on Linux tmpfs.
//! `CANDOR_R1722_OFF` restores the filesystem's order (the A/B arm).

use std::path::{Path, PathBuf};
use std::process::Command;

fn bin() -> &'static str {
    env!("CARGO_BIN_EXE_candor-scan")
}

const W: &str = "std::fs::write(std::env::temp_dir().join(\"r1722\"), b\"x\").unwrap();";

fn files(a_eff: bool) -> Vec<(String, String)> {
    let (ea, eb) = if a_eff { (W, "") } else { ("", W) };
    vec![
        ("a.rs".into(), format!("pub struct EngineA;\nimpl EngineA {{ pub fn run(&self) {{ {ea} }} }}\npub struct Holder {{ pub inner: EngineA }}\npub fn make() -> Holder {{ Holder {{ inner: EngineA }} }}\n")),
        ("b.rs".into(), format!("pub struct EngineB;\nimpl EngineB {{ pub fn run(&self) {{ {eb} }} }}\npub struct Holder {{ pub inner: EngineB }}\npub fn make() -> Holder {{ Holder {{ inner: EngineB }} }}\n")),
        ("c.rs".into(), "use crate::a::make as make_a;\nuse crate::b::make as make_b;\npub fn consume_a_zq() { let h = make_a(); h.inner.run(); }\npub fn consume_b_zq() { let h = make_b(); h.inner.run(); }\n".into()),
        // the same twin shape one directory down, so the order of DIRECTORIES matters too
        ("n/p.rs".into(), format!("pub struct EngineP;\nimpl EngineP {{ pub fn go(&self) {{ {ea} }} }}\npub struct Holder2 {{ pub inner: EngineP }}\npub fn make2() -> Holder2 {{ Holder2 {{ inner: EngineP }} }}\n")),
        ("m/q.rs".into(), format!("pub struct EngineQ;\nimpl EngineQ {{ pub fn go(&self) {{ {eb} }} }}\npub struct Holder2 {{ pub inner: EngineQ }}\npub fn make2() -> Holder2 {{ Holder2 {{ inner: EngineQ }} }}\n")),
        ("n/mod.rs".into(), "pub mod p;\n".into()),
        ("m/mod.rs".into(), "pub mod q;\n".into()),
        ("d.rs".into(), "use crate::n::p::make2 as make_p;\nuse crate::m::q::make2 as make_q;\npub fn consume_p_zq() { let h = make_p(); h.inner.go(); }\npub fn consume_q_zq() { let h = make_q(); h.inner.go(); }\n".into()),
        ("main.rs".into(), "mod a;\nmod b;\nmod c;\nmod d;\nmod n;\nmod m;\nfn main() { c::consume_a_zq(); d::consume_p_zq(); }\n".into()),
    ]
}

fn build(tag: &str, reverse: bool, a_eff: bool) -> PathBuf {
    // ONE path for both orders: the report names its files, so two paths would differ for a reason unrelated to the walk.
    let d = std::env::temp_dir().join(format!("candor-r1722-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(d.join("src")).unwrap();
    std::fs::write(d.join("Cargo.toml"), "[package]\nname = \"fx\"\nversion = \"0.1.0\"\nedition = \"2021\"\n[dependencies]\n").unwrap();
    let mut fs = files(a_eff);
    if reverse {
        fs.reverse();
    }
    for (p, s) in fs {
        let f = d.join("src").join(p);
        std::fs::create_dir_all(f.parent().unwrap()).unwrap();
        std::fs::write(f, s).unwrap();
    }
    d
}

fn scan(dir: &Path, envs: &[(&str, &str)]) -> String {
    let mut cmd = Command::new(bin());
    cmd.arg(dir.to_string_lossy().as_ref()).arg("--json");
    cmd.env_remove("CANDOR_DEPS").env_remove("CANDOR_CONFIG").env_remove("CANDOR_POLICY").env_remove("CANDOR_R1657_OFF").env_remove("CANDOR_R1722_OFF");
    for (k, v) in envs {
        cmd.env(k, v);
    }
    let out = cmd.output().expect("run candor-scan");
    assert!(matches!(out.status.code(), Some(0) | Some(1)), "scan failed: {}", String::from_utf8_lossy(&out.stderr));
    String::from_utf8(out.stdout).unwrap()
}

/// Forward vs reverse creation order, under the default engine and under the legacy last-wins path: byte-identical reports.
#[test]
fn r1722_the_report_does_not_depend_on_the_directory_listing_order() {
    for a_eff in [true, false] {
        for envs in [&[][..], &[("CANDOR_R1657_OFF", "1")][..]] {
            let fwd = build("fwd", false, a_eff);
            let one = scan(&fwd, envs);
            let _ = std::fs::remove_dir_all(&fwd);
            let rev = build("fwd", true, a_eff); // same path as `fwd`, created in reverse
            let two = scan(&rev, envs);
            let _ = std::fs::remove_dir_all(&rev);
            assert_eq!(one, two, "a_eff={a_eff} envs={envs:?}: the report differs by creation order");
        }
    }
}
