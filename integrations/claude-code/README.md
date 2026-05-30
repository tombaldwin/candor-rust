# candor × Claude Code

The problem this solves: when you paste "use candor" into a coding agent, **you have no way to
know whether the agent actually ran it, whether it worked, or whether its answer is trustworthy.**
Everything routes through the agent's narration, and narration can't be verified.

This integration gives *you* (the human) a deterministic, un-fakeable signal instead — a one-line
**receipt** that appears in your transcript whenever your Rust code changes:

```
candor · 143 fns · 54 Db, 16 Net, 27 Fs, 21 Exec · 0 unresolved · fresh @8c4c9053 · coverage ✓ · report: .candor/report.*.json
```

It refreshes automatically every turn you touch Rust, and it is honest about its own freshness and
its own blind spots.

## What you get

| Surface | Trigger | You see |
|---|---|---|
| **`/candor`** slash command | you type it | the receipt, on demand, regenerated |
| **Stop hook** (auto-refresh) | end of any turn that changed `.rs` files | the receipt via a `systemMessage` — and **silence** on turns that didn't |
| **`.candor/report.*.json`** | every run | the full machine-readable effect map, on disk, inspectable |

### Why it's trustworthy

The receipt is produced by a shell script (`candor-run.sh`) that the Stop hook runs **directly** —
not through the model. The agent cannot fabricate it, skip it, or misreport it. The hook surfaces
the line to you through Claude Code's `systemMessage` channel (not the agent's prose). What you read
is what candor actually computed.

### Freshness is visible, never silent

Every receipt carries an 8-char content hash of your Rust sources (`@8c4c9053`). The states:

- **`fresh @hash`** — re-analyzed this turn; the map matches your code now.
- **`current @hash (no Rust change)`** — nothing changed; the existing map is still exact.
- **`⚠ STALE — sources changed but the crate did not compile`** — candor analyzes compiled HIR, so
  mid-broken-refactor it *can't* refresh. It says so and keeps the last good map, rather than
  pretending. Fix the build and run `/candor`.

A stale-but-labeled map is honest; a stale-but-fresh-looking one is the lie this whole project
exists to prevent.

### Coverage is visible too

candor is only as complete as its classifier. The receipt checks your `Cargo.toml` against candor's
calibrated crate set and warns when a dependency *looks* effectful but has no rule:

```
… · ⚠ coverage: scylla,lapin uncalibrated — Db/Net may be incomplete for code using them · …
```

This is the safeguard against the failure mode that motivated the integration: candor once reported
4 of 20 DB call sites on a real app because a driver wasn't calibrated, with no signal anything was
missing. Now the gap is surfaced. (The coverage check is a curated heuristic — it nudges, it does
not certify. Treat a warning as "read the source for code using that crate.")

## Install

From the Rust project you want instrumented:

```sh
/path/to/candor/integrations/claude-code/install.sh
```

It installs the scripts under `.claude/candor/`, the `/candor` command under `.claude/commands/`,
merges a `Stop` hook into `.claude/settings.json` (non-destructively — existing hooks are kept), and
pins the candor dylib path in `.candor/config`. Re-running is safe and idempotent.

**Prerequisite:** a built candor dylib in the clone. Either build candor (`cargo build` / `cargo
candor update` in the clone) or set `CANDOR_HOME`/`CANDOR_LIB` in `.candor/config`. `python3` is used
for JSON handling and recommended; without it the receipt degrades to a plain function count.

## Updates — the clone is the single source of truth

The install does **not** copy the scripts into your project — it installs thin **stubs** that delegate
to the clone (`$CANDOR_HOME/integrations/claude-code/…`, pinned in `.candor/config`). So updating
everything is one step in the clone:

```sh
cargo candor update           # = git pull --ff-only + cargo build, in the clone
```

That pulls the **engine, these scripts, and `AGENTS.md` at the same commit** — nothing in your
project drifts independently. Because candor isn't on crates.io (it depends on `clippy_utils` via
git), the clone *is* the distribution; this just makes refreshing it atomic.

The receipt is honest about its own version, the same way it's honest about freshness and coverage:

- every line is stamped `candor @<sha>` — the exact engine commit that produced this map;
- if the **dylib is older than the engine source** (you pulled but didn't rebuild), it appends
  `⚠ dylib older than source — rebuild: cargo candor update` (cheap mtime check, every run);
- `/candor` additionally does a network check and appends `⚠ candor update available` when the clone
  is behind its remote (the per-turn Stop hook stays offline and fast, so it never blocks on git).

So the doc, the binary, and the report you're reading all declare the same version — they can't
silently desync.

After install, the clone's local `AGENTS.md` is your copy of the agent instructions (auditable,
offline, versioned by the same `cargo candor update`). Point your project's `CLAUDE.md` at it instead
of the GitHub URL if you want the instructions to update in lockstep with the engine.

## Honest limits

- **It does not measure whether *this session* was cheaper or faster.** Per-session agent uplift
  isn't observable from inside one session — that needs a deliberate A/B benchmark, not a live
  meter. The receipt tells you candor ran and what it found; it makes no efficiency claim.
- **It can't prove the agent *relied* on the report** vs. re-deriving from source. It proves the map
  exists, is current, and what's in it. Making the report authoritative is a workflow choice (read
  `.candor/report.*.json`), not something a hook can force.
- **Auto-refresh adds latency** to turns that changed Rust (a `cargo dylint` run — seconds to tens of
  seconds on a real crate). Turns that didn't touch Rust cost nothing. To make it asynchronous
  instead, run the hook in the background and read the prior turn's receipt — a future option.
- **Coverage is heuristic.** The receipt's `Cargo.toml`-based check is a best-effort nudge, not a
  certificate. Its calibrated crate list lives in `candor-run.sh` alongside the engine in the same
  clone, so the two no longer drift *across installations* (one `cargo candor update` moves both) —
  but keeping the two lists in sync within the repo is still a maintainer chore. The fully
  authoritative version (candor emitting the crates it encountered-but-couldn't-classify) is a
  planned engine upgrade.

## Uninstall

Remove `.claude/candor/`, `.claude/commands/candor.md`, the `Stop` entry naming `stop-hook.sh` from
`.claude/settings.json`, and the `.candor/` directory.
