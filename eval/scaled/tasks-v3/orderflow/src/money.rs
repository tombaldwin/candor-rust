//! Money values and currency codes. Pure arithmetic — no I/O.

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Currency {
    Usd,
    Eur,
    Gbp,
}

#[derive(Clone, Copy, Debug)]
pub struct Money {
    pub cents: i64,
    pub currency: Currency,
}

impl Money {
    pub fn new(cents: i64, currency: Currency) -> Self {
        Money { cents, currency }
    }

    /// Convert to `to` at `rate_milli` (the rate × 1000, so 1.000 == 1000).
    pub fn convert(self, rate_milli: i64, to: Currency) -> Money {
        Money {
            cents: self.cents * rate_milli / 1000,
            currency: to,
        }
    }

    pub fn add(self, other: Money) -> Money {
        Money {
            cents: self.cents + other.cents,
            currency: self.currency,
        }
    }

    pub fn scale_percent(self, pct: i64) -> Money {
        Money {
            cents: self.cents * pct / 100,
            currency: self.currency,
        }
    }
}
