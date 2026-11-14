//! SOUNDNESS R1697 / R1702 / R1736 / R1743 — four places where a name reached through a re-export or re-binding named
//! nothing and the call or the join was SILENT.
//!
//! * R1697 — a one-segment name through a CHAIN of private globs (`executor { mod consolidate { fn quad_zz } use
//!   consolidate::*; mod select { use super::quad_zz; mod four { use super::*; .. quad_zz(t) } } }`): ABSENT.
//! * R1702 — a dependency item re-bound by `pub(crate) use crate::X` (X from a root `pub use dep::*` / `pub use dep::{X}`),
//!   reached from another module (`crate::visit::go()`, `use crate::visit::go as g`, `use crate::visit::*`): ABSENT.
//! * R1736 — the facade resolver read only the facade's root file and inline modules; a re-export in a `mod x;` FILE
//!   module was neither followed nor hedged, so a consumer's `impl fac::body::Body` missed the owner's key WITH a lock.
//! * R1743 — a producer keying under the FACADE (`fac#de::Visitor::visit`) was not joined by a consumer implementing the
//!   OWNER's trait (`impl fac_core::de::Visitor`) that does not depend on the facade.
//!
//! Every effectful case is asserted by `deny Fs <fn>` exiting 1, and loses that with its kill switch
//! (`CANDOR_R1697_OFF`, `CANDOR_R1736_OFF`, `CANDOR_R1743_OFF`) — the proof the fixture reaches the change. The negative
//! cases pin the refusals: a name a module might bind itself (an item macro) or that two globs supply is never sent to a
//! dependency or guessed. Fixture fns carry unique `_zq` names because `deny <E> <fn>` matches by PREFIX.

use std::path::{Path, PathBuf};
use std::process::Command;

fn bin() -> &'static str {
    env!("CARGO_BIN_EXE_candor-scan")
}

fn tmpdir(tag: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("candor-r1697-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn write(p: &Path, s: &str) {
    std::fs::create_dir_all(p.parent().unwrap()).unwrap();
    std::fs::write(p, s).unwrap();
}

/// A package `name` under `root` with path dependencies `deps` and the given `src/` files.
fn pkg(root: &Path, name: &str, deps: &[&str], files: &[(&str, &str)]) -> PathBuf {
    let d = root.join(name);
    let deps: String = deps.iter().map(|x| format!("{x} = {{ path = \"../{x}\" }}\n")).collect();
    write(&d.join("Cargo.toml"), &format!("[package]\nname = \"{name}\"\nversion = \"0.1.0\"\nedition = \"2021\"\n[dependencies]\n{deps}"));
    for (p, s) in files {
        write(&d.join("src").join(p), s);
    }
    d
}

fn lock(dir: &Path, names: &[&str]) {
    let body: String = names.iter().map(|n| format!("[[package]]\nname = \"{n}\"\nversion = \"0.1.0\"\n\n")).collect();
    write(&dir.join("Cargo.lock"), &format!("version = 4\n\n{body}"));
}

const SWITCHES: [&str; 7] = [
    "CANDOR_DEPS",
    "CANDOR_CONFIG",
    "CANDOR_POLICY",
    "CANDOR_R1697_OFF",
    "CANDOR_R1736_OFF",
    "CANDOR_R1743_OFF",
    "CANDOR_R1657S2_OFF",
];

/// Write `dir`'s report to `out` (a `CANDOR_DEPS` prefix), chained over `deps`.
fn produce(dir: &Path, deps: Option<&Path>, out: &Path) {
    let mut cmd = Command::new(bin());
    cmd.arg(dir.to_string_lossy().as_ref()).arg("--out").arg(out);
    for v in SWITCHES {
        cmd.env_remove(v);
    }
    if let Some(d) = deps {
        cmd.env("CANDOR_DEPS", d);
    }
    let o = cmd.output().expect("run candor-scan");
    assert!(o.status.code().is_some_and(|c| (0..=2).contains(&c)), "{}", String::from_utf8_lossy(&o.stderr));
}

/// `deny Fs <f>` over `dir` chained on `deps`, with `env` set: the exit code.
fn deny_fs(dir: &Path, f: &str, deps: Option<&Path>, env: &[(&str, &str)]) -> i32 {
    let pol = dir.join(format!("policy-{f}.txt"));
    std::fs::write(&pol, format!("deny Fs {f}\n")).unwrap();
    let mut cmd = Command::new(bin());
    cmd.arg(dir.to_string_lossy().as_ref()).arg("--json").arg("--policy").arg(&pol);
    for v in SWITCHES {
        cmd.env_remove(v);
    }
    if let Some(d) = deps {
        cmd.env("CANDOR_DEPS", d);
    }
    for (k, v) in env {
        cmd.env(k, v);
    }
    let o = cmd.output().expect("run candor-scan");
    let c = o.status.code().unwrap_or(-1);
    assert!(c == 0 || c == 1, "scan failed ({c}): {}", String::from_utf8_lossy(&o.stderr));
    c
}

/// The `inferred` effects of `f`'s row over `dir` chained on `deps`, with `env` set (no policy: a result-moving switch
/// makes a GATE refuse, so the switched arm is read from the report). An absent row is no effects.
fn infers(dir: &Path, f: &str, deps: Option<&Path>, env: &[(&str, &str)]) -> Vec<String> {
    let mut cmd = Command::new(bin());
    cmd.arg(dir.to_string_lossy().as_ref()).arg("--json");
    for v in SWITCHES {
        cmd.env_remove(v);
    }
    if let Some(d) = deps {
        cmd.env("CANDOR_DEPS", d);
    }
    for (k, v) in env {
        cmd.env(k, v);
    }
    let o = cmd.output().expect("run candor-scan");
    let v: serde_json::Value = serde_json::from_slice(&o.stdout).expect("report json");
    v["functions"].as_array().unwrap().iter().find(|r| r["fn"] == f)
        .and_then(|r| r["inferred"].as_array())
        .map(|a| a.iter().filter_map(|e| e.as_str().map(str::to_string)).collect())
        .unwrap_or_default()
}

fn off_loses_fs(dir: &Path, f: &str, deps: Option<&Path>, switch: &str) -> bool {
    infers(dir, f, deps, &[]).iter().any(|e| e == "Fs") && !infers(dir, f, deps, &[(switch, "1")]).iter().any(|e| e == "Fs")
}

#[test]
fn a_one_segment_name_through_a_chain_of_private_globs_is_charged() {
    let root = tmpdir("chain");
    let d = pkg(
        &root,
        "chain",
        &[],
        &[(
            "lib.rs",
            "pub mod executor {\n\
             mod consolidate { pub(super) fn quad_zq(t: &str) { std::fs::write(t, b\"x\").unwrap(); } }\n\
             use consolidate::*;\n\
             pub mod select {\n  use super::quad_zq;\n\
              pub mod four { use super::*; pub fn all_zq(t: &str) { quad_zq(t) } }\n }\n}\n",
        )],
    );
    assert_eq!(deny_fs(&d, "executor::select::four::all_zq", None, &[]), 1);
    assert!(off_loses_fs(&d, "executor::select::four::all_zq", None, "CANDOR_R1697_OFF"), "the fixture must reach R1697");
}

/// The R1702 world: `dgdep` (a dependency with an effectful `go` and `Datetime::parse`) and its report.
fn dep_world(root: &Path) -> PathBuf {
    let dep = pkg(
        root,
        "dgdep",
        &[],
        &[(
            "lib.rs",
            "fn w(t: &str) { std::fs::write(t, b\"x\").unwrap(); }\n\
             pub struct Datetime;\n\
             impl Datetime { pub fn parse(t: &str) -> Datetime { w(t); Datetime } pub fn inert(_t: &str) -> Datetime { Datetime } }\n\
             pub fn go(t: &str) { w(t) }\npub fn quiet(_t: &str) {}\n\
             #[macro_export]\nmacro_rules! mk { () => { pub fn go(_t: &str) {} }; }\n\
             pub mod digest { pub fn sum(t: &str) { super::w(t) } }\n",
        )],
    );
    let reps = root.join("reps");
    std::fs::create_dir_all(&reps).unwrap();
    produce(&dep, None, &reps.join("dgdep"));
    reps
}

const CONSUMERS: &str = "pub mod visit {\n pub(crate) use crate::Datetime;\n pub(crate) use crate::go;\n pub(crate) use crate::quiet;\n}\n\
pub mod other {\n\
 pub fn e_o_assoc_zq(t: &str) { let _ = crate::visit::Datetime::parse(t); }\n\
 pub fn p_o_assoc_zq(t: &str) { let _ = crate::visit::Datetime::inert(t); }\n\
 pub fn e_o_fn_zq(t: &str) { crate::visit::go(t) }\n\
 pub fn p_o_fn_zq(t: &str) { crate::visit::quiet(t) }\n}\n\
pub mod named { use crate::visit::go as g; pub fn e_n_fn_zq(t: &str) { g(t) } }\n\
pub mod viaglob { use crate::visit::*; pub fn e_g_fn_zq(t: &str) { go(t) } pub fn p_g_fn_zq(t: &str) { quiet(t) } }\n";

#[test]
fn a_dependency_item_rebound_through_a_local_use_is_charged() {
    let root = tmpdir("rebind");
    let reps = dep_world(&root);
    for (tag, head) in [
        ("glob", "pub use dgdep::*;\n"),
        ("privglob", "use dgdep::*;\n"),
        ("named", "pub use dgdep::{Datetime, go, quiet};\n"),
    ] {
        let d = pkg(&root, &format!("app_{tag}"), &["dgdep"], &[("lib.rs", &format!("{head}{CONSUMERS}"))]);
        for f in ["other::e_o_assoc_zq", "other::e_o_fn_zq", "named::e_n_fn_zq", "viaglob::e_g_fn_zq"] {
            assert_eq!(deny_fs(&d, f, Some(&reps), &[]), 1, "{tag}: {f} performs the dependency's write");
            // the NAMED root re-export reached through a glob was already charged by the glob-candidate route
            if !(tag == "named" && f == "viaglob::e_g_fn_zq") {
                assert!(off_loses_fs(&d, f, Some(&reps), "CANDOR_R1697_OFF"), "{tag}: {f} must reach R1702");
            }
        }
        for f in ["other::p_o_assoc_zq", "other::p_o_fn_zq", "viaglob::p_g_fn_zq"] {
            assert_eq!(deny_fs(&d, f, Some(&reps), &[]), 0, "{tag}: the pure twin {f} stays pure");
        }
    }
}

#[test]
fn a_name_a_module_may_bind_itself_is_not_sent_to_its_external_glob() {
    // `facade` globs the dependency AND invokes the DEPENDENCY's item macro, which defines a pure `go` in `facade`: that `go`
    // shadows the glob (rustc compiles it), and this scan does not expand a foreign macro, so `crate::visit::go` must not be attributed to `dgdep::go` (a fabricated `Fs`).
    let root = tmpdir("opaque");
    let reps = dep_world(&root);
    let d = pkg(
        &root,
        "app_opaque",
        &["dgdep"],
        &[(
            "lib.rs",
            "pub mod facade { pub use dgdep::*; dgdep::mk!(); }\n\
             pub mod visit { pub(crate) use crate::facade::go; }\n\
             pub mod other { pub fn p_mac_zq(t: &str) { crate::visit::go(t) } }\n",
        )],
    );
    assert_eq!(deny_fs(&d, "other::p_mac_zq", Some(&reps), &[]), 0);
}

/// The facade worlds: `own` declares `Body` and dispatches on it; `fac` re-exports it from a FILE module (`pub mod body;`),
/// `fac_core`-style; `prodf` dispatches on the FACADE's spelling.
fn facade_world(root: &Path) -> PathBuf {
    pkg(root, "own", &[], &[("lib.rs", "pub mod de { pub trait Body { fn poll_zq(&self, t: &str); } }\npub fn drive(b: &dyn de::Body, t: &str) { b.poll_zq(t) }\n")]);
    pkg(root, "fac", &["own"], &[("lib.rs", "pub mod body;\npub use own::de;\n"), ("body/mod.rs", "pub use own::de::Body;\n")]);
    let pf = pkg(root, "prodf", &["fac"], &[("lib.rs", "pub fn drivef<B: fac::de::Body>(b: B, t: &str) { b.poll_zq(t) }\n")]);
    lock(&pf, &["fac", "own", "prodf"]);
    let reps = root.join("reps");
    std::fs::create_dir_all(&reps).unwrap();
    produce(&root.join("own"), None, &reps.join("own"));
    produce(&pf, Some(&reps), &reps.join("prodf"));
    reps
}

#[test]
fn an_implementor_spelled_through_a_facades_file_module_joins_the_owner() {
    let root = tmpdir("filemod");
    let reps = facade_world(&root);
    let c = pkg(
        &root,
        "consf",
        &["own", "fac"],
        &[(
            "lib.rs",
            "pub struct Loud;\nimpl fac::body::Body for Loud { fn poll_zq(&self, t: &str) { let _ = std::fs::remove_file(t); } }\n\
             pub fn e_file_zq(t: &str) { own::drive(&Loud, t) }\npub fn p_nocall_zq(t: &str) -> usize { t.len() }\n",
        )],
    );
    lock(&c, &["consf", "fac", "own"]);
    assert_eq!(deny_fs(&c, "e_file_zq", Some(&reps), &[]), 1);
    assert!(off_loses_fs(&c, "e_file_zq", Some(&reps), "CANDOR_R1736_OFF"), "the fixture must reach R1736");
    assert_eq!(deny_fs(&c, "p_nocall_zq", Some(&reps), &[]), 0);
}

#[test]
fn an_owner_spelled_implementor_joins_a_producer_keyed_under_the_facade() {
    let root = tmpdir("ownerimpl");
    let reps = facade_world(&root);
    // the consumer depends on the OWNER and the producer, never on the facade; its Cargo.lock pins the facade transitively
    let c = pkg(
        &root,
        "conso",
        &["own", "prodf"],
        &[(
            "lib.rs",
            "pub struct Loud;\nimpl own::de::Body for Loud { fn poll_zq(&self, t: &str) { let _ = std::fs::remove_file(t); } }\n\
             pub fn e_owner_zq(t: &str) { prodf::drivef(Loud, t) }\npub fn p_nocall_zq(t: &str) -> usize { t.len() }\n",
        )],
    );
    // A TRANSITIVE facade is read only from a registry checkout the lock pins (a path source is read only for a direct
    // dependency), so `fac` is pinned as a registry package in a fixture `CARGO_HOME` whose `.crate` hashes to the checksum.
    let home = root.join("cargo-home");
    let idx = "index.crates.io-0000000000000000";
    write(&home.join(format!("registry/cache/{idx}/fac-0.1.0.crate")), "abc");
    let src = home.join(format!("registry/src/{idx}/fac-0.1.0"));
    write(&src.join("Cargo.toml"), "[package]\nname = \"fac\"\nversion = \"0.1.0\"\nedition = \"2021\"\n[dependencies]\nown = \"0.1\"\n");
    write(&src.join("src/lib.rs"), "pub mod body;\npub use own::de;\n");
    write(&src.join("src/body/mod.rs"), "pub use own::de::Body;\n");
    write(
        &c.join("Cargo.lock"),
        "version = 4\n\n[[package]]\nname = \"conso\"\nversion = \"0.1.0\"\n\n[[package]]\nname = \"fac\"\nversion = \"0.1.0\"\n\
         source = \"registry+https://github.com/rust-lang/crates.io-index\"\n\
         checksum = \"ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad\"\n\n\
         [[package]]\nname = \"own\"\nversion = \"0.1.0\"\n\n[[package]]\nname = \"prodf\"\nversion = \"0.1.0\"\n",
    );
    let h = home.to_string_lossy().into_owned();
    let env = [("CARGO_HOME", h.as_str())];
    assert_eq!(deny_fs(&c, "e_owner_zq", Some(&reps), &env), 1);
    assert!(infers(&c, "e_owner_zq", Some(&reps), &env).iter().any(|e| e == "Fs"));
    assert!(
        !infers(&c, "e_owner_zq", Some(&reps), &[env[0], ("CANDOR_R1743_OFF", "1")]).iter().any(|e| e == "Fs"),
        "the fixture must reach R1743"
    );
    // no lock-pinned source for the facade (no CARGO_HOME checkout): nothing is added — the reading may only add
    assert!(!infers(&c, "e_owner_zq", Some(&reps), &[("CARGO_HOME", "/nonexistent-r1743")]).iter().any(|e| e == "Fs"));
    assert_eq!(deny_fs(&c, "p_nocall_zq", Some(&reps), &env), 0);
}

#[test]
fn a_name_bound_twice_in_one_module_keeps_the_declaration() {
    // tokio-websockets `sha`: `pub fn digest` beside `use ring::digest` (a MODULE: the other namespace). `crate::sha::digest(..)`
    // is the fn; the first cut of the namespace took the `use` alone, edged the dependency's module and lost the fn's
    // effects (`invisible:[openssl, sha1_smol]` in the corpus). A name a module both declares and imports is answered for
    // neither, so the fn's own effect (here `Env`) stays.
    let root = tmpdir("twin");
    let reps = dep_world(&root);
    let d = pkg(
        &root,
        "app_twin",
        &["dgdep"],
        &[(
            "lib.rs",
            "pub mod sha {\n #[allow(unused_imports)]\n use dgdep::digest;\n\
             pub fn digest(_t: &str) { let _ = std::env::var(\"R1697TWIN\"); }\n}\n\
             pub mod user { use crate::sha::digest; pub fn e_twin_zq(t: &str) { digest(t) } }\n",
        )],
    );
    assert!(infers(&d, "user::e_twin_zq", Some(&reps), &[]).iter().any(|e| e == "Env"), "the declared fn's Env was lost");
}
