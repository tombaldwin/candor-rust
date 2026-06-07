//! The pricing domain. This module is the PURE core of the application:
//! given a catalogue and an FX rate that has ALREADY been supplied to it, it
//! computes quotes. It must never reach for the network, the filesystem, the
//! clock, the environment, or a subprocess. See ARCHITECTURE.md.
//!
//! The FX rate lives in `self.rate_milli`; it is set by the service layer
//! (which is allowed to do I/O) via `set_rate`. Pricing only ever READS it.

use crate::money::{Currency, Money};

pub struct Pricing {
    /// USD->target rate in milli-units (1000 == parity). Defaults to parity.
    rate_milli: i64,
}

impl Pricing {
    pub fn new() -> Self {
        Pricing { rate_milli: 1000 }
    }

    /// Supply a fresh FX rate. Called by the service layer.
    pub fn set_rate(&mut self, rate_milli: i64) {
        self.rate_milli = rate_milli;
    }

    /// List price for a SKU, in USD minor units. A tiny fixed catalogue.
    fn list_price_usd(&self, sku: &str) -> Money {
        let minor = match sku {
            "WIDGET" => 1999,
            "GADGET" => 4999,
            "GIZMO" => 9900,
            _ => 0,
        };
        Money::new(minor, Currency::Usd)
    }

    /// Quote a SKU in the requested currency, applying the current FX rate.
    /// Pure: reads the catalogue and `self.rate_milli`, does arithmetic.
    pub fn quote(&self, sku: &str, currency: Currency) -> Money {
        let base = self.list_price_usd(sku);
        base.convert(self.rate_milli, currency)
    }
}

impl Default for Pricing {
    fn default() -> Self {
        Pricing::new()
    }
}
