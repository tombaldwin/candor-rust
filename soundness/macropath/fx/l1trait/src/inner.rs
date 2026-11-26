use crate::defs::{mktr_l1trait, mktrp_l1trait};
pub struct S_l1trait;
pub struct SQ_l1trait;
pub trait T_l1trait { self::mktr_l1trait!{ go_l1trait } }
impl T_l1trait for S_l1trait {}
pub trait TQ_l1trait { self::mktrp_l1trait!{ qgo_l1trait } }
impl TQ_l1trait for SQ_l1trait {}
