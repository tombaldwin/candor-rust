pub trait BaseS { fn s_go(&self); fn s_fn(); }
pub trait SubS: BaseS {}
pub mod m { pub trait Deep { fn df(); } }
pub trait Mid: m::Deep {}
pub trait Top: Mid {}
use fxdep_l2kf::Far;
pub trait OverFar: Far {}
pub trait SupA { fn sa(); }
pub trait SupB { fn sb(); }
pub trait Dt { fn f(); }
pub trait SupD { fn sf(); }
pub trait Flat { fn f_fn(); }
macro_rules! decl_t { () => { pub trait MadeT { fn mk(); } } }
decl_t!();
pub trait OverMade: MadeT {}
pub struct DE;
impl BaseS for DE { fn s_go(&self) { ::std::fs::write(::std::env::var("MARK").unwrap(), b"x").unwrap(); } fn s_fn() { ::std::fs::write(::std::env::var("MARK").unwrap(), b"x").unwrap(); } } impl SubS for DE {}
impl m::Deep for DE { fn df() { ::std::fs::write(::std::env::var("MARK").unwrap(), b"x").unwrap(); } } impl Mid for DE {} impl Top for DE {}
impl Far for DE { fn ff() { ::std::fs::write(::std::env::var("MARK").unwrap(), b"x").unwrap(); } } impl OverFar for DE {}
impl Flat for DE { fn f_fn() { ::std::fs::write(::std::env::var("MARK").unwrap(), b"x").unwrap(); } }
impl MadeT for DE { fn mk() { ::std::fs::write(::std::env::var("MARK").unwrap(), b"x").unwrap(); } } impl OverMade for DE {}
