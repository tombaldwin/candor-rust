#!/usr/bin/env python3
"""Check one generated crate's candor report against its truth.json.

Prints `OK` if every function the generator knows reaches the effect is reported with that effect OR
with `Unknown` (a sound over-approximation). Otherwise prints `FAIL <effect> :: <offenders> :: forms`.
A reachable function that is reported PURE — or omitted from the report (candor omits effect-free fns)
— is a SILENT UNDER-REPORT: the failure this harness hunts. `Unknown` is a PASS (soundness, not
precision).

Usage:  check.py <crate-dir>
"""
import glob
import json
import sys


def main():
    d = sys.argv[1]
    truth = json.load(open(d + "/truth.json"))
    effect = truth["effect"]
    expect = set(truth["expect"])

    entries = {}
    for f in glob.glob(d + "/r.*.*.json"):
        # The lint writes report sidecars next to the report (e.g. `<crate>.<kind>.callgraph.json`)
        # that this naive glob also matches but which are NOT effect reports; skip them.
        if f.endswith(".callgraph.json"):
            continue
        try:
            doc = json.load(open(f))
        except Exception:
            continue
        arr = doc.get("functions", doc) if isinstance(doc, dict) else doc
        if not isinstance(arr, list):  # not a report shape (e.g. a sidecar dict) — skip defensively
            continue
        for e in arr:
            if not isinstance(e, dict):
                continue
            leaf = e.get("fn", "").split("::")[-1]
            entries[leaf] = e.get("inferred", [])

    bad = []
    for fn in sorted(expect):
        inf = entries.get(fn)
        if inf is None:  # omitted from the report => candor judged it PURE
            bad.append(fn + "(pure/omitted)")
        elif effect not in inf and "Unknown" not in inf:  # present but neither the effect nor Unknown
            bad.append(fn + "{" + ",".join(inf) + "}")

    if bad:
        print("FAIL " + effect + " :: " + " ".join(bad) + " :: forms=" + json.dumps(truth["forms"]))
    else:
        print("OK")


if __name__ == "__main__":
    main()
