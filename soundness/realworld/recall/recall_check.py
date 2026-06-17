#!/usr/bin/env python3
"""Non-syscall RECALL check — candor-scan vs KNOWN crate semantics (ground truth where strace can't reach:
Db/Log/Rand/etc. aren't syscall-distinguishable). For each fn in expected.json, candor must predict the
known effect OR disclose uncertainty (Unknown/blind/invisible/unresolved); a silent-pure is an under-report.
Green on KNOWN (tracked) gaps, red on NEW. Usage: recall_check.py <report.json> <expected.json>"""
import json, sys

# Tracked, awaiting a fix — keeps the recall check a clean gate (red only on NEW under-reports).
#   log_slog — UNCALIBRATED Log crate reached via a MACRO (slog::info!). candor-scan neither classifies it
#     nor DISCLOSES it (visit_macro only records a macro it can classify, so an uncalibrated macro reach
#     never reaches the κ-ledger) → silent-pure. Distinct from the builder-chain family: the fix is to
#     DISCLOSE uncalibrated macro reaches to declared deps as blind (so it reads Unknown, honest). [P1]
KNOWN_UNDER = set()  # log_slog FIXED: visit_macro now discloses crate-qualified macro reaches

def main(report, expected):
    rep = {f["fn"]: f for f in json.load(open(report)).get("functions", [])}
    exp = {k: v for k, v in json.load(open(expected)).items() if not k.startswith("_")}
    ok = known = new = 0
    for fn, eff in exp.items():
        f = rep.get(fn)
        inf = set(f.get("inferred", [])) if f else set()
        uncertain = bool(f and ("Unknown" in inf or f.get("invisible") or f.get("blind")
                                or f.get("unresolved") or f.get("incomplete")))
        if eff in inf or "Unknown" in inf or uncertain:
            ok += 1; print(f"  honest  {fn:14} -> {sorted(inf) or 'disclosed'} (expected {eff})")
        elif fn in KNOWN_UNDER:
            known += 1; print(f"  KNOWN   {fn:14} -> silent-pure, expected {eff} (tracked)")
        else:
            new += 1; print(f"  NEW ✗   {fn:14} -> silent-pure, expected {eff}")
    print(f"\nrecall: {ok} honest, {known} KNOWN under-report(s), {new} NEW under-report(s)")
    return 1 if new else 0

if __name__ == "__main__":
    sys.exit(main(sys.argv[1], sys.argv[2]))
