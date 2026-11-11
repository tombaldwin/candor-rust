//! SOUNDNESS R1657 stage 2 — ONE RE-EXPORT TABLE: A NAME A LOCAL `use` RE-EXPORTS DENOTES THE DEFINITION(S) BEHIND IT, ON
//! EVERY READER THAT ASKS `expand`.
//!
//! A local NAMED re-export (`pub use crate::tu as sys;`, `pub(crate) use crate::a::sys;`, `pub use handle::Handle;`,
//! `pub use segment::Tbl as RenamedTbl;`, `pub use io::w as root_w;`) reached exactly two readers — the free-fn tail2 index
//! (`reexport_target`) and R959's synthetic unit — through `Reexport`. Every reader that types a value or expands a path
//! (`expand`, hence `type_path`, field / return / payload types, ctor paths, globs) saw the RE-EXPORT spelling, which names no
//! definition, and the call was ABSENT. Now the crate-wide table (`lang::reexport_lookup`, built by
//! `decls::build_reexport_names`, every chain followed to its end and every `#[cfg]` arm kept) answers on a `use`-map miss.
//!
//! Measured over a 328-entry generated family (lane scratch `rustagent-stage2/fam/gen.py`: 26 bindings — module, fn and type
//! re-exports; renamed / not; `pub` / `pub(crate)`; inline / file / child modules; 3-module chains; a `#[cfg]` arm set
//! chained; `super::` and uniform one-segment paths — times every consumer spelling, each with a uniquely named PURE twin),
//! every entry EXECUTED on macOS: 164 effectful entries SILENT 108 on published 0.41.3, 80 on aacd3880 (+28 disclosed by the
//! R1664 floor), 0 here; the 164 pure twins read pure on every arm. Rows: R1642, R1664, R1684 (residuals), R1579 (hop 1).
//!
//! Each case asserts the effect AND `deny Fs <fn>` exiting 1; `CANDOR_R1657S2_OFF` restores the old reading, which proves the
//! fixture reaches the code. Fixture fns carry unique `_zq` names because `deny <E> <fn>` matches by PREFIX.

use std::path::{Path, PathBuf};
use std::process::Command;

fn bin() -> &'static str {
    env!("CARGO_BIN_EXE_candor-scan")
}

fn fixture(tag: &str, name: &str, files: &[(&str, &str)], deps: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("candor-r1657s2-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(d.join("src")).unwrap();
    std::fs::write(
        d.join("Cargo.toml"),
        format!("[package]\nname = \"{name}\"\nversion = \"0.1.0\"\nedition = \"2021\"\n[workspace]\n[dependencies]\n{deps}"),
    )
    .unwrap();
    for (p, s) in files {
        let f = d.join("src").join(p);
        std::fs::create_dir_all(f.parent().unwrap()).unwrap();
        std::fs::write(f, s).unwrap();
    }
    d
}

/// The report (`--json`), and the exit code of `policy` when one is given. `off` sets `CANDOR_R1657S2_OFF`; `deps` chains.
fn scan(dir: &Path, policy: Option<&str>, off: bool, deps: Option<&Path>) -> (i32, serde_json::Value) {
    let mut cmd = Command::new(bin());
    cmd.arg(dir.to_string_lossy().as_ref()).arg("--json");
    if let Some(p) = policy {
        let pf = dir.join("policy.txt");
        std::fs::write(&pf, p).unwrap();
        cmd.arg("--policy").arg(&pf);
    }
    for v in ["CANDOR_DEPS", "CANDOR_CONFIG", "CANDOR_POLICY", "CANDOR_R1657S2_OFF", "CANDOR_R1664_OFF"] {
        cmd.env_remove(v);
    }
    if off {
        cmd.env("CANDOR_R1657S2_OFF", "1");
    }
    if let Some(d) = deps {
        cmd.env("CANDOR_DEPS", d);
    }
    let out = cmd.output().expect("run candor-scan");
    let code = out.status.code().unwrap_or(-1);
    assert!((0..=2).contains(&code), "scan failed ({code}): {}", String::from_utf8_lossy(&out.stderr));
    (code, serde_json::from_slice(&out.stdout).expect("report json"))
}

/// Scan `dir` as a dependency report into `<out>/<name>` (a `CANDOR_DEPS` directory).
fn produce(dir: &Path, out: &Path, name: &str, off: bool) {
    std::fs::create_dir_all(out).unwrap();
    let mut cmd = Command::new(bin());
    cmd.arg(dir.to_string_lossy().as_ref()).arg("--out").arg(out.join(name));
    for v in ["CANDOR_DEPS", "CANDOR_CONFIG", "CANDOR_POLICY", "CANDOR_R1657S2_OFF"] {
        cmd.env_remove(v);
    }
    if off {
        cmd.env("CANDOR_R1657S2_OFF", "1");
    }
    let st = cmd.output().expect("run candor-scan");
    assert!(st.status.code().is_some_and(|c| (0..=2).contains(&c)), "{}", String::from_utf8_lossy(&st.stderr));
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

const W: &str = "std::fs::write(std::env::temp_dir().join(\"r1657s2\"), b\"x\").unwrap();";

/// The in-crate spellings: a module re-exported by name and by glob from a NON-root module (R1684 residual), a `pub(crate)`
/// chain over three modules whose first hop is a `#[cfg]` arm set (R1684 residual), a renamed fn re-export reached through a
/// glob as a ONE-segment name, a renamed TYPE re-export (R1664), a type re-exported un-renamed from a private submodule (the
/// stage-1a class-A residue, tokio's `pub use handle::Handle`), and a uniform one-segment `pub use inner as sys;`.
fn in_crate_src() -> String {
    format!(
        "#![allow(unused_imports, dead_code)]\n\
pub mod tu {{ pub fn fx() {{ {W} }} pub struct Tbl; impl Tbl {{ pub fn poke(&self) {{ {W} }} }} }}\n\
pub mod tq {{ pub fn qx() {{}} pub struct Qbl; impl Qbl {{ pub fn qpoke(&self) {{}} }} }}\n\
#[cfg(windows)] pub mod tw {{ pub fn fx() {{}} }}\n\
pub mod plat {{ pub use crate::tu as sys; pub use crate::tq as qsys; pub use crate::tu::fx as gx; }}\n\
pub mod m_named {{ use crate::plat::sys; pub fn named_zq() {{ sys::fx() }} }}\n\
pub mod m_glob {{ use crate::plat::*; pub fn glob_zq() {{ sys::fx() }} pub fn glob_fn_zq() {{ gx() }} }}\n\
pub mod m_path {{ pub fn path_zq() {{ crate::plat::sys::fx() }} pub fn pure_ctl_zq() {{ crate::plat::qsys::qx() }} }}\n\
pub mod a {{ #[cfg(windows)] pub(crate) use crate::tw as sys; #[cfg(unix)] pub(crate) use crate::tu as sys; }}\n\
pub mod b {{ pub(crate) use crate::a::sys; }}\n\
pub mod c {{ pub(crate) use crate::b::sys; }}\n\
pub mod m_chain {{ use crate::c::sys; pub fn chain_zq() {{ sys::fx() }} }}\n\
pub mod cht {{ pub(crate) use crate::tu::Tbl as RenamedTbl; pub(crate) use crate::tq::Qbl as RenamedPure; }}\n\
pub mod m_ty {{ use crate::cht::RenamedTbl; use crate::cht::RenamedPure; type Store = crate::cht::RenamedTbl;\n\
  pub fn ty_alias_zq(c: &Store) {{ c.poke() }} pub fn ty_path_zq(c: &crate::cht::RenamedTbl) {{ c.poke() }}\n\
  pub fn ty_imported_zq(c: &RenamedTbl) {{ c.poke() }} pub fn ty_pure_twin_zq(c: &RenamedPure) {{ c.qpoke() }} }}\n\
pub mod runtime {{ mod handle {{ pub struct EngR; impl EngR {{ pub fn run(&self) {{ {W} }} }} pub struct Handle {{ pub inner: EngR }} }}\n\
  pub use handle::{{EngR, Handle}}; }}\n\
pub mod other {{ pub struct EngO; impl EngO {{ pub fn run(&self) {{}} }} pub struct Handle {{ pub inner: EngO }} }}\n\
pub mod m_owner {{ pub fn owner_zq(h: &crate::runtime::Handle) {{ h.inner.run(); }} pub fn owner_other_zq(h: &crate::other::Handle) {{ h.inner.run(); }} }}\n\
pub mod uni {{ pub mod inner {{ pub fn fx() {{ {W} }} }} pub use inner as sys; }}\n\
pub mod m_uni {{ use crate::uni::sys; pub fn uni_zq() {{ sys::fx() }} }}\n\
fn main() {{}}\n"
    )
}

#[test]
fn r1657s2_reexported_names_resolve_on_every_reader() {
    let src = in_crate_src();
    let d = fixture("inc", "fxs2", &[("main.rs", src.as_str())], "");
    let (_, on) = scan(&d, None, false, None);
    let (_, off) = scan(&d, None, true, None);
    let charged = [
        "m_named::named_zq",
        "m_glob::glob_zq",
        "m_glob::glob_fn_zq",
        "m_path::path_zq",
        "m_chain::chain_zq",
        "m_ty::ty_alias_zq",
        "m_ty::ty_path_zq",
        "m_ty::ty_imported_zq",
        "m_owner::owner_zq",
        "m_uni::uni_zq",
    ];
    for f in charged {
        assert!(inferred(&on, f).contains(&"Fs".to_string()), "{f} must carry the re-exported definition's Fs {on:#}");
        let (code, _) = scan(&d, Some(&format!("deny Fs {f}\n")), false, None);
        assert_eq!(code, 1, "deny Fs {f} must fire");
        // The kill switch gives back the old reading: no Fs (absent, or the stage-1a / R1664 disclosure).
        assert!(!inferred(&off, f).contains(&"Fs".to_string()), "{f}: CANDOR_R1657S2_OFF must restore the old reading {off:#}");
    }
    // Pure twins with unique names: charged on NO arm.
    for f in ["m_path::pure_ctl_zq", "m_ty::ty_pure_twin_zq", "m_owner::owner_other_zq"] {
        assert!(!inferred(&on, f).contains(&"Fs".to_string()), "{f} is pure {on:#}");
        assert!(!inferred(&on, f).contains(&"Unknown".to_string()), "{f} resolves to a pure definition {on:#}");
    }
    // The stage-1a / R1664 disclosure is what the switch restores on the two type spellings it covered.
    assert_eq!(inferred(&off, "m_owner::owner_zq"), ["Unknown"], "{off:#}");
    assert_eq!(inferred(&off, "m_ty::ty_path_zq"), ["Unknown"], "{off:#}");
    let _ = std::fs::remove_dir_all(&d);
}

/// R1642 — a RENAME OF A RE-EXPORT (`pub use io::w as root_w;` over `pub use rw::w`) published no unit under `root_w`, so a
/// chained consumer's `pd::root_w()` matched nothing and read PURE (and chaining DELETED the unchained `invisible:[pd]`). The
/// R959 synthetic unit now follows the re-export table to the definition. Control: the rename of the DEFINITION (`root_w2`).
#[test]
fn r1642_a_rename_of_a_reexport_publishes_a_unit_a_chained_consumer_joins() {
    let pd_src = format!(
        "pub mod io {{ pub(crate) mod rw {{ pub fn w() {{ {W} }} pub fn calm() {{}} }} pub use rw::{{calm, w}}; }}\n\
pub use io::w as root_w;\npub use io::rw::w as root_w2;\npub use io::calm as root_calm;\n"
    );
    let pd = fixture("pd", "pds2", &[("lib.rs", pd_src.as_str())], "");
    let cons_src = "pub fn c_root_zq() { pds2::root_w() }\npub fn c_root2_zq() { pds2::root_w2() }\npub fn c_calm_zq() { pds2::root_calm() }\n";
    let cons = fixture("cons", "conss2", &[("lib.rs", cons_src)], &format!("pds2 = {{ path = \"{}\" }}\n", pd.display()));
    for off in [false, true] {
        let deps = std::env::temp_dir().join(format!("candor-r1657s2-deps{}-{}", off as u8, std::process::id()));
        let _ = std::fs::remove_dir_all(&deps);
        produce(&pd, &deps, "pds2", off);
        let (_, r) = scan(&cons, None, off, Some(&deps));
        assert!(inferred(&r, "c_root2_zq").contains(&"Fs".to_string()), "control: the R959 shape joins on every arm {r:#}");
        assert!(!inferred(&r, "c_calm_zq").contains(&"Fs".to_string()), "the pure rename stays pure {r:#}");
        if off {
            assert!(row(&r, "c_root_zq").is_none(), "pre-stage-2: the rename of a re-export reads ABSENT chained {r:#}");
        } else {
            assert!(inferred(&r, "c_root_zq").contains(&"Fs".to_string()), "R1642: joined through the new unit {r:#}");
            let pol = cons.join("p1642.txt");
            std::fs::write(&pol, "deny Fs c_root_zq\n").unwrap();
            let st = Command::new(bin())
                .arg(cons.to_string_lossy().as_ref())
                .arg("--policy")
                .arg(&pol)
                .env("CANDOR_DEPS", &deps)
                .env_remove("CANDOR_R1657S2_OFF")
                .env_remove("CANDOR_CONFIG")
                .env_remove("CANDOR_POLICY")
                .output()
                .unwrap();
            assert_eq!(st.status.code(), Some(1), "deny Fs c_root_zq must fire chained");
        }
        let _ = std::fs::remove_dir_all(&deps);
    }
    let _ = std::fs::remove_dir_all(&pd);
    let _ = std::fs::remove_dir_all(&cons);
}

/// R1579 — a dependency's free fn returning a type written through a re-export (`pub use context::Context;` in a NON-root
/// module, `pub fn new() -> Context`) published no return type, so the consumer's receiver was untyped and disclosed. Hop 1
/// now joins. THE HAZARD the row names, pinned: a HOP 2 whose return the surface cannot carry (`-> super::stream::Stream`,
/// and through `?` on a `Result`) must stay DISCLOSED, not read pure through the joined hop 1.
#[test]
fn r1579_a_reexported_return_type_joins_and_hop_two_stays_disclosed() {
    let pd_src = format!(
        "pub mod js {{\n\
  pub mod context {{ pub struct Context; impl Context {{ pub fn publish(&self) {{ {W} }}\n\
    pub fn get_stream(&self) -> super::stream::Stream {{ super::stream::Stream }}\n\
    pub fn get_stream_res(&self) -> Result<super::stream::Stream, String> {{ Ok(super::stream::Stream) }} }} }}\n\
  pub mod stream {{ pub struct Stream; impl Stream {{ pub fn get_info(&self) {{ {W} }} }} }}\n\
  pub use context::Context;\n\
  pub fn new() -> Context {{ Context }}\n}}\n"
    );
    let pd = fixture("pdjs", "pdjs2", &[("lib.rs", pd_src.as_str())], "");
    let cons_src = "pub fn hop1_zq() { let j = pdjs2::js::new(); j.publish(); }\n\
pub fn hop2_zq() { let j = pdjs2::js::new(); let s = j.get_stream(); s.get_info(); }\n\
pub fn hop2res_zq() -> Result<(), String> { let j = pdjs2::js::new(); let s = j.get_stream_res()?; s.get_info(); Ok(()) }\n";
    let cons = fixture("consjs", "consjs2", &[("lib.rs", cons_src)], &format!("pdjs2 = {{ path = \"{}\" }}\n", pd.display()));
    let deps = std::env::temp_dir().join(format!("candor-r1657s2-depsjs-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&deps);
    produce(&pd, &deps, "pdjs2", false);
    let (_, r) = scan(&cons, None, false, Some(&deps));
    assert_eq!(inferred(&r, "hop1_zq"), ["Env", "Fs"], "hop 1 joins through the qualified return {r:#}");
    for f in ["hop2_zq", "hop2res_zq"] {
        let inf = inferred(&r, f);
        assert!(inf.contains(&"Fs".to_string()) || inf.contains(&"Unknown".to_string()), "{f}: hop 2 must not go silent {r:#}");
    }
    let _ = std::fs::remove_dir_all(&deps);
    produce(&pd, &deps, "pdjs2", true);
    let (_, r) = scan(&cons, None, true, Some(&deps));
    assert_eq!(inferred(&r, "hop1_zq"), ["Unknown"], "pre-stage-2: the hop-1 receiver is untyped and disclosed {r:#}");
    let _ = std::fs::remove_dir_all(&deps);
    let _ = std::fs::remove_dir_all(&pd);
    let _ = std::fs::remove_dir_all(&cons);
}
