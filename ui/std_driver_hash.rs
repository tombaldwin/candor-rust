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

// The sibling drivers. An earlier version of this file said they were "unchanged by the args fix"; that
// was FALSE, and it is the sentence that stopped the regression being measured. `PartialEq<Rhs = Self>`
// declares a TRAIT parameter beyond `Self`, the shared args builder padded it with `()`, and
// `<E as PartialEq<()>>::eq` resolves to nothing — so every driver edge through `eq` silently vanished
// and ten soundness-fuzzer seeds went `pure/omitted` over a real effect. It could not be caught HERE
// because every `PartialEq` in this file was DERIVED, i.e. pure either way. `EqEffectful` below is that
// missing arm: a hand-written, effectful `PartialEq` reached through both std drivers that run `eq`.
//
// `Ord::cmp` really does have no generics of its own and no trait parameter beyond `Self`, so it was
// unaffected — which is the point: one member of a "sibling" set surviving says nothing about the rest.
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

// A hand-written, EFFECTFUL `PartialEq`. `PartialEq<Rhs = Self>` is the only trait in the driver set
// with a trait parameter the call site does not name, so it is the only one whose args must come from
// the parameter's DECLARED DEFAULT rather than from a placeholder.
struct EqEffectful(u32);
impl PartialEq for EqEffectful {
    fn eq(&self, other: &Self) -> bool {
        let _ = std::process::Command::new("id").status(); // Exec
        self.0 == other.0
    }
}
impl Eq for EqEffectful {}
impl Hash for EqEffectful {
    // Derived-equivalent and pure ON PURPOSE: `HashSet::insert` drives BOTH `hash` and `eq`, so a pure
    // `hash` is what makes the `eq` edge the only thing `set_insert_effectful_eq` can be charged for.
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.0.hash(state);
    }
}

// OVER-CHARGE CONTROL for the `eq` edge: a derived (pure) `PartialEq` under the same two drivers.
#[derive(PartialEq, Eq, Hash)]
struct EqDerived(u32);

fn set_insert_effectful() {
    let mut s = HashSet::new();
    s.insert(Effectful); // Fs, through `<Effectful as Hash>::hash`
}
fn vec_contains_effectful_eq() {
    let v = vec![EqEffectful(0)];
    let _ = v.contains(&EqEffectful(1)); // Exec, through `<EqEffectful as PartialEq>::eq`
}
fn set_insert_effectful_eq() {
    let mut s = HashSet::new();
    s.insert(EqEffectful(0));
    s.insert(EqEffectful(1)); // Exec, through `<EqEffectful as PartialEq>::eq`
}
fn vec_contains_derived_stays_pure() {
    let v = vec![EqDerived(0)];
    let _ = v.contains(&EqDerived(1)); // PURE
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
    vec_contains_effectful_eq();
    set_insert_effectful_eq();
    vec_contains_derived_stays_pure();
}
