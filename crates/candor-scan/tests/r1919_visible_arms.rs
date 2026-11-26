//! Lane rust-r1919 — SOUNDNESS R1919: cfg-twin arms that are VISIBLE (hand-written, or produced by a bare local macro the engine
//! expands, or by a `crate::w!` path R-ROOT now expands inside a cfg arm). Fixtures are the checked-in attack matrix
//! (`soundness/cfgtwin/fx`, section R and the D-cells); every `_e` probe writes its marker when run (`run.py --exec`) and every `_q`
//! probe is pure. `CANDOR_R1919_OFF=1` restores main.
//!
//! The row read these as a hidden-text cover gap. They are not: the same silences read on hand-written arms with no macro at all, so
//! the defects are in the resolvers that read two VISIBLE arms:
//!   r1 an alias arm naming a crate-ROOT fn; r2 a gated override beside an impl that inherits the default; r3 `macro_rules!` twins
//!   gated through their module; r4 two default bodies of one trait leaf; r5 a produced arm's binding deleted by an out-of-line
//!   stand-in; and R-ROOT's refusal inside cfg arms, lifted.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};

fn fx(cell: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../soundness/cfgtwin/fx").join(cell)
}

type Rows = BTreeMap<String, Vec<String>>;

static N: AtomicUsize = AtomicUsize::new(0);

/// One scan in a directory of its own (test name, cell, pid and a per-process counter), removed afterwards.
fn scan_with(cell: &str, who: &str, env: &[(&str, &str)]) -> Rows {
    let d = std::env::temp_dir().join(format!(
        "candor-r1919-{who}-{cell}-{}-{}",
        std::process::id(),
        N.fetch_add(1, Ordering::SeqCst)
    ));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    let out = d.join("out");
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_candor-scan"));
    cmd.arg(fx(cell)).arg("--out").arg(out.join("r"));
    for k in ["CANDOR_DEPS", "CANDOR_CONFIG", "CANDOR_POLICY", "CANDOR_R1919_OFF", "CANDOR_R_ROOT_OFF"] {
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
                let inf = f["inferred"].as_array().map(|a| a.iter().filter_map(|e| e.as_str().map(str::to_string)).collect::<Vec<_>>());
                r.entry(k).or_default().extend(inf.unwrap_or_default());
            }
        }
    }
    let _ = std::fs::remove_dir_all(&d);
    r
}

fn effects<'a>(r: &'a Rows, probe: &str) -> &'a [String] {
    r.iter().find(|(k, _)| *k == probe || k.ends_with(&format!("::{probe}"))).map(|(_, v)| v.as_slice()).unwrap_or(&[])
}
fn fs(r: &Rows, p: &str) -> bool {
    effects(r, p).iter().any(|e| e == "Fs")
}
fn unknown(r: &Rows, p: &str) -> bool {
    effects(r, p).iter().any(|e| e == "Unknown")
}
fn pure(r: &Rows, p: &str) -> bool {
    effects(r, p).is_empty()
}

const OFF: &[(&str, &str)] = &[("CANDOR_R1919_OFF", "1")];

#[test]
fn r1919_an_alias_arm_naming_a_crate_root_fn_is_edged_in_every_spelling() {
    let r = scan_with("r1", "r1", &[]);
    for p in [
        "r1_tw_e", "r1_rev_e", "r1_in_e", "r1_three_e", "r1_crate_e", "r1_bodyuse_e", "r1_bodyrel_e", "r1_moduse_e", "r1_fnptr_e",
        "r1_reexp_e", "r1_reexp2_e", "r1_sole_e", "r1_p_e",
    ] {
        assert!(fs(&r, p), "{p}: an executed write through a root-fn alias arm must read Fs: {:?}", effects(&r, p));
    }
    assert!(pure(&r, "r1_p_q"), "r1_p_q is pure: {:?}", effects(&r, "r1_p_q"));
    // falsified: main read every one of them ABSENT
    let off = scan_with("r1", "r1off", OFF);
    for p in ["r1_tw_e", "r1_rev_e", "r1_in_e", "r1_reexp_e", "r1_sole_e", "r1_p_e"] {
        assert!(!fs(&off, p), "{p}: the switch must restore main's silence (else this test cannot fail): {:?}", effects(&off, p));
    }
}

#[test]
fn r1919_a_gated_override_beside_an_inheriting_impl_unions_the_default() {
    let r = scan_with("r2", "r2", &[]);
    for p in [
        "r2_meth_e", "r2_path_e", "r2_trpath_e", "r2_ufcs_e", "r2_dyn_e", "r2_gen_e", "r2_ref_e", "r2_rev_e", "r2_gmod_e", "r2_member_e",
        "r2_gentr_e", "r2_p_e",
    ] {
        assert!(fs(&r, p), "{p}: the inherited default's write must reach the caller: {:?}", effects(&r, p));
    }
    // controls: a pure default, two overriding arms (block and member), never the default's effect
    for p in ["r2_pure_q", "r2_bothov_q", "r2_bothmem_q"] {
        assert!(pure(&r, p), "{p}: no arm inherits an effectful default: {:?}", effects(&r, p));
    }
    let off = scan_with("r2", "r2off", OFF);
    for p in ["r2_meth_e", "r2_member_e", "r2_p_e"] {
        assert!(!fs(&off, p), "{p}: falsified on the switch: {:?}", effects(&off, p));
    }
}

#[test]
fn r1919_module_gated_macro_rules_twins_disclose() {
    let r = scan_with("r3", "r3", &[]);
    for p in ["r3_inline_e", "r3_rev_e", "r3_nested_e", "r3_p_e", "r3_child_e", "r3_export_e"] {
        assert!(unknown(&r, p) || fs(&r, p), "{p}: a twinned macro over an executed write must disclose: {:?}", effects(&r, p));
        assert!(unknown(&r, p), "{p}: the pair is an R208 twin and discloses `ambiguous:`: {:?}", effects(&r, p));
    }
    assert!(pure(&r, "r3_same_q"), "two identical templates are one answer: {:?}", effects(&r, "r3_same_q"));
    let off = scan_with("r3", "r3off", OFF);
    assert!(!unknown(&off, "r3_inline_e") && !fs(&off, "r3_inline_e"), "falsified: main read r3_inline_e ABSENT: {:?}", effects(&off, "r3_inline_e"));
}

#[test]
fn r1919_two_default_bodies_of_one_trait_leaf_disclose_unless_every_claimant_is_pure() {
    let r = scan_with("r4", "r4", &[]);
    for p in ["r4_meth_e", "r4_path_e", "r4_pmeth_e", "r4_p_e"] {
        assert!(unknown(&r, p) || fs(&r, p), "{p}: two default claimants over an executed write must not read ABSENT: {:?}", effects(&r, p));
    }
    assert!(pure(&r, "r4_unrel_q"), "two pure claimants cost nothing: {:?}", effects(&r, "r4_unrel_q"));
    let off = scan_with("r4", "r4off", OFF);
    assert!(pure(&off, "r4_meth_e"), "falsified: main read r4_meth_e ABSENT: {:?}", effects(&off, "r4_meth_e"));
    // (two pure `#[path]` defaults staying pure is the cfg-twin matrix's `g5` state A, scanned there without its `edit/` file)
}

#[test]
fn r1919_a_produced_arm_beside_an_out_of_line_arm_keeps_its_binding() {
    let r = scan_with("r5", "r5", &[]);
    assert!(fs(&r, "r5_h_e"), "hand-written inline + out-of-line: {:?}", effects(&r, "r5_h_e"));
    assert!(fs(&r, "r5_p_e"), "produced inline + out-of-line: {:?}", effects(&r, "r5_p_e"));
    assert!(pure(&r, "r5_p_q"), "r5_p_q is pure: {:?}", effects(&r, "r5_p_q"));
    // d3v5: `crate::wuse!` in the inline arm, now expanded by R-ROOT: must not read lower than main's `Unknown`
    let d = scan_with("d3v5", "d3v5", &[]);
    assert!(fs(&d, "d3v5_e"), "d3v5_e: {:?}", effects(&d, "d3v5_e"));
    let off = scan_with("r5", "r5off", OFF);
    assert!(!fs(&off, "r5_h_e"), "falsified: main read r5_h_e ABSENT: {:?}", effects(&off, "r5_h_e"));
}

#[test]
fn r1919_r_root_expands_inside_a_cfg_arm_and_reads_no_lower() {
    // the cells R-ROOT refused for this lane (`soundness/cfgtwin` d*v5, d*v7, d12*): expanded now, every write still read
    for (cell, p) in [("d1v5", "d1v5_e"), ("d10v5", "d10v5_e"), ("d3hv5", "d3hv5_e"), ("d5pv5", "d5pv5_e")] {
        let r = scan_with(cell, cell, &[]);
        assert!(fs(&r, p), "{p}: the expanded `crate::wuse!` arm is a resolution: {:?}", effects(&r, p));
        let q = p.replace("_e", "_q");
        assert!(pure(&r, &q), "{q} is pure: {:?}", effects(&r, &q));
    }
    for (cell, p) in [("d1v7", "d1v7_e"), ("d12v2", "d12v2_e"), ("d12pv2", "d12pv2_e"), ("d12pv7", "d12pv7_e")] {
        let r = scan_with(cell, cell, &[]);
        assert!(unknown(&r, p) || fs(&r, p), "{p}: expanded, the arm must still disclose: {:?}", effects(&r, p));
    }
    // OFF restores the refusal: the arm stays hidden text and the cover holds it (`Unknown`, as on main)
    let off = scan_with("d1v5", "d1v5off", OFF);
    assert!(unknown(&off, "d1v5_e") && !fs(&off, "d1v5_e"), "OFF is main's hold: {:?}", effects(&off, "d1v5_e"));
}

#[test]
fn r1919_the_inheriting_arm_is_the_same_impl_by_identity_not_by_leaf() {
    // Fable's merge review D1: a pure gated override was edged to the default because a DIFFERENT impl with the same leaves
    // inherited it. Each `_q` executes pure; each must read pure.
    for (cell, p) in [
        ("r7leaf", "f_r2leaf_q"), ("r7gen", "f_r2gen_q"), ("r7leafcfg", "f_r2leafcfg_q"), ("r7trleaf", "f_r2trleaf_q2"),
        ("r7memcol", "f_r2memcol_q"), ("r7samecfg", "f_r2samecfg_q"), ("r7samecfg", "f_r2samecfg_q2"),
    ] {
        let r = scan_with(cell, cell, &[]);
        assert!(pure(&r, p), "{cell}/{p}: executed pure, must not be charged or hedged: {:?}", effects(&r, p));
    }
    for (cell, p) in [("r7leaf", "f_r2leaf_e"), ("r7leafcfg", "f_r2leafcfg_e"), ("r7samecfg", "f_r2samecfg_e")] {
        let r = scan_with(cell, cell, &[]);
        assert!(fs(&r, p), "{cell}/{p}: {:?}", effects(&r, p));
    }
    // the positive control: one impl spelled three ways (imported name, `crate::`, `super::`) keeps its inheriting edge
    let r = scan_with("r7idmatch", "r7idmatch", &[]);
    for p in ["r7idmatch_e", "r7idmatch_sup_e"] {
        assert!(fs(&r, p), "{p}: one identity across spellings must keep the edge: {:?}", effects(&r, p));
    }
    let off = scan_with("r7idmatch", "r7idmatchoff", OFF);
    assert!(!fs(&off, "r7idmatch_e"), "falsified on the switch: {:?}", effects(&off, "r7idmatch_e"));
}
