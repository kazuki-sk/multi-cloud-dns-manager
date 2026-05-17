use sqlx::{sqlite::SqliteConnectOptions, SqlitePool};
use std::str::FromStr;

/// Concrete pool type.  SQLite is the default backend; the type alias keeps
/// the rest of the codebase decoupled from the driver module.
pub type DbPool = SqlitePool;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error(transparent)]
    Database(#[from] sqlx::Error),
    #[error(transparent)]
    Migrate(#[from] sqlx::migrate::MigrateError),
}

/// Create a connection pool for the given `DATABASE_URL`.
///
/// `PRAGMA foreign_keys = ON` is set via [`SqliteConnectOptions::pragma`] so
/// every connection in the pool enforces FK constraints from the moment it is
/// opened (SQLite disables this by default).
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
