# candor-scan calibration on 35 real third-party crates

**What this is.** The stable scanner (`candor-scan`) is syntactic — it sees what's *written*, not what
the compiler *resolves*. This is the first calibration of how accurate it actually is, run against
**real, popular crates** (not candor's self-made fixtures), made possible by the fact that the scanner
parses source and never builds — so it runs on anything already vendored under `~/.cargo/registry`.

**How.** 35 crates spanning every effect category (HTTP, async runtimes, databases, filesystem,
crypto, time, parsing, channels, logging). For each, the newest version on disk was scanned with
`candor-scan <dir> --json` and the per-effect function counts tabulated. Reproduce with
[`sweep.py`](sweep.py). Raw output: [`results-notest.json`](results-notest.json). 2 crates (ureq, zip)
weren't on disk.

This run **drove two scanner fixes** (below); the numbers here are post-fix.

## Headline

After the fixes, on these 35 crates the scanner has **no remaining false positives in library code** —
every effect it reports is real. All its errors are now **under-reports**, and they cluster on exactly
the three things a syntactic backend structurally cannot see: **FFI**, **dynamic method dispatch**, and
**macros**. That is the honest, expected failure mode, and it's documented per-crate below.

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

## Results — accuracy by category

**A. Accurate** — reported effects match the crate's real, syntactically-visible surface:

| crate | effects | note |
|---|---|---|
| tokio | Fs/Net/Clock/Ipc/Rand/Log | async runtime; its real syscalls are path-qualified (std/mio/libc-free paths) |
| mio | Net/Fs/Ipc | low-level non-blocking I/O |
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
| rusqlite | Db | libsqlite3 **FFI** |
| git2 | Net | libgit2 **FFI** |
| nix | Fs/Net/Exec/Ipc | thin **FFI** over `libc::*` (the classifier has no `libc`) |
| rand | Rand | entropy via `getrandom` / method-style calls |
| crossbeam-channel | Ipc | crossbeam's own channel types aren't in the classifier (it knows `std::sync::mpsc`, tokio) |
| walkdir | Fs | `read_dir` consumed through std iterator **methods** |
| tracing, log | Log | **macro**-based emission |
| clap | Env | mostly operates on already-parsed args |

## Takeaways for the project

1. **The classifier is sound on what it sees.** Across 35 real crates, every library-code effect the
   scanner reported was real. It under-claims; it does not over-claim. That's the right direction for a
   trust tool — and it validates the curated-allowlist classifier on code it was never tuned against.
2. **The biggest accuracy lever found here was non-code, not classification** — excluding test/build
   noise. Shipped.
3. **The highest-value classifier extension is `libc`.** Three of the under-reports (nix, and the FFI
   half of rusqlite/git2) collapse to "we don't model `libc`." A modest `libc::{open,read,write,…}→Fs`,
   `{socket,connect,bind}→Net`, `{fork,execve}→Exec` table would light up the whole FFI-thin tier.
   Candidate next step.
4. **What stays the nightly lint's job:** anything behind method dispatch (hyper/reqwest `Net`), macros
   (tracing `Log`), or non-std channels (crossbeam `Ipc`). The two backends are complementary, exactly
   as documented — this sweep quantifies *where* the line falls.
