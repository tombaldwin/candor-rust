//! SOUNDNESS R1578 — a receiver whose value an UNCHAINED dependency returned is typed from that dependency's own source
//! signature, so its method reaches the classifier like a parameter-typed receiver does. `let js =
//! async_nats::jetstream::new(c); js.query_account()` sends a request (EXECUTED in the lane against a real nats-server,
//! `~/candor-handoff/rust-vein/`), and the caller read ABSENT without a lockfile and `[]` + `invisible` with one; the
//! `jetstream::new` / `with_domain` / builder-chain spellings likewise. The fixtures here are hermetic: the dependency is a
//! `path` dependency carrying the classified crate's NAME and a minimal copy of the signatures that matter, so the
//! classifier's own `async_nats` rules are what charges. `CANDOR_R1578_OFF=1` restores the published behaviour, and every
//! charged case must then read uncharged again — the proof that the fixtures reach the new code.

use std::path::{Path, PathBuf};
use std::process::Command;

fn bin() -> &'static str {
    env!("CARGO_BIN_EXE_candor-scan")
}

fn tmpdir(tag: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("candor-r1578-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn write(p: &Path, s: &str) {
    std::fs::create_dir_all(p.parent().unwrap()).unwrap();
    std::fs::write(p, s).unwrap();
}

/// The dependency: `async-nats`'s shape where it matters — `jetstream::new` re-exporting `context::Context`, a builder
/// whose step returns `Self`, a same-named free fn in ANOTHER module returning a different type, a `Result` return, a
/// generic return, and a type leaf declared twice.
fn dep(root: &Path) {
    let d = root.join("async-nats");
    write(&d.join("Cargo.toml"), "[package]\nname = \"async-nats\"\nversion = \"0.50.0\"\nedition = \"2021\"\n");
    write(&d.join("src/lib.rs"), "pub mod jetstream;\npub mod other;\n");
    write(
        &d.join("src/jetstream/mod.rs"),
        "pub mod context;\npub mod kv;\npub mod stream;\npub use context::Context;\n\
         pub fn new(c: u8) -> Context { Context::make(c) }\n\
         pub fn with_domain(c: u8, _d: &str) -> Context { Context::make(c) }\n\
         pub fn fallible(c: u8) -> Result<Context, ()> { Ok(Context::make(c)) }\n\
         pub fn generic<T: Default>() -> T { T::default() }\n\
         pub fn inner_store() -> kv::inner::Store { kv::inner::Store }\n",
    );
    write(
        &d.join("src/jetstream/context.rs"),
        "pub struct Context(u8);\nimpl Context {\n pub(crate) fn make(c: u8) -> Self { Context(c) }\n \
         pub async fn query_account(&self) -> u8 { self.0 }\n pub fn client(&self) -> u8 { self.0 }\n}\n",
    );
    write(
        &d.join("src/jetstream/stream.rs"),
        "pub struct Stream;\npub struct DirectGetBuilder(u64);\n\
         impl Stream { pub fn direct_get_builder(&self) -> DirectGetBuilder { DirectGetBuilder(0) } }\n\
         impl DirectGetBuilder {\n pub fn sequence(self, s: u64) -> Self { DirectGetBuilder(s) }\n \
         pub async fn send(self) -> u64 { self.0 }\n}\n",
    );
    // A free fn of the SAME name in another module, returning a different type: never the one `jetstream::new` names.
    write(&d.join("src/other.rs"), "pub struct Other;\nimpl Other { pub async fn query_account(&self) {} }\npub fn new() -> Other { Other }\n");
    // `Store` is declared TWICE in `kv` — the classified `kv::Store` and a pure `kv::inner::Store` — so a return naming the
    // leaf cannot be placed in one module, and `inner_store()` must not be read as the classified one.
    write(&d.join("src/jetstream/kv.rs"), "pub struct Store;\nimpl Store { pub async fn put(&self) {} }\n\
         pub mod inner { pub struct Store; impl Store { pub async fn put(&self) {} } }\n");
}

fn consumer(root: &Path, name: &str, lock: bool) -> PathBuf {
    let d = root.join(name);
    write(
        &d.join("Cargo.toml"),
        &format!("[package]\nname = \"{name}\"\nversion = \"0.1.0\"\nedition = \"2021\"\n[dependencies]\nasync-nats = {{ path = \"../async-nats\" }}\n"),
    );
    if lock {
        write(
            &d.join("Cargo.lock"),
            &format!("version = 4\n\n[[package]]\nname = \"async-nats\"\nversion = \"0.50.0\"\n\n[[package]]\nname = \"{name}\"\nversion = \"0.1.0\"\n"),
        );
    }
    write(
        &d.join("src/lib.rs"),
        "use async_nats::jetstream;\n\
         pub async fn e_new_let(c: u8) -> u8 { let js = jetstream::new(c); js.query_account().await }\n\
         pub async fn e_full_path(c: u8) -> u8 { let js = async_nats::jetstream::new(c); js.query_account().await }\n\
         pub async fn e_inline(c: u8) -> u8 { jetstream::new(c).query_account().await }\n\
         pub async fn e_with_domain(c: u8) -> u8 { let js = jetstream::with_domain(c, \"hub\"); js.query_account().await }\n\
         pub async fn e_builder_bound(s: &jetstream::stream::Stream) -> u64 { let b = s.direct_get_builder().sequence(1); b.send().await }\n\
         pub fn c_pure_method(c: u8) -> u8 { let js = jetstream::new(c); js.client() }\n\
         pub async fn n_generic() -> u8 { let js: async_nats::jetstream::Context = jetstream::generic(); js.query_account().await }\n\
         pub async fn n_twin() { let t = jetstream::inner_store(); t.put().await }\n",
    );
    d
}

/// Scan `dir` (never chained) and return `fn -> (inferred, invisible)` for every row.
fn scan(dir: &Path, off: bool) -> std::collections::HashMap<String, (Vec<String>, Vec<String>)> {
    let mut cmd = Command::new(bin());
    cmd.arg(dir.to_string_lossy().as_ref()).arg("--json").env_remove("CANDOR_DEPS").env_remove("CANDOR_R1578_OFF")
        .env_remove("CANDOR_CONFIG").env_remove("CANDOR_POLICY");
    if off {
        cmd.env("CANDOR_R1578_OFF", "1");
    }
    let out = cmd.output().expect("run candor-scan");
    assert!(out.status.code().is_some_and(|c| c == 0 || c == 1), "scan failed: {}", String::from_utf8_lossy(&out.stderr));
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).expect("report json");
    let strs = |x: &serde_json::Value| -> Vec<String> {
        x.as_array().map(|a| a.iter().filter_map(|e| e.as_str().map(str::to_string)).collect()).unwrap_or_default()
    };
    v["functions"].as_array().unwrap().iter()
        .map(|f| (f["fn"].as_str().unwrap().to_string(), (strs(&f["inferred"]), strs(&f["invisible"]))))
        .collect()
}

fn net(rows: &std::collections::HashMap<String, (Vec<String>, Vec<String>)>, f: &str) -> bool {
    rows.get(f).is_some_and(|r| r.0.iter().any(|e| e == "Net"))
}

/// `e_builder_bound` (round 3): `s: &Stream` then `.direct_get_builder()` (`&self`) and `.sequence(1)` (`self`, on the owned
/// builder it returned) — both steps are ones R1200's probe provably resolves to the inherent.
const CHARGED: &[&str] = &["e_new_let", "e_full_path", "e_inline", "e_with_domain", "e_builder_bound"];

#[test]
fn an_unchained_dependency_return_types_the_receiver_when_the_lock_pins_a_present_source() {
    let root = tmpdir("arms");
    dep(&root);
    // Round 2, the owner's ruling: WITHOUT a lockfile nothing is typed — every case reads as it did before R1578.
    let lockless = scan(&consumer(&root, "app_nolock", false), false);
    for f in CHARGED {
        assert!(!net(&lockless, f), "app_nolock: `{f}` was typed without a lock pinning the source: {:?}", lockless.get(*f));
    }
    for (name, lock) in [("app_lock", true)] {
        let app = consumer(&root, name, lock);
        let on = scan(&app, false);
        for f in CHARGED {
            assert!(net(&on, f), "{name}: `{f}` must be charged Net through the dependency's declared return; row {:?}", on.get(*f));
        }
        // The pure control binds the SAME value and calls a method no rule charges: it must not be charged.
        assert!(!net(&on, "c_pure_method"), "{name}: the pure control was charged: {:?}", on.get("c_pure_method"));
        // A return whose leaf the dependency declares twice is refused, never placed in a guessed module: `inner_store()`
        // returns the PURE `kv::inner::Store`, and reading it as the classified `kv::Store` would charge `put` (pinned by
        // a mutant that drops the uniqueness check: this assertion fails). `n_generic` is annotated, so the annotation
        // types it — the published route, checked only for not regressing.
        assert!(!net(&on, "n_twin"), "{name}: `n_twin` was typed through a refused return: {:?}", on.get("n_twin"));
        assert!(net(&on, "n_generic"), "{name}: the annotated control must keep its published charge");
        // OFF: the published behaviour — every charged case reads uncharged again.
        let off = scan(&app, true);
        for f in CHARGED {
            assert!(!net(&off, f), "{name}: with CANDOR_R1578_OFF `{f}` still charged — the fixture does not reach R1578");
        }
    }
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn a_same_named_free_fn_in_another_module_is_never_the_one_the_path_names() {
    // `jetstream::new` and `other::new` share a leaf; `other::new` returns `Other`, whose `query_account` is pure. If the
    // module were ignored the union {Context, Other} would refuse and `e_new_let` would read uncharged; if `other::new` were
    // read for `jetstream::new` the call would be keyed to `Other`. Both are pinned by `e_new_let` charging, and the
    // reverse spelling must key to `Other` (no rule) and stay uncharged.
    let root = tmpdir("module");
    dep(&root);
    let app = consumer(&root, "app_mod", true);
    std::fs::write(
        app.join("src/lib.rs"),
        "pub async fn e_new_let(c: u8) -> u8 { let js = async_nats::jetstream::new(c); js.query_account().await }\n\
         pub async fn c_other(c: u8) { let o = async_nats::other::new(); o.query_account().await }\n",
    )
    .unwrap();
    let on = scan(&app, false);
    assert!(net(&on, "e_new_let"), "row {:?}", on.get("e_new_let"));
    assert!(!net(&on, "c_other"), "`other::new` was read as `jetstream::new`: {:?}", on.get("c_other"));
    let _ = std::fs::remove_dir_all(&root);
}

/// SOUNDNESS R1580 — where the dependency's signature does NOT type the value, the untyped-receiver marker is the floor.
/// A REGISTRY dependency with no lockfile is covered by the classifier, so the κ ledger is silent and the caller must
/// disclose `Unknown` (`dispatch:untyped cross-package receiver`). The callee name exists in no real `async-nats`, so no
/// registry checkout this machine holds can type it: the floor is reached whatever `~/.cargo/registry` contains. A `path`
/// dependency wearing the classified name is NOT covered (an impostor), so there the ledger speaks (`invisible`) and no
/// `Unknown` is added beside it. The floor is OPT-IN (`CANDOR_R1580=1`, round 6); the default is the published silence (ABSENT), pinned below.
/// `fn -> (inferred, invisible, unknownWhy)`.
type Row3 = (Vec<String>, Vec<String>, Vec<String>);

#[test]
fn an_untypable_dependency_value_discloses_in_the_unchained_arm() {
    let root = tmpdir("floor");
    dep(&root);
    let src = "use async_nats::jetstream;\n\
               pub async fn f_floor(c: u8) -> u8 { let js = jetstream::r1580_no_such_factory(c); js.query_account().await }\n";
    let rows = |dir: &Path, off: bool| -> std::collections::HashMap<String, Row3> {
        let mut cmd = Command::new(bin());
        cmd.arg(dir.to_string_lossy().as_ref()).arg("--json").env_remove("CANDOR_DEPS").env_remove("CANDOR_R1580")
            .env_remove("CANDOR_R1578_OFF").env_remove("CANDOR_CONFIG").env_remove("CANDOR_POLICY");
        if !off {
            cmd.env("CANDOR_R1580", "1");
        }
        let out = cmd.output().expect("run candor-scan");
        let v: serde_json::Value = serde_json::from_slice(&out.stdout).expect("report json");
        let strs = |x: &serde_json::Value| -> Vec<String> {
            x.as_array().map(|a| a.iter().filter_map(|e| e.as_str().map(str::to_string)).collect()).unwrap_or_default()
        };
        v["functions"].as_array().unwrap().iter()
            .map(|f| (f["fn"].as_str().unwrap().to_string(), (strs(&f["inferred"]), strs(&f["invisible"]), strs(&f["unknownWhy"]))))
            .collect()
    };
    // A registry dependency, no lockfile: covered, so `Unknown` with the reason.
    let reg = root.join("floor_registry");
    write(&reg.join("Cargo.toml"), "[package]\nname = \"floor_registry\"\nversion = \"0.1.0\"\nedition = \"2021\"\n[dependencies]\nasync-nats = \"0.35.1\"\n");
    write(&reg.join("src/lib.rs"), src);
    let on = rows(&reg, false);
    let r = on.get("f_floor").unwrap_or_else(|| panic!("f_floor ABSENT — the floor did not fire: {on:?}"));
    assert!(r.0.iter().any(|e| e == "Unknown") && r.2.iter().any(|w| w == "dispatch:untyped cross-package receiver"), "{r:?}");
    let off = rows(&reg, true);
    assert!(!off.contains_key("f_floor"), "the DEFAULT (no opt-in) must be the published silence: {:?}", off.get("f_floor"));
    // A `path` dependency wearing the name: not covered, so the ledger discloses and no `Unknown` is added beside it.
    let app = consumer(&root, "floor_path", false);
    std::fs::write(app.join("src/lib.rs"), src).unwrap();
    let on = rows(&app, false);
    let r = on.get("f_floor").unwrap_or_else(|| panic!("f_floor ABSENT under a path dependency: {on:?}"));
    assert!(r.1.iter().any(|c| c == "async_nats") && !r.0.iter().any(|e| e == "Unknown"), "{r:?}");
    let _ = std::fs::remove_dir_all(&root);
}
