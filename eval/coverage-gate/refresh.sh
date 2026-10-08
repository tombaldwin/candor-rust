#!/usr/bin/env bash
# eval/coverage-gate/refresh.sh — the coverage-gate-refresh workflow's regenerate-and-diff, as ONE script so
# it runs identically on a laptop and on the runner (the workflow calls this; it does not re-spell it).
#
# Usage: eval/coverage-gate/refresh.sh REGISTRY_SRC OUT_DIR [GITHUB_OUTPUT]
#   REGISTRY_SRC  ~/.cargo/registry/src/index.crates.io-HASH, already `cargo fetch`ed for the fixture
#   OUT_DIR       receives fresh/{covered,open}.tsv and, when built, baseline/{covered,open}.tsv
# Requires `target/release/candor-scan` built from the tree being checked. Exits as `--diff-manifests` does.
#
# ENGINE CHANGE vs CRATES.IO DRIFT. A grown row has two possible causes that need different reviews: the
# upstream source moved, or OUR engine/classify changed since the manifest was written (run 37608278047:
# all 74 "drift" rows were the second). So the generator is run a SECOND time, over the SAME fetch, at the
# commit that last wrote the manifests — the manifest's own engine — and `--diff-manifests --baseline`
# splits every finding on it. When the engine-relevant paths are unchanged since that commit the fresh run
# IS the baseline and nothing is rebuilt. Run locally over a DIRTY tree, "HEAD" means the working tree.
set -euo pipefail
REG=$1; OUT=$2; GHO=${3:-}
ROOT=$(git rev-parse --show-toplevel); cd "$ROOT"
CG=eval/coverage-gate
mkdir -p "$OUT/fresh" "$OUT/baseline"

python3 "$CG/generate.py" --registry "$REG"
cp "$CG/covered.tsv" "$CG/open.tsv" "$OUT/fresh/"
git checkout -- "$CG/covered.tsv" "$CG/open.tsv"   # the checked-in files are what the diff reads

M=$(git log -1 --format=%H -- "$CG/open.tsv" "$CG/covered.tsv")
ENGINE_PATHS=(crates/candor-scan crates/candor-classify crates/candor-report Cargo.lock "$CG/generate.py" "$CG/classify_check")
if git diff --quiet "$M" -- "${ENGINE_PATHS[@]}"; then
  echo "refresh: engine paths unchanged since the manifest's commit ${M:0:12} — the fresh run is the baseline"
  cp "$OUT/fresh/"*.tsv "$OUT/baseline/"
else
  echo "refresh: engine changed since the manifest's commit ${M:0:12} — regenerating with that engine"
  BASE=$(mktemp -d "${TMPDIR:-/tmp}/cg-baseline.XXXXXX")
  git worktree add --detach "$BASE/wt" "$M" >/dev/null
  trap 'git worktree remove --force "$BASE/wt" >/dev/null 2>&1 || true; rm -rf "${BASE:?}"' EXIT
  (cd "$BASE/wt" && cargo build --release -p candor-scan && python3 "$CG/generate.py" --registry "$REG")
  cp "$BASE/wt/$CG/covered.tsv" "$BASE/wt/$CG/open.tsv" "$OUT/baseline/"
fi

args=(--diff-manifests "$CG/covered.tsv" "$OUT/fresh/covered.tsv" "$CG/open.tsv" "$OUT/fresh/open.tsv"
      --baseline "$OUT/baseline/covered.tsv" "$OUT/baseline/open.tsv")
[ -n "$GHO" ] && args+=(--github-output "$GHO")
python3 "$CG/generate.py" "${args[@]}"
