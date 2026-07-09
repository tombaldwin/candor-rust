//! `.candor/config` (spec §3.4): checked-in configuration discovery for the scanner.


/// The shared §config key vocabulary; a key outside it WARNS (typo protection — a misspelt `policy`
/// must not silently drop the gate), a known-but-unimplemented key (this engine reads `policy` + `deps`)
/// is inert. Values: first token = key (ASCII-lowercased), rest of line = value; `#` comments; blanks.
pub(crate) const CONFIG_KEYS: [&str; 7] = ["policy", "baseline", "strict", "no-ambient", "closed-world", "taint", "deps"];

/// The subset of [`CONFIG_KEYS`] this engine actually wires to a mode. The rest are spec-inert here —
/// but a checked-in `baseline` key that silently does nothing is a DECLARED-GATE-SILENTLY-OFF (the
/// reader believes the ratchet is on), so an inert recognized key warns loudly instead of staying mute.
pub(crate) const CONFIG_KEYS_IMPLEMENTED: [&str; 2] = ["policy", "deps"];

/// Locate + parse `.candor/config` for the scan of `dir` (candor-spec §config): $CANDOR_CONFIG if set
/// (its path MUST be usable — exit 2 otherwise), else the nearest `.candor/config` walking UP from the
/// target, else the CWD's, else empty. A discovered-but-unreadable file also exits 2 (fail-closed).
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
            eprintln!("candor-scan: ignoring unknown config key '{key}' in {}", file.display());
            continue;
        }
        if !CONFIG_KEYS_IMPLEMENTED.contains(&key.as_str()) {
            eprintln!(
                "candor-scan: config key '{key}' is recognized by the candor family but not \
                 implemented by candor-scan — that gate/mode is NOT active on this scan \
                 (the nightly lint / another engine enforces it)"
            );
            continue;
        }
        cfg.insert(key, val);
    }
    // SPEC §3.4: a RELATIVE path value resolves against the config's HOME directory — the directory
    // CONTAINING the `.candor/` dir (the repo root the config travels with) — never the process CWD.
    // So `policy .candor/gate.pol` and a root-relative `policy arch.policy` both mean what the author
    // wrote. An out-of-tree $CANDOR_CONFIG override anchors to the file's own directory.
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
    if let Some(d) = cfg.get_mut("deps") {
        // CANDOR_DEPS is a `:`-separated list of files/directories; resolve each element.
        *d = d.split(':').map(&resolve).collect::<Vec<_>>().join(":");
    }
    cfg
}
