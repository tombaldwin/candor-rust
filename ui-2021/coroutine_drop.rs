// Sibling of tests/integration.sh's 9c-iii (a CLOSURE capturing an effectful-Drop value by move,
// dropped WITHOUT ever being called): the mir_spike.rs `local_drop_impls` walker has a doc comment
// claiming "CLOSURE/COROUTINE" share the same arm-per-TyKind treatment, but the shipped regression
// coverage (tests/integration.sh 9c-iii) only ever drove the Closure arm — the Coroutine and
// CoroutineClosure arms (an `async {}` block / an `async || {}` closure, both their own `TyKind`,
// never a `TyKind::Adt`) had zero fixture coverage. Guard-deletion proved each is independently load-
// bearing: deleting either arm alone (leaving the other + Closure intact) makes exactly its own
// function vanish from the report — a distinct silent under-report from the closure case, not a
// re-test of it.
#![allow(unused)]

struct Guard;
impl Drop for Guard {
    fn drop(&mut self) {
        let _ = std::net::TcpStream::connect("10.0.0.2:9"); // Net
    }
}

// A `Coroutine` (an `async {}` block's Future) that captured Guard BY MOVE, dropped without ever
// being polled/awaited.
fn coroutine_scope_exit() {
    let g = Guard;
    let _fut = async move {
        let _ = &g;
    };
}

// A `CoroutineClosure` (an `async || {}` closure) that captured Guard BY MOVE, dropped without ever
// being called (calling it is what would produce the `Coroutine` above).
fn coroutine_closure_scope_exit() {
    let g = Guard;
    let _c = async move || {
        let _ = &g;
    };
}

fn main() {
    coroutine_scope_exit();
    coroutine_closure_scope_exit();
}
