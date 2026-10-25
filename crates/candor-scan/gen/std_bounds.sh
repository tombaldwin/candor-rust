#!/usr/bin/env bash
# gen/std_bounds.sh [--check] — regenerate src/std_bounds.tsv (call-target round 2's std bound table) from the rust-src
# of the PINNED toolchain, or (--check) verify the committed table reproduces from it byte for byte, the way java's
# `soundness/kappa_table/derive.sh --check` pins κ.
#
# The pin is the `rustc -V` line in the table's first line. The rust-src read is that toolchain's
# (`rustup component add rust-src --toolchain <channel>`); a different toolchain is refused rather than compared,
# because a different std is a different table, not a failed reproduction.
set -uo pipefail
HERE="$(cd "$(dirname "$0")" && pwd)"; CRATE="$(cd "$HERE/.." && pwd)"; ROOT="$(cd "$CRATE/../.." && pwd)"
TABLE="$CRATE/src/std_bounds.tsv"
TC="${STD_BOUNDS_TOOLCHAIN:-stable}"
RUSTC_V="$(rustup run "$TC" rustc -V)" || { echo "std_bounds.sh: no toolchain '$TC'" >&2; exit 2; }
SRC="$(rustup run "$TC" rustc --print sysroot)/lib/rustlib/src/rust/library"
[ -d "$SRC/core/src" ] || { echo "std_bounds.sh: no rust-src for $TC ($SRC) — rustup component add rust-src --toolchain $TC" >&2; exit 2; }
if [ "${1:-}" = "--check" ]; then
  PIN="$(head -1 "$TABLE" | sed -n 's/.*rust-src of \(rustc [^—]*\) — .*/\1/p' | sed 's/ *$//')"
  if [ "$PIN" != "$RUSTC_V" ]; then
    echo "std_bounds.sh: the table is pinned to '$PIN'; toolchain '$TC' is '$RUSTC_V' — set STD_BOUNDS_TOOLCHAIN to the pinned one" >&2
    exit 2
  fi
  export STD_BOUNDS_CHECK=1
fi
STD_BOUNDS_SRC="$SRC" STD_BOUNDS_RUSTC="$RUSTC_V" STD_BOUNDS_OUT="$TABLE" \
  cargo test -q --manifest-path "$ROOT/Cargo.toml" -p candor-scan --release std_bounds_gen::regenerate -- --ignored --exact
