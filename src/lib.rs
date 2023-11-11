pub mod database;

use crate::database::models::{NewPost, Post};
use crate::database::schema::posts::dsl::posts;
use crate::database::Database;
use anyhow::{anyhow, Context};
use chrono::{DateTime, NaiveDateTime, TimeZone};
use diesel::{ExpressionMethods, OptionalExtension, QueryDsl, RunQueryDsl};
use lazy_regex::regex;
use log::{debug, error, info};
use reqwest::IntoUrl;
use rss::Item;
use serenity::http::Http;
use serenity::json::Value;
use serenity::model::channel::Embed;
use serenity::model::webhook::Webhook;

pub struct DisbahnClient {
    database: Database,
    webhook: Webhook,
    http: Http,
    rss_url: String,
}

impl DisbahnClient {
    pub fn new(database: Database, webhook: Webhook, http: Http, rss_url: String) -> Self {
        Self {
            database,
            webhook,
            http,
            rss_url,
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

    fn icon_name_to_colour(name: &str) -> u32 {
        match name {
            "HIM1" => 0xf5c211,
            "HIM2" => 0xc1121c,
            _ => 0x154889,
        }
    }

    fn item_to_embed(item: &rss::Item) -> anyhow::Result<Value> {
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

        let embed =
            Embed::fake(|e| {
                e.title(title)
            .url(link)
            .thumbnail(icon_url)
            .colour(Self::icon_name_to_colour(icon))
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

    pub async fn refresh(&mut self) -> anyhow::Result<()> {
        debug!("Refreshing RSS feed ...");
        let channel = Self::get_rss_channel(&self.rss_url)
            .await
            .map_err(|e| anyhow!(e.to_string()))
            .with_context(|| "Failed to get RSS channel")?;

        let items = channel.items();

        for item in items {
            if let Err(err) = self.refresh_item(item).await {
                error!("Error refreshing item: {err}");
            }
        }

        debug!("Done.");
        Ok(())
    }

    async fn refresh_item(&mut self, item: &Item) -> anyhow::Result<()> {
        use crate::database::schema::posts::{self, dsl};

        let guid = item.guid().ok_or(anyhow!("Missing GUID"))?.value();
        let pub_date_str = item.pub_date().ok_or(anyhow!("Missing publication date"))?;
        let pub_datetime = DateTime::parse_from_rfc2822(pub_date_str)
            .with_context(|| format!("Unable to parse publication date string {pub_date_str:?}"))?
            .naive_utc()
            .and_utc();

        let existing_post: Option<Post> = posts
            .filter(dsl::webhook_id.eq(i64::from_le_bytes(self.webhook.id.0.to_le_bytes())))
            .filter(dsl::announcement_id.eq(guid))
            .first(self.database.conn())
            .optional()
            .with_context(|| "Error loading posts from database")?;

        if let Some(existing_post) = existing_post {
            if existing_post.last_updated().and_utc() < pub_datetime {
                info!("Updated item: {guid}");
                let embed = Self::item_to_embed(item)?;
                self.webhook
                    .edit_message(&self.http, existing_post.message_id(), |w| {
                        w.embeds(vec![embed])
                    })
                    .await
                    .with_context(|| "Failed to edit message")?;

                diesel::update(
                    posts.find((guid, i64::from_le_bytes(self.webhook.id.0.to_le_bytes()))),
                )
                .set(dsl::last_updated.eq(pub_datetime.naive_utc()))
                .execute(self.database.conn())
                .with_context(|| "Error updating post in database")?;
            }
        } else {
            info!("New item: {guid}");
            let embed = Self::item_to_embed(item)?;
            let message = self
                .webhook
                .execute(&self.http, true, |w| w.embeds(vec![embed]))
                .await
                .with_context(|| "Failed to send message")?
                .with_context(|| "Discord did not return a message id")?;

            diesel::insert_into(posts::table)
                .values(NewPost::new(
                    guid,
                    self.webhook.id,
                    message.id,
                    pub_datetime.naive_utc(),
                ))
                .execute(self.database.conn())
                .with_context(|| "Error inserting new post into database")?;
        }
        Ok(())
    }
}
