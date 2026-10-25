#!/usr/bin/env python3
"""Build, EXECUTE, scan and gate every call-target matrix cell, under one or more candor-scan builds.

usage: run.py <root> <jobs> <name=bindir>[,<name=bindir>...] [cid-substring-filter | --rescan=<name=bindir>]

<root> holds `cells/` (from gen.py) and receives `results.jsonl` (resumable: a cid already there is skipped) and
`markers/`. Each cell is built with rustc and RUN once; the effect it really performs is observed (Fs/Exec marker
files under <root>/markers, Net via a TCP listener on 127.0.0.1:$CT_PORT logging the cell id) and compared with the
generator's expectation. Then every arm (solo | ch = dependency report chained via CANDOR_DEPS | unch = dependency
present, not chained) is scanned by EACH named build, measured at the CALLER `zz_caller`: its report row,
`deny <E> zz_caller` and `deny Unknown zz_caller` through that build's candor-query over the scan's own report.

Classes (effect cells): CHARGED (`deny E` 1) / DISCLOSED (`deny E` 0, `deny Unknown` 1) / LEDGER-ONLY (both 0, the row
carries `invisible`/`incomplete`) / SILENT (both 0, nothing on the row). No-effect cells: PURE-OK / PURE+UNKNOWN /
OVER (a concrete charge that is the sound over-approximation of an abstract site) / FABRICATED (any other concrete
charge). Building and executing once per cell and scanning with every build holds the program and its ground truth
constant across builds, so a class change is the build's.
"""
import json, os, sys, subprocess, socket, threading, shutil, time
from concurrent.futures import ThreadPoolExecutor

ROOT = sys.argv[1]
JOBS = int(sys.argv[2])
BINS = [tuple(x.split("=", 1)) for x in sys.argv[3].split(",")]
FILT = sys.argv[4] if len(sys.argv) > 4 else ""
CELLS = os.path.join(ROOT, "cells")
MARK = os.path.join(ROOT, "markers")
RES = os.path.join(ROOT, "results.jsonl")
PORT = int(os.environ.get("CT_PORT", "47123"))
os.makedirs(MARK, exist_ok=True)

NET_SEEN = set()
lock = threading.Lock()
def listener():
    srv = socket.socket(); srv.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
    srv.bind(("127.0.0.1", PORT)); srv.listen(64)
    while True:
        c, _ = srv.accept()
        def h(c=c):
            data = b""
            c.settimeout(5)
            try:
                while True:
                    b = c.recv(4096)
                    if not b: break
                    data += b
            except Exception:
                pass
            for line in data.decode(errors="replace").split():
                with lock:
                    NET_SEEN.add(line.strip())
            c.close()
        threading.Thread(target=h, daemon=True).start()
threading.Thread(target=listener, daemon=True).start()
time.sleep(0.3)

ENV = {k: v for k, v in os.environ.items() if not k.startswith("CANDOR")}

def sh(cmd, cwd=None, env=None, timeout=180):
    try:
        p = subprocess.run(cmd, cwd=cwd, env=env or ENV, stdout=subprocess.PIPE, stderr=subprocess.STDOUT, timeout=timeout)
        return p.returncode, p.stdout.decode(errors="replace")
    except subprocess.TimeoutExpired:
        return -9, "TIMEOUT"

def rustc(args, cwd):
    return sh(["rustc", "--edition", "2021", "-C", "debuginfo=0", "-A", "warnings"] + args, cwd=cwd)

def build_and_run(d, meta):
    b = os.path.join(d, "build")
    os.makedirs(b, exist_ok=True)
    ext = []
    if meta["has_lib"]:
        rc, out = rustc(["--crate-type", "rlib", "--crate-name", "ctlib", "lib/src/lib.rs", "--out-dir", "build"], d)
        if rc != 0: return "BUILD-FAIL-lib", out
        ext = ["--extern", "ctlib=build/libctlib.rlib", "-L", "build"]
    rc, out = rustc(["--crate-type", "rlib", "--crate-name", "ctapp", "app/src/lib.rs", "--out-dir", "build"] + ext, d)
    if rc != 0: return "BUILD-FAIL-app", out
    rc, out = rustc(["--crate-name", "ctrun", "app/src/main.rs", "--extern", "ctapp=build/libctapp.rlib", "-L", "build", "-o", "build/run"] + ext, d)
    if rc != 0: return "BUILD-FAIL-main", out
    cid = meta["cid"]
    for e in ("fs", "exec"):
        p = os.path.join(MARK, cid + "." + e)
        if os.path.exists(p): os.remove(p)
    with lock:
        NET_SEEN.discard(cid)
    rc, out = sh([os.path.join(b, "run")], cwd=b, timeout=30)
    time.sleep(0.05)
    E = meta["effect"]
    if E == "Net":
        for _ in range(40):
            with lock:
                if cid in NET_SEEN: break
            time.sleep(0.05)
        with lock:
            happened = cid in NET_SEEN
    else:
        happened = os.path.exists(os.path.join(MARK, cid + "." + E.lower()))
    return ("RAN" if rc == 0 else "RAN-rc%d" % rc), happened

def gate(query, report, rule, d, tag):
    pol = os.path.join(d, "pol." + tag)
    open(pol, "w").write(rule + "\n")
    return sh([query, "gate", "--report", report, "--policy", pol])

def scan_arm(d, meta, arm, bname, bdir):
    scan = os.path.realpath(os.path.join(bdir, "candor-scan"))
    query = os.path.realpath(os.path.join(bdir, "candor-query"))
    out = os.path.join(d, "o-%s-%s" % (bname, arm))
    if os.path.exists(out): shutil.rmtree(out)
    os.makedirs(out)
    env = dict(ENV)
    if arm in ("ch", "chns"):
        dep = os.path.join(out, "dep")
        os.makedirs(dep)
        rc, o = sh([scan, "lib", "--out", os.path.join(dep, "r")], cwd=d)
        if rc != 0: return {"err": "DEP-SCAN rc=%d %s" % (rc, o[-400:])}
        if arm == "chns":
            # FABLE-REVIEW §3.5 — the chained report WITHOUT its §2.2 call-graph sidecar.
            for fn in os.listdir(dep):
                if fn.endswith(".callgraph.json"):
                    os.remove(os.path.join(dep, fn))
        env["CANDOR_DEPS"] = dep
    rc, o = sh([scan, "app", "--out", os.path.join(out, "a")], cwd=d, env=env)
    rep = os.path.join(out, "a.ctapp.scan.json")
    if not os.path.exists(rep): return {"err": "SCAN rc=%d %s" % (rc, o[-400:])}
    j = json.load(open(rep))
    row = None
    for f in j.get("functions", []):
        if f.get("fn") == "zz_caller":
            row = f
    collide = [f["fn"] for f in j.get("functions", []) if f.get("fn", "").startswith("zz_caller") and f.get("fn") != "zz_caller"]
    E = meta["effect"]
    gE, oE = gate(query, rep, "deny %s zz_caller" % E, out, "E")
    gU, oU = gate(query, rep, "deny Unknown zz_caller", out, "U")
    return {"row": row, "gE": gE, "gU": gU, "collide": collide,
            "gEout": oE[-300:] if gE not in (0, 1) else "", "gUout": oU[-300:] if gU not in (0, 1) else ""}

def classify(meta, a):
    if "err" in a: return "ERR"
    row = a["row"] or {}
    inf = set(row.get("inferred", []))
    E = meta["effect"]
    conc = {x for x in inf if x != "Unknown"}
    if meta["gt"]:
        if a["gE"] == 1:
            extra = conc - {E}
            return "CHARGED" + ("+EXTRA:" + ",".join(sorted(extra)) if extra else "")
        if a["gU"] == 1: return "DISCLOSED"
        if row.get("invisible") or row.get("incomplete"): return "LEDGER-ONLY"
        return "SILENT"
    if conc:
        return ("OVER" if meta["over_ok"] else "FABRICATED") + ":" + ",".join(sorted(conc))
    if a["gU"] == 1: return "PURE+UNKNOWN"
    return "PURE-OK"

def do(cid):
    d = os.path.join(CELLS, cid)
    meta = json.load(open(os.path.join(d, "meta.json")))
    st, happened = build_and_run(d, meta)
    rec = {"cid": cid, "family": meta["family"], "kind": meta["kind"], "where": meta["where"], "n": meta["n"],
           "effect": meta["effect"], "gt": meta["gt"], "over_ok": meta["over_ok"], "run": st}
    if st.startswith("BUILD-FAIL"):
        rec["build_out"] = happened[-1500:]
        return rec
    rec["happened"] = happened
    rec["gt_ok"] = (happened == meta["gt"])
    rec["arms"] = []
    for arm in meta["arms"]:
        ent = {"arm": arm}
        for bname, bdir in BINS:
            a = scan_arm(d, meta, arm, bname, bdir)
            a["class"] = classify(meta, a)
            ent[bname] = a
        rec["arms"].append(ent)
    shutil.rmtree(os.path.join(d, "build"), ignore_errors=True)
    return rec

def rescan(name_dir):
    """--rescan NAME=DIR: re-scan every measured cell with ONE build and replace that build's entries, keeping the
    program and the EXECUTED ground truth already in results.jsonl (nothing is rebuilt or re-run). Used to re-measure
    a changed build without paying for the executions again; the other builds' entries are untouched."""
    bname, bdir = name_dir.split("=", 1)
    recs = [json.loads(l) for l in open(RES)]
    def one(rec):
        if rec["run"].startswith("BUILD"): return rec
        d = os.path.join(CELLS, rec["cid"])
        meta = json.load(open(os.path.join(d, "meta.json")))
        for ent in rec["arms"]:
            a = scan_arm(d, meta, ent["arm"], bname, bdir)
            a["class"] = classify(meta, a)
            ent[bname] = a
        return rec
    with ThreadPoolExecutor(JOBS) as ex:
        out = list(ex.map(one, recs))
    tmp = RES + ".tmp"
    with open(tmp, "w") as fo:
        for r in out: fo.write(json.dumps(r) + "\n")
    os.replace(tmp, RES)
    print("=== rescan done", bname, len(out), flush=True)

def main():
    if FILT.startswith("--rescan="):
        return rescan(FILT[len("--rescan="):])
    done = set()
    if os.path.exists(RES):
        for l in open(RES):
            try: done.add(json.loads(l)["cid"])
            except Exception: pass
    todo = sorted(c for c in os.listdir(CELLS) if FILT in c and c not in done)
    print("todo", len(todo), "done", len(done), flush=True)
    n = 0
    with ThreadPoolExecutor(JOBS) as ex, open(RES, "a") as fo:
        for rec in ex.map(do, todo):
            fo.write(json.dumps(rec) + "\n"); fo.flush()
            n += 1
            if n % 100 == 0: print(n, flush=True)
    print("=== done ===", flush=True)

if __name__ == "__main__":
    main()
