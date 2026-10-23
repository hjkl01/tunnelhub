mod auth;
mod proxy;
mod services;
mod state;

use std::{env, net::SocketAddr, path::PathBuf, time::Duration};

use axum::{
    extract::{Request, State},
    middleware::{self, Next},
    response::Response,
    routing::get,
    Router,
};
use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};
use tower_http::trace::TraceLayer;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

use crate::state::AppState;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::registry()
        .with(tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "tunnelhub_server=info,tower_http=info".into()))
        .with(tracing_subscriber::fmt::layer())
        .init();

    let database_url = env::var("TUNNELHUB_DATABASE_URL")
        .unwrap_or_else(|_| default_database_url());
    let options = SqliteConnectOptions::new()
        .filename(database_url.trim_start_matches("sqlite://"))
        .create_if_missing(true)
        .foreign_keys(true);
    let pool = SqlitePoolOptions::new()
        .max_connections(5)
        .acquire_timeout(Duration::from_secs(5))
        .connect_with(options)
        .await?;

    sqlx::query("CREATE TABLE IF NOT EXISTS admin_users (id INTEGER PRIMARY KEY CHECK (id = 1), username TEXT NOT NULL UNIQUE, password_hash TEXT NOT NULL, totp_secret TEXT NOT NULL, totp_enabled INTEGER NOT NULL DEFAULT 0, created_at TEXT NOT NULL)")
        .execute(&pool)
        .await?;
    sqlx::query("CREATE TABLE IF NOT EXISTS sessions (token_hash TEXT PRIMARY KEY NOT NULL, user_id INTEGER NOT NULL REFERENCES admin_users(id) ON DELETE CASCADE, created_at TEXT NOT NULL, expires_at TEXT NOT NULL)")
        .execute(&pool)
        .await?;
    sqlx::query("CREATE INDEX IF NOT EXISTS idx_sessions_expires_at ON sessions(expires_at)")
        .execute(&pool)
        .await?;
    sqlx::query("CREATE TABLE IF NOT EXISTS recovery_codes (id INTEGER PRIMARY KEY AUTOINCREMENT, code_hash TEXT NOT NULL UNIQUE, created_at TEXT NOT NULL, used_at TEXT)")
        .execute(&pool)
        .await?;

    sqlx::query(
        r#"CREATE TABLE IF NOT EXISTS services (
            id TEXT PRIMARY KEY NOT NULL,
            name TEXT NOT NULL,
            hostname TEXT NOT NULL UNIQUE,
            target_url TEXT NOT NULL,
            enabled INTEGER NOT NULL DEFAULT 1,
            created_at TEXT NOT NULL,
            updated_at TEXT NOT NULL
        )"#,
    )
    .execute(&pool)
    .await?;

    let client = reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(5))
        .redirect(reqwest::redirect::Policy::none())
        .build()?;
    let state = AppState { pool, client };
    let app = Router::new()
        .route("/api/health", get(health))
        .nest("/api/auth", auth::router())
        .nest("/api/services", services::router().route_layer(middleware::from_fn_with_state(state.clone(), auth::require_session)))
        .fallback(proxy::handle)
        .with_state(state)
        .layer(TraceLayer::new_for_http());

    let address: SocketAddr = env::var("TUNNELHUB_LISTEN")
        .unwrap_or_else(|_| "0.0.0.0:8080".to_string())
        .parse()?;
    let listener = tokio::net::TcpListener::bind(address).await?;
    tracing::info!(%address, "TunnelHub API listening");
    axum::serve(listener, app).await?;
    Ok(())
}

async fn health() -> axum::Json<serde_json::Value> {
    axum::Json(serde_json::json!({
        "status": "ok",
        "service": "tunnelhub-server",
        "version": env!("CARGO_PKG_VERSION")
    }))
}

fn default_database_url() -> String {
    let root = env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .or_else(|| env::var_os("HOME").map(|home| PathBuf::from(home).join(".local/share")))
        .unwrap_or_else(|| PathBuf::from("."));
    let directory = root.join("tunnelhub");
    if let Err(error) = std::fs::create_dir_all(&directory) {
        tracing::warn!(?error, "Could not create data directory; using local database");
        return "sqlite://tunnelhub.db".to_string();
    }
    format!("sqlite://{}", directory.join("tunnelhub.db").display())
}


async fn require_admin_api(State(state): State<AppState>, request: Request, next: Next) -> Response {
    let trusted = request
        .headers()
        .get("x-tunnelhub-admin")
        .and_then(|value| value.to_str().ok())
        == Some("1");
    if !trusted {
        let mut request = request;
        let original_uri = request
            .extensions()
            .get::<axum::extract::OriginalUri>()
            .map(|original| original.0.clone());
        if let Some(uri) = original_uri {
            *request.uri_mut() = uri;
        }
        return proxy::handle(State(state), request).await;
    }
    next.run(request).await
}
