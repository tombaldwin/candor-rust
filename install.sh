#!/usr/bin/env bash
# install.sh — one-shot, idempotent install so `cargo candor …` Just Works in any of your Rust
# projects, and keeps working after a `cargo clean` or a toolchain bump. Re-run any time to refresh.
#
# It does three things:
#   1. Builds the lint (rustup auto-fetches the pinned nightly + rustc-dev from rust-toolchain.toml).
#   2. Stashes the dylib + the candor-query and candor-scan binaries under ~/.candor — a STABLE home that survives a
#      `cargo clean` in this clone (which lives in target/ and would otherwise vanish).
#   3. Symlinks `cargo-candor` into ~/.cargo/bin (on PATH for every Rust user), so `cargo candor`
#      works from any directory.
#
# The pinned nightly is inherent to dylint (it links rustc internals), but rustup installs it for you
# on the build below — you never manage it by hand. Nothing here touches your projects' toolchains;
# the lint runs in its own.
set -euo pipefail

CLONE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
CACHE="${CANDOR_CACHE:-$HOME/.candor}"
BIN="${CARGO_HOME:-$HOME/.cargo}/bin"

echo "candor: installing from $CLONE" >&2

# 1. Build. rust-toolchain.toml pins the nightly + components; rustup fetches them here if missing.
echo "candor: building (rustup may fetch the pinned nightly + rustc-dev the first time)…" >&2
( cd "$CLONE" && cargo build --workspace )

LIB="$(ls "$CLONE"/target/debug/libcandor@*.dylib "$CLONE"/target/debug/libcandor@*.so 2>/dev/null | head -1 || true)"
[ -n "$LIB" ] || { echo "candor: build produced no dylib — see the errors above." >&2; exit 1; }
Q="$(ls "$CLONE"/target/debug/candor-query 2>/dev/null | head -1 || true)"
S="$(ls "$CLONE"/target/debug/candor-scan 2>/dev/null | head -1 || true)"

# 2. Stable home: dylib + query + scan binaries + a pointer back to the clone.
mkdir -p "$CACHE/lib" "$CACHE/bin"
cp -f "$LIB" "$CACHE/lib/"
[ -n "$Q" ] && cp -f "$Q" "$CACHE/bin/"
[ -n "$S" ] && cp -f "$S" "$CACHE/bin/"   # the stable scanner — usable with NO nightly toolchain
printf '%s\n' "$CLONE" > "$CACHE/clone"

# 3. `cargo candor` everywhere: symlink the wrapper onto PATH.
mkdir -p "$BIN"
ln -sf "$CLONE/cargo-candor" "$BIN/cargo-candor"

echo "candor: installed ✓" >&2
echo "  engine   $(basename "$LIB")" >&2
echo "  stable   $CACHE/  (dylib + query + scan — survives 'cargo clean')" >&2
echo "  command  $BIN/cargo-candor  →  use 'cargo candor …' in any project" >&2
case ":$PATH:" in
  *":$BIN:"*) echo "  PATH     ok" >&2 ;;
  *) echo "  ⚠ PATH   $BIN is not on your PATH — add it so 'cargo candor' resolves." >&2 ;;
esac
echo >&2
echo "  try:  cd <a rust project> && cargo candor audit" >&2
echo "  Claude Code hook (optional):  $CLONE/integrations/claude-code/install.sh" >&2
