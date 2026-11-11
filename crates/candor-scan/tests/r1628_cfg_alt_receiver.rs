//! SOUNDNESS R1628 — a dependency receiver whose ROOT binding has several conditional arms is typed per arm, and the arms'
//! answers are UNIONED (SPEC ⟨0.38⟩: a conditional binding resolves to the union of its arms) when they agree, never picked.
//! `reqwest::Client` is bound twice at the root, `if_hyper!` over `async_impl` and `if_wasm!` over `wasm`, so `Client::get` could
//! not be placed and `let rb = c.get(u); rb.send().await` read without `Log` over an EXECUTED 2 `log` records (field fixture
//! `~/candor-handoff/rust-r1619/` and `rustagent-r1628/fx`). The fixture is a hermetic `path` crate named `reqwest` carrying the
//! shape: two local `macro_rules!` wrappers over `#[cfg]`, one foreign item macro beside them (so the crate-wide reading
//! refuses), and the two arms' modules. `CANDOR_R1628_OFF=1` restores the published answer, and every charged case must then
//! read uncharged — the proof that the fixture reaches the new code.
//!
//! THE DANGEROUS DIRECTION, mutation-tested in the lane: the arms may DISAGREE, and a pick (the first arm, or the arm a host
//! cfg evaluation names) would then be the order-resolution ⟨0.21⟩ forbids. `Pool::take` returns `RequestBuilder` in one arm and
//! an inert type in the other; `Ctx::go` returns `RequestBuilder` in one arm and a type no public path names in the other. Neither may be typed as a `RequestBuilder`.

use std::path::{Path, PathBuf};
use std::process::Command;

fn bin() -> &'static str {
    env!("CARGO_BIN_EXE_candor-scan")
}

fn tmpdir(tag: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("candor-r1628-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn write(p: &Path, s: &str) {
    std::fs::create_dir_all(p.parent().unwrap()).unwrap();
    std::fs::write(p, s).unwrap();
}

fn deps(root: &Path) {
    let d = root.join("reqwest");
    write(&d.join("Cargo.toml"), "[package]\nname = \"reqwest\"\nversion = \"0.13.5\"\nedition = \"2021\"\n[dependencies]\npin-project-lite = \"0.2\"\n");
    write(
        &d.join("src/lib.rs"),
        "macro_rules! if_wasm {\n ($($item:item)*) => {$(\n #[cfg(all(target_arch = \"wasm32\", any(target_os = \"unknown\", target_os = \"none\")))]\n $item\n )*}\n}\n\
         macro_rules! if_hyper {\n ($($item:item)*) => {$(\n #[cfg(not(all(target_arch = \"wasm32\", any(target_os = \"unknown\", target_os = \"none\"))))]\n $item\n )*}\n}\n\
         mod util;\n\
         if_hyper! {\n mod async_impl;\n pub use self::async_impl::{Client, RequestBuilder, Pool, Ctx};\n}\n\
         if_wasm! {\n mod wasm;\n pub use self::wasm::{Client, RequestBuilder, Pool, Ctx};\n}\n",
    );
    // the foreign item macro that makes the crate-wide reading refuse
    write(&d.join("src/util.rs"), "pin_project_lite::pin_project! { pub struct Fut<F> { #[pin] f: F } }\n");
    write(
        &d.join("src/async_impl/mod.rs"),
        "mod client;\nmod request;\npub use self::client::{Client, Pool, Ctx};\npub use self::request::RequestBuilder;\n",
    );
    write(
        &d.join("src/async_impl/client.rs"),
        "use super::request::RequestBuilder;\npub struct Client;\npub struct Pool;\npub struct Ctx;\n\
         impl Client {\n pub fn get(&self, _u: &str) -> RequestBuilder { RequestBuilder }\n}\n\
         impl Pool {\n pub fn take(&self) -> RequestBuilder { RequestBuilder }\n}\n\
         impl Ctx {\n pub fn go(&self) -> RequestBuilder { RequestBuilder }\n}\n",
    );
    write(
        &d.join("src/async_impl/request.rs"),
        "pub struct RequestBuilder;\nimpl RequestBuilder {\n pub async fn send(self) -> u8 { 0 }\n pub fn header(self, _k: &str) -> Self { self }\n}\n",
    );
    write(
        &d.join("src/wasm/mod.rs"),
        "mod client;\nmod request;\npub use self::client::{Client, Pool, Ctx};\npub use self::request::RequestBuilder;\n",
    );
    write(
        &d.join("src/wasm/client.rs"),
        "use super::request::RequestBuilder;\npub struct Client;\npub struct Pool;\npub struct Inert;\npub struct Ctx;\npub struct Inner;\n\
         impl Client {\n pub fn get(&self, _u: &str) -> RequestBuilder { RequestBuilder }\n}\n\
         impl Pool {\n pub fn take(&self) -> Inert { Inert }\n}\nimpl Inert {\n pub fn send(self) {}\n}\nimpl Ctx {\n pub fn go(&self) -> Inner { Inner }\n}\nimpl Inner {\n pub async fn send(self) {}\n}\n",
    );
    write(&d.join("src/wasm/request.rs"), "pub struct RequestBuilder;\nimpl RequestBuilder {\n pub async fn send(self) -> u8 { 0 }\n pub fn header(self, _k: &str) -> Self { self }\n}\n");
}

fn consumer(root: &Path) -> PathBuf {
    let d = root.join("app");
    write(&d.join("Cargo.toml"), "[package]\nname = \"app\"\nversion = \"0.1.0\"\nedition = \"2021\"\n[dependencies]\nreqwest = { path = \"../reqwest\" }\n");
    write(
        &d.join("Cargo.lock"),
        "version = 4\n\n[[package]]\nname = \"app\"\nversion = \"0.1.0\"\n\n[[package]]\nname = \"reqwest\"\nversion = \"0.13.5\"\n",
    );
    write(
        &d.join("src/lib.rs"),
        "pub async fn s_rq_let(c: &reqwest::Client, u: &str) { let rb = c.get(u); let _ = rb.send().await; }\n\
         pub async fn s_rq_step(c: &reqwest::Client, u: &str) { let rb = c.get(u).header(\"k\"); let _ = rb.send().await; }\n\
         pub fn t_rq_build(c: &reqwest::Client, u: &str) { let rb = c.get(u); let _rb = rb.header(\"k\"); }\n\
         pub async fn h_split(p: &reqwest::Pool) { let h = p.take(); let _ = h.send(); }\n\
         pub async fn h_unnamed(x: &reqwest::Ctx) { let h = x.go(); let _ = h.send().await; }\n",
    );
    d
}

fn scan(dir: &Path, off: bool) -> std::collections::HashMap<String, Vec<String>> {
    let mut cmd = Command::new(bin());
    cmd.arg(dir.to_string_lossy().as_ref()).arg("--json").env_remove("CANDOR_DEPS").env_remove("CANDOR_R1628_OFF")
        .env_remove("CANDOR_R1619_OFF").env_remove("CANDOR_R1578_OFF").env_remove("CANDOR_CONFIG").env_remove("CANDOR_POLICY");
    if off {
        cmd.env("CANDOR_R1628_OFF", "1");
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

fn log(rows: &std::collections::HashMap<String, Vec<String>>, f: &str) -> bool {
    rows.get(f).is_some_and(|r| r.iter().any(|e| e == "Log"))
}

#[test]
fn a_receiver_with_conditional_root_arms_is_typed_when_the_arms_agree_and_never_picked() {
    let root = tmpdir("arms");
    deps(&root);
    let app = consumer(&root);
    let on = scan(&app, false);
    for f in ["s_rq_let", "s_rq_step"] {
        assert!(log(&on, f), "`{f}` must carry Log through the unioned arms; row {:?}", on.get(f));
    }
    // the pure twin binds the same value and never sends
    assert!(!log(&on, "t_rq_build"), "the pure twin was charged: {:?}", on.get("t_rq_build"));
    // the arms disagree: neither may be typed as the arm that classifies
    for f in ["h_split", "h_unnamed"] {
        assert!(!log(&on, f), "`{f}` was typed by picking one arm: {:?}", on.get(f));
    }
    // falsification: with the arm union off, the charged cases read as published (uncharged)
    let off = scan(&app, true);
    for f in ["s_rq_let", "s_rq_step"] {
        assert!(!log(&off, f), "`{f}` is charged with CANDOR_R1628_OFF — the fixture does not reach the new code: {:?}", off.get(f));
    }
}
