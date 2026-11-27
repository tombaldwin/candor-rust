pub struct Qe_fl22; pub struct Qq_fl22;
#[cfg(not(unix))] impl fxdep_fl22::TD for Qe_fl22 { fn go(&self) {} }
#[cfg(unix)] impl fxdep_fl22::TD for Qe_fl22 {}
#[cfg(not(unix))] impl fxdep_fl22::TQ for Qq_fl22 { fn goq(&self) {} }
#[cfg(unix)] impl fxdep_fl22::TQ for Qq_fl22 {}

pub fn fl22_e() { use fxdep_fl22::TD; Qe_fl22.go(); }
pub fn fl22_q() { use fxdep_fl22::TQ; Qq_fl22.goq(); }

fn main() {
    match std::env::args().nth(1).unwrap().as_str() {
        "fl22_e" => fl22_e(),
        "fl22_q" => fl22_q(),
        _ => panic!("unknown probe"),
    }
}
