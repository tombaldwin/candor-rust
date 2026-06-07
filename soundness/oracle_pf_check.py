#!/usr/bin/env python3
"""Per-function dynamic-oracle check (Bet 1, phase 2 — strengthened).

The whole-program oracle only checks `main`. This one reconstructs the CALL STACK at the moment the
effect syscall actually fired — from `eprintln` entry/exit markers (`CFE <fn>` / `CFX <fn>`, emitted by
the instrumented program, visible to strace as `write(2,…)` but invisible to candor) interleaved with
the effect syscall in the trace. Every function that was ON THE STACK when the effect fired
*demonstrably* performs that effect transitively, so candor MUST report each with the effect or
`Unknown`. A function on the stack at the effect but reported PURE/omitted is a per-function silent
under-report — caught against the kernel's own record, attributed to the exact function.

Prints OK / SKIP <reason> / FAIL <detail>.   Usage: oracle_pf_check.py <crate-dir> <strace-log>
"""
import glob
import json
import re
import sys

MARKER_RE = re.compile(r'write\(2, "CF([EX]) (\w+)')


def report_inferred(crate_dir):
    out = {}
    for f in glob.glob(crate_dir + "/r.*.*.json"):
        try:
            doc = json.load(open(f))
        except Exception:
            continue
        arr = doc.get("functions", doc) if isinstance(doc, dict) else doc
        for e in arr:
            out[e.get("fn", "").split("::")[-1]] = set(e.get("inferred", []))
    return out


def main():
    crate_dir, trace_log = sys.argv[1], sys.argv[2]
    truth = json.load(open(crate_dir + "/truth.json"))
    effect, marker = truth["effect"], truth.get("marker")
    if marker is None:
        print("SKIP %s-not-syscall-observable" % effect)
        return
    try:
        lines = open(trace_log, errors="replace").read().splitlines()
    except Exception:
        print("SKIP no-trace")
        return

    stack = []
    on_stack_at_effect = set()
    for line in lines:
        m = MARKER_RE.search(line)
        if m:
            kind, name = m.group(1), m.group(2)
            if kind == "E":
                stack.append(name)
            else:  # exit: pop the last matching name (no recursion in generated chains, but be safe)
                for i in range(len(stack) - 1, -1, -1):
                    if stack[i] == name:
                        del stack[i]
                        break
            continue
        # the effect syscall itself (its marker appears in an openat/connect line, never in a CF marker)
        if marker in line:
            on_stack_at_effect.update(stack)

    if not on_stack_at_effect:
        print("SKIP %s-marker-not-observed" % effect)
        return

    inferred = report_inferred(crate_dir)
    bad = []
    for fn in sorted(on_stack_at_effect):
        inf = inferred.get(fn)
        if inf is None:
            bad.append(fn + "(pure/omitted)")
        elif effect not in inf and "Unknown" not in inf:
            bad.append(fn + "{" + ",".join(sorted(inf)) + "}")
    if bad:
        print("FAIL %s :: on-stack-at-effect but candor missed it: %s" % (effect, " ".join(bad)))
    else:
        print("OK")


if __name__ == "__main__":
    main()
