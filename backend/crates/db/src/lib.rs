use std::str::FromStr;

use sqlx::{sqlite::SqliteConnectOptions, SqlitePool};

pub type DbPool = SqlitePool;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error(transparent)]
    Database(#[from] sqlx::Error),
    #[error(transparent)]
    Migrate(#[from] sqlx::migrate::MigrateError),
}

/// Create a SQLite connection pool for the given DATABASE_URL.
///
/// The database file is created automatically if it does not exist
/// (`create_if_missing(true)`).  `PRAGMA foreign_keys = ON` is set at
/// connection-options level so FK constraints are enforced on every connection.
pub async fn connect(database_url: &str) -> Result<DbPool, Error> {
    let opts = SqliteConnectOptions::from_str(database_url)?
        .create_if_missing(true)
        .pragma("foreign_keys", "ON");
    Ok(SqlitePool::connect_with(opts).await?)
}

/// Run all pending migrations against the pool.
pub async fn migrate(pool: &DbPool) -> Result<(), Error> {
    sqlx::migrate!("./migrations").run(pool).await?;
    Ok(())
}
