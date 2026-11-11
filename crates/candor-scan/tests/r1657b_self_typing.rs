//! SOUNDNESS R1657 stage 1b / R1652 — `self` IS TYPED BY THE IMPL'S ORIGIN-QUALIFIED SELF TYPE.
//!
//! Stage 1a qualified a bare declared type where it is minted, but `self` kept the impl's unit key: the bare leaf. Every
//! value derived from `self` (`let st = *self;`, `self.as_ref().get_ref()`, `let s = self;`) was owned by a leaf that
//! same-leaf twins share, so a field read through it was refused and disclosed (`ambiguous:same-name types ... declare
//! field ... differently`) where the program had already settled it. Measured on rustls 0.23 `client::tls13::
//! ExpectFinished::handle` (its `KeyLog` chain) and tokio-sync 0.2.0-alpha.6 `oneshot::Receiver::poll`.
//!
//! Every fixture is EXECUTED ground truth (lane scratch `rustagent-stage1b/fx/<name>`, `cargo run`; the effectful body writes
//! a temp file, the twin's body is pure). Guards: a self type bound by a `#[cfg]`-duplicated `use` (`g1`, which the first
//! cut read ABSENT over a write) and a self type declared only under `#[cfg]` (`g3`) keep the bare leaf and stay disclosed.
//! Fixture fns carry unique names because `deny <E> <fn>` matches by PREFIX.

use std::path::{Path, PathBuf};
use std::process::Command;

fn bin() -> &'static str {
    env!("CARGO_BIN_EXE_candor-scan")
}

fn fixture(tag: &str, files: &[(&str, &str)]) -> PathBuf {
    let d = std::env::temp_dir().join(format!("candor-r1657b-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(d.join("src")).unwrap();
    std::fs::write(d.join("Cargo.toml"), "[package]\nname = \"fx\"\nversion = \"0.1.0\"\nedition = \"2021\"\n[dependencies]\n").unwrap();
    for (p, s) in files {
        let f = d.join("src").join(p);
        std::fs::create_dir_all(f.parent().unwrap()).unwrap();
        std::fs::write(f, s).unwrap();
    }
    d
}

/// The report (`--json`), and the exit code of `policy` when one is given. `off` sets `CANDOR_R1657B_OFF`.
fn scan(dir: &Path, policy: Option<&str>, off: bool) -> (i32, serde_json::Value) {
    let mut cmd = Command::new(bin());
    cmd.arg(dir.to_string_lossy().as_ref()).arg("--json");
    if let Some(p) = policy {
        let pf = dir.join("policy.txt");
        std::fs::write(&pf, p).unwrap();
        cmd.arg("--policy").arg(&pf);
    }
    for v in ["CANDOR_DEPS", "CANDOR_CONFIG", "CANDOR_POLICY", "CANDOR_R1657B_OFF", "CANDOR_R1657_OFF", "CANDOR_R1624_OFF"] {
        cmd.env_remove(v);
    }
    if off {
        cmd.env("CANDOR_R1657B_OFF", "1");
    }
    let out = cmd.output().expect("run candor-scan");
    let code = out.status.code().unwrap_or(-1);
    assert!(code == 0 || code == 1, "scan failed ({code}): {}", String::from_utf8_lossy(&out.stderr));
    (code, serde_json::from_slice(&out.stdout).expect("report json"))
}

fn inferred(v: &serde_json::Value, f: &str) -> Vec<String> {
    let row = v["functions"].as_array().unwrap().iter().find(|r| r["fn"].as_str() == Some(f));
    let mut out: Vec<String> = row
        .and_then(|r| r["inferred"].as_array())
        .map(|a| a.iter().filter_map(|e| e.as_str().map(str::to_string)).collect())
        .unwrap_or_default();
    out.sort();
    out
}

fn why(v: &serde_json::Value, f: &str) -> Vec<String> {
    let row = v["functions"].as_array().unwrap().iter().find(|r| r["fn"].as_str() == Some(f));
    row.and_then(|r| r["unknownWhy"].as_array())
        .map(|a| a.iter().filter_map(|e| e.as_str().map(str::to_string)).collect())
        .unwrap_or_default()
}

const W: &str = "std::fs::write(std::env::temp_dir().join(\"r1657b\"), b\"x\").unwrap();";

/// rustls's shape (fixture `b1`): `self: Box<Self>`, `let st = *self;`, then a field read; `client` writes, `server` is pure.
fn boxed(client_decl: &str) -> Vec<(&'static str, String)> {
    vec![
        ("main.rs", "mod client;\nmod server;\npub trait State { fn handle(self: Box<Self>); }\nfn main() { let c: Box<dyn State> = Box::new(client::Expect { ks: client::KsA }); c.handle(); }\n".to_string()),
        ("client.rs", format!("pub struct KsA;\nimpl KsA {{ pub fn log(&self) {{ {W} }} }}\n{client_decl}\nimpl crate::State for Expect {{ fn handle(self: Box<Self>) {{ let st = *self; st.ks.log(); }} }}\n")),
        ("server.rs", "pub struct KsB;\nimpl KsB { pub fn log(&self) { let _ = 1 + 1; } }\npub struct Expect { pub ks: KsB }\nimpl crate::State for Expect { fn handle(self: Box<Self>) { let st = *self; st.ks.log(); } }\n".to_string()),
    ]
}

fn files<'a>(v: &'a [(&'a str, String)]) -> Vec<(&'a str, &'a str)> {
    v.iter().map(|(p, s)| (*p, s.as_str())).collect()
}

#[test]
fn r1657b_a_copy_of_self_reads_its_own_field_and_the_kill_switch_restores_the_refusal() {
    let fx = boxed("pub struct Expect { pub ks: KsA }");
    let d = fixture("boxed", &files(&fx));
    let (_, v) = scan(&d, None, false);
    assert_eq!(inferred(&v, "client::Expect::handle"), vec!["Env", "Fs"], "the write is charged (executed):\n{v:#}");
    assert!(inferred(&v, "server::Expect::handle").is_empty(), "the pure twin is pure (executed):\n{v:#}");
    let (code, _) = scan(&d, Some("deny Fs client::Expect::handle"), false);
    assert_eq!(code, 1, "deny Fs over the write fires");
    let (code, _) = scan(&d, Some("deny Unknown server::Expect::handle"), false);
    assert_eq!(code, 0, "the pure twin's hedge is resolved");
    // The switch restores the pre-1b reading: both twins refused and disclosed.
    let (_, off) = scan(&d, None, true);
    for f in ["client::Expect::handle", "server::Expect::handle"] {
        assert_eq!(inferred(&off, f), vec!["Unknown"], "OFF: `{f}` is the refused field read:\n{off:#}");
        assert!(why(&off, f).iter().any(|w| w.contains("declare field `ks` differently")), "OFF: `{f}` names the twin field");
    }
    let _ = std::fs::remove_dir_all(&d);
}

#[test]
fn r1657b_a_pinned_self_projection_reads_its_own_field() {
    // tokio-sync's shape (fixture `b2`): `self: Pin<&mut Self>`, `self.as_ref().get_ref().inner.as_ref()`.
    let recv = |eff: &str| {
        format!("use std::future::Future;\nuse std::pin::Pin;\nuse std::task::{{Context, Poll}};\npub struct Inner;\nimpl Inner {{ pub fn poll_recv(&self) -> u8 {{ {eff} 1 }} }}\npub struct Receiver {{ pub inner: Option<Inner> }}\nimpl Future for Receiver {{\n    type Output = u8;\n    fn poll(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<u8> {{\n        if let Some(inner) = self.as_ref().get_ref().inner.as_ref() {{ Poll::Ready(inner.poll_recv()) }} else {{ Poll::Pending }}\n    }}\n}}\n")
    };
    let d = fixture("pin", &[("main.rs", "mod oneshot;\nmod watch;\nfn main() {}\n"), ("oneshot.rs", &recv(W)), ("watch.rs", &recv(""))]);
    let (_, v) = scan(&d, None, false);
    assert_eq!(inferred(&v, "oneshot::Receiver::poll"), vec!["Env", "Fs"], "{v:#}");
    assert!(inferred(&v, "watch::Receiver::poll").is_empty(), "{v:#}");
    let (_, off) = scan(&d, None, true);
    assert_eq!(inferred(&off, "oneshot::Receiver::poll"), vec!["Unknown"], "OFF: refused:\n{off:#}");
    let _ = std::fs::remove_dir_all(&d);
}

#[test]
fn r1657b_an_imported_self_type_reads_its_own_field() {
    // Fixture `b3`: the impl sits in a third module and names its self type through `use` (or a written path).
    let d = fixture(
        "imported",
        &[
            ("main.rs", "mod a;\nmod b;\nmod imp;\nmod imp2;\npub trait Go { fn go(&self); fn go2(&self); }\nfn main() {}\n"),
            ("a.rs", &format!("pub struct Wa;\nimpl Wa {{ pub fn run(&self) {{ {W} }} }}\npub struct H {{ pub w: Wa }}\n")),
            ("b.rs", "pub struct Wb;\nimpl Wb { pub fn run(&self) {} }\npub struct H { pub w: Wb }\n"),
            ("imp.rs", "use crate::a::H;\nimpl crate::Go for H {\n    fn go(&self) { let s = self; s.w.run(); }\n    fn go2(&self) { self.w.run(); }\n}\n"),
            ("imp2.rs", "impl crate::Go for crate::b::H {\n    fn go(&self) { let s = self; s.w.run(); }\n    fn go2(&self) { self.w.run(); }\n}\n"),
        ],
    );
    let (_, v) = scan(&d, None, false);
    for f in ["imp::H::go", "imp::H::go2"] {
        assert_eq!(inferred(&v, f), vec!["Env", "Fs"], "`{f}` writes (executed):\n{v:#}");
    }
    for f in ["imp2::H::go", "imp2::H::go2"] {
        assert!(inferred(&v, f).is_empty(), "`{f}` is pure (executed):\n{v:#}");
    }
    let _ = std::fs::remove_dir_all(&d);
}

#[test]
fn r1657b_a_self_type_bound_by_a_cfg_duplicated_use_stays_disclosed() {
    // Fixture `g1`: the first cut typed `self` by the LAST-written arm (`crate::b::H`, pure) on a unix build that compiles
    // the `crate::a::H` arm, and `imp::H::go` went ABSENT over a write. The bare leaf's refusal is the answer.
    let d = fixture(
        "cfgarm",
        &[
            ("main.rs", "mod a;\nmod b;\nmod imp;\npub trait Go { fn go(&self); }\nfn main() {}\n"),
            ("a.rs", &format!("pub struct Wa;\nimpl Wa {{ pub fn run(&self) {{ {W} }} }}\npub struct H {{ pub w: Wa }}\n")),
            ("b.rs", "pub struct Wb;\nimpl Wb { pub fn run(&self) {} }\npub struct H { pub w: Wb }\n"),
            ("imp.rs", "#[cfg(unix)]\nuse crate::a::H;\n#[cfg(not(unix))]\nuse crate::b::H;\nimpl crate::Go for H { fn go(&self) { let s = self; s.w.run(); } }\n"),
        ],
    );
    let (_, v) = scan(&d, None, false);
    assert_eq!(inferred(&v, "imp::H::go"), vec!["Unknown"], "disclosed, never pure:\n{v:#}");
    let (code, _) = scan(&d, Some("deny Unknown imp::H::go"), false);
    assert_eq!(code, 1);
    let _ = std::fs::remove_dir_all(&d);
}

#[test]
fn r1657b_a_self_type_declared_only_under_cfg_stays_disclosed() {
    // Fixture `g3`: `anchor_origin` leaves a cfg-only declaration bare (R1153 g2), so `self` keeps the bare leaf.
    let fx = boxed("#[cfg(unix)]\npub struct Expect { pub ks: KsA }\n#[cfg(not(unix))]\npub struct Expect { pub ks: KsA, pub pad: u8 }");
    let d = fixture("cfgonly", &files(&fx));
    let (_, v) = scan(&d, None, false);
    assert_eq!(inferred(&v, "client::Expect::handle"), vec!["Unknown"], "disclosed, never pure:\n{v:#}");
    let _ = std::fs::remove_dir_all(&d);
}

#[test]
fn r1657b_a_blanket_impl_over_a_parameter_named_like_a_declared_struct_keeps_its_dispatch() {
    // Fixture `g4`: `struct T` beside `impl<T: Run> Go for T`. `self` is the PARAMETER, never `crate::m::T`.
    let d = fixture(
        "generic",
        &[(
            "main.rs",
            &format!("mod m {{\n    pub struct T {{ pub n: u8 }}\n    impl T {{ pub fn run(&self) {{}} }}\n    pub trait Run {{ fn run(&self); }}\n    pub trait Go {{ fn go(&self); }}\n    impl<T: Run> Go for T {{ fn go(&self) {{ self.run() }} }}\n}}\nmod e {{\n    pub struct Eff;\n    impl crate::m::Run for Eff {{ fn run(&self) {{ {W} }} }}\n}}\nfn main() {{}}\n"),
        )],
    );
    let (_, v) = scan(&d, None, false);
    assert_eq!(inferred(&v, "m::T::go"), vec!["Env", "Fs"], "the bound dispatch reaches `Eff::run`:\n{v:#}");
    let _ = std::fs::remove_dir_all(&d);
}

#[test]
fn r1657b_a_guessed_hop_that_settles_on_the_caller_is_disclosed() {
    // Fixture `h1` (regex-automata 0.4 `hybrid::regex::Regex::reset_cache`'s shape): `self.forward().reset_cache()` with
    // `forward` DECLARED `-> &DFA`, a same-leaf `Regex` elsewhere. The chain walk typed the tail on `Regex` itself; before
    // 1b the contested `Regex::forward` hedged the site, the first 1b cut settled the tail on the CALLER and the unit went
    // ABSENT over a write (executed). It now discloses the untyped receiver, naming `DFA::reset_cache`.
    let d = fixture(
        "selfhop",
        &[(
            "main.rs",
            &format!("mod hy {{\n    pub mod dfa {{\n        pub struct DFA;\n        impl DFA {{ pub fn reset_cache(&self) {{ {W} }} }}\n    }}\n    pub mod regex {{\n        use super::dfa::DFA;\n        pub struct Regex {{ pub forward: DFA }}\n        impl Regex {{\n            pub fn forward(&self) -> &DFA {{ &self.forward }}\n            pub fn reset_cache(&self) {{ self.forward().reset_cache(); }}\n        }}\n    }}\n}}\nmod meta {{\n    pub struct Regex;\n    impl Regex {{ pub fn forward(&self) -> u8 {{ 0 }} }}\n}}\nfn main() {{}}\n"),
        )],
    );
    let (_, v) = scan(&d, None, false);
    assert_eq!(inferred(&v, "hy::regex::Regex::reset_cache"), vec!["Unknown"], "disclosed, never absent:\n{v:#}");
    assert!(why(&v, "hy::regex::Regex::reset_cache").iter().any(|w| w.contains("DFA::reset_cache")), "names the lost callee:\n{v:#}");
    let (code, _) = scan(&d, Some("deny Unknown hy::regex::Regex::reset_cache"), false);
    assert_eq!(code, 1);
    let _ = std::fs::remove_dir_all(&d);
}
