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
# If the full build fails (no nightly / can't fetch rustc-dev — a stable-only or offline box), DON'T
# abort: build just the stable crates (candor-scan + candor-query) with `+stable`, so candor still works
# zero-install via the syntactic backend. The nightly lint adds the soundness contract when available.
echo "candor: building (rustup may fetch the pinned nightly + rustc-dev the first time)…" >&2
if ( cd "$CLONE" && cargo build --workspace ); then
  :
else
  echo "candor: full build failed (nightly lint unavailable?) — building the STABLE backend only…" >&2
  ( cd "$CLONE" && cargo +stable build -p candor-scan -p candor-query ) \
    || { echo "candor: could not build even the stable backend — see the errors above." >&2; exit 1; }
fi

S="$(ls "$CLONE"/target/debug/candor-scan 2>/dev/null | head -1 || true)"
[ -n "$S" ] || { echo "candor: no candor-scan binary produced — see the errors above." >&2; exit 1; }
Q="$(ls "$CLONE"/target/debug/candor-query 2>/dev/null | head -1 || true)"
# Pick the NEWEST dylib via `candor-query locate` (the ONE tested locator — newest by mtime, NOT
# alphabetical, so a pinned-nightly bump can't stash the stale-toolchain engine). Empty on the
# stable-only build (no dylib) — that's the degraded path handled below.
LIB=""
[ -n "$Q" ] && LIB="$("$Q" locate lib "$CLONE/target/debug" 2>/dev/null || true)"

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
if [ -n "$LIB" ]; then
  echo "  engine   $(basename "$LIB")  (nightly lint — soundness contract)" >&2
else
  echo "  engine   stable backend only (candor-scan) — nightly lint not built; queries/receipt work," >&2
  echo "           enforcement (guard/policy) needs the lint. Re-run with the nightly available to add it." >&2
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
