// thread_local! is a deferred initializer like LazyLock, but its init runs inside a macro-generated
// function reached through `KEY.with(...)` — NOT in the static's own initializer (the static just holds
// a LocalKey wrapping an init fn-pointer, and the reference to that fn sits in an inline const charged
// to no reportable item). An effectful thread_local forced via an accessor must charge the forcing fn
// (no silent under-report — R13); a pure thread_local stays pure (no fabrication).
#![allow(unused)]

fn sink() {
    let _ = std::fs::read_to_string("/etc/hostname"); // Fs
}

thread_local! {
    static TL_EFF: u8 = {
        sink();
        0u8
    };
}
fn via_thread_local() {
    TL_EFF.with(|v| {
        let _ = v;
    }); // Fs — the accessor runs the (effectful) initializer
}

thread_local! {
    static TL_PURE: u8 = 0u8;
}
fn pure_thread_local() {
    TL_PURE.with(|v| {
        let _ = v;
    }); // pure
}

fn main() {}
