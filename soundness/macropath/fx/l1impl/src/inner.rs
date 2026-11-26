use crate::defs::{mkm_l1impl, mkmp_l1impl};
pub struct S_l1impl;
impl S_l1impl { self::mkm_l1impl!{ go_l1impl } }
pub struct SQ_l1impl;
impl SQ_l1impl { self::mkmp_l1impl!{ qgo_l1impl } }
