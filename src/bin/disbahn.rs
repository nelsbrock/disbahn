use disbahn::database::Database;
use disbahn::DisbahnClient;
use serenity::http::Http;
use std::env;
use std::env::VarError;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    dotenvy::dotenv()?;

    env_logger::builder()
        .filter_module(module_path!(), log::LevelFilter::Debug)
        .init();

    let database_url = match env::var("DATABASE_URL") {
        Ok(url) => url,
        Err(VarError::NotPresent) => "disbahn.db".to_string(),
        Err(err) => return Err(err.into()),
    };
    let webhook_url = env::var("WEBHOOK_URL")?;
    let feed_url = env::var("FEED_URL")?;

    let database = Database::connect(&database_url)?;
    let http = Http::new("");
    let webhook = http.get_webhook_from_url(&webhook_url).await.unwrap();

    let mut disbahn_client = DisbahnClient::new(database, webhook, http, feed_url);

    disbahn_client.refresh().await
}
