//! SOUNDNESS R1715 / R1711 (c) — the two halves of one join, made consistent once R1710 resolves renamed `extern crate`s.
//!
//! R1715: a chained consumer keys its obligation-2 implementor from its OWN `use` map, so `impl serde::de::Visitor for V`
//! keyed `serde#de::Visitor::…`, while serde only re-exports serde_core's module (`pub use serde_core::{de, ..}`, written
//! inside the zero-argument `macro_rules! crate_root` its root invokes) and a producer keys the dispatch under the trait's
//! owner, `serde_core#…`. Nothing followed the facade. Published 0.41.3 joined only because serde_json's `extern crate
//! serde_core as serde;` made it publish `serde#…` by accident; once R1710 corrected that, a facade consumer's Visitor that
//! deletes a file read `deny Fs` 0 (EXECUTED in the lane, scratchpad `rustagent-r1715/fam`, 15 crates, every spelling).
//! The same gap on the CALL side: `serde::de::Deserialize::deserialize(..)` asked `serde#…` while the producer's union row
//! is `serde_core#…`.
//!
//! R1711 (c): Pass A keyed an `impl Y::de::Error for E` in a file other than the root with that file's `use` map, which the
//! root's `extern crate X as Y;` does not reach, so the producer's own implementor index said `Y#…` while every Pass B site
//! said `X#…`; the `implemented` witness that mints a path reference's key (`de::Error::invalid_type(..)`) then missed and
//! the key vanished, and the union rows published under an owner no report carries.
//!
//! Hermetic: `fac_core` (the owner), `fac` (a facade re-exporting `fac_core::de` from inside a zero-argument macro, as serde
//! does), `prod` (a producer dispatching on the owner's trait), each a `path` dependency the consumer's Cargo.lock pins.
//! `CANDOR_R1715_OFF=1` / `CANDOR_R1711C_OFF=1` restore the old reading, and every charged case must then lose its charge —
//! the proof that the fixtures reach the change.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Command;

fn bin() -> &'static str {
    env!("CARGO_BIN_EXE_candor-scan")
}

fn tmpdir(tag: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("candor-r1715-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn write(p: &Path, s: &str) {
    std::fs::create_dir_all(p.parent().unwrap()).unwrap();
    std::fs::write(p, s).unwrap();
}

fn pkg(root: &Path, name: &str, deps: &[&str], lib: &str) -> PathBuf {
    let d = root.join(name);
    let deps: String = deps.iter().map(|x| format!("{x} = {{ path = \"../{x}\" }}\n")).collect();
    write(&d.join("Cargo.toml"), &format!("[package]\nname = \"{name}\"\nversion = \"0.1.0\"\nedition = \"2021\"\n[dependencies]\n{deps}"));
    write(&d.join("src/lib.rs"), lib);
    d
}

fn lock(dir: &Path, names: &[&str]) {
    let body: String = names.iter().map(|n| format!("[[package]]\nname = \"{n}\"\nversion = \"0.1.0\"\n\n")).collect();
    write(&dir.join("Cargo.lock"), &format!("version = 4\n\n{body}"));
}

type Rows = HashMap<String, (Vec<String>, Vec<String>, Vec<String>)>;

/// Scan `dir` (chained over `deps` when given) -> `fn -> (inferred, unknownWhy, dispatchesOn)`, plus the report's own hashes.
fn scan(dir: &Path, deps: Option<&Path>, out: Option<&Path>, env: &[(&str, &str)]) -> (Rows, Vec<String>) {
    let mut cmd = Command::new(bin());
    cmd.arg(dir.to_string_lossy().as_ref());
    match out {
        Some(o) => cmd.arg("--out").arg(o),
        None => cmd.arg("--json"),
    };
    cmd.env_remove("CANDOR_DEPS").env_remove("CANDOR_CONFIG").env_remove("CANDOR_POLICY")
        .env_remove("CANDOR_R1715_OFF").env_remove("CANDOR_R1711C_OFF").env_remove("CANDOR_R1710_OFF")
        .env("CANDOR_CACHE", dir.join(".r1715-cache"));
    if let Some(d) = deps {
        cmd.env("CANDOR_DEPS", d);
    }
    for (k, v) in env {
        cmd.env(k, v);
    }
    let o = cmd.output().expect("run candor-scan");
    assert!(o.status.code().is_some_and(|c| c == 0 || c == 1), "scan failed: {}", String::from_utf8_lossy(&o.stderr));
    let _ = std::fs::remove_dir_all(dir.join(".r1715-cache"));
    let v: serde_json::Value = match out {
        Some(p) => {
            let pre = p.file_name().unwrap().to_string_lossy().to_string();
            let f = std::fs::read_dir(p.parent().unwrap()).unwrap().filter_map(Result::ok)
                .map(|e| e.path())
                .find(|e| {
                    let n = e.file_name().unwrap().to_string_lossy().to_string();
                    n.starts_with(&format!("{pre}.")) && n.ends_with(".scan.json")
                })
                .expect("report written");
            serde_json::from_slice(&std::fs::read(f).unwrap()).unwrap()
        }
        None => serde_json::from_slice(&o.stdout).expect("report json"),
    };
    let strs = |x: &serde_json::Value| -> Vec<String> {
        x.as_array().map(|a| a.iter().filter_map(|e| e.as_str().map(str::to_string)).collect()).unwrap_or_default()
    };
    let fns = v["functions"].as_array().unwrap();
    let rows = fns.iter()
        .map(|f| (f["fn"].as_str().unwrap().to_string(), (strs(&f["inferred"]), strs(&f["unknownWhy"]), strs(&f["dispatchesOn"]))))
        .collect();
    let hashes = fns.iter().filter_map(|f| f["hash"].as_str().map(str::to_string)).collect();
    (rows, hashes)
}

fn has(rows: &Rows, f: &str, e: &str) -> bool {
    rows.get(f).is_some_and(|r| r.0.iter().any(|x| x == e))
}

/// `fac_core` (owner), `fac` (facade), `prod` (producer) and the producer reports, chained in dependency order. The
/// producer's own implementor reads the ENVIRONMENT and the consumer's deletes a file, so the two joins are told apart by
/// effect (a shared effect would let the producer's union entry stand in for the consumer's lost implementor).
fn world(root: &Path) -> PathBuf {
    pkg(root, "fac_core", &[], "pub mod de {\n pub trait Visitor { fn visit(&self); }\n}\n");
    // serde's shape: the re-export sits inside a zero-argument `macro_rules!` that the root invokes.
    pkg(root, "fac", &["fac_core"], "macro_rules! crate_root {\n () => {\n  pub use fac_core::de;\n };\n}\ncrate_root!();\n");
    pkg(
        root,
        "prod",
        &["fac_core"],
        "pub fn drive<V: fac_core::de::Visitor>(v: V) { v.visit() }\n\
         pub struct Loudp;\n\
         impl fac_core::de::Visitor for Loudp { fn visit(&self) { let _ = std::env::var(\"R1715P\"); } }\n",
    );
    let reps = root.join("reps");
    std::fs::create_dir_all(&reps).unwrap();
    scan(&root.join("fac_core"), None, Some(&reps.join("fac_core")), &[]);
    scan(&root.join("prod"), Some(&reps), Some(&reps.join("prod")), &[]);
    reps
}

const CONSUMER: &str = "use fac::de::Visitor;\n\
pub struct Loud;\n\
impl Visitor for Loud { fn visit(&self) { let _ = std::fs::remove_file(\"/nonexistent/r1715c\"); } }\n\
pub fn e_impl_join() { prod::drive(Loud) }\n\
pub fn e_call_join() { fac::de::Visitor::visit(&prod::Loudp) }\n\
pub struct Quiet;\n\
impl fac_core::de::Visitor for Quiet { fn visit(&self) {} }\n";

#[test]
fn a_facade_spelled_implementor_and_call_join_the_owners_keys() {
    let root = tmpdir("facade");
    let reps = world(&root);
    let app = pkg(&root, "app", &["fac", "fac_core", "prod"], CONSUMER);
    lock(&app, &["app", "fac", "fac_core", "prod"]);

    let (on, _) = scan(&app, Some(&reps), None, &[]);
    // the implementor half: prod dispatches `fac_core#de::Visitor::visit`, the consumer wrote `fac::de::Visitor`
    assert!(has(&on, "e_impl_join", "Fs"), "facade-spelled implementor did not join: {:?}", on.get("e_impl_join"));
    // the call half: `fac::de::Visitor::visit` reaches prod's union for `fac_core#de::Visitor::visit` (prod's `Loudp`: Env)
    assert!(has(&on, "e_call_join", "Env"), "facade-spelled call did not join the owner's union: {:?}", on.get("e_call_join"));

    // reach: with the switch off both read as before (the fixture reaches the new code)
    let (off, _) = scan(&app, Some(&reps), None, &[("CANDOR_R1715_OFF", "1")]);
    assert!(!has(&off, "e_impl_join", "Fs"), "R1715 off still charged e_impl_join: {:?}", off.get("e_impl_join"));
    assert!(!has(&off, "e_call_join", "Env"), "R1715 off still charged e_call_join: {:?}", off.get("e_call_join"));
}

#[test]
fn without_a_lockfile_the_lost_join_is_hedged_with_the_members_dispatch_reason() {
    let root = tmpdir("nolock");
    let reps = world(&root);
    let app = pkg(&root, "app", &["fac", "fac_core", "prod"], CONSUMER);
    let (rows, _) = scan(&app, Some(&reps), None, &[]);
    for f in ["e_impl_join", "e_call_join"] {
        let r = rows.get(f).unwrap_or_else(|| panic!("{f} ABSENT without a lockfile: a silent purity claim"));
        assert!(r.0.iter().any(|x| x == "Unknown"), "{f}: no hedge without a lockfile: {r:?}");
        assert!(r.1.iter().any(|w| w == "dispatch:de::Visitor.visit" || w == "dispatch:Visitor.visit"),
            "{f}: the hedge does not name the lost member: {r:?}");
    }
    // reach: the hedges are this change's. The CALL half reads pure with the switch off (a silent purity claim — the
    // assertion above can fail); the implementor half's row also carries prod's own `drive` residue, so its hedge is
    // proved by the §E1 reach counter firing on that unit with the switch on and not with it off.
    let (off, _) = scan(&app, Some(&reps), None, &[("CANDOR_R1715_OFF", "1")]);
    assert!(!off.get("e_call_join").is_some_and(|r| r.0.iter().any(|x| x == "Unknown")),
        "the lockless call was hedged with R1715 off — the fixture does not reach the change: {:?}", off.get("e_call_join"));
    let blind = |env: &[(&str, &str)]| -> bool {
        let mut cmd = Command::new(bin());
        cmd.arg(app.to_string_lossy().as_ref()).arg("--json").env_remove("CANDOR_CONFIG").env_remove("CANDOR_POLICY")
            .env_remove("CANDOR_R1715_OFF").env("CANDOR_DEPS", &reps).env("CANDOR_R977_DEBUG", "1")
            .env("CANDOR_CACHE", app.join(".r1715-cache"));
        for (k, v) in env {
            cmd.env(k, v);
        }
        let o = cmd.output().expect("run candor-scan");
        let _ = std::fs::remove_dir_all(app.join(".r1715-cache"));
        String::from_utf8_lossy(&o.stderr).lines().any(|l| l.starts_with("R1715BLIND\te_impl_join\t"))
    };
    assert!(blind(&[]), "the lockless implementor join was not hedged by R1715");
    assert!(!blind(&[("CANDOR_R1715_OFF", "1")]), "R1715BLIND fired with the switch off");
}

#[test]
fn a_facade_that_does_not_reexport_the_trait_joins_nothing() {
    // `fac` declares its OWN `de::Visitor` (same leaf, same member): the readable source proves it distinct, so no join and
    // no hedge — the fabrication control.
    let root = tmpdir("own");
    let reps = world(&root);
    pkg(&root, "fac", &["fac_core"], "pub mod de {\n pub trait Visitor { fn visit(&self); }\n}\n");
    let app = pkg(&root, "app", &["fac", "fac_core", "prod"], CONSUMER);
    lock(&app, &["app", "fac", "fac_core", "prod"]);
    let (rows, _) = scan(&app, Some(&reps), None, &[]);
    assert!(!has(&rows, "e_impl_join", "Fs"), "joined a facade's OWN same-name trait: {:?}", rows.get("e_impl_join"));
    assert!(!has(&rows, "e_call_join", "Env"), "a call of the facade's OWN trait joined the owner's union: {:?}", rows.get("e_call_join"));
}

#[test]
fn a_producer_keys_its_own_implementors_under_the_renamed_extern_crates_target() {
    // R1711 (c): `extern crate fac_core as fc;` at the root, the impl and the path reference in ANOTHER file.
    let root = tmpdir("rekey");
    pkg(&root, "fac_core", &[], "pub mod de {\n pub trait Error { fn invalid(m: u8) -> Self; }\n pub trait Go { fn go(&self); }\n}\n");
    let p = pkg(&root, "rprod", &["fac_core"], "extern crate fac_core as fc;\npub mod error;\npub mod user;\npub mod cfgd;\n");
    let man = std::fs::read_to_string(p.join("Cargo.toml")).unwrap();
    write(&p.join("Cargo.toml"), &format!("{man}[features]\nnope = []\n"));
    // an INACTIVE-feature `use` (serde_json's `#[cfg(feature = "raw_value")] use serde::de::Visitor;`): the bound read
    // inside that scope must key under the target too
    write(&p.join("src/cfgd.rs"), "#[cfg(feature = \"nope\")]\nuse fc::de::Go;\n#[cfg(feature = \"nope\")]\npub fn drive<T: Go>(t: T) { t.go() }\n");
    write(&p.join("src/error.rs"), "use fc::de;\npub struct MyErr;\nimpl de::Error for MyErr { fn invalid(_m: u8) -> Self { MyErr } }\n");
    // `de::Error::invalid` with a concrete Self: the key is minted only by the crate's own implementor witness
    write(&p.join("src/user.rs"), "use fc::de;\npub fn mk() -> crate::error::MyErr { de::Error::invalid(1) }\n");
    let (on, hashes) = scan(&p, None, None, &[]);
    assert!(on.get("user::mk").is_some_and(|r| r.2.iter().any(|k| k == "fac_core#de::Error::invalid")),
        "the path reference lost its key: {:?}", on.get("user::mk"));
    assert!(hashes.iter().any(|h| h == "fac_core#de::Error::invalid"), "union row not under the owner: {hashes:?}");
    assert!(!hashes.iter().any(|h| h.starts_with("fc#")), "a union row published under the alias: {hashes:?}");
    assert!(on.get("cfgd::drive").is_some_and(|r| r.2.iter().any(|k| k == "fac_core#de::Go::go")),
        "a bound under an inactive-feature `use` keyed under the alias: {:?}", on.get("cfgd::drive"));
    let (off, hashes_off) = scan(&p, None, None, &[("CANDOR_R1711C_OFF", "1")]);
    assert!(!off.get("user::mk").is_some_and(|r| r.2.iter().any(|k| k == "fac_core#de::Error::invalid")),
        "R1711C off still keyed: {:?}", off.get("user::mk"));
    assert!(!hashes_off.iter().any(|h| h == "fac_core#de::Error::invalid"), "R1711C off still keyed the union: {hashes_off:?}");
    assert!(!off.get("cfgd::drive").is_some_and(|r| r.2.iter().any(|k| k == "fac_core#de::Go::go")),
        "R1711C off still keyed the inactive-feature bound: {:?}", off.get("cfgd::drive"));
}
