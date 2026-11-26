use crate::defs::{mktr_l1timpl, mktrp_l1timpl};
pub trait Tr_l1timpl { fn go_l1timpl(&self); }
pub trait TrQ_l1timpl { fn qgo_l1timpl(&self); }
pub struct S_l1timpl;
impl Tr_l1timpl for S_l1timpl { self::mktr_l1timpl!{ go_l1timpl } }
pub struct SQ_l1timpl;
impl TrQ_l1timpl for SQ_l1timpl { self::mktrp_l1timpl!{ qgo_l1timpl } }
