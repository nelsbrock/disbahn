// @generated automatically by Diesel CLI.

diesel::table! {
    posts (announcement_id, webhook_id) {
        announcement_id -> Text,
        webhook_id -> BigInt,
        message_id -> BigInt,
        last_updated -> Timestamp,
    }
}
