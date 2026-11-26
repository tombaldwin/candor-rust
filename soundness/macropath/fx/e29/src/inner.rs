mod helpers { pub fn h_e29() {} }
use helpers::*;
thread_local! { static X_e29: u8 = 0; }
#[cfg(unix)] pub fn gated_e29() {}
#[cfg(not(unix))] pub fn gated_e29() {}
pub fn p_go_e29() { tl_touch() }
