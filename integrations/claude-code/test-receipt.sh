#!/usr/bin/env bash
# Tests for candor-run.sh's receipt logic — coverage detection, freshness, and the
# sidecar-vs-report glob (the bug that once made the calibrated/encountered sidecars get
# parsed as reports). Runs WITHOUT a `cargo dylint` build: we pre-populate `.candor` with
# fixture reports and freeze the source hash so the script treats them as current.
set -uo pipefail
HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
RUN="$HERE/candor-run.sh"
command -v python3 >/dev/null || { echo "SKIP: python3 required"; exit 0; }

pass=0; fail=0
chk()  { if printf '%s' "$2" | grep -qE -- "$3"; then echo "  ok   $1"; pass=$((pass+1)); else echo "  FAIL $1 — want /$3/"; echo "       in: $2"; fail=$((fail+1)); fi; }
nchk() { if printf '%s' "$2" | grep -qE -- "$3"; then echo "  FAIL $1 — unwanted /$3/"; fail=$((fail+1)); else echo "  ok   $1"; pass=$((pass+1)); fi; }

T=$(mktemp -d)/proj; mkdir -p "$T/src" "$T/.candor"
printf '[package]\nname="proj"\nversion="0.1.0"\nedition="2021"\n[dependencies]\n' > "$T/Cargo.toml"
printf 'fn a(){}\nfn b(){}\n' > "$T/src/lib.rs"

# Three real report entries…
cat > "$T/.candor/report.proj.Rlib.json" <<'JSON'
[{"fn":"a","loc":"src/lib.rs:1","inferred":["Fs"],"direct":["Fs"],"unresolved":false},
 {"fn":"b","loc":"src/lib.rs:2","inferred":["Net","Unknown"],"direct":["Net"],"unresolved":true},
 {"fn":"c","loc":"src/lib.rs:3","inferred":["Db"],"direct":["Db"],"unresolved":false}]
JSON
# …and the two sidecars that must NOT be counted as functions (the glob-collision regression).
echo '{"crates":["reqwest"],"prefixes":["aws_sdk_"]}' > "$T/.candor/report.calibrated.json"
echo '["scylla","serde","tokio"]' > "$T/.candor/report.encountered-proj-Rlib.json"

# Freeze the source hash so candor-run treats the report as CURRENT (no dylint re-run).
hash=$(find "$T" -name '*.rs' -not -path '*/target/*' -not -path '*/.git/*' -print0 \
       | sort -z | xargs -0 shasum 2>/dev/null | shasum | cut -d' ' -f1)
echo "$hash" > "$T/.candor/state"

out=$("$RUN" "$T" 2>/dev/null)
echo "receipt: $out"
chk  "counts 3 fns (sidecars excluded — the glob-collision regression)" "$out" '3 fns'
chk  "effect breakdown rendered"                                        "$out" '(Db|Net|Fs)'
chk  "1 unresolved (b carries Unknown)"                                 "$out" '1 unresolved'
chk  "freshness = current (source hash unchanged)"                      "$out" 'current @'
chk  "coverage flags the uncalibrated effectful dep (scylla)"           "$out" 'scylla'
nchk "calibrated/std deps NOT flagged (reqwest/serde/tokio)"            "$out" '(reqwest|serde|tokio)'

# A real source change must flip freshness off "current".
printf 'fn a(){}\nfn b(){}\nfn d(){}\n' > "$T/src/lib.rs"
out2=$("$RUN" "$T" 2>/dev/null || true)
nchk "after an edit, no longer reports 'current'"                       "$out2" 'current @'
rm -rf "$(dirname "$T")"

# The receipt must stamp the dylib's TRUE build version (read from the embedded tag), not a git
# guess — the honest-provenance fix. Needs a built dylib + strings; skip otherwise.
LIBV=$(find "$HERE/../../target/debug" -maxdepth 1 \( -name 'libcandor@*.dylib' -o -name 'libcandor@*.so' \) 2>/dev/null | head -1)
tag=""; [ -n "$LIBV" ] && command -v strings >/dev/null 2>&1 && \
  tag=$(strings -a "$LIBV" 2>/dev/null | grep -oE 'candor-build-version=[0-9a-fA-F]+' | head -1 | cut -d= -f2)
if [ -n "$tag" ]; then
  V=$(mktemp -d)/p; mkdir -p "$V/src" "$V/.candor"
  printf '[package]\nname="p"\nversion="0.1.0"\nedition="2021"\n[dependencies]\n' > "$V/Cargo.toml"
  printf 'fn a(){}\n' > "$V/src/lib.rs"
  echo '[{"fn":"a","loc":"src/lib.rs:1","inferred":["Fs"],"direct":["Fs"],"unresolved":false}]' > "$V/.candor/report.p.Rlib.json"
  echo "CANDOR_LIB=$LIBV" > "$V/.candor/config"
  vh=$(find "$V" -name '*.rs' -not -path '*/target/*' -print0 | sort -z | xargs -0 shasum 2>/dev/null | shasum | cut -d' ' -f1)
  echo "$vh" > "$V/.candor/state"
  outv=$("$RUN" "$V" 2>/dev/null)
  chk "receipt stamps the dylib's true build version (@$tag)"           "$outv" "@$tag"
  rm -rf "$(dirname "$V")"
else
  echo "  skip version-stamp check (no built dylib / no strings)"
fi
echo
echo "receipt tests: $pass passed, $fail failed"
[ "$fail" -eq 0 ]
