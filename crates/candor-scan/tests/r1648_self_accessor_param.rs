//! SOUNDNESS R1648 — an accessor that returns its impl's type parameter, reached through `self` (or a parameter / an
//! annotated `let`) rather than a field, typed the value as the RECEIVER's own type (R979's original reading), so
//! `self.as_file_mut().read(buf)` in tempfile's by-value `impl<F: Read> Read for NamedTempFile<F>` formed
//! `NamedTempFile::read` — the unit itself — and the read vanished. EXECUTED fixture rustagent-r1648/fx/acc: every
//! variant below reads one byte of a real file (and the byte changes when the file is rewritten); V1-V4, V6-V12 and the
//! three `c_*` consumers were ABSENT on published 0.41.3 and on 457663d. The FIELD spelling `self.file.read(buf)` reads
//! `Unknown` `dispatch:Read.read`, which is what the generic variants now read; the concrete variants (V8, V11) resolve
//! to `Fs`. The pure twin dispatches over a crate-local trait whose only impl is pure and must stay pure.

use std::path::{Path, PathBuf};
use std::process::Command;

fn bin() -> &'static str {
    env!("CARGO_BIN_EXE_candor-scan")
}

fn fixture(tag: &str, src: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("candor-r1648-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(d.join("src")).unwrap();
    std::fs::write(d.join("Cargo.toml"), "[package]\nname = \"acc\"\nversion = \"0.1.0\"\nedition = \"2021\"\n").unwrap();
    std::fs::write(d.join("src/lib.rs"), src).unwrap();
    d
}

fn run(dir: &Path, off: bool, policy: Option<&Path>) -> (i32, serde_json::Value) {
    let mut cmd = Command::new(bin());
    cmd.arg(dir.to_string_lossy().as_ref()).arg("--json");
    if let Some(p) = policy {
        cmd.arg("--policy").arg(p);
    }
    cmd.env_remove("CANDOR_DEPS").env_remove("CANDOR_CONFIG").env_remove("CANDOR_POLICY").env_remove("CANDOR_R1648_OFF");
    if off {
        cmd.env("CANDOR_R1648_OFF", "1");
    }
    let o = cmd.output().expect("run candor-scan");
    let code = o.status.code().unwrap_or(-1);
    let v = serde_json::from_slice(&o.stdout).unwrap_or(serde_json::Value::Null);
    (code, v)
}

fn scan(dir: &Path, off: bool) -> serde_json::Value {
    let (code, v) = run(dir, off, None);
    assert!(code == 0 || code == 1, "scan failed ({code})");
    v
}

fn row<'a>(v: &'a serde_json::Value, f: &str) -> Option<&'a serde_json::Value> {
    v["functions"].as_array().unwrap().iter().find(|r| r["fn"].as_str() == Some(f))
}

fn inferred(v: &serde_json::Value, f: &str) -> Vec<String> {
    let mut out: Vec<String> = row(v, f)
        .and_then(|r| r["inferred"].as_array())
        .map(|a| a.iter().filter_map(|e| e.as_str().map(str::to_string)).collect())
        .unwrap_or_default();
    out.sort();
    out
}

fn why(v: &serde_json::Value, f: &str) -> Vec<String> {
    row(v, f)
        .and_then(|r| r["unknownWhy"].as_array())
        .map(|a| a.iter().filter_map(|e| e.as_str().map(str::to_string)).collect())
        .unwrap_or_default()
}

const SRC: &str = r#"use std::io::{self, Read};
use std::marker::PhantomData;

// V1: the row. By-value impl over the bounded parameter, accessor reached through `self`.
pub mod v1 {
    use super::*;
    pub struct NtA<F> { pub file: F }
    impl<F> NtA<F> { pub fn as_file_mut(&mut self) -> &mut F { &mut self.file } }
    impl<F: Read> Read for NtA<F> {
        fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> { self.as_file_mut().read(buf) }
    }
}
// V2: the bound in a `where` clause.
pub mod v2 {
    use super::*;
    pub struct NtB<F> { pub file: F }
    impl<F> NtB<F> { pub fn as_file_mut(&mut self) -> &mut F { &mut self.file } }
    impl<F> Read for NtB<F> where F: Read {
        fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> { self.as_file_mut().read(buf) }
    }
}
// V3: a lifetime parameter before the type parameter, written in both impls.
pub mod v3 {
    use super::*;
    pub struct NtC<'a, F> { pub file: F, pub _p: PhantomData<&'a ()> }
    impl<'a, F> NtC<'a, F> { pub fn as_file_mut(&mut self) -> &mut F { &mut self.file } }
    impl<'a, F: Read> Read for NtC<'a, F> {
        fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> { self.as_file_mut().read(buf) }
    }
}
// V4: an inherent method, not a trait impl.
pub mod v4 {
    use super::*;
    pub struct NtD<F> { pub file: F }
    impl<F> NtD<F> { pub fn as_file_mut(&mut self) -> &mut F { &mut self.file } }
    impl<F: Read> NtD<F> {
        pub fn first_byte(&mut self) -> u8 { let mut b = [0u8; 1]; self.as_file_mut().read(&mut b).unwrap(); b[0] }
    }
}
// V5: a binding between the accessor and the call.
pub mod v5 {
    use super::*;
    pub struct NtE<F> { pub file: F }
    impl<F> NtE<F> { pub fn as_file_mut(&mut self) -> &mut F { &mut self.file } }
    impl<F: Read> Read for NtE<F> {
        fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> { let f = self.as_file_mut(); f.read(buf) }
    }
}
// V6: the UFCS spelling.
pub mod v6 {
    use super::*;
    pub struct NtF<F> { pub file: F }
    impl<F> NtF<F> { pub fn as_file_mut(&mut self) -> &mut F { &mut self.file } }
    impl<F: Read> Read for NtF<F> {
        fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> { Read::read(self.as_file_mut(), buf) }
    }
}
// V7: two parameters, the accessor returns the second.
pub mod v7 {
    use super::*;
    pub struct NtG<G, F> { pub tag: G, pub file: F }
    impl<G, F> NtG<G, F> { pub fn as_file_mut(&mut self) -> &mut F { &mut self.file } }
    impl<G: std::fmt::Debug, F: Read> Read for NtG<G, F> {
        fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> { self.as_file_mut().read(buf) }
    }
}
// V8: a CONCRETE crate-local reader as the argument (not a std handle).
pub mod v8 {
    use super::*;
    pub struct MyR(pub std::fs::File);
    impl Read for MyR { fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> { self.0.read(buf) } }
    pub struct NtH<F> { pub file: F }
    impl<F> NtH<F> { pub fn as_file_mut(&mut self) -> &mut F { &mut self.file } }
    impl Read for NtH<MyR> {
        fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> { self.as_file_mut().read(buf) }
    }
}
// V9: the accessor reached through `Self::` path call.
pub mod v9 {
    use super::*;
    pub struct NtI<F> { pub file: F }
    impl<F> NtI<F> { pub fn as_file_mut(&mut self) -> &mut F { &mut self.file } }
    impl<F: Read> Read for NtI<F> {
        fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> { Self::as_file_mut(self).read(buf) }
    }
}
// V10-V12: the accessor on a PARAMETER / annotated LET of the generic type (R1007's shape).
pub mod v10 {
    use super::*;
    pub struct NtJ<F> { pub file: F }
    impl<F> NtJ<F> { pub fn as_file_mut(&mut self) -> &mut F { &mut self.file } }
    pub fn via_param<F: Read>(p: &mut NtJ<F>, buf: &mut [u8]) -> usize { p.as_file_mut().read(buf).unwrap() }
    pub fn via_param_conc(p: &mut NtJ<crate::v8::MyR>, buf: &mut [u8]) -> usize { p.as_file_mut().read(buf).unwrap() }
    pub fn via_let<F: Read>(f: F, buf: &mut [u8]) -> usize { let mut l: NtJ<F> = NtJ { file: f }; l.as_file_mut().read(buf).unwrap() }
}
// V13: a member no bound declares - a crate-local extension trait blanket-implemented for every `W: Write` (gif's
// `self.writer()?.write_le(..)`). Must keep reaching the blanket impl's write.
pub mod v13 {
    use std::io::{self, Write};
    pub trait WriteLe { fn write_le(&mut self, b: u8) -> io::Result<()>; }
    impl<W: Write + ?Sized> WriteLe for W { fn write_le(&mut self, b: u8) -> io::Result<()> { self.write_all(&[b]) } }
    pub struct Enc<W: Write> { pub w: Option<W> }
    impl<W: Write> Enc<W> {
        fn writer(&mut self) -> Result<&mut W, ()> { self.w.as_mut().ok_or(()) }
        pub fn trailer(&mut self) -> Result<(), ()> { self.writer()?.write_le(0x3b).map_err(|_| ()) }
    }
}
// PURE TWIN: same shape over a crate-local trait whose only impl is pure.
pub mod twin {
    pub trait Tally { fn tally_zq(&self) -> usize; }
    pub struct Beads(pub Vec<u8>);
    impl Tally for Beads { fn tally_zq(&self) -> usize { self.0.len() } }
    pub struct HolderZq<F> { pub inner: F }
    impl<F> HolderZq<F> { pub fn as_inner_mut(&mut self) -> &mut F { &mut self.inner } }
    impl<F: Tally> HolderZq<F> {
        pub fn count_puretwin_zq(&mut self) -> usize { self.as_inner_mut().tally_zq() }
    }
}

pub fn c_v1(n: &mut v1::NtA<std::fs::File>) -> usize { let mut b = [0u8; 1]; n.read(&mut b).unwrap() }
pub fn c_v4(n: &mut v4::NtD<std::fs::File>) -> u8 { n.first_byte() }
pub fn c_v8(n: &mut v8::NtH<v8::MyR>) -> usize { let mut b = [0u8; 1]; n.read(&mut b).unwrap() }
pub fn c_twin_zq(n: &mut twin::HolderZq<twin::Beads>) -> usize { n.count_puretwin_zq() }
"#;

const GENERIC: &[&str] = &[
    "v1::NtA::read",
    "v2::NtB::read",
    "v3::NtC::read",
    "v4::NtD::first_byte",
    "v6::NtF::read",
    "v7::NtG::read",
    "v9::NtI::read",
    "v10::via_param",
    "v10::via_let",
];
const CONCRETE: &[&str] = &["v8::NtH::read", "v10::via_param_conc", "c_v8"];

#[test]
fn r1648_accessor_through_self_param_let_is_charged() {
    let d = fixture("on", SRC);
    let v = scan(&d, false);
    for f in GENERIC {
        assert_eq!(inferred(&v, f), vec!["Unknown"], "{f}: the generic variant must disclose");
        assert_eq!(why(&v, f), vec!["dispatch:Read.read"], "{f}: the reason must be the field spelling's");
    }
    for f in CONCRETE {
        assert_eq!(inferred(&v, f), vec!["Fs"], "{f}: the concrete variant must resolve");
    }
    assert_eq!(inferred(&v, "c_v1"), vec!["Unknown"]);
    assert_eq!(inferred(&v, "c_v4"), vec!["Unknown"]);
    // The binding spelling was already disclosed (untyped receiver); it now carries the true dispatch reason.
    assert_eq!(why(&v, "v5::NtE::read"), vec!["dispatch:Read.read"]);
    // V13: a member no bound declares (an extension trait blanket-implemented for every `W: Write`) keeps reaching the
    // blanket impl; this lane's first cut dispatched it over `Write` alone and dropped it (gif's `Encoder::write_trailer`).
    assert_eq!(inferred(&v, "v13::Enc::trailer"), vec!["Unknown"], "the extension-trait call must not vanish");
    // Pure twins: the same accessor shape over a crate-local trait whose only impl is pure.
    for f in ["twin::HolderZq::count_puretwin_zq", "c_twin_zq"] {
        assert!(inferred(&v, f).is_empty(), "{f}: pure twin must stay pure, got {:?}", inferred(&v, f));
    }
    let _ = std::fs::remove_dir_all(&d);
}

#[test]
fn r1648_kill_switch_restores_the_silence() {
    let d = fixture("off", SRC);
    let v = scan(&d, true);
    for f in GENERIC.iter().chain(CONCRETE) {
        assert!(row(&v, f).is_none(), "{f}: with CANDOR_R1648_OFF the pre-fix silence returns, got {:?}", row(&v, f));
    }
    for f in ["c_v1", "c_v4"] {
        assert!(row(&v, f).is_none(), "{f}");
    }
    assert_eq!(inferred(&v, "v13::Enc::trailer"), vec!["Unknown"], "V13 reads the same with the switch off");
    let _ = std::fs::remove_dir_all(&d);
}

#[test]
fn r1648_gates_flip_over_the_executed_read() {
    let d = fixture("gate", SRC);
    for (pol, want) in [
        ("deny Fs v8::NtH::read\n", 1),
        ("deny Fs c_v8\n", 1),
        ("deny Fs Unknown v1::NtA::read\n", 1),
        ("deny Fs Unknown c_v1\n", 1),
        ("deny Fs Unknown twin::HolderZq::count_puretwin_zq\n", 0),
    ] {
        let p = d.join("p.pol");
        std::fs::write(&p, pol).unwrap();
        let (code, _) = run(&d, false, Some(&p));
        assert_eq!(code, want, "{pol}");
    }
    let _ = std::fs::remove_dir_all(&d);
}

const DEP_SRC: &str = r#"use std::io::Read;
pub struct Holder<C> { pub c: C }
impl<C> Holder<C> { pub fn conn(&mut self) -> &mut C { &mut self.c } }
impl<C: depx::Svc> Holder<C> {
    pub fn go_dep_zq(&mut self) -> usize { self.conn().call().unwrap() }
}
pub struct FileSvc(pub std::fs::File);
impl depx::Svc for FileSvc { fn call(&mut self) -> std::io::Result<usize> { let mut b = [0u8; 1]; self.0.read(&mut b) } }
pub fn param_dep_zq<C: depx::Svc>(c: &mut C) -> usize { c.call().unwrap() }
impl<C: depx::Svc> Holder<C> {
    pub fn field_dep_zq(&mut self) -> usize { self.c.call().unwrap() }
}
"#;

/// The accessor's bound is a DEPENDENCY trait (`C: depx::Svc`): `self.conn().call()` was ABSENT with no key and no
/// `invisible` (EXECUTED rustagent-r1648/fx/dep: `go_dep_zq` reads one byte of a real file through `FileSvc`), where
/// the field spelling `self.c.call()` reads `Unknown` `dispatch:Svc.call` with the `dispatchesOn` key. It now reads as
/// the field spelling does.
#[test]
fn r1648_dependency_trait_bound_reads_as_the_field_spelling() {
    let d = std::env::temp_dir().join(format!("candor-r1648-dep-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(d.join("src")).unwrap();
    std::fs::create_dir_all(d.join("depx/src")).unwrap();
    std::fs::write(d.join("Cargo.toml"),
        "[package]\nname = \"depb\"\nversion = \"0.1.0\"\nedition = \"2021\"\n[workspace]\n[dependencies]\ndepx = { path = \"depx\" }\n").unwrap();
    std::fs::write(d.join("depx/Cargo.toml"), "[package]\nname = \"depx\"\nversion = \"0.1.0\"\nedition = \"2021\"\n").unwrap();
    std::fs::write(d.join("depx/src/lib.rs"), "pub trait Svc { fn call(&mut self) -> std::io::Result<usize>; }\n").unwrap();
    std::fs::write(d.join("src/lib.rs"), DEP_SRC).unwrap();
    let v = scan(&d, false);
    assert_eq!(inferred(&v, "Holder::field_dep_zq"), vec!["Unknown"]);
    assert_eq!(inferred(&v, "Holder::go_dep_zq"), vec!["Unknown"], "the accessor spelling must disclose as the field does");
    assert_eq!(why(&v, "Holder::go_dep_zq"), vec!["dispatch:Svc.call"]);
    let off = scan(&d, true);
    assert!(row(&off, "Holder::go_dep_zq").is_none(), "with CANDOR_R1648_OFF the pre-fix silence returns");
    let _ = std::fs::remove_dir_all(&d);
}
