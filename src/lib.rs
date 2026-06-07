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

extern crate rustc_ast;
extern crate rustc_errors;
extern crate rustc_hir;
extern crate rustc_middle;

use std::collections::{BTreeSet, HashMap, HashSet};

use candor_report::{
    report_entries, report_files, report_version, to_report_json, ReportEntry, ReportMeta, EFFECTS,
};

use rustc_hir::def::DefKind;
use rustc_hir::def_id::{DefId, LocalDefId};
use rustc_hir::{Expr, ExprKind, HirId};
use rustc_lint::{LateContext, LateLintPass};
use rustc_middle::ty::TyCtxt;

dylint_linting::impl_late_lint! {
    /// ### What it does
    /// Reports, for each function, the transitive set of capabilities/effects it
    /// exercises — Net, Fs, Exec, Env, Clock, Log, Clipboard, Rand, Db, Ipc, plus
    /// `Unknown` for calls it can't resolve — by classifying callee `DefId`s and
    /// propagating along local calls.
    ///
    /// ### Why is this bad?
    /// It isn't — it's an audit. It makes a function's effect surface legible from its
    /// definition, the way an explicit effect annotation would.
    pub CANDOR,
    Warn,
    "aggregates each function's transitive capability/effect set",
    Candor::new()
}

/// Emit a candor lint diagnostic at `sp` with message `msg`.
///
/// Vendored from the single `clippy_utils::diagnostics::span_lint` helper candor used to depend on —
/// minus clippy's docs-page hyperlink (candor is not a clippy lint, so there's no page to link). It's
/// a thin wrapper over rustc's own `LintContext::emit_span_lint`; replicating these few lines lets
/// candor drop its ONLY git-only dependency (`clippy_utils`), so the crate can be published to
/// crates.io (which forbids git deps, not `rustc_private`). Mirrors the upstream shape exactly so it
/// compiles against the same pinned nightly.
fn span_lint(
    cx: &impl rustc_lint::LintContext,
    lint: &'static rustc_lint::Lint,
    sp: impl Into<rustc_errors::MultiSpan>,
    msg: impl Into<rustc_errors::DiagMessage>,
) {
    use rustc_errors::{Diag, DiagCtxtHandle, Diagnostic, Level};
    struct CandorDiag<F: FnOnce(&mut Diag<'_, ()>)>(F);
    impl<'a, F: FnOnce(&mut Diag<'_, ()>)> Diagnostic<'a, ()> for CandorDiag<F> {
        fn into_diag(self, dcx: DiagCtxtHandle<'a>, level: Level) -> Diag<'a, ()> {
            let mut diag = Diag::new(dcx, level, "");
            (self.0)(&mut diag);
            diag
        }
    }
    let sp = sp.into();
    cx.emit_span_lint(
        lint,
        sp.clone(),
        CandorDiag(move |diag: &mut Diag<'_, ()>| {
            diag.primary_message(msg);
            diag.span(sp);
        }),
    );
}

/// The effect recorded for a call candor cannot resolve to a concrete callee.
const UNKNOWN: &str = "Unknown";

/// Sentinel in the `fs` read/write set marking that a function inherits `Fs` with an UNKNOWN kind
/// (e.g. across a crate boundary, where the dependency's report records no read/write detail). When
/// present we emit no `fs` detail at all — a missing claim is honest; a partial one would mislead.
const FS_UNKNOWN: &str = "?";

/// Transitive fixpoint over the local call graph: `acc[f] = seed[f] ∪ ⋃ { acc[g] : g ∈ calls[f] }`.
/// Used for the effect set, the Fs read/write detail, AND the literal Net host detail (all ride the
/// same graph). Generic over the element so it serves both `&'static str` (effects/fs) and owned
/// `String` (hosts). Takes the pre-seeded map by value and grows it to convergence (the sets only
/// grow, bounded by a finite alphabet — for hosts, the finite set of literals in the crate — so it
/// terminates).
fn propagate<T: Ord + Clone>(
    mut acc: HashMap<LocalDefId, BTreeSet<T>>,
    calls: &HashMap<LocalDefId, HashSet<LocalDefId>>,
) -> HashMap<LocalDefId, BTreeSet<T>> {
    let mut changed = true;
    while changed {
        changed = false;
        let callers: Vec<LocalDefId> = calls.keys().copied().collect();
        for f in callers {
            let mut add: BTreeSet<T> = BTreeSet::new();
            if let Some(callees) = calls.get(&f) {
                for g in callees {
                    if let Some(gk) = acc.get(g) {
                        add.extend(gk.iter().cloned());
                    }
                }
            }
            if add.is_empty() {
                continue;
            }
            let entry = acc.entry(f).or_default();
            let before = entry.len();
            entry.extend(add);
            if entry.len() != before {
                changed = true;
            }
        }
    }
    acc
}

pub struct Candor {
    /// Effects performed directly in a function's own body (and its inline closures).
    direct: HashMap<LocalDefId, BTreeSet<&'static str>>,
    /// Filesystem access *kind* ("read"/"write") performed directly, when a directly-classified `Fs`
    /// call's verb tells us which (e.g. `fs::write` → write, `File::open` → read). Propagated through
    /// the call graph like effects and surfaced as the report's optional `fs` detail — a NON-breaking
    /// refinement of `Fs` (the effect itself is unchanged, so no baseline regresses).
    fs_direct: HashMap<LocalDefId, BTreeSet<&'static str>>,
    /// Literal Net endpoints performed directly: when a directly-classified `Net` call carries a
    /// string-LITERAL address/URL (`TcpStream::connect("rates.internal:7070")`,
    /// `reqwest::get("https://api.example.com/x")`), the host part. Propagated like `fs_direct` and
    /// surfaced as the report's optional `hosts` detail — a non-breaking refinement of `Net`. Only the
    /// *statically visible* (literal) subset; a runtime-computed address is simply absent (the host is
    /// undecidable in general, so this is honest best-effort, never a completeness claim).
    net_hosts_direct: HashMap<LocalDefId, BTreeSet<String>>,
    /// Local-crate functions each function calls, for transitive propagation.
    calls: HashMap<LocalDefId, HashSet<LocalDefId>>,
    /// Project-supplied classifier rules: (effect, is_crate_prefix, prefix).
    extra: Vec<(&'static str, bool, String)>,
    /// CANDOR_PARANOID: also treat generic static trait dispatch as Unknown.
    paranoid: bool,
    /// External (non-std, non-local) crates we actually saw resolved calls into.
    /// Ground truth for the coverage blind-spot check — emitted beside the report.
    encountered: BTreeSet<String>,
    /// Cross-crate effect oracle: a map from a function's stable `DefPathHash` (as a `(u64, u64)`
    /// `(StableCrateId, local-hash)` pair) to its already-transitive effect set, loaded from this
    /// project's OTHER crates' reports. Lets a call into the project's own lib (from its bin) or a
    /// sibling workspace member inherit the callee's effects — closing the within-crate-only
    /// propagation hole (CRITIQUE §8). Keyed by `DefPathHash` because it's stable across crates
    /// (unlike `def_path_str`, which reexport-shortens external defs); the structured pair is a
    /// zero-alloc `Copy` key, vs. the old `format!("{:?}", …)` which allocated on every call.
    cross: HashMap<(u64, u64), Vec<&'static str>>,
    /// Effects a function inherits via a cross-crate call. Kept separate from `direct` (so the
    /// report's `direct` stays "own body") and folded into the fixpoint in check_crate_post.
    via_cross: HashMap<LocalDefId, BTreeSet<&'static str>>,
    /// CANDOR_EXPLAIN=<query>: when set, record where each effect enters (the call + location) so
    /// `cargo candor explain` can trace the path from a function to the source of each effect.
    explain: Option<String>,
    /// Per-function effect *sites*: the calls in a body that introduce an effect (a classified leaf,
    /// a cross-crate inheritance, or an unresolvable call). Populated only in explain mode.
    sites: HashMap<LocalDefId, Vec<EffectSite>>,
    /// CANDOR_POLICY: declared effect-boundary rules to enforce (AS-EFF-006).
    policy: Vec<PolicyRule>,
    /// CANDOR_TAINT: flag effects whose argument derives from a function parameter (AS-EFF-007).
    taint: bool,
    /// Per-function effects performed on a parameter-derived (caller-controlled) argument.
    tainted: HashMap<LocalDefId, BTreeSet<&'static str>>,
}

/// Where an effect enters a function's body — the callee that produced it and the source location.
struct EffectSite {
    eff: &'static str,
    via: String,
    loc: String,
}

/// Effects that represent *ambient authority* — a global resource reachable just by
/// naming it (vs. a capability you must be handed). These are what `CANDOR_NO_AMBIENT`
/// and cap-std care about. `Log` is intentionally excluded (not an authority).
const AMBIENT: [&str; 9] =
    ["Net", "Fs", "Exec", "Env", "Clock", "Clipboard", "Rand", "Db", "Ipc"];

/// The engine's build identity, stamped by build.rs. `CANDOR_VERSION` is the source commit the
/// dylib was built from (not the source tree's current HEAD — see build.rs). Emitted into every
/// report's sidecar so a report is self-describing, and embedded verbatim in `CANDOR_BUILD_TAG`
/// so `cargo-candor` can read the *true* engine version with `strings`, without running it.
const CANDOR_VERSION: &str = env!("CANDOR_VERSION");
const CANDOR_TOOLCHAIN: &str = env!("CANDOR_TOOLCHAIN");

/// A contiguous ASCII tag retained in the dylib (`#[used]` blocks dead-strip) so a build tool can
/// recover the engine's true build version without loading or running it:
///   strings -a libcandor@*.dylib | grep candor-build-version=
#[used]
static CANDOR_BUILD_TAG: &str = concat!("candor-build-version=", env!("CANDOR_VERSION"));

impl Candor {
    pub fn new() -> Self {
        // A *set-but-unreadable* CANDOR_CONFIG must be loud: silently ignoring it would
        // make the user believe their crates are covered when they aren't.
        let extra = match std::env::var("CANDOR_CONFIG") {
            Ok(p) => match std::fs::read_to_string(&p) {
                Ok(s) => parse_config(&s),
                Err(e) => {
                    eprintln!("candor: CANDOR_CONFIG={p:?} could not be read ({e}); ignoring it");
                    Vec::new()
                }
            },
            Err(_) => Vec::new(),
        };
        let paranoid = std::env::var("CANDOR_PARANOID").is_ok();
        let explain = std::env::var("CANDOR_EXPLAIN").ok().filter(|s| !s.is_empty());
        // A set-but-unreadable CANDOR_POLICY must be loud — silently passing would let a violation
        // through while the user believes the boundary is enforced.
        let policy = match std::env::var("CANDOR_POLICY") {
            Ok(p) => match std::fs::read_to_string(&p) {
                Ok(s) => parse_policy(&s),
                Err(e) => {
                    eprintln!("candor: CANDOR_POLICY={p:?} could not be read ({e}); policy NOT enforced");
                    Vec::new()
                }
            },
            Err(_) => Vec::new(),
        };
        Self {
            direct: HashMap::new(),
            fs_direct: HashMap::new(),
            net_hosts_direct: HashMap::new(),
            calls: HashMap::new(),
            extra,
            paranoid,
            encountered: BTreeSet::new(),
            cross: HashMap::new(),
            via_cross: HashMap::new(), // (cross map keyed by structured DefPathHash, not a string)
            explain,
            sites: HashMap::new(),
            policy,
            taint: std::env::var("CANDOR_TAINT").is_ok(),
            tainted: HashMap::new(),
        }
    }

    /// BFS the call graph from `start` to the nearest function whose body directly produces
    /// `effect` (a classified leaf, a cross-crate call, or an unresolvable call) — the shortest
    /// path explaining *why* `start` has `effect`. Used by `CANDOR_EXPLAIN`.
    fn find_source(&self, start: LocalDefId, effect: &str) -> Option<Vec<LocalDefId>> {
        use std::collections::VecDeque;
        let mut queue = VecDeque::from([start]);
        let mut seen: HashSet<LocalDefId> = HashSet::from([start]);
        let mut prev: HashMap<LocalDefId, LocalDefId> = HashMap::new();
        while let Some(n) = queue.pop_front() {
            if self.sites.get(&n).is_some_and(|v| v.iter().any(|s| s.eff == effect)) {
                let mut path = vec![n];
                let mut cur = n;
                while cur != start {
                    cur = prev[&cur];
                    path.push(cur);
                }
                path.reverse();
                return Some(path);
            }
            if let Some(callees) = self.calls.get(&n) {
                for &c in callees {
                    if seen.insert(c) {
                        prev.insert(c, n);
                        queue.push_back(c);
                    }
                }
            }
        }
        None
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

/// One declared effect-boundary rule (`CANDOR_POLICY`). `effects` empty ⇒ a `pure` rule (ANY effect
/// is forbidden). `scope` is a path substring the rule applies to (None = the whole crate). Checked
/// against a function's *transitive* (inferred) effects — so "domain must not do Net" catches domain
/// code that reaches the network through a helper, the boundary violation an agent can't see.
struct PolicyRule {
    effects: BTreeSet<&'static str>,
    scope: Option<String>,
    raw: String,
}

/// Parse a `CANDOR_POLICY` file. One rule per line; `#` comments and blanks ignored:
///
///     deny Net Db  domain     # functions whose path contains "domain" must not perform Net or Db
///     deny Exec               # no function anywhere may perform Exec
///     pure         parse      # functions whose path contains "parse" must be effect-free
///
/// In a `deny` rule, leading tokens that name a known effect are forbidden; the first non-effect
/// token (if any) is the scope. `pure <scope>` forbids all effects in scope.
fn parse_policy(text: &str) -> Vec<PolicyRule> {
    let mut rules = Vec::new();
    for raw_line in text.lines() {
        let line = raw_line.split('#').next().unwrap_or("").trim();
        if line.is_empty() {
            continue;
        }
        let mut toks = line.split_whitespace();
        match toks.next().unwrap_or("") {
            "deny" => {
                let mut effects = BTreeSet::new();
                let mut scope = None;
                for t in toks {
                    let e = if t == UNKNOWN { Some(UNKNOWN) } else { cap_from_name(t) };
                    match e {
                        Some(e) => {
                            effects.insert(e);
                        }
                        None => {
                            scope = Some(t.to_string());
                            break;
                        }
                    }
                }
                if effects.is_empty() {
                    eprintln!("candor: ignoring policy rule (no known effect named): {line}");
                    continue;
                }
                rules.push(PolicyRule { effects, scope, raw: line.to_string() });
            }
            "pure" => rules.push(PolicyRule {
                effects: BTreeSet::new(),
                scope: toks.next().map(str::to_string),
                raw: line.to_string(),
            }),
            other => eprintln!("candor: ignoring policy rule (unknown kind `{other}`): {line}"),
        }
    }
    rules
}

/// A function's stable cross-crate identity: the `DefPathHash` as a `(StableCrateId, local-hash)`
/// pair of `u64`s. This value is identical whether the def is viewed from its home crate (a
/// `LocalDefId`) or from a dependent (an external `DefId`) — unlike `def_path_str`, which
/// reexport-shortens external paths. `Copy`, so it's a zero-alloc map key on the hot path.
fn dph(tcx: TyCtxt<'_>, did: DefId) -> (u64, u64) {
    let h = tcx.def_path_hash(did);
    (h.stable_crate_id().as_u64(), h.local_hash().as_u64())
}

/// The on-disk string form of `dph`, written to a report entry's `hash` field. Hex of the two
/// `u64`s — a stable representation (unlike `DefPathHash`'s internal `Debug`).
fn dph_hex(tcx: TyCtxt<'_>, did: DefId) -> String {
    let (a, b) = dph(tcx, did);
    format!("{a:016x}{b:016x}")
}

/// Parse the 32-hex `hash` field back into the structured key. None for malformed/absent hashes.
fn parse_dph(s: &str) -> Option<(u64, u64)> {
    if s.len() != 32 {
        return None;
    }
    Some((u64::from_str_radix(&s[..16], 16).ok()?, u64::from_str_radix(&s[16..], 16).ok()?))
}

/// Load the per-crate reports of this project's OTHER crates (`<prefix>.<crate>.<type>.json`, all
/// but our own `<me>.<me_kind>` — a package's lib and bin share the crate name but differ by type,
/// so the bin must still load the lib's report) into a `DefPathHash -> effects` map for
/// cross-crate resolution. Each function's *inferred* (already-transitive) set is what a caller in
/// another crate inherits. Skips `<prefix>.calibrated.json` / `encountered-*` sidecars (one segment).
///
/// `trust_siblings`: when false (live analysis, `CANDOR_JSON`), a sibling produced by a DIFFERENT
/// engine is downgraded to `Unknown` (candor-spec §2.1 — its effects were computed by rules this
/// engine may have changed). When true (the guard, `CANDOR_BASELINE`), the "siblings" ARE the
/// baseline's own snapshot of the project's crates — intentionally from the baseline commit — so we
/// trust them as-is; downgrading them would change the very cross-inclusive set the guard exists to
/// reproduce, firing a spurious AS-EFF-005 every time the engine moves ahead of the baseline.
fn load_cross_reports(
    prefix: &str,
    me: &str,
    me_kind: &str,
    trust_siblings: bool,
) -> HashMap<(u64, u64), Vec<&'static str>> {
    let mut out: HashMap<(u64, u64), Vec<&'static str>> = HashMap::new();
    for rf in report_files(prefix) {
        // Skip our OWN report (by crate name AND type); DefPathHash keys are globally unique so
        // all other crates merge into one map. (Own entries are local defs and the cross path is
        // guarded by `!def_id.is_local()`, so loading them would be harmless — just wasteful.)
        if rf.krate == me && rf.kind == me_kind {
            continue;
        }
        let Ok(text) = std::fs::read_to_string(&rf.path) else { continue };
        // Version-aware trust (candor-spec §2.1): a sibling report produced by a DIFFERENT engine
        // was computed by rules this engine may have changed, so we must not silently trust its
        // effects — downgrade everything inherited from it to `Unknown`. (A legacy v0.1 report has no
        // version; we can't check it, so it's trusted as before.)
        let stale = !trust_siblings && report_version(&text).is_some_and(|v| v != CANDOR_VERSION);
        let Some(arr) = report_entries(&text) else { continue };
        for e in arr {
            let Some(key) = parse_dph(&e.hash) else { continue };
            let effs: Vec<&'static str> = if stale {
                vec![UNKNOWN]
            } else {
                e.inferred
                    .iter()
                    .filter_map(|s| if s.as_str() == UNKNOWN { Some(UNKNOWN) } else { cap_from_name(s.as_str()) })
                    .collect()
            };
            if !effs.is_empty() {
                out.insert(key, effs);
            }
        }
    }
    out
}


/// Load a baseline candor JSON into `fn name -> inferred effect set`.
fn load_baseline(path: &str) -> Option<HashMap<String, BTreeSet<String>>> {
    let text = std::fs::read_to_string(path).ok()?;
    let entries = report_entries(&text)?;
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
    /// A resolved callee `DefId`. `dynamic` = reached via `dyn Trait` dispatch (the
    /// `DefId` is then the *trait* method; CHA finds the concrete impls).
    Def { did: DefId, dynamic: bool },
    /// A call candor cannot see through at all (fn pointer, `impl Fn` callback).
    Unresolved,
}

/// Classify a call site. We still resolve `dyn Trait` method calls to the trait method
/// `DefId` (flagged `dynamic`) so CHA can enumerate impls; only genuinely opaque calls
/// (fn pointers, closures through generic params) are `Unresolved`.
fn resolve_callee<'tcx>(cx: &LateContext<'tcx>, expr: &Expr<'tcx>) -> Option<Callee> {
    use rustc_middle::ty::TyKind;
    // Defensive: `typeck_results()` panics (ICE) for an expr outside a typechecked
    // body. An effect checker must never abort the build, so bail gracefully instead.
    let typeck = cx.maybe_typeck_results()?;
    match expr.kind {
        ExprKind::MethodCall(_, receiver, _, _) => {
            let recv_ty = typeck.expr_ty_adjusted(receiver).peel_refs();
            let dynamic = matches!(recv_ty.kind(), TyKind::Dynamic(..));
            match typeck.type_dependent_def_id(expr.hir_id) {
                Some(did) => Some(Callee::Def { did, dynamic }),
                None => Some(Callee::Unresolved),
            }
        }
        ExprKind::Call(callee, _) => match typeck.expr_ty(callee).kind() {
            TyKind::FnDef(did, _) => Some(Callee::Def { did: *did, dynamic: false }),
            TyKind::FnPtr(..) => Some(Callee::Unresolved), // function pointer
            TyKind::Closure(..) => None,                   // inline closure body counted lexically
            TyKind::Param(..) | TyKind::Alias(..) | TyKind::Dynamic(..) => Some(Callee::Unresolved),
            _ => None,
        },
        _ => None,
    }
}

/// Class Hierarchy Analysis: the impl methods a (trait) call could dispatch to. Scoped
/// to traits defined in THIS crate — we can enumerate their impls and see the bodies,
/// and they're the project's own effectful traits. For non-local traits (std/deps) we
/// can't see impl bodies, so the caller keeps an honest `Unknown` for `dyn` over them.
fn cha_targets(tcx: TyCtxt<'_>, method_did: DefId) -> Vec<DefId> {
    let Some(trait_did) = tcx.trait_of_assoc(method_did) else {
        return Vec::new();
    };
    if !trait_did.is_local() {
        return Vec::new();
    }
    let impls = tcx.trait_impls_of(trait_did);
    let impl_dids = impls
        .non_blanket_impls()
        .values()
        .flatten()
        .copied()
        .chain(impls.blanket_impls().iter().copied());
    let mut out = Vec::new();
    for impl_did in impl_dids {
        if let Some(&impl_method) = tcx.impl_item_implementor_ids(impl_did).get(&method_did) {
            out.push(impl_method);
        }
    }
    out
}

/// Resolve a (non-`dyn`) trait-method call to the single concrete impl it dispatches to, when
/// the receiver type is known — so candor can use the ONE real target instead of CHA-expanding
/// to every impl (the over-approximation that yields confident false positives, CRITIQUE §9).
/// Returns None for `dyn`/generic receivers that can't be pinned down here, so the caller falls
/// back to CHA. Only method calls carry the receiver substs on the call expr.
fn devirtualize<'tcx>(cx: &LateContext<'tcx>, expr: &Expr<'tcx>, method_did: DefId) -> Option<DefId> {
    if !matches!(expr.kind, ExprKind::MethodCall(..)) {
        return None;
    }
    // `Instance::try_resolve` asserts the def is a Fn/AssocFn/Const; method calls are always
    // AssocFn today, but guard explicitly so an unexpected DefKind can never ICE the build (an
    // effect checker must degrade to Unknown, never abort compilation).
    if !matches!(cx.tcx.def_kind(method_did), DefKind::Fn | DefKind::AssocFn) {
        return None;
    }
    let typeck = cx.maybe_typeck_results()?;
    let args = typeck.node_args(expr.hir_id);
    let instance =
        rustc_middle::ty::Instance::try_resolve(cx.tcx, cx.typing_env(), method_did, args)
            .ok()
            .flatten()?;
    Some(instance.def_id())
}

/// The exact third-party crates `classify` has effect rules for, and the crate-name
/// PREFIXES it recognizes. This is the single source of truth for "what candor knows":
/// it is emitted beside the JSON report (`<prefix>.calibrated.json`) so the Claude Code
/// receipt's coverage check reads candor's real coverage instead of a hand-copied list.
/// Keep in lockstep with `classify` below — the `calibrated_set_covers_classifier` test
/// enforces that every named crate the classifier matches appears here.
const CALIBRATED_CRATES: [&str; 44] = [
    // network (aws_config resolves credentials over the network on `.load()`;
    // git2 remote ops — fetch/push/connect — contact the network; async_net is smol's net layer)
    "reqwest", "isahc", "ureq", "aws_config", "git2", "tokio_tcp", "tokio_udp", "async_net",
    "async_nats", "lapin", "lettre", "tungstenite", "elasticsearch", "tonic", "rdkafka",
    // database (see DB_CRATES in classify)
    "sqlx", "rusqlite", "postgres", "tokio_postgres", "diesel", "redis", "mongodb",
    "mysql", "mysql_async", "sea_orm", "deadpool_postgres",
    // filesystem (async_fs = smol; fs_err = std::fs wrapper; tempfile; glob) / entropy /
    // subprocess (async_process = smol; duct) / env (dotenvy/dotenv) / clock (time) / log / clipboard
    "memmap2", "fs_err", "async_fs", "tempfile", "glob",
    "rand", "getrandom", "fastrand",
    "portable_pty", "async_process", "duct",
    "dotenvy", "dotenv",
    "chrono", "time", "tracing", "log", "arboard",
];
const CALIBRATED_PREFIXES: [&str; 3] = ["aws_sdk_", "aws_smithy", "cap_"];

/// Crates `classify` matches by PATH prefix rather than crate-name equality (their effectful modules
/// are recognised, e.g. `tokio::net::`/`async_std::fs::`/`mio::net::`), so they're absent from
/// `CALIBRATED_CRATES` (which the liveness test probes by crate name). The coverage check must still
/// treat them as *covered* — otherwise it would mislabel the most common async crates as blind spots.
const PATH_CALIBRATED_CRATES: [&str; 3] = ["tokio", "async_std", "mio"];

/// Database client crates whose execution verbs are I/O (see the DB branch in `classify`).
/// Module-level so `db_crates_are_calibrated` can enforce `DB_CRATES ⊆ CALIBRATED_CRATES`.
const DB_CRATES: [&str; 11] = [
    "sqlx", "rusqlite", "postgres", "tokio_postgres", "diesel", "redis", "mongodb",
    "mysql", "mysql_async", "sea_orm", "deadpool_postgres",
];

/// For a call already classified as `Fs`, the access *kind* its leaf verb implies: `["read"]`,
/// `["write"]`, `["read","write"]` (e.g. `fs::copy`), or `&[]` when the verb doesn't say (so we make
/// no claim). Keyed off the std::fs / `File` / `OpenOptions` verb vocabulary — a syntactic refinement
/// of an effect candor already proved, NOT a soundness claim. `OpenOptions::open`'s direction is set
/// by runtime flags, so it's deliberately left unannotated.
fn fs_kind(path: &str) -> &'static [&'static str] {
    if path.contains("OpenOptions") {
        return &[];
    }
    // The leaf method/function name (strip any trailing generics / parens).
    let leaf = path.rsplit("::").next().unwrap_or(path);
    let leaf = leaf.split('<').next().unwrap_or(leaf).trim_matches(|c| c == '(' || c == ')');
    const WRITE: [&str; 26] = [
        "write", "write_all", "write_at", "write_vectored", "write_fmt", "create", "create_new",
        "create_dir", "create_dir_all", "remove_file", "remove_dir", "remove_dir_all", "rename",
        "set_permissions", "set_len", "set_modified", "set_times", "hard_link", "soft_link",
        "symlink", "symlink_file", "symlink_dir", "truncate", "append", "sync_all", "sync_data",
    ];
    const READ: [&str; 16] = [
        "read", "read_to_string", "read_to_end", "read_at", "read_dir", "read_link", "read_exact",
        "read_vectored", "metadata", "symlink_metadata", "open", "canonicalize", "try_exists",
        "exists", "file_type", "read_to",
    ];
    if leaf == "copy" {
        return &["read", "write"];
    }
    if WRITE.contains(&leaf) {
        return &["write"];
    }
    if READ.contains(&leaf) {
        return &["read"];
    }
    &[]
}

/// The fs read/write detail to record for a call classified `Fs` — but ONLY when that classification
/// came from the BUILT-IN classifier (`builtin == Some("Fs")`), whose paths are real std::fs /
/// known-fs-crate verbs `fs_kind`'s table understands. A user `extra` rule can label any crate `Fs`,
/// and its method names needn't be std::fs verbs (an in-memory `Builder::append()` would otherwise
/// mis-tag as `Fs(write)`), so we make no read/write claim for those. `&[]` for any non-Fs effect too.
fn fs_detail_for(builtin: Option<&'static str>, path: &str) -> &'static [&'static str] {
    if builtin == Some("Fs") { fs_kind(path) } else { &[] }
}

/// The host part of a string literal that looks like a network endpoint — the statically-visible
/// subset of "who does this Net call talk to". Strips the scheme and path, leaving `host[:port]`:
/// `"https://api.example.com/v1"` → `api.example.com`, `"rates.internal:7070"` → `rates.internal:7070`.
/// Returns `None` for a literal that doesn't look host-ish (no dot or colon, or has whitespace) so a
/// non-address string argument (a header value, an HTTP verb) isn't mistaken for a host. Full
/// host-by-runtime-value is undecidable — this only ever sees literals, and says so.
fn net_host_literal(s: &str) -> Option<String> {
    let s = s.trim();
    // authority = after `://` (if a URL), up to the first `/` (drop any path)
    let authority = s.split_once("://").map(|(_, rest)| rest).unwrap_or(s);
    let authority = authority.split('/').next().unwrap_or(authority);
    // drop userinfo (`user:pass@host`) if present
    let authority = authority.rsplit_once('@').map(|(_, h)| h).unwrap_or(authority);
    if authority.is_empty() || authority.contains(char::is_whitespace) {
        return None;
    }
    // must look like a host: a dotted name / IP, or a `host:port`
    if authority.contains('.') || authority.contains(':') {
        Some(authority.to_string())
    } else {
        None
    }
}

/// Extract literal Net hosts from a call expr's arguments (not the receiver — that's the client/socket,
/// not the address). Scans string-literal args through `net_host_literal`. Called only for a call
/// already classified `Net`, so the literal we find is the endpoint, not an unrelated string.
fn net_hosts_in_call(expr: &Expr<'_>) -> BTreeSet<String> {
    use rustc_ast::LitKind;
    let args: &[Expr<'_>] = match expr.kind {
        ExprKind::Call(_, args) => args,
        ExprKind::MethodCall(_, _, args, _) => args,
        _ => return BTreeSet::new(),
    };
    let mut out = BTreeSet::new();
    for a in args {
        if let ExprKind::Lit(lit) = &a.kind {
            if let LitKind::Str(sym, _) = lit.node {
                if let Some(host) = net_host_literal(sym.as_str()) {
                    out.insert(host);
                }
            }
        }
    }
    out
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
    // aws-config resolves credentials/region on `.load()` — it reaches the IMDS metadata
    // endpoint / STS over the network (and reads ~/.aws + env). Builders (`defaults()`,
    // `SdkConfig::builder()`, `BehaviorVersion::latest()`) are pure; the `load` is the I/O.
    // (Found hardening on a real app, ebman: `builder.load().await` was classified pure.)
    if crate_name == "aws_config" {
        if path.ends_with("::load") || path.ends_with("::load_defaults") {
            return Some("Net");
        }
        return None;
    }
    // git2 (libgit2 FFI): remote operations contact the network; everything else is local
    // to the .git directory. Match the remote verbs precisely — NOT bare `::clone`, which is
    // the `Clone`-trait dup of a `Remote` handle (pure), not `Repository::clone`. (Found
    // hardening on gitui: `remote.fetch`/`remote.push` were classified network-free — a git
    // client reporting it makes no network calls.)
    if crate_name == "git2" {
        if path.ends_with("::fetch")
            || path.ends_with("::push")
            || path.ends_with("::download")
            || path.ends_with("::connect")
            || path.ends_with("::connect_auth")
            || path.ends_with("::ls")
            || path.ends_with("::upload")
        {
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
    // Message-queue clients fully encapsulate the socket (the underlying tokio::net lives
    // inside the crate, unseen), so a user's connect/publish/consume calls ARE the I/O
    // boundary — to a remote broker, hence Net. Match the broker round-trip verbs (snake_case
    // methods); the CamelCase option/property builders stay pure. (Found hardening on consumer
    // apps: lapin `basic_publish`/`queue_declare` and async-nats `publish`/`subscribe` were
    // classified pure — a message-queue client reporting no I/O.)
    if crate_name == "async_nats" {
        if path.ends_with("::connect")
            || path.contains("::publish")
            || path.ends_with("::subscribe")
            || path.ends_with("::queue_subscribe")
            || path.contains("::request")
            || path.ends_with("::flush")
        {
            return Some("Net");
        }
        return None;
    }
    if crate_name == "lapin" {
        if path.ends_with("::connect")
            || path.ends_with("::create_channel")
            || path.contains("::basic_")
            || path.contains("::queue_")
            || path.contains("::exchange_")
            || path.contains("::tx_")
            || path.ends_with("::confirm_select")
            || path.ends_with("::close")
        {
            return Some("Net");
        }
        return None;
    }
    // SMTP email — lettre's `Transport::send` is the network dispatch; Message building is
    // pure. (Found hardening on a lettre consumer: `mailer.send(&email)` classified pure.)
    if crate_name == "lettre" {
        if path.ends_with("::send") || path.ends_with("::send_raw") {
            return Some("Net");
        }
        return None;
    }
    // WebSockets — tungstenite (the modern successor to the old `websocket` crate). connect
    // and the socket read/write/send are network; Message constructors are pure. (Found on a
    // tungstenite consumer: connect + send + read classified pure.)
    if crate_name == "tungstenite" {
        if path.ends_with("::connect")
            || path.ends_with("::read")
            || path.ends_with("::write")
            || path.ends_with("::send")
            || path.ends_with("::close")
            || path.ends_with("::flush")
            || path.ends_with("::read_message")
            || path.ends_with("::write_message")
        {
            return Some("Net");
        }
        return None;
    }
    // elasticsearch: request builders are pure; only the `.send()` dispatch is HTTP I/O
    // (same shape as reqwest / the AWS SDK). (Found on an elasticsearch consumer.)
    if crate_name == "elasticsearch" && path.ends_with("::send") {
        return Some("Net");
    }
    // gRPC — tonic. The transport connect and the Grpc client RPC dispatch are network;
    // codecs and request/response wrappers are pure. (connect repro-confirmed on a consumer;
    // the unary/streaming RPC verbs are from the tonic::client::Grpc API.)
    if crate_name == "tonic" {
        if path.ends_with("::connect")
            || path.ends_with("::unary")
            || path.ends_with("::server_streaming")
            || path.ends_with("::client_streaming")
            || path.ends_with("::streaming")
        {
            return Some("Net");
        }
        return None;
    }
    // Kafka — rdkafka (FFI to librdkafka). Producer send + consumer poll/recv/subscribe/
    // commit are network round-trips to the brokers. (API-calibrated + unit-tested; a real
    // repro needs librdkafka/cmake, deferred.)
    if crate_name == "rdkafka" {
        if path.ends_with("::send")
            || path.ends_with("::send_result")
            || path.ends_with("::recv")
            || path.ends_with("::poll")
            || path.ends_with("::subscribe")
            || path.ends_with("::commit")
            || path.ends_with("::commit_message")
            || path.ends_with("::commit_consumer_state")
            || path.ends_with("::store_offset")
            || path.ends_with("::seek")
            || path.ends_with("::fetch_metadata")
            || path.ends_with("::fetch_watermarks")
            || path.ends_with("::flush")
        {
            return Some("Net");
        }
        return None;
    }
    // cap-std: capability-oriented std. I/O goes *through* a held capability handle
    // (Dir/Pool/Clock/...), so these calls ARE the effect. Recognising them means a
    // cap-std project's real I/O is detected and matches the capability it declared
    // (via `declared_caps`/`capstd_cap`) — conformance against unforgeable capabilities.
    if crate_name.starts_with("cap_") {
        if path.contains("::net::Unix") || path.contains("::os::") {
            return Some("Ipc");
        }
        if path.contains("::net") {
            return Some("Net");
        }
        if path.contains("::time") {
            return Some("Clock");
        }
        if path.contains("::fs") || crate_name == "cap_tempfile" || crate_name == "cap_directories" {
            return Some("Fs");
        }
        return None;
    }
    // Local IPC (Unix-domain sockets) is I/O but not *network* — keep it distinct so
    // CANDOR_NO_AMBIENT and audits don't conflate it with internet access. async-std puts its
    // Unix sockets under `os::unix::net` (mirroring std); async-net (smol's net layer) under
    // `unix`.
    if path.starts_with("tokio::net::Unix")
        || path.starts_with("std::os::unix::net")
        || path.starts_with("async_std::os::unix::net")
        || path.starts_with("async_net::unix")
    {
        return Some("Ipc");
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
    // Legacy tokio 0.1 socket crates — `tokio_tcp`/`tokio_udp` are *entirely* networking
    // (no pure types to over-flag), so the whole crate is Net. (Found hardening on websocat,
    // which is still on tokio 0.1: its `tokio_tcp::TcpStream::connect` was classified
    // network-free — a network tool confidently reporting 0 Net.)
    if matches!(crate_name, "tokio_tcp" | "tokio_udp") {
        return Some("Net");
    }
    // The other async runtimes mirror tokio's module layout, and their `net` modules hold only
    // socket I/O types (the pure `SocketAddr`/`IpAddr` are re-exports that resolve to `std::net`,
    // so they're excluded by def-path). `mio` is the low-level non-blocking-socket layer under
    // tokio/others; `async_net` is smol's net crate. Closes the async-std/smol/mio gap the
    // tokio_tcp note flagged. (Calibrated by module structure — these crates ARE networking — not
    // a live repro; the TCP/UDP types are defined in-crate so the def-path prefix is exact.)
    if path.starts_with("async_std::net::")
        || path.starts_with("mio::net::")
        || crate_name == "async_net"
    {
        return Some("Net");
    }
    // Database clients. Like the AWS/HTTP builders, only the execution verbs are I/O;
    // query *construction* is pure. Best-effort across crates (tune via CANDOR_CONFIG).
    // Note: bare `::query` is deliberately omitted — it executes in postgres/rusqlite but
    // only *builds* in sqlx, so including it would false-positive sqlx's `query()` builder.
    if DB_CRATES.contains(&crate_name) {
        // Postgres / SQLite-family clients: `query`/`batch_execute`/`prepare`/etc. ARE the
        // execution (round-trips to the server). sqlx is the outlier where bare `query()`
        // only BUILDS — it keeps the narrow set below. (Found by running on a real
        // tokio-postgres app, pgman: candor had reported only 4 of ~20 DB call sites.)
        if matches!(crate_name, "postgres" | "tokio_postgres" | "deadpool_postgres" | "rusqlite") {
            const PG: [&str; 13] = [
                "::query", "::query_one", "::query_opt", "::query_raw", "::execute",
                "::batch_execute", "::simple_query", "::prepare", "::prepare_typed",
                "::copy_in", "::copy_out", "::transaction", "::connect",
            ];
            if PG.iter().any(|v| path.ends_with(v)) {
                return Some("Db");
            }
            return None;
        }
        // redis: the way redis is ACTUALLY used is the high-level `Commands`/`AsyncCommands`
        // traits (`con.get`/`set`/`hset`/`lpush`/…) — every method is a round-trip — plus
        // connection establishment. The shared VERBS below only catch the low-level
        // `cmd("GET").query(con)`, so without this a normal redis user's calls classify as
        // PURE. (Found hardening on redis-rs: a fn doing `con.get`/`set` reported no effects.)
        if crate_name == "redis"
            && (path.contains("Commands::")
                || path.contains("::get_connection")
                || path.contains("::get_async_connection")
                || path.contains("::get_multiplexed_async_connection")
                || path.contains("ConnectionManager")
                || path.ends_with("::query")
                || path.ends_with("::query_async")
                || path.ends_with("::req_command")
                || path.ends_with("::req_packed_command")
                || path.ends_with("::req_packed_commands"))
        {
            return Some("Db");
        }
        // mongodb: a document-store API with none of the SQL verbs — the user calls
        // `coll.find_one`/`insert_one`/`aggregate`/… and `Client::with_uri_str`. Without
        // these a mongodb user's calls classify PURE. (Found hardening: a fn doing
        // `find_one`+`insert_one` reported no effects.) Handle accessors (name/namespace)
        // and option/doc builders don't match these verbs, so they stay pure.
        if crate_name == "mongodb" {
            const MONGO: [&str; 27] = [
                "::with_uri_str", "::connect", "::find", "::find_one", "::insert_one",
                "::insert_many", "::update_one", "::update_many", "::delete_one",
                "::delete_many", "::replace_one", "::aggregate", "::count_documents",
                "::estimated_document_count", "::count", "::distinct", "::run_command",
                "::find_one_and_update", "::find_one_and_delete", "::find_one_and_replace",
                "::list_collections", "::list_collection_names", "::list_databases",
                "::list_database_names", "::create_collection", "::create_index", "::watch",
            ];
            if MONGO.iter().any(|v| path.ends_with(v)) {
                return Some("Db");
            }
            return None;
        }
        // mysql / mysql_async: the `query`/`exec` families + `get_conn`/`ping` execute
        // immediately — no build-then-execute split like sqlx, so matching `::query` is safe
        // here. Same DB-verb-dialect gap class as redis/mongodb; calibrated from the Queryable
        // API (unit-tested; a real-app repro is the remaining confirmation).
        if matches!(crate_name, "mysql" | "mysql_async") {
            const MY: [&str; 16] = [
                "::query", "::query_first", "::query_iter", "::query_map", "::query_fold",
                "::query_drop", "::exec", "::exec_first", "::exec_iter", "::exec_map",
                "::exec_fold", "::exec_drop", "::exec_batch", "::prep", "::ping", "::get_conn",
            ];
            if MY.iter().any(|v| path.ends_with(v)) {
                return Some("Db");
            }
            return None;
        }
        // sea_orm: an ORM whose execution is split from building (like sqlx). The query
        // BUILDERS (`Entity::find`, `Entity::insert`) are pure; execution happens at `.all`/
        // `.one`/`.count`/`.stream` and `Insert/Update/Delete::exec`. The write path via an
        // ActiveModel (`model.insert(db)`) executes too — distinguished from the `EntityTrait`
        // builder by the trait in the path (`ActiveModelTrait::`). (Found hardening on a
        // sea_orm consumer app: `.all(db)` reads and `ActiveModel::insert` writes were pure.)
        if crate_name == "sea_orm" {
            if path.ends_with("::all")
                || path.ends_with("::one")
                || path.ends_with("::count")
                || path.ends_with("::stream")
                || path.ends_with("::exec")
                || path.ends_with("::exec_with_returning")
                || path.ends_with("::exec_without_returning")
                || path.ends_with("::connect")
                || path.ends_with("::execute")
                || path.ends_with("::execute_unprepared")
                || path.ends_with("::query_one")
                || path.ends_with("::query_all")
                || path.ends_with("::fetch_page")
                || path.ends_with("::num_items")
                || path.contains("ActiveModelTrait::")
            {
                return Some("Db");
            }
            return None;
        }
        const VERBS: [&str; 16] = [
            "::execute", "::query_row", "::query_map", "::query_one", "::fetch_one",
            "::fetch_all", "::fetch_optional", "::fetch", "::connect", "::acquire",
            "::begin", "::commit", "::rollback", "::load", "::get_result", "::get_results",
        ];
        if VERBS.iter().any(|v| path.ends_with(v)) {
            return Some("Db");
        }
        return None;
    }
    // Filesystem. `tokio::fs`/`async_std::fs` are the async mirrors of `std::fs`; `async_fs` is
    // smol's fs crate; `fs_err` is a drop-in `std::fs` wrapper (its whole surface is fs I/O).
    if path.starts_with("std::fs::")
        || path.starts_with("tokio::fs::")
        || path.starts_with("async_std::fs::")
        || crate_name == "async_fs"
        || crate_name == "fs_err"
        || crate_name == "memmap2"
    {
        return Some("Fs");
    }
    // tempfile: creating a temp file/dir touches the disk. Match the create/persist verbs (the
    // `Builder` setters — prefix/suffix/rand_bytes — stay pure). `persist`/`keep` rename/retain
    // the file on disk; `close` removes it.
    if crate_name == "tempfile"
        && (path.ends_with("::tempfile")
            || path.ends_with("::tempfile_in")
            || path.ends_with("::tempdir")
            || path.ends_with("::tempdir_in")
            || path.ends_with("NamedTempFile::new")
            || path.ends_with("NamedTempFile::new_in")
            || path.ends_with("TempDir::new")
            || path.ends_with("TempDir::new_in")
            || path.ends_with("::persist")
            || path.ends_with("::persist_noclobber")
            || path.ends_with("::keep"))
    {
        return Some("Fs");
    }
    // glob: walks the filesystem to expand a pattern (the returned iterator reads directories).
    // `Pattern::matches` is pure string matching — match only the directory-walking entry points.
    if crate_name == "glob" && (path.ends_with("::glob") || path.ends_with("::glob_with")) {
        return Some("Fs");
    }
    // Randomness / entropy. `getrandom`/`fastrand` are effectful end-to-end. `rand` is NOT — it
    // mixes entropy/generation (effectful) with *pure* distribution constructors (`Uniform::new`,
    // `Normal::new`) and deterministic-seed constructors (`seed_from_u64`). Flagging the whole crate
    // over-reported those as `Rand`; match only the calls that actually consume randomness — the
    // entropy sources (`OsRng`, `thread_rng`/`rng`, `from_entropy`/`from_os_rng`) and the generation
    // verbs (`gen*`/`random*`/`fill*`/`sample*`/`next_u*`). A `Uniform::new` is now correctly pure.
    if crate_name == "getrandom" || crate_name == "fastrand" {
        return Some("Rand");
    }
    if crate_name == "rand" {
        let rng_verb = path.ends_with("::gen")
            || path.ends_with("::gen_range")
            || path.ends_with("::gen_bool")
            || path.ends_with("::gen_ratio")
            || path.ends_with("::random")
            || path.ends_with("::random_range")
            || path.ends_with("::random_bool")
            || path.ends_with("::random_ratio")
            || path.ends_with("::fill")
            || path.ends_with("::fill_bytes")
            || path.ends_with("::try_fill")
            || path.ends_with("::try_fill_bytes")
            || path.ends_with("::sample")
            || path.ends_with("::sample_iter")
            || path.ends_with("::next_u32")
            || path.ends_with("::next_u64")
            || path.ends_with("::thread_rng")
            || path.ends_with("::rng")
            || path.ends_with("::from_entropy")
            || path.ends_with("::from_os_rng");
        if rng_verb || path.contains("OsRng") {
            return Some("Rand");
        }
        return None;
    }
    // Subprocess spawning. `tokio::process` is the async mirror of `std::process` — it exists
    // only to spawn/control subprocesses (`Command`/`Child`, no pure data types like std's
    // `Stdio`/`ExitStatus`/`exit`), so spawning through it is Exec just the same. Without this an
    // async app's `tokio::process::Command::new(..).spawn()` classified pure — a silent under-report
    // of subprocess execution, the dangerous direction (mirrors the tokio::fs/tokio::net coverage).
    if path.starts_with("std::process::Command")
        || path.starts_with("std::process::Child")
        || path.starts_with("tokio::process::Command")
        || path.starts_with("tokio::process::Child")
        || path.starts_with("async_std::process::Command")
        || path.starts_with("async_std::process::Child")
        || crate_name == "async_process"
        || crate_name == "portable_pty"
    {
        return Some("Exec");
    }
    // duct: a subprocess-orchestration crate. `cmd()`/`cmd!` only *build* an Expression; the
    // spawn/wait happens at `run`/`read`/`start`. Match the execution verbs, not the builder.
    if crate_name == "duct"
        && (path.ends_with("::run")
            || path.ends_with("::read")
            || path.ends_with("::start")
            || path.ends_with("::read_chars"))
    {
        return Some("Exec");
    }
    if path.starts_with("std::env::") {
        return Some("Env");
    }
    // dotenvy / dotenv: load environment variables (reading a `.env` file and mutating the process
    // environment). Match the load/read entry points; `Error`/builder types stay pure.
    if matches!(crate_name, "dotenvy" | "dotenv")
        && (path.ends_with("::dotenv")
            || path.ends_with("::dotenv_override")
            || path.ends_with("::from_path")
            || path.ends_with("::from_path_override")
            || path.ends_with("::from_filename")
            || path.ends_with("::from_filename_override")
            || path.ends_with("::from_read")
            || path.ends_with("::from_read_override")
            || path.ends_with("::load")
            || path.ends_with("::var")
            || path.ends_with("::vars"))
    {
        return Some("Env");
    }
    // Wall-clock reads. Match the `now` accessor precisely (ends_with), not any path
    // containing the substring "now". The `time` crate (distinct from `std::time`/`chrono`)
    // reads the clock via `now_utc`/`now_local` (and the deprecated `Instant::now`).
    if (crate_name == "chrono" || path.starts_with("std::time::")) && path.ends_with("::now") {
        return Some("Clock");
    }
    if crate_name == "time"
        && (path.ends_with("::now_utc") || path.ends_with("::now_local") || path.ends_with("::now"))
    {
        return Some("Clock");
    }
    if crate_name == "tracing" {
        return Some("Log");
    }
    // The `log` facade: its macros route through `log::__private_api`; the crate's types
    // (`Level`, `LevelFilter`) are pure, so match the logging entry, not the whole crate.
    if crate_name == "log" && path.contains("::__private_api") {
        return Some("Log");
    }
    if crate_name == "arboard" {
        return Some("Clipboard");
    }
    None
}

fn cap_from_name(name: &str) -> Option<&'static str> {
    EFFECTS.iter().copied().find(|e| *e == name)
}

/// Map a cap-std capability *type* to the effect it authorises. Holding one of these
/// (e.g. `&Dir`) is the real, unforgeable right to perform that effect — so candor
/// treats it as a declared capability, exactly like its own `&Fs` token.
fn capstd_cap(crate_name: &str, type_name: &str) -> Option<&'static str> {
    if !crate_name.starts_with("cap_") {
        return None;
    }
    Some(match type_name {
        "Dir" => "Fs",
        "TcpListener" | "TcpStream" | "UdpSocket" | "Pool" => "Net",
        "UnixListener" | "UnixStream" | "UnixDatagram" => "Ipc",
        "SystemClock" | "MonotonicClock" => "Clock",
        _ => return None,
    })
}

/// Conventionally-pure std/core/alloc traits. Dynamic dispatch over these (e.g.
/// `.to_string()` / `.source()` on a `&dyn std::error::Error`) is overwhelmingly
/// side-effect-free, so we DON'T stamp it `Unknown` — doing so floods reports with
/// false positives (found in the wild: `dyn Error` error-formatting taints whole call
/// trees). This only matters for NON-local impls; a project's own effectful impl of
/// these is local and resolved precisely by CHA. Traits where dispatch genuinely hides
/// I/O (Iterator, Fn*, Drop, io::Write, …) are deliberately excluded.
fn is_pure_std_trait(crate_name: &str, trait_name: &str) -> bool {
    matches!(crate_name, "core" | "std" | "alloc")
        && matches!(
            trait_name,
            "Display"
                | "Debug"
                | "Error"
                | "ToString"
                | "Clone"
                | "PartialEq"
                | "Eq"
                | "PartialOrd"
                | "Ord"
                | "Hash"
                | "Default"
        )
}

/// std traits whose dispatch genuinely *hides I/O*: the impl behind a generic `R: Read` / `W: Write`
/// could be a file (`Fs`), a socket (`Net`), or a pure in-memory buffer — candor can't see which
/// across the (non-local) trait, so a generic call over one is honestly `Unknown`, NOT assumed pure.
/// Matched by full path so `std::io::Write` (effectful) is never confused with `core::fmt::Write`
/// (pure formatting). Bounded to the std I/O traits — the case where "assumed pure" is a real
/// *under*-report (the dangerous direction) without flooding the way marking *all* generic dispatch
/// Unknown would (that stays behind `CANDOR_PARANOID`).
fn is_effectful_std_trait(trait_path: &str) -> bool {
    matches!(
        trait_path,
        "std::io::Read" | "std::io::Write" | "std::io::BufRead" | "std::io::Seek"
    )
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
            let name = tcx.item_name(adt.did());
            let krate = tcx.crate_name(adt.did().krate);
            if let Some(c) = cap_from_name(name.as_str())
                .or_else(|| capstd_cap(krate.as_str(), name.as_str()))
            {
                out.insert(c);
            }
        }
    }
    out
}

/// Items we attribute effects to and report on: functions, plus const/static
/// initializers (a `static X: T = effectful();` performs its effect at init).
fn is_reportable_item(dk: DefKind) -> bool {
    matches!(
        dk,
        DefKind::Fn
            | DefKind::AssocFn
            | DefKind::Const { .. }
            | DefKind::AssocConst { .. }
            | DefKind::Static { .. }
    )
}

// --- Conformance-mode decisions: the set arithmetic behind the AS-EFF diagnostics, factored
// out of check_crate_post so they can be unit-tested without a compiler. ---

/// AS-EFF-001 surface: effects performed but not declared. `Unknown` is excluded — it isn't a
/// declarable capability (it's handled by AS-EFF-003).
fn undeclared_effects<'a>(inferred: &BTreeSet<&'a str>, declared: &BTreeSet<&'a str>) -> Vec<&'a str> {
    inferred.iter().copied().filter(|e| *e != UNKNOWN && !declared.contains(e)).collect()
}

/// AS-EFF-002 surface: capabilities declared but never performed.
fn overdeclared_effects<'a>(declared: &BTreeSet<&'a str>, inferred: &BTreeSet<&'a str>) -> Vec<&'a str> {
    declared.iter().copied().filter(|c| !inferred.contains(c)).collect()
}

/// AS-EFF-004 surface: direct reaches for ambient authority (vs. a received capability).
fn ambient_effects<'a>(direct: &BTreeSet<&'a str>) -> Vec<&'a str> {
    direct.iter().copied().filter(|e| AMBIENT.contains(e)).collect()
}

/// AS-EFF-005 surface: effects gained versus a saved baseline.
fn gained_effects<'a>(inferred: &BTreeSet<&'a str>, baseline: &BTreeSet<String>) -> Vec<&'a str> {
    inferred.iter().copied().filter(|e| !baseline.contains(*e)).collect()
}

/// Nearest enclosing reportable item of `hir_id`, walking up out of closures so that
/// effects performed inside an inline closure are charged to the item that owns it.
fn enclosing_named_fn(tcx: TyCtxt<'_>, hir_id: HirId) -> Option<LocalDefId> {
    let mut owner = tcx.hir_enclosing_body_owner(hir_id);
    loop {
        let dk = tcx.def_kind(owner.to_def_id());
        if is_reportable_item(dk) {
            return Some(owner);
        }
        if matches!(dk, DefKind::Closure) {
            let closure_hir = tcx.local_def_id_to_hir_id(owner);
            let parent = tcx.hir_enclosing_body_owner(closure_hir);
            if parent == owner {
                return None;
            }
            owner = parent;
        } else {
            return None;
        }
    }
}

// --- Taint heuristic (CANDOR_TAINT): flag an effect whose argument derives from a function
// parameter — e.g. `fs::read(format!("/var/cache/{key}"))` where `key` is a param. This is the
// injection class (path traversal / command injection / SSRF). It is an INTRAPROCEDURAL, SYNTACTIC
// heuristic — a review nudge, NOT sound taint analysis. It misses cross-function flow, flow through
// struct fields, and builder chains; it over-flags a param that is actually validated. Honest signal,
// stated limits. ---

/// HirIds of the binding patterns in a function's parameters (the "untrusted input" surface).
fn param_bindings(tcx: TyCtxt<'_>, def_id: LocalDefId) -> std::collections::HashSet<HirId> {
    let mut out = std::collections::HashSet::new();
    if !matches!(tcx.def_kind(def_id.to_def_id()), DefKind::Fn | DefKind::AssocFn) {
        return out;
    }
    let body = tcx.hir_body_owned_by(def_id);
    for p in body.params {
        p.pat.walk(|pat| {
            if let rustc_hir::PatKind::Binding(_, hir_id, _, _) = pat.kind {
                out.insert(hir_id);
            }
            true
        });
    }
    out
}

/// True if `e` (or any sub-expression, including inside macro expansions like `format!`) references
/// one of `locals` — the syntactic core of the taint heuristic.
fn expr_uses_local(e: &Expr<'_>, locals: &std::collections::HashSet<HirId>) -> bool {
    use rustc_hir::intravisit::Visitor;
    struct V<'a> {
        locals: &'a std::collections::HashSet<HirId>,
        found: bool,
    }
    impl<'v, 'a> Visitor<'v> for V<'a> {
        fn visit_expr(&mut self, e: &'v Expr<'v>) {
            if self.found {
                return;
            }
            if let ExprKind::Path(rustc_hir::QPath::Resolved(_, path)) = e.kind {
                if let rustc_hir::def::Res::Local(id) = path.res {
                    if self.locals.contains(&id) {
                        self.found = true;
                        return;
                    }
                }
            }
            rustc_hir::intravisit::walk_expr(self, e);
        }
    }
    let mut v = V { locals, found: false };
    v.visit_expr(e);
    v.found
}

/// True if the effect call's argument(s)/receiver derive from a parameter of the enclosing function.
fn effect_arg_from_param(tcx: TyCtxt<'_>, call: &Expr<'_>, caller: LocalDefId) -> bool {
    let params = param_bindings(tcx, caller);
    if params.is_empty() {
        return false;
    }
    let operands: Vec<&Expr<'_>> = match call.kind {
        ExprKind::Call(_, args) => args.iter().collect(),
        ExprKind::MethodCall(_, recv, args, _) => std::iter::once(recv).chain(args.iter()).collect(),
        _ => return false,
    };
    operands.iter().any(|a| expr_uses_local(a, &params))
}

impl<'tcx> LateLintPass<'tcx> for Candor {
    fn check_crate(&mut self, cx: &LateContext<'tcx>) {
        // Cross-crate resolution: load THIS project's other crates' reports so calls into them
        // resolve transitively. dylint lints dependencies before dependents, so a dependency
        // crate's report is on disk when we get here. Load from CANDOR_JSON (snapshot/audit) OR
        // CANDOR_BASELINE (the guard) — so the guard computes the SAME cross-inclusive effect set
        // the baseline was snapshotted with, instead of a within-crate-only set (which would make
        // the AS-EFF-005 diff compare two different effect models).
        // CANDOR_JSON (snapshot/audit) loads LIVE sibling reports — version-trust applies. CANDOR_BASELINE
        // (the guard) loads the baseline's OWN snapshot of the siblings — trust them as-is, so the engine
        // moving ahead of the baseline doesn't spuriously downgrade them to Unknown (see load_cross_reports).
        let (prefix, trust_siblings) = match std::env::var("CANDOR_JSON") {
            Ok(p) => (Some(p), false),
            Err(_) => (std::env::var("CANDOR_BASELINE").ok(), true),
        };
        if let Some(prefix) = prefix {
            let me = cx.tcx.crate_name(rustc_hir::def_id::LOCAL_CRATE).to_string();
            let me_kind = cx
                .tcx
                .crate_types()
                .iter()
                .map(|t| format!("{t:?}"))
                .collect::<Vec<_>>()
                .join("-");
            self.cross = load_cross_reports(&prefix, &me, &me_kind, trust_siblings);
        }
    }

    fn check_expr(&mut self, cx: &LateContext<'tcx>, expr: &'tcx Expr<'tcx>) {
        // A named function referenced as a VALUE — passed as a callback, stored in a struct, handed
        // to a higher-order combinator (`iter().map(parse)`, `thread::spawn(work)`, `register(cb)`)
        // — isn't *called* here, but its effects are reachable through whoever invokes it. Add a call
        // edge so they propagate, exactly as an inline closure's body is charged to its enclosing fn.
        // Without it, an effectful fn passed as a callback looked pure to its passer — a silent
        // under-report. (This is the statically-resolvable half of the closure problem: the callee
        // identity is a known `FnDef`. The still-deferred residue is the *receiving* side — an
        // `impl Fn` parameter whose concrete target needs interprocedural flow, kept honest `Unknown`.
        // The callee position of a normal call is also a `FnDef` path; re-adding that edge is a
        // harmless no-op on the `calls` set.)
        if let ExprKind::Path(..) = expr.kind {
            if let Some(typeck) = cx.maybe_typeck_results() {
                if let rustc_middle::ty::TyKind::FnDef(did, _) = typeck.expr_ty(expr).kind() {
                    if let (Some(local), Some(caller)) =
                        (did.as_local(), enclosing_named_fn(cx.tcx, expr.hir_id))
                    {
                        if matches!(cx.tcx.def_kind(*did), DefKind::Fn | DefKind::AssocFn) {
                            self.calls.entry(caller).or_default().insert(local);
                        }
                    }
                }
            }
        }

        let Some(callee) = resolve_callee(cx, expr) else {
            return;
        };
        let Some(caller) = enclosing_named_fn(cx.tcx, expr.hir_id) else {
            return;
        };

        let (def_id, dynamic) = match callee {
            // A call we cannot see through at all (fn pointer / `impl Fn` callback).
            Callee::Unresolved => {
                self.direct.entry(caller).or_default().insert(UNKNOWN);
                if self.explain.is_some() {
                    let loc = cx.tcx.sess.source_map().span_to_diagnostic_string(expr.span);
                    self.sites.entry(caller).or_default().push(EffectSite {
                        eff: UNKNOWN,
                        via: "unresolvable call (fn-pointer / closure)".to_string(),
                        loc,
                    });
                }
                return;
            }
            Callee::Def { did, dynamic } => (did, dynamic),
        };

        // Record a local call edge for transitive propagation.
        let add_edge = |this: &mut Self, target: DefId| {
            if let Some(local) = target.as_local() {
                if matches!(cx.tcx.def_kind(target), DefKind::Fn | DefKind::AssocFn) {
                    this.calls.entry(caller).or_default().insert(local);
                }
            }
        };
        add_edge(self, def_id);

        // Resolve a trait-method call to the impls whose effects it could perform. PREFER
        // devirtualization: a call on a CONCRETE (non-`dyn`) receiver of a LOCAL trait
        // dispatches to exactly ONE impl, and we can see its body — so use it instead of
        // CHA-expanding to every impl (the over-approximation that made a pure `self.applies()`
        // inherit a sibling rule's effect — CRITIQUE §9). CHA remains the sound fallback for
        // `dyn`/generic dispatch we can't pin down. (Non-local traits: neither sees the body;
        // left to the `Unknown` logic below.)
        let trait_did = cx.tcx.trait_of_assoc(def_id);
        let mut cha_resolved = false;
        if let Some(td) = trait_did {
            // Only accept a devirtualized target we can actually analyze: a LOCAL fn/method whose
            // body we'll see. If resolution lands on a non-local target (would be silently dropped
            // by `add_edge`, leaving `cha_resolved = true` to suppress the honest `Unknown`), fall
            // back to CHA instead. (Defensive: orphan rules make a local trait's impls local, so
            // this rarely fires — but it keeps the soundness invariant explicit.)
            let devirt = if !dynamic && td.is_local() {
                devirtualize(cx, expr, def_id).filter(|t| t.is_local())
            } else {
                None
            };
            match devirt {
                Some(target) => {
                    add_edge(self, target);
                    cha_resolved = true;
                }
                None => {
                    for target in cha_targets(cx.tcx, def_id) {
                        cha_resolved = true;
                        add_edge(self, target);
                    }
                }
            }
        }

        // Honest `Unknown` only when dispatch is genuinely unresolvable here:
        //  - a `dyn` call over a NON-local trait (we can't see its impl bodies), or
        //  - (paranoid) any trait dispatch CHA couldn't pin to local impls.
        // ...but NOT for conventionally-pure std traits (Display/Error/…), where the
        // overwhelmingly-pure dispatch would otherwise flood reports with false Unknowns.
        if let Some(td) = trait_did {
            let pure = is_pure_std_trait(cx.tcx.crate_name(td.krate).as_str(), cx.tcx.item_name(td).as_str());
            // Generic (non-`dyn`) dispatch is assumed pure by default (marking it all Unknown floods —
            // that's `CANDOR_PARANOID`). EXCEPT over a known-effectful std trait (`io::Read`/`Write`/…),
            // where "assumed pure" is a real under-report: the reader/writer behind the generic could be
            // a file or socket. So those are Unknown by default too — bounded, doesn't flood.
            let effectful_dispatch = is_effectful_std_trait(&cx.tcx.def_path_str(td));
            if !cha_resolved && !pure && (dynamic || self.paranoid || effectful_dispatch) {
                self.direct.entry(caller).or_default().insert(UNKNOWN);
                if self.explain.is_some() {
                    let loc = cx.tcx.sess.source_map().span_to_diagnostic_string(expr.span);
                    let via = format!("unresolvable dispatch over `{}`", cx.tcx.def_path_str(td));
                    self.sites.entry(caller).or_default().push(EffectSite { eff: UNKNOWN, via, loc });
                }
            }
        }

        // Record a directly-performed effect (built-in classifier, then project rules).
        let crate_name = cx.tcx.crate_name(def_id.krate);
        // Note every EXTERNAL crate we actually saw a resolved call into — ground truth for
        // the coverage blind-spot check (catches deps declared in workspace members, which a
        // root-manifest scan misses). std/local are excluded; the consumer filters the rest.
        if !def_id.is_local()
            && !matches!(crate_name.as_str(), "std" | "core" | "alloc" | "proc_macro" | "test")
        {
            self.encountered.insert(crate_name.to_string());
        }
        let path = cx.tcx.def_path_str(def_id);
        let builtin = classify(crate_name.as_str(), &path);
        let effect = builtin.or_else(|| classify_extra(crate_name.as_str(), &path, &self.extra));
        if let Some(effect) = effect {
            self.direct.entry(caller).or_default().insert(effect);
            // Non-breaking Fs refinement: when the verb tells us read vs write, record it (propagated
            // like effects below, surfaced as the report's `fs` detail). The `Fs` effect is unchanged.
            // Gated to built-in Fs classification — see `fs_detail_for`.
            let kinds = fs_detail_for(builtin, &path);
            if !kinds.is_empty() {
                self.fs_direct.entry(caller).or_default().extend(kinds.iter().copied());
            }
            // Non-breaking Net refinement: a literal address/URL argument tells us the endpoint.
            // Gated to built-in Net classification (a user `extra` rule's arg shape is unknown).
            if builtin == Some("Net") {
                let hosts = net_hosts_in_call(expr);
                if !hosts.is_empty() {
                    self.net_hosts_direct.entry(caller).or_default().extend(hosts);
                }
            }
            if self.explain.is_some() {
                let loc = cx.tcx.sess.source_map().span_to_diagnostic_string(expr.span);
                self.sites.entry(caller).or_default().push(EffectSite { eff: effect, via: path.clone(), loc });
            }
            // Taint heuristic: an injection-class effect on a parameter-derived argument.
            if self.taint
                && matches!(effect, "Fs" | "Exec" | "Db" | "Net" | "Env" | "Ipc")
                && effect_arg_from_param(cx.tcx, expr, caller)
            {
                self.tainted.entry(caller).or_default().insert(effect);
            }
        } else if !def_id.is_local() && !self.cross.is_empty() {
            // Cross-crate: a call into one of THIS project's other crates (its lib, a sibling
            // workspace member). Inherit the callee's already-transitive effects, looked up by its
            // stable DefPathHash (matches whether the dependency emitted it locally or we see it
            // externally). For a TRAIT-method call the callee `def_id` is the trait method, but the
            // dependency keyed its report by the concrete IMPL method — so devirtualize to that
            // impl first (when the receiver is concrete), else the lookup would always miss.
            let key_did = devirtualize(cx, expr, def_id).unwrap_or(def_id);
            if let Some(effs) = self.cross.get(&dph(cx.tcx, key_did)).cloned() {
                for e in &effs {
                    self.via_cross.entry(caller).or_default().insert(*e);
                }
                if self.explain.is_some() {
                    let loc = cx.tcx.sess.source_map().span_to_diagnostic_string(expr.span);
                    let via = format!("cross-crate call to `{}`", cx.tcx.def_path_str(key_did));
                    for e in &effs {
                        self.sites.entry(caller).or_default().push(EffectSite { eff: e, via: via.clone(), loc: loc.clone() });
                    }
                }
            }
        }
    }

    fn check_crate_post(&mut self, cx: &LateContext<'tcx>) {
        // effects[f] = direct[f] ∪ ⋃ { effects[g] : g ∈ calls[f] }, to a fixpoint.
        let mut eff: HashMap<LocalDefId, BTreeSet<&'static str>> = self.direct.clone();
        // Fold in effects inherited via cross-crate calls (kept out of `direct` so the report's
        // `direct` stays "own body"; they appear in `inferred` and propagate transitively).
        for (f, effs) in &self.via_cross {
            eff.entry(*f).or_default().extend(effs.iter().copied());
        }
        for f in self.calls.keys() {
            eff.entry(*f).or_default();
        }
        // Seed every reportable item so over-declaration (declared-but-unused) is covered.
        for owner in cx.tcx.hir_body_owners() {
            if is_reportable_item(cx.tcx.def_kind(owner.to_def_id())) {
                eff.entry(owner).or_default();
            }
        }

        let eff = propagate(eff, &self.calls);

        // Filesystem read/write detail rides the SAME propagation helper, in a separate set that never
        // touches `eff`:  fs[f] = fs_direct[f] ∪ ⋃ { fs[g] : g ∈ calls[f] }.  A function that reaches
        // the filesystem across a crate boundary inherits `Fs` but NO recorded kind (a dependency's
        // report omits it) — seed it with `FS_UNKNOWN` so we present an empty `fs` (no claim) rather
        // than a misleading partial like `Fs(read)` for a fn that also writes via that dependency.
        let mut fs_seed = self.fs_direct.clone();
        for (f, effs) in &self.via_cross {
            if effs.contains("Fs") {
                fs_seed.entry(*f).or_default().insert(FS_UNKNOWN);
            }
        }
        let fsacc = propagate(fs_seed, &self.calls);

        // Literal Net host detail rides the SAME propagation, in its own set that never touches `eff`:
        // hosts[f] = net_hosts_direct[f] ∪ ⋃ { hosts[g] : g ∈ calls[f] }. No cross-crate sentinel (unlike
        // fs): an absent host already reads as "no literal endpoint visible here" — a runtime-computed
        // address, or a dependency whose report carried none — which is the honest, never-over-claiming
        // interpretation. So a fn with `Net` but empty `hosts` means "talks to the network, endpoint not
        // statically known", exactly right.
        let hostsacc = propagate(self.net_hosts_direct.clone(), &self.calls);

        // CANDOR_EXPLAIN=<query>: for each matching function, trace the call path to where each of
        // its effects originates (the leaf call + location). A dedicated query mode — print and
        // return without the normal report/diagnostics.
        if let Some(query) = self.explain.clone() {
            let mut targets: Vec<LocalDefId> = eff
                .iter()
                .filter(|(_, e)| !e.is_empty())
                .map(|(f, _)| *f)
                .filter(|f| cx.tcx.def_path_str(f.to_def_id()).contains(&query))
                .collect();
            targets.sort_by_cached_key(|f| cx.tcx.def_path_str(f.to_def_id()));
            if targets.is_empty() {
                eprintln!("candor explain: no effectful function matching `{query}`.");
            }
            for f in &targets {
                eprintln!("\ncandor explain — {}", cx.tcx.def_path_str(f.to_def_id()));
                for &e in eff[f].iter() {
                    match self.find_source(*f, e) {
                        Some(path) => {
                            let leaf = *path.last().unwrap();
                            let chain = path
                                .iter()
                                .map(|d| cx.tcx.def_path_str(d.to_def_id()))
                                .collect::<Vec<_>>()
                                .join(" → ");
                            eprintln!("  {e:<9} {chain}");
                            if let Some(s) = self.sites.get(&leaf).and_then(|v| v.iter().find(|s| s.eff == e)) {
                                eprintln!("            └ {} via {} at {}", cx.tcx.def_path_str(leaf.to_def_id()), s.via, s.loc);
                            }
                        }
                        None => eprintln!("  {e:<9} (origin not localizable — inherited or via an unresolved path)"),
                    }
                }
            }
            return;
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
        let entry_fn = cx.tcx.entry_fn(()).map(|(did, _)| did);
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
        // A set-but-unloadable CANDOR_BASELINE is the dangerous case: the guard would
        // silently pass (no AS-EFF-005 ever fires). Make that loud.
        let baseline = match std::env::var("CANDOR_BASELINE") {
            Ok(prefix) => {
                let file = format!("{prefix}.{krate}.{kinds}.json");
                let loaded = load_baseline(&file);
                if loaded.is_none() {
                    eprintln!(
                        "candor: CANDOR_BASELINE set but {file:?} could not be loaded — \
                         the regression guard is NOT active"
                    );
                }
                loaded
            }
            Err(_) => None,
        };
        let any_enforce = strict_var.is_some()
            || no_ambient_var.is_some()
            || baseline.is_some()
            || !self.policy.is_empty()
            || self.taint;

        // Stable ordering for reproducible output.
        let mut items: Vec<LocalDefId> = eff.keys().copied().collect();
        items.sort_by_cached_key(|f| cx.tcx.def_path_str(f.to_def_id()));

        let mut json_entries: Vec<ReportEntry> = Vec::new();
        let owned = |v: &[&str]| v.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        let owned_set = |s: &BTreeSet<&str>| s.iter().map(|x| x.to_string()).collect::<Vec<_>>();

        for f in items {
            let span = cx.tcx.def_span(f);
            // Macro-generated items. The blanket `from_expansion()` skip was added to suppress the
            // flood from tracing's `__CALLSITE` *statics* — but it also hid macro-generated
            // FUNCTIONS (an `async_trait` method, a derive-impl method, a user decl-macro fn) that
            // can genuinely perform I/O. Narrow it: still skip macro-generated consts/statics (where
            // the flood lives — a `static __CALLSITE = …` per log site) but ANALYZE macro-generated
            // functions, so an effectful one is visible in the report and to the AS-EFF modes. (Their
            // bodies were always traced for propagation; this gives them their own row + diagnostics.)
            if span.from_expansion()
                && !matches!(cx.tcx.def_kind(f.to_def_id()), DefKind::Fn | DefKind::AssocFn)
            {
                continue;
            }
            let effs = &eff[&f];
            let name = cx.tcx.def_path_str(f.to_def_id());
            let declared = declared_caps(cx.tcx, f);
            let direct = self.direct.get(&f).cloned().unwrap_or_default();
            let has_unknown = effs.contains(UNKNOWN);
            let undeclared = undeclared_effects(effs, &declared);
            let unused = overdeclared_effects(&declared, effs);

            if json_path.is_some() {
                if effs.is_empty() && declared.is_empty() {
                    continue;
                }
                let loc = cx.tcx.sess.source_map().span_to_diagnostic_string(span);
                // The effect-relevant call graph: local callees that are themselves effectful.
                let mut calls: Vec<String> = self
                    .calls
                    .get(&f)
                    .map(|cs| {
                        cs.iter()
                            .filter(|c| eff.get(c).is_some_and(|e| !e.is_empty()))
                            .map(|c| cx.tcx.def_path_str(c.to_def_id()))
                            .collect()
                    })
                    .unwrap_or_default();
                calls.sort();
                json_entries.push(ReportEntry {
                    func: name,
                    loc,
                    calls,
                    inferred: owned_set(effs),
                    direct: owned_set(&direct),
                    declared: owned_set(&declared),
                    undeclared: owned(&undeclared),
                    overdeclared: owned(&unused),
                    unresolved: has_unknown,
                    hash: dph_hex(cx.tcx, f.to_def_id()),
                    // Empty when the kind is incomplete (FS_UNKNOWN — Fs reached cross-crate with no
                    // recorded detail): present no read/write claim rather than a misleading partial.
                    fs: match fsacc.get(&f) {
                        Some(s) if !s.contains(FS_UNKNOWN) => owned_set(s),
                        _ => Vec::new(),
                    },
                    // The literal Net endpoints statically visible from here (empty = none visible).
                    hosts: hostsacc.get(&f).map(|s| s.iter().cloned().collect()).unwrap_or_default(),
                });
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
                span_lint(
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
                let ambient = ambient_effects(&direct);
                if !ambient.is_empty() {
                    span_lint(
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
                // AS-EFF-001: performs an effect it does not declare. The real entry
                // point (per tcx.entry_fn) is exempt — it legitimately holds the bundle.
                if !undeclared.is_empty() && Some(f.to_def_id()) != entry_fn {
                    let have = if declared.is_empty() {
                        "no capabilities".to_string()
                    } else {
                        format!("only {{ {} }}", join(&declared))
                    };
                    span_lint(
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
                    span_lint(
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
                    span_lint(
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
                    let gained = gained_effects(effs, prior);
                    if !gained.is_empty() {
                        span_lint(
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

            // AS-EFF-006 (CANDOR_POLICY): the function transitively performs an effect a declared
            // boundary forbids. This is the architectural-invariant check — it catches an agent
            // putting I/O in a layer that's meant to be pure, which it can't see from a local edit.
            for rule in &self.policy {
                if let Some(scope) = &rule.scope {
                    if !name.contains(scope.as_str()) {
                        continue;
                    }
                }
                let bad: Vec<&str> = if rule.effects.is_empty() {
                    effs.iter().copied().collect() // `pure` rule: any effect is a violation
                } else {
                    effs.iter().copied().filter(|e| rule.effects.contains(e)).collect()
                };
                if !bad.is_empty() {
                    let scope = rule.scope.as_deref().map(|s| format!(" (scope `{s}`)")).unwrap_or_default();
                    span_lint(
                        cx,
                        CANDOR,
                        span,
                        format!(
                            "[AS-EFF-006] `{name}` performs {{ {} }}, forbidden by policy{scope}: `{}`",
                            bad.join(", "),
                            rule.raw
                        ),
                    );
                }
            }

            // AS-EFF-007 (CANDOR_TAINT): performs an injection-class effect on a parameter-derived
            // argument — a heuristic review nudge (see the taint helpers for its honest limits).
            if self.taint {
                if let Some(t) = self.tainted.get(&f).filter(|t| !t.is_empty()) {
                    span_lint(
                        cx,
                        CANDOR,
                        span,
                        format!(
                            "[AS-EFF-007] `{name}` performs {{ {} }} on caller-derived input \
                             (an injection surface — validate/sanitize it, or confirm the source is \
                             trusted); heuristic, may over- or under-flag",
                            t.iter().copied().collect::<Vec<_>>().join(", ")
                        ),
                    );
                }
            }
        }

        if let Some(prefix) = &json_path {
            let file = format!("{prefix}.{krate}.{kinds}.json");
            // v0.2: a self-describing envelope { candor: {version, toolchain}, functions: [...] }.
            let meta = ReportMeta { version: CANDOR_VERSION.into(), toolchain: CANDOR_TOOLCHAIN.into() };
            match to_report_json(&meta, &json_entries) {
                Ok(body) => match std::fs::write(&file, body) {
                    Ok(()) => eprintln!("candor: wrote {} entries to {file}", json_entries.len()),
                    Err(e) => eprintln!("candor: failed to write {file:?} ({e})"),
                },
                Err(e) => eprintln!("candor: failed to serialize report ({e})"),
            }
            // Emit candor's calibrated crate set alongside the report, so downstream
            // coverage checks read it from the engine rather than a duplicated copy. Also stamp
            // the engine's build identity here so the report is self-describing (a consumer in any
            // language, or the guard, can tell which candor produced it — and a newer engine can
            // refuse to silently trust a sibling crate's report from a different version).
            let calib = serde_json::json!({
                "candor_version": CANDOR_VERSION,
                "toolchain": CANDOR_TOOLCHAIN,
                // `.as_slice()`: serde only derives Serialize for arrays up to length 32.
                "crates": CALIBRATED_CRATES.as_slice(),
                "prefixes": CALIBRATED_PREFIXES.as_slice(),
                "path_crates": PATH_CALIBRATED_CRATES.as_slice(),
            });
            let cfile = format!("{prefix}.calibrated.json");
            if let Err(e) = std::fs::write(&cfile, calib.to_string()) {
                eprintln!("candor: failed to write {cfile:?} ({e})");
            }
            // Emit the external crates we actually saw called, one file per crate+kind (a
            // package emits the same crate name as both rlib and bin — they must NOT share a
            // file or the sparser one overwrites the other). Named `encountered-<krate>-<kind>`
            // (a single middle segment) so it does NOT match the `.*.*.json` report glob.
            let efile = format!("{prefix}.encountered-{krate}-{kinds}.json");
            let seen: Vec<&str> = self.encountered.iter().map(|s| s.as_str()).collect();
            if let Err(e) = std::fs::write(&efile, serde_json::to_string(&seen).unwrap_or_default()) {
                eprintln!("candor: failed to write {efile:?} ({e})");
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
        // Legacy tokio 0.1 socket crates (found on websocat) — entire crate is networking.
        assert_eq!(classify("tokio_tcp", "tokio_tcp::TcpStream::connect"), Some("Net"));
        assert_eq!(classify("tokio_udp", "tokio_udp::UdpSocket::bind"), Some("Net"));
        // ...but the pure data types living alongside them must NOT be flagged.
        assert_eq!(classify("std", "std::net::SocketAddr::new"), None);
        assert_eq!(classify("std", "std::net::Ipv4Addr::new"), None);
        // Unix-domain sockets are local IPC, not network.
        assert_eq!(classify("tokio", "tokio::net::UnixStream::connect"), Some("Ipc"));
        assert_eq!(classify("std", "std::os::unix::net::UnixStream::connect"), Some("Ipc"));
    }

    #[test]
    fn classify_databases() {
        // Execution verbs are Db I/O...
        assert_eq!(classify("sqlx", "sqlx::Pool::acquire"), Some("Db"));
        assert_eq!(classify("rusqlite", "rusqlite::Connection::execute"), Some("Db"));
        assert_eq!(classify("postgres", "postgres::Client::query_one"), Some("Db"));
        assert_eq!(classify("diesel", "diesel::RunQueryDsl::get_results"), Some("Db"));
        // ...but pure row/value accessors in the same crate are not.
        assert_eq!(classify("rusqlite", "rusqlite::Row::get"), None);
        assert_eq!(classify("sqlx", "sqlx::Column::name"), None);
        // Postgres-family: `query`/`batch_execute`/etc. ARE execution (found on pgman).
        assert_eq!(classify("tokio_postgres", "tokio_postgres::Client::query"), Some("Db"));
        assert_eq!(classify("tokio_postgres", "tokio_postgres::Client::batch_execute"), Some("Db"));
        assert_eq!(classify("postgres", "postgres::Client::query_opt"), Some("Db"));
        // ...but sqlx's bare `query()` is a builder, still excluded.
        assert_eq!(classify("sqlx", "sqlx::query"), None);
        // memmap2 is filesystem-backed.
        assert_eq!(classify("memmap2", "memmap2::MmapOptions::map"), Some("Fs"));
        // redis: the high-level Commands API + connection setup are Db (found on redis-rs);
        // the low-level cmd().query() too. Pure value accessors are not.
        assert_eq!(classify("redis", "redis::Commands::get"), Some("Db"));
        assert_eq!(classify("redis", "redis::AsyncCommands::set"), Some("Db"));
        assert_eq!(classify("redis", "redis::Client::get_connection"), Some("Db"));
        assert_eq!(classify("redis", "redis::cmd::Cmd::query"), Some("Db"));
        assert_eq!(classify("redis", "redis::Value::as_sequence"), None);
        // mongodb: document-store verbs are Db (found on a consumer app); handles/builders pure.
        assert_eq!(classify("mongodb", "mongodb::Client::with_uri_str"), Some("Db"));
        assert_eq!(classify("mongodb", "mongodb::Collection::find_one"), Some("Db"));
        assert_eq!(classify("mongodb", "mongodb::Collection::insert_one"), Some("Db"));
        assert_eq!(classify("mongodb", "mongodb::Collection::aggregate"), Some("Db"));
        assert_eq!(classify("mongodb", "mongodb::Collection::name"), None);
        assert_eq!(classify("mongodb", "mongodb::Database::collection"), None);
        // mysql/mysql_async: query/exec families execute immediately; accessors stay pure.
        assert_eq!(classify("mysql", "mysql::Conn::query_drop"), Some("Db"));
        assert_eq!(classify("mysql_async", "mysql_async::Conn::exec_drop"), Some("Db"));
        assert_eq!(classify("mysql", "mysql::Row::get"), None);
    }

    #[test]
    fn classify_aws_config_and_log() {
        // aws-config: `.load()` resolves credentials (Net); builders/types are pure.
        // (Found hardening on ebman: `aws_config::defaults(..).load()` was missed.)
        assert_eq!(classify("aws_config", "aws_config::loader::ConfigLoader::load"), Some("Net"));
        assert_eq!(classify("aws_config", "aws_config::load_defaults"), Some("Net"));
        assert_eq!(classify("aws_config", "aws_config::SdkConfig::builder"), None);
        assert_eq!(classify("aws_config", "aws_config::BehaviorVersion::latest"), None);
        // `log` facade: macros route through `__private_api`; Level/LevelFilter are pure.
        assert_eq!(classify("log", "log::__private_api::log"), Some("Log"));
        assert_eq!(classify("log", "log::LevelFilter::Info"), None);
    }

    #[test]
    fn classify_sea_orm() {
        // Executors are Db (found + validated on a sea_orm consumer app)…
        assert_eq!(classify("sea_orm", "sea_orm::Select::all"), Some("Db"));
        assert_eq!(classify("sea_orm", "sea_orm::Selector::one"), Some("Db"));
        assert_eq!(classify("sea_orm", "sea_orm::Insert::exec"), Some("Db"));
        assert_eq!(classify("sea_orm", "sea_orm::Database::connect"), Some("Db"));
        // …including the ActiveModel write path…
        assert_eq!(classify("sea_orm", "sea_orm::ActiveModelTrait::insert"), Some("Db"));
        // …but the query/insert BUILDERS stay pure — the ambiguity that made this tricky.
        assert_eq!(classify("sea_orm", "sea_orm::EntityTrait::find"), None);
        assert_eq!(classify("sea_orm", "sea_orm::EntityTrait::insert"), None);
    }

    #[test]
    fn classify_network_client_libs() {
        // SMTP, websockets, search — client libs that encapsulate the socket (found on
        // consumer apps). The executor verb is Net; builders/constructors stay pure.
        assert_eq!(classify("lettre", "lettre::SmtpTransport::send"), Some("Net"));
        assert_eq!(classify("lettre", "lettre::Message::builder"), None);
        assert_eq!(classify("tungstenite", "tungstenite::connect"), Some("Net"));
        assert_eq!(classify("tungstenite", "tungstenite::WebSocket::read"), Some("Net"));
        assert_eq!(classify("tungstenite", "tungstenite::Message::into_text"), None);
        assert_eq!(classify("elasticsearch", "elasticsearch::Search::send"), Some("Net"));
        assert_eq!(classify("elasticsearch", "elasticsearch::SearchParts::Index"), None);
        // gRPC + Kafka
        assert_eq!(classify("tonic", "tonic::transport::Endpoint::connect"), Some("Net"));
        assert_eq!(classify("tonic", "tonic::client::Grpc::unary"), Some("Net"));
        assert_eq!(classify("tonic", "tonic::Request::new"), None);
        assert_eq!(classify("rdkafka", "rdkafka::producer::FutureProducer::send"), Some("Net"));
        assert_eq!(classify("rdkafka", "rdkafka::consumer::StreamConsumer::recv"), Some("Net"));
        assert_eq!(classify("rdkafka", "rdkafka::message::BorrowedMessage::payload"), None);
    }

    #[test]
    fn classify_message_queues() {
        // Broker round-trip verbs are Net (found hardening on consumer apps).
        assert_eq!(classify("async_nats", "async_nats::connect"), Some("Net"));
        assert_eq!(classify("async_nats", "async_nats::Client::publish"), Some("Net"));
        assert_eq!(classify("async_nats", "async_nats::Client::subscribe"), Some("Net"));
        assert_eq!(classify("lapin", "lapin::Connection::connect"), Some("Net"));
        assert_eq!(classify("lapin", "lapin::Channel::basic_publish"), Some("Net"));
        assert_eq!(classify("lapin", "lapin::Channel::queue_declare"), Some("Net"));
        // CamelCase option/property builders stay pure.
        assert_eq!(classify("lapin", "lapin::BasicProperties::default"), None);
        assert_eq!(classify("lapin", "lapin::options::BasicPublishOptions::default"), None);
        assert_eq!(classify("async_nats", "async_nats::Subject::from"), None);
    }

    #[test]
    fn classify_git2_remote_network() {
        // Remote operations contact the network. (Found hardening on gitui: fetch/push
        // were classified network-free — a git client reporting no network calls.)
        assert_eq!(classify("git2", "git2::Remote::fetch"), Some("Net"));
        assert_eq!(classify("git2", "git2::Remote::push"), Some("Net"));
        assert_eq!(classify("git2", "git2::Remote::connect"), Some("Net"));
        // Local ops, and the Clone-trait dup of a Remote handle, are NOT network.
        assert_eq!(classify("git2", "git2::Repository::statuses"), None);
        assert_eq!(classify("git2", "git2::Remote::clone"), None);
        assert_eq!(classify("git2", "git2::Oid::from_str"), None);
    }

    #[test]
    fn classify_async_runtimes_and_aux_crates() {
        // async-std / smol(async-net/fs/process) / mio mirror std+tokio — same effects.
        assert_eq!(classify("async_std", "async_std::net::TcpStream::connect"), Some("Net"));
        assert_eq!(classify("mio", "mio::net::TcpListener::bind"), Some("Net"));
        assert_eq!(classify("async_net", "async_net::TcpStream::connect"), Some("Net"));
        assert_eq!(classify("async_net", "async_net::unix::UnixStream::connect"), Some("Ipc"));
        assert_eq!(classify("async_std", "async_std::os::unix::net::UnixStream::connect"), Some("Ipc"));
        assert_eq!(classify("async_std", "async_std::fs::read"), Some("Fs"));
        assert_eq!(classify("async_fs", "async_fs::read"), Some("Fs"));
        assert_eq!(classify("async_std", "async_std::process::Command::spawn"), Some("Exec"));
        assert_eq!(classify("async_process", "async_process::Command::spawn"), Some("Exec"));
        // SocketAddr/IpAddr re-exports resolve to std::net (not async_std::net) → not flagged.
        assert_eq!(classify("std", "std::net::SocketAddr::new"), None);
        // fs_err is a std::fs drop-in — wholesale Fs.
        assert_eq!(classify("fs_err", "fs_err::read_to_string"), Some("Fs"));
        assert_eq!(classify("fs_err", "fs_err::File::open"), Some("Fs"));
        // tempfile: create/persist verbs touch disk; Builder setters stay pure.
        assert_eq!(classify("tempfile", "tempfile::tempfile"), Some("Fs"));
        assert_eq!(classify("tempfile", "tempfile::NamedTempFile::new"), Some("Fs"));
        assert_eq!(classify("tempfile", "tempfile::NamedTempFile::persist"), Some("Fs"));
        assert_eq!(classify("tempfile", "tempfile::Builder::prefix"), None);
        // glob walks the filesystem; Pattern matching is pure.
        assert_eq!(classify("glob", "glob::glob"), Some("Fs"));
        assert_eq!(classify("glob", "glob::Pattern::matches"), None);
        // duct: run/read/start execute; cmd() only builds.
        assert_eq!(classify("duct", "duct::Expression::run"), Some("Exec"));
        assert_eq!(classify("duct", "duct::Expression::read"), Some("Exec"));
        assert_eq!(classify("duct", "duct::cmd"), None);
        // dotenvy/dotenv load env (file read + process-env mutation).
        assert_eq!(classify("dotenvy", "dotenvy::dotenv"), Some("Env"));
        assert_eq!(classify("dotenvy", "dotenvy::from_filename"), Some("Env"));
        assert_eq!(classify("dotenv", "dotenv::dotenv"), Some("Env"));
        // the `time` crate's clock reads (distinct from std::time / chrono).
        assert_eq!(classify("time", "time::OffsetDateTime::now_utc"), Some("Clock"));
        assert_eq!(classify("time", "time::OffsetDateTime::now_local"), Some("Clock"));
        assert_eq!(classify("time", "time::Instant::now"), Some("Clock"));
        assert_eq!(classify("time", "time::Duration::seconds"), None);
    }

    #[test]
    fn db_crates_are_calibrated() {
        // The emitted calibrated set must cover every DB client the classifier knows,
        // or the receipt's coverage check would flag a recognized crate as a blind spot.
        for c in DB_CRATES {
            assert!(
                CALIBRATED_CRATES.contains(&c),
                "DB crate `{c}` is matched by classify() but missing from CALIBRATED_CRATES"
            );
        }
    }

    #[test]
    fn calibrated_crates_are_live() {
        // Conversely, every crate we advertise as calibrated must actually be matched by
        // classify() for some representative path — a dead entry would silently suppress a
        // real coverage warning.
        for c in CALIBRATED_CRATES {
            let probes = [
                format!("{c}::X::send"),
                format!("{c}::X::execute"),
                format!("{c}::X::call"),
                format!("{c}::X::query"),
                format!("{c}::X::fetch_one"),
                format!("{c}::Remote::fetch"),
                format!("{c}::X::connect"),
                format!("{c}::Utc::now"),
                format!("{c}::X::load"),
                format!("{c}::__private_api::log"),
                format!("{c}::tempfile"),    // tempfile
                format!("{c}::glob"),        // glob
                format!("{c}::X::run"),      // duct
                format!("{c}::dotenv"),      // dotenvy / dotenv
                format!("{c}::random"),      // rand (verb-gated)
                format!("{c}::X::anything"),
            ];
            assert!(
                probes.iter().any(|p| classify(c, p).is_some()),
                "calibrated crate `{c}` is matched by no path in classify() — dead list entry"
            );
        }
    }

    #[test]
    fn capstd_capabilities_and_ops() {
        // Capability TYPES (declared by holding them).
        assert_eq!(capstd_cap("cap_std", "Dir"), Some("Fs"));
        assert_eq!(capstd_cap("cap_primitives", "Dir"), Some("Fs"));
        assert_eq!(capstd_cap("cap_std", "Pool"), Some("Net"));
        assert_eq!(capstd_cap("cap_std", "SystemClock"), Some("Clock"));
        assert_eq!(capstd_cap("cap_std", "UnixStream"), Some("Ipc"));
        assert_eq!(capstd_cap("std", "Dir"), None); // only cap-std types count
        // Capability OPERATIONS (the effect, via classify).
        assert_eq!(classify("cap_std", "cap_std::fs::Dir::open"), Some("Fs"));
        assert_eq!(classify("cap_primitives", "cap_primitives::fs::Dir::read_to_string"), Some("Fs"));
        assert_eq!(classify("cap_std", "cap_std::net::Pool::connect"), Some("Net"));
        assert_eq!(classify("cap_std", "cap_std::time::SystemClock::now"), Some("Clock"));
    }

    #[test]
    fn classify_other_effects_precisely() {
        assert_eq!(classify("std", "std::fs::read_to_string"), Some("Fs"));
        assert_eq!(classify("tokio", "tokio::fs::read"), Some("Fs"));
        // Exec is subprocess spawning — not std::process::exit / Stdio / ExitStatus.
        assert_eq!(classify("std", "std::process::Command::new"), Some("Exec"));
        assert_eq!(classify("std", "std::process::Child::wait"), Some("Exec"));
        assert_eq!(classify("std", "std::process::exit"), None);
        // tokio::process is the async mirror — spawning through it is Exec too.
        assert_eq!(classify("tokio", "tokio::process::Command::spawn"), Some("Exec"));
        assert_eq!(classify("tokio", "tokio::process::Child::wait"), Some("Exec"));
        assert_eq!(classify("std", "std::env::var"), Some("Env"));
        assert_eq!(classify("chrono", "chrono::Utc::now"), Some("Clock"));
        assert_eq!(classify("std", "std::time::SystemTime::now"), Some("Clock"));
        assert_eq!(classify("std", "std::time::Instant::now"), Some("Clock"));
        // ...but only the `now` accessor — not any path containing "now" / other clock fns.
        assert_eq!(classify("chrono", "chrono::Duration::num_days"), None);
        assert_eq!(classify("std", "std::time::Instant::elapsed"), None);
        assert_eq!(classify("getrandom", "getrandom::getrandom"), Some("Rand"));
        assert_eq!(classify("tracing", "tracing::event"), Some("Log"));
        assert_eq!(classify("arboard", "arboard::Clipboard::set_text"), Some("Clipboard"));
        // Unrelated crates are pure.
        assert_eq!(classify("serde", "serde::Serialize::serialize"), None);
        assert_eq!(classify("std", "std::vec::Vec::push"), None);
    }

    #[test]
    fn effectful_std_traits_are_io_only() {
        // Generic dispatch over these hides real I/O → Unknown (not assumed pure).
        assert!(is_effectful_std_trait("std::io::Write"));
        assert!(is_effectful_std_trait("std::io::Read"));
        assert!(is_effectful_std_trait("std::io::BufRead"));
        assert!(is_effectful_std_trait("std::io::Seek"));
        // core::fmt::Write is PURE formatting — must NOT be confused with std::io::Write.
        assert!(!is_effectful_std_trait("core::fmt::Write"));
        // Conventionally-pure traits stay pure.
        assert!(!is_effectful_std_trait("std::iter::Iterator"));
        assert!(!is_effectful_std_trait("serde::Serialize"));
    }

    #[test]
    fn net_host_literal_extracts_endpoints() {
        // URLs → bare host (scheme + path stripped), host:port kept, userinfo dropped.
        assert_eq!(net_host_literal("https://api.example.com/v1/x"), Some("api.example.com".into()));
        assert_eq!(net_host_literal("http://api.example.com:8443/x"), Some("api.example.com:8443".into()));
        assert_eq!(net_host_literal("rates.internal:7070"), Some("rates.internal:7070".into()));
        assert_eq!(net_host_literal("1.2.3.4:80"), Some("1.2.3.4:80".into()));
        assert_eq!(net_host_literal("user:pass@db.internal:5432"), Some("db.internal:5432".into()));
        // Non-host strings (an HTTP verb, a header value, a path) are NOT mistaken for hosts.
        assert_eq!(net_host_literal("GET"), None);
        assert_eq!(net_host_literal("application/json"), None);
        assert_eq!(net_host_literal("hello world"), None);
        assert_eq!(net_host_literal(""), None);
    }

    #[test]
    fn fs_kind_classifies_read_write() {
        // reads
        assert_eq!(fs_kind("std::fs::read_to_string"), &["read"][..]);
        assert_eq!(fs_kind("std::fs::File::open"), &["read"][..]);
        assert_eq!(fs_kind("std::fs::metadata"), &["read"][..]);
        assert_eq!(fs_kind("std::fs::read_dir"), &["read"][..]);
        // writes
        assert_eq!(fs_kind("std::fs::write"), &["write"][..]);
        assert_eq!(fs_kind("std::fs::File::create"), &["write"][..]);
        assert_eq!(fs_kind("std::fs::remove_dir_all"), &["write"][..]);
        assert_eq!(fs_kind("std::fs::rename"), &["write"][..]);
        // copy touches both ends
        assert_eq!(fs_kind("std::fs::copy"), &["read", "write"][..]);
        // direction we can't know from the verb → no claim (honest), even though classify says Fs.
        assert!(fs_kind("std::fs::OpenOptions::open").is_empty());
        assert!(fs_kind("memmap2::MmapOptions::map").is_empty());
        assert!(fs_kind("cap_std::fs::Dir::entries").is_empty());
    }

    #[test]
    fn fs_detail_only_for_builtin_fs() {
        // Built-in Fs classification → the verb table applies.
        assert_eq!(fs_detail_for(Some("Fs"), "std::fs::write"), &["write"][..]);
        assert_eq!(fs_detail_for(Some("Fs"), "std::fs::read_to_string"), &["read"][..]);
        // A user `extra` rule labels the crate Fs (builtin is None) and the leaf collides with a
        // write verb — but we must NOT claim a kind for it (the regression: false `Fs(write)`).
        assert!(fs_detail_for(None, "mybuf::Builder::append").is_empty());
        assert!(fs_detail_for(None, "std::fs::write").is_empty());
        // A different built-in effect never carries fs detail.
        assert!(fs_detail_for(Some("Net"), "std::fs::write").is_empty());
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
    fn pure_std_traits() {
        // dyn dispatch over these is treated as pure (no Unknown flood) — found in the
        // wild: `dyn Error` formatting was tainting whole call trees.
        assert!(is_pure_std_trait("std", "Error"));
        assert!(is_pure_std_trait("core", "Display"));
        assert!(is_pure_std_trait("alloc", "ToString"));
        assert!(is_pure_std_trait("core", "Clone"));
        // ...but traits where dispatch can genuinely hide I/O stay Unknown.
        assert!(!is_pure_std_trait("std", "Iterator"));
        assert!(!is_pure_std_trait("std", "Write"));
        assert!(!is_pure_std_trait("std", "Drop"));
        // ...and a same-named trait from a non-std crate is not whitelisted.
        assert!(!is_pure_std_trait("mycrate", "Display"));
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
        // Malformed JSON yields None too — a corrupt baseline must never crash the build.
        std::fs::write(&path, "not valid json {[").unwrap();
        assert!(load_baseline(path.to_str().unwrap()).is_none());
        let _ = std::fs::remove_file(&path);
    }

    // (report_entries / report_version / round-trip are tested in the `candor-report` crate.)

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

    #[test]
    fn dph_hex_round_trips_and_rejects_garbage() {
        // The 32-hex on-disk form (`{a:016x}{b:016x}`, the body of dph_hex) must round-trip
        // through parse_dph — this is the cross-crate key contract between writer and reader.
        for (a, b) in [(0u64, 0u64), (1, 2), (0x0123_4567_89ab_cdef, 0xfedc_ba98_7654_3210), (u64::MAX, u64::MAX)] {
            let hex = format!("{a:016x}{b:016x}");
            assert_eq!(hex.len(), 32);
            assert_eq!(parse_dph(&hex), Some((a, b)));
        }
        assert_eq!(parse_dph(""), None);
        assert_eq!(parse_dph("tooshort"), None);
        assert_eq!(parse_dph(&"0".repeat(33)), None); // wrong length
        assert_eq!(parse_dph("zz234567_89abcdef0123456789abcde"), None); // 32 chars, non-hex
    }

    #[test]
    fn classify_precision_residuals() {
        // `Env` is the WHOLE `std::env` (a documented breadth, CRITIQUE §9) — current_dir counts.
        assert_eq!(classify("std", "std::env::current_dir"), Some("Env"));
        assert_eq!(classify("std", "std::env::vars"), Some("Env"));
        // HTTP clients beyond reqwest: only the dispatch, not builders.
        assert_eq!(classify("isahc", "isahc::Request::send"), Some("Net"));
        assert_eq!(classify("ureq", "ureq::Request::call"), Some("Net"));
        assert_eq!(classify("ureq", "ureq::Request::set"), None);
        // Randomness family + pty subprocess + mmap.
        assert_eq!(classify("rand", "rand::random"), Some("Rand"));
        // `rand` is verb-gated: entropy/generation calls are Rand, pure constructors are NOT.
        assert_eq!(classify("rand", "rand::Rng::gen_range"), Some("Rand"));
        assert_eq!(classify("rand", "rand::rngs::OsRng::next_u32"), Some("Rand"));
        assert_eq!(classify("rand", "rand::thread_rng"), Some("Rand"));
        assert_eq!(classify("rand", "rand::distributions::Uniform::new"), None); // pure constructor
        assert_eq!(classify("rand", "rand::rngs::StdRng::seed_from_u64"), None); // deterministic seed
        assert_eq!(classify("rand", "rand::distributions::Distribution::sample"), Some("Rand"));
        assert_eq!(classify("fastrand", "fastrand::u32"), Some("Rand"));
        assert_eq!(classify("portable_pty", "portable_pty::native_pty_system"), Some("Exec"));
        assert_eq!(classify("memmap2", "memmap2::Mmap::flush"), Some("Fs"));
        // The honest default: an unrecognized crate is pure (the coverage net warns separately).
        assert_eq!(classify("some_random_crate", "some_random_crate::foo::bar"), None);
    }

    #[test]
    fn conformance_decisions() {
        let set = |xs: &[&'static str]| xs.iter().copied().collect::<BTreeSet<&str>>();

        // AS-EFF-001 (undeclared): performs Net+Fs, declares only Net -> {Fs}; Unknown never counts.
        assert_eq!(undeclared_effects(&set(&["Net", "Fs", "Unknown"]), &set(&["Net"])), vec!["Fs"]);
        assert!(undeclared_effects(&set(&["Net"]), &set(&["Net", "Fs"])).is_empty());
        // Unknown alone is never an AS-EFF-001 (it's AS-EFF-003).
        assert!(undeclared_effects(&set(&["Unknown"]), &set(&[])).is_empty());

        // AS-EFF-002 (overdeclared): declares Fs but never performs it -> {Fs}.
        assert_eq!(overdeclared_effects(&set(&["Net", "Fs"]), &set(&["Net"])), vec!["Fs"]);
        assert!(overdeclared_effects(&set(&["Net"]), &set(&["Net", "Db"])).is_empty());

        // AS-EFF-004 (ambient): direct Net/Fs are ambient authority; Log is NOT (not in AMBIENT).
        assert_eq!(ambient_effects(&set(&["Net", "Log", "Fs"])), vec!["Fs", "Net"]);
        assert!(ambient_effects(&set(&["Log"])).is_empty());

        // AS-EFF-005 (gained vs baseline): only NEW effects fire; fewer effects never "gains".
        let baseline: BTreeSet<String> = ["Net".to_string()].into_iter().collect();
        assert_eq!(gained_effects(&set(&["Net", "Db"]), &baseline), vec!["Db"]);
        assert!(gained_effects(&set(&["Net"]), &baseline).is_empty());
        assert!(gained_effects(&set(&[]), &baseline).is_empty());
    }

    #[test]
    fn policy_parses() {
        let rules = parse_policy(
            "# the domain layer must stay pure of I/O\n\
             deny Net Db  domain\n\
             deny Exec\n\
             pure  parse\n\
             nonsense line\n\
             deny notaneffect\n",
        );
        // 3 valid rules; the unknown-kind line and the no-known-effect `deny` are dropped.
        assert_eq!(rules.len(), 3);
        // `deny Net Db domain` → {Db, Net} scoped to "domain"
        assert_eq!(rules[0].effects, ["Db", "Net"].into_iter().collect::<BTreeSet<_>>());
        assert_eq!(rules[0].scope.as_deref(), Some("domain"));
        // `deny Exec` → {Exec}, whole crate
        assert!(rules[1].effects.contains("Exec") && rules[1].scope.is_none());
        // `pure parse` → empty effect set (means "any effect"), scoped to "parse"
        assert!(rules[2].effects.is_empty() && rules[2].scope.as_deref() == Some("parse"));
        // `Unknown` is a denyable token; a bare `deny` with no effect is ignored.
        assert_eq!(parse_policy("deny Unknown core")[0].effects, ["Unknown"].into_iter().collect());
        assert!(parse_policy("deny\ndeny   \n").is_empty());
    }

    #[test]
    fn reportable_items() {
        // Functions and consts/statics are reported; type/module/closure defs are not.
        assert!(is_reportable_item(DefKind::Fn));
        assert!(is_reportable_item(DefKind::AssocFn));
        assert!(!is_reportable_item(DefKind::Struct));
        assert!(!is_reportable_item(DefKind::Enum));
        assert!(!is_reportable_item(DefKind::Trait));
        assert!(!is_reportable_item(DefKind::Mod));
        assert!(!is_reportable_item(DefKind::Closure));
    }

    #[test]
    fn load_cross_reports_filters_and_maps() {
        let dir = std::env::temp_dir().join("candor_cross_unit");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let prefix = dir.join("r");
        let prefix = prefix.to_str().unwrap();
        let h_lib = "0123456789abcdef0123456789abcdef";
        let h_own = "1111111111111111aaaaaaaaaaaaaaaa";
        let w = |name: &str, body: String| std::fs::write(dir.join(name), body).unwrap();

        // A dependency report — SHOULD load (only the entry with a valid hash + non-empty effects).
        w(
            "r.dep.Rlib.json",
            format!(
                r#"[{{"fn":"dep::f","inferred":["Net","Fs"],"hash":"{h_lib}"}},
                    {{"fn":"dep::pure","inferred":[],"hash":"{h_lib}"}},
                    {{"fn":"dep::old","inferred":["Db"]}}]"#
            ),
        );
        // Our OWN report (me = mybin/Executable) — MUST be skipped (same crate name, our type).
        w("r.mybin.Executable.json", format!(r#"[{{"fn":"own","inferred":["Exec"],"hash":"{h_own}"}}]"#));
        // Sidecars (one middle segment, or a dotted token) — MUST be skipped.
        w("r.calibrated.json", r#"{"crates":[]}"#.to_string());
        w("r.encountered-dep-Rlib.json", r#"["serde"]"#.to_string());

        let m = load_cross_reports(prefix, "mybin", "Executable", false);

        // dep::f loaded under its parsed-hash key with both effects.
        let mut got = m.get(&parse_dph(h_lib).unwrap()).cloned().unwrap_or_default();
        got.sort();
        assert_eq!(got, vec!["Fs", "Net"]);
        // dep::pure (empty effects) and dep::old (no hash) are dropped; own report is skipped.
        assert!(m.get(&parse_dph(h_own).unwrap()).is_none(), "own report must be skipped");
        assert_eq!(m.len(), 1, "only the one dependency entry with hash + effects should load");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn cross_report_version_mismatch_downgrades_to_unknown() {
        // report_version reads the v0.2 envelope header; a legacy bare array has none.
        assert_eq!(report_version(r#"{"candor":{"version":"v9"},"functions":[]}"#).as_deref(), Some("v9"));
        assert!(report_version("[]").is_none());

        let dir = std::env::temp_dir().join("candor_cross_ver_unit");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let prefix = dir.join("r");
        let prefix = prefix.to_str().unwrap();
        let h = "0123456789abcdef0123456789abcdef";
        let w = |body: String| std::fs::write(dir.join("r.dep.Rlib.json"), body).unwrap();

        // A v0.2 report from a DIFFERENT engine → inherited effects downgraded to Unknown (§2.1).
        w(format!(r#"{{"candor":{{"version":"OTHER"}},"functions":[{{"fn":"dep::f","inferred":["Net","Fs"],"hash":"{h}"}}]}}"#));
        let m = load_cross_reports(prefix, "mybin", "Executable", false);
        assert_eq!(m.get(&parse_dph(h).unwrap()).cloned().unwrap_or_default(), vec![UNKNOWN]);

        // The SAME engine version → trusted as-is.
        w(format!(r#"{{"candor":{{"version":"{CANDOR_VERSION}"}},"functions":[{{"fn":"dep::f","inferred":["Net","Fs"],"hash":"{h}"}}]}}"#));
        let m = load_cross_reports(prefix, "mybin", "Executable", false);
        let mut got = m.get(&parse_dph(h).unwrap()).cloned().unwrap_or_default();
        got.sort();
        assert_eq!(got, vec!["Fs", "Net"]);

        // Guard mode (trust_siblings=true): an OTHER-version report is the baseline's own snapshot, so
        // it is trusted as-is — NOT downgraded. (Otherwise the guard fires AS-EFF-005 every time the
        // engine moves ahead of the baseline it is comparing against.)
        w(format!(r#"{{"candor":{{"version":"OTHER"}},"functions":[{{"fn":"dep::f","inferred":["Net","Fs"],"hash":"{h}"}}]}}"#));
        let m = load_cross_reports(prefix, "mybin", "Executable", true);
        let mut got = m.get(&parse_dph(h).unwrap()).cloned().unwrap_or_default();
        got.sort();
        assert_eq!(got, vec!["Fs", "Net"]);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
