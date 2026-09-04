#!/usr/bin/env python3
"""Check one generated twin-pair crate's candor report against its truth.json.

Shared by `run_q.sh` (gen_q.py — `?`-position equivalence) and `run_macro.sh` (gen_macro.py —
macro/direct-spelling equivalence). Both generators emit the same `truth.json` shape: a list of
`{base, twin, shape, equal}` pairs, where BASE and TWIN are two spellings of ONE program produced
from one description. `equal: true` means the two spellings are exactly equivalent and must be
charged identically; `equal: false` means only `inferred(TWIN) ⊇ inferred(BASE)` is claimed.

Verdicts:

  VIOLATION   the TWIN lost an effect the BASE was charged — `inferred(TWIN) ⊉ inferred(BASE)`.
              A silent under-report: two spellings of one program, and this one certifies pure or
              narrower what the other was charged for, over drops `examples/gt.rs` executed.
  DRIFT       (equal-pairs only) the TWIN gained an effect the BASE lacks. Same program, so this
              is the same defect seen from the other side: the BASE is the one under-reporting.
  BOTH-PURE   neither spelling is charged, although the ground-truth run performed at least one
              in-frame drop. Also a silent under-report, but NOT the differential these gates are
              calibrated on, so it is counted and printed separately and never counted as a pass.
  NO-GROUND-TRUTH  a spelling performed no in-frame drop at all. A control asserting an absence
              over a program that never runs the construction is asserting something about nothing
              (§E3), so the pair is not judged.

KNOWN-OPEN BASELINE (`--known <file>`). candor-rust HEAD does not satisfy either property today:
there is a standing class of macro spellings the collector never sees. The baseline file records
those shapes, MEASURED exhaustively, so that a NEW instance fails while the standing ones are
listed rather than silently tolerated. Every line in it is a silent under-report that is still
open — the file is a debt register, not a list of acceptable behaviours. A baselined shape that is
now clean is printed as STALE so the list can be pruned; the gate does not fail on it.

Usage:  check_pair.py <crate-dir> <report.json> [--known <file>] [--stale]
"""
import json
import sys


def load_report(path):
    doc = json.load(open(path))
    arr = doc.get("functions", []) if isinstance(doc, dict) else doc
    out = {}
    for e in arr:
        if not isinstance(e, dict):
            continue
        leaf = e.get("fn", "").split("::")[-1]
        # Duplicate `fn` rows (R129) must not silently take the last value: UNION them, the
        # conservative reading for a check that hunts for LOST effects.
        out.setdefault(leaf, set()).update(e.get("inferred") or [])
    return out


def load_gt(d):
    """gt.txt: `<base d_err> <base d_ok> <twin d_err> <twin d_ok> <base-name>` per pair."""
    gt = {}
    try:
        for line in open(d + "/gt.txt"):
            p = line.split()
            if len(p) == 5:
                gt[p[4]] = tuple(int(x) for x in p[:4])
    except OSError:
        pass
    return gt


def load_known(path, kind):
    known = set()
    if not path:
        return known
    try:
        for line in open(path):
            line = line.rstrip("\n")
            if not line or line.startswith("#"):
                continue
            parts = line.split("\t")
            if len(parts) == 3 and parts[0] == kind:
                known.add((parts[1], parts[2]))
    except OSError:
        pass
    return known


def main():
    d, rep = sys.argv[1], sys.argv[2]
    known_path = None
    if "--known" in sys.argv:
        known_path = sys.argv[sys.argv.index("--known") + 1]

    truth = json.load(open(d + "/truth.json"))
    known = load_known(known_path, truth.get("kind", "?"))
    entries = load_report(rep)
    gt = load_gt(d)

    new, old, both_pure, no_gt = [], [], [], []
    seen = set()
    for pr in truth["pairs"]:
        b, t, shape = pr["base"], pr["twin"], pr["shape"]
        sh = ",".join("%s=%s" % (k, shape[k]) for k in sorted(shape))
        g = gt.get(b)
        if g is None or (g[0] + g[1]) == 0 or (g[2] + g[3]) == 0:
            no_gt.append("%s/%s drops=%s [%s]" % (b, t, g, sh))
            continue
        ib, it = entries.get(b, set()), entries.get(t, set())
        if not ib and not it:
            both_pure.append("%s/%s drops=%s [%s]" % (b, t, g, sh))
            continue
        for kindname, delta in (("VIOLATION", ib - it),
                                ("DRIFT", (it - ib) if pr.get("equal") else set())):
            if not delta:
                continue
            seen.add((kindname, sh))
            line = ("%s %s->%s %s=%s base=%s twin=%s drops=%s [%s]"
                    % (kindname, b, t, "lost" if kindname == "VIOLATION" else "gained",
                       sorted(delta), sorted(ib) or "ABSENT", sorted(it) or "ABSENT", g, sh))
            (old if (kindname, sh) in known else new).append(line)

    # STALE only makes sense over an EXHAUSTIVE crate: a random seed touches a handful of shapes,
    # so "not seen here" says nothing about the baseline. `--stale` is passed by baseline.sh only.
    stale = sorted(k for k in known if k not in seen) if (known_path and "--stale" in sys.argv) else []

    n = len(truth["pairs"])
    head = "OK" if not new else "FAIL"
    print("%s pairs=%d new=%d known-open=%d both-pure=%d no-ground-truth=%d stale-baseline=%d"
          % (head, n, len(new), len(old), len(both_pure), len(no_gt), len(stale)))
    for v in new:
        print("  " + v)
    for v in old:
        print("  KNOWN-OPEN " + v)
    for v in both_pure:
        print("  BOTH-PURE " + v)
    for v in no_gt:
        print("  NO-GROUND-TRUTH " + v)
    for k, sh in stale:
        print("  STALE-BASELINE %s [%s]" % (k, sh))


if __name__ == "__main__":
    main()
