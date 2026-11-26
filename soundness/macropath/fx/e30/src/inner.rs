mod helpers { pub fn h_e30() {} }
use helpers::*;
thread_local! { static X_e30: u8 = 0; }
#[cfg(unix)] pub fn gated_e30() {}
#[cfg(not(unix))] pub fn gated_e30() {}
pub fn p_go_e30() { tl_touch() }
