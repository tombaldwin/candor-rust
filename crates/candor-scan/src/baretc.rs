//! SOUNDNESS R1908 / R1914 / R1899 (lane rust-macropath-l3) — R-BARE-TC: which BARE invocations of a toolchain macro name may
//! not bind the toolchain's macro, by crate kind and by MODULE.
//!
//! THE QUESTION. `thread_local! { .. }` at item position is the toolchain's macro unless something rebinds the name in that
//! module. R1899 made the answer "hidden text" whenever this FILE held a glob `use dep::*;` or a `#[macro_use] extern crate dep;`.
//! Executed (`soundness/macropath/tcimmune.sh`; matrix group E):
//! - **A glob never rebinds a name of the std macro prelude in a crate that has it** (E0659: "glob import vs any other name from
//!   an outer scope"), so every hold a glob caused there is FALSE. A crate has std's macro prelude in every configuration unless
//!   its root says `no_std` / `no_core` in some form (`#![no_std]`, `#![cfg_attr(.., no_std)]`) and holds no `#[macro_use]
//!   extern crate std;`, and no module says `no_implicit_prelude` (`F3`).
//! - **In a crate without it, a glob still never rebinds a name of the CORE prelude** (`write!`, `panic!`, `assert!`, E0659
//!   again) and does rebind the rest (`thread_local!`, `vec!`, `println!`, `dbg!`, `asm!`, ..): `GLOB_IMMUNE_CORE`.
//! - **A glob rebinds only in the module that holds it** (imports do not inherit into an inline `mod`, and an invocation is read
//!   in its own module), so the question is asked per MODULE (`F4`): does a glob of THIS module's item list supply a name a
//!   dependency may export? That is true of a foreign glob, of a glob whose target the engine cannot place, and of a glob of a
//!   local module that (transitively) holds such a glob or imports a toolchain-named macro, or whose item-position macro text
//!   it cannot read as writing no `use`. `use Kind::*;` over an enum supplies nothing.
//!
//! Everything else R1899 held stays held: a `use dep::thread_local;` in the invocation's own module (`non_toolchain_imports`,
//! now per module), a `#[macro_use] extern crate dep;` of the file, a `macro_rules!` of the name.
//!
//! The facts are computed once per scan from the R-ROOT file summaries (`rroot::FileSummary`), by module tree placement, and
//! stored in the crate macro table under `TC_PREFIX` so their digest joins every file's cache key. A file the tree does not
//! place under exactly one root has no entry and keeps R1899's reading. `CANDOR_R_BARE_TC_OFF=1` computes none.
use crate::rroot::{self, FileSummary};
use std::collections::{BTreeMap, HashMap, HashSet};

/// The key prefix of a file's R-BARE-TC entry in `mbe::CrateMacros`: `TC_PREFIX + file`. No macro name begins with `\u{7}`.
pub(crate) const TC_PREFIX: &str = "\u{7}tc:";

pub(crate) fn off() -> bool {
    static OFF: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *OFF.get_or_init(|| candor_report::env::switch("CANDOR_R_BARE_TC_OFF"))
}

/// The names of `mbe::TOOLCHAIN_BANG_MACROS` a glob can NOT rebind in a crate with std's macro prelude: each one E0659s when a
/// dependency's `use dep::*;` supplies the name (`tcimmune.sh`, rustc 1.99, editions 2015/2018/2021 alike). A name missing here
/// stays rebindable (the hold direction): `try`, `ready`, `addr_of`, `pin`, `asm`, `format_args_nl`, ..
pub(crate) const GLOB_IMMUNE_STD: &[&str] = &[
    "assert", "assert_eq", "assert_ne", "cfg", "column", "compile_error", "concat", "const_format_args", "dbg", "debug_assert",
    "debug_assert_eq", "debug_assert_ne", "env", "eprint", "eprintln", "file", "format", "format_args", "include", "include_bytes",
    "include_str", "is_x86_feature_detected", "line", "log_syntax", "matches", "module_path", "option_env", "panic", "print",
    "println", "stringify", "thread_local", "todo", "trace_macros", "unimplemented", "unreachable", "vec", "write", "writeln",
];

/// The same, in a crate WITHOUT std's macro prelude (`#![no_std]`): the core prelude's macros only.
pub(crate) const GLOB_IMMUNE_CORE: &[&str] = &[
    "assert", "assert_eq", "assert_ne", "cfg", "column", "compile_error", "concat", "const_format_args", "debug_assert",
    "debug_assert_eq", "debug_assert_ne", "env", "file", "format_args", "include", "include_bytes", "include_str", "line",
    "log_syntax", "matches", "module_path", "option_env", "panic", "stringify", "todo", "trace_macros", "unimplemented",
    "unreachable", "write", "writeln",
];

/// What one file's splice reads (see the module doc).
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct TcFile {
    /// F3: the crate root has std's macro prelude in every configuration
    pub f3: bool,
    /// the inline module paths of this file (`""` = the file's own list, `"a::b"` an inline module) whose own globs may supply a
    /// name a dependency exports
    pub supply: HashSet<String>,
    /// design revision 2 (D1): `(inline module path, toolchain name)` for each explicit `use` leaf of this file that is rooted at
    /// `crate` / `self` / `super` and LANDS on a re-export that may be a dependency's (a foreign glob or name, an unplaced module,
    /// unreadable text, or nothing the engine can place), at any cfg twin
    pub uses: HashSet<(String, String)>,
}

impl TcFile {
    fn immune(&self, name: &str) -> bool {
        (if self.f3 { GLOB_IMMUNE_STD } else { GLOB_IMMUNE_CORE }).contains(&name)
    }

    /// May a glob of the module at inline path `modkey` rebind the bare toolchain name `name`?
    pub(crate) fn glob_rebinds(&self, name: &str, modkey: &str) -> bool {
        self.supply.contains(modkey) && !self.immune(name)
    }

    /// Does an explicit `use` of `name` in the module at `modkey` (`None`: any module of the file) land on a dependency's macro?
    pub(crate) fn use_lands_foreign(&self, name: &str, modkey: Option<&str>) -> bool {
        match modkey {
            Some(k) => self.uses.contains(&(k.to_string(), name.to_string())),
            None => self.uses.iter().any(|(_, n)| n == name),
        }
    }

    /// May a glob of ANY module of this file? (a reading that stands for every nested module at once)
    pub(crate) fn glob_rebinds_anywhere(&self, name: &str) -> bool {
        !self.supply.is_empty() && !self.immune(name)
    }

    /// The table value. It carries every bit the splice reads, so the crate-macro digest (a cache key) moves with it; the module
    /// names are folded into a checksum, not written, so no scan of the table for a word can match a module's name.
    fn encode(&self) -> String {
        use std::hash::{Hash, Hasher};
        let mut v: Vec<&str> = self.supply.iter().map(String::as_str).collect();
        v.sort_unstable();
        let mut u: Vec<(&str, &str)> = self.uses.iter().map(|(a, b)| (a.as_str(), b.as_str())).collect();
        u.sort_unstable();
        let mut h = std::collections::hash_map::DefaultHasher::new();
        v.hash(&mut h);
        u.hash(&mut h);
        format!("{}|{}|{}|{:x}", u8::from(self.f3), v.len(), u.len(), h.finish())
    }
}

#[derive(Default, Debug)]
pub(crate) struct TcFacts {
    pub files: BTreeMap<String, TcFile>,
}

impl TcFacts {
    pub(crate) fn table(&self) -> Vec<(String, String)> {
        self.files.iter().map(|(f, t)| (format!("{TC_PREFIX}{f}"), t.encode())).collect()
    }

    pub(crate) fn file(&self, rel: &str) -> Option<TcFile> {
        self.files.get(rel).cloned()
    }
}

/// The module a glob names, as far as the tree tells.
enum Target {
    /// a dependency, an unplaced module, or a path this pass cannot follow: it may supply anything
    Supplies,
    /// a toolchain path, or an enum / struct / trait / alias of this crate: it supplies no macro
    Nothing,
    Module((String, Vec<String>)),
}

/// Build every file's facts from the R-ROOT summaries. `edition2018(root)` as in `rroot::build`.
pub(crate) fn build(summaries: &HashMap<String, FileSummary>, edition2018: &dyn Fn(&str) -> bool) -> TcFacts {
    let mut facts = TcFacts::default();
    if off() {
        return facts;
    }
    let rroot::Placed { placed, module_all, unplaced } = rroot::place(summaries);
    // ---- F3, per root. A file that is not a crate root by name (`fuzz/fuzz_targets/x.rs`, a `[[bin]] path`) yet says `no_std` is
    // a crate root the placement does not know: any file it `#[path]`-includes may be compiled under it, so no file is promised
    // std's prelude.
    let any_no_prelude = summaries.values().any(|s| s.no_implicit_prelude);
    let unknown_root = summaries.iter().any(|(f, s)| s.root_no_std && !rroot::is_root_file(f));
    let f3_of = |root: &str| -> bool {
        summaries.get(root).is_some_and(|s| !any_no_prelude && !unknown_root && (!s.root_no_std || s.root_macro_use_std))
    };
    // ---- F4: the fixpoint over the module graph
    let ids: Vec<(String, Vec<String>)> = {
        let mut v: Vec<_> = module_all.keys().cloned().collect();
        v.sort();
        v
    };
    let ix: HashMap<&(String, Vec<String>), usize> = ids.iter().enumerate().map(|(i, k)| (k, i)).collect();
    let n = ids.len();
    let mut base = vec![false; n]; // an own unread invocation, or a glob that may supply anything
    let mut tc_names = vec![false; n]; // an own `use` binding a toolchain-named macro through a non-toolchain path
    let mut edges: Vec<Vec<usize>> = vec![Vec::new(); n];
    for (i, (root, mp)) in ids.iter().enumerate() {
        let ed = edition2018(root);
        for (file, mix) in &module_all[&ids[i]] {
            let m = &summaries[file].mods[*mix];
            base[i] |= m.unread;
            tc_names[i] |= m.tc_names;
            for (segs, abs) in &m.globs {
                match resolve(root, mp, segs, *abs, ed, &module_all, &unplaced, summaries) {
                    Target::Supplies => base[i] = true,
                    Target::Nothing => {}
                    Target::Module(k) => match ix.get(&k) {
                        Some(&j) => edges[i].push(j),
                        None => base[i] = true,
                    },
                }
            }
        }
    }
    // exports[T]: a glob OF T supplies; supplies[M]: a glob IN M supplies
    let mut exports: Vec<bool> = (0..n).map(|i| base[i] || tc_names[i]).collect();
    loop {
        let mut changed = false;
        for i in 0..n {
            if !exports[i] && edges[i].iter().any(|&j| exports[j]) {
                exports[i] = true;
                changed = true;
            }
        }
        if !changed {
            break;
        }
    }
    let supplies: Vec<bool> = (0..n).map(|i| base[i] || edges[i].iter().any(|&j| exports[j])).collect();
    // ---- per file: only a file exactly one root places (as R-ROOT's `files`)
    for (file, ps) in &placed {
        let roots: HashSet<&String> = ps.iter().map(|p| &p.root).collect();
        if roots.len() != 1 || ps.len() != 1 {
            continue;
        }
        let p = &ps[0];
        // A crate whose root holds a non-toolchain `#[macro_use] extern crate dep;` may bind the name there (the macro_use prelude
        // outranks std's, E0659 does not apply) with or without a glob, which is L3(c)'s question and not shipped. R1899's
        // reading of a glob (any glob of the file holds) was right for such a crate by accident; it stays exactly as it was.
        if summaries.get(&p.root).is_some_and(|s| s.root_macro_use_dep) {
            continue;
        }
        let mut t = TcFile { f3: f3_of(&p.root), ..Default::default() };
        let ed = edition2018(&p.root);
        for m in &summaries[file].mods {
            let mut full = p.modpath.clone();
            full.extend(m.inline.iter().cloned());
            if ix.get(&(p.root.clone(), full.clone())).is_some_and(|&i| supplies[i]) {
                t.supply.insert(m.inline.join("::"));
            }
            // D1: this module's explicit `use` leaves of a toolchain name, rooted at crate / self / super, by where they LAND
            for (bound, segs, leaf, abs) in &m.full_uses {
                if *abs || !crate::mbe::TOOLCHAIN_BANG_MACROS.contains(&bound.as_str()) {
                    continue;
                }
                if !matches!(segs.first().map(String::as_str), Some("crate" | "self" | "super")) {
                    continue; // a dependency or bare head: arm 2's head test (`non_toolchain_imports_in`) answers
                }
                let mut seen = HashSet::new();
                let land = lands(&p.root, &full, segs, leaf, ed, &module_all, &unplaced, summaries, &mut seen);
                if !matches!(land, Prov::Safe) {
                    t.uses.insert((m.inline.join("::"), bound.clone()));
                }
            }
        }
        facts.files.insert(file.clone(), t);
    }
    facts
}

/// What a name resolves to: provably the toolchain's or a local `macro_rules!`; may be a dependency's (or unreadable); or nothing found.
#[derive(Clone, Copy, PartialEq)]
enum Prov {
    Safe,
    Foreign,
    Nothing,
}

fn join(a: Prov, b: Prov) -> Prov {
    match (a, b) {
        (Prov::Foreign, _) | (_, Prov::Foreign) => Prov::Foreign,
        (Prov::Safe, _) | (_, Prov::Safe) => Prov::Safe,
        _ => Prov::Nothing,
    }
}

/// Where does the explicit `use <segs>::<leaf>;` written in module `(root, mp)` land? `segs` starts with `crate`/`self`/`super`.
/// A target module that is not placed, a foreign path or glob on the way, unreadable item text, and a name nothing provides are all
/// "may be a dependency's" (`Foreign`); every cfg twin of a module is read.
#[allow(clippy::too_many_arguments)]
fn lands(
    root: &str,
    mp: &[String],
    segs: &[String],
    leaf: &str,
    ed2018: bool,
    modules: &HashMap<(String, Vec<String>), Vec<(String, usize)>>,
    unplaced: &HashSet<(String, Vec<String>)>,
    summaries: &HashMap<String, FileSummary>,
    seen: &mut HashSet<(String, Vec<String>, String)>,
) -> Prov {
    // the module the path names: resolve `segs` as a glob path would be, but require a MODULE
    let target = match resolve(root, mp, segs, false, ed2018, modules, unplaced, summaries) {
        Target::Module((_, p)) => p,
        _ => return Prov::Foreign,
    };
    provides(root, &target, leaf, ed2018, modules, unplaced, summaries, seen)
}

/// Does module `(root, mp)` provide `name`, and is it the toolchain's / local, or possibly a dependency's?
#[allow(clippy::too_many_arguments)]
fn provides(
    root: &str,
    mp: &[String],
    name: &str,
    ed2018: bool,
    modules: &HashMap<(String, Vec<String>), Vec<(String, usize)>>,
    unplaced: &HashSet<(String, Vec<String>)>,
    summaries: &HashMap<String, FileSummary>,
    seen: &mut HashSet<(String, Vec<String>, String)>,
) -> Prov {
    let key = (root.to_string(), mp.to_vec());
    if unplaced.contains(&key) {
        return Prov::Foreign;
    }
    let Some(defs) = modules.get(&key) else { return Prov::Foreign };
    if !seen.insert((root.to_string(), mp.to_vec(), name.to_string())) {
        return Prov::Nothing; // a cycle adds nothing
    }
    let mut acc = Prov::Nothing;
    for (file, ix) in defs {
        let m = &summaries[file].mods[*ix];
        if m.unread {
            return Prov::Foreign;
        }
        if m.rules.iter().any(|r| r == name) {
            acc = join(acc, Prov::Safe);
        }
        for (bound, segs, leaf, abs) in &m.full_uses {
            if bound != name {
                continue;
            }
            let head = segs.first().map(String::as_str);
            let one = if *abs {
                if matches!(head.or(Some(leaf.as_str())), Some("std" | "core" | "alloc")) { Prov::Safe } else { Prov::Foreign }
            } else {
                match head {
                    Some("std" | "core" | "alloc") if !modules.contains_key(&(root.to_string(), vec![head.unwrap().to_string()])) => Prov::Safe,
                    None => {
                        // `use name;` / `use name as other;`: a macro of this module's textual scope (a local definition), else unknown
                        if m.rules.iter().any(|r| r == leaf) { Prov::Safe } else { Prov::Foreign }
                    }
                    Some("crate" | "self" | "super") => lands(root, mp, segs, leaf, ed2018, modules, unplaced, summaries, seen),
                    // a bare head: a child module of this one (2018) or of the root (2015) is a local path; anything else a dependency
                    Some(h) => {
                        let local = if ed2018 { mp.iter().cloned().chain([h.to_string()]).collect::<Vec<_>>() } else { vec![h.to_string()] };
                        if modules.contains_key(&(root.to_string(), local)) {
                            let mut full = vec!["self".to_string()];
                            full.extend(segs.iter().cloned());
                            let full = if ed2018 { full } else { std::iter::once("crate".to_string()).chain(segs.iter().cloned()).collect() };
                            lands(root, mp, &full, leaf, ed2018, modules, unplaced, summaries, seen)
                        } else {
                            Prov::Foreign
                        }
                    }
                }
            };
            acc = join(acc, one);
        }
        for (g, abs) in &m.globs {
            let one = match resolve(root, mp, g, *abs, ed2018, modules, unplaced, summaries) {
                Target::Supplies => Prov::Foreign,
                Target::Nothing => {
                    // a glob of the toolchain's own crate supplies the toolchain's macro of the name; an enum supplies nothing
                    if matches!(g.first().map(String::as_str), Some("std" | "core" | "alloc")) { Prov::Safe } else { Prov::Nothing }
                }
                Target::Module((_, t)) => provides(root, &t, name, ed2018, modules, unplaced, summaries, seen),
            };
            acc = join(acc, one);
        }
    }
    acc
}

/// Follow a glob's path from the module `(root, mp)`.
#[allow(clippy::too_many_arguments)]
fn resolve(
    root: &str,
    mp: &[String],
    segs: &[String],
    abs: bool,
    ed2018: bool,
    modules: &HashMap<(String, Vec<String>), Vec<(String, usize)>>,
    unplaced: &HashSet<(String, Vec<String>)>,
    summaries: &HashMap<String, FileSummary>,
) -> Target {
    if abs || segs.is_empty() {
        return Target::Supplies;
    }
    let head = segs[0].as_str();
    if matches!(head, "std" | "core" | "alloc") && !modules.contains_key(&(root.to_string(), vec![head.to_string()])) {
        return Target::Nothing;
    }
    let mut cur: Vec<String>;
    let mut rest: &[String] = segs;
    match head {
        "crate" => {
            cur = Vec::new();
            rest = &segs[1..];
        }
        "self" => {
            cur = mp.to_vec();
            rest = &segs[1..];
        }
        "super" => {
            cur = mp.to_vec();
            while rest.first().map(String::as_str) == Some("super") {
                if cur.pop().is_none() {
                    return Target::Supplies;
                }
                rest = &rest[1..];
            }
        }
        // 2018 uniform paths: relative to this module; 2015: relative to the root
        _ => cur = if ed2018 { mp.to_vec() } else { Vec::new() },
    }
    let key = |p: &[String]| (root.to_string(), p.to_vec());
    for (i, seg) in rest.iter().enumerate() {
        let last = i + 1 == rest.len();
        let mut next = cur.clone();
        next.push(seg.clone());
        if modules.contains_key(&key(&next)) {
            cur = next;
            continue;
        }
        if unplaced.contains(&key(&next)) {
            return Target::Supplies;
        }
        // not a module: only an enum/struct/trait/alias at the END of the path is understood
        let declares = last
            && modules
                .get(&key(&cur))
                .is_some_and(|defs| defs.iter().any(|(f, mix)| summaries[f].mods[*mix].types.iter().any(|t| t == seg)));
        return if declares { Target::Nothing } else { Target::Supplies };
    }
    if modules.contains_key(&key(&cur)) {
        Target::Module(key(&cur))
    } else {
        Target::Supplies
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn facts(files: &[(&str, &str)]) -> TcFacts {
        let m: HashMap<String, FileSummary> = files
            .iter()
            .map(|(r, src)| {
                let f: syn::File = syn::parse_str(src).unwrap();
                (r.to_string(), rroot::summarize(&f.items, &f.attrs, None))
            })
            .collect();
        build(&m, &|_| true)
    }

    #[test]
    fn f3_is_std_prelude_in_every_configuration() {
        for (root, f3) in [
            ("mod a;", true),
            ("#![no_std] mod a;", false),
            ("#![cfg_attr(not(feature = \"std\"), no_std)] mod a;", false),
            ("#![no_std] #[macro_use] extern crate std; mod a;", true),
            ("#![no_std] #[cfg(feature = \"std\")] #[macro_use] extern crate std; mod a;", false),
            ("#![no_std] extern crate std; mod a;", false),
        ] {
            let f = facts(&[("src/lib.rs", root), ("src/a.rs", "use dep::*;")]);
            assert_eq!(f.files["src/a.rs"].f3, f3, "{root}");
        }
        let f = facts(&[("src/lib.rs", "mod a;"), ("src/a.rs", "")]);
        assert!(f.files["src/a.rs"].f3);
        let f = facts(&[("src/lib.rs", "mod a;"), ("src/a.rs", "#[no_implicit_prelude] mod q {}")]);
        assert!(!f.files["src/a.rs"].f3, "no_implicit_prelude anywhere: no std prelude is promised");
    }

    #[test]
    fn a_glob_supplies_per_module() {
        let f = facts(&[("src/lib.rs", "mod a; mod b; mod c;"), ("src/a.rs", "use dep::*; mod x { }"), ("src/b.rs", "mod x { use dep::*; }"), ("src/c.rs", "use super::b::*;")]);
        assert_eq!(f.files["src/a.rs"].supply, ["".to_string()].into_iter().collect());
        assert_eq!(f.files["src/b.rs"].supply, ["x".to_string()].into_iter().collect(), "an inline module's glob is its own");
        assert!(f.files["src/c.rs"].supply.is_empty(), "b's own glob sits in b::x, not in b: nothing leaks through b");
    }

    #[test]
    fn local_globs_follow_the_tree_and_unplaced_targets_supply() {
        let lib = "mod prelude { pub use dep::*; } mod pure { pub fn f() {} } mod chain { pub use crate::prelude::*; } mod a; mod b; mod d; mod e; mod g; mod h; \
                   #[path = \"elsewhere.rs\"] mod hidden;";
        let f = facts(&[
            ("src/lib.rs", lib),
            ("src/a.rs", "use crate::prelude::*;"),
            ("src/b.rs", "use crate::pure::*;"),
            ("src/d.rs", "use crate::chain::*;"),
            ("src/e.rs", "use crate::hidden::*;"),
            ("src/g.rs", "enum K { A } use self::K::*; use std::fmt::*;"),
            ("src/h.rs", "use crate::nosuch::*;"),
        ]);
        let sup = |n: &str| f.files[&format!("src/{n}.rs")].supply.contains("");
        assert!(sup("a"), "a glob of a module holding a foreign glob (E9)");
        assert!(!sup("b"), "a glob of a pure local module");
        assert!(sup("d"), "a glob of a glob of a foreign glob");
        assert!(sup("e"), "a #[path] module is not placed");
        assert!(!sup("g"), "an enum glob and a toolchain glob supply nothing");
        assert!(sup("h"), "an unresolvable path");
    }

    #[test]
    fn macro_text_the_summary_cannot_read_supplies() {
        // E10 (g7): a local macro that writes a `use`, invoked in the module a glob names
        let f = facts(&[
            ("src/lib.rs", "mod prelude; mod a;"),
            ("src/prelude.rs", "macro_rules! reexp { () => { pub use dep::*; } } reexp!();"),
            ("src/a.rs", "use crate::prelude::*;"),
        ]);
        assert!(f.files["src/a.rs"].supply.contains(""));
        assert!(f.files["src/prelude.rs"].supply.contains(""), "the emitting module's own invocations see the glob it emitted");
        // a clean local macro (no `use`, no nested invocation) leaves the module as written; a toolchain invocation too
        let f = facts(&[("src/lib.rs", "mod pure; mod a;"), ("src/pure.rs", "macro_rules! mk { () => { pub fn f() {} } } mk!(); thread_local! { static X: u8 = 0; }"), ("src/a.rs", "use crate::pure::*;")]);
        assert!(!f.files["src/a.rs"].supply.contains(""));
        // a foreign item macro may write anything
        let f = facts(&[("src/lib.rs", "mod pure; mod a;"), ("src/pure.rs", "dep::reexport!();"), ("src/a.rs", "use crate::pure::*;")]);
        assert!(f.files["src/a.rs"].supply.contains(""));
        // a toolchain-named import is a supply too
        let f = facts(&[("src/lib.rs", "mod pure; mod a;"), ("src/pure.rs", "pub use dep::thread_local;"), ("src/a.rs", "use crate::pure::*;")]);
        assert!(f.files["src/a.rs"].supply.contains(""));
        let f = facts(&[("src/lib.rs", "mod pure; mod a;"), ("src/pure.rs", "pub use dep::other;"), ("src/a.rs", "use crate::pure::*;")]);
        assert!(!f.files["src/a.rs"].supply.contains(""));
    }

    #[test]
    fn cfg_twin_modules_and_two_roots_are_not_guessed() {
        let f = facts(&[("src/lib.rs", "#[cfg(unix)] mod p { pub use dep::*; } #[cfg(not(unix))] mod p { } mod a;"), ("src/a.rs", "use crate::p::*;")]);
        assert!(f.files["src/a.rs"].supply.contains(""), "one twin supplies: the glob may");
        let f = facts(&[("src/lib.rs", "mod both;"), ("src/main.rs", "mod both;"), ("src/both.rs", "use dep::*;")]);
        assert!(!f.files.contains_key("src/both.rs"), "two roots place it: no entry, R1899's reading stands");
    }

    #[test]
    fn a_root_with_a_macro_use_dependency_and_an_unknown_root_keep_r1899() {
        // a glob in a std crate whose root has `#[macro_use] extern crate dep;`: dep's macro outranks std's, so R1899's hold
        // (accidentally right) is kept: no entry, main's reading
        let f = facts(&[("src/lib.rs", "#[macro_use] extern crate dep; mod a;"), ("src/a.rs", "use other::*;")]);
        assert!(!f.files.contains_key("src/a.rs"));
        // `extern crate std` is not a dependency
        let f = facts(&[("src/lib.rs", "#[macro_use] extern crate std; mod a;"), ("src/a.rs", "use other::*;")]);
        assert!(f.files.contains_key("src/a.rs"));
        // a file that says no_std without being a crate root by name: nobody is promised std's prelude
        let f = facts(&[("src/lib.rs", "mod a;"), ("src/a.rs", ""), ("fuzz/fuzz_targets/t.rs", "#![no_std] #[path = \"../../src/a.rs\"] mod a;")]);
        assert!(!f.files["src/a.rs"].f3);
    }

    #[test]
    fn explicit_uses_are_classified_by_where_they_land() {
        let uses = |f: &TcFacts, file: &str, m: &str| f.files[file].uses.contains(&(m.to_string(), "thread_local".to_string()));
        // g5 / g5c: a same-file prelude holding a dependency glob; g5n: by name; g5x: another file; g7: super::
        let f = facts(&[
            ("src/lib.rs", "mod inner; mod prelude; mod p2; mod x;"),
            ("src/inner.rs", "mod pre { pub use dep::*; } use self::pre::thread_local; use crate::inner::pre::thread_local as tl2;"),
            ("src/prelude.rs", "pub use dep::*;"),
            ("src/p2.rs", "pub use dep::thread_local;"),
            ("src/x.rs", "use crate::prelude::thread_local; use super::p2::thread_local as t3;"),
        ]);
        assert!(uses(&f, "src/inner.rs", ""), "g5, self::");
        assert!(uses(&f, "src/x.rs", ""), "g5x / g5n, crate:: and super::");
        // controls: a std re-export, a local macro, a std glob
        let f = facts(&[
            ("src/lib.rs", "mod inner; mod prelude; mod loc; mod sg; mod a;"),
            ("src/inner.rs", "mod pre { pub use std::thread_local; } use self::pre::thread_local;"),
            ("src/prelude.rs", "pub use std::thread_local;"),
            ("src/loc.rs", "macro_rules! thread_local { () => {} } pub(crate) use thread_local;"),
            ("src/sg.rs", "pub use std::*;"),
            ("src/a.rs", "use crate::prelude::thread_local; use crate::loc::thread_local as t2; use crate::sg::thread_local as t3;"),
        ]);
        assert!(!uses(&f, "src/inner.rs", "") && !uses(&f, "src/a.rs", ""), "toolchain and local landings do not hold");
        // cfg twins: one twin foreign holds; an unplaced or unknown module holds
        let f = facts(&[
            ("src/lib.rs", "mod prelude; mod a; mod b;"),
            ("src/prelude.rs", "#[cfg(feature = \"loom\")] pub use dep::thread_local; #[cfg(not(feature = \"loom\"))] pub use std::thread_local;"),
            ("src/a.rs", "use crate::prelude::thread_local;"),
            ("src/b.rs", "use crate::nosuch::thread_local;"),
        ]);
        assert!(uses(&f, "src/a.rs", "") && uses(&f, "src/b.rs", ""), "twins and unplaced modules hold");
    }

    #[test]
    fn macro_use_is_read_through_cfg_attr() {
        for (root, kept) in [
            ("#[cfg_attr(feature = \"m\", macro_use)] extern crate dep; mod a;", true),
            ("#[cfg_attr(unix, cfg_attr(feature = \"m\", macro_use))] extern crate dep; mod a;", true),
            ("#[macro_use] extern crate dep as d; mod a;", true),
            ("#[cfg_attr(feature = \"m\", macro_use)] extern crate std; mod a;", false),
        ] {
            let f = facts(&[("src/lib.rs", root), ("src/a.rs", "")]);
            assert_eq!(!f.files.contains_key("src/a.rs"), kept, "{root}");
        }
    }

    #[test]
    fn immunity_tables() {
        let t = TcFile { f3: true, supply: ["".to_string()].into_iter().collect(), ..Default::default() };
        assert!(!t.glob_rebinds("thread_local", "") && !t.glob_rebinds("write", "") && t.glob_rebinds("asm", ""));
        let t = TcFile { f3: false, supply: ["".to_string()].into_iter().collect(), ..Default::default() };
        assert!(t.glob_rebinds("thread_local", "") && !t.glob_rebinds("write", "") && !t.glob_rebinds("thread_local", "x"));
        for n in GLOB_IMMUNE_CORE {
            assert!(GLOB_IMMUNE_STD.contains(n), "{n}: a core-prelude name is a std-prelude name");
        }
    }
}
