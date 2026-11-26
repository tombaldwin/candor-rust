#[cfg(unix)]
crate::mkfn!{ go_d10 }
#[cfg(not(unix))]
pub fn go_d10() {}
#[cfg(unix)]
crate::mkfnp!{ qgo_d10 }
#[cfg(not(unix))]
pub fn qgo_d10() {}
pub fn p_inner_d10() { go_d10() }
pub fn q_inner_d10() { qgo_d10() }
