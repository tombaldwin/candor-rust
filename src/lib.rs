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
    /// External (non-std, non-local) crates we actually saw resolved calls into.
    /// Ground truth for the coverage blind-spot check — emitted beside the report.
    encountered: BTreeSet<String>,
}

/// Effects that represent *ambient authority* — a global resource reachable just by
/// naming it (vs. a capability you must be handed). These are what `CANDOR_NO_AMBIENT`
/// and cap-std care about. `Log` is intentionally excluded (not an authority).
const AMBIENT: [&str; 9] =
    ["Net", "Fs", "Exec", "Env", "Clock", "Clipboard", "Rand", "Db", "Ipc"];

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
        Self { direct: HashMap::new(), calls: HashMap::new(), extra, paranoid, encountered: BTreeSet::new() }
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

/// One entry of the JSON report (output). Serialized with serde so escaping is correct
/// for any path/loc — the hand-rolled escaper missed control characters.
#[derive(serde::Serialize)]
struct ReportEntry {
    #[serde(rename = "fn")]
    func: String,
    loc: String,
    inferred: Vec<String>,
    direct: Vec<String>,
    declared: Vec<String>,
    undeclared: Vec<String>,
    overdeclared: Vec<String>,
    unresolved: bool,
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

/// The exact third-party crates `classify` has effect rules for, and the crate-name
/// PREFIXES it recognizes. This is the single source of truth for "what candor knows":
/// it is emitted beside the JSON report (`<prefix>.calibrated.json`) so the Claude Code
/// receipt's coverage check reads candor's real coverage instead of a hand-copied list.
/// Keep in lockstep with `classify` below — the `calibrated_set_covers_classifier` test
/// enforces that every named crate the classifier matches appears here.
const CALIBRATED_CRATES: [&str; 25] = [
    // network (aws_config resolves credentials over the network on `.load()`;
    // git2 remote ops — fetch/push/connect — contact the network)
    "reqwest", "isahc", "ureq", "aws_config", "git2",
    // database (see DB_CRATES in classify)
    "sqlx", "rusqlite", "postgres", "tokio_postgres", "diesel", "redis", "mongodb",
    "mysql", "mysql_async", "sea_orm", "deadpool_postgres",
    // filesystem / entropy / subprocess / clock / log / clipboard
    "memmap2", "rand", "getrandom", "fastrand", "portable_pty", "chrono", "tracing", "log", "arboard",
];
const CALIBRATED_PREFIXES: [&str; 3] = ["aws_sdk_", "aws_smithy", "cap_"];

/// Database client crates whose execution verbs are I/O (see the DB branch in `classify`).
/// Module-level so `db_crates_are_calibrated` can enforce `DB_CRATES ⊆ CALIBRATED_CRATES`.
const DB_CRATES: [&str; 11] = [
    "sqlx", "rusqlite", "postgres", "tokio_postgres", "diesel", "redis", "mongodb",
    "mysql", "mysql_async", "sea_orm", "deadpool_postgres",
];

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
    // CANDOR_NO_AMBIENT and audits don't conflate it with internet access.
    if path.starts_with("tokio::net::Unix") || path.starts_with("std::os::unix::net") {
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
    if path.starts_with("std::fs::") || path.starts_with("tokio::fs::") || crate_name == "memmap2" {
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
    // Wall-clock reads. Match the `now` accessor precisely (ends_with), not any path
    // containing the substring "now".
    if (crate_name == "chrono" || path.starts_with("std::time::")) && path.ends_with("::now") {
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

const EFFECTS: [&str; 10] =
    ["Net", "Fs", "Exec", "Env", "Clock", "Log", "Clipboard", "Rand", "Db", "Ipc"];

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

impl<'tcx> LateLintPass<'tcx> for Candor {
    fn check_expr(&mut self, cx: &LateContext<'tcx>, expr: &'tcx Expr<'tcx>) {
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

        // Class Hierarchy Analysis: if this is a (local) trait method — whether reached
        // by `dyn` dispatch or a generic bound — add edges to every impl so their effects
        // propagate. This is what lets candor see through trait objects soundly.
        let trait_did = cx.tcx.trait_of_assoc(def_id);
        let mut cha_resolved = false;
        if trait_did.is_some() {
            for target in cha_targets(cx.tcx, def_id) {
                cha_resolved = true;
                add_edge(self, target);
            }
        }

        // Honest `Unknown` only when dispatch is genuinely unresolvable here:
        //  - a `dyn` call over a NON-local trait (we can't see its impl bodies), or
        //  - (paranoid) any trait dispatch CHA couldn't pin to local impls.
        // ...but NOT for conventionally-pure std traits (Display/Error/…), where the
        // overwhelmingly-pure dispatch would otherwise flood reports with false Unknowns.
        if let Some(td) = trait_did {
            let pure = is_pure_std_trait(cx.tcx.crate_name(td.krate).as_str(), cx.tcx.item_name(td).as_str());
            if !cha_resolved && !pure && (dynamic || self.paranoid) {
                self.direct.entry(caller).or_default().insert(UNKNOWN);
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
        // Seed every reportable item so over-declaration (declared-but-unused) is covered.
        for owner in cx.tcx.hir_body_owners() {
            if is_reportable_item(cx.tcx.def_kind(owner.to_def_id())) {
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
        let any_enforce =
            strict_var.is_some() || no_ambient_var.is_some() || baseline.is_some();

        // Stable ordering for reproducible output.
        let mut items: Vec<LocalDefId> = eff.keys().copied().collect();
        items.sort_by_cached_key(|f| cx.tcx.def_path_str(f.to_def_id()));

        let mut json_entries: Vec<ReportEntry> = Vec::new();
        let owned = |v: &[&str]| v.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        let owned_set = |s: &BTreeSet<&str>| s.iter().map(|x| x.to_string()).collect::<Vec<_>>();

        for f in items {
            let span = cx.tcx.def_span(f);
            // Skip macro-generated items (e.g. tracing's `__CALLSITE` statics): they're
            // not code the developer wrote or can edit, and would flood the report.
            if span.from_expansion() {
                continue;
            }
            let effs = &eff[&f];
            let name = cx.tcx.def_path_str(f.to_def_id());
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
                json_entries.push(ReportEntry {
                    func: name,
                    loc,
                    inferred: owned_set(effs),
                    direct: owned_set(&direct),
                    declared: owned_set(&declared),
                    undeclared: owned(&undeclared),
                    overdeclared: owned(&unused),
                    unresolved: has_unknown,
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
                // AS-EFF-001: performs an effect it does not declare. The real entry
                // point (per tcx.entry_fn) is exempt — it legitimately holds the bundle.
                if !undeclared.is_empty() && Some(f.to_def_id()) != entry_fn {
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
            match serde_json::to_string_pretty(&json_entries) {
                Ok(body) => match std::fs::write(&file, body) {
                    Ok(()) => eprintln!("candor: wrote {} entries to {file}", json_entries.len()),
                    Err(e) => eprintln!("candor: failed to write {file:?} ({e})"),
                },
                Err(e) => eprintln!("candor: failed to serialize report ({e})"),
            }
            // Emit candor's calibrated crate set alongside the report, so downstream
            // coverage checks read it from the engine rather than a duplicated copy.
            let calib = serde_json::json!({
                "crates": CALIBRATED_CRATES,
                "prefixes": CALIBRATED_PREFIXES,
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
                format!("{c}::Utc::now"),
                format!("{c}::X::load"),
                format!("{c}::__private_api::log"),
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
