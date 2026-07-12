## Summary

The `Engine::expand` method now intercepts template tokens prefixed with `exec:` and delegates to a new `execute_command` function that spawns a shell subprocess via `sh -c`. This introduces an `Exec` effect that propagates transitively through the entire rendering stack: `api::render_one` and `api::render_many` both gain the effect via `page::Page::render`, which calls `expand`; additionally `report::build_all` inherits it through the same chain, meaning any periodic rebuild job calling these APIs will now execute shell commands during template expansion.
