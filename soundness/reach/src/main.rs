//! candor-reach — the drop-glue REACH probe. SOUNDNESS R766.
//!
//! WHAT THIS IS FOR, and why it lives in the repo rather than in a scratchpad.
//!
//! R766: the previous reach probe counted *the crate declares a local `impl Drop`* AND *the shape
//! appears*. That is not the conjunction a FIX needs. The engine only ever marks a construction
//! whose leaf is in `drop_relevant` (`collector.rs:694`), so a shape landing on a destructor-less
//! type cannot move a row no matter what the drop/escape model is changed to. R189 was ordered at
//! "146 sites across 60 of 473 crates" on the old predicate and measured, with the fix in hand,
//! REACH 204 / ADDED 0 / REMOVED 0 / CHANGED 0: the decision fired 204 times and not one row
//! followed. A whole work queue was ordered on numbers of that kind.
//!
//! It is in `soundness/` because R288 is the same failure one level up: fifteen scratchpad copies of
//! one A/B script, every one keyed wrong because it was copied from the last one. A measurement
//! procedure with no owner gets re-derived per lane and each copy carries its own defect.
//!
//! USAGE
//!     cargo run --release -- --crates <dir>      # each immediate subdirectory is one crate
//!     cargo run --release -- --crate  <dir>      # one crate
//!     ... --hits N        print N sample TIGHT hits per row (they are the audit trail)
//!     ... --tsv FILE      write every hit as TSV
//!     ... --expect FILE   CALIBRATION MODE: assert per-row tight/loose counts, exit 1 on any miss
//!
//! READ `soundness/REACH.md` BEFORE QUOTING A NUMBER FROM IT. It states what this probe cannot see,
//! in the same breath as the counts, which is the discipline R766 exists to install.

mod index;
mod rows;
mod shapes;

use index::{has_cfg_test, CrateIndex};
use shapes::Hit;
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::path::{Path, PathBuf};

fn rs_files(root: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(d) = stack.pop() {
        let rd = match std::fs::read_dir(&d) {
            Ok(x) => x,
            Err(_) => continue,
        };
        for e in rd.flatten() {
            let p = e.path();
            let name = p.file_name().and_then(|s| s.to_str()).unwrap_or("").to_string();
            if p.is_dir() {
                // NOT scanned: test/bench/example/fuzz trees and vendored deps. The engine's default
                // is `include_tests = false`, so counting them would inflate reach with code no
                // user's gate runs over. Stated in REACH.md as a deliberate exclusion.
                if matches!(name.as_str(), "tests" | "benches" | "examples" | "fuzz" | "target" | ".git") {
                    continue;
                }
                stack.push(p);
            } else if name.ends_with(".rs") && name != "build.rs" {
                out.push(p);
            }
        }
    }
    out.sort();
    out
}

fn scan_crate(krate: &str, dir: &Path) -> (CrateIndex, Vec<Hit>) {
    let files = rs_files(dir);
    let mut parsed: Vec<(String, syn::File)> = Vec::new();
    let mut idx = CrateIndex::default();
    for f in &files {
        let src = match std::fs::read_to_string(f) {
            Ok(s) => s,
            Err(_) => {
                idx.files_failed += 1;
                continue;
            }
        };
        match syn::parse_file(&src) {
            Ok(ast) => {
                idx.files_ok += 1;
                let rel = f.strip_prefix(dir).unwrap_or(f).to_string_lossy().to_string();
                parsed.push((rel, ast));
            }
            Err(_) => idx.files_failed += 1,
        }
    }
    for (_, ast) in &parsed {
        idx.collect_items(&ast.items);
    }
    idx.finish();

    let mut hits = Vec::new();
    for (rel, ast) in &parsed {
        walk_items(&idx, krate, rel, "", &ast.items, &mut hits);
    }
    (idx, hits)
}

fn walk_items(
    idx: &CrateIndex,
    krate: &str,
    file: &str,
    prefix: &str,
    items: &[syn::Item],
    out: &mut Vec<Hit>,
) {
    for it in items {
        match it {
            syn::Item::Mod(m) => {
                if has_cfg_test(&m.attrs) {
                    continue;
                }
                if let Some((_, inner)) = &m.content {
                    let p = format!("{prefix}{}::", m.ident);
                    walk_items(idx, krate, file, &p, inner, out);
                }
            }
            syn::Item::Fn(f) => {
                if has_cfg_test(&f.attrs) {
                    continue;
                }
                let name = format!("{prefix}{}", f.sig.ident);
                let s = rows::Scope { krate, file, func: &name };
                rows::match_fn(idx, &s, &f.sig, &f.block, out);
            }
            syn::Item::Impl(im) => {
                let owner = match &*im.self_ty {
                    syn::Type::Path(tp) => {
                        tp.path.segments.last().map(|s| s.ident.to_string()).unwrap_or_default()
                    }
                    _ => String::new(),
                };
                for item in &im.items {
                    if let syn::ImplItem::Fn(m) = item {
                        if has_cfg_test(&m.attrs) {
                            continue;
                        }
                        let name = format!("{prefix}{owner}::{}", m.sig.ident);
                        let s = rows::Scope { krate, file, func: &name };
                        rows::match_fn(idx, &s, &m.sig, &m.block, out);
                    }
                }
            }
            syn::Item::Trait(tr) => {
                for item in &tr.items {
                    if let syn::TraitItem::Fn(m) = item {
                        if let Some(b) = &m.default {
                            let name = format!("{prefix}{}::{}", tr.ident, m.sig.ident);
                            let s = rows::Scope { krate, file, func: &name };
                            rows::match_fn(idx, &s, &m.sig, b, out);
                        }
                    }
                }
            }
            _ => {}
        }
    }
}

#[derive(Default, Clone)]
struct Tally {
    loose_sites: usize,
    tight_sites: usize,
    loose_crates: BTreeSet<String>,
    tight_crates: BTreeSet<String>,
    /// Crate NAMES with the version stripped, and the per-name site count.
    ///
    /// The census holds 1,625 ENTRIES but only 937 distinct names — `async-nats` at two versions,
    /// `h2` at two, `crossbeam-utils` at three. Every "N crates" figure the old ordering quoted
    /// counted entries, so a three-version crate voted three times. The distinct-name column and the
    /// concentration line below are what stop a number that is really one crate's house style from
    /// reading as a broad vein.
    tight_names: BTreeMap<String, usize>,
}

/// `async-nats-0.35.1` -> `async-nats`.
fn crate_name(entry: &str) -> String {
    let b = entry.as_bytes();
    let mut i = entry.len();
    while let Some(p) = entry[..i].rfind('-') {
        // a version segment starts with a digit
        if p + 1 < b.len() && b[p + 1].is_ascii_digit() {
            return entry[..p].to_string();
        }
        i = p;
        if i == 0 {
            break;
        }
    }
    entry.to_string()
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let mut mode_dir: Option<PathBuf> = None;
    let mut single = false;
    let mut nhits = 0usize;
    let mut tsv: Option<PathBuf> = None;
    let mut expect: Option<PathBuf> = None;
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--crates" => {
                mode_dir = Some(PathBuf::from(&args[i + 1]));
                i += 1
            }
            "--crate" => {
                mode_dir = Some(PathBuf::from(&args[i + 1]));
                single = true;
                i += 1
            }
            "--hits" => {
                nhits = args[i + 1].parse().unwrap_or(0);
                i += 1
            }
            "--tsv" => {
                tsv = Some(PathBuf::from(&args[i + 1]));
                i += 1
            }
            "--expect" => {
                expect = Some(PathBuf::from(&args[i + 1]));
                i += 1
            }
            x => {
                eprintln!("candor-reach: unknown argument {x}");
                std::process::exit(2)
            }
        }
        i += 1;
    }
    let Some(root) = mode_dir else {
        eprintln!("usage: candor-reach --crates <dir> | --crate <dir> [--hits N] [--tsv F] [--expect F]");
        std::process::exit(2);
    };

    let mut crates: Vec<(String, PathBuf)> = Vec::new();
    if single {
        let n = root.file_name().and_then(|s| s.to_str()).unwrap_or("crate").to_string();
        crates.push((n, root.clone()));
    } else {
        let mut v: Vec<PathBuf> = std::fs::read_dir(&root)
            .unwrap_or_else(|e| {
                eprintln!("candor-reach: {}: {e}", root.display());
                std::process::exit(2)
            })
            .flatten()
            .map(|e| e.path())
            .filter(|p| p.is_dir())
            .collect();
        v.sort();
        for p in v {
            let n = p.file_name().and_then(|s| s.to_str()).unwrap_or("?").to_string();
            crates.push((n, p));
        }
    }

    let mut tallies: BTreeMap<&'static str, Tally> = BTreeMap::new();
    for r in rows::ROWS {
        tallies.insert(r, Tally::default());
    }
    let mut all_hits: Vec<Hit> = Vec::new();
    let mut crates_with_drop = 0usize;
    let mut files_ok = 0usize;
    let mut files_failed = 0usize;
    let mut drop_types_total = 0usize;

    for (n, p) in &crates {
        let (idx, hits) = scan_crate(n, p);
        files_ok += idx.files_ok;
        files_failed += idx.files_failed;
        drop_types_total += idx.drop_types.len();
        if !idx.drop_types.is_empty() {
            crates_with_drop += 1;
        }
        for h in &hits {
            if let Some(t) = tallies.get_mut(h.row) {
                if h.tight {
                    t.tight_sites += 1;
                    t.tight_crates.insert(h.krate.clone());
                    *t.tight_names.entry(crate_name(&h.krate)).or_insert(0) += 1;
                } else {
                    t.loose_sites += 1;
                    t.loose_crates.insert(h.krate.clone());
                }
            }
        }
        all_hits.extend(hits);
    }

    // LOOSE is the superset: every tight hit is also a shape hit.
    for t in tallies.values_mut() {
        t.loose_sites += t.tight_sites;
        let add: Vec<String> = t.tight_crates.iter().cloned().collect();
        t.loose_crates.extend(add);
    }

    println!("candor-reach — drop-glue REACH, SOUNDNESS R766");
    println!(
        "corpus: {} crates, {} .rs files parsed, {} unparseable",
        crates.len(),
        files_ok,
        files_failed
    );
    println!(
        "drop-relevance: {crates_with_drop} crates declare a local `impl Drop` ({drop_types_total} types total)"
    );
    println!();
    println!(
        "{:<8} {:>10} {:>8} {:>10} {:>8} {:>7}   {}",
        "row", "LOOSE", "entries", "TIGHT", "entries", "names", "most concentrated in"
    );
    for (r, t) in &tallies {
        let mut top: Vec<(&String, &usize)> = t.tight_names.iter().collect();
        top.sort_by(|a, b| b.1.cmp(a.1).then(a.0.cmp(b.0)));
        let conc: String = top
            .iter()
            .take(3)
            .map(|(n, c)| format!("{n} {c}"))
            .collect::<Vec<_>>()
            .join(", ");
        println!(
            "{:<8} {:>10} {:>8} {:>10} {:>8} {:>7}   {}",
            r,
            t.loose_sites,
            t.loose_crates.len(),
            t.tight_sites,
            t.tight_crates.len(),
            t.tight_names.len(),
            conc
        );
    }

    if nhits > 0 {
        println!();
        println!("sample TIGHT hits (the audit trail — every one is checkable by hand):");
        let mut per: HashMap<&str, usize> = HashMap::new();
        for h in &all_hits {
            if !h.tight {
                continue;
            }
            let c = per.entry(h.row).or_insert(0);
            if *c >= nhits {
                continue;
            }
            *c += 1;
            println!("  {:<6} {}  {}:{}  {}  leaf={}", h.row, h.krate, h.file, h.line, h.func, h.leaf);
        }
    }

    if let Some(f) = tsv {
        let mut s = String::from("row\ttight\tcrate\tfile\tline\tfunc\tleaf\n");
        for h in &all_hits {
            s.push_str(&format!(
                "{}\t{}\t{}\t{}\t{}\t{}\t{}\n",
                h.row, h.tight, h.krate, h.file, h.line, h.func, h.leaf
            ));
        }
        std::fs::write(&f, s).expect("write tsv");
        eprintln!("candor-reach: wrote {}", f.display());
    }

    if let Some(f) = expect {
        // CALIBRATION MODE. Every line `row loose tight` must hold EXACTLY. A probe that has never
        // been shown to fail is not evidence; this is the mechanism that shows it.
        let txt = std::fs::read_to_string(&f).expect("read expectations");
        let mut bad = 0;
        println!();
        println!("CALIBRATION against {}", f.display());
        for line in txt.lines() {
            let line = line.split('#').next().unwrap_or("").trim();
            if line.is_empty() {
                continue;
            }
            let p: Vec<&str> = line.split_whitespace().collect();
            if p.len() != 3 {
                eprintln!("  bad expectation line: {line}");
                bad += 1;
                continue;
            }
            let (row, el, et) = (p[0], p[1].parse::<usize>().unwrap(), p[2].parse::<usize>().unwrap());
            let t = tallies.get(row).cloned().unwrap_or_default();
            let ok = t.loose_sites == el && t.tight_sites == et;
            println!(
                "  {:<6} loose {:>4} (want {:>4})   tight {:>4} (want {:>4})   {}",
                row,
                t.loose_sites,
                el,
                t.tight_sites,
                et,
                if ok { "OK" } else { "MISS" }
            );
            if !ok {
                bad += 1;
            }
        }
        if bad > 0 {
            println!("CALIBRATION FAILED — {bad} row(s) off. The probe is not evidence until this is 0.");
            std::process::exit(1);
        }
        println!("CALIBRATION OK");
    }
}
