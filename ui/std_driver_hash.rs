// The set-`insert` → `<E as Hash>::hash` driver edge, which had never once run.
//
// `std_driver_local_edges` recovers the trait method a non-local std driver invokes on its element:
// `set.insert(e)` runs `<E as Hash>::hash`, so a LOCAL effectful `Hash` impl reached only that way was
// otherwise silently pure. The recovery built its generic args as `mk_args(&[E])` — one argument, for a
// method that declares two (`Hash::hash<H: Hasher>` carries its own `H`). `Instance::try_resolve` does
// not return `None` for a short args list; it raises a `span_delayed_bug`, and rustc turns that into an
// INTERNAL COMPILER ERROR at the end of the build. So `HashSet::insert` of ANY local type failed to
// compile under candor, in six lines, and the edge this file tests never landed.
//
// Two things this pins, then. That the ICE is gone — the fixture cannot compile otherwise — and that the
// edge it was hiding actually fires: `set_insert_effectful` must carry `Fs`.
//
// Note the comment above `local_trait_method_for_self`, which says a local-ADT gate "sidesteps a rustc
// ICE ... with 'missing value for assoc item in impl'". It is about a sibling of this exact bug, it
// reads as though the class were handled, and it is why nobody measured this one: a LOCAL element type
// sails straight through a gate that only rejects types mentioning no local ADT.
#![allow(unused)]
use std::collections::{BTreeSet, HashSet};
use std::hash::{Hash, Hasher};

#[derive(PartialEq, Eq)]
struct Effectful;
impl Hash for Effectful {
    fn hash<H: Hasher>(&self, _state: &mut H) {
        let _ = std::fs::read_to_string("/db"); // Fs
    }
}

// OVER-CHARGE CONTROL: a derived (pure) `Hash` reached by exactly the same driver stays pure.
#[derive(PartialEq, Eq, Hash)]
struct Derived;

// The sibling drivers, unchanged by the args fix — they resolve methods with no generics of their own,
// so they were never short an argument. Here so a regression in the shared args builder shows up on
// more than the one method that motivated it.
#[derive(PartialEq, Eq)]
struct Ordered;
impl PartialOrd for Ordered {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(Ord::cmp(self, other))
    }
}
impl Ord for Ordered {
    fn cmp(&self, _other: &Self) -> std::cmp::Ordering {
        let _ = std::env::var("PATH"); // Env
        std::cmp::Ordering::Equal
    }
}

fn set_insert_effectful() {
    let mut s = HashSet::new();
    s.insert(Effectful); // Fs, through `<Effectful as Hash>::hash`
}
fn set_insert_derived_stays_pure() {
    let mut s = HashSet::new();
    s.insert(Derived); // PURE
}
fn btreeset_insert_effectful() {
    let mut s = BTreeSet::new();
    s.insert(Ordered); // Env, through `<Ordered as Ord>::cmp`
}

fn main() {
    set_insert_effectful();
    set_insert_derived_stays_pure();
    btreeset_insert_effectful();
}
