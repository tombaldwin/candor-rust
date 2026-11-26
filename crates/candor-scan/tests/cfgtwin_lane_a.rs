//! cfg-twin lane A — SOUNDNESS R1862 (with R1824, R1828), R1897, R1899, R1901: an effectful member defined only in an
//! unexpanded macro arm of a `#[cfg]` module twin read ABSENT over an executed write on v0.43.0 (`5b001dbf`) and main
//! (`5308d90a`) when the arms were out-of-line (`mod x;`, `#[path]`, `cfg_attr(path)`), hand-written, `cfg_if!`-lifted, or
//! gated only through an enclosing module. The fixtures are the checked-in attack matrix (`soundness/cfgtwin/fx`, rendered by
//! `gen.py`); every `_e` probe below writes its marker when run (`run.py --exec`, recorded in `expect.tsv`'s `exec` column) and
//! every `_q` probe is pure. The `pub` and `main` columns of `expect.tsv` hold the v0.43.0 / main readings these assertions
//! differ from; the `_q` controls are the over-hold side (S1, S5′, G1, H6/H7) and the floors are what v0.43.0 already held (S2).

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::Command;

fn fx(cell: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../soundness/cfgtwin/fx").join(cell)
}

/// fn -> inferred
type Rows = BTreeMap<String, Vec<String>>;

fn scan(dir: &Path, cache: &Path, extra: &[&str]) -> Rows {
    let out = cache.join("out");
    let _ = std::fs::remove_dir_all(&out);
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_candor-scan"));
    cmd.arg(dir.to_string_lossy().as_ref()).arg("--out").arg(out.join("r")).args(extra);
    for k in ["CANDOR_DEPS", "CANDOR_CONFIG", "CANDOR_POLICY", "CANDOR_R1857_OFF", "CANDOR_I1S2_TWIN_OFF", "CANDOR_I1S2C_XMOD_OFF"] {
        cmd.env_remove(k);
    }
    cmd.env("CANDOR_CACHE_DIR", cache.join("c"));
    let o = cmd.output().expect("run candor-scan");
    assert!(o.status.code().is_some_and(|c| c == 0 || c == 1), "scan failed: {}", String::from_utf8_lossy(&o.stderr));
    let mut r = Rows::new();
    for e in std::fs::read_dir(&out).unwrap() {
        let p = e.unwrap().path();
        let n = p.file_name().unwrap().to_string_lossy().to_string();
        if n.ends_with(".json") && !n.ends_with(".callgraph.json") && !n.ends_with(".locs.json") && !n.ends_with(".hierarchy.json") {
            let v: serde_json::Value = serde_json::from_slice(&std::fs::read(&p).unwrap()).unwrap();
            for f in v["functions"].as_array().unwrap() {
                let inf = f["inferred"].as_array().map(|a| a.iter().filter_map(|e| e.as_str().map(str::to_string)).collect()).unwrap_or_default();
                r.insert(f["fn"].as_str().unwrap().to_string(), inf);
            }
        }
    }
    r
}

fn tmp(tag: &str) -> PathBuf {
    // Tests run as parallel threads of one process and several read the same cell, so the pid alone collided: one test's
    // `remove_dir_all` deleted another's `out/` mid-scan (CI, main 97b5cfb0: `read_dir` NotFound). A per-call counter
    // makes every directory unique.
    static N: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
    let n = N.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let d = std::env::temp_dir().join(format!("candor-cfgtwinA-{tag}-{}-{n}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn reading(cell: &str) -> Rows {
    scan(&fx(cell), &tmp(cell), &[])
}

fn row<'a>(r: &'a Rows, probe: &str) -> Option<&'a Vec<String>> {
    r.iter().find(|(k, _)| *k == probe || k.ends_with(&format!("::{probe}"))).map(|(_, v)| v)
}

fn unknown(r: &Rows, probe: &str) -> bool {
    row(r, probe).is_some_and(|v| v.iter().any(|e| e == "Unknown"))
}

fn effect(r: &Rows, probe: &str) -> bool {
    row(r, probe).is_some_and(|v| v.iter().any(|e| e != "Unknown" && e != "Env"))
}

#[test]
fn every_closed_cell_reads_unknown_over_its_write() {
    // Each was ABSENT on v0.43.0 and main (expect.tsv `pub`/`main` = `-`) and writes its marker when run.
    let cells = [
        // R1828 / R1862 `k_hand`: hand-written inline module arms (D2), three arms (D10), nested depth (D12), same file twice
        // with same-list twins inside it (D11, including a trait twin)
        ("d2v1", "d2v1_e"), ("d2v2", "d2v2_e"), ("d2v7", "d2v7_e"), ("d2v8", "d2v8_e"),
        ("d10v1", "d10v1_e"), ("d12v1", "d12v1_e"), ("d11v1", "d11v1_e"), ("d11v2", "d11v2_e"),
        // R1824 / R1862 `k_outline2`: an inline live arm beside an out-of-line pure arm, produced and hand-written, `mod.rs`
        ("d3v1", "d3v1_e"), ("d3v4", "d3v4_e"), ("d3hv2", "d3hv2_e"), ("d4v3", "d4v3_e"),
        // `k_handout`: out-of-line live arm, hand-written inline pure arm
        ("d5hv1", "d5hv1_e"), ("d5hv7", "d5hv7_e"),
        // R1862 `k_path`: both arms `#[path]`, produced and hand-written; `cfg_attr(path)`; cfg_if! out-of-line (R1901)
        ("d6v2", "d6v2_e"), ("d6hv3", "d6hv3_e"), ("d7v4", "d7v4_e"), ("d9v4", "d9v4_e"),
        // R1901: cfg_if! inline branches, the module only hidden text declares
        ("d8v1", "d8v1_e"), ("d8v8", "d8v8_e"),
        // R1897: an impl arm gated only through its enclosing module (the default leg), and its use sites
        ("d13", "d13_e"), ("u6d13", "u6d13_e"),
        // what the hidden text is (D2·V2): local path, `$crate`, a dependency macro, one naming nothing it is passed, a
        // token paster; R1899: a bare `thread_local!` a `use` rebinds to a dependency's macro
        ("h1", "h1_e"), ("h2", "h2_e"), ("h5", "h5_e"), ("h5b", "h5b_e"), ("h10", "h10_e"), ("h9", "h9_e"),
        // positions: impl-item and body-position (by path) text in the live arm, out-of-line pure arm
        ("p3d3", "p3d3_e"), ("p6d3", "p6d3_e"), ("p6d6", "p6d6_e"),
        // hidden text naming a different item than the probe's pure twin
        ("g2", "g2_e"),
    ];
    let mut bad = Vec::new();
    for (cell, probe) in cells {
        let r = reading(cell);
        // R1919 (lane rust-r1919): R-ROOT now expands the `crate::w*!` hidden text inside a cfg arm, so a cell whose arm was
        // hedged may now read the write itself (a resolution: the effect, with or without the hedge). Silent is the failure.
        if !unknown(&r, probe) && !effect(&r, probe) {
            bad.push(format!("{cell}/{probe}: {:?}", row(&r, probe)));
        }
    }
    assert!(bad.is_empty(), "cells still silent over an executed write: {bad:?}");
}

#[test]
fn a_module_gated_impl_keeps_its_default_effect_beside_the_hedge() {
    // E15: module-gated visible impl, effectful local default, hidden pure override. v0.43.0 read `Fs` with no `Unknown`.
    let r = reading("e15");
    assert!(unknown(&r, "e15_e") && effect(&r, "e15_e"), "e15_e: {:?}", row(&r, "e15_e"));
}

#[test]
fn the_controls_stay_pure() {
    // S5′: a hand-written inline arm beside an out-of-line sibling is covered from the sibling FILE's reading, never by
    // extending R1857's opaque stand-in, so its pure twin (named in no hidden text) stays pure. S1: one file declared twice
    // is one arm. G1: visible-visible pairs are not hedged. H6/H7: `thread_local!` / `compile_error!` define no callable.
    let pure = [
        ("d3hv2", "d3hv2_q"), ("d4v2", "d4v2_q"), ("d5hv2", "d5hv2_q"), ("d2v1", "d2v1_q"), ("d11v1", "d11v1_q"),
        ("d12v1", "d12v1_q"), ("d6v2", "d6v2_q") /* hand-made pure probe through a `#[path]` arm: R986 hedges it, see below */,
        ("g1", "g1_q"), ("g2", "g2_q"), ("h6", "h6_q"), ("h7", "h7_q"), ("h1", "h1_q"),
    ];
    let mut bad = Vec::new();
    for (cell, probe) in pure {
        let r = reading(cell);
        // d6v2_q reads `Unknown` on v0.43.0 already (R986 `ambiguous:path-redirected module`): it must not GAIN an effect.
        if effect(&r, probe) || (cell != "d6v2" && unknown(&r, probe)) {
            bad.push(format!("{cell}/{probe}: {:?}", row(&r, probe)));
        }
    }
    assert!(bad.is_empty(), "a pure control was charged: {bad:?}");
    let g1 = reading("g1");
    assert!(effect(&g1, "g1_e") && !unknown(&g1, "g1_e"), "g1_e (visible-visible) must read its effect, unhedged: {:?}", row(&g1, "g1_e"));
}

#[test]
fn the_floor_v043_held_is_kept() {
    // S2: R1857's stand-in covers a PRODUCED inline arm beside an out-of-line sibling whole; it is kept where v0.43.0 applies
    // it (D3/D5/D5p `_q`, and D5·V5, the seeded C3 for its queued removal).
    // R1919 (lane rust-r1919): `d5v5_q` and the two `_e` probes whose hidden text was `crate::w*!` are expanded by R-ROOT now:
    // the `_e`s read their write (a resolution), and `d5v5_q`, executed pure, resolves to its pure arms (removal partition C1).
    for (cell, probe) in [("d3v2", "d3v2_q"), ("d5v5", "d5v5_e"), ("d5pv2", "d5pv2_q"), ("d1v1", "d1v1_e"), ("d12pv1", "d12pv1_e")] {
        let r = reading(cell);
        assert!(unknown(&r, probe) || (probe.ends_with("_e") && effect(&r, probe)), "{cell}/{probe} lost the v0.43.0 hedge: {:?}", row(&r, probe));
    }
    let r = reading("d5v5");
    assert!(!effect(&r, "d5v5_q"), "d5v5_q is executed pure: {:?}", row(&r, "d5v5_q"));
}

#[test]
fn a_claim_from_another_file_follows_an_edit_on_a_warm_cache() {
    // G5 / Fable S7: the claim a `#[path]` arm's file makes is joined at the merge; edit that file and rescan with the SAME
    // cache, cold and `--incremental`, and the pure sibling arm must follow it both ways.
    let d = tmp("g5");
    let src = d.join("crate");
    let copy = |from: &Path, to: &Path| {
        for e in walk(from) {
            let rel = e.strip_prefix(from).unwrap();
            // `edit/` holds the state-B replacement the test copies in by hand below. Left inside the crate directory it is a file no
            // `mod` places, which the scanner still analyses as an orphan module: its unexpanded trait-item text would then be state
            // A's hidden text (R1853's member evidence is keyed on the owner's leaf), and the assertion is about THIS file's claim.
            if rel.starts_with("edit") {
                continue;
            }
            let t = to.join(rel);
            std::fs::create_dir_all(t.parent().unwrap()).unwrap();
            std::fs::copy(&e, &t).unwrap();
        }
    };
    copy(&fx("g5"), &src);
    for flags in [&[][..], &["--incremental"][..]] {
        let cache = d.join(format!("cache{}", flags.len()));
        std::fs::copy(fx("g5").join("src/live_g5.rs"), src.join("src/live_g5.rs")).unwrap();
        let a = scan(&src, &cache, flags);
        assert!(!unknown(&a, "g5_e"), "state A has no hidden text: {:?}", row(&a, "g5_e"));
        std::fs::copy(fx("g5").join("edit/live_g5.rs"), src.join("src/live_g5.rs")).unwrap();
        let b = scan(&src, &cache, flags);
        assert!(unknown(&b, "g5_e"), "state B after the edit, warm cache {flags:?}: {:?}", row(&b, "g5_e"));
        std::fs::copy(fx("g5").join("src/live_g5.rs"), src.join("src/live_g5.rs")).unwrap();
        let c = scan(&src, &cache, flags);
        assert!(!unknown(&c, "g5_e"), "back to state A, warm cache {flags:?}: a stale claim was replayed: {:?}", row(&c, "g5_e"));
    }
}

fn walk(d: &Path) -> Vec<PathBuf> {
    let mut v = Vec::new();
    for e in std::fs::read_dir(d).unwrap() {
        let p = e.unwrap().path();
        if p.is_dir() {
            v.extend(walk(&p));
        } else {
            v.push(p);
        }
    }
    v
}

#[test]
fn a_hand_written_macro_arm_keeps_expanding_beside_the_disclosure() {
    // The removal this lane's first corpus A/B found: marking a hand-written `macro_rules!` arm as an R208 twin stopped it
    // expanding in some positions, and serde_core's `IpAddr::deserialize` lost the calls (and two `dispatchesOn` keys) its
    // nested expansion made. This one-file shape does not reproduce that position (the A/B is the evidence for it); it pins
    // the requirement: the arm keeps its effect AND discloses that its twin may be hidden text.
    let d = tmp("mdef");
    let w = "::std::fs::write(::std::env::var(\"MARK\").unwrap(), b\"x\").unwrap();";
    std::fs::create_dir_all(d.join("crate/src")).unwrap();
    std::fs::write(d.join("crate/Cargo.toml"), "[package]\nname = \"mdefhand\"\nversion = \"0.1.0\"\nedition = \"2021\"\n[workspace]\n").unwrap();
    std::fs::write(
        d.join("crate/src/lib.rs"),
        format!(
            "mod mm {{ macro_rules! wmdef_ {{ ($n:ident) => {{ macro_rules! $n {{ () => {{ {w} }} }} }} }} pub(crate) use wmdef_ as wmdef; }}\n\
             #[cfg(unix)] crate::mm::wmdef! {{ kh }}\n\
             #[cfg(not(unix))] macro_rules! kh {{ () => {{ {w} }} }}\n\
             pub fn via_kh() {{ kh!() }}\n"
        ),
    )
    .unwrap();
    let r = scan(&d.join("crate"), &d, &[]);
    assert!(effect(&r, "via_kh"), "the visible arm's expansion must stay charged: {:?}", row(&r, "via_kh"));
    assert!(unknown(&r, "via_kh"), "and disclose the hidden twin: {:?}", row(&r, "via_kh"));
}

#[test]
fn a_unit_marked_from_another_files_claim_keeps_its_location() {
    // A marker pushed onto a fn with no attributes became its first token, and a `call_site` span put the unit at `:1:1`
    // (getrandom 0.2.17's `#[path]` arms in this lane's first A/B). The marked unit must keep the line it is written on.
    let d = tmp("loc");
    let w = "::std::fs::write(::std::env::var(\"MARK\").unwrap(), b\"x\").unwrap();";
    let c = d.join("crate");
    std::fs::create_dir_all(c.join("src")).unwrap();
    std::fs::write(c.join("Cargo.toml"), "[package]\nname = \"locarm\"\nversion = \"0.1.0\"\nedition = \"2021\"\n[workspace]\n").unwrap();
    std::fs::write(
        c.join("src/lib.rs"),
        format!(
            "#[macro_export] macro_rules! wfn {{ ($v:vis fn $n:ident();) => {{ $v fn $n() {{ {w} }} }} }}\n\
             #[cfg(unix)] #[path = \"live.rs\"] pub mod m;\n\
             #[cfg(not(unix))] #[path = \"pure.rs\"] pub mod m;\n\
             pub fn probe() {{ m::go(); }}\n"
        ),
    )
    .unwrap();
    std::fs::write(c.join("src/live.rs"), "crate::wfn! { pub fn go(); }\n").unwrap();
    std::fs::write(c.join("src/pure.rs"), "\n\npub fn go() {}\n").unwrap();
    let out = d.join("o");
    let o = Command::new(env!("CARGO_BIN_EXE_candor-scan"))
        .arg(&c)
        .arg("--out")
        .arg(out.join("r"))
        .env("CANDOR_CACHE_DIR", d.join("cache"))
        .env_remove("CANDOR_DEPS")
        .env_remove("CANDOR_CONFIG")
        .output()
        .unwrap();
    assert!(o.status.code().is_some_and(|c| c == 0 || c == 1));
    let mut found = false;
    for e in std::fs::read_dir(&out).unwrap() {
        let p = e.unwrap().path();
        if !p.to_string_lossy().ends_with(".scan.json") {
            continue;
        }
        let v: serde_json::Value = serde_json::from_slice(&std::fs::read(&p).unwrap()).unwrap();
        for f in v["functions"].as_array().unwrap() {
            if f["fn"].as_str() == Some("pure::go") {
                found = true;
                assert_eq!(f["loc"].as_str(), Some("src/pure.rs:3:1"), "{f}");
                assert!(f["inferred"].as_array().unwrap().iter().any(|e| e == "Unknown"), "{f}");
            }
        }
    }
    assert!(found, "pure::go must be marked from live.rs's claim");
}

#[test]
fn a_toolchain_rooted_import_keeps_the_toolchain_exclusion() {
    // R1899's guard is on the BINDING: `use std::thread_local;` (proc-macro2's fallback.rs) binds the toolchain's macro, so a
    // bare `thread_local!` there is still excluded from hidden text and a hand-written gated fn beside it is not hedged. The
    // first cut read the import as a rebinding and hedged 203 proc-macro2 rows; the matrix's H9 (`use dep::thread_local;`)
    // must still read `Unknown`, which `every_closed_cell_reads_unknown_over_its_write` pins.
    let d = tmp("tcstd");
    let c = d.join("crate");
    std::fs::create_dir_all(c.join("src")).unwrap();
    std::fs::write(c.join("Cargo.toml"), "[package]\nname = \"tcstd\"\nversion = \"0.1.0\"\nedition = \"2021\"\n[workspace]\n").unwrap();
    std::fs::write(
        c.join("src/lib.rs"),
        "macro_rules! unused_local { () => {} }\n\
         use std::thread_local;\n\
         thread_local! { static X: u8 = 0; }\n\
         #[cfg(unix)] pub fn go_tc() { let _ = std::fs::write(\"/tmp/never\", b\"x\"); }\n\
         #[cfg(not(unix))] pub fn go_tc() {}\n\
         pub fn probe_tc() { go_tc() }\n",
    )
    .unwrap();
    let r = scan(&c, &d, &[]);
    assert!(effect(&r, "probe_tc") && !unknown(&r, "probe_tc"), "probe_tc: {:?}", row(&r, "probe_tc"));
}
