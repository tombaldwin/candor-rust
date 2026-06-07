#!/usr/bin/env python3
"""Dynamic-oracle check for one generated crate (Bet 1, phase 2).

GROUND TRUTH FROM REALITY: we ran the program under a syscall tracer. If its effect actually executed
(its distinctive marker appears in the trace), candor's static prediction for the program (the entry
point `main`'s transitive `inferred`) MUST contain that effect — or `Unknown`, a sound
over-approximation. A program that demonstrably performs an effect at runtime which candor predicts
NOWHERE is a silent under-report: the worst bug. Unlike the construction checker, this trusts nothing
about how the crate was generated — only what the kernel saw.

Prints one of:
  OK                          candor predicted the observed effect (or Unknown)
  SKIP <reason>               not checkable (Env isn't syscall-observable, or the effect didn't run)
  FAIL <detail>              ran the effect, candor missed it

Usage:  oracle_check.py <crate-dir> <strace-log>
"""
import glob
import json
import sys


def predicted_effects(crate_dir):
    """candor's predicted effect set for the program = `main`'s transitive `inferred` (the entry
    point), falling back to the union over all functions if `main` isn't reported."""
    main_inf = None
    union = set()
    for f in glob.glob(crate_dir + "/r.*.*.json"):
        try:
            doc = json.load(open(f))
        except Exception:
            continue
        arr = doc.get("functions", doc) if isinstance(doc, dict) else doc
        for e in arr:
            inf = set(e.get("inferred", []))
            union |= inf
            if e.get("fn", "").split("::")[-1] == "main":
                main_inf = inf
    return main_inf if main_inf is not None else union


def main():
    crate_dir, trace_log = sys.argv[1], sys.argv[2]
    truth = json.load(open(crate_dir + "/truth.json"))
    effect, marker = truth["effect"], truth.get("marker")

    if marker is None:
        print("SKIP %s-not-syscall-observable" % effect)
        return

    try:
        trace = open(trace_log, errors="replace").read()
    except Exception:
        print("SKIP no-trace")
        return

    if marker not in trace:
        # The effect's syscall never executed (a runtime quirk, not a candor problem) — nothing to check.
        print("SKIP %s-marker-not-observed" % effect)
        return

    pred = predicted_effects(crate_dir)
    if effect in pred or "Unknown" in pred:
        print("OK")
    else:
        print("FAIL ran %s (marker %r seen in trace) but candor predicts {%s} — a REAL effect missed"
              % (effect, marker, ",".join(sorted(pred))))


if __name__ == "__main__":
    main()
