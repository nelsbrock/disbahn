use super::schema::*;
use chrono::NaiveDateTime;
use diesel::{Insertable, Queryable};
use getset::Getters;
use serenity::model::id::{MessageId, WebhookId};
use std::borrow::Cow;

#[derive(Queryable, Getters)]
pub struct Post {
    #[getset(get = "pub")]
    announcement_id: String,
    webhook_id: i64,
    message_id: i64,
    #[getset(get = "pub")]
    last_updated: NaiveDateTime,
}

impl Post {
    pub fn webhook_id(&self) -> WebhookId {
        WebhookId(u64::from_le_bytes(self.webhook_id.to_le_bytes()))
    }

    pub fn message_id(&self) -> MessageId {
        MessageId(u64::from_le_bytes(self.message_id.to_le_bytes()))
    }
}

#[derive(Insertable)]
#[diesel(table_name = posts)]
pub struct NewPost<'a> {
    announcement_id: Cow<'a, str>,
    webhook_id: i64,
    message_id: i64,
    last_updated: NaiveDateTime,
}

impl<'a> NewPost<'a> {
    pub fn new(
        announcement_id: impl Into<Cow<'a, str>>,
        webhook_id: WebhookId,
        message_id: MessageId,
        last_updated: NaiveDateTime,
    ) -> Self {
        Self {
            announcement_id: announcement_id.into(),
            webhook_id: i64::from_le_bytes(webhook_id.0.to_le_bytes()),
            message_id: i64::from_le_bytes(message_id.0.to_le_bytes()),
            last_updated,
        }
    }
}
