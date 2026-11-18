//! `enginex serve` (par défaut) ou `enginex reset-password`.

use std::{net::SocketAddr, time::Duration};

use enginex_server::{AppState, Config, app, auth, db};
use tracing_subscriber::EnvFilter;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")))
        // Pas de codes couleur dans `docker logs`.
        .with_ansi(std::io::IsTerminal::is_terminal(&std::io::stdout()))
        .init();
    let cmd = std::env::args().nth(1).unwrap_or_else(|| "serve".into());
    match cmd.as_str() {
        "serve" => serve().await,
        "reset-password" => reset_password().await,
        other => anyhow::bail!("commande inconnue : {other} (serve | reset-password)"),
    }
}

async fn serve() -> anyhow::Result<()> {
    let config = Config::from_env()?;
    config.check_data_dir()?;
    let pool = db::open(&config.data_dir).await?;
    let bind = config.bind;
    let state = AppState::new(config, pool);
    auth::ensure_setup_code(&state)
        .await
        .map_err(|e| anyhow::anyhow!("lecture de la base : {e}"))?;
    let listener = tokio::net::TcpListener::bind(bind).await?;
    tracing::info!("EngineX {} écoute sur {bind}", env!("CARGO_PKG_VERSION"));
    axum::serve(listener, app(state).into_make_service_with_connect_info::<SocketAddr>())
        .with_graceful_shutdown(shutdown_signal())
        .await?;
    Ok(())
}

/// SIGTERM ou SIGINT ; les requêtes en cours ont ensuite 10 s pour finir.
async fn shutdown_signal() {
    let ctrl_c = async {
        let _ = tokio::signal::ctrl_c().await;
    };
    let term = async {
        match tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()) {
            Ok(mut s) => {
                s.recv().await;
            }
            Err(_) => std::future::pending::<()>().await,
        }
    };
    tokio::select! { _ = ctrl_c => {}, _ = term => {} }
    tracing::info!("arrêt demandé");
    tokio::spawn(async {
        tokio::time::sleep(Duration::from_secs(10)).await;
        std::process::exit(0);
    });
}

async fn reset_password() -> anyhow::Result<()> {
    let config = Config::from_env()?;
    let pool = db::open(&config.data_dir).await?;
    sqlx::query("DELETE FROM settings WHERE key = 'password_hash'")
        .execute(&pool)
        .await?;
    sqlx::query("DELETE FROM sessions").execute(&pool).await?;
    println!(
        "Mot de passe effacé. Ouvrez EngineX : un nouveau code de configuration est écrit dans les logs (docker logs enginex)."
    );
    Ok(())
}
