//! THE ONE PLACE THIS WORKSPACE READS ITS PROCESS ENVIRONMENT (SPEC §3.4 ⟨0.42⟩, SOUNDNESS R1634).
//!
//! Every environment-VARIABLE read in every crate goes through a function below, and
//! `crates/candor-scan/tests/env_lint.rs` fails the suite on any other (shown failing on seeded spellings and on a
//! real source file with one raw read appended). That is what makes the clause enforceable without a list: §3.4 says
//! "the registration is the read", because a list of switch names beside the code that reads them is an allowlist,
//! and it fails in the ABSENCE direction — a switch added without its entry is exactly the silent one. Here the read
//! IS the registration: there is no table of names in this file.
//!
//! FOUR KINDS OF READ, and the kind is chosen at the call site by what the variable does:
//!
//! * [`switch`] / [`switch_value`] — a RESULT-MOVING switch: a kill switch for a closed defect, an opt-in mode, a
//!   measurement arm. Its value selects which logic runs. When the value, as the caller reads it, selects a branch
//!   other than the one an unset variable selects, the name is recorded. The recorded names are the report's
//!   `switches` (§2) and make a gate refuse (§3.3 cause (e)).
//! * [`trace`] — a DIAGNOSTIC: the value only decides whether something is written to stderr. No report or verdict
//!   byte can depend on it, so nothing is recorded (§3.4: "a read that only writes to stderr is out of scope"). A
//!   trace read whose answer reaches a result is a switch: reclassify it.
//! * [`operator`] / [`operator_os`] — an INPUT the operator supplies: a path to a policy, baseline, dependency
//!   report or configuration file, the report locator, a registry location, a tooling sink. Configuration, out of
//!   scope (§3.4: "a variable that SUPPLIES AN INPUT").
//!
//! An engine MUST NOT read every switch at process start (§3.4 (a)): a read happens where the logic the variable
//! selects is reached, and a name is consulted only when that read EXECUTES. So `gate --report` in an environment
//! holding a scan-time switch consults nothing and judges a clean report as it would without the variable.

use std::collections::BTreeSet;
use std::ffi::OsString;
use std::sync::Mutex;

/// The set of switch names SET and CONSULTED so far. A type (not a bare static) so the unit tests can exercise one without touching the
/// process-wide registry: every report this process writes names the global one, so a test that recorded into it would mark every
/// later report in the same test binary as switched.
struct Consulted(Mutex<BTreeSet<String>>);

impl Consulted {
    const fn new() -> Self {
        Consulted(Mutex::new(BTreeSet::new()))
    }
    fn record(&self, name: &str) {
        // A `match` with a guard, not nested `if`s: the nested form trips `clippy::collapsible_if` on a newer stable than this repo's
        // floor, and the collapsed form needs let-chains. The `contains` keeps a hot path (a switch read at every call site) from
        // allocating a name it already holds.
        match self.0.lock() {
            Ok(mut s) if !s.contains(name) => {
                s.insert(name.to_string());
            }
            _ => {}
        }
    }
    /// Code-point order: `BTreeSet<String>` iterates in byte order, which for UTF-8 is code-point order.
    fn names(&self) -> Vec<String> {
        self.0.lock().map(|s| s.iter().cloned().collect()).unwrap_or_default()
    }
    fn clear(&self) {
        if let Ok(mut s) = self.0.lock() {
            s.clear();
        }
    }
}

static CONSULTED: Consulted = Consulted::new();

/// A RESULT-MOVING switch whose non-default branch is selected by the variable's PRESENCE (every `_OFF` kill switch
/// and every `=1`-style opt-in this engine ships). Records `name` when it is present.
pub fn switch(name: &str) -> bool {
    switch_value(name, Selects::Present)
}

/// WHICH VALUES of a switch select the non-default branch. An enum and not a closure on purpose: a closure parameter is a call the
/// engine's own effect analysis cannot resolve, so every caller of a closure-taking `switch` would carry `Unknown` in the engine's
/// self-scan (measured: 113 functions of candor-scan gained it), and the registration would cost the disclosure it exists to improve.
#[derive(Clone, Copy)]
pub enum Selects<'a> {
    /// any value, including the empty one: the variable merely has to exist
    Present,
    /// any UTF-8 value other than this one (`CANDOR_CHARGE_AT_CTOR=0` is the default, so only another value is set)
    NotEqual(&'a str),
    /// exactly one of these UTF-8 values
    AnyOf(&'a [&'a str]),
    /// one of `whole` (switches everything), or a comma-separated list naming `member` (switches only that consumer)
    ListOrAny { whole: &'a [&'a str], member: &'a str },
}

/// A RESULT-MOVING switch whose branch depends on the VALUE: `sel` says which values, as the engine reads them, select a branch
/// other than the one an unset variable selects. The name is recorded exactly when one does, so `CANDOR_X=1` beside a default of
/// "on" is not SET (§3.4 (a)). A value that is not valid UTF-8 selects nothing but `Present`.
pub fn switch_value(name: &str, sel: Selects<'_>) -> bool {
    let set = decide(sel, std::env::var_os(name).as_deref());
    if set {
        CONSULTED.record(name);
    }
    set
}

/// Does `value` (`None` = unset) select a branch other than the one an unset variable selects? Pure, so it is tested without the environment.
fn decide(sel: Selects<'_>, value: Option<&std::ffi::OsStr>) -> bool {
    let Some(v) = value else { return false };
    let text = v.to_str();
    match sel {
        Selects::Present => true,
        Selects::NotEqual(d) => text.is_some_and(|t| t != d),
        Selects::AnyOf(vs) => text.is_some_and(|t| vs.contains(&t)),
        Selects::ListOrAny { whole, member } => text.is_some_and(|t| whole.contains(&t) || t.split(',').any(|k| k.trim() == member)),
    }
}

/// A DIAGNOSTIC read: present or not. Records nothing; the caller promises the answer only reaches stderr.
pub fn trace(name: &str) -> bool {
    std::env::var_os(name).is_some()
}

/// An INPUT the operator supplies (a path, a locator, a sink). Same contract as `std::env::var`.
pub fn operator(name: &str) -> Result<String, std::env::VarError> {
    std::env::var(name)
}

/// [`operator`], without the UTF-8 requirement.
pub fn operator_os(name: &str) -> Option<OsString> {
    std::env::var_os(name)
}

/// The switches SET and CONSULTED by this process so far, sorted by code point, no duplicates (§2 `switches`).
/// `BTreeSet<String>` iterates in byte order, which for UTF-8 is code-point order.
pub fn consulted_switches() -> Vec<String> {
    CONSULTED.names()
}

static SWITCHED_DEPS: Mutex<std::collections::BTreeMap<String, BTreeSet<String>>> = Mutex::new(std::collections::BTreeMap::new());

/// ⟨0.42⟩ SPEC §3.4 (d): a chained dependency report CARRIED `switches`, so it was produced by an altered engine and an absent row in it is
/// a claim (⟨0.21⟩'s purity claim) about code that engine misread. Recorded here by the dependency loader, read by the verdict writer
/// (`switchedDependencies`, ordered by package, names code-point sorted). The consumer's own run is not switched and cannot judge a
/// dependency's environment, so this never changes an exit code.
pub fn record_switched_dependency(package: &str, names: &[String]) {
    if names.is_empty() {
        return;
    }
    if let Ok(mut m) = SWITCHED_DEPS.lock() {
        m.entry(package.to_string()).or_default().extend(names.iter().cloned());
    }
}

/// The recorded `(package, switches)` pairs, ordered by package.
pub fn switched_dependencies() -> Vec<(String, Vec<String>)> {
    SWITCHED_DEPS.lock().map(|m| m.iter().map(|(p, s)| (p.clone(), s.iter().cloned().collect())).collect()).unwrap_or_default()
}

/// Test seam: forget what was consulted (an in-process harness that runs several scans).
pub fn reset_consulted_switches() {
    CONSULTED.clear();
}

/// The refusal's `reason` (§3.3 (e); §3.1: a refusal always says why): names the switches and says to unset them.
pub fn switch_refusal_reason(names: &[String]) -> String {
    format!(
        "a result-moving switch is set: {} — the answer depends on an environment variable that selects which logic \
         ran, so the gate cannot say whether the code passes (SPEC §3.4). Unset {} and re-run.",
        names.join(", "),
        if names.len() == 1 { "it" } else { "them" }
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::OsStr;

    /// These tests never touch the process-wide registry or the process environment: every report this test binary writes names
    /// the global registry, so a test that recorded into it would mark every other test's report as switched.
    #[test]
    fn names_are_recorded_once_and_come_back_in_code_point_order() {
        let c = Consulted::new();
        c.record("CANDOR_B");
        c.record("CANDOR_A");
        c.record("CANDOR_B");
        c.record("CANDOR_a");
        c.record("CANDOR_É");
        assert_eq!(c.names(), ["CANDOR_A", "CANDOR_B", "CANDOR_a", "CANDOR_É"]);
        c.clear();
        assert!(c.names().is_empty());
    }

    #[test]
    fn a_value_that_selects_the_default_branch_is_not_set() {
        let v = |s: &'static str| Some(OsStr::new(s));
        assert!(!decide(Selects::Present, None), "unset is never set");
        assert!(decide(Selects::Present, v("")), "present, even empty");
        assert!(!decide(Selects::NotEqual("0"), v("0")), "the default value is not a switch");
        assert!(decide(Selects::NotEqual("0"), v("1")), "any other value is");
        assert!(!decide(Selects::AnyOf(&["A", "B"]), v("1")), "a value outside the set selects the default");
        assert!(decide(Selects::AnyOf(&["A", "B"]), v("B")));
        let sel = |member| Selects::ListOrAny { whole: &["", "1", "all"], member };
        assert!(!decide(sel("try_into"), v("parse,collect")), "a list naming only other consumers selects the default for this one");
        assert!(decide(sel("collect"), v("parse, collect")), "a list naming this consumer is a switch (whitespace tolerated)");
        assert!(decide(sel("anything"), v("")), "the empty value switches every consumer");
        assert!(decide(sel("anything"), v("all")));
        #[cfg(unix)]
        {
            use std::os::unix::ffi::OsStrExt;
            let bad = OsStr::from_bytes(&[0xff, 0xfe]);
            assert!(decide(Selects::Present, Some(bad)), "a non-UTF-8 value is still PRESENT");
            assert!(!decide(Selects::NotEqual("0"), Some(bad)) && !decide(Selects::AnyOf(&["A"]), Some(bad)), "…and selects nothing by value");
        }
    }

    #[test]
    fn the_reason_names_every_switch() {
        let r = switch_refusal_reason(&["A".into(), "B".into()]);
        assert!(r.contains("A, B") && r.contains("Unset them"));
        assert!(switch_refusal_reason(&["A".into()]).contains("Unset it"));
    }
}
