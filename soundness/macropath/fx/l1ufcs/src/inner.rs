use crate::defs::{mkm_l1ufcs, mkmp_l1ufcs};
pub struct S_l1ufcs;
impl S_l1ufcs { self::mkm_l1ufcs!{ go_l1ufcs } }
pub struct SQ_l1ufcs;
impl SQ_l1ufcs { self::mkmp_l1ufcs!{ qgo_l1ufcs } }
