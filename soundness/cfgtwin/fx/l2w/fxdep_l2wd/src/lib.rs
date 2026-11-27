pub trait Flat { fn f_go(&self); fn f_fn(); }
pub trait BaseS { fn s_go(&self); fn s_fn(); }
pub trait SubS: BaseS {}
pub trait Mid: m::Deep {}
pub trait Top: Mid {}
pub trait OverIn: In {}
pub trait SupA { fn sa(); }
pub trait SupB { fn sb(); }
pub trait Dt { fn f(); }
pub trait SupD { fn sf(); }
pub trait Own { fn o_go(&self); }
pub trait OwnSub: Own { fn o_go(&self); }
pub trait FlatQ { fn f_go(&self); fn f_fn(); }
pub trait BaseSQ { fn s_go(&self); fn s_fn(); }
pub trait SubSQ: BaseSQ {}
pub trait MidQ: m::DeepQ {}
pub trait TopQ: MidQ {}
pub trait OverInQ: InQ {}
pub trait SupAQ { fn sa(); }
pub trait SupBQ { fn sb(); }
pub trait DtQ { fn f(); }
pub trait SupDQ { fn sf(); }
pub trait OwnQ { fn o_go(&self); }
pub trait OwnSubQ: OwnQ { fn o_go(&self); }
pub mod m { pub trait Deep { fn dg(&self); fn df(); } pub trait DeepQ { fn dg(&self); fn df(); } }
mod inner { pub trait In { fn ig(&self); } pub trait InQ { fn ig(&self); } }
pub use inner::{In, InQ};
use fxdep_l2wf::Far;
pub trait OverFar: Far {}
pub trait OverFarQ: fxdep_l2wf::FarQ {}
