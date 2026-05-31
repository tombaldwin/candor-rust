use crate::page::Page;

/// Render the site's static pages. Run by a periodic rebuild job that assumes rendering is cheap.
pub fn build_all(page: &Page) -> String {
    let mut out = String::new();
    for tokens in [["brand", "year"].as_slice(), ["brand", "missing"].as_slice()] {
        out.push_str(&page.render(tokens));
        out.push('\n');
    }
    out
}
