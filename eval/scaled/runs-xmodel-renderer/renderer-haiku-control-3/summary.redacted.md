## Summary

`Engine::expand` now detects tokens with an `exec:` prefix and executes them as shell commands via `Command::new("sh")`, returning trimmed stdout on success or `None` on failure. This new capability transitively affects `Page::render_token` and `Page::render`, which call `expand` to resolve all template tokens. Downstream, `api::render_one`, `api::render_many`, `report::build_all`, and the `main` entry point all transitively gain the ability to execute arbitrary shell commands through template token expansion.
