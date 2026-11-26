pub mod deep { use crate::defs::{mkfn_l1deep, mkfnp_l1deep}; self::mkfn_l1deep!{ go_l1deep } self::mkfnp_l1deep!{ qgo_l1deep } }
pub fn rel_l1deep() { deep::go_l1deep() }
pub fn relq_l1deep() { deep::qgo_l1deep() }
