#!/usr/bin/env bash
# KNOWN, TRIAGED under-reports — the ONE allowlist mechanism for BOTH syscall oracles.
#
# Why one file. `run.sh` (program-level verdict) carried a `KNOWN_UNDER` list; `pf/run_pf.sh`
# (per-function verdict) carried none, so three correct reds against genuinely-open SOUNDNESS rows
# took the per-function disclosure-recall calibration OFFLINE rather than reporting a known gap
# (SOUNDNESS R102 — the §H aggregation shape: the detector works, the aggregator discards it).
# The repair is not a second list; two harnesses answering one question is how this family produced
# its worst defects. Both oracles source THIS file and call THESE functions.
#
# Intent, unchanged from run.sh's original comment: tracked so the oracle is a clean gate — green on
# known gaps, red only on NEW findings. An entry is a DEBT, not a dismissal: each names the SOUNDNESS
# row it is open against and the mechanism, so a reader hitting the suppression can find the row.
#
# ENTRY FORMAT:  <driver>|<SOUNDNESS row>|<one-line mechanism>
#
# THE RATCHET. An allowlist that only speaks in the failing branch is a gate that can never go red
# again: the day the underlying defect is fixed, the entry stays, and the NEXT regression in that
# driver is absorbed forever. So `known_under_ratchet` inspects every entry against what the run
# actually observed, and FAILS the oracle when an entry is stale (allowlisted driver PASSED) or dead
# (names no driver the oracle ran). A skipped driver is no evidence either way and only warns —
# it must not be able to turn a transient build flake into a red, nor be silently read as still-open.

# ---------------------------------------------------------------------------------------------
# The lists. Keep an entry ONLY while its row is open; deleting the row's fix and the entry in the
# same change is the point of the ratchet.
# ---------------------------------------------------------------------------------------------

# Program-level oracle (soundness/realworld/run.sh). Empty: both builder-chain under-reports this
# oracle found (duct cmd!()→run, ureq get()→call) are FIXED by the GENERALIZED
# scan_builder_entry_effect table in candor-scan (over-approximate the entry for the syntactic
# engine; the deep engine types the terminal verb and stays precise). New verb-keyed crates that
# under-report get a table row + leave this empty.
KNOWN_UNDER_PROGRAM=()

# Per-function oracle (soundness/realworld/pf/run_pf.sh). Three drivers were added at candor-rust
# `1aeeaba` as RUNTIME WITNESSES for rows that are open and stay open: they exist to be red, so that
# the day the engine is fixed the ratchet below forces the entry out with the fix.
#
# THE RATCHET HAS NOW DONE THAT THREE TIMES ON REAL FIXES, which is the only evidence it works on one
# rather than on the six seeded cases it was proven against. R99's two stated-open shapes were closed;
# both drivers went green; this file printed `✗ STALE ALLOWLIST ENTRY` for each and exited 1; the two
# entries left in the same commit as the fix. `pf_oncelock_cb` (R101) went the same way one commit later.
#
# NOW EMPTY, AND THE EMPTINESS IS THE POINT. It is the completion criterion this list was built with:
# every driver the per-function oracle runs is now falsifiable, so the recall denominator is the whole
# driver set (28/28) and NOTHING is suppressed. A red from here on is a NEW finding by construction —
# there is no entry left that could absorb one. Keep it that way: an entry added here is a DEBT with a
# row number, not a way to quiet a driver, and the ratchet will demand it back the day the row closes.
KNOWN_UNDER_PERFN=()

# ---------------------------------------------------------------------------------------------
# The mechanism. Both oracles use these and only these.
# ---------------------------------------------------------------------------------------------

# known_under_lookup <driver> <entry>...
#   Prints "<row>|<mechanism>" and exits 0 when <driver> is allowlisted; exits 1 otherwise.
known_under_lookup() {
  local want="$1"; shift
  local e
  for e in "$@"; do
    case "$e" in
      "$want|"*) printf '%s\n' "${e#*|}"; return 0 ;;
    esac
  done
  return 1
}

# known_under_ratchet <status-lines> <entry>...
#   <status-lines> is one "<driver> <known|pass|skip|fail>" per line, for every driver the oracle
#   actually adjudicated this run. Prints findings; exits non-zero if any entry is STALE or DEAD.
#
#   known -> the entry did its job this run. Silent.
#   pass  -> STALE. The gap looks closed and the suppression is now load-bearing for nothing except
#            hiding the next regression. RED, with the remedy named.
#   skip  -> no evidence either way (build failure, effect did not execute). Warn only: a transient
#            skip must not manufacture a red, and must not be read as "still open" either.
#   (absent) -> DEAD. The entry matches no driver this oracle ran — a typo or a deleted driver. An
#            entry that can never match suppresses nothing and hides its own rot. RED.
known_under_ratchet() {
  local statuses="$1"; shift
  local bad=0 e drv rest row why st
  for e in "$@"; do
    [ -n "$e" ] || continue
    drv="${e%%|*}"; rest="${e#*|}"; row="${rest%%|*}"; why="${rest#*|}"
    st="$(printf '%s\n' "$statuses" | awk -v d="$drv" '$1==d {print $2; exit}')"
    case "$st" in
      known) : ;;
      pass)
        echo "  ✗ STALE ALLOWLIST ENTRY: $drv is on KNOWN_UNDER (SOUNDNESS $row) but PASSED this run."
        echo "      The gap it tracks reads CLOSED. Remove the entry in the same change as the fix —"
        echo "      while it is listed, the next regression in $drv is absorbed silently and this"
        echo "      oracle can never go red on it again."
        echo "      entry: $drv|$row|$why"
        bad=$((bad+1)) ;;
      skip|broke)
        echo "  ⚠ UNVERIFIED ALLOWLIST ENTRY: $drv (SOUNDNESS $row) did not reach a verdict this run"
        echo "      ($st) — no evidence either way, so it is neither confirmed still-open nor stale."
        echo "      (A driver that did not RUN is red on its own account; see the oracle's summary.)" ;;
      fail)
        # Cannot happen: a failing allowlisted driver is reported `known` by both oracles. Stated so a
        # future refactor that breaks the coupling is loud rather than silently un-ratcheted.
        echo "  ✗ ALLOWLIST NOT APPLIED: $drv (SOUNDNESS $row) failed but was reported as a NEW finding."
        bad=$((bad+1)) ;;
      *)
        echo "  ✗ DEAD ALLOWLIST ENTRY: $drv (SOUNDNESS $row) matches no driver this oracle ran."
        echo "      An entry that can never match suppresses nothing and hides its own rot: fix the"
        echo "      driver name or delete the entry."
        bad=$((bad+1)) ;;
    esac
  done
  [ "$bad" -eq 0 ]
}
