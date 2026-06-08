# Real-world validation — candor's policy + detail features on non-fixture code

The Bet 3 features (literal allowlists, layering) and the Bet 4 detail extraction were proven on
fixtures. This is a check against **real, unplanned code**: candor's own workspace (`candor` the lint,
`candor-report`, and the `build.rs` build script) — genuine application code with real Net/Fs/Exec/Env
surface, not written to exercise candor. Guaranteed-buildable (it's the repo), so no toolchain risk.

## What worked (confirmed on real code)

- **Audit surfaces real effects.** `cargo candor audit` over the workspace reports 11 effectful
  functions — `main { Exec Fs }`, `<Candor>::check_crate_post { Env Fs Log }`, `Candor::new { Env Fs }`,
  `load_baseline`/`load_cross_reports`/`load_layer_reach { Fs }` — all correct.

- **Exec detection + command-literal extraction.** candor flags that `build.rs`'s `main` performs
  `Exec`, and extracts the exact command: `cmds = ["git"]`. Verified against source — `build.rs` does
  `Command::new("git").args(["rev-parse","--short","HEAD"])`. Accurate, no false command.

- **Fs path-literal extraction.** candor reports `paths = [".git/HEAD", "rust-toolchain",
  "rust-toolchain.toml"]` for the same function — and these are exactly the three
  `std::fs::read_to_string(...)` calls in `build.rs` (not the `println!("cargo:rerun-if-changed=…")`
  directives, which are correctly NOT attributed, since extraction is gated to `Fs`-classified calls).

- **The `allow Exec` allowlist (AS-EFF-008) works on real code.** `allow Exec in main git` → passes
  (git is the only command). `allow Exec in main rustc` → flags `main reaches { git } outside the
  allowlist`. So the supply-chain boundary certifies a real build script's subprocess surface.

## Usability findings (genuine sharp edges, surfaced by real names)

These aren't bugs — the behaviour is exactly `scope_matches` — but real crate/module names exposed two
gotchas worth documenting (and candidates for a future ergonomic pass):

1. **A crate's own functions don't carry the crate name in their path — FIXED.** Inside the `candor`
   crate, a function's `def_path_str` is `load_cross_reports` / `Candor::new`, *not*
   `candor::load_cross_reports`. So a layering rule whose `from` scope was a **crate name**
   (`forbid candor -> candor_report`) matched none of that crate's own functions and was a **silent
   no-op** — exactly the false-security the trust contract opposes. **Fixed in this change:** policy
   scope matching now runs against the crate-prefixed path (`<crate>::<path>`), so a crate-name scope
   matches the crate's own functions while module/type-name scopes still match (the segment is present
   either way). Verified on candor itself — `forbid candor -> candor_report` now correctly flags
   `check_crate`/`load_baseline`/`load_cross_reports` reaching into `candor_report` (a real,
   intentional dependency). Integration test §9d guards it; reverting the prefix re-opens the no-op.

2. **Scope matching is segment-prefix and case-sensitive.** `candor` is a prefix of `candor_report`, so
   a scope `candor` also matches `candor_report::…`; and `Candor` (the impl type) does *not* match a
   scope spelled `candor`. Both are predictable from the rule (`name.split("::").any(|s|
   s.starts_with(scope))`) but easy to trip over. Prefer distinctive scope names that aren't prefixes
   of a sibling.

## Not done (and why)

- **An external crates.io crate** was attempted (`which`, a small PATH-lookup crate) but the sandbox
  correctly **refused to clone + build untrusted external code** (its build scripts / proc-macros would
  execute). That's the right call; it needs explicit user authorization to run, so it's left as a
  follow-up rather than worked around. The candor dogfood above is genuine non-fixture code and covers
  the same ground (real Exec/Fs/Env + literal extraction + a real allowlist verdict).

## Bottom line

The new detail extraction and the `allow Exec` enforcement are **accurate on real, unplanned code** —
the command and paths candor reports for `build.rs` match the source exactly, and the allowlist gives
the right verdict. The one rough edge is scope-name ergonomics for `layering` (crate-name `from` scopes
silently match nothing); documented above and a clean target for a future usability improvement.
