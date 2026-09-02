# shellcheck shell=bash
# _lib.sh — the ONE locator + liveness primitive for the Bet 3 teeth scripts.
#
# Why this file exists (measured 2026-09-02, R108). verify.sh and verify-layering.sh each carried
#
#     LIB=".../target/debug/$(basename "$(find .../target/debug -maxdepth 1 \
#            -name 'libcandor@*.dylib' -o -name 'libcandor@*.so' | head -1)")"
#     [ -e "$LIB" ] || { echo "FAIL: no candor dylib (run cargo build first)"; exit 1; }
#
# which has two defects, and BOTH of them fail in the miss direction:
#
#   1. PLATFORM-BLIND, ORDER-DEPENDENT SELECTION. The glob accepts a foreign-ABI library and `head -1`
#      takes whatever the directory happens to list first — not the newest, not the host's. A Linux
#      `libcandor@…-unknown-linux-gnu.so` left in `target/debug` by a container leg made `cargo dylint`
#      refuse the lib path on macOS, so candor never ran and neither AS-EFF-008 nor AS-EFF-009 fired.
#   2. AN UNFIREABLE GUARD. When `find` matches nothing, `basename ""` is "" and `$LIB` becomes the
#      DIRECTORY `…/target/debug/` (or `/` when target/ does not exist) — both of which `-e` accepts.
#      So "no candor dylib" could never print; an unbuilt tree produced the same silent miss.
#
# The consequence is what makes this worth a file rather than a one-line edit: with candor not running,
# the positive assertion fails AND the paired absence control ("the allowed host is not flagged", "the
# pure function is not flagged") PASSES — because absence is exactly what a dead instrument produces.
# A reader then sees "the harness is live, the detection is broken", which is the inverse of the truth.
# So the locator is fixed here once, and `assert_live` makes the absence controls unable to pass over a
# run that did not happen.

# Newest host-ABI candor lint library in <dir>, or empty. Host extension only: handing `cargo dylint` a
# foreign-ABI library is not a degraded run, it is no run. mtime, not name order: the toolchain is IN the
# filename, so a stale `libcandor@nightly-2025-…` would otherwise sort ahead of a fresh 2026 build.
candor_lib() {
  local dir="$1" ext newest="" f
  case "$(uname -s)" in Darwin) ext="dylib" ;; *) ext="so" ;; esac
  [ -d "$dir" ] || return 0
  for f in "$dir"/libcandor@*."$ext"; do
    [ -f "$f" ] || continue
    if [ -z "$newest" ] || [ "$f" -nt "$newest" ]; then newest="$f"; fi
  done
  printf '%s' "$newest"
}

# Resolve or die. Prints the path on stdout; every diagnostic goes to stderr so `$(...)` stays clean.
require_candor_lib() {
  local dir="$1" lib
  lib="$(candor_lib "$dir")"
  if [ -z "$lib" ]; then
    echo "FAIL: no candor lint library for $(uname -s) in $dir — run \`cargo build\` in the candor repo first" >&2
    return 1
  fi
  printf '%s' "$lib"
}

# The instrument must be proven to have RUN before any absence assertion is credited. dylint reports a
# bad/unloadable lib path as `Error: …` and checks nothing; a crate that WAS linted always leaves cargo's
# own INDENTED `    Checking <crate>` / `    Finished` progress in the captured stream. The indentation
# matters: dylint's own unindented `Checking with toolchain …` preamble prints even on the runs that go on
# to load nothing, so matching it would re-open the hole. Either signal missing means the run did not
# happen, and a "not flagged" verdict below would be measuring nothing.
# BOTH checks below are ANCHORED, so BOTH are broken by ANSI colour — and they break in opposite
# directions, which is why the strip happens once, here, before either runs.
#
# Measured 2026-09-02, on the push that shipped this helper: CI went RED with "cargo never reported
# checking a crate" while the very next lines of its own output read
# `\e[1m\e[92m    Checking\e[0m httpkit v0.1.0`. GitHub's runner makes cargo colourise, so the escape
# precedes the indentation and `^ +` cannot match. Locally cargo emits plain text and it passed — the
# instrument behaved differently in the two environments, which is the property an instrument must not
# have.
#
# The false FAIL is the SAFE direction and is merely how it was noticed. The dangerous one is the
# `^(Error|error):` arm above it: a COLOURISED dylint refusal would not match, `assert_live` would
# return 0, and every absence assertion below it would be credited over a run that loaded nothing —
# reopening R108 exactly, in CI only, where nobody reads the log of a green job.
strip_ansi() { sed $'s/\033\[[0-9;]*[a-zA-Z]//g'; }

assert_live() {
  local what="$1" out="$2"
  # Strip once; both greps below keep their anchors, so the indentation discrimination the comment
  # above depends on (cargo's indented progress vs dylint's unindented preamble) is unchanged.
  local clean
  clean="$(printf '%s\n' "$out" | strip_ansi)"
  if printf '%s\n' "$clean" | grep -qE '^(Error|error):'; then
    echo "FAIL: $what — candor did not run (dylint refused the lint library); every absence check below would be vacuous" >&2
    printf '%s\n' "$clean" | grep -E '^(Error|error):' | head -3 >&2
    return 1
  fi
  if ! printf '%s\n' "$clean" | grep -qE '^ +(Checking|Finished) '; then
    echo "FAIL: $what — cargo never reported checking a crate; the instrument did not run" >&2
    printf '%s\n' "$clean" | head -5 >&2
    return 1
  fi
  return 0
}
