//! SOUNDNESS R1617 — second effects for the Log rows and the postgres-connect / first-use-clock rows, charged at the scan's
//! main path, with the `deny <E2>` 0 -> 1 flip on a fixture and a same-type pure neighbour at 0.
//!
//! Every positive was EXECUTED with the effect observed in the call window (see `ALSO_EFFECTS` row evidence); the pure
//! neighbours were EXECUTED at 0 events / 0 log records.

use std::path::{Path, PathBuf};
use std::process::Command;

fn bin() -> &'static str {
    env!("CARGO_BIN_EXE_candor-scan")
}

fn fixture(tag: &str, src: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("candor-r1617-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(d.join("src")).unwrap();
    std::fs::write(
        d.join("Cargo.toml"),
        "[package]\nname = \"fx\"\nversion = \"0.1.0\"\nedition = \"2021\"\n[dependencies]\ntokio-postgres = \"0.7\"\npostgres = \"0.19\"\nreqwest = \"0.13\"\nfastrand = \"2\"\ntempfile = \"3\"\n",
    )
    .unwrap();
    std::fs::write(d.join("src/lib.rs"), src).unwrap();
    d
}

fn scan(dir: &Path, policy: Option<&str>) -> (i32, serde_json::Value) {
    let mut cmd = Command::new(bin());
    cmd.arg(dir.to_string_lossy().as_ref()).arg("--json");
    let pf = dir.join("policy.txt");
    if let Some(p) = policy {
        std::fs::write(&pf, p).unwrap();
        cmd.arg("--policy").arg(&pf);
    }
    let out = cmd
        .env_remove("CANDOR_DEPS")
        .env_remove("CANDOR_CONFIG")
        .env_remove("CANDOR_POLICY")
        .output()
        .expect("run candor-scan");
    let code = out.status.code().unwrap_or(-1);
    assert!(code == 0 || code == 1, "scan failed ({code}): {}", String::from_utf8_lossy(&out.stderr));
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).expect("report json");
    (code, v)
}

fn field(v: &serde_json::Value, f: &str, key: &str) -> Vec<String> {
    let row = v["functions"].as_array().unwrap().iter().find(|r| r["fn"].as_str() == Some(f));
    let mut out: Vec<String> = row
        .and_then(|r| r[key].as_array())
        .map(|a| a.iter().filter_map(|e| e.as_str().map(str::to_string)).collect())
        .unwrap_or_default();
    out.sort();
    out
}


#[test]
fn log_and_connect_rows_flip_their_gates_and_pure_neighbours_do_not() {
    let src = "\
pub async fn f_query(c: &tokio_postgres::Client) { let _ = c.query(\"select 1\", &[]).await; }\n\
pub async fn f_typed(c: &tokio_postgres::Client) { let _ = c.query_typed(\"select 1\", &[]).await; }\n\
pub fn c_closed(c: &tokio_postgres::Client) -> bool { c.is_closed() }\n\
pub fn f_sync(c: &mut postgres::Client) { let _ = c.batch_execute(\"select 1\"); }\n\
pub fn f_connect(cfg: &postgres::Config) { let _ = cfg.connect(postgres::NoTls); }\n\
pub async fn f_exec(c: &reqwest::Client, r: reqwest::Request) { let _ = c.execute(r).await; }\n\
pub fn f_fr() -> u32 { fastrand::u32(..) }\n\
pub fn c_fr_seeded() -> fastrand::Rng { fastrand::Rng::with_seed(7) }\n\
pub fn f_tf() { let _ = tempfile::NamedTempFile::new(); }\n\
pub fn c_tf_path(f: &tempfile::NamedTempFile) -> &std::path::Path { f.path() }\n";
    let d = fixture("main", src);
    let (_, v) = scan(&d, None);
    let s = |x: &[&str]| x.iter().map(|e| e.to_string()).collect::<Vec<_>>();
    assert_eq!(field(&v, "f_query", "inferred"), s(&["Db", "Log"]), "{v:#}");
    assert_eq!(field(&v, "f_sync", "inferred"), s(&["Db", "Log"]));
    assert_eq!(field(&v, "f_typed", "inferred"), s(&["Db"]), "query_typed logs nothing (EXECUTED 0 records)");
    assert_eq!(field(&v, "f_connect", "inferred"), s(&["Db", "Ipc", "Net", "Rand"]));
    assert_eq!(field(&v, "f_exec", "inferred"), s(&["Log", "Net"]));
    assert_eq!(field(&v, "f_fr", "inferred"), s(&["Clock", "Rand"]));
    assert_eq!(field(&v, "f_tf", "inferred"), s(&["Clock", "Fs"]));
    for f in ["c_closed", "c_fr_seeded", "c_tf_path"] {
        assert!(field(&v, f, "inferred").is_empty(), "{f} is a pure neighbour:\n{v:#}");
    }
    // The gate flips: `deny <E2>` exits 1 over the code that really does it (it was 0 before this row), 0 over its neighbour.
    for (e, f) in [("Log", "f_query"), ("Log", "f_sync"), ("Log", "f_exec"), ("Net", "f_connect"), ("Ipc", "f_connect"),
                   ("Rand", "f_connect"), ("Clock", "f_fr"), ("Clock", "f_tf")] {
        let (code, _) = scan(&d, Some(&format!("deny {e} {f}\n")));
        assert_eq!(code, 1, "deny {e} {f}");
    }
    for (e, f) in [("Log", "f_typed"), ("Log", "c_closed"), ("Clock", "c_fr_seeded"), ("Clock", "c_tf_path")] {
        let (code, _) = scan(&d, Some(&format!("deny {e} {f}\n")));
        assert_eq!(code, 0, "deny {e} {f} must stay clean");
    }
    // a Net second effect has a destination the call's literal does not name: marked, never certified
    assert_eq!(field(&v, "f_connect", "incomplete"), s(&["Net"]));
    let _ = std::fs::remove_dir_all(&d);
}
