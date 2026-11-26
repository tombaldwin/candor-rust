use crate::defs::{mkfn, mkfnp};
self::mkfn!{ go_d11 }
self::mkfnp!{ qgo_d11 }
pub fn p_inner_d11() { go_d11() }
pub fn q_inner_d11() { qgo_d11() }
