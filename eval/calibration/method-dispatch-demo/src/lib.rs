use reqwest::Client;
use sqlx::PgPool;

pub struct App {
    http: reqwest::Client,
    db: PgPool,
}

impl App {
    // method dispatch on struct fields — invisible without receiver-type inference
    pub async fn fetch_user(&self, id: i64) -> String {
        let row = self.db.fetch_one("SELECT * FROM users").await;  // sqlx -> Db
        let _ = self.http.get("https://api.example.com").send().await; // reqwest chain -> Net
        format!("{id}")
    }

    // pure: builds a string, calls only local pure helpers
    pub fn format_label(&self, name: &str) -> String {
        normalize(name)
    }
}

// param-typed receiver
pub async fn ping(client: &Client) -> bool {
    client.execute(make_req()).await.is_ok()  // reqwest -> Net
}

// constructor-typed local
pub async fn one_shot() {
    let c = reqwest::Client::new();
    let _ = c.get("https://example.com").send().await; // reqwest chain -> Net
}

fn normalize(s: &str) -> String { s.trim().to_string() }
fn make_req() -> reqwest::Request { todo!() }
