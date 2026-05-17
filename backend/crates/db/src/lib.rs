use sqlx::{pool::PoolOptions, Any, AnyPool};

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
/// For SQLite connections, `PRAGMA foreign_keys = ON` is automatically
/// executed on every new connection so FK constraints are enforced at
/// the storage layer (SQLite disables this by default).
pub async fn connect(database_url: &str) -> Result<DbPool, Error> {
    sqlx::any::install_default_drivers();
    if database_url.starts_with("sqlite:") {
        Ok(PoolOptions::<Any>::new()
            .after_connect(|conn, _| {
                Box::pin(async move {
                    sqlx::query("PRAGMA foreign_keys = ON")
                        .execute(&mut *conn)
                        .await?;
                    Ok(())
                })
            })
            .connect(database_url)
            .await?)
    } else {
        Ok(AnyPool::connect(database_url).await?)
    }
}

/// Run all pending migrations against the pool.
pub async fn migrate(pool: &DbPool) -> Result<(), Error> {
    sqlx::migrate!("./migrations").run(pool).await?;
    Ok(())
}
