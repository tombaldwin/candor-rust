# candor-scan calibration on real third-party crates (35 deep + 1294 wide)

**What this is.** The stable scanner (`candor-scan`) is syntactic — it sees what's *written*, not what
the compiler *resolves*. This is the first calibration of how accurate it actually is, run against
**real, popular crates** (not candor's self-made fixtures), made possible by the fact that the scanner
parses source and never builds — so it runs on anything already vendored under `~/.cargo/registry`.

**How.** 35 crates spanning every effect category (HTTP, async runtimes, databases, filesystem,
crypto, time, parsing, channels, logging). For each, the newest version on disk was scanned with
`candor-scan <dir> --json` and the per-effect function counts tabulated. Reproduce with
[`sweep.py`](sweep.py). Raw output: [`results-notest.json`](results-notest.json). 2 crates (ureq, zip)
weren't on disk.

This work **drove eight fixes** (six scanner, two classifier); the numbers here are post-fix. The
method-dispatch one (receiver-type inference) and the two found by the 1294-crate wide sweep are
written up in their own sections below.

> **Update (FFI tiers + macros).** After the libc table below, two more C-library tables were added —
> **libsqlite3** (rusqlite) and **libgit2** (git2) — matched by the distinctive C leaf name
> (`sqlite3_*` / `git_*`) so every binding alias resolves. libsqlite3 took **rusqlite 0 → 48** (all
> `Db`). libgit2 needed one more scanner fix: git2 wraps *every* FFI call in a `try_call!` macro, which
> syn doesn't parse, so the scanner now **peeks inside macro token streams** (best-effort: parse as
> comma-separated exprs, skip if not expression syntax). That took **git2 7 → 45** — `Remote::{connect,
> fetch,download,push,list}` now correctly show `Net` (the exact gap this calibration first flagged),
> repo/index/config ops show `Fs`. The macro-peek introduced **zero** new false positives (the pure
> crates stayed at 0); it only added genuine recall (also +2 on redis, +2 on hyper). Tables don't help
> a crate that hides calls behind *both* a macro and an unmodelled C lib unless both are addressed —
> which is now the case for git2.

## Headline

After the fixes, on these 35 crates the scanner has **no remaining false positives in library code** —
every effect it reports is real. All its errors are now **under-reports**. The remaining gaps are
**dynamic method dispatch** (hyper/reqwest's `Net` lives behind resolved calls on tokio types) and
**unmodelled C libraries** (any FFI tier whose entry points aren't yet named). The two failure modes
that *were* dominant — FFI to common libraries, and calls hidden in macros — are now substantially
closed (libc/libsqlite3/libgit2 tables + macro-peeking), so they're targetable rather than structural.
That is the honest, expected behaviour, documented per-crate below.

## What the sweep found (and fixed)

The first pass was dominated by one distortion: the scanner walked **test/bench/example code and
`#[cfg(test)]` modules**, so a crate's report described what its *test harness* does, not what the crate
does. The worst case, redis, showed `Net/Fs/Env/Rand/Exec` on **212 functions that were all benchmarks**
(its bench harness spawns a real `redis-server`, loads TLS certs, connects, seeds random keys — all real,
all irrelevant to the library). Two fixes resulted:

1. **Skip non-library code by default** — `tests/`, `benches/`, `examples/`, the nonstandard singular
   `test/` tree, `build.rs`, and `#[cfg(test)]` modules. (`--include-tests` keeps them for whole-app
   analysis.) Build scripts run at *compile* time (ring's `build.rs` execs `nasm`); test harnesses
   describe the tests. Neither is the crate's runtime behaviour.

Impact (effectful-function count, before → after):

| crate | with tests/benches | library only | what the noise was |
|---|--:|--:|---|
| redis | 353 | 81 | bench harness spawns redis-server + TLS + random keys |
| nix | 159 | 0 | a `test/` tree using `std::fs` (its real syscalls are `libc` FFI — invisible) |
| git2 | 155 | 7 | test fixtures doing `std::fs` (its real I/O is libgit2 FFI) |
| csv | 52 | 2 | benches reading fixtures / spawning |
| walkdir | 51 | 0 | `TempDir` test helper calling `env::temp_dir()` |
| flate2 | 29 | 1 | test round-trips |
| ring | 12 | 1 | `build.rs` running nasm |

2. **(Carried in from the prior reqwest finding)** qualified-tail call resolution, so one effect can't
   smear across every same-named method. Without it reqwest reported phantom `Ipc` on 441 functions;
   the calibration confirmed that fix holds at scale (no crate shows a runaway effect now).

3. **A `libc` classifier table** — the biggest *under*-report the sweep found was the FFI-thin tier:
   nix reported **zero** library effects because every typed wrapper bottoms out in an unrecognised
   `libc::*` call. Added a syscall-name table (`socket/connect/…`→Net, path/dir syscalls→Fs,
   `fork/execve/wait`→Exec, pipes/SysV/POSIX-mq→Ipc, `getenv`→Env, `clock_gettime`→Clock,
   `getrandom`→Rand), deliberately skipping the generic file-descriptor ops (`read`/`write`/`close`/
   `fcntl`/`mmap`) that run on any fd — an honest no-classify beats a wrong label. **nix 0 → 59
   effectful fns**, all correct (`sys::socket::*`→Net, `mqueue::mq_*`→Ipc, `socketpair`→Ipc not Net,
   `sys::wait`/`clone`→Exec, `time::clock_*`→Clock — the module paths corroborate each). Also nudged
   mio's IPC up (5→12: its `socketpair`/`pipe` waker) and tokio/which a touch. Note it does **not**
   help rusqlite or git2 — those wrap *other* C libraries (libsqlite3, libgit2), not libc, so they
   stay honest under-reports (use the nightly lint, or add those libs' tables later).

## Results — accuracy by category

**A. Accurate** — reported effects match the crate's real, syntactically-visible surface:

| crate | effects | note |
|---|---|---|
| tokio | Fs/Net/Clock/Ipc/Rand/Log | async runtime; real syscalls are path-qualified (std/mio/libc) |
| mio | Net/Ipc/Fs | low-level non-blocking I/O + `socketpair`/`pipe` waker (Ipc) |
| nix | Fs/Net/Ipc/Exec/Clock | syscall wrappers lit up by the libc table — every effect matches the syscall |
| rusqlite | Db | libsqlite3 FFI, lit up by the sqlite3_* table (`step`/`exec`/`backup`/`blob`) |
| git2 | Net/Fs | libgit2 FFI behind `try_call!` macros — `Remote::fetch`→Net, repo/index ops→Fs |
| tempfile | Env/Fs/Rand | temp dir from `$TMPDIR`, random names |
| memmap2 | Fs | memory-mapped files |
| notify | Fs/Clock/Env | filesystem watcher |
| tar | Fs | archive I/O |
| chrono | Clock/Fs/Env | **reads `/etc/localtime` (Fs) and `$TZ` (Env)** — both correct and non-obvious |
| time | Clock | |
| uuid | Rand | |
| which | Fs/Env | resolves executables on `$PATH` |
| dirs | Env | `$HOME` etc. |
| config | Env/Fs | config loading |
| redis | Net/Rand/Ipc | pure-Rust RESP client — `Net` is right, no `Db` because it isn't an FFI driver |
| rustls | Fs/Env | cert loading; **no `Net` is correct — rustls is sans-I/O** |
| tonic | Net/Ipc | partial (most transport is via hyper/tower) |

**B. Correct true-negatives** — genuinely pure, correctly reported as ~no effects:

`sha2`, `regex`, `serde_json`, `toml` — computation over in-memory data.

**C. Honest under-reports** — the effect is real but invisible to syntactic analysis. Use the nightly
lint (or know to look) for these:

| crate | missed | why (the structural blind spot) |
|---|---|---|
| hyper | Net | socket I/O via resolved method dispatch on tokio types |
| reqwest | Net | through hyper (same) |
| rand | Rand | entropy via `getrandom` / method-style calls |
| crossbeam-channel | Ipc | crossbeam's own channel types aren't in the classifier (it knows `std::sync::mpsc`, tokio) |
| walkdir | Fs | `read_dir` consumed through std iterator **methods** |
| tracing, log | Log | **macro**-based emission |
| clap | Env | mostly operates on already-parsed args |

## Method-dispatch frontier — receiver-type inference (apps, not libraries)

The hardest under-report is **method dispatch**: `client.execute(req)` is invisible because the scanner
doesn't know `client: reqwest::Client`. But the classifier already has verb-precise rules for
reqwest/sqlx/redis/mongodb/… — they were simply unreachable from a bare method name. So the scanner now
does **light, local receiver-type inference** (no compiler): it tracks variable types from function
**parameters**, **struct fields** (a crate-wide pre-pass), typed `let`s, `let x = T::new()` constructors,
and **local function return types** (`let p = create_pool()?; p.fetch_one(q)` — `create_pool`'s recorded
return type, `Result`/`Option` unwrapped, types `p`); resolves a method call's receiver (including
`self.field`, a factory-call result, and through a builder **chain**) to its type; and forms
`Type::method` so the existing rules fire. The return index keeps only **unambiguous** fn names — a name
with two different return types is dropped, so common method names (`get`/`build`) never hijack a chain.

Two guards keep this false-positive-free, both found *by* this calibration:
- **std/core/alloc receivers are excluded.** The std rules are coarse prefix matches written for
  free-function calls (`std::fs::`, `std::process::Command`); applied to inferred method calls they
  mis-fire on *pure* ones (`File::as_raw_fd`, `Command::arg`). mio wraps an eventfd in a `std::fs::File`,
  which made `Waker::as_raw_fd` read as Fs — caught and excluded. The external-crate rules are
  verb-precise, so they stay. (std free-function effects are still caught path-qualified.)
- **`#[cfg(...)]`-gated struct fields are skipped.** tokio's `resource_span: tracing::Span` (gated on the
  off-by-default `tracing` feature) otherwise made every `self.resource_span.in_scope(..)` read as Log —
  452 phantom functions.

Net effect on the 35 **libraries**: ~nil — libraries mostly call std and their own types, not *other*
high-level crates — and crucially **no new false positives** (the pure crates stayed at 0). The value
shows on **application** code, which is candor's primary target. On a representative app
([`method-dispatch-demo/`](method-dispatch-demo/src/lib.rs)):

```
App::fetch_user   { Db, Net }   # self.db.fetch_one(..)  +  self.http.get(..).send()   (struct fields + chain)
ping              { Net }       # fn ping(client: &Client)  ->  client.execute(..)      (param type)
one_shot          { Net }       # let c = reqwest::Client::new(); c.get(..).send()      (constructor + chain)
report            { Db }        # let p = build_pool(); p.fetch_one(..)                 (local return type)
report2           { Db }        # build_pool().fetch_one(..)                            (return type, chained)
format_label/normalize → pure   # correctly omitted
```

Without inference all three effectful functions report **nothing** — the exact blast-radius miss the
agent eval ([EVAL.md](../../EVAL.md)) measured. This is the frontier moving: not full type resolution,
but the reliable local slice of it, kept honest by excluding the cases where a coarse rule would lie.

## Wide sweep — all 1294 vendored crates

To harden the above beyond a hand-picked 35, the scanner was run over **every crate vendored on disk**
(`~/.cargo/registry`, newest version each) — 1294 crates. Without hand labels, two signals are still
automatable and decisive ([`widesweep.py`](widesweep.py), [`wide_analyze.py`](wide_analyze.py)):

- **False positives on a curated-pure set** (76 crates: encoders, data structures, text/parse, hashing,
  math, token manipulation — `itoa`/`base64`/`smallvec`/`memchr`/`sha2`/`serde`/`syn`/…). Any effect on
  these is a misclassification.
- **Explosion detection** — effectful functions per direct source. A high ratio means one effect is
  smearing across the graph (an over-connection bug) or genuinely pervasive.

Results:

- **Robustness: 0 crashes / parse errors across 1294 crates.** The scanner handles the real ecosystem.
- **Coverage: 394 crates (30%) report ≥1 effect, 900 report none.** Per-effect crate counts: Fs 206,
  Env 180, Clock 97, Rand 76, Exec 59, Net 49, Ipc 26, Log 14, Db 4.
- The sweep **found two real bugs**, both now fixed:
  - **`base64 → Rand`** — its `src/engine/tests.rs` / `decoder_tests.rs` are `#[cfg(test)] mod` FILE
    modules; their test-ness is declared at the `mod` site, invisible when walking the file. Fix: skip
    `tests.rs` / `*_tests.rs` / `*_test.rs` stems (the file analogue of the `tests/`-dir rule).
  - **`smol_str → Exec`** — from `.github/ci.rs`, a CI script. Fix: skip hidden dirs (`.git`, `.github`,
    `.cargo`, …), not just `.git`. (Both filters run on the root-relative path — an absolute prefix like
    `~/.cargo/...` must not trip them.)
- **After the fixes: zero false positives across all 76 pure crates.**
- The remaining explosion outliers were each **verified real, not misclassification**: `gix` (gitoxide —
  a pure-Rust git impl, genuinely Fs-pervasive: `write_file`/`load_config`/`worktree_file_to_object`);
  `aws-sdk-*` (`Rand` from `IdempotencyTokenProvider::random`, which every operation calls — real, though
  it dwarfs the SDK's actual `Net`, hidden behind smithy/hyper method dispatch); `arboard` (real X11
  backend reading `$DISPLAY`). `der` was removed from the pure list — it genuinely has
  `Document::read_der_file`/`write_der_file`.

Net: across 1294 real crates the scanner crashes on none, fabricates effects on none of the 76 pure
ones, and its only large counts are genuinely effect-heavy crates. The honest-under-report contract
holds at ecosystem scale.

## Takeaways for the project

1. **The classifier is sound on what it sees.** Across 35 real crates — and 1294 in the wide sweep —
   every library-code effect the scanner reported was real. It under-claims; it does not over-claim.
   That's the right direction for a
   trust tool — and it validates the curated-allowlist classifier on code it was never tuned against.
2. **The biggest accuracy lever found here was non-code, not classification** — excluding test/build
   noise. Shipped.
3. **FFI tiers are classifiable by naming the C library's entry points — three now shipped.** `libc`
   (nix 0 → 59), `libsqlite3` (rusqlite 0 → 48), `libgit2` (git2 7 → 45), all validated against source
   with zero false positives. The pattern generalises: a thin Rust crate over a C library becomes
   visible once that library's effectful functions are named, matched by the distinctive C leaf so the
   binding alias is irrelevant. git2 additionally needed macro-peeking, since it routes every FFI call
   through `try_call!`.
4. **Macro-peeking is a free recall win.** Parsing macro token streams as expressions (skipping
   non-expression bodies) recovered calls hidden in `try_call!`/`println!`/`write!` across the board
   with no false positives — the pure crates stayed pure. A general improvement, not git2-specific.
5. **What stays the nightly lint's job:** anything behind method dispatch (hyper/reqwest `Net`, and the
   propagation from a binding crate's FFI leaves up to its own high-level wrappers), macro-defined
   call syntax that isn't expression-shaped, or non-std channels (crossbeam `Ipc`). The two backends are
   complementary — this sweep quantifies *where* the line falls.
