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
pub mod srv {{ pub mod builder {{ pub struct Builder; pub fn builder() -> Builder {{ Builder }} impl Builder {{ pub fn build(&self) {{ {W} }} }} }}\n\
  pub use builder::builder; pub use builder::Builder;\n\
  pub mod conn {{ pub fn twin_below_zq() {{ super::builder::builder().build() }} pub fn twin_value_zq() {{ crate::srv::builder().build() }} }} }}\n\
pub mod pv {{ use crate::tu as sys; pub mod child {{ use super::sys; pub fn priv_super_zq() {{ sys::fx() }} pub fn priv_path_zq() {{ super::sys::fx() }} }}\n\
  pub mod child2 {{ pub fn priv_crate_zq() {{ crate::pv::sys::fx() }} }} }}\n\
pub mod gl {{ pub mod inner {{ pub fn spl() {{ {W} }} }} pub use inner::spl; }}\n\
pub mod m_shadow {{ use crate::gl::*; fn spl() {{}} pub fn shadow_zq() {{ spl() }} }}\n\
pub mod cli {{ pub mod builder {{ pub struct Builder; pub fn builder() -> Builder {{ Builder }} impl Builder {{ pub fn build(&self) {{}} }} }}\n\
  pub use builder::builder; }}\n\
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
        "pv::child::priv_path_zq",
        "pv::child2::priv_crate_zq",
        "srv::conn::twin_below_zq",
        "srv::conn::twin_value_zq",
    ];
    for f in charged {
        assert!(inferred(&on, f).contains(&"Fs".to_string()), "{f} must carry the re-exported definition's Fs {on:#}");
        let (code, _) = scan(&d, Some(&format!("deny Fs {f}\n")), false, None);
        assert_eq!(code, 1, "deny Fs {f} must fire");
        // The kill switch gives back the old reading: no Fs (absent, or the stage-1a / R1664 disclosure).
        if !f.starts_with("srv::") {
            assert!(!inferred(&off, f).contains(&"Fs".to_string()), "{f}: CANDOR_R1657S2_OFF must restore the old reading {off:#}");
        }
    }
    // A PRIVATE binding read by a descendant through `use super::sys` already resolved (the descendant's own map); through
    // a PATH (`super::sys::fx()`, `crate::pv::sys::fx()`) it named nothing before the table carried private bindings (R1689).
    assert!(inferred(&off, "pv::child::priv_super_zq").contains(&"Fs".to_string()), "{off:#}");
    // h3's shape: `pub use builder::builder;` beside `mod builder;` — the fn re-export must not redirect a path written
    // BELOW the module (`super::builder::builder()`), which aacd3880 resolved and a first cut of the table broke.
    assert!(inferred(&off, "srv::conn::twin_below_zq").contains(&"Fs".to_string()), "the module path resolves without the table {off:#}");
    // Pure twins with unique names: charged on NO arm.
    // rayon's shape: a module's own `fn spl` shadows the glob's re-exported `spl` (a one-segment name is any namespace's):
    // never the glob's `Fs` (the bare leaf's own `ambiguous:` hedge may stand, as on aacd3880).
    assert!(!inferred(&on, "m_shadow::shadow_zq").contains(&"Fs".to_string()), "{on:#}");
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

/// Two shapes the corpus release bar found, both EXECUTED (lane scratch `rustagent-stage2/fx/ffi`, the binary calls libc):
///   * a cfg `#[path]` arm set re-exporting a FOREIGN DECLARATION (generator's `pub use self::asm::swap_registers`): the
///     table names each arm's declaration exactly; declarations are not in the tail2 index, so the first cut formed no
///     edge and the caller lost its `native:extern fn` (Unknown -> ABSENT). `arm_extern_zq` must keep its Unknown.
///   * R1690 — a call through a GLOB re-export of a module whose items an unexpanded macro hides (objc2's
///     `ffi::objc_autoreleaseReturnValue`, declared inside `extern_c_unwind!`, reached through `pub use self::rc::*`):
///     ABSENT on published 0.41.3 and aacd3880, now disclosed `macro:`. Here the macro is a dependency's (unreadable).
#[test]
fn r1690_foreign_declarations_behind_reexports_stay_disclosed() {
    let mac = fixture("macdep", "macdep", &[("lib.rs", "#[macro_export]\nmacro_rules! ext { ($($t:tt)*) => { extern \"C\" { $($t)* } } }\n")], "");
    let src = "mod ffi { mod rc; pub use self::rc::*; }\n\
mod detail { #[cfg_attr(unix, path = \"a.rs\")] #[cfg_attr(windows, path = \"b.rs\")] mod asm; pub use self::asm::labs; }\n\
pub mod user { pub fn glob_macro_zq() -> i32 { use crate::ffi; unsafe { ffi::my_abs_r1690(-3) } }\n\
  pub fn arm_extern_zq() -> i64 { use crate::detail::labs; unsafe { labs(-4) } } }\n\
fn main() { println!(\"{} {}\", user::glob_macro_zq(), user::arm_extern_zq()); }\n";
    let ext = "extern \"C\" { pub fn labs(x: i64) -> i64; }\n";
    let d = fixture(
        "ffi",
        "ffis2",
        &[
            ("main.rs", src),
            ("ffi/rc.rs", "macdep::ext! { #[link_name = \"abs\"] pub fn my_abs_r1690(x: i32) -> i32; }\n"),
            ("detail/a.rs", ext),
            ("detail/b.rs", ext),
        ],
        &format!("macdep = {{ path = \"{}\" }}\n", mac.display()),
    );
    let (_, on) = scan(&d, None, false, None);
    assert_eq!(inferred(&on, "user::arm_extern_zq"), ["Unknown"], "{on:#}");
    assert_eq!(inferred(&on, "user::glob_macro_zq"), ["Unknown"], "{on:#}");
    let w = row(&on, "user::glob_macro_zq").and_then(|r| r["unknownWhy"].as_array()).cloned().unwrap_or_default();
    assert!(w.iter().any(|x| x.as_str() == Some("macro:module items hidden by an unexpanded macro")), "{on:#}");
    let (_, off) = scan(&d, None, true, None);
    assert!(row(&off, "user::glob_macro_zq").is_none(), "pre-stage-2: ABSENT over a foreign call {off:#}");
    let _ = std::fs::remove_dir_all(&d);
    let _ = std::fs::remove_dir_all(&mac);
}

/// pnet_sys's shape: `extern crate libc;` in a module whose child PRIVATELY imports it (`use super::libc;`) and which globs
/// that child (`use self::public::*`). `libc::sendto(..)` names the extern crate; the first cut of the private-binding table
/// sent it through the glob to a crate-local path and the classifier's `Net` was lost. The extern crate (and any manifest
/// dependency's name) is never a table edge, and a module's own `extern crate` beats a glob.
#[test]
fn r1657s2_an_extern_crate_rebound_privately_is_not_a_local_item() {
    let src = "pub mod imp {\n\
extern crate libc;\n\
pub mod public { use super::libc; pub type CSocket = libc::c_int; }\n\
use self::public::*;\n\
pub unsafe fn sendto_zq(s: CSocket, b: *const libc::c_void, n: libc::size_t) -> libc::ssize_t { libc::sendto(s, b, n, 0, std::ptr::null(), 0) } }\n\
fn main() {}\n";
    let d = fixture("pnet", "pnets2", &[("main.rs", src)], "libc = \"0.2\"\n");
    let (_, on) = scan(&d, None, false, None);
    assert!(inferred(&on, "imp::sendto_zq").contains(&"Net".to_string()), "{on:#}");
    let _ = std::fs::remove_dir_all(&d);
}


/// PART 89's draft cfg-gated-local-type cell (lane scratch `rustagent-stage2/spec/part89-cfglocal.patch`): one `#[cfg]` arm
/// RE-EXPORTS `Tool` from another module, the other DECLARES a local `Tool`. EXECUTED on macOS (`fx/p89b10`): the unix arm
/// is built and sets the env var; published 0.41.3 and aacd3880 charge `Fs` ALONE (the cfg-off declaration picked), so
/// `deny Env probe` exited 0 over a real `set_var`. The re-export table unions the declared arm with the re-exported one.
#[test]
fn r1657s2_a_cfg_gated_local_type_beside_a_reexport_arm_is_the_union() {
    for (tag, re, local) in [("p89a", "unix", "not(unix)"), ("p89b", "not(unix)", "unix")] {
        let src = format!(
            "pub mod a {{ pub struct Tool; impl Tool {{ pub fn act() {{ std::env::set_var(\"CANDOR_PART89\", \"1\"); }} }} }}\n\
pub mod sys {{ #[cfg({re})] pub use crate::a::Tool; #[cfg({local})] pub struct Tool;\n\
  #[cfg({local})] impl Tool {{ pub fn act() {{ let _ = std::fs::write(\"/tmp/benign\", b\"x\"); }} }} }}\n\
use crate::sys::Tool;\npub fn probe() {{ Tool::act(); }}\n"
        );
        let d = fixture(tag, "p89s2", &[("lib.rs", src.as_str())], "");
        let (_, on) = scan(&d, None, false, None);
        assert_eq!(inferred(&on, "probe"), ["Env", "Fs"], "{tag}: the union of both arms {on:#}");
        let (rc, _) = scan(&d, Some("deny Env probe\n"), false, None);
        assert_eq!(rc, 1, "{tag}: deny Env must fire");
        let (_, off) = scan(&d, None, true, None);
        assert!(!inferred(&off, "probe").contains(&"Env".to_string()), "{tag}: pre-stage-2 picked the local arm {off:#}");
        let _ = std::fs::remove_dir_all(&d);
    }
}

/// One scan with explicit extra env; returns (exit code, report, stderr). `deps` chains.
fn scan_env(dir: &Path, deps: Option<&Path>, env: &[(&str, &str)]) -> (serde_json::Value, String) {
    let mut cmd = Command::new(bin());
    cmd.arg(dir.to_string_lossy().as_ref()).arg("--json");
    for v in ["CANDOR_DEPS", "CANDOR_CONFIG", "CANDOR_POLICY", "CANDOR_R1657S2_OFF", "CANDOR_R1657S2B_OFF", "CANDOR_R1657S2_INSTR"] {
        cmd.env_remove(v);
    }
    if let Some(d) = deps {
        cmd.env("CANDOR_DEPS", d);
    }
    for (k, v) in env {
        cmd.env(k, v);
    }
    let out = cmd.output().expect("run candor-scan");
    assert!(out.status.code().is_some_and(|c| (0..=2).contains(&c)), "{}", String::from_utf8_lossy(&out.stderr));
    (serde_json::from_slice(&out.stdout).expect("report json"), String::from_utf8_lossy(&out.stderr).into_owned())
}

/// R1657 stage 2b — an arm that ENDS in a module that globs something this scan cannot enumerate is not a determined
/// definition. toml_edit: `pub use toml_datetime::*;` at the root binds `Datetime`; `pub(crate) use crate::Datetime;` in
/// `visit` was filed as `visit::Datetime -> crate::Datetime`, an arm that names nothing. `undetermined` counted only LOCAL
/// globs. EXECUTED family (lane scratch `rustagent-stage2b/fam/gen2.py`, 5 variants x every consumer spelling, each spelling
/// with a uniquely named pure twin): every consumer reads identically on published 0.41.3, aacd3880, the stage-2 tip and this
/// one, so the arm was never observable as a behaviour — the refusal is pinned through the reach counters, and the
/// SPELLINGS that must keep reading as they did are pinned beside it.
///
/// The exception that must NOT be lost: a name the glob-bearing module DECLARES ITSELF (here the fn `helper_zq`, which the
/// table's types-only `declared` set does not hold) is answered by the declaration, not by the glob.
#[test]
fn r1657s2b_an_arm_ending_behind_an_external_glob_is_refused_and_the_modules_own_names_are_not() {
    let pd_src = format!(
        "pub struct Datetime;\nimpl Datetime {{ pub fn parse(_s: &str) -> Datetime {{ {W} Datetime }} pub fn inert() -> Datetime {{ Datetime }} }}\n\
pub fn go_zq() {{ {W} }}\npub fn quiet_zq() {{}}\n"
    );
    let pd = fixture("g2pd", "dgdep", &[("lib.rs", pd_src.as_str())], "");
    let cons_src = format!(
        "#![allow(unused_imports, dead_code)]\n\
pub use dgdep::*;\n\
pub mod visit {{\n\
    pub(crate) use crate::Datetime;\n    pub(crate) use crate::go_zq;\n    pub(crate) use crate::quiet_zq;\n\
    pub fn v_assoc_zq() {{ let _ = Datetime::parse(\"a\"); }}\n    pub fn v_calm_zq() {{ let _ = Datetime::inert(); }}\n\
    pub fn v_fn_zq() {{ go_zq(); }}\n    pub fn v_quiet_zq() {{ quiet_zq(); }}\n\
}}\n\
pub mod other {{\n    pub fn o_direct_zq() {{ let _ = crate::Datetime::parse(\"c\"); }}\n    pub fn o_calm_zq() {{ let _ = crate::Datetime::inert(); }}\n}}\n\
pub mod util {{\n    pub use dgdep::*;\n    pub fn helper_zq() {{ {W} }}\n    pub fn tame_zq() {{}}\n}}\n\
pub mod facade {{\n    pub use crate::util::helper_zq;\n    pub use crate::util::tame_zq;\n}}\n\
pub mod user {{\n    pub fn u_helper_zq() {{ crate::facade::helper_zq() }}\n    pub fn u_tame_zq() {{ crate::facade::tame_zq() }}\n}}\n"
    );
    let cons = fixture("g2cons", "g2cons", &[("lib.rs", cons_src.as_str())], &format!("dgdep = {{ path = \"{}\" }}\n", pd.display()));
    let deps = std::env::temp_dir().join(format!("candor-r1657s2b-deps-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&deps);
    produce(&pd, &deps, "dgdep", false);

    let (on, err_on) = scan_env(&cons, Some(&deps), &[("CANDOR_R1657S2_INSTR", "1")]);
    let (off2b, err_off2b) = scan_env(&cons, Some(&deps), &[("CANDOR_R1657S2B_OFF", "1"), ("CANDOR_R1657S2_INSTR", "1")]);
    let (off2, _) = scan_env(&cons, Some(&deps), &[("CANDOR_R1657S2_OFF", "1")]);

    // The fixture REACHES the refusal: the arm is refused here and filed with the stage-2b switch off.
    assert!(err_on.contains("R1657S2GLOBREFUSE\tvisit::Datetime\tcrate::Datetime"), "refused: {err_on}");
    assert!(!err_on.contains("R1657S2NAME\tvisit::Datetime\t"), "not filed: {err_on}");
    assert!(err_off2b.contains("R1657S2NAME\tvisit::Datetime\tcrate::Datetime"), "stage-2 filed an arm naming nothing: {err_off2b}");
    // ...and the module's OWN declared names stay determined: `facade::helper_zq -> crate::util::helper_zq` is filed.
    assert!(err_on.contains("R1657S2NAME\tfacade::helper_zq\tcrate::util::helper_zq"), "declared name kept: {err_on}");
    // Every consumer spelling reads the same on both and the effectful ones charge, the pure twins stay pure.
    for f in ["visit::v_assoc_zq", "visit::v_fn_zq", "other::o_direct_zq"] {
        for (arm, r) in [("on", &on), ("2b off", &off2b)] {
            assert!(inferred(r, f).contains(&"Fs".to_string()), "{arm}: {f} charges chained {r:#}");
        }
    }
    for f in ["visit::v_calm_zq", "visit::v_quiet_zq", "other::o_calm_zq"] {
        for (arm, r) in [("on", &on), ("2b off", &off2b)] {
            assert!(!inferred(r, f).contains(&"Fs".to_string()), "{arm}: pure twin {f} stays pure {r:#}");
        }
    }
    // The module's own fn, reached through a re-export of it: resolved by the table (ABSENT before stage 2), still resolved.
    assert!(inferred(&on, "user::u_helper_zq").contains(&"Fs".to_string()), "declared fn behind an external-glob module still resolves {on:#}");
    assert!(!inferred(&on, "user::u_tame_zq").contains(&"Fs".to_string()), "its pure twin stays pure {on:#}");
    assert!(inferred(&off2, "user::u_helper_zq").contains(&"Fs".to_string()), "(the free-fn tail2 route already answered it) {off2:#}");
    let _ = std::fs::remove_dir_all(&deps);
    let _ = std::fs::remove_dir_all(&pd);
    let _ = std::fs::remove_dir_all(&cons);
}
