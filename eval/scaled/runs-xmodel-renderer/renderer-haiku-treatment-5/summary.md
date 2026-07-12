## Summary

The `Engine::expand` method now intercepts tokens starting with `"exec:"`, extracting the command suffix and delegating to a new `execute_command` function that runs the shell command via `sh -c`. This introduces shell execution capability into the templating pipeline. The `Exec` effect transitively propagates through all downstream rendering functions: `Page::render_token` and `Page::render` (which call `Engine::expand`), both API entry points `api::render_one` and `api::render_many` (which call `Page::render`), and ultimately the `main` function (which calls both API functions).
