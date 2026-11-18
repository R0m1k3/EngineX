//! Base SQLite (WAL) et migrations embarquées.

use std::{path::Path, str::FromStr, time::Duration};

use sqlx::sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePool, SqlitePoolOptions};

pub async fn open(data_dir: &Path) -> anyhow::Result<SqlitePool> {
    let url = format!("sqlite://{}", data_dir.join("enginex.db").display());
    let opts = SqliteConnectOptions::from_str(&url)?
        .create_if_missing(true)
        .journal_mode(SqliteJournalMode::Wal)
        .busy_timeout(Duration::from_secs(5))
        .foreign_keys(true);
    let pool = SqlitePoolOptions::new().max_connections(8).connect_with(opts).await?;
    sqlx::migrate!("./migrations").run(&pool).await?;
    Ok(pool)
}
