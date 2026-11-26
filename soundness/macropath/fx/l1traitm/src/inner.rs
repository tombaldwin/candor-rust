use crate::defs::{mktr_l1traitm, mktrp_l1traitm};
pub struct S_l1traitm;
pub struct SQ_l1traitm;
pub trait T_l1traitm { self::mktr_l1traitm!{ go_l1traitm } }
impl T_l1traitm for S_l1traitm {}
pub trait TQ_l1traitm { self::mktrp_l1traitm!{ qgo_l1traitm } }
impl TQ_l1traitm for SQ_l1traitm {}
