//! SOUNDNESS R1723 / R1724 — a FIELD typed by its owner's own type parameter, and a same-leaf twin of the owner.
//!
//! R1723: `fields` (and the drop-glue owner closure built from it) is LEAF-keyed and keeps the LAST same-leaf declaration
//! merged. With `struct RespFut<F> { inner: F }` in one file and `struct RespFut { inner: Quiet }` in another, which twin the
//! walk merged last decided whether `Pin::new(&mut self.inner).poll(cx)` (or `self.as_mut().project().inner.poll(cx)`) hedged
//! or read ABSENT; once R1722 sorted the walk, the FILE NAMES decided it. The same shape in the drop glue: `Wrap { inner:
//! Inner }` over `#[cfg]` twins of `Inner` owned the built arm's guard or nothing, by file name.
//!
//! With NO twin at all, the pin-project spelling `self.as_mut().project().inner.poll(cx)` (and `let this =
//! self.project(); this.inner.poll(cx)`) read ABSENT on v0.41.3 and every later build (same fix).
//!
//! R1724: a dispatch on a value bounded by a trait spelled through a CRATE-LOCAL path that names no trait the scan can see
//! (async-std re-exports `futures_io::AsyncBufRead as BufRead` inside a `cfg_std!` macro) formed no key and no disclosure;
//! it now discloses `dispatch:<Trait>.<member>`. EXECUTED (`rustagent-r1723/fx/r1724`: both members write a file).
//!
//! The fixture is EXECUTED ground truth (handoff `rustagent-r1723/fx`, `cargo run` per arm, both file orders): every `_zq`
//! poll / write arm below writes a file, the `calm_` twins and the `foff` arm (a non-default feature) do not. Each case is
//! built in BOTH file orders (the generic / effectful declarations in `aa.rs`, then in `zz.rs`). `CANDOR_R1723_OFF` restores
//! the leaf-keyed reading (the A/B arm).
//!
//! Fixture fns carry a unique `_zq` terminator because `deny <E> <fn>` matches by PREFIX.

use std::path::{Path, PathBuf};
use std::process::Command;

fn bin() -> &'static str {
    env!("CARGO_BIN_EXE_candor-scan")
}

const GENERIC: &str = "\
use std::{future::Future, pin::Pin, task::{Context, Poll}};
pub struct RespFut<F> { pub inner: F, pub n: u8 }
impl<F: Future<Output = u8> + Unpin> RespFut<F> {
    pub fn pin_new_zq(&mut self, cx: &mut Context<'_>) -> Poll<u8> { Pin::new(&mut self.inner).poll(cx) }
}
pin_project_lite::pin_project! {
    pub struct ProjFut<F> { #[pin] pub inner: F, pub n: u8 }
}
impl<F: Future<Output = u8>> Future for ProjFut<F> {
    type Output = u8;
    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<u8> { self.as_mut().project().inner.poll(cx) }
}
pin_project_lite::pin_project! {
    pub struct BindFut<F> { #[pin] pub inner: F, pub n: u8 }
}
impl<G: Future<Output = u8>> Future for BindFut<G> {
    type Output = u8;
    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<u8> { let this = self.project(); this.inner.poll(cx) }
}
#[cfg(unix)]
pub mod back { pub struct Inner { pub s: crate::Loud2 } }
#[cfg(not(feature = \"alt\"))]
pub mod fdef { pub struct FInner { pub s: crate::Loud2 } }
#[cfg(feature = \"alt\")]
pub mod foff { pub struct GInner { pub s: crate::Loud2 } }
";

const CONCRETE: &str = "\
pub struct Quiet;
impl Quiet { pub fn poll(&mut self) -> u8 { 0 } }
pub struct RespFut { pub inner: Quiet, pub n: u8 }
pub struct ProjFut { pub inner: Quiet, pub n: u8 }
pub struct BindFut { pub inner: Quiet, pub n: u8 }
impl RespFut { pub fn calm_zq(&mut self) -> u8 { self.inner.poll() } }
#[cfg(not(unix))]
pub mod back { pub struct Inner { pub s: u8 } }
#[cfg(feature = \"alt\")]
pub mod fdef { pub struct FInner { pub s: u8 } }
#[cfg(not(feature = \"alt\"))]
pub mod foff { pub struct GInner { pub s: u8 } }
";

/// `generic_first`: the generic / effectful declarations sort first (`aa.rs`), else last (`zz.rs`).
fn fixture(tag: &str, generic_first: bool) -> PathBuf {
    let d = std::env::temp_dir().join(format!("candor-r1723-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(d.join("src")).unwrap();
    std::fs::write(
        d.join("Cargo.toml"),
        "[package]\nname = \"fx\"\nversion = \"0.1.0\"\nedition = \"2021\"\n[dependencies]\npin-project-lite = \"0.2\"\n[features]\ndefault = []\nalt = []\n",
    )
    .unwrap();
    let (g, c) = if generic_first { ("aa", "zz") } else { ("zz", "aa") };
    std::fs::write(d.join("src").join(format!("{g}.rs")), GENERIC).unwrap();
    std::fs::write(d.join("src").join(format!("{c}.rs")), CONCRETE).unwrap();
    // the same generic shapes with NO twin
    let solo = GENERIC
        .lines()
        .filter(|l| !l.starts_with("#[cfg") && !l.starts_with("pub mod "))
        .collect::<Vec<_>>()
        .join("\n")
        .replace("RespFut", "SRespFut")
        .replace("ProjFut", "SProjFut")
        .replace("BindFut", "SBindFut");
    std::fs::write(d.join("src/solo.rs"), solo).unwrap();
    let main = format!(
        "mod aa;\nmod zz;\nmod solo;\n\
         pub struct Loud2;\n\
         impl Drop for Loud2 {{ fn drop(&mut self) {{ std::fs::write(std::env::temp_dir().join(\"r1723\"), b\"d\").unwrap(); }} }}\n\
         #[cfg(unix)] use {g}::back::Inner;\n#[cfg(not(unix))] use {c}::back::Inner;\n\
         #[cfg(not(feature = \"alt\"))] use {g}::fdef::FInner;\n#[cfg(feature = \"alt\")] use {c}::fdef::FInner;\n\
         #[cfg(feature = \"alt\")] use {g}::foff::GInner;\n#[cfg(not(feature = \"alt\"))] use {c}::foff::GInner;\n\
         pub struct Wrap {{ pub inner: Inner }}\npub struct FWrap {{ pub inner: FInner }}\npub struct GWrap {{ pub inner: GInner }}\n\
         pub struct Holder {{ pub w: Wrap, pub fw: FWrap, pub gw: GWrap }}\n\
         pub fn reset_wcfg_zq(h: &mut Holder, n: Inner) {{ h.w = Wrap {{ inner: n }}; }}\n\
         pub fn reset_wfeat_zq(h: &mut Holder, n: FInner) {{ h.fw = FWrap {{ inner: n }}; }}\n\
         pub fn reset_woff_zq(h: &mut Holder, n: GInner) {{ h.gw = GWrap {{ inner: n }}; }}\n\
         fn main() {{}}\n"
    );
    std::fs::write(d.join("src/main.rs"), main).unwrap();
    d
}

fn scan(dir: &Path, off: bool) -> serde_json::Value {
    let mut cmd = Command::new(bin());
    cmd.arg(dir.to_string_lossy().as_ref()).arg("--json");
    cmd.env_remove("CANDOR_DEPS").env_remove("CANDOR_CONFIG").env_remove("CANDOR_POLICY").env_remove("CANDOR_R1723_OFF");
    if off {
        cmd.env("CANDOR_R1723_OFF", "1");
    }
    let out = cmd.output().expect("run candor-scan");
    assert!(out.status.success(), "scan failed: {}", String::from_utf8_lossy(&out.stderr));
    serde_json::from_slice(&out.stdout).expect("report json")
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

const PINNED: &str = "dispatch:generic pinned receiver of `poll`";

#[test]
fn r1723_param_field_poll_hedges_in_both_file_orders() {
    for (tag, first) in [("gfirst", true), ("glast", false)] {
        let d = fixture(tag, first);
        let g = if first { "aa" } else { "zz" };
        let c = if first { "zz" } else { "aa" };
        let v = scan(&d, false);
        for f in [format!("{g}::RespFut::pin_new_zq"), format!("{g}::ProjFut::poll"), format!("{g}::BindFut::poll")] {
            assert_eq!(inferred(&v, &f), vec!["Unknown".to_string()], "{tag}: `{f}` polls a caller-chosen future:\n{v:#}");
            assert!(why(&v, &f).iter().any(|w| w == PINNED), "{tag}: `{f}` must name the true reason:\n{v:#}");
        }
        // the concrete twin's own pure method is not charged by the generic twin's hedge
        assert!(row(&v, &format!("{c}::RespFut::calm_zq")).is_none(), "{tag}: the pure twin must stay pure:\n{v:#}");
        let _ = std::fs::remove_dir_all(&d);
    }
}

#[test]
fn r1723_switch_restores_the_order_dependent_reading() {
    // The A/B arm: with the switch the two orders disagree on at least one spelling (the defect), without it they agree.
    let mut off_rows = Vec::new();
    let mut on_rows = Vec::new();
    for (tag, first) in [("swfirst", true), ("swlast", false)] {
        let d = fixture(tag, first);
        let g = if first { "aa" } else { "zz" };
        let (on, off) = (scan(&d, false), scan(&d, true));
        let fs = [format!("{g}::RespFut::pin_new_zq"), format!("{g}::ProjFut::poll")];
        on_rows.push(fs.iter().map(|f| inferred(&on, f)).collect::<Vec<_>>());
        off_rows.push(fs.iter().map(|f| inferred(&off, f)).collect::<Vec<_>>());
        let _ = std::fs::remove_dir_all(&d);
    }
    assert_eq!(on_rows[0], on_rows[1], "the fixed reading must not depend on the file order");
    assert_ne!(off_rows[0], off_rows[1], "the switched-off arm must reproduce the order dependence (else this test cannot fail)");
}

#[test]
fn r1723_projection_poll_without_a_twin_hedges() {
    let d = fixture("solo", true);
    let v = scan(&d, false);
    for f in ["solo::SProjFut::poll", "solo::SBindFut::poll", "solo::SRespFut::pin_new_zq"] {
        assert!(why(&v, f).iter().any(|w| w == PINNED), "`{f}` polls a caller-chosen future:\n{v:#}");
    }
    let off = scan(&d, true);
    assert!(row(&off, "solo::SProjFut::poll").is_none(), "the switched-off arm must reproduce the no-twin silence:\n{off:#}");
    let _ = std::fs::remove_dir_all(&d);
}

#[test]
fn r1723_twin_drop_glue_unions_built_arms_in_both_file_orders() {
    let mut off_cells = Vec::new();
    for (tag, first) in [("dfirst", true), ("dlast", false)] {
        let d = fixture(tag, first);
        let v = scan(&d, false);
        // the platform arm built here drops the guard: charged whichever file sorts last
        if cfg!(unix) {
            assert!(inferred(&v, "reset_wcfg_zq").contains(&"Fs".to_string()), "{tag}: platform arm:\n{v:#}");
        }
        // the default-feature arm drops the guard
        assert!(inferred(&v, "reset_wfeat_zq").contains(&"Fs".to_string()), "{tag}: default-feature arm:\n{v:#}");
        // the only guard-dropping arm is under a non-default feature: the default build never runs it
        assert!(!inferred(&v, "reset_woff_zq").contains(&"Fs".to_string()), "{tag}: non-default arm:\n{v:#}");
        let off = scan(&d, true);
        off_cells.push((inferred(&off, "reset_wfeat_zq"), inferred(&off, "reset_woff_zq")));
        let _ = std::fs::remove_dir_all(&d);
    }
    assert_ne!(off_cells[0], off_cells[1], "the switched-off arm must reproduce the order dependence (else this test cannot fail)");
}

const R1724_LIB: &str = "\
pub mod io {
    macro_rules! hide { ($($i:item)*) => { $($i)* } }
    hide! { pub use futures_io::AsyncBufRead as BufRead; }
}
pub mod chain {
    use crate::io::BufRead;
    use std::pin::Pin;
    use std::task::{Context, Poll};
    pin_project_lite::pin_project! { pub struct Take<T> { #[pin] pub inner: T } }
    impl<T: BufRead> Take<T> {
        pub fn fill_zq(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<usize>> {
            let this = self.project();
            match this.inner.poll_fill_buf(cx) { Poll::Ready(Ok(b)) => Poll::Ready(Ok(b.len())), Poll::Ready(Err(e)) => Poll::Ready(Err(e)), Poll::Pending => Poll::Pending }
        }
        pub fn consume_zq(self: Pin<&mut Self>, n: usize) { let this = self.project(); this.inner.consume(n) }
        pub fn pinned_zq(self: Pin<&mut Self>) -> bool { let this = self.project(); this.inner.as_ref().get_ref() as *const T as usize == 0 }
    }
}
";

#[test]
fn r1724_unkeyable_local_bound_discloses() {
    let d = std::env::temp_dir().join(format!("candor-r1724-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(d.join("src")).unwrap();
    std::fs::write(
        d.join("Cargo.toml"),
        "[package]\nname = \"fx\"\nversion = \"0.1.0\"\nedition = \"2021\"\n[dependencies]\nfutures-io = \"0.3\"\npin-project-lite = \"0.2\"\n",
    )
    .unwrap();
    std::fs::write(d.join("src/lib.rs"), R1724_LIB).unwrap();
    let v = scan(&d, false);
    assert!(why(&v, "chain::Take::consume_zq").iter().any(|w| w == "dispatch:BufRead.consume"), "{v:#}");
    assert!(why(&v, "chain::Take::fill_zq").iter().any(|w| w == "dispatch:BufRead.poll_fill_buf"), "{v:#}");
    // a member of the projection's own wrapper (`Pin::as_ref`, `Pin::get_ref`) is not the trait's
    assert!(row(&v, "chain::Take::pinned_zq").is_none(), "{v:#}");
    let mut cmd = Command::new(bin());
    cmd.arg(d.to_string_lossy().as_ref()).arg("--json").env("CANDOR_R1724_OFF", "1");
    cmd.env_remove("CANDOR_DEPS").env_remove("CANDOR_CONFIG").env_remove("CANDOR_POLICY").env_remove("CANDOR_R1723_OFF");
    let off: serde_json::Value = serde_json::from_slice(&cmd.output().unwrap().stdout).unwrap();
    assert!(row(&off, "chain::Take::consume_zq").is_none(), "the switched-off arm must reproduce the silence:\n{off:#}");
    let _ = std::fs::remove_dir_all(&d);
}

#[test]
fn r1723_twin_param_field_bound_is_the_owners_own() {
    // bzip2's shape: `bufread::Dec<R> { obj: R }` (`impl<R: BufRead>`) beside `write::Dec<W: Write> { obj: Option<W> }`. The leaf-keyed bound
    // entry is the LAST twin's (`Write`, which declares no `fill_buf`): the read dispatched over it and read pure.
    for (tag, a, b) in [("bfirst", "bufread", "write"), ("blast", "zbufread", "write")] {
        let d = std::env::temp_dir().join(format!("candor-r1723b-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(d.join("src")).unwrap();
        std::fs::write(d.join("Cargo.toml"), "[package]\nname = \"fx\"\nversion = \"0.1.0\"\nedition = \"2021\"\n").unwrap();
        std::fs::write(d.join("src/lib.rs"), format!("pub mod {a};\npub mod {b};\n")).unwrap();
        std::fs::write(
            d.join("src").join(format!("{a}.rs")),
            "use std::io;\nuse std::io::prelude::*;\npub struct Dec<R> { obj: R }\n\
             impl<R: BufRead> Read for Dec<R> {\n    fn read(&mut self, _b: &mut [u8]) -> io::Result<usize> { let n = self.obj.fill_buf()?.len(); self.obj.consume(n); Ok(n) }\n}\n",
        )
        .unwrap();
        std::fs::write(
            d.join("src").join(format!("{b}.rs")),
            "use std::io;\nuse std::io::prelude::*;\npub struct Dec<W: Write> { obj: Option<W> }\n\
             impl<W: Write> Write for Dec<W> {\n    fn write(&mut self, b: &[u8]) -> io::Result<usize> { self.obj.as_mut().unwrap().write(b) }\n    fn flush(&mut self) -> io::Result<()> { Ok(()) }\n}\n",
        )
        .unwrap();
        let v = scan(&d, false);
        let f = format!("{a}::Dec::read");
        assert!(why(&v, &f).iter().any(|w| w == "dispatch:BufRead.fill_buf"), "{tag}: `{f}`:\n{v:#}");
        let _ = std::fs::remove_dir_all(&d);
    }
}

#[test]
fn r1723_blanket_subtrait_member_reaches_the_supertraits_implementors() {
    // combine's shape: `trait RangeStream: RangeStreamOnce {}` with `impl<I: RangeStreamOnce> RangeStream for I`, and a
    // member of the SUPERTRAIT called on a `S: RangeStream` field. The blanket's self type is a parameter name, so the
    // dispatch formed `I::u_zq` and read pure (every arm, v0.41.3 included); EXECUTED `rustagent-r1723/fx/q6` writes.
    let d = std::env::temp_dir().join(format!("candor-r1723c-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(d.join("src")).unwrap();
    std::fs::write(d.join("Cargo.toml"), "[package]\nname = \"fx\"\nversion = \"0.1.0\"\nedition = \"2021\"\n").unwrap();
    std::fs::write(
        d.join("src/lib.rs"),
        "pub trait Once { fn u_zq(&mut self) -> u8; }\npub trait Rs: Once {}\npub struct Loud;\n\
         impl Once for Loud { fn u_zq(&mut self) -> u8 { std::fs::write(std::env::temp_dir().join(\"r1723c\"), b\"x\").unwrap(); 1 } }\n\
         impl<T: Once> Rs for T {}\n\
         pub struct Solo<S>(pub S);\nimpl<S: Rs> Solo<S> { pub fn solo_zq(&mut self) -> u8 { self.0.u_zq() } }\n\
         pub fn param_zq<S: Rs>(s: &mut S) -> u8 { s.u_zq() }\n",
    )
    .unwrap();
    let v = scan(&d, false);
    for f in ["Solo::solo_zq", "param_zq"] {
        assert!(inferred(&v, f).contains(&"Fs".to_string()), "`{f}` reaches `Loud::u_zq`:\n{v:#}");
    }
    let off = scan(&d, true);
    assert!(inferred(&off, "Solo::solo_zq").is_empty(), "the switched-off arm must reproduce the silence:\n{off:#}");
    let _ = std::fs::remove_dir_all(&d);
}
