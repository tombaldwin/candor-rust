//! SOUNDNESS R1627 — `ExponentialBackoffBuilder::new().build().next_backoff()` (and its `let`, `&mut Self`-step, re-exported and
//! closure spellings) over an UNCHAINED dependency read `[Clock]` only: R1200's method-probe rule refused to type an OWNED
//! receiver whose inherent method takes `&self` (`build`), so the value stayed typed as the chain's root and `next_backoff` met
//! no rule. EXECUTED in the lane (`~/…/rustagent-r1627/fx`, real `backoff` 0.4.0, DYLD interposer on `getentropy`): every `f_*`
//! spelling draws entropy once; `deny Rand <unit>` 0 → 1 on each.
//!
//! The probe only matters when a TRAIT method of the same name is in scope at the consumer, so the refusal is lifted by a proof
//! about the CONSUMER (`depsig_probe.rs`): no trait in scope declares the method. The `h_*` consumers each hold one thing that
//! breaks that proof — a local blanket trait with a by-value `build`, an imported dependency trait that declares it, a glob from
//! the dependency, a statement-position foreign macro — and must stay untyped. `CANDOR_R1627_OFF=1` restores the published
//! behaviour, and every charged case must then read uncharged: the proof the fixtures reach the new code.

use std::path::{Path, PathBuf};
use std::process::Command;

fn bin() -> &'static str {
    env!("CARGO_BIN_EXE_candor-scan")
}

fn tmpdir(tag: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("candor-r1627-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn write(p: &Path, s: &str) {
    std::fs::create_dir_all(p.parent().unwrap()).unwrap();
    std::fs::write(p, s).unwrap();
}

/// A hermetic `backoff` (the classified crate's NAME, the shapes that refused): `ExponentialBackoffBuilder::build(&self)` and
/// the `&mut Self` steps, a root `pub use`, a trait `Extra` with a blanket by-value `build`, a `prelude` glob. `fmac`: a foreign
/// item macro in the crate (`pin_project!`) sends typing down the per-item proof instead of `judged`.
fn dep(root: &Path, fmac: bool) {
    let d = root.join("backoff");
    write(&d.join("Cargo.toml"), "[package]\nname = \"backoff\"\nversion = \"0.4.0\"\nedition = \"2021\"\n[dependencies]\npin-project-lite = \"0.2\"\n");
    write(
        &d.join("src/lib.rs"),
        &format!(
            "pub mod exponential;\npub mod extra;\npub mod backoff {{ pub trait Backoff {{}} }}\npub mod prelude {{ pub use crate::extra::Extra; }}\n\
             pub use crate::exponential::{{ExponentialBackoff, ExponentialBackoffBuilder}};\n{}{}",
            if fmac { "pin_project_lite::pin_project! { pub struct Fut<F> { #[pin] f: F } }\n" } else { "" },
            "#[macro_export] macro_rules! wrap { ($e:expr) => {{ use $crate::extra::Extra; $e }}; }\n"
        ),
    );
    write(
        &d.join("src/exponential.rs"),
        "pub struct ExponentialBackoffBuilder { n: u32 }\npub struct ExponentialBackoff { n: u32 }\n\
         impl ExponentialBackoffBuilder {\n pub fn new() -> Self { ExponentialBackoffBuilder { n: 0 } }\n \
         pub fn with_multiplier(&mut self, _m: f64) -> &mut Self { self }\n pub fn with_max(&mut self, _m: u32) -> &mut Self { self }\n \
         pub fn build(&self) -> ExponentialBackoff { ExponentialBackoff { n: self.n } }\n \
         pub fn into_parts(self) -> ExponentialBackoff { ExponentialBackoff { n: self.n } }\n}\n\
         impl ExponentialBackoff {\n pub fn next_backoff(&mut self) -> Option<u32> { None }\n pub fn reset(&mut self) { self.n = 0; }\n}\n",
    );
    write(&d.join("src/extra.rs"), "pub trait Extra { fn build(self) -> u8; }\nimpl<T> Extra for T { fn build(self) -> u8 { 0 } }\n");
}

const MAIN_FNS: &str = "\
use backoff::ExponentialBackoffBuilder;\nmod r { pub use backoff::ExponentialBackoffBuilder as EBB; }\n\
pub fn s_chain_path() -> Option<u32> { backoff::ExponentialBackoffBuilder::new().build().next_backoff() }\n\
pub fn s_chain_use() -> Option<u32> { ExponentialBackoffBuilder::new().build().next_backoff() }\n\
pub fn s_chain_reexp() -> Option<u32> { r::EBB::new().build().next_backoff() }\n\
pub fn s_chain_with() -> Option<u32> { ExponentialBackoffBuilder::new().with_multiplier(2.0).with_max(3).build().next_backoff() }\n\
pub fn s_let_b() -> Option<u32> { let mut b = ExponentialBackoffBuilder::new(); let mut e = b.build(); e.next_backoff() }\n\
pub fn s_let_with() -> Option<u32> { let mut b = ExponentialBackoffBuilder::new(); b.with_multiplier(2.0); let mut e = b.build(); e.next_backoff() }\n\
pub fn s_let_built() -> Option<u32> { let mut e = ExponentialBackoffBuilder::new().build(); e.next_backoff() }\n\
pub fn s_closure() -> Option<u32> { let f = || ExponentialBackoffBuilder::new().build().next_backoff(); f() }\n\
pub fn s_closure_let() -> Option<u32> { let f = |x: u32| { let mut e = ExponentialBackoffBuilder::new().build(); let _ = x; e.next_backoff() }; f(1) }\n\
fn mk_builder() -> ExponentialBackoffBuilder { ExponentialBackoffBuilder::new() }\n\
pub fn s_via_local_fn() -> Option<u32> { mk_builder().build().next_backoff() }\n\
pub fn t_build_only() -> backoff::ExponentialBackoff { ExponentialBackoffBuilder::new().build() }\n\
pub fn t_chain_reset() { ExponentialBackoffBuilder::new().build().reset() }\n\
pub fn t_let_reset() { let mut e = ExponentialBackoffBuilder::new().build(); e.reset() }\n\
pub fn t_with_only() { let mut b = ExponentialBackoffBuilder::new(); b.with_multiplier(2.0).with_max(1); }\n\
pub fn t_local_let() -> Option<u32> { let mut e = loc::B::new().build(); e.next_backoff() }\n\
pub fn t_local_chain() -> Option<u32> { loc::B::new().build().next_backoff() }\n\
mod loc { pub struct B; pub struct E; impl B { pub fn new() -> B { B } pub fn build(&self) -> E { E } } impl E { pub fn next_backoff(&mut self) -> Option<u32> { None } } }\n";

const CHARGED: &[&str] = &[
    "s_chain_path", "s_chain_use", "s_chain_reexp", "s_chain_with", "s_let_b", "s_let_with", "s_let_built", "s_closure", "s_closure_let",
    "s_via_local_fn",
];
const PURE: &[&str] = &["t_build_only", "t_chain_reset", "t_let_reset", "t_with_only", "t_local_chain", "t_local_let"];

fn consumer(root: &Path, name: &str, body: &str) -> PathBuf {
    let d = root.join(name);
    write(
        &d.join("Cargo.toml"),
        &format!("[package]\nname = \"{name}\"\nversion = \"0.1.0\"\nedition = \"2021\"\n[dependencies]\nbackoff = {{ path = \"../backoff\" }}\n"),
    );
    write(
        &d.join("Cargo.lock"),
        &format!("version = 4\n\n[[package]]\nname = \"{name}\"\nversion = \"0.1.0\"\n\n[[package]]\nname = \"backoff\"\nversion = \"0.4.0\"\n"),
    );
    write(&d.join("src/lib.rs"), body);
    d
}

fn scan(dir: &Path, off: bool) -> std::collections::HashMap<String, Vec<String>> {
    let mut cmd = Command::new(bin());
    cmd.arg(dir.to_string_lossy().as_ref()).arg("--json").env_remove("CANDOR_DEPS").env_remove("CANDOR_R1627_OFF")
        .env_remove("CANDOR_R1578_OFF").env_remove("CANDOR_CONFIG").env_remove("CANDOR_POLICY");
    if off {
        cmd.env("CANDOR_R1627_OFF", "1");
    }
    let out = cmd.output().expect("run candor-scan");
    assert!(out.status.code().is_some_and(|c| c == 0 || c == 1), "scan failed: {}", String::from_utf8_lossy(&out.stderr));
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).expect("report json");
    v["functions"].as_array().unwrap().iter()
        .map(|f| {
            let inf = f["inferred"].as_array().map(|a| a.iter().filter_map(|e| e.as_str().map(str::to_string)).collect()).unwrap_or_default();
            (f["fn"].as_str().unwrap().to_string(), inf)
        })
        .collect()
}

fn rand(rows: &std::collections::HashMap<String, Vec<String>>, f: &str) -> bool {
    rows.get(f).is_some_and(|r| r.iter().any(|e| e == "Rand"))
}

fn arms(fmac: bool) {
    let root = tmpdir(if fmac { "fmac" } else { "plain" });
    dep(&root, fmac);
    let app = consumer(&root, "app", MAIN_FNS);
    let on = scan(&app, false);
    for f in CHARGED {
        assert!(rand(&on, f), "`{f}` must be charged Rand (fmac={fmac}); row {:?}", on.get(*f));
    }
    for f in PURE {
        assert!(!rand(&on, f), "the pure twin `{f}` was charged Rand (fmac={fmac}): {:?}", on.get(*f));
    }
    let off = scan(&app, true);
    for f in CHARGED {
        assert!(!rand(&off, f), "`{f}` is charged with CANDOR_R1627_OFF — the fixture does not reach the new code (fmac={fmac}): {:?}", off.get(*f));
    }
}

#[test]
fn a_builder_chain_over_an_owned_receiver_is_typed_through_the_per_item_proof() {
    arms(true);
}

#[test]
fn a_builder_chain_over_an_owned_receiver_is_typed_through_judged() {
    arms(false);
}

/// Each hazard makes a trait method named `build` reachable at the consumer (or leaves that open): the inherent method may not be
/// the one that runs, so the value stays the chain's root, exactly as published.
#[test]
fn a_trait_that_could_supply_the_method_keeps_the_published_reading() {
    let root = tmpdir("hazard");
    dep(&root, true);
    let call = "pub fn h_chain() -> Option<u32> { ExponentialBackoffBuilder::new().build().next_backoff() }\n\
                pub fn h_let() -> Option<u32> { let mut e = ExponentialBackoffBuilder::new().build(); e.next_backoff() }\n";
    let hazards: &[(&str, String)] = &[
        // a local trait: a blanket by-value `build` beats the inherent `&self` one on an owned receiver
        ("h1", format!("use backoff::ExponentialBackoffBuilder;\npub trait Mine {{ fn build(self) -> u8; }}\nimpl<T> Mine for T {{ fn build(self) -> u8 {{ 0 }} }}\n{call}")),
        // an imported dependency trait that declares it
        ("h2", format!("use backoff::ExponentialBackoffBuilder;\nuse backoff::extra::Extra;\n{call}")),
        // a glob from the dependency: its trait set is not read
        ("h3", format!("use backoff::ExponentialBackoffBuilder;\nuse backoff::prelude::*;\n{call}")),
        // a statement-position foreign macro may expand to a `use`
        ("h4", format!("use backoff::ExponentialBackoffBuilder;\npub fn noise() {{ foreign::emit!(); }}\n{call}")),
        // an item-position foreign macro may write a trait
        // a foreign EXPRESSION macro whose expansion injects `use Extra` around the call it was handed (the call is walked out of
        // the invocation tokens, in a scope the AST never shows); `Extra::build(self)` wins over the inherent `build(&self)`
        ("h6", "use backoff::ExponentialBackoffBuilder;\npub fn h_chain() -> Option<u32> { backoff::wrap!(ExponentialBackoffBuilder::new().build().next_backoff()) }\n\
                pub fn h_let() -> Option<u32> { backoff::wrap!({ let mut e = ExponentialBackoffBuilder::new().build(); e.next_backoff() }) }\n".to_string()),
        // the user's own `use` inside the tokens of a std macro
        ("h7", "use backoff::ExponentialBackoffBuilder;\npub fn h_chain() -> Option<u32> { let r = { use backoff::extra::Extra; ExponentialBackoffBuilder::new().build().next_backoff() }; println!(\"{:?}\", { use backoff::extra::Extra as _; r }); r }\n\
                pub fn h_let() -> Option<u32> { let mut e = ExponentialBackoffBuilder::new().build(); e.next_backoff() }\n".to_string()),
        ("h5", format!("use backoff::ExponentialBackoffBuilder;\nforeign::emit_items! {{ }}\n{call}")),
    ];
    for (name, body) in hazards {
        let app = consumer(&root, name, body);
        let on = scan(&app, false);
        for f in ["h_chain", "h_let"] {
            assert!(!rand(&on, f), "{name}: `{f}` was typed although a trait could supply `build`: {:?}", on.get(f));
        }
    }
    // the same two functions with nothing in scope ARE typed — the hazards, not the shape, are what refuse
    let ok = consumer(&root, "h0", &format!("use backoff::ExponentialBackoffBuilder;\n{call}"));
    let on = scan(&ok, false);
    for f in ["h_chain", "h_let"] {
        assert!(rand(&on, f), "control: `{f}` must be charged Rand with no trait in scope: {:?}", on.get(f));
    }
}

/// A chain over a root that is NO dependency (`<M as Trait>::make(k).map_err(..)?`, a generic) bound to an UNTYPED name must keep
/// R1095's untyped-receiver hedge: the first corpus A/B showed a chain provenance kept on such a binding filed the `<untyped>`
/// marker, whose presence suppressed the hedge while the marker, its root being no dependency, was inert — `Unknown` -> `[]`
/// over 29 units (mongodb `client::auth::mac`'s exact shape). The local `Cache::update` is the candidate the hedge names.
#[test]
fn an_untyped_binding_over_a_chain_keeps_the_untyped_receiver_hedge() {
    let root = tmpdir("hedge");
    dep(&root, true);
    let app = consumer(
        &root,
        "hg",
        "pub struct Cache;\nimpl Cache { pub fn update(&mut self, k: &[u8]) { std::fs::write(\"/tmp/r1627-x\", k).unwrap(); } }\n\
         pub fn u_mac<M: backoff::backoff::Backoff>(key: &[u8]) -> Result<(), ()> {\n \
         let mut mac = <M as backoff::backoff::Backoff>::new_from_slice(key).map_err(|_| ())?;\n mac.update(key);\n Ok(())\n}\n",
    );
    let on = scan(&app, false);
    let off = scan(&app, true);
    assert!(off.get("u_mac").is_some_and(|r| r.iter().any(|e| e == "Unknown")), "control: the published reading hedges: {:?}", off.get("u_mac"));
    assert!(on.get("u_mac").is_some_and(|r| r.iter().any(|e| e == "Unknown")), "the hedge was lost with nothing replacing it: {:?}", on.get("u_mac"));
}
