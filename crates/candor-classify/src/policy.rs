//! The canonical CANDOR_POLICY DSL parser (candor-spec SPEC §6.2).
//!
//! This is the **single** Rust implementation of the policy grammar — shared by the nightly dylint
//! gate (`src/lib.rs`, AS-EFF-006/008/009) and the stable `candor-query` (`whatif`, and the
//! `parsepolicy` dump the cross-impl conformance suite diffs against the JVM engine). Keeping one
//! parser here is what makes "the gate means the same thing in every language" a fact rather than a
//! hope: the Rust gate, the Rust pre-edit tool, and the cross-impl differential all read THIS code.
//!
//! Pure, stable Rust (string parsing only — no rustc types), so it lives beside the classifier.

use crate::cap_from_name;
use std::collections::{BTreeMap, BTreeSet};

/// The honesty marker (SPEC §4). Denyable so `deny Unknown <scope>` forbids the *unverifiable* case.
pub const UNKNOWN: &str = "Unknown";

/// The NORMATIVE projection of a raw `unknown_why` reason onto a fixed, cross-engine reason CLASS
/// (candor-spec REASON-SCOPED-UNKNOWN-DESIGN.md §1). Reason-scoped policies (`deny E Unknown[class]`)
/// quantify over these classes, so the mapping MUST be identical in every engine — this mirrors the
/// java reference `ReasonClass` (its `classify(String)` path, since rust emits raw string reasons). The
/// class set is CLOSED (six members); a raw reason matching no pinned prefix maps to `Unresolved` —
/// conservative: it stays in scope of any `Unknown[*]` / `Unknown[dynamic]` policy, never silently
/// tolerated.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ReasonClass {
    /// reflection / metaprogramming
    Reflect,
    /// unresolved virtual/dynamic dispatch, same-name ambiguity, invokedynamic
    Dispatch,
    /// callback / closure / function-value / async-continuation indirection
    Indirect,
    /// FFI / native boundary
    Native,
    /// generic unresolvable call/import, AND the catch-all for any unrecognized raw reason
    Unresolved,
    /// analysis not wired up (fixable, not a real dynamic hole): missing-config / no-tsconfig
    Setup,
}

impl ReasonClass {
    /// The lowercase policy-facing token (`deny E Unknown[<token>]`).
    pub fn token(self) -> &'static str {
        match self {
            ReasonClass::Reflect => "reflect",
            ReasonClass::Dispatch => "dispatch",
            ReasonClass::Indirect => "indirect",
            ReasonClass::Native => "native",
            ReasonClass::Unresolved => "unresolved",
            ReasonClass::Setup => "setup",
        }
    }

    /// Parse a policy-facing token back to a class; `None` if it names no class.
    pub fn from_token(t: &str) -> Option<ReasonClass> {
        Some(match t {
            "reflect" => ReasonClass::Reflect,
            "dispatch" => ReasonClass::Dispatch,
            "indirect" => ReasonClass::Indirect,
            "native" => ReasonClass::Native,
            "unresolved" => ReasonClass::Unresolved,
            "setup" => ReasonClass::Setup,
            _ => return None,
        })
    }

    /// Map a raw `unknown_why` reason to its normative class — prefix-based (raw reasons carry a
    /// `kind:detail` shape, e.g. `dispatch:foo::Bar`), unrecognized → `Unresolved`. Byte-identical
    /// intent to the java `ReasonClass.classify(String)`.
    ///
    /// ⟨0.24⟩ THIS IS THE ONLY PLACE THIS ENGINE HOLDS SPEC §4's KIND VOCABULARY. Every other reference
    /// to a kind is either a raw string being emitted or the `dispatch:` prefix test in candor-query's
    /// dispatch frontier; there is no typed kind enum here. §4's "AN ENGINE HOLDS THIS VOCABULARY TWICE,
    /// AND THE HALVES DRIFT" paragraph records the JVM engine classifying `ambiguous` correctly HERE
    /// while its typed `Kind` enum lacked the kind entirely — one token, two answers, inside one engine,
    /// concealed precisely because this half was right. Holding it once is why that is not reachable in
    /// this engine; a future typed representation must be added at the same commit as its control
    /// (`off_vocabulary_kinds_round_trip_and_classify_through_the_catch_all`).
    ///
    /// The five §4 kinds are `reflect`/`native`/`dispatch`/`callback`/`ambiguous`. `ambiguous` maps to
    /// `dispatch` and rust is its only PRODUCER; `indy`/`task-handoff` are candor-java's migration kinds
    /// and `dep:`/`dep-stale:` are swift's registered per-dependency-ENTRY kinds, reaching `Unresolved`
    /// through the catch-all, which is the class §6.2 prescribes for them.
    pub fn classify(why: &str) -> ReasonClass {
        let w = why.trim().to_ascii_lowercase();
        if w.starts_with("reflect") || w == "dynamicmemberlookup" {
            ReasonClass::Reflect
        } else if w.starts_with("native") {
            ReasonClass::Native
        } else if w.starts_with("callback") || w.starts_with("closure") || w.starts_with("task-handoff") {
            ReasonClass::Indirect
        } else if w.starts_with("dispatch") || w.starts_with("indy") || w.starts_with("ambiguous") {
            ReasonClass::Dispatch
        } else if w.starts_with("missing-config") || w.starts_with("no-tsconfig") || w.starts_with("no-node_modules") {
            ReasonClass::Setup
        } else {
            ReasonClass::Unresolved
        }
    }

    /// The `dynamic` alias — every GENUINE blind-spot class (excludes `setup`), incl. `unresolved` (the
    /// catch-all) so `Unknown[dynamic]` never under-gates. The design's recommended usable strict gate.
    pub fn dynamic_set() -> BTreeSet<ReasonClass> {
        [
            ReasonClass::Reflect,
            ReasonClass::Dispatch,
            ReasonClass::Indirect,
            ReasonClass::Native,
            ReasonClass::Unresolved,
        ]
        .into_iter()
        .collect()
    }
}

/// SPEC §6.2 ⟨0.24⟩ — does a function's TRANSITIVE reason-class set intersect `want` (class tokens)?
///
/// THE `None`/EMPTY ARM IS THE FAIL-CLOSED NET, and it is why this lives here rather than being
/// open-coded twice. §6.2: "a function whose `Unknown` carries no recorded reason CONTRIBUTES
/// `unresolved` to its class set — so a narrowed filter never *silently* tolerates a hole it failed to
/// classify." Read the other way round, `!contains ⇒ exclude` over an empty set drops the entry from
/// EVERY filter including one naming its own class: a silent under-report wearing a filter.
///
/// `classes` is the ACCUMULATED (post-`propagate_str`) set, never the direct `unknownWhy`: a reason
/// names a site in the function's OWN body (§4), so a function whose `Unknown` is purely inherited
/// carries none, and matching against the direct field answers a different question. SHARED by the
/// `deny E Unknown[class]` gate (candor-scan) and `unverified --class` (candor-query) so a gate and the
/// disclosure explaining it cannot disagree about which holes a class names.
///
/// The empty arm is a NET, not a route: it keys on the ABSENCE of a class set, so any other reason on
/// the same function hides whatever it was covering. The reasonless case that CAN co-occur with a
/// reason — a direct `Unknown` the unit did not name — must therefore contribute `unresolved` into the
/// DIRECT map before propagation (candor-scan's `unknown_via_dep`, candor-query's report-side signature),
/// not arrive here by absence.
pub fn reason_class_matches(classes: Option<&BTreeSet<String>>, want: &BTreeSet<&str>) -> bool {
    match classes {
        None => want.contains(ReasonClass::Unresolved.token()),
        Some(cs) if cs.is_empty() => want.contains(ReasonClass::Unresolved.token()),
        Some(cs) => cs.iter().any(|t| want.contains(t.as_str())),
    }
}

/// Parse `unknown-alias <name> = <class,…>` lines from `.candor/config` text (⟨0.19⟩, SPEC §6.2) into a
/// name→classes map. A name that shadows a built-in (`*`/`dynamic`/a class token) is warned-and-skipped (a
/// config alias may not redefine a built-in), as is a definition naming no valid class. Byte-shape with the
/// java reference `Config.addAlias`.
pub fn parse_unknown_aliases(config_text: &str) -> std::collections::BTreeMap<String, BTreeSet<ReasonClass>> {
    let mut out = std::collections::BTreeMap::new();
    for raw in config_text.lines() {
        let line = raw.split('#').next().unwrap_or("").trim();
        if line.is_empty() {
            continue;
        }
        let mut it = line.splitn(2, char::is_whitespace);
        // Case-INSENSITIVE key match, like the config loaders in java/ts/swift (which lowercase the key) —
        // a case-sensitive match here made `Unknown-Alias …` define an alias everywhere BUT rust (a four-way
        // parse divergence; caught in review). The rest of the line (name + classes) stays case-sensitive.
        if !it.next().is_some_and(|k| k.eq_ignore_ascii_case("unknown-alias")) {
            continue;
        }
        let val = it.next().unwrap_or("").trim();
        let Some((name, classes)) = val.split_once('=') else {
            eprintln!("candor: ignoring `unknown-alias` (want `unknown-alias <name> = <class,…>`): {val}");
            continue;
        };
        let name = name.trim();
        if name.is_empty() || name == "*" || name == "dynamic" || ReasonClass::from_token(name).is_some() {
            eprintln!("candor: ignoring `unknown-alias` with reserved/empty name `{name}` (may not shadow `*`/`dynamic`/a class token)");
            continue;
        }
        let mut set = BTreeSet::new();
        let mut bad: Vec<&str> = Vec::new();
        for cn in classes.split(',') {
            let cn = cn.trim();
            if cn.is_empty() {
                continue;
            }
            if cn == "dynamic" {
                set.extend(ReasonClass::dynamic_set());
            } else if let Some(rc) = ReasonClass::from_token(cn) {
                set.insert(rc);
            } else {
                bad.push(cn);
            }
        }
        // ⟨0.24⟩ **AN UNRECOGNISED TOKEN REFUSES THE WHOLE DEFINITION** — SPEC §6.2 `be0b9a9`: the rule
        // binds every policy value list, and *"the second is the sharper one: the typo is in the
        // vocabulary the policy is written against rather than in the policy itself, and it fails open
        // identically."*
        //
        // MEASURED: `unknown-alias corp = dispatch,nativ` → the DEFINITION silently became `{dispatch}`,
        // so `deny Unknown[corp]` exited 0 over a native-caused hole that `= dispatch,native` catches.
        // The alias resolved, `used_aliases` recorded it, the verdict named the config — every disclosure
        // fired correctly ABOUT A DEFINITION THAT WAS NOT THE ONE ON DISK.
        //
        // REFUSING THE DEFINITION rather than minting a new error channel is what makes this fit: an
        // alias that does not exist is one the policy's own `Unknown[<name>]` cannot resolve, so it lands
        // on the `errors` path already there and the gate routes refuse with exit 2 — naming the token
        // AND the accepted set, as §6.2 requires. It also keeps the blast radius honest: a config
        // defining ten aliases, one of them typo'd and NONE of them mentioned by this policy, changed
        // nothing, and turning that into a red gate would be the mirror over-reach. The refusal is
        // triggered by USE, exactly like `used_aliases` (recorded at the point of use, for the same
        // reason).
        if !bad.is_empty() {
            eprintln!(
                "candor: REFUSING `unknown-alias {name}` — it names unrecognised reason-class(es) `{}` \
                 (accepted: reflect, dispatch, indirect, native, unresolved, setup, plus `dynamic`). The \
                 definition is refused WHOLE rather than narrowed to the tokens that parsed: keeping the \
                 rest would silently redefine `{name}` as something narrower than the config says, and a \
                 policy using it would gate less than it reads. `{name}` is now undefined, so a policy \
                 naming it is a policy error (exit 2).",
                bad.join("`, `")
            );
        } else if set.is_empty() {
            eprintln!("candor: ignoring `unknown-alias {name}` — no valid reason-class");
        } else {
            out.insert(name.to_string(), set);
        }
    }
    out
}

/// ⟨0.20⟩ Parse `net-partner <host>` lines from `.candor/config` (NET-DESTINATION-CLASS-DESIGN.md): the
/// per-project set of business-partner hosts the `Net` destination-class classifier treats as
/// `known-partner`. Multi-value (repeatable key); the value is host-normalized (`:port` stripped,
/// lowercased) like `MODEL_HOSTS`. Case-insensitive key match, mirroring `parse_unknown_aliases` + the
/// java/ts/swift config loaders. A partner is per-project — never a universal list.
pub fn parse_net_partners(config_text: &str) -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    for raw in config_text.lines() {
        let line = raw.split('#').next().unwrap_or("").trim();
        if line.is_empty() {
            continue;
        }
        let mut it = line.splitn(2, char::is_whitespace);
        if !it.next().is_some_and(|k| k.eq_ignore_ascii_case("net-partner")) {
            continue;
        }
        let val = it.next().unwrap_or("").trim();
        if !val.is_empty() {
            out.insert(host_part(val).to_ascii_lowercase());
        }
    }
    out
}

/// Discover `.candor/config` text for a policy/scan anchored at `start`: `$CANDOR_CONFIG` if set + readable,
/// else the nearest `.candor/config` walking UP from `start`, else `None`. Read-only + lenient (no
/// process-exit — the caller decides fail-closed); used to resolve `unknown-alias` for the §6.2 gate +
/// `parsepolicy` so both reflect the same checked-in config.
pub fn discover_config_text(start: &std::path::Path) -> Option<String> {
    discover_config(start).map(|(_, t)| t)
}

/// THE RUNNING BINARY'S REFUSAL WRITER, registered by whichever entry armed a sink.
///
/// (These paragraphs once sat ABOVE this static while documenting `discover_config`, which is below it —
/// so rustdoc rendered that function's path/canonicalization rationale as a description of this hook.
/// They now live on the item they describe: a doc block belongs to the next item, and separating it with
/// a blank line is not a fix — clippy's `empty_line_after_doc_comments` rejects that, and CI said so.)
///
/// [`discover_config`] is SHARED and sits BELOW every gate sink, so its `exit(2)` for an unreadable
/// config could not write the refusal document `--gate-json` was promised. Measured: on the `gate`
/// verb route an unreadable `CANDOR_CONFIG` exited 2 with an EMPTY stream in candor-scan and
/// candor-ts while candor-java and candor-swift wrote the refusal. The FILE sink was covered — the
/// armed placeholder survives an exit that writes nothing — so only the stream, which cannot be
/// pre-armed, was exposed.
///
/// NOT registered at startup: `set_refusal_sink` is called by the gate entries that arm a sink, so a
/// process that never arms one still exits 2 plainly. The comment inside `discover_config` already
/// records this cause being fixed once, for the EXIT
/// CODE: the scan route refused and the query route did not. That fix stopped at the exit code and
/// left the machine channel, which is exactly the split conformance PART 35 and PART 36 exist to
/// keep apart. A hook rather than a Result because every caller of this function wants the same
/// answer — refuse through whatever sink this process armed — and threading a new error type through
/// all of them would create the second copy of a rule that this project keeps getting bitten by.
static REFUSAL_SINK: std::sync::OnceLock<fn(&str) -> !> = std::sync::OnceLock::new();

/// Register the process's refusal writer. Idempotent; the first registration wins.
pub fn set_refusal_sink(f: fn(&str) -> !) {
    let _ = REFUSAL_SINK.set(f);
}

/// Exit 2 through the registered sink when there is one, and plainly when there is not — a library
/// consumer that never armed a sink must still get the documented exit code.
fn refuse_unevaluable(reason: &str) -> ! {
    if let Some(f) = REFUSAL_SINK.get() {
        f(reason)
    }
    std::process::exit(2)
}

/// ⟨0.24⟩ As [`discover_config_text`], but ALSO the PATH the text came from, canonicalized.
///
/// **WHY THE PATH IS NOW LOAD-BEARING** (SPEC §3.1): a `.candor/config` supplying `unknown-alias`
/// vocabulary can move a verdict 0→1, and discovery walks PARENT DIRECTORIES, so a file anywhere above
/// the policy participates — ambient, and until now invisible in the output. *"A verdict changed by a
/// file the operator cannot see named in the output is the ambient-input failure this whole format
/// exists to refuse; the remedy is the same one used everywhere else here — not to forbid the input, but
/// to make it impossible for it to act unnamed."* So the gate document NAMES it, and the path has to
/// travel out of discovery for that to be possible.
///
/// CANONICALIZED because the two routes reach the same file from different working directories, and
/// §3.1's byte-equality MUST is about the DOCUMENT: a relative path would differ between them for no
/// reason other than where each was invoked.
pub fn discover_config(start: &std::path::Path) -> Option<(std::path::PathBuf, String)> {
    let canon = |p: &std::path::Path| std::fs::canonicalize(p).unwrap_or_else(|_| p.to_path_buf());
    // CONFIGURED-BUT-UNUSABLE FAILS LOUD, ON THIS ROUTE TOO. `.ok()` turned an unreadable config into
    // "no config", so the run continued WITHOUT whatever it declared — a policy, a baseline, an engine
    // pin, an `unknown-alias` vocabulary. The SCAN route already refuses; this QUERY route did not, so
    // `gate --report R --policy P` with a broken CANDOR_CONFIG exited 1 here and 2 in java and ts on the
    // same input. §3.4's posture does not vary by verb.
    if let Ok(p) = std::env::var("CANDOR_CONFIG") {
        let p = std::path::PathBuf::from(p);
        match std::fs::read_to_string(&p) {
            Ok(t) => return Some((canon(&p), t)),
            Err(e) => {
                eprintln!("candor: CANDOR_CONFIG set but {} could not be read ({e}) — failing (exit 2,", p.display());
                eprintln!("        unevaluable). A config that cannot be read is a guard the operator believes is on.");
                refuse_unevaluable(&format!("CANDOR_CONFIG set but {} could not be read ({e})", p.display()));
            }
        }
    }
    let start = canon(start);
    let mut cur = if start.is_dir() { Some(start.as_path()) } else { start.parent() };
    while let Some(d) = cur {
        let cand = d.join(".candor/config");
        if cand.is_file() {
            match std::fs::read_to_string(&cand) {
                Ok(t) => return Some((canon(&cand), t)),
                Err(e) => {
                    eprintln!("candor: {} exists but could not be read ({e}) — failing (exit 2,", cand.display());
                    eprintln!("        unevaluable). Treating it as absent would run without what it declares.");
                    refuse_unevaluable(&format!("{} exists but could not be read ({e})", cand.display()));
                }
            }
        }
        cur = d.parent();
    }
    None
}

/// One `deny <Effect…> [scope]` / `pure <scope>` rule (AS-EFF-006). `effects` empty ⇒ a `pure` rule
/// (ANY effect forbidden). `scope` is a path segment-scope the rule applies to (None = whole unit).
#[derive(Debug, Clone)]
pub struct PolicyRule {
    pub effects: BTreeSet<&'static str>,
    pub scope: Option<String>,
    /// Reason-class filter on an `Unknown` membership (REASON-SCOPED-UNKNOWN-DESIGN.md): empty ⇒
    /// `Unknown[*]` (any reason — the bare form); non-empty ⇒ the Unknown hit fires ONLY for a fn whose
    /// (transitive) reason classes include one of these. Ignored when `effects` doesn't contain `Unknown`.
    pub unknown_classes: BTreeSet<ReasonClass>,
    /// Destination-class filter on a `Net` membership (NET-DESTINATION-CLASS-DESIGN.md): empty ⇒ `Net[*]`
    /// (any destination — the bare form); non-empty ⇒ the Net hit fires ONLY for a fn whose (transitive)
    /// destination classes include one of these. Ignored when `effects` doesn't contain `Net`.
    pub net_classes: BTreeSet<String>,
    pub raw: String,
}

/// One `allow <Effect> [in <scope>] <literal>…` rule (AS-EFF-008). The effect is one of the four
/// that carry a literal surface (`Net` hosts / `Exec` commands / `Fs` paths / `Db` tables); a
/// function in `scope` performing it may reach ONLY the listed literals. Matching is
/// effect-specific (`literal_allowed`).
#[derive(Debug, Clone)]
pub struct AllowRule {
    pub effect: &'static str,
    pub scope: Option<String>,
    pub literals: BTreeSet<String>,
    pub raw: String,
}

/// One `forbid <A> -> <B>` module-layering rule (AS-EFF-009): a function in scope `A` must not
/// transitively call into scope `B`.
#[derive(Debug, Clone)]
pub struct LayerRule {
    pub from: String,
    pub to: String,
    pub raw: String,
}

/// ⟨0.24⟩ ONE POLICY LINE THE PARSER DID NOT HONOUR AS WRITTEN (SPEC §3.1 `901f14d` / `195d45a`).
///
/// **THE DEFECT.** `parsepolicy` emitted **no `errors` key at all** (measured 2026-07-28 on the
/// conformance battery: java 10, ts 4, rust 0, swift 0). Every one of these facts existed — they were
/// printed to stderr as "ignoring policy rule …" — so the verb whose entire purpose is to let a consumer
/// diff what an engine made of a policy answered the question with the not-honoured half deleted. Worse,
/// it was INCONSISTENT with this engine's own gate: the gate refuses an unrecognised class token while
/// the parse narrowed it silently, which is two answers to one question.
///
/// **`kind` IS A CLOSED SET, AND IT IS THE SPEC'S, NOT THE REFERENCE ENGINE'S.** `901f14d` pins four
/// values: `reason-class/alias`, `Net destination-class`, `effect-name`, `rule-kind`. Measured, candor-java
/// emits `forbid form`, `allow values` and `rule kind` (space, not hyphen) — three values outside the set
/// and one spelling divergence — and candor-ts renames `kind`→`vocabulary` and `rule`→`where` and emits
/// `accepted` as a PROSE STRING. This engine follows the clause: a line that names a rule keyword but does
/// not form that keyword's rule formed no rule kind, so it is `rule-kind`.
///
/// `accepted` is an ARRAY OF TOKENS — the tokens that WOULD have been honoured in the position the bad one
/// occupies. Empty where the position is open-ended (a host, a path), which is a fact about the grammar
/// rather than a gap in the report.
#[derive(Debug, Clone)]
pub struct PolicyError {
    /// One of [`PolicyError::KIND_REASON_CLASS`], [`PolicyError::KIND_NET_CLASS`],
    /// [`PolicyError::KIND_EFFECT_NAME`], [`PolicyError::KIND_RULE_KIND`].
    pub kind: &'static str,
    /// The offending token, verbatim. Empty when the position was EMPTY (a missing arrow, an `allow` with
    /// no values) — which is itself the finding.
    pub token: String,
    /// The tokens accepted in that position. Empty ⇒ the position takes an open-ended literal.
    pub accepted: Vec<String>,
    /// The raw policy line, verbatim.
    pub rule: String,
    /// The human sentence — the same text the stderr channel carries, so the two cannot disagree.
    pub message: String,
    /// ⟨0.24⟩ Does this error make the policy UNHONOURABLE, so every gate route must refuse (exit 2)?
    ///
    /// FATAL and REPORTED are different questions and this field is the only place they are told apart.
    /// A dropped `nonsense line` is reported and survivable — the rest of the policy means what it says.
    /// A rewritten `deny Unknown[dispatch,nativ]` is not: the rule that RAN is not the rule that was
    /// written, and the direction that matters NARROWS it.
    pub fatal: bool,
}

impl PolicyError {
    pub const KIND_REASON_CLASS: &'static str = "reason-class/alias";
    pub const KIND_NET_CLASS: &'static str = "Net destination-class";
    pub const KIND_EFFECT_NAME: &'static str = "effect-name";
    pub const KIND_RULE_KIND: &'static str = "rule-kind";
}

/// The rule kinds parsed from a CANDOR_POLICY file.
#[derive(Default, Debug)]
pub struct ParsedPolicy {
    pub rules: Vec<PolicyRule>,
    pub allow_rules: Vec<AllowRule>,
    pub layer_rules: Vec<LayerRule>,
    /// ⟨0.24⟩ POLICY ERRORS — a policy that cannot be honoured AS WRITTEN (SPEC §6.2). Non-empty ⇒ every
    /// gate route MUST refuse: exit 2, the unreadable-policy posture. Not a warning list: the rules in
    /// `rules` are what the text would mean if the error were tolerated, and tolerating it is the defect.
    ///
    /// Today the only member is an unrecognised reason-class/alias token in an `Unknown[…]` filter. The
    /// asymmetry that used to justify a warning — "a dropped policy token leaves a WIDER rule standing,
    /// so the failure is loud" — is false in the case that matters, and the false half is FAIL-OPEN:
    ///
    ///   - `deny Unknown[corp]` (sole unrecognised token) — the filter empties and the rule WIDENS to a
    ///     bare `deny Unknown`, while the engine prints "ignoring policy rule" and then KEEPS and
    ///     re-scopes it. Merely surprising, but a FALSE DISCLOSURE.
    ///   - `deny Unknown[dispatch,nativ]` (a typo BESIDE valid tokens) — the token is dropped, the rule
    ///     NARROWS to `[dispatch]`, and it stops gating native-caused holes entirely while the operator
    ///     reads a gate that looks armed. **That is the fail-open, and it is the common case: a typo
    ///     lands beside correct tokens far more often than alone.**
    ///
    /// A policy that cannot be honoured as written is not silently rewritten into a different policy.
    ///
    /// ⟨0.24⟩ THIS LIST NOW HOLDS EVERY LINE THE PARSER DID NOT HONOUR, fatal or not (SPEC §3.1
    /// `195d45a`) — `parsepolicy` reports them all, and the gate routes refuse on
    /// [`ParsedPolicy::fatal_messages`] alone. Widening the LIST without widening what REFUSES is the
    /// whole of the change: a dropped `nonsense line` was always survivable and stays so.
    pub errors: Vec<PolicyError>,
    /// ⟨0.24⟩ The `.candor/config` `unknown-alias` definitions this policy actually resolved a token
    /// through (SPEC §3.1) — **name → the reason-class TOKENS it expanded to**, not a bare name list.
    /// Non-empty ⇒ a config file supplied vocabulary that PARTICIPATED in the verdict, and the
    /// `--gate-json` document MUST name that file. Recorded at the point of USE, not from the alias map:
    /// a config defining ten aliases none of which the policy mentions changed nothing, and naming it
    /// would train the reader to ignore the field.
    ///
    /// ⟨0.24⟩ **THE VALUE TRAVELS WITH THE NAME, AND THAT IS A SPEC MUST** (§3.1, candor-spec `7f5b5ba`).
    /// This engine shipped the bare name — as did java and swift — and candor-ts kept the map and won the
    /// argument from the clause's OWN sentence: `configSources: [path]` is rejected there because *a
    /// disclosure that names the source but not the content leaves the reader knowing they were affected
    /// and not how*, and `["corp"]` fails that same test one level down. **`corp = reflect` and
    /// `corp = reflect,native` gate DIFFERENTLY under one unchanged policy line**, so a reader given only
    /// the name cannot tell which gate ran. The map is a strict superset — the keys recover the old array.
    ///
    /// Class TOKENS rather than `ReasonClass`, so the wire order is the token's alphabetical one (which
    /// is what candor-ts's `[...set].sort()` produces) and not `ReasonClass`'s declaration order.
    pub used_aliases: BTreeMap<String, BTreeSet<String>>,
}

impl ParsedPolicy {
    /// ⟨0.24⟩ The messages of the errors that make the policy UNHONOURABLE — what every gate route
    /// refuses on. Non-empty ⇒ exit 2, the unreadable-policy posture.
    ///
    /// Separate from `errors` because REPORTED and FATAL are different questions, and conflating them in
    /// either direction is a defect: refusing on every dropped line would make `nonsense line` fail a
    /// build, and reporting only the fatal ones is the silent narrowing this rung exists to close.
    pub fn fatal_messages(&self) -> Vec<&str> {
        self.errors.iter().filter(|e| e.fatal).map(|e| e.message.as_str()).collect()
    }
}

/// The hostname part of a `host[:port]` literal, port stripped — so `api.stripe.com` in a rule accepts
/// a reached `api.stripe.com:443`. IPv6-aware: a bracketed `[host]:port` yields the bracketed host, and
/// a BARE IPv6 literal (>1 colon, no brackets) has no port to strip and is returned whole — a naive
/// first-colon split collapsed every `2001:db8::*` to `2001`, so one allowed IPv6 accepted any address
/// in that block (/code-review). A hostname/IPv4 `host` or `host:port` (≤1 colon) splits at the colon.
pub fn host_part(h: &str) -> &str {
    if let Some(rest) = h.strip_prefix('[') {
        // `[ipv6]` or `[ipv6]:port` — the host is between the brackets.
        return rest.split(']').next().unwrap_or(rest);
    }
    if h.matches(':').count() > 1 {
        return h; // bare IPv6 literal — no port suffix to strip
    }
    h.split(':').next().unwrap_or(h)
}

/// The basename of a command (`/usr/bin/git` → `git`), so `allow Exec … git` accepts an absolute path.
pub fn cmd_base(c: &str) -> &str {
    c.rsplit(['/', '\\']).next().unwrap_or(c)
}

/// Whether an allowed path `a` covers a reached path `r` (SPEC §6.2: path-boundary-respecting prefix).
/// A directory covers itself and everything beneath it, but NOT a sibling sharing a textual prefix
/// (`/etc/app` ⊉ `/etc/apppwned`); a `..` that climbs out is never covered; absolute/relative are
/// never conflated.
pub fn fs_path_covered(a: &str, r: &str) -> bool {
    if r.split(['/', '\\']).any(|c| c == "..") {
        return false;
    }
    let absolute = |s: &str| s.starts_with('/') || s.starts_with('\\');
    if absolute(a) != absolute(r) {
        return false;
    }
    let norm = |s: &str| -> Vec<String> {
        s.split(['/', '\\'])
            .filter(|c| !c.is_empty() && *c != ".")
            .map(|c| c.to_string())
            .collect()
    };
    let (ac, rc) = (norm(a), norm(r));
    ac.len() <= rc.len() && ac.iter().zip(&rc).all(|(x, y)| x == y)
}

/// Whether an allowed table entry `a` covers a reached table `r` (SPEC §6.2): case-insensitive
/// exact match on the (possibly schema-qualified) name, or a `schema.*` entry covering every table
/// in that schema. Strict on qualification — an allowed `entries` does NOT cover a reached
/// `ledger.entries` (write both forms if your queries mix them); silent widening is the failure
/// mode an allowlist exists to prevent.
pub fn db_table_covered(a: &str, r: &str) -> bool {
    let (a, r) = (a.to_lowercase(), r.to_lowercase());
    if let Some(schema) = a.strip_suffix(".*") {
        return r.strip_prefix(schema).is_some_and(|rest| rest.starts_with('.'));
    }
    a == r
}

/// Whether a reached literal is allowed under an effect-specific match (SPEC §6.2): `Net` host by
/// name (port ignored), `Exec` command by basename, `Fs` path by boundary-respecting prefix,
/// `Db` table by qualified name or `schema.*`.
pub fn literal_allowed(effect: &str, reached: &str, allow: &BTreeSet<String>) -> bool {
    match effect {
        // `Llm` ⟨0.13⟩ rides the Net host literal (SPEC §1) — matched by hostname like `Net`.
        "Net" | "Llm" => allow.iter().any(|a| host_part(a) == host_part(reached)),
        "Exec" => allow.iter().any(|a| cmd_base(a) == cmd_base(reached)),
        "Fs" => allow.iter().any(|a| fs_path_covered(a, reached)),
        "Db" => allow.iter().any(|a| db_table_covered(a, reached)),
        _ => allow.contains(reached),
    }
}

/// Split a function name (or scope) into PATH SEGMENTS on either separator. Reports reach the Rust gate
/// AND `candor-query` from BOTH the Rust engines (`::`-separated names) and the JVM/Swift/TS engines
/// (`.`-separated names — `candor-query` is explicitly built to read them). Segmenting on `::` ALONE
/// left a scoped `deny`/`pure` rule silently INERT on a dotted name: the scope matched nothing, so
/// `whatif` returned a false green on the security boundary (gate-evasion). The JVM engine's own
/// `scopeMatches` already splits on `.`; this aligns the Rust side. A `:`/`.` never appears WITHIN a
/// real segment, so splitting on both never over-segments a Rust name (no spurious match).
fn name_segments(s: &str) -> Vec<&str> {
    s.split(['.', ':']).filter(|p| !p.is_empty()).collect()
}

/// A policy scope matches a function name by **path segment** (SPEC §6.2), not substring: split both
/// into segments (on `::` or `.`); the scope matches a contiguous run of name-segments where every
/// segment except the last matches exactly and the last is a prefix. So `domain` matches
/// `app::domain::h`, `com.acme.domain.h`, and `domain_logic` but not `subdomain`.
pub fn scope_matches(name: &str, scope: &str) -> bool {
    let segs = name_segments(name);
    let parts = name_segments(scope);
    if parts.is_empty() || parts.len() > segs.len() {
        return false;
    }
    let (last, init) = parts.split_last().unwrap();
    segs.windows(parts.len()).any(|w| {
        let (w_last, w_init) = w.split_last().unwrap();
        w_init == init && w_last.starts_with(last)
    })
}

/// Reconstruct a rule's source form and the `Unknown`-forbidding upgrade for it: `pure <scope>` →
/// (`"pure <scope>"`, `"deny Unknown <scope>"`); `deny <E…> <scope>` → (`"deny <E…> <scope>"`,
/// `"deny <E…> Unknown <scope>"`). Shared so the gate note and `candor unverified` name the identical
/// rule and upgrade — one source of truth for the disclosure's advice.
///
/// ⟨0.24⟩ **THE NARROWING FILTERS ARE RENDERED, and they had to start being rendered in the same commit
/// that made them REACHABLE here.** Making `unverified_hole_rule` filter-aware is what first lets a
/// `deny Unknown[reflect]` / `deny Net[unknown-host]` rule be the rule a hole is disclosed under — and
/// this reconstruction dropped the bracket, so the fix would have printed the operator's narrowed rule
/// back to them as the WIDE one (`deny Unknown`) and advised the nonsense upgrade `deny Unknown
/// Unknown`. That is the same mis-attribution `whatif` carries, arriving through the fix for a different
/// defect: the hazard on this rung is that each correction manufactures its own mirror, so the rendering
/// moves with the predicate rather than after it.
///
/// A rule carrying NO filter renders byte-identically to before, which is what keeps conformance PARTs
/// 12c/12d (`deny Db Net Unknown domain`, four-way) unmoved.
///
/// THE UPGRADE SPLITS on whether the rule already denies `Unknown`. If it does, it can only be here
/// NARROWED — a bare `deny … Unknown` fires on every `Unknown`, so the function would be a violation and
/// not a hole — and the upgrade is that term WIDENED to bare `Unknown`, not a second `Unknown` appended.
pub fn rule_and_upgrade(r: &PolicyRule) -> (String, String) {
    let scope = r.scope.clone().unwrap_or_default();
    let suffix = if scope.is_empty() { String::new() } else { format!(" {scope}") };
    if r.effects.is_empty() {
        // `pure` forbids real effects but not Unknown; to REQUIRE provable purity, add a deny-Unknown.
        return (format!("pure{suffix}"), format!("deny Unknown{suffix}"));
    }
    // One effect term, with its narrowing filter if it has one. Class tokens sorted by TOKEN string, as
    // `parsepolicy` sorts them and as the java reference's `.sorted()` does — the dump and the
    // disclosure must spell one rule one way.
    let term = |e: &str| -> String {
        if e == UNKNOWN && !r.unknown_classes.is_empty() {
            let mut t: Vec<&str> = r.unknown_classes.iter().map(|c| c.token()).collect();
            t.sort_unstable();
            format!("{UNKNOWN}[{}]", t.join(","))
        } else if e == "Net" && !r.net_classes.is_empty() {
            format!("Net[{}]", r.net_classes.iter().map(String::as_str).collect::<Vec<_>>().join(","))
        } else {
            e.to_string()
        }
    };
    let effs = r.effects.iter().map(|e| term(e)).collect::<Vec<_>>().join(" ");
    if r.effects.contains(UNKNOWN) {
        let widened =
            r.effects.iter().map(|e| if *e == UNKNOWN { UNKNOWN.to_string() } else { term(e) }).collect::<Vec<_>>();
        (format!("deny {effs}{suffix}"), format!("deny {}{suffix}", widened.join(" ")))
    } else {
        (format!("deny {effs}{suffix}"), format!("deny {effs} {UNKNOWN}{suffix}"))
    }
}

/// The single predicate for a provable-purity hole (eval/fixloop/DISPATCH-NOTE.md): a function that is
/// `Unknown`, sits in a `pure`/`deny <E>` scope, and PASSES that rule (carries none of its forbidden real
/// effects) — so its compliance is asserted but not verified (the Unknown could hide the very effect the
/// rule forbids; the classic case is a fn/closure-injected port). A *real* violation is the gate's job, not
/// this. Returns the first governing rule under which the function is such a hole, or `None` if it is not
/// one. Shared by candor-scan's gate note and candor-query's `unverified` so "what a hole is" has ONE
/// definition — the two paths can never drift (conformance PART 12d pins their agreement).
///
/// ⟨0.24⟩ **"PASSES" IS NOW ASKED OF THE GATE, NOT OF A SECOND COPY OF IT.** This predicate computed the
/// passing test from `r.effects` alone — "does the rule NAME an effect this function has?" — which is
/// the pre-⟨0.19⟩ question, still being asked after two rungs gave rules a NARROWING FILTER. So on
/// `deny Unknown[reflect]` over an `indirect` hole the gate TOLERATED (exit 0, the class does not match)
/// while this read the same rule as violated; and a hole is *by definition* a function that PASSES its
/// rule while `Unknown`, so the real hole was reclassified as a violation-that-isn't and **deleted from
/// the disclosure** — `unverified` answering "every function in a pure/deny layer is PROVABLY clean ✓"
/// over a function the gate had just declined to clear. MEASURED 2026-07-28, and reachable with no alias
/// in play at all: one layer below the widening `ea0df4f` closed, in the same four verbs.
///
/// It now calls [`crate::gate::rule_hits`] — the gate's own firing decision — so the two cannot disagree
/// again. That needs the function's TRANSITIVE reason classes and its ⟨0.20⟩ destination classes, the
/// same two accumulators the gate reads, which is why they are parameters now rather than derivable.
///
/// **THE DIRECTION IS THE MIRROR ARGUMENT.** A filter can only ever SHRINK what a rule charges, so a
/// filter-aware pass test can only ever find MORE holes — this cannot silently suppress a disclosure
/// that used to appear. Pinned in both directions by
/// `a_narrowed_rule_the_gate_tolerates_is_a_hole_and_the_one_it_fires_on_is_not`, in one run, because
/// the fixture proving the fabrication is closed cannot show the reach closed with it. A WITHHELD filter
/// — nothing to read — likewise counts as PASSING here: the gate did not clear the function, and an
/// advisory note's fail-safe direction is to disclose.
pub fn unverified_hole_rule<'a, S: AsRef<str>>(
    name: &str,
    effects: &[S],
    reason_classes: Option<&BTreeSet<String>>,
    net_classes: &[String],
    rules: &'a [PolicyRule],
) -> Option<&'a PolicyRule> {
    if !effects.iter().any(|e| e.as_ref() == UNKNOWN) {
        return None;
    }
    let effs: Vec<&str> = effects.iter().map(AsRef::as_ref).collect();
    rules.iter().find(|r| {
        // in the rule's scope (a scopeless rule governs the whole unit) …
        if let Some(s) = &r.scope {
            if !scope_matches(name, s) {
                return false;
            }
        }
        // … and PASSES it — the gate's own answer, narrowing filters and all. Empty `hits` IS passing.
        crate::gate::rule_hits(r, &effs, reason_classes, net_classes).hits.is_empty()
    })
}

/// Parse a CANDOR_POLICY file (SPEC §6.2). One rule per line; `#` comments and blanks ignored:
///
/// ```text
/// deny Net Db  domain     # functions whose path contains segment "domain" must not perform Net or Db
/// deny Exec               # no function anywhere may perform Exec
/// deny Unknown  api        # functions in "api" must be fully resolvable (forbid the unverifiable)
/// pure         parse      # functions whose path contains segment "parse" must be effect-free
/// allow Net in billing  api.stripe.com
/// forbid domain -> infra
/// ```
///
/// In a `deny` rule, leading tokens that name a known effect (or `Unknown`) are forbidden; the FIRST
/// non-effect token is the scope and ends the rule. A `deny` naming no known effect is dropped (it is
/// NOT a `pure` rule). Malformed/unknown lines are ignored with a warning — never silently widened.
/// The §6.2 token separator: ASCII whitespace ONLY (space/tab/CR/LF/VT/FF). `split_whitespace`/`trim`
/// use Unicode `White_Space`, which would split a NBSP/ideographic space that Java drops — a gateless-
/// green cross-engine divergence (adversarial DSL review). A non-ASCII space stays part of its token, so
/// the rule is malformed and ignored, uniformly.
fn is_ascii_ws(c: char) -> bool {
    matches!(c, ' ' | '\t' | '\n' | '\x0b' | '\x0c' | '\r')
}

pub fn parse_policy(text: &str) -> ParsedPolicy {
    parse_policy_impl(text, true, &std::collections::BTreeMap::new())
}
/// As [`parse_policy`] but with `.candor/config` `unknown-alias` definitions (⟨0.19⟩, SPEC §6.2): an
/// `Unknown[<name>]` filter resolves a user-defined `<name>` to its reason classes. The gate + `parsepolicy`
/// pass the discovered aliases (via [`parse_unknown_aliases`]); a config alias never changes what bare
/// `deny E Unknown` means (always `Unknown[*]`), so the rule stays legible from the policy alone.
pub fn parse_policy_with_aliases(text: &str, aliases: &std::collections::BTreeMap<String, std::collections::BTreeSet<ReasonClass>>) -> ParsedPolicy {
    parse_policy_impl(text, true, aliases)
}
/// ⟨0.24⟩ [`parse_policy_with_aliases`] but SILENT — for a caller that must inspect
/// [`ParsedPolicy::errors`] / [`ParsedPolicy::used_aliases`] BEFORE it parses for real (candor-scan
/// refuses before touching the classifier's accumulators). Silent so the ordinary parse warnings are not
/// printed twice on the same text.
pub fn parse_policy_silent(
    text: &str,
    aliases: &std::collections::BTreeMap<String, BTreeSet<ReasonClass>>,
) -> ParsedPolicy {
    parse_policy_impl(text, false, aliases)
}
/// Same as [`parse_policy`] but SILENT about malformed rules — for a SECOND, advisory re-parse within the
/// same run (candor-scan parses once for the gate check and again for the `unverified` disclosure), so the
/// CI log doesn't print every "ignoring policy rule …" warning twice (#21). The first parse already warned.
pub fn parse_policy_quiet(text: &str) -> ParsedPolicy {
    parse_policy_impl(text, false, &std::collections::BTreeMap::new())
}
fn parse_policy_impl(text: &str, warn: bool, aliases: &std::collections::BTreeMap<String, std::collections::BTreeSet<ReasonClass>>) -> ParsedPolicy {
    macro_rules! warn_ignore { ($($a:tt)*) => { if warn { eprintln!($($a)*); } } }
    let mut out = ParsedPolicy::default();
    // ⟨0.24⟩ Record a line the parser did not honour (SPEC §3.1 `195d45a`), on the ONE list `parsepolicy`
    // reports and the gate routes filter for `fatal`. The stderr sentence and `message` are the SAME
    // string by construction — a disclosure that can drift from the one beside it is how this family
    // produced a FALSE disclosure once already (conformance PART 13b).
    macro_rules! not_honoured {
        ($fatal:expr, $kind:expr, $token:expr, $accepted:expr, $rule:expr, $msg:expr) => {{
            let message: String = $msg;
            out.errors.push(PolicyError {
                kind: $kind,
                token: ($token).to_string(),
                accepted: ($accepted).iter().map(|s: &&str| s.to_string()).collect(),
                rule: ($rule).to_string(),
                message,
                fatal: $fatal,
            });
        }};
    }
    // `str::lines()` splits on \n and \r\n but NOT bare \r — a classic-Mac file then collapses to ONE
    // line, and since \r is also an in-line ASCII-ws token separator (is_ascii_ws), every rule after the
    // first was glued into the first rule's tokens and dropped (sweep [16], a gateless-green divergence).
    // Java's Files.readAllLines (the reference) breaks on bare \r too — normalize to match it. Allocation
    // only when a bare \r is actually present (the overwhelmingly-common \n / \r\n files are untouched).
    let normalized;
    let text = if text.contains('\r') {
        normalized = text.replace("\r\n", "\n").replace('\r', "\n");
        normalized.as_str()
    } else {
        text
    };
    for raw_line in text.lines() {
        let line = raw_line.split('#').next().unwrap_or("").trim_matches(is_ascii_ws);
        if line.is_empty() {
            continue;
        }
        let mut toks = line.split(is_ascii_ws).filter(|s| !s.is_empty());
        match toks.next().unwrap_or("") {
            "allow" => {
                let effect = match toks.next().unwrap_or("") {
                    "Net" => "Net",
                    // `Llm` ⟨0.13⟩ rides the Net host literal (SPEC §1) — `allow Llm <host…>` restricts which
                    // MODEL hosts a scope may reach, matched by hostname like Net (its reached surface IS the
                    // Net host surface). Match candor-java's Policy.parsePolicy.
                    "Llm" => "Llm",
                    "Exec" => "Exec",
                    "Fs" => "Fs",
                    "Db" => "Db",
                    other => {
                        let msg = format!(
                            "unknown effect-name `{other}` in `allow` (accepted: Db, Exec, Fs, Llm, Net \
                             \u{2014} `allow` covers only the effects carrying a literal surface: Net/Llm \
                             hosts, Exec commands, Fs paths, Db tables): {line}"
                        );
                        warn_ignore!("candor: policy error — {msg}");
                        // ⟨0.24⟩ FATAL (SPEC §6.2 `1e1748a`). MEASURED four-way before this:
                        // `allow Nett host.example` -> exit 0 on rust, ts, java AND swift. The rule is
                        // DELETED and the certification silently vanishes, so the operator reads an
                        // armed allowlist that does not exist.
                        //
                        // The grammar defence that kept the token rule inside the bracket does NOT
                        // reach here: `allow`'s effect position is a fixed, closed set with **no scope
                        // reading available**, so an unrecognised token there is unambiguously a typo
                        // and there is no legitimate policy it could be. This document already calls a
                        // dropped rule "the limit case of silently rewritten into a different policy…
                        // a bigger rewrite than a narrowed filter" — and the bigger rewrite was
                        // warning-only while the smaller one was already exit 2.
                        not_honoured!(
                            true,
                            PolicyError::KIND_EFFECT_NAME,
                            other,
                            ["Db", "Exec", "Fs", "Llm", "Net"],
                            line,
                            msg
                        );
                        continue;
                    }
                };
                let mut rest: Vec<&str> = toks.collect();
                let scope = if rest.first() == Some(&"in") {
                    let s = rest.get(1).map(|s| s.to_string());
                    rest.drain(..2.min(rest.len()));
                    s
                } else {
                    None
                };
                let literals: BTreeSet<String> = rest.iter().map(|h| h.to_string()).collect();
                if literals.is_empty() {
                    let msg = format!("`allow {effect}` names no values: {line}");
                    warn_ignore!("candor: ignoring policy rule ({msg})");
                    // `accepted` is EMPTY on purpose: the position takes an open-ended literal (a host, a
                    // path, a command, a table), so there is no token list to offer. A fact about the
                    // grammar, not a gap in the report.
                    not_honoured!(false, PolicyError::KIND_RULE_KIND, "", [], line, msg);
                    continue;
                }
                out.allow_rules.push(AllowRule { effect, scope, literals, raw: line.to_string() });
            }
            "deny" => {
                let mut effects = BTreeSet::new();
                let mut scope = None;
                // Reason-class filter on `Unknown` (REASON-SCOPED-UNKNOWN-DESIGN.md): empty ⇒ `Unknown[*]`
                // (any reason — the bare form); non-empty ⇒ only those classes. `*` = all.
                let mut unknown_classes: BTreeSet<ReasonClass> = BTreeSet::new();
                let mut unknown_star = false;
                // Destination-class filter on `Net` (NET-DESTINATION-CLASS-DESIGN.md): empty ⇒ `Net[*]`
                // (any destination — the bare form); non-empty ⇒ only those classes. `*` = all.
                let mut net_classes: BTreeSet<String> = BTreeSet::new();
                let mut net_star = false;
                for t in toks {
                    // `Net[unknown-host]` / `Net[*]` / `Net[known-telemetry,unknown-host]`: the destination-scoped form.
                    if let Some(inner) = t.strip_prefix("Net[").and_then(|s| s.strip_suffix(']')) {
                        effects.insert("Net");
                        for cn in inner.split(',') {
                            let cn = cn.trim();
                            if cn.is_empty() {
                                continue;
                            }
                            if cn == "*" {
                                net_star = true;
                            } else if crate::NET_DEST_CLASSES.contains(&cn) {
                                net_classes.insert(cn.to_string());
                            } else {
                                // ⟨0.24⟩ A POLICY ERROR, not a warning — SPEC §6.2 `be0b9a9`. Byte-identical
                                // in shape to the reason-class arm below, and byte-identical in harm:
                                // MEASURED `deny Net[known-telemetry,unknown-hosst]` → exit 0 where the
                                // correctly-spelled rule exits 1. The typo is dropped, the filter NARROWS
                                // to `[known-telemetry]`, and the gate stops covering unidentifiable
                                // destinations while the operator reads a gate that looks armed.
                                not_honoured!(
                                    true,
                                    PolicyError::KIND_NET_CLASS,
                                    cn,
                                    ["known-telemetry", "known-partner", "unknown-host", "*"],
                                    line,
                                    format!(
                                        "unrecognised Net destination-class `{cn}` in `{line}` — \
                                         accepted: known-telemetry, known-partner, unknown-host, plus `*`"
                                    )
                                );
                            }
                        }
                        continue;
                    }
                    // `Unknown[dispatch,reflect]` / `Unknown[*]` / `Unknown[dynamic]`: the reason-scoped form.
                    if let Some(inner) = t.strip_prefix("Unknown[").and_then(|s| s.strip_suffix(']')) {
                        effects.insert(UNKNOWN);
                        for cn in inner.split(',') {
                            let cn = cn.trim();
                            if cn.is_empty() {
                                continue;
                            }
                            if cn == "*" {
                                unknown_star = true;
                            } else if cn == "dynamic" {
                                unknown_classes.extend(ReasonClass::dynamic_set());
                            } else if let Some(rc) = ReasonClass::from_token(cn) {
                                unknown_classes.insert(rc);
                            } else if let Some(a) = aliases.get(cn) {
                                unknown_classes.extend(a.iter().copied()); // ⟨0.19⟩ config `unknown-alias`
                                // ⟨0.24⟩ → the verdict names it AND what it expanded to (SPEC §3.1
                                // `7f5b5ba`): the NAME alone cannot tell a reader which gate ran.
                                out.used_aliases
                                    .insert(cn.to_string(), a.iter().map(|c| c.token().to_string()).collect());
                            } else {
                                // ⟨0.24⟩ A POLICY ERROR, not a warning — see `ParsedPolicy::errors`. The
                                // token is still dropped below so `rules` stays well-formed for the
                                // advisory readers (`unverified`, `parsepolicy`); the gate routes refuse
                                // on `errors` before any of it is used as a verdict.
                                not_honoured!(
                                    true,
                                    PolicyError::KIND_REASON_CLASS,
                                    cn,
                                    [
                                        "reflect",
                                        "dispatch",
                                        "indirect",
                                        "native",
                                        "unresolved",
                                        "setup",
                                        "dynamic",
                                        "*"
                                    ],
                                    line,
                                    format!(
                                        "unrecognised reason-class/alias `{cn}` in `{line}` — accepted: \
                                         reflect, dispatch, indirect, native, unresolved, setup, plus the \
                                         aliases `dynamic` and `*`, plus any `unknown-alias` defined in \
                                         the `.candor/config` beside the policy. (⟨0.24⟩ an \
                                         `unknown-alias` whose OWN definition names an unrecognised class \
                                         is refused WHOLE, so a typo in the config surfaces as an \
                                         undefined alias here — check the `unknown-alias` lines too, and \
                                         the line above this one.)"
                                    )
                                );
                            }
                        }
                        continue;
                    }
                    let e = if t == UNKNOWN { Some(UNKNOWN) } else { cap_from_name(t) };
                    match e {
                        Some(e) => {
                            effects.insert(e);
                            if e == UNKNOWN {
                                unknown_star = true; // bare Unknown ⇒ all classes
                            }
                            if e == "Net" {
                                net_star = true; // bare Net ⇒ all destinations
                            }
                        }
                        None => {
                            scope = Some(t.to_string());
                            break;
                        }
                    }
                }
                if effects.is_empty() {
                    // The accepted set is the §1 effect vocabulary plus `Unknown` — SORTED, so the
                    // document is deterministic and diffable across engines.
                    let mut acc: Vec<&str> = candor_report::EFFECTS.to_vec();
                    acc.push(UNKNOWN);
                    acc.sort_unstable();
                    let msg = format!(
                        "`deny` names no known effect (accepted: {}): {line}",
                        acc.join(", ")
                    );
                    warn_ignore!("candor: policy error — {msg}");
                    // ⟨0.24⟩ FATAL (SPEC §6.2 `1e1748a`). MEASURED four-way: `deny Nett app` -> exit 0
                    // on all four; the rule is DELETED and the gate is green. `Nett` is read as the
                    // SCOPE (the first unrecognised token ends the effect list), so the line parses to
                    // a deny of NOTHING.
                    //
                    // **A `deny` whose effect list ends up EMPTY is malformed under EITHER reading** —
                    // typo-in-the-effect or scope-with-no-effect are both nonsense — so there is no
                    // legitimate policy it could be and refusing it loses nothing. What stays open is
                    // only the genuinely ambiguous middle (`deny Net Exex app`: at least one valid
                    // effect plus an unrecognised trailing token that MIGHT be a scope), which the
                    // parser cannot tell from a legitimate scope and which `parsepolicy` shows either
                    // way by dumping the `scope` it read.
                    not_honoured!(
                        true,
                        PolicyError::KIND_EFFECT_NAME,
                        scope.as_deref().unwrap_or(""),
                        acc,
                        line,
                        msg
                    );
                    continue;
                }
                // `*` (or bare `Unknown`) means all classes ⇒ empty filter (matches any Unknown).
                if unknown_star {
                    unknown_classes.clear();
                } else if !unknown_classes.is_empty() && !unknown_classes.contains(&ReasonClass::Unresolved) {
                    // A2 under-gating lint: a narrowed scope that omits `unresolved` (the catch-all for holes
                    // an engine couldn't classify) may silently tolerate exactly those — flag it (advisory).
                    warn_ignore!("candor: policy rule narrows `Unknown[…]` but omits `unresolved` — may UNDER-gate on holes the engine couldn't classify; add `unresolved` (or use `dynamic`) to stay conservative: {line}");
                }
                // `*` (or bare `Net`) means all destinations ⇒ empty filter (matches any Net).
                if net_star {
                    net_classes.clear();
                }
                out.rules.push(PolicyRule { effects, scope, unknown_classes, net_classes, raw: line.to_string() });
            }
            "pure" => out.rules.push(PolicyRule {
                effects: BTreeSet::new(),
                scope: toks.next().map(str::to_string),
                unknown_classes: BTreeSet::new(),
                net_classes: BTreeSet::new(),
                raw: line.to_string(),
            }),
            "forbid" => {
                let a = toks.next().unwrap_or("");
                let arrow = toks.next().unwrap_or("");
                let b = toks.next().unwrap_or("");
                if a.is_empty() || arrow != "->" || b.is_empty() {
                    let msg = format!("`forbid` is malformed (want `forbid <scope> -> <scope>`): {line}");
                    warn_ignore!("candor: ignoring layering rule ({msg})");
                    // The token reported is whatever sat in the ARROW position — `->` must be its own
                    // token, so `forbid glued->arrow` finds nothing there and that absence is the finding.
                    not_honoured!(false, PolicyError::KIND_RULE_KIND, arrow, ["->"], line, msg);
                    continue;
                }
                out.layer_rules.push(LayerRule {
                    from: a.to_string(),
                    to: b.to_string(),
                    raw: line.to_string(),
                });
            }
            other => {
                let msg = format!(
                    "unknown rule kind `{other}` (accepted: allow, deny, forbid, pure): {line}"
                );
                warn_ignore!("candor: ignoring policy rule ({msg})");
                not_honoured!(
                    false,
                    PolicyError::KIND_RULE_KIND,
                    other,
                    ["allow", "deny", "forbid", "pure"],
                    line,
                    msg
                );
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    #[test]
    fn db_table_covering_is_strict() {
        use super::db_table_covered as c;
        assert!(c("ledger.entries", "Ledger.Entries")); // case-insensitive exact
        assert!(c("ledger.*", "ledger.entries"));       // schema wildcard
        assert!(!c("ledger.*", "ledgerx.entries"));     // boundary-respecting
        assert!(!c("entries", "ledger.entries"));       // no silent qualification widening
        assert!(c("entries", "entries"));
    }

    #[test]
    fn allow_db_parses_and_gates() {
        let p = super::parse_policy("allow Db in billing  ledger.* customers\n");
        assert_eq!(p.allow_rules.len(), 1);
        assert_eq!(p.allow_rules[0].effect, "Db");
        assert!(super::literal_allowed("Db", "ledger.entries", &p.allow_rules[0].literals));
        assert!(super::literal_allowed("Db", "customers", &p.allow_rules[0].literals));
        assert!(!super::literal_allowed("Db", "audit.log", &p.allow_rules[0].literals));
    }

    use super::*;

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
        assert_eq!(rules.len(), 3);
        assert_eq!(rules[0].effects, ["Db", "Net"].into_iter().collect::<BTreeSet<_>>());
        assert_eq!(rules[0].scope.as_deref(), Some("domain"));
        assert!(rules[1].effects.contains("Exec") && rules[1].scope.is_none());
        assert!(rules[2].effects.is_empty() && rules[2].scope.as_deref() == Some("parse"));
        // sweep [16]: a classic-Mac (bare \r) multi-rule policy must NOT collapse to the first rule.
        let cr = parse_policy("deny Net a\rdeny Exec b\rdeny Db c\r");
        assert_eq!(cr.rules.len(), 3, "bare-CR lines must each parse");
        assert!(cr.rules.iter().any(|r| r.effects.contains("Exec") && r.scope.as_deref() == Some("b")));
        // mixed \r\n and bare \r normalize identically.
        assert_eq!(parse_policy("deny Net a\r\ndeny Exec b\r").rules.len(), 2);
        // `Unknown` is a denyable token; a bare `deny` with no effect is ignored.
        assert_eq!(parse_policy("deny Unknown core").rules[0].effects, ["Unknown"].into_iter().collect());
        assert!(parse_policy("deny\ndeny   \n").rules.is_empty());
        // a `deny` whose first token is a non-effect names no effect -> dropped, NOT a pure rule.
        assert!(parse_policy("deny notaneffect scope").rules.is_empty());
        // the first non-effect token ENDS the rule: a later effect token is not collected.
        let p2 = parse_policy("deny Net foo Db");
        assert_eq!(p2.rules[0].effects, ["Net"].into_iter().collect::<BTreeSet<_>>());
        assert_eq!(p2.rules[0].scope.as_deref(), Some("foo"));
        // NBSP is NOT a token separator (only ASCII White_Space is) — pinned to MATCH Java, which
        // drops it: a `deny\u{a0}Net` is one token `deny\u{a0}Net`, NOT `deny` + `Net`, so it names no
        // known effect and is dropped. Splitting on Unicode whitespace here would let candor see a deny
        // the JVM engine doesn't — a gateless-divergence between impls. (See is_ascii_ws.)
        assert!(parse_policy("deny\u{a0}Net core").rules.is_empty(),
                "an NBSP between deny and the effect must NOT split into separate tokens");
        // The NBSP rides INTO the scope token rather than separating it: `deny Net\u{a0}domain` is
        // `deny` + `Net` + `\u{a0}domain` — Net is the effect, the scope keeps the NBSP verbatim.
        let nb = parse_policy("deny Net \u{a0}domain");
        assert_eq!(nb.rules.len(), 1);
        assert_eq!(nb.rules[0].effects, ["Net"].into_iter().collect::<BTreeSet<_>>());
        assert_eq!(nb.rules[0].scope.as_deref(), Some("\u{a0}domain"));
    }

    #[test]
    fn reason_scoped_unknown_parses() {
        use super::ReasonClass::*;
        // `Unknown[dispatch,indirect]` narrows the Unknown membership to those classes.
        let p = parse_policy("deny Net Unknown[dispatch,indirect] dom\n");
        let r = &p.rules[0];
        assert!(r.effects.contains("Unknown") && r.effects.contains("Net"));
        assert_eq!(r.scope.as_deref(), Some("dom"));
        assert_eq!(r.unknown_classes, [Dispatch, Indirect].into_iter().collect());
        // bare `Unknown` and `Unknown[*]` ⇒ empty filter (all classes).
        assert!(parse_policy("deny Net Unknown dom\n").rules[0].unknown_classes.is_empty(), "bare Unknown ⇒ all");
        assert!(parse_policy("deny Net Unknown[*] dom\n").rules[0].unknown_classes.is_empty(), "Unknown[*] ⇒ all");
        // `dynamic` alias = every genuine class incl. unresolved, excl. setup.
        assert_eq!(
            parse_policy("deny Net Unknown[dynamic] dom\n").rules[0].unknown_classes,
            [Reflect, Dispatch, Indirect, Native, Unresolved].into_iter().collect()
        );
        // config `unknown-alias` (⟨0.19⟩): a user-defined name resolves; a reserved name is rejected.
        let aliases = super::parse_unknown_aliases(
            "unknown-alias risky = reflect,native\nunknown-alias telemetry = indirect\nunknown-alias reflect = native\n");
        assert_eq!(aliases.get("risky"), Some(&[Reflect, Native].into_iter().collect()));
        assert_eq!(aliases.get("telemetry"), Some(&[Indirect].into_iter().collect()));
        assert!(!aliases.contains_key("reflect"), "a config alias may not shadow a class token");
        // the `unknown-alias` KEY matches case-insensitively (parity with java/ts/swift, which lowercase it)
        assert_eq!(super::parse_unknown_aliases("Unknown-Alias hot = native\n").get("hot"),
                   Some(&[Native].into_iter().collect()), "the unknown-alias key must match case-insensitively");
        let pr = super::parse_policy_with_aliases("deny Net Unknown[risky] api\n", &aliases);
        assert_eq!(pr.rules[0].unknown_classes, [Reflect, Native].into_iter().collect());
        // an UNDEFINED alias name is dropped-with-warning → empty filter (behaves like bare Unknown[*])
        assert!(super::parse_policy_with_aliases("deny Net Unknown[nope] api\n", &aliases).rules[0].unknown_classes.is_empty());
        // classify: raw reason tokens → normative classes (mirrors java ReasonClass.classify).
        assert_eq!(ReasonClass::classify("reflect:Class.forName"), Reflect);
        assert_eq!(ReasonClass::classify("native:extern fn"), Native);
        assert_eq!(ReasonClass::classify("callback:unresolved call"), Indirect);
        assert_eq!(ReasonClass::classify("ambiguous:same-name local defs"), Dispatch);
        assert_eq!(ReasonClass::classify("unresolved"), Unresolved);
        assert_eq!(ReasonClass::classify("whatever-new"), Unresolved); // conservative catch-all
    }

    /// THE CONTROL SPEC §4 ⟨0.24⟩ MAKES A SHOULD: a FABRICATED, off-vocabulary kind must still behave as
    /// §2 forward-compatibility requires. Without it, "added a fifth kind" and "stopped checking the kind
    /// set" are the same diff — the classifier is one `_ =>` arm away from either.
    ///
    /// This engine holds the §4 vocabulary ONCE (the raw `kind:detail` string, read back only through
    /// `classify`), so there is no typed half here to drift from it. That is why the JVM engine's failure
    /// — a string classifier correct on `ambiguous` since July while its typed `Kind` enum lacked the kind
    /// entirely, one token classified two ways inside one engine — is not reproducible here. If a typed
    /// kind representation is ever added, this test is where its half gets its control.
    #[test]
    fn off_vocabulary_kinds_round_trip_and_classify_through_the_catch_all() {
        use ReasonClass::*;
        // A kind no engine emits and no section names. §2: tolerated, and classified CONSERVATIVELY —
        // `unresolved`, the catch-all, so a narrowed `Unknown[unresolved]`/`[dynamic]`/`[*]` still bites it.
        assert_eq!(ReasonClass::classify("banana:whatever"), Unresolved);
        assert_eq!(ReasonClass::classify("banana:dispatch of a banana"), Unresolved,
                   "a canonical kind appearing in the DETAIL must not leak into the classification");
        // …and it must not be swallowed into a narrower class. These are the four wrong answers.
        for wrong in [Reflect, Dispatch, Indirect, Native] {
            assert_ne!(ReasonClass::classify("banana:whatever"), wrong);
        }
        // The five §4 ⟨0.24⟩ kinds all classify, and `ambiguous` is the fifth — pinned beside the
        // fabricated one deliberately: one arm chain answers both, so a change that stops distinguishing
        // them fails here rather than in the field.
        assert_eq!(ReasonClass::classify("reflect:x"), Reflect);
        assert_eq!(ReasonClass::classify("native:x"), Native);
        assert_eq!(ReasonClass::classify("dispatch:Owner.member"), Dispatch);
        assert_eq!(ReasonClass::classify("callback:x"), Indirect);
        assert_eq!(ReasonClass::classify("ambiguous:x"), Dispatch);
        // ⟨0.24⟩ `dep:<hash>` / `dep-stale:<pkg>` are REGISTERED §4 kinds, not migration ones — swift
        // emits them per dependency ENTRY, and this engine CONSUMES swift/ts reports through
        // candor-query. §6.2 pins their class as `unresolved`, which is where the catch-all lands them;
        // pinned so a future prefix arm cannot move them without saying so.
        assert_eq!(ReasonClass::classify("dep:9f2c1a"), Unresolved);
        assert_eq!(ReasonClass::classify("dep-stale:somepkg"), Unresolved);
    }

    #[test]
    fn net_destination_class_parses_and_classifies() {
        // `Net[unknown-host,known-telemetry]` narrows the Net membership to those destination classes.
        let p = parse_policy("deny Net[unknown-host,known-telemetry] dom\n");
        let r = &p.rules[0];
        assert!(r.effects.contains("Net"));
        assert_eq!(r.scope.as_deref(), Some("dom"));
        assert_eq!(
            r.net_classes,
            ["unknown-host", "known-telemetry"].iter().map(|s| s.to_string()).collect()
        );
        // bare `Net` and `Net[*]` ⇒ empty filter (all destinations).
        assert!(parse_policy("deny Net dom\n").rules[0].net_classes.is_empty(), "bare Net ⇒ all");
        assert!(parse_policy("deny Net[*] dom\n").rules[0].net_classes.is_empty(), "Net[*] ⇒ all");
        // an unknown destination-class is dropped-with-warning → empty filter (behaves like bare Net[*]).
        assert!(parse_policy("deny Net[nope] dom\n").rules[0].net_classes.is_empty());
        // the classifier: telemetry (subdomain-aware), model host, unresolved, and the config-partner path.
        let no_partners = BTreeSet::new();
        assert_eq!(crate::net_dest_class("sentry.io", &no_partners), "known-telemetry");
        assert_eq!(crate::net_dest_class("us.i.posthog.com", &no_partners), "known-telemetry"); // 0.20.1 corpus-grown
        assert_eq!(crate::net_dest_class("o1.ingest.sentry.io", &no_partners), "known-telemetry");
        assert_eq!(crate::net_dest_class("api.openai.com", &no_partners), "known-partner", "a model host is known-partner");
        assert_eq!(crate::net_dest_class("evil.example.com", &no_partners), "unknown-host");
        let partners: BTreeSet<String> = ["api.stripe.com".to_string()].into_iter().collect();
        assert_eq!(crate::net_dest_class("api.stripe.com", &partners), "known-partner", "config-declared partner");
        assert_eq!(crate::net_dest_class("api.stripe.com", &no_partners), "unknown-host", "partner is config-only");
        // `net-partner` config parsing: host-normalized, case-insensitive key, multi-value.
        let pset = super::parse_net_partners("net-partner Api.Stripe.com:443\nNET-PARTNER hooks.stripe.com\n");
        assert!(pset.contains("api.stripe.com") && pset.contains("hooks.stripe.com"));
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
        assert_eq!(p.allow_rules.len(), 4); // Clock carries no literal surface — rejected; Db now does
        assert_eq!((p.allow_rules[0].effect, p.allow_rules[0].scope.as_deref()), ("Net", Some("billing")));
        assert_eq!(
            p.allow_rules[0].literals,
            ["api.stripe.com", "hooks.stripe.com"].iter().map(|s| s.to_string()).collect()
        );
        assert_eq!((p.allow_rules[1].effect, p.allow_rules[1].scope.as_deref()), ("Exec", Some("ci")));
        assert!(p.allow_rules[1].literals.contains("git"));
        assert_eq!((p.allow_rules[2].effect, p.allow_rules[2].scope.as_deref()), ("Fs", Some("config")));
        assert_eq!((p.allow_rules[3].effect, p.allow_rules[3].scope.is_none()), ("Net", true));

        let set = |xs: &[&str]| xs.iter().map(|s| s.to_string()).collect::<BTreeSet<_>>();
        assert!(literal_allowed("Net", "api.stripe.com:443", &set(&["api.stripe.com"])));
        // IPv6: a bare literal is matched WHOLE (no first-colon collapse), so a different address in the
        // same block is NOT accepted; a bracketed `[host]:port` matches the bare host. (/code-review.)
        assert!(literal_allowed("Net", "2001:db8::aa", &set(&["2001:db8::aa"])));
        assert!(!literal_allowed("Net", "2001:db8::ff", &set(&["2001:db8::aa"])));
        assert!(!literal_allowed("Net", "2001:dead::1", &set(&["2001:db8::aa"])));
        assert!(literal_allowed("Net", "[2001:db8::aa]:443", &set(&["2001:db8::aa"])));
        assert_eq!(host_part("2001:db8::aa"), "2001:db8::aa");
        assert_eq!(host_part("[2001:db8::aa]:443"), "2001:db8::aa");
        assert_eq!(host_part("api.stripe.com:443"), "api.stripe.com");
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
        assert_eq!(p.layer_rules.len(), 2);
        assert_eq!((p.layer_rules[0].from.as_str(), p.layer_rules[0].to.as_str()), ("domain", "infra"));
        assert_eq!((p.layer_rules[1].from.as_str(), p.layer_rules[1].to.as_str()), ("app::web", "app::db"));
    }

    #[test]
    fn scope_matches_by_segment_not_substring() {
        assert!(scope_matches("app::domain::handle", "domain"));
        assert!(scope_matches("domain::handle", "domain"));
        assert!(scope_matches("app::domain", "domain"));
        assert!(scope_matches("crate::domain_logic", "domain"));
        assert!(!scope_matches("app::subdomain::handle", "domain"));
        assert!(!scope_matches("app::not_my_domain::f", "domain"));
        // multi-segment: intermediates exact, last is a prefix, contiguous.
        assert!(scope_matches("crate::net::client::send", "net::client"));
        assert!(scope_matches("crate::net::client_pool::get", "net::client"));
        assert!(!scope_matches("crate::net::server::send", "net::client"));
        assert!(!scope_matches("crate::network::client::send", "net::client"));
        assert!(!scope_matches("crate::net::x::client", "net::client"));
        assert!(!scope_matches("net", "net::client"));
        // DOTTED names (JVM/Swift/TS reports `candor-query` consumes): a scope must match across `.` too,
        // else a scoped deny/pure rule is silently inert → whatif false-green (gate-evasion). Both a
        // `.`-written and a `::`-written scope must match a dotted name.
        assert!(scope_matches("com.acme.domain.Pricing.quote", "domain"));
        assert!(scope_matches("com.acme.domain.Pricing.quote", "acme.domain"));
        assert!(scope_matches("com.acme.domain.Pricing.quote", "acme::domain"));
        assert!(scope_matches("com.acme.infra.Net.fetch", "infra.Net"));
        assert!(!scope_matches("com.acme.subdomain.h", "domain"));
        assert!(!scope_matches("com.acme.domain.h", "infra"));
    }

    #[test]
    fn fs_path_covered_respects_boundaries() {
        assert!(fs_path_covered("/etc/app", "/etc/app"));
        assert!(fs_path_covered("/etc/app", "/etc/app/cfg.toml"));
        assert!(fs_path_covered("/etc/app/", "/etc/app/cfg"));
        assert!(!fs_path_covered("/etc/app", "/etc/apppwned"));
        assert!(!fs_path_covered("/etc/app", "/etc/application/x"));
        assert!(!fs_path_covered("/etc/app/cfg", "/etc/app"));
        assert!(!fs_path_covered("/etc/app", "/etc/app/../passwd"));
        assert!(fs_path_covered("/", "/etc/app/x"));
        assert!(!fs_path_covered("etc/app", "/etc/app/cfg"));
        assert!(!fs_path_covered("/etc/app", "etc/app/cfg"));
        assert!(fs_path_covered("etc/app", "etc/app/cfg"));
    }

    /// ⟨0.24⟩ THE PROVABLE-PURITY DISCLOSURE MUST ASK THE GATE WHAT "PASSES" MEANS — SPEC §6.2, and both
    /// directions in ONE test, because killing an over-charge is exactly where a silent under-report gets
    /// introduced and the fixture proving the fabrication is closed cannot show the reach closed with it.
    ///
    /// A hole is a function that PASSES its rule while `Unknown`. `unverified_hole_rule` used to compute
    /// PASSES from `r.effects` alone — the pre-⟨0.19⟩ question, asked after two rungs gave rules a
    /// NARROWING FILTER — so a rule the gate TOLERATES was read here as violated and the hole was DELETED
    /// from the disclosure. MEASURED 2026-07-28: `deny Unknown[reflect]` over an `indirect` hole → gate
    /// exit 0, `unverified` "every function in a pure/deny layer is PROVABLY clean ✓".
    ///
    /// ROW 1 (the fix) — the filter does NOT match, so the gate tolerates and this IS a hole.
    /// ROW 2 (the mirror) — the SAME rule, SAME function, filter spelled to MATCH: the gate fires, so it
    /// is a violation and NOT a hole. Without row 2 the fix is satisfied by a predicate that calls
    /// everything a hole, which is the mirror over-report of the thing being fixed.
    /// ROW 3 — the ⟨0.20⟩ `Net[dest…]` filter, the same shape on the other narrowing axis.
    /// ROW 4 — no filter at all: byte-identical to pre-⟨0.24⟩, which is what keeps conformance PARTs
    /// 12c/12d (four-way) from moving.
    #[test]
    fn a_narrowed_rule_the_gate_tolerates_is_a_hole_and_the_one_it_fires_on_is_not() {
        let effs = ["Unknown"];
        let indirect: BTreeSet<String> = ["indirect".to_string()].into_iter().collect();
        let hole = |src: &str, classes: Option<&BTreeSet<String>>, nets: &[String]| -> Option<String> {
            let p = parse_policy(src);
            unverified_hole_rule("app::go", &effs, classes, nets, &p.rules).map(|r| rule_and_upgrade(r).1)
        };

        // ROW 1 — tolerated by the gate (indirect ∉ {reflect}) ⇒ a hole, and the upgrade WIDENS the
        // filter rather than appending a second `Unknown`.
        assert_eq!(
            hole("deny Unknown[reflect]\n", Some(&indirect), &[]).as_deref(),
            Some("deny Unknown"),
            "a rule the gate TOLERATES leaves the function unproven — that is the disclosure's whole subject"
        );

        // ROW 2 — THE MIRROR. Same rule, same signature, filter spelled to match: the gate FIRES, so this
        // is a violation and the disclosure must stay silent about it.
        assert_eq!(
            hole("deny Unknown[indirect]\n", Some(&indirect), &[]),
            None,
            "a rule the gate FIRES on is a violation, not a hole — filter-awareness must not start \
             disclosing the gate's own findings back as unproven passes"
        );
        assert_eq!(hole("deny Unknown[dynamic]\n", Some(&indirect), &[]), None, "`dynamic` covers indirect");

        // ROW 3 — the ⟨0.20⟩ destination filter, both ways, on a fn carrying Net BESIDE its Unknown.
        let netfn = ["Net".to_string(), "Unknown".to_string()];
        let telemetry = vec!["known-telemetry".to_string()];
        let p = parse_policy("deny Net[unknown-host]\n");
        assert_eq!(
            unverified_hole_rule("app::go", &netfn, Some(&indirect), &telemetry, &p.rules)
                .map(|r| rule_and_upgrade(r).1)
                .as_deref(),
            Some("deny Net[unknown-host] Unknown"),
            "a `Net[dest…]` the fn's destinations do not match is tolerated, so the Unknown beside it is a hole"
        );
        let p = parse_policy("deny Net[known-telemetry]\n");
        assert_eq!(
            unverified_hole_rule("app::go", &netfn, Some(&indirect), &telemetry, &p.rules).map(|r| r.raw.clone()),
            None,
            "MIRROR: the matching destination filter FIRES, so it is a violation and not a hole"
        );

        // ROW 4 — UNFILTERED, unchanged. `deny Unknown` fires on every Unknown (never a hole); `pure` and
        // `deny Fs` pass a fn with no real effect (always a hole) — the forms PARTs 12c/12d pin four-way.
        assert_eq!(hole("deny Unknown\n", Some(&indirect), &[]), None);
        assert_eq!(hole("pure\n", Some(&indirect), &[]).as_deref(), Some("deny Unknown"));
        assert_eq!(hole("deny Net Db  domain\ndeny Fs\n", Some(&indirect), &[]).as_deref(), Some("deny Fs Unknown"));
        assert_eq!(
            hole("deny Net Db  go\n", Some(&indirect), &[]).as_deref(),
            Some("deny Db Net Unknown go"),
            "the sorted multi-effect upgrade PART 12c-deny pins in all four engines"
        );

        // A WITHHELD filter — no class set to read — counts as PASSING, so the hole is disclosed rather
        // than dropped. The gate withholds there too; between an advisory note that speaks and one that
        // goes quiet over a rule that never ran, only the first stays true.
        assert_eq!(hole("deny Unknown[reflect]\n", None, &[]).as_deref(), Some("deny Unknown"));
    }
}

// ── ⟨0.27⟩ SPEC §3.4 `engine` — the engine↔baseline coupling ─────────────────────────────────────
/// What an `engine` pin says about the build that is running. Data, not a print-and-exit, so every
/// branch is testable — including the two that MUST NOT change the exit code.
#[derive(Debug, PartialEq, Eq)]
pub enum PinVerdict {
    /// No pin, or a pin qualified for another implementation. Today's behaviour, exactly.
    Absent,
    Match,
    /// A different version — the engine↔baseline coupling is broken. Exit 2 (UNEVALUABLE, never 1).
    Mismatch,
    /// Present but unreadable (`engine latest`, a bare `engine`, trailing junk). Exit 2: a pin that
    /// cannot be read is a guard the operator believes is on. This is the one place §6.2's
    /// warn-and-skip posture INVERTS — skipping a key that ADDS something costs that key; skipping a
    /// PIN costs the guard.
    Malformed,
    /// Well-formed, and this build cannot state its own release. UNANSWERABLE — §3.1's rule applies:
    /// disclosed, never scored, INCLUDING as satisfied.
    Undetermined,
}

/// The pin that applies to `impl_name` — the qualified form wins over the unqualified one, and the
/// LAST occurrence wins (matching candor-java's map semantics). Two lines that DISAGREE about the same
/// key return a value that cannot parse, so they surface as [`PinVerdict::Malformed`] rather than one
/// silently discarding the other: two lines disagreeing about which engine to run is not a preference
/// to resolve, it is a question the config leaves unanswered.
pub fn engine_pin_for(text: &str, impl_name: &str) -> Option<String> {
    const IMPLS: [&str; 5] = ["java", "rust", "ts", "swift", "agents"];
    let (mut wild, mut qual): (Option<String>, Option<String>) = (None, None);
    let mut bad = false;
    for raw in text.lines() {
        let line = raw.split('#').next().unwrap_or("").trim();
        if line.is_empty() {
            continue;
        }
        let mut it = line.split_whitespace();
        if !it.next().is_some_and(|k| k.eq_ignore_ascii_case("engine")) {
            continue;
        }
        let rest: Vec<&str> = it.collect();
        let slot = |cur: &mut Option<String>, v: String| {
            if cur.as_ref().is_some_and(|p| *p != v) {
                *cur = Some(format!("{} / {v}", cur.as_ref().unwrap()));
            } else {
                *cur = Some(v);
            }
        };
        // A KNOWN QUALIFIER DECIDES OWNERSHIP BEFORE ARITY. Checking the one-token case first made `engine swift` a WILDCARD pin whose version is the literal "swift" -> MALFORMED -> exit 2 in every engine, so one operator forgetting a version on a qualified line killed the whole family. SPEC 3.4 says the skip is whole-line 'whatever follows it' -- and nothing following it is a case of that too.
        if let Some(head) = rest.first() {
            if IMPLS.contains(&head.to_ascii_lowercase().as_str()) {
                if head.eq_ignore_ascii_case(impl_name) {
                    if rest.len() == 2 { slot(&mut qual, rest[1].to_string()); } else { bad = true; }
                }
                continue;                                     // another impl's line, whatever follows it
            }
        }
        match rest.len() {
            0 => bad = true,                                  // a bare `engine` line
            1 => slot(&mut wild, rest[0].to_string()),        // engine <version>
            _ => bad = true,                                  // trailing junk / unknown qualifier
        }
    }
    if bad {
        return Some("<unreadable>".to_string());
    }
    // AN UNREADABLE UNQUALIFIED LINE IS NOT HIDDEN BY A QUALIFIED PIN. `qual ?? wild` returned the qua
    // lified value, so `engine garbage` beside a good qualified line passed SILENTLY here while candor-java ex
    // ited 2 — the exact mirror of the bug just fixed in java, four engines the other way. Unreadability is a property of the LINE; precedence only decides which VERSION applies.
    if let Some(w) = &wild {
        if normalize_version(w).is_none() { return Some(w.clone()); }
    }
    qual.or(wild)
}

/// [`PinVerdict`] for `pin` against `running`. Pure: no printing, no exit.
pub fn pin_verdict(pin: Option<&str>, running: &str) -> PinVerdict {
    let Some(pin) = pin else { return PinVerdict::Absent };
    let Some(want) = normalize_version(pin) else { return PinVerdict::Malformed };
    if running.trim().is_empty() || running == "unknown" {
        return PinVerdict::Undetermined;
    }
    if want == normalize_version(running).unwrap_or_else(|| running.trim().to_string()) {
        PinVerdict::Match
    } else {
        PinVerdict::Mismatch
    }
}

/// A pin token → its comparable form, or None when it is not a version at all. A leading `v` is
/// optional (the GitHub-tag `v0.27.0` and the crate `0.27.0` are the same pin) and a two-part `0.27`
/// means `0.27.0`. Anything else — `latest`, a branch name — is MALFORMED rather than a version that
/// can never match: the difference decides whether the operator reads "wrong version" or "that is not
/// a version".
fn normalize_version(raw: &str) -> Option<String> {
    let s = raw.trim().strip_prefix(['v', 'V']).unwrap_or_else(|| raw.trim());
    let parts: Vec<&str> = s.split('.').collect();
    if !(parts.len() == 2 || parts.len() == 3) || !parts.iter().all(|p| !p.is_empty() && p.bytes().all(|b| b.is_ascii_digit())) {
        return None;
    }
    Some(if parts.len() == 2 { format!("{s}.0") } else { s.to_string() })
}
