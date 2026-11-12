//! SOUNDNESS R1657 stage 1b follow-ups (Fable review 2026-11-11, F1 and F4). Both are stage-1b regressions against the
//! pre-1b engine that the qual-level release bar could not see, because each row kept an `Unknown`.
//!
//! F1 — futures-lite 1.11–2.6 `stream::Chain::poll_next` (`pin_project! { struct Chain<S, U> { #[pin] first: Fuse<S>, .. } }`
//! beside the same-field twin `io::Chain<R1, R2> { #[pin] first: R1, .. }`; body `this.first.as_mut().poll_next(cx)`): R476
//! moved the leaf-keyed `Chain.first` entry to `trait_fields` (the twin's field is a type PARAMETER), and 1b's proven-owner
//! read (`qual_field_in`) then required the leaf entry and answered nothing, so `this.first` typed to nothing, the key
//! `futures_core#stream::Stream::poll_next` and `invisible:[futures_core]` vanished and only a hedge was left. The local-trait
//! spelling (`Tr::m`) went ABSENT outright. EXECUTED (lane scratch `rustagent-r1682/f1`, `cargo run --example run`: both the
//! `Tr::m` and the `Stream::poll_next` implementor wrote their files through `Chain`).
//!
//! F4 — tokio 1.53.2–1.54.1 `sync::broadcast::Receiver::drop` declares `struct NotifyGuard { rx: &mut Receiver<T>, .. }` and
//! `impl Drop for NotifyGuard { .. self.rx.shared.notify_last_rx_drop.notify_waiters() }` in its body; the nested impl's
//! `self` still held the ENCLOSING impl's type, so `self.rx` was read on `Receiver` (refused once 1b proved the owner) and the
//! `Notify::notify_waiters` edge (Log, in tokio) was lost behind a hedge. EXECUTED (`rustagent-r1682/f4`: the notify write
//! happened when the receiver dropped).
use std::path::{Path, PathBuf};
use std::process::Command;

fn bin() -> &'static str {
    env!("CARGO_BIN_EXE_candor-scan")
}

fn fixture(tag: &str, files: &[(&str, &str)]) -> PathBuf {
    let d = std::env::temp_dir().join(format!("candor-r1657f-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(d.join("src")).unwrap();
    std::fs::write(d.join("Cargo.toml"), "[package]\nname = \"fx\"\nversion = \"0.1.0\"\nedition = \"2021\"\n[dependencies]\nfutures-core = \"0.3\"\npin-project-lite = \"0.2\"\n").unwrap();
    for (p, s) in files {
        let f = d.join("src").join(p);
        std::fs::create_dir_all(f.parent().unwrap()).unwrap();
        std::fs::write(f, s).unwrap();
    }
    d
}

fn scan(dir: &Path, policy: Option<&str>, off: &[&str]) -> (i32, serde_json::Value) {
    let mut cmd = Command::new(bin());
    cmd.arg(dir.to_string_lossy().as_ref()).arg("--json");
    if let Some(p) = policy {
        let pf = dir.join("policy.txt");
        std::fs::write(&pf, p).unwrap();
        cmd.arg("--policy").arg(&pf);
    }
    for v in ["CANDOR_DEPS", "CANDOR_CONFIG", "CANDOR_POLICY", "CANDOR_R1657F1_OFF", "CANDOR_R1657F4_OFF"] {
        cmd.env_remove(v);
    }
    for v in off {
        cmd.env(v, "1");
    }
    let out = cmd.output().expect("run candor-scan");
    let code = out.status.code().unwrap_or(-1);
    assert!((0..=2).contains(&code), "scan failed ({code}): {}", String::from_utf8_lossy(&out.stderr));
    (code, serde_json::from_slice(&out.stdout).expect("report json"))
}

fn field(v: &serde_json::Value, f: &str, key: &str) -> Vec<String> {
    let row = v["functions"].as_array().unwrap().iter().find(|r| r["fn"].as_str() == Some(f));
    let mut out: Vec<String> = row
        .and_then(|r| r[key].as_array())
        .map(|a| a.iter().filter_map(|e| e.as_str().map(str::to_string)).collect())
        .unwrap_or_default();
    out.sort();
    out
}

const F1: &[(&str, &str)] = &[
    ("lib.rs", r####"use core::pin::Pin;
use core::task::{Context, Poll};
pub mod io;
pub trait Tr { fn m(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<u8>; }
pub mod stream;
pub mod future;
"####),
    ("stream.rs", r####"use crate::*;

    use futures_core::Stream;
    pin_project_lite::pin_project! {
        /// local wrapper over a caller-chosen inner value
        pub struct W<S> { #[pin] inner: S, pub done: bool }
    }
    impl<S: Tr> Tr for W<S> {
        fn m(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<u8> { let this = self.project(); this.inner.m(cx) }
    }
    impl<S: Stream> Stream for W<S> {
        type Item = S::Item;
        fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<S::Item>> { let this = self.project(); this.inner.poll_next(cx) }
    }
    pin_project_lite::pin_project! {
        pub struct Chain<S, U> { #[pin] first: W<S>, #[pin] second: W<U> }
    }
    impl<S: Tr, U: Tr> Tr for Chain<S, U> {
        fn m(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<u8> {
            let mut this = self.project();
            if !this.first.done { return this.first.as_mut().m(cx); }
            this.second.as_mut().m(cx)
        }
    }
    impl<S: Stream, U: Stream<Item = S::Item>> Stream for Chain<S, U> {
        type Item = S::Item;
        fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<S::Item>> {
            let mut this = self.project();
            if !this.first.done { let next = core::task::ready!(this.first.as_mut().poll_next(cx)); if let Some(n) = next { return Poll::Ready(Some(n)); } }
            let n = core::task::ready!(this.second.as_mut().poll_next(cx)); Poll::Ready(n)
        }
    }
    pub trait StreamExt: Stream {
        fn poll_next(&mut self, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> where Self: Unpin { Stream::poll_next(Pin::new(self), cx) }
    }
    impl<S: Stream + ?Sized> StreamExt for S {}
    pin_project_lite::pin_project! { pub struct Race<S> { #[pin] s: S } }
    impl<S: Stream> Stream for Race<S> {
        type Item = S::Item;
        fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<S::Item>> { let this = self.project(); this.s.poll_next(cx) }
    }
    pub fn chain<S, U>(a: S, b: U) -> Chain<S, U> { Chain { first: W { inner: a, done: false }, second: W { inner: b, done: false } } }
"####),
    ("future.rs", r####"use core::pin::Pin;
use core::task::{Context, Poll};
use core::future::Future;
pin_project_lite::pin_project! {
    /// same-LEAF twin of `stream::W`, over a future
    pub struct W<F> { #[pin] fut: F, pub done: bool }
}
impl<F: Future> Future for W<F> {
    type Output = F::Output;
    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<F::Output> { let this = self.project(); this.fut.poll(cx) }
}
pin_project_lite::pin_project! { pub struct Race<A, B> { #[pin] a: A, #[pin] b: B } }
impl<A: Future, B: Future<Output = A::Output>> Future for Race<A, B> {
    type Output = A::Output;
    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<A::Output> { let this = self.project(); if let Poll::Ready(t) = this.a.poll(cx) { return Poll::Ready(t); } this.b.poll(cx) }
}
"####),
    ("io.rs", r####"use core::pin::Pin;
use core::task::{Context, Poll};
pin_project_lite::pin_project! {
    /// same-field twin: `first` is the bare parameter here
    pub struct Chain<R1, R2> { #[pin] first: R1, #[pin] second: R2, done_first: bool }
}
impl<R1: crate::Tr, R2: crate::Tr> crate::Tr for Chain<R1, R2> {
    fn m(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<u8> { let this = self.project(); if !*this.done_first { return this.first.m(cx); } this.second.m(cx) }
}
"####),
];

const F4: &[(&str, &str)] = &[
    ("lib.rs", r####"pub mod notify;
pub mod broadcast;
pub mod mpsc;

pub mod inlmod {

use crate::notify::Notify;
pub struct Shared { pub notify_last_rx_drop: Notify }
pub struct Receiver<T> { pub shared: Shared, pub next: u64, pub _t: Option<T> }
impl<T> Drop for Receiver<T> {
    fn drop(&mut self) {
        struct NotifyGuard<'a, T> { rx: &'a mut Receiver<T>, notify: bool }
        impl<T> Drop for NotifyGuard<'_, T> {
            fn drop(&mut self) {
                if self.notify { self.rx.shared.notify_last_rx_drop.notify_waiters(); }
            }
        }
        let guard = NotifyGuard { rx: self, notify: true };
        let _ = guard.rx.next;
    }
}
pub fn r1657f4_inl_drop_rx() { let _r: Receiver<u8> = Receiver { shared: Shared { notify_last_rx_drop: Notify }, next: 0, _t: None }; }


}
"####),
    ("notify.rs", r####"
pub struct Notify;
impl Notify { pub fn notify_waiters(&self) { std::fs::write(std::env::var("OUT").unwrap() + "/notify", b"x").unwrap(); } }
/// same-leaf twin of the body-local guard below, with a pure `notify_waiters`
pub struct NotifyGuard;
impl NotifyGuard { pub fn notify_waiters(&self) {} }

"####),
    ("broadcast.rs", r####"
use crate::notify::Notify;
pub struct Shared { pub notify_last_rx_drop: Notify }
pub struct Receiver<T> { pub shared: Shared, pub next: u64, pub _t: Option<T> }
impl<T> Drop for Receiver<T> {
    fn drop(&mut self) {
        struct NotifyGuard<'a, T> { rx: &'a mut Receiver<T>, notify: bool }
        impl<T> Drop for NotifyGuard<'_, T> {
            fn drop(&mut self) {
                if self.notify { self.rx.shared.notify_last_rx_drop.notify_waiters(); }
            }
        }
        let guard = NotifyGuard { rx: self, notify: true };
        let _ = guard.rx.next;
    }
}
pub fn r1657f4_drop_rx() { let _r: Receiver<u8> = Receiver { shared: Shared { notify_last_rx_drop: Notify }, next: 0, _t: None }; }

"####),
    ("mpsc.rs", r####"
/// same-leaf twin of `broadcast::Receiver`, declaring `rx` and `shared` differently
pub struct Receiver { pub rx: u8, pub shared: u8 }

"####),
];

#[test]
fn f1_a_projected_wrapper_field_beside_a_parameter_typed_twin_keeps_its_edge_and_key() {
    let d = fixture("f1", F1);
    let (_, v) = scan(&d, None, &[]);
    let disp = field(&v, "stream::Chain::poll_next", "dispatchesOn");
    assert!(disp.iter().any(|k| k == "futures_core#Stream::poll_next"), "the Stream key must survive: {disp:?}");
    assert!(field(&v, "stream::Chain::poll_next", "invisible").contains(&"futures_core".to_string()));
    let why = field(&v, "stream::Chain::poll_next", "unknownWhy");
    assert!(!why.iter().any(|w| w.contains("untyped receiver")), "resolved, not hedged: {why:?}");
    // The local-trait spelling: ABSENT before the fix; the row now carries the dispatch key of `S: Tr`.
    assert!(field(&v, "stream::Chain::m", "dispatchesOn").contains(&"fx#Tr::m".to_string()), "{:?}", field(&v, "stream::Chain::m", "dispatchesOn"));
    // Kill switch: the stage-1b reading returns.
    let (_, off) = scan(&d, None, &["CANDOR_R1657F1_OFF"]);
    // SOUNDNESS R1722 — this used to assert the legacy row's `dispatchesOn` is EMPTY, which was the outcome of a last-wins pick between the
    // two same-leaf `W` twins and so depended on the directory-listing order (the `future.rs` twin won on APFS, `stream.rs` wins in name
    // order). The legacy reading is not a stable thing to pin; the OFF scan is still run, so the switch stays exercised.
    let _ = off;
}

#[test]
fn f4_a_body_local_impl_types_self_as_its_own_self_type() {
    let d = fixture("f4", F4);
    for f in ["broadcast::Receiver::drop", "inlmod::Receiver::drop"] {
        let (_, v) = scan(&d, None, &[]);
        assert!(field(&v, f, "inferred").contains(&"Fs".to_string()), "`{f}`: {:?}", field(&v, f, "inferred"));
        let (code, _) = scan(&d, Some(&format!("deny Fs {f}\n")), &[]);
        assert_eq!(code, 1, "`deny Fs {f}` must fire");
        let (_, off) = scan(&d, None, &["CANDOR_R1657F4_OFF"]);
        assert!(!field(&off, f, "inferred").contains(&"Fs".to_string()), "the switch must restore the hedge on `{f}`");
    }
}

/// F4's (c'') leg. tokio 1.25–1.4x `TaskLocalFuture::fmt` formats a caller-chosen `T: Debug` inside a body-local
/// `impl Debug for TransparentOption<'_, T>`. Once F4 typed that impl's `self`, the row read pure where 0.41.3 carried a
/// (wrong-reason) hedge — and a LIVE chained consumer (real tokio 1.36 report; join proven by a control probe) whose `T`
/// writes a file in `Debug::fmt` (EXECUTED) went from `Unknown` on 0.41.3 to ABSENT: no consumer-side route answers a
/// dependency wrapper that formats the consumer's `T`. The true reason is disclosed on exactly such a unit:
/// `dispatch:Debug.fmt`. A module-level impl of the same shape is unchanged (it was not this change's).
#[test]
fn f4_a_body_local_formatter_over_a_caller_chosen_type_discloses_its_dispatch() {
    let src = "use std::fmt;\n\
        pub struct Outer<T> { slot: Option<T> }\n\
        impl<T: fmt::Debug> fmt::Debug for Outer<T> {\n\
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {\n\
                struct Inner<'a, T> { value: &'a Option<T> }\n\
                impl<'a, T: fmt::Debug> fmt::Debug for Inner<'a, T> {\n\
                    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {\n\
                        match self.value.as_ref() { Some(value) => value.fmt(f), None => f.pad(\"<missing>\") }\n\
                    }\n\
                }\n\
                f.debug_struct(\"Outer\").field(\"value\", &Inner { value: &self.slot }).finish()\n\
            }\n\
        }\n";
    let d = fixture("f4fmt", &[("lib.rs", src)]);
    let (_, v) = scan(&d, None, &[]);
    assert_eq!(field(&v, "Outer::fmt", "unknownWhy"), vec!["dispatch:Debug.fmt".to_string()], "{v:#}");
    let (_, off) = scan(&d, None, &["CANDOR_R1657F4_OFF"]);
    assert!(!field(&off, "Outer::fmt", "unknownWhy").iter().any(|w| w == "dispatch:Debug.fmt"));
}
