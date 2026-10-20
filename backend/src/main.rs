mod api;
mod ffmpeg;
mod ffprobe;
mod jobs;
mod render;
mod storage;

use std::{env, net::SocketAddr, path::PathBuf, sync::Arc};

use anyhow::Context;
use sqlx::{SqlitePool, sqlite::SqliteConnectOptions};
use tokio::sync::mpsc;
use tower_http::cors::{Any, CorsLayer};
use tracing::info;

#[derive(Clone)]
pub struct AppState {
    pub db: SqlitePool,
    pub storage_root: Arc<PathBuf>,
    pub ffmpeg: String,
    pub ffprobe: String,
    pub max_upload_bytes: u64,
    pub job_tx: mpsc::Sender<String>,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            env::var("RUST_LOG").unwrap_or_else(|_| "video_editor=info,tower_http=info".into()),
        )
        .init();

    let database_url =
        env::var("DATABASE_URL").unwrap_or_else(|_| "sqlite://video_editor.db".into());
    let options = database_url
        .parse::<SqliteConnectOptions>()
        .context("DATABASE_URL inválida")?
        .create_if_missing(true);
    let db = SqlitePool::connect_with(options)
        .await
        .context("conectar ao SQLite")?;
    sqlx::migrate!()
        .run(&db)
        .await
        .context("executar migrations")?;

    let storage_root = PathBuf::from(env::var("STORAGE_ROOT").unwrap_or_else(|_| "storage".into()));
    tokio::fs::create_dir_all(&storage_root)
        .await
        .context("criar storage")?;

    let (job_tx, job_rx) = mpsc::channel(32);
    let state = AppState {
        db,
        storage_root: Arc::new(storage_root),
        ffmpeg: env::var("FFMPEG_BIN").unwrap_or_else(|_| "ffmpeg".into()),
        ffprobe: env::var("FFPROBE_BIN").unwrap_or_else(|_| "ffprobe".into()),
        max_upload_bytes: env::var("MAX_UPLOAD_BYTES")
            .ok()
            .and_then(|value| value.parse().ok())
            .unwrap_or(2 * 1024 * 1024 * 1024),
        job_tx,
    };

    jobs::resume_queued(&state.db, &state.job_tx).await?;
    tokio::spawn(jobs::run_worker(state.clone(), job_rx));

    let app = api::router(state).layer(
        CorsLayer::new()
            .allow_origin(Any)
            .allow_methods(Any)
            .allow_headers(Any),
    );
    let address: SocketAddr = env::var("BIND_ADDR")
        .unwrap_or_else(|_| "127.0.0.1:3000".into())
        .parse()
        .context("BIND_ADDR inválido")?;
    let listener = tokio::net::TcpListener::bind(address).await?;
    info!(%address, "video editor API pronta");
    axum::serve(listener, app).await.context("servidor HTTP")?;
    Ok(())
}
