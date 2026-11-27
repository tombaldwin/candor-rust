#![allow(unused, non_camel_case_types, non_snake_case, dead_code)]
use fxdep_wg3::{DE, DQ};
use fxdep_wg3::Sub as DS;
use fxdep_wg3::q::Sub as DSQ;
pub struct ItE_wg3; impl Iterator for ItE_wg3 { type Item = DE; fn next(&mut self) -> Option<DE> { None } }
pub struct ItQ_wg3; impl Iterator for ItQ_wg3 { type Item = DQ; fn next(&mut self) -> Option<DQ> { None } }
mod m_ren_e { use super::*; 
    pub fn path<T: DS>() { T::sf() }
    pub fn meth<T: DS>(t: &T) { t.sm() }
    pub fn proj<I: Iterator>() where I::Item: DS { I::Item::sf() }
}
pub fn wg3_ren_path_e() { m_ren_e::path::<DE>() }
pub fn wg3_ren_meth_e() { m_ren_e::meth(&DE) }
pub fn wg3_ren_proj_e() { m_ren_e::proj::<ItE_wg3>() }
mod m_ren_q { use super::*; 
    pub fn path<T: DSQ>() { T::sf() }
    pub fn meth<T: DSQ>(t: &T) { t.sm() }
    pub fn proj<I: Iterator>() where I::Item: DSQ { I::Item::sf() }
}
pub fn wg3_ren_path_q() { m_ren_q::path::<DQ>() }
pub fn wg3_ren_meth_q() { m_ren_q::meth(&DQ) }
pub fn wg3_ren_proj_q() { m_ren_q::proj::<ItQ_wg3>() }
mod m_err_e { use super::*; use fxdep_wg3::Error as Tr_err_e; 
    pub fn path<T: fxdep_wg3::Error>() { T::ef() }
    pub fn meth<T: fxdep_wg3::Error>(t: &T) { t.em() }
    pub fn proj<I: Iterator>() where I::Item: fxdep_wg3::Error { I::Item::ef() }
}
pub fn wg3_err_path_e() { m_err_e::path::<DE>() }
pub fn wg3_err_meth_e() { m_err_e::meth(&DE) }
pub fn wg3_err_proj_e() { m_err_e::proj::<ItE_wg3>() }
mod m_err_q { use super::*; use fxdep_wg3::q::Error as Tr_err_q; 
    pub fn path<T: fxdep_wg3::q::Error>() { T::ef() }
    pub fn meth<T: fxdep_wg3::q::Error>(t: &T) { t.em() }
    pub fn proj<I: Iterator>() where I::Item: fxdep_wg3::q::Error { I::Item::ef() }
}
pub fn wg3_err_path_q() { m_err_q::path::<DQ>() }
pub fn wg3_err_meth_q() { m_err_q::meth(&DQ) }
pub fn wg3_err_proj_q() { m_err_q::proj::<ItQ_wg3>() }
mod m_read_e { use super::*; use fxdep_wg3::Read as Tr_read_e; 
    pub fn path<T: fxdep_wg3::Read>() { T::rf() }
    pub fn meth<T: fxdep_wg3::Read>(t: &T) { t.rm() }
    pub fn proj<I: Iterator>() where I::Item: fxdep_wg3::Read { I::Item::rf() }
}
pub fn wg3_read_path_e() { m_read_e::path::<DE>() }
pub fn wg3_read_meth_e() { m_read_e::meth(&DE) }
pub fn wg3_read_proj_e() { m_read_e::proj::<ItE_wg3>() }
mod m_read_q { use super::*; use fxdep_wg3::q::Read as Tr_read_q; 
    pub fn path<T: fxdep_wg3::q::Read>() { T::rf() }
    pub fn meth<T: fxdep_wg3::q::Read>(t: &T) { t.rm() }
    pub fn proj<I: Iterator>() where I::Item: fxdep_wg3::q::Read { I::Item::rf() }
}
pub fn wg3_read_path_q() { m_read_q::path::<DQ>() }
pub fn wg3_read_meth_q() { m_read_q::meth(&DQ) }
pub fn wg3_read_proj_q() { m_read_q::proj::<ItQ_wg3>() }
mod m_ptr_e { use super::*; use fxdep_wg3::Pointee as Tr_ptr_e; 
    pub fn path<T: fxdep_wg3::Pointee>() { T::pf() }
    pub fn meth<T: fxdep_wg3::Pointee>(t: &T) { t.pm() }
    pub fn proj<I: Iterator>() where I::Item: fxdep_wg3::Pointee { I::Item::pf() }
}
pub fn wg3_ptr_path_e() { m_ptr_e::path::<DE>() }
pub fn wg3_ptr_meth_e() { m_ptr_e::meth(&DE) }
pub fn wg3_ptr_proj_e() { m_ptr_e::proj::<ItE_wg3>() }
mod m_ptr_q { use super::*; use fxdep_wg3::q::Pointee as Tr_ptr_q; 
    pub fn path<T: fxdep_wg3::q::Pointee>() { T::pf() }
    pub fn meth<T: fxdep_wg3::q::Pointee>(t: &T) { t.pm() }
    pub fn proj<I: Iterator>() where I::Item: fxdep_wg3::q::Pointee { I::Item::pf() }
}
pub fn wg3_ptr_path_q() { m_ptr_q::path::<DQ>() }
pub fn wg3_ptr_meth_q() { m_ptr_q::meth(&DQ) }
pub fn wg3_ptr_proj_q() { m_ptr_q::proj::<ItQ_wg3>() }


fn main() {
    match std::env::args().nth(1).unwrap().as_str() {
        "wg3_ren_path_e" => wg3_ren_path_e(),
        "wg3_ren_meth_e" => wg3_ren_meth_e(),
        "wg3_ren_proj_e" => wg3_ren_proj_e(),
        "wg3_ren_path_q" => wg3_ren_path_q(),
        "wg3_ren_meth_q" => wg3_ren_meth_q(),
        "wg3_ren_proj_q" => wg3_ren_proj_q(),
        "wg3_err_path_e" => wg3_err_path_e(),
        "wg3_err_meth_e" => wg3_err_meth_e(),
        "wg3_err_proj_e" => wg3_err_proj_e(),
        "wg3_err_path_q" => wg3_err_path_q(),
        "wg3_err_meth_q" => wg3_err_meth_q(),
        "wg3_err_proj_q" => wg3_err_proj_q(),
        "wg3_read_path_e" => wg3_read_path_e(),
        "wg3_read_meth_e" => wg3_read_meth_e(),
        "wg3_read_proj_e" => wg3_read_proj_e(),
        "wg3_read_path_q" => wg3_read_path_q(),
        "wg3_read_meth_q" => wg3_read_meth_q(),
        "wg3_read_proj_q" => wg3_read_proj_q(),
        "wg3_ptr_path_e" => wg3_ptr_path_e(),
        "wg3_ptr_meth_e" => wg3_ptr_meth_e(),
        "wg3_ptr_proj_e" => wg3_ptr_proj_e(),
        "wg3_ptr_path_q" => wg3_ptr_path_q(),
        "wg3_ptr_meth_q" => wg3_ptr_meth_q(),
        "wg3_ptr_proj_q" => wg3_ptr_proj_q(),
        _ => panic!("unknown probe"),
    }
}
