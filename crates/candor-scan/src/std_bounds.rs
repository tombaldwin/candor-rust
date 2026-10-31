//! Call-target round 2 — THE STD BOUND TABLE: which user members a std entry point may run, read off std's own
//! declarations rather than kept as hand lists.
//!
//! `std_bounds.tsv` is GENERATED from the pinned toolchain's rust-src (`std_bounds_gen.rs`, driven by
//! `gen/std_bounds.sh`, which also `--check`s that the committed table reproduces). Each row is a declaration:
//!
//! * a trait (`T`): supertraits and members;
//! * an impl of a trait for a std shape (`I`): `impl<T: Debug, A> Debug for Vec<T, A>` — the TYPE ARGUMENT that must
//!   implement which traits, so formatting a `Vec<S>` may run `S`'s `Debug`;
//! * a std fn or method with a bounded generic parameter (`F`): `[T]::contains where T: PartialEq` — WHERE that parameter
//!   reaches the call (the receiver's type argument, the receiver itself, its `Item`, an argument, the return type).
//!
//! THE ONE RULE an engine reads off a row: a BOUND LICENSES THE BOUND TRAIT'S MEMBERS, AND ITS SUPERTRAITS', on the type
//! that fills the slot. "Licenses" is a may-run, never a must-run: `v.sort()` may run `T::cmp`, and whether this std
//! release happens to call `partial_cmp` instead is a body fact the table does not claim. The members licensed are a
//! trait's REQUIRED members — the ones every implementor defines, so an edge to `T::<member>` names the implementor's own
//! body; a provided member a type overrides is the stated residual (an override's edge would share the key space of an
//! inherent method of the same name).
//!
//! Everything here answers from the table alone; `collector.rs` owns the typing that fills a slot and the edge it forms.

use std::collections::{BTreeSet, HashMap};

static TABLE: &str = include_str!("std_bounds.tsv");

/// Where a bounded generic parameter reaches a call.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Slot {
    /// the receiver's (or implementing type's) `n`th type argument (`Vec<T>`'s `T` is `Elem(0)`)
    Elem(usize),
    /// every element of a tuple
    ElemAll,
    /// the receiver itself (a trait method's `Self`; a blanket impl's `T`)
    SelfRecv,
    /// the receiver's `Self::Item`
    Item,
    /// the `n`th non-receiver argument
    Arg(usize),
    /// only the return type (`str::parse::<F>`, `Iterator::collect::<B>`)
    Ret,
}

#[derive(Clone, Debug)]
pub(crate) struct SlotBound {
    pub(crate) slot: Slot,
    /// trait PATHS (`core::cmp::PartialEq`)
    pub(crate) traits: Vec<&'static str>,
    /// the slot's value is moved into the returned value (`Iterator::map`'s `Self`) rather than used at the call
    pub(crate) deferred: bool,
}

#[derive(Debug)]
pub(crate) struct TraitRow {
    pub(crate) sealed: bool,
    pub(crate) supers: Vec<&'static str>,
    pub(crate) required: Vec<&'static str>,
    pub(crate) provided: Vec<&'static str>,
    pub(crate) provided_deferred: Vec<&'static str>,
}

#[derive(Debug)]
pub(crate) struct FnRow {
    /// `trait:<path>`, `fn:<module path>`, or an inherent shape (`Vec`, `[]`, `str`, `Path`)
    pub(crate) owner: &'static str,
    pub(crate) nargs: usize,
    pub(crate) slots: Vec<SlotBound>,
}

#[derive(Default)]
pub(crate) struct Table {
    traits: HashMap<&'static str, TraitRow>,
    by_leaf: HashMap<&'static str, Vec<&'static str>>,
    /// member name → the open std traits declaring it
    declaring: HashMap<&'static str, Vec<&'static str>>,
    /// (trait path, shape) → one entry per impl
    impls: HashMap<String, Vec<Vec<SlotBound>>>,
    /// fn name → rows
    fns: HashMap<&'static str, Vec<FnRow>>,
    shapes: BTreeSet<&'static str>,
}

fn parse_slots(s: &'static str) -> Vec<SlotBound> {
    let mut out = Vec::new();
    for part in s.split(';').filter(|p| !p.is_empty()) {
        let Some((slot, rest)) = part.split_once('=') else { continue };
        let (traits, deferred) = match rest.strip_suffix(":d") {
            Some(t) => (t, true),
            None => (rest, false),
        };
        let slot = match slot {
            "s" => Slot::SelfRecv,
            "i" => Slot::Item,
            "r" => Slot::Ret,
            "e*" => Slot::ElemAll,
            x if x.starts_with('e') => match x[1..].parse() {
                Ok(n) => Slot::Elem(n),
                Err(_) => continue,
            },
            x if x.starts_with('a') => match x[1..].parse() {
                Ok(n) => Slot::Arg(n),
                Err(_) => continue,
            },
            _ => continue,
        };
        out.push(SlotBound { slot, traits: traits.split('+').collect(), deferred });
    }
    out
}

fn list(s: &'static str) -> Vec<&'static str> {
    s.split(',').filter(|x| !x.is_empty()).collect()
}

pub(crate) fn table() -> &'static Table {
    static T: std::sync::OnceLock<Table> = std::sync::OnceLock::new();
    T.get_or_init(|| {
        let mut t = Table::default();
        for line in TABLE.lines() {
            if line.starts_with('#') {
                continue;
            }
            let f: Vec<&'static str> = line.split('\t').collect();
            match f.first().copied() {
                Some("T") if f.len() >= 7 => {
                    let leaf = f[1].rsplit("::").next().unwrap_or(f[1]);
                    t.by_leaf.entry(leaf).or_default().push(f[1]);
                    t.traits.insert(
                        f[1],
                        TraitRow {
                            sealed: f[2] == "sealed",
                            supers: list(f[3]),
                            required: list(f[4]),
                            provided: list(f[5]),
                            provided_deferred: list(f[6]),
                        },
                    );
                }
                Some("I") if f.len() >= 4 => {
                    if f[2] != "*" && f[2].chars().next().is_some_and(|c| !c.is_lowercase()) {
                        t.shapes.insert(f[2]);
                    }
                    t.impls.entry(format!("{}\t{}", f[1], f[2])).or_default().push(parse_slots(f[3]));
                }
                Some("F") if f.len() >= 5 => {
                    if !f[1].contains(':') && (f[1] == "str" || f[1].chars().next().is_some_and(|c| !c.is_lowercase())) {
                        t.shapes.insert(f[1]);
                    }
                    t.fns.entry(f[2]).or_default().push(FnRow {
                        owner: f[1],
                        nargs: f[3].parse().unwrap_or(usize::MAX),
                        slots: parse_slots(f[4]),
                    });
                }
                _ => {}
            }
        }
        let mut declaring: HashMap<&'static str, Vec<&'static str>> = HashMap::new();
        for (k, r) in &t.traits {
            if r.sealed {
                continue;
            }
            for m in r.required.iter().chain(&r.provided).chain(&r.provided_deferred) {
                declaring.entry(m).or_default().push(k);
            }
        }
        for v in declaring.values_mut() {
            v.sort();
            v.dedup();
        }
        t.declaring = declaring;
        t
    })
}

/// A trait path's leaf (`core::fmt::Debug` → `Debug`).
pub(crate) fn leaf(path: &str) -> &str {
    path.rsplit("::").next().unwrap_or(path)
}

impl Table {
    pub(crate) fn trait_row(&self, path: &str) -> Option<&TraitRow> {
        self.traits.get(path)
    }

    /// The std trait a WRITTEN bound or trait name means: its leaf, disambiguated by a written module qualifier
    /// (`fmt::Write`, `io::Write`) where the leaf names more than one std trait. `None` when the name is not a std
    /// trait's, or stays ambiguous.
    pub(crate) fn resolve_trait(&self, written: &str) -> Option<&'static str> {
        let segs: Vec<&str> = written.split("::").collect();
        let leaf = segs.last()?;
        let cands = self.by_leaf.get(leaf)?;
        if cands.len() == 1 {
            return Some(cands[0]);
        }
        if segs.len() >= 2 {
            let q = segs[segs.len() - 2];
            let hit: Vec<&&str> = cands.iter().filter(|c| c.split("::").any(|s| s == q)).collect();
            if hit.len() == 1 {
                return Some(hit[0]);
            }
        }
        None
    }

    /// Every std trait path whose leaf is `leaf` (`Write` → io's and fmt's).
    pub(crate) fn traits_by_leaf(&self, leaf: &str) -> &[&'static str] {
        self.by_leaf.get(leaf).map(Vec::as_slice).unwrap_or(&[])
    }

    /// The trait's OWN required members (no supertraits), as (trait leaf, member). A sealed trait has none user-side.
    pub(crate) fn own_required(&self, path: &str) -> Vec<(&'static str, &'static str)> {
        match self.traits.get_key_value(path) {
            Some((&k, r)) if !r.sealed => r.required.iter().map(|m| (leaf(k), *m)).collect(),
            _ => Vec::new(),
        }
    }

    /// THE RULE: the members a bound on `path` licenses — its REQUIRED members and every supertrait's, as
    /// (declaring trait leaf, member). A sealed trait licenses nothing (no user type implements it).
    pub(crate) fn licensed(&self, path: &str) -> Vec<(&'static str, &'static str)> {
        let mut out: Vec<(&'static str, &'static str)> = Vec::new();
        let mut seen: BTreeSet<&str> = BTreeSet::new();
        let mut stack: Vec<&str> = vec![path];
        while let Some(p) = stack.pop() {
            if !seen.insert(p) {
                continue;
            }
            let Some((&key, row)) = self.traits.get_key_value(p) else { continue };
            if row.sealed {
                continue;
            }
            // The `Fn` family's own member (`call_once`, `extern "rust-call"`) is unstable to IMPLEMENT, so the table does
            // not list it — but a bound on it is exactly "this argument is called". It is licensed as the pseudo-member
            // `call`, which the collector reads as "invoke the callable argument".
            if matches!(key, FN_ONCE | FN_MUT | FN) {
                if !out.contains(&(leaf(key), "call")) {
                    out.push((leaf(key), "call"));
                }
                continue;
            }
            for m in &row.required {
                if !out.contains(&(leaf(key), m)) {
                    out.push((leaf(key), m));
                }
            }
            stack.extend(row.supers.iter().copied());
        }
        out
    }

    /// The `F` rows of a method `name` on an inherent std SHAPE (`Vec`, `[]`, `Option`, `HashSet`, `str`, `Path`) with
    /// `nargs` non-receiver arguments.
    pub(crate) fn inherent(&self, shape: &str, name: &str, nargs: usize) -> Vec<&FnRow> {
        self.fns
            .get(name)
            .map(|v| v.iter().filter(|r| r.owner == shape && r.nargs == nargs).collect())
            .unwrap_or_default()
    }

    /// The `F` rows of trait `path`'s member `name`.
    pub(crate) fn trait_method(&self, path: &str, name: &str) -> Option<&FnRow> {
        let owner = format!("trait:{path}");
        self.fns.get(name).and_then(|v| v.iter().find(|r| r.owner == owner))
    }

    /// The std traits that DECLARE a member `name` (required or provided), open ones only.
    pub(crate) fn traits_declaring(&self, name: &str) -> Vec<&'static str> {
        self.declaring.get(name).cloned().unwrap_or_default()
    }

    /// The `F` row of a free std fn `name` whose module path contains `module` (the written path's last module
    /// segment: `mem` for `std::mem::take`, `thread` for `std::thread::spawn`, `io` for `std::io::copy`).
    pub(crate) fn free_fn(&self, module: &str, name: &str, nargs: usize) -> Option<&FnRow> {
        self.fns.get(name).and_then(|v| {
            v.iter().find(|r| {
                r.nargs == nargs
                    && r.owner.strip_prefix("fn:").is_some_and(|m| m.split("::").any(|s| s == module))
            })
        })
    }

    /// The bounds an impl of trait `path` for std shape `shape` puts on the shape's type arguments; several rows (one
    /// per impl) are unioned. `None` when std declares no impl of that trait for that shape.
    pub(crate) fn impl_bounds(&self, path: &str, shape: &str) -> Option<Vec<&SlotBound>> {
        let rows = self.impls.get(&format!("{path}\t{shape}"))?;
        Some(rows.iter().flatten().collect())
    }

    /// Whether `path`'s member `name` is a PROVIDED member whose return value carries `Self` (a lazy adapter:
    /// `Iterator::map`, `Read::bytes`, `by_ref`).
    pub(crate) fn provided_deferred(&self, path: &str, name: &str) -> bool {
        self.traits.get(path).is_some_and(|r| r.provided_deferred.contains(&name))
    }

    /// Whether `path` declares `name` as a member that RUNS on the receiver when called: a required member, or a
    /// provided one that does not hand `Self` back inside its result.
    pub(crate) fn member_runs(&self, path: &str, name: &str) -> bool {
        self.traits.get(path).is_some_and(|r| r.required.contains(&name) || r.provided.contains(&name))
    }

    /// Whether `path`'s member `name` is REQUIRED (no default body).
    pub(crate) fn is_required(&self, path: &str, name: &str) -> bool {
        self.traits.get(path).is_some_and(|r| r.required.contains(&name))
    }
}

pub(crate) const FN_ONCE: &str = "core::ops::function::FnOnce";
pub(crate) const FN_MUT: &str = "core::ops::function::FnMut";
pub(crate) const FN: &str = "core::ops::function::Fn";
pub(crate) const ITERATOR: &str = "core::iter::traits::iterator::Iterator";
pub(crate) const DOUBLE_ENDED: &str = "core::iter::traits::double_ended::DoubleEndedIterator";

/// One member a std entry point may run on the value that fills `slot`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Hook {
    pub(crate) slot: Slot,
    /// the leaf of the trait that DECLARES the member (`PartialEq` for `eq`, though the bound said `Ord`)
    pub(crate) trait_leaf: &'static str,
    pub(crate) member: &'static str,
    /// the slot's value is moved into the call's result rather than used at the call (`Iterator::map`'s `Self`)
    pub(crate) deferred: bool,
}

/// The receiver of a method call, as the table can be asked about it.
#[derive(Clone, Copy, Debug)]
pub(crate) enum Recv<'a> {
    /// a value of a std SHAPE (`Vec`, `[]`, `[;]`, `Option`, `HashSet`, `BTreeMap`, `str`, `Path`, …): its INHERENT
    /// methods (and, through `Deref`, its target's — `Vec` → `[]`, `String` → `str`, `PathBuf` → `Path`)
    Shape(&'a str),
    /// a value of a type that implements std traits itself (a user type, a generic parameter): the std TRAIT members
    /// a call of this name can be — declared by the trait, or supplied by a blanket impl (`ToString for T: Display`)
    Implementor,
}

fn hooks_of(slots: &[SlotBound], out: &mut Vec<Hook>) {
    let t = table();
    for sb in slots {
        // a bound trait std implements for EVERY type with some other bound (`impl<I: Iterator> IntoIterator for I`)
        // licenses that bound's members too: an `IntoIterator` slot filled by an iterator runs its `next`.
        let mut traits: Vec<&'static str> = sb.traits.clone();
        for tr in &sb.traits {
            if let Some(bl) = t.impl_bounds(tr, "*") {
                for b in bl.iter().filter(|b| b.slot == Slot::SelfRecv) {
                    traits.extend(b.traits.iter().copied());
                }
            }
        }
        for tr in &traits {
            for (tl, m) in t.licensed(tr) {
                let h = Hook { slot: sb.slot, trait_leaf: tl, member: m, deferred: sb.deferred };
                if !out.contains(&h) {
                    out.push(h);
                }
            }
        }
    }
}

/// The SHAPE whose inherent methods a `Deref` receiver also answers with (the table's own `Deref` impls, read for the
/// three std owners whose target is a std shape: `Vec<T>` → `[T]`, `String` → `str`, `PathBuf` → `Path`).
fn deref_shape(shape: &str) -> Option<&'static str> {
    match shape {
        "Vec" => Some("[]"),
        "String" => Some("str"),
        "PathBuf" => Some("Path"),
        "[;]" => Some("[]"),
        _ => None,
    }
}

/// THE ENTRY POINT for a METHOD CALL `recv.name(..nargs..)`: every member the call may run, per slot. `bounds`: for an
/// `Implementor` receiver whose written trait bounds are known (a generic parameter), only std traits among those bounds
/// and their supertraits answer (`fn f<T: Ord>(a: T) { a.max(b) }`); `None` asks every std trait declaring the name.
pub(crate) fn method_hooks(recv: Recv<'_>, name: &str, nargs: usize, bounds: Option<&[String]>) -> Vec<Hook> {
    let t = table();
    let mut out = Vec::new();
    match recv {
        Recv::Shape(shape) => {
            let mut rows = t.inherent(shape, name, nargs);
            if rows.is_empty() {
                if let Some(d) = deref_shape(shape) {
                    rows = t.inherent(d, name, nargs);
                }
            }
            for r in rows {
                hooks_of(&r.slots, &mut out);
            }
            // …and the members of the std TRAITS std implements for the shape (`Clone for Vec<T>`, `Extend for Vec<T>`,
            // `RangeBounds for Range<T>`): the trait member's own slots (its `Self` is the shape, whose impl's bounds
            // the caller reaches through `impl_hooks`).
            let mut shapes = vec![shape];
            if let Some(d) = deref_shape(shape) {
                shapes.push(d);
            }
            for tr in t.traits_declaring(name) {
                if !shapes.iter().any(|sh| t.impl_bounds(tr, sh).is_some()) {
                    continue;
                }
                if let Some(row) = t.trait_method(tr, name).filter(|r| r.nargs == nargs) {
                    hooks_of(&row.slots, &mut out);
                }
            }
        }
        Recv::Implementor => {
            let allowed: Option<BTreeSet<&'static str>> = bounds.map(|bs| {
                let mut s = BTreeSet::new();
                let mut stack: Vec<&'static str> = bs.iter().filter_map(|b| t.resolve_trait(b)).collect();
                while let Some(p) = stack.pop() {
                    if s.insert(p) {
                        if let Some(r) = t.trait_row(p) {
                            stack.extend(r.supers.iter().copied());
                        }
                    }
                }
                s
            });
            for tr in t.traits_declaring(name) {
                // a blanket impl of the declaring trait (`impl<T: Display> ToString for T`) puts ITS bound on the receiver
                let blanket = t.impl_bounds(tr, "*").unwrap_or_default();
                let via_blanket: Vec<&'static str> =
                    blanket.iter().filter(|sb| sb.slot == Slot::SelfRecv).flat_map(|sb| sb.traits.iter().copied()).collect();
                if let Some(a) = &allowed {
                    if !a.contains(tr) && !via_blanket.iter().any(|b| a.contains(b)) {
                        continue;
                    }
                }
                if let Some(row) = t.trait_method(tr, name) {
                    if row.nargs != nargs {
                        continue;
                    }
                    // The row's own `Self` slot names the trait itself. A call of the trait's member `name` on an
                    // implementor runs that implementor's `name` — or, for a PROVIDED member it does not override, the
                    // default body, which drives the trait's OWN required members (`count` → `next`, `write_all` →
                    // `write`, `max` → `cmp`). It does not run the SUPERTRAITS' members (EXECUTED-shape over-charge:
                    // `cause.source()` licensed `Error`'s supertraits `Debug`/`Display` and charged anyhow's `Chain::len`
                    // with `Backtrace::fmt`'s `Env`). So that one trait licenses its own required members (and its blanket
                    // impl's bound, `ToString` → `Display`); every other slot reads the full rule.
                    let mut rest: Vec<SlotBound> = Vec::new();
                    for sb in &row.slots {
                        if sb.slot == Slot::SelfRecv && sb.traits.first() == Some(&tr) {
                            // A REQUIRED member is the implementor's own body and runs no sibling: `w.flush()` is `flush`
                            // only (EXECUTED-shape over-charge: zopfli `GzipEncoder::flush` gained `DeflateEncoder::write`'s
                            // `Log` when a required call licensed every required member).
                            let own = t.own_required(tr);
                            let own: Vec<_> = if own.iter().any(|(_, m)| *m == name) {
                                own.into_iter().filter(|(_, m)| *m == name).collect()
                            } else {
                                own
                            };
                            for (tl, m) in own {
                                let h = Hook { slot: Slot::SelfRecv, trait_leaf: tl, member: m, deferred: sb.deferred };
                                if !out.contains(&h) {
                                    out.push(h);
                                }
                            }
                            let mut others = sb.clone();
                            others.traits.remove(0);
                            if let Some(bl) = t.impl_bounds(tr, "*") {
                                for b in bl.iter().filter(|b| b.slot == Slot::SelfRecv) {
                                    others.traits.extend(b.traits.iter().copied());
                                }
                            }
                            if !others.traits.is_empty() {
                                rest.push(others);
                            }
                        } else {
                            rest.push(sb.clone());
                        }
                    }
                    hooks_of(&rest, &mut out);
                    continue;
                } else {
                    // a member with no bounded slot row is still the trait's own member, run on the receiver
                    for (tl, m) in t.licensed(tr) {
                        let h = Hook { slot: Slot::SelfRecv, trait_leaf: tl, member: m, deferred: t.provided_deferred(tr, name) };
                        if !out.contains(&h) {
                            out.push(h);
                        }
                    }
                }
                for b in via_blanket {
                    for (tl, m) in t.licensed(b) {
                        let h = Hook { slot: Slot::SelfRecv, trait_leaf: tl, member: m, deferred: t.provided_deferred(tr, name) };
                        if !out.contains(&h) {
                            out.push(h);
                        }
                    }
                }
            }
        }
    }
    out
}

/// THE ENTRY POINT for a call of a free std fn (`std::mem::take(x)`, `std::cmp::max(a, b)`, `std::io::copy(r, w)`,
/// `std::thread::spawn(f)`) written with module segment `module`.
pub(crate) fn fn_hooks(module: &str, name: &str, nargs: usize) -> Vec<Hook> {
    let mut out = Vec::new();
    if let Some(r) = table().free_fn(module, name, nargs) {
        hooks_of(&r.slots, &mut out);
    }
    out
}

/// THE ENTRY POINT for a std trait's member run on a value of std SHAPE `shape` (formatting a `Vec<S>`, comparing two
/// `Option<S>`, cloning a tuple): the members its impl's bounds license on each type argument. `None` when std has no
/// impl of `trait_leaf` for the shape (it then runs nothing of the user's).
pub(crate) fn impl_hooks(trait_leaf: &str, shape: &str) -> Option<Vec<Hook>> {
    let t = table();
    let mut out = Vec::new();
    let mut any = false;
    for tr in t.traits_by_leaf(trait_leaf) {
        if let Some(sbs) = t.impl_bounds(tr, shape) {
            any = true;
            let owned: Vec<SlotBound> = sbs.into_iter().cloned().collect();
            hooks_of(&owned, &mut out);
        }
    }
    any.then_some(out)
}

/// The required members a bound on the std trait written `written` licenses, as (declaring trait leaf, member).
pub(crate) fn licensed_by(written: &str) -> Vec<(&'static str, &'static str)> {
    let t = table();
    t.resolve_trait(written).map(|p| t.licensed(p)).unwrap_or_default()
}

/// Whether `name` is an `Iterator` / `DoubleEndedIterator` member that RUNS the receiver (`count`, `is_sorted`, `next`,
/// `nth_back`) — every member but the lazy adapters, which hand `Self` back inside their result (`map`, `rev`).
pub(crate) fn iterator_member_runs(name: &str) -> bool {
    let t = table();
    t.member_runs(ITERATOR, name) || t.member_runs(DOUBLE_ENDED, name)
}

/// Whether `name` is a lazy `Iterator` / `DoubleEndedIterator` adapter (its result carries `Self`: `map`, `rev`, `by_ref`).
pub(crate) fn iterator_member_lazy(name: &str) -> bool {
    let t = table();
    t.provided_deferred(ITERATOR, name) || t.provided_deferred(DOUBLE_ENDED, name)
}

/// Whether a call of `name` hands its receiver back as (part of) an ITERATOR the chain model follows by its receiver:
/// a lazy `Iterator` / `DoubleEndedIterator` adapter (`map`, `rev`) or `IntoIterator::into_iter`. A deferred member of any
/// other trait (`BufRead::lines`, `Read::bytes`) wraps the receiver in a value whose own drive runs a DIFFERENT member, so
/// it is licensed at the call.
pub(crate) fn iterator_chain_member(name: &str) -> bool {
    iterator_member_lazy(name) || name == "into_iter"
}

/// Whether `name` is a member of `DoubleEndedIterator` (so a chain naming it drives `next_back`).
pub(crate) fn double_ended_member(name: &str) -> bool {
    let t = table();
    t.trait_row(DOUBLE_ENDED).is_some_and(|r| r.required.contains(&name) || r.provided.contains(&name))
        || name == "rev"
}

/// Whether std trait `trait_leaf` (any std trait of that leaf) REQUIRES `member` — no default body, so `Trait::m(&t)`
/// over an implementor can only run that implementor's own `m`.
pub(crate) fn std_required_member(trait_leaf: &str, member: &str) -> bool {
    let t = table();
    t.traits_by_leaf(trait_leaf).iter().any(|p| t.is_required(p, member))
}

/// Whether `leaf` is a std SHAPE the table has rows for (an implementing type of an `I` row, or the owner of an
/// inherent `F` row): `Vec`, `[]`, `Option`, `HashSet`, `BTreeMap`, `String`, `Path`, …
pub(crate) fn is_shape(leaf: &str) -> bool {
    let t = table();
    t.shapes.contains(leaf)
}

/// For a std CONSTRUCTOR-like fn that MOVES an argument into the value it returns (`BufWriter::new(inner: W)`,
/// `BufReader::with_capacity(cap, inner: R)`): (type-argument index, argument index) pairs, read off the row's deferred
/// `eN` and `aN` slots that carry the same bounds. The argument is not used at the call (nothing of it runs there); it
/// becomes the returned value's type argument, which a later call on that value reaches through the shape's impl.
pub(crate) fn moved_args(shape: &str, name: &str, nargs: usize) -> Vec<(usize, usize)> {
    let mut out = Vec::new();
    for r in table().inherent(shape, name, nargs) {
        let elems: Vec<(usize, &Vec<&'static str>)> = r
            .slots
            .iter()
            .filter_map(|s| match s.slot {
                Slot::Elem(i) if s.deferred => Some((i, &s.traits)),
                _ => None,
            })
            .collect();
        for s in &r.slots {
            if let (Slot::Arg(k), true) = (s.slot, s.deferred) {
                let m: Vec<usize> = elems.iter().filter(|(_, t)| **t == s.traits).map(|(i, _)| *i).collect();
                if m.len() == 1 && !out.contains(&(m[0], k)) {
                    out.push((m[0], k));
                }
            }
        }
    }
    out
}

/// Whether `leaf` names a std trait (`Clone`, `Iterator`, `Write`).
pub(crate) fn is_std_trait_leaf(leaf: &str) -> bool {
    !table().traits_by_leaf(leaf).is_empty()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn has(v: &[Hook], slot: Slot, tl: &str, m: &str) -> bool {
        v.iter().any(|h| h.slot == slot && h.trait_leaf == tl && h.member == m)
    }

    /// The table answers the hand lists' own cases from std's declarations (each row read off rust-src).
    #[test]
    fn std_bounds_answers_the_hand_lists_cases() {
        let c = method_hooks(Recv::Shape("Vec"), "contains", 1, None);
        assert!(has(&c, Slot::Elem(0), "PartialEq", "eq"), "{c:?}");
        let s = method_hooks(Recv::Shape("[]"), "sort", 0, None);
        assert!(has(&s, Slot::Elem(0), "Ord", "cmp") && has(&s, Slot::Elem(0), "PartialOrd", "partial_cmp"), "{s:?}");
        let s = method_hooks(Recv::Shape("Vec"), "sort", 0, None);
        assert!(has(&s, Slot::Elem(0), "Ord", "cmp"), "Vec derefs to [T]: {s:?}");
        let h = method_hooks(Recv::Shape("HashSet"), "insert", 1, None);
        assert!(has(&h, Slot::Elem(0), "Hash", "hash") && has(&h, Slot::Elem(0), "PartialEq", "eq"), "{h:?}");
        let t = method_hooks(Recv::Implementor, "to_string", 0, None);
        assert!(has(&t, Slot::SelfRecv, "Display", "fmt"), "{t:?}");
        let o = method_hooks(Recv::Implementor, "to_owned", 0, None);
        assert!(has(&o, Slot::SelfRecv, "Clone", "clone"), "{o:?}");
        let n = method_hooks(Recv::Implementor, "count", 0, None);
        assert!(has(&n, Slot::SelfRecv, "Iterator", "next"), "{n:?}");
        let m = method_hooks(Recv::Implementor, "max", 1, None);
        assert!(has(&m, Slot::SelfRecv, "Ord", "cmp"), "{m:?}");
        let w = method_hooks(Recv::Implementor, "write_all", 1, None);
        assert!(has(&w, Slot::SelfRecv, "Write", "write"), "{w:?}");
        let r = method_hooks(Recv::Implementor, "read_to_end", 1, None);
        assert!(has(&r, Slot::SelfRecv, "Read", "read"), "{r:?}");
        let srt = method_hooks(Recv::Implementor, "is_sorted", 0, None);
        assert!(has(&srt, Slot::SelfRecv, "Iterator", "next") && has(&srt, Slot::Item, "PartialOrd", "partial_cmp"), "{srt:?}");
        let mp = method_hooks(Recv::Implementor, "map", 1, None);
        assert!(mp.iter().any(|h| h.member == "next" && h.deferred), "a lazy adapter is deferred: {mp:?}");
        let tk = fn_hooks("mem", "take", 1);
        assert!(has(&tk, Slot::Arg(0), "Default", "default"), "{tk:?}");
        let sp = fn_hooks("thread", "spawn", 1);
        assert!(has(&sp, Slot::Arg(0), "FnOnce", "call"), "{sp:?}");
        let cp = fn_hooks("io", "copy", 2);
        assert!(has(&cp, Slot::Arg(0), "Read", "read") && has(&cp, Slot::Arg(1), "Write", "write"), "{cp:?}");
        let mx = fn_hooks("cmp", "max", 2);
        assert!(has(&mx, Slot::Arg(0), "Ord", "cmp"), "{mx:?}");
        let pn = method_hooks(Recv::Shape("Path"), "new", 1, None);
        assert!(has(&pn, Slot::Arg(0), "AsRef", "as_ref"), "{pn:?}");
        let ps = method_hooks(Recv::Shape("str"), "parse", 0, None);
        assert!(has(&ps, Slot::Ret, "FromStr", "from_str"), "{ps:?}");
        let dv = impl_hooks("Debug", "Vec").unwrap();
        assert!(has(&dv, Slot::Elem(0), "Debug", "fmt"), "{dv:?}");
        let dt = impl_hooks("Debug", "()").unwrap();
        assert!(has(&dt, Slot::ElemAll, "Debug", "fmt"), "{dt:?}");
        let co = impl_hooks("Clone", "Option").unwrap();
        assert!(has(&co, Slot::Elem(0), "Clone", "clone"), "{co:?}");
        assert!(iterator_member_runs("is_sorted") && iterator_member_runs("count") && !iterator_member_runs("map"));
        assert!(iterator_member_lazy("map") && iterator_member_lazy("rev") && !iterator_member_lazy("count"));
        assert!(std_required_member("Iterator", "next") && std_required_member("Write", "flush") && !std_required_member("Ord", "max"));
        // bounded: `T: Ord` answers `max` through Ord, not Iterator; an unrelated bound answers nothing
        let b = method_hooks(Recv::Implementor, "max", 1, Some(&["Ord".to_string()]));
        assert!(has(&b, Slot::SelfRecv, "Ord", "cmp"), "{b:?}");
        let b = method_hooks(Recv::Implementor, "max", 1, Some(&["Flush2".to_string()]));
        assert!(b.is_empty(), "{b:?}");
    }
}
