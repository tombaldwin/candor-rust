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

// --- Rand: getrandom is end-to-end; `rand` is verb-split (thread_rng() builder + .gen() draw) ---
pub fn rand_random() { let _: u64 = rand::random(); }
pub fn rand_thread_rng() { use rand::Rng; let _: u64 = rand::thread_rng().gen(); }
pub fn rand_getrandom(b: &mut [u8]) { let _ = getrandom::getrandom(b); }
pub fn rand_uuid() { let _ = uuid::Uuid::new_v4(); }            // uncalibrated → expect DISCLOSED (honest)

// --- Clipboard: arboard's Clipboard::new() opens the connection (itself Clipboard) + set_text verb ---
pub fn clip_arboard() { if let Ok(mut c) = arboard::Clipboard::new() { let _ = c.set_text("m".into()); } }

// --- Ipc: std unix sockets are Ipc; interprocess is uncalibrated → expect DISCLOSED ---
pub fn ipc_unix() { let _ = std::os::unix::net::UnixStream::connect("/tmp/candor-ipc"); }
pub fn ipc_interprocess() { let _ = interprocess::local_socket::LocalSocketStream::connect("/tmp/x"); }
