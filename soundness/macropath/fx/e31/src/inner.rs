mod helpers { pub fn h_e31() {} }
use helpers::*;
thread_local! { static X_e31: u8 = 0; }
#[cfg(unix)] pub fn gated_e31() {}
#[cfg(not(unix))] pub fn gated_e31() {}
pub fn p_go_e31() { tl_touch() }
