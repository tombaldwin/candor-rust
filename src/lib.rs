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
}

impl Candor {
    pub fn new() -> Self {
        let extra = std::env::var("CANDOR_CONFIG")
            .ok()
            .and_then(|p| std::fs::read_to_string(p).ok())
            .map(|s| parse_config(&s))
            .unwrap_or_default();
        Self { direct: HashMap::new(), calls: HashMap::new(), extra }
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
    match expr.kind {
        ExprKind::MethodCall(_, receiver, _, _) => {
            let recv_ty = cx.typeck_results().expr_ty_adjusted(receiver).peel_refs();
            if matches!(recv_ty.kind(), TyKind::Dynamic(..)) {
                return Some(Callee::Unresolved); // dyn dispatch
            }
            match cx.typeck_results().type_dependent_def_id(expr.hir_id) {
                Some(did) => Some(Callee::Def(did)),
                None => Some(Callee::Unresolved),
            }
        }
        ExprKind::Call(callee, _) => match cx.typeck_results().expr_ty(callee).kind() {
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
    if path.starts_with("std::fs::") || path.starts_with("tokio::fs::") {
        return Some("Fs");
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

const EFFECTS: [&str; 7] = ["Net", "Fs", "Exec", "Env", "Clock", "Log", "Clipboard"];

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

        // Three modes, selected by environment:
        //   (default)               audit — report each function's inferred effect set.
        //   CANDOR_STRICT=1   conformance over the whole crate (inferred ⊆ declared).
        //   CANDOR_STRICT=<p> conformance scoped to functions whose path starts <p>
        //                           (incremental adoption — check one module at a time).
        //   CANDOR_JSON=<f>   write a machine-readable report to <f>, suppress warnings.
        let strict_var = std::env::var("CANDOR_STRICT").ok();
        let json_path = std::env::var("CANDOR_JSON").ok();
        let in_scope = |name: &str| match strict_var.as_deref() {
            None => false,
            Some("1") | Some("") => true,
            Some(prefix) => name.starts_with(prefix),
        };

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

            if strict_var.is_some() {
                if !in_scope(&name) {
                    continue;
                }
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
                // capabilities cannot be verified complete (this is the honest answer to
                // dynamic dispatch / fn-pointers / callbacks — never a silent pass).
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
            } else {
                // Audit mode: report the inferred set, marking inherited effects with `*`.
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
            }
        }

        if let Some(prefix) = &json_path {
            // One file per compiled crate. A single package emits several crates that
            // SHARE a crate name (e.g. the `ebman` rlib from lib.rs and the `ebman` bin
            // from main.rs), so we disambiguate by crate type too, else they overwrite.
            let krate = cx.tcx.crate_name(rustc_hir::def_id::LOCAL_CRATE);
            let kinds: String = cx
                .tcx
                .crate_types()
                .iter()
                .map(|t| format!("{t:?}"))
                .collect::<Vec<_>>()
                .join("-");
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
