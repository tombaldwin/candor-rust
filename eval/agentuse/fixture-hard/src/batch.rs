use crate::report::export_pdf;
pub fn batch_export(days: &[Vec<Vec<u64>>]) -> String { export_pdf(days) }
pub fn nightly_job(days: &[Vec<Vec<u64>>]) -> String { batch_export(days) }
