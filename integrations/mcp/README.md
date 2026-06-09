# candor MCP server

Exposes candor's **instant** read-only queries as native [MCP](https://modelcontextprotocol.io) tools,
so an agent (Claude Code, …) reaches for them reflexively — in one cheap tool call — instead of
grepping and reading source. The queries serve from candor's kept-fresh report, so they're fast;
run `cargo candor watch &` in the project to keep the report fresh as you edit.

## Tools

| tool | what it answers | replaces |
|---|---|---|
| `candor_effects(function)` | a function's effect set (transitive + direct) | reading its source |
| `candor_where(effect)` | which functions perform an effect (sources vs inheritors) | grepping the codebase |
| `candor_callers(function)` | the **blast radius** — every transitive caller (who's affected if you change it), incl. pure ones | tracing callers across files by hand |
| `candor_whatif(function, effect)` | **PRE-EDIT VERDICT** — if I add this effect here, what propagates *and* does it break the deny/pure policy? | edit → run the gate → revert |
| `candor_diff()` | how recent edits changed the effect surface (incl. non-local blast radius) | tracing callers by hand |

`candor_whatif` is the one to reach for *before* introducing a side effect: it crosses the blast radius
with the architecture policy and returns the boundary violations deterministically, without writing code.

## Register

It's a self-contained Python script (no SDK). Point Claude Code at it — either:

```sh
claude mcp add --transport stdio candor -- python3 /abs/path/to/candor/integrations/mcp/candor-mcp.py
```

or, project-scoped and committed, add `.mcp.json` at your project root (see `mcp.json.example`):

```json
{
  "mcpServers": {
    "candor": {
      "type": "stdio",
      "command": "python3",
      "args": ["/abs/path/to/candor/integrations/mcp/candor-mcp.py"]
    }
  }
}
```

The server runs `cargo candor … --json` in the project (its working directory) and returns the result.
First call may generate the report (a one-time re-lint); with `cargo candor watch` running, every call is
instant. Keep the surface small and query judiciously — each call is a round-trip.
