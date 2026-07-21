# FROZEN confirmatory corpus — Rust (syscall arm)

The Rust analog of `candor-java/eval/corpus-confirmatory`. The **mechanism is already proven and CI-green**:
`soundness/realworld/run.sh` runs real crates under `strace` and checks candor-scan's prediction against the
kernel (program-level); `soundness/realworld/pf/run_pf.sh` does the **per-function** version (reconstructs
the call stack at each effect syscall from `CFE/CFX` markers and checks every on-stack function carries the
effect or discloses `Unknown` — validated 4/4 honest, 2026-07-20). This directory adds the **frozen,
pre-registered** discipline on a **held-out** crate corpus.

> Status: **harness + pre-registration authored; the frozen run over this manifest is NOT yet executed**
> here — the Rust arm needs Linux + `strace` (Docker on the author's macOS was too CPU-starved to build
> candor-scan in reasonable time alongside other containers). It is written to run on a Linux CI runner or a
> non-loaded Linux host, where the mechanism it reuses is already green. No Rust confirmatory *result* is
> claimed until it runs; the JVM arm is the one executed frozen result.

## What is (to be) frozen, in one commit before the run

- **Engine:** `candor-scan` (the deployed stable engine), built once and pinned by `sha256` — `run_frozen.sh`
  aborts on a hash mismatch (the hash is filled in on the target at freeze time; left as `PENDING` here
  precisely because it must be the *Linux* binary's hash, computed where the run happens).
- **Corpus:** `manifest.tsv` — held-out crates (version-pinned), **excluding every crate used to calibrate
  or A/B the classifier** (`std`/`ureq`/`minreq`/`socket2`/`duct`/`xshell`/`subprocess`/`fs-err`/`fs-extra`/
  `tempfile`/`walkdir`/`glob`/`memmap2`/`filetime`/`zip`/`libc`/`syn`/`tokio`/`hyper`/`serde_json`/`clap`/
  `reqwest`/`h2`/`flate2` — see `soundness/SOUNDNESS-LOG.md`).
- **Protocol:** for each crate — `candor-scan` the source → per-function report; build the crate's **own
  test binary** (`cargo test --no-run`) and run *that binary* under `strace -f` (never `cargo` itself,
  whose compiler syscalls would pollute); map observed `openat`/`connect`/`execve`/`unlink` to effect
  classes `Fs`/`Net`/`Exec`; **program-level H** — every observed class must be named by some crate function
  or disclosed `Unknown`; a `-k` (kernel-unwind) refinement upgrades to per-function where debug symbols
  demangle. Report per crate: analyzed / observed-classes / covered / **violation** (an observed class no
  function names or discloses).
- **Acceptance:** zero *undisclosed* observed-effect classes (a program-level silent under-report is the
  cardinal sin); disclosed `Unknown` is a pass.
