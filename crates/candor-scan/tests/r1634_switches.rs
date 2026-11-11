//! SPEC ⟨0.42⟩ §2 `switches` / §3.4 / §3.3 cause (e) — SOUNDNESS R1634, candor-scan's half.
//!
//! A result-moving environment variable is NAMED in the report when it was set and consulted, `spec` is unchanged, a gate
//! under one is the §3.1 refusal (exit 2, the report still written, a violation does not dominate), a chained report that
//! carries one gets an advisory with the exit untouched, and an `--incremental` run that consulted one stores nothing.
//! ONE REAL SWITCH on a fixture where it demonstrably moves a result: `CANDOR_R1608_OFF` removes `c_let_chain` from
//! `functions[]` (the same fixture and switch conformance PART 109 uses), so every arm below has its control.

use std::path::{Path, PathBuf};
use std::process::Command;

fn bin() -> &'static str {
    env!("CARGO_BIN_EXE_candor-scan")
}

const SRC: &str = "fn hit() { let _ = std::fs::write(\"/tmp/candor-r1634-never-run\", \"x\"); }\n\
pub struct H;\npub struct F(u8);\n\
impl H { pub fn new() -> Self { H } pub fn find(&self) -> F { F(0) } }\n\
impl F { pub fn filter(self, x: u8) -> Self { F(self.0 + x) } pub fn all(self) { hit() } }\n\
pub fn c_let_chain() { let f = H::new().find().filter(1); f.all() }\n\
pub fn c_pure() -> u8 { 1 }\n";

struct Fx {
    dir: PathBuf,
}
impl Fx {
    fn new(tag: &str) -> Self {
        let dir = std::env::temp_dir().join(format!("candor-r1634-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("src")).unwrap();
        std::fs::write(dir.join("Cargo.toml"), "[package]\nname = \"swfx\"\nversion = \"0.1.0\"\nedition = \"2021\"\n").unwrap();
        std::fs::write(dir.join("src/lib.rs"), SRC).unwrap();
        std::fs::write(dir.join("p.candor"), "deny Fs c_let_chain\n").unwrap();
        Fx { dir }
    }
    fn p(&self, rel: &str) -> String {
        self.dir.join(rel).to_string_lossy().into_owned()
    }
    /// Run the scanner over the fixture. `env` is the ONLY CANDOR_ state the child sees.
    fn run(&self, args: &[&str], env: &[(&str, &str)]) -> (i32, String, String) {
        let mut cmd = Command::new(bin());
        cmd.arg(&self.dir).args(args);
        for (k, _) in std::env::vars() {
            if k.starts_with("CANDOR_") {
                cmd.env_remove(k);
            }
        }
        for (k, v) in env {
            cmd.env(k, v);
        }
        let out = cmd.output().unwrap();
        (out.status.code().unwrap_or(-1), String::from_utf8_lossy(&out.stdout).into_owned(), String::from_utf8_lossy(&out.stderr).into_owned())
    }
    fn report_at(&self, out_prefix: &str) -> serde_json::Value {
        let dir = Path::new(out_prefix).parent().unwrap();
        let stem = Path::new(out_prefix).file_name().unwrap().to_string_lossy().into_owned();
        let mut found: Vec<PathBuf> = std::fs::read_dir(dir)
            .unwrap()
            .filter_map(Result::ok)
            .map(|e| e.path())
            .filter(|p| {
                let n = p.file_name().unwrap().to_string_lossy().into_owned();
                n.starts_with(&stem) && n.ends_with(".scan.json") && !n.contains("callgraph") && !n.contains("hierarchy")
            })
            .collect();
        found.sort();
        assert_eq!(found.len(), 1, "exactly one report under {out_prefix}: {found:?}");
        serde_json::from_str(&std::fs::read_to_string(&found[0]).unwrap()).unwrap()
    }
}
impl Drop for Fx {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

fn has_unit(v: &serde_json::Value, name: &str) -> bool {
    v["functions"].as_array().unwrap().iter().any(|f| f["fn"] == name)
}

#[test]
fn the_report_names_a_set_and_consulted_switch_and_only_that() {
    let fx = Fx::new("report");
    let (_rc, out, _e) = fx.run(&["--json"], &[]);
    let plain: serde_json::Value = serde_json::from_str(&out).unwrap();
    assert!(has_unit(&plain, "c_let_chain"), "control: the unswitched scan reports the chain");
    assert!(plain.get("switches").is_none(), "a scan under no switch is byte-identical to a pre-rung report: no `switches` key");
    let (_rc, out, _e) = fx.run(&["--json"], &[("CANDOR_R1608_OFF", "1")]);
    let sw: serde_json::Value = serde_json::from_str(&out).unwrap();
    assert!(!has_unit(&sw, "c_let_chain"), "the switch must MOVE a result on this fixture or nothing below measures anything");
    assert_eq!(sw["switches"], serde_json::json!(["CANDOR_R1608_OFF"]));
    assert_eq!(sw["candor"]["spec"], plain["candor"]["spec"], "§3.4 (b): `spec` is not changed");
    // a prefixed name the engine never reads is not listed (dumping the environment would bury the one that mattered)
    let (_rc, out, _e) = fx.run(&["--json"], &[("CANDOR_R0_OFF", "1"), ("CANDOR_SOME_HARNESS_VAR", "x")]);
    let un: serde_json::Value = serde_json::from_str(&out).unwrap();
    assert!(un.get("switches").is_none(), "{un}");
    // a diagnostic variable only writes to stderr: it moves no byte and is not a switch
    let (_rc, out, err) = fx.run(&["--json"], &[("CANDOR_ALIAS_DEBUG", "1"), ("CANDOR_R1608_INSTR", "1")]);
    let dbg: serde_json::Value = serde_json::from_str(&out).unwrap();
    assert!(dbg.get("switches").is_none() && has_unit(&dbg, "c_let_chain"), "trace reads record nothing and move nothing");
    assert!(!err.is_empty());
    // sorted, each name once, however many sites read it
    let (_rc, out, _e) = fx.run(&["--json"], &[("CANDOR_R1608_OFF", "1"), ("CANDOR_R1494_OFF", "1"), ("CANDOR_R1319G_OFF", "1")]);
    let many: serde_json::Value = serde_json::from_str(&out).unwrap();
    let names: Vec<String> = many["switches"].as_array().unwrap().iter().map(|x| x.as_str().unwrap().to_string()).collect();
    let mut sorted = names.clone();
    sorted.sort();
    sorted.dedup();
    assert_eq!(names, sorted, "code-point sorted, no duplicates");
    assert!(names.contains(&"CANDOR_R1608_OFF".to_string()));
}

#[test]
fn a_policy_run_under_a_set_switch_refuses_writes_its_report_and_a_violation_does_not_dominate() {
    let fx = Fx::new("gate");
    let (pol, gj, out) = (fx.p("p.candor"), fx.p("v.json"), fx.p("out/r"));
    // control: unswitched, the denied effect fires
    let (rc, _o, _e) = fx.run(&["--policy", &pol, "--gate-json", &gj, "--out", &out], &[]);
    assert_eq!(rc, 1, "control");
    let v: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(&gj).unwrap()).unwrap();
    assert!(v["violations"].as_array().is_some_and(|a| !a.is_empty()) && v.get("refused").is_none(), "{v}");
    // switched: the chain's Fs is gone from the report, so the OLD behaviour was exit 0 ok:true. Now exit 2, refusal.
    let out2 = fx.p("out2/r");
    let (rc, _o, err) = fx.run(&["--policy", &pol, "--gate-json", &gj, "--out", &out2], &[("CANDOR_R1608_OFF", "1")]);
    assert_eq!(rc, 2, "stderr: {err}");
    let v: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(&gj).unwrap()).unwrap();
    assert_eq!(v["ok"], false);
    assert_eq!(v["refused"], true);
    assert!(v.get("violations").is_none(), "the refusal makes no claim about violations: {v}");
    assert_eq!(v["switches"], serde_json::json!(["CANDOR_R1608_OFF"]));
    assert!(v["reason"].as_str().unwrap().contains("CANDOR_R1608_OFF") && v["reason"].as_str().unwrap().contains("Unset"), "{v}");
    // the refused run STILL WROTE its report, carrying `switches` (the report is what a harness attributes on)
    let rep = fx.report_at(&out2);
    assert_eq!(rep["switches"], serde_json::json!(["CANDOR_R1608_OFF"]));
    // a violation established by the switched engine does not dominate: here the ARM that keeps the finding is `deny` over a
    // unit the switch does NOT remove, so the rule fires on carried evidence and the run is STILL exit 2
    std::fs::write(fx.p("p2.candor"), "deny Fs hit\n").unwrap();
    let (rc, _o, _e) = fx.run(&["--policy", &fx.p("p2.candor"), "--gate-json", &gj, "--out", &fx.p("out3/r")], &[]);
    assert_eq!(rc, 1, "control: a violation on a unit the switch leaves alone");
    let (rc, _o, _e) = fx.run(&["--policy", &fx.p("p2.candor"), "--gate-json", &gj, "--out", &fx.p("out4/r")], &[("CANDOR_R1608_OFF", "1")]);
    assert_eq!(rc, 2, "the finding of a switched engine is not evidence: exit 2, not 1");
    // a scan that asks no question has nothing to refuse
    let (rc, _o, _e) = fx.run(&["--json"], &[("CANDOR_R1608_OFF", "1")]);
    assert_eq!(rc, 0, "no policy, no baseline: disclosure only, nothing refuses");
    // the stream sink gets the same document
    let (rc, o, _e) = fx.run(&["--policy", &pol, "--gate-json", "-", "--out", &fx.p("out5/r")], &[("CANDOR_R1608_OFF", "1")]);
    assert_eq!(rc, 2);
    let v: serde_json::Value = serde_json::from_str(&o).unwrap();
    assert_eq!(v["refused"], true);
    assert_eq!(v["switches"], serde_json::json!(["CANDOR_R1608_OFF"]));
}

#[test]
fn a_report_that_carries_switches_makes_the_gate_over_it_refuse_exactly_as_the_scan_did() {
    // §3.1 byte-equality over "any report a scan produced": the verdict document the refused scan wrote and the document
    // `candor-query gate --report` writes over the report that scan produced must agree.
    let fx = Fx::new("routes");
    let (pol, gj) = (fx.p("p.candor"), fx.p("v.json"));
    let out = fx.p("out/r");
    let (rc, _o, _e) = fx.run(&["--policy", &pol, "--gate-json", &gj, "--out", &out], &[("CANDOR_R1608_OFF", "1")]);
    assert_eq!(rc, 2);
    let scan_doc = std::fs::read_to_string(&gj).unwrap();
    let q = Path::new(bin()).with_file_name("candor-query");
    if !q.exists() {
        eprintln!("candor-query is not built beside candor-scan ({}): the route-equality arm is covered by candor-query's own tests", q.display());
        return;
    }
    let gj2 = fx.p("v2.json");
    let o = Command::new(&q)
        .args(["gate", "--report", &out, "--policy", &pol, "--gate-json", &gj2])
        .env_remove("CANDOR_R1608_OFF")
        .output()
        .unwrap();
    assert_eq!(o.status.code(), Some(2), "{}", String::from_utf8_lossy(&o.stderr));
    assert_eq!(scan_doc, std::fs::read_to_string(&gj2).unwrap(), "the two routes write one document over one switched report");
}

#[test]
fn a_chained_report_that_carries_switches_gets_an_advisory_and_the_exit_is_untouched() {
    let fx = Fx::new("chain");
    // a dependency report, scanned for real, then the key injected by hand (the way a switched producer's report looks)
    let dep = Fx::new("chain-dep");
    std::fs::write(dep.dir.join("Cargo.toml"), "[package]\nname = \"swdep\"\nversion = \"0.1.0\"\nedition = \"2021\"\n").unwrap();
    std::fs::write(dep.dir.join("src/lib.rs"), "pub fn run() { let _ = std::fs::write(\"/tmp/candor-r1634-never-run\", \"x\"); }\n").unwrap();
    let dprefix = dep.p("dr/r");
    let (rc, _o, e) = dep.run(&["--out", &dprefix], &[]);
    assert!(rc == 0 || rc == 1, "{e}");
    let rep_path = std::fs::read_dir(dep.dir.join("dr"))
        .unwrap()
        .filter_map(Result::ok)
        .map(|e| e.path())
        .find(|p| p.to_string_lossy().ends_with(".scan.json") && !p.to_string_lossy().contains("callgraph") && !p.to_string_lossy().contains("hierarchy"))
        .unwrap();
    let plain_dir = fx.dir.join("deps-plain");
    let inj_dir = fx.dir.join("deps-inj");
    for d in [&plain_dir, &inj_dir] {
        std::fs::create_dir_all(d).unwrap();
    }
    let mut v: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(&rep_path).unwrap()).unwrap();
    let fname = rep_path.file_name().unwrap();
    std::fs::write(plain_dir.join(fname), serde_json::to_string(&v).unwrap()).unwrap();
    v["switches"] = serde_json::json!(["CANDOR_R1608_OFF"]);
    std::fs::write(inj_dir.join(fname), serde_json::to_string(&v).unwrap()).unwrap();
    for sfx in [".callgraph.json", ".hierarchy.json"] {
        let side = PathBuf::from(rep_path.to_string_lossy().replace(".json", sfx));
        if side.exists() {
            for d in [&plain_dir, &inj_dir] {
                std::fs::copy(&side, d.join(side.file_name().unwrap())).unwrap();
            }
        }
    }
    // the consumer
    std::fs::write(fx.dir.join("Cargo.toml"), "[package]\nname = \"swapp\"\nversion = \"0.1.0\"\nedition = \"2021\"\n[dependencies]\nswdep = { path = \"../dep\" }\n").unwrap();
    std::fs::write(fx.dir.join("src/lib.rs"), "pub fn go() { swdep::run() }\n").unwrap();
    std::fs::write(fx.dir.join("chain.candor"), "deny Fs go\n").unwrap();
    let chain = |d: &Path, tag: &str| {
        let gj = fx.p(&format!("{tag}.json"));
        let (rc, _o, err) = fx.run(
            &["--policy", &fx.p("chain.candor"), "--gate-json", &gj, "--out", &fx.p(&format!("o-{tag}/r"))],
            &[("CANDOR_DEPS", d.to_string_lossy().as_ref())],
        );
        (rc, err, serde_json::from_str::<serde_json::Value>(&std::fs::read_to_string(&gj).unwrap()).unwrap())
    };
    let (rc_p, err_p, doc_p) = chain(&plain_dir, "plain");
    assert_eq!(rc_p, 1, "the join is live: the plain chain fires. {err_p}");
    let (rc_i, err_i, doc_i) = chain(&inj_dir, "inj");
    assert_eq!(rc_i, rc_p, "§3.4 (d): the exit is untouched. {err_i}");
    assert_eq!(err_i.matches("CANDOR_R1608_OFF").count(), 1, "the advisory names the switch once: {err_i}");
    assert!(!err_p.contains("CANDOR_R1608_OFF"));
    assert_eq!(doc_i["switchedDependencies"], serde_json::json!([{ "package": "swdep", "switches": ["CANDOR_R1608_OFF"] }]), "{doc_i}");
    assert!(doc_p.get("switchedDependencies").is_none());
    let (mut a, mut b) = (doc_i.clone(), doc_p.clone());
    a.as_object_mut().unwrap().remove("switchedDependencies");
    b.as_object_mut().unwrap().remove("switchedDependencies");
    assert_eq!(a, b, "the rest of the verdict is untouched");
}

#[test]
fn a_run_that_consulted_a_switch_never_primes_the_incremental_cache() {
    // The cache-replay hole: entries derived under a switch, replayed by the next run with the variable UNSET, would give a report
    // carrying the switched rows and naming nothing.
    let fx = Fx::new("cache");
    let cache = fx.dir.join(".candor/cache/scan-cache.json");
    let (rc, _o, e) = fx.run(&["--incremental", "--out", &fx.p("o1/r")], &[("CANDOR_R1608_OFF", "1")]);
    assert!(rc == 0 || rc == 1, "{e}");
    assert!(!cache.exists(), "a switched run must store nothing");
    // …so the next unswitched --incremental run derives the chain from scratch (and does store)
    let (_rc, out, _e) = fx.run(&["--incremental", "--json"], &[]);
    let v: serde_json::Value = serde_json::from_str(&out).unwrap();
    assert!(has_unit(&v, "c_let_chain") && v.get("switches").is_none(), "an unswitched warm run must not replay the switched answer");
    assert!(cache.exists(), "control: an unswitched --incremental run stores its cache");
    // a switched run over a WARM cache reads it (a replayed entry is the unswitched answer) but does not rewrite it
    let before = std::fs::read(&cache).unwrap();
    std::fs::write(fx.dir.join("src/extra.rs"), "pub fn extra() {}\n").unwrap();
    std::fs::write(fx.dir.join("src/lib.rs"), format!("{SRC}mod extra;\n")).unwrap();
    let (rc, _o, e) = fx.run(&["--incremental", "--out", &fx.p("o2/r")], &[("CANDOR_R1608_OFF", "1")]);
    assert!(rc == 0 || rc == 1, "{e}");
    assert_eq!(before, std::fs::read(&cache).unwrap(), "a switched run leaves the cache exactly as it found it");
}
