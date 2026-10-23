//! SOUNDNESS R1136 — the (method name, argument count) pairs std itself can answer a method call with.
//!
//! A method call on a std-typed receiver (`d.extend()` on a `u8`, `it.sorted()` on an iterator) whose name
//! and arity no STABLE public std/core/alloc method has, and which the crate does not define, can only be
//! a dependency's EXTENSION-TRAIT method brought into scope by an import (`use num_conv::prelude::*`,
//! `use itertools::Itertools`). Its body is in that dependency, so the call must disclose the dependency in
//! `invisible` exactly as the qualified spelling (`num_conv::Extend::extend(d)`) already does.
//!
//! The table is GENERATED (`gen/std_methods.py` over the toolchain's rust-src) rather than written, because
//! it is a DENYLIST of what needs no disclosure: a std method missing from it only adds an advisory
//! `invisible` entry, while a hand-written list would silently stop being complete the day std grows.

static TABLE: &str = include_str!("std_methods.tsv");

fn set() -> &'static std::collections::HashSet<(&'static str, usize)> {
    static S: std::sync::OnceLock<std::collections::HashSet<(&'static str, usize)>> = std::sync::OnceLock::new();
    S.get_or_init(|| {
        TABLE
            .lines()
            .filter(|l| !l.starts_with('#'))
            .filter_map(|l| {
                let (n, a) = l.split_once('\t')?;
                Some((n, a.trim().parse().ok()?))
            })
            .collect()
    })
}

/// Whether std has a stable public method `name` taking `argc` arguments besides `self`.
pub(crate) fn is_std_method(name: &str, argc: usize) -> bool {
    set().contains(&(name, argc))
}

/// SOUNDNESS F1 (Fable REVIEW rust0416) — whether std has a stable public method `name` at ANY arity: the drift test for a member
/// a dependency trait provably does not declare (`Result::map_err`, `Option::unwrap_or`, `Iterator::filter_map` reached through a
/// chain or a spread bound). A member std does NOT have cannot be std drift.
pub(crate) fn is_std_method_name(name: &str) -> bool {
    set().iter().any(|(n, _)| *n == name)
}
