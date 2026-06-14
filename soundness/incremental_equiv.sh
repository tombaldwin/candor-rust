#!/usr/bin/env bash
# incremental_equiv.sh — the EQUIVALENCE FUZZER and permanent guard for candor-scan's --incremental cache.
#
# CORRECTNESS IS THE WHOLE FEATURE. An incremental scan MUST produce a report (and call-graph sidecar)
# BYTE-FOR-BYTE IDENTICAL to a full scan-from-scratch, for ANY sequence of edits. This fuzzer proves it:
# it clones a real multi-file crate, seeds the cache with one full scan, then performs MANY random edits —
# modify a fn body, add/remove a fn, change a struct field, add a use, create a new file, delete a file,
# rename a file — and after EACH edit runs BOTH:
#     (a) an INCREMENTAL scan  (reusing the cache), and
#     (b) a FULL scan-from-scratch  (a pristine copy of the same edited tree, no cache),
# then asserts the two reports + call-graph sidecars are byte-identical. A SINGLE mismatch is a FAILURE:
# the cache served a stale/wrong answer, the cardinal sin. Run it to a clean pass; keep it in the repo.
#
#   bash soundness/incremental_equiv.sh [N_EDITS] [SEED]
#     N_EDITS  number of random edits to apply (default 120)
#     SEED     RNG seed for reproducibility (default: time-based)
#
# Target crate: tokio (a large, idiomatic multi-file crate) from the cargo registry, else ripgrep, else
# the candor-scan crate itself. Override with CRATE=/path/to/crate.
set -uo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

N_EDITS="${1:-120}"
SEED="${2:-$RANDOM}"

echo "incremental_equiv: building candor-scan (release)…"
cargo build -q --release -p candor-scan --manifest-path "$ROOT/Cargo.toml" \
  || { echo "FAIL: candor-scan did not build"; exit 1; }
BIN="$ROOT/target/release/candor-scan"
[ -x "$BIN" ] || { echo "FAIL: no candor-scan binary at $BIN"; exit 1; }

# --- locate a real multi-file target crate ----------------------------------------------------------
CRATE="${CRATE:-}"
if [ -z "$CRATE" ]; then
  for c in ~/.cargo/registry/src/*/tokio-1*/ ~/.cargo/registry/src/*/ripgrep-*/ "$ROOT/crates/candor-scan"; do
    [ -d "$c" ] && [ -f "$c/Cargo.toml" ] && { CRATE="$c"; break; }
  done
fi
[ -n "$CRATE" ] && [ -d "$CRATE" ] || { echo "FAIL: no target crate found (set CRATE=…)"; exit 1; }
echo "incremental_equiv: target crate = $CRATE"
echo "incremental_equiv: $N_EDITS edits, seed=$SEED"

WORK="$(mktemp -d)"; trap 'rm -rf "$WORK"' EXIT
LIVE="$WORK/live"      # the evolving tree (carries the incremental cache)
cp -r "$CRATE" "$LIVE"
rm -rf "$LIVE/.candor" "$LIVE/target"

# Seed the cache with one full incremental scan.
"$BIN" "$LIVE" --incremental --out "$WORK/seed" >/dev/null 2>&1

# The Python driver applies the random edits and runs the byte-identity assertion. It owns the edit
# classes + the RNG so the sequence is reproducible from SEED.
BIN="$BIN" LIVE="$LIVE" WORK="$WORK" N_EDITS="$N_EDITS" SEED="$SEED" python3 - <<'PY'
import os, sys, random, subprocess, glob, shutil, hashlib

BIN  = os.environ["BIN"]
LIVE = os.environ["LIVE"]
WORK = os.environ["WORK"]
N    = int(os.environ["N_EDITS"])
SEED = int(os.environ["SEED"])
rng  = random.Random(SEED)

def src_files():
    out = []
    for dp, dn, fn in os.walk(LIVE):
        if ".candor" in dp or "/target" in dp or "/tests" in dp or "/benches" in dp or "/examples" in dp:
            continue
        for f in fn:
            if f.endswith(".rs"):
                out.append(os.path.join(dp, f))
    return out

def report_digest(prefix_dir, tag):
    # Hash every report + callgraph sidecar this scan wrote, keyed by filename, into one stable digest.
    h = hashlib.sha256()
    files = sorted(glob.glob(os.path.join(prefix_dir, f"{tag}.*.json")))
    for f in files:
        h.update(os.path.basename(f).replace(tag, "T").encode())
        with open(f, "rb") as fh:
            h.update(fh.read())
    return h.hexdigest(), len(files)

def scan(tree, tag, incremental):
    args = [BIN, tree, "--out", os.path.join(WORK, tag)]
    if incremental:
        args.insert(2, "--incremental")
    subprocess.run(args, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
    return report_digest(WORK, tag)

# --- edit classes -----------------------------------------------------------------------------------
counter = [0]
def uniq():
    counter[0] += 1
    return f"{counter[0]}_{rng.randint(0,1<<30)}"

def edit_body(f):
    # Append a fn whose body calls a real effectful path — exercises Pass B resolution + classification.
    u = uniq()
    eff = rng.choice([
        'std::fs::read("/tmp/x").ok();',
        'std::process::Command::new("ls");',
        'std::env::var("X").ok();',
        'let _ = 1 + 1;',                       # pure
        'std::net::TcpStream::connect("h:1").ok();',
    ])
    with open(f, "a") as fh:
        fh.write(f"\npub fn fuzz_fn_{u}() {{ {eff} }}\n")
    return "body"

def remove_last_fn(f):
    # Remove a previously-appended fuzz fn (decl-changing: a fn disappears from the resolution index).
    txt = open(f).read()
    lines = txt.splitlines(keepends=True)
    for i in range(len(lines)-1, -1, -1):
        if lines[i].lstrip().startswith("pub fn fuzz_fn_"):
            del lines[i]
            open(f, "w").write("".join(lines))
            return "remove_fn"
    return edit_body(f)

def add_struct_field(f):
    # Append a struct with a typed field (decl-changing: feeds the field index, can retype a method recv).
    u = uniq()
    ty = rng.choice(["std::process::Command", "String", "reqwest::Client", "u64"])
    with open(f, "a") as fh:
        fh.write(f"\npub struct FuzzS_{u} {{ pub fld: {ty} }}\n")
    return "struct_field"

def add_use(f):
    u = rng.choice(["std::fs", "std::env", "std::process::Command", "std::collections::HashMap"])
    with open(f, "a") as fh:
        fh.write(f"\n#[allow(unused_imports)] use {u};\n")
    return "use"

def new_file(_):
    d = os.path.join(LIVE, "src")
    if not os.path.isdir(d):
        d = LIVE
    u = uniq()
    p = os.path.join(d, f"fuzz_mod_{u}.rs")
    with open(p, "w") as fh:
        fh.write(f"pub fn nf_{u}() {{ std::fs::read(\"/tmp/y\").ok(); }}\n")
    return "new_file"

def delete_file(_):
    fuzz = [f for f in src_files() if os.path.basename(f).startswith("fuzz_mod_")]
    if not fuzz:
        return new_file(_)
    os.remove(rng.choice(fuzz))
    return "delete_file"

def rename_file(_):
    fuzz = [f for f in src_files() if os.path.basename(f).startswith("fuzz_mod_")]
    if not fuzz:
        return new_file(_)
    f = rng.choice(fuzz)
    u = uniq()
    shutil.move(f, os.path.join(os.path.dirname(f), f"fuzz_ren_{u}.rs"))
    return "rename_file"

def mutate_in_place(f):
    # Edit EXISTING content: flip an effectful call to a different effect inside a prior fuzz fn (a
    # body-only change to an already-cached file), or no-op fall back to appending. Stresses re-deriving
    # a file whose bytes change but whose decls may or may not.
    txt = open(f).read()
    repls = [
        ('std::fs::read("/tmp/x")',  'std::env::var("Z")'),
        ('std::env::var("X")',       'std::fs::read("/tmp/q")'),
        ('std::process::Command::new("ls")', 'std::process::Command::new("ps")'),
    ]
    for a, b in repls:
        if a in txt:
            open(f, "w").write(txt.replace(a, b, 1))
            return "mutate"
    return edit_body(f)

def retype_struct_field(f):
    # Change an existing fuzz struct's field type (a DECL change that can retype method resolution
    # crate-wide) — the index-hash-bump path, the correctness linchpin of the cache.
    txt = open(f).read()
    import re
    m = re.search(r"(pub struct FuzzS_\w+ \{ pub fld: )([\w:]+)( \})", txt)
    if not m:
        return add_struct_field(f)
    new_ty = "reqwest::Client" if "Command" in m.group(2) else "std::process::Command"
    open(f, "w").write(txt[:m.start()] + m.group(1) + new_ty + m.group(3) + txt[m.end():])
    return "retype_field"

CLASSES = [edit_body, edit_body, mutate_in_place, mutate_in_place, remove_last_fn,
           add_struct_field, retype_struct_field, add_use, new_file, delete_file, rename_file]

fails = 0
hist = {}
for step in range(1, N+1):
    files = src_files()
    if not files:
        new_file(None); files = src_files()
    op = rng.choice(CLASSES)
    if op in (new_file, delete_file, rename_file):
        kind = op(None)
    else:
        kind = op(rng.choice(files))
    hist[kind] = hist.get(kind, 0) + 1

    # Incremental scan of the LIVE tree (carries the cache from prior steps).
    inc_dig, inc_n = scan(LIVE, "inc", incremental=True)
    # Full scan-from-scratch of a PRISTINE COPY of the same edited tree (no cache at all).
    fresh = os.path.join(WORK, "fresh")
    shutil.rmtree(fresh, ignore_errors=True)
    shutil.copytree(LIVE, fresh, ignore=shutil.ignore_patterns(".candor", "target"))
    full_dig, full_n = scan(fresh, "full", incremental=False)

    if inc_dig != full_dig or inc_n != full_n:
        fails += 1
        print(f"  MISMATCH at step {step} (op={kind}): inc={inc_dig[:12]}({inc_n}) full={full_dig[:12]}({full_n})")
        # Dump a diff of the first differing report for root-causing.
        for inc_f in sorted(glob.glob(os.path.join(WORK, "inc.*.json"))):
            full_f = inc_f.replace("inc.", "full.")
            if os.path.exists(full_f):
                a = open(inc_f).read(); b = open(full_f).read()
                if a != b:
                    print(f"    first differing report: {os.path.basename(inc_f)}")
                    da = subprocess.run(["diff", full_f, inc_f], capture_output=True, text=True).stdout
                    print("\n".join(da.splitlines()[:30]))
                    break
        if fails >= 3:
            print("incremental_equiv: stopping after 3 mismatches")
            break

print()
print(f"incremental_equiv: edit mix: {hist}")
if fails == 0:
    print(f"incremental_equiv: OK — {N} edits, every incremental scan byte-identical to a full scan")
    sys.exit(0)
else:
    print(f"incremental_equiv: FAIL — {fails} mismatch(es)")
    sys.exit(1)
PY
