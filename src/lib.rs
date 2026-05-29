#![feature(rustc_private)]
#![warn(unused_extern_crates)]

// candor — a type-aware capability/effect detector for Rust.
//
// Stateful. We record, per function, (a) the effects it performs DIRECTLY and (b) the
// local functions it calls. After the whole crate is seen, we compute a transitive
// fixpoint so each function reports its FULL effect set — including effects inherited
// through helpers it calls. A high-level handler that does no I/O itself still surfaces
// { Net, Fs, ... } if its callees do.
//
// SOUNDNESS / honesty: a call we cannot statically resolve to a callee — dynamic
// dispatch (`dyn Trait`), a function pointer, or a closure reached through a generic
// `impl Fn` parameter — could perform ANY effect. Rather than silently assume such a
// call is pure (the original sin: "lying by omission"), we record an `Unknown` effect.
// In conformance mode a function carrying `Unknown` cannot be certified honest
// (AS-EFF-003). Statically-dispatched generic trait calls are still assumed to honour
// their bound (a documented residual gap), to keep the audit from drowning in Unknown.
//
// The built-in classifier (`classify`) knows a fixed set of crates; a project can add
// its own crate/path → effect rules via a CANDOR_CONFIG file (see `parse_config`).

extern crate rustc_hir;
extern crate rustc_middle;

use std::collections::{BTreeSet, HashMap, HashSet};

use rustc_hir::def::DefKind;
use rustc_hir::def_id::{DefId, LocalDefId};
use rustc_hir::{Expr, ExprKind, HirId};
use rustc_lint::{LateContext, LateLintPass};
use rustc_middle::ty::TyCtxt;

dylint_linting::impl_late_lint! {
    /// ### What it does
    /// Reports, for each function, the transitive set of capabilities/effects it
    /// exercises (network, filesystem, process spawn, env, clock, logging, clipboard),
    /// resolved from callee `DefId`s and propagated through local calls.
    ///
    /// ### Why is this bad?
    /// It isn't — it's an audit. It makes a function's effect surface legible from its
    /// definition, the way an explicit effect annotation would.
    pub CANDOR,
    Warn,
    "aggregates each function's transitive capability/effect set",
    Candor::new()
}

/// The effect recorded for a call candor cannot resolve to a concrete callee.
const UNKNOWN: &str = "Unknown";

pub struct Candor {
    /// Effects performed directly in a function's own body (and its inline closures).
    direct: HashMap<LocalDefId, BTreeSet<&'static str>>,
    /// Local-crate functions each function calls, for transitive propagation.
    calls: HashMap<LocalDefId, HashSet<LocalDefId>>,
    /// Project-supplied classifier rules: (effect, is_crate_prefix, prefix).
    extra: Vec<(&'static str, bool, String)>,
    /// CANDOR_PARANOID: also treat generic static trait dispatch as Unknown.
    paranoid: bool,
}

/// Effects that represent *ambient authority* — a global resource reachable just by
/// naming it (vs. a capability you must be handed). These are what `CANDOR_NO_AMBIENT`
/// and cap-std care about. `Log` is intentionally excluded (not an authority).
const AMBIENT: [&str; 7] = ["Net", "Fs", "Exec", "Env", "Clock", "Clipboard", "Rand"];

impl Candor {
    pub fn new() -> Self {
        let extra = std::env::var("CANDOR_CONFIG")
            .ok()
            .and_then(|p| std::fs::read_to_string(p).ok())
            .map(|s| parse_config(&s))
            .unwrap_or_default();
        let paranoid = std::env::var("CANDOR_PARANOID").is_ok();
        Self { direct: HashMap::new(), calls: HashMap::new(), extra, paranoid }
    }
}

/// Parse a CANDOR_CONFIG file: one rule per line, `<Effect> <crate|path> <prefix>`,
/// blank lines and `#` comments ignored. The effect must be one of the known names.
///
///     # extend the classifier with this project's own effectful crates
///     Net   crate  reqwest
///     Fs    path   mycrate::storage::
fn parse_config(text: &str) -> Vec<(&'static str, bool, String)> {
    let mut out = Vec::new();
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let mut it = line.split_whitespace();
        match (it.next(), it.next(), it.next()) {
            (Some(eff), Some(kind), Some(prefix)) => {
                if let Some(e) = cap_from_name(eff) {
                    out.push((e, kind == "crate", prefix.to_string()));
                }
            }
            _ => {}
        }
    }
    out
}

/// One entry of a saved candor JSON report, for baseline diffing.
#[derive(serde::Deserialize)]
struct BaselineEntry {
    #[serde(rename = "fn")]
    func: String,
    inferred: Vec<String>,
}

/// Load a baseline candor JSON into `fn name -> inferred effect set`.
fn load_baseline(path: &str) -> Option<HashMap<String, BTreeSet<String>>> {
    let text = std::fs::read_to_string(path).ok()?;
    let entries: Vec<BaselineEntry> = serde_json::from_str(&text).ok()?;
    Some(
        entries
            .into_iter()
            .map(|e| (e.func, e.inferred.into_iter().collect()))
            .collect(),
    )
}

/// Project-supplied rules, consulted only when the built-in `classify` returns None.
fn classify_extra(
    crate_name: &str,
    path: &str,
    extra: &[(&'static str, bool, String)],
) -> Option<&'static str> {
    for (eff, is_crate, prefix) in extra {
        let hit = if *is_crate { crate_name.starts_with(prefix.as_str()) } else { path.starts_with(prefix.as_str()) };
        if hit {
            return Some(eff);
        }
    }
    None
}

/// What a call expression resolves to.
enum Callee {
    /// A concrete function/method we can classify and (if local) follow.
    Def(DefId),
    /// A call candor cannot see through — its effects are unknowable here.
    Unresolved,
}

/// Classify a call site. Crucially, calls that cannot be resolved to a concrete callee
/// (dynamic dispatch, fn pointers, calls through `impl Fn` parameters) return
/// `Unresolved` instead of being silently dropped.
fn resolve_callee<'tcx>(cx: &LateContext<'tcx>, expr: &Expr<'tcx>) -> Option<Callee> {
    use rustc_middle::ty::TyKind;
    // Defensive: `typeck_results()` panics (ICE) for an expr outside a typechecked
    // body. An effect checker must never abort the build, so bail gracefully instead.
    let typeck = cx.maybe_typeck_results()?;
    match expr.kind {
        ExprKind::MethodCall(_, receiver, _, _) => {
            let recv_ty = typeck.expr_ty_adjusted(receiver).peel_refs();
            if matches!(recv_ty.kind(), TyKind::Dynamic(..)) {
                return Some(Callee::Unresolved); // dyn dispatch
            }
            match typeck.type_dependent_def_id(expr.hir_id) {
                Some(did) => Some(Callee::Def(did)),
                None => Some(Callee::Unresolved),
            }
        }
        ExprKind::Call(callee, _) => match typeck.expr_ty(callee).kind() {
            TyKind::FnDef(did, _) => Some(Callee::Def(*did)),
            TyKind::FnPtr(..) => Some(Callee::Unresolved), // function pointer
            TyKind::Closure(..) => None,                   // inline closure body counted lexically
            TyKind::Param(..) | TyKind::Alias(..) | TyKind::Dynamic(..) => Some(Callee::Unresolved),
            _ => None,
        },
        _ => None,
    }
}

/// Classify a resolved callee by the crate it belongs to and its full path.
fn classify(crate_name: &str, path: &str) -> Option<&'static str> {
    if crate_name.starts_with("aws_sdk_") || crate_name.starts_with("aws_smithy") {
        // Only request dispatch is network I/O; builder setters/accessors are pure.
        if path.ends_with("::send") || path.ends_with("::send_with") {
            return Some("Net");
        }
        return None;
    }
    // HTTP clients use the same builder pattern as the AWS SDK: only the dispatch is
    // I/O. (Found by the eval: ebman's reqwest calls to the Anthropic API + webhooks
    // were silently classified network-free because reqwest wasn't recognized.)
    if crate_name == "reqwest" || crate_name == "isahc" {
        if path.ends_with("::send") || path.ends_with("::execute") {
            return Some("Net");
        }
        return None;
    }
    if crate_name == "ureq" && path.ends_with("::call") {
        return Some("Net");
    }
    // Raw sockets. Match the I/O *types* only — `std::net` also holds pure data types
    // (SocketAddr, IpAddr, …) whose construction must NOT be flagged.
    if path.starts_with("std::net::TcpStream")
        || path.starts_with("std::net::TcpListener")
        || path.starts_with("std::net::UdpSocket")
        || path.starts_with("tokio::net::")
    {
        return Some("Net");
    }
    if path.starts_with("std::fs::") || path.starts_with("tokio::fs::") {
        return Some("Fs");
    }
    // Randomness / entropy. getrandom + fastrand are effectful end-to-end; `rand` also
    // contains pure distribution constructors, so this slightly over-reports there.
    if crate_name == "rand" || crate_name == "getrandom" || crate_name == "fastrand" {
        return Some("Rand");
    }
    if path.starts_with("std::process::Command")
        || path.starts_with("std::process::Child")
        || crate_name == "portable_pty"
    {
        return Some("Exec");
    }
    if path.starts_with("std::env::") {
        return Some("Env");
    }
    if (crate_name == "chrono" || path.starts_with("std::time::")) && path.contains("now") {
        return Some("Clock");
    }
    if crate_name == "tracing" {
        return Some("Log");
    }
    if crate_name == "arboard" {
        return Some("Clipboard");
    }
    None
}

const EFFECTS: [&str; 8] = ["Net", "Fs", "Exec", "Env", "Clock", "Log", "Clipboard", "Rand"];

fn cap_from_name(name: &str) -> Option<&'static str> {
    EFFECTS.iter().copied().find(|e| *e == name)
}

/// Capabilities a function declares by taking the matching token as a parameter
/// (e.g. `&Fs` declares the right to perform `Fs`). This is the Rust expression of
/// the spec's "capabilities as typed parameters" pillar.
fn declared_caps(tcx: TyCtxt<'_>, def_id: LocalDefId) -> BTreeSet<&'static str> {
    let mut out = BTreeSet::new();
    let did = def_id.to_def_id();
    if !matches!(tcx.def_kind(did), DefKind::Fn | DefKind::AssocFn) {
        return out;
    }
    let sig = tcx.fn_sig(did).instantiate_identity().skip_binder();
    for input in sig.inputs().iter() {
        let ty = input.peel_refs();
        if let Some(adt) = ty.ty_adt_def() {
            if let Some(c) = cap_from_name(tcx.item_name(adt.did()).as_str()) {
                out.insert(c);
            }
        }
    }
    out
}

/// Nearest enclosing *named* function of `hir_id`, walking up out of closures so that
/// effects performed inside an inline closure are charged to the function that owns it.
fn enclosing_named_fn(tcx: TyCtxt<'_>, hir_id: HirId) -> Option<LocalDefId> {
    let mut owner = tcx.hir_enclosing_body_owner(hir_id);
    loop {
        match tcx.def_kind(owner.to_def_id()) {
            DefKind::Fn | DefKind::AssocFn => return Some(owner),
            DefKind::Closure => {
                let closure_hir = tcx.local_def_id_to_hir_id(owner);
                let parent = tcx.hir_enclosing_body_owner(closure_hir);
                if parent == owner {
                    return None;
                }
                owner = parent;
            }
            _ => return None,
        }
    }
}

impl<'tcx> LateLintPass<'tcx> for Candor {
    fn check_expr(&mut self, cx: &LateContext<'tcx>, expr: &'tcx Expr<'tcx>) {
        let Some(callee) = resolve_callee(cx, expr) else {
            return;
        };
        let Some(caller) = enclosing_named_fn(cx.tcx, expr.hir_id) else {
            return;
        };

        let def_id = match callee {
            // A call we cannot see through could do anything — record it honestly.
            Callee::Unresolved => {
                self.direct.entry(caller).or_default().insert(UNKNOWN);
                return;
            }
            Callee::Def(def_id) => def_id,
        };

        // Paranoid: a call resolving to a *trait-declared* method is statically
        // dispatched over a generic bound — the concrete impl (and its effects) are
        // unknown here. Off by default because it would flag every .clone()/.fmt()/etc.
        if self.paranoid && cx.tcx.trait_of_assoc(def_id).is_some() {
            self.direct.entry(caller).or_default().insert(UNKNOWN);
        }

        // Record a local call edge for transitive propagation.
        if let Some(local) = def_id.as_local() {
            if matches!(cx.tcx.def_kind(def_id), DefKind::Fn | DefKind::AssocFn) {
                self.calls.entry(caller).or_default().insert(local);
            }
        }

        // Record a directly-performed effect (built-in classifier, then project rules).
        let crate_name = cx.tcx.crate_name(def_id.krate);
        let path = cx.tcx.def_path_str(def_id);
        let effect = classify(crate_name.as_str(), &path)
            .or_else(|| classify_extra(crate_name.as_str(), &path, &self.extra));
        if let Some(effect) = effect {
            self.direct.entry(caller).or_default().insert(effect);
        }
    }

    fn check_crate_post(&mut self, cx: &LateContext<'tcx>) {
        // effects[f] = direct[f] ∪ ⋃ { effects[g] : g ∈ calls[f] }, to a fixpoint.
        let mut eff: HashMap<LocalDefId, BTreeSet<&'static str>> = self.direct.clone();
        for f in self.calls.keys() {
            eff.entry(*f).or_default();
        }
        // Seed every function so over-declaration (declared-but-unused) is covered too.
        for owner in cx.tcx.hir_body_owners() {
            if matches!(cx.tcx.def_kind(owner.to_def_id()), DefKind::Fn | DefKind::AssocFn) {
                eff.entry(owner).or_default();
            }
        }

        let mut changed = true;
        while changed {
            changed = false;
            let callers: Vec<LocalDefId> = self.calls.keys().copied().collect();
            for f in callers {
                let mut add: BTreeSet<&'static str> = BTreeSet::new();
                if let Some(callees) = self.calls.get(&f) {
                    for g in callees {
                        if let Some(ge) = eff.get(g) {
                            add.extend(ge.iter().copied());
                        }
                    }
                }
                let entry = eff.entry(f).or_default();
                let before = entry.len();
                entry.extend(add);
                if entry.len() != before {
                    changed = true;
                }
            }
        }

        // Modes, selected by environment:
        //   (default)            audit — report each function's inferred effect set.
        //   CANDOR_JSON=<f>      write a machine-readable report; suppress warnings.
        //   CANDOR_STRICT=1|<p>  conformance: inferred ⊆ declared (whole crate or scoped to <p>).
        //   CANDOR_NO_AMBIENT=1|<p>  enforcement: flag any DIRECT use of ambient authority
        //                            (the cap-std-aligned answer to advisory tokens — route
        //                            those calls through an injected capability instead).
        //   CANDOR_BASELINE=<prefix> regression guard: flag any function that GAINED an effect
        //                            vs. a previously-saved report (a CANDOR_JSON snapshot).
        // Per-crate file naming, shared by JSON output, baseline input. A package emits
        // several crates SHARING a name (rlib + bin), so disambiguate by crate type too.
        let krate = cx.tcx.crate_name(rustc_hir::def_id::LOCAL_CRATE);
        let kinds: String = cx
            .tcx
            .crate_types()
            .iter()
            .map(|t| format!("{t:?}"))
            .collect::<Vec<_>>()
            .join("-");

        let strict_var = std::env::var("CANDOR_STRICT").ok();
        let no_ambient_var = std::env::var("CANDOR_NO_AMBIENT").ok();
        let json_path = std::env::var("CANDOR_JSON").ok();
        let baseline = std::env::var("CANDOR_BASELINE")
            .ok()
            .and_then(|prefix| load_baseline(&format!("{prefix}.{krate}.{kinds}.json")));
        let any_enforce =
            strict_var.is_some() || no_ambient_var.is_some() || baseline.is_some();

        // Stable ordering for reproducible output.
        let mut items: Vec<LocalDefId> = eff.keys().copied().collect();
        items.sort_by_cached_key(|f| cx.tcx.def_path_str(f.to_def_id()));

        let mut json_entries: Vec<String> = Vec::new();

        for f in items {
            let effs = &eff[&f];
            let name = cx.tcx.def_path_str(f.to_def_id());
            let span = cx.tcx.def_span(f);
            let declared = declared_caps(cx.tcx, f);
            let direct = self.direct.get(&f).cloned().unwrap_or_default();
            let has_unknown = effs.contains(UNKNOWN);
            // `Unknown` is not a declarable capability — it's handled by AS-EFF-003.
            let undeclared: Vec<&str> = effs
                .iter()
                .copied()
                .filter(|e| *e != UNKNOWN && !declared.contains(e))
                .collect();
            let unused: Vec<&str> =
                declared.iter().copied().filter(|c| !effs.contains(c)).collect();

            if json_path.is_some() {
                if effs.is_empty() && declared.is_empty() {
                    continue;
                }
                let loc = cx.tcx.sess.source_map().span_to_diagnostic_string(span);
                json_entries.push(format!(
                    "  {{\"fn\": {}, \"loc\": {}, \"inferred\": {}, \"direct\": {}, \
                     \"declared\": {}, \"undeclared\": {}, \"overdeclared\": {}, \
                     \"unresolved\": {}}}",
                    json_str(&name),
                    json_str(&loc),
                    json_set(effs),
                    json_set(&direct),
                    json_set(&declared),
                    json_vec(&undeclared),
                    json_vec(&unused),
                    has_unknown,
                ));
                continue;
            }

            // Audit mode (no enforcement env set): report the inferred set.
            if !any_enforce {
                if effs.is_empty() {
                    continue;
                }
                let parts: Vec<String> = effs
                    .iter()
                    .map(|e| {
                        if direct.contains(e) {
                            (*e).to_string()
                        } else {
                            format!("{e}*")
                        }
                    })
                    .collect();
                clippy_utils::diagnostics::span_lint(
                    cx,
                    CANDOR,
                    span,
                    format!("`{name}` effects: {{ {} }}   (* = via callee)", parts.join(", ")),
                );
                continue;
            }

            // AS-EFF-004 (CANDOR_NO_AMBIENT): this function reaches for ambient authority
            // directly. cap-std's lesson: don't — receive a capability and route through it.
            if in_scope(no_ambient_var.as_deref(), &name) {
                let ambient: Vec<&str> =
                    direct.iter().copied().filter(|e| AMBIENT.contains(e)).collect();
                if !ambient.is_empty() {
                    clippy_utils::diagnostics::span_lint(
                        cx,
                        CANDOR,
                        span,
                        format!(
                            "[AS-EFF-004] `{name}` uses ambient authority {{ {} }} directly; \
                             route it through an injected capability (e.g. cap-std) instead",
                            ambient.join(", ")
                        ),
                    );
                }
            }

            // Conformance (CANDOR_STRICT): inferred ⊆ declared.
            if in_scope(strict_var.as_deref(), &name) {
                // AS-EFF-001: performs an effect it does not declare.
                if !undeclared.is_empty() {
                    let have = if declared.is_empty() {
                        "no capabilities".to_string()
                    } else {
                        format!("only {{ {} }}", join(&declared))
                    };
                    clippy_utils::diagnostics::span_lint(
                        cx,
                        CANDOR,
                        span,
                        format!(
                            "[AS-EFF-001] `{name}` performs {{ {} }} but declares {have}; \
                             add the missing capability parameter(s)",
                            undeclared.join(", ")
                        ),
                    );
                }
                // AS-EFF-003: performs effects candor cannot resolve, so its declared
                // capabilities cannot be verified complete (the honest answer to dynamic
                // dispatch / fn-pointers / callbacks — never a silent pass).
                if has_unknown {
                    clippy_utils::diagnostics::span_lint(
                        cx,
                        CANDOR,
                        span,
                        format!(
                            "[AS-EFF-003] `{name}` makes calls candor cannot resolve \
                             (dynamic dispatch, fn-pointer, or callback); its effect set is \
                             not provably complete and conformance cannot be certified"
                        ),
                    );
                }
                // AS-EFF-002: declares a capability it never exercises.
                if !unused.is_empty() {
                    clippy_utils::diagnostics::span_lint(
                        cx,
                        CANDOR,
                        span,
                        format!(
                            "[AS-EFF-002] `{name}` declares {{ {} }} but never uses it; \
                             drop the capability parameter(s)",
                            unused.join(", ")
                        ),
                    );
                }
            }

            // AS-EFF-005 (CANDOR_BASELINE): a function gained an effect since the saved
            // report. New functions (absent from the baseline) are not flagged — they're
            // new code, reviewed normally; the guard is for *regressions* in existing fns.
            if let Some(base) = &baseline {
                if let Some(prior) = base.get(&name) {
                    let gained: Vec<&str> =
                        effs.iter().copied().filter(|e| !prior.contains(*e)).collect();
                    if !gained.is_empty() {
                        clippy_utils::diagnostics::span_lint(
                            cx,
                            CANDOR,
                            span,
                            format!(
                                "[AS-EFF-005] `{name}` gained effect {{ {} }} not present in the \
                                 baseline; an existing function started performing a new effect",
                                gained.join(", ")
                            ),
                        );
                    }
                }
            }
        }

        if let Some(prefix) = &json_path {
            let file = format!("{prefix}.{krate}.{kinds}.json");
            let body = format!("[\n{}\n]\n", json_entries.join(",\n"));
            if std::fs::write(&file, body).is_ok() {
                eprintln!("candor: wrote {} entries to {file}", json_entries.len());
            }
        }
    }
}

fn join(set: &BTreeSet<&str>) -> String {
    set.iter().copied().collect::<Vec<_>>().join(", ")
}

/// Is `name` covered by an enforcement scope env var? Unset → no; `1`/empty → whole
/// crate; otherwise a path prefix (incremental, one module at a time).
fn in_scope(var: Option<&str>, name: &str) -> bool {
    match var {
        None => false,
        Some("1") | Some("") => true,
        Some(prefix) => name.starts_with(prefix),
    }
}

fn json_str(s: &str) -> String {
    format!("\"{}\"", s.replace('\\', "\\\\").replace('"', "\\\""))
}

fn json_set(set: &BTreeSet<&str>) -> String {
    let inner: Vec<String> = set.iter().map(|s| json_str(s)).collect();
    format!("[{}]", inner.join(", "))
}

fn json_vec(v: &[&str]) -> String {
    let inner: Vec<String> = v.iter().map(|s| json_str(s)).collect();
    format!("[{}]", inner.join(", "))
}

#[test]
fn ui() {
    dylint_testing::ui_test(env!("CARGO_PKG_NAME"), "ui");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classify_network_precisely() {
        // AWS: only request dispatch, never the builder setters/accessors.
        assert_eq!(classify("aws_sdk_ec2", "aws_sdk_ec2::op::run::send"), Some("Net"));
        assert_eq!(classify("aws_sdk_ec2", "aws_sdk_ec2::op::run::instance_id"), None);
        // HTTP clients: dispatch only, not the builder chain (found by the eval).
        assert_eq!(classify("reqwest", "reqwest::RequestBuilder::send"), Some("Net"));
        assert_eq!(classify("reqwest", "reqwest::RequestBuilder::json"), None);
        assert_eq!(classify("reqwest", "reqwest::Client::execute"), Some("Net"));
        // Raw sockets are network — the regression guard against AWS-only detection.
        assert_eq!(classify("std", "std::net::TcpStream::connect"), Some("Net"));
        assert_eq!(classify("std", "std::net::UdpSocket::bind"), Some("Net"));
        assert_eq!(classify("tokio", "tokio::net::TcpStream::connect"), Some("Net"));
        // ...but the pure data types living alongside them must NOT be flagged.
        assert_eq!(classify("std", "std::net::SocketAddr::new"), None);
        assert_eq!(classify("std", "std::net::Ipv4Addr::new"), None);
    }

    #[test]
    fn classify_other_effects_precisely() {
        assert_eq!(classify("std", "std::fs::read_to_string"), Some("Fs"));
        assert_eq!(classify("tokio", "tokio::fs::read"), Some("Fs"));
        // Exec is subprocess spawning — not std::process::exit / Stdio / ExitStatus.
        assert_eq!(classify("std", "std::process::Command::new"), Some("Exec"));
        assert_eq!(classify("std", "std::process::Child::wait"), Some("Exec"));
        assert_eq!(classify("std", "std::process::exit"), None);
        assert_eq!(classify("std", "std::env::var"), Some("Env"));
        assert_eq!(classify("chrono", "chrono::Utc::now"), Some("Clock"));
        assert_eq!(classify("std", "std::time::SystemTime::now"), Some("Clock"));
        assert_eq!(classify("getrandom", "getrandom::getrandom"), Some("Rand"));
        assert_eq!(classify("tracing", "tracing::event"), Some("Log"));
        assert_eq!(classify("arboard", "arboard::Clipboard::set_text"), Some("Clipboard"));
        // Unrelated crates are pure.
        assert_eq!(classify("serde", "serde::Serialize::serialize"), None);
        assert_eq!(classify("std", "std::vec::Vec::push"), None);
    }

    #[test]
    fn config_parsing_and_extra_rules() {
        let rules = parse_config(
            "# a comment\n\nNet  crate  reqwest\nFs   path   mycrate::io::\nBogus crate x\n",
        );
        assert_eq!(rules.len(), 2); // the unknown-effect line is dropped
        assert_eq!(classify_extra("reqwest", "reqwest::Client::new", &rules), Some("Net"));
        assert_eq!(classify_extra("other", "mycrate::io::read", &rules), Some("Fs"));
        assert_eq!(classify_extra("other", "elsewhere::thing", &rules), None);
    }

    #[test]
    fn scope_matching() {
        assert!(!in_scope(None, "anything"));
        assert!(in_scope(Some("1"), "anything"));
        assert!(in_scope(Some(""), "anything"));
        assert!(in_scope(Some("app::config"), "app::config::load"));
        assert!(!in_scope(Some("app::config"), "app::other::load"));
    }

    #[test]
    fn baseline_round_trips() {
        let path = std::env::temp_dir().join("candor_unit_baseline.json");
        std::fs::write(
            &path,
            r#"[{"fn":"a::b","inferred":["Net","Log"]},{"fn":"c","inferred":[]}]"#,
        )
        .unwrap();
        let m = load_baseline(path.to_str().unwrap()).unwrap();
        assert_eq!(m.len(), 2);
        assert!(m["a::b"].contains("Net") && m["a::b"].contains("Log"));
        assert!(m["c"].is_empty());
        // A missing or unreadable baseline yields None (never panics).
        assert!(load_baseline("/no/such/candor/baseline.json").is_none());
    }

    #[test]
    fn cap_names_match_effects() {
        assert_eq!(cap_from_name("Net"), Some("Net"));
        assert_eq!(cap_from_name("Rand"), Some("Rand"));
        assert_eq!(cap_from_name("Nonsense"), None);
        // Every ambient authority must be a known effect name.
        for a in AMBIENT {
            assert!(cap_from_name(a).is_some(), "ambient {a} not in EFFECTS");
        }
    }
}
