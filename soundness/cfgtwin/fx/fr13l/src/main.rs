mod a_fr13l { use std::slice::SliceIndex; use fxdep_fr13l::GetU; pub fn e() { let _ = usize::get(1u8); } }
mod b_fr13l { use std::slice::SliceIndex; use std::ops::Range; use fxdep_fr13l::GetU; pub fn e() { let _ = Range::<usize>::get(1u8); } }

pub fn fr13l_usize_e() { a_fr13l::e(); }
pub fn fr13l_range_e() { b_fr13l::e(); }

fn main() {
    match std::env::args().nth(1).unwrap().as_str() {
        "fr13l_usize_e" => fr13l_usize_e(),
        "fr13l_range_e" => fr13l_range_e(),
        _ => panic!("unknown probe"),
    }
}
