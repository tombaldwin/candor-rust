//! SOUNDNESS R1420(a) — a trait default called on a CONCRETE implementor is charged what it runs for
//! THAT implementor, not the union of every implementor's `self.m()`. Each fixture was BUILT AND EXECUTED in the lane
//! (`~/candor-handoff/calltarget-round3/rust/fx/`): `pure` cases perform no effect, `eff` cases write a file. The pure
//! ones read `Fs` on published 0.41.3 (fabrication); every effectful one must keep `Fs` (no new silence), including the
//! shapes a wrong narrowing would drop: an inherited effectful default, a supertrait member, a non-`self` receiver, a
//! default chained through another default, an override of the middle default, a same-leaf twin, a generic receiver.
//! `CANDOR_CT3A_OFF=1` restores the union, and the pure cases must then read `Fs` again (the fixtures reach the code).

use std::path::PathBuf;
use std::process::Command;

fn bin() -> &'static str {
    env!("CARGO_BIN_EXE_candor-scan")
}

const W: &str = "let _ = std::fs::write(\"/tmp/ct3-never-run\", b\"x\");";
const P: &str = "let _z = 1u8.wrapping_add(1);";

fn caller_effects(name: &str, src: &str, call: &str, off: bool) -> Vec<String> {
    caller_effects_in(name, src, call, off, false)
}

/// `incremental`: scan twice with `--incremental` and read the SECOND (warm) report, so the instance is minted from FnInfos
/// that went through the Pass-B cache (the `self_disp` indices round-trip as `sdx`; scan.rs debug-asserts they still point
/// at the `self.m()` calls).
fn caller_effects_in(name: &str, src: &str, call: &str, off: bool, incremental: bool) -> Vec<String> {
    let d: PathBuf = std::env::temp_dir().join(format!("candor-ct3-{name}-{}-{}-{}", std::process::id(), off, incremental));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(d.join("src")).unwrap();
    std::fs::write(d.join("Cargo.toml"), format!("[package]\nname = \"ct3{}\"\n", name.replace('_', ""))).unwrap();
    let src = src.replace("EFF", W).replace("PURE", P);
    std::fs::write(d.join("src/lib.rs"), format!("#![allow(unused)]\n{src}\npub fn zz_caller() {{ {call} }}\n")).unwrap();
    let run = || {
        let mut cmd = Command::new(bin());
        cmd.arg(d.to_string_lossy().as_ref()).arg("--json").env_remove("CANDOR_DEPS").env_remove("CANDOR_CT3A_OFF");
        if incremental {
            cmd.arg("--incremental");
        }
        if off {
            cmd.env("CANDOR_CT3A_OFF", "1");
        }
        let out = cmd.output().expect("run candor-scan");
        assert!(out.status.code().is_some_and(|c| c == 0 || c == 1), "{name}: scan failed: {}", String::from_utf8_lossy(&out.stderr));
        out
    };
    if incremental {
        let _ = run();
        assert!(d.join(".candor/cache/scan-cache.json").exists(), "{name}: the first --incremental run left no cache");
    }
    let out = run();
    let _ = std::fs::remove_dir_all(&d);
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).expect("report json");
    let row = v["functions"].as_array().unwrap().iter().find(|f| f["fn"] == "zz_caller").cloned();
    row.map(|r| r["inferred"].as_array().unwrap().iter().map(|e| e.as_str().unwrap().to_string()).collect())
        .unwrap_or_default()
}

const T: &str = "pub trait Tr { fn act(&self); fn go(&self) { self.act() } }\npub struct S; impl Tr for S { fn act(&self) { EFF } }\npub struct P; impl Tr for P { fn act(&self) { PURE } }";

fn cases() -> Vec<(&'static str, String, &'static str, bool)> {
    vec![
        ("pure", T.to_string(), "P.go();", false),
        ("eff", T.to_string(), "S.go();", true),
        ("inh_default", "pub trait Tr { fn act(&self) { EFF } fn go(&self) { self.act() } }\npub struct S; impl Tr for S { fn act(&self) { PURE } }\npub struct P; impl Tr for P {}".into(), "P.go();", true),
        ("nonself", "pub trait Tr { fn act(&self); fn go(&self, o: &dyn Tr) { self.act(); o.act() } }\npub struct S; impl Tr for S { fn act(&self) { EFF } }\npub struct P; impl Tr for P { fn act(&self) { PURE } }".into(), "P.go(&S);", true),
        ("super", "pub trait Sup { fn sm(&self); }\npub trait Tr: Sup { fn go(&self) { self.sm() } }\npub struct S; impl Sup for S { fn sm(&self) { PURE } } impl Tr for S {}\npub struct P; impl Sup for P { fn sm(&self) { EFF } } impl Tr for P {}".into(), "P.go();", true),
        ("generic", format!("{T}\npub fn h<X: Tr>(x: &X) {{ x.go() }}"), "h(&P);", true),
        ("chain_pure", "pub trait Tr { fn act(&self); fn mid(&self) { self.act() } fn go(&self) { self.mid() } }\npub struct S; impl Tr for S { fn act(&self) { EFF } }\npub struct P; impl Tr for P { fn act(&self) { PURE } }".into(), "P.go();", false),
        ("chain_eff", "pub trait Tr { fn act(&self); fn mid(&self) { self.act() } fn go(&self) { self.mid() } }\npub struct S; impl Tr for S { fn act(&self) { EFF } }\npub struct P; impl Tr for P { fn act(&self) { PURE } }".into(), "S.go();", true),
        ("override_mid", "pub trait Tr { fn act(&self); fn mid(&self) { self.act() } fn go(&self) { self.mid() } }\npub struct S; impl Tr for S { fn act(&self) { PURE } }\npub struct P; impl Tr for P { fn act(&self) { PURE } fn mid(&self) { EFF } }".into(), "P.go();", true),
        ("twin_eff", "pub trait Tr { fn act(&self); fn go(&self) { self.act() } }\npub mod m1 { pub struct B; impl super::Tr for B { fn act(&self) { EFF } } }\npub mod m2 { pub struct B; impl super::Tr for B { fn act(&self) { PURE } } }".into(), "m1::B.go();", true),
        ("mod_var_pure", "pub mod m { pub trait Tr { fn act(&self); fn go(&self) { self.act() } }\npub struct S; impl Tr for S { fn act(&self) { EFF } }\npub struct P; impl Tr for P { fn act(&self) { PURE } } }\nuse m::Tr;".into(), "let p = m::P; p.go();", false),
        ("field_pure", format!("{T}\npub struct H {{ pub p: P }}"), "let h = H { p: P }; h.p.go();", false),
        ("direct", "pub trait Tr { fn act(&self); fn go(&self) { EFF self.act() } }\npub struct S; impl Tr for S { fn act(&self) { PURE } }\npub struct P; impl Tr for P { fn act(&self) { PURE } }".into(), "P.go();", true),
        // A MODULE spelled like a primitive implementor, made by a macro the scan does not expand (prost's `encoding::bool`):
        // `bool::go(..)` is read as the primitive's inherited `Tr::go`, and only the default's union stands over the macro
        // fn's real write. A primitive/std implementor is never instantiated, so the union (and the charge) stays.
        ("prim_module", "pub trait Tr { fn act(&self); fn go(&self) { self.act() } }\nimpl Tr for bool { fn act(&self) { PURE } }\npub struct S; impl Tr for S { fn act(&self) { EFF } }\nmacro_rules! mk { ($n:ident) => { pub mod $n { pub fn go(_b: &bool) { EFF } } } }\nmk!(bool);".into(), "bool::go(&true);", true),
        ("inh_vs", "pub trait Tr { fn act(&self); fn go(&self) { self.act() } }\npub struct P; impl P { pub fn act(&self) { PURE } } impl Tr for P { fn act(&self) { EFF } }\npub struct S; impl Tr for S { fn act(&self) { PURE } }".into(), "P.go();", true),
    ]
}

#[test]
fn ct3_a_default_on_a_concrete_implementor_runs_for_that_implementor() {
    for (name, src, call, eff) in cases() {
        let on = caller_effects(name, &src, call, false);
        assert_eq!(on.contains(&"Fs".to_string()), eff,
            "{name}: zz_caller {:?} — expected Fs {} (executed ground truth)", on, if eff { "charged" } else { "absent" });
    }
}

#[test]
fn ct3_a_kill_switch_restores_the_union_on_the_pure_cases() {
    for (name, src, call, eff) in cases() {
        if eff {
            continue;
        }
        let off = caller_effects(name, &src, call, true);
        assert!(off.contains(&"Fs".to_string()),
            "{name}: with CANDOR_CT3A_OFF=1 the default's union must charge Fs (else the fixture never reached the code): {off:?}");
    }
}

#[test]
fn ct3_a_warm_cache_gives_the_same_answer() {
    for (name, src, call, eff) in cases() {
        let warm = caller_effects_in(name, &src, call, false, true);
        assert_eq!(warm.contains(&"Fs".to_string()), eff, "{name}: warm (cached) scan {warm:?} disagrees with ground truth");
    }
}

/// The instance narrows EFFECTS, never `dispatchesOn` keys: a key a sibling implementor's body put on the default's union
/// stays on a concretely typed caller (zerovec `ZeroMap2d::check_invariants` on the census A/B: the key the generic
/// `self.keys0` site really needs arrived only through that union). Over-disclosure, exactly as before the change.
#[test]
fn ct3_a_instance_keeps_the_defaults_dispatch_keys() {
    let src = "pub trait Q { fn q(&self); }\npub struct Z; impl Q for Z { fn q(&self) {} }\npub fn qq() -> &'static dyn Q { &Z }\n\
               pub trait Tr { fn act(&self); fn go(&self) { self.act() } }\n\
               pub struct S; impl Tr for S { fn act(&self) { qq().q() } }\n\
               pub struct P; impl Tr for P { fn act(&self) {} }\n\
               pub fn zz_caller() { P.go(); }\n";
    for off in [false, true] {
        let d: PathBuf = std::env::temp_dir().join(format!("candor-ct3-keys-{}-{off}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(d.join("src")).unwrap();
        std::fs::write(d.join("Cargo.toml"), "[package]\nname = \"ct3keys\"\n").unwrap();
        std::fs::write(d.join("src/lib.rs"), src).unwrap();
        let mut cmd = Command::new(bin());
        cmd.arg(d.to_string_lossy().as_ref()).arg("--json").env_remove("CANDOR_DEPS").env_remove("CANDOR_CT3A_OFF");
        if off {
            cmd.env("CANDOR_CT3A_OFF", "1");
        }
        let out = cmd.output().expect("run candor-scan");
        let _ = std::fs::remove_dir_all(&d);
        let v: serde_json::Value = serde_json::from_slice(&out.stdout).expect("report json");
        let row = v["functions"].as_array().unwrap().iter().find(|f| f["fn"] == "zz_caller").cloned().unwrap_or_default();
        let keys: Vec<String> = row["dispatchesOn"].as_array().map(|a| a.iter().map(|k| k.as_str().unwrap().to_string()).collect()).unwrap_or_default();
        assert!(keys.iter().any(|k| k.ends_with("Q::q")), "off={off}: zz_caller lost the sibling's Q::q key: {row}");
    }
}
