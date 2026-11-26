//! Lane rust-macropath-l2 — R-ROOT (SOUNDNESS R1910, R1853's root cases). Fixtures are the checked-in attack matrix
//! (`soundness/macropath/fx`, groups C, D and N); every `_e` probe writes its marker when run (`run.py --exec`) unless the
//! cell's note says it is the reversed (pure) shape, and every `_q` probe is pure.
//!
//! A macro PATH whose module part is the crate root (`crate::m!`, `$crate::m!` as transcribed, `self::m!` at the root,
//! `super^k::m!` reaching it) binds the crate's ONE ungated root-namespace macro of that name — a `#[macro_export]` (anywhere,
//! a fn body or an expansion included) or a root `use` of a local definition its module re-exports — whatever glob the root
//! holds. Body position: the root-glob route read the write as `invisible:[dep]` (b4x, b4w, b4v) or not at all (b4z, ru,
//! fb). Item / impl-item / trait-item position: the invocation was left unexpanded, so its callers read `Unknown` (or, before
//! L1(a), nothing). Everything else is refused and reads as main does. `CANDOR_R_ROOT_OFF=1` restores main.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};

fn fx(cell: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../soundness/macropath/fx").join(cell)
}

type Rows = BTreeMap<String, (Vec<String>, Vec<String>)>;

fn strs(v: &serde_json::Value) -> Vec<String> {
    v.as_array().map(|a| a.iter().filter_map(|e| e.as_str().map(str::to_string)).collect()).unwrap_or_default()
}

static N: AtomicUsize = AtomicUsize::new(0);

/// One scan in a directory of its own (test name, cell, pid and a per-process counter), removed afterwards.
fn scan_with(cell: &str, who: &str, env: &[(&str, &str)], policy: Option<&str>) -> (Rows, i32) {
    let d = std::env::temp_dir().join(format!(
        "candor-rroot-{who}-{cell}-{}-{}",
        std::process::id(),
        N.fetch_add(1, Ordering::SeqCst)
    ));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    let out = d.join("out");
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_candor-scan"));
    cmd.arg(fx(cell)).arg("--out").arg(out.join("r"));
    if let Some(p) = policy {
        std::fs::write(d.join("policy"), p).unwrap();
        cmd.arg("--policy").arg(d.join("policy"));
    }
    for k in ["CANDOR_DEPS", "CANDOR_CONFIG", "CANDOR_POLICY", "CANDOR_R_ROOT_OFF"] {
        cmd.env_remove(k);
    }
    for (k, v) in env {
        cmd.env(k, v);
    }
    cmd.env("CANDOR_CACHE_DIR", d.join("c"));
    let o = cmd.output().expect("run candor-scan");
    let rc = o.status.code().unwrap_or(-1);
    assert!(rc == 0 || rc == 1, "scan failed: {}", String::from_utf8_lossy(&o.stderr));
    let mut r = Rows::new();
    for e in std::fs::read_dir(&out).unwrap() {
        let p = e.unwrap().path();
        let n = p.file_name().unwrap().to_string_lossy().to_string();
        if n.ends_with(".json") && !n.ends_with(".callgraph.json") && !n.ends_with(".locs.json") && !n.ends_with(".hierarchy.json") {
            let v: serde_json::Value = serde_json::from_slice(&std::fs::read(&p).unwrap()).unwrap();
            for f in v["functions"].as_array().unwrap() {
                let k = f["fn"].as_str().unwrap().to_string();
                let ent = r.entry(k).or_default();
                ent.0.extend(strs(&f["inferred"]));
                ent.1.extend(strs(&f["invisible"]));
            }
        }
    }
    let _ = std::fs::remove_dir_all(&d);
    (r, rc)
}

fn scan(cell: &str, who: &str) -> Rows {
    scan_with(cell, who, &[], None).0
}

fn row<'a>(r: &'a Rows, probe: &str) -> Option<&'a (Vec<String>, Vec<String>)> {
    r.iter().find(|(k, _)| *k == probe || k.ends_with(&format!("::{probe}"))).map(|(_, v)| v)
}
fn fs(r: &Rows, p: &str) -> bool {
    row(r, p).is_some_and(|v| v.0.iter().any(|e| e == "Fs"))
}
fn unknown(r: &Rows, p: &str) -> bool {
    row(r, p).is_some_and(|v| v.0.iter().any(|e| e == "Unknown"))
}

#[test]
fn r1910_body_a_root_path_binds_the_root_export_through_any_root_glob() {
    // C2 b4x, C3 b4w (dependency glob), C4 b4z (local glob), C5 (prelude glob), C6 b4v ($crate:: through a wrapper), C7 super::,
    // C8 self:: in a root fn, C8b super::super::, C9 lim1 (local_inner_macros), C10 ru (root re-export of a local definition),
    // C11 e2g (an EMITTED export), C14 fb / C14g fbg (an export in a fn body), C18 (beside a #[macro_use] extern crate),
    // N8 (super:: from an inline module), N9 (a RENAMED root re-export).
    for cell in ["c2", "c3", "c4", "c5", "c6", "c7", "c8", "c8b", "c9", "c10", "c11", "c14", "c14g", "c18", "n8", "n9"] {
        let r = scan(cell, "body");
        let (e, q) = (format!("{cell}_e"), format!("{cell}_q"));
        assert!(fs(&r, &e), "{cell}: the write is charged: {:?}", row(&r, &e));
        assert!(!fs(&r, &q), "{cell}: the pure twin is not: {:?}", row(&r, &q));
        assert!(row(&r, &e).is_none_or(|v| v.1.is_empty()), "{cell}: no `invisible` left for the dependency: {:?}", row(&r, &e));
    }
}

#[test]
fn r1910_body_the_reversed_and_refused_shapes_charge_nothing() {
    // C3r / C6r / C7r: the root export is PURE and the glob's dependency macro writes (export beats glob). C9r: local_inner_macros
    // reaches the pure export. C9c (lim0): without the attribute `mk!` resolves at the call site, the dependency's pure one.
    // C17: `$crate` from a DEPENDENCY's macro is the dependency. N2r: the root re-exports a module that re-exports the
    // dependency. N4: a BIN root's `crate::mk!` is not the LIB root's export. N5r / N7: `self::` in a non-root module. N6:
    // `super::` from depth 2 stops at a non-root module. N1r: the f3crev2 item shape.
    for cell in ["c3r", "c6r", "c7r", "c9r", "c9c", "c17", "n2r", "n4", "n5r", "n6", "n7"] {
        let r = scan(cell, "rev");
        for p in [format!("{cell}_e"), format!("{cell}_q")] {
            assert!(!fs(&r, &p), "{cell}: {p} is pure in rustc: {:?}", row(&r, &p));
        }
    }
    let r = scan("n1r", "rev");
    for p in ["n1r_e", "n1r_in_e", "n1r_use_e"] {
        assert!(!fs(&r, p), "{p}: {:?}", row(&r, p));
    }
}

#[test]
fn r1853_item_impl_and_trait_positions_expand_a_root_path_and_charge_every_caller() {
    // D1 (no glob), D2 (dependency glob), D4 (super::), D5 (local glob), D6 (root re-export of a local definition): the path,
    // bare, and `use`-renamed callers of the minted fn. D7: an inherent method, by method call and UFCS. D8: a trait default.
    for cell in ["d1", "d2", "d4", "d5", "d6"] {
        let r = scan(cell, "item");
        for p in ["_e", "_in_e", "_use_e"] {
            let p = format!("{cell}{p}");
            assert!(fs(&r, &p), "{cell}: {p}: {:?}", row(&r, &p));
        }
        for p in ["_q", "_in_q", "_use_q"] {
            let p = format!("{cell}{p}");
            assert!(!fs(&r, &p), "{cell}: {p}: {:?}", row(&r, &p));
        }
    }
    let r = scan("d7", "item");
    assert!(fs(&r, "d7_e") && fs(&r, "d7_ufcs_e") && !fs(&r, "d7_q") && !fs(&r, "d7_ufcs_q"), "{r:?}");
    let r = scan("d8", "item");
    assert!(fs(&r, "d8_e") && fs(&r, "d8_m_e") && !fs(&r, "d8_q") && !fs(&r, "d8_m_q"), "{r:?}");
    // D3 / D7r: the root export is PURE and the dependency's same-named item macro writes: nothing is charged.
    for (cell, ps) in [("d3", &["d3_e", "d3_in_e", "d3_use_e"][..]), ("d7r", &["d7r_e", "d7r_ufcs_e"][..])] {
        let r = scan(cell, "item");
        for p in ps {
            assert!(!fs(&r, p), "{cell}: {p}: {:?}", row(&r, p));
        }
    }
}

#[test]
fn r1853_a_refused_root_path_keeps_its_hedge() {
    // D9 (cfg twins of the export), D10 (a gated invocation), D11 (`self::` through a module's `use`), D12 (`self::` whose only
    // binding is a glob chain to the dependency), N1 (f3c2: the root `use dep::mkfn;` beside a textual-only local mkfn): the
    // invocation stays hidden text and every writer caller stays `Unknown`, as on main.
    for (cell, ps) in [
        ("d9", &["d9_e", "d9_in_e", "d9_use_e"][..]),
        ("d10", &["d10_e", "d10_in_e", "d10_use_e"][..]),
        ("d11", &["d11_e", "d11_in_e", "d11_use_e"][..]),
        ("d12", &["d12_e", "d12_in_e"][..]),
        ("n1", &["n1_e", "n1_in_e", "n1_use_e"][..]),
    ] {
        let r = scan(cell, "refused");
        for p in ps {
            assert!(unknown(&r, p) || fs(&r, p), "{cell}: {p} silent: {:?}", row(&r, p));
        }
    }
}

#[test]
fn r1910_the_gate_flips_and_the_kill_switch_restores_main() {
    // `deny Fs c2_e`: exit 1 (the write is charged). Under `CANDOR_R_ROOT_OFF=1` (a result-moving switch, so the gate itself
    // refuses to answer) the row is `invisible` only, which is main's reading; the item cell is main's `Unknown`.
    let (_, rc) = scan_with("c2", "gate", &[], Some("deny Fs c2_e\n"));
    assert_eq!(rc, 1);
    let (r, _) = scan_with("c2", "gate", &[("CANDOR_R_ROOT_OFF", "1")], None);
    assert!(!fs(&r, "c2_e") && row(&r, "c2_e").is_some_and(|v| !v.1.is_empty()), "{:?}", row(&r, "c2_e"));
    let (r, _) = scan_with("d1", "gate", &[("CANDOR_R_ROOT_OFF", "1")], None);
    assert!(unknown(&r, "d1_e") && !fs(&r, "d1_e"), "{:?}", row(&r, "d1_e"));
}
