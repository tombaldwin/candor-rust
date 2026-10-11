#!/usr/bin/env bash
# run_handoff.sh — the HAND-OFF gate for the stable scanner (candor-scan), with EXECUTED ground truth.
#
# Each seed generates a crate (`gen_drop.py --handoff`) whose cells hand one fresh `H` (a type whose
# `Drop` writes a file) to something that drops it, across three axes — call POSITION (stmt / tail /
# `?` operand / closure tail), hand-off TARGET (local fn / wrapper param / generic param / closure param
# / std `drop` / a callee that stores into a Vec this frame owns) and BINDING (direct / let / shadowed /
# sibling). The crate's `gt` binary RUNS every cell against a drop counter; candor-scan then scans the
# crate. A cell that dropped (GT > 0) whose row is neither charged nor disclosed (`Unknown`) is a
# SILENT UNDER-REPORT, and the gate fails on one unless its shape is in the measured known-open
# register (`soundness/known_open_handoff.tsv`, kind `h`). The register is a debt list, printed every run.
#
# WHY. `run_drop.sh` measures only the DEEP engine, and `run_q.sh`/`run_macro.sh` compare two spellings
# of one program — none of them reaches a value that dies in a CALLEE, which is where R198, R200, R323
# and R1121 all lived.
#
#   bash soundness/run_handoff.sh [N]          # N random seeds (12 cells each), default 20
#   bash soundness/run_handoff.sh --all        # the whole 120-cell grid, once
#   bash soundness/run_handoff.sh --calibrate  # seed a KNOWN silence outside the register; must FAIL
#   bash soundness/run_handoff.sh --lv         # the checked-in 68-cell lv fixture (soundness/handoff/lv)
#   CANDOR_SCAN_BIN=/path/to/candor-scan bash soundness/run_handoff.sh 20
#
# Exit 0 = clean, 1 = a real finding, 2 = harness/build error, 3 = SELFSKIP (stated reason).
set -uo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
LABEL="soundness (hand-off)"

command -v cargo >/dev/null 2>&1 || { echo "$LABEL: SELFSKIP — no cargo on PATH"; echo "RESULT: SELFSKIP"; exit 3; }
python3 -c 'import json' >/dev/null 2>&1 || { echo "$LABEL: SELFSKIP — no python3"; echo "RESULT: SELFSKIP"; exit 3; }

GEN_TOOLCHAIN="${CANDOR_GEN_TOOLCHAIN:-stable}"
gcargo() {
  if [ -n "$GEN_TOOLCHAIN" ]; then RUSTUP_TOOLCHAIN="$GEN_TOOLCHAIN" RUSTFLAGS="" cargo "$@"
  else RUSTFLAGS="" cargo "$@"; fi
}
if ! gcargo --version >/dev/null 2>&1; then
  echo "$LABEL: toolchain '$GEN_TOOLCHAIN' unavailable — falling back to the active toolchain"
  GEN_TOOLCHAIN=""
fi

if [ -n "${CANDOR_SCAN_BIN:-}" ]; then
  SCAN="$CANDOR_SCAN_BIN"
  [ -x "$SCAN" ] || { echo "$LABEL: CANDOR_SCAN_BIN='$SCAN' is not executable"; echo "RESULT: ERROR"; exit 2; }
else
  echo "$LABEL: building candor-scan…"
  TDIR="${CARGO_TARGET_DIR:-$ROOT/target}"
  CARGO_TARGET_DIR="$TDIR" cargo build --release -p candor-scan --manifest-path "$ROOT/Cargo.toml" >/dev/null 2>&1 \
    || { echo "$LABEL: candor-scan did not build"; echo "RESULT: ERROR"; exit 2; }
  SCAN="$TDIR/release/candor-scan"
fi

ALL=""; CAL=""
if [ "${1:-}" = "--lv" ]; then
  # THE CHECKED-IN FIXTURE (rustagent-liveness's lv, 68 cells over R198/R200/R323 spellings). Same rule as
  # the generated arm: a cell that drops must be charged or disclosed unless lv.expect.tsv lists it.
  WORK="$(mktemp -d)"; trap 'rm -rf "$WORK"' EXIT
  cp -R "$ROOT/soundness/handoff/lv" "$WORK/lv"
  if ! ( cd "$WORK/lv" && CARGO_TARGET_DIR="$WORK/t" gcargo run -q --bin probe >"$WORK/gt.tsv" 2>"$WORK/gt.err" ); then
    echo "$LABEL: lv fixture did not build/run"; sed -n 1,20p "$WORK/gt.err"; echo "RESULT: ERROR"; exit 2
  fi
  ( cd "$WORK/lv" && "$SCAN" . --out "$WORK/out/r" >/dev/null 2>"$WORK/scan.err" )
  python3 - "$WORK" "$ROOT/soundness/handoff/lv.expect.tsv" <<'PY'
import glob, json, sys
w, exp = sys.argv[1], sys.argv[2]
expect = {}
for l in open(exp):
    if l.startswith("#") or not l.strip():
        continue
    p = l.rstrip("\n").split("\t")
    expect[p[0]] = p[1]
gt = {}
for l in open(w + "/gt.tsv"):
    k, v = l.rstrip("\n").split("\t")
    gt[k.split("[")[0].split("(")[0]] = int(v)
rows = {}
for f in glob.glob(w + "/out/r*.json"):
    if f.endswith((".callgraph.json", ".hierarchy.json", ".locs.json")):
        continue
    for r in json.load(open(f)).get("functions", []):
        rows[r["fn"]] = r
if not rows or len(gt) < 60:
    print("lv: ERROR — %d rows, %d executed cells: not a measurement" % (len(rows), len(gt))); print("RESULT: ERROR"); sys.exit(2)
bad = []
for fn, n in gt.items():
    seen = bool(rows.get(fn, {}).get("inferred"))
    if n > 0 and not seen and expect.get(fn) != "SILENT":
        bad.append("SILENT (not stated): %s (executed %d drop(s))" % (fn, n))
    if n > 0 and seen and expect.get(fn) == "SILENT":
        print("  now CHARGED, still stated SILENT (shrink lv.expect.tsv): " + fn)
    if n == 0 and seen and expect.get(fn) != "OVER":
        print("  charged at 0 executed drops, not stated OVER: " + fn)
print("soundness (hand-off): lv — %d cells executed, %d stated silent, %d stated over" % (
    len(gt), sum(1 for v in expect.values() if v == "SILENT"), sum(1 for v in expect.values() if v == "OVER")))
for b in bad:
    print("  " + b)
print("RESULT: " + ("FAIL" if bad else "OK"))
sys.exit(1 if bad else 0)
PY
  exit $?
fi
case "${1:-}" in
  --all) ALL="--all"; SEEDS="${SEEDS:-1}" ;;
  --calibrate) CAL=1; SEEDS="${SEEDS:-1}" ;;
  *) SEEDS="${SEEDS:-$(seq 1 "${1:-20}")}" ;;
esac

WORK="$(mktemp -d)"; trap 'rm -rf "$WORK"' EXIT
GT_TARGET="$WORK/gt-target"   # one shared target dir: the cells differ, the deps do not
fail=0; err=0; cells=0
for s in $SEEDS; do
  d="$WORK/s$s"
  if [ -n "$CAL" ]; then
    CANDOR_HANDOFF_SEED_SILENCE=1 python3 "$ROOT/soundness/gen_drop.py" --handoff "$s" "$d" || { err=1; continue; }
  else
    python3 "$ROOT/soundness/gen_drop.py" --handoff "$s" "$d" $ALL || { err=1; continue; }
  fi
  if ! ( cd "$d" && CARGO_TARGET_DIR="$GT_TARGET" gcargo run -q --bin gt >"$d/gt.tsv" 2>"$d/gt.err" ); then
    echo "  seed $s: GENERATOR BUG — the crate does not build/run"; sed -n 1,20p "$d/gt.err"; err=1; continue
  fi
  rm -rf "$d/target"
  ( cd "$d" && "$SCAN" . --out "$d/out/r" >/dev/null 2>"$d/scan.err" )
  if ! ls "$d"/out/r*.json >/dev/null 2>&1; then
    echo "  seed $s: NO REPORT"; sed -n 1,10p "$d/scan.err"; err=1; continue
  fi
  res="$(python3 - "$d" "$ROOT/soundness/known_open_handoff.tsv" <<'PY'
import glob, json, sys
d, reg = sys.argv[1], sys.argv[2]
known = set()
for l in open(reg):
    p = l.rstrip("\n").split("\t")
    if len(p) == 3 and p[0] == "h":
        known.add(p[2])
truth = json.load(open(d + "/truth.json"))["shapes"]
gt = {}
for l in open(d + "/gt.tsv"):
    k, v = l.rstrip("\n").split("\t")
    gt[k] = int(v)
rows = {}
for f in glob.glob(d + "/out/r*.json"):
    if f.endswith((".callgraph.json", ".hierarchy.json", ".locs.json")):
        continue
    doc = json.load(open(f))
    for r in doc.get("functions", []):
        rows[r["fn"]] = r
# R242 — a hollow scan judges nothing; refuse to read its silence as anything.
if not rows:
    print("ERROR no rows at all — a hollow scan cannot be a clean result"); sys.exit(0)
new, debt, closed, over = [], [], [], []
for name, shape in truth.items():
    if name not in gt:
        print("ERROR cell %s has no executed ground truth" % name); sys.exit(0)
    inf = rows.get(name, {}).get("inferred", [])
    seen = bool(inf)
    if gt[name] > 0 and not seen:
        (debt if shape in known else new).append(shape)
    elif gt[name] > 0 and seen and shape in known:
        closed.append(shape)
    elif gt[name] == 0 and seen:
        over.append(shape)
print("CELLS %d NEW %d DEBT %d CLOSED %d OVER %d" % (len(truth), len(new), len(debt), len(closed), len(over)))
for x in new: print("  SILENT (not in the register): " + x)
for x in closed: print("  now CHARGED, still in the register (shrink it): " + x)
for x in over: print("  charged at GT 0 (over-report, not a failure): " + x)
PY
)"
  echo "$res" | sed "s/^/  seed $s: /" | grep -v "NEW 0 DEBT" || true
  echo "$res" | grep -q '^ERROR' && err=1
  n="$(echo "$res" | sed -n 's/^CELLS \([0-9]*\) NEW \([0-9]*\).*/\1 \2/p')"
  cells=$((cells + ${n%% *}))
  [ "${n##* }" != "0" ] && fail=1
done

if [ -n "$CAL" ]; then
  # The calibration PASSES only when the seeded silence was reported as a failure.
  if [ "$fail" = 1 ] && [ "$err" = 0 ]; then echo "$LABEL: calibration OK — the seeded silence was caught"; echo "RESULT: OK"; exit 0; fi
  echo "$LABEL: CALIBRATION FAILED — a known silence outside the register went unreported"; echo "RESULT: FAIL"; exit 1
fi
echo "$LABEL: $cells cell(s) checked"
echo "$LABEL: known-open debt (kind h):"; grep -c '^h	' "$ROOT/soundness/known_open_handoff.tsv" | sed 's/^/  /'
[ "$err" = 1 ] && { echo "RESULT: ERROR"; exit 2; }
[ "$fail" = 1 ] && { echo "RESULT: FAIL"; exit 1; }
echo "RESULT: OK"
