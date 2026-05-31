mod api;
mod engine;
mod page;
mod report;

use page::Page;

fn main() {
    let page = Page::new();
    println!("{}", api::render_one(&page, &["brand", "year"]));
    for line in api::render_many(&page, &[&["brand"], &["year"]]) {
        println!("{line}");
    }
    print!("{}", report::build_all(&page));
}
