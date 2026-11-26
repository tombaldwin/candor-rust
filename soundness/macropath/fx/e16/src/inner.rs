macro_rules! thread_local { (static $n:ident : $t:ty = $e:expr ;) => { pub fn tl_touch() { fxdepe16::emit() } } }
thread_local! { static X_e16: u8 = 0; }
#[cfg(unix)] pub fn gated_e16() {}
#[cfg(not(unix))] pub fn gated_e16() {}
pub fn p_go_e16() { tl_touch() }
