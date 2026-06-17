// One fn per (real crate, KNOWN effect). candor-scan must predict the effect OR disclose uncertainty
// (Unknown/blind/invisible); a silent-pure is an under-report. See recall/expected.json.

// --- Log: candor-classify keys log/tracing on the emit macro (now classified). slog is the probe. ---
pub fn log_tracing() { tracing::info!("m"); }
pub fn log_log() { log::warn!("m"); }
pub fn log_slog(l: &slog::Logger) { slog::info!(l, "m"); }

// --- Db: rusqlite opens directly; sqlx/diesel/postgres are BUILDER-CHAINs whose Db is on a verb ---
pub fn db_rusqlite() { let _ = rusqlite::Connection::open("x.db"); }
pub fn db_sqlx(p: &sqlx::PgPool) { let _ = sqlx::query("SELECT 1").execute(p); }
pub fn db_diesel(c: &mut diesel::PgConnection) { use diesel::RunQueryDsl; let _ = diesel::sql_query("SELECT 1").execute(c); }
pub fn db_postgres(c: &mut postgres::Client) { let _ = c.query("SELECT 1", &[]); }
