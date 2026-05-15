use sqlx::AnyPool;

pub type DbPool = AnyPool;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error(transparent)]
    Database(#[from] sqlx::Error),
    #[error(transparent)]
    Migrate(#[from] sqlx::migrate::MigrateError),
}

/// Create a connection pool for the given DATABASE_URL.
///
/// Supported schemes: `sqlite:` and `postgres:` / `postgresql:`.
/// For SQLite, callers should issue `PRAGMA foreign_keys = ON` per
/// connection to enforce FK constraints at the storage layer
/// (SQLite does not enable this by default).
pub async fn connect(database_url: &str) -> Result<DbPool, Error> {
    sqlx::any::install_default_drivers();
    Ok(AnyPool::connect(database_url).await?)
}

/// Run all pending migrations against the pool.
pub async fn migrate(pool: &DbPool) -> Result<(), Error> {
    sqlx::migrate!("./migrations").run(pool).await?;
    Ok(())
}
