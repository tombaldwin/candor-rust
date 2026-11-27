mod reexp_fl15g { pub use fxdep_fl15g::*; }
mod g_fl15g {
    use crate::reexp_fl15g::*;
    pub fn gen_q<T: Default>() -> T { T::default() }
}

pub fn fl15g_q() { let _: u8 = g_fl15g::gen_q::<u8>(); }

fn main() {
    match std::env::args().nth(1).unwrap().as_str() {
        "fl15g_q" => fl15g_q(),
        _ => panic!("unknown probe"),
    }
}
