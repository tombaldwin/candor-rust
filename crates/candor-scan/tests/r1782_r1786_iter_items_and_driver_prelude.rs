//! SOUNDNESS R1782 and R1786 — two SILENT under-reports, each EXECUTED before it was fixed (handoff `rust-r1782/`).
//!
//! R1782 — AN `Fs` CALL ON THE ITEMS OF A STD ITERATOR SOURCE. `env::split_paths(..).find_map(|p| p.join(..).exists())` (cc 1.4.7
//! `Build::find_working_gnu_prefix`) read `['Env']` with no `Unknown`: the element resolver had no answer for a std iterator SOURCE, so
//! every binder over it bound the name to nothing. `p.ancestors()` is the method twin. Separately, a one-parameter ADAPTER closure was
//! walked body-only, so a WRITTEN annotation (`|p: PathBuf|`) was never read: over an untyped iterator `p` bound to nothing, and over an
//! `impl Iterator<Item = PathBuf>` it bound to the ITERATOR's own dispatch leaves. EXECUTED: each `s*` unit below stats a real path.
//!
//! R1786 — A RULED DEPENDENCY CALL SPELLED THROUGH THE CALIBRATED CRATE'S OWN RE-EXPORT MODULE. Every sqlx driver does `pub(crate) use
//! sqlx_core::driver_prelude::*;` then `net::connect_tcp(..)`; the scanner spells it `sqlx_core::driver_prelude::net::connect_tcp`, no rule
//! matched, and a calibrated crate's unmatched call reads as reviewed-pure — sqlx-postgres `PgStream::connect`, a real TCP connect, had no
//! row. EXECUTED: 8 spellings, 8 connects; before the fix the 3 without the prelude segment charged `Net` and the 5 through it were ABSENT.
//!
//! Kill switch: `CANDOR_R1782_OFF` (R1782 only; R1786 is a classifier table fact with no switch).

use std::path::{Path, PathBuf};
use std::process::Command;

fn bin() -> &'static str {
    env!("CARGO_BIN_EXE_candor-scan")
}

const ITEMS: &str = r#"use std::{env, ffi::OsStr, path::{Path, PathBuf}};
pub fn s_find_map(v: &OsStr) -> Option<u8> { env::split_paths(v).find_map(|e| if e.join("sh").exists() { Some(1) } else { None }) }
pub fn s_annotated(v: &OsStr) -> bool { env::split_paths(v).any(|p: PathBuf| p.exists()) }
pub fn s_for(v: &OsStr) -> bool { for p in env::split_paths(v) { if p.is_dir() { return true; } } false }
pub fn s_bound(v: &OsStr) -> bool { let it = std::env::split_paths(v); for p in it { if p.is_file() { return true; } } false }
pub fn s_fold(v: &OsStr) -> bool { env::split_paths(v).fold(false, |a, p| a || p.exists()) }
pub fn s_anc(p: &Path) -> bool { p.ancestors().any(|a| a.join("Cargo.toml").exists()) }
pub fn s_anc_for(p: &Path) -> bool { for a in p.ancestors() { if a.join("Cargo.toml").exists() { return true; } } false }
fn opaque() -> impl Iterator<Item = PathBuf> { vec![PathBuf::from("/")].into_iter() }
pub fn s_opq_ann() -> bool { opaque().any(|p: PathBuf| p.exists()) }
pub fn s_opq_param(it: impl Iterator<Item = PathBuf>) -> usize { it.filter(|p: &PathBuf| p.exists()).count() }
// Pure twins: the same sources and binders, with only effect-free members of the item.
pub fn q_split_len(v: &OsStr) -> usize { env::split_paths(v).map(|p| p.as_os_str().len()).sum() }
pub fn q_anc_count(p: &Path) -> usize { p.ancestors().filter(|a| a.as_os_str().len() > 1).count() }
pub fn q_opq_ann() -> usize { opaque().map(|p: PathBuf| p.as_os_str().len()).sum() }
pub fn q_strings(v: Vec<String>) -> usize { v.into_iter().filter(|s: &String| s.len() > 1).count() }
"#;

const PRELUDE: &str = r#"pub(crate) use sqlx_core::driver_prelude::*;
pub struct W;
pub async fn d_direct(h: &str, p: u16) { let _ = sqlx_core::net::connect_tcp(h, p, W).await; }
pub mod stream {
    use crate::net::{self};
    pub async fn d_glob(h: &str, p: u16) { let _ = net::connect_tcp(h, p, crate::W).await; }
}
pub async fn d_glob_full(h: &str, p: u16) { let _ = crate::net::connect_tcp(h, p, W).await; }
pub mod stream3 {
    use crate::net;
    pub async fn d_glob_use_plain(h: &str, p: u16) { let _ = net::connect_tcp(h, p, crate::W).await; }
}
pub async fn d_prelude(h: &str, p: u16) { let _ = sqlx_core::driver_prelude::net::connect_tcp(h, p, W).await; }
pub mod lg {
    use crate::logger::QueryLogger;
    pub fn d_logger(sql: &str) { let _ = QueryLogger::new(sql, Default::default()); }
}
// Pure control through the same prelude: a type re-exported by it, and a module the rules do not charge.
pub fn q_error() -> Option<crate::Error> { None }
"#;

/// R1782 round 2 — the SECOND fixture, and the one the first round's change failed (Fable's probe, mirroring backtrace 0.3.77
/// `macho.rs:17/197` with an EFFECTFUL member). The annotation names a local alias of an associated-type PROJECTION, which the typer
/// cannot dealias. Installing it made `nlist.name()` a typed call with no claimant, dropped with no disclosure: `parse_ann` read
/// `[Unknown]` on 0.42.0 and main and ABSENT on the round-1 branch (`deny Fs Unknown parse_ann` 1/1/0). EXECUTED
/// (`rust-r1782/fixtures/alias`): `parse_ann` reads `/etc/hosts` once per element. The annotation is now taken only when its
/// type resolves; an unresolvable one keeps the untyped route, so the hedge names the lost member. A concrete or one-hop
/// alias annotation still resolves and charges. The imported spellings (`use crate::m::MachNlist`) install a crate-rooted path
/// that is just as unresolvable, so a prefix test would not do.
const ALIAS: &str = r#"use std::fs;
pub trait Hdr { type Nl; }
pub struct Mach;
pub struct Sym;
impl Sym { pub fn name(&self) -> String { fs::read_to_string("/etc/hosts").unwrap() } }
impl Hdr for Mach { type Nl = Sym; }
type MachNlist = <Mach as Hdr>::Nl;
pub fn parse_ann<T: AsRef<[MachNlist]>>(t: T) -> Vec<String> {
    t.as_ref().iter().filter_map(|nlist: &MachNlist| { let n = nlist.name(); Some(n) }).collect()
}
pub fn parse_un<T: AsRef<[MachNlist]>>(t: T) -> Vec<String> {
    t.as_ref().iter().filter_map(|nlist| { let n = nlist.name(); Some(n) }).collect()
}
pub fn parse_conc<T: AsRef<[Sym]>>(t: T) -> Vec<String> {
    t.as_ref().iter().filter_map(|nlist: &Sym| { let n = nlist.name(); Some(n) }).collect()
}
type PlainAlias = Sym;
pub fn parse_plain<T: AsRef<[PlainAlias]>>(t: T) -> Vec<String> {
    t.as_ref().iter().filter_map(|nlist: &PlainAlias| { let n = nlist.name(); Some(n) }).collect()
}
pub mod m { pub type Projected = <super::Mach as super::Hdr>::Nl; }
pub mod user {
    use crate::m::Projected;
    pub fn imp_ann<T: AsRef<[Projected]>>(t: T) -> Vec<String> { t.as_ref().iter().filter_map(|n: &Projected| Some(n.name())).collect() }
    pub fn qual_ann<T: AsRef<[Projected]>>(t: T) -> Vec<String> { t.as_ref().iter().filter_map(|n: &crate::m::Projected| Some(n.name())).collect() }
}
"#;

fn fixture(tag: &str, src: &str, deps: &str) -> PathBuf {
    let root = std::env::temp_dir().join(format!("candor-r1782-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(root.join("app/src")).unwrap();
    std::fs::write(
        root.join("app/Cargo.toml"),
        format!("[package]\nname = \"app\"\nversion = \"0.1.0\"\nedition = \"2021\"\n[dependencies]\n{deps}"),
    )
    .unwrap();
    std::fs::write(root.join("app/src/lib.rs"), src).unwrap();
    root
}

fn scan(root: &Path, policy: Option<&str>, off: bool) -> (i32, serde_json::Value) {
    let mut cmd = Command::new(bin());
    cmd.arg(root.join("app").to_string_lossy().as_ref()).arg("--json");
    if let Some(p) = policy {
        let pf = root.join("policy.txt");
        std::fs::write(&pf, p).unwrap();
        cmd.arg("--policy").arg(&pf);
    }
    for (k, _) in std::env::vars() {
        if k.starts_with("CANDOR_") {
            cmd.env_remove(k);
        }
    }
    if off {
        cmd.env("CANDOR_R1782_OFF", "1");
    }
    let out = cmd.output().expect("run candor-scan");
    let code = out.status.code().unwrap_or(-1);
    // A result-moving switch makes a gate refuse (exit 2, SPEC §3.4) rather than answer.
    let refused_by_switch = off && policy.is_some() && code == 2;
    assert!(code == 0 || code == 1 || refused_by_switch, "scan failed ({code}): {}", String::from_utf8_lossy(&out.stderr));
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(!err.contains("matched NO function"), "a policy rule bound nothing, so its exit says nothing: {err}");
    (code, serde_json::from_slice(&out.stdout).expect("report json"))
}

fn inferred(v: &serde_json::Value, f: &str) -> Vec<String> {
    v["functions"]
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r["fn"].as_str() == Some(f))
        .and_then(|r| r["inferred"].as_array())
        .map(|a| a.iter().filter_map(|e| e.as_str().map(str::to_string)).collect())
        .unwrap_or_default()
}

const CHARGED: [&str; 9] =
    ["s_find_map", "s_annotated", "s_for", "s_bound", "s_fold", "s_anc", "s_anc_for", "s_opq_ann", "s_opq_param"];

#[test]
fn r1782_items_of_std_path_sources_and_written_closure_annotations_are_charged() {
    let d = fixture("items", ITEMS, "");
    let (_, on) = scan(&d, None, false);
    for u in CHARGED {
        assert!(inferred(&on, u).contains(&"Fs".to_string()), "{u} must carry Fs: {on:#}");
    }
    for u in ["q_split_len", "q_anc_count", "q_opq_ann", "q_strings"] {
        assert!(!inferred(&on, u).contains(&"Fs".to_string()), "{u} is a pure twin: {on:#}");
    }
    // The gate flip, on the exact unit (no prefix sibling shares these names).
    for u in ["s_find_map", "s_annotated", "s_for", "s_anc", "s_opq_ann", "s_opq_param"] {
        assert_eq!(scan(&d, Some(&format!("deny Fs  {u}\n")), false).0, 1, "deny Fs {u}");
    }
    let _ = std::fs::remove_dir_all(&d);
}

/// The fixture reaches the changed code: with the switch off every unit reads as it did on 0.42.0 (no `Fs`), and a gate over the switch
/// refuses rather than answering (the 0.42.0 gate passed: exit 0, measured on the published binary).
#[test]
fn r1782_switch_off_reproduces_the_silence() {
    let d = fixture("items-off", ITEMS, "");
    let (_, off) = scan(&d, None, true);
    for u in CHARGED {
        assert!(!inferred(&off, u).contains(&"Fs".to_string()), "{u} was charged with the switch off, so this fixture proves nothing: {off:#}");
    }
    assert_eq!(scan(&d, Some("deny Fs  s_find_map\n"), true).0, 2);
    let _ = std::fs::remove_dir_all(&d);
}

#[test]
fn r1786_a_ruled_call_through_the_driver_prelude_is_charged() {
    let d = fixture("prelude", PRELUDE, "sqlx-core = \"0.9.0\"\n");
    let (_, v) = scan(&d, None, false);
    for u in ["d_direct", "stream::d_glob", "d_glob_full", "stream3::d_glob_use_plain", "d_prelude"] {
        assert_eq!(inferred(&v, u), ["Net"], "{u}: {v:#}");
    }
    assert_eq!(inferred(&v, "lg::d_logger"), ["Clock"], "{v:#}");
    assert!(inferred(&v, "q_error").is_empty(), "{v:#}");
    for u in ["stream::d_glob", "d_glob_full", "stream3::d_glob_use_plain", "d_prelude"] {
        assert_eq!(scan(&d, Some(&format!("deny Net  {u}\n")), false).0, 1, "deny Net {u}");
    }
    let _ = std::fs::remove_dir_all(&d);
}

#[test]
fn r1782_an_annotation_the_typer_cannot_resolve_keeps_the_receiver_disclosed() {
    let d = fixture("alias", ALIAS, "");
    let (_, v) = scan(&d, None, false);
    let why = |f: &str| -> Vec<String> {
        v["functions"].as_array().unwrap().iter().find(|r| r["fn"].as_str() == Some(f))
            .and_then(|r| r["unknownWhy"].as_array())
            .map(|a| a.iter().filter_map(|e| e.as_str().map(str::to_string)).collect()).unwrap_or_default()
    };
    for u in ["parse_ann", "parse_un", "user::imp_ann", "user::qual_ann"] {
        assert_eq!(inferred(&v, u), ["Unknown"], "{u}: {v:#}");
        assert!(why(u).iter().any(|w| w.starts_with("dispatch:untyped receiver of `name`")), "{u}: the hedge must name the lost member: {v:#}");
        assert_eq!(scan(&d, Some(&format!("deny Fs Unknown  {u}\n")), false).0, 1, "deny Fs Unknown {u}");
    }
    for u in ["parse_conc", "parse_plain"] {
        assert_eq!(inferred(&v, u), ["Fs"], "{u}: {v:#}");
        assert_eq!(scan(&d, Some(&format!("deny Fs  {u}\n")), false).0, 1, "deny Fs {u}");
    }
    let _ = std::fs::remove_dir_all(&d);
}
