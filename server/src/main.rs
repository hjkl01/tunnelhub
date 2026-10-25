mod auth;
mod caddy;
mod proxy;
mod services;
mod state;

use std::{env, net::SocketAddr, path::PathBuf, time::Duration};

use axum::{middleware, routing::get, Router};
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
    sqlx::query("CREATE TABLE IF NOT EXISTS auth_login_attempts (id INTEGER PRIMARY KEY AUTOINCREMENT, username TEXT NOT NULL, attempted_at TEXT NOT NULL)")
        .execute(&pool)
        .await?;
    sqlx::query("CREATE INDEX IF NOT EXISTS idx_auth_login_attempts_user_time ON auth_login_attempts(username, attempted_at)")
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

    let service_columns: Vec<String> = sqlx::query_scalar("SELECT name FROM pragma_table_info('services')")
        .fetch_all(&pool).await?;
    if !service_columns.iter().any(|name| name == "access_mode") {
        sqlx::query("ALTER TABLE services ADD COLUMN access_mode TEXT NOT NULL DEFAULT 'public'")
            .execute(&pool).await?;
    }
    sqlx::query("CREATE TABLE IF NOT EXISTS members (id INTEGER PRIMARY KEY AUTOINCREMENT, username TEXT NOT NULL UNIQUE, password_hash TEXT NOT NULL, enabled INTEGER NOT NULL DEFAULT 1, created_at TEXT NOT NULL)")
        .execute(&pool).await?;
    sqlx::query("CREATE TABLE IF NOT EXISTS user_groups (id INTEGER PRIMARY KEY AUTOINCREMENT, name TEXT NOT NULL UNIQUE, created_at TEXT NOT NULL)")
        .execute(&pool).await?;
    sqlx::query("CREATE TABLE IF NOT EXISTS group_members (group_id INTEGER NOT NULL REFERENCES user_groups(id) ON DELETE CASCADE, member_id INTEGER NOT NULL REFERENCES members(id) ON DELETE CASCADE, PRIMARY KEY (group_id, member_id))")
        .execute(&pool).await?;
    sqlx::query("CREATE TABLE IF NOT EXISTS service_acl (service_id TEXT NOT NULL REFERENCES services(id) ON DELETE CASCADE, subject_type TEXT NOT NULL CHECK (subject_type IN ('user', 'group')), subject_id INTEGER NOT NULL, PRIMARY KEY (service_id, subject_type, subject_id))")
        .execute(&pool).await?;
    sqlx::query("CREATE TABLE IF NOT EXISTS member_sessions (token_hash TEXT PRIMARY KEY NOT NULL, member_id INTEGER NOT NULL REFERENCES members(id) ON DELETE CASCADE, created_at TEXT NOT NULL, expires_at TEXT NOT NULL)")
        .execute(&pool).await?;
    sqlx::query("CREATE INDEX IF NOT EXISTS idx_member_sessions_expires_at ON member_sessions(expires_at)")
        .execute(&pool).await?;
    sqlx::query("CREATE TABLE IF NOT EXISTS member_login_attempts (id INTEGER PRIMARY KEY AUTOINCREMENT, username TEXT NOT NULL, attempted_at TEXT NOT NULL)")
        .execute(&pool).await?;
    sqlx::query("CREATE INDEX IF NOT EXISTS idx_member_login_attempts_user_time ON member_login_attempts(username, attempted_at)")
        .execute(&pool).await?;

    let client = reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(5))
        .redirect(reqwest::redirect::Policy::none())
        .build()?;
    let state = AppState { pool: pool.clone(), client };
    let app = Router::new()
        .route("/api/health", get(health))
        .route("/_internal/proxy-auth/{id}", get(proxy::authorize_proxy))
        .nest("/api/auth", auth::router())
        .nest("/api/services", services::router().route_layer(middleware::from_fn_with_state(state.clone(), auth::require_session)))
        .fallback(proxy::handle)
        .with_state(state)
        .layer(TraceLayer::new_for_http());

    let caddy_enabled = env::var("TUNNELHUB_CADDY_ENABLED")
        .map(|value| value.eq_ignore_ascii_case("true") || value == "1")
        .unwrap_or(false);
    let address: SocketAddr = env::var("TUNNELHUB_LISTEN")
        .unwrap_or_else(|_| if caddy_enabled { "127.0.0.1:8081".to_string() } else { "0.0.0.0:8080".to_string() })
        .parse()?;

    // In container mode Caddy is the public HTTP entry point. Rust remains bound
    // to loopback and owns the API, embedded frontend, and forward-auth checks.
    let mut caddy_child = if caddy_enabled {
        let binary = env::var("TUNNELHUB_CADDY_BIN").unwrap_or_else(|_| "caddy".to_string());
        let caddy_listen = env::var("TUNNELHUB_CADDY_LISTEN")
            .unwrap_or_else(|_| "0.0.0.0:8080".to_string());
        let config_path = env::var_os("TUNNELHUB_CADDY_CONFIG")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("/data/Caddyfile"));
        Some(caddy::start(pool.clone(), binary, caddy_listen, config_path).await?)
    } else {
        None
    };

    let listener = tokio::net::TcpListener::bind(address).await?;
    tracing::info!(%address, caddy_enabled, "TunnelHub API listening");
    if let Some(child) = caddy_child.as_mut() {
        tokio::select! {
            result = axum::serve(listener, app) => result?,
            status = child.wait() => {
                return Err(anyhow::anyhow!("Caddy exited unexpectedly: {:?}", status?));
            }
        }
    } else {
        axum::serve(listener, app).await?;
    }
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

