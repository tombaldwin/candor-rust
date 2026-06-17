//! candor-classify — the curated effect classifier (crate+path -> effect), extracted to a STABLE
//! crate so both the nightly `rustc_private` lint AND a stable backend share ONE source of truth
//! (no drift). Pure string logic; no rustc internals. The effect vocabulary lives in candor-report.

use candor_report::EFFECTS;

/// The canonical CANDOR_POLICY DSL parser (SPEC §6.2), shared by the nightly gate and candor-query.
pub mod policy;

/// Project-supplied rules, consulted only when the built-in `classify` returns None.
pub fn classify_extra(
    crate_name: &str,
    path: &str,
    extra: &[(&'static str, bool, String)],
) -> Option<&'static str> {
    for (eff, is_crate, prefix) in extra {
        let hit = if *is_crate { crate_name.starts_with(prefix.as_str()) } else { path.starts_with(prefix.as_str()) };
        if hit {
            return Some(eff);
        }
    }
    None
}

/// The exact third-party crates `classify` has effect rules for, and the crate-name
/// PREFIXES it recognizes. This is the single source of truth for "what candor knows":
/// it is emitted beside the JSON report (`<prefix>.calibrated.json`) so the Claude Code
/// receipt's coverage check reads candor's real coverage instead of a hand-copied list.
/// Keep in lockstep with `classify` below — the `db_crates_are_calibrated` and
/// `calibrated_crates_are_live` tests (in this crate's `tests` module) enforce both directions.
pub const CALIBRATED_CRATES: [&str; 59] = [
    // network (aws_config resolves credentials over the network on `.load()`;
    // git2 remote ops — fetch/push/connect — contact the network; async_net is smol's net layer;
    // pnet is raw L2/L3 packet capture)
    "reqwest", "isahc", "ureq", "curl", "aws_config", "git2", "tokio_tcp", "tokio_udp", "async_net",
    "async_nats", "lapin", "lettre", "tungstenite", "elasticsearch", "tonic", "rdkafka", "pnet",
    // directory traversal (ignore = gitignore-aware walker, powers ripgrep/fd; its walk executors are Fs)
    // + filesystem watching (notify = inotify/FSEvents/kqueue wrapper; powers watchexec/cargo-watch)
    "ignore", "notify",
    // database (see DB_CRATES in classify)
    "sqlx", "rusqlite", "postgres", "tokio_postgres", "diesel", "redis", "mongodb",
    "mysql", "mysql_async", "sea_orm", "deadpool_postgres",
    // filesystem (async_fs = smol; fs_err = std::fs wrapper; tempfile; glob) / entropy /
    // subprocess (async_process = smol; duct) / env (dotenvy/dotenv) / clock (time) / log / clipboard
    "memmap2", "fs_err", "async_fs", "tempfile", "glob",
    "rand", "getrandom", "fastrand",
    // entropy: the password-hashing tier (salt mints + bcrypt's internal salt) + the OsRng source
    "argon2", "bcrypt", "scrypt", "pbkdf2", "password_hash", "rand_core",
    "portable_pty", "async_process", "duct",
    "dotenvy", "dotenv",
    "chrono", "time", "tracing", "log", "arboard",
    // compiler diagnostic emission (a dylint lint's output) — see the Log rules in classify
    "rustc_lint", "rustc_errors",
    // raw syscalls via FFI — the syscall-name table that lights up the FFI-thin tier (nix is routed
    // through the same table by leaf name, so a consumer of nix is covered without nix's own source)
    "libc", "nix", "rustix",
];

pub const CALIBRATED_PREFIXES: [&str; 3] = ["aws_sdk_", "aws_smithy", "cap_"];

/// Crates `classify` matches by PATH prefix rather than crate-name equality (their effectful modules
/// are recognised, e.g. `tokio::net::`/`async_std::fs::`/`mio::net::`), so they're absent from
/// `CALIBRATED_CRATES` (which the liveness test probes by crate name). The coverage check must still
/// treat them as *covered* — otherwise it would mislabel the most common async crates as blind spots.
pub const PATH_CALIBRATED_CRATES: [&str; 3] = ["tokio", "async_std", "mio"];

/// Representative path tails (each appended to a crate name) that the `calibrated_crates_are_live`
/// liveness test probes: at least one must match for every `CALIBRATED_CRATES` entry, else the entry is
/// dead. Exported as ONE source of truth because the nightly lint crate (`src/lib.rs`) runs the SAME
/// liveness test — when the two probe lists were duplicated they drifted, and a rule keyed on a
/// distinctive tail (pnet `::datalink::channel`, ignore `::WalkBuilder::build_parallel`, notify
/// `::RecommendedWatcher::new`) added to only one list silently broke the other crate's `cargo test`.
pub const CALIBRATION_PROBE_TAILS: &[&str] = &[
    "::X::send", "::X::execute", "::X::call", "::X::query", "::X::fetch_one", "::Remote::fetch",
    "::datalink::channel", "::WalkBuilder::build_parallel", "::RecommendedWatcher::new",
    "::X::connect", "::Utc::now", "::X::load", "::__private_api::log", "::tempfile", "::glob",
    "::X::run", "::dotenv", "::random", "::emit", "::X::emit_span_lint", "::X::anything",
    "::SaltString::generate", "::hash", "::OsRng::fill_bytes",
    // verb-precise crates whose whole-crate rules were narrowed to the effectful surface (the pure
    // accessors/ctors/data-types now return None), so the liveness probe must name an EFFECTFUL path:
    "::Mmap::map", "::event", "::u32", "::Clipboard::get_text", "::spawn_command",
];

/// Database client crates whose execution verbs are I/O (see the DB branch in `classify`).
/// Module-level so `db_crates_are_calibrated` can enforce `DB_CRATES ⊆ CALIBRATED_CRATES`.
pub const DB_CRATES: [&str; 11] = [
    "sqlx", "rusqlite", "postgres", "tokio_postgres", "diesel", "redis", "mongodb",
    "mysql", "mysql_async", "sea_orm", "deadpool_postgres",
];

/// Pure file-descriptor *ownership-transfer* leaves. These ADOPT an already-open descriptor
/// (`from_raw_fd`/`from_raw_socket`/`from_raw_handle`), EXTRACT/BORROW one
/// (`into_raw_fd`/`into_raw_socket`/`into_raw_handle`, `as_raw_fd`/`as_raw_socket`/`as_raw_handle`),
/// or UNWRAP an async wrapper back to its std type (`into_std`) — none of them issue a syscall or
/// perform I/O. candor's cardinal sin is calling a PURE function effectful, and these collide with the
/// coarse std-type PREFIX rules (`std::net::TcpStream`/`std::fs::File`/`std::os::unix::net` → Net/Fs/Ipc)
/// even though the descriptor was opened ELSEWHERE. The portable_pty/async_process Exec rule already
/// exempts `from_raw_fd`; this generalises the same carve-out across the net/fs/ipc prefix rules.
/// (Found by a real-world sweep of tokio: `TcpStream::into_std`, `*::from_raw_fd`, `*::as_raw_fd` all
/// fabricated Net/Fs/Ipc.)
const PURE_FD_TRANSFER: &[&str] = &[
    "from_raw_fd", "from_raw_socket", "from_raw_handle",
    "into_raw_fd", "into_raw_socket", "into_raw_handle",
    "as_raw_fd", "as_raw_socket", "as_raw_handle",
    "into_std",
    // `SocketAddr::from_pathname` (std/async-std unix net) builds an address STRUCT from a path —
    // it opens no socket. The `std::os::unix::net` prefix rule below would otherwise fabricate Ipc
    // on it. (Found sweeping socket2: `SockAddr::as_unix` → `from_pathname` reported Ipc.)
    "from_pathname",
];

/// Classify a resolved callee by the crate it belongs to and its full path.
pub fn classify(crate_name: &str, path: &str) -> Option<&'static str> {
    // Pure fd ownership-transfer/extraction leaves are never an effect, regardless of which std I/O
    // type they hang off — exempt them BEFORE the coarse prefix rules can fabricate Net/Fs/Ipc.
    if PURE_FD_TRANSFER.contains(&path.rsplit("::").next().unwrap_or(path)) {
        return None;
    }
    if crate_name.starts_with("aws_sdk_") || crate_name.starts_with("aws_smithy") {
        // Only request dispatch is network I/O; builder setters/accessors are pure.
        if path.ends_with("::send") || path.ends_with("::send_with") {
            return Some("Net");
        }
        return None;
    }
    // aws-config resolves credentials/region on `.load()` — it reaches the IMDS metadata
    // endpoint / STS over the network (and reads ~/.aws + env). Builders (`defaults()`,
    // `SdkConfig::builder()`, `BehaviorVersion::latest()`) are pure; the `load` is the I/O.
    // (Found hardening on a real app, ebman: `builder.load().await` was classified pure.)
    if crate_name == "aws_config" {
        if path.ends_with("::load") || path.ends_with("::load_defaults") {
            return Some("Net");
        }
        return None;
    }
    // git2 (libgit2 FFI): remote operations contact the network; everything else is local
    // to the .git directory. Match the remote verbs precisely — NOT bare `::clone`, which is
    // the `Clone`-trait dup of a `Remote` handle (pure), not `Repository::clone`. (Found
    // hardening on gitui: `remote.fetch`/`remote.push` were classified network-free — a git
    // client reporting it makes no network calls.)
    if crate_name == "git2" {
        if path.ends_with("::fetch")
            || path.ends_with("::push")
            || path.ends_with("::download")
            || path.ends_with("::connect")
            || path.ends_with("::connect_auth")
            || path.ends_with("::ls")
            || path.ends_with("::upload")
        {
            return Some("Net");
        }
        return None;
    }
    // libc — raw syscalls via FFI. The FFI-thin tier (nix, and the syscall layer beneath rusqlite/git2)
    // is invisible to a name classifier unless we model libc directly: a 35-crate calibration
    // (eval/calibration) showed nix reporting ZERO library effects because every wrapper bottoms out in
    // an unrecognised `libc::*` call. Classify by syscall name, but ONLY the UNAMBIGUOUS ones — the
    // socket family is Net, path/dir syscalls are Fs, spawn/exec/wait is Exec, SysV/pipe IPC is Ipc,
    // env/clock/entropy each their own. We deliberately SKIP the generic file-descriptor ops
    // (read/write/close/lseek/dup/fcntl/ioctl/poll/select/epoll*/mmap): they operate on ANY fd — file,
    // socket, or pipe — so a fixed label would mis-categorise as often as it helps. An honest
    // no-classify (under-report) beats emitting the WRONG effect. Pure conversions (htons/inet_pton/
    // gmtime) are also skipped.
    //
    // `nix` (the idiomatic SAFE libc wrapper, in ~every Rust systems/CLI crate) is routed through the
    // SAME table: its functions keep the syscall leaf name (`nix::fcntl::open`, `nix::sys::socket::connect`,
    // `nix::unistd::execvp`). Without this, a CONSUMER of nix analysed without nix's own source (the
    // stable scanner, single-crate) sees `nix::*` cross-crate and under-reports — serialport-rs opens its
    // device via `nix::fcntl::open` and reported ZERO Fs. The nightly lint reaches `libc::*` THROUGH nix's
    // body; this gives the scanner the same coverage directly. (Found sweeping serialport-rs.)
    // `rustix` is the same shape as nix but does RAW syscalls (no libc underneath), so its functions MUST
    // be classified directly. Its leaf names are the syscall names too (`rustix::time::clock_settime`,
    // `rustix::fs::mkfifoat`/`symlink`/`stat`, `rustix::net::connect`) — route it through the same table.
    // The rustix-specific `*at`/variant leaves it doesn't share with libc just under-report (the safe
    // direction). VALIDATED, not speculative: coreutils' `date` reads/sets the clock via
    // `rustix::time::clock_getres`/`clock_settime` and reported Clock=0; the file I/O that goes through
    // std::fs was already correct, which is why only the rustix-only effects (Clock/Ipc) were missing.
    if crate_name == "libc" || crate_name == "nix" || crate_name == "rustix" {
        let f = path.rsplit("::").next().unwrap_or(path);
        // path / directory / metadata syscalls (incl. *64 and *at variants)
        const FS: &[&str] = &[
            "open", "open64", "openat", "openat2", "creat", "creat64", "stat", "stat64", "lstat",
            "lstat64", "fstatat", "fstatat64", "newfstatat", "statx", "access", "faccessat",
            "faccessat2", "mkdir", "mkdirat", "rmdir", "unlink", "unlinkat", "rename", "renameat",
            "renameat2", "link", "linkat", "symlink", "symlinkat", "readlink", "readlinkat", "chmod",
            "fchmodat", "chown", "lchown", "fchownat", "truncate", "truncate64", "ftruncate",
            "ftruncate64", "opendir", "fdopendir", "readdir", "readdir64", "readdir_r", "closedir",
            "rewinddir", "seekdir", "telldir", "scandir", "mkstemp", "mkstemps", "mkostemp", "mkdtemp",
            "mknod", "mknodat", "chdir", "fchdir", "getcwd", "get_current_dir_name", "chroot",
            "pivot_root", "statfs", "statfs64", "fstatfs", "fstatfs64", "statvfs", "fstatvfs", "mount",
            "umount", "umount2", "fsync", "fdatasync", "sync", "syncfs", "sync_file_range", "fallocate",
            "posix_fallocate", "posix_fadvise", "sendfile", "sendfile64", "copy_file_range", "flock",
            "getdents", "getdents64", "utime", "utimes", "lutimes", "futimens", "utimensat", "futimesat",
            "realpath",
        ];
        // socket family — these operate only on sockets, so Net is unambiguous (AF_UNIX domain isn't
        // visible at the call, so a Unix socket reads as Net rather than Ipc; acceptable over-general).
        const NET: &[&str] = &[
            "socket", "setsockopt", "getsockopt", "bind", "listen", "accept", "accept4", "connect",
            "shutdown", "send", "sendto", "sendmsg", "sendmmsg", "recv", "recvfrom", "recvmsg",
            "recvmmsg", "getpeername", "getsockname", "getaddrinfo", "freeaddrinfo", "getnameinfo",
        ];
        // process creation / replacement / reaping
        const EXEC: &[&str] = &[
            "fork", "vfork", "clone", "clone3", "execl", "execlp", "execle", "execv", "execvp",
            "execvpe", "execve", "execveat", "fexecve", "posix_spawn", "posix_spawnp", "system",
            "popen", "pclose", "wait", "waitpid", "wait3", "wait4", "waitid",
        ];
        // pipes / FIFOs / SysV + POSIX message queues, semaphores, shared memory; socketpair (AF_UNIX)
        const IPC: &[&str] = &[
            "pipe", "pipe2", "mkfifo", "mkfifoat", "socketpair", "msgget", "msgsnd", "msgrcv", "msgctl",
            "semget", "semop", "semtimedop", "semctl", "shmget", "shmat", "shmdt", "shmctl", "mq_open",
            "mq_send", "mq_receive", "mq_timedsend", "mq_timedreceive", "mq_close", "mq_unlink",
        ];
        const ENV: &[&str] = &["getenv", "secure_getenv", "setenv", "putenv", "unsetenv", "clearenv"];
        const CLOCK: &[&str] = &[
            "time", "gettimeofday", "clock_gettime", "clock_getres", "nanosleep", "clock_nanosleep",
            // SETTING the system clock is a clock effect too (was unclassified — found on coreutils `date`,
            // which sets it via `clock_settime`).
            "clock_settime", "settimeofday", "stime", "adjtime", "adjtimex", "clock_adjtime",
        ];
        const RAND: &[&str] = &["getrandom", "getentropy", "arc4random", "arc4random_buf", "arc4random_uniform"];
        if FS.contains(&f) {
            return Some("Fs");
        }
        if NET.contains(&f) {
            return Some("Net");
        }
        if EXEC.contains(&f) {
            return Some("Exec");
        }
        if IPC.contains(&f) {
            return Some("Ipc");
        }
        if ENV.contains(&f) {
            return Some("Env");
        }
        if CLOCK.contains(&f) {
            return Some("Clock");
        }
        if RAND.contains(&f) {
            return Some("Rand");
        }
        return None;
    }
    // C-library FFI bindings: libsqlite3 (under rusqlite) and libgit2 (under git2). Like the libc tier,
    // these crates are thin Rust over a C library, so their real I/O is invisible until the C entry
    // points are named. Match by the DISTINCTIVE C function name (`sqlite3_*` / `git_*`) via the call's
    // LEAF — independent of the binding crate's alias: rusqlite calls `ffi::sqlite3_step`, git2 calls
    // `raw::git_remote_fetch`, and the nightly lint resolves the same to `libsqlite3_sys`/`libgit2_sys`;
    // all spellings share the leaf. Only the I/O-performing entry points are listed — the in-memory
    // accessors (`sqlite3_bind_*`/`sqlite3_column_*`, `git_*_oid`/strarray/options builders) stay pure,
    // so a non-listed `sqlite3_`/`git_` leaf returns None (under-report, never a wrong effect). Calibrated
    // + validated against rusqlite 0.39 / git2 0.20 source (eval/calibration).
    {
        let leaf = path.rsplit("::").next().unwrap_or(path);
        if let Some(rest) = leaf.strip_prefix("sqlite3_") {
            let _ = rest;
            // SQLite C API operations that touch the database (open/exec/step/prepare/backup/blob/wal).
            const DB: &[&str] = &[
                "sqlite3_open", "sqlite3_open_v2", "sqlite3_open16", "sqlite3_close", "sqlite3_close_v2",
                "sqlite3_exec", "sqlite3_step", "sqlite3_prepare", "sqlite3_prepare_v2",
                "sqlite3_prepare_v3", "sqlite3_prepare16", "sqlite3_prepare16_v2", "sqlite3_prepare16_v3",
                "sqlite3_get_table", "sqlite3_backup_init", "sqlite3_backup_step", "sqlite3_backup_finish",
                "sqlite3_blob_open", "sqlite3_blob_read", "sqlite3_blob_write", "sqlite3_blob_reopen",
                "sqlite3_load_extension", "sqlite3_wal_checkpoint", "sqlite3_wal_checkpoint_v2",
            ];
            return DB.contains(&leaf).then_some("Db");
        }
        if leaf.starts_with("git_") {
            // libgit2: remote/transport operations contact the network … (incl. submodule clone/update,
            // which `git_clone`/fetch the subrepo over its remote — `allow_fetch` defaults on; an A/B on
            // git2 0.20 caught `Submodule::update`/`clone` reporting no `Net`).
            const NET: &[&str] = &[
                "git_clone", "git_remote_connect", "git_remote_connect_ext", "git_remote_fetch",
                "git_remote_download", "git_remote_upload", "git_remote_push", "git_remote_ls",
                "git_submodule_clone", "git_submodule_update",
            ];
            // … and repository/index/odb/checkout/ref/config operations touch the on-disk .git store.
            const FS: &[&str] = &[
                "git_repository_open", "git_repository_open_ext", "git_repository_open_bare",
                "git_repository_init", "git_repository_init_ext", "git_repository_discover",
                "git_checkout_tree", "git_checkout_head", "git_checkout_index", "git_index_read",
                "git_index_write", "git_index_write_tree", "git_index_write_tree_to",
                "git_index_add_bypath", "git_index_add_all", "git_odb_open", "git_odb_read",
                "git_odb_write", "git_odb_open_wstream", "git_odb_open_rstream",
                "git_blob_create_fromdisk", "git_blob_create_fromworkdir", "git_blob_create_from_disk",
                "git_blob_create_from_workdir", "git_blob_create_from_stream", "git_commit_create",
                "git_commit_create_v", "git_reference_create", "git_reference_set_target",
                "git_reference_delete", "git_config_open_default", "git_config_open_ondisk",
                "git_config_add_file_ondisk", "git_tag_create", "git_treebuilder_write",
                "git_packbuilder_write",
            ];
            if NET.contains(&leaf) {
                return Some("Net");
            }
            if FS.contains(&leaf) {
                return Some("Fs");
            }
            return None;
        }
        if leaf.starts_with("curl_") {
            // libcurl (under the `curl` crate, called `curl_sys::curl_*`). Only the entry points that
            // PERFORM network I/O: the blocking transfer (`curl_easy_perform`), raw socket send/recv,
            // the HTTP/2 keepalive PING (`upkeep`), and the multi-interface transfer pumps. The large
            // pure surface (setopt/init/cleanup/reset/getinfo/escape/multi_add_handle/fdset/info_read)
            // stays unclassified, as do `curl_multi_wait`/`poll` (readiness WAIT on sockets, no payload —
            // the loop's `perform` is the tagged boundary, per the I/O-boundary principle). An A/B on
            // curl 0.4 caught the whole crate reporting ZERO Net (`Easy::perform` read as pure).
            const NET: &[&str] = &[
                "curl_easy_perform", "curl_easy_send", "curl_easy_recv", "curl_easy_upkeep",
                "curl_multi_perform", "curl_multi_socket_action",
            ];
            return NET.contains(&leaf).then_some("Net");
        }
        if let Some(op) = leaf.strip_prefix("SSL_") {
            // OpenSSL (libssl, under the `openssl`/`native-tls` crates, called `ffi::SSL_*`). The TLS
            // handshake and record I/O run over the peer socket -> Net. Unlike libc read/write, an SSL_*
            // op is ~always over a network BIO (the rare memory-BIO/sans-IO case is the honest exception
            // we accept). The crypto surface (EVP_*/SHA*/AES*) and pure setup (SSL_CTX_new/SSL_set_fd) are
            // NOT here; `BIO_*` is skipped (a BIO may be memory or socket). Validated vs openssl 0.9 source.
            const SSL_NET: &[&str] = &[
                "connect", "accept", "do_handshake", "read", "read_ex", "write", "write_ex", "peek",
                "peek_ex", "shutdown",
            ];
            return SSL_NET.contains(&op).then_some("Net");
        }
    }
    // HTTP clients use the same builder pattern as the AWS SDK: only the dispatch is
    // I/O. (Found by the eval: ebman's reqwest calls to the Anthropic API + webhooks
    // were silently classified network-free because reqwest wasn't recognized.)
    if crate_name == "reqwest" || crate_name == "isahc" {
        // The builder chain is pure; the dispatch (`::send`/`::execute`) is the I/O. PLUS the one-shot
        // CONVENIENCE functions `reqwest::get` / `reqwest::blocking::get` / `isahc::get`, which send
        // immediately — they're not the `Client::get` builder (a different path, `reqwest::Client::get`),
        // so an exact match avoids false-positiving the builder. (Found running on `xh`: a one-shot
        // `reqwest::get(url)` was classified network-free.)
        if path.ends_with("::send")
            || path.ends_with("::execute")
            || path == "reqwest::get"
            || path == "reqwest::blocking::get"
            || path == "isahc::get"
        {
            return Some("Net");
        }
        return None;
    }
    if crate_name == "ureq" && path.ends_with("::call") {
        return Some("Net");
    }
    // The `curl` crate (libcurl's safe binding — cargo's own HTTP client): the dispatch verbs are
    // `perform` (Easy/Easy2/Transfer/Multi), raw-socket `send`/`recv`, the keepalive `upkeep`, and the
    // multi-interface `action` (socket_action). The big setopt-style builder surface stays pure.
    // `Multi::timeout` is deliberately NOT matched: `Easy::timeout` is a pure CURLOPT_TIMEOUT setter
    // sharing the leaf — an under-report on the rare event-loop kick beats mis-tagging every consumer
    // that sets a timeout. (Consumer-side companion to the curl_* FFI tier, same A/B finding.)
    if crate_name == "curl"
        && (path.ends_with("::perform")
            || path.ends_with("::send")
            || path.ends_with("::recv")
            || path.ends_with("::upkeep")
            || path.ends_with("::action"))
    {
        return Some("Net");
    }
    // The modern async-HTTP / TLS / QUIC / DNS stack — the LAYER reqwest/ureq/isahc build on, and that
    // crates use DIRECTLY. Found by the independent-method differential on `oha` (2026-06-17): candor
    // honestly DISCLOSED these as blind but never CLASSIFIED them, leaving real Net reaches uncovered.
    // Verb-keyed (the pure type/builder/codec surface stays None) and CRATE-GATED, so generic verbs
    // (request/connect/get/read/write/accept) never fabricate across unrelated crates. Same precision
    // discipline as the reqwest/curl rules above; complements the scan_builder_entry_effect entries.
    match crate_name {
        // hyper 1.x client connection I/O (the builder/Body/Request types stay pure).
        "hyper" if path.ends_with("::send_request") || path.ends_with("::handshake") => return Some("Net"),
        // hyper-util's pooled legacy Client + its TCP connectors.
        "hyper_util" if path.ends_with("::request") || path.ends_with("::connect") => return Some("Net"),
        // hickory (trust-dns) resolver — issues DNS queries over the network.
        "hickory_resolver"
            if path.ends_with("::lookup_ip") || path.ends_with("::lookup") || path.ends_with("_lookup")
                || path.ends_with("::resolve") => return Some("Net"),
        // HTTP/3 over QUIC.
        "h3" if path.ends_with("::send_request") || path.ends_with("::recv_data")
            || path.ends_with("::recv_response") || path.ends_with("::send_data") => return Some("Net"),
        // QUIC transport (UDP socket send/recv).
        "quinn" if path.ends_with("::connect") || path.ends_with("::accept") || path.ends_with("::open_bi")
            || path.ends_with("::open_uni") || path.ends_with("::accept_bi") || path.ends_with("::accept_uni")
            || path.ends_with("::send_datagram") || path.ends_with("::read_datagram") => return Some("Net"),
        // TLS-over-TCP stream adapters — the actual socket handshake/I/O (the config/cert types stay pure).
        "tokio_rustls" | "native_tls"
            if path.ends_with("::connect") || path.ends_with("::accept") || path.ends_with("::handshake") =>
            return Some("Net"),
        // AF_VSOCK host<->guest sockets — inter-process / VM comms.
        "tokio_vsock" if path.ends_with("::connect") || path.ends_with("::bind") || path.ends_with("::accept") =>
            return Some("Ipc"),
        // Loads the OS trust store from disk (cert files / keychain).
        "rustls_native_certs" if path.ends_with("::load_native_certs") => return Some("Fs"),
        // Reads host/process config from the OS (CPU count, cgroup quota; resource limits).
        "num_cpus" if path.ends_with("::get") || path.ends_with("::get_physical") => return Some("Env"),
        "rlimit" if path.ends_with("::getrlimit") || path.ends_with("::setrlimit")
            || path.ends_with("::increase_nofile_limit") => return Some("Env"),
        _ => {}
    }
    // Message-queue clients fully encapsulate the socket (the underlying tokio::net lives
    // inside the crate, unseen), so a user's connect/publish/consume calls ARE the I/O
    // boundary — to a remote broker, hence Net. Match the broker round-trip verbs (snake_case
    // methods); the CamelCase option/property builders stay pure. (Found hardening on consumer
    // apps: lapin `basic_publish`/`queue_declare` and async-nats `publish`/`subscribe` were
    // classified pure — a message-queue client reporting no I/O.)
    if crate_name == "async_nats" {
        if path.ends_with("::connect")
            || path.contains("::publish")
            || path.ends_with("::subscribe")
            || path.ends_with("::queue_subscribe")
            || path.contains("::request")
            || path.ends_with("::flush")
        {
            return Some("Net");
        }
        return None;
    }
    if crate_name == "lapin" {
        if path.ends_with("::connect")
            || path.ends_with("::create_channel")
            || path.contains("::basic_")
            || path.contains("::queue_")
            || path.contains("::exchange_")
            || path.contains("::tx_")
            || path.ends_with("::confirm_select")
            || path.ends_with("::close")
        {
            return Some("Net");
        }
        return None;
    }
    // SMTP email — lettre's `Transport::send` is the network dispatch; Message building is
    // pure. (Found hardening on a lettre consumer: `mailer.send(&email)` classified pure.)
    if crate_name == "lettre" {
        if path.ends_with("::send") || path.ends_with("::send_raw") {
            return Some("Net");
        }
        return None;
    }
    // WebSockets — tungstenite (the modern successor to the old `websocket` crate). connect
    // and the socket read/write/send are network; Message constructors are pure. (Found on a
    // tungstenite consumer: connect + send + read classified pure.)
    if crate_name == "tungstenite" {
        if path.ends_with("::connect")
            || path.ends_with("::read")
            || path.ends_with("::write")
            || path.ends_with("::send")
            || path.ends_with("::close")
            || path.ends_with("::flush")
            || path.ends_with("::read_message")
            || path.ends_with("::write_message")
        {
            return Some("Net");
        }
        return None;
    }
    // elasticsearch: request builders are pure; only the `.send()` dispatch is HTTP I/O
    // (same shape as reqwest / the AWS SDK). (Found on an elasticsearch consumer.)
    if crate_name == "elasticsearch" && path.ends_with("::send") {
        return Some("Net");
    }
    // gRPC — tonic. The transport connect and the Grpc client RPC dispatch are network;
    // codecs and request/response wrappers are pure. (connect repro-confirmed on a consumer;
    // the unary/streaming RPC verbs are from the tonic::client::Grpc API.)
    if crate_name == "tonic" {
        if path.ends_with("::connect")
            || path.ends_with("::unary")
            || path.ends_with("::server_streaming")
            || path.ends_with("::client_streaming")
            || path.ends_with("::streaming")
        {
            return Some("Net");
        }
        return None;
    }
    // Kafka — rdkafka (FFI to librdkafka). Producer send + consumer poll/recv/subscribe/
    // commit are network round-trips to the brokers. (API-calibrated + unit-tested; a real
    // repro needs librdkafka/cmake, deferred.)
    if crate_name == "rdkafka" {
        if path.ends_with("::send")
            || path.ends_with("::send_result")
            || path.ends_with("::recv")
            || path.ends_with("::poll")
            || path.ends_with("::subscribe")
            || path.ends_with("::commit")
            || path.ends_with("::commit_message")
            || path.ends_with("::commit_consumer_state")
            || path.ends_with("::store_offset")
            || path.ends_with("::seek")
            || path.ends_with("::fetch_metadata")
            || path.ends_with("::fetch_watermarks")
            || path.ends_with("::flush")
        {
            return Some("Net");
        }
        return None;
    }
    // cap-std: capability-oriented std. I/O goes *through* a held capability handle
    // (Dir/Pool/Clock/...), so these calls ARE the effect. Recognising them means a
    // cap-std project's real I/O is detected and matches the capability it declared
    // (via `declared_caps`/`capstd_cap`) — conformance against unforgeable capabilities.
    if crate_name.starts_with("cap_") {
        if path.contains("::net::Unix") || path.contains("::os::") {
            return Some("Ipc");
        }
        if path.contains("::net") {
            return Some("Net");
        }
        if path.contains("::time") {
            return Some("Clock");
        }
        if path.contains("::fs") || crate_name == "cap_tempfile" || crate_name == "cap_directories" {
            return Some("Fs");
        }
        return None;
    }
    // Local IPC (Unix-domain sockets) is I/O but not *network* — keep it distinct so
    // CANDOR_NO_AMBIENT and audits don't conflate it with internet access. async-std puts its
    // Unix sockets under `os::unix::net` (mirroring std); async-net (smol's net layer) under
    // `unix`.
    if path.starts_with("tokio::net::Unix")
        || path.starts_with("std::os::unix::net")
        || path.starts_with("async_std::os::unix::net")
        || path.starts_with("async_net::unix")
    {
        return Some("Ipc");
    }
    // Raw packet capture / raw sockets — libpnet (the dominant low-level networking crate; powers
    // bandwhich, sniffers, custom-protocol tools). `datalink::channel` opens an L2 socket and
    // `transport::transport_channel` an L3/L4 raw socket — both ARE network I/O. Packet construction
    // (pnet_packet / pnet_base, MacAddr, Ethernet frames…) is pure and stays unclassified. The actual
    // frame read/write happens via methods on the returned Sender/Receiver (trait-object dispatch the
    // syntactic backend can't resolve), so the channel-open call is the precise Net boundary. (Found
    // scanning bandwhich — a packet sniffer — which reported Net 0.)
    if crate_name == "pnet" || crate_name == "pnet_datalink" || crate_name == "pnet_transport" {
        if path.ends_with("::channel") || path.ends_with("::transport_channel") {
            return Some("Net");
        }
        return None;
    }
    // Directory traversal — `ignore` (BurntSushi's gitignore-aware walker; powers ripgrep, fd). The walk
    // EXECUTORS read the directory tree from disk = Fs. Type-precise on purpose: the configuration builders
    // (`OverrideBuilder::build`, `GitignoreBuilder::build`, the `WalkBuilder` setters) and `DirEntry`
    // accessors are PURE — only `WalkBuilder::build`/`build_parallel` (which kick off the walk) and
    // `WalkParallel::run` (which drives it) touch the filesystem. A bare `build` would wrongly flag the
    // config builders. (Found scanning fd — a file finder — which reported Fs 2: its own `fs::read_dir`
    // was caught, but the `ignore`-based traversal that IS fd was invisible cross-crate.)
    if crate_name == "ignore" {
        if path == "ignore::WalkBuilder::build"
            || path == "ignore::WalkBuilder::build_parallel"
            || path.ends_with("::WalkParallel::run")
            // `add_ignore(path)` LOOKS like a config setter but reads that ignore file from disk at call
            // time (it returns the read error) — unlike the pure `add_custom_ignore_filename(name)` which
            // only stores a filename string. The lone Fs-touching builder method in the otherwise-pure setter
            // surface, so it was silently pure under the covered-crate floor.
            || path == "ignore::WalkBuilder::add_ignore"
        {
            return Some("Fs");
        }
        return None;
    }
    // Filesystem watching — `notify` (the de-facto fs-watch crate: watchexec, cargo-watch, mdbook). A
    // watcher opens an OS notification handle (inotify / FSEvents / kqueue / ReadDirectoryChanges) and
    // registers paths — observing filesystem state changes = Fs. The lifecycle boundary: any
    // `*Watcher::new` constructor (RecommendedWatcher/PollWatcher/INotifyWatcher/FsEventWatcher/…), the
    // `recommended_watcher` convenience fn, and the `watch`/`unwatch` registration verbs. `Config`/`Event`/
    // `EventKind` data types stay pure. (Found scanning watchexec: its watcher-`create` read Fs 0.)
    if crate_name == "notify" {
        if path.ends_with("Watcher::new")
            || path.ends_with("::recommended_watcher")
            || path.ends_with("::watch")
            || path.ends_with("::unwatch")
        {
            return Some("Fs");
        }
        return None;
    }
    // std DNS resolution — `("host", 80).to_socket_addrs()` / `std::net::lookup_host("host")` perform a
    // real getaddrinfo query (Net), but the classify table covered only the socket I/O *types*, so they
    // floored silently (sweep [37]; the syntactic engine modelled DNS only at the libc layer).
    if path.ends_with("::to_socket_addrs")
        || path == "std::net::lookup_host"
        || path.ends_with("ToSocketAddrs::to_socket_addrs")
    {
        return Some("Net");
    }
    // Raw sockets. Match the I/O *types* only — `std::net` also holds pure data types
    // (SocketAddr, IpAddr, …) whose construction must NOT be flagged.
    if path.starts_with("std::net::TcpStream")
        || path.starts_with("std::net::TcpListener")
        || path.starts_with("std::net::UdpSocket")
        || path.starts_with("tokio::net::")
    {
        // …but the PURE accessors read back local/option state — no network I/O — so the whole-type Net
        // rule fabricated Net on them (sweep [24], the cardinal sin; mirrors the arboard/memmap2 accessor
        // carve-outs). local_addr/peer_addr return bound/connected addresses; nodelay/ttl/take_error read
        // socket options/state. Every genuine verb (connect/read/write/send/recv/accept) stays Net.
        if path.ends_with("::local_addr")
            || path.ends_with("::peer_addr")
            || path.ends_with("::nodelay")
            || path.ends_with("::ttl")
            || path.ends_with("::take_error")
        {
            return None;
        }
        return Some("Net");
    }
    // Legacy tokio 0.1 socket crates — `tokio_tcp`/`tokio_udp` are *entirely* networking
    // (no pure types to over-flag), so the whole crate is Net. (Found hardening on websocat,
    // which is still on tokio 0.1: its `tokio_tcp::TcpStream::connect` was classified
    // network-free — a network tool confidently reporting 0 Net.)
    if matches!(crate_name, "tokio_tcp" | "tokio_udp") {
        return Some("Net");
    }
    // The other async runtimes mirror tokio's module layout, and their `net` modules hold only
    // socket I/O types (the pure `SocketAddr`/`IpAddr` are re-exports that resolve to `std::net`,
    // so they're excluded by def-path). `mio` is the low-level non-blocking-socket layer under
    // tokio/others; `async_net` is smol's net crate. Closes the async-std/smol/mio gap the
    // tokio_tcp note flagged. (Calibrated by module structure — these crates ARE networking — not
    // a live repro; the TCP/UDP types are defined in-crate so the def-path prefix is exact.)
    if path.starts_with("async_std::net::")
        || path.starts_with("mio::net::")
        || crate_name == "async_net"
    {
        return Some("Net");
    }
    // Database clients. Like the AWS/HTTP builders, only the execution verbs are I/O;
    // query *construction* is pure. Best-effort across crates (tune via CANDOR_CONFIG).
    // Note: bare `::query` is deliberately omitted — it executes in postgres/rusqlite but
    // only *builds* in sqlx, so including it would false-positive sqlx's `query()` builder.
    if DB_CRATES.contains(&crate_name) {
        // Postgres / SQLite-family clients: `query`/`batch_execute`/`prepare`/etc. ARE the
        // execution (round-trips to the server). sqlx is the outlier where bare `query()`
        // only BUILDS — it keeps the narrow set below. (Found by running on a real
        // tokio-postgres app, pgman: candor had reported only 4 of ~20 DB call sites.)
        if matches!(crate_name, "postgres" | "tokio_postgres" | "deadpool_postgres" | "rusqlite") {
            const PG: [&str; 19] = [
                "::query", "::query_one", "::query_opt", "::query_raw", "::execute",
                "::batch_execute", "::simple_query", "::prepare", "::prepare_typed",
                "::copy_in", "::copy_out", "::transaction", "::connect",
                // rusqlite's dialect of the same verbs (a verb-probe found the CANONICAL rusqlite
                // consumer API classifying pure): `query_row` is the one-row read, `query_map`/
                // `query_and_then` the many-row reads, `execute_batch` is rusqlite's name for
                // batch_execute, `prepare_cached` round-trips like prepare. `query_typed` is
                // tokio_postgres 0.7.10+.
                "::query_row", "::query_map", "::query_and_then", "::execute_batch",
                "::prepare_cached", "::query_typed",
            ];
            if PG.iter().any(|v| path.ends_with(v)) {
                return Some("Db");
            }
            // rusqlite only: opening the database IS the connection establishment (`Connection::
            // open`/`open_in_memory`/`open_with_flags` — the embedded analog of `::connect`).
            if crate_name == "rusqlite"
                && (path.ends_with("::open")
                    || path.ends_with("::open_in_memory")
                    || path.ends_with("::open_with_flags"))
            {
                return Some("Db");
            }
            return None;
        }
        // redis: the way redis is ACTUALLY used is the high-level `Commands`/`AsyncCommands`
        // traits (`con.get`/`set`/`hset`/`lpush`/…) — every method is a round-trip — plus
        // connection establishment. The shared VERBS below only catch the low-level
        // `cmd("GET").query(con)`, so without this a normal redis user's calls classify as
        // PURE. (Found hardening on redis-rs: a fn doing `con.get`/`set` reported no effects.)
        if crate_name == "redis"
            && (path.contains("Commands::")
                || path.contains("::get_connection")
                || path.contains("::get_async_connection")
                || path.contains("::get_multiplexed_async_connection")
                // a live `ConnectionManager` round-trips (Db), but `ConnectionManagerConfig` is a pure
                // in-memory builder (set_number_of_retries/set_max_delay) — exclude it (adversarial review).
                // `ConnectionManager::clone` is an Arc refcount bump — no Db round-trip (sweep [27]).
                || (path.contains("ConnectionManager") && !path.contains("ConnectionManagerConfig")
                    && !path.ends_with("::clone"))
                || path.ends_with("::query")
                || path.ends_with("::query_async")
                || path.ends_with("::req_command")
                || path.ends_with("::req_packed_command")
                || path.ends_with("::req_packed_commands"))
        {
            return Some("Db");
        }
        // mongodb: a document-store API with none of the SQL verbs — the user calls
        // `coll.find_one`/`insert_one`/`aggregate`/… and `Client::with_uri_str`. Without
        // these a mongodb user's calls classify PURE. (Found hardening: a fn doing
        // `find_one`+`insert_one` reported no effects.) Handle accessors (name/namespace)
        // and option/doc builders don't match these verbs, so they stay pure.
        if crate_name == "mongodb" {
            const MONGO: [&str; 27] = [
                "::with_uri_str", "::connect", "::find", "::find_one", "::insert_one",
                "::insert_many", "::update_one", "::update_many", "::delete_one",
                "::delete_many", "::replace_one", "::aggregate", "::count_documents",
                "::estimated_document_count", "::count", "::distinct", "::run_command",
                "::find_one_and_update", "::find_one_and_delete", "::find_one_and_replace",
                "::list_collections", "::list_collection_names", "::list_databases",
                "::list_database_names", "::create_collection", "::create_index", "::watch",
            ];
            if MONGO.iter().any(|v| path.ends_with(v)) {
                return Some("Db");
            }
            return None;
        }
        // mysql / mysql_async: the `query`/`exec` families + `get_conn`/`ping` execute
        // immediately — no build-then-execute split like sqlx, so matching `::query` is safe
        // here. Same DB-verb-dialect gap class as redis/mongodb; calibrated from the Queryable
        // API (unit-tested; a real-app repro is the remaining confirmation).
        if matches!(crate_name, "mysql" | "mysql_async") {
            const MY: [&str; 16] = [
                "::query", "::query_first", "::query_iter", "::query_map", "::query_fold",
                "::query_drop", "::exec", "::exec_first", "::exec_iter", "::exec_map",
                "::exec_fold", "::exec_drop", "::exec_batch", "::prep", "::ping", "::get_conn",
            ];
            if MY.iter().any(|v| path.ends_with(v)) {
                return Some("Db");
            }
            return None;
        }
        // sea_orm: an ORM whose execution is split from building (like sqlx). The query
        // BUILDERS (`Entity::find`, `Entity::insert`) are pure; execution happens at `.all`/
        // `.one`/`.count`/`.stream` and `Insert/Update/Delete::exec`. The write path via an
        // ActiveModel (`model.insert(db)`) executes too — distinguished from the `EntityTrait`
        // builder by the trait in the path (`ActiveModelTrait::`). (Found hardening on a
        // sea_orm consumer app: `.all(db)` reads and `ActiveModel::insert` writes were pure.)
        if crate_name == "sea_orm" {
            // sea_orm RE-EXPORTS sea_query (`sea_orm::sea_query::…`), whose builder algebra collides with
            // the execution verbs: `Func::count(col)` builds a COUNT() expr, `Condition::all()` AND-groups
            // filters, `Expr::count(…)` — all PURE, none touch a db. The `::all`/`::count`/`::one` execution
            // rule fabricated Db on them (sweep [5]). sea_query is pure query construction end-to-end, so
            // exclude the whole re-exported namespace first.
            if path.contains("sea_query") {
                return None;
            }
            if path.ends_with("::all")
                || path.ends_with("::one")
                || path.ends_with("::count")
                || path.ends_with("::stream")
                || path.ends_with("::exec")
                || path.ends_with("::exec_with_returning")
                || path.ends_with("::exec_without_returning")
                || path.ends_with("::connect")
                || path.ends_with("::execute")
                || path.ends_with("::execute_unprepared")
                || path.ends_with("::query_one")
                || path.ends_with("::query_all")
                || path.ends_with("::fetch_page")
                || path.ends_with("::num_items")
                || path.contains("ActiveModelTrait::")
            {
                return Some("Db");
            }
            return None;
        }
        // (Reached by sqlx + diesel — the build-vs-execute-split crates.) `first` is diesel's
        // LIMIT-1 round trip and `load_iter` its 2.x streaming execution; `fetch_many` is sqlx's
        // multi-result stream. All crate-gated, so a std `Vec::first` never resolves here.
        const VERBS: [&str; 19] = [
            "::execute", "::query_row", "::query_map", "::query_one", "::fetch_one",
            "::fetch_all", "::fetch_optional", "::fetch", "::fetch_many", "::connect",
            "::acquire", "::begin", "::commit", "::rollback", "::load", "::load_iter",
            "::first", "::get_result", "::get_results",
        ];
        if VERBS.iter().any(|v| path.ends_with(v)) {
            return Some("Db");
        }
        return None;
    }
    // std::path::Path / PathBuf STAT-family methods hit the filesystem (each is a stat/readlink/
    // readdir syscall) — unlike the rest of the std::path surface, which is pure string manipulation
    // (join/file_name/extension/parent/…). Verb-precise so the scanner's receiver inference can safely
    // route a `path.symlink_metadata()` method call here. (A blackout screen caught gix-dir — an entire
    // directory WALKER — reporting ZERO Fs because all its I/O is Path-method calls; same class as
    // fd's residual `Path::symlink_metadata` under-report.)
    if let Some(m) = path
        .strip_prefix("std::path::Path::")
        .or_else(|| path.strip_prefix("std::path::PathBuf::"))
    {
        const STAT: &[&str] = &[
            "metadata", "symlink_metadata", "canonicalize", "read_link", "read_dir", "exists",
            "try_exists", "is_file", "is_dir", "is_symlink",
        ];
        return STAT.contains(&m).then_some("Fs");
    }
    // Filesystem. `tokio::fs`/`async_std::fs` are the async mirrors of `std::fs`; `async_fs` is
    // smol's fs crate; `fs_err` is a drop-in `std::fs` wrapper (its whole surface is fs I/O).
    if path.starts_with("std::fs::")
        || path.starts_with("tokio::fs::")
        || path.starts_with("async_std::fs::")
        || crate_name == "async_fs"
        || crate_name == "fs_err"
    {
        return Some("Fs");
    }
    // memmap2: only `MmapOptions::map*` (and the in-place `Mmap::flush`/`make_*` protection
    // changes / `remap`) actually issue the mmap/msync/mprotect/mremap syscall = Fs. The rest of the
    // crate is PURE: `MmapOptions::new`/setters BUILD the request, and once a region is mapped, reads
    // over it (`Mmap::len`/`is_empty`/`as_ptr`/`as_mut_ptr`/`deref` into the byte slice) are plain
    // memory access with no syscall. Whole-crate Fs fabricated Fs on those reads (a `m.len()` the
    // scanner's receiver inference routes to `memmap2::Mmap::len`). Match the syscall-issuing verbs;
    // everything else returns None (pure). `map*` covers `map`/`map_mut`/`map_exec`/`map_copy`/
    // `map_copy_read_only`/`map_raw`/`map_raw_read_only`/`map_anon`.
    if crate_name == "memmap2" {
        let m = path.rsplit("::").next().unwrap_or(path);
        if m.starts_with("map")
            || m == "flush"
            || m == "flush_async"
            || m == "flush_range"
            || m == "flush_async_range"
            || m == "remap"
            || m.starts_with("make_")
            || m == "advise"
            || m == "advise_range"
            || m == "lock"
            || m == "unlock"
        {
            return Some("Fs");
        }
        return None;
    }
    // tempfile: creating a temp file/dir touches the disk. Match the create/persist verbs (the
    // `Builder` setters — prefix/suffix/rand_bytes — stay pure). `persist`/`keep` rename/retain
    // the file on disk; `close` removes it.
    if crate_name == "tempfile"
        && (path.ends_with("::tempfile")
            || path.ends_with("::tempfile_in")
            || path.ends_with("::tempdir")
            || path.ends_with("::tempdir_in")
            || path.ends_with("NamedTempFile::new")
            || path.ends_with("NamedTempFile::new_in")
            || path.ends_with("TempDir::new")
            || path.ends_with("TempDir::new_in")
            || path.ends_with("::persist")
            || path.ends_with("::persist_noclobber")
            || path.ends_with("::keep"))
    {
        return Some("Fs");
    }
    // glob: walks the filesystem to expand a pattern (the returned iterator reads directories).
    // `Pattern::matches` is pure string matching — match only the directory-walking entry points.
    if crate_name == "glob" && (path.ends_with("::glob") || path.ends_with("::glob_with")) {
        return Some("Fs");
    }
    // Password-hashing / KDF crates — the entropy tier (the TS engine's CTA lesson: an invisible
    // argon2 landed on exactly the call a security review cares about). In this engine's
    // verb-precise style the ENTROPY is the salt mint: `SaltString::generate(OsRng)` in the
    // password-hash API family, and bcrypt's `hash`/`hash_with_result` (salt minted internally).
    // Verification and explicit-salt hashing are deterministic recomputation — pure. `rand_core`
    // carries the OsRng source itself (otherwise the most common salt mint is invisible).
    if matches!(crate_name, "argon2" | "scrypt" | "pbkdf2" | "password_hash") {
        if path.contains("SaltString::generate") {
            return Some("Rand");
        }
        return None;
    }
    if crate_name == "bcrypt" {
        if path.ends_with("::hash") || path.ends_with("::hash_with_result") {
            return Some("Rand");
        }
        return None;
    }
    if crate_name == "rand_core" {
        if path.contains("OsRng")
            || path.ends_with("::next_u32")
            || path.ends_with("::next_u64")
            || path.ends_with("::fill_bytes")
        {
            return Some("Rand");
        }
        return None;
    }
    // Randomness / entropy. `getrandom`/`fastrand` are effectful end-to-end. `rand` is NOT — it
    // mixes entropy/generation (effectful) with *pure* distribution constructors (`Uniform::new`,
    // `Normal::new`) and deterministic-seed constructors (`seed_from_u64`). Flagging the whole crate
    // over-reported those as `Rand`; match only the calls that actually consume randomness — the
    // entropy sources (`OsRng`, `thread_rng`/`rng`, `from_entropy`/`from_os_rng`) and the generation
    // verbs (`gen*`/`random*`/`fill*`/`sample*`/`next_u*`). A `Uniform::new` is now correctly pure.
    if crate_name == "getrandom" {
        return Some("Rand");
    }
    // fastrand: like `rand`, it mixes entropy-consuming generation (effectful) with PURE deterministic
    // pieces. `Rng::with_seed(42)` is a DETERMINISTIC seeded constructor (consumes no entropy — the same
    // seed gives the same stream), and `Rng::fork`/`Rng::clone` just split/copy existing state. Those are
    // PURE; whole-crate Rand fabricated Rand on them. The effect is the value-drawing methods (`u32`/
    // `usize`/`bool`/`f64`/`char`/`alphanumeric`/`choice`/`choose_multiple`/`shuffle`/`fill`/the range
    // forms) AND the entropy-seeded entry points: bare `Rng::new()` (seeds from the global entropy-backed
    // generator), `fastrand::seed`, and the top-level `fastrand::u32(..)` free functions (which draw from
    // the thread-local generator). `with_seed` is exempted explicitly; any other method on an `Rng`
    // (i.e. a value draw) is Rand.
    if crate_name == "fastrand" {
        let m = path.rsplit("::").next().unwrap_or(path);
        // Provably pure: deterministic seeded ctor + state split/copy.
        if m == "with_seed" || m == "fork" || m == "clone" {
            return None;
        }
        // Everything else fastrand exposes either draws a value or seeds from entropy → Rand. (The crate
        // has no pure data types beyond the `Rng` handle itself, so a non-draw stray would have to be a
        // method we don't recognise — keep the effect, the safe direction.)
        return Some("Rand");
    }
    if crate_name == "rand" {
        let rng_verb = path.ends_with("::gen")
            || path.ends_with("::gen_range")
            || path.ends_with("::gen_bool")
            || path.ends_with("::gen_ratio")
            || path.ends_with("::random")
            || path.ends_with("::random_range")
            || path.ends_with("::random_bool")
            || path.ends_with("::random_ratio")
            || path.ends_with("::random_iter") // rand 0.9 iterator generator
            || path.ends_with("::gen_iter")
            || path.ends_with("::fill")
            || path.ends_with("::fill_bytes")
            || path.ends_with("::try_fill")
            || path.ends_with("::try_fill_bytes")
            || path.ends_with("::sample")
            || path.ends_with("::sample_iter")
            || path.ends_with("::next_u32")
            || path.ends_with("::next_u64")
            || path.ends_with("::thread_rng")
            || path.ends_with("::rng")
            || path.ends_with("::from_entropy")
            || path.ends_with("::from_os_rng");
        // `OsRng` is the OS entropy SOURCE, but `clone`/`fork`/`default` just copy or construct the
        // (zero-sized) handle and draw no entropy — pure, exactly like the `fastrand` arm's clone/fork
        // exemption above. The actual draws (`fill_bytes`/`next_u*`/…) are caught by `rng_verb`. Without
        // this exemption the blanket `contains("OsRng")` fabricated `Rand` on `OsRng::clone` (adversarial
        // review: OsRng is a unit struct, cloning consumes nothing).
        let m = path.rsplit("::").next().unwrap_or(path);
        let os_rng = path.contains("OsRng") && !matches!(m, "clone" | "fork" | "default");
        if rng_verb || os_rng {
            return Some("Rand");
        }
        return None;
    }
    // Subprocess spawning. `tokio::process` is the async mirror of `std::process` — it exists
    // only to spawn/control subprocesses (`Command`/`Child`, no pure data types like std's
    // `Stdio`/`ExitStatus`/`exit`), so spawning through it is Exec just the same. Without this an
    // async app's `tokio::process::Command::new(..).spawn()` classified pure — a silent under-report
    // of subprocess execution, the dangerous direction (mirrors the tokio::fs/tokio::net coverage).
    if path.starts_with("std::process::Command")
        || path.starts_with("std::process::Child")
        || path.starts_with("tokio::process::Command")
        || path.starts_with("tokio::process::Child")
        || path.starts_with("async_std::process::Command")
        || path.starts_with("async_std::process::Child")
    {
        // PURE read-backs of the builder's stored fields / the cached pid — no spawn, no syscall — so the
        // whole-type Exec rule fabricated Exec on them (sweep [23]; mirrors the portable_pty getter carve-
        // out just below). get_program/get_args/get_envs/get_current_dir read the Command; Child::id reads
        // the cached pid. Every genuine verb (new/spawn/output/status/wait/kill) stays Exec.
        if path.ends_with("::get_program")
            || path.ends_with("::get_args")
            || path.ends_with("::get_envs")
            || path.ends_with("::get_current_dir")
            || path.ends_with("Child::id")
        {
            return None;
        }
        return Some("Exec");
    }
    // portable_pty / async_process are whole-crate Exec EXCEPT for the proven-pure surface they expose:
    // the `CommandBuilder` GETTERS (`get_argv`/`get_cwd`/`get_env`/`as_unix_command_line`…) read back
    // configuration, and the PURE DATA types (`PtySize::default`, `ExitStatus`/`Stdio`/`CommandBuilder`
    // construction/setters). The earlier `is_cmd_naming_method` fix stopped the head-refinement LEAK, but
    // the BASE Exec still fabricated on these accessors (a `cmd.get_cwd()` the scanner routes to
    // `portable_pty::CommandBuilder::get_cwd`). Subtract the read-back getters and the obvious pure
    // ctors/setters; the spawn/wait/exec surface (`spawn_command`/`openpty`/`wait`/`kill`/`exec`…) keeps
    // Exec. SUBTRACT only what is provably pure — when unrecognised, KEEP Exec (the safe direction).
    if crate_name == "async_process" || crate_name == "portable_pty" {
        let m = path.rsplit("::").next().unwrap_or(path);
        // configuration read-back getters — pure (no spawn).
        if m.starts_with("get_") || m == "as_unix_command_line" {
            return None;
        }
        // pure data-type ctors/setters/derives that NAME no program and spawn nothing.
        if matches!(
            m,
            "default" | "new" | "piped" | "null" | "inherit" | "from_raw_fd"
                | "arg" | "args" | "arg0" | "env" | "envs" | "env_clear" | "env_remove"
                | "cwd" | "current_dir" | "rows" | "cols"
                | "clone" | "fmt" | "eq" | "ne" | "hash"
        ) {
            return None;
        }
        return Some("Exec");
    }
    // duct: a subprocess-orchestration crate. `cmd()`/`cmd!` only *build* an Expression; the
    // spawn/wait happens at `run`/`read`/`start`. Match the execution verbs, not the builder.
    if crate_name == "duct"
        && (path.ends_with("::run")
            || path.ends_with("::read")
            || path.ends_with("::start")
            || path.ends_with("::read_chars"))
    {
        return Some("Exec");
    }
    if path.starts_with("std::env::") {
        return Some("Env");
    }
    // dotenvy / dotenv: load environment variables (reading a `.env` file and mutating the process
    // environment). Match the load/read entry points; `Error`/builder types stay pure.
    if matches!(crate_name, "dotenvy" | "dotenv")
        && (path.ends_with("::dotenv")
            || path.ends_with("::dotenv_override")
            || path.ends_with("::from_path")
            || path.ends_with("::from_path_override")
            || path.ends_with("::from_filename")
            || path.ends_with("::from_filename_override")
            || path.ends_with("::from_read")
            || path.ends_with("::from_read_override")
            || path.ends_with("::load")
            || path.ends_with("::var")
            || path.ends_with("::vars"))
    {
        return Some("Env");
    }
    // Wall-clock reads. Match the `now` accessor precisely (ends_with), not any path
    // containing the substring "now". The `time` crate (distinct from `std::time`/`chrono`)
    // reads the clock via `now_utc`/`now_local` (and the deprecated `Instant::now`).
    if (crate_name == "chrono" || path.starts_with("std::time::")) && path.ends_with("::now") {
        return Some("Clock");
    }
    if crate_name == "time"
        && (path.ends_with("::now_utc") || path.ends_with("::now_local") || path.ends_with("::now"))
    {
        return Some("Clock");
    }
    // `tracing`: same principle as the `log` facade below — the crate's TYPES are pure data, so match
    // the emit, not the whole crate. The actual program output is the macro-expanded
    // `Subscriber::event`/`event!`/`Span::*enter*` dispatch and the `Span::new*`/`Span::record`
    // recording path that drives the subscriber. The data-type accessors — `Level::as_str`,
    // `Span::is_disabled`/`metadata`/`id`, and constructing/reading `Level`/`LevelFilter`/`Span`/
    // `Event`/`Metadata`/`Field`/`FieldSet`/`Id` — are PURE (no output is produced), so whole-crate Log
    // fabricated Log on them. Match the emit verbs; everything else returns None.
    if crate_name == "tracing" {
        let m = path.rsplit("::").next().unwrap_or(path);
        // The user-facing emit MACROS (`tracing::info!`/`warn!`/…) — candor-scan is pre-expansion, so it
        // sees the raw macro path `tracing::info`, not the expanded `__tracing`/`Subscriber::event` the
        // deep (post-expansion) engine sees. Only the macro names; the pure DATA types (Level/Span/Event)
        // have other tails and stay None.
        if m == "trace" || m == "debug" || m == "info" || m == "warn" || m == "error"
            || m == "trace_span" || m == "debug_span" || m == "info_span" || m == "warn_span"
            || m == "error_span" || m == "span"
            || m == "event"
            || m == "new_span"
            || m == "record"
            || m == "record_follows_from"
            || m == "enter"
            || m == "exit"
            || m == "in_scope"
            || m == "entered"
            || path.contains("::__macro_support")
            || path.contains("::__tracing")
            || path.contains("Subscriber::event")
            || path.contains("Subscriber::new_span")
            || path.contains("Subscriber::enter")
            || path.contains("Subscriber::exit")
        {
            return Some("Log");
        }
        return None;
    }
    // The `log` facade: its macros route through `log::__private_api`; the crate's types
    // (`Level`, `LevelFilter`) are pure, so match the logging entry, not the whole crate.
    if crate_name == "log" {
        // Expanded macro form (deep engine) OR the raw user-facing macro names (candor-scan, pre-expansion).
        // `log::Level`/`LevelFilter`/`Record`/`Metadata` have other tails, so the type surface stays pure.
        let m = path.rsplit("::").next().unwrap_or(path);
        if path.contains("::__private_api")
            || m == "error" || m == "warn" || m == "info" || m == "debug" || m == "trace" || m == "log"
        {
            return Some("Log");
        }
    }
    // Compiler diagnostic emission — the ONE genuinely effectful operation in the otherwise-pure
    // rustc_* surface (a dylint lint's actual OUTPUT: it writes warnings/errors to the compiler's
    // diagnostic sink). Classified `Log` (same family as `tracing`/`log` — program output). Match the
    // emission verbs precisely; rustc_lint/rustc_errors are mostly pure types (Lint, LintId, the Diag
    // BUILDERS), and only the terminal `emit`/`emit_span_lint` actually produces output.
    if crate_name == "rustc_lint"
        && (path.ends_with("::emit_span_lint")
            || path.ends_with("::span_lint")
            || path.ends_with("::span_lint_hir"))
    {
        return Some("Log");
    }
    if crate_name == "rustc_errors"
        && (path.ends_with("::emit")
            || path.ends_with("::emit_diagnostic")
            || path.ends_with("::emit_now"))
    {
        return Some("Log");
    }
    // arboard: the effectful surface is the `Clipboard` handle's read/write verbs (each talks to the
    // OS clipboard / X11/Wayland/Win32/NSPasteboard server). The data types — chiefly `arboard::Error`
    // (whose `Display`/`to_string` formatting is pure) and the `ImageData`/`GetExtLinux`/`SetExtLinux`
    // option types — are PURE, so whole-crate Clipboard fabricated Clipboard on e.g. an error
    // `to_string()`. Match the handle verbs; everything else returns None. `Clipboard::new` opens the
    // connection to the clipboard server, so it's an effect too; `get`/`set` return the
    // builder-then-read `Get`/`Set` cursors whose `text`/`image`/`html` terminals do the I/O.
    if crate_name == "arboard" {
        let m = path.rsplit("::").next().unwrap_or(path);
        if m == "new"
            || m == "get"
            || m == "set"
            || m == "clear"
            || m == "get_text"
            || m == "set_text"
            || m == "set_html"
            || m == "get_image"
            || m == "set_image"
            || m == "text"
            || m == "image"
            || m == "html"
        {
            return Some("Clipboard");
        }
        return None;
    }
    None
}

pub fn cap_from_name(name: &str) -> Option<&'static str> {
    EFFECTS.iter().copied().find(|e| *e == name)
}

/// Refine the `Exec` cliff (spec §4 ⟨0.5⟩): the effects a *literal, statically-known* subprocess
/// head implies, matched by basename (`/usr/bin/curl` → `curl`). The head's effects are ADDED to a
/// caller that already carries `Exec` (a subprocess is still spawned — `Exec` is never dropped); an
/// unrecognised or dynamically-built head returns `&[]` and keeps the bare cliff (never guess). A
/// **candor engine** reads `Fs`/`Env` only — spec §7 item 12 (the analyzer self-boundary) guarantees
/// that, so that case is spec-supplied, not curation. The rest is a small curated table under the
/// same under-report rule as the crate classifier. INVARIANT: every head here is an external tool
/// that does NOT run the analysed project's own code (so `make`/`npm`/`cargo` are deliberately
/// absent — they stay the cliff). The reference engines share this table so the `Exec` boundary —
/// the one boundary every engine hits — refines identically (the §4-consistency argument).
pub fn classify_command_head(cmd: &str) -> &'static [&'static str] {
    // Only UNAMBIGUOUS single-effect tools belong here. A multi-modal head (`git status` is local,
    // `git push` is Net; `rsync` local-vs-remote) would FABRICATE the effect for its common case —
    // the under-report rule forbids it, so such heads keep the bare cliff.
    match cmd.rsplit(['/', '\\']).next().unwrap_or(cmd) {
        "curl" | "wget" | "http" | "ssh" | "scp" | "sftp" | "ftp" | "telnet" => &["Net"],
        "psql" | "mysql" | "sqlite3" | "mongosh" | "mongo" | "redis-cli" | "cqlsh" | "influx" => &["Db"],
        // candor engines — Fs/Env only, guaranteed by spec §7 item 12 (the analyzer self-boundary)
        "candor" | "candor-run.sh" | "candor-scan" | "candor-query" | "candor-java"
        | "candor-classify" | "candor-report" | "cargo-candor" => &["Env", "Fs"],
        _ => &[],
    }
}

/// Whether a subprocess-builder method only MODIFIES the command (`.arg`, `.env`, `.current_dir`)
/// rather than NAMING the program (`Command::new`, `duct::cmd`). A WHOLE-CRATE-Exec crate
/// (`portable_pty`, `duct`, `async_process`) classifies *every* method as `Exec`, so the
/// head-refinement must skip these: an arg or env-var-name literal that happened to match a head
/// (`.env("psql", …)`, `.arg("curl")`) would FABRICATE that effect — the §1 under-report rule. The
/// method is the call path's last segment.
pub fn is_cmd_builder_method(method: &str) -> bool {
    matches!(
        method,
        "arg" | "args" | "arg0" | "env" | "envs" | "env_clear" | "env_remove" | "current_dir"
            | "cwd" | "stdin" | "stdout" | "stderr" | "pre_exec" | "creation_flags" | "uid" | "gid"
            | "groups" | "process_group"
    )
}

/// Whether a subprocess method NAMES the program (so its first string literal IS the command head to
/// refine): `Command::new("curl")`, `duct::cmd("curl", …)`. The head-refinement must fire ONLY here —
/// an ALLOWLIST, not "any method except known modifiers". A whole-crate-Exec crate classifies EVERY
/// method as `Exec`, so a denylist leaked NON-naming methods that aren't modifiers — a getter like
/// `CommandBuilder::get_env("psql")` (reading back an env-var KEY, not a program) fed `"psql"` to the
/// head classifier and FABRICATED `Db` (review find). Only `new`/`cmd` name a program; everything else
/// (modifiers, getters `get_*`, custom builder methods) keeps the bare `Exec` cliff — under-refine
/// (safe) rather than fabricate. `std::process::Command` is verb-precise so getters never fire `Exec`
/// there anyway; the allowlist makes the whole-crate-Exec crates safe too.
pub fn is_cmd_naming_method(method: &str) -> bool {
    matches!(method, "new" | "cmd")
}

/// The masking guard (AS-EFF-008): a Net call whose method takes the HOST/URL as an argument is
/// "establishing" — a classified Net call here with no captured host literal leaves the endpoint
/// structurally INVISIBLE (a runtime-built host), so the surface is incomplete and the gate must fail
/// closed (else a benign sibling literal masks the runtime endpoint). An ALLOWLIST of connection-
/// establishing verbs — the SAFE direction: a USE-verb on an already-connected socket
/// (`stream.write`/`read`/`flush`, `socket.send`/`recv`) is NOT here, so a missing literal there (the
/// host was fixed at `connect`) never false-positives. Under-catching an unusual establishing verb is a
/// missed mask (sound-with-disclosure), never a broken gate. The arg is the method (path's last segment).
pub fn is_net_establishing(method: &str) -> bool {
    matches!(
        method,
        "connect"
            | "connect_timeout"
            | "get"
            | "post"
            | "put"
            | "patch"
            | "delete"
            | "head"
            | "request"
            | "send_to"
            | "lookup_host"
            | "to_socket_addrs"
    )
}

/// Map a cap-std capability *type* to the effect it authorises. Holding one of these
/// (e.g. `&Dir`) is the real, unforgeable right to perform that effect — so candor
/// treats it as a declared capability, exactly like its own `&Fs` token.
pub fn capstd_cap(crate_name: &str, type_name: &str) -> Option<&'static str> {
    if !crate_name.starts_with("cap_") {
        return None;
    }
    Some(match type_name {
        "Dir" => "Fs",
        "TcpListener" | "TcpStream" | "UdpSocket" | "Pool" => "Net",
        "UnixListener" | "UnixStream" | "UnixDatagram" => "Ipc",
        "SystemClock" | "MonotonicClock" => "Clock",
        _ => return None,
    })
}

/// Table names a SQL string literal STATICALLY reaches — the `Db` analog of the `Net` host /
/// `Exec` command / `Fs` path literal surface (feeds `allow Db in <scope> <table>…`, AS-EFF-008).
/// Conservative by construction, because a wrong capture here would FABRICATE: the string must
/// open with a SQL statement keyword, and only identifiers in table position are taken —
/// `FROM`/`JOIN` anywhere, `INTO` anywhere, statement-leading `UPDATE`/`TRUNCATE`, and
/// `TABLE` (create/drop/alter), skipping `ONLY`/`IF NOT EXISTS`. `UPDATE` mid-statement is
/// deliberately ignored (`FOR UPDATE SKIP LOCKED` must not yield a table "skip"). A
/// dynamically-built query yields nothing — the gate's opaque case — never a guess.
/// Output is lower-cased, quote/backtick-stripped, `schema.table` kept qualified, deduped.
/// SPEC §2 pins this algorithm token-for-token across engines; the cross-impl vector battery
/// (candor-spec conformance/tables/vectors.json, run.sh Part 4b) enforces the JVM/TS mirrors.
pub fn tables_in_sql(sql: &str) -> Vec<String> {
    const STMT: &[&str] =
        &["select", "insert", "update", "delete", "create", "drop", "alter", "truncate", "merge", "replace", "with"];
    // Tokens that can FOLLOW a table-introducing keyword without being a table.
    const SKIP: &[&str] = &["only", "if", "not", "exists", "table"];
    // Identifier-position tokens that are grammar, not a table (subqueries, locking clauses…).
    const STOP: &[&str] = &[
        "select", "set", "where", "values", "on", "using", "group", "order", "by", "limit",
        "returning", "as", "inner", "outer", "left", "right", "cross", "lateral", "natural",
        "union", "all", "distinct", "case", "when", "null", "default", "skip", "nowait", "of",
        "from", "join", "into", "update", "delete", "insert",
    ];
    // `,` survives as its OWN token (not a space): it's what lets `FROM t1, t2` continue the table
    // list without fabricating from other comma-ridden positions (column lists, ON clauses).
    let cleaned: String = sql
        .to_lowercase()
        .chars()
        .flat_map(|c| match c {
            '(' | ')' | ';' => vec![' '],
            ',' => vec![' ', ',', ' '],
            _ => vec![c],
        })
        .collect();
    let toks: Vec<&str> = cleaned.split_whitespace().collect();
    let Some(first) = toks.first() else { return Vec::new() };
    if !STMT.contains(first) {
        return Vec::new(); // not SQL — nothing to certify, nothing fabricated
    }
    let ident = |t: &str| -> Option<String> {
        let t = t.trim_matches(|c| matches!(c, '"' | '`' | '\''));
        let mut chars = t.chars();
        let ok_first = chars.next().is_some_and(|c| c.is_ascii_alphabetic() || c == '_');
        let ok_rest = t.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '.' | '$' | '"' | '`'));
        (ok_first && ok_rest && !STOP.contains(&t)).then(|| t.replace(['"', '`'], ""))
    };
    let mut out: Vec<String> = Vec::new();
    let mut push = |t: Option<String>| {
        if let Some(t) = t {
            if !out.contains(&t) {
                out.push(t);
            }
        }
    };
    for (i, tok) in toks.iter().enumerate() {
        let table_pos = match *tok {
            "from" | "join" | "into" | "table" => true,
            // statement-leading only (see doc comment): `update t set …`, `truncate [table] t`.
            "update" | "truncate" => i == 0,
            _ => false,
        };
        if !table_pos {
            continue;
        }
        let mut j = i + 1;
        while j < toks.len() && SKIP.contains(&toks[j]) {
            j += 1;
        }
        let Some(next) = toks.get(j) else { continue };
        let Some(first) = ident(next) else { continue };
        push(Some(first));
        // Comma-ADJACENT continuation only: `FROM t1, t2, t3` takes all three, while an alias breaks
        // the chain (`FROM t1 a, t2` keeps just t1 — an under-report, never a guess: skipping an
        // alias to chase the comma would fabricate tables out of `INSERT INTO t (a, b)`'s column
        // list, whose parens are spaces by the time we tokenize).
        while j + 2 < toks.len() && toks[j + 1] == "," {
            let Some(more) = ident(toks[j + 2]) else { break };
            push(Some(more));
            j += 2;
        }
    }
    out
}

#[cfg(test)]
mod tests {
    #[test]
    fn sql_table_extraction_is_conservative() {
        use super::tables_in_sql as t;
        assert_eq!(t("SELECT id FROM users WHERE x = 1"), vec!["users"]);
        assert_eq!(t("select * from ledger.entries e join customers c on c.id = e.cid"),
                   vec!["ledger.entries", "customers"]);
        assert_eq!(t("INSERT INTO audit_log (a) VALUES (?1)"), vec!["audit_log"]);
        assert_eq!(t("UPDATE accounts SET v = ?"), vec!["accounts"]);
        assert_eq!(t("DELETE FROM sessions WHERE id = ?"), vec!["sessions"]);
        assert_eq!(t("CREATE TABLE IF NOT EXISTS cache (k TEXT)"), vec!["cache"]);
        assert_eq!(t("TRUNCATE TABLE staging"), vec!["staging"]);
        // FOR UPDATE locking clause must not yield a phantom table (mid-statement update ignored)
        assert_eq!(t("SELECT * FROM jobs FOR UPDATE SKIP LOCKED"), vec!["jobs"]);
        // a subquery in FROM position yields nothing for that position
        assert_eq!(t("SELECT * FROM (SELECT 1) q"), Vec::<String>::new());
        // not SQL -> nothing (never fabricate)
        assert_eq!(t("/tmp/some/path"), Vec::<String>::new());
        assert_eq!(t("hello world from nowhere"), Vec::<String>::new());
        // comma-ADJACENT continuation: a FROM list takes every table in the chain…
        assert_eq!(t("SELECT a FROM t1, t2, s.t3 WHERE x = 1"), vec!["t1", "t2", "s.t3"]);
        // …but an alias breaks it (under-report, never a guess)…
        assert_eq!(t("SELECT a FROM t1 a1, t2 WHERE x = 1"), vec!["t1"]);
        // …which is exactly what keeps a column list from fabricating (parens are spaces by now).
        assert_eq!(t("INSERT INTO t (a, b) VALUES (1, 2)"), vec!["t"]);
        // a subquery after the comma stops the chain too
        assert_eq!(t("SELECT a FROM t1, (SELECT 1) q"), vec!["t1"]);
    }

    use super::*;

    #[test]
    fn db_crates_are_calibrated() {
        // The calibrated set must cover every DB client the classifier knows, or the receipt's coverage
        // check would flag a recognized crate as a blind spot. (Was nightly-lint-only; now runs on stable.)
        for c in DB_CRATES {
            assert!(
                CALIBRATED_CRATES.contains(&c),
                "DB crate `{c}` is matched by classify() but missing from CALIBRATED_CRATES"
            );
        }
    }

    #[test]
    fn calibrated_crates_are_live() {
        // Conversely, every crate advertised as calibrated must actually be matched by classify() for
        // some representative path — a dead entry would silently suppress a real coverage warning.
        for c in CALIBRATED_CRATES {
            assert!(
                CALIBRATION_PROBE_TAILS.iter().any(|t| classify(c, &format!("{c}{t}")).is_some()),
                "calibrated crate `{c}` is matched by no path in classify() — dead list entry"
            );
        }
    }

    #[test]
    fn async_http_stack_classifies() {
        // The modern async-HTTP/TLS/QUIC/DNS stack (found by the independent-method differential on oha):
        // verb-keyed Net/Ipc/Fs/Env, crate-gated so generic verbs never fabricate across crates.
        assert_eq!(classify("hyper", "hyper::client::conn::http1::SendRequest::send_request"), Some("Net"));
        assert_eq!(classify("hyper", "hyper::client::conn::http1::handshake"), Some("Net"));
        assert_eq!(classify("hyper_util", "hyper_util::client::legacy::Client::request"), Some("Net"));
        assert_eq!(classify("hickory_resolver", "hickory_resolver::Resolver::lookup_ip"), Some("Net"));
        assert_eq!(classify("quinn", "quinn::Endpoint::connect"), Some("Net"));
        assert_eq!(classify("tokio_rustls", "tokio_rustls::TlsConnector::connect"), Some("Net"));
        assert_eq!(classify("native_tls", "native_tls::TlsConnector::connect"), Some("Net"));
        assert_eq!(classify("tokio_vsock", "tokio_vsock::VsockStream::connect"), Some("Ipc"));
        assert_eq!(classify("rustls_native_certs", "rustls_native_certs::load_native_certs"), Some("Fs"));
        assert_eq!(classify("num_cpus", "num_cpus::get"), Some("Env"));
        assert_eq!(classify("rlimit", "rlimit::setrlimit"), Some("Env"));
        // pure surface stays None (no fabrication): builder/type/config paths, and other crates' generic verbs
        assert_eq!(classify("hyper", "hyper::Request::builder"), None);
        assert_eq!(classify("hyper", "hyper::body::Bytes::new"), None);
        assert_eq!(classify("native_tls", "native_tls::TlsConnectorBuilder::min_protocol_version"), None);
        assert_eq!(classify("serde", "serde::Deserialize::request"), None); // generic verb, wrong crate
    }

    #[test]
    fn log_tracing_emit_macros_classify_pre_expansion() {
        // candor-scan is pre-expansion: it sees the raw macro path (`log::info`, `tracing::warn`), not the
        // expanded dispatch the deep engine sees. Both the user-facing macro names AND the type surface:
        assert_eq!(classify("log", "log::info"), Some("Log"));
        assert_eq!(classify("log", "log::error"), Some("Log"));
        assert_eq!(classify("tracing", "tracing::warn"), Some("Log"));
        assert_eq!(classify("tracing", "tracing::info_span"), Some("Log"));
        // pure data-type surface stays None (no fabricated Log)
        assert_eq!(classify("log", "log::Level::as_str"), None);
        assert_eq!(classify("tracing", "tracing::Level::INFO"), None);
    }

    #[test]
    fn classify_core_effects() {
        // A representative smoke test of the classifier's main families, so the published crate is not
        // shipped untested (these used to live only in the nightly-only src/lib.rs).
        assert_eq!(classify("std", "std::fs::read_to_string"), Some("Fs"));
        // std::path stat-family methods are Fs (each is a stat/readdir syscall); the pure
        // string-manipulation surface stays unclassified (the blackout screen's gix-dir find).
        assert_eq!(classify("std", "std::path::Path::symlink_metadata"), Some("Fs"));
        assert_eq!(classify("std", "std::path::PathBuf::read_dir"), Some("Fs"));
        assert_eq!(classify("std", "std::path::Path::exists"), Some("Fs"));
        assert_eq!(classify("std", "std::path::Path::join"), None); // pure string manipulation
        assert_eq!(classify("std", "std::path::PathBuf::file_name"), None);
        assert_eq!(classify("std", "std::path::Path::parent"), None);
        assert_eq!(classify("std", "std::process::Command::new"), Some("Exec"));
        assert_eq!(classify("std", "std::env::var"), Some("Env"));
        assert_eq!(classify("reqwest", "reqwest::Client::execute"), Some("Net"));
        // one-shot convenience fns send immediately → Net; the `Client::get` builder stays pure.
        assert_eq!(classify("reqwest", "reqwest::get"), Some("Net"));
        assert_eq!(classify("reqwest", "reqwest::blocking::get"), Some("Net"));
        assert_eq!(classify("reqwest", "reqwest::Client::get"), None);
        assert_eq!(classify("reqwest", "reqwest::RequestBuilder::header"), None);
        // nix routes through the libc syscall table (same leaves): I/O classified, generic fd ops skipped.
        assert_eq!(classify("nix", "nix::fcntl::open"), Some("Fs"));
        assert_eq!(classify("nix", "nix::sys::socket::connect"), Some("Net"));
        assert_eq!(classify("nix", "nix::unistd::execvp"), Some("Exec"));
        assert_eq!(classify("nix", "nix::unistd::write"), None); // generic fd op — deliberately unclassified
        assert_eq!(classify("nix", "nix::unistd::getpid"), None); // not I/O
        // rustix does raw syscalls (no libc underneath) → classified directly by leaf, same table.
        assert_eq!(classify("rustix", "rustix::time::clock_settime"), Some("Clock"));
        assert_eq!(classify("rustix", "rustix::fs::symlink"), Some("Fs"));
        assert_eq!(classify("rustix", "rustix::net::connect"), Some("Net"));
        assert_eq!(classify("rustix", "rustix::io::read"), None); // generic fd op
        // pnet raw packet capture: channel openers are Net, packet construction stays pure.
        assert_eq!(classify("pnet", "pnet::datalink::channel"), Some("Net"));
        assert_eq!(classify("pnet", "pnet::transport::transport_channel"), Some("Net"));
        assert_eq!(classify("pnet_datalink", "pnet_datalink::channel"), Some("Net"));
        assert_eq!(classify("pnet", "pnet::packet::ethernet::EthernetPacket::new"), None);
        assert_eq!(classify("pnet_base", "pnet_base::MacAddr::new"), None);
        // ignore (gitignore-aware walker): walk executors are Fs, config builders stay pure.
        assert_eq!(classify("ignore", "ignore::WalkBuilder::build_parallel"), Some("Fs"));
        assert_eq!(classify("ignore", "ignore::WalkBuilder::build"), Some("Fs"));
        assert_eq!(classify("ignore", "ignore::WalkParallel::run"), Some("Fs"));
        assert_eq!(classify("ignore", "ignore::WalkBuilder::add_ignore"), Some("Fs")); // reads the ignore file
        assert_eq!(classify("ignore", "ignore::overrides::OverrideBuilder::build"), None); // pure config
        assert_eq!(classify("ignore", "ignore::gitignore::GitignoreBuilder::build"), None); // pure config
        assert_eq!(classify("ignore", "ignore::DirEntry::path"), None); // pure accessor
        // notify fs-watching: watcher constructors + watch/unwatch are Fs, data types stay pure.
        assert_eq!(classify("notify", "notify::RecommendedWatcher::new"), Some("Fs"));
        assert_eq!(classify("notify", "notify::PollWatcher::new"), Some("Fs"));
        assert_eq!(classify("notify", "notify::recommended_watcher"), Some("Fs"));
        assert_eq!(classify("notify", "notify::INotifyWatcher::watch"), Some("Fs"));
        assert_eq!(classify("notify", "notify::Config::default"), None); // pure config
        assert_eq!(classify("notify", "notify::Event::new"), None); // pure data type
        assert_eq!(classify("rusqlite", "rusqlite::Connection::execute"), Some("Db"));
        // the rusqlite verb DIALECT (a verb probe found the canonical consumer API classifying pure):
        assert_eq!(classify("rusqlite", "rusqlite::Connection::query_row"), Some("Db"));
        assert_eq!(classify("rusqlite", "rusqlite::Statement::query_map"), Some("Db"));
        assert_eq!(classify("rusqlite", "rusqlite::Connection::execute_batch"), Some("Db"));
        assert_eq!(classify("rusqlite", "rusqlite::Connection::prepare_cached"), Some("Db"));
        assert_eq!(classify("rusqlite", "rusqlite::Connection::open"), Some("Db"));
        assert_eq!(classify("rusqlite", "rusqlite::Connection::open_in_memory"), Some("Db"));
        // …but `open` stays rusqlite-only (postgres has no open; nothing else may borrow it):
        assert_eq!(classify("postgres", "postgres::Client::open"), None);
        assert_eq!(classify("tokio_postgres", "tokio_postgres::Client::query_typed"), Some("Db"));
        // diesel's LIMIT-1 + streaming executions; sqlx's multi-result stream:
        assert_eq!(classify("diesel", "diesel::RunQueryDsl::first"), Some("Db"));
        assert_eq!(classify("diesel", "diesel::RunQueryDsl::load_iter"), Some("Db"));
        assert_eq!(classify("sqlx", "sqlx::query::Query::fetch_many"), Some("Db"));
        // sqlx's bare `query()` builder must STAY pure (the original sqlx lesson):
        assert_eq!(classify("sqlx", "sqlx::query"), None);
        // tracing: the emit/span-lifecycle dispatch is Log; the pure DATA-type accessors are not
        // (whole-crate Log fabricated Log on `Level::as_str` / `Span::is_disabled` — the data types are
        // pure, same principle as the `log` facade).
        assert_eq!(classify("tracing", "tracing::event"), Some("Log"));
        assert_eq!(classify("tracing", "tracing::Span::new_span"), Some("Log"));
        assert_eq!(classify("tracing", "tracing::Span::record"), Some("Log"));
        assert_eq!(classify("tracing", "tracing::Span::enter"), Some("Log"));
        assert_eq!(classify("tracing", "tracing::Level::as_str"), None); // pure accessor
        assert_eq!(classify("tracing", "tracing::Span::is_disabled"), None); // pure state read
        assert_eq!(classify("tracing", "tracing::Span::metadata"), None); // pure accessor
        assert_eq!(classify("tracing", "tracing::metadata::Level::TRACE"), None); // pure data type
        assert_eq!(classify("tracing", "tracing::field::Field::name"), None); // pure data type
        // memmap2: only the syscall-issuing map/flush/protect verbs are Fs; reads over an already-mapped
        // region (len/as_ptr/is_empty) and the request builder are PURE (whole-crate Fs fabricated Fs).
        assert_eq!(classify("memmap2", "memmap2::MmapOptions::map"), Some("Fs"));
        assert_eq!(classify("memmap2", "memmap2::MmapOptions::map_mut"), Some("Fs"));
        assert_eq!(classify("memmap2", "memmap2::Mmap::flush"), Some("Fs"));
        assert_eq!(classify("memmap2", "memmap2::MmapMut::make_read_only"), Some("Fs"));
        assert_eq!(classify("memmap2", "memmap2::Mmap::len"), None); // length read — pure
        assert_eq!(classify("memmap2", "memmap2::Mmap::is_empty"), None); // pure
        assert_eq!(classify("memmap2", "memmap2::Mmap::as_ptr"), None); // pointer — pure
        assert_eq!(classify("memmap2", "memmap2::MmapOptions::new"), None); // request builder — pure
        // arboard: the Clipboard handle's read/write verbs are Clipboard; `arboard::Error` formatting
        // and option data types are PURE (whole-crate Clipboard fabricated Clipboard on `Error::to_string`).
        assert_eq!(classify("arboard", "arboard::Clipboard::new"), Some("Clipboard"));
        assert_eq!(classify("arboard", "arboard::Clipboard::get_text"), Some("Clipboard"));
        assert_eq!(classify("arboard", "arboard::Clipboard::set_text"), Some("Clipboard"));
        assert_eq!(classify("arboard", "arboard::Clipboard::clear"), Some("Clipboard"));
        assert_eq!(classify("arboard", "arboard::Error::to_string"), None); // error formatting — pure
        assert_eq!(classify("arboard", "arboard::Error::fmt"), None); // Display impl — pure
        assert_eq!(classify("arboard", "arboard::ImageData::to_owned_img"), None); // pure data type
        // fastrand: value draws + entropy-seeded entry points are Rand; the DETERMINISTIC seeded ctor
        // `with_seed` and state split/copy (`fork`/`clone`) are PURE (whole-crate Rand fabricated Rand).
        assert_eq!(classify("fastrand", "fastrand::u32"), Some("Rand")); // top-level draw
        assert_eq!(classify("fastrand", "fastrand::Rng::usize"), Some("Rand"));
        assert_eq!(classify("fastrand", "fastrand::Rng::shuffle"), Some("Rand"));
        assert_eq!(classify("fastrand", "fastrand::Rng::new"), Some("Rand")); // entropy-seeded
        assert_eq!(classify("fastrand", "fastrand::Rng::with_seed"), None); // deterministic ctor — pure
        assert_eq!(classify("fastrand", "fastrand::Rng::fork"), None); // state split — pure
        assert_eq!(classify("fastrand", "fastrand::Rng::clone"), None); // state copy — pure
        // portable_pty / async_process: spawn/wait keep Exec; config GETTERS and pure data ctors/setters
        // do NOT (base Exec fabricated on `CommandBuilder::get_cwd` / `PtySize::default` / `Stdio::piped`).
        assert_eq!(classify("portable_pty", "portable_pty::PtySystem::openpty"), Some("Exec"));
        assert_eq!(classify("portable_pty", "portable_pty::SlavePty::spawn_command"), Some("Exec"));
        assert_eq!(classify("portable_pty", "portable_pty::CommandBuilder::get_argv"), None); // getter
        assert_eq!(classify("portable_pty", "portable_pty::CommandBuilder::get_cwd"), None); // getter
        assert_eq!(classify("portable_pty", "portable_pty::PtySize::default"), None); // pure data type
        assert_eq!(classify("portable_pty", "portable_pty::CommandBuilder::new"), None); // builder ctor
        assert_eq!(classify("async_process", "async_process::Command::spawn"), Some("Exec"));
        assert_eq!(classify("async_process", "async_process::Command::output"), Some("Exec"));
        assert_eq!(classify("async_process", "async_process::Stdio::piped"), None); // pure data type
        assert_eq!(classify("async_process", "async_process::Stdio::null"), None); // pure data type
        // FFI tiers (matched by distinctive leaf, alias-independent)
        assert_eq!(classify("libc", "libc::open"), Some("Fs"));
        assert_eq!(classify("libc", "libc::connect"), Some("Net"));
        assert_eq!(classify("libc", "libc::read"), None); // generic fd op — deliberately unclassified
        assert_eq!(classify("ffi", "ffi::sqlite3_step"), Some("Db"));
        assert_eq!(classify("raw", "raw::git_remote_fetch"), Some("Net"));
        // libgit2 clone + submodule clone/update fetch over the network (an A/B on git2 0.20 caught
        // `Submodule::update`/`clone` and `Repository::clone` reporting no Net — the latter because the
        // `src/build.rs` module was being dropped as if it were the Cargo build script).
        assert_eq!(classify("raw", "raw::git_clone"), Some("Net"));
        assert_eq!(classify("raw", "raw::git_submodule_clone"), Some("Net"));
        assert_eq!(classify("raw", "raw::git_submodule_update"), Some("Net"));
        assert_eq!(classify("raw", "raw::git_submodule_open"), None); // local subrepo open — not Net
        // libcurl: the transfer/raw-socket entry points are Net (an A/B on curl 0.4 caught the whole
        // crate reporting ZERO Net); the big setopt/init/getinfo surface — and the readiness-wait
        // multi_wait/poll — stay unclassified (the loop's perform is the boundary).
        assert_eq!(classify("curl_sys", "curl_sys::curl_easy_perform"), Some("Net"));
        assert_eq!(classify("curl_sys", "curl_sys::curl_easy_send"), Some("Net"));
        assert_eq!(classify("curl_sys", "curl_sys::curl_multi_perform"), Some("Net"));
        assert_eq!(classify("curl_sys", "curl_sys::curl_multi_socket_action"), Some("Net"));
        assert_eq!(classify("curl_sys", "curl_sys::curl_easy_setopt"), None); // in-memory option write
        assert_eq!(classify("curl_sys", "curl_sys::curl_easy_init"), None); // handle alloc
        assert_eq!(classify("curl_sys", "curl_sys::curl_multi_wait"), None); // readiness wait, no payload
        // consumer-side `curl` crate rule: the dispatch verbs are Net, the setopt builders pure.
        assert_eq!(classify("curl", "curl::easy::Easy::perform"), Some("Net"));
        assert_eq!(classify("curl", "curl::multi::Multi::perform"), Some("Net"));
        assert_eq!(classify("curl", "curl::easy::Easy::send"), Some("Net"));
        assert_eq!(classify("curl", "curl::easy::Easy::url"), None); // CURLOPT setter — pure
        assert_eq!(classify("curl", "curl::easy::Easy::timeout"), None); // pure setter; Multi::timeout under-reported by design
        assert_eq!(classify("ffi", "ffi::SSL_connect"), Some("Net"));
        // pure crates stay pure
        assert_eq!(classify("serde", "serde::Serialize::serialize"), None);
        assert_eq!(classify("std", "std::vec::Vec::push"), None);

        // ── sweep 2026-06-17: fabrication carve-outs + DNS coverage (each fails pre-fix) ──
        // [24] std::net socket accessors are pure; the I/O verbs stay Net.
        assert_eq!(classify("std", "std::net::TcpStream::connect"), Some("Net"));
        assert_eq!(classify("std", "std::net::TcpStream::local_addr"), None);
        assert_eq!(classify("std", "std::net::TcpStream::nodelay"), None);
        assert_eq!(classify("std", "std::net::TcpStream::ttl"), None);
        assert_eq!(classify("std", "std::net::UdpSocket::peer_addr"), None);
        // [37] std DNS resolution is Net (was floored).
        assert_eq!(classify("std", "std::net::lookup_host"), Some("Net"));
        assert_eq!(classify("std", "core::net::ToSocketAddrs::to_socket_addrs"), Some("Net"));
        // [23] std::process getters are pure; spawn/new stay Exec.
        assert_eq!(classify("std", "std::process::Command::get_program"), None);
        assert_eq!(classify("std", "std::process::Command::get_args"), None);
        assert_eq!(classify("std", "std::process::Child::id"), None);
        assert_eq!(classify("std", "std::process::Command::spawn"), Some("Exec"));
        // [27] redis ConnectionManager::clone is an Arc bump (pure); a query round-trips.
        assert_eq!(classify("redis", "redis::aio::ConnectionManager::clone"), None);
        assert_eq!(classify("redis", "redis::aio::ConnectionManager::send_packed_command"), Some("Db"));
        // [5] sea_orm re-exported sea_query builder algebra is pure; execution verbs stay Db.
        assert_eq!(classify("sea_orm", "sea_orm::sea_query::Func::count"), None);
        assert_eq!(classify("sea_orm", "sea_orm::sea_query::Condition::all"), None);
        assert_eq!(classify("sea_orm", "sea_orm::Select::all"), Some("Db"));
    }

    #[test]
    fn rand_osrng_handle_ops_are_pure_but_draws_are_rand() {
        // Adversarial-review fabrication: the blanket `contains("OsRng")` tagged `OsRng::clone` Rand,
        // but OsRng is a unit struct — clone/fork/default draw no entropy. The real draws still fire.
        assert_eq!(classify("rand", "rand::rngs::OsRng::clone"), None);
        assert_eq!(classify("rand", "rand::rngs::OsRng::default"), None);
        assert_eq!(classify("rand", "rand::rngs::OsRng::fill_bytes"), Some("Rand")); // a real draw
        assert_eq!(classify("rand", "rand::rngs::OsRng::next_u32"), Some("Rand"));
        assert_eq!(classify("rand", "rand::Rng::gen"), Some("Rand")); // verb path unaffected
        assert_eq!(classify("rand", "rand::distributions::Uniform::new"), None); // pure ctor still pure
    }

    #[test]
    fn redis_connection_manager_config_builder_is_pure() {
        // Adversarial-review fabrication: `contains("ConnectionManager")` hit the pure *Config* builder.
        assert_eq!(classify("redis", "redis::aio::ConnectionManagerConfig::new"), None);
        assert_eq!(classify("redis", "redis::aio::ConnectionManagerConfig::set_max_delay"), None);
        // the LIVE manager still round-trips (Db).
        assert_eq!(classify("redis", "redis::aio::ConnectionManager::new"), Some("Db"));
        assert_eq!(classify("redis", "redis::Commands::get"), Some("Db"));
    }

    #[test]
    fn pure_fd_transfer_is_not_an_effect() {
        // ADOPTING / EXTRACTING / BORROWING an already-open descriptor (or unwrapping an async type back
        // to its std type) issues NO syscall — it must be PURE even though it hangs off a std I/O type
        // whose prefix rule would otherwise fire Net/Fs/Ipc. (Real tokio sweep: `into_std`, `from_raw_fd`,
        // `as_raw_fd` all fabricated effects.)
        assert_eq!(classify("std", "std::net::TcpStream::from_raw_fd"), None);
        assert_eq!(classify("std", "std::net::TcpStream::into_raw_fd"), None);
        assert_eq!(classify("std", "std::net::TcpStream::as_raw_fd"), None);
        assert_eq!(classify("std", "std::net::TcpListener::from_raw_fd"), None);
        assert_eq!(classify("std", "std::net::UdpSocket::from_raw_socket"), None);
        assert_eq!(classify("std", "std::fs::File::from_raw_fd"), None);
        assert_eq!(classify("std", "std::fs::File::into_raw_fd"), None);
        assert_eq!(classify("std", "std::fs::File::as_raw_handle"), None);
        assert_eq!(classify("std", "std::os::unix::net::UnixStream::from_raw_fd"), None);
        // `SocketAddr::from_pathname` builds an address struct, opens no socket — pure. (socket2 sweep.)
        assert_eq!(classify("std", "std::os::unix::net::SocketAddr::from_pathname"), None);
        assert_eq!(classify("tokio", "tokio::net::TcpStream::from_raw_fd"), None);
        assert_eq!(classify("tokio", "tokio::net::TcpStream::into_std"), None); // unwrap → std type, pure
        assert_eq!(classify("tokio", "tokio::fs::File::into_std"), None);
        // …but a REAL open/connect on the SAME types still fires the effect — the carve-out is leaf-precise.
        assert_eq!(classify("std", "std::net::TcpStream::connect"), Some("Net"));
        assert_eq!(classify("std", "std::fs::File::open"), Some("Fs"));
        assert_eq!(classify("std", "std::fs::read"), Some("Fs"));
        assert_eq!(classify("std", "std::os::unix::net::UnixStream::connect"), Some("Ipc"));
        assert_eq!(classify("tokio", "tokio::net::TcpStream::connect"), Some("Net"));
    }

    #[test]
    fn command_head_refines_the_exec_cliff() {
        use super::classify_command_head as h;
        // unambiguous external tools classify by basename (spec §4 ⟨0.5⟩)
        assert_eq!(h("curl"), &["Net"]);
        assert_eq!(h("telnet"), &["Net"]);
        assert_eq!(h("sftp"), &["Net"]);
        assert_eq!(h("/usr/local/bin/psql"), &["Db"]); // basename match strips the path
        assert_eq!(h("mongo"), &["Db"]);
        assert_eq!(h("cqlsh"), &["Db"]);
        // a candor engine is Fs/Env — spec-SUPPLIED by §7 item 12, not curation
        assert_eq!(h("candor-scan"), &["Env", "Fs"]);
        assert_eq!(h("candor-run.sh"), &["Env", "Fs"]);
        // an unrecognised head adds nothing — the bare Exec cliff stands (never guess). `make`/`npm`
        // run the project's own code; `git`/`rsync` are multi-modal (local vs remote) — all keep the
        // cliff rather than fabricate an effect for the common case.
        assert_eq!(h("some-unknown-tool"), &[] as &[&str]);
        assert_eq!(h("make"), &[] as &[&str]);
        assert_eq!(h("npm"), &[] as &[&str]);
        assert_eq!(h("git"), &[] as &[&str]);
        assert_eq!(h("rsync"), &[] as &[&str]);
        // a builder MODIFIER (`.arg`/`.env`) names no program — its literal must NOT refine (a
        // whole-crate-Exec crate classifies every method; `.env("psql",..)` must not fabricate Db).
        assert!(is_cmd_builder_method("env") && is_cmd_builder_method("arg") && is_cmd_builder_method("current_dir"));
        assert!(!is_cmd_builder_method("new")); // Command::new NAMES the program
        assert!(!is_cmd_builder_method("cmd")); // duct::cmd NAMES the program
        // The gate that ADMITS a literal to classify_command_head is an ALLOWLIST of program-NAMING
        // methods, not the builder denylist. Inversion matters: a whole-crate-Exec crate (portable_pty)
        // classifies EVERY method as Exec, so a getter like `cmd.get_env("psql")` — absent from the
        // builder denylist — would have leaked "psql" to the head and FABRICATED Db. Only `new`/`cmd`
        // name a program, so only they may refine.
        assert!(is_cmd_naming_method("new") && is_cmd_naming_method("cmd"));
        assert!(!is_cmd_naming_method("get_env")); // a GETTER, not a namer — the leak this closes
        assert!(!is_cmd_naming_method("arg") && !is_cmd_naming_method("env") && !is_cmd_naming_method("current_dir"));
    }

    #[test]
    fn net_establishing_allowlist() {
        // sweep [3]/[7]: the masking guard's establishing-verb allowlist — host-bearing connect/request
        // verbs establish (a runtime host there is invisible); USE-verbs on a connected socket do NOT.
        assert!(is_net_establishing("connect") && is_net_establishing("connect_timeout"));
        assert!(is_net_establishing("get") && is_net_establishing("post") && is_net_establishing("request"));
        assert!(is_net_establishing("send_to") && is_net_establishing("to_socket_addrs"));
        // use-verbs (host fixed at connect) must NOT be establishing — else `connect("h").write()` flags.
        assert!(!is_net_establishing("write") && !is_net_establishing("read") && !is_net_establishing("send"));
        assert!(!is_net_establishing("flush") && !is_net_establishing("recv") && !is_net_establishing("peek"));
    }
}
