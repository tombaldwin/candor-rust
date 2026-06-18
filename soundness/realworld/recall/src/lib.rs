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

// --- Env: std::env reads the process environment; etcetera reads $HOME/$XDG (calibrated 2026-06-18) ---
pub fn env_std() { let _ = std::env::var("PATH"); }
pub fn env_vars() { for _ in std::env::vars() {} }
pub fn env_etcetera() { let _ = etcetera::home_dir(); }

// --- Clock: std SystemTime/Instant read the wall/monotonic clock; jiff::now (calibrated 2026-06-18) ---
pub fn clock_std() { let _ = std::time::SystemTime::now(); }
pub fn clock_instant() { let _ = std::time::Instant::now(); }
pub fn clock_jiff() { let _ = jiff::Timestamp::now(); }

// --- Seam propagation (the deferred/indirect classes found in the 2026-06-18 adversarial sweep). An
// effect reached only THROUGH the seam must propagate to the forcing/writing fn — not stay on the seam's
// own body. Routed through Clock (SystemTime::now) to keep this corpus non-syscall + runs-anywhere. These
// gate the SCAN engine via known semantics, the ground-truth complement to the deep engine's ui/ fixtures
// (lazy-init: candor-rust ui/deferred_effects.rs; thread_local: ui/thread_local_effects.rs; write-fmt
// writer side: ui/write_trait.rs + scan 0.5.18). A silent-pure on any forcing fn is an under-report. ---
static SEAM_LAZY: std::sync::LazyLock<u8> = std::sync::LazyLock::new(|| { let _ = std::time::SystemTime::now(); 0 });
pub fn seam_lazy_force() { let _ = *SEAM_LAZY; }                                  // Clock (forces the lazy init)

thread_local! { static SEAM_TL: u8 = { let _ = std::time::SystemTime::now(); 0 }; }
pub fn seam_thread_local() { SEAM_TL.with(|v| { let _ = v; }); }                 // Clock (forces the thread_local init)

struct ClockWriter;
impl std::fmt::Write for ClockWriter {
    fn write_str(&mut self, _s: &str) -> std::fmt::Result { let _ = std::time::SystemTime::now(); Ok(()) }
}
pub fn seam_write_fmt(w: &mut ClockWriter) { use std::fmt::Write as _; let _ = write!(w, "x"); } // Clock (write! drives the writer)
