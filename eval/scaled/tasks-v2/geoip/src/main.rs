mod api;
mod report;
mod resolver;
mod service;

use service::GeoService;

fn main() {
    let svc = GeoService::new();
    println!("{}", api::lookup_one(&svc, "203.0.113.1"));
    println!("{}", api::lookup_many(&svc, &["203.0.113.1", "198.51.100.7"]));
    print!("{}", report::summary(&svc));
}
