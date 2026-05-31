use crate::page::Page;

/// Handle `POST /render` for one template line (a sequence of tokens).
pub fn render_one(page: &Page, tokens: &[&str]) -> String {
    page.render(tokens)
}

/// Handle `POST /render-batch` for several template lines.
pub fn render_many(page: &Page, lines: &[&[&str]]) -> Vec<String> {
    lines.iter().map(|l| page.render(l)).collect()
}
