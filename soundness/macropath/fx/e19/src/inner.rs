mod x { use fxdepe19::thread_local; }
thread_local! { static X_e19: u8 = 0; }
#[cfg(unix)] pub fn gated_e19() {}
#[cfg(not(unix))] pub fn gated_e19() {}
pub fn p_go_e19() { X_e19.with(|_| ()) }
