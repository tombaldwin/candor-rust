// The drop walker's recursion guard, and the ownership marker the curated container list was missing.
//
// `local_drop_impls`'s `seen` set used to be keyed on the ADT's `DefId`. That made the walk ORDER-
// DEPENDENT and silently lossy: the first instantiation of a generic ADT claimed the key, so every later
// one — a different type argument, a different drop set — returned immediately. Fields are walked in
// DECLARATION ORDER, so `struct S { a: Cellish<u8>, b: Cellish<Guard> }` lost `Guard::drop` while the
// same struct with the two fields SWAPPED was caught. The three `std` shapes below are why that mattered
// in practice rather than in principle: `Mutex<Guard>`, `RwLock<Guard>` and `RefCell<Guard>` each reach
// their payload through `UnsafeCell<T>`, and each walks a DIFFERENT `UnsafeCell` (a poison flag, a borrow
// counter, a platform lock) before it — so all three read silent-pure over an effectful guard.
//
// The second half is `PhantomData`. It is not another curated container name; it is the LANGUAGE'S OWN
// declaration of the property the list was hand-enumerating — `PhantomData<T>` among a struct's fields is
// exactly how a raw-pointer container tells drop-check "dropping me can drop a `T`". Following its
// argument recovers `std::vec::IntoIter<Guard>` (a `for` loop left early still drops the rest) and every
// hand-written arena that marks ownership the documented way, without a new name per victim.
//
// The third half is the RECURSION BOUND. It is a DEPTH, not a set of ancestor `DefId`s, because a
// nested same-ADT type (`Box<Box<Guard>>`, `Option<Option<Guard>>`, `Cellish<Cellish<Guard>>`) is its
// own "ancestor" while being an ordinary finite type — an ancestor set keyed on the `DefId` cut all
// three and reintroduced, one construct over, exactly the class the `seen` key change closes.
//
// Guard-deletion, all three halves, each independently load-bearing: restore the `DefId` key and the
// three `decl_order_*` functions plus all three `via_{mutex,rwlock,refcell}` go silent; drop
// `PhantomData` from `is_std_owning_container` and `via_vec_into_iter` + `via_phantom_arena` go silent;
// swap the depth bound for an ancestor `DefId` set and `nested_box` / `nested_option` /
// `nested_user_generic` go silent — while the rest of the file stays green in each case.
#![allow(unused)]
use std::marker::PhantomData;

#[derive(PartialEq, Eq, PartialOrd, Ord, Hash)]
struct Guard;
impl Drop for Guard {
    fn drop(&mut self) {
        let _ = std::net::TcpStream::connect("10.0.0.2:9"); // Net
    }
}

// ---- the same generic ADT at two instantiations, the effectful one declared SECOND ----
struct Cellish<T>(T);
struct DeclOrder {
    first: Cellish<u8>,     // walked first — used to claim the shared recursion key
    second: Cellish<Guard>, // the payload, skipped as "already seen"
}
fn decl_order_field() {
    let x = DeclOrder { first: Cellish(0), second: Cellish(Guard) };
    let _ = x; // Net
}
fn decl_order_tuple() {
    let x = (Cellish(0u8), Cellish(Guard));
    let _ = x; // Net
}
fn decl_order_explicit_drop() {
    // The explicit-`drop()` route shares the same walker, so it inherited the identical hole.
    drop(DeclOrder { first: Cellish(0), second: Cellish(Guard) }); // Net
}

// ---- the std shapes that hole silenced ----
fn via_mutex() {
    let x = std::sync::Mutex::new(Guard);
    let _ = x; // Net
}
fn via_rwlock() {
    let x = std::sync::RwLock::new(Guard);
    let _ = x; // Net
}
fn via_refcell() {
    let x = std::cell::RefCell::new(Guard);
    let _ = x; // Net
}

// ---- the rest of the curated owning-container list ----
// A.2: `is_std_owning_container` names eleven types, and every fixture that ever drove it used `Vec` or
// `Box` — the two the original bug report happened to involve. Deleting the other nine left the suite
// green, so nine of eleven entries were present rather than tested. One row each; the list is now the
// thing the fixture covers, not the pair that prompted it.
fn via_box() {
    let x = Box::new(Guard);
    let _ = x; // Net
}
fn via_vec() {
    let x = vec![Guard];
    let _ = x; // Net
}
fn via_vecdeque() {
    let mut x = std::collections::VecDeque::new();
    x.push_back(Guard);
    let _ = x; // Net
}
fn via_rc() {
    let x = std::rc::Rc::new(Guard);
    let _ = x; // Net
}
fn via_arc() {
    let x = std::sync::Arc::new(Guard);
    let _ = x; // Net
}
fn via_btreemap() {
    let mut x = std::collections::BTreeMap::new();
    x.insert(1u8, Guard);
    let _ = x; // Net
}
fn via_btreeset() {
    let mut x = std::collections::BTreeSet::new();
    x.insert(Ordered(Guard));
    let _ = x; // Net
}
fn via_hashmap() {
    let mut x = std::collections::HashMap::new();
    x.insert(1u8, Guard);
    let _ = x; // Net
}
fn via_hashset() {
    let mut x = std::collections::HashSet::new();
    x.insert(Ordered(Guard));
    let _ = x; // Net
}
fn via_linked_list() {
    let mut x = std::collections::LinkedList::new();
    x.push_back(Guard);
    let _ = x; // Net
}
fn via_binary_heap() {
    let mut x = std::collections::BinaryHeap::new();
    x.push(Ordered(Guard));
    let _ = x; // Net
}
// The set/heap containers need their element ordered and hashed; the wrapper carries `Guard` so the
// walk still has to cross the container's heap indirection to find its destructor.
#[derive(PartialEq, Eq, PartialOrd, Ord, Hash)]
struct Ordered(Guard);

// ---- the SAME ADT nested inside ITSELF: not a cycle, just a type ----
// Any guard keyed on the ADT's `DefId` — the old whole-walk `seen`, or an ancestor set — treats the
// INNER `Box`/`Option`/`Cellish` as already-visited and cuts the walk there, even though the type is
// perfectly finite. MEASURED: three of these four went silent-pure under an ancestor set keyed on the
// `DefId`, which is why the walker's recursion bound is a DEPTH and not an ancestor set. `nested_vec`
// is here as the one that stayed green under both — a fixture drawn only from the failures would have
// let the next `DefId`-keyed guard straight back in.
fn nested_box() {
    let x: Box<Box<Guard>> = Box::new(Box::new(Guard));
    let _ = x; // Net
}
fn nested_option() {
    let x: Option<Option<Guard>> = Some(Some(Guard));
    let _ = x; // Net
}
fn nested_user_generic() {
    let x = Cellish(Cellish(Guard));
    let _ = x; // Net
}
fn nested_vec() {
    let x: Vec<Vec<Guard>> = vec![vec![Guard]];
    let _ = x; // Net
}

// ---- PhantomData<T> as the ownership marker ----
fn via_vec_into_iter() {
    let it = vec![Guard].into_iter();
    let _ = it; // Net — dropping a partly-consumed IntoIter drops the remaining elements
}
struct Arena<T> {
    p: *mut T,
    own: PhantomData<T>, // the documented "I own a T for drop purposes" marker
}
impl<T> Drop for Arena<T> {
    fn drop(&mut self) {
        unsafe { std::ptr::drop_in_place(self.p) }
    }
}
fn via_phantom_arena() {
    let a = Arena { p: Box::into_raw(Box::new(Guard)), own: PhantomData };
    let _ = a; // Net
}

// ---- OVER-CHARGE CONTROLS: each has a real (pure) Drop, so the walker RUNS and must find nothing ----
// The variance-only spellings of PhantomData do not own their parameter, and must not pull Guard in.
struct VarianceRef<'a> {
    p: *const u8,
    v: PhantomData<&'a Guard>,
}
impl<'a> Drop for VarianceRef<'a> {
    fn drop(&mut self) {}
}
fn variance_ref_marker_stays_pure() {
    let x = VarianceRef { p: 0 as *const u8, v: PhantomData };
    let _ = x; // PURE
}
struct VarianceFn {
    p: *const u8,
    v: PhantomData<fn() -> Guard>,
}
impl Drop for VarianceFn {
    fn drop(&mut self) {}
}
fn variance_fn_marker_stays_pure() {
    let x = VarianceFn { p: 0 as *const u8, v: PhantomData };
    let _ = x; // PURE
}
// `ManuallyDrop<Guard>` never runs Guard's destructor at all — charging it would be a fabrication.
fn manually_drop_stays_pure() {
    let x = std::mem::ManuallyDrop::new(Guard);
    let _ = x; // PURE
}
// The same container over a payload with no destructor: the walker runs and finds nothing.
fn mutex_over_plain_payload_stays_pure() {
    let x = std::sync::Mutex::new(0u8);
    let _ = x; // PURE
}

fn main() {
    decl_order_field();
    decl_order_tuple();
    decl_order_explicit_drop();
    via_mutex();
    via_rwlock();
    via_refcell();
    nested_box();
    nested_option();
    nested_user_generic();
    nested_vec();
    via_vec_into_iter();
    via_phantom_arena();
    variance_ref_marker_stays_pure();
    variance_fn_marker_stays_pure();
    manually_drop_stays_pure();
    mutex_over_plain_payload_stays_pure();
    via_box();
    via_vec();
    via_vecdeque();
    via_rc();
    via_arc();
    via_btreemap();
    via_btreeset();
    via_hashmap();
    via_hashset();
    via_linked_list();
    via_binary_heap();
}
