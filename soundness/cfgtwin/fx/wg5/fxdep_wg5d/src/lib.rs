pub trait Flat { fn f_go(&self); fn f_fn(); }
pub trait BaseS { fn s_go(&self); fn s_fn(); }
pub trait SubS: BaseS {}
pub trait FlatQ { fn f_go(&self); fn f_fn(); }
pub trait BaseSQ { fn s_go(&self); fn s_fn(); }
pub trait SubSQ: BaseSQ {}
