//! Reporting layer — aggregates invoices into monthly figures and exports them.
use crate::invoice::render_invoice;

pub fn monthly_report(orders: &[Vec<&str>]) -> u64 {
    orders.iter().map(|o| render_invoice(o)).sum()
}

pub fn export_csv(orders: &[Vec<&str>]) -> String {
    let total = monthly_report(orders);
    format!("month,total\n2026-06,{total}\n")
}
