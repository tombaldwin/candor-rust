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
//! CALL RESOLUTION. The local call graph is name-resolved, not type-resolved. A qualified `Type::method`
//! call (or an associated-fn call `RequestBuilder::new()`) is matched on its 2-segment tail, but ONLY when
//! that tail is UNAMBIGUOUS, which keeps same-named methods on different types distinct. A bare
//! free-function call falls back to a unique leaf. A `.method()` call whose receiver type is inferred to a
//! LOCAL type resolves through that type's `Type::method` tail (so `x.go()` reaches a local `S::go`); an
//! external or un-inferrable receiver leaves the bare `.method()` with no definite target, so it
//! under-reports rather than guess (this is what stops `range.start()` — on the external `FloatRange` —
//! linking to a unique local `Clipboard::start`). We deliberately do NOT link a many-way-ambiguous name:
//! on a real crate that would link every `.new()` to all 100+ `*::new` defs and smear one type's effect
//! across the whole graph. Under-reporting an ambiguous edge is the honest failure
//! mode; fabricating one is never ok. The shared resolver is `resolve_target`.
//!
//! Usage:  candor-scan [<crate-dir>] [--out <prefix>] [--json] [--include-tests]
//!   default dir = ".", default prefix = "<dir>/.candor/report"; writes <prefix>.<crate>.scan.json (+ a
//!   callgraph sidecar so `cargo candor callers <fn>` works on the stable report too). `--json` prints
//!   the report to stdout instead. By DEFAULT only the crate's library/binary source is scanned —
//!   `tests/`, `benches/`, `examples/`, `test/`, the root `build.rs` (the Cargo build script — but NOT a
//!   `src/build.rs` source module), and `#[cfg(test)]` modules are skipped, so
//!   the report describes what the CRATE does, not what its harness does (`--include-tests` keeps them).
//!   See eval/calibration for accuracy on 35 real crates.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::path::Path;

use candor_report::ReportEntry;
use syn::visit::Visit;

/// A call observed in a function body: the (use-expanded) path string and the leaf name.
struct Call {
    path: String,            // "std::fs::read", "compute_price", "pricing::priced"
    leaf: String,            // last segment
    str_arg: Option<String>, // first string-literal argument (host/cmd/path detail)
    /// Synthesized from receiver-type inference (`reqwest::Client::send` from `client.send()`). Used for
    /// external-crate classification ONLY — excluded from local call-graph edges, since its `Type::method`
    /// tail could spuriously link to a same-named LOCAL method the call doesn't actually target.
    typed: bool,
    /// A METHOD call (`x.foo()`) vs a free-function/path call (`foo()`, `m::foo()`). When the receiver type
    /// can't be inferred, an unqualified method call has NO sound bare-leaf target — linking it to a
    /// same-named def would guess (`.bool()`→free `random::bool::bool`, `range.start()`→`Clipboard::start`),
    /// fabricating that def's effect. So such calls resolve to nothing; only the receiver-typed/qualified
    /// form (the `typed` call) links a method edge. Found on nushell (Rand/Clipboard on the random cmds).
    method: bool,
}

/// One function the scan found: its module-qualified name, where, and the calls in its body.
struct FnInfo {
    qual: String,
    leaf: String,
    loc: String,
    calls: Vec<Call>,
    /// The body invoked a callable the syntactic scan can't see through — a closure / fn-pointer value
    /// (`(cb)()`, `arr[i]()`, a local bound to a closure). The target could perform ANY effect, so the
    /// function can't honestly be certified pure: it's marked `Unknown` (matching the nightly lint's
    /// soundness fallback) rather than silently reported clean.
    unresolved: bool,
}

/// `struct-name-leaf -> { field -> expanded-type-path }`, e.g. `App -> { http: reqwest::Client }`.
/// Built crate-wide in a pre-pass so a method call on `self.http` can be resolved to its type and
/// classified by the existing per-crate method rules (`reqwest::Client::execute` -> Net).
type FieldIndex = HashMap<String, HashMap<String, String>>;

/// `fn-leaf -> expanded return-type-path`, e.g. `create_pool -> sqlx::Pool` (Result/Option unwrapped).
/// Lets type inference flow through a LOCAL factory function: `let p = create_pool()?; p.fetch_one(q)`.
/// Only UNAMBIGUOUS leaves are kept — a name with two different return types across the crate is dropped
/// (no guess), like the unique-leaf call-graph rule.
type ReturnIndex = HashMap<String, String>;

/// `trait leaf -> the local types that `impl Trait for Type` it` — the syntactic CHA universe for
/// dispatch-typed receivers (the JVM engine's bounded-CHA move, done on syntax). Keyed by leaf like
/// the other name indexes; includes impls of EXTERNAL traits for local types (the JVM resolves
/// interface impls the same way regardless of where the interface is declared).
type TraitImplIndex = HashMap<String, Vec<String>>;
/// `struct leaf -> field name -> trait bound leaves` for dispatch-typed FIELDS (`store: Box<dyn
/// Store>`) — the DI pattern `self.store.save()`, which `FieldIndex` can't carry (no concrete type).
type TraitFieldIndex = HashMap<String, HashMap<String, Vec<String>>>;

/// A locally-declared trait: how many declarations share the leaf (ambiguity check) and which
/// method names the declaration itself carries — CHA resolves ONLY calls to a declared method of
/// an unambiguous local trait (review found the wider rule fabricating: `impl Iterator for
/// RowIter` + `fn f(it: impl Iterator)` charged pure `f` with RowIter's Db).
#[derive(Default)]
struct LocalTrait {
    count: usize,
    methods: std::collections::HashSet<String>,
}

/// The trait indexes Pass A builds (impl universe, local declarations, dispatch-typed fields),
/// bundled so Pass B threads one handle instead of three more arguments.
#[derive(Clone, Copy)]
struct TraitIndexes<'a> {
    impls: &'a TraitImplIndex,
    decls: &'a HashMap<String, LocalTrait>,
    fields: &'a TraitFieldIndex,
}

struct CallCollector<'a> {
    uses: &'a HashMap<String, String>,
    /// local variable / param / `self` -> expanded type path, grown as `let`s are visited in order.
    vars: HashMap<String, String>,
    /// local variable / param -> trait bound leaves, for dispatch-typed receivers (`t: &dyn Store`,
    /// `s: impl Store`, `x: X` under `X: Store`). Disjoint from `vars` (no concrete type to put there).
    trait_vars: HashMap<String, Vec<String>>,
    fields: &'a FieldIndex,
    trait_fields: &'a TraitFieldIndex,
    /// trait leaf -> local impl types (None entries never exist; absent = no local impl).
    trait_impls: &'a TraitImplIndex,
    /// leaf -> the local trait declaration(s) sharing it: ambiguity count + declared method names.
    local_traits: &'a HashMap<String, LocalTrait>,
    returns: &'a ReturnIndex,
    calls: Vec<Call>,
    /// locals bound to a closure (`let f = |..| ..`), so a later `f()` is recognised as a closure
    /// invocation the scan can't see through — not a call to a free fn named `f`.
    closure_vars: std::collections::HashSet<String>,
    /// set once the body invokes a callable we can't resolve (see `FnInfo::unresolved`).
    unresolved: bool,
}

fn path_to_string(p: &syn::Path) -> String {
    p.segments.iter().map(|s| s.ident.to_string()).collect::<Vec<_>>().join("::")
}

/// A loaded sibling-report function: the effects + literal surfaces a consumer's call inherits.
#[derive(Clone, Default)]
struct DepFn {
    effects: Vec<&'static str>,
    hosts: Vec<String>,
    cmds: Vec<String>,
    paths: Vec<String>,
    tables: Vec<String>,
}

/// The CANDOR_DEPS index: `crate#tail2` and `crate#leaf` keys (UNAMBIGUOUS only — a key two dep
/// functions share is dropped, the same under-report-don't-guess rule as `resolve_target`), plus
/// the covered crate set. A report whose producing version differs from this binary's is
/// DOWNGRADED to `Unknown` rather than silently trusted (spec §2.1).
#[derive(Default)]
struct DepIndex {
    by_key: HashMap<String, DepFn>,
    crates: std::collections::HashSet<String>,
}

fn load_dep_reports(spec: Option<&str>) -> DepIndex {
    let mut idx = DepIndex::default();
    let Some(spec) = spec else { return idx };
    // Canonical-path dedup: the same report loaded twice would self-collide on every key and be
    // dropped as 'ambiguous', silently killing the chain (review: --deps + CANDOR_DEPS=.candor/deps
    // — the natural combination — did exactly that). Directories walk RECURSIVELY: --deps writes
    // one subdirectory per name@version.
    let mut files: Vec<std::path::PathBuf> = Vec::new();
    let mut seen_files: std::collections::HashSet<std::path::PathBuf> = std::collections::HashSet::new();
    let mut push_file = |f: std::path::PathBuf, files: &mut Vec<std::path::PathBuf>| {
        let canon = std::fs::canonicalize(&f).unwrap_or(f);
        if seen_files.insert(canon.clone()) {
            files.push(canon);
        }
    };
    for tok in spec.split(':').filter(|t| !t.is_empty()) {
        let p = Path::new(tok);
        if p.is_dir() {
            for e in walkdir::WalkDir::new(p).into_iter().filter_map(Result::ok) {
                let f = e.path();
                let name = f.file_name().and_then(|n| n.to_str()).unwrap_or("");
                if f.is_file() && name.ends_with(".json") && !name.contains("callgraph") {
                    push_file(f.to_path_buf(), &mut files);
                }
            }
        } else if p.is_file() {
            push_file(p.to_path_buf(), &mut files);
        } else {
            eprintln!("candor-scan: CANDOR_DEPS entry not found, skipped: {tok}");
        }
    }
    let my_version = format!("scan-{}", env!("CARGO_PKG_VERSION"));
    let mut ambiguous: std::collections::HashSet<String> = std::collections::HashSet::new();
    for f in &files {
        let Ok(text) = std::fs::read_to_string(f) else {
            eprintln!("candor-scan: CANDOR_DEPS report unreadable, skipped: {}", f.display());
            continue;
        };
        let Ok(v) = serde_json::from_str::<serde_json::Value>(&text) else {
            eprintln!("candor-scan: CANDOR_DEPS report unparsable, skipped: {}", f.display());
            continue;
        };
        // v0.2+ envelope or the v0.1 bare array; the producing version comes from the envelope.
        let version = v.pointer("/candor/version").and_then(|x| x.as_str()).unwrap_or("");
        let stale = version != my_version;
        let Some(fns) = v.get("functions").and_then(|x| x.as_array()).or_else(|| v.as_array()) else { continue };
        // The crate a report covers: the entries' `hash` prefix (`crate#qual`), else the filename
        // (`report.<crate>.scan.json`).
        let file_crate = f
            .file_name()
            .and_then(|n| n.to_str())
            .and_then(|n| n.strip_suffix(".scan.json"))
            .and_then(|n| n.rsplit('.').next())
            .map(str::to_string);
        // Register the crate at FILE level: an all-pure crate's report has zero entries, and that
        // emptiness is its honest claim — the crate is covered, not invisible.
        if let Some(c) = &file_crate {
            idx.crates.insert(c.clone());
        }
        for e in fns {
            let Some(qual) = e.get("fn").and_then(|x| x.as_str()) else { continue };
            let krate = e
                .get("hash")
                .and_then(|x| x.as_str())
                .and_then(|h| h.split_once('#'))
                .map(|(c, _)| c.to_string())
                .or_else(|| file_crate.clone());
            let Some(krate) = krate else { continue };
            idx.crates.insert(krate.clone());
            let mut de = DepFn::default();
            if stale {
                de.effects.push("Unknown"); // §2.1: a different producer version is not trusted
            } else {
                for s in e.get("inferred").and_then(|x| x.as_array()).into_iter().flatten() {
                    if let Some(s) = s.as_str() {
                        // unknown vocabulary (a future spec's effect) is honestly Unknown
                        de.effects.push(candor_classify::cap_from_name(s).unwrap_or("Unknown"));
                    }
                }
                let strs = |k: &str| -> Vec<String> {
                    e.get(k)
                        .and_then(|x| x.as_array())
                        .into_iter()
                        .flatten()
                        .filter_map(|s| s.as_str().map(str::to_string))
                        .collect()
                };
                de.hosts = strs("hosts");
                de.cmds = strs("cmds");
                de.paths = strs("paths");
                de.tables = strs("tables");
            }
            let mut keys = vec![format!("{krate}#{}", qual.rsplit("::").next().unwrap_or(qual))];
            if let Some(t2) = tail2(qual) {
                keys.push(format!("{krate}#{t2}"));
            }
            for k in keys {
                if ambiguous.contains(&k) {
                    continue;
                }
                if idx.by_key.contains_key(&k) {
                    idx.by_key.remove(&k); // two dep fns share the key — drop it, never guess
                    ambiguous.insert(k);
                } else {
                    idx.by_key.insert(k, de.clone());
                }
            }
        }
    }
    idx
}

// ── shared Cargo.toml line primitives (line-based on purpose — no toml dependency) ────────────────
// The ONE place table-header and scalar parsing live, so a manifest-syntax quirk (`[ spaced ]`
// headers, a trailing `# comment`) is handled once across the three readers below rather than
// drifting between them.

/// A `[section]` header line → its inner name, surrounding spaces tolerated (`[ workspace ]` →
/// "workspace"); None for any non-header line.
fn toml_section(line: &str) -> Option<&str> {
    let l = line.trim();
    Some(l.strip_prefix('[')?.strip_suffix(']')?.trim())
}

/// A scalar `key = "value"` / `key = value` on this line — `key` matched as the WHOLE key (then `=`),
/// the value quote-trimmed and an out-of-quotes trailing `# comment` stripped. None if not this key.
fn toml_scalar<'a>(line: &'a str, key: &str) -> Option<&'a str> {
    let rest = line.trim().strip_prefix(key)?.trim_start().strip_prefix('=')?.trim();
    Some(if let Some(q) = rest.strip_prefix('"') {
        q.split('"').next().unwrap_or(q)
    } else {
        rest.split('#').next().unwrap_or(rest).trim()
    })
}

/// Dependency names declared by EVERY Cargo.toml under the scan root (a workspace's members each
/// declare their own — review: reading only the root manifest left member-declared deps invisible
/// to the κ ledger on the most common project layout), normalized to crate-root form (`-` -> `_`).
/// dev-/build-dependencies are the harness's and the build script's universe, not the crate's
/// runtime one — excluded, like tests/ and build.rs.
fn cargo_deps(dir: &str) -> (std::collections::HashSet<String>, HashMap<String, String>) {
    let mut out = std::collections::HashSet::new();
    let mut renames = HashMap::new();
    // Honour the SAME nested-package rule as the source walk (filter_entry above): a subdir with its
    // own Cargo.toml is a different package whose deps are ITS universe, not this crate's — scan_target
    // scans it separately. Without this, a nested fixture/path-dep's deps polluted the parent's κ
    // ledger (the source walk skips the nested sources, so the two had drifted out of agreement).
    for entry in walkdir::WalkDir::new(dir)
        .into_iter()
        .filter_entry(|e| {
            if e.depth() == 0 || !e.file_type().is_dir() {
                return true;
            }
            let name = e.file_name().to_str().unwrap_or("");
            if name == "target" || (name.starts_with('.') && name != "." && name != "..") {
                return false;
            }
            !e.path().join("Cargo.toml").is_file()
        })
        .filter_map(Result::ok)
    {
        let p = entry.path();
        if p.file_name().and_then(|n| n.to_str()) != Some("Cargo.toml") {
            continue;
        }
        if let Ok(text) = std::fs::read_to_string(p) {
            cargo_toml_deps(&text, &mut out, &mut renames);
        }
    }
    (out, renames)
}

/// One manifest's dependency names, all four header forms: `[dependencies]` /
/// `[workspace.dependencies]` / `[target.….dependencies]` sections, and the table-header
/// declarations `[dependencies.name]` / `[target.….dependencies.name]` (review: the old
/// `ends_with("dependencies]")` gate made the header-form branch unreachable — a table-header
/// dep was invisible to the ledger, execution-verified).
fn cargo_toml_deps(
    text: &str,
    out: &mut std::collections::HashSet<String>,
    renames: &mut HashMap<String, String>,
) {
    // A dependency RENAME (`tui-common = { package = "tb-tui-common" }`) means the manifest KEY is
    // what the code imports while the registry/report knows the REAL package — without the map,
    // --deps scanned the real crate and the join/ledger missed it under the key (found live on
    // ebman: tui_common stayed "invisible" with its report sitting right there).
    // Match `package` only as a KEY (`{ … package = "real" }`), not as a substring of a dependency
    // KEY (`my-package = "1.2"` previously parsed its own version as a rename target) or a value:
    // `package` must sit at a token boundary and be followed by `=`.
    let pkg_re = |l: &str| -> Option<String> {
        let bytes = l.as_bytes();
        let mut search = 0;
        while let Some(rel) = l[search..].find("package") {
            let i = search + rel;
            let boundary = i == 0 || matches!(bytes[i - 1], b'{' | b',' | b' ' | b'\t');
            if boundary {
                if let Some(rest) = l[i + "package".len()..].trim_start().strip_prefix('=') {
                    if let Some(rest) = rest.trim_start().strip_prefix('"') {
                        return rest.split('"').next().map(|s| s.replace('-', "_"));
                    }
                }
            }
            search = i + "package".len();
        }
        None
    };
    let mut in_deps = false;
    let mut header_key: Option<String> = None; // the `[dependencies.name]` we're inside, if any
    for line in text.lines() {
        let l = line.trim();
        if let Some(inner) = toml_section(line) {
            let harness = inner.contains("dev-dependencies") || inner.contains("build-dependencies");
            in_deps = !harness && (inner == "dependencies" || inner.ends_with(".dependencies"));
            header_key = None;
            if !harness && !in_deps {
                let name = inner
                    .rfind(".dependencies.")
                    .map(|i| &inner[i + ".dependencies.".len()..])
                    .or_else(|| inner.strip_prefix("dependencies."));
                if let Some(name) = name {
                    if !name.is_empty() && !name.contains('.') {
                        let key = name.trim_matches('"').replace('-', "_");
                        out.insert(key.clone());
                        header_key = Some(key);
                    }
                }
            }
            continue;
        }
        if l.is_empty() || l.starts_with('#') {
            continue;
        }
        // inside a `[dependencies.name]` table: a `package = "real"` line is the rename
        if let Some(key) = &header_key {
            if l.starts_with("package") {
                if let Some(real) = pkg_re(l) {
                    renames.insert(key.clone(), real);
                }
            }
            continue;
        }
        if !in_deps {
            continue;
        }
        if let Some(name) = l.split('=').next() {
            let name = name.trim().trim_matches('"');
            if !name.is_empty() {
                let key = name.replace('-', "_");
                // A rename only appears in an INLINE TABLE value (`key = { … package = "real" }`),
                // never as a bare `package = "0.1"` (which is a dependency NAMED package) — so search
                // only inside the braces.
                if let Some(brace) = l.find('{') {
                    if let Some(real) = pkg_re(&l[brace..]) {
                        if real != key {
                            renames.insert(key.clone(), real);
                        }
                    }
                }
                out.insert(key);
            }
        }
    }
}

/// The trait leaves of a type-param-bound list (`T: Store + Send` -> ["Store", "Send"]). Marker
/// bounds need no filtering here: a leaf only ever matters if it later matches a local trait or a
/// local impl, and nobody locally declares `trait Send`.
fn bound_leaves(bounds: &syn::punctuated::Punctuated<syn::TypeParamBound, syn::Token![+]>) -> Vec<String> {
    bounds
        .iter()
        .filter_map(|b| match b {
            syn::TypeParamBound::Trait(t) => t.path.segments.last().map(|s| s.ident.to_string()),
            _ => None,
        })
        .collect()
}

/// The trait bound leaves of a DISPATCH-typed `syn::Type`: `&dyn T`, `impl T`, `Box<dyn T>` (and the
/// other single-arg smart pointers), or a bare generic param `X` declared `X: T`. Returns empty for
/// a concrete type — `type_path` owns those.
fn trait_leaves(ty: &syn::Type, generic_bounds: &HashMap<String, Vec<String>>) -> Vec<String> {
    match ty {
        syn::Type::Reference(r) => trait_leaves(&r.elem, generic_bounds),
        syn::Type::Paren(p) => trait_leaves(&p.elem, generic_bounds),
        syn::Type::Group(g) => trait_leaves(&g.elem, generic_bounds),
        syn::Type::TraitObject(t) => bound_leaves(&t.bounds),
        syn::Type::ImplTrait(t) => bound_leaves(&t.bounds),
        syn::Type::Path(p) => {
            if let Some(id) = p.path.get_ident() {
                return generic_bounds.get(&id.to_string()).cloned().unwrap_or_default();
            }
            // Box<dyn T> / Rc / Arc / RefCell / Mutex / RwLock — peel the wrapper, recurse on the arg.
            let Some(seg) = p.path.segments.last() else { return Vec::new() };
            let wrapper = matches!(seg.ident.to_string().as_str(), "Box" | "Rc" | "Arc" | "RefCell" | "Mutex" | "RwLock" | "Cell");
            if !wrapper {
                return Vec::new();
            }
            let syn::PathArguments::AngleBracketed(args) = &seg.arguments else { return Vec::new() };
            args.args
                .iter()
                .find_map(|a| match a {
                    syn::GenericArgument::Type(inner) => Some(trait_leaves(inner, generic_bounds)),
                    _ => None,
                })
                .unwrap_or_default()
        }
        _ => Vec::new(),
    }
}

/// `X -> [trait leaves]` for a signature's generic params, from both inline bounds (`fn f<X: Store>`)
/// and where-clauses (`where X: Store`).
fn generic_bounds_of(sig: &syn::Signature) -> HashMap<String, Vec<String>> {
    let mut m: HashMap<String, Vec<String>> = HashMap::new();
    for gp in &sig.generics.params {
        if let syn::GenericParam::Type(tp) = gp {
            let leaves = bound_leaves(&tp.bounds);
            if !leaves.is_empty() {
                m.entry(tp.ident.to_string()).or_default().extend(leaves);
            }
        }
    }
    if let Some(w) = &sig.generics.where_clause {
        for pred in &w.predicates {
            if let syn::WherePredicate::Type(pt) = pred {
                if let syn::Type::Path(p) = &pt.bounded_ty {
                    if let Some(id) = p.path.get_ident() {
                        let leaves = bound_leaves(&pt.bounds);
                        if !leaves.is_empty() {
                            m.entry(id.to_string()).or_default().extend(leaves);
                        }
                    }
                }
            }
        }
    }
    m
}

/// The (use-expanded) type path of a `syn::Type`, ignoring references and generic args:
/// `&reqwest::Client` -> `reqwest::Client`, `Pool<Postgres>` -> `sqlx::Pool` (via `uses`). `None` for
/// non-nameable types (impl Trait, tuples, …) where there's nothing to classify a method against.
fn type_path(ty: &syn::Type, uses: &HashMap<String, String>) -> Option<String> {
    match ty {
        syn::Type::Reference(r) => type_path(&r.elem, uses),
        syn::Type::Paren(p) => type_path(&p.elem, uses),
        syn::Type::Group(g) => type_path(&g.elem, uses),
        syn::Type::Path(p) => Some(expand(&path_to_string(&p.path), uses)),
        _ => None,
    }
}

/// Constructor-style associated function names: `let x = Foo::new(..)` (or `::connect().await?`) means
/// `x: Foo`. Conservative set of names that return `Self` (or `Result<Self>`), so the inferred type is
/// reliable. A non-constructor assoc call (`Foo::parse`) is NOT treated as producing a `Foo`.
fn is_ctor(name: &str) -> bool {
    matches!(
        name,
        "new" | "default" | "builder" | "with_capacity" | "connect" | "open" | "init" | "from"
            | "from_path" | "from_str" | "with_config" | "create"
    )
}

/// The type a call expression produces (peeling `&`/`(..)`/`?`/`.await`), by two routes:
///   1. a constructor `Path::ctor(..)` -> the `Path` type (`reqwest::Client::new()` -> `reqwest::Client`);
///   2. a LOCAL free function whose return type the pre-pass recorded (`create_pool()` -> `sqlx::Pool`).
/// Returns the expanded type path. `returns` is the crate-wide fn-leaf -> return-type index.
fn ctor_type(expr: &syn::Expr, uses: &HashMap<String, String>, returns: &ReturnIndex) -> Option<String> {
    match expr {
        syn::Expr::Reference(r) => ctor_type(&r.expr, uses, returns),
        syn::Expr::Paren(p) => ctor_type(&p.expr, uses, returns),
        syn::Expr::Try(t) => ctor_type(&t.expr, uses, returns),
        syn::Expr::Await(a) => ctor_type(&a.base, uses, returns),
        syn::Expr::Call(c) => {
            let syn::Expr::Path(p) = &*c.func else { return None };
            let full = path_to_string(&p.path);
            let leaf = full.rsplit("::").next().unwrap_or(&full);
            if let Some((ty, last)) = full.rsplit_once("::") {
                // `Type::ctor(..)` yields `Type` — but ONLY when the receiver is a TYPE, not a module.
                // Require the receiver's last segment to be type-like (UpperCamel), so `Client::new` →
                // Client but `serde_json::from_str` (module path) does NOT infer the module as a type.
                let ty_leaf = ty.rsplit("::").next().unwrap_or(ty);
                let type_like = ty_leaf.chars().next().is_some_and(|c| c.is_uppercase());
                if is_ctor(last) && type_like {
                    return Some(expand(ty, uses));
                }
            }
            // a local factory function call — its recorded (unambiguous) return type
            returns.get(leaf).cloned()
        }
        // `let s = S {..};` — a struct literal names its type directly.
        syn::Expr::Struct(s) => type_from_value_path(&path_to_string(&s.path), uses),
        // `let s = S;` — a UNIT-struct literal (or `let c = Color::Red;`, a unit enum variant, whose
        // value is typed as the ENUM). Gated by CamelCase so `let a = b;` (a variable copy) and
        // `let m = MAX_SIZE;` (a SCREAMING_SNAKE const) never mis-infer a type.
        syn::Expr::Path(p) => type_from_value_path(&path_to_string(&p.path), uses),
        _ => None,
    }
}

/// The type a VALUE path denotes, for `let` inference: `S` → `S`; `m::S` → `m::S`; `Color::Red`
/// (UpperCamel::UpperCamel = a unit enum variant) → `Color`. Only CamelCase leaves count as types —
/// a snake_case variable or SCREAMING_SNAKE const yields None (no inference; honest under-report).
fn type_from_value_path(full: &str, uses: &HashMap<String, String>) -> Option<String> {
    let camel = |s: &str| {
        // CamelCase = UpperCamel start, and either a single CHARACTER (`S`) or containing a lowercase
        // (distinguishes a type from a SCREAMING_SNAKE const). `chars().count()`, not `s.len()`: a
        // single-codepoint non-ASCII type ident (`struct É;`) is multi-BYTE and must still count as one
        // character. (/code-review.)
        let mut ch = s.chars();
        ch.next().is_some_and(|c| c.is_uppercase())
            && (s.chars().count() == 1 || s.chars().any(|c| c.is_lowercase()))
    };
    let segs: Vec<&str> = full.split("::").collect();
    let last = segs.last()?;
    if !camel(last) {
        return None;
    }
    // `Enum::Variant` — two trailing CamelCase segments: the VALUE's type is the enum (the penultimate).
    if segs.len() >= 2 && camel(segs[segs.len() - 2]) {
        return Some(expand(&segs[..segs.len() - 1].join("::"), uses));
    }
    Some(expand(full, uses))
}

/// Peel `Result<T, _>` / `Option<T>` / `io::Result<T>` to the inner `T` — a fallible constructor's
/// useful type is what it yields after `?`. Returns the inner type, or the type unchanged.
fn unwrap_result_option(ty: &syn::Type) -> &syn::Type {
    let syn::Type::Path(p) = ty else { return ty };
    let Some(seg) = p.path.segments.last() else { return ty };
    if matches!(seg.ident.to_string().as_str(), "Result" | "Option" | "IoResult") {
        if let syn::PathArguments::AngleBracketed(args) = &seg.arguments {
            if let Some(syn::GenericArgument::Type(inner)) = args.args.first() {
                return inner;
            }
        }
    }
    ty
}

/// Expand a call path against this file's `use` map: if the first segment is the last segment of some
/// `use a::b::Name`, replace it with the full `a::b::Name`. Turns `fs::read` → `std::fs::read`,
/// `Command::new` → `std::process::Command::new`. `crate`/`self`/`super` prefixes are stripped (local).
fn expand(path: &str, uses: &HashMap<String, String>) -> String {
    let mut segs: Vec<&str> = path.split("::").collect();
    // A path rooted at `crate`/`self`/`super` is EXPLICITLY crate-local — it is NOT subject to the file's
    // `use` aliases, so after stripping the prefix we return it as-is. (Re-applying `uses` here would let
    // a `use other::config;` import hijack a local `crate::config::load` call.)
    let rooted_local = matches!(segs.first().copied(), Some("crate" | "self" | "super"));
    while matches!(segs.first().copied(), Some("crate" | "self" | "super")) {
        segs.remove(0);
    }
    if segs.is_empty() {
        return path.to_string();
    }
    if !rooted_local {
        if let Some(full) = uses.get(segs[0]) {
            let rest = &segs[1..];
            return if rest.is_empty() { full.clone() } else { format!("{full}::{}", rest.join("::")) };
        }
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

impl<'a> CallCollector<'a> {
    /// Best-effort type of a method-call receiver, so `recv.method()` can be classified as
    /// `Type::method`. Resolves a bare variable/param/`self` (via `vars`), a `base.field` access (via
    /// the struct `FieldIndex`), and peels `&`/`(..)`/`?`/`.await`. For a method CHAIN
    /// (`client.get(url).send()`) it returns the BASE receiver's type — the chain stays within one
    /// crate's builder family, and the classifier verb-gates per crate, so attributing the terminal
    /// verb to the base type is correct in practice (`reqwest::Client` + `::send` -> Net).
    fn resolve_recv_type(&self, expr: &syn::Expr) -> Option<String> {
        match expr {
            syn::Expr::Reference(r) => self.resolve_recv_type(&r.expr),
            syn::Expr::Paren(p) => self.resolve_recv_type(&p.expr),
            syn::Expr::Group(g) => self.resolve_recv_type(&g.expr),
            syn::Expr::Try(t) => self.resolve_recv_type(&t.expr),
            syn::Expr::Await(a) => self.resolve_recv_type(&a.base),
            syn::Expr::MethodCall(m) => {
                // Walk through the chain to the base receiver's type. We deliberately do NOT consult the
                // return-type index by method NAME here: a method name doesn't identify the method, so a
                // single crate-wide `fn conn() -> redis::Connection` would otherwise hijack every
                // `x.conn().get()` on an unrelated `x`, fabricating a Db effect. The return index is used
                // only for free-function factory calls (the `Expr::Call` arm via `ctor_type`), where the
                // name does identify the function.
                self.resolve_recv_type(&m.receiver)
            }
            syn::Expr::Path(p) => {
                let name = p.path.get_ident()?.to_string();
                self.vars.get(&name).cloned()
            }
            syn::Expr::Field(f) => {
                let base = self.resolve_recv_type(&f.base)?;
                // Named field (`self.http`) or TUPLE field (`self.0`, incl. chained `self.0.0` via the
                // recursion — newtype wrappers; found by the PROVE-IT dogfood on ureq's
                // `ConfigBuilder(Scoped(..))`). Both index by the member's string form.
                let key = match &f.member {
                    syn::Member::Named(field) => field.to_string(),
                    syn::Member::Unnamed(idx) => idx.index.to_string(),
                };
                let base_leaf = base.rsplit("::").next().unwrap_or(&base);
                self.fields.get(base_leaf)?.get(&key).cloned()
            }
            syn::Expr::Call(_) => ctor_type(expr, self.uses, self.returns),
            _ => None,
        }
    }

    /// The trait bounds of a DISPATCH-typed receiver — a `&dyn T`/`impl T`/generic param (via
    /// `trait_vars`) or a trait-typed field (`self.store` where `store: Box<dyn Store>`, via
    /// `trait_fields`). Empty when the receiver has a concrete type (`resolve_recv_type` owns it)
    /// or can't be resolved at all.
    fn resolve_recv_traits(&self, expr: &syn::Expr) -> Vec<String> {
        // Hot-path guard: with no dispatch-typed vars or fields in scope (the overwhelmingly
        // common case), every lookup below is a guaranteed miss — skip the recursive walk.
        if self.trait_vars.is_empty() && self.trait_fields.is_empty() {
            return Vec::new();
        }
        match expr {
            syn::Expr::Reference(r) => self.resolve_recv_traits(&r.expr),
            syn::Expr::Paren(p) => self.resolve_recv_traits(&p.expr),
            syn::Expr::Group(g) => self.resolve_recv_traits(&g.expr),
            syn::Expr::Try(t) => self.resolve_recv_traits(&t.expr),
            syn::Expr::Await(a) => self.resolve_recv_traits(&a.base),
            syn::Expr::Path(p) => p
                .path
                .get_ident()
                .and_then(|id| self.trait_vars.get(&id.to_string()).cloned())
                .unwrap_or_default(),
            syn::Expr::Field(f) => {
                let Some(base) = self.resolve_recv_type(&f.base) else { return Vec::new() };
                let key = match &f.member {
                    syn::Member::Named(field) => field.to_string(),
                    syn::Member::Unnamed(idx) => idx.index.to_string(),
                };
                let base_leaf = base.rsplit("::").next().unwrap_or(&base);
                self.trait_fields.get(base_leaf).and_then(|m| m.get(&key).cloned()).unwrap_or_default()
            }
            _ => Vec::new(),
        }
    }
}

impl<'a, 'ast> Visit<'ast> for CallCollector<'a> {
    fn visit_expr_call(&mut self, node: &'ast syn::ExprCall) {
        // Peel `(..)`/`{..}` wrappers around the callee so `(f)()` is treated like `f()`.
        let mut func = &*node.func;
        loop {
            match func {
                syn::Expr::Paren(p) => func = &p.expr,
                syn::Expr::Group(g) => func = &g.expr,
                _ => break,
            }
        }
        match func {
            syn::Expr::Path(p) => {
                // A local bound to a closure — `let f = |..| ..` — has its body walked LEXICALLY by this
                // same visitor, so `f()` adds nothing and is NOT a blind spot. (Skip recording it as a
                // phantom call to a free fn `f`, too.) Any other path is a normal call. The
                // `!is_empty()` short-circuit avoids allocating the ident String on the common path
                // (most fns define no closures, so the set is empty).
                let is_closure_call = !self.closure_vars.is_empty()
                    && p.path.get_ident().is_some_and(|id| self.closure_vars.contains(&id.to_string()));
                if !is_closure_call {
                    let path = expand(&path_to_string(&p.path), self.uses);
                    let leaf = path.rsplit("::").next().unwrap_or(&path).to_string();
                    self.calls.push(Call { path, leaf, str_arg: first_str_lit(&node.args), typed: false, method: false });
                }
            }
            // The callee is a COMPUTED value, not a path or a visible local closure: `(self.handler)()`,
            // `arr[i]()`, `make_cb()()`. The scan can't identify the target or see its body — it could
            // perform any effect — so the enclosing function can't be certified pure: honest `Unknown`.
            _ => self.unresolved = true,
        }
        syn::visit::visit_expr_call(self, node);
    }
    fn visit_expr_method_call(&mut self, node: &'ast syn::ExprMethodCall) {
        let leaf = node.method.to_string();
        let str_arg = first_str_lit(&node.args);
        // Leaf-only call: feeds the intra-crate call graph and bare-leaf classification.
        self.calls.push(Call { path: leaf.clone(), leaf: leaf.clone(), str_arg: str_arg.clone(), typed: false, method: true });
        // Typed call: if the receiver's type resolves, form `Type::method` so the existing per-crate
        // method rules (reqwest/sqlx/redis/…) — unreachable from a bare method name — can fire. This is
        // the method-dispatch frontier: light, local type inference, no compiler.
        //
        // EXTERNAL types only. The external-crate rules are verb-precise (`ends_with("::execute")`), so
        // they're safe to apply to an inferred method call. The std rules are coarse PREFIX matches
        // (`std::fs::`, `std::process::Command`) written for free-function/constructor calls — applied to
        // arbitrary method calls they mis-fire on pure ones (`File::as_raw_fd`, `Command::arg`). So skip
        // std/core/alloc receivers: their free-function effects are already caught path-qualified, and an
        // honest miss on a std method beats a wrong effect on a pure one.
        if let Some(ty) = self.resolve_recv_type(&node.receiver) {
            let cr = ty.split("::").next().unwrap_or("");
            // EXCEPTION to the std exclusion: `std::path::Path`/`PathBuf` receivers route through —
            // the classifier has a VERB-PRECISE stat-family rule for them (metadata/read_dir/exists/…
            // → Fs; the pure join/file_name surface returns None), so the coarse-prefix mis-fire risk
            // doesn't apply. Without this an entire directory walker reads as pure (gix-dir: zero Fs).
            let std_path_recv = ty == "std::path::Path" || ty == "std::path::PathBuf";
            if !matches!(cr, "std" | "core" | "alloc") || std_path_recv {
                let path = format!("{ty}::{leaf}");
                self.calls.push(Call { path, leaf: leaf.clone(), str_arg, typed: true, method: true });
            }
        } else {
            // DISPATCH-typed receiver (`&dyn T` / `impl T` / `X: T` / a `Box<dyn T>` field): no
            // concrete type to classify against — previously a SILENT miss (the documented
            // trait-object hole). The JVM engine's bounded-CHA lesson, done on syntax, gated
            // THREE ways (each gate review-earned):
            //  - the trait must be LOCALLY DECLARED and unambiguous — resolving through local
            //    impls of an EXTERNAL trait fabricated effects onto pure generic fns
            //    (`impl Iterator for RowIter` + `fn f(it: impl Iterator)` charged f with
            //    RowIter's Db — execution-verified); external dispatch stays a documented miss;
            //  - the trait's own declaration must carry the called METHOD — a same-named method
            //    on a non-dispatching bound (`T: Store + Default` hitting a Default impl's
            //    `save`) is the same fabrication, and a supertrait call (`.clone()` on a bound
            //    param) must not flood Unknown;
            //  - the dispatch must be narrow (≤12 impls, the cross-engine bound) → edges to
            //    every local implementor; otherwise (or with no impl visible) honest `Unknown`.
            for tr in self.resolve_recv_traits(&node.receiver) {
                let Some(lt) = self.local_traits.get(&tr) else { continue }; // external: documented miss
                if !lt.methods.contains(&leaf) {
                    continue; // supertrait/blanket call — not this trait's dispatch
                }
                if lt.count > 1 {
                    self.unresolved = true; // ambiguous local leaf — never guess between traits
                    continue;
                }
                match self.trait_impls.get(&tr) {
                    Some(impls) if impls.len() <= 12 => {
                        for ty in impls {
                            self.calls.push(Call {
                                path: format!("{ty}::{leaf}"),
                                leaf: leaf.clone(),
                                str_arg: str_arg.clone(),
                                typed: true,
                                method: true,
                            });
                        }
                    }
                    _ => self.unresolved = true, // >12, or no impl visible: honest indeterminacy
                }
            }
        }
        syn::visit::visit_expr_method_call(self, node);
    }
    fn visit_local(&mut self, node: &'ast syn::Local) {
        // Record `let x: T = ..` (annotated) and `let x = T::new(..)` (constructor) so later method
        // calls on `x` resolve. Visited in source order, before any use of `x` (Rust requires it).
        if let syn::Pat::Type(pt) = &node.pat {
            if let syn::Pat::Ident(id) = &*pt.pat {
                // Dispatch-typing first (`let s: Box<dyn Store>` reads as concrete `Box` otherwise).
                let leaves = trait_leaves(&pt.ty, &HashMap::new());
                if !leaves.is_empty() {
                    self.vars.remove(&id.ident.to_string()); // a stale concrete binding must not shadow the rebind
                    self.trait_vars.insert(id.ident.to_string(), leaves);
                } else if let Some(ty) = type_path(&pt.ty, self.uses) {
                    self.vars.insert(id.ident.to_string(), ty);
                }
            }
        } else if let syn::Pat::Ident(id) = &node.pat {
            if let Some(init) = &node.init {
                if matches!(&*init.expr, syn::Expr::Closure(_)) {
                    // `let f = |..| ..` — remember `f` so a later `f()` is seen as a closure call.
                    self.closure_vars.insert(id.ident.to_string());
                } else {
                    // Rebinding the name to a NON-closure (a fn-pointer, a value) — drop any stale
                    // closure marking so a later `f()` isn't wrongly treated as a visible closure.
                    self.closure_vars.remove(&id.ident.to_string());
                    if let Some(ty) = ctor_type(&init.expr, self.uses, self.returns) {
                        self.vars.insert(id.ident.to_string(), ty);
                    }
                }
            }
        }
        syn::visit::visit_local(self, node);
    }
    fn visit_macro(&mut self, node: &'ast syn::Macro) {
        // syn does not parse a macro's body, so every call hidden inside one is invisible by default —
        // a real miss on crates that route effectful calls through a macro (git2 wraps EVERY libgit2 FFI
        // call in `try_call!(raw::git_...())`; `println!("{}", f())` hides `f`). Best-effort: parse the
        // token stream as comma-separated expressions and walk any that parse. If the body isn't
        // expression syntax (`quote!{}`, `matches!(x, Pat)`, macro_rules arms), parsing fails and we skip
        // — so this only ever ADDS visibility, never breaks. Owned exprs, so visit a local copy.
        let parser = syn::punctuated::Punctuated::<syn::Expr, syn::Token![,]>::parse_terminated;
        if let Ok(exprs) = syn::parse::Parser::parse2(parser, node.tokens.clone()) {
            for e in &exprs {
                self.visit_expr(e);
            }
        }
    }
}

/// True if the item carries any `#[cfg(...)]` attribute (conditionally compiled).
fn has_cfg(attrs: &[syn::Attribute]) -> bool {
    attrs.iter().any(|a| a.path().is_ident("cfg"))
}

/// True if a file stem names a conventional `#[cfg(test)] mod` FILE module (`tests.rs`, `foo_tests.rs`,
/// `foo_test.rs`) — whose test-ness is declared at the `mod` site, invisible when walking the file.
fn is_test_file_stem(stem: &str) -> bool {
    stem == "tests" || stem == "test" || stem.ends_with("_tests") || stem.ends_with("_test")
}

/// True if a crate-root-RELATIVE path is the Cargo BUILD SCRIPT — i.e. exactly `build.rs` at the root.
/// It runs at COMPILE time, never the crate's runtime behaviour, so it's skipped. A nested `src/build.rs`
/// is NOT the build script — it's an ordinary source module that merely shares the name (git2's
/// `src/build.rs` is `RepoBuilder`, the clone/fetch NETWORK surface) and must be scanned.
fn is_build_script(rel: &std::path::Path) -> bool {
    rel == std::path::Path::new("build.rs")
}

/// True if `test` is POSITIVELY required by this cfg predicate node (recursing through `any`/`all` to
/// any depth, but NOT through `not` — a `test` under `not()` means "compile when NOT testing", i.e.
/// production code that must NOT be skipped).
fn cfg_meta_requires_test(m: &syn::meta::ParseNestedMeta) -> bool {
    if m.path.is_ident("test") {
        return true;
    }
    if m.path.is_ident("any") || m.path.is_ident("all") {
        let mut inner_test = false;
        // (the parse may error on a non-meta tail like a bare `not(unix)` group; `test` is recorded
        // before that, and the error is swallowed — we only care whether a positive `test` was seen.)
        let _ = m.parse_nested_meta(|inner| {
            if cfg_meta_requires_test(&inner) {
                inner_test = true;
            }
            Ok(())
        });
        return inner_test;
    }
    false // `not(...)`, `feature = "..."`, target predicates, etc.
}

/// True if an item carries a `#[cfg(...)]` that POSITIVELY requires `test` — a test-only module the
/// default scan skips, since its effects describe the crate's TESTS, not the crate. `#[cfg(not(test))]`
/// (production code) and `#[cfg(all(unix, not(test)))]` are correctly NOT treated as test.
fn is_cfg_test(attrs: &[syn::Attribute]) -> bool {
    attrs.iter().any(|a| {
        a.path().is_ident("cfg") && {
            let mut found = false;
            let _ = a.parse_nested_meta(|m| {
                if cfg_meta_requires_test(&m) {
                    found = true;
                }
                Ok(())
            });
            found
        }
    })
}

#[allow(clippy::too_many_arguments)]
fn scan_items(
    items: &[syn::Item],
    modpath: &str,
    file: &str,
    include_tests: bool,
    fields: &FieldIndex,
    returns: &ReturnIndex,
    traits: TraitIndexes,
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
                out.push(fninfo(&n, &qual(&n), file, &f.sig, &f.block, None, uses, fields, returns, traits));
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
                        out.push(fninfo(&n, &q, file, &m.sig, &m.block, tyname.as_deref(), uses, fields, returns, traits));
                    }
                }
            }
            syn::Item::Mod(m) => {
                if !include_tests && is_cfg_test(&m.attrs) {
                    continue; // a #[cfg(test)] module — its effects are the tests', not the crate's
                }
                if let Some((_, inner)) = &m.content {
                    let sub = qual(&m.ident.to_string());
                    let mut subuses = uses.clone();
                    scan_items(inner, &sub, file, include_tests, fields, returns, traits, &mut subuses, out);
                }
            }
            _ => {}
        }
    }
}

/// Seed a function's variable→type map from its parameters (`fn h(c: &reqwest::Client)`) and, for an
/// impl method, `self` → the impl type. These are the most reliable type facts available syntactically.
fn seed_vars(sig: &syn::Signature, self_ty: Option<&str>, uses: &HashMap<String, String>) -> HashMap<String, String> {
    let mut vars = HashMap::new();
    if let Some(t) = self_ty {
        vars.insert("self".to_string(), t.to_string());
    }
    for arg in &sig.inputs {
        if let syn::FnArg::Typed(pt) = arg {
            if let syn::Pat::Ident(id) = &*pt.pat {
                if let Some(ty) = type_path(&pt.ty, uses) {
                    vars.insert(id.ident.to_string(), ty);
                }
            }
        }
    }
    vars
}

/// The dispatch-typed counterpart of `seed_vars`: params whose type is a trait bound rather than a
/// concrete path (`t: &dyn Store`, `s: impl Store`, `x: X` under `X: Store`) -> their bound leaves.
fn seed_trait_vars(sig: &syn::Signature) -> HashMap<String, Vec<String>> {
    let gb = generic_bounds_of(sig);
    let mut m = HashMap::new();
    for arg in &sig.inputs {
        if let syn::FnArg::Typed(pt) = arg {
            if let syn::Pat::Ident(id) = &*pt.pat {
                let leaves = trait_leaves(&pt.ty, &gb);
                if !leaves.is_empty() {
                    m.insert(id.ident.to_string(), leaves);
                }
            }
        }
    }
    m
}

#[allow(clippy::too_many_arguments)]
fn fninfo(
    leaf: &str,
    qual: &str,
    file: &str,
    sig: &syn::Signature,
    block: &syn::Block,
    self_ty: Option<&str>,
    uses: &HashMap<String, String>,
    fields: &FieldIndex,
    returns: &ReturnIndex,
    traits: TraitIndexes,
) -> FnInfo {
    // Function-LOCAL `use` statements (`fn f() { use rustix::time::clock_settime; … }`) are body
    // STATEMENTS, not module items, so the module-level use map misses them — every call they import then
    // fails to resolve to its crate and is under-reported (found on coreutils `date`: its rustix clock
    // read is imported by a fn-local `use`). Merge them in. (Top-level body stmts — the overwhelmingly
    // common placement; a `use` buried in a nested block is rare and left to the module fallback.)
    let mut local_uses = HashMap::new();
    for stmt in &block.stmts {
        if let syn::Stmt::Item(syn::Item::Use(u)) = stmt {
            collect_use(&u.tree, String::new(), &mut local_uses);
        }
    }
    let merged: HashMap<String, String>;
    let uses: &HashMap<String, String> = if local_uses.is_empty() {
        uses
    } else {
        let mut m = uses.clone();
        m.extend(local_uses);
        merged = m;
        &merged
    };
    // Dispatch-typing WINS where both could apply: `x: X` under `X: Store` also looks like a
    // concrete type `X` to `type_path` (and `Box<dyn Store>` looks like `Box`), which would shadow
    // the CHA route with a meaningless receiver type.
    let trait_vars = seed_trait_vars(sig);
    let mut vars = seed_vars(sig, self_ty, uses);
    for k in trait_vars.keys() {
        vars.remove(k);
    }
    let mut c = CallCollector {
        uses,
        vars,
        trait_vars,
        fields,
        trait_fields: traits.fields,
        trait_impls: traits.impls,
        local_traits: traits.decls,
        returns,
        calls: Vec::new(),
        closure_vars: std::collections::HashSet::new(),
        unresolved: false,
    };
    for stmt in &block.stmts {
        c.visit_stmt(stmt);
    }
    FnInfo {
        qual: qual.to_string(),
        leaf: leaf.to_string(),
        loc: file.to_string(),
        calls: c.calls,
        unresolved: c.unresolved,
    }
}

/// Record `fn-leaf -> return type` into `rets`, tracking ambiguity: a leaf seen with two different
/// return types is set to `None` (dropped later), so only UNAMBIGUOUS names survive. Result/Option are
/// unwrapped to the success type.
fn record_return(
    sig: &syn::Signature,
    uses: &HashMap<String, String>,
    rets: &mut HashMap<String, Option<String>>,
    self_ty: Option<&str>,
) {
    let syn::ReturnType::Type(_, ty) = &sig.output else { return };
    let Some(mut tp) = type_path(unwrap_result_option(ty), uses) else { return };
    // An impl method returning `Self` (`fn new_with_defaults() -> Self`) must index its IMPL type,
    // not the literal "Self": vars typed "Self" form `Self::method` calls that resolve to no local
    // def — so an ordinary `let agent = Agent::new_with_defaults(); agent.run(..)` silently dropped
    // its edge (found by the PROVE-IT dogfood on ureq: 3 public API entry points missing from a
    // blast radius). Worse, two same-named ctors on DIFFERENT types both recording "Self" defeated
    // the ambiguity check. `Result<Self>`/`Option<Self>` arrive here already unwrapped.
    if tp == "Self" {
        match self_ty {
            Some(s) => tp = s.to_string(),
            None => return, // `Self` outside an impl — nothing safe to record
        }
    }
    let leaf = sig.ident.to_string();
    match rets.get(&leaf) {
        None => {
            rets.insert(leaf, Some(tp));
        }
        Some(Some(prev)) if *prev != tp => {
            rets.insert(leaf, None); // conflicting return types — ambiguous, drop
        }
        _ => {}
    }
}

/// Pre-pass: index struct field types (`App -> { http: reqwest::Client }`) AND function return types
/// (`create_pool -> sqlx::Pool`), expanded via each module's `use` map. Recurses into modules like
/// `scan_items`. Field index keyed by struct leaf; return map keyed by fn leaf (ambiguous names dropped
/// by the caller). A name collision is rare and at worst yields a wrong (still verb-gated) classify.
#[allow(clippy::too_many_arguments)]
fn collect_decls(
    items: &[syn::Item],
    include_tests: bool,
    uses: &mut HashMap<String, String>,
    fields: &mut FieldIndex,
    rets: &mut HashMap<String, Option<String>>,
    trait_impls: &mut TraitImplIndex,
    local_traits: &mut HashMap<String, LocalTrait>,
    trait_fields: &mut TraitFieldIndex,
) {
    for it in items {
        if let syn::Item::Use(u) = it {
            collect_use(&u.tree, String::new(), uses);
        }
    }
    let no_generics = HashMap::new();
    for it in items {
        match it {
            syn::Item::Struct(s) => {
                match &s.fields {
                    syn::Fields::Named(named) => {
                        let entry = fields.entry(s.ident.to_string()).or_default();
                        for f in &named.named {
                            // Skip `#[cfg(...)]`-gated fields: they aren't unconditionally present, so
                            // inferring effects through them mis-fires. (tokio's `resource_span:
                            // tracing::Span`, gated on the off-by-default `tracing` feature, otherwise made
                            // every `self.resource_span.in_scope(..)` read as Log — 452 phantom functions.)
                            if has_cfg(&f.attrs) {
                                continue;
                            }
                            if let Some(name) = &f.ident {
                                // Dispatch-typing first: `store: Box<dyn Store>` reads as concrete
                                // `Box` to `type_path`, which would shadow the CHA route.
                                let leaves = trait_leaves(&f.ty, &no_generics);
                                if !leaves.is_empty() {
                                    trait_fields
                                        .entry(s.ident.to_string())
                                        .or_default()
                                        .insert(name.to_string(), leaves);
                                } else if let Some(ty) = type_path(&f.ty, uses) {
                                    entry.insert(name.to_string(), ty);
                                }
                            }
                        }
                    }
                    // TUPLE structs index by position (`"0"`, `"1"`), so a newtype-wrapped receiver
                    // (`self.0.run()`, chained `self.0.0`) resolves like a named field. Same
                    // `#[cfg]` rule.
                    syn::Fields::Unnamed(unnamed) => {
                        let entry = fields.entry(s.ident.to_string()).or_default();
                        for (i, f) in unnamed.unnamed.iter().enumerate() {
                            if has_cfg(&f.attrs) {
                                continue;
                            }
                            if let Some(ty) = type_path(&f.ty, uses) {
                                entry.insert(i.to_string(), ty);
                            }
                        }
                    }
                    syn::Fields::Unit => {}
                }
            }
            syn::Item::Fn(f) => record_return(&f.sig, uses, rets, None),
            syn::Item::Trait(t) => {
                let e = local_traits.entry(t.ident.to_string()).or_default();
                e.count += 1;
                for ti in &t.items {
                    if let syn::TraitItem::Fn(m) = ti {
                        e.methods.insert(m.sig.ident.to_string());
                    }
                }
            }
            syn::Item::Impl(im) => {
                let self_ty = impl_type_name(&im.self_ty);
                // `impl Trait for Type` — a CHA edge from the trait leaf to the implementing type.
                if let (Some((_, tr, _)), Some(ty)) = (&im.trait_, &self_ty) {
                    if let Some(leaf) = tr.segments.last() {
                        trait_impls.entry(leaf.ident.to_string()).or_default().push(ty.clone());
                    }
                }
                for ii in &im.items {
                    if let syn::ImplItem::Fn(m) = ii {
                        record_return(&m.sig, uses, rets, self_ty.as_deref());
                    }
                }
            }
            syn::Item::Mod(m) => {
                // Skip `#[cfg(test)]` modules here too (Pass B / scan_items already does): otherwise a
                // test module's struct fields and fn return types leak into the crate-wide index and get
                // used to type PRODUCTION code (e.g. `struct App { http: MockClient }` in `mod tests`
                // colliding with the real App).
                if !include_tests && is_cfg_test(&m.attrs) {
                    continue;
                }
                if let Some((_, inner)) = &m.content {
                    let mut subuses = uses.clone();
                    collect_decls(inner, include_tests, &mut subuses, fields, rets, trait_impls, local_traits, trait_fields);
                }
            }
            _ => {}
        }
    }
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
            let id = n.ident.to_string();
            if id == "self" {
                // `use a::b::{self, ..}` imports the MODULE `b` itself under name `b` → map `b -> a::b`
                // so a later `b::func()` resolves. Without this, `self` was mapped uselessly as
                // `b::self` and the module alias was lost. (Found on coreutils `ls`: `use std::fs::{self,
                // Metadata}` then `fs::read_dir` was unresolved → a file lister reporting ZERO Fs.)
                if let Some(last) = prefix.rsplit("::").next() {
                    out.insert(last.to_string(), prefix.clone());
                }
            } else {
                out.insert(id.clone(), join(&prefix, &id));
            }
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
    // Anchor at the LAST `src/` component, not just a leading one. A workspace member's code lives at
    // `crates/<name>/src/…`, so the module path is what FOLLOWS that `src` — otherwise the filesystem path
    // from the scan root mangles `crates/cli/src/decompress.rs` into `crates::cli::src::decompress`, which
    // ALSO breaks intra-crate call resolution (call sites use the real module path, not the dir path).
    // Found scanning ripgrep's workspace root — every name came out `crates::…::src::…` and `main` was lost.
    if let Some(i) = comps.iter().rposition(|c| c == "src") {
        comps.drain(..=i);
    }
    if let Some(last) = comps.last() {
        let stem = last.trim_end_matches(".rs").to_string();
        if stem == "lib" || stem == "main" || stem == "mod" {
            comps.pop();
        } else {
            // A dotted file stem encodes a NESTED module path — the tonic / prost gRPC convention names
            // a file `envoy.service.accesslog.v3.rs` for the module `envoy::service::accesslog::v3`. Split
            // on `.` so the qualified name is `::`-separated, not one ugly dotted segment.
            let parts: Vec<String> = stem.split('.').map(String::from).collect();
            comps.pop();
            comps.extend(parts);
        }
    }
    comps.join("::")
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let mut dir = ".".to_string();
    let mut prefix = String::new();
    let mut want_json = false;
    let mut include_tests = false;
    let mut policy_path: Option<String> = None;
    let mut deps_mode = false;
    let mut it = args.iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "--out" => prefix = it.next().cloned().unwrap_or_default(),
            "--json" => want_json = true,
            "--include-tests" => include_tests = true,
            "--policy" => policy_path = it.next().cloned(),
            "--deps" => deps_mode = true,
            "-V" | "--version" => {
                println!("candor-scan {}", env!("CARGO_PKG_VERSION"));
                return;
            }
            // The agent contract for THE INSTALLED VERSION, embedded at build time — doc and
            // binary cannot drift (the §2.1 version-trust rule applied to documentation). Agents
            // are told to run this instead of trusting a vendored/remote copy.
            "--agents" => {
                println!("<!-- candor-scan {} · the agent contract for this installed version -->", env!("CARGO_PKG_VERSION"));
                print!("{}", include_str!("../AGENTS.md"));
                return;
            }
            "-h" | "--help" => {
                println!("candor-scan {} — stable-Rust effect scanner (no nightly)", env!("CARGO_PKG_VERSION"));
                println!();
                println!("USAGE:  candor-scan [<dir>] [--out <prefix>] [--json] [--include-tests] [--policy <file>]");
                println!();
                println!("  <dir>             crate root to scan (default: .). A [workspace] root scans");
                println!("                    every member: one report per member under the one prefix.");
                println!("                    A nested dir with its own Cargo.toml is a different package");
                println!("                    and is never folded into the parent's report.");
                println!("  --out <prefix>    report path prefix (default: <dir>/.candor/report);");
                println!("                    writes <prefix>.<crate>.scan.json + a call-graph sidecar");
                println!("  --json            print the report to stdout instead of writing files");
                println!("  --include-tests   also scan tests/ benches/ examples/ and #[cfg(test)] modules");
                println!("                    (off by default → the report describes the crate, not its harness)");
                println!("  --deps            scan the Cargo.lock dependency tree first (registry sources from");
                println!("                    ~/.cargo/registry/src) into <dir>/.candor/deps/, then scan <dir>");
                println!("                    CHAINED over those reports — effects cross every crate boundary");
                println!("                    without κ needing to know the crates.");
                println!("  --policy <file>   enforce a CANDOR_POLICY file (deny/pure/allow/forbid, spec §6.2)");
                println!("                    over this scan; exit 1 on violation. ADVISORY FLOOR: the syntactic");
                println!("                    backend under-reports, so a miss can pass — the nightly engine is");
                println!("                    the sound gate. (CANDOR_POLICY env is honoured when flag absent.)");
                println!();
                println!("  CANDOR_DEPS=<p:…> chain sibling reports (files or directories of *.json): an");
                println!("                    unclassified call into a crate a report covers inherits that");
                println!("                    function's effects + literal surfaces (spec §2). Scan the dep");
                println!("                    once, chain it everywhere; the κ ledger names what to scan next.");
                println!("  -V, --version     print version");
                println!();
                println!("Syntactic, so it under-reports vs the full candor nightly lint (no Unknown). It never");
                println!("fabricates an effect. See https://github.com/tombaldwin/candor");
                return;
            }
            other => {
                // An unknown flag must FAIL, not become a path: an agent following a newer doc
                // against an older binary ran `candor-scan --agents` and scanned a directory
                // literally named `--agents`; a typo'd `--polcy` would silently drop the gate.
                if other.starts_with('-') {
                    eprintln!("candor-scan: unknown flag '{other}' (see --help)");
                    std::process::exit(2);
                }
                dir = a.clone();
            }
        }
    }
    // The policy source is resolved HERE, once (flag wins, CANDOR_POLICY env as fallback) — never
    // inside scan_one, so --deps dependency scans can't inherit the root gate via the env.
    let policy = policy_path.or_else(|| std::env::var("CANDOR_POLICY").ok());
    if deps_mode {
        std::process::exit(run_with_deps(&dir, prefix, want_json, include_tests, policy));
    }
    // Cross-crate report chaining (spec §2): CANDOR_DEPS names sibling reports (a `:`-separated
    // list of files and/or directories of *.json); an unclassified qualified call into a crate one
    // of them covers inherits that function's recorded effects + literal surfaces. The stable
    // scanner's half of the dep-scan story: scan the dep once, chain it everywhere.
    let deps_idx = load_dep_reports(std::env::var("CANDOR_DEPS").ok().as_deref());
    // scan_target handles both a single crate and a `[workspace]` root (one report per member under
    // one prefix — candor-query's multi-crate merge consumes them together; the policy gates each).
    std::process::exit(scan_target(&dir, prefix, want_json, include_tests, policy, &deps_idx));
}

/// Options for one crate scan. `policy` is RESOLVED by the caller (flag or CANDOR_POLICY env) —
/// scan_one itself never reads the env, so dependency scans under --deps can genuinely run
/// gate-free (review: the env fallback inside scan_one ran the root policy 328 times against
/// dependency internals). `quiet` suppresses the per-scan receipts (dep scans; the --deps summary
/// line speaks for them).
struct ScanOpts<'a> {
    prefix: String,
    want_json: bool,
    include_tests: bool,
    policy: Option<String>,
    quiet: bool,
    deps_idx: &'a DepIndex,
}

/// One crate scan, end to end (parse -> passes -> report -> receipt -> policy gate). Returns the
/// process exit code. Factored out of `main` so `--deps` can scan a dependency tree IN-PROCESS —
/// candor-scan's own self-gate (`deny Exec`) rightly forbids the spawn-yourself shortcut.
fn scan_one(dir: &str, opts: ScanOpts) -> (i32, Option<String>) {
    let ScanOpts { prefix, want_json, include_tests, policy: policy_path, quiet, deps_idx } = opts;
    let root = Path::new(dir);
    let crate_name = read_crate_name(root).unwrap_or_else(|| "crate".to_string());

    // Parse every in-scope .rs file ONCE (syn parses are reused across both passes below).
    let mut parsed: Vec<(String, syn::File)> = Vec::new();
    for entry in walkdir::WalkDir::new(root)
        .into_iter()
        // A nested directory carrying its own Cargo.toml is a DIFFERENT package (Cargo's own
        // semantics) — folding its files into this crate collides same-named fns across packages
        // and cross-wires the merged call graph (the repo-root self-scan merged 194 eval-fixture
        // `main`s into one unit). It gets its own scan: workspace member, --deps, or directly.
        .filter_entry(|e| {
            if e.depth() == 0 || !e.file_type().is_dir() {
                return true;
            }
            // Prune build/tooling dirs by NAME first — cheap, and it skips DESCENT into huge `target/`
            // and `.git/` trees (the dominant cost on a warm checkout) before the per-dir Cargo.toml
            // stat. A name starting with `.` is a hidden tooling dir (`.git`/`.github`/`.cargo`/…).
            let name = e.file_name().to_str().unwrap_or("");
            if name == "target" || (name.starts_with('.') && name != "." && name != "..") {
                return false;
            }
            // A nested dir carrying its own Cargo.toml is a DIFFERENT package (Cargo's own semantics):
            // folding its files into this crate collides same-named fns across packages and cross-wires
            // the merged call graph. It gets its own scan (workspace member, --deps, or directly).
            !e.path().join("Cargo.toml").is_file()
        })
        .filter_map(Result::ok)
    {
        let p = entry.path();
        if !p.is_file() || p.extension().and_then(|e| e.to_str()) != Some("rs") {
            continue;
        }
        // All path-shape filters run on the path RELATIVE to the scan root — an absolute prefix can itself
        // contain `target`/`.cargo`/… (a vendored crate lives under `~/.cargo/registry/...`), which must
        // not trip them.
        let rel = p.strip_prefix(root).unwrap_or(p);
        // target/ build artifacts; hidden dirs (`.git`, `.github`, `.cargo`, …) holding tooling/CI scripts,
        // not library code (smol_str's `.github/ci.rs` otherwise reported a phantom `Exec`).
        if rel.components().any(|c| {
            c.as_os_str()
                .to_str()
                .is_some_and(|s| s == "target" || (s.starts_with('.') && s != "." && s != ".."))
        }) {
            continue;
        }
        // The Cargo BUILD SCRIPT is `<crate-root>/build.rs` — it runs at COMPILE time (ring's build.rs
        // execs nasm), never the crate's runtime behaviour, so skip it. But ONLY at the root: a nested
        // `src/build.rs` is an ordinary source module that merely shares the name (git2's `src/build.rs`
        // is `RepoBuilder` — the whole clone/fetch NETWORK surface), and dropping it silently under-reports
        // (an A/B found `git2::Repository::clone` reporting no `Net` because its module had vanished).
        if is_build_script(rel) {
            continue;
        }
        // Cargo's non-library compilation targets (tests/, benches/, examples/) — and the common nonstandard
        // singular `test/` tree (e.g. nix) — describe what the crate's HARNESS does (spawn a server, read
        // fixtures, seed RNG), not what the crate itself does. Scanning them conflates the two (redis's bench
        // harness alone showed Exec/Net/Fs/Env/Rand on 200+ fns). Skip by default; `--include-tests` keeps them.
        if !include_tests
            && rel.components().any(|c| {
                matches!(
                    c.as_os_str().to_str(),
                    Some("tests") | Some("test") | Some("benches") | Some("examples")
                )
            })
        {
            continue;
        }
        // A `#[cfg(test)] mod tests;` FILE module is invisible here — its test-ness is declared at the
        // `mod` site, not in the file — so a `tests.rs` / `*_tests.rs` / `*_test.rs` file's effects (a
        // seeded RNG, a temp file) would be mis-read as the crate's. By convention these stems are test
        // modules; skip them by default. (base64's `engine/tests.rs` otherwise reported a phantom `Rand`.)
        if !include_tests {
            if let Some(stem) = p.file_stem().and_then(|s| s.to_str()) {
                if is_test_file_stem(stem) {
                    continue;
                }
            }
        }
        let Ok(text) = std::fs::read_to_string(p) else { continue };
        let Ok(file) = syn::parse_file(&text) else { continue };
        parsed.push((rel.to_string_lossy().into_owned(), file));
    }

    // Pass A — index struct field types and function return types crate-wide, so a method call on
    // `self.field` or on the result of a local factory function can be typed and classified. The
    // trait indexes ride along: impl universe + declaration counts + dispatch-typed fields, for the
    // syntactic-CHA resolution of `dyn`/`impl Trait`/generic-bound receivers.
    let mut fields: FieldIndex = HashMap::new();
    let mut rets_tmp: HashMap<String, Option<String>> = HashMap::new();
    let mut trait_impls: TraitImplIndex = HashMap::new();
    let mut trait_decls: HashMap<String, LocalTrait> = HashMap::new();
    let mut trait_fields: TraitFieldIndex = HashMap::new();
    for (_, file) in &parsed {
        let mut uses = HashMap::new();
        collect_decls(&file.items, include_tests, &mut uses, &mut fields, &mut rets_tmp,
                      &mut trait_impls, &mut trait_decls, &mut trait_fields);
    }
    // Keep only unambiguous fn-leaf -> return-type mappings (a name with conflicting return types was
    // marked `None`); a guessed type would mis-classify.
    let returns: ReturnIndex = rets_tmp.into_iter().filter_map(|(k, v)| v.map(|t| (k, t))).collect();
    let traits = TraitIndexes { impls: &trait_impls, decls: &trait_decls, fields: &trait_fields };

    // Pass B — collect each function's calls (now with receiver-type inference available).
    let mut fns: Vec<FnInfo> = Vec::new();
    for (rel, file) in &parsed {
        let modpath = module_path(Path::new(rel));
        let mut uses = HashMap::new();
        scan_items(&file.items, &modpath, rel, include_tests, &fields, &returns, traits, &mut uses, &mut fns);
    }

    // The κ-coverage ledger: Cargo.toml's [dependencies] are the crate's TRUE external universe, so a
    // dep the calls actually reach whose classification never fires — and that isn't in a calibrated
    // tier — is a named blind spot (invisible, not Unknown: the curated-κ caveat). Counted here,
    // disclosed in the receipt, so the caveat is per-scan evidence instead of a doc footnote.
    let (deps, dep_renames) = cargo_deps(dir);
    let mut dep_seen: HashMap<String, usize> = HashMap::new(); // dep crate root -> call-site count
    let mut dep_classified: std::collections::HashSet<String> = std::collections::HashSet::new();

    // Two name indexes for resolving a call to a local definition. `by_leaf` keys on the bare last
    // segment (`new`); `by_tail2` keys on the last TWO segments (`RequestBuilder::new`). The leaf index
    // alone catastrophically over-connects on real crates: every call to *some* `new()` would link to
    // ALL `*::new` defs (in reqwest, 181 of them), smearing one type's effect across the whole graph.
    // So a `Type::method`/`mod::fn` call matches the qualified tail (keeping `RequestBuilder::new` distinct
    // from `Body::new`) and a bare free call matches the leaf — BOTH only when the match is UNAMBIGUOUS
    // (exactly one def), under-reporting rather than fabricating. See `resolve_target` + the module doc.
    let mut by_leaf: HashMap<String, Vec<String>> = HashMap::new();
    let mut by_tail2: HashMap<String, Vec<String>> = HashMap::new();
    // Type names with a LOCAL definition — the penultimate `Type` segment of a `Type::method` qual. A
    // receiver-typed method call resolves to a local method ONLY if its type is in here, so an external
    // `reqwest::Client::send` can't mis-link to a same-named local `Client::send` (an inverse fabrication).
    let mut local_types: std::collections::HashSet<String> = std::collections::HashSet::new();
    for f in &fns {
        by_leaf.entry(f.leaf.clone()).or_default().push(f.qual.clone());
        if let Some(t2) = tail2(&f.qual) {
            if let Some(ty) = t2.split("::").next() {
                if ty.chars().next().is_some_and(|c| c.is_uppercase()) {
                    local_types.insert(ty.to_string());
                }
            }
            by_tail2.entry(t2).or_default().push(f.qual.clone());
        }
    }

    let mut direct: HashMap<String, BTreeSet<&'static str>> = HashMap::new();
    let mut hosts: HashMap<String, BTreeSet<String>> = HashMap::new();
    let mut cmds: HashMap<String, BTreeSet<String>> = HashMap::new();
    let mut paths: HashMap<String, BTreeSet<String>> = HashMap::new();
    let mut tables: HashMap<String, BTreeSet<String>> = HashMap::new();
    let mut calls: HashMap<String, BTreeSet<String>> = HashMap::new();
    let mut loc: HashMap<String, String> = HashMap::new();
    for f in &fns {
        loc.entry(f.qual.clone()).or_insert_with(|| f.loc.clone());
        // The body invoked a callable the scan can't see through (closure / fn-pointer value): it could
        // perform any effect, so record an honest `Unknown` (propagated like any effect, surfaced in the
        // receipt's unresolved count) instead of silently certifying the function pure.
        if f.unresolved {
            direct.entry(f.qual.clone()).or_default().insert("Unknown");
        }
        for c in &f.calls {
            let cr = c.path.split("::").next().unwrap_or("");
            let classified = candor_classify::classify(cr, &c.path);
            // κ ledger: a qualified call into a declared dependency. (A bare leaf has no `::`, so it
            // can't name a crate; a local module sharing a dep's name is the rare accepted ambiguity.)
            if c.path.contains("::") && deps.contains(cr) {
                *dep_seen.entry(cr.to_string()).or_insert(0) += 1;
                if classified.is_some() {
                    dep_classified.insert(cr.to_string());
                }
            }
            // Cross-crate chaining (spec §2): an UNCLASSIFIED qualified call into a crate a
            // CANDOR_DEPS sibling report covers inherits that function's recorded effects and
            // literal surfaces — joined unambiguous-tail2-first, then unambiguous leaf, like
            // `resolve_target`. A chained dep is covered, not a κ blind spot.
            // a renamed dependency: the call's crate root is the manifest KEY; reports/registry
            // know the REAL package — join and cover under the real name
            let cr_real: &str = dep_renames.get(cr).map(String::as_str).unwrap_or(cr);
            if classified.is_none() && c.path.contains("::") && deps_idx.crates.contains(cr_real) {
                // Join on the call path RELATIVE to the crate root: a multi-segment rel joins by
                // its qualified tail ONLY (review: a bare-leaf fallback for qualified paths let a
                // pure `MockDb::connect` inherit `Db::connect`'s effects — fabrication); a
                // single-segment rel (a crate-root free fn) IS the dep's leaf key.
                let rel = c.path.strip_prefix(&format!("{cr}::")).unwrap_or(&c.path);
                let hit = if rel.contains("::") {
                    tail2(rel).and_then(|t2| deps_idx.by_key.get(&format!("{cr_real}#{t2}")))
                } else {
                    deps_idx.by_key.get(&format!("{cr_real}#{rel}"))
                };
                if let Some(de) = hit {
                    for e in &de.effects {
                        direct.entry(f.qual.clone()).or_default().insert(e);
                    }
                    hosts.entry(f.qual.clone()).or_default().extend(de.hosts.iter().cloned());
                    cmds.entry(f.qual.clone()).or_default().extend(de.cmds.iter().cloned());
                    paths.entry(f.qual.clone()).or_default().extend(de.paths.iter().cloned());
                    tables.entry(f.qual.clone()).or_default().extend(de.tables.iter().cloned());
                    dep_classified.insert(cr.to_string());
                }
            }
            if let Some(eff) = classified {
                direct.entry(f.qual.clone()).or_default().insert(eff);
                if let Some(s) = &c.str_arg {
                    match eff {
                        "Net" => { hosts.entry(f.qual.clone()).or_default().insert(host_part(s)); }
                        "Exec" => {
                            // Capture the program head + refine the cliff (spec §4 ⟨0.5⟩) ONLY at a
                            // program-NAMING call (`new`/`cmd`), an ALLOWLIST — not "any method except a
                            // known modifier". A whole-crate-Exec crate (portable_pty/duct) classifies
                            // EVERY method as Exec, so a denylist leaked non-naming methods (a getter
                            // `get_env("psql")` reads back a KEY, not a program) → fabricated Db + polluted
                            // the `cmds` surface (a false `allow Exec` match). Method = the path's last segment.
                            if candor_classify::is_cmd_naming_method(c.path.rsplit("::").next().unwrap_or("")) {
                                cmds.entry(f.qual.clone()).or_default().insert(s.clone());
                                direct.entry(f.qual.clone()).or_default()
                                    .extend(candor_classify::classify_command_head(s).iter().copied());
                            }
                        }
                        "Fs" => { paths.entry(f.qual.clone()).or_default().insert(s.clone()); }
                        // Table-position identifiers in a SQL string literal — the Db literal
                        // surface (feeds `allow Db …`); a dynamically-built query yields nothing.
                        "Db" => { tables.entry(f.qual.clone()).or_default().extend(candor_classify::tables_in_sql(s)); }
                        _ => {}
                    }
                }
            }
            // Resolve the call to a local definition via the precise, uniqueness-filtered `resolve_target`.
            // A receiver-typed `Type::method` call (`x.go()` inferred to `S::go`) resolves to the local
            // method ONLY when `Type` is locally defined — this recovers the common `x.method()` edge that
            // a bare leaf can't safely provide, while an external `reqwest::Client::send` is left to the
            // classifier (its type isn't local, so it can't mis-link to a same-named local `Client::send`).
            // A non-typed call uses the leaf/qualified-tail routes; std/core/alloc are the classifier's.
            let resolvable = if c.typed {
                tail2(&c.path)
                    .and_then(|t2| t2.split("::").next().map(str::to_string))
                    .is_some_and(|ty| local_types.contains(&ty))
            } else {
                !matches!(cr, "std" | "core" | "alloc")
            };
            if resolvable {
                let targets = resolve_target(&c.path, &c.leaf, c.method, &by_tail2, &by_leaf);
                if let Some(targets) = targets {
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
    let tablesacc = propagate_str(&tables, &calls, &all);

    let mut entries: Vec<ReportEntry> = Vec::new();
    let mut cg: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for q in &all {
        // SPEC §2.2: the sidecar records EVERY analyzed function — including a LEAF with no local
        // callees, as an empty list. Omitting leaves made an uncalled FFI-only fn (nix `unistd::pipe`)
        // invisible to `whatif`/`callers` ("no function matching") even though it's in the report;
        // an always-present key also lets a consumer distinguish "no callers" from "no such function".
        cg.insert(q.clone(), calls.get(q).map(|cs| cs.iter().cloned().collect()).unwrap_or_default());
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
            // Honest blind-spot signal: this function (transitively) reached a callable the scan couldn't
            // see through. Mirrors the lint's `unresolved = has Unknown`, so the receipt's unresolved
            // count is truthful for the stable backend too — not a hardcoded 0.
            unresolved: inf.contains("Unknown"),
            // The cross-crate join key (spec §2): `crate#qual`, derivable by any consumer from its
            // own syntactic view of the call — what CANDOR_DEPS chaining matches against.
            hash: format!("{crate_name}#{q}"),
            fs: Vec::new(),
            hosts: hostsacc.get(q).map(|s| s.iter().cloned().collect()).unwrap_or_default(),
            cmds: cmdsacc.get(q).map(|s| s.iter().cloned().collect()).unwrap_or_default(),
            paths: pathsacc.get(q).map(|s| s.iter().cloned().collect()).unwrap_or_default(),
            tables: tablesacc.get(q).map(|s| s.iter().cloned().collect()).unwrap_or_default(),
            calls: calls.get(q).map(|cs| cs.iter().cloned().collect()).unwrap_or_default(),
            // The syntactic backend has ONE Unknown origin: a call it couldn't see through (a closure /
            // fn-pointer value). Tag it directly-introduced Unknowns so the receipt matches the lint's
            // `unknownWhy` (candor-spec §2). Coarser than the lint's per-trait tag — by design.
            unknown_why: if direct.get(q).is_some_and(|d| d.contains("Unknown")) {
                vec!["callback:unresolved call".to_string()]
            } else {
                Vec::new()
            },
            // candor-spec §2 `entryPoint`: syntactically we can only spot `main` (the program root). The
            // lint also flags `#[no_mangle]`; the scanner can't see attributes, so it under-marks — the
            // sound direction for an optional reachability hint.
            entry_point: q.rsplit("::").next() == Some("main"),
        });
    }
    entries.sort_by(|a, b| a.func.cmp(&b.func));

    let meta = candor_report::ReportMeta {
        version: format!("scan-{}", env!("CARGO_PKG_VERSION")),
        toolchain: "stable".into(),
        spec: candor_report::SPEC_VERSION.into(),
    };
    let body = candor_report::to_packaged_report_json(&meta, &crate_name, &entries).unwrap_or_default();
    // With want_json the body is RETURNED to the caller (which prints one document for a single
    // crate, or wraps N members in a JSON array) rather than printed here — printing per-call gave
    // concatenated, unparseable JSON for a workspace scan.
    let json_body = if want_json {
        Some(body.clone())
    } else {
        let prefix = if prefix.is_empty() { format!("{dir}/.candor/report") } else { prefix };
        if let Some(parent) = Path::new(&prefix).parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let file = format!("{prefix}.{crate_name}.scan.json");
        // Atomic write (temp + rename): a concurrent `candor-query` / `cargo candor watch` reader must
        // never see a half-written report (see candor_report::write_atomic).
        let _ = candor_report::write_atomic(Path::new(&file), body.as_bytes());
        let cgfile = format!("{prefix}.{crate_name}.scan.callgraph.json");
        let _ = candor_report::write_atomic(Path::new(&cgfile), serde_json::to_string(&cg).unwrap_or_default().as_bytes());
        if !quiet {
            eprintln!(
                "candor-scan: wrote {} effectful functions to {file} (stable syntactic backend — see --help)",
                entries.len()
            );
        }
        None
    };

    // The κ-coverage disclosure: dependencies the code demonstrably CALLS that the classifier knows
    // nothing about. Their effects are INVISIBLE — not Unknown — so the report's silence about them
    // is not purity evidence. This turns the curated-κ caveat from a doc footnote into per-scan,
    // named evidence (the argon2 lesson: the blind spot landed on exactly the call a security review
    // cared about).
    let mut unlisted: Vec<(&String, usize)> = dep_seen
        .iter()
        .filter(|(cr, _)| {
            !dep_classified.contains(*cr)
                // A crate with a loaded sibling report is COVERED even when no join fired: the
                // report omits pure functions, so join-less calls are its honest purity claim —
                // the opposite of invisible. (Without this, --deps named serde_json a blind spot.)
                // A RENAMED dep is covered under its real package name.
                && !deps_idx.crates.contains(dep_renames.get(cr.as_str()).map(String::as_str).unwrap_or(cr.as_str()))
                && !candor_classify::CALIBRATED_CRATES.contains(&cr.as_str())
                && !candor_classify::PATH_CALIBRATED_CRATES.contains(&cr.as_str())
                && !candor_classify::CALIBRATED_PREFIXES.iter().any(|p| cr.starts_with(p))
        })
        .map(|(cr, n)| (cr, *n))
        .collect();
    if !unlisted.is_empty() && !quiet {
        unlisted.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(b.0)));
        let shown: Vec<String> =
            unlisted.iter().take(8).map(|(cr, n)| format!("{cr} ({n} call{})", if *n == 1 { "" } else { "s" })).collect();
        let more = if unlisted.len() > 8 { format!(" + {} more", unlisted.len() - 8) } else { String::new() };
        eprintln!(
            "candor-scan: κ doesn't know {} dependenc{} this code calls into — effects through {} are INVISIBLE (not Unknown): {}{}",
            unlisted.len(),
            if unlisted.len() == 1 { "y" } else { "ies" },
            if unlisted.len() == 1 { "it" } else { "them" },
            shown.join(", "),
            more
        );
    }

    // The stable policy gate (spec §6.2 / AS-EFF-006/008/009) — the ADVISORY FLOOR. The syntactic
    // backend under-reports (a missed effect can pass), so this is a floor, never the sound gate
    // (that's the nightly engine / the JVM engine). It still catches every boundary crossing the
    // scan CAN see, deterministically, with zero extra install.
    if let Some(pp) = policy_path {
        let Ok(text) = std::fs::read_to_string(&pp) else {
            // A set-but-unreadable policy must be LOUD — silently passing would let a violation ship.
            eprintln!("candor-scan: policy {pp:?} could not be read; gate NOT enforced");
            return (2, json_body);
        };
        let v = policy_violations(&text, &all, &inferred, &calls, &hostsacc, &cmdsacc, &pathsacc, &tablesacc);
        for line in &v {
            println!("{line}");
        }
        if v.is_empty() {
            eprintln!("candor-scan: policy ✓ (advisory floor — the syntactic backend under-reports; the nightly engine is the sound gate)");
        } else {
            eprintln!("candor-scan: {} policy violation(s) (advisory floor — a clean run is necessary, not sufficient)", v.len());
            return (1, json_body);
        }
    }
    (0, json_body)
}

/// Scan a TARGET — a single crate, or a `[workspace]` root fanned out into one report per member
/// under the shared prefix. The one place both the plain and `--deps` paths funnel through, so a
/// workspace is never scanned as one merged package (colliding same-named fns) nor pruned to an
/// empty report by the nested-package filter. With `want_json`, prints ONE JSON document for a
/// single crate and a JSON ARRAY for a workspace — never concatenated documents. Returns the exit code.
fn scan_target(
    dir: &str,
    prefix: String,
    want_json: bool,
    include_tests: bool,
    policy: Option<String>,
    deps_idx: &DepIndex,
) -> i32 {
    let members = workspace_members(Path::new(dir));
    if members.is_empty() {
        if has_workspace_table(Path::new(dir)) {
            // A [workspace] with zero RESOLVED members: scanning the root as one crate would let the
            // nested-package filter prune every member into an empty report that passes any gate
            // vacuously (§6.2's forbidden state). Warn loudly; the single-crate scan below still
            // covers the root package's own sources, if any.
            eprintln!("candor-scan: `{dir}` declares [workspace] but no members resolved — \
                       check `members`/globs; scan member crates directly to gate them");
        }
        let (code, json) = scan_one(dir, ScanOpts {
            prefix, want_json, include_tests, policy, quiet: false, deps_idx,
        });
        if let Some(b) = json {
            println!("{b}");
        }
        return code;
    }
    let prefix = if prefix.is_empty() { format!("{dir}/.candor/report") } else { prefix };
    let mut dirs: Vec<String> = Vec::new();
    if read_crate_name(Path::new(dir)).is_some() {
        dirs.push(dir.to_string()); // the workspace manifest also declares a root package
    }
    dirs.extend(members);
    let mut rc = 0;
    let mut bodies: Vec<String> = Vec::new();
    for d in &dirs {
        let (code, json) = scan_one(d, ScanOpts {
            prefix: prefix.clone(), want_json, include_tests, policy: policy.clone(), quiet: false, deps_idx,
        });
        rc = rc.max(code);
        if let Some(b) = json {
            bodies.push(b);
        }
    }
    if want_json {
        println!("[{}]", bodies.join(","));
    } else {
        eprintln!("candor-scan: workspace — {} package report(s) under one prefix", dirs.len());
    }
    rc
}

/// `--deps`: read Cargo.lock, scan every REGISTRY dependency's unbuilt source from
/// `~/.cargo/registry/src/<index>/` into `<dir>/.candor/deps/`, then scan the root crate chained
/// over those reports (plus anything CANDOR_DEPS already names). Path/git/workspace deps have no
/// registry checkout and are skipped with a note — chain them by scanning them yourself.
fn run_with_deps(dir: &str, prefix: String, want_json: bool, include_tests: bool, policy: Option<String>) -> i32 {
    let lock = match std::fs::read_to_string(format!("{dir}/Cargo.lock")) {
        Ok(t) => t,
        Err(_) => {
            eprintln!("candor-scan: --deps needs {dir}/Cargo.lock (run `cargo generate-lockfile` first)");
            return 2;
        }
    };
    // [[package]] blocks: name + version + source. Only registry deps have a checkout to scan;
    // the root crate itself has no `source` line and is naturally skipped.
    let mut pkgs: Vec<(String, String)> = Vec::new();
    let (mut name, mut version, mut registry) = (String::new(), String::new(), false);
    let flush = |name: &mut String, version: &mut String, registry: &mut bool, pkgs: &mut Vec<(String, String)>| {
        if *registry && !name.is_empty() && !version.is_empty() {
            pkgs.push((name.clone(), version.clone()));
        }
        name.clear();
        version.clear();
        *registry = false;
    };
    for line in lock.lines() {
        let l = line.trim();
        if l == "[[package]]" {
            flush(&mut name, &mut version, &mut registry, &mut pkgs);
        } else if let Some(v) = l.strip_prefix("name = ") {
            name = v.trim_matches('"').to_string();
        } else if let Some(v) = l.strip_prefix("version = ") {
            version = v.trim_matches('"').to_string();
        } else if l.starts_with("source = ") && l.contains("registry+") {
            registry = true;
        }
    }
    flush(&mut name, &mut version, &mut registry, &mut pkgs);

    let registry_roots: Vec<std::path::PathBuf> = dirs_cargo_registry_src();
    let deps_dir = format!("{dir}/.candor/deps");
    let _ = std::fs::create_dir_all(&deps_dir);
    let (mut scanned, mut cached, mut missing) = (0usize, 0usize, Vec::new());
    let no_deps = DepIndex::default();
    for (n, v) in &pkgs {
        let Some(src) = registry_roots.iter().map(|r| r.join(format!("{n}-{v}"))).find(|p| p.is_dir()) else {
            missing.push(format!("{n}-{v}"));
            continue;
        };
        // One subdirectory PER name@version: two locked versions of one crate must not overwrite
        // each other's report (review: last-write-wins silently fed the root the wrong version's
        // effects); with both present, conflicting keys drop as ambiguous — never-guess intact.
        let sub = format!("{deps_dir}/{n}@{v}");
        let already = std::fs::read_dir(&sub).ok().is_some_and(|rd| {
            rd.flatten().any(|e| {
                let f = e.file_name();
                let f = f.to_string_lossy();
                f.ends_with(".scan.json") && !f.contains("callgraph")
            })
        });
        if already {
            cached += 1; // registry checkouts are immutable per name@version — the report stands
            continue;
        }
        let _ = std::fs::create_dir_all(&sub);
        // Dep scans are quiet, unchained, report-only, and POLICY-FREE (the resolved root policy
        // is deliberately not passed): their job is the report files. A registry dep is a single
        // published package, so scan_one (not scan_target) is right; the json body is unused.
        let _ = scan_one(&src.to_string_lossy(), ScanOpts {
            prefix: format!("{sub}/report"),
            want_json: false,
            include_tests: false,
            policy: None,
            quiet: true,
            deps_idx: &no_deps,
        });
        scanned += 1;
    }
    eprintln!(
        "candor-scan: --deps scanned {scanned} of {} registry dependencies into {deps_dir}{}{} \
(floor-engine reports: a dep's silent misses pass through — the κ caveat applies to the chain too)",
        pkgs.len(),
        if cached > 0 { format!(" ({cached} already scanned — cached)") } else { String::new() },
        if missing.is_empty() {
            String::new()
        } else {
            format!(" ({} without a local checkout: {}{})", missing.len(),
                missing.iter().take(5).cloned().collect::<Vec<_>>().join(", "),
                if missing.len() > 5 { ", …" } else { "" })
        }
    );
    // Chain the fresh dep reports (plus anything CANDOR_DEPS already names) under the root scan.
    // load_dep_reports dedups canonical paths, so deps_dir appearing in CANDOR_DEPS too is safe.
    let spec = match std::env::var("CANDOR_DEPS") {
        Ok(extra) if !extra.is_empty() => format!("{deps_dir}:{extra}"),
        _ => deps_dir.clone(),
    };
    let idx = load_dep_reports(Some(&spec));
    // The final root scan goes through scan_target so `--deps <workspace>` fans out over members
    // too — the nested-package filter would otherwise prune them all into an empty, gate-passing report.
    scan_target(dir, prefix, want_json, include_tests, policy, &idx)
}

/// The cargo registry source roots (`~/.cargo/registry/src/<index-hash>/`), where unbuilt
/// dependency sources live. CARGO_HOME is honoured.
fn dirs_cargo_registry_src() -> Vec<std::path::PathBuf> {
    let home = std::env::var("CARGO_HOME")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|_| Path::new(&std::env::var("HOME").unwrap_or_default()).join(".cargo"));
    std::fs::read_dir(home.join("registry").join("src"))
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.is_dir())
        .collect()
}

/// Evaluate a CANDOR_POLICY (parsed by the SHARED §6.2 parser in candor-classify, so this gate can
/// never disagree with the nightly/JVM gates on grammar) over a finished scan. Returns one line per
/// violation: deny/pure (AS-EFF-006) against the transitive `inferred` sets, literal allowlists
/// (AS-EFF-008) against the transitive hosts/cmds/paths/tables surfaces, layering `forbid A -> B`
/// (AS-EFF-009) by reachability over the local call graph.
#[allow(clippy::too_many_arguments)]
fn policy_violations(
    policy_text: &str,
    all: &[String],
    inferred: &HashMap<String, BTreeSet<&'static str>>,
    calls: &HashMap<String, BTreeSet<String>>,
    hostsacc: &HashMap<String, BTreeSet<String>>,
    cmdsacc: &HashMap<String, BTreeSet<String>>,
    pathsacc: &HashMap<String, BTreeSet<String>>,
    tablesacc: &HashMap<String, BTreeSet<String>>,
) -> Vec<String> {
    use candor_classify::policy::{literal_allowed, parse_policy, scope_matches};
    let p = parse_policy(policy_text);
    let empty: BTreeSet<&'static str> = BTreeSet::new();
    let mut out = Vec::new();
    for q in all {
        let inf = inferred.get(q).unwrap_or(&empty);
        // AS-EFF-006 — deny/pure: forbidden effects in the transitive set.
        for r in &p.rules {
            if let Some(s) = &r.scope {
                if !scope_matches(q, s) {
                    continue;
                }
            }
            let hits: Vec<&str> = if r.effects.is_empty() {
                inf.iter().copied().collect() // `pure` — ANY effect (Unknown included: not certifiably pure)
            } else {
                inf.iter().copied().filter(|e| r.effects.contains(e)).collect()
            };
            if !hits.is_empty() {
                out.push(format!("[AS-EFF-006] `{q}` performs {{ {} }}, forbidden by policy: `{}`", hits.join(", "), r.raw));
            }
        }
        // AS-EFF-008 — literal allowlists over the transitive literal surfaces.
        for r in &p.allow_rules {
            if let Some(s) = &r.scope {
                if !scope_matches(q, s) {
                    continue;
                }
            }
            if !inf.contains(r.effect) {
                continue;
            }
            let lits = match r.effect {
                "Net" => hostsacc.get(q),
                "Exec" => cmdsacc.get(q),
                "Db" => tablesacc.get(q),
                _ => pathsacc.get(q),
            };
            match lits {
                Some(ls) if !ls.is_empty() => {
                    let bad: Vec<&str> =
                        ls.iter().filter(|l| !literal_allowed(r.effect, l, &r.literals)).map(String::as_str).collect();
                    if !bad.is_empty() {
                        out.push(format!("[AS-EFF-008] `{q}` reaches {{ {} }} outside the allowlist: `{}`", bad.join(", "), r.raw));
                    }
                }
                _ => out.push(format!(
                    "[AS-EFF-008] `{q}` performs {} with no visible literal — the surface cannot be certified: `{}`",
                    r.effect, r.raw
                )),
            }
        }
        // AS-EFF-009 — layering: no fn in scope A may transitively reach scope B.
        for r in &p.layer_rules {
            if !scope_matches(q, &r.from) {
                continue;
            }
            let mut seen: BTreeSet<&str> = BTreeSet::new();
            let mut stack: Vec<&str> = calls.get(q).map(|cs| cs.iter().map(String::as_str).collect()).unwrap_or_default();
            let mut hit: Option<&str> = None;
            while let Some(n) = stack.pop() {
                if !seen.insert(n) {
                    continue;
                }
                if scope_matches(n, &r.to) {
                    hit = Some(n);
                    break;
                }
                if let Some(cs) = calls.get(n) {
                    stack.extend(cs.iter().map(String::as_str));
                }
            }
            if let Some(h) = hit {
                out.push(format!("[AS-EFF-009] `{q}` reaches into a forbidden layer (via `{h}`): `{}`", r.raw));
            }
        }
    }
    out.sort();
    out
}

/// The last two `::`-segments of a path (`a::b::Type::new` → `Type::new`), the key used to resolve a
/// `Type::method` call to its definition without colliding every same-named method. `None` for a path
/// with fewer than two segments (a bare leaf — only an unqualified FREE call resolves by leaf; a bare
/// method call with an unresolved receiver under-reports, see `resolve_target`).
fn tail2(path: &str) -> Option<String> {
    let segs: Vec<&str> = path.split("::").collect();
    let n = segs.len();
    if n < 2 {
        return None;
    }
    Some(format!("{}::{}", segs[n - 2], segs[n - 1]))
}

/// Resolve a call to the local definition(s) it links to in the intra-crate graph, or `None` to
/// under-report. A QUALIFIED path (`a::Job::run`, `mod::helper`, or an associated-fn call `Type::new()`)
/// matches on its precise 2-segment tail, but ONLY when that tail is UNAMBIGUOUS — two same-named types in
/// different modules share a tail (`a::Job::run` / `b::Job::run`), so linking a many-way tail would
/// fabricate one type's effect onto the other's caller (the same flood the bare-leaf index causes, one
/// level up). An UNQUALIFIED free-function call falls back to a unique bare leaf. An UNQUALIFIED method
/// call with an unresolved receiver names no definite target, so it under-reports rather than guess —
/// this is what stops `range.start()` linking to a unique local `Clipboard::start`. NB a receiver-typed
/// `Type::method` call DOES arrive here (via the qualified-tail branch) — but only after the caller has
/// confirmed `Type` is locally defined, so an external `reqwest::Client::send` is filtered out upstream.
fn resolve_target<'a>(
    path: &str,
    leaf: &str,
    method: bool,
    by_tail2: &'a HashMap<String, Vec<String>>,
    by_leaf: &'a HashMap<String, Vec<String>>,
) -> Option<&'a Vec<String>> {
    if path.contains("::") {
        tail2(path).and_then(|t2| by_tail2.get(&t2)).filter(|v| v.len() == 1)
    } else if method {
        None
    } else {
        by_leaf.get(leaf).filter(|v| v.len() == 1)
    }
}

fn host_part(h: &str) -> String {
    let a = h.split_once("://").map(|(_, r)| r).unwrap_or(h);
    let a = a.split('/').next().unwrap_or(a);
    a.rsplit_once('@').map(|(_, h)| h).unwrap_or(a).to_string()
}

fn read_crate_name(root: &Path) -> Option<String> {
    let txt = std::fs::read_to_string(root.join("Cargo.toml")).ok()?;
    let mut in_package = false;
    for line in txt.lines() {
        if let Some(section) = toml_section(line) {
            in_package = section == "package"; // only [package]'s `name` is the crate name
            continue;
        }
        // `name` inside `[package]` only (a `name =` in `[[bin]]`/`[dependencies]` is not the crate name).
        if in_package {
            if let Some(v) = toml_scalar(line, "name") {
                return Some(v.replace('-', "_"));
            }
        }
    }
    None
}

/// The string entries of `key = [ ... ]` inside `[table]` — line-based (the manifest subset that
/// matters), multi-line arrays included. No TOML dependency, same trade as the parsers above.
fn toml_string_array(txt: &str, table: &str, key: &str) -> Vec<String> {
    let (mut in_table, mut collecting) = (false, false);
    let mut out = Vec::new();
    for line in txt.lines() {
        let l = line.trim();
        if !collecting {
            if let Some(section) = toml_section(line) {
                in_table = section == table;
                continue;
            }
        }
        if !in_table {
            continue;
        }
        let rest = if let Some(r) = l.strip_prefix(key) {
            let r = r.trim_start();
            let Some(r) = r.strip_prefix('=') else { continue };
            collecting = true;
            r
        } else if collecting {
            l
        } else {
            continue;
        };
        let mut parts = rest.split('"');
        parts.next();
        while let Some(s) = parts.next() {
            out.push(s.to_string());
            if parts.next().is_none() {
                break;
            }
        }
        if rest.contains(']') {
            collecting = false;
        }
    }
    out
}

/// True if the manifest declares a `[workspace]` table at all (distinct from "has members"): a
/// workspace root with zero RESOLVED members must warn, not silently fall through to a single-crate
/// scan whose nested-package filter then prunes every member into an empty report.
fn has_workspace_table(root: &Path) -> bool {
    std::fs::read_to_string(root.join("Cargo.toml"))
        .map(|t| t.lines().any(|l| l.trim() == "[workspace]"))
        .unwrap_or(false)
}

/// Member directories of the root manifest's `[workspace]`, joined to `root`, honouring `exclude`,
/// expanding globs (a bare `*` = root's immediate children, `prefix/*` = a dir's children), and
/// DEDUPLICATED (a member listed explicitly AND matched by a glob otherwise scans/prints twice).
/// Empty when there is no `members` key. A `*`-pattern this simple matcher can't expand is WARNED,
/// never silently dropped (a dropped member yields a vacuous gate, the §6.2 forbidden state).
fn workspace_members(root: &Path) -> Vec<String> {
    let Ok(txt) = std::fs::read_to_string(root.join("Cargo.toml")) else { return Vec::new() };
    let members = toml_string_array(&txt, "workspace", "members");
    if members.is_empty() {
        return Vec::new();
    }
    let exclude = toml_string_array(&txt, "workspace", "exclude");
    // Expand a `<base>/*` (base "" for a bare `*`) to its child dirs carrying a Cargo.toml.
    let expand = |base: &str| -> Vec<String> {
        let dir = if base.is_empty() { root.to_path_buf() } else { root.join(base) };
        let mut found: Vec<String> = std::fs::read_dir(dir)
            .into_iter()
            .flatten()
            .filter_map(Result::ok)
            .filter(|e| e.path().join("Cargo.toml").is_file())
            .map(|e| {
                let n = e.file_name().to_string_lossy().into_owned();
                if base.is_empty() { n } else { format!("{base}/{n}") }
            })
            .collect();
        found.sort();
        found
    };
    let mut rels: Vec<String> = Vec::new();
    for m in members {
        if m == "*" {
            rels.extend(expand(""));
        } else if let Some(base) = m.strip_suffix("/*") {
            rels.extend(expand(base));
        } else if m.contains('*') {
            eprintln!("candor-scan: workspace member glob `{m}` is not a trailing `*` — not expanded; \
                       scan its crates directly or list them explicitly");
        } else if root.join(&m).join("Cargo.toml").is_file() {
            rels.push(m);
        }
    }
    rels.retain(|m| !exclude.iter().any(|e| m == e || m.starts_with(&format!("{e}/"))));
    rels.sort();
    rels.dedup();
    rels.into_iter().map(|m| root.join(m).to_string_lossy().into_owned()).collect()
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
        // `use std::fs::{self, Metadata}` imports the MODULE `fs` itself → map `fs -> std::fs`.
        let mut o2 = HashMap::new();
        collect_use(&syn::parse_str("std::fs::{self, Metadata}").unwrap(), String::new(), &mut o2);
        assert_eq!(o2.get("fs").map(String::as_str), Some("std::fs"));
        assert_eq!(o2.get("Metadata").map(String::as_str), Some("std::fs::Metadata"));
        assert_eq!(o2.get("self"), None); // not the useless `fs::self`
    }

    #[test]
    fn module_path_mirrors_file_based_resolution() {
        assert_eq!(module_path(Path::new("src/lib.rs")), "");
        assert_eq!(module_path(Path::new("src/main.rs")), "");
        assert_eq!(module_path(Path::new("src/pricing.rs")), "pricing");
        assert_eq!(module_path(Path::new("src/billing/mod.rs")), "billing");
        assert_eq!(module_path(Path::new("src/billing/tax.rs")), "billing::tax");
        // a dotted file stem (tonic/prost gRPC codegen) is a nested module path, not one segment.
        assert_eq!(
            module_path(Path::new("src/generated/envoy.service.auth.v3.rs")),
            "generated::envoy::service::auth::v3"
        );
        // a WORKSPACE member's path anchors at its OWN `src/`, not the scan root — otherwise the dir
        // path (`crates/cli/src/decompress.rs`) mangles into `crates::cli::src::decompress`.
        assert_eq!(module_path(Path::new("crates/cli/src/decompress.rs")), "decompress");
        assert_eq!(module_path(Path::new("crates/ignore/src/walk.rs")), "walk");
        assert_eq!(module_path(Path::new("crates/core/src/main.rs")), "");
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
    fn tail2_keys_on_the_qualified_method() {
        assert_eq!(tail2("a::b::RequestBuilder::new").as_deref(), Some("RequestBuilder::new"));
        assert_eq!(tail2("pricing::compute_price").as_deref(), Some("pricing::compute_price"));
        assert_eq!(tail2("send"), None); // a bare method leaf — no type qualifier to disambiguate
    }

    #[test]
    fn qualified_tail_disambiguates_same_named_methods() {
        // Two distinct `new`s; a `RequestBuilder::new` call must resolve to ONLY the RequestBuilder one,
        // never to every `*::new` (the leaf-collision over-connection that smeared one effect crate-wide).
        let fns = ["http::RequestBuilder::new", "body::Body::new"];
        let mut by_leaf: HashMap<String, Vec<String>> = HashMap::new();
        let mut by_tail2: HashMap<String, Vec<String>> = HashMap::new();
        for q in fns {
            by_leaf.entry("new".into()).or_default().push(q.into());
            by_tail2.entry(tail2(q).unwrap()).or_default().push(q.into());
        }
        // a `RequestBuilder::new(...)` call — routed through PRODUCTION `resolve_target` (qualified tail).
        assert_eq!(resolve_target("api::RequestBuilder::new", "new", false, &by_tail2, &by_leaf),
                   Some(&vec!["http::RequestBuilder::new".to_string()]));
        // a bare `.new()`-by-leaf with two candidates resolves to NEITHER (ambiguous → under-report)
        assert_eq!(resolve_target("new", "new", true, &by_tail2, &by_leaf), None);
    }

    #[test]
    fn macro_bodies_are_walked_for_hidden_calls() {
        // git2 hides every libgit2 FFI call in `try_call!(...)`; format macros hide call args. Both
        // must be collected, while a non-expression macro body (matches!) is skipped without panicking.
        let uses = HashMap::new();
        let fields = FieldIndex::new();
        let block: syn::Block = syn::parse_str(
            "{ try_call!(raw::git_remote_fetch(x)); println!(\"{}\", helper()); let _ = matches!(y, Some(_)); }",
        )
        .unwrap();
        let returns = ReturnIndex::new();
        let (ti, td, tf) = (TraitImplIndex::new(), HashMap::new(), TraitFieldIndex::new());
        let mut c = CallCollector {
            uses: &uses,
            vars: HashMap::new(),
            trait_vars: HashMap::new(),
            fields: &fields,
            trait_fields: &tf,
            trait_impls: &ti,
            local_traits: &td,
            returns: &returns,
            calls: Vec::new(),
            closure_vars: std::collections::HashSet::new(),
            unresolved: false,
        };
        for stmt in &block.stmts {
            c.visit_stmt(stmt);
        }
        let leaves: Vec<&str> = c.calls.iter().map(|c| c.leaf.as_str()).collect();
        assert!(leaves.contains(&"git_remote_fetch"), "call inside try_call! macro was missed: {leaves:?}");
        assert!(leaves.contains(&"helper"), "call inside println! macro was missed: {leaves:?}");
    }

    #[test]
    fn receiver_type_inference_resolves_method_dispatch() {
        // A method call on a param/field/typed-let of a known type resolves to `Type::method`, so the
        // existing per-crate rules fire. Build a collector with `client: reqwest::Client` in scope.
        let uses = HashMap::new();
        let mut fields = FieldIndex::new();
        // struct App { http: reqwest::Client }
        fields.entry("App".into()).or_default().insert("http".into(), "reqwest::Client".into());
        let mut vars = HashMap::new();
        vars.insert("client".to_string(), "reqwest::Client".to_string());
        vars.insert("self".to_string(), "App".to_string());
        let returns = ReturnIndex::new();
        let (ti, td, tf) = (TraitImplIndex::new(), HashMap::new(), TraitFieldIndex::new());
        let block: syn::Block =
            syn::parse_str("{ client.get(url).send(); self.http.execute(req); }").unwrap();
        let mut c = CallCollector {
            uses: &uses,
            vars,
            trait_vars: HashMap::new(),
            fields: &fields,
            trait_fields: &tf,
            trait_impls: &ti,
            local_traits: &td,
            returns: &returns,
            calls: Vec::new(),
            closure_vars: std::collections::HashSet::new(),
            unresolved: false,
        };
        for stmt in &block.stmts {
            c.visit_stmt(stmt);
        }
        let typed: Vec<&str> = c.calls.iter().map(|c| c.path.as_str()).collect();
        // chain `client.get(url).send()` → base type reqwest::Client, terminal verb send
        assert!(typed.contains(&"reqwest::Client::send"), "chain not typed to base: {typed:?}");
        // field access `self.http.execute(req)` resolves via the struct field index
        assert!(typed.contains(&"reqwest::Client::execute"), "field recv not typed: {typed:?}");
        // and both classify as Net through the shared classifier
        assert_eq!(candor_classify::classify("reqwest", "reqwest::Client::send"), Some("Net"));
        assert_eq!(candor_classify::classify("reqwest", "reqwest::Client::execute"), Some("Net"));
    }

    #[test]
    fn cargo_toml_deps_handles_all_header_forms() {
        let mut out = std::collections::HashSet::new();
        let mut renames = HashMap::new();
        cargo_toml_deps(
            "[package]\nname = \"x\"\n\n[dependencies]\nserde_json = \"1\"\nleft-pad = \"1\"\n\n[dependencies.table-header]\nversion = \"1\"\n\n[target.'cfg(unix)'.dependencies.nix]\nversion = \"0.29\"\n\n[target.'cfg(windows)'.dependencies]\nwinapi = \"0.3\"\n\n[workspace.dependencies]\nshared-dep = \"2\"\n\n[dev-dependencies]\ncriterion = \"0.5\"\n\n[build-dependencies]\ncc = \"1\"\n\n[dev-dependencies.proptest]\nversion = \"1\"\n",
            &mut out,
            &mut renames,
        );
        for d in ["serde_json", "left_pad", "table_header", "nix", "winapi", "shared_dep"] {
            assert!(out.contains(d), "missing {d}: {out:?}");
        }
        for d in ["criterion", "cc", "proptest", "x"] {
            assert!(!out.contains(d), "harness/package dep leaked: {d}");
        }
        // dependency RENAMES, both forms (found live: ebman's `tui-common = { package = "tb-tui-common" }`)
        let mut out2 = std::collections::HashSet::new();
        let mut ren2 = HashMap::new();
        cargo_toml_deps(
            "[dependencies]\ntui-common = { version = \"0.1\", package = \"tb-tui-common\" }\n\n[dependencies.short-name]\nversion = \"1\"\npackage = \"the-real-crate\"\n",
            &mut out2,
            &mut ren2,
        );
        assert!(out2.contains("tui_common") && out2.contains("short_name"), "{out2:?}");
        assert_eq!(ren2.get("tui_common").map(String::as_str), Some("tb_tui_common"));
        assert_eq!(ren2.get("short_name").map(String::as_str), Some("the_real_crate"));
        // A dep whose KEY contains "package" must NOT parse its own version as a rename, and a dep
        // literally NAMED `package` is a dependency, not a rename (the substring-match regression).
        let mut out3 = std::collections::HashSet::new();
        let mut ren3 = HashMap::new();
        cargo_toml_deps(
            "[dependencies]\nmy-package = \"1.2\"\nfoo-package = { version = \"2\" }\npackage = \"0.1\"\nreal = { version = \"1\", package = \"renamed-crate\" }\n",
            &mut out3,
            &mut ren3,
        );
        assert!(out3.contains("my_package") && out3.contains("foo_package") && out3.contains("package"));
        assert!(ren3.get("my_package").is_none(), "key-substring 'package' fabricated a rename: {ren3:?}");
        assert!(ren3.get("foo_package").is_none(), "{ren3:?}");
        assert!(ren3.get("package").is_none(), "a dep named `package` is not a rename: {ren3:?}");
        assert_eq!(ren3.get("real").map(String::as_str), Some("renamed_crate"), "real rename lost: {ren3:?}");
    }

    #[test]
    fn dep_report_chaining_joins_unambiguously_and_distrusts_stale_versions() {
        let d = std::env::temp_dir().join(format!("candor-deps-test-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&d);
        let me = format!("scan-{}", env!("CARGO_PKG_VERSION"));
        // same-version report: effects + surfaces join; two fns sharing a leaf drop that key
        std::fs::write(d.join("report.billing.scan.json"), format!(r#"{{
            "candor": {{"version": "{me}", "toolchain": "stable", "spec": "0.3"}},
            "functions": [
              {{"fn": "ledger::Ledger::post", "inferred": ["Db"], "tables": ["ledger.entries"], "hash": "billing#ledger::Ledger::post"}},
              {{"fn": "a::dup", "inferred": ["Net"], "hash": "billing#a::dup"}},
              {{"fn": "b::dup", "inferred": ["Fs"], "hash": "billing#b::dup"}}
            ]}}"#)).unwrap();
        // a STALE producer version: §2.1 — downgraded to Unknown, never silently trusted
        std::fs::write(d.join("report.old_dep.scan.json"), r#"{
            "candor": {"version": "scan-0.0.1", "toolchain": "stable", "spec": "0.3"},
            "functions": [{"fn": "io::go", "inferred": ["Exec"], "hash": "old_dep#io::go"}]}"#).unwrap();
        let idx = load_dep_reports(Some(d.to_str().unwrap()));
        assert!(idx.crates.contains("billing") && idx.crates.contains("old_dep"));
        let post = idx.by_key.get("billing#Ledger::post").expect("tail2 key");
        assert_eq!(post.effects, vec!["Db"]);
        assert_eq!(post.tables, vec!["ledger.entries"]);
        assert!(idx.by_key.contains_key("billing#post"), "unambiguous leaf key");
        assert!(!idx.by_key.contains_key("billing#dup"), "shared leaf must be dropped, never guessed");
        assert!(idx.by_key.contains_key("billing#a::dup"), "tail2 still disambiguates the dups");
        let old = idx.by_key.get("old_dep#go").expect("stale entry present");
        assert_eq!(old.effects, vec!["Unknown"], "stale version must downgrade to Unknown");
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn dispatch_typed_receivers_resolve_via_local_impls_or_read_unknown() {
        // The trait-object hole, closed: `t.save()` on a `&dyn Store` either edges to the LOCAL
        // implementors (syntactic CHA, the JVM engine's bounded move) or reads honest Unknown —
        // never silent-pure. External traits stay out (no Unknown flood on `impl Iterator`).
        let uses = HashMap::new();
        let fields = FieldIndex::new();
        let returns = ReturnIndex::new();
        let mut ti = TraitImplIndex::new();
        ti.insert("Store".into(), vec!["PgStore".into(), "MemStore".into()]);
        let mut td: HashMap<String, LocalTrait> = HashMap::new();
        td.insert("Store".into(), LocalTrait { count: 1, methods: ["save".to_string()].into_iter().collect() });
        td.insert("Sink".into(), LocalTrait { count: 1, methods: ["flush".to_string()].into_iter().collect() }); // no impl in sight
        let mut tf = TraitFieldIndex::new();
        // struct App { store: Box<dyn Store> }
        tf.entry("App".into()).or_default().insert("store".into(), vec!["Store".into()]);
        let run = |src: &str, sig: &str| {
            let sig: syn::Signature = syn::parse_str(sig).unwrap();
            let blk: syn::Block = syn::parse_str(src).unwrap();
            let trait_vars = seed_trait_vars(&sig);
            let mut vars = seed_vars(&sig, Some("App"), &uses);
            for k in trait_vars.keys() {
                vars.remove(k); // same precedence as fninfo: dispatch-typing wins
            }
            vars.insert("self".to_string(), "App".to_string());
            let mut c = CallCollector {
                uses: &uses,
                vars,
                trait_vars,
                fields: &fields,
                trait_fields: &tf,
                trait_impls: &ti,
                local_traits: &td,
                returns: &returns,
                calls: Vec::new(),
                closure_vars: std::collections::HashSet::new(),
                unresolved: false,
            };
            for stmt in &blk.stmts {
                c.visit_stmt(stmt);
            }
            (c.calls.iter().map(|c| c.path.clone()).collect::<Vec<_>>(), c.unresolved)
        };
        // &dyn param → typed edges to BOTH local impls, not unresolved
        let (paths, unres) = run("{ t.save(x); }", "fn f(t: &dyn Store)");
        assert!(paths.contains(&"PgStore::save".to_string()), "dyn param not CHA-resolved: {paths:?}");
        assert!(paths.contains(&"MemStore::save".to_string()), "dyn param missed an impl: {paths:?}");
        assert!(!unres, "narrow local dispatch must not read Unknown");
        // impl-Trait param and generic bound take the same route
        let (paths, _) = run("{ s.save(x); }", "fn f(s: impl Store)");
        assert!(paths.contains(&"PgStore::save".to_string()), "impl-Trait param not resolved: {paths:?}");
        let (paths, _) = run("{ x.save(y); }", "fn f<X: Store>(x: X)");
        assert!(paths.contains(&"PgStore::save".to_string()), "generic bound not resolved: {paths:?}");
        // the DI field: self.store is Box<dyn Store> via the trait-field index
        let (paths, _) = run("{ self.store.save(x); }", "fn f(&self)");
        assert!(paths.contains(&"PgStore::save".to_string()), "trait-typed field not resolved: {paths:?}");
        // a LOCAL trait with no visible impl: something implements it somewhere — honest Unknown
        let (_, unres) = run("{ k.flush(); }", "fn f(k: &dyn Sink)");
        assert!(unres, "local trait without impls must read Unknown, not silent-pure");
        // an EXTERNAL trait (not locally declared, no local impls): documented miss, NO flood
        let (paths, unres) = run("{ it.next(); }", "fn f(it: impl Iterator)");
        assert!(!unres && !paths.iter().any(|p| p.contains("::next")), "external trait must stay out: {paths:?}");
        // FABRICATION GUARD (review, execution-verified): an EXTERNAL trait with a LOCAL impl
        // must STILL stay out — `impl Iterator for RowIter` + `f(it: impl Iterator)` must not
        // charge f with RowIter's effects.
        {
            let mut ti2 = TraitImplIndex::new();
            ti2.insert("Iterator".into(), vec!["RowIter".into()]);
            let sig: syn::Signature = syn::parse_str("fn f(it: impl Iterator)").unwrap();
            let blk: syn::Block = syn::parse_str("{ it.next(); }").unwrap();
            let mut c = CallCollector {
                uses: &uses, vars: HashMap::new(), trait_vars: seed_trait_vars(&sig),
                fields: &fields, trait_fields: &tf, trait_impls: &ti2, local_traits: &td,
                returns: &returns, calls: Vec::new(),
                closure_vars: std::collections::HashSet::new(), unresolved: false,
            };
            for stmt in &blk.stmts { c.visit_stmt(stmt); }
            assert!(!c.calls.iter().any(|x| x.path == "RowIter::next"),
                    "external-trait local impl must not resolve (fabrication)");
            assert!(!c.unresolved, "external trait must not flood Unknown either");
        }
        // a method the LOCAL trait does NOT declare (supertrait/blanket call) — out, no flood
        let (paths, unres) = run("{ t.clone(); }", "fn f(t: &dyn Store)");
        assert!(!unres && !paths.iter().any(|p| p.ends_with("::clone")),
                "undeclared method must neither edge nor flood: {paths:?}");
        // the cross-engine CHA bound: 12 impls resolve, 13 read honest Unknown
        {
            let wide = |n: usize, src: &str, sig: &str| {
                let mut ti2 = TraitImplIndex::new();
                ti2.insert("Store".into(), (0..n).map(|i| format!("S{i}")).collect());
                let sig: syn::Signature = syn::parse_str(sig).unwrap();
                let blk: syn::Block = syn::parse_str(src).unwrap();
                let mut c = CallCollector {
                    uses: &uses, vars: HashMap::new(), trait_vars: seed_trait_vars(&sig),
                    fields: &fields, trait_fields: &tf, trait_impls: &ti2, local_traits: &td,
                    returns: &returns, calls: Vec::new(),
                    closure_vars: std::collections::HashSet::new(), unresolved: false,
                };
                for stmt in &blk.stmts { c.visit_stmt(stmt); }
                (c.calls.iter().filter(|x| x.typed).count(), c.unresolved)
            };
            let (edges, unres) = wide(12, "{ t.save(x); }", "fn f(t: &dyn Store)");
            assert!(edges == 12 && !unres, "12 impls must resolve (the shared bound)");
            let (edges, unres) = wide(13, "{ t.save(x); }", "fn f(t: &dyn Store)");
            assert!(edges == 0 && unres, "13 impls must read Unknown, not resolve");
        }
    }

    #[test]
    fn return_type_inference_flows_through_local_factories() {
        // `let p = create_pool()?; p.fetch_one(q)` — create_pool's recorded return type lets p resolve.
        let uses = HashMap::new();
        let fields = FieldIndex::new();
        let mut returns = ReturnIndex::new();
        returns.insert("create_pool".to_string(), "sqlx::PgPool".to_string());
        let (ti, td, tf) = (TraitImplIndex::new(), HashMap::new(), TraitFieldIndex::new());
        let block: syn::Block =
            syn::parse_str("{ let p = create_pool()?; p.fetch_one(q); }").unwrap();
        let mut c = CallCollector {
            uses: &uses,
            vars: HashMap::new(),
            trait_vars: HashMap::new(),
            fields: &fields,
            trait_fields: &tf,
            trait_impls: &ti,
            local_traits: &td,
            returns: &returns,
            calls: Vec::new(),
            closure_vars: std::collections::HashSet::new(),
            unresolved: false,
        };
        for stmt in &block.stmts {
            c.visit_stmt(stmt);
        }
        let typed: Vec<&str> = c.calls.iter().map(|c| c.path.as_str()).collect();
        assert!(typed.contains(&"sqlx::PgPool::fetch_one"), "return-typed recv not resolved: {typed:?}");

        // A computed-callable invocation (a closure / fn-pointer the scan can't see through) marks the
        // function `unresolved` (→ honest `Unknown`), while a LOCAL closure whose body IS visible does
        // not — its effects were already walked lexically.
        let mk = |src: &str| {
            let blk: syn::Block = syn::parse_str(src).unwrap();
            let mut cc = CallCollector {
                uses: &uses,
                vars: HashMap::new(),
                trait_vars: HashMap::new(),
                fields: &fields,
                trait_fields: &tf,
                trait_impls: &ti,
                local_traits: &td,
                returns: &returns,
                calls: Vec::new(),
                closure_vars: std::collections::HashSet::new(),
                unresolved: false,
            };
            for stmt in &blk.stmts {
                cc.visit_stmt(stmt);
            }
            cc.unresolved
        };
        assert!(mk("{ (handlers[k])(); }"), "indexed callable must be unresolved");
        assert!(mk("{ (self.cb)(); }"), "fn-pointer field call must be unresolved");
        assert!(!mk("{ let g = |x: i32| x + 1; let _ = g(3); }"), "local closure body is visible — not unresolved");
        assert!(!mk("{ helper(); other::thing(); }"), "ordinary path calls are not unresolved");

        // unwrap_result_option peels Result/Option to the success type
        let r: syn::Type = syn::parse_str("std::io::Result<reqwest::Client>").unwrap();
        assert_eq!(type_path(unwrap_result_option(&r), &uses).as_deref(), Some("reqwest::Client"));
        let o: syn::Type = syn::parse_str("Option<PgPool>").unwrap();
        assert_eq!(type_path(unwrap_result_option(&o), &uses).as_deref(), Some("PgPool"));
    }

    #[test]
    fn test_file_stems_are_recognised() {
        assert!(is_test_file_stem("tests")); // src/foo/tests.rs
        assert!(is_test_file_stem("test"));
        assert!(is_test_file_stem("decoder_tests")); // base64's read/decoder_tests.rs
        assert!(is_test_file_stem("engine_test"));
        // legitimate non-test modules must NOT be excluded
        assert!(!is_test_file_stem("latest")); // not `_test`-suffixed (no underscore boundary)
        assert!(!is_test_file_stem("request"));
        assert!(!is_test_file_stem("contest"));
        assert!(!is_test_file_stem("lib"));
    }

    #[test]
    fn stable_policy_gate_evaluates_all_three_rule_kinds() {
        let all = vec!["api::handle".to_string(), "db::run".to_string(), "ui::draw".to_string()];
        let mut inferred: HashMap<String, BTreeSet<&'static str>> = HashMap::new();
        inferred.insert("api::handle".into(), ["Net"].into_iter().collect());
        inferred.insert("db::run".into(), ["Db"].into_iter().collect());
        let mut calls: HashMap<String, BTreeSet<String>> = HashMap::new();
        calls.insert("ui::draw".into(), ["db::run".to_string()].into_iter().collect());
        let mut hosts: HashMap<String, BTreeSet<String>> = HashMap::new();
        hosts.insert("api::handle".into(), ["evil.example.com".to_string()].into_iter().collect());
        let empty = HashMap::new();
        // deny fires on the transitive set; allow flags the out-of-list host; forbid sees ui -> db.
        let mut tables: HashMap<String, BTreeSet<String>> = HashMap::new();
        tables.insert("db::run".into(), ["audit.log".to_string()].into_iter().collect());
        // deny fires on the transitive set; allow flags the out-of-list host; forbid sees ui -> db.
        let v = policy_violations(
            "deny Net api\nallow Net in api good.example.com\nforbid ui -> db\n",
            &all, &inferred, &calls, &hosts, &empty, &empty, &tables,
        );
        assert_eq!(v.len(), 3, "{v:?}");
        assert!(v.iter().any(|l| l.contains("[AS-EFF-006]") && l.contains("api::handle")));
        assert!(v.iter().any(|l| l.contains("[AS-EFF-008]") && l.contains("evil.example.com")));
        assert!(v.iter().any(|l| l.contains("[AS-EFF-009]") && l.contains("ui::draw")));
        // clean policy -> no violations; `pure` flags ANY effect incl. the Db fn.
        assert!(policy_violations("deny Exec\n", &all, &inferred, &calls, &hosts, &empty, &empty, &tables).is_empty());
        assert_eq!(policy_violations("pure db\n", &all, &inferred, &calls, &hosts, &empty, &empty, &tables).len(), 1);
        // the Db table allowlist: db::run reaches audit.log — outside `ledger.*` -> violation;
        // covered by `audit.*` -> clean. ui::draw INHERITS Db but the literal propagation is the
        // caller's tablesacc, supplied here only for db::run, so only db::run flags.
        let bad = policy_violations("allow Db in db ledger.*\n", &all, &inferred, &calls, &hosts, &empty, &empty, &tables);
        assert_eq!(bad.len(), 1, "{bad:?}");
        assert!(bad[0].contains("audit.log"));
        assert!(policy_violations("allow Db in db audit.*\n", &all, &inferred, &calls, &hosts, &empty, &empty, &tables).is_empty());
    }

    #[test]
    fn only_root_build_rs_is_the_build_script() {
        use std::path::Path;
        // the Cargo build script — crate-root `build.rs` — IS skipped
        assert!(is_build_script(Path::new("build.rs")));
        // a nested `build.rs` is an ordinary source module and must NOT be skipped (the regression:
        // git2's `src/build.rs` is `RepoBuilder`, the whole clone/fetch network surface)
        assert!(!is_build_script(Path::new("src/build.rs")));
        assert!(!is_build_script(Path::new("src/foo/build.rs")));
        assert!(!is_build_script(Path::new("build/mod.rs"))); // a `build` module dir, not the script
    }

    #[test]
    fn cfg_test_modules_are_recognised() {
        let yes1: syn::ItemMod = syn::parse_str("#[cfg(test)] mod tests {}").unwrap();
        let yes2: syn::ItemMod =
            syn::parse_str("#[cfg(any(test, feature = \"x\"))] mod tests {}").unwrap();
        let no1: syn::ItemMod = syn::parse_str("#[cfg(feature = \"std\")] mod imp {}").unwrap();
        let no2: syn::ItemMod = syn::parse_str("mod real {}").unwrap();
        // deeper nesting positively requiring test → still skipped
        let yes3: syn::ItemMod =
            syn::parse_str("#[cfg(any(all(test, unix), windows))] mod t {}").unwrap();
        // `not(test)` is PRODUCTION code — must NOT be treated as a test module (the regression fix)
        let prod1: syn::ItemMod = syn::parse_str("#[cfg(not(test))] mod prod {}").unwrap();
        let prod2: syn::ItemMod = syn::parse_str("#[cfg(all(unix, not(test)))] mod prod {}").unwrap();
        assert!(is_cfg_test(&yes1.attrs));
        assert!(is_cfg_test(&yes2.attrs));
        assert!(is_cfg_test(&yes3.attrs));
        assert!(!is_cfg_test(&no1.attrs));
        assert!(!is_cfg_test(&no2.attrs));
        assert!(!is_cfg_test(&prod1.attrs), "cfg(not(test)) is production, not a test module");
        assert!(!is_cfg_test(&prod2.attrs), "cfg(all(unix, not(test))) is production");
    }

    #[test]
    fn expand_does_not_alias_a_crate_rooted_path() {
        // `crate::config::load` is explicitly crate-local; a `use other::config;` import must NOT hijack it.
        let u = uses(&[("config", "other::config")]);
        assert_eq!(expand("crate::config::load", &u), "config::load");
        assert_eq!(expand("self::config::load", &u), "config::load");
        // a NON-rooted bare `config::load` still expands via the use alias
        assert_eq!(expand("config::load", &u), "other::config::load");
    }

    #[test]
    fn ctor_type_rejects_a_module_path_receiver() {
        // `serde_json::from_str(s)` must NOT infer the MODULE `serde_json` as a type (lower-case receiver);
        // `reqwest::Client::new()` must still infer `reqwest::Client` (UpperCamel type receiver).
        let u = HashMap::new();
        let r = ReturnIndex::new();
        let modcall: syn::Expr = syn::parse_str("serde_json::from_str(s)").unwrap();
        assert_eq!(ctor_type(&modcall, &u, &r), None);
        let typecall: syn::Expr = syn::parse_str("reqwest::Client::new()").unwrap();
        assert_eq!(ctor_type(&typecall, &u, &r).as_deref(), Some("reqwest::Client"));
    }

    #[test]
    fn struct_literal_bindings_infer_their_type() {
        // `let s = S;` / `let s = S{..};` must type `s` so `s.go()` resolves (was the last named
        // receiver-inference gap: both read pure while `let s: S = S;` worked).
        let u = HashMap::new();
        let r = ReturnIndex::new();
        let t = |src: &str| ctor_type(&syn::parse_str::<syn::Expr>(src).unwrap(), &u, &r);
        assert_eq!(t("S").as_deref(), Some("S")); // unit-struct literal
        assert_eq!(t("S { a: 1 }").as_deref(), Some("S")); // struct literal
        assert_eq!(t("m::S { a: 1 }").as_deref(), Some("m::S")); // module-qualified
        assert_eq!(t("Color::Red").as_deref(), Some("Color")); // unit ENUM variant → the enum
        assert_eq!(t("Color::Red { x: 1 }").as_deref(), Some("Color")); // struct enum variant → the enum
        // negative gates: a variable copy and a SCREAMING_SNAKE const must NOT infer a type.
        assert_eq!(t("other_var"), None);
        assert_eq!(t("MAX_SIZE"), None);
        assert_eq!(t("config::MAX_SIZE"), None);
    }

    #[test]
    fn self_returning_ctor_types_the_local_and_the_edge_survives() {
        // The PROVE-IT-on-ureq miss: `fn new_with_defaults() -> Self` indexed the literal "Self", so
        // `let agent = Agent::new_with_defaults(); agent.run(..)` formed `Self::run` — no local def,
        // edge silently dropped, and the caller read pure though run() does I/O.
        let src = r#"
            pub struct Agent;
            impl Agent {
                pub fn new_with_defaults() -> Self { Agent }
                pub fn run(&self) { let _ = std::fs::read("/tmp/x"); }
            }
            pub fn top() { let agent = Agent::new_with_defaults(); agent.run() }
        "#;
        let file: syn::File = syn::parse_str(src).unwrap();
        let mut uses = HashMap::new();
        let mut fields: FieldIndex = HashMap::new();
        let mut rets: HashMap<String, Option<String>> = HashMap::new();
        let (mut ti, mut td, mut tf) = (TraitImplIndex::new(), HashMap::new(), TraitFieldIndex::new());
        collect_decls(&file.items, false, &mut uses, &mut fields, &mut rets, &mut ti, &mut td, &mut tf);
        assert_eq!(rets.get("new_with_defaults"), Some(&Some("Agent".to_string())),
                   "Self must resolve to the impl type, not the literal");
    }

    #[test]
    fn tuple_struct_fields_index_by_position() {
        // The other PROVE-IT miss: `self.0.0.run()` (ureq's ConfigBuilder newtype chain) — tuple
        // fields weren't in the FieldIndex, so the receiver never typed and the edge dropped.
        let src = r#"
            pub struct Inner;
            pub struct Outer(Inner);
            pub struct Stack(Outer);
        "#;
        let file: syn::File = syn::parse_str(src).unwrap();
        let mut uses = HashMap::new();
        let mut fields: FieldIndex = HashMap::new();
        let mut rets: HashMap<String, Option<String>> = HashMap::new();
        let (mut ti, mut td, mut tf) = (TraitImplIndex::new(), HashMap::new(), TraitFieldIndex::new());
        collect_decls(&file.items, false, &mut uses, &mut fields, &mut rets, &mut ti, &mut td, &mut tf);
        assert_eq!(fields["Outer"]["0"], "Inner");
        assert_eq!(fields["Stack"]["0"], "Outer");
    }

    #[test]
    fn embedded_agents_contract_matches_the_repo_doc() {
        // --agents prints the contract EMBEDDED at build time; this gate keeps the packaged copy
        // (crates/candor-scan/AGENTS.md, the only file a crates.io tarball can carry) in lockstep
        // with the repo-root AGENTS.md. If this fails: cp AGENTS.md crates/candor-scan/AGENTS.md
        let embedded = include_str!("../AGENTS.md");
        assert!(embedded.contains("candor-scan"), "the contract must describe this tool");
        // The repo-root doc exists ONLY in a workspace checkout. In a published-crate / `cargo
        // vendor` layout `../../AGENTS.md` is absent (panic) or an UNRELATED file (false diff), so
        // `cargo test` on the shipped crate would fail spuriously. Only assert the drift gate when
        // the root doc is actually present AND is candor's own (contains the marker) — otherwise
        // skip: the include_str above already proves the packaged copy compiles in.
        match std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/../../AGENTS.md")) {
            Ok(root) if root.contains("instructions for an AI coding agent") => {
                assert_eq!(embedded, root, "crate AGENTS.md drifted from the repo root — re-copy it");
            }
            _ => { /* not a workspace checkout (registry/vendor layout) — drift gate N/A */ }
        }
    }

    #[test]
    fn toml_primitives_tolerate_spacing_and_comments() {
        // The shared toml_section/toml_scalar fix a latent inconsistency: a `[ spaced ]` header and a
        // trailing `# comment` are now handled uniformly across all three manifest readers.
        assert_eq!(toml_section("[ workspace ]"), Some("workspace"));
        assert_eq!(toml_section("[package]"), Some("package"));
        assert_eq!(toml_section("name = \"x\""), None);
        assert_eq!(toml_scalar("name = \"my-crate\"  # the name", "name"), Some("my-crate"));
        assert_eq!(toml_scalar("name=bare # c", "name"), Some("bare"));
        assert_eq!(toml_scalar("namespace = \"x\"", "name"), None); // key is whole, not a prefix
        // read_crate_name through a spaced header + comment.
        let d = std::env::temp_dir().join(format!("candor-scan-tomlhdr-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        std::fs::write(d.join("Cargo.toml"), "[ package ]\nname = \"spaced-crate\"  # trailing\n").unwrap();
        assert_eq!(read_crate_name(&d).as_deref(), Some("spaced_crate"));
        // toml_string_array through a spaced [ workspace ] header.
        assert_eq!(
            toml_string_array("[ workspace ]\nmembers = [\"a\", \"b\"]\n", "workspace", "members"),
            vec!["a", "b"]
        );
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn cargo_deps_excludes_nested_package_manifests() {
        // The κ dep universe is the crate's OWN deps — a nested package (fixture, path-dep) is scanned
        // separately, so its deps must not pollute the parent's ledger (matching the source walk).
        let d = std::env::temp_dir().join(format!("candor-scan-nesteddeps-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(d.join("fixture")).unwrap();
        std::fs::write(d.join("Cargo.toml"), "[package]\nname = \"outer\"\n[dependencies]\nserde = \"1\"\n").unwrap();
        std::fs::write(d.join("fixture/Cargo.toml"), "[package]\nname = \"inner\"\n[dependencies]\nreqwest = \"0.12\"\n").unwrap();
        let (deps, _) = cargo_deps(&d.to_string_lossy());
        assert!(deps.contains("serde"), "the crate's own dep is present: {deps:?}");
        assert!(!deps.contains("reqwest"), "a nested package's dep leaked into the parent: {deps:?}");
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn toml_string_array_reads_inline_and_multiline_members() {
        let txt = "[package]\nname = \"x\"\n\n[workspace]\nmembers = [\"crates/a\", \"crates/b\"]\nexclude = [\n  \"eval\",\n  \"sample\",\n]\n";
        assert_eq!(toml_string_array(txt, "workspace", "members"), vec!["crates/a", "crates/b"]);
        assert_eq!(toml_string_array(txt, "workspace", "exclude"), vec!["eval", "sample"]);
        assert!(toml_string_array(txt, "workspace", "default-members").is_empty());
        assert!(toml_string_array("[dependencies]\nserde = \"1\"\n", "workspace", "members").is_empty());
    }

    #[test]
    fn workspace_members_expand_globs_and_honour_exclude() {
        let d = std::env::temp_dir().join(format!("candor-scan-ws-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        for m in ["crates/a", "crates/skipme", "tools/one", "crates/no-manifest"] {
            std::fs::create_dir_all(d.join(m)).unwrap();
            if m != "crates/no-manifest" {
                std::fs::write(d.join(m).join("Cargo.toml"), "[package]\nname = \"m\"\n").unwrap();
            }
        }
        std::fs::write(
            d.join("Cargo.toml"),
            "[workspace]\nmembers = [\"crates/*\", \"tools/one\", \"gone/away\"]\nexclude = [\"crates/skipme\"]\n",
        )
        .unwrap();
        let got: Vec<String> = workspace_members(&d)
            .into_iter()
            .map(|p| p.strip_prefix(&format!("{}/", d.to_string_lossy())).unwrap().to_string())
            .collect();
        assert_eq!(got, vec!["crates/a", "tools/one"], "glob expands, exclude + missing-manifest drop");
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn workspace_members_bare_star_and_dedup() {
        let d = std::env::temp_dir().join(format!("candor-scan-ws2-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        for m in ["a", "b"] {
            std::fs::create_dir_all(d.join(m)).unwrap();
            std::fs::write(d.join(m).join("Cargo.toml"), "[package]\nname = \"m\"\n").unwrap();
        }
        // bare `*` = immediate children; AND `a` listed explicitly too — must dedup, not double-scan.
        std::fs::write(d.join("Cargo.toml"), "[workspace]\nmembers = [\"*\", \"a\"]\n").unwrap();
        let got: Vec<String> = workspace_members(&d)
            .into_iter()
            .map(|p| p.strip_prefix(&format!("{}/", d.to_string_lossy())).unwrap().to_string())
            .collect();
        assert_eq!(got, vec!["a", "b"], "bare * expands to children, deduped against the explicit `a`");
        assert!(has_workspace_table(&d));
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn workspace_root_scans_members_even_under_deps_filter() {
        // The --deps × workspace regression: the nested-package filter prunes member dirs, so a
        // workspace root scanned as one crate yields an empty report. scan_target must fan out.
        let d = std::env::temp_dir().join(format!("candor-scan-wsfan-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        for (m, body) in [("a", "pub fn ea() { let _ = std::fs::read(\"/x\"); }"),
                          ("b", "pub fn eb() { let _ = std::process::Command::new(\"x\"); }")] {
            std::fs::create_dir_all(d.join(m).join("src")).unwrap();
            std::fs::write(d.join(m).join("Cargo.toml"), format!("[package]\nname = \"{m}\"\n")).unwrap();
            std::fs::write(d.join(m).join("src/lib.rs"), body).unwrap();
        }
        std::fs::write(d.join("Cargo.toml"), "[workspace]\nmembers = [\"a\", \"b\"]\n").unwrap();
        let idx = load_dep_reports(None);
        let prefix = d.join("out/r").to_string_lossy().into_owned();
        let rc = scan_target(&d.to_string_lossy(), prefix.clone(), false, false, None, &idx);
        assert_eq!(rc, 0);
        let ra = std::fs::read_to_string(format!("{prefix}.a.scan.json")).unwrap();
        let rb = std::fs::read_to_string(format!("{prefix}.b.scan.json")).unwrap();
        assert!(ra.contains("ea") && ra.contains("Fs"), "member a not scanned: {ra}");
        assert!(rb.contains("eb") && rb.contains("Exec"), "member b not scanned: {rb}");
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn nested_packages_are_not_modules_of_the_parent() {
        // The repo-root self-scan merged 194 eval-fixture `main`s into ONE unit (un-namespaced
        // collision -> cross-wired inheritance). A subtree with its own Cargo.toml is a different
        // package: the parent's walk must not descend into it.
        let d = std::env::temp_dir().join(format!("candor-scan-nested-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(d.join("src")).unwrap();
        std::fs::create_dir_all(d.join("fixture/src")).unwrap();
        std::fs::write(d.join("Cargo.toml"), "[package]\nname = \"outer\"\n").unwrap();
        std::fs::write(d.join("src/lib.rs"), "pub fn outer_eff() { let _ = std::fs::read(\"/x\"); }\n").unwrap();
        std::fs::write(d.join("fixture/Cargo.toml"), "[package]\nname = \"inner\"\n").unwrap();
        std::fs::write(
            d.join("fixture/src/lib.rs"),
            "pub fn inner_eff() { let _ = std::process::Command::new(\"x\"); }\n",
        )
        .unwrap();
        let idx = load_dep_reports(None);
        let prefix = d.join("out/r").to_string_lossy().into_owned();
        let (rc, _) = scan_one(&d.to_string_lossy(), ScanOpts {
            prefix: prefix.clone(), want_json: false, include_tests: false,
            policy: None, quiet: true, deps_idx: &idx,
        });
        assert_eq!(rc, 0);
        let rep = std::fs::read_to_string(format!("{prefix}.outer.scan.json")).unwrap();
        assert!(rep.contains("outer_eff"), "the parent's own fn must report: {rep}");
        assert!(!rep.contains("inner_eff") && !rep.contains("Exec"),
                "nested package's fn leaked into the parent report: {rep}");
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn report_is_written_atomically_no_tmp_leftovers() {
        // A concurrent `candor-query` / `cargo candor watch` reader must never observe a half-written
        // report (write_atomic: temp + rename). We assert the rename discipline by its observable
        // effect: the scan leaves NO `.tmp.*` file behind, and both written files are WHOLE valid JSON.
        let d = std::env::temp_dir().join(format!("candor-scan-atomic-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(d.join("src")).unwrap();
        std::fs::write(d.join("Cargo.toml"), "[package]\nname = \"atomiccrate\"\n").unwrap();
        std::fs::write(d.join("src/lib.rs"), "pub fn eff() { let _ = std::fs::read(\"/x\"); }\n").unwrap();
        let idx = load_dep_reports(None);
        let outdir = d.join("out");
        let prefix = outdir.join("r").to_string_lossy().into_owned();
        let (rc, _) = scan_one(&d.to_string_lossy(), ScanOpts {
            prefix, want_json: false, include_tests: false, policy: None, quiet: true, deps_idx: &idx,
        });
        assert_eq!(rc, 0);
        // no temp turds: every output file ends in `.json`, never `.tmp.<pid>`.
        let leftovers: Vec<String> = std::fs::read_dir(&outdir).unwrap()
            .filter_map(|e| e.ok().map(|e| e.file_name().to_string_lossy().into_owned()))
            .filter(|n| n.contains(".tmp")).collect();
        assert!(leftovers.is_empty(), "atomic write left temp files behind: {leftovers:?}");
        // both files parse as a whole — the post-rename invariant a concurrent reader relies on.
        for name in ["r.atomiccrate.scan.json", "r.atomiccrate.scan.callgraph.json"] {
            let body = std::fs::read_to_string(outdir.join(name)).unwrap();
            serde_json::from_str::<serde_json::Value>(&body)
                .unwrap_or_else(|e| panic!("{name} is not whole JSON ({e}): {body}"));
        }
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn classifier_resolves_a_std_fs_call() {
        // guards the shared-classifier contract the scanner relies on: an expanded std::fs path is Fs.
        assert_eq!(candor_classify::classify("std", "std::fs::read_to_string"), Some("Fs"));
        assert_eq!(candor_classify::classify("std", "std::process::Command::new"), Some("Exec"));
    }

    #[test]
    fn resolve_target_is_precise_and_never_fabricates() {
        // Exercises the PRODUCTION `resolve_target` (not a copy) so a regression in `run`'s resolution is
        // caught here. Defs: a unique `bool` free fn, a unique `start` method, a unique `Worker::run`
        // method, and TWO same-named `Job::run` methods in different modules (an ambiguous 2-segment tail).
        let mut by_leaf: HashMap<String, Vec<String>> = HashMap::new();
        let mut by_tail2: HashMap<String, Vec<String>> = HashMap::new();
        for q in ["random::bool::bool", "clip::ClipboardThread::start", "util::helper",
                  "app::Worker::run", "a::Job::run", "b::Job::run"] {
            by_leaf.entry(q.rsplit("::").next().unwrap().into()).or_default().push(q.into());
            by_tail2.entry(tail2(q).unwrap()).or_default().push(q.into());
        }
        // (a) qualified `Value::bool(..)` — external `Value`, tail absent locally → NONE (never the
        // unique-leaf `random::bool::bool`; the original nushell Rand-on-146-fns fabrication).
        assert_eq!(resolve_target("Value::bool", "bool", false, &by_tail2, &by_leaf), None);
        // (b) unresolved-receiver method `range.start()` → NONE (never the unique `ClipboardThread::start`).
        assert_eq!(resolve_target("start", "start", true, &by_tail2, &by_leaf), None);
        // (c) unqualified free call `helper()` with a unique def → resolves.
        assert_eq!(resolve_target("helper", "helper", false, &by_tail2, &by_leaf),
                   Some(&vec!["util::helper".to_string()]));
        // (d) associated-fn call `Worker::run()` (qualified, unique tail) → resolves to the one local def.
        assert_eq!(resolve_target("Worker::run", "run", false, &by_tail2, &by_leaf),
                   Some(&vec!["app::Worker::run".to_string()]));
        // (e) AMBIGUOUS tail `Job::run` (two types, two modules) → NONE: linking both would fabricate one
        // type's effect onto the other's caller (the bug the `len()==1` filter on the tail2 branch fixes).
        assert_eq!(resolve_target("Job::run", "run", false, &by_tail2, &by_leaf), None);
    }
}
