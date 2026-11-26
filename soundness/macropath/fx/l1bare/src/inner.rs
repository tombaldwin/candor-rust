use crate::defs::{mkfn_l1bare, mkfnp_l1bare};
self::mkfn_l1bare!{ go_l1bare }
self::mkfnp_l1bare!{ qgo_l1bare }
pub fn p_inner_l1bare() { go_l1bare() }
pub fn q_inner_l1bare() { qgo_l1bare() }
pub fn l1bare_d() { go_l1bare() }
pub fn l1bare_dq() { qgo_l1bare() }
