## Summary

`Engine::expand` was enhanced to recognize and execute `{{exec:CMD}}` template directives by detecting the `exec:` prefix, stripping it, and running the command via `sh -c`, returning the trimmed stdout or None if execution fails. The new private `exec_command` helper method performs the actual shell invocation. All downstream functions that call expand now transitively gain shell-command execution capability: `Page::render_token`, `Page::render`, `api::render_one`, `api::render_many`, and `report::build_all` can all expand exec directives inline within template token sequences.
