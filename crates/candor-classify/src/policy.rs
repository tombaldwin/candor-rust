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

/// One `deny <Effect…> [scope]` / `pure <scope>` rule (AS-EFF-006). `effects` empty ⇒ a `pure` rule
/// (ANY effect forbidden). `scope` is a path segment-scope the rule applies to (None = whole unit).
#[derive(Debug, Clone)]
pub struct PolicyRule {
    pub effects: BTreeSet<&'static str>,
    pub scope: Option<String>,
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
        "Net" => allow.iter().any(|a| host_part(a) == host_part(reached)),
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
                    "Exec" => "Exec",
                    "Fs" => "Fs",
                    "Db" => "Db",
                    _ => {
                        eprintln!(
                            "candor: ignoring policy rule (allow supports only Net hosts / Exec commands / Fs paths / Db tables): {line}"
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
                    eprintln!("candor: ignoring policy rule (allow {effect} names no values): {line}");
                    continue;
                }
                out.allow_rules.push(AllowRule { effect, scope, literals, raw: line.to_string() });
            }
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
                out.rules.push(PolicyRule { effects, scope, raw: line.to_string() });
            }
            "pure" => out.rules.push(PolicyRule {
                effects: BTreeSet::new(),
                scope: toks.next().map(str::to_string),
                raw: line.to_string(),
            }),
            "forbid" => {
                let a = toks.next().unwrap_or("");
                let arrow = toks.next().unwrap_or("");
                let b = toks.next().unwrap_or("");
                if a.is_empty() || arrow != "->" || b.is_empty() {
                    eprintln!("candor: ignoring layering rule (want `forbid <scope> -> <scope>`): {line}");
                    continue;
                }
                out.layer_rules.push(LayerRule {
                    from: a.to_string(),
                    to: b.to_string(),
                    raw: line.to_string(),
                });
            }
            other => eprintln!("candor: ignoring policy rule (unknown kind `{other}`): {line}"),
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
