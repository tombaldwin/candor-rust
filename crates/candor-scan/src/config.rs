//! `.candor/config` (spec §3.4): checked-in configuration discovery for the scanner.


/// The shared §config key vocabulary; a key outside it WARNS (typo protection — a misspelt `policy`
/// must not silently drop the gate), a known-but-unimplemented key (this engine reads `policy` +
/// `baseline` + `deps`) is inert. Values: first token = key (ASCII-lowercased), rest of line =
/// value; `#` comments; blanks.
/// ⟨0.28 PROPOSED⟩ `engine` is in the vocabulary and NOT implemented here on purpose: candor-java enforces
/// the pin, and a key this spec defines must never be reported as an unknown one — that would tell an
/// operator their pin was ignored while a sibling engine was enforcing it.
pub(crate) const CONFIG_KEYS: [&str; 11] = ["policy", "baseline", "strict", "no-ambient", "closed-world", "taint", "deps", "unknown-alias", "net-partner", "unknown-ratchet", "engine"];

/// The subset of [`CONFIG_KEYS`] this engine actually wires to a mode. The rest are spec-inert here —
/// but a checked-in enforcement key that silently does nothing is a DECLARED-GATE-SILENTLY-OFF (the
/// reader believes the gate is on), so an inert recognized key warns loudly instead of staying mute.
/// `unknown-ratchet` is a boolean opt-in on the AS-EFF-005 baseline guard (a NEWLY-introduced Unknown
/// vs the baseline FAILS instead of staying advisory); its value threads through `flag`, not `cfg`.
pub(crate) const CONFIG_KEYS_IMPLEMENTED: [&str; 5] = ["policy", "baseline", "deps", "unknown-ratchet", "engine"];

/// Locate + parse `.candor/config` for the scan of `dir` (candor-spec §config): $CANDOR_CONFIG if set
/// (its path MUST be usable — exit 2 otherwise), else the nearest `.candor/config` walking UP from the
/// target, else the CWD's, else empty. A discovered-but-unreadable file also exits 2 (fail-closed).
/// WHICH config file this run reads, with NO side effects (SPEC §3.4).
///
/// Extracted so the §3.3.1 sink guard can ask the same question the loader answers instead of
/// re-deriving the walk. A review took the guard's own copy apart: it anchored relative values one
/// level too high whenever `CANDOR_CONFIG` pointed outside a `.candor/` directory, so it protected a
/// path the run never reads while the real policy went unguarded and was destroyed at exit 0.
pub(crate) fn discover_config_file(dir: &str) -> Option<std::path::PathBuf> {
    if let Ok(p) = std::env::var("CANDOR_CONFIG") {
        let pb = std::path::PathBuf::from(&p);
        return if pb.is_file() { Some(pb) } else { None };
    }
    let start = std::fs::canonicalize(dir).unwrap_or_else(|_| std::path::PathBuf::from(dir));
    let mut cur = if start.is_dir() { Some(start.as_path()) } else { start.parent() };
    while let Some(d) = cur {
        let cand = d.join(".candor/config");
        if cand.exists() {
            return Some(cand);
        }
        cur = d.parent();
    }
    let cwd = std::path::PathBuf::from(".candor/config");
    if cwd.exists() { Some(cwd) } else { None }
}

/// Every FILE a config names, already home-anchored — for the §3.3.1 sink guard.
///
/// Lenient by construction: it runs before the real config load and must not pre-empt its refusal, so
/// an unreadable config teaches it nothing rather than exiting. Shares `parse_config_text` with the
/// loader, which is the point — a second parser is a second set of holes.
pub(crate) fn config_inputs(dir: &str) -> Vec<(String, String)> {
    let mut out = Vec::new();
    let Some(file) = discover_config_file(dir) else { return out };
    out.push((file.display().to_string(), "the discovered .candor/config".to_string()));
    let Ok(text) = std::fs::read_to_string(&file) else { return out };
    let cfg = parse_config_text(&text, &file, false);
    for key in ["policy", "baseline"] {
        if let Some(v) = cfg.get(key) {
            if !v.is_empty() {
                out.push((v.clone(), format!("the config's `{key}`")));
            }
        }
    }
    if let Some(d) = cfg.get("deps") {
        for one in d.split(':').filter(|x| !x.is_empty()) {
            out.push((one.to_string(), "the config's `deps`".to_string()));
        }
    }
    out
}

pub(crate) fn load_candor_config(dir: &str) -> std::collections::HashMap<String, String> {
    let file: Option<std::path::PathBuf> = match std::env::var("CANDOR_CONFIG") {
        Ok(p) => {
            let pb = std::path::PathBuf::from(&p);
            if !pb.is_file() {
                eprintln!("candor-scan: CANDOR_CONFIG set but {p} is not a readable file — failing (exit 2)");
                std::process::exit(2);
            }
            Some(pb)
        }
        Err(_) => {
            let start = std::fs::canonicalize(dir).unwrap_or_else(|_| std::path::PathBuf::from(dir));
            let mut cur = if start.is_dir() { Some(start.as_path()) } else { start.parent() };
            let mut found = None;
            while let Some(d) = cur {
                let cand = d.join(".candor/config");
                if cand.exists() {
                    found = Some(cand);
                    break;
                }
                cur = d.parent();
            }
            found.or_else(|| {
                let cwd = std::path::PathBuf::from(".candor/config");
                if cwd.exists() { Some(cwd) } else { None }
            })
        }
    };
    let Some(file) = file else { return std::collections::HashMap::new() };
    let text = match std::fs::read_to_string(&file) {
        Ok(t) => t,
        Err(e) => {
            eprintln!("candor-scan: config {} exists but could not be read ({e}) — failing (exit 2)", file.display());
            std::process::exit(2);
        }
    };
    parse_config_text(&text, &file, true)
}

/// Parse a config's TEXT into its values, home-anchored. `disclose` controls only whether unknown and
/// unimplemented keys are announced — the VALUES are identical either way, which is what lets the sink
/// guard reuse it without doubling every diagnostic.
pub(crate) fn parse_config_text(
    text: &str,
    file: &std::path::Path,
    disclose: bool,
) -> std::collections::HashMap<String, String> {
    let mut cfg = std::collections::HashMap::new();
    for raw in text.lines() {
        let line = raw.split('#').next().unwrap_or("").trim();
        if line.is_empty() {
            continue;
        }
        let mut it = line.splitn(2, char::is_whitespace);
        let key = it.next().unwrap_or("").to_ascii_lowercase();
        let val = it.next().unwrap_or("").trim().to_string();
        if !CONFIG_KEYS.contains(&key.as_str()) {
            if disclose {
                eprintln!("candor-scan: ignoring unknown config key '{key}' in {}", file.display());
            }
            continue;
        }
        if key == "unknown-alias" || key == "net-partner" {
            // MULTI-VALUE keys, both extracted from the config TEXT rather than this single-value map.
            continue;
        }
        if !CONFIG_KEYS_IMPLEMENTED.contains(&key.as_str()) {
            if disclose {
                eprintln!(
                    "candor-scan: config key '{key}' is recognized by the candor family but not \
                     implemented by candor-scan — that gate/mode is NOT active on this scan \
                     (the nightly lint / another engine enforces it)"
                );
            }
            continue;
        }
        cfg.insert(key, val);
    }
    // SPEC §3.4: a RELATIVE path value resolves against the config's HOME directory — the directory
    // CONTAINING the `.candor/` dir — never the process CWD. An out-of-tree $CANDOR_CONFIG override
    // anchors to the file's own directory.
    let base = {
        let parent = file.parent().map(std::path::Path::to_path_buf).unwrap_or_default();
        if parent.file_name().and_then(|n| n.to_str()) == Some(".candor") {
            parent.parent().map(std::path::Path::to_path_buf).unwrap_or(parent)
        } else {
            parent
        }
    };
    let resolve = |v: &str| -> String {
        if v.is_empty() || std::path::Path::new(v).is_absolute() {
            v.to_string()
        } else {
            base.join(v).to_string_lossy().into_owned()
        }
    };
    if let Some(p) = cfg.get_mut("policy") {
        *p = resolve(p);
    }
    if let Some(b) = cfg.get_mut("baseline") {
        *b = resolve(b);
    }
    if let Some(d) = cfg.get_mut("deps") {
        // WHITESPACE, `:` OR `,` — SPEC §3.4's separator set, and what CANDOR_DEPS accepts. Splitting on
        // `:` alone left a space-separated list as ONE unresolvable token, so every dep after the first
        // was neither resolved here nor protected by the sink guard that reads this.
        *d = d
            .split([':', ',', ' ', '\t'])
            .filter(|x| !x.is_empty())
            .map(&resolve)
            .collect::<Vec<_>>()
            .join(":");
    }
    cfg
}

/// A boolean opt-in with env-override, mirroring candor-java's `Config.flag`: the env var's PRESENCE
/// means ON (an env var can't express OFF), else the config file's truthy value (`true`/`1`/`yes`, or a
/// bare key with no value). Default OFF when neither is set. Used for `unknown-ratchet` /
/// `CANDOR_UNKNOWN_RATCHET` — the AS-EFF-005 baseline-guard knob.
pub(crate) fn flag(cfg: &std::collections::HashMap<String, String>, key: &str, env_var: &str) -> bool {
    if std::env::var_os(env_var).is_some() {
        return true;
    }
    match cfg.get(key) {
        None => false,
        Some(v) => {
            let v = v.trim();
            v.is_empty() || v.eq_ignore_ascii_case("true") || v == "1" || v.eq_ignore_ascii_case("yes")
        }
    }
}

/// ⟨0.27⟩ SPEC §3.4 `engine` — THE ENGINE↔BASELINE COUPLING, enforced instead of hoped for.
///
/// The committed `baseline` is a snapshot of what one engine build reported, and an engine swap is
/// baseline-invalidating. What the pin adds over the provenance checks already in place is that it is
/// DECLARATIVE: a build id is a hash a consumer cannot write down, so the intended version lived in CI
/// configuration, decoupled from the baseline it is married to. A declared pin also tells tooling which
/// engine to FETCH, and it reaches a run with NO baseline configured at all.
///
/// TWO OF THE FIVE VERDICTS MUST NOT CHANGE THE EXIT CODE: an ABSENT pin (so the key is opt-in by
/// construction) and an UNDETERMINED one, where §3.1's unanswerable-condition rule applies — disclosed,
/// never scored, *including* as satisfied. Exit 2, never 1: the run is UNEVALUABLE, not violating.
pub(crate) fn enforce_engine_pin(dir: &str) {
    let Some(text) = candor_classify::policy::discover_config_text(std::path::Path::new(dir)) else { return };
    let pin = candor_classify::policy::engine_pin_for(&text, "rust");
    let running = env!("CARGO_PKG_VERSION");
    use candor_classify::policy::PinVerdict::*;
    match candor_classify::policy::pin_verdict(pin.as_deref(), running) {
        Absent | Match => {}
        Malformed => {
            eprintln!("candor-scan: .candor/config has an `engine` line that is not an engine version.");
            eprintln!("        want `engine <version>` (e.g. `engine v{running}`) or `engine <impl> <version>`");
            eprintln!("        (e.g. `engine rust v{running}`) for a repo scanned by more than one engine.");
            eprintln!("        Failing (exit 2) rather than ignoring it: a pin that cannot be read is a");
            eprintln!("        guard the operator believes is on.");
            // Through the standard refusal machinery, so the document reaches a `--gate-json -` STREAM
            // as well as a file. Arming only covers files (a stream has no stale document to replace),
            // so a refusal on the stream route wrote NOTHING — §3.1 says a refusal must still write one.
            crate::gate::record_gate_refusal(".candor/config has an `engine` line that is not an engine version");
            crate::gate::write_gate_json(2);
            std::process::exit(2);
        }
        Mismatch => {
            let p = pin.unwrap_or_default();
            eprintln!("candor-scan: .candor/config pins engine {p} but this build is candor-scan {running}.");
            eprintln!("        The pin and the committed baseline move together — a newer engine resolves more");
            eprintln!("        dispatch, so its report is not comparable with a baseline the pinned engine wrote.");
            eprintln!("        Either run the pinned engine, or update the pin and regenerate the baseline in the");
            eprintln!("        same change. Exit 2 (unevaluable), not 1 — this is not a policy violation.");
            crate::gate::record_gate_refusal(format!(
                ".candor/config pins engine {p} but this build is candor-scan {running}"));
            crate::gate::write_gate_json(2);
            std::process::exit(2);
        }
        Undetermined => {
            eprintln!("candor-scan: .candor/config pins an engine version, and this build does not know its own");
            eprintln!("        release, so the pin CANNOT be checked. Disclosed, not scored — neither passed nor");
            eprintln!("        failed.");
        }
    }
}
