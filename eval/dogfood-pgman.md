# Dogfooding candor on pgman (a real 66-file Rust TUI)

Ran candor + the new tools (`whatif`, `rewire`) on `pgman` — a real project with a real architecture
invariant (*direct `Db` access must stay in the data layer: `src/conn.rs` + `src/query/`*). Goal: prove the
tools on real code, or honestly bound them. Result: one clear win, one validated-but-narrower-than-pitched.

## 1. The core analysis is correct on real code ✓

`candor-scan` on pgman: 137 effectful functions (Db 56, Clock 46, Env 41, Fs 29, Exec 23, Net 19,
Clipboard 18). **Every direct `Db` source is in `conn::` or `query::`** — exactly the documented boundary,
zero leaks. candor maps the data layer of a real 66-file crate correctly, no hand-holding.

## 2. `whatif` — clear, real value ✓

The natural pre-edit question on pgman: *"can I just run this query directly here?"* Tested on a real UI
function, against the boundary (`deny Db app`):

```
$ whatif App::on_key Db
  adding `Db` to `app::App::on_key`
  → propagates to: app::App::on_key, app::handle::App::on_event
  ⚠ WOULD VIOLATE policy (2):  app::App::on_key, app::handle::App::on_event  (deny Db app)
```

Correct and useful: an agent about to run a query inside the event handler is told, *before writing code*,
that it breaks the data-access boundary and should route through `query/`. This is the durable value
(graph traversal + deterministic gate) holding up on real code.

## 3. `rewire` — real, but narrower than the demo implied ~

Simulated a realistic buggy refactor: `app::App::close_tx` (which commits/rolls back a transaction via the
data layer) stops calling `conn::tx_commit`/`tx_rollback` — a *serious* correctness bug. `rewire` caught it:

```
app::App::close_tx  ⊘  no longer calls: conn::tx_commit, conn::tx_rollback
```

**But the honest caveat:** the existing effect-`diff` *also* caught this one — `close_tx` went `Db → pure`,
because the dropped calls carried the `Db` effect. So for **effectful** de-wirings, `cargo candor diff`
already flags them. `rewire`'s **unique** contribution is the **pure** de-wiring — dropping a call that
carried *no effect yet* (exactly the gate-gaming in `eval/whatif-behavior`, where `api::handle` went
pure→pure and only the call-graph saw the disconnect). That's a real but **rarer** case than "any
refactor that breaks a data path." Pitch `rewire` as the companion that covers what the effect-diff
*can't* — not as the primary regression catcher.

Second limitation: a function **move/rename** (`app::run_cost_explain` → `query::explain::run_cost_explain`,
a real pgman refactor) shows up as a *dropped* edge at the old name plus an *added* one at the new — a
false positive. `rewire` is a heuristic nudge ("verify this"), not a hard gate.

## Net

The **measured-durable value survives real code**: candor's analysis is accurate, and `whatif` gives a
correct, useful pre-edit boundary verdict on a real project. `rewire` is genuinely useful but **niche** —
it earns its keep specifically on pure-de-wiring / gate-gaming, where the effect-diff is blind; elsewhere
the effect-diff already covers it. Honest takeaway: lead with `whatif` + the gate; position `rewire` as the
narrow second check it actually is.
