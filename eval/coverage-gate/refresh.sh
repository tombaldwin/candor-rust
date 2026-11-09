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
cp "$CG/covered.tsv" "$CG/open.tsv" "$CG/unreachable.tsv" "$OUT/fresh/"
git checkout -- "$CG/covered.tsv" "$CG/open.tsv" "$CG/unreachable.tsv"   # the checked-in files are what the diff reads

# THE BASELINE ENGINE (SOUNDNESS R1507). It must be the engine that last REGENERATED the manifests, which
# `git log -1 -- open.tsv covered.tsv` is NOT: a splice commit (rows hand-added to open.tsv, nothing
# regenerated) moves it, and every engine change since the real regeneration is then labelled crates.io
# drift. Two sources, in order:
#   1. the `# engine-sha:` stamp generate.py writes into both manifests at regeneration. A splice leaves it
#      alone. Used only when both manifests agree, the sha names a commit, and it is an ancestor of HEAD.
#   2. otherwise (unstamped history, a dirty-tree generation, a rewritten history) the newest commit that
#      touched the manifests and is NOT a splice. A splice = a commit whose only change to the two files is
#      added lines in open.tsv; a regeneration rewrites covered.tsv or removes/changes open.tsv rows.
# Either way the choice is printed with its source. A wrong baseline only mislabels growth (engine vs drift);
# it can never turn a grown row into a pass, because the gate's verdict does not read the baseline.
engine_paths() { python3 "$CG/generate.py" --print-engine-paths; }
is_splice() {  # $1 = commit; true when its only change to the manifests is pure additions to open.tsv
  local nums; nums=$(git diff --numstat "$1^" "$1" -- "$CG/open.tsv" "$CG/covered.tsv" 2>/dev/null) || return 1
  git rev-parse --verify -q "$1^" >/dev/null || return 1   # a root commit is a regeneration
  [ -n "$nums" ] || return 1
  [ "$(printf '%s\n' "$nums" | awk 'NF && !($3 ~ /open\.tsv$/ && $2 == 0) {bad=1} END{print bad+0}')" = 0 ]
}
M=""; WHY=""
SO=$(python3 "$CG/generate.py" --manifest-engine "$CG/open.tsv")
SC=$(python3 "$CG/generate.py" --manifest-engine "$CG/covered.tsv")
if [ -n "$SO" ] && [ "$SO" = "$SC" ]; then
  if git cat-file -e "$SO^{commit}" 2>/dev/null && git merge-base --is-ancestor "$SO" HEAD 2>/dev/null; then
    M=$SO; WHY="engine-sha stamp"
  else
    echo "refresh: stamped engine ${SO:0:12} is not an ancestor of HEAD (or not fetched) — falling back to the history walk"
  fi
elif [ -n "$SO$SC" ]; then
  echo "refresh: the two manifests' engine stamps disagree (open='${SO:0:12}' covered='${SC:0:12}') — falling back to the history walk"
fi
if [ -z "$M" ]; then
  while read -r c; do
    if is_splice "$c"; then continue; fi
    M=$c; WHY="newest non-splice manifest commit (no usable stamp)"; break
  done < <(git log --format=%H -- "$CG/open.tsv" "$CG/covered.tsv")
  [ -n "$M" ] || { echo "refresh: no regeneration commit found in history" >&2; exit 2; }
fi
echo "refresh: baseline engine ${M:0:12} (source: $WHY)"
ENGINE_PATHS=()
while read -r p; do ENGINE_PATHS+=("$p"); done < <(engine_paths)
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

# SOUNDNESS R1528: the verdict gate reads OPEN_ENTRY_VERDICTS from THIS tree's candor-classify and
# grandfathered.tsv from THIS tree — the verdicts that clear a grown row are the ones committed at HEAD.
args=(--diff-manifests "$CG/covered.tsv" "$OUT/fresh/covered.tsv" "$CG/open.tsv" "$OUT/fresh/open.tsv"
      --baseline "$OUT/baseline/covered.tsv" "$OUT/baseline/open.tsv"
      --unreachable "$CG/unreachable.tsv" "$OUT/fresh/unreachable.tsv")
[ -n "$GHO" ] && args+=(--github-output "$GHO")
python3 "$CG/generate.py" "${args[@]}"
