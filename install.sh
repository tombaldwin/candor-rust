#!/usr/bin/env bash
# install.sh — one-shot, idempotent install so `cargo candor …` Just Works in any of your Rust
# projects, and keeps working after a `cargo clean` or a toolchain bump. Re-run any time to refresh.
#
# It does three things:
#   1. Builds the STABLE tools (candor-scan + candor-query) — no nightly, no rustc-dev. candor-scan is the
#      ONLY user route. With `--oracle` it ALSO builds the rustc dylint lint, which is the family's ORACLE
#      instrument (soundness fuzzers, censuses, Self-guard), not something a user needs: rustup then
#      auto-fetches the pinned nightly + rustc-dev from rust-toolchain.toml.
#   2. Stashes the binaries (and the dylib, under --oracle) under ~/.candor — a STABLE home that survives a
#      `cargo clean` in this clone (which lives in target/ and would otherwise vanish).
#   3. Symlinks `cargo-candor` into ~/.cargo/bin (on PATH for every Rust user), so `cargo candor`
#      works from any directory.
#
# Usage:  ./install.sh [--oracle]     (CANDOR_INSTALL_ORACLE=1 is the same as --oracle)
set -euo pipefail

ORACLE="${CANDOR_INSTALL_ORACLE:-0}"
for a in "$@"; do
  case "$a" in
    --oracle) ORACLE=1 ;;
    *) echo "candor: unknown option '$a' (usage: ./install.sh [--oracle])" >&2; exit 2 ;;
  esac
done

CLONE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
CACHE="${CANDOR_CACHE:-$HOME/.candor}"
BIN="${CARGO_HOME:-$HOME/.cargo}/bin"

echo "candor: installing from $CLONE" >&2

# 1. Build. The stable tools are the install. The lint (the oracle instrument) is built only on request,
# and a failure to build it (no nightly / can't fetch rustc-dev) does not fail the install.
echo "candor: building candor-scan + candor-query (stable Rust)…" >&2
( cd "$CLONE" && cargo +stable build -p candor-scan -p candor-query ) \
  || { echo "candor: could not build the stable tools — see the errors above." >&2; exit 1; }
if [ "$ORACLE" = 1 ]; then
  # rust-toolchain.toml pins the nightly + components; rustup fetches them here if missing.
  echo "candor: --oracle: building the dylint lint too (rustup may fetch the pinned nightly + rustc-dev the first time)…" >&2
  ( cd "$CLONE" && cargo build --workspace ) \
    || echo "candor: the oracle lint did not build (nightly unavailable?) — the stable tools above are installed and are the user route." >&2
fi

S="$(ls "$CLONE"/target/debug/candor-scan 2>/dev/null | head -1 || true)"
[ -n "$S" ] || { echo "candor: no candor-scan binary produced — see the errors above." >&2; exit 1; }
Q="$(ls "$CLONE"/target/debug/candor-query 2>/dev/null | head -1 || true)"
# Pick the NEWEST dylib via `candor-query locate` (the ONE tested locator — newest by mtime, NOT
# alphabetical, so a pinned-nightly bump can't stash the stale-toolchain engine). Empty on the
# stable-only build (no dylib) — that's the degraded path handled below.
LIB=""
[ "$ORACLE" = 1 ] && [ -n "$Q" ] && LIB="$("$Q" locate lib "$CLONE/target/debug" 2>/dev/null || true)"

# 2. Stable home: dylib (if built) + query + scan binaries + a pointer back to the clone.
mkdir -p "$CACHE/lib" "$CACHE/bin"
[ -n "$LIB" ] && cp -f "$LIB" "$CACHE/lib/"
[ -n "$Q" ] && cp -f "$Q" "$CACHE/bin/"
cp -f "$S" "$CACHE/bin/"   # the stable scanner — usable with NO nightly toolchain (the zero-install floor)
printf '%s\n' "$CLONE" > "$CACHE/clone"

# 3. `cargo candor` everywhere: symlink the wrapper onto PATH. Also a plain `candor` command (the wrapper
#    already drops a leading `candor` word, so `candor <cmd>` and `cargo candor <cmd>` both work) — this is
#    the form the docs use, and the one tab-completion attaches to.
mkdir -p "$BIN"
ln -sf "$CLONE/cargo-candor" "$BIN/cargo-candor"
ln -sf "$CLONE/cargo-candor" "$BIN/candor"

echo "candor: installed ✓" >&2
echo "  engine   candor-scan (stable, syntactic) — the user route; policy/baseline gates: candor-scan --policy / CANDOR_BASELINE" >&2
if [ -n "$LIB" ]; then
  echo "  oracle   $(basename "$LIB")  (rustc lint — the family's oracle instrument; reached only with CANDOR_BACKEND=lint)" >&2
fi
echo "  stable   $CACHE/  (query + scan${LIB:+ + dylib} — survives 'cargo clean')" >&2
echo "  command  $BIN/candor  →  'candor …' (or 'cargo candor …') in any project" >&2
# Tab completion: static subcommands/flags + DYNAMIC effect & function-name completion from the scanned
# report (`candor path <TAB>` tabs through the functions candor found). Printed, never auto-appended — you
# own your shell config.
COMP="$CLONE/completions"
case "${SHELL##*/}" in
  zsh)  echo "  complete  add to ~/.zshrc:  fpath=($COMP \$fpath); autoload -Uz compinit; compinit" >&2 ;;
  bash) echo "  complete  add to ~/.bashrc:  source $COMP/candor.bash" >&2 ;;
  *)    echo "  complete  source $COMP/candor.bash (bash), or add $COMP to fpath (zsh)" >&2 ;;
esac
case ":$PATH:" in
  *":$BIN:"*) echo "  PATH     ok" >&2 ;;
  *) echo "  ⚠ PATH   $BIN is not on your PATH — add it so 'cargo candor' resolves." >&2 ;;
esac
echo >&2
echo "  try:  cd <a rust project> && cargo candor audit" >&2
echo "  Claude Code hook (optional):  $CLONE/integrations/claude-code/install.sh" >&2
