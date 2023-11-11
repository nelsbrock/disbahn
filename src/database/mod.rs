pub mod models;
pub mod schema;

use anyhow::format_err;
use diesel::prelude::*;
use diesel::SqliteConnection;
use diesel_migrations::{embed_migrations, EmbeddedMigrations, MigrationHarness};
use log::debug;

pub const MIGRATIONS: EmbeddedMigrations = embed_migrations!("migrations");

pub struct Database {
    conn: SqliteConnection,
}

impl Database {
    fn new(conn: SqliteConnection) -> Self {
        Self { conn }
    }

    pub fn connect(url: &str) -> anyhow::Result<Self> {
        let mut connection = SqliteConnection::establish(url)
            .map_err(|err| format_err!("Unable to connect to database at {url}: {err}"))?;
        debug!("Established connection to SQLite database at {url}");

        let migration_versions = connection
            .run_pending_migrations(MIGRATIONS)
            .map_err(|err| format_err!("Unable to run database migrations: {err}"))?;
        if !migration_versions.is_empty() {
            debug!("Ran database migrations for versions {migration_versions:?}");
        }

        Ok(Self::new(connection))
    }

    pub fn conn(&mut self) -> &mut SqliteConnection {
        &mut self.conn
    }
}
