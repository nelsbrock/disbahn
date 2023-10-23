use anyhow::{anyhow, Context};
use chrono::{DateTime, NaiveDateTime, TimeZone, Utc};
use lazy_regex::regex;
use log::{debug, info};
use reqwest::IntoUrl;
use serenity::http::Http;
use serenity::json::Value;
use serenity::model::channel::Embed;
use serenity::model::webhook::Webhook;
use std::borrow::Cow;
use std::env;

struct RefreshDb(sled::Db);

impl RefreshDb {
    fn get(&self, guid: &str) -> anyhow::Result<Option<DateTime<Utc>>> {
        self.0
            .get(guid)
            .with_context(|| "db error: unable to get value")?
            .map(|b| {
                bincode::deserialize::<i64>(&b)
                    .with_context(|| "db error: unable to deserialize i64")
            })
            .transpose()?
            .map(|i| {
                Utc.timestamp_opt(i, 0)
                    .single()
                    .with_context(|| "db error: unable to parse timestamp from i64")
            })
            .transpose()
    }

    fn insert(&self, guid: String, datetime: DateTime<Utc>) -> anyhow::Result<()> {
        let timestamp = datetime.timestamp();
        let bytes = bincode::serialize(&timestamp).with_context(|| "db error")?;
        self.0.insert(guid, bytes).with_context(|| "db error")?;
        Ok(())
    }
}

struct DisbahnClient {
    webhook: Webhook,
    http: Http,
    rss_url: String,
    known_guids: RefreshDb,
}

impl DisbahnClient {
    fn new(webhook: Webhook, http: Http, rss_url: String, known_guids: RefreshDb) -> Self {
        Self {
            webhook,
            http,
            rss_url,
            known_guids,
        }
    }

    async fn get_rss_channel<T: IntoUrl>(url: T) -> anyhow::Result<rss::Channel> {
        let content = reqwest::get(url).await?.bytes().await?;
        let channel = rss::Channel::read_from(&content[..])?;
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
        let re_times = regex!(r#"^.*<br\s*/>\s*<br\s*/>"#i);
        let re_bold = regex!(r#"<b\s*>((.|\n)*?)</b\s*>"#i);
        let re_italic = regex!(r#"<i\s*>((.|\n)*?)</i\s*>"#i);
        let re_strikethrough = regex!(r#"<s\s*>((.|\n)*?)</s\s*>"#i);
        let re_newline = regex!(r#"<br\s*/?>"#i);

        let input = re_times.replace(input, "");
        let input = re_bold.replace_all(&input, "**$1**");
        let input = re_italic.replace_all(&input, "*$1*");
        let input = re_strikethrough.replace_all(&input, "~~$1~~");
        let input = re_newline.replace_all(&input, "\n");
        input.to_string()
    }

    fn icon_name_to_url(name: &str) -> &str {
        match name {
            "HIM1" => "https://upload.wikimedia.org/wikipedia/commons/thumb/a/a9/Zeichen_123_-_Arbeitsstelle%2C_StVO_2013.svg/273px-Zeichen_123_-_Arbeitsstelle%2C_StVO_2013.svg.png",
            "HIM2" => "https://upload.wikimedia.org/wikipedia/commons/thumb/0/02/Zeichen_101_-_Gefahrstelle%2C_StVO_1970.svg/273px-Zeichen_101_-_Gefahrstelle%2C_StVO_1970.svg.png",
            _ => "https://upload.wikimedia.org/wikipedia/commons/thumb/5/56/Zeichen_365-61_-_Informationsstelle%2C_StVO_2013.svg/240px-Zeichen_365-61_-_Informationsstelle%2C_StVO_2013.svg.png",
        }
    }

    fn item_to_embed(item: &rss::Item, is_update: bool) -> anyhow::Result<Value> {
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
        let validity_begin = Self::validity_time_to_timestamp(validity_begin)?;

        let validity_end = &categories
            .iter()
            .find(|c| c.domain() == Some("validityEnd"))
            .ok_or(anyhow!("Missing validityEnd category"))?
            .name;
        let validity_end = Self::validity_time_to_timestamp(validity_end)?;

        let icon = categories
            .iter()
            .find(|c| c.domain() == Some("icon"))
            .map(|c| c.name())
            .unwrap_or("");
        let icon_url = Self::icon_name_to_url(icon);

        let description = Self::html_to_discord_markdown(
            item.description().ok_or(anyhow!("Missing description"))?,
        );

        let pub_date_str = item.pub_date().ok_or(anyhow!("Missing publication date"))?;
        let pub_datetime = DateTime::parse_from_rfc2822(pub_date_str)
            .with_context(|| format!("Unable to parse publication date string {pub_date_str:?}"))?
            .naive_utc()
            .and_utc();

        let pub_timestamp = pub_datetime.to_rfc3339_opts(chrono::SecondsFormat::Secs, true);

        let embed = Embed::fake(|e| {
            e.title(if is_update {
                Cow::Owned(format!("UPDATE: {title}"))
            } else {
                Cow::Borrowed(title)
            })
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

    async fn refresh(&mut self) -> anyhow::Result<()> {
        debug!("Refreshing RSS feed ...");
        let channel = Self::get_rss_channel(&self.rss_url)
            .await
            .map_err(|e| anyhow!(e.to_string()))
            .with_context(|| "Failed to get RSS channel")?;

        let items = channel.items();

        for item in items {
            let guid = item.guid().ok_or(anyhow!("Missing GUID"))?.value();
            let pub_date_str = item.pub_date().ok_or(anyhow!("Missing publication date"))?;
            let pub_datetime = DateTime::parse_from_rfc2822(pub_date_str)
                .with_context(|| {
                    format!("Unable to parse publication date string {pub_date_str:?}")
                })?
                .naive_utc()
                .and_utc();

            let is_update;
            if let Some(last_pub_datetime) = self.known_guids.get(guid)? {
                if last_pub_datetime < pub_datetime {
                    is_update = true;
                    info!("Updated item: {guid}");
                } else {
                    continue;
                }
            } else {
                is_update = false;
                info!("New item: {guid}");
            }

            let embed = Self::item_to_embed(item, is_update)?;
            self.webhook
                .execute(&self.http, false, |w| w.embeds(vec![embed]))
                .await
                .with_context(|| "Failed to execute webhook")?;
            self.known_guids
                .insert(guid.to_string(), pub_datetime)
                .with_context(|| "Failed to write to database")?;
        }

        debug!("Done.");
        Ok(())
    }
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    dotenvy::dotenv()?;

    env_logger::builder()
        .filter_module(module_path!(), log::LevelFilter::Debug)
        .init();

    let webhook_url = env::var("WEBHOOK_URL")?;
    let feed_url = env::var("FEED_URL")?;
    let known_guids_db = env::var("KNOWN_GUIDS_DB")?;

    let known_guids_db =
        RefreshDb(sled::open(known_guids_db).with_context(|| "unable to open database")?);

    let http = Http::new("");
    let webhook = http.get_webhook_from_url(&webhook_url).await.unwrap();

    let mut disbahn_client = DisbahnClient::new(webhook, http, feed_url, known_guids_db);

    disbahn_client.refresh().await
}
