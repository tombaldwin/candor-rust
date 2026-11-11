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
/// Round 3: the provenance carries the receiver's form, and a method step is typed only where the probe provably reaches
/// the inherent (owned + `self`, `&` + `&self`, `&mut` + `&mut self`). Here the receiver is OWNED and the inherent takes
/// `&self`, so the step must still refuse; the `&Stream` twin (`f_ref`) is decidable and types.
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
         pub async fn f_factory(c: u8) -> u8 { let js = async_nats::jetstream::new(c); js.query_account().await }\n\
         pub async fn f_ref(s: &Stream) -> u64 { let b = s.direct_get_builder().sequence(1); b.send().await }\n");
    let (rows, _) = scan(&app, &root.join("cargo-home"));
    assert!(!net(&rows, "f_probe"), "a `&self` inherent was assumed to win the method probe: {:?}", rows.get("f_probe"));
    assert!(net(&rows, "f_factory"), "a factory step must still type: {:?}", rows.get("f_factory"));
    assert!(net(&rows, "f_ref"), "`&Stream` + `&self`, then owned + `self`: decidable, must type: {:?}", rows.get("f_ref"));
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
    // SOUNDNESS R1603: the lock's checksum must match the `.crate` cargo kept beside the checkout.
    write(&home.join("registry/cache/index.crates.io-0000000000000000/async-nats-0.49.0.crate"), "abc");
    let src = "pub async fn f(c: u8) -> u8 { let js = async_nats::jetstream::new(c); js.query_account().await }\n";
    let mk = |name: &str, lock_ver: Option<&str>| -> PathBuf {
        let d = root.join(name);
        write(&d.join("Cargo.toml"), &format!(
            "[package]\nname = \"{name}\"\nversion = \"0.1.0\"\nedition = \"2021\"\n[dependencies]\nasync-nats = \"0.49\"\n"));
        if let Some(v) = lock_ver {
            write(&d.join("Cargo.lock"), &format!(
                "version = 4\n\n[[package]]\nname = \"async-nats\"\nversion = \"{v}\"\n\
                 source = \"registry+https://github.com/rust-lang/crates.io-index\"\n\
                 checksum = \"{ABC}\"\n\n\
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

/// ROUND 3 — `?`, `unwrap`/`expect` and `.await` are recorded in the provenance and read against the signature: a `Result`
/// / `Option` the dependency DECLARES is peeled by exactly one `?`/`unwrap`, an `async fn`'s value by `.await`. A peel the
/// signature does not license (a method called on the unpeeled `Result`, a future never awaited) types nothing.
/// Round 4: a factory followed DIRECTLY by `.unwrap()` / `.expect()` (`let js = dep::maybe(c).unwrap()`) now records
/// provenance (SOUNDNESS R1587), and an ASSOCIATED fn factory (`Context::open(c)`) is read like a free fn.
#[test]
fn round3_peels_are_read_against_the_signature() {
    let root = tmpdir("peel");
    nats_src(&root.join("async-nats"), "0.50.0",
        "pub fn fallible(c: u8) -> Result<Context, ()> { Ok(Context::make(c)) }\n\
         impl context::Context { pub fn open(c: u8) -> Self { Context::make(c) } }\n\
         pub fn maybe(c: u8) -> Option<Context> { Some(Context::make(c)) }\n\
         pub async fn open_ctx(c: u8) -> Result<Context, ()> { Ok(Context::make(c)) }\n");
    let app = consumer(&root, "peelapp", true, false,
        "use async_nats::jetstream;\n\
         pub async fn p_try(c: u8) -> Result<u8, ()> { let js = jetstream::fallible(c)?; Ok(js.query_account().await) }\n\
         pub async fn p_unwrap_chain(s: &jetstream::stream::Stream) -> u64 { let b = s.direct_get_builder().sequence(1); b.send().await }\n\
         pub async fn p_unwrap(c: u8) -> u8 { let js = jetstream::maybe(c).unwrap(); js.query_account().await }\n\
         pub async fn p_expect_inline(c: u8) -> u8 { jetstream::fallible(c).expect(\"x\").query_account().await }\n\
         pub async fn p_assoc(c: u8) -> u8 { let js = jetstream::context::Context::open(c); js.query_account().await }\n\
         pub async fn p_await_try(c: u8) -> Result<u8, ()> { let js = jetstream::open_ctx(c).await?; Ok(js.query_account().await) }\n\
         pub fn n_unpeeled(c: u8) -> bool { let r = jetstream::fallible(c); r.is_ok() }\n\
         pub async fn n_no_await(c: u8) -> bool { let f = jetstream::open_ctx(c); drop(f); true }\n");
    let (rows, _) = scan(&app, &root.join("cargo-home"));
    for f in ["p_try", "p_await_try", "p_unwrap_chain", "p_unwrap", "p_expect_inline", "p_assoc"] {
        assert!(net(&rows, f), "`{f}`: the peeled value must type to the dependency's Context: {:?}", rows.get(f));
    }
    for f in ["n_unpeeled", "n_no_await"] {
        assert!(!net(&rows, f), "`{f}` was charged: {:?}", rows.get(f));
    }
    let _ = std::fs::remove_dir_all(&root);
}

/// SOUNDNESS R1587 — the CHAINED arm is unchanged. The `.unwrap()` on the factory's value is itself an `<untyped>` marker
/// (`maybe::unwrap`), whose chained `returns` lookup misses, so the chained caller already DISCLOSES; the new peeled
/// provenance adds a second marker that discloses the same reason. Pinned: the chained report is byte-identical with and
/// without R1587, while the unchained lock-pinned arm now types the call (`round3_peels_are_read_against_the_signature`).
#[test]
fn r1587_the_chained_arm_already_discloses_and_is_unchanged() {
    let root = tmpdir("r1587ch");
    nats_src(&root.join("async-nats"), "0.50.0", "pub fn maybe(c: u8) -> Option<Context> { Some(Context::make(c)) }\n");
    let dep_out = root.join("depreports");
    let st = Command::new(bin()).arg(root.join("async-nats").to_string_lossy().as_ref()).arg("--out")
        .arg(dep_out.join("r").to_string_lossy().as_ref()).env_remove("CANDOR_DEPS").env_remove("CANDOR_CONFIG")
        .env_remove("CANDOR_POLICY").output().expect("scan dep");
    assert!(st.status.code().is_some_and(|c| c == 0 || c == 1), "{}", String::from_utf8_lossy(&st.stderr));
    let app = consumer(&root, "chapp", true, false,
        "pub async fn f_peel(c: u8) -> u8 { let js = async_nats::jetstream::maybe(c).unwrap(); js.query_account().await }\n\
         pub async fn f_inline(c: u8) -> u8 { async_nats::jetstream::maybe(c).expect(\"x\").query_account().await }\n");
    let run = |off: bool| -> Vec<u8> {
        let mut cmd = Command::new(bin());
        cmd.arg(app.to_string_lossy().as_ref()).arg("--json").env("CANDOR_DEPS", &dep_out)
            .env("CARGO_HOME", root.join("cargo-home")).env_remove("CANDOR_CONFIG").env_remove("CANDOR_POLICY")
            .env_remove("CANDOR_R1587_OFF");
        if off {
            cmd.env("CANDOR_R1587_OFF", "1");
        }
        cmd.output().unwrap().stdout
    };
    let on = run(false);
    let v: serde_json::Value = serde_json::from_slice(&on).unwrap();
    for f in ["f_peel", "f_inline"] {
        let r = v["functions"].as_array().unwrap().iter().find(|x| x["fn"] == f).unwrap_or_else(|| panic!("{f} ABSENT chained: {v:#}"));
        assert!(r["unknownWhy"].as_array().is_some_and(|w| w.iter().any(|x| x == "dispatch:untyped cross-package receiver")), "{r:#}");
    }
    // SPEC ⟨0.42⟩: the switched report NAMES the switch (`switches`), and that is the ONLY difference — the claim under test is that
    // R1587 moves no row in the chained arm, so compare everything else.
    let mut off: serde_json::Value = serde_json::from_slice(&run(true)).unwrap();
    assert_eq!(off["switches"], serde_json::json!(["CANDOR_R1587_OFF"]), "the switched report names the switch it consulted");
    off.as_object_mut().unwrap().remove("switches");
    assert_eq!(v, off, "the chained report must be identical with and without R1587, but for the `switches` key");
    let _ = std::fs::remove_dir_all(&root);
}

/// A one-crate app with `deps` as its `[dependencies]` body and `src` as its lib, scanned unchained with an empty
/// `CARGO_HOME`; `optin` sets `CANDOR_R1580=1`.
fn app_scan(root: &Path, name: &str, deps: &str, src: &str, optin: bool) -> serde_json::Value {
    let d = root.join(name);
    write(&d.join("Cargo.toml"), &format!("[package]\nname = \"{name}\"\nversion = \"0.1.0\"\nedition = \"2021\"\n[dependencies]\n{deps}\n"));
    write(&d.join("src/lib.rs"), src);
    let mut cmd = Command::new(bin());
    cmd.arg(d.to_string_lossy().as_ref()).arg("--json").env("CARGO_HOME", root.join("cargo-home"))
        .env_remove("CANDOR_DEPS").env_remove("CANDOR_CONFIG").env_remove("CANDOR_POLICY").env_remove("CANDOR_R1580");
    if optin {
        cmd.env("CANDOR_R1580", "1");
    }
    serde_json::from_slice(&cmd.output().expect("scan").stdout).expect("report json")
}

fn row<'a>(v: &'a serde_json::Value, f: &str) -> Option<&'a serde_json::Value> {
    v["functions"].as_array().unwrap().iter().find(|x| x["fn"] == f)
}

/// SOUNDNESS R1591 — the κ ledger's coverage exemption is asked of the REAL package, never only of the alias. A registry
/// dependency renamed TO a calibrated crate's name (`reqwest = { package = "zz-uncalibrated-http" }`) was exempted as if
/// reviewed and its unruled call read ABSENT, while the real-name spelling disclosed `invisible`. The reverse rename
/// (`htclient = { package = "reqwest" }`) now reads covered exactly as `reqwest` itself does — the classifier's own claim.
#[test]
fn r1591_coverage_is_keyed_on_the_real_package_not_the_alias() {
    let root = tmpdir("r1591");
    let alias = app_scan(&root, "alias", "reqwest = { version = \"1\", package = \"zz-uncalibrated-http\" }",
        "pub fn f() -> u8 { reqwest::fetch_thing() }\n", false);
    let real = app_scan(&root, "real", "zz-uncalibrated-http = \"1\"", "pub fn f() -> u8 { zz_uncalibrated_http::fetch_thing() }\n", false);
    let revalias = app_scan(&root, "revalias", "htclient = { version = \"0.11\", package = \"reqwest\" }",
        "pub fn f() -> u8 { htclient::some_unruled_fn() }\n", false);
    let realrq = app_scan(&root, "realrq", "reqwest = \"0.11\"", "pub fn f() -> u8 { reqwest::some_unruled_fn() }\n", false);
    let inv = |v: &serde_json::Value| row(v, "f").and_then(|r| r["invisible"].as_array().map(|a| a.len())).unwrap_or(0);
    assert_eq!(inv(&alias), 1, "the alias of an UNcalibrated package must disclose like its real name: {alias:#}");
    assert_eq!(inv(&real), 1, "{real:#}");
    assert!(row(&realrq, "f").is_none(), "a calibrated crate's unruled call stays covered: {realrq:#}");
    assert!(row(&revalias, "f").is_none(), "a renamed calibrated crate reads covered like its real name: {revalias:#}");
    let _ = std::fs::remove_dir_all(&root);
}

/// SOUNDNESS R1580 (opt-in) — the floor's half of `renamed_dependency_calls_classify_under_the_real_package_name`: with
/// `CANDOR_R1580=1` the unkeyed `.text()` on the untyped `get(u).unwrap()` IS disclosed, identically in both spellings
/// (R1591 makes the alias read covered, so both get the covered crate's `Unknown`); by default it is not.
#[test]
fn r1580_optin_discloses_the_unkeyed_call_in_both_spellings() {
    let root = tmpdir("r1580optin");
    let src = |c: &str| format!("pub fn fetch(u: &str) -> String {{ {c}::blocking::get(u).unwrap().text().unwrap() }}\n");
    for optin in [true, false] {
        let a = app_scan(&root, "al", "htclient = { version = \"0.11\", package = \"reqwest\" }", &src("htclient"), optin);
        let u = app_scan(&root, "un", "reqwest = { version = \"0.11\" }", &src("reqwest"), optin);
        for (arm, v) in [("aliased", &a), ("unaliased", &u)] {
            let r = row(v, "fetch").unwrap_or_else(|| panic!("{arm}: fetch ABSENT: {v:#}"));
            let unk = r["inferred"].as_array().unwrap().iter().any(|e| e == "Unknown");
            assert_eq!(unk, optin, "{arm} (opt-in {optin}): {r:#}");
        }
        let key = |v: &serde_json::Value| row(v, "fetch").map(|r| (r["inferred"].clone(), r["invisible"].clone(), r["unknownWhy"].clone()));
        assert_eq!(key(&a), key(&u), "the two spellings must agree (opt-in {optin})");
    }
    let _ = std::fs::remove_dir_all(&root);
}

/// SHA-256("abc"): the bytes a fixture `.crate` holds, so the lock's `checksum` is a known constant.
const ABC: &str = "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad";
const NOT_ABC: &str = "0000000000000000000000000000000000000000000000000000000000000001";

/// SOUNDNESS R1603 — the lock-pinned registry source is read only from the checkout the LOCK names: the `registry/src/<index>/`
/// whose `registry/cache/<index>/<name>-<version>.crate` hashes to the lock's `checksum`. Cargo keeps no checksum file in the
/// checkout itself. (a) match -> typed and recorded; (b) the same name and version checked out only under another index (no
/// matching `.crate`) -> not typed, no field; (c) checksum differs -> not typed; (d) a lock entry with no checksum, and
/// (e) a checkout whose `.crate` is gone -> not typed (cannot be checked, so as if absent).
#[test]
fn r1603_registry_source_must_match_the_locks_checksum() {
    let mut fails: Vec<String> = Vec::new();
    macro_rules! chk {
        ($c:expr, $($m:tt)+) => { if !$c { fails.push(format!($($m)+)); } };
    }
    let src = "pub async fn f(c: u8) -> u8 { let js = async_nats::jetstream::new(c); js.query_account().await }\n";
    let lock_for = |name: &str, checksum: Option<&str>| {
        let ck = checksum.map(|c| format!("checksum = \"{c}\"\n")).unwrap_or_default();
        format!("version = 4\n\n[[package]]\nname = \"async-nats\"\nversion = \"0.49.0\"\n\
                 source = \"registry+https://github.com/rust-lang/crates.io-index\"\n{ck}\n\
                 [[package]]\nname = \"{name}\"\nversion = \"0.1.0\"\n")
    };
    let run = |tag: &str, setup: &dyn Fn(&Path), checksum: Option<&str>| {
        let root = tmpdir(tag);
        let home = root.join("cargo-home");
        setup(&home);
        let d = root.join("app");
        write(&d.join("Cargo.toml"), "[package]\nname = \"app\"\nversion = \"0.1.0\"\nedition = \"2021\"\n[dependencies]\nasync-nats = \"0.49\"\n");
        write(&d.join("Cargo.lock"), &lock_for("app", checksum));
        write(&d.join("src/lib.rs"), src);
        let r = scan(&d, &home);
        let _ = std::fs::remove_dir_all(&root);
        r
    };
    let crates_io = "index.crates.io-0000000000000000";
    let other = "my-registry-1111111111111111";
    let checkout = |home: &Path, idx: &str| nats_src(&home.join("registry/src").join(idx).join("async-nats-0.49.0"), "0.49.0", "");
    let krate = |home: &Path, idx: &str, bytes: &str| write(&home.join("registry/cache").join(idx).join("async-nats-0.49.0.crate"), bytes);

    // (a) the lock's registry checkout, its `.crate` hashing to the lock's checksum
    let (rows, v) = run("a", &|h| { checkout(h, crates_io); krate(h, crates_io, "abc"); }, Some(ABC));
    chk!(net(&rows, "f"), "(a) the matching checkout must type: {:?}", rows.get("f"));
    chk!(v["dependencySources"][0]["package"] == "async-nats", "(a) {v:#}");
    chk!(v["dependencySources"][0]["origin"] == "registry", "(a) {v:#}");

    // (a') the lock's registry is not crates.io: still typed, from whichever index holds the matching `.crate`
    let (rows, v) = run("a2", &|h| { checkout(h, other); krate(h, other, "abc"); }, Some(ABC));
    chk!(net(&rows, "f") && v.get("dependencySources").is_some(), "(a') {:?}\n{v:#}", rows.get("f"));

    // (b) same name and version, but only under a DIFFERENT index whose `.crate` is another registry's bytes
    let (rows, v) = run("b", &|h| { checkout(h, other); krate(h, other, "not the same crate"); }, Some(ABC));
    chk!(!net(&rows, "f"), "(b) read another registry's checkout: {:?}", rows.get("f"));
    chk!(v.get("dependencySources").is_none(), "(b) nothing read, nothing recorded:\n{v:#}");

    // (b') the genuine checkout AND a lookalike under another index: the genuine one is read, the lookalike never is
    let lookalike = "extern crate-less";
    let (rows, _v) = run("b2", &|h| {
        checkout(h, crates_io); krate(h, crates_io, "abc");
        // the lookalike's `query_account` is pure and its factory is a different type
        let d = h.join("registry/src").join(other).join("async-nats-0.49.0");
        nats_src(&d, "0.49.0", "");
        write(&d.join("src/jetstream/context.rs"), "pub struct Context(u8);\nimpl Context { pub(crate) fn make(c: u8) -> Self { Context(c) } }\n");
        krate(h, other, lookalike);
    }, Some(ABC));
    chk!(net(&rows, "f"), "(b') the genuine checkout must still type: {:?}", rows.get("f"));

    // (c) the lock's checksum differs from the `.crate` beside the only checkout
    let (rows, v) = run("c", &|h| { checkout(h, crates_io); krate(h, crates_io, "abc"); }, Some(NOT_ABC));
    chk!(!net(&rows, "f"), "(c) checksum mismatch typed anyway: {:?}", rows.get("f"));
    chk!(v.get("dependencySources").is_none(), "(c) {v:#}");

    // (d) no checksum in the lock, (e) no `.crate` to check against: cannot be verified, so as if absent
    let (rows, v) = run("d", &|h| { checkout(h, crates_io); krate(h, crates_io, "abc"); }, None);
    chk!(!net(&rows, "f") && v.get("dependencySources").is_none(), "(d) {:?}\n{v:#}", rows.get("f"));
    let (rows, v) = run("e", &|h| checkout(h, crates_io), Some(ABC));
    chk!(!net(&rows, "f") && v.get("dependencySources").is_none(), "(e) {:?}\n{v:#}", rows.get("f"));
    assert!(fails.is_empty(), "{} fixture(s) failed:\n{}", fails.len(), fails.join("\n"));
}
