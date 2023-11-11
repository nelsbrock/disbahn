CREATE TABLE posts (
    announcement_id TEXT NOT NULL,
    webhook_id UNSIGNED BIG INT NOT NULL,
    message_id UNSIGNED BIG INT NOT NULL,
    last_updated TIMESTAMP NOT NULL,
    PRIMARY KEY (announcement_id, webhook_id)
);