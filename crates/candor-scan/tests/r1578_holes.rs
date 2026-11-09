//! SOUNDNESS R1578, round 2 — the four typing holes the review of 5e45a4c named, each as a hermetic fixture, plus the owner's
//! containment ruling (2026-11-09): dependency source may ADD a charge only when the project's Cargo.lock pins a version
//! whose source is present; a lockless tree or an absent source behaves as before R1578. Every fixture here FAILED on
//! 5e45a4c (the hole shown) and passes on the fix.
//!
//! The dependency carries the classified crate's NAME (`async-nats`) so the classifier's own rules decide what a typed call
//! is charged; the fixtures are about which TYPE the receiver gets, not about the rules.

use std::path::{Path, PathBuf};
use std::process::Command;

fn bin() -> &'static str {
    env!("CARGO_BIN_EXE_candor-scan")
}

fn tmpdir(tag: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("candor-r1578h-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn write(p: &Path, s: &str) {
    std::fs::create_dir_all(p.parent().unwrap()).unwrap();
    std::fs::write(p, s).unwrap();
}

type Rows = std::collections::HashMap<String, (Vec<String>, Vec<String>)>;

/// Scan `dir` unchained, with `CARGO_HOME` pointed at `home` (so the registry the engine may read is exactly the one the
/// fixture built), and return `fn -> (inferred, invisible)` plus the whole report.
fn scan(dir: &Path, home: &Path) -> (Rows, serde_json::Value) {
    let out = Command::new(bin())
        .arg(dir.to_string_lossy().as_ref())
        .arg("--json")
        .env("CARGO_HOME", home)
        .env_remove("CANDOR_DEPS")
        .env_remove("CANDOR_R1578_OFF")
        .env_remove("CANDOR_CONFIG")
        .env_remove("CANDOR_POLICY")
        .output()
        .expect("run candor-scan");
    assert!(out.status.code().is_some_and(|c| c == 0 || c == 1), "scan failed: {}", String::from_utf8_lossy(&out.stderr));
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).expect("report json");
    let strs = |x: &serde_json::Value| -> Vec<String> {
        x.as_array().map(|a| a.iter().filter_map(|e| e.as_str().map(str::to_string)).collect()).unwrap_or_default()
    };
    let rows = v["functions"].as_array().unwrap().iter()
        .map(|f| (f["fn"].as_str().unwrap().to_string(), (strs(&f["inferred"]), strs(&f["invisible"]))))
        .collect();
    (rows, v)
}

fn net(rows: &Rows, f: &str) -> bool {
    rows.get(f).is_some_and(|r| r.0.iter().any(|e| e == "Net"))
}

/// The base dependency source: `jetstream::new -> Context` (Net-ruled `query_account`), a builder.
const JS_MOD: &str = "pub mod context;\npub mod stream;\npub use context::Context;\n\
                      pub fn new(c: u8) -> Context { Context::make(c) }\n";
const JS_CONTEXT: &str = "pub struct Context(u8);\nimpl Context {\n pub(crate) fn make(c: u8) -> Self { Context(c) }\n \
                          pub async fn query_account(&self) -> u8 { self.0 }\n}\n";
const JS_STREAM: &str = "pub struct Stream;\npub struct DirectGetBuilder(u64);\n\
                         impl Stream { pub fn direct_get_builder(&self) -> DirectGetBuilder { DirectGetBuilder(0) } }\n\
                         pub fn builder() -> DirectGetBuilder { DirectGetBuilder(0) }\n\
                         impl DirectGetBuilder {\n pub fn sequence(self, s: u64) -> Self { DirectGetBuilder(s) }\n \
                         pub async fn send(self) -> u64 { self.0 }\n}\n";

fn nats_src(d: &Path, version: &str, extra_mod: &str) {
    write(&d.join("Cargo.toml"), &format!("[package]\nname = \"async-nats\"\nversion = \"{version}\"\nedition = \"2021\"\n"));
    write(&d.join("src/lib.rs"), "pub mod jetstream;\n");
    write(&d.join("src/jetstream/mod.rs"), &format!("{JS_MOD}{extra_mod}"));
    write(&d.join("src/jetstream/context.rs"), JS_CONTEXT);
    write(&d.join("src/jetstream/stream.rs"), JS_STREAM);
}

/// A consumer of a PATH dependency `../async-nats` (and optionally `../ext`), locked (the lock lists the path package with no
/// `source`, as cargo writes it) or not.
fn consumer(root: &Path, name: &str, lock: bool, ext: bool, src: &str) -> PathBuf {
    let d = root.join(name);
    let extdep = if ext { "ext = { path = \"../ext\" }\n" } else { "" };
    write(&d.join("Cargo.toml"), &format!(
        "[package]\nname = \"{name}\"\nversion = \"0.1.0\"\nedition = \"2021\"\n[dependencies]\nasync-nats = {{ path = \"../async-nats\" }}\n{extdep}"));
    if lock {
        let extlock = if ext { "\n[[package]]\nname = \"ext\"\nversion = \"0.1.0\"\n" } else { "" };
        write(&d.join("Cargo.lock"), &format!(
            "version = 4\n\n[[package]]\nname = \"async-nats\"\nversion = \"0.50.0\"\n{extlock}\n[[package]]\nname = \"{name}\"\nversion = \"0.1.0\"\n"));
    }
    write(&d.join("src/lib.rs"), src);
    d
}

/// HOLE 1 — the return's PATH PREFIX was ignored: `-> other::Context` (another crate's type) was typed as the dependency's
/// own `jetstream::context::Context` because only the last segment was read, and the call was charged that type's `Net`
/// (a fabrication). Both the qualified spelling and the `use other::Context;` spelling must refuse.
#[test]
fn hole1_a_foreign_return_is_never_typed_as_the_dependencys_same_leaf_type() {
    let root = tmpdir("h1");
    nats_src(&root.join("async-nats"), "0.50.0",
        "pub fn foreign_q(c: u8) -> other::Context { other::Context::new(c) }\n\
         pub mod imp { use other::Context; pub fn foreign_u(c: u8) -> Context { Context::new(c) } }\n");
    let app = consumer(&root, "h1app", true, false,
        "pub async fn f_q(c: u8) { let x = async_nats::jetstream::foreign_q(c); x.query_account().await; }\n\
         pub async fn f_u(c: u8) { let x = async_nats::jetstream::imp::foreign_u(c); x.query_account().await; }\n\
         pub async fn f_ok(c: u8) -> u8 { let x = async_nats::jetstream::new(c); x.query_account().await }\n");
    let (rows, _) = scan(&app, &root.join("cargo-home"));
    assert!(!net(&rows, "f_q"), "`-> other::Context` was typed as the dependency's own Context: {:?}", rows.get("f_q"));
    assert!(!net(&rows, "f_u"), "`use other::Context; -> Context` was typed as the dependency's own Context: {:?}", rows.get("f_u"));
    assert!(net(&rows, "f_ok"), "the control (a return the dependency declares) must still type: {:?}", rows.get("f_ok"));
    let _ = std::fs::remove_dir_all(&root);
}

/// HOLE 2 — R1200's method-probe refusal was bypassed for METHOD steps. A by-value method of a trait in scope at the
/// consumer (`ext::Ext`, blanket over every `T`) beats the dependency's `&self` inherent `direct_get_builder` on an OWNED
/// receiver, so `b` is `ext::Other`, not `DirectGetBuilder`, and charging `DirectGetBuilder::send`'s `Net` fabricates.
/// Which method the probe reaches depends on the receiver's reference-ness, which the provenance marker does not carry, so
/// a METHOD step is not typed at all; a FACTORY (free fn) step — no probe — still is (the control).
#[test]
fn hole2_a_method_step_is_typed_only_where_the_inherent_provably_wins_the_probe() {
    let root = tmpdir("h2");
    nats_src(&root.join("async-nats"), "0.50.0", "");
    write(&root.join("ext/Cargo.toml"), "[package]\nname = \"ext\"\nversion = \"0.1.0\"\nedition = \"2021\"\n");
    write(&root.join("ext/src/lib.rs"),
        "pub struct Other;\nimpl Other { pub async fn send(self) -> u64 { 0 } }\n\
         pub trait Ext { fn direct_get_builder(self) -> Other where Self: Sized { Other } }\nimpl<T> Ext for T {}\n");
    let app = consumer(&root, "h2app", true, true,
        "use ext::Ext;\nuse async_nats::jetstream::stream::Stream;\n\
         pub async fn f_probe(s: Stream) -> u64 { let b = s.direct_get_builder(); b.send().await }\n\
         pub async fn f_factory(c: u8) -> u8 { let js = async_nats::jetstream::new(c); js.query_account().await }\n");
    let (rows, _) = scan(&app, &root.join("cargo-home"));
    assert!(!net(&rows, "f_probe"), "a `&self` inherent was assumed to win the method probe: {:?}", rows.get("f_probe"));
    assert!(net(&rows, "f_factory"), "a factory step must still type: {:?}", rows.get("f_factory"));
    let _ = std::fs::remove_dir_all(&root);
}

/// HOLE 3 + THE OWNER'S RULING — only the version the project's Cargo.lock pins, and only when its source is present. On
/// 5e45a4c a lock naming an ABSENT version fell through to every semver-compatible checkout (here `0.49.0`), and a lockless
/// tree was typed from that union too. Now: lock pins `0.49.1` (absent) -> no typing; lockless -> no typing; lock pins
/// `0.49.0` (present) -> typed, and the report records the source it read.
#[test]
fn hole3_only_a_lock_pinned_present_source_may_type() {
    let root = tmpdir("h3");
    let home = root.join("cargo-home");
    let reg = home.join("registry/src/index.crates.io-0000000000000000");
    nats_src(&reg.join("async-nats-0.49.0"), "0.49.0", "");
    let src = "pub async fn f(c: u8) -> u8 { let js = async_nats::jetstream::new(c); js.query_account().await }\n";
    let mk = |name: &str, lock_ver: Option<&str>| -> PathBuf {
        let d = root.join(name);
        write(&d.join("Cargo.toml"), &format!(
            "[package]\nname = \"{name}\"\nversion = \"0.1.0\"\nedition = \"2021\"\n[dependencies]\nasync-nats = \"0.49\"\n"));
        if let Some(v) = lock_ver {
            write(&d.join("Cargo.lock"), &format!(
                "version = 4\n\n[[package]]\nname = \"async-nats\"\nversion = \"{v}\"\n\
                 source = \"registry+https://github.com/rust-lang/crates.io-index\"\n\n\
                 [[package]]\nname = \"{name}\"\nversion = \"0.1.0\"\n"));
        }
        write(&d.join("src/lib.rs"), src);
        d
    };
    let (absent, v_absent) = scan(&mk("h3absent", Some("0.49.1")), &home);
    assert!(!net(&absent, "f"), "typed from a version the lock does not pin: {:?}", absent.get("f"));
    assert!(v_absent.get("dependencySources").is_none(), "nothing was read, so nothing may be recorded:\n{v_absent:#}");
    let (lockless, v_lockless) = scan(&mk("h3nolock", None), &home);
    assert!(!net(&lockless, "f"), "a lockless tree was typed from a semver guess: {:?}", lockless.get("f"));
    assert!(v_lockless.get("dependencySources").is_none(), "{v_lockless:#}");
    let (pinned, v_pinned) = scan(&mk("h3pinned", Some("0.49.0")), &home);
    assert!(net(&pinned, "f"), "the lock-pinned, present source must type: {:?}", pinned.get("f"));
    let ds = &v_pinned["dependencySources"];
    assert_eq!(ds[0]["package"], "async-nats", "{v_pinned:#}");
    assert_eq!(ds[0]["version"], "0.49.0", "{v_pinned:#}");
    assert_eq!(ds[0]["origin"], "registry", "{v_pinned:#}");
    assert!(ds[0]["typed"].as_u64().is_some_and(|n| n >= 1), "{v_pinned:#}");
    let _ = std::fs::remove_dir_all(&root);
}

/// HOLE 4 — the typed path is new reach for the old LOCAL route: `tail2` keys on `Context::query_account`, so with the
/// consumer declaring its own `struct Context` the dependency call was EDGED to the consumer's twin. Measured on 5e45a4c the
/// classifier still charged the dependency's `Net` here (the review predicted a silence; this fixture shows a FABRICATED
/// edge instead — the twin's `Fs` charged to a caller that never runs it). A typed dependency call whose type leaf the
/// consumer also DECLARES must not take the local route. (A local impl ON the dependency's type — no local declaration of
/// the leaf, rav1e's `impl .. for bitstream_io::BitWriter` — still does.)
#[test]
fn hole4_a_same_leaf_local_type_never_answers_for_the_dependencys_type() {
    let root = tmpdir("h4");
    nats_src(&root.join("async-nats"), "0.50.0", "");
    let app = consumer(&root, "h4app", true, false,
        "pub struct Context;\nimpl Context { pub async fn query_account(&self) -> u8 { let _ = std::fs::write(\"/tmp/never\", \"\"); 0 } }\n\
         pub async fn f_twin(c: u8) -> u8 { let js = async_nats::jetstream::new(c); js.query_account().await }\n");
    let (rows, v) = scan(&app, &root.join("cargo-home"));
    assert!(net(&rows, "f_twin"), "the local twin answered for the dependency's Context and silenced its Net: {:?}", rows.get("f_twin"));
    let f = v["functions"].as_array().unwrap().iter().find(|f| f["fn"] == "f_twin").unwrap();
    assert!(f.get("calls").is_none_or(|c| !c.as_array().unwrap().iter().any(|x| x == "Context::query_account")),
        "the dependency's Context was edged to the consumer's own same-leaf type (fabricating its Fs): {f:#}");
    assert!(!rows.get("f_twin").unwrap().0.iter().any(|e| e == "Fs"), "the local twin's Fs was charged: {f:#}");
    let _ = std::fs::remove_dir_all(&root);
}
