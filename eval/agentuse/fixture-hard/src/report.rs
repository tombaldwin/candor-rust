//! Batch reporting (the big, obvious branch).
use crate::orders::order_total;
pub fn daily_rollup(orders: &[Vec<u64>]) -> u64 { orders.iter().map(|o| order_total(o)).sum() }
pub fn monthly_rollup(days: &[Vec<Vec<u64>>]) -> u64 { days.iter().map(|d| daily_rollup(d)).sum() }
pub fn export_pdf(days: &[Vec<Vec<u64>>]) -> String { format!("total={}", monthly_rollup(days)) }
