#!/usr/bin/env bash
# ci/std-tables-check.sh — the generated std tables (`src/std_bounds.tsv`, `src/std_assoc.tsv`) must reproduce from the rust-src of the
# toolchain their first line names. Installs that toolchain's rust-src when rustup can (CI), and refuses when it cannot (exit 2 from the
# generators: a different std is a different table, not a failed reproduction).
#
#   STD_BOUNDS_TOOLCHAIN   the pinned toolchain (default 1.99.1: the `rustc -V` on the tables' first line). Change it WITH the tables.
#   STD_TABLES_NO_INSTALL=1  do not `rustup toolchain install` (a machine that already has it)
set -uo pipefail
cd "$(dirname "$0")/.." || exit 2
export STD_BOUNDS_TOOLCHAIN="${STD_BOUNDS_TOOLCHAIN:-1.99.1}"
if [ "${STD_TABLES_NO_INSTALL:-}" != 1 ]; then
  rustup toolchain install "$STD_BOUNDS_TOOLCHAIN" --profile minimal --component rust-src --no-self-update || exit 2
fi
# `cargo test ... --exact` exits 0 when NO test matched the filter (a renamed generator test): a pass over nothing, so require the count.
out=$(bash crates/candor-scan/gen/std_bounds.sh --check 2>&1) || { echo "$out"; echo "std-tables-check: std_bounds.sh --check FAILED" >&2; exit 1; }
echo "$out" | tail -n 12
echo "$out" | grep -q '^test result: ok. 1 passed' || { echo "std-tables-check: std_bounds.sh --check ran no generator test" >&2; exit 1; }
bash crates/candor-scan/gen/std_assoc.sh --check || exit 1
echo "std-tables-check: both tables reproduce"
