#!/usr/bin/env python3
"""DISCLOSURE-RECALL calibration — falsify candor's report IN PLACE so a known cardinal sin is injected.

Why this exists: the syscall oracle reporting "0 violations" is only evidence that H held if the oracle
COULD have reported one. That is not self-evident — a marker that never fires, a report path that resolves
to nothing, a verdict that accepts any non-empty claim would each produce a silent, permanent green. So we
seed the exact defect the oracle exists to catch and require it to turn red.

The mutation is applied to candor's OUTPUT, downstream of the analyzer and upstream of the verdict, which
is precisely where a false all-clear lives: the program still runs the effect under strace, but the
signature now denies it. Nothing else in the pipeline is stubbed, so what gets calibrated is the real
instrument (build -> run -> trace -> scan -> verdict), not a re-implementation of it.

Modes:
  silent  the canonical false all-clear — every effect AND every disclosure flag stripped, so candor
          claims sound-complete purity of a frame that demonstrably issues syscalls.
  wrong   a decoy effect (Rand, used by no driver) replaces the real one, disclosure still stripped.
          Separates "the verdict checks for THE effect" from "the verdict checks for any effect at all";
          a checker that only tested non-emptiness would pass `silent` and fail only here. On a report with
          no function entries (a wholly pure program) the claim is synthesized rather than edited, so the
          pure control is falsifiable too and the fabrication branch of the verdict gets exercised.

Usage: mutate_report.py <silent|wrong> <report.json>
"""
import json, sys

FLAGS = ("unresolved", "invisible", "blind", "incomplete")
DECOY = "Rand"


def main(mode, path):
    d = json.load(open(path))
    funcs = d.get("functions", [])
    if mode == "transitive":
        # The pf drivers are all main -> middle -> leaf, so main is always ON the stack at the effect and is
        # never the frame that issues it. Selecting by that role (rather than by driver-specific names)
        # keeps the mutant generic.
        funcs = [f for f in funcs if f.get("fn", "").split("::")[-1] == "main"]
        if not funcs:
            print(f"[mutate:{mode}] no entry point in {path.split('/')[-1]} — nothing falsified", file=sys.stderr)
            return
    if mode == "wrong" and not funcs:
        # A report with no function entries -- what candor emits for a wholly pure program -- cannot be
        # falsified by editing what is there. Left alone, the pure control would sail through the wrong
        # pass untouched and the verdict's FABRICATION branch would never be exercised, which is exactly
        # the kind of untested-because-unfalsifiable green this battery exists to rule out. So synthesize
        # the claim instead: a signature asserting an effect the program demonstrably never issues.
        funcs = [{"fn": "candor_recall_seed::main", "inferred": []}]
        d["functions"] = funcs
    for f in funcs:
        f["inferred"] = [DECOY] if mode == "wrong" else []
        f["unknownWhy"] = []
        for k in FLAGS:
            if k in f:
                f[k] = False
    if mode == "transitive":
        json.dump(d, open(path, "w"))
        print(f"[mutate:{mode}] falsified the entry point only in {path.split('/')[-1]}", file=sys.stderr)
        return
    # Top-level disclosure surfaces travel with the report (the coverage envelope / kappa ledger); a
    # falsified signature that left them intact would be caught by the wrong channel.
    for k in ("unknownWhy", "kappa", "unanalyzed"):
        if isinstance(d.get(k), list):
            d[k] = []
    json.dump(d, open(path, "w"))
    print(f"[mutate:{mode}] falsified {len(funcs)} fn signature(s) in {path.split('/')[-1]}", file=sys.stderr)


if __name__ == "__main__":
    if len(sys.argv) != 3 or sys.argv[1] not in ("silent", "wrong", "transitive"):
        print(__doc__, file=sys.stderr)
        sys.exit(2)
    main(sys.argv[1], sys.argv[2])
