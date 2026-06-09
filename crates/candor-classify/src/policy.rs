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

/// One `allow <Effect> [in <scope>] <literal>…` rule (AS-EFF-008). The effect is one of the three
/// that carry a literal surface (`Net`/`Exec`/`Fs`); a function in `scope` performing it may reach
/// ONLY the listed literals. Matching is effect-specific (`literal_allowed`).
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

/// The hostname part of a `host[:port]` literal (everything before the first `:`). Host-allowlist
/// matching is by hostname so `api.stripe.com` in a rule accepts a reached `api.stripe.com:443`.
pub fn host_part(h: &str) -> &str {
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

/// Whether a reached literal is allowed under an effect-specific match (SPEC §6.2): `Net` host by
/// name (port ignored), `Exec` command by basename, `Fs` path by boundary-respecting prefix.
pub fn literal_allowed(effect: &str, reached: &str, allow: &BTreeSet<String>) -> bool {
    match effect {
        "Net" => allow.iter().any(|a| host_part(a) == host_part(reached)),
        "Exec" => allow.iter().any(|a| cmd_base(a) == cmd_base(reached)),
        "Fs" => allow.iter().any(|a| fs_path_covered(a, reached)),
        _ => allow.contains(reached),
    }
}

/// A policy scope matches a function name by **path segment** (SPEC §6.2), not substring: split both
/// on `::`; the scope matches a contiguous run of name-segments where every segment except the last
/// matches exactly and the last is a prefix. So `domain` matches `app::domain::h` and `domain_logic`
/// but not `subdomain`. (Used directly by the Rust gate; the JVM engine mirrors it over `.`.)
pub fn scope_matches(name: &str, scope: &str) -> bool {
    let segs: Vec<&str> = name.split("::").collect();
    let parts: Vec<&str> = scope.split("::").collect();
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
pub fn parse_policy(text: &str) -> ParsedPolicy {
    let mut out = ParsedPolicy::default();
    for raw_line in text.lines() {
        let line = raw_line.split('#').next().unwrap_or("").trim();
        if line.is_empty() {
            continue;
        }
        let mut toks = line.split_whitespace();
        match toks.next().unwrap_or("") {
            "allow" => {
                let effect = match toks.next().unwrap_or("") {
                    "Net" => "Net",
                    "Exec" => "Exec",
                    "Fs" => "Fs",
                    _ => {
                        eprintln!(
                            "candor: ignoring policy rule (allow supports only Net hosts / Exec commands / Fs paths): {line}"
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
             allow Db  whatever\n\
             allow Net in nohosts\n\
             allow\n",
        );
        assert_eq!(p.allow_rules.len(), 4);
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
