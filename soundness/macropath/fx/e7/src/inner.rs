mod x { use fxdepe7::*; }
thread_local! { static X_e7: u8 = 0; }
#[cfg(unix)] pub fn gated_e7() {}
#[cfg(not(unix))] pub fn gated_e7() {}
pub fn p_go_e7() { X_e7.with(|_| ()) }
