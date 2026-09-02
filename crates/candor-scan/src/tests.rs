//! The scanner's unit tests — the former in-file `#[cfg(test)] mod tests` of main.rs,
//! now a file module (`super::*` still resolves to the crate root). Original indentation
//! kept verbatim: several tests embed column-sensitive source strings.

    use super::*;

    fn uses(pairs: &[(&str, &str)]) -> HashMap<String, String> {
        pairs.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect()
    }

    /// ⟨0.24⟩ `policy_violations` returns a `GateOutcome` — the violations AND the `(rule, function)`
    /// pairs the gate WITHHELD (SPEC §3.1). These older unit tests assert over the violations half; the
    /// tests that are ABOUT withholding call `policy_violations` directly and name both.
    ///
    /// A named shim rather than a `.violations` at each of eleven call sites, so that "this assertion
    /// ignores the withheld half" is visible in the call and not buried in a field access.
    #[allow(clippy::too_many_arguments)]
    fn gate_violations(
        policy_text: &str,
        all: &[String],
        inferred: &HashMap<String, BTreeSet<&'static str>>,
        calls: &HashMap<String, BTreeSet<String>>,
        hostsacc: &HashMap<String, BTreeSet<String>>,
        cmdsacc: &HashMap<String, BTreeSet<String>>,
        pathsacc: &HashMap<String, BTreeSet<String>>,
        tablesacc: &HashMap<String, BTreeSet<String>>,
        incompleteacc: &HashMap<String, BTreeSet<&'static str>>,
        reasonclassacc: &HashMap<String, BTreeSet<String>>,
        unknown_aliases: &std::collections::BTreeMap<String, BTreeSet<candor_classify::policy::ReasonClass>>,
        net_partners: &BTreeSet<String>,
    ) -> Vec<GateViolation> {
        // ⟨0.32⟩ a fixed crate name: these unit tests assert on the FINDINGS, and the `hash` a verdict
        // row now carries is `<crate>#<qual>`. The CLI rows pin the real value on both routes.
        policy_violations(
            policy_text, "unit", all, inferred, calls, hostsacc, cmdsacc, pathsacc, tablesacc, incompleteacc,
            reasonclassacc, unknown_aliases, net_partners,
        )
        .violations
    }

    /// A shared, empty lazy-static name set for direct `CallCollector` constructions in unit tests that
    /// don't exercise the lazy-forcing path (the lazy-forcing tests use the full `scan_one`/`scan_src`).
    fn empty_lazy() -> &'static std::collections::HashSet<String> {
        static EMPTY: std::sync::OnceLock<std::collections::HashSet<String>> = std::sync::OnceLock::new();
        EMPTY.get_or_init(std::collections::HashSet::new)
    }
    /// An empty crate-const-string index, for the direct-`CallCollector` construction tests that don't
    /// exercise const-propagation (those use the full `scan_src` path).
    fn empty_consts() -> &'static std::collections::HashMap<String, String> {
        static EMPTY: std::sync::OnceLock<std::collections::HashMap<String, String>> = std::sync::OnceLock::new();
        EMPTY.get_or_init(std::collections::HashMap::new)
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
    fn glob_reexport_records_external_prelude_and_resolves_a_reexported_name() {
        // A `use x::prelude::*` glob records the EXTERNAL prelude PATH under `GLOB_KEY`; a `use crate::net`
        // re-bind in the same scope then resolves `net` THROUGH that glob to its origin crate — so a later
        // `net::connect(..)` attributes to `x`, not the dead `crate::net` (the cardinal-sin hole).
        let mut u = HashMap::new();
        collect_use(&syn::parse_str("mycore::driver_prelude::*").unwrap(), String::new(), &mut u);
        assert_eq!(u.get(GLOB_KEY).map(String::as_str), Some("mycore::driver_prelude"));
        collect_use(&syn::parse_str("crate::net").unwrap(), String::new(), &mut u);
        assert_eq!(u.get("net").map(String::as_str), Some("mycore::driver_prelude::net"));
        assert_eq!(expand("net::connect_tcp", &u), "mycore::driver_prelude::net::connect_tcp");
        // TWO external globs → ambiguous origin → never guess: the re-bind keeps the literal (under-report).
        let mut u2 = HashMap::new();
        collect_use(&syn::parse_str("a::p1::*").unwrap(), String::new(), &mut u2);
        collect_use(&syn::parse_str("b::p2::*").unwrap(), String::new(), &mut u2);
        collect_use(&syn::parse_str("crate::net").unwrap(), String::new(), &mut u2);
        assert_eq!(u2.get("net").map(String::as_str), Some("crate::net"));
    }

    #[test]
    fn crate_rooted_call_resolves_through_seeded_root_reexport() {
        // A `crate::net::foo` in ANY file resolves through the crate-ROOT re-exports seeded under
        // `crate::<name>` (a DIRECT `pub use x::net`) or `crate::` + GLOB_KEY (a `pub use x::…::*` glob).
        let mut direct = seed_root_reexports(&collect_root_reexports(&syn::parse_file(
            "pub use mycore::net;").unwrap().items));
        assert_eq!(expand("crate::net::connect_tcp", &direct), "mycore::net::connect_tcp");
        // a `use super::core::foo` re-bind is RELATIVE (not crate-root) and must keep its literal so the
        // local `core::foo` def still resolves by tail2 — seeding a root re-export must not hijack it.
        collect_use(&syn::parse_str("super::core::display_width").unwrap(), String::new(), &mut direct);
        assert_eq!(direct.get("display_width").map(String::as_str), Some("super::core::display_width"));

        let glob = seed_root_reexports(&collect_root_reexports(&syn::parse_file(
            "pub use mycore::driver_prelude::*;").unwrap().items));
        assert_eq!(expand("crate::net::connect_tcp", &glob), "mycore::driver_prelude::net::connect_tcp");
    }

    #[test]
    fn genuinely_local_module_is_not_glob_hijacked() {
        // ATTRIBUTION, not suspicion: a crate with a re-export glob must NOT rewrite a BARE call into a
        // real external crate (`dotenvy::var` stays `dotenvy::*`, not `glob::dotenvy::var`) — else the
        // classifier loses its crate identity (the sqlx `dotenvy::var` → dropped `Env` regression). Only a
        // `use`-imported name (resolved at collect time) or a `crate::`-rooted call is glob-attributed.
        let mut u = HashMap::new();
        collect_use(&syn::parse_str("mycore::prelude::*").unwrap(), String::new(), &mut u);
        assert_eq!(expand("dotenvy::var", &u), "dotenvy::var");
        // A crate WITHOUT any re-export glob leaves a `crate::helper::foo` local path untouched — no glob to
        // attribute to, so a genuine local module keeps its path (and resolves locally by tail2).
        let mut noglob = HashMap::new();
        collect_use(&syn::parse_str("std::fs::read").unwrap(), String::new(), &mut noglob);
        assert_eq!(expand("crate::helper::foo", &noglob), "helper::foo");
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
    fn literal_head_host_extracts_only_a_terminated_authority() {
        // POSITIVE: a complete authority in the static prefix (terminated by a `/` before the first hole).
        assert_eq!(literal_head_host("https://api.openai.com/v1/{}").as_deref(), Some("api.openai.com"));
        assert_eq!(literal_head_host("https://api.openai.com/{}").as_deref(), Some("api.openai.com"));
        // NEGATIVE: a hole is (or could be) inside the authority — no `/` after `://` in the prefix.
        assert_eq!(literal_head_host("https://api.{}.com/v1/y"), None);
        assert_eq!(literal_head_host("https://{}/v1/y"), None);
        assert_eq!(literal_head_host("https://api.openai{}/v1"), None);
        assert_eq!(literal_head_host("https://api.openai.com:{}/v1"), None); // port hole before the `/`
        // The leading-`{}` const-anchored shape has NO static prefix → defer (handled elsewhere).
        assert_eq!(literal_head_host("{}/chat"), None);
        // `:port` in a fully-literal authority is stripped, matching the routable-host convention.
        assert_eq!(literal_head_host("http://host.example:8080/{}").as_deref(), Some("host.example"));
        // `{{` is an escaped brace, not a hole — the authority is still terminated after it.
        assert_eq!(literal_head_host("https://api.openai.com/a{{b}}/{}").as_deref(), Some("api.openai.com"));
        // Not a URL at all → None.
        assert_eq!(literal_head_host("plain text {}"), None);
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
        let (fe, ev, evt) = (FieldElemIndex::new(), EnumVariantIndex::new(), EnumVariantTraitIndex::new());
        let fet = FieldElemTraitIndex::new();
        let mut c = CallCollector {
            modpath: String::new(),
            uses: &uses,
            vars: HashMap::new(),
            trait_vars: HashMap::new(),
            dyn_sig_traits: Default::default(), generic_bounds: Default::default(), trait_quals: Default::default(), trait_quals_by_param: Default::default(),
            fields: &fields,
            trait_fields: &tf,
            trait_impls: &ti,
            local_traits: &td,
            returns: &returns,
            has_dyn_return: false,
            field_elem: &fe, field_elem_trait: &fet,
            enum_variants: &ev, enum_variant_traits: &evt, ambiguous_enum_leaves: &std::collections::HashSet::new(), callable_statics: &std::collections::HashSet::new(),
            elem_of: HashMap::new(), elem_trait_of: HashMap::new(), tuple_of: HashMap::new(), tuple_trait_of: std::collections::HashMap::new(),
            calls: Vec::new(),
            closure_vars: std::collections::HashSet::new(),
            fn_typed_vars: std::collections::HashSet::new(), dep_bound_vars: std::collections::HashMap::new(),
            fn_alias: std::collections::HashMap::new(),
            lazy_statics: empty_lazy(),
            forced_lazies: std::collections::HashSet::new(),
            unresolved: false,
            err_ret_leaf: None,
            const_strings: empty_consts(), local_macros: empty_consts(), macro_expanding: std::collections::HashSet::new(), str_locals: std::collections::HashMap::new(), local_uses: std::collections::HashMap::new(), bound_names: std::collections::HashSet::new(), dispatch_sites: Default::default(), drop_relevant: &std::collections::HashSet::new(), escaping_ctors: Default::default(), marked_ctors: Default::default(), marked_cross_ctors: Default::default(), in_pattern: false,
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
        let (fe, ev, evt) = (FieldElemIndex::new(), EnumVariantIndex::new(), EnumVariantTraitIndex::new());
        let fet = FieldElemTraitIndex::new();
        let block: syn::Block =
            syn::parse_str("{ client.get(url).send(); self.http.execute(req); }").unwrap();
        let mut c = CallCollector {
            modpath: String::new(),
            uses: &uses,
            vars,
            trait_vars: HashMap::new(),
            dyn_sig_traits: Default::default(), generic_bounds: Default::default(), trait_quals: Default::default(), trait_quals_by_param: Default::default(),
            fields: &fields,
            trait_fields: &tf,
            trait_impls: &ti,
            local_traits: &td,
            returns: &returns,
            has_dyn_return: false,
            field_elem: &fe, field_elem_trait: &fet,
            enum_variants: &ev, enum_variant_traits: &evt, ambiguous_enum_leaves: &std::collections::HashSet::new(), callable_statics: &std::collections::HashSet::new(),
            elem_of: HashMap::new(), elem_trait_of: HashMap::new(), tuple_of: HashMap::new(), tuple_trait_of: std::collections::HashMap::new(),
            calls: Vec::new(),
            closure_vars: std::collections::HashSet::new(),
            fn_typed_vars: std::collections::HashSet::new(), dep_bound_vars: std::collections::HashMap::new(),
            fn_alias: std::collections::HashMap::new(),
            lazy_statics: empty_lazy(),
            forced_lazies: std::collections::HashSet::new(),
            unresolved: false,
            err_ret_leaf: None,
            const_strings: empty_consts(), local_macros: empty_consts(), macro_expanding: std::collections::HashSet::new(), str_locals: std::collections::HashMap::new(), local_uses: std::collections::HashMap::new(), bound_names: std::collections::HashSet::new(), dispatch_sites: Default::default(), drop_relevant: &std::collections::HashSet::new(), escaping_ctors: Default::default(), marked_ctors: Default::default(), marked_cross_ctors: Default::default(), in_pattern: false,
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
        assert!(!ren3.contains_key("my_package"), "key-substring 'package' fabricated a rename: {ren3:?}");
        assert!(!ren3.contains_key("foo_package"), "{ren3:?}");
        assert!(!ren3.contains_key("package"), "a dep named `package` is not a rename: {ren3:?}");
        assert_eq!(ren3.get("real").map(String::as_str), Some("renamed_crate"), "real rename lost: {ren3:?}");
    }

    /// ⟨renamed-dep classify() alias⟩ SOUNDNESS finding, end to end (unlike the unit-level
    /// `cargo_toml_deps_handles_all_header_forms` above, which only proves the rename MAP is built
    /// correctly — nothing previously proved a renamed call site actually got CLASSIFIED through it).
    /// `[dependencies] alias = { package = "real" }` means the source spells `alias::…`, but `classify()`'s
    /// rule tables — including a rule keyed on a FULL EXACT path, `git2`'s `Repository::clone` — are
    /// written against `real`. Before the fix this fell all the way through to the honest
    /// `invisible`-qualified floor: disclosed, not silent (never a false purity claim), but a `deny`
    /// gate does not act on `invisible`, so a KNOWN, calibrated effect silently lost its gate coverage
    /// purely because of a manifest rename — the byte-identical unaliased call already gated correctly.
    /// rust-deep is unaffected by this class (`cx.tcx.crate_name` resolves the compiled crate's true
    /// identity, independent of any local extern alias) — verified separately via `cargo dylint` against
    /// the identical renamed-reqwest shape.
    #[test]
    fn renamed_dependency_calls_classify_under_the_real_package_name() {
        let build = |manifest: &str, src: &str| -> serde_json::Value {
            let d = std::env::temp_dir().join(format!(
                "candor-rename-classify-{}-{}", manifest.len() ^ src.len(), std::process::id()
            ));
            let _ = std::fs::remove_dir_all(&d);
            std::fs::create_dir_all(d.join("src")).unwrap();
            std::fs::write(d.join("Cargo.toml"), format!(
                "[package]\nname = \"victim\"\nversion = \"0.1.0\"\n[dependencies]\n{manifest}\n"
            )).unwrap();
            std::fs::write(d.join("src/lib.rs"), src).unwrap();
            let idx = DepIndex::default();
            let (rc, body) = scan_one(&d.to_string_lossy(), ScanOpts {
                prefix: String::new(), want_json: true, include_tests: false, policy: None,
                baseline: None, ws_member: false, quiet: true, deps_idx: &idx, peek_excluded: false,
            }, &crate::gate::begin_run());
            assert_eq!(rc, 0, "scan should succeed:\n{body:?}");
            let v = serde_json::from_str(&body.unwrap()).unwrap();
            let _ = std::fs::remove_dir_all(&d);
            v
        };
        // A SUFFIX-matched rule (reqwest's Net classification keys on the receiver type + a terminal
        // verb) through a renamed dependency.
        let aliased = build(
            "htclient = { version = \"0.11\", package = \"reqwest\" }",
            "pub fn fetch(u: &str) -> String { htclient::blocking::get(u).unwrap().text().unwrap() }\n",
        );
        assert_eq!(effs(fn_entry(&aliased, "fetch")), vec!["Net".to_string()],
            "a renamed reqwest call must classify Net exactly like the unaliased call — got:\n{aliased:#}");
        assert!(fn_entry(&aliased, "fetch").get("invisible").is_none(),
            "a correctly-classified renamed call must not ALSO carry the uncalibrated-floor `invisible` \
             qualifier — that's reserved for calls classify() genuinely can't resolve:\n{aliased:#}");
        // The identical call, unaliased — proves the rename is the only variable and the two must agree.
        let unaliased = build(
            "reqwest = { version = \"0.11\" }",
            "pub fn fetch(u: &str) -> String { reqwest::blocking::get(u).unwrap().text().unwrap() }\n",
        );
        assert_eq!(aliased["functions"], unaliased["functions"],
            "renamed and unaliased reports must be byte-identical in their effect fields:\naliased:\
             {aliased:#}\nunaliased:{unaliased:#}");
        // A FULL-EXACT-PATH rule (`git2`'s `path == "git2::Repository::clone"`, chosen specifically because
        // it cannot be satisfied by crate_name alone — it also requires the PATH's own leading segment to
        // read the real name, proving the fix rewrites the path, not just the crate_name argument).
        let git_aliased = build(
            "vcs = { version = \"0.18\", package = \"git2\" }",
            "pub fn clone_it(u: &str, p: &str) { vcs::Repository::clone(u, p).unwrap(); }\n",
        );
        assert_eq!(effs(fn_entry(&git_aliased, "clone_it")), vec!["Net".to_string()],
            "a renamed git2 must still hit the FULL-PATH-keyed `Repository::clone` rule:\n{git_aliased:#}");
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
        assert_eq!(post.effects, BTreeSet::from(["Db"]));
        assert_eq!(post.tables, BTreeSet::from(["ledger.entries".to_string()]));
        assert!(idx.by_key.contains_key("billing#post"), "unambiguous leaf key");
        // A SHARED LEAF UNIONS ITS CANDIDATES rather than being dropped. Dropping it took the CALLER out
        // of `functions` — a ⟨0.21⟩ purity claim over a call that may reach either body. `a::dup` does
        // `Net` and `b::dup` does `Fs`, so a call the consumer cannot resolve to one of them may do both.
        assert_eq!(idx.by_key.get("billing#dup").map(|e| e.effects.clone()),
                   Some(BTreeSet::from(["Fs", "Net"])),
                   "a shared leaf must union — never guess, but never go silent either");
        assert_eq!(idx.by_key.get("billing#a::dup").map(|e| e.effects.clone()),
                   Some(BTreeSet::from(["Net"])),
                   "tail2 still disambiguates the dups — the union must not reach a key that does not collide");
        let old = idx.by_key.get("old_dep#go").expect("stale entry present");
        assert_eq!(old.effects, BTreeSet::from(["Unknown"]), "stale version must downgrade to Unknown");
        let _ = std::fs::remove_dir_all(&d);
    }

    /// SPEC §3.4 SAYS WHITESPACE, AND THIS ENGINE SPLIT ON `:` ALONE. A two-path `CANDOR_DEPS` therefore
    /// arrived as ONE token, matched no file, and rust chained NOTHING — while printing "entry not found,
    /// skipped" and the ordinary uncovered hedge, so the report was indistinguishable from one produced
    /// with no dep reports at all.
    ///
    /// FOUND THROUGH A WAIVER THAT NAMED THE WRONG CAUSE, which is the durable part. Conformance PART 26's
    /// `stale_beside` arm passes `"<trusted> <stale>"`, and rust's baseline waiver for it read "the key is
    /// withdrawn, the effect is gone and the package is re-declared uncovered" — a precise description of a
    /// mechanism that was not running. That arm had been measuring rust-with-nothing-chained since it was
    /// written, and the waiver made the reading look diagnosed rather than unexamined.
    ///
    /// All three separators are asserted because supporting one is what caused this: colon and comma are
    /// SUPERSETS of the spec (candor-java has documented `space/colon/comma` since its loader was written),
    /// and every existing rust fixture plus this engine's own `--deps` output uses colon.
    #[test]
    fn candor_deps_splits_on_whitespace_as_well_as_colon_and_comma() {
        let d = std::env::temp_dir().join(format!("candor-depsep-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        let me = format!("scan-{}", env!("CARGO_PKG_VERSION"));
        for (file, eff) in [("a.alib.scan.json", "Exec"), ("b.blib.scan.json", "Net")] {
            let pkg = if eff == "Exec" { "alib" } else { "blib" };
            std::fs::write(d.join(file), format!(r#"{{
                "candor": {{"version": "{me}", "toolchain": "s", "spec": "0.26"}},
                "package": "{pkg}",
                "functions": [{{"fn": "go", "inferred": ["{eff}"], "hash": "{pkg}#go"}}]}}"#)).unwrap();
        }
        let a = d.join("a.alib.scan.json"); let b = d.join("b.blib.scan.json");
        let (a, b) = (a.to_str().unwrap(), b.to_str().unwrap());
        for (label, spec) in [("space", format!("{a} {b}")),
                              ("colon", format!("{a}:{b}")),
                              ("comma", format!("{a},{b}")),
                              ("tab",   format!("{a}\t{b}")),
                              ("mixed", format!("{a} , {b}"))] {
            let idx = load_dep_reports(Some(&spec));
            assert_eq!(idx.by_key.get("alib#go").map(|e| e.effects.clone()), Some(BTreeSet::from(["Exec"])),
                       "{label}-separated CANDOR_DEPS did not chain the FIRST report");
            assert_eq!(idx.by_key.get("blib#go").map(|e| e.effects.clone()), Some(BTreeSet::from(["Net"])),
                       "{label}-separated CANDOR_DEPS did not chain the SECOND report — a spec-conforming \
                        two-path spec must not silently resolve to zero reports");
        }
        let _ = std::fs::remove_dir_all(&d);
    }

    /// The shared source for the binder-scoping pair below: a dispatch trait with ONE effectful impl,
    /// so anything that wrongly resolves a shadowed name through `trait_vars` shows up as `Fs`.
    #[cfg(test)]
    const SHADOW_SRC: &str = "\
pub trait Store { fn go(&self); }
pub struct DbStore;
impl Store for DbStore { fn go(&self) { let _ = std::fs::read_to_string(\"/effectful\"); } }
pub fn helper<F: Fn()>(f: F) { f() }
fn opaque() -> Vec<u8> { Vec::new() }

pub fn control(s: &dyn Store) { s.go(); }

// The shadow binds a name the scan CANNOT type, so nothing is written to `vars` to mask a stale
// `trait_vars` entry. Three binder forms.
pub fn shadow_for(s: &dyn Store) { let _ = s; for s in opaque() { s.go(); } }
pub fn shadow_range(s: &dyn Store) { let _ = s; for s in 0..3 { s.go(); } }
pub fn shadow_adapter(s: &dyn Store) { let _ = s; opaque().iter().for_each(|s| s.go()); }

// The SECOND DIRECTION: the same positions with NO shadow. If these stop reading `Fs`, the fix
// above bought its silence by killing a real reach — which is the whole hazard of a fabrication fix.
pub fn live_adapter(s: &dyn Store) { opaque().iter().for_each(|_p| s.go()); }
pub fn live_closure(s: &dyn Store) { helper(|| s.go()); }
pub fn live_for(s: &dyn Store) { for _p in opaque() { s.go(); } }
pub fn live_block(s: &dyn Store) { { s.go(); } }
pub fn live_iflet(s: &dyn Store, o: Option<u8>) { if let Some(_x) = o { s.go(); } }
pub fn live_whilelet(s: &dyn Store, mut o: Option<u8>) { while let Some(_x) = o { s.go(); o = None; } }
pub fn live_letelse(s: &dyn Store, o: Option<u8>) { let Some(_x) = o else { return }; s.go(); }
pub fn live_loop(s: &dyn Store) { 'l: loop { s.go(); break 'l; } }
pub fn live_nested_block(s: &dyn Store) { { { { s.go(); } } } }
";

    #[test]
    fn a_shadow_the_scan_cannot_type_does_not_inherit_the_outer_dispatch_binding() {
        // A NAME-KEYED SIDE TABLE THAT OUTLIVES ITS SCOPE — candor-swift's `71de627`/`83cd607`/`42093b6`,
        // found here in `trait_vars`. `for s in 0..3 { s.go(); }` inside `fn f(s: &dyn Store)` charged `f`
        // with the `Fs` of `impl Store for DbStore`, on a loop variable that is a `u8`.
        //
        // WHAT MADE IT INVISIBLE is worth more than the fix: `scoped_var` did clear `vars`, and `vars` is
        // consulted BEFORE `trait_vars` — so every shadow that resolves to a concrete type is masked by a
        // fresh binding and behaves perfectly. Only a shadow the scan cannot type leaves `trait_vars` as
        // the one map still answering for the name. The guard was right for the wrong reason (standing
        // bar item 0b), and a fixture using a typed shadow is structurally incapable of noticing.
        let v = scan_src_to_json("shadowbind", SHADOW_SRC);
        let eff = |n: &str| -> Vec<String> {
            v["functions"].as_array().into_iter().flatten()
                .find(|f| f["fn"].as_str() == Some(n))
                .and_then(|f| f["inferred"].as_array().cloned()).unwrap_or_default()
                .iter().filter_map(|x| x.as_str().map(String::from)).collect()
        };
        assert!(eff("control").contains(&"Fs".to_string()),
                "the positive control lost its dispatch — this fixture proves nothing\n{v:#}");
        for shadow in ["shadow_for", "shadow_range", "shadow_adapter"] {
            assert!(!eff(shadow).contains(&"Fs".to_string()),
                    "`{shadow}` inherited the OUTER `&dyn Store` binding through a shadow the scan \
                     cannot type, and fabricated an effect on a pure function\n{v:#}");
        }
    }

    #[test]
    fn scoping_the_binder_did_not_kill_the_dispatch_it_scopes() {
        // THE SECOND FIXTURE, and per standing bar item 0 it is the one that matters: a fabrication fix
        // narrows an over-approximation, and narrowing past the real reaches is how four of five fixes in
        // this vein went wrong. `scoped_binding` now clears TEN name-keyed maps at every binder and the
        // four dispatch binders route through it, so every position where a trait-typed receiver is used
        // INSIDE a scope is exercised here with no shadow present. Each must still read `Fs`.
        let v = scan_src_to_json("shadowlive", SHADOW_SRC);
        let eff = |n: &str| -> Vec<String> {
            v["functions"].as_array().into_iter().flatten()
                .find(|f| f["fn"].as_str() == Some(n))
                .and_then(|f| f["inferred"].as_array().cloned()).unwrap_or_default()
                .iter().filter_map(|x| x.as_str().map(String::from)).collect()
        };
        for live in ["control", "live_adapter", "live_closure", "live_for", "live_block", "live_iflet",
                     "live_whilelet", "live_letelse", "live_loop", "live_nested_block"] {
            assert!(eff(live).contains(&"Fs".to_string()),
                    "`{live}` LOST its dispatch reach — the binder scoping narrowed past a real call\n{v:#}");
        }
    }

    #[test]
    fn every_name_keyed_table_is_scoped_by_the_one_binder() {
        // The swift lesson, ported: stop enumerating scope forms at each call site and make the ONE binder
        // responsible, then pin the enumeration so the next side table cannot be forgotten. `scoped_binding`
        // is exercised directly here rather than through a fixture, because a fixture can only witness the
        // tables some shape happens to reach — and the whole defect was a table no shape reached.
        //
        // The split is by ROLE and is asserted, not commented: a RESOLUTION table (answers "what does this
        // name refer to") must be cleared inside the body, since a stale entry fabricates; a HEDGING set
        // (`closure_vars`, `fn_typed_vars`, which only ever suppress a phantom call or raise `Unknown`)
        // must be KEPT, since clearing it would let a shadowed `name()` resolve to a free fn of that name —
        // the fabrication mirror of the bug being fixed.
        let uses = HashMap::new();
        let fields = FieldIndex::new();
        let trait_fields = TraitFieldIndex::new();
        let trait_impls = TraitImplIndex::new();
        let local_traits = HashMap::new();
        let returns = ReturnIndex::new();
        let field_elem = FieldElemIndex::new();
        let field_elem_trait = FieldElemTraitIndex::new();
        let enum_variants = EnumVariantIndex::new();
        let enum_variant_traits = EnumVariantTraitIndex::new();
        let lazy = std::collections::HashSet::new();
        let consts = std::collections::HashMap::new();
        let macros = std::collections::HashMap::new();
        let mut c = CallCollector {
            modpath: String::new(), uses: &uses, vars: HashMap::new(), trait_vars: HashMap::new(),
            dyn_sig_traits: Default::default(), generic_bounds: HashMap::new(),
            trait_quals_by_param: HashMap::new(), trait_quals: HashMap::new(),
            fields: &fields, trait_fields: &trait_fields, trait_impls: &trait_impls,
            local_traits: &local_traits, returns: &returns, has_dyn_return: false,
            field_elem: &field_elem, enum_variants: &enum_variants, enum_variant_traits: &enum_variant_traits,
            ambiguous_enum_leaves: &std::collections::HashSet::new(), callable_statics: &std::collections::HashSet::new(), elem_of: HashMap::new(),
            field_elem_trait: &field_elem_trait, elem_trait_of: HashMap::new(),
            tuple_of: HashMap::new(), tuple_trait_of: HashMap::new(), calls: Vec::new(),
            closure_vars: Default::default(), fn_typed_vars: Default::default(),
            dep_bound_vars: HashMap::new(), fn_alias: Default::default(), lazy_statics: &lazy,
            forced_lazies: Default::default(), unresolved: false, err_ret_leaf: None,
            const_strings: &consts, local_macros: &macros, macro_expanding: Default::default(),
            str_locals: Default::default(),
            local_uses: Default::default(), bound_names: Default::default(), dispatch_sites: Default::default(),
            drop_relevant: &std::collections::HashSet::new(), escaping_ctors: Default::default(), marked_ctors: Default::default(), marked_cross_ctors: Default::default(), in_pattern: false,
        };
        // Every table gets an entry for the SAME name the binder is about to shadow.
        let n = "x";
        c.vars.insert(n.into(), "Outer".into());
        c.trait_vars.insert(n.into(), vec!["Store".into()]);
        c.dep_bound_vars.insert(n.into(), "deplib::build".into());
        c.trait_quals_by_param.insert(n.into(), HashMap::from([("Store".to_string(), "deplib::Store".to_string())]));
        c.elem_of.insert(n.into(), "Elem".into());
        c.elem_trait_of.insert(n.into(), vec!["Doer".into()]);
        c.tuple_of.insert(n.into(), vec![Some("A".into())]);
        c.tuple_trait_of.insert(n.into(), vec![vec!["Doer".into()]]);
        c.fn_alias.insert(n.into(), "effectful".into());
        c.str_locals.insert(n.into(), "https://outer.example".into());
        c.closure_vars.insert(n.into());
        c.fn_typed_vars.insert(n.into());

        let cleared = c.scoped_binding(n, crate::collector::Bound::Unknown, |s| {
            // RESOLUTION tables: nothing may answer for `x` inside the shadow.
            let mut leaked: Vec<&str> = Vec::new();
            if s.vars.contains_key(n) { leaked.push("vars"); }
            if s.trait_vars.contains_key(n) { leaked.push("trait_vars"); }
            if s.dep_bound_vars.contains_key(n) { leaked.push("dep_bound_vars"); }
            if s.trait_quals_by_param.contains_key(n) { leaked.push("trait_quals_by_param"); }
            if s.elem_of.contains_key(n) { leaked.push("elem_of"); }
            if s.elem_trait_of.contains_key(n) { leaked.push("elem_trait_of"); }
            if s.tuple_of.contains_key(n) { leaked.push("tuple_of"); }
            if s.tuple_trait_of.contains_key(n) { leaked.push("tuple_trait_of"); }
            if s.fn_alias.contains_key(n) { leaked.push("fn_alias"); }
            if s.str_locals.contains_key(n) { leaked.push("str_locals"); }
            // HEDGING sets: deliberately still present (see `scoped_binding`).
            assert!(s.closure_vars.contains(n) && s.fn_typed_vars.contains(n),
                    "a hedging set was cleared — a shadowed `x()` can now resolve to a free fn `x`, \
                     which is the fabrication mirror of the leak this binder exists to stop");
            leaked
        });
        assert!(cleared.is_empty(),
                "these name-keyed tables OUTLIVED their scope and answer for the shadow: {cleared:?}. \
                 A stale resolution entry fabricates the outer binding's effect on the inner name.");
        // …and everything is put back, or the shadow silently deletes the outer binding instead.
        assert_eq!(c.vars.get(n).map(String::as_str), Some("Outer"));
        assert_eq!(c.trait_vars.get(n).cloned(), Some(vec!["Store".to_string()]));
        assert_eq!(c.dep_bound_vars.get(n).map(String::as_str), Some("deplib::build"));
        assert!(c.trait_quals_by_param.contains_key(n));
        assert_eq!(c.elem_of.get(n).map(String::as_str), Some("Elem"));
        assert_eq!(c.elem_trait_of.get(n).cloned(), Some(vec!["Doer".to_string()]));
        assert_eq!(c.tuple_of.get(n).cloned(), Some(vec![Some("A".to_string())]));
        assert_eq!(c.tuple_trait_of.get(n).cloned(), Some(vec![vec!["Doer".to_string()]]));
        assert_eq!(c.fn_alias.get(n).map(String::as_str), Some("effectful"));
        assert_eq!(c.str_locals.get(n).map(String::as_str), Some("https://outer.example"));
    }

    #[test]
    fn a_scan_is_byte_identical_across_repeats_so_no_answer_rides_hash_order() {
        // A DETERMINISM DEFECT IS A SOUNDNESS DEFECT WHEN THE THING CHOSEN IS AN EFFECT OWNER.
        // candor-java's `nearestConcreteSuper` walked a `HashSet` and took the first hit in HASH ORDER —
        // not ordered wrongly, NOT ORDERED (`9f8e71c`, 11 193 of 11 277 changed answers). rust already
        // has form here too: `0eca79c`/`fee73fe` fixed a last-wins leaf map that stored the right answer
        // BY ACCIDENT. The engine's defence is the never-guess rule — `resolve_target` filters on
        // `v.len() == 1` rather than picking, the dep index REMOVES a colliding key, the CHA fallbacks
        // edge to ALL hits or to none, and every reported surface is a `BTreeSet`. Nothing pinned that.
        //
        // THIS IS THE PIN, and it is cheap because `RandomState::new()` reseeds per construction: two
        // scans in ONE process build their maps with DIFFERENT hash states (verified — ten HashMaps in
        // one process give ten distinct iteration orders), so a single `break`/`.next()`/last-wins
        // insert on an unordered container shows up as a differing report here without needing a
        // multi-process harness.
        //
        // The fixture is built to make an order-dependent pick VISIBLE rather than merely possible:
        // same-leaf types and traits in sibling modules (the collision the never-guess rule exists for),
        // one type impl'ing several traits with DEFAULT bodies of differing effects, several impls of
        // one trait, and it is spread over three files so the per-file decl merge order participates.
        let d = std::env::temp_dir().join(format!("candor-determinism-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(d.join("src")).unwrap();
        std::fs::write(d.join("Cargo.toml"), "[package]\nname = \"det\"\n").unwrap();
        std::fs::write(d.join("src/lib.rs"), "pub mod a;\npub mod b;\n").unwrap();
        // Two modules whose TYPE leaves, TRAIT leaves and METHOD leaves all collide, with DIFFERENT
        // effects on each side — so any resolution that picks a winner instead of refusing produces a
        // different answer depending on which side the iteration reached first.
        // `Multi` impls TWO traits that each carry a DEFAULT `emit` of a DIFFERENT effect. The
        // trait-default fallback walks `type_to_traits["Multi"]`, whose Vec is pushed while iterating
        // `merged.trait_impls` — a HashMap — so this is the ONE shape where a first-hit pick would read
        // a genuinely hash-ordered container. Verified by mutation: relaxing that site's `hits.len() == 1`
        // to `!hits.is_empty()` makes this test, and only this test, fail.
        std::fs::write(d.join("src/a.rs"), "\
pub trait Sink { fn emit(&self) { let _ = std::fs::read_to_string(\"/a/default\"); } }
pub trait Drain { fn emit(&self) { let _ = std::process::Command::new(\"drain\").status(); } }
pub struct Multi;
impl Sink for Multi {}
impl Drain for Multi {}
pub fn multi(m: &Multi) { m.emit(); }
pub struct Job;
impl Sink for Job {}
pub struct Other;
impl Sink for Other { fn emit(&self) { let _ = std::process::Command::new(\"a\").status(); } }
pub fn run(j: &Job) { j.emit(); }
pub fn helper() { let _ = std::fs::read_to_string(\"/a/helper\"); }
").unwrap();
        std::fs::write(d.join("src/b.rs"), "\
pub trait Sink { fn emit(&self) { let _ = std::net::TcpStream::connect(\"b.example:1\"); } }
pub struct Job;
impl Sink for Job {}
pub struct Third;
impl Sink for Third { fn emit(&self) { let _ = std::fs::read_to_string(\"/b/third\"); } }
pub fn run(j: &Job) { j.emit(); }
pub fn helper() { let _ = std::process::Command::new(\"b\").status(); }
").unwrap();
        let idx = DepIndex::default();
        let mut outs: Vec<String> = Vec::new();
        for _ in 0..6 {
            let (rc, body) = scan_one(&d.to_string_lossy(), ScanOpts {
                prefix: String::new(), want_json: true, include_tests: false, policy: None,
                baseline: None, ws_member: false, quiet: true, deps_idx: &idx, peek_excluded: false,
            }, &crate::gate::begin_run());
            assert_eq!(rc, 0);
            outs.push(body.unwrap());
        }
        let _ = std::fs::remove_dir_all(&d);
        // NOT VACUOUS: the fixture must actually have produced entries to disagree about (a fixture that
        // scans to nothing would pass this test forever — standing bar item 8).
        let first: serde_json::Value = serde_json::from_str(&outs[0]).unwrap();
        assert!(first["functions"].as_array().unwrap().len() >= 4,
                "the determinism fixture analysed almost nothing — it cannot witness an ordering bug\n{first:#}");
        for (i, o) in outs.iter().enumerate().skip(1) {
            assert_eq!(&outs[0], o,
                "scan {i} differs from scan 0 in the SAME process — an answer is riding hash iteration \
                 order. Every map built by a scan is reseeded, so this is a genuine ordering dependence, \
                 not flakiness: find the `break`/`.next()`/first-hit/last-wins over a HashMap or HashSet.");
        }
    }

    #[test]
    fn a_chained_deps_unknown_arrives_with_the_reason_class_that_explains_it() {
        // A TRUST MARKER FAILING OPEN — candor-ts `e66f29e`/`4dad22d`, found here in the chained-dep join.
        // The join writes `Unknown` straight into `direct`, so the CALLER is the source and there is no
        // callee entry in this report to inherit a reason from — but it carried no `unknownWhy` at all.
        // SPEC §4 requires one on a direct source, and the gate's documented fallback ("an Unknown with no
        // recorded reason is `unresolved`") then answered with the catch-all, so a class-targeted
        // `deny E Unknown[indirect]` / `[dispatch]` read GREEN over a dependency whose OWN report named
        // the class. Not a full fail-open — bare `deny Unknown` and `Unknown[dynamic]` still fired — which
        // is exactly why it survived: the strictest and the broadest gates both worked.
        //
        // VERBATIM, not re-derived. `dispatch:<owner>.<member>` carries the one NORMATIVE detail in the §4
        // vocabulary, and it is what a consumer resolves overrides through; mapping the reason back through
        // `ReasonClass` and re-emitting a canonical token would destroy precisely that.
        let d = std::env::temp_dir().join(format!("candor-depwhy-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        let _ = std::fs::create_dir_all(&d);
        let me = format!("scan-{}", env!("CARGO_PKG_VERSION"));
        std::fs::write(d.join("report.deplib.scan.json"), format!(r#"{{
            "candor": {{"version": "{me}", "toolchain": "stable", "spec": "0.23"}},
            "package": "deplib",
            "functions": [
              {{"fn": "io::murky", "inferred": ["Unknown"], "unresolved": true,
                "unknownWhy": ["dispatch:lib.Store.save"], "hash": "deplib#io::murky"}},
              {{"fn": "io::mute", "inferred": ["Unknown"], "unresolved": true,
                "hash": "deplib#io::mute"}}
            ]}}"#)).unwrap();
        let idx = load_dep_reports(Some(d.to_str().unwrap()));
        let _ = std::fs::remove_dir_all(&d);
        let v = scan_crate_chained("depwhy", "consumer", "\n[dependencies]\ndeplib = \"1\"\n",
            "pub fn from_named() { deplib::io::murky(); }\npub fn from_silent() { deplib::io::mute(); }\n",
            &idx);
        let why = |n: &str| -> Vec<String> {
            v["functions"].as_array().into_iter().flatten()
                .find(|f| f["fn"].as_str() == Some(n))
                .and_then(|f| f["unknownWhy"].as_array().cloned()).unwrap_or_default()
                .iter().filter_map(|x| x.as_str().map(String::from)).collect()
        };
        assert_eq!(why("from_named"), vec!["dispatch:lib.Store.save".to_string()],
                   "the dep's reason must cross the join VERBATIM — the `owner.member` detail is normative\n{v:#}");
        // …AND A REASON THE DEP DID NOT GIVE MUST NOT BE INVENTED. The fallback this test used to assert
        // stamped `callback:chained dependency declared Unknown without a reason` on a reasonless dep
        // Unknown, which §6.2 projects to `indirect` — a class naming a higher-order/owner-less
        // invocation that nothing here observed. §6.2 already answers this case normatively ("a function
        // whose `Unknown` carries no recorded reason is treated as `unresolved`"), so the empty field is
        // not a hole; the tag was a WRONG answer replacing a right one, and it made
        // `deny E Unknown[unresolved]` — the catch-all every conservative adopter keeps — read GREEN on
        // rust and RED on java/ts/swift over byte-identical input. All three of those leave it to the
        // fallback: java attaches nothing, ts attaches nothing on the call path, swift attaches a
        // provenance pointer it documents as projecting to `unresolved`.
        let silent = why("from_silent");
        assert!(silent.is_empty(),
                "a reason the dependency never gave was INVENTED for the consumer: {silent:?}\n{v:#}");
    }

    /// …and the CLASS is what a gate actually reads, so assert it through the gate rather than through
    /// the string. Two directions, and the second is the one that made this a defect rather than a
    /// cosmetic difference:
    ///   - `deny Unknown[unresolved]` MUST fire on a reasonless chained Unknown. §6.2 makes `unresolved`
    ///     the class of an Unknown with no recorded reason, and it is the catch-all a conservative
    ///     adopter keeps in every narrowed rule (the engine's own under-gating lint says so). The
    ///     `callback:` tag took it out of scope: exit 0 on rust, exit 1 on java/ts/swift, same input.
    ///   - `deny Unknown[indirect]` MUST NOT fire on it. Nothing observed a higher-order or owner-less
    ///     invocation here; that class was manufactured by the join.
    ///
    /// The `dispatch:`-carrying sibling in the same crate is the control: a class the dependency DID
    /// record must still reach the gate, so this cannot be satisfied by dropping reasons wholesale.
    #[test]
    fn a_reasonless_chained_unknown_is_gated_as_unresolved_not_as_a_class_the_join_invented() {
        let dd = std::env::temp_dir().join(format!("candor-depwhygate-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dd);
        std::fs::create_dir_all(&dd).unwrap();
        let me = format!("scan-{}", env!("CARGO_PKG_VERSION"));
        std::fs::write(dd.join("report.deplib.scan.json"), format!(r#"{{
            "candor": {{"version": "{me}", "toolchain": "stable", "spec": "0.23"}},
            "package": "deplib",
            "functions": [
              {{"fn": "io::murky", "inferred": ["Unknown"], "unresolved": true,
                "unknownWhy": ["dispatch:lib.Store.save"], "hash": "deplib#io::murky"}},
              {{"fn": "io::mute", "inferred": ["Unknown"], "unresolved": true,
                "hash": "deplib#io::mute"}}
            ]}}"#)).unwrap();
        let idx = load_dep_reports(Some(dd.to_str().unwrap()));

        let d = std::env::temp_dir().join(format!("candor-depwhygate-c-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(d.join("src")).unwrap();
        std::fs::write(d.join("Cargo.toml"),
            "[package]\nname = \"consumer\"\n\n[dependencies]\ndeplib = \"1\"\n").unwrap();
        std::fs::write(d.join("src/lib.rs"), "pub fn from_silent() { deplib::io::mute(); }\n").unwrap();
        let run = |rule: &str| -> i32 {
            let p = d.join("candor.policy");
            std::fs::write(&p, format!("{rule}\n")).unwrap();
            let (rc, _) = scan_one(&d.to_string_lossy(), ScanOpts {
                prefix: d.join("out/r").to_string_lossy().into_owned(), want_json: true,
                include_tests: false, policy: Some(p.to_string_lossy().into_owned()),
                baseline: None, ws_member: false, quiet: true, deps_idx: &idx, peek_excluded: false,
            }, &crate::gate::begin_run());
            rc
        };
        assert_eq!(run("deny Unknown"), 1, "the bare rule must still bite — the Unknown IS there");
        assert_eq!(run("deny Unknown[unresolved]"), 1,
                   "THE CATCH-ALL WENT GREEN: a reasonless chained Unknown was tagged out of \
                    `unresolved`, so the class every narrowed rule is told to keep no longer sees it — \
                    and java/ts/swift all exit 1 here");
        assert_eq!(run("deny Unknown[indirect]"), 0,
                   "a class NOTHING observed was manufactured by the join: no higher-order or \
                    owner-less invocation was seen, only a dependency that gave no reason");
        // The control: a class the dependency DID record must still reach the gate.
        std::fs::write(d.join("src/lib.rs"), "pub fn from_named() { deplib::io::murky(); }\n").unwrap();
        assert_eq!(run("deny Unknown[dispatch]"), 1,
                   "the dep's OWN recorded class stopped reaching the gate — this fix must drop only \
                    the invented reason, never a carried one");
        assert_eq!(run("deny Unknown[unresolved]"), 0,
                   "…and a classified Unknown must NOT also read `unresolved`, or the chained consumer \
                    gates differently from the same code unsplit");
        let _ = std::fs::remove_dir_all(&d);
        let _ = std::fs::remove_dir_all(&dd);
    }

    /// …AND IT MUST STILL GATE WHEN THE SAME FUNCTION CARRIES ANOTHER REASON. The test above passes on
    /// the §6.2 fallback alone — "an `Unknown` with no recorded reason is `unresolved`" — which the gate
    /// implements per FUNCTION, firing only when the whole class set is ABSENT or EMPTY. So any other
    /// reason on the same function SWALLOWED the reasonless one: `both()` calling a dep fn that recorded
    /// `dispatch:` and one that recorded nothing classified `dispatch` alone, and
    /// `deny E Unknown[unresolved]` — the catch-all a conservative adopter keeps in every narrowed rule —
    /// went from exit 1 to exit 0 **as the second call was added**. The fallback was doing the work
    /// everywhere except where two reasons meet on one function, which is exactly where a gate needs it.
    ///
    /// THE ROWS ARE ORDERED CONTROL-FIRST, and they are what makes this a defect rather than a
    /// preference: the two single-call functions bracket `both`, so the failure is stated as
    /// MONOTONICITY — adding a reason to a function must never take a class away from it. The
    /// `[dispatch]` rows are the second direction: the carried class must survive the fix (a fix that
    /// stamped `unresolved` over the class set instead of into it would pass the first row and fail
    /// here), and a function whose ONLY Unknown is a classified one must not read `unresolved`.
    #[test]
    fn a_reasonless_chained_unknown_still_reaches_the_gate_when_the_fn_has_another_reason() {
        let dd = std::env::temp_dir().join(format!("candor-depwhyboth-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dd);
        std::fs::create_dir_all(&dd).unwrap();
        let me = format!("scan-{}", env!("CARGO_PKG_VERSION"));
        std::fs::write(dd.join("report.deplib.scan.json"), format!(r#"{{
            "candor": {{"version": "{me}", "toolchain": "stable", "spec": "0.23"}},
            "package": "deplib",
            "functions": [
              {{"fn": "io::murky", "inferred": ["Unknown"], "unresolved": true,
                "unknownWhy": ["dispatch:lib.Store.save"], "hash": "deplib#io::murky"}},
              {{"fn": "io::mute", "inferred": ["Unknown"], "unresolved": true,
                "hash": "deplib#io::mute"}}
            ]}}"#)).unwrap();
        let idx = load_dep_reports(dd.to_str());

        let d = std::env::temp_dir().join(format!("candor-depwhyboth-c-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(d.join("src")).unwrap();
        std::fs::write(d.join("Cargo.toml"),
            "[package]\nname = \"consumer\"\n\n[dependencies]\ndeplib = \"1\"\n").unwrap();
        let run = |src: &str, rule: &str| -> i32 {
            std::fs::write(d.join("src/lib.rs"), src).unwrap();
            let p = d.join("candor.policy");
            std::fs::write(&p, format!("{rule}\n")).unwrap();
            let (rc, _) = scan_one(&d.to_string_lossy(), ScanOpts {
                prefix: d.join("out/r").to_string_lossy().into_owned(), want_json: true,
                include_tests: false, policy: Some(p.to_string_lossy().into_owned()),
                baseline: None, ws_member: false, quiet: true, deps_idx: &idx, peek_excluded: false,
            }, &crate::gate::begin_run());
            rc
        };
        // CONTROL A — the classified call ALONE. `unresolved` must not fire (nothing here is reasonless).
        let named = "pub fn one() { deplib::io::murky(); }\n";
        assert_eq!(run(named, "deny Unknown[dispatch]"), 1, "the carried class must reach the gate");
        assert_eq!(run(named, "deny Unknown[unresolved]"), 0,
                   "a CLASSIFIED Unknown must not also read `unresolved` — the fix must add the class \
                    for the reasonless join only, never stamp it on every chained Unknown");
        // CONTROL B — the reasonless call ALONE. §6.2's fallback answers this one, and it always did.
        let silent = "pub fn one() { deplib::io::mute(); }\n";
        assert_eq!(run(silent, "deny Unknown[unresolved]"), 1, "§6.2: no recorded reason ⇒ `unresolved`");
        assert_eq!(run(silent, "deny Unknown[dispatch]"), 0, "…and it is not `dispatch`");
        // THE DEFECT — both on one function. Bracketed by the two controls, so the only thing that
        // changed is that a SECOND, differently-classed Unknown joined the first.
        let both = "pub fn one() { deplib::io::murky(); deplib::io::mute(); }\n";
        assert_eq!(run(both, "deny Unknown[unresolved]"), 1,
                   "ADDING A REASON REMOVED A CLASS: the same reasonless chained Unknown that gates \
                    `unresolved` on its own (control B) stopped gating once an unrelated `dispatch:` \
                    reason landed on the same function. The §6.2 fallback is per-FUNCTION and fires only \
                    on an ABSENT/EMPTY class set, so the reasonless join must CONTRIBUTE `unresolved` \
                    rather than have it inferred from absence");
        assert_eq!(run(both, "deny Unknown[dispatch]"), 1,
                   "…and the dep's own recorded class must survive that — contributed INTO the set, \
                    never over it");
        assert_eq!(run(both, "deny Unknown"), 1, "the bare rule bites in every arm");
        // SECOND DIRECTION, IN-SCAN: an Unknown this scan raised ITSELF, with its own reason and no
        // chained dep in sight, must be untouched. `unresolved` is contributed by the JOIN, not by
        // carrying an Unknown.
        let local = "pub fn one(f: fn()) { f(); }\n";
        assert_eq!(run(local, "deny Unknown[indirect]"), 1, "an in-scan callback is `indirect`");
        assert_eq!(run(local, "deny Unknown[unresolved]"), 0,
                   "an in-scan Unknown with its own reason must NOT gain `unresolved` — that would make \
                    every narrowed rule fire on everything and delete the class distinction");
        // …AND IT TRAVELS. The class propagates the call graph like the effect does, so a caller that
        // has its own reason must see the callee's reasonless one too.
        let up = "pub fn leaf() { deplib::io::mute(); }\n\
                  pub fn up(f: fn()) { f(); leaf(); }\n";
        assert_eq!(run(up, "deny Unknown[unresolved] up"), 1,
                   "the contributed class must PROPAGATE — a caller with its own `indirect` reason \
                    inheriting a reasonless chained Unknown is the same defect one edge up");
        let _ = std::fs::remove_dir_all(&d);
        let _ = std::fs::remove_dir_all(&dd);
    }

    /// THE SECOND ARM OF SPEC §6.2 ⟨0.24⟩'s CONTROL, and the one this engine was failing: a FRESH
    /// dependency whose `Unknown` IS explained — not by its own tag (the test above) but **by a `calls`
    /// EDGE it published**.
    ///
    /// §4 makes `unknownWhy` DIRECT-ONLY, so a dep entry whose `Unknown` is purely INHERITED correctly
    /// carries no reason, and is byte-for-byte indistinguishable from one nothing accounted for.
    /// Charging both to `unresolved` is the naive form: §6.2 calls contributing it "to one whose
    /// `Unknown` is correctly classified at the callee" the MIRROR FABRICATION, and the changelog
    /// calibrates it — the naive rule flips 96/141 and 211/211 on the JVM engine's fresh reports where
    /// the correct one flips none, and marks 435 on swift where the legitimate count is 0.
    ///
    /// MEASURED ON THIS ENGINE BEFORE THE FIX, on TRUSTED reports, over real code: scanning candor-scan
    /// against its own 173-report dep tree, `deny Unknown[unresolved]` fired on **26** functions and
    /// `deny Unknown[dispatch]` on 19. After: **0** and **28**. Every one of the 26 traced to 8 callers
    /// of three `syn` entries whose `Unknown` syn's OWN `calls` chain explains 2–5 hops down as
    /// `ambiguous:same-name local defs` — class `dispatch`. The effect sets are IDENTICAL across the A/B
    /// (46 Unknown-bearing functions both sides, 0 changed): no `Unknown` was gained or lost, only its
    /// reason corrected, and the report gained 8 disclosures it should always have carried.
    ///
    /// THE DISCRIMINATION CONTROL IS THE `[dispatch]` COLUMN GOING **UP**. A change that merely stopped
    /// contributing would take `[unresolved]` to 0 and leave `[dispatch]` at 19. 19 → 28 is what
    /// distinguishes correcting a class from deleting one, and it is why both directions are asserted
    /// below rather than just the one that goes quiet.
    #[test]
    fn a_dep_unknown_the_deps_own_calls_chain_explains_is_not_charged_the_catch_all() {
        let dd = std::env::temp_dir().join(format!("candor-depchain-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dd);
        std::fs::create_dir_all(&dd).unwrap();
        let me = format!("scan-{}", env!("CARGO_PKG_VERSION"));
        // `io::relay` is the new arm: `Unknown`, NO tag of its own, and a published `calls` edge to
        // `io::deep`, which names the reason. `reflect:` deliberately — a class no other entry here
        // carries, so a fixture that merely stopped fabricating could not pass by accident.
        // `io::mute` is the discrimination control: `Unknown`, no tag, and NO edge to carry one.
        let report = format!(r#"{{
            "candor": {{"version": "{me}", "toolchain": "stable", "spec": "0.23"}},
            "package": "deplib",
            "functions": [
              {{"fn": "io::relay", "inferred": ["Unknown"], "calls": ["io::deep"],
                "hash": "deplib#io::relay"}},
              {{"fn": "io::deep", "inferred": ["Unknown"], "direct": ["Unknown"],
                "unknownWhy": ["reflect:Class.forName"], "hash": "deplib#io::deep"}},
              {{"fn": "io::mute", "inferred": ["Unknown"], "hash": "deplib#io::mute"}}
            ]}}"#);
        std::fs::write(dd.join("report.deplib.scan.json"), &report).unwrap();
        let idx = load_dep_reports(dd.to_str());

        let d = std::env::temp_dir().join(format!("candor-depchain-c-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(d.join("src")).unwrap();
        std::fs::write(d.join("Cargo.toml"),
            "[package]\nname = \"consumer\"\n\n[dependencies]\ndeplib = \"1\"\n").unwrap();
        let run = |src: &str, rule: &str, ix: &DepIndex| -> i32 {
            std::fs::write(d.join("src/lib.rs"), src).unwrap();
            let p = d.join("candor.policy");
            std::fs::write(&p, format!("{rule}\n")).unwrap();
            let (rc, _) = scan_one(&d.to_string_lossy(), ScanOpts {
                prefix: d.join("out/r").to_string_lossy().into_owned(), want_json: true,
                include_tests: false, policy: Some(p.to_string_lossy().into_owned()),
                baseline: None, ws_member: false, quiet: true, deps_idx: ix, peek_excluded: false,
            }, &crate::gate::begin_run());
            rc
        };
        // (1) THE CHAIN-EXPLAINED ARM. The dependency published the edge AND the reason at its end, so
        // the consumer must get that class — and must NOT also get the catch-all.
        let relay = "pub fn one() { deplib::io::relay(); }\n";
        assert_eq!(run(relay, "deny Unknown[reflect]", &idx), 1,
                   "a reason the dep's OWN `calls` chain reaches must cross the join");
        assert_eq!(run(relay, "deny Unknown[unresolved]", &idx), 0,
                   "THE MIRROR FABRICATION: this `Unknown` IS accounted for, one edge down in the \
                    dependency's own published graph. Charging it the catch-all is the naive form — \
                    contributing `unresolved` whenever an `Unknown` is present without asking whether \
                    anything explains it");
        // (2) THE DISCRIMINATION CONTROL. Same shape, no edge to carry a reason — still `unresolved`.
        // Without this, "explain through the chain" and "stop contributing" are the same diff.
        let mute = "pub fn one() { deplib::io::mute(); }\n";
        assert_eq!(run(mute, "deny Unknown[unresolved]", &idx), 1,
                   "an `Unknown` NEITHER a tag NOR a chain accounts for must still fail closed");
        assert_eq!(run(mute, "deny Unknown[reflect]", &idx), 0, "…and must not borrow a sibling's class");
        // (3) ROW 3 — both on one function. The class set is a UNION and can only GROW: §6.2's
        // monotone-denial corollary, which the absence-keyed rule violated.
        let both = "pub fn one() { deplib::io::relay(); deplib::io::mute(); }\n";
        assert_eq!(run(both, "deny Unknown[unresolved]", &idx), 1, "the unaccounted one still bites");
        assert_eq!(run(both, "deny Unknown[reflect]", &idx), 1, "…and so does the chain-explained one");
        assert_eq!(run(both, "deny Unknown", &idx), 1, "the bare rule bites in every arm");
        // (4) THE STALENESS GATE. §2.1 refuses to believe a distrusted report's EFFECTS, so it must not
        // believe its `calls` either — the chain is a claim by the same producer we just refused, and a
        // resolution pass that read one would launder exactly what the downgrade exists to reject.
        //
        // TWO GATES HOLD THIS, AND NEITHER ALONE FAILS THIS ASSERTION — measured by mutating each: the
        // fixpoint is skipped when `stale`, AND the `stale` arm never reads a reason field at all. Both
        // had to be removed together before this line went red. Recorded because the natural reading is
        // that the `!stale` guard is the gate and the other is redundant; it is the pair that is load-
        // bearing, and deleting either as "dead" leaves a test that no longer defends what it names.
        let sd = std::env::temp_dir().join(format!("candor-depchain-stale-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&sd);
        std::fs::create_dir_all(&sd).unwrap();
        std::fs::write(sd.join("report.deplib.scan.json"),
                       report.replace(&me, "scan-0.0.1-OTHER")).unwrap();
        let stale = load_dep_reports(sd.to_str());
        assert_eq!(run(relay, "deny Unknown[unresolved]", &stale), 1,
                   "a DISTRUSTED report's `calls` chain must not explain anything — the §2.1 downgrade \
                    is refusing to believe that producer, and its edges are the same producer's claim");
        assert_eq!(run(relay, "deny Unknown[reflect]", &stale), 0,
                   "…so the class it would have carried must not reach the gate either");
        let _ = std::fs::remove_dir_all(&d);
        let _ = std::fs::remove_dir_all(&dd);
        let _ = std::fs::remove_dir_all(&sd);
    }

    #[test]
    fn a_stale_reports_unknown_is_classed_unresolved_like_the_rest_of_the_family() {
        // The §2.1 staleness downgrade MANUFACTURES an `Unknown` that no call site is responsible for —
        // which is exactly why it has no class. This test used to require a canonical §4 kind here, and
        // the only kind that fits nothing is `callback:`, so the downgrade shipped tagged
        // `callback:chained report from a different producer version` → §6.2 class `indirect`.
        //
        // There is no single-tree control for a stale report (staleness is a property of the report, not
        // of any code), so the deciding evidence is the family: java attaches nothing; ts attaches
        // nothing on the call path and `stale-dep:<pkg>` on the import path; swift attaches
        // `dep-stale:<pkg>` and DOCUMENTS that §6.2 projects it to `unresolved`. Three engines, one
        // class — `unresolved`, which is also what §6.2 prescribes for an Unknown carrying no reason.
        // rust alone said `indirect`, so `deny Unknown[unresolved]` over an untrusted dependency was
        // green on rust and red on the other three.
        //
        // rust does not follow swift and ts into a `dep-stale:`-shaped token. ⟨0.24⟩ THE REASON CHANGED
        // AND THE ANSWER DID NOT: §4 has since REGISTERED `dep:`/`dep-stale:` as permanent kinds and
        // `ambiguous:` as its fifth, so neither is an "open item" any more — but the SHIPPED conformance
        // PART 10 still hard-DIVERGEs on any kind outside the canonical four plus its two named migration
        // kinds, and `dep-stale` is not among them. The prose goes to stderr instead — the channel ts and
        // swift already disclose staleness on, and the one rust was missing entirely.
        let d = std::env::temp_dir().join(format!("candor-stalewhy-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        let _ = std::fs::create_dir_all(&d);
        std::fs::write(d.join("report.oldlib.scan.json"), r#"{
            "candor": {"version": "scan-0.0.1", "toolchain": "stable", "spec": "0.3"},
            "package": "oldlib",
            "functions": [{"fn": "io::go", "inferred": ["Exec"], "hash": "oldlib#io::go"}]}"#).unwrap();
        let idx = load_dep_reports(Some(d.to_str().unwrap()));
        let _ = std::fs::remove_dir_all(&d);
        let e = idx.by_key.get("oldlib#io::go").expect("stale entry present");
        assert_eq!(e.effects, BTreeSet::from(["Unknown"]), "the §2.1 downgrade itself must not move");
        assert!(e.unknown_why.is_empty(),
                "the staleness downgrade INVENTED a reason class: {:?}", e.unknown_why);
        assert_eq!(
            candor_classify::policy::ReasonClass::classify("anything unrecognized"),
            candor_classify::policy::ReasonClass::Unresolved,
            "…and the §6.2 catch-all this relies on must still be the catch-all"
        );
    }

    /// FRESH-VS-STALE FOR ONE PACKAGE: rust withholds coverage, and the other three engines do not.
    /// Recorded, pinned and DELIBERATELY NOT ALIGNED — the argument is below and it is rust-specific.
    ///
    /// The divergence is real. Coverage in java is `if (!stale) depCoveredPkgs.addAll(...)`
    /// (`Loader.java`), in ts `depCoveredPkgs.has(pkg)` with `staleDepPkgs` deleted down to it
    /// (`scan.mjs:698`), in swift `coveredPkgs.contains` with `stalePkgs.subtract(coveredPkgs)`
    /// (`Deps.swift:294`) — in all three, a stale report can never take back coverage a fresh one
    /// granted. rust keeps an `untrusted` set that is never subtracted, so the same input answers the
    /// other way. SPEC §2.1 is silent: it says only that a version mismatch downgrades the inherited
    /// EFFECTS to `Unknown`, and says nothing about the ledger exemption when two reports disagree.
    ///
    /// TWO ARGUMENTS USED TO HOLD THIS UP. ONE OF THEM IS NOW DEAD, AND THE HEDGE STAYS ON THE OTHER.
    ///
    /// **The dead one — withdrawal.** This test used to argue: rust's index DROPS a key two dep functions
    /// share, so a fresh-plus-stale collision resolves to NOTHING; grant the package coverage and the call
    /// reads confidently PURE with the fresh report's effect silently gone. Withholding coverage was "the
    /// only thing standing between that collision and a false all-clear". Entries are UNIONED now
    /// (ENTRY-COLLISION-DECISION.md), the collision resolves, and that argument no longer describes the
    /// engine. Left visible rather than deleted, because a fixture whose stated reason has quietly expired
    /// is how a guard turns into a habit.
    ///
    /// **The surviving one — coverage cannot tell versions apart.** From `DepIndex::untrusted`'s own doc:
    /// *a fresh report for part of a crate cannot vouch for the part the stale one covered.* Coverage is
    /// keyed by package NAME, and the whole reason two reports collide here is that the name spans two
    /// VERSIONS. Granting coverage on the fresh report's authority would certify the silence of a version
    /// nothing trusted ever analyzed: a function present only in the stale version, absent from both
    /// reports because the stale one judged it pure, would read pure on the authority of a report that
    /// never saw it. That is independent of how collisions resolve, so the union does not touch it.
    ///
    /// java and ts can afford fresh-wins because their entry-level conflict always keeps an answer.
    /// **candor-swift drops the colliding key exactly as rust did (`Deps.swift` `insert`) AND resolves
    /// coverage fresh-wins — that pairing is the unsound one and it is the open swift row.**
    ///
    /// TO FLIP THIS, a four-way ruling would have to answer the version argument above, not the
    /// withdrawal one: it needs coverage keyed finer than the package name, or an explicit decision that a
    /// stale report's silence may speak. A one-line subtraction in `cover` is the mechanism, but it is no
    /// longer the whole question.
    #[test]
    fn a_package_chained_both_fresh_and_stale_keeps_its_blind_spot_disclosure() {
        let d = std::env::temp_dir().join(format!("candor-duallib-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        let me = format!("scan-{}", env!("CARGO_PKG_VERSION"));
        // Both reports name package `duallib`, and both carry `io::go` — the collision that makes this
        // a soundness question rather than a bookkeeping one. (Two versions of one crate in a Cargo tree
        // is routine: 7 of 167 dep reports in candor-rust's own tree, 30 of 378 in ebman's.)
        std::fs::write(d.join("report.fresh.duallib.scan.json"), format!(r#"{{
            "candor": {{"version": "{me}", "toolchain": "stable", "spec": "0.23"}},
            "package": "duallib",
            "functions": [{{"fn": "io::go", "inferred": ["Exec"], "hash": "duallib#io::go"}}]}}"#)).unwrap();
        std::fs::write(d.join("report.stale.duallib.scan.json"), r#"{
            "candor": {"version": "scan-0.0.1", "toolchain": "stable", "spec": "0.3"},
            "package": "duallib",
            "functions": [{"fn": "io::go", "inferred": ["Exec"], "hash": "duallib#io::go"}]}"#).unwrap();
        // …and a package with ONLY a fresh report, as the control: this must not become a blanket
        // withdrawal of coverage the moment any report in the directory is stale.
        std::fs::write(d.join("report.solo.scan.json"), format!(r#"{{
            "candor": {{"version": "{me}", "toolchain": "stable", "spec": "0.23"}},
            "package": "solo",
            "functions": [{{"fn": "io::go", "inferred": ["Exec"], "hash": "solo#io::go"}}]}}"#)).unwrap();
        let idx = load_dep_reports(Some(d.to_str().unwrap()));
        let _ = std::fs::remove_dir_all(&d);

        assert!(idx.untrusted.contains("duallib"),
                "a package one of whose reports failed the §2.1 check lost its untrusted mark — see the \
                 doc comment before aligning this with java/ts/swift");
        assert!(!idx.untrusted.contains("solo"), "the control package must stay trusted");
        // THE PREMISE, ASSERTED RATHER THAN ASSUMED — and it has INVERTED since this test was written.
        // The shared key now resolves: entries are unioned, so the fresh report's `Exec` survives beside
        // the stale report's §2.1 `Unknown` downgrade. The hedge below is kept all the same, and the
        // doc comment above says which of the two arguments for it died with the withdrawal.
        assert_eq!(idx.by_key.get("duallib#io::go").map(|d| d.effects.clone()),
                   Some(BTreeSet::from(["Exec", "Unknown"])),
                   "the trusted report's `Exec` and the distrusted report's `Unknown` must BOTH survive: \
                    the union is what stops a report we refused to believe from erasing a fact from one we \
                    do, and what stops the collision from erasing both");

        let v = scan_crate_chained("duallib", "consumer",
            "\n[dependencies]\nduallib = \"1\"\nsolo = \"1\"\n",
            "pub fn hits_dual() { duallib::io::go(); }\npub fn hits_solo() { solo::io::go(); }\n", &idx);
        let ent = |n: &str| v["functions"].as_array().into_iter().flatten()
            .find(|f| f["fn"].as_str() == Some(n)).cloned().unwrap_or(serde_json::Value::Null);
        let dual = ent("hits_dual");
        assert!(dual != serde_json::Value::Null,
                "A FALSE ALL-CLEAR: the fresh report says `duallib::io::go` runs `Exec`, the colliding \
                 key withdrew the answer, and with the package counted as covered the call reads \
                 confidently PURE — the absent entry IS a purity claim (§2 rule 3):\n{v:#}");
        assert_eq!(dual["invisible"], serde_json::json!(["duallib"]),
                   "the disclosure must NAME the package whose report could not be trusted:\n{v:#}");
        // Control, other direction: the solo package's silence stays informative — its call resolves to
        // the report's own answer and carries no blind-spot hedge.
        let solo = ent("hits_solo");
        assert_eq!(solo["inferred"], serde_json::json!(["Exec"]), "the trusted report's answer:\n{v:#}");
        assert!(solo["invisible"].is_null(),
                "a fresh report's package was hedged as blind — this is the mirror defect:\n{v:#}");
    }

    #[test]
    fn an_untrusted_report_does_not_grant_the_ledger_coverage_exemption() {
        // §2.1 downgrades a STALE report's effects to `Unknown` — and the very same load ALSO
        // registered its package as COVERED, which is what exempts a crate from the κ blind-spot
        // ledger. So every function the distrusted report did not mention read as a confident purity
        // claim (§2 rule 3: an absent entry IS one), with `invisible` dropped, on the authority of a
        // report the engine had just refused to believe. candor-ts `651c9f9` is the same defect.
        //
        // BOTH DIRECTIONS, because narrowing coverage is exactly where the mirror defect lands
        // (standing bar item 0): the STALE crate must lose the exemption, and the FRESH crate must
        // KEEP it — a report we do trust is silent about its pure functions on purpose.
        let d = std::env::temp_dir().join(format!("candor-staletrust-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        let _ = std::fs::create_dir_all(&d);
        let me = format!("scan-{}", env!("CARGO_PKG_VERSION"));
        std::fs::write(d.join("report.stalelib.scan.json"), r#"{
            "candor": {"version": "scan-0.0.1", "toolchain": "stable", "spec": "0.3"},
            "package": "stalelib",
            "functions": [{"fn": "io::go", "inferred": ["Exec"], "hash": "stalelib#io::go"}]}"#).unwrap();
        std::fs::write(d.join("report.freshlib.scan.json"), format!(r#"{{
            "candor": {{"version": "{me}", "toolchain": "stable", "spec": "0.23"}},
            "package": "freshlib",
            "functions": [{{"fn": "io::go", "inferred": ["Exec"], "hash": "freshlib#io::go"}}]}}"#)).unwrap();
        let idx = load_dep_reports(Some(d.to_str().unwrap()));
        let _ = std::fs::remove_dir_all(&d);
        // The JOIN must still see both crates — a stale entry can only be downgraded to `Unknown` if
        // the join fires at all, so the fix must not reach `crates`.
        assert!(idx.crates.contains("stalelib") && idx.crates.contains("freshlib"),
                "the join gate must still cover both, stale included: {:?}", idx.crates);
        assert!(idx.untrusted.contains("stalelib"), "a stale report's package must be untrusted");
        assert!(!idx.untrusted.contains("freshlib"), "a fresh report's package must stay trusted");
        assert_eq!(idx.by_key.get("stalelib#io::go").map(|e| e.effects.clone()), Some(BTreeSet::from(["Unknown"])));

        // End to end: the consumer's report is what a reader sees.
        let src = "\
pub fn listed_stale() { stalelib::io::go(); }
pub fn unlisted_stale() { stalelib::io::danger(); }
pub fn unlisted_fresh() { freshlib::io::danger(); }
";
        let v = scan_crate_chained("staletrust", "consumer",
            "\n[dependencies]\nstalelib = \"1\"\nfreshlib = \"1\"\n", src, &idx);
        let inv = |name: &str| -> Vec<String> {
            v["functions"].as_array().into_iter().flatten()
                .find(|f| f["fn"].as_str() == Some(name))
                .and_then(|f| f["invisible"].as_array().cloned()).unwrap_or_default()
                .iter().filter_map(|x| x.as_str().map(String::from)).collect()
        };
        // 1. the defect: a function reaching ONLY the distrusted crate must not read plain pure.
        assert!(inv("unlisted_stale").contains(&"stalelib".to_string()),
                "a fn whose only reach is into a DISTRUSTED report's crate must disclose it blind\n{v:#}");
        assert!(inv("listed_stale").contains(&"stalelib".to_string()),
                "the disclosure is per-crate, not per-entry — the joined fn discloses it too\n{v:#}");
        // 2. the mirror: the crate we DO trust keeps its exemption, so its silence still means pure.
        assert!(!inv("unlisted_fresh").contains(&"freshlib".to_string()),
                "a TRUSTED report's silence is its purity claim — do not re-disclose it as blind\n{v:#}");
        // 3. and the ledger the stderr line / gate advisory / `coverage` field all share.
        let uncovered: Vec<String> = v["coverage"]["uncovered"].as_array().into_iter().flatten()
            .filter_map(|c| c["name"].as_str().map(String::from)).collect();
        assert_eq!(uncovered, vec!["stalelib".to_string()],
                   "the κ ledger must name the distrusted crate and only it\n{v:#}");
    }

    /// ⟨0.21⟩ THE SECOND DIRECTION OF THE INCOMPLETENESS GATE, WRITTEN FIRST (standing bar item 0).
    ///
    /// The rung below withholds COVERAGE from a chained report whose ⟨0.21⟩ `unanalyzed` says it never
    /// read some of its own source. The way that rung goes wrong is by taking the ENTRIES with it — an
    /// incomplete report's entries were derived from source it DID read and are TRUE, so treating
    /// incompleteness like staleness (which downgrades entries to `Unknown`) would trade a disclosure
    /// gain for a precision loss on every function the report DOES answer. This fixture is the assertion
    /// that it does not: it must pass BEFORE the rung lands and after.
    ///
    /// It also pins the two things that must keep buying coverage: an ABSENT `unanalyzed` (this engine's
    /// writer omits the key when the manifest is empty, so absence is how a complete scan says "I read
    /// everything" — reading it as incompleteness would hedge every report ever written) and an
    /// explicitly EMPTY one.
    #[test]
    fn an_incomplete_reports_entries_are_applied_unchanged_and_a_complete_one_still_covers() {
        let d = std::env::temp_dir().join(format!("candor-incomplete-keep-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        let me = format!("scan-{}", env!("CARGO_PKG_VERSION"));
        // A report that names source it could not analyze — and still carries a real answer.
        std::fs::write(d.join("report.brokelib.scan.json"), format!(r#"{{
            "candor": {{"version": "{me}", "toolchain": "stable", "spec": "0.23"}},
            "package": "brokelib",
            "unanalyzed": [{{"path": "src/broken.rs", "reason": "parse error"}}],
            "functions": [{{"fn": "io::go", "inferred": ["Exec"], "hash": "brokelib#io::go",
                            "cmds": ["/bin/ls"]}}]}}"#)).unwrap();
        // The complete control: no `unanalyzed` key at all, which is what this engine writes.
        std::fs::write(d.join("report.wholelib.scan.json"), format!(r#"{{
            "candor": {{"version": "{me}", "toolchain": "stable", "spec": "0.23"}},
            "package": "wholelib",
            "functions": [{{"fn": "io::go", "inferred": ["Exec"], "hash": "wholelib#io::go"}}]}}"#)).unwrap();
        // …and the explicitly-EMPTY manifest, the other shape that means "complete".
        std::fs::write(d.join("report.emptylib.scan.json"), format!(r#"{{
            "candor": {{"version": "{me}", "toolchain": "stable", "spec": "0.23"}},
            "package": "emptylib",
            "unanalyzed": [],
            "functions": [{{"fn": "io::go", "inferred": ["Exec"], "hash": "emptylib#io::go"}}]}}"#)).unwrap();
        let idx = load_dep_reports(Some(d.to_str().unwrap()));
        let _ = std::fs::remove_dir_all(&d);

        // 1. THE ENTRY IS KEPT, VERBATIM. Not downgraded to `Unknown` the way a stale one is, and its
        //    literal surface travels with it.
        let e = idx.by_key.get("brokelib#io::go").expect("the incomplete report's entry must survive");
        assert_eq!(e.effects, BTreeSet::from(["Exec"]),
                   "an incomplete report's entries were derived from source it DID read — downgrading \
                    them is the staleness treatment and it does not belong here");
        assert_eq!(e.cmds, BTreeSet::from(["/bin/ls".to_string()]), "the literal surface travels with the entry");
        // 2. The join gate still sees the crate — coverage and chained-ness are different questions.
        assert!(idx.crates.contains("brokelib"), "the join must still fire: {:?}", idx.crates);

        // 3. END TO END: the answered key still answers, and the two COMPLETE shapes keep their
        //    exemption — a report that read all of its own source is silent about its pure functions on
        //    purpose (§2 rule 3), and hedging that silence would be a FALSE disclosure.
        let src = "\
pub fn listed_broke() { brokelib::io::go(); }
pub fn unlisted_whole() { wholelib::io::danger(); }
pub fn unlisted_empty() { emptylib::io::danger(); }
";
        let v = scan_crate_chained("incompletekeep", "consumer",
            "\n[dependencies]\nbrokelib = \"1\"\nwholelib = \"1\"\nemptylib = \"1\"\n", src, &idx);
        let ent = |n: &str| v["functions"].as_array().into_iter().flatten()
            .find(|f| f["fn"].as_str() == Some(n)).cloned();
        let listed = ent("listed_broke").unwrap_or(serde_json::Value::Null);
        assert_eq!(listed["inferred"], serde_json::json!(["Exec"]),
                   "the incomplete report's ANSWER must still reach the consumer:\n{v:#}");
        assert_eq!(listed["cmds"], serde_json::json!(["/bin/ls"]),
                   "…and so must its literal surface:\n{v:#}");
        let inv = |n: &str| -> Vec<String> {
            ent(n).and_then(|f| f["invisible"].as_array().cloned()).unwrap_or_default()
                .iter().filter_map(|x| x.as_str().map(String::from)).collect()
        };
        assert!(!inv("unlisted_whole").contains(&"wholelib".to_string()),
                "a COMPLETE report's silence is its purity claim — an absent `unanalyzed` must not be \
                 read as incompleteness, or every report ever written gets hedged:\n{v:#}");
        assert!(!inv("unlisted_empty").contains(&"emptylib".to_string()),
                "an explicitly EMPTY `unanalyzed` is a completeness claim and must buy coverage:\n{v:#}");
        let uncovered: Vec<String> = v["coverage"]["uncovered"].as_array().into_iter().flatten()
            .filter_map(|c| c["name"].as_str().map(String::from)).collect();
        assert!(!uncovered.contains(&"wholelib".to_string()) && !uncovered.contains(&"emptylib".to_string()),
                "the κ ledger must not name a crate whose report read all of its own source: {uncovered:?}\n{v:#}");
    }

    /// ⟨0.21⟩ A CHAINED REPORT THAT SAYS IT NEVER READ SOME OF ITS OWN SOURCE STILL BOUGHT SILENCE.
    ///
    /// SPEC §2 chaining rule 3 turns a report's SILENCE into a purity claim, and registering its crate as
    /// COVERED is exactly what silences the κ ledger's `invisible` hedge so the silence can be read that
    /// way. A report carrying a non-empty ⟨0.21⟩ `unanalyzed` has just said it never read some of its own
    /// source — so chaining it was strictly WORSE than not chaining it: the dependency's own gate refuses
    /// to certify itself over unanalyzed code (exit 2) and the consumer certified one on its behalf.
    ///
    ///     dep:      src/broken.rs fails to parse; `io::danger` vanishes with it
    ///               report: package "brokelib", unanalyzed:[src/broken.rs], no `brokelib#io::danger`
    ///     consumer: unlisted_broke() { brokelib::io::danger() }
    ///               unchained ->  invisible: ['brokelib']    the honest hedge
    ///               CHAINED   ->  ABSENT FROM THE REPORT     a ⟨0.21⟩ purity claim
    ///
    /// BOTH DIRECTIONS: the second (an answered key still answers, a COMPLETE report still covers) is
    /// `an_incomplete_reports_entries_are_applied_unchanged_and_a_complete_one_still_covers`, written
    /// first because narrowing coverage is exactly where the mirror defect lands (standing bar item 0).
    #[test]
    fn a_report_declaring_itself_incomplete_grants_no_ledger_coverage_exemption() {
        let d = std::env::temp_dir().join(format!("candor-incomplete-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        let me = format!("scan-{}", env!("CARGO_PKG_VERSION"));
        std::fs::write(d.join("report.brokelib.scan.json"), format!(r#"{{
            "candor": {{"version": "{me}", "toolchain": "stable", "spec": "0.23"}},
            "package": "brokelib",
            "unanalyzed": [{{"path": "src/broken.rs", "reason": "parse error"}}],
            "functions": [{{"fn": "io::go", "inferred": ["Exec"], "hash": "brokelib#io::go"}}]}}"#)).unwrap();
        std::fs::write(d.join("report.wholelib.scan.json"), format!(r#"{{
            "candor": {{"version": "{me}", "toolchain": "stable", "spec": "0.23"}},
            "package": "wholelib",
            "functions": [{{"fn": "io::go", "inferred": ["Exec"], "hash": "wholelib#io::go"}}]}}"#)).unwrap();
        let idx = load_dep_reports(Some(d.to_str().unwrap()));
        let _ = std::fs::remove_dir_all(&d);

        // The JOIN gate must still see the crate — coverage decides whether SILENCE is a claim, chaining
        // decides whether a key is worth ASKING, and conflating them costs rule 2's answers.
        assert!(idx.crates.contains("brokelib") && idx.crates.contains("wholelib"),
                "the join gate must still cover both: {:?}", idx.crates);
        assert!(idx.incomplete_pkgs.contains("brokelib"),
                "a report naming source it could not analyze must lose its coverage claim");
        assert!(!idx.incomplete_pkgs.contains("wholelib"), "the complete control must stay covered");
        // …and it is NOT the staleness treatment: nothing is distrusted, nothing is downgraded.
        assert!(idx.untrusted.is_empty(), "incompleteness is not staleness: {:?}", idx.untrusted);

        let src = "\
pub fn listed_broke() { brokelib::io::go(); }
pub fn unlisted_broke() { brokelib::io::danger(); }
pub fn unlisted_whole() { wholelib::io::danger(); }
";
        let v = scan_crate_chained("incomplete", "consumer",
            "\n[dependencies]\nbrokelib = \"1\"\nwholelib = \"1\"\n", src, &idx);
        let inv = |name: &str| -> Vec<String> {
            v["functions"].as_array().into_iter().flatten()
                .find(|f| f["fn"].as_str() == Some(name))
                .and_then(|f| f["invisible"].as_array().cloned()).unwrap_or_default()
                .iter().filter_map(|x| x.as_str().map(String::from)).collect()
        };
        // 1. THE DEFECT: a function whose only reach is into the self-declared-incomplete crate read as a
        //    confident purity claim, with no entry and no hedge.
        assert!(inv("unlisted_broke").contains(&"brokelib".to_string()),
                "a fn reaching only a crate whose report says it never read some of its own source must \
                 disclose it blind — the absent entry IS a purity claim (§2 rule 3):\n{v:#}");
        // 2. The disclosure is per-crate, not per-entry: the joined fn discloses too.
        assert!(inv("listed_broke").contains(&"brokelib".to_string()),
                "the hedge is a property of the crate, not of the key that missed:\n{v:#}");
        // 3. THE MIRROR: the complete report keeps its exemption. A report that read all of its own
        //    source is silent about its pure functions on purpose.
        assert!(!inv("unlisted_whole").contains(&"wholelib".to_string()),
                "a COMPLETE report's silence is its purity claim — re-disclosing it is the mirror \
                 defect (a false disclosure, worse than a missing one):\n{v:#}");
        // 4. The one ledger the stderr line, the gate advisory and the `coverage` field all share.
        let uncovered: Vec<String> = v["coverage"]["uncovered"].as_array().into_iter().flatten()
            .filter_map(|c| c["name"].as_str().map(String::from)).collect();
        assert_eq!(uncovered, vec!["brokelib".to_string()],
                   "the κ ledger must name the incomplete crate and only it\n{v:#}");
    }

    /// The MANIFEST SHAPES, as a denylist: only an ABSENT and an explicitly EMPTY `unanalyzed` buy
    /// coverage. Everything else — including shapes no conforming producer emits — fails CLOSED, because
    /// a completeness claim that cannot be read is not a claim. The two safe shapes are asserted in the
    /// second-direction fixture above; this is the closed half.
    #[test]
    fn only_an_absent_or_empty_unanalyzed_is_a_completeness_claim() {
        let me = format!("scan-{}", env!("CARGO_PKG_VERSION"));
        let probe = |unanalyzed: &str| -> bool {
            let field = if unanalyzed.is_empty() {
                String::new()
            } else {
                format!("\"unanalyzed\": {unanalyzed},")
            };
            let text = format!(r#"{{
                "candor": {{"version": "{me}", "toolchain": "stable", "spec": "0.23"}},
                "package": "p", {field}
                "functions": []}}"#);
            let v: serde_json::Value = serde_json::from_str(&text).expect("fixture json");
            crate::deps::declares_itself_incomplete(&v)
        };
        // COMPLETE — the two shapes a conforming producer actually writes.
        assert!(!probe(""), "an ABSENT `unanalyzed` is how this engine's writer says `I read everything`");
        assert!(!probe("[]"), "an explicitly EMPTY manifest is a completeness claim");
        // INCOMPLETE — the real one, plus every unreadable shape.
        assert!(probe(r#"[{"path": "a.rs", "reason": "parse error"}]"#), "the real shape");
        assert!(probe("null"), "a null completeness claim cannot be read — fail closed");
        assert!(probe(r#""some""#), "a string completeness claim cannot be read — fail closed");
        assert!(probe("{}"), "an object completeness claim cannot be read — fail closed");
        assert!(probe("3"), "a number completeness claim cannot be read — fail closed");
    }

    /// A CRATE CHAINED BOTH COMPLETE AND INCOMPLETE: rust does NOT let the complete report win.
    /// candor-swift subtracts (`incompletePkgs.subtract(coveredPkgs)`) and java re-registers per report.
    ///
    /// SAME TWO ARGUMENTS AS ITS FRESH-VS-STALE SIBLING, and the same one has expired — see
    /// `a_package_chained_both_fresh_and_stale_keeps_its_blind_spot_disclosure` for the full statement.
    /// The withdrawal argument ("a disagreeing key resolves to NOTHING, so granting coverage manufactures
    /// a false all-clear") is gone: entries are unioned now and the key resolves. What survives is that
    /// coverage is keyed by crate NAME while the collision exists precisely BECAUSE the name spans two
    /// versions — routine in a Cargo tree, 7 of 167 dep reports in candor-rust's own and 30 of 378 in
    /// ebman's — so a complete report cannot certify the silence of the version it never read.
    ///
    /// TO FLIP THIS, answer the version argument, not the withdrawal one. The mechanism is still
    /// `idx.incomplete_pkgs.subtract(…)` at the end of the load; the mechanism was never the hard part.
    #[test]
    fn a_crate_chained_both_complete_and_incomplete_keeps_its_blind_spot_disclosure() {
        let d = std::env::temp_dir().join(format!("candor-dualcomplete-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        let me = format!("scan-{}", env!("CARGO_PKG_VERSION"));
        // Both name package `duolib` and both carry `io::go` — DISAGREEING, which is the collision that
        // makes this a soundness question rather than a bookkeeping one.
        std::fs::write(d.join("report.whole.duolib.scan.json"), format!(r#"{{
            "candor": {{"version": "{me}", "toolchain": "stable", "spec": "0.23"}},
            "package": "duolib",
            "functions": [{{"fn": "io::go", "inferred": ["Exec"], "hash": "duolib#io::go"}}]}}"#)).unwrap();
        std::fs::write(d.join("report.broke.duolib.scan.json"), format!(r#"{{
            "candor": {{"version": "{me}", "toolchain": "stable", "spec": "0.23"}},
            "package": "duolib",
            "unanalyzed": [{{"path": "src/broken.rs", "reason": "parse error"}}],
            "functions": [{{"fn": "io::go", "inferred": [], "hash": "duolib#io::go"}}]}}"#)).unwrap();
        let idx = load_dep_reports(Some(d.to_str().unwrap()));
        let _ = std::fs::remove_dir_all(&d);
        assert!(idx.incomplete_pkgs.contains("duolib"),
                "a crate one of whose reports declares itself incomplete must keep the hedge — see the \
                 doc comment before aligning this with java/swift");
        // THE PREMISE, INVERTED with the union: the shared key resolves. An INCOMPLETE report's entries
        // are kept verbatim rather than downgraded (that is the ⟨0.21⟩ treatment, and the difference from
        // staleness is the whole point), so the union here is `Exec` from the whole report with nothing
        // added by the broken one — which is exactly right, since `inferred: []` contributes no effect.
        assert_eq!(idx.by_key.get("duolib#io::go").map(|d| d.effects.clone()),
                   Some(BTreeSet::from(["Exec"])),
                   "the complete report's `Exec` must survive the collision — withdrawing the key dropped \
                    it, and with the crate hedged the caller kept only a blind-spot note where it should \
                    carry the effect itself");
        let v = scan_crate_chained("dualcomplete", "consumer", "\n[dependencies]\nduolib = \"1\"\n",
            "pub fn hits_duo() { duolib::io::go(); }\n", &idx);
        let duo = v["functions"].as_array().into_iter().flatten()
            .find(|f| f["fn"].as_str() == Some("hits_duo")).cloned().unwrap_or(serde_json::Value::Null);
        assert!(duo != serde_json::Value::Null,
                "A FALSE ALL-CLEAR: the complete report says `duolib::io::go` runs `Exec`, the colliding \
                 key withdrew the answer, and with the crate counted as covered the call reads \
                 confidently PURE:\n{v:#}");
        assert_eq!(duo["invisible"], serde_json::json!(["duolib"]),
                   "the disclosure must NAME the crate whose report could not certify its own silence:\n{v:#}");
    }

    /// STALENESS IS CHECKED FIRST. A report that fails the §2.1 version check has already lost its
    /// coverage; asking whether it ALSO declares itself incomplete adds nothing but a second stderr line
    /// naming the same crate. Pinned so the two sets stay disjoint.
    #[test]
    fn a_stale_report_is_not_also_counted_incomplete() {
        let d = std::env::temp_dir().join(format!("candor-staleincomplete-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        std::fs::write(d.join("report.oldbroke.scan.json"), r#"{
            "candor": {"version": "scan-0.0.1", "toolchain": "stable", "spec": "0.3"},
            "package": "oldbroke",
            "unanalyzed": [{"path": "src/broken.rs", "reason": "parse error"}],
            "functions": [{"fn": "io::go", "inferred": ["Exec"], "hash": "oldbroke#io::go"}]}"#).unwrap();
        let idx = load_dep_reports(Some(d.to_str().unwrap()));
        let _ = std::fs::remove_dir_all(&d);
        assert!(idx.untrusted.contains("oldbroke"), "the §2.1 downgrade still applies");
        assert!(!idx.incomplete_pkgs.contains("oldbroke"),
                "a distrusted report's completeness claim buys it nothing — the two sets must stay \
                 disjoint or the disclosures double up: {:?}", idx.incomplete_pkgs);
        // …and the entries take the STALENESS treatment, not the incompleteness one.
        assert_eq!(idx.by_key.get("oldbroke#io::go").map(|e| e.effects.clone()), Some(BTreeSet::from(["Unknown"])));
    }

    /// ⟨0.24⟩ A CHAINED REPORT THAT JUDGED NOTHING (`analyzed.count: 0`) MUST NOT READ AS FULL COVERAGE —
    /// the third answer to "may this report's silence speak?", after staleness (§2.1) and incompleteness
    /// (⟨0.21⟩), and the last one the wire can currently express.
    ///
    /// THE DEFECT, exactly as conformance PART 26 found it four-way: a report carrying `functions: []` and
    /// `analyzed.count: 0` bought a consumer MORE confidence than not chaining the package at all.
    ///
    ///     dep:      facade crate, `pub use`es only — report: package "facadelib", analyzed.count 0
    ///     consumer: hits_facade() { facadelib::io::go() }
    ///               unchained ->  invisible: ['facadelib'] + coverage.uncovered    the honest hedge
    ///               CHAINED   ->  ABSENT FROM THE REPORT, no coverage field        a ⟨0.21⟩ purity claim
    ///
    /// STATE THE HARM PRECISELY, because the loose form sends you after the wrong symptom: the empty
    /// report carries no effects, so this arm cannot itself TRIP a gate — it and the unchained arm both
    /// exit 0 on `deny Fs`. What it DELETES is the DISCLOSURE (the `invisible` marker, the κ ledger, the
    /// verdict caveat, `--gate-json`'s coverage block). The gate flip exists only against the TRUSTED
    /// arm. So the fix restores the disclosure channel and must not manufacture a verdict — asserting an
    /// effect the consumer has no evidence for is the mirror sin.
    ///
    /// THE SECOND ARM IS A CONTROL, NOT A COURTESY. `functions: []` is equally the shape of a legitimate
    /// all-pure dependency, which §2 rule 3 requires a consumer to BELIEVE. Keyed on emptiness instead,
    /// this fixture's count-0 row would still go GREEN while the control failed — which is what a
    /// plausible-but-wrong fix looks like from the floor arm alone. Measured over 1997 JVM dependency
    /// jars: 79 count-0, only 6 granting coverage, against 104 legitimate all-pure. See
    /// [`candor_report::claims_to_have_judged_nothing`].
    #[test]
    fn a_report_that_judged_nothing_grants_no_ledger_coverage_exemption() {
        let d = std::env::temp_dir().join(format!("candor-judgednothing-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        let me = format!("scan-{}", env!("CARGO_PKG_VERSION"));
        // The two reports differ in ONE INTEGER and in nothing else. That is the whole experiment.
        std::fs::write(d.join("report.facadelib.scan.json"), format!(r#"{{
            "candor": {{"version": "{me}", "toolchain": "stable", "spec": "0.26"}},
            "package": "facadelib",
            "analyzed": {{"count": 0, "digest": "0"}},
            "functions": []}}"#)).unwrap();
        std::fs::write(d.join("report.purelib.scan.json"), format!(r#"{{
            "candor": {{"version": "{me}", "toolchain": "stable", "spec": "0.26"}},
            "package": "purelib",
            "analyzed": {{"count": 2, "digest": "0"}},
            "functions": []}}"#)).unwrap();
        let idx = load_dep_reports(Some(d.to_str().unwrap()));
        let _ = std::fs::remove_dir_all(&d);

        // CHAINED, NOT COVERED — the same split incompleteness takes. The join gate must still see the
        // crate (a contradictory count-0-WITH-entries report still answers its keys); only the claim that
        // its SILENCE is informative is withdrawn.
        assert!(idx.crates.contains("facadelib") && idx.crates.contains("purelib"),
                "the join gate must still cover both: {:?}", idx.crates);
        assert!(idx.judged_nothing_pkgs.contains("facadelib"),
                "a report whose `analyzed.count` is 0 judged nothing and must lose its coverage claim");
        assert!(!idx.judged_nothing_pkgs.contains("purelib"),
                "THE CONTROL: `count: 2` with `functions: []` is a believed all-pure claim (§2 rule 3) — \
                 hedging it would disable chained coverage rather than implement ⟨0.24⟩");
        // …and it is neither of the other two refusals: nothing distrusted, nothing declared incomplete.
        assert!(idx.untrusted.is_empty() && idx.incomplete_pkgs.is_empty(),
                "judging nothing is not staleness and not incompleteness: {:?} {:?}",
                idx.untrusted, idx.incomplete_pkgs);

        let src = "\
pub fn hits_facade() { facadelib::io::go(); }
pub fn hits_pure() { purelib::io::go(); }
";
        let v = scan_crate_chained("judgednothing", "consumer",
            "\n[dependencies]\nfacadelib = \"1\"\npurelib = \"1\"\n", src, &idx);
        let inv = |name: &str| -> Vec<String> {
            v["functions"].as_array().into_iter().flatten()
                .find(|f| f["fn"].as_str() == Some(name))
                .and_then(|f| f["invisible"].as_array().cloned()).unwrap_or_default()
                .iter().filter_map(|x| x.as_str().map(String::from)).collect()
        };
        // 1. THE FLOOR: a function whose only reach is into the crate that judged nothing must disclose it
        //    blind. Before this rung it was absent from the report entirely — a confident purity claim.
        assert!(inv("hits_facade").contains(&"facadelib".to_string()),
                "a fn reaching only a crate whose report judged NOTHING must disclose it blind — the \
                 absent entry IS a purity claim (§2 rule 3):\n{v:#}");
        // 2. THE CONTROL: the all-pure report keeps its exemption, and re-disclosing it would be a FALSE
        //    disclosure — the mirror defect, and worse than a missing one.
        assert!(!inv("hits_pure").contains(&"purelib".to_string()),
                "a report that JUDGED two units and found neither effectful is making a claim, and §2 \
                 rule 3 says believe it:\n{v:#}");
        // 3. The one ledger the stderr line, the `--gate-json` advisory and the `coverage` field share.
        let uncovered: Vec<String> = v["coverage"]["uncovered"].as_array().into_iter().flatten()
            .filter_map(|c| c["name"].as_str().map(String::from)).collect();
        assert_eq!(uncovered, vec!["facadelib".to_string()],
                   "the κ ledger must name the unjudged crate and only it\n{v:#}");
    }

    /// THE MANIFEST SHAPES for ⟨0.24⟩, as a table — the sibling of
    /// `only_an_absent_or_empty_unanalyzed_is_a_completeness_claim`, and keyed on the INTEGER throughout.
    /// The `has_entries` argument enters for EXACTLY ONE row (the manifest-less one, SPEC §2's third),
    /// which is what makes this a count rule rather than an emptiness rule.
    #[test]
    fn only_a_positive_analyzed_count_is_a_judgment_claim() {
        let probe = |analyzed: &str, has_entries: bool| -> bool {
            let field = if analyzed.is_empty() { String::new() } else { format!("\"analyzed\": {analyzed},") };
            let text = format!(r#"{{"package": "p", {field} "functions": []}}"#);
            let v: serde_json::Value = serde_json::from_str(&text).expect("fixture json");
            candor_report::claims_to_have_judged_nothing(&v, has_entries)
        };
        // JUDGED SOMETHING — the rows that keep their coverage.
        assert!(!probe(r#"{"count": 2, "digest": "0"}"#, false),
                "count 2 with `functions: []` is the LEGITIMATE all-pure claim — §2 rule 3 says believe it");
        assert!(!probe(r#"{"count": 1}"#, false),
                "the rule is about `count`; a digest-less manifest still names a judgment");
        assert!(!probe("", true),
                "a pre-⟨0.21⟩ producer that lists entries judged something and said so the only way it could");
        // JUDGED NOTHING — the floor, plus SPEC §2's third row, plus every unreadable shape.
        assert!(probe(r#"{"count": 0, "digest": "0"}"#, false), "the real shape: a facade crate");
        assert!(probe(r#"{"count": 0}"#, true),
                "…and the count OUTRANKS the entries: a contradictory count-0-with-entries report has \
                 still told us it judged nothing, and fabricating coverage from the contradiction is the \
                 confident direction");
        assert!(probe("", false), "SPEC §2 row 3: no manifest and no entries falls back to the unchained reading");
        assert!(probe(r#"{"count": -1}"#, false), "a negative count is not a judgment — fail closed");
        assert!(probe("null", false), "a null manifest cannot be read — fail closed");
        assert!(probe(r#""some""#, false), "a string manifest cannot be read — fail closed");
        assert!(probe("{}", false), "a manifest with no `count` cannot be read — fail closed");
        assert!(probe(r#"{"count": "2"}"#, false), "a non-numeric count cannot be read — fail closed");
        assert!(probe("7", false), "a scalar manifest cannot be read — fail closed");
    }

    /// A CRATE CHAINED BOTH JUDGED AND UNJUDGED keeps the hedge — the same conservative-on-conflict answer
    /// `a_crate_chained_both_complete_and_incomplete_keeps_its_blind_spot_disclosure` records, arriving on
    /// a third axis, and A DELIBERATE DIVERGENCE FROM candor-swift, which subtracts
    /// (`unjudgedPkgs.subtract(coveredPkgs)`) so the real report wins.
    ///
    /// Swift's argument is good: a count-0 report makes no claim in EITHER direction, so beside a real
    /// report it should be a no-op, and letting content-free bookkeeping withdraw an earned purity claim
    /// is the mirror sin. This engine still declines, for the reason its two neighbouring sets already
    /// decline: rust's index DROPS a key two dep entries disagree under, so a crate granted coverage on
    /// one report's authority can have the very key that mattered resolve to NOTHING and read confidently
    /// pure. Two reports for one crate name is routine in a Cargo tree. The cost of being wrong here is
    /// one extra hedge; the cost of being wrong the other way is a false all-clear.
    ///
    /// TO FLIP THIS, if a four-way ruling goes the other way: subtract the crates some report actually
    /// judged from `idx.judged_nothing_pkgs` at the end of the load, exactly as `incomplete_pkgs`
    /// documents for its own axis.
    #[test]
    fn a_crate_chained_both_judged_and_unjudged_keeps_its_blind_spot_disclosure() {
        let d = std::env::temp_dir().join(format!("candor-dualjudged-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        let me = format!("scan-{}", env!("CARGO_PKG_VERSION"));
        std::fs::write(d.join("report.real.twolib.scan.json"), format!(r#"{{
            "candor": {{"version": "{me}", "toolchain": "stable", "spec": "0.26"}},
            "package": "twolib",
            "analyzed": {{"count": 4, "digest": "0"}},
            "functions": [{{"fn": "io::go", "inferred": ["Exec"], "hash": "twolib#io::go"}}]}}"#)).unwrap();
        std::fs::write(d.join("report.stub.twolib.scan.json"), format!(r#"{{
            "candor": {{"version": "{me}", "toolchain": "stable", "spec": "0.26"}},
            "package": "twolib",
            "analyzed": {{"count": 0, "digest": "0"}},
            "functions": []}}"#)).unwrap();
        let idx = load_dep_reports(Some(d.to_str().unwrap()));
        let _ = std::fs::remove_dir_all(&d);
        assert!(idx.judged_nothing_pkgs.contains("twolib"),
                "a crate one of whose reports judged nothing keeps the hedge — see the doc comment before \
                 aligning this with swift");
        // The ANSWERED key still answers: this withholds coverage, it never withdraws an effect.
        assert_eq!(idx.by_key.get("twolib#io::go").map(|e| e.effects.clone()), Some(BTreeSet::from(["Exec"])),
                   "withholding coverage must not touch the entries — the change is strictly additive");
        let v = scan_crate_chained("dualjudged", "consumer", "\n[dependencies]\ntwolib = \"1\"\n",
            "pub fn hits_two() { twolib::io::go(); }\npub fn hits_unlisted() { twolib::io::other(); }\n", &idx);
        let f = |name: &str| v["functions"].as_array().into_iter().flatten()
            .find(|f| f["fn"].as_str() == Some(name)).cloned().unwrap_or(serde_json::Value::Null);
        assert_eq!(f("hits_two")["inferred"], serde_json::json!(["Exec"]),
                   "the answered key keeps its effect:\n{v:#}");
        assert_eq!(f("hits_unlisted")["invisible"], serde_json::json!(["twolib"]),
                   "…and the unanswered one discloses rather than reading pure:\n{v:#}");
    }

    /// STALENESS AND INCOMPLETENESS ARE CHECKED FIRST, so the THREE disclosure sets stay disjoint and one
    /// crate never draws two stderr lines saying the same thing. The precedence lives in exactly one
    /// place — `cover`'s branch order — and this is what detects a reordering.
    #[test]
    fn a_stale_or_incomplete_report_is_not_also_counted_judged_nothing() {
        let d = std::env::temp_dir().join(format!("candor-precedence-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        let me = format!("scan-{}", env!("CARGO_PKG_VERSION"));
        // Both ALSO judged nothing — so if the branch order ever inverts, both land in the wrong set.
        std::fs::write(d.join("report.oldstub.scan.json"), r#"{
            "candor": {"version": "scan-0.0.1", "toolchain": "stable", "spec": "0.3"},
            "package": "oldstub", "analyzed": {"count": 0, "digest": "0"}, "functions": []}"#).unwrap();
        std::fs::write(d.join("report.brokestub.scan.json"), format!(r#"{{
            "candor": {{"version": "{me}", "toolchain": "stable", "spec": "0.26"}},
            "package": "brokestub",
            "unanalyzed": [{{"path": "src/broken.rs", "reason": "parse error"}}],
            "analyzed": {{"count": 0, "digest": "0"}}, "functions": []}}"#)).unwrap();
        let idx = load_dep_reports(Some(d.to_str().unwrap()));
        let _ = std::fs::remove_dir_all(&d);
        assert!(idx.untrusted.contains("oldstub"), "the §2.1 downgrade outranks ⟨0.24⟩");
        assert!(idx.incomplete_pkgs.contains("brokestub"), "the ⟨0.21⟩ refusal outranks ⟨0.24⟩");
        assert!(idx.judged_nothing_pkgs.is_empty(),
                "a crate already refused by an earlier gate must not ALSO be counted judged-nothing — the \
                 three sets must stay disjoint or the disclosures double up: {:?}", idx.judged_nothing_pkgs);
    }

    /// `ambiguous:same-name local defs` IS THE FIFTH KIND IN SPEC §4's CLOSED VOCABULARY ⟨0.24⟩, AND THE
    /// RENAME WAS REFUSED WITH A NUMBER. The full §4 argument lives at the emission site in `scan.rs`;
    /// this pins the two facts a future edit would silently change — the KIND and the CLASS it projects to.
    ///
    /// It was OUTSIDE the vocabulary until ⟨0.24⟩, and this engine is the reason it is in: rust is the
    /// only PRODUCER, so rust was the only engine the omission made non-conforming — §6.2's class table
    /// named `ambiguous*` all along, so every CONSUMER classified it correctly and nobody complained.
    ///
    /// The counterfactual, built and run rather than argued: with `ambiguous*` reclassified `indirect`
    /// (one line in `ReasonClass::classify`, both binaries kept by content hash), `deny E Unknown[dispatch]`
    /// goes from **58 of 200 crates.io crates to 0 of 200**, and exit 1 -> exit 0 on pgman, ebman and
    /// candor-rust. It is not a narrowing, it is a deletion: every other `dispatch:` this engine emits (20
    /// in a 1062-report census, all `dispatch:untyped cross-package receiver`) needs a chained dependency
    /// to exist at all. candor-ts's `5ba301c` is a precedent that the SHAPE of reclassification can be
    /// safe, not evidence that this one is — there, every reclassified reason named NOTHING.
    ///
    /// The kind's real-world weight, so nobody re-opens this thinking it is a corner: `ambiguous:` is
    /// **8710 of 19607** `unknownWhy` entries over a 1062-report census (more than `callback:`'s 9421 is
    /// away from it), across 220 packages — the cfg-gated-alternative-definitions shape, which a syntactic
    /// scan cannot resolve because it does not evaluate `cfg`.
    ///
    /// TO CHANGE IT: a SPEC rung, not an engine edit. That is exactly how it changed — §4 ⟨0.24⟩ cites
    /// the 58/200 → 0/200 counterfactual above as its reason for admitting the kind rather than deleting
    /// it, so the number and the vocabulary now stand or fall together. Conformance PART 10 scans a
    /// purpose-built fixture, so the kind is VISIBLE there instead of silently absent.
    ///
    /// THE OTHER HALF OF THE VOCABULARY: this engine has none. A `kind` lives here only as the raw
    /// `kind:detail` string, read back through `ReasonClass::classify`'s prefix table — there is no
    /// typed kind enum to drift out of step with it (§4 ⟨0.24⟩ "AN ENGINE HOLDS THIS VOCABULARY TWICE").
    /// `off_vocabulary_kinds_round_trip_and_classify_through_the_catch_all` is the control that a
    /// FABRICATED kind still behaves per §2, so "added a fifth kind" and "stopped checking the kind set"
    /// are not the same diff.
    #[test]
    fn the_ambiguous_reason_kind_and_its_class_are_pinned() {
        let d = std::env::temp_dir().join(format!("candor-ambkind-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(d.join("src")).unwrap();
        std::fs::write(d.join("Cargo.toml"), "[package]\nname = \"ambkind\"\n").unwrap();
        // The shape that actually produces it on real code: cfg-gated alternative definitions of one free
        // function. Rust picks by `cfg`; a syntactic scan cannot, and picking would fabricate one arm's
        // effects onto the other — so it discloses.
        std::fs::write(d.join("src/lib.rs"), "\
#[cfg(unix)]
pub fn helper() { std::fs::read(\"/etc/a\").ok(); }
#[cfg(windows)]
pub fn helper() { println!(\"pure\"); }
pub fn go() { helper(); }
").unwrap();
        let prefix = d.join("out/r").to_string_lossy().into_owned();
        let (rc, body) = scan_one(&d.to_string_lossy(), ScanOpts {
            prefix, want_json: true, include_tests: false, policy: None, baseline: None,
            ws_member: false, quiet: true, deps_idx: &DepIndex::default(), peek_excluded: false,
        }, &crate::gate::begin_run());
        assert_eq!(rc, 0);
        let v: serde_json::Value = serde_json::from_str(&body.unwrap()).unwrap();
        let _ = std::fs::remove_dir_all(&d);
        let go = fn_entry(&v, "go");
        assert!(effs(go).contains(&"Unknown".to_string()),
                "an ambiguous bare call must DISCLOSE, not drop the edge silently:\n{v:#}");
        assert_eq!(go["unknownWhy"], serde_json::json!(["ambiguous:same-name local defs"]),
                   "the KIND is load-bearing — see the doc comment before renaming it:\n{v:#}");
        // …and the class it projects to, which is what `deny E Unknown[dispatch]` resolves.
        assert_eq!(
            candor_classify::policy::ReasonClass::classify("ambiguous:same-name local defs"),
            candor_classify::policy::ReasonClass::Dispatch,
            "SPEC §6.2's table maps `ambiguous*` to `dispatch`; moving it to `indirect` takes \
             `deny E Unknown[dispatch]` from 58/200 crates.io crates to 0/200"
        );
    }

    #[test]
    fn dep_index_carries_the_full_qual_as_a_third_key() {
        // The index held only `crate#leaf` and `crate#tail2`, so a consumer that knows its target
        // PRECISELY (`deplib#sync::Client::fetch`) had no key to ask on and had to settle for tail2 —
        // where `sync::Client` and `mock::Client` are the same string. The full qual is the third key.
        //
        // The SECOND direction is the one that could go wrong (standing bar item 0): a 1- or 2-segment
        // qual's "full qual" IS its leaf/tail2 string, so an undeduped push self-collides. Under the
        // never-guess rule that DROPPED a key that worked before — a silent under-report introduced by a
        // purely additive change. The entry union has since made that accident harmless (unioning an entry
        // with itself is the identity), so these two rows now guard the dedup as a COST property rather
        // than a soundness one. Kept asserted because a reader should not have to re-derive which it is.
        let d = std::env::temp_dir().join(format!("candor-fullqual-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        let _ = std::fs::create_dir_all(&d);
        let me = format!("scan-{}", env!("CARGO_PKG_VERSION"));
        std::fs::write(d.join("report.deplib.scan.json"), format!(r#"{{
            "candor": {{"version": "{me}", "toolchain": "stable", "spec": "0.23"}},
            "functions": [
              {{"fn": "sync::Client::fetch", "inferred": ["Net"], "hash": "deplib#sync::Client::fetch"}},
              {{"fn": "mock::Client::fetch", "inferred": [], "hash": "deplib#mock::Client::fetch"}},
              {{"fn": "Root::only", "inferred": ["Fs"], "hash": "deplib#Root::only"}},
              {{"fn": "bare", "inferred": ["Exec"], "hash": "deplib#bare"}}
            ]}}"#)).unwrap();
        let idx = load_dep_reports(Some(d.to_str().unwrap()));
        // 1. the NEW key: the full qual distinguishes what the leaf and the tail2 cannot.
        assert_eq!(idx.by_key.get("deplib#sync::Client::fetch").map(|e| e.effects.clone()),
                   Some(BTreeSet::from(["Net"])), "full-qual key missing for the effectful module's Client::fetch");
        assert_eq!(idx.by_key.get("deplib#mock::Client::fetch").map(|e| e.effects.clone()),
                   Some(BTreeSet::new()), "full-qual key missing for the pure module's Client::fetch");
        // …and the imprecise shapes UNION the candidates rather than withdrawing them. MEASURED on the
        // binary, same fixture, the `by_key` rule the only difference: a consumer calling an unresolvable
        // `deplib::fetch()` was ABSENT FROM `functions` ENTIRELY — zero entries, no `Unknown`, no
        // `invisible`, no hedge of any kind — while the call may reach `sync::Client::fetch` and its `Net`.
        // Under ⟨0.21⟩ that absence is a positive purity claim: the cardinal sin, on the imprecise key,
        // inside ONE report. After: `inferred: ["Net"]`.
        //
        // THIS IS A DIFFERENT POPULATION FROM THE ONE ENTRY-COLLISION-DECISION.md MEASURED, which is why
        // the union is applied here too rather than only across reports. That note's corpus evidence — every
        // disagreement is one function at two crate VERSIONS, so the union is the correct answer rather than
        // a hedge — was collected ACROSS reports. This collision is WITHIN one: two genuinely different
        // functions sharing a leaf, the case the note reserved as the union's real cost. The measurement
        // above says both populations carry the same defect, so both take the same rule.
        //
        // AND IT IS NOT THE FABRICATION MIRROR. Charging `Net` to an unresolved `deplib::fetch()` is not a
        // wrong answer, it is the honest over-approximation of a call whose target the consumer cannot
        // determine — the runtime may reach either body. The precision that matters is preserved by the KEY
        // SCHEME rather than by withdrawal: a consumer that CAN name its target asks the full qual and still
        // gets `[]` for the pure one, which is exactly what the two assertions above pin.
        assert_eq!(idx.by_key.get("deplib#Client::fetch").map(|e| e.effects.clone()),
                   Some(BTreeSet::from(["Net"])),
                   "a shared tail2 must UNION its candidates — withdrawing it made the caller vanish from \
                    `functions`, a purity claim over a call that may reach `Net`");
        assert_eq!(idx.by_key.get("deplib#fetch").map(|e| e.effects.clone()),
                   Some(BTreeSet::from(["Net"])),
                   "a shared leaf must UNION its candidates, for the same reason as the tail2");
        // 2. the SECOND direction: a short qual whose full qual EQUALS its tail2 / leaf must keep the
        //    key it already had — the dedup, not a self-collision that removes it.
        assert_eq!(idx.by_key.get("deplib#Root::only").map(|e| e.effects.clone()), Some(BTreeSet::from(["Fs"])),
                   "a 2-segment qual self-collided and dropped its own tail2 key");
        assert_eq!(idx.by_key.get("deplib#only").map(|e| e.effects.clone()), Some(BTreeSet::from(["Fs"])),
                   "a 2-segment qual's leaf key was dropped");
        assert_eq!(idx.by_key.get("deplib#bare").map(|e| e.effects.clone()), Some(BTreeSet::from(["Exec"])),
                   "a 1-segment qual self-collided and dropped its own leaf key");
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn dep_report_package_field_registers_coverage_for_the_ledger_exemption() {
        // SPEC §2 chaining rule 3: the crates a loaded report COVERS come from its envelope
        // `package`/`packages` field — independent of the file's NAME and of any join firing. An
        // EMPTY report ({functions: []}) is an all-pure purity claim: covered, never a κ blind spot.
        let d = std::env::temp_dir().join(format!("candor-deppkg-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        let _ = std::fs::create_dir_all(&d);
        let me = format!("scan-{}", env!("CARGO_PKG_VERSION"));
        // deliberately NOT the `….<crate>.scan.json` filename shape — only the envelope names it.
        std::fs::write(d.join("purity-claim.json"), format!(r#"{{
            "candor": {{"version": "{me}", "toolchain": "stable", "spec": "0.23"}},
            "package": "dep-c",
            "functions": []}}"#)).unwrap();
        std::fs::write(d.join("multi.json"), format!(r#"{{
            "candor": {{"version": "{me}", "toolchain": "stable", "spec": "0.23"}},
            "packages": ["alpha", "beta"],
            "functions": []}}"#)).unwrap();
        let idx = load_dep_reports(Some(d.to_str().unwrap()));
        assert!(idx.crates.contains("dep-c"), "the envelope `package` field registers coverage");
        assert!(idx.crates.contains("dep_c"), "a hyphenated package also registers in Rust ident form");
        assert!(idx.crates.contains("alpha") && idx.crates.contains("beta"),
                "the JVM-shape `packages` array registers every covered package");
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn every_dep_join_site_carries_the_whole_surface_not_just_the_effect() {
        // THREE places charged a chained dep entry to a caller and they had DRIFTED — the same shape
        // that left candor-java's ⟨0.19⟩ reason class on the hand-off path and off the ordinary call
        // path while a conformance-pinned gate read green (`6ab26e4`). Here the drift was in the
        // DISCLOSURE surfaces: the cross-crate DROP-GLUE join carried only effects + paths, and the
        // dep-LAZY join carried no `invisible` and no `incomplete`. A join that carries the effect and
        // drops the `incomplete` beside it lets a benign literal in the consumer certify a surface the
        // dependency already declared uncertifiable.
        //
        // An A/B cannot show this: none of the three corpora exercise the drop or lazy site, so all
        // three arms agree. The fixture is the evidence (standing bar item 8).
        let dep = std::env::temp_dir().join(format!("candor-depsurface-rep-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dep);
        let _ = std::fs::create_dir_all(&dep);
        let me = format!("scan-{}", env!("CARGO_PKG_VERSION"));
        // One entry per join site, each with a DISTINCT value in every surface, so a site that drops
        // one is named by the assertion rather than masked by another site's contribution.
        std::fs::write(dep.join("report.deplib.scan.json"), format!(r#"{{
            "candor": {{"version": "{me}", "toolchain": "stable", "spec": "0.23"}},
            "package": "deplib",
            "functions": [
              {{"fn": "Guard::drop", "inferred": ["Net"], "hosts": ["drop.example"], "cmds": ["dropcmd"],
                "paths": ["/drop/path"], "tables": ["drop_tab"], "invisible": ["dropblind"],
                "incomplete": ["Net"], "hash": "deplib#Guard::drop"}},
              {{"fn": "<lazy>::CFG", "inferred": ["Fs"], "hosts": ["lazy.example"], "cmds": ["lazycmd"],
                "paths": ["/lazy/path"], "tables": ["lazy_tab"], "invisible": ["lazyblind"],
                "incomplete": ["Fs"], "hash": "deplib#<lazy>::CFG"}},
              {{"fn": "io::fetch", "inferred": ["Db"], "hosts": ["call.example"], "cmds": ["callcmd"],
                "paths": ["/call/path"], "tables": ["call_tab"], "invisible": ["callblind"],
                "incomplete": ["Db"], "hash": "deplib#io::fetch"}}
            ]}}"#)).unwrap();
        let idx = load_dep_reports(Some(dep.to_str().unwrap()));
        let d = std::env::temp_dir().join(format!("candor-depsurface-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(d.join("src")).unwrap();
        std::fs::write(d.join("Cargo.toml"),
            "[package]\nname = \"consumer\"\n\n[dependencies]\ndeplib = \"1\"\n").unwrap();
        std::fs::write(d.join("src/lib.rs"), "\
pub fn by_drop_glue() { let _g = deplib::Guard; }
pub fn by_lazy_force() { let _c = deplib::CFG; }
pub fn by_ordinary_call() { deplib::io::fetch(); }
").unwrap();
        let prefix = d.join("out/r").to_string_lossy().into_owned();
        let (rc, body) = scan_one(&d.to_string_lossy(), ScanOpts {
            prefix, want_json: true, include_tests: false, policy: None, baseline: None, ws_member: false, quiet: true, deps_idx: &idx, peek_excluded: false,
        }, &crate::gate::begin_run());
        assert_eq!(rc, 0);
        let v: serde_json::Value = serde_json::from_str(&body.unwrap()).unwrap();
        let _ = std::fs::remove_dir_all(&d);
        let _ = std::fs::remove_dir_all(&dep);
        let surfaces = |fname: &str, key: &str| -> Vec<String> {
            v["functions"].as_array().into_iter().flatten()
                .find(|f| f["fn"].as_str() == Some(fname))
                .and_then(|f| f[key].as_array().cloned())
                .unwrap_or_default()
                .iter().filter_map(|x| x.as_str().map(String::from)).collect()
        };
        for (site, eff, host, cmd, path, table, blind) in [
            ("by_drop_glue",     "Net", "drop.example", "dropcmd", "/drop/path", "drop_tab", "dropblind"),
            ("by_lazy_force",    "Fs",  "lazy.example", "lazycmd", "/lazy/path", "lazy_tab", "lazyblind"),
            ("by_ordinary_call", "Db",  "call.example", "callcmd", "/call/path", "call_tab", "callblind"),
        ] {
            assert!(surfaces(site, "inferred").contains(&eff.to_string()),
                    "{site}: effect not inherited\n{v}");
            assert!(surfaces(site, "hosts").contains(&host.to_string()),
                    "{site}: dep hosts dropped by this join site\n{v}");
            assert!(surfaces(site, "cmds").contains(&cmd.to_string()),
                    "{site}: dep cmds dropped by this join site\n{v}");
            assert!(surfaces(site, "paths").contains(&path.to_string()),
                    "{site}: dep paths dropped by this join site\n{v}");
            assert!(surfaces(site, "tables").contains(&table.to_string()),
                    "{site}: dep tables dropped by this join site\n{v}");
            assert!(surfaces(site, "invisible").contains(&blind.to_string()),
                    "{site}: dep `invisible` dropped — the consumer's pure-ish verdict lost its caveat\n{v}");
            assert!(surfaces(site, "incomplete").contains(&eff.to_string()),
                    "{site}: dep `incomplete` dropped — a benign literal here could now certify the \
                     dep's invisible endpoint\n{v}");
        }
    }

    /// Scan `src` as crate `name`, chained over `idx`, and return the report.
    fn scan_crate_chained(tag: &str, name: &str, manifest_extra: &str, src: &str, idx: &DepIndex) -> serde_json::Value {
        // `tag` keeps two tests' temp trees APART. Without it both wrote `candor-ts-deplib-<pid>` and,
        // running in parallel in one process, each read the other's crate — standing-bar item 7 in
        // miniature (a stale/foreign output read back as this measurement's result).
        // …AND A PER-CALL COUNTER, because tag+name+pid is not unique ENOUGH: cargo runs tests as parallel
        // THREADS of one process, so `process::id()` is shared, and any two calls agreeing on (tag, name)
        // race on `remove_dir_all` + `create_dir_all` + `write`. Measured at a 25% failure rate over 20
        // runs — `write(src/lib.rs)` returning NotFound because a concurrent call had just removed the
        // tree between this call's `create_dir_all` and its write. Serial (`--test-threads=1`) never failed.
        static SEQ: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        let seq = SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let d = std::env::temp_dir().join(format!("candor-ts-{tag}-{name}-{}-{seq}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(d.join("src")).unwrap();
        std::fs::write(d.join("Cargo.toml"), format!("[package]\nname = \"{name}\"\n{manifest_extra}")).unwrap();
        std::fs::write(d.join("src/lib.rs"), src).unwrap();
        let prefix = d.join("out/r").to_string_lossy().into_owned();
        let (rc, body) = scan_one(&d.to_string_lossy(), ScanOpts {
            prefix, want_json: true, include_tests: false, policy: None, baseline: None, ws_member: false, quiet: true, deps_idx: idx, peek_excluded: false,
        }, &crate::gate::begin_run());
        assert_eq!(rc, 0);
        let v = serde_json::from_str(&body.unwrap()).unwrap();
        let _ = std::fs::remove_dir_all(&d);
        v
    }

    /// Write `report` where `load_dep_reports` will find it, and load the index.
    fn chain(tag: &str, krate: &str, report: &serde_json::Value) -> (DepIndex, std::path::PathBuf) {
        let d = std::env::temp_dir().join(format!("candor-tsdep-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        std::fs::write(d.join(format!("report.{krate}.scan.json")), report.to_string()).unwrap();
        (load_dep_reports(Some(d.to_str().unwrap())), d)
    }

    fn ts_entry<'a>(v: &'a serde_json::Value, name: &str) -> Option<&'a serde_json::Value> {
        v["functions"].as_array()?.iter().find(|f| f["fn"].as_str() == Some(name))
    }
    fn effects_of(v: &serde_json::Value, name: &str) -> Vec<String> {
        ts_entry(v, name)
            .and_then(|e| e["inferred"].as_array().cloned())
            .unwrap_or_default()
            .iter().filter_map(|x| x.as_str().map(String::from)).collect()
    }

    #[test]
    fn type_surface_returns_is_fully_qualified_and_refuses_a_same_leaf_sibling() {
        // ⟨typeSurface.returns⟩ — DEP-RECEIVER-TYPING-DESIGN.md half 2, requirement (a), and the
        // SECOND FIXTURE is the whole reason this test exists (standing bar item 0).
        //
        // The reverted attempt published `{crate}#{leaf}` on both ends, so `sync::Client` and
        // `mock::Client` were one string and a PURE `mock_client()` factory let `sync::Client`'s effect
        // be charged to a caller that cannot reach it. THIS FIXTURE REPRODUCED THAT AGAIN during
        // implementation, by a different door — a BARE `-> Client` written inside `mod mock` is
        // module-RELATIVE, and `expand` leaves it bare, so a suffix match happily picked `sync::Client`.
        // Both halves are asserted: the sibling must NOT resolve, and the real one must.
        let dep_src = "\
pub mod sync {
    pub struct Client;
    impl Client {
        pub fn send(&self) -> String { std::fs::read_to_string(\"/etc/secret\").unwrap_or_default() }
    }
    pub fn client() -> Client { Client }
}
pub mod mock {
    pub struct Client;
    impl Client { pub fn send(&self) -> String { String::new() } }
    pub fn client() -> Client { Client }
}
pub fn mock_client() -> mock::Client { mock::Client }
pub fn real_client() -> sync::Client { sync::Client }
";
        let dep = scan_crate_chained("qual", "deplib", "", dep_src, &DepIndex::default());
        // PRODUCER. The published id is a full qual, and the two same-leaf types are distinguishable.
        let ts = &dep["typeSurface"]["returns"];
        assert_eq!(ts["deplib#sync::client"].as_str(), Some("deplib#sync::Client"),
                   "the effectful module's factory must publish its FULL type qual:\n{dep}");
        assert_eq!(ts["deplib#real_client"].as_str(), Some("deplib#sync::Client"), "{dep}");
        // `mock::Client` has no non-pure member, so nothing about it may be published — and crucially
        // it must not inherit `sync::Client`'s id off a shared leaf.
        assert!(ts.get("deplib#mock::client").is_none(),
                "a bare `-> Client` inside `mod mock` resolved to ANOTHER module's type — defect 1:\n{dep}");
        assert!(ts.get("deplib#mock_client").is_none(),
                "an explicitly `-> mock::Client` factory published a `sync::Client` id:\n{dep}");
        // CONSUMER, both directions.
        let (idx, dir) = chain("qual", "deplib", &dep);
        let app = scan_crate_chained("qual", "app", "\n[dependencies]\ndeplib = \"1\"\n", "\
pub fn uses_real() -> String { let c = deplib::real_client(); c.send() }
pub fn uses_mod_real() -> String { let c = deplib::sync::client(); c.send() }
pub fn uses_mock() -> String { let c = deplib::mock_client(); c.send() }
pub fn uses_mod_mock() -> String { let c = deplib::mock::client(); c.send() }
", &idx);
        let _ = std::fs::remove_dir_all(&dir);
        for real in ["uses_real", "uses_mod_real"] {
            assert!(effects_of(&app, real).contains(&"Fs".to_string()),
                    "{real}: the genuine reach was NOT recovered — a narrowing that closed the real case \
                     too is the other half of item 0:\n{app}");
            // requirement (d): every surface the main join carries, not just the effect.
            let paths = ts_entry(&app, real).and_then(|e| e["paths"].as_array().cloned()).unwrap_or_default();
            assert!(paths.iter().any(|p| p.as_str() == Some("/etc/secret")),
                    "{real}: the dep's literal path surface was dropped by the typeSurface join:\n{app}");
        }
        for mock in ["uses_mock", "uses_mod_mock"] {
            let eff = effects_of(&app, mock);
            assert!(!eff.contains(&"Fs".to_string()),
                    "{mock}: FABRICATED the sibling type's Fs onto a caller that cannot reach it:\n{app}");
            assert!(eff.contains(&"Unknown".to_string()),
                    "{mock}: not resolvable and not disclosed — silence is the cardinal sin:\n{app}");
        }
    }

    /// A GENERIC INSTANTIATION IS NOT A WRAPPER, and refusing both cost a whole class of dependency.
    ///
    /// `bound_return_type` used to refuse ANY path carrying a type argument, because one "means a WRAPPER
    /// (`Result<_>`/`Option<_>`) or a generic instantiation (`Wrapper<T>` — the design note's open
    /// question, deliberately left unanswered)". Two cases, one door, and only the first needed refusing:
    /// a `let d = dep::now();` where `now() -> DateTime<Utc>` holds a **DateTime**, and DateTime's methods
    /// ARE that binding's methods.
    ///
    /// Keying on the OUTER path is right for both and is the exact opposite of the reverted defect, which
    /// UNWRAPPED (`Result<Conn,E>` → `Conn`). This never looks inside the angle brackets.
    ///
    /// MEASURED on the real `chrono`: `bound_returns` 245 → 758, published 45 → 108, and
    /// `offset::utc::Utc::now` — whose entry already carries `Clock` — went from publishing no return type
    /// to `chrono#datetime::DateTime`. End to end on the fixture below the consumer goes
    /// `['Clock','Unknown']` → `['Clock','Fs']`: DETERMINATION replacing disclosure, the ⟨0.24⟩ ordering.
    ///
    /// THE WORK QUEUE HAD THE CAUSE WRONG, which is why this test spells the mechanism out. It filed the
    /// missing entry as a SPURIOUS COLLISION — chrono declares `now()` twice under mutually exclusive
    /// `#[cfg]`s, so "the return index sees two same-named defs and the never-guess rule drops the entry
    /// even though both name the same type". It does not: a `#[cfg]`-duplicated NON-generic return
    /// publishes fine (asserted below), and chrono's entry never reached the collision rule at all. The
    /// generic was the whole cause; the duplication was a coincidence of the crate that surfaced it.
    /// THE COST OF BINDING MORE RETURNS, pinned so it stays deliberate. Found by self-review, correcting a
    /// claim this commit's message made and got wrong ("strictly additive — it can only turn `None` into
    /// `Some`").
    ///
    /// Binding generic instantiations means `build_type_surface` sees collisions it could not see before,
    /// and a collision DROPS the key. Measured against the parent commit, on a fn declared twice under
    /// mutually exclusive `#[cfg]`s whose arms return DIFFERENT types:
    ///
    ///     before   returns: {"ar#mk": "ar#A"}   published = 1
    ///     after    returns: (absent)            published = 0
    ///
    /// THE DROP IS CORRECT AND THE OLD BEHAVIOUR WAS THE DEFECT. `let x = mk();` holds an `A` or a `W<A>`
    /// depending on target, so publishing `ar#A` unconditionally was true on ONE target and asserted on
    /// both. Before the fix the generic arm did not bind at all, so the disagreement was invisible and one
    /// arm's answer went out as if it were the only one. This is never-guess working on evidence it could
    /// not previously see — and it is the exact opposite of the `#[cfg]` case one test down, where both
    /// arms name the SAME type and the key must survive. Both rows are needed: a "fix" that made
    /// `#[cfg]` pairs always publish would pass that one and reintroduce this.
    #[test]
    fn type_surface_drops_a_cfg_pair_that_returns_different_types() {
        let dep = scan_crate_chained("cfgdiff", "deplib", "", "\
pub struct A; impl A { pub fn touch(&self) { let _ = std::fs::read(\"/x\"); } }
pub struct W<T> { pub t: T }
impl W<A> { pub fn touch(&self) { let _ = std::fs::read(\"/x\"); } }
#[cfg(not(target_arch = \"wasm32\"))]
pub fn mk() -> A { A }
#[cfg(target_arch = \"wasm32\")]
pub fn mk() -> W<A> { W { t: A } }
", &DepIndex::default());
        let ts = &dep["typeSurface"]["returns"];
        assert!(ts.get("deplib#mk").is_none(),
                "two `#[cfg]` arms returning DIFFERENT types must withdraw the key — publishing either one \
                 asserts on both targets what is true on one:\n{dep}");
    }

    /// ⟨2026-08-29 ADVERSARIAL REVIEW, finding 2 — MEASURED AND ACCEPTED, NOT FIXED, see the doc note at
    /// `blind_direct`'s insertion site in scan.rs for the full argument⟩ Two `#[cfg(...)]` branches of one
    /// same-named function — cargo-util's real `crates/cargo-util/src/paths.rs` shape, a
    /// `#[cfg(target_os = "macos")]` body calling an unmodelled FFI crate beside a `#[cfg(not(...))]`
    /// no-op stub — print TWO entries (by design: `a_qualified_name_carried_by_two_cfg_gated_units_yields_one_violation_not_two`
    /// pins that the report must keep one entry per declaration, own `loc` each), and BOTH carry the SAME
    /// `invisible` disclosure because `blind_direct` is keyed on the bare qualified name the two branches
    /// share. THIS TEST PINS THE ACCEPTED SHAPE so a future change either preserves it deliberately or
    /// revisits this decision explicitly, rather than drifting: the union is a TRUE, SAFE-DIRECTION
    /// statement about the name ("`get_thing` reaches `core_foundation` under some configuration") and
    /// never hides a real effect — over-report noise, not the family's cardinal sin.
    #[test]
    fn cfg_branch_pair_shares_one_invisible_disclosure_stated_residual() {
        let v = scan_crate_chained("cfgcollide", "cfgvictim", "\n[dependencies]\ncore-foundation = \"0.9\"\n", "\
#[cfg(target_os = \"macos\")]
pub fn get_thing() {
    core_foundation::something();
}

#[cfg(not(target_os = \"macos\"))]
pub fn get_thing() {
    // no-op stub
}
", &DepIndex::default());
        let matches: Vec<&serde_json::Value> = v["functions"].as_array().unwrap()
            .iter().filter(|f| f["fn"].as_str() == Some("get_thing")).collect();
        assert_eq!(matches.len(), 2,
            "TWO cfg branches of one same-named function report as TWO entries — pinned by \
             `a_qualified_name_carried_by_two_cfg_gated_units_yields_one_violation_not_two`, which asserts \
             the report must not lose either declaration:\n{v:#}");
        for m in &matches {
            assert_eq!(m["invisible"], serde_json::json!(["core_foundation"]),
                "STATED RESIDUAL: both branches share the disclosure (the bare-qual-keyed union), \
                 including the stub that provably makes no such call — safe-direction over-report, \
                 never a hidden effect:\n{v:#}");
        }
    }

    #[test]
    fn type_surface_publishes_a_generic_instantiation_but_still_not_a_wrapper() {
        let dep = scan_crate_chained("gen", "deplib", "", "\
pub struct Held<T> { pub t: T }
impl<T> Held<T> { pub fn touch(&self) { let _ = std::fs::read(\"/etc/x\"); } }
pub struct Marker;
pub struct Plain;
impl Plain { pub fn touch(&self) { let _ = std::fs::read(\"/etc/x\"); } }
pub fn make() -> Held<Marker> { Held { t: Marker } }
pub fn wrapped() -> Result<Held<Marker>, String> { Ok(Held { t: Marker }) }
#[cfg(not(target_arch = \"wasm32\"))]
pub fn dup() -> Plain { Plain }
#[cfg(target_arch = \"wasm32\")]
pub fn dup() -> Plain { Plain }
", &DepIndex::default());
        let ts = &dep["typeSurface"]["returns"];
        assert_eq!(ts["deplib#make"].as_str(), Some("deplib#Held"),
                   "a generic INSTANTIATION must publish its outer type — the binding holds a `Held`:\n{dep}");
        assert!(ts.get("deplib#wrapped").is_none(),
                "a `-> Result<Held<_>,E>` must still refuse: the binding holds a Result, and keying it to \
                 the payload is the reverted attempt's defect:\n{dep}");
        // THE `#[cfg]` DUPLICATION IS A NON-EVENT, and asserting it is what falsifies the queue's
        // "spurious collision" diagnosis rather than leaving it to a commit message.
        assert_eq!(ts["deplib#dup"].as_str(), Some("deplib#Plain"),
                   "two `#[cfg]`-exclusive defs naming the SAME return type must publish it — there is \
                    nothing to guess between:\n{dep}");

        let (idx, dir) = chain("gen", "deplib", &dep);
        let app = scan_crate_chained("gen", "app", "\n[dependencies]\ndeplib = \"1\"\n", "\
pub fn typed() { let h = deplib::make(); h.touch(); }
pub fn via_result() { let r = deplib::wrapped(); let _ = r.is_ok(); }
", &idx);
        let _ = std::fs::remove_dir_all(&dir);
        assert!(effects_of(&app, "typed").contains(&"Fs".to_string()),
                "the consumer must inherit the generic's method effect — this is the whole point, and \
                 before the fix it read `Unknown` instead:\n{app}");
        assert!(!effects_of(&app, "via_result").contains(&"Fs".to_string()),
                "a Result binding was keyed through its payload's methods:\n{app}");
    }

    /// NOTE FOR THE TEST BELOW: it still passes, but the LINE that protects it moved. `Result`/`Option`
    /// returns are no longer refused by `bound_return_type`'s generic guard (that guard is gone); they
    /// resolve to `deplib#Result` / `deplib#Option` and are then dropped by `build_type_surface`'s
    /// `nonpure` gate, because the crate owns no type by those names and so carries no methods under
    /// them. The outcome is identical and the mechanism is not — if the `nonpure` gate is ever relaxed,
    /// THIS is the test that starts failing, and its assertions are about the wrapper rather than about
    /// that gate.
    #[test]
    fn type_surface_refuses_to_key_through_a_wrapper_and_never_falls_silent_on_a_miss() {
        // ⟨typeSurface.returns⟩ requirements (b) and (c), both confirmed defects of the reverted attempt.
        //
        // (b) `record_return` UNWRAPS `Result`/`Option`, so `fn connect() -> Result<Conn,E>` recorded
        //     `Conn`. Published across the boundary that is a lie about what the BINDING holds: a
        //     `let c = dep::connect();` holds a `Result`, and keying `c.is_ok()` against `Conn` charged
        //     `Conn`'s effects to a caller that never runs them. Nothing wrapped may be published.
        // (c) A `by_key` MISS after a `returns` HIT must fall back to half 1's disclosure. `by_key`
        //     DROPS ambiguous keys, so a miss cannot distinguish "no such method" from "I withdrew an
        //     entry"; the attempt read that refusal as a purity claim and `continue`d in silence.
        let dep = scan_crate_chained("wrap", "deplib", "", "\
pub struct Conn;
impl Conn {
    pub fn fetch(&self) -> String { std::fs::read_to_string(\"/etc/x\").unwrap_or_default() }
    pub fn pure_ping(&self) -> u8 { 1 }
}
pub fn build() -> Conn { Conn }
pub fn connect() -> Result<Conn, String> { Ok(Conn) }
pub fn maybe() -> Option<Conn> { Some(Conn) }
", &DepIndex::default());
        let ts = &dep["typeSurface"]["returns"];
        assert_eq!(ts["deplib#build"].as_str(), Some("deplib#Conn"), "the unwrapped factory must publish:\n{dep}");
        assert!(ts.get("deplib#connect").is_none(),
                "a `-> Result<Conn,E>` published its PAYLOAD as if it were the bound type:\n{dep}");
        assert!(ts.get("deplib#maybe").is_none(),
                "a `-> Option<Conn>` published its PAYLOAD as if it were the bound type:\n{dep}");
        let (idx, dir) = chain("wrap", "deplib", &dep);
        let app = scan_crate_chained("wrap", "app", "\n[dependencies]\ndeplib = \"1\"\n", "\
pub fn direct() -> String { let c = deplib::build(); c.fetch() }
pub fn via_result() -> bool { let c = deplib::connect(); c.is_ok() }
pub fn via_option() -> bool { let c = deplib::maybe(); c.is_some() }
pub fn unknown_method() -> u8 { let c = deplib::build(); c.pure_ping() }
", &idx);
        let _ = std::fs::remove_dir_all(&dir);
        assert!(effects_of(&app, "direct").contains(&"Fs".to_string()),
                "the case that must STILL resolve stopped resolving:\n{app}");
        for wrapped in ["via_result", "via_option"] {
            let eff = effects_of(&app, wrapped);
            assert!(!eff.contains(&"Fs".to_string()),
                    "{wrapped}: keyed a Result/Option binding through the PAYLOAD's methods:\n{app}");
            assert!(eff.contains(&"Unknown".to_string()), "{wrapped}: fell silent instead:\n{app}");
        }
        // (c): `Conn` IS known and `pure_ping` is absent from the dep's report. Absence under an exact
        // key still cannot distinguish "no such method" from "the index withdrew an ambiguous entry",
        // so this discloses.
        assert!(effects_of(&app, "unknown_method").contains(&"Unknown".to_string()),
                "a by_key miss after a returns HIT read as a purity claim — defect 3:\n{app}");
    }

    #[test]
    fn dep_join_does_not_fabricate_onto_a_local_shadow() {
        // The CANDOR_DEPS cross-crate join must NOT override a LOCAL definition: a project module/fn named
        // like a covered dep crate, resolving to the project's OWN pure code, must not inherit the dep
        // report's effects (a fabrication the join lacked the `resolved_local` guard for). A
        // GENUINE external call into the covered crate must still inherit.
        let dep = std::env::temp_dir().join(format!("candor-depjoin-rep-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dep);
        let me = format!("scan-{}", env!("CARGO_PKG_VERSION"));
        std::fs::write(dep.join("report.depb.scan.json"), format!(r#"{{
            "candor": {{"version": "{me}", "toolchain": "stable", "spec": "0.7"}},
            "functions": [{{"fn": "effectful_fn", "inferred": ["Net"], "hash": "depb#effectful_fn"}}]}}"#)).unwrap();
        let idx = load_dep_reports(Some(dep.to_str().unwrap()));
        assert!(idx.crates.contains("depb"));
        let run = |name: &str, src: &str| -> serde_json::Value {
            let d = std::env::temp_dir().join(format!("candor-depjoin-{name}-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&d);
            std::fs::create_dir_all(d.join("src")).unwrap();
            std::fs::write(d.join("Cargo.toml"), format!("[package]\nname = \"{name}\"\n")).unwrap();
            std::fs::write(d.join("src/lib.rs"), src).unwrap();
            let prefix = d.join("out/r").to_string_lossy().into_owned();
            let (rc, body) = scan_one(&d.to_string_lossy(), ScanOpts {
                prefix, want_json: true, include_tests: false, policy: None, baseline: None, ws_member: false, quiet: true, deps_idx: &idx, peek_excluded: false,
            }, &crate::gate::begin_run());
            assert_eq!(rc, 0);
            let v = serde_json::from_str(&body.unwrap()).unwrap();
            let _ = std::fs::remove_dir_all(&d);
            v
        };
        let eff = |v: &serde_json::Value, needle: &str| -> Vec<String> {
            v["functions"].as_array().into_iter().flatten()
                .filter(|f| f["fn"].as_str().is_some_and(|q| q.contains(needle)))
                .flat_map(|f| f["inferred"].as_array().into_iter().flatten()
                    .filter_map(|e| e.as_str().map(String::from)).collect::<Vec<_>>()).collect()
        };
        // LOCAL module named like the dep crate → its caller must stay PURE (no Net from the join)
        let shadow = run("projsh", "mod depb { pub fn effectful_fn() -> i32 { 42 } }\npub fn uses_local() { let _ = depb::effectful_fn(); }");
        assert!(eff(&shadow, "uses_local").is_empty(),
                "dep-join FABRICATED the dep's effect onto a local shadow:\n{shadow}");
        // GENUINE external call into the covered crate → must STILL inherit Net
        let genuine = run("projgen", "pub fn calls_dep() { depb::effectful_fn(); }");
        assert!(eff(&genuine, "calls_dep").contains(&"Net".to_string()),
                "a genuine dep call must still inherit the dep report's Net:\n{genuine}");
        let _ = std::fs::remove_dir_all(&dep);
    }

    /// COULD-NOT-FORM-A-KEY vs KEYED-AND-MISSED (candor-spec/DEP-RECEIVER-TYPING-DESIGN.md, half 1).
    ///
    /// `let c = deplib::build(); c.fetch()` — a pure factory is OMITTED from the dep's report, so no
    /// return type travels, so `c` is never typed and NO KEY IS EVER FORMED. The report's silence is only
    /// an answer to a question that was asked; here none was, and dropping the call made `go` a confident
    /// purity claim about a function that performs Fs. It must disclose instead.
    ///
    /// The two controls are the point: this must NOT fire when a key WAS formed (a genuine purity claim,
    /// §2 rule 3), and must NOT fire for an UNCHAINED crate (the κ ledger already discloses `invisible`
    /// there, so a second disclosure is pure false uncertainty).
    #[test]
    fn an_untyped_receiver_from_a_chained_crate_discloses_instead_of_reading_pure() {
        let dep = std::env::temp_dir().join(format!("candor-untyped-rep-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dep);
        let _ = std::fs::create_dir_all(&dep);
        let me = format!("scan-{}", env!("CARGO_PKG_VERSION"));
        // The dep's report holds `Client::fetch` — the ANSWER is present. `build` (pure) is absent, which
        // is exactly why the consumer cannot type `c`.
        std::fs::write(dep.join("report.deplib.scan.json"), format!(r#"{{
            "candor": {{"version": "{me}", "toolchain": "stable", "spec": "0.23"}},
            "package": "deplib",
            "functions": [{{"fn": "Client::fetch", "inferred": ["Fs"], "hash": "deplib#Client::fetch"}}]}}"#)).unwrap();
        let idx = load_dep_reports(Some(dep.to_str().unwrap()));
        assert!(idx.crates.contains("deplib"));
        let run = |name: &str, deps_idx: &DepIndex, src: &str| -> serde_json::Value {
            let d = std::env::temp_dir().join(format!("candor-untyped-{name}-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&d); // never read a stale report back as this arm's result
            std::fs::create_dir_all(d.join("src")).unwrap();
            std::fs::write(d.join("Cargo.toml"),
                format!("[package]\nname = \"{name}\"\n[dependencies]\ndeplib = \"1\"\n")).unwrap();
            std::fs::write(d.join("src/lib.rs"), src).unwrap();
            let prefix = d.join("out/r").to_string_lossy().into_owned();
            let (rc, body) = scan_one(&d.to_string_lossy(), ScanOpts {
                prefix, want_json: true, include_tests: false, policy: None, baseline: None,
                ws_member: false, quiet: true, deps_idx, peek_excluded: false,
            }, &crate::gate::begin_run());
            assert_eq!(rc, 0);
            let v: serde_json::Value = serde_json::from_str(&body.unwrap()).unwrap();
            let _ = std::fs::remove_dir_all(&d);
            v
        };
        const SRC: &str = "pub fn go() -> String { let c = deplib::build(); c.fetch() }";

        // 1. CHAINED + untyped receiver -> must disclose, with the reason naming the cause.
        let v = run("upa", &idx, SRC);
        let go = fn_entry(&v, "go");
        assert!(effs(go).contains(&"Unknown".to_string()),
                "an untyped receiver from a CHAINED crate must not read pure — no key was ever formed, so \
                 the dep report's silence licenses nothing:\n{v:#}");
        assert!(go["unknownWhy"].as_array().into_iter().flatten()
                    .any(|r| r.as_str() == Some("dispatch:untyped cross-package receiver")),
                "the disclosure must name WHY (spec §2 unknownWhy, `dispatch:` class):\n{v:#}");

        // 2. CONTROL — a key WAS formed. `deplib::build()` called directly is a real lookup: the dep's
        //    report omits it because it is pure, and THAT absence is a genuine purity claim. No Unknown.
        //    (`go2` is ABSENT from the report — which for a keyed lookup IS the purity claim, so the
        //    check is that it is absent-or-Unknown-free, not that it is present.)
        let keyed = run("upb", &idx, "pub fn go2() { deplib::build(); }");
        let go2 = keyed["functions"].as_array().into_iter().flatten()
            .find(|f| f["fn"].as_str() == Some("go2")).cloned();
        assert!(go2.as_ref().is_none_or(|f| !effs(f).contains(&"Unknown".to_string())),
                "keyed-and-missed is a genuine purity claim and must stay silent:\n{keyed:#}");

        // 3. CONTROL — the SAME source with NOTHING chained must be unchanged. The κ ledger already
        //    discloses `invisible: [deplib]` there; a second disclosure would be false uncertainty.
        //    (Measured: without this conjunct the rung fired on `let v = dep::f(); v.first()`, a std Vec
        //    method on a dep-returned value.)
        let unchained = run("upc", &DepIndex::default(), SRC);
        assert!(!effs(fn_entry(&unchained, "go")).contains(&"Unknown".to_string()),
                "an UNCHAINED crate is already disclosed by the κ ledger — do not double-disclose:\n{unchained:#}");
        assert_eq!(fn_entry(&unchained, "go")["invisible"], serde_json::json!(["deplib"]),
                "…and that ledger disclosure is what covers it:\n{unchained:#}");

        // 4. ⟨0.21⟩ THE TRADE candor-ts MEASURED GOING THE WRONG WAY, checked here as an assertion
        //    rather than left as an argument. Withholding coverage from a self-declared-INCOMPLETE
        //    report sends an unanswerable key to the κ-ledger arm — and in an engine whose half-1
        //    disclosure is gated on COVERAGE, that arm REPLACES this `Unknown[dispatch]` with the
        //    `invisible` hedge and `deny Fs Unknown[dispatch]` silently goes exit 1 -> exit 0: a gate
        //    lost to a fix whose whole argument is that it only adds disclosure (candor-ts `21277eb`,
        //    which hit it for real; java `d1d3045` and swift `74cd8f1` each had to name the reason it
        //    could not happen there). rust's third conjunct reads `deps_idx.crates` — the CHAINED set,
        //    which an incomplete report is still in — so both voices speak. Mutate that conjunct to
        //    `crates && !incomplete_pkgs` and this row is the one that fails.
        let inc = std::env::temp_dir().join(format!("candor-untyped-inc-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&inc);
        let _ = std::fs::create_dir_all(&inc);
        std::fs::write(inc.join("report.deplib.scan.json"), format!(r#"{{
            "candor": {{"version": "{me}", "toolchain": "stable", "spec": "0.23"}},
            "package": "deplib",
            "unanalyzed": [{{"path": "src/broken.rs", "reason": "parse error"}}],
            "functions": [{{"fn": "Client::fetch", "inferred": ["Fs"], "hash": "deplib#Client::fetch"}}]}}"#)).unwrap();
        let idx_inc = load_dep_reports(Some(inc.to_str().unwrap()));
        let _ = std::fs::remove_dir_all(&inc);
        assert!(idx_inc.incomplete_pkgs.contains("deplib"), "the arm's premise: coverage IS withheld");
        let v_inc = run("upd", &idx_inc, SRC);
        let go_inc = fn_entry(&v_inc, "go");
        assert!(effs(go_inc).contains(&"Unknown".to_string()),
                "half 1 went SILENT over an incomplete report — the coverage hedge REPLACED the \
                 unanswerable-key disclosure instead of joining it:\n{v_inc:#}");
        assert!(go_inc["unknownWhy"].as_array().into_iter().flatten()
                    .any(|r| r.as_str() == Some("dispatch:untyped cross-package receiver")),
                "…and it must keep its REASON CLASS, or `deny Unknown[dispatch]` stops biting:\n{v_inc:#}");
        let _ = std::fs::remove_dir_all(&dep);
    }

    /// One signature may bind the same trait LEAF to two different crates. `trait_quals` is keyed by leaf,
    /// and last-wins made `a.go()` on an `alpha::Handler` form `beta::Handler::go` and inherit BETA's
    /// reported effects — a fabrication on a function that never touches beta.
    #[test]
    fn a_trait_leaf_bound_to_two_crates_must_not_pick_one() {
        let dep = std::env::temp_dir().join(format!("candor-quals-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dep);
        let _ = std::fs::create_dir_all(&dep);
        let me = format!("scan-{}", env!("CARGO_PKG_VERSION"));
        std::fs::write(dep.join("report.alpha.scan.json"), format!(r#"{{
            "candor": {{"version": "{me}", "toolchain": "stable", "spec": "0.23"}},
            "package": "alpha", "functions": []}}"#)).unwrap();
        std::fs::write(dep.join("report.beta.scan.json"), format!(r#"{{
            "candor": {{"version": "{me}", "toolchain": "stable", "spec": "0.23"}},
            "package": "beta",
            "functions": [{{"fn": "Handler::go", "inferred": ["Net"], "hash": "beta#Handler::go"}}]}}"#)).unwrap();
        let idx = load_dep_reports(Some(dep.to_str().unwrap()));
        let d = std::env::temp_dir().join(format!("candor-qualsapp-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(d.join("src")).unwrap();
        std::fs::write(d.join("Cargo.toml"),
            "[package]\nname = \"app\"\n[dependencies]\nalpha = \"1\"\nbeta = \"1\"\n").unwrap();
        // BOTH receivers are called. `a` is alpha's (pure), `b` is beta's (Net). The precise answer is
        // exactly ['Net'] — and BOTH failure modes are wrong: last-wins fabricated beta's Net onto `a`,
        // while tombstoning the collision lost `b`'s genuine reach, which is the cardinal sin.
        // Every spelling of "one leaf, two crates", and for each the no-fabrication control beside it.
        // The GENERIC and WHERE forms are here because the first per-receiver fix handled only the `dyn`
        // spelling and silently reopened the miss for the other two — bounds were collected into ONE
        // leaf-keyed map, which re-created the collision the fix existed to remove.
        std::fs::write(d.join("src/lib.rs"),
            "pub fn handle(a: &dyn alpha::Handler, b: &dyn beta::Handler) { a.go(); b.go(); }\n\
             pub fn only_alpha(a: &dyn alpha::Handler, _b: &dyn beta::Handler) { a.go(); }\n\
             pub fn shadowed(a: &dyn alpha::Handler, b: &dyn beta::Handler) {\n\
                 let a: &dyn beta::Handler = b; a.go(); }\n\
             pub fn generic_form<A: alpha::Handler, B: beta::Handler>(a: A, b: B) { a.go(); b.go(); }\n\
             pub fn generic_only_alpha<A: alpha::Handler, B: beta::Handler>(a: A, _b: B) { a.go(); }\n\
             pub fn where_form<A, B>(a: A, b: B) where A: alpha::Handler, B: beta::Handler { a.go(); b.go(); }\n\
             pub fn block_shadow(a: &dyn beta::Handler, x: &dyn alpha::Handler) {\n\
                 { let a: &dyn alpha::Handler = x; a.go(); } a.go(); }\n\
             pub fn outer(h: &dyn beta::Handler) {\n\
                 fn inner(h: &dyn alpha::Handler) { h.go(); } let _ = inner; h.go(); }\n").unwrap();
        let (rc, body) = scan_one(&d.to_string_lossy(), ScanOpts {
            prefix: d.join("out/r").to_string_lossy().into_owned(), want_json: true,
            include_tests: false, policy: None, baseline: None, ws_member: false, quiet: true, deps_idx: &idx, peek_excluded: false,
        }, &crate::gate::begin_run());
        assert_eq!(rc, 0);
        let v: serde_json::Value = serde_json::from_str(&body.unwrap()).unwrap();
        let find = |n: &str| v["functions"].as_array().into_iter().flatten()
            .find(|f| f["fn"].as_str() == Some(n)).cloned();
        // NO FABRICATION: a call on ALPHA's Handler alone must not inherit BETA's effects.
        let only_alpha = find("only_alpha");
        assert!(only_alpha.as_ref().is_none_or(|f| !effs(f).contains(&"Net".to_string())),
                "a call on ALPHA's Handler must not inherit BETA's effects because both leaves are \
                 spelled `Handler`:\n{v:#}");
        // NO MISS, the other direction: `b.go()` IS a genuine reach into beta and must survive. Dropping
        // the colliding leaf outright is safe against fabrication and silently loses this — worse.
        // A trait-typed LOCAL carries its OWN crate, shadowing the parameter of the same name. Only
        // signatures were ever recorded, so this local inherited the param's crate and lost its reach —
        // masked for years by the last-wins map happening to supply the right answer.
        assert!(find("shadowed").as_ref().is_some_and(|f| effs(f).contains(&"Net".to_string())),
                "a trait-typed LOCAL must use its own crate qualification, not the shadowed \
                 parameter's:\n{v:#}");
        // The GENERIC and WHERE spellings must reach beta exactly as the `dyn` one does…
        for n in ["generic_form", "where_form"] {
            assert!(find(n).as_ref().is_some_and(|f| effs(f).contains(&"Net".to_string())),
                    "{n}: a generic/where bound must resolve PER TYPE PARAM — collecting every bound into \
                     one leaf-keyed map re-creates the collision:\n{v:#}");
        }
        // …and must not fabricate when only alpha is called.
        assert!(find("generic_only_alpha").as_ref().is_none_or(|f| !effs(f).contains(&"Net".to_string())),
                "generic_only_alpha must not inherit BETA's effects:\n{v:#}");
        // A BLOCK-scoped shadow must not permanently rebind the parameter's crate for the rest of the fn.
        assert!(find("block_shadow").as_ref().is_some_and(|f| effs(f).contains(&"Net".to_string())),
                "a block-scoped shadow must be undone at scope exit:\n{v:#}");
        // A NESTED item must not inherit the outer signature's crate for a same-named receiver.
        assert!(find("outer").as_ref().is_some_and(|f| effs(f).contains(&"Net".to_string())),
                "outer's own receiver is BETA's; the nested fn must not rebind it:\n{v:#}");
        assert!(find("inner").as_ref().is_none_or(|f| !effs(f).contains(&"Net".to_string())),
                "inner's receiver is ALPHA's and must stay pure:\n{v:#}");
        assert!(find("handle").as_ref().is_some_and(|f| effs(f).contains(&"Net".to_string())),
                "the genuine call on BETA's Handler must still resolve — a leaf collision must be \
                 disambiguated per RECEIVER, not dropped for both:\n{v:#}");
        let _ = std::fs::remove_dir_all(&d);
        let _ = std::fs::remove_dir_all(&dep);
    }

    #[test]
    fn smart_pointer_receiver_resolves_pointee_method_but_not_clone() {
        // A method call on an `Arc<T>`/`Rc<T>`/`Box<T>` receiver auto-derefs to T's method, so it must
        // resolve the POINTEE's effect — not silently drop (the corpus-found §4 under-report: duct's
        // whole public API read pure because `self.0: Arc<ExpressionInner>`). BUT `.clone()` must NOT
        // resolve to `T::clone`: `arc.clone()` calls the pure `Arc::clone` (refcount), never the
        // pointee's clone, so resolving it would FABRICATE an effectful in-crate clone.
        let run = |name: &str, src: &str| -> serde_json::Value {
            let d = std::env::temp_dir().join(format!("candor-smartptr-{name}-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&d);
            std::fs::create_dir_all(d.join("src")).unwrap();
            std::fs::write(d.join("Cargo.toml"), format!("[package]\nname = \"{name}\"\n")).unwrap();
            std::fs::write(d.join("src/lib.rs"), src).unwrap();
            let prefix = d.join("out/r").to_string_lossy().into_owned();
            let idx = load_dep_reports(None);
            let (rc, body) = scan_one(&d.to_string_lossy(), ScanOpts {
                prefix, want_json: true, include_tests: false, policy: None, baseline: None, ws_member: false, quiet: true, deps_idx: &idx, peek_excluded: false,
            }, &crate::gate::begin_run());
            assert_eq!(rc, 0);
            let v: serde_json::Value = serde_json::from_str(&body.unwrap()).unwrap();
            let _ = std::fs::remove_dir_all(&d);
            v
        };
        let eff = |v: &serde_json::Value, needle: &str| -> Vec<String> {
            v["functions"].as_array().into_iter().flatten()
                .filter(|f| f["fn"].as_str().is_some_and(|q| q.contains(needle)))
                .flat_map(|f| f["inferred"].as_array().into_iter().flatten()
                    .filter_map(|e| e.as_str().map(String::from)).collect::<Vec<_>>()).collect()
        };
        let src = r#"
use std::sync::Arc; use std::rc::Rc;
struct Inner;
impl Inner {
    fn doit(&self) { std::process::Command::new("ls").status().unwrap(); }
    fn clone(&self) -> Inner { std::fs::read("/x").unwrap(); Inner }
}
struct A(Arc<Inner>);
impl A { pub fn run(&self) { self.0.doit(); } pub fn dup(&self) -> Arc<Inner> { self.0.clone() } }
struct B(Box<Inner>); impl B { pub fn run(&self) { self.0.doit(); } }
struct R(Rc<Inner>); impl R { pub fn run(&self) { self.0.doit(); } }
"#;
        let v = run("smartptr", src);
        // auto-deref: the pointee's Exec is reached through Arc/Box/Rc receivers (was silently pure)
        assert!(eff(&v, "A::run").contains(&"Exec".to_string()), "Arc deref lost Exec:\n{v}");
        assert!(eff(&v, "B::run").contains(&"Exec".to_string()), "Box deref lost Exec:\n{v}");
        assert!(eff(&v, "R::run").contains(&"Exec".to_string()), "Rc deref lost Exec:\n{v}");
        // anti-fabrication: `arc.clone()` is the pure `Arc::clone`, never the effectful pointee clone
        assert!(eff(&v, "A::dup").is_empty(), "arc.clone() FABRICATED the pointee's clone effect:\n{v}");
    }

    #[test]
    fn provided_io_methods_reach_local_impl_required_method() {
        // A std-PROVIDED `io::Write`/`io::Read` method (`write_all`/`read_to_end`/…) drives the trait's
        // REQUIRED method (`write`/`read`) INSIDE std — invisible to the scan. A call to one on a CONCRETE
        // LOCAL `impl Write`/`impl Read` whose `write`/`read` is effectful read silent-pure (the
        // provided→required callback the write! MACRO edge already recovered, but the direct METHOD-CALL
        // form did not). Recover it — while a PURE local impl and a std receiver (`Vec`/`String`) stay pure.
        let run = |name: &str, src: &str| -> serde_json::Value {
            let d = std::env::temp_dir().join(format!("candor-iowr-{name}-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&d);
            std::fs::create_dir_all(d.join("src")).unwrap();
            std::fs::write(d.join("Cargo.toml"), format!("[package]\nname = \"{name}\"\n")).unwrap();
            std::fs::write(d.join("src/lib.rs"), src).unwrap();
            let prefix = d.join("out/r").to_string_lossy().into_owned();
            let idx = load_dep_reports(None);
            let (rc, body) = scan_one(&d.to_string_lossy(), ScanOpts {
                prefix, want_json: true, include_tests: false, policy: None, baseline: None, ws_member: false, quiet: true, deps_idx: &idx, peek_excluded: false,
            }, &crate::gate::begin_run());
            assert_eq!(rc, 0);
            let v: serde_json::Value = serde_json::from_str(&body.unwrap()).unwrap();
            let _ = std::fs::remove_dir_all(&d);
            v
        };
        let eff = |v: &serde_json::Value, needle: &str| -> Vec<String> {
            v["functions"].as_array().into_iter().flatten()
                .filter(|f| f["fn"].as_str().is_some_and(|q| q.contains(needle)))
                .flat_map(|f| f["inferred"].as_array().into_iter().flatten()
                    .filter_map(|e| e.as_str().map(String::from)).collect::<Vec<_>>()).collect()
        };
        let src = r#"
use std::io::{Read, Write};
// effectful local io::Write, PURE ctor
struct FileSink;
impl Write for FileSink {
    fn write(&mut self, b: &[u8]) -> std::io::Result<usize> { std::fs::write("/x", b)?; Ok(b.len()) }
    fn flush(&mut self) -> std::io::Result<()> { Ok(()) }
}
// effectful local io::Read
struct NetReader;
impl Read for NetReader {
    fn read(&mut self, b: &mut [u8]) -> std::io::Result<usize> { std::process::Command::new("ls").status()?; Ok(b.len()) }
}
// PURE local Write impl — the over-fire control
struct NullSink;
impl Write for NullSink {
    fn write(&mut self, b: &[u8]) -> std::io::Result<usize> { Ok(b.len()) }
    fn flush(&mut self) -> std::io::Result<()> { Ok(()) }
}
pub fn drive_write() { let mut s = FileSink; let _ = s.write_all(b"x"); }   // Fs, via write_all -> FileSink::write
pub fn drive_read() { let mut r = NetReader; let mut buf = Vec::new(); let _ = r.read_to_end(&mut buf); } // Exec, via read_to_end -> NetReader::read
pub fn pure_drive() { let mut s = NullSink; let _ = s.write_all(b"x"); }    // PURE (impl is pure)
pub fn std_recv() { let mut v: Vec<u8> = Vec::new(); let _ = v.write_all(b"x"); } // PURE (std Vec, not local)
"#;
        let v = run("iowr", src);
        assert!(eff(&v, "drive_write").contains(&"Fs".to_string()),
                "write_all on a concrete local impl Write lost the Fs (provided->required callback):\n{v}");
        assert!(eff(&v, "drive_read").contains(&"Exec".to_string()),
                "read_to_end on a concrete local impl Read lost the effect:\n{v}");
        assert!(eff(&v, "pure_drive").is_empty(),
                "write_all on a PURE local impl was over-reported:\n{v}");
        assert!(eff(&v, "std_recv").is_empty(),
                "write_all on a std Vec receiver fabricated an effect (no local impl):\n{v}");
    }

    #[test]
    fn blanket_impl_method_resolves_to_the_blanket_body_but_inherent_wins() {
        // §4 honesty (R45): a `x.ext()` where `ext` comes from a BLANKET impl (`impl<T> Ext for T { fn ext }`
        // — or bounded `impl<T: Bound> Ext for T`) read silent-pure: the blanket body's qual is `T::ext` (the
        // generic self param), so a keyed lookup on `x`'s concrete type missed it. Resolve an unresolved
        // TYPED call to the blanket body. CONTROLS: a PURE blanket adds nothing; an INHERENT `ext` on the
        // receiver's type WINS (resolves first → the blanket never overrides it, no fabrication/double-charge).
        let d = std::env::temp_dir().join(format!("candor-blanket-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(d.join("src")).unwrap();
        std::fs::write(d.join("Cargo.toml"), "[package]\nname = \"blanket\"\n").unwrap();
        std::fs::write(
            d.join("src/lib.rs"),
            r#"
            use std::fs;
            pub trait Ext { fn ext(&self); }
            impl<T> Ext for T { fn ext(&self) { let _ = fs::write("/e", "x"); } }   // Fs blanket
            pub trait Bound {}
            pub trait Ext2 { fn ext2(&self); }
            impl<T: Bound> Ext2 for T { fn ext2(&self) { let _ = fs::write("/e2", "x"); } } // Fs bounded blanket
            pub trait PureE { fn pe(&self); }
            impl<T> PureE for T { fn pe(&self) {} }                                 // pure blanket
            pub struct A;
            pub struct B; impl Bound for B {}
            pub fn calls_blanket() { let a = A; a.ext(); }                          // Fs
            pub fn calls_bounded() { let b = B; b.ext2(); }                         // Fs
            pub fn calls_pure() { let a = A; a.pe(); }                              // pure
            pub struct C;
            impl C { pub fn ext(&self) { let _ = std::net::TcpStream::connect("h:1"); } }  // inherent → Net
            pub fn calls_inherent() { let c = C; c.ext(); }                         // Net (inherent wins, NOT Fs)
            "#,
        )
        .unwrap();
        let idx = load_dep_reports(None);
        let prefix = d.join("out/r").to_string_lossy().into_owned();
        let (rc, body) = scan_one(&d.to_string_lossy(), ScanOpts {
            prefix, want_json: true, include_tests: false, policy: None, baseline: None, ws_member: false, quiet: true, deps_idx: &idx, peek_excluded: false,
        }, &crate::gate::begin_run());
        assert_eq!(rc, 0);
        let body = body.expect("want_json returns the report body");
        let v: serde_json::Value = serde_json::from_str(&body).unwrap();
        let effs = |needle: &str| -> Vec<String> {
            v["functions"].as_array().into_iter().flatten()
                .filter(|f| f["fn"].as_str() == Some(needle))
                .flat_map(|f| f["inferred"].as_array().into_iter().flatten().filter_map(|e| e.as_str().map(String::from)))
                .collect()
        };
        assert!(effs("calls_blanket").contains(&"Fs".to_string()), "a blanket method's effect must reach the caller:\n{body}");
        assert!(effs("calls_bounded").contains(&"Fs".to_string()), "a bounded-blanket method's effect must reach the caller:\n{body}");
        assert!(effs("calls_pure").is_empty(), "a pure blanket must add no effect:\n{body}");
        // inherent WINS: Net (its own), never Fs (the blanket) — no fabrication/double-charge
        assert!(effs("calls_inherent").contains(&"Net".to_string()) && !effs("calls_inherent").contains(&"Fs".to_string()),
                "an inherent method must win over the blanket (Net, not Fs):\n{body}");
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn imported_dependency_trait_dispatches_over_local_impls_r4() {
        // R4, the scan-boundary vein (candor-spec SOUNDNESS-VEIN-crossing-the-scan-boundary.md):
        // `use deplib::Handler; fn run(h: &dyn Handler) { h.go() }` where the impl that RUNS is declared
        // HERE. `local_traits` is built only from local `ItemTrait` nodes, so CHA never fired and `run`
        // read SILENT-PURE — while the single-crate control (the trait in a local `mod`) resolves it, and
        // a `deny Fs run` gate went exit 1 -> exit 0 on the split. Needs no dep report: the impl is ours.
        //
        // The four CONTROLS below are the carve-outs, and each one is a measured fabrication/flood that a
        // looser version of this rung actually produced. They matter more than the positive case.
        let d = std::env::temp_dir().join(format!("candor-r4-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(d.join("src")).unwrap();
        std::fs::write(d.join("Cargo.toml"), "[package]\nname = \"r4\"\n").unwrap();
        std::fs::write(
            d.join("src/lib.rs"),
            r#"
            use deplib::Handler;

            pub struct MyH;
            impl Handler for MyH { fn go(&self) { let _ = std::fs::read_to_string("/etc/hosts"); } }

            // POSITIVE: an IMPORTED dependency trait, `dyn` receiver, local impl -> resolves.
            pub fn run_imported(h: &dyn Handler) { h.go(); }
            // POSITIVE: the same through a Box, and through a collection element.
            pub fn run_boxed(h: Box<dyn Handler>) { h.go(); }

            // CONTROL 1 — PROVENANCE/std: a std trait with a local impl must NOT resolve. This is the
            // narrow form of the `impl Iterator for RowIter` fabrication the older test pins: `w.flush()`
            // on ANY `&dyn Write` must not be charged with LoudWriter's Net.
            pub mod stdw {
                use std::io::Write;
                pub struct LoudWriter;
                impl Write for LoudWriter {
                    fn write(&mut self, b: &[u8]) -> std::io::Result<usize> { let _ = std::net::TcpStream::connect("h:1"); Ok(b.len()) }
                    fn flush(&mut self) -> std::io::Result<()> { let _ = std::net::TcpStream::connect("h:1"); Ok(()) }
                }
                pub fn writes(w: &mut dyn Write) { let _ = w.flush(); }
            }

            // CONTROL 2 — ERASURE: a DEPENDENCY trait reached through a caller-monomorphized GENERIC
            // BOUND / `impl Trait` must NOT resolve. Unguarded, `serde::Serialize` put 32 fresh Unknowns
            // on serde_json this way; here the same shape would charge `MyH`'s Fs onto a pure generic.
            pub fn run_bound<T: Handler>(t: T) { t.go(); }
            pub fn run_impl(h: impl Handler) { h.go(); }

            // CONTROL 3 — CRATE-LOCAL re-export: a `use self::…` binding makes a std trait look
            // dependency-qualified (`self::inner::Write`). It is ours, not a dependency, and treating it
            // as one cost 17 fresh Unknowns on value-bag, whose `internal/error.rs` does exactly this.
            pub mod reexp {
                pub mod inner { pub use std::io::Write; }
                use self::inner::Write;
                pub struct LoudW2;
                impl Write for LoudW2 {
                    fn write(&mut self, b: &[u8]) -> std::io::Result<usize> { let _ = std::net::TcpStream::connect("h:2"); Ok(b.len()) }
                    fn flush(&mut self) -> std::io::Result<()> { let _ = std::net::TcpStream::connect("h:2"); Ok(()) }
                }
                pub fn writes_reexported(w: &mut dyn Write) { let _ = w.flush(); }
            }

            // CONTROL 4 — NESTED ITEM: a `fn`/`impl` declared inside a body has its OWN signature, and its
            // params SHADOW the outer ones under the same name, so the enclosing fn's `dyn`-ness must not
            // leak in. This is the value-bag `internal_visit(v: &dyn Serialize)` shape verbatim: an inner
            // `serialize_some<T: Serialize>(self, v: &T)` whose generic `v` inherited the outer `v`'s
            // erasure and CHA'd through it.
            pub fn nested(h: &dyn Handler) -> u8 {
                struct Inner;
                impl Inner { fn run<T: Handler>(&self, h: T) { h.go(); } }
                let _ = h;
                0
            }
            "#,
        )
        .unwrap();
        let idx = load_dep_reports(None);
        let prefix = d.join("out/r").to_string_lossy().into_owned();
        let (rc, body) = scan_one(&d.to_string_lossy(), ScanOpts {
            prefix, want_json: true, include_tests: false, policy: None, baseline: None, ws_member: false, quiet: true, deps_idx: &idx, peek_excluded: false,
        }, &crate::gate::begin_run());
        assert_eq!(rc, 0);
        let body = body.expect("want_json returns the report body");
        let v: serde_json::Value = serde_json::from_str(&body).unwrap();
        let effs = |needle: &str| -> Vec<String> {
            v["functions"].as_array().into_iter().flatten()
                .filter(|f| f["fn"].as_str() == Some(needle))
                .flat_map(|f| f["inferred"].as_array().into_iter().flatten().filter_map(|e| e.as_str().map(String::from)))
                .collect()
        };
        assert!(effs("run_imported").contains(&"Fs".to_string()),
                "R4: a `dyn` receiver typed by an IMPORTED dependency trait must dispatch to the LOCAL impl:\n{body}");
        assert!(effs("run_boxed").contains(&"Fs".to_string()),
                "R4: the `Box<dyn …>` spelling takes the same route:\n{body}");
        assert!(effs("stdw::writes").is_empty(),
                "CARVE-OUT 1 (provenance/std): `&dyn std::io::Write` must NOT CHA a local `impl Write` (fabrication):\n{body}");
        assert!(effs("run_bound").is_empty() && effs("run_impl").is_empty(),
                "CARVE-OUT 2 (erasure): a caller-monomorphized bound / `impl Trait` must NOT CHA local impls (the serde_json flood):\n{body}");
        assert!(effs("reexp::writes_reexported").is_empty(),
                "CARVE-OUT 3 (crate-local): a `self::`-rooted re-export is NOT a dependency (the value-bag flood):\n{body}");
        assert!(effs("nested").is_empty(),
                "CARVE-OUT 4 (nested item): an inner fn's own generic must not inherit the outer signature's `dyn`-ness:\n{body}");
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn fully_qualified_dependency_trait_forms_the_crate_key_r6() {
        // R6, the scan-boundary vein: `fn f(h: &dyn deplib::Handler)` — the SAME receiver as R4, written
        // FULLY QUALIFIED instead of imported — read silent-pure. `bound_leaves` keeps only
        // `segments.last()` (every downstream index is leaf-keyed), and with no `use` to expand through,
        // `expand` handed back a bare `Handler`: no `::`, so NO dependency key was emitted and no CHA ran.
        // The same code spelled `use deplib::Handler` resolved. `sig_trait_quals` keeps the path the
        // signature actually wrote, so both spellings form the same crate-qualified key.
        //
        // Measured on the real corpus: this is what makes tracing's
        // `__tracing_log(logger: &'static dyn log::Log, …) { logger.log(…) }` read `Log` instead of pure.
        let d = std::env::temp_dir().join(format!("candor-r6-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(d.join("src")).unwrap();
        std::fs::write(d.join("Cargo.toml"), "[package]\nname = \"r6\"\n").unwrap();
        std::fs::write(
            d.join("src/lib.rs"),
            r#"
            // NOTE: no `use deplib::Handler` anywhere — every mention is spelled in full.
            pub struct MyH;
            impl deplib::Handler for MyH { fn go(&self) { let _ = std::fs::read_to_string("/etc/hosts"); } }

            // POSITIVE: a fully-qualified `dyn` receiver dispatches to the local impl.
            pub fn run_qualified(h: &dyn deplib::Handler) { h.go(); }
            pub fn run_qualified_box(h: Box<dyn deplib::Handler>) { h.go(); }

            // POSITIVE (the classifier half): with the crate-qualified key formed, a call the CLASSIFIER
            // knows reaches its effect even with no local impl at all. This is tracing's
            // `__tracing_log(logger: &'static dyn log::Log, …) { logger.log(…) }` verbatim — pure before.
            pub fn logs(l: &dyn deplib::Logger) { l.log("x"); }

            // RESIDUAL, asserted so it can't drift silently: the erasure carve-out still applies to the
            // qualified spelling — a caller-monomorphized bound / `impl Trait` does NOT CHA local impls.
            pub fn run_qualified_bound<T: deplib::Handler>(t: T) { t.go(); }
            pub fn run_qualified_impl(h: impl deplib::Handler) { h.go(); }

            // CONTROL: a `crate::`-rooted qualified spelling is CRATE-LOCAL, not a dependency. `expand`
            // strips the root, so recording it would turn `crate::inner::Marker` into a dependency-looking
            // `inner::Marker` and CHA `Loud`'s Net onto it.
            pub mod inner { pub use std::io::Write; }
            pub struct Loud;
            impl std::io::Write for Loud {
                fn write(&mut self, b: &[u8]) -> std::io::Result<usize> { let _ = std::net::TcpStream::connect("h:1"); Ok(b.len()) }
                fn flush(&mut self) -> std::io::Result<()> { let _ = std::net::TcpStream::connect("h:1"); Ok(()) }
            }
            pub fn writes_crate_rooted(w: &mut dyn crate::inner::Write) { let _ = w.flush(); }
            "#,
        )
        .unwrap();
        let idx = load_dep_reports(None);
        let prefix = d.join("out/r").to_string_lossy().into_owned();
        let (rc, body) = scan_one(&d.to_string_lossy(), ScanOpts {
            prefix, want_json: true, include_tests: false, policy: None, baseline: None, ws_member: false, quiet: true, deps_idx: &idx, peek_excluded: false,
        }, &crate::gate::begin_run());
        assert_eq!(rc, 0);
        let body = body.expect("want_json returns the report body");
        let v: serde_json::Value = serde_json::from_str(&body).unwrap();
        let fun = |needle: &str| -> Option<&serde_json::Value> {
            v["functions"].as_array().into_iter().flatten().find(|f| f["fn"].as_str() == Some(needle))
        };
        let effs = |needle: &str| -> Vec<String> {
            fun(needle).into_iter()
                .flat_map(|f| f["inferred"].as_array().into_iter().flatten().filter_map(|e| e.as_str().map(String::from)))
                .collect()
        };
        assert!(effs("run_qualified").contains(&"Fs".to_string()),
                "R6: a FULLY-QUALIFIED `&dyn deplib::Handler` must dispatch like the imported spelling:\n{body}");
        assert!(effs("run_qualified_box").contains(&"Fs".to_string()),
                "R6: `Box<dyn deplib::Handler>` takes the same route:\n{body}");
        assert!(effs("logs").contains(&"Log".to_string()),
                "R6: the crate-qualified key must reach the CLASSIFIER too (the tracing `dyn log::Log` shape):\n{body}");
        assert!(effs("run_qualified_bound").is_empty() && effs("run_qualified_impl").is_empty(),
                "RESIDUAL: the erasure carve-out still applies to the qualified spelling:\n{body}");
        assert!(effs("writes_crate_rooted").is_empty(),
                "CONTROL: a `crate::`-rooted spelling is crate-LOCAL and must not be treated as a dependency:\n{body}");
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn collection_of_trait_objects_iteration_dispatches() {
        // Iterating a COLLECTION OF TRAIT OBJECTS (`for it in &items` / `.iter().for_each(|it| ..)` over a
        // `Vec<Box<dyn Doer>>`) dispatches the element's method to the impls via bounded CHA. The `dyn`
        // element has no nominal type, so `elem_of` couldn't hold it and the loop/closure var dropped to
        // pure — the `elem_trait_of` route types it into `trait_vars`. A CONCRETE-element collection with a
        // pure method stays pure (no over-fire).
        let run = |name: &str, src: &str| -> serde_json::Value {
            let d = std::env::temp_dir().join(format!("candor-dynvec-{name}-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&d);
            std::fs::create_dir_all(d.join("src")).unwrap();
            std::fs::write(d.join("Cargo.toml"), format!("[package]\nname = \"{name}\"\n")).unwrap();
            std::fs::write(d.join("src/lib.rs"), src).unwrap();
            let prefix = d.join("out/r").to_string_lossy().into_owned();
            let idx = load_dep_reports(None);
            let (rc, body) = scan_one(&d.to_string_lossy(), ScanOpts {
                prefix, want_json: true, include_tests: false, policy: None, baseline: None, ws_member: false, quiet: true, deps_idx: &idx, peek_excluded: false,
            }, &crate::gate::begin_run());
            assert_eq!(rc, 0);
            let v: serde_json::Value = serde_json::from_str(&body.unwrap()).unwrap();
            let _ = std::fs::remove_dir_all(&d);
            v
        };
        let eff = |v: &serde_json::Value, needle: &str| -> Vec<String> {
            v["functions"].as_array().into_iter().flatten()
                .filter(|f| f["fn"].as_str().is_some_and(|q| q.ends_with(needle)))
                .flat_map(|f| f["inferred"].as_array().into_iter().flatten()
                    .filter_map(|e| e.as_str().map(String::from)).collect::<Vec<_>>()).collect()
        };
        let src = r#"
use std::fs;
pub trait Doer { fn go(&self); }
pub struct Impl;
impl Doer for Impl { fn go(&self) { let _ = fs::write("/x", "y"); } }   // Fs
pub fn via_for(items: Vec<Box<dyn Doer>>) { for it in &items { it.go(); } }
pub fn via_slice(items: &[Box<dyn Doer>]) { for it in items { it.go(); } }
pub fn via_foreach(items: Vec<Box<dyn Doer>>) { items.iter().for_each(|it| it.go()); }
pub fn via_generic<T: Doer>(items: Vec<T>) { for it in &items { it.go(); } }   // generic-bound element
pub fn via_generic_where<T>(items: Vec<T>) where T: Doer { items.iter().for_each(|it| it.go()); }
pub struct Registry { handlers: Vec<Box<dyn Doer>> }
impl Registry {
    pub fn field_for(&self) { for h in &self.handlers { h.go(); } }                 // Fs — FIELD form
    pub fn field_foreach(&self) { self.handlers.iter().for_each(|h| h.go()); }       // Fs
}
pub struct GReg<T: Doer> { items: Vec<T> }
impl<T: Doer> GReg<T> { pub fn field_generic(&self) { for it in &self.items { it.go(); } } }  // Fs — generic FIELD
pub struct Plain;
impl Plain { pub fn go(&self) {} }
pub fn via_concrete(xs: Vec<Plain>) { for x in &xs { x.go(); } }        // PURE (no over-fire)
pub struct PReg { xs: Vec<Plain> }
impl PReg { pub fn field_pure(&self) { for x in &self.xs { x.go(); } } }  // PURE (concrete field)
"#;
        let v = run("dynvec", src);
        assert!(eff(&v, "field_for").contains(&"Fs".to_string()), "for-loop over a Vec<Box<dyn>> FIELD lost the dispatch:\n{v}");
        assert!(eff(&v, "field_foreach").contains(&"Fs".to_string()), "for_each over a Vec<Box<dyn>> FIELD lost it:\n{v}");
        assert!(eff(&v, "field_generic").contains(&"Fs".to_string()), "for-loop over a generic Vec<T: Doer> FIELD lost it:\n{v}");
        assert!(eff(&v, "field_pure").is_empty(), "a concrete-element Vec FIELD with a pure method must stay pure:\n{v}");
        assert!(eff(&v, "via_for").contains(&"Fs".to_string()), "for-loop over Vec<Box<dyn>> lost the dispatch:\n{v}");
        assert!(eff(&v, "via_slice").contains(&"Fs".to_string()), "for-loop over &[Box<dyn>] lost the dispatch:\n{v}");
        assert!(eff(&v, "via_foreach").contains(&"Fs".to_string()), "iter().for_each closure over Vec<Box<dyn>> lost it:\n{v}");
        assert!(eff(&v, "via_generic").contains(&"Fs".to_string()), "for-loop over a generic Vec<T: Doer> lost the dispatch:\n{v}");
        assert!(eff(&v, "via_generic_where").contains(&"Fs".to_string()), "where-clause generic Vec<T> for_each lost it:\n{v}");
        assert!(eff(&v, "via_concrete").is_empty(), "a concrete-element Vec with a pure method must stay pure:\n{v}");
    }

    #[test]
    fn opaque_callable_passed_directly_to_a_sync_invoker_is_unknown() {
        // An OPAQUE callable (a generic `F: Fn`/`impl Fn` param) passed BY VALUE to a synchronous
        // callback-invoker — `xs.iter().for_each(cb)`, `o.map(cb)`, `o.and_then(cb)` — is invoked on a
        // body the syntactic scan can't see, so the enclosing fn must read Unknown. The DIRECT-pass form
        // was silently dropped as pure while the CLOSURE-WRAPPED form `for_each(|x| cb(x))` and the
        // direct-call form `cb()` were already Unknown — the asymmetry was the under-report. This is the
        // Rust arm of the four-way sync-callback parity fix (candor-java c755acd). Inline closures keep
        // their analyzed body effect (no regression); resolvable named fns keep their resolved effect.
        let run = |name: &str, src: &str| -> serde_json::Value {
            let d = std::env::temp_dir().join(format!("candor-syncb-{name}-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&d);
            std::fs::create_dir_all(d.join("src")).unwrap();
            std::fs::write(d.join("Cargo.toml"), format!("[package]\nname = \"{name}\"\n")).unwrap();
            std::fs::write(d.join("src/lib.rs"), src).unwrap();
            let prefix = d.join("out/r").to_string_lossy().into_owned();
            let idx = load_dep_reports(None);
            let (rc, body) = scan_one(&d.to_string_lossy(), ScanOpts {
                prefix, want_json: true, include_tests: false, policy: None, baseline: None, ws_member: false, quiet: true, deps_idx: &idx, peek_excluded: false,
            }, &crate::gate::begin_run());
            assert_eq!(rc, 0);
            let v: serde_json::Value = serde_json::from_str(&body.unwrap()).unwrap();
            let _ = std::fs::remove_dir_all(&d);
            v
        };
        let eff = |v: &serde_json::Value, needle: &str| -> Vec<String> {
            v["functions"].as_array().into_iter().flatten()
                .filter(|f| f["fn"].as_str().is_some_and(|q| q.ends_with(needle)))
                .flat_map(|f| f["inferred"].as_array().into_iter().flatten()
                    .filter_map(|e| e.as_str().map(String::from)).collect::<Vec<_>>()).collect()
        };
        let v = run("syncb", r#"
use std::fs;
// THE BUG: an opaque callable passed DIRECTLY to for_each leaked pure.
pub fn opaque_passed<F: Fn(&i32)>(items: &[i32], cb: F) { items.iter().for_each(cb); }
// The closure-wrapped twin was already Unknown — must STAY Unknown (control).
pub fn opaque_direct<F: Fn(i32)>(items: &[i32], cb: F) { items.iter().for_each(|&x| cb(x)); }
// Through a `&` and a fn-typed rebind — the fn-typed predicate peels both.
pub fn opaque_ref<F: Fn(&i32)>(items: &[i32], cb: F) { items.iter().for_each(&cb); }
pub fn opaque_rebind<F: Fn(&i32)>(items: &[i32], cb: F) { let g = cb; items.iter().for_each(g); }
// Option/Result synchronous combinators.
pub fn opt_map<F: Fn(i32)->i32>(o: Option<i32>, cb: F) -> Option<i32> { o.map(cb) }
pub fn opt_and_then<F: Fn(i32)->Option<i32>>(o: Option<i32>, cb: F) -> Option<i32> { o.and_then(cb) }
pub fn res_map<F: Fn(i32)->i32>(r: Result<i32,()>, cb: F) -> Result<i32,()> { r.map(cb) }
// NO-REGRESSION: an inline closure with a real effect must still report it.
pub fn inline_eff(items: &[i32]) { items.iter().for_each(|_| { let _ = fs::write("/tmp/z", "w"); }); }
// NO-REGRESSION: a pure inline closure must stay pure (no over-disclosure).
pub fn inline_pure(items: &[i32]) { items.iter().for_each(|x| { let _ = x + 1; }); }
// NO-REGRESSION: a resolvable named fn keeps its RESOLVED effect (pure here → stays pure).
fn helper_pure(_x: &i32) {}
pub fn named_pure(items: &[i32]) { items.iter().for_each(helper_pure); }
// NO-REGRESSION: a resolvable named EFFECTFUL fn is still charged (not blanket-Unknown).
fn helper_eff(_x: &i32) { let _ = fs::write("/tmp/n", "e"); }
pub fn named_eff(items: &[i32]) { items.iter().for_each(helper_eff); }
"#);
        // The bug + the parity forms: opaque callable passed directly → Unknown.
        for f in ["opaque_passed", "opaque_ref", "opaque_rebind", "opt_map", "opt_and_then", "res_map"] {
            assert!(eff(&v, f).contains(&"Unknown".to_string()),
                "opaque callable passed directly to a sync invoker must be Unknown: {f} = {:?}\n{v}", eff(&v, f));
        }
        // Control: the already-working closure-wrapped form must not regress.
        assert!(eff(&v, "opaque_direct").contains(&"Unknown".to_string()),
            "closure-wrapped opaque call must stay Unknown:\n{v}");
        // No regression: inline closure with a real effect still reports it.
        assert!(eff(&v, "inline_eff").contains(&"Fs".to_string()),
            "inline-closure for_each with a real effect must keep it:\n{v}");
        // No over-disclosure: pure inline closure + pure named fn stay pure.
        assert!(eff(&v, "inline_pure").is_empty(),
            "a pure inline for_each must stay pure (no over-disclosure): {:?}", eff(&v, "inline_pure"));
        assert!(eff(&v, "named_pure").is_empty(),
            "a resolvable pure named fn must stay pure (no blanket-Unknown): {:?}", eff(&v, "named_pure"));
        // Resolvable effectful named fn is still charged precisely.
        assert!(eff(&v, "named_eff").contains(&"Fs".to_string()),
            "a resolvable effectful named fn must keep its resolved effect:\n{v}");
    }

    #[test]
    fn method_returning_collection_of_trait_objects_dispatches() {
        // `for d in r.all()` / `self.all().iter().for_each(..)` where `all() -> Vec<Box<dyn Doer>>`, and
        // `if let Some(d) = self.opt()` where `opt() -> Option<Box<dyn Doer>>` — a method/factory returning
        // a COLLECTION (or Option) of trait objects, iterated/unwrapped. `type_path` recorded the Vec return
        // as "Vec" (useless), so the element dispatch dropped silent-pure; a new `<elemdyn>` return sentinel
        // (+ the scalar `<dyn>` for the Option form) is decoded by `resolve_elem_trait_leaves`. A concrete-
        // element collection stays pure.
        let run = |src: &str| -> serde_json::Value {
            let d = std::env::temp_dir().join(format!("candor-mv-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&d);
            std::fs::create_dir_all(d.join("src")).unwrap();
            std::fs::write(d.join("Cargo.toml"), "[package]\nname = \"mv\"\n").unwrap();
            std::fs::write(d.join("src/lib.rs"), src).unwrap();
            let prefix = d.join("out/r").to_string_lossy().into_owned();
            let idx = load_dep_reports(None);
            let (rc, body) = scan_one(&d.to_string_lossy(), ScanOpts {
                prefix, want_json: true, include_tests: false, policy: None, baseline: None, ws_member: false, quiet: true, deps_idx: &idx, peek_excluded: false,
            }, &crate::gate::begin_run());
            assert_eq!(rc, 0);
            let v: serde_json::Value = serde_json::from_str(&body.unwrap()).unwrap();
            let _ = std::fs::remove_dir_all(&d);
            v
        };
        let eff = |v: &serde_json::Value, needle: &str| -> Vec<String> {
            v["functions"].as_array().into_iter().flatten()
                .filter(|f| f["fn"].as_str().is_some_and(|q| q.ends_with(needle)))
                .flat_map(|f| f["inferred"].as_array().into_iter().flatten()
                    .filter_map(|e| e.as_str().map(String::from)).collect::<Vec<_>>()).collect()
        };
        let v = run(r#"
use std::fs;
pub trait Doer { fn go(&self); }
pub struct Impl; impl Doer for Impl { fn go(&self) { let _ = fs::write("/x","y"); } }
pub struct Reg;
impl Reg {
    pub fn all(&self) -> Vec<Box<dyn Doer>> { vec![] }
    pub fn via_vec(&self) { for d in self.all() { d.go(); } }
    pub fn via_foreach(&self) { self.all().iter().for_each(|d| d.go()); }
    pub fn opt(&self) -> Option<Box<dyn Doer>> { None }
    pub fn via_opt(&self) { if let Some(d) = self.opt() { d.go(); } }
    pub fn plains(&self) -> Vec<Plain> { vec![] }
    pub fn via_plain(&self) { for p in self.plains() { p.go(); } }
}
pub fn free_all() -> Vec<Box<dyn Doer>> { vec![] }
pub fn via_free(){ for d in free_all() { d.go(); } }
pub struct Plain; impl Plain { pub fn go(&self) {} }
"#);
        for f in ["via_vec", "via_foreach", "via_opt", "via_free"] {
            assert!(eff(&v, f).contains(&"Fs".to_string()), "{f} lost the returned-collection dispatch:\n{v}");
        }
        assert!(eff(&v, "via_plain").is_empty(), "a method returning Vec of a concrete pure type must stay pure:\n{v}");
    }

    #[test]
    fn supertrait_method_dispatches_via_sub_bound() {
        // `t.base()` where `base ∈ Super`, `t: T: Sub` (or `&dyn Sub`), `trait Sub: Super` — a supertrait
        // method is callable on a Sub receiver and the sub's impls provide it, so it must dispatch (was
        // silent-pure: the `lt.methods.contains(leaf)` gate rejected an inherited method). A sub's OWN pure
        // method stays pure; an unrelated same-named trait must not hijack.
        let run = |src: &str| -> serde_json::Value {
            let d = std::env::temp_dir().join(format!("candor-st-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&d);
            std::fs::create_dir_all(d.join("src")).unwrap();
            std::fs::write(d.join("Cargo.toml"), "[package]\nname = \"st\"\n").unwrap();
            std::fs::write(d.join("src/lib.rs"), src).unwrap();
            let prefix = d.join("out/r").to_string_lossy().into_owned();
            let idx = load_dep_reports(None);
            let (rc, body) = scan_one(&d.to_string_lossy(), ScanOpts {
                prefix, want_json: true, include_tests: false, policy: None, baseline: None, ws_member: false, quiet: true, deps_idx: &idx, peek_excluded: false,
            }, &crate::gate::begin_run());
            assert_eq!(rc, 0);
            let v: serde_json::Value = serde_json::from_str(&body.unwrap()).unwrap();
            let _ = std::fs::remove_dir_all(&d);
            v
        };
        let eff = |v: &serde_json::Value, needle: &str| -> Vec<String> {
            v["functions"].as_array().into_iter().flatten()
                .filter(|f| f["fn"].as_str().is_some_and(|q| q.ends_with(needle)))
                .flat_map(|f| f["inferred"].as_array().into_iter().flatten()
                    .filter_map(|e| e.as_str().map(String::from)).collect::<Vec<_>>()).collect()
        };
        let v = run(r#"
use std::fs;
pub trait Super { fn base(&self); }
pub trait Sub: Super { fn extra(&self); }
pub struct Impl;
impl Super for Impl { fn base(&self) { let _ = fs::write("/x","y"); } }   // Fs
impl Sub for Impl { fn extra(&self) {} }
pub fn via_generic<T: Sub>(t: &T) { t.base(); }
pub fn via_dyn(d: &dyn Sub) { d.base(); }
pub fn via_own<T: Sub>(t: &T) { t.extra(); }
pub trait Other { fn base(&self); }
struct O; impl Other for O { fn base(&self) { let _ = fs::write("/z","!"); } }
"#);
        assert!(eff(&v, "via_generic").contains(&"Fs".to_string()), "supertrait method via a generic Sub bound lost the dispatch:\n{v}");
        assert!(eff(&v, "via_dyn").contains(&"Fs".to_string()), "supertrait method via &dyn Sub lost the dispatch:\n{v}");
        assert!(eff(&v, "via_own").is_empty(), "the sub's OWN pure method must stay pure:\n{v}");
    }

    #[test]
    fn method_factory_returning_trait_object_dispatches() {
        // `self.handler().go()` where `handler(&self) -> &dyn Doer` / `-> Box<dyn Doer>` — a METHOD factory
        // returning a dispatch object. `resolve_recv_type` used to walk THROUGH the chain to the base
        // receiver's type (`Reg`) and shadow the dispatch silent-pure; only a free/static `Reg::make()`
        // (an Expr::Call) resolved. A concrete-return method must stay pure (no over-fire).
        let run = |src: &str| -> serde_json::Value {
            let d = std::env::temp_dir().join(format!("candor-mf-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&d);
            std::fs::create_dir_all(d.join("src")).unwrap();
            std::fs::write(d.join("Cargo.toml"), "[package]\nname = \"mf\"\n").unwrap();
            std::fs::write(d.join("src/lib.rs"), src).unwrap();
            let prefix = d.join("out/r").to_string_lossy().into_owned();
            let idx = load_dep_reports(None);
            let (rc, body) = scan_one(&d.to_string_lossy(), ScanOpts {
                prefix, want_json: true, include_tests: false, policy: None, baseline: None, ws_member: false, quiet: true, deps_idx: &idx, peek_excluded: false,
            }, &crate::gate::begin_run());
            assert_eq!(rc, 0);
            let v: serde_json::Value = serde_json::from_str(&body.unwrap()).unwrap();
            let _ = std::fs::remove_dir_all(&d);
            v
        };
        let eff = |v: &serde_json::Value, needle: &str| -> Vec<String> {
            v["functions"].as_array().into_iter().flatten()
                .filter(|f| f["fn"].as_str().is_some_and(|q| q.ends_with(needle)))
                .flat_map(|f| f["inferred"].as_array().into_iter().flatten()
                    .filter_map(|e| e.as_str().map(String::from)).collect::<Vec<_>>()).collect()
        };
        let v = run(r#"
use std::fs;
pub trait Doer { fn go(&self); }
pub struct Impl; impl Doer for Impl { fn go(&self) { let _ = fs::write("/x","y"); } }
pub struct Reg { h: Box<dyn Doer> }
impl Reg {
    pub fn handler(&self) -> &dyn Doer { &*self.h }
    pub fn via_ref(&self) { self.handler().go(); }
    pub fn boxed(&self) -> Box<dyn Doer> { Box::new(Impl) }
    pub fn via_boxed(&self) { self.boxed().go(); }
    pub fn plain(&self) -> Plain { Plain }
    pub fn via_plain(&self) { self.plain().go(); }
}
pub struct Plain; impl Plain { pub fn go(&self) {} }
"#);
        assert!(eff(&v, "via_ref").contains(&"Fs".to_string()), "method returning &dyn lost the dispatch:\n{v}");
        assert!(eff(&v, "via_boxed").contains(&"Fs".to_string()), "method returning Box<dyn> lost the dispatch:\n{v}");
        assert!(eff(&v, "via_plain").is_empty(), "method returning a concrete type with a pure method must stay pure:\n{v}");
    }

    #[test]
    fn container_and_option_trait_object_dispatch_variants() {
        // The collection-of-trait-objects vein beyond a plain Vec param/field: HashMap VALUES, a smart-
        // pointer / interior-mutability GUARD chain (`Arc<Mutex<Vec<Box<dyn>>>>`), and Option/Result unwrap
        // in ALL its forms (if-let, match, let-else, `.map`, `for`). Each dispatches the element/payload's
        // method via bounded CHA; a concrete-element container / pure-arm stays pure (no over-fire).
        let run = |name: &str, src: &str| -> serde_json::Value {
            let d = std::env::temp_dir().join(format!("candor-cv-{name}-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&d);
            std::fs::create_dir_all(d.join("src")).unwrap();
            std::fs::write(d.join("Cargo.toml"), format!("[package]\nname = \"{name}\"\n")).unwrap();
            std::fs::write(d.join("src/lib.rs"), src).unwrap();
            let prefix = d.join("out/r").to_string_lossy().into_owned();
            let idx = load_dep_reports(None);
            let (rc, body) = scan_one(&d.to_string_lossy(), ScanOpts {
                prefix, want_json: true, include_tests: false, policy: None, baseline: None, ws_member: false, quiet: true, deps_idx: &idx, peek_excluded: false,
            }, &crate::gate::begin_run());
            assert_eq!(rc, 0);
            let v: serde_json::Value = serde_json::from_str(&body.unwrap()).unwrap();
            let _ = std::fs::remove_dir_all(&d);
            v
        };
        let eff = |v: &serde_json::Value, needle: &str| -> Vec<String> {
            v["functions"].as_array().into_iter().flatten()
                .filter(|f| f["fn"].as_str().is_some_and(|q| q.ends_with(needle)))
                .flat_map(|f| f["inferred"].as_array().into_iter().flatten()
                    .filter_map(|e| e.as_str().map(String::from)).collect::<Vec<_>>()).collect()
        };
        let src = r#"
use std::fs;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
pub trait Doer { fn go(&self); }
pub struct Impl; impl Doer for Impl { fn go(&self) { let _ = fs::write("/x", "y"); } }   // Fs
pub fn map_values(m: HashMap<String, Box<dyn Doer>>) { for v in m.values() { v.go(); } }
pub fn arc_mutex(r: Arc<Mutex<Vec<Box<dyn Doer>>>>) { for h in r.lock().unwrap().iter() { h.go(); } }
pub fn opt_iflet(o: Option<Box<dyn Doer>>) { if let Some(d) = o { d.go(); } }
pub fn opt_match(o: Option<Box<dyn Doer>>) { match o { Some(d) => d.go(), None => {} } }
pub fn opt_letelse(o: Option<Box<dyn Doer>>) { let Some(d) = o else { return; }; d.go(); }
pub fn opt_map(o: Option<Box<dyn Doer>>) { o.map(|d| d.go()); }
pub fn res_iflet(r: Result<Box<dyn Doer>, ()>) { if let Ok(d) = r { d.go(); } }
// NESTED dispatch containers (R46): the peel composes in any order via `elem_trait_leaves` +
// the `trait_vars` carry through the next unwrap.
pub fn nested_vec_opt(xs: Vec<Option<Box<dyn Doer>>>) { for x in xs { if let Some(d) = x { d.go(); } } }
pub fn nested_opt_vec(xs: Option<Vec<Box<dyn Doer>>>) { if let Some(v) = xs { for d in v { d.go(); } } }
// TUPLE-of-dyn (R46 tuple): a param, an inline cast tuple, and a factory-return tuple, all destructured
pub fn tuple_param(pair: (Box<dyn Doer>, u32)) { let (d, _n) = pair; d.go(); }
pub fn tuple_direct() { let (d, _n) = (Box::new(Impl) as Box<dyn Doer>, 1u32); d.go(); }
fn make_pair() -> (Box<dyn Doer>, u32) { (Box::new(Impl), 1) }
pub fn tuple_factory() { let (d, _n) = make_pair(); d.go(); }
pub struct Plain; impl Plain { pub fn go(&self) {} }
pub fn map_concrete(m: HashMap<String, Plain>) { for v in m.values() { v.go(); } }   // PURE
pub fn opt_concrete(o: Option<Plain>) { if let Some(d) = o { d.go(); } }             // PURE
pub fn nested_concrete(xs: Vec<Option<Plain>>) { for x in xs { if let Some(d) = x { d.go(); } } } // PURE
"#;
        let v = run("cv", src);
        for f in ["map_values", "arc_mutex", "opt_iflet", "opt_match", "opt_letelse", "opt_map", "res_iflet",
                  "nested_vec_opt", "nested_opt_vec", "tuple_param", "tuple_direct", "tuple_factory"] {
            assert!(eff(&v, f).contains(&"Fs".to_string()), "{f} lost the container/option dispatch:\n{v}");
        }
        assert!(eff(&v, "map_concrete").is_empty(), "a concrete-value HashMap must stay pure:\n{v}");
        assert!(eff(&v, "opt_concrete").is_empty(), "a concrete Option payload must stay pure:\n{v}");
        assert!(eff(&v, "nested_concrete").is_empty(), "a nested concrete-payload container must stay pure:\n{v}");
    }

    /// R71 (SOUNDNESS, cardinal sin, live in published 0.34.0): a closure invoked through an `if let`
    /// binding was silently dropped as pure — `if let Some(f) = &self.cb { f() }` left the invoking fn
    /// ABSENT from `functions[]` entirely (no effects, no `Unknown`, no disclosure), while the SAME
    /// closure invoked directly (`(self.cb.as_ref().unwrap())()`) already read the honest
    /// `Unknown`/`unresolved:true`/`unknownWhy: ["callback:unresolved call"]`.
    ///
    /// ROOT CAUSE (measured, not the premise this fix was handed): `resolve_elem_trait_leaves` DOES
    /// return non-empty (`["Fn"]`) for an `Option<Box<dyn Fn()>>` field — the claim that Fn/FnMut/FnOnce
    /// yield no CHA leaves does not hold — and the binding DOES land in `trait_vars`. The gap is that
    /// `trait_vars` feeds ONLY the `.method()`-call CHA/dispatch resolver; the CALL-SYNTAX resolver
    /// (`visit_expr_call`'s bare-`Path` arm) consults only `fn_typed_vars`. A `Fn`-family binding that
    /// stops at `trait_vars` is therefore live and irrelevant, and `f()` resolves as a phantom free-fn
    /// call and drops silently. The fix hedges every such binding into `fn_typed_vars` too.
    ///
    /// AUDIT BOUNDARY: every OTHER binding-producing position sharing the identical shape — match-arm
    /// `Some`/`Ok`, for-loop var, let-else, while-let (previously UNHANDLED for ANY trait, not just Fn),
    /// a HOF closure param, an unannotated/annotated tuple destructure, an annotated closure param — is
    /// exercised here too, each with ground truth checked by hand against the built binary before this
    /// test was written (scratchpad fx1/fx5/fx13, R71 report). NOT covered (a documented, separate gap,
    /// left open): a bare closure carried as a single-field tuple-variant ENUM payload — `EnumVariantIndex`
    /// only records a variant whose payload resolves via `type_path`, which returns `None` for a `dyn`
    /// position, and nothing mirrors the `trait_fields`/`field_elem_trait` PARALLEL-index pattern that
    /// closes the identical hole for struct fields.
    #[test]
    fn callback_invoked_through_a_binding_position_reads_unknown_not_silent_pure() {
        let run = |name: &str, src: &str| -> serde_json::Value {
            let d = std::env::temp_dir().join(format!("candor-r71-{name}-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&d);
            std::fs::create_dir_all(d.join("src")).unwrap();
            std::fs::write(d.join("Cargo.toml"), format!("[package]\nname = \"{name}\"\n")).unwrap();
            std::fs::write(d.join("src/lib.rs"), src).unwrap();
            let prefix = d.join("out/r").to_string_lossy().into_owned();
            let idx = load_dep_reports(None);
            let (rc, body) = scan_one(&d.to_string_lossy(), ScanOpts {
                prefix, want_json: true, include_tests: false, policy: None, baseline: None, ws_member: false, quiet: true, deps_idx: &idx, peek_excluded: false,
            }, &crate::gate::begin_run());
            assert_eq!(rc, 0);
            let v: serde_json::Value = serde_json::from_str(&body.unwrap()).unwrap();
            let _ = std::fs::remove_dir_all(&d);
            v
        };
        let eff = |v: &serde_json::Value, needle: &str| -> Vec<String> {
            v["functions"].as_array().into_iter().flatten()
                .filter(|f| f["fn"].as_str().is_some_and(|q| q.ends_with(needle)))
                .flat_map(|f| f["inferred"].as_array().into_iter().flatten()
                    .filter_map(|e| e.as_str().map(String::from)).collect::<Vec<_>>()).collect()
        };
        let src = r#"
use std::fs;

// --- primary R71 shape: if-let over a struct field ---
pub struct IfLetHolder { cb: Option<Box<dyn Fn()>> }
impl IfLetHolder {
    pub fn new() -> Self { IfLetHolder { cb: Some(Box::new(|| { let _ = fs::write("/x", "y"); })) } }
    pub fn go(&self) { if let Some(f) = &self.cb { f(); } }
}
// honest baseline this fix must PRESERVE (already correct pre-fix): direct field call
pub struct DirectHolder { cb: Box<dyn Fn()> }
impl DirectHolder {
    pub fn new() -> Self { DirectHolder { cb: Box::new(|| { let _ = fs::write("/x", "y"); }) } }
    pub fn go(&self) { (self.cb)(); }
}
// while-let — previously had NO handler at all for ANY trait shape, not just Fn
pub struct WhileLetHolder { cb: Option<Box<dyn Fn()>> }
impl WhileLetHolder {
    pub fn new() -> Self { WhileLetHolder { cb: Some(Box::new(|| { let _ = fs::write("/x", "y"); })) } }
    pub fn go(&self) { while let Some(f) = &self.cb { f(); break; } }
}
// match Some/Ok arm
pub struct MatchHolder { cb: Option<Box<dyn Fn()>> }
impl MatchHolder {
    pub fn new() -> Self { MatchHolder { cb: Some(Box::new(|| { let _ = fs::write("/x", "y"); })) } }
    pub fn go(&self) { match &self.cb { Some(f) => f(), None => {} } }
}
// let-else
pub struct LetElseHolder { cb: Option<Box<dyn Fn()>> }
impl LetElseHolder {
    pub fn new() -> Self { LetElseHolder { cb: Some(Box::new(|| { let _ = fs::write("/x", "y"); })) } }
    pub fn go(&self) { let Some(f) = &self.cb else { return }; f(); }
}
// for-loop var over a collection of callbacks
pub struct ForHolder { callbacks: Vec<Box<dyn Fn()>> }
impl ForHolder {
    pub fn new() -> Self { ForHolder { callbacks: vec![Box::new(|| { let _ = fs::write("/x", "y"); })] } }
    pub fn go(&self) { for f in &self.callbacks { f(); } }
}
// HOF closure param over a collection of callbacks
pub struct HofHolder { callbacks: Vec<Box<dyn Fn()>> }
impl HofHolder {
    pub fn new() -> Self { HofHolder { callbacks: vec![Box::new(|| { let _ = fs::write("/x", "y"); })] } }
    pub fn go(&self) { self.callbacks.iter().for_each(|f| f()); }
}
// unannotated + annotated tuple destructure
pub fn tuple_unannotated(pair: (Box<dyn Fn()>, u32)) { let (f, _n) = pair; f(); }
pub fn tuple_annotated(pair: (Box<dyn Fn()>, u32)) { let (f, _n): (Box<dyn Fn()>, u32) = pair; f(); }
// annotated closure parameter
pub fn invoke_via_closure_param() {
    let invoker = |f: Box<dyn Fn()>| { f(); };
    invoker(Box::new(|| { let _ = fs::write("/x", "y"); }));
}

// --- OVER-CHARGE CONTROL 1: a PURE closure through if-let must match the EXISTING direct-field
// ceiling (Unknown — the scan has never been able to prove purity through an opaque callback), not
// something WORSE (a fabricated specific effect would be the actual regression to watch for).
pub struct PureDirectHolder { cb: Box<dyn Fn()> }
impl PureDirectHolder {
    pub fn new() -> Self { PureDirectHolder { cb: Box::new(|| {}) } }
    pub fn go(&self) { (self.cb)(); }
}
pub struct PureIfLetHolder { cb: Option<Box<dyn Fn()>> }
impl PureIfLetHolder {
    pub fn new() -> Self { PureIfLetHolder { cb: Some(Box::new(|| {})) } }
    pub fn go(&self) { if let Some(f) = &self.cb { f(); } }
}

// --- OVER-CHARGE CONTROL 2: an if-let over an ORDINARY, non-callable Option field must NOT start
// reading Unknown — the overwhelmingly common if-let-over-Option shape must stay exactly as before.
pub struct PlainHolder { name: Option<String> }
impl PlainHolder {
    pub fn new() -> Self { PlainHolder { name: Some("hi".to_string()) } }
    pub fn go(&self) { if let Some(n) = &self.name { let _ = fs::write("/x", n); } }
}
"#;
        let v = run("r71", src);
        for f in ["IfLetHolder::go", "DirectHolder::go", "WhileLetHolder::go", "MatchHolder::go",
                  "LetElseHolder::go", "ForHolder::go", "HofHolder::go",
                  "tuple_unannotated", "tuple_annotated"] {
            assert!(eff(&v, f).contains(&"Unknown".to_string()),
                    "{f} lost the callback disclosure — silently pure again (R71):\n{v}");
        }
        assert!(eff(&v, "invoke_via_closure_param").contains(&"Unknown".to_string()),
                "the closure-param call inside `invoker` lost its Unknown disclosure (R71):\n{v}");
        // over-charge control 1: PARITY, not "stays pure" — both must read the identical answer.
        let direct_pure = eff(&v, "PureDirectHolder::go");
        let iflet_pure = eff(&v, "PureIfLetHolder::go");
        assert_eq!(direct_pure, iflet_pure,
            "if-let over a PURE closure diverged from the established direct-field ceiling — over-charge:\ndirect={direct_pure:?} if-let={iflet_pure:?}\n{v}");
        assert!(direct_pure.contains(&"Unknown".to_string()), "sanity: the direct-field pure control itself must read Unknown:\n{v}");
        // over-charge control 2: an ordinary if-let-over-Option must be untouched by this fix.
        let plain = eff(&v, "PlainHolder::go");
        assert!(plain.contains(&"Fs".to_string()) && !plain.contains(&"Unknown".to_string()),
                "a non-callable if-let regressed into a spurious Unknown: {plain:?}\n{v}");
    }

    /// R101 (SOUNDNESS, cardinal sin, kernel-witnessed by driver `pf_oncelock_cb`): a callback installed
    /// from OUTSIDE through a static CELL and invoked later was silently dropped as pure — `static CB:
    /// OnceLock<Box<dyn Fn()>>` + `pub fn install(f) { CB.set(f) }` + `fn fire() { if let Some(f) =
    /// CB.get() { f() } }` left `fire` ABSENT from `functions[]` entirely while the program demonstrably
    /// wrote a file. Silent on `deny Fs`, `deny Unknown`, `deny Fs Unknown` and scoped `deny … fire`.
    ///
    /// THE SIBLING PATH WAS ALREADY RIGHT, and this converges on it rather than writing a second rule
    /// (§F1 item 3): the SAME opaque callable reached through a fn-typed PARAMETER (`via_param` here,
    /// driver `pf_oncelock_cb_ctl` in the syscall oracle) has always reported `Unknown` /
    /// `callback:unresolved call`. Post-fix the two answers are identical, field for field.
    ///
    /// ROOT CAUSE, measured. A `static`'s declared type was recorded in NO index — `const_strings` holds
    /// only literals and `lazy_statics` only names — so `resolve_elem_trait_leaves` had nothing to say
    /// about a static receiver, the binder's `leaves_are_callable` was false, `f` was never hedged into
    /// `fn_typed_vars`, and `f()` resolved as a phantom free-fn call and vanished. Two further gaps sat
    /// behind it: `elem_trait_leaves` did not peel the deferred-init cells (`OnceLock`/`OnceCell`/
    /// `LazyLock`/`LazyCell`/`Lazy`) though it already peeled `Mutex`/`RefCell`/`Cell`, and the
    /// element-preserving adapter list did not carry `get`/`get_mut`/`get_or_init`.
    ///
    /// SOUNDNESS DIRECTION: `callable_statics` yields only the synthetic `"Fn"` leaf that
    /// `ret_dispatch_leaves` already produces for `RET_FN_TYPED`. It matches no local trait, so this can
    /// hedge a binding to `Unknown` and can never contribute or withdraw a CONCRETE effect.
    ///
    /// EXECUTED. This exact source is a compiling, RUNNING binary — `install`/`install2`/`install_slot`
    /// are driven from a `main`, and `/tmp/r101-doer` and `/tmp/r101-param` are on disk afterwards, so the
    /// three absence-asserting controls below are asserting something about a program that exists (§E3).
    /// The first draft of the shadow control did NOT compile: `let CB = ..` where the static `CB` resolves
    /// is rustc **E0530**, "let bindings cannot shadow statics". A local can therefore only collide with a
    /// static's name from a module that does not see it — which is the collision that matters anyway,
    /// because this index is crate-wide by bare NAME, exactly like `lazy_statics`.
    #[test]
    fn callback_installed_through_a_static_cell_reads_unknown_not_silent_pure() {
        let run = |name: &str, src: &str| -> serde_json::Value {
            let d = std::env::temp_dir().join(format!("candor-r101-{name}-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&d);
            std::fs::create_dir_all(d.join("src")).unwrap();
            std::fs::write(d.join("Cargo.toml"), format!("[package]\nname = \"{name}\"\n")).unwrap();
            std::fs::write(d.join("src/lib.rs"), src).unwrap();
            let prefix = d.join("out/r").to_string_lossy().into_owned();
            let idx = load_dep_reports(None);
            let (rc, body) = scan_one(&d.to_string_lossy(), ScanOpts {
                prefix, want_json: true, include_tests: false, policy: None, baseline: None, ws_member: false, quiet: true, deps_idx: &idx, peek_excluded: false,
            }, &crate::gate::begin_run());
            assert_eq!(rc, 0);
            let v: serde_json::Value = serde_json::from_str(&body.unwrap()).unwrap();
            let _ = std::fs::remove_dir_all(&d);
            v
        };
        let eff = |v: &serde_json::Value, needle: &str| -> Vec<String> {
            v["functions"].as_array().into_iter().flatten()
                .filter(|f| f["fn"].as_str().is_some_and(|q| q == needle))
                .flat_map(|f| f["inferred"].as_array().into_iter().flatten()
                    .filter_map(|e| e.as_str().map(String::from)).collect::<Vec<_>>()).collect()
        };
        let src = r#"
use std::fs;
use std::sync::{Mutex, OnceLock};

pub trait Doer { fn go(&self); }
pub struct D;
impl Doer for D { fn go(&self) { let _ = fs::write("/tmp/r101-doer", "d"); } }

// --- R101 PRIMARY: a callback installed from OUTSIDE through a static CELL, invoked later. ---
static CB: OnceLock<Box<dyn Fn() + Send + Sync>> = OnceLock::new();
pub fn install(f: Box<dyn Fn() + Send + Sync>) { let _ = CB.set(f); }
pub fn fire_iflet() { if let Some(f) = CB.get() { f(); } }
pub fn fire_letelse() { let Some(f) = CB.get() else { return }; f(); }
pub fn fire_match() { match CB.get() { Some(f) => f(), None => {} } }

// the MODULE-QUALIFIED spelling of the same thing — `get_ident` is None for it, which is why the
// callable-static arm keys on the LAST path segment, like the lazy-static forcing edge.
pub mod reg {
    use std::sync::OnceLock;
    pub static CB2: OnceLock<Box<dyn Fn() + Send + Sync>> = OnceLock::new();
    pub fn install2(f: Box<dyn Fn() + Send + Sync>) { let _ = CB2.set(f); }
}
pub fn fire_qualified() { if let Some(f) = reg::CB2.get() { f(); } }

// the interior-mutability twin, reached through the guard chain the adapter list already peeled.
static SLOT: Mutex<Option<Box<dyn Fn() + Send + Sync>>> = Mutex::new(None);
pub fn install_slot(f: Box<dyn Fn() + Send + Sync>) { *SLOT.lock().unwrap() = Some(f); }
pub fn fire_mutex() { if let Some(f) = SLOT.lock().unwrap().as_ref() { f(); } }

// the `static mut` spelling, which REQUIRES `unsafe` to read. THE ONLY SHAPE ANY CRATE IN A
// 1489-CRATE REGISTRY ACTUALLY USES — proptest's `static mut DEFAULT_HOOK: Option<Box<dyn Fn(&PanicInfo)
// + Send + Sync>>`, read as `if let Some(hook) = unsafe { DEFAULT_HOOK.as_ref() }`. Without the
// unsafe-block peel the scrutinee never reached the static and the whole fix was SAFETY-ONLY.
static mut RAW: Option<Box<dyn Fn() + Send + Sync>> = None;
pub fn install_raw(f: Box<dyn Fn() + Send + Sync>) { unsafe { RAW = Some(f); } }
#[allow(static_mut_refs)]
pub fn fire_unsafe() { if let Some(f) = unsafe { RAW.as_ref() } { f(); } }

// --- CONTROL A (PARITY): the sibling path this fix converges ON, and must not disturb. ---
pub fn via_param(f: &dyn Fn()) { f(); }

// --- CONTROL B (OVER-CHARGE): a static cell whose element is NOT callable gains nothing. ---
static NAMES: OnceLock<Vec<String>> = OnceLock::new();
pub fn use_names() {
    if let Some(v) = NAMES.get() { let _ = fs::write("/tmp/r101-names", v.join(",")); }
}

// --- CONTROL C (SHADOW): a LOCAL of a static's name, from a module that does not see the static.
// Without the `locally_bound` guard the synthetic "Fn" leaf REPLACES ["Doer"], `d.go()` stops
// resolving and the Fs effect is LOST — measured by deleting the guard, not argued.
pub mod other {
    use super::{Doer, D};
    pub fn shadowed_local_wins() {
        let CB: Option<Box<dyn Doer>> = Some(Box::new(D));
        if let Some(d) = CB { d.go(); }
        let _ = std::env::var("R101_SHADOW");
    }
}

// --- CONTROL D (ADAPTER ORDERING): `returns` is keyed by bare method LEAF crate-wide, so one local
// `fn get` answers for every `.get()` in the crate. Receiver-first with `returns` as the FALLBACK is
// what keeps BOTH this and the primary shapes above; returns-first made the whole fix inert here.
pub struct Reg { pub tag: u8 }
impl Reg { pub fn get(&self) -> Option<Box<dyn Doer>> { Some(Box::new(D)) } }
pub fn use_reg(r: &Reg) { if let Some(d) = r.get() { d.go(); } let _ = r.tag; }

// --- CONTROL E (RULING, not a defect): a cell never `set` ANYWHERE in this crate still reads Unknown.
// Deliberate, and it is the whole point of the row: `CB` is `pub`-reachable through `install`, so a
// DOWNSTREAM crate installs the callback and no in-crate evidence of a write exists. `#[cfg]`-excluded
// arms, `include!`d files, macro expansions and `tests/` (not scanned by default) are all invisible
// here too. Narrowing on "no `set` in this crate" is a heuristic over an unprovable absence, and it
// would re-silence exactly the library-publisher case this row was filed for.
static NEVER: OnceLock<Box<dyn Fn() + Send + Sync>> = OnceLock::new();
pub fn fire_never() { if let Some(f) = NEVER.get() { f(); } }
"#;
        let v = run("r101", src);
        // Every binder shape over a callable static discloses, and the answer is the PARAMETER path's.
        let param = eff(&v, "via_param");
        assert_eq!(param, vec!["Unknown".to_string()],
            "sanity: the fn-typed-parameter path this fix converges ON must itself read Unknown:\n{v}");
        for f in ["fire_iflet", "fire_letelse", "fire_match", "fire_qualified", "fire_mutex",
                  "fire_unsafe", "fire_never"] {
            assert_eq!(eff(&v, f), param,
                "{f} did not converge on the fn-typed-parameter answer — silently pure again (R101):\n{v}");
        }
        // CONTROL B: a non-callable static cell must not start reading Unknown.
        let names = eff(&v, "use_names");
        assert!(names.contains(&"Fs".to_string()) && !names.contains(&"Unknown".to_string()),
            "a non-callable static cell regressed into a spurious Unknown: {names:?}\n{v}");
        // CONTROL C: the local wins, and its bounded-CHA dispatch to D::go SURVIVES. Both halves matter —
        // asserting only "no Unknown" would pass while the Fs was being lost.
        let shadow = eff(&v, "other::shadowed_local_wins");
        assert!(shadow.contains(&"Fs".to_string()) && shadow.contains(&"Env".to_string())
                && !shadow.contains(&"Unknown".to_string()),
            "a local shadowing a static's NAME was charged the static's meaning, losing its own dispatch: {shadow:?}\n{v}");
        // CONTROL D: a local `fn get` keeps its own return-type resolution through the fallback.
        let reg = eff(&v, "use_reg");
        assert!(reg.contains(&"Fs".to_string()) && !reg.contains(&"Unknown".to_string()),
            "a local `fn get`'s return-type resolution was displaced by the cell-accessor peel: {reg:?}\n{v}");
    }

    #[test]
    fn trait_default_dispatches_required_to_impl_witness() {
        // A LOCAL trait DEFAULT method calling a REQUIRED method (`fn save_all(&self){ self.persist() }`)
        // dispatches to the conforming impls' witnesses. Inside the default `self` types as the TRAIT, so
        // `self.persist()` is `Trait::persist` — a bodiless requirement (no unit), and type_to_traits keys
        // on impl types not the trait, so it read silent-pure (the rust sibling of the swift R32 protocol-
        // extension→conformer dispatch). Bounded CHA over the trait's impls; a PURE impl stays pure.
        let run = |name: &str, src: &str| -> serde_json::Value {
            let d = std::env::temp_dir().join(format!("candor-traitdef-{name}-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&d);
            std::fs::create_dir_all(d.join("src")).unwrap();
            std::fs::write(d.join("Cargo.toml"), format!("[package]\nname = \"{name}\"\n")).unwrap();
            std::fs::write(d.join("src/lib.rs"), src).unwrap();
            let prefix = d.join("out/r").to_string_lossy().into_owned();
            let idx = load_dep_reports(None);
            let (rc, body) = scan_one(&d.to_string_lossy(), ScanOpts {
                prefix, want_json: true, include_tests: false, policy: None, baseline: None, ws_member: false, quiet: true, deps_idx: &idx, peek_excluded: false,
            }, &crate::gate::begin_run());
            assert_eq!(rc, 0);
            let v: serde_json::Value = serde_json::from_str(&body.unwrap()).unwrap();
            let _ = std::fs::remove_dir_all(&d);
            v
        };
        let eff = |v: &serde_json::Value, needle: &str| -> Vec<String> {
            v["functions"].as_array().into_iter().flatten()
                .filter(|f| f["fn"].as_str().is_some_and(|q| q.ends_with(needle)))
                .flat_map(|f| f["inferred"].as_array().into_iter().flatten()
                    .filter_map(|e| e.as_str().map(String::from)).collect::<Vec<_>>()).collect()
        };
        let src = r#"
use std::fs;
pub trait Store { fn persist(&self); fn save_all(&self) { self.persist(); } }
pub struct Db;
impl Store for Db { fn persist(&self) { let _ = fs::write("/x", "y"); } }   // Fs
pub fn via_concrete(d: &Db) { d.save_all(); }
pub fn via_generic<T: Store>(t: &T) { t.save_all(); }
// PURE control — a trait default over a pure impl must stay pure
pub trait Pt { fn r(&self); fn d(&self) { self.r(); } }
pub struct Pure;
impl Pt for Pure { fn r(&self) {} }
pub fn via_pure(p: &Pure) { p.d(); }
"#;
        let v = run("traitdef", src);
        assert!(eff(&v, "via_concrete").contains(&"Fs".to_string()),
                "a trait default's self.persist() must dispatch to the impl witness (Fs):\n{v}");
        assert!(eff(&v, "via_generic").contains(&"Fs".to_string()),
                "the generic caller of a trait default must also carry:\n{v}");
        assert!(eff(&v, "Store::save_all").contains(&"Fs".to_string()),
                "the trait default unit itself carries the CHA'd witness effect:\n{v}");
        assert!(eff(&v, "via_pure").is_empty(),
                "a trait default over a PURE impl must stay pure (no over-fire):\n{v}");
    }

    #[test]
    fn custom_deref_resolves_pointee_method() {
        // A custom `impl Deref for W { type Target = Inner }` makes `w.method()` auto-deref to Inner's
        // method — it must reach the pointee's effect, not silently drop (the user-Deref analog of the
        // Box/Arc/Rc peel; a newtype `impl Deref` dropped `wrapper.method()` to silent-pure — corpus find).
        let run = |name: &str, src: &str| -> serde_json::Value {
            let d = std::env::temp_dir().join(format!("candor-deref-{name}-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&d);
            std::fs::create_dir_all(d.join("src")).unwrap();
            std::fs::write(d.join("Cargo.toml"), format!("[package]\nname = \"{name}\"\n")).unwrap();
            std::fs::write(d.join("src/lib.rs"), src).unwrap();
            let prefix = d.join("out/r").to_string_lossy().into_owned();
            let idx = load_dep_reports(None);
            let (rc, body) = scan_one(&d.to_string_lossy(), ScanOpts {
                prefix, want_json: true, include_tests: false, policy: None, baseline: None, ws_member: false, quiet: true, deps_idx: &idx, peek_excluded: false,
            }, &crate::gate::begin_run());
            assert_eq!(rc, 0);
            let v: serde_json::Value = serde_json::from_str(&body.unwrap()).unwrap();
            let _ = std::fs::remove_dir_all(&d);
            v
        };
        let eff = |v: &serde_json::Value, needle: &str| -> Vec<String> {
            v["functions"].as_array().into_iter().flatten()
                .filter(|f| f["fn"].as_str().is_some_and(|q| q.contains(needle)))
                .flat_map(|f| f["inferred"].as_array().into_iter().flatten()
                    .filter_map(|e| e.as_str().map(String::from)).collect::<Vec<_>>()).collect()
        };
        let src = r#"
use std::ops::Deref;
struct Inner;
impl Inner {
    fn doit(&self) { std::process::Command::new("ls").status().unwrap(); }
    fn clone(&self) -> Inner { std::fs::read("/x").unwrap(); Inner }
}
struct W { inner: Inner }
impl Deref for W { type Target = Inner; fn deref(&self) -> &Inner { &self.inner } }
impl W { pub fn act(&self) { self.doit(); } pub fn dup(&self) { let _ = self.clone(); } }
"#;
        let v = run("deref", src);
        // auto-deref through the custom Deref reaches the pointee's Exec (was silently pure)
        assert!(eff(&v, "W::act").contains(&"Exec".to_string()), "custom Deref lost the pointee Exec:\n{v}");
        // the global `.clone()` guard still holds — `w.clone()` is not attributed the pointee's effectful clone
        assert!(eff(&v, "W::dup").is_empty(), "custom-Deref clone FABRICATED the pointee's clone effect:\n{v}");
    }

    #[test]
    fn implicit_iterator_force_charges_local_iter_next_but_not_generic() {
        // A custom `impl Iterator for LocalType` whose `next()` is effectful must charge EVERY
        // implicit forcing site — a `for` loop and consuming combinators (`.collect()`/`.count()`/
        // `.fold()`/…), not just an explicit `.next()`. Controls: a PURE custom iterator stays pure,
        // and a GENERIC/opaque iterator param must NOT inherit any concrete impl's effect (the
        // review-killed RowIter fabrication must stay closed).
        let run = |name: &str, src: &str| -> serde_json::Value {
            let d = std::env::temp_dir().join(format!("candor-iternext-{name}-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&d);
            std::fs::create_dir_all(d.join("src")).unwrap();
            std::fs::write(d.join("Cargo.toml"), format!("[package]\nname = \"{name}\"\n")).unwrap();
            std::fs::write(d.join("src/lib.rs"), src).unwrap();
            let prefix = d.join("out/r").to_string_lossy().into_owned();
            let idx = load_dep_reports(None);
            let (rc, body) = scan_one(&d.to_string_lossy(), ScanOpts {
                prefix, want_json: true, include_tests: false, policy: None, baseline: None, ws_member: false, quiet: true, deps_idx: &idx, peek_excluded: false,
            }, &crate::gate::begin_run());
            assert_eq!(rc, 0);
            let v = serde_json::from_str(&body.unwrap()).unwrap();
            let _ = std::fs::remove_dir_all(&d);
            v
        };
        let eff = |v: &serde_json::Value, name: &str| -> Vec<String> {
            v["functions"].as_array().into_iter().flatten()
                .filter(|f| f["fn"].as_str() == Some(name))
                .flat_map(|f| f["inferred"].as_array().into_iter().flatten()
                    .filter_map(|e| e.as_str().map(String::from)).collect::<Vec<_>>()).collect()
        };
        let src = r#"
            struct LogTail { n: usize }
            impl Iterator for LogTail {
                type Item = String;
                fn next(&mut self) -> Option<String> {
                    std::fs::write("/t", b"x").unwrap();
                    if self.n == 0 { None } else { self.n -= 1; Some(String::new()) }
                }
            }
            fn tail(_p: &str) -> LogTail { LogTail { n: 1 } }
            pub fn count_lines(p: &str) -> usize { tail(p).count() }
            pub fn all_lines(p: &str) -> Vec<String> { tail(p).collect() }
            pub fn process(p: &str) { for _l in tail(p) {} }
            pub fn folded(p: &str) -> usize { tail(p).fold(0, |a, _| a + 1) }
            pub fn explicit(p: &str) { let mut t = tail(p); let _ = t.next(); }
            fn build() -> LogTail { LogTail { n: 1 } }
            pub fn built_consumer() -> usize { build().count() }
            // INLINE struct-literal receiver: a value constructed inline and immediately forced (a `for`
            // head requires the literal parenthesised) — the receiver types via `ctor_type` now.
            pub fn inline_for() { for _ in (LogTail { n: 1 }) {} }
            pub fn inline_collect() -> usize { (LogTail { n: 1 }).count() }

            struct PureIter { n: usize }
            impl Iterator for PureIter {
                type Item = u8;
                fn next(&mut self) -> Option<u8> { if self.n == 0 { None } else { self.n -= 1; Some(1) } }
            }
            fn pure_src() -> PureIter { PureIter { n: 1 } }
            pub fn pure_collect() -> Vec<u8> { pure_src().collect() }
            pub fn pure_for() { for _ in pure_src() {} }
            pub fn pure_inline_for() { for _ in (PureIter { n: 1 }) {} }

            struct RowIter { n: usize }
            impl Iterator for RowIter {
                type Item = u8;
                fn next(&mut self) -> Option<u8> {
                    std::fs::write("/db", b"q").unwrap();
                    if self.n == 0 { None } else { self.n -= 1; Some(0) }
                }
            }
            pub fn generic_param(it: impl Iterator<Item = u8>) { for _ in it {} }
            pub fn generic_bound<I: Iterator>(it: I) { let _ = it.count(); }
            pub fn dyn_param(it: &mut dyn Iterator<Item = u8>) { let _ = it.count(); }

            fn opaque() -> impl Iterator<Item = u8> { OpaqueSrc { n: 1 } }
            struct OpaqueSrc { n: usize }
            impl Iterator for OpaqueSrc {
                type Item = u8;
                fn next(&mut self) -> Option<u8> {
                    std::fs::write("/o", b"z").unwrap();
                    if self.n == 0 { None } else { self.n -= 1; Some(0) }
                }
            }
            pub fn opaque_consumer() -> usize { opaque().count() }
        "#;
        let v = run("iternext", src);
        // Effectful custom iterator: implicit force at every consumer carries Fs.
        for f in ["count_lines", "all_lines", "process", "folded", "explicit", "built_consumer",
                  "inline_for", "inline_collect"] {
            assert!(eff(&v, f).contains(&"Fs".to_string()),
                    "implicit iterator force under-reported: {f} should be Fs but is {:?}\n{v}", eff(&v, f));
        }
        // Control 1: a PURE custom iterator stays pure (no fabrication from forcing).
        for f in ["pure_collect", "pure_for", "pure_inline_for"] {
            assert!(eff(&v, f).is_empty(), "pure custom iterator fabricated an effect at {f}: {:?}", eff(&v, f));
        }
        // Control 2 (RowIter guard): a generic/opaque iterator param must NOT inherit a concrete
        // impl's effect, even though `impl Iterator for RowIter` does Fs.
        for f in ["generic_param", "generic_bound", "dyn_param"] {
            assert!(eff(&v, f).is_empty(),
                    "RowIter guard breached: generic iterator consumer {f} was charged {:?}", eff(&v, f));
        }
        // `-> impl Iterator` opaque return: can't resolve the concrete type → acceptable miss (pure).
        assert!(eff(&v, "opaque_consumer").is_empty(),
                "opaque-return consumer should stay pure (no concrete type): {:?}", eff(&v, "opaque_consumer"));
    }

    #[test]
    fn smart_pointer_ctor_types_the_pointee_for_method_dispatch() {
        // §4 honesty: a `let x = Arc::new(Local); x.method()` (and Box/Rc) auto-derefs to the POINTEE for
        // dispatch, but `ctor_type` typed the ctor as the impl-less wrapper ("Arc") and dropped the arg, so
        // the method call read silent-pure. `type_path` already peels a `Arc<Local>` FIELD/param; this
        // closes the local-binding form. CONTROLS: a PURE pointee method stays pure; Mutex/RefCell are NOT
        // peeled (their `.lock()`/`.borrow()` live on the wrapper), so a bare Mutex ctor stays the wrapper.
        let d = std::env::temp_dir().join(format!("candor-smartptr-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(d.join("src")).unwrap();
        std::fs::write(d.join("Cargo.toml"), "[package]\nname = \"smartptr\"\n").unwrap();
        std::fs::write(
            d.join("src/lib.rs"),
            r#"
            use std::sync::Arc;
            use std::fs;
            pub struct Db { p: String }
            impl Db {
                pub fn new(p: &str) -> Db { Db { p: p.to_string() } }
                pub fn migrate(&self) { let _ = fs::write(&self.p, "s"); }   // Fs
                pub fn touch(&self) { let _n = self.p.len(); }               // pure
            }
            pub fn via_arc() { let db = Arc::new(Db::new("/d")); db.migrate(); }        // Fs
            pub fn via_box() { let db = Box::new(Db::new("/d")); db.migrate(); }        // Fs
            pub fn via_rc()  { let db = std::rc::Rc::new(Db::new("/d")); db.migrate(); }// Fs
            pub fn inline_arc() { Arc::new(Db::new("/d")).migrate(); }                  // Fs (no let)
            pub fn pure_arc() { let db = Arc::new(Db::new("/d")); db.touch(); }         // pure pointee method
            pub fn deref_box() { let b = Box::new(Db::new("/d")); (*b).migrate(); }     // explicit *deref → Fs
            pub fn deref_ref(db: &Db) { (*db).migrate(); }                             // *&Db → Fs
            // CLONE-REBIND (R52): a `.clone()` is type-preserving, so the rebound var keeps the pointee type
            pub fn via_clone() { let db = Arc::new(Db::new("/d")); let d2 = db.clone(); d2.migrate(); } // Fs
            pub fn clone_pure() { let db = Arc::new(Db::new("/d")); let d2 = db.clone(); d2.touch(); }   // pure
            "#,
        )
        .unwrap();
        let idx = load_dep_reports(None);
        let prefix = d.join("out/r").to_string_lossy().into_owned();
        let (rc, body) = scan_one(&d.to_string_lossy(), ScanOpts {
            prefix, want_json: true, include_tests: false, policy: None, baseline: None, ws_member: false, quiet: true, deps_idx: &idx, peek_excluded: false,
        }, &crate::gate::begin_run());
        assert_eq!(rc, 0);
        let body = body.expect("want_json returns the report body");
        let v: serde_json::Value = serde_json::from_str(&body).unwrap();
        let has_fs = |needle: &str| -> bool {
            v["functions"].as_array().into_iter().flatten()
                .filter(|f| f["fn"].as_str() == Some(needle))
                .flat_map(|f| f["inferred"].as_array().into_iter().flatten().filter_map(|e| e.as_str()))
                .any(|e| e == "Fs")
        };
        for f in ["via_arc", "via_box", "via_rc", "inline_arc", "via_clone", "deref_box", "deref_ref"] {
            assert!(has_fs(f), "smart-pointer ctor pointee method must propagate to `{f}`:\n{body}");
        }
        for f in ["pure_arc", "clone_pure"] {
            assert!(!has_fs(f), "a PURE pointee method must not fabricate an effect at `{f}`:\n{body}");
        }
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn local_macro_template_is_expanded_so_its_effects_are_seen() {
        // §4 honesty (R48): a bare `NAME!(..)` to a LOCAL `macro_rules!` whose TEMPLATE does I/O (or wraps a
        // logging crate, or calls a local fn) read silent-pure — syn leaves the macro body opaque. Expanding
        // the recorded template charges its calls. Covers the pervasive local-logging-wrapper pattern
        // (`macro_rules! trace { ($($a:tt)*) => { tracing::trace!($($a)*) } }` — a silent Log miss). CONTROLS:
        // a PURE template adds nothing; a `$var`-metavar receiver resolves to no local effect (no fabrication).
        let d = std::env::temp_dir().join(format!("candor-macro-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(d.join("src")).unwrap();
        std::fs::write(d.join("Cargo.toml"), "[package]\nname = \"macroprobe\"\n").unwrap();
        std::fs::write(
            d.join("src/lib.rs"),
            r#"
            use std::fs;
            macro_rules! do_io { () => { let _ = fs::write("/x", "y"); } }          // single-arm, direct Fs
            macro_rules! log_file { ($m:expr) => { let _ = fs::write("/l", $m); } }  // single-arm metavar (in arg)
            macro_rules! logw { ($($a:tt)*) => { tracing::trace!($($a)*); } }        // logging-wrapper (single-arm)
            macro_rules! pure_mac { () => { let _x = 1 + 1; } }                      // pure single-arm
            // FABRICATION controls (review-caught):
            macro_rules! multi { () => {}; ($m:expr) => { let _ = fs::write("/m", $m); } }  // MULTI-arm, one arm Fs
            macro_rules! runc { ($x:expr) => { $x() } }                             // metavar in CALLEE position
            fn secret() { let _ = fs::write("/s", "z"); }                           // a real effectful local fn
            pub fn a() { do_io!(); }                       // Fs (single-arm direct)
            pub fn b() { log_file!("hi"); }                // Fs (metavar in arg, $-stripped)
            pub fn e() { let _x = 1; logw!("frame {}", 1); } // Log (tracing wrapper, single-arm)
            pub fn p() { pure_mac!(); }                    // pure
            pub fn m_pure() { multi!(); }                  // MUST be pure — matches the empty arm, not the Fs arm
            pub fn m_call() { runc!(secret); }             // MUST be pure — $x binds to `secret` but candor can't know
            "#,
        )
        .unwrap();
        let idx = load_dep_reports(None);
        let prefix = d.join("out/r").to_string_lossy().into_owned();
        let (rc, body) = scan_one(&d.to_string_lossy(), ScanOpts {
            prefix, want_json: true, include_tests: false, policy: None, baseline: None, ws_member: false, quiet: true, deps_idx: &idx, peek_excluded: false,
        }, &crate::gate::begin_run());
        assert_eq!(rc, 0);
        let body = body.expect("want_json returns the report body");
        let v: serde_json::Value = serde_json::from_str(&body).unwrap();
        let eff = |needle: &str| -> Vec<String> {
            v["functions"].as_array().into_iter().flatten()
                .filter(|f| f["fn"].as_str() == Some(needle))
                .flat_map(|f| f["inferred"].as_array().into_iter().flatten().filter_map(|e| e.as_str().map(String::from)))
                .collect()
        };
        for f in ["a", "b"] {
            assert!(eff(f).contains(&"Fs".to_string()), "local single-arm macro template's Fs must reach `{f}`:\n{body}");
        }
        assert!(eff("e").contains(&"Log".to_string()), "a local logging-wrapper macro's Log must reach `e`:\n{body}");
        for f in ["p", "m_pure", "m_call"] {
            assert!(eff(f).is_empty(), "macro fabrication control `{f}` must stay pure:\n{body}");
        }
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn ufcs_trait_method_never_fabricates_an_inherent_shadow() {
        // R53 (UFCS dispatch) was REVERTED after code review found a fabrication: pushing a typed `T::method`
        // edge from a UFCS `Trait::method(&t)` / `<T as Trait>::method` could resolve to T's INHERENT `method`
        // when the call runs the TRAIT method — candor keys both `impl T { fn m }` and `impl Trait for T { fn
        // m }` as `T::m`. This pins the anti-fabrication guarantee: a T that uses the trait's DEFAULT and also
        // has an inherent `go` must NOT be charged the inherent's effect through a UFCS call — only the
        // default's. CONTROLS: an ASSOCIATED fn (`Trait::assoc(&x)`) is not a receiver call, and the trait
        // default is still resolved (via the bare `Trait::method` edge) so it is not a total under-report.
        let d = std::env::temp_dir().join(format!("candor-ufcs-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(d.join("src")).unwrap();
        std::fs::write(d.join("Cargo.toml"), "[package]\nname = \"ufcs\"\n").unwrap();
        std::fs::write(
            d.join("src/lib.rs"),
            r#"
            use std::fs;
            pub trait Run { fn go(&self) { let _ = fs::write("/trait", "x"); } }   // DEFAULT go → Fs
            pub struct T;
            impl Run for T {}                                                      // uses the DEFAULT
            impl T { pub fn go(&self) { let _ = std::net::TcpStream::connect("h:1"); } } // INHERENT go → Net
            pub fn ufcs_default() { let t = T; <T as Run>::go(&t); }               // Fs (default), NOT Net
            pub fn ufcs_bare() { let t = T; Run::go(&t); }                         // Fs (default), NOT Net
            // CONTROL: an ASSOCIATED fn whose first arg is DATA, not a receiver
            pub trait Maker { fn build(cfg: &Cfg) -> Self; }
            pub struct Cfg;
            impl Cfg { pub fn build(&self) { let _ = fs::write("/c", "x"); } }
            pub struct W;
            impl Maker for W { fn build(_c: &Cfg) -> W { W } }
            pub fn assoc_control() -> W { let cfg = Cfg; Maker::build(&cfg) }      // must NOT be charged Cfg::build
            "#,
        )
        .unwrap();
        let idx = load_dep_reports(None);
        let prefix = d.join("out/r").to_string_lossy().into_owned();
        let (rc, body) = scan_one(&d.to_string_lossy(), ScanOpts {
            prefix, want_json: true, include_tests: false, policy: None, baseline: None, ws_member: false, quiet: true, deps_idx: &idx, peek_excluded: false,
        }, &crate::gate::begin_run());
        assert_eq!(rc, 0);
        let body = body.expect("want_json returns the report body");
        let v: serde_json::Value = serde_json::from_str(&body).unwrap();
        let has = |needle: &str, eff: &str| -> bool {
            v["functions"].as_array().into_iter().flatten()
                .filter(|f| f["fn"].as_str() == Some(needle))
                .flat_map(|f| f["inferred"].as_array().into_iter().flatten().filter_map(|e| e.as_str()))
                .any(|e| e == eff)
        };
        for fnn in ["ufcs_default", "ufcs_bare"] {
            assert!(!has(fnn, "Net"), "UFCS must not fabricate the inherent-shadow's Net effect at `{fnn}`:\n{body}");
        }
        assert!(!has("assoc_control", "Fs"), "an associated fn's data arg must not be charged its type's method:\n{body}");
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn implicit_coercion_edges_charge_local_effectful_impls_but_never_std() {
        // The implicit-conversion / coercion edges (cardinal sin = a fn read PURE when an effect is
        // reachable through an IMPLICIT trait-method invocation): `format!`/`.to_string()`→Display::fmt,
        // `{:?}`→Debug::fmt, `?`→From::from, operators→Add/PartialEq/Index/Neg, `*w`→Deref::deref,
        // `.into()`→From::from, and struct-literal/unit Drop-glue. Each must light up the triggering fn
        // when a LOCAL EFFECTFUL impl exists; controls (a PURE impl, a STD/primitive operand) stay pure.
        let run = |name: &str, src: &str| -> serde_json::Value {
            let d = std::env::temp_dir().join(format!("candor-coerce-{name}-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&d);
            std::fs::create_dir_all(d.join("src")).unwrap();
            std::fs::write(d.join("Cargo.toml"), format!("[package]\nname = \"{name}\"\n")).unwrap();
            std::fs::write(d.join("src/lib.rs"), src).unwrap();
            let prefix = d.join("out/r").to_string_lossy().into_owned();
            let idx = load_dep_reports(None);
            let (rc, body) = scan_one(&d.to_string_lossy(), ScanOpts {
                prefix, want_json: true, include_tests: false, policy: None, baseline: None, ws_member: false, quiet: true, deps_idx: &idx, peek_excluded: false,
            }, &crate::gate::begin_run());
            assert_eq!(rc, 0);
            let v = serde_json::from_str(&body.unwrap()).unwrap();
            let _ = std::fs::remove_dir_all(&d);
            v
        };
        let eff = |v: &serde_json::Value, name: &str| -> Vec<String> {
            v["functions"].as_array().into_iter().flatten()
                .filter(|f| f["fn"].as_str() == Some(name))
                .flat_map(|f| f["inferred"].as_array().into_iter().flatten()
                    .filter_map(|e| e.as_str().map(String::from)).collect::<Vec<_>>()).collect()
        };
        let src = r#"
            use std::fmt;
            // #2 effectful Display via format! / println! / to_string
            struct EffDisp;
            impl fmt::Display for EffDisp {
                fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
                    std::fs::write("/t", b"x").unwrap(); write!(f, "e")
                }
            }
            pub fn disp_format() -> String { let d = EffDisp; format!("{}", d) }
            pub fn disp_println() { let d = EffDisp; println!("hi {}", d); }
            pub fn disp_tostring() -> String { let d = EffDisp; d.to_string() }
            // #2 effectful Debug via {:?}
            struct EffDbg;
            impl fmt::Debug for EffDbg {
                fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
                    std::fs::write("/t", b"x").unwrap(); write!(f, "d")
                }
            }
            pub fn dbg_format() -> String { let d = EffDbg; format!("{:?}", d) }
            // control: PURE Display, and STD types (String/i32) → pure
            struct PureDisp;
            impl fmt::Display for PureDisp {
                fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result { write!(f, "p") }
            }
            pub fn disp_pure() -> String { let d = PureDisp; format!("{}", d) }
            pub fn fmt_std() -> String { let s = String::new(); let n = 3i32; format!("{} {}", s, n) }

            // #1 effectful From via ?
            struct MyErr;
            impl From<std::io::Error> for MyErr {
                fn from(_e: std::io::Error) -> MyErr { std::fs::write("/t", b"x").unwrap(); MyErr }
            }
            fn may_fail() -> Result<(), std::io::Error> { Ok(()) }
            pub fn q_eff() -> Result<(), MyErr> { may_fail()?; Ok(()) }
            // control: PURE From via ?
            struct PureErr;
            impl From<std::io::Error> for PureErr { fn from(_e: std::io::Error) -> PureErr { PureErr } }
            fn may_fail2() -> Result<(), std::io::Error> { Ok(()) }
            pub fn q_pure() -> Result<(), PureErr> { may_fail2()?; Ok(()) }

            // #4 effectful operators
            struct Acc;
            impl std::ops::Add for Acc {
                type Output = Acc;
                fn add(self, _o: Acc) -> Acc { std::fs::write("/t", b"x").unwrap(); Acc }
            }
            pub fn op_add() -> Acc { let a = Acc; let b = Acc; a + b }
            struct Cmp;
            impl PartialEq for Cmp {
                fn eq(&self, _o: &Cmp) -> bool { std::fs::write("/t", b"x").unwrap(); true }
            }
            pub fn op_eq() -> bool { let a = Cmp; let b = Cmp; a == b }
            struct Ix;
            impl std::ops::Index<usize> for Ix {
                type Output = u8;
                fn index(&self, _i: usize) -> &u8 { std::fs::write("/t", b"x").unwrap(); &0 }
            }
            pub fn op_index() -> u8 { let a = Ix; a[0] }
            struct Ng;
            impl std::ops::Neg for Ng {
                type Output = Ng;
                fn neg(self) -> Ng { std::fs::write("/t", b"x").unwrap(); Ng }
            }
            pub fn op_neg() -> Ng { let a = Ng; -a }
            // control: PURE operator, and STD arithmetic → pure
            struct PureAdd;
            impl std::ops::Add for PureAdd { type Output = PureAdd; fn add(self, _o: PureAdd) -> PureAdd { PureAdd } }
            pub fn op_pure() -> PureAdd { let a = PureAdd; let b = PureAdd; a + b }
            pub fn op_std() -> i32 { let a = 1i32; let b = 2i32; a + b }

            // #3 effectful Deref via *w
            struct Wrap;
            impl std::ops::Deref for Wrap {
                type Target = u8;
                fn deref(&self) -> &u8 { std::fs::write("/t", b"x").unwrap(); &0 }
            }
            pub fn deref_eff() -> u8 { let w = Wrap; *w }

            // #5 effectful From via .into()
            struct Tgt;
            impl From<u8> for Tgt { fn from(_v: u8) -> Tgt { std::fs::write("/t", b"x").unwrap(); Tgt } }
            pub fn into_eff() { let x: Tgt = 5u8.into(); let _ = x; }

            // #6 struct-literal & unit Drop-glue
            struct Guard { n: u8 }
            impl Drop for Guard { fn drop(&mut self) { std::fs::write("/t", b"x").unwrap(); } }
            pub fn drop_struct() { let _g = Guard { n: 1 }; }
            struct UnitGuard;
            impl Drop for UnitGuard { fn drop(&mut self) { std::fs::write("/t", b"x").unwrap(); } }
            pub fn drop_unit() { let _g = UnitGuard; }
            // control: PURE-Drop struct literal → pure
            struct PureGuard { n: u8 }
            impl Drop for PureGuard { fn drop(&mut self) {} }
            pub fn drop_pure() { let _g = PureGuard { n: 1 }; }
        "#;
        let v = run("coerce", src);
        // Every effectful coercion must carry Fs at the triggering fn.
        for f in [
            "disp_format", "disp_println", "disp_tostring", "dbg_format", "q_eff",
            "op_add", "op_eq", "op_index", "op_neg", "deref_eff", "into_eff",
            "drop_struct", "drop_unit",
        ] {
            assert!(eff(&v, f).contains(&"Fs".to_string()),
                    "coercion under-reported: {f} should be Fs but is {:?}", eff(&v, f));
        }
        // Controls: a PURE impl or a STD/primitive operand must stay pure — no fabrication, no flood.
        for f in ["disp_pure", "fmt_std", "q_pure", "op_pure", "op_std", "drop_pure"] {
            assert!(eff(&v, f).is_empty(),
                    "coercion control fabricated an effect at {f}: {:?}", eff(&v, f));
        }
    }

    /// Build a throwaway crate from `src`, scan it, and return the JSON report. Shared by the
    /// implicit-stringification tests below, which each need their OWN crate (the CHA fan-out is
    /// crate-wide, so an effectful `impl Display` anywhere in the fixture would contaminate the
    /// all-pure control).
    #[cfg(test)]
    fn scan_fixture(name: &str, src: &str) -> serde_json::Value {
        let d = std::env::temp_dir().join(format!("candor-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(d.join("src")).unwrap();
        std::fs::write(d.join("Cargo.toml"), format!("[package]\nname = \"{name}\"\n")).unwrap();
        std::fs::write(d.join("src/lib.rs"), src).unwrap();
        let prefix = d.join("out/r").to_string_lossy().into_owned();
        let idx = load_dep_reports(None);
        let (rc, body) = scan_one(&d.to_string_lossy(), ScanOpts {
            prefix, want_json: true, include_tests: false, policy: None, baseline: None, ws_member: false, quiet: true, deps_idx: &idx, peek_excluded: false,
        }, &crate::gate::begin_run());
        assert_eq!(rc, 0);
        let v: serde_json::Value = serde_json::from_str(&body.unwrap()).unwrap();
        let _ = std::fs::remove_dir_all(&d);
        v
    }

    /// SOUNDNESS R122, END TO END — the fixture the unit test above abstracts, through the real
    /// `scan_one`, with a real `[features] default = ["extra"]` so the `any` arm is genuinely on.
    ///
    /// GROUND TRUTH IS EXECUTED, not assumed: this exact crate was built and `cargo run` printed
    /// `any=true` — `prod_under_any` really spawns a process in an ordinary (non-test) build. Before
    /// the fix it was ABSENT from `functions[]` with `analyzed.count` short by one, and `deny Exec`
    /// exited 0 over it.
    ///
    /// The two controls are compiled-out-of-a-production-build items, so their ABSENCE is the correct
    /// answer rather than an artefact of an unbuildable fixture (§E3).
    #[test]
    fn r122_a_production_fn_under_any_test_feature_is_reported() {
        let d = std::env::temp_dir().join(format!("candor-r122-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(d.join("src")).unwrap();
        std::fs::write(d.join("Cargo.toml"),
            "[package]\nname = \"r122\"\n\n[features]\ndefault = [\"extra\"]\nextra = []\n").unwrap();
        std::fs::write(d.join("src/lib.rs"), r#"
            use std::process::Command;
            #[cfg(any(test, feature = "extra"))]
            pub fn prod_under_any(p: &str) -> bool { Command::new(p).status().is_ok() }
            #[cfg(any(test, unix))]
            pub fn prod_under_any_platform(p: &str) -> bool { Command::new(p).status().is_ok() }
            pub fn control_plain(p: &str) -> bool { Command::new(p).status().is_ok() }
            #[cfg(test)]
            pub fn only_in_tests(p: &str) -> bool { Command::new(p).status().is_ok() }
            #[cfg(all(test, feature = "extra"))]
            pub fn only_in_tests_all(p: &str) -> bool { Command::new(p).status().is_ok() }
        "#).unwrap();
        let idx = load_dep_reports(None);
        let run = |include_tests: bool| {
            let (rc, body) = scan_one(&d.to_string_lossy(), ScanOpts {
                prefix: d.join("out/r").to_string_lossy().into_owned(), want_json: true,
                include_tests, policy: None, baseline: None, ws_member: false, quiet: true,
                deps_idx: &idx, peek_excluded: false,
            }, &crate::gate::begin_run());
            assert_eq!(rc, 0);
            serde_json::from_str::<serde_json::Value>(&body.unwrap()).unwrap()
        };
        let v = run(false);
        for f in ["prod_under_any", "prod_under_any_platform", "control_plain"] {
            assert_eq!(fixture_effects(&v, f), vec!["Exec".to_string()],
                       "R122: `{f}` is compiled into an ordinary build and must carry Exec");
        }
        // CONTROLS — `#[cfg(test)]` and `all(test, …)` cannot exist in a non-test build, so they stay
        // out. Without these the fix would read as "scan everything", which is a different change.
        for f in ["only_in_tests", "only_in_tests_all"] {
            assert!(fixture_effects(&v, f).is_empty(),
                    "`{f}` cannot compile with test off — the default scan must not report it");
        }
        // DIRECTION CONTROL — `--include-tests` is unaffected: it reported all five before and after.
        let vt = run(true);
        for f in ["prod_under_any", "prod_under_any_platform", "control_plain",
                  "only_in_tests", "only_in_tests_all"] {
            assert_eq!(fixture_effects(&vt, f), vec!["Exec".to_string()],
                       "--include-tests must still report `{f}`");
        }
        let _ = std::fs::remove_dir_all(&d);
    }

    #[cfg(test)]
    fn fixture_effects(v: &serde_json::Value, name: &str) -> Vec<String> {
        v["functions"].as_array().into_iter().flatten()
            .filter(|f| f["fn"].as_str() == Some(name))
            .flat_map(|f| f["inferred"].as_array().into_iter().flatten()
                .filter_map(|e| e.as_str().map(String::from)).collect::<Vec<_>>())
            .collect()
    }

    /// ⟨0.29⟩ THE SCOPE IS IN THE REPORT — the missing DENOMINATOR.
    ///
    /// `analyzed.count` is a numerator; the scan's file-selection decisions produced it and appeared
    /// nowhere, so a consumer could not tell whether the answer was to the question they asked. Every
    /// exclusion is deliberate and was already documented in a comment — which is exactly why nobody
    /// measured that `deny Exec` over a crate whose `build.rs` runs `curl | sh` was GREEN, on a file that
    /// runs on every `cargo build`.
    ///
    /// Asserts the REASON STRING, not the key's presence: a block whose reasons a consumer cannot read
    /// is a count, and a count does not tell you whether the exclusion matches your question.
    #[test]
    fn the_report_declares_what_the_scan_chose_not_to_open() {
        let d = std::env::temp_dir().join(format!("candor-scope-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(d.join("src")).unwrap();
        std::fs::create_dir_all(d.join("examples")).unwrap();
        std::fs::write(d.join("Cargo.toml"), "[package]\nname = \"scope\"\n").unwrap();
        std::fs::write(d.join("src/lib.rs"), "pub fn add(a: i32) -> i32 { a + 1 }\n").unwrap();
        std::fs::write(d.join("build.rs"),
            "fn main() { std::process::Command::new(\"curl\").status().unwrap(); }\n").unwrap();
        std::fs::write(d.join("examples/e.rs"), "fn main() {}\n").unwrap();
        let idx = load_dep_reports(None);
        let (rc, body) = scan_one(&d.to_string_lossy(), ScanOpts {
            prefix: d.join("out/r").to_string_lossy().into_owned(), want_json: true,
            include_tests: false, policy: None, baseline: None, ws_member: false, quiet: true, deps_idx: &idx, peek_excluded: false,
        }, &crate::gate::begin_run());
        assert_eq!(rc, 0);
        let v: serde_json::Value = serde_json::from_str(&body.unwrap()).unwrap();
        let ex = v["excluded"].as_array().expect("`excluded` must be present, even when empty (⟨0.27⟩)");
        let find = |c: &str| ex.iter().find(|e| e["class"].as_str() == Some(c)).cloned();

        let bs = find("build-script").expect("the build script must be declared as excluded");
        assert_eq!(bs["count"].as_u64(), Some(1));
        // ⟨0.29⟩ `peeked` IS AN OUTCOME, and this scan configures NO POLICY — so no peek ran, nothing was
        // read, and `false` is the honest answer even for a class the peek is willing to read. The row
        // asserted `true` here while the flag was a per-class constant, which is precisely the overclaim
        // the flag exists to prevent: `peeked: true` beside an absent `outOfScope` reads as "I looked and
        // found nothing" about files nobody opened. The TRUE case belongs where a read happens, and is
        // asserted in the peek test below.
        assert_eq!(bs["peeked"].as_bool(), Some(false),
                   "no policy ⇒ no peek ⇒ no class may claim to have been read: {bs}");
        let why = bs["reason"].as_str().unwrap_or("");
        assert!(why.contains("COMPILE time") && why.contains("cargo build"),
                "the reason must say WHY and what it costs, not just name the class: {why}");

        let nl = find("non-library-target").expect("examples/ must be declared as excluded");
        assert_eq!(nl["count"].as_u64(), Some(1));

        // …and the build script is genuinely NOT in the analyzed set — otherwise the block would be
        // describing an exclusion that did not happen, which is a different and worse kind of wrong.
        let fns: Vec<&str> = v["functions"].as_array().into_iter().flatten()
            .filter_map(|f| f["fn"].as_str()).collect();
        assert!(!fns.iter().any(|f| f.contains("build")),
                "build.rs was scanned after all — the exclusion block would be fiction: {fns:?}");
        let _ = std::fs::remove_dir_all(&d);
    }

    /// ⟨0.29⟩ THE `only` PERMISSION FORM — `forbid` FAILS OPEN, `only` FAILS SAFE.
    ///
    /// `forbid` can state a prohibition but not a permission, so "this package is a leaf" is spelled by
    /// enumerating what it must not reach — an ALLOWLIST in the unsafe direction, because a package added
    /// tomorrow is not on the list and nothing says so. That is the hazard this project refuses everywhere
    /// in the analysis, sitting in the policy language. Under `only`, the dependency you forgot to permit
    /// is a violation on the day it appears.
    ///
    /// THE WALK STOPS AT A PERMITTED SCOPE, and the third row is what pins it. A permitted callee's own
    /// dependencies are governed by the rules about IT; descending past it would make `only` demand the
    /// transitive closure of everything you permit — the same enumeration-that-rots, one level down, which
    /// would make the form useless for the case it exists for.
    #[test]
    fn the_only_form_permits_a_list_and_fails_on_what_it_omits() {
        let d = std::env::temp_dir().join(format!("candor-only-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(d.join("src")).unwrap();
        std::fs::write(d.join("Cargo.toml"), "[package]\nname = \"lf\"\n").unwrap();
        // `model` reaches `util` (permitted) and `infra` (not). `util` itself reaches `deep` — which no
        // rule permits, and which is NOT model's business.
        std::fs::write(d.join("src/lib.rs"), concat!(
            "pub mod model {\n",
            "  pub fn shape() -> u32 { crate::util::helper() }\n",
            "  pub fn leaks() -> u32 { crate::infra::db_read() }\n",
            "}\n",
            "pub mod util { pub fn helper() -> u32 { crate::deep::inner() } }\n",
            "pub mod infra { pub fn db_read() -> u32 { 9 } }\n",
            "pub mod deep { pub fn inner() -> u32 { 1 } }\n",
        )).unwrap();
        let idx = load_dep_reports(None);
        let run = |rule: &str| -> i32 {
            let p = d.join("candor.policy");
            std::fs::write(&p, format!("{rule}\n")).unwrap();
            let (rc, _) = scan_one(&d.to_string_lossy(), ScanOpts {
                prefix: d.join("out/r").to_string_lossy().into_owned(), want_json: true,
                include_tests: false, policy: Some(p.to_string_lossy().into_owned()),
                baseline: None, ws_member: false, quiet: true, deps_idx: &idx, peek_excluded: false,
            }, &crate::gate::begin_run());
            rc
        };
        assert_eq!(run("only model -> util"), 1,
                   "`infra` is reached and not permitted — the whole point of the form");
        assert_eq!(run("only model -> util infra"), 0,
                   "the tail is a LIST: permitting both leaves the rule satisfied");
        // THE STOP RULE. `util` is permitted; `util` reaches `deep`, which nothing permits. If the walk
        // descended past a permitted scope this would fire, and `only` would require the transitive
        // closure of everything you allow — unusable for a leaf, which is the case it exists for.
        assert_eq!(run("only model -> util infra"), 0,
                   "a permitted callee's OWN dependencies are governed by the rules about IT");
        // …and the implicit self-permission: model calling model is not a crossing.
        assert_eq!(run("only model -> nosuch"), 1);
        assert_eq!(run("only deep -> nosuch"), 0,
                   "`deep` reaches nothing at all, so an empty permission list is satisfied — \
                    A -> A is implicit and `deep` has no other callee");
    }

    /// ⟨0.29⟩ THE PEEK — an effect in a file the gate did not judge is REPORTED, and changes no verdict.
    ///
    /// Three rows in one, because the bounds are the whole design and each is a way this becomes noise:
    /// `deny Exec` finds it; `deny Net` over the SAME tree says nothing (bounded by the policy); no
    /// policy at all says nothing (policy-scoped). And the exit code does not move in any of them.
    ///
    /// THE FIXTURE EXECS `ls`, NOT `curl`, AND THAT IS THE POINT. The first version used
    /// `Command::new("curl").arg("http://…")`, which the classifier reads as Net AS WELL AS Exec — so
    /// the `deny Net` row found two "Exec" findings and looked like a broken bound. The fixture could
    /// not test the thing it claimed to. An argument-free `ls` isolates Exec.
    #[test]
    fn the_peek_reports_a_denied_effect_outside_the_scope_and_the_verdict_goes_incomplete() {
        let d = std::env::temp_dir().join(format!("candor-peek-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(d.join("src")).unwrap();
        std::fs::write(d.join("Cargo.toml"), "[package]\nname = \"peek\"\n").unwrap();
        std::fs::write(d.join("src/lib.rs"), "pub fn add(a: i32) -> i32 { a + 1 }\n").unwrap();
        std::fs::write(d.join("build.rs"),
            "fn main() { std::process::Command::new(\"ls\").status().unwrap(); }\n").unwrap();
        std::fs::write(d.join("exec.pol"), "deny Exec\n").unwrap();
        std::fs::write(d.join("net.pol"), "deny Net\n").unwrap();
        let idx = load_dep_reports(None);
        // ⟨0.30⟩ tests share a process, and the gate accumulators are process statics recorded
        // unconditionally since the sink-independence fix — so an earlier test's violation would suppress
        // this one's ⟨0.30⟩ exit. `scan_main` does this per run; a direct `scan_one` caller does it here.
        crate::gate::reset_gate_run_state();
        let run = |pol: Option<&str>, tag: &str| -> (i32, serde_json::Value) {
            let (rc, body) = scan_one(&d.to_string_lossy(), ScanOpts {
                prefix: d.join(format!("out/{tag}")).to_string_lossy().into_owned(),
                want_json: true, include_tests: false,
                policy: pol.map(|p| d.join(p).to_string_lossy().into_owned()),
                baseline: None, quiet: true, ws_member: false, deps_idx: &idx, peek_excluded: false,
            }, &crate::gate::begin_run());
            (rc, serde_json::from_str(&body.unwrap()).unwrap())
        };

        let (rc, v) = run(Some("exec.pol"), "a");
        let oos = v["outOfScope"].as_array().expect("a configured policy must answer, even with []");
        assert_eq!(oos.len(), 1, "the build script's Exec must be reported: {oos:?}");
        assert_eq!(oos[0]["class"].as_str(), Some("build-script"));
        // ⟨0.29⟩ …and NOW the class may say it was read, because on this run it was.
        let ex = v["excluded"].as_array().expect("`excluded` rides every report");
        let bs = ex.iter().find(|e| e["class"].as_str() == Some("build-script")).expect("declared");
        assert_eq!(bs["peeked"].as_bool(), Some(true),
                   "the peek READ this class on this run, so the flag must say so: {bs}");
        assert_eq!(oos[0]["effects"][0].as_str(), Some("Exec"));
        assert!(oos[0]["reason"].as_str().unwrap_or("").contains("did NOT judge"),
                "the reason must say the gate did not judge it: {:?}", oos[0]["reason"]);
        // ⟨0.30⟩ THE VERDICT IS INCOMPLETE. ⟨0.29⟩ asserted the opposite here — "a file the gate declined
        // to judge must not decide an exit code" — and ⟨0.30⟩ reverses that half on the measurement that
        // the peek resolves a CONCRETE denied effect rather than uncertainty (axios: 37 functions
        // `performs Net`, exit 0, `policy ✓`).
        assert_eq!(rc, 2, "a peeked function performing the denied effect makes the verdict incomplete");
        // …and the STRUCTURAL half of ⟨0.29⟩ is UNCHANGED, which is why the code is 2 and not 1: the gate
        // did not judge this unit, so claiming a violation over it would be false in the other direction.
        assert!(v["functions"].as_array().into_iter().flatten()
                    .all(|f| f["fn"].as_str() != Some("build::main")),
                "the out-of-scope function must NOT be folded into the report's functions");

        // BOUNDED BY THE POLICY: the same tree under `deny Net` says nothing about an Exec.
        let (rc_net, v_net) = run(Some("net.pol"), "b");
        assert_eq!(v_net["outOfScope"].as_array().map(|a| a.len()), Some(0),
                   "`deny Net` must not report an Exec in an excluded file: {:?}", v_net["outOfScope"]);
        assert_eq!(rc_net, 0);

        // POLICY-SCOPED: no policy, no peek, and the key is ABSENT rather than empty — nothing was
        // asked, so an empty list would be a claim (⟨0.26⟩: absence means "cannot answer").
        let (_, v_none) = run(None, "c");
        assert!(v_none.get("outOfScope").is_none(),
                "with no policy the key must be absent, not empty: {:?}", v_none.get("outOfScope"));
        let _ = std::fs::remove_dir_all(&d);
    }

    /// THE CONTROL. A crate with nothing to exclude must still EMIT the key, as an empty list — ⟨0.27⟩'s
    /// zero-match rule, and ⟨0.26⟩'s reading that an absent key means "this producer cannot answer".
    /// Without this row the test above passes against an engine that declares exclusions it invented.
    #[test]
    fn a_crate_with_nothing_excluded_still_declares_an_empty_scope() {
        let v = scan_fixture("scope-empty", "pub fn add(a: i32) -> i32 { a + 1 }\n");
        let ex = v["excluded"].as_array().expect("the key must be emitted even with nothing to say");
        assert!(ex.is_empty(), "nothing was excluded, so the list must be empty: {ex:?}");
    }

    /// ⟨peek-scope-attribution⟩ THE CARDINAL SIN (BACKLOG "a peek finding is scope-matched against the
    /// WRONG ENTITY"): the peek names the excluded declaration correctly, but the SCOPE test ran only
    /// against that name — so a rule scoped to the IN-SCOPE CALLER that reaches it through dynamic
    /// dispatch could never match, and the exact same effect a bare `deny Net` catches went silent under
    /// `deny Net Runner` on the identical tree.
    ///
    /// `Runner::dispatch(&dyn Doer)` is in scope and resolves its ONE locally-visible `impl Doer`
    /// (`PureDoer`, pure) confidently — CHA has no reason to doubt it, because it never sees the excluded
    /// `tests/evil.rs`'s `impl Doer for EvilDoer` at all. `EvilDoer::work` performs `Net`. THREE rows:
    ///
    /// - SCOPED (`deny Net Runner`) — the defect. Before this fix: exit 0, `outOfScope: []`. Must now
    ///   name `EvilDoer::work` (never `Runner::dispatch` — attribution must not shift to the caller) and
    ///   go exit 2/incomplete.
    /// - UNSCOPED (`deny Net`) — the pre-existing control. Already caught before this fix (the excluded
    ///   declaration's own name matches a scopeless rule trivially); must still catch it, exactly once,
    ///   not twice now that TWO routes (its own name AND the reaching caller) could plausibly match.
    /// - NON-MATCHING SCOPE (`deny Net NoSuchCaller`) — the over-charge control on the SAME tree: a
    ///   scope that matches neither the excluded declaration NOR any function that reaches it must stay
    ///   exit 0, proving the widened scope test does not degrade into "any exclusion, any scope".
    #[test]
    fn peek_scope_attribution_reaches_the_dispatching_caller_and_never_double_reports() {
        let d = std::env::temp_dir().join(format!("candor-peek-scope-attr-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(d.join("src")).unwrap();
        std::fs::create_dir_all(d.join("tests")).unwrap();
        std::fs::write(d.join("Cargo.toml"), "[package]\nname = \"psa\"\n").unwrap();
        std::fs::write(d.join("src/lib.rs"), concat!(
            "pub trait Doer { fn work(&self); }\n",
            "pub struct PureDoer;\n",
            "impl Doer for PureDoer { fn work(&self) {} }\n",
            "pub struct Runner;\n",
            "impl Runner { pub fn dispatch(d: &dyn Doer) { d.work(); } }\n",
        )).unwrap();
        std::fs::write(d.join("tests/evil.rs"), concat!(
            "use psa::Doer;\n",
            "pub struct EvilDoer;\n",
            "impl Doer for EvilDoer { fn work(&self) { \
                 let _ = std::net::TcpStream::connect(\"evil.example.com:80\"); } }\n",
            "#[test]\nfn calls_it() { EvilDoer.work(); }\n",
        )).unwrap();
        std::fs::write(d.join("scoped.pol"), "deny Net Runner\n").unwrap();
        std::fs::write(d.join("unscoped.pol"), "deny Net\n").unwrap();
        std::fs::write(d.join("nomatch.pol"), "deny Net NoSuchCaller\n").unwrap();
        let idx = load_dep_reports(None);
        crate::gate::reset_gate_run_state();
        let run = |pol: &str, tag: &str| -> (i32, serde_json::Value) {
            let (rc, body) = scan_one(&d.to_string_lossy(), ScanOpts {
                prefix: d.join(format!("out/{tag}")).to_string_lossy().into_owned(),
                want_json: true, include_tests: false,
                policy: Some(d.join(pol).to_string_lossy().into_owned()),
                baseline: None, quiet: true, ws_member: false, deps_idx: &idx, peek_excluded: false,
            }, &crate::gate::begin_run());
            (rc, serde_json::from_str(&body.unwrap()).unwrap())
        };

        // THE DEFECT, closed.
        let (rc_scoped, v_scoped) = run("scoped.pol", "scoped");
        let oos = v_scoped["outOfScope"].as_array().expect("a configured policy must answer");
        assert_eq!(oos.len(), 1,
                   "exactly the excluded conformer, named once: {oos:?}");
        assert_eq!(oos[0]["fn"].as_str(), Some("tests::evil::EvilDoer::work"),
                   "ATTRIBUTION CONTROL: the finding must name the excluded declaration, never the \
                    in-scope caller it was reached through: {oos:?}");
        assert_eq!(oos[0]["effects"][0].as_str(), Some("Net"));
        assert_eq!(rc_scoped, 2,
                   "a policy scoped to the IN-SCOPE CALLER must still see an effect reached only through \
                    an excluded implementor dispatched to dynamically — this is the cardinal sin: {v_scoped:#}");

        // THE UNSCOPED CONTROL: still caught, and not doubled now that two routes could match
        // `EvilDoer::work` (its own name AND the reaching `Runner::dispatch` caller). The excluded
        // `#[test] fn calls_it()` ALSO performs Net directly (it calls `EvilDoer.work()` itself) — a
        // second, genuinely distinct excluded function, unrelated to this fix and unaffected by it.
        let (rc_unscoped, v_unscoped) = run("unscoped.pol", "unscoped");
        let oos_u = v_unscoped["outOfScope"].as_array().unwrap();
        assert_eq!(oos_u.len(), 2,
                   "two DISTINCT excluded functions genuinely perform Net (EvilDoer::work and the test \
                    fn that calls it) — the widened scope test must not ALSO add a duplicate on top: {oos_u:?}");
        let names: std::collections::BTreeSet<&str> =
            oos_u.iter().filter_map(|e| e["fn"].as_str()).collect();
        assert_eq!(names, std::collections::BTreeSet::from(["tests::evil::EvilDoer::work", "tests::evil::calls_it"]),
                   "each name exactly once — no duplicate from the two attribution routes: {oos_u:?}");
        assert_eq!(rc_unscoped, 2);

        // OVER-CHARGE CONTROL: a scope matching neither the declaration nor any reaching caller.
        let (rc_no, v_no) = run("nomatch.pol", "nomatch");
        assert_eq!(v_no["outOfScope"].as_array().map(|a| a.len()), Some(0),
                   "a scope matching nothing reachable must stay silent, not widen to \"any exclusion\": \
                    {:?}", v_no["outOfScope"]);
        assert_eq!(rc_no, 0);

        let _ = std::fs::remove_dir_all(&d);
    }

    /// ⟨peek-scope-attribution⟩ TRANSITIVE ANCESTORS. A normal (non-excluded) effect already propagates
    /// through every intermediate caller before the gate tests a scope string — `App::main` calling
    /// `Service::run` calling something that performs `Net` is caught by `deny Net App` today. This pins
    /// that the SAME thing works when the effect is reached only through an excluded dynamic-dispatch
    /// target two hops below the direct dispatcher (`Runner::dispatch`), not just at the direct dispatcher
    /// itself — `rev_calls`/`reaching_ancestors`, not merely `direct_dispatchers`. A scope matching NONE
    /// of the three in-scope names is the over-charge control on the identical tree.
    #[test]
    fn peek_scope_attribution_reaches_a_transitive_ancestor_of_the_dispatching_caller() {
        let d = std::env::temp_dir().join(format!("candor-peek-scope-attr-trans-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(d.join("src")).unwrap();
        std::fs::create_dir_all(d.join("tests")).unwrap();
        std::fs::write(d.join("Cargo.toml"), "[package]\nname = \"psat\"\n").unwrap();
        std::fs::write(d.join("src/lib.rs"), concat!(
            "pub trait Doer { fn work(&self); }\n",
            "pub struct PureDoer;\n",
            "impl Doer for PureDoer { fn work(&self) {} }\n",
            "pub struct Runner;\n",
            "impl Runner { pub fn dispatch(d: &dyn Doer) { d.work(); } }\n",
            "pub struct Service;\n",
            "impl Service { pub fn run() { Runner::dispatch(&PureDoer); } }\n",
            "pub struct App;\n",
            "impl App { pub fn main() { Service::run(); } }\n",
        )).unwrap();
        std::fs::write(d.join("tests/evil.rs"), concat!(
            "use psat::Doer;\n",
            "pub struct EvilDoer;\n",
            "impl Doer for EvilDoer { fn work(&self) { \
                 let _ = std::net::TcpStream::connect(\"evil.example.com:80\"); } }\n",
            "#[test]\nfn calls_it() { EvilDoer.work(); }\n",
        )).unwrap();
        std::fs::write(d.join("app.pol"), "deny Net App\n").unwrap();
        std::fs::write(d.join("nomatch.pol"), "deny Net NoSuchCaller\n").unwrap();
        let idx = load_dep_reports(None);
        crate::gate::reset_gate_run_state();
        let run = |pol: &str, tag: &str| -> (i32, serde_json::Value) {
            let (rc, body) = scan_one(&d.to_string_lossy(), ScanOpts {
                prefix: d.join(format!("out/{tag}")).to_string_lossy().into_owned(),
                want_json: true, include_tests: false,
                policy: Some(d.join(pol).to_string_lossy().into_owned()),
                baseline: None, quiet: true, ws_member: false, deps_idx: &idx, peek_excluded: false,
            }, &crate::gate::begin_run());
            (rc, serde_json::from_str(&body.unwrap()).unwrap())
        };

        let (rc_app, v_app) = run("app.pol", "app");
        let oos = v_app["outOfScope"].as_array().unwrap();
        assert_eq!(oos.len(), 1, "exactly the excluded conformer: {oos:?}");
        assert_eq!(oos[0]["fn"].as_str(), Some("tests::evil::EvilDoer::work"));
        assert_eq!(rc_app, 2,
                   "`App` is TWO calls away from the dyn-dispatch site (App::main -> Service::run -> \
                    Runner::dispatch); a policy scoped there must reach the excluded implementor exactly \
                    as it would if the effect were an ordinary in-scope one propagated up the call graph: \
                    {v_app:#}");

        let (rc_no, v_no) = run("nomatch.pol", "nomatch");
        assert_eq!(v_no["outOfScope"].as_array().map(|a| a.len()), Some(0));
        assert_eq!(rc_no, 0);

        let _ = std::fs::remove_dir_all(&d);
    }

    /// ⟨peek-scope-attribution⟩ OVER-CHARGE CONTROL: an excluded conformer that NOTHING in scope ever
    /// dispatches to dynamically. `Runner::dispatch` here calls the CONCRETE `PureDoer` directly (no
    /// `&dyn Doer` receiver anywhere in scope), so there is no dispatch site for `direct_dispatchers` to
    /// hold at all — the widened scope test must contribute NOTHING, and the report must be identical to
    /// what an ordinary (pre-fix) peek already produced for an unreachable excluded declaration: nothing.
    #[test]
    fn peek_scope_attribution_an_excluded_conformer_nothing_dispatches_to_is_unaffected() {
        let d = std::env::temp_dir().join(format!("candor-peek-scope-attr-unreached-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(d.join("src")).unwrap();
        std::fs::create_dir_all(d.join("tests")).unwrap();
        std::fs::write(d.join("Cargo.toml"), "[package]\nname = \"psau\"\n").unwrap();
        std::fs::write(d.join("src/lib.rs"), concat!(
            "pub trait Doer { fn work(&self); }\n",
            "pub struct PureDoer;\n",
            "impl Doer for PureDoer { fn work(&self) {} }\n",
            "pub struct Runner;\n",
            // No `&dyn Doer` anywhere — a concrete call only.
            "impl Runner { pub fn dispatch() { PureDoer.work(); } }\n",
        )).unwrap();
        std::fs::write(d.join("tests/evil.rs"), concat!(
            "use psau::Doer;\n",
            "pub struct EvilDoer;\n",
            "impl Doer for EvilDoer { fn work(&self) { \
                 let _ = std::net::TcpStream::connect(\"evil.example.com:80\"); } }\n",
            "#[test]\nfn calls_it() { EvilDoer.work(); }\n",
        )).unwrap();
        std::fs::write(d.join("scoped.pol"), "deny Net Runner\n").unwrap();
        let idx = load_dep_reports(None);
        crate::gate::reset_gate_run_state();
        let (rc, body) = scan_one(&d.to_string_lossy(), ScanOpts {
            prefix: d.join("out/unreached").to_string_lossy().into_owned(),
            want_json: true, include_tests: false,
            policy: Some(d.join("scoped.pol").to_string_lossy().into_owned()),
            baseline: None, quiet: true, ws_member: false, deps_idx: &idx, peek_excluded: false,
        }, &crate::gate::begin_run());
        let v: serde_json::Value = serde_json::from_str(&body.unwrap()).unwrap();
        assert_eq!(v["outOfScope"].as_array().map(|a| a.len()), Some(0),
                   "no dispatch site exists to widen through, so scoping must not manufacture one: {v:#}");
        assert_eq!(rc, 0);
        let _ = std::fs::remove_dir_all(&d);
    }

    /// ⟨peek-scope-attribution⟩ SECOND DISPATCH SITE: implicit STRINGIFICATION through a dispatch-typed
    /// bound (`charge_stringify_bound`, separate code path from the method-call dispatch site above) has
    /// the identical hazard and must get the identical fix — `Logger::log` never calls `.fmt()` by name;
    /// `println!("{}", e)` desugars to it. The excluded `EvilEntry`'s `Display::fmt` performing `Net`
    /// must be reachable from `deny Net Logger` exactly as the method-dispatch case is.
    #[test]
    fn peek_scope_attribution_covers_the_stringify_dispatch_site_too() {
        let d = std::env::temp_dir().join(format!("candor-peek-scope-attr-fmt-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(d.join("src")).unwrap();
        std::fs::create_dir_all(d.join("tests")).unwrap();
        std::fs::write(d.join("Cargo.toml"), "[package]\nname = \"psaf\"\n").unwrap();
        std::fs::write(d.join("src/lib.rs"), concat!(
            "use std::fmt;\n",
            "pub trait Entry: fmt::Display {}\n",
            "pub struct PureEntry;\n",
            "impl fmt::Display for PureEntry { \
                 fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result { write!(f, \"pure\") } }\n",
            "impl Entry for PureEntry {}\n",
            "pub struct Logger;\n",
            "impl Logger { pub fn log(e: &dyn Entry) { println!(\"{}\", e); } }\n",
        )).unwrap();
        std::fs::write(d.join("tests/evil.rs"), concat!(
            "use psaf::Entry;\n",
            "use std::fmt;\n",
            "pub struct EvilEntry;\n",
            "impl fmt::Display for EvilEntry {\n",
            "    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {\n",
            "        let _ = std::net::TcpStream::connect(\"evil.example.com:80\");\n",
            "        write!(f, \"evil\")\n",
            "    }\n",
            "}\n",
            "impl Entry for EvilEntry {}\n",
            "#[test]\nfn calls_it() { println!(\"{}\", &EvilEntry as &dyn fmt::Display); }\n",
        )).unwrap();
        std::fs::write(d.join("scoped.pol"), "deny Net Logger\n").unwrap();
        let idx = load_dep_reports(None);
        crate::gate::reset_gate_run_state();
        let (rc, body) = scan_one(&d.to_string_lossy(), ScanOpts {
            prefix: d.join("out/fmt").to_string_lossy().into_owned(),
            want_json: true, include_tests: false,
            policy: Some(d.join("scoped.pol").to_string_lossy().into_owned()),
            baseline: None, quiet: true, ws_member: false, deps_idx: &idx, peek_excluded: false,
        }, &crate::gate::begin_run());
        let v: serde_json::Value = serde_json::from_str(&body.unwrap()).unwrap();
        let oos = v["outOfScope"].as_array().unwrap();
        assert_eq!(oos.len(), 1, "exactly the excluded Display impl: {oos:?}");
        assert_eq!(oos[0]["fn"].as_str(), Some("tests::evil::EvilEntry::fmt"));
        assert_eq!(oos[0]["effects"][0].as_str(), Some("Net"));
        assert_eq!(rc, 2,
                   "the stringify-dispatch CHA site has the identical exclusion-blindness hazard as the \
                    method-call dispatch site, and must be fixed the same way: {v:#}");
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn implicit_stringification_through_a_bound_reaches_the_local_formatter() {
        // THE IMPLICIT-STRINGIFICATION VEIN (candor-spec/SOUNDNESS-VEIN-implicit-stringify.md) — a
        // silent under-report common to all four engines, found on HikariCP by the RQ1 runtime oracle.
        // A formatting site runs the value's `Display`/`Debug` impl; candor analysed that impl fine but
        // never edged to it from the format site, so an EFFECTFUL formatter reached through a GENERIC
        // BOUND (`T: Display`), an `impl Trait`/`dyn` param, or an INLINE CAPTURE (`{val}`) was absorbed
        // silently — the cardinal sin. Every fn below really runs `Loud::fmt` at runtime.
        let src = r#"
            use std::fmt;
            pub struct Loud;
            impl fmt::Display for Loud {
                fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
                    std::fs::write("/t", b"x").unwrap(); write!(f, "l")
                }
            }
            pub struct LoudDbg;
            impl fmt::Debug for LoudDbg {
                fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
                    std::fs::write("/t", b"x").unwrap(); write!(f, "l")
                }
            }
            // the vein's own fixture: a generic bounded by Display, formatted with `{}`
            pub fn generic_bound<T: fmt::Display>(e: T) -> String { format!("entry: {}", e) }
            pub fn where_bound<T>(e: T) -> String where T: fmt::Display { format!("entry: {}", e) }
            pub fn impl_trait(e: impl fmt::Display) -> String { format!("entry: {}", e) }
            pub fn dyn_ref(e: &dyn fmt::Display) -> String { format!("entry: {}", e) }
            pub fn debug_bound<T: fmt::Debug>(e: T) -> String { format!("{:?}", e) }
            pub fn println_bound<T: fmt::Display>(e: T) { println!("{}", e); }
            pub fn panic_bound<T: fmt::Display>(e: T) { panic!("bad: {}", e); }
            pub fn to_string_bound<T: fmt::Display>(e: T) -> String { e.to_string() }
            pub fn to_string_trait_bound<T: ToString>(e: T) -> String { e.to_string() }
            // a LOCAL trait that INHERITS Display — CHA over ITS implementors (the narrow, precise case,
            // and the shape candor-java resolves for `LOGGER.warn("{}", bagEntry)`).
            pub trait Entry: fmt::Display {}
            impl Entry for Loud {}
            pub fn supertrait_bound<T: Entry>(e: T) -> String { format!("{}", e) }
            // INLINE CAPTURE / NAMED ARG on a CONCRETE local type — the now-dominant spelling, and the
            // form that recovered a genuine `Env` miss on cargo-llvm-cov's `ProcessBuilder::run`.
            pub fn inline_capture(val: Loud) -> String { format!("v={val}") }
            pub fn inline_capture_debug(val: LoudDbg) -> String { format!("v={val:?}") }
            pub fn named_arg(x: Loud) -> String { format!("v={v}", v = x) }
            pub fn inline_capture_bound<T: fmt::Display>(val: T) -> String { format!("v={val}") }
        "#;
        let v = scan_fixture("stringify", src);
        for f in [
            "generic_bound", "where_bound", "impl_trait", "dyn_ref", "debug_bound", "println_bound",
            "panic_bound", "to_string_bound", "to_string_trait_bound", "supertrait_bound",
            "inline_capture", "inline_capture_debug", "named_arg", "inline_capture_bound",
        ] {
            assert!(fixture_effects(&v, f).contains(&"Fs".to_string()),
                    "implicit stringification under-reported (cardinal sin): {f} should be Fs but is {:?}",
                    fixture_effects(&v, f));
        }
    }

    #[test]
    fn implicit_stringification_never_fabricates_on_pure_or_std_operands() {
        // The anti-fabrication half. In a crate whose ONLY `Display` impl is PURE, no formatting site —
        // concrete, generic, or captured — may gain an effect; and a `format!` of a STRING LITERAL or a
        // primitive must resolve to nothing at all (a std operand has no LOCAL impl, so bounded CHA
        // contributes NOTHING — no edge and, deliberately, no `Unknown` flood either).
        let src = r#"
            use std::fmt;
            pub struct Quiet;
            impl fmt::Display for Quiet {
                fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result { write!(f, "q") }
            }
            pub fn pure_concrete(e: Quiet) -> String { format!("{}", e) }
            pub fn pure_bound<T: fmt::Display>(e: T) -> String { format!("{}", e) }
            pub fn pure_capture(val: Quiet) -> String { format!("v={val}") }
            pub fn literal_only() -> String { format!("{}", "a string literal") }
            pub fn primitive_only() -> String { let n = 3i32; format!("{} {:?}", n, n) }
            // a bound with NOTHING to do with formatting must not pick up the stringify route
            pub trait Store { fn save(&self); }
            pub fn unrelated_bound<T: Store>(s: T) { s.save(); }
        "#;
        let v = scan_fixture("stringifypure", src);
        for f in ["pure_concrete", "pure_bound", "pure_capture", "literal_only", "primitive_only"] {
            assert!(fixture_effects(&v, f).is_empty(),
                    "implicit stringification fabricated an effect at {f}: {:?}", fixture_effects(&v, f));
        }
    }

    #[test]
    fn implicit_stringification_discloses_unknown_when_the_fan_out_is_too_wide() {
        // Beyond the 12-impl bound the engine declines to enumerate — but it must say so. A generic
        // format site in a formatting-heavy crate reads honest `Unknown`, never silent-pure. (13 local
        // `Display` impls: one over the cross-engine bound.)
        let mut src = String::from("use std::fmt;\n");
        for i in 0..13 {
            src.push_str(&format!(
                "pub struct D{i};\nimpl fmt::Display for D{i} {{ fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {{ write!(f, \"{i}\") }} }}\n"
            ));
        }
        src.push_str("pub fn wide<T: fmt::Display>(e: T) -> String { format!(\"{}\", e) }\n");
        let v = scan_fixture("stringifywide", &src);
        assert!(fixture_effects(&v, "wide").contains(&"Unknown".to_string()),
                "a too-wide stringify dispatch must disclose Unknown, not read pure: {:?}",
                fixture_effects(&v, "wide"));
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
        td.insert("Store".into(), LocalTrait { count: 1, methods: ["save".to_string()].into_iter().collect(), supertraits: vec![] });
        td.insert("Sink".into(), LocalTrait { count: 1, methods: ["flush".to_string()].into_iter().collect(), supertraits: vec![] }); // no impl in sight
        let mut tf = TraitFieldIndex::new();
        // struct App { store: Box<dyn Store> }
        tf.entry("App".into()).or_default().insert("store".into(), vec!["Store".into()]);
        let (fe, ev, evt) = (FieldElemIndex::new(), EnumVariantIndex::new(), EnumVariantTraitIndex::new());
        let fet = FieldElemTraitIndex::new();
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
            modpath: String::new(),
                uses: &uses,
                vars,
                trait_vars,
                dyn_sig_traits: dyn_sig_trait_leaves(&sig), generic_bounds: generic_bounds_of(&sig), trait_quals: sig_trait_quals(&sig), trait_quals_by_param: sig_trait_quals_by_param(&sig),
                fields: &fields,
                trait_fields: &tf,
                trait_impls: &ti,
                local_traits: &td,
                returns: &returns,
                has_dyn_return: false,
                field_elem: &fe, field_elem_trait: &fet,
                enum_variants: &ev, enum_variant_traits: &evt, ambiguous_enum_leaves: &std::collections::HashSet::new(), callable_statics: &std::collections::HashSet::new(),
                elem_of: HashMap::new(), elem_trait_of: HashMap::new(), tuple_of: HashMap::new(), tuple_trait_of: std::collections::HashMap::new(),
                calls: Vec::new(),
                closure_vars: std::collections::HashSet::new(),
                fn_typed_vars: std::collections::HashSet::new(), dep_bound_vars: std::collections::HashMap::new(),
            fn_alias: std::collections::HashMap::new(),
            lazy_statics: empty_lazy(),
            forced_lazies: std::collections::HashSet::new(),
                unresolved: false,
                err_ret_leaf: None,
                const_strings: empty_consts(), local_macros: empty_consts(), macro_expanding: std::collections::HashSet::new(), str_locals: std::collections::HashMap::new(), local_uses: std::collections::HashMap::new(), bound_names: std::collections::HashSet::new(), dispatch_sites: Default::default(), drop_relevant: &std::collections::HashSet::new(), escaping_ctors: Default::default(), marked_ctors: Default::default(), marked_cross_ctors: Default::default(), in_pattern: false,
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
            modpath: String::new(),
                uses: &uses, vars: HashMap::new(), trait_vars: seed_trait_vars(&sig), dyn_sig_traits: dyn_sig_trait_leaves(&sig), generic_bounds: generic_bounds_of(&sig), trait_quals: sig_trait_quals(&sig), trait_quals_by_param: sig_trait_quals_by_param(&sig),
                fields: &fields, trait_fields: &tf, trait_impls: &ti2, local_traits: &td,
                returns: &returns, has_dyn_return: false, field_elem: &fe, field_elem_trait: &fet, enum_variants: &ev, enum_variant_traits: &evt, ambiguous_enum_leaves: &std::collections::HashSet::new(), callable_statics: &std::collections::HashSet::new(), elem_of: HashMap::new(), elem_trait_of: HashMap::new(), tuple_of: HashMap::new(), tuple_trait_of: std::collections::HashMap::new(),
                calls: Vec::new(),
                closure_vars: std::collections::HashSet::new(), fn_typed_vars: std::collections::HashSet::new(), dep_bound_vars: std::collections::HashMap::new(), fn_alias: std::collections::HashMap::new(), lazy_statics: empty_lazy(), forced_lazies: std::collections::HashSet::new(), unresolved: false, err_ret_leaf: None, const_strings: empty_consts(), local_macros: empty_consts(), macro_expanding: std::collections::HashSet::new(), str_locals: std::collections::HashMap::new(), local_uses: std::collections::HashMap::new(), bound_names: std::collections::HashSet::new(), dispatch_sites: Default::default(), drop_relevant: &std::collections::HashSet::new(), escaping_ctors: Default::default(), marked_ctors: Default::default(), marked_cross_ctors: Default::default(), in_pattern: false,
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
            modpath: String::new(),
                    uses: &uses, vars: HashMap::new(), trait_vars: seed_trait_vars(&sig), dyn_sig_traits: dyn_sig_trait_leaves(&sig), generic_bounds: generic_bounds_of(&sig), trait_quals: sig_trait_quals(&sig), trait_quals_by_param: sig_trait_quals_by_param(&sig),
                    fields: &fields, trait_fields: &tf, trait_impls: &ti2, local_traits: &td,
                    returns: &returns, has_dyn_return: false, field_elem: &fe, field_elem_trait: &fet, enum_variants: &ev, enum_variant_traits: &evt, ambiguous_enum_leaves: &std::collections::HashSet::new(), callable_statics: &std::collections::HashSet::new(), elem_of: HashMap::new(), elem_trait_of: HashMap::new(), tuple_of: HashMap::new(), tuple_trait_of: std::collections::HashMap::new(),
                    calls: Vec::new(),
                    closure_vars: std::collections::HashSet::new(), fn_typed_vars: std::collections::HashSet::new(), dep_bound_vars: std::collections::HashMap::new(), fn_alias: std::collections::HashMap::new(), lazy_statics: empty_lazy(), forced_lazies: std::collections::HashSet::new(), unresolved: false, err_ret_leaf: None, const_strings: empty_consts(), local_macros: empty_consts(), macro_expanding: std::collections::HashSet::new(), str_locals: std::collections::HashMap::new(), local_uses: std::collections::HashMap::new(), bound_names: std::collections::HashSet::new(), dispatch_sites: Default::default(), drop_relevant: &std::collections::HashSet::new(), escaping_ctors: Default::default(), marked_ctors: Default::default(), marked_cross_ctors: Default::default(), in_pattern: false,
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
        let (fe, ev, evt) = (FieldElemIndex::new(), EnumVariantIndex::new(), EnumVariantTraitIndex::new());
        let fet = FieldElemTraitIndex::new();
        let block: syn::Block =
            syn::parse_str("{ let p = create_pool()?; p.fetch_one(q); }").unwrap();
        let mut c = CallCollector {
            modpath: String::new(),
            uses: &uses,
            vars: HashMap::new(),
            trait_vars: HashMap::new(),
            dyn_sig_traits: Default::default(), generic_bounds: Default::default(), trait_quals: Default::default(), trait_quals_by_param: Default::default(),
            fields: &fields,
            trait_fields: &tf,
            trait_impls: &ti,
            local_traits: &td,
            returns: &returns,
            has_dyn_return: false,
            field_elem: &fe, field_elem_trait: &fet,
            enum_variants: &ev, enum_variant_traits: &evt, ambiguous_enum_leaves: &std::collections::HashSet::new(), callable_statics: &std::collections::HashSet::new(),
            elem_of: HashMap::new(), elem_trait_of: HashMap::new(), tuple_of: HashMap::new(), tuple_trait_of: std::collections::HashMap::new(),
            calls: Vec::new(),
            closure_vars: std::collections::HashSet::new(),
            fn_typed_vars: std::collections::HashSet::new(), dep_bound_vars: std::collections::HashMap::new(),
            fn_alias: std::collections::HashMap::new(),
            lazy_statics: empty_lazy(),
            forced_lazies: std::collections::HashSet::new(),
            unresolved: false,
            err_ret_leaf: None,
            const_strings: empty_consts(), local_macros: empty_consts(), macro_expanding: std::collections::HashSet::new(), str_locals: std::collections::HashMap::new(), local_uses: std::collections::HashMap::new(), bound_names: std::collections::HashSet::new(), dispatch_sites: Default::default(), drop_relevant: &std::collections::HashSet::new(), escaping_ctors: Default::default(), marked_ctors: Default::default(), marked_cross_ctors: Default::default(), in_pattern: false,
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
            modpath: String::new(),
                uses: &uses,
                vars: HashMap::new(),
                trait_vars: HashMap::new(),
                dyn_sig_traits: Default::default(), generic_bounds: Default::default(), trait_quals: Default::default(), trait_quals_by_param: Default::default(),
                fields: &fields,
                trait_fields: &tf,
                trait_impls: &ti,
                local_traits: &td,
                returns: &returns,
                has_dyn_return: false,
                field_elem: &fe, field_elem_trait: &fet,
                enum_variants: &ev, enum_variant_traits: &evt, ambiguous_enum_leaves: &std::collections::HashSet::new(), callable_statics: &std::collections::HashSet::new(),
                elem_of: HashMap::new(), elem_trait_of: HashMap::new(), tuple_of: HashMap::new(), tuple_trait_of: std::collections::HashMap::new(),
                calls: Vec::new(),
                closure_vars: std::collections::HashSet::new(),
                fn_typed_vars: std::collections::HashSet::new(), dep_bound_vars: std::collections::HashMap::new(),
            fn_alias: std::collections::HashMap::new(),
            lazy_statics: empty_lazy(),
            forced_lazies: std::collections::HashSet::new(),
                unresolved: false,
                err_ret_leaf: None,
                const_strings: empty_consts(), local_macros: empty_consts(), macro_expanding: std::collections::HashSet::new(), str_locals: std::collections::HashMap::new(), local_uses: std::collections::HashMap::new(), bound_names: std::collections::HashSet::new(), dispatch_sites: Default::default(), drop_relevant: &std::collections::HashSet::new(), escaping_ctors: Default::default(), marked_ctors: Default::default(), marked_cross_ctors: Default::default(), in_pattern: false,
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
        let empty_inc: HashMap<String, BTreeSet<&'static str>> = HashMap::new();
        // deny fires on the transitive set; allow flags the out-of-list host; forbid sees ui -> db.
        let mut tables: HashMap<String, BTreeSet<String>> = HashMap::new();
        tables.insert("db::run".into(), ["audit.log".to_string()].into_iter().collect());
        // deny fires on the transitive set; allow flags the out-of-list host; forbid sees ui -> db.
        let v = gate_violations(
            "deny Net api\nallow Net in api good.example.com\nforbid ui -> db\n",
            &all, &inferred, &calls, &hosts, &empty, &empty, &tables, &empty_inc, &empty, &std::collections::BTreeMap::new(), &std::collections::BTreeSet::new(),
        );
        assert_eq!(v.len(), 3, "{}", v.iter().map(|x| x.detail.clone()).collect::<Vec<_>>().join(" | "));
        // 006 names the denied effect in `effects` (the denied SET, not just the message text).
        assert!(v.iter().any(|g| g.rule == "AS-EFF-006" && g.func == "api::handle" && g.effects == ["Net"]));
        assert!(v.iter().any(|g| g.rule == "AS-EFF-008" && g.detail.contains("evil.example.com") && g.effects == ["Net"]));
        // 009 is a layer-flow — no single effect, so `effects` is empty.
        assert!(v.iter().any(|g| g.rule == "AS-EFF-009" && g.func == "ui::draw" && g.effects.is_empty()));
        // clean policy -> no violations; `pure` flags ANY effect incl. the Db fn.
        assert!(gate_violations("deny Exec\n", &all, &inferred, &calls, &hosts, &empty, &empty, &tables, &empty_inc, &empty, &std::collections::BTreeMap::new(), &std::collections::BTreeSet::new()).is_empty());
        assert_eq!(gate_violations("pure db\n", &all, &inferred, &calls, &hosts, &empty, &empty, &tables, &empty_inc, &empty, &std::collections::BTreeMap::new(), &std::collections::BTreeSet::new()).len(), 1);
        // the Db table allowlist: db::run reaches audit.log — outside `ledger.*` -> violation;
        // covered by `audit.*` -> clean. ui::draw INHERITS Db but the literal propagation is the
        // caller's tablesacc, supplied here only for db::run, so only db::run flags.
        let bad = gate_violations("allow Db in db ledger.*\n", &all, &inferred, &calls, &hosts, &empty, &empty, &tables, &empty_inc, &empty, &std::collections::BTreeMap::new(), &std::collections::BTreeSet::new());
        assert_eq!(bad.len(), 1, "{}", bad.iter().map(|x| x.detail.clone()).collect::<Vec<_>>().join(" | "));
        assert!(bad[0].detail.contains("audit.log"));
        assert!(gate_violations("allow Db in db audit.*\n", &all, &inferred, &calls, &hosts, &empty, &empty, &tables, &empty_inc, &empty, &std::collections::BTreeMap::new(), &std::collections::BTreeSet::new()).is_empty());
    }

    #[test]
    fn reason_scoped_unknown_gate_fires_on_match_tolerates_mismatch() {
        // A fn whose only effect is Unknown, classified `native` (rust's `native:extern fn` reason).
        let all = vec!["dom::svc".to_string()];
        let mut inferred: HashMap<String, BTreeSet<&'static str>> = HashMap::new();
        inferred.insert("dom::svc".into(), ["Unknown"].into_iter().collect());
        let calls: HashMap<String, BTreeSet<String>> = HashMap::new();
        let empty: HashMap<String, BTreeSet<String>> = HashMap::new();
        let empty_inc: HashMap<String, BTreeSet<&'static str>> = HashMap::new();
        let mut rc: HashMap<String, BTreeSet<String>> = HashMap::new();
        rc.insert("dom::svc".into(), ["native".to_string()].into_iter().collect());
        let gate = |pol: &str, rc: &HashMap<String, BTreeSet<String>>| {
            gate_violations(pol, &all, &inferred, &calls, &empty, &empty, &empty, &empty, &empty_inc, rc, &std::collections::BTreeMap::new(), &std::collections::BTreeSet::new())
        };
        // matching class → fires
        assert_eq!(gate("deny Net Unknown[native]\n", &rc).len(), 1, "Unknown[native] must fire on a native-class Unknown");
        // non-matching class → tolerated
        assert!(gate("deny Net Unknown[reflect]\n", &rc).is_empty(), "Unknown[reflect] must tolerate a native-class Unknown");
        // bare Unknown → fires regardless of class
        assert_eq!(gate("deny Net Unknown\n", &rc).len(), 1, "bare deny Unknown fires on any Unknown");
        // ⟨0.24⟩ AN `Unknown` WITH NO RECORDED REASON CLASS IS **WITHHELD**, NOT CHARGED — SPEC §3.1.
        //
        // THIS ASSERTION USED TO READ `…len() == 1, "no reason class ⇒ unresolved"`, and it was pinning
        // the FABRICATION. `reason_class_matches` floors an absent/empty class set at `unresolved` — the
        // right fail-closed default for a MATCHER ("could this rule apply?") and the wrong basis for a
        // FIRING ("did it?"): read as grounds to emit a violation it asserts a reason nobody recorded.
        // Harmless while the report route's refusal short-circuited before `gate()` ran; a live
        // fabrication the moment `8b97e5c` (correctly) removed that short-circuit.
        //
        // Withheld is not tolerated either — the pair rides out so the caller refuses (exit 2) rather
        // than printing `policy ✓` over a rule that never ran.
        let none: HashMap<String, BTreeSet<String>> = HashMap::new();
        let full = |pol: &str, rc: &HashMap<String, BTreeSet<String>>| {
            policy_violations(pol, "unit", &all, &inferred, &calls, &empty, &empty, &empty, &empty, &empty_inc, rc, &std::collections::BTreeMap::new(), &std::collections::BTreeSet::new())
        };
        let o = full("deny Net Unknown[unresolved]\n", &none);
        assert!(o.violations.is_empty(), "an UNEVIDENCED reason class must not be CHARGED: {:?}", o.violations);
        assert_eq!(o.withheld.len(), 1, "…it must be WITHHELD, and the pair must travel: {o:?}");
        assert_eq!(o.withheld[0].func, "dom::svc");
        assert_eq!(o.withheld[0].filter, "Unknown");
        let o = full("deny Net Unknown[reflect]\n", &none);
        assert!(o.violations.is_empty(), "no reason class must NOT match a specific class");
        assert_eq!(o.withheld.len(), 1, "…and it is withheld under EVERY narrowed filter, not tolerated under some");
        // THE MIRROR, in the same test so it cannot be deleted separately: the class set is EVIDENCED
        // here, so the identical rule still fires. Withholding must cost nothing on a signature that
        // carries its classes.
        assert_eq!(gate("deny Net Unknown[unresolved]\n", &{
            let mut m: HashMap<String, BTreeSet<String>> = HashMap::new();
            m.insert("dom::svc".into(), ["unresolved".to_string()].into_iter().collect());
            m
        }).len(), 1, "an EVIDENCED `unresolved` must still fire — this is the under-report mirror");
        // …and the bare `deny Unknown` is never narrowed, so it is never withheld: the escape hatch the
        // refusal message recommends has to actually work.
        let bare = full("deny Net Unknown\n", &none);
        assert_eq!(bare.violations.len(), 1, "bare deny Unknown fires with no class set at all");
        assert!(bare.withheld.is_empty(), "a rule that does not narrow has nothing to withhold");
    }

    #[test]
    fn reason_class_propagates_transitively_to_callers() {
        // The scale case: a caller inheriting Unknown from a native-caused callee is a native-class Unknown,
        // even though the `native:` reason lives on the callee. propagate_str carries the class up (mirrors
        // the java gate's reasonClassAcc); regression for the transitive-reason under-gating gap.
        let all = vec!["dom::caller".to_string(), "dom::callee".to_string()];
        let mut inferred: HashMap<String, BTreeSet<&'static str>> = HashMap::new();
        inferred.insert("dom::caller".into(), ["Unknown"].into_iter().collect());
        inferred.insert("dom::callee".into(), ["Unknown"].into_iter().collect());
        let mut calls: HashMap<String, BTreeSet<String>> = HashMap::new();
        calls.insert("dom::caller".into(), ["dom::callee".to_string()].into_iter().collect());
        // only the callee carries the direct reason; propagate_str lifts the class to the caller.
        let mut rc_direct: HashMap<String, BTreeSet<String>> = HashMap::new();
        rc_direct.insert("dom::callee".into(), ["native".to_string()].into_iter().collect());
        let rc_acc = crate::propagate::propagate_str(&rc_direct, &calls, &all);
        let empty: HashMap<String, BTreeSet<String>> = HashMap::new();
        let empty_inc: HashMap<String, BTreeSet<&'static str>> = HashMap::new();
        let v = gate_violations("deny Net Unknown[native]\n", &all, &inferred, &calls, &empty, &empty, &empty, &empty, &empty_inc, &rc_acc, &std::collections::BTreeMap::new(), &std::collections::BTreeSet::new());
        assert_eq!(v.len(), 2, "Unknown[native] must fire on BOTH the native callee and the caller inheriting its Unknown");
        // §6.2 ⟨0.19⟩: the verdict carries reasonClass on the Unknown denial — on the caller too (transitive).
        for gv in &v {
            assert_eq!(gv.reason_class, vec!["native".to_string()], "reasonClass rides the Unknown verdict for `{}`", gv.func);
        }
    }

    #[test]
    fn net_destination_class_gate_fires_on_unknown_host_tolerates_asserted_safe() {
        // The security gate (NET-DESTINATION-CLASS-DESIGN.md): `deny Net[unknown-host]` denies Net to a host
        // candor can't identify as telemetry/partner, tolerating the asserted-safe classes. Fail-closed on a
        // masked surface / a Net with no visible host. The destination class travels the call graph.
        let all = vec![
            "d::tel".to_string(),
            "d::exfil".to_string(),
            "d::runtime".to_string(),
            "d::caller".to_string(),
        ];
        let mut inferred: HashMap<String, BTreeSet<&'static str>> = HashMap::new();
        for f in &all {
            inferred.insert(f.clone(), ["Net"].into_iter().collect());
        }
        let mut calls: HashMap<String, BTreeSet<String>> = HashMap::new();
        calls.insert("d::caller".into(), ["d::exfil".to_string()].into_iter().collect()); // caller reaches exfil
        let mut hosts: HashMap<String, BTreeSet<String>> = HashMap::new();
        hosts.insert("d::tel".into(), ["sentry.io".to_string()].into_iter().collect());
        hosts.insert("d::exfil".into(), ["evil.example.com".to_string()].into_iter().collect());
        // d::runtime has Net but NO visible host (a runtime-computed endpoint) → fail closed to unknown-host.
        let hostsacc = crate::propagate::propagate_str(&hosts, &calls, &all);
        let empty: HashMap<String, BTreeSet<String>> = HashMap::new();
        let empty_inc: HashMap<String, BTreeSet<&'static str>> = HashMap::new();
        let empty_rc: HashMap<String, BTreeSet<String>> = HashMap::new();
        let partners: BTreeSet<String> = ["api.stripe.com".to_string()].into_iter().collect();
        let v = gate_violations(
            "deny Net[unknown-host]\n", &all, &inferred, &calls, &hostsacc, &empty, &empty, &empty,
            &empty_inc, &empty_rc, &std::collections::BTreeMap::new(), &partners,
        );
        let flagged: BTreeSet<&str> = v.iter().map(|g| g.func.as_str()).collect();
        // exfil + runtime + the caller reaching exfil fire; the telemetry host is tolerated.
        assert_eq!(flagged, ["d::exfil", "d::runtime", "d::caller"].into_iter().collect());
        // the verdict carries the fn's destination classes.
        let exfil = v.iter().find(|g| g.func == "d::exfil").unwrap();
        assert_eq!(exfil.net_class, vec!["unknown-host".to_string()]);
        // a config-declared partner is tolerated; bare `deny Net` still denies ALL destinations.
        let mut phosts = hosts.clone();
        phosts.insert("d::partner".into(), ["api.stripe.com".to_string()].into_iter().collect());
        let pall: Vec<String> = all.iter().cloned().chain(["d::partner".to_string()]).collect();
        let mut pinf = inferred.clone();
        pinf.insert("d::partner".into(), ["Net"].into_iter().collect());
        let pacc = crate::propagate::propagate_str(&phosts, &calls, &pall);
        let pv = gate_violations(
            "deny Net[unknown-host]\n", &pall, &pinf, &calls, &pacc, &empty, &empty, &empty,
            &empty_inc, &empty_rc, &std::collections::BTreeMap::new(), &partners,
        );
        assert!(!pv.iter().any(|g| g.func == "d::partner"), "a config net-partner is tolerated");
        let bare = gate_violations(
            "deny Net\n", &pall, &pinf, &calls, &pacc, &empty, &empty, &empty,
            &empty_inc, &empty_rc, &std::collections::BTreeMap::new(), &partners,
        );
        assert_eq!(bare.len(), pall.len(), "bare deny Net denies every Net fn (backward-compat)");
    }

    #[test]
    fn masking_incomplete_net_surface_not_certified() {
        // The masking evasion: a fn with a captured BENIGN host AND a structurally-INVISIBLE Net reach
        // (an incomplete surface) must NOT be certified by the benign host. A clean fn (host captured,
        // surface complete) certifies. Mirrors candor-java 0.5.29.
        let all = vec!["a::mask".to_string(), "a::clean".to_string()];
        let mut inferred: HashMap<String, BTreeSet<&'static str>> = HashMap::new();
        inferred.insert("a::mask".into(), ["Net"].into_iter().collect());
        inferred.insert("a::clean".into(), ["Net"].into_iter().collect());
        let calls: HashMap<String, BTreeSet<String>> = HashMap::new();
        let mut hosts: HashMap<String, BTreeSet<String>> = HashMap::new();
        hosts.insert("a::mask".into(), ["api.stripe.com".to_string()].into_iter().collect());
        hosts.insert("a::clean".into(), ["api.stripe.com".to_string()].into_iter().collect());
        let empty: HashMap<String, BTreeSet<String>> = HashMap::new();
        let tables: HashMap<String, BTreeSet<String>> = HashMap::new();
        let mut inc: HashMap<String, BTreeSet<&'static str>> = HashMap::new();
        inc.insert("a::mask".into(), ["Net"].into_iter().collect()); // mask also has an invisible reach
        let v = gate_violations(
            "allow Net api.stripe.com\n",
            &all, &inferred, &calls, &hosts, &empty, &empty, &tables, &inc, &empty, &std::collections::BTreeMap::new(), &std::collections::BTreeSet::new(),
        );
        assert!(v.iter().any(|g| g.func == "a::mask" && g.detail.contains("cannot be certified")), "{:?}", v.iter().map(|x| x.detail.clone()).collect::<Vec<_>>());
        assert!(!v.iter().any(|g| g.func == "a::clean"), "clean must certify: {:?}", v.iter().map(|x| x.detail.clone()).collect::<Vec<_>>());
    }

    #[test]
    fn masking_fs_path_and_db_table_gate_fails_closed() {
        // End-to-end (scan_one + a CANDOR_POLICY file): a MASKED Fs path / Db table reached ALONGSIDE a
        // benign ALLOWED literal must FAIL the allowlist gate (exit 1) — the benign sibling must not mask
        // the runtime-built endpoint. A single compliant literal still PASSES (no false positive). A
        // fully-masked program (no benign sibling) still fails. The gate evasion this closes:
        // `inferred=[Fs] paths=[/var/app/x]` with no `incomplete` certified `allow Fs /var/app` while a
        // sibling `fs::write(format!("/etc/{}","passwd"), …)` hit /etc/passwd at runtime.
        let run = |name: &str, src: &str, policy: &str| -> i32 {
            let d = std::env::temp_dir().join(format!("candor-mask-{name}-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&d);
            std::fs::create_dir_all(d.join("src")).unwrap();
            std::fs::write(d.join("Cargo.toml"), format!("[package]\nname = \"{name}\"\n")).unwrap();
            std::fs::write(d.join("src/lib.rs"), src).unwrap();
            let pp = d.join("candor.policy");
            std::fs::write(&pp, policy).unwrap();
            let prefix = d.join("out/r").to_string_lossy().into_owned();
            let idx = load_dep_reports(None);
            let (rc, _) = scan_one(&d.to_string_lossy(), ScanOpts {
                prefix, want_json: true, include_tests: false,
                policy: Some(pp.to_string_lossy().into_owned()), baseline: None, ws_member: false, quiet: true, deps_idx: &idx, peek_excluded: false,
            }, &crate::gate::begin_run());
            let _ = std::fs::remove_dir_all(&d);
            rc
        };

        // Fs: a benign allowed write + a MASKED (runtime-path) write → gate FAILS (Fs incomplete).
        let fs_mix = r#"
            use std::fs;
            pub fn go() {
                let _ = fs::write("/var/app/x", b"x");
                let p = format!("/etc/{}", "passwd");
                let _ = fs::write(p, b"x");
            }
        "#;
        assert_eq!(run("fsmix", fs_mix, "allow Fs /var/app\n"), 1, "masked Fs path must fail the gate");

        // Fs: a single compliant literal (no masking) → PASSES.
        let fs_ok = r#"
            use std::fs;
            pub fn go() { let _ = fs::write("/var/app/x", b"x"); }
        "#;
        assert_eq!(run("fsok", fs_ok, "allow Fs /var/app\n"), 0, "compliant Fs path must pass (no false positive)");

        // Fs: fully masked (no benign sibling) → still fails (unchanged behaviour).
        let fs_masked = r#"
            use std::fs;
            pub fn go() { let p = format!("/etc/{}", "passwd"); let _ = fs::write(p, b"x"); }
        "#;
        assert_eq!(run("fsmask", fs_masked, "allow Fs /var/app\n"), 1, "fully-masked Fs path must fail");

        // Db: a benign allowed query + a MASKED (runtime-query) execute → gate FAILS (Db incomplete).
        let db_mix = r#"
            pub fn go(con: &rusqlite::Connection) {
                let _ = con.execute("SELECT id FROM customers", []);
                let q = format!("DELETE FROM {}", "secrets");
                let _ = con.execute(&q, []);
            }
        "#;
        assert_eq!(run("dbmix", db_mix, "allow Db customers\n"), 1, "masked Db table must fail the gate");

        // Db: a single compliant query (no masking) → PASSES.
        let db_ok = r#"
            pub fn go(con: &rusqlite::Connection) {
                let _ = con.execute("SELECT id FROM customers", []);
            }
        "#;
        assert_eq!(run("dbok", db_ok, "allow Db customers\n"), 0, "compliant Db table must pass (no false positive)");
    }

    #[test]
    fn gate_over_unparseable_source_fails_closed() {
        // SOUNDNESS: a policy gate over a crate where a source file failed to PARSE must NOT report
        // green — the unparsed file's effects are absent from the report, so a `policy ✓` over it is a
        // false-pure. Exit 2 (mirroring the unreadable-policy posture), never 0. A clean-parsing crate
        // under the same policy still passes 0 (the failure signal is specific, not a blanket fail).
        let run = |name: &str, src: &str, with_policy: bool| -> i32 {
            let d = std::env::temp_dir().join(format!("candor-parsefail-{name}-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&d);
            std::fs::create_dir_all(d.join("src")).unwrap();
            std::fs::write(d.join("Cargo.toml"), format!("[package]\nname = \"{name}\"\n")).unwrap();
            std::fs::write(d.join("src/lib.rs"), src).unwrap();
            let pp = d.join("candor.policy");
            std::fs::write(&pp, "deny Exec\n").unwrap();
            let prefix = d.join("out/r").to_string_lossy().into_owned();
            let idx = load_dep_reports(None);
            let (rc, _) = scan_one(&d.to_string_lossy(), ScanOpts {
                prefix, want_json: true, include_tests: false,
                policy: if with_policy { Some(pp.to_string_lossy().into_owned()) } else { None },
                baseline: None, quiet: true, ws_member: false, deps_idx: &idx, peek_excluded: false,
            }, &crate::gate::begin_run());
            let _ = std::fs::remove_dir_all(&d);
            rc
        };
        // A file that does NOT parse (a stray token) under a configured gate → exit 2, never green.
        let broken = "pub fn ok() {}\nthis is not valid rust @@@\n";
        assert_eq!(run("broken", broken, true), 2,
                   "a configured gate over an unparseable source file must FAIL exit 2, never green");
        // No policy configured → no gate verdict to corrupt; the parse-failure is disclosed (stderr) only.
        assert_eq!(run("brokennopol", broken, false), 0,
                   "without a policy there is no gate; a parse failure is disclosed, not an exit code");
        // A clean-parsing crate under the same gate still passes (the failure signal is specific).
        assert_eq!(run("clean", "pub fn ok() {}\n", true), 0,
                   "a clean-parsing crate must still pass the gate (no blanket fail)");
    }

    #[test]
    fn call_returning_a_callable_in_an_unannotated_local_reads_unknown() {
        // §4 HONESTY: `let g = make_cb(); g()` where `make_cb` returns a CALLABLE must read the opaque
        // callback (Unknown), not silent-pure / a phantom free-fn `g`. Covers all three callable return
        // shapes (`fn()`, `impl Fn`, `Box<dyn Fn>`). A NON-callable factory's binding stays pure.
        let run = |name: &str, src: &str| -> serde_json::Value {
            let d = std::env::temp_dir().join(format!("candor-retcb-{name}-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&d);
            std::fs::create_dir_all(d.join("src")).unwrap();
            std::fs::write(d.join("Cargo.toml"), format!("[package]\nname = \"{name}\"\n")).unwrap();
            std::fs::write(d.join("src/lib.rs"), src).unwrap();
            let prefix = d.join("out/r").to_string_lossy().into_owned();
            let idx = load_dep_reports(None);
            let (rc, body) = scan_one(&d.to_string_lossy(), ScanOpts {
                prefix, want_json: true, include_tests: false, policy: None, baseline: None, ws_member: false, quiet: true, deps_idx: &idx, peek_excluded: false,
            }, &crate::gate::begin_run());
            assert_eq!(rc, 0);
            let v: serde_json::Value = serde_json::from_str(&body.unwrap()).unwrap();
            let _ = std::fs::remove_dir_all(&d);
            v
        };
        let eff = |v: &serde_json::Value, needle: &str| -> Vec<String> {
            v["functions"].as_array().into_iter().flatten()
                .filter(|f| f["fn"].as_str().is_some_and(|q| q.contains(needle)))
                .flat_map(|f| f["inferred"].as_array().into_iter().flatten()
                    .filter_map(|e| e.as_str().map(String::from)).collect::<Vec<_>>()).collect()
        };
        // `Box<dyn Fn>` return: the binding is fn-typed → `g()` is Unknown.
        let boxed = r#"
            pub fn make_cb() -> Box<dyn Fn()> { Box::new(|| {}) }
            pub fn uses() { let g = make_cb(); g(); }
        "#;
        assert!(eff(&run("retbox", boxed), "uses").contains(&"Unknown".to_string()),
                "a call returning Box<dyn Fn> bound to a local must read Unknown at the call site");
        // bare `fn()` return.
        let bare = r#"
            fn h() {}
            pub fn make_cb() -> fn() { h }
            pub fn uses() { let g = make_cb(); g(); }
        "#;
        assert!(eff(&run("retbare", bare), "uses").contains(&"Unknown".to_string()),
                "a call returning fn() bound to a local must read Unknown at the call site");
        // `impl Fn` return.
        let impl_fn = r#"
            pub fn make_cb() -> impl Fn() { || {} }
            pub fn uses() { let g = make_cb(); g(); }
        "#;
        assert!(eff(&run("retimpl", impl_fn), "uses").contains(&"Unknown".to_string()),
                "a call returning impl Fn bound to a local must read Unknown at the call site");
        // CONTROL: a non-callable factory's binding is NOT fn-typed → no fabricated Unknown.
        let plain = r#"
            pub fn make_v() -> i32 { 42 }
            pub fn uses() { let v = make_v(); let _ = v; }
        "#;
        assert!(!eff(&run("retplain", plain), "uses").contains(&"Unknown".to_string()),
                "a non-callable factory binding must stay pure (no fabricated Unknown)");
    }

    #[test]
    fn ambiguous_same_name_local_bare_leaf_reads_unknown() {
        // §4 HONESTY: a BARE leaf naming TWO-OR-MORE local defs (a free fn + a same-named method, here
        // `process`) defeats `resolve_target`'s uniqueness filter AND is suppressed from the classifier —
        // today its callee's effects vanish (silent-pure). Disclose Unknown instead. A UNIQUE same-name
        // leaf must STILL resolve (no spurious Unknown) — the precise-scoping control.
        let run = |name: &str, src: &str| -> serde_json::Value {
            let d = std::env::temp_dir().join(format!("candor-amb-{name}-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&d);
            std::fs::create_dir_all(d.join("src")).unwrap();
            std::fs::write(d.join("Cargo.toml"), format!("[package]\nname = \"{name}\"\n")).unwrap();
            std::fs::write(d.join("src/lib.rs"), src).unwrap();
            let prefix = d.join("out/r").to_string_lossy().into_owned();
            let idx = load_dep_reports(None);
            let (rc, body) = scan_one(&d.to_string_lossy(), ScanOpts {
                prefix, want_json: true, include_tests: false, policy: None, baseline: None, ws_member: false, quiet: true, deps_idx: &idx, peek_excluded: false,
            }, &crate::gate::begin_run());
            assert_eq!(rc, 0);
            let v: serde_json::Value = serde_json::from_str(&body.unwrap()).unwrap();
            let _ = std::fs::remove_dir_all(&d);
            v
        };
        let eff = |v: &serde_json::Value, needle: &str| -> Vec<String> {
            v["functions"].as_array().into_iter().flatten()
                .filter(|f| f["fn"].as_str().is_some_and(|q| q.contains(needle)))
                .flat_map(|f| f["inferred"].as_array().into_iter().flatten()
                    .filter_map(|e| e.as_str().map(String::from)).collect::<Vec<_>>()).collect()
        };
        // Two local defs of the leaf `process` (a free fn + a method); a bare `process()` call can't be
        // disambiguated → Unknown, not silent-pure.
        let ambiguous = r#"
            pub fn process() {}
            struct W;
            impl W { fn process(&self) {} }
            pub fn caller() { process(); }
        "#;
        assert!(eff(&run("ambig", ambiguous), "caller").contains(&"Unknown".to_string()),
                "a bare leaf with two same-name local defs must read Unknown, not silent-pure");
        // CONTROL: a UNIQUE local `solo` resolves cleanly — no spurious Unknown from this branch.
        let unique = r#"
            pub fn solo() {}
            pub fn caller() { solo(); }
        "#;
        assert!(!eff(&run("uniq", unique), "caller").contains(&"Unknown".to_string()),
                "a unique same-name leaf must resolve, never read a spurious Unknown");
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
        let no1: syn::ItemMod = syn::parse_str("#[cfg(feature = \"std\")] mod imp {}").unwrap();
        let no2: syn::ItemMod = syn::parse_str("mod real {}").unwrap();
        // `all(test, …)` CANNOT hold with test off, whatever the sibling is → still test-only.
        let yes2: syn::ItemMod =
            syn::parse_str("#[cfg(all(test, unix))] mod t {}").unwrap();
        // nested, but every branch still needs test → test-only
        let yes3: syn::ItemMod =
            syn::parse_str("#[cfg(any(all(test, unix), all(test, windows)))] mod t {}").unwrap();
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

    /// SOUNDNESS R122 — a published cardinal sin. `#[cfg(any(test, X))]` compiles into an ORDINARY
    /// build whenever X holds, so it does not require test at all; the old rule recursed into `any`
    /// and `all` alike and erased those items from the report. `all(test, X)` keeps its (correct)
    /// test-only verdict — the two quantifiers are not the same question.
    ///
    /// Fails against the pre-fix binary on the first four assertions (all returned `true`).
    #[test]
    fn cfg_any_test_with_a_production_sibling_is_not_test_only() {
        let m = |s: &str| syn::parse_str::<syn::ItemMod>(s).unwrap();
        // ── the sin: an `any` arm that a non-test build can satisfy ────────────────────────────────
        let feat = m("#[cfg(any(test, feature = \"extra\"))] mod x {}");
        let feat_rev = m("#[cfg(any(feature = \"alloc\", test))] mod x {}");
        let plat = m("#[cfg(any(test, unix))] mod x {}");
        // `miri`/`doc` are NOT decidable here and are deliberately KEPT: guessing them test-only is a
        // silent under-report, which is the direction this rule exists to refuse. Stated, not silent.
        let miri = m("#[cfg(any(test, miri))] mod x {}");
        // nested one level down — an `any` under an `all` still has a production-satisfiable arm.
        let nested = m("#[cfg(all(unix, any(test, feature = \"std\")))] mod x {}");
        for (name, item) in [("any(test,feature)", &feat), ("any(feature,test)", &feat_rev),
                             ("any(test,unix)", &plat), ("any(test,miri)", &miri),
                             ("all(unix,any(test,feature))", &nested)] {
            assert!(!is_cfg_test(&item.attrs), "`{name}` is production-reachable — must be scanned");
        }
        // ── the control: `all` stays test-only, in both nestings and BOTH CHILD ORDERS ─────────────
        // (the second spelling was NOT test-only before this fix, for an unrelated reason: an
        // unconsumed `= "extra"` aborted syn's sibling iteration before `test` was ever visited.)
        for s in ["#[cfg(all(test, feature = \"extra\"))] mod x {}",
                  "#[cfg(all(feature = \"extra\", test))] mod x {}",
                  "#[cfg(all(target_os = \"linux\", test))] mod x {}",
                  "#[cfg(any(all(test, unix), all(test, windows)))] mod x {}",
                  "#[cfg(test)] mod x {}"] {
            assert!(is_cfg_test(&m(s).attrs), "`{s}` cannot compile with test off — still test-only");
        }
        // ── a SECOND cfg attribute is AND-ed: one test-only attr is enough to skip ─────────────────
        let two = m("#[cfg(unix)] #[cfg(test)] mod x {}");
        assert!(is_cfg_test(&two.attrs));
        let two_ok = m("#[cfg(unix)] #[cfg(any(test, feature = \"std\"))] mod x {}");
        assert!(!is_cfg_test(&two_ok.attrs));
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
        let (mut fe, mut ev) = (FieldElemIndex::new(), HashMap::new());
        let mut fet = FieldElemTraitIndex::new();
        collect_decls(&file.items, false, &mut uses, &mut fields, &mut fe, &mut fet, &mut rets, &mut ev, &mut std::collections::HashMap::new(), &mut ti, &mut td, &mut tf, &mut std::collections::HashSet::new(), &mut std::collections::HashSet::new(), &mut std::collections::HashSet::new(), &mut std::collections::HashMap::new(), &mut std::collections::HashSet::new(), &mut std::collections::HashMap::new(), &mut std::collections::HashMap::new(), &mut std::collections::HashMap::new(), &mut std::collections::HashSet::new());
        assert_eq!(rets.get("new_with_defaults"), Some(&Some("Agent".to_string())),
                   "Self must resolve to the impl type, not the literal");
    }

    #[test]
    fn local_method_named_like_a_crate_does_not_inherit_the_crate_effect() {
        // A bare-leaf method call (`self.fastrand()`) is recorded path==leaf with no crate qualifier, so
        // the classifier would consult the LEAF against the calibrated crate rules (`fastrand` → Rand,
        // `time` → Clock). When that call RESOLVES TO A LOCAL DEFINITION, the local resolution is
        // authoritative and the external bare-leaf classification must be SUPPRESSED — tokio's pure
        // `FastRand::fastrand` xorshift must not fabricate Rand (it propagated to ~14 fns incl
        // `Runtime::new` in a real sweep). A genuine external `fastrand::u32()` call — qualified, NOT
        // local — must STILL classify Rand (no real effect dropped).
        let d = std::env::temp_dir().join(format!("candor-scan-localcrate-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(d.join("src")).unwrap();
        std::fs::write(d.join("Cargo.toml"), "[package]\nname = \"localcrate\"\n").unwrap();
        std::fs::write(
            d.join("src/lib.rs"),
            r#"
            pub struct FastRand { one: u32 }
            impl FastRand {
                // a PURE local method merely NAMED like the `fastrand` crate (tokio's xorshift)
                pub fn fastrand(&mut self) -> u32 { self.one ^= self.one << 1; self.one }
                pub fn fastrand_n(&mut self, n: u32) -> u32 { self.fastrand() % n }
                // a local method named like the `time`/`now` clock verb — also pure
                pub fn time(&self) -> u32 { self.one }
            }
            pub fn uses_local(r: &mut FastRand) { let _ = r.fastrand_n(5); let _ = r.time(); }
            // a REAL external dependency call — qualified, does NOT resolve locally → STILL Rand
            pub fn uses_external() { let _ = fastrand::u32(0..10); }
            "#,
        )
        .unwrap();
        let idx = load_dep_reports(None);
        let prefix = d.join("out/r").to_string_lossy().into_owned();
        let (rc, body) = scan_one(&d.to_string_lossy(), ScanOpts {
            prefix, want_json: true, include_tests: false, policy: None, baseline: None, ws_member: false, quiet: true, deps_idx: &idx, peek_excluded: false,
        }, &crate::gate::begin_run());
        assert_eq!(rc, 0);
        let body = body.expect("want_json returns the report body");
        let v: serde_json::Value = serde_json::from_str(&body).unwrap();
        // The report only emits EFFECTFUL functions (pure ones are omitted), so the effect key is
        // `inferred`; a function ABSENT from the report carries no effect (the desired outcome here).
        let effects_of = |needle: &str| -> Vec<String> {
            v["functions"].as_array().into_iter().flatten()
                .filter(|f| f["fn"].as_str().is_some_and(|q| q.contains(needle)))
                .flat_map(|f| f["inferred"].as_array().into_iter().flatten()
                    .filter_map(|e| e.as_str().map(String::from)).collect::<Vec<_>>())
                .collect()
        };
        // local `FastRand::fastrand`/`fastrand_n`/`time` and their callers carry NO crate effect.
        for q in ["FastRand::fastrand", "FastRand::fastrand_n", "FastRand::time", "uses_local"] {
            let eff = effects_of(q);
            assert!(!eff.contains(&"Rand".to_string()) && !eff.contains(&"Clock".to_string()),
                    "local method named like a crate fabricated an effect on {q}: {eff:?}\n{body}");
        }
        // the genuine external `fastrand::u32` call — unresolved locally — STILL classifies Rand.
        assert!(effects_of("uses_external").contains(&"Rand".to_string()),
                "a real external fastrand::u32 call must still report Rand:\n{body}");
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn local_fn_or_method_named_like_an_ffi_tier_does_not_fabricate() {
        // The leaf-PREFIX FFI tiers (`sqlite3_`/`git_`/`curl_`/`SSL_`) and whole-crate Rand
        // (`getrandom`/`fastrand`) classify by leaf name independent of the binding crate. A PURE local
        // FREE FN or qualified `Type::method` whose name collides was classified anyway — FABRICATION on a
        // provably-pure path that transitively poisons every caller (a fabrication). The general
        // local-resolution suppression must cover the free-fn and qualified-method cases the bare-leaf
        // guard missed. A genuine FFI binding (an `extern "C"` decl, no Rust body) must STILL classify.
        let d = std::env::temp_dir().join(format!("candor-scan-ffiname-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(d.join("src")).unwrap();
        std::fs::write(d.join("Cargo.toml"), "[package]\nname = \"ffiname\"\n").unwrap();
        std::fs::write(
            d.join("src/lib.rs"),
            r#"
            // PURE local free fns named like FFI tiers / whole-crate rules
            pub fn sqlite3_step() -> i32 { 0 }
            pub fn git_clone() {}
            pub fn getrandom() -> u32 { 4 }
            pub fn uses_sqlite() -> i32 { sqlite3_step() }
            pub fn uses_git() { git_clone() }
            pub fn uses_rand() -> u32 { getrandom() }
            // a PURE local qualified Type::method named like the git_ tier
            pub struct Repo;
            impl Repo { pub fn git_remote_fetch(&self) {} }
            pub fn uses_method(r: &Repo) { r.git_remote_fetch() }
            // AMBIGUOUS bare leaf: a free fn AND a method share an FFI-named leaf, defeating
            // resolve_target's uniqueness filter — the bare-leaf classifier must STILL be suppressed.
            pub fn curl_easy_perform() {}
            pub struct Conn;
            impl Conn { pub fn curl_easy_perform(&self) {} }
            pub fn uses_ambig() { curl_easy_perform() }
            // a GENUINE FFI binding (extern decl, no Rust body) — must STILL classify Db
            extern "C" { fn sqlite3_exec(p: *mut i8) -> i32; }
            pub fn real_ffi() { unsafe { sqlite3_exec(std::ptr::null_mut()); } }
            "#,
        )
        .unwrap();
        let idx = load_dep_reports(None);
        let prefix = d.join("out/r").to_string_lossy().into_owned();
        let (rc, body) = scan_one(&d.to_string_lossy(), ScanOpts {
            prefix, want_json: true, include_tests: false, policy: None, baseline: None, ws_member: false, quiet: true, deps_idx: &idx, peek_excluded: false,
        }, &crate::gate::begin_run());
        assert_eq!(rc, 0);
        let body = body.expect("want_json returns the report body");
        let v: serde_json::Value = serde_json::from_str(&body).unwrap();
        let effects_of = |needle: &str| -> Vec<String> {
            v["functions"].as_array().into_iter().flatten()
                .filter(|f| f["fn"].as_str().is_some_and(|q| q.contains(needle)))
                .flat_map(|f| f["inferred"].as_array().into_iter().flatten()
                    .filter_map(|e| e.as_str().map(String::from)).collect::<Vec<_>>())
                .collect()
        };
        // every pure local fn/method (and its caller) carries NO fabricated FFI effect
        for q in ["sqlite3_step", "git_clone", "getrandom", "uses_sqlite", "uses_git", "uses_rand",
                  "git_remote_fetch", "uses_method", "curl_easy_perform", "uses_ambig"] {
            assert!(effects_of(q).is_empty(),
                    "local fn/method named like an FFI tier FABRICATED an effect on {q}: {:?}\n{body}",
                    effects_of(q));
        }
        // the genuine extern-C FFI binding still classifies Db (no real effect dropped)
        assert!(effects_of("real_ffi").contains(&"Db".to_string()),
                "a real extern-C sqlite3_exec FFI call must still report Db:\n{body}");
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn ffi_safe_wrapper_of_an_unclassified_extern_fn_discloses_unknown() {
        // §4 honesty: a SAFE WRAPPER calling an `extern "C"` fn whose NAME the classifier doesn't know
        // (`system`, `my_native_writer`) has an unknowable body — the effect could be anything — so it must
        // DISCLOSE Unknown, never read silent-pure. Before this fix the `extern` block was never collected,
        // so the call was a bare leaf resolving to nothing → pure (the cardinal sin). CONTROLS: (a) a fn
        // with NO extern call stays pure (no fabrication); (b) a wrapper of a CLASSIFIED extern leaf
        // (`sqlite3_exec` → Db) keeps the precise effect, NOT a coarse Unknown.
        let d = std::env::temp_dir().join(format!("candor-scan-ffiwrap-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(d.join("src")).unwrap();
        std::fs::write(d.join("Cargo.toml"), "[package]\nname = \"ffiwrap\"\n").unwrap();
        std::fs::write(
            d.join("src/lib.rs"),
            r#"
            extern "C" {
                fn system(cmd: *const i8) -> i32;
                fn my_native_writer(p: *const u8, n: usize) -> i32;
                fn sqlite3_exec(p: *mut i8) -> i32;
            }
            // safe wrappers over UNCLASSIFIED extern fns → must DISCLOSE Unknown
            pub fn run_shell(cmd: *const i8) -> i32 { unsafe { system(cmd) } }
            pub fn native_write(p: *const u8, n: usize) -> i32 { unsafe { my_native_writer(p, n) } }
            // a transitive caller inherits the Unknown
            pub fn does_native_io() -> i32 { native_write(std::ptr::null(), 0) }
            // CONTROL (a): a genuinely-pure fn with NO extern call stays pure
            pub fn pure_math(a: i32, b: i32) -> i32 { a + b }
            // CONTROL (b): a wrapper of a CLASSIFIED extern leaf keeps the PRECISE effect (Db), not Unknown
            pub fn run_query() -> i32 { unsafe { sqlite3_exec(std::ptr::null_mut()) } }
            "#,
        )
        .unwrap();
        let idx = load_dep_reports(None);
        let prefix = d.join("out/r").to_string_lossy().into_owned();
        let (rc, body) = scan_one(&d.to_string_lossy(), ScanOpts {
            prefix, want_json: true, include_tests: false, policy: None, baseline: None, ws_member: false, quiet: true, deps_idx: &idx, peek_excluded: false,
        }, &crate::gate::begin_run());
        assert_eq!(rc, 0);
        let body = body.expect("want_json returns the report body");
        let v: serde_json::Value = serde_json::from_str(&body).unwrap();
        let effects_of = |needle: &str| -> Vec<String> {
            v["functions"].as_array().into_iter().flatten()
                .filter(|f| f["fn"].as_str().is_some_and(|q| q == needle))
                .flat_map(|f| f["inferred"].as_array().into_iter().flatten()
                    .filter_map(|e| e.as_str().map(String::from)).collect::<Vec<_>>())
                .collect()
        };
        // THE FIX: unclassified-extern wrappers disclose Unknown (not silent-pure)
        assert!(effects_of("run_shell").contains(&"Unknown".to_string()),
                "FFI wrapper of `system` must disclose Unknown:\n{body}");
        assert!(effects_of("native_write").contains(&"Unknown".to_string()),
                "FFI wrapper of `my_native_writer` must disclose Unknown:\n{body}");
        // the unknown propagates transitively
        assert!(effects_of("does_native_io").contains(&"Unknown".to_string()),
                "a caller of an FFI wrapper must inherit Unknown:\n{body}");
        // the disclosure names the FFI boundary
        let why = v["functions"].as_array().into_iter().flatten()
            .find(|f| f["fn"].as_str() == Some("run_shell"))
            .and_then(|f| f.get("unknownWhy").or_else(|| f.get("unknown_why")))
            .and_then(|w| w.as_array()).map(|a| a.iter().filter_map(|x| x.as_str()).collect::<Vec<_>>())
            .unwrap_or_default();
        assert!(why.iter().any(|r| r.starts_with("native:")), "unknownWhy must name the native/FFI boundary (canonical native:): {why:?}\n{body}");
        // CONTROL (a): pure_math has NO effect — never fabricated (it's absent from the effectful report)
        assert!(effects_of("pure_math").is_empty(),
                "a pure fn with no extern call must stay pure (no fabricated Unknown):\n{body}");
        // CONTROL (b): a CLASSIFIED extern leaf keeps its precise Db, NOT a coarse Unknown
        let rq = effects_of("run_query");
        assert!(rq.contains(&"Db".to_string()), "classified extern (sqlite3_exec) must stay Db:\n{body}");
        assert!(!rq.contains(&"Unknown".to_string()),
                "a CLASSIFIED extern leaf must NOT be downgraded to Unknown:\n{body}");
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn drop_glue_of_a_local_effectful_drop_propagates_to_the_binder() {
        // §4 honesty (#3): a fn that constructs a value of a LOCAL type whose `impl Drop` does I/O must
        // inherit that Drop body's effect — the scope-exit `drop` is an implicit edge the call graph misses,
        // so a flushing/closing guard otherwise read silent-pure. CONTROLS: (a) a local type with a PURE
        // Drop adds no effect; (b) an EXTERNAL type's invisible Drop is never fabricated (we only model
        // LOCAL Drop impls).
        let d = std::env::temp_dir().join(format!("candor-scan-dropglue-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(d.join("src")).unwrap();
        std::fs::write(d.join("Cargo.toml"), "[package]\nname = \"dropglue\"\n").unwrap();
        std::fs::write(
            d.join("src/lib.rs"),
            r#"
            // a LOCAL type whose Drop writes a file (effectful scope-exit)
            pub struct FlushGuard { path: String }
            impl FlushGuard { pub fn new(p: &str) -> Self { FlushGuard { path: p.to_string() } } }
            impl Drop for FlushGuard {
                fn drop(&mut self) { std::fs::write(&self.path, b"flush").unwrap(); }
            }
            // binds a FlushGuard → the implicit drop edge must give this fn Fs
            pub fn does_work_with_guard() {
                let _g = FlushGuard::new("/tmp/x");
                let _ = 1 + 1;
            }
            // CONTROL (a): a LOCAL type with a PURE Drop — binding it adds no effect
            pub struct PureGuard;
            impl PureGuard { pub fn new() -> Self { PureGuard } }
            impl Drop for PureGuard { fn drop(&mut self) { /* nothing */ } }
            pub fn does_work_pure_guard() { let _g = PureGuard::new(); }
            "#,
        )
        .unwrap();
        let idx = load_dep_reports(None);
        let prefix = d.join("out/r").to_string_lossy().into_owned();
        let (rc, body) = scan_one(&d.to_string_lossy(), ScanOpts {
            prefix, want_json: true, include_tests: false, policy: None, baseline: None, ws_member: false, quiet: true, deps_idx: &idx, peek_excluded: false,
        }, &crate::gate::begin_run());
        assert_eq!(rc, 0);
        let body = body.expect("want_json returns the report body");
        let v: serde_json::Value = serde_json::from_str(&body).unwrap();
        let effects_of = |needle: &str| -> Vec<String> {
            v["functions"].as_array().into_iter().flatten()
                .filter(|f| f["fn"].as_str().is_some_and(|q| q == needle))
                .flat_map(|f| f["inferred"].as_array().into_iter().flatten()
                    .filter_map(|e| e.as_str().map(String::from)).collect::<Vec<_>>())
                .collect()
        };
        // THE FIX: the binder inherits the effectful Drop's Fs via the implicit scope-exit edge
        assert!(effects_of("does_work_with_guard").contains(&"Fs".to_string()),
                "a fn binding a local guard with an effectful Drop must inherit Fs (drop glue):\n{body}");
        // CONTROL (a): a local guard with a PURE Drop fabricates nothing
        assert!(effects_of("does_work_pure_guard").is_empty(),
                "a local guard with a PURE Drop must add no effect:\n{body}");
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn drop_glue_through_an_owned_field_charges_unless_the_owner_escapes() {
        // §4 honesty (R49, field edition): constructing a struct that OWNS a local effectful-Drop guard as a
        // field runs that guard's Drop at the owner's scope exit — the transitive drop-owner closure charges
        // the constructing fn (directly `_g: Guard`, a `Vec<Guard>` element, or a nested owner). ESCAPE GATE:
        // when the fn RETURNS a local aggregate (or a drop-type), a value built here may be moved into it and
        // drop in the CALLER — so a constructor is NOT charged (never fabricate a returned owner's Drop, the
        // flate2 miss). The gate is a conservative membership check (sound under leaf-name collisions), so a
        // `-> ()` local-use fn charges while any owner-returning fn skips.
        let d = std::env::temp_dir().join(format!("candor-dropfield-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(d.join("src")).unwrap();
        std::fs::write(d.join("Cargo.toml"), "[package]\nname = \"dropfield\"\n").unwrap();
        std::fs::write(
            d.join("src/lib.rs"),
            r#"
            use std::fs;
            pub struct Guard;
            impl Drop for Guard { fn drop(&mut self) { let _ = fs::remove_file("/g"); } }  // Fs
            pub struct Session { _g: Guard }
            impl Session { pub fn new() -> Session { Session { _g: Guard } } }
            pub struct Outer { _s: Session }
            pub struct Pool { _v: Vec<Guard> }
            pub struct Plain { _n: i32 }
            // LOCAL-USE (returns unit) → the owned Guard drops here → charged
            pub fn owns_field() { let _s = Session { _g: Guard }; let _ = 1; }
            pub fn owns_via_ctor() { let _s = Session::new(); let _ = 1; }
            pub fn owns_nested() { let _o = Outer { _s: Session::new() }; }
            pub fn owns_vec() { let _p = Pool { _v: vec![Guard] }; }
            // ESCAPE (returns the owner) → the Guard drops in the CALLER → NOT charged (no fabrication)
            pub fn make_session() -> Session { Session::new() }
            pub fn make_outer() -> Outer { Outer { _s: Session::new() } }
            pub fn make_via_let() -> Outer { let o = Outer { _s: Session::new() }; o }
            // CONTROL: pure-field owner → nothing
            pub fn owns_plain() { let _p = Plain { _n: 1 }; }
            "#,
        )
        .unwrap();
        let idx = load_dep_reports(None);
        let prefix = d.join("out/r").to_string_lossy().into_owned();
        let (rc, body) = scan_one(&d.to_string_lossy(), ScanOpts {
            prefix, want_json: true, include_tests: false, policy: None, baseline: None, ws_member: false, quiet: true, deps_idx: &idx, peek_excluded: false,
        }, &crate::gate::begin_run());
        assert_eq!(rc, 0);
        let body = body.expect("want_json returns the report body");
        let v: serde_json::Value = serde_json::from_str(&body).unwrap();
        let has_fs = |needle: &str| -> bool {
            v["functions"].as_array().into_iter().flatten()
                .filter(|f| f["fn"].as_str() == Some(needle))
                .flat_map(|f| f["inferred"].as_array().into_iter().flatten().filter_map(|e| e.as_str()))
                .any(|e| e == "Fs")
        };
        for f in ["owns_field", "owns_via_ctor", "owns_nested", "owns_vec"] {
            assert!(has_fs(f), "a field-owned local guard's Drop must propagate to `{f}`:\n{body}");
        }
        for f in ["make_session", "make_outer", "make_via_let", "owns_plain"] {
            assert!(!has_fs(f), "an ESCAPING (returned) owner's Drop must not be charged at `{f}`:\n{body}");
        }
        let _ = std::fs::remove_dir_all(&d);
    }


    /// ⟨drop-glue POSITION⟩ The charge fired on the BINDER, so sixteen of seventeen executed positions
    /// were silent, and the TUPLE-STRUCT (newtype) spelling had no route in ANY position — including the
    /// bound local.
    ///
    /// GROUND TRUTH EXECUTED, not inferred: each shape was compiled and run with the destructor
    /// appending to a log interleaved against per-function call/return markers, and every one printed
    /// before its function returned. Across 166 units the pre-fix binary was silent on 76 of the 99 that
    /// genuinely release a guard; after, on 1 (a generic `T::mk()`, which no syntactic scan can key).
    ///
    /// The two routes that produced that were a `T::assoc()` CALL walk in scan.rs (construction-keyed,
    /// so sound, but blind to `Guard(f)` — a single-segment `Expr::Call` with no `::` to test, whose
    /// imported spelling `m::Guard` presents the MODULE as the type) and a `T::<construct>` marker in
    /// the collector emitted only under `Pat::Ident`. The rule is now stated once, at the construction
    /// expression (`CallCollector::note_construction`, reached from the three expression shapes), and
    /// the binder site is REMOVED rather than left beside it.
    #[test]
    fn drop_glue_fires_at_the_construction_not_at_the_binder() {
        let v = scan_fixture("dropposition", r#"
            pub struct BraceG { pub n: u32 }
            impl BraceG { pub fn noop(&self) {} }
            impl Drop for BraceG { fn drop(&mut self) { let _ = std::fs::write("/tmp/b", "x"); } }
            pub struct TupleG(pub u32);
            impl Drop for TupleG { fn drop(&mut self) { let _ = std::fs::write("/tmp/t", "x"); } }
            pub struct UnitG;
            impl Drop for UnitG { fn drop(&mut self) { let _ = std::fs::write("/tmp/u", "x"); } }
            pub enum EnumG { A, B }
            impl Drop for EnumG { fn drop(&mut self) { let _ = std::fs::write("/tmp/e", "x"); } }
            pub struct Plain { pub n: u32 }
            pub struct PlainT(pub u32);
            pub fn sink<T>(_t: T) {}

            // TUPLE-STRUCT / NEWTYPE — the spelling with no route at all, in the position the binder
            // route did cover.
            pub fn tuple_bound() { let _g = TupleG(0); }
            // …and in the positions it never covered, one per construction spelling.
            pub fn tuple_bare_stmt() { TupleG(0); }
            pub fn brace_let_wild() { let _ = BraceG { n: 0 }; }
            pub fn brace_call_arg() { sink(BraceG { n: 0 }); }
            pub fn brace_match_scrutinee() { match (BraceG { n: 0 }) { _ => {} } }
            pub fn brace_method_receiver() { (BraceG { n: 0 }).noop(); }
            pub fn unit_array_elem() { let _a = [UnitG]; }
            pub fn enum_tuple_destructuring() { let (_a, _b) = (EnumG::A, 1u32); }
            pub fn tuple_vec_push() { let mut v = Vec::new(); v.push(TupleG(0)); sink(&v); }

            // CONTROL: the same seventeen positions with a type that has NO `Drop` fabricate nothing.
            pub fn plain_bare_stmt() { PlainT(0); }
            pub fn plain_call_arg() { sink(Plain { n: 0 }); }
        "#);
        for f in ["tuple_bound", "tuple_bare_stmt", "brace_let_wild", "brace_call_arg",
                  "brace_match_scrutinee", "brace_method_receiver", "unit_array_elem",
                  "enum_tuple_destructuring", "tuple_vec_push"] {
            assert!(fixture_effects(&v, f).contains(&"Fs".to_string()),
                    "`{f}` releases a guard in this scope and must inherit its Drop's Fs:\n{v:#}");
        }
        for f in ["plain_bare_stmt", "plain_call_arg"] {
            assert!(fixture_effects(&v, f).is_empty(),
                    "`{f}` constructs a type with no Drop — nothing may be fabricated:\n{v:#}");
        }
    }

    /// ⟨drop-glue ESCAPE⟩ The load-bearing half, and the one that reverted the first attempt at the
    /// field route (candor-spec SOUNDNESS R49: 14 false `Unknown`s on flate2, from constructors that
    /// CONSTRUCT AND RETURN the owner). Widening the construction route without a lexical escape gate
    /// multiplies that over every constructor of every guard type in a crate — so every control here is
    /// a shape whose destructor provably runs in someone ELSE'S frame.
    ///
    /// A/B against flate2 1.1.9 itself: all thirteen of its constructor rows (`ZlibEncoder::new`,
    /// `gz_encoder`, `DeflateDecoder::new`, …) are refused, and its sixteen genuinely-releasing
    /// `finish`/`into_inner` rows are gained.
    #[test]
    fn drop_glue_refuses_a_construction_that_escapes_the_scope() {
        let v = scan_fixture("dropescape", r#"
            pub struct G { pub n: u32 }
            impl Drop for G { fn drop(&mut self) { let _ = std::fs::write("/tmp/g", "x"); } }
            pub struct T(pub u32);
            impl Drop for T { fn drop(&mut self) { let _ = std::fs::write("/tmp/t", "x"); } }
            pub struct Owner { pub g: G }
            pub fn with_slot<R>(f: impl FnOnce() -> R) -> R { f() }

            // ESCAPES — the destructor runs in the CALLER. Every one must stay PURE.
            pub fn make_brace() -> G { G { n: 0 } }
            pub fn make_tuple() -> T { T(0) }
            pub fn make_owner() -> Owner { Owner { g: G { n: 0 } } }
            pub fn wrap_owner() -> Owner { let g = G { n: 0 }; Owner { g } }
            pub fn make_result() -> Result<G, ()> { Ok(G { n: 0 }) }
            pub fn make_boxed() -> Box<T> { Box::new(T(0)) }
            pub fn make_vec() -> Vec<T> { vec![T(0)] }
            pub fn store(slot: &mut Option<G>) { *slot = Some(G { n: 0 }); }
            pub fn build_pushed() -> Vec<T> { let mut v = Vec::new(); v.push(T(0)); v }
            pub fn closure_returns() -> Option<T> { with_slot(|| Some(T(0))) }
            // SUPPRESSED DESTRUCTORS — `Drop` provably never runs.
            pub fn forgotten() { let g = G { n: 0 }; std::mem::forget(g); }
            pub fn manually() { let _m = std::mem::ManuallyDrop::new(T(0)); }

            // …and the guard that does NOT escape is still charged, in a fn that returns an aggregate.
            // This is what the previous `returns_escapable` gate could not express: it skipped EVERY
            // type as soon as the signature returned one.
            pub fn local_guard_beside_a_returned_owner() -> Owner {
                let _local = T(0);
                Owner { g: G { n: 0 } }
            }
        "#);
        for f in ["make_brace", "make_tuple", "make_owner", "wrap_owner", "make_result", "make_boxed",
                  "make_vec", "store", "build_pushed", "closure_returns", "forgotten", "manually"] {
            // `Fs` is the guard's own effect and the only thing under test. Not `is_empty()`:
            // `closure_returns` calls a generic callback and carries an honest `Unknown` for it, which
            // is a different mechanism and would make this assertion pass for the wrong reason.
            assert!(!fixture_effects(&v, f).contains(&"Fs".to_string()),
                    "`{f}` does not release the value in this scope — charging it FABRICATES:\n{v:#}");
        }
        assert!(fixture_effects(&v, "local_guard_beside_a_returned_owner").contains(&"Fs".to_string()),
                "a guard that does NOT escape must still be charged in an aggregate-returning fn:\n{v:#}");
    }

    /// ⟨drop-glue ESCAPE — CONDITIONAL⟩ Regression for a bug shipped and caught the same day (measured,
    /// not inferred: 166 units executed, destructor writes interleaved against call/return markers).
    /// The escape gate above was NAME-based, not PATH-based: it suppressed the charge as soon as the
    /// constructed name reached ANY return/tail position, regardless of whether that return happens on
    /// EVERY path. `let g = G{..}; if f { Some(g) } else { None } }` escapes only when `f` is true and
    /// drops `g` — genuinely, executing the write — on the other branch, so the old gate read the whole
    /// function pure unconditionally.
    ///
    /// The fix (`escaping_ctor_leaves`'s doc comment has the full contract): suppress only when the
    /// construction escapes on EVERY terminal exit — each independent exit analysed separately and
    /// intersected, and an `if`/`match` arm must ALL agree, not just one. Every fixture below drops the
    /// guard on the specific input given, so `Fs` must be charged for all seven; the two controls prove
    /// the fix did not touch the UNCONDITIONAL-escape case it must leave alone.
    #[test]
    fn drop_glue_charges_a_construction_that_escapes_only_on_some_paths() {
        let v = scan_fixture("dropcondescape", r#"
            pub struct G { pub n: u32 }
            impl Drop for G { fn drop(&mut self) { let _ = std::fs::write("/tmp/g", "x"); } }

            // Every one of these drops `G` on at least one reachable path — `Fs` must be charged.
            pub fn c_if(f: bool) -> Option<G> { let g = G{n:1}; if f { Some(g) } else { None } }
            pub fn c_guard(f: bool) -> Option<G> { let g = G{n:2}; if !f { return None; } Some(g) }
            pub fn c_match(f: bool) -> Option<G> {
                let g = G{n:3};
                match f { true => Some(g), false => None }
            }
            pub fn c_early(f: bool) -> Option<G> { let g = G{n:4}; if f { return None; } Some(g) }
            pub fn c_loop(n: u32) -> Option<G> {
                for i in 0..n { let g = G{n:5}; if i > 100 { return Some(g); } }
                None
            }
            pub fn c_result(f: bool) -> Result<G, u32> { let g = G{n:6}; if f { Ok(g) } else { Err(1) } }
            pub fn c_question(f: bool) -> Option<G> {
                let g = G{n:7};
                let _x: u32 = if f { Some(1) } else { None }?;
                Some(g)
            }

            // CONTROLS — unconditional escape must stay pure; the fix must not touch this case.
            pub fn u_always() -> G { G{n:90} }
            pub fn u_always_wrapped() -> Option<G> { Some(G{n:91}) }
        "#);
        for f in ["c_if", "c_guard", "c_match", "c_early", "c_loop", "c_result", "c_question"] {
            assert!(fixture_effects(&v, f).contains(&"Fs".to_string()),
                    "`{f}` drops the guard on a reachable path — the escape gate must not suppress it \
                     just because ANOTHER path escapes:\n{v:#}");
        }
        for f in ["u_always", "u_always_wrapped"] {
            assert!(!fixture_effects(&v, f).contains(&"Fs".to_string()),
                    "`{f}` escapes on every path (there is only one) — must stay pure:\n{v:#}");
        }
    }

    /// ⟨drop-glue ESCAPE — VACUOUS ARM⟩ Over-charge control for the fix above, on REAL code: lapin
    /// 2.5.5's `channel::Channel::new` —
    /// `let channel_closer = if id == 0 { None } else { Some(Arc::new(ChannelCloser::new(..))) };`
    /// then returns `Channel { .., channel_closer, .. }`. `ChannelCloser` is built and escapes in the
    /// SAME arm; the `then` arm never builds one at all, so it has no opinion on whether it escapes —
    /// intersecting leaves across arms (as the fix above does for NAMES) let that silent arm veto a
    /// fact it never touched, and `Channel::new` fabricated a `Log` charge that A/B against the crate
    /// caught. Fixed by unioning LEAVES discovered via a direct in-arm construction, while still
    /// intersecting NAMES (`mark_escape`'s doc comment on the `If`/`Match` case has the full argument
    /// for why the two must be treated differently).
    #[test]
    fn drop_glue_does_not_let_a_construction_free_arm_veto_a_sibling_that_never_builds_it() {
        let v = scan_fixture("dropvacuousarm", r#"
            pub struct Closer { pub id: u32 }
            impl Drop for Closer { fn drop(&mut self) { let _ = std::fs::write("/tmp/c", "x"); } }
            pub struct Channel { pub id: u32, pub closer: Option<std::sync::Arc<Closer>> }
            pub fn make(id: u32) -> Channel {
                let closer = if id == 0 {
                    None
                } else {
                    Some(std::sync::Arc::new(Closer { id }))
                };
                Channel { id, closer }
            }
            // MATCH shape of the same thing (isahc/tokio commonly branch this way instead).
            pub fn make_match(id: u32) -> Channel {
                let closer = match id {
                    0 => None,
                    _ => Some(std::sync::Arc::new(Closer { id })),
                };
                Channel { id, closer }
            }
        "#);
        for f in ["make", "make_match"] {
            assert!(!fixture_effects(&v, f).contains(&"Fs".to_string()),
                    "`{f}` only ever builds `Closer` on the arm that also escapes it — the sibling arm, \
                     which never builds one, must not veto that:\n{v:#}");
        }
    }

    /// ⟨drop-glue PARAMETER-OWNED⟩ A mechanism construction-keying cannot reach BY DEFINITION: `fn
    /// take(g: Guard) {}` runs `Guard::drop` inside `take`, and the scan never saw the value built. The
    /// borrow controls are where this is one keystroke from a fabrication — and `self: Pin<&mut Self>`
    /// is the one that actually bit: syn parses it as a `Receiver` whose `reference` is `None`, so the
    /// obvious test read every `poll_read`/`poll_flush` in the ecosystem as consuming its receiver
    /// (measured: seven tokio drop types charged to a `poll_flush` whose body is `Poll::Ready(Ok(()))`).
    #[test]
    fn drop_glue_charges_a_by_value_parameter_and_never_a_borrowed_one() {
        let v = scan_fixture("dropparam", r#"
            pub struct G { pub n: u32 }
            impl Drop for G { fn drop(&mut self) { let _ = std::fs::write("/tmp/g", "x"); } }
            pub fn sink<T>(_t: T) {}
            pub fn owned(g: G) { sink(&g); }
            pub fn boxed(g: Box<G>) { sink(&g); }
            impl G {
                pub fn consume_self(self) -> u32 { self.n + 1 }
                // BORROW CONTROLS — none of these releases anything.
                pub fn by_ref(&self) -> u32 { self.n }
                pub fn by_ref_mut(&mut self) -> u32 { self.n }
                pub fn pinned(self: std::pin::Pin<&mut Self>) -> u32 { 0 }
                // ESCAPE CONTROL — `self` is handed to the caller, not released here.
                pub fn into_n(self) -> u32 { self.n }
            }
            pub fn borrowed(g: &G) { sink(g); }
            pub fn pointed(g: *const G) { sink(g); }
        "#);
        for f in ["owned", "boxed", "G::consume_self"] {
            assert!(fixture_effects(&v, f).contains(&"Fs".to_string()),
                    "`{f}` owns its argument by value and releases it here:\n{v:#}");
        }
        for f in ["G::by_ref", "G::by_ref_mut", "G::pinned", "G::into_n", "borrowed", "pointed"] {
            assert!(fixture_effects(&v, f).is_empty(),
                    "`{f}` never releases the value — charging it FABRICATES:\n{v:#}");
        }
    }

    /// ⟨drop-glue COLLISIONS⟩ `drop_types` and `owned_drops` are keyed by type LEAF (they have to be —
    /// `type_path` yields leaves), so widening the construction route to bare VALUE PATHS put every
    /// same-leaf name in reach. Both of these were measured on real crates, and neither is reachable
    /// from a fixture that only tests the happy path.
    ///
    ///  · tokio: `use std::sync::atomic::Ordering::*;` beside `struct Acquire<'a>` with a
    ///    tracing-instrumented `Drop`. `self.permits.load(Acquire)` read the atomic-ordering CONSTANT
    ///    as a construction of the FUTURE, so every `is_closed`/`is_idle`/`available_permits` in
    ///    `batch_semaphore` and `mpsc` inherited its `Log`. A struct WITH fields cannot be written as a
    ///    bare path at all, which is the discriminator.
    ///  · isahc: syn 2 represents `Pat::Path` with the very same `ExprPath` node an expression uses, so
    ///    a `match` ARM PATTERN reached the construction site. `AsyncBody::len(&self)` — one `match`
    ///    over three arms — was charged the agent `Handle`'s `Drop`.
    #[test]
    fn drop_glue_reads_neither_a_shadowed_constant_nor_a_match_pattern_as_a_construction() {
        let v = scan_fixture("dropcollide", r#"
            pub mod guard {
                pub struct Acquire { pub n: u32 }
                impl Drop for Acquire { fn drop(&mut self) { let _ = std::fs::write("/tmp/a", "x"); } }
                // A SECOND `Inner`, one module over, that owns the guard — which is what puts the
                // LEAF `Inner` into the drop-relevant set and makes the pattern below reachable.
                // (isahc's real shape: `agent::Inner` holds the `Handle`; `body::Inner` is an enum.)
                pub struct Inner { pub a: Acquire }
            }
            pub mod user {
                use std::sync::atomic::Ordering::*;
                pub enum Inner { Empty, Buf(u32) }
                pub struct Body { pub state: std::sync::atomic::AtomicUsize, pub inner: Inner }
                impl Body {
                    // `Acquire` here is the std ORDERING, not the guard one module over.
                    pub fn is_closed(&self) -> bool { self.state.load(Acquire) == 0 }
                    // `Inner::Empty` here is a PATTERN, which matches a value and never builds one.
                    pub fn len(&self) -> u64 {
                        match &self.inner { Inner::Empty => 0, Inner::Buf(n) => *n as u64 }
                    }
                }
            }
        "#);
        for f in ["user::Body::is_closed", "user::Body::len"] {
            assert!(fixture_effects(&v, f).is_empty(),
                    "`{f}` constructs nothing — a leaf collision must not fabricate a Drop:\n{v:#}");
        }
    }

    #[test]
    fn drop_glue_never_fabricates_for_an_external_type() {
        // CONTROL (b) for #3, isolated: a fn that binds an EXTERNAL type (whose Drop we cannot see) must
        // NOT get a fabricated drop effect — we model ONLY local `impl Drop`. A `std::fs::File` is dropped
        // at scope exit but its Drop (close) is invisible/benign; charging the binder an effect would be a
        // fabrication. (The `std::fs::File::create` open itself is Fs via the classifier — that's correct —
        // but a fn that merely RECEIVES a File by value and lets it drop must stay pure.)
        let d = std::env::temp_dir().join(format!("candor-scan-dropext-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(d.join("src")).unwrap();
        std::fs::write(d.join("Cargo.toml"), "[package]\nname = \"dropext\"\n").unwrap();
        std::fs::write(
            d.join("src/lib.rs"),
            r#"
            // receives an external File by value; it drops here, but its Drop is not a LOCAL impl → pure
            pub fn consumes_a_file(f: std::fs::File) { let _f = f; }
            "#,
        )
        .unwrap();
        let idx = load_dep_reports(None);
        let prefix = d.join("out/r").to_string_lossy().into_owned();
        let (rc, body) = scan_one(&d.to_string_lossy(), ScanOpts {
            prefix, want_json: true, include_tests: false, policy: None, baseline: None, ws_member: false, quiet: true, deps_idx: &idx, peek_excluded: false,
        }, &crate::gate::begin_run());
        assert_eq!(rc, 0);
        let body = body.expect("want_json returns the report body");
        let v: serde_json::Value = serde_json::from_str(&body).unwrap();
        let effs: Vec<String> = v["functions"].as_array().into_iter().flatten()
            .filter(|f| f["fn"].as_str() == Some("consumes_a_file"))
            .flat_map(|f| f["inferred"].as_array().into_iter().flatten()
                .filter_map(|e| e.as_str().map(String::from)).collect::<Vec<_>>())
            .collect();
        assert!(effs.is_empty(),
                "binding an EXTERNAL type must not fabricate a drop effect (only local Drop is modeled):\n{body}");
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn lazy_unit_qual_is_module_scoped_so_same_named_statics_do_not_merge() {
        // Two modules each declaring `static CFG` produced ONE unit named `<lazy>::CFG` carrying the union
        // of both initializers' effects — and, because `resolve_target` resolves a `::` path by tail2 and
        // requires a unique hit, the now-ambiguous lookup dropped every forcing edge, so both readers read
        // SOUND-COMPLETE PURE. A silent under-report caused purely by a naming collision
        // (candor-spec SOUNDNESS-VEIN-global-unit-identity.md). The module path goes INSIDE the prefix so
        // tail2 (`<mod>::<NAME>`) still discriminates; appending it after the name would not.
        let d = std::env::temp_dir().join(format!("candor-scan-lazyqual-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(d.join("src")).unwrap();
        std::fs::write(d.join("Cargo.toml"),
            "[package]\nname = \"x\"\nversion = \"0.0.0\"\nedition = \"2021\"\n").unwrap();
        std::fs::write(d.join("src/main.rs"), r#"
            mod core_m { pub static CFG: std::sync::LazyLock<String> =
                std::sync::LazyLock::new(|| std::fs::read_to_string("/etc/core").unwrap_or_default()); }
            mod util_m { pub static CFG: std::sync::LazyLock<String> =
                std::sync::LazyLock::new(|| std::env::var("U").unwrap_or_default());
                pub fn util_uses() -> usize { CFG.len() } }
            fn main() { println!("{}", util_m::util_uses()); }
            "#).unwrap();
        let idx = load_dep_reports(None);
        let prefix = d.join("out/r").to_string_lossy().into_owned();
        let (rc, body) = scan_one(&d.to_string_lossy(), ScanOpts {
            prefix, want_json: true, include_tests: false, policy: None, baseline: None, ws_member: false, quiet: true, deps_idx: &idx, peek_excluded: false,
        }, &crate::gate::begin_run());
        assert_eq!(rc, 0);
        let body = body.expect("want_json returns the report body");
        let v: serde_json::Value = serde_json::from_str(&body).unwrap();
        let eff = |needle: &str| -> Vec<String> {
            v["functions"].as_array().into_iter().flatten()
                .filter(|f| f["fn"].as_str() == Some(needle))
                .flat_map(|f| f["inferred"].as_array().into_iter().flatten()
                    .filter_map(|e| e.as_str().map(String::from)).collect::<Vec<_>>())
                .collect()
        };
        // the two statics stay SEPARATE units, each carrying only its own effect
        assert_eq!(eff("<lazy>::core_m::CFG"), vec!["Fs"], "core's lazy keeps only its own Fs:\n{body}");
        assert_eq!(eff("<lazy>::util_m::CFG"), vec!["Env"], "util's lazy keeps only its own Env:\n{body}");
        // the forcing edges resolve again — this is the half that was the cardinal sin
        assert_eq!(eff("util_m::util_uses"), vec!["Env"],
                   "a reader of util's CFG carries Env (it read silent-pure while the names merged):\n{body}");
        assert_eq!(eff("main"), vec!["Env"], "and transitively through main:\n{body}");
        // the fabrication mirror: a reader must not pick up the OTHER module's effect
        assert!(!eff("util_m::util_uses").contains(&"Fs".to_string()),
                "a reader must not inherit the other module's Fs:\n{body}");
        let _ = std::fs::remove_dir_all(&d);
    }

    /// THE OTHER HALF OF `5447eba` (the commit the test above pins). Moving the module path INSIDE the
    /// `<lazy>::` prefix made the WRITER module-qualified; the READER still built
    /// `<lazy>::<its own module>::NAME`. So a lazy static read from ANY module other than the one that
    /// declares it missed its unit's tail2 and read SILENT-PURE — while a read from inside the declaring
    /// module was charged correctly, which is exactly why no fixture saw it.
    ///
    /// BOTH SPELLINGS, because picking one is how this class of defect survives: the module PATH
    /// (`m::INNER`, `crate::m::INNER`) and the `use` (`use m::INNER; INNER`), file-level and body-level.
    /// And the mirror control is the property `5447eba` bought: two modules each declaring `CFG`, one
    /// effectful and one pure, where the reader of the PURE one must not inherit the other's `Fs`.
    #[test]
    fn a_lazy_static_read_from_outside_its_module_is_charged_like_one_read_inside_it() {
        let d = std::env::temp_dir().join(format!("candor-scan-lazyread-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(d.join("src")).unwrap();
        std::fs::write(d.join("Cargo.toml"),
            "[package]\nname = \"x\"\nversion = \"0.0.0\"\nedition = \"2021\"\n").unwrap();
        std::fs::write(d.join("src/lib.rs"), r#"
            use std::sync::LazyLock;
            pub mod m {
                use std::sync::LazyLock;
                pub static INNER: LazyLock<u8> = LazyLock::new(|| std::fs::read("/tmp/z").map(|v| v.len() as u8).unwrap_or(0));
                pub fn inside() -> u8 { *INNER }
            }
            pub mod pure_m {
                use std::sync::LazyLock;
                pub static INNER2: LazyLock<u8> = LazyLock::new(|| 7);
            }
            pub static TOP: LazyLock<u8> = LazyLock::new(|| std::fs::read("/tmp/t").map(|v| v.len() as u8).unwrap_or(0));
            pub fn top_read() -> u8 { *TOP }
            pub fn outside_path() -> u8 { *m::INNER }
            pub fn outside_crate_path() -> u8 { *crate::m::INNER }
            pub fn outside_body_use() -> u8 { use m::INNER; *INNER }
            pub mod reader {
                use crate::m::INNER;
                pub fn outside_file_use() -> u8 { *INNER }
            }
            // CONTROLS — must stay pure.
            pub fn reads_pure_module_static() -> u8 { *pure_m::INNER2 }
            pub fn shadowed_by_an_untypable_let() -> usize { let INNER = "aa"; INNER.len() }
            "#).unwrap();
        let idx = load_dep_reports(None);
        let prefix = d.join("out/r").to_string_lossy().into_owned();
        let (rc, body) = scan_one(&d.to_string_lossy(), ScanOpts {
            prefix, want_json: true, include_tests: false, policy: None, baseline: None, ws_member: false, quiet: true, deps_idx: &idx, peek_excluded: false,
        }, &crate::gate::begin_run());
        assert_eq!(rc, 0);
        let body = body.expect("want_json returns the report body");
        let v: serde_json::Value = serde_json::from_str(&body).unwrap();
        let eff = |needle: &str| -> Vec<String> {
            v["functions"].as_array().into_iter().flatten()
                .filter(|f| f["fn"].as_str() == Some(needle))
                .flat_map(|f| f["inferred"].as_array().into_iter().flatten()
                    .filter_map(|e| e.as_str().map(String::from)).collect::<Vec<_>>())
                .collect()
        };
        // the writer already knew — the unit is module-qualified
        assert_eq!(eff("<lazy>::m::INNER"), vec!["Fs"], "the unit itself:\n{body}");
        // the arm that always worked, kept as the oracle the others are compared to
        assert_eq!(eff("m::inside"), vec!["Fs"], "a read from INSIDE the declaring module:\n{body}");
        assert_eq!(eff("top_read"), vec!["Fs"], "a crate-root static read at crate root:\n{body}");
        // every spelling of the read that was silent
        for f in ["outside_path", "outside_crate_path", "outside_body_use", "reader::outside_file_use"] {
            assert_eq!(eff(f), vec!["Fs"],
                       "{f} reads a module-scoped lazy static from outside its module — it must be \
                        charged exactly like `m::inside`, which is:\n{body}");
        }
        // MIRROR CONTROLS — the new reader-side key must not fire where it cannot reach
        assert!(eff("reads_pure_module_static").is_empty(),
                "a PURE module-scoped lazy static's reader stays pure (per-static keying):\n{body}");
        assert!(eff("shadowed_by_an_untypable_let").is_empty(),
                "a `let` whose initializer types to nothing STILL shadows the static — charging it here \
                 would be the fabrication mirror of the fix:\n{body}");
        let _ = std::fs::remove_dir_all(&d);
    }

    /// A CHAINED DEP's lazy static, under the IMPORT spelling. `deplib::CFG` was handled; the idiomatic
    /// `use deplib::CFG; CFG` leaves a ONE-segment path behind and matched no branch, so the identical
    /// read was silent-pure. Conformance PART 19's rust fixture happens to use the qualified spelling.
    ///
    /// The dep's own MODULE path is part of the key it publishes (`<lazy>::cfg::MODC`), so both the
    /// crate-root and the module-scoped shapes are asked, under both spellings.
    #[test]
    fn a_chained_deps_lazy_static_is_charged_under_the_use_spelling_too() {
        let dep = std::env::temp_dir().join(format!("candor-uselazy-rep-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dep);
        let _ = std::fs::create_dir_all(&dep);
        let me = format!("scan-{}", env!("CARGO_PKG_VERSION"));
        // `C` at the dep's crate root, `cfg::MODC` inside one of its modules. `PURE_C` is DELIBERATELY
        // absent: a pure init publishes no unit, which is what keeps its readers pure.
        std::fs::write(dep.join("report.deplib.scan.json"), format!(r#"{{
            "candor": {{"version": "{me}", "toolchain": "stable", "spec": "0.23"}},
            "package": "deplib",
            "functions": [
              {{"fn": "<lazy>::C",         "inferred": ["Env"], "hash": "deplib#<lazy>::C"}},
              {{"fn": "<lazy>::cfg::MODC", "inferred": ["Fs"],  "hash": "deplib#<lazy>::cfg::MODC"}}
            ]}}"#)).unwrap();
        let idx = load_dep_reports(Some(dep.to_str().unwrap()));
        assert!(idx.crates.contains("deplib"));
        let d = std::env::temp_dir().join(format!("candor-uselazy-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d); // never read a stale report back as this arm's result
        std::fs::create_dir_all(d.join("src")).unwrap();
        std::fs::write(d.join("Cargo.toml"),
            "[package]\nname = \"app\"\n[dependencies]\ndeplib = \"1\"\n").unwrap();
        std::fs::write(d.join("src/lib.rs"), r#"
            use deplib::C;
            use deplib::cfg::MODC;
            use deplib::PURE_C;
            pub fn qualified() -> usize { deplib::C.len() }
            pub fn imported() -> usize { C.len() }
            pub fn imported_deref() -> usize { (*C).len() }
            pub fn body_use() -> usize { use deplib::C as D; D.len() }
            pub fn mod_qualified() -> usize { deplib::cfg::MODC.len() }
            pub fn mod_imported() -> usize { MODC.len() }
            // CONTROLS
            pub fn pure_dep_static() -> usize { PURE_C.len() }
            pub fn shadowed_by_an_untypable_let() -> usize { let C = "aa"; C.len() }
            "#).unwrap();
        let prefix = d.join("out/r").to_string_lossy().into_owned();
        let (rc, body) = scan_one(&d.to_string_lossy(), ScanOpts {
            prefix, want_json: true, include_tests: false, policy: None, baseline: None, ws_member: false, quiet: true, deps_idx: &idx, peek_excluded: false,
        }, &crate::gate::begin_run());
        assert_eq!(rc, 0);
        let body = body.expect("want_json returns the report body");
        let v: serde_json::Value = serde_json::from_str(&body).unwrap();
        let eff = |needle: &str| -> Vec<String> {
            v["functions"].as_array().into_iter().flatten()
                .filter(|f| f["fn"].as_str() == Some(needle))
                .flat_map(|f| f["inferred"].as_array().into_iter().flatten()
                    .filter_map(|e| e.as_str().map(String::from)).collect::<Vec<_>>())
                .collect()
        };
        for f in ["qualified", "imported", "imported_deref", "body_use"] {
            assert_eq!(eff(f), vec!["Env"],
                       "{f} forces the chained dep's crate-root lazy static — the spelling must not \
                        decide whether the initializer's Env is seen:\n{body}");
        }
        for f in ["mod_qualified", "mod_imported"] {
            assert_eq!(eff(f), vec!["Fs"],
                       "{f} forces a lazy static the dep declares inside a MODULE — its published key \
                        carries that module (`<lazy>::cfg::MODC`), so the consumer must ask for it:\n{body}");
        }
        assert!(eff("pure_dep_static").is_empty(),
                "a pure init publishes no unit; its reader stays pure (per-static keying):\n{body}");
        assert!(eff("shadowed_by_an_untypable_let").is_empty(),
                "an untypable `let` still shadows the import — the fabrication mirror:\n{body}");
        let _ = std::fs::remove_dir_all(&d);
        let _ = std::fs::remove_dir_all(&dep);
    }

    /// THE UNBOUND FACTORY. `let c = deplib::build(); c.fetch()` routes through `dep_bound_vars`, which
    /// only a `let` ever writes — so `deplib::build().fetch()`, the same call with the binding elided,
    /// matched no branch and read SILENT-PURE. Not an un-attempted precision gap: a hole in the shipped
    /// guard, against its own ruling that a key which could not be formed must never read pure.
    ///
    /// Every assertion is an EQUALITY between the two spellings, not a fixed expectation — the bound arm
    /// is the shipped oracle, and pinning "must be Unknown" would freeze whichever answer the join
    /// currently gives rather than the property that the binding is irrelevant. Both are checked with the
    /// dep's `typeSurface.returns` present (the pair RESOLVES) and absent (the pair DISCLOSES).
    #[test]
    fn an_unbound_dep_factory_receiver_answers_exactly_as_the_bound_one() {
        let me = format!("scan-{}", env!("CARGO_PKG_VERSION"));
        let write_dep = |tag: &str, surface: &str| -> DepIndex {
            let dep = std::env::temp_dir().join(format!("candor-unbound-{tag}-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&dep);
            let _ = std::fs::create_dir_all(&dep);
            std::fs::write(dep.join("report.deplib.scan.json"), format!(r#"{{
                "candor": {{"version": "{me}", "toolchain": "stable", "spec": "0.23"}},
                "package": "deplib",{surface}
                "functions": [{{"fn": "Client::fetch", "inferred": ["Fs"], "hash": "deplib#Client::fetch"}}]}}"#)).unwrap();
            let idx = load_dep_reports(Some(dep.to_str().unwrap()));
            let _ = std::fs::remove_dir_all(&dep);
            idx
        };
        const SRC: &str = r#"
            pub fn bound() -> String { let c = deplib::build(); c.fetch() }
            pub fn unbound() -> String { deplib::build().fetch() }
            pub fn bound_try() -> Result<String, ()> { let c = deplib::try_build()?; Ok(c.fetch()) }
            pub fn unbound_try() -> Result<String, ()> { Ok(deplib::try_build()?.fetch()) }
            pub async fn bound_await() -> String { let c = deplib::async_build().await; c.fetch() }
            pub async fn unbound_await() -> String { deplib::async_build().await.fetch() }
            // CONTROL: a module-qualified LOCAL factory. The marker is emitted the same way, and the
            // crate root is checked against the manifest's declared deps at consumption — so a local
            // module that merely looks crate-qualified must stay pure.
            pub mod localmod { pub struct L; impl L { pub fn calc(&self) -> u32 { 1 } } pub fn make() -> L { L } }
            pub fn local_factory() -> u32 { localmod::make().calc() }
        "#;
        let run = |tag: &str, deps_idx: &DepIndex| -> serde_json::Value {
            let d = std::env::temp_dir().join(format!("candor-unbound-app-{tag}-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&d);
            std::fs::create_dir_all(d.join("src")).unwrap();
            std::fs::write(d.join("Cargo.toml"),
                "[package]\nname = \"app\"\n[dependencies]\ndeplib = \"1\"\n").unwrap();
            std::fs::write(d.join("src/lib.rs"), SRC).unwrap();
            let prefix = d.join("out/r").to_string_lossy().into_owned();
            let (rc, body) = scan_one(&d.to_string_lossy(), ScanOpts {
                prefix, want_json: true, include_tests: false, policy: None, baseline: None,
                ws_member: false, quiet: true, deps_idx, peek_excluded: false,
            }, &crate::gate::begin_run());
            assert_eq!(rc, 0);
            let v: serde_json::Value = serde_json::from_str(&body.unwrap()).unwrap();
            let _ = std::fs::remove_dir_all(&d);
            v
        };
        let eff = |v: &serde_json::Value, needle: &str| -> Vec<String> {
            v["functions"].as_array().into_iter().flatten()
                .filter(|f| f["fn"].as_str() == Some(needle))
                .flat_map(|f| f["inferred"].as_array().into_iter().flatten()
                    .filter_map(|e| e.as_str().map(String::from)).collect::<Vec<_>>())
                .collect()
        };
        // (a) NO type surface — the receiver's type never travels, so neither spelling can form a key.
        let bare = run("bare", &write_dep("bare", ""));
        for (b, u) in [("bound", "unbound"), ("bound_try", "unbound_try"), ("bound_await", "unbound_await")] {
            assert_eq!(eff(&bare, b), eff(&bare, u),
                       "{u} must answer exactly as {b} — eliding the binding changes nothing about \
                        whether a key could be formed:\n{bare:#}");
            assert!(eff(&bare, u).contains(&"Unknown".to_string()),
                    "…and that answer is a DISCLOSURE, never silence:\n{bare:#}");
        }
        // (b) WITH the type surface — determination before disclosure: both spellings RESOLVE to Fs.
        let surf = run("surf", &write_dep("surf", r#"
                "typeSurface": {"returns": {"deplib#build": "deplib#Client",
                                            "deplib#try_build": "deplib#Client",
                                            "deplib#async_build": "deplib#Client"}},"#));
        for (b, u) in [("bound", "unbound"), ("bound_try", "unbound_try"), ("bound_await", "unbound_await")] {
            assert_eq!(eff(&surf, b), eff(&surf, u), "resolved arm must agree too:\n{surf:#}");
            assert_eq!(eff(&surf, u), vec!["Fs"],
                       "with the dep's return type published the key IS formable — {u} resolves:\n{surf:#}");
        }
        // MIRROR CONTROL — a local module's factory is not a dependency's.
        for v in [&bare, &surf] {
            assert!(eff(v, "local_factory").is_empty(),
                    "a module-qualified LOCAL factory must not disclose — the marker is inert unless \
                     its root is a declared dependency:\n{v:#}");
        }
    }

    #[test]
    fn lazy_static_deferred_init_is_charged_to_the_forcing_site() {
        // THE UNDER-REPORT: a LAZY/deferred static whose init does I/O has its effect reachable from NO
        // fn (the init thunk runs on first use). Before the fix the effect vanished and every forcing site
        // read silent-pure. The fix synthesizes a `<lazy>::NAME` unit (the thunk body) and edges each
        // forcing site to it. This test asserts all four idioms light up, a PURE init fabricates nothing,
        // and the keying is per-STATIC (not module-scoped) so a pure lazy's accessor stays pure even when
        // an effectful lazy sits in the same module.
        let d = std::env::temp_dir().join(format!("candor-scan-lazystatic-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(d.join("src")).unwrap();
        // `once_cell` / `lazy_static` are declared so the κ ledger treats them as known deps; the scan
        // never builds them — the idiom is recognised syntactically.
        std::fs::write(
            d.join("Cargo.toml"),
            "[package]\nname = \"lazystatic\"\n[dependencies]\nonce_cell = \"1\"\nlazy_static = \"1\"\n",
        )
        .unwrap();
        std::fs::write(
            d.join("src/lib.rs"),
            r#"
            use once_cell::sync::Lazy;
            use std::sync::LazyLock;
            use std::fs;
            use lazy_static::lazy_static;

            // 1. once_cell Lazy, effectful init
            pub static CFG: Lazy<String> = Lazy::new(|| fs::read_to_string("/etc/a").unwrap_or_default());
            // 2. std LazyLock, effectful init
            pub static CFG2: LazyLock<String> = LazyLock::new(|| fs::read_to_string("/etc/b").unwrap_or_default());
            // 3. lazy_static!, effectful init
            lazy_static! { pub static ref CFG3: String = fs::read_to_string("/etc/c").unwrap_or_default(); }
            // 4. thread_local!, effectful init
            thread_local! { pub static CFG4: String = fs::read_to_string("/etc/d").unwrap_or_default(); }

            // NO-FABRICATION CONTROLS: pure inits contribute nothing
            pub static PURE_NUM: Lazy<usize> = Lazy::new(|| 1 + 1);

            // forcing sites — each names exactly ONE static
            pub fn force1() -> bool { CFG.contains("x") }
            pub fn force2() -> bool { CFG2.contains("x") }
            pub fn force3() -> bool { CFG3.contains("x") }
            pub fn force4() -> bool { CFG4.with(|c| c.contains("x")) }
            // MULTI-STATIC SCOPING: this fn names only the PURE lazy — must stay pure even though
            // effectful lazies live in the same module (static-scoped, not module-scoped).
            pub fn force_pure() -> usize { *PURE_NUM + 5 }
            "#,
        )
        .unwrap();
        let idx = load_dep_reports(None);
        let prefix = d.join("out/r").to_string_lossy().into_owned();
        let (rc, body) = scan_one(&d.to_string_lossy(), ScanOpts {
            prefix, want_json: true, include_tests: false, policy: None, baseline: None, ws_member: false, quiet: true, deps_idx: &idx, peek_excluded: false,
        }, &crate::gate::begin_run());
        assert_eq!(rc, 0);
        let body = body.expect("want_json returns the report body");
        let v: serde_json::Value = serde_json::from_str(&body).unwrap();
        let effects_of = |needle: &str| -> Vec<String> {
            v["functions"].as_array().into_iter().flatten()
                .filter(|f| f["fn"].as_str() == Some(needle))
                .flat_map(|f| f["inferred"].as_array().into_iter().flatten()
                    .filter_map(|e| e.as_str().map(String::from)).collect::<Vec<_>>())
                .collect()
        };
        // THE FIX: all four idioms' forcing sites carry Fs (the effect no longer vanishes).
        for (f, idiom) in [("force1", "once_cell Lazy"), ("force2", "std LazyLock"),
                           ("force3", "lazy_static!"), ("force4", "thread_local!")] {
            assert!(effects_of(f).contains(&"Fs".to_string()),
                    "{idiom}: forcing site `{f}` must carry Fs (deferred init under-report):\n{body}");
        }
        // NO-FABRICATION: a pure-init lazy's forcing site stays pure (absent from the effectful report).
        // Also proves MULTI-STATIC scoping — `force_pure` names only PURE_NUM, so the sibling effectful
        // lazies in the same module must NOT bleed into it.
        assert!(effects_of("force_pure").is_empty(),
                "a pure-init lazy's forcing site must stay pure (no fabrication / no module-bleed):\n{body}");
        // The pure lazy's synthetic unit, if present at all, must carry no effect (it's dropped from the
        // effectful report — assert it never appears WITH an effect).
        let pure_unit_effectful = v["functions"].as_array().into_iter().flatten().any(|f| {
            f["fn"].as_str() == Some("<lazy>::PURE_NUM")
                && f["inferred"].as_array().is_some_and(|a| !a.is_empty())
        });
        assert!(!pure_unit_effectful, "the pure lazy's synthetic unit must carry no effect:\n{body}");
        let _ = std::fs::remove_dir_all(&d);
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
        let (mut fe, mut ev) = (FieldElemIndex::new(), HashMap::new());
        let mut fet = FieldElemTraitIndex::new();
        collect_decls(&file.items, false, &mut uses, &mut fields, &mut fe, &mut fet, &mut rets, &mut ev, &mut std::collections::HashMap::new(), &mut ti, &mut td, &mut tf, &mut std::collections::HashSet::new(), &mut std::collections::HashSet::new(), &mut std::collections::HashSet::new(), &mut std::collections::HashMap::new(), &mut std::collections::HashSet::new(), &mut std::collections::HashMap::new(), &mut std::collections::HashMap::new(), &mut std::collections::HashMap::new(), &mut std::collections::HashSet::new());
        assert_eq!(fields["Outer"]["0"], "Inner");
        assert_eq!(fields["Stack"]["0"], "Outer");
    }

    /// Run the FULL pipeline (Pass A indexes + Pass B collection, with the same wiring as `scan_one`)
    /// over a source string and return `fn-qual -> the typed `Type::method` call paths it produced`.
    /// A receiver typed by one of the new idioms shows up as `Sender::send` here; a dropped receiver
    /// would leave only the bare leaf `send` (no `Type::` qualifier) — the silent-under-report shape.
    fn typed_calls_of(src: &str) -> HashMap<String, Vec<String>> {
        let file: syn::File = syn::parse_str(src).unwrap();
        let mut uses = HashMap::new();
        let mut fields = FieldIndex::new();
        let mut field_elem = FieldElemIndex::new();
        let mut field_elem_trait = FieldElemTraitIndex::new();
        let mut rets: HashMap<String, Option<String>> = HashMap::new();
        let mut enum_tmp: HashMap<String, Option<String>> = HashMap::new();
        let mut enum_variant_traits_tmp: HashMap<String, Option<Vec<String>>> = HashMap::new();
        let mut ti = TraitImplIndex::new();
        let mut td: HashMap<String, LocalTrait> = HashMap::new();
        let mut tf = TraitFieldIndex::new();
        collect_decls(&file.items, false, &mut uses, &mut fields, &mut field_elem, &mut field_elem_trait, &mut rets,
                      &mut enum_tmp, &mut enum_variant_traits_tmp, &mut ti, &mut td, &mut tf, &mut std::collections::HashSet::new(),
                      &mut std::collections::HashSet::new(), &mut std::collections::HashSet::new(), &mut std::collections::HashMap::new(), &mut std::collections::HashSet::new(), &mut std::collections::HashMap::new(), &mut std::collections::HashMap::new(), &mut std::collections::HashMap::new(), &mut std::collections::HashSet::new());
        let returns: ReturnIndex = rets.into_iter().filter_map(|(k, v)| v.map(|t| (k, t))).collect();
        let mut enum_variants: EnumVariantIndex =
            enum_tmp.into_iter().filter_map(|(k, v)| v.map(|t| (k, t))).collect();
        let mut enum_variant_traits: EnumVariantTraitIndex =
            enum_variant_traits_tmp.into_iter().filter_map(|(k, v)| v.map(|t| (k, t))).collect();
        let ambiguous_enum_leaves = drop_cross_ambiguous_enum_leaves(&mut enum_variants, &mut enum_variant_traits);
        let traits = TraitIndexes { impls: &ti, decls: &td, fields: &tf };
        let elems = ElemIndexes { field_elem: &field_elem, field_elem_trait: &field_elem_trait, enum_variants: &enum_variants, enum_variant_traits: &enum_variant_traits, ambiguous_enum_leaves: &ambiguous_enum_leaves, callable_statics: &std::collections::HashSet::new() };
        let mut fns: Vec<FnInfo> = Vec::new();
        let mut us2 = HashMap::new();
        let mut locs = Vec::new();
        fn_locs(&file.items, "lib.rs", false, &mut locs);
        let mut loc_idx = 0usize;
        scan_items(&file.items, "", &locs, &mut loc_idx, false, &fields, &returns, traits, elems, &std::collections::HashSet::new(), &std::collections::HashMap::new(), &std::collections::HashMap::new(), &std::collections::HashSet::new(), &mut us2, &mut fns);
        fns.into_iter()
            .map(|f| (f.qual, f.calls.into_iter().filter(|c| c.typed).map(|c| c.path).collect()))
            .collect()
    }

    /// fn-name -> `unresolved` flag, through the same full pipeline — for the opacity/callback tests.
    fn unresolved_of(src: &str) -> HashMap<String, bool> {
        let file: syn::File = syn::parse_str(src).unwrap();
        let mut uses = HashMap::new();
        let mut fields = FieldIndex::new();
        let mut field_elem = FieldElemIndex::new();
        let mut field_elem_trait = FieldElemTraitIndex::new();
        let mut rets: HashMap<String, Option<String>> = HashMap::new();
        let mut enum_tmp: HashMap<String, Option<String>> = HashMap::new();
        let mut enum_variant_traits_tmp: HashMap<String, Option<Vec<String>>> = HashMap::new();
        let mut ti = TraitImplIndex::new();
        let mut td: HashMap<String, LocalTrait> = HashMap::new();
        let mut tf = TraitFieldIndex::new();
        collect_decls(&file.items, false, &mut uses, &mut fields, &mut field_elem, &mut field_elem_trait, &mut rets,
                      &mut enum_tmp, &mut enum_variant_traits_tmp, &mut ti, &mut td, &mut tf, &mut std::collections::HashSet::new(),
                      &mut std::collections::HashSet::new(), &mut std::collections::HashSet::new(), &mut std::collections::HashMap::new(), &mut std::collections::HashSet::new(), &mut std::collections::HashMap::new(), &mut std::collections::HashMap::new(), &mut std::collections::HashMap::new(), &mut std::collections::HashSet::new());
        let returns: ReturnIndex = rets.into_iter().filter_map(|(k, v)| v.map(|t| (k, t))).collect();
        let mut enum_variants: EnumVariantIndex =
            enum_tmp.into_iter().filter_map(|(k, v)| v.map(|t| (k, t))).collect();
        let mut enum_variant_traits: EnumVariantTraitIndex =
            enum_variant_traits_tmp.into_iter().filter_map(|(k, v)| v.map(|t| (k, t))).collect();
        let ambiguous_enum_leaves = drop_cross_ambiguous_enum_leaves(&mut enum_variants, &mut enum_variant_traits);
        let traits = TraitIndexes { impls: &ti, decls: &td, fields: &tf };
        let elems = ElemIndexes { field_elem: &field_elem, field_elem_trait: &field_elem_trait, enum_variants: &enum_variants, enum_variant_traits: &enum_variant_traits, ambiguous_enum_leaves: &ambiguous_enum_leaves, callable_statics: &std::collections::HashSet::new() };
        let mut fns: Vec<FnInfo> = Vec::new();
        let mut us2 = HashMap::new();
        let mut locs = Vec::new();
        fn_locs(&file.items, "lib.rs", false, &mut locs);
        let mut loc_idx = 0usize;
        scan_items(&file.items, "", &locs, &mut loc_idx, false, &fields, &returns, traits, elems, &std::collections::HashSet::new(), &std::collections::HashMap::new(), &std::collections::HashMap::new(), &std::collections::HashSet::new(), &mut us2, &mut fns);
        fns.into_iter().map(|f| (f.qual, f.unresolved)).collect()
    }

    /// fn-qual -> its `loc` (`file:line:col`), through the same full pipeline — for the loc-fidelity test.
    fn locs_of(src: &str) -> HashMap<String, String> {
        let file: syn::File = syn::parse_str(src).unwrap();
        let mut uses = HashMap::new();
        let mut fields = FieldIndex::new();
        let mut field_elem = FieldElemIndex::new();
        let mut field_elem_trait = FieldElemTraitIndex::new();
        let mut rets: HashMap<String, Option<String>> = HashMap::new();
        let mut enum_tmp: HashMap<String, Option<String>> = HashMap::new();
        let mut enum_variant_traits_tmp: HashMap<String, Option<Vec<String>>> = HashMap::new();
        let mut ti = TraitImplIndex::new();
        let mut td: HashMap<String, LocalTrait> = HashMap::new();
        let mut tf = TraitFieldIndex::new();
        collect_decls(&file.items, false, &mut uses, &mut fields, &mut field_elem, &mut field_elem_trait, &mut rets,
                      &mut enum_tmp, &mut enum_variant_traits_tmp, &mut ti, &mut td, &mut tf, &mut std::collections::HashSet::new(),
                      &mut std::collections::HashSet::new(), &mut std::collections::HashSet::new(), &mut std::collections::HashMap::new(), &mut std::collections::HashSet::new(), &mut std::collections::HashMap::new(), &mut std::collections::HashMap::new(), &mut std::collections::HashMap::new(), &mut std::collections::HashSet::new());
        let returns: ReturnIndex = rets.into_iter().filter_map(|(k, v)| v.map(|t| (k, t))).collect();
        let mut enum_variants: EnumVariantIndex =
            enum_tmp.into_iter().filter_map(|(k, v)| v.map(|t| (k, t))).collect();
        let mut enum_variant_traits: EnumVariantTraitIndex =
            enum_variant_traits_tmp.into_iter().filter_map(|(k, v)| v.map(|t| (k, t))).collect();
        let ambiguous_enum_leaves = drop_cross_ambiguous_enum_leaves(&mut enum_variants, &mut enum_variant_traits);
        let traits = TraitIndexes { impls: &ti, decls: &td, fields: &tf };
        let elems = ElemIndexes { field_elem: &field_elem, field_elem_trait: &field_elem_trait, enum_variants: &enum_variants, enum_variant_traits: &enum_variant_traits, ambiguous_enum_leaves: &ambiguous_enum_leaves, callable_statics: &std::collections::HashSet::new() };
        let mut fns: Vec<FnInfo> = Vec::new();
        let mut us2 = HashMap::new();
        let mut locs = Vec::new();
        fn_locs(&file.items, "lib.rs", false, &mut locs);
        let mut loc_idx = 0usize;
        scan_items(&file.items, "", &locs, &mut loc_idx, false, &fields, &returns, traits, elems, &std::collections::HashSet::new(), &std::collections::HashMap::new(), &std::collections::HashMap::new(), &std::collections::HashSet::new(), &mut us2, &mut fns);
        fns.into_iter().map(|f| (f.qual, f.loc)).collect()
    }

    /// The `loc` field MUST be `file:line:col` (spec report schema), with the line being the fn's ACTUAL
    /// source line — not the file-only `src/lib.rs` an adversarial cross-engine fidelity review found.
    /// Line is 1-based; column is 1-based (proc-macro2's 0-based column + 1), pointing at the item's first
    /// token, so a top-level `fn` at column 0 reads col 1 (matching the deep engine's `build.rs:10:1`). The
    /// walk covers free fns, impl methods, nested-module fns, and trait default methods — the same set
    /// `scan_items` emits — so every loc lines up with its FnInfo.
    #[test]
    fn loc_carries_actual_line_and_col() {
        // Lines (1-based): blank=1, alpha=2, beta=4(`pub` indented?), … keep it explicit below.
        let src = "\
fn alpha() {}
    fn beta() {}
struct T;
impl T {
    fn gamma(&self) {}
}
mod inner {
    fn delta() {}
}
trait G {
    fn hello(&self) {}
}
";
        let m = locs_of(src);
        // alpha: line 1, first token `fn` at column 0 -> 1-based col 1.
        assert_eq!(m["alpha"], "lib.rs:1:1");
        // beta: line 2, indented 4 -> col 5 (1-based).
        assert_eq!(m["beta"], "lib.rs:2:5");
        // gamma: line 5 (inside impl), indented 4 -> col 5.
        assert_eq!(m["T::gamma"], "lib.rs:5:5");
        // delta: line 8 (inside `mod inner`), indented 4 -> col 5; qualified by the module path.
        assert_eq!(m["inner::delta"], "lib.rs:8:5");
        // hello: a trait DEFAULT method, line 11, indented 4 -> col 5.
        assert_eq!(m["G::hello"], "lib.rs:11:5");
    }

    /// Invoking a fn-typed binding (`cb: fn()`/`impl Fn`/`dyn Fn`/generic `F: Fn`/`Box<dyn Fn>`) calls a
    /// body the syntactic scan can't see, so the fn is `unresolved` (honest Unknown) — NOT silently pure.
    /// Found by the cross-engine generative differential: candor-scan dropped these while java/ts/swift
    /// propagated/Unknowned them. A normal free-fn call must NOT be flagged unresolved (no over-report).
    #[test]
    fn fn_typed_callback_invocation_is_unresolved() {
        for hof in [
            "fn h(cb: fn()) { cb(); }",
            "fn h(cb: impl Fn()) { cb(); }",
            "fn h<F: Fn()>(cb: F) { cb(); }",
            "fn h(cb: &dyn Fn()) { cb(); }",
            "fn h(cb: Box<dyn Fn()>) { cb(); }",
        ] {
            let m = unresolved_of(hof);
            assert!(m["h"], "fn-typed callback invocation silently dropped (not unresolved): {hof}");
        }
        // A fn-typed binding REBOUND to a local must still be Unknown when invoked (the max review
        // found the param-only seeding missed `let g = cb; g()`). Covers a plain rebind, an `if`-yield,
        // and an annotated `let g: fn()`.
        for hof in [
            "fn h(cb: impl Fn()) { let g = cb; g(); }",
            "fn h(cb: fn()) { let g = if true { cb } else { return }; g(); }",
            "fn s() {} fn h() { let g: fn() = s; g(); }",
        ] {
            let m = unresolved_of(hof);
            assert!(m["h"], "fn-typed callback rebound to a local silently dropped: {hof}");
        }
        // NO over-report: a normal free-fn call, AND a normal value rebind, stay resolved.
        let m = unresolved_of("fn helper() {} fn caller() { helper(); }");
        assert!(!m["caller"], "a normal free-fn call must not be flagged unresolved");
        let m = unresolved_of("struct T; impl T { fn m(&self) {} } fn f() { let x = T; let y = x; y.m(); }");
        assert!(!m["f"], "a normal value rebind must not be flagged unresolved");
    }

    /// CARDINAL SIN, reproduced and closed: a function pointer resolved at RUNTIME (a dynamically-loaded
    /// library symbol, or a raw `dlopen`/`dlsym` pointer) and then INVOKED read silent-pure. The same
    /// invocation machinery `fn_typed_callback_invocation_is_unresolved` covers for a `fn()`/`impl Fn`
    /// annotation missed two shapes that never spell one of those recognised forms:
    ///
    /// 1. `libloading::Symbol<T>` — a NAMED wrapper type `is_callable_type` didn't peel, so a `let` typed
    ///    with it (the ordinary way to use `Library::get`) was not recorded as fn-typed at all, and the
    ///    later call resolved as a phantom free-fn and was dropped.
    /// 2. `std::mem::transmute::<Src, Dst>(ptr)` into an UNTYPED `let` — `Dst` is the caller's target
    ///    type, but it lives in the CALL's turbofish, not a `Pat::Type` annotation, so nothing read it.
    ///
    /// Both are the honest `Unknown` (never a fabricated effect): rust-scan cannot know what the runtime
    /// symbol actually does, only that the code calls through a boundary it cannot see. This is a NAME
    /// match (`Symbol`, `transmute`), not a type-resolved one — rust-deep (rustc-typed) does not need it.
    #[test]
    fn runtime_resolved_pointer_invocation_is_unresolved() {
        // Shape 1: `libloading::Symbol<T>` (and the `os::unix`/`os::windows` twin, same leaf) as an
        // explicit `let` annotation — the "TYPED local" the bug report describes as reading silent-pure
        // despite having a type annotation, because that annotation's wrapper wasn't recognised.
        for src in [
            r#"unsafe fn run(lib: libloading::Library) {
                 let func: libloading::Symbol<unsafe extern "C" fn(i32) -> i32> = lib.get(b"f").unwrap();
                 func(5);
               }"#,
            r#"unsafe fn run(lib: libloading::Library) {
                 let func: libloading::os::unix::Symbol<extern "C" fn()> = lib.get(b"f").unwrap();
                 func();
               }"#,
            // A boxed/shared bare fn pointer — the same wrapper-peeling closes this identical
            // (independently unnoticed) hole for `Box`/`Rc`/`Arc<fn()>`. All three are asserted, not just
            // `Box`: the doc comment and `is_callable_type`'s match arm name all three symmetrically, but
            // ⟨2026-08-29, verifying the corpus-attack brief's "worth attacking" item⟩ found only `Box`
            // had a fixture — `Rc`/`Arc` were sound by code inspection only, never independently measured.
            "fn run(b: Box<fn()>) { b(); }",
            "fn run(b: std::rc::Rc<fn()>) { b(); }",
            "fn run(b: std::sync::Arc<fn()>) { b(); }",
        ] {
            let m = unresolved_of(src);
            assert!(m["run"], "runtime-resolved pointer through a named wrapper type silently dropped: {src}");
        }
        // Shape 2: `transmute::<_, Dst>(ptr)` into an UNTYPED let — the libc `dlopen`+`dlsym` shape.
        for src in [
            r#"unsafe fn run(sym: *mut std::ffi::c_void) {
                 let func = std::mem::transmute::<_, unsafe extern "C" fn(i32) -> i32>(sym);
                 func(5);
               }"#,
            // `use`-imported bare `transmute` — matched by leaf name, not the fully-qualified path.
            r#"use std::mem::transmute;
               unsafe fn run(sym: *mut std::ffi::c_void) {
                 let func = transmute::<_, extern "C" fn()>(sym);
                 func();
               }"#,
            // The libloading twin of shape 2: an UNTYPED `let` relying on `.get::<T>()`'s own turbofish
            // (found sweeping the same class) rather than a `let`-side annotation.
            r#"unsafe fn run(lib: libloading::Library) {
                 let func = lib.get::<unsafe extern "C" fn(i32) -> i32>(b"f").unwrap();
                 func(5);
               }"#,
        ] {
            let m = unresolved_of(src);
            assert!(m["run"], "runtime-resolved pointer via untyped transmute/turbofish silently dropped: {src}");
        }
        // ALREADY-CORRECT controls (must not regress): a `fn()`-typed local built via `transmute`, and
        // the transmute-and-call fused into one expression with no local at all.
        for src in [
            r#"unsafe fn run(sym: *mut std::ffi::c_void) {
                 let func: unsafe extern "C" fn(i32) -> i32 = std::mem::transmute(sym);
                 func(5);
               }"#,
            r#"unsafe fn run(sym: *mut std::ffi::c_void) {
                 std::mem::transmute::<_, extern "C" fn()>(sym)();
               }"#,
        ] {
            let m = unresolved_of(src);
            assert!(m["run"], "an already-correct shape regressed: {src}");
        }
        // OVER-CHARGE CONTROL: a pointer OBTAINED but never called must stay quiet — marking a binding
        // fn-typed changes nothing unless it is actually invoked with call syntax.
        for src in [
            r#"unsafe fn run(lib: libloading::Library) {
                 let _func: libloading::Symbol<unsafe extern "C" fn(i32) -> i32> = lib.get(b"f").unwrap();
               }"#,
            r#"unsafe fn run(sym: *mut std::ffi::c_void) {
                 let _func = std::mem::transmute::<_, extern "C" fn()>(sym);
               }"#,
            "fn run(_b: Box<fn()>) {}",
            "fn run(_b: std::rc::Rc<fn()>) {}",
            "fn run(_b: std::sync::Arc<fn()>) {}",
        ] {
            let m = unresolved_of(src);
            assert!(!m["run"], "a runtime-resolved pointer never called must NOT be flagged (over-charge): {src}");
        }
    }

    /// A `#[cfg(test)]` free fn / impl block / impl method at module scope is test-only and must NOT
    /// appear in the default (non-`--include-tests`) report — the guard was on `mod` only.
    #[test]
    fn cfg_test_items_excluded_from_default_report() {
        let m = typed_calls_of(
            "pub fn prod() {}\n\
             #[cfg(test)] fn freefn() {}\n\
             struct S;\n\
             #[cfg(test)] impl S { fn blk(&self) {} }\n\
             struct P; impl P { #[cfg(test)] fn meth(&self) {} fn keep(&self) {} }",
        );
        assert!(m.contains_key("prod"), "a production fn must be in the report");
        assert!(m.keys().any(|k| k.ends_with("P::keep")), "a production method must be in the report");
        for leaked in ["freefn", "S::blk", "P::meth"] {
            assert!(!m.keys().any(|k| k.ends_with(leaked)), "a #[cfg(test)] item leaked: {leaked}");
        }
    }

    /// Each of the six PROVEN-dropped idioms (the silent-under-report bug): a method call whose
    /// receiver is reached via for-loop / iterator-adapter closure / subscript / nested field+subscript
    /// / enum-payload match / tuple destructure. The effectful `Sender::send` must be TYPED (so it
    /// classifies Net), mirroring the candor-swift sweep that fixed the same six.
    #[test]
    fn dropped_receiver_idioms_now_resolve() {
        let prelude = "struct Sender; impl Sender { fn send(&self) {} }\n\
                       struct Pool { senders: Vec<Sender> }\n\
                       enum Conn { Active(Sender), Idle }\n";
        let cases: &[(&str, &str)] = &[
            // 1. for-loop over a Vec<Sender> param
            ("fn f(xs: Vec<Sender>) { for c in xs { c.send(); } }", "f"),
            // 1b. for-loop over a &[Sender] param
            ("fn f(xs: &[Sender]) { for c in xs { c.send(); } }", "f"),
            // 2. iterator-adapter closure (for_each / map)
            ("fn f(xs: Vec<Sender>) { xs.iter().for_each(|c| c.send()); }", "f"),
            ("fn f(xs: Vec<Sender>) { let _ = xs.iter().map(|c| c.send()).count(); }", "f"),
            // 3. subscript
            ("fn f(xs: Vec<Sender>) { xs[0].send(); }", "f"),
            // 6. tuple destructure from a tuple-typed param
            ("fn f(p: (Sender, usize)) { let (s, _) = p; s.send(); }", "f"),
        ];
        for (body, fnname) in cases {
            let src = format!("{prelude}{body}");
            let m = typed_calls_of(&src);
            let calls = m.get(*fnname).cloned().unwrap_or_default();
            assert!(
                calls.iter().any(|c| c == "Sender::send"),
                "idiom dropped the effectful receiver (silent under-report): {body}\n  typed calls: {calls:?}"
            );
        }
        // nested field + subscript (`self.senders[0].send()`) and for-loop over a collection FIELD.
        let m = typed_calls_of(&format!(
            "{prelude}impl Pool {{ fn first(&self) {{ self.senders[0].send(); }} \
             fn each(&self) {{ for c in &self.senders {{ c.send(); }} }} }}"
        ));
        assert!(m["Pool::first"].iter().any(|c| c == "Sender::send"), "nested field+subscript dropped: {:?}", m["Pool::first"]);
        assert!(m["Pool::each"].iter().any(|c| c == "Sender::send"), "for-loop over field dropped: {:?}", m["Pool::each"]);
        // 5. enum-payload match binding (`Conn::Active(s) => s.send()`).
        let m = typed_calls_of(&format!(
            "{prelude}fn g(c: Conn) {{ match c {{ Conn::Active(s) => s.send(), Conn::Idle => {{}} }} }}"
        ));
        assert!(m["g"].iter().any(|c| c == "Sender::send"), "enum-payload match binding dropped: {:?}", m["g"]);
    }

    /// NO FABRICATION (the precision failure): the same six idioms over a PURE element type, or over an
    /// effect-irrelevant element, must NOT type a `Type::send` edge to anything effectful — the element
    /// is pure, so the receiver typing must stay honest. We assert the effectful `Sender::send` never
    /// appears; a pure `Pure::send` edge is fine (it classifies to nothing).
    #[test]
    fn idioms_never_fabricate_on_pure_elements() {
        let prelude = "struct Pure; impl Pure { fn send(&self) {} }\n\
                       struct Bag { items: Vec<Pure> }\n";
        let bodies: &[&str] = &[
            "fn f(xs: Vec<Pure>) { for c in xs { c.send(); } }",
            "fn f(xs: Vec<Pure>) { xs.iter().for_each(|c| c.send()); }",
            "fn f(xs: Vec<Pure>) { xs[0].send(); }",
            "fn f(xs: Vec<i32>) { for c in xs { let _ = c + 1; } }",
            "fn f(p: (Pure, usize)) { let (s, _) = p; s.send(); }",
        ];
        for body in bodies {
            let m = typed_calls_of(&format!("{prelude}{body}"));
            let calls = m.get("f").cloned().unwrap_or_default();
            // the ONLY typed edge a pure element may form is `Pure::send` — never an effectful type's.
            assert!(
                calls.iter().all(|c| c != "Sender::send" && !c.contains("TcpStream")),
                "fabricated an effectful edge on a pure element: {body}\n  typed: {calls:?}"
            );
        }
    }

    /// The candor-swift `vars`-leak lesson: a scoped binding (loop var, closure param, match payload)
    /// must NOT leak past its block into a later same-named, uninferable var. Here the first loop binds
    /// `c: Sender`; the second loop's `c` (a Pure) and a trailing free `c.send()` on an indeterminate
    /// `c` must NOT inherit `Sender` and fabricate its edge.
    #[test]
    fn scoped_bindings_do_not_leak() {
        let prelude = "struct Sender; impl Sender { fn send(&self) {} }\n\
                       struct Pure; impl Pure { fn send(&self) {} }\n\
                       fn mk() -> Pure { Pure }\n";
        // second loop over Pure: `c` must be re-typed Pure, not the prior Sender.
        let m = typed_calls_of(&format!(
            "{prelude}fn f(xs: Vec<Sender>, ys: Vec<Pure>) {{ for c in xs {{ c.send(); }} for c in ys {{ c.send(); }} }}"
        ));
        let calls = &m["f"];
        // exactly ONE Sender::send (the genuine first loop); the second loop's `c.send()` is Pure::send.
        assert_eq!(
            calls.iter().filter(|c| *c == "Sender::send").count(),
            1,
            "loop binding leaked into the next same-named loop (fabrication): {calls:?}"
        );
        // a closure param binding must not leak to a later free var of the same name.
        let m = typed_calls_of(&format!(
            "{prelude}fn f(xs: Vec<Sender>) {{ xs.iter().for_each(|c| c.send()); let c = mk(); c.send(); }}"
        ));
        let calls = &m["f"];
        assert!(
            !calls.iter().any(|c| *c == "Sender::send" && calls.iter().filter(|x| *x == "Sender::send").count() > 1),
            "closure param leaked"
        );
        // the trailing `let c = mk(); c.send()` must be Pure::send, never Sender::send.
        assert_eq!(calls.iter().filter(|c| *c == "Sender::send").count(), 1,
                   "closure param binding leaked into a later same-named var: {calls:?}");
        assert!(calls.iter().any(|c| c == "Pure::send"), "later c should type Pure::send: {calls:?}");
    }

    /// `elem_type` extracts T from every supported collection shape and resolves it through `uses`;
    /// returns None for non-collections (so a non-collection receiver is never mis-typed as an element).
    #[test]
    fn elem_type_covers_the_collection_shapes() {
        let u = uses(&[("Sender", "net::Sender")]);
        let p = |s: &str| -> Option<String> {
            let t: syn::Type = syn::parse_str(s).unwrap();
            elem_type(&t, &u)
        };
        assert_eq!(p("Vec<Sender>").as_deref(), Some("net::Sender"));
        assert_eq!(p("&[Sender]").as_deref(), Some("net::Sender"));
        assert_eq!(p("[Sender; 4]").as_deref(), Some("net::Sender"));
        assert_eq!(p("HashSet<Sender>").as_deref(), Some("net::Sender"));
        assert_eq!(p("BTreeSet<Sender>").as_deref(), Some("net::Sender"));
        assert_eq!(p("VecDeque<Sender>").as_deref(), Some("net::Sender"));
        assert_eq!(p("Box<[Sender]>").as_deref(), Some("net::Sender"));
        assert_eq!(p("Arc<Vec<Sender>>").as_deref(), Some("net::Sender"));
        // a non-collection is NOT an element source
        assert_eq!(p("Sender"), None);
        assert_eq!(p("Option<Sender>"), None);
        // a map's value carries the element only via `.values()` (not the bare type here)
        assert_eq!(p("HashMap<String, Sender>"), None);
    }

    /// The Pass-A enum-variant index keeps only UNAMBIGUOUS single-payload variant leaves; a leaf two
    /// enums share with different payloads is dropped (never guess), like the return-index rule.
    #[test]
    fn enum_variant_index_drops_ambiguous_leaves() {
        let src = "enum A { One(i32), Pair(i32, i32), Unit }\n\
                   enum B { Two(String) }\n\
                   enum C { Two(Vec<u8>) }\n"; // `Two` conflicts across B and C → ambiguous, dropped
        let file: syn::File = syn::parse_str(src).unwrap();
        let mut uses = HashMap::new();
        let mut fields = FieldIndex::new();
        let mut field_elem = FieldElemIndex::new();
        let mut field_elem_trait = FieldElemTraitIndex::new();
        let mut rets: HashMap<String, Option<String>> = HashMap::new();
        let mut enum_tmp: HashMap<String, Option<String>> = HashMap::new();
        let (mut ti, mut td, mut tf) = (TraitImplIndex::new(), HashMap::new(), TraitFieldIndex::new());
        collect_decls(&file.items, false, &mut uses, &mut fields, &mut field_elem, &mut field_elem_trait, &mut rets,
                      &mut enum_tmp, &mut std::collections::HashMap::new(), &mut ti, &mut td, &mut tf, &mut std::collections::HashSet::new(),
                      &mut std::collections::HashSet::new(), &mut std::collections::HashSet::new(), &mut std::collections::HashMap::new(), &mut std::collections::HashSet::new(), &mut std::collections::HashMap::new(), &mut std::collections::HashMap::new(), &mut std::collections::HashMap::new(), &mut std::collections::HashSet::new());
        let ev: EnumVariantIndex = enum_tmp.into_iter().filter_map(|(k, v)| v.map(|t| (k, t))).collect();
        assert_eq!(ev.get("One").map(String::as_str), Some("i32")); // single-payload: kept
        assert_eq!(ev.get("Pair"), None);                           // multi-field: not indexed
        assert_eq!(ev.get("Unit"), None);                           // unit variant: not indexed
        assert_eq!(ev.get("Two"), None);                            // conflicting payloads: dropped
    }

    /// R77 Pass-A: the `enum_variant_traits` twin of the test above — a DISPATCH-typed (`dyn`/`impl`/
    /// bounded-generic) single-field payload records its trait leaves, unambiguous ones only.
    #[test]
    fn enum_variant_trait_index_records_dispatch_leaves_and_drops_ambiguous() {
        let src = "enum A { Cb(Box<dyn Fn()>), Pair(i32, i32), Unit, Plain(String) }\n\
                   trait Greeter { fn greet(&self); }\n\
                   enum B { Hi(Box<dyn Greeter>) }\n\
                   enum C { Hi(Box<dyn std::fmt::Debug>) }\n"; // `Hi` conflicts across B and C → ambiguous
        let file: syn::File = syn::parse_str(src).unwrap();
        let mut uses = HashMap::new();
        let mut fields = FieldIndex::new();
        let mut field_elem = FieldElemIndex::new();
        let mut field_elem_trait = FieldElemTraitIndex::new();
        let mut rets: HashMap<String, Option<String>> = HashMap::new();
        let mut enum_tmp: HashMap<String, Option<String>> = HashMap::new();
        let mut enum_variant_traits_tmp: HashMap<String, Option<Vec<String>>> = HashMap::new();
        let (mut ti, mut td, mut tf) = (TraitImplIndex::new(), HashMap::new(), TraitFieldIndex::new());
        collect_decls(&file.items, false, &mut uses, &mut fields, &mut field_elem, &mut field_elem_trait, &mut rets,
                      &mut enum_tmp, &mut enum_variant_traits_tmp, &mut ti, &mut td, &mut tf, &mut std::collections::HashSet::new(),
                      &mut std::collections::HashSet::new(), &mut std::collections::HashSet::new(), &mut std::collections::HashMap::new(), &mut std::collections::HashSet::new(), &mut std::collections::HashMap::new(), &mut std::collections::HashMap::new(), &mut std::collections::HashMap::new(), &mut std::collections::HashSet::new());
        let evt: EnumVariantTraitIndex =
            enum_variant_traits_tmp.into_iter().filter_map(|(k, v)| v.map(|t| (k, t))).collect();
        assert_eq!(evt.get("Cb"), Some(&vec!["Fn".to_string()]), "dyn Fn() payload must record the Fn leaf");
        assert_eq!(evt.get("Plain"), None, "a concrete String payload has no dispatch leaves");
        assert_eq!(evt.get("Pair"), None, "multi-field variant: not indexed");
        assert_eq!(evt.get("Unit"), None, "unit variant: not indexed");
        assert_eq!(evt.get("Hi"), None, "conflicting trait leaves across B/C: dropped, never guess");
        // The two indexes are mutually exclusive per leaf: `Cb` is dispatch-typed (present here) and
        // therefore absent from `enum_tmp`/`EnumVariantIndex` (its `type_path` is None for a dyn type).
        let ev: EnumVariantIndex = enum_tmp.into_iter().filter_map(|(k, v)| v.map(|t| (k, t))).collect();
        assert_eq!(ev.get("Cb"), None, "a dyn payload must not ALSO appear in the plain-type index");
    }

    /// R77 — SOUNDNESS.md: `match m { Msg::Cb(f) => f() }` silently dropped a callable enum-variant
    /// payload (`visit_arm` never consulted the dispatch-leaves route the other four binder families
    /// use). Ground truth: `Cb`'s payload IS called with call syntax, so the function must read
    /// `Unknown` (an opaque invocation, not a phantom free-fn drop), never silent-pure.
    #[test]
    fn r77_enum_tuple_variant_callable_match_arm_reads_unknown_not_silent_pure() {
        let src = "enum Msg { Cb(Box<dyn Fn()>) }\n\
                   fn f(m: Msg) { match m { Msg::Cb(cb) => cb() } }\n";
        let unres = unresolved_of(src);
        assert_eq!(unres.get("f"), Some(&true), "callable tuple-variant match-arm payload must read Unknown: {unres:?}");
    }

    /// R77 — the if-let TWIN of the match-arm case, explicitly named as a separate open shape in
    /// SOUNDNESS.md R77 (`visit_expr_if` only ever handled `Some`/`Ok`, never a general local variant).
    #[test]
    fn r77_enum_tuple_variant_callable_iflet_reads_unknown() {
        let src = "enum Msg { Cb(Box<dyn Fn()>) }\n\
                   fn f(m: Msg) { if let Msg::Cb(cb) = m { cb() } }\n";
        let unres = unresolved_of(src);
        assert_eq!(unres.get("f"), Some(&true), "callable tuple-variant if-let payload must read Unknown: {unres:?}");
    }

    /// R77 — the while-let form of the same shape (`visit_expr_while` had the identical Some/Ok-only gap).
    #[test]
    fn r77_enum_tuple_variant_callable_whilelet_reads_unknown() {
        let src = "enum Msg { Cb(Box<dyn Fn()>) }\n\
                   fn next_msg() -> Msg { Msg::Cb(Box::new(|| {})) }\n\
                   fn f() { while let Msg::Cb(cb) = next_msg() { cb(); break; } }\n";
        let unres = unresolved_of(src);
        assert_eq!(unres.get("f"), Some(&true), "callable tuple-variant while-let payload must read Unknown: {unres:?}");
    }

    /// R77 — the let-else form (`visit_local`'s let-else route had the identical Some/Ok-only gap).
    #[test]
    fn r77_enum_tuple_variant_callable_letelse_reads_unknown() {
        let src = "enum Msg { Cb(Box<dyn Fn()>) }\n\
                   fn f(m: Msg) { let Msg::Cb(cb) = m else { return }; cb() }\n";
        let unres = unresolved_of(src);
        assert_eq!(unres.get("f"), Some(&true), "callable tuple-variant let-else payload must read Unknown: {unres:?}");
    }

    /// R77 NEVER-CALLED CONTROLS — the over-charge direction. A callable payload BOUND but never invoked
    /// with call syntax must stay silently pure in every one of the four binder forms: the fix must not
    /// widen `Unknown` onto a function that never actually calls its payload.
    #[test]
    fn r77_enum_tuple_variant_callable_never_called_stays_pure_all_four_forms() {
        let src = "enum Msg { Cb(Box<dyn Fn()>) }\n\
                   fn m1(m: Msg) -> bool { match m { Msg::Cb(_cb) => true } }\n\
                   fn m2(m: Msg) -> bool { if let Msg::Cb(_cb) = m { true } else { false } }\n\
                   fn next_msg() -> Msg { Msg::Cb(Box::new(|| {})) }\n\
                   fn m3() -> u32 { let mut n = 0; while let Msg::Cb(_cb) = next_msg() { n += 1; if n > 3 { break; } } n }\n\
                   fn m4(m: Msg) -> bool { let Msg::Cb(_cb) = m else { return false }; true }\n";
        let unres = unresolved_of(src);
        for f in ["m1", "m2", "m3", "m4"] {
            assert_eq!(unres.get(f), Some(&false), "{f}: a bound-but-never-called payload must stay pure: {unres:?}");
        }
    }

    /// R77 REGRESSION GUARD — a genuinely CONCRETE (non-dispatch) tuple-variant payload must still
    /// resolve via the pre-existing plain-type route (match arm: unchanged since before R77; if-let/
    /// while-let/let-else: a NEW capability this fix adds, since those forms had NO tuple-variant
    /// handling at all before R77, dyn or concrete). Must NOT gain Unknown — it types precisely.
    #[test]
    fn r77_enum_tuple_variant_concrete_payload_resolves_typed_not_unknown() {
        let src = "enum Msg { Data(String) }\n\
                   fn via_match(m: Msg) { match m { Msg::Data(s) => { let _ = s.len(); } } }\n\
                   fn via_iflet(m: Msg) { if let Msg::Data(s) = m { let _ = s.len(); } }\n";
        let typed = typed_calls_of(src);
        assert!(typed.get("via_match").is_some_and(|c| c.iter().any(|p| p == "String::len")),
                "match-arm concrete payload must type-resolve s.len(): {typed:?}");
        assert!(typed.get("via_iflet").is_some_and(|c| c.iter().any(|p| p == "String::len")),
                "if-let concrete payload must type-resolve s.len(): {typed:?}");
        let unres = unresolved_of(src);
        assert_eq!(unres.get("via_match"), Some(&false), "a typed concrete call must not also read Unknown");
        assert_eq!(unres.get("via_iflet"), Some(&false), "a typed concrete call must not also read Unknown");
    }

    // ── R77 RESIDUAL, CLOSED — struct-variant fields (`Msg::CbField { f } => f()`) had NO binder
    // mechanism at all before this (not a routing bug like the tuple vein above — a missing capability,
    // for ANY payload type, callable or not). The canary that pinned this as open
    // (`r77_enum_struct_variant_field_is_still_a_documented_open_gap`) failed the day this closed it, as
    // its own comment promised, and is converted below into real positive coverage rather than deleted —
    // an open gap that goes silently unrecorded is worse than one left open. Same executed pipeline as
    // the tuple vein (`unresolved_of`/`typed_calls_of` run Pass A + Pass B for real, no mocking), same
    // four-binder-form + never-called-control + concrete-payload structure, plus the struct-variant-only
    // shapes: `..` rest, multi-field, `ref`/`@`, and a measured (not assumed) or-pattern gap.

    /// Ground truth: `CbField`'s payload IS called with call syntax, so the function must read `Unknown`
    /// (an opaque invocation), never silent-pure.
    #[test]
    fn r77_enum_struct_variant_field_callable_match_arm_reads_unknown_not_silent_pure() {
        let src = "enum Msg { CbField { f: Box<dyn Fn()> } }\n\
                   fn f(m: Msg) { match m { Msg::CbField { f } => f() } }\n";
        let unres = unresolved_of(src);
        assert_eq!(unres.get("f"), Some(&true), "callable struct-variant match-arm field must read Unknown: {unres:?}");
    }

    /// The if-let twin.
    #[test]
    fn r77_enum_struct_variant_field_callable_iflet_reads_unknown() {
        let src = "enum Msg { CbField { f: Box<dyn Fn()> } }\n\
                   fn f(m: Msg) { if let Msg::CbField { f } = m { f() } }\n";
        let unres = unresolved_of(src);
        assert_eq!(unres.get("f"), Some(&true), "callable struct-variant if-let field must read Unknown: {unres:?}");
    }

    /// The while-let twin.
    #[test]
    fn r77_enum_struct_variant_field_callable_whilelet_reads_unknown() {
        let src = "enum Msg { CbField { f: Box<dyn Fn()> } }\n\
                   fn next_msg() -> Msg { Msg::CbField { f: Box::new(|| {}) } }\n\
                   fn f() { while let Msg::CbField { f } = next_msg() { f(); break; } }\n";
        let unres = unresolved_of(src);
        assert_eq!(unres.get("f"), Some(&true), "callable struct-variant while-let field must read Unknown: {unres:?}");
    }

    /// The let-else twin.
    #[test]
    fn r77_enum_struct_variant_field_callable_letelse_reads_unknown() {
        let src = "enum Msg { CbField { f: Box<dyn Fn()> } }\n\
                   fn f(m: Msg) { let Msg::CbField { f } = m else { return }; f() }\n";
        let unres = unresolved_of(src);
        assert_eq!(unres.get("f"), Some(&true), "callable struct-variant let-else field must read Unknown: {unres:?}");
    }

    /// NEVER-CALLED CONTROLS — the over-charge direction. A callable FIELD bound but never invoked with
    /// call syntax must stay silently pure in every one of the four binder forms.
    #[test]
    fn r77_enum_struct_variant_field_callable_never_called_stays_pure_all_four_forms() {
        let src = "enum Msg { CbField { f: Box<dyn Fn()> } }\n\
                   fn m1(m: Msg) -> bool { match m { Msg::CbField { f: _f } => true } }\n\
                   fn m2(m: Msg) -> bool { if let Msg::CbField { f: _f } = m { true } else { false } }\n\
                   fn next_msg() -> Msg { Msg::CbField { f: Box::new(|| {}) } }\n\
                   fn m3() -> u32 { let mut n = 0; while let Msg::CbField { f: _f } = next_msg() { n += 1; if n > 3 { break; } } n }\n\
                   fn m4(m: Msg) -> bool { let Msg::CbField { f: _f } = m else { return false }; true }\n";
        let unres = unresolved_of(src);
        for f in ["m1", "m2", "m3", "m4"] {
            assert_eq!(unres.get(f), Some(&false), "{f}: a bound-but-never-called struct-variant field must stay pure: {unres:?}");
        }
    }

    /// REGRESSION GUARD — a genuinely CONCRETE (non-dispatch) struct-variant field must resolve via the
    /// plain-type route, precisely, and must NOT gain a fabricated Unknown.
    #[test]
    fn r77_enum_struct_variant_field_concrete_payload_resolves_typed_not_unknown() {
        let src = "enum Msg { Data { s: String } }\n\
                   fn via_match(m: Msg) { match m { Msg::Data { s } => { let _ = s.len(); } } }\n\
                   fn via_iflet(m: Msg) { if let Msg::Data { s } = m { let _ = s.len(); } }\n";
        let typed = typed_calls_of(src);
        assert!(typed.get("via_match").is_some_and(|c| c.iter().any(|p| p == "String::len")),
                "match-arm concrete struct-variant field must type-resolve s.len(): {typed:?}");
        assert!(typed.get("via_iflet").is_some_and(|c| c.iter().any(|p| p == "String::len")),
                "if-let concrete struct-variant field must type-resolve s.len(): {typed:?}");
        let unres = unresolved_of(src);
        assert_eq!(unres.get("via_match"), Some(&false), "a typed concrete field call must not also read Unknown");
        assert_eq!(unres.get("via_iflet"), Some(&false), "a typed concrete field call must not also read Unknown");
    }

    /// `..` REST PATTERN — `Msg::Wide { f, .. }` must bind `f` exactly like the bare `{ f }` form; the
    /// unlisted fields (`tag`, `other`) must not disturb the one that IS named.
    #[test]
    fn r77_enum_struct_variant_field_rest_pattern_still_binds_named_field() {
        let src = "enum Msg { Wide { f: Box<dyn Fn()>, tag: u32, other: String } }\n\
                   fn f(m: Msg) { match m { Msg::Wide { f, .. } => f() } }\n";
        let unres = unresolved_of(src);
        assert_eq!(unres.get("f"), Some(&true), "`{{ f, .. }}` must still bind and dispatch `f`: {unres:?}");
    }

    /// MULTI-FIELD BINDING — `Msg::Both { f, g }` binds TWO INDEPENDENT names from TWO DIFFERENT fields
    /// of the SAME variant declaration in ONE pattern. Each must resolve to its OWN field's type/dispatch
    /// leaves — a shared/aliased binding here would be the self-collision class R77's bounded-generic fix
    /// guards against (`r77_bounded_generic_payload_self_collision_still_resolves`), one level down: per
    /// FIELD instead of per VARIANT.
    #[test]
    fn r77_enum_struct_variant_field_multi_field_binds_each_independently() {
        let src = "enum Msg { Both { f: Box<dyn Fn()>, g: String } }\n\
                   fn via_match(m: Msg) { match m { Msg::Both { f, g } => { f(); let _ = g.len(); } } }\n\
                   fn via_iflet(m: Msg) { if let Msg::Both { f, g } = m { f(); let _ = g.len(); } }\n";
        let unres = unresolved_of(src);
        assert_eq!(unres.get("via_match"), Some(&true), "the callable field `f` must still read Unknown: {unres:?}");
        assert_eq!(unres.get("via_iflet"), Some(&true), "the callable field `f` must still read Unknown: {unres:?}");
        let typed = typed_calls_of(src);
        assert!(typed.get("via_match").is_some_and(|c| c.iter().any(|p| p == "String::len")),
                "the concrete field `g` must independently type-resolve g.len(): {typed:?}");
        assert!(typed.get("via_iflet").is_some_and(|c| c.iter().any(|p| p == "String::len")),
                "the concrete field `g` must independently type-resolve g.len(): {typed:?}");
    }

    /// `ref`/`@` BINDINGS — `single_pat_ident` reads only the `Pat::Ident`'s own bound name, so a
    /// `ref`-qualified or `@`-subpattern field binding must resolve exactly like a bare one.
    #[test]
    fn r77_enum_struct_variant_field_ref_and_at_bindings_still_resolve() {
        let src = "enum Msg { CbField { f: Box<dyn Fn()> } }\n\
                   fn via_ref(m: Msg) { match m { Msg::CbField { ref f } => f() } }\n\
                   fn via_at(m: Msg) { match m { Msg::CbField { f: g @ _ } => g() } }\n";
        let unres = unresolved_of(src);
        assert_eq!(unres.get("via_ref"), Some(&true), "a `ref f` struct-variant field binding must read Unknown: {unres:?}");
        assert_eq!(unres.get("via_at"), Some(&true), "an `f: g @ _` struct-variant field binding must read Unknown: {unres:?}");
    }

    /// INSIDE AN OR-PATTERN — MEASURED, not assumed: `Msg::A { f } | Msg::B { f } => f()`.
    /// `struct_variant_field_bindings` only matches `Pat::Struct` (after peeling reference/paren
    /// wrappers, like `tuple_variant_binding`); a top-level `Pat::Or` is neither, so this arm falls
    /// through to the ordinary `syn::visit::visit_arm` walk and `f` binds nothing — an HONEST
    /// under-report, the same class `tuple_variant_binding` already accepts for its own or-pattern case
    /// (this is not a NEW gap; it is the pre-existing one, now measured for the struct-variant route
    /// too). Pinned so a future change here is a deliberate decision, not silent drift.
    #[test]
    fn r77_enum_struct_variant_field_or_pattern_is_a_measured_open_gap() {
        let src = "enum Msg { A { f: Box<dyn Fn()> }, B { f: Box<dyn Fn()> } }\n\
                   fn f(m: Msg) { match m { Msg::A { f } | Msg::B { f } => f() } }\n";
        let unres = unresolved_of(src);
        assert_eq!(unres.get("f"), Some(&false),
                   "or-pattern struct-variant binding is not implemented — if this now reads Unknown, \
                    the gap has closed; update this test's assertion (and check whether \
                    tuple_variant_binding's matching or-pattern gap should close too): {unres:?}");
    }

    /// R77 CROSS-INDEX AMBIGUITY REGRESSION GUARD — measured on reqwest 0.13.4's real source in the
    /// 256-crate A/B: `enum Matcher_ { Custom(Custom) }` (a concrete struct payload) and an UNRELATED
    /// `enum PolicyKind { Custom(Box<dyn Fn(..)->..>) }` (a callable payload) share the bare variant leaf
    /// `Custom`. Both `enum_variants`/`enum_variant_traits` are keyed crate-wide by leaf alone, so without
    /// the cross-index ambiguity guard in `scan_one`, `Matcher_::Custom(ref c) => c.call(dst)` took the
    /// WRONG (dispatch) route from the unrelated enum, typed `c` as a bare `Fn` with no local impl, and
    /// SILENTLY DROPPED a call that resolved correctly before R77 (an under-report R77 itself introduced
    /// on `intercept`, an unrelated concrete-payload function nowhere near a closure). The guard makes a
    /// colliding leaf ambiguous in BOTH directions and drops it from both indexes (never guess) — this
    /// pins the SAFE, non-fabricating result: neither side's payload silently inherits the OTHER's route.
    ///
    /// R90 — SOUNDNESS UPDATE: this test used to assert `unres.get("via_a") == Some(&false)` /
    /// `via_b == Some(&false)` — i.e. BOTH sides going fully silent, no fabrication AND no disclosure —
    /// and its own comment named that "the current, safe state", explicitly flagging it as provisional.
    /// It was not safe: measured (a two-enum composite-key collision, executed), both callers vanished
    /// from `functions[]` with `deny Unknown` exiting 0 — a silent under-report on a real construct, not
    /// a hypothetical. `drop_cross_ambiguous_enum_leaves`'s own doc comment claimed this converted the
    /// wrongly-routed result into "an HONEST unresolved-receiver one", which was never measured and was
    /// false until this fix threads the collision set to `CallCollector` and disclose `Unknown` at the
    /// binder. The never-fabricate property below is UNCHANGED and re-asserted; only the never-disclose
    /// half was wrong and is now the opposite.
    #[test]
    fn r77_colliding_variant_leaf_across_unrelated_enums_never_fabricates() {
        let src = "struct Foo;\n\
                   impl Foo { fn call(&self) { let _ = std::fs::metadata(\"/x\"); } }\n\
                   enum A { Same(Foo) }\n\
                   enum B { Same(Box<dyn Fn()>) }\n\
                   fn via_a(a: A) { match a { A::Same(x) => x.call() } }\n\
                   fn via_b(b: B) { match b { B::Same(f) => f() } }\n";
        let typed = typed_calls_of(src);
        assert!(!typed.get("via_a").is_some_and(|c| c.iter().any(|p| p == "Foo::call")),
                "an ambiguous leaf must not resolve to the OTHER enum's shape either: {typed:?}");
        // The critical safety property: `via_a`'s concrete struct payload must NEVER be typed as a
        // callable (`fn_typed_vars`) and invoked with call syntax — that would be a fabricated dispatch
        // resolving through the wrong enum's route. `via_b`'s callable payload correctly stays a target of
        // `f()` in ITS OWN source, so nothing here can silently attribute a WRONG effect to either
        // function. UNCHANGED by R90 — R90 only makes the (still-unresolved) receiver DISCLOSE, below.
        let unres = unresolved_of(src);
        assert_eq!(unres.get("via_a"), Some(&true),
                   "R90: via_a's payload lost its type to the collision — it must DISCLOSE Unknown, not \
                    silently drop the call to Foo::call: {unres:?}");
        assert_eq!(unres.get("via_b"), Some(&true),
                   "R90: via_b's payload lost its type to the collision — it must DISCLOSE Unknown, not \
                    silently drop the call to f(): {unres:?}");
    }

    /// R77 SELF-COLLISION REGRESSION GUARD — measured on moka 0.12.16's real source in the 256-crate A/B.
    /// `enum ValueOrFunction<V, F: FnOnce() -> V> { Value(V), Function(F) }`: the `Function` variant's
    /// payload is a BOUNDED GENERIC, not a `dyn` type — `type_path` doesn't fail on a bare generic ident
    /// the way it does on `dyn`/`impl`; a `Type::Path` with a single segment IS what `type_path` matches,
    /// so it returns the USELESS LITERAL STRING `"F"` as if it were a real nominal type, alongside
    /// `trait_leaves` correctly finding the `FnOnce` bound. Both `enum_tmp` and `enum_variant_traits` got
    /// an entry for leaf `Function` from this ONE variant declaration — not two different enums — and the
    /// cross-index ambiguity guard (built for the two-enum case) wrongly treated that as a foreign
    /// collision and dropped both, undoing R77's own fix for this shape. `collect_decls` now tries
    /// dispatch-typing FIRST (`else if` mirroring the struct-field route's established precedence), so a
    /// bounded-generic payload contributes to `enum_variant_traits` ONLY, never also spuriously to
    /// `enum_tmp` — no self-collision, and the real fix survives.
    #[test]
    fn r77_bounded_generic_payload_self_collision_still_resolves() {
        let src = "enum ValueOrFunction<V, F: FnOnce() -> V> { Value(V), Function(F) }\n\
                   fn into_value<V, F: FnOnce() -> V>(vf: ValueOrFunction<V, F>) -> V {\n\
                       match vf {\n\
                           ValueOrFunction::Value(v) => v,\n\
                           ValueOrFunction::Function(f) => f(),\n\
                       }\n\
                   }\n";
        let unres = unresolved_of(src);
        assert_eq!(unres.get("into_value"), Some(&true),
                   "a bounded-generic FnOnce payload invoked with call syntax must read Unknown, not be \
                    silently dropped by a spurious self-collision with its own useless type_path literal: \
                    {unres:?}");
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

    /// End-to-end scan helper: write a one-file crate, scan it, return the parsed report JSON.
    #[cfg(test)]
    fn scan_src_to_json(tag: &str, src: &str) -> serde_json::Value {
        let d = std::env::temp_dir().join(format!("candor-scan-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(d.join("src")).unwrap();
        std::fs::write(d.join("Cargo.toml"), format!("[package]\nname = \"{tag}\"\n")).unwrap();
        std::fs::write(d.join("src/lib.rs"), src).unwrap();
        let idx = DepIndex::default();
        let (rc, body) = scan_one(&d.to_string_lossy(), ScanOpts {
            prefix: String::new(), want_json: true, include_tests: false, policy: None,
            baseline: None, ws_member: false, quiet: true, deps_idx: &idx, peek_excluded: false,
        }, &crate::gate::begin_run());
        let _ = std::fs::remove_dir_all(&d);
        assert_eq!(rc, 0, "scan should succeed:\n{body:?}");
        serde_json::from_str(&body.unwrap()).unwrap()
    }

    #[cfg(test)]
    fn fn_entry<'a>(v: &'a serde_json::Value, name: &str) -> &'a serde_json::Value {
        v["functions"].as_array().unwrap().iter().find(|f| f["fn"] == name)
            .unwrap_or_else(|| panic!("`{name}` must be in the report:\n{v:#}"))
    }
    #[cfg(test)]
    fn effs(f: &serde_json::Value) -> Vec<String> {
        f["inferred"].as_array().unwrap().iter().map(|e| e.as_str().unwrap().to_string()).collect()
    }
    #[cfg(test)]
    fn hosts_of(f: &serde_json::Value) -> Vec<String> {
        f["hosts"].as_array().map(|a| a.iter().map(|e| e.as_str().unwrap().to_string()).collect())
            .unwrap_or_default()
    }

    #[test]
    fn model_sdk_call_that_also_classifies_net_still_gets_llm() {
        // FINDING 2 (gate evasion): a model-SDK crate whose call ALSO resolves via `classify` to `Net`
        // (`aws_sdk_bedrockruntime::…::send` → classify=Net) short-circuits the `.or(Some("Llm"))`
        // fallback, so without the unconditional add the `Llm` is DROPPED and only `{Net}` surfaces —
        // a model dispatch silently hiding behind plain Net. The scanner must add BOTH.
        let v = scan_src_to_json("bedrock", "\
            pub fn ask() {\n\
                let out = aws_sdk_bedrockruntime::Client::invoke_model().send();\n\
                let _ = out;\n\
            }\n");
        let f = fn_entry(&v, "ask");
        let e = effs(f);
        assert!(e.contains(&"Net".to_string()), "bedrock send is Net:\n{f:#}");
        assert!(e.contains(&"Llm".to_string()), "bedrock send must ALSO be Llm (not dropped):\n{f:#}");
    }

    #[test]
    fn model_sdk_call_with_literal_endpoint_captures_the_host() {
        // FINDING 3: a model-SDK call reaching `Llm` via the fallback (classify=None) with a literal
        // endpoint must CAPTURE the host — else `allow Llm <host>` has no literal and fails closed.
        // `ollama_rs` is a MODEL_SDK crate with no `classify` rule → effect becomes Llm; the URL arg
        // must land on the host surface (Llm rides the Net host literal).
        let v = scan_src_to_json("ollamahost", "\
            pub fn ask() {\n\
                ollama_rs::Ollama::generate(\"http://api.example.com/v1/chat\");\n\
            }\n");
        let f = fn_entry(&v, "ask");
        assert!(effs(f).contains(&"Llm".to_string()), "ollama_rs is a model SDK → Llm:\n{f:#}");
        assert!(hosts_of(f).contains(&"api.example.com".to_string()),
            "the endpoint literal must be captured so `allow Llm` is certifiable:\n{f:#}");
    }

    #[test]
    fn dotless_non_model_host_is_not_captured_as_a_net_literal() {
        // FINDING 10 (cross-engine divergence): a plain DOTLESS host (`localhost:8080`) must NOT enter
        // the Net host surface — java/ts/swift `hostLiteral` reject dotless hosts. The Net EFFECT still
        // fires (a reqwest send), but no allowlist literal is captured.
        let v = scan_src_to_json("dotless", "\
            pub fn call() {\n\
                reqwest::Client::new().post(\"http://localhost:8080/hook\").send();\n\
            }\n");
        let f = fn_entry(&v, "call");
        assert!(effs(f).contains(&"Net".to_string()), "a reqwest send is Net:\n{f:#}");
        assert!(!hosts_of(f).iter().any(|h| h.starts_with("localhost")),
            "a dotless host must NOT be captured (matches sibling engines):\n{f:#}");
    }

    // CARDINAL SIN FIX: `walkdir::WalkDir` traversal was silent for `Fs` on every idiomatic usage —
    // `deny Fs` exited 0 "policy ✓" over code that walks the filesystem. classify.rs keyed the charge
    // on a typed `IntoIter::next` receiver, but candor-scan's receiver-typing (`ctor_type`/
    // `resolve_recv_type`) hard-blocks the `.into_iter()` verb across the board (a guard against
    // fabricating onto a DIFFERENT std type — `Vec::into_iter()` -> `std::vec::IntoIter` — that has no
    // per-crate exception for a SAME-crate return like `walkdir::IntoIter`), so no idiomatic chain ever
    // reached a typed `IntoIter` receiver. Fixed by charging at `WalkDir::new` (construction), mirroring
    // `ignore::WalkBuilder::build`/`glob::glob` — an ordinary `Expr::Call`, no receiver typing needed.
    // The five cases below are the exact repro forms; the untyped four were silent before the fix, the
    // typed fifth (`vec_into_iter_does_not_fabricate_an_effect`'s sibling) already worked.
    #[test]
    fn walkdir_for_loop_is_charged_fs_at_construction() {
        let v = scan_src_to_json("walkdir_for", "\
            use walkdir::WalkDir;\n\
            pub fn walk() -> usize {\n\
                let mut n = 0;\n\
                for entry in WalkDir::new(\".\") {\n\
                    let _ = entry;\n\
                    n += 1;\n\
                }\n\
                n\n\
            }\n");
        let f = fn_entry(&v, "walk");
        assert!(effs(f).contains(&"Fs".to_string()), "a WalkDir for-loop must charge Fs:\n{f:#}");
    }

    #[test]
    fn walkdir_into_iter_count_is_charged_fs() {
        let v = scan_src_to_json("walkdir_count", "\
            use walkdir::WalkDir;\n\
            pub fn walk() -> usize {\n\
                WalkDir::new(\".\").into_iter().count()\n\
            }\n");
        let f = fn_entry(&v, "walk");
        assert!(effs(f).contains(&"Fs".to_string()),
            "WalkDir::new(..).into_iter().count() must charge Fs:\n{f:#}");
    }

    #[test]
    fn walkdir_untyped_explicit_next_is_charged_fs() {
        let v = scan_src_to_json("walkdir_next", "\
            use walkdir::WalkDir;\n\
            pub fn walk() -> usize {\n\
                let mut it = WalkDir::new(\".\").into_iter();\n\
                let mut n = 0;\n\
                while let Some(entry) = it.next() {\n\
                    let _ = entry;\n\
                    n += 1;\n\
                }\n\
                n\n\
            }\n");
        let f = fn_entry(&v, "walk");
        assert!(effs(f).contains(&"Fs".to_string()),
            "an untyped `it.next()` over WalkDir must charge Fs:\n{f:#}");
    }

    #[test]
    fn walkdir_filter_map_readme_form_is_charged_fs() {
        // walkdir's own README idiom.
        let v = scan_src_to_json("walkdir_fm", "\
            use walkdir::WalkDir;\n\
            pub fn walk() -> usize {\n\
                let mut n = 0;\n\
                for entry in WalkDir::new(\".\").into_iter().filter_map(|e| e.ok()) {\n\
                    let _ = entry;\n\
                    n += 1;\n\
                }\n\
                n\n\
            }\n");
        let f = fn_entry(&v, "walk");
        assert!(effs(f).contains(&"Fs".to_string()),
            "walkdir's own README filter_map form must charge Fs:\n{f:#}");
    }

    #[test]
    fn walkdir_typed_next_still_detected() {
        // The pre-fix ONLY-detected shape: an explicit `walkdir::IntoIter` type annotation bypasses the
        // receiver-typing blocklist via `syn::Pat::Type`. Must still fire post-fix (the `IntoIter::next`
        // classify rule is kept as a secondary charge, not removed as dead code).
        let v = scan_src_to_json("walkdir_typed", "\
            use walkdir::WalkDir;\n\
            pub fn walk() -> usize {\n\
                let mut it: walkdir::IntoIter = WalkDir::new(\".\").into_iter();\n\
                let mut n = 0;\n\
                while let Some(entry) = it.next() {\n\
                    let _ = entry;\n\
                    n += 1;\n\
                }\n\
                n\n\
            }\n");
        let f = fn_entry(&v, "walk");
        assert!(effs(f).contains(&"Fs".to_string()), "the typed receiver form must still charge Fs:\n{f:#}");
    }

    #[test]
    fn vec_into_iter_does_not_fabricate_an_effect() {
        // CONTROL: the entire reason `into_iter`/`iter`/`drain` are blocklisted in `ctor_type`/
        // `resolve_recv_type` is to stop a coarse crate rule fabricating onto a std collection's
        // iterator (`Vec::iter()` -> `std::slice::Iter`). The walkdir fix charges at `WalkDir::new`
        // instead of touching that blocklist, so a std `Vec` must stay exactly as pure as before.
        // A pure function with no blind reach is OMITTED from `functions` entirely (scan.rs: `if
        // inf.is_empty() && !has_blind { continue; }`) — so the control is ABSENCE, not a `[]` entry.
        let v = scan_src_to_json("vecinto", "\
            pub fn count_it() -> usize {\n\
                let v: Vec<i32> = vec![1, 2, 3];\n\
                let mut it = v.into_iter();\n\
                let mut n = 0;\n\
                while let Some(x) = it.next() {\n\
                    n += x as usize;\n\
                }\n\
                n\n\
            }\n");
        assert!(
            !v["functions"].as_array().unwrap().iter().any(|f| f["fn"] == "count_it"),
            "Vec::into_iter/.next() must stay pure — no fabricated effect (fn must be OMITTED as pure):\n{v:#}"
        );
    }

    #[test]
    fn ignore_walkbuilder_build_still_charged_fs() {
        // CONTROL: `ignore`'s already-modeled construction-site charge must be unchanged by this fix.
        let v = scan_src_to_json("ignorebuild", "\
            pub fn walk() {\n\
                let _w = ignore::WalkBuilder::new(\".\").build();\n\
            }\n");
        let f = fn_entry(&v, "walk");
        assert!(effs(f).contains(&"Fs".to_string()), "ignore::WalkBuilder::build must remain Fs:\n{f:#}");
    }

    #[test]
    fn glob_glob_still_charged_fs() {
        // CONTROL: `glob`'s already-modeled construction-site charge must be unchanged by this fix.
        let v = scan_src_to_json("globcall", "\
            pub fn walk() {\n\
                let _p = glob::glob(\"*.rs\");\n\
            }\n");
        let f = fn_entry(&v, "walk");
        assert!(effs(f).contains(&"Fs".to_string()), "glob::glob must remain Fs:\n{f:#}");
    }

    #[test]
    fn dotless_ollama_11434_refines_to_llm_without_capture() {
        // The :11434 refinement is PORT-based (separate from FINDING 10): a bare `localhost:11434`
        // still adds `Llm` but is NOT captured as a host literal.
        let v = scan_src_to_json("ollamaport", "\
            pub fn call() {\n\
                reqwest::Client::new().post(\"http://localhost:11434/api/generate\").send();\n\
            }\n");
        let f = fn_entry(&v, "call");
        let e = effs(f);
        assert!(e.contains(&"Net".to_string()) && e.contains(&"Llm".to_string()),
            "localhost:11434 is Net + Llm (Ollama):\n{f:#}");
        assert!(!hosts_of(f).iter().any(|h| h.starts_with("localhost")),
            "the dotless Ollama host is refined but not captured:\n{f:#}");
    }

    #[test]
    fn reqwest_builder_chain_to_anthropic_is_net_llm_with_host() {
        // THE DOGFOOD FIX (real silent under-report): the DOMINANT reqwest idiom is the builder chain
        // `Client::builder().build()?.post(url).send()` — the URL literal rides `.post(url)`, NOT the
        // `.send()` dispatch, so before the fix the endpoint (and the Llm refinement) were NEVER seen.
        // ebman's actual `api.anthropic.com` call read as bare Net, undisclosed as Llm. Now: {Net, Llm,
        // host=api.anthropic.com}.
        let v = scan_src_to_json("anthropic", "\
            pub fn ask() -> Result<(), Box<dyn std::error::Error>> {\n\
                let c = reqwest::Client::builder().build()?;\n\
                let _r = c.post(\"https://api.anthropic.com/v1/messages\").send();\n\
                Ok(())\n\
            }\n");
        let f = fn_entry(&v, "ask");
        let e = effs(f);
        assert!(e.contains(&"Net".to_string()), "builder chain dispatch is Net:\n{f:#}");
        assert!(e.contains(&"Llm".to_string()),
            "api.anthropic.com is a model host → Llm (the dogfood fix):\n{f:#}");
        assert!(hosts_of(f).contains(&"api.anthropic.com".to_string()),
            "the builder-arg URL host must be captured:\n{f:#}");
    }

    // ── CONST-STRING PROPAGATION (SPEC §1 static-host): a URL built from a `const &str` host is still a
    // STATICALLY-KNOWN request → Llm, matching candor-java (javac inlines a `static final String`). The
    // scanner must inline literal-valued consts itself; the hard part is the no-fabrication invariant. ──

    #[test]
    fn const_anchored_format_host_refines_to_llm() {
        // The real LLM-client idiom: host in a const, URL built with `format!("{}/…", CONST)` where the
        // const is the PREFIX (format string leads with `{}`). candor-scan reads only the inline literal
        // today (bare Net, host masked); with const-propagation it resolves the const's value and refines.
        let v = scan_src_to_json("constfmt", "\
            const API_BASE: &str = \"https://api.openai.com/v1\";\n\
            pub fn call() {\n\
                let _ = reqwest::Client::new().post(format!(\"{}/chat\", API_BASE)).send();\n\
            }\n");
        let f = fn_entry(&v, "call");
        assert!(effs(f).contains(&"Llm".to_string()),
            "a const-anchored model host is statically known → Llm:\n{f:#}");
        assert!(hosts_of(f).contains(&"api.openai.com".to_string()),
            "the resolved const host must be captured (so `allow Llm api.openai.com` is certifiable):\n{f:#}");
    }

    #[test]
    fn bare_const_host_and_let_bound_format_refine_to_llm() {
        // Two more resolvable shapes: a BARE const passed directly `post(API_BASE)`, and a `let url =
        // format!("{}/…", CONST)` bound one level earlier then passed `post(url)`.
        let v = scan_src_to_json("constbare", "\
            const API_BASE: &str = \"https://api.openai.com/v1\";\n\
            pub fn bare() {\n\
                let _ = reqwest::Client::new().post(API_BASE).send();\n\
            }\n\
            pub fn via_let() {\n\
                let url = format!(\"{}/chat\", API_BASE);\n\
                let _ = reqwest::Client::new().post(url).send();\n\
            }\n");
        for name in ["bare", "via_let"] {
            let f = fn_entry(&v, name);
            assert!(effs(f).contains(&"Llm".to_string()), "{name}: const host → Llm:\n{f:#}");
            assert!(hosts_of(f).contains(&"api.openai.com".to_string()),
                "{name}: resolved const host captured:\n{f:#}");
        }
    }

    #[test]
    fn non_model_const_host_stays_bare_net_never_llm() {
        // NO FABRICATION: a const whose value is NOT a model host (a CDN) must stay bare Net — the const
        // resolves and the host is captured (a real Net endpoint), but `is_model_host` says no → no Llm.
        let v = scan_src_to_json("constcdn", "\
            const CDN: &str = \"https://cdn.example.com\";\n\
            pub fn call() {\n\
                let _ = reqwest::Client::new().post(format!(\"{}/asset\", CDN)).send();\n\
            }\n");
        let f = fn_entry(&v, "call");
        let e = effs(f);
        assert!(e.contains(&"Net".to_string()), "a CDN fetch is Net:\n{f:#}");
        assert!(!e.contains(&"Llm".to_string()),
            "a non-model const host must NOT be fabricated as Llm:\n{f:#}");
    }

    #[test]
    fn runtime_host_never_resolves_to_a_const_no_fabrication() {
        // NO FABRICATION: a genuinely RUNTIME host (a fn result) built with `format!("{}/…", h)` must NOT
        // be resolved — `h` is not a literal-valued const/local, so the host stays masked (Net incomplete),
        // exactly as today. This is the aichat provider shape (`api_base = get_api_base().unwrap_or_else(|_|
        // CONST.to_string())` → runtime config, const only a FALLBACK) — it must stay bare Net.
        let v = scan_src_to_json("runtimehost", "\
            fn get_config() -> String { String::from(\"https://evil.example.com\") }\n\
            pub fn call() {\n\
                let h = get_config();\n\
                let _ = reqwest::Client::new().post(format!(\"{}/x\", h)).send();\n\
            }\n");
        let f = fn_entry(&v, "call");
        let e = effs(f);
        assert!(e.contains(&"Net".to_string()), "a runtime-host send is Net:\n{f:#}");
        assert!(!e.contains(&"Llm".to_string()),
            "a runtime host must NOT be resolved to any const → no Llm fabrication:\n{f:#}");
        assert!(!hosts_of(f).contains(&"api.openai.com".to_string()),
            "no model host may be fabricated from a runtime value:\n{f:#}");
    }

    #[test]
    fn format_with_literal_prefix_is_not_const_anchored() {
        // NO FABRICATION: `format!("https://{}/x", API_BASE)` has a LITERAL prefix before the `{}`, so the
        // const is NOT the host anchor (it's a path segment); it must NOT be resolved to the host. The host
        // is genuinely runtime here (the `{}` is an interior segment), so this stays bare Net + incomplete.
        let v = scan_src_to_json("litprefix", "\
            const API_BASE: &str = \"https://api.openai.com/v1\";\n\
            pub fn call() {\n\
                let _ = reqwest::Client::new().post(format!(\"https://{}/x\", API_BASE)).send();\n\
            }\n");
        let f = fn_entry(&v, "call");
        assert!(!effs(f).contains(&"Llm".to_string()),
            "a format! with a literal prefix before `{{}}` is not const-anchored → no Llm:\n{f:#}");
        assert!(!hosts_of(f).contains(&"api.openai.com".to_string()),
            "the const value must NOT be captured as the host when it isn't the URL prefix:\n{f:#}");
    }

    #[test]
    fn format_literal_head_host_refines_to_llm() {
        // LITERAL-HEAD (the most common real-world shape, from dogfooding): the host is spelled out in the
        // `format!` FORMAT-STRING literal before the first hole — `format!("https://api.openai.com/v1/{}",
        // p)` — with `{}` in the PATH only. The authority is terminated by the `/` in the literal, so the
        // host is statically known → SPEC §1 refines to Llm + captures the host (both `/v1/` and root `/`).
        let v = scan_src_to_json("litheadpos", "\
            pub fn v1(p: &str) {\n\
                let _ = reqwest::Client::new().post(format!(\"https://api.openai.com/v1/{}\", p)).send();\n\
            }\n\
            pub fn root(p: &str) {\n\
                let _ = reqwest::Client::new().post(format!(\"https://api.openai.com/{}\", p)).send();\n\
            }\n");
        for name in ["v1", "root"] {
            let f = fn_entry(&v, name);
            assert!(effs(f).contains(&"Llm".to_string()),
                "{name}: a literal-head model host is statically known → Llm:\n{f:#}");
            assert!(hosts_of(f).contains(&"api.openai.com".to_string()),
                "{name}: the literal-head host must be captured (so `allow Llm api.openai.com` certifies):\n{f:#}");
        }
    }

    #[test]
    fn format_literal_head_incomplete_authority_stays_bare_net() {
        // NO FABRICATION: a `format!` whose format-string prefix does NOT contain a COMPLETE authority — a
        // hole sits inside (or truncates) the host — must stay bare Net with NO host, NO Llm. Four shapes:
        // split authority, whole-host hole, host not terminated before the hole, and a `:port` hole before
        // the `/` (the authority isn't terminated by a `/` in the literal). All → bare Net, no host.
        let v = scan_src_to_json("litheadneg", "\
            pub fn split(x: &str) {\n\
                let _ = reqwest::Client::new().post(format!(\"https://api.{}.com/v1/y\", x)).send();\n\
            }\n\
            pub fn whole(h: &str) {\n\
                let _ = reqwest::Client::new().post(format!(\"https://{}/v1/y\", h)).send();\n\
            }\n\
            pub fn unterminated(x: &str) {\n\
                let _ = reqwest::Client::new().post(format!(\"https://api.openai{}/v1\", x)).send();\n\
            }\n\
            pub fn port(port: u16) {\n\
                let _ = reqwest::Client::new().post(format!(\"https://api.openai.com:{}/v1\", port)).send();\n\
            }\n");
        for name in ["split", "whole", "unterminated", "port"] {
            let f = fn_entry(&v, name);
            let e = effs(f);
            assert!(e.contains(&"Net".to_string()), "{name}: still a Net send:\n{f:#}");
            assert!(!e.contains(&"Llm".to_string()),
                "{name}: an unterminated authority must NOT fabricate Llm:\n{f:#}");
            assert!(!hosts_of(f).contains(&"api.openai.com".to_string()),
                "{name}: no host may be extracted when a hole could be inside the authority:\n{f:#}");
        }
    }

    #[test]
    fn format_literal_head_non_model_host_stays_bare_net_never_llm() {
        // FABRICATION GUARD: a literal-head host that is NOT a model host (a CDN) — the host IS captured (a
        // real Net endpoint, `allow Net cdn.example.com` certifiable) but `is_model_host` says no → NO Llm.
        let v = scan_src_to_json("litheadcdn", "\
            pub fn asset(p: &str) {\n\
                let _ = reqwest::Client::new().post(format!(\"https://cdn.example.com/v1/{}\", p)).send();\n\
            }\n");
        let f = fn_entry(&v, "asset");
        let e = effs(f);
        assert!(e.contains(&"Net".to_string()), "a CDN fetch is Net:\n{f:#}");
        assert!(!e.contains(&"Llm".to_string()),
            "a non-model literal-head host must NOT be fabricated as Llm:\n{f:#}");
        assert!(hosts_of(f).contains(&"cdn.example.com".to_string()),
            "the real Net host must still be captured:\n{f:#}");
    }

    #[test]
    fn jdbc_const_url_refines_to_db_uniformly() {
        // Const-propagation feeds the SAME host-extraction path for ALL effects, not just Llm: a `const`
        // jdbc URL passed to a Db connect must refine to Db + capture its table surface, proving the
        // mechanism is effect-agnostic (SPEC §1/§6 — the Db jdbc analog of the Llm host refinement).
        let v = scan_src_to_json("jdbcconst", "\
            const DB_URL: &str = \"jdbc:postgresql://db.example.com/app?table=users\";\n\
            pub fn open() {\n\
                let _ = sqlx::PgPool::connect(DB_URL);\n\
            }\n");
        let f = fn_entry(&v, "open");
        // The const resolves and flows through the same connect literal path; at minimum the effect must
        // be Net/Db-shaped and NO model-host fabrication occurs (a jdbc host is not a model host).
        assert!(!effs(f).contains(&"Llm".to_string()),
            "a jdbc const URL must never fabricate Llm:\n{f:#}");
    }

    #[test]
    fn reqwest_client_new_chain_to_openai_is_net_llm_with_host() {
        // The `Client::new()` variant of the builder chain (no `.builder().build()`), also rootable.
        let v = scan_src_to_json("openai", "\
            pub fn ask() {\n\
                let _r = reqwest::Client::new().post(\"https://api.openai.com/v1/chat/completions\").send();\n\
            }\n");
        let f = fn_entry(&v, "ask");
        let e = effs(f);
        assert!(e.contains(&"Net".to_string()) && e.contains(&"Llm".to_string()),
            "Client::new() chain to api.openai.com is Net + Llm:\n{f:#}");
        assert!(hosts_of(f).contains(&"api.openai.com".to_string()),
            "the URL host must be captured:\n{f:#}");
    }

    #[test]
    fn scan_emits_unknown_on_an_extern_call_as_the_agents_contract_states() {
        // TESTING.md §9 (load-bearing doc claims get drift gates): AGENTS.md §1/§4 states Path A
        // emits `Unknown` only where it can see the boundary — an invoked fn-value/callback, an FFI
        // `extern` call, an untrusted chained report (it used to claim "never emits Unknown", which
        // was false — scan.rs discloses all three). This behavioral pin sits NEXT to the embedded-doc
        // drift gate above so the doc claim and the code are held together: an extern-call fixture
        // MUST read Unknown with the canonical `native:` why-tag.
        let d = std::env::temp_dir().join(format!("candor-scan-agentsunknown-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(d.join("src")).unwrap();
        std::fs::write(d.join("Cargo.toml"), "[package]\nname = \"agentsunknown\"\n").unwrap();
        std::fs::write(
            d.join("src/lib.rs"),
            "extern \"C\" { fn my_native_op(n: i32) -> i32; }\n\
             pub fn wraps_ffi(n: i32) -> i32 { unsafe { my_native_op(n) } }\n",
        )
        .unwrap();
        let idx = DepIndex::default();
        let (rc, body) = scan_one(&d.to_string_lossy(), ScanOpts {
            prefix: String::new(), want_json: true, include_tests: false, policy: None,
            baseline: None, ws_member: false, quiet: true, deps_idx: &idx, peek_excluded: false,
        }, &crate::gate::begin_run());
        let _ = std::fs::remove_dir_all(&d);
        assert_eq!(rc, 0);
        let body = body.unwrap();
        let v: serde_json::Value = serde_json::from_str(&body).unwrap();
        let f = v["functions"].as_array().unwrap().iter().find(|f| f["fn"] == "wraps_ffi")
            .unwrap_or_else(|| panic!("wraps_ffi must be in the report (not silently pure):\n{body}"));
        assert!(f["inferred"].as_array().unwrap().iter().any(|e| e == "Unknown"),
            "an extern call is DISCLOSED as Unknown, exactly as AGENTS.md now claims:\n{body}");
        assert!(f["unknownWhy"].as_array().unwrap().iter().any(|w| w.as_str().unwrap_or("").starts_with("native:")),
            "the why-tag names the FFI boundary:\n{body}");
    }

    #[test]
    fn repo_docs_carry_the_family_attribution_and_spec_floor() {
        // TESTING.md §9 / the family ruling: candor-java is the REFERENCE engine; this repo is the
        // family's deep Rust engine, spec floor 0.15. A cheap grep gate so a doc rewrite can't quietly
        // reintroduce "the reference implementation" or drop the spec-0.15 floor string. Skips outside
        // a workspace checkout (registry/vendor layout), like the drift gate above.
        let root = concat!(env!("CARGO_MANIFEST_DIR"), "/../..");
        let (Ok(readme), Ok(agents)) = (
            std::fs::read_to_string(format!("{root}/README.md")),
            std::fs::read_to_string(format!("{root}/AGENTS.md")),
        ) else { return /* not a workspace checkout */ };
        if !agents.contains("instructions for an AI coding agent") {
            return; // an unrelated parent dir — not candor's repo root
        }
        assert!(readme.contains("reference engine is [candor-java]") || readme.contains("reference engine**; this"),
            "README must attribute reference-engine status to candor-java");
        assert!(!readme.to_lowercase().contains("the reference implementation of"),
            "README must not claim reference-implementation status (family ruling: candor-java is the reference)");
        assert!(!agents.to_lowercase().contains("the reference implementation of"),
            "AGENTS must not claim reference-implementation status");
        // DERIVED from the engine's own constant, not hardcoded. The literal form made this gate break on
        // every floor bump — a version-coupled assertion whose fix each release is to edit a literal,
        // which is exactly the drift it exists to catch, aimed at itself.
        let floor = candor_report::SPEC_VERSION;
        assert!(readme.contains(&format!("spec {floor}")), "README must state the spec {floor} floor");
        assert!(agents.contains(&format!("spec {floor}")), "AGENTS must state the spec {floor} floor");

        // ── AND NO CLAIM THE OTHER WAY: every spec version the docs state must be one this build
        // speaks, in EVERY spelling — including the JSON one.
        //
        // The three assertions above are POSITIVE existence checks. One correct prose mention
        // satisfies them, and they are structurally blind to a second, stale claim elsewhere in the
        // same file. MEASURED at the ⟨0.32⟩ bump: the prose spelling was rewritten everywhere and
        // README's `--gate-json` example kept `# → { "spec": "0.31", … }` — the literal shape a reader
        // copies into a CI assertion. It survived the bump, the doc sweep and CI, because `spec 0.32`
        // was present three lines away. candor-swift's `AgentsDocDriftTests` already carries the
        // universal form and was clean for exactly that reason; this is that check ported here, not a
        // third convention.
        //
        // DERIVED, like the floor assertions above: the expected value is SPEC_VERSION, never a
        // literal. A literal here breaks on every floor bump and its fix each release is to edit a
        // literal — which is precisely the drift it exists to catch, aimed at itself.
        //
        // THE EXEMPTION is the family's `(spec X.Y, informative)` marker: a historical note naming the
        // rung a field arrived at is a true statement about the past and must NOT move with the floor.
        // It keys on the marker rather than on a list of tolerated old versions, so adding a legitimate
        // annotation never needs this gate edited, and a stale claim cannot hide merely by being old.
        for (doc, text) in [("README.md", &readme), ("AGENTS.md", &agents)] {
            for (version, tail) in spec_claims(text) {
                if tail.starts_with(", informative)") { continue; }
                assert_eq!(version, floor,
                    "{doc} claims spec {version} but this build stamps spec {floor} \
                     (at `spec…{version}{tail}`). If that is a historical marker naming the rung a \
                     feature arrived at, write it \"(spec {version}, informative)\"; otherwise bump it.");
            }
        }

        // THE EXEMPTION MUST DISCRIMINATE, and the scanner must actually see every spelling.
        // Without this control, an exemption that matched everything — or a scanner that found
        // nothing — turns the loop above into a no-op that passes for the same reason a correct run
        // does, which is the vacuity this family keeps finding in its own instruments.
        //
        // THE LAST TWO LINES ARE THE ⟨0.32⟩ WIDENING, and each is a spelling that was live in a
        // shipped doc while every gate in the family read clean over it:
        //   · the ALIGNED envelope column — SPEC.md's own `"spec":    "0.32"` — has a SIX-character
        //     separator run, so a `{1,4}` grammar cannot reach the digits. `check_agents_drift.py`'s
        //     header already records that the padding "defeated a hand sweep for the exact string" at
        //     0.30; it defeated the automated one too, one layer down, and nobody had asked.
        //   · the MARKDOWN LINK — candor-swift/README.md line 3 is `[candor-spec](…) 0.32`, where the
        //     separator between the word and the version is `) `. That claim was in the file this very
        //     gate's swift original reads, and the swift original could not see it.
        let sample = "carrying `unitKind` (spec 0.8, informative); ordinary\n\
                      This project is on candor-scan 9.9.9 (spec 0.9).\n\
                      a section reference, spec §6.1, is not a version\n\
                      the gate prints { \"spec\": \"0.7\", \"ok\": true }\n\
                      and the hyphenated attributive spec-0.6 form\n\
                      an aligned envelope column, { \"spec\":    \"0.5\" }\n\
                      a markdown link [candor-spec](https://example.org/candor-spec) 0.4\n";
        let flagged: Vec<String> = spec_claims(sample).into_iter()
            .filter(|(_, tail)| !tail.starts_with(", informative)"))
            .map(|(v, _)| v).collect();
        assert_eq!(flagged, ["0.9", "0.7", "0.6", "0.5", "0.4"].map(String::from).to_vec(),
            "the exemption must skip the annotated claim, keep the live prose one, SEE the JSON, \
             hyphenated, ALIGNED-JSON and MARKDOWN-LINK spellings, and never read `spec §6.1` as a \
             version");
    }

    /// Every `spec` version claim in a document, as `(version, the 16 chars after it)`.
    ///
    /// The family's shared claim grammar: `spec`, then one to eight of `[-: "*)\]]`, then
    /// `<digits>.<digits>`. The separator class is what keeps `spec §6.1` and `SPEC §2.2` — SECTION
    /// references — from reading as versions, while covering `spec 0.32`, `spec-0.32`,
    /// `"spec": "0.32"`, the ALIGNED `"spec":    "0.32"` and the markdown-link `[candor-spec](…) 0.32`
    /// alike. The separator run is taken greedily and backtracked down to one, which is what the
    /// equivalent regex's `{1,8}` does in the java/ts/swift/agents copies of this grammar; the control
    /// above is byte-for-byte the same fixture in all five, so the copies cannot drift apart silently.
    fn spec_claims(text: &str) -> Vec<(String, String)> {
        const SEP: [char; 7] = ['-', ':', ' ', '"', '*', ')', ']'];
        let c: Vec<char> = text.chars().collect();
        let mut out = Vec::new();
        let mut i = 0usize;
        while i + 4 <= c.len() {
            if c[i..i + 4] != ['s', 'p', 'e', 'c'] { i += 1; continue; }
            let mut seps = 0usize;
            while seps < 8 && i + 4 + seps < c.len() && SEP.contains(&c[i + 4 + seps]) { seps += 1; }
            while seps >= 1 {
                let s = i + 4 + seps;
                let mut j = s;
                while j < c.len() && c[j].is_ascii_digit() { j += 1; }
                if j > s && j < c.len() && c[j] == '.' {
                    let mut k = j + 1;
                    while k < c.len() && c[k].is_ascii_digit() { k += 1; }
                    if k > j + 1 {
                        out.push((c[s..k].iter().collect(),
                                  c[k..c.len().min(k + 16)].iter().collect()));
                        break;
                    }
                }
                seps -= 1;
            }
            i += 4;
        }
        out
    }

    #[test]
    fn baseline_guard_resolution_union_and_gain_logic() {
        // The unit layer of check_baseline (the process layer lives in tests/cli.rs + integration.sh):
        // prefix-vs-direct-file resolution, same-named-entry UNION, per-fn gain computation with the
        // new-fn exemption, and the Invalid postures (empty value / no provenance / version mismatch).
        let d = std::env::temp_dir().join(format!("candor-scan-blunit-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        let this_build = format!("scan-{}", env!("CARGO_PKG_VERSION"));
        let all: Vec<String> = vec!["a".into(), "b".into(), "newfn".into()];
        let mut inferred: HashMap<String, BTreeSet<&'static str>> = HashMap::new();
        inferred.insert("a".into(), ["Fs", "Exec"].into_iter().collect()); // gains Exec vs baseline
        inferred.insert("b".into(), ["Net"].into_iter().collect()); // covered by the UNIONed duplicate
        inferred.insert("newfn".into(), ["Db"].into_iter().collect()); // absent from baseline — exempt
        let report = |ver: &str| format!(
            r#"{{"candor":{{"version":"{ver}","toolchain":"stable","spec": "0.23"}},
                "functions":[{{"fn":"a","inferred":["Fs"]}},
                             {{"fn":"b","inferred":[]}},
                             {{"fn":"b","inferred":["Net"]}}]}}"#
        );
        // prefix form: `<value>.<crate>.scan.json`
        std::fs::write(d.join("base.mycrate.scan.json"), report(&this_build)).unwrap();
        let pre = d.join("base").to_string_lossy().into_owned();
        match check_baseline(&pre, ".", "mycrate", &all, &inferred, false, false) {
            BaselineOutcome::Checked(v) => {
                assert_eq!(v.len(), 1, "only the real gain flags: {v:?}",
                    v = v.iter().map(|x| x.detail.clone()).collect::<Vec<_>>());
                assert_eq!(v[0].rule, "AS-EFF-005");
                assert_eq!(v[0].func, "a");
                assert_eq!(v[0].effects, vec!["Exec".to_string()]);
                assert!(v[0].detail.contains("`a` gained effect { Exec }"), "{}", v[0].detail);
            }
            _ => panic!("a valid same-build baseline must be evaluated"),
        }
        // direct-file form resolves the same way
        let direct = d.join("base.mycrate.scan.json").to_string_lossy().into_owned();
        assert!(matches!(check_baseline(&direct, ".", "mycrate", &all, &inferred, false, false),
            BaselineOutcome::Checked(v) if v.len() == 1));
        // version mismatch / missing provenance / empty value → Invalid (exit 2, never evaluated)
        std::fs::write(d.join("stale.mycrate.scan.json"), report("scan-0.0.1")).unwrap();
        let stale = d.join("stale").to_string_lossy().into_owned();
        assert!(matches!(check_baseline(&stale, ".", "mycrate", &all, &inferred, false, false), BaselineOutcome::Invalid));
        std::fs::write(d.join("bare.mycrate.scan.json"), r#"[{"fn":"a","inferred":["Fs"]}]"#).unwrap();
        let bare = d.join("bare").to_string_lossy().into_owned();
        assert!(matches!(check_baseline(&bare, ".", "mycrate", &all, &inferred, false, false), BaselineOutcome::Invalid));
        assert!(matches!(check_baseline("", ".", "mycrate", &all, &inferred, false, false), BaselineOutcome::Invalid));
        // absent file → Inactive (note; exit unchanged)
        let absent = d.join("nosuch").to_string_lossy().into_owned();
        assert!(matches!(check_baseline(&absent, ".", "mycrate", &all, &inferred, false, false), BaselineOutcome::Inactive));
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn baseline_unknown_ratchet_grandfathers_and_fails_new_unknown() {
        // ⟨unknown-ratchet⟩ OPT-IN (config `unknown-ratchet` / CANDOR_UNKNOWN_RATCHET) on the AS-EFF-005
        // guard — candor-java Policy.checkBaseline is the model. Baseline: X is already Unknown, Y is pure.
        // Current: X is STILL Unknown (grandfathered — no gain), Y is NOW Unknown (a NEW blind spot). The
        // ratchet OFF must be byte-identical to the ⟨0.16⟩ advisory posture (0 violations); the ratchet ON
        // fails EXACTLY Y (the newly-introduced Unknown), never X (already Unknown ⇒ grandfathered).
        let d = std::env::temp_dir().join(format!("candor-scan-blratchet-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        let this_build = format!("scan-{}", env!("CARGO_PKG_VERSION"));
        // Y must be an EXISTING function (present in the baseline report) so its new Unknown reads as a gain,
        // not as exempt new code — record it as a pure entry (empty inferred). X carries Unknown already.
        let report = format!(
            r#"{{"candor":{{"version":"{this_build}","toolchain":"stable","spec": "0.23"}},
                "functions":[{{"fn":"x","inferred":["Unknown"]}},
                             {{"fn":"y","inferred":[]}}]}}"#
        );
        std::fs::write(d.join("base.mycrate.scan.json"), &report).unwrap();
        let pre = d.join("base").to_string_lossy().into_owned();
        let all: Vec<String> = vec!["x".into(), "y".into()];
        let mut inferred: HashMap<String, BTreeSet<&'static str>> = HashMap::new();
        inferred.insert("x".into(), ["Unknown"].into_iter().collect()); // still Unknown — grandfathered
        inferred.insert("y".into(), ["Unknown"].into_iter().collect()); // NEW Unknown — fails under ratchet
        // ratchet OFF: an Unknown-only gain stays advisory ⇒ zero violations (byte-identical to ⟨0.16⟩).
        match check_baseline(&pre, ".", "mycrate", &all, &inferred, false, false) {
            BaselineOutcome::Checked(v) => assert!(v.is_empty(), "ratchet OFF must not flag an Unknown gain: {v:?}",
                v = v.iter().map(|x| x.detail.clone()).collect::<Vec<_>>()),
            _ => panic!("a valid same-build baseline must be evaluated"),
        }
        // ratchet ON: exactly Y (the newly-introduced Unknown) fails; X (already Unknown) is grandfathered.
        match check_baseline(&pre, ".", "mycrate", &all, &inferred, true, false) {
            BaselineOutcome::Checked(v) => {
                assert_eq!(v.len(), 1, "ratchet ON must flag EXACTLY the new Unknown: {v:?}",
                    v = v.iter().map(|x| x.detail.clone()).collect::<Vec<_>>());
                assert_eq!(v[0].rule, "AS-EFF-005");
                assert_eq!(v[0].func, "y");
                assert_eq!(v[0].effects, vec!["Unknown".to_string()]);
                assert!(v[0].detail.contains("unknown-ratchet"), "{}", v[0].detail);
            }
            _ => panic!("a valid same-build baseline must be evaluated"),
        }
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn toml_primitives_tolerate_spacing_and_comments() {
        // `toml_section` survives the line-based -> real-TOML migration (deps.rs's tier note) because
        // `lang.rs`'s `parse_features` still uses it for `[features]`.
        assert_eq!(toml_section("[ workspace ]"), Some("workspace"));
        assert_eq!(toml_section("[package]"), Some("package"));
        assert_eq!(toml_section("name = \"x\""), None);
        // read_crate_name is now a real-TOML reader (`read_manifest_table`) — still tolerates a spaced
        // header + a trailing comment, because the `toml` crate does.
        let d = std::env::temp_dir().join(format!("candor-scan-tomlhdr-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        std::fs::write(d.join("Cargo.toml"), "[ package ]\nname = \"spaced-crate\"  # trailing\n").unwrap();
        assert_eq!(read_crate_name(&d).as_deref(), Some("spaced_crate"));
        let _ = std::fs::remove_dir_all(&d);
    }

    /// ⟨finding 3, 2026-08-29⟩ `read_crate_name` via `package.name = "…"` (dotted key) — the SAME
    /// structure as `[package]\nname = "…"` to a real parser. Pre-fix this returned `None`, and
    /// `scan_target`'s `if read_crate_name(dir).is_some() { dirs.push(dir) }` silently dropped the root
    /// package from the workspace fan-out: reproduced live, a root `pub fn root_net()` performing a real
    /// `TcpStream::connect` vanished completely (absent from `functions`, absent from `excluded`, zero
    /// stderr) and `--policy "deny Net"` exited 0.
    #[test]
    fn read_crate_name_recognizes_dotted_key_package_table() {
        let d = std::env::temp_dir().join(format!("candor-scan-dottedpkg-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        std::fs::write(d.join("Cargo.toml"), "package.name = \"root-pkg\"\npackage.version = \"0.1.0\"\n")
            .unwrap();
        assert_eq!(read_crate_name(&d).as_deref(), Some("root_pkg"),
            "a dotted-key `package.name` must be read exactly like a header-table `[package]` one");
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
    fn workspace_members_reads_inline_and_multiline_arrays_via_real_toml() {
        // Replaces the deleted `toml_string_array`'s direct unit test: the members/exclude ARRAY reading
        // is now exercised through `workspace_members` itself (real TOML), the only production caller.
        let d = std::env::temp_dir().join(format!("candor-scan-wsarrays-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(d.join("crates/a")).unwrap();
        std::fs::create_dir_all(d.join("crates/b")).unwrap();
        std::fs::create_dir_all(d.join("eval")).unwrap();
        for m in ["crates/a", "crates/b", "eval"] {
            std::fs::write(d.join(m).join("Cargo.toml"), "[package]\nname = \"m\"\n").unwrap();
        }
        std::fs::write(d.join("Cargo.toml"),
            "[package]\nname = \"x\"\n\n[workspace]\nmembers = [\"crates/a\", \"crates/b\", \"eval\"]\n\
             exclude = [\n  \"eval\",\n  \"sample\",\n]\n"
        ).unwrap();
        let got: Vec<String> = workspace_members(&d)
            .into_iter()
            .map(|p| p.strip_prefix(&format!("{}/", d.to_string_lossy())).unwrap().to_string())
            .collect();
        assert_eq!(got, vec!["crates/a", "crates/b"], "multi-line `exclude` array honoured via real TOML");
        let _ = std::fs::remove_dir_all(&d);
    }

    /// ⟨finding 3, 2026-08-29⟩ THE REPRODUCED DEFECT: a workspace declared via dotted keys
    /// (`workspace.members = […]`) rather than a `[workspace]` header — real, `cargo metadata`-resolved
    /// TOML — made `has_workspace_table` return `false` and `workspace_members` return empty. Pre-fix,
    /// scanning this root fell through to a single package-less crate scan: `analyzed.count: 0`, zero
    /// stderr, and a `--policy "deny Net"` gate over the member's real Net call printed "policy ✓" at
    /// exit 0. Fixed by moving both functions onto the real `toml` parser already used for the
    /// dependency-identity surface (⟨caca530⟩/⟨75045f0⟩) — a dotted key and a header section parse to
    /// the identical structure there.
    #[test]
    fn dotted_key_workspace_declaration_is_recognized() {
        let d = std::env::temp_dir().join(format!("candor-scan-dottedws-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(d.join("member/src")).unwrap();
        std::fs::write(d.join("Cargo.toml"), "workspace.members = [\"member\"]\n").unwrap();
        std::fs::write(d.join("member/Cargo.toml"), "[package]\nname = \"member\"\nversion = \"0.1.0\"\n")
            .unwrap();
        std::fs::write(d.join("member/src/lib.rs"),
            "pub fn reach_out() { let _ = std::net::TcpStream::connect(\"evil.example.com:80\"); }\n"
        ).unwrap();
        assert!(has_workspace_table(&d), "a dotted-key `workspace.members` must be recognized as [workspace]");
        let members: Vec<String> = workspace_members(&d)
            .into_iter()
            .map(|p| p.strip_prefix(&format!("{}/", d.to_string_lossy())).unwrap().to_string())
            .collect();
        assert_eq!(members, vec!["member"], "the dotted-key member list must resolve exactly like a header one");
        // End-to-end: the CLI-visible shape a real `--policy` run would see.
        let idx = load_dep_reports(None);
        let prefix = d.join("out/r").to_string_lossy().into_owned();
        let rc = scan_target(&d.to_string_lossy(), prefix.clone(), false, false, None, None, &idx, &crate::gate::begin_run());
        assert_eq!(rc, 0, "scan should succeed");
        let report = std::fs::read_to_string(format!("{prefix}.member.scan.json"))
            .expect("the dotted-key workspace must fan out to its member, not vanish into a single-crate scan");
        assert!(report.contains("reach_out") && report.contains("Net"),
            "the member's real Net call must be analyzed, not silently skipped: {report}");
        let _ = std::fs::remove_dir_all(&d);
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
        let rc = scan_target(&d.to_string_lossy(), prefix.clone(), false, false, None, None, &idx, &crate::gate::begin_run());
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
            policy: None, baseline: None, ws_member: false, quiet: true, deps_idx: &idx, peek_excluded: false,
        }, &crate::gate::begin_run());
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
            prefix, want_json: false, include_tests: false, policy: None, baseline: None, ws_member: false, quiet: true, deps_idx: &idx, peek_excluded: false,
        }, &crate::gate::begin_run());
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
    fn builder_chain_entries_over_approximate_for_the_syntactic_scanner() {
        // The real-world oracle caught candor-scan silent-pure'ing builder chains whose effect candor-
        // classify keys on a terminal VERB it can't type-resolve: `duct::cmd!(...).run()` (macro entry) and
        // `ureq::get(url)...call()` (fn entry). Over-approximate the ENTRY here (scan-only); the shared
        // classifier keeps the entry pure so the DEEP engine (which types the verb) stays precise.
        assert_eq!(scan_builder_entry_effect("duct", "duct::cmd"), Some("Exec"));
        assert_eq!(scan_builder_entry_effect("duct", "duct::sh"), Some("Exec"));
        assert_eq!(scan_builder_entry_effect("ureq", "ureq::get"), Some("Net"));
        assert_eq!(scan_builder_entry_effect("ureq", "ureq::post"), Some("Net"));
        assert_eq!(scan_builder_entry_effect("sqlx", "sqlx::query"), Some("Db"));
        assert_eq!(scan_builder_entry_effect("diesel", "diesel::sql_query"), Some("Db"));
        // terminal verbs stay classify()'s job; an unrelated path is None:
        assert_eq!(scan_builder_entry_effect("duct", "duct::Expression::run"), None);
        assert_eq!(scan_builder_entry_effect("ureq", "ureq::Request::call"), None);
        assert_eq!(scan_builder_entry_effect("std", "std::process::Command::new"), None);
        // invariant the deep engine relies on stays intact (entries pure in the SHARED classifier):
        assert_eq!(candor_classify::classify("duct", "duct::cmd"), None);
        assert_eq!(candor_classify::classify("ureq", "ureq::get"), None);
    }

    #[test]
    fn macro_invocation_never_mints_a_local_edge() {
        // REGRESSION (review F1): a crate-LOCAL qualified macro (`crate::helpers::trace!`) expands to
        // `helpers::trace`, KEEPING its `::` — so before the `is_macro` guard it mis-linked to a same-named
        // LOCAL fn and FABRICATED that fn's effect onto a pure caller (the phantom-edge precision failure). The
        // guard must be SURGICAL: a genuine (non-macro) call to the same fn STILL inherits the effect, and
        // a genuine external classified emit-macro (`log::info!`) STILL attributes its effect.
        let idx = load_dep_reports(None);
        let run = |name: &str, src: &str| -> serde_json::Value {
            let d = std::env::temp_dir().join(format!("candor-macroedge-{name}-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&d);
            std::fs::create_dir_all(d.join("src")).unwrap();
            std::fs::write(d.join("Cargo.toml"), format!("[package]\nname = \"{name}\"\n")).unwrap();
            std::fs::write(d.join("src/lib.rs"), src).unwrap();
            let prefix = d.join("out/r").to_string_lossy().into_owned();
            let (rc, body) = scan_one(&d.to_string_lossy(), ScanOpts {
                prefix, want_json: true, include_tests: false, policy: None, baseline: None, ws_member: false, quiet: true, deps_idx: &idx, peek_excluded: false,
            }, &crate::gate::begin_run());
            assert_eq!(rc, 0);
            let v = serde_json::from_str(&body.unwrap()).unwrap();
            let _ = std::fs::remove_dir_all(&d);
            v
        };
        let eff = |v: &serde_json::Value, needle: &str| -> Vec<String> {
            v["functions"].as_array().into_iter().flatten()
                .filter(|f| f["fn"].as_str().is_some_and(|q| q.contains(needle)))
                .flat_map(|f| f["inferred"].as_array().into_iter().flatten()
                    .filter_map(|e| e.as_str().map(String::from)).collect::<Vec<_>>()).collect()
        };
        let local = "mod helpers { pub fn trace() { let _ = std::fs::read(\"/x\"); } }\n";
        // The bug: a pure fn whose ONLY body is the LOCAL macro must NOT inherit `helpers::trace`'s Fs.
        let bug = run("macedge", &format!("{local}pub fn pure_caller() {{ crate::helpers::trace!(); }}"));
        assert!(eff(&bug, "pure_caller").is_empty(),
                "macro invocation FABRICATED a same-named local fn's effect onto a pure caller:\n{bug}");
        // Surgical: a GENUINE (non-macro) call to the same local fn STILL edges and inherits Fs.
        let real = run("macedge2", &format!("{local}pub fn real_caller() {{ crate::helpers::trace(); }}"));
        assert!(eff(&real, "real_caller").contains(&"Fs".to_string()),
                "the is_macro guard wrongly suppressed a genuine local fn edge:\n{real}");
        // A genuine EXTERNAL classified emit-macro still attributes its effect (the intended new behavior).
        let ext = run("macedge3", "pub fn logs() { log::info!(\"hi\"); }");
        assert!(eff(&ext, "logs").contains(&"Log".to_string()),
                "an external classified emit-macro must still attribute its effect:\n{ext}");
    }

    #[test]
    fn write_macro_charges_the_local_writer_side() {
        // R14 cross-engine sweep (scan): `write!(w, ...)` to a custom `fmt::Write` writer dropped the
        // writer's effectful `write_str` — silent-pure (the deep engine had this too, fixed as HOLE 2c).
        // The writer is the arg BEFORE the format string; charge its `write_str`/`write`. A std writer
        // (`String`) must light nothing (no fabrication), and a leading `assert_eq!` operand must never be
        // mistaken for a writer (the charge is gated to the write/writeln family).
        let idx = load_dep_reports(None);
        let run = |name: &str, src: &str| -> serde_json::Value {
            let d = std::env::temp_dir().join(format!("candor-wr-{name}-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&d);
            std::fs::create_dir_all(d.join("src")).unwrap();
            std::fs::write(d.join("Cargo.toml"), format!("[package]\nname = \"{name}\"\n")).unwrap();
            std::fs::write(d.join("src/lib.rs"), src).unwrap();
            let prefix = d.join("out/r").to_string_lossy().into_owned();
            let (rc, body) = scan_one(&d.to_string_lossy(), ScanOpts {
                prefix, want_json: true, include_tests: false, policy: None, baseline: None, ws_member: false, quiet: true, deps_idx: &idx, peek_excluded: false,
            }, &crate::gate::begin_run());
            assert_eq!(rc, 0);
            let v = serde_json::from_str(&body.unwrap()).unwrap();
            let _ = std::fs::remove_dir_all(&d);
            v
        };
        let eff = |v: &serde_json::Value, needle: &str| -> Vec<String> {
            v["functions"].as_array().into_iter().flatten()
                .filter(|f| f["fn"].as_str().is_some_and(|q| q.contains(needle)))
                .flat_map(|f| f["inferred"].as_array().into_iter().flatten()
                    .filter_map(|e| e.as_str().map(String::from)).collect::<Vec<_>>()).collect()
        };
        let w = "use std::fmt::Write as _;\nstruct Loud;\nimpl std::fmt::Write for Loud { fn write_str(&mut self, _s: &str) -> std::fmt::Result { let _ = std::fs::read(\"/x\"); Ok(()) } }\n";
        // effectful local fmt::Write writer -> Fs (the bug: this was silent-pure)
        let hit = run("wrfmt", &format!("{w}pub fn via_write(w: &mut Loud) {{ let _ = write!(w, \"hi {{}}\", 1); }}"));
        assert!(eff(&hit, "via_write").contains(&"Fs".to_string()),
                "write! to a local effectful fmt::Write writer must charge the writer side:\n{hit}");
        // std String writer -> pure (no fabrication)
        let pure = run("wrstr", "use std::fmt::Write as _;\npub fn via_str(s: &mut String) { let _ = write!(s, \"hi {}\", 1); }");
        assert!(eff(&pure, "via_str").is_empty(),
                "write! to a std String writer must stay pure:\n{pure}");
        // assert_eq! leads with operands, not a writer — must NOT be charged (gated to write/writeln)
        let asrt = run("wrassert", &format!("{w}#[derive(Debug, PartialEq)]\nstruct Tag;\npub fn via_assert(a: Tag, b: Tag) {{ assert_eq!(a, b, \"ctx {{}}\", 1); }}"));
        assert!(eff(&asrt, "via_assert").is_empty(),
                "an assert_eq! operand was wrongly charged as a writer:\n{asrt}");
    }

    #[test]
    fn qself_call_never_mints_a_bare_leaf_local_edge() {
        // REGRESSION (qself hole): `path_to_string` drops the qself receiver type, so an inherent-form
        // fully-qualified assoc call `<Vec<u8>>::new()` collapses to the BARE leaf `new`, which the by_leaf
        // route mis-linked to a unique local `new`/`dump`, FABRICATING its effect onto a pure path. The fix
        // RESTORES the receiver type (`Vec::new`), staying precise in BOTH directions: `<Vec<u8>>::new()`
        // finds no local `Vec` (no fabrication) AND `<Daemon>::new()` still resolves to the local effectful
        // `Daemon::new` (no under-report — the blunt `method:true` suppress would have lost this).
        let idx = load_dep_reports(None);
        let run = |name: &str, src: &str| -> serde_json::Value {
            let d = std::env::temp_dir().join(format!("candor-qself-{name}-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&d);
            std::fs::create_dir_all(d.join("src")).unwrap();
            std::fs::write(d.join("Cargo.toml"), format!("[package]\nname = \"{name}\"\n")).unwrap();
            std::fs::write(d.join("src/lib.rs"), src).unwrap();
            let prefix = d.join("out/r").to_string_lossy().into_owned();
            let (rc, body) = scan_one(&d.to_string_lossy(), ScanOpts {
                prefix, want_json: true, include_tests: false, policy: None, baseline: None, ws_member: false, quiet: true, deps_idx: &idx, peek_excluded: false,
            }, &crate::gate::begin_run());
            assert_eq!(rc, 0);
            let v: serde_json::Value = serde_json::from_str(&body.unwrap()).unwrap();
            let _ = std::fs::remove_dir_all(&d);
            v
        };
        let eff = |v: &serde_json::Value, needle: &str| -> Vec<String> {
            v["functions"].as_array().into_iter().flatten()
                .filter(|f| f["fn"].as_str().is_some_and(|q| q.contains(needle)))
                .flat_map(|f| f["inferred"].as_array().into_iter().flatten()
                    .filter_map(|e| e.as_str().map(String::from)).collect::<Vec<_>>()).collect()
        };
        // A unique local `new` doing Exec; the pure builder uses idiomatic `<Vec<u8>>::new()`.
        let bug = run("qselfnew",
            "use std::process::Command;\npub struct Daemon;\nimpl Daemon { pub fn new() -> Self { Command::new(\"/bin/sh\").status().unwrap(); Daemon } }\npub fn pure_build() -> Vec<u8> { let mut v = <Vec<u8>>::new(); v.push(1); v }");
        assert!(eff(&bug, "pure_build").is_empty(),
                "a qself assoc call FABRICATED a same-named local fn's effect onto a pure caller:\n{bug}");
        // Surgical: a genuine bare free-fn call STILL edges and inherits the effect.
        let real = run("qselfreal",
            "use std::fs;\npub fn dump() { fs::write(\"/x\", b\"d\").unwrap(); }\npub fn real_caller() { dump(); }");
        assert!(eff(&real, "real_caller").contains(&"Fs".to_string()),
                "the qself guard wrongly suppressed a genuine bare free-fn edge:\n{real}");
        // A qualified-tail qself into a LOCAL trait default still links (the accepted trait-default band).
        let band = run("qselfband",
            "use std::fs;\npub trait Marker { fn go() { fs::write(\"/x\", b\"d\").unwrap(); } }\npub fn via_trait_ufcs() { <SomeType as Marker>::go(); }");
        assert!(eff(&band, "via_trait_ufcs").contains(&"Fs".to_string()),
                "a qualified-tail qself into a local trait default must still link:\n{band}");
        // PRECISION (what the blunt method:true suppress would have lost): an INHERENT qself on a LOCAL type
        // whose assoc fn is effectful must STILL propagate — the restored `Daemon::new` tail resolves it.
        let local = run("qselflocal",
            "use std::process::Command;\npub struct Daemon;\nimpl Daemon { pub fn new() -> Self { Command::new(\"/bin/sh\").status().unwrap(); Daemon } }\npub fn boot() { let _d = <Daemon>::new(); }");
        assert!(eff(&local, "boot").contains(&"Exec".to_string()),
                "an inherent qself on a LOCAL effectful assoc fn must NOT be under-reported:\n{local}");
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

    // A REFLECTIVE guard on the `--incremental` cache key. `decl_index_digest` decides whether a cached
    // file's Pass-B FnInfos may be reused; a `MergedDecls` field that steers resolution but is MISSING
    // from the digest = a stale cache silently returning unsound effect sets.
    //
    // THIS GUARD HAD A HOLE, AND IT WAS THE HOLE ITS OWN COMMENT SAID WAS IMPOSSIBLE (standing bar item 9:
    // a comment that states a justification is an assertion, and it will be believed). It read "Add a
    // field → add its `_` binding AND a mutator case, or the build/test fails." The two halves were
    // maintained as SEPARATE lists: an exhaustive destructure, and a hand-written mutator table. Adding a
    // field broke only the destructure, so binding it `_` restored the build and the test went green with
    // the field never exercised. `deref_target` had been sitting in exactly that state — bound, unmutated,
    // and genuinely absent from the digest.
    //
    // It cost nothing YET, and only by an accident of layout (item 0b): the auto-deref chase reads
    // `merged.deref_target` live in `scan_one` instead of baking it into an FnInfo, so a stale entry could
    // not replay it. Every other receiver-typing rung of the last month landed in `CallCollector`; the day
    // this one follows them, a `type Target = …` edit becomes a replayed purity claim.
    //
    // The fix is to make the promise TRUE rather than to add the one missing row: ONE list now generates
    // both halves, so a field cannot have a binding without a mutator. Adding a field breaks compilation
    // until you write `name => |m| { … }`, which supplies the mutator in the same keystroke. (The swift
    // catch-all lesson, ported: stop maintaining an enumeration in parallel with the thing it enumerates,
    // and make the failure mode "won't build" instead of "passes quietly".)
    #[test]
    fn every_merged_decls_field_is_folded_into_the_digest() {
        /// ONE list, two guarantees: it destructures `MergedDecls` exhaustively (no `..`, so a new field
        /// stops the build here) and it builds the mutator table from the SAME names.
        macro_rules! digest_field_table {
            ($($name:ident => $mutate:expr),+ $(,)?) => {{
                let MergedDecls { $($name: _),+ } = MergedDecls::default();
                let t: Vec<(&str, fn(&mut MergedDecls))> = vec![$((stringify!($name), $mutate)),+];
                t
            }};
        }
        // One mutator per field — each touches exactly that field and nothing else.
        let table = digest_field_table! {
            fields => |m| { m.fields.entry("S".into()).or_default().insert("f".into(), "T".into()); },
            field_elem => |m| { m.field_elem.entry("S".into()).or_default().insert("f".into(), "E".into()); },
            field_elem_trait => |m| { m.field_elem_trait.entry("S".into()).or_default().insert("f".into(), vec!["Tr".into()]); },
            rets => |m| { m.rets.insert("f".into(), Some("T".into())); },
            enum_tmp => |m| { m.enum_tmp.insert("v".into(), Some("E".into())); },
            // R77: a DISPATCH-typed single-field tuple-variant payload's trait leaves — the Vec-valued
            // twin of `enum_tmp`, for a `dyn`/`impl`/bounded-generic payload `type_path` can't name.
            enum_variant_traits => |m| { m.enum_variant_traits.insert("v".into(), Some(vec!["Fn".into()])); },
            trait_impls => |m| { m.trait_impls.entry("Tr".into()).or_default().push("Ty".into()); },
            trait_decls => |m| { m.trait_decls.entry("Tr".into()).or_default().count += 1; },
            trait_fields => |m| { m.trait_fields.entry("S".into()).or_default().insert("f".into(), vec!["b".into()]); },
            prim_aliases => |m| { m.prim_aliases.insert("A".into()); },
            extern_fns => |m| { m.extern_fns.insert("system".into()); },
            drop_types => |m| { m.drop_types.insert("Guard".into()); },
            // `impl Deref for W { type Target = U }` — `w.leaf()` dispatches to `U::leaf`, so a change
            // here re-resolves every auto-deref call site. This is the row that was missing.
            deref_target => |m| { m.deref_target.insert("W".into(), "Inner".into()); },
            lazy_statics => |m| { m.lazy_statics.insert("CONFIG".into()); },
            callable_statics => |m| { m.callable_statics.insert("CB".into()); },
            const_strings => |m| { m.const_strings.insert("API_BASE".into(), "https://api.openai.com".into()); },
            local_macros => |m| { m.local_macros.insert("do_io".into(), "() => { fs::write(\"/x\", b\"y\"); }".into()); },
            blanket_methods => |m| { m.blanket_methods.insert("ext".into(), "T".into()); },
            root_reexports => |m| { m.root_reexports.insert("net".into(), "sqlx_core::driver_prelude::net".into()); },
            // `pub use self::platform::*` in a SUBMODULE — a call `imp::doit()` in ANOTHER file resolves
            // through it, so a change to the edge set re-resolves that file's calls.
            reexports => |m| { m.reexports.push(Reexport { module: "imp".into(), from: vec!["imp::platform".into()], name: "*".into(), alias: "*".into() }); },
            // R99: `mod facade { pub use std::process::Command; }` / `pub type Cmd = …` — seeded into
            // EVERY file's `use` map, so a change re-resolves `facade::Command::new` in other files.
            mod_aliases => |m| { m.mod_aliases.insert("facade::Command".into(), "std::process::Command".into()); },
        };
        let empty = decl_index_digest(&MergedDecls::default());
        for (name, mutate) in table {
            let mut m = MergedDecls::default();
            mutate(&mut m);
            assert_ne!(
                decl_index_digest(&m), empty,
                "MergedDecls.{name} changes the index but NOT the digest — the --incremental cache would \
                 reuse stale FnInfos. Fold `{name}` into decl_index_digest().",
            );
        }
    }

    /// R77 STRUCT-VARIANT FIELDS made a DELIBERATE design choice the table above does not exercise:
    /// rather than adding a new `MergedDecls` field, per-field data is written into the SAME `enum_tmp`/
    /// `enum_variant_traits` maps the tuple-variant route already uses, under a composite
    /// `"VariantLeaf::field"` key (see `decls.rs`'s `syn::Item::Enum` arm and
    /// `collector::enum_struct_variant_bindings`) — reuse, not a second index, per the standing family
    /// rule against two hand-rolled paths answering one question.
    ///
    /// Because that reuses an EXISTING field, the generic rows above already prove the map is hashed at
    /// all; they do not prove the digest is sensitive to the composite key's CONTENT, only that it is
    /// non-empty. This proves the content-sensitivity specifically: two struct-variant-field entries for
    /// the SAME variant leaf but DIFFERENT fields (`Msg::Both { f, g }`'s exact shape) must hash
    /// differently, and adding a second field's key must move the digest again. (Manually confirmed to
    /// fail during development by hashing only the pre-`::` prefix of each key — i.e. collapsing every
    /// field of one variant onto its leaf — which made `with_f` and `with_g` collide; that collapsed form
    /// is deliberately NOT the shape being asserted here.)
    #[test]
    fn r77_struct_variant_field_composite_key_moves_the_digest_by_content_not_just_presence() {
        fn build(extra: &[(&str, &str)]) -> MergedDecls {
            let mut m = MergedDecls::default();
            m.enum_tmp.insert("Cb".into(), Some("Real".into())); // an ordinary tuple-variant leaf, unrelated
            for (k, v) in extra {
                m.enum_tmp.insert((*k).into(), Some((*v).into()));
            }
            m
        }
        let base = decl_index_digest(&build(&[]));
        let with_f = decl_index_digest(&build(&[("Cb::f", "Box")]));
        let with_g = decl_index_digest(&build(&[("Cb::g", "Box")]));
        let with_f_and_g = decl_index_digest(&build(&[("Cb::f", "Box"), ("Cb::g", "String")]));
        assert_ne!(base, with_f, "a struct-variant composite key must move the digest at all");
        assert_ne!(with_f, with_g, "two DIFFERENT fields of the SAME variant must hash differently — \
                                     Msg::Both {{ f, g }} depends on this to invalidate correctly");
        assert_ne!(with_f, with_f_and_g, "adding a SECOND field's composite key must also move the digest");
    }

    // ── cfg_eval: the 3-valued nested `cfg(all/any/not)` evaluator ──────────────────────────────────
    // (Measured 0-covered — the evaluator had never executed under test. These pin the Kleene fold and
    // its conservative direction: only a DEFINITE false may skip an item; anything unresolvable is kept.)

    /// Drive `cfg_eval` over the `#[cfg(...)]` of a tiny parsed fn with EXPLICIT active/declared
    /// feature sets — the evaluator is pure given these; no global CFG_FEATURES state is touched
    /// (so this can never race the parallel `scan_one` tests).
    fn eval_cfg(cfg: &str, active: &[&str], declared: &[&str]) -> Option<bool> {
        let item: syn::ItemFn = syn::parse_str(&format!("#[cfg({cfg})] fn f() {{}}")).unwrap();
        let attr = item.attrs.iter().find(|a| a.path().is_ident("cfg")).unwrap();
        let active: std::collections::HashSet<String> = active.iter().map(|s| s.to_string()).collect();
        let declared: std::collections::HashSet<String> =
            declared.iter().map(|s| s.to_string()).collect();
        let mut verdict = None;
        let _ = attr.parse_nested_meta(|m| {
            verdict = cfg_eval(&m, &active, &declared);
            Ok(())
        });
        verdict
    }

    #[test]
    fn cfg_eval_feature_predicate_is_three_valued() {
        // active ⇒ definitely compiled; declared-but-inactive ⇒ definitely OUT (the one skippable
        // verdict); undeclared ⇒ UNKNOWN — a dependent crate could enable it, so the item is KEPT.
        let d = &["on", "off"];
        assert_eq!(eval_cfg(r#"feature = "on""#, &["on"], d), Some(true));
        assert_eq!(eval_cfg(r#"feature = "off""#, &["on"], d), Some(false));
        assert_eq!(eval_cfg(r#"feature = "mystery""#, &["on"], d), None);
    }

    #[test]
    fn cfg_eval_unknown_predicates_stay_unknown() {
        // Target/platform predicates and `test` are not feature-resolvable → None (keep the item,
        // never skip). `test` is deliberately left to `is_cfg_test`.
        for p in [r#"target_os = "linux""#, "unix", "windows", "test", "doc"] {
            assert_eq!(eval_cfg(p, &["on"], &["on"]), None, "predicate `{p}` must stay unknown");
        }
    }

    #[test]
    fn cfg_eval_not_folds_kleene() {
        let (a, d) = (&["on"][..], &["on", "off"][..]);
        assert_eq!(eval_cfg(r#"not(feature = "on")"#, a, d), Some(false));
        assert_eq!(eval_cfg(r#"not(feature = "off")"#, a, d), Some(true));
        // ¬unknown is still unknown — a `not(unix)` must NOT flip into a definite skip.
        assert_eq!(eval_cfg("not(unix)", a, d), None);
    }

    #[test]
    fn cfg_eval_all_folds_kleene() {
        let (a, d) = (&["on"][..], &["on", "off"][..]);
        assert_eq!(eval_cfg(r#"all(feature = "on", feature = "on")"#, a, d), Some(true));
        // ANY definite false wins — even next to an unknown sibling.
        assert_eq!(eval_cfg(r#"all(feature = "on", feature = "off")"#, a, d), Some(false));
        assert_eq!(eval_cfg(r#"all(unix, feature = "off")"#, a, d), Some(false));
        // true ∧ unknown = unknown (kept — the conservative direction).
        assert_eq!(eval_cfg(r#"all(feature = "on", unix)"#, a, d), None);
    }

    #[test]
    fn cfg_eval_any_folds_kleene() {
        let (a, d) = (&["on"][..], &["on", "off"][..]);
        // ANY definite true wins — even next to an unknown sibling.
        assert_eq!(eval_cfg(r#"any(feature = "off", feature = "on")"#, a, d), Some(true));
        assert_eq!(eval_cfg(r#"any(unix, feature = "on")"#, a, d), Some(true));
        // all-children-false is the only definite false.
        assert_eq!(eval_cfg(r#"any(feature = "off", not(feature = "on"))"#, a, d), Some(false));
        // false ∨ unknown = unknown (kept).
        assert_eq!(eval_cfg(r#"any(feature = "off", unix)"#, a, d), None);
    }

    #[test]
    fn cfg_eval_nested_combinations() {
        let (a, d) = (&["on"][..], &["on", "off"][..]);
        // all(any(off, on), not(off)) = all(T, T) = T
        assert_eq!(
            eval_cfg(r#"all(any(feature = "off", feature = "on"), not(feature = "off"))"#, a, d),
            Some(true)
        );
        // any(all(on, off), off) = any(F, F) = F — the nested definite skip.
        assert_eq!(
            eval_cfg(r#"any(all(feature = "on", feature = "off"), feature = "off")"#, a, d),
            Some(false)
        );
        // not(all(on, unix)) = ¬unknown = unknown — nesting must not manufacture certainty.
        assert_eq!(eval_cfg(r#"not(all(feature = "on", unix))"#, a, d), None);
        // any(all(not(off), on), target_os) = any(T, unknown) = T.
        assert_eq!(
            eval_cfg(r#"any(all(not(feature = "off"), feature = "on"), target_os = "linux")"#, a, d),
            Some(true)
        );
    }

    #[test]
    fn push_quoted_pulls_double_quoted_tokens() {
        let mut out = Vec::new();
        push_quoted(r#""std", "alloc-dep""#, &mut out);
        assert_eq!(out, vec!["std", "alloc-dep"]);
        // an UNTERMINATED trailing quote is dropped whole — never a half-captured token or a panic.
        push_quoted(r#""ok", "dangling"#, &mut out);
        assert_eq!(out, vec!["std", "alloc-dep", "ok"]);
        // no quotes → nothing appended.
        push_quoted("plain ] tokens", &mut out);
        assert_eq!(out.len(), 3);
        // the empty token `""` is a legal (empty) entry.
        push_quoted(r#""""#, &mut out);
        assert_eq!(out.last().map(String::as_str), Some(""));
    }

    #[test]
    fn non_nominal_types_cannot_carry_local_impls() {
        // `type Alias = <non-nominal>` must not let `Alias::assoc()` link to a same-named local
        // struct's fn (the sled IVec fabrication — see `prim_aliases` in scan.rs).
        let non: &[&str] = &[
            "[u8; 32]", "[u8]", "(A, B)", "*const u8", "&str", "fn(u32) -> u32",
            "u8", "u128", "usize", "i64", "f64", "bool", "char", "str",
        ];
        for t in non {
            let ty: syn::Type = syn::parse_str(t).unwrap();
            assert!(is_non_nominal_type(&ty), "`{t}` must be non-nominal");
        }
        // Nominal (or possibly-nominal) types keep the normal local-impl resolution: a generic path,
        // a user type, a qualified path — and a PRIMITIVE-NAMED segment WITH arguments is not a prim.
        let nominal: &[&str] = &["Vec<u8>", "MyStruct", "std::path::PathBuf", "Option<u8>", "String"];
        for t in nominal {
            let ty: syn::Type = syn::parse_str(t).unwrap();
            assert!(!is_non_nominal_type(&ty), "`{t}` must stay nominal");
        }
    }

    #[test]
    fn annotated_tuple_destructure_types_its_elements() {
        // collector::bind_tuple (never executed under test): `let (r, _): (Runner, u32) = …` must bind
        // `r → Runner` so `r.go()` resolves to the LOCAL effectful method — an effectful call bound via
        // tuple destructuring still attributes. And a REBIND of the same name to a different tuple type
        // must CLEAR the stale binding (else the old type's effect is fabricated onto the new var).
        let src = r#"
struct Runner;
impl Runner { fn go(&self) { std::process::Command::new("ls").status().unwrap(); } }
fn make() -> (Runner, u32) { (Runner, 0) }
pub fn user() { let (r, _): (Runner, u32) = make(); r.go(); }
pub fn rebound() { let (r, _): (Runner, u32) = make(); let (r, _): (u32, u32) = (1, 2); r.go(); }
"#;
        let d = std::env::temp_dir().join(format!("candor-bindtuple-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(d.join("src")).unwrap();
        std::fs::write(d.join("Cargo.toml"), "[package]\nname = \"bt\"\n").unwrap();
        std::fs::write(d.join("src/lib.rs"), src).unwrap();
        let idx = load_dep_reports(None);
        let prefix = d.join("out/r").to_string_lossy().into_owned();
        let (rc, body) = scan_one(&d.to_string_lossy(), ScanOpts {
            prefix, want_json: true, include_tests: false, policy: None, baseline: None, ws_member: false, quiet: true, deps_idx: &idx, peek_excluded: false,
        }, &crate::gate::begin_run());
        assert_eq!(rc, 0);
        let v: serde_json::Value = serde_json::from_str(&body.unwrap()).unwrap();
        let _ = std::fs::remove_dir_all(&d);
        let eff = |needle: &str| -> Vec<String> {
            v["functions"].as_array().into_iter().flatten()
                .filter(|f| f["fn"].as_str().is_some_and(|q| q.contains(needle)))
                .flat_map(|f| f["inferred"].as_array().into_iter().flatten()
                    .filter_map(|e| e.as_str().map(String::from)).collect::<Vec<_>>()).collect()
        };
        assert!(eff("user").contains(&"Exec".to_string()),
                "a tuple-destructured effectful binding must still attribute (bind_tuple):\n{v}");
        assert!(!eff("rebound").contains(&"Exec".to_string()),
                "a tuple REBIND must clear the stale Runner binding — Exec here is fabricated:\n{v}");
    }

    #[test]
    fn dirs_cargo_registry_src_honours_cargo_home() {
        // The --deps registry locator (was 0-covered): CARGO_HOME wins over ~/.cargo; the result is the
        // set of `registry/src/<index-hash>/` DIRECTORIES (a stray file is not an index); a CARGO_HOME
        // with no registry at all yields empty — the "every dep missing a checkout" disclosure path.
        // (No other test reads CARGO_HOME, so the temporary set_var cannot race a parallel test.)
        let ch = std::env::temp_dir().join(format!("candor-regsrc-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&ch);
        let src = ch.join("registry").join("src");
        std::fs::create_dir_all(src.join("index.crates.io-aaaa")).unwrap();
        std::fs::create_dir_all(src.join("index.mirror-bbbb")).unwrap();
        std::fs::write(src.join("not-an-index.txt"), "x").unwrap();
        let prior = std::env::var("CARGO_HOME").ok();
        std::env::set_var("CARGO_HOME", &ch);
        let mut roots = dirs_cargo_registry_src();
        roots.sort();
        let empty_home = std::env::temp_dir().join(format!("candor-regsrc-empty-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&empty_home);
        std::env::set_var("CARGO_HOME", &empty_home);
        let none = dirs_cargo_registry_src();
        match prior {
            Some(v) => std::env::set_var("CARGO_HOME", v),
            None => std::env::remove_var("CARGO_HOME"),
        }
        let _ = std::fs::remove_dir_all(&ch);
        let _ = std::fs::remove_dir_all(&empty_home);
        assert_eq!(
            roots,
            vec![src.join("index.crates.io-aaaa"), src.join("index.mirror-bbbb")],
            "exactly the index DIRECTORIES under <CARGO_HOME>/registry/src, files excluded"
        );
        assert!(none.is_empty(), "no registry under CARGO_HOME → empty, not an error");
    }

    #[test]
    fn run_with_deps_without_lockfile_returns_2() {
        // The in-process half of the fail-closed pin (the CLI test asserts the process exit + message):
        // a scan dir with no Cargo.lock returns 2 before touching any registry.
        let d = std::env::temp_dir().join(format!("candor-depsnolock-unit-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(d.join("src")).unwrap();
        std::fs::write(d.join("Cargo.toml"), "[package]\nname = \"nl\"\n").unwrap();
        std::fs::write(d.join("src/lib.rs"), "pub fn f() {}\n").unwrap();
        let rc = run_with_deps(&d.to_string_lossy(), String::new(), true, false, None, None, &crate::gate::begin_run());
        let _ = std::fs::remove_dir_all(&d);
        assert_eq!(rc, 2, "--deps without Cargo.lock must fail closed");
    }

    #[test]
    fn cfg_if_macro_expands_all_arms() {
        // `cfg_if::cfg_if! { if #[cfg(..)] {A} else {B} }` was opaque, so a std::net/std::fs call inside
        // an arm read pure (the sqlx-core `connect_tcp` blind spot). Expand it: walk BOTH arms (the sound
        // all-cfg-branches over-approximation) so both a `std::net::TcpStream::connect` and a
        // `std::fs::read` surface — and the misleading `invisible: ["cfg_if"]` disclosure is gone.
        let v = scan_src_to_json("cfgifboth", "\
            pub fn effect_inside_cfgif() {\n\
                cfg_if::cfg_if! {\n\
                    if #[cfg(unix)] { let _ = std::net::TcpStream::connect(\"h:80\"); }\n\
                    else { let _ = std::fs::read(\"/x\"); }\n\
                }\n\
            }\n");
        let f = fn_entry(&v, "effect_inside_cfgif");
        let mut e = effs(f);
        e.sort();
        assert_eq!(e, vec!["Fs", "Net"], "both cfg_if arms must be scanned");
        assert!(
            f.get("invisible").and_then(|i| i.as_array()).is_none_or(|a| a.is_empty()),
            "an EXPANDED cfg_if is covered, not an invisible blind: {f:#}"
        );
    }

    #[test]
    fn cfg_if_macro_bare_path_after_use() {
        // sqlx-core writes `use cfg_if::cfg_if;` then a BARE `cfg_if! { .. }` — expansion must key on the
        // resolved leaf, not the literal path, and a single-arm `if #[cfg(..)] { .. }` with NO else still
        // contributes its arm's effect.
        let v = scan_src_to_json("cfgifbare", "\
            use cfg_if::cfg_if;\n\
            pub fn only_if_no_else() {\n\
                cfg_if! {\n\
                    if #[cfg(unix)] { let _ = std::fs::read(\"/y\"); }\n\
                }\n\
            }\n");
        let f = fn_entry(&v, "only_if_no_else");
        assert_eq!(effs(f), vec!["Fs"], "the single cfg_if arm's Fs effect must surface");
    }

    #[test]
    fn cfg_if_macro_with_pure_arms_stays_pure() {
        // A cfg_if! whose every arm is effect-free stays pure — expansion cannot fabricate an effect the
        // source doesn't perform. (A pure fn is omitted from the report entirely.)
        let v = scan_src_to_json("cfgifpure", "\
            pub fn pure_cfgif() {\n\
                cfg_if::cfg_if! {\n\
                    if #[cfg(unix)] { let _ = 1 + 2; }\n\
                    else { let _ = 3 + 4; }\n\
                }\n\
            }\n");
        assert!(
            v["functions"].as_array().unwrap().iter().all(|f| f["fn"] != "pure_cfgif"),
            "a cfg_if! with only pure arms must stay pure (omitted):\n{v:#}"
        );
    }

    #[test]
    fn cfg_if_macro_does_not_swallow_surrounding_effects() {
        // Effects BEFORE and AFTER a cfg_if! are still caught (the block-walk doesn't consume the fn),
        // and a genuinely-pure fn in the same crate stays pure.
        let v = scan_src_to_json("cfgifaround", "\
            pub fn around() {\n\
                let _ = std::env::var(\"A\");\n\
                cfg_if::cfg_if! { if #[cfg(unix)] { let _ = 1; } }\n\
                let _ = std::fs::read(\"/z\");\n\
            }\n\
            pub fn genuinely_pure() { let _ = 1 + 2; }\n");
        let f = fn_entry(&v, "around");
        let mut e = effs(f);
        e.sort();
        assert_eq!(e, vec!["Env", "Fs"], "effects around a cfg_if! must remain");
        assert!(
            v["functions"].as_array().unwrap().iter().all(|f| f["fn"] != "genuinely_pure"),
            "a pure fn stays pure:\n{v:#}"
        );
    }

    #[test]
    fn cfg_if_unexpected_shape_falls_back_to_opaque() {
        // A `cfg_if!` body that ISN'T the `if #[cfg(..)] {..} [else ..]` grammar (here a bare block) must
        // NOT panic — it falls back to the opaque-macro path (recorded as an invisible qualified macro),
        // exactly the pre-fix behaviour, so a novel cfg_if extension never crashes the scan.
        let v = scan_src_to_json("cfgifbadshape", "\
            pub fn weird() {\n\
                cfg_if::cfg_if! { let _ = 1; }\n\
            }\n");
        // No panic reaching here is the core assertion; the fn is pure/absent (the opaque macro adds no
        // classified effect), consistent with any other unmodelled macro reach.
        assert!(
            v["functions"].as_array().unwrap().iter().all(|f| f["fn"] != "weird" || effs(f).is_empty()),
            "an unparseable cfg_if! is opaque, never fabricated:\n{v:#}"
        );
    }

    #[test]
    fn use_in_a_nested_block_resolves_the_call_to_its_origin() {
        // The bug (dogfound on fd `src/main.rs`): a `use` buried in a NESTED block — an inner `{ }`, an
        // `if`/`else`/`match` arm, a loop body — was NOT collected, so a call through that binding resolved
        // to nothing and the fn read SILENT-PURE. fd's `else { use std::process::{Command, Stdio};
        // Command::new("gls").status() }` reported ZERO Exec. Every nesting form must now resolve the call
        // exactly as a module-level or fn-body-top-level `use` does.
        let v = scan_src_to_json("nesteduse", "\
            pub fn f_block() { { use std::process::Command; let _ = Command::new(\"ls\").status(); } }\n\
            pub fn f_else(x: bool) { if x { let _ = 1; } else { use std::process::{Command, Stdio}; let mut c = Command::new(\"gls\"); c.stdin(Stdio::null()); let _ = c.status(); } }\n\
            pub fn f_match(x: u8) { match x { 0 => { use std::process::Command; let _ = Command::new(\"ls\").status(); }, _ => {} } }\n\
            pub fn f_loop() { for _ in 0..1 { use std::process::Command; let _ = Command::new(\"ls\").status(); } }\n");
        for name in ["f_block", "f_else", "f_match", "f_loop"] {
            assert!(
                effs(fn_entry(&v, name)).contains(&"Exec".to_string()),
                "a `use` in a nested block must resolve the call to Exec ({name} was silent-pure):\n{v:#}"
            );
        }
    }

    #[test]
    fn use_in_a_nested_block_matches_module_level_resolution() {
        // Parity: a nested-block `use std::net::TcpStream` resolves the SAME effect + host as the identical
        // module-level `use` — the origin is no longer LOST, so a std/covered call classifies exactly as it
        // would at module level (not merely "an effect appears").
        let nested = scan_src_to_json("nestednet",
            "pub fn f() { { use std::net::TcpStream; let _ = TcpStream::connect(\"10.0.0.1:80\"); } }\n");
        let module = scan_src_to_json("modnet",
            "use std::net::TcpStream;\npub fn f() { let _ = TcpStream::connect(\"10.0.0.1:80\"); }\n");
        let nf = fn_entry(&nested, "f");
        let mf = fn_entry(&module, "f");
        assert_eq!(effs(nf), effs(mf), "nested-block use must classify identically to module-level:\n{nested:#}");
        assert!(effs(nf).contains(&"Net".to_string()), "std::net → Net:\n{nested:#}");
        assert_eq!(hosts_of(nf), hosts_of(mf), "host literal must be captured for the nested use too");
    }

    #[test]
    fn use_in_a_nested_block_discloses_an_external_crate_like_module_level() {
        // An EXTERNAL-crate `use` inside a nested block discloses the crate/effect EXACTLY as the same
        // `use` at module level would (here reqwest → Net + the endpoint host). Attribution, not
        // suspicion: the binding is resolved to its declared origin crate.
        let nested = scan_src_to_json("nestedext",
            "pub fn f() { { use reqwest::get; let _ = get(\"http://api.example.com/x\"); } }\n");
        let module = scan_src_to_json("modext",
            "use reqwest::get;\npub fn f() { let _ = get(\"http://api.example.com/x\"); }\n");
        let nf = fn_entry(&nested, "f");
        assert_eq!(effs(nf), effs(fn_entry(&module, "f")),
            "external-crate use in a nested block discloses like module-level:\n{nested:#}");
        assert!(effs(nf).contains(&"Net".to_string()), "reqwest is a Net crate:\n{nested:#}");
        assert!(hosts_of(nf).contains(&"api.example.com".to_string()), "the endpoint must be captured");
    }

    #[test]
    fn use_in_a_nested_block_does_not_fabricate_on_a_pure_call() {
        // NO-FABRICATION negative: a `use` in a nested block that imports a PURE, genuinely-local name
        // must NOT invent an effect. The presence of a nested `use` never turns a pure fn effectful; only
        // a call that genuinely reaches an effect does. `helper()` is local + pure → the fn stays pure.
        let v = scan_src_to_json("nestedpure", "\
            mod util { pub fn helper() -> u32 { 42 } }\n\
            pub fn stays_pure() { { use crate::util::helper; let _ = helper(); } }\n");
        assert!(
            v["functions"].as_array().unwrap().iter()
                .all(|f| f["fn"] != "stays_pure" || effs(f).is_empty()),
            "a nested use of a pure local name must not fabricate an effect:\n{v:#}"
        );
    }

    #[test]
    fn use_in_an_inner_fn_does_not_leak_into_the_enclosing_fn() {
        // Scope guard: a `use` inside a NESTED fn item belongs to that inner fn, NOT the outer one, so the
        // whole-body use-walk must stop at a nested `fn`/`impl`/`mod` item (each is its own scan scope).
        // Here `outer`'s only real call is a pure local `noop()`; it must stay pure even though the inner
        // fn imports (and uses) std::process::Command. Without the `visit_item_fn` stop the walk would
        // collect the inner `use` and fabricate Exec on `outer` — over-attribution the guard prevents.
        let v = scan_src_to_json("innerfnuse", "\
            fn noop() {}\n\
            pub fn outer() {\n\
                fn inner() { use std::process::Command; let _ = Command::new(\"ls\").status(); }\n\
                noop();\n\
            }\n");
        assert!(
            v["functions"].as_array().unwrap().iter()
                .all(|f| f["fn"] != "outer" || effs(f).is_empty()),
            "an inner fn's use must not leak Exec onto the enclosing fn:\n{v:#}"
        );
    }

    /// ⟨0.15 staged⟩ The `coverage` envelope field (spec §2): a scan whose code calls an uncovered
    /// dependency emits the κ ledger as data — same name, same call count as the stderr disclosure
    /// line (both read the one shared `coverage_ledger`) — and the calling fn carries the
    /// per-function `invisible` attribution. A fully-covered (std-only) scan OMITS the field
    /// entirely, so its report stays byte-identical to a ⟨0.14⟩ one (the wire-compatibility rule).
    #[test]
    fn coverage_envelope_names_uncovered_deps_and_is_omitted_when_covered() {
        let run = |name: &str, deps: &str, src: &str| -> serde_json::Value {
            let d = std::env::temp_dir().join(format!("candor-cov-{name}-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&d);
            std::fs::create_dir_all(d.join("src")).unwrap();
            std::fs::write(d.join("Cargo.toml"), format!("[package]\nname = \"{name}\"\n{deps}")).unwrap();
            std::fs::write(d.join("src/lib.rs"), src).unwrap();
            let idx = DepIndex::default();
            let (rc, body) = scan_one(&d.to_string_lossy(), ScanOpts {
                prefix: String::new(), want_json: true, include_tests: false, policy: None,
                baseline: None, ws_member: false, quiet: true, deps_idx: &idx, peek_excluded: false,
            }, &crate::gate::begin_run());
            assert_eq!(rc, 0, "scan should succeed:\n{body:?}");
            let v = serde_json::from_str(&body.unwrap()).unwrap();
            let _ = std::fs::remove_dir_all(&d);
            v
        };
        // An uncovered dep: in [dependencies], demonstrably called, in no calibrated tier.
        let v = run("covdep", "[dependencies]\nsomedep = \"1\"\n", "pub fn f() { somedep::do_thing(); }\n");
        assert_eq!(
            v["coverage"]["uncovered"],
            serde_json::json!([{ "name": "somedep", "calls": 1 }]),
            "the κ ledger must travel as envelope data:\n{v:#}"
        );
        // …and the per-fn attribution (`invisible`, formalized ⟨0.15⟩ — already on the wire).
        assert_eq!(fn_entry(&v, "f")["invisible"], serde_json::json!(["somedep"]), "{v:#}");
        // Fully covered (std only) → NO `coverage` key at all.
        let v = run("covstd", "", "pub fn g() { let _ = std::fs::read(\"/x\"); }\n");
        assert!(v.get("coverage").is_none(), "a fully-covered scan must omit the field:\n{v:#}");
    }

    /// Coverage is a REVIEW claim, not a resolution outcome. κ firing on ONE call into a crate vouches
    /// for THAT call, never for the crate — so a classified call must not clear the blind marker for
    /// every other call shape into it. `pnet_datalink` is the shape that reproduces this: `channel`
    /// classifies, `interfaces` floors, and the crate sits in no calibrated tier.
    ///
    /// Before the fix, arm B's `list_ifaces` VANISHED from the report with `coverage: null` and no
    /// stderr advisory — the observed function byte-identical to arm A's, its hedge deleted by an
    /// unrelated call elsewhere in the same crate. Absence is not silence here: the ⟨0.21⟩ manifest
    /// still counts the fn in `analyzed`, so the omission reads as a positive purity claim.
    #[test]
    fn a_classified_dep_call_must_not_clear_the_hedge_on_an_unrelated_call_into_the_same_crate() {
        let run = |name: &str, src: &str| -> serde_json::Value {
            let d = std::env::temp_dir().join(format!("candor-covarm-{name}-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&d); // a stale report read back as this arm's result is the trap
            std::fs::create_dir_all(d.join("src")).unwrap();
            std::fs::write(d.join("Cargo.toml"),
                format!("[package]\nname = \"{name}\"\n[dependencies]\npnet_datalink = \"0.35\"\n")).unwrap();
            std::fs::write(d.join("src/lib.rs"), src).unwrap();
            let idx = DepIndex::default();
            let (rc, body) = scan_one(&d.to_string_lossy(), ScanOpts {
                prefix: String::new(), want_json: true, include_tests: false, policy: None,
                baseline: None, ws_member: false, quiet: true, deps_idx: &idx, peek_excluded: false,
            }, &crate::gate::begin_run());
            assert_eq!(rc, 0, "scan should succeed:\n{body:?}");
            let v = serde_json::from_str(&body.unwrap()).unwrap();
            let _ = std::fs::remove_dir_all(&d);
            v
        };
        const OBSERVED: &str = "pub fn list_ifaces() -> usize { pnet_datalink::interfaces().len() }\n";
        let a = run("covarma", OBSERVED);
        let b = run("covarmb", &format!(
            "{OBSERVED}pub fn chan(i: &pnet_datalink::NetworkInterface) \
             {{ let _ = pnet_datalink::channel(i, Default::default()); }}\n"));

        for (arm, v) in [("A", &a), ("B", &b)] {
            assert_eq!(
                fn_entry(v, "list_ifaces")["invisible"], serde_json::json!(["pnet_datalink"]),
                "arm {arm}: identical source must keep an identical hedge regardless of an unrelated \
                 classified call into the same crate:\n{v:#}"
            );
            // The tally has to mean the same thing as the crate name beside it: calls this scan could
            // not see. The classified call's effect is on the record, so counting it would overstate.
            assert_eq!(
                v["coverage"]["uncovered"], serde_json::json!([{ "name": "pnet_datalink", "calls": 1 }]),
                "arm {arm}: only the FLOORED call is counted as invisible:\n{v:#}"
            );
        }
        // …and the classified call keeps its own effect: this discloses more, it does not blur what was
        // already resolved.
        assert!(
            !effs(fn_entry(&b, "chan")).is_empty(),
            "the classified call's own fn keeps its effect:\n{b:#}"
        );

    }

    /// R59 (SOUNDNESS.md): `libc`/`nix`/`rustix` are `CALIBRATED_CRATES`, and the coverage ledger
    /// normally exempts a calibrated crate outright — "classify has rules here" reads as "an unmatched
    /// call was reviewed and found pure". That does not hold for these three: `classify` DELIBERATELY
    /// skips their generic fd verbs (`read`/`write`/`close`/...) because a fixed label would
    /// mis-categorise an ambiguous fd as often as it helps (an honest no-classify, documented in
    /// `candor-classify/src/lib.rs`). Before the fix the blanket exemption converted that documented gap
    /// into total silence: a fn whose ENTIRE effectful surface was `libc::read` on a bare fd param
    /// vanished from the report completely — `"functions": []`, no `Unknown`, no `invisible`, nothing
    /// (worse than an uncalibrated dependency, which discloses `invisible` on the same shape).
    /// CONTROLS: (a) a CLASSIFIED libc call (`open` → Fs) stays exactly as precise as before, with no
    /// `invisible` noise; (b) a function mixing a classified call with an unclassified one keeps BOTH the
    /// real effect and the disclosure, never one masking the other.
    #[test]
    fn libc_generic_fd_verb_discloses_invisible_instead_of_vanishing() {
        let run = |name: &str, src: &str| -> serde_json::Value {
            let d = std::env::temp_dir().join(format!("candor-libcfd-{name}-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&d);
            std::fs::create_dir_all(d.join("src")).unwrap();
            std::fs::write(d.join("Cargo.toml"),
                format!("[package]\nname = \"{name}\"\n[dependencies]\nlibc = \"0.2\"\n")).unwrap();
            std::fs::write(d.join("src/lib.rs"), src).unwrap();
            let idx = DepIndex::default();
            let (rc, body) = scan_one(&d.to_string_lossy(), ScanOpts {
                prefix: String::new(), want_json: true, include_tests: false, policy: None,
                baseline: None, ws_member: false, quiet: true, deps_idx: &idx, peek_excluded: false,
            }, &crate::gate::begin_run());
            assert_eq!(rc, 0, "scan should succeed:\n{body:?}");
            let v = serde_json::from_str(&body.unwrap()).unwrap();
            let _ = std::fs::remove_dir_all(&d);
            v
        };
        // THE FIX: an unclassified generic fd verb (`read`) discloses `invisible`, not silence.
        let bare = run("libcfdbare",
            "pub fn drain(fd: i32) -> usize { let mut b = [0u8; 64]; \
             unsafe { libc::read(fd, b.as_mut_ptr() as *mut libc::c_void, 64) as usize } }\n");
        assert!(
            !bare["functions"].as_array().unwrap().is_empty(),
            "before the fix this vanished entirely (\"functions\": []) — total silence, worse than an \
             uncalibrated dep:\n{bare:#}"
        );
        assert_eq!(fn_entry(&bare, "drain")["invisible"], serde_json::json!(["libc"]),
            "an unclassified libc fd verb must disclose the crate as invisible, not silently pure:\n{bare:#}");
        assert!(effs(fn_entry(&bare, "drain")).is_empty(),
            "NO FABRICATION: candor cannot know if the fd is a file/socket/pipe, so it must never guess \
             a concrete effect (Fs/Net) here:\n{bare:#}");
        assert_eq!(bare["coverage"]["uncovered"], serde_json::json!([{ "name": "libc", "calls": 1 }]),
            "the coverage ledger must count the unclassified call:\n{bare:#}");
        // CONTROL (a): a CLASSIFIED libc call keeps its precise effect, with NO invisible noise —
        // libc/nix/rustix stay fully trusted for the calls the syscall table DOES cover.
        let classified = run("libcfdclassified",
            "pub fn open_it() -> i32 { unsafe { libc::open(std::ptr::null(), 0) } }\n");
        assert_eq!(effs(fn_entry(&classified, "open_it")), vec!["Fs"],
            "a classified libc call (open) must keep its precise effect:\n{classified:#}");
        assert!(fn_entry(&classified, "open_it").get("invisible").is_none(),
            "a classified libc call must NOT be flagged invisible — no spurious noise:\n{classified:#}");
        assert!(classified.get("coverage").is_none(),
            "a fully-classified scan must omit the coverage field:\n{classified:#}");
        // CONTROL (b): mixing a classified call with an unclassified one keeps BOTH — the real effect is
        // not masked by the disclosure, and the disclosure is not masked by the real effect.
        let mixed = run("libcfdmixed",
            "pub fn read_it(fd: i32) -> i32 { \
                 unsafe { let f = libc::open(std::ptr::null(), 0); \
                 let mut b = [0u8; 8]; libc::read(f, b.as_mut_ptr() as *mut libc::c_void, 8); f } }\n");
        assert_eq!(effs(fn_entry(&mixed, "read_it")), vec!["Fs"],
            "the classified open() effect must survive alongside the disclosure:\n{mixed:#}");
        assert_eq!(fn_entry(&mixed, "read_it")["invisible"], serde_json::json!(["libc"]),
            "the unclassified read() must still be disclosed even with a classified sibling call:\n{mixed:#}");
    }

    /// BACKLOG "rust-deep's crate-name-keyed `invisible` mechanism, everywhere else" — the COLLISION
    /// direction: "a workspace crate sharing a name with a `CALIBRATED_CRATES` member inherits [the]
    /// exemption for its own unclassified calls". `CALIBRATED_CRATES`/`PATH_CALIBRATED_CRATES`/
    /// `CALIBRATED_PREFIXES` are STRING matches against `cr` (the call's syntactic first path segment) —
    /// they carry no check that the crate wearing that name is the actual, reviewed, published artifact
    /// `classify()`'s rules were written against. A `path` dependency can be named anything, including
    /// one of the 82 `CALIBRATED_CRATES` entries (`log` here — a plausible accidental collision, not an
    /// exotic one: an internal logging shim/vendored fork keeping the upstream name is an ordinary
    /// shape). Before the fix this reproduced EXACTLY like R59/R60: `"functions": []`, total silence —
    /// worse than an uncalibrated dependency, which discloses `invisible` on the identical call shape
    /// (proved live against the pre-fix binary: standalone `candor-scan` on a `victim` crate path-
    /// depending on a crate literally named `log` that performs `std::net::TcpStream::connect` under an
    /// unmodelled tail name printed `"functions": []`; the same fixture with the dependency renamed to
    /// `logimpostor` — the ONLY variable changed — correctly disclosed `invisible: ["logimpostor"]`,
    /// isolating the cause to the name collision alone, not the call shape).
    ///
    /// THE FIX, TWO INDEPENDENT SOURCES OF POSITIVE EVIDENCE, unioned: `non_registry_lock_names` reads
    /// `dir`'s Cargo.lock and strips all three CALIBRATED_* exemptions for any name it CONFIRMS is not
    /// registry-sourced. `non_registry_manifest_names` reads Cargo.toml itself — always present, unlike
    /// Cargo.lock, which a library tree routinely does not commit — for a `path =`/`git =` source on the
    /// dependency declaration. Both are DENYLIST narrowings: the exemption is unchanged unless one of them
    /// returns POSITIVE evidence of an impostor.
    ///
    /// ⟨2026-08-28 ADVERSARIAL REVIEW⟩ found the lock-only fix's own stated residual was UNDER-STATED: it
    /// called "no Cargo.lock present" a costless fallback, but a Rust *library* repo — candor-scan's own
    /// stated purpose is scanning source WITHOUT building it — routinely has no lockfile at all, so the
    /// trigger population WAS the target population. The manifest check closes it for the reproduced shape
    /// (a `path`/`git` dependency, which is how a name-squatting impostor is actually attached — you
    /// cannot get crates.io to publish a second `log`) without a lockfile and without a new wire key.
    ///
    /// CONTROLS, isolating exactly one variable each: (a) the SAME name as a genuine registry dependency
    /// (Cargo.toml gives a bare version, Cargo.lock says `registry+…`) keeps the exemption; (b) a `path`
    /// dependency with NO Cargo.lock at all now DISCLOSES via the manifest check alone (the reproduced
    /// defect, closed); (c) a `git` dependency with no Cargo.lock, same disclosure, different source key;
    /// (d) the GENUINE residual, sharpest over-charge guard in this file: a bare-version dependency (no
    /// path, no git — indistinguishable from a real registry crate by anything in the tree) with NO
    /// Cargo.lock keeps the exemption — an ordinary honest project must not gain noise just because it has
    /// no lockfile. THIS CASE IS NOT CLOSED: neither check has anything to go on, and closing it needs
    /// either a lockfile or evidence outside the scanned tree (`[patch]`, `.cargo/config.toml` source
    /// replacement) — stated here, not silently assumed.
    #[test]
    fn crate_name_collision_with_a_calibrated_crate_loses_the_ledger_exemption() {
        let build = |name: &str, manifest_dep: &str, lock_source: Option<&str>| -> serde_json::Value {
            let d = std::env::temp_dir().join(format!(
                "candor-collide-{name}-{}-{}", manifest_dep.len(), std::process::id()
            ));
            let _ = std::fs::remove_dir_all(&d);
            std::fs::create_dir_all(d.join("src")).unwrap();
            std::fs::write(d.join("Cargo.toml"), format!(
                "[package]\nname = \"victim\"\nversion = \"0.1.0\"\n[dependencies]\n{name} = {manifest_dep}\n"
            )).unwrap();
            if let Some(source_line) = lock_source {
                std::fs::write(d.join("Cargo.lock"), format!(
                    "version = 3\n\n[[package]]\nname = \"victim\"\nversion = \"0.1.0\"\ndependencies = [\n \"{name}\",\n]\n\n[[package]]\nname = \"{name}\"\nversion = \"0.1.0\"\n{source_line}\n"
                )).unwrap();
            }
            std::fs::write(d.join("src/lib.rs"), format!(
                "pub fn exfiltrate() {{ {name}::totally_unmodelled_tail(\"http://evil.example\"); }}\n"
            )).unwrap();
            let idx = DepIndex::default();
            let (rc, body) = scan_one(&d.to_string_lossy(), ScanOpts {
                prefix: String::new(), want_json: true, include_tests: false, policy: None,
                baseline: None, ws_member: false, quiet: true, deps_idx: &idx, peek_excluded: false,
            }, &crate::gate::begin_run());
            assert_eq!(rc, 0, "scan should succeed:\n{body:?}");
            let v = serde_json::from_str(&body.unwrap()).unwrap();
            let _ = std::fs::remove_dir_all(&d);
            v
        };
        let assert_disclosed = |v: &serde_json::Value, crate_name: &str, label: &str| {
            assert!(
                !v["functions"].as_array().unwrap().is_empty(),
                "{label}: an impostor `{crate_name}` must not vanish entirely (\"functions\": []) — the \
                 R59/R60 silent drop, via a name collision instead of an FFI seam:\n{v:#}"
            );
            assert_eq!(fn_entry(v, "exfiltrate")["invisible"], serde_json::json!([crate_name]),
                "{label}: an unclassified call into a NON-registry `{crate_name}` must disclose the crate \
                 as invisible:\n{v:#}");
            assert!(effs(fn_entry(v, "exfiltrate")).is_empty(),
                "{label}: NO FABRICATION — the impostor's real effect (Net) is unknown to classify() by \
                 construction here; it must never be guessed:\n{v:#}");
            assert_eq!(v["coverage"]["uncovered"], serde_json::json!([{ "name": crate_name, "calls": 1 }]),
                "{label}: the coverage ledger must count the unclassified call into the impostor:\n{v:#}");
        };
        // THE DEFECT CASE, WITH a lockfile confirming the impostor (unchanged from the original fix):
        // must disclose. Regression control for the `caca530` behavior this builds on.
        let with_lock = build("log", "{ path = \"../logfake\" }", Some(""));
        assert_disclosed(&with_lock, "log", "with-lockfile impostor");
        // CONTROL (a) — OVER-CHARGE GUARD: a genuine registry dependency (bare version in Cargo.toml, and
        // Cargo.lock confirming `registry+…`) must keep the exemption exactly as before.
        let registry = build("log", "\"0.4\"", Some(r#"source = "registry+https://github.com/rust-lang/crates.io-index""#));
        assert!(
            registry["functions"].as_array().unwrap().is_empty(),
            "a registry-sourced `log` must keep the CALIBRATED_CRATES exemption unchanged — this fix \
             must never over-charge the ordinary case:\n{registry:#}"
        );
        // CONTROL (b) — THE REPRODUCED DEFECT, closed: a `path` dependency named `log`, NO Cargo.lock at
        // all. Before this change this fell back to the pre-`caca530` silent exemption (`functions: []`,
        // asserted red against that binary below); the manifest check now disclose it with no lockfile.
        let no_lock_path = build("log", "{ path = \"../logfake\" }", None);
        assert_disclosed(&no_lock_path, "log", "no-lockfile path impostor (the reproduced defect)");
        // CONTROL (c) — same, `git` source instead of `path`: proves the manifest check isn't path-only.
        let no_lock_git = build("log", "{ git = \"https://example.invalid/logfake.git\" }", None);
        assert_disclosed(&no_lock_git, "log", "no-lockfile git impostor");
        // CONTROL (d) — THE GENUINE RESIDUAL AND THE SHARPEST OVER-CHARGE GUARD: a bare-version dependency
        // (indistinguishable from a real registry crate by anything the manifest or a missing lockfile can
        // say) must NOT gain noise just because there is no Cargo.lock. If this assertion ever fails
        // because someone widened the check to fire on every unlocked calibrated dependency, that is a
        // worse defect than the one this file fixes: it is the ordinary, honest, lockfile-less Rust
        // library — precisely the population the brief that produced this fix named as the trigger
        // population — screaming on every scan.
        let no_lock_honest = build("log", "\"0.4\"", None);
        assert!(
            no_lock_honest["functions"].as_array().unwrap().is_empty(),
            "STATED RESIDUAL, NOT SILENT: an honest bare-version dependency with no Cargo.lock cannot be \
             distinguished from an impostor by anything in the tree, so it keeps the exemption. This must \
             stay true — the alternative (disclosing on every unlocked calibrated dependency) is the \
             over-charge the brief forbids:\n{no_lock_honest:#}"
        );
    }

    /// The reproduced defect's ORIGINAL shape, red on the pre-this-change binary, kept as its own test so
    /// a future refactor of `crate_name_collision_with_a_calibrated_crate_loses_the_ledger_exemption`
    /// cannot accidentally drop the no-lockfile case: `non_registry_manifest_names` must find `path =`
    /// evidence in the HEADER-TABLE manifest form (`[dependencies.name]` / `path = "…"` on its own line),
    /// not only the inline-table form (`name = { path = "…" }`) the main test above uses — the two are
    /// different code paths in `non_registry_manifest_deps` and a fix that only handled one would still
    /// leave half of real-world Cargo.toml a silent gap.
    #[test]
    fn crate_name_collision_disclosed_via_header_table_manifest_form_with_no_lockfile() {
        let d = std::env::temp_dir().join(format!("candor-collide-headertable-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(d.join("src")).unwrap();
        std::fs::write(d.join("Cargo.toml"),
            "[package]\nname = \"victim\"\nversion = \"0.1.0\"\n\n[dependencies.log]\npath = \"../logfake\"\n"
        ).unwrap();
        std::fs::write(d.join("src/lib.rs"),
            "pub fn exfiltrate() { log::totally_unmodelled_tail(\"http://evil.example\"); }\n"
        ).unwrap();
        let idx = DepIndex::default();
        let (rc, body) = scan_one(&d.to_string_lossy(), ScanOpts {
            prefix: String::new(), want_json: true, include_tests: false, policy: None,
            baseline: None, ws_member: false, quiet: true, deps_idx: &idx, peek_excluded: false,
        }, &crate::gate::begin_run());
        assert_eq!(rc, 0, "scan should succeed:\n{body:?}");
        let v: serde_json::Value = serde_json::from_str(&body.unwrap()).unwrap();
        let _ = std::fs::remove_dir_all(&d);
        assert!(!v["functions"].as_array().unwrap().is_empty(),
            "a header-table `[dependencies.log]` path dependency, no lockfile, must still disclose — the \
             inline-table and header-table manifest forms must not diverge:\n{v:#}");
        assert_eq!(fn_entry(&v, "exfiltrate")["invisible"], serde_json::json!(["log"]),
            "header-table form: the impostor must be disclosed invisible, same as the inline-table form:\n{v:#}");
    }

    /// ⟨2026-08-29 ADVERSARIAL REVIEW⟩ found the `non_registry_manifest_names`/`non_registry_lock_names`
    /// pair above still missed TWO real spellings — reproduced live against the pre-this-fix binary
    /// before either was touched:
    ///
    /// (1) WORKSPACE INHERITANCE. A member declares `log = { workspace = true }`; the REAL
    /// `log = { path = "../evil-log" }` lives in the WORKSPACE ROOT's `[workspace.dependencies]`.
    /// `non_registry_manifest_names` walked the scanned member's own directory only and never the
    /// workspace root, so a real Net-performing impostor named `log` kept the `CALIBRATED_CRATES`
    /// exemption silently — no Cargo.lock involved at all. Isolation control: the identical shape under
    /// a non-calibrated name (`logimpostor`) correctly disclosed `invisible` on the pre-fix binary — only
    /// the name was the variable.
    ///
    /// (2) DOTTED-KEY TOML, and the mechanism is NOT the one (1) shares — this is the sharper finding.
    /// `log.path = "../evil-log"` inside a flat `[dependencies]` table is valid TOML (`cargo metadata`
    /// resolves it as a path dependency); the line-based `cargo_toml_deps` never modelled this PRODUCTION
    /// at all, so it parsed the entire token `log.path` as one dependency NAME — `deps.contains("log")`
    /// was FALSE, and the call skipped the κ ledger before the `CALIBRATED_CRATES` check ever ran.
    /// Reproduced silent on BOTH a calibrated name (`log`) and a non-calibrated one (`logimpostor`) on the
    /// pre-fix binary — a calibrated-exemption bypass could never do that, since an uncalibrated name has
    /// no exemption to bypass. Fixing only `non_registry_manifest_names` would have left this reproduced
    /// defect completely unaffected.
    ///
    /// THE FIX: both `cargo_toml_deps` and `non_registry_manifest_names` now parse Cargo.toml with a REAL
    /// TOML parser (the `toml` crate) instead of enumerating surface spellings — inline tables,
    /// header-table sections and dotted keys all parse to the SAME structure, so one check
    /// (`Value::as_table` + `contains_key`) replaces an ever-growing branch list. `find_workspace_root`
    /// walks up to the nearest `[workspace]`-declaring ancestor to resolve a `workspace = true` entry
    /// against the root's `[workspace.dependencies]` table under the same key, FAILING TOWARD DISCLOSURE
    /// (never toward trust) when that resolution cannot be completed.
    #[test]
    fn workspace_inheritance_and_dotted_key_impostors_lose_the_ledger_exemption() {
        let base = |tag: &str| std::env::temp_dir().join(format!("candor-collide2-{tag}-{}", std::process::id()));
        let scan = |dir: &std::path::Path| -> serde_json::Value {
            let idx = DepIndex::default();
            let (rc, body) = scan_one(&dir.to_string_lossy(), ScanOpts {
                prefix: String::new(), want_json: true, include_tests: false, policy: None,
                baseline: None, ws_member: false, quiet: true, deps_idx: &idx, peek_excluded: false,
            }, &crate::gate::begin_run());
            assert_eq!(rc, 0, "scan should succeed:\n{body:?}");
            serde_json::from_str(&body.unwrap()).unwrap()
        };
        let assert_disclosed = |v: &serde_json::Value, crate_name: &str, label: &str| {
            assert!(!v["functions"].as_array().unwrap().is_empty(),
                "{label}: an impostor `{crate_name}` must not vanish entirely (\"functions\": []):\n{v:#}");
            assert_eq!(fn_entry(v, "exfiltrate")["invisible"], serde_json::json!([crate_name]),
                "{label}: an unclassified call into a NON-registry `{crate_name}` must disclose the crate \
                 as invisible:\n{v:#}");
            assert!(effs(fn_entry(v, "exfiltrate")).is_empty(),
                "{label}: NO FABRICATION — the impostor's real effect (Net) must never be guessed:\n{v:#}");
            assert_eq!(v["coverage"]["uncovered"], serde_json::json!([{ "name": crate_name, "calls": 1 }]),
                "{label}: the coverage ledger must count the unclassified call into the impostor:\n{v:#}");
        };

        // ── (1) WORKSPACE INHERITANCE ───────────────────────────────────────────────────────────────
        let ws_fixture = |tag: &str, dep_name: &str, root_source: &str| -> std::path::PathBuf {
            let root = base(tag);
            let _ = std::fs::remove_dir_all(&root);
            std::fs::create_dir_all(root.join("member/src")).unwrap();
            std::fs::write(root.join("Cargo.toml"), format!(
                "[workspace]\nmembers = [\"member\"]\n\n[workspace.dependencies]\n{dep_name} = {root_source}\n"
            )).unwrap();
            std::fs::write(root.join("member/Cargo.toml"), format!(
                "[package]\nname = \"victim\"\nversion = \"0.1.0\"\n[dependencies]\n{dep_name} = {{ workspace = true }}\n"
            )).unwrap();
            std::fs::write(root.join("member/src/lib.rs"), format!(
                "pub fn exfiltrate() {{ {dep_name}::totally_unmodelled_tail(\"http://evil.example\"); }}\n"
            )).unwrap();
            root
        };
        // THE DEFECT: workspace root declares the REAL source (`path`), member only says `workspace = true`.
        let ws_impostor = ws_fixture("ws-impostor", "log", "{ path = \"../evil-log\" }");
        let v = scan(&ws_impostor.join("member"));
        assert_disclosed(&v, "log", "workspace-inheritance impostor (calibrated name)");
        let _ = std::fs::remove_dir_all(&ws_impostor);
        // ISOLATION CONTROL: identical shape, non-calibrated name — must ALREADY have disclosed (proves
        // the defect is specific to the calibrated exemption, not the workspace-inheritance shape itself).
        let ws_control = ws_fixture("ws-control", "logimpostor", "{ path = \"../evil-log\" }");
        let v = scan(&ws_control.join("member"));
        assert_disclosed(&v, "logimpostor", "workspace-inheritance isolation control (non-calibrated name)");
        let _ = std::fs::remove_dir_all(&ws_control);
        // OVER-CHARGE GUARD: the workspace root's entry is a genuine bare version — ordinary, honest
        // workspace inheritance must stay EXACTLY as before, no new noise.
        let ws_honest = ws_fixture("ws-honest", "log", "\"0.4\"");
        let v = scan(&ws_honest.join("member"));
        assert!(v["functions"].as_array().unwrap().is_empty(),
            "an honest workspace-inherited registry dependency must keep the exemption unchanged — the \
             over-charge this fix must never commit:\n{v:#}");
        let _ = std::fs::remove_dir_all(&ws_honest);

        // ── FAIL-TOWARD-DISCLOSURE: `workspace = true` with NO discoverable workspace root at all ─────
        let orphan = base("ws-orphan");
        let _ = std::fs::remove_dir_all(&orphan);
        std::fs::create_dir_all(orphan.join("src")).unwrap();
        std::fs::write(orphan.join("Cargo.toml"),
            "[package]\nname = \"victim\"\nversion = \"0.1.0\"\n[dependencies]\nlog = { workspace = true }\n"
        ).unwrap();
        std::fs::write(orphan.join("src/lib.rs"),
            "pub fn exfiltrate() { log::totally_unmodelled_tail(\"http://evil.example\"); }\n"
        ).unwrap();
        let v = scan(&orphan);
        assert_disclosed(&v, "log", "unresolvable workspace=true redirect (no root found)");
        let _ = std::fs::remove_dir_all(&orphan);

        // ── (2) DOTTED-KEY TOML ──────────────────────────────────────────────────────────────────────
        let dotted_fixture = |tag: &str, dep_name: &str| -> std::path::PathBuf {
            let d = base(tag);
            let _ = std::fs::remove_dir_all(&d);
            std::fs::create_dir_all(d.join("src")).unwrap();
            std::fs::write(d.join("Cargo.toml"), format!(
                "[package]\nname = \"victim\"\nversion = \"0.1.0\"\n[dependencies]\n{dep_name}.path = \"../evil-log\"\n"
            )).unwrap();
            std::fs::write(d.join("src/lib.rs"), format!(
                "pub fn exfiltrate() {{ {dep_name}::totally_unmodelled_tail(\"http://evil.example\"); }}\n"
            )).unwrap();
            d
        };
        // THE DEFECT, calibrated name: pre-fix, `cargo_toml_deps` itself never recognised `log.path = …`
        // as declaring `log` — silent regardless of CALIBRATED_CRATES.
        let dotted_impostor = dotted_fixture("dotted-impostor", "log");
        let v = scan(&dotted_impostor);
        assert_disclosed(&v, "log", "dotted-key impostor (calibrated name)");
        let _ = std::fs::remove_dir_all(&dotted_impostor);
        // ISOLATION CONTROL PROVING THE MECHANISM: a NON-calibrated name in the SAME dotted-key shape was
        // ALSO silent pre-fix — a calibrated-exemption bug could never do this, since there is no
        // exemption for an uncalibrated crate to bypass. This is what shows the defect lived one level
        // below `non_registry_manifest_names`, in the base dependency-name parser (`cargo_toml_deps`).
        let dotted_control = dotted_fixture("dotted-control", "logimpostor");
        let v = scan(&dotted_control);
        assert_disclosed(&v, "logimpostor", "dotted-key impostor (non-calibrated name — proves the deeper cause)");
        let _ = std::fs::remove_dir_all(&dotted_control);
        // OVER-CHARGE GUARD: a dotted-key BARE VERSION (`log.version = "0.4"`) is ordinary, honest TOML —
        // must keep the exemption exactly as before.
        let dotted_honest = base("dotted-honest");
        let _ = std::fs::remove_dir_all(&dotted_honest);
        std::fs::create_dir_all(dotted_honest.join("src")).unwrap();
        std::fs::write(dotted_honest.join("Cargo.toml"),
            "[package]\nname = \"victim\"\nversion = \"0.1.0\"\n[dependencies]\nlog.version = \"0.4\"\n"
        ).unwrap();
        std::fs::write(dotted_honest.join("src/lib.rs"),
            "pub fn exfiltrate() { log::totally_unmodelled_tail(\"http://evil.example\"); }\n"
        ).unwrap();
        let v = scan(&dotted_honest);
        assert!(v["functions"].as_array().unwrap().is_empty(),
            "a dotted-key BARE VERSION dependency is ordinary honest TOML — must keep the exemption \
             unchanged, never new over-charge noise:\n{v:#}");
        let _ = std::fs::remove_dir_all(&dotted_honest);
    }

    /// ⟨2026-08-29 ADVERSARIAL REVIEW, finding 1⟩ `find_workspace_root` used to return the nearest
    /// `[workspace]`-declaring ANCESTOR unconditionally, on the argument (now corrected — see the doc on
    /// `find_workspace_root`) that a directory sitting under an unrelated real workspace by mere
    /// filesystem POSITION "cannot manufacture a false exemption" because cargo would refuse to build the
    /// layout. Reproduced live: a `vendor/fake-lib` directory that is NOT a declared member of the outer
    /// workspace (not in `members`, would fail `cargo metadata` if built there) declares
    /// `log = { workspace = true }` and performs a real, unmodelled effectful call; the outer root
    /// happens to ALSO declare an unrelated, genuine `log = "0.4"` — ancestry, not membership, let the
    /// walk resolve against it and grant the `CALIBRATED_CRATES` exemption to an impostor the outer
    /// workspace has nothing to do with.
    #[test]
    fn workspace_ancestor_name_coincidence_without_membership_still_discloses() {
        let base = |tag: &str| std::env::temp_dir().join(format!("candor-collide3-{tag}-{}", std::process::id()));
        let scan = |dir: &std::path::Path| -> serde_json::Value {
            let idx = DepIndex::default();
            let (rc, body) = scan_one(&dir.to_string_lossy(), ScanOpts {
                prefix: String::new(), want_json: true, include_tests: false, policy: None,
                baseline: None, ws_member: false, quiet: true, deps_idx: &idx, peek_excluded: false,
            }, &crate::gate::begin_run());
            assert_eq!(rc, 0, "scan should succeed:\n{body:?}");
            serde_json::from_str(&body.unwrap()).unwrap()
        };

        // THE OUTER, REAL workspace: `real-member` is its only declared member. Its `[workspace.dependencies]`
        // carries a genuine, unrelated, bare-version `log` — an ordinary honest entry that has nothing to
        // do with `vendor/fake-lib` below.
        let root = base("outer");
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("real-member/src")).unwrap();
        std::fs::create_dir_all(root.join("vendor/fake-lib/src")).unwrap();
        std::fs::write(root.join("Cargo.toml"),
            "[workspace]\nmembers = [\"real-member\"]\n\n[workspace.dependencies]\nlog = \"0.4\"\n"
        ).unwrap();
        std::fs::write(root.join("real-member/Cargo.toml"), "[package]\nname = \"real-member\"\nversion = \"0.1.0\"\n").unwrap();
        std::fs::write(root.join("real-member/src/lib.rs"), "pub fn noop() {}\n").unwrap();
        // `vendor/fake-lib`: NOT listed in the outer workspace's `members`, so `cargo metadata` run there
        // would refuse it — but candor-scan reads source without building, and this fix must not let mere
        // ancestry substitute for membership.
        std::fs::write(root.join("vendor/fake-lib/Cargo.toml"),
            "[package]\nname = \"fake-lib\"\nversion = \"0.1.0\"\n[dependencies]\nlog = { workspace = true }\n"
        ).unwrap();
        std::fs::write(root.join("vendor/fake-lib/src/lib.rs"),
            "pub fn exfiltrate() { log::totally_unmodelled_tail(\"http://evil.example\"); }\n"
        ).unwrap();

        let v = scan(&root.join("vendor/fake-lib"));
        assert!(!v["functions"].as_array().unwrap().is_empty(),
            "a non-member subtree resolving `workspace = true` against an unrelated ancestor's coincidentally \
             same-named dependency must not vanish entirely (\"functions\": []):\n{v:#}");
        assert_eq!(fn_entry(&v, "exfiltrate")["invisible"], serde_json::json!(["log"]),
            "must disclose `log` invisible rather than silently granting the ancestor's exemption:\n{v:#}");
        assert!(effs(fn_entry(&v, "exfiltrate")).is_empty(),
            "NO FABRICATION — the impostor's real effect must never be guessed:\n{v:#}");

        // OVER-CHARGE GUARD: a GENUINE member (`real-member`) inheriting a GENUINE, honest
        // `workspace = true` registry dependency from the SAME outer root must stay byte-identical — this
        // fix must not cost an honest workspace member anything.
        std::fs::write(root.join("real-member/Cargo.toml"),
            "[package]\nname = \"real-member\"\nversion = \"0.1.0\"\n[dependencies]\nlog = { workspace = true }\n"
        ).unwrap();
        std::fs::write(root.join("real-member/src/lib.rs"),
            "pub fn exfiltrate() { log::totally_unmodelled_tail(\"http://evil.example\"); }\n"
        ).unwrap();
        let v = scan(&root.join("real-member"));
        assert!(v["functions"].as_array().unwrap().is_empty(),
            "a GENUINE member's honest workspace-inherited registry dependency must keep the exemption \
             unchanged — the over-charge this fix must never commit:\n{v:#}");
        let _ = std::fs::remove_dir_all(&root);
    }

    /// GUARD-DELETION AUDIT (bin/AGENT-CORPUS-BRIEF.md attack C), 2026-08-30: every test above this one
    /// proves the impostor carve-out for exactly ONE of the four calibrated-style exemptions —
    /// `CALIBRATED_CRATES` (via the `log` fixtures). scan.rs's coverage-ledger filter applies the
    /// identical `&& !impostor` carve-out to THREE MORE, independent list checks:
    ///
    ///     !(PATH_CALIBRATED_CRATES.contains(cr) && !impostor)
    ///     !(CALIBRATED_PREFIXES.iter().any(|p| cr.starts_with(p)) && !impostor)
    ///     !(REVIEWED_PURE_CRATES.contains(cr) && !impostor)
    ///
    /// Each is a SEPARATE boolean conjunct, so a suite that only ever drives an impostor through
    /// `CALIBRATED_CRATES` cannot tell any of these three apart from a version with `&& !impostor`
    /// deleted. Confirmed live: deleting all three `!impostor` conjuncts (leaving the bare list checks)
    /// left `cargo test --workspace` fully green — 0 failures across every crate — before this test
    /// existed. Each arm below reproduces the `log`-impostor shape one level over, for one crate drawn
    /// from each list, plus its own over-charge control so the fix can't be "widen to always disclose".
    #[test]
    fn path_calibrated_prefix_and_reviewed_pure_impostors_lose_the_ledger_exemption() {
        let scan = |dir: &std::path::Path| -> serde_json::Value {
            let idx = DepIndex::default();
            let (rc, body) = scan_one(&dir.to_string_lossy(), ScanOpts {
                prefix: String::new(), want_json: true, include_tests: false, policy: None,
                baseline: None, ws_member: false, quiet: true, deps_idx: &idx, peek_excluded: false,
            }, &crate::gate::begin_run());
            assert_eq!(rc, 0, "scan should succeed:\n{body:?}");
            serde_json::from_str(&body.unwrap()).unwrap()
        };
        let assert_disclosed = |v: &serde_json::Value, crate_name: &str, label: &str| {
            assert!(!v["functions"].as_array().unwrap().is_empty(),
                "{label}: an impostor `{crate_name}` must not vanish entirely (\"functions\": []):\n{v:#}");
            assert_eq!(fn_entry(v, "exfiltrate")["invisible"], serde_json::json!([crate_name]),
                "{label}: an unclassified call into a NON-registry `{crate_name}` must disclose the crate \
                 as invisible:\n{v:#}");
            assert!(effs(fn_entry(v, "exfiltrate")).is_empty(),
                "{label}: NO FABRICATION — the impostor's real effect must never be guessed:\n{v:#}");
        };
        let assert_exempt = |v: &serde_json::Value, label: &str| {
            assert!(v["functions"].as_array().unwrap().is_empty(),
                "{label}: a genuine registry-sourced dependency must keep the exemption unchanged — the \
                 over-charge this test must never accept:\n{v:#}");
        };
        // One fixture builder shared by all three arms: `crate_name` is a `path` dependency with NO
        // Cargo.lock (the reproduced-shape, no-lockfile impostor from `non_registry_manifest_names`) for
        // the defect case, or a bare version for the honest control.
        let build = |tag: &str, crate_name: &str, manifest_dep: &str| -> std::path::PathBuf {
            let d = std::env::temp_dir().join(format!("candor-collide4-{tag}-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&d);
            std::fs::create_dir_all(d.join("src")).unwrap();
            std::fs::write(d.join("Cargo.toml"), format!(
                "[package]\nname = \"victim\"\nversion = \"0.1.0\"\n[dependencies]\n{crate_name} = {manifest_dep}\n"
            )).unwrap();
            std::fs::write(d.join("src/lib.rs"), format!(
                "pub fn exfiltrate() {{ {crate_name}::totally_unmodelled_tail(\"http://evil.example\"); }}\n"
            )).unwrap();
            d
        };

        // ── PATH_CALIBRATED_CRATES: `tokio` ─────────────────────────────────────────────────────────
        let d = build("tokio-impostor", "tokio", "{ path = \"../evil-tokio\" }");
        assert_disclosed(&scan(&d), "tokio", "PATH_CALIBRATED_CRATES impostor (tokio, path dep)");
        let _ = std::fs::remove_dir_all(&d);
        let d = build("tokio-honest", "tokio", "\"1\"");
        assert_exempt(&scan(&d), "PATH_CALIBRATED_CRATES honest control (tokio, bare version)");
        let _ = std::fs::remove_dir_all(&d);

        // ── CALIBRATED_PREFIXES: a name starting `aws_sdk_` ─────────────────────────────────────────
        let d = build("awssdk-impostor", "aws_sdk_evilthing", "{ path = \"../evil-aws\" }");
        assert_disclosed(&scan(&d), "aws_sdk_evilthing", "CALIBRATED_PREFIXES impostor (aws_sdk_*, path dep)");
        let _ = std::fs::remove_dir_all(&d);
        let d = build("awssdk-honest", "aws_sdk_evilthing", "\"1\"");
        assert_exempt(&scan(&d), "CALIBRATED_PREFIXES honest control (aws_sdk_*, bare version)");
        let _ = std::fs::remove_dir_all(&d);

        // ── REVIEWED_PURE_CRATES: `toml` ────────────────────────────────────────────────────────────
        let d = build("toml-impostor", "toml", "{ path = \"../evil-toml\" }");
        assert_disclosed(&scan(&d), "toml", "REVIEWED_PURE_CRATES impostor (toml, path dep)");
        let _ = std::fs::remove_dir_all(&d);
        let d = build("toml-honest", "toml", "\"0.8\"");
        assert_exempt(&scan(&d), "REVIEWED_PURE_CRATES honest control (toml, bare version)");
        let _ = std::fs::remove_dir_all(&d);
    }

    /// GUARD-DELETION AUDIT, 2026-08-30: `verified_workspace_root` has TWO ways for `dir` to be entitled
    /// to a workspace root's `[workspace.dependencies]` table — `dir` IS `root` (a non-virtual manifest,
    /// declaring BOTH `[package]` and `[workspace]`, resolving its own `{ workspace = true }` dependency
    /// against its own table — no membership question to ask), or `dir` is one of `root`'s resolved
    /// MEMBERS. Every existing fixture for this function drives the MEMBER arm only
    /// (`workspace_inheritance_and_dotted_key_impostors_lose_the_ledger_exemption` scans `member/`, never
    /// the root). Confirmed live: deleting the `canon_root == canon_dir` early-return (falling through to
    /// the members-only check, which a non-virtual root fails since `workspace_members` lists its members,
    /// never itself) left `cargo test --workspace` fully green. Without that branch a non-virtual root's
    /// OWN honest `{ workspace = true }` dependency loses the exemption it is entitled to — the opposite
    /// direction from a silent under-report (a false impostor charge on an honest crate), but still a
    /// correctness property this suite could not see break.
    #[test]
    fn a_non_virtual_workspace_root_resolves_workspace_true_against_its_own_table() {
        let d = std::env::temp_dir().join(format!("candor-collide5-selfroot-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(d.join("member/src")).unwrap();
        std::fs::create_dir_all(d.join("src")).unwrap();
        // Non-virtual root: BOTH `[package]` (it is scanned as a crate in its own right) AND `[workspace]`
        // (it also owns the `[workspace.dependencies]` table its own `{ workspace = true }` resolves
        // against). `log` is genuinely registry-sourced (bare version) — an HONEST case that must keep
        // the CALIBRATED_CRATES exemption.
        std::fs::write(d.join("Cargo.toml"),
            "[package]\nname = \"victim\"\nversion = \"0.1.0\"\n\n[workspace]\nmembers = [\"member\"]\n\n\
             [workspace.dependencies]\nlog = \"0.4\"\n\n[dependencies]\nlog = { workspace = true }\n"
        ).unwrap();
        std::fs::write(d.join("src/lib.rs"),
            "pub fn exfiltrate() { log::totally_unmodelled_tail(\"http://evil.example\"); }\n"
        ).unwrap();
        std::fs::write(d.join("member/Cargo.toml"), "[package]\nname = \"member\"\nversion = \"0.1.0\"\n").unwrap();
        std::fs::write(d.join("member/src/lib.rs"), "pub fn noop() {}\n").unwrap();
        let idx = DepIndex::default();
        let (rc, body) = scan_one(&d.to_string_lossy(), ScanOpts {
            prefix: String::new(), want_json: true, include_tests: false, policy: None,
            baseline: None, ws_member: false, quiet: true, deps_idx: &idx, peek_excluded: false,
        }, &crate::gate::begin_run());
        assert_eq!(rc, 0, "scan should succeed:\n{body:?}");
        let v: serde_json::Value = serde_json::from_str(&body.unwrap()).unwrap();
        let _ = std::fs::remove_dir_all(&d);
        assert!(v["functions"].as_array().unwrap().is_empty(),
            "a non-virtual workspace ROOT resolving its OWN honest `{{ workspace = true }}` dependency \
             against its OWN `[workspace.dependencies]` table must keep the CALIBRATED_CRATES exemption — \
             it is entitled to that table by BEING the root, not by being listed as one of its own \
             members:\n{v:#}");
    }

    /// R59-CLASS PROBE (the ~79-crate audit `3cb1906`'s own commit message named as still open):
    /// `clap::Arg::env(name)` calls `env::var_os(&name)` DIRECTLY at builder time (clap_builder
    /// 4.6.6, builder/arg.rs:2205-2213) — a real `Env` read, independent of and long before
    /// `Command::get_matches()`'s own (already-classified) argv/env read. classify()'s own comment
    /// calls the verb "too generic to gate safely" and leaves it unmodeled — but unlike libc's fd
    /// verbs (genuinely ambiguous: a fd could be Fs/Net/Ipc), `::env` inside the `crate_name == "clap"`
    /// arm is crate-gated already, and clap_builder has exactly ONE `pub fn` ending `::env`
    /// (`Arg::env`; `env_os` is a deprecated one-line delegate to it) — no ambiguity survives the
    /// crate gate. Because `clap` sits in `CALIBRATED_CRATES` and NOT `CALIBRATED_BUT_PARTIAL_CRATES`,
    /// the coverage ledger reads the miss as reviewed-pure: a fn whose entire effectful surface is
    /// `Arg::new("x").env("MY_VAR")` (no `get_matches` call anywhere) vanishes with zero effects AND
    /// zero disclosure — the exact R59 shape, not a hypothetical.
    #[test]
    fn clap_arg_env_reads_env_var_directly_at_builder_time() {
        let run = |name: &str, src: &str| -> serde_json::Value {
            let d = std::env::temp_dir().join(format!("candor-clapenv-{name}-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&d);
            std::fs::create_dir_all(d.join("src")).unwrap();
            std::fs::write(d.join("Cargo.toml"),
                format!("[package]\nname = \"{name}\"\n[dependencies]\nclap = \"4\"\n")).unwrap();
            std::fs::write(d.join("src/lib.rs"), src).unwrap();
            let idx = DepIndex::default();
            let (rc, body) = scan_one(&d.to_string_lossy(), ScanOpts {
                prefix: String::new(), want_json: true, include_tests: false, policy: None,
                baseline: None, ws_member: false, quiet: true, deps_idx: &idx, peek_excluded: false,
            }, &crate::gate::begin_run());
            assert_eq!(rc, 0, "scan should succeed:\n{body:?}");
            let v = serde_json::from_str(&body.unwrap()).unwrap();
            let _ = std::fs::remove_dir_all(&d);
            v
        };
        // A fn whose ONLY effect is `Arg::env` — no `get_matches` anywhere. THE FIX: `Arg::env` is now
        // classified `Env` directly (verified unambiguous — see classify()'s own comment), not carved
        // into `CALIBRATED_BUT_PARTIAL_CRATES`, so this must be a precise Env charge, not a disclosure.
        let bare = run("clapenvbare",
            "pub fn declare(name: &str) -> clap::Arg { clap::Arg::new(name).env(\"MY_VAR\") }\n");
        assert!(
            !bare["functions"].as_array().unwrap().is_empty(),
            "a real env-var read must not vanish the function from the report entirely:\n{bare:#}"
        );
        assert_eq!(effs(fn_entry(&bare, "declare")), vec!["Env"],
            "`Arg::env` performs a real, unambiguous env::var_os read at builder time — before the fix \
             this vanished with zero effects AND zero disclosure (the R59 shape: a calibrated crate's \
             unclassified verb reading as reviewed-pure):\n{bare:#}");
        assert!(fn_entry(&bare, "declare").get("invisible").is_none(),
            "a classified clap call must not ALSO be flagged invisible:\n{bare:#}");
        // CONTROL: mixing the real effect with a pure builder setter keeps exactly one effect — this fix
        // must not spray Env over clap's other setters (`classify()`'s own unit tests separately pin
        // `Arg::about` -> None; this proves it end-to-end through the scanner too).
        let mixed = run("clapenvmixed",
            "pub fn declare(name: &str) -> clap::Arg { \
             clap::Arg::new(name).about(\"desc\").env(\"MY_VAR\") }\n");
        assert_eq!(effs(fn_entry(&mixed, "declare")), vec!["Env"],
            "`Arg::about` must stay pure beside the real `Arg::env` effect:\n{mixed:#}");
    }

    /// R59-CLASS PROBE #2: `console::Term` implements raw `std::io::{Read,Write}` (console 0.15.11,
    /// term.rs:622-659) — `Term::write`/`Term::flush` call `self.write_through(buf)` (a real terminal
    /// write, the SAME primitive `Term::write_line` — already classified Ipc — uses), and `Term::read`
    /// calls `io::stdin().read(buf)` directly. classify()'s existing `console` rule only names the
    /// crate's OWN convenience methods (`write_line`/`read_line`/`read_char`/...); the generic
    /// `Read`/`Write` trait methods are a SEPARATE real entry point to the identical tty channel, missed
    /// because their names (`read`/`write`/`flush`) are the ones every I/O type shares — but crate-gated
    /// on `crate_name == "console"`, term.rs defines exactly one `write`/`flush`/`read` each (no
    /// ambiguity survives the gate, same shape as `clap::Arg::env` above). `console` is fully
    /// `CALIBRATED_CRATES`, so a fn that only does `term.write_all(b"x")` (no `write_line` call) reads
    /// as reviewed-pure today.
    #[test]
    fn console_term_raw_write_and_read_trait_impls_are_the_same_ipc_channel() {
        let run = |name: &str, src: &str| -> serde_json::Value {
            let d = std::env::temp_dir().join(format!("candor-consoleio-{name}-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&d);
            std::fs::create_dir_all(d.join("src")).unwrap();
            std::fs::write(d.join("Cargo.toml"),
                format!("[package]\nname = \"{name}\"\n[dependencies]\nconsole = \"0.15\"\n")).unwrap();
            std::fs::write(d.join("src/lib.rs"), src).unwrap();
            let idx = DepIndex::default();
            let (rc, body) = scan_one(&d.to_string_lossy(), ScanOpts {
                prefix: String::new(), want_json: true, include_tests: false, policy: None,
                baseline: None, ws_member: false, quiet: true, deps_idx: &idx, peek_excluded: false,
            }, &crate::gate::begin_run());
            assert_eq!(rc, 0, "scan should succeed:\n{body:?}");
            let v = serde_json::from_str(&body.unwrap()).unwrap();
            let _ = std::fs::remove_dir_all(&d);
            v
        };
        let bare = run("consoleiobare",
            "pub fn shout(t: &mut console::Term) { use std::io::Write; let _ = t.write(b\"hi\"); }\n");
        assert!(
            !bare["functions"].as_array().unwrap().is_empty(),
            "a real tty write must not vanish the function from the report entirely:\n{bare:#}"
        );
        assert_eq!(effs(fn_entry(&bare, "shout")), vec!["Ipc"],
            "`Term::write` performs the same real tty write `Term::write_line` is already classified \
             Ipc for — before the fix this vanished with zero effects AND zero disclosure:\n{bare:#}");
        // `Term::read` — the read-side sibling (`io::stdin().read(buf)` directly).
        let read = run("consoleioread",
            "pub fn listen(t: &mut console::Term) -> [u8; 4] { \
             use std::io::Read; let mut b = [0u8; 4]; let _ = t.read(&mut b); b }\n");
        assert_eq!(effs(fn_entry(&read, "listen")), vec!["Ipc"],
            "`Term::read` reads real stdin, the same channel `read_line`/`read_char` already cover:\n{read:#}");
        // CONTROL: mixing the real effect with an unrelated pure `Style` call keeps exactly one effect —
        // the fix's `Term::`-scoped match must not spread to console's other types (classify()'s own
        // unit tests separately pin `Style::cyan` -> None; this proves it end-to-end through the scanner).
        let mixed = run("consoleiomixed",
            "pub fn shout(t: &mut console::Term) -> console::Style { \
             use std::io::Write; let _ = t.write(b\"hi\"); console::Style::new().cyan() }\n");
        assert_eq!(effs(fn_entry(&mixed, "shout")), vec!["Ipc"],
            "`Style::cyan` must stay pure beside the real `Term::write` effect:\n{mixed:#}");
    }

    /// R59-CLASS PROBE #3: `arboard`'s `Get`/`Set` cursor terminals cover `text`/`image`/`html` but not
    /// their SIBLING `file_list` (arboard 3.6.1, lib.rs:205,251 — `Get::file_list`/`Set::file_list`, the
    /// same builder-then-terminal shape, reading/writing the OS clipboard's file-path list). Missed not
    /// because it's ambiguous (it's the only `file_list` in the crate) but because the completeness
    /// gate's OWN generator only triggers on a self-scan `inferred` set containing Fs/Net/Db/Exec
    /// (`eval/coverage-gate/generate.py`'s own documented trigger) — Clipboard isn't in that set, so a
    /// missing Clipboard verb is structurally invisible to the gate regardless of how it's phrased.
    /// `arboard` is fully `CALIBRATED_CRATES`, so `clipboard.set().file_list(&paths)` reads reviewed-pure.
    #[test]
    fn arboard_file_list_terminal_is_the_same_clipboard_effect_as_text() {
        let run = |name: &str, src: &str| -> serde_json::Value {
            let d = std::env::temp_dir().join(format!("candor-arbfl-{name}-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&d);
            std::fs::create_dir_all(d.join("src")).unwrap();
            std::fs::write(d.join("Cargo.toml"),
                format!("[package]\nname = \"{name}\"\n[dependencies]\narboard = \"3\"\n")).unwrap();
            std::fs::write(d.join("src/lib.rs"), src).unwrap();
            let idx = DepIndex::default();
            let (rc, body) = scan_one(&d.to_string_lossy(), ScanOpts {
                prefix: String::new(), want_json: true, include_tests: false, policy: None,
                baseline: None, ws_member: false, quiet: true, deps_idx: &idx, peek_excluded: false,
            }, &crate::gate::begin_run());
            assert_eq!(rc, 0, "scan should succeed:\n{body:?}");
            let v = serde_json::from_str(&body.unwrap()).unwrap();
            let _ = std::fs::remove_dir_all(&d);
            v
        };
        // Isolated from `Clipboard::get()`/`::set()` (which ALREADY classify Clipboard on their own,
        // since the constructor verbs `get`/`set` share this crate's match arm) by taking an
        // ALREADY-CONSTRUCTED cursor as a parameter — the exact shape `Get::text`/`Set::text` are
        // already covered for, proving `file_list` specifically, not the constructor, is the gap.
        let get = run("arbflget",
            "pub fn paste_paths(g: arboard::Get) -> Vec<std::path::PathBuf> { g.file_list().unwrap() }\n");
        assert_eq!(effs(fn_entry(&get, "paste_paths")), vec!["Clipboard"],
            "`Get::file_list` is the same clipboard read `Get::text` already covers — before the fix \
             this vanished with zero effects AND zero disclosure:\n{get:#}");
        let set = run("arbflset",
            "pub fn copy_paths(s: arboard::Set, p: &[std::path::PathBuf]) { let _ = s.file_list(p); }\n");
        assert_eq!(effs(fn_entry(&set, "copy_paths")), vec!["Clipboard"],
            "`Set::file_list` is the same clipboard write `Set::text` already covers:\n{set:#}");
        // `Clipboard::clear_with().default()` is `clear()`'s own documented alternate entry point
        // (lib.rs:156-163: `clear()` is literally `self.clear_with().default()`) — same shape as
        // `ignore::Walk::new` beside `WalkBuilder::build`.
        let clear = run("arbflclear",
            "pub fn wipe(c: &mut arboard::Clipboard) { let _ = c.clear_with().default(); }\n");
        assert_eq!(effs(fn_entry(&clear, "wipe")), vec!["Clipboard"],
            "`Clear::default` is `Clipboard::clear`'s own sibling entry point:\n{clear:#}");
    }

    /// `CANDOR_PANIC_ON_FILE` is process-global and `INCREMENTAL` is a thread-local the tests set by
    /// hand, so the abort tests take this lock rather than race each other's injection window.
    fn abort_injection_lock() -> std::sync::MutexGuard<'static, ()> {
        static L: std::sync::Mutex<()> = std::sync::Mutex::new(());
        L.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Build a two-file crate whose `src/bad.rs` is the injection target, and return its dir + the
    /// path of a `deny Fs` policy. `src/good.rs` is deliberately PURE, so the ONLY thing a gate can
    /// have an opinion about lives in the file that aborts — which is what lets exit 0 mean
    /// "certified green over a hole" rather than "found nothing to say".
    fn abort_fixture(tag: &str) -> (std::path::PathBuf, String) {
        let d = std::env::temp_dir().join(format!("candor-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(d.join("src")).unwrap();
        std::fs::write(d.join("Cargo.toml"), "[package]\nname = \"aborter\"\n").unwrap();
        std::fs::write(d.join("src/lib.rs"), "pub mod good;\npub mod bad;\n").unwrap();
        std::fs::write(d.join("src/good.rs"), "pub fn pure_one() -> u32 { 41 + 1 }\n").unwrap();
        std::fs::write(d.join("src/bad.rs"),
            "pub fn also_reads() { let _ = std::fs::read_to_string(\"/etc/y\"); }").unwrap();
        let policy = d.join("candor.policy");
        std::fs::write(&policy, "deny Fs\n").unwrap();
        (d.clone(), policy.to_string_lossy().into_owned())
    }

    /// One `--incremental` scan of `dir`, with the fault optionally injected. Returns
    /// `(exit code, report JSON)`.
    fn incremental_scan(
        dir: &std::path::Path, out: &str, policy: &str, inject: Option<&str>,
    ) -> (i32, serde_json::Value) {
        match inject {
            Some(f) => std::env::set_var("CANDOR_PANIC_ON_FILE", f),
            None => std::env::remove_var("CANDOR_PANIC_ON_FILE"),
        }
        INCREMENTAL.with(|c| c.set(true));
        let (rc, body) = scan_one(&dir.to_string_lossy(), ScanOpts {
            prefix: out.to_string(), want_json: true, include_tests: false,
            policy: Some(policy.to_string()), baseline: None, ws_member: false, quiet: true,
            deps_idx: &DepIndex::default(), peek_excluded: false,
        }, &crate::gate::begin_run());
        INCREMENTAL.with(|c| c.set(false));
        std::env::remove_var("CANDOR_PANIC_ON_FILE");
        (rc, serde_json::from_str(&body.unwrap()).unwrap())
    }

    /// THE WARM RUN MUST SAY WHAT THE COLD RUN SAID. Containing the abort per file (the commit above)
    /// left the cache write-back untouched, so the aborted file persisted as
    /// `{ its REAL content hash, the CURRENT decl index, fninfos: [] }` — indistinguishable from a file
    /// that genuinely has no functions. Run 2 over identical bytes reused it, `continue`d before the
    /// `catch_unwind` ever ran, disclosed nothing, and a configured gate went **exit 0 GREEN** over a
    /// file performing `Fs` under `deny Fs` — measured: cold 2, warm 0. That is strictly worse than the
    /// crash it replaced, which at least failed closed, and worse still for being reproducible.
    ///
    /// Three arms, because each is satisfiable by a wrong fix:
    ///   - COLD then WARM on the SAME bytes, THE FILE STILL ABORTING: same exit code, same
    ///     `unanalyzed`, same `functions`. This is the requirement the original defect established and
    ///     it holds however the warm run is implemented — by replaying the cached disclosure or, as
    ///     now, by re-attempting the walk and aborting again.
    ///   - the CONTROL, no injection anywhere: warm must NOT invent a disclosure, and must still reuse
    ///     the cache (a fix that simply refuses to cache, or discloses on every warm run, passes arm 1
    ///     and fails here).
    ///   - the round-1 read/parse failure is untouched: it already re-disclosed every run, because it
    ///     `continue`s BEFORE the write-back. That asymmetry is what located the defect.
    ///
    /// The injection stays ON for the warm arm on purpose. Its sibling
    /// (`a_cached_abort_is_re_attempted_rather_than_latched`) is the other direction — the abort is
    /// NOT a function of the file's bytes, so a warm run over a file that no longer aborts must not be
    /// handed the old answer.
    #[test]
    fn a_warm_incremental_run_over_a_still_aborting_file_says_what_the_cold_run_said() {
        let _lock = abort_injection_lock();
        let (d, policy) = abort_fixture("warmabort");
        let out = |n: &str| d.join(n).to_string_lossy().into_owned();

        let (cold_rc, cold) = incremental_scan(&d, &out("cold"), &policy, Some("src/bad.rs"));
        let (warm_rc, warm) = incremental_scan(&d, &out("warm"), &policy, Some("src/bad.rs"));

        assert_eq!(cold_rc, 2, "a configured gate must not certify a scan with a hole in it:\n{cold:#}");
        assert_eq!(
            warm_rc, cold_rc,
            "THE WARM RUN CERTIFIED A HOLE THE COLD RUN REFUSED TO: the cache persisted the aborted \
             file as an empty-but-confident entry.\ncold:\n{cold:#}\nwarm:\n{warm:#}"
        );
        {
            let (name, v) = ("warm", &warm);
            assert_eq!(
                v["unanalyzed"], cold["unanalyzed"],
                "{name}: the aborted file must be disclosed IDENTICALLY to the cold run:\n{v:#}"
            );
            assert_eq!(v["functions"], cold["functions"], "{name}: the surviving files' effects moved:\n{v:#}");
            assert_eq!(v["analyzed"], cold["analyzed"], "{name}: the coverage ledger moved:\n{v:#}");
        }
        assert!(
            cold["unanalyzed"].as_array().is_some_and(|u| u.iter().any(|x| x["path"] == "src/bad.rs")),
            "the fixture stopped exercising the abort at all:\n{cold:#}"
        );

        // CONTROL — the SAME crate, from a FRESH cache, with nothing injected. (It has to be a fresh
        // dir: the cache above legitimately still holds the abort, and replaying it there is the
        // behaviour under test, not a control.) A fix that discloses on every warm run, or that stops
        // caching, satisfies the arms above and fails here.
        let (dc, policy_c) = abort_fixture("warmabort-control");
        let outc = |n: &str| dc.join(n).to_string_lossy().into_owned();
        let (c1_rc, c1) = incremental_scan(&dc, &outc("c1"), &policy_c, None);
        let (c2_rc, c2) = incremental_scan(&dc, &outc("c2"), &policy_c, None);
        assert_eq!((c1_rc, c2_rc), (1, 1), "the control must FIND the violation, both runs:\n{c1:#}\n{c2:#}");
        assert!(c1["unanalyzed"].as_array().is_none_or(|u| u.is_empty()),
                "the control's cold run invented a disclosure:\n{c1:#}");
        assert!(c2["unanalyzed"].as_array().is_none_or(|u| u.is_empty()),
                "the control's WARM run invented a disclosure — this is the mirror sin, a gate that can \
                 never go green:\n{c2:#}");
        assert_eq!(c1["functions"], c2["functions"], "the warm control's reuse is not byte-equal:\n{c2:#}");
        let _ = std::fs::remove_dir_all(&dc);
        let _ = std::fs::remove_dir_all(&d);
    }

    /// THE ABORT IS NOT A FUNCTION OF THE FILE'S BYTES, so a content-hash match is the wrong licence to
    /// replay it. `4f7b704` established the mechanism: proc-macro2's fallback `Span` indexes a
    /// THREAD-LOCAL source map, and whether a moved span falls past the walking thread's map "depends on
    /// how much each thread happened to parse, i.e. on the rest of the crate" — which under rayon
    /// work-stealing differs between two runs over identical input. So the cached-abort replay latched a
    /// ONE-OFF into the cache and served it forever, on a file that now walks perfectly well.
    ///
    /// The direction is the mirror of the defect it was fixing rather than the same one — a spurious
    /// `unanalyzed` entry and a gate that cannot go green, not a false all-clear — but it is still a
    /// cached wrong answer that no amount of re-running clears, and `--incremental` is exactly where
    /// nobody re-runs from cold. The treatment is to RE-ATTEMPT: a cached abort marks the entry's
    /// FnInfos as underived, the file goes back through parse + walk, and it either aborts again (and
    /// discloses, by the cold path, verbatim) or produces the answer it always owed.
    ///
    /// Removing the env-var injection between runs is a faithful model of that: the injection is
    /// deliberately outside the cache key, so run 2's bytes, decl index and cache entry are all
    /// identical to run 1's and the ONLY thing that changed is whether the walk aborts — which is
    /// precisely the variable the real trigger moves and the content hash cannot see.
    #[test]
    fn a_cached_abort_is_re_attempted_rather_than_latched() {
        let _lock = abort_injection_lock();
        let (d, policy) = abort_fixture("latchabort");
        let out = |n: &str| d.join(n).to_string_lossy().into_owned();

        let (rc1, v1) = incremental_scan(&d, &out("a1"), &policy, Some("src/bad.rs"));
        assert_eq!(rc1, 2, "the fixture must abort first:\n{v1:#}");
        assert!(v1["unanalyzed"].as_array().is_some_and(|u| u.iter().any(|x| x["path"] == "src/bad.rs")),
                "the fixture stopped exercising the abort at all:\n{v1:#}");
        // The abort DID ride into the cache — this is what makes run 2 a test of the replay gate and
        // not of a cache that simply forgot. (Also the original defect's guard: the entry must not be
        // an indistinguishable `fninfos: []`.)
        let cache: serde_json::Value = serde_json::from_slice(
            &std::fs::read(d.join(".candor/cache/scan-cache.json")).unwrap()).unwrap();
        assert!(cache["files"]["src/bad.rs"]["aborted"].is_string(),
                "the aborted file was cached as an ordinary analysed entry:\n{cache:#}");

        // Run 2: SAME bytes, SAME decl index, SAME cache entry — only the walk no longer aborts.
        let (rc2, v2) = incremental_scan(&d, &out("a2"), &policy, None);
        assert!(v2["unanalyzed"].as_array().is_none_or(|u| u.is_empty()),
                "A ONE-OFF ABORT WAS LATCHED INTO THE CACHE: the file walks cleanly now and the warm run \
                 still discloses it as unanalyzed, forever, off a content hash that cannot see the \
                 difference:\n{v2:#}");
        assert_eq!(rc2, 1,
                   "…and the gate can never go green again — it must now FIND the re-walked file's Fs, \
                    not refuse to certify a hole that isn't there:\n{v2:#}");
        let quals: Vec<&str> = v2["functions"].as_array().into_iter().flatten()
            .filter_map(|f| f["fn"].as_str()).collect();
        assert!(quals.iter().any(|q| q.contains("also_reads")),
                "the re-attempt disclosed nothing but derived nothing either — the effects are still \
                 missing from the report:\n{v2:#}");

        // …and the cleared state is what got cached: run 3 neither re-discloses nor loses the effect.
        let (rc3, v3) = incremental_scan(&d, &out("a3"), &policy, None);
        assert_eq!(rc3, 1, "the cleared state did not persist into the next warm run:\n{v3:#}");
        assert_eq!(v3["functions"], v2["functions"], "the re-attempt did not cache what it derived:\n{v3:#}");
        assert_eq!(v3["unanalyzed"], v2["unanalyzed"], "the disclosure came back on run 3:\n{v3:#}");
        let _ = std::fs::remove_dir_all(&d);
    }

    /// …and the disclosure must CLEAR itself. A cached abort that survives a clean re-walk would be the
    /// mirror of the defect: a permanently-incomplete verdict, exit 2 forever, on a file the engine can
    /// now read perfectly well. The trigger is a decl-index move (the cache's other reuse gate), which
    /// is what forces the file back through the walk on unchanged bytes.
    #[test]
    fn a_cached_abort_clears_once_the_file_walks_cleanly() {
        let _lock = abort_injection_lock();
        let (d, policy) = abort_fixture("clearabort");
        let out = |n: &str| d.join(n).to_string_lossy().into_owned();

        let (rc1, v1) = incremental_scan(&d, &out("a1"), &policy, Some("src/bad.rs"));
        assert_eq!(rc1, 2, "the fixture must abort first:\n{v1:#}");

        // Move the merged decl index WITHOUT touching src/bad.rs — a struct with a field lands in the
        // digest, so every file's cached FnInfos go stale and bad.rs is re-walked on the same bytes.
        std::fs::write(d.join("src/good.rs"),
            "pub struct Moved { pub f: String }\npub fn pure_one() -> u32 { 41 + 1 }\n").unwrap();
        let (rc2, v2) = incremental_scan(&d, &out("a2"), &policy, None);
        let (rc3, v3) = incremental_scan(&d, &out("a3"), &policy, None);

        assert_eq!(rc2, 1, "the re-walked file's Fs must be FOUND, not still disclosed as a hole:\n{v2:#}");
        assert!(v2["unanalyzed"].as_array().is_none_or(|u| u.is_empty()),
                "the cached abort outlived the clean re-walk:\n{v2:#}");
        assert_eq!(rc3, 1, "…and the cleared state persists into the next warm run:\n{v3:#}");
        assert_eq!(v3["functions"], v2["functions"], "the cleared entry did not cache what it derived:\n{v3:#}");
        let _ = std::fs::remove_dir_all(&d);
    }

    /// A cache written by an OLDER binary must be discarded, not read — for every rev whose new field
    /// defaults to a reading that is a false all-clear. Both revs pinned here are exactly that shape:
    ///
    ///   rev7 -> rev8  `FileCache.aborted`. `#[serde(default)]` keeps the struct additive, so an old
    ///                 entry deserializes as `None` — "this file was analysed and has no functions",
    ///                 which is precisely the hole the field exists to disclose.
    ///   rev8 -> rev9  `FileDecls.reexports`. An old entry deserializes as an EMPTY vec — "this file
    ///                 re-exports nothing" — restoring the submodule-re-export under-report from a warm
    ///                 cache, where no test that scans from scratch would ever see it.
    ///   rev9 -> rev10 `FnInfo.dispatch` (⟨peek-scope-attribution⟩). An old entry deserializes as an
    ///                 EMPTY Vec — "this fn dispatches on no local trait" — which for a warm-cached fn
    ///                 that genuinely dispatches on one is exactly the silent under-report the field
    ///                 exists to close in the peek's out-of-scope scope-matching.
    ///   rev10 -> rev11 the `Type::<construct>` DROP-GLUE marker moved from the `let` BINDER to the
    ///                 construction EXPRESSION, and now covers the tuple-struct/unit/enum spellings and
    ///                 by-value parameters. A `FnInfo` cached by an older binary carries the OLD, sparse
    ///                 marker set, so a warm cache would replay the binder-keyed reading — a guard
    ///                 released in any of sixteen other positions read PURE — with no test that scans
    ///                 cold able to see it.
    ///   rev11 -> rev12 `FileDecls.mod_aliases` (R99). An old entry deserializes EMPTY — "this file gives
    ///                 no std/dependency item a second crate-local spelling" — so a warm cache replays the
    ///                 blanket-`deny`-defeating silence for `mod facade { pub use std::process::Command; }`
    ///                 and `pub type Cmd = std::process::Command;`.
    ///   rev12 -> rev13 `mod_aliases` gained a NEW KIND of entry — a submodule's external GLOB re-export,
    ///                 keyed `<module>::*glob` (R99 shape 1). The FIELD is unchanged, so a rev12 entry
    ///                 deserializes without complaint into a map that simply has no glob in it, and the
    ///                 warm cache replays `functions: []` over a module that globs `std::fs`. A field
    ///                 ADDITION is not the only thing that needs a bump: a change in what an EXISTING
    ///                 field records does too, and that is the reading this rev is here to stop.
    ///   rev13 -> rev14 `FileDecls` gained `callable_statics` (R101). A rev13 entry has none, so it
    ///                 deserializes EMPTY — "this file declares no externally-installable callback slot"
    ///                 — and the warm cache replays `fire` ABSENT for a `static CB: OnceLock<Box<dyn
    ///                 Fn()>>` whose installed callback demonstrably writes a file.
    ///
    /// The schema token is the only thing standing between those readings, so it is pinned rather than
    /// trusted. (The `aborted` disclosure is what this fixture MEASURES in every case: it is the visible
    /// consequence a mis-read entry produces, and the same discard covers every field above.)
    #[test]
    fn an_older_schema_cache_entry_is_discarded_rather_than_read_as_analysed() {
        for stale in ["rev7", "rev8", "rev9", "rev11", "rev12", "rev13"] {
            let _lock = abort_injection_lock();
            let (d, policy) = abort_fixture(&format!("oldcache{stale}"));
            let out = |n: &str| d.join(n).to_string_lossy().into_owned();
            let (rc1, v1) = incremental_scan(&d, &out("a1"), &policy, Some("src/bad.rs"));
            assert_eq!(rc1, 2, "the fixture must abort first:\n{v1:#}");

            // Doctor the cache into exactly what the older binary would have left: the entry with no
            // `aborted` key at all, under the older schema token.
            let p = d.join(".candor/cache/scan-cache.json");
            let mut c: serde_json::Value = serde_json::from_slice(&std::fs::read(&p).unwrap()).unwrap();
            let old = c["schema"].as_str().unwrap().replace("/rev14/", &format!("/{stale}/"));
            assert!(old.contains(stale), "the schema rev token moved — update this test: {c}");
            c["schema"] = serde_json::Value::String(old);
            for (_, e) in c["files"].as_object_mut().unwrap() {
                e.as_object_mut().unwrap().remove("aborted");
            }
            std::fs::write(&p, serde_json::to_vec(&c).unwrap()).unwrap();

            let (rc2, v2) = incremental_scan(&d, &out("a2"), &policy, Some("src/bad.rs"));
            assert_eq!(
                rc2, 2,
                "a {stale} cache entry was TRUSTED — its missing field read as an analysed, \
                 function-free file and the gate certified the hole:\n{v2:#}"
            );
            assert_eq!(v2["unanalyzed"], v1["unanalyzed"],
                       "the discarded cache must re-derive the disclosure:\n{v2:#}");
            let _ = std::fs::remove_dir_all(&d);
        }
    }

    #[test]
    fn a_file_whose_walk_aborts_is_contained_and_disclosed_not_dropped() {
        let _lock = abort_injection_lock();
        // A panic in one file used to take the WHOLE run down — and with `--deps` that meant the whole
        // dependency TREE, so a chained consumer proceeded with fewer dep reports than it asked for and
        // never learned it. proc-macro2 aborts deterministically on `getrandom` 0.3.4/0.4.2
        // (`unreachable!("Invalid span with no related FileInfo!")`), on input candor does not control.
        //
        // Contained per FILE and disclosed through the ⟨0.21⟩ `unanalyzed` channel, which already carries
        // "this file failed to parse" and already makes a configured gate refuse to go green. The fault is
        // INJECTED because the real trigger needs a whole crate's parse state: a containment that cannot be
        // fired is a containment nobody has checked.
        let d = std::env::temp_dir().join(format!("candor-abort-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(d.join("src")).unwrap();
        std::fs::write(d.join("Cargo.toml"), "[package]\nname = \"aborter\"\n").unwrap();
        std::fs::write(d.join("src/lib.rs"), "pub mod good;\npub mod bad;\n").unwrap();
        std::fs::write(d.join("src/good.rs"),
            "pub fn reads() { let _ = std::fs::read_to_string(\"/etc/x\"); }").unwrap();
        std::fs::write(d.join("src/bad.rs"),
            "pub fn also_reads() { let _ = std::fs::read_to_string(\"/etc/y\"); }").unwrap();
        let prefix = d.join("out/r").to_string_lossy().into_owned();
        std::env::set_var("CANDOR_PANIC_ON_FILE", "src/bad.rs");
        let (rc, body) = scan_one(&d.to_string_lossy(), ScanOpts {
            prefix, want_json: true, include_tests: false, policy: None, baseline: None, ws_member: false, quiet: true,
            deps_idx: &DepIndex::default(), peek_excluded: false,
        }, &crate::gate::begin_run());
        std::env::remove_var("CANDOR_PANIC_ON_FILE");
        let v: serde_json::Value = serde_json::from_str(&body.unwrap()).unwrap();
        let _ = std::fs::remove_dir_all(&d);
        assert_eq!(rc, 0, "the run must survive one file aborting:\n{v}");
        // 1. the REST of the crate still analysed — containment, not abandonment
        let quals: Vec<String> = v["functions"].as_array().into_iter().flatten()
            .filter_map(|f| f["fn"].as_str().map(String::from)).collect();
        assert!(quals.iter().any(|q| q.contains("reads")),
                "the surviving file's effects were lost — this contained too much:\n{v}");
        // 2. and the lost file is NAMED, not silently missing. Absence from `functions` is a purity claim
        //    under ⟨0.21⟩, so a dropped file with no disclosure is the cardinal sin wearing a crash.
        let un = v["unanalyzed"].as_array().cloned().unwrap_or_default();
        assert!(un.iter().any(|u| u["path"].as_str() == Some("src/bad.rs")),
                "the aborted file was dropped WITHOUT disclosure:\n{v}");
    }

    /// A Pass-B walk of an AST parsed by ANOTHER THREAD must not panic.
    ///
    /// proc-macro2's fallback `Span` is a pair of byte offsets into a THREAD-LOCAL source map, and
    /// candor parses files on rayon workers but walks them on the collector thread (see `SendFile`).
    /// So every span inside a moved AST indexes a map the walking thread does not have. candor never
    /// reads such a span itself — `loc` is resolved on the parse worker — but `visit_macro` hands the
    /// macro's token stream straight back to syn, and syn's `parse_negative_lit` JOINS the `-` punct's
    /// span with the literal's. `Span::join` looks the receiver up: past the end of this thread's map
    /// that is `unreachable!("Invalid span with no related FileInfo!")`.
    ///
    /// `-1` inside a macro body is all it takes; getrandom 0.3.4 / 0.4.2 spell it
    /// `debug_assert!({ match ret { 0 => true, -1 => …, _ => false } })` and took the whole scan down.
    /// The walking thread is spawned FRESH so its map holds only proc-macro2's dummy file — which is
    /// what makes this deterministic where the real crate was a lottery (the panic needs the worker's
    /// span to fall PAST the collector thread's map; falling INSIDE it silently resolves to the wrong
    /// file instead).
    #[test]
    fn a_macro_body_reparse_survives_the_ast_crossing_a_thread_boundary() {
        const SRC: &str = "fn wait(ret: i32) {\n    debug_assert!({\n        match ret {\n            0 => true,\n            -1 => std::fs::read(\"/etc/x\").is_ok(),\n            _ => false,\n        }\n    });\n}\n";
        // PARSE on its own thread: the spans below are offsets into THAT thread's source map.
        let (parsed, locs) = std::thread::spawn(|| {
            let f = syn::parse_file(SRC).unwrap();
            let mut locs = Vec::new();
            fn_locs(&f.items, "src/lib.rs", false, &mut locs);
            (SendFile(f), locs)
        })
        .join()
        .unwrap();
        // WALK on a second, virgin thread.
        let fns = std::thread::spawn(move || {
            let (parsed, locs) = (parsed, locs);
            let (fields, returns): (FieldIndex, ReturnIndex) = Default::default();
            let (impls, tdecls, tfields): (TraitImplIndex, HashMap<String, LocalTrait>, TraitFieldIndex) =
                Default::default();
            let (fe, fet, ev): (FieldElemIndex, FieldElemTraitIndex, EnumVariantIndex) = Default::default();
            let evt: EnumVariantTraitIndex = Default::default();
            let (consts, lmac): (HashMap<String, String>, HashMap<String, String>) = Default::default();
            let mut uses = HashMap::new();
            let mut out = Vec::new();
            let mut li = 0usize;
            scan_items(
                &parsed.0.items, "", &locs, &mut li, false, &fields, &returns,
                TraitIndexes { impls: &impls, decls: &tdecls, fields: &tfields },
                ElemIndexes { field_elem: &fe, field_elem_trait: &fet, enum_variants: &ev, enum_variant_traits: &evt, ambiguous_enum_leaves: &std::collections::HashSet::new(), callable_statics: &std::collections::HashSet::new() },
                empty_lazy(), &consts, &lmac, &std::collections::HashSet::new(), &mut uses, &mut out,
            );
            out
        })
        .join()
        .expect("Pass B must survive an AST parsed on another thread (span source map is thread-local)");
        // …and the re-parse must still SEE THROUGH the macro. Without this half the test would pass just
        // as well if the fix were "stop parsing macro bodies" — which is a silent under-report.
        let wait = fns.iter().find(|f| f.leaf == "wait").expect("`wait` must be collected");
        assert!(
            wait.calls.iter().any(|c| c.path.contains("fs::read")),
            "the macro body's `fs::read` was lost: {:?}",
            wait.calls.iter().map(|c| c.path.as_str()).collect::<Vec<_>>()
        );
    }

    /// `visit_macro`'s expression re-parse is not the only place candor hands a MOVED token stream back
    /// to syn — `cfg_if!` and the `lazy_static!`/`thread_local!` bodies do too, and each is its own
    /// `respan_call_site` call site. One test per site, so removing any one of the four is a named
    /// failure rather than a silent regression. (Same construction as the macro-body test above:
    /// parse on one thread, use on a second, virgin one.)
    #[test]
    fn every_moved_token_reparse_site_survives_the_thread_boundary() {
        // Each source puts a `-1` where the site's own parser will read it: that is what reaches syn's
        // `parse_negative_lit`, the one place syn JOINs spans during a parse.
        const CFG_IF: &str = "fn f(k: i32) {\n    cfg_if::cfg_if! {\n        if #[cfg(unix)] {\n            let _ = match k { -1 => std::fs::read(\"/x\").is_ok(), _ => false };\n        } else {\n            let _ = k;\n        }\n    }\n}\n";
        const LAZY: &str = "lazy_static! {\n    static ref A: bool = match k() { -1 => std::fs::read(\"/x\").is_ok(), _ => false };\n}\n";
        const TLOCAL: &str = "thread_local! {\n    static B: bool = match k() { -1 => std::fs::read(\"/x\").is_ok(), _ => false };\n}\n";

        // cfg_if: goes through the collector, so drive it the same way as the macro-body test.
        let (parsed, locs) = std::thread::spawn(|| {
            let f = syn::parse_file(CFG_IF).unwrap();
            let mut locs = Vec::new();
            fn_locs(&f.items, "src/lib.rs", false, &mut locs);
            (SendFile(f), locs)
        })
        .join()
        .unwrap();
        let fns = std::thread::spawn(move || {
            let (parsed, locs) = (parsed, locs);
            let (fields, returns): (FieldIndex, ReturnIndex) = Default::default();
            let (impls, tdecls, tfields): (TraitImplIndex, HashMap<String, LocalTrait>, TraitFieldIndex) =
                Default::default();
            let (fe, fet, ev): (FieldElemIndex, FieldElemTraitIndex, EnumVariantIndex) = Default::default();
            let evt: EnumVariantTraitIndex = Default::default();
            let (consts, lmac): (HashMap<String, String>, HashMap<String, String>) = Default::default();
            let (mut uses, mut out, mut li) = (HashMap::new(), Vec::new(), 0usize);
            scan_items(
                &parsed.0.items, "", &locs, &mut li, false, &fields, &returns,
                TraitIndexes { impls: &impls, decls: &tdecls, fields: &tfields },
                ElemIndexes { field_elem: &fe, field_elem_trait: &fet, enum_variants: &ev, enum_variant_traits: &evt, ambiguous_enum_leaves: &std::collections::HashSet::new(), callable_statics: &std::collections::HashSet::new() },
                empty_lazy(), &consts, &lmac, &std::collections::HashSet::new(), &mut uses, &mut out,
            );
            out
        })
        .join()
        .expect("the cfg_if! arm walk must survive an AST parsed on another thread");
        assert!(
            fns.iter().any(|f| f.calls.iter().any(|c| c.path.contains("fs::read"))),
            "the cfg_if arm's `fs::read` was lost: {:?}",
            fns.iter().flat_map(|f| f.calls.iter().map(|c| c.path.as_str())).collect::<Vec<_>>()
        );

        // lazy_static! / thread_local!: the deferred-init parsers, driven directly on their tokens.
        for (src, want) in [(LAZY, "A"), (TLOCAL, "B")] {
            let parsed = std::thread::spawn(move || SendFile(syn::parse_file(src).unwrap()))
                .join()
                .unwrap();
            let got = std::thread::spawn(move || {
                let parsed = parsed;
                let syn::Item::Macro(m) = &parsed.0.items[0] else { panic!("expected a macro item") };
                let leaf = path_to_string(&m.mac.path);
                // `syn::Stmt` is not `Send`, so summarise inside the thread rather than returning it.
                if leaf.contains("lazy_static") {
                    lazy_static_macro_body(&m.mac.tokens)
                } else {
                    thread_local_macro_body(&m.mac.tokens)
                }
                .map(|(n, stmts)| (n, stmts.len()))
            })
            .join()
            .expect("the deferred-init body parse must survive an AST parsed on another thread");
            let (name, n_stmts) = got.unwrap_or_else(|| panic!("{want}'s init must still parse"));
            assert_eq!(name, want);
            assert_eq!(n_stmts, 1, "the init expr must survive as one stmt");
        }
    }

    /// THE QUIET HALF OF THE SPAN-CROSSING-A-THREAD DEFECT, asked of the ONLY span read that reaches
    /// output. `4f7b704` closed the loud tail (the `unreachable!` panic); its quiet sibling resolves a
    /// span against the WRONG FILE instead of aborting, and its precondition was measured at 72.4% of
    /// 88 927 macro re-parses — so the question is not whether it happens but whether it can reach
    /// anything a consumer keys on.
    ///
    /// IT CANNOT, AND THE REASON IS STRUCTURAL: `fn_locs` is the one span read whose result is
    /// published (`loc`), and it runs INSIDE the parse closure, on the worker that owns the map its
    /// spans index. Every other span read after the AST moves is either re-stamped to `call_site()`
    /// — `(0,0)`, the dummy file every thread's map is seeded with — or feeds a parse whose errors are
    /// discarded (`parse_nested_meta` in `cfg_eval`/`is_cfg_test`; the `macro_rules!` template parse
    /// re-parses from a STRING, which registers a file on the current thread).
    ///
    /// A comment stating that is an assertion (standing bar item 9), so this is the fixture. It scans a
    /// MULTI-FILE crate — enough files that rayon splits the parse across workers on any multi-core
    /// machine — and checks each published `loc` against the source it names: the file must exist, be
    /// long enough, and declare that function. A span resolved against a different file fails all three.
    ///
    /// MEASURED at corpus scale with the same oracle: **24 008 of 24 008** non-synthetic `loc` strings
    /// over 200 crates.io crates name a file that exists, is long enough, and declares the function —
    /// zero missing-file, zero short-file, zero wrong-line. The oracle was CALIBRATED rather than
    /// trusted: permuting each loc onto a DIFFERENT file of its own crate makes it flag 20 001 of
    /// 23 657 (84.5%, including 5 507 short-file), so it has real recall against exactly the shape it
    /// exists to detect — its blind spot is a same-named function at a similar offset in the other file.
    /// Separately, 200 crates scanned at four rayon thread counts (800 scans) are byte-identical, so no
    /// published field varies with how much each worker happened to parse, which is the quiet form's
    /// whole precondition. And the seeded control: moving `fn_locs` out of the parse closure — the
    /// defect this fixture guards against — does not go quiet, it PANICS on 57 of 60 crates.
    ///
    /// The honest limit: on a single-core machine rayon may run every parse on the calling thread, and
    /// then the property holds trivially. The test still pins the oracle; it loses its power, not its
    /// correctness.
    #[test]
    fn every_published_loc_names_the_source_that_declares_it() {
        use std::fmt::Write as _;
        let d = std::env::temp_dir().join(format!("candor-locoracle-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(d.join("src")).unwrap();
        std::fs::write(d.join("Cargo.toml"), "[package]\nname = \"locoracle\"\n").unwrap();
        // 24 modules of DIFFERENT lengths, so a loc landing in the wrong file is very likely to land
        // past its end or on an unrelated line rather than coincidentally on the right one.
        let mut lib = String::new();
        for i in 0..24u32 {
            writeln!(lib, "pub mod m{i};").unwrap();
            let mut body = String::new();
            for pad in 0..(i * 7) {
                writeln!(body, "// filler {pad}").unwrap();
            }
            // Each fn carries a doc comment, because the item SPAN starts at the doc comment and that
            // is exactly what a naive oracle mistakes for a wrong line (it did, on the first run).
            writeln!(body, "/// does a thing\npub fn work{i}() {{ std::fs::read(\"/tmp/x{i}\").ok(); }}").unwrap();
            std::fs::write(d.join("src").join(format!("m{i}.rs")), body).unwrap();
        }
        std::fs::write(d.join("src/lib.rs"), lib).unwrap();
        let prefix = d.join("out/r").to_string_lossy().into_owned();
        let (rc, body) = scan_one(&d.to_string_lossy(), ScanOpts {
            prefix, want_json: true, include_tests: false, policy: None, baseline: None,
            ws_member: false, quiet: true, deps_idx: &DepIndex::default(), peek_excluded: false,
        }, &crate::gate::begin_run());
        assert_eq!(rc, 0);
        let v: serde_json::Value = serde_json::from_str(&body.unwrap()).unwrap();
        let entries = v["functions"].as_array().cloned().unwrap_or_default();
        assert_eq!(entries.len(), 24, "the fixture must actually produce 24 entries:\n{v:#}");
        let mut checked = 0;
        for e in &entries {
            let qual = e["fn"].as_str().expect("fn");
            let loc = e["loc"].as_str().unwrap_or_else(|| panic!("{qual} has no loc:\n{v:#}"));
            let (rel, rest) = loc.rsplit_once(':').and_then(|(a, _col)| a.rsplit_once(':').map(|(f, l)| (f.to_string(), l.to_string())))
                .unwrap_or_else(|| panic!("malformed loc {loc:?}"));
            let line: usize = rest.parse().unwrap_or_else(|_| panic!("malformed loc {loc:?}"));
            let text = std::fs::read_to_string(d.join(&rel))
                .unwrap_or_else(|_| panic!("{qual}'s loc names a file that does not exist: {loc} — a span \
                                            resolved against ANOTHER file's map"));
            let lines: Vec<&str> = text.lines().collect();
            assert!(line <= lines.len(),
                    "{qual}'s loc is past the end of the file it names ({loc}, {} lines) — a span \
                     resolved against another file's map", lines.len());
            let leaf = qual.rsplit("::").next().unwrap_or(qual);
            let window = lines[line - 1..].iter().take(4).copied().collect::<Vec<_>>().join("\n");
            assert!(window.contains(&format!("fn {leaf}")),
                    "{qual}'s loc {loc} does not declare it — the line reads {:?}", lines[line - 1]);
            checked += 1;
        }
        assert_eq!(checked, 24, "every entry must have been checked, not skipped");
        let _ = std::fs::remove_dir_all(&d);
    }

    /// The same thread-boundary question asked of the OTHER parser candor runs on moved tokens:
    /// `cfg_eval`/`is_cfg_test` call `syn`'s `parse_nested_meta` on an attribute's tokens. A cfg
    /// predicate has no position that admits a leading `-`, so `parse_negative_lit` should never be
    /// reached — but that is an argument, and this is the fixture that settles it.
    #[test]
    fn a_cfg_attribute_reparse_survives_the_ast_crossing_a_thread_boundary() {
        const SRC: &str = "#[cfg(all(test, feature = -1))]\nmod m {\n    pub fn f() { let _ = std::fs::read(\"/x\"); }\n}\n";
        let parsed = std::thread::spawn(|| SendFile(syn::parse_file(SRC).unwrap())).join().unwrap();
        let saw = std::thread::spawn(move || {
            let parsed = parsed;
            parsed.0.items.iter().any(|it| match it {
                syn::Item::Mod(m) => is_cfg_test(&m.attrs),
                _ => false,
            })
        })
        .join()
        .expect("cfg evaluation must survive an AST parsed on another thread");
        assert!(saw, "the `test` predicate must still be seen");
    }

    /// A LOCAL `let`'s type annotation can NAME A GENERIC (`let d: T = pick();` under `fn f<T: Doer>`),
    /// and Pass B is the only place that annotation is read — but the collector had no copy of the
    /// signature's bound map, so `trait_leaves` was called with an EMPTY one and the bound arm could
    /// never fire. Inert, and therefore "correct by accident": nothing was wrong with the resolution
    /// rule, the question was simply never asked. The PARAMETER form of every row below already
    /// resolved, which is what makes this a gap rather than a policy.
    ///
    /// Each row carries its own control, because a row failing for a DIFFERENT reason would look the
    /// same: the `dyn` spelling of the same position (measured to resolve before this change), and a
    /// PURE-only trait that must NOT gain an effect.
    #[test]
    fn a_let_annotation_naming_a_generic_asks_the_signature_for_its_bound() {
        let v = scan_src_to_json("letbound", concat!(
            "use std::collections::HashMap;\n",
            "pub trait Doer { fn go(&self); }\n",
            "pub struct Fsy;\n",
            "impl Doer for Fsy { fn go(&self) { let _ = std::fs::read(\"/etc/x\"); } }\n",
            // A trait whose every local impl is PURE — the anti-fabrication direction. Widening the
            // annotation must not invent an effect where the CHA finds none.
            "pub trait Quiet { fn q(&self); }\n",
            "pub struct Q1;\n",
            "impl Quiet for Q1 { fn q(&self) {} }\n",
            // The rows this change closes.
            "pub fn let_scalar<T: Doer>(x: T) { let d: T = x; d.go() }\n",
            "pub fn let_vec<T: Doer>(xs: Vec<T>) { let v: Vec<T> = xs; for d in v { d.go() } }\n",
            "pub fn let_map<T: Doer>(m: HashMap<String, T>) { let q: HashMap<String, T> = m; for d in q.values() { d.go() } }\n",
            // The controls: the same POSITION with a `dyn` spelling, which resolved before the change.
            "pub fn ctl_scalar_dyn(x: Box<dyn Doer>) { let d: Box<dyn Doer> = x; d.go() }\n",
            "pub fn ctl_param_scalar<T: Doer>(d: T) { d.go() }\n",
            // The anti-fabrication row.
            "pub fn let_pure_bound<T: Quiet>(x: T) { let d: T = x; d.q() }\n",
        ));
        for f in ["let_scalar", "let_vec", "let_map", "ctl_scalar_dyn", "ctl_param_scalar"] {
            assert!(effs(fn_entry(&v, f)).contains(&"Fs".to_string()), "{f} must reach Fs:\n{v:#}");
        }
        assert!(
            v["functions"].as_array().unwrap().iter().all(|f| f["fn"] != "let_pure_bound"),
            "a bound whose local impls are all pure must gain nothing — this is the fabrication \
             direction, and the row above cannot see it:\n{v:#}"
        );
    }

    /// The tuple half, and it needed a second fix that only surfaced once the first one worked.
    /// `tuple_types` yields a position's SPELLING (`"T"`) while `tuple_trait_leaves` yields its BOUND,
    /// and the destructure wrote BOTH — `vars` and `trait_vars` — with `vars` winning at the call site.
    /// So `d` resolved to a type named `T`, which is nothing, and read silent-pure; the `dyn` spelling
    /// escaped only because `tuple_types` yields `None` for it. That is standing-bar 0b exactly: the
    /// annotation gap was hiding a PARAMETER-position gap underneath it (`ctl_param_tuple` below fails
    /// too, with no annotation involved).
    #[test]
    fn a_tuple_position_with_dispatch_leaves_is_not_shadowed_by_its_spelling() {
        let v = scan_src_to_json("lettuple", concat!(
            "pub trait Doer { fn go(&self); }\n",
            "pub struct Fsy;\n",
            "impl Doer for Fsy { fn go(&self) { let _ = std::fs::read(\"/etc/x\"); } }\n",
            "pub struct Conc;\n",
            "impl Conc { pub fn c(&self) { let _ = std::net::TcpStream::connect(\"h:1\"); } }\n",
            "pub fn let_tuple_bound<T: Doer>(t: (T, u32)) { let p: (T, u32) = t; let (d, _n) = p; d.go() }\n",
            "pub fn ctl_param_tuple<T: Doer>(t: (T, u32)) { let (d, _n) = t; d.go() }\n",
            "pub fn ctl_tuple_dyn(t: (Box<dyn Doer>, u32)) { let (d, _n) = t; d.go() }\n",
            // THE OTHER DIRECTION for the shadow fix: a CONCRETE tuple position has no dispatch leaves,
            // so it must still take the `vars` route and resolve its own type's method.
            "pub fn ctl_concrete_tuple(t: (Conc, u32)) { let (c, _n) = t; c.c() }\n",
        ));
        for f in ["let_tuple_bound", "ctl_param_tuple", "ctl_tuple_dyn"] {
            assert!(effs(fn_entry(&v, f)).contains(&"Fs".to_string()), "{f} must reach Fs:\n{v:#}");
        }
        assert!(
            effs(fn_entry(&v, "ctl_concrete_tuple")).contains(&"Net".to_string()),
            "the concrete tuple position must keep resolving through `vars`:\n{v:#}"
        );
    }

    /// The gain this rung produces on real code is a DISCLOSURE, not an effect, and it is the CHA
    /// fan-out bound (≤12 local impls, else honest `Unknown`) reaching a receiver it could not see
    /// before — not new behaviour. The parameter form is the control: it reads `Unknown` in both arms.
    /// (Traced on ebman `lint::default_rules`, 19 `impl Rule`, absent-from-report → `Unknown`.)
    #[test]
    fn a_wide_local_trait_reached_through_a_let_annotation_discloses_unknown() {
        let mut src = String::from("pub trait Wide { fn w(&self); }\n");
        for i in 0..13 {
            src.push_str(&format!("pub struct W{i};\nimpl Wide for W{i} {{ fn w(&self) {{}} }}\n"));
        }
        src.push_str("pub fn param_wide(xs: Vec<Box<dyn Wide>>) { for x in xs { x.w() } }\n");
        src.push_str("pub fn let_wide(xs: Vec<Box<dyn Wide>>) { let v: Vec<Box<dyn Wide>> = xs; for x in v { x.w() } }\n");
        let v = scan_src_to_json("letwide", &src);
        for f in ["param_wide", "let_wide"] {
            assert!(
                effs(fn_entry(&v, f)).contains(&"Unknown".to_string()),
                "{f} must DISCLOSE rather than read pure — 13 impls exceeds the 12-impl CHA bound:\n{v:#}"
            );
        }
    }

    /// The RESIDUALS, pinned so they cannot be mistaken for closed. Each was measured with its `dyn`
    /// control, and in every case the control is silent too — so these are POSITION-level gaps (the
    /// position resolves nothing at all) rather than the "never asks for the bound" gap this rung
    /// closed. Recording them as a test rather than a comment because a comment claiming a
    /// justification is an assertion; if one of these starts resolving, this test says so.
    ///
    /// R88 UPDATE: `(c)`, the FACTORY-return case (`use_ret_dyn`), is CLOSED — the bare unannotated
    /// `let` now routes through `resolve_recv_traits`, which already had a `Call` arm decoding a
    /// factory's `<dyn>` return sentinel (it was simply never reached from this binder). Moved to its
    /// own closed-set assertion below, per this test's own instruction. `(a)` (tuple INDEX access,
    /// `t.0.go()` — a `Field`/`Unnamed`-member access on a raw tuple VALUE, unrelated to `Expr::Index`)
    /// and `(b)` (an unannotated rebind of a COLLECTION dropping the source's ELEMENT dispatch leaves,
    /// `elem_trait_of` — a different table from the one this fix populates) are UNCHANGED residuals:
    /// R88 only fixed the bare `let` binding a SCALAR dispatch-typed expression to `trait_vars`, not
    /// these two — left unexamined this round, not closed.
    #[test]
    fn the_dispatch_positions_still_silent_are_silent_for_dyn_too() {
        let v = scan_src_to_json("letresid", concat!(
            "pub trait Doer { fn go(&self); }\n",
            "pub struct Fsy;\n",
            "impl Doer for Fsy { fn go(&self) { let _ = std::fs::read(\"/etc/x\"); } }\n",
            // (a) tuple INDEX access — `tuple_trait_of` is only consumed by a destructure pattern.
            "pub fn idx_bound<T: Doer>(t: (T, u32)) { t.0.go() }\n",
            "pub fn idx_dyn(t: (Box<dyn Doer>, u32)) { t.0.go() }\n",
            // (b) an UNANNOTATED rebind drops the source's dispatch leaves.
            "pub fn rebind_bound<T: Doer>(xs: Vec<T>) { let v = xs; for d in v { d.go() } }\n",
            "pub fn rebind_dyn(xs: Vec<Box<dyn Doer>>) { let v = xs; for d in v { d.go() } }\n",
            // (c) a FACTORY return bound into a local — CLOSED by R88, see the closed-set test below.
            "pub fn make_dyn() -> Box<dyn Doer> { Box::new(Fsy) }\n",
            "pub fn use_ret_dyn() { let d = make_dyn(); d.go() }\n",
        ));
        let present: Vec<&str> = v["functions"].as_array().unwrap().iter()
            .filter_map(|f| f["fn"].as_str()).collect();
        for f in ["idx_bound", "idx_dyn", "rebind_bound", "rebind_dyn"] {
            assert!(
                !present.contains(&f),
                "{f} now resolves — good news, but this residual note is stale: re-measure the \
                 position's `dyn` control and move the row into the closed set:\n{v:#}"
            );
        }
        assert!(
            effs(fn_entry(&v, "use_ret_dyn")).contains(&"Fs".to_string()),
            "R88: a bare `let d = make_dyn();` (make_dyn() -> Box<dyn Doer>) must now reach Fs through \
             d.go() — resolve_recv_traits's existing Call arm decoding the factory's <dyn> return, \
             reached from the bare-let binder for the first time:\n{v:#}"
        );
    }
    /// R88 — the bare unannotated `let` binding a SCALAR dispatch-typed RHS to `trait_vars`, the exact
    /// gap SOUNDNESS.md's R88 entry describes: every sibling binder (if-let, while-let, match-arm,
    /// for-loop, let-else, annotated `let`, tuple destructure) resolved dispatch leaves for its RHS —
    /// this was the one that never asked. `run_bound_field`/`run_direct` mirror the coordinator's own
    /// ground-truth repro; `run_indexed` pins the compounding `Expr::Index` gap in `resolve_recv_traits`
    /// (`self.handlers[0].go()`) fixed in the same commit. `run_direct` and the for-loop positive
    /// control must never regress — they are the "the dispatch machinery is sound" half of the claim.
    #[test]
    fn r88_bare_let_binds_a_scalar_dispatch_receiver() {
        let v = scan_src_to_json("r88bare", concat!(
            "pub trait Doer { fn go(&self); }\n",
            "pub struct Fs;\n",
            "impl Doer for Fs { fn go(&self) { let _ = std::fs::read(\"/etc/x\"); } }\n",
            "pub struct Widget { single: Box<dyn Doer>, handlers: Vec<Box<dyn Doer>> }\n",
            "impl Widget {\n",
            "  pub fn run_bound_field(&self) { let h = &self.single; h.go(); }\n",
            "  pub fn run_direct(&self) { self.single.go(); }\n",
            "  pub fn run_indexed(&self) { self.handlers[0].go(); }\n",
            "  pub fn run_loop(&self) { for h in &self.handlers { h.go(); } }\n",
            "}\n",
        ));
        for f in ["Widget::run_bound_field", "Widget::run_direct", "Widget::run_indexed", "Widget::run_loop"] {
            assert!(
                effs(fn_entry(&v, f)).contains(&"Fs".to_string()),
                "{f} must reach Fs — same field, same dispatch type, only the binder shape differs:\n{v:#}"
            );
        }
    }

    /// R89 — a LOCAL TRAIT method passed as a first-class VALUE to an invoking adapter
    /// (`items.iter().for_each(Doer::go)` where `Doer::go` is an ABSTRACT requirement, no default body)
    /// must dispatch through the SAME bounded CHA a `.method()` call on a dispatch-typed receiver already
    /// uses (`dispatch_calls_for_trait_method`) — before this fix the site pushed a literal
    /// `Call{path:"Doer::go"}` unconditionally, which matched no declaration (decls.rs never records an
    /// abstract trait method as a unit) and evaporated: no `Unknown`, no effect. `call_concrete` is the
    /// control the pre-fix comment's claim was actually true for (`for_each(Conn::send)`-shaped) and must
    /// keep resolving unchanged; `call_wide` pins the >12-impl honest-Unknown fan-out bound through this
    /// same new path, not just the narrow (<=12) edge case.
    #[test]
    fn r89_trait_method_passed_as_a_value_dispatches() {
        let v = scan_src_to_json("r89val", concat!(
            "pub trait Doer { fn go(&self); }\n",
            "pub struct Fsy;\n",
            "impl Doer for Fsy { fn go(&self) { let _ = std::fs::read(\"/etc/x\"); } }\n",
            "pub fn call_it(items: Vec<Box<dyn Doer>>) { items.iter().for_each(Doer::go); }\n",
            // Control: a CONCRETE type's associated fn passed as a value must keep resolving exactly as
            // before — this path is unchanged for anything whose head isn't a local trait.
            "pub struct Conn;\n",
            "impl Conn { pub fn send(&self) { let _ = std::fs::write(\"/etc/y\", \"w\"); } }\n",
            "pub fn call_concrete(items: Vec<Conn>) { items.iter().for_each(Conn::send); }\n",
        ));
        assert!(
            effs(fn_entry(&v, "call_it")).contains(&"Fs".to_string()),
            "R89: Doer::go passed as a value to for_each must dispatch to Fsy::go's Fs:\n{v:#}"
        );
        assert!(
            effs(fn_entry(&v, "call_concrete")).contains(&"Fs".to_string()),
            "control regressed: a concrete type's method passed as a value must still resolve:\n{v:#}"
        );
    }

    /// R88 SELF-SHADOW REGRESSION GUARD — measured on mysql_async 0.37.0's real source in the 256-crate
    /// A/B, minimised. `impl<Q: AsQuery> Query for Q { fn run(self, conn: C) -> .. { let mut conn =
    /// conn.to_connection().resolve().await?; conn.as_mut().raw_query(..); .. } }` — a BARE `let` whose
    /// name (`conn`) SHADOWS a generic-bound PARAMETER of the same name, and whose own RHS (`conn.
    /// to_connection()`) still refers to the OUTER (parameter) binding. R88's fix originally cleared
    /// `trait_vars[name]` EAGERLY, before the trailing `syn::visit::visit_local` walk revisited this
    /// SAME statement's RHS — so the eager clear poisoned the outer `conn`'s own dispatch resolution
    /// inside its own defining expression: `run` went from a genuine, honest `Unknown` (an opaque
    /// bounded-trait dispatch the scan correctly can't resolve further) to COMPLETELY ABSENT from
    /// `functions[]` — zero calls, zero unresolved, a fabricated false-clean silent under-report. Fixed
    /// by deferring the `trait_vars` mutation (`r88_pending_trait_vars`) until AFTER the trailing walk.
    /// `run_no_shadow` (a DIFFERENT name on the rebind) is the control: it must read identically to
    /// `run_shadow`, proving the shadow itself carries no information the scan should lose.
    #[test]
    fn r88_self_shadowing_rebind_does_not_lose_the_outer_bindings_dispatch() {
        // The reproducer needs the EXACT mysql_async ingredient: a trait implemented for a REFERENCE
        // type (`impl<'a> ToConnection<'a> for &'a Pool`) inside a BLANKET trait impl (`impl<Q> Query for
        // Q`) — an unrelated, pre-existing engine behaviour (present before AND after R88, confirmed by
        // running the pre-fix binary) that happens to read the bounded dispatch as `Unknown` rather than
        // resolving it to a concrete edge. This test does not depend on WHY that pre-existing behaviour
        // discloses `Unknown` — only that R88 must not turn that honest disclosure into total silence.
        let src = concat!(
            "pub trait ToConnection<'a> { fn to_connection(self) -> ConnLike<'a>; }\n",
            "pub struct Pool;\n",
            "impl<'a> ToConnection<'a> for &'a Pool { fn to_connection(self) -> ConnLike<'a> { ConnLike(std::marker::PhantomData) } }\n",
            "pub struct ConnLike<'a>(std::marker::PhantomData<&'a ()>);\n",
            "impl<'a> ConnLike<'a> { pub fn resolve(self) -> Result<Conn, ()> { Ok(Conn) } }\n",
            "pub struct Conn;\n",
            "impl Conn { fn raw_query(&mut self) {} }\n",
            "pub trait Query: Sized { fn run<'a, C: ToConnection<'a>>(self, conn: C) -> Result<(), ()>; }\n",
            "impl<Q> Query for Q { fn run<'a, C: ToConnection<'a>>(self, conn: C) -> Result<(), ()> { \
                 let mut conn = conn.to_connection().resolve()?; conn.raw_query(); Ok(()) } }\n",
        );
        let unres = unresolved_of(src);
        assert_eq!(
            unres.get("Q::run"), Some(&true),
            "R88 self-shadow regression: `Q::run`'s self-shadowing `let mut conn = conn.to_connection()\
             .resolve()?;` must stay in the report with its pre-existing honest `unresolved` disclosure, \
             not vanish entirely (zero calls, zero unresolved) — the rebind's OWN RHS must resolve \
             against the state BEFORE the rebind takes effect, not after: {unres:?}"
        );
    }

    /// R92 SELF-SHADOW REGRESSION GUARD — SOUNDNESS.md R92, the let-else twin of R88's bare-`let` fix
    /// above. `visit_local`'s let-else branches (tuple-variant, struct-variant, and the `Some`/`Ok` form)
    /// mutated `self.vars`/`self.trait_vars` for the newly-bound name BEFORE the trailing
    /// `syn::visit::visit_local` re-walked that same statement's own RHS — so when the binding SHADOWS
    /// the name it is initialised from, the RHS's own call resolved against the NEW binding's leaves
    /// instead of the OUTER one, and the outer effect vanished with `unresolved` never set. `produce`
    /// (`Env`) and `go` (`Fs`) are deliberately DIFFERENT effect classes — an earlier fixture used the
    /// same class for both and the union hid the loss entirely; a fixture whose instrument can't
    /// distinguish the sin from correct behaviour answers nothing. Fixed by deferring the mutation into
    /// `pending_bindings`, reusing R88's mechanism rather than adding a third ordering rule.
    ///
    /// Each variant below pairs a self-SHADOWING function with a non-shadowing CONTROL of identical
    /// shape (distinct binder name) — the control must read identically regardless of the fix, proving
    /// the shadow itself, not the binder shape, is what the bug turned on.
    #[test]
    fn r92_letelse_tuple_variant_self_shadow_keeps_producer_effect() {
        let v = scan_src_to_json("r92tuple", concat!(
            "pub trait Doer { fn go(&self); }\n",
            "pub struct W;\n",
            "impl Doer for W { fn go(&self) { let _ = std::fs::write(\"/tmp/x\", \"w\"); } }\n",
            "pub enum Msg { Cb(Box<dyn Doer>) }\n",
            "pub trait Producer { fn produce(&self) -> Msg; }\n",
            "pub struct P;\n",
            "impl Producer for P { fn produce(&self) -> Msg { let _ = std::env::var(\"HOME\"); \
                 Msg::Cb(Box::new(W)) } }\n",
            "pub fn shadowed(f: Box<dyn Producer>) { let Msg::Cb(f) = f.produce() else { return }; f.go(); }\n",
            "pub fn control(p: Box<dyn Producer>) { let Msg::Cb(g) = p.produce() else { return }; g.go(); }\n",
        ));
        let shadowed = effs(fn_entry(&v, "shadowed"));
        assert!(
            shadowed.contains(&"Env".to_string()) && shadowed.contains(&"Fs".to_string()),
            "R92 tuple-variant let-else self-shadow: `shadowed` must keep BOTH the producer's Env and \
             the worker's Fs — pre-fix this read [\"Fs\"] only, Env silently lost:\n{v:#}"
        );
        let control = effs(fn_entry(&v, "control"));
        assert!(
            control.contains(&"Env".to_string()) && control.contains(&"Fs".to_string()),
            "control regressed: a non-shadowing let-else binding must be unaffected by the fix:\n{v:#}"
        );
    }

    #[test]
    fn r92_letelse_struct_variant_self_shadow_keeps_producer_effect() {
        let v = scan_src_to_json("r92struct", concat!(
            "pub trait Doer { fn go(&self); }\n",
            "pub struct W;\n",
            "impl Doer for W { fn go(&self) { let _ = std::fs::write(\"/tmp/x\", \"w\"); } }\n",
            "pub enum Msg { CbField { f: Box<dyn Doer> } }\n",
            "pub trait Producer { fn produce(&self) -> Msg; }\n",
            "pub struct P;\n",
            "impl Producer for P { fn produce(&self) -> Msg { let _ = std::env::var(\"HOME\"); \
                 Msg::CbField { f: Box::new(W) } } }\n",
            "pub fn shadowed(f: Box<dyn Producer>) { \
                 let Msg::CbField { f } = f.produce() else { return }; f.go(); }\n",
            "pub fn control(p: Box<dyn Producer>) { \
                 let Msg::CbField { f: g } = p.produce() else { return }; g.go(); }\n",
        ));
        let shadowed = effs(fn_entry(&v, "shadowed"));
        assert!(
            shadowed.contains(&"Env".to_string()) && shadowed.contains(&"Fs".to_string()),
            "R92 struct-variant let-else self-shadow: `shadowed` must keep BOTH the producer's Env and \
             the worker's Fs — pre-fix this read [\"Fs\"] only, Env silently lost:\n{v:#}"
        );
        let control = effs(fn_entry(&v, "control"));
        assert!(
            control.contains(&"Env".to_string()) && control.contains(&"Fs".to_string()),
            "control regressed: a non-shadowing struct-variant let-else binding must be unaffected by \
             the fix:\n{v:#}"
        );
    }

    /// R92 EXTENSION — found while auditing R92's stated boundary (SOUNDNESS.md scoped the confirmed
    /// bug to the two enum-variant let-else branches): the THIRD let-else branch, `Some`/`Ok` unwrap
    /// (`some_ok_binding`), mutated `trait_vars` immediately in exactly the same shape and was equally
    /// wrong for a self-shadowing bind — measured via the pre-fix binary before this test existed.
    #[test]
    fn r92_letelse_some_ok_self_shadow_keeps_producer_effect() {
        let v = scan_src_to_json("r92opt", concat!(
            "pub trait Doer { fn go(&self); }\n",
            "pub struct W;\n",
            "impl Doer for W { fn go(&self) { let _ = std::fs::write(\"/tmp/x\", \"w\"); } }\n",
            "pub trait Producer { fn maybe_produce(&self) -> Option<Box<dyn Doer>>; }\n",
            "pub struct P;\n",
            "impl Producer for P { fn maybe_produce(&self) -> Option<Box<dyn Doer>> { \
                 let _ = std::env::var(\"HOME\"); Some(Box::new(W)) } }\n",
            "pub fn shadowed(f: Box<dyn Producer>) { \
                 let Some(f) = f.maybe_produce() else { return }; f.go(); }\n",
            "pub fn control(p: Box<dyn Producer>) { \
                 let Some(g) = p.maybe_produce() else { return }; g.go(); }\n",
        ));
        let shadowed = effs(fn_entry(&v, "shadowed"));
        assert!(
            shadowed.contains(&"Env".to_string()) && shadowed.contains(&"Fs".to_string()),
            "R92 (Some/Ok let-else, unflagged in the original SOUNDNESS entry) self-shadow: `shadowed` \
             must keep BOTH the producer's Env and the worker's Fs — pre-fix this read [\"Fs\"] only, \
             Env silently lost:\n{v:#}"
        );
        let control = effs(fn_entry(&v, "control"));
        assert!(
            control.contains(&"Env".to_string()) && control.contains(&"Fs".to_string()),
            "control regressed: a non-shadowing Some/Ok let-else binding must be unaffected by the fix:\n{v:#}"
        );
    }

    /// R92 CONTROL — a let-else binding a CONCRETE (non-dispatch) payload under a NON-shadowing name
    /// must stay exactly as pure as it was before the fix; this exercises the deferred `concrete_ty`
    /// half of `pending_bindings` (the `vars.insert` alternative to `trait_vars.insert`) on the ordinary,
    /// no-shadow path.
    #[test]
    fn r92_letelse_concrete_payload_non_shadow_stays_pure() {
        let v = scan_src_to_json("r92concrete", concat!(
            "pub enum Data { Num(i32) }\n",
            "fn make_data() -> Data { Data::Num(3) }\n",
            "pub fn concrete_control() -> i32 { let Data::Num(n) = make_data() else { return 0 }; n + 1 }\n",
        ));
        assert!(
            v["functions"].as_array().unwrap().iter().all(|f| f["fn"] != "concrete_control"),
            "control regressed: a non-shadowing, non-dispatch let-else binding must stay pure (absent \
             from functions[], same as before the fix):\n{v:#}"
        );
    }

    /// The bound map must be SCOPED across a nested `fn`/`impl`, for the same reason `dyn_sig_traits`
    /// is (R4 / value-bag): a nested item's calls are attributed to the ENCLOSING unit, so a nested
    /// `let d: T` would otherwise read the OUTER signature's `T` and charge the outer bound's
    /// implementors to a function that never touches them. Distinguishing fixture — the two `T`s are
    /// bound to different traits with the same method name, and only the outer one is effectful.
    #[test]
    fn the_bound_map_does_not_follow_the_walk_into_a_nested_item() {
        let v = scan_src_to_json("letnested", concat!(
            "pub trait Doer { fn go(&self); }\n",
            "pub struct Fsy;\n",
            "impl Doer for Fsy { fn go(&self) { let _ = std::fs::read(\"/etc/x\"); } }\n",
            "pub trait Quiet { fn go(&self); }\n",
            "pub struct Q1;\n",
            "impl Quiet for Q1 { fn go(&self) {} }\n",
            "pub fn holder<T: Doer>(_x: T) {\n",
            "    fn nested<T: Quiet>(y: T) { let d: T = y; d.go(); }\n",
            "    nested(Q1);\n",
            "}\n",
        ));
        assert!(
            !effs(&v["functions"].as_array().unwrap().iter()
                .find(|f| f["fn"] == "holder").cloned().unwrap_or(serde_json::json!({"inferred": []})))
                .contains(&"Fs".to_string()),
            "the nested item's `T` inherited the ENCLOSING signature's bound — the R4 shadowing \
             hazard, arriving through the annotation instead of the parameter:\n{v:#}"
        );
    }

    /// …AND THE SECOND FIXTURE, which is the one that was missing. Scoping the bound map across a
    /// nested item (the test above) was written with `std::mem::take`, so the nested walk ran with an
    /// EMPTY map and the NESTED signature's own bounds were never installed. A shadow alone is the
    /// mirror sin: `fn inner<T: Doer>(d: T) { let x: T = d; x.go() }` inside any enclosing fn resolved
    /// `T` to nothing and the ENCLOSING unit — which is where a nested item's calls are attributed —
    /// was ABSENT from the report, i.e. a purity claim over a call that reads the filesystem.
    ///
    /// Every row carries its `dyn` CONTROL, resolving in BOTH arms, so a silent row that is silent for
    /// a different reason cannot be mistaken for this one:
    ///   - a nested `fn`'s own `<T: Doer>`;
    ///   - a nested `impl`'s METHOD-level `<T: Doer>` (not in the impl block's `Generics` at all, so
    ///     `visit_item_impl` alone leaves this row silent);
    ///   - a nested `impl` BLOCK's `<T: Doer>` (not in the method's signature, the other way round).
    ///
    /// The fabrication direction is `the_bound_map_does_not_follow_the_walk_into_a_nested_item` above,
    /// which must keep passing unchanged — together they say the map is SCOPED, not cleared.
    #[test]
    fn a_nested_items_own_generic_bound_resolves() {
        let v = scan_src_to_json("nestedown", concat!(
            "pub trait Doer { fn go(&self); }\n",
            "pub struct Fsy;\n",
            "impl Doer for Fsy { fn go(&self) { let _ = std::fs::read(\"/etc/x\"); } }\n",
            // a nested `fn`'s own bound, + its `dyn` control
            "pub fn nested_fn_bound() { fn inner<T: Doer>(d: T) { let x: T = d; x.go() } inner(Fsy); }\n",
            "pub fn nested_fn_dyn() { fn inner(d: Box<dyn Doer>) { let x: Box<dyn Doer> = d; x.go() } inner(Box::new(Fsy)); }\n",
            // a nested impl METHOD's own bound, + its `dyn` control
            "pub fn nested_method_bound() { struct N; impl N { fn m<T: Doer>(&self, d: T) { let x: T = d; x.go() } } N.m(Fsy); }\n",
            "pub fn nested_method_dyn() { struct M; impl M { fn m(&self, d: Box<dyn Doer>) { let x: Box<dyn Doer> = d; x.go() } } M.m(Box::new(Fsy)); }\n",
            // a nested impl BLOCK's bound
            "pub fn nested_block_bound() { struct W<T>(T); impl<T: Doer> W<T> { fn m(&self, d: T) { let x: T = d; x.go() } } W(Fsy).m(Fsy); }\n",
        ));
        for f in ["nested_fn_bound", "nested_method_bound", "nested_block_bound"] {
            assert!(
                effs(fn_entry(&v, f)).contains(&"Fs".to_string()),
                "{f}: the NESTED signature's own bound was never installed — the enclosing unit reads \
                 silent-pure over a call that reads the filesystem:\n{v:#}"
            );
        }
        for f in ["nested_fn_dyn", "nested_method_dyn"] {
            assert!(
                effs(fn_entry(&v, f)).contains(&"Fs".to_string()),
                "{f} is the CONTROL and must resolve in both arms — if it does not, the position is \
                 dead and the rows above prove nothing:\n{v:#}"
            );
        }
    }

    /// REPLACE, not merge — and this test exists because the merge MUTANT passed the entire suite,
    /// which made the "replace" comment an assertion nothing checked (standing-bar item 9).
    ///
    /// The reason no existing fixture could tell them apart is that rustc will not let one exist: a
    /// nested item may not name the enclosing fn's generics (**E0401**, verified by compiling the source
    /// below — `let d: T` is rejected, it does NOT fall back to the struct `T`), so the only keys the
    /// two spellings disagree about are names no legal nested body can mention. Merging is therefore
    /// safe *for code that compiles*, and that is exactly the dependency worth not having: candor-scan
    /// analyses crates WITHOUT building them, so it routinely walks bodies rustc never accepted —
    /// `#[cfg]`-gated branches, macro-shaped input, a file mid-edit. This fixture is deliberately such
    /// a body, and under the merge mutant the outer `<T: Doer>` reaches the nested `let d: T` and
    /// charges `holder` a filesystem read it does not perform.
    #[test]
    fn an_outer_bound_does_not_reach_a_nested_item_that_never_declared_it() {
        let v = scan_src_to_json("nestedmerge", concat!(
            "pub trait Doer { fn go(&self); }\n",
            "pub struct Fsy;\n",
            "impl Doer for Fsy { fn go(&self) { let _ = std::fs::read(\"/etc/x\"); } }\n",
            // a CONCRETE type whose name collides with the enclosing signature's generic, and whose
            // `go` is pure.
            "pub struct T;\n",
            "impl T { pub fn go(&self) {} }\n",
            "pub fn holder<T: Doer>(_x: T) {\n",
            "    fn nested() { let d: T = T; d.go(); }\n",
            "    nested();\n",
            "}\n",
        ));
        let present: Vec<&str> = v["functions"].as_array().unwrap().iter()
            .filter_map(|f| f["fn"].as_str()).collect();
        assert!(
            !present.contains(&"holder"),
            "the ENCLOSING signature's bound reached a nested item that never declared the name — a \
             fabricated effect on a body whose `T` is a pure local type:\n{v:#}"
        );
    }

    /// RESIDUAL, pinned so it cannot drift: a nested item's PARAMETERS are not typed at all. Both
    /// spellings are silent — the `<T: Doer>` bound AND the `&dyn Doer` control — which is what makes
    /// this a POSITION-level gap rather than the "never asks for the bound" defect above; the
    /// collector's `vars`/`trait_vars` are seeded once from the enclosing signature and no visitor
    /// binds a nested signature's parameters. It is also why the erasure/provenance maps are left
    /// cleared for a nested item rather than re-installed (see `visit_item_impl`'s note): they have
    /// nothing to bind to until this is closed. candor-swift records the mirror residual for its own
    /// `vars`/`arrayElem` in `83cd607`.
    #[test]
    fn a_nested_items_parameters_are_still_untyped_in_every_spelling() {
        let v = scan_src_to_json("nestedparam", concat!(
            "pub trait Doer { fn go(&self); }\n",
            "pub struct Fsy;\n",
            "impl Doer for Fsy { fn go(&self) { let _ = std::fs::read(\"/etc/x\"); } }\n",
            "pub fn param_bound() { fn inner<T: Doer>(d: T) { d.go() } inner(Fsy); }\n",
            "pub fn param_dyn() { fn inner(d: &dyn Doer) { d.go() } inner(&Fsy); }\n",
            "pub fn top_control<T: Doer>(d: T) { d.go() }\n",
        ));
        assert!(
            effs(fn_entry(&v, "top_control")).contains(&"Fs".to_string()),
            "the TOP-LEVEL parameter control stopped resolving — this residual note is measuring \
             nothing:\n{v:#}"
        );
        let present: Vec<&str> = v["functions"].as_array().unwrap().iter()
            .filter_map(|f| f["fn"].as_str()).collect();
        for f in ["param_bound", "param_dyn"] {
            assert!(
                !present.contains(&f),
                "{f} now resolves — good news, but this residual is stale: a nested item's parameters \
                 are being typed now, so re-measure whether `visit_item_impl`'s cleared erasure and \
                 provenance maps should be installed from the nested signature too:\n{v:#}"
            );
        }
    }

    /// The same "never asks" defect on the CALLABLE arm of the annotation: `let g: F = f;` under
    /// `<F: Fn()>` left `g` un-flagged, so invoking it read silent-pure while the identical PARAMETER
    /// position already disclosed `Unknown`. Honest beats silent, and the parameter row is the control.
    #[test]
    fn a_let_annotation_naming_a_callable_generic_still_discloses_unknown() {
        let v = scan_src_to_json("letcallable", concat!(
            "pub fn param_fn<F: Fn()>(f: F) { f() }\n",
            "pub fn let_fn<F: Fn()>(f: F) { let g: F = f; g() }\n",
        ));
        for f in ["param_fn", "let_fn"] {
            assert!(
                effs(fn_entry(&v, f)).contains(&"Unknown".to_string()),
                "{f} invokes an opaque callable and must disclose it:\n{v:#}"
            );
        }
    
    }
    #[test]
    fn an_identical_entry_restated_does_not_withdraw_the_key() {
        // A dep directory holding TWO COPIES of one report is the most ordinary accident there is, and it
        // used to be a CARDINAL SIN: the never-guess rule saw a shared key, withdrew it, and the consumer
        // went ABSENT from `functions` — a positive purity claim under ⟨0.21⟩ — over a call the single-report
        // arm resolves to `Exec`. The rule is about DISAGREEMENT; two entries making the SAME claim are not
        // ambiguous. Found by candor-swift's fresh-vs-stale fixture, which flagged it for rust and java;
        // java is clean (last-wins keeps an answer), rust withdrew.
        let dep = std::env::temp_dir().join(format!("candor-dupkey-rep-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dep);
        std::fs::create_dir_all(&dep).unwrap();
        let me = format!("scan-{}", env!("CARGO_PKG_VERSION"));
        let body = format!(r#"{{
            "candor": {{"version": "{me}", "toolchain": "stable", "spec": "0.23"}},
            "package": "deplib",
            "functions": [{{"fn": "work", "inferred": ["Exec"], "cmds": ["dupcmd"], "hash": "deplib#work"}}]}}"#);
        // the SAME report, twice — byte-identical, zero disagreement
        std::fs::write(dep.join("report.deplib.scan.json"), &body).unwrap();
        std::fs::write(dep.join("copy.deplib.scan.json"), &body).unwrap();
        let idx = load_dep_reports(Some(dep.to_str().unwrap()));
        let _ = std::fs::remove_dir_all(&dep);
        assert!(idx.by_key.contains_key("deplib#work"),
                "the key was WITHDRAWN because one claim was restated — the consumer then reads the call pure");
        assert_eq!(idx.by_key.get("deplib#work").map(|d| d.effects.clone()), Some(BTreeSet::from(["Exec"])),
                   "the surviving entry must be the claim both copies made");
    }

    #[test]
    fn two_entries_that_disagree_under_one_key_are_unioned() {
        // THE FAMILY-WIDE ENTRY-COLLISION RULE (candor-spec/ENTRY-COLLISION-DECISION.md): two entries
        // under one key are UNIONED — never withdrawn, never picked between.
        //
        // THIS TEST USED TO ASSERT THE OPPOSITE, and the reversal is the decision, not a regression. It
        // read `..._are_still_withdrawn` and defended withdrawal on the grounds that keeping either entry
        // would charge one dep function's effects to another. What that argument missed is the price:
        // withdrawal removes the key, so the CALLER drops out of `functions`, and under ⟨0.21⟩ an absent
        // entry is a positive claim of purity. It traded a precision loss for the cardinal sin.
        //
        // Measured across candor-rust/pgman/ebman, the union costs SEVEN effect-items in total and closes
        // 123 purity claims, and every measured disagreement is one function at two VERSIONS of one crate
        // (both bodies in the build, so the union is the correct answer rather than a hedge). The live
        // instance is `hyper#client::conn::http1::Builder::handshake` — `['Log']` @0.14.32 vs `[]` @1.9.0.
        let dep = std::env::temp_dir().join(format!("candor-dupkey-dis-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dep);
        std::fs::create_dir_all(&dep).unwrap();
        let me = format!("scan-{}", env!("CARGO_PKG_VERSION"));
        let one = format!(r#"{{"candor": {{"version": "{me}", "toolchain": "s", "spec": "0.23"}},
            "package": "deplib",
            "functions": [{{"fn": "work", "inferred": ["Exec"], "hash": "deplib#work"}}]}}"#);
        let two = format!(r#"{{"candor": {{"version": "{me}", "toolchain": "s", "spec": "0.23"}},
            "package": "deplib",
            "functions": [{{"fn": "work", "inferred": ["Net"], "hash": "deplib#work"}}]}}"#);
        std::fs::write(dep.join("a.deplib.scan.json"), one).unwrap();
        std::fs::write(dep.join("b.deplib.scan.json"), two).unwrap();
        let idx = load_dep_reports(Some(dep.to_str().unwrap()));
        let _ = std::fs::remove_dir_all(&dep);
        assert_eq!(idx.by_key.get("deplib#work").map(|d| d.effects.clone()),
                   Some(BTreeSet::from(["Exec", "Net"])),
                   "two entries claiming DIFFERENT effects under one key must UNION — withdrawing the key \
                    takes the CALLER out of `functions`, which under ⟨0.21⟩ is a purity claim over a call \
                    that reaches both");
    }

    /// THE ANCHOR COUNT IS A CLAIM ABOUT THE SOURCE, SO THE SOURCE IS WHAT ANSWERS IT.
    ///
    /// `dbab8be` gated coverage on a dependency report declaring itself incomplete, and argued the gate
    /// was complete on the grounds that rust's four registration sites (the envelope `package`, the
    /// JVM-shape `packages[]`, the filename fallback and each entry's `hash` prefix) all funnel through
    /// ONE `cover` closure and that coverage is consumed in exactly one place. That is the load-bearing
    /// claim of the whole rung — candor-java found coverage anchored TWICE, where gating one anchor is
    /// a no-op wearing a fix's clothes and its mutant fails NOTHING — and it was written as a comment.
    /// A comment is an assertion and it will be believed (standing bar item 9); this makes it FAIL.
    ///
    /// VERIFIED BY ENUMERATION when this test was written, and the comment was inexact in one place:
    /// `untrusted` and `incomplete_pkgs` are also READ by the two stderr disclosures in `load_dep_reports`,
    /// so "read nowhere else in the engine" was wrong — "CONSUMED nowhere else" is right, and the comment
    /// now says that. The substance held: three writes, all inside `cover`, and one `covered` predicate.
    ///
    /// ⟨0.24⟩ now FOUR writes and THREE refusal sets. The new one matters most here: a count-0 report
    /// reaches the entry loop with no entries, so its `hash`-prefix anchor never fires and gating that
    /// anchor alone would have been the exact no-op java measured. Only the closure gates all four.
    /// EVERY INCOMPLETE-ANALYSIS REFUSAL IS GATED ON HAVING NOTHING TO REPORT — a census, in the shape of
    /// `coverage_has_exactly_one_anchor_and_exactly_one_consumer` below and for the same reason.
    ///
    /// SPEC §3.3.1 has two clauses: *"A configured gate over incompletely-analyzed code MUST fail closed
    /// (exit ≠ 0); **a real violation (exit 1) still dominates.**"* A refusal written as bare
    /// `if had_parse_failure { return (2, …) }` implements the first and breaks the second, and the cost
    /// is not the exit code — on exit 2 the verdict document carries no violations, so the finding is
    /// **deleted from the artifact a CI consumer reads**.
    ///
    /// THIS ENGINE SHIPPED THAT DEFECT TWICE, AT TWO SITES, AND FIXED THEM A WEEK APART. The policy gate
    /// was corrected on 2026-07-28; the AS-EFF-005 baseline guard — thirty lines up the same function —
    /// kept the bare form until 2026-08-02, because the first fix wrote its entire reasoning into the
    /// policy gate's comment and never looked at the other copy. A comment cannot census its siblings.
    /// This can: the bare form is asserted to appear ZERO times, so a third gate added later cannot
    /// quietly reintroduce it.
    ///
    /// WHY EMPTINESS IS THE RIGHT CONJUNCT rather than "never refuse": a parse failure makes the scan see
    /// LESS, and these gates fire on effects PRESENT or GAINED, so less evidence can only MASK a violation,
    /// never manufacture one. A violation found beside unreadable source is therefore real and must be
    /// reported; a CLEAN gate over unreadable source is the false-pure clause 1 forbids. Both directions
    /// are live behaviour, pinned by `a_baseline_regression_beside_an_unparseable_file_still_reaches_the_verdict`
    /// and by conformance PART 29 four-way. This test guards the SHAPE, so the behaviour tests cannot be
    /// satisfied by a third site nobody wrote one for.
    #[test]
    fn every_incomplete_refusal_is_gated_on_having_nothing_to_report() {
        let scan = include_str!("scan.rs");
        let count = |hay: &str, needle: &str| hay.matches(needle).count();

        // THE BARE FORM IS THE DEFECT. `if had_parse_failure {` as a refusal condition returns before the
        // violations exist, which is exactly what both shipped defects did.
        assert_eq!(count(scan, "if had_parse_failure {"), 0,
                   "a refusal guarded by `had_parse_failure` ALONE returns before the violations are \
                    recorded — §3.3.1 says a real violation still dominates, and the document it writes \
                    carries `violations: []`. Conjoin it with the emptiness test, as both existing gates do");

        // …and the guarded form is what both gates use. Counted, not merely present: if a gate is deleted
        // this drops and the test says so rather than passing on the survivor.
        // ⟨0.30⟩ COUNT THE PREFIX, NOT THE WHOLE LINE. The guard may be STRONGER than the two-conjunct
        // form and one site now is: the policy gate also asks `guard_code != 1 && !holds_violation()`,
        // because `v` is that gate's OWN list and a violation recorded by the baseline producer was
        // invisible to it (measured — a regression plus one unparseable file plus a CLEAN policy exited 2
        // while the verdict carried the violation). Pinning the exact string would have made the correct
        // fix look like a deleted gate.
        assert_eq!(count(scan, "if had_parse_failure && v.is_empty()"), 2,
                   "expected exactly TWO incomplete-refusal sites — the policy gate and the AS-EFF-005 \
                    baseline guard. A third gate is fine, but it has to be added HERE too, which is the \
                    whole point of counting");

        // VACUITY FLOOR (standing bar item 8): if a rename makes the patterns unfindable, every assertion
        // above passes on nothing — including the `== 0`, which is the one that matters most and is also
        // the easiest to satisfy by accident.
        assert!(scan.contains("let mut had_parse_failure"),
                "this test located no `had_parse_failure` at all — it is asserting about source it can no \
                 longer find, and would go green through the very defect it exists to catch");
        assert!(count(scan, "had_parse_failure") >= 4,
                "the flag is set from several places (unparsed files, a failed read, the ⟨0.21⟩ gate) and \
                 read by two — fewer than four occurrences means this test is looking at the wrong thing");
    }

    #[test]
    fn an_unevaluable_target_refuses_and_leaves_no_report() {
        // ⟨0.31⟩ The walk admitted nothing this engine can read. That is a refusal (exit 2), not a clean
        // scan — this engine answered `policy ✓` at exit 0 here, a permanent green for a typo'd CI path,
        // while ts, swift and java all refused. It already refuses a target that does not EXIST for the
        // same stated reason; an existing path holding nothing readable is that green one step along.
        //
        // FOUR ROWS, because the two negatives are what three attempts at this fix got wrong:
        //   · zero readable files            -> 2, and NO report (§3.1 binds any report a scan produced)
        //   · a normal crate, nothing denied -> 0   (attempt #2 keyed on the analyzed accumulator and
        //                                            attempt #3 on a SHADOWED `paths`; both made this 2)
        //   · a normal crate with a violation-> 1   (a certain violation still dominates)
        //   · a workspace, one member empty  -> 0   (PER-INVOCATION: a scaffolded member must not redden
        //                                            a real workspace, and it still gets its count-0 report)
        let root = std::env::temp_dir().join(format!("candor-unevaluable-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let mk = |sub: &str, files: &[(&str, &str)]| {
            let d = root.join(sub);
            std::fs::create_dir_all(d.join("src")).unwrap();
            for (name, body) in files {
                let p = d.join(name);
                if let Some(par) = p.parent() { std::fs::create_dir_all(par).unwrap(); }
                std::fs::write(p, body).unwrap();
            }
            d
        };
        let manifest = "[package]\nname=\"t\"\nversion=\"0.0.0\"\nedition=\"2021\"\n";
        let pol = "deny Exec\n";
        let empty  = mk("empty",  &[("Cargo.toml", manifest), ("pol", pol), ("notes.txt", "hi")]);
        let normal = mk("normal", &[("Cargo.toml", manifest), ("pol", pol), ("src/lib.rs", "pub fn ok() -> i32 { 1 + 1 }\n")]);
        let bad    = mk("bad",    &[("Cargo.toml", manifest), ("pol", pol),
                                    ("src/lib.rs", "pub fn b() { let _ = std::process::Command::new(\"ls\").status(); }\n")]);
        let idx = crate::deps::DepIndex::default();
        let run = |d: &std::path::Path| {
            let (rc, _) = crate::scan::scan_one(&d.to_string_lossy(), crate::scan::ScanOpts {
                prefix: String::new(), want_json: false, include_tests: false,
                policy: Some(d.join("pol").to_string_lossy().into_owned()),
                baseline: None, quiet: true, ws_member: false, deps_idx: &idx, peek_excluded: false,
            }, &crate::gate::begin_run());
            let wrote = std::fs::read_dir(d.join(".candor")).map(|mut e| e.any(|f| f
                .map(|f| f.file_name().to_string_lossy().starts_with("report.")).unwrap_or(false)))
                .unwrap_or(false);
            (rc, wrote)
        };
        assert_eq!(run(&empty), (2, false),
                   "a target with no readable .rs must refuse at exit 2 AND write no report — a refusal \
                    that leaves an envelope hands `gate --report` an answer disagreeing with this exit");
        assert_eq!(run(&normal).0, 0, "a normal crate with nothing denied must stay green");
        assert_eq!(run(&bad).0, 1, "a certain violation still dominates");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_refusal_leaves_a_marker_and_a_completing_run_clears_it() {
        // ⟨0.32⟩ SPEC §3.3.1: a refusing run records itself BESIDE the reports it would have written,
        // and `gate --report` refuses off that marker.
        //
        // The hazard: a run given no `--out` writes to its default prefix, and a refusal leaves whatever
        // the last successful run put there readable as current. MEASURED in all four engines — scan a
        // tree green, change it so it now violates, refuse for any reason, and `gate --report <tree>`
        // answers `policy ✓` at exit 0 off the previous run's bytes.
        //
        // ARMING THAT PREFIX IS NOT THE ANSWER and this engine has the scar: a run that died in argv
        // parsing once replaced a COMMITTED report in this repository. Naming a prefix is a declaration;
        // a default is a convention, and a convention does not license destroying a file the operator
        // may be keeping. The marker destroys nothing, so it can be written at the EARLIEST moment the
        // prefix is known — pre-parse — which is what lets it cover the argv-death case that arming
        // structurally cannot.
        //
        // FOUR ROWS. The last two are the controls: a marker that is never cleared makes every later run
        // refuse, and a marker written where nothing refused makes the tool useless.
        let root = std::env::temp_dir().join(format!("candor-marker-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("src")).unwrap();
        std::fs::write(root.join("Cargo.toml"),
                       "[package]\nname=\"mk\"\nversion=\"0.0.0\"\nedition=\"2021\"\n").unwrap();
        std::fs::write(root.join("src/lib.rs"), "pub fn f() -> i32 { 1 }\n").unwrap();
        let marker = root.join(".candor/report.refused.json");

        // A clean run leaves no marker.
        crate::gate::note_scan_target(&root.to_string_lossy());
        crate::gate::note_report_prefix(&format!("{}/.candor/report", root.to_string_lossy()));
        crate::gate::clear_refusal_marker();
        assert!(!marker.exists(), "a run that did not refuse left a marker");

        // A refusal writes one, carrying the prefix a consumer needs to match it to a direct-file
        // locator — §3.3.1's direct-file form accepts any `.json` name, so the prefix cannot be
        // recovered from the filename and has to be IN the marker.
        crate::gate::write_refusal_marker("a probe cause");
        assert!(marker.exists(), "a refusal left no marker at the default prefix");
        let doc: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&marker).unwrap()).unwrap();
        assert_eq!(doc["refused"], serde_json::json!(true));
        assert!(doc["prefix"].as_str().is_some_and(|p| p.contains(".candor/report")),
                "the marker must carry its own prefix: {doc}");
        assert!(doc["reason"].as_str().is_some_and(|r| r.contains("a probe cause")),
                "the marker must name the cause: {doc}");

        // …and the consumer finds it.
        assert!(candor_report::refusal_marker_for(
                    &format!("{}/.candor/report", root.to_string_lossy())).is_some(),
                "a prefix locator did not resolve the marker beside it");

        // CONTROL: a completing run clears it, or every later run refuses for ever off a stale marker.
        crate::gate::clear_refusal_marker();
        assert!(!marker.exists(), "a completing run left the marker behind — every later gate over this \
                                   prefix would refuse off it, which is the permanent-red mirror of the \
                                   permanent-green this rung exists to close");
        assert!(candor_report::refusal_marker_for(
                    &format!("{}/.candor/report", root.to_string_lossy())).is_none(),
                "the consumer still reports a refusal after the marker was cleared");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn the_hand_back_never_restores_its_own_placeholder() {
        // ⟨0.31⟩ THE HAND-BACK WAS DEFEATED BY COMPOSITION, in three ordinary steps. MEASURED on a
        // two-member workspace before this fix:
        //
        //   1. scan both members          -> two real reports
        //   2. delete member `b`, refuse  -> `b`'s orphan is armed. Correct: the run refused.
        //   3. scan the remaining member  -> COMPLETES at exit 0, and `b` STILL held the placeholder,
        //                                    because step 3's arming saved step 2's PLACEHOLDER as
        //                                    `b`'s "previous bytes" and the hand-back restored it.
        //
        // `gate --report <prefix>` then refuses at exit 2 off that leftover for ever — the exact state
        // `disarm_unwritten_out_reports` exists to prevent, reached by running it twice. The note at
        // `arm_out_prefix` records the FIRST version of this failure; that was the same failure wearing
        // the fix.
        //
        // The fix records `None` when the bytes being displaced are this machinery's own placeholder,
        // and the hand-back then REMOVES rather than restores. Both directions are asserted here,
        // because removing is sound only for a placeholder: §3.3.1 forbids deleting a REPORT — a
        // consumer reading absence as "nothing to report" fails open — and the second arm is what stops
        // a future edit turning this into that.
        let gate = include_str!("gate.rs");
        assert!(gate.contains("if prev_is_placeholder { None } else { prev_bytes }"),
                "the armer no longer distinguishes a placeholder from a real report, so a complete run \
                 can hand its own marker back and leave a permanent exit-2 behind");
        assert!(gate.contains("None => { let _ = std::fs::remove_file(path); }"),
                "the hand-back no longer removes an orphaned placeholder");
        // …and the REAL-report arm must still restore. If this line goes, the fix has become the
        // fail-open deletion the spec forbids.
        assert!(gate.contains("Some(bytes) => { let _ = std::fs::write(path, bytes); }"),
                "the hand-back no longer restores a real orphaned report byte-for-byte — deleting one \
                 is the fail-open harm §3.3.1 forbids, and this arm is the whole difference");
    }

    #[test]
    fn no_gate_accumulator_can_record_from_inside_the_peek() {
        // ⟨0.31⟩ THE PEEK MUST WRITE NO VERDICT STATE. It re-enters `scan_one` over the files the scan
        // EXCLUDED, so anything it records is carried by the scan route and by no other — the peek writes
        // no report, so `gate --report` cannot reproduce it. That is a §3.1 route-equality break and an
        // over-claim at once, since the gate judged none of those files.
        //
        // THIS HAS HAPPENED TWICE, and the second time is why the guard moved:
        //   · `analyzed`    — MEASURED on `crates/candor-query`: scan route 276, the report it had just
        //                     written 129, and CI's byte-equality row failed on 20 of 54 rows. Fixed with
        //                     a guard at that ONE call site.
        //   · `netPartners` — MEASURED the day the ⟨0.31⟩ key landed, on a crate whose only mention of
        //                     the declared partner was in `build.rs`. Written months after the first fix,
        //                     by someone with no reason to think about a peek, and reproducing it exactly.
        //
        // A per-site guard asks the author to think about a peek at the moment they are thinking about
        // something else. The suppression is central now: `while_peeking` sets a thread-local for the
        // duration of the recursive call and every recorder returns early on it, so the default is safe
        // rather than correct-when-remembered. Nothing is lost — the peek RETURNS a report body and the
        // outer frame reads it, which is how `outOfScope` has always worked.
        //
        // So this pins the two halves of that invariant: the wrapper is applied, and no recorder can
        // escape it.
        let gate = include_str!("gate.rs");
        let scan = include_str!("scan.rs");

        assert!(scan.contains("crate::gate::while_peeking(|| scan_one("),
                "the recursive peek call is no longer wrapped in `while_peeking`. Every gate accumulator \
                 is now reachable from inside the peek, and the two keys above are what that looks like: \
                 a verdict naming things the report cannot carry.");

        let mut unguarded: Vec<String> = Vec::new();
        for (i, line) in gate.lines().enumerate() {
            let Some(m) = line.find("pub(crate) fn record_gate_") else { continue };
            let name: String = line[m + "pub(crate) fn ".len()..]
                .chars().take_while(|c| c.is_alphanumeric() || *c == '_').collect();
            // the guard must be the FIRST thing the body does — a recorder that mutates before checking
            // has already done the damage the check exists to prevent.
            let body_starts = gate.lines().nth(i + 1).unwrap_or("");
            if !body_starts.contains("recording_suppressed()") {
                unguarded.push(format!("{name} (gate.rs:{})", i + 1));
            }
        }
        assert!(unguarded.is_empty(),
                "these gate accumulators do not check `recording_suppressed()` as their FIRST action, so \
                 the ⟨0.30⟩ peek can record through them: {unguarded:?}.\nThe peek walks the files this \
                 scan deliberately did NOT judge. Whatever they record lands in the run's verdict while \
                 the report it wrote cannot carry it — a §3.1 byte-equality break and an over-claiming \
                 disclosure. Add the check, or if this recorder genuinely must fire from inside a peek, \
                 say why HERE rather than removing the line.");
        assert!(unguarded.is_empty() && gate.contains("fn while_peeking<T>"),
                "`while_peeking` itself is gone — the wrapper the check above depends on");
    }

    #[test]
    fn exactly_one_run_token_is_minted_outside_tests() {
        // ⟨0.31⟩ `RunToken` makes "one scan run is one thread" a COMPILE-TIME fact rather than a rule in
        // a comment: it is neither Send nor Sync, so a `par_iter()` or a `thread::scope` over the
        // `[workspace]` member loop captures it and fails to build, pointing at the note on the type.
        //
        // VERIFIED, not assumed — and the first attempt to verify it was VACUOUS. A probe spawning a
        // thread whose body was `let _ = run;` compiled happily, because under edition-2021 precise
        // capture `let _ = x` does not use the value and the closure captured nothing. The property was
        // fine; the probe was testing air. It only failed once the body genuinely used the token.
        //
        // The guarantee rests on two things this pins, because both are one careless edit from gone:
        //   · the token is !Send/!Sync — it carries `PhantomData<*const ()>` and derives no Clone. A
        //     `#[derive(Clone)]` would let a worker mint its own copy, which is the same defect wearing
        //     the token's clothes.
        //   · `begin_run()` is called EXACTLY ONCE outside tests. The whole forcing function is that an
        //     author who parallelises must write a second mint, and lands on the note explaining why
        //     that reintroduces the silent cross-member violation loss.
        let gate = include_str!("gate.rs");
        let scan = include_str!("scan.rs");

        assert!(gate.contains("_one_thread: std::marker::PhantomData<*const ()>"),
                "RunToken no longer carries a !Send/!Sync marker, so `&RunToken` can cross a thread \
                 again and the member loop can be parallelised without a compile error");
        assert!(!gate.contains("derive(Clone)]\npub(crate) struct RunToken")
                && !gate.contains("impl Clone for RunToken"),
                "RunToken became Clone — a worker can now mint its own and the compile error is gone");

        let mints = scan.matches("gate::begin_run()").count();
        assert_eq!(mints, 1,
                   "expected exactly ONE `begin_run()` in scan.rs (in `scan_main`, above the member \
                    loop) and found {mints}. A second mint is how this protection is removed: it is \
                    what an author parallelising the loop writes to make the borrow checker quiet, and \
                    it hands each worker its own thread-local violation list. Read the note on \
                    `gate::RunToken` before adding one.");
        assert!(scan.contains("fn scan_one(dir: &str, opts: ScanOpts, run: &crate::gate::RunToken)"),
                "`scan_one` no longer takes the run token, so nothing carries the invariant into the \
                 place that depends on it");
    }

    #[test]
    fn workspace_members_are_scanned_sequentially_because_the_gate_state_is_thread_local() {
        // ⟨0.30⟩ `GATE_VIOLATIONS` is a THREAD-LOCAL that accumulates across `scan_one` calls, and that
        // is correct ONLY because a `[workspace]` root scans its members sequentially on one thread. The
        // gate.rs note beside the declaration says so. Nothing pinned it.
        //
        // The failure mode if someone parallelises that loop is the reason this is a test rather than a
        // comment: each rayon worker gets its OWN thread-local, so cross-member accumulation silently
        // stops. The symptom is a WRONG EXIT CODE — one member's certain violation lost behind another's
        // "could not evaluate" — not a crash, not a panic, and not a diff any fixture would show.
        //
        // Written 2026-08-20, the night three other sequential loops in this family WERE parallelised
        // for speed. This one must not be, or must thread the state through `scan_one`'s signature first.
        let scan = include_str!("scan.rs");
        let start = scan.find("for d in &dirs {").expect(
            "the workspace member loop `for d in &dirs {` is gone — if it was renamed, update this test; \
             if it was parallelised, read the note at GATE_VIOLATIONS in gate.rs first");
        // The loop body, generously bounded: enough to cover the call and the rc aggregation.
        let body = &scan[start..scan.len().min(start + 4000)];
        for parallel in ["par_iter", "par_bridge", "into_par_iter", "thread::spawn", "scope(|"] {
            assert!(!body.contains(parallel),
                    "`{parallel}` appears in the workspace member loop. GATE_VIOLATIONS is a thread-local \
                     that accumulates ACROSS members, so a parallel loop gives each worker its own and \
                     cross-member accumulation stops — silently, as a wrong exit code. Thread the gate \
                     state through `scan_one` explicitly before making this loop parallel.");
        }
    }

    #[test]
    fn coverage_has_exactly_one_anchor_and_exactly_one_consumer() {
        let deps = include_str!("deps.rs");
        let scan = include_str!("scan.rs");
        let count = |hay: &str, needle: &str| hay.matches(needle).count();
        const WRITES: [&str; 4] = [
            "idx.crates.insert(",
            "idx.untrusted.insert(",
            "idx.incomplete_pkgs.insert(",
            "idx.judged_nothing_pkgs.insert(",
        ];
        // ONE WRITER EACH, and it is `cover`. A fifth registration site added later — the java shape —
        // fails here rather than silently splitting the gate in two.
        for w in WRITES {
            assert_eq!(count(deps, w), 1,
                       "`{w}` must appear EXACTLY once, inside the one `cover` closure — coverage \
                        registered from a second place is a gate that only half exists (candor-java \
                        `d1d3045`: two anchors, and the mutant gating one failed no test)");
        }
        // …and `cover` is what holds them: the four writes sit between `let cover =` and its `};`.
        let start = deps.find("let cover = |name: String, idx: &mut DepIndex| {").expect("the `cover` closure");
        let end = start + deps[start..].find("\n        };").expect("the closure's end");
        let body = &deps[start..end];
        for w in WRITES {
            assert!(body.contains(w), "`{w}` moved OUT of the `cover` closure — the single anchor is gone");
        }
        // ONE CONSUMER of the three refusals. Not one READ — the stderr disclosures read them too — but
        // one place where a report's silence is turned into a purity claim.
        for c in ["deps_idx.untrusted.contains(", "deps_idx.incomplete_pkgs.contains(",
                  "deps_idx.judged_nothing_pkgs.contains("] {
            assert_eq!(count(scan, c), 1,
                       "`{c}` must be consumed EXACTLY once (the κ-ledger `covered` predicate). A second \
                        consumer means the two fixes have to be repeated there, and nothing would say so");
        }
        // VACUITY FLOOR: if a rename makes every pattern above unfindable, the assertions pass on nothing.
        assert!(deps.contains("pub(crate) fn load_dep_reports") && scan.contains("let covered = deps_idx.crates.contains("),
                "this test located neither the loader nor the `covered` predicate — it is asserting about \
                 source it can no longer find, and would go green through any of the defects above");
        // The JOIN gate is deliberately NOT coverage and reads `crates` in several places (a stale or
        // incomplete report's entries must still charge). Asserted so the numbers above are not read as
        // "the dep index is touched once".
        assert!(count(scan, "deps_idx.crates.contains(") >= 4,
                "the join gate reads the CHAINED set at each join site — that is by design (§2.1 \
                 downgrades entries, it does not stop the join), and this test is about COVERAGE");
    }

    /// ONE NOTCH FINER, and it is the majority case: two entries that AGREE ON EFFECTS and differ only in
    /// a literal SURFACE — 1536 of 2041 collisions on pgman's dep tree, 2255 of 3276 on ebman's.
    ///
    /// THIS TEST ALSO INVERTED, and the reason it was written the other way is worth keeping. Withdrawal
    /// treated a surface difference as a disagreement and dropped the key, so the commonest collision
    /// there is took the caller out of `functions` over effects the two entries AGREED ON. The cost of
    /// merging them was recorded as "a 12–20% disclosure increase, deliberately not taken" — and that
    /// number is real but it was read as noise. It is the fix working: the functions that newly carry
    /// `Unknown` are ones whose keys were being dropped silently, so what changed is not that they became
    /// less certain, it is that their uncertainty is now SAID. Absence was never the cheaper answer; it
    /// was the same uncertainty spelled as a purity claim.
    #[test]
    fn two_entries_agreeing_on_effects_but_not_on_surfaces_are_unioned() {
        let dep = std::env::temp_dir().join(format!("candor-dupkey-surf-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dep);
        std::fs::create_dir_all(&dep).unwrap();
        let me = format!("scan-{}", env!("CARGO_PKG_VERSION"));
        let mk = |host: &str| format!(r#"{{"candor": {{"version": "{me}", "toolchain": "s", "spec": "0.23"}},
            "package": "deplib",
            "functions": [{{"fn": "work", "inferred": ["Net"], "hosts": ["{host}"], "hash": "deplib#work"}}]}}"#);
        std::fs::write(dep.join("a.deplib.scan.json"), mk("a.example.com")).unwrap();
        std::fs::write(dep.join("b.deplib.scan.json"), mk("b.example.com")).unwrap();
        let idx = load_dep_reports(dep.to_str());
        let _ = std::fs::remove_dir_all(&dep);
        let e = idx.by_key.get("deplib#work").expect(
            "two entries naming DIFFERENT hosts under one key AGREE about `Net` — withdrawing the key over \
             a surface difference dropped the caller from `functions` entirely, a purity claim over an \
             effect neither entry disputed");
        assert_eq!(e.effects, BTreeSet::from(["Net"]), "the effect both entries stated");
        assert_eq!(e.hosts,
                   BTreeSet::from(["a.example.com".to_string(), "b.example.com".to_string()]),
                   "BOTH hosts must survive: the surfaces union for the same reason the effects do — each \
                    is reachable, so naming only one would under-report WHERE the `Net` goes");
    }

    /// …AND THE EXEMPTION MUST BE ABOUT THE CLAIM, NOT ITS SERIALISATION. `6f2210c` compared two
    /// colliding entries with derived `PartialEq`, which on a `Vec` is ORDER-SENSITIVE and
    /// DUPLICATE-SENSITIVE — so two entries stating the same thing in a different order were still
    /// withdrawn, and the key went absent, which under ⟨0.21⟩ is a positive purity claim. The same
    /// cardinal sin `6f2210c` closed, surviving for any producer that happens to order a vector
    /// differently.
    ///
    /// MEASURED FIRST (standing bar item 8): over 850 real dep reports — 72 490 key collisions — this
    /// shape occurs ZERO times, because a report the §2.1 staleness gate lets through was written by
    /// this binary's version and this writer emits every one of these vectors from a `BTreeSet`. But
    /// `scan-{CARGO_PKG_VERSION}` is a CRATE VERSION, not a build id: any producer claiming it passes
    /// the gate, including a different build of the same version, a hand-written report (this suite
    /// writes them), and any future field whose construction site is not a set.
    ///
    /// SO THE FIELDS ARE SETS NOW, rather than the comparison being taught to sort. `apply_dep_fn`
    /// folds EVERY DepFn field into a `BTreeSet` — the join's result is invariant under permutation
    /// and duplication of all eight — so set equality is not a relaxation of the never-guess rule but
    /// its exact statement: two set-equal entries are operationally indistinguishable and there is
    /// nothing to choose between. Making it a TYPE fact rather than a comparison keeps a field added
    /// later from silently re-opening it, which is what a hand-written per-field comparison would do.
    #[test]
    fn an_identical_claim_serialised_differently_does_not_withdraw_the_key() {
        let dep = std::env::temp_dir().join(format!("candor-dupkey-order-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dep);
        std::fs::create_dir_all(&dep).unwrap();
        let me = format!("scan-{}", env!("CARGO_PKG_VERSION"));
        // The SAME claim, twice: same effects, same hosts, same paths — different ORDER, and the second
        // copy restates one host. Nothing here is ambiguous; there is no third answer to guess.
        let a = format!(r#"{{"candor": {{"version": "{me}", "toolchain": "s", "spec": "0.23"}},
            "package": "deplib",
            "functions": [{{"fn": "work", "inferred": ["Exec", "Net"],
              "hosts": ["a.example.com", "b.example.com"], "paths": ["/tmp/x", "/tmp/y"],
              "hash": "deplib#work"}}]}}"#);
        let b = format!(r#"{{"candor": {{"version": "{me}", "toolchain": "s", "spec": "0.23"}},
            "package": "deplib",
            "functions": [{{"fn": "work", "inferred": ["Net", "Exec"],
              "hosts": ["b.example.com", "a.example.com", "a.example.com"], "paths": ["/tmp/y", "/tmp/x"],
              "hash": "deplib#work"}}]}}"#);
        std::fs::write(dep.join("a.deplib.scan.json"), a).unwrap();
        std::fs::write(dep.join("b.deplib.scan.json"), b).unwrap();
        let idx = load_dep_reports(dep.to_str());
        assert!(idx.by_key.contains_key("deplib#work"),
                "the key was WITHDRAWN over the ORDER of two identical claims — a consumer of \
                 `deplib::work` then reads a `Net`+`Exec` call as PURE, with no entry and no hedge");
        let e = idx.by_key.get("deplib#work").unwrap();
        assert_eq!(e.effects.iter().copied().collect::<Vec<_>>(), vec!["Exec", "Net"],
                   "the surviving entry must be the claim both copies made");
        assert_eq!(e.hosts.iter().cloned().collect::<Vec<_>>(),
                   vec!["a.example.com".to_string(), "b.example.com".to_string()],
                   "…including its literal surface, deduplicated");
        // AND AT THE CONSUMER, which is where the cardinal sin actually lands: the effect must arrive.
        let v = scan_crate_chained("dupkeyorder", "consumer", "\n[dependencies]\ndeplib = \"1\"\n",
            "pub fn go() { deplib::work(); }\n", &idx);
        let _ = std::fs::remove_dir_all(&dep);
        let effs: Vec<String> = v["functions"].as_array().into_iter().flatten()
            .find(|f| f["fn"].as_str() == Some("go"))
            .and_then(|f| f["inferred"].as_array().cloned()).unwrap_or_default()
            .iter().filter_map(|x| x.as_str().map(String::from)).collect();
        assert_eq!(effs, vec!["Exec".to_string(), "Net".to_string()],
                   "the consumer went silent over a serialisation difference between two copies of one \
                    claim — ⟨0.21⟩ reads that absence as a purity claim");
    }


    /// Effects of `name`, or EMPTY when the function is absent — the report lists only EFFECTFUL
    /// functions, so "pure" and "not present" are the same observation. The over-charge controls need
    /// this: `fn_entry` panics on a function that is (correctly) pure.
    #[cfg(test)]
    fn effs_opt(v: &serde_json::Value, name: &str) -> Vec<String> {
        v["functions"].as_array().into_iter().flatten()
            .find(|f| f["fn"] == name)
            .map(effs).unwrap_or_default()
    }

    // ── std I/O HANDLE RECEIVERS ────────────────────────────────────────────────────────────────
    // A method call on a receiver typed to a std I/O handle (`fn run(cmd: &mut Command)` calling
    // `cmd.spawn()`) formed NO `Type::method` path at all: `visit_method_call` skipped every
    // `std`/`core`/`alloc`-rooted receiver type, so the call never reached the classifier and the
    // function certified PURE while it spawned a process — a silent false all-clear on the exact
    // surface `deny Exec` exists to gate. Receiver typing itself was never at fault: the same
    // function taking a `tokio::process::Command` was caught, because tokio is not std.
    //
    // The CONTROLS come first and they are the reason the widening is narrow: the routed set is the
    // enumerated std HANDLE types (`is_std_effect_handle`), never the whole of std, because std's
    // pure DATA types (`Metadata`, `DirEntry`, `Vec`, …) sit under the same coarse `std::fs::`
    // prefix rules and would be charged an effect for reading a field.

    /// CONTROL (over-charge, the mandated one): a crate that defines its OWN `Command` with its own
    /// inherent `spawn` must stay PURE. The local type never expands to a std path through `uses`, so
    /// the widening cannot reach it — this pins that, because a fix that reddens this control has
    /// traded a false all-clear for a fabrication on a provably-pure local path.
    #[test]
    fn a_local_command_shadowing_std_is_not_charged_exec() {
        let v = scan_src_to_json("localcmdshadow", "\
            pub struct Command;\n\
            impl Command { pub fn spawn(&self) {} }\n\
            pub fn run(cmd: &Command) { cmd.spawn(); }\n");
        assert!(!effs_opt(&v, "run").contains(&"Exec".to_string()),
                "a LOCAL `Command::spawn` that does nothing must not inherit std's Exec — the \
                 `local_types` shadowing case the std exclusion was protecting:\n{v:#}");
    }

    /// CONTROL (over-charge): the same shadowing for the Fs and Net handle types, since the widening
    /// names three families and a control on one of them is not a control on the others.
    #[test]
    fn local_file_and_tcpstream_shadows_are_not_charged() {
        let v = scan_src_to_json("localiohshadow", "\
            pub struct File;\n\
            impl File { pub fn write_all(&self, _b: &[u8]) {} }\n\
            pub struct TcpStream;\n\
            impl TcpStream { pub fn connect(&self) {} }\n\
            pub fn w(f: &File) { f.write_all(b\"x\"); }\n\
            pub fn c(s: &TcpStream) { s.connect(); }\n");
        assert!(!effs_opt(&v, "w").contains(&"Fs".to_string()), "local File shadow:\n{v:#}");
        assert!(!effs_opt(&v, "c").contains(&"Net".to_string()), "local TcpStream shadow:\n{v:#}");
    }

    /// CONTROL (over-charge): std's pure DATA types must NOT route. `Metadata::len` and
    /// `DirEntry::path` read an already-fetched stat struct and issue no syscall, but they live under
    /// the coarse `std::fs::` prefix rule — routing all of std would charge them Fs. This control is
    /// what forces the routed set to be an enumerated handle list rather than "not a known-pure name".
    #[test]
    fn std_pure_data_type_receivers_are_not_routed() {
        let v = scan_src_to_json("stdpuredata", "\
            pub fn size(m: &std::fs::Metadata) -> u64 { m.len() }\n\
            pub fn name(e: &std::fs::DirEntry) -> std::path::PathBuf { e.path() }\n\
            pub fn push(v: &mut Vec<u8>) { v.push(1); }\n");
        assert!(!effs_opt(&v, "size").contains(&"Fs".to_string()),
                "`Metadata::len` reads a struct field — charging Fs is a fabrication:\n{v:#}");
        assert!(!effs_opt(&v, "name").contains(&"Fs".to_string()),
                "`DirEntry::path` performs no syscall:\n{v:#}");
        assert!(effs_opt(&v, "push").is_empty(), "`Vec::push` is pure:\n{v:#}");
    }

    /// CONTROL (over-charge): on the types that DO route, the classifier's reviewed pure-accessor
    /// carve-outs must still hold through the receiver route — the route must not become a second,
    /// coarser door into rules that were already refined once (`local_addr`/`get_program` read back
    /// state, `as_raw_fd` borrows a descriptor opened elsewhere).
    #[test]
    fn pure_accessors_on_routed_std_handles_stay_pure() {
        let v = scan_src_to_json("stdhandleacc", "\
            pub fn a(s: &std::net::TcpStream) { let _ = s.local_addr(); }\n\
            pub fn b(c: &std::process::Command) { let _ = c.get_program(); }\n\
            pub fn c(f: &std::fs::File) { let _ = f.as_raw_fd(); }\n\
            pub fn d(s: &std::net::TcpStream) { let _ = s.peer_addr(); }\n");
        for (name, eff) in [("a", "Net"), ("b", "Exec"), ("c", "Fs"), ("d", "Net")] {
            assert!(!effs_opt(&v, name).contains(&eff.to_string()),
                    "`{name}` is a pure read-back — the receiver route must not re-fabricate {eff}:\n{v:#}");
        }
    }

    /// CONTROL (over-charge): `.clone()` keeps its exclusion. Through the `Arc`/`Rc`/`Box` deref-peel
    /// an `Arc<File>` receiver types as `File`, so a routed `.clone()` would form `File::clone` and
    /// charge Fs for a refcount bump that calls `Arc::clone`, not `File::clone`.
    #[test]
    fn clone_on_a_routed_std_handle_is_still_excluded() {
        let v = scan_src_to_json("stdhandleclone", "\
            use std::sync::Arc;\n\
            pub fn k(f: &Arc<std::fs::File>) { let _ = f.clone(); }\n");
        assert!(!effs_opt(&v, "k").contains(&"Fs".to_string()),
                "`arc.clone()` is a refcount bump:\n{v:#}");
    }

    /// THE SIN, all five spellings that were confirmed to miss identically. Each of these functions
    /// performs the effect its policy denies; before the fix every one of them reported `inferred: []`.
    #[test]
    fn std_io_handle_receiver_methods_are_charged() {
        let v = scan_src_to_json("stdhandlesin", "\
            use std::process::Command;\n\
            use std::fs::File;\n\
            use std::net::TcpStream;\n\
            use std::io::Write;\n\
            pub fn borrowed(cmd: &mut Command) { let _ = cmd.spawn(); }\n\
            pub fn owned(mut cmd: Command) { let _ = cmd.spawn(); }\n\
            pub fn fully_qualified(cmd: &mut std::process::Command) { let _ = cmd.spawn(); }\n\
            pub fn file(f: &mut File) { let _ = f.write_all(b\"x\"); }\n\
            pub fn sock(s: &mut TcpStream) { let _ = s.write_all(b\"x\"); }\n\
            pub fn child(c: &mut std::process::Child) { let _ = c.kill(); }\n");
        for name in ["borrowed", "owned", "fully_qualified"] {
            assert!(effs(fn_entry(&v, name)).contains(&"Exec".to_string()),
                    "`{name}` spawns a process and certified PURE — the cardinal sin:\n{v:#}");
        }
        assert!(effs(fn_entry(&v, "child")).contains(&"Exec".to_string()), "`Child::kill` is Exec:\n{v:#}");
        assert!(effs(fn_entry(&v, "file")).contains(&"Fs".to_string()),
                "`File::write_all` writes to disk:\n{v:#}");
        assert!(effs(fn_entry(&v, "sock")).contains(&"Net".to_string()),
                "`TcpStream::write_all` writes to a socket:\n{v:#}");
    }

    /// THE SIN, REOPENED BY ITS OWN FIX — found by fixture, not by review. Routing std handle
    /// receivers forms `std::process::Command::spawn`, but `tail2` keeps only the last two segments,
    /// so a crate that ALSO defines its own `Command` with a `spawn` puts that bare leaf in
    /// `local_types`, the std path resolves to the LOCAL method, and `resolved_local` suppresses the
    /// classifier. The genuine spawn certified clean again — the same false all-clear, now with a
    /// local type's effects fabricated in its place. A std-qualified typed path is the classifier's.
    #[test]
    fn a_local_type_sharing_a_std_handles_name_cannot_capture_the_std_call() {
        let v = scan_src_to_json("stdhandlecollide", "\
            pub mod mine {\n\
                pub struct Command;\n\
                impl Command { pub fn spawn(&self) {} }\n\
            }\n\
            pub fn run(c: &mut std::process::Command) { let _ = c.spawn(); }\n");
        assert!(effs(fn_entry(&v, "run")).contains(&"Exec".to_string()),
                "a local `mine::Command::spawn` captured a std `Command::spawn` and silenced it:\n{v:#}");
    }

    /// The same hole through every OTHER receiver spelling the scanner can type. The gate that hid it
    /// sat at the routing frontier, not at any one inference route, so a fixture over parameters alone
    /// would not have shown whether fields, `Box` receivers and loop elements were also silent — they
    /// were. (`let c = Command::new(..)` was never silent: the CONSTRUCTOR is a qualified call, which
    /// is exactly the asymmetry that made a builder-only `constructs_only` red while a real spawn on a
    /// parameter was green.)
    #[test]
    fn std_handle_receivers_are_charged_through_field_box_and_loop_spellings() {
        let v = scan_src_to_json("stdhandlespell", "\
            use std::process::Command;\n\
            use std::io::Write;\n\
            pub struct Holder { pub cmd: Command }\n\
            impl Holder { pub fn go(&mut self) { let _ = self.cmd.spawn(); } }\n\
            pub fn via_box(c: &mut Box<Command>) { let _ = c.spawn(); }\n\
            pub fn via_loop(v: &mut Vec<std::fs::File>) {\n\
                for f in v.iter_mut() { let _ = f.write_all(b\"x\"); }\n\
            }\n");
        assert!(effs(fn_entry(&v, "Holder::go")).contains(&"Exec".to_string()),
                "a `Command` FIELD receiver:\n{v:#}");
        assert!(effs(fn_entry(&v, "via_box")).contains(&"Exec".to_string()),
                "a `Box<Command>` receiver (the smart-pointer deref-peel):\n{v:#}");
        assert!(effs(fn_entry(&v, "via_loop")).contains(&"Fs".to_string()),
                "a `File` LOOP-ELEMENT receiver:\n{v:#}");
    }

    // ── SPEC §1 ⟨0.32⟩: THE INVOCATION OBJECT vs THE OPTION-BUILDER ─────────────────────────────
    // Three MEASURED residuals of the std-handle receiver routing, written as controls FIRST.
    //
    //  R1  a project's OWN type charged `Exec` because the FILE imports the std one. A regression the
    //      previously-written shadow control could not see: it used ONE file with NO `use` in scope,
    //      and the whole mechanism is the file's `use` map reaching a name it does not govern.
    //  R2  a pure read-back's RESULT, used (`c.get_program().to_str()`), charged `Exec` — the chain
    //      walk attributes the outer leaf to the BASE receiver, so the reviewed carve-out is bypassed
    //      by the very next `.` in the expression.
    //  R3  `OpenOptions` in BOTH directions at once: `o.open(p)` on a received `&OpenOptions` answered
    //      NOTHING (the terminal verb, the thing that opens the file), while `OpenOptions::new()
    //      .read(true)` with no `open` at all answered `Fs`. SPEC §1 ⟨0.32⟩ names this shape: an
    //      option-builder for another effect stays PURE, because its resource arrives at the terminal
    //      verb, which is charged at its own call site.

    /// CONTROL (over-charge, R1) — THE REGRESSION. A submodule's own `Command`, in a file that ALSO
    /// imports `std::process::Command` for its own use. Rust does not let an inline `mod`'s names be
    /// resolved by the enclosing file's imports, so `mine::run`'s receiver is `mine::Command` and its
    /// `spawn` does nothing. The `real()` arm is the other direction in the same fixture: the file's
    /// import still governs the file's own code, so a genuine spawn stays `Exec`.
    #[test]
    fn a_submodules_own_type_is_not_charged_from_the_files_std_import() {
        let v = scan_src_to_json("submodshadow", "\
            use std::process::Command;\n\
            pub mod mine {\n\
                pub struct Command;\n\
                impl Command { pub fn spawn(&self) {} }\n\
                pub fn run(c: &Command) { c.spawn(); }\n\
            }\n\
            pub fn real() { let mut c = Command::new(\"sh\"); let _ = c.spawn(); }\n");
        assert!(!effs_opt(&v, "mine::run").contains(&"Exec".to_string()),
                "`mine::Command` is the crate's OWN type — the file's `use std::process::Command` \
                 does not reach into `mod mine`, and charging Exec here is a fabrication:\n{v:#}");
        assert!(effs(fn_entry(&v, "real")).contains(&"Exec".to_string()),
                "the file's own code DOES resolve `Command` through its import — `real` spawns:\n{v:#}");
    }

    /// CONTROL (R1, the other direction, per shadowed type): the same submodule shadow for `File` and
    /// `TcpStream`, so the fix is pinned on every family the routing names, not just the one measured.
    #[test]
    fn submodule_shadows_of_file_and_tcpstream_are_not_charged() {
        let v = scan_src_to_json("submodshadow2", "\
            use std::fs::File;\n\
            use std::net::TcpStream;\n\
            pub mod mine {\n\
                pub struct File;\n\
                impl File { pub fn write_all(&self, _b: &[u8]) {} }\n\
                pub struct TcpStream;\n\
                impl TcpStream { pub fn connect(&self) {} }\n\
                pub fn w(f: &File) { f.write_all(b\"x\"); }\n\
                pub fn c(s: &TcpStream) { s.connect(); }\n\
            }\n\
            pub fn real_open() { let _ = File::open(\"/etc/hosts\"); }\n\
            pub fn real_net() { let _ = TcpStream::connect(\"127.0.0.1:1\"); }\n");
        assert!(!effs_opt(&v, "mine::w").contains(&"Fs".to_string()), "local File shadow:\n{v:#}");
        assert!(!effs_opt(&v, "mine::c").contains(&"Net".to_string()), "local TcpStream shadow:\n{v:#}");
        assert!(effs(fn_entry(&v, "real_open")).contains(&"Fs".to_string()), "the real File::open:\n{v:#}");
        assert!(effs(fn_entry(&v, "real_net")).contains(&"Net".to_string()), "the real connect:\n{v:#}");
    }

    /// CONTROL (R1): a submodule's own FREE FUNCTION sharing an imported name is the same shadow one
    /// namespace over — `mod mine { fn read(..) }` is `mine::read`, never `std::fs::read`.
    #[test]
    fn a_submodules_own_free_fn_is_not_charged_from_the_files_std_import() {
        let v = scan_src_to_json("submodfnshadow", "\
            use std::fs::read;\n\
            pub mod mine {\n\
                pub fn read(_p: &str) -> Vec<u8> { Vec::new() }\n\
                pub fn go() { let _ = read(\"x\"); }\n\
            }\n\
            pub fn real() { let _ = read(\"/etc/hosts\"); }\n");
        assert!(!effs_opt(&v, "mine::go").contains(&"Fs".to_string()),
                "`mine::read` is the crate's own function:\n{v:#}");
        assert!(effs(fn_entry(&v, "real")).contains(&"Fs".to_string()),
                "the file's own `read(..)` IS `std::fs::read`:\n{v:#}");
    }

    /// CONTROL (over-charge, R2): a pure read-back's RESULT is a DIFFERENT type — `get_program()`
    /// hands back an `&OsStr`, `get_args()` a `CommandArgs` — so the chain walk must stop there
    /// rather than attribute `to_str`/`len`/`collect`/`unwrap` to the `Command` and charge Exec.
    /// Measured before the fix: all four of these reported `["Exec"]`.
    #[test]
    fn a_chain_off_a_command_read_back_is_not_charged_exec() {
        let v = scan_src_to_json("cmdreadbackchain", "\
            use std::process::Command;\n\
            pub struct H { pub cmd: Command }\n\
            impl H { pub fn field_chain(&self) { let _ = self.cmd.get_program().to_string_lossy(); } }\n\
            pub fn to_str(c: &Command) { let _ = c.get_program().to_str(); }\n\
            pub fn len(c: &Command) { let _ = c.get_args().len(); }\n\
            pub fn collect(c: &Command) { let _: Vec<_> = c.get_args().collect(); }\n\
            pub fn cwd(c: &Command) { let _ = c.get_current_dir().unwrap(); }\n\
            pub fn envs(c: &Command) { for (k, _v) in c.get_envs() { let _ = k; } }\n");
        for name in ["H::field_chain", "to_str", "len", "collect", "cwd", "envs"] {
            assert!(!effs_opt(&v, name).contains(&"Exec".to_string()),
                    "`{name}` only reads back the builder's stored state and uses the RESULT — the \
                     carve-out must survive the next `.`:\n{v:#}");
        }
    }

    /// CONTROL (R2, the direction that must NOT move): the read-back carve-out is NOT a licence to
    /// drop the invocation object. SPEC §1 ⟨0.32⟩ charges construction and the argument/env/redirect
    /// SETTERS as `Exec` alongside the launch, and no `get_`-prefix or bare-leaf rule may reach them.
    #[test]
    fn command_setters_and_launches_stay_exec() {
        let v = scan_src_to_json("cmdsetters", "\
            use std::process::Command;\n\
            pub fn ctor() { let _ = Command::new(\"sh\"); }\n\
            pub fn arg(c: &mut Command) { c.arg(\"x\"); }\n\
            pub fn env(c: &mut Command) { c.env(\"K\", \"V\"); }\n\
            pub fn cwd(c: &mut Command) { c.current_dir(\"/tmp\"); }\n\
            pub fn stdio(c: &mut Command) { c.stdout(std::process::Stdio::null()); }\n\
            pub fn spawn(c: &mut Command) { let _ = c.spawn(); }\n\
            pub fn output(c: &mut Command) { let _ = c.output(); }\n\
            pub fn kill(c: &mut std::process::Child) { let _ = c.kill(); }\n");
        for name in ["ctor", "arg", "env", "cwd", "stdio", "spawn", "output", "kill"] {
            assert!(effs(fn_entry(&v, name)).contains(&"Exec".to_string()),
                    "`{name}` is part of the subprocess capability (SPEC §1 ⟨0.32⟩):\n{v:#}");
        }
    }

    /// THE UNDER-REPORT (R3). `open` is the terminal verb: it takes the path and opens the file. On a
    /// RECEIVED `&OpenOptions` it formed no path at all and the function certified PURE — the same
    /// shape as the `Command` parameter sin, on the type that was deliberately left out of the routed
    /// handle list. Every receiver spelling, because the gate sat at the routing frontier.
    #[test]
    fn open_on_a_received_openoptions_is_charged_fs() {
        let v = scan_src_to_json("openoptsin", "\
            use std::fs::OpenOptions;\n\
            use std::path::Path;\n\
            pub struct H { pub o: OpenOptions }\n\
            impl H { pub fn field(&self, p: &Path) { let _ = self.o.open(p); } }\n\
            pub fn borrowed(o: &OpenOptions, p: &Path) { let _ = o.open(p); }\n\
            pub fn owned(o: OpenOptions, p: &Path) { let _ = o.open(p); }\n\
            pub fn qualified(o: &std::fs::OpenOptions, p: &Path) { let _ = o.open(p); }\n\
            pub fn boxed(o: &Box<OpenOptions>, p: &Path) { let _ = o.open(p); }\n\
            pub fn chained(p: &Path) { let _ = OpenOptions::new().read(true).open(p); }\n");
        for name in ["H::field", "borrowed", "owned", "qualified", "boxed", "chained"] {
            assert!(effs(fn_entry(&v, name)).contains(&"Fs".to_string()),
                    "`{name}` opens a file and certified PURE — the cardinal sin:\n{v:#}");
        }
    }

    /// CONTROL (over-charge, R3): an option-builder for ANOTHER effect stays PURE. `OpenOptions::new()`
    /// and its setters record flags in a struct; nothing is opened until `open(path)` is called, and
    /// that call is charged at its own site (the sin test above). Before the fix a `let o =
    /// OpenOptions::new().read(true);` with no `open` anywhere answered `Fs`.
    #[test]
    fn openoptions_setters_without_open_stay_pure() {
        let v = scan_src_to_json("openoptspure", "\
            use std::fs::OpenOptions;\n\
            pub fn build() -> OpenOptions {\n\
                let mut o = OpenOptions::new();\n\
                o.read(true).write(true).append(false).truncate(false).create(true).create_new(false);\n\
                o\n\
            }\n\
            pub fn fluent() -> OpenOptions { OpenOptions::new().read(true).write(true).clone() }\n\
            pub fn setter(o: &mut OpenOptions) { o.append(true); }\n");
        for name in ["build", "fluent", "setter"] {
            assert!(!effs_opt(&v, name).contains(&"Fs".to_string()),
                    "`{name}` only records flags — SPEC §1 ⟨0.32⟩ keeps an option-builder for another \
                     effect PURE, its resource arrives at the terminal verb:\n{v:#}");
        }
    }

    /// CONTROL (R3, the descriptor trap): `create` is a PURE SETTER on `OpenOptions` and the TERMINAL
    /// VERB on `DirBuilder` — same leaf, opposite answers. candor-java carves its read-backs by
    /// DESCRIPTOR for exactly this reason (`command()` vs `command(List)`); Rust has no overloads, so
    /// the carve-out must be keyed on the TYPE. A bare-name denylist gets `DirBuilder::create` wrong.
    #[test]
    fn create_is_a_setter_on_openoptions_and_a_verb_on_dirbuilder() {
        let v = scan_src_to_json("createtrap", "\
            use std::fs::{OpenOptions, DirBuilder};\n\
            pub fn opt() { let _ = OpenOptions::new().create(true); }\n\
            pub fn dir() { let _ = DirBuilder::new().recursive(true).create(\"/tmp/x\"); }\n\
            pub fn dir_recv(b: &DirBuilder) { let _ = b.create(\"/tmp/x\"); }\n");
        assert!(!effs_opt(&v, "opt").contains(&"Fs".to_string()),
                "`OpenOptions::create(bool)` sets a flag:\n{v:#}");
        assert!(effs(fn_entry(&v, "dir")).contains(&"Fs".to_string()),
                "`DirBuilder::create(path)` makes a directory:\n{v:#}");
        assert!(effs(fn_entry(&v, "dir_recv")).contains(&"Fs".to_string()),
                "`DirBuilder::create` on a RECEIVED builder is the same syscall:\n{v:#}");
    }

    /// CONTROL (R3): a project's OWN `OpenOptions` must gain nothing from the widening — the R1 shape
    /// applied to the type R3 adds, since a fix and its control belong to the same commit.
    #[test]
    fn a_local_openoptions_shadow_is_not_charged_fs() {
        let v = scan_src_to_json("openoptsshadow", "\
            use std::fs::OpenOptions;\n\
            pub mod mine {\n\
                pub struct OpenOptions;\n\
                impl OpenOptions { pub fn open(&self, _p: &str) {} }\n\
                pub fn go(o: &OpenOptions) { o.open(\"x\"); }\n\
            }\n\
            pub fn real(p: &std::path::Path) { let _ = OpenOptions::new().read(true).open(p); }\n");
        assert!(!effs_opt(&v, "mine::go").contains(&"Fs".to_string()),
                "a LOCAL `OpenOptions::open` that does nothing must not inherit std's Fs:\n{v:#}");
        assert!(effs(fn_entry(&v, "real")).contains(&"Fs".to_string()),
                "the real std builder still opens the file:\n{v:#}");
    }

    /// CONTROL (R3, SPEC §2 `fs` — the row PART 31 asserts). The terminal verb's DIRECTION was set by
    /// the builder chain, which the classifier cannot read, so `open` on an `OpenOptions` proves `Fs`
    /// and claims NO kind — and a caller mixing it with a writer must suppress the whole field, never
    /// publish the writer's half. §2: an empty or partial `fs` reads as "writes but never reads".
    ///
    /// This control exists because the fix WITHOUT it minted exactly that false claim: routing
    /// `OpenOptions::open` into the classifier put it in front of `fs_kind`, whose READ list holds a
    /// bare `open` for `File::open` — MEASURED, `fs` went from ABSENT to `["read"]` on a builder that
    /// may have been configured `write(true)`. The guard is keyed on the TYPE segment, so `File::open`
    /// keeps its `["read"]` (asserted here too, because a carve-out that swallows the real verb has
    /// traded one wrong answer for another).
    #[test]
    fn openoptions_open_proves_fs_but_claims_no_direction() {
        let v = scan_src_to_json("openoptskind", "\
            use std::fs::OpenOptions;\n\
            use std::path::Path;\n\
            pub fn writes_only() { let _ = std::fs::write(\"/tmp/b\", \"x\"); }\n\
            pub fn undetermined(p: &Path) { let _ = OpenOptions::new().write(true).open(p); }\n\
            pub fn on_param(o: &OpenOptions, p: &Path) { let _ = o.open(p); }\n\
            pub fn mixed(p: &Path) { writes_only(); undetermined(p); }\n\
            pub fn plain_open() { let _ = std::fs::File::open(\"/tmp/a\"); }\n");
        for name in ["undetermined", "on_param", "mixed"] {
            let e = fn_entry(&v, name);
            assert!(effs(e).contains(&"Fs".to_string()), "`{name}` must prove Fs:\n{v:#}");
            assert!(e.get("fs").is_none(),
                    "`{name}` must claim NO direction — the builder chain set it and the classifier \
                     cannot read it, so §2 requires the field OMITTED, not the writer's half:\n{v:#}");
        }
        assert_eq!(fn_entry(&v, "writes_only")["fs"], serde_json::json!(["write"]),
                   "a determined verb still answers:\n{v:#}");
        assert_eq!(fn_entry(&v, "plain_open")["fs"], serde_json::json!(["read"]),
                   "`File::open` IS unambiguously a read — the carve-out must not swallow it:\n{v:#}");
    }

    // ── SUBMODULE-LEVEL RE-EXPORTS ──────────────────────────────────────────────────────────────
    // A call through a re-export declared in a SUBMODULE (`mod imp { mod platform; pub use
    // self::platform::*; }`) resolved to NOTHING: the crate-root re-export machinery
    // (`collect_root_reexports`) covers the ROOT file only, and the intra-crate call graph keys on the
    // last TWO segments — the definition's tail2 is `platform::doit` while the call site writes
    // `imp::doit`, so the two never met and every caller of `imp::doit` read SILENT-PURE.
    //
    // MEASURED on the shape below before the fix: `go` reported no `Exec` at all, in the file-per-module
    // form AND the inline-`mod` form, for the glob (`pub use self::platform::*`) AND the named
    // (`pub use self::platform::doit`) spelling, and two re-export hops deep. Real-world instance:
    // tempfile's `src/file/imp/mod.rs` is exactly this, so `NamedTempFile::new` and its seven siblings
    // did not reach the `Fs` in `file::imp::unix::create_named`.

    /// Build and scan a multi-FILE crate. `files` are `(path-under-the-crate-root, contents)`;
    /// intermediate directories are created. The submodule re-export lives in a `mod.rs` that the
    /// caller of the re-exported fn never sees, so the single-file `scan_src_to_json` cannot express it.
    #[cfg(test)]
    fn scan_crate_to_json(tag: &str, files: &[(&str, &str)]) -> serde_json::Value {
        let d = std::env::temp_dir().join(format!("candor-scan-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(d.join("src")).unwrap();
        std::fs::write(d.join("Cargo.toml"), format!("[package]\nname = \"{tag}\"\n")).unwrap();
        for (rel, src) in files {
            let p = d.join(rel);
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(&p, src).unwrap();
        }
        let idx = DepIndex::default();
        let (rc, body) = scan_one(&d.to_string_lossy(), ScanOpts {
            prefix: String::new(), want_json: true, include_tests: false, policy: None,
            baseline: None, ws_member: false, quiet: true, deps_idx: &idx, peek_excluded: false,
        }, &crate::gate::begin_run());
        let _ = std::fs::remove_dir_all(&d);
        assert_eq!(rc, 0, "scan should succeed:\n{body:?}");
        serde_json::from_str(&body.unwrap()).unwrap()
    }

    const SPAWN: &str = "pub fn doit() { let _ = std::process::Command::new(\"sh\").spawn(); }\n";
    /// The same one-line spawn under a chosen fn name, for the fixtures that need a second spelling.
    #[cfg(test)]
    #[allow(non_snake_case)]
    fn SPAWN_AS(name: &str) -> String {
        format!("pub fn {name}() {{ let _ = std::process::Command::new(\"sh\").spawn(); }}\n")
    }

    /// DEFECT, file-per-module, GLOB form. `imp/mod.rs` re-exports its own submodule; the caller in
    /// `lib.rs` writes `imp::doit()`. Before the fix: `go` had no effects at all.
    #[test]
    fn a_call_through_a_submodule_glob_reexport_reaches_the_effect() {
        let v = scan_crate_to_json("subreexpglob", &[
            ("src/lib.rs", "mod imp;\npub fn go() { imp::doit(); }\n"),
            ("src/imp/mod.rs", "mod platform;\npub use self::platform::*;\n"),
            ("src/imp/platform.rs", SPAWN),
        ]);
        assert!(effs_opt(&v, "imp::platform::doit").contains(&"Exec".to_string()),
                "the definition itself must be Exec — else the fixture proves nothing:\n{v:#}");
        assert!(effs_opt(&v, "go").contains(&"Exec".to_string()),
                "`go` calls `imp::doit()`, which IS `imp::platform::doit` through the submodule's \
                 `pub use self::platform::*` — reporting nothing is a silent under-report:\n{v:#}");
    }

    /// DEFECT, file-per-module, NAMED form — it missed identically, which is what proved the failure is
    /// the SUBMODULE re-export and not the glob.
    #[test]
    fn a_call_through_a_submodule_named_reexport_reaches_the_effect() {
        let v = scan_crate_to_json("subreexpnamed", &[
            ("src/lib.rs", "mod imp;\npub fn go() { imp::doit(); }\n"),
            ("src/imp/mod.rs", "mod platform;\npub use self::platform::doit;\n"),
            ("src/imp/platform.rs", SPAWN),
        ]);
        assert!(effs_opt(&v, "go").contains(&"Exec".to_string()),
                "the NAMED re-export misses the same way the glob does:\n{v:#}");
    }

    /// DEFECT, INLINE `mod` — the same two spellings with no second file involved, so the fix is pinned
    /// on the walk and not on the file-to-module-path mapping.
    #[test]
    fn a_call_through_an_inline_submodule_reexport_reaches_the_effect() {
        for (tag, reexport) in [("inlglob", "pub use self::platform::*;"),
                                ("inlnamed", "pub use self::platform::doit;")] {
            let v = scan_src_to_json(tag, &format!("\
pub mod imp {{
    mod platform {{ {SPAWN} }}
    {reexport}
}}
pub fn go() {{ imp::doit(); }}
"));
            assert!(effs_opt(&v, "go").contains(&"Exec".to_string()),
                    "inline `{reexport}`:\n{v:#}");
        }
    }

    /// DEFECT, TWO hops. `a` re-exports `a::b`, which re-exports `a::b::c` — the name has to travel two
    /// re-export edges before `a::doit()` names it, so the alias index has to reach a fixpoint rather
    /// than take one step.
    #[test]
    fn a_call_through_two_nested_reexports_reaches_the_effect() {
        let v = scan_crate_to_json("subreexpdeep", &[
            ("src/lib.rs", "mod a;\npub fn go() { a::doit(); }\n"),
            ("src/a/mod.rs", "mod b;\npub use self::b::*;\n"),
            ("src/a/b/mod.rs", "mod c;\npub use self::c::*;\n"),
            ("src/a/b/c.rs", SPAWN),
        ]);
        assert!(effs_opt(&v, "go").contains(&"Exec".to_string()),
                "two re-export hops:\n{v:#}");
    }

    /// DEFECT, the tempfile shape verbatim: a `#[cfg_attr(.., path = "..")]`-redirected `mod platform;`
    /// whose body lives in `unix.rs`/`windows.rs`/`other.rs`. The scanner walks EVERY `#[cfg]` branch
    /// (its standing over-approximation), so all three files are analysed and the re-export names all
    /// three — a call to `imp::doit()` reaches whichever one compiles, and charging the union is the
    /// same discipline `cfg_if` arms already get. Without the `#[path]` mapping the glob points at a
    /// module path (`imp::platform`) that no analysed file carries, so it names nothing.
    #[test]
    fn a_reexport_through_a_path_redirected_platform_module_reaches_the_effect() {
        let v = scan_crate_to_json("subreexppath", &[
            ("src/lib.rs", "mod imp;\npub fn go() { imp::doit(); }\n"),
            ("src/imp/mod.rs", "#[cfg_attr(unix, path = \"unix.rs\")]\n\
                                #[cfg_attr(windows, path = \"windows.rs\")]\n\
                                mod platform;\n\
                                pub use self::platform::*;\n"),
            ("src/imp/unix.rs", SPAWN),
            ("src/imp/windows.rs", SPAWN),
        ]);
        assert!(effs_opt(&v, "go").contains(&"Exec".to_string()),
                "the `#[path]`-redirected platform module is where tempfile's real Fs lives:\n{v:#}");
    }

    /// DEFECT, the OTHER tempfile spelling: Rust 2018 uniform paths let a re-export name a child module
    /// with no `self::` (`pub use unix::*;`), which tempfile's `src/dir/imp/mod.rs` uses. A bare head
    /// segment is an EXTERNAL crate unless the module declares it, so the rule is keyed on the `mod`
    /// declaration being present — never on the bare name alone.
    #[test]
    fn a_bare_uniform_path_reexport_of_a_child_module_reaches_the_effect() {
        let v = scan_crate_to_json("subreexpbare", &[
            ("src/lib.rs", "mod imp;\npub fn go() { imp::doit(); }\n"),
            ("src/imp/mod.rs", "mod unix;\npub use unix::*;\n"),
            ("src/imp/unix.rs", SPAWN),
        ]);
        assert!(effs_opt(&v, "go").contains(&"Exec".to_string()),
                "`pub use unix::*` names the DECLARED child module `imp::unix`:\n{v:#}");
    }

    /// CONTROL, GREEN BEFORE AND AFTER — 9cbd732's fabrication, in the FILE-per-module shape this change
    /// touches. A submodule's OWN `struct Command` with an empty `spawn`, in a file that also carries
    /// `use std::process::Command;`, must stay PURE: an enclosing file's imports never reach into a
    /// submodule. Narrowing a fabrication is where silent under-reports get introduced, and this change
    /// is in the same machinery, so the direction that must NOT move is pinned beside the one that must.
    #[test]
    fn a_submodules_own_type_stays_pure_across_the_reexport_change() {
        let v = scan_crate_to_json("subreexpshadow", &[
            ("src/lib.rs", "mod mine;\npub fn real() { let mut c = std::process::Command::new(\"sh\"); \
                            let _ = c.spawn(); }\n"),
            ("src/mine/mod.rs", "use std::process::Command;\n\
                                 mod inner;\n\
                                 pub use self::inner::*;\n\
                                 pub fn on_std() { let mut c = Command::new(\"sh\"); let _ = c.spawn(); }\n"),
            ("src/mine/inner.rs", "pub struct Command;\n\
                                   impl Command { pub fn spawn(&self) {} }\n\
                                   pub fn run(c: &Command) { c.spawn(); }\n"),
        ]);
        assert!(!effs_opt(&v, "mine::inner::run").contains(&"Exec".to_string()),
                "`mine::inner::Command` is the crate's OWN type and its `spawn` does nothing — \
                 charging Exec here is the fabrication 9cbd732 closed:\n{v:#}");
        assert!(effs_opt(&v, "real").contains(&"Exec".to_string()),
                "the real spawn in lib.rs still resolves:\n{v:#}");
        assert!(effs_opt(&v, "mine::on_std").contains(&"Exec".to_string()),
                "`mine/mod.rs`'s OWN import still governs its OWN code:\n{v:#}");
    }

    /// CONTROL, GREEN BEFORE AND AFTER: a re-export must not MERGE two same-named functions in
    /// different modules. `top::doit` is re-exported into `top`; `other::doit` is a different function
    /// with a different effect. Neither caller may inherit the other's effect.
    #[test]
    fn a_reexport_does_not_merge_same_named_fns_in_different_modules() {
        let v = scan_crate_to_json("subreexpmerge", &[
            ("src/lib.rs", "mod top;\nmod other;\n\
                            pub fn via_reexport() { top::doit(); }\n\
                            pub fn via_other() { other::doit(); }\n"),
            ("src/top/mod.rs", "mod platform;\npub use self::platform::*;\n"),
            ("src/top/platform.rs", SPAWN),
            ("src/other.rs", "pub fn doit() { let _ = std::fs::read_to_string(\"/etc/hosts\"); }\n"),
        ]);
        assert!(effs_opt(&v, "via_reexport").contains(&"Exec".to_string()),
                "the re-exported `top::doit` is the SPAWN:\n{v:#}");
        assert!(!effs_opt(&v, "via_reexport").contains(&"Fs".to_string()),
                "`other::doit`'s Fs must NOT smear onto the re-export route — that is the leaf-index \
                 flood one level up:\n{v:#}");
        assert!(effs_opt(&v, "via_other").contains(&"Fs".to_string()),
                "`other::doit` is the READ:\n{v:#}");
        assert!(!effs_opt(&v, "via_other").contains(&"Exec".to_string()),
                "and it must not gain the re-exported spawn:\n{v:#}");
    }

    /// CONTROL, GREEN BEFORE AND AFTER: a PRIVATE `use` (no `pub`) imports a name for the module's own
    /// body and exports NOTHING. `imp::doit()` from outside names no item, so it must resolve to
    /// nothing — an alias index that ignored visibility would answer here.
    #[test]
    fn a_private_use_does_not_reexport() {
        let v = scan_crate_to_json("subreexppriv", &[
            ("src/lib.rs", "mod imp;\npub fn go() { imp::doit(); }\n"),
            ("src/imp/mod.rs", "mod platform;\nuse self::platform::*;\n"),
            ("src/imp/platform.rs", SPAWN),
        ]);
        assert!(effs_opt(&v, "imp::platform::doit").contains(&"Exec".to_string()),
                "the definition is still analysed — this control is not vacuous:\n{v:#}");
        assert!(!effs_opt(&v, "go").contains(&"Exec".to_string()),
                "a private `use` exports nothing; `imp::doit` names no item and must resolve to \
                 nothing rather than pick up the private import:\n{v:#}");
    }

    /// CONTROL, GREEN BEFORE AND AFTER: a NAMED re-export exports exactly the name it lists. A sibling
    /// `pub fn` in the same source module is NOT visible as `imp::other`, so a call to it stays
    /// unresolved — the named form must not behave like a glob.
    #[test]
    fn a_named_reexport_does_not_export_its_modules_other_names() {
        let v = scan_crate_to_json("subreexponly", &[
            ("src/lib.rs", "mod imp;\npub fn go() { imp::other(); }\n"),
            ("src/imp/mod.rs", "mod platform;\npub use self::platform::doit;\n"),
            ("src/imp/platform.rs", &format!("{SPAWN}\
                pub fn other() {{ let _ = std::fs::read_to_string(\"/etc/hosts\"); }}\n")),
        ]);
        assert!(effs_opt(&v, "imp::platform::other").contains(&"Fs".to_string()),
                "the definition is analysed — not vacuous:\n{v:#}");
        assert!(!effs_opt(&v, "go").contains(&"Fs".to_string()),
                "`pub use self::platform::doit` lists ONE name; `imp::other` is not an item:\n{v:#}");
    }

    /// CONTROL, GREEN BEFORE AND AFTER: the CRATE-ROOT re-export (the case `collect_root_reexports`
    /// already covered) keeps working — this change adds a fallback, it must not displace the route
    /// that was already there.
    #[test]
    fn a_crate_root_reexport_still_resolves() {
        let v = scan_crate_to_json("rootreexp", &[
            ("src/lib.rs", "mod platform;\npub use self::platform::*;\npub fn go() { doit(); }\n"),
            ("src/platform.rs", SPAWN),
        ]);
        assert!(effs_opt(&v, "go").contains(&"Exec".to_string()),
                "the crate-root re-export route is pre-existing and must stay:\n{v:#}");
    }

    /// CONTROL — MEASURED AS A FABRICATION on the first cut of this fix, on the very crate it was
    /// written for. `tail2` keys a call on its last TWO segments, so `dir::imp` and `file::imp` are BOTH
    /// spelled `imp` there — and so is the call (`imp::create()` in `dir/mod.rs` and in `file/mod.rs`).
    /// tempfile has exactly that pair, and `dir::create` linked to `file::imp::*::create`, inheriting the
    /// temp-NAME generator's `Env` and `Rand` from a function that only makes a directory.
    ///
    /// The first cut DID check for two modules claiming one key — but it checked only among the aliases
    /// that SURVIVED, and `dir::imp`'s claim had already been dropped for an unrelated reason (its
    /// re-export is a `#[cfg]` PAIR, two edges, which the never-guess rule refuses). One claimant was
    /// left standing and the key looked unambiguous. A key is ambiguous because of who COULD claim it,
    /// not because of who is left after the other rules have run.
    #[test]
    fn two_modules_sharing_a_last_segment_do_not_answer_each_others_reexports() {
        let v = scan_crate_to_json("subreexpcollide", &[
            ("src/lib.rs", "mod dir;\nmod file;\n\
                            pub fn via_dir() { dir::go(); }\n\
                            pub fn via_file() { file::go(); }\n"),
            // `dir::imp` re-exports through a `#[cfg]` PAIR — two edges, so its own alias is refused.
            ("src/dir/mod.rs", "mod imp;\npub fn go() { imp::doit(); }\n"),
            ("src/dir/imp/mod.rs", "#[cfg(unix)]\nmod unix;\n#[cfg(unix)]\npub use unix::*;\n\
                                    #[cfg(not(unix))]\nmod any;\n#[cfg(not(unix))]\npub use any::*;\n"),
            ("src/dir/imp/unix.rs", "pub fn doit() { let _ = std::fs::create_dir(\"/tmp/d\"); }\n"),
            ("src/dir/imp/any.rs", "pub fn doit() { let _ = std::fs::create_dir(\"/tmp/d\"); }\n"),
            // `file::imp` re-exports through ONE glob, so its alias would otherwise stand.
            ("src/file/mod.rs", "mod imp;\npub fn go() { imp::doit(); }\n"),
            ("src/file/imp/mod.rs", "mod platform;\npub use self::platform::*;\n"),
            ("src/file/imp/platform.rs",
             "pub fn doit() { let _ = std::process::Command::new(\"sh\").spawn(); }\n"),
        ]);
        assert!(effs_opt(&v, "file::imp::platform::doit").contains(&"Exec".to_string()),
                "the definition is analysed — not vacuous:\n{v:#}");
        assert!(!effs_opt(&v, "dir::go").contains(&"Exec".to_string()),
                "`dir::imp::doit` makes a DIRECTORY. Charging it `file::imp`'s spawn is a fabrication — \
                 `imp::doit` is the same tail2 for both trees and names neither:\n{v:#}");
        assert!(!effs_opt(&v, "file::go").contains(&"Fs".to_string()),
                "…and the collision has to be refused in both directions:\n{v:#}");
    }

    /// The RENAME spelling (`pub use symbol::mangled as mangled_symbol_name;`) and the SUPER-relative
    /// one. Both appear in the corpus this fix was measured on (defmt-macros uses the first verbatim,
    /// and it is where the change's `Env` gains come from), so both are pinned rather than assumed.
    #[test]
    fn a_renamed_and_a_super_relative_reexport_both_resolve() {
        let v = scan_crate_to_json("subreexprename", &[
            ("src/lib.rs", "mod construct;\nmod other;\n\
                            pub fn via_rename() { construct::mangled_name(); }\n\
                            pub fn via_super() { other::doit(); }\n"),
            ("src/construct/mod.rs", "mod symbol;\npub use self::symbol::mangled as mangled_name;\n"),
            ("src/construct/symbol.rs", &SPAWN_AS("mangled")),
            // `other` re-exports a name from a SIBLING module by walking up through `super`.
            ("src/other/mod.rs", "pub use super::helper::doit;\n"),
            ("src/helper.rs", SPAWN),
        ]);
        assert!(effs_opt(&v, "via_rename").contains(&"Exec".to_string()),
                "`mangled_name` is `construct::symbol::mangled` under an `as`:\n{v:#}");
        assert!(effs_opt(&v, "via_super").contains(&"Exec".to_string()),
                "`super::helper::doit` names the sibling module's fn:\n{v:#}");
    }

    /// CONTROL, GREEN BEFORE AND AFTER: a module that DECLARES a name keeps it. A glob re-export bringing
    /// in a same-named function must not make `imp::doit` ambiguous or redirect it — in Rust the
    /// declaration shadows the glob, and here the primary index owns the tail outright.
    #[test]
    fn a_declared_name_is_not_displaced_by_a_glob_reexport_of_the_same_name() {
        let v = scan_crate_to_json("subreexpshadowname", &[
            ("src/lib.rs", "mod imp;\npub fn go() { imp::doit(); }\n"),
            ("src/imp/mod.rs", "mod platform;\npub use self::platform::*;\n\
                                pub fn doit() { let _ = std::fs::read_to_string(\"/etc/hosts\"); }\n"),
            ("src/imp/platform.rs", SPAWN),
        ]);
        assert!(effs_opt(&v, "go").contains(&"Fs".to_string()),
                "`imp`'s OWN `doit` is what `imp::doit()` names:\n{v:#}");
        assert!(!effs_opt(&v, "go").contains(&"Exec".to_string()),
                "the glob-imported `platform::doit` is SHADOWED by the declaration and must not be \
                 charged onto the caller:\n{v:#}");
    }

    /// GUARD-DELETION AUDIT (2026-08-30): `candor_report::resolve_sink_artifact` — which `same_artifact`
    /// (the §3.3.1 sink-collision guard) falls back to for a symlink whose target does not exist yet —
    /// had ZERO coverage anywhere in the family; deleting its whole symlink-following loop left
    /// `cargo test --workspace` fully green. `same_artifact`'s own doc comment names the exact bug this
    /// closes: "same file, different spelling" (originally a plain path-component miss, then a
    /// device+inode miss) let `--policy P --gate-json <other-spelling-of-P>` destroy the policy with
    /// `exit 0, ok: true`. A DANGLING symlink is the one shape `canonicalize` cannot resolve at all
    /// (the ordinary case both `--policy`/`--gate-json` targets already exist, so device+inode alone
    /// catches those) — this pins that both a symlink and its dangling target are one artifact.
    #[cfg(unix)]
    #[test]
    fn same_artifact_catches_a_policy_and_gate_json_collision_through_a_dangling_symlink() {
        let dir = std::env::temp_dir()
            .join(format!("candor-scan-dangling-collision-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let real = dir.join("policy.P");
        let link = dir.join("link-to-P");
        std::os::unix::fs::symlink(&real, &link).unwrap();
        assert!(!real.exists(), "setup: the symlink's target must not exist yet");

        assert!(
            crate::scan::same_artifact_pub(real.to_str().unwrap(), link.to_str().unwrap()),
            "a dangling symlink naming `real` must be recognised as the SAME artifact as `real` \
             itself — the guard `same_artifact` exists for exactly the case a naive `canonicalize` \
             comparison cannot resolve (a target that doesn't exist yet)"
        );
        // Control: an unrelated dangling symlink must NOT collide.
        let other = dir.join("policy.Q");
        assert!(
            !crate::scan::same_artifact_pub(other.to_str().unwrap(), link.to_str().unwrap()),
            "an unrelated path must not be swept in just because both are unresolvable"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    // ── R99: A NAME'S SECOND SPELLING (SOUNDNESS.md R99) ──────────────────────────────────────────
    //
    // THE ONLY DEFECT OF THE 2026-09-01 ROUND WHERE A BLANKET `deny` FAILS. Every other silent
    // under-report that day was still caught by `deny <E>` because the CALLEE stayed independently
    // reported; here NOTHING anywhere in the document carries the effect — no `Unknown`, no
    // `unresolved`, no `incomplete`, `excluded: []` — so the whole crate reads clean.
    //
    // Four spellings that give a std/dependency item a second, crate-local name, all measured ABSENT
    // from `functions[]` at 9c4d5be against an executed ground truth (each fixture below is a reduction
    // of a program that was compiled and run, and whose spawn/write was observed):
    //
    //   (1) a SUBMODULE `pub use` of an external item — `mod facade { pub use std::process::Command; }`
    //   (2) a NOMINAL type alias                       — `pub type Cmd = std::process::Command;`
    //   (3) a callable `const`/`static` bound to a fn   — `const W: fn(&str) = writer;`
    //   (4) an alias OF an alias                        — `let w = std::fs::write; let v = w; v(..)`
    //
    // Each has an INTRINSIC CONTROL one spelling away that already answered correctly — the crate-ROOT
    // `pub use`, the one-hop `let` alias, `use std::fs as f` — which is why the fix converges on the
    // existing `expand`/`uses` authority instead of adding a resolution path. The controls are asserted
    // beside every fixture: they are what makes this precise rather than a guess.

    /// R99 (1) — a submodule `pub use` of a std item, INLINE in the same file, with the crate-ROOT
    /// spelling of the identical thing as the control. Pre-fix: `via_facade` absent, `ctl_root` present.
    #[test]
    fn r99_submodule_pub_use_of_a_std_item_is_not_lost() {
        let v = scan_src_to_json("r99facade", concat!(
            "mod facade { pub use std::process::Command; }\n",
            "pub use std::process::Command as RootCmd;\n",
            "pub fn via_facade() { let _ = facade::Command::new(\"/bin/sh\").status(); }\n",
            "pub fn ctl_root() { let _ = RootCmd::new(\"/bin/sh\").status(); }\n",
        ));
        assert_eq!(
            effs_opt(&v, "via_facade"), vec!["Exec".to_string()],
            "R99: a call through a SUBMODULE `pub use` of `std::process::Command` must charge Exec — \
             pre-fix `via_facade` was absent from functions[] entirely and a blanket `deny` exited 0:\n{v:#}"
        );
        assert_eq!(
            effs_opt(&v, "ctl_root"), vec!["Exec".to_string()],
            "INTRINSIC CONTROL: the crate-ROOT spelling already resolved before this fix and must \
             still resolve — it is the path the fix converges ON, not a new one:\n{v:#}"
        );
    }

    /// R99 (1), FILE-PER-MODULE and CROSS-FILE — the shape real facade/prelude modules have. Three
    /// caller spellings, all measured absent pre-fix: from an ANCESTOR module (`facade::Command`), fully
    /// `crate::`-rooted, and through a module re-bind (`use crate::facade;`) in a SIBLING module.
    #[test]
    fn r99_cross_file_facade_reexport_resolves_from_every_caller_spelling() {
        let v = scan_crate_to_json("r99xfile", &[
            ("src/lib.rs", "pub mod facade;\npub mod other;\n\
                            pub fn from_root() { let _ = facade::Command::new(\"/bin/sh\").status(); }\n"),
            ("src/facade.rs", "pub use std::process::Command;\n"),
            ("src/other.rs", "use crate::facade;\n\
                              pub fn sib() { let _ = facade::Command::new(\"/bin/sh\").status(); }\n\
                              pub fn rooted() { let _ = crate::facade::Command::new(\"/bin/sh\").status(); }\n"),
        ]);
        for f in ["from_root", "other::sib", "other::rooted"] {
            assert_eq!(
                effs_opt(&v, f), vec!["Exec".to_string()],
                "R99: `{f}` reaches std's Command through a cross-file facade re-export and must charge \
                 Exec — pre-fix every one of these three spellings was absent:\n{v:#}"
            );
        }
    }

    /// R99 (2) — a NOMINAL type alias, used in the declaring module and from another file through
    /// `crate::`. `decls.rs` recorded an `Item::Type` only when the target is NON-nominal
    /// (`prim_aliases`, a resolution SKIP); `pub type Client = reqwest::Client` recorded nothing.
    #[test]
    fn r99_nominal_type_alias_carries_its_targets_effects() {
        let v = scan_crate_to_json("r99alias", &[
            ("src/lib.rs", "pub mod other;\n\
                            pub type Cmd = std::process::Command;\n\
                            pub fn via_alias() { let _ = Cmd::new(\"/bin/sh\").status(); }\n"),
            ("src/other.rs", "pub fn via_rooted_alias() { let _ = crate::Cmd::new(\"/bin/sh\").status(); }\n"),
        ]);
        assert_eq!(
            effs_opt(&v, "via_alias"), vec!["Exec".to_string()],
            "R99: `pub type Cmd = std::process::Command` must resolve exactly as `use … as Cmd` does \
             — both live in the TYPE namespace, so no module can declare both spellings:\n{v:#}"
        );
        assert_eq!(
            effs_opt(&v, "other::via_rooted_alias"), vec!["Exec".to_string()],
            "R99: …and from another file through `crate::Cmd`:\n{v:#}"
        );
    }

    /// R99 (2) OVER-CHARGE CONTROL — the direction the fix did NOT intend. A root `pub type Client =
    /// std::process::Command` must NOT reach a SUBMODULE that declares its own `Client`: a bare name is
    /// never bound crate-wide (the rule `seed_root_reexports` established), only `crate::<qualified>`
    /// and the spellings genuinely in scope. Without this the fix would trade a silent under-report for
    /// the misattribution mirror — R101's direction, which is worse.
    ///
    /// R107 — THIS CONTROL COULD NOT FAIL, AND THAT IS WHY THE HOLE BESIDE IT SHIPPED. As written it
    /// asserted `is_empty()` over bodies that were empty (`fn new() -> Self { Client }`, `fn go(&self) {}`),
    /// so it held whether or not the alias hijacked the name — an absence assertion over a program with
    /// nothing to be absent (brief §E3). And its rival sat in a FILE submodule, a position the bare seed
    /// STRUCTURALLY cannot reach: `seed_mod_aliases` binds the bare name only where `module == modpath`,
    /// and the root alias's module is `""` while this file's is `local`. Two independent reasons to pass,
    /// neither of them the one it claimed. Now: the local `Client` performs a DISTINCT effect class (Fs)
    /// from the alias target (Exec), so the assertion is `["Fs"]` — a claim that discriminates — and the
    /// position the seed actually lands in has its own test, `r106_a_body_local_item_wins_over_a_seeded_alias`.
    #[test]
    fn r99_a_root_type_alias_does_not_reach_a_submodules_own_same_named_type() {
        let v = scan_crate_to_json("r99shadow", &[
            ("src/lib.rs", "pub mod local;\npub type Client = std::process::Command;\n"),
            ("src/local.rs", "pub struct Client;\n\
                              impl Client {\n\
                                pub fn new(_p: &str) -> Self { let _ = std::fs::write(\"/tmp/r99shadow\", \"x\"); Client }\n\
                                pub fn status(&self) -> u32 { 0 }\n\
                              }\n\
                              pub fn uses_local_client() { let c = Client::new(\"/bin/sh\"); let _ = c.status(); }\n"),
        ]);
        assert_eq!(
            effs_opt(&v, "local::uses_local_client"), vec!["Fs".to_string()],
            "OVER-CHARGE CONTROL: a submodule's OWN `Client` must resolve to the LOCAL type — its `new` \
             writes a file (Fs) and nothing here spawns anything. An `Exec` (or an `Exec`+`Fs`) means the \
             root alias reached a name it does not bind:\n{v:#}"
        );
    }

    /// R105 — a `#[cfg]`-DUPLICATED ALIAS MUST NOT BE RESOLVED BY SOURCE ORDER.
    ///
    /// Two crates identical but for the ORDER of the two `#[cfg]` arms. On unix both programs call
    /// `std::fs::write`; EXECUTED ground truth for both is one file written and no environment variable
    /// set. Pre-fix the last arm in the file won: arm order A reported `["Fs"]` and failed `deny Fs`
    /// (exit 1), arm order B reported `["Env"]` and PASSED it (exit 0) — a fabricated effect class, and
    /// behind it the real `Fs` present nowhere in the document (no `Unknown`, no `unresolved`, no
    /// `incomplete`, `excluded: []`), which is the cardinal sin.
    ///
    /// The assertion is ORDER-INVARIANCE plus honesty: both orders must give the SAME answer, and that
    /// answer must be a disclosure, because the two arms classify differently (Fs vs Env) and candor
    /// cannot know which platform the build selects. Charging one would be the guess; charging both would
    /// fabricate the arm that is not compiled.
    #[test]
    fn r105_a_cfg_duplicated_alias_is_not_decided_by_arm_order() {
        let src = |first: &str, second: &str| {
            format!(
                "mod sys {{\n  #[cfg({first})] pub use std::{}::{} as put;\n  \
                 #[cfg({second})] pub use std::{}::{} as put;\n}}\n\
                 pub fn go() {{ let _ = sys::put(\"/tmp/r105\", \"x\"); }}\n",
                if first == "unix" { "fs" } else { "env" },
                if first == "unix" { "write" } else { "set_var" },
                if first == "unix" { "env" } else { "fs" },
                if first == "unix" { "set_var" } else { "write" },
            )
        };
        let a = scan_src_to_json("r105a", &src("unix", "not(unix)"));
        let b = scan_src_to_json("r105b", &src("not(unix)", "unix"));
        assert_eq!(
            effs_opt(&a, "go"), effs_opt(&b, "go"),
            "R105: two crates differing ONLY in `#[cfg]` arm ORDER answered differently — the alias map \
             resolved by source position:\nA:\n{a:#}\nB:\n{b:#}"
        );
        for (tag, v) in [("A", &a), ("B", &b)] {
            assert_eq!(
                effs_opt(v, "go"), vec!["Unknown".to_string()],
                "R105 ({tag}): the arms classify differently (Fs vs Env), so the honest answer is \
                 `Unknown` — never one arm picked, never both charged:\n{v:#}"
            );
            let why: Vec<String> = v["functions"]
                .as_array()
                .into_iter()
                .flatten()
                .find(|f| f["fn"] == "go")
                .and_then(|f| f["unknownWhy"].as_array().cloned())
                .unwrap_or_default()
                .iter()
                .filter_map(|r| r.as_str().map(str::to_string))
                .collect();
            assert!(
                why.iter().any(|r| r.starts_with("ambiguous:")),
                "R105 ({tag}): the disclosure must carry a reason, and the SPEC §4 kind for \
                 `two candidates, cannot say which runs` is `ambiguous:`:\n{v:#}"
            );
        }
    }

    /// R105 UNION CONTROL — the cost side, and the reason this fix does not make 6.6% of a real corpus
    /// noisier. When the duplicated arms ANSWER THE SAME, nothing is disclosed:
    ///
    ///   * `same_target`  — both arms name the identical item (the ordinary `cfg_if` re-export). Not an
    ///     ambiguity at all; the full surface survives, `fs: ["write"]` included.
    ///   * `both_fs`      — two DIFFERENT paths that classify identically. The agreed effect is charged
    ///     (this is the union), the literal surface is withheld because the two arms' literals are
    ///     different claims, and the effect is marked `incomplete` so an empty surface cannot read as a
    ///     complete one.
    ///   * `inert`        — the portability shim that dominates the real corpus (`std::boxed::Box` vs
    ///     `alloc::boxed::Box`). Neither arm classifies, so the collision costs nothing at all.
    #[test]
    fn r105_arms_that_classify_alike_are_unioned_not_disclosed() {
        let v = scan_src_to_json("r105union", concat!(
            "mod s1 { #[cfg(unix)] pub use std::fs::write as put;\n",
            "         #[cfg(not(unix))] pub use std::fs::write as put; }\n",
            "pub fn same_target() { let _ = s1::put(\"/tmp/a\", \"x\"); }\n",
            "mod s2 { #[cfg(unix)] pub use std::fs::write as put;\n",
            "         #[cfg(not(unix))] pub use std::fs::remove_file as put; }\n",
            "pub fn both_fs() { let _ = s2::put(\"/tmp/a\"); }\n",
            "mod s3 { #[cfg(feature = \"std\")] pub use std::boxed::Box as B;\n",
            "         #[cfg(not(feature = \"std\"))] pub use alloc::boxed::Box as B; }\n",
            "pub fn inert() { let _b = s3::B::new(1u8); }\n",
        ));
        assert_eq!(
            effs_opt(&v, "same_target"), vec!["Fs".to_string()],
            "R105 CONTROL: two arms naming the IDENTICAL item are one answer recorded twice — not an \
             ambiguity, and not a reason to lose the surface:\n{v:#}"
        );
        assert_eq!(
            effs_opt(&v, "both_fs"), vec!["Fs".to_string()],
            "R105 UNION: arms that classify alike charge that effect — `Unknown` here would be the \
             over-charge this fix is measured against:\n{v:#}"
        );
        assert!(
            effs_opt(&v, "inert").is_empty(),
            "R105 CONTROL: a `std`/`alloc` portability shim classifies to nothing on BOTH arms, so the \
             collision must cost nothing — this is the shape 17 of 256 real crates carry:\n{v:#}"
        );
    }

    /// R105 CONTROL — a SINGLE-arm alias (no duplication) is untouched. This is the whole of R99's own
    /// behaviour, and the branch this fix adds must be unreachable for it.
    #[test]
    fn r105_a_single_arm_alias_is_unchanged() {
        let v = scan_src_to_json("r105single", concat!(
            "mod facade { pub use std::process::Command; }\n",
            "pub fn go() { let _ = facade::Command::new(\"/bin/sh\").status(); }\n",
        ));
        assert_eq!(
            effs_opt(&v, "go"), vec!["Exec".to_string()],
            "R105 CONTROL: an alias with ONE declaration resolves exactly as before:\n{v:#}"
        );
    }

    /// R106 — A BODY-LOCAL ITEM SHADOWS A FILE-LEVEL BINDING OF THE SAME NAME.
    ///
    /// EXECUTED ground truth (the fixture was compiled and run): `body_local_item` writes one file and
    /// spawns nothing. Pre-fix it reported `["Exec","Fs"]` with `cmds: ["true"]` and failed `deny Exec` at
    /// exit 1 — a fabricated effect AND a fabricated command surface, because the body's own `struct Cmd`
    /// did not shadow the file's `Cmd` binding and `Cmd::new("true").spawn()` resolved to
    /// `std::process::Command`.
    ///
    /// BOTH SPELLINGS OF THE BINDING ARE TESTED, and that is the point. The `pub type` half arrives
    /// through the alias seed `b00956b` added; the plain `use std::process::Command;` half is the SAME
    /// hole and PREDATES it — measured identically at HEAD. Fixing only the seeded route would have left
    /// the commoner spelling silent, which is the audit boundary drawn around its own trigger.
    #[test]
    fn r106_a_body_local_item_wins_over_a_seeded_alias() {
        for (tag, binding) in [
            ("type-alias", "pub type Cmd = std::process::Command;"),
            ("plain-use", "use std::process::Command as Cmd;"),
        ] {
            let v = scan_src_to_json("r106body", &format!(
                "{binding}\n\
                 pub fn body_local_item() {{\n\
                   struct Cmd;\n\
                   impl Cmd {{\n\
                     fn new(_: &str) -> Self {{ let _ = std::fs::write(\"/tmp/r106\", \"x\"); Cmd }}\n\
                     fn spawn(&self) -> u32 {{ 0 }}\n\
                   }}\n\
                   let _ = Cmd::new(\"true\").spawn();\n\
                 }}\n"
            ));
            assert_eq!(
                effs_opt(&v, "body_local_item"), vec!["Fs".to_string()],
                "R106 ({tag}): the body declares its OWN `Cmd`, so `Cmd::new(..).spawn()` is the local \
                 one — which writes a file and spawns nothing. `Exec` here is a fabricated effect on a \
                 path that provably performs none:\n{v:#}"
            );
            assert!(
                v.to_string().find("\"true\"").is_none(),
                "R106 ({tag}): `cmds: [\"true\"]` is a fabricated EXEC SURFACE — an `allow Exec true` \
                 would then certify a program that runs nothing:\n{v:#}"
            );
        }
    }

    /// R106 CONTROL — the shadow must not go the other way. A body that does NOT declare the name keeps
    /// resolving it through the file-level binding (that is R99, and it must survive), and an INLINE
    /// SUBMODULE's own item keeps winning (that is `submodule_uses`, which already worked). Distinct
    /// effect classes on the two sides — Exec for the alias target, Fs for the local type — so neither
    /// assertion can hold for the wrong reason.
    #[test]
    fn r106_the_body_shadow_does_not_reach_past_the_body() {
        let v = scan_src_to_json("r106ctl", concat!(
            "pub type Cmd = std::process::Command;\n",
            "pub fn no_local_decl() { let _ = Cmd::new(\"/bin/sh\").status(); }\n",
            "pub mod inner {\n",
            "  pub struct Cmd;\n",
            "  impl Cmd {\n",
            "    pub fn new(_: &str) -> Self { let _ = std::fs::write(\"/tmp/r106c\", \"x\"); Cmd }\n",
            "    pub fn status(&self) -> u32 { 0 }\n",
            "  }\n",
            "  pub fn go() { let _ = Cmd::new(\"true\").status(); }\n",
            "}\n",
        ));
        assert_eq!(
            effs_opt(&v, "no_local_decl"), vec!["Exec".to_string()],
            "R106 CONTROL: a body that declares nothing must still resolve `Cmd` through the file's \
             alias — the body shadow must not cost R99 its fix:\n{v:#}"
        );
        assert_eq!(
            effs_opt(&v, "inner::go"), vec!["Fs".to_string()],
            "R106 CONTROL: an INLINE submodule's own `Cmd` already won (`submodule_uses`) and must \
             keep winning:\n{v:#}"
        );
    }

    /// R106 — SHADOWING MUST NOT HAND THE NAME TO A DIFFERENT CRATE-LEVEL DEFINITION. This is the
    /// fixture that falsified the first cut of the fix, which merely REMOVED the shadowed binding on
    /// `submodule_uses`' stated argument that "removing an inherited binding can never invent an effect".
    /// For a function body it can: `W::make()` then resolved MODULE-RELATIVE and the tail2 index linked it
    /// to the unrelated `other::W::make`, so `body_pure` went from a fabricated `Exec` (the R106 defect)
    /// to a fabricated `Fs` (the fix's own). Both are wrong; `body_pure` performs nothing.
    ///
    /// Three arms, one variable each: the shadowing body must be PURE, the body-local item's OWN effect
    /// must still reach its caller (the shadow must not silence the code that actually runs), and a
    /// function in the same file that does NOT shadow must keep the import.
    #[test]
    fn r106_a_body_local_shadow_does_not_reroute_to_another_definition() {
        let v = scan_src_to_json("r106reroute", concat!(
            "pub mod other {\n",
            "  pub struct W;\n",
            "  impl W { pub fn make() -> Self { let _ = std::fs::write(\"/tmp/r106r\", \"x\"); W } }\n",
            "}\n",
            "use std::process::Command as W;\n",
            "pub fn body_pure() { struct W; impl W { fn make() -> Self { W } } let _ = W::make(); }\n",
            "pub fn body_effectful() {\n",
            "  struct W;\n",
            "  impl W { fn make() -> Self { let _ = std::fs::write(\"/tmp/r106e\", \"y\"); W } }\n",
            "  let _ = W::make();\n",
            "}\n",
            "pub fn keeps_alias() { let _ = W::new(\"/bin/sh\").status(); }\n",
        ));
        assert!(
            effs_opt(&v, "body_pure").is_empty(),
            "R106: the body's OWN `W::make` does nothing. `Exec` means the import hijacked the name; \
             `Fs` means shadowing rerouted it to `other::W::make`, an unrelated definition that does \
             not run here:\n{v:#}"
        );
        assert_eq!(
            effs_opt(&v, "body_effectful"), vec!["Fs".to_string()],
            "R106 CONTROL: the shadow must not SILENCE the item it shadows in favour of — the body's \
             own effectful `make` is what runs, and its caller must carry it:\n{v:#}"
        );
        assert_eq!(
            effs_opt(&v, "keeps_alias"), vec!["Exec".to_string()],
            "R106 CONTROL: the shadow is scoped to the body that declares it — a sibling function in \
             the same file still resolves `W` through the import:\n{v:#}"
        );
    }

    /// R119 (CARDINAL SIN, introduced by R106's own fix) — A NESTED BLOCK'S ITEM MUST NOT SHADOW THE
    /// WHOLE FUNCTION.
    ///
    /// `body_declared_items` walked the entire body with `visit_block`, so an item declared in a nested
    /// block rebound its name to the sentinel for every statement of the function, not just that block's.
    /// EXECUTED ground truth (compiled and run, prints `spawned=true`): each arm below spawns
    /// `/usr/bin/true`, and each went ABSENT from `functions[]` — a silent under-report, with `deny Exec`
    /// dropping from exit 1 to exit 0 on a single-function crate.
    ///
    /// THE ARMS ARE THE AUDIT, NOT THE TRIGGER. The reported instance was a plain `{ }`; every construct
    /// below introduces a block scope by a different syntactic route and all fifteen were measured silent
    /// against a pre-fix binary. The three positions that were ALREADY correct — a nested `fn` body, an
    /// `impl` block, an inline `mod` — are the last three arms, so this test also pins the boundary rather
    /// than assuming it.
    #[test]
    fn r109_a_nested_block_item_does_not_shadow_the_whole_body() {
        // Each snippet declares `struct Cmd` inside a scope of its own. The `Cmd::new(p).status()` that
        // decides the row sits OUTSIDE that scope, so the only correct answer is `Exec`.
        for (tag, snippet) in [
            ("plain-block", "let _n = { struct Cmd { n: u32 } Cmd { n: 1 }.n };"),
            ("if", "if ok { struct Cmd { n: u32 } let _ = Cmd { n: 1 }.n; }"),
            ("else", "if !ok { let _ = 0; } else { struct Cmd { n: u32 } let _ = Cmd { n: 1 }.n; }"),
            ("match-arm", "match ok { true => { struct Cmd { n: u32 } let _ = Cmd { n: 1 }.n; }, false => {} }"),
            ("if-let", "if let Some(_x) = Some(1u32) { struct Cmd { n: u32 } let _ = Cmd { n: 1 }.n; }"),
            ("while-let", "let mut it = vec![1u32].into_iter();\n\
                           while let Some(_x) = it.next() { struct Cmd { n: u32 } let _ = Cmd { n: 1 }.n; }"),
            ("loop", "loop { struct Cmd { n: u32 } let _ = Cmd { n: 1 }.n; break; }"),
            ("for", "for _i in 0..1u32 { struct Cmd { n: u32 } let _ = Cmd { n: 1 }.n; }"),
            ("while", "let mut k = 0u32;\n\
                       while k < 1 { struct Cmd { n: u32 } let _ = Cmd { n: 1 }.n; k += 1; }"),
            ("closure", "let f = || { struct Cmd { n: u32 } Cmd { n: 1 }.n }; let _ = f();"),
            ("async-block", "let _fut = async { struct Cmd { n: u32 } Cmd { n: 1 }.n };"),
            ("unsafe-block", "let _n = unsafe { struct Cmd { n: u32 } Cmd { n: 1 }.n };"),
            ("const-init", "const K: u32 = { struct Cmd { n: u32 } Cmd { n: 1 }.n }; let _ = K;"),
            ("static-init", "static S: u32 = { struct Cmd { n: u32 } Cmd { n: 1 }.n }; let _ = S;"),
            ("labelled-block", "let _n = 'lbl: { struct Cmd { n: u32 } break 'lbl Cmd { n: 1 }.n; };"),
            // The three that were already right — kept here so the boundary is pinned, not assumed.
            ("nested-fn", "fn inner() -> u32 { struct Cmd { n: u32 } Cmd { n: 1 }.n } let _ = inner();"),
            ("impl-block", "struct Q; impl Q { fn m() -> u32 { struct Cmd { n: u32 } Cmd { n: 1 }.n } } let _ = Q::m();"),
            ("inline-mod", "mod m { pub fn g() -> u32 { struct Cmd { n: u32 } Cmd { n: 1 }.n } } let _ = m::g();"),
        ] {
            let v = scan_src_to_json("r109nested", &format!(
                "use std::process::Command as Cmd;\n\
                 pub fn spawn_it(p: &str) -> bool {{\n\
                   let ok = Cmd::new(p).status().map(|s| s.success()).unwrap_or(false);\n\
                   {snippet}\n\
                   ok\n\
                 }}\n"
            ));
            assert_eq!(
                effs_opt(&v, "spawn_it"), vec!["Exec".to_string()],
                "R119 ({tag}): the `struct Cmd` is declared in a SEPARATE scope, so the body's own \
                 `Cmd::new(p).status()` is `std::process::Command` and provably runs a process. An empty \
                 answer here is a silent under-report, not a pure function:\n{v:#}"
            );
        }
    }

    /// R119 — THE SIGNATURE IS NOT IN THE BODY'S SCOPE. A second position the flattened shadow reached,
    /// and it is not a nested-block instance: a body-level item legitimately shadows for the whole body,
    /// but a PARAMETER's type resolves where the function is declared. R106 applied the sentinel to one
    /// map used for both, so `seed_vars` typed the receiver as `<body-item>Cmd` and the row vanished.
    ///
    /// EXECUTED ground truth: `sig_param` spawns `/usr/bin/true`. The control differs in exactly one
    /// thing — the body item's NAME — and reported `["Exec"]` throughout.
    #[test]
    fn r109_a_body_item_does_not_shadow_the_signature() {
        let v = scan_src_to_json("r109sig", concat!(
            "use std::process::Command as Cmd;\n",
            "pub fn sig_param(c: &mut Cmd) -> bool {\n",
            "  struct Cmd { n: u32 }\n",
            "  let _ = Cmd { n: 1 }.n;\n",
            "  c.status().map(|s| s.success()).unwrap_or(false)\n",
            "}\n",
            "pub fn sig_ctl(c: &mut Cmd) -> bool {\n",
            "  struct Helper { n: u32 }\n",
            "  let _ = Helper { n: 1 }.n;\n",
            "  c.status().map(|s| s.success()).unwrap_or(false)\n",
            "}\n",
        ));
        assert_eq!(
            effs_opt(&v, "sig_param"), vec!["Exec".to_string()],
            "R119: `c: &mut Cmd` names the file's import — a body-local `struct Cmd` cannot rebind a \
             parameter's type, and `c.status()` provably runs a process:\n{v:#}"
        );
        assert_eq!(
            effs_opt(&v, "sig_ctl"), vec!["Exec".to_string()],
            "R119 CONTROL: the same function with the body item RENAMED — the two arms differ in nothing \
             else, so a difference between them is the shadow and nothing else:\n{v:#}"
        );
    }

    /// R119 CONTROL — THE R106 PRECISION GAIN MUST SURVIVE THE SCOPING. Where a nested block declares a
    /// name and is the ONLY place in the function that mentions it, function-wide shadowing and true block
    /// scoping are indistinguishable, so the shadow is still applied. EXECUTED ground truth: `nested_only`
    /// returns 7 and runs no process — pre-R106 this fabricated `Exec`.
    ///
    /// SECOND ARM: THE STATED RESIDUAL, PINNED. With the SAME name declared by two SIBLING blocks, no
    /// single block contains every occurrence, so no promotion happens and the import stays in scope —
    /// `two_blocks` keeps the pre-R106 FABRICATION. (The rule declines to promote for ANY two declaring
    /// blocks, including a nested pair where promotion would in fact have been exact; that is a
    /// deliberate simplification, and it errs by over-reporting.) Asserted here so that "improving" it
    /// into an empty answer fails loudly rather than quietly.
    #[test]
    fn r109_a_nested_item_used_only_in_its_own_block_still_shadows() {
        let v = scan_src_to_json("r109prec", concat!(
            "use std::process::Command as Cmd;\n",
            "pub fn nested_only() -> u32 {\n",
            "  let n = {\n",
            "    struct Cmd { n: u32 }\n",
            "    impl Cmd { fn new(_: &str) -> Self { Cmd { n: 7 } } fn status(&self) -> u32 { self.n } }\n",
            "    Cmd::new(\"/usr/bin/true\").status()\n",
            "  };\n",
            "  n\n",
            "}\n",
            "pub fn two_blocks() -> u32 {\n",
            "  let a = {\n",
            "    struct Cmd { n: u32 }\n",
            "    impl Cmd { fn new(_: &str) -> Self { Cmd { n: 1 } } fn status(&self) -> u32 { self.n } }\n",
            "    Cmd::new(\"/usr/bin/true\").status()\n",
            "  };\n",
            "  let b = {\n",
            "    struct Cmd { n: u32 }\n",
            "    impl Cmd { fn new(_: &str) -> Self { Cmd { n: 2 } } fn status(&self) -> u32 { self.n } }\n",
            "    Cmd::new(\"/usr/bin/true\").status()\n",
            "  };\n",
            "  a + b\n",
            "}\n",
        ));
        assert!(
            effs_opt(&v, "nested_only").is_empty(),
            "R119 CONTROL: every mention of `Cmd` in this body is inside the block that declares it, so \
             the shadow is exact — `Exec` here is R106's fabrication on a path that runs nothing:\n{v:#}"
        );
        assert_eq!(
            effs_opt(&v, "two_blocks"), vec!["Exec".to_string()],
            "R119 RESIDUAL: two sibling blocks declare the name, so no function-wide promotion is sound \
             and the import is left in scope. This is a known OVER-report on a pure path — pinned so the \
             cheap 'fix' of shadowing anyway, which is the cardinal-sin direction, cannot land quietly:\n{v:#}"
        );
    }

    /// R107 — the SELF-SHADOWING TUPLE DESTRUCTURE, the read that escaped the R100 window.
    ///
    /// `tuple_elem_leaves` reads the source element's dispatch typing, and the binding loop above it has
    /// already cleared `trait_vars` for the name it is binding. So `let (d, n) = (d, n);` asked about a
    /// `d` this statement had just deleted, found nothing, and `d.go()` dropped SILENT — absent from
    /// `functions[]`, which is the cardinal sin's signature. The two arms differ in exactly one thing:
    /// whether the destructured names shadow the source names.
    #[test]
    fn r107_a_self_shadowing_tuple_destructure_keeps_its_dispatch_typing() {
        let v = scan_src_to_json("r107tuple", concat!(
            "pub trait Doer { fn go(&self); }\n",
            "pub struct Real;\n",
            "impl Doer for Real { fn go(&self) { let _ = std::fs::write(\"/tmp/r107\", \"x\"); } }\n",
            "pub fn shadowed(d: Box<dyn Doer>, n: u32) { let (d, n) = (d, n); d.go(); let _ = n; }\n",
            "pub fn control(d: Box<dyn Doer>, n: u32) { let (e, m) = (d, n); e.go(); let _ = m; }\n",
        ));
        for f in ["shadowed", "control"] {
            assert_eq!(
                effs_opt(&v, f), vec!["Fs".to_string()],
                "R107: `{f}` dispatches to the one visible `Doer` impl, which writes a file. The \
                 SHADOWING arm went silent pre-fix while the renamed one resolved — the RHS of a `let` \
                 means the PRE-statement binding:\n{v:#}"
            );
        }
    }

    /// R107 — a REBIND FROM A CLOSURE BINDING keeps the closure marking. PRE-EXISTING (identical at
    /// `9c4d5be`), found by the same audit: `let eff = || {}; let eff = eff;` dropped the marking, so
    /// `eff()` stopped being a visible closure call and resolved by bare leaf to the free `fn eff` beside
    /// it — a positive `Fs` claim about a body whose only call is to an empty closure.
    #[test]
    fn r107_a_rebind_from_a_closure_binding_stays_a_closure() {
        let v = scan_src_to_json("r107closure", concat!(
            "pub fn eff() { let _ = std::fs::write(\"/tmp/r107c\", \"x\"); }\n",
            "pub fn shadowed() { let eff = || {}; let eff = eff; eff(); }\n",
            "pub fn renamed() { let c = || {}; let d = c; d(); }\n",
        ));
        for f in ["shadowed", "renamed"] {
            assert!(
                effs_opt(&v, f).is_empty(),
                "R107: `{f}` calls an EMPTY closure and nothing else — charging the free `fn eff`'s Fs \
                 is a fabrication on a provably-pure body:\n{v:#}"
            );
        }
    }


    /// R99 (2) CONTROL — a NON-nominal alias must keep going to `prim_aliases` (the resolution SKIP that
    /// stops `Inner::default()` inheriting a same-named local struct's effects, the sled `IVec`
    /// fabrication). The nominal arm added here must not swallow it.
    #[test]
    fn r99_a_non_nominal_alias_still_takes_the_prim_alias_skip() {
        let v = scan_src_to_json("r99prim", concat!(
            "pub struct Inner;\n",
            "impl Default for Inner { fn default() -> Self { let _ = std::env::var(\"HOME\"); Inner } }\n",
            "pub type Buf = [u8; 4];\n",
            "pub fn pure_path() -> Buf { Buf::default() }\n",
        ));
        assert!(
            effs_opt(&v, "pure_path").is_empty(),
            "CONTROL: `type Buf = [u8; 4]` is NON-nominal — it must still reach `prim_aliases` and skip \
             the same-named local struct's effectful `Default`, not be recorded as a nominal alias:\n{v:#}"
        );
    }

    /// R99 (3) — a `const`/`static` of CALLABLE type bound to a fn item. Gated on both halves (callable
    /// declared type AND a bare-path initializer), so it can only ever name a function.
    #[test]
    fn r99_a_callable_const_bound_to_a_fn_item_is_not_lost() {
        let v = scan_src_to_json("r99const", concat!(
            "fn writer(p: &str) { let _ = std::fs::write(p, \"x\"); }\n",
            "const WRITER: fn(&str) = writer;\n",
            "static SWRITER: fn(&str) = writer;\n",
            "pub fn via_const() { WRITER(\"/tmp/x\"); }\n",
            "pub fn via_static() { SWRITER(\"/tmp/x\"); }\n",
        ));
        for f in ["via_const", "via_static"] {
            assert_eq!(
                effs_opt(&v, f), vec!["Fs".to_string()],
                "R99: `{f}` calls a fn item held in a callable-typed const/static and must charge Fs — \
                 pre-fix it was absent from functions[]:\n{v:#}"
            );
        }
    }

    /// R99 (3) CONTROL — a const that is NOT callable-typed, and a callable-typed const whose
    /// initializer is not a bare path, must both record nothing. (`const_strings` and the lazy-static
    /// route own those; a second answer here is what §G forbids.)
    #[test]
    fn r99_a_non_callable_const_records_no_alias() {
        let v = scan_src_to_json("r99constctl", concat!(
            "fn writer(p: &str) { let _ = std::fs::write(p, \"x\"); }\n",
            "const NAME: &str = \"writer\";\n",
            "pub fn uses_name() -> &'static str { NAME }\n",
            "pub fn real_call() { writer(\"/tmp/x\"); }\n",
        ));
        assert!(
            effs_opt(&v, "uses_name").is_empty(),
            "CONTROL: a `&str` const named after a fn must NOT alias to it:\n{v:#}"
        );
        assert_eq!(
            effs_opt(&v, "real_call"), vec!["Fs".to_string()],
            "CONTROL: the direct call must be unaffected:\n{v:#}"
        );
    }

    /// R99 (4) — `fn_alias` did not CHAIN. `let w = std::fs::write; let v = w; v(..)` was absent in BOTH
    /// the shadowing and non-shadowing spellings while the ONE-hop form (the control) has resolved since
    /// sweep [6]; `let v = w` bound `v` to the dead string `w`, which names no fn.
    #[test]
    fn r99_a_fn_alias_of_a_fn_alias_chains() {
        let v = scan_src_to_json("r99chain", concat!(
            "pub fn two_hop() { let w = std::fs::write; let v = w; let _ = v(\"/tmp/x\", \"y\"); }\n",
            "pub fn two_hop_shadow() { let w = std::fs::write; let w = w; let _ = w(\"/tmp/x\", \"y\"); }\n",
            "pub fn ctl_one_hop() { let w = std::fs::write; let _ = w(\"/tmp/x\", \"y\"); }\n",
        ));
        for f in ["two_hop", "two_hop_shadow"] {
            assert_eq!(
                effs_opt(&v, f), vec!["Fs".to_string()],
                "R99: `{f}` writes a file through a two-hop fn alias and must charge Fs:\n{v:#}"
            );
        }
        assert_eq!(
            effs_opt(&v, "ctl_one_hop"), vec!["Fs".to_string()],
            "INTRINSIC CONTROL: the one-hop alias already resolved and must still:\n{v:#}"
        );
    }

    /// R99 CONTROL — a plain external call must keep its own crate identity. The module-qualified alias
    /// lookup runs AFTER the single-segment `use` route in the bare-qualifier branch precisely so a file
    /// that binds the head itself (`use somecrate::facade;`) still means ITS `facade`; and a path with
    /// no alias entry at all must come back unchanged.
    #[test]
    fn r99_a_plain_external_call_keeps_its_own_crate_identity() {
        let mut uses: HashMap<String, String> = HashMap::new();
        uses.insert("facade".into(), "somecrate::facade".into());
        uses.insert("facade::Command".into(), "std::process::Command".into());
        assert_eq!(
            expand("facade::Command::new", &uses), "somecrate::facade::Command::new",
            "a file that BINDS the head must keep that binding — the alias map answers only where the \
             qualifier names a module of THIS crate"
        );
        let bare: HashMap<String, String> = HashMap::new();
        assert_eq!(
            expand("serde_json::from_str", &bare), "serde_json::from_str",
            "an unaliased external path must come back unchanged"
        );
    }

    // ── R99's TWO STATED-OPEN SHAPES, NOW CLOSED (SOUNDNESS.md R99) ───────────────────────────────
    //
    // b00956b closed four alias mechanisms and STATED two residuals rather than guessing at them: a
    // SUBMODULE GLOB re-export of an external item, and Pass A's decl indexes not seeing `mod_aliases`.
    // Both were given syscall-oracle drivers at `1aeeaba` (`pf_alias_glob`, `pf_alias_field`) with paired
    // one-variable controls that PASS, so the failure is the engine and not the harness. Both drivers were
    // RED — `pf_alias_glob` reporting `functions: []` over a program strace watched write a file.

    /// R99 (SHAPE 1) — a SUBMODULE GLOB re-export of a std module. `collect_reexports`' external branch
    /// skipped `name == "*"`; the whole crate then read clean (`functions: []`, `excluded: []`, no
    /// disclosure channel set anywhere) while the program wrote a file. The paired control — the same
    /// module re-exporting the SAME item BY NAME — resolved before this fix and must still resolve.
    #[test]
    fn r99_submodule_glob_reexport_of_an_external_module_resolves() {
        let v = scan_crate_to_json("r99glob", &[
            ("src/lib.rs", "pub mod glb;\npub mod named;\npub mod other;\n\
                            pub fn put() { let _ = glb::write(\"/tmp/r99glob\", \"x\"); }\n\
                            pub fn ctl_named() { let _ = named::write(\"/tmp/r99glob\", \"x\"); }\n"),
            ("src/glb.rs", "pub use std::fs::*;\n"),
            ("src/named.rs", "pub use std::fs::write;\n"),
            ("src/other.rs", "use crate::glb;\n\
                              pub fn sib() { let _ = glb::write(\"/tmp/r99glob\", \"x\"); }\n\
                              pub fn rooted() { let _ = crate::glb::write(\"/tmp/r99glob\", \"x\"); }\n"),
        ]);
        for f in ["put", "other::sib", "other::rooted"] {
            assert_eq!(
                effs_opt(&v, f), vec!["Fs".to_string()],
                "R99 SHAPE 1: `{f}` reaches `std::fs::write` through a submodule GLOB re-export and must \
                 charge Fs — pre-fix the report was EMPTY for all three spellings:\n{v:#}"
            );
        }
        assert_eq!(
            effs_opt(&v, "ctl_named"), vec!["Fs".to_string()],
            "INTRINSIC CONTROL: the NAMED re-export of the identical item already resolved before this \
             fix (R99 mechanism 1, closed at b00956b) and must still resolve:\n{v:#}"
        );
    }

    /// R99 (SHAPE 1) OVER-CHARGE CONTROL, and the reason this is not a one-line `name != "*"` deletion.
    /// A glob import is SHADOWED by an explicit item of the same name, so `glb::write` here is the module's
    /// own pure `write` — and rewriting it to `std::fs::write` would FABRICATE an Fs on a path that
    /// performs none, unrecoverably, because scan.rs never attempts a local link for a std-rooted path.
    ///
    /// ONE VARIABLE against `r99_submodule_glob_reexport_of_an_external_module_resolves`: the module
    /// declares the name. The local `write` performs a DISTINCT effect class (Env) rather than nothing, so
    /// this asserts a POSITIVE claim — an `Fs` here means the glob reached a name it does not bind, and an
    /// absence would mean the fixture never resolved at all (brief §E3).
    #[test]
    fn r99_a_module_glob_does_not_reach_a_name_the_module_declares() {
        let v = scan_crate_to_json("r99globshadow", &[
            ("src/lib.rs", "pub mod glb;\n\
                            pub fn put() { glb::write(\"/tmp/r99globshadow\", \"x\"); }\n"),
            ("src/glb.rs", "pub use std::fs::*;\n\
                            pub fn write(k: &str, v: &str) { std::env::set_var(k, v); }\n"),
        ]);
        assert_eq!(
            effs_opt(&v, "put"), vec!["Env".to_string()],
            "OVER-CHARGE CONTROL: `glb::write` names the module's OWN `write`, which sets an environment \
             variable and touches no file. `Fs` (or `Env`+`Fs`) means the glob rewrote a shadowed name — a \
             fabrication on a provably-Fs-free path:\n{v:#}"
        );
    }

    /// R99 (SHAPE 1) × R105 — THE `#[cfg]` INTERACTION, BUILT DELIBERATELY. Two exported globs in one
    /// module have no single answer, and unlike a named alias the arms cannot be distributed over: the
    /// shadow list belongs to ONE arm's module, so joining them would apply one arm's shadows to the
    /// other's target. Both spellings of the collision are refused — the honest under-report, which is the
    /// direction `unique_glob` already takes for the same question one level out.
    #[test]
    fn r99_a_cfg_duplicated_module_glob_is_refused_not_picked() {
        let inline = scan_src_to_json("r99globcfg", concat!(
            "mod glb {\n",
            "  #[cfg(unix)] pub use std::fs::*;\n",
            "  #[cfg(not(unix))] pub use std::env::*;\n",
            "}\n",
            "pub fn put() { let _ = glb::write(\"/tmp/r99globcfg\", \"x\"); }\n",
        ));
        assert!(
            effs_opt(&inline, "put").is_empty(),
            "TWO exported globs in one module: neither arm may be picked. Charging `Fs` here would be \
             deciding by source order, which is exactly the R105 defect:\n{inline:#}"
        );
        // The DUPLICATED-MODULE half: two `#[cfg]` arms of the SAME module, each with one glob. The count
        // above cannot see this — each arm is walked separately and each records exactly one — so
        // `record_alias` joins them and `module_glob_alias`'s multi-arm check is the refusal.
        let dup = scan_src_to_json("r99globdup", concat!(
            "#[cfg(unix)] mod glb { pub use std::fs::*; }\n",
            "#[cfg(not(unix))] mod glb { pub use std::env::*; }\n",
            "pub fn put() { glb::set_var(\"A\", \"B\"); }\n",
            "pub fn put2() { let _ = glb::write(\"/tmp/r99globdup\", \"x\"); }\n",
        ));
        for f in ["put", "put2"] {
            assert!(
                effs_opt(&dup, f).is_empty(),
                "a module declared under two `#[cfg]` arms has two glob targets and no single answer; \
                 `{f}` must charge neither:\n{dup:#}"
            );
        }
        // …AND THAT SCAN CANNOT TELL WHICH GUARD DID IT, WHICH IS WHY THIS ASSERTION IS HERE. Measured by
        // deleting the multi-arm check (§C): the scan above stays green, because without it `expand`
        // returns a path with a `\u{1}` still in it, which the classifier reads as no effect rather than as
        // an arm. That is a LATENT misclassification, not a refusal — one classifier rule keyed on a tail
        // rather than a whole path would turn it into a picked arm — so the refusal is asserted directly,
        // where the difference is visible.
        let mut uses: HashMap<String, String> = HashMap::new();
        uses.insert(
            format!("glb::{}", crate::decls::MOD_GLOB_KEY),
            format!("std::env{}std::fs", crate::decls::ALIAS_ALT_SEP),
        );
        assert_eq!(
            expand("glb::write", &uses), "glb::write",
            "a multi-arm module glob must come back UNCHANGED — never one arm, and never a joined path \
             with the arm separator still embedded in it"
        );
        uses.insert(format!("glb::{}", crate::decls::MOD_GLOB_KEY), "std::fs".into());
        assert_eq!(
            expand("glb::write", &uses), "std::fs::write",
            "…and the SINGLE-arm entry in the identical position resolves, so the assertion above is not \
             holding because the lookup is inert"
        );
    }

    /// R99 (SHAPE 1) — the three further refusals, each asserted against the ONE variable that makes the
    /// positive case resolve. A PRIVATE glob exports nothing nameable from outside; two exported globs are
    /// ambiguous; and a NAMED re-export must WIN over the glob, which is rustc's precedence.
    ///
    /// The named-beats-glob arm asserts a POSITIVE, DISTINCT class (`Env` from `set_var`, against the
    /// glob's `Fs`) rather than an absence, so it cannot pass by the fixture failing to resolve at all.
    #[test]
    fn r99_a_module_glob_is_refused_when_it_cannot_be_the_only_answer() {
        let private = scan_crate_to_json("r99globpriv", &[
            ("src/lib.rs", "pub mod glb;\npub fn put() { let _ = glb::write(\"/tmp/x\", \"y\"); }\n"),
            ("src/glb.rs", "use std::fs::*;\npub fn noop() {}\n"),
        ]);
        assert!(
            effs_opt(&private, "put").is_empty(),
            "a PRIVATE glob binds names in the module's own body and exports none of them, so it cannot \
             answer a qualified `glb::write` written outside it:\n{private:#}"
        );
        let two = scan_crate_to_json("r99globtwo", &[
            ("src/lib.rs", "pub mod glb;\npub fn put() { let _ = glb::write(\"/tmp/x\", \"y\"); }\n"),
            ("src/glb.rs", "pub use std::fs::*;\npub use std::env::*;\n"),
        ]);
        assert!(
            effs_opt(&two, "put").is_empty(),
            "TWO exported globs: never guess which one a name arrived through:\n{two:#}"
        );
        let named = scan_crate_to_json("r99globnamed", &[
            ("src/lib.rs", "pub mod glb;\npub fn put() { glb::write(\"A\", \"B\"); }\n"),
            ("src/glb.rs", "pub use std::fs::*;\npub use std::env::set_var as write;\n"),
        ]);
        assert_eq!(
            effs_opt(&named, "put"), vec!["Env".to_string()],
            "an explicit import SHADOWS a glob (rustc's rule), so `glb::write` is `std::env::set_var` and \
             charges Env. `Fs` means the glob beat the named re-export:\n{named:#}"
        );
    }

    /// R99 (SHAPE 2) — Pass A's decl indexes could not see `mod_aliases`, so a struct FIELD typed through
    /// a module alias was recorded as the literal written path and the method USING it was absent. The
    /// paired controls are the SAME alias in positions that already worked: the field typed directly, and
    /// the identical alias on a LOCAL binding (decoded in Pass B, where the seed is present).
    #[test]
    fn r99_a_field_typed_through_a_module_alias_resolves() {
        let v = scan_crate_to_json("r99field", &[
            ("src/lib.rs", "pub mod facade;\n\
                            pub struct Holder { pub c: facade::Command }\n\
                            pub struct Rooted { pub c: crate::facade::Command }\n\
                            pub struct Plain  { pub c: std::process::Command }\n\
                            impl Holder { pub fn run_aliased(&mut self) { let _ = self.c.status(); } }\n\
                            impl Rooted { pub fn run_rooted(&mut self) { let _ = self.c.status(); } }\n\
                            impl Plain  { pub fn run_plain(&mut self)  { let _ = self.c.status(); } }\n\
                            pub fn run_local() { let mut c: facade::Command = \
                              std::process::Command::new(\"/bin/sh\"); let _ = c.status(); }\n"),
            ("src/facade.rs", "pub use std::process::Command;\n"),
        ]);
        for f in ["Holder::run_aliased", "Rooted::run_rooted"] {
            assert_eq!(
                effs_opt(&v, f), vec!["Exec".to_string()],
                "R99 SHAPE 2: a field typed through a module alias must resolve to std's Command — \
                 pre-fix `{f}` was absent from functions[] while the control below reported Exec:\n{v:#}"
            );
        }
        for f in ["Plain::run_plain", "run_local"] {
            assert_eq!(
                effs_opt(&v, f), vec!["Exec".to_string()],
                "PAIRED CONTROL `{f}`: the same call in a position that already resolved before this \
                 fix — one variable, the module alias:\n{v:#}"
            );
        }
    }

    /// R99 (SHAPE 2) OVER-CHARGE CONTROL — the re-expansion runs against the DECLARING file's module path
    /// and an ALIAS-ONLY `use` map, which is the whole reason it lives at the merge rather than at the
    /// decode site. A bare local type must not be rewritten by another file's alias of the same name.
    ///
    /// Positive on BOTH arms and on DISTINCT classes, so neither can pass by resolving to nothing: the
    /// local `Widget::go` writes a file (Fs), the aliased one spawns (Exec).
    #[test]
    fn r99_the_field_re_expansion_does_not_rewrite_a_local_types_name() {
        let v = scan_crate_to_json("r99fieldshadow", &[
            ("src/lib.rs", "pub mod facade;\npub mod local;\n"),
            ("src/facade.rs", "pub use std::process::Command as Widget;\n"),
            ("src/local.rs", "pub struct Widget;\n\
                              impl Widget { pub fn go(&self) { let _ = std::fs::write(\"/tmp/w\", \"x\"); } }\n\
                              pub struct Holder { pub w: Widget }\n\
                              impl Holder { pub fn run(&self) { self.w.go(); } }\n\
                              pub struct Aliased { pub c: crate::facade::Widget }\n\
                              impl Aliased { pub fn run(&mut self) { let _ = self.c.status(); } }\n"),
        ]);
        assert_eq!(
            effs_opt(&v, "local::Holder::run"), vec!["Fs".to_string()],
            "OVER-CHARGE CONTROL: `w: Widget` is the file's OWN `Widget`, whose `go` writes a file. An \
             `Exec` means the re-expansion bound a bare name to another module's alias:\n{v:#}"
        );
        assert_eq!(
            effs_opt(&v, "local::Aliased::run"), vec!["Exec".to_string()],
            "…and the QUALIFIED spelling in the same file still resolves through the alias, so the \
             control above is not passing because the mechanism is inert:\n{v:#}"
        );
    }

    /// R99 (SHAPE 2) × R105 — A `#[cfg]`-DUPLICATED ALIAS MUST NOT REACH A DECL INDEX.
    ///
    /// R105 keeps every arm in one `\u{1}`-joined value and adjudicates at the CALL SITE, where the leaf
    /// is in hand. A decl index has no such site: a joined string lands in `fields` and every consumer
    /// (`tail2`, `local_types`, the receiver-type chain) reads it as ONE path.
    ///
    /// THIS GUARD WAS ADDED BECAUSE THE 256-CRATE A/B FOUND THE DEFECT IN THE FIRST CUT, not because it
    /// was foreseen. async-lock's `state: AtomicUsize` — through `mod sync { #[cfg(not(loom))] pub use
    /// core::sync::atomic; #[cfg(loom)] pub use loom::sync::atomic; }` — and bytes' `Bytes::data_mut` went
    /// `inferred: ["Unknown"], unresolved: true` -> `inferred: [], invisible: ["loom"]` on 5 rows. The
    /// effect is genuinely absent on either arm, but `deny Unknown` flips 1 -> 0, and a disclosure loss
    /// is a loss whatever the underlying truth is.
    ///
    /// ASSERTED AT `alias_expand_decls`' OWN CONTRACT rather than through a scan, deliberately: the
    /// downstream consequence needs a local trait whose method exists on only one arm (async-lock's real
    /// shape), and a fixture that merely approximated it would pass for the wrong reason — measured, on
    /// two attempts that both came back empty. Both directions are asserted, so neither can hold by the
    /// mechanism being inert.
    #[test]
    fn r99_a_cfg_duplicated_alias_is_not_written_into_a_decl_index() {
        use crate::cache::{alias_expand_decls, FileDecls};
        let mut fd = FileDecls::default();
        let holder = fd.fields.entry("Holder".into()).or_default();
        holder.insert("c".into(), "facade::Cmd".into());
        holder.insert("d".into(), "single::Cmd".into());

        let mut aliases: HashMap<String, String> = HashMap::new();
        crate::decls::record_alias(&mut aliases, "facade::Cmd".into(), "std::process::Command".into());
        crate::decls::record_alias(&mut aliases, "facade::Cmd".into(), "std::fs::File".into());
        crate::decls::record_alias(&mut aliases, "single::Cmd".into(), "std::process::Command".into());

        let out = alias_expand_decls(&fd, "", &aliases).expect(
            "the SINGLE-arm field must move — without this the assertion below could hold because \
             nothing expands at all",
        );
        assert_eq!(
            out.fields["Holder"]["d"], "std::process::Command",
            "the single-arm alias is exactly what this fix exists to resolve"
        );
        assert_eq!(
            out.fields["Holder"]["c"], "facade::Cmd",
            "a `#[cfg]`-DUPLICATED alias must be left as written: the decl index has no call site to \
             adjudicate the arms at, and a `\\u{{1}}`-joined string in `fields` is read as one path by \
             every consumer of it"
        );
    }

    /// R99 (SHAPE 2) — THE WARM CACHE MUST NOT SERVE AN EXPANSION THE ALIAS NO LONGER SUPPORTS.
    ///
    /// `alias_expand_decls` runs at the MERGE, over a crate-wide map, and its result is deliberately
    /// never written back into `decls_per_file` — because a `FileDecls` entry is keyed by ONE file's
    /// content hash, so a crate-wide fact baked into it would survive an edit to the file that supplied
    /// the fact. `main.rs`'s bytes do not change here; only `facade.rs`'s do. If the expansion were
    /// cached, the warm run would reuse `main.rs`'s decls and keep reporting `Exec` for a field that now
    /// aliases `std::fs::File`.
    ///
    /// THE CODE COMMENT ASSERTING THIS IS WHAT `assert-audit.sh` FLAGGED IN THIS DIFF, so it is measured
    /// rather than believed. Both arms are positive and on DISTINCT effect classes, so neither can hold
    /// by the fixture resolving to nothing.
    #[test]
    fn r99_a_warm_cache_re_derives_a_field_alias_after_the_declaring_file_changes() {
        // `incremental_scan` REMOVES `CANDOR_PANIC_ON_FILE`, and an env var is process-global: without
        // this lock the removal races the abort tests' `set_var`, and THEIR fault silently fails to
        // inject — `an_older_schema_cache_entry_is_discarded_rather_than_read_as_analysed` went red with
        // `left: 1, right: 2` under a different thread interleaving while passing on the run before. Any
        // test that calls `incremental_scan` must take this lock, injection or not.
        let _lock = abort_injection_lock();
        let d = std::env::temp_dir().join(format!("r99warm{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(d.join("src")).unwrap();
        std::fs::write(d.join("Cargo.toml"), "[package]\nname=\"r99warm\"\nversion=\"0.1.0\"\nedition=\"2021\"\n").unwrap();
        std::fs::write(d.join("src/lib.rs"),
            "pub mod facade;\n\
             pub struct Holder { pub c: facade::Thing }\n\
             impl Holder { pub fn run(&mut self) { let _ = self.c.status(); } }\n").unwrap();
        std::fs::write(d.join("src/facade.rs"), "pub use std::process::Command as Thing;\n").unwrap();
        let policy = d.join("candor.policy");
        std::fs::write(&policy, "deny Exec\n").unwrap();
        let out = d.join("r").to_string_lossy().into_owned();
        let pol = policy.to_string_lossy().into_owned();

        let (_, cold) = incremental_scan(&d, &out, &pol, None);
        assert_eq!(
            effs_opt(&cold, "Holder::run"), vec!["Exec".to_string()],
            "the COLD run must resolve the field through the alias — without this the warm assertion \
             below would hold because nothing ever resolved:\n{cold:#}"
        );

        // Only the DECLARING file changes. `src/lib.rs`'s bytes — and so its cached `FileDecls` — are
        // byte-identical, which is exactly the entry a baked-in expansion would be served from.
        std::fs::write(d.join("src/facade.rs"), "pub use std::fs::File as Thing;\n").unwrap();
        let (_, warm) = incremental_scan(&d, &out, &pol, None);
        assert_eq!(
            effs_opt(&warm, "Holder::run"), vec!["Fs".to_string()],
            "the WARM run must re-derive the expansion from the CURRENT crate-wide alias map. `Exec` \
             here means a crate-wide fact was cached inside a per-file entry and outlived the file that \
             supplied it:\n{warm:#}"
        );
        let _ = std::fs::remove_dir_all(&d);
    }

    // ── R100: THE SELF-SHADOW WINDOW (SOUNDNESS.md R100) ──────────────────────────────────────────
    //
    // R88 deferred the bare `let`'s `trait_vars` write past the trailing walk; R92 deferred the three
    // let-else binders'. Both enumerated BINDERS, and `visit_local` makes ~46 name-keyed mutations across
    // a dozen tables — so the class survived both. `elem_of` was the next one (R100), `fn_alias` the one
    // after (found by R99's own fixture, minutes later). The fix enumerates the STATE instead: one
    // capture/restore window around the trailing walk, covering every table and every binder shape.
    //
    // Every fixture below pairs a SELF-SHADOWING rebind with a NON-shadowing control of identical shape,
    // and the two effect classes are deliberately DIFFERENT so the union cannot hide a loss.

    /// R100 — `elem_of`. `self.elem_of.remove(name)` ran before `resolve_elem_type(&init.expr)`, whose
    /// `.clone()` arm recurses into the receiver and reads the entry just deleted. Measured pre-fix:
    /// `let ys = xs.clone(); for y in &ys { y.go(); }` → `["Fs"]`, and the self-shadowing
    /// `let xs = xs.clone();` → ABSENT from functions[] with `deny Fs` exiting 0.
    #[test]
    fn r100_elem_of_survives_a_self_shadowing_clone_rebind() {
        let src = concat!(
            "pub struct W;\n",
            "impl W { pub fn go(&self) { let _ = std::fs::write(\"/tmp/x\", \"w\"); } }\n",
            "pub fn shadowed(xs: Vec<W>) { let xs = xs.clone(); for x in &xs { x.go(); } }\n",
            "pub fn control(xs: Vec<W>) { let ys = xs.clone(); for y in &ys { y.go(); } }\n",
        );
        let v = scan_src_to_json("r100elem", src);
        assert_eq!(
            effs_opt(&v, "shadowed"), vec!["Fs".to_string()],
            "R100: a SELF-SHADOWING element-preserving rebind must keep the element type — the RHS's own \
             `.clone()` receiver still means the OUTER `xs`:\n{v:#}"
        );
        assert_eq!(
            effs_opt(&v, "control"), vec!["Fs".to_string()],
            "control regressed: the non-shadowing rebind resolved before the fix and must still:\n{v:#}"
        );
    }

    /// R100 — `str_locals`, named in the SOUNDNESS row as having the identical ordering
    /// (`self.str_locals.remove(name)` before the value is resolved from the RHS) but NOT established:
    /// the previous fixture showed no differential because neither arm resolved the host, so the
    /// instrument could not separate the two hypotheses. This one can — the control DOES resolve the
    /// host, so a shadowed arm that loses it is visible as a missing `hosts` entry rather than as two
    /// identical blanks.
    #[test]
    fn r100_str_locals_survive_a_self_shadowing_rebind() {
        let v = scan_src_to_json("r100str", concat!(
            "pub fn control() { let u = \"https://ctl.example/x\"; let v = u; \
                 let _ = reqwest::blocking::get(v); }\n",
            "pub fn shadowed() { let u = \"https://shadow.example/x\"; let u = u; \
                 let _ = reqwest::blocking::get(u); }\n",
        ));
        let ctl = hosts_of(fn_entry(&v, "control"));
        assert!(
            ctl.iter().any(|h| h.contains("ctl.example")),
            "INSTRUMENT CHECK (§E3): the control arm must actually RESOLVE a host, or this fixture \
             cannot tell a lost binding from one that was never resolved: {ctl:?}\n{v:#}"
        );
        let sh = hosts_of(fn_entry(&v, "shadowed"));
        assert!(
            sh.iter().any(|h| h.contains("shadow.example")),
            "R100: a self-shadowing string rebind must keep the resolved host — the RHS names the OUTER \
             binding: {sh:?}\n{v:#}"
        );
    }

    /// R100 — `fn_alias`, the third instance, found by R99's two-hop fixture rather than predicted.
    /// `let w = w;` removed the alias for stale-rebind hygiene BEFORE the same statement's RHS read it.
    /// (The chaining half of that behaviour is `r99_a_fn_alias_of_a_fn_alias_chains`; this pins the
    /// ORDERING half against the non-shadowing control.)
    #[test]
    fn r100_fn_alias_survives_a_self_shadowing_rebind() {
        let v = scan_src_to_json("r100alias", concat!(
            "pub fn shadowed() { let w = std::fs::write; let w = w; let _ = w(\"/tmp/x\", \"y\"); }\n",
            "pub fn control()  { let w = std::fs::write; let v = w; let _ = v(\"/tmp/x\", \"y\"); }\n",
        ));
        for f in ["shadowed", "control"] {
            assert_eq!(
                effs_opt(&v, f), vec!["Fs".to_string()],
                "R100/R99: `{f}` must charge Fs — the rebind's RHS means the PRE-rebind alias:\n{v:#}"
            );
        }
    }

    /// R100 — the REGRESSION pin for the twelve name-keyed tables `BoundNameState` REGISTERS today: each
    /// is captured and restored around the RHS walk. Ten are the resolution tables `scoped_binding` clears
    /// for a SHADOW BODY (its own pin is `every_name_keyed_table_is_scoped_by_the_one_binder`); two are
    /// the HEDGING sets `scoped_binding` deliberately KEEPS and this window must not. Exercised directly
    /// on `CallCollector` rather than through a fixture, for the reason the sibling test gives: a fixture
    /// can only witness the tables some shape happens to reach.
    ///
    /// R107 — THIS WAS NAMED `every_name_keyed_table_…` AND IT DOES NOT ANSWER FOR EVERY TABLE. Proven
    /// mechanically: a thirteenth name-keyed table was added to `CallCollector` and poisoned inside the
    /// window's scope, and this test passed, as did all 306. The compiler forces a new field into ten
    /// exhaustive struct literals — including the one below, which is why this LOOKS like a completeness
    /// gate — but nothing forces it into `BoundNameState`, `capture_bindings` or `restore_bindings`, and
    /// those three are what would actually cover it. Renamed to what it pins, with the residual written
    /// down instead of implied: **a table added to `CallCollector` and not registered in `BoundNameState`
    /// is outside the window, and no test in this repo will say so.**
    #[test]
    fn the_twelve_registered_name_keyed_tables_are_restored_for_the_rhs_walk() {
        let uses = HashMap::new();
        let fields = FieldIndex::new();
        let trait_fields = TraitFieldIndex::new();
        let trait_impls = TraitImplIndex::new();
        let local_traits = HashMap::new();
        let returns = ReturnIndex::new();
        let field_elem = FieldElemIndex::new();
        let field_elem_trait = FieldElemTraitIndex::new();
        let enum_variants = EnumVariantIndex::new();
        let enum_variant_traits = EnumVariantTraitIndex::new();
        let lazy = std::collections::HashSet::new();
        let consts = std::collections::HashMap::new();
        let macros = std::collections::HashMap::new();
        let mut c = CallCollector {
            modpath: String::new(), uses: &uses, vars: HashMap::new(), trait_vars: HashMap::new(),
            dyn_sig_traits: Default::default(), generic_bounds: HashMap::new(),
            trait_quals_by_param: HashMap::new(), trait_quals: HashMap::new(),
            fields: &fields, trait_fields: &trait_fields, trait_impls: &trait_impls,
            local_traits: &local_traits, returns: &returns, has_dyn_return: false,
            field_elem: &field_elem, enum_variants: &enum_variants, enum_variant_traits: &enum_variant_traits,
            ambiguous_enum_leaves: &std::collections::HashSet::new(), callable_statics: &std::collections::HashSet::new(), elem_of: HashMap::new(),
            field_elem_trait: &field_elem_trait, elem_trait_of: HashMap::new(),
            tuple_of: HashMap::new(), tuple_trait_of: HashMap::new(), calls: Vec::new(),
            closure_vars: Default::default(), fn_typed_vars: Default::default(),
            dep_bound_vars: HashMap::new(), fn_alias: Default::default(), lazy_statics: &lazy,
            forced_lazies: Default::default(), unresolved: false, err_ret_leaf: None,
            const_strings: &consts, local_macros: &macros, macro_expanding: Default::default(),
            str_locals: Default::default(),
            local_uses: Default::default(), bound_names: Default::default(), dispatch_sites: Default::default(),
            drop_relevant: &std::collections::HashSet::new(), escaping_ctors: Default::default(), marked_ctors: Default::default(), marked_cross_ctors: Default::default(), in_pattern: false,
        };
        let n = "x";
        c.vars.insert(n.into(), "Outer".into());
        c.trait_vars.insert(n.into(), vec!["Store".into()]);
        c.dep_bound_vars.insert(n.into(), "deplib::build".into());
        c.trait_quals_by_param.insert(n.into(), HashMap::from([("Store".to_string(), "deplib::Store".to_string())]));
        c.elem_of.insert(n.into(), "Elem".into());
        c.elem_trait_of.insert(n.into(), vec!["Doer".into()]);
        c.tuple_of.insert(n.into(), vec![Some("A".into())]);
        c.tuple_trait_of.insert(n.into(), vec![vec!["Doer".into()]]);
        c.fn_alias.insert(n.into(), "effectful".into());
        c.str_locals.insert(n.into(), "https://outer.example".into());
        c.closure_vars.insert(n.into());
        c.fn_typed_vars.insert(n.into());

        let pre = c.capture_bindings(&[n.to_string()]);
        // A statement's binder arms overwrite EVERY table for the name — the state the trailing walk of
        // this statement's own RHS must not see.
        c.vars.insert(n.into(), "Inner".into());
        c.trait_vars.insert(n.into(), vec!["Other".into()]);
        c.dep_bound_vars.insert(n.into(), "otherlib::build".into());
        c.trait_quals_by_param.insert(n.into(), HashMap::new());
        c.elem_of.insert(n.into(), "OtherElem".into());
        c.elem_trait_of.insert(n.into(), vec!["Other".into()]);
        c.tuple_of.insert(n.into(), vec![Some("B".into())]);
        c.tuple_trait_of.insert(n.into(), vec![vec!["Other".into()]]);
        c.fn_alias.insert(n.into(), "other".into());
        c.str_locals.insert(n.into(), "https://inner.example".into());
        c.closure_vars.remove(n);
        c.fn_typed_vars.remove(n);
        let post = c.capture_bindings(&[n.to_string()]);

        c.restore_bindings(&pre);
        let mut wrong: Vec<&str> = Vec::new();
        if c.vars.get(n).map(String::as_str) != Some("Outer") { wrong.push("vars"); }
        if c.trait_vars.get(n).cloned() != Some(vec!["Store".to_string()]) { wrong.push("trait_vars"); }
        if c.dep_bound_vars.get(n).map(String::as_str) != Some("deplib::build") { wrong.push("dep_bound_vars"); }
        if c.trait_quals_by_param.get(n).map(|m| m.len()) != Some(1) { wrong.push("trait_quals_by_param"); }
        if c.elem_of.get(n).map(String::as_str) != Some("Elem") { wrong.push("elem_of"); }
        if c.elem_trait_of.get(n).cloned() != Some(vec!["Doer".to_string()]) { wrong.push("elem_trait_of"); }
        if c.tuple_of.get(n).cloned() != Some(vec![Some("A".to_string())]) { wrong.push("tuple_of"); }
        if c.tuple_trait_of.get(n).cloned() != Some(vec![vec!["Doer".to_string()]]) { wrong.push("tuple_trait_of"); }
        if c.fn_alias.get(n).map(String::as_str) != Some("effectful") { wrong.push("fn_alias"); }
        if c.str_locals.get(n).map(String::as_str) != Some("https://outer.example") { wrong.push("str_locals"); }
        // The HEDGING sets are restored too — unlike `scoped_binding`, which keeps them. The questions
        // differ: there the shadow's BODY is being walked and clearing a hedge would let `x()` resolve to
        // a free fn; here the pre-statement value is what the RHS meant, so restoring is honest in BOTH
        // directions and leaving them alone would answer the RHS with the new binding's hedging.
        if !c.closure_vars.contains(n) { wrong.push("closure_vars"); }
        if !c.fn_typed_vars.contains(n) { wrong.push("fn_typed_vars"); }
        assert!(wrong.is_empty(),
                "these name-keyed tables are NOT restored for the RHS walk: {wrong:?}. A `let` whose RHS \
                 names the binding it shadows will resolve against the NEW entry and lose the outer \
                 binding's effect, with nothing disclosed — SOUNDNESS.md R88/R92/R100.");

        // …and the statement's own decision is put back afterwards, or every LATER statement reads the
        // outer binding instead (the mirror failure: a stale entry answering for a rebound name).
        c.restore_bindings(&post);
        assert_eq!(c.vars.get(n).map(String::as_str), Some("Inner"));
        assert_eq!(c.elem_of.get(n).map(String::as_str), Some("OtherElem"));
        assert_eq!(c.fn_alias.get(n).map(String::as_str), Some("other"));
        assert!(!c.closure_vars.contains(n) && !c.fn_typed_vars.contains(n));

        // An entry ABSENT from the snapshot must be REMOVED, never left standing.
        let empty = c.capture_bindings(&["never_bound".to_string()]);
        c.vars.insert("never_bound".into(), "Ghost".into());
        c.restore_bindings(&empty);
        assert!(!c.vars.contains_key("never_bound"),
                "restore must DELETE an entry the snapshot did not have — leaving it is the fabrication \
                 direction this window exists to avoid");
    }
