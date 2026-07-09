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
// its own crate/path → effect rules via a CANDOR_RULES file (see `parse_rules`).

extern crate rustc_ast;
extern crate rustc_errors;
extern crate rustc_hir;
extern crate rustc_middle;
extern crate rustc_span;

use std::collections::{BTreeSet, HashMap, HashSet};

use candor_report::{
    report_entries, report_files, report_has_envelope, report_version, ReportEntry, ReportMeta,
};
// The curated effect classifier lives in a STABLE crate (candor-classify), shared with the stable
// `candor-scan` backend so there is one source of truth (no drift). (`DB_CRATES` is referenced only
// by a consistency test, qualified at the use site.)
use candor_classify::{
    cap_from_name, capstd_cap, classify, classify_command_head, classify_extra,
    is_cmd_naming_method, CALIBRATED_CRATES, CALIBRATED_PREFIXES, PATH_CALIBRATED_CRATES,
};
// The CANDOR_POLICY DSL parser is the SHARED canonical one (candor-spec SPEC §6.2), so the nightly
// gate, stable candor-query (whatif/parsepolicy), and the JVM engine can't drift on the grammar.
use candor_classify::policy::{
    literal_allowed, parse_policy, scope_matches, AllowRule, LayerRule, ParsedPolicy, PolicyRule,
};

use rustc_hir::def::DefKind;
use rustc_hir::def_id::{DefId, LocalDefId};
use rustc_hir::{Expr, ExprKind, HirId};
use rustc_lint::{LateContext, LateLintPass};
use rustc_middle::ty::TyCtxt;

mod mir_spike;

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
    use rustc_errors::{Diag, DiagCtxtHandle, DiagMessage, Diagnostic, Level, MultiSpan};
    // Store the message + span DIRECTLY rather than a decorate closure. clippy's `span_lint` carries a
    // closure because it's the generic base of an *extensible* family (`span_lint_and_then`); candor
    // only ever sets the message + span, so the closure is pure overhead — and a closure stored in a
    // struct field is exactly what candor can't see through (it was candor's one self-`Unknown`). The
    // direct form is simpler AND analyzable: dropping an abstraction candor didn't need, not contorting
    // code to please the analyzer.
    struct CandorDiag {
        sp: MultiSpan,
        msg: DiagMessage,
    }
    impl<'a> Diagnostic<'a, ()> for CandorDiag {
        fn into_diag(self, dcx: DiagCtxtHandle<'a>, level: Level) -> Diag<'a, ()> {
            let mut diag = Diag::new(dcx, level, "");
            diag.primary_message(self.msg);
            diag.span(self.sp);
            diag
        }
    }
    let sp = sp.into();
    cx.emit_span_lint(lint, sp.clone(), CandorDiag { sp, msg: msg.into() });
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
    /// External crates a function calls DIRECTLY that κ neither classifies nor resolves into (the floor:
    /// a non-std external call whose `classify` returned None — candor cannot see through it). Propagated
    /// like `net_hosts_direct`; surfaced as the report's `invisible` detail so `inferred: []` is never an
    /// unqualified "pure" claim PER FUNCTION (the deep engine floors unmodeled crates like the syntactic
    /// engines; this discloses it — the honesty contract, sweep [4]/[19]).
    invisible_direct: HashMap<LocalDefId, BTreeSet<String>>,
    /// Effects whose literal SURFACE this fn leaves incomplete: a host-establishing Net call / a cmd-naming
    /// Exec call / a path-naming Fs call / a query-bearing Db call performed with a RUNTIME (non-literal)
    /// locator, so the endpoint is structurally invisible to the gate. Propagated like effects; the
    /// AS-EFF-008 allowlist fails CLOSED on an incomplete surface even when benign literals are present
    /// (else the benign literal masks the runtime endpoint — sweep [3]/[7]). All four locator-bearing
    /// effects (Net/Exec/Fs/Db), each via its establishing-allowlist predicate (is_net_establishing /
    /// is_cmd_naming_method / is_fs_path_arg / is_db_query_arg), matching candor-scan + candor-java.
    incomplete_direct: HashMap<LocalDefId, BTreeSet<&'static str>>,
    /// Literal subprocess commands a function runs directly (the program in `Command::new("git")`).
    /// Propagated like `net_hosts_direct`; surfaced as the report's optional `cmds` detail and enforced
    /// by `allow Exec …` (AS-EFF-008). Static-literal subset only — a runtime command is simply absent.
    exec_cmds_direct: HashMap<LocalDefId, BTreeSet<String>>,
    /// Literal filesystem paths a function touches directly (the path in a built-in `Fs` call).
    /// Propagated like `net_hosts_direct`; surfaced as the report's optional `paths` detail and enforced
    /// by `allow Fs …` (AS-EFF-008). Static-literal subset only — a runtime path is simply absent.
    fs_paths_direct: HashMap<LocalDefId, BTreeSet<String>>,
    /// Literal database tables a function touches directly (table-position identifiers in a SQL string
    /// literal at a built-in `Db` call). Propagated like `net_hosts_direct`; surfaced as the report's
    /// optional `tables` detail and enforced by `allow Db …` (AS-EFF-008). Static-literal subset only —
    /// a dynamically-built query (or an ORM call carrying no SQL) is simply absent.
    db_tables_direct: HashMap<LocalDefId, BTreeSet<String>>,
    /// Local-crate functions each function calls, for transitive propagation.
    calls: HashMap<LocalDefId, HashSet<LocalDefId>>,
    /// Closure-flow, *receiving* side (bounded). Per FREE fn, the parameter indices it INVOKES as a
    /// callback (`fn apply(f: impl Fn()) { f() }` → {0}). The honest `Unknown` for such a call is
    /// DEFERRED, not inserted on the spot — resolved PER CALL SITE in `check_crate_post` from
    /// `callback_sites` / `callback_site_unknown`: each callback's effects flow to the SPECIFIC caller
    /// that passed it (not unioned onto the HOF), while the HOF keeps a non-propagating `Unknown` in its
    /// own report (the honest standalone answer for invoking an opaque param). Free fns only, so arg index equals param
    /// index (a method's `self` would offset it).
    param_calls: HashMap<LocalDefId, BTreeSet<usize>>,
    /// A fn that drives an iterator-family trait method (`Iterator`/`IntoIterator`/`Sum`/…) on one of its
    /// OWN generic params (`fn run<I: Iterator>(it: I) { it.for_each(..) }`). Standalone it can't pin the
    /// concrete `I::next` (could be a LOCAL effectful iterator), so it carries a REPORT-ONLY honest
    /// `Unknown` (`generic-iter:<method>`), injected AFTER the fixpoint — exactly the HOF-param model:
    /// honest standalone, but NON-propagating so it never re-pollutes the precise local callers that
    /// monomorphized it (`caller`, resolved via `generic_callee_local_edges`). Maps to the why-tag.
    /// Scoped to iter-driver traits ONLY — a `Clone`/`Display` generic dispatch is pure-std (no flood).
    generic_iter_unknown: HashMap<LocalDefId, String>,
    /// PER-CALL-SITE closure flow: for each *calling* fn
    /// that passes a LOCAL named fn at argument `i` of HOF `F`, the targets it passed THERE. Lets the
    /// callback's effects flow to the SPECIFIC caller that passed it (`handler_io -> fetch_remote`),
    /// instead of being unioned into `F` and then leaking to EVERY caller of `F` (the fabrication that
    /// made a pure caller passing a pure callback inherit a sibling caller's effectful one). Keyed by
    /// (caller, F, param) — caller is `LocalDefId` (always local), F is `DefId`. Free-fn HOFs only, so
    /// arg index == param index.
    callback_sites: HashMap<(LocalDefId, DefId, usize), HashSet<DefId>>,
    /// PER-CALL-SITE unresolvable callbacks: (caller, F, param) where THIS caller passed an
    /// unresolvable callback (closure / fn-ptr / non-local fn / generic value) at `F`'s invoked param.
    /// The honest `Unknown` is attributed to THAT caller (it genuinely can't see what it passed),
    /// not unioned onto `F`.
    callback_site_unknown: HashSet<(LocalDefId, DefId, usize)>,
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
    /// Net *hosts* a sibling crate's function reaches, keyed by its stable `DefPathHash` — the host
    /// detail from its report. Lets the host allowlist (AS-EFF-008) see an endpoint that lives across
    /// the crate boundary, so "billing may only reach Stripe" holds even when the actual `connect` is
    /// in a shared lib. Empty unless a cross report carried `hosts`.
    cross_hosts: HashMap<(u64, u64), BTreeSet<String>>,
    /// Subprocess commands a sibling crate's function runs, keyed by `DefPathHash` (its report's `cmds`).
    /// Lets `allow Exec …` see a command that lives across the crate boundary. Empty unless carried.
    cross_cmds: HashMap<(u64, u64), BTreeSet<String>>,
    /// Filesystem paths a sibling crate's function touches, keyed by `DefPathHash` (its report's
    /// `paths`). Lets `allow Fs …` see a path that lives across the crate boundary. Empty unless carried.
    cross_paths: HashMap<(u64, u64), BTreeSet<String>>,
    /// Database tables a sibling crate's function touches, keyed by `DefPathHash` (its report's
    /// `tables`). Lets `allow Db …` see a table that lives across the crate boundary. Empty unless carried.
    cross_tables: HashMap<(u64, u64), BTreeSet<String>>,
    /// CANDOR_EXPLAIN=<query>: when set, record where each effect enters (the call + location) so
    /// `cargo candor explain` can trace the path from a function to the source of each effect.
    explain: Option<String>,
    /// Per-function effect *sites*: the calls in a body that introduce an effect (a classified leaf,
    /// a cross-crate inheritance, or an unresolvable call). Populated only in explain mode.
    sites: HashMap<LocalDefId, Vec<EffectSite>>,
    /// CANDOR_POLICY: declared effect-boundary rules to enforce (AS-EFF-006).
    policy: Vec<PolicyRule>,
    /// CANDOR_POLICY: declared literal-allowlist rules to enforce (AS-EFF-008: Net hosts / Exec commands
    /// / Fs paths). Parsed from the same file.
    allow_rules: Vec<AllowRule>,
    /// CANDOR_POLICY: declared module-layering rules to enforce (AS-EFF-009). Parsed from the same file.
    layer_rules: Vec<LayerRule>,
    /// Cross-crate callees of each local function: `caller -> [(callee DefPathHash, callee path)]`,
    /// recorded for layering (AS-EFF-009) when any `forbid` rule is present. The path lets us match a
    /// `to` scope (a direct dependency on another crate); the hash lets us chain to that callee's own
    /// `layerreach` summary (a dependency *laundered through* that crate). Empty without `forbid` rules.
    cross_callees: HashMap<LocalDefId, Vec<((u64, u64), String)>>,
    /// Per sibling-crate function (`DefPathHash`), the set of `forbid` *target* scopes it transitively
    /// reaches — loaded from that crate's `layerreach` sidecar (written during this same enforce pass,
    /// crates linted dependency-first). This is what makes layering follow a dependency through a THIRD
    /// crate: `app -> util -> infra` is caught because `util`'s sidecar records that `util::f` reaches
    /// `infra`. Empty unless layering sidecars are present (workspace enforce mode).
    cross_layer_reach: HashMap<(u64, u64), BTreeSet<String>>,
    /// The report prefix (`CANDOR_REPORTS`/`CANDOR_JSON`/`CANDOR_BASELINE`) this run resolves siblings
    /// against — also where the `layerreach` sidecar is written, so dependent crates in the same enforce
    /// pass can read it. `None` when no prefix is set (single-crate run).
    reports_prefix: Option<String>,
    /// CANDOR_TAINT: flag effects whose argument derives from a function parameter (AS-EFF-007).
    taint: bool,
    /// Per-function effects performed on a parameter-derived (caller-controlled) argument.
    tainted: HashMap<LocalDefId, BTreeSet<&'static str>>,
    /// Why each function introduces `Unknown` DIRECTLY: an origin tag per unresolvable site
    /// (`dispatch:<trait>`, `callback:<fn-ptr / closure>`). Always populated (cheap, unlike `sites`
    /// which is explain-only) so the report's `unknownWhy` can tell the improvable opacity (a dispatch
    /// that would resolve with more inputs) from the irreducible — see candor-spec §2.
    unknown_why: HashMap<LocalDefId, BTreeSet<String>>,
    /// CANDOR_VIOLATIONS: a sentinel file the engine APPENDS one line to per ENFORCEMENT violation
    /// (the baseline gain AS-EFF-005 and the policy gates AS-EFF-006/008/009). This is the
    /// MACHINE-READABLE verdict the `cargo-candor` wrapper consumes (file non-empty ⇒ exit 1),
    /// instead of grepping the human diagnostic text for `AS-EFF-…` tokens — a reword or a dylint
    /// output-stream change can drop the literal token and silently turn the gate green, but it
    /// cannot stop the sentinel write. The text diagnostics are still emitted for humans. Opened in
    /// append mode so every crate in a one-pass workspace `cargo dylint` accumulates into the one
    /// file (the wrapper truncates/creates it fresh before the run). `None` ⇒ no sentinel (audit,
    /// JSON, and any run where the wrapper didn't ask for one — a no-op, never a write).
    violations_sink: Option<String>,
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
        // A *set-but-unreadable* CANDOR_RULES must be loud: silently ignoring it would
        // make the user believe their crates are covered when they aren't.
        // (Renamed from CANDOR_CONFIG — no fallback: that name now means the spec-§3.4
        // `.candor/config` override path family-wide, and one variable meaning two
        // incompatible things is worse than a clean break.)
        let extra = match std::env::var("CANDOR_RULES") {
            Ok(p) => match std::fs::read_to_string(&p) {
                Ok(s) => parse_rules(&s),
                Err(e) => {
                    eprintln!("candor: CANDOR_RULES={p:?} could not be read ({e}); ignoring it");
                    Vec::new()
                }
            },
            Err(_) => Vec::new(),
        };
        let paranoid = std::env::var("CANDOR_PARANOID").is_ok();
        let explain = std::env::var("CANDOR_EXPLAIN").ok().filter(|s| !s.is_empty());
        // A set-but-unreadable CANDOR_POLICY FAILS the run (spec §6.2 MUST): proceeding gateless on
        // a typo'd path is a gate that silently passes everything. Exiting the compiler process is
        // blunt, but it is the only path here that makes `cargo dylint` exit nonzero — a stderr
        // line alone left CI green (the java engine shipped exactly that bug).
        let parsed_policy = match std::env::var("CANDOR_POLICY") {
            Ok(p) => match std::fs::read_to_string(&p) {
                Ok(s) => parse_policy(&s),
                Err(e) => {
                    eprintln!("candor: CANDOR_POLICY={p:?} could not be read ({e}) — failing (exit 2), policy NOT evaluated");
                    std::process::exit(2);
                }
            },
            Err(_) => ParsedPolicy::default(),
        };
        let ParsedPolicy { rules: policy, allow_rules, layer_rules } = parsed_policy;
        Self {
            direct: HashMap::new(),
            fs_direct: HashMap::new(),
            net_hosts_direct: HashMap::new(),
            invisible_direct: HashMap::new(),
            incomplete_direct: HashMap::new(),
            exec_cmds_direct: HashMap::new(),
            fs_paths_direct: HashMap::new(),
            db_tables_direct: HashMap::new(),
            param_calls: HashMap::new(),
            generic_iter_unknown: HashMap::new(),
            callback_sites: HashMap::new(),
            callback_site_unknown: HashSet::new(),
            calls: HashMap::new(),
            extra,
            paranoid,
            encountered: BTreeSet::new(),
            cross: HashMap::new(),
            via_cross: HashMap::new(), // (cross map keyed by structured DefPathHash, not a string)
            cross_hosts: HashMap::new(),
            cross_cmds: HashMap::new(),
            cross_paths: HashMap::new(),
            cross_tables: HashMap::new(),
            explain,
            sites: HashMap::new(),
            policy,
            allow_rules,
            layer_rules,
            cross_callees: HashMap::new(),
            cross_layer_reach: HashMap::new(),
            reports_prefix: None,
            taint: std::env::var("CANDOR_TAINT").is_ok(),
            tainted: HashMap::new(),
            unknown_why: HashMap::new(),
            // The wrapper sets this to a fresh sentinel path before an enforcing run; absent/empty
            // means "no machine signal requested" (audit, JSON, or a direct `cargo dylint` invocation).
            violations_sink: std::env::var("CANDOR_VIOLATIONS").ok().filter(|s| !s.is_empty()),
        }
    }

    /// Append one line — `<code> <function>` — to the `CANDOR_VIOLATIONS` sentinel for an ENFORCEMENT
    /// violation, so the wrapper has a machine signal that doesn't depend on grepping the diagnostic
    /// prose. Append (not truncate) so multiple crates in a single workspace `cargo dylint` pass all
    /// land in the one file. A no-op when no sink is set. A write error is reported but non-fatal: the
    /// human diagnostic still fired, and failing the compile here would be a worse failure mode than a
    /// degraded signal (the wrapper additionally surfaces any AS-EFF text it sees).
    fn record_violation(&self, code: &str, func: &str) {
        let Some(path) = &self.violations_sink else { return };
        use std::io::Write;
        match std::fs::OpenOptions::new().create(true).append(true).open(path) {
            Ok(mut f) => {
                if let Err(e) = writeln!(f, "{code} {func}") {
                    eprintln!("candor: could not append to CANDOR_VIOLATIONS={path:?} ({e})");
                }
            }
            Err(e) => eprintln!("candor: could not open CANDOR_VIOLATIONS={path:?} ({e})"),
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

/// Parse a CANDOR_RULES file (classifier extensions): one rule per line,
/// `<Effect> <crate|path> <prefix>`, blank lines and `#` comments ignored. The effect must be one
/// of the known names.
///
///     # extend the classifier with this project's own effectful crates
///     Net   crate  reqwest
///     Fs    path   mycrate::storage::
fn parse_rules(text: &str) -> Vec<(&'static str, bool, String)> {
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
    // `get(..16)`/`get(16..)`, NOT `&s[..16]`: a 32-BYTE string can split a multi-byte UTF-8 char at
    // index 16 (a corrupt/hand-edited `hash` field), and slicing there panics — an ICE that aborts the
    // user's build. `get` returns None on a non-char-boundary, so we degrade to "unresolvable" instead.
    let (hi, lo) = (s.get(..16)?, s.get(16..)?);
    Some((u64::from_str_radix(hi, 16).ok()?, u64::from_str_radix(lo, 16).ok()?))
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
///
/// Everything a dependent crate inherits from its siblings' reports, keyed by `DefPathHash`: effects,
/// and the literal detail surfaces the allowlists need (Net hosts, Exec commands, Fs paths).
#[derive(Default)]
struct CrossData {
    effects: HashMap<(u64, u64), Vec<&'static str>>,
    hosts: HashMap<(u64, u64), BTreeSet<String>>,
    cmds: HashMap<(u64, u64), BTreeSet<String>>,
    paths: HashMap<(u64, u64), BTreeSet<String>>,
    tables: HashMap<(u64, u64), BTreeSet<String>>,
}

fn load_cross_reports(prefix: &str, me: &str, me_kind: &str, trust_siblings: bool) -> CrossData {
    let mut out: HashMap<(u64, u64), Vec<&'static str>> = HashMap::new();
    let mut hosts: HashMap<(u64, u64), BTreeSet<String>> = HashMap::new();
    let mut cmds: HashMap<(u64, u64), BTreeSet<String>> = HashMap::new();
    let mut paths: HashMap<(u64, u64), BTreeSet<String>> = HashMap::new();
    let mut tables: HashMap<(u64, u64), BTreeSet<String>> = HashMap::new();
    for rf in report_files(prefix) {
        // Skip our OWN report (by crate name AND type); DefPathHash keys are globally unique so
        // all other crates merge into one map. (Own entries are local defs and the cross path is
        // guarded by `!def_id.is_local()`, so loading them would be harmless — just wasteful.)
        if rf.krate == me && rf.kind == me_kind {
            continue;
        }
        // A sibling report `report_files` returned (so it IS a `<crate>.<kind>.json` report, not a
        // sidecar) that we then can't read or parse is a corrupt/partial write — fail LOUD, not silent.
        // Skipping it silently degrades cross-crate resolution: an effect that lives in that sibling
        // would be invisible to a caller here, so an enforcement run (guard/policy) could pass when it
        // should fire. Warn so the degradation is visible; resolution still proceeds for the rest.
        let text = match std::fs::read_to_string(&rf.path) {
            Ok(t) => t,
            Err(e) => {
                eprintln!(
                    "candor: sibling report {:?} could not be read ({e}) — its cross-crate effects are \
                     NOT loaded; an effect reaching it may be under-reported",
                    rf.path
                );
                continue;
            }
        };
        // Version-aware trust (candor-spec §2.1): a sibling report produced by a DIFFERENT engine
        // was computed by rules this engine may have changed, so we must not silently trust its
        // effects — downgrade everything inherited from it to `Unknown`. A legacy v0.1 report (a bare
        // array, no envelope) has no version and is trusted as documented. But an envelope WITHOUT a
        // parseable version is a partial write / corruption — NOT a v0.1 report — so don't trust it
        // either (treat as stale): a missing version where one is expected can't certify provenance.
        let stale = !trust_siblings
            && match report_version(&text) {
                Some(v) => v != CANDOR_VERSION,
                None => report_has_envelope(&text),
            };
        let Some(arr) = report_entries(&text) else {
            eprintln!(
                "candor: sibling report {:?} could not be parsed (corrupt/partial write?) — its \
                 cross-crate effects are NOT loaded; an effect reaching it may be under-reported",
                rf.path
            );
            continue;
        };
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
            // Carry the sibling's literal detail (hosts / commands / paths) so the allowlists can see
            // values that live across the crate boundary. Dropped when stale — a downgraded report's
            // literal claims are no more trustworthy than its effects.
            if !stale {
                if !e.hosts.is_empty() {
                    hosts.entry(key).or_default().extend(e.hosts.iter().cloned());
                }
                if !e.cmds.is_empty() {
                    cmds.entry(key).or_default().extend(e.cmds.iter().cloned());
                }
                if !e.paths.is_empty() {
                    paths.entry(key).or_default().extend(e.paths.iter().cloned());
                }
                if !e.tables.is_empty() {
                    tables.entry(key).or_default().extend(e.tables.iter().cloned());
                }
            }
        }
    }
    CrossData { effects: out, hosts, cmds, paths, tables }
}

/// The on-disk name of a crate's layering-reachability sidecar (`<prefix>.<crate>.<kind>.layerreach.json`).
/// A 3-segment name, so `report_files` (which wants exactly two) never mistakes it for an effect report.
fn layer_reach_path(prefix: &str, krate: &str, kinds: &str) -> String {
    format!("{prefix}.{krate}.{kinds}.layerreach.json")
}

/// Write this crate's `layerreach` sidecar: for each local function (by hex `DefPathHash`), the set of
/// `forbid`-target scopes it transitively reaches. Dependent crates load it (later in the same enforce
/// pass) so a dependency *laundered through* this crate is still caught (AS-EFF-009).
fn write_layer_reach(path: &str, reach: &HashMap<String, Vec<String>>) {
    if let Ok(body) = serde_json::to_string(reach) {
        let _ = std::fs::write(path, body);
    }
}

/// Load every `layerreach` sidecar under `prefix` into `DefPathHash -> reached target scopes`. These are
/// written by sibling crates earlier in the same enforce pass; merged across all of them.
fn load_layer_reach(prefix: &str) -> HashMap<(u64, u64), BTreeSet<String>> {
    let mut out: HashMap<(u64, u64), BTreeSet<String>> = HashMap::new();
    let p = std::path::Path::new(prefix);
    let dir = p.parent().filter(|d| !d.as_os_str().is_empty()).map(|d| d.to_path_buf()).unwrap_or_else(|| std::path::PathBuf::from("."));
    let Some(base) = p.file_name().and_then(|s| s.to_str()) else { return out };
    let prefix_dot = format!("{base}.");
    let Ok(rd) = std::fs::read_dir(&dir) else { return out };
    for ent in rd.flatten() {
        let name = ent.file_name();
        let Some(name) = name.to_str() else { continue };
        if !name.starts_with(&prefix_dot) || !name.ends_with(".layerreach.json") {
            continue;
        }
        let Ok(text) = std::fs::read_to_string(ent.path()) else { continue };
        let Ok(map) = serde_json::from_str::<HashMap<String, Vec<String>>>(&text) else { continue };
        for (hex, scopes) in map {
            if let Some(key) = parse_dph(&hex) {
                out.entry(key).or_default().extend(scopes);
            }
        }
    }
    out
}


/// Load a baseline candor JSON into `fn name -> inferred effect set`. Same-named entries (e.g. `main`
/// across an rlib+bin, or distinct monomorphizations sharing a path) are UNIONed, not last-write-wins:
/// the baseline is the over-approximation of what a name was already permitted to reach, so dropping
/// any colliding entry's effects would let a gained effect slip past the guard unflagged.
fn load_baseline(path: &str) -> Option<HashMap<String, BTreeSet<String>>> {
    let text = std::fs::read_to_string(path).ok()?;
    let entries = report_entries(&text)?;
    let mut out: HashMap<String, BTreeSet<String>> = HashMap::new();
    for e in entries {
        out.entry(e.func).or_default().extend(e.inferred);
    }
    Some(out)
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
/// Is a method-call receiver a trait object — i.e. is the dispatch dynamic (vtable, unresolvable)?
/// `peel_refs()` alone only strips `&`/`&mut`, so it misses an **arbitrary self type**:
/// `Arc<dyn Job>::run(self: Arc<Self>)` (common in actor / async-trait code). Walk through the
/// std smart pointers (`Box`/`Rc`/`Arc`/`Pin`, which carry their pointee in the first type arg) so a
/// `dyn` behind one is detected as dynamic — otherwise a non-local `dyn` call gets no honest `Unknown`.
fn is_dyn_receiver<'tcx>(tcx: TyCtxt<'tcx>, ty: rustc_middle::ty::Ty<'tcx>) -> bool {
    use rustc_middle::ty::TyKind;
    let mut ty = ty.peel_refs();
    for _ in 0..8 {
        // bounded against any pathological nesting
        match ty.kind() {
            TyKind::Dynamic(..) => return true,
            TyKind::Adt(adt, args)
                if matches!(tcx.item_name(adt.did()).as_str(), "Box" | "Rc" | "Arc" | "Pin") =>
            {
                match args.types().next() {
                    Some(inner) => ty = inner.peel_refs(),
                    None => return false,
                }
            }
            // An OPAQUE alias (`impl Trait` in return position) whose defining use is in THIS crate:
            // the hidden type is knowable — reveal it and keep walking. Without this, a method call on
            // an `impl Iterator` that secretly IS a `Box<dyn Iterator>` fell into the unresolvable-
            // generic-stays-pure calibration: no edge, no `Unknown` — a silent under-report through a
            // completely ordinary API shape (`which`'s `all_results().and_then(|mut i| i.next())`;
            // found by dogfooding the umbrella AGENTS.md route on the `which` crate).
            TyKind::Alias(alias) => {
                let rustc_middle::ty::AliasTyKind::Opaque { def_id } = alias.kind else {
                    return false;
                };
                if def_id.as_local().is_none() {
                    return false;
                }
                ty = tcx.type_of(def_id).instantiate(tcx, alias.args).skip_normalization().peel_refs();
            }
            _ => return false,
        }
    }
    false
}

fn resolve_callee<'tcx>(cx: &LateContext<'tcx>, expr: &Expr<'tcx>) -> Option<Callee> {
    use rustc_middle::ty::TyKind;
    // Defensive: `typeck_results()` panics (ICE) for an expr outside a typechecked
    // body. An effect checker must never abort the build, so bail gracefully instead.
    let typeck = cx.maybe_typeck_results()?;
    match expr.kind {
        ExprKind::MethodCall(_, receiver, _, _) => {
            let dynamic = is_dyn_receiver(cx.tcx, typeck.expr_ty_adjusted(receiver));
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
            // Any OTHER callee type of a Call is still a callable we can't see through — a `dyn Fn`
            // behind a smart pointer (`Box`/`Rc`/`Arc<dyn Fn>`), a `&dyn Fn`, or a user type with an
            // `Fn` impl. These MUST be `Unresolved` (→ `Unknown`), not `None`: returning `None` made
            // `check_expr` bail before recording anything, so a directly-invoked boxed callback was
            // silently assumed pure — a soundness under-report. (Closures stay `None`: counted lexically.)
            _ => Some(Callee::Unresolved),
        },
        // OVERLOADED OPERATORS desugar to a trait-method call but are NOT `Call`/`MethodCall` nodes:
        // `a + b` is `Binary`, `-a` / `*p` is `Unary`, `a[i]` is `Index`, `a += b` is `AssignOp`. typeck
        // records the resolved TRAIT method (`core::ops::Add::add`, `Index::index`, `Deref::deref`, …)
        // on the operator expr's own hir_id — but ONLY when it's overloaded (a user/std impl); a builtin
        // op on primitives (`1 + 2`, slice `arr[i]`) has no `type_dependent_def_id`, so `None` = no call.
        // The trait method is non-local (`core`), so we must devirtualize it to the CONCRETE impl via the
        // operand types (operators dispatch statically, never `dyn`): a LOCAL impl is the real target to
        // edge into; a non-local/std impl (`String + &str`, `Vec` indexing, `Arc` deref) resolves to std
        // and is treated as pure, matching the std-trait calibration; an unresolvable generic stays pure.
        // Without this, an effectful `impl Add`/`Index`/`Deref` reached through operator sugar was
        // invisible to the call graph — the caller looked pure though the impl performs I/O. A silent
        // under-report; teeth: soundness/gen.py `op_add`/`index`/`deref` forms.
        ExprKind::Unary(..)
        | ExprKind::Index(..)
        | ExprKind::AssignOp(..) => {
            let method_did = typeck.type_dependent_def_id(expr.hir_id)?;
            match devirtualize(cx, expr, method_did) {
                Some(Devirt::Static(did)) => Some(Callee::Def { did, dynamic: false }),
                // An operator that resolves still-virtual (vanishingly rare — operators dispatch
                // statically) is honestly dynamic: keep the trait method + flag it so check_expr CHAs
                // or marks it `Unknown` rather than treating it as a pinned target.
                Some(Devirt::StillVirtual) => Some(Callee::Def { did: method_did, dynamic: true }),
                None => None,
            }
        }
        ExprKind::Binary(op, lhs, _) => {
            use rustc_hir::BinOpKind::*;
            // COMPARISON operators need a dedicated path — the normal type_dependent_def_id route misses
            // the operand's LOCAL impl two different ways: `==`/`!=` record NO type_dependent_def_id at
            // all, and `<`/`<=`/`>`/`>=` record one pointing at the non-local DEFAULT PartialOrd method
            // (lt/le/gt/ge) which forwards to — but HIDES — the local `partial_cmp`. Either way an
            // effectful eq/partial_cmp reached via comparison sugar was silent-pure in the SOUND gate (its
            // worst hole). Resolve the operand's eq/partial_cmp directly. Arithmetic/bitwise ops (and Index/
            // Unary/AssignOp above) keep the type_dependent_def_id path.
            if matches!(op.node, Eq | Ne | Lt | Le | Gt | Ge) {
                resolve_cmp_op(cx, typeck, op.node, lhs)
            } else {
                typeck.type_dependent_def_id(expr.hir_id).and_then(|method_did| {
                    match devirtualize(cx, expr, method_did) {
                        Some(Devirt::Static(did)) => Some(Callee::Def { did, dynamic: false }),
                        Some(Devirt::StillVirtual) => Some(Callee::Def { did: method_did, dynamic: true }),
                        None => None,
                    }
                })
            }
        }
        _ => None,
    }
}

/// Is `did` a runtime/external entry point — invoked from outside project Rust code, so a reachability
/// ROOT (candor-spec §2 `entryPoint`)? The program `main`, or an exported symbol (`#[no_mangle]` /
/// `#[export_name]`) the linker/C/FFI calls. Far narrower than the JVM port's reflective surface — Rust
/// has no framework-reflection entry points — which the spec explicitly allows (population is
/// runtime-specific).
fn rust_is_entry_point(cx: &LateContext<'_>, did: DefId, entry_fn: Option<DefId>) -> bool {
    if Some(did) == entry_fn {
        return true;
    }
    if !matches!(cx.tcx.def_kind(did), DefKind::Fn | DefKind::AssocFn) {
        return false;
    }
    use rustc_middle::middle::codegen_fn_attrs::CodegenFnAttrFlags as F;
    // `#[no_mangle]` exports the symbol under its own name for the linker / C / FFI to call — an
    // external entry point. (`#[export_name]` is rarer and not separately flagged here.)
    cx.tcx.codegen_fn_attrs(did).flags.contains(F::NO_MANGLE)
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

/// The outcome of trying to devirtualize a trait-method dispatch.
#[derive(Clone, Copy)]
enum Devirt {
    /// Statically resolved to exactly this impl method — a real devirtualization.
    Static(DefId),
    /// Resolution says the call is STILL virtual (vtable dispatch): a `dyn` receiver the structural
    /// `is_dyn_receiver` check didn't recognise (e.g. behind a custom smart pointer / arbitrary self
    /// type). The caller must treat it as dynamic — CHA the local impls, or honest `Unknown` for a
    /// non-local trait — and must NOT edge to the (bodyless) trait method `instance.def_id()` returns.
    StillVirtual,
}

/// Resolve a trait-method dispatch to the single concrete impl it lands on, when the receiver/operand
/// type is known — so candor can use the ONE real target instead of CHA-expanding to every impl (the
/// over-approximation that yields confident false positives, CRITIQUE §9). Returns None for
/// generic receivers that can't be pinned down here, `Devirt::StillVirtual` for a dispatch that's
/// actually dynamic, and `Devirt::Static` for a real resolution — so the caller falls back to CHA in
/// the first two cases. Handles method calls, overloaded operators (`Binary`/`Unary`/`Index`/
/// `AssignOp`), AND fully-qualified trait Calls — including the ones the compiler GENERATES, like
/// `Future::poll(..)` from a `.await` desugar or `Trait::method(x)` UFCS. Method/operator nodes carry
/// their substs as `node_args` on the expr; a `Call` carries them on the callee path's `FnDef` type.
fn devirtualize<'tcx>(cx: &LateContext<'tcx>, expr: &Expr<'tcx>, method_did: DefId) -> Option<Devirt> {
    // `Instance::try_resolve` asserts the def is a Fn/AssocFn/Const; method calls are always
    // AssocFn today, but guard explicitly so an unexpected DefKind can never ICE the build (an
    // effect checker must degrade to Unknown, never abort compilation).
    if !matches!(cx.tcx.def_kind(method_did), DefKind::Fn | DefKind::AssocFn) {
        return None;
    }
    let typeck = cx.maybe_typeck_results()?;
    // The generic args that pin the impl. A `Call`'s substs live on the callee path's `FnDef`
    // (`node_args(call_expr)` is empty there); method/operator nodes carry them on the expr itself.
    let args = match expr.kind {
        ExprKind::Call(callee, _) => match typeck.expr_ty(callee).kind() {
            rustc_middle::ty::TyKind::FnDef(_, substs) => *substs,
            _ => return None,
        },
        ExprKind::MethodCall(..)
        | ExprKind::Binary(..)
        | ExprKind::Unary(..)
        | ExprKind::Index(..)
        | ExprKind::AssignOp(..) => typeck.node_args(expr.hir_id),
        _ => return None,
    };
    // First under the ordinary analysis typing env; on failure, RETRY with opaques revealed
    // (post-analysis — what codegen resolves under). The analysis env returns `Ok(None)` when the
    // receiver's type is an OPAQUE alias (`impl Trait` in return position), so a method call on a
    // returned `impl Iterator` resolved nowhere and fell into the unresolvable-generic-stays-pure
    // calibration: no edge, no `Unknown` — a silent under-report through a completely ordinary API
    // shape (`which`'s `which_all(..).and_then(|mut i| i.next())`, where the hidden type is a local
    // effectful iterator; found by dogfooding the umbrella AGENTS.md route). Revealing pins the call
    // to the one concrete impl; a still-generic or still-`dyn` reveal falls through to CHA/`Unknown`
    // exactly as before, so the retry only ever ADDS resolution, never invents one.
    let resolve = |env: rustc_middle::ty::TypingEnv<'tcx>,
                   args: rustc_middle::ty::GenericArgsRef<'tcx>| {
        rustc_middle::ty::Instance::try_resolve(cx.tcx, env, method_did, args).ok().flatten()
    };
    let instance = resolve(cx.typing_env(), args).or_else(|| {
        let env = cx.typing_env().with_post_analysis_normalized(cx.tcx);
        let args = cx.tcx.try_normalize_erasing_regions(env, rustc_middle::ty::Unnormalized::new(args)).unwrap_or(args);
        resolve(env, args)
    })?;
    // A `Virtual` instance means resolution did NOT devirtualize — the call is still vtable dispatch
    // (a `dyn` the structural `is_dyn_receiver` check didn't recognise, e.g. behind a custom smart
    // pointer with an arbitrary self type). `instance.def_id()` is then the BODYLESS trait method, so
    // edging to it would falsely mark the dispatch resolved and hide every real impl. Report it as
    // still-virtual instead, so the caller CHAs the local impls (or keeps an honest `Unknown`).
    match instance.def {
        rustc_middle::ty::InstanceKind::Virtual(..) => Some(Devirt::StillVirtual),
        _ => Some(Devirt::Static(instance.def_id())),
    }
}

/// Where an overloaded-deref adjustment resolved, and whether to trust it as a pinned target.
enum DerefStep {
    /// Resolved statically to this concrete impl method (`<T as Deref>::deref` / `DerefMut::deref_mut`).
    /// Local → real edge; non-local (std `Box`/`Rc`/`Arc`/`Pin`) → caller drops it (pure calibration).
    Static(DefId),
    /// The deref could not be pinned to a concrete impl (a generic/`dyn` smart pointer): honest
    /// `Unknown`, never silent-pure.
    Unresolved,
}

/// Recover the IMPLICIT `Deref::deref` / `DerefMut::deref_mut` calls the compiler inserts as expression
/// ADJUSTMENTS — invisible to the HIR `ExprKind` walk. Auto-deref during method resolution (`w.ping()`
/// where `W: Deref<Target=Inner>`), field access through a smart pointer (`s.field`), and deref-coercion
/// at a call/arg/return/assignment site (`takes(&s)` coercing `&S → &Inner`) all desugar to overloaded
/// `deref(_mut)` calls recorded NOT as `Call`/`MethodCall`/`Unary(Deref)` nodes but as
/// `Adjust::Deref(DerefAdjustKind::Overloaded(OverloadedDeref))` in `typeck.expr_adjustments(expr)`. A
/// LOCAL effectful `Deref` impl reached only this way was reported with NEITHER its effect NOR `Unknown`
/// — silently pure (the explicit `*w` `Unary(Deref)` arm handled the visible case; this was the hole).
///
/// For each overloaded-deref step we resolve `<FromTy as Deref/DerefMut>::deref(_mut)` to its concrete
/// impl via `Instance::try_resolve`, threading the from-type through the chain (a single expr can carry
/// MULTIPLE deref steps — chained coercion `&A → &B → &C`). A LOCAL resolution is a real call edge; a
/// non-local one (std `Box`/`Rc`/`Arc` deref) contributes nothing, matching the std-trait calibration —
/// no fabrication; an unresolvable/generic deref is `Unresolved` (→ `Unknown`), never silent-pure.
/// Teeth: soundness/gen.py `autoderef` form + /tmp/candor_probe p5/p5b/p5c/p8 repros.
fn overloaded_deref_steps<'tcx>(cx: &LateContext<'tcx>, expr: &Expr<'tcx>) -> Vec<DerefStep> {
    use rustc_middle::ty::adjustment::{Adjust, DerefAdjustKind};
    let Some(typeck) = cx.maybe_typeck_results() else { return Vec::new() };
    let adjustments = typeck.expr_adjustments(expr);
    if adjustments.is_empty() {
        return Vec::new();
    }
    let mut steps = Vec::new();
    // The type FED INTO the next adjustment. Adjustments apply left-to-right starting from the expr's
    // own (unadjusted) type; each adjustment's `.target` is the type AFTER it. An overloaded
    // `deref(_mut)` has signature `&T -> &U`, so its `Self` (the impl's receiver type) is the pointee
    // `T` — recovered by peeling references off the current input type.
    let mut cur = typeck.expr_ty(expr);
    for adj in adjustments {
        if let Adjust::Deref(DerefAdjustKind::Overloaded(od)) = adj.kind {
            // `<T as Deref>::deref` / `<T as DerefMut>::deref_mut` — the bodyless trait method.
            let method_did = od.method_call(cx.tcx);
            // Self = the pointee type T (peel the `&`/`&mut` the deref operates through).
            let self_ty = cur.peel_refs();
            // The Deref/DerefMut traits have a single generic param (Self); resolve to the concrete impl.
            let gargs = cx.tcx.mk_args(&[self_ty.into()]);
            let resolve = |env: rustc_middle::ty::TypingEnv<'tcx>| {
                rustc_middle::ty::Instance::try_resolve(cx.tcx, env, method_did, gargs)
                    .ok()
                    .flatten()
            };
            let instance = resolve(cx.typing_env())
                .or_else(|| resolve(cx.typing_env().with_post_analysis_normalized(cx.tcx)));
            match instance.map(|i| (i.def, i.def_id())) {
                // A real, static resolution: trust it (local → edge, non-local → dropped by caller).
                Some((rustc_middle::ty::InstanceKind::Virtual(..), _)) | None => {
                    steps.push(DerefStep::Unresolved)
                }
                Some((_, did)) => steps.push(DerefStep::Static(did)),
            }
        }
        cur = adj.target;
    }
    steps
}




/// Recover the `?` error-conversion edge. `x?` on a `Result<_, E1>` inside a fn returning
/// `Result<_, E2>` desugars to a call to the std `FromResidual::from_residual`, whose body invokes
/// `<E2 as From<E1>>::from` to convert the error. That `From::from` is a LOCAL impl candor can't see
/// THROUGH the non-local std `from_residual` — so an effectful error conversion reached ONLY via `?`
/// is a silent under-report. From the call's Self type (the from_residual return = the fn's return
/// `Result<_, E2>`) and the residual arg (`Result<Infallible, E1>`), resolve `<E2 as From<E1>>::from`
/// and return it when LOCAL. Precise: a std/blanket `From` (e.g. the identity `From<T> for T`)
/// resolves non-local → no edge → correctly pure; no flooding `Unknown`. Result-only (the Residual
/// shape it reads); other `Try` types are nightly-only and rare, left as the residual gap. Every
/// guard fails to `None` (no edge) — adding an edge only ever ADDS soundness, never removes it.
fn from_residual_local_edge<'tcx>(
    cx: &LateContext<'tcx>,
    expr: &Expr<'tcx>,
    callee_did: DefId,
) -> Option<DefId> {
    // Gate to the std `FromResidual::from_residual` call of the `?` desugar (not a same-named user fn).
    let trait_did = cx.tcx.trait_of_assoc(callee_did)?;
    if cx.tcx.item_name(trait_did).as_str() != "FromResidual"
        || !matches!(cx.tcx.crate_name(trait_did.krate).as_str(), "core" | "std" | "alloc")
    {
        return None;
    }
    let ExprKind::Call(_, args) = expr.kind else { return None };
    let typeck = cx.maybe_typeck_results()?;
    // The error type of a `Result<_, E>` (diagnostic-item gated so a user `Result` alias can't spoof it).
    let err_of_result = |ty: rustc_middle::ty::Ty<'tcx>| -> Option<rustc_middle::ty::Ty<'tcx>> {
        if let rustc_middle::ty::TyKind::Adt(def, substs) = ty.kind() {
            if cx.tcx.is_diagnostic_item(rustc_span::sym::Result, def.did()) {
                return Some(substs.type_at(1));
            }
        }
        None
    };
    let e2 = err_of_result(typeck.expr_ty(expr))?; // Self = fn return Result<_, E2>
    let e1 = err_of_result(typeck.expr_ty(args.first()?))?; // residual = Result<Infallible, E1>
    // Resolve `<E2 as From<E1>>::from` to its concrete impl method.
    let from_trait = cx.tcx.get_diagnostic_item(rustc_span::sym::From)?;
    let from_fn = cx
        .tcx
        .associated_item_def_ids(from_trait)
        .iter()
        .copied()
        .find(|d| matches!(cx.tcx.def_kind(*d), DefKind::AssocFn))?;
    let gargs = cx.tcx.mk_args(&[e2.into(), e1.into()]); // From's args are [Self=E2, T=E1]
    let inst = rustc_middle::ty::Instance::try_resolve(cx.tcx, cx.typing_env(), from_fn, gargs)
        .ok()
        .flatten()?;
    let did = inst.def_id();
    did.is_local().then_some(did)
}

/// Instance-resolve a (generic) callee to its concrete impl method and keep it only if LOCAL.
fn resolve_local_method<'tcx>(
    cx: &LateContext<'tcx>,
    fn_did: DefId,
    gargs: rustc_middle::ty::GenericArgsRef<'tcx>,
) -> Option<DefId> {
    let inst = rustc_middle::ty::Instance::try_resolve(cx.tcx, cx.typing_env(), fn_did, gargs)
        .ok()
        .flatten()?;
    let did = inst.def_id();
    did.is_local().then_some(did)
}

/// RETURN-TYPE-directed std driver edge (HOLE: `collect`/`into`/`parse`). A std method selects a LOCAL
/// trait impl by the call's RESULT type, then runs it inside its non-local body — invisible through the
/// std fn, so an effectful `FromIterator`/`From`/`FromStr` impl reached this way was silently pure (the
/// receiver-directed iter-combinator bridge only peels the RECEIVER, never the return type). Recover the
/// one edge to the local impl method (precise; a non-local/std target resolves to std and is dropped).
fn return_type_driver_local_edge<'tcx>(
    cx: &LateContext<'tcx>,
    expr: &Expr<'tcx>,
    callee_did: DefId,
) -> Option<DefId> {
    let typeck = cx.maybe_typeck_results()?;
    let ExprKind::MethodCall(_, receiver, _, _) = expr.kind else { return None };
    let method = cx.tcx.item_name(callee_did);
    // Unwrap `Result<T, _>` to its Ok type (diagnostic-item gated so a user `Result` alias can't spoof).
    let ok_of_result = |ty: rustc_middle::ty::Ty<'tcx>| -> Option<rustc_middle::ty::Ty<'tcx>> {
        if let rustc_middle::ty::TyKind::Adt(def, substs) = ty.kind() {
            if cx.tcx.is_diagnostic_item(rustc_span::sym::Result, def.did()) {
                return Some(substs.type_at(0));
            }
        }
        None
    };
    let assoc_fn = |trait_did: DefId| -> Option<DefId> {
        cx.tcx
            .associated_item_def_ids(trait_did)
            .iter()
            .copied()
            .find(|d| matches!(cx.tcx.def_kind(*d), DefKind::AssocFn))
    };

    // `x.into()` → `<ResultTy as From<SrcTy>>::from` (the `T: Into<U>` blanket is `U: From<T>`).
    if method.as_str() == "into" {
        let trait_did = cx.tcx.trait_of_assoc(callee_did)?;
        if cx.tcx.item_name(trait_did).as_str() != "Into" {
            return None;
        }
        let result_ty = typeck.expr_ty(expr);
        let src_ty = typeck.expr_ty_adjusted(receiver).peel_refs();
        let from_trait = cx.tcx.get_diagnostic_item(rustc_span::sym::From)?;
        let gargs = cx.tcx.mk_args(&[result_ty.into(), src_ty.into()]); // From's args [Self=Result, T=Src]
        return resolve_local_method(cx, assoc_fn(from_trait)?, gargs);
    }
    // `s.parse::<T>()` → `<T as FromStr>::from_str` (T is the Ok type of the `Result<T, T::Err>` return).
    // FromStr is not a diagnostic item; recover it from `parse`'s own `F: FromStr` bound.
    if method.as_str() == "parse" {
        let target = ok_of_result(typeck.expr_ty(expr))?;
        let fromstr_trait = cx.tcx.predicates_of(callee_did).predicates.iter().find_map(|(p, _)| {
            let tp = p.as_trait_clause()?;
            let did = tp.def_id();
            (cx.tcx.item_name(did).as_str() == "FromStr").then_some(did)
        })?;
        let gargs = cx.tcx.mk_args(&[target.into()]); // FromStr's only param is Self
        return resolve_local_method(cx, assoc_fn(fromstr_trait)?, gargs);
    }
    // `it.collect::<T>()` → `<T as FromIterator<Item>>::from_iter`. from_iter needs [Self=T, A=Item,
    // I=iterator]: pull the iterator type (the receiver) + its `Iterator::Item`, and the result type T.
    if method.as_str() == "collect" {
        let trait_did = cx.tcx.trait_of_assoc(callee_did)?;
        if cx.tcx.item_name(trait_did).as_str() != "Iterator" {
            return None;
        }
        let result_ty = typeck.expr_ty(expr);
        let iter_ty = typeck.expr_ty_adjusted(receiver);
        let iter_trait = cx.tcx.get_diagnostic_item(rustc_span::sym::Iterator)?;
        let item_assoc = cx
            .tcx
            .associated_items(iter_trait)
            .in_definition_order()
            .find(|a| matches!(a.kind, rustc_middle::ty::AssocKind::Type { .. }))?
            .def_id;
        let item_ty = cx
            .tcx
            .try_normalize_erasing_regions(
                cx.typing_env(),
                rustc_middle::ty::Unnormalized::new(rustc_middle::ty::Ty::new_projection(
                    cx.tcx, item_assoc, [iter_ty],
                )),
            )
            .ok()?;
        let fromiter_trait = cx.tcx.get_diagnostic_item(rustc_span::sym::FromIterator)?;
        let gargs = cx.tcx.mk_args(&[result_ty.into(), item_ty.into(), iter_ty.into()]);
        return resolve_local_method(cx, assoc_fn(fromiter_trait)?, gargs);
    }
    None
}

/// `core::mem::drop(x)` runs `x`'s destructor INSIDE mem::drop's non-local body — so an effectful local
/// `Drop` impl reached via an explicit `drop(guard)` (early lock/file/connection release) was silent-pure
/// (scope-end drop-glue IS modeled, but moving the value into `mem::drop` relocates the destructor to a
/// std fn the engine doesn't walk). Resolve `<T as Drop>::drop` for the argument type and edge it if local.
fn mem_drop_local_edge<'tcx>(
    cx: &LateContext<'tcx>,
    expr: &Expr<'tcx>,
    callee_did: DefId,
) -> Option<DefId> {
    if !cx.tcx.is_diagnostic_item(rustc_span::sym::mem_drop, callee_did) {
        return None;
    }
    let ExprKind::Call(_, args) = expr.kind else { return None };
    let typeck = cx.maybe_typeck_results()?;
    let arg_ty = typeck.expr_ty_adjusted(args.first()?);
    let drop_trait = cx.tcx.lang_items().drop_trait()?;
    let drop_fn = cx
        .tcx
        .associated_item_def_ids(drop_trait)
        .iter()
        .copied()
        .find(|d| matches!(cx.tcx.def_kind(*d), DefKind::AssocFn))?;
    let gargs = cx.tcx.mk_args(&[arg_ty.into()]); // Drop's only param is Self
    resolve_local_method(cx, drop_fn, gargs)
}

/// Resolve a COMPARISON-operator call (`==`/`!=` -> PartialEq::eq, `<`/`<=`/`>`/`>=` -> PartialOrd::
/// partial_cmp) to its concrete LOCAL impl method, pinned by the operand type. Comparison operators don't
/// record a `type_dependent_def_id` even when overloaded, so resolve_callee can't see them via the normal
/// operator path — without this an effectful PartialEq/PartialOrd impl reached through comparison sugar is
/// invisible (a silent under-report in the sound gate). Mirrors the `From` resolver above (two type
/// params). A non-local/std impl resolves to std (pure calibration); an unresolvable generic stays dynamic.
fn resolve_cmp_op<'tcx>(
    cx: &LateContext<'tcx>,
    typeck: &rustc_middle::ty::TypeckResults<'tcx>,
    op: rustc_hir::BinOpKind,
    lhs: &Expr<'tcx>,
) -> Option<Callee> {
    use rustc_hir::BinOpKind::*;
    // Lang items (guaranteed present) rather than diagnostic items — PartialOrd's diagnostic item isn't
    // reliably set, which left `<`/`<=`/`>`/`>=` unresolved.
    let (trait_did, method) = match op {
        Eq | Ne => (cx.tcx.lang_items().eq_trait()?, "eq"),
        Lt | Le | Gt | Ge => (cx.tcx.lang_items().partial_ord_trait()?, "partial_cmp"),
        _ => return None,
    };
    let trait_fn = cx
        .tcx
        .associated_items(trait_did)
        .in_definition_order()
        .find(|a| a.is_fn() && a.name().as_str() == method)?
        .def_id;
    // PartialEq<Rhs = Self> / PartialOrd<Rhs = Self>: the impl is pinned by [Self = T, Rhs = T] where T is
    // the operand type (peel the auto-ref the `&self`/`&Rhs` receiver adds).
    let t = typeck.expr_ty_adjusted(lhs).peel_refs();
    let gargs = cx.tcx.mk_args(&[t.into(), t.into()]);
    let resolve = |env: rustc_middle::ty::TypingEnv<'tcx>| {
        rustc_middle::ty::Instance::try_resolve(cx.tcx, env, trait_fn, gargs).ok().flatten()
    };
    let inst = resolve(cx.typing_env())
        .or_else(|| resolve(cx.typing_env().with_post_analysis_normalized(cx.tcx)))?;
    match inst.def {
        // still vtable dispatch (vanishingly rare for operators) — honest dynamic, CHA'd or Unknown.
        rustc_middle::ty::InstanceKind::Virtual(..) => Some(Callee::Def { did: trait_fn, dynamic: true }),
        _ => Some(Callee::Def { did: inst.def_id(), dynamic: false }),
    }
}

/// The outcome of resolving a std iterator-combinator (or `core::fmt`) call to the LOCAL trait impl(s)
/// its hidden std-internal callback reaches — the soundness recovery for those two silent-pure holes.
enum CallbackEdges {
    /// One or more LOCAL impl methods to edge to (e.g. the receiver's `Iterator::next`, or a value's
    /// `Display::fmt`). Their body effects then propagate to the caller.
    Local(Vec<DefId>),
    /// A combinator/consumer WAS hit (so the caller really does drive the hidden callback) but the
    /// concrete local impl couldn't be recovered — honest `Unknown`, never silent-pure. Carries the
    /// `unknownWhy` reason tag.
    Unknown(String),
}

/// The std `Iterator` adapter/wrapper ADTs that carry an inner iterator in a type argument (`Map<I, F>`,
/// `Filter<I, P>`, `Take<I>`, `Enumerate<I>`, …). We peel through these to reach the user iterator that
/// actually performs the I/O in its `next()`. Matched by `core`/`alloc` item name — std adapters all
/// live in `core::iter::adapters` / `core::iter::sources`; a user type sharing one of these names lives
/// in a non-std crate, so the crate check keeps them apart.
fn is_std_iter_adapter(name: &str) -> bool {
    matches!(
        name,
        "Map" | "Filter" | "FilterMap" | "Enumerate" | "Zip" | "Take" | "TakeWhile" | "Skip"
            | "SkipWhile" | "StepBy" | "Peekable" | "Rev" | "Cloned" | "Copied" | "Cycle"
            | "Chain" | "FlatMap" | "Flatten" | "Fuse" | "Inspect" | "Scan" | "MapWhile"
            | "ByRefSized" | "Intersperse" | "IntersperseWith"
    )
}

/// std iterator-family traits whose combinator/consumer methods drive the receiver's `Iterator::next`
/// (or `IntoIterator::into_iter`) INTERNALLY — i.e. through a non-local std body candor can't follow.
/// A call resolving to one of these (when its concrete impl is std, not a LOCAL override) is exactly the
/// silent-pure hole: the outer method is pure for ITSELF, but the hidden `next()` it calls may be a
/// LOCAL effectful impl. `Iterator` (for_each/map/collect/sum/count/fold/last/nth/…), the consumer
/// traits `Sum`/`Product`/`FromIterator` (`.collect()`), and `IntoIterator` (whose `into_iter` may be a
/// local effectful impl). `next`/`into_iter` themselves dispatch to the LOCAL impl directly and are
/// resolved by the ordinary devirtualize path, so re-edging them here is a harmless no-op.
fn is_iter_driver_trait(crate_name: &str, trait_name: &str) -> bool {
    matches!(crate_name, "core" | "std" | "alloc")
        && matches!(trait_name, "Iterator" | "IntoIterator" | "FromIterator" | "Sum" | "Product")
}

/// True when an iter-driver call (`it.for_each(..)`, `it.sum()`) is dispatched on a receiver whose type
/// is (or wraps, e.g. `Map<I, _>`) a GENERIC PARAM (`I` in `fn run<I: Iterator>(it: I)`) — the silent-pure
/// generic-receiver shape. Standalone the impl can't be pinned (the concrete `I` could be a LOCAL
/// effectful iterator), so the enclosing fn carries a report-only honest `Unknown`. A CONCRETE receiver
/// (`Rows`, `vec.iter()`) is `false` here — HOLE 1 / the call-site monomorphization handle those — so this
/// never floods ordinary iteration; and it's gated to iter-driver traits, so `Clone`/`Display` on a
/// generic param (pure-std) is untouched.
fn iter_receiver_is_generic_param<'tcx>(recv_ty: rustc_middle::ty::Ty<'tcx>) -> bool {
    use rustc_middle::ty::TyKind;
    // A generic-param (`I`) or associated-projection (`I::IntoIter`) anywhere in the receiver type — bare
    // (`it: I`) or wrapped by a std adapter (`Map<I, F>`, `Chain<I, J>`) — means the concrete iterator
    // can't be pinned at this site. (`Ty::walk` yields every nested generic arg.)
    recv_ty.peel_refs().walk().any(|g| {
        matches!(
            g.as_type().map(|t| t.kind()),
            Some(TyKind::Param(..)) | Some(TyKind::Alias(..))
        )
    })
}

/// Find the LOCAL effectful trait impls a std iterator-combinator call hides (HOLE 1). The call
/// resolved to a std `Iterator`/`IntoIterator`/`Sum`/`Product`/`FromIterator` method whose body pulls
/// the receiver's `Iterator::next` / `IntoIterator::into_iter` — a LOCAL impl candor can't see THROUGH
/// the std method, so an effect in that `next()` is silently lost (`It.for_each(..)`, `It.map(..).
/// collect()`, `It.sum()`, `for x in it.map(..) {}`). From the receiver type we peel std adapters
/// (`Map`/`Filter`/…) to the underlying user iterator ADT(s), then resolve their LOCAL `Iterator::next`
/// (and `IntoIterator::into_iter`) impl methods. Returns `Local(edges)` when ≥1 local impl is recovered;
/// `Unknown(reason)` when a driver method WAS hit but no local impl could be pinned (honest, never
/// silent-pure); `None` when the receiver is wholly std (`vec.iter()` — pure, contributes nothing, NO
/// Unknown flood). Teeth: soundness/gen.py `iter_combinator` form.
fn iter_combinator_local_edges<'tcx>(
    cx: &LateContext<'tcx>,
    expr: &Expr<'tcx>,
    trait_did: DefId,
    callee_did: DefId,
) -> Option<CallbackEdges> {
    let tk = cx.tcx.crate_name(trait_did.krate);
    let ti = cx.tcx.item_name(trait_did);
    if !is_iter_driver_trait(tk.as_str(), ti.as_str()) {
        return None;
    }
    // Only when the OUTER method resolves to a NON-local (std) impl: a LOCAL combinator/consumer impl is
    // an ordinary local edge already (handled by devirtualize/CHA), and re-driving it here would be
    // redundant. A std combinator is precisely the body candor can't follow.
    let resolved_local = matches!(devirtualize(cx, expr, callee_did), Some(Devirt::Static(t)) if t.is_local());
    if resolved_local {
        return None;
    }
    let ExprKind::MethodCall(_, receiver, _, _) = expr.kind else { return None };
    let typeck = cx.maybe_typeck_results()?;
    let recv_ty = typeck.expr_ty_adjusted(receiver);
    let method = cx.tcx.item_name(callee_did);

    // Peel std iterator adapters (`Map<I, F>` → `I`) down to the underlying user iterator ADT(s), then
    // collect each one's LOCAL `Iterator::next` / `IntoIterator::into_iter` impl method (shared with the
    // generic-receiver recovery, which peels the same way on the monomorphized Self type).
    let (edges, saw_local_iter) = peel_iter_to_local_next_seen(cx, recv_ty.peel_refs());
    if !edges.is_empty() {
        return Some(CallbackEdges::Local(edges));
    }
    // A driver method WAS hit. If the receiver contained a LOCAL iterator type but we couldn't pin its
    // `next` impl, be honest (`Unknown`) rather than silently pure. A wholly-std receiver (`vec.iter()`,
    // `0..n`) contributed no local ADT — correctly pure, no Unknown.
    if saw_local_iter {
        return Some(CallbackEdges::Unknown(format!("iter-combinator:{method}")));
    }
    None
}

/// Peel std iterator adapters (`Map<I, F>` → `I`, `Chain<A, B>` → `A`,`B`) off `recv_ty` down to the
/// underlying user iterator ADT(s) and resolve each one's LOCAL `Iterator::next` / `IntoIterator::
/// into_iter` impl method — the iterator's effectful body a std combinator/consumer drives internally.
/// Bounded depth against pathological adapter nesting; a single receiver can carry several inner
/// iterators, so a small worklist. Returns the local edges plus whether ANY local iterator ADT was seen
/// (so the caller can choose honest `Unknown` over silent-pure when no impl could be pinned). A wholly-std
/// receiver (`vec.iter()`, `0..n`) yields `(empty, false)` — pure, contributes nothing, no Unknown flood.
fn peel_iter_to_local_next_seen<'tcx>(
    cx: &LateContext<'tcx>,
    recv_ty: rustc_middle::ty::Ty<'tcx>,
) -> (Vec<DefId>, bool) {
    use rustc_middle::ty::TyKind;
    let mut edges: Vec<DefId> = Vec::new();
    let mut saw_local_iter = false;
    let mut work = vec![(recv_ty.peel_refs(), 0u32)];
    while let Some((ty, depth)) = work.pop() {
        if depth > 8 {
            continue;
        }
        let TyKind::Adt(adt, args) = ty.kind() else { continue };
        let adt_did = adt.did();
        let adt_name = cx.tcx.item_name(adt_did);
        let adt_krate = cx.tcx.crate_name(adt_did.krate);
        if is_std_iter_adapter(adt_name.as_str())
            && matches!(adt_krate.as_str(), "core" | "std" | "alloc")
        {
            for arg in args.types() {
                work.push((arg.peel_refs(), depth + 1));
            }
            continue;
        }
        if adt_did.is_local() {
            saw_local_iter = true;
            for (sym, trait_lang) in
                [("next", rustc_span::sym::Iterator), ("into_iter", rustc_span::sym::IntoIterator)]
            {
                if let Some(m) = local_trait_method_for_self(cx, ty, sym, trait_lang) {
                    edges.push(m);
                }
            }
        }
    }
    (edges, saw_local_iter)
}

/// As `peel_iter_to_local_next_seen` but returns only the local `next`/`into_iter` edges — used by the
/// generic-receiver recovery, where the monomorphized Self type (`Rows`) replaces a `Param`.
fn peel_iter_to_local_next<'tcx>(
    cx: &LateContext<'tcx>,
    recv_ty: rustc_middle::ty::Ty<'tcx>,
) -> Vec<DefId> {
    peel_iter_to_local_next_seen(cx, recv_ty).0
}

/// Resolve the concrete LOCAL impl method named `method_sym` of the std iterator trait `trait_lang`
/// (`Iterator`/`IntoIterator`) for receiver self-type `self_ty` — the `next()` / `into_iter()` a std
/// combinator drives internally. Returns the impl method `DefId` only when it's LOCAL (the real edge);
/// a std/non-local impl (`vec.iter()`'s `next`) yields `None` so it contributes nothing.
fn local_trait_method_for_self<'tcx>(
    cx: &LateContext<'tcx>,
    self_ty: rustc_middle::ty::Ty<'tcx>,
    method_sym: &str,
    trait_lang: rustc_span::Symbol,
) -> Option<DefId> {
    // This only ever yields a LOCAL impl method; for a self type that mentions no local ADT (a tuple,
    // a std collection, a bare ref) resolution can only land on a non-local impl → None. Skipping those
    // is a precision no-op AND sidesteps a rustc ICE (nightly `Instance::try_resolve` delayed-bugs with
    // "missing value for assoc item in impl" via core's `impl_hash_tuple` when resolution transitively
    // touches a tuple's Hash impl — e.g. a HashMap's `(K, V)` Item self type). Gate on "mentions a local
    // ADT" so we never feed those types to try_resolve. (Remove when the upstream ICE is fixed.)
    if !self_ty
        .walk()
        .filter_map(|g| g.as_type())
        .any(|t| matches!(t.kind(), rustc_middle::ty::TyKind::Adt(a, _) if a.did().is_local()))
    {
        return None;
    }
    let trait_did = cx.tcx.get_diagnostic_item(trait_lang)?;
    local_trait_method_by_did(cx, self_ty, method_sym, trait_did)
}

/// As `local_trait_method_for_self`, but the trait is given by DefId — for traits WITHOUT a diagnostic
/// item (`fmt::Write`/`io::Write`, used by the `write!` writer-side edge). Resolves `<self_ty as
/// Trait>::method_sym` to its concrete LOCAL impl method, or None (non-local / virtual / unresolvable).
fn local_trait_method_by_did<'tcx>(
    cx: &LateContext<'tcx>,
    self_ty: rustc_middle::ty::Ty<'tcx>,
    method_sym: &str,
    trait_did: DefId,
) -> Option<DefId> {
    // Same local-ADT gate as the caller (a self type mentioning no local ADT resolves non-local → None,
    // and also sidesteps the tuple-Hash `try_resolve` ICE — see `local_trait_method_for_self`).
    if !self_ty
        .walk()
        .filter_map(|g| g.as_type())
        .any(|t| matches!(t.kind(), rustc_middle::ty::TyKind::Adt(a, _) if a.did().is_local()))
    {
        return None;
    }
    let trait_fn = cx
        .tcx
        .associated_items(trait_did)
        .in_definition_order()
        .find(|a| a.is_fn() && a.name().as_str() == method_sym)?
        .def_id;
    let gargs = cx.tcx.mk_args(&[self_ty.into()]);
    let inst = rustc_middle::ty::Instance::try_resolve(cx.tcx, cx.typing_env(), trait_fn, gargs)
        .ok()
        .flatten()
        .or_else(|| {
            let env = cx.typing_env().with_post_analysis_normalized(cx.tcx);
            rustc_middle::ty::Instance::try_resolve(cx.tcx, env, trait_fn, gargs).ok().flatten()
        })?;
    if matches!(inst.def, rustc_middle::ty::InstanceKind::Virtual(..)) {
        return None;
    }
    let did = inst.def_id();
    did.is_local().then_some(did)
}

/// WRITER-side fmt hole: `write!`/`writeln!` lower to `w.write_fmt(args)`, whose default `fmt::Write` /
/// `io::Write` impl drives the writer's REQUIRED method (`write_str` / `write`). candor sees only the
/// non-local default `write_fmt`, so a LOCAL effectful writer reached only via `write!` was silent-pure
/// (the writer side, distinct from HOLE 2's ARGUMENT-side `Display`). Recover the edge to the receiver's
/// local required method. A std writer (`String`/`Vec`/`Stdout`) resolves non-local → None (pure).
fn fmt_write_local_edge<'tcx>(
    cx: &LateContext<'tcx>,
    expr: &Expr<'tcx>,
    callee_did: DefId,
) -> Option<DefId> {
    if callee_did.is_local() || cx.tcx.item_name(callee_did).as_str() != "write_fmt" {
        return None;
    }
    let trait_did = cx.tcx.trait_of_assoc(callee_did)?;
    // The required method the default `write_fmt` drives, per which Write trait this is.
    let driven = match cx.tcx.def_path_str(trait_did).as_str() {
        "core::fmt::Write" | "std::fmt::Write" => "write_str",
        "std::io::Write" => "write",
        _ => return None,
    };
    let ExprKind::MethodCall(_, receiver, _, _) = expr.kind else {
        return None;
    };
    let typeck = cx.maybe_typeck_results()?;
    let recv_ty = typeck.expr_ty_adjusted(receiver).peel_refs();
    local_trait_method_by_did(cx, recv_ty, driven, trait_did)
}

/// HOLE — a NON-LOCAL std DRIVER method whose body invokes a trait method on its RECEIVER or ELEMENT
/// type that candor never sees: `x.to_string()` → `<X as Display>::fmt`; `v.contains(e)`/`v.clone()`/
/// `s.to_vec()`/`v.sort()`/`set.insert(e)` → `<E as PartialEq/Clone/Ord/Hash>::method`. candor sees only
/// the std method DefId, not the `<T as Trait>::m` dispatch over a LOCAL type — so a local EFFECTFUL impl
/// is reached silently (sweep [25]/[26]). Recover the edge to the LOCAL impl method; a std element
/// (`Vec<u32>`) or a pure derived impl resolves to a non-effectful target → no fabrication. Soundness only.
fn std_driver_local_edges<'tcx>(
    cx: &LateContext<'tcx>,
    expr: &Expr<'tcx>,
    callee_did: DefId,
) -> Vec<DefId> {
    use rustc_middle::ty::TyKind;
    use rustc_span::sym;
    if callee_did.is_local()
        || !matches!(cx.tcx.crate_name(callee_did.krate).as_str(), "core" | "std" | "alloc")
    {
        return vec![];
    }
    let ExprKind::MethodCall(_, receiver, _, _) = expr.kind else { return vec![] };
    let Some(typeck) = cx.maybe_typeck_results() else { return vec![] };
    let recv_ty = typeck.expr_ty_adjusted(receiver).peel_refs();
    let method = cx.tcx.item_name(callee_did);
    let method = method.as_str();
    let mut out = Vec::new();
    let mut push = |cx: &LateContext<'tcx>, ty: rustc_middle::ty::Ty<'tcx>, m: &str, tr: rustc_span::Symbol| {
        if let Some(did) = local_trait_method_for_self(cx, ty, m, tr) {
            out.push(did);
        }
    };
    // RECEIVER-typed driver: `x.to_string()` formats through `<X as Display>::fmt`.
    if method == "to_string" {
        push(cx, recv_ty, "fmt", sym::Display);
        return out;
    }
    // ELEMENT-typed drivers over a sequence/set container — the element type and which trait the verb
    // drives depend on the container KIND (a `Vec::insert` is positional and drives NOTHING — only a SET
    // `insert` drives Hash/Ord; conflating them would fabricate).
    let (elem, is_set, ordered) = match recv_ty.kind() {
        TyKind::Slice(e) | TyKind::Array(e, _) => (Some(*e), false, false),
        TyKind::Adt(adt, args) => match cx.tcx.item_name(adt.did()).as_str() {
            "Vec" | "VecDeque" | "LinkedList" => (args.types().next(), false, false),
            "HashSet" => (args.types().next(), true, false),
            "BTreeSet" | "BinaryHeap" => (args.types().next(), true, true),
            _ => (None, false, false),
        },
        _ => (None, false, false),
    };
    let Some(elem) = elem else { return out };
    match method {
        "clone" | "to_vec" => push(cx, elem, "clone", sym::Clone),
        "contains" => {
            if ordered {
                push(cx, elem, "cmp", sym::Ord);
            } else {
                push(cx, elem, "eq", sym::PartialEq);
            }
        }
        "sort" | "sort_unstable" | "sort_by" if !is_set => push(cx, elem, "cmp", sym::Ord),
        "insert" if ordered => push(cx, elem, "cmp", sym::Ord),
        "insert" if is_set => {
            push(cx, elem, "hash", sym::Hash);
            push(cx, elem, "eq", sym::PartialEq);
        }
        _ => {}
    }
    out
}

/// Find the LOCAL `Display`/`Debug`/… `fmt` impls a `core::fmt` formatting call hides (HOLE 2). The
/// `println!`/`format!`/`write!` macros lower each formatted value to a `core::fmt::rt::Argument::
/// new_<kind>(&value)` constructor; the actual `<kind>::fmt(&value, f)` call happens INSIDE the std
/// `core::fmt` machinery, invisible to candor — so a LOCAL `impl Display for T` whose `fmt()` does I/O
/// is reached silently. We detect the `Argument::new_<kind>` constructor, map `<kind>` to its fmt trait
/// (`new_display`→`Display`, `new_debug`→`Debug`, hex/oct/bin/exp/pointer→the matching trait), and
/// resolve the formatted value's LOCAL impl of that trait's `fmt`. A std `Display` (`i32`/`String`)
/// resolves non-local → no edge → correctly pure. Teeth: soundness/gen.py `display_fmt` form.
fn fmt_argument_local_edge<'tcx>(
    cx: &LateContext<'tcx>,
    expr: &Expr<'tcx>,
    callee_did: DefId,
) -> Option<CallbackEdges> {
    // Must be a `core::fmt::rt::Argument::new_<kind>` (or `new_<kind>`-shaped) constructor in core/std.
    let krate = cx.tcx.crate_name(callee_did.krate);
    if !matches!(krate.as_str(), "core" | "std" | "alloc") {
        return None;
    }
    let method = cx.tcx.item_name(callee_did);
    if !method.as_str().starts_with("new_") {
        return None;
    }
    // Confirm it's the fmt `Argument` inherent constructor, not an unrelated `new_*` fn elsewhere in
    // core/std. The method lives in an inherent `impl Argument { … }`, so check the impl's SELF type is
    // the `Argument` ADT. (Reading `item_name` on the parent `impl` DefId would ICE — an impl block has
    // no name — so resolve the self type instead.)
    let impl_did = cx.tcx.impl_of_assoc(callee_did)?;
    let self_ty = cx.tcx.type_of(impl_did).instantiate_identity().skip_normalization();
    let rustc_middle::ty::TyKind::Adt(self_adt, _) = self_ty.kind() else { return None };
    if cx.tcx.item_name(self_adt.did()).as_str() != "Argument" {
        return None;
    }
    // The fmt trait this constructor formats through is exactly the bound on its single type param
    // (`fn new_display<T: Display>(x: &T)`). Read it off the fn's predicates rather than mapping
    // `new_<kind>` → a `sym::` item (only Display/Debug/Pointer have diagnostic-item symbols; the
    // hex/oct/bin/exp traits don't), so EVERY fmt kind is covered uniformly.
    let trait_did = fmt_constructor_trait(cx, callee_did)?;
    let ExprKind::Call(_, args) = expr.kind else { return None };
    let typeck = cx.maybe_typeck_results()?;
    let arg = args.first()?; // `Argument::new_<kind>(&value)` — one arg, a reference to the value.
    let val_ty = typeck.expr_ty(arg).peel_refs();
    // Only a LOCAL ADT can carry a LOCAL fmt impl; a std type's fmt is non-local (pure, no edge).
    if !matches!(val_ty.kind(), rustc_middle::ty::TyKind::Adt(adt, _) if adt.did().is_local()) {
        return None;
    }
    match fmt_impl_for(cx, val_ty, trait_did) {
        Some(m) => Some(CallbackEdges::Local(vec![m])),
        // A local type whose fmt we couldn't pin (a blanket/derived impl resolved non-local) is pure —
        // a derived `Debug` is generated and effect-free, and a std blanket Display is non-local. No
        // Unknown here: that would flood every `format!` of a local type with a derived Debug.
        None => None,
    }
}

/// The fmt trait a `core::fmt::rt::Argument::new_<kind>` constructor formats through — the single trait
/// bound on its type parameter `T` (`new_display<T: Display>` → the `Display` trait `DefId`). Reading it
/// from the fn's predicates covers every fmt kind uniformly (Display/Debug/Octal/Hex/Binary/Exp/Pointer)
/// without needing a `sym::` diagnostic item per trait (most don't have one).
fn fmt_constructor_trait(cx: &LateContext<'_>, callee_did: DefId) -> Option<DefId> {
    for (clause, _) in cx.tcx.predicates_of(callee_did).predicates {
        if let Some(tp) = clause.as_trait_clause() {
            let trait_did = tp.def_id();
            // The fmt traits live in `core::fmt`; skip the implicit `Sized`/marker bounds.
            if matches!(cx.tcx.crate_name(trait_did.krate).as_str(), "core" | "std" | "alloc")
                && cx.tcx.def_path_str(trait_did).contains("fmt::")
            {
                return Some(trait_did);
            }
        }
    }
    None
}

/// Resolve `<self_ty as FmtTrait>::fmt` to its concrete LOCAL impl method, or `None` for a non-local
/// (std/blanket/derived-in-another-crate) impl. The fmt traits each have a single `fmt` assoc fn.
fn fmt_impl_for<'tcx>(
    cx: &LateContext<'tcx>,
    self_ty: rustc_middle::ty::Ty<'tcx>,
    trait_did: DefId,
) -> Option<DefId> {
    let fmt_fn = cx
        .tcx
        .associated_items(trait_did)
        .in_definition_order()
        .find(|a| a.is_fn() && a.name().as_str() == "fmt")?
        .def_id;
    let gargs = cx.tcx.mk_args(&[self_ty.into()]);
    let inst = rustc_middle::ty::Instance::try_resolve(cx.tcx, cx.typing_env(), fmt_fn, gargs)
        .ok()
        .flatten()
        .or_else(|| {
            let env = cx.typing_env().with_post_analysis_normalized(cx.tcx);
            rustc_middle::ty::Instance::try_resolve(cx.tcx, env, fmt_fn, gargs).ok().flatten()
        })?;
    if matches!(inst.def, rustc_middle::ty::InstanceKind::Virtual(..)) {
        return None;
    }
    let did = inst.def_id();
    did.is_local().then_some(did)
}

/// MONOMORPHIZATION-AWARE resolution of a generic callee's internal generic trait-method dispatches —
/// the soundness recovery for the silent-pure GENERIC-RECEIVER hole. When `caller` calls a LOCAL generic
/// fn/method `callee_did` with CONCRETE type args (`run_generic::<Rows>(Rows(3))`), the callee's body may
/// drive a trait method on one of its OWN generic params (`it.for_each(..)` → internally `<I as
/// Iterator>::next`). Inside the callee `I` is an unresolved `TyKind::Param`, so candor reports the callee
/// pure for ITSELF — and the effect in the CONCRETE impl (`<Rows as Iterator>::next`, local + effectful) is
/// silently lost at every monomorphizing call site too.
///
/// At THIS call site the concrete substs are known. We recover them (`run_generic`'s `[Rows]`), then walk
/// the callee's HIR body for trait-method calls (`MethodCall`/UFCS `Call`/overloaded operators), and for
/// each take the callee's OWN generic args for that inner call (expressed in terms of the callee's
/// params, e.g. `[I]` for `<I as Iterator>::next`) and INSTANTIATE them with the call-site substs —
/// `<I as Iterator>::next` under `I=Rows` becomes `<Rows as Iterator>::next`. Resolving that pinned
/// instance lands on the LOCAL `Rows::next` impl → a precise edge from `caller`, so its `Fs` propagates.
///
/// PRECISION, NO FABRICATION, NO FLOOD: we edge ONLY where the pinned instance resolves to a LOCAL impl.
/// A generic consumer used with a std/pure iterator (`run_generic(0..3)` / `run_generic(vec.iter())`)
/// resolves `<Range as Iterator>::next` / `<slice::Iter as Iterator>::next` NON-local → no edge → stays
/// pure (no fabricated effect it can't reach). A `g<T: Clone>(t){ t.clone() }` called with `T=u32`
/// resolves `<u32 as Clone>::clone` non-local → no edge → no Unknown flood. Only a CONCRETE local
/// effectful impl reached through the substitution gains an edge. We also RECURSE through an intermediate
/// generic free-fn the callee forwards to (`forward<J>(j){ run_generic(j) }`): the inner `run_generic(j)`
/// call's substs are monomorphized under THIS site's substs and re-walked, so a chain of generic
/// forwarders down to the iterator driver is still resolved (bounded depth + a visited set against
/// cycles). Returns the LOCAL impl methods to edge `caller` to. Teeth: soundness/gen.py `generic_iter` form.
fn generic_callee_local_edges<'tcx>(
    cx: &LateContext<'tcx>,
    expr: &Expr<'tcx>,
    callee_did: DefId,
) -> Vec<DefId> {
    let mut out: Vec<DefId> = Vec::new();
    let Some(callee_local) = callee_did.as_local() else { return out };
    if !matches!(cx.tcx.def_kind(callee_did), DefKind::Fn | DefKind::AssocFn) {
        return out;
    }
    if !cx.tcx.generics_of(callee_did).requires_monomorphization(cx.tcx) {
        return out;
    }
    // The CONCRETE call-site substs that pin the callee's generic params (`run_generic`'s `[Rows]`). A
    // `Call`'s substs ride the callee path's `FnDef`; a `MethodCall`'s ride the expr's `node_args`.
    let Some(caller_typeck) = cx.maybe_typeck_results() else { return out };
    let site_args = match expr.kind {
        ExprKind::Call(callee, _) => match caller_typeck.expr_ty(callee).kind() {
            rustc_middle::ty::TyKind::FnDef(_, substs) => *substs,
            _ => return out,
        },
        ExprKind::MethodCall(..) => caller_typeck.node_args(expr.hir_id),
        _ => return out,
    };
    let mut visited = std::collections::HashSet::new();
    collect_generic_callee_edges(cx, callee_local, site_args, 0, &mut visited, &mut out);
    out.dedup();
    out
}

/// True if any type in `args` is (or contains) a still-generic `Param` — the substitution didn't fully
/// pin the instance, so it can't resolve to a concrete impl (it'd land on the bodyless trait method).
fn args_still_generic<'tcx>(args: rustc_middle::ty::GenericArgsRef<'tcx>) -> bool {
    args.iter().any(|a| {
        a.as_type().is_some_and(|t| {
            t.walk()
                .any(|g| matches!(g.as_type().map(|x| x.kind()), Some(rustc_middle::ty::TyKind::Param(..))))
        })
    })
}

/// The recursive core of `generic_callee_local_edges`: walk LOCAL generic fn `callee`'s HIR body under the
/// CONCRETE `site_args`, resolving each internal generic trait-method dispatch (and peeling iter-driver
/// std defaults to the LOCAL `next`/`into_iter`), and recursing through an intermediate generic free-fn the
/// body forwards to. Bounded depth + a `visited` set of `(fn, args)` so a recursive/mutually-recursive
/// generic forwarder can't loop. Pushes LOCAL impl edges to `out`.
fn collect_generic_callee_edges<'tcx>(
    cx: &LateContext<'tcx>,
    callee: rustc_span::def_id::LocalDefId,
    site_args: rustc_middle::ty::GenericArgsRef<'tcx>,
    depth: u32,
    visited: &mut std::collections::HashSet<(rustc_span::def_id::LocalDefId, String)>,
    out: &mut Vec<DefId>,
) {
    use rustc_hir::intravisit::Visitor;
    if depth > 6 {
        return;
    }
    // Nothing concrete to pin (the caller is itself generic and merely forwards its own param) — bail.
    if args_still_generic(site_args) {
        return;
    }
    if !cx.tcx.has_typeck_results(callee) {
        return;
    }
    // Cycle/redundancy guard, keyed by (fn, monomorphized args).
    if !visited.insert((callee, format!("{site_args:?}"))) {
        return;
    }
    let callee_typeck = cx.tcx.typeck(callee);
    let env = cx.typing_env();

    // Monomorphize a callee-internal call's own generic args (`node_args`, bound by the callee's generics)
    // with the concrete `site_args` for THIS instantiation.
    let mono = |inner_args: rustc_middle::ty::GenericArgsRef<'tcx>| {
        cx.tcx
            .try_instantiate_and_normalize_erasing_regions(
                site_args,
                env,
                rustc_middle::ty::EarlyBinder::bind(inner_args),
            )
            .unwrap_or(inner_args)
    };
    // Resolve an instance under the analysis env, retrying with opaques revealed.
    let resolve = |did: DefId, args: rustc_middle::ty::GenericArgsRef<'tcx>| {
        rustc_middle::ty::Instance::try_resolve(cx.tcx, env, did, args)
            .ok()
            .flatten()
            .or_else(|| {
                let env2 = env.with_post_analysis_normalized(cx.tcx);
                rustc_middle::ty::Instance::try_resolve(cx.tcx, env2, did, args).ok().flatten()
            })
    };

    // A callee-internal trait-method dispatch (`<I as Iterator>::next`, an operator, a `<T as MyTrait>::m`):
    // monomorphize + resolve. LOCAL impl → edge; a non-local iter-driver default → peel the (now concrete)
    // Self type to the user iterator's LOCAL `next`/`into_iter` (the HOLE-1 recovery on a monomorphized Self).
    let mut on_trait_call = |method_did: DefId, inner_args: rustc_middle::ty::GenericArgsRef<'tcx>| {
        if !matches!(cx.tcx.def_kind(method_did), DefKind::Fn | DefKind::AssocFn) {
            return;
        }
        let Some(trait_did) = cx.tcx.trait_of_assoc(method_did) else { return };
        let mono_args = mono(inner_args);
        if args_still_generic(mono_args) {
            return;
        }
        if let Some(inst) = resolve(method_did, mono_args) {
            if !matches!(inst.def, rustc_middle::ty::InstanceKind::Virtual(..)) {
                let did = inst.def_id();
                if did.is_local() && did != method_did {
                    out.push(did);
                    return;
                }
            }
        }
        // Non-local resolution: only an iter-driver default hides a LOCAL `next` behind it. Peel the
        // concrete Self (`mono_args[0]`) to the user iterator's local `next`/`into_iter` (no fabrication
        // for a wholly-std receiver, no flood for `Clone`/`Display`).
        let tk = cx.tcx.crate_name(trait_did.krate);
        let ti = cx.tcx.item_name(trait_did);
        if is_iter_driver_trait(tk.as_str(), ti.as_str()) {
            if let Some(self_ty) = mono_args.types().next() {
                for m in peel_iter_to_local_next(cx, self_ty) {
                    out.push(m);
                }
            }
        }
    };

    // A callee-internal FREE-fn call. If it's a TRAIT method via UFCS, treat it as a trait dispatch. If
    // it's an ordinary generic LOCAL fn the body FORWARDS to (`forward<J>(j){ run_generic(j) }`), recurse
    // under its monomorphized substs so a chain of generic forwarders down to the driver is resolved.
    let mut forward_targets: Vec<(rustc_span::def_id::LocalDefId, rustc_middle::ty::GenericArgsRef<'tcx>)> =
        Vec::new();
    {
        struct V<'a, 'tcx, FT, FF> {
            tcx: TyCtxt<'tcx>,
            callee_typeck: &'a rustc_middle::ty::TypeckResults<'tcx>,
            on_trait: FT,
            on_free: FF,
        }
        impl<'a, 'tcx, FT, FF> Visitor<'tcx> for V<'a, 'tcx, FT, FF>
        where
            FT: FnMut(DefId, rustc_middle::ty::GenericArgsRef<'tcx>),
            FF: FnMut(DefId, rustc_middle::ty::GenericArgsRef<'tcx>),
        {
            fn visit_expr(&mut self, e: &'tcx Expr<'tcx>) {
                match e.kind {
                    ExprKind::MethodCall(..)
                    | ExprKind::Binary(..)
                    | ExprKind::Unary(..)
                    | ExprKind::Index(..)
                    | ExprKind::AssignOp(..) => {
                        if let Some(m) = self.callee_typeck.type_dependent_def_id(e.hir_id) {
                            (self.on_trait)(m, self.callee_typeck.node_args(e.hir_id));
                        }
                    }
                    // A `Call` to a `FnDef`: a UFCS trait method (→ trait dispatch), or an ordinary free
                    // fn the body forwards to (→ recurse if it's a LOCAL generic fn). The `FnDef`'s substs
                    // are the call's generic args.
                    ExprKind::Call(callee, _) => {
                        if let rustc_middle::ty::TyKind::FnDef(did, substs) =
                            self.callee_typeck.expr_ty(callee).kind()
                        {
                            if self.tcx.trait_of_assoc(*did).is_some() {
                                (self.on_trait)(*did, substs);
                            } else {
                                (self.on_free)(*did, substs);
                            }
                        }
                    }
                    _ => {}
                }
                rustc_hir::intravisit::walk_expr(self, e);
            }
        }
        let on_free = |did: DefId, substs: rustc_middle::ty::GenericArgsRef<'tcx>| {
            if let Some(local) = did.as_local() {
                if cx.tcx.generics_of(did).requires_monomorphization(cx.tcx) {
                    forward_targets.push((local, mono(substs)));
                }
            }
        };
        let body = cx.tcx.hir_body_owned_by(callee);
        let mut v = V {
            tcx: cx.tcx,
            callee_typeck,
            on_trait: &mut on_trait_call,
            on_free,
        };
        v.visit_expr(body.value);
    }
    // Recurse into the generic forwarders found, under their monomorphized substs.
    for (next_fn, next_args) in forward_targets {
        collect_generic_callee_edges(cx, next_fn, next_args, depth + 1, visited, out);
    }
}

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

/// The first string-literal argument of a call, if any. The program for `Command::new("git")` and the
/// path for `fs::read("/etc/x")` are both the first arg, so this serves the `Exec`/`Fs` allowlists the
/// way `net_hosts_in_call` serves `Net`. Called only for a call already classified with that effect.
fn first_str_lit_arg(expr: &Expr<'_>) -> Option<String> {
    use rustc_ast::LitKind;
    let args: &[Expr<'_>] = match expr.kind {
        ExprKind::Call(_, args) => args,
        ExprKind::MethodCall(_, _, args, _) => args,
        _ => return None,
    };
    for a in args {
        if let ExprKind::Lit(lit) = &a.kind {
            if let LitKind::Str(sym, _) = lit.node {
                let s = sym.as_str().trim();
                if !s.is_empty() {
                    return Some(s.to_string());
                }
            }
        }
    }
    None
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
/// `std::io::{Read,Write,BufRead,Seek}` — keyed on `(crate, item)` (cheap; no per-call path String).
/// The defining crate disambiguates `std::io::Write` (effectful) from `core::fmt::Write` (pure
/// formatting), which share the item name `Write` — so this never false-flags `fmt::Write`. Bounded to
/// the std I/O traits — where "assumed pure" is a real *under*-report — without the flood that marking
/// *all* generic dispatch Unknown would cause (that stays behind `CANDOR_PARANOID`).
fn is_effectful_std_trait(crate_name: &str, item_name: &str) -> bool {
    crate_name == "std" && matches!(item_name, "Read" | "Write" | "BufRead" | "Seek")
}

/// `core::fmt::Write` is pure (it formats into a buffer/`Formatter`, no I/O) but isn't in
/// `is_pure_std_trait`'s name list — and it can't be added there, since that list is crate-agnostic
/// across core/std/alloc and would then also mark the *effectful* `std::io::Write` pure (same item
/// name). Keyed on the defining crate so only `core::fmt::Write` matches.
fn is_pure_fmt_write(crate_name: &str, item_name: &str) -> bool {
    crate_name == "core" && item_name == "Write"
}

/// Collect the capability tokens a type carries, peeling common WRAPPERS so a cap behind
/// `Option<&Fs>` / `Vec<&Fs>` / `Box<&Fs>` / `Result<&Fs, _>` / a tuple is still recognised — not just
/// a bare `&Fs`. Without this, wrapping a capability produced a FALSE AS-EFF-001 ("performs Fs but
/// declares no capability") even though it was declared, just inside a container. Bounded depth.
fn caps_in_ty<'tcx>(tcx: TyCtxt<'tcx>, ty: rustc_middle::ty::Ty<'tcx>, depth: u32, out: &mut BTreeSet<&'static str>) {
    use rustc_middle::ty::TyKind;
    if depth > 4 {
        return;
    }
    let ty = ty.peel_refs();
    match ty.kind() {
        TyKind::Adt(adt, args) => {
            let name = tcx.item_name(adt.did());
            let krate = tcx.crate_name(adt.did().krate);
            if let Some(c) =
                cap_from_name(name.as_str()).or_else(|| capstd_cap(krate.as_str(), name.as_str()))
            {
                out.insert(c);
                return; // this node IS a capability — don't descend into its own generics
            }
            // a non-cap container (Option/Vec/Box/Result/…) — look inside its type arguments.
            for arg in args.types() {
                caps_in_ty(tcx, arg, depth + 1, out);
            }
        }
        TyKind::Tuple(elems) => {
            for e in elems.iter() {
                caps_in_ty(tcx, e, depth + 1, out);
            }
        }
        _ => {}
    }
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
        caps_in_ty(tcx, *input, 0, &mut out);
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

/// Collects every LOCAL fn referenced as a VALUE within a body (descending into nested bodies). Used to
/// recover a `thread_local!`'s deferred init fn (`__rust_std_internal_init_fn`), which the macro places
/// inside the `LocalKey::new(...)` construction in the static/const initializer — see `local_key_init_fns`.
struct FnRefCollector<'tcx> {
    tcx: TyCtxt<'tcx>,
    out: Vec<LocalDefId>,
}

impl<'tcx> rustc_hir::intravisit::Visitor<'tcx> for FnRefCollector<'tcx> {
    type NestedFilter = rustc_middle::hir::nested_filter::All;
    fn maybe_tcx(&mut self) -> TyCtxt<'tcx> {
        self.tcx
    }
    fn visit_expr(&mut self, e: &'tcx Expr<'tcx>) {
        if let ExprKind::Path(rustc_hir::QPath::Resolved(_, p)) = e.kind {
            if let rustc_hir::def::Res::Def(DefKind::Fn, did) = p.res {
                if let Some(l) = did.as_local() {
                    self.out.push(l);
                }
            }
        }
        rustc_hir::intravisit::walk_expr(self, e);
    }
}

/// The LOCAL fns a `thread_local!` item's initializer references as a value — its deferred init
/// (`__rust_std_internal_init_fn`), which the macro builds inside the `LocalKey::new(...)` construction.
/// That body lives in an inline const, so `enclosing_named_fn` charges it to NO reportable item, and the
/// accessor (`LocalKey::with`) is non-local std — leaving the init's effects orphaned from the call
/// graph. Edging a forcing fn to these propagates them (the thread_local analog of the lazy-init edge).
fn local_key_init_fns(tcx: TyCtxt<'_>, tl_did: LocalDefId) -> Vec<LocalDefId> {
    let Some(body) = tcx.hir_maybe_body_owned_by(tl_did) else {
        return vec![];
    };
    let mut c = FnRefCollector { tcx, out: vec![] };
    rustc_hir::intravisit::Visitor::visit_body(&mut c, &body);
    c.out
}

// --- Taint heuristic (CANDOR_TAINT): flag an effect whose argument derives from a function
// parameter — e.g. `fs::read(format!("/var/cache/{key}"))` where `key` is a param. This is the
// injection class (path traversal / command injection / SSRF). It is an INTRAPROCEDURAL, SYNTACTIC
// heuristic — a review nudge, NOT sound taint analysis. It misses cross-function flow, flow through
// struct fields, and builder chains; it over-flags a param that is actually validated. Honest signal,
// stated limits. ---

/// HirIds of the binding patterns in a function's parameters (the "untrusted input" surface).
/// If `call` invokes one of `caller`'s own callback PARAMETERS (`fn apply(f: impl Fn()) { f() }`),
/// the 0-based parameter index. FREE functions only — a method's `self` would offset arg vs param
/// index, breaking the call-site matching. This is the receiving side of the closure problem: rather
/// than a blanket `Unknown` for `f()`, we defer and resolve it from the fns passed at `apply`'s call
/// sites (see `param_calls`).
fn invoked_param_index(tcx: TyCtxt<'_>, call: &Expr<'_>, caller: LocalDefId) -> Option<usize> {
    if !matches!(tcx.def_kind(caller.to_def_id()), DefKind::Fn) {
        return None; // free fn only
    }
    let ExprKind::Call(callee, _) = call.kind else { return None };
    let ExprKind::Path(rustc_hir::QPath::Resolved(_, path)) = callee.kind else { return None };
    let rustc_hir::def::Res::Local(local) = path.res else { return None };
    let body = tcx.hir_body_owned_by(caller);
    for (i, p) in body.params.iter().enumerate() {
        let mut found = false;
        p.pat.walk(|pat| {
            if let rustc_hir::PatKind::Binding(_, hir_id, _, _) = pat.kind {
                if hir_id == local {
                    found = true;
                }
            }
            true
        });
        if found {
            return Some(i);
        }
    }
    None
}

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
        // CANDOR_REPORTS is a READ-ONLY cross-resolution prefix for ENFORCEMENT modes (policy/strict),
        // which write no report of their own: snapshot every workspace crate once, then enforce with the
        // siblings loaded, so a policy boundary (e.g. a host allowlist) sees effects/hosts that physically
        // live in another crate. Live trust applies, exactly like CANDOR_JSON.
        let (prefix, trust_siblings) = match std::env::var("CANDOR_JSON") {
            Ok(p) => (Some(p), false),
            Err(_) => match std::env::var("CANDOR_BASELINE") {
                Ok(p) => (Some(p), true),
                Err(_) => (std::env::var("CANDOR_REPORTS").ok(), false),
            },
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
            let cd = load_cross_reports(&prefix, &me, &me_kind, trust_siblings);
            self.cross = cd.effects;
            self.cross_hosts = cd.hosts;
            self.cross_cmds = cd.cmds;
            self.cross_paths = cd.paths;
            self.cross_tables = cd.tables;
            // Layering (AS-EFF-009): load the `forbid`-target scopes each sibling function reaches, from
            // the `layerreach` sidecars written earlier in this enforce pass (dependency-first order).
            if !self.layer_rules.is_empty() {
                self.cross_layer_reach = load_layer_reach(&prefix);
            }
            self.reports_prefix = Some(prefix);
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
                        // A `FnDef` path being CAST AWAY from callability (`fs_helper as usize`,
                        // `f as *const ()`) is never invoked through that cast value — a `usize` /
                        // raw-data-pointer can't be called — so propagating the callback's effects to
                        // the casting fn is a fabrication. Suppress the edge ONLY when the parent is a
                        // cast whose TARGET type is non-callable; a `fn`-pointer cast (`f as fn()`) stays
                        // callable, so KEEP the edge there, exactly as for `vec![f]` / `map(f)` / a struct
                        // field. (Soundness: this only ever DROPS an edge for a provably-uncallable cast
                        // target — it can never hide a real call, which goes through `resolve_callee`
                        // below, not this value-reference edge.)
                        let cast_away = match cx.tcx.parent_hir_node(expr.hir_id) {
                            rustc_hir::Node::Expr(p) => match p.kind {
                                ExprKind::Cast(inner, _) if inner.hir_id == expr.hir_id => !matches!(
                                    typeck.expr_ty(p).kind(),
                                    rustc_middle::ty::TyKind::FnPtr(..)
                                        | rustc_middle::ty::TyKind::FnDef(..)
                                        | rustc_middle::ty::TyKind::Closure(..)
                                ),
                                _ => false,
                            },
                            _ => false,
                        };
                        if !cast_away && matches!(cx.tcx.def_kind(*did), DefKind::Fn | DefKind::AssocFn)
                        {
                            self.calls.entry(caller).or_default().insert(local);
                        }
                    }
                }
            }
        }

        // A reference to a local `static` FORCES its initializer. For a deferred-init wrapper
        // (`LazyLock`/`LazyCell`/`OnceLock`/`once_cell::Lazy`/`lazy_static!`/`thread_local!`) the
        // initializer closure runs at the first access SITE, not at the static's declaration — so an
        // effectful initializer reached only by naming the static was charged to the static item but
        // NEVER to the forcing function: a silent under-report (the lazy-init seam, see
        // ui/deferred_effects.rs). Add an edge from the enclosing fn to the static, exactly as the scan
        // engine edges a forcing body to the static's synthetic init unit. Sound and non-fabricating: a
        // static is a reportable item with its own (already-propagated) effect set, so a PURE static
        // contributes nothing; and in safe Rust a static's only route to a RUNTIME effect IS a deferred
        // closure (a plain `static X = const_expr;` is const-evaluated and cannot perform I/O), so no
        // edge to a pure static can ever fabricate. (Conservative on `&STATIC` without a deref — naming
        // forces, matching the scan engine; over-approximation in the safe direction.)
        if let ExprKind::Path(rustc_hir::QPath::Resolved(_, path)) = expr.kind {
            if let rustc_hir::def::Res::Def(DefKind::Static { .. }, did) = path.res {
                if let (Some(static_local), Some(caller)) =
                    (did.as_local(), enclosing_named_fn(cx.tcx, expr.hir_id))
                {
                    self.calls.entry(caller).or_default().insert(static_local);
                }
            }
        }

        // `thread_local!` FORCE: a method call on a `LocalKey` receiver (`KEY.with(…)`, `with_borrow`,
        // `set`, …) runs the thread_local's DEFERRED initializer. The macro places that init in a local fn
        // referenced inside `LocalKey::new(…)` in KEY's initializer — but that reference sits in an inline
        // const (charged to NO reportable item by `enclosing_named_fn`) and the accessor is non-local std,
        // so the init's effects were orphaned from the call graph: a `.with()`-forced effectful
        // thread_local read silently pure. Edge the forcing fn to the init fn (the thread_local analog of
        // the LazyLock static-ref edge above; here the effect is NOT in the item's own initializer but
        // behind the accessor). Sound + non-fabricating: only edges to a LOCAL fn the item references, so a
        // pure-init thread_local (or a std/external LocalKey) contributes nothing. Teeth: ui/thread_local_effects.rs.
        if let ExprKind::MethodCall(_, receiver, _, _) = expr.kind {
            if let Some(typeck) = cx.maybe_typeck_results() {
                if let rustc_middle::ty::TyKind::Adt(adt, _) =
                    typeck.expr_ty(receiver).peel_refs().kind()
                {
                    if cx.tcx.item_name(adt.did()).as_str() == "LocalKey"
                        && cx.tcx.crate_name(adt.did().krate).as_str() == "std"
                    {
                        if let ExprKind::Path(rustc_hir::QPath::Resolved(_, p)) = receiver.kind {
                            if let rustc_hir::def::Res::Def(
                                DefKind::Const { .. } | DefKind::Static { .. },
                                tl_did,
                            ) = p.res
                            {
                                if let (Some(tl_local), Some(caller)) =
                                    (tl_did.as_local(), enclosing_named_fn(cx.tcx, expr.hir_id))
                                {
                                    for init in local_key_init_fns(cx.tcx, tl_local) {
                                        self.calls.entry(caller).or_default().insert(init);
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }

        // IMPLICIT overloaded `Deref`/`DerefMut` calls the compiler inserts as expression ADJUSTMENTS
        // (auto-deref during method resolution, field access through a smart pointer, deref-coercion at
        // a call/arg/return/assignment site). These are NOT `Call`/`MethodCall`/`Unary(Deref)` HIR nodes
        // — they live in `typeck.expr_adjustments(expr)` — so `resolve_callee` never sees them, and a
        // LOCAL effectful `Deref` impl reached only this way was reported neither with its effect nor
        // `Unknown`: silently pure (the smart-pointer hole). Add a call edge per overloaded-deref step
        // EXACTLY as the explicit `Unary(Deref)` arm does. This runs for EVERY expr (a field access /
        // coercion site is not a call, so the `resolve_callee` early-return below would skip it).
        let deref_steps = overloaded_deref_steps(cx, expr);
        if !deref_steps.is_empty() {
            if let Some(caller) = enclosing_named_fn(cx.tcx, expr.hir_id) {
                for step in deref_steps {
                    match step {
                        // Local impl → real edge (its body's effects propagate). Non-local std deref
                        // (`Box`/`Rc`/`Arc`/`Pin`) → drop it: matches the std-trait pure calibration, no
                        // fabrication.
                        DerefStep::Static(did) => {
                            if let Some(local) = did.as_local() {
                                if matches!(cx.tcx.def_kind(did), DefKind::Fn | DefKind::AssocFn) {
                                    self.calls.entry(caller).or_default().insert(local);
                                }
                            }
                        }
                        // An unresolvable/generic overloaded deref: honest `Unknown`, never silent-pure.
                        DerefStep::Unresolved => {
                            self.direct.entry(caller).or_default().insert(UNKNOWN);
                            self.unknown_why
                                .entry(caller)
                                .or_default()
                                .insert("deref:unresolvable overloaded auto-deref".to_string());
                            if self.explain.is_some() {
                                let loc =
                                    cx.tcx.sess.source_map().span_to_diagnostic_string(expr.span);
                                self.sites.entry(caller).or_default().push(EffectSite {
                                    eff: UNKNOWN,
                                    via: "unresolvable overloaded auto-deref".to_string(),
                                    loc,
                                });
                            }
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
                // Receiving side of the closure problem: if this is `caller` invoking its OWN callback
                // parameter, DON'T stamp `Unknown` here — defer it. check_crate_post resolves it from
                // the concrete fns passed at caller's call sites; the `Unknown` only stands if some
                // caller passes an unresolvable callback. (Free fns only — see invoked_param_index.)
                if let Some(i) = invoked_param_index(cx.tcx, expr, caller) {
                    self.param_calls.entry(caller).or_default().insert(i);
                    return;
                }
                self.direct.entry(caller).or_default().insert(UNKNOWN);
                self.unknown_why
                    .entry(caller)
                    .or_default()
                    .insert("callback:fn-pointer / closure".to_string());
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
        // Resolve trait dispatch up-front so the BASE edge can be suppressed when devirt PROVES the
        // call lands on a concrete override (see resolved_override).
        let trait_did = cx.tcx.trait_of_assoc(def_id);
        let mut cha_resolved = false;
        // May be upgraded to `true` if resolution reveals the call is actually virtual (a `dyn` the
        // structural `is_dyn_receiver` check missed) — so the `Unknown` logic below stays honest.
        let mut dynamic = dynamic;
        let devirt = if trait_did.is_some() && !dynamic {
            devirtualize(cx, expr, def_id)
        } else {
            None
        };

        // The BASE edge to def_id (the typeck-resolved callee). For a non-trait call, def_id IS the
        // target. For trait dispatch, def_id is the TRAIT method: a required method is bodyless
        // (harmless), a PROVIDED method carries the DEFAULT body — keep that edge so a non-overriding
        // impl inheriting the default isn't under-reported (`cha_targets` via `impl_item_implementor_ids`
        // misses non-overriding impls, so this base edge is the only thing that counts the default body
        // under generic/CHA dispatch). EXCEPT when devirt PROVED the call resolves to a concrete
        // OVERRIDE (a local target ≠ def_id): the default body then provably never runs for this
        // concrete receiver, so attributing its effects is a confident false positive (a pure override
        // of an effectful default inheriting the default's effect — the precision bug devirt exists to
        // kill, previously covered only for required methods, see ui/inherited.rs).
        let resolved_override =
            matches!(devirt, Some(Devirt::Static(t)) if t.is_local() && t != def_id);
        if !resolved_override {
            add_edge(self, def_id);
        }

        // `?` ERROR-CONVERSION edge: the `?` desugar calls the std `FromResidual::from_residual`, whose
        // body invokes a LOCAL `<E2 as From<E1>>::from` to convert the error — invisible THROUGH the
        // non-local std fn, so an effectful error conversion reached only via `?` looked pure. Recover
        // that one edge from the call's types (see from_residual_local_edge). Adds soundness only.
        if let Some(from_did) = from_residual_local_edge(cx, expr, def_id) {
            add_edge(self, from_did);
        }

        // RETURN-TYPE-directed std drivers (`collect`/`into`/`parse`): a std method selects a LOCAL
        // `FromIterator`/`From`/`FromStr` impl by the call's RESULT type and runs it through its non-local
        // body — invisible like the `?`-From edge above. Recover that one local edge. Soundness only.
        if let Some(driver_did) = return_type_driver_local_edge(cx, expr, def_id) {
            add_edge(self, driver_did);
        }

        // `mem::drop(x)` relocates `x`'s destructor into the non-local mem::drop body — recover the edge
        // to the local `Drop::drop` (explicit early-release of an effectful guard, else silent-pure).
        if let Some(drop_did) = mem_drop_local_edge(cx, expr, def_id) {
            add_edge(self, drop_did);
        }

        // HOLE 1 — std ITERATOR-COMBINATOR / consumer driving a LOCAL `Iterator::next` / `into_iter`.
        // `It.for_each(..)`, `It.map(..).collect()`, `It.sum()`, `for x in it.map(..) {}` resolve the
        // OUTER call to a std `Iterator`/`Sum`/`FromIterator`/`IntoIterator` method (pure for itself),
        // but its body pulls the receiver's LOCAL `next()` — invisible THROUGH the std body. Recover the
        // edge to that local `next`/`into_iter` (precise), or honest `Unknown` if a driver was hit but
        // the local impl couldn't be pinned. A wholly-std receiver (`vec.iter()`) contributes nothing.
        if let Some(td) = trait_did {
            match iter_combinator_local_edges(cx, expr, td, def_id) {
                Some(CallbackEdges::Local(edges)) => {
                    for e in edges {
                        add_edge(self, e);
                    }
                }
                Some(CallbackEdges::Unknown(why)) => {
                    self.direct.entry(caller).or_default().insert(UNKNOWN);
                    self.unknown_why.entry(caller).or_default().insert(why.clone());
                    if self.explain.is_some() {
                        let loc = cx.tcx.sess.source_map().span_to_diagnostic_string(expr.span);
                        self.sites.entry(caller).or_default().push(EffectSite {
                            eff: UNKNOWN,
                            via: why,
                            loc,
                        });
                    }
                }
                // No CONCRETE local iterator at this site. If the receiver is a GENERIC PARAM over an
                // iter-driver trait (`fn run<I: Iterator>(it: I) { it.for_each(..) }`), the concrete `I`
                // can't be pinned standalone — it could be a LOCAL effectful iterator. Record the
                // enclosing fn for a REPORT-ONLY honest `Unknown` (injected post-fixpoint, NON-propagating)
                // so it's honest standalone without re-polluting the precise local callers that
                // monomorphize it. Scoped to iter-driver traits + a generic receiver: ordinary iteration
                // (concrete/std receiver) and `Clone`/`Display` generic dispatch are untouched — no flood.
                None => {
                    let tk = cx.tcx.crate_name(td.krate);
                    let ti = cx.tcx.item_name(td);
                    if is_iter_driver_trait(tk.as_str(), ti.as_str()) {
                        if let ExprKind::MethodCall(_, receiver, _, _) = expr.kind {
                            if let Some(typeck) = cx.maybe_typeck_results() {
                                let recv_ty = typeck.expr_ty_adjusted(receiver);
                                if iter_receiver_is_generic_param(recv_ty) {
                                    let method = cx.tcx.item_name(def_id);
                                    self.generic_iter_unknown
                                        .entry(caller)
                                        .or_insert_with(|| format!("generic-iter:{method}"));
                                }
                            }
                        }
                    }
                }
            }
        }

        // HOLE 2 — `core::fmt` formatting (`println!`/`format!`/`write!`) reaching a LOCAL `Display`/
        // `Debug`/… `fmt`. The macro lowers each value to `core::fmt::rt::Argument::new_<kind>(&value)`;
        // the real `fmt(&value, f)` happens inside std's fmt machinery, invisible — so a local effectful
        // `impl Display for T` is reached silently. Recover the edge to that local `fmt`. A std `Display`
        // (`i32`/`String`) resolves non-local → no edge → pure.
        if let Some(CallbackEdges::Local(edges)) = fmt_argument_local_edge(cx, expr, def_id) {
            for e in edges {
                add_edge(self, e);
            }
        }

        // HOLE 2b — a non-local std DRIVER method (`to_string`/`contains`/`clone`/`sort`/set-`insert`)
        // running a LOCAL effectful `Display`/`PartialEq`/`Clone`/`Ord`/`Hash` impl over its receiver/
        // element type (sweep [25]/[26]). Recover the local edge; pure/std targets resolve non-local.
        for e in std_driver_local_edges(cx, expr, def_id) {
            add_edge(self, e);
        }

        // HOLE 2c — the WRITER side of `write!`/`writeln!`: `w.write_fmt(..)`'s default impl drives a
        // LOCAL effectful `fmt::Write::write_str` / `io::Write::write` on the receiver. Recover that edge
        // (the arg-Display side is HOLE 2 above). Teeth: ui/write_trait.rs.
        if let Some(e) = fmt_write_local_edge(cx, expr, def_id) {
            add_edge(self, e);
        }

        // GENERIC-RECEIVER hole — a LOCAL generic callee whose body drives a trait method on one of its
        // OWN generic params (`run_generic::<I>(it){ it.for_each(..) }` → internally `<I as Iterator>::
        // next`). Inside the callee `I` is an unresolved `Param`, so candor reports it pure for itself and
        // the CONCRETE impl's effect is lost at every monomorphizing call site too. At THIS site the
        // concrete substs are known (`I=Rows`): re-resolve the callee's internal generic trait dispatches
        // UNDER those substs (`<Rows as Iterator>::next` → the LOCAL `Rows::next`) and edge `caller` to
        // each pinned LOCAL impl — precise, never fabricated (a std/pure iterator resolves non-local → no
        // edge), no flood (`<u32 as Clone>::clone` resolves non-local → no edge). Adds soundness only.
        for e in generic_callee_local_edges(cx, expr, def_id) {
            add_edge(self, e);
        }

        // Closure-flow bookkeeping: record what's passed at each arg position of a FREE fn call, so a
        // callback parameter that fn invokes can later be resolved to these concrete targets. A named
        // fn (`FnDef`) is a resolvable target; a closure / fn-pointer / generic value is unresolvable
        // (forces that position back to `Unknown`). Free fns only → arg index == param index.
        if let ExprKind::Call(_, args) = expr.kind {
            if matches!(cx.tcx.def_kind(def_id), DefKind::Fn) {
                if let Some(typeck) = cx.maybe_typeck_results() {
                    for (i, arg) in args.iter().enumerate() {
                        // ONLY a LOCAL named fn is a resolvable callback target (we can edge to its
                        // body). A non-local fn-item, a closure, a fn-pointer, or any opaque/`dyn`
                        // callable (`impl Fn` return, `&dyn Fn`, `Box<dyn Fn>`) we can't enumerate —
                        // record it as a PER-SITE unresolvable so this caller's deferred `Unknown`
                        // STANDS instead of being silently dropped. (A non-callable value at a
                        // non-callback position also lands here; harmless — that key is consulted only
                        // for an invoked param, where the arg is necessarily a callable.) SOUNDNESS:
                        // routing a *non-local* fn into the resolvable set would let the resolver see a
                        // non-empty target set, filter it to an empty `locals`, and add neither an edge
                        // nor the `Unknown` → a false `pure`; routing it to the unresolvable set is the
                        // honest fallback. The flow is keyed by the CALLER (per call site) so a
                        // callback's effects reach the specific fn that passed it, never every caller of
                        // the HOF (the union fabrication — see `callback_sites`).
                        match typeck.expr_ty(arg).kind() {
                            rustc_middle::ty::TyKind::FnDef(t, _)
                                if t.as_local().is_some()
                                    && matches!(cx.tcx.def_kind(*t), DefKind::Fn | DefKind::AssocFn) =>
                            {
                                self.callback_sites
                                    .entry((caller, def_id, i))
                                    .or_default()
                                    .insert(*t);
                            }
                            _ => {
                                self.callback_site_unknown.insert((caller, def_id, i));
                            }
                        }
                    }
                }
            }
        }

        // Resolve a trait-method call to the impls whose effects it could perform. PREFER
        // devirtualization: a call on a CONCRETE (non-`dyn`) receiver of a LOCAL trait
        // dispatches to exactly ONE impl, and we can see its body — so use it instead of
        // CHA-expanding to every impl (the over-approximation that made a pure `self.applies()`
        // inherit a sibling rule's effect — CRITIQUE §9). CHA remains the sound fallback for
        // `dyn`/generic dispatch we can't pin down. (Non-local traits: neither sees the body;
        // left to the `Unknown` logic below.)
        if trait_did.is_some() {
            // Prefer a real devirtualization (computed above) to a LOCAL impl whose body we can see,
            // over CHA-expanding to every impl. We attempt this for ANY non-`dyn` dispatch, not just
            // LOCAL traits: a LOCAL impl of a NON-local trait is exactly where the silent holes live —
            // a custom `Future::poll` reached via `.await`, an effectful `Clone`/`Display`/operator
            // impl. CHA is the sound fallback when resolution lands non-local, stays virtual, or can't
            // pin the target.
            // CHA fallback: enumerate the local impls the dispatch could reach. Used when devirt didn't
            // resolve to a LOCAL impl (non-local target, still-virtual, or unresolvable).
            let mut cha = |this: &mut Self| {
                for target in cha_targets(cx.tcx, def_id) {
                    cha_resolved = true;
                    add_edge(this, target);
                }
            };
            match devirt {
                // A real, static resolution to a LOCAL impl — the one true target.
                Some(Devirt::Static(target)) if target.is_local() => {
                    add_edge(self, target);
                    cha_resolved = true;
                }
                // Still virtual: resolution proved this is dynamic dispatch the structural check missed.
                // Mark it dynamic so a non-local trait (CHA empty) gets an honest `Unknown` below, and
                // CHA the local impls for a local trait. NEVER edge to the bodyless trait method.
                Some(Devirt::StillVirtual) => {
                    dynamic = true;
                    cha(self);
                }
                // Resolved non-local, or couldn't resolve: CHA the local impls.
                _ => cha(self),
            }
        }

        // Honest `Unknown` only when dispatch is genuinely unresolvable here:
        //  - a `dyn` call over a NON-local trait (we can't see its impl bodies), or
        //  - (paranoid) any trait dispatch CHA couldn't pin to local impls.
        // ...but NOT for conventionally-pure std traits (Display/Error/…), where the
        // overwhelmingly-pure dispatch would otherwise flood reports with false Unknowns.
        if let Some(td) = trait_did {
            let tk = cx.tcx.crate_name(td.krate);
            let tk = tk.as_str();
            let ti = cx.tcx.item_name(td);
            let ti = ti.as_str();
            // `core::fmt::Write` is pure formatting — never `Unknown`, even via `&mut dyn fmt::Write`.
            let pure = is_pure_std_trait(tk, ti) || is_pure_fmt_write(tk, ti);
            // Generic (non-`dyn`) dispatch is assumed pure by default (marking it all Unknown floods —
            // that's `CANDOR_PARANOID`). EXCEPT over a known-effectful std trait (`io::Read`/`Write`/…),
            // where "assumed pure" is a real under-report: the reader/writer behind the generic could be
            // a file or socket. So those are Unknown by default too — bounded, doesn't flood.
            let effectful_dispatch = is_effectful_std_trait(tk, ti);
            if !cha_resolved && !pure && (dynamic || self.paranoid || effectful_dispatch) {
                self.direct.entry(caller).or_default().insert(UNKNOWN);
                self.unknown_why
                    .entry(caller)
                    .or_default()
                    .insert(format!("dispatch:{}", cx.tcx.def_path_str(td)));
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
        // FLOOR DISCLOSURE (sweep [4]/[19]): a DIRECT external call (not a trait dispatch — those CHA-resolve
        // to local impls, or are disclosed as `Unknown` above) that κ does NOT classify is a reach candor
        // cannot see through. The deep engine floors it to pure like the syntactic engines; record the crate
        // so the fn's `invisible` qualifies its pure verdict (the honesty contract, propagated below). std-
        // like crates are known-pure-frontier, excluded — matching the `encountered` coverage filter.
        if effect.is_none() && trait_did.is_none() && !def_id.is_local()
            && !matches!(crate_name.as_str(), "std" | "core" | "alloc" | "proc_macro" | "test")
        {
            self.invisible_direct.entry(caller).or_default().insert(crate_name.to_string());
        }
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
                } else if candor_classify::is_net_establishing(path.rsplit("::").next().unwrap_or("")) {
                    // a host-ESTABLISHING Net call with no captured host literal → runtime endpoint, invisible
                    // to the gate (masking; sweep [3]/[7]). Use-verbs (write/read/send) are not establishing.
                    self.incomplete_direct.entry(caller).or_default().insert("Net");
                }
            }
            // Non-breaking Exec/Fs refinements: the literal command (`Command::new("git")`) / path
            // (`fs::read("/etc/x")`) is the first string-literal argument. Gated to the built-in
            // classifier (a user `extra` rule's arg shape is unknown). Feeds the `allow Exec/Fs …`
            // allowlists, exactly as host literals feed `allow Net …`.
            if builtin == Some("Exec") {
                if let Some(cmd) = first_str_lit_arg(expr) {
                    // Capture the program head + refine the cliff (spec §4 ⟨0.5⟩) ONLY at a program-
                    // NAMING call (`new`/`cmd`), an ALLOWLIST — not "any method except a known modifier".
                    // A whole-crate-Exec crate classifies EVERY method as Exec, so a denylist leaked a
                    // getter (`get_env("psql")` reads a KEY) → fabricated Db + polluted `cmds` (review find).
                    if is_cmd_naming_method(path.rsplit("::").next().unwrap_or("")) {
                        self.direct
                            .entry(caller)
                            .or_default()
                            .extend(classify_command_head(&cmd).iter().copied());
                        self.exec_cmds_direct.entry(caller).or_default().insert(cmd);
                    }
                } else if is_cmd_naming_method(path.rsplit("::").next().unwrap_or("")) {
                    // a program-NAMING Exec call (`Command::new(runtime_var)`) with no literal head → the
                    // command is invisible to the gate (masking; sweep [3]/[7]).
                    self.incomplete_direct.entry(caller).or_default().insert("Exec");
                }
            }
            if builtin == Some("Fs") {
                if let Some(p) = first_str_lit_arg(expr) {
                    self.fs_paths_direct.entry(caller).or_default().insert(p);
                } else {
                    // a path-NAMING Fs call (`fs::write(p,…)`/`File::open(p)`) with no literal path → the
                    // path is a runtime value, invisible to the gate (masking; the AS-EFF-008 guard
                    // generalized from Net/Exec to Fs). Use the SHARED establishing-allowlist predicate
                    // (`is_fs_path_arg`), and EXCLUDE the path-stat METHODS whose path is the RECEIVER, not
                    // an arg (`p.metadata()`/`p.exists()` resolve to `std::path::Path::*`/`PathBuf::*`) —
                    // those carry no path arg, so a missing literal there must not false-positive.
                    let leaf = path.rsplit("::").next().unwrap_or("");
                    let stat_method = path.starts_with("std::path::Path::")
                        || path.starts_with("std::path::PathBuf::");
                    if !stat_method && candor_classify::is_fs_path_arg(leaf) {
                        self.incomplete_direct.entry(caller).or_default().insert("Fs");
                    }
                }
            }
            // Non-breaking Db refinement: table-position identifiers in a SQL string literal are the
            // tables this call statically reaches. Same posture as hosts/cmds/paths: the decidable
            // subset only — `tables_in_sql` yields nothing for a dynamically-built query (the gate's
            // opaque case), never a guess. Feeds `allow Db in <scope> <table>…` (AS-EFF-008).
            if builtin == Some("Db") {
                if let Some(sql) = first_str_lit_arg(expr) {
                    let ts = candor_classify::tables_in_sql(&sql);
                    if !ts.is_empty() {
                        self.db_tables_direct.entry(caller).or_default().extend(ts);
                    }
                    // (a literal SQL with no table — `SELECT 1` — is visible-but-tableless, NOT incomplete.)
                } else if candor_classify::is_db_query_arg(path.rsplit("::").next().unwrap_or("")) {
                    // a SQL-QUERY-bearing Db call (`con.execute(sql,…)`/`query`/`prepare`) with no literal
                    // query → the table is a runtime value, invisible to the gate (masking; the AS-EFF-008
                    // guard generalized from Net/Exec to Db). The allowlist excludes build-then-execute
                    // terminals (`fetch_all`/`load`/`all`) and lifecycle ops (`connect`/`open`/`begin`),
                    // whose query is built structurally (no maskable string literal).
                    self.incomplete_direct.entry(caller).or_default().insert("Db");
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
        } else if !def_id.is_local() {
            // Cross-crate: a call into one of THIS project's other crates (its lib, a sibling
            // workspace member). For a TRAIT-method call the callee `def_id` is the trait method, but
            // the dependency keyed its report by the concrete IMPL method — so devirtualize to that
            // impl first (when the receiver is concrete), else the lookup would always miss.
            let key_did = match devirtualize(cx, expr, def_id) {
                Some(Devirt::Static(did)) => did,
                _ => def_id, // still-virtual / unresolvable → the trait method is the key fallback
            };
            // Layering across crates (AS-EFF-009): record this cross-crate callee (its DefPathHash +
            // path) so check_crate_post can compute reachability — a direct dependency (callee path
            // matches a `forbid -> B` scope) or one laundered through that crate (its `layerreach`
            // summary, keyed by the hash). Recorded from the callee identity alone, so it works even
            // with no sibling report loaded.
            if !self.layer_rules.is_empty() {
                let callee_path = cx.tcx.def_path_str(key_did);
                self.cross_callees.entry(caller).or_default().push((dph(cx.tcx, key_did), callee_path));
            }
            if self.cross.is_empty() {
                return;
            }
            // Inherit the callee's already-transitive effects, looked up by its stable DefPathHash
            // (matches whether the dependency emitted it locally or we see it externally), plus its
            // literal detail (hosts / commands / paths) into the caller's direct sets, so within-crate
            // propagation carries them up to every transitive caller — the allowlists then see a value
            // that physically lives in another crate.
            let cross_key = dph(cx.tcx, key_did);
            if let Some(hs) = self.cross_hosts.get(&cross_key) {
                self.net_hosts_direct.entry(caller).or_default().extend(hs.iter().cloned());
            }
            if let Some(cs) = self.cross_cmds.get(&cross_key) {
                self.exec_cmds_direct.entry(caller).or_default().extend(cs.iter().cloned());
            }
            if let Some(ps) = self.cross_paths.get(&cross_key) {
                self.fs_paths_direct.entry(caller).or_default().extend(ps.iter().cloned());
            }
            if let Some(ts) = self.cross_tables.get(&cross_key) {
                self.db_tables_direct.entry(caller).or_default().extend(ts.iter().cloned());
            }
            if let Some(effs) = self.cross.get(&cross_key).cloned() {
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
        // Bet 4 spike (CANDOR_MIR=1): run the experimental MIR-based extractor INSTEAD of the HIR
        // analysis and return. Non-production — it exists to gather evidence for a possible core rewrite.
        if std::env::var("CANDOR_MIR").is_ok() {
            mir_spike::run(cx.tcx);
            return;
        }
        // Closure-flow resolution (receiving side), PER CALL SITE. A HOF `H` that invokes a callback
        // parameter doesn't perform the callback's effects ITSELF — the *caller* that passed the
        // callback does. So instead of unioning every callback ever passed to `H` and edging `H` to all
        // of them (which then leaked to EVERY caller of `H` — a pure caller passing a pure callback
        // inheriting a sibling caller's effectful one, the fabrication this fixes), attribute each
        // callback to the SPECIFIC caller that passed it: edge `caller -> callback_target`.
        //
        // The HOF stays HONEST in its OWN report (SPEC §4 trust contract: an unresolvable invocation is
        // never silently pure). From `H`'s own standalone standpoint it invokes an opaque callable, so
        // it ALWAYS carries `Unknown` for the invoked param — but a NON-PROPAGATING one (injected into
        // the report AFTER the fixpoint, see `hof_param_unknown` below), so it never flows back down the
        // normal `caller -> H` edge to re-pollute the precise local callers we just fixed. This is the
        // crux of the per-site model: the HOF is honestly indeterminate standalone, while each caller
        // gets the EXACT effect of the callback IT passed.
        //
        // No under-report: every concrete invocation of `H`'s param is attributed to SOME function that
        // can't drop it — a local caller (edge to the resolvable target, or its own `Unknown` for an
        // unresolvable callback, recorded in `callback_site_unknown`), AND `H` itself keeps the honest
        // `Unknown` for any caller we can't see (an external crate, an entry point, or simply a HOF that
        // is never called at all but still reported on its own).
        let param_calls = std::mem::take(&mut self.param_calls);
        let callback_sites = std::mem::take(&mut self.callback_sites);
        let callback_site_unknown = std::mem::take(&mut self.callback_site_unknown);
        // HOFs that must carry the non-propagating, report-only `Unknown` for an invoked param.
        let mut hof_param_unknown: HashSet<LocalDefId> = HashSet::new();
        for (hof, indices) in &param_calls {
            for &i in indices {
                // PER-SITE: edge each LOCAL caller to the resolvable named callback it passed HERE.
                for ((caller, h, pi), targets) in &callback_sites {
                    if *h == hof.to_def_id() && *pi == i {
                        let locals: Vec<LocalDefId> = targets
                            .iter()
                            .filter(|t| matches!(cx.tcx.def_kind(**t), DefKind::Fn | DefKind::AssocFn))
                            .filter_map(|t| t.as_local())
                            .collect();
                        if !locals.is_empty() {
                            self.calls.entry(*caller).or_default().extend(locals);
                        }
                    }
                }
                // PER-SITE: a caller that passed an UNRESOLVABLE callback (closure / fn-ptr / non-local
                // fn / generic value) carries the honest `Unknown` ITSELF — it genuinely can't see what
                // it handed over. (This DOES propagate to that caller's callers, correctly: they reach
                // an indeterminate effect through it.)
                for (caller, h, pi) in &callback_site_unknown {
                    if *h == hof.to_def_id() && *pi == i {
                        self.direct.entry(*caller).or_default().insert(UNKNOWN);
                        self.unknown_why
                            .entry(*caller)
                            .or_default()
                            .insert("callback:unresolvable callback passed".to_string());
                    }
                }
                // The HOF itself: ALWAYS honest `Unknown` for the invocation (it invokes an opaque
                // param). Report-only (non-propagating) so it never leaks to the precise local callers.
                hof_param_unknown.insert(*hof);
                self.unknown_why
                    .entry(*hof)
                    .or_default()
                    .insert("callback:invoked param (opaque from the HOF's own standpoint)".to_string());
            }
        }

        // Implicit-drop edges (narrow, production use of MIR): a value going out of scope runs its
        // `Drop::drop`, which HIR has no node for — so an effectful guard (I/O on drop) was silently
        // dropped from the effect graph. Add `caller -> drop-impl` edges from MIR's `Drop` terminators
        // so those effects propagate. (See eval/bet4/FINDINGS.md — the Bet 4 spike surfaced this hole.)
        for (caller, drop_impl) in mir_spike::drop_edges(cx.tcx) {
            self.calls.entry(caller).or_default().insert(drop_impl);
        }

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

        let mut eff = propagate(eff, &self.calls);
        // NON-PROPAGATING HOF param-invocation `Unknown`: injected AFTER the fixpoint so an exported
        // HOF stays honest in its OWN report (an external caller can't be enumerated) without that
        // `Unknown` flowing back down `caller -> HOF` to re-pollute the precise local callers, whose
        // callbacks were already attributed per call site above. (If a HOF already carries a real
        // `Unknown` from its body, this is a harmless no-op on the set.)
        for hof in &hof_param_unknown {
            eff.entry(*hof).or_default().insert(UNKNOWN);
        }
        // NON-PROPAGATING generic-iterator `Unknown`: a fn driving an iter-driver trait method on its OWN
        // generic param (`fn run<I: Iterator>(it: I) { it.for_each(..) }`) can't pin the concrete `I`
        // standalone (it could be a LOCAL effectful iterator), so it stays honest in its own report.
        // Injected AFTER the fixpoint, exactly like the HOF case, so it never flows back down a precise
        // local caller that already monomorphized it to the concrete impl (`caller`'s precise `Fs`).
        let generic_iter_unknown = std::mem::take(&mut self.generic_iter_unknown);
        for (f, why) in &generic_iter_unknown {
            eff.entry(*f).or_default().insert(UNKNOWN);
            self.unknown_why.entry(*f).or_default().insert(why.clone());
        }

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
        // invisible[f] = invisible_direct[f] ∪ ⋃ { invisible[g] : g ∈ calls[f] } — the blind crates a fn
        // transitively reaches (the disclosed floor; sweep [4]/[19]). Same graph as hosts/effects.
        let invisibleacc = propagate(self.invisible_direct.clone(), &self.calls);
        // incomplete[f] = incomplete_direct[f] ∪ ⋃ { incomplete[g] : g ∈ calls[f] } — a caller transitively
        // reaches a callee's invisible endpoint, so it inherits the surface-incompleteness (masking [3]/[7]).
        let incompleteacc = propagate(self.incomplete_direct.clone(), &self.calls);
        let cmdsacc = propagate(self.exec_cmds_direct.clone(), &self.calls);
        let pathsacc = propagate(self.fs_paths_direct.clone(), &self.calls);
        let tablesacc = propagate(self.db_tables_direct.clone(), &self.calls);

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
                         the regression guard CANNOT evaluate this crate (fail closed)"
                    );
                    // Fail-closed machine signal: a per-crate baseline gap (a NEW workspace member, a
                    // renamed crate, a typo'd prefix) must not read as a silent pass — the wrapper maps
                    // this sentinel to exit 2 ("guard not evaluated"), distinct from a real AS-EFF-005
                    // (exit 1). A no-op when no CANDOR_VIOLATIONS sink is set (direct `cargo dylint`
                    // runs still get the loud stderr above).
                    self.record_violation("GUARD-UNAVAILABLE", &file);
                }
                loaded
            }
            Err(_) => None,
        };
        let any_enforce = strict_var.is_some()
            || no_ambient_var.is_some()
            || baseline.is_some()
            || !self.policy.is_empty()
            || !self.allow_rules.is_empty()
            || !self.layer_rules.is_empty()
            || self.taint;

        // CANDOR_JSON takes the report path below and `continue`s past every enforcement gate, so an
        // enforcement var set ALONGSIDE it is silently a no-op — a CI step that means to fail on a
        // violation would pass green. Make that loud instead of letting it pass unnoticed.
        if json_path.is_some() && any_enforce {
            eprintln!(
                "candor: CANDOR_JSON is set, so this run only WRITES a report — enforcement \
                 (CANDOR_STRICT/POLICY/BASELINE/NO_AMBIENT/taint) is NOT applied. Run the enforcing \
                 mode WITHOUT CANDOR_JSON to actually gate."
            );
        }

        // Stable ordering for reproducible output.
        let mut items: Vec<LocalDefId> = eff.keys().copied().collect();
        items.sort_by_cached_key(|f| cx.tcx.def_path_str(f.to_def_id()));

        // AS-EFF-009 layering: compute, for each local function, the set of `forbid`-TARGET scopes it
        // transitively reaches, then flag any function in a `from` scope that reaches its paired `to`.
        // The reach surface is seeded from each function's DIRECT callees — local or cross-crate — whose
        // path matches a target scope (a direct dependency on B), PLUS the reach of any cross-crate
        // callee as recorded in its `layerreach` sidecar (a dependency *laundered through* that crate).
        // `propagate` then carries it transitively over the local call graph. This unifies within-crate,
        // direct cross-crate, and third-crate-laundered layering. The sidecar we write below lets the
        // crates that depend on THIS one do the same. `reach` maps a `to`-scope → an example reached path
        // (for the diagnostic); only the scope set participates in propagation.
        let mut layer_viol: HashMap<LocalDefId, Vec<(String, String)>> = HashMap::new();
        if !self.layer_rules.is_empty() {
            // Scope-match against the CRATE-PREFIXED path (`<crate>::<path>`), because a crate's own
            // functions' `def_path_str` omits the crate name — so a `from`/`to` scope spelled as the
            // crate name would otherwise match nothing (a silent no-op). Module/type-name scopes still
            // match (the segment is present either way). Cross-crate callee paths already carry the
            // crate, so those (in `cross_callees`) are matched as-is, not through `name_of`.
            let name_of = |g: LocalDefId| format!("{krate}::{}", cx.tcx.def_path_str(g.to_def_id()));
            let tos: BTreeSet<&str> = self.layer_rules.iter().map(|r| r.to.as_str()).collect();
            // Seed: scopes each function reaches via a DIRECT callee (local path match, cross path match,
            // or cross callee's sidecar reach). Track an example path per (fn, scope) for the message.
            let mut seed: HashMap<LocalDefId, BTreeSet<String>> = HashMap::new();
            let mut example: HashMap<(LocalDefId, String), String> = HashMap::new();
            let note = |seed: &mut HashMap<LocalDefId, BTreeSet<String>>,
                            example: &mut HashMap<(LocalDefId, String), String>,
                            f: LocalDefId,
                            scope: &str,
                            via: &str| {
                seed.entry(f).or_default().insert(scope.to_string());
                example.entry((f, scope.to_string())).or_insert_with(|| via.to_string());
            };
            for (&f, callees) in &self.calls {
                for &c in callees {
                    let cpath = name_of(c);
                    for to in &tos {
                        if scope_matches(&cpath, to) {
                            note(&mut seed, &mut example, f, to, &cpath);
                        }
                    }
                }
            }
            for (&f, ccs) in &self.cross_callees {
                for (hash, cpath) in ccs {
                    for to in &tos {
                        if scope_matches(cpath, to) {
                            note(&mut seed, &mut example, f, to, cpath);
                        }
                    }
                    if let Some(reached) = self.cross_layer_reach.get(hash) {
                        for s in reached {
                            note(&mut seed, &mut example, f, s, cpath);
                        }
                    }
                }
            }
            let reach = propagate(seed, &self.calls);

            // Write this crate's sidecar (hex DefPathHash -> reached scopes) for dependent crates.
            if let Some(prefix) = &self.reports_prefix {
                let mut sidecar: HashMap<String, Vec<String>> = HashMap::new();
                for (f, scopes) in &reach {
                    if !scopes.is_empty() {
                        sidecar.insert(dph_hex(cx.tcx, f.to_def_id()), scopes.iter().cloned().collect());
                    }
                }
                write_layer_reach(&layer_reach_path(prefix, &krate.to_string(), &kinds), &sidecar);
            }

            // Flag: a function in scope `from` that reaches its rule's `to` scope.
            for (&f, scopes) in &reach {
                let fname = name_of(f);
                for rule in &self.layer_rules {
                    if scopes.contains(&rule.to) && scope_matches(&fname, &rule.from) {
                        let via = example.get(&(f, rule.to.clone())).cloned().unwrap_or_else(|| rule.to.clone());
                        layer_viol.entry(f).or_default().push((rule.raw.clone(), via));
                    }
                }
            }
        }

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
            // Crate-prefixed form for policy scope matching (so a crate-name scope isn't a silent
            // no-op — see the layering note above). `name` itself stays unprefixed for display.
            let scope_name = format!("{krate}::{name}");
            let declared = declared_caps(cx.tcx, f);
            let mut direct = self.direct.get(&f).cloned().unwrap_or_default();
            // A param-invoking HOF's own `Unknown` is DIRECT (the opacity originates in this fn's body,
            // invoking an opaque callable) — but it is kept OUT of `self.direct` so it does NOT propagate
            // down `caller -> HOF` and re-pollute the precise per-site callers. Re-add it HERE, for
            // display/JSON only, so the report reads "{ Unknown }" (no `*`, not "via callee") and the
            // `unknown_why` origin tag is emitted — matching the SPEC §4 trust contract for an opaque
            // self-invocation.
            if hof_param_unknown.contains(&f) {
                direct.insert(UNKNOWN);
            }
            // Same for a generic-iterator driver: its `Unknown` ORIGINATES in this fn's body (driving an
            // iter-driver method on its own generic param), kept out of `self.direct` so it doesn't
            // propagate to precise callers. Re-add for display/JSON so the `unknown_why` origin tag
            // (`generic-iter:<method>`) is emitted.
            if generic_iter_unknown.contains_key(&f) {
                direct.insert(UNKNOWN);
            }
            let has_unknown = effs.contains(UNKNOWN);
            let undeclared = undeclared_effects(effs, &declared);
            let unused = overdeclared_effects(&declared, effs);

            if json_path.is_some() {
                // Effect-free fns MAY be omitted (spec §2) — but NOT when the fn carries a disclosed
                // floor (`invisible`): dropping it erases the disclosure, and its absent entry reads as
                // an UNQUALIFIED pure claim — the exact silent under-report the field exists to prevent.
                // (The syscall oracle caught this live: minreq/xshell/subprocess/fs_extra callers were
                // floored to invisible at the call site, then omitted here — "certain pure" downstream.)
                let invisible_only = invisibleacc.get(&f).is_some_and(|s| !s.is_empty());
                if effs.is_empty() && declared.is_empty() && !invisible_only {
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
                    entry_point: rust_is_entry_point(cx, f.to_def_id(), entry_fn),
                    hash: dph_hex(cx.tcx, f.to_def_id()),
                    // Empty when the kind is incomplete (FS_UNKNOWN — Fs reached cross-crate with no
                    // recorded detail): present no read/write claim rather than a misleading partial.
                    fs: match fsacc.get(&f) {
                        Some(s) if !s.contains(FS_UNKNOWN) => owned_set(s),
                        _ => Vec::new(),
                    },
                    // The literal Net endpoints statically visible from here (empty = none visible).
                    hosts: hostsacc.get(&f).map(|s| s.iter().cloned().collect()).unwrap_or_default(),
                    // Literal subprocess commands / filesystem paths statically visible (empty = none).
                    cmds: cmdsacc.get(&f).map(|s| s.iter().cloned().collect()).unwrap_or_default(),
                    paths: pathsacc.get(&f).map(|s| s.iter().cloned().collect()).unwrap_or_default(),
                    tables: tablesacc.get(&f).map(|s| s.iter().cloned().collect()).unwrap_or_default(),
                    // Why this fn DIRECTLY introduces Unknown (origin tags), so a consumer can tell
                    // improvable opacity from irreducible. Only when `direct` actually carries Unknown
                    // (a cross-crate-inherited Unknown is transitive, not introduced here).
                    unknown_why: if direct.contains(UNKNOWN) {
                        self.unknown_why.get(&f).map(|s| s.iter().cloned().collect()).unwrap_or_default()
                    } else {
                        Vec::new()
                    },
                    // The blind external crates this fn transitively reaches (the disclosed floor): empty
                    // unless it calls into an unmodeled, unwalkable crate — then `inferred` is a LOWER bound.
                    invisible: invisibleacc.get(&f).map(|s| s.iter().cloned().collect()).unwrap_or_default(),
                    // Effects with a masking-incomplete surface — carried so a cross-crate consumer inherits
                    // the incompleteness ([3]/[7]/[30]); the gate already fails closed locally on it.
                    incomplete: incompleteacc.get(&f).map(|s| s.iter().map(|e| e.to_string()).collect()).unwrap_or_default(),
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
                        self.record_violation("AS-EFF-005", &name);
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
                    if !scope_matches(&scope_name, scope) {
                        continue;
                    }
                }
                // A `deny`d effect that's actually present is a definite violation; `Unknown` is an
                // UNPROVABLE case — the function makes an unresolvable call that COULD perform the
                // forbidden effect, so the boundary can't be certified. Both must flag (silently
                // passing an unprovable boundary is the §4 trust contract's forbidden direction; under
                // a policy-only run there's no AS-EFF-003 backstop). A `pure` rule already treats any
                // effect — including `Unknown` — as a violation.
                let bad: Vec<&str> = if rule.effects.is_empty() {
                    effs.iter().copied().collect() // `pure` rule: any effect is a violation
                } else {
                    effs.iter()
                        .copied()
                        .filter(|e| *e == UNKNOWN || rule.effects.contains(e))
                        .collect()
                };
                if !bad.is_empty() {
                    let scope = rule.scope.as_deref().map(|s| format!(" (scope `{s}`)")).unwrap_or_default();
                    let caveat = if bad.contains(&UNKNOWN) {
                        " — `Unknown` is an unresolvable call that MAY perform a forbidden effect; \
                         the boundary can't be certified"
                    } else {
                        ""
                    };
                    self.record_violation("AS-EFF-006", &name);
                    span_lint(
                        cx,
                        CANDOR,
                        span,
                        format!(
                            "[AS-EFF-006] `{name}` performs {{ {} }}, forbidden by policy{scope}: `{}`{caveat}",
                            bad.join(", "),
                            rule.raw
                        ),
                    );
                }
            }

            // AS-EFF-008 (CANDOR_POLICY allowlist): a function in scope performs an effect (Net/Exec/Fs)
            // reaching a literal OUTSIDE its declared allowlist — or one candor cannot pin down (a
            // dynamically-built value leaving no literal). The opaque case can't be silently certified
            // (the §4 trust contract's forbidden direction), so it flags with a caveat. Checked against
            // the TRANSITIVE literal surface: this catches an un-sanctioned host/command/path reached
            // through a deep or cross-crate callee — the "billing only talks to Stripe" / "this layer
            // only runs git" / "config only reads /etc/app" boundary a local edit can't verify.
            for rule in &self.allow_rules {
                if let Some(scope) = &rule.scope {
                    if !scope_matches(&scope_name, scope) {
                        continue;
                    }
                }
                if !effs.contains(rule.effect) {
                    continue; // the effect isn't performed ⇒ the allowlist is trivially satisfied
                }
                // The transitive literal surface for this effect (hosts / commands / paths).
                let reached = match rule.effect {
                    "Net" => hostsacc.get(&f),
                    "Exec" => cmdsacc.get(&f),
                    "Fs" => pathsacc.get(&f),
                    "Db" => tablesacc.get(&f),
                    _ => None,
                };
                // Literals reached that no allowlist entry covers (effect-specific match, see
                // `literal_allowed`).
                let bad: Vec<&str> = reached
                    .map(|set| {
                        set.iter()
                            .filter(|v| !literal_allowed(rule.effect, v, &rule.literals))
                            .map(|v| v.as_str())
                            .collect()
                    })
                    .unwrap_or_default();
                // The effect is present but candor sees NO literal at all (a fully dynamic value) — it
                // can't certify the surface, so it flags. NOTE: a function that reaches a KNOWN allowed
                // value but ALSO makes an `Unknown` call is NOT flagged here: AS-EFF-008 certifies the
                // surface candor can SEE; the residual "an unresolvable call might reach anything" risk is
                // exactly what AS-EFF-003/006 cover. Folding `Unknown` in here would fire on essentially
                // every real effectful function, making the allowlist unusable.
                let opaque = reached.map(|set| set.is_empty()).unwrap_or(true);
                // The surface is INCOMPLETE for this effect (a host-establishing / cmd-naming call left the
                // endpoint invisible): can't certify even with visible literals, else a benign literal masks
                // the runtime endpoint (the masking evasion; sweep [3]/[7]). Matches candor-scan / the family.
                let surface_incomplete = incompleteacc.get(&f).is_some_and(|s| s.contains(rule.effect));
                if !bad.is_empty() || opaque || surface_incomplete {
                    let scope = rule.scope.as_deref().map(|s| format!(" (scope `{s}`)")).unwrap_or_default();
                    let noun = match rule.effect {
                        "Exec" => "a command",
                        "Fs" => "a path",
                        "Db" => "a table",
                        _ => "a host",
                    };
                    let detail = if !bad.is_empty() {
                        format!("reaches {{ {} }} outside the allowlist", bad.join(", "))
                    } else {
                        format!(
                            "performs {} to {noun} candor cannot determine (a dynamically-built value); \
                             the allowlist cannot be certified",
                            rule.effect
                        )
                    };
                    self.record_violation("AS-EFF-008", &name);
                    span_lint(
                        cx,
                        CANDOR,
                        span,
                        format!("[AS-EFF-008] `{name}` {detail}, forbidden by policy{scope}: `{}`", rule.raw),
                    );
                }
            }

            // AS-EFF-009 (CANDOR_POLICY layering): this function (in a `from` scope) transitively calls
            // into a forbidden layer — the dependency-direction architecture rule. Precomputed above by
            // reverse reachability over the call graph; emitted here where the function's span is known.
            if let Some(viols) = layer_viol.get(&f) {
                for (raw, tgt) in viols {
                    self.record_violation("AS-EFF-009", &name);
                    span_lint(
                        cx,
                        CANDOR,
                        span,
                        format!(
                            "[AS-EFF-009] `{name}` reaches into a forbidden layer (via `{tgt}`), \
                             violating policy: `{raw}`"
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
            let meta = ReportMeta {
                version: CANDOR_VERSION.into(),
                toolchain: CANDOR_TOOLCHAIN.into(),
                spec: candor_report::SPEC_VERSION.into(),
            };
            match candor_report::to_packaged_report_json(&meta, krate.as_str(), &json_entries) {
                Ok(body) => match candor_report::write_atomic(std::path::Path::new(&file), body.as_bytes()) {
                    Ok(()) => eprintln!("candor: wrote {} entries to {file}", json_entries.len()),
                    Err(e) => eprintln!("candor: failed to write {file:?} ({e})"),
                },
                Err(e) => eprintln!("candor: failed to serialize report ({e})"),
            }
            // Full local call graph sidecar (every function's direct callees by path, INCLUDING pure
            // ones the report omits). `cargo candor callers <fn>` reads it to answer "who (transitively)
            // calls X" for ANY function — the pre-edit blast-radius question an agent asks before adding
            // an effect ("I'm about to touch X; who depends on it?"). The main report only records
            // effect-relevant edges, so it can't answer that for a pure X. 3-segment name ⇒ report_files
            // ignores it. (Surfaced by the agent-use eval, eval/agentuse.)
            // SPEC §2.2: EVERY analyzed function is a key — a LEAF with no local callees gets an empty
            // list (iterating `eff`, which is seeded with every reportable item, not just `self.calls`,
            // which only holds fns that make a call). Omitting leaves made an uncalled leaf invisible
            // to `whatif`/`callers`, and an always-present key distinguishes "no callers" from "no such
            // function".
            let mut cg: std::collections::BTreeMap<String, Vec<String>> = std::collections::BTreeMap::new();
            for f in eff.keys() {
                let cs: Vec<String> = self
                    .calls
                    .get(f)
                    .map(|callees| {
                        let mut v: Vec<String> =
                            callees.iter().map(|c| cx.tcx.def_path_str(c.to_def_id())).collect();
                        v.sort();
                        v
                    })
                    .unwrap_or_default();
                cg.insert(cx.tcx.def_path_str(f.to_def_id()), cs);
            }
            let cgfile = format!("{prefix}.{krate}.{kinds}.callgraph.json");
            if let Ok(body) = serde_json::to_string(&cg) {
                let _ = candor_report::write_atomic(std::path::Path::new(&cgfile), body.as_bytes());
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
    // Test-only helpers of the shared policy parser (not called by the gate itself, which uses the
    // higher-level `literal_allowed`); these duplicate-coverage tests double as a smoke check that the
    // shared module is wired in on the nightly side too.
    use candor_classify::policy::{cmd_base, fs_path_covered};

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
        // Compiler diagnostic emission (a dylint lint's output) → Log; the Diag BUILDERS stay pure.
        assert_eq!(classify("rustc_lint", "rustc_lint::context::LintContext::emit_span_lint"), Some("Log"));
        assert_eq!(classify("rustc_errors", "rustc_errors::diagnostic::Diag::emit"), Some("Log"));
        assert_eq!(classify("rustc_errors", "rustc_errors::diagnostic::Diag::primary_message"), None);
        assert_eq!(classify("rustc_lint", "rustc_lint::Lint::default_level"), None);
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
        for c in candor_classify::DB_CRATES {
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
        // Probe tails are the SHARED `CALIBRATION_PROBE_TAILS` const (candor-classify) — not a local copy —
        // so this test and candor-classify's own copy can't drift (they did once: pnet/ignore/notify rules
        // use distinctive tails this list was missing, silently breaking the invariant here).
        for c in CALIBRATED_CRATES {
            assert!(
                candor_classify::CALIBRATION_PROBE_TAILS
                    .iter()
                    .any(|t| classify(c, &format!("{c}{t}")).is_some()),
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
    fn classify_libc_syscalls_by_category() {
        // The FFI-thin tier (nix etc.) bottoms out in raw libc — classify the unambiguous syscalls.
        assert_eq!(classify("libc", "libc::open"), Some("Fs"));
        assert_eq!(classify("libc", "libc::openat"), Some("Fs"));
        assert_eq!(classify("libc", "libc::unlinkat"), Some("Fs"));
        assert_eq!(classify("libc", "libc::statx"), Some("Fs"));
        assert_eq!(classify("libc", "libc::socket"), Some("Net"));
        assert_eq!(classify("libc", "libc::connect"), Some("Net"));
        assert_eq!(classify("libc", "libc::sendto"), Some("Net"));
        assert_eq!(classify("libc", "libc::execve"), Some("Exec"));
        assert_eq!(classify("libc", "libc::fork"), Some("Exec"));
        assert_eq!(classify("libc", "libc::posix_spawn"), Some("Exec"));
        assert_eq!(classify("libc", "libc::pipe2"), Some("Ipc"));
        assert_eq!(classify("libc", "libc::shmget"), Some("Ipc"));
        assert_eq!(classify("libc", "libc::socketpair"), Some("Ipc")); // AF_UNIX pair → Ipc, not Net
        assert_eq!(classify("libc", "libc::getenv"), Some("Env"));
        assert_eq!(classify("libc", "libc::clock_gettime"), Some("Clock"));
        assert_eq!(classify("libc", "libc::getrandom"), Some("Rand"));
        // Generic fd ops are DELIBERATELY unclassified — they run on any fd, so a fixed label would
        // mis-categorise; honest under-report beats wrong effect.
        assert_eq!(classify("libc", "libc::read"), None);
        assert_eq!(classify("libc", "libc::write"), None);
        assert_eq!(classify("libc", "libc::close"), None);
        assert_eq!(classify("libc", "libc::fcntl"), None);
        assert_eq!(classify("libc", "libc::mmap"), None);
        // Pure conversions/constants stay pure.
        assert_eq!(classify("libc", "libc::htons"), None);
        assert_eq!(classify("libc", "libc::O_RDONLY"), None);
    }

    #[test]
    fn classify_c_library_ffi_by_leaf() {
        // libsqlite3 (rusqlite calls `ffi::sqlite3_*`; lint resolves to `libsqlite3_sys`) — matched on
        // the distinctive leaf regardless of the binding crate's alias.
        assert_eq!(classify("ffi", "ffi::sqlite3_open"), Some("Db"));
        assert_eq!(classify("libsqlite3_sys", "libsqlite3_sys::sqlite3_step"), Some("Db"));
        assert_eq!(classify("ffi", "ffi::sqlite3_exec"), Some("Db"));
        assert_eq!(classify("ffi", "ffi::sqlite3_backup_step"), Some("Db"));
        // pure in-memory accessors stay pure (bind params / read columns do no I/O — that's at step)
        assert_eq!(classify("ffi", "ffi::sqlite3_bind_int"), None);
        assert_eq!(classify("ffi", "ffi::sqlite3_column_text"), None);
        assert_eq!(classify("ffi", "ffi::sqlite3_free"), None);
        // libgit2 (git2 calls `raw::git_*`) — remote ops Net, on-disk repo ops Fs.
        assert_eq!(classify("raw", "raw::git_remote_fetch"), Some("Net"));
        assert_eq!(classify("raw", "raw::git_clone"), Some("Net"));
        assert_eq!(classify("libgit2_sys", "libgit2_sys::git_remote_push"), Some("Net"));
        assert_eq!(classify("raw", "raw::git_repository_open"), Some("Fs"));
        assert_eq!(classify("raw", "raw::git_index_write"), Some("Fs"));
        assert_eq!(classify("raw", "raw::git_checkout_tree"), Some("Fs"));
        // pure libgit2 helpers (oid formatting, options init, type queries) stay pure
        assert_eq!(classify("raw", "raw::git_oid_fromstr"), None);
        assert_eq!(classify("raw", "raw::git_clone_init_options"), None);
        assert_eq!(classify("raw", "raw::git_remote_name"), None);
        // a non-libgit2 function that merely starts with `git_` is not falsely classified
        assert_eq!(classify("myapp", "myapp::git_dir_helper"), None);
        // libssl (openssl calls `ffi::SSL_*`): TLS-over-socket ops -> Net; crypto/setup stay pure.
        assert_eq!(classify("ffi", "ffi::SSL_connect"), Some("Net"));
        assert_eq!(classify("ffi", "ffi::SSL_read"), Some("Net"));
        assert_eq!(classify("ffi", "ffi::SSL_do_handshake"), Some("Net"));
        assert_eq!(classify("openssl_sys", "openssl_sys::SSL_shutdown"), Some("Net"));
        assert_eq!(classify("ffi", "ffi::SSL_CTX_new"), None); // context setup is pure
        assert_eq!(classify("ffi", "ffi::SSL_set_fd"), None); // just sets the socket fd
    }

    #[test]
    fn scope_matches_by_segment_not_substring() {
        assert!(scope_matches("app::domain::handle", "domain"));
        assert!(scope_matches("domain::handle", "domain"));
        assert!(scope_matches("app::domain", "domain"));
        assert!(scope_matches("crate::domain_logic", "domain")); // segment-prefixed fn name
        // the substring bug: `subdomain` (scope mid-word) must NOT match scope `domain`.
        assert!(!scope_matches("app::subdomain::handle", "domain"));
        assert!(!scope_matches("app::not_my_domain::f", "domain"));
    }

    #[test]
    fn scope_matches_multi_segment() {
        // A multi-segment scope (`a::b`) must match a contiguous run of segments, with the LAST
        // segment allowed to prefix-match a fn name and INTERMEDIATE segments matched exactly.
        assert!(scope_matches("crate::net::client::send", "net::client"));
        assert!(scope_matches("crate::net::client", "net::client"));
        assert!(scope_matches("crate::net::client_pool::get", "net::client")); // last seg prefix
        // wrong intermediate segment: `net::server` must not satisfy scope `net::client`.
        assert!(!scope_matches("crate::net::server::send", "net::client"));
        // intermediates are NOT prefix-matched: `network::client` ≠ scope `net::client`.
        assert!(!scope_matches("crate::network::client::send", "net::client"));
        // segments must be CONTIGUOUS: `net::x::client` is not `net::client`.
        assert!(!scope_matches("crate::net::x::client", "net::client"));
        // a scope longer than the path can never match.
        assert!(!scope_matches("net", "net::client"));
    }

    #[test]
    fn fs_path_covered_respects_boundaries() {
        // an allowed dir covers itself and anything beneath it…
        assert!(fs_path_covered("/etc/app", "/etc/app"));
        assert!(fs_path_covered("/etc/app", "/etc/app/cfg.toml"));
        assert!(fs_path_covered("/etc/app/", "/etc/app/cfg")); // trailing slash on allow is fine
        // …but NOT a sibling that merely shares a textual prefix (the old `starts_with` bug).
        assert!(!fs_path_covered("/etc/app", "/etc/apppwned"));
        assert!(!fs_path_covered("/etc/app", "/etc/application/x"));
        // a deeper dir does not cover its parent.
        assert!(!fs_path_covered("/etc/app/cfg", "/etc/app"));
        // `..` in the reached path can climb out → never covered.
        assert!(!fs_path_covered("/etc/app", "/etc/app/../passwd"));
        // a root/empty allow covers everything.
        assert!(fs_path_covered("/", "/etc/app/x"));
        // absolute vs relative must NOT be conflated (norm drops the leading empty component): a
        // relative allow does not cover an absolute reached path, nor the reverse — they're different
        // filesystem locations (a relative path resolves against CWD).
        assert!(!fs_path_covered("etc/app", "/etc/app/cfg"));
        assert!(!fs_path_covered("/etc/app", "etc/app/cfg"));
        // …but matched rootedness still covers.
        assert!(fs_path_covered("etc/app", "etc/app/cfg"));
        // wired through literal_allowed for Fs.
        let allow: BTreeSet<String> = ["/etc/app".to_string()].into_iter().collect();
        assert!(literal_allowed("Fs", "/etc/app/cfg", &allow));
        assert!(!literal_allowed("Fs", "/etc/apppwned", &allow));
    }

    #[test]
    fn effectful_std_traits_are_io_only() {
        // Generic dispatch over these hides real I/O → Unknown (not assumed pure).
        assert!(is_effectful_std_trait("std", "Write"));
        assert!(is_effectful_std_trait("std", "Read"));
        assert!(is_effectful_std_trait("std", "BufRead"));
        assert!(is_effectful_std_trait("std", "Seek"));
        // core::fmt::Write is PURE formatting — same item name as std::io::Write, disambiguated by crate.
        assert!(!is_effectful_std_trait("core", "Write"));
        assert!(is_pure_fmt_write("core", "Write"));
        assert!(!is_pure_fmt_write("std", "Write")); // std::io::Write is NOT pure
        // Conventionally-pure traits stay pure (not effectful).
        assert!(!is_effectful_std_trait("std", "Iterator"));
        assert!(!is_effectful_std_trait("serde", "Serialize"));
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
    fn rules_parsing_and_extra_rules() {
        let rules = parse_rules(
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
        // 32 BYTES but a multi-byte char straddles index 16 — must NOT panic (was an ICE: `&s[..16]`
        // on a non-char-boundary). "é" is 2 bytes: 15 ASCII + é + 15 ASCII = 32 bytes, boundary at 16
        // falls mid-char.
        let straddle = format!("{}é{}", "a".repeat(15), "a".repeat(15));
        assert_eq!(straddle.len(), 32);
        assert_eq!(parse_dph(&straddle), None);
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
        let p = parse_policy(
            "# the domain layer must stay pure of I/O\n\
             deny Net Db  domain\n\
             deny Exec\n\
             pure  parse\n\
             nonsense line\n\
             deny notaneffect\n",
        );
        let rules = &p.rules;
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
        assert_eq!(parse_policy("deny Unknown core").rules[0].effects, ["Unknown"].into_iter().collect());
        assert!(parse_policy("deny\ndeny   \n").rules.is_empty());
    }

    #[test]
    fn allowlist_parses() {
        let p = parse_policy(
            "allow Net in billing  api.stripe.com  hooks.stripe.com\n\
             allow Exec in ci  git\n\
             allow Fs in config  /etc/app\n\
             allow Net  github.com\n\
             allow Clock  whatever\n\
             allow Net in nohosts\n\
             allow\n",
        );
        // The four well-formed Net/Exec/Fs rules survive; `allow Clock` (no literal surface), the
        // scoped Net rule that names no hosts, and the bare `allow` are dropped. (`allow Db <table>`
        // is part of the grammar — see the candor-classify policy tests.)
        assert_eq!(p.allow_rules.len(), 4);
        // `allow Net in billing …`
        assert_eq!((p.allow_rules[0].effect, p.allow_rules[0].scope.as_deref()), ("Net", Some("billing")));
        assert_eq!(
            p.allow_rules[0].literals,
            ["api.stripe.com", "hooks.stripe.com"].iter().map(|s| s.to_string()).collect()
        );
        // `allow Exec in ci git`
        assert_eq!((p.allow_rules[1].effect, p.allow_rules[1].scope.as_deref()), ("Exec", Some("ci")));
        assert!(p.allow_rules[1].literals.contains("git"));
        // `allow Fs in config /etc/app`
        assert_eq!((p.allow_rules[2].effect, p.allow_rules[2].scope.as_deref()), ("Fs", Some("config")));
        // `allow Net github.com` — no `in`, so whole-crate scope.
        assert_eq!((p.allow_rules[3].effect, p.allow_rules[3].scope.is_none()), ("Net", true));

        // effect-specific matching: host by name, command by basename, path by prefix.
        let set = |xs: &[&str]| xs.iter().map(|s| s.to_string()).collect::<BTreeSet<_>>();
        assert!(literal_allowed("Net", "api.stripe.com:443", &set(&["api.stripe.com"])));
        assert!(literal_allowed("Exec", "/usr/bin/git", &set(&["git"])));
        assert!(!literal_allowed("Exec", "/usr/bin/curl", &set(&["git"])));
        assert!(literal_allowed("Fs", "/etc/app/conf.toml", &set(&["/etc/app"])));
        assert!(!literal_allowed("Fs", "/etc/shadow", &set(&["/etc/app"])));
        assert_eq!(cmd_base("/usr/bin/git"), "git");
    }

    #[test]
    fn layering_rule_parses() {
        let p = parse_policy(
            "forbid domain -> infra\n\
             forbid  app::web  ->  app::db \n\
             forbid domain infra\n\
             forbid domain ->\n\
             forbid\n",
        );
        // Only the two well-formed `forbid <A> -> <B>` rules survive; the missing-arrow, missing-target,
        // and bare `forbid` lines are dropped.
        assert_eq!(p.layer_rules.len(), 2);
        assert_eq!((p.layer_rules[0].from.as_str(), p.layer_rules[0].to.as_str()), ("domain", "infra"));
        assert_eq!((p.layer_rules[1].from.as_str(), p.layer_rules[1].to.as_str()), ("app::web", "app::db"));
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
        // dep::f also carries literal host/command/path detail, which must propagate into the cross maps.
        w(
            "r.dep.Rlib.json",
            format!(
                r#"[{{"fn":"dep::f","inferred":["Net","Fs","Exec"],"hosts":["api.stripe.com"],"cmds":["git"],"paths":["/etc/app"],"hash":"{h_lib}"}},
                    {{"fn":"dep::pure","inferred":[],"hash":"{h_lib}"}},
                    {{"fn":"dep::old","inferred":["Db"]}}]"#
            ),
        );
        // Our OWN report (me = mybin/Executable) — MUST be skipped (same crate name, our type).
        w("r.mybin.Executable.json", format!(r#"[{{"fn":"own","inferred":["Exec"],"hash":"{h_own}"}}]"#));
        // Sidecars (one middle segment, or a dotted token) — MUST be skipped.
        w("r.calibrated.json", r#"{"crates":[]}"#.to_string());
        w("r.encountered-dep-Rlib.json", r#"["serde"]"#.to_string());

        let cd = load_cross_reports(prefix, "mybin", "Executable", false);
        let m = &cd.effects;

        // dep::f loaded under its parsed-hash key with all three effects.
        let mut got = m.get(&parse_dph(h_lib).unwrap()).cloned().unwrap_or_default();
        got.sort();
        assert_eq!(got, vec!["Exec", "Fs", "Net"]);
        // dep::pure (empty effects) and dep::old (no hash) are dropped; own report is skipped.
        assert!(m.get(&parse_dph(h_own).unwrap()).is_none(), "own report must be skipped");
        assert_eq!(m.len(), 1, "only the one dependency entry with hash + effects should load");
        // dep::f's host/command/path detail crossed the boundary into the cross maps (the scale path
        // for the AS-EFF-008 allowlists).
        let k = parse_dph(h_lib).unwrap();
        assert_eq!(cd.hosts.get(&k).cloned().unwrap_or_default(), ["api.stripe.com".to_string()].into_iter().collect());
        assert_eq!(cd.cmds.get(&k).cloned().unwrap_or_default(), ["git".to_string()].into_iter().collect());
        assert_eq!(cd.paths.get(&k).cloned().unwrap_or_default(), ["/etc/app".to_string()].into_iter().collect());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn load_cross_reports_skips_corrupt_sibling_but_keeps_the_rest() {
        // A corrupt/partial sibling report (a valid `<crate>.<kind>.json` name, but unparseable JSON)
        // must be skipped gracefully — never panic, and never abort loading the OTHER siblings. (The
        // skip is paired with a stderr warning so the cross-crate degradation is loud, not silent.)
        let dir = std::env::temp_dir().join("candor_cross_corrupt_unit");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let prefix = dir.join("r");
        let prefix = prefix.to_str().unwrap();
        let h_good = "0123456789abcdef0123456789abcdef";
        let w = |name: &str, body: &str| std::fs::write(dir.join(name), body).unwrap();

        // A good sibling and a corrupt one (truncated mid-write); both pass `report_files`' name filter.
        w("r.good.Rlib.json", &format!(r#"[{{"fn":"good::f","inferred":["Net"],"hash":"{h_good}"}}]"#));
        w("r.bad.Rlib.json", r#"[{"fn":"bad::f","inferred":["Fs"#);

        let cd = load_cross_reports(prefix, "mybin", "Executable", false);
        // The good sibling still loaded despite the corrupt one being present.
        assert_eq!(
            cd.effects.get(&parse_dph(h_good).unwrap()).cloned().unwrap_or_default(),
            vec!["Net"],
            "a corrupt sibling must not block loading the valid ones"
        );
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
        let m = load_cross_reports(prefix, "mybin", "Executable", false).effects;
        assert_eq!(m.get(&parse_dph(h).unwrap()).cloned().unwrap_or_default(), vec![UNKNOWN]);

        // The SAME engine version → trusted as-is.
        w(format!(r#"{{"candor":{{"version":"{CANDOR_VERSION}"}},"functions":[{{"fn":"dep::f","inferred":["Net","Fs"],"hash":"{h}"}}]}}"#));
        let m = load_cross_reports(prefix, "mybin", "Executable", false).effects;
        let mut got = m.get(&parse_dph(h).unwrap()).cloned().unwrap_or_default();
        got.sort();
        assert_eq!(got, vec!["Fs", "Net"]);

        // Guard mode (trust_siblings=true): an OTHER-version report is the baseline's own snapshot, so
        // it is trusted as-is — NOT downgraded. (Otherwise the guard fires AS-EFF-005 every time the
        // engine moves ahead of the baseline it is comparing against.)
        w(format!(r#"{{"candor":{{"version":"OTHER"}},"functions":[{{"fn":"dep::f","inferred":["Net","Fs"],"hash":"{h}"}}]}}"#));
        let m = load_cross_reports(prefix, "mybin", "Executable", true).effects;
        let mut got = m.get(&parse_dph(h).unwrap()).cloned().unwrap_or_default();
        got.sort();
        assert_eq!(got, vec!["Fs", "Net"]);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
