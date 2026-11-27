//! SOUNDNESS R1757 (round 2) — std's INHERENT associated items per type leaf, and the fn names std's public traits
//! declare, read from rust-src (`gen/std_assoc.py`).
//!
//! A path call `Ty::m(..)` on a std type runs Ty's inherent `m` whenever one exists — Rust resolves an inherent
//! associated item before any trait member — so `Duration::new(1, 0)` is std's even beside a dependency's
//! `impl Load for Duration { fn new() }`, while `Instant::new()` / `u64::new()` (no inherent) run the dependency's.
//! That is a fact per TYPE, which is why it is a generated table and not a list of names: a name list either joins
//! where std's inherent wins (an over-charge) or withholds a disclosure where no inherent exists (a silence).
//! Over-inclusion (a name std does not really have) only withholds a join or a hedge, i.e. the published 0.42.0
//! behaviour, never a charge.

static TABLE: &str = include_str!("std_assoc.tsv");

struct Tbl {
    inherent: std::collections::HashSet<(&'static str, &'static str)>,
    trait_fns: std::collections::HashSet<&'static str>,
    /// SOUNDNESS R1913 — fn name -> the SEALED std traits (`pub impl(..) trait`) that declare it
    sealed: std::collections::HashMap<&'static str, Vec<(&'static str, Vec<&'static str>)>>,
    /// SOUNDNESS R1951 — trait leaf -> the first edition whose prelude imports it (`P` rows)
    prelude: std::collections::HashMap<&'static str, u16>,
}

fn tbl() -> &'static Tbl {
    static S: std::sync::OnceLock<Tbl> = std::sync::OnceLock::new();
    S.get_or_init(|| {
        let mut t = Tbl {
            inherent: Default::default(),
            trait_fns: Default::default(),
            sealed: Default::default(),
            prelude: Default::default(),
        };
        for l in TABLE.lines().filter(|l| !l.starts_with('#')) {
            let f: Vec<&str> = l.split('\t').collect();
            match f.as_slice() {
                ["I", ty, n] => {
                    t.inherent.insert((ty, n));
                }
                ["T", n] => {
                    t.trait_fns.insert(n);
                }
                ["TS", n, tr, impls] => {
                    t.sealed.entry(n).or_default().push((tr, impls.split(',').filter(|i| *i != "-").collect()));
                }
                ["P", ed, leaf] => {
                    if let Ok(e) = ed.parse::<u16>() {
                        let v = t.prelude.entry(leaf).or_insert(e);
                        *v = (*v).min(e);
                    }
                }
                _ => {}
            }
        }
        t
    })
}

/// Does the std type with this LEAF have a stable inherent `pub fn name` (any receiver)?
pub(crate) fn has_inherent(ty_leaf: &str, name: &str) -> bool {
    tbl().inherent.contains(&(ty_leaf, name))
}

/// Is this a std type leaf the table knows (it has at least one inherent fn)?
pub(crate) fn is_std_type(ty_leaf: &str) -> bool {
    static K: std::sync::OnceLock<std::collections::HashSet<&'static str>> = std::sync::OnceLock::new();
    K.get_or_init(|| tbl().inherent.iter().map(|(t, _)| *t).collect()).contains(ty_leaf)
}

/// Does any public std trait declare a fn of this name (`default`, `from`, `from_str`, `fmt`, …)? Such a member on a
/// std type may be std's own trait impl, so an UNNAMED-trait path call is neither joined nor hedged on it.
pub(crate) fn is_std_trait_fn(name: &str) -> bool {
    tbl().trait_fns.contains(name)
}

/// SOUNDNESS R1913 — the SEALED std traits (`pub impl(self) trait CommandExt`, `OsStrExt`, `IsTerminal`, ..) that declare a fn of this
/// name, each with the NOMINAL std types it is implemented for (empty when the generator found none). Not part of `is_std_trait_fn`: a
/// sealed trait's member is std's only where the trait is IMPORTED and the call's TYPE implements it, so a name-only withholding would hide a
/// dependency trait's member of the same name wherever only the dependency's trait is in scope or the type is another one.
pub(crate) fn sealed_std_traits(name: &str) -> &[(&'static str, Vec<&'static str>)] {
    tbl().sealed.get(name).map(Vec::as_slice).unwrap_or(&[])
}

/// SOUNDNESS R1951 — the first edition (2015, 2021, 2024) whose prelude imports a std trait of this leaf, or `None` (in no prelude). Read
/// from rust-src's `core::prelude::rust_20XX` and the `v1` preludes; a name the preludes import that is not a trait leaf is not listed.
pub(crate) fn prelude_since(trait_leaf: &str) -> Option<u16> {
    tbl().prelude.get(trait_leaf).copied()
}

#[cfg(test)]
mod tests {
    // SOUNDNESS R1951 / R1956 (lane L3) — the generated scope facts the std-type path rule reads. Each is a fact of rust-src a wrong
    // regeneration would move; the generator's own self-check (`gen/std_assoc.py`, `STD_ASSOC_SELFCHECK_DROP_ROW`) is the drop-must-fail half.
    #[test]
    fn prelude_editions_are_read_from_rust_src() {
        assert_eq!(super::prelude_since("Default"), Some(2015));
        assert_eq!(super::prelude_since("Iterator"), Some(2015));
        assert_eq!(super::prelude_since("TryFrom"), Some(2021));
        assert_eq!(super::prelude_since("FromIterator"), Some(2021));
        assert_eq!(super::prelude_since("Future"), Some(2024));
        assert_eq!(super::prelude_since("FromStr"), None);
        assert_eq!(super::prelude_since("SliceIndex"), None);
    }

    #[test]
    fn std_bounds_scope_facts() {
        let tb = crate::std_bounds::table();
        assert!(tb.impl_unconditional("core::default::Default", "String"));
        assert!(!tb.impl_unconditional("core::default::Default", "Instant"));
        assert!(tb.impl_unconditional("core::convert::TryFrom", "*"));
        // `impl From<Arc<W>> for RawWaker` is `#[cfg(target_has_atomic = "ptr")]` while RawWaker is not: gated, so never a withholding
        assert!(tb.impl_bounds("core::convert::From", "RawWaker").is_some() && !tb.impl_unconditional("core::convert::From", "RawWaker"));
        // co-gated with its type (alloc::sync is target_has_atomic-gated, and so is Arc): not conditional
        assert!(tb.impl_unconditional("core::default::Default", "Arc"));
        assert!(tb.is_impl_shape("Instant") && !tb.is_impl_shape("NonZeroU8"));
        assert!(tb.uncovered_member("from_raw_fd") && !tb.uncovered_member("from_str"));
        assert!(tb.traits_declaring_any("from_str").contains(&"core::str::traits::FromStr"));
        assert!(!tb.traits_declaring_any("get").contains(&"core::slice::index::SliceIndex"));
    }
}
