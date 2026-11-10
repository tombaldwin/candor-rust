//! SOUNDNESS R1619 — a dependency value bound in a `let` is typed through the lock-pinned source even where the source holds
//! a foreign item macro or an external `pub use` glob somewhere, because the typing question is answered per ITEM
//! (`depsig_strict.rs`): Rust allows one definition of a name per module and one inherent method name per type, so a visible
//! definition present in every build is the only one there is. EXECUTED in the lane (`~/candor-handoff/rust-r1619/`): `let b =
//! c.build_transaction(); b.start()` sends `START TRANSACTION`, `let pool = cfg.create_pool(..)?; pool.get()` opens a TCP
//! connection and draws entropy, and both read ABSENT before. The dependencies here are hermetic `path` crates carrying the
//! classified crates' NAMES and a copy of the shapes that refused: `tokio-postgres`'s `pub use postgres_types::*` (an external
//! glob, `use:leaf`), a `pin_project!` (a foreign item macro, `fmac`), and `deadpool-postgres`'s `managed_reexports!`, which
//! makes `Pool` out of sight. `CANDOR_R1619_OFF=1` restores the published behaviour, and every charged case must then read
//! uncharged — the proof that the fixtures reach the new code.

use std::path::{Path, PathBuf};
use std::process::Command;

fn bin() -> &'static str {
    env!("CARGO_BIN_EXE_candor-scan")
}

fn tmpdir(tag: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("candor-r1619-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn write(p: &Path, s: &str) {
    std::fs::create_dir_all(p.parent().unwrap()).unwrap();
    std::fs::write(p, s).unwrap();
}

fn deps(root: &Path) {
    // tokio-postgres: `Client::build_transaction` returns `TransactionBuilder`, declared in a private module and re-exported at
    // the root; an external glob in `types` and a foreign item macro in `client` made `judged` refuse the whole crate.
    let d = root.join("tokio-postgres");
    write(&d.join("Cargo.toml"), "[package]\nname = \"tokio-postgres\"\nversion = \"0.7.18\"\nedition = \"2021\"\n[dependencies]\npostgres-types = \"0.2\"\npin-project-lite = \"0.2\"\nforeign = \"1\"\n");
    write(
        &d.join("src/lib.rs"),
        "mod client;\nmod transaction_builder;\nmod hazard;\npub mod types;\n\
         pub use crate::client::Client;\npub use crate::transaction_builder::TransactionBuilder;\npub use crate::hazard::Odd;\n",
    );
    write(&d.join("src/types.rs"), "pub use postgres_types::*;\n");
    write(
        &d.join("src/client.rs"),
        "use crate::transaction_builder::TransactionBuilder;\n\
         pin_project_lite::pin_project! { pub struct Fut<F> { #[pin] f: F } }\n\
         pub struct Client(u8);\n\
         impl Client {\n pub fn build_transaction(&mut self) -> TransactionBuilder<'_> { TransactionBuilder::new(self) }\n}\n",
    );
    write(
        &d.join("src/transaction_builder.rs"),
        "use crate::client::Client;\npub struct TransactionBuilder<'a> { c: &'a mut Client }\n\
         impl<'a> TransactionBuilder<'a> {\n pub(crate) fn new(c: &'a mut Client) -> TransactionBuilder<'a> { TransactionBuilder { c } }\n \
         pub fn read_only(self, _b: bool) -> Self { self }\n pub async fn start(self) -> u8 { let _ = self.c; 0 }\n}\n",
    );
    // THE DANGEROUS DIRECTION, three ways a per-crate refusal was hiding a WRONG type that the per-item proof must still refuse:
    //  * `Odd::cfgd` is `#[cfg]`'d and a foreign macro in the same crate is HANDED its name — the other build's definition may be
    //    that macro's, returning something else;
    //  * `Odd::inst` sits in an impl over ONE instantiation (`Odd<u8>`), which a hidden `impl Odd<u16>` may duplicate;
    //  * `Odd::hidden` returns `TransactionBuilder` written bare in a module that neither declares nor imports it, beside a foreign
    //    macro that may bind it: the crate's other `TransactionBuilder` is not the one it names.
    write(
        &d.join("src/hazard.rs"),
        "foreign::make! { cfgd TransactionBuilder }\n\
         pub struct Odd<T>(T);\n\
         impl<T> Odd<T> {\n #[cfg(feature = \"x\")]\n pub fn cfgd(&self) -> crate::transaction_builder::TransactionBuilder<'static> { loop {} }\n \
         pub fn hidden(&self) -> TransactionBuilder<'static> { loop {} }\n}\n\
         impl Odd<u8> {\n pub fn inst(&self) -> crate::transaction_builder::TransactionBuilder<'static> { loop {} }\n}\n",
    );
    // deadpool-postgres: `Pool` is made by a foreign macro at the root, so `create_pool`'s `Result<Pool, _>` names a type only
    // its PATH can give; `deadpool_postgres::Pool::get` is what the classifier keys.
    let d = root.join("deadpool-postgres");
    write(&d.join("Cargo.toml"), "[package]\nname = \"deadpool-postgres\"\nversion = \"0.14.2\"\nedition = \"2021\"\n[dependencies]\ndeadpool = \"0.12\"\n");
    write(
        &d.join("src/lib.rs"),
        "mod config;\npub use self::config::Config;\npub use deadpool::managed::reexports::*;\n\
         deadpool::managed_reexports!(\"tokio_postgres\", Manager, Object, Error, ConfigError);\n\
         pub struct Manager;\npub struct Object;\npub struct Error;\npub struct ConfigError;\n",
    );
    write(
        &d.join("src/config.rs"),
        "use super::Pool;\npub struct Config;\npub struct CreatePoolError;\n\
         impl Config {\n pub fn create_pool(&self, _r: Option<u8>) -> Result<Pool, CreatePoolError> { loop {} }\n}\n",
    );
}

fn consumer(root: &Path) -> PathBuf {
    let d = root.join("app");
    write(
        &d.join("Cargo.toml"),
        "[package]\nname = \"app\"\nversion = \"0.1.0\"\nedition = \"2021\"\n[dependencies]\n\
         tokio-postgres = { path = \"../tokio-postgres\" }\ndeadpool-postgres = { path = \"../deadpool-postgres\" }\n",
    );
    write(
        &d.join("Cargo.lock"),
        "version = 4\n\n[[package]]\nname = \"app\"\nversion = \"0.1.0\"\n\n[[package]]\nname = \"deadpool-postgres\"\nversion = \"0.14.2\"\n\n\
         [[package]]\nname = \"tokio-postgres\"\nversion = \"0.7.18\"\n",
    );
    write(
        &d.join("src/lib.rs"),
        "type E = Box<dyn std::error::Error>;\n\
         pub async fn s_tx_let(c: &mut tokio_postgres::Client) { let b = c.build_transaction(); let _ = b.start().await; }\n\
         pub async fn s_tx_let_step(c: &mut tokio_postgres::Client) { let b = c.build_transaction().read_only(true); let _ = b.start().await; }\n\
         pub async fn s_pool_q(cfg: &deadpool_postgres::Config) -> Result<(), E> { let pool = cfg.create_pool(None)?; let _ = pool.get().await; Ok(()) }\n\
         pub async fn s_pool_ok(cfg: &deadpool_postgres::Config) -> Option<()> { let pool = cfg.create_pool(None).ok()?; let _ = pool.get().await; Some(()) }\n\
         pub fn t_tx_ro(c: &mut tokio_postgres::Client) { let b = c.build_transaction(); let _b = b.read_only(true); }\n\
         pub fn t_pool_status(cfg: &deadpool_postgres::Config) -> Option<()> { let pool = cfg.create_pool(None).ok()?; let _ = pool.status(); Some(()) }\n\
         pub async fn h_cfgd(o: &tokio_postgres::Odd<u8>) { let b = o.cfgd(); let _ = b.start().await; }\n\
         pub async fn h_inst(o: &tokio_postgres::Odd<u8>) { let b = o.inst(); let _ = b.start().await; }\n\
         pub async fn h_hidden(o: &tokio_postgres::Odd<u8>) { let b = o.hidden(); let _ = b.start().await; }\n",
    );
    d
}

fn scan(dir: &Path, off: bool) -> std::collections::HashMap<String, Vec<String>> {
    let mut cmd = Command::new(bin());
    cmd.arg(dir.to_string_lossy().as_ref()).arg("--json").env_remove("CANDOR_DEPS").env_remove("CANDOR_R1619_OFF")
        .env_remove("CANDOR_R1578_OFF").env_remove("CANDOR_CONFIG").env_remove("CANDOR_POLICY");
    if off {
        cmd.env("CANDOR_R1619_OFF", "1");
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

fn db(rows: &std::collections::HashMap<String, Vec<String>>, f: &str) -> bool {
    rows.get(f).is_some_and(|r| r.iter().any(|e| e == "Db"))
}

const CHARGED: &[&str] = &["s_tx_let", "s_tx_let_step", "s_pool_q", "s_pool_ok"];

#[test]
fn a_let_bound_dependency_value_is_typed_per_item_through_a_crate_judged_refuses() {
    let root = tmpdir("arms");
    deps(&root);
    let app = consumer(&root);
    let on = scan(&app, false);
    for f in CHARGED {
        assert!(db(&on, f), "`{f}` must be charged Db through the per-item proof; row {:?}", on.get(*f));
    }
    // the pure twins bind the same values and call nothing a rule charges
    for f in ["t_tx_ro", "t_pool_status"] {
        assert!(!db(&on, f), "the pure twin `{f}` was charged: {:?}", on.get(f));
    }
    // the per-crate refusal stood for these: the per-item proof must refuse them too
    for f in ["h_cfgd", "h_inst", "h_hidden"] {
        assert!(!db(&on, f), "`{f}` was typed through a definition that may not be the one compiled: {:?}", on.get(f));
    }
    // falsification: with the per-item proof off, every charged case reads as published (uncharged)
    let off = scan(&app, true);
    for f in CHARGED {
        assert!(!db(&off, f), "`{f}` is charged with CANDOR_R1619_OFF — the fixture does not reach the new code: {:?}", off.get(*f));
    }
}
