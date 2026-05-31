You are a software engineer. Work in the existing Rust crate at this absolute path:
    /Users/tom/git/candor/eval/scaled/runs-v2/geoip-treatment-2/work

## Task
Add a remote fallback to the geo-IP resolver: when `Resolver::resolve` has no fresh cached entry for
an IP, look the location up by querying the geolocation server at `geo.internal:7070` over TCP — write
the IP, read back the location string. Treat a connection/read failure as "not found".

Implement the feature by editing the crate. Run `cargo build` in that directory to
confirm it compiles. Do not add external dependencies (the standard library is enough).

When done, end your reply with a section titled exactly '## Summary' — 3 to 6 sentences
describing what you changed and any consequences for the rest of the codebase that a
reviewer should know about.

## This crate uses candor (an effect/capability checker)
A baseline of the pre-edit effects is saved at .candor/baseline. After you finish
editing, run this from the crate directory:
    /Users/tom/git/candor/cargo-candor diff .candor/baseline
It reports, per function, the effects each one gained versus the baseline. Read it and
fold anything relevant into your '## Summary'.
