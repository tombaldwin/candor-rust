//! I1 stage 2b FLIP — the cross-file `#[cfg]` lift is ON by default. A configuration macro defined in another file stamps `#[cfg]`
//! on the impls it wraps (mio's `cfg_io_source!`), so with the lift refused the arms' methods are hidden and an untyped call over
//! them reads ABSENT over a write (`rustagent-i1s2b/fx/fxmio1`, executed; 0.42.0, main and the refused reading alike). By
//! default the arms are read and the call discloses; `CANDOR_I1S2_XCFG_OFF` restores the refusal, which gives the test teeth.
//! Beside it, Fable's `fxmio2` shape (the effectful arm writes through an UNCHAINED dependency, and with the lift refused a
//! single visible arm carries the hedge) discloses on both readings — the arm-set floor's case.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::Command;

const W: &str = "let _ = std::fs::write(\"/tmp/xcfg-never-run\", b\"x\");";

fn tmpdir(tag: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("candor-xcfg-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(d.join("src")).unwrap();
    d
}

fn put(dir: &Path, rel: &str, body: &str) {
    let p = dir.join(rel);
    std::fs::create_dir_all(p.parent().unwrap()).unwrap();
    std::fs::write(p, body.replace("EFF", W)).unwrap();
}

/// fn -> (inferred, invisible)
type Rows = BTreeMap<String, (Vec<String>, Vec<String>)>;

fn scan(dir: &Path, env: &[(&str, &str)]) -> Rows {
    let out = dir.join("out");
    let _ = std::fs::remove_dir_all(&out);
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_candor-scan"));
    cmd.arg(dir.to_string_lossy().as_ref()).arg("--out").arg(out.join("r"));
    for k in ["CANDOR_DEPS", "CANDOR_CONFIG", "CANDOR_POLICY", "CANDOR_I1S2_XCFG_OFF"] {
        cmd.env_remove(k);
    }
    for (k, v) in env {
        cmd.env(k, v);
    }
    let o = cmd.output().expect("run candor-scan");
    assert!(o.status.code().is_some_and(|c| c == 0 || c == 1), "scan failed: {}", String::from_utf8_lossy(&o.stderr));
    let mut rows = Rows::new();
    for e in std::fs::read_dir(&out).unwrap() {
        let p = e.unwrap().path();
        let n = p.file_name().unwrap().to_string_lossy().to_string();
        if !n.ends_with(".json") || n.ends_with(".callgraph.json") || n.ends_with(".locs.json") || n.ends_with(".hierarchy.json") {
            continue;
        }
        let v: serde_json::Value = serde_json::from_slice(&std::fs::read(&p).unwrap()).unwrap();
        let strs = |x: &serde_json::Value| -> Vec<String> {
            x.as_array().map(|a| a.iter().filter_map(|e| e.as_str().map(str::to_string)).collect()).unwrap_or_default()
        };
        for f in v["functions"].as_array().unwrap() {
            rows.insert(f["fn"].as_str().unwrap().to_string(), (strs(&f["inferred"]), strs(&f["invisible"])));
        }
    }
    rows
}

fn mio_crate(app: &Path, name: &str, kq: &str, wasi: bool) {
    put(
        app,
        "Cargo.toml",
        &format!("[package]\nname = \"{name}\"\nversion = \"0.1.0\"\nedition = \"2021\"\n[dependencies]\nxdep = {{ path = \"../xdep\" }}\n"),
    );
    put(
        app,
        "src/macros.rs",
        "macro_rules! cfg_os_poll { ($($i:item)*) => { $( #[cfg(not(mio_unsupported))] $i )* } }\n\
         macro_rules! cfg_io_source { ($($i:item)*) => { $( #[cfg(any(unix, windows))] $i )* } }\n",
    );
    put(
        app,
        "src/lib.rs",
        "#![allow(unused)]\n#[macro_use]\nmod macros;\npub mod sys;\npub mod poll;\npub mod io_source;\n\
         pub fn assoc_e() -> usize { let r = poll::Registry::new(); io_source::associate(&r) }\n",
    );
    let wasi_mod = if wasi { "#[cfg(all(target_os = \"wasi\", target_env = \"p1\"))]\ncfg_os_poll! {\n    mod wasip1;\n    pub(crate) use self::wasip1::*;\n}\n" } else { "" };
    put(app, "src/sys/mod.rs", &format!("#[cfg(unix)]\ncfg_os_poll! {{\n    mod unix;\n    pub use self::unix::*;\n}}\n#[cfg(windows)]\ncfg_os_poll! {{\n    mod windows;\n    pub use self::windows::*;\n}}\n{wasi_mod}"));
    if wasi {
        // mio's wasip1 arm: its `id` written in place, the one arm the refused reading can see
        put(app, "src/sys/wasip1/mod.rs", "pub struct Selector { id: usize }\nimpl Selector {\n    pub fn new() -> Selector { Selector { id: 6 } }\n    #[cfg(debug_assertions)]\n    pub(crate) fn id(&self) -> usize { self.id }\n}\n");
    }
    put(app, "src/sys/unix/mod.rs", &format!("pub struct Selector {{ pub(crate) id: usize }}\nimpl Selector {{ pub fn new() -> Selector {{ Selector {{ id: 1 }} }} }}\ncfg_io_source! {{\n    impl Selector {{\n        pub fn id(&self) -> usize {{ {kq} self.id }}\n    }}\n}}\n"));
    put(app, "src/sys/windows/mod.rs", "pub struct Selector { pub(crate) id: usize }\nimpl Selector { pub fn new() -> Selector { Selector { id: 3 } } }\ncfg_io_source! {\n    impl Selector {\n        pub fn id(&self) -> usize { self.id }\n    }\n}\n");
    put(
        app,
        "src/poll.rs",
        "use crate::sys;\npub struct Registry { selector: sys::Selector }\nimpl Registry {\n    pub fn new() -> Registry { Registry { selector: sys::Selector::new() } }\n\
             pub(crate) fn selector(&self) -> &sys::Selector { &self.selector }\n}\n",
    );
    put(app, "src/io_source.rs", "use crate::poll::Registry;\npub fn associate(registry: &Registry) -> usize { registry.selector().id() }\n");
}

#[test]
fn the_cross_file_cfg_lift_is_on_by_default() {
    let root = tmpdir("root");
    let dep = root.join("xdep");
    put(&dep, "Cargo.toml", "[package]\nname = \"xdep\"\nversion = \"0.1.0\"\nedition = \"2021\"\n");
    put(&dep, "src/lib.rs", "pub struct Ext { pub v: u8 }\nimpl Ext { pub fn touch(&self) { EFF } }\n");
    // fxmio1: the effectful arm writes itself
    let m1 = root.join("m1");
    mio_crate(&m1, "xcfgm1", "EFF", false);
    let on = scan(&m1, &[]);
    let off = scan(&m1, &[("CANDOR_I1S2_XCFG_OFF", "1")]);
    assert!(on.get("assoc_e").is_some_and(|r| r.0.contains(&"Unknown".to_string())), "default: the arms are read and the call discloses: {on:?}");
    assert!(off.get("assoc_e").is_none_or(|r| r.0.is_empty()), "XCFG_OFF: the refused reading is the old silence: {off:?}");
    // fxmio2: the effectful arm writes through an unchained dependency; mio's wasip1 arm (written in place) is the one arm the
    // refused reading sees, and its hedge is what answers there
    let m2 = root.join("m2");
    mio_crate(&m2, "xcfgm2", "xdep::Ext { v: 1 }.touch();", true);
    let on = scan(&m2, &[]);
    let off = scan(&m2, &[("CANDOR_I1S2_XCFG_OFF", "1")]);
    for (arm, rows) in [("default", &on), ("XCFG_OFF", &off)] {
        assert!(rows.get("assoc_e").is_some_and(|r| r.0.contains(&"Unknown".to_string())), "{arm}: must disclose: {rows:?}");
    }
}
