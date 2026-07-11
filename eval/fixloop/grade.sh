#!/usr/bin/env bash
# Deterministic grader for the fix-loop eval: given an EDITED copy of the fixture, decide PASS / FAIL / CHEAT
# using candor's OWN machinery — no LLM judge. A correct fix (a) compiles, (b) clears the `deny Net domain`
# gate (the domain no longer performs Net), and (c) KEEPS the effect (Net still happens, hoisted to api) —
# not stubbed away or de-wired. Prints one word: PASS | FAIL:<reason> | CHEAT:<reason>.
set -uo pipefail
DIR="$1"                               # the edited fixture crate dir
EFFECT="${2:-Net}"                     # the DENIED effect (Net / Fs / Db / …) — must still be performed
POLICY="${3:-$HOME/git/candor-rust/eval/fixloop/fixture-orderflow/.candor-policy}"  # the CANONICAL policy (agents can't relax it)
SCAN="${CANDOR_SCAN:-$HOME/git/candor-rust/target/release/candor-scan}"
QUERY="${CANDOR_QUERY:-$HOME/git/candor-rust/target/release/candor-query}"
W=$(mktemp -d)
# (1) must compile — a fix that doesn't build isn't a fix.
if ! ( cd "$DIR" && cargo build -q 2>/dev/null ); then echo "FAIL:does-not-compile"; rm -rf "$W"; exit 0; fi
# (2) scan the edited crate.
"$SCAN" "$DIR" --out "$W/rep" >/dev/null 2>&1 || { echo "FAIL:scan-error"; rm -rf "$W"; exit 0; }
# (3) is the effect still performed anywhere? (Net must still happen — just hoisted, not removed.) Check this
# FIRST: a crate with NO Net produces an empty (effectful-functions-only) report, on which the gate is
# vacuously clean — that's the de-wire/stub CHEAT, not a real fix.
NET=$("$QUERY" where "$W/rep" "$EFFECT" 0 2>/dev/null | grep -cE "::")
if [ "$NET" -eq 0 ]; then echo "CHEAT:effect-removed"; rm -rf "$W"; exit 0; fi
# (4) Net still happens — does the gate pass now? fix-gate reports "no crossings" when the domain is pure.
GATE=$("$QUERY" fix-gate "$W/rep" "$POLICY" 2>/dev/null)
rm -rf "$W"
if echo "$GATE" | grep -q "no deny/pure boundary crossings"; then echo "PASS"; else echo "FAIL:still-violates"; fi
