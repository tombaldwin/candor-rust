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
  demangle.
- **Acceptance:** zero *undisclosed* observed-effect classes (a program-level silent under-report is the
  cardinal sin); disclosed `Unknown` is a pass.

### Columns emitted (`results/FROZEN-SUMMARY.tsv`)

`crate  tag  observed_raw  observed_crate  named  unknown_only  violations  level  verdict`

- **`observed_raw`** — every effect class the kernel emitted under the strace harness. **This is the set
  the H-violation check runs on.**
- **`observed_crate`** *(informational)* — `observed_raw` minus a measured **harness baseline**. Before the
  corpus loop the harness compiles a throwaway crate with one `#[test] fn noop(){}`, runs *its* test binary
  through the identical strace pipeline, and records the effect classes the runner itself produces (libtest
  opens files → `Fs`; the runtime may open a control socket → `Net`; the parallel runner may spawn → `Exec`).
  Subtracting that gives a coverage story about the crate's *own* effects rather than the runner's.
  **⚠ This column NEVER gates.** Subtracting the baseline from the *checked* set could delete a class that is
  both a harness artifact and a genuine crate effect — that would hide a real under-report (the cardinal
  sin). Over-observation is the safe direction, so the violation check stays on `observed_raw`. A loud
  banner + an in-line `SOUNDNESS:` comment in `run_frozen.sh` pin this; the Python iterates `observed_raw`.
- **`named`** *(strong coverage)* — observed_raw classes some crate function's `inferred` set *literally*
  contains. This is real, function-attributed coverage.
- **`unknown_only`** *(weak coverage)* — observed_raw classes covered *only* by a disclosed `Unknown`
  somewhere in the report. Honest, but near-vacuous (every crate has at least one `Unknown`). Splitting
  `named` from `unknown_only` de-vacuums the old "covered" column: it says *how strongly* each class is held.
- **`violations`** — observed_raw classes NO function names AND NO function discloses `Unknown`. The cardinal
  sin. Empty = H holds.
- **`level`** — `perfn` when the `-k` kernel stacks reconstructed and at least one on-stack frame demangled
  to a reported crate function (a true per-function check ran, like the JVM arm); `program` when they did
  not (honest fallback — we never *claim* per-function where we only have program-level).

### Per-function `-k` check (best-effort, honest fallback)

Alongside the program-level pass, each test binary is *also* traced with `strace -k`, which appends the
kernel stack unwind after every effect syscall. We demangle the Rust frames (legacy `_ZN…E` decoded in
Python — path components length-prefixed, trailing `h…` hash dropped; v0/`_R…` and already-demangled
`crate::mod::fn` fall to a last-`::`-segment leaf), keep only frames whose leaf matches a function
`candor-scan` reported for *this* crate (std/libc/other-crate frames aren't ours to check), and assert
per-function H for each: the on-stack crate function must name the syscall's class or disclose `Unknown`.
A reported crate function demonstrably on the stack at an effect it reads pure and does not disclose is a
**per-function** silent under-report (`PF-VIOLATION`), attributed to the exact function — this is the
transitive check the JVM arm does. When no effect stack yields an attributable crate frame (symbols
stripped, no frame pointers), the crate records `level=program` and only the program-level verdict stands;
we never assert per-function on a program-level-only run.

## Result (GitHub CI, ubuntu-latest, 2026-07-21)

5/5 held-out crates: every observed effect class (Fs/Net/Exec, kernel-witnessed) is **covered** by
candor-scan (named or `Unknown`-disclosed). **0 program-level false all-clears; H holds on all.** The
table below is the earlier run's program-level view; the harness now additionally emits `observed_crate`
(baseline-subtracted, informational), the `named` / `unknown_only` split, and a per-function `level` (see
the *Columns emitted* section) — those richer columns land on the next CI run of this commit.

| crate | tag | observed_raw | named | unknown_only | violations |
|---|---|---|---|---|---|
| sysinfo | v0.31.4 | Fs,Net,Exec | Fs | Net,Exec | 0 |
| tar | 0.4.46 | Fs,Net,Exec | Fs | Net,Exec | 0 |
| notify | notify-6.1.1 | Fs,Exec | Fs | Exec | 0 |
| os_pipe | 1.2.1 | Fs,Net,Exec | Fs | Net,Exec | 0 |
| tiny_http | 0.12.0 | Fs,Net,Exec | Fs,Net | Exec | 0 |

(The `named` / `unknown_only` split above is illustrative of the new columns' shape — the crate's own Fs
is function-named; Net/Exec that come mostly from the libtest runner are typically held only by a disclosed
`Unknown`. Exact values are whatever the CI run records.)

Caveat, stated plainly: the primary check is **program-level** on `observed_raw`, not per-function — the
`-k` per-function upgrade runs best-effort and falls back to `program` when Rust frames don't demangle. The
`libtest` harness inflates `observed_raw` (it opens files, sockets for the runner, and spawns for
parallelism); `observed_crate` reports it with the measured baseline removed, but **only informationally** —
the violation check reads `observed_raw`, because over-observation is the safe direction (it can only make a
class easier to cover, never hide a real miss) whereas baseline-subtracting the checked set could mask a
real effect that shares a class with a harness artifact. The mechanism-independent kernel oracle is the
strength; the coarseness is measured and reported, not hidden.
