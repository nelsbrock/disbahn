use disbahn::database::Database;
use disbahn::DisbahnClient;
use serenity::http::Http;
use std::env;
use anyhow::Context;

fn env_var(name: &str) -> anyhow::Result<String> {
    env::var(name).with_context(|| format!("Unable to fetch environment variable {name}"))
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    dotenvy::dotenv()?;

    env_logger::builder()
        .filter_module(module_path!(), log::LevelFilter::Debug)
        .init();

    let database_url = env_var("DATABASE_URL")?;
    let webhook_url = env_var("WEBHOOK_URL")?;
    let feed_url = env_var("FEED_URL")?;

    let database = Database::connect(&database_url)?;
    let http = Http::new("");
    let webhook = http.get_webhook_from_url(&webhook_url).await.unwrap();

    let mut disbahn_client = DisbahnClient::new(database, webhook, http, feed_url);

    disbahn_client.refresh().await
}
