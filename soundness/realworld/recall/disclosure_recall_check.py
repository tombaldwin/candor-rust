#!/usr/bin/env python3
"""DISCLOSURE-RECALL check — did the syscall oracle actually CATCH the cardinal sins we seeded?

Input: the logs written by disclosure_recall.sh -- a control pass plus one per seeded mutant -- all produced
by the REAL oracle, the mutants differing only in that candor's report was falsified before the verdict
read it.

The check is per-driver, not aggregate. From the control pass we compute the FALSIFIABLE set: drivers whose
effect demonstrably executed under strace this run (marker fired), so a falsified signature is a detectable
lie. Drivers that were skipped -- build failure, or the effect did not execute -- are NOT falsifiable this
run and are reported as uncalibrated rather than quietly counted as successes. Then each mutant pass must
flag EXACTLY that set: every falsifiable driver caught (no blind spots) and no others (no spurious reds).

Recall is reported as a fraction of what the run could see, and the uncalibrated remainder is printed by
name. An oracle that can only be falsified on 3 of 20 drivers has a recall of 1.0 and is still nearly blind;
the two numbers have to travel together.
"""
import re, sys

CASE = re.compile(r"^\s{2}(\S+): ran=(\d)\s+effect=(\S+)")
PF_OK = re.compile(r"^\s{2}(pf_\S+): OK \((\d+) fns on stack")
PF_SKIP = re.compile(r"^\s{2}(pf_\S+): SKIP (.*)$")
PF_FAILED = re.compile(r"per-function under-report on:(.*)$")
MULTI = re.compile(r"^\s{2}(\S+): \(multi-effect\)")
MULTI_EFF = re.compile(r"^\s{4}\[(\w+)\] (?:✓|ⓘ)")
MULTI_SKIP = re.compile(r"^\s{4}\[(\w+)\] SKIP \((.*)\)")
FAILED = re.compile(r"NEW under-reporting drivers:(.*)$")
FAB = re.compile(r"(\d+) fabrication\(s\)")


def parse_control_pf(text):
    """Per-function oracle: a driver is falsifiable when at least one function was ON THE STACK at the
    effect syscall, since that is the set the verdict actually adjudicates."""
    falsifiable, skipped = set(), {}
    for ln in text.splitlines():
        m = PF_OK.match(ln)
        if m:
            name, on_stack = m.group(1), int(m.group(2))
            if on_stack:
                falsifiable.add(name)
            else:
                skipped[name] = "no function on the stack at the effect"
            continue
        m = PF_SKIP.match(ln)
        if m:
            skipped[m.group(1)] = m.group(2)
            continue
        if "build failed — SKIP" in ln or "no candor report — SKIP" in ln:
            skipped[ln.strip().split(":")[0]] = "did not build / no report"
    return falsifiable, skipped


def parse_flagged_pf(text):
    for ln in text.splitlines():
        m = PF_FAILED.search(ln)
        if m:
            return set(m.group(1).split())
    return set()


def parse_control(text):
    """-> (falsifiable set of driver keys, dict of skipped driver -> reason)."""
    falsifiable, skipped, cur_multi = set(), {}, None
    for ln in text.splitlines():
        m = CASE.match(ln)
        if m:
            cur_multi = None
            name, ran, eff = m.group(1), m.group(2), m.group(3)
            if eff == "none":
                continue                      # the pure control: nothing to hide, calibrates fabrication
            if ran == "1":
                falsifiable.add(name)
            else:
                skipped[name] = f"{eff} did not execute under strace"
            continue
        m = MULTI.match(ln)
        if m:
            cur_multi = m.group(1)
            continue
        if cur_multi:
            m = MULTI_EFF.match(ln)
            if m:
                falsifiable.add(f"{cur_multi}:{m.group(1)}")
                continue
            # A multi-effect driver whose marker did not fire is uncalibrated for THAT effect. Dropping it
            # from both counts would be the silent truncation this checker exists to prevent: recall would
            # read 1.0 over a quietly smaller denominator.
            m = MULTI_SKIP.match(ln)
            if m:
                skipped[f"{cur_multi}:{m.group(1)}"] = m.group(2)
                continue
        if ": no source — SKIP" in ln or "failed — SKIP" in ln or ": no binary — SKIP" in ln:
            skipped[ln.strip().split(":")[0]] = "did not build"
    return falsifiable, skipped


def parse_flagged(text):
    for ln in text.splitlines():
        m = FAILED.search(ln)
        if m:
            return set(m.group(1).split())
    return set()


def fabrications(text):
    m = FAB.search(text)
    return int(m.group(1)) if m else 0


def main(fmt, control_log, *mutant_args):
    """mutant_args are `<mode>=<log path>` pairs, so each oracle can be calibrated with the mutants that
    are meaningful for it (only the per-function verdict can see a lie confined to one transitive frame)."""
    pf = fmt == "perfn"
    ctl_parse = parse_control_pf if pf else parse_control
    flag_parse = parse_flagged_pf if pf else parse_flagged
    control = open(control_log).read()
    if flag_parse(control):
        print("ABORT: the control pass is already red — calibrate against a green oracle, not a broken one.")
        return 2

    falsifiable, skipped = ctl_parse(control)
    if not falsifiable:
        print("ABORT: no driver was falsifiable this run (no effect executed). Nothing to calibrate.")
        return 2

    print(f"falsifiable this run: {len(falsifiable)} driver(s) whose effect demonstrably executed")
    if skipped:
        print(f"UNCALIBRATED ({len(skipped)}) — no evidence either way for these:")
        for k, v in sorted(skipped.items()):
            print(f"    {k}: {v}")

    rc = 0
    for arg in mutant_args:
        mode, _, log = arg.partition("=")
        text = open(log).read()
        flagged = flag_parse(text)
        caught = falsifiable & flagged
        missed = falsifiable - flagged
        spurious = flagged - falsifiable
        print(f"\n[{mode}] seeded {len(falsifiable)}, oracle flagged {len(flagged)} "
              f"-> recall {len(caught)}/{len(falsifiable)}")
        for m in sorted(missed):
            print(f"    ✗ MISSED  {m}: signature falsified, effect ran, oracle stayed green — a BLIND SPOT")
            rc = 1
        for s in sorted(spurious):
            print(f"    ✗ SPURIOUS {s}: flagged but was not falsifiable — the verdict is reading something else")
            rc = 1
        if mode == "wrong" and not pf:
            # the pure control now carries a decoy effect it never issued: the fabrication mirror must fire
            if fabrications(text) < 1:
                print("    ✗ the pure control was given a decoy effect and no fabrication was reported")
                rc = 1
            else:
                print("    ✓ fabrication mirror fired on the pure control (decoy effect, no syscall)")

    print("\nRESULT: " + ("disclosure recall COMPLETE on the falsifiable set" if rc == 0
                          else "BLIND SPOTS FOUND — a green run of this oracle is not evidence"))
    return rc


if __name__ == "__main__":
    sys.exit(main(*sys.argv[1:]))
