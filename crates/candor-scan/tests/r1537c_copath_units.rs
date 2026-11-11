//! SOUNDNESS R1537 (c) / R1420 (c) — SPEC §2 ⟨0.42⟩ CO-PATH UNITS: an inherent method and a trait's implementation of the
//! same name on one type are TWO units. They were one merged unit keyed `Ty::m`, published twice at the inherent's span and
//! charged the union of both bodies, so every caller of either was charged both (a fabrication EXECUTED on published
//! 0.40.16). Each fixture below was BUILT AND EXECUTED in the lane (`rustagent-r1537c/fx/cx`, one crate per cell, every
//! cell's observed body matching the expectation here): the inherent body reads `Env`, the trait body touches `Fs`, and the
//! caller `zz` must carry exactly the effect of the body that runs — or, where the site cannot prove which one runs, BOTH
//! (the union; never one picked, never neither). The `TR` cells are the silence direction: a split that picked the inherent
//! method there would drop a real `Fs`. `CANDOR_R1537C_OFF=1` restores the merge, and every cell then reads the union again
//! (the fixtures reach the code).

use std::path::{Path, PathBuf};
use std::process::Command;

fn bin() -> &'static str {
    env!("CARGO_BIN_EXE_candor-scan")
}

const INH: &str = "let _ = std::env::var(\"R1537_INH\");";
const TR: &str = "let _ = std::fs::metadata(\"/nonexistent-r1537-trait\");";

fn std_pair() -> String {
    format!("pub trait Tr {{ fn act(&self); }}\npub struct S;\nimpl S {{ pub fn act(&self) {{ {INH} }} }}\nimpl Tr for S {{ fn act(&self) {{ {TR} }} }}\n")
}

fn rank_pair() -> String {
    format!("pub trait Tr {{ fn act(&self); }}\npub struct S;\nimpl S {{ pub fn act(&mut self) {{ {INH} }} }}\nimpl Tr for S {{ fn act(&self) {{ {TR} }} }}\n")
}

fn cond_pair() -> String {
    format!("pub trait Tr {{ fn act(&self); }}\npub struct W<T>(pub T);\nimpl W<u8> {{ pub fn act(&self) {{ {INH} }} }}\nimpl<T> Tr for W<T> {{ fn act(&self) {{ {TR} }} }}\n")
}

fn private_pair(inside: &str) -> String {
    format!("pub trait Tr {{ fn act(&self); }}\npub mod inner {{ pub struct S; impl S {{ fn act(&self) {{ {INH} }} }} impl super::Tr for S {{ fn act(&self) {{ {TR} }} }} {inside} }}\npub use inner::S;\n")
}

/// (name, source, expected effects at `zz` on the candidate). "Env" = the inherent ran; "Fs" = the trait's ran; both = the
/// union (where the program runs one of them and the site cannot prove which, or where it runs both).
fn cells() -> Vec<(&'static str, String, &'static [&'static str])> {
    let p = std_pair();
    vec![
        ("dot_ref", format!("{p}pub fn zz(s: &S) {{ s.act() }}"), &["Env"]),
        ("dot_owned_local", format!("{p}pub fn zz() {{ let s = S; s.act() }}"), &["Env"]),
        ("ufcs_qself", format!("{p}pub fn zz(s: &S) {{ <S as Tr>::act(s) }}"), &["Fs"]),
        ("ufcs_trait_path", format!("{p}pub fn zz(s: S) {{ Tr::act(&s) }}"), &["Fs"]),
        ("path_inherent", format!("{p}pub fn zz(s: &S) {{ S::act(s) }}"), &["Env"]),
        ("qself_no_trait", format!("{p}pub fn zz(s: &S) {{ <S>::act(s) }}"), &["Env"]),
        (
            "self_path_in_impl",
            format!("pub trait Tr {{ fn act(&self); fn go(&self); }}\npub struct S;\nimpl S {{ pub fn act(&self) {{ {INH} }} }}\nimpl Tr for S {{ fn act(&self) {{ {TR} }} fn go(&self) {{ Self::act(self) }} }}\npub fn zz(s: &S) {{ s.go() }}"),
            &["Env"],
        ),
        ("dyn", format!("{p}pub fn zz(d: &dyn Tr) {{ d.act() }}"), &["Fs"]),
        ("generic", format!("{p}pub fn zz<T: Tr>(t: &T) {{ t.act() }}"), &["Fs"]),
        // `&mut self` inherent vs `&self` trait on an OWNED receiver: the probe's autoref step `&S` reaches the trait first.
        ("rank_owned_trait_wins", format!("{}pub fn zz(mut s: S) {{ s.act() }}", rank_pair()), &["Env", "Fs"]),
        ("rank_mutref_inh_wins", format!("{}pub fn zz(s: &mut S) {{ s.act() }}", rank_pair()), &["Env"]),
        // a PRIVATE inherent method the caller cannot see is skipped by the probe: the trait's runs (executed).
        ("private_inh_outside", format!("{}pub fn zz(s: &S) {{ s.act() }}", private_pair("")), &["Env", "Fs"]),
        ("private_inh_inside", format!("{}pub fn zz(s: &S) {{ inner::zz_in(s) }}", private_pair("pub fn zz_in(s: &S) { s.act() }")), &["Env"]),
        // an inherent block that does not apply to every instance (`impl W<u8>`): on a `W<u16>` the trait's runs.
        ("cond_inh_other_arg", format!("{}pub fn zz(w: &W<u16>) {{ w.act() }}", cond_pair()), &["Env", "Fs"]),
        ("cond_inh_same_arg", format!("{}pub fn zz(w: &W<u8>) {{ w.act() }}", cond_pair()), &["Env", "Fs"]),
        (
            "display",
            format!("pub struct S;\nimpl S {{ pub fn fmt(&self) -> u8 {{ {INH} 0 }} }}\nimpl std::fmt::Display for S {{ fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {{ {TR} write!(f, \"s\") }} }}\npub fn zz(s: &S) -> String {{ format!(\"{{}}\", s) }}"),
            &["Fs"],
        ),
        (
            "operator",
            format!("use std::ops::Add;\n#[derive(Clone, Copy)] pub struct S;\nimpl S {{ pub fn add(&self) {{ {INH} }} }}\nimpl Add for S {{ type Output = S; fn add(self, _o: S) -> S {{ {TR} S }} }}\npub fn zz(s: S) -> S {{ s + s }}"),
            &["Fs"],
        ),
        (
            "rank_ref_trait_wins",
            format!("pub trait Tr {{ fn act(&self); }}\npub struct S;\nimpl S {{ pub fn act(self) {{ {INH} }} }}\nimpl Tr for S {{ fn act(&self) {{ {TR} }} }}\npub fn zz(s: &S) {{ s.act() }}"),
            &["Env", "Fs"],
        ),
        (
            "no_trait_import_dot",
            format!("pub mod t {{ pub trait Tr {{ fn act(&self); }} }}\npub struct S;\nimpl S {{ pub fn act(&self) {{ {INH} }} }}\nimpl t::Tr for S {{ fn act(&self) {{ {TR} }} }}\npub fn zz(s: &S) {{ s.act() }}"),
            &["Env"],
        ),
        ("both_spellings", format!("{p}pub fn zz(s: &S) {{ s.act(); <S as Tr>::act(s) }}"), &["Env", "Fs"]),
        // a member that DELEGATES to its sibling through a route no resolver forms (a foreign trait's UFCS path, a same-leaf
        // twin `self`): the sibling-call rule keeps the sibling's body (num-bigint-dig `BigUint::sqrt`, font-kit `full_name`).
        (
            "delegate_foreign_ufcs",
            format!("pub struct S;\nimpl S {{ pub fn next(&mut self) -> Option<u8> {{ Iterator::next(self) }} }}\nimpl Iterator for S {{ type Item = u8; fn next(&mut self) -> Option<u8> {{ {TR} None }} }}\npub fn zz(s: &mut S) {{ s.next(); }}"),
            &["Fs"],
        ),
        (
            "delegate_twin_self",
            format!("pub trait Ld {{ fn name(&self); }}\npub mod a {{ pub struct Font; impl Font {{ pub fn name(&self) {{ {TR} }} }} impl super::Ld for Font {{ fn name(&self) {{ self.name() }} }} }}\npub mod b {{ pub struct Font; impl Font {{ pub fn name(&self) {{ }} }} impl super::Ld for Font {{ fn name(&self) {{ self.name() }} }} }}\npub fn zz(f: &a::Font) {{ Ld::name(f) }}"),
            &["Fs"],
        ),
    ]
}

fn write_crate(d: &Path, name: &str, src: &str, deps: &str) {
    let _ = std::fs::remove_dir_all(d);
    std::fs::create_dir_all(d.join("src")).unwrap();
    std::fs::write(d.join("Cargo.toml"), format!("[package]\nname = \"{name}\"\nversion = \"0.0.0\"\nedition = \"2021\"\n{deps}")).unwrap();
    std::fs::write(d.join("src/lib.rs"), format!("#![allow(unused)]\n{src}\n")).unwrap();
}

fn tmp(tag: &str) -> PathBuf {
    std::env::temp_dir().join(format!("candor-r1537c-{tag}-{}", std::process::id()))
}

/// Scan `d` writing `<out>.<crate>.scan.json` + sidecar; returns (report, sidecar).
fn scan(d: &Path, out: &Path, env: &[(&str, &str)], deps: Option<&Path>, incremental: bool) -> (serde_json::Value, serde_json::Value) {
    let mut cmd = Command::new(bin());
    cmd.arg(d.to_string_lossy().as_ref()).arg("--out").arg(out.to_string_lossy().as_ref())
        .env_remove("CANDOR_DEPS").env_remove("CANDOR_POLICY").env_remove("CANDOR_R1537C_OFF");
    if incremental {
        cmd.arg("--incremental");
    }
    if let Some(p) = deps {
        cmd.env("CANDOR_DEPS", p.to_string_lossy().as_ref());
    }
    for (k, v) in env {
        cmd.env(k, v);
    }
    let o = cmd.output().expect("run candor-scan");
    assert!(o.status.code().is_some_and(|c| c == 0 || c == 1), "scan failed: {}", String::from_utf8_lossy(&o.stderr));
    let dir = out.parent().unwrap();
    let stem = out.file_name().unwrap().to_string_lossy().to_string();
    let find = |suffix: &str| -> serde_json::Value {
        for e in std::fs::read_dir(dir).unwrap() {
            let n = e.unwrap().file_name().to_string_lossy().to_string();
            if n.starts_with(&format!("{stem}.")) && n.ends_with(suffix) && (suffix != ".scan.json" || !n.contains("callgraph")) {
                return serde_json::from_slice(&std::fs::read(dir.join(n)).unwrap()).unwrap();
            }
        }
        panic!("no {suffix} beside {}", out.display())
    };
    (find(".scan.json"), find(".scan.callgraph.json"))
}

fn effects(rep: &serde_json::Value, fn_: &str) -> Vec<String> {
    let mut v: Vec<String> = rep["functions"].as_array().unwrap().iter()
        .filter(|f| f["fn"] == fn_ && f["interfaceUnion"].as_bool() != Some(true))
        .flat_map(|f| f["inferred"].as_array().unwrap().iter().map(|e| e.as_str().unwrap().to_string()).collect::<Vec<_>>())
        .filter(|e| e == "Env" || e == "Fs")
        .collect();
    v.sort();
    v.dedup();
    v
}

fn cell_effects(name: &str, src: &str, off: bool, incremental: bool) -> Vec<String> {
    let d = tmp(&format!("{name}-{off}-{incremental}"));
    write_crate(&d, "cx", src, "");
    let out = d.join("out").join("r");
    std::fs::create_dir_all(out.parent().unwrap()).unwrap();
    let env: &[(&str, &str)] = if off { &[("CANDOR_R1537C_OFF", "1")] } else { &[] };
    if incremental {
        let _ = scan(&d, &out, env, None, true);
    }
    let (rep, _) = scan(&d, &out, env, None, incremental);
    let _ = std::fs::remove_dir_all(&d);
    effects(&rep, "zz")
}

#[test]
fn r1537c_each_caller_is_charged_the_member_that_runs_or_the_union() {
    let mut bad = Vec::new();
    for (name, src, want) in cells() {
        let got = cell_effects(name, &src, false, false);
        if got != want.iter().map(|s| s.to_string()).collect::<Vec<_>>() {
            bad.push(format!("{name}: got {got:?}, want {want:?}"));
        }
    }
    assert!(bad.is_empty(), "R1537 (c) cells:\n{}", bad.join("\n"));
}

#[test]
fn r1537c_off_restores_the_merged_union_on_every_cell() {
    // The fixtures reach the code: with the split off, every caller reads both bodies again (the pre-fix fabrication).
    for (name, src, _) in cells() {
        // the delegation cells' inherent body performs nothing of its own: their union is the trait body's `Fs`.
        let want: &[&str] = if name.starts_with("delegate_") { &["Fs"] } else { &["Env", "Fs"] };
        assert_eq!(cell_effects(name, &src, true, false), want, "{name}: CANDOR_R1537C_OFF=1 must restore the union");
    }
}

#[test]
fn r1537c_a_warm_incremental_scan_keeps_the_split() {
    // `impl_kind`/`impl_trait`/`impl_flags` and `Call::cside` ride the Pass-B cache (rev170): a warm scan must read the same.
    for (name, src, want) in cells().into_iter().filter(|(n, _, _)| matches!(*n, "dot_ref" | "ufcs_qself" | "private_inh_outside" | "operator")) {
        let got = cell_effects(name, &src, false, true);
        assert_eq!(got, want.iter().map(|s| s.to_string()).collect::<Vec<_>>(), "{name}: warm --incremental scan");
    }
}

#[test]
fn r1537c_the_report_spells_each_member_and_no_bare_entry() {
    let d = tmp("shape");
    write_crate(&d, "cx", &format!("pub mod m {{ {} }}\nuse m::{{S, Tr}};\npub fn call_pure(s: &S) {{ s.act() }}\npub fn call_eff(s: &S) {{ <S as Tr>::act(s) }}", std_pair()), "");
    let out = d.join("out").join("r");
    std::fs::create_dir_all(out.parent().unwrap()).unwrap();
    let (rep, side) = scan(&d, &out, &[], None, false);
    let fns: Vec<&str> = rep["functions"].as_array().unwrap().iter().map(|f| f["fn"].as_str().unwrap()).collect();
    assert!(!fns.contains(&"m::S::act"), "no member is spelled with the bare path: {fns:?}");
    assert_eq!(fns.iter().filter(|f| **f == "m::S::act(S)").count(), 1, "{fns:?}");
    assert_eq!(fns.iter().filter(|f| **f == "m::S::act(Tr)").count(), 1, "{fns:?}");
    let row = |n: &str| rep["functions"].as_array().unwrap().iter().find(|f| f["fn"] == n).unwrap().clone();
    assert_eq!(row("m::S::act(Tr)")["hash"], "cx#m::S::act(Tr)");
    assert_ne!(row("m::S::act(Tr)")["loc"], row("m::S::act(S)")["loc"], "each member at its own span");
    assert_eq!(effects(&rep, "m::S::act(S)"), vec!["Env"]);
    assert_eq!(effects(&rep, "m::S::act(Tr)"), vec!["Fs"]);
    // §2.2: a sidecar key per member, none under the bare path; a caller's edge names the member it reaches.
    let keys: Vec<&String> = side.as_object().unwrap().keys().collect();
    assert!(keys.iter().any(|k| *k == "m::S::act(S)") && keys.iter().any(|k| *k == "m::S::act(Tr)"), "{keys:?}");
    assert!(!keys.iter().any(|k| *k == "m::S::act"), "the bare union node is internal: {keys:?}");
    assert_eq!(side["call_pure"], serde_json::json!(["m::S::act(S)"]));
    assert_eq!(side["call_eff"], serde_json::json!(["m::S::act(Tr)"]));
    // the trait's union entry stays over the bare union node, as before the split: narrowing it to the trait members was
    // tried and exposed pre-existing silences the inherent body's reach had been covering (zerocopy's `Ptr::as_ref`
    // `native` class), so it waits for the typing that can see them. It must still carry the effectful member.
    let union = rep["functions"].as_array().unwrap().iter().find(|f| f["fn"] == "m::Tr::act" && f["interfaceUnion"] == true).unwrap();
    assert_eq!(union["inferred"], serde_json::json!(["Env", "Fs"]), "{union}");
    // §6.2 ⟨0.42⟩: a scope written before the split binds every member by the last-segment prefix; a disambiguated scope binds
    // that member only; the trait's name, which sits only inside a disambiguator, binds nothing.
    let gate = |policy: &str| -> i32 {
        let pf = d.join("p.policy");
        std::fs::write(&pf, format!("{policy}\n")).unwrap();
        let o = Command::new(bin()).arg(d.to_string_lossy().as_ref()).arg("--out").arg(d.join("g").join("r").to_string_lossy().as_ref())
            .env_remove("CANDOR_DEPS").env_remove("CANDOR_R1537C_OFF").env("CANDOR_POLICY", pf.to_string_lossy().as_ref())
            .output().unwrap();
        o.status.code().unwrap()
    };
    std::fs::create_dir_all(d.join("g")).unwrap();
    assert_eq!(gate("deny Fs m::S::act"), 1);
    assert_eq!(gate("deny Env m::S::act"), 1);
    assert_eq!(gate("deny Fs m::S::act(S)"), 0);
    assert_eq!(gate("deny Fs m::S::act(Tr)"), 1);
    // R1420 (c) at the gate: each caller fires on its own member's effect and not on the sibling's.
    assert_eq!(gate("deny Env call_pure"), 1);
    assert_eq!(gate("deny Fs call_pure"), 0);
    assert_eq!(gate("deny Fs call_eff"), 1);
    assert_eq!(gate("deny Env call_eff"), 0);
    let _ = std::fs::remove_dir_all(&d);
}

#[test]
fn r1537c_a_chained_consumer_joins_the_bare_path_to_the_union() {
    let root = tmp("chain");
    let _ = std::fs::remove_dir_all(&root);
    let lib = root.join("cplib");
    write_crate(&lib, "cplib", &format!("pub mod m {{ pub mod tm {{ pub trait Tr {{ fn act(&self); }} }}\npub struct S;\nimpl S {{ pub fn act(&self) {{ {INH} }} }}\nimpl tm::Tr for S {{ fn act(&self) {{ {TR} }} }} }}"), "");
    let app = root.join("capp");
    write_crate(&app, "capp", "use cplib::m::{S, tm::Tr};\npub fn zz_dot(s: &S) { s.act() }\npub fn zz_path(s: &S) { S::act(s) }\npub fn zz_full(s: &S) { cplib::m::S::act(s) }\npub fn zz_ufcs(s: &S) { <S as Tr>::act(s) }\npub fn zz_dyn(d: &dyn Tr) { d.act() }", "[dependencies]\ncplib = { path = \"../cplib\" }\n");
    let pout = root.join("o").join("p");
    std::fs::create_dir_all(pout.parent().unwrap()).unwrap();
    let _ = scan(&lib, &pout, &[], None, false);
    let prod = root.join("o").join("p.cplib.scan.json");
    let cout = root.join("o").join("c");
    let (rep, _) = scan(&app, &cout, &[], Some(&prod), false);
    // Every spelling joins the UNION of the members — SPEC §2 ⟨0.42⟩ permits it where the consumer cannot prove the member,
    // and a rust consumer sees neither the inherent block's applicability nor (through the trait's union entry, which still
    // unions the bare node) the trait member alone. Over-charge, never a miss.
    assert_eq!(effects(&rep, "zz_dot"), vec!["Env", "Fs"]);
    assert_eq!(effects(&rep, "zz_path"), vec!["Env", "Fs"]);
    assert_eq!(effects(&rep, "zz_full"), vec!["Env", "Fs"]);
    assert_eq!(effects(&rep, "zz_ufcs"), vec!["Env", "Fs"]);
    assert_eq!(effects(&rep, "zz_dyn"), vec!["Env", "Fs"]);
    // THE SILENCE THE BARE KEY PREVENTS (R1429's shape): without it the split producer's members answer no name the consumer
    // spells, and the path calls read ABSENT over a real `Env` read. The switch turns the consumer's bare-path join off.
    let off = root.join("o").join("coff");
    let (rep_off, _) = scan(&app, &off, &[("CANDOR_R1537C_OFF", "1")], Some(&prod), false);
    assert!(effects(&rep_off, "zz_path").is_empty() && effects(&rep_off, "zz_full").is_empty(), "the seed must be silent without the join");
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn r1537c_a_delegating_member_reaches_its_sibling() {
    // font-kit's shape: `impl Ld for Font { fn name(&self) { self.name() } }` on a type whose leaf a second module also
    // declares, so `self.name()` is refused as a same-leaf twin and forms no edge. Merged, that body WAS the inherent one;
    // split, the trait member read PURE over the inherent body it runs (executed: the write ran). The sibling-call rule
    // keeps it; `CANDOR_R1537C_OFF=1` shows the member the merged unit was.
    let src = format!("pub trait Ld {{ fn name(&self); }}\npub mod a {{ pub struct Font; impl Font {{ pub fn name(&self) {{ {TR} }} }} impl super::Ld for Font {{ fn name(&self) {{ self.name() }} }} }}\npub mod b {{ pub struct Font; impl Font {{ pub fn name(&self) {{ }} }} impl super::Ld for Font {{ fn name(&self) {{ self.name() }} }} }}");
    let d = tmp("delegate");
    write_crate(&d, "cx", &src, "");
    let out = d.join("out").join("r");
    std::fs::create_dir_all(out.parent().unwrap()).unwrap();
    let (rep, side) = scan(&d, &out, &[], None, false);
    assert_eq!(effects(&rep, "a::Font::name(Ld)"), vec!["Fs"], "the delegating trait member carries its sibling's body");
    assert_eq!(side["a::Font::name(Ld)"], serde_json::json!(["a::Font::name(Font)"]));
    assert!(effects(&rep, "b::Font::name(Ld)").is_empty(), "control: the other twin's sibling is pure");
    let _ = std::fs::remove_dir_all(&d);
}
