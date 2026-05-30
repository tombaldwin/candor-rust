---
description: Run candor and show a fresh effect-map receipt for this Rust project
allowed-tools: Bash(*candor-run.sh*)
---
The line below is the live output of candor (deterministic — it actually ran).
Show it to the user verbatim; it is a single status line. Do not summarize, re-run,
or editorialize. If it reports a coverage gap or unresolved functions, remind the
user those parts of the map may be incomplete and the source should be read there.

!`"${CLAUDE_PROJECT_DIR}/.claude/candor/candor-run.sh" --force "${CLAUDE_PROJECT_DIR}"`
