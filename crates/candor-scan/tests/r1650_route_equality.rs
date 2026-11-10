//! SOUNDNESS R1650 — SPEC §3.1 route equality over a trait default's per-implementor INSTANCE (R1420(a)).
//! `scan --policy` gated over `Tr::go\u{1f}inst\u{1f}P`, a unit the report never contains, so `candor-query gate --report`
//! over the report the same scan wrote listed fewer violations: the verdicts differed by the rows naming the instance.
//! Each case runs BOTH routes over one fixture and asserts the verdict bytes are equal and the exits are equal.
//! (Executed in the lane: an instance never fires a policy its default does not — its calls are a subset of the default's
//! and its name only extends the default's last segment — so equality is reached by gating the written units, never by
//! dropping a real violation: the effectful default still fires, and so does every caller.)

use std::path::PathBuf;
use std::process::Command;

fn scan_bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_candor-scan"))
}

/// `candor-query` is a sibling binary of the same target dir; build it if this test target ran alone. Never skip: a skipped
/// route comparison would be a vacuous pass.
fn query_bin() -> PathBuf {
    let sib = scan_bin().with_file_name(format!("candor-query{}", std::env::consts::EXE_SUFFIX));
    if !sib.exists() {
        let cargo = std::env::var("CARGO").unwrap_or_else(|_| "cargo".into());
        let target = sib.parent().unwrap().parent().unwrap().to_path_buf();
        let st = Command::new(cargo)
            .args(["build", "-p", "candor-query", "--target-dir"])
            .arg(&target)
            .status()
            .expect("cargo build candor-query");
        assert!(st.success() && sib.exists(), "could not build candor-query next to candor-scan");
    }
    sib
}

const SRC: &str = "pub trait Tr { fn act(&self); fn go(&self) { self.act() } }\n\
pub struct P; impl Tr for P { fn act(&self) { let _ = std::fs::read(\"/x\"); } }\n\
pub struct Q; impl Tr for Q { fn act(&self) {} }\n\
pub fn caller_p() { P.go() }\npub fn caller_q() { Q.go() }\n";

fn routes(name: &str, policy: &str) -> (i32, i32, String, String) {
    let d = std::env::temp_dir().join(format!("candor-r1650-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(d.join("src")).unwrap();
    std::fs::write(d.join("Cargo.toml"), "[package]\nname = \"fxr\"\n").unwrap();
    std::fs::write(d.join("src/lib.rs"), SRC).unwrap();
    let pol = d.join("pol");
    std::fs::write(&pol, policy).unwrap();
    let (rep, a, b) = (d.join("rep"), d.join("a.json"), d.join("b.json"));
    let s = Command::new(scan_bin())
        .arg(&d)
        .arg("--out")
        .arg(&rep)
        .arg("--policy")
        .arg(&pol)
        .arg("--gate-json")
        .arg(&a)
        .env_remove("CANDOR_CT3A_OFF")
        .output()
        .expect("run candor-scan");
    let q = Command::new(query_bin())
        .args(["gate", "--report"])
        .arg(&rep)
        .arg("--policy")
        .arg(&pol)
        .arg("--gate-json")
        .arg(&b)
        .output()
        .expect("run candor-query");
    let (ta, tb) = (std::fs::read_to_string(&a).expect("scan verdict"), std::fs::read_to_string(&b).expect("query verdict"));
    let _ = std::fs::remove_dir_all(&d);
    (s.status.code().unwrap(), q.status.code().unwrap(), ta, tb)
}

#[test]
fn a_default_instance_is_not_gated_by_the_scan_route_when_the_report_route_cannot_see_it() {
    for policy in ["deny Fs\n", "deny Fs go\n", "deny Fs Tr\n", "pure go\n", "deny Fs caller_p\n"] {
        let (ra, rb, a, b) = routes("eq", policy);
        assert_eq!(ra, 1, "{policy:?}: the effectful default must still fire on the scan route");
        assert_eq!(rb, 1, "{policy:?}: ...and on the report route");
        assert!(
            !a.contains("\\u001finst\\u001f"),
            "{policy:?}: the verdict names an INSTANCE unit no report contains:\n{a}"
        );
        assert_eq!(a, b, "{policy:?}: §3.1 — `gate --report` over the scan's report must be byte-equal to `scan --policy`");
    }
}

#[test]
fn the_pure_implementors_caller_stays_clear_on_both_routes_and_the_effectful_ones_stay_charged() {
    // The narrowing R1420(a) made must survive: caller_q is clean, caller_p is charged, on BOTH routes.
    let (ra, rb, a, b) = routes("callers", "deny Fs caller_q\n");
    assert_eq!((ra, rb), (0, 0), "caller_q runs only Q's pure act");
    assert_eq!(a, b);
    let (ra, rb, a, b) = routes("callerp", "deny Fs caller_p\n");
    assert_eq!((ra, rb), (1, 1), "caller_p runs P's Fs act: no silence");
    assert!(a.contains("caller_p"), "{a}");
    assert_eq!(a, b);
}
