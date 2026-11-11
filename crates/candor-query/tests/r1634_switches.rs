//! SPEC ⟨0.42⟩ §3.4 (c) — a gate (and the advisory verbs that answer `ok`) over a report whose producer ran under a set
//! result-moving switch is UNANSWERABLE: exit 2 and the §3.1 refusal document naming the switches (SOUNDNESS R1634).
//! Reports are written by hand (the candor-scan binary is a sibling crate), so the load path is exactly what a real
//! switched report takes. Every refusal arm has its control: the SAME report without the key is judged as before.

use std::path::PathBuf;
use std::process::Command;

fn bin() -> &'static str {
    env!("CARGO_BIN_EXE_candor-query")
}

struct Fx {
    dir: PathBuf,
}
impl Fx {
    fn new(name: &str) -> Self {
        let dir = std::env::temp_dir().join(format!("candor-query-r1634-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("p.candor"), "deny Fs app.f\n").unwrap();
        Fx { dir }
    }
    fn prefix(&self, tag: &str) -> String {
        self.dir.join(tag).to_string_lossy().into_owned()
    }
    /// A one-function report `app.f` performing Fs; `switches` is spliced in verbatim when given (a JSON value).
    fn report(&self, tag: &str, switches: Option<&str>) -> String {
        let sw = switches.map(|s| format!("\"switches\": {s},")).unwrap_or_default();
        let text = format!(
            r#"{{
  "candor": {{ "version": "scan-test", "toolchain": "stable", "spec": "0.41" }},
  "package": "app",
  "analyzed": {{ "count": 1, "digest": "0" }},
  {sw}
  "functions": [ {{ "fn": "app.f", "hash": "app#app.f", "inferred": ["Fs"], "direct": ["Fs"] }} ]
}}"#
        );
        let prefix = self.prefix(tag);
        std::fs::write(format!("{prefix}.app.scan.json"), text).unwrap();
        std::fs::write(format!("{prefix}.app.scan.callgraph.json"), r#"{"app.f":[]}"#).unwrap();
        prefix
    }
    fn policy(&self) -> String {
        self.dir.join("p.candor").to_string_lossy().into_owned()
    }
    fn run(&self, args: &[&str]) -> (i32, String, String) {
        let mut cmd = Command::new(bin());
        cmd.args(args);
        // A CANDOR_ variable inherited from the harness must not move the answer.
        for (k, _) in std::env::vars() {
            if k.starts_with("CANDOR_") {
                cmd.env_remove(k);
            }
        }
        let out = cmd.output().unwrap();
        (out.status.code().unwrap_or(-1), String::from_utf8_lossy(&out.stdout).into_owned(), String::from_utf8_lossy(&out.stderr).into_owned())
    }
}
impl Drop for Fx {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

fn is_switch_refusal(doc: &serde_json::Value, names: &[&str]) -> bool {
    doc["ok"] == false
        && doc["refused"] == true
        && doc.get("violations").is_none()
        && doc["switches"] == serde_json::json!(names)
        && doc["reason"].as_str().is_some_and(|r| names.iter().all(|n| r.contains(n)) && r.contains("Unset"))
}

#[test]
fn gate_report_refuses_over_a_report_that_carries_switches_and_judges_the_same_report_without_them() {
    let fx = Fx::new("gate");
    let plain = fx.report("plain", None);
    let sw = fx.report("sw", Some(r#"["CANDOR_R1608_OFF"]"#));
    let gj = fx.dir.join("v.json").to_string_lossy().into_owned();
    // control: the unswitched report is judged and the denied effect fires
    let (rc, _o, _e) = fx.run(&["gate", "--report", &plain, "--policy", &fx.policy(), "--gate-json", &gj]);
    assert_eq!(rc, 1, "control: a plain report with the denied effect exits 1");
    let doc: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(&gj).unwrap()).unwrap();
    assert!(doc.get("refused").is_none() && doc.get("switches").is_none() && doc["violations"].is_array(), "{doc}");
    // the switched report: exit 2, the refusal document, NO violations although a rule fires on the carried evidence
    let (rc, _o, err) = fx.run(&["gate", "--report", &sw, "--policy", &fx.policy(), "--gate-json", &gj]);
    assert_eq!(rc, 2, "stderr: {err}");
    let doc: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(&gj).unwrap()).unwrap();
    assert!(is_switch_refusal(&doc, &["CANDOR_R1608_OFF"]), "{doc}");
    // …and on the stream sink too (`--json`)
    let (rc, out, _e) = fx.run(&["gate", "--report", &sw, "--policy", &fx.policy(), "--json"]);
    assert_eq!(rc, 2);
    let doc: serde_json::Value = serde_json::from_str(&out).unwrap();
    assert!(is_switch_refusal(&doc, &["CANDOR_R1608_OFF"]), "{doc}");
    // the names are unioned across reports under one locator, code-point sorted, each once
    let a = fx.report("multi", Some(r#"["CANDOR_R1608_OFF","CANDOR_R1494_OFF"]"#));
    std::fs::write(
        format!("{a}.zzz.scan.json"),
        r#"{"candor":{"version":"scan-test","toolchain":"stable","spec":"0.41"},"package":"zzz","analyzed":{"count":0,"digest":"0"},"switches":["CANDOR_R1494_OFF","CANDOR_A_FIRST"],"functions":[]}"#,
    )
    .unwrap();
    let (rc, out, _e) = fx.run(&["gate", "--report", &a, "--policy", &fx.policy(), "--json"]);
    assert_eq!(rc, 2);
    let doc: serde_json::Value = serde_json::from_str(&out).unwrap();
    assert_eq!(doc["switches"], serde_json::json!(["CANDOR_A_FIRST", "CANDOR_R1494_OFF", "CANDOR_R1608_OFF"]), "{doc}");
}

#[test]
fn a_present_but_unreadable_switches_key_is_corrupt_input_never_no_switches() {
    let fx = Fx::new("corrupt");
    for bad in ["[1]", "\"CANDOR_X\"", "[\"\"]", "{\"a\":1}"] {
        let p = fx.report("bad", Some(bad));
        let (rc, _o, err) = fx.run(&["gate", "--report", &p, "--policy", &fx.policy()]);
        assert_eq!(rc, 2, "switches={bad}: {err}");
        assert!(err.contains("`switches`"), "the refusal names the key (switches={bad}): {err}");
    }
    // an EMPTY list is a run under no switch: judged
    let p = fx.report("empty", Some("[]"));
    let (rc, _o, _e) = fx.run(&["gate", "--report", &p, "--policy", &fx.policy()]);
    assert_eq!(rc, 1, "an empty `switches` list is not a switch");
}

#[test]
fn the_advisory_verbs_that_answer_ok_refuse_with_or_without_strict_and_fix_does_not() {
    let fx = Fx::new("verbs");
    let plain = fx.report("plain", None);
    let sw = fx.report("sw", Some(r#"["CANDOR_R1608_OFF"]"#));
    let pol = fx.policy();
    for (verb, extra) in [("unverified", vec![]), ("unverified", vec!["--strict"]), ("fix-gate", vec![]), ("fix-gate", vec!["--strict"])] {
        let mut a = vec![verb, "--report", sw.as_str(), "--policy", pol.as_str(), "--json"];
        a.extend(extra.iter().copied());
        let (rc, out, _e) = fx.run(&a);
        assert_eq!(rc, 2, "{verb} {extra:?} over a switched report");
        let doc: serde_json::Value = serde_json::from_str(&out).unwrap_or_else(|e| panic!("{verb}: {e}: {out}"));
        assert!(is_switch_refusal(&doc, &["CANDOR_R1608_OFF"]), "{verb}: {doc}");
        // control: the same verb over the unswitched report is not refused
        let mut c = vec![verb, "--report", plain.as_str(), "--policy", pol.as_str(), "--json"];
        c.extend(extra.iter().copied());
        let (rc, out, _e) = fx.run(&c);
        assert!(!out.contains("\"refused\""), "{verb} {extra:?} control: {out} (rc {rc})");
    }
    let (rc, _o, err) = fx.run(&["whatif", "app.f", "Fs", "--report", &sw, "--policy", &pol]);
    assert_eq!(rc, 2, "whatif over a switched report: {err}");
    assert!(err.contains("CANDOR_R1608_OFF"), "{err}");
    let (rc, _o, _e) = fx.run(&["whatif", "app.f", "Fs", "--report", &plain, "--policy", &pol]);
    assert_ne!(rc, 2, "control: whatif over the unswitched report answers");
    // `fix` answers no `ok` and is not bound (it still answers over the switched report)
    let (rc, _o, err) = fx.run(&["fix", "app.f", "Fs", "--report", &sw, "--policy", &pol]);
    assert_ne!(rc, 2, "fix is not bound by §3.4 (c): {err}");
    assert!(!err.contains("result-moving switch"), "{err}");
}
