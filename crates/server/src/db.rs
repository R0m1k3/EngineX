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
    backup_before(&pool, data_dir, 6, "enginex.db.pre-0006").await?;
    sqlx::migrate!("./migrations").run(&pool).await?;
    Ok(pool)
}

/// Copie cohérente de la base (WAL compris) avant d'appliquer la migration
/// `version` sur une base existante : une image plus ancienne refuse une base
/// qui contient une migration inconnue, cette copie permet d'y revenir.
async fn backup_before(pool: &SqlitePool, data_dir: &Path, version: i64, name: &str) -> anyhow::Result<()> {
    let has_table: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name = '_sqlx_migrations'")
            .fetch_one(pool)
            .await?;
    if has_table == 0 {
        return Ok(());
    }
    let applied: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM _sqlx_migrations WHERE version = ?")
        .bind(version)
        .fetch_one(pool)
        .await?;
    let dest = data_dir.join(name);
    if applied > 0 || dest.exists() {
        return Ok(());
    }
    sqlx::query("VACUUM INTO ?")
        .bind(dest.display().to_string())
        .execute(pool)
        .await?;
    tracing::info!("copie de la base avant la migration {version} : {}", dest.display());
    Ok(())
}
