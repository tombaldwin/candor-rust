#!/usr/bin/env python3
"""Grade one agent's solution for the agent-use eval (Experiment A).

Objective metrics, none of which consult candor's live output (the blast-radius ground truth is
hand-verified from the fixture's call graph, not read from candor):

  blast_recall   : of the 6 functions that TRANSITIVELY gain Fs if compute_price does, how many the
                   agent listed in BLAST.txt (the awareness metric).
  missed         : which of the 6 the agent missed (health_probe is the dangerous one to miss).
  direction_err  : did the agent wrongly list a CALLEE of compute_price (base_price/margin) as
                   affected — a sign it confused call direction.
  pricing_pure   : did src/pricing.rs stay free of Fs syntax (the shipped-code metric: the correct
                   action is to relocate the logging, since health_probe must stay I/O-free).
  used_candor    : did the agent invoke candor at all (from the usage-log shim), and which commands.

Usage:  grade.py <solution-dir>
"""
import json
import os
import re
import sys

# Hand-verified from the fixture's call graph: the functions that transitively call compute_price and
# would therefore gain Fs if it did. (compute_price itself gains it directly, not transitively.)
GROUND_TRUTH = {"line_item", "render_invoice", "monthly_report", "export_csv", "health_probe", "main"}
# Callees of compute_price — NOT affected; listing them is a call-direction error.
CALLEES = {"base_price", "margin"}

IO_RE = re.compile(r"std::fs|fs::|File::|OpenOptions|std::io::Write|write!|writeln!")


def read(path):
    try:
        return open(path).read()
    except OSError:
        return ""


def main():
    d = sys.argv[1]
    blast = read(os.path.join(d, "BLAST.txt"))
    decision = read(os.path.join(d, "DECISION.txt")).strip()
    pricing = read(os.path.join(d, "src", "pricing.rs"))
    usage = read(os.path.join(d, ".candor-usage.log"))

    # words mentioned in BLAST.txt (identifier tokens)
    tokens = set(re.findall(r"[A-Za-z_][A-Za-z0-9_]*", blast))
    reported = GROUND_TRUTH & tokens
    missed = sorted(GROUND_TRUTH - reported)
    direction_err = sorted(CALLEES & tokens)

    # candor usage (treatment only): the shim logged "ts|<args>" lines.
    calls = [ln.split("|", 1)[1] for ln in usage.splitlines() if "|" in ln]
    used_candor = 1 if calls else 0

    out = {
        "blast_recall": round(len(reported) / len(GROUND_TRUTH), 3),
        "reported": sorted(reported),
        "missed": missed,
        "missed_health_probe": int("health_probe" in missed),
        "direction_err": direction_err,
        "pricing_pure": int(IO_RE.search(pricing) is None),
        "decision": decision[:60],
        "used_candor": used_candor,
        "candor_calls": calls,
    }
    print(json.dumps(out))


if __name__ == "__main__":
    main()
