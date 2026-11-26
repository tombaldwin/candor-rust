use crate::defs::{mkfn_l1let, mkfnp_l1let};
self::mkfn_l1let!{ go_l1let }
self::mkfnp_l1let!{ qgo_l1let }
pub fn p_inner_l1let() { let go_l1let = || (); go_l1let() }
