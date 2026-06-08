//! The tax domain — pure arithmetic. The target of this change.
pub fn apply_tax(amount: u64) -> u64 {
    amount + rate(amount)
}
fn rate(amount: u64) -> u64 { amount / 5 }
