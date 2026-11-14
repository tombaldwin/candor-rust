//! SOUNDNESS R1748 (a)/(b) — two PRODUCER-side mis-keyings that let a consumer's `ser::Error` implementor join a
//! deserializing path (EXECUTED `t4_ser_error_core` / `c43_deerr`, lane `rustagent-r1748`).
//!
//! (a) serde_core declares `de::Error` and `ser::Error` through `declare_error_trait!`, whose matcher repeats
//! `$($supertrait:ident)::+`. The macro expander read only one-token separators, so the traits were never seen, and every
//! call naming them (`Error::custom(..)` in a default body, `E::custom` under `E: Error`, `S::Error::custom`) was keyed under
//! the crate's only HAND-WRITTEN `Error` — `std_error::Error`, which declares no `custom`. A consumer could join that key
//! only by its tail, i.e. every same-leaf implementor. The fix records the traits a local macro declares and spells the key
//! under the trait the call names; the in-crate CHA is unchanged.
//!
//! (b) serde_json's `impl ser::Serializer for RawValueStrEmitter { fn serialize_bool(..) -> Result<()> { Err(ser::Error::
//! custom(..)) } }` published `serde_core#ser::Error::custom` although `Self` is fixed by the declared error type
//! (`Result<T> = result::Result<T, Error>`, the crate's own). Such a call is no dispatch: no consumer type can be `Self`.
//!
//! Hermetic: `ocore` (the owner, shaped like serde_core), `pjs` (a producer shaped like serde_json), consumers pinned by a
//! Cargo.lock. Each kill switch restores the old key, the proof that the fixtures reach the change.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Command;

fn bin() -> &'static str {
    env!("CARGO_BIN_EXE_candor-scan")
}

fn tmpdir(tag: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("candor-r1748p-{tag}-{}", std::process::id()));
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
        .env_remove("CANDOR_R1748A_OFF").env_remove("CANDOR_R1748B_OFF").env_remove("CANDOR_R1748SEP_OFF").env_remove("CANDOR_R1715_OFF").env_remove("CANDOR_R1711C_OFF").env_remove("CANDOR_R1739_OFF").env_remove("CANDOR_R1740_OFF").env_remove("CANDOR_R1745_OFF").env_remove("CANDOR_R1748_OFF").env_remove("CANDOR_R1743_OFF").env_remove("CANDOR_R1710_OFF")
        .env("CANDOR_CACHE", dir.join(".r1748p-cache"));
    if let Some(d) = deps {
        cmd.env("CANDOR_DEPS", d);
    }
    for (k, v) in env {
        cmd.env(k, v);
    }
    let o = cmd.output().expect("run candor-scan");
    assert!(o.status.code().is_some_and(|c| c == 0 || c == 1), "scan failed: {}", String::from_utf8_lossy(&o.stderr));
    let _ = std::fs::remove_dir_all(dir.join(".r1748p-cache"));
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


fn ocore(root: &Path) {
    let de = "use crate::lib::Display;\n\
        pub use crate::std_error::Error as StdError;\n\
        macro_rules! declare_error_trait {\n\
          (Error: Sized $(+ $($s:ident)::+)*) => {\n\
            pub trait Error: Sized $(+ $($s)::+)* {\n\
              fn custom(m: u8) -> Self;\n\
              fn invalid_type(u: u8) -> Self { Error::custom(u) }\n\
            }\n\
          };\n\
        }\n\
        declare_error_trait!(Error: Sized + Display);\n\
        pub trait Deserializer: Sized {\n type Error: Error;\n\
          fn deserialize_i128(self) -> Result<(), Self::Error> { Err(Error::custom(1)) }\n}\n\
        pub fn fail<E: Error>() -> E { E::custom(1) }\n\
        pub struct E;\nimpl core::fmt::Display for E { fn fmt(&self, f: &mut core::fmt::Formatter) -> core::fmt::Result { f.write_str(\"e\") } }\n\
        impl Error for E { fn custom(_m: u8) -> Self { E } }\n";
    let ser = "use crate::lib::Display;\n\
        macro_rules! declare_error_trait {\n\
          (Error: Sized $(+ $($s:ident)::+)*) => {\n\
            pub trait Error: Sized $(+ $($s)::+)* { fn custom(m: u8) -> Self; }\n\
          };\n\
        }\n\
        declare_error_trait!(Error: Sized + Display);\n\
        pub trait Serializer: Sized { type Error: Error; }\n\
        pub fn poison<S: Serializer>() -> Result<(), S::Error> { Err(S::Error::custom(2)) }\n\
        pub struct SE;\nimpl core::fmt::Display for SE { fn fmt(&self, f: &mut core::fmt::Formatter) -> core::fmt::Result { f.write_str(\"s\") } }\n\
        impl Error for SE { fn custom(_m: u8) -> Self { SE } }\n";
    let d = pkg(root, "ocore", &[], "pub mod lib { pub use core::fmt::Display; }\nmod std_error;\npub mod de;\npub mod ser;\n");
    write(&d.join("src/std_error.rs"), "pub trait Error: core::fmt::Debug + core::fmt::Display { fn source(&self) -> Option<u8> { None } }\n");
    write(&d.join("src/de.rs"), de);
    write(&d.join("src/ser.rs"), ser);
}

fn keys(rows: &Rows, f: &str) -> Vec<String> {
    rows.get(f).map(|r| r.2.clone()).unwrap_or_default()
}

#[test]
fn a_macro_declared_traits_member_is_keyed_under_the_trait_the_call_names() {
    let root = tmpdir("a");
    ocore(&root);
    let rep = root.join("reps");
    std::fs::create_dir_all(&rep).unwrap();
    let (rows, _) = scan(&root.join("ocore"), None, Some(&rep.join("ocore")), &[]);
    for (f, want) in [
        ("de::Deserializer::deserialize_i128", "ocore#de::Error::custom"),
        ("de::fail", "ocore#de::Error::custom"),
        ("ser::poison", "ocore#ser::Error::custom"),
    ] {
        let k = keys(&rows, f);
        assert!(k.iter().any(|x| x == want), "{f}: no {want}: {k:?}");
        assert!(!k.iter().any(|x| x.contains("std_error")), "{f}: still keyed under std_error: {k:?}");
    }
    // Both halves reach the change: without the trait record, and without the two-character separator, the leaf key is back.
    for sw in ["CANDOR_R1748A_OFF", "CANDOR_R1748SEP_OFF"] {
        let (off, _) = scan(&root.join("ocore"), None, None, &[(sw, "1")]);
        let k = keys(&off, "de::fail");
        assert!(k.iter().any(|x| x == "ocore#std_error::Error::custom"), "{sw}: the fixture does not reach the change: {k:?}");
    }
    // A consumer's SER implementor no longer joins a DE path (lock: the owner's tree source is read)…
    let app = pkg(&root, "app", &["ocore"],
        "pub struct SerL;\nimpl core::fmt::Display for SerL { fn fmt(&self, f: &mut core::fmt::Formatter) -> core::fmt::Result { f.write_str(\"x\") } }\n\
         impl ocore::ser::Error for SerL { fn custom(_m: u8) -> Self { let _ = std::fs::remove_file(\"/nonexistent/r1748a\"); SerL } }\n\
         pub fn e_quiet() { let _: ocore::de::E = ocore::de::fail::<ocore::de::E>(); }\n");
    lock(&app, &["app", "ocore"]);
    let (on, _) = scan(&app, Some(&rep), None, &[]);
    assert!(!has(&on, "e_quiet", "Fs"), "a ser::Error implementor joined a de::Error dispatch: {:?}", on.get("e_quiet"));
    // …and a DE implementor keeps it: the key is the trait's own, so the join is EXACT (the csv-shaped positive control).
    let app2 = pkg(&root, "app2", &["ocore"],
        "pub struct DeL;\nimpl core::fmt::Display for DeL { fn fmt(&self, f: &mut core::fmt::Formatter) -> core::fmt::Result { f.write_str(\"x\") } }\n\
         impl ocore::de::Error for DeL { fn custom(_m: u8) -> Self { let _ = std::fs::remove_file(\"/nonexistent/r1748d\"); DeL } }\n\
         pub fn e_loud() { let _: DeL = ocore::de::fail::<DeL>(); }\n");
    lock(&app2, &["app2", "ocore"]);
    for lockless in [false, true] {
        if lockless {
            let _ = std::fs::remove_file(app2.join("Cargo.lock"));
        }
        let (rows2, _) = scan(&app2, Some(&rep), None, &[]);
        assert!(has(&rows2, "e_loud", "Fs"), "a de::Error implementor lost its join (lock-less {lockless}): {:?}", rows2.get("e_loud"));
    }
}

#[test]
fn a_call_whose_self_is_the_declared_error_type_publishes_no_dispatch() {
    let root = tmpdir("b");
    ocore(&root);
    pkg(&root, "pjs", &["ocore"],
        "pub mod error {\n pub struct Error;\n impl core::fmt::Display for Error { fn fmt(&self, f: &mut core::fmt::Formatter) -> core::fmt::Result { f.write_str(\"j\") } }\n\
          impl ocore::ser::Error for Error { fn custom(_m: u8) -> Self { Error } }\n\
          pub type Result<T> = core::result::Result<T, Error>;\n}\n\
         use crate::error::{Error, Result};\n\
         pub fn emit() -> Result<()> { Err(ocore::ser::Error::custom(1)) }\n\
         pub fn two() -> core::result::Result<(), Error> { if true { return Err(ocore::ser::Error::custom(1)); } Ok(()) }\n\
         pub fn open<X: ocore::ser::Error>() -> core::result::Result<(), X> { Err(ocore::ser::Error::custom(1)) }\n\
         pub fn bound<X: ocore::ser::Error>() -> Result<()> { let r: core::result::Result<(), X> = Err(ocore::ser::Error::custom(1)); let _ = r; Ok(()) }\n");
    let rep = root.join("reps");
    std::fs::create_dir_all(&rep).unwrap();
    scan(&root.join("ocore"), None, Some(&rep.join("ocore")), &[]);
    let (rows, _) = scan(&root.join("pjs"), Some(&rep), Some(&rep.join("pjs")), &[]);
    let k = "ocore#ser::Error::custom";
    for f in ["emit", "two"] {
        assert!(!keys(&rows, f).iter().any(|x| x == k), "{f}: Self is the declared error type, still a dispatch: {:?}", rows.get(f));
    }
    // the open one stays open: `Self` is the caller's `X`, in the return type and in an annotated binding alike
    for f in ["open", "bound"] {
        assert!(keys(&rows, f).iter().any(|x| x == k), "{f}: an open Self lost its key: {:?}", rows.get(f));
    }
    let (off, _) = scan(&root.join("pjs"), Some(&rep), None, &[("CANDOR_R1748B_OFF", "1")]);
    assert!(keys(&off, "emit").iter().any(|x| x == k), "R1748B off: the fixture does not reach the change: {:?}", off.get("emit"));
    // the consumer: its ser implementor joins only the open dispatch
    let app = pkg(&root, "app", &["ocore", "pjs"],
        "pub struct L;\nimpl core::fmt::Display for L { fn fmt(&self, f: &mut core::fmt::Formatter) -> core::fmt::Result { f.write_str(\"x\") } }\n\
         impl ocore::ser::Error for L { fn custom(_m: u8) -> Self { let _ = std::fs::remove_file(\"/nonexistent/r1748b\"); L } }\n\
         pub fn e_emit() { let _ = pjs::emit(); }\n\
         pub fn e_open() { let _ = pjs::open::<L>(); }\n");
    lock(&app, &["app", "ocore", "pjs"]);
    let (a, _) = scan(&app, Some(&rep), None, &[]);
    assert!(!has(&a, "e_emit", "Fs"), "a consumer implementor joined a call whose Self is the producer's own type: {:?}", a.get("e_emit"));
    assert!(has(&a, "e_open", "Fs"), "the open dispatch lost the consumer's implementor: {:?}", a.get("e_open"));
}
