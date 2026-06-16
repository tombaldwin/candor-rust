#!/usr/bin/env python3
# candor-mcp.py — a minimal MCP (Model Context Protocol) stdio server that exposes candor's INSTANT
# read-only queries as native agent tools. No SDK: newline-delimited JSON-RPC 2.0 over stdio.
#
# Why: candor's queries are fast (they read the kept-fresh report — run `cargo candor watch &`), but
# an agent only saves time if it reaches for them reflexively instead of grepping and reading source.
# As MCP tools the agent calls them in one cheap call, like it already calls grep.
#
# Register (project-scoped, committed) by adding to your project's .mcp.json:
#   { "mcpServers": { "candor": { "type": "stdio", "command": "python3",
#                                  "args": ["/abs/path/to/candor/integrations/mcp/candor-mcp.py"] } } }
# or:  claude mcp add --transport stdio candor -- python3 /abs/path/.../candor-mcp.py
#
# The server runs `cargo candor <q> --json` in the project (its cwd); each query reads the report and
# returns instantly when it's fresh. Output goes to the agent; all logging stays on stderr.
import json
import os
import subprocess
import sys

HERE = os.path.dirname(os.path.abspath(__file__))
CARGO_CANDOR = os.path.normpath(os.path.join(HERE, "..", "..", "cargo-candor"))  # the candor clone root

TOOLS = [
    {
        "name": "candor_effects",
        "description": "The effect set of a Rust function (INSTANT, from candor's report). Use before "
                       "editing to see what it does to the outside world without reading its source. "
                       "Returns its transitive `inferred` effects and the `direct` ones it performs itself.",
        "inputSchema": {
            "type": "object",
            "properties": {"function": {"type": "string", "description": "function name — resolved exact > segment-suffix (`Type::method`) > substring; pass a more specific name to narrow, a bare leaf to browse"}},
            "required": ["function"],
        },
    },
    {
        "name": "candor_where",
        "description": "Which functions perform a given effect (INSTANT), split into the direct sources "
                       "and the functions that inherit it transitively. Faster than grepping the codebase. "
                       "Effects: Net Fs Db Exec Env Clock Ipc Log Rand Clipboard Unknown.",
        "inputSchema": {
            "type": "object",
            "properties": {"effect": {"type": "string", "description": "an effect name, e.g. Net"}},
            "required": ["effect"],
        },
    },
    {
        "name": "candor_callers",
        "description": "The blast radius of a function (INSTANT) — every function that TRANSITIVELY calls "
                       "it, i.e. who is affected if you change it. Works for ANY function, including a "
                       "PURE one you're about to make effectful. Use before changing behaviour/signature. "
                       "Enumerating 3-5 layers of callers by hand is exactly what's easy to under-count.",
        "inputSchema": {
            "type": "object",
            "properties": {"function": {"type": "string", "description": "function name — resolved exact > segment-suffix (`Type::method`) > substring; pass a more specific name to narrow, a bare leaf to browse"}},
            "required": ["function"],
        },
    },
    {
        "name": "candor_whatif",
        "description": "PRE-EDIT VERDICT (INSTANT): before you add a side effect to a function, ask what it "
                       "would do. Given a function and an effect you're about to introduce (e.g. a network "
                       "call), returns the blast radius (every transitive caller that would gain the effect) "
                       "AND — against the project's policy — which functions would VIOLATE a deny/pure "
                       "architecture boundary. Answers 'if I make this network call here, does it break the "
                       "architecture?' deterministically, WITHOUT writing code. Call this before introducing "
                       "Net/Fs/Db/Exec/Env to a function instead of editing, running the gate, and reverting.",
        "inputSchema": {
            "type": "object",
            "properties": {
                "function": {"type": "string", "description": "the function you're about to add the effect to (name; exact/segment-suffix preferred over substring)"},
                "effect": {"type": "string", "description": "the effect you'd introduce: Net Fs Db Exec Env Clock Ipc Log Rand Clipboard"},
            },
            "required": ["function", "effect"],
        },
    },
    {
        "name": "candor_diff",
        "description": "How your recent edits changed each function's effect surface vs the committed "
                       "baseline (INSTANT) — what gained or lost an effect, INCLUDING the non-local blast "
                       "radius (a network call you add deep in a helper shows +Net on every caller). Use "
                       "after editing to check you didn't change the effect surface unintentionally.",
        "inputSchema": {"type": "object", "properties": {}, "required": []},
    },
]


def run_query(args):
    try:
        r = subprocess.run([CARGO_CANDOR, *args], capture_output=True, text=True, timeout=300)
        return r.stdout.strip() or r.stderr.strip() or "(no output)"
    except Exception as e:  # noqa: BLE001 — surface any failure to the agent as text
        return f"candor: query failed ({e})"


def arg(args, key):
    """Required-arg getter. A missing/empty value is a clear error, not a silent whole-report query
    (an unset `function` would otherwise run `show "" --json`). A leading-dash value is rejected — it
    would be parsed as a FLAG by candor-query (argument injection from a tool argument)."""
    v = args.get(key, "")
    if not isinstance(v, str) or v == "":
        raise ValueError(f"missing required argument: {key}")
    if v.startswith("-"):
        raise ValueError(f"argument {key!r} may not start with '-'")
    return v


def dispatch(name, args):
    try:
        if name == "candor_effects":
            return run_query(["show", arg(args, "function"), "--json"])
        if name == "candor_where":
            return run_query(["where", arg(args, "effect"), "--json"])
        if name == "candor_callers":
            return run_query(["callers", arg(args, "function"), "--json"])
        if name == "candor_whatif":
            return run_query(["whatif", arg(args, "function"), arg(args, "effect"), "--json"])
        if name == "candor_diff":
            return run_query(["diff", "--json"])
    except ValueError as e:
        return f"candor: {e}"
    return None


def send(mid, result=None, error=None):
    msg = {"jsonrpc": "2.0", "id": mid}
    if error is not None:
        msg["error"] = error
    else:
        msg["result"] = result
    sys.stdout.write(json.dumps(msg) + "\n")
    sys.stdout.flush()


def main():
    for line in sys.stdin:
        line = line.strip()
        if not line:
            continue
        try:
            req = json.loads(line)
        except Exception:
            continue  # not parseable; nothing we can reply to without an id
        mid = req.get("id")
        method = req.get("method")
        if mid is None:
            continue  # a notification (e.g. notifications/initialized) — no response
        if method == "initialize":
            send(mid, result={
                "protocolVersion": req.get("params", {}).get("protocolVersion", "2025-06-18"),
                "capabilities": {"tools": {}},
                "serverInfo": {"name": "candor", "version": "1.0.0"},
            })
        elif method == "tools/list":
            send(mid, result={"tools": TOOLS})
        elif method == "tools/call":
            params = req.get("params", {})
            text = dispatch(params.get("name"), params.get("arguments", {}))
            if text is None:
                send(mid, result={"content": [{"type": "text", "text": f"unknown tool: {params.get('name')}"}], "isError": True})
            else:
                send(mid, result={"content": [{"type": "text", "text": text}]})
        else:
            send(mid, error={"code": -32601, "message": "Method not found"})


if __name__ == "__main__":
    main()
