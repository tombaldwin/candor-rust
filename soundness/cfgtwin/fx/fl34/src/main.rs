mod g_fl34 {
    use fxdep_fl34::Af;
    pub fn assoc2<T: fxdep_fl34::HasA>() { T::A::f() }
    pub fn assoc_q<T: fxdep_fl34::HasA>() { <T as fxdep_fl34::HasA>::A::f() }
    pub fn assoc_sup<T: fxdep_fl34::Sub>() { T::S::f() }
    pub fn assoc_two<T: fxdep_fl34::HasA + fxdep_fl34::HasB>() { T::A::f() }
}

pub fn fl34_e() { g_fl34::assoc2::<fxdep_fl34::Imp>(); }
pub fn fl34_qself_e() { g_fl34::assoc_q::<fxdep_fl34::Imp>(); }
pub fn fl34_sup_e() { g_fl34::assoc_sup::<fxdep_fl34::Imp>(); }
pub fn fl34_two_e() { g_fl34::assoc_two::<fxdep_fl34::Imp>(); }
pub fn fl34_q() { g_fl34::assoc2::<fxdep_fl34::Pur>(); }

fn main() {
    match std::env::args().nth(1).unwrap().as_str() {
        "fl34_e" => fl34_e(),
        "fl34_qself_e" => fl34_qself_e(),
        "fl34_sup_e" => fl34_sup_e(),
        "fl34_two_e" => fl34_two_e(),
        "fl34_q" => fl34_q(),
        _ => panic!("unknown probe"),
    }
}
