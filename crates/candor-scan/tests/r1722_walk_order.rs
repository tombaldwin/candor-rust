//! SOUNDNESS R1722 — the scan is a function of the TREE, not of the filesystem's directory-listing order.
//!
//! `walkdir` yields entries in `read_dir` order: APFS, ext4, overlayfs and tmpfs differ (tmpfs lists in reverse creation
//! order). A last-wins pick over same-named twins therefore answered differently per machine — on CI the legacy
//! (`CANDOR_R1657_OFF`) path picked the pure twin where the dev machine picked the effectful one.
//!
//! NOTE (R1722b): the forward/reverse-creation test below is VACUOUS on any filesystem that lists independently of creation order
//! (APFS and ext4 list in name-hash order: measured, it passes with the sort removed). The tests that carry weight are
//! `deps::r1722_ordered_walk_tests` (a scrambled 24-name directory, fails with the sort removed on APFS) and
//! `r1722b_a_dependency_source_index_reads_the_files_in_name_order` (a fixed expected value, not a comparison of two runs).
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

/// SOUNDNESS R1722b — `depsig::SrcIndex::build` (a dependency's source, read to type a projection member) walks in name order.
///
/// Two files of one dependency declare a trait `Store` whose associated-type bound names a DIFFERENT trait declaring `commit`
/// (`q.rs`: `TxA`, `b.rs`: `TxB`). `assoc_bound_declares` keeps the LAST match over the files, so the union hedge's reason
/// (`dispatch:<Tx>.commit`) names the trait of whichever file the walk yields last. Name order is `b.rs` then `q.rs`, so the
/// answer is `TxA` on every machine. The expected value is FIXED rather than compared across two runs, because on APFS the raw
/// listing is name-hash order (`q.rs` before `b.rs`, measured) and the unsorted walk answers `TxB` — the two-run comparison above
/// passes either way. The reason string is the only thing the order moves here (`Unknown` either way), so it is a
/// report-text drift, not a gate flip; the test pins it because a report that is not a function of the tree cannot be diffed.
#[test]
fn r1722b_a_dependency_source_index_reads_the_files_in_name_order() {
    let root = std::env::temp_dir().join(format!("candor-r1722b-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    let (base, prod) = (root.join("base"), root.join("prod"));
    for d in [&base, &prod] {
        std::fs::create_dir_all(d.join("src")).unwrap();
    }
    std::fs::write(base.join("Cargo.toml"), "[package]\nname = \"f1base\"\nversion = \"0.1.0\"\n").unwrap();
    // created in the order that tmpfs would list REVERSED, so the unsorted walk is wrong there too
    std::fs::write(base.join("src/b.rs"), "pub trait TxB { fn commit(self); }\npub trait Store { type T: TxB; fn begin(&self) -> Self::T; }\n").unwrap();
    std::fs::write(base.join("src/q.rs"), "pub trait TxA { fn commit(self); }\npub trait Store { type T: TxA; fn begin(&self) -> Self::T; }\n").unwrap();
    std::fs::write(base.join("src/lib.rs"), "pub mod q;\npub mod b;\n").unwrap();
    std::fs::write(prod.join("Cargo.toml"), format!("[package]\nname = \"f1prod\"\nversion = \"0.1.0\"\n[dependencies]\nf1base = {{ path = \"{}\" }}\n", base.display())).unwrap();
    std::fs::write(prod.join("src/lib.rs"), "use f1base::q::Store;\npub trait Job { fn run<S: Store>(&self, s: S); }\npub struct Doc;\nimpl Job for Doc { fn run<S: Store>(&self, s: S) { let t = s.begin(); t.commit(); } }\n").unwrap();
    let out = scan(&prod, &[]);
    let _ = std::fs::remove_dir_all(&root);
    let v: serde_json::Value = serde_json::from_str(out.trim()).expect("pure JSON report");
    let why: Vec<String> = v["functions"].as_array().unwrap().iter()
        .filter(|f| f["interfaceUnion"] == true && f["fn"] == "Job::run")
        .flat_map(|f| f["unknownWhy"].as_array().cloned().unwrap_or_default())
        .filter_map(|w| w.as_str().map(str::to_string))
        .collect();
    assert!(why.iter().any(|w| w == "dispatch:TxA.commit") && !why.iter().any(|w| w == "dispatch:TxB.commit"),
        "the hedge names the trait of the file the NAME-ORDERED walk yields last (q.rs -> TxA): {why:?}");
}
