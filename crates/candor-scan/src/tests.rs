//! The scanner's unit tests — the former in-file `#[cfg(test)] mod tests` of main.rs,
//! now a file module (`super::*` still resolves to the crate root). Original indentation
//! kept verbatim: several tests embed column-sensitive source strings.

    use super::*;

    fn uses(pairs: &[(&str, &str)]) -> HashMap<String, String> {
        pairs.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect()
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
        let (fe, ev) = (FieldElemIndex::new(), EnumVariantIndex::new());
        let fet = FieldElemTraitIndex::new();
        let mut c = CallCollector {
            modpath: String::new(),
            uses: &uses,
            vars: HashMap::new(),
            trait_vars: HashMap::new(),
            dyn_sig_traits: Default::default(), trait_quals: Default::default(),
            fields: &fields,
            trait_fields: &tf,
            trait_impls: &ti,
            local_traits: &td,
            returns: &returns,
            has_dyn_return: false,
            field_elem: &fe, field_elem_trait: &fet,
            enum_variants: &ev,
            elem_of: HashMap::new(), elem_trait_of: HashMap::new(), tuple_of: HashMap::new(), tuple_trait_of: std::collections::HashMap::new(),
            calls: Vec::new(),
            closure_vars: std::collections::HashSet::new(),
            fn_typed_vars: std::collections::HashSet::new(),
            fn_alias: std::collections::HashMap::new(),
            lazy_statics: empty_lazy(),
            forced_lazies: std::collections::HashSet::new(),
            unresolved: false,
            err_ret_leaf: None,
            const_strings: empty_consts(), local_macros: empty_consts(), macro_expanding: std::collections::HashSet::new(), str_locals: std::collections::HashMap::new(),
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
        let (fe, ev) = (FieldElemIndex::new(), EnumVariantIndex::new());
        let fet = FieldElemTraitIndex::new();
        let block: syn::Block =
            syn::parse_str("{ client.get(url).send(); self.http.execute(req); }").unwrap();
        let mut c = CallCollector {
            modpath: String::new(),
            uses: &uses,
            vars,
            trait_vars: HashMap::new(),
            dyn_sig_traits: Default::default(), trait_quals: Default::default(),
            fields: &fields,
            trait_fields: &tf,
            trait_impls: &ti,
            local_traits: &td,
            returns: &returns,
            has_dyn_return: false,
            field_elem: &fe, field_elem_trait: &fet,
            enum_variants: &ev,
            elem_of: HashMap::new(), elem_trait_of: HashMap::new(), tuple_of: HashMap::new(), tuple_trait_of: std::collections::HashMap::new(),
            calls: Vec::new(),
            closure_vars: std::collections::HashSet::new(),
            fn_typed_vars: std::collections::HashSet::new(),
            fn_alias: std::collections::HashMap::new(),
            lazy_statics: empty_lazy(),
            forced_lazies: std::collections::HashSet::new(),
            unresolved: false,
            err_ret_leaf: None,
            const_strings: empty_consts(), local_macros: empty_consts(), macro_expanding: std::collections::HashSet::new(), str_locals: std::collections::HashMap::new(),
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
                prefix, want_json: true, include_tests: false, policy: None, baseline: None, quiet: true, deps_idx: &idx,
            });
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
                prefix, want_json: true, include_tests: false, policy: None, baseline: None, quiet: true, deps_idx: &idx,
            });
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
                prefix, want_json: true, include_tests: false, policy: None, baseline: None, quiet: true, deps_idx: &idx,
            });
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
            prefix, want_json: true, include_tests: false, policy: None, baseline: None, quiet: true, deps_idx: &idx,
        });
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
            prefix, want_json: true, include_tests: false, policy: None, baseline: None, quiet: true, deps_idx: &idx,
        });
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
            prefix, want_json: true, include_tests: false, policy: None, baseline: None, quiet: true, deps_idx: &idx,
        });
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
                prefix, want_json: true, include_tests: false, policy: None, baseline: None, quiet: true, deps_idx: &idx,
            });
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
                prefix, want_json: true, include_tests: false, policy: None, baseline: None, quiet: true, deps_idx: &idx,
            });
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
                prefix, want_json: true, include_tests: false, policy: None, baseline: None, quiet: true, deps_idx: &idx,
            });
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
                prefix, want_json: true, include_tests: false, policy: None, baseline: None, quiet: true, deps_idx: &idx,
            });
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
                prefix, want_json: true, include_tests: false, policy: None, baseline: None, quiet: true, deps_idx: &idx,
            });
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
                prefix, want_json: true, include_tests: false, policy: None, baseline: None, quiet: true, deps_idx: &idx,
            });
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
                prefix, want_json: true, include_tests: false, policy: None, baseline: None, quiet: true, deps_idx: &idx,
            });
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
                prefix, want_json: true, include_tests: false, policy: None, baseline: None, quiet: true, deps_idx: &idx,
            });
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
                prefix, want_json: true, include_tests: false, policy: None, baseline: None, quiet: true, deps_idx: &idx,
            });
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
            prefix, want_json: true, include_tests: false, policy: None, baseline: None, quiet: true, deps_idx: &idx,
        });
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
            prefix, want_json: true, include_tests: false, policy: None, baseline: None, quiet: true, deps_idx: &idx,
        });
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
            prefix, want_json: true, include_tests: false, policy: None, baseline: None, quiet: true, deps_idx: &idx,
        });
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
                prefix, want_json: true, include_tests: false, policy: None, baseline: None, quiet: true, deps_idx: &idx,
            });
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
            prefix, want_json: true, include_tests: false, policy: None, baseline: None, quiet: true, deps_idx: &idx,
        });
        assert_eq!(rc, 0);
        let v: serde_json::Value = serde_json::from_str(&body.unwrap()).unwrap();
        let _ = std::fs::remove_dir_all(&d);
        v
    }

    #[cfg(test)]
    fn fixture_effects(v: &serde_json::Value, name: &str) -> Vec<String> {
        v["functions"].as_array().into_iter().flatten()
            .filter(|f| f["fn"].as_str() == Some(name))
            .flat_map(|f| f["inferred"].as_array().into_iter().flatten()
                .filter_map(|e| e.as_str().map(String::from)).collect::<Vec<_>>())
            .collect()
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
        let (fe, ev) = (FieldElemIndex::new(), EnumVariantIndex::new());
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
                dyn_sig_traits: dyn_sig_trait_leaves(&sig), trait_quals: sig_trait_quals(&sig),
                fields: &fields,
                trait_fields: &tf,
                trait_impls: &ti,
                local_traits: &td,
                returns: &returns,
                has_dyn_return: false,
                field_elem: &fe, field_elem_trait: &fet,
                enum_variants: &ev,
                elem_of: HashMap::new(), elem_trait_of: HashMap::new(), tuple_of: HashMap::new(), tuple_trait_of: std::collections::HashMap::new(),
                calls: Vec::new(),
                closure_vars: std::collections::HashSet::new(),
                fn_typed_vars: std::collections::HashSet::new(),
            fn_alias: std::collections::HashMap::new(),
            lazy_statics: empty_lazy(),
            forced_lazies: std::collections::HashSet::new(),
                unresolved: false,
                err_ret_leaf: None,
                const_strings: empty_consts(), local_macros: empty_consts(), macro_expanding: std::collections::HashSet::new(), str_locals: std::collections::HashMap::new(),
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
                uses: &uses, vars: HashMap::new(), trait_vars: seed_trait_vars(&sig), dyn_sig_traits: dyn_sig_trait_leaves(&sig), trait_quals: sig_trait_quals(&sig),
                fields: &fields, trait_fields: &tf, trait_impls: &ti2, local_traits: &td,
                returns: &returns, has_dyn_return: false, field_elem: &fe, field_elem_trait: &fet, enum_variants: &ev, elem_of: HashMap::new(), elem_trait_of: HashMap::new(), tuple_of: HashMap::new(), tuple_trait_of: std::collections::HashMap::new(),
                calls: Vec::new(),
                closure_vars: std::collections::HashSet::new(), fn_typed_vars: std::collections::HashSet::new(), fn_alias: std::collections::HashMap::new(), lazy_statics: empty_lazy(), forced_lazies: std::collections::HashSet::new(), unresolved: false, err_ret_leaf: None, const_strings: empty_consts(), local_macros: empty_consts(), macro_expanding: std::collections::HashSet::new(), str_locals: std::collections::HashMap::new(),
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
                    uses: &uses, vars: HashMap::new(), trait_vars: seed_trait_vars(&sig), dyn_sig_traits: dyn_sig_trait_leaves(&sig), trait_quals: sig_trait_quals(&sig),
                    fields: &fields, trait_fields: &tf, trait_impls: &ti2, local_traits: &td,
                    returns: &returns, has_dyn_return: false, field_elem: &fe, field_elem_trait: &fet, enum_variants: &ev, elem_of: HashMap::new(), elem_trait_of: HashMap::new(), tuple_of: HashMap::new(), tuple_trait_of: std::collections::HashMap::new(),
                    calls: Vec::new(),
                    closure_vars: std::collections::HashSet::new(), fn_typed_vars: std::collections::HashSet::new(), fn_alias: std::collections::HashMap::new(), lazy_statics: empty_lazy(), forced_lazies: std::collections::HashSet::new(), unresolved: false, err_ret_leaf: None, const_strings: empty_consts(), local_macros: empty_consts(), macro_expanding: std::collections::HashSet::new(), str_locals: std::collections::HashMap::new(),
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
        let (fe, ev) = (FieldElemIndex::new(), EnumVariantIndex::new());
        let fet = FieldElemTraitIndex::new();
        let block: syn::Block =
            syn::parse_str("{ let p = create_pool()?; p.fetch_one(q); }").unwrap();
        let mut c = CallCollector {
            modpath: String::new(),
            uses: &uses,
            vars: HashMap::new(),
            trait_vars: HashMap::new(),
            dyn_sig_traits: Default::default(), trait_quals: Default::default(),
            fields: &fields,
            trait_fields: &tf,
            trait_impls: &ti,
            local_traits: &td,
            returns: &returns,
            has_dyn_return: false,
            field_elem: &fe, field_elem_trait: &fet,
            enum_variants: &ev,
            elem_of: HashMap::new(), elem_trait_of: HashMap::new(), tuple_of: HashMap::new(), tuple_trait_of: std::collections::HashMap::new(),
            calls: Vec::new(),
            closure_vars: std::collections::HashSet::new(),
            fn_typed_vars: std::collections::HashSet::new(),
            fn_alias: std::collections::HashMap::new(),
            lazy_statics: empty_lazy(),
            forced_lazies: std::collections::HashSet::new(),
            unresolved: false,
            err_ret_leaf: None,
            const_strings: empty_consts(), local_macros: empty_consts(), macro_expanding: std::collections::HashSet::new(), str_locals: std::collections::HashMap::new(),
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
                dyn_sig_traits: Default::default(), trait_quals: Default::default(),
                fields: &fields,
                trait_fields: &tf,
                trait_impls: &ti,
                local_traits: &td,
                returns: &returns,
                has_dyn_return: false,
                field_elem: &fe, field_elem_trait: &fet,
                enum_variants: &ev,
                elem_of: HashMap::new(), elem_trait_of: HashMap::new(), tuple_of: HashMap::new(), tuple_trait_of: std::collections::HashMap::new(),
                calls: Vec::new(),
                closure_vars: std::collections::HashSet::new(),
                fn_typed_vars: std::collections::HashSet::new(),
            fn_alias: std::collections::HashMap::new(),
            lazy_statics: empty_lazy(),
            forced_lazies: std::collections::HashSet::new(),
                unresolved: false,
                err_ret_leaf: None,
                const_strings: empty_consts(), local_macros: empty_consts(), macro_expanding: std::collections::HashSet::new(), str_locals: std::collections::HashMap::new(),
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
        let v = policy_violations(
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
        assert!(policy_violations("deny Exec\n", &all, &inferred, &calls, &hosts, &empty, &empty, &tables, &empty_inc, &empty, &std::collections::BTreeMap::new(), &std::collections::BTreeSet::new()).is_empty());
        assert_eq!(policy_violations("pure db\n", &all, &inferred, &calls, &hosts, &empty, &empty, &tables, &empty_inc, &empty, &std::collections::BTreeMap::new(), &std::collections::BTreeSet::new()).len(), 1);
        // the Db table allowlist: db::run reaches audit.log — outside `ledger.*` -> violation;
        // covered by `audit.*` -> clean. ui::draw INHERITS Db but the literal propagation is the
        // caller's tablesacc, supplied here only for db::run, so only db::run flags.
        let bad = policy_violations("allow Db in db ledger.*\n", &all, &inferred, &calls, &hosts, &empty, &empty, &tables, &empty_inc, &empty, &std::collections::BTreeMap::new(), &std::collections::BTreeSet::new());
        assert_eq!(bad.len(), 1, "{}", bad.iter().map(|x| x.detail.clone()).collect::<Vec<_>>().join(" | "));
        assert!(bad[0].detail.contains("audit.log"));
        assert!(policy_violations("allow Db in db audit.*\n", &all, &inferred, &calls, &hosts, &empty, &empty, &tables, &empty_inc, &empty, &std::collections::BTreeMap::new(), &std::collections::BTreeSet::new()).is_empty());
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
            policy_violations(pol, &all, &inferred, &calls, &empty, &empty, &empty, &empty, &empty_inc, rc, &std::collections::BTreeMap::new(), &std::collections::BTreeSet::new())
        };
        // matching class → fires
        assert_eq!(gate("deny Net Unknown[native]\n", &rc).len(), 1, "Unknown[native] must fire on a native-class Unknown");
        // non-matching class → tolerated
        assert!(gate("deny Net Unknown[reflect]\n", &rc).is_empty(), "Unknown[reflect] must tolerate a native-class Unknown");
        // bare Unknown → fires regardless of class
        assert_eq!(gate("deny Net Unknown\n", &rc).len(), 1, "bare deny Unknown fires on any Unknown");
        // an Unknown with NO recorded reason class → treated as `unresolved` (conservative)
        let none: HashMap<String, BTreeSet<String>> = HashMap::new();
        assert_eq!(gate("deny Net Unknown[unresolved]\n", &none).len(), 1, "no reason class ⇒ unresolved");
        assert!(gate("deny Net Unknown[reflect]\n", &none).is_empty(), "no reason class must NOT match a specific class");
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
        let v = policy_violations("deny Net Unknown[native]\n", &all, &inferred, &calls, &empty, &empty, &empty, &empty, &empty_inc, &rc_acc, &std::collections::BTreeMap::new(), &std::collections::BTreeSet::new());
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
        let v = policy_violations(
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
        let pv = policy_violations(
            "deny Net[unknown-host]\n", &pall, &pinf, &calls, &pacc, &empty, &empty, &empty,
            &empty_inc, &empty_rc, &std::collections::BTreeMap::new(), &partners,
        );
        assert!(!pv.iter().any(|g| g.func == "d::partner"), "a config net-partner is tolerated");
        let bare = policy_violations(
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
        let v = policy_violations(
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
                policy: Some(pp.to_string_lossy().into_owned()), baseline: None, quiet: true, deps_idx: &idx,
            });
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
                baseline: None, quiet: true, deps_idx: &idx,
            });
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
                prefix, want_json: true, include_tests: false, policy: None, baseline: None, quiet: true, deps_idx: &idx,
            });
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
                prefix, want_json: true, include_tests: false, policy: None, baseline: None, quiet: true, deps_idx: &idx,
            });
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
        let (mut fe, mut ev) = (FieldElemIndex::new(), HashMap::new());
        let mut fet = FieldElemTraitIndex::new();
        collect_decls(&file.items, false, &mut uses, &mut fields, &mut fe, &mut fet, &mut rets, &mut ev, &mut ti, &mut td, &mut tf, &mut std::collections::HashSet::new(), &mut std::collections::HashSet::new(), &mut std::collections::HashSet::new(), &mut std::collections::HashMap::new(), &mut std::collections::HashSet::new(), &mut std::collections::HashMap::new(), &mut std::collections::HashMap::new(), &mut std::collections::HashMap::new());
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
            prefix, want_json: true, include_tests: false, policy: None, baseline: None, quiet: true, deps_idx: &idx,
        });
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
            prefix, want_json: true, include_tests: false, policy: None, baseline: None, quiet: true, deps_idx: &idx,
        });
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
            prefix, want_json: true, include_tests: false, policy: None, baseline: None, quiet: true, deps_idx: &idx,
        });
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
            prefix, want_json: true, include_tests: false, policy: None, baseline: None, quiet: true, deps_idx: &idx,
        });
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
            prefix, want_json: true, include_tests: false, policy: None, baseline: None, quiet: true, deps_idx: &idx,
        });
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
            prefix, want_json: true, include_tests: false, policy: None, baseline: None, quiet: true, deps_idx: &idx,
        });
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
            prefix, want_json: true, include_tests: false, policy: None, baseline: None, quiet: true, deps_idx: &idx,
        });
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
            prefix, want_json: true, include_tests: false, policy: None, baseline: None, quiet: true, deps_idx: &idx,
        });
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
        collect_decls(&file.items, false, &mut uses, &mut fields, &mut fe, &mut fet, &mut rets, &mut ev, &mut ti, &mut td, &mut tf, &mut std::collections::HashSet::new(), &mut std::collections::HashSet::new(), &mut std::collections::HashSet::new(), &mut std::collections::HashMap::new(), &mut std::collections::HashSet::new(), &mut std::collections::HashMap::new(), &mut std::collections::HashMap::new(), &mut std::collections::HashMap::new());
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
        let mut ti = TraitImplIndex::new();
        let mut td: HashMap<String, LocalTrait> = HashMap::new();
        let mut tf = TraitFieldIndex::new();
        collect_decls(&file.items, false, &mut uses, &mut fields, &mut field_elem, &mut field_elem_trait, &mut rets,
                      &mut enum_tmp, &mut ti, &mut td, &mut tf, &mut std::collections::HashSet::new(),
                      &mut std::collections::HashSet::new(), &mut std::collections::HashSet::new(), &mut std::collections::HashMap::new(), &mut std::collections::HashSet::new(), &mut std::collections::HashMap::new(), &mut std::collections::HashMap::new(), &mut std::collections::HashMap::new());
        let returns: ReturnIndex = rets.into_iter().filter_map(|(k, v)| v.map(|t| (k, t))).collect();
        let enum_variants: EnumVariantIndex =
            enum_tmp.into_iter().filter_map(|(k, v)| v.map(|t| (k, t))).collect();
        let traits = TraitIndexes { impls: &ti, decls: &td, fields: &tf };
        let elems = ElemIndexes { field_elem: &field_elem, field_elem_trait: &field_elem_trait, enum_variants: &enum_variants };
        let mut fns: Vec<FnInfo> = Vec::new();
        let mut us2 = HashMap::new();
        let mut locs = Vec::new();
        fn_locs(&file.items, "lib.rs", false, &mut locs);
        let mut loc_idx = 0usize;
        scan_items(&file.items, "", &locs, &mut loc_idx, false, &fields, &returns, traits, elems, &std::collections::HashSet::new(), &std::collections::HashMap::new(), &std::collections::HashMap::new(), &mut us2, &mut fns);
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
        let mut ti = TraitImplIndex::new();
        let mut td: HashMap<String, LocalTrait> = HashMap::new();
        let mut tf = TraitFieldIndex::new();
        collect_decls(&file.items, false, &mut uses, &mut fields, &mut field_elem, &mut field_elem_trait, &mut rets,
                      &mut enum_tmp, &mut ti, &mut td, &mut tf, &mut std::collections::HashSet::new(),
                      &mut std::collections::HashSet::new(), &mut std::collections::HashSet::new(), &mut std::collections::HashMap::new(), &mut std::collections::HashSet::new(), &mut std::collections::HashMap::new(), &mut std::collections::HashMap::new(), &mut std::collections::HashMap::new());
        let returns: ReturnIndex = rets.into_iter().filter_map(|(k, v)| v.map(|t| (k, t))).collect();
        let enum_variants: EnumVariantIndex =
            enum_tmp.into_iter().filter_map(|(k, v)| v.map(|t| (k, t))).collect();
        let traits = TraitIndexes { impls: &ti, decls: &td, fields: &tf };
        let elems = ElemIndexes { field_elem: &field_elem, field_elem_trait: &field_elem_trait, enum_variants: &enum_variants };
        let mut fns: Vec<FnInfo> = Vec::new();
        let mut us2 = HashMap::new();
        let mut locs = Vec::new();
        fn_locs(&file.items, "lib.rs", false, &mut locs);
        let mut loc_idx = 0usize;
        scan_items(&file.items, "", &locs, &mut loc_idx, false, &fields, &returns, traits, elems, &std::collections::HashSet::new(), &std::collections::HashMap::new(), &std::collections::HashMap::new(), &mut us2, &mut fns);
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
        let mut ti = TraitImplIndex::new();
        let mut td: HashMap<String, LocalTrait> = HashMap::new();
        let mut tf = TraitFieldIndex::new();
        collect_decls(&file.items, false, &mut uses, &mut fields, &mut field_elem, &mut field_elem_trait, &mut rets,
                      &mut enum_tmp, &mut ti, &mut td, &mut tf, &mut std::collections::HashSet::new(),
                      &mut std::collections::HashSet::new(), &mut std::collections::HashSet::new(), &mut std::collections::HashMap::new(), &mut std::collections::HashSet::new(), &mut std::collections::HashMap::new(), &mut std::collections::HashMap::new(), &mut std::collections::HashMap::new());
        let returns: ReturnIndex = rets.into_iter().filter_map(|(k, v)| v.map(|t| (k, t))).collect();
        let enum_variants: EnumVariantIndex =
            enum_tmp.into_iter().filter_map(|(k, v)| v.map(|t| (k, t))).collect();
        let traits = TraitIndexes { impls: &ti, decls: &td, fields: &tf };
        let elems = ElemIndexes { field_elem: &field_elem, field_elem_trait: &field_elem_trait, enum_variants: &enum_variants };
        let mut fns: Vec<FnInfo> = Vec::new();
        let mut us2 = HashMap::new();
        let mut locs = Vec::new();
        fn_locs(&file.items, "lib.rs", false, &mut locs);
        let mut loc_idx = 0usize;
        scan_items(&file.items, "", &locs, &mut loc_idx, false, &fields, &returns, traits, elems, &std::collections::HashSet::new(), &std::collections::HashMap::new(), &std::collections::HashMap::new(), &mut us2, &mut fns);
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
                      &mut enum_tmp, &mut ti, &mut td, &mut tf, &mut std::collections::HashSet::new(),
                      &mut std::collections::HashSet::new(), &mut std::collections::HashSet::new(), &mut std::collections::HashMap::new(), &mut std::collections::HashSet::new(), &mut std::collections::HashMap::new(), &mut std::collections::HashMap::new(), &mut std::collections::HashMap::new());
        let ev: EnumVariantIndex = enum_tmp.into_iter().filter_map(|(k, v)| v.map(|t| (k, t))).collect();
        assert_eq!(ev.get("One").map(String::as_str), Some("i32")); // single-payload: kept
        assert_eq!(ev.get("Pair"), None);                           // multi-field: not indexed
        assert_eq!(ev.get("Unit"), None);                           // unit variant: not indexed
        assert_eq!(ev.get("Two"), None);                            // conflicting payloads: dropped
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
            baseline: None, quiet: true, deps_idx: &idx,
        });
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
            baseline: None, quiet: true, deps_idx: &idx,
        });
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
        assert!(readme.contains("spec 0.23"), "README must state the spec 0.23 floor");
        assert!(agents.contains("spec 0.23"), "AGENTS must state the spec 0.23 floor");
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
        match check_baseline(&pre, ".", "mycrate", &all, &inferred, false) {
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
        assert!(matches!(check_baseline(&direct, ".", "mycrate", &all, &inferred, false),
            BaselineOutcome::Checked(v) if v.len() == 1));
        // version mismatch / missing provenance / empty value → Invalid (exit 2, never evaluated)
        std::fs::write(d.join("stale.mycrate.scan.json"), report("scan-0.0.1")).unwrap();
        let stale = d.join("stale").to_string_lossy().into_owned();
        assert!(matches!(check_baseline(&stale, ".", "mycrate", &all, &inferred, false), BaselineOutcome::Invalid));
        std::fs::write(d.join("bare.mycrate.scan.json"), r#"[{"fn":"a","inferred":["Fs"]}]"#).unwrap();
        let bare = d.join("bare").to_string_lossy().into_owned();
        assert!(matches!(check_baseline(&bare, ".", "mycrate", &all, &inferred, false), BaselineOutcome::Invalid));
        assert!(matches!(check_baseline("", ".", "mycrate", &all, &inferred, false), BaselineOutcome::Invalid));
        // absent file → Inactive (note; exit unchanged)
        let absent = d.join("nosuch").to_string_lossy().into_owned();
        assert!(matches!(check_baseline(&absent, ".", "mycrate", &all, &inferred, false), BaselineOutcome::Inactive));
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
        match check_baseline(&pre, ".", "mycrate", &all, &inferred, false) {
            BaselineOutcome::Checked(v) => assert!(v.is_empty(), "ratchet OFF must not flag an Unknown gain: {v:?}",
                v = v.iter().map(|x| x.detail.clone()).collect::<Vec<_>>()),
            _ => panic!("a valid same-build baseline must be evaluated"),
        }
        // ratchet ON: exactly Y (the newly-introduced Unknown) fails; X (already Unknown) is grandfathered.
        match check_baseline(&pre, ".", "mycrate", &all, &inferred, true) {
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
        let rc = scan_target(&d.to_string_lossy(), prefix.clone(), false, false, None, None, &idx);
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
            policy: None, baseline: None, quiet: true, deps_idx: &idx,
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
            prefix, want_json: false, include_tests: false, policy: None, baseline: None, quiet: true, deps_idx: &idx,
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
                prefix, want_json: true, include_tests: false, policy: None, baseline: None, quiet: true, deps_idx: &idx,
            });
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
                prefix, want_json: true, include_tests: false, policy: None, baseline: None, quiet: true, deps_idx: &idx,
            });
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
                prefix, want_json: true, include_tests: false, policy: None, baseline: None, quiet: true, deps_idx: &idx,
            });
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
    // from the digest = a stale cache silently returning unsound effect sets. Rust has no runtime field
    // reflection, so this stands in for it two ways: (1) the exhaustive destructure below stops COMPILING
    // the moment a field is added to `MergedDecls` until you bind it here — forcing the question "is it in
    // the digest?"; (2) every field is then mutated in isolation and the digest MUST move, proving each is
    // actually folded in. Add a field → add its `_` binding AND a mutator case, or the build/test fails.
    #[test]
    fn every_merged_decls_field_is_folded_into_the_digest() {
        // (1) Compile-time exhaustiveness: no `..`, so a new field breaks this line until it's listed.
        let MergedDecls {
            fields: _,
            field_elem: _,
            field_elem_trait: _,
            rets: _,
            enum_tmp: _,
            trait_impls: _,
            trait_decls: _,
            trait_fields: _,
            prim_aliases: _,
            extern_fns: _,
            drop_types: _,
            deref_target: _,
            lazy_statics: _,
            const_strings: _,
            local_macros: _,
            blanket_methods: _,
            root_reexports: _,
        } = MergedDecls::default();

        let empty = decl_index_digest(&MergedDecls::default());

        // (2) One mutator per field — each touches exactly that field and nothing else.
        type Mutator = fn(&mut MergedDecls);
        let mutators: Vec<(&str, Mutator)> = vec![
            ("fields", |m| { m.fields.entry("S".into()).or_default().insert("f".into(), "T".into()); }),
            ("field_elem", |m| { m.field_elem.entry("S".into()).or_default().insert("f".into(), "E".into()); }),
            ("field_elem_trait", |m| { m.field_elem_trait.entry("S".into()).or_default().insert("f".into(), vec!["Tr".into()]); }),
            ("rets", |m| { m.rets.insert("f".into(), Some("T".into())); }),
            ("enum_tmp", |m| { m.enum_tmp.insert("v".into(), Some("E".into())); }),
            ("trait_impls", |m| { m.trait_impls.entry("Tr".into()).or_default().push("Ty".into()); }),
            ("trait_decls", |m| { m.trait_decls.entry("Tr".into()).or_default().count += 1; }),
            ("trait_fields", |m| { m.trait_fields.entry("S".into()).or_default().insert("f".into(), vec!["b".into()]); }),
            ("prim_aliases", |m| { m.prim_aliases.insert("A".into()); }),
            ("extern_fns", |m| { m.extern_fns.insert("system".into()); }),
            ("drop_types", |m| { m.drop_types.insert("Guard".into()); }),
            ("lazy_statics", |m| { m.lazy_statics.insert("CONFIG".into()); }),
            ("const_strings", |m| { m.const_strings.insert("API_BASE".into(), "https://api.openai.com".into()); }),
            ("local_macros", |m| { m.local_macros.insert("do_io".into(), "() => { fs::write(\"/x\", b\"y\"); }".into()); }),
            ("blanket_methods", |m| { m.blanket_methods.insert("ext".into(), "T".into()); }),
            ("root_reexports", |m| { m.root_reexports.insert("net".into(), "sqlx_core::driver_prelude::net".into()); }),
        ];
        for (name, mutate) in mutators {
            let mut m = MergedDecls::default();
            mutate(&mut m);
            assert_ne!(
                decl_index_digest(&m), empty,
                "MergedDecls.{name} changes the index but NOT the digest — the --incremental cache would \
                 reuse stale FnInfos. Fold `{name}` into decl_index_digest().",
            );
        }
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
            prefix, want_json: true, include_tests: false, policy: None, baseline: None, quiet: true, deps_idx: &idx,
        });
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
        let rc = run_with_deps(&d.to_string_lossy(), String::new(), true, false, None, None);
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
                baseline: None, quiet: true, deps_idx: &idx,
            });
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
