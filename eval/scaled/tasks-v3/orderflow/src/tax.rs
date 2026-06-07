//! Sales tax — a pure percentage applied to a money amount.

use crate::money::Money;

pub struct Tax {
    pct: i64,
}

impl Tax {
    pub fn new(pct: i64) -> Self {
        Tax { pct }
    }

    /// The tax due on `amount`.
    pub fn apply(&self, amount: Money) -> Money {
        amount.scale_percent(self.pct)
    }
}
