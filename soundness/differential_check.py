#!/usr/bin/env python3
"""Difference the two rust engines' §3.3 gate verdicts over ONE crate — SOUNDNESS R758.

Reads `<deep>.gate` (the nightly rustc/MIR engine, via CANDOR_GATE_JSON) and `<scan>.gate`
(candor-scan, via --gate-json), both `{ spec, ok, violations[] }`, and partitions the violated
functions three ways:

  FLOOR-ONLY   candor-scan flags it and the deep engine does not. candor-scan calls itself a
               "syntactic floor — a clean run is necessary, not sufficient", so the deep engine is
               supposed to be a SUPERSET of it. This bucket is the gate: non-empty => exit 1.
  DEEP-ONLY    the deep engine flags it and candor-scan does not. EXPECTED and not a failure — the
               deep engine resolves dispatch, generics and transitive edges the syntactic floor
               cannot see. Printed as INFO so the run says what it saw.
  BOTH         agreement.

A FLOOR-ONLY row is A PAIR TO READ, NOT A VERDICT. It does not say which engine is wrong. The two
known directions, both real:
  * the deep engine is silently under-reporting — this is R756's exact signature, where a drop inside
    a closure body read 0 violations under `deny Fs <enclosing fn>` while candor-scan read 1;
  * candor-scan is over-charging — its documented direction, e.g. an item behind an inactive `#[cfg]`,
    which rustc never compiles and syn always parses. That shape is the CALIBRATION (`--calibrate`).

THE JOIN IS ON NAMES, AND THAT IS A STATED WEAKNESS, NOT AN OVERSIGHT. The two engines' `hash` fields
are NOT comparable: candor-scan writes §2.2's `package#fn` while the nightly engine writes a rustc
`DefPathHash` (`dph_hex`, which its own cross-crate chaining parses back), and the nightly engine's
VERDICT rows omit `hash` altogether (`..Default::default()`), so there is no shared join key to use —
see the row filed alongside this script. So one normalisation is applied, and only one:
`<T as Trait>::m` -> `<last segment of T>::m`, which is the single spelling difference the two engines
actually produce (`<Guard as std::ops::Drop>::drop` vs `Guard::drop`). Both engines' FULL violation
lists are printed whenever a bucket is non-empty, so a spelling artefact is visible at a glance rather
than read as a soundness finding.

Usage:  differential_check.py <deep-gate.json> <scan-gate.json> [label]
Exit:   0 clean, 1 FLOOR-ONLY non-empty, 2 a verdict could not be read at all.
"""
import json
import os
import re
import sys

TRAIT_QUAL = re.compile(r"^<(?P<ty>.+?) as .+?>::(?P<m>.+)$")


def norm(name):
    """The one normalisation. See the module docstring."""
    m = TRAIT_QUAL.match(name)
    if m:
        return m.group("ty").rsplit("::", 1)[-1] + "::" + m.group("m")
    return name


def read(path, who):
    # A verdict that could not be read must reach the exit code, not be treated as "no violations" —
    # that is the failure shape `AGENT-CORPUS-BRIEF.md` §H names (the detector works, the aggregator
    # discards the detection).
    if not os.path.exists(path):
        print("  DIFFERENTIAL CANNOT RUN: %s verdict %s was never written" % (who, path))
        return None
    try:
        doc = json.load(open(path))
    except Exception as e:  # noqa: BLE001 — any malformed verdict is the same "cannot run"
        print("  DIFFERENTIAL CANNOT RUN: %s verdict %s is not readable JSON (%s)" % (who, path, e))
        return None
    if not isinstance(doc, dict) or "violations" not in doc:
        print("  DIFFERENTIAL CANNOT RUN: %s verdict %s has no `violations` key" % (who, path))
        return None
    return doc


def main():
    deep_p, scan_p = sys.argv[1], sys.argv[2]
    label = sys.argv[3] if len(sys.argv) > 3 else "crate"
    deep, scan = read(deep_p, "deep"), read(scan_p, "candor-scan")
    if deep is None or scan is None:
        return 2

    d_raw = sorted({v.get("fn", "") for v in deep["violations"]})
    s_raw = sorted({v.get("fn", "") for v in scan["violations"]})
    d = {norm(n) for n in d_raw}
    s = {norm(n) for n in s_raw}

    floor_only = sorted(s - d)
    deep_only = sorted(d - s)
    both = sorted(d & s)

    print(
        "  %s: agree=%d  deep-only=%d  FLOOR-ONLY=%d  (deep ok=%s, scan ok=%s)"
        % (label, len(both), len(deep_only), len(floor_only), deep["ok"], scan["ok"])
    )
    if deep_only:
        print("    INFO deep-only (expected — the floor is syntactic): " + " ".join(deep_only))
    if floor_only:
        print("    FLOOR-ONLY — A PAIR TO READ, NOT A VERDICT: " + " ".join(floor_only))
        print("      deep  violations: " + (" ".join(d_raw) or "(none)"))
        print("      scan  violations: " + (" ".join(s_raw) or "(none)"))
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
