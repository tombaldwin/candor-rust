use crate::engine::Engine;

/// The page layer: renders templates by expanding their tokens through the engine.
pub struct Page {
    engine: Engine,
}

impl Page {
    pub fn new() -> Self {
        Page { engine: Engine::new() }
    }

    /// Render one token to its expansion (or the literal `{{token}}` if unknown).
    pub fn render_token(&self, token: &str) -> String {
        self.engine.expand(token).unwrap_or_else(|| format!("{{{{{token}}}}}"))
    }

    /// Render a sequence of tokens, space-separated.
    pub fn render(&self, tokens: &[&str]) -> String {
        tokens.iter().map(|t| self.render_token(t)).collect::<Vec<_>>().join(" ")
    }
}
