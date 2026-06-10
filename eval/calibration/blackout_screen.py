#!/usr/bin/env python3
"""Blackout screen: find crates whose PURPOSE implies an effect but whose candor report is
near-silent on it. Generalizes the A/B heuristic that caught the libcurl blackout (curl 0.4
reported 3 fns / 0 Net): a thin binding or client crate that reports nothing is far more
likely a classifier/scanner gap than a genuinely pure crate.

Method: scan the newest vendored version of every crate under ~/.cargo/registry with
candor-scan, then flag crates where a name/keyword-implied effect has ZERO occurrences.
The flag list is a SCREEN (high recall, manual verification required), not a verdict:
interface/type-only crates, macros, and *-sys shims (extern decls, no bodies) legitimately
report nothing — *-sys/-bindings/-macros/derive crates are excluded up front.

Usage: python3 blackout_screen.py [out.json]
"""
import json, subprocess, glob, os, re, sys, tempfile, shutil
from collections import Counter

REG = sorted(glob.glob(os.path.expanduser("~/.cargo/registry/src/index.crates.io-*")))[-1]
# Resolve the scanner relative to this file (eval/calibration/ -> repo root), env-overridable — NOT a
# hardcoded machine path (which broke silently after the candor->candor-rust rename; /code-review).
_ROOT = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
SCAN = os.environ.get("CANDOR_SCAN_BIN") or f"{_ROOT}/target/release/candor-scan"
if not os.path.exists(SCAN):
    sys.exit(f"FATAL: candor-scan not at {SCAN} — build it (cargo build --release -p candor-scan) or set "
             f"CANDOR_SCAN_BIN. (The screen would otherwise silently report '0 suspects'.)")

# keyword (substring of crate name, '_'-normalized) -> effect the name implies
IMPLIES = [
    # network clients / protocols
    (r"(^|_)(http|hyper|ws|websocket|websockets|ssh|ftp|smtp|imap|pop3|mqtt|amqp|nats|kafka|grpc|quic|dns|resolver)(_|$)", "Net"),
    (r"(^|_)(tcp|udp|socket|socks|proxy|tls|reqwest|curl|fetch)(_|$)", "Net"),
    # databases / stores
    (r"(^|_)(sql|sqlite|postgres|mysql|mongo|mongodb|redis|cassandra|dynamo|etcd|leveldb|rocksdb|lmdb)(_|$)", "Db"),
    # subprocess
    (r"(^|_)(subprocess|process|command|exec|shell|pty)(_|$)", "Exec"),
    # filesystem
    (r"(^|_)(fs|file|dir|tempfile|tempdir|walkdir|watch|notify|zip|tar|archive|glob)(_|$)", "Fs"),
    # entropy
    (r"(^|_)(rand|random|entropy|uuid)(_|$)", "Rand"),
]
# name fragments that mean "no body to scan / pure-by-design" -> skip
SKIP = re.compile(r"(-sys$|_sys$|-bindings|-macros?$|_macros?$|-derive$|_derive$|-types$|_types$|-core$|_core$|-traits?$|_traits?$|-codegen|-build$|-test|-mock|parser|-model|-schema|-spec$)")

def newest(prefix_dir):
    by_name = {}
    for d in glob.glob(f"{REG}/*"):
        b = os.path.basename(d)
        m = re.match(r"(.+?)-(\d+\.\d+\.\d+)", b)
        if not m:
            continue
        name, ver = m.group(1), tuple(int(x) for x in m.group(2).split("."))
        if name not in by_name or ver > by_name[name][0]:
            by_name[name] = (ver, d)
    return {n: d for n, (_, d) in by_name.items()}

def scan(src):
    tmp = tempfile.mkdtemp()
    try:
        shutil.copytree(src, tmp, dirs_exist_ok=True, symlinks=True)
        r = subprocess.run([SCAN, tmp], capture_output=True, timeout=120)
        reps = [p for p in glob.glob(f"{tmp}/.candor/report.*.scan.json") if "callgraph" not in p]
        if not reps:
            return None
        d = json.load(open(reps[0]))
        fns = d["functions"] if isinstance(d, dict) else d
        eff = Counter()
        for e in fns:
            for x in e.get("inferred", []):
                eff[x] += 1
        return {"fns": len(fns), "effects": dict(eff)}
    except Exception:
        return None
    finally:
        shutil.rmtree(tmp, ignore_errors=True)

def main():
    crates = newest(REG)
    suspects, scanned = [], 0
    for name, d in sorted(crates.items()):
        norm = name.replace("-", "_")
        if SKIP.search(name):
            continue
        implied = {eff for pat, eff in IMPLIES if re.search(pat, norm)}
        if not implied:
            continue
        res = scan(d)
        scanned += 1
        if res is None:
            continue
        missing = sorted(e for e in implied if res["effects"].get(e, 0) == 0)
        if missing:
            suspects.append({
                "crate": os.path.basename(d), "implies": sorted(implied), "missing": missing,
                "effectful_fns": res["fns"], "effects": res["effects"],
            })
        if scanned % 50 == 0:
            print(f"  …{scanned} scanned, {len(suspects)} suspects", file=sys.stderr)
    # near-silent first: the fewer effectful fns, the likelier a blackout
    suspects.sort(key=lambda s: (s["effectful_fns"], s["crate"]))
    out = sys.argv[1] if len(sys.argv) > 1 else "blackout-suspects.json"
    json.dump({"scanned": scanned, "suspects": suspects}, open(out, "w"), indent=1)
    print(f"{scanned} keyword-matched crates scanned; {len(suspects)} suspects -> {out}")

if __name__ == "__main__":
    main()
