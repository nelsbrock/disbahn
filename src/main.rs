use anyhow::{anyhow, Context};
use chrono::{DateTime, NaiveDateTime, TimeZone};
use lazy_regex::regex;
use log::{debug, error, info};
use reqwest::IntoUrl;
use rss::{Channel, Item};
use serenity::http::Http;
use serenity::json::Value;
use serenity::model::channel::Embed;
use serenity::model::prelude::Webhook;
use std::collections::HashSet;
use std::env;
use std::fs::{File, OpenOptions};
use std::io::{BufRead, BufReader, Write};
use std::time::Duration;

const REFRESH_INTERVAL: Duration = Duration::from_secs(10 * 60);

async fn get_rss_channel<T: IntoUrl>(url: T) -> anyhow::Result<Channel> {
    let content = reqwest::get(url).await?.bytes().await?;
    let channel = Channel::read_from(&content[..])?;
    Ok(channel)
}

fn validity_time_to_timestamp(input: &str) -> anyhow::Result<i64> {
    let naive = NaiveDateTime::parse_from_str(input, "%Y-%m-%d %H:%M:%S")?;
    let timestamp = chrono_tz::Europe::Berlin
        .from_local_datetime(&naive)
        .unwrap()
        .timestamp();
    Ok(timestamp)
}

fn html_to_discord_markdown(input: &str) -> String {
    let re_times = regex!(r#"^.*<br/><br/>"#i);
    let re_newline = regex!(r#"<br\s*/?>"#i);
    let re_bold = regex!(r#"<b>(.*?)</b>"#i);
    let re_italic = regex!(r#"<i>(.*?)</i>"#i);
    let re_strikethrough = regex!(r#"<s>(.*?)</s>"#i);

    let input = re_times.replace(input, "");
    let input = re_newline.replace_all(&input, "\n");
    let input = re_bold.replace_all(&input, "**$1**");
    let input = re_italic.replace_all(&input, "*$1*");
    let input = re_strikethrough.replace_all(&input, "~~$1~~");
    input.to_string()
}

fn icon_name_to_url(name: &str) -> &str {
    match name {
        "HIM1" => "https://upload.wikimedia.org/wikipedia/commons/thumb/a/a9/Zeichen_123_-_Arbeitsstelle%2C_StVO_2013.svg/273px-Zeichen_123_-_Arbeitsstelle%2C_StVO_2013.svg.png",
        "HIM2" => "https://upload.wikimedia.org/wikipedia/commons/thumb/0/02/Zeichen_101_-_Gefahrstelle%2C_StVO_1970.svg/273px-Zeichen_101_-_Gefahrstelle%2C_StVO_1970.svg.png",
        _ => "https://upload.wikimedia.org/wikipedia/commons/thumb/8/8a/RWB-RWBA_Information.svg/240px-RWB-RWBA_Information.svg.png",
    }
}

fn item_to_embed(item: &Item) -> anyhow::Result<Value> {
    const COLOUR: u32 = 0x008d4f;
    const FOOTER_ICON_URL: &str = "https://www.zuginfo.nrw/img/customer/apple-touch-icon.png";

    let categories = item.categories();

    let title = item.title().ok_or(anyhow!("Missing title"))?;
    let link = item.link().ok_or(anyhow!("Missing link"))?;

    let validity_begin = &categories
        .iter()
        .find(|c| c.domain() == Some("validityBegin"))
        .ok_or(anyhow!("Missing validityBegin category"))?
        .name;
    let validity_begin = validity_time_to_timestamp(validity_begin)?;

    let validity_end = &categories
        .iter()
        .find(|c| c.domain() == Some("validityEnd"))
        .ok_or(anyhow!("Missing validityEnd category"))?
        .name;
    let validity_end = validity_time_to_timestamp(validity_end)?;

    let icon = categories
        .iter()
        .find(|c| c.domain() == Some("icon"))
        .map(|c| c.name())
        .unwrap_or("");
    let icon_url = icon_name_to_url(icon);

    let description =
        html_to_discord_markdown(item.description().ok_or(anyhow!("Missing description"))?);

    let pub_date = item.pub_date().ok_or(anyhow!("Missing publication date"))?;
    let pub_timestamp = DateTime::parse_from_rfc2822(pub_date)
        .with_context(|| format!("Unable to parse publication date string {pub_date:?}"))?
        .naive_utc()
        .and_utc()
        .to_rfc3339_opts(chrono::SecondsFormat::Secs, true);

    let embed = Embed::fake(|e| {
        e.title(title)
            .url(link)
            .thumbnail(icon_url)
            .colour(COLOUR)
            .description(description)
            .field("Beginn:", format!("<t:{}:F>", validity_begin), true)
            .field("Ende:", format!("<t:{}:F>", validity_end), true)
            .field("Hinweis:", include_str!("hint.txt"), false)
            .timestamp(pub_timestamp)
            .footer(|f| {
                f.text("Quelle: https://zuginfo.nrw/ \u{2013} Alle Angaben ohne Gewehr \u{1F52B}")
                    .icon_url(FOOTER_ICON_URL)
            })
    });

    Ok(embed)
}

async fn refresh_and_process_rss(
    feed_url: &str,
    http: &Http,
    webhook: &Webhook,
    known_guids: &mut HashSet<String>,
    known_guids_file: &mut File,
) -> anyhow::Result<()> {
    debug!("Refreshing RSS feed ...");
    let channel = get_rss_channel(feed_url)
        .await
        .map_err(|e| anyhow!(e.to_string()))
        .with_context(|| "Failed to get RSS channel")?;
    for item in channel.items() {
        let guid = item.guid().ok_or(anyhow!("Missing GUID"))?.value();
        if !known_guids.contains(guid) {
            info!("New item: {}", guid);
            known_guids.insert(guid.to_owned());
            writeln!(known_guids_file, "{}", guid).with_context(|| "Failed to write known GUID")?;
            let embed = item_to_embed(item)?;
            webhook
                .execute(&http, false, |w| w.embeds(vec![embed]))
                .await
                .with_context(|| "Failed to execute webhook")?;
        }
    }
    debug!("Done.");
    Ok(())
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    dotenvy::dotenv()?;

    env_logger::builder()
        .filter_module(module_path!(), log::LevelFilter::Debug)
        .init();

    let webhook_url = env::var("WEBHOOK_URL")?;
    let feed_url = env::var("FEED_URL")?;
    let known_guids_file = env::var("KNOWN_GUIDS_FILE")?;

    let mut known_guids_file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .open(known_guids_file)?;

    let http = Http::new("");
    let webhook = http.get_webhook_from_url(&webhook_url).await.unwrap();

    let mut known_guids: HashSet<String> = HashSet::new();
    for line in BufReader::new(&known_guids_file).lines() {
        let line = line?;
        known_guids.insert(line);
    }

    loop {
        if let Err(err) = refresh_and_process_rss(
            &feed_url,
            &http,
            &webhook,
            &mut known_guids,
            &mut known_guids_file,
        )
        .await
        {
            error!("Error: {}", err);
        }

        tokio::time::sleep(REFRESH_INTERVAL).await;
    }
}
