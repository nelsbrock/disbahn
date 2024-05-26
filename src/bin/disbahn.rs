use anyhow::{anyhow, Context};
use disbahn::database::Database;
use disbahn::DisbahnClient;
use log::error;
use serenity::http::Http;
use std::env;
use std::time::Duration;
use tokio::io;

const DAEMON_INTERVAL_SECS: i64 = 300;

fn env_var(name: &str) -> anyhow::Result<String> {
    env::var(name).with_context(|| format!("Unable to fetch environment variable {name}"))
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    if let Err(err) = dotenvy::dotenv() {
        if !err.not_found() {
            return Err(err).context("Unable to load .env file");
        }
    }

    env_logger::builder()
        .filter_module(module_path!(), log::LevelFilter::Debug)
        .init();

    let mut args = env::args().skip(1);
    let daemon = match args.next() {
        Some(s) if s == "daemon" => true,
        Some(s) => {
            return Err(anyhow!(format!(
                "invalid argument `{s}`; the only allowed argument is `daemon`"
            )))
        }
        None => false,
    };

    if args.next().is_some() {
        return Err(anyhow!("too many arguments"));
    }

    let database_url = env_var("DATABASE_URL")?;
    let webhook_url = env_var("WEBHOOK_URL")?;
    let feed_url = env_var("FEED_URL")?;

    let database = Database::connect(&database_url)?;
    let http = Http::new("");
    let webhook = http.get_webhook_from_url(&webhook_url).await.unwrap();

    let mut disbahn_client = DisbahnClient::new(database, webhook, http, feed_url);

    if daemon {
        loop {
            let now = chrono::Utc::now().timestamp();
            let sleep_secs = (now / DAEMON_INTERVAL_SECS + 1) * DAEMON_INTERVAL_SECS - now;
            let sleep_duration = sleep_secs.try_into().expect("sleep_secs is negative");
            let shutdown = tokio::select! {
                result = wait_for_shutdown_signal() => {
                    result.expect("error on waiting for shutdown signal"); true
                },
                _ = tokio::time::sleep(Duration::from_secs(sleep_duration)) => false,
            };
            if shutdown {
                break Ok(());
            }
            if let Err(err) = disbahn_client.refresh().await {
                error!("{}", err)
            }
        }
    } else {
        disbahn_client.refresh().await
    }
}

async fn wait_for_shutdown_signal() -> io::Result<()> {
    let ctrl_c = tokio::signal::ctrl_c();

    #[cfg(unix)]
    {
        use tokio::signal::unix;

        let sigterm = async {
            unix::signal(unix::SignalKind::terminate())?.recv().await;
            Ok(())
        };

        tokio::select! {
            result = ctrl_c => result,
            result = sigterm => result,
        }
    }

    #[cfg(not(unix))]
    ctrl_c.await
}
