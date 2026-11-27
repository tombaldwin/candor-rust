#![allow(unused, non_camel_case_types, non_snake_case, dead_code)]
pub struct Mine; pub struct Quiet;
impl fxdep_l2wd::Flat for Mine { fn f_go(&self) { ::std::fs::write(::std::env::var("MARK").unwrap(), b"x").unwrap(); } fn f_fn() { ::std::fs::write(::std::env::var("MARK").unwrap(), b"x").unwrap(); } }
impl fxdep_l2wd::BaseS for Mine { fn s_go(&self) { ::std::fs::write(::std::env::var("MARK").unwrap(), b"x").unwrap(); } fn s_fn() { ::std::fs::write(::std::env::var("MARK").unwrap(), b"x").unwrap(); } } impl fxdep_l2wd::SubS for Mine {}
impl fxdep_l2wd::m::Deep for Mine { fn dg(&self) { ::std::fs::write(::std::env::var("MARK").unwrap(), b"x").unwrap(); } fn df() { ::std::fs::write(::std::env::var("MARK").unwrap(), b"x").unwrap(); } } impl fxdep_l2wd::Mid for Mine {} impl fxdep_l2wd::Top for Mine {}
impl fxdep_l2wf::Far for Mine { fn ff() { ::std::fs::write(::std::env::var("MARK").unwrap(), b"x").unwrap(); } fn fg(&self) { ::std::fs::write(::std::env::var("MARK").unwrap(), b"x").unwrap(); } } impl fxdep_l2wd::OverFar for Mine {}
impl fxdep_l2wd::In for Mine { fn ig(&self) { ::std::fs::write(::std::env::var("MARK").unwrap(), b"x").unwrap(); } } impl fxdep_l2wd::OverIn for Mine {}
impl fxdep_l2wd::SupA for Mine { fn sa() { ::std::fs::write(::std::env::var("MARK").unwrap(), b"x").unwrap(); } } impl fxdep_l2wd::SupB for Mine { fn sb() {} } impl fxdep_l2wp::LSub2 for Mine {}
impl fxdep_l2wd::Dt for Mine { fn f() {} } impl fxdep_l2wd::SupD for Mine { fn sf() { ::std::fs::write(::std::env::var("MARK").unwrap(), b"x").unwrap(); } } impl fxdep_l2wp::Lt5 for Mine {}
impl fxdep_l2wd::Own for Mine { fn o_go(&self) {} } impl fxdep_l2wd::OwnSub for Mine { fn o_go(&self) { ::std::fs::write(::std::env::var("MARK").unwrap(), b"x").unwrap(); } }
pub struct ItMine; impl Iterator for ItMine { type Item = Mine; fn next(&mut self) -> Option<Mine> { None } }
impl fxdep_l2wd::FlatQ for Quiet { fn f_go(&self) {  } fn f_fn() {  } }
impl fxdep_l2wd::BaseSQ for Quiet { fn s_go(&self) {  } fn s_fn() {  } } impl fxdep_l2wd::SubSQ for Quiet {}
impl fxdep_l2wd::m::DeepQ for Quiet { fn dg(&self) {  } fn df() {  } } impl fxdep_l2wd::MidQ for Quiet {} impl fxdep_l2wd::TopQ for Quiet {}
impl fxdep_l2wf::FarQ for Quiet { fn ff() {  } fn fg(&self) {  } } impl fxdep_l2wd::OverFarQ for Quiet {}
impl fxdep_l2wd::InQ for Quiet { fn ig(&self) {  } } impl fxdep_l2wd::OverInQ for Quiet {}
impl fxdep_l2wd::SupAQ for Quiet { fn sa() {  } } impl fxdep_l2wd::SupBQ for Quiet { fn sb() {} } impl fxdep_l2wp::LSub2Q for Quiet {}
impl fxdep_l2wd::DtQ for Quiet { fn f() {} } impl fxdep_l2wd::SupDQ for Quiet { fn sf() {  } } impl fxdep_l2wp::Lt5Q for Quiet {}
impl fxdep_l2wd::OwnQ for Quiet { fn o_go(&self) {} } impl fxdep_l2wd::OwnSubQ for Quiet { fn o_go(&self) {  } }
pub struct ItQuiet; impl Iterator for ItQuiet { type Item = Quiet; fn next(&mut self) -> Option<Quiet> { None } }
impl fxdep_l2wp::g::LSubDG for Mine {} impl fxdep_l2wp::g::LSubDGQ for Quiet {}
pub fn l2w_flat_e() { fxdep_l2wp::p_flat_e(&Mine); }
pub fn l2w_flat_q() { fxdep_l2wp::p_flat_q(&Quiet); }
pub fn l2w_supmeth_e() { fxdep_l2wp::p_supmeth_e(&Mine); }
pub fn l2w_supmeth_q() { fxdep_l2wp::p_supmeth_q(&Quiet); }
pub fn l2w_suppath_e() { fxdep_l2wp::p_suppath_e::<Mine>(); }
pub fn l2w_suppath_q() { fxdep_l2wp::p_suppath_q::<Quiet>(); }
pub fn l2w_supqself_e() { fxdep_l2wp::p_supqself_e::<Mine>(); }
pub fn l2w_supqself_q() { fxdep_l2wp::p_supqself_q::<Quiet>(); }
pub fn l2w_proj_e() { fxdep_l2wp::p_proj_e::<ItMine>(); }
pub fn l2w_proj_q() { fxdep_l2wp::p_proj_q::<ItQuiet>(); }
pub fn l2w_deep_e() { fxdep_l2wp::p_deep_e(&Mine); }
pub fn l2w_deep_q() { fxdep_l2wp::p_deep_q(&Quiet); }
pub fn l2w_deeppath_e() { fxdep_l2wp::p_deeppath_e::<Mine>(); }
pub fn l2w_deeppath_q() { fxdep_l2wp::p_deeppath_q::<Quiet>(); }
pub fn l2w_far_e() { fxdep_l2wp::p_far_e(&Mine); }
pub fn l2w_far_q() { fxdep_l2wp::p_far_q(&Quiet); }
pub fn l2w_farpath_e() { fxdep_l2wp::p_farpath_e::<Mine>(); }
pub fn l2w_farpath_q() { fxdep_l2wp::p_farpath_q::<Quiet>(); }
pub fn l2w_reexp_e() { fxdep_l2wp::p_reexp_e(&Mine); }
pub fn l2w_reexp_q() { fxdep_l2wp::p_reexp_q(&Quiet); }
pub fn l2w_two_e() { fxdep_l2wp::p_two_e::<Mine>(); }
pub fn l2w_two_q() { fxdep_l2wp::p_two_q::<Quiet>(); }
pub fn l2w_mixed_e() { fxdep_l2wp::p_mixed_e::<Mine>(); }
pub fn l2w_mixed_q() { fxdep_l2wp::p_mixed_q::<Quiet>(); }
pub fn l2w_own_e() { fxdep_l2wp::p_own_e(&Mine); }
pub fn l2w_own_q() { fxdep_l2wp::p_own_q(&Quiet); }
pub fn l2w_gsuppath_e() { fxdep_l2wp::g::p_gsuppath_e::<Mine>(); }
pub fn l2w_gsuppath_q() { fxdep_l2wp::g::p_gsuppath_q::<Quiet>(); }
pub fn l2w_gsupqself_e() { fxdep_l2wp::g::p_gsupqself_e::<Mine>(); }
pub fn l2w_gsupqself_q() { fxdep_l2wp::g::p_gsupqself_q::<Quiet>(); }
pub fn l2w_gproj_e() { fxdep_l2wp::g::p_gproj_e::<ItMine>(); }
pub fn l2w_gproj_q() { fxdep_l2wp::g::p_gproj_q::<ItQuiet>(); }
pub fn l2w_glsdmeth_e() { fxdep_l2wp::g::p_glsdmeth_e(&Mine); }
pub fn l2w_glsdmeth_q() { fxdep_l2wp::g::p_glsdmeth_q(&Quiet); }


fn main() {
    match std::env::args().nth(1).unwrap().as_str() {
        "l2w_flat_e" => l2w_flat_e(),
        "l2w_flat_q" => l2w_flat_q(),
        "l2w_supmeth_e" => l2w_supmeth_e(),
        "l2w_supmeth_q" => l2w_supmeth_q(),
        "l2w_suppath_e" => l2w_suppath_e(),
        "l2w_suppath_q" => l2w_suppath_q(),
        "l2w_supqself_e" => l2w_supqself_e(),
        "l2w_supqself_q" => l2w_supqself_q(),
        "l2w_proj_e" => l2w_proj_e(),
        "l2w_proj_q" => l2w_proj_q(),
        "l2w_deep_e" => l2w_deep_e(),
        "l2w_deep_q" => l2w_deep_q(),
        "l2w_deeppath_e" => l2w_deeppath_e(),
        "l2w_deeppath_q" => l2w_deeppath_q(),
        "l2w_far_e" => l2w_far_e(),
        "l2w_far_q" => l2w_far_q(),
        "l2w_farpath_e" => l2w_farpath_e(),
        "l2w_farpath_q" => l2w_farpath_q(),
        "l2w_reexp_e" => l2w_reexp_e(),
        "l2w_reexp_q" => l2w_reexp_q(),
        "l2w_two_e" => l2w_two_e(),
        "l2w_two_q" => l2w_two_q(),
        "l2w_mixed_e" => l2w_mixed_e(),
        "l2w_mixed_q" => l2w_mixed_q(),
        "l2w_own_e" => l2w_own_e(),
        "l2w_own_q" => l2w_own_q(),
        "l2w_gsuppath_e" => l2w_gsuppath_e(),
        "l2w_gsuppath_q" => l2w_gsuppath_q(),
        "l2w_gsupqself_e" => l2w_gsupqself_e(),
        "l2w_gsupqself_q" => l2w_gsupqself_q(),
        "l2w_gproj_e" => l2w_gproj_e(),
        "l2w_gproj_q" => l2w_gproj_q(),
        "l2w_glsdmeth_e" => l2w_glsdmeth_e(),
        "l2w_glsdmeth_q" => l2w_glsdmeth_q(),
        _ => panic!("unknown probe"),
    }
}
