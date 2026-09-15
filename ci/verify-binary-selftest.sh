#!/usr/bin/env bash
# verify-binary-selftest.sh — prove `ci/verify-binary.sh` can FAIL.
#
# WHY. `verify-binary.sh` is the only thing standing between a broken build and a published release
# asset, and it is a gate whose green is otherwise unfalsifiable: if an edit made it vacuous — a
# threshold inverted, a check made unreachable, a `python3` block that silently exits 0 — every release
# would keep going green and the next empty binary would ship. That is the shape candor-java shipped at
# v0.32.0: two native binaries that ran, exited 0 and reported an EMPTY scan, 0 functions where the jar
# found 210. Its `ci/native-parity-selftest.sh` exists for the same reason and runs in `ci.yml`, where
# no GraalVM is needed; this is that idea for the packaged Rust binaries.
#
# The arms below are STUBS, not real engines, so this needs no build and no network and runs in `ci.yml`
# on every push — the gate that guards the gate has to be cheaper than the thing it guards, or it gets
# skipped when it matters.
set -uo pipefail

HERE="$(cd "$(dirname "$0")" && pwd)"
VERIFY="$HERE/verify-binary.sh"
WORK="$(mktemp -d)"; trap 'rm -rf "$WORK"' EXIT
fails=0

# A stub candor-scan whose report content is whatever we hand it, so each arm below can produce exactly
# one defect and nothing else. `--version` is answered separately so the version arm can differ.
mkstub() {  # $1 = dir, $2 = version string, $3 = functions JSON, $4 = analyzed count
  mkdir -p "$1"
  cat > "$1/candor-scan" <<EOF
#!/bin/sh
case "\$1" in --version) echo "$2"; exit 0;; esac
out=""; while [ \$# -gt 0 ]; do [ "\$1" = "--out" ] && out="\$2"; shift; done
printf '%s\n' '{"candor":{"version":"x","toolchain":"y","spec":"0.38"},"functions":$3,"analyzed":{"count":$4}}' > "\$out"
exit 0
EOF
  # candor-query answers anything containing Fs, so only the arm that targets IT can fail on it.
  cat > "$1/candor-query" <<EOF
#!/bin/sh
case "\$1" in --version) echo "$2"; exit 0;; esac
echo "Fs: something"
exit 0
EOF
  chmod +x "$1/candor-scan" "$1/candor-query"
}

# The shape of a healthy report: enough functions, enough analyzed, and the required effect set.
GOOD_FNS='[{"fn":"a","inferred":["Fs"]},{"fn":"b","inferred":["Env"]},{"fn":"c","inferred":["Exec"]},{"fn":"d","inferred":["Clock"]},{"fn":"e","inferred":["Fs"]},{"fn":"f","inferred":["Fs"]},{"fn":"g","inferred":["Fs"]},{"fn":"h","inferred":["Fs"]},{"fn":"i","inferred":["Fs"]},{"fn":"j","inferred":["Fs"]}]'

check() {  # $1 = label, $2 = dir, $3 = expected rc (0 pass / 1 refuse), $4 = version arg
  local out rc
  out="$("$VERIFY" "$2/candor-scan" "$2/candor-query" "${4:-}" 2>&1)"; rc=$?
  if [ "$rc" -eq "$3" ]; then
    echo "  ok   $1"
  else
    echo "  ✘    $1 — expected rc=$3, got rc=$rc"
    printf '%s\n' "$out" | sed 's/^/         | /'
    fails=$((fails+1))
  fi
}

echo "verify-binary-selftest: the gate must PASS a sound binary and REFUSE each defect it exists for"

# THE POSITIVE ARM FIRST. A checker that only ever refuses is as useless as one that only ever passes,
# and it is the arm that catches a threshold raised past what a real engine produces.
mkstub "$WORK/good" "candor-scan 0.38.0 (spec 0.38)" "$GOOD_FNS" 12
check "a sound binary PASSES" "$WORK/good" 0 0.38.0

# 1. THE v0.32.0 DEFECT: runs, exits 0, finds nothing.
mkstub "$WORK/empty" "candor-scan 0.38.0 (spec 0.38)" '[]' 0
check "an EMPTY report is refused" "$WORK/empty" 1 0.38.0

# 2. Analysed something, but far less than the fixture demonstrably contains — the partial version of
#    the same defect, which a "did it produce a report at all" check would wave through.
mkstub "$WORK/thin" "candor-scan 0.38.0 (spec 0.38)" '[{"fn":"a","inferred":["Fs"]}]' 2
check "an UNDER-REPORTING binary is refused" "$WORK/thin" 1 0.38.0

# 3. Right counts, missing an effect CLASS. Catches a build that lost a classifier table rather than
#    the whole scan — invisible to any count-only threshold.
MISSING_EXEC="$(printf '%s' "$GOOD_FNS" | sed 's/"Exec"/"Fs"/')"
mkstub "$WORK/noexec" "candor-scan 0.38.0 (spec 0.38)" "$MISSING_EXEC" 12
check "a MISSING EFFECT CLASS is refused" "$WORK/noexec" 1 0.38.0

# 4. Sound report, wrong build. The only arm that can catch a binary built from the wrong ref — and the
#    one that caught a stale 0.37.0 target/release on this checker's first real run.
# A version that can never be a live floor. Using the IMMEDIATELY-PRIOR one (0.37) made this fixture
# impersonate the exact string a real bump-miss produces, and `release-preflight [2]` flagged it as one
# on the very next cut. Any wrong version proves this arm; an ancient one proves it without ever
# colliding with a floor again. (Same rule as historical prose: do not pin a fixture to a value that
# has to move.)
mkstub "$WORK/oldver" "candor-scan 0.1.0 (spec 0.1)" "$GOOD_FNS" 12
check "a WRONG VERSION is refused" "$WORK/oldver" 1 0.38.0

# 5. …and with no expected version passed, that same binary must PASS — otherwise the version check is
#    firing on something other than the version, and arm 4 proves nothing.
check "…and passes when no version is demanded" "$WORK/oldver" 0 ""

# 6. The QUERY half. A working scanner beside a broken query is the asymmetry that started all this.
mkstub "$WORK/badq" "candor-scan 0.38.0 (spec 0.38)" "$GOOD_FNS" 12
printf '#!/bin/sh\ncase "$1" in --version) echo "candor-query 0.38.0 (spec 0.38)"; exit 0;; esac\nexit 3\n' \
  > "$WORK/badq/candor-query"; chmod +x "$WORK/badq/candor-query"
check "a BROKEN candor-query is refused" "$WORK/badq" 1 0.38.0

echo
if [ "$fails" -eq 0 ]; then
  echo "verify-binary-selftest: OK — 7 arms, both directions: a sound binary passes, and an empty report,"
  echo "  an under-report, a missing effect class, a wrong version and a broken query are each refused."
  exit 0
fi
echo "verify-binary-selftest: FAILED — $fails arm(s). The release-asset gate is not measuring what it claims." >&2
exit 1
