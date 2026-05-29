// Smoke test: an effect-free program must produce no diagnostics (empty main.stderr).
// The real behavioural coverage lives in the `#[cfg(test)] mod tests` unit tests in
// src/lib.rs, which exercise the classifier's precision rules directly.
fn main() {}
