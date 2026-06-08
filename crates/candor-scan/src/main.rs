//! candor-scan — a STABLE-Rust effect scanner. Produces the same candor report JSON the nightly
//! `rustc_private` lint does, but with a purely syntactic backend: walk the crate's `.rs` files, parse
//! with `syn`, build a name-resolved-enough call graph, classify path-qualified calls with the shared
//! `candor-classify`, and propagate transitively. No nightly, no `rustc-dev`, no dylint — `cargo install`
//! and run anywhere.
//!
//! HONEST PRECISION TRADE vs the lint. This is syntactic, so it sees what's written, not what's
//! resolved. It CATCHES: path-qualified effect calls (`std::fs::read`, `reqwest::Client::execute`,
//! `Command::new`), including `use`-aliased prefixes; and intra-crate calls (matched by name) for
//! transitive propagation. It MISSES (silently — it does NOT emit `Unknown`): effects reached only
//! through a method call whose receiver type isn't path-qualified, trait-object dispatch, closures /
//! fn-pointers, macros, and cross-crate propagation by stable identity. So on resolution-heavy code it
//! under-reports relative to the lint. Use the lint when you need the soundness contract; use this when
//! you need zero-friction, stable, installable triage. Shares the lint's classifier — one source of truth.
//!
//! Usage:  candor-scan [<crate-dir>] [--out <prefix>] [--json]
//!   default dir = ".", default prefix = "<dir>/.candor/report"; writes <prefix>.<crate>.scan.json (+ a
//!   callgraph sidecar so `cargo candor callers <fn>` works on the stable report too). `--json` prints
//!   the report to stdout instead.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::path::Path;

use candor_report::ReportEntry;
use syn::visit::Visit;

/// A call observed in a function body: the (use-expanded) path string and the leaf name.
struct Call {
    path: String,            // "std::fs::read", "compute_price", "pricing::priced"
    leaf: String,            // last segment
    str_arg: Option<String>, // first string-literal argument (host/cmd/path detail)
}

/// One function the scan found: its module-qualified name, where, and the calls in its body.
struct FnInfo {
    qual: String,
    leaf: String,
    loc: String,
    calls: Vec<Call>,
}

struct CallCollector<'a> {
    uses: &'a HashMap<String, String>,
    calls: Vec<Call>,
}

fn path_to_string(p: &syn::Path) -> String {
    p.segments.iter().map(|s| s.ident.to_string()).collect::<Vec<_>>().join("::")
}

/// Expand a call path against this file's `use` map: if the first segment is the last segment of some
/// `use a::b::Name`, replace it with the full `a::b::Name`. Turns `fs::read` → `std::fs::read`,
/// `Command::new` → `std::process::Command::new`. `crate`/`self`/`super` prefixes are stripped (local).
fn expand(path: &str, uses: &HashMap<String, String>) -> String {
    let mut segs: Vec<&str> = path.split("::").collect();
    while matches!(segs.first().copied(), Some("crate" | "self" | "super")) {
        segs.remove(0);
    }
    if segs.is_empty() {
        return path.to_string();
    }
    if let Some(full) = uses.get(segs[0]) {
        let rest = &segs[1..];
        return if rest.is_empty() { full.clone() } else { format!("{full}::{}", rest.join("::")) };
    }
    segs.join("::")
}

fn first_str_lit(args: &syn::punctuated::Punctuated<syn::Expr, syn::token::Comma>) -> Option<String> {
    for a in args {
        if let syn::Expr::Lit(syn::ExprLit { lit: syn::Lit::Str(s), .. }) = a {
            let v = s.value();
            if !v.trim().is_empty() {
                return Some(v);
            }
        }
    }
    None
}

impl<'a, 'ast> Visit<'ast> for CallCollector<'a> {
    fn visit_expr_call(&mut self, node: &'ast syn::ExprCall) {
        if let syn::Expr::Path(p) = &*node.func {
            let path = expand(&path_to_string(&p.path), self.uses);
            let leaf = path.rsplit("::").next().unwrap_or(&path).to_string();
            self.calls.push(Call { path, leaf, str_arg: first_str_lit(&node.args) });
        }
        syn::visit::visit_expr_call(self, node);
    }
    fn visit_expr_method_call(&mut self, node: &'ast syn::ExprMethodCall) {
        let leaf = node.method.to_string();
        self.calls.push(Call { path: leaf.clone(), leaf, str_arg: first_str_lit(&node.args) });
        syn::visit::visit_expr_method_call(self, node);
    }
}

fn scan_items(
    items: &[syn::Item],
    modpath: &str,
    file: &str,
    uses: &mut HashMap<String, String>,
    out: &mut Vec<FnInfo>,
) {
    for it in items {
        if let syn::Item::Use(u) = it {
            collect_use(&u.tree, String::new(), uses);
        }
    }
    let qual = |name: &str| if modpath.is_empty() { name.to_string() } else { format!("{modpath}::{name}") };
    for it in items {
        match it {
            syn::Item::Fn(f) => {
                let n = f.sig.ident.to_string();
                out.push(fninfo(&n, &qual(&n), file, &f.block, uses));
            }
            syn::Item::Impl(im) => {
                let tyname = impl_type_name(&im.self_ty);
                for ii in &im.items {
                    if let syn::ImplItem::Fn(m) = ii {
                        let n = m.sig.ident.to_string();
                        let q = match &tyname {
                            Some(t) => qual(&format!("{t}::{n}")),
                            None => qual(&n),
                        };
                        out.push(fninfo(&n, &q, file, &m.block, uses));
                    }
                }
            }
            syn::Item::Mod(m) => {
                if let Some((_, inner)) = &m.content {
                    let sub = qual(&m.ident.to_string());
                    let mut subuses = uses.clone();
                    scan_items(inner, &sub, file, &mut subuses, out);
                }
            }
            _ => {}
        }
    }
}

fn fninfo(leaf: &str, qual: &str, file: &str, block: &syn::Block, uses: &HashMap<String, String>) -> FnInfo {
    let mut c = CallCollector { uses, calls: Vec::new() };
    for stmt in &block.stmts {
        c.visit_stmt(stmt);
    }
    FnInfo { qual: qual.to_string(), leaf: leaf.to_string(), loc: file.to_string(), calls: c.calls }
}

fn impl_type_name(ty: &syn::Type) -> Option<String> {
    if let syn::Type::Path(p) = ty {
        return p.path.segments.last().map(|s| s.ident.to_string());
    }
    None
}

fn collect_use(tree: &syn::UseTree, prefix: String, out: &mut HashMap<String, String>) {
    let join = |p: &str, s: &str| if p.is_empty() { s.to_string() } else { format!("{p}::{s}") };
    match tree {
        syn::UseTree::Path(p) => collect_use(&p.tree, join(&prefix, &p.ident.to_string()), out),
        syn::UseTree::Name(n) => {
            out.insert(n.ident.to_string(), join(&prefix, &n.ident.to_string()));
        }
        syn::UseTree::Rename(r) => {
            out.insert(r.rename.to_string(), join(&prefix, &r.ident.to_string()));
        }
        syn::UseTree::Group(g) => {
            for t in &g.items {
                collect_use(t, prefix.clone(), out);
            }
        }
        syn::UseTree::Glob(_) => {}
    }
}

/// Module path implied by a file's location under `src/` (root files → ""; `foo.rs`/`foo/mod.rs` →
/// "foo"; `foo/bar.rs` → "foo::bar"). Best-effort mirror of file-based module resolution.
fn module_path(rel: &Path) -> String {
    let mut comps: Vec<String> =
        rel.components().filter_map(|c| c.as_os_str().to_str().map(String::from)).collect();
    if comps.first().map(String::as_str) == Some("src") {
        comps.remove(0);
    }
    if let Some(last) = comps.last_mut() {
        let stem = last.trim_end_matches(".rs").to_string();
        if stem == "lib" || stem == "main" || stem == "mod" {
            comps.pop();
        } else {
            *last = stem;
        }
    }
    comps.join("::")
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let mut dir = ".".to_string();
    let mut prefix = String::new();
    let mut want_json = false;
    let mut it = args.iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "--out" => prefix = it.next().cloned().unwrap_or_default(),
            "--json" => want_json = true,
            "-h" | "--help" => {
                eprintln!("candor-scan [<dir>] [--out <prefix>] [--json] — stable, syntactic effect report");
                return;
            }
            _ => dir = a.clone(),
        }
    }
    let root = Path::new(&dir);
    let crate_name = read_crate_name(root).unwrap_or_else(|| "crate".to_string());

    let mut fns: Vec<FnInfo> = Vec::new();
    for entry in walkdir::WalkDir::new(root).into_iter().filter_map(Result::ok) {
        let p = entry.path();
        if !p.is_file() || p.extension().and_then(|e| e.to_str()) != Some("rs") {
            continue;
        }
        if p.components().any(|c| matches!(c.as_os_str().to_str(), Some("target") | Some(".git"))) {
            continue;
        }
        let Ok(text) = std::fs::read_to_string(p) else { continue };
        let Ok(file) = syn::parse_file(&text) else { continue };
        let rel = p.strip_prefix(root).unwrap_or(p);
        let modpath = module_path(rel);
        let mut uses = HashMap::new();
        scan_items(&file.items, &modpath, &rel.to_string_lossy(), &mut uses, &mut fns);
    }

    let mut by_leaf: HashMap<String, Vec<String>> = HashMap::new();
    for f in &fns {
        by_leaf.entry(f.leaf.clone()).or_default().push(f.qual.clone());
    }

    let mut direct: HashMap<String, BTreeSet<&'static str>> = HashMap::new();
    let mut hosts: HashMap<String, BTreeSet<String>> = HashMap::new();
    let mut cmds: HashMap<String, BTreeSet<String>> = HashMap::new();
    let mut paths: HashMap<String, BTreeSet<String>> = HashMap::new();
    let mut calls: HashMap<String, BTreeSet<String>> = HashMap::new();
    let mut loc: HashMap<String, String> = HashMap::new();
    for f in &fns {
        loc.entry(f.qual.clone()).or_insert_with(|| f.loc.clone());
        for c in &f.calls {
            let cr = c.path.split("::").next().unwrap_or("");
            if let Some(eff) = candor_classify::classify(cr, &c.path) {
                direct.entry(f.qual.clone()).or_default().insert(eff);
                if let Some(s) = &c.str_arg {
                    match eff {
                        "Net" => { hosts.entry(f.qual.clone()).or_default().insert(host_part(s)); }
                        "Exec" => { cmds.entry(f.qual.clone()).or_default().insert(s.clone()); }
                        "Fs" => { paths.entry(f.qual.clone()).or_default().insert(s.clone()); }
                        _ => {}
                    }
                }
            }
            if !matches!(cr, "std" | "core" | "alloc") {
                if let Some(targets) = by_leaf.get(&c.leaf) {
                    for t in targets {
                        if t != &f.qual {
                            calls.entry(f.qual.clone()).or_default().insert(t.clone());
                        }
                    }
                }
            }
        }
    }

    let all: Vec<String> = fns.iter().map(|f| f.qual.clone()).collect();
    let inferred = propagate(&direct, &calls, &all);
    let hostsacc = propagate_str(&hosts, &calls, &all);
    let cmdsacc = propagate_str(&cmds, &calls, &all);
    let pathsacc = propagate_str(&paths, &calls, &all);

    let mut entries: Vec<ReportEntry> = Vec::new();
    let mut cg: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for q in &all {
        if let Some(cs) = calls.get(q) {
            cg.insert(q.clone(), cs.iter().cloned().collect());
        }
        let inf = inferred.get(q).cloned().unwrap_or_default();
        if inf.is_empty() {
            continue;
        }
        entries.push(ReportEntry {
            func: q.clone(),
            loc: loc.get(q).cloned().unwrap_or_default(),
            inferred: inf.iter().map(|s| s.to_string()).collect(),
            direct: direct.get(q).map(|d| d.iter().map(|s| s.to_string()).collect()).unwrap_or_default(),
            declared: Vec::new(),
            undeclared: Vec::new(),
            overdeclared: Vec::new(),
            unresolved: false,
            hash: String::new(),
            fs: Vec::new(),
            hosts: hostsacc.get(q).map(|s| s.iter().cloned().collect()).unwrap_or_default(),
            cmds: cmdsacc.get(q).map(|s| s.iter().cloned().collect()).unwrap_or_default(),
            paths: pathsacc.get(q).map(|s| s.iter().cloned().collect()).unwrap_or_default(),
            calls: calls.get(q).map(|cs| cs.iter().cloned().collect()).unwrap_or_default(),
        });
    }
    entries.sort_by(|a, b| a.func.cmp(&b.func));

    let meta = candor_report::ReportMeta {
        version: format!("scan-{}", env!("CARGO_PKG_VERSION")),
        toolchain: "stable".into(),
    };
    let body = candor_report::to_report_json(&meta, &entries).unwrap_or_default();
    if want_json {
        println!("{body}");
    } else {
        let prefix = if prefix.is_empty() { format!("{dir}/.candor/report") } else { prefix };
        if let Some(parent) = Path::new(&prefix).parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let file = format!("{prefix}.{crate_name}.scan.json");
        let _ = std::fs::write(&file, &body);
        let _ = std::fs::write(
            format!("{prefix}.{crate_name}.scan.callgraph.json"),
            serde_json::to_string(&cg).unwrap_or_default(),
        );
        eprintln!(
            "candor-scan: wrote {} effectful functions to {file} (stable syntactic backend — see --help)",
            entries.len()
        );
    }
}

fn host_part(h: &str) -> String {
    let a = h.split_once("://").map(|(_, r)| r).unwrap_or(h);
    let a = a.split('/').next().unwrap_or(a);
    a.rsplit_once('@').map(|(_, h)| h).unwrap_or(a).to_string()
}

fn read_crate_name(root: &Path) -> Option<String> {
    let txt = std::fs::read_to_string(root.join("Cargo.toml")).ok()?;
    for line in txt.lines() {
        let l = line.trim();
        if let Some(rest) = l.strip_prefix("name") {
            if let Some(v) = rest.split('=').nth(1) {
                return Some(v.trim().trim_matches('"').replace('-', "_"));
            }
        }
    }
    None
}

fn propagate(
    direct: &HashMap<String, BTreeSet<&'static str>>,
    calls: &HashMap<String, BTreeSet<String>>,
    all: &[String],
) -> HashMap<String, BTreeSet<&'static str>> {
    let mut acc = direct.clone();
    for f in all {
        acc.entry(f.clone()).or_default();
    }
    let mut changed = true;
    while changed {
        changed = false;
        for f in all {
            let add: BTreeSet<&'static str> = calls
                .get(f)
                .map(|cs| cs.iter().filter_map(|c| acc.get(c)).flatten().copied().collect())
                .unwrap_or_default();
            let e = acc.entry(f.clone()).or_default();
            let before = e.len();
            e.extend(add);
            if e.len() != before {
                changed = true;
            }
        }
    }
    acc
}

fn propagate_str(
    direct: &HashMap<String, BTreeSet<String>>,
    calls: &HashMap<String, BTreeSet<String>>,
    all: &[String],
) -> HashMap<String, BTreeSet<String>> {
    let mut acc = direct.clone();
    let mut changed = true;
    while changed {
        changed = false;
        for f in all {
            let add: BTreeSet<String> = calls
                .get(f)
                .map(|cs| cs.iter().filter_map(|c| acc.get(c)).flatten().cloned().collect())
                .unwrap_or_default();
            if add.is_empty() {
                continue;
            }
            let e = acc.entry(f.clone()).or_default();
            let before = e.len();
            e.extend(add);
            if e.len() != before {
                changed = true;
            }
        }
    }
    acc
}

#[cfg(test)]
mod tests {
    use super::*;

    fn uses(pairs: &[(&str, &str)]) -> HashMap<String, String> {
        pairs.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect()
    }

    #[test]
    fn expand_uses_the_use_map_and_strips_local_prefixes() {
        let u = uses(&[("fs", "std::fs"), ("Command", "std::process::Command")]);
        assert_eq!(expand("fs::read_to_string", &u), "std::fs::read_to_string");
        assert_eq!(expand("Command::new", &u), "std::process::Command::new");
        // crate/self/super are local and stripped, leaving the rest unresolved (matched by leaf later)
        assert_eq!(expand("crate::pricing::priced", &u), "pricing::priced");
        assert_eq!(expand("self::helper", &u), "helper");
        // an unknown first segment passes through unchanged
        assert_eq!(expand("foo::bar", &u), "foo::bar");
    }

    #[test]
    fn collect_use_expands_groups_and_renames() {
        let mut out = HashMap::new();
        // `use std::process::{Command, Stdio as Pipe};`
        let tree: syn::UseTree = syn::parse_str("std::process::{Command, Stdio as Pipe}").unwrap();
        collect_use(&tree, String::new(), &mut out);
        assert_eq!(out.get("Command").map(String::as_str), Some("std::process::Command"));
        assert_eq!(out.get("Pipe").map(String::as_str), Some("std::process::Stdio"));
    }

    #[test]
    fn module_path_mirrors_file_based_resolution() {
        assert_eq!(module_path(Path::new("src/lib.rs")), "");
        assert_eq!(module_path(Path::new("src/main.rs")), "");
        assert_eq!(module_path(Path::new("src/pricing.rs")), "pricing");
        assert_eq!(module_path(Path::new("src/billing/mod.rs")), "billing");
        assert_eq!(module_path(Path::new("src/billing/tax.rs")), "billing::tax");
    }

    #[test]
    fn host_part_strips_scheme_path_and_userinfo() {
        assert_eq!(host_part("https://api.stripe.com/v1/charges"), "api.stripe.com");
        assert_eq!(host_part("user:pass@db.internal:5432"), "db.internal:5432");
        assert_eq!(host_part("example.com"), "example.com");
    }

    #[test]
    fn propagate_is_transitive_across_the_call_graph() {
        // leaf has Fs directly; mid calls leaf; top calls mid — both must inherit Fs.
        let mut direct: HashMap<String, BTreeSet<&'static str>> = HashMap::new();
        direct.insert("leaf".into(), ["Fs"].into_iter().collect());
        let mut calls: HashMap<String, BTreeSet<String>> = HashMap::new();
        calls.insert("mid".into(), ["leaf".to_string()].into_iter().collect());
        calls.insert("top".into(), ["mid".to_string()].into_iter().collect());
        let all = vec!["leaf".to_string(), "mid".to_string(), "top".to_string(), "pure".to_string()];
        let acc = propagate(&direct, &calls, &all);
        assert!(acc["leaf"].contains("Fs"));
        assert!(acc["mid"].contains("Fs"));
        assert!(acc["top"].contains("Fs"));
        assert!(acc["pure"].is_empty());
    }

    #[test]
    fn classifier_resolves_a_std_fs_call() {
        // guards the shared-classifier contract the scanner relies on: an expanded std::fs path is Fs.
        assert_eq!(candor_classify::classify("std", "std::fs::read_to_string"), Some("Fs"));
        assert_eq!(candor_classify::classify("std", "std::process::Command::new"), Some("Exec"));
    }
}
