#!/usr/bin/env python3
"""Grade one agent's solution for the HARD agent-use eval (re-run with the callers fix).

The blast radius of tax::apply_tax is 16 functions across 7 files; the dangerous must-stay-pure caller
(realtime::run_stream) reaches apply_tax through a SEPARATE subtree (realtime → pricing helper), so a
hand-tracer focused on the big invoice/report/api/batch branch can miss it. candor's `callers apply_tax`
now returns the whole set (incl. run_stream) instantly. Metrics are objective and don't read candor's
output (ground truth is hand-verified from the call graph).

  blast_recall   : of the 16 transitive callers, how many in BLAST.txt.
  missed_run_stream : did the agent miss the dangerous realtime caller (→ likely wrong placement).
  placement_correct : did Fs stay OUT of the run_stream-reachable subtree (tax.rs/pricing.rs/realtime.rs)
                      AND logging was actually added — i.e. relocated safely.
  added_logging  : did any Fs land anywhere (did they do the task at all).
  used_candor / candor_calls : adoption, from the usage-log shim.

Usage:  grade-hard.py <solution-dir>
"""
import json
import os
import re
import sys

GROUND_TRUTH = {
    "priced", "line_total", "invoice_total", "order_total", "daily_rollup", "monthly_rollup",
    "export_pdf", "batch_export", "nightly_job", "api_quote", "handle_request", "serve",
    "spot_quote", "stream_tick", "run_stream", "main",
}
DANGEROUS = "run_stream"
CALLEES = {"rate"}  # apply_tax's callee — listing it is a call-direction error
# Files transitively reachable FROM run_stream: realtime.rs (run_stream/stream_tick/spot_quote),
# pricing.rs (priced), tax.rs (apply_tax/rate). Fs in any of these breaks the per-tick budget.
RUNSTREAM_FILES = ["tax.rs", "pricing.rs", "realtime.rs"]
IO_RE = re.compile(r"std::fs|fs::|File::|OpenOptions|std::io::Write|write!|writeln!")


def read(p):
    try:
        return open(p).read()
    except OSError:
        return ""


def main():
    d = sys.argv[1]
    tokens = set(re.findall(r"[A-Za-z_][A-Za-z0-9_]*", read(os.path.join(d, "BLAST.txt"))))
    reported = GROUND_TRUTH & tokens
    missed = sorted(GROUND_TRUTH - reported)

    src = os.path.join(d, "src")
    runstream_io = any(IO_RE.search(read(os.path.join(src, f))) for f in RUNSTREAM_FILES)
    all_src = "".join(read(os.path.join(src, f)) for f in os.listdir(src)) if os.path.isdir(src) else ""
    added_logging = IO_RE.search(all_src) is not None

    usage = read(os.path.join(d, ".candor-usage.log"))
    calls = [ln.split("|", 1)[1] for ln in usage.splitlines() if "|" in ln]

    out = {
        "blast_recall": round(len(reported) / len(GROUND_TRUTH), 3),
        "missed_count": len(missed),
        "missed_run_stream": int(DANGEROUS in missed),
        "direction_err": sorted(CALLEES & tokens),
        "placement_correct": int((not runstream_io) and added_logging),
        "added_logging": int(added_logging),
        "runstream_broken": int(runstream_io),
        "used_candor": int(bool(calls)),
        "candor_calls": calls,
        "decision": read(os.path.join(d, "DECISION.txt")).strip()[:60],
    }
    print(json.dumps(out))


if __name__ == "__main__":
    main()
