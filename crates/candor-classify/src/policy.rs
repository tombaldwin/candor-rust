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
use std::collections::BTreeSet;

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
                eprintln!("candor: `unknown-alias {name}` names unknown reason-class `{cn}` — skipped");
            }
        }
        if set.is_empty() {
            eprintln!("candor: ignoring `unknown-alias {name}` — no valid reason-class");
        } else {
            out.insert(name.to_string(), set);
        }
    }
    out
}

/// ⟨0.21⟩ Parse `net-partner <host>` lines from `.candor/config` (NET-DESTINATION-CLASS-DESIGN.md): the
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
    if let Ok(p) = std::env::var("CANDOR_CONFIG") {
        return std::fs::read_to_string(&p).ok();
    }
    let start = std::fs::canonicalize(start).unwrap_or_else(|_| start.to_path_buf());
    let mut cur = if start.is_dir() { Some(start.as_path()) } else { start.parent() };
    while let Some(d) = cur {
        let cand = d.join(".candor/config");
        if cand.is_file() {
            return std::fs::read_to_string(&cand).ok();
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

/// The rule kinds parsed from a CANDOR_POLICY file.
#[derive(Default, Debug)]
pub struct ParsedPolicy {
    pub rules: Vec<PolicyRule>,
    pub allow_rules: Vec<AllowRule>,
    pub layer_rules: Vec<LayerRule>,
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
pub fn rule_and_upgrade(r: &PolicyRule) -> (String, String) {
    let scope = r.scope.clone().unwrap_or_default();
    let suffix = if scope.is_empty() { String::new() } else { format!(" {scope}") };
    if r.effects.is_empty() {
        // `pure` forbids real effects but not Unknown; to REQUIRE provable purity, add a deny-Unknown.
        (format!("pure{suffix}"), format!("deny Unknown{suffix}"))
    } else {
        let effs = r.effects.iter().copied().collect::<Vec<_>>().join(" ");
        (format!("deny {effs}{suffix}"), format!("deny {effs} Unknown{suffix}"))
    }
}

/// The single predicate for a provable-purity hole (eval/fixloop/DISPATCH-NOTE.md): a function that is
/// `Unknown`, sits in a `pure`/`deny <E>` scope, and PASSES that rule (carries none of its forbidden real
/// effects) — so its compliance is asserted but not verified (the Unknown could hide the very effect the
/// rule forbids; the classic case is a fn/closure-injected port). A *real* violation is the gate's job, not
/// this. Returns the first governing rule under which the function is such a hole, or `None` if it is not
/// one. Shared by candor-scan's gate note and candor-query's `unverified` so "what a hole is" has ONE
/// definition — the two paths can never drift (conformance PART 12d pins their agreement).
pub fn unverified_hole_rule<'a, S: AsRef<str>>(
    name: &str,
    effects: &[S],
    rules: &'a [PolicyRule],
) -> Option<&'a PolicyRule> {
    if !effects.iter().any(|e| e.as_ref() == UNKNOWN) {
        return None;
    }
    rules.iter().find(|r| {
        // in the rule's scope (a scopeless rule governs the whole unit) …
        if let Some(s) = &r.scope {
            if !scope_matches(name, s) {
                return false;
            }
        }
        // … and PASSES it (no forbidden real effect — else it is a violation the gate already reports).
        let violates = if r.effects.is_empty() {
            effects.iter().any(|e| e.as_ref() != UNKNOWN)
        } else {
            effects.iter().any(|e| r.effects.contains(e.as_ref()))
        };
        !violates
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
/// Same as [`parse_policy`] but SILENT about malformed rules — for a SECOND, advisory re-parse within the
/// same run (candor-scan parses once for the gate check and again for the `unverified` disclosure), so the
/// CI log doesn't print every "ignoring policy rule …" warning twice (#21). The first parse already warned.
pub fn parse_policy_quiet(text: &str) -> ParsedPolicy {
    parse_policy_impl(text, false, &std::collections::BTreeMap::new())
}
fn parse_policy_impl(text: &str, warn: bool, aliases: &std::collections::BTreeMap<String, std::collections::BTreeSet<ReasonClass>>) -> ParsedPolicy {
    macro_rules! warn_ignore { ($($a:tt)*) => { if warn { eprintln!($($a)*); } } }
    let mut out = ParsedPolicy::default();
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
                    _ => {
                        warn_ignore!(
"candor: ignoring policy rule (allow supports only Net hosts / Llm hosts / Exec commands / Fs paths / Db tables): {line}"
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
                    warn_ignore!("candor: ignoring policy rule (allow {effect} names no values): {line}");
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
                                warn_ignore!("candor: policy rule names unknown Net destination-class `{cn}` (known: known-telemetry,known-partner,unknown-host, or *): {line}");
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
                            } else {
                                warn_ignore!("candor: policy rule names unknown reason-class/alias `{cn}` (known: reflect,dispatch,indirect,native,unresolved,setup; aliases: dynamic,*, or a config `unknown-alias`): {line}");
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
                    warn_ignore!("candor: ignoring policy rule (no known effect named): {line}");
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
                    warn_ignore!("candor: ignoring layering rule (want `forbid <scope> -> <scope>`): {line}");
                    continue;
                }
                out.layer_rules.push(LayerRule {
                    from: a.to_string(),
                    to: b.to_string(),
                    raw: line.to_string(),
                });
            }
            other => warn_ignore!("candor: ignoring policy rule (unknown kind `{other}`): {line}"),
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
}
